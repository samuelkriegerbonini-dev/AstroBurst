pub mod cache;
pub mod feather;
#[cfg(test)]
mod tests;

use std::borrow::Cow;

use ndarray::{Array2, Zip};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::astrometry::wcs::{angular_separation, pixel_edge_corners, WcsTransform};
use crate::core::history::id::Fnv1a;
use crate::core::imaging::geometry::GeometryOp;
use crate::core::imaging::luminance::rgb_to_luminance;
use crate::core::imaging::masked_stretch::normalize_to_01;
use crate::core::imaging::region::{paint_exclusions, RegionError, RegionShape};
use crate::core::imaging::star_mask::{generate_star_mask, StarMaskConfig, StarMaskResult};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig};
use crate::infra::image_source::load_plane;
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

pub use self::feather::{blend, feather};

pub const COVERAGE_FLOOR: f32 = 0.01;
pub const WCS_MATCH_TOLERANCE: f64 = 1e-9;
pub const WCS_CORNER_TOLERANCE_PX: f64 = 0.5;
const TAN_CTYPES: [&str; 2] = ["RA---TAN", "DEC--TAN"];
const ARCSEC_PER_DEG: f64 = 3600.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkMode {
    #[default]
    NonZero,
    Zero,
    Nan,
    NonNan,
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MaskMods {
    pub feather_px: f32,
    pub invert: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskSource {
    #[default]
    OpInput,
    Node(String),
    Composite,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WcsSummary {
    pub crval: [f64; 2],
    pub crpix: [f64; 2],
    pub cd: [[f64; 2]; 2],
    pub naxis: [usize; 2],
}

type RenderField = fn(&WcsSummary) -> String;

impl WcsSummary {
    pub fn of(wcs: &WcsTransform, (rows, cols): (usize, usize)) -> Self {
        let (crpix1, crpix2, crval1, crval2, cd, _) = wcs.raw_params();
        Self { crval: [crval1, crval2], crpix: [crpix1, crpix2], cd, naxis: [cols, rows] }
    }

    pub fn mismatch(&self, target: &Self, own: &str, other: &str) -> Option<String> {
        let fields: [(&str, bool, RenderField); 4] = [
            ("NAXIS", self.naxis == target.naxis, |s| format!("{}×{}", s.naxis[0], s.naxis[1])),
            ("CRPIX", close(&self.crpix, &target.crpix, 1.0), |s| format!("({:.6}, {:.6})", s.crpix[0], s.crpix[1])),
            ("CRVAL", close(&self.crval, &target.crval, 1.0), |s| format!("({:.8}, {:.8})", s.crval[0], s.crval[1])),
            ("CD", close(&self.cd.concat(), &target.cd.concat(), 0.0), |s| {
                format!("({:.6e}, {:.6e}, {:.6e}, {:.6e})", s.cd[0][0], s.cd[0][1], s.cd[1][0], s.cd[1][1])
            }),
        ];
        let differing: Vec<&(&str, bool, RenderField)> = fields.iter().filter(|(_, same, _)| !same).collect();
        if differing.is_empty() {
            return None;
        }
        let names: Vec<&str> = differing.iter().map(|(name, _, _)| *name).collect();
        let show = |summary: &Self| {
            differing.iter().map(|(name, _, render)| format!("{name} {}", render(summary))).collect::<Vec<_>>().join(", ")
        };
        Some(format!("WCS summary differs in {}: {} on {own}, {} on {other}", join_with_and(&names), show(self), show(target)))
    }

    fn tan_transform(&self) -> Option<WcsTransform> {
        let mut header = HduHeader::empty();
        let cards = [
            ("NAXIS1", self.naxis[0].to_string()),
            ("NAXIS2", self.naxis[1].to_string()),
            ("CTYPE1", TAN_CTYPES[0].to_string()),
            ("CTYPE2", TAN_CTYPES[1].to_string()),
            ("CRPIX1", self.crpix[0].to_string()),
            ("CRPIX2", self.crpix[1].to_string()),
            ("CRVAL1", self.crval[0].to_string()),
            ("CRVAL2", self.crval[1].to_string()),
            ("CD1_1", self.cd[0][0].to_string()),
            ("CD1_2", self.cd[0][1].to_string()),
            ("CD2_1", self.cd[1][0].to_string()),
            ("CD2_2", self.cd[1][1].to_string()),
        ];
        for (key, value) in cards {
            header.set(key, value);
        }
        WcsTransform::from_header(&header).ok()
    }

    pub fn corner_offset_px(&self, target: &Self) -> Option<f64> {
        let (drawn, image) = (self.tan_transform()?, target.tan_transform()?);
        let arcsec_per_px = image.pixel_scale_arcsec();
        pixel_edge_corners(target.naxis[0], target.naxis[1]).into_iter().try_fold(0.0_f64, |worst, (x, y)| {
            let (on_mask, on_image) = (drawn.pixel_to_world(x, y), image.pixel_to_world(x, y));
            let offset = angular_separation(on_mask.ra, on_mask.dec, on_image.ra, on_image.dec) * ARCSEC_PER_DEG / arcsec_per_px;
            offset.is_finite().then_some(worst.max(offset))
        })
    }
}

fn close(a: &[f64], b: &[f64], floor: f64) -> bool {
    let scale = a.iter().chain(b).fold(floor, |scale, v| scale.max(v.abs()));
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= WCS_MATCH_TOLERANCE * scale)
}

fn join_with_and(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => only.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub stem: String,
    pub geometry: Option<GeometryOp>,
    pub wcs: Option<WcsSummary>,
}

fn geometry_name(geometry: Option<GeometryOp>) -> &'static str {
    geometry.map_or("none", GeometryOp::name)
}

pub fn check_file_grid(mask: &Grid, image: &Grid) -> Result<(), MaskError> {
    let refused = |detail: String| MaskError::FileGrid { mask: mask.stem.clone(), image: image.stem.clone(), detail };
    if mask.geometry != image.geometry {
        return Err(refused(format!(
            "geometry step {} on {}, {} on {}",
            geometry_name(image.geometry),
            image.stem,
            geometry_name(mask.geometry),
            mask.stem
        )));
    }
    match (&mask.wcs, &image.wcs) {
        (Some(drawn), Some(target)) => drawn.mismatch(target, &mask.stem, &image.stem).map_or(Ok(()), |detail| Err(refused(detail))),
        _ => Ok(()),
    }
}

pub fn check_regions_grid(source_wcs: Option<&WcsSummary>, image: &Grid) -> Result<(), MaskError> {
    let Some(drawn) = source_wcs else {
        return Ok(());
    };
    let refused = |detail: String| Err(MaskError::RegionsGrid { image: image.stem.clone(), detail });
    let Some(target) = &image.wcs else {
        return refused(format!("{} has no WCS", image.stem));
    };
    let Some(mismatch) = drawn.mismatch(target, "the mask", &image.stem) else {
        return Ok(());
    };
    if drawn.naxis != target.naxis {
        return refused(mismatch);
    }
    match drawn.corner_offset_px(target) {
        Some(offset) if offset <= WCS_CORNER_TOLERANCE_PX => Ok(()),
        Some(offset) => refused(format!(
            "{mismatch}; the image corners are {offset:.2} px apart, over the {WCS_CORNER_TOLERANCE_PX} px limit"
        )),
        None => refused(mismatch),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "StarMaskConfig", default, deny_unknown_fields)]
struct StarParams {
    growth_factor: f64,
    softness: f64,
    #[serde(skip)]
    detection_sigma: f64,
    min_fwhm: f64,
    max_fwhm: f64,
    max_eccentricity: f64,
    luminance_protect: bool,
    luminance_ceiling: f64,
}

impl Default for StarParams {
    fn default() -> Self {
        let StarMaskConfig {
            growth_factor,
            softness,
            detection_sigma,
            min_fwhm,
            max_fwhm,
            max_eccentricity,
            luminance_protect,
            luminance_ceiling,
        } = StarMaskConfig::default();
        Self {
            growth_factor,
            softness,
            detection_sigma,
            min_fwhm,
            max_fwhm,
            max_eccentricity,
            luminance_protect,
            luminance_ceiling,
        }
    }
}

fn default_star_sigma() -> f32 {
    StarMaskConfig::default().detection_sigma as f32
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaskRecipe {
    Luminance {
        #[serde(default)]
        auto_stf: bool,
    },
    Range {
        lo: f32,
        hi: f32,
        #[serde(default)]
        fuzz: f32,
    },
    Stars {
        #[serde(default, with = "StarParams")]
        config: StarMaskConfig,
        #[serde(default = "default_star_sigma")]
        sigma: f32,
    },
    Regions {
        shapes: Vec<RegionShape>,
        #[serde(default)]
        exclude_shapes: Vec<RegionShape>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_wcs: Option<WcsSummary>,
    },
    File {
        path: String,
        #[serde(default)]
        mark: MarkMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lo: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hi: Option<f32>,
    },
}

impl MaskRecipe {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Luminance { .. } => "luminance",
            Self::Range { .. } => "range",
            Self::Stars { .. } => "stars",
            Self::Regions { .. } => "regions",
            Self::File { .. } => "file",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MaskSpec {
    #[serde(flatten)]
    pub mods: MaskMods,
    #[serde(flatten)]
    pub recipe: MaskRecipe,
}

impl MaskSpec {
    pub fn rasterize(&self, planes: &[&Array2<f32>]) -> Result<Array2<f32>, MaskError> {
        rasterize(&self.recipe, &self.mods, planes)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MaskError {
    #[error("mask {id} is {}×{}, image is {}×{}", .mask.1, .mask.0, .image.1, .image.0)]
    Dims { id: String, mask: (usize, usize), image: (usize, usize) },
    #[error("mask {0} is not in this image's history")]
    Unknown(String),
    #[error("mask {0} was deleted; pick another mask or create it again")]
    Deleted(String),
    #[error("a mask is built from one or three planes; got {0}")]
    PlaneCount(usize),
    #[error("a mask needs planes of one size; got {0:?}")]
    PlaneDims(Vec<(usize, usize)>),
    #[error("{role} {index} ({kind}): {source}")]
    Region { role: &'static str, index: usize, kind: &'static str, source: RegionError },
    #[error("{0}")]
    Invalid(String),
    #[error("only a regions mask has a label image; this mask is {0}")]
    NotRegions(&'static str),
    #[error("mask file {path} cannot be read: {reason}")]
    File { path: String, reason: String },
    #[error("star mask: {0}")]
    Stars(String),
    #[error("mask file {mask} was not rotated or flipped like {image} ({detail}); redraw or re-export the mask")]
    FileGrid { mask: String, image: String, detail: String },
    #[error("region mask was drawn on another grid than {image} ({detail}); redraw the regions on {image}")]
    RegionsGrid { image: String, detail: String },
    #[error(
        "a blend needs base, processed and mask of one size; got base {}×{}, processed {}×{}, mask {}×{}",
        .base.1, .base.0, .processed.1, .processed.0, .mask.1, .mask.0
    )]
    BlendDims { base: (usize, usize), processed: (usize, usize), mask: (usize, usize) },
}

pub struct Rasterized {
    pub mask: Array2<f32>,
    pub stars: Option<usize>,
}

pub fn recipe_hash(recipe: &MaskRecipe, mods: &MaskMods) -> u64 {
    let canonical = serde_json::to_string(&(recipe, mods)).expect("a mask recipe has no map keys, so it always serialises");
    Fnv1a::default().update(canonical.as_bytes()).finish()
}

pub fn check_dims(mask_id: &str, mask: (usize, usize), image: (usize, usize)) -> Result<(), MaskError> {
    if mask == image {
        Ok(())
    } else {
        Err(MaskError::Dims { id: mask_id.to_string(), mask, image })
    }
}

pub fn coverage(mask: &Array2<f32>) -> f64 {
    if mask.is_empty() {
        return 0.0;
    }
    let covered = match mask.as_slice() {
        Some(values) => values.par_iter().filter(|&&m| m > COVERAGE_FLOOR).count(),
        None => mask.iter().filter(|&&m| m > COVERAGE_FLOOR).count(),
    };
    covered as f64 / mask.len() as f64
}

pub fn rasterize(recipe: &MaskRecipe, mods: &MaskMods, planes: &[&Array2<f32>]) -> Result<Array2<f32>, MaskError> {
    rasterize_with_stars(recipe, mods, planes).map(|rasterized| rasterized.mask)
}

#[cfg(test)]
thread_local! {
    static RASTERIZED_ON_THIS_THREAD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn rasterized_on_this_thread() -> usize {
    RASTERIZED_ON_THIS_THREAD.with(std::cell::Cell::get)
}

pub fn rasterize_with_stars(
    recipe: &MaskRecipe,
    mods: &MaskMods,
    planes: &[&Array2<f32>],
) -> Result<Rasterized, MaskError> {
    #[cfg(test)]
    RASTERIZED_ON_THIS_THREAD.with(|count| count.set(count.get() + 1));
    check_mods(mods)?;
    let dims = input_dims(planes)?;
    let (mask, stars) = match recipe {
        MaskRecipe::Luminance { auto_stf } => (luminance_mask(&luminance(planes), *auto_stf), None),
        MaskRecipe::Range { lo, hi, fuzz } => (range_mask(&luminance(planes), *lo, *hi, *fuzz)?, None),
        MaskRecipe::Stars { config, sigma } => {
            let detected = star_mask(&luminance(planes), config, *sigma)?;
            (detected.mask, Some(detected.stars_masked))
        }
        MaskRecipe::Regions { shapes, exclude_shapes, .. } => (regions_mask(shapes, exclude_shapes, dims)?, None),
        MaskRecipe::File { path, mark, lo, hi } => (file_mask(path, *mark, *lo, *hi)?, None),
    };
    Ok(Rasterized { mask: with_mods(mask, mods), stars })
}

pub fn label_image(recipe: &MaskRecipe, dims: (usize, usize)) -> Result<Array2<i32>, MaskError> {
    let MaskRecipe::Regions { shapes, exclude_shapes, .. } = recipe else {
        return Err(MaskError::NotRegions(recipe.kind()));
    };
    check_shapes(shapes, exclude_shapes)?;
    let excluded = painted(exclude_shapes, dims);
    let mut labels = Array2::<i32>::zeros(dims);
    for (label, shape) in (1..=i32::MAX).zip(shapes) {
        paint_cells(shape, dims, |cell| labels[cell] = label);
    }
    Zip::from(&mut labels).and(&excluded).par_for_each(|label, &out| {
        if out != 0 {
            *label = 0;
        }
    });
    Ok(labels)
}

fn check_mods(mods: &MaskMods) -> Result<(), MaskError> {
    if mods.feather_px.is_finite() && mods.feather_px >= 0.0 {
        Ok(())
    } else {
        Err(MaskError::Invalid(format!("feather must be a finite number of pixels >= 0; got {}", mods.feather_px)))
    }
}

fn input_dims(planes: &[&Array2<f32>]) -> Result<(usize, usize), MaskError> {
    if !matches!(planes.len(), 1 | 3) {
        return Err(MaskError::PlaneCount(planes.len()));
    }
    let dims: Vec<(usize, usize)> = planes.iter().map(|plane| plane.dim()).collect();
    if dims.iter().any(|&d| d != dims[0]) {
        return Err(MaskError::PlaneDims(dims));
    }
    Ok(dims[0])
}

fn luminance<'a>(planes: &[&'a Array2<f32>]) -> Cow<'a, Array2<f32>> {
    match planes {
        [r, g, b] => Cow::Owned(rgb_to_luminance(r, g, b)),
        [plane, ..] if plane.is_standard_layout() => Cow::Borrowed(*plane),
        [plane, ..] => Cow::Owned(plane.as_standard_layout().into_owned()),
        [] => Cow::Owned(Array2::zeros((0, 0))),
    }
}

fn map_par(values: &Array2<f32>, weight: impl Fn(f32) -> f32 + Sync + Send) -> Array2<f32> {
    Zip::from(values).par_map_collect(|&v| weight(v))
}

fn luminance_mask(luminance: &Array2<f32>, stretch: bool) -> Array2<f32> {
    if !stretch {
        return normalize_to_01(luminance);
    }
    let stats = compute_image_stats(luminance);
    apply_stf_f32(luminance, &auto_stf(&stats, &AutoStfConfig::default()), &stats)
}

fn range_mask(values: &Array2<f32>, lo: f32, hi: f32, fuzz: f32) -> Result<Array2<f32>, MaskError> {
    if !(lo.is_finite() && hi.is_finite() && lo <= hi && fuzz.is_finite() && fuzz >= 0.0) {
        return Err(MaskError::Invalid(format!(
            "a range mask needs finite lo <= hi and a finite fuzz >= 0; got lo {lo}, hi {hi}, fuzz {fuzz}"
        )));
    }
    Ok(map_par(values, |v| range_weight(v, lo, hi, fuzz)))
}

fn range_weight(v: f32, lo: f32, hi: f32, fuzz: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let outside = (lo - v).max(v - hi);
    if outside <= 0.0 {
        1.0
    } else if outside < fuzz {
        1.0 - outside / fuzz
    } else {
        0.0
    }
}

fn star_mask(luminance: &Array2<f32>, config: &StarMaskConfig, sigma: f32) -> Result<StarMaskResult, MaskError> {
    if !(sigma.is_finite() && sigma > 0.0) {
        return Err(MaskError::Invalid(format!("a star mask needs a finite detection sigma > 0; got {sigma}")));
    }
    let config = StarMaskConfig { detection_sigma: f64::from(sigma), ..config.clone() };
    generate_star_mask(&normalize_to_01(luminance), &config).map_err(MaskError::Stars)
}

fn check_shapes(shapes: &[RegionShape], exclude_shapes: &[RegionShape]) -> Result<(), MaskError> {
    let roles = shapes.iter().map(|shape| ("shape", shape)).enumerate();
    let excludes = exclude_shapes.iter().map(|shape| ("exclude shape", shape)).enumerate();
    for (index, (role, shape)) in roles.chain(excludes) {
        shape
            .validate()
            .map_err(|source| MaskError::Region { role, index: index + 1, kind: shape.kind(), source })?;
    }
    Ok(())
}

fn painted(shapes: &[RegionShape], dims: (usize, usize)) -> Array2<u8> {
    let mut cover = Array2::zeros(dims);
    paint_exclusions(&mut cover, shapes);
    cover
}

fn paint_cells(shape: &RegionShape, (rows, cols): (usize, usize), mut paint: impl FnMut([usize; 2])) {
    if matches!(shape, RegionShape::Line { .. } | RegionShape::Point { .. }) {
        return;
    }
    let bounds = shape.bounds();
    let (x0, y0) = (bounds.x0.max(0), bounds.y0.max(0));
    let (x1, y1) = (bounds.x1.min(cols as i64 - 1), bounds.y1.min(rows as i64 - 1));
    for py in y0..=y1 {
        for px in x0..=x1 {
            if shape.contains(px as f64, py as f64) {
                paint([py as usize, px as usize]);
            }
        }
    }
}

fn regions_mask(
    shapes: &[RegionShape],
    exclude_shapes: &[RegionShape],
    dims: (usize, usize),
) -> Result<Array2<f32>, MaskError> {
    check_shapes(shapes, exclude_shapes)?;
    let included = painted(shapes, dims);
    let excluded = painted(exclude_shapes, dims);
    Ok(Zip::from(&included).and(&excluded).par_map_collect(|&inside, &out| {
        if inside != 0 && out == 0 {
            1.0
        } else {
            0.0
        }
    }))
}

fn file_mask(path: &str, mark: MarkMode, lo: Option<f32>, hi: Option<f32>) -> Result<Array2<f32>, MaskError> {
    let bounds = match (lo, hi) {
        (Some(lo), Some(hi)) if lo.is_finite() && hi.is_finite() && lo <= hi => Some((lo, hi)),
        _ => None,
    };
    if mark == MarkMode::Range && bounds.is_none() {
        return Err(MaskError::Invalid(format!("mark range needs finite lo <= hi; got lo {lo:?}, hi {hi:?}")));
    }
    let loaded = load_plane(&ImageRef::parse(path))
        .map_err(|e| MaskError::File { path: path.to_string(), reason: format!("{e:#}") })?;
    Ok(map_par(&loaded.arr, |v| if marks(mark, bounds, v) { 1.0 } else { 0.0 }))
}

fn marks(mark: MarkMode, bounds: Option<(f32, f32)>, v: f32) -> bool {
    match mark {
        MarkMode::NonZero => v != 0.0 && !v.is_nan(),
        MarkMode::Zero => v == 0.0,
        MarkMode::Nan => v.is_nan(),
        MarkMode::NonNan => !v.is_nan(),
        MarkMode::Range => bounds.is_some_and(|(lo, hi)| (lo..=hi).contains(&v)),
    }
}

fn with_mods(mask: Array2<f32>, mods: &MaskMods) -> Array2<f32> {
    let mut mask = if mods.feather_px > 0.0 {
        feather(&mask, mods.feather_px).mapv_into(|m| m.clamp(0.0, 1.0))
    } else {
        mask
    };
    if mods.invert {
        mask.par_mapv_inplace(|m| 1.0 - m);
    }
    mask
}
