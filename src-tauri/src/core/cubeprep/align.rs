use ndarray::{s, Array2};
use serde::{Deserialize, Serialize};

use crate::core::alignment::wcs_reproject::{
    prefilter_coarse_target, prefilter_companions, relative_rotation_deg, reproject_exact_companions,
    reproject_exact_to_grid, reproject_interpolated_companions, reproject_to_grid_with, CoarseMap,
    CompanionInput, CompanionMaps, Interpolation, MAP_TOLERANCE_PX, PREFILTER_MIN_RATIO, REF_TIE_TOLERANCE,
};
use crate::core::astrometry::wcs::{WcsKind, WcsTransform};
use crate::core::cubeprep::footprint::without_sip;
use crate::core::imaging::cutout::{shift_header, CutoutRect};
use crate::core::imaging::push_history;
use crate::core::metadata::units::{
    apply_area_correction, area_correction, classify_unit, AreaCorrection, AreaRule, PixelGeometry, UnitClass, UnitError,
};
use crate::infra::fits::writer::filter_header;
use crate::infra::wcs_source::header_wcs;
use crate::math::polygon::{self, Point};
use crate::types::header::HduHeader;
use crate::types::image::IntPlane;

const DEFAULT_BORDER_CROP: usize = 1;
const SAME_GRID_TOLERANCE_PX: f64 = 1e-6;
const GRID_SAMPLE_FRACTIONS: [f64; 3] = [0.0, 0.5, 1.0];
const ARCSEC2_PER_DEG2: f64 = 3600.0 * 3600.0;
const PHYSICAL_CARDS: [&str; 6] = ["LTV1", "LTV2", "LTM1_1", "LTM1_2", "LTM2_1", "LTM2_2"];
const LIGHTSTACK_SURFACE_BRIGHTNESS_WARNING: &str = "area rule lightstack multiplies surface brightness by the pixel-area ratio, as LIGHTSTACK 0.4.0 does; the aligned image does not conserve flux";
const NO_OVERLAP_WARNING: &str = "the input footprint does not overlap the reference grid; the aligned image is empty";
const PIXEL_AREA_HISTORY: &str = "Per-pixel values divided by the relative pixel area of a distorted input grid before resampling and multiplied by that of a distorted reference grid after it";
const ALIGN_ERROR_HISTORY: &str = "ERR propagated with squared effective resampling weights assuming independent input pixels; input covariance is unknown and output covariance is not stored";
const ALIGN_DQ_HISTORY: &str = "DQ propagated by bitwise OR of nonzero effective contributors; DQ flags do not exclude SCI samples";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResampleMethod {
    Bilinear,
    Bicubic,
    Exact,
}

