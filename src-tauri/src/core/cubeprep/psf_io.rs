use std::collections::{HashMap, HashSet};
use std::path::Path;

use ndarray::{s, Array2};
use serde::{Deserialize, Serialize};

use crate::core::astrometry::wcs::read_effective_cd;
use crate::core::cubeprep::cube::set_text;
use crate::core::imaging::asinh::{asinh_refusal, asinh_scale};
use crate::core::imaging::push_history;
use crate::infra::fits::reader::{parse_header_at, ParsedHdu};
use crate::math::bspline::{affine_transform, interpolate_line, shift, zoomed_len, SplineMode};
use crate::math::gauss_newton::fit_least_squares;
use crate::types::header::HduHeader;

pub const STPSF_PREFERENCE: [&str; 4] = ["DET_DIST", "DET_SAMP", "OVERDIST", "OVERSAMP"];
pub const CENTROID_THRESHOLD: f64 = 1e-4;
pub const CENTRE_TOLERANCE_PX: f64 = 0.02;
pub const EDGE_ENERGY_WARNING: f64 = 0.01;
pub const SCALE_RTOL: f64 = 1e-6;
pub const GAUSSIAN_SIGMA_TO_FWHM: f64 = 2.354_820_045_030_949_3;
const GAUSSIAN_FIT_ITERATIONS: usize = 200;
const NEGATIVE_INPUT_WARNING: &str = "percent of the pixels are negative";
const TRUNCATED_INPUT_WARNING: &str = "may be truncated";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PsfError {
    #[error("{label}: {message}")]
    Read { label: String, message: String },
    #[error("{0}: no HDU holds a 2D image")]
    NoImage(String),
    #[error("{label}: HDU {hdu} holds {planes} planes; a PSF must be one 2D plane")]
    Cube { label: String, hdu: usize, planes: usize },
    #[error("{0}: no pixel scale (no PIXSCALE, PIXELSCL or celestial WCS card); give pixel_scale_arcsec")]
    NoScale(String),
    #[error("the pixel scale must be a positive number of arcsec, got {0}")]
    InvalidScale(f64),
    #[error("the PSF is empty")]
    Empty,
    #[error("the PSF sums to {0}, so it cannot be normalized")]
    NotNormalizable(f64),
    #[error("the {method} resampling integrates pixel areas and cannot upsample from {source_arcsec} to {target_arcsec} arcsec per pixel")]
    Upsampling { method: &'static str, source_arcsec: f64, target_arcsec: f64 },
    #[error("the target shape {rows}x{cols} is empty")]
    EmptyTarget { rows: usize, cols: usize },
    #[error("no PSF has the reference filter {0}")]
    NoReference(String),
    #[error("the reference filter {filter} matches {count} PSFs")]
    AmbiguousReference { filter: String, count: usize },
}

pub const ASINH_PLANE_REFUSAL: &str =
    "the pixels are asinh-scaled (BSOFTEN and BOFFSET cards); Cube Prep reads PSF, kernel and noise planes as stored, so give a file with linear pixels";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleSource {
    Given,
    Pixscale,
    Pixelscl,
    Wcs,
}

impl ScaleSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Given => "given",
            Self::Pixscale => "PIXSCALE",
            Self::Pixelscl => "PIXELSCL",
            Self::Wcs => "WCS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PsfResampleMethod {
    #[default]
    Area,
    Lightstack,
}

