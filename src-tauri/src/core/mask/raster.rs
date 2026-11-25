use std::borrow::Cow;

use ndarray::{Array2, Zip};
use rayon::prelude::*;
use crate::core::imaging::luminance::rgb_to_luminance;
use crate::core::imaging::masked_stretch::normalize_to_01;
use crate::core::imaging::region::{paint_exclusions, RegionShape};
use crate::core::imaging::star_mask::{generate_star_mask, StarMaskConfig, StarMaskResult};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig};
use crate::infra::image_source::load_plane;
use crate::types::image_ref::ImageRef;

use super::error::MaskError;
use super::feather::feather;
use super::recipe::{MarkMode, MaskMods, MaskRecipe};

pub const COVERAGE_FLOOR: f32 = 0.01;

pub struct Rasterized {
    pub mask: Array2<f32>,
    pub stars: Option<usize>,
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