impl ResampleMethod {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bilinear => "bilinear",
            Self::Bicubic => "bicubic",
            Self::Exact => "exact",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum ReferencePolicy {
    Coarsest,
    Finest,
    Filter { filter: String },
    Path { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignOptions {
    pub method: ResampleMethod,
    pub reference: ReferencePolicy,
    pub border_crop: usize,
    pub area_rule: Option<AreaRule>,
    pub per_actual_pixel: bool,
}

impl Default for AlignOptions {
    fn default() -> Self {
        Self {
            method: ResampleMethod::Exact,
            reference: ReferencePolicy::Coarsest,
            border_crop: DEFAULT_BORDER_CROP,
            area_rule: None,
            per_actual_pixel: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReferenceCandidate<'a> {
    pub path: &'a str,
    pub scale_arcsec: f64,
    pub filter: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReferenceChoice {
    pub index: usize,
    pub rule: &'static str,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AlignError {
    #[error(transparent)]
    Unit(#[from] UnitError),
    #[error("no image to align")]
    Empty,
    #[error("no input has a usable pixel scale to choose the reference grid from")]
    NoScale,
    #[error("no input matches the reference {0}")]
    NoReference(String),
    #[error("the reference filter {filter} matches {count} inputs")]
    AmbiguousReference { filter: String, count: usize },
    #[error("border_crop {border_crop} leaves nothing of the {cols}x{rows} reference grid")]
    BorderCrop { border_crop: usize, rows: usize, cols: usize },
    #[error("{plane} shape {actual:?} does not match SCI shape {expected:?}")]
    CompanionShape { plane: &'static str, actual: (usize, usize), expected: (usize, usize) },
}

fn usable_scale(candidate: &ReferenceCandidate) -> bool {
    candidate.scale_arcsec.is_finite() && candidate.scale_arcsec > 0.0
}

fn normalized_path(path: &str) -> String {
    path.replace('\\', "/")
}

pub fn choose_reference(candidates: &[ReferenceCandidate], policy: &ReferencePolicy) -> Result<ReferenceChoice, AlignError> {
    if candidates.is_empty() {
        return Err(AlignError::Empty);
    }
    let extreme = |fold: fn(f64, f64) -> f64| {
        candidates.iter().filter(|c| usable_scale(c)).map(|c| c.scale_arcsec).reduce(fold).ok_or(AlignError::NoScale)
    };
    match policy {
        ReferencePolicy::Coarsest => {
            let coarsest = extreme(f64::max)?;
            let index = candidates
                .iter()
                .rposition(|c| usable_scale(c) && c.scale_arcsec >= coarsest * (1.0 - REF_TIE_TOLERANCE))
                .ok_or(AlignError::NoScale)?;
            Ok(ReferenceChoice { index, rule: "coarsest" })
        }
        ReferencePolicy::Finest => {
            let finest = extreme(f64::min)?;
            let index = candidates
                .iter()
                .position(|c| usable_scale(c) && c.scale_arcsec <= finest * (1.0 + REF_TIE_TOLERANCE))
                .ok_or(AlignError::NoScale)?;
            Ok(ReferenceChoice { index, rule: "finest" })
        }
        ReferencePolicy::Filter { filter } => {
            let matches: Vec<usize> = candidates
                .iter()
                .enumerate()
                .filter(|(_, c)| c.filter.is_some_and(|name| name.eq_ignore_ascii_case(filter)))
                .map(|(index, _)| index)
                .collect();
            match matches[..] {
                [index] => Ok(ReferenceChoice { index, rule: "filter" }),
                [] => Err(AlignError::NoReference(filter.clone())),
                _ => Err(AlignError::AmbiguousReference { filter: filter.clone(), count: matches.len() }),
            }
        }
        ReferencePolicy::Path { path } => {
            let wanted = normalized_path(path);
            candidates
                .iter()
                .position(|c| normalized_path(c.path) == wanted)
                .map(|index| ReferenceChoice { index, rule: "path" })
                .ok_or_else(|| AlignError::NoReference(path.clone()))
        }
    }
}

#[derive(Clone, Copy)]
pub struct AlignGrid<'a> {
    pub label: &'a str,
    pub header: &'a HduHeader,
    pub wcs: &'a WcsTransform,
    pub rows: usize,
    pub cols: usize,
}

#[derive(Clone, Copy)]
pub struct AlignInput<'a> {
    pub image: &'a Array2<f32>,
    pub header: &'a HduHeader,
    pub wcs: &'a WcsTransform,
}

pub fn same_grid(input: &WcsTransform, input_dims: (usize, usize), grid: &WcsTransform, grid_dims: (usize, usize)) -> bool {
    let (rows, cols) = input_dims;
    if input_dims != grid_dims || rows == 0 || cols == 0 {
        return false;
    }
    let samples: Vec<(f64, f64)> = GRID_SAMPLE_FRACTIONS
        .iter()
        .flat_map(|&fy| GRID_SAMPLE_FRACTIONS.iter().map(move |&fx| (fx * cols as f64 - 0.5, fy * rows as f64 - 0.5)))
        .collect();
    let sky: Vec<(f64, f64)> = input.pixel_to_world_batch(&samples).into_iter().map(|c| (c.ra, c.dec)).collect();
    grid.world_to_pixel_batch(&sky)
        .iter()
        .zip(&samples)
        .all(|(&(gx, gy), &(x, y))| (gx - x).abs() <= SAME_GRID_TOLERANCE_PX && (gy - y).abs() <= SAME_GRID_TOLERANCE_PX)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlignPlan {
    pub reprojected: bool,
    pub area: Option<AreaCorrection>,
    pub warnings: Vec<String>,
}

pub fn plan_alignment(input: &AlignInput, grid: &AlignGrid, options: &AlignOptions) -> Result<AlignPlan, AlignError> {
    let crop = options.border_crop;
    if crop.saturating_mul(2) >= grid.rows || crop.saturating_mul(2) >= grid.cols {
        return Err(AlignError::BorderCrop { border_crop: crop, rows: grid.rows, cols: grid.cols });
    }
    let reprojected = !same_grid(input.wcs, input.image.dim(), grid.wcs, (grid.rows, grid.cols));
    let mut warnings: Vec<String> = input.wcs.pv_warning().map(str::to_string).into_iter().collect();
    if let Some(refusal) = input.wcs.gwcs_refusal() {
        warnings.push(format!("gWCS not used: {refusal}"));
    }
    let area = if reprojected {
        let correction = area_correction(input.header, options.area_rule, &PixelGeometry::between(input.wcs, grid.wcs))?;
        let surface_brightness = matches!(classify_unit(input.header).class, UnitClass::SurfaceBrightness { .. });
        if correction.rule == AreaRule::Lightstack && correction.data_factor != 1.0 && surface_brightness {
            warnings.push(LIGHTSTACK_SURFACE_BRIGHTNESS_WARNING.to_string());
        }
        Some(correction)
    } else {
        None
    };
    Ok(AlignPlan { reprojected, area, warnings })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlignReport {
    pub reprojected: bool,
    pub method: Option<ResampleMethod>,
    pub prefilter_k: usize,
    pub scale_ratio: f64,
    pub rotation_deg: f64,
    pub area: Option<AreaCorrection>,
    pub pixel_area_corrected: bool,
    pub coverage: f64,
    pub flux_ratio: Option<f64>,
    pub border_crop: usize,
    pub rows: usize,
    pub cols: usize,
    pub wcs_kind: WcsKind,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AlignedImage {
    pub image: Array2<f32>,
    pub header: HduHeader,
    pub report: AlignReport,
}

#[derive(Clone, Copy)]
pub struct AlignCompanions<'a> {
    pub err: Option<&'a Array2<f32>>,
    pub dq: Option<&'a IntPlane>,
    pub no_data_dq: u32,
}

pub struct AlignedScience {
    pub aligned: AlignedImage,
    pub err: Option<Array2<f32>>,
    pub dq: IntPlane,
}

fn resample(input: &AlignInput, grid: &AlignGrid, method: ResampleMethod) -> (Array2<f32>, Option<Array2<f32>>, usize, Option<String>) {
    let (rows, cols) = (grid.rows, grid.cols);
    match method {
        ResampleMethod::Bilinear => {
            let (image, _) = reproject_to_grid_with(input.image, input.wcs, grid.wcs, rows, cols, Interpolation::Bilinear);
            (image, None, 1, None)
        }
        ResampleMethod::Bicubic => {
            let ratio = grid.wcs.pixel_scale_arcsec() / input.wcs.pixel_scale_arcsec();
            let reduced = prefilter_coarse_target(input.image, input.header, ratio)
                .and_then(|(image, header, k)| header_wcs(&header).ok().map(|wcs| (image, wcs, k)));
            match reduced {
                Some((image, wcs, k)) => {
                    let warning = (input.wcs.kind() == WcsKind::Gwcs).then(|| {
                        format!("bicubic block-averaged the input by {k} and placed the result with its FITS header WCS, which only approximates its gWCS; exact and bilinear resample through the gWCS itself")
                    });
                    (reproject_to_grid_with(&image, &wcs, grid.wcs, rows, cols, Interpolation::Bicubic).0, None, k, warning)
                }
                None => {
                    let warning = (ratio >= PREFILTER_MIN_RATIO).then(|| {
                        format!("bicubic resampled onto a grid {ratio:.2} times coarser without its block-average prefilter, because the input header has no FITS WCS to rescale or the image is smaller than one block; fine structure can alias")
                    });
                    (reproject_to_grid_with(input.image, input.wcs, grid.wcs, rows, cols, Interpolation::Bicubic).0, None, 1, warning)
                }
            }
        }
        ResampleMethod::Exact => {
            let (image, footprint, _) = reproject_exact_to_grid(input.image, input.wcs, grid.wcs, rows, cols);
            (image, Some(footprint), 1, None)
        }
    }
}

fn header_on_grid(input_header: &HduHeader, grid_header: &HduHeader) -> HduHeader {
    let mut header = filter_header(input_header, false, true).unwrap_or_else(HduHeader::empty);
    for key in PHYSICAL_CARDS {
        header.remove(key);
    }
    let grid_cards = filter_header(grid_header, true, false).map(|wcs| wcs.cards).unwrap_or_default();
    let physical = PHYSICAL_CARDS.iter().filter_map(|&key| grid_header.get(key).map(|value| (key.to_string(), value.to_string())));
    for (key, value) in grid_cards.into_iter().chain(physical) {
        header.set(&key, value);
    }
    header
}

fn pixel_area_arcsec2(wcs: &WcsTransform) -> f64 {
    let cd = wcs.raw_params().4;
    (cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]).abs() * ARCSEC2_PER_DEG2
}

fn area_weighted(header: &HduHeader, area: Option<&AreaCorrection>) -> bool {
    match classify_unit(header).class {
        UnitClass::SurfaceBrightness { .. } => true,
        UnitClass::PerPixelPhysical { .. } => false,
        UnitClass::PerPixelCalibrated { .. } | UnitClass::Unknown => area.is_none_or(|correction| correction.rule == AreaRule::Keep),
    }
}

fn flux_ratio(input: &AlignInput, grid: &AlignGrid, plan: &AlignPlan, image: &Array2<f32>, footprint: Option<&Array2<f32>>, crop: usize) -> Option<f64> {
    let (rows, cols) = (grid.rows, grid.cols);
    let kept = |x: f64, y: f64| {
        x >= crop as f64 - 0.5 && x < (cols - crop) as f64 - 0.5 && y >= crop as f64 - 0.5 && y < (rows - crop) as f64 - 0.5
    };
    let flux_out: f64 = image
        .indexed_iter()
        .filter(|((y, x), value)| value.is_finite() && kept(*x as f64, *y as f64))
        .map(|((y, x), value)| f64::from(*value) * footprint.map_or(1.0, |f| f64::from(f[[y, x]])))
        .sum();
    let (in_rows, in_cols) = input.image.dim();
    let map = plan.reprojected.then(|| CoarseMap::within(grid.wcs, input.wcs, (0.0, 0.0), in_rows, in_cols, MAP_TOLERANCE_PX));
    let flux_in: f64 = input
        .image
        .indexed_iter()
        .filter(|((y, x), value)| {
            let (gx, gy) = map.as_ref().map_or((*x as f64, *y as f64), |map| map.sample(*x as f64, *y as f64));
            value.is_finite() && kept(gx, gy)
        })
        .map(|(_, value)| f64::from(*value))
        .sum();
    let (weight_in, weight_out) = if area_weighted(input.header, plan.area.as_ref()) {
        (pixel_area_arcsec2(input.wcs), pixel_area_arcsec2(grid.wcs))
    } else {
        (1.0, 1.0)
    };
    let ratio = (flux_out * weight_out) / (flux_in * weight_in);
    ratio.is_finite().then_some(ratio)
}

fn holds_per_pixel_values(header: &HduHeader, area: Option<&AreaCorrection>) -> bool {
    area.is_some_and(|correction| match correction.rule {
        AreaRule::Lightstack => false,
        AreaRule::Scale => true,
        AreaRule::Keep => matches!(classify_unit(header).class, UnitClass::PerPixelCalibrated { .. }),
    })
}

fn linear_twin(header: &HduHeader, wcs: &WcsTransform, rows: usize, cols: usize) -> Option<WcsTransform> {
    match wcs.kind() {
        WcsKind::Gwcs => {
            let (crpix1, crpix2, crval1, crval2, cd, _) = wcs.raw_params();
            let mut header = HduHeader::empty();
            header.set("CTYPE1", "RA---TAN".to_string());
            header.set("CTYPE2", "DEC--TAN".to_string());
            header.set("NAXIS1", cols.to_string());
            header.set("NAXIS2", rows.to_string());
            for (key, value) in [
                ("CRPIX1", crpix1),
                ("CRPIX2", crpix2),
                ("CRVAL1", crval1),
                ("CRVAL2", crval2),
                ("CD1_1", cd[0][0]),
                ("CD1_2", cd[0][1]),
                ("CD2_1", cd[1][0]),
                ("CD2_2", cd[1][1]),
            ] {
                header.set_f64(key, value);
            }
            WcsTransform::from_header(&header).ok()
        }
        WcsKind::Header if !matches!(wcs.sip_forward_terms(), (None, None)) => header_wcs(&without_sip(header)).ok(),
        WcsKind::Header => None,
    }
}

fn relative_pixel_area(header: &HduHeader, wcs: &WcsTransform, rows: usize, cols: usize) -> Option<Array2<f32>> {
    let linear = linear_twin(header, wcs, rows, cols)?;
    let corners = CoarseMap::within(&linear, wcs, (-0.5, -0.5), rows + 1, cols + 1, MAP_TOLERANCE_PX);
    let corner = |i: usize, j: usize| -> Point {
        let (x, y) = corners.sample(i as f64 - 0.5, j as f64 - 0.5);
        [x, y]
    };
    Some(Array2::from_shape_fn((rows, cols), |(y, x)| {
        polygon::area(&[corner(x, y), corner(x + 1, y), corner(x + 1, y + 1), corner(x, y + 1)]) as f32
    }))
}

fn crop_border(image: Array2<f32>, mut header: HduHeader, crop: usize) -> (Array2<f32>, HduHeader) {
    let (rows, cols) = image.dim();
    if crop == 0 {
        header.set("NAXIS1", cols.to_string());
        header.set("NAXIS2", rows.to_string());
        return (image, header);
    }
    let rect = CutoutRect { x0: crop as i64, y0: crop as i64, width: cols - 2 * crop, height: rows - 2 * crop };
    let cropped = image.slice(s![crop..rows - crop, crop..cols - crop]).to_owned();
    let mut header = shift_header(&header, &rect);
    push_history(&mut header, &format!("Cropped {crop} pixels from each border"));
    (cropped, header)
}

fn alignment_areas(input: &AlignInput, grid: &AlignGrid, options: &AlignOptions, plan: &AlignPlan) -> (Option<Array2<f32>>, Option<Array2<f32>>) {
    let per_pixel = options.per_actual_pixel && plan.reprojected && holds_per_pixel_values(input.header, plan.area.as_ref());
    let (in_rows, in_cols) = input.image.dim();
    let input_area = per_pixel.then(|| relative_pixel_area(input.header, input.wcs, in_rows, in_cols)).flatten();
    let grid_area = per_pixel.then(|| relative_pixel_area(grid.header, grid.wcs, grid.rows, grid.cols)).flatten();
    (input_area, grid_area)
}

pub fn align_image(input: &AlignInput, grid: &AlignGrid, options: &AlignOptions, plan: &AlignPlan) -> AlignedImage {
    let (input_area, grid_area) = alignment_areas(input, grid, options, plan);
    let flattened = input_area.as_ref().map(|area| input.image / area);
    let source = AlignInput { image: flattened.as_ref().unwrap_or(input.image), ..*input };
    let (mut image, footprint, prefilter_k, resample_warning) =
        if plan.reprojected { resample(&source, grid, options.method) } else { (input.image.to_owned(), None, 1, None) };
    if let Some(area) = &grid_area {
        image *= area;
    }
    let pixel_area = input_area.or(grid_area);
    let mut header = if plan.reprojected { header_on_grid(input.header, grid.header) } else { input.header.clone() };
    let note = if plan.reprojected {
        format!("Reprojected onto the grid of {} with the {} method", grid.label, options.method.label())
    } else {
        format!("Already on the grid of {}: copied without resampling", grid.label)
    };
    push_history(&mut header, &note);
    if pixel_area.is_some() {
        push_history(&mut header, PIXEL_AREA_HISTORY);
    }
    if let Some(area) = &plan.area {
        apply_area_correction(area, &mut image, None, &mut header);
    }
    let flux_ratio = flux_ratio(input, grid, plan, &image, footprint.as_ref(), options.border_crop);
    let (image, header) = crop_border(image, header, options.border_crop);
    let (rows, cols) = image.dim();
    let coverage = if image.is_empty() { 0.0 } else { image.iter().filter(|v| v.is_finite()).count() as f64 / image.len() as f64 };
    let mut warnings = plan.warnings.clone();
    warnings.extend(resample_warning);
    if coverage == 0.0 {
        warnings.push(NO_OVERLAP_WARNING.to_string());
    }
    let report = AlignReport {
        reprojected: plan.reprojected,
        method: plan.reprojected.then_some(options.method),
        prefilter_k,
        scale_ratio: input.wcs.pixel_scale_arcsec() / grid.wcs.pixel_scale_arcsec(),
        rotation_deg: relative_rotation_deg(input.wcs, grid.wcs),
        area: plan.area.clone(),
        pixel_area_corrected: pixel_area.is_some(),
        coverage,
        flux_ratio,
        border_crop: options.border_crop,
        rows,
        cols,
        wcs_kind: input.wcs.kind(),
        warnings,
    };
    AlignedImage { image, header, report }
}

pub fn align_image_with_companions(
    input: &AlignInput,
    grid: &AlignGrid,
    options: &AlignOptions,
    plan: &AlignPlan,
    companions: AlignCompanions<'_>,
) -> Result<AlignedScience, AlignError> {
    for (plane, dims) in [("ERR", companions.err.map(Array2::dim)), ("DQ", companions.dq.map(|dq| dq.bits.dim()))] {
        if let Some(actual) = dims.filter(|dims| *dims != input.image.dim()) {
            return Err(AlignError::CompanionShape { plane, actual, expected: input.image.dim() });
        }
    }
    let mut aligned = align_image(input, grid, options, plan);
    let (input_area, grid_area) = alignment_areas(input, grid, options, plan);
    let flattened = input_area.as_ref().map(|area| input.image / area);
    let source = flattened.as_ref().unwrap_or(input.image);
    let mut variance = companions.err.map(|plane| plane.mapv(|err| {
        if err.is_finite() && err >= 0.0 { f64::from(err).powi(2) } else { f64::NAN }
    }));
    if let (Some(variance), Some(area)) = (variance.as_mut(), input_area.as_ref()) {
        for (value, &factor) in variance.iter_mut().zip(area) {
            *value /= f64::from(factor).powi(2);
        }
    }
    let data = CompanionInput { variance: variance.as_ref(), dq: companions.dq.map(|plane| &plane.bits), no_data_dq: companions.no_data_dq };
    let dims = (grid.rows, grid.cols);
    let mut maps = if !plan.reprojected {
        CompanionMaps { variance: variance.clone(), dq: companions.dq.map_or_else(|| Array2::zeros(input.image.dim()), |plane| plane.bits.clone()) }
    } else {
        match options.method {
            ResampleMethod::Bilinear => reproject_interpolated_companions(source, data, input.wcs, grid.wcs, dims, Interpolation::Bilinear),
            ResampleMethod::Exact => reproject_exact_companions(source, data, input.wcs, grid.wcs, dims),
            ResampleMethod::Bicubic => {
                let ratio = grid.wcs.pixel_scale_arcsec() / input.wcs.pixel_scale_arcsec();
                let reduced = prefilter_coarse_target(source, input.header, ratio)
                    .and_then(|(image, header, k)| header_wcs(&header).ok().map(|wcs| (image, wcs, k)));
                match reduced {
                    Some((image, wcs, k)) => {
                        let reduced = prefilter_companions(source, data, k);
                        let data = CompanionInput { variance: reduced.variance.as_ref(), dq: Some(&reduced.dq), no_data_dq: companions.no_data_dq };
                        reproject_interpolated_companions(&image, data, &wcs, grid.wcs, dims, Interpolation::Bicubic)
                    }
                    None => reproject_interpolated_companions(source, data, input.wcs, grid.wcs, dims, Interpolation::Bicubic),
                }
            }
        }
    };
    if let Some(variance) = maps.variance.as_mut() {
        if let Some(area) = grid_area.as_ref() {
            for (value, &factor) in variance.iter_mut().zip(area) {
                *value *= f64::from(factor).powi(2);
            }
        }
        if let Some(area) = &plan.area {
            *variance *= area.data_factor.powi(2);
        }
    }
    let crop = options.border_crop;
    let (rows, cols) = maps.dq.dim();
    let mut err = maps.variance.map(|variance| variance.slice(s![crop..rows - crop, crop..cols - crop]).mapv(|value| {
        let sigma = value.sqrt() as f32;
        if sigma.is_finite() { sigma } else { f32::NAN }
    }));
    let mut dq = maps.dq.slice(s![crop..rows - crop, crop..cols - crop]).to_owned();
    for ((y, x), &value) in aligned.image.indexed_iter() {
        if !value.is_finite() {
            dq[[y, x]] |= companions.no_data_dq;
            if let Some(err) = err.as_mut() {
                err[[y, x]] = f32::NAN;
            }
        }
    }
    if plan.reprojected && companions.err.is_some() {
        aligned.report.warnings.push(ALIGN_ERROR_HISTORY.to_string());
        push_history(&mut aligned.header, ALIGN_ERROR_HISTORY);
    }
    if companions.dq.is_some() {
        aligned.report.warnings.push(ALIGN_DQ_HISTORY.to_string());
        push_history(&mut aligned.header, ALIGN_DQ_HISTORY);
    }
    Ok(AlignedScience { aligned, err, dq: IntPlane { bits: dq, signed: companions.dq.is_some_and(|plane| plane.signed) } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::alignment::wcs_reproject::reproject_exact_to_grid;
    use crate::core::metadata::units::UnitError;

    const ARCSEC: f64 = 1.0 / 3600.0;

    fn grid_header(dims: (usize, usize), crpix: (f64, f64), crval: (f64, f64), scale_arcsec: f64, rotation_deg: f64, bunit: &str) -> HduHeader {
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let s = scale_arcsec * ARCSEC;
        let mut header = HduHeader::empty();
        for (key, value) in [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("BUNIT", bunit), ("TELESCOP", "JWST")] {
            header.set(key, value.to_string());
        }
        header.set("NAXIS1", dims.0.to_string());
        header.set("NAXIS2", dims.1.to_string());
        for (key, value) in [
            ("CRPIX1", crpix.0),
            ("CRPIX2", crpix.1),
            ("CRVAL1", crval.0),
            ("CRVAL2", crval.1),
            ("CD1_1", -s * cos),
            ("CD1_2", -s * sin),
            ("CD2_1", -s * sin),
            ("CD2_2", s * cos),
        ] {
            header.set_f64(key, value);
        }
        header
    }

    fn wcs_of(header: &HduHeader) -> WcsTransform {
        WcsTransform::from_header(header).unwrap()
    }

    fn options(method: ResampleMethod, border_crop: usize, area_rule: Option<AreaRule>) -> AlignOptions {
        AlignOptions { method, reference: ReferencePolicy::Coarsest, border_crop, area_rule, per_actual_pixel: false }
    }

    fn history(header: &HduHeader) -> String {
        header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.as_str()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn align_companions_match_the_science_transfer_for_interpolation_prefilter_and_pixel_areas() {
        for (method, ratio, distorted) in [
            (ResampleMethod::Bilinear, 1.0, false),
            (ResampleMethod::Bicubic, 1.0, false),
            (ResampleMethod::Exact, 2.0, false),
            (ResampleMethod::Bicubic, 2.0, false),
            (ResampleMethod::Exact, 2.0, true),
        ] {
            let header = grid_header((8, 8), (4.5, 4.5), (150.0, 2.0), 0.04, 0.0, "Jy");
            let header = if distorted { with_sip(header) } else { header };
            let target = grid_header((8, 8), (4.0, 4.0), (150.0, 2.0), 0.04 * ratio, 0.0, "Jy");
            let target = if distorted { with_sip(target) } else { target };
            let (source_wcs, target_wcs) = (wcs_of(&header), wcs_of(&target));
            let grid = AlignGrid { label: "reference", header: &target, wcs: &target_wcs, rows: 8, cols: 8 };
            let science = Array2::from_shape_fn((8, 8), |(y, x)| (y * 8 + x) as f32 - 20.0);
            let error = Array2::from_shape_fn((8, 8), |(y, x)| 0.5 + (y + x) as f32 / 8.0);
            let input = AlignInput { image: &science, header: &header, wcs: &source_wcs };
            let opts = AlignOptions { per_actual_pixel: distorted, ..options(method, 1, None) };
            let plan = plan_alignment(&input, &grid, &opts).unwrap();
            let result = align_image_with_companions(&input, &grid, &opts, &plan, AlignCompanions { err: Some(&error), dq: None, no_data_dq: 513 }).unwrap();
            let original = align_image(&input, &grid, &opts, &plan);
            assert!(result.aligned.image.iter().zip(&original.image).all(|(a, b)| a.to_bits() == b.to_bits()), "{method:?} SCI changed");
            let mut expected = Array2::<f64>::zeros(original.image.dim());
            for ((sy, sx), &sigma) in error.indexed_iter() {
                let mut impulse = Array2::zeros(science.dim());
                impulse[[sy, sx]] = 1.0;
                let response = align_image(&AlignInput { image: &impulse, ..input }, &grid, &opts, &plan);
                for (sum, &weight) in expected.iter_mut().zip(&response.image) {
                    *sum += f64::from(weight).powi(2) * f64::from(sigma).powi(2);
                }
            }
            for ((actual, expected), &value) in result.err.as_ref().unwrap().iter().zip(&expected).zip(&original.image) {
                if value.is_finite() {
                    let tolerance = 128.0 * f64::from(f32::EPSILON) * expected.max(1.0);
                    assert!((f64::from(*actual).powi(2) - expected).abs() <= tolerance, "{method:?} ratio {ratio} distorted {distorted}: {} vs {expected}", f64::from(*actual).powi(2));
                } else {
                    assert!(actual.is_nan());
                }
            }
        }
    }

    #[test]
    fn align_companions_do_not_turn_invalid_err_into_valid_uncertainty_or_mask_science() {
        let header = grid_header((8, 8), (4.5, 4.5), (150.0, 2.0), 0.04, 0.0, "Jy");
        let target = grid_header((8, 8), (4.0, 4.0), (150.0, 2.0), 0.08, 0.0, "Jy");
        let (source_wcs, target_wcs) = (wcs_of(&header), wcs_of(&target));
        let grid = AlignGrid { label: "reference", header: &target, wcs: &target_wcs, rows: 8, cols: 8 };
        let science = Array2::from_elem((8, 8), 3.0);
        let input = AlignInput { image: &science, header: &header, wcs: &source_wcs };
        for method in [ResampleMethod::Bilinear, ResampleMethod::Bicubic, ResampleMethod::Exact] {
            let opts = options(method, 0, None);
            let plan = plan_alignment(&input, &grid, &opts).unwrap();
            for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -2.0] {
                let error = Array2::from_elem((8, 8), invalid);
                let result = align_image_with_companions(&input, &grid, &opts, &plan, AlignCompanions { err: Some(&error), dq: None, no_data_dq: 513 }).unwrap();
                assert!(result.err.unwrap().iter().all(|value| value.is_nan()), "{method:?}: {invalid}");
                assert!(result.aligned.image[[3, 3]].is_finite(), "ERR must not mask SCI");
            }
            let result = align_image_with_companions(&input, &grid, &opts, &plan, AlignCompanions { err: None, dq: None, no_data_dq: 513 }).unwrap();
            assert!(result.err.is_none());
            let wrong_shape = Array2::from_elem((1, 1), 1.0);
            assert!(matches!(align_image_with_companions(&input, &grid, &opts, &plan, AlignCompanions { err: Some(&wrong_shape), dq: None, no_data_dq: 513 }), Err(AlignError::CompanionShape { plane: "ERR", .. })));
        }
    }

    fn textured(rows: usize, cols: usize, seed: u64) -> Array2<f32> {
        let mut state = seed;
        Array2::from_shape_fn((rows, cols), |_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as f64 / (1u64 << 31) as f64) as f32 * 50.0 + 10.0
        })
    }

    fn gaussian_scene(rows: usize, cols: usize, stars: &[(f64, f64, f64)], sigma: f64) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            stars
                .iter()
                .map(|&(sx, sy, flux)| {
                    let r2 = (x as f64 - sx).powi(2) + (y as f64 - sy).powi(2);
                    flux * (-r2 / (2.0 * sigma * sigma)).exp() / (2.0 * std::f64::consts::PI * sigma * sigma)
                })
                .sum::<f64>() as f32
        })
    }

    fn finite_sum(image: &Array2<f32>) -> f64 {
        image.iter().filter(|v| v.is_finite()).map(|v| f64::from(*v)).sum()
    }

    const SIP_CURVATURE: f64 = 4e-4;

    fn with_sip(mut header: HduHeader) -> HduHeader {
        header.set("CTYPE1", "RA---TAN-SIP".to_string());
        header.set("CTYPE2", "DEC--TAN-SIP".to_string());
        for (key, value) in [("A_ORDER", 2.0), ("B_ORDER", 2.0), ("A_2_0", SIP_CURVATURE), ("B_0_2", SIP_CURVATURE)] {
            header.set_f64(key, value);
        }
        header
    }

    fn sip_relative_area(header: &HduHeader, x: usize, y: usize) -> f64 {
        let u = x as f64 + 1.0 - header.get_f64("CRPIX1").unwrap();
        let v = y as f64 + 1.0 - header.get_f64("CRPIX2").unwrap();
        (1.0 + 2.0 * SIP_CURVATURE * u) * (1.0 + 2.0 * SIP_CURVATURE * v)
    }

    fn aperture_sum(image: &Array2<f32>, cx: f64, cy: f64, radius: f64) -> f64 {
        image
            .indexed_iter()
            .filter(|((y, x), v)| v.is_finite() && (*x as f64 - cx).hypot(*y as f64 - cy) <= radius)
            .map(|(_, v)| f64::from(*v))
            .sum()
    }

    fn scipy_bilinear(image: &Array2<f32>, sx: f64, sy: f64) -> f32 {
        let (rows, cols) = image.dim();
        if sx < -0.5 || sy < -0.5 || sx > cols as f64 - 0.5 || sy > rows as f64 - 0.5 {
            return f32::NAN;
        }
        let (x, y) = (sx.clamp(0.0, (cols - 1) as f64), sy.clamp(0.0, (rows - 1) as f64));
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(cols - 1), (y0 + 1).min(rows - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let mut value = 0.0f64;
        for (yy, xx, w) in [(y0, x0, (1.0 - fx) * (1.0 - fy)), (y0, x1, fx * (1.0 - fy)), (y1, x0, (1.0 - fx) * fy), (y1, x1, fx * fy)] {
            if w != 0.0 {
                value += f64::from(image[[yy, xx]]) * w;
            }
        }
        value as f32
    }

    fn percentile_range(values: &[f32]) -> f64 {
        let mut sorted: Vec<f64> = values.iter().filter(|v| v.is_finite()).map(|v| f64::from(*v)).collect();
        sorted.sort_by(f64::total_cmp);
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
        at(0.999) - at(0.001)
    }

    #[test]
    fn bilinear_mode_matches_reproject_interp_on_tan_grid() {
        let input_header = grid_header((64, 64), (32.5, 32.5), (150.0, 2.0), 0.04, 7.0, "MJy/sr");
        let target_header = grid_header((50, 50), (25.5, 25.5), (150.00001, 2.00002), 0.05, -3.0, "MJy/sr");
        let (input_wcs, target_wcs) = (wcs_of(&input_header), wcs_of(&target_header));
        let grid = AlignGrid { label: "target", header: &target_header, wcs: &target_wcs, rows: 50, cols: 50 };
        let blobs = [(20.0, 18.0, 4000.0), (41.5, 30.2, 2500.0), (33.3, 47.9, 3000.0), (8.0, 52.0, 1500.0)];
        let mut scene = gaussian_scene(64, 64, &blobs, 3.0);
        scene.indexed_iter_mut().for_each(|((y, x), v)| *v += 10.0 + 0.05 * x as f32 + 0.03 * y as f32);
        scene[[22, 40]] = f32::NAN;
        let input = AlignInput { image: &scene, header: &input_header, wcs: &input_wcs };
        let bilinear = options(ResampleMethod::Bilinear, 0, None);
        let plan = plan_alignment(&input, &grid, &bilinear).unwrap();
        assert!(plan.reprojected);
        let aligned = align_image(&input, &grid, &bilinear, &plan);
        assert_eq!(aligned.report.method, Some(ResampleMethod::Bilinear));
        assert_eq!(aligned.report.prefilter_k, 1);

        let mut reference = Array2::from_elem((50, 50), f32::NAN);
        let mut safe = Array2::from_elem((50, 50), false);
        for ((y, x), value) in reference.indexed_iter_mut() {
            let sky = target_wcs.pixel_to_world(x as f64, y as f64);
            let (sx, sy) = input_wcs.world_to_pixel(sky.ra, sky.dec);
            *value = scipy_bilinear(&scene, sx, sy);
            let edge = [sx + 0.5, 63.5 - sx, sy + 0.5, 63.5 - sy].into_iter().map(f64::abs).fold(f64::INFINITY, f64::min);
            let fraction = [sx.fract().abs(), 1.0 - sx.fract().abs(), sy.fract().abs(), 1.0 - sy.fract().abs()].into_iter().fold(f64::INFINITY, f64::min);
            safe[[y, x]] = edge > 1e-3 && fraction > 1e-3;
        }
        let range = percentile_range(reference.as_slice().unwrap());
        let (mut compared, mut nan) = (0, 0);
        for ((y, x), expected) in reference.indexed_iter() {
            let got = aligned.image[[y, x]];
            if safe[[y, x]] {
                assert_eq!(got.is_nan(), expected.is_nan(), "({x}, {y}): {got} against {expected}");
            }
            if got.is_finite() && expected.is_finite() && (2..48).contains(&x) && (2..48).contains(&y) {
                assert!(f64::from((got - expected).abs()) <= 1e-3 * range, "({x}, {y}): {got} against {expected}");
                compared += 1;
            }
            nan += usize::from(expected.is_nan());
        }
        assert!(compared > 1500 && nan > 4, "{compared} compared, {nan} NaN");

        let ramp = Array2::from_shape_fn((64, 64), |(y, x)| (5.0 + 0.2 * x as f64 - 0.1 * y as f64) as f32);
        let ramp_input = AlignInput { image: &ramp, ..input };
        let ramp_out = align_image(&ramp_input, &grid, &bilinear, &plan_alignment(&ramp_input, &grid, &bilinear).unwrap());
        for ((y, x), got) in ramp_out.image.indexed_iter().filter(|(_, v)| v.is_finite()) {
            let sky = target_wcs.pixel_to_world(x as f64, y as f64);
            let (sx, sy) = input_wcs.world_to_pixel(sky.ra, sky.dec);
            let expected = 5.0 + 0.2 * sx.clamp(0.0, 63.0) - 0.1 * sy.clamp(0.0, 63.0);
            assert!((f64::from(*got) - expected).abs() < 1e-3, "({x}, {y}): {got} against {expected}");
        }
    }

    #[test]
    fn exact_mode_conserves_total_flux_on_2x_downsample() {
        let input_header = grid_header((96, 96), (48.5, 48.5), (150.0, 2.0), 0.02, 20.0, "Jy");
        let target_header = grid_header((64, 64), (32.5, 32.5), (150.0, 2.0), 0.04, 0.0, "Jy");
        let (input_wcs, target_wcs) = (wcs_of(&input_header), wcs_of(&target_header));
        let grid = AlignGrid { label: "F444W", header: &target_header, wcs: &target_wcs, rows: 64, cols: 64 };
        let stars = [(20.3, 22.7, 120.0), (70.1, 25.4, 60.0), (47.6, 48.2, 200.0), (25.9, 71.3, 90.0), (72.8, 74.5, 150.0), (50.2, 15.0, 75.0)];
        let scene = gaussian_scene(96, 96, &stars, 1.2);
        let input = AlignInput { image: &scene, header: &input_header, wcs: &input_wcs };
        let exact = options(ResampleMethod::Exact, 0, None);
        let plan = plan_alignment(&input, &grid, &exact).unwrap();
        let area = plan.area.clone().expect("a reprojected Jy image carries an area correction");
        assert_eq!(area.rule, AreaRule::Scale);
        assert!((area.data_factor - 4.0).abs() < 1e-9, "{area:?}");
        let aligned = align_image(&input, &grid, &exact, &plan);
        let (total_in, total_out) = (finite_sum(&scene), finite_sum(&aligned.image));
        assert!(((total_out - total_in) / total_in).abs() < 1e-6, "{total_out} Jy against {total_in} Jy");
        assert!((aligned.report.flux_ratio.unwrap() - 1.0).abs() < 1e-6, "{:?}", aligned.report.flux_ratio);
        for &(x, y, _) in &stars {
            let sky = input_wcs.pixel_to_world(x, y);
            let (ox, oy) = target_wcs.world_to_pixel(sky.ra, sky.dec);
            let inside = |image: &Array2<f32>, cx: f64, cy: f64, r: f64| -> f64 {
                image.indexed_iter().filter(|((py, px), _)| (*px as f64 - cx).hypot(*py as f64 - cy) <= r).map(|(_, v)| f64::from(*v)).sum()
            };
            let (before, after) = (inside(&scene, x, y, 10.0), inside(&aligned.image, ox, oy, 5.0));
            assert!(((after - before) / before).abs() < 1e-4, "star at ({x}, {y}): {after} against {before}");
        }

        let flat = Array2::from_elem((96, 96), 7.0f32);
        let flat_header = grid_header((96, 96), (48.5, 48.5), (150.0, 2.0), 0.02, 20.0, "MJy/sr");
        let flat_input = AlignInput { image: &flat, header: &flat_header, wcs: &input_wcs };
        let flat_out = align_image(&flat_input, &grid, &exact, &plan_alignment(&flat_input, &grid, &exact).unwrap());
        assert!(flat_out.image.iter().filter(|v| v.is_finite()).all(|v| (v - 7.0).abs() < 1e-5));
        assert!(flat_out.report.coverage > 0.5 && flat_out.report.coverage < 1.0, "{}", flat_out.report.coverage);

        let (same, footprint, report) = reproject_exact_to_grid(&scene, &input_wcs, &input_wcs, 96, 96);
        assert!(same.iter().zip(scene.iter()).all(|(a, b)| (a - b).abs() <= 1e-6 * b.abs().max(1e-3)));
        assert!(footprint.iter().all(|f| (f - 1.0).abs() < 1e-6));
        assert!((report.coverage - 1.0).abs() < 1e-12);

        let (_, edge_footprint, _) = reproject_exact_to_grid(&flat, &input_wcs, &target_wcs, 64, 64);
        let covered: f64 = edge_footprint.iter().map(|f| f64::from(*f)).sum();
        assert!((covered - 96.0 * 96.0 / 4.0).abs() < 1e-3, "{covered} output pixels of footprint");
        assert!(edge_footprint.iter().any(|f| *f > 0.0 && *f < 0.99));
    }

    #[test]
    fn coarsest_reference_is_default_for_cubes() {
        let defaults = AlignOptions::default();
        assert_eq!(defaults.reference, ReferencePolicy::Coarsest);
        assert_eq!(
            (defaults.method, defaults.border_crop, defaults.area_rule, defaults.per_actual_pixel),
            (ResampleMethod::Exact, 1, None, false)
        );
        let candidate = |path, scale_arcsec, filter| ReferenceCandidate { path, scale_arcsec, filter };
        let candidates = [
            candidate("d1_HST_F814W.fits", 0.04, Some("F814W")),
            candidate("d1_JWST_F150W.fits", 0.02, Some("F150W")),
            candidate("d1_JWST_F277W.fits", 0.0401, Some("F277W")),
            candidate("d1_JWST_F444W.fits", 0.04, Some("F444W")),
            candidate("blank.fits", f64::NAN, None),
            candidate("d1_JWST_F200W.fits", 0.02, Some("F200W")),
        ];
        assert_eq!(choose_reference(&candidates, &defaults.reference), Ok(ReferenceChoice { index: 3, rule: "coarsest" }));
        assert_eq!(choose_reference(&candidates, &ReferencePolicy::Finest), Ok(ReferenceChoice { index: 1, rule: "finest" }));
        let by_filter = ReferencePolicy::Filter { filter: "f444w".into() };
        assert_eq!(choose_reference(&candidates, &by_filter), Ok(ReferenceChoice { index: 3, rule: "filter" }));
        let by_path = ReferencePolicy::Path { path: "d1_JWST_F200W.fits".into() };
        assert_eq!(choose_reference(&candidates, &by_path), Ok(ReferenceChoice { index: 5, rule: "path" }));
        let missing = ReferencePolicy::Filter { filter: "F999W".into() };
        assert_eq!(choose_reference(&candidates, &missing), Err(AlignError::NoReference("F999W".into())));
        let twice = [candidates[0], candidate("other_F814W.fits", 0.05, Some("F814W"))];
        let f814w = ReferencePolicy::Filter { filter: "F814W".into() };
        assert_eq!(choose_reference(&twice, &f814w), Err(AlignError::AmbiguousReference { filter: "F814W".into(), count: 2 }));
        assert_eq!(choose_reference(&[candidates[4]], &defaults.reference), Err(AlignError::NoScale));
        assert_eq!(choose_reference(&[], &defaults.reference), Err(AlignError::Empty));
        let parsed: AlignOptions = serde_json::from_str(r#"{"method":"bilinear","reference":{"policy":"filter","filter":"F444W"},"border_crop":0,"area_rule":"lightstack"}"#).unwrap();
        assert_eq!(parsed.reference, ReferencePolicy::Filter { filter: "F444W".into() });
        assert_eq!((parsed.method, parsed.border_crop, parsed.area_rule), (ResampleMethod::Bilinear, 0, Some(AreaRule::Lightstack)));
        let partial: AlignOptions = serde_json::from_str(r#"{"method":"bilinear"}"#).unwrap();
        assert_eq!(partial, AlignOptions { method: ResampleMethod::Bilinear, ..AlignOptions::default() });
        let actual: AlignOptions = serde_json::from_str(r#"{"per_actual_pixel":true}"#).unwrap();
        assert_eq!(actual, AlignOptions { per_actual_pixel: true, ..AlignOptions::default() });
    }

    #[test]
    fn already_aligned_input_is_copied_not_resampled() {
        let header = grid_header((48, 40), (24.5, 20.5), (150.0, 2.0), 0.04, 0.0, "MJy/sr");
        let wcs = wcs_of(&header);
        let grid = AlignGrid { label: "F444W", header: &header, wcs: &wcs, rows: 40, cols: 48 };
        let mut image = textured(40, 48, 3);
        image[[7, 11]] = f32::NAN;
        let mut own = header.clone();
        own.set("FILTER", "F277W".to_string());
        let input = AlignInput { image: &image, header: &own, wcs: &wcs };
        let copy = options(ResampleMethod::Bilinear, 0, None);
        let plan = plan_alignment(&input, &grid, &copy).unwrap();
        assert_eq!(plan, AlignPlan { reprojected: false, area: None, warnings: vec![] });
        let aligned = align_image(&input, &grid, &copy, &plan);
        assert!(aligned.image.iter().zip(image.iter()).all(|(a, b)| a.to_bits() == b.to_bits()));
        assert_eq!((aligned.report.reprojected, aligned.report.method, aligned.report.area.clone()), (false, None, None));
        assert!((aligned.report.flux_ratio.unwrap() - 1.0).abs() < 1e-15);
        assert_eq!(aligned.header.get_f64("CRPIX1"), Some(24.5));
        assert_eq!(aligned.header.get("FILTER"), Some("F277W"));
        assert!(history(&aligned.header).contains("copied without resampling"), "{}", history(&aligned.header));

        let mut encoded = header.clone();
        for key in ["CD1_1", "CD1_2", "CD2_1", "CD2_2"] {
            encoded.remove(key);
        }
        encoded.set_f64("CDELT1", -0.04 * ARCSEC);
        encoded.set_f64("CDELT2", 0.04 * ARCSEC);
        encoded.set_f64("PC1_1", 1.0);
        encoded.set_f64("PC2_2", 1.0);
        assert!(same_grid(&wcs_of(&encoded), (40, 48), &wcs, (40, 48)), "a PC+CDELT spelling of the same grid needs no resampling");
        let mut shifted = header.clone();
        shifted.set_f64("CRPIX1", 24.75);
        assert!(!same_grid(&wcs_of(&shifted), (40, 48), &wcs, (40, 48)));
        assert!(!same_grid(&wcs, (40, 47), &wcs, (40, 48)));
        let shifted_wcs = wcs_of(&shifted);
        let moved = AlignInput { image: &image, header: &shifted, wcs: &shifted_wcs };
        assert!(plan_alignment(&moved, &grid, &copy).unwrap().reprojected);
    }

    #[test]
    fn border_crop_shifts_crpix_by_n() {
        let header = grid_header((48, 40), (24.5, 20.5), (150.0, 2.0), 0.04, 0.0, "MJy/sr");
        let wcs = wcs_of(&header);
        let grid = AlignGrid { label: "F444W", header: &header, wcs: &wcs, rows: 40, cols: 48 };
        let image = textured(40, 48, 9);
        let input = AlignInput { image: &image, header: &header, wcs: &wcs };
        let cropped = options(ResampleMethod::Exact, 2, None);
        let aligned = align_image(&input, &grid, &cropped, &plan_alignment(&input, &grid, &cropped).unwrap());
        assert_eq!(aligned.image.dim(), (36, 44));
        assert_eq!(aligned.image, image.slice(ndarray::s![2..38, 2..46]).to_owned());
        assert_eq!((aligned.header.get_f64("CRPIX1"), aligned.header.get_f64("CRPIX2")), (Some(22.5), Some(18.5)));
        assert_eq!((aligned.header.get_i64("NAXIS1"), aligned.header.get_i64("NAXIS2")), (Some(44), Some(36)));
        assert_eq!((aligned.report.rows, aligned.report.cols, aligned.report.border_crop), (36, 44, 2));
        let (a, b) = (wcs_of(&aligned.header).pixel_to_world(0.0, 0.0), wcs.pixel_to_world(2.0, 2.0));
        assert!((a.ra - b.ra).abs() < 1e-12 && (a.dec - b.dec).abs() < 1e-12);
        assert!(history(&aligned.header).contains("Cropped 2 pixels from each border"), "{}", history(&aligned.header));

        let input_header = grid_header((64, 64), (32.5, 32.5), (150.0, 2.0), 0.03, 5.0, "MJy/sr");
        let input_wcs = wcs_of(&input_header);
        let other = textured(64, 64, 4);
        let resampled = AlignInput { image: &other, header: &input_header, wcs: &input_wcs };
        let one = options(ResampleMethod::Bilinear, 1, None);
        let out = align_image(&resampled, &grid, &one, &plan_alignment(&resampled, &grid, &one).unwrap());
        assert_eq!(out.image.dim(), (38, 46));
        assert_eq!((out.header.get_f64("CRPIX1"), out.header.get_f64("CRPIX2")), (Some(23.5), Some(19.5)));
        let too_wide = options(ResampleMethod::Exact, 20, None);
        assert_eq!(
            plan_alignment(&input, &grid, &too_wide),
            Err(AlignError::BorderCrop { border_crop: 20, rows: 40, cols: 48 })
        );
    }

    #[test]
    fn flat_fielded_per_pixel_values_on_a_distorted_input_are_not_divided_by_the_pixel_area_map() {
        let target_header = grid_header((40, 40), (20.5, 20.5), (150.0, 2.0), 0.08, 0.0, "Jy");
        let target_wcs = wcs_of(&target_header);
        let grid = AlignGrid { label: "F444W", header: &target_header, wcs: &target_wcs, rows: 40, cols: 40 };
        let stars = [(12.0, 12.0, 100.0), (50.0, 14.0, 100.0), (32.0, 32.0, 100.0), (14.0, 50.0, 100.0), (52.0, 52.0, 100.0)];
        let defaults = AlignOptions { border_crop: 0, ..AlignOptions::default() };
        for (bunit, calibration) in [("Jy", None), ("ELECTRONS/S", Some(("PHOTFNU", 1e-7)))] {
            let mut header = with_sip(grid_header((64, 64), (32.5, 32.5), (150.0, 2.0), 0.04, 0.0, bunit));
            if let Some((key, value)) = calibration {
                header.set_f64(key, value);
            }
            let wcs = wcs_of(&header);
            let factor = |aligned: &AlignedImage| calibration.map_or(1.0, |(key, _)| aligned.header.get_f64(key).unwrap() / header.get_f64(key).unwrap());

            let flat = Array2::from_elem((64, 64), 7.0f32);
            let input = AlignInput { image: &flat, header: &header, wcs: &wcs };
            let plan = plan_alignment(&input, &grid, &defaults).unwrap();
            let expected = 7.0 * plan.area.as_ref().unwrap().data_factor;
            let aligned = align_image(&input, &grid, &defaults, &plan);
            let worst = aligned.image.iter().filter(|v| v.is_finite()).map(|v| (f64::from(*v) / expected - 1.0).abs()).fold(0.0, f64::max);
            assert!(worst < 1e-5, "{bunit}: a flat-fielded image is {worst} away from flat after alignment");
            assert!(!aligned.report.pixel_area_corrected, "{bunit}");
            assert!(!history(&aligned.header).contains("relative pixel area"), "{}", history(&aligned.header));

            let physical = gaussian_scene(64, 64, &stars, 1.5);
            let per_nominal_pixel = Array2::from_shape_fn((64, 64), |(y, x)| (f64::from(physical[[y, x]]) / sip_relative_area(&header, x, y)) as f32);
            let input = AlignInput { image: &per_nominal_pixel, header: &header, wcs: &wcs };
            let aligned = align_image(&input, &grid, &defaults, &plan_alignment(&input, &grid, &defaults).unwrap());
            for &(x, y, _) in &stars {
                let sky = wcs.pixel_to_world(x, y);
                let (ox, oy) = target_wcs.world_to_pixel(sky.ra, sky.dec);
                let (before, after) = (aperture_sum(&physical, x, y, 8.0), aperture_sum(&aligned.image, ox, oy, 4.0) * factor(&aligned));
                assert!(((after - before) / before).abs() < 2e-3, "{bunit} star at ({x}, {y}): {after} against {before}");
            }
        }
    }

    #[test]
    fn per_actual_pixel_values_on_a_distorted_input_keep_each_source_flux_when_asked() {
        let target_header = grid_header((40, 40), (20.5, 20.5), (150.0, 2.0), 0.08, 0.0, "Jy");
        let target_wcs = wcs_of(&target_header);
        let grid = AlignGrid { label: "F444W", header: &target_header, wcs: &target_wcs, rows: 40, cols: 40 };
        let stars = [(12.0, 12.0, 100.0), (50.0, 14.0, 100.0), (32.0, 32.0, 100.0), (14.0, 50.0, 100.0), (52.0, 52.0, 100.0)];
        let scene = gaussian_scene(64, 64, &stars, 1.5);
        let distorted = |bunit: &str, extra: &[(&str, f64)]| {
            let mut header = with_sip(grid_header((64, 64), (32.5, 32.5), (150.0, 2.0), 0.04, 0.0, bunit));
            for (key, value) in extra {
                header.set_f64(key, *value);
            }
            header
        };
        let exact = AlignOptions { per_actual_pixel: true, ..options(ResampleMethod::Exact, 0, None) };
        for (bunit, extra, calibration) in [("Jy", vec![], None), ("ELECTRONS/S", vec![("PHOTFNU", 1e-7)], Some("PHOTFNU"))] {
            let header = distorted(bunit, &extra);
            let wcs = wcs_of(&header);
            let input = AlignInput { image: &scene, header: &header, wcs: &wcs };
            let aligned = align_image(&input, &grid, &exact, &plan_alignment(&input, &grid, &exact).unwrap());
            assert!(aligned.report.pixel_area_corrected, "{bunit}");
            assert!(history(&aligned.header).contains("relative pixel area"), "{}", history(&aligned.header));
            let factor = calibration.map_or(1.0, |key| aligned.header.get_f64(key).unwrap() / header.get_f64(key).unwrap());
            for &(x, y, _) in &stars {
                let sky = wcs.pixel_to_world(x, y);
                let (ox, oy) = target_wcs.world_to_pixel(sky.ra, sky.dec);
                let (before, after) = (aperture_sum(&scene, x, y, 8.0), aperture_sum(&aligned.image, ox, oy, 4.0) * factor);
                assert!(((after - before) / before).abs() < 2e-3, "{bunit} star at ({x}, {y}): {after} against {before}");
            }
        }

        let flat = Array2::from_elem((64, 64), 7.0f32);
        let header = distorted("MJy/sr", &[]);
        let wcs = wcs_of(&header);
        let input = AlignInput { image: &flat, header: &header, wcs: &wcs };
        let aligned = align_image(&input, &grid, &exact, &plan_alignment(&input, &grid, &exact).unwrap());
        assert!(aligned.image.iter().filter(|v| v.is_finite()).all(|v| (v - 7.0).abs() < 1e-5), "surface brightness is not flattened");
        assert!(!aligned.report.pixel_area_corrected);
    }

    #[test]
    fn a_distorted_reference_grid_receives_flux_per_nominal_pixel_unless_actual_pixels_are_asked() {
        let target_header = with_sip(grid_header((40, 40), (20.5, 20.5), (150.0, 2.0), 0.08, 0.0, "Jy"));
        let target_wcs = wcs_of(&target_header);
        let grid = AlignGrid { label: "SIP", header: &target_header, wcs: &target_wcs, rows: 40, cols: 40 };
        let input_header = grid_header((64, 64), (32.5, 32.5), (150.0, 2.0), 0.04, 0.0, "Jy");
        let input_wcs = wcs_of(&input_header);
        let stars = [(10.0, 10.0, 100.0), (54.0, 12.0, 100.0), (32.0, 32.0, 100.0), (12.0, 53.0, 100.0), (53.0, 54.0, 100.0)];
        let scene = gaussian_scene(64, 64, &stars, 1.5);
        let input = AlignInput { image: &scene, header: &input_header, wcs: &input_wcs };
        let nominal = options(ResampleMethod::Exact, 0, None);
        let actual = AlignOptions { per_actual_pixel: true, ..nominal.clone() };
        let per_nominal_pixel = align_image(&input, &grid, &nominal, &plan_alignment(&input, &grid, &nominal).unwrap());
        let per_actual_pixel = align_image(&input, &grid, &actual, &plan_alignment(&input, &grid, &actual).unwrap());
        assert!(!per_nominal_pixel.report.pixel_area_corrected);
        assert!(per_actual_pixel.report.pixel_area_corrected);
        let physical = Array2::from_shape_fn((40, 40), |(y, x)| {
            (f64::from(per_nominal_pixel.image[[y, x]]) * sip_relative_area(&target_header, x, y)) as f32
        });
        for &(x, y, _) in &stars {
            let sky = input_wcs.pixel_to_world(x, y);
            let (ox, oy) = target_wcs.world_to_pixel(sky.ra, sky.dec);
            let before = aperture_sum(&scene, x, y, 8.0);
            for (convention, after) in [("nominal", aperture_sum(&physical, ox, oy, 4.0)), ("actual", aperture_sum(&per_actual_pixel.image, ox, oy, 4.0))] {
                assert!(((after - before) / before).abs() < 2e-3, "per {convention} pixel, star at ({x}, {y}): {after} against {before}");
            }
        }
    }

    #[test]
    fn bicubic_onto_a_coarser_grid_warns_when_its_prefilter_leaves_the_input_wcs() {
        use std::sync::Arc;

        use crate::core::astrometry::gwcs::test_support::fixture_pipeline;
        use crate::core::astrometry::gwcs::{GwcsOrigin, GwcsSource};

        let bicubic = options(ResampleMethod::Bicubic, 0, None);
        let scene = textured(64, 64, 5);
        let run = |header: &HduHeader, wcs: &WcsTransform, target: &HduHeader| {
            let target_wcs = wcs_of(target);
            let grid = AlignGrid { label: "F444W", header: target, wcs: &target_wcs, rows: 24, cols: 24 };
            let input = AlignInput { image: &scene, header, wcs };
            align_image(&input, &grid, &bicubic, &plan_alignment(&input, &grid, &bicubic).unwrap()).report
        };
        let input_header = grid_header((64, 64), (32.5, 32.5), (150.0, 2.0), 0.04, 0.0, "MJy/sr");
        let input_wcs = wcs_of(&input_header);
        let target = grid_header((24, 24), (12.5, 12.5), (150.0, 2.0), 0.1, 0.0, "MJy/sr");
        let prefiltered = run(&input_header, &input_wcs, &target);
        assert_eq!(prefiltered.prefilter_k, 2);
        assert!(prefiltered.warnings.is_empty(), "{:?}", prefiltered.warnings);

        let mut bare = HduHeader::empty();
        bare.set("BUNIT", "MJy/sr".to_string());
        let skipped = run(&bare, &input_wcs, &target);
        assert_eq!(skipped.prefilter_k, 1);
        assert!(skipped.warnings.iter().any(|w| w.contains("without its block-average prefilter")), "{:?}", skipped.warnings);

        let source = Arc::new(GwcsSource {
            pipeline: fixture_pipeline("wcs_jwst_nircam_cal300.asdf"),
            origin: GwcsOrigin::AsdfTree { key: "wcs".into() },
            wcsinfo_sip_max_err_px: None,
            wcsinfo_sip_inv_err_px: None,
        });
        let mut dims = HduHeader::empty();
        dims.set("NAXIS1", "64".to_string());
        dims.set("NAXIS2", "64".to_string());
        let gwcs = WcsTransform::from_gwcs(source, &dims).unwrap();
        let (crpix1, crpix2, crval1, crval2, cd, _) = gwcs.raw_params();
        let mut twin = dims.clone();
        twin.set("BUNIT", "MJy/sr".to_string());
        twin.set("CTYPE1", "RA---TAN".to_string());
        twin.set("CTYPE2", "DEC--TAN".to_string());
        for (key, value) in [("CRPIX1", crpix1), ("CRPIX2", crpix2), ("CRVAL1", crval1), ("CRVAL2", crval2), ("CD1_1", cd[0][0]), ("CD1_2", cd[0][1]), ("CD2_1", cd[1][0]), ("CD2_2", cd[1][1])] {
            twin.set_f64(key, value);
        }
        let centre = gwcs.pixel_to_world(31.5, 31.5);
        let coarse = grid_header((24, 24), (12.5, 12.5), (centre.ra, centre.dec), 2.5 * gwcs.pixel_scale_arcsec(), 0.0, "MJy/sr");
        let approximated = run(&twin, &gwcs, &coarse);
        assert_eq!((approximated.wcs_kind, approximated.prefilter_k), (WcsKind::Gwcs, 2));
        assert!(approximated.warnings.iter().any(|w| w.contains("FITS header WCS") && w.contains("gWCS")), "{:?}", approximated.warnings);
    }

    #[test]
    fn the_area_rule_follows_the_unit_and_flags_lightstack_on_surface_brightness() {
        let target_header = grid_header((16, 16), (8.5, 8.5), (150.0, 2.0), 0.04, 0.0, "MJy/sr");
        let target_wcs = wcs_of(&target_header);
        let grid = AlignGrid { label: "F444W", header: &target_header, wcs: &target_wcs, rows: 16, cols: 16 };
        let scene = gaussian_scene(32, 32, &[(15.2, 16.1, 100.0), (8.0, 9.0, 40.0)], 1.5);
        let exact = options(ResampleMethod::Exact, 0, None);
        let run = |bunit: &str, rule: Option<AreaRule>| {
            let mut header = grid_header((32, 32), (16.5, 16.5), (150.0, 2.0), 0.02, 0.0, bunit);
            header.set_f64("PIXAR_SR", (0.02 * std::f64::consts::PI / 648_000.0).powi(2));
            let wcs = wcs_of(&header);
            let input = AlignInput { image: &scene, header: &header, wcs: &wcs };
            let opts = AlignOptions { area_rule: rule, ..exact.clone() };
            plan_alignment(&input, &grid, &opts).map(|plan| (plan.clone(), align_image(&input, &grid, &opts, &plan)))
        };

        let (plan, jy) = run("Jy", None).unwrap();
        let area = plan.area.as_ref().unwrap();
        assert!(area.rule == AreaRule::Scale && (area.data_factor - 4.0).abs() < 1e-9, "{area:?}");
        assert!((finite_sum(&jy.image) / finite_sum(&scene) - 1.0).abs() < 1e-6);
        assert!((jy.report.flux_ratio.unwrap() - 1.0).abs() < 1e-6, "{:?}", jy.report.flux_ratio);

        let (plan, mjy) = run("MJy/sr", None).unwrap();
        assert_eq!(plan.area.as_ref().map(|a| (a.rule, a.data_factor)), Some((AreaRule::Keep, 1.0)));
        assert!((mjy.header.get_f64("PIXAR_SR").unwrap() / (0.04 * std::f64::consts::PI / 648_000.0).powi(2) - 1.0).abs() < 1e-9);
        assert!((mjy.report.flux_ratio.unwrap() - 1.0).abs() < 1e-6, "{:?}", mjy.report.flux_ratio);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);

        assert_eq!(run("ADU", None).unwrap_err(), AlignError::Unit(UnitError::UnknownUnit { bunit: Some("ADU".into()) }));
        let (_, adu) = run("ADU", Some(AreaRule::Lightstack)).unwrap();
        assert!((finite_sum(&adu.image) / finite_sum(&scene) - 1.0).abs() < 1e-6);
        assert!((adu.report.flux_ratio.unwrap() - 1.0).abs() < 1e-6);

        let (plan, bright) = run("MJy/sr", Some(AreaRule::Lightstack)).unwrap();
        assert!((bright.report.flux_ratio.unwrap() - 4.0).abs() < 1e-5, "{:?}", bright.report.flux_ratio);
        assert!(plan.warnings.iter().any(|w| w.contains("LIGHTSTACK") && w.contains("surface brightness")), "{:?}", plan.warnings);
    }
}