impl PsfResampleMethod {
    pub fn label(self) -> &'static str {
        match self {
            Self::Area => "area",
            Self::Lightstack => "lightstack",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedPsf {
    pub data: Array2<f64>,
    pub header: HduHeader,
    pub hdu: usize,
    pub extname: Option<String>,
    pub pixel_scale_arcsec: f64,
    pub scale_source: ScaleSource,
    pub oversampling: Option<f64>,
    pub filter: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GaussianFit {
    pub amplitude: f64,
    pub x: f64,
    pub y: f64,
    pub sigma_x: f64,
    pub sigma_y: f64,
    pub theta: f64,
    pub fwhm_px: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PsfDiagnostics {
    pub rows: usize,
    pub cols: usize,
    pub odd: bool,
    pub non_finite_pixels: usize,
    pub sum: f64,
    pub negative_fraction: f64,
    pub peak_row: usize,
    pub peak_col: usize,
    pub centroid_x: f64,
    pub centroid_y: f64,
    pub offset_x_px: f64,
    pub offset_y_px: f64,
    pub edge_energy_fraction: f64,
    pub fwhm_px: Option<f64>,
    pub fwhm_arcsec: Option<f64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LightstackPadding {
    pub padded: Array2<f64>,
    pub source_pixel_scale_arcsec: f64,
    pub target_pixel_scale_arcsec: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LightstackResampling {
    pub resampled: Array2<f64>,
    pub clipped_negative_fraction: f64,
    pub clipped_negative_flux_fraction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PsfPrepareOptions {
    pub target_pixel_scale_arcsec: Option<f64>,
    pub target_shape: Option<(usize, usize)>,
    pub method: PsfResampleMethod,
    pub recentre: bool,
}

impl Default for PsfPrepareOptions {
    fn default() -> Self {
        Self { target_pixel_scale_arcsec: None, target_shape: None, method: PsfResampleMethod::Area, recentre: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResampleRecord {
    pub method: PsfResampleMethod,
    pub source_pixel_scale_arcsec: f64,
    pub target_pixel_scale_arcsec: f64,
    pub factor: f64,
    pub padded_rows: Option<usize>,
    pub padded_cols: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PreparedPsf {
    pub data: Array2<f64>,
    pub pixel_scale_arcsec: f64,
    pub before: PsfDiagnostics,
    pub after: PsfDiagnostics,
    pub padded_to_odd: bool,
    pub recentred_by_px: Option<[f64; 2]>,
    pub resampled: Option<ResampleRecord>,
    pub clipped_negative_fraction: f64,
    pub clipped_negative_flux_fraction: f64,
    pub warnings: Vec<String>,
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header.get(key).map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn axis_len(header: &HduHeader, axis: i64) -> usize {
    header.get_i64(&format!("NAXIS{axis}")).and_then(|len| usize::try_from(len).ok()).unwrap_or(0)
}

fn read_error(label: &str, message: impl Into<String>) -> PsfError {
    PsfError::Read { label: label.to_string(), message: message.into() }
}

pub fn image_hdus(bytes: &[u8], label: &str) -> Result<Vec<(usize, ParsedHdu)>, PsfError> {
    let mut found = Vec::new();
    let (mut offset, mut index) = (0usize, 0usize);
    while offset < bytes.len() {
        let parsed = match parse_header_at(bytes, offset) {
            Ok(parsed) => parsed,
            Err(error) if index == 0 => return Err(read_error(label, format!("{error:#}"))),
            Err(_) => break,
        };
        let next = parsed.next_hdu_offset;
        let header = &parsed.header;
        let is_image = index == 0 || header.get("XTENSION").is_some_and(|kind| kind.trim().eq_ignore_ascii_case("IMAGE"));
        if is_image && header.get_i64("NAXIS").unwrap_or(0) >= 2 && axis_len(header, 1) > 0 && axis_len(header, 2) > 0 {
            found.push((index, parsed));
        }
        if next <= offset {
            break;
        }
        offset = next;
        index += 1;
    }
    Ok(found)
}

fn decode_value(chunk: &[u8], bitpix: i64, blank: Option<i64>) -> Option<f64> {
    let integer = |value: i64| (blank != Some(value)).then_some(value as f64);
    match bitpix {
        8 => integer(chunk[0] as i64),
        16 => integer(i16::from_be_bytes([chunk[0], chunk[1]]) as i64),
        32 => integer(i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as i64),
        64 => integer(i64::from_be_bytes(chunk.try_into().expect("8-byte chunk"))),
        -32 => Some(f32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64),
        _ => Some(f64::from_be_bytes(chunk.try_into().expect("8-byte chunk"))),
    }
}

pub fn decode_planes_f64(bytes: &[u8], parsed: &ParsedHdu, label: &str) -> Result<Vec<Array2<f64>>, PsfError> {
    let header = &parsed.header;
    if asinh_scale(header).is_some() || asinh_refusal(header).is_some() {
        return Err(read_error(label, ASINH_PLANE_REFUSAL));
    }
    let bitpix = header.get_i64("BITPIX").ok_or_else(|| read_error(label, "BITPIX is missing"))?;
    let width = match bitpix {
        8 => 1,
        16 => 2,
        32 | -32 => 4,
        64 | -64 => 8,
        other => return Err(read_error(label, format!("BITPIX {other} is not a FITS image type"))),
    };
    let (cols, rows) = (axis_len(header, 1), axis_len(header, 2));
    let naxis = header.get_i64("NAXIS").unwrap_or(0);
    let planes = (3..=naxis).try_fold(1usize, |count, axis| count.checked_mul(axis_len(header, axis)));
    let count = planes.and_then(|planes| planes.checked_mul(rows)).and_then(|n| n.checked_mul(cols));
    let raw = count
        .and_then(|count| count.checked_mul(width))
        .and_then(|size| parsed.data_start.checked_add(size))
        .and_then(|end| bytes.get(parsed.data_start..end))
        .ok_or_else(|| read_error(label, "the data unit ends past the end of the file"))?;
    let bscale = header.get_f64("BSCALE").unwrap_or(1.0);
    let bzero = header.get_f64("BZERO").unwrap_or(0.0);
    let blank = header.get_i64("BLANK");
    let values: Vec<f64> =
        raw.chunks_exact(width).map(|chunk| decode_value(chunk, bitpix, blank).map_or(f64::NAN, |v| v * bscale + bzero)).collect();
    Ok(values
        .chunks_exact((rows * cols).max(1))
        .map(|plane| Array2::from_shape_vec((rows, cols), plane.to_vec()).expect("plane length matches its shape"))
        .collect())
}

fn keyword_scale(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|scale| scale.is_finite() && *scale > 0.0)
}

pub(crate) fn wcs_axis_scales(header: &HduHeader) -> Option<(f64, f64)> {
    card_text(header, "CTYPE1")?;
    let cd = read_effective_cd(header).ok()?;
    Some((cd[0][0].hypot(cd[1][0]) * 3600.0, cd[0][1].hypot(cd[1][1]) * 3600.0))
}

pub fn wcs_scale(header: &HduHeader) -> Option<f64> {
    let (scale_x, scale_y) = wcs_axis_scales(header)?;
    let usable = |scale: f64| scale.is_finite() && scale > 0.0;
    let rectangular = usable(scale_x) && usable(scale_y) && differs(scale_y, scale_x);
    Some(if rectangular { (scale_x + scale_y) / 2.0 } else { scale_x }).filter(|scale| usable(*scale))
}

fn differs(a: f64, b: f64) -> bool {
    (a / b - 1.0).abs() > SCALE_RTOL
}

pub(crate) fn pixel_scale(header: &HduHeader, given: Option<f64>, warnings: &mut Vec<String>) -> Result<Option<(f64, ScaleSource)>, PsfError> {
    if let Some(scale) = given {
        if !(scale.is_finite() && scale > 0.0) {
            return Err(PsfError::InvalidScale(scale));
        }
        return Ok(Some((scale, ScaleSource::Given)));
    }
    let pixscale = keyword_scale(header, "PIXSCALE");
    let pixelscl = keyword_scale(header, "PIXELSCL");
    let wcs = wcs_scale(header);
    let chosen = pixscale.map(|s| (s, ScaleSource::Pixscale)).or(pixelscl.map(|s| (s, ScaleSource::Pixelscl))).or(wcs.map(|s| (s, ScaleSource::Wcs)));
    if let Some((scale, source)) = chosen {
        for (other, name) in [(pixelscl, "PIXELSCL"), (wcs, "the WCS")] {
            if let Some(other) = other.filter(|&other| differs(other, scale)) {
                warnings.push(format!("{name} gives {other} arcsec per pixel; the {} scale {scale} is used", source.label()));
            }
        }
    }
    Ok(chosen)
}

pub fn read_psf(bytes: &[u8], label: &str, pixel_scale_arcsec: Option<f64>) -> Result<LoadedPsf, PsfError> {
    let hdus = image_hdus(bytes, label)?;
    let extname = |parsed: &ParsedHdu| card_text(&parsed.header, "EXTNAME").map(|name| name.to_ascii_uppercase());
    let (hdu, parsed) = STPSF_PREFERENCE
        .iter()
        .find_map(|wanted| hdus.iter().find(|(_, parsed)| extname(parsed).as_deref() == Some(*wanted)))
        .or_else(|| hdus.first())
        .ok_or_else(|| PsfError::NoImage(label.to_string()))?;
    let mut planes = decode_planes_f64(bytes, parsed, label)?;
    if planes.len() != 1 {
        return Err(PsfError::Cube { label: label.to_string(), hdu: *hdu, planes: planes.len() });
    }
    let header = parsed.header.clone();
    let mut warnings = Vec::new();
    let (pixel_scale_arcsec, scale_source) =
        pixel_scale(&header, pixel_scale_arcsec, &mut warnings)?.ok_or_else(|| PsfError::NoScale(label.to_string()))?;
    Ok(LoadedPsf {
        data: planes.remove(0),
        hdu: *hdu,
        extname: extname(parsed),
        pixel_scale_arcsec,
        scale_source,
        oversampling: header.get_f64("OVERSAMP"),
        filter: card_text(&header, "FILTER"),
        header,
        warnings,
    })
}

pub fn load_psf(path: &Path, pixel_scale_arcsec: Option<f64>) -> Result<LoadedPsf, PsfError> {
    let label = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|error| PsfError::Read { label: label.clone(), message: error.to_string() })?;
    read_psf(&bytes, &label, pixel_scale_arcsec)
}

pub fn centroid_weighted(psf: &Array2<f64>, threshold: f64) -> (f64, f64) {
    let peak = psf.iter().copied().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
    let floor = threshold * peak;
    let (mut total, mut sum_x, mut sum_y) = (0.0, 0.0, 0.0);
    for ((r, c), &value) in psf.indexed_iter() {
        if value.is_finite() && value >= floor {
            total += value;
            sum_x += c as f64 * value;
            sum_y += r as f64 * value;
        }
    }
    (sum_x / total, sum_y / total)
}

fn peak_of(psf: &Array2<f64>) -> Option<(usize, usize, f64)> {
    let mut best: Option<(usize, usize, f64)> = None;
    for ((r, c), &value) in psf.indexed_iter() {
        if value.is_finite() && best.is_none_or(|(_, _, peak)| value > peak) {
            best = Some((r, c, value));
        }
    }
    best
}

fn gaussian_quadratic(dx: f64, dy: f64, p: &[f64], jac: &mut [f64]) -> f64 {
    let (amplitude, a, b, c) = (p[0], p[3], p[4], p[5]);
    let shape = (-(a * dx * dx + b * dx * dy + c * dy * dy)).exp();
    let value = amplitude * shape;
    jac[0] = shape;
    jac[1] = value * (2.0 * a * dx + b * dy);
    jac[2] = value * (b * dx + 2.0 * c * dy);
    jac[3] = -value * dx * dx;
    jac[4] = -value * dx * dy;
    jac[5] = -value * dy * dy;
    value
}

pub fn fit_gaussian(psf: &Array2<f64>) -> Option<GaussianFit> {
    let data = psf.mapv(|v| if v.is_finite() { v } else { 0.0 });
    let (rows, cols) = data.dim();
    let (yc, xc, peak) = peak_of(&data)?;
    if !(peak > 0.0) {
        return None;
    }
    let above = data.iter().filter(|&&v| v > peak / 2.0).count();
    let fwhm0 = 2.0 * (above as f64 / std::f64::consts::PI).sqrt();
    let half = 5.0f64.max(3.0 * fwhm0).ceil() as usize;
    let (r0, r1) = (yc.saturating_sub(half), (yc + half).min(rows - 1));
    let (c0, c1) = (xc.saturating_sub(half), (xc + half).min(cols - 1));
    let width = c1 - c0 + 1;
    let samples = (r1 - r0 + 1) * width;
    let index: Vec<f64> = (0..samples).map(|i| i as f64).collect();
    let values: Vec<f64> = (0..samples).map(|i| data[[r0 + i / width, c0 + i % width]]).collect();
    let weights = vec![1.0; samples];
    let inverse_variance = 0.5 / (fwhm0 / GAUSSIAN_SIGMA_TO_FWHM).powi(2);
    let start = [peak, xc as f64, yc as f64, inverse_variance, 0.0, inverse_variance];
    let model = |at: f64, p: &[f64], jac: &mut [f64]| {
        let i = at as usize;
        gaussian_quadratic((c0 + i % width) as f64 - p[1], (r0 + i / width) as f64 - p[2], p, jac)
    };
    let fit = fit_least_squares(&index, &values, &weights, &start, &[f64::NEG_INFINITY; 6], &[f64::INFINITY; 6], GAUSSIAN_FIT_ITERATIONS, model).ok()?;
    let [amplitude, x, y, a, b, c] = fit.params[..] else { return None };
    let determinant = 4.0 * a * c - b * b;
    if !(a > 0.0 && c > 0.0 && determinant > 0.0) {
        return None;
    }
    let (sxx, syy, sxy) = (2.0 * c / determinant, 2.0 * a / determinant, -b / determinant);
    let mean = 0.5 * (sxx + syy);
    let spread = (0.25 * (sxx - syy).powi(2) + sxy * sxy).sqrt();
    let (major, minor) = ((mean + spread).sqrt(), (mean - spread).max(0.0).sqrt());
    Some(GaussianFit {
        amplitude,
        x,
        y,
        sigma_x: major,
        sigma_y: minor,
        theta: 0.5 * (2.0 * sxy).atan2(sxx - syy),
        fwhm_px: 0.5 * (major + minor) * GAUSSIAN_SIGMA_TO_FWHM,
    })
}

pub fn diagnose_psf(psf: &Array2<f64>, pixel_scale_arcsec: Option<f64>) -> PsfDiagnostics {
    let (rows, cols) = psf.dim();
    let total = (rows * cols).max(1) as f64;
    let non_finite_pixels = psf.iter().filter(|v| !v.is_finite()).count();
    let sum: f64 = psf.iter().filter(|v| v.is_finite()).sum();
    let negative_fraction = psf.iter().filter(|&&v| v < 0.0).count() as f64 / total;
    let (peak_row, peak_col, _) = peak_of(psf).unwrap_or((0, 0, f64::NAN));
    let (centroid_x, centroid_y) = centroid_weighted(psf, CENTROID_THRESHOLD);
    let offset_x_px = centroid_x - (cols as f64 - 1.0) / 2.0;
    let offset_y_px = centroid_y - (rows as f64 - 1.0) / 2.0;
    let frame: f64 = psf
        .indexed_iter()
        .filter(|((r, c), v)| v.is_finite() && (*r == 0 || *c == 0 || *r + 1 == rows || *c + 1 == cols))
        .map(|(_, v)| *v)
        .sum();
    let edge_energy_fraction = frame / sum;
    let fwhm_px = fit_gaussian(psf).map(|fit| fit.fwhm_px);
    let fwhm_arcsec = fwhm_px.zip(pixel_scale_arcsec).map(|(fwhm, scale)| fwhm * scale);
    let odd = rows % 2 == 1 && cols % 2 == 1;
    let mut warnings = Vec::new();
    if !odd {
        warnings.push(format!("the PSF is {cols}x{rows}; matching kernels need odd sizes so the PSF has a central pixel"));
    }
    if non_finite_pixels > 0 {
        warnings.push(format!("{non_finite_pixels} pixels are not finite"));
    }
    if (sum - 1.0).abs() > 1e-6 {
        warnings.push(format!("the PSF sums to {sum}, not 1"));
    }
    if offset_x_px.abs().max(offset_y_px.abs()) > CENTRE_TOLERANCE_PX {
        warnings.push(format!("the PSF centroid is off centre by ({offset_x_px:.3}, {offset_y_px:.3}) px"));
    }
    if edge_energy_fraction > EDGE_ENERGY_WARNING {
        warnings.push(format!(
            "{:.2} percent of the flux lies in the outermost pixel frame at the edge; the PSF may be truncated",
            100.0 * edge_energy_fraction
        ));
    }
    if negative_fraction > 0.0 {
        warnings.push(format!("{:.2} percent of the pixels are negative", 100.0 * negative_fraction));
    }
    if fwhm_px.is_none() {
        warnings.push("the Gaussian FWHM fit failed".to_string());
    }
    PsfDiagnostics {
        rows,
        cols,
        odd,
        non_finite_pixels,
        sum,
        negative_fraction,
        peak_row,
        peak_col,
        centroid_x,
        centroid_y,
        offset_x_px,
        offset_y_px,
        edge_energy_fraction,
        fwhm_px,
        fwhm_arcsec,
        warnings,
    }
}

pub fn make_odd(psf: &Array2<f64>) -> Array2<f64> {
    let (rows, cols) = psf.dim();
    let mut padded = Array2::zeros((rows + 1 - rows % 2, cols + 1 - cols % 2));
    padded.slice_mut(s![..rows, ..cols]).assign(psf);
    padded
}

pub fn normalize_psf(psf: &Array2<f64>) -> Result<Array2<f64>, PsfError> {
    let cleaned = psf.mapv(|v| if v.is_finite() && v > 0.0 { v } else { 0.0 });
    let total: f64 = cleaned.sum();
    if !(total.is_finite() && total > 0.0) {
        return Err(PsfError::NotNormalizable(total));
    }
    Ok(cleaned.mapv(|v| v / total))
}

pub fn recentre_psf(psf: &Array2<f64>) -> (Array2<f64>, [f64; 2]) {
    let (rows, cols) = psf.dim();
    let (x, y) = centroid_weighted(psf, CENTROID_THRESHOLD);
    let offset = [(cols as f64 - 1.0) / 2.0 - x, (rows as f64 - 1.0) / 2.0 - y];
    (shift(psf, [offset[1], offset[0]], SplineMode::Constant(0.0)), offset)
}

fn checked_scale(scale: f64) -> Result<f64, PsfError> {
    if scale.is_finite() && scale > 0.0 {
        Ok(scale)
    } else {
        Err(PsfError::InvalidScale(scale))
    }
}

fn nansum(values: &Array2<f64>) -> f64 {
    values.iter().filter(|v| !v.is_nan()).sum()
}

fn negative_share(values: &Array2<f64>) -> (f64, f64) {
    let (mut negative_pixels, mut negative_flux, mut positive_flux) = (0usize, 0.0, 0.0);
    for &v in values {
        if v < 0.0 {
            negative_pixels += 1;
            negative_flux -= v;
        } else if v > 0.0 {
            positive_flux += v;
        }
    }
    let fraction = negative_pixels as f64 / values.len().max(1) as f64;
    (fraction, if positive_flux > 0.0 { negative_flux / positive_flux } else { 0.0 })
}

fn odd_up(len: usize) -> usize {
    len + 1 - len % 2
}

pub fn lightstack_pad(
    psf: &Array2<f64>,
    source_pixel_scale_arcsec: f64,
    target_pixel_scale_arcsec: f64,
    target_shape: (usize, usize),
) -> Result<LightstackPadding, PsfError> {
    let (ny, nx) = psf.dim();
    if ny == 0 || nx == 0 {
        return Err(PsfError::Empty);
    }
    let source = checked_scale(source_pixel_scale_arcsec)?;
    let target = checked_scale(target_pixel_scale_arcsec)?;
    let required = |len: usize| odd_up((len as f64 * target / source).ceil() as usize);
    let (padded_ny, padded_nx) = (odd_up(required(target_shape.0).max(ny)), odd_up(required(target_shape.1).max(nx)));
    let mut padded = Array2::zeros((padded_ny, padded_nx));
    let (y0, x0) = (padded_ny / 2 - ny / 2, padded_nx / 2 - nx / 2);
    padded.slice_mut(s![y0..y0 + ny, x0..x0 + nx]).assign(psf);
    let total = nansum(&padded);
    if !(total.is_finite() && total > 0.0) {
        return Err(PsfError::NotNormalizable(total));
    }
    padded.mapv_inplace(|v| v / total);
    Ok(LightstackPadding { padded, source_pixel_scale_arcsec: source, target_pixel_scale_arcsec: target })
}

pub fn lightstack_resample_measured(
    psf: &Array2<f64>,
    source_pixel_scale_arcsec: f64,
    target_pixel_scale_arcsec: f64,
    output_shape: Option<(usize, usize)>,
) -> Result<LightstackResampling, PsfError> {
    let (ny, nx) = psf.dim();
    let scale = checked_scale(source_pixel_scale_arcsec)? / checked_scale(target_pixel_scale_arcsec)?;
    let shape = output_shape.unwrap_or((zoomed_len(ny, scale), zoomed_len(nx, scale)));
    if shape.0 == 0 || shape.1 == 0 {
        return Err(PsfError::EmptyTarget { rows: shape.0, cols: shape.1 });
    }
    let step = 1.0 / scale;
    let centre = |len: usize| (len as f64 - 1.0) / 2.0;
    let offset = [centre(ny) - step * centre(shape.0), centre(nx) - step * centre(shape.1)];
    let mut resampled = affine_transform(psf, [[step, 0.0], [0.0, step]], offset, shape, SplineMode::Constant(0.0));
    let (clipped_negative_fraction, clipped_negative_flux_fraction) = negative_share(&resampled);
    resampled.mapv_inplace(|v| if v < 0.0 { 0.0 } else { v });
    let total = nansum(&resampled);
    if !(total.is_finite() && total > 0.0) {
        return Err(PsfError::NotNormalizable(total));
    }
    resampled.mapv_inplace(|v| v / total);
    Ok(LightstackResampling { resampled, clipped_negative_fraction, clipped_negative_flux_fraction })
}

pub fn lightstack_resample(
    psf: &Array2<f64>,
    source_pixel_scale_arcsec: f64,
    target_pixel_scale_arcsec: f64,
    output_shape: Option<(usize, usize)>,
) -> Result<Array2<f64>, PsfError> {
    lightstack_resample_measured(psf, source_pixel_scale_arcsec, target_pixel_scale_arcsec, output_shape).map(|measured| measured.resampled)
}

fn edge_coordinates(input_len: usize, output_len: usize, factor: f64) -> Vec<f64> {
    let centre_in = (input_len as f64 - 1.0) / 2.0;
    let centre_out = (output_len as f64 - 1.0) / 2.0;
    (0..output_len)
        .flat_map(|o| {
            let centre = centre_in + (o as f64 - centre_out) * factor;
            [centre - factor / 2.0, centre + factor / 2.0]
        })
        .map(|edge| (edge + 0.5).clamp(0.0, input_len as f64))
        .collect()
}

fn rebin_line(values: impl Iterator<Item = f64>, coordinates: &[f64]) -> Vec<f64> {
    let mut running = 0.0;
    let cumulative: Vec<f64> = std::iter::once(0.0)
        .chain(values.map(|value| {
            running += if value.is_finite() { value } else { 0.0 };
            running
        }))
        .collect();
    interpolate_line(&cumulative, coordinates, SplineMode::Nearest).chunks_exact(2).map(|edges| edges[1] - edges[0]).collect()
}

pub fn area_resample(
    psf: &Array2<f64>,
    source_pixel_scale_arcsec: f64,
    target_pixel_scale_arcsec: f64,
    output_shape: (usize, usize),
) -> Result<Array2<f64>, PsfError> {
    let source = checked_scale(source_pixel_scale_arcsec)?;
    let target = checked_scale(target_pixel_scale_arcsec)?;
    let factor = target / source;
    if factor < 1.0 - SCALE_RTOL {
        return Err(PsfError::Upsampling { method: PsfResampleMethod::Area.label(), source_arcsec: source, target_arcsec: target });
    }
    if output_shape.0 == 0 || output_shape.1 == 0 {
        return Err(PsfError::EmptyTarget { rows: output_shape.0, cols: output_shape.1 });
    }
    let (rows, cols) = psf.dim();
    let row_edges = edge_coordinates(rows, output_shape.0, factor);
    let col_edges = edge_coordinates(cols, output_shape.1, factor);
    let columns: Vec<Vec<f64>> = (0..cols).map(|c| rebin_line(psf.column(c).iter().copied(), &row_edges)).collect();
    let rebinned: Vec<Vec<f64>> = (0..output_shape.0).map(|o| rebin_line(columns.iter().map(|column| column[o]), &col_edges)).collect();
    let integrated = Array2::from_shape_fn(output_shape, |(o, p)| rebinned[o][p]);
    let total: f64 = integrated.sum();
    if !(total.is_finite() && total > 0.0) {
        return Err(PsfError::NotNormalizable(total));
    }
    Ok(integrated.mapv(|v| v / total))
}

fn same_scale(source: f64, target: f64) -> bool {
    !differs(target, source)
}

pub fn default_target_shape(shape: (usize, usize), source_pixel_scale_arcsec: f64, target_pixel_scale_arcsec: f64) -> (usize, usize) {
    if same_scale(source_pixel_scale_arcsec, target_pixel_scale_arcsec) {
        return shape;
    }
    let ratio = source_pixel_scale_arcsec / target_pixel_scale_arcsec;
    let axis = |len: usize| odd_up(zoomed_len(len, ratio).max(1));
    (axis(shape.0), axis(shape.1))
}

pub fn prepare_psf(psf: &LoadedPsf, options: &PsfPrepareOptions) -> Result<PreparedPsf, PsfError> {
    if psf.data.is_empty() {
        return Err(PsfError::Empty);
    }
    let source = checked_scale(psf.pixel_scale_arcsec)?;
    let target = checked_scale(options.target_pixel_scale_arcsec.unwrap_or(source))?;
    let before = diagnose_psf(&psf.data, Some(source));
    let mut warnings = psf.warnings.clone();
    if before.non_finite_pixels > 0 {
        warnings.push(format!("{} non-finite pixels were set to 0", before.non_finite_pixels));
    }
    let mut data = psf.data.mapv(|v| if v.is_finite() { v } else { 0.0 });
    let padded_to_odd = !before.odd;
    if padded_to_odd {
        data = make_odd(&data);
        warnings.push(format!("padded from {}x{} to {}x{} with zeros after the data, as LIGHTSTACK's make_odd", before.cols, before.rows, data.ncols(), data.nrows()));
    }
    let mut recentred_by_px = None;
    if options.recentre {
        let (x, y) = centroid_weighted(&data, CENTROID_THRESHOLD);
        let (dx, dy) = ((data.ncols() as f64 - 1.0) / 2.0 - x, (data.nrows() as f64 - 1.0) / 2.0 - y);
        if dx.abs().max(dy.abs()) > CENTRE_TOLERANCE_PX {
            let (shifted, offset) = recentre_psf(&data);
            data = shifted;
            recentred_by_px = Some(offset);
        }
    }
    let target_shape = options.target_shape.unwrap_or_else(|| default_target_shape(data.dim(), source, target));
    if target_shape.0 == 0 || target_shape.1 == 0 {
        return Err(PsfError::EmptyTarget { rows: target_shape.0, cols: target_shape.1 });
    }
    let record = |method: PsfResampleMethod, padded: Option<(usize, usize)>| ResampleRecord {
        method,
        source_pixel_scale_arcsec: source,
        target_pixel_scale_arcsec: target,
        factor: target / source,
        padded_rows: padded.map(|p| p.0),
        padded_cols: padded.map(|p| p.1),
    };
    let (resampled_data, resampled, (clipped_negative_fraction, clipped_negative_flux_fraction)) = if same_scale(source, target) && target_shape == data.dim() {
        let share = negative_share(&data);
        (data, None, share)
    } else if same_scale(source, target) || options.method == PsfResampleMethod::Area {
        let integrated = area_resample(&data, source, target, target_shape)?;
        let share = negative_share(&integrated);
        (integrated, Some(record(PsfResampleMethod::Area, None)), share)
    } else {
        let padding = lightstack_pad(&data, source, target, target_shape)?;
        let padded = padding.padded.dim();
        let measured = lightstack_resample_measured(&padding.padded, source, target, Some(target_shape))?;
        let share = (measured.clipped_negative_fraction, measured.clipped_negative_flux_fraction);
        (measured.resampled, Some(record(PsfResampleMethod::Lightstack, Some(padded))), share)
    };
    if clipped_negative_fraction > 0.0 {
        warnings.push(format!(
            "{:.2} percent of the pixels were negative and set to 0; their absolute flux is {:.3} percent of the kept flux",
            100.0 * clipped_negative_fraction,
            100.0 * clipped_negative_flux_fraction
        ));
    }
    let data = normalize_psf(&resampled_data)?;
    let after = diagnose_psf(&data, Some(target));
    let hidden_kinds: &[&str] = match &resampled {
        Some(_) => &[NEGATIVE_INPUT_WARNING, TRUNCATED_INPUT_WARNING],
        None => &[TRUNCATED_INPUT_WARNING],
    };
    for &kind in hidden_kinds {
        if after.warnings.iter().any(|warning| warning.contains(kind)) {
            continue;
        }
        if let Some(hidden) = before.warnings.iter().find(|warning| warning.contains(kind)) {
            warnings.push(format!("input PSF: {hidden}"));
        }
    }
    warnings.extend(after.warnings.iter().map(|warning| format!("prepared PSF: {warning}")));
    Ok(PreparedPsf {
        data,
        pixel_scale_arcsec: target,
        before,
        after,
        padded_to_odd,
        recentred_by_px,
        resampled,
        clipped_negative_fraction,
        clipped_negative_flux_fraction,
        warnings,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PsfSetOptions {
    pub target_pixel_scale_arcsec: Option<f64>,
    pub reference: Option<String>,
    pub method: PsfResampleMethod,
    pub recentre: bool,
}

impl Default for PsfSetOptions {
    fn default() -> Self {
        Self { target_pixel_scale_arcsec: None, reference: None, method: PsfResampleMethod::Area, recentre: true }
    }
}

#[derive(Debug, Clone)]
pub struct PsfSet {
    pub prepared: Vec<PreparedPsf>,
    pub reference_index: Option<usize>,
    pub target_pixel_scale_arcsec: Option<f64>,
    pub target_shape: Option<(usize, usize)>,
}

pub fn prepare_psf_set(psfs: &[LoadedPsf], options: &PsfSetOptions) -> Result<PsfSet, PsfError> {
    let reference_index = match options.reference.as_deref().map(str::trim) {
        None => None,
        Some(wanted) => {
            let matches: Vec<usize> = psfs
                .iter()
                .enumerate()
                .filter(|(_, psf)| psf.filter.as_deref().is_some_and(|filter| filter.trim().eq_ignore_ascii_case(wanted)))
                .map(|(index, _)| index)
                .collect();
            match matches.as_slice() {
                [index] => Some(*index),
                [] => return Err(PsfError::NoReference(wanted.to_string())),
                _ => return Err(PsfError::AmbiguousReference { filter: wanted.to_string(), count: matches.len() }),
            }
        }
    };
    let target_pixel_scale_arcsec = options.target_pixel_scale_arcsec.or(reference_index.map(|index| psfs[index].pixel_scale_arcsec));
    let single = |psf: &LoadedPsf, target_shape: Option<(usize, usize)>| {
        prepare_psf(psf, &PsfPrepareOptions { target_pixel_scale_arcsec, target_shape, method: options.method, recentre: options.recentre })
    };
    let reference = reference_index.map(|index| single(&psfs[index], None)).transpose()?;
    let target_shape = reference.as_ref().map(|prepared| prepared.data.dim());
    let mut prepared = psfs
        .iter()
        .enumerate()
        .map(|(index, psf)| match &reference {
            Some(prepared) if Some(index) == reference_index => Ok(prepared.clone()),
            _ => single(psf, target_shape),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !options.recentre {
        let reference_at = reference.as_ref().zip(reference_index);
        for (index, plane) in prepared.iter_mut().enumerate() {
            let note = match reference_at {
                Some((_, at)) if at == index => None,
                Some((target, at)) => {
                    let (dx, dy) = (target.after.offset_x_px - plane.after.offset_x_px, target.after.offset_y_px - plane.after.offset_y_px);
                    let filter = psfs[at].filter.as_deref().map_or("", str::trim);
                    (dx.abs().max(dy.abs()) > CENTRE_TOLERANCE_PX).then(|| {
                        format!("prepared PSF: recentre is off, so the kernel matching this PSF to the reference PSF {filter} will shift this plane by about ({dx:.3}, {dy:.3}) px, the difference of their centroid offsets")
                    })
                }
                None => (plane.after.offset_x_px.abs().max(plane.after.offset_y_px.abs()) > CENTRE_TOLERANCE_PX)
                    .then(|| "prepared PSF: recentre is off, so against a centred target PSF the matching kernel will shift this plane by the opposite of its centroid offset".to_string()),
            };
            plane.warnings.extend(note);
        }
    }
    Ok(PsfSet { prepared, reference_index, target_pixel_scale_arcsec, target_shape })
}

pub fn psf_header_base() -> HduHeader {
    HduHeader { cards: Vec::new(), index: HashMap::new(), string_keys: Some(HashSet::new()) }
}

pub fn set_local_psf_wcs(header: &mut HduHeader, rows: usize, cols: usize, scale: f64) {
    for (key, value) in [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CUNIT1", "deg"), ("CUNIT2", "deg")] {
        set_text(header, key, value);
    }
    for (key, value) in [
        ("CRVAL1", 0.0),
        ("CRVAL2", 0.0),
        ("CRPIX1", (cols as f64 + 1.0) / 2.0),
        ("CRPIX2", (rows as f64 + 1.0) / 2.0),
        ("CD1_1", -scale / 3600.0),
        ("CD1_2", 0.0),
        ("CD2_1", 0.0),
        ("CD2_2", scale / 3600.0),
    ] {
        header.set_f64(key, value);
    }
}

pub fn prepared_psf_header(psf: &LoadedPsf, prepared: &PreparedPsf, source_name: &str) -> HduHeader {
    let (rows, cols) = prepared.data.dim();
    let scale = prepared.pixel_scale_arcsec;
    let mut header = psf_header_base();
    set_local_psf_wcs(&mut header, rows, cols, scale);
    set_text(&mut header, "BUNIT", "normalized");
    if let Some(filter) = &psf.filter {
        set_text(&mut header, "FILTER", filter);
    }
    header.set_f64("PIXSCALE", scale);
    if let Some(fwhm) = prepared.after.fwhm_arcsec {
        header.set_f64("PSFFWHM", fwhm);
    }
    let extname = psf.extname.as_deref().map(|name| format!(" ({name})")).unwrap_or_default();
    push_history(
        &mut header,
        &format!("PSF prepared from {source_name} HDU {}{extname} at {} arcsec per pixel from {}", psf.hdu, psf.pixel_scale_arcsec, psf.scale_source.label()),
    );
    if prepared.padded_to_odd {
        push_history(&mut header, "Padded with zeros after the data to an odd size");
    }
    if let Some([dx, dy]) = prepared.recentred_by_px {
        push_history(&mut header, &format!("Shifted by ({dx:.4}, {dy:.4}) px with a cubic B-spline to centre its flux-weighted centroid"));
    }
    if let Some(record) = &prepared.resampled {
        let how = match record.method {
            PsfResampleMethod::Area => "by integrating a cubic spline of the cumulative flux over each output pixel",
            PsfResampleMethod::Lightstack => "with the LIGHTSTACK 0.4.0 padding and cubic B-spline resampling",
        };
        push_history(&mut header, &format!("Resampled from {} to {} arcsec per pixel {how}", record.source_pixel_scale_arcsec, record.target_pixel_scale_arcsec));
    }
    push_history(&mut header, "Non-finite and negative pixels set to 0 and the PSF normalized to unit sum");
    header
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cubeprep::kernel::{build_kernel, KernelOptions};
    use crate::infra::fits::reader::test_fixtures::write_raw_hdus;
    use crate::math::bspline::scipy_golden::{array2, assert_close, load, number, vector};
    use serde_json::Value;

    fn gaussian(size: usize, sigma: f64, centre: (f64, f64)) -> Array2<f64> {
        Array2::from_shape_fn((size, size), |(r, c)| {
            let (dy, dx) = (r as f64 - centre.1, c as f64 - centre.0);
            (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()
        })
    }

    fn moffat(size: usize, alpha2: f64) -> Array2<f64> {
        let centre = (size as f64 - 1.0) / 2.0;
        Array2::from_shape_fn((size, size), |(r, c)| {
            let (y, x) = (r as f64 - centre, c as f64 - centre);
            let q = 1.0 + (x * x + y * y) / alpha2;
            1.0 / (q * q)
        })
    }

    fn loaded(data: Array2<f64>, scale: f64) -> LoadedPsf {
        LoadedPsf {
            data,
            header: HduHeader::empty(),
            hdu: 0,
            extname: None,
            pixel_scale_arcsec: scale,
            scale_source: ScaleSource::Given,
            oversampling: None,
            filter: None,
            warnings: Vec::new(),
        }
    }

    fn f64_hdu(primary: bool, data: &Array2<f64>, extra: &[(&'static str, String)]) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let (rows, cols) = data.dim();
        let mut cards: Vec<(&'static str, String)> = if primary {
            vec![("SIMPLE", "T".into())]
        } else {
            vec![("XTENSION", "'IMAGE   '".into())]
        };
        cards.extend([("BITPIX", "-64".into()), ("NAXIS", "2".into()), ("NAXIS1", cols.to_string()), ("NAXIS2", rows.to_string())]);
        if primary {
            cards.push(("EXTEND", "T".into()));
        } else {
            cards.extend([("PCOUNT", "0".into()), ("GCOUNT", "1".into())]);
        }
        cards.extend(extra.iter().cloned());
        (cards, data.iter().flat_map(|v| v.to_be_bytes()).collect())
    }

    fn d1_psf(name: &str) -> LoadedPsf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack/d1/psf").join(name);
        load_psf(&path, None).unwrap()
    }

    fn clipped_fwhm_cases(gaussian_fwhm: &Value) -> &[Value] {
        let clipped = &gaussian_fwhm["clipped_psfs"];
        if clipped.is_null() {
            return &[];
        }
        clipped["psfs"].as_array().or_else(|| clipped.as_array()).filter(|cases| !cases.is_empty()).map_or_else(
            || panic!("gaussian_fwhm.clipped_psfs must be a non-empty list of {{input, fwhm_px}} cases or hold one under psfs, got {clipped}"),
            Vec::as_slice,
        )
    }

    #[test]
    fn gaussian_psf_resampled_keeps_fwhm_within_one_percent() {
        let sigma = 6.0;
        let source = loaded(gaussian(121, sigma, (60.0, 60.0)), 0.01);
        let input_fwhm_arcsec = GAUSSIAN_SIGMA_TO_FWHM * sigma * 0.01;
        let measured = fit_gaussian(&source.data).unwrap().fwhm_px * 0.01;
        assert!((measured / input_fwhm_arcsec - 1.0).abs() < 1e-6, "Gaussian fit of the input: {measured} against {input_fwhm_arcsec}");

        let options = PsfPrepareOptions { target_pixel_scale_arcsec: Some(0.04), method: PsfResampleMethod::Lightstack, ..PsfPrepareOptions::default() };
        let lightstack = prepare_psf(&source, &options).unwrap();
        let fwhm = lightstack.after.fwhm_arcsec.unwrap();
        assert!((fwhm / input_fwhm_arcsec - 1.0).abs() < 0.01, "lightstack resampling: FWHM {fwhm} against {input_fwhm_arcsec}");

        let area = prepare_psf(&source, &PsfPrepareOptions { method: PsfResampleMethod::Area, ..options }).unwrap();
        let integrated = GAUSSIAN_SIGMA_TO_FWHM * (sigma * sigma + 16.0 / 12.0).sqrt() * 0.01;
        let fwhm = area.after.fwhm_arcsec.unwrap();
        eprintln!(
            "Gaussian sigma 6 px at 0.01\" to 0.04\": lightstack {:+.4}% of the input FWHM, area {:+.4}% of the pixel-integrated FWHM",
            100.0 * (lightstack.after.fwhm_arcsec.unwrap() / input_fwhm_arcsec - 1.0),
            100.0 * (fwhm / integrated - 1.0)
        );
        assert!((fwhm / integrated - 1.0).abs() < 0.01, "area resampling: FWHM {fwhm} against the pixel-integrated {integrated}");
        assert!(fwhm > lightstack.after.fwhm_arcsec.unwrap());
    }

    #[test]
    fn resampled_psf_is_odd_and_sums_to_one() {
        let source = loaded(moffat(40, 20.0), 0.02);
        for method in [PsfResampleMethod::Area, PsfResampleMethod::Lightstack] {
            for target in [0.05, 0.04, 0.0333] {
                let prepared = prepare_psf(&source, &PsfPrepareOptions { target_pixel_scale_arcsec: Some(target), method, ..PsfPrepareOptions::default() }).unwrap();
                let (rows, cols) = prepared.data.dim();
                assert!(rows % 2 == 1 && cols % 2 == 1, "{method:?} to {target}: {rows}x{cols}");
                let total: f64 = prepared.data.sum();
                assert!((total - 1.0).abs() < 1e-12, "{method:?} to {target}: sum {total}");
                assert!(prepared.data.iter().all(|&v| v >= 0.0 && v.is_finite()));
                assert_eq!((prepared.after.peak_row, prepared.after.peak_col), (rows / 2, cols / 2), "{method:?} to {target}");
                assert!(prepared.after.offset_x_px.abs() < 0.05 && prepared.after.offset_y_px.abs() < 0.05, "{method:?} to {target}: {:?}", prepared.after);
                assert_eq!(prepared.pixel_scale_arcsec, target);
                assert!(prepared.padded_to_odd);
            }
        }
        let shaped = prepare_psf(&source, &PsfPrepareOptions { target_pixel_scale_arcsec: Some(0.05), target_shape: Some((11, 13)), ..PsfPrepareOptions::default() }).unwrap();
        assert_eq!(shaped.data.dim(), (11, 13));
        assert_eq!(prepare_psf(&source, &PsfPrepareOptions { target_pixel_scale_arcsec: Some(0.01), ..PsfPrepareOptions::default() }).unwrap_err(), PsfError::Upsampling { method: "area", source_arcsec: 0.02, target_arcsec: 0.01 });
    }

    #[test]
    fn stpsf_detector_sampled_extension_is_preferred() {
        let dir = tempfile::tempdir().unwrap();
        let oversampled = gaussian(16, 4.0, (7.5, 7.5));
        let detector = gaussian(4, 1.0, (1.5, 1.5));
        let name = |n: &str| format!("'{n:<8}'");
        let stpsf = |path: &Path, distorted: bool| {
            let mut hdus = vec![
                f64_hdu(true, &oversampled, &[("EXTNAME", name("OVERSAMP")), ("PIXELSCL", "0.0078".into()), ("OVERSAMP", "4".into()), ("DET_SAMP", "4".into()), ("FILTER", "'F200W   '".into())]),
                f64_hdu(false, &detector, &[("EXTNAME", name("DET_SAMP")), ("PIXELSCL", "0.0312".into()), ("OVERSAMP", "1".into()), ("FILTER", "'F200W   '".into())]),
            ];
            if distorted {
                hdus.push(f64_hdu(false, &oversampled, &[("EXTNAME", name("OVERDIST")), ("PIXELSCL", "0.0078".into()), ("OVERSAMP", "4".into())]));
                hdus.push(f64_hdu(false, &detector.mapv(|v| 2.0 * v), &[("EXTNAME", name("DET_DIST")), ("PIXELSCL", "0.0312".into()), ("OVERSAMP", "1".into())]));
            }
            write_raw_hdus(path, &hdus);
        };
        let plain = dir.path().join("stpsf.fits");
        stpsf(&plain, false);
        let psf = load_psf(&plain, None).unwrap();
        assert_eq!((psf.hdu, psf.extname.as_deref()), (1, Some("DET_SAMP")));
        assert_eq!(psf.data, detector);
        assert_eq!((psf.pixel_scale_arcsec, psf.scale_source, psf.oversampling), (0.0312, ScaleSource::Pixelscl, Some(1.0)));
        assert_eq!(psf.filter.as_deref(), Some("F200W"));

        let distorted = dir.path().join("stpsf_dist.fits");
        stpsf(&distorted, true);
        let psf = load_psf(&distorted, None).unwrap();
        assert_eq!((psf.hdu, psf.extname.as_deref()), (3, Some("DET_DIST")));
        assert_eq!(psf.data, detector.mapv(|v| 2.0 * v));

        let given = load_psf(&plain, Some(0.031)).unwrap();
        assert_eq!((given.pixel_scale_arcsec, given.scale_source), (0.031, ScaleSource::Given));

        let bare = dir.path().join("bare.fits");
        write_raw_hdus(&bare, &[f64_hdu(true, &detector, &[])]);
        assert_eq!(load_psf(&bare, None).unwrap_err(), PsfError::NoScale(bare.display().to_string()));

        let conflicting = dir.path().join("conflicting.fits");
        let wcs = [("PIXSCALE", "0.04".to_string()), ("CTYPE1", "'RA---TAN'".to_string()), ("CTYPE2", "'DEC--TAN'".to_string()), ("CD1_1", "-1.3888888888888889E-05".to_string()), ("CD2_2", "1.3888888888888889E-05".to_string())];
        write_raw_hdus(&conflicting, &[f64_hdu(true, &detector, &wcs)]);
        let psf = load_psf(&conflicting, None).unwrap();
        assert_eq!((psf.pixel_scale_arcsec, psf.scale_source), (0.04, ScaleSource::Pixscale));
        assert!(psf.warnings.iter().any(|w| w.starts_with("the WCS gives 0.05") && w.contains("the PIXSCALE scale 0.04 is used")), "{:?}", psf.warnings);
        let from_wcs = dir.path().join("wcs_only.fits");
        write_raw_hdus(&from_wcs, &[f64_hdu(true, &detector, &wcs[1..])]);
        let psf = load_psf(&from_wcs, None).unwrap();
        assert!((psf.pixel_scale_arcsec - 0.05).abs() < 1e-12 && psf.scale_source == ScaleSource::Wcs, "{} {:?}", psf.pixel_scale_arcsec, psf.scale_source);

        let d1 = d1_psf("d1_psf_JWST_F150W_20mas.fits");
        assert_eq!((d1.hdu, d1.pixel_scale_arcsec, d1.scale_source, d1.data.dim()), (0, 0.02, ScaleSource::Pixscale, (41, 41)));
        assert!((d1.data.sum() - 1.0).abs() < 1e-12, "D1 PSFs keep their float64 values");
    }

    fn psf_file_with(dir: &Path, name: &str, cards: &[(&'static str, String)]) -> std::path::PathBuf {
        let detector = gaussian(4, 1.0, (1.5, 1.5));
        let mut extra = vec![("PIXELSCL", "0.0312".to_string())];
        extra.extend(cards.iter().cloned());
        let path = dir.join(name);
        write_raw_hdus(&path, &[f64_hdu(true, &detector, &extra)]);
        path
    }

    fn asinh_refusal_of(path: &Path) -> PsfError {
        PsfError::Read { label: path.display().to_string(), message: ASINH_PLANE_REFUSAL.to_string() }
    }

    #[test]
    fn an_asinh_scaled_psf_is_refused_instead_of_being_read_as_linear() {
        let dir = tempfile::tempdir().unwrap();
        let path = psf_file_with(dir.path(), "asinh.fits", &[("BSOFTEN", "0.25".into()), ("BOFFSET", "3.5".into())]);
        assert_eq!(load_psf(&path, None).unwrap_err(), asinh_refusal_of(&path));
    }

    #[test]
    fn one_asinh_card_is_enough_to_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [(&str, Vec<(&'static str, String)>); 3] = [
            ("offset_only.fits", vec![("BOFFSET", "3.5".into())]),
            ("soften_only.fits", vec![("BSOFTEN", "0.25".into())]),
            ("zero_soften.fits", vec![("BSOFTEN", "0".into()), ("BOFFSET", "3.5".into())]),
        ];
        for (name, cards) in cases {
            let path = psf_file_with(dir.path(), name, &cards);
            assert_eq!(load_psf(&path, None).unwrap_err(), asinh_refusal_of(&path), "{name}");
        }
    }

    #[test]
    fn a_linearised_file_is_read_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let detector = gaussian(4, 1.0, (1.5, 1.5));
        let linearised = psf_file_with(dir.path(), "linearised.fits", &[("ABSOFTEN", "0.25".into()), ("ABOFFSET", "3.5".into())]);
        assert_eq!(load_psf(&linearised, None).unwrap().data, detector);
        let plain = psf_file_with(dir.path(), "plain.fits", &[]);
        assert_eq!(load_psf(&plain, None).unwrap().data, detector);
    }

    #[test]
    fn the_refusal_names_the_plane_it_was_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = psf_file_with(dir.path(), "asinh.fits", &[("BSOFTEN", "0.25".into()), ("BOFFSET", "3.5".into())]);
        let bytes = std::fs::read(&path).unwrap();
        let hdus = image_hdus(&bytes, "k.fits").unwrap();
        let message = decode_planes_f64(&bytes, &hdus[0].1, "kernel k.fits").unwrap_err().to_string();
        assert_eq!(message, format!("kernel k.fits: {ASINH_PLANE_REFUSAL}"));
    }

    #[test]
    fn wcs_scale_is_the_mean_axis_scale_and_keeps_the_first_axis_bits_on_square_pixels() {
        use crate::core::cubeprep::cube::test_support::{cd_grid, header_of, pc_grid};
        use crate::infra::wcs_source::header_wcs;

        let mut rectangular = header_of(&cd_grid((64, 64), 0.1355, 0.0));
        rectangular.set("CD2_2", format!("{:E}", 0.1211 / 3600.0));
        let scale = wcs_scale(&rectangular).unwrap();
        assert!((scale - 0.1283).abs() < 1e-12, "0.1355 x 0.1211 arcsec pixels have a mean scale of 0.1283 arcsec, got {scale}");
        let reported = header_wcs(&rectangular).unwrap().pixel_scale_arcsec();
        assert!((scale / reported - 1.0).abs() < 1e-14, "{scale} against the {reported} the WCS tools and the cube PIXSCALE report");

        let first_axis_bits = |header: &HduHeader| {
            let cd = read_effective_cd(header).unwrap();
            Some((cd[0][0].hypot(cd[1][0]) * 3600.0).to_bits())
        };
        for rotation in [0.0, 17.3, 45.0, 123.4, 270.0] {
            for arcsec in [0.02, 0.031, 0.04, 0.13] {
                for grid in [cd_grid((64, 64), arcsec, rotation), pc_grid((64, 64), arcsec, rotation)] {
                    let header = header_of(&grid);
                    assert_eq!(wcs_scale(&header).map(f64::to_bits), first_axis_bits(&header), "{arcsec} arcsec at {rotation} deg");
                }
            }
        }

        let mut folders = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack")];
        let (mut checked, mut last_bits_apart) = (0, 0);
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                } else if path.extension().is_some_and(|extension| extension == "fits") {
                    let label = path.display().to_string();
                    for (hdu, parsed) in image_hdus(&std::fs::read(&path).unwrap(), &label).unwrap() {
                        if let Some((scale_x, scale_y)) = wcs_axis_scales(&parsed.header) {
                            assert_eq!(wcs_scale(&parsed.header).map(f64::to_bits), first_axis_bits(&parsed.header), "{label} HDU {hdu}");
                            checked += 1;
                            last_bits_apart += usize::from(scale_x != scale_y);
                        }
                    }
                }
            }
        }
        assert!(checked >= 76 && last_bits_apart >= 3, "{checked} LIGHTSTACK fixture headers with a celestial WCS, {last_bits_apart} with axis scales that differ in the last bits");

        let with_cd = |cd: [(&str, f64); 4]| {
            let mut header = header_of(&cd_grid((64, 64), 0.05, 0.0));
            for (key, value) in cd {
                header.set(key, format!("{value:E}"));
            }
            header
        };
        let no_second_axis = with_cd([("CD1_1", -1e-5), ("CD2_1", 2e-6), ("CD1_2", 0.0), ("CD2_2", 0.0)]);
        assert_eq!(wcs_scale(&no_second_axis).map(f64::to_bits), first_axis_bits(&no_second_axis));
        let no_first_axis = with_cd([("CD1_1", 0.0), ("CD2_1", 0.0), ("CD1_2", 2e-6), ("CD2_2", 1e-5)]);
        assert_eq!(wcs_scale(&no_first_axis), None);
    }

    #[test]
    fn off_center_psf_is_reported_in_pixels() {
        let psf = gaussian(31, 2.0, (15.0 + 1.3, 15.0 - 0.6));
        let report = diagnose_psf(&psf, Some(0.04));
        assert!((report.offset_x_px - 1.3).abs() < 0.01 && (report.offset_y_px + 0.6).abs() < 0.01, "{report:?}");
        assert_eq!((report.peak_row, report.peak_col), (14, 16));
        assert!(report.warnings.iter().any(|w| w.contains("off centre")), "{:?}", report.warnings);
        assert!(report.odd);
        assert!((report.fwhm_px.unwrap() - GAUSSIAN_SIGMA_TO_FWHM * 2.0).abs() < 1e-3);
        assert!((report.fwhm_arcsec.unwrap() - GAUSSIAN_SIGMA_TO_FWHM * 2.0 * 0.04).abs() < 1e-4);

        let (recentred, shift) = recentre_psf(&psf);
        assert!((shift[0] + 1.3).abs() < 0.01 && (shift[1] - 0.6).abs() < 0.01, "{shift:?}");
        let after = diagnose_psf(&recentred, Some(0.04));
        assert!(after.offset_x_px.abs() < 0.01 && after.offset_y_px.abs() < 0.01, "{after:?}");
        assert!(!after.warnings.iter().any(|w| w.contains("off centre")), "{:?}", after.warnings);

        let truncated = diagnose_psf(&gaussian(9, 3.0, (4.0, 4.0)), None);
        assert!(truncated.edge_energy_fraction > EDGE_ENERGY_WARNING);
        assert!(truncated.warnings.iter().any(|w| w.contains("edge")), "{:?}", truncated.warnings);
        let even = diagnose_psf(&gaussian(10, 1.5, (4.5, 4.5)), None);
        assert!(!even.odd && even.warnings.iter().any(|w| w.contains("odd")), "{:?}", even.warnings);
    }

    #[test]
    fn lightstack_resampling_matches_scipy_affine_transform_with_its_geometry() {
        let golden = load();
        let cases = &golden["lightstack_resample"];
        let d1 = d1_psf("d1_psf_JWST_F150W_20mas.fits");
        let moffat_input = moffat(61, 16.0);
        for (key, psf) in [("d1_f150w", &d1.data), ("moffat_tutorial2", &moffat_input)] {
            let case = &cases[key];
            let (source, target) = (number(&case["source_pixel_scale_arcsec"]), number(&case["target_pixel_scale_arcsec"]));
            let shape = vector(&case["target_shape"]);
            let shape = (shape[0] as usize, shape[1] as usize);
            let padding = lightstack_pad(psf, source, target, shape).unwrap();
            let padded = vector(&case["padded_shape"]);
            assert_eq!(padding.padded.dim(), (padded[0] as usize, padded[1] as usize), "{key}");
            let resampled = lightstack_resample(&padding.padded, padding.source_pixel_scale_arcsec, target, Some(shape)).unwrap();
            let expected = array2(&case["resampled"]);
            assert_close(&resampled, &expected, 1e-12, key);
            let peak = expected.iter().fold(0.0f64, |acc, &v| acc.max(v));
            let worst = resampled.iter().zip(expected.iter()).fold(0.0f64, |acc, (a, b)| acc.max((a - b).abs()));
            assert!(worst < 1e-4 * peak, "{key}: {worst} against the plan bound {}", 1e-4 * peak);
            assert!((resampled.sum() - 1.0).abs() < 1e-12, "{key}: sum {}", resampled.sum());
        }
        assert_eq!(lightstack_resample(&moffat(61, 16.0), 0.05, 0.1, None).unwrap().dim(), (30, 30), "Python rounds 30.5 to 30");

        let hst = moffat(61, 4.0);
        let padding = lightstack_pad(&hst, 0.05, 1.004, (61, 61)).unwrap();
        assert_eq!(padding.padded.dim(), (1225, 1225), "tutorial 2 prints Padded: (1225, 1225) for its 61x61 HST PSFs");
        let resampled = lightstack_resample(&padding.padded, 0.05, 1.004, Some((61, 61))).unwrap();
        assert_eq!(resampled.dim(), (61, 61));
        assert!((resampled.sum() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn area_resampling_integrates_a_spline_of_the_cumulative_flux() {
        let golden = load();
        let case = &golden["cumulative_rebin"];
        let d1 = d1_psf("d1_psf_JWST_F150W_20mas.fits");
        let shape = vector(&case["output_shape"]);
        let factor = number(&case["factor"]);
        let got = area_resample(&d1.data, 0.02, 0.02 * factor, (shape[0] as usize, shape[1] as usize)).unwrap();
        assert_close(&got, &array2(&case["output"]), 1e-12, "cumulative rebin of the D1 F150W PSF");

        let fine = Array2::from_shape_fn((12, 12), |(r, c)| ((r * 12 + c) * 7 % 11) as f64 + 1.0);
        let blocks = area_resample(&fine, 0.01, 0.04, (3, 3)).unwrap();
        let total = fine.sum();
        for (o, p) in [(0, 0), (1, 2), (2, 1)] {
            let block: f64 = fine.slice(s![4 * o..4 * o + 4, 4 * p..4 * p + 4]).sum();
            assert!((blocks[[o, p]] - block / total).abs() < 1e-13, "aligned 4x4 block ({o}, {p}): {} against {}", blocks[[o, p]], block / total);
        }
    }

    #[test]
    fn psf_set_takes_its_scale_and_shape_from_the_reference_psf() {
        let mut moffat_psf = loaded(moffat(40, 20.0), 0.02);
        moffat_psf.filter = Some("F200W".into());
        let psfs = vec![d1_psf("d1_psf_JWST_F150W_20mas.fits"), d1_psf("d1_psf_JWST_F444W_40mas.fits"), moffat_psf];
        let options = PsfSetOptions { reference: Some(" f444w ".into()), ..PsfSetOptions::default() };
        let set = prepare_psf_set(&psfs, &options).unwrap();
        assert_eq!((set.reference_index, set.target_pixel_scale_arcsec, set.target_shape), (Some(1), Some(0.04), Some((21, 21))));
        for prepared in &set.prepared {
            assert_eq!((prepared.data.dim(), prepared.pixel_scale_arcsec), ((21, 21), 0.04));
            assert!((prepared.data.sum() - 1.0).abs() < 1e-12);
        }
        assert!(set.prepared[1].resampled.is_none(), "the reference is already on the cube scale");
        assert_eq!(set.prepared[0].resampled.as_ref().map(|r| r.method), Some(PsfResampleMethod::Area));
        assert!(set.prepared[2].padded_to_odd && set.prepared[2].recentred_by_px.is_some());

        let unmatched = PsfSetOptions { reference: Some("F999W".into()), ..PsfSetOptions::default() };
        assert_eq!(prepare_psf_set(&psfs, &unmatched).unwrap_err(), PsfError::NoReference("F999W".into()));
        let twice = vec![psfs[1].clone(), psfs[1].clone()];
        assert_eq!(prepare_psf_set(&twice, &options).unwrap_err(), PsfError::AmbiguousReference { filter: "f444w".into(), count: 2 });
        let own_scale = prepare_psf_set(&psfs, &PsfSetOptions::default()).unwrap();
        assert_eq!((own_scale.reference_index, own_scale.target_pixel_scale_arcsec, own_scale.target_shape), (None, None, None));
        assert_eq!(own_scale.prepared[0].data.dim(), (41, 41));

        let header = prepared_psf_header(&psfs[0], &set.prepared[0], "d1_psf_JWST_F150W_20mas.fits");
        assert_eq!(header.get_f64("PIXSCALE"), Some(0.04));
        assert_eq!((header.get_f64("CRPIX1"), header.get_f64("CRPIX2")), (Some(11.0), Some(11.0)));
        assert!((header.get_f64("CD1_1").unwrap() + 0.04 / 3600.0).abs() < 1e-18 && (header.get_f64("CD2_2").unwrap() - 0.04 / 3600.0).abs() < 1e-18);
        assert_eq!((header.get("CTYPE1"), header.get("FILTER"), header.get("BUNIT")), (Some("RA---TAN"), Some("F150W"), Some("normalized")));
        assert_eq!(header.is_string_value("FILTER"), Some(true));
        assert!(header.get_f64("PSFFWHM").is_some_and(|fwhm| (fwhm - set.prepared[0].after.fwhm_arcsec.unwrap()).abs() < 1e-9));
        let history: Vec<&str> = header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, text)| text.as_str()).collect();
        assert!(history.iter().any(|line| line.contains("d1_psf_JWST_F150W_20mas.fits")), "{history:?}");
        assert!(history.join(" ").contains("cumulative flux"), "{history:?}");
    }

    #[test]
    fn an_even_psf_centred_on_its_geometric_centre_warns_off_centre_without_recentre() {
        let even = gaussian(40, 1.2, (19.5, 19.5));
        let options = PsfSetOptions { method: PsfResampleMethod::Lightstack, recentre: false, ..PsfSetOptions::default() };
        let set = prepare_psf_set(&[loaded(even.clone(), 0.04)], &options).unwrap();
        let prepared = &set.prepared[0];
        assert_eq!(prepared.data, normalize_psf(&make_odd(&even)).unwrap(), "recentre off leaves the padded PSF where make_odd put it");
        assert!(prepared.recentred_by_px.is_none() && prepared.padded_to_odd && prepared.resampled.is_none());
        assert!((prepared.after.offset_x_px + 0.5).abs() < 1e-9 && (prepared.after.offset_y_px + 0.5).abs() < 1e-9, "{:?}", prepared.after);
        assert!(prepared.warnings.iter().any(|w| w == "prepared PSF: the PSF centroid is off centre by (-0.500, -0.500) px"), "{:?}", prepared.warnings);
        let note = "prepared PSF: recentre is off, so against a centred target PSF the matching kernel will shift this plane by the opposite of its centroid offset";
        assert_eq!(prepared.warnings.iter().filter(|w| w.contains("recentre is off")).collect::<Vec<_>>(), vec![note], "without a reference PSF the shift has no number");
        assert!(prepared.warnings.iter().any(|w| w.contains("padded from 40x40 to 41x41")), "{:?}", prepared.warnings);
        assert_eq!(prepared.warnings.iter().filter(|w| w.starts_with("prepared PSF: ")).count(), 2, "{:?}", prepared.warnings);

        let recentred = prepare_psf_set(&[loaded(even, 0.04)], &PsfSetOptions { recentre: true, ..options }).unwrap();
        let prepared = &recentred.prepared[0];
        assert!(prepared.recentred_by_px.is_some() && prepared.after.offset_x_px.abs() < CENTRE_TOLERANCE_PX, "{:?}", prepared.after);
        assert!(!prepared.warnings.iter().any(|w| w.contains("off centre") || w.contains("recentre is off")), "{:?}", prepared.warnings);
    }

    #[test]
    fn the_kernel_shift_note_uses_the_offset_relative_to_the_reference_psf() {
        let psf = |size: usize, sigma: f64, scale: f64, filter: &str| {
            let centre = (size as f64 - 1.0) / 2.0;
            LoadedPsf { filter: Some(filter.into()), ..loaded(gaussian(size, sigma, (centre, centre)), scale) }
        };
        let options = PsfSetOptions { target_pixel_scale_arcsec: Some(0.04), reference: Some("F444W".into()), method: PsfResampleMethod::Lightstack, recentre: false };
        let notes = |prepared: &PreparedPsf| prepared.warnings.iter().filter(|w| w.contains("recentre is off")).cloned().collect::<Vec<_>>();
        let kernel_moment = |source: &Array2<f64>, target: &Array2<f64>| {
            let kernel = build_kernel(source, target, &KernelOptions::default()).unwrap().data;
            let (rows, cols, total) = (kernel.nrows(), kernel.ncols(), kernel.sum());
            kernel.indexed_iter().fold((0.0, 0.0), |(x, y), ((r, c), &v)| (x + v * (c as f64 - (cols / 2) as f64) / total, y + v * (r as f64 - (rows / 2) as f64) / total))
        };

        let filters = [("F814W", 1.0), ("F150W", 1.2), ("F200W", 1.4), ("F277W", 1.6), ("F356W", 1.8), ("F444W", 2.0)];
        let all_even: Vec<LoadedPsf> = filters.iter().map(|&(filter, sigma)| psf(40, sigma, 0.04, filter)).collect();
        let set = prepare_psf_set(&all_even, &options).unwrap();
        for (prepared, (filter, _)) in set.prepared.iter().zip(filters) {
            assert!(prepared.warnings.iter().any(|w| w == "prepared PSF: the PSF centroid is off centre by (-0.500, -0.500) px"), "{filter}: {:?}", prepared.warnings);
            assert!(notes(prepared).is_empty(), "{filter}: every PSF carries the same make_odd offset, so no kernel shifts its plane: {:?}", prepared.warnings);
        }
        let (mx, my) = kernel_moment(&set.prepared[1].data, &set.prepared[5].data);
        assert!(mx.abs() < 0.01 && my.abs() < 0.01, "F150W to F444W, both even: kernel first moment ({mx}, {my})");

        let mixed = [psf(80, 2.4, 0.02, "F150W"), psf(41, 1.4, 0.04, "F200W"), psf(40, 2.0, 0.04, "F444W")];
        let set = prepare_psf_set(&mixed, &options).unwrap();
        let reference = &set.prepared[2];
        assert!(notes(reference).is_empty(), "the reference plane is never convolved: {:?}", reference.warnings);
        for (index, filter, expected) in [(0, "F150W", -0.25), (1, "F200W", -0.5)] {
            let plane = &set.prepared[index];
            let (dx, dy) = (reference.after.offset_x_px - plane.after.offset_x_px, reference.after.offset_y_px - plane.after.offset_y_px);
            assert!((dx - expected).abs() < 0.01 && (dy - expected).abs() < 0.01, "{filter}: ({dx}, {dy}) against {expected}");
            let line = format!(
                "prepared PSF: recentre is off, so the kernel matching this PSF to the reference PSF F444W will shift this plane by about ({dx:.3}, {dy:.3}) px, the difference of their centroid offsets"
            );
            assert_eq!(notes(plane), vec![line], "{filter}");
            let (mx, my) = kernel_moment(&plane.data, &reference.data);
            eprintln!("{filter} to an even F444W reference: note ({dx:.4}, {dy:.4}) px, kernel first moment ({mx:.4}, {my:.4}) px");
            assert!((mx - dx).abs() < 0.01 && (my - dy).abs() < 0.01, "{filter}: kernel first moment ({mx}, {my}) against the note ({dx}, {dy})");
        }
        assert!(!set.prepared[1].warnings.iter().any(|w| w.contains("off centre")), "the F200W PSF itself is centred: {:?}", set.prepared[1].warnings);

        let centred_target = [psf(40, 1.2, 0.04, "F150W"), psf(41, 2.0, 0.04, "F444W")];
        let set = prepare_psf_set(&centred_target, &options).unwrap();
        let line = "prepared PSF: recentre is off, so the kernel matching this PSF to the reference PSF F444W will shift this plane by about (0.500, 0.500) px, the difference of their centroid offsets";
        assert_eq!(notes(&set.prepared[0]), vec![line.to_string()]);
        assert!(notes(&set.prepared[1]).is_empty());
        let (mx, my) = kernel_moment(&set.prepared[0].data, &set.prepared[1].data);
        assert!((mx - 0.5).abs() < 0.01 && (my - 0.5).abs() < 0.01, "even F150W to a centred odd F444W: kernel first moment ({mx}, {my})");
    }

    #[test]
    fn d1_psfs_through_the_recipe_options_raise_no_warning() {
        let names = [
            ("F814W", "d1_psf_HST_F814W_40mas.fits"),
            ("F150W", "d1_psf_JWST_F150W_20mas.fits"),
            ("F200W", "d1_psf_JWST_F200W_20mas.fits"),
            ("F277W", "d1_psf_JWST_F277W_40mas.fits"),
            ("F356W", "d1_psf_JWST_F356W_40mas.fits"),
            ("F444W", "d1_psf_JWST_F444W_40mas.fits"),
        ];
        let psfs: Vec<LoadedPsf> = names.iter().map(|&(filter, name)| LoadedPsf { filter: Some(filter.into()), ..d1_psf(name) }).collect();
        let options = PsfSetOptions { target_pixel_scale_arcsec: Some(0.04), reference: Some("F444W".into()), method: PsfResampleMethod::Lightstack, recentre: false };
        let set = prepare_psf_set(&psfs, &options).unwrap();
        assert_eq!(set.reference_index, Some(5));
        for (prepared, (filter, _)) in set.prepared.iter().zip(names) {
            assert!(prepared.warnings.is_empty(), "{filter}: {:?}", prepared.warnings);
            assert!(prepared.after.offset_x_px.abs() < 1e-9 && prepared.after.offset_y_px.abs() < 1e-9, "{filter}: {:?}", prepared.after);
        }
    }

    #[test]
    fn the_lightstack_path_reports_clipped_negative_pixels_and_flux() {
        let narrow = loaded(gaussian(21, 0.7, (10.0, 10.0)), 0.031);
        let options = PsfPrepareOptions { target_pixel_scale_arcsec: Some(0.04), method: PsfResampleMethod::Lightstack, ..PsfPrepareOptions::default() };
        let prepared = prepare_psf(&narrow, &options).unwrap();
        let padding = lightstack_pad(&narrow.data, 0.031, 0.04, prepared.data.dim()).unwrap();
        let measured = lightstack_resample_measured(&padding.padded, 0.031, 0.04, Some(prepared.data.dim())).unwrap();
        assert_eq!(measured.resampled, lightstack_resample(&padding.padded, 0.031, 0.04, Some(prepared.data.dim())).unwrap(), "the wrapper returns the measured array bit for bit");
        assert_eq!((prepared.clipped_negative_fraction, prepared.clipped_negative_flux_fraction), (measured.clipped_negative_fraction, measured.clipped_negative_flux_fraction));
        assert!(prepared.data.iter().all(|&v| v >= 0.0) && (prepared.data.sum() - 1.0).abs() < 1e-12);
        eprintln!(
            "Gaussian sigma 0.7 px, 0.031\" to 0.04\" with the LIGHTSTACK method: {:.4} of the pixels negative, negative flux {:.5} of the kept flux",
            prepared.clipped_negative_fraction, prepared.clipped_negative_flux_fraction
        );
        assert_eq!(prepared.data.dim(), (17, 17));
        assert!((prepared.clipped_negative_fraction - 132.0 / 289.0).abs() < 1e-12, "{}", prepared.clipped_negative_fraction);
        assert!((prepared.clipped_negative_flux_fraction - 0.01421).abs() < 1e-4, "{}", prepared.clipped_negative_flux_fraction);
        let clipped = prepared.warnings.iter().find(|w| w.contains("were negative and set to 0")).unwrap_or_else(|| panic!("{:?}", prepared.warnings));
        let expected = format!(
            "{:.2} percent of the pixels were negative and set to 0; their absolute flux is {:.3} percent of the kept flux",
            100.0 * prepared.clipped_negative_fraction,
            100.0 * prepared.clipped_negative_flux_fraction
        );
        assert_eq!(clipped, &expected);

        for name in ["d1_psf_JWST_F150W_20mas.fits", "d1_psf_JWST_F200W_20mas.fits"] {
            let parity = prepare_psf(&d1_psf(name), &PsfPrepareOptions { target_shape: Some((21, 21)), recentre: false, ..options.clone() }).unwrap();
            eprintln!("{name} 0.02\" to 0.04\" (21x21, LIGHTSTACK): warnings {:?}, offset ({:.4}, {:.4}) px", parity.warnings, parity.after.offset_x_px, parity.after.offset_y_px);
            assert_eq!((parity.clipped_negative_fraction, parity.clipped_negative_flux_fraction), (0.0, 0.0), "{name}: {:?}", parity.warnings);
            assert!(parity.warnings.is_empty(), "{name}: the D1 PSFs keep an empty warning list: {:?}", parity.warnings);
        }

        let ringing = Array2::from_shape_vec((1, 6), vec![0.0, 0.5, -0.25, 0.25, f64::NAN, 0.25]).unwrap();
        assert_eq!(negative_share(&ringing), (1.0 / 6.0, 0.25));
        let area = prepare_psf(&loaded(moffat(41, 4.0).mapv(|v| v - 0.0005), 0.02), &PsfPrepareOptions { target_pixel_scale_arcsec: Some(0.04), ..PsfPrepareOptions::default() }).unwrap();
        assert!(area.clipped_negative_fraction > 0.0 && area.clipped_negative_flux_fraction > 0.0, "{:?}", area.warnings);
        assert!(area.warnings.iter().any(|w| w.contains("percent of the kept flux")), "{:?}", area.warnings);
    }

    #[test]
    fn input_warnings_hidden_by_the_lightstack_padding_are_forwarded() {
        let truncated = loaded(gaussian(9, 3.0, (4.0, 4.0)).mapv(|v| v - 0.2), 0.02);
        let before = diagnose_psf(&truncated.data, Some(0.02));
        assert!(before.warnings.iter().any(|w| w.contains("truncated")) && before.warnings.iter().any(|w| w.contains("are negative")), "{:?}", before.warnings);
        let options = PsfPrepareOptions { target_pixel_scale_arcsec: Some(0.04), target_shape: Some((21, 21)), method: PsfResampleMethod::Lightstack, ..PsfPrepareOptions::default() };
        let prepared = prepare_psf(&truncated, &options).unwrap();
        assert!(!prepared.after.warnings.iter().any(|w| w.contains("truncated") || w.contains("are negative")), "{:?}", prepared.after.warnings);
        let forwarded: Vec<&String> = prepared.warnings.iter().filter(|w| w.starts_with("input PSF: ")).collect();
        assert_eq!(forwarded.len(), 2, "{:?}", prepared.warnings);
        assert!(forwarded.iter().any(|w| w.contains("may be truncated")) && forwarded.iter().any(|w| w.contains("percent of the pixels are negative")), "{forwarded:?}");

        let kept = prepare_psf(&truncated, &PsfPrepareOptions { target_pixel_scale_arcsec: Some(0.02), target_shape: Some((9, 9)), ..options.clone() }).unwrap();
        assert!(kept.after.warnings.iter().any(|w| w.contains("truncated")), "{:?}", kept.after.warnings);
        assert!(kept.warnings.iter().any(|w| w.starts_with("prepared PSF: ") && w.contains("truncated")), "{:?}", kept.warnings);
        assert!(!kept.warnings.iter().any(|w| w.starts_with("input PSF: ")), "without resampling nothing is hidden: {:?}", kept.warnings);
        let negative: Vec<&String> = kept.warnings.iter().filter(|w| w.contains("negative")).collect();
        assert_eq!(negative.len(), 1, "the clipped warning already reports the input negatives: {:?}", kept.warnings);
        assert!(negative[0].starts_with(&format!("{:.2} percent of the pixels were negative and set to 0", 100.0 * before.negative_fraction)), "{negative:?}");

        let area = prepare_psf(&truncated, &PsfPrepareOptions { method: PsfResampleMethod::Area, ..options }).unwrap();
        assert!(area.warnings.iter().any(|w| w.starts_with("input PSF: ") && w.contains("percent of the pixels are negative")), "{:?}", area.warnings);
    }

    #[test]
    fn gaussian_fwhm_matches_a_minpack_fit_on_the_lightstack_box() {
        let golden = load();
        let cases = &golden["gaussian_fwhm"];
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack");
        let clipped = clipped_fwhm_cases(cases);
        for wanted in ["d1/psf/d1_psf_JWST_F356W_40mas.fits", "d1/psf/d1_psf_JWST_F444W_40mas.fits"] {
            assert!(clipped.is_empty() || clipped.iter().any(|case| case["input"] == wanted), "gaussian_fwhm.clipped_psfs lacks {wanted}");
        }
        let mut checked = 0;
        for case in cases["psfs"].as_array().unwrap().iter().chain(clipped) {
            let input = case["input"].as_str().unwrap();
            let psf = load_psf(&fixtures.join(input), None).unwrap();
            let (got, expected) = (fit_gaussian(&psf.data).unwrap().fwhm_px, number(&case["fwhm_px"]));
            assert!((got / expected - 1.0).abs() < 1e-5, "{input}: {got} px against MINPACK {expected} px");
            checked += 1;
        }
        assert_eq!(checked, if clipped.is_empty() { 5 } else { 7 }, "five full-box PSFs, plus F356W and F444W once the clipped goldens exist");
        let rotated = &cases["rotated"];
        let (sigma, centre) = (vector(&rotated["sigma"]), vector(&rotated["centre"]));
        let theta = number(&rotated["theta"]);
        let image = Array2::from_shape_fn((31, 31), |(r, c)| {
            let (dx, dy) = (c as f64 - centre[0], r as f64 - centre[1]);
            let (sin, cos) = theta.sin_cos();
            let (u, v) = (dx * cos + dy * sin, -dx * sin + dy * cos);
            (-(u * u / (2.0 * sigma[0] * sigma[0]) + v * v / (2.0 * sigma[1] * sigma[1]))).exp()
        });
        let fit = fit_gaussian(&image).unwrap();
        assert!((fit.fwhm_px / number(&rotated["fwhm_px"]) - 1.0).abs() < 1e-6, "{fit:?}");
        assert!((fit.sigma_x - 2.5).abs() < 1e-6 && (fit.sigma_y - 1.5).abs() < 1e-6 && (fit.theta - theta).abs() < 1e-6, "{fit:?}");
        assert!((fit.x - centre[0]).abs() < 1e-6 && (fit.y - centre[1]).abs() < 1e-6, "{fit:?}");
    }

    #[test]
    fn clipped_fwhm_goldens_in_an_unknown_layout_are_refused() {
        let case = serde_json::json!({ "input": "d1/psf/d1_psf_JWST_F356W_40mas.fits", "fwhm_px": 3.470861740116989 });
        assert!(clipped_fwhm_cases(&serde_json::json!({ "psfs": [] })).is_empty(), "no clipped_psfs key adds no case");
        assert_eq!(clipped_fwhm_cases(&serde_json::json!({ "clipped_psfs": [case.clone()] })).len(), 1);
        assert_eq!(clipped_fwhm_cases(&serde_json::json!({ "clipped_psfs": { "description": "box clipped", "psfs": [case.clone(), case.clone()] } })).len(), 2);
        for layout in [serde_json::json!({ "description": "box clipped", "cases": [case.clone()] }), serde_json::json!([]), serde_json::json!({ "psfs": [] }), serde_json::json!("F356W")] {
            let golden = serde_json::json!({ "clipped_psfs": layout });
            let refused = std::panic::catch_unwind(|| clipped_fwhm_cases(&golden).len());
            let payload = refused.expect_err(&format!("{layout}: an unknown layout must fail, not add zero cases"));
            let message = payload.downcast_ref::<String>().map(String::as_str).or_else(|| payload.downcast_ref::<&str>().copied()).unwrap_or_default();
            assert!(message.starts_with("gaussian_fwhm.clipped_psfs must be"), "{layout}: {message}");
        }
    }

    #[test]
    fn make_odd_and_normalize_follow_lightstack() {
        let even = Array2::from_shape_fn((4, 5), |(r, c)| (r * 5 + c) as f64);
        let odd = make_odd(&even);
        assert_eq!(odd.dim(), (5, 5));
        assert_eq!(odd.row(4).sum(), 0.0, "LIGHTSTACK pads the missing row after the data");
        assert_eq!(odd[[0, 0]], 0.0);
        assert_eq!(odd[[3, 4]], 19.0);
        let noisy = Array2::from_shape_vec((1, 4), vec![f64::NAN, -1.0, 1.0, 3.0]).unwrap();
        assert_eq!(normalize_psf(&noisy).unwrap(), Array2::from_shape_vec((1, 4), vec![0.0, 0.0, 0.25, 0.75]).unwrap());
        assert_eq!(normalize_psf(&Array2::from_elem((2, 2), -1.0)).unwrap_err(), PsfError::NotNormalizable(0.0));
    }
}
