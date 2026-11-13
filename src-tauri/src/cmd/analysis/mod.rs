pub(crate) mod area;
mod time_series;
pub use time_series::*;
mod status;
pub use status::*;
mod depth;
pub use depth::*;
mod forced;
pub use forced::*;
use area::AreaWeights;
use crate::service::area::holds_one_plane;

use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use tauri::ipc::Response;
use rayon::prelude::*;

use crate::cmd::catalog::{cone_query_for_field, field_geometry, FieldGeometry};
use crate::cmd::common::{
    blocking_cmd, cached_header, load_cached, load_cached_full, load_companions, source_path,
    HEADER_DISPLAY_REFERRED,
};
use crate::types::constants::{
    HISTOGRAM_BINS, HISTOGRAM_BINS_DISPLAY, RES_BINS, RES_BIN_COUNT, RES_MIN, RES_MAX,
    RES_DATA_MIN, RES_DATA_MAX, RES_MEDIAN, RES_MEAN, RES_SIGMA, RES_MAD, RES_TOTAL_PIXELS,
    RES_AUTO_STF, RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT, RES_ELAPSED_MS,
    RES_RA, RES_DEC, RES_GMAG, RES_BP_RP, RES_SEPARATION_ARCSEC,
    RES_PHOTOMETRY, RES_SKY, RES_GAIA, RES_GAIN_USED,
    RES_SUBFRAMES, RES_TOTAL, RES_ACCEPTED, RES_REJECTED,
    RES_MASKED, RES_DQ_EXCLUDED, RES_LABEL, RES_WARNINGS,
    RES_ROWS, RES_INDEX, RES_ERROR, RES_N_MEASURED, RES_N_FAILED, RES_N_DETECTED,
};
use crate::types::image::{AutoStfConfig, Histogram, ImageStats, StfParams};
use crate::core::analysis::fft::{compute_power_spectrum, FftResult};
use crate::core::analysis::gain::{gain_model, photon_noise_warning, GainModel};
use crate::core::analysis::photometry::{
    measure_star_prepared, saturation_level, MaskedImage, PhotometryConfig, StarPhotometry, MAX_APERTURE_RADIUS,
    MIN_APERTURE_RADIUS,
};
use crate::core::analysis::star_detection::{detect_stars as detect_stars_core, DetectionResult};
use crate::core::astrometry::catalog::{
    query_gaia_cached, CatalogHit, CatalogRow, ConeQuery, GAIA_MATCH_FALLBACK_ROWS, GAIA_MATCH_MAG_LIMIT,
};
use crate::core::astrometry::wcs::WcsTransform;
use crate::infra::wcs_source::load_wcs;
use crate::core::imaging::dq_flags::{apply_exclusion, exclusion_map};
use crate::core::imaging::layer_roles::{classify_layer, LayerRole};
use crate::core::imaging::luminance::rgb_to_luminance;
use crate::core::imaging::region::{paint_exclusions, RegionShape};
use crate::core::imaging::stats::{
    build_histogram, compute_histogram_with_stats, compute_image_stats, downsample_histogram, is_valid_pixel,
};
use crate::core::imaging::stf::auto_stf;
use crate::core::metadata::photcal::{missing_calibration_reason, MagSystem, PhotCal};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::band_planes::band_plane_of_path;
use crate::infra::fits::dispatcher::list_source_extensions;
use crate::infra::fits::reader::HduInfo;
use crate::infra::image_source::ErrOrigin;
use crate::types::header::HduHeader;
use crate::types::image_ref::PlaneSelector;

const PAR_THRESHOLD: usize = 1_000_000;
const MAX_BATCH_POINTS: usize = 5000;
const PAR_BATCH_POINTS: usize = 64;
const MAX_SKY_ANNULUS_RADIUS: f64 = 512.0;
const FFT_WINDOWED_FLAG: u32 = 1;
const FFT_DOWNSAMPLED_FLAG: u32 = 2;
const FFT_HEADER_BYTES: usize = 40;
const GAIA_MATCH_RADIUS_ARCSEC: f64 = 5.0;
const GAIA_MATCH_CONE_DEG: f64 = 0.01;
const ARCSEC_PER_ARCMIN: f64 = 60.0;
const IDENTITY_STF: StfParams = StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 };
const HISTOGRAM_CHUNK: usize = 65536;
const MIN_HISTOGRAM_WINDOW: f64 = 1e-10;

fn is_display_referred(path: &str) -> bool {
    cached_header(path)
        .ok()
        .and_then(|h| h.get(HEADER_DISPLAY_REFERRED).map(|v| v.trim().trim_matches('\'').trim() == "T"))
        .unwrap_or(false)
}

pub(crate) struct DqMask {
    pub map: ndarray::Array2<u8>,
    pub excluded: u64,
}

pub(crate) fn resolve_dq_mask(path: &str, exclude_dq: bool, dims: (usize, usize)) -> Option<DqMask> {
    if !exclude_dq {
        return None;
    }
    let comps = match load_companions(path) {
        Ok(comps) => comps,
        Err(e) => {
            log::warn!("DQ exclusion unavailable for {}: {:#}", path, e);
            return None;
        }
    };
    let exclusion = comps.exclusion_mask();
    let (entry, _) = comps.dq.filter(|_| exclusion != 0)?;
    let plane = entry.int_plane().filter(|plane| plane.bits.dim() == dims)?;
    let map = exclusion_map(&plane.bits, exclusion);
    let excluded = map.iter().filter(|&&v| v != 0).count() as u64;
    Some(DqMask { map, excluded })
}

pub(crate) struct PixelMask {
    pub map: ndarray::Array2<u8>,
    pub dq_excluded: Option<u64>,
    pub region_excluded: u64,
    pub exclude_skipped: usize,
}

impl PixelMask {
    pub fn masked(&self) -> bool {
        self.dq_excluded.is_some() || self.region_excluded > 0
    }

    pub fn notes(&self) -> Vec<String> {
        exclude_skipped_note(self.exclude_skipped).into_iter().collect()
    }
}

fn exclude_skipped_note(skipped: usize) -> Option<String> {
    let plural = if skipped == 1 { "" } else { "s" };
    (skipped > 0).then(|| format!("{skipped} exclude region{plural} skipped: invalid shape"))
}

pub(crate) fn resolve_pixel_mask(
    path: &str,
    exclude_dq: bool,
    exclude: &[RegionShape],
    dims: (usize, usize),
) -> Option<PixelMask> {
    let dq = resolve_dq_mask(path, exclude_dq, dims);
    if dq.is_none() && exclude.is_empty() {
        return None;
    }
    let dq_excluded = dq.as_ref().map(|m| m.excluded);
    let mut map = dq.map_or_else(|| ndarray::Array2::zeros(dims), |m| m.map);
    let paint = paint_exclusions(&mut map, exclude);
    Some(PixelMask { map, dq_excluded, region_excluded: paint.painted, exclude_skipped: paint.skipped })
}

fn display_frame(measured: &ImageStats, full: &ImageStats) -> ImageStats {
    let contains = full.min <= measured.min && measured.max <= full.max;
    if full.valid_count == 0 || !contains {
        return measured.clone();
    }
    ImageStats { min: full.min, max: full.max, ..measured.clone() }
}

fn reexpress_stf(stf: &StfParams, from: &ImageStats, to: &ImageStats) -> StfParams {
    let to_range = to.max - to.min;
    let from_range = from.max - from.min;
    let usable = |range: f64| range.is_finite() && range > 0.0;
    let same_frame = from.min == to.min && from.max == to.max;
    if same_frame || !usable(to_range) || !usable(from_range) {
        return *stf;
    }
    let map = |v: f64| ((from.min + v * from_range) - to.min) / to_range;
    StfParams { shadow: map(stf.shadow), midtone: stf.midtone, highlight: map(stf.highlight) }
}

fn histogram_window_arg(lo: Option<f64>, hi: Option<f64>) -> anyhow::Result<Option<(f64, f64)>> {
    let (lo, hi) = match (lo, hi) {
        (None, None) => return Ok(None),
        (Some(lo), Some(hi)) => (lo, hi),
        _ => anyhow::bail!("histogram window needs both lo and hi"),
    };
    if !lo.is_finite() || !hi.is_finite() {
        anyhow::bail!("histogram window ({lo}, {hi}) must be finite");
    }
    if hi <= lo {
        anyhow::bail!("histogram window hi {hi} must be larger than lo {lo}");
    }
    if hi - lo < MIN_HISTOGRAM_WINDOW {
        anyhow::bail!("histogram window from lo {lo} to hi {hi} is narrower than {MIN_HISTOGRAM_WINDOW}");
    }
    Ok(Some((lo, hi)))
}

fn windowed_histogram(arr: &ndarray::Array2<f32>, lo: f64, hi: f64) -> Histogram {
    let mut hist = build_histogram(&[], HISTOGRAM_BINS, lo, hi);
    let range = hi - lo;
    if hist.bins.is_empty() || range < MIN_HISTOGRAM_WINDOW {
        return hist;
    }
    let bins = hist.bins.len();
    let last = bins - 1;
    let inv_bin_width = bins as f64 / range;
    let bin_of = |v: f32| -> Option<usize> {
        let vd = v as f64;
        (is_valid_pixel(v) && vd >= lo && vd <= hi).then(|| (((vd - lo) * inv_bin_width) as usize).min(last))
    };
    let count_into = |mut local: Vec<u32>, v: f32| {
        if let Some(i) = bin_of(v) {
            local[i] += 1;
        }
        local
    };
    hist.bins = match arr.as_slice() {
        Some(slice) => slice
            .par_chunks(HISTOGRAM_CHUNK)
            .fold(|| vec![0u32; bins], |local, chunk| chunk.iter().copied().fold(local, count_into))
            .reduce_with(|mut a, b| {
                for (ai, bi) in a.iter_mut().zip(b.iter()) {
                    *ai += bi;
                }
                a
            })
            .unwrap_or_else(|| vec![0u32; bins]),
        None => arr.iter().copied().fold(vec![0u32; bins], count_into),
    };
    hist
}

#[tauri::command]
pub async fn compute_histogram(
    path: String,
    exclude_dq: Option<bool>,
    lo: Option<f64>,
    hi: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let window = histogram_window_arg(lo, hi)?;
        let cached = load_cached(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), cached.arr().dim());

        let masked_arr = match &mask {
            Some(m) => Some(apply_exclusion(cached.arr(), &m.map)?),
            None => None,
        };
        let masked_stats = masked_arr.as_ref().map(compute_image_stats);
        let arr = masked_arr.as_ref().unwrap_or(cached.arr());
        let stats = masked_stats.as_ref().unwrap_or(cached.stats());
        let display_referred = is_display_referred(&path);
        let frame = if display_referred {
            ImageStats { min: 0.0, max: 1.0, ..stats.clone() }
        } else {
            display_frame(stats, cached.stats())
        };

        let hist = match window {
            Some((lo, hi)) => windowed_histogram(arr, lo, hi),
            None => compute_histogram_with_stats(arr, &frame),
        };
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);
        let stf_params = if display_referred {
            IDENTITY_STF
        } else {
            reexpress_stf(&auto_stf(stats, &AutoStfConfig::default()), stats, &frame)
        };

        Ok(json!({
            RES_BINS: display_bins,
            RES_BIN_COUNT: display_bins.len(),
            RES_MIN: hist.min,
            RES_MAX: hist.max,
            RES_DATA_MIN: frame.min,
            RES_DATA_MAX: frame.max,
            RES_MEDIAN: stats.median,
            RES_MEAN: stats.mean,
            RES_SIGMA: stats.sigma,
            RES_MAD: stats.mad,
            RES_TOTAL_PIXELS: stats.valid_count,
            RES_AUTO_STF: {
                RES_SHADOW: stf_params.shadow,
                RES_MIDTONE: stf_params.midtone,
                RES_HIGHLIGHT: stf_params.highlight,
            },
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_MASKED: mask.is_some(),
            RES_DQ_EXCLUDED: mask.as_ref().map(|m| m.excluded),
        }))
    })
}

fn fft_header(fft: &FftResult, dc: f32, max_val: f32, elapsed_ms: u32) -> Vec<u8> {
    let (rows, cols) = fft.spectrum.dim();
    let mut buf = Vec::with_capacity(FFT_HEADER_BYTES + rows * cols);
    buf.extend_from_slice(&(cols as u32).to_le_bytes());
    buf.extend_from_slice(&(rows as u32).to_le_bytes());
    buf.extend_from_slice(&dc.to_le_bytes());
    buf.extend_from_slice(&max_val.to_le_bytes());
    buf.extend_from_slice(&elapsed_ms.to_le_bytes());
    buf.extend_from_slice(&(fft.padded_cols as u32).to_le_bytes());
    buf.extend_from_slice(&(fft.padded_rows as u32).to_le_bytes());
    let flags = FFT_WINDOWED_FLAG | if fft.downsampled { FFT_DOWNSAMPLED_FLAG } else { 0 };
    buf.extend_from_slice(&flags.to_le_bytes());
    buf.extend_from_slice(&(fft.image_cols as u32).to_le_bytes());
    buf.extend_from_slice(&(fft.image_rows as u32).to_le_bytes());
    buf
}

#[tauri::command]
pub async fn compute_fft_spectrum(path: String) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let t0 = Instant::now();
        let fft_result = compute_power_spectrum(load_cached(&path)?.arr())?;
        let spectrum = &fft_result.spectrum;
        let (rows, cols) = spectrum.dim();
        let pixel_count = rows * cols;

        let slice = spectrum.as_slice().expect("FFT spectrum must be contiguous");

        let (min_val, max_val) = if pixel_count > PAR_THRESHOLD {
            (
                slice.par_iter().cloned().reduce(|| f32::INFINITY, f32::min),
                slice.par_iter().cloned().reduce(|| f32::NEG_INFINITY, f32::max),
            )
        } else {
            slice.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &v| (mn.min(v), mx.max(v)))
        };

        let range = (max_val - min_val).max(1e-10);
        let inv_range = 255.0 / range;
        let dc = spectrum[[rows / 2, cols / 2]];
        let elapsed_ms = t0.elapsed().as_millis() as u32;
        let mut buf = fft_header(&fft_result, dc, max_val, elapsed_ms);

        let pixels: Vec<u8> = if pixel_count > PAR_THRESHOLD {
            slice.par_iter().map(|&v| ((v - min_val) * inv_range) as u8).collect()
        } else {
            slice.iter().map(|&v| ((v - min_val) * inv_range) as u8).collect()
        };
        buf.extend(pixels);

        Ok(Response::new(buf))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}

fn capped_detection_json(mut result: DetectionResult, max_stars: usize, t0: Instant) -> anyhow::Result<serde_json::Value> {
    let n_detected = result.stars.len();
    result.stars.truncate(max_stars);
    let mut val = serde_json::to_value(&result)?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert(RES_N_DETECTED.to_string(), json!(n_detected));
        obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
    }
    Ok(val)
}

#[tauri::command]
pub async fn detect_stars(
    path: String,
    sigma: f64,
    max_stars: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        capped_detection_json(detect_stars_core(load_cached(&path)?.arr(), sigma), max_stars, t0)
    })
}

#[tauri::command]
pub async fn detect_stars_composite(
    sigma: f64,
    max_stars: usize,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let (r, g, b) = crate::cmd::io::rgb_source_planes(path.as_deref())?;
        let lum = composite_detection_luminance(&r, &g, &b)?;
        capped_detection_json(detect_stars_core(&lum, sigma), max_stars, t0)
    })
}

fn composite_detection_luminance(
    r: &ndarray::Array2<f32>,
    g: &ndarray::Array2<f32>,
    b: &ndarray::Array2<f32>,
) -> anyhow::Result<ndarray::Array2<f32>> {
    if r.dim() != g.dim() || r.dim() != b.dim() {
        anyhow::bail!(
            "Composite planes differ in size: R {:?}, G {:?}, B {:?}",
            r.dim(),
            g.dim(),
            b.dim()
        );
    }
    Ok(rgb_to_luminance(r, g, b))
}

pub const PHOTOMETRY_SATURATED_FLAG: &str = "SATURATED";
pub const HEADER_PROCESSING_PROVENANCE: &str = "ABPROC";
pub const RES_PHOTCAL: &str = "photcal";
pub(crate) const DQ_NOT_APPLIED_PREFIX: &str = "DQ exclusion requested but not applied: ";

pub(crate) struct PhotometryPlanes {
    pub err: Option<ImageEntry>,
    pub saturated: Option<ndarray::Array2<u8>>,
}

pub(crate) fn photometry_planes(path: &str, dims: (usize, usize)) -> PhotometryPlanes {
    let comps = match load_companions(path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("companion planes unavailable for {}: {:#}", path, e);
            return PhotometryPlanes { err: None, saturated: None };
        }
    };
    let saturation = comps.saturated_mask();
    let err = comps.err.filter(|entry| entry.arr().dim() == dims);
    let saturated = comps.dq.filter(|_| saturation != 0).and_then(|(entry, _)| {
        let plane = entry.int_plane()?;
        (plane.bits.dim() == dims).then(|| exclusion_map(&plane.bits, saturation))
    });
    PhotometryPlanes { err, saturated }
}

pub(crate) fn apply_calibration(phot: &mut StarPhotometry, cal: &PhotCal) {
    if let Some(c) = cal.calibrate(phot.net_flux, Some(phot.flux_err)) {
        phot.flux_jy = Some(c.flux_jy);
        phot.flux_err_jy = c.flux_err_jy;
        phot.mag_ab = c.mag_ab;
        phot.mag_ab_err = c.mag_ab_err;
        phot.st_mag = c.st_mag;
    }
    phot.mag_ab_total = phot
        .flux_total
        .and_then(|total| cal.calibrate(total, None))
        .and_then(|c| c.mag_ab);
}

fn photcal_json(cal: &PhotCal) -> anyhow::Result<serde_json::Value> {
    let mut val = serde_json::to_value(cal)?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert(RES_LABEL.to_string(), json!(cal.label()));
    }
    Ok(val)
}

fn gaia_match_outcome(coord_ra: f64, coord_dec: f64, rows: &[&CatalogRow]) -> (serde_json::Value, Option<String>) {
    let cos_dec = coord_dec.to_radians().cos();
    let mut best: Option<(f64, usize)> = None;
    for (i, s) in rows.iter().enumerate() {
        let mut dra = (coord_ra - s.ra).abs();
        if dra > 180.0 {
            dra = 360.0 - dra;
        }
        let sep = ((dra * cos_dec).powi(2) + (coord_dec - s.dec).powi(2)).sqrt() * 3600.0;
        if sep < GAIA_MATCH_RADIUS_ARCSEC && best.map_or(true, |(bd, _)| sep < bd) {
            best = Some((sep, i));
        }
    }
    match best {
        Some((sep, i)) => (
            json!({
                RES_GMAG: rows[i].g,
                RES_BP_RP: rows[i].bp_rp,
                RES_SEPARATION_ARCSEC: sep,
            }),
            None,
        ),
        None => (
            serde_json::Value::Null,
            Some(format!("Gaia: no G<17 star within {GAIA_MATCH_RADIUS_ARCSEC}\"")),
        ),
    }
}

pub(crate) fn gaia_rows_for_match(
    coord: (f64, f64),
    field: &ConeQuery,
    cached: &mut dyn FnMut(&ConeQuery) -> Result<CatalogHit, String>,
) -> Result<Arc<Vec<CatalogRow>>, String> {
    let hit = cached(field)?;
    if hit.rows.len() < field.max_rows {
        return Ok(hit.rows);
    }
    let per_star = ConeQuery {
        ra: coord.0,
        dec: coord.1,
        radius_deg: GAIA_MATCH_CONE_DEG,
        mag_limit: Some(GAIA_MATCH_MAG_LIMIT),
        max_rows: GAIA_MATCH_FALLBACK_ROWS,
    };
    Ok(cached(&per_star)?.rows)
}

fn gaia_match_field(geometry: &FieldGeometry) -> ConeQuery {
    cone_query_for_field(
        geometry,
        Some(geometry.diagonal_arcmin / 2.0 + GAIA_MATCH_RADIUS_ARCSEC / ARCSEC_PER_ARCMIN),
        Some(GAIA_MATCH_MAG_LIMIT),
        None,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserMagSystem {
    Ab,
    Vega,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct UserZeroPoint {
    pub zp: f64,
    pub system: UserMagSystem,
    pub per_second: bool,
}

pub(crate) use crate::core::metadata::photcal::USER_ZERO_POINT_WARNING;

fn check_user_zero_point(zero_point: Option<&UserZeroPoint>) -> anyhow::Result<()> {
    if let Some(UserZeroPoint { zp, .. }) = zero_point {
        if !zp.is_finite() {
            anyhow::bail!("user zero point {zp} must be a finite magnitude");
        }
    }
    Ok(())
}

pub(crate) fn user_photcal(header: Option<&HduHeader>, zero_point: &UserZeroPoint) -> anyhow::Result<(PhotCal, Vec<String>)> {
    let bunit = header.and_then(|h| crate::core::metadata::photcal::card_text(h, "BUNIT"));
    let rate = bunit.as_deref().is_some_and(|unit| {
        crate::core::metadata::photcal::is_rate_unit(&crate::core::metadata::photcal::normalized_unit(unit))
    });
    let exptime = header.and_then(|h| h.get_f64("EXPTIME")).filter(|t| t.is_finite() && *t > 0.0);
    if zero_point.per_second && !rate && exptime.is_none() {
        anyhow::bail!(
            "a zero point per second needs EXPTIME to turn the pixel values into a rate, and this header has none (BUNIT '{}')",
            bunit.as_deref().unwrap_or("missing")
        );
    }
    let system = match zero_point.system {
        UserMagSystem::Ab => MagSystem::Ab,
        UserMagSystem::Vega => MagSystem::Vega,
    };
    Ok((PhotCal::user_zero_point_for(header, zero_point.zp, system, zero_point.per_second), Vec::new()))
}

pub(crate) struct PhotometryContext {
    pub entry: ImageEntry,
    pub mask: Option<DqMask>,
    pub planes: PhotometryPlanes,
    pub area: AreaWeights,
    pub wcs: Option<WcsTransform>,
    pub photcal: Option<PhotCal>,
    pub gain_model: GainModel,
    pub warnings: Vec<String>,
    pub layers: LayerReport,
    pub calibration_source: Option<&'static str>,
    pub calibration_gap: Option<String>,
    pub exclusion: Option<ndarray::Array2<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LayerReport {
    pub err_source: Option<String>,
    pub dq_source: Option<String>,
    pub dq_flags: Option<String>,
    pub dq_exclusion_available: bool,
    pub dq_gap: Option<String>,
    pub err_note: Option<String>,
    pub err_gap: Option<String>,
}

fn plane_label(plane: &PlaneSelector) -> String {
    match plane {
        PlaneSelector::Hdu(index) => format!("HDU {index}"),
        PlaneSelector::Array(key) => format!("array {key}"),
        PlaneSelector::Auto => "primary image".to_string(),
    }
}

fn err_origin_text(origin: &ErrOrigin) -> String {
    match origin.n_no_data {
        0 => origin.source.clone(),
        1 => format!("{}, 1 pixel without data", origin.source),
        n => format!("{}, {n} pixels without data", origin.source),
    }
}

pub(crate) fn layer_report(path: &str, entry: &ImageEntry) -> LayerReport {
    let dims = entry.arr().dim();
    let references = entry.companions();
    let dq_reference = references
        .and_then(|c| c.dq.as_ref())
        .map(|r| format!("DQ plane ({})", plane_label(&r.plane)));
    let loaded = match load_companions(path) {
        Ok(loaded) => loaded,
        Err(e) => {
            return LayerReport {
                dq_source: dq_reference,
                dq_gap: Some(format!("the companion planes could not be loaded ({e:#})")),
                ..LayerReport::default()
            };
        }
    };
    let err_source = match &loaded.err_origin {
        Some(origin) => Some(err_origin_text(origin)),
        None => references
            .and_then(|c| c.err.as_ref())
            .filter(|_| loaded.err.as_ref().is_some_and(|plane| plane.arr().dim() == dims))
            .map(|r| format!("ERR plane ({})", plane_label(&r.plane))),
    };
    let err_note = loaded
        .err_origin
        .as_ref()
        .filter(|origin| origin.role != LayerRole::Sigma || origin.assumed || origin.n_no_data > 0)
        .map(|origin| format!("error layer: {}", err_origin_text(origin)));
    let dq_flags = match &loaded.flags {
        Some(flags) => Some(flags.source.clone()),
        None => loaded.dq.as_ref().map(|(_, table)| table.label().to_string()),
    };
    let dq_exclusion_available = loaded.exclusion_mask() != 0
        && loaded
            .dq
            .as_ref()
            .is_some_and(|(companion, _)| companion.int_plane().is_some_and(|plane| plane.bits.dim() == dims));
    let dq_gap = match (&loaded.dq_unavailable, &loaded.flags, &dq_reference) {
        _ if dq_exclusion_available => None,
        (Some(reason), _, _) => Some(reason.clone()),
        (None, Some(flags), _) if !flags.is_named() => Some(format!(
            "the flag layer has no usable bit names ({}), so nothing is excluded until bits are chosen",
            flags.source
        )),
        (None, Some(flags), _) if flags.exclusion_mask == 0 => {
            Some(format!("no flag of this layer is selected for exclusion ({})", flags.source))
        }
        (None, _, None) => Some("this image has no DQ plane".to_string()),
        (None, _, Some(source)) => Some(format!("the {source} could not be read as an integer mask of the image size")),
    };
    LayerReport {
        err_source,
        dq_source: loaded.dq_origin.or(dq_reference),
        dq_flags,
        dq_exclusion_available,
        dq_gap,
        err_note,
        err_gap: loaded.err_unavailable,
    }
}

fn no_data_exclusion(path: &str, planes: &PhotometryPlanes, mask: Option<&DqMask>) -> Option<ndarray::Array2<u8>> {
    let sigma = planes.err.as_ref()?.arr();
    load_companions(path).ok()?.err_origin.filter(|origin| origin.n_no_data > 0)?;
    let mut excluded = mask.map_or_else(|| ndarray::Array2::zeros(sigma.dim()), |dq| dq.map.clone());
    ndarray::Zip::from(&mut excluded).and(sigma).for_each(|flag, value| {
        if value.is_nan() {
            *flag = 1;
        }
    });
    Some(excluded)
}

fn uncalibrated_layer_reason(path: &str, entry: &ImageEntry) -> Option<String> {
    let layer = entry.plane_info().and_then(|info| info.pairing.layer.clone()).or_else(|| {
        let source = source_path(path);
        let file_name = std::path::Path::new(&source).file_name().and_then(|name| name.to_str()).unwrap_or("");
        entry.header().map(|header| classify_layer(header, file_name))
    })?;
    let kind = match layer.role {
        LayerRole::Mask => "a mask",
        LayerRole::Variance => "a variance",
        LayerRole::InverseVariance => "an inverse-variance",
        LayerRole::Science | LayerRole::Sigma | LayerRole::Other => return None,
    };
    Some(format!("no calibration applied: this plane is {kind} layer ({})", layer.basis))
}

pub(crate) fn dq_not_applied_warning(report: &LayerReport) -> Option<String> {
    report.dq_gap.as_ref().map(|gap| format!("{DQ_NOT_APPLIED_PREFIX}{gap}"))
}

pub(crate) fn has_err_extension(path: &str, dims: (usize, usize)) -> bool {
    let Ok(entry) = load_cached_full(path).or_else(|_| load_cached(path)) else {
        return false;
    };
    let Some(reference) = entry.companions().and_then(|c| c.err.as_ref()) else {
        return false;
    };
    let (rows, cols) = dims;
    let fits_the_image = |h: &HduInfo| h.naxis1 == cols as i64 && h.naxis2 == rows as i64;
    let listed = |file: &str| list_source_extensions(file).unwrap_or_default();
    match (&reference.plane, band_plane_of_path(&reference.path)) {
        (PlaneSelector::Array(_), _) => false,
        (_, Some(band)) => listed(&band.file)
            .iter()
            .any(|h| h.index == band.hdu && h.has_data && h.naxis == 3 && fits_the_image(h) && (band.plane as i64) < h.naxis3),
        (PlaneSelector::Hdu(index), None) => {
            listed(&reference.path).iter().any(|h| h.index == *index && holds_one_plane(h) && fits_the_image(h))
        }
        (PlaneSelector::Auto, None) => {
            listed(&reference.path).iter().find(|h| h.has_data).is_some_and(|h| holds_one_plane(h) && fits_the_image(h))
        }
    }
}

pub(crate) fn photometry_context(path: &str, exclude_dq: bool) -> anyhow::Result<PhotometryContext> {
    photometry_context_with(path, exclude_dq, None)
}

pub(crate) fn photometry_context_with(
    path: &str,
    exclude_dq: bool,
    zero_point: Option<&UserZeroPoint>,
) -> anyhow::Result<PhotometryContext> {
    let entry = load_cached_full(path).or_else(|_| load_cached(path))?;
    let dims = entry.arr().dim();
    let mask = resolve_dq_mask(path, exclude_dq, dims);
    let planes = photometry_planes(path, dims);
    let layers = layer_report(path, &entry);
    let header = entry.header();
    let wcs = header.and_then(|h| load_wcs(path, h).ok());
    let gain_model = gain_model(header, planes.err.is_some());
    let mut warnings: Vec<String> = Vec::new();
    let header_calibration = match uncalibrated_layer_reason(path, &entry) {
        Some(layer_reason) => Err(layer_reason),
        None => Ok(header.and_then(|h| PhotCal::from_header(h, wcs.as_ref()))),
    };
    let (mut photcal, calibration_source, calibration_gap) = match (header_calibration, zero_point) {
        (Err(layer_reason), _) => (None, None, Some(layer_reason)),
        (Ok(Some(cal)), typed) => {
            warnings.extend(cal.warnings.iter().cloned());
            if typed.is_some() {
                warnings.push(format!("user zero point not applied: the header calibrates this image ({})", cal.origin));
            }
            (Some(cal), Some("header"), None)
        }
        (Ok(None), Some(typed)) => {
            let (cal, _) = user_photcal(header, typed)?;
            warnings.extend(cal.warnings.iter().cloned());
            (Some(cal), Some("user"), None)
        }
        (Ok(None), None) => (None, None, Some(missing_calibration_reason(header))),
    };
    warnings.extend(calibration_gap.clone());
    warnings.extend(layers.err_note.clone());
    warnings.extend(layers.err_gap.as_ref().map(|gap| format!("error layer not used: {gap}")));
    let area = AreaWeights::resolve(path, &entry, photcal.as_mut(), &mut warnings);
    warnings.extend(processed_data_warning(header));
    warnings.extend(wcs.as_ref().and_then(|w| w.pv_warning()).map(str::to_string));
    if exclude_dq && mask.is_none() {
        warnings.extend(dq_not_applied_warning(&layers));
    }
    let exclusion = no_data_exclusion(path, &planes, mask.as_ref());
    Ok(PhotometryContext {
        entry,
        mask,
        planes,
        area,
        wcs,
        photcal,
        gain_model,
        warnings,
        layers,
        calibration_source,
        calibration_gap,
        exclusion,
    })
}

pub(crate) fn processed_data_warning(header: Option<&HduHeader>) -> Option<String> {
    let provenance = header?.get(HEADER_PROCESSING_PROVENANCE)?;
    Some(format!("photometry on processed data ({})", provenance.trim().trim_matches('\'').trim()))
}

pub(crate) fn axis_scales_arcsec(wcs: &WcsTransform) -> (f64, f64) {
    axis_scales_of_cd(wcs.raw_params().4)
}

pub(crate) fn pixel_area_scale_arcsec(wcs: &WcsTransform) -> Option<f64> {
    area_scale_of_cd(wcs.raw_params().4)
}

const PIXEL_SCALE_ANISOTROPY_TOLERANCE: f64 = 0.01;

pub(crate) fn axis_scale_warning(wcs: &WcsTransform) -> Option<String> {
    let (sx, sy) = axis_scales_arcsec(wcs);
    let s = pixel_area_scale_arcsec(wcs)?;
    ((sx - sy).abs() / sx.max(sy) > PIXEL_SCALE_ANISOTROPY_TOLERANCE).then(|| {
        format!(
            "pixel scales differ per axis ({sx:.5} x {sy:.5} arcsec); radii in arcsec use the square root of the pixel area ({s:.5} arcsec)"
        )
    })
}

pub(crate) fn pixel_units(header: Option<&HduHeader>) -> (Option<String>, Option<String>) {
    let Some(header) = header else {
        return (None, None);
    };
    let Some(bunit) = crate::core::metadata::photcal::card_text(header, "BUNIT") else {
        return (None, None);
    };
    let summed_unit = match crate::core::metadata::units::classify_unit(header).class {
        crate::core::metadata::units::UnitClass::SurfaceBrightness { .. } => {
            format!("{bunit}{}", crate::cmd::line_measure::REGION_SUM_UNIT_SUFFIX)
        }
        _ => bunit.clone(),
    };
    (Some(bunit), Some(summed_unit))
}

fn axis_scales_of_cd(cd: [[f64; 2]; 2]) -> (f64, f64) {
    (cd[0][0].hypot(cd[1][0]) * 3600.0, cd[0][1].hypot(cd[1][1]) * 3600.0)
}

fn area_scale_of_cd(cd: [[f64; 2]; 2]) -> Option<f64> {
    let scale = (cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]).abs().sqrt() * 3600.0;
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

impl PhotometryContext {
    fn config(&self, aperture_radius: Option<f64>, sky_annulus: Option<(f64, f64)>, gain: Option<f64>) -> PhotometryConfig {
        PhotometryConfig {
            aperture_radius: aperture_radius.filter(|r| r.is_finite() && *r > 0.0),
            saturation: Some(saturation_level(self.entry.header(), self.entry.stats().max)),
            gain: gain.filter(|g| g.is_finite() && *g > 0.0),
            sky_annulus,
            ..PhotometryConfig::default()
        }
    }

    fn masked_image(&self) -> Result<MaskedImage<'_>, String> {
        self.area.masked_image(&self.entry, self.exclusion.as_ref().or(self.mask.as_ref().map(|m| &m.map)))
    }

    fn measure(&self, source: &MaskedImage, x: f64, y: f64, config: &PhotometryConfig) -> Result<StarPhotometry, String> {
        let mut phot = measure_star_prepared(
            source,
            self.planes.err.as_ref().map(|e| e.arr()),
            self.planes.saturated.as_ref(),
            x,
            y,
            config,
        )?;
        if let Some(cal) = &self.photcal {
            apply_calibration(&mut phot, cal);
        }
        Ok(phot)
    }

    fn sky_json(&self, phot: &StarPhotometry) -> serde_json::Value {
        match &self.wcs {
            Some(wcs) => {
                let coord = wcs.pixel_to_world(phot.x, phot.y);
                if coord.ra.is_finite() && coord.dec.is_finite() {
                    json!({ RES_RA: coord.ra, RES_DEC: coord.dec })
                } else {
                    serde_json::Value::Null
                }
            }
            None => serde_json::Value::Null,
        }
    }

    fn photcal_json(&self) -> anyhow::Result<serde_json::Value> {
        match &self.photcal {
            Some(cal) => photcal_json(cal),
            None => Ok(serde_json::Value::Null),
        }
    }
}

#[cfg(test)]
pub(crate) fn photometry_for_path(
    path: &str,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    sky_annulus: Option<(f64, f64)>,
    gaia_match: bool,
    exclude_dq: bool,
    gain: Option<f64>,
) -> anyhow::Result<serde_json::Value> {
    photometry_for_path_with_catalog(
        path,
        x,
        y,
        aperture_radius,
        sky_annulus,
        gaia_match,
        exclude_dq,
        gain,
        None,
        &mut |q| query_gaia_cached(q),
    )
}

pub(crate) fn photometry_for_path_with_catalog(
    path: &str,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    sky_annulus: Option<(f64, f64)>,
    gaia_match: bool,
    exclude_dq: bool,
    gain: Option<f64>,
    zero_point: Option<&UserZeroPoint>,
    cached: &mut dyn FnMut(&ConeQuery) -> Result<CatalogHit, String>,
) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let ctx = photometry_context_with(path, exclude_dq, zero_point)?;
    let config = ctx.config(aperture_radius, sky_annulus, gain);
    let source = ctx.masked_image().map_err(anyhow::Error::msg)?;
    let phot = ctx.measure(&source, x, y, &config).map_err(|e| anyhow::anyhow!(e))?;

    let sky = ctx.sky_json(&phot);
    let mut warnings = ctx.warnings.clone();
    warnings.extend(photon_noise_warning(&ctx.gain_model, config.gain.is_some(), phot.err_used));
    let gaia = match (&ctx.wcs, gaia_match) {
        (Some(wcs), true) => {
            let coord = wcs.pixel_to_world(phot.x, phot.y);
            if coord.ra.is_finite() && coord.dec.is_finite() {
                let (rows_n, cols) = ctx.entry.arr().dim();
                let geometry = field_geometry(wcs, cols, rows_n);
                let field = gaia_match_field(&geometry);
                match gaia_rows_for_match((coord.ra, coord.dec), &field, cached) {
                    Ok(rows) => {
                        let with_colour: Vec<&CatalogRow> = rows.iter().filter(|r| r.bp_rp.is_some()).collect();
                        let (gaia, warning) = gaia_match_outcome(coord.ra, coord.dec, &with_colour);
                        warnings.extend(warning);
                        gaia
                    }
                    Err(reason) => {
                        warnings.push(format!("Gaia query failed: {reason}"));
                        serde_json::Value::Null
                    }
                }
            } else {
                warnings.push("Gaia match skipped: the position has no sky coordinate".into());
                serde_json::Value::Null
            }
        }
        (None, true) => {
            warnings.push("Gaia match skipped: the file has no celestial WCS".into());
            serde_json::Value::Null
        }
        (_, false) => serde_json::Value::Null,
    };
    let (bunit, summed_unit) = pixel_units(ctx.entry.header());

    Ok(json!({
        RES_PHOTOMETRY: serde_json::to_value(&phot)?,
        RES_SKY: sky,
        RES_GAIA: gaia,
        RES_PHOTCAL: ctx.photcal_json()?,
        "bunit": bunit,
        "summed_unit": summed_unit,
        RES_WARNINGS: warnings,
        RES_GAIN_USED: config.gain,
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        RES_MASKED: ctx.mask.is_some(),
    }))
}

fn annulus_arg(inner: Option<f64>, outer: Option<f64>) -> anyhow::Result<Option<(f64, f64)>> {
    let (i, o) = match (inner, outer) {
        (None, None) => return Ok(None),
        (Some(i), Some(o)) => (i, o),
        _ => anyhow::bail!("sky annulus needs both an inner and an outer radius"),
    };
    if !i.is_finite() || i <= 0.0 {
        anyhow::bail!("sky annulus inner radius {i} must be a positive finite number of pixels");
    }
    if !o.is_finite() || o <= 0.0 {
        anyhow::bail!("sky annulus outer radius {o} must be a positive finite number of pixels");
    }
    if o > MAX_SKY_ANNULUS_RADIUS {
        anyhow::bail!("sky annulus outer radius {o} must be at most {MAX_SKY_ANNULUS_RADIUS} pixels");
    }
    if o <= i {
        anyhow::bail!("sky annulus outer radius {o} must be larger than the inner radius {i}");
    }
    Ok(Some((i, o)))
}

fn aperture_radius_arg(aperture_radius: Option<f64>) -> anyhow::Result<Option<f64>> {
    if let Some(r) = aperture_radius {
        if !(MIN_APERTURE_RADIUS..=MAX_APERTURE_RADIUS).contains(&r) {
            anyhow::bail!("aperture radius {r} must be between {MIN_APERTURE_RADIUS} and {MAX_APERTURE_RADIUS} pixels");
        }
    }
    Ok(aperture_radius)
}

fn check_annulus_clears_aperture(aperture_radius: Option<f64>, sky_annulus: Option<(f64, f64)>) -> anyhow::Result<()> {
    if let (Some(r_ap), Some((r_in, _))) = (aperture_radius.filter(|r| r.is_finite() && *r > 0.0), sky_annulus) {
        if r_in <= r_ap {
            anyhow::bail!("sky annulus inner radius {r_in} must be larger than the aperture radius {r_ap}");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn measure_photometry_cmd(
    path: String,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    annulus_inner: Option<f64>,
    annulus_outer: Option<f64>,
    gaia_match: Option<bool>,
    exclude_dq: Option<bool>,
    gain: Option<f64>,
    zero_point: Option<UserZeroPoint>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        if !x.is_finite() || !y.is_finite() {
            anyhow::bail!("photometry position ({x}, {y}) must be finite");
        }
        let aperture_radius = aperture_radius_arg(aperture_radius)?;
        let sky_annulus = annulus_arg(annulus_inner, annulus_outer)?;
        check_annulus_clears_aperture(aperture_radius, sky_annulus)?;
        check_user_zero_point(zero_point.as_ref())?;
        photometry_for_path_with_catalog(
            &path,
            x,
            y,
            aperture_radius,
            sky_annulus,
            gaia_match.unwrap_or(false),
            exclude_dq.unwrap_or(false),
            gain,
            zero_point.as_ref(),
            &mut |q| query_gaia_cached(q),
        )
    })
}

#[tauri::command]
pub async fn measure_photometry_batch_cmd(
    path: String,
    points: Vec<(f64, f64)>,
    aperture_radius: Option<f64>,
    annulus_inner: Option<f64>,
    annulus_outer: Option<f64>,
    gain: Option<f64>,
    exclude_dq: Option<bool>,
    with_growth_curve: Option<bool>,
    zero_point: Option<UserZeroPoint>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        if points.is_empty() {
            anyhow::bail!("batch photometry needs at least one point");
        }
        if points.len() > MAX_BATCH_POINTS {
            anyhow::bail!("batch photometry of {} points exceeds the limit of {MAX_BATCH_POINTS}", points.len());
        }
        if let Some((i, (x, y))) = points.iter().enumerate().find(|(_, (x, y))| !x.is_finite() || !y.is_finite()) {
            anyhow::bail!("batch photometry point {i} ({x}, {y}) must be finite");
        }
        let aperture_radius = aperture_radius_arg(aperture_radius)?;
        let sky_annulus = annulus_arg(annulus_inner, annulus_outer)?;
        check_annulus_clears_aperture(aperture_radius, sky_annulus)?;
        check_user_zero_point(zero_point.as_ref())?;
        let with_growth_curve = with_growth_curve.unwrap_or(false);

        let ctx = photometry_context_with(&path, exclude_dq.unwrap_or(false), zero_point.as_ref())?;
        let config = ctx.config(aperture_radius, sky_annulus, gain);
        let source = ctx.masked_image().map_err(anyhow::Error::msg)?;
        let measure = |&(x, y): &(f64, f64)| ctx.measure(&source, x, y, &config);
        let measured: Vec<Result<StarPhotometry, String>> = if points.len() > PAR_BATCH_POINTS {
            points.par_iter().map(measure).collect()
        } else {
            points.iter().map(measure).collect()
        };

        let mut n_measured = 0usize;
        let mut all_err_used = true;
        let mut rows = Vec::with_capacity(measured.len());
        for (index, outcome) in measured.into_iter().enumerate() {
            match outcome {
                Ok(mut phot) => {
                    n_measured += 1;
                    all_err_used &= phot.err_used;
                    if !with_growth_curve {
                        phot.growth_curve.clear();
                    }
                    let sky = ctx.sky_json(&phot);
                    rows.push(json!({
                        RES_INDEX: index,
                        RES_PHOTOMETRY: serde_json::to_value(&phot)?,
                        RES_SKY: sky,
                        RES_ERROR: serde_json::Value::Null,
                    }));
                }
                Err(e) => rows.push(json!({
                    RES_INDEX: index,
                    RES_PHOTOMETRY: serde_json::Value::Null,
                    RES_SKY: serde_json::Value::Null,
                    RES_ERROR: e,
                })),
            }
        }
        let n_failed = rows.len() - n_measured;
        let mut warnings = ctx.warnings.clone();
        warnings.extend(photon_noise_warning(&ctx.gain_model, config.gain.is_some(), all_err_used));
        let (bunit, summed_unit) = pixel_units(ctx.entry.header());

        Ok(json!({
            RES_ROWS: rows,
            RES_PHOTCAL: ctx.photcal_json()?,
            "bunit": bunit,
            "summed_unit": summed_unit,
            RES_WARNINGS: warnings,
            RES_GAIN_USED: config.gain,
            RES_MASKED: ctx.mask.is_some(),
            RES_N_MEASURED: n_measured,
            RES_N_FAILED: n_failed,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn photometry_gain_model_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let entry = load_cached_full(&path).or_else(|_| load_cached(&path))?;
        let dims = entry.arr().dim();
        let has_err = has_err_extension(&path, dims)
            || entry
                .companions()
                .and_then(|c| c.err.as_ref())
                .is_some_and(|r| matches!(r.plane, PlaneSelector::Array(_)));
        Ok(serde_json::to_value(gain_model(entry.header(), has_err))?)
    })
}

#[tauri::command]
pub async fn analyze_subframes_cmd(
    paths: Vec<String>,
    max_fwhm: Option<f64>,
    max_eccentricity: Option<f64>,
    min_snr: Option<f64>,
    min_stars: Option<usize>,
    fwhm_weight: Option<f64>,
    eccentricity_weight: Option<f64>,
    snr_weight: Option<f64>,
    noise_weight: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();

        let config = crate::core::analysis::subframe::SubframeWeightConfig {
            max_fwhm: max_fwhm.unwrap_or(8.0),
            max_eccentricity: max_eccentricity.unwrap_or(0.7),
            min_snr: min_snr.unwrap_or(5.0),
            min_stars: min_stars.unwrap_or(5),
            fwhm_weight: fwhm_weight.unwrap_or(1.0),
            eccentricity_weight: eccentricity_weight.unwrap_or(0.5),
            snr_weight: snr_weight.unwrap_or(1.0),
            noise_weight: noise_weight.unwrap_or(0.3),
        };

        let mut metrics: Vec<crate::core::analysis::subframe::SubframeMetrics> = paths
            .par_iter()
            .map(|p| {
                let entry = load_cached(p)?;
                Ok(crate::core::analysis::subframe::analyze_subframe(entry.arr(), p, &config))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        crate::core::analysis::subframe::normalize_weights(&mut metrics);

        let accepted = metrics.iter().filter(|m| m.accepted).count();
        let rejected = metrics.len() - accepted;

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_SUBFRAMES: metrics,
            RES_TOTAL: metrics.len(),
            RES_ACCEPTED: accepted,
            RES_REJECTED: rejected,
            RES_ELAPSED_MS: elapsed,
        }))
    })
}

#[cfg(test)]
pub(crate) mod layer_fixtures {
    use crate::infra::fits::reader::test_fixtures::{write_raw_hdus, write_test_mef, HduData, TestHdu};
    use crate::infra::fits::writer::write_fits_mono;
    use crate::service::area::test_fixtures::gaussian_pixels;
    use crate::types::header::HduHeader;
    use crate::types::image_ref::BandPlaneKey;

    pub const SIZE: usize = 64;
    pub const NO_DATA_PIXELS: [(usize, usize); 3] = [(32, 30), (32, 31), (33, 30)];
    pub const AFW_BAD_PIXEL: (usize, usize) = (32, 34);
    pub const AFW_SATURATED_PIXEL: (usize, usize) = (32, 32);
    pub const AFW_DETECTED_BLOCK: std::ops::Range<usize> = 30..35;
    pub const UNNAMED_FLAG_PIXEL: (usize, usize) = (32, 33);

    pub fn layer(extname: &'static str, data: HduData, extra_cards: Vec<(&'static str, String)>) -> TestHdu {
        TestHdu { extname: Some(extname), extver: None, cols: SIZE, rows: SIZE, data, extra_cards }
    }

    pub fn star(extname: &'static str) -> TestHdu {
        layer(extname, HduData::F32(gaussian_pixels(SIZE, 1000.0, 2.0, 100.0)), vec![])
    }

    pub fn flat(extname: &'static str, value: f32) -> TestHdu {
        layer(extname, HduData::F32(vec![value; SIZE * SIZE]), vec![])
    }

    pub fn flagged(pixels: &[(usize, usize)], value: i32) -> Vec<i32> {
        let mut bits = vec![0i32; SIZE * SIZE];
        for (row, col) in pixels {
            bits[row * SIZE + col] = value;
        }
        bits
    }

    fn first_plane_key(path: &std::path::Path) -> String {
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    pub fn afw_star_mef(path: &std::path::Path, error: TestHdu) -> String {
        let mut mask = vec![0i32; SIZE * SIZE];
        for row in AFW_DETECTED_BLOCK {
            for col in AFW_DETECTED_BLOCK {
                mask[row * SIZE + col] = 32;
            }
        }
        mask[AFW_BAD_PIXEL.0 * SIZE + AFW_BAD_PIXEL.1] |= 1;
        mask[AFW_SATURATED_PIXEL.0 * SIZE + AFW_SATURATED_PIXEL.1] |= 2;
        let planes = vec![("MP_BAD", "0".to_string()), ("MP_SAT", "1".to_string()), ("HIERARCH MP_DETECTED", "5".to_string())];
        write_test_mef(path, &[], &[star("IMAGE"), layer("MASK", HduData::I32(mask), planes), error]);
        first_plane_key(path)
    }

    pub fn inverse_variance_star_mef(path: &std::path::Path) -> String {
        let mut inverse_variance = vec![0.25f32; SIZE * SIZE];
        for (row, col) in NO_DATA_PIXELS {
            inverse_variance[row * SIZE + col] = 0.0;
        }
        write_test_mef(path, &[], &[star("SCI"), layer("IVAR", HduData::F32(inverse_variance), vec![])]);
        first_plane_key(path)
    }

    pub fn unnamed_flag_star_mef(path: &std::path::Path) -> String {
        write_test_mef(path, &[], &[star("SCI"), layer("FLAG", HduData::I32(flagged(&[UNNAMED_FLAG_PIXEL], 1 << 24)), vec![])]);
        first_plane_key(path)
    }

    pub const LEGACY_BRICK_IMAGE: &str = "legacysurvey-0001m001-image-r.fits";
    pub const LEGACY_BRICK_INVVAR: &str = "legacysurvey-0001m001-invvar-r.fits";
    pub const CUTOUT_PLANES: usize = 3;
    pub const CUTOUT_STAR_PLANE: usize = 1;

    pub fn legacy_brick(dir: &std::path::Path, invvar_size: Option<usize>) -> String {
        let star = ndarray::Array2::from_shape_vec((SIZE, SIZE), gaussian_pixels(SIZE, 1000.0, 2.0, 100.0)).unwrap();
        let mut header = HduHeader::empty();
        header.set("BUNIT", "nanomaggy".to_string());
        let image = dir.join(LEGACY_BRICK_IMAGE).to_str().unwrap().to_string();
        write_fits_mono(&image, &star, Some(&header)).unwrap();
        if let Some(size) = invvar_size {
            let weights = ndarray::Array2::from_elem((size, size), 0.25f32);
            write_fits_mono(dir.join(LEGACY_BRICK_INVVAR).to_str().unwrap(), &weights, None).unwrap();
        }
        image
    }

    pub fn legacy_band_cutout(path: &std::path::Path, error_size: usize) -> String {
        let image_cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", SIZE.to_string()),
            ("NAXIS2", SIZE.to_string()),
            ("NAXIS3", CUTOUT_PLANES.to_string()),
            ("EXTEND", "T".into()),
            ("IMAGETYP", "'IMAGE   '".into()),
            ("BANDS", "'grz     '".into()),
            ("BAND0", "'g       '".into()),
            ("BAND1", "'r       '".into()),
            ("BAND2", "'z       '".into()),
            ("SURVEY", "'LegacySurvey'".into()),
        ];
        let star = gaussian_pixels(SIZE, 1000.0, 2.0, 100.0);
        let image: Vec<u8> = (0..CUTOUT_PLANES)
            .flat_map(|plane| if plane == CUTOUT_STAR_PLANE { star.clone() } else { vec![100.0f32; SIZE * SIZE] })
            .flat_map(f32::to_be_bytes)
            .collect();
        let invvar_cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", error_size.to_string()),
            ("NAXIS2", error_size.to_string()),
            ("NAXIS3", CUTOUT_PLANES.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("IMAGETYP", "'INVVAR  '".into()),
        ];
        let weights: Vec<u8> = vec![0.25f32; error_size * error_size * CUTOUT_PLANES].into_iter().flat_map(f32::to_be_bytes).collect();
        write_raw_hdus(path, &[(image_cards, image), (invvar_cards, weights)]);
        BandPlaneKey::new(path.to_str().unwrap(), 0, CUTOUT_STAR_PLANE).key()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use super::layer_fixtures::*;
    use crate::service::area::test_fixtures::*;
    use crate::core::analysis::gain::{PoissonRoute, UnitClass, SKY_ONLY_ERRORS_WARNING};
    use crate::cmd::common::dq_exclusion;
    use crate::core::analysis::aperture::circular_aperture;
    use crate::core::astrometry::catalog::{catalog_cache_key, CatalogCacheKey, CatalogSource};
    use crate::core::metadata::photcal::{AB_MAG_ZERO_POINT, JY_PER_MJY};
    use crate::infra::fits::reader::test_fixtures::{
        cube_hdu, empty_primary_cards, sci_err_dq_mef_with_dq_cards, write_raw_hdus, write_test_mef, HduData, TestHdu,
    };
    use crate::types::constants::RES_GAIN_USED;

    fn wfpc2_502nmos() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../exampleFits/sample-data/502nmos.fits");
        assert!(path.is_file(), "the repository sample {} is missing", path.display());
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn real_data_502nmos_gain_model() {
        let sample = wfpc2_502nmos();
        let header = cached_header(&sample).unwrap();
        let dims = (
            header.get_i64("NAXIS2").unwrap() as usize,
            header.get_i64("NAXIS1").unwrap() as usize,
        );
        assert_eq!(dims, (1600, 1600));
        let has_err = has_err_extension(&sample, dims);
        assert!(!has_err, "502nmos has a single HDU and no ERR extension");
        let model = gain_model(Some(&header), has_err);
        assert_eq!(model.gain_e_per_adu, Some(7.0), "{model:?}");
        assert_eq!(model.source.as_deref(), Some("ATODGAIN"));
        assert_eq!(model.unit_class, UnitClass::Counts);
        assert_eq!(model.ncombine, Some(2));
        assert_eq!(model.combine_method, None);
        assert!(!model.combine_scaled);
        assert_eq!(model.effective_gain, Some(7.0));
        assert_eq!(model.fallback_gain, None);
        assert_eq!(model.poisson_route, PoissonRoute::HeaderGain);
        let note = model.note.as_deref().unwrap_or("");
        assert!(note.contains("NCOMBINE=2") && note.contains("14"), "{note}");
    }

    fn jwst_star_mef(path: &std::path::Path, sci_cards: Vec<(&'static str, String)>) -> String {
        let size = 64;
        let mut dq = vec![0i32 - 2147483647 - 1; size * size];
        dq[32 * size + 33] = 2 - 2147483647 - 1;
        dq[30 * size + 30] = 1 - 2147483647 - 1;
        let mut cards = vec![
            ("BUNIT", "'MJy/sr'".to_string()),
            ("PIXAR_SR", "2.1E-13".to_string()),
            ("PHOTMJSR", "0.5".to_string()),
            ("TELESCOP", "'JWST'".to_string()),
        ];
        cards.extend(sci_cards);
        write_test_mef(
            path,
            &[],
            &[
                TestHdu {
                    extname: Some("SCI"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: HduData::F32(gaussian_pixels(size, 1000.0, 2.0, 100.0)),
                    extra_cards: cards,
                },
                TestHdu {
                    extname: Some("ERR"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: HduData::F32(vec![0.5; size * size]),
                    extra_cards: vec![("BUNIT", "'MJy/sr'".into())],
                },
                TestHdu {
                    extname: Some("DQ"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: HduData::I32(dq),
                    extra_cards: vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into())],
                },
            ],
        );
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    #[test]
    fn photometry_for_path_calibrates_with_err_and_dq_companions() {
        let dir = tempfile::tempdir().unwrap();
        let key = jwst_star_mef(&dir.path().join("jwst_star.fits"), vec![]);
        let out = photometry_for_path(&key, 32.0, 32.0, None, None, false, true, None).unwrap();
        let phot = &out[RES_PHOTOMETRY];
        assert_eq!(out[RES_MASKED], true);
        assert_eq!(phot["err_used"], true);
        assert_eq!(phot["n_saturated"], 1);
        assert_eq!(phot["saturated"], true);
        assert_eq!(phot["n_masked"], 1);
        let net = phot["net_flux"].as_f64().unwrap();
        let flux_jy = phot["flux_jy"].as_f64().unwrap();
        assert!((flux_jy - net * 2.1e-13 * 1e6).abs() < 1e-15, "flux_jy={flux_jy} net={net}");
        let mag_ab = phot["mag_ab"].as_f64().unwrap();
        assert!((mag_ab - (-2.5 * flux_jy.log10() + 8.90)).abs() < 1e-9);
        let flux_err = phot["flux_err"].as_f64().unwrap();
        assert!((phot["flux_err_jy"].as_f64().unwrap() - flux_err * 2.1e-7).abs() < 1e-15);
        assert!(phot["mag_ab_err"].as_f64().unwrap() > 0.0);
        assert!(phot["st_mag"].is_null());
        assert_eq!(out[RES_PHOTCAL]["convention"]["kind"], "jwst_mjy_sr");
        assert_eq!(out[RES_PHOTCAL]["bunit"], "MJy/sr");
        assert!(out[RES_PHOTCAL][RES_LABEL].as_str().unwrap().contains("JWST MJy/sr"));
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(out[RES_SKY].is_null() && out[RES_GAIA].is_null());

        let unmasked = photometry_for_path(&key, 32.0, 32.0, None, None, false, false, None).unwrap();
        assert_eq!(unmasked[RES_MASKED], false);
        assert_eq!(unmasked[RES_PHOTOMETRY]["n_masked"], 0);
        assert_eq!(unmasked[RES_PHOTOMETRY]["n_saturated"], 1);
    }

    #[test]
    fn photometry_for_path_warns_on_processed_data_and_missing_calibration() {
        let dir = tempfile::tempdir().unwrap();
        let key = jwst_star_mef(
            &dir.path().join("processed.fits"),
            vec![("ABPROC", "'ghs_stretch'".to_string())],
        );
        let out = photometry_for_path(&key, 32.0, 32.0, Some(5.0), None, false, false, None).unwrap();
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap() == "photometry on processed data (ghs_stretch)"),
            "{warnings:?}"
        );
        assert!((out[RES_PHOTOMETRY]["aperture_radius"].as_f64().unwrap() - 5.0).abs() < 1e-9);

        let plain = dir.path().join("plain.fits");
        let size = 64;
        let mut arr = ndarray::Array2::<f32>::zeros((size, size));
        for (i, v) in gaussian_pixels(size, 1000.0, 2.0, 100.0).into_iter().enumerate() {
            arr[[i / size, i % size]] = v;
        }
        crate::infra::fits::writer::write_fits_mono(plain.to_str().unwrap(), &arr, None).unwrap();
        let out = photometry_for_path(plain.to_str().unwrap(), 32.0, 32.0, None, None, false, false, Some(2.0)).unwrap();
        assert!(out[RES_PHOTCAL].is_null());
        let phot = &out[RES_PHOTOMETRY];
        assert!(phot["flux_jy"].is_null() && phot["mag_ab"].is_null());
        assert_eq!(phot["err_used"], false);
        assert_eq!(phot["n_saturated"], 0);
        assert_eq!(phot["saturated"], false, "a single peak at the image maximum is not saturated");
        assert_eq!(phot["saturation_source"], "image maximum");
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap().contains("no photometric calibration")),
            "{warnings:?}"
        );
        let with_gain = phot["flux_err"].as_f64().unwrap();
        let without = photometry_for_path(plain.to_str().unwrap(), 32.0, 32.0, None, None, false, false, None).unwrap();
        assert!(with_gain > without[RES_PHOTOMETRY]["flux_err"].as_f64().unwrap());
    }

    #[test]
    fn photometry_for_path_reads_the_saturation_level_from_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let size = 64;
        let mut arr = ndarray::Array2::<f32>::zeros((size, size));
        for (i, v) in gaussian_pixels(size, 1000.0, 2.0, 100.0).into_iter().enumerate() {
            arr[[i / size, i % size]] = v;
        }
        let mut header = crate::types::header::HduHeader::empty();
        header.set("SATURATE", "1000".to_string());
        let path = dir.path().join("saturate.fits");
        crate::infra::fits::writer::write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        let out = photometry_for_path(path.to_str().unwrap(), 32.0, 32.0, None, None, false, false, None).unwrap();
        let phot = &out[RES_PHOTOMETRY];
        assert_eq!(phot["peak"], 1100.0);
        assert_eq!(phot["saturated"], true);
        assert_eq!(phot["saturation_source"], "SATURATE");
        assert_eq!(phot["n_saturated"], 0);

        let key = jwst_star_mef(&dir.path().join("dq.fits"), vec![("SATURATE", "1000".to_string())]);
        let with_dq = photometry_for_path(&key, 32.0, 32.0, None, None, false, false, None).unwrap();
        assert_eq!(with_dq[RES_PHOTOMETRY]["saturation_source"], "DQ SATURATED");
        assert_eq!(with_dq[RES_PHOTOMETRY]["n_saturated"], 1);
    }

    fn noise(n: usize, sigma: f64, seed: u64) -> Vec<f32> {
        let mut state = seed;
        let mut uniform = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        (0..n)
            .map(|_| (sigma * (-2.0 * uniform().ln()).sqrt() * (2.0 * std::f64::consts::PI * uniform()).cos()) as f32)
            .collect()
    }

    fn hot_pixel_mef(path: &std::path::Path) -> String {
        let size = 64;
        let mut sci: Vec<f32> = noise(size * size, 100.0, 3).into_iter().map(|v| 500.0 + v).collect();
        sci[10 * size + 10] = 60000.0;
        let mut dq = vec![0i32 - 2147483647 - 1; size * size];
        dq[10 * size + 10] = 1 - 2147483647 - 1;
        write_test_mef(
            path,
            &[],
            &[
                TestHdu {
                    extname: Some("SCI"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: HduData::F32(sci),
                    extra_cards: vec![("TELESCOP", "'JWST'".to_string())],
                },
                TestHdu {
                    extname: Some("DQ"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: HduData::I32(dq),
                    extra_cards: vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into())],
                },
            ],
        );
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn render_one(v: f64, stf: &StfParams, stats: &ImageStats) -> u8 {
        crate::core::imaging::stf::apply_stf(&ndarray::Array2::from_elem((1, 1), v as f32), stf, stats)[0]
    }

    #[tokio::test]
    async fn a_dq_masked_auto_stf_renders_the_same_through_the_unmasked_range() {
        let dir = tempfile::tempdir().unwrap();
        let key = hot_pixel_mef(&dir.path().join("hot.fits"));
        let out = compute_histogram(key.clone(), Some(true), None, None).await.unwrap();
        assert_eq!(out[RES_MASKED], true);
        let entry = load_cached(&key).unwrap();
        let full = entry.stats();
        assert_eq!(out[RES_DATA_MAX].as_f64(), Some(full.max), "the returned range is not the one the CPU renders with");
        assert_eq!(out[RES_DATA_MIN].as_f64(), Some(full.min));
        assert_eq!(out[RES_MAX].as_f64(), Some(full.max), "histogram bins and STF use different ranges");

        let mask = resolve_dq_mask(&key, true, entry.arr().dim()).expect("mask");
        let masked = compute_image_stats(&apply_exclusion(entry.arr(), &mask.map).unwrap());
        let intended = auto_stf(&masked, &AutoStfConfig::default());
        let stf = StfParams {
            shadow: out[RES_AUTO_STF][RES_SHADOW].as_f64().unwrap(),
            midtone: out[RES_AUTO_STF][RES_MIDTONE].as_f64().unwrap(),
            highlight: out[RES_AUTO_STF][RES_HIGHLIGHT].as_f64().unwrap(),
        };
        assert_eq!(out[RES_MEDIAN].as_f64(), Some(masked.median));
        for v in [masked.median, masked.median + 2.0 * masked.sigma, masked.max] {
            let want = render_one(v, &intended, &masked) as i32;
            let got = render_one(v, &stf, full) as i32;
            assert!((want - got).abs() <= 1, "value {v}: analysis stretch {want}, CPU/export stretch {got}");
        }
    }

    #[tokio::test]
    async fn histogram_statistics_keep_negative_sky_and_skip_zero_padding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drz.fits").to_str().unwrap().to_string();
        let mut data = ndarray::Array2::from_shape_vec((64, 64), noise(64 * 64, 3.0, 9)).unwrap();
        data.slice_mut(ndarray::s![.., ..16]).fill(0.0);
        crate::infra::fits::writer::write_fits_mono(&path, &data, None).unwrap();

        let out = compute_histogram(path, None, None, None).await.unwrap();
        let median = out[RES_MEDIAN].as_f64().unwrap();
        assert!(median.abs() < 0.3, "sky median {median} of a zero-mean sky");
        assert!(out[RES_DATA_MIN].as_f64().unwrap() < -5.0, "the negative half of the sky is missing");
        assert_eq!(out[RES_TOTAL_PIXELS].as_u64(), Some(64 * 48), "zero padding was counted as sky");
        let sigma = out[RES_SIGMA].as_f64().unwrap();
        assert!((sigma - 3.0).abs() < 0.3, "sigma {sigma} for a true sigma of 3");
    }

    #[tokio::test]
    async fn a_display_referred_output_gets_the_identity_stretch_over_zero_to_one() {
        let dir = tempfile::tempdir().unwrap();
        let data = ndarray::Array2::from_shape_fn((32, 32), |(y, x)| 0.2 + 0.4 * ((y * 32 + x) as f32 / 1024.0));
        let stretched = dir.path().join("m31_arcsinh.fits").to_str().unwrap().to_string();
        let header = crate::service::common::derived_output_header(
            None,
            "arcsinh",
            crate::cmd::common::OutputValues::DisplayReferred,
        );
        crate::cmd::common::write_derived_fits(&stretched, &data, Some(&header)).unwrap();
        let linear = dir.path().join("m31_linear.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&linear, &data, None).unwrap();

        let out = compute_histogram(stretched, None, None, None).await.unwrap();
        let stf = &out[RES_AUTO_STF];
        assert_eq!(
            (stf[RES_SHADOW].as_f64(), stf[RES_MIDTONE].as_f64(), stf[RES_HIGHLIGHT].as_f64()),
            (Some(0.0), Some(0.5), Some(1.0)),
            "a computed stretch was auto-stretched again: {stf}"
        );
        assert_eq!(out[RES_DATA_MIN].as_f64(), Some(0.0));
        assert_eq!(out[RES_DATA_MAX].as_f64(), Some(1.0));
        assert_eq!(out[RES_MIN].as_f64(), Some(0.0));
        assert_eq!(out[RES_MAX].as_f64(), Some(1.0));
        assert!((out[RES_MEDIAN].as_f64().unwrap() - 0.4).abs() < 0.01);

        let plain = compute_histogram(linear, None, None, None).await.unwrap();
        assert_eq!(plain[RES_DATA_MIN].as_f64(), Some(0.2f32 as f64));
        assert_ne!(plain[RES_AUTO_STF][RES_MIDTONE].as_f64(), Some(0.5));
    }

    #[test]
    fn a_dq_table_that_excludes_nothing_leaves_the_measurement_unmasked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hsc_mask.fits");
        let mut mask = vec![0i32; 16];
        mask[5] = 1;
        write_test_mef(
            &path,
            &[("TELESCOP", "'Subaru'".into())],
            &[
                TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(vec![1.0; 16]), extra_cards: vec![] },
                TestHdu { extname: Some("MASK"), extver: None, cols: 4, rows: 4, data: HduData::I32(mask), extra_cards: vec![] },
            ],
        );
        let k = format!("{}#hdu=2", path.to_str().unwrap());
        let dims = load_cached(&k).unwrap().arr().dim();
        assert!(dq_exclusion(&k).unwrap().is_some());
        assert!(resolve_dq_mask(&k, true, dims).is_none());
        assert!(resolve_pixel_mask(&k, true, &[], dims).is_none());
    }

    #[test]
    fn resolve_dq_mask_counts_excluded_pixels_on_mef() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hist.fits");
        let mut dq = vec![-2147483648i32; 16];
        dq[1] = -2147483647;
        dq[6] = -2147483645;
        dq[9] = -2147483646;
        sci_err_dq_mef_with_dq_cards(&path, 4, 4, dq, vec![("TELESCOP", "'JWST'".into())]);
        let key = format!("{}#hdu=1", path.to_str().unwrap());
        let entry = load_cached(&key).unwrap();
        let mask = resolve_dq_mask(&key, true, entry.arr().dim()).expect("mask");
        assert_eq!(mask.excluded, 2);
        assert_eq!(mask.map[[0, 1]], 1);
        assert_eq!(mask.map[[1, 2]], 1);
        assert_eq!(mask.map[[2, 1]], 0);
        let masked = apply_exclusion(entry.arr(), &mask.map).unwrap();
        assert_eq!(compute_image_stats(&masked).valid_count, entry.stats().valid_count - 2);
        assert!(resolve_dq_mask(&key, false, entry.arr().dim()).is_none());
        assert!(resolve_dq_mask(&key, true, (1, 1)).is_none());
        assert!(resolve_dq_mask(&format!("{}#hdu=4", path.to_str().unwrap()), true, (4, 4)).is_none());
    }

    #[tokio::test]
    async fn composite_star_detection_of_an_rgb_file_uses_that_file_and_not_the_blend_slots() {
        let _guard = crate::cmd::helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("osc_stars.fits").to_str().unwrap().to_string();
        let size = 64;
        let star = gaussian_pixels(size, 1000.0, 2.0, 100.0);
        let mut state = 12345u32;
        let r = ndarray::Array2::from_shape_fn((size, size), |(y, x)| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            star[y * size + x] + (state >> 24) as f32 / 64.0
        });
        crate::infra::fits::writer::write_fits_rgb(&path, &r, &r, &r, None).unwrap();
        let blend = ndarray::Array2::from_elem((size, size), 7.0f32);
        let blend_stats = compute_image_stats(&blend);
        crate::cmd::helpers::insert_composite_and_orig(
            blend.clone(),
            blend.clone(),
            blend,
            blend_stats.clone(),
            blend_stats.clone(),
            blend_stats,
        );

        let of_file = detect_stars_composite(5.0, 200, Some(path)).await;
        let of_slots = detect_stars_composite(5.0, 200, None).await;
        crate::cmd::helpers::clear_composite();
        let of_file = of_file.unwrap();
        let stars = of_file["stars"].as_array().unwrap();
        assert!(!stars.is_empty(), "the star in the RGB file was not found");
        assert_eq!(of_file[RES_N_DETECTED].as_u64(), Some(stars.len() as u64));
        let (x, y) = (stars[0]["x"].as_f64().unwrap(), stars[0]["y"].as_f64().unwrap());
        assert!((x - 32.0).abs() < 1.0 && (y - 32.0).abs() < 1.0, "brightest star at ({x}, {y})");
        assert!(of_slots.unwrap()["stars"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn composite_star_detection_measures_native_luminance_and_keeps_padding_at_exact_zero() {
        let _guard = crate::cmd::helpers::composite_test_lock().await;
        let size = 256;
        let pad = 64;
        let star = gaussian_pixels(size, 4000.0, 2.0, 0.0);
        let mut state = 4242u32;
        let mut noise = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 24) as f32 - 127.5) / 8.0
        };
        let mut plane = |gain: f32| {
            ndarray::Array2::from_shape_fn((size, size), |(y, x)| {
                if y < pad && x < pad {
                    0.0
                } else {
                    gain * (10.0 + star[y * size + x] + noise())
                }
            })
        };
        let r = plane(11.4);
        let mut g = plane(1.0);
        let b = plane(12.1);
        g[[200, 40]] = -60.0;

        let lum = crate::core::imaging::luminance::rgb_to_luminance(&r, &g, &b);
        assert!(lum.iter().cloned().fold(f32::INFINITY, f32::min) < 0.0, "the luminance needs a negative minimum");
        assert_eq!(lum.iter().filter(|&&v| v == 0.0).count(), pad * pad, "padding must stay exactly zero");
        let expected = detect_stars_core(&lum, 5.0);
        assert!(expected.background_median > 1.0, "expected native units, got {}", expected.background_median);
        assert!(!expected.stars.is_empty());

        crate::cmd::helpers::insert_composite_and_orig(
            r.clone(),
            g.clone(),
            b.clone(),
            compute_image_stats(&r),
            compute_image_stats(&g),
            compute_image_stats(&b),
        );
        let of_slots = detect_stars_composite(5.0, 200, None).await;
        crate::cmd::helpers::clear_composite();
        let of_slots = of_slots.unwrap();

        assert_eq!(of_slots["background_median"].as_f64(), Some(expected.background_median));
        assert_eq!(of_slots["background_sigma"].as_f64(), Some(expected.background_sigma));
        assert_eq!(of_slots[RES_N_DETECTED].as_u64(), Some(expected.stars.len() as u64));
        assert_eq!(of_slots["image_width"].as_u64(), Some(size as u64));
        let brightest = &of_slots["stars"][0];
        assert_eq!(brightest["flux"].as_f64(), Some(expected.stars[0].flux));
        assert!((brightest["x"].as_f64().unwrap() - 128.0).abs() < 1.0);
    }

    #[tokio::test]
    async fn composite_star_detection_refuses_planes_of_different_sizes() {
        let _guard = crate::cmd::helpers::composite_test_lock().await;
        let r = ndarray::Array2::from_elem((16, 16), 3.0f32);
        let g = ndarray::Array2::from_elem((16, 12), 3.0f32);
        let b = ndarray::Array2::from_elem((16, 16), 3.0f32);
        crate::cmd::helpers::insert_composite_and_orig(
            r.clone(),
            g.clone(),
            b.clone(),
            compute_image_stats(&r),
            compute_image_stats(&g),
            compute_image_stats(&b),
        );
        let result = detect_stars_composite(5.0, 200, None).await;
        crate::cmd::helpers::clear_composite();
        let err = result.unwrap_err();
        assert!(err.contains("(16, 16)") && err.contains("(16, 12)"), "{err}");
    }

    fn gaussian_fits(dir: &std::path::Path, name: &str) -> String {
        let size = 64;
        let mut arr = ndarray::Array2::<f32>::zeros((size, size));
        for (i, v) in gaussian_pixels(size, 1000.0, 2.0, 100.0).into_iter().enumerate() {
            arr[[i / size, i % size]] = v;
        }
        let path = dir.join(name);
        crate::infra::fits::writer::write_fits_mono(path.to_str().unwrap(), &arr, None).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn batch_photometry_of_one_point_equals_the_single_measurement_field_by_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "batch.fits");
        let single = measure_photometry_cmd(path.clone(), 31.0, 33.0, Some(5.0), Some(10.0), Some(15.0), Some(false), None, Some(2.0), None)
            .await
            .unwrap();
        let batch = measure_photometry_batch_cmd(path.clone(), vec![(31.0, 33.0)], Some(5.0), Some(10.0), Some(15.0), Some(2.0), None, Some(true), None)
            .await
            .unwrap();
        let rows = batch[RES_ROWS].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][RES_INDEX], 0);
        assert!(rows[0][RES_ERROR].is_null());
        assert_eq!(rows[0][RES_PHOTOMETRY], single[RES_PHOTOMETRY]);
        assert_eq!(rows[0][RES_SKY], single[RES_SKY]);
        assert_eq!(batch[RES_PHOTCAL], single[RES_PHOTCAL]);
        assert_eq!(batch[RES_WARNINGS], single[RES_WARNINGS]);
        assert_eq!(batch[RES_MASKED], false);
        assert_eq!(batch[RES_N_MEASURED], 1);
        assert_eq!(batch[RES_N_FAILED], 0);
        let phot = &rows[0][RES_PHOTOMETRY];
        assert_eq!(phot["sky_inner"], 10.0);
        assert_eq!(phot["sky_outer"], 15.0);
        assert!(!phot["growth_curve"].as_array().unwrap().is_empty());
        assert!(phot["growth_curve"][0]["r"].is_number() && phot["growth_curve"][0]["flux"].is_number());

        let mixed = measure_photometry_batch_cmd(path, vec![(32.0, 32.0), (5.0, 5.0)], None, None, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(mixed[RES_N_MEASURED], 2);
        assert_eq!(mixed[RES_ROWS][1][RES_INDEX], 1);
    }

    fn sky_only_warnings(out: &serde_json::Value) -> usize {
        out[RES_WARNINGS]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w.as_str() == Some(SKY_ONLY_ERRORS_WARNING))
            .count()
    }

    #[test]
    fn click_photometry_without_gain_or_err_plane_warns_about_photon_noise() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "photon_noise.fits");
        let without = photometry_for_path(&path, 32.0, 32.0, None, None, false, false, None).unwrap();
        assert_eq!(sky_only_warnings(&without), 1, "{}", without[RES_WARNINGS]);
        assert!(without[RES_GAIN_USED].is_null(), "{}", without[RES_GAIN_USED]);
        let with = photometry_for_path(&path, 32.0, 32.0, None, None, false, false, Some(1.5)).unwrap();
        assert_eq!(sky_only_warnings(&with), 0, "{}", with[RES_WARNINGS]);
        assert_eq!(with[RES_GAIN_USED], 1.5);
    }

    #[tokio::test]
    async fn batch_photometry_without_gain_warns_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "batch_photon_noise.fits");
        let points = vec![(32.0, 32.0), (30.0, 34.0), (34.0, 30.0)];
        let out = measure_photometry_batch_cmd(path.clone(), points.clone(), Some(5.0), None, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(out[RES_N_MEASURED], 3);
        assert_eq!(sky_only_warnings(&out), 1, "{}", out[RES_WARNINGS]);
        assert!(out[RES_GAIN_USED].is_null(), "{}", out[RES_GAIN_USED]);
        let with = measure_photometry_batch_cmd(path, points, Some(5.0), None, None, Some(2.0), None, None, None).await.unwrap();
        assert_eq!(sky_only_warnings(&with), 0, "{}", with[RES_WARNINGS]);
        assert_eq!(with[RES_GAIN_USED], 2.0);
    }

    #[tokio::test]
    async fn photometry_gain_model_cmd_reads_egain_from_the_measured_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut header = HduHeader::empty();
        header.set("EGAIN", "0.25".to_string());
        let path = dir.path().join("egain.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&path, &ndarray::Array2::from_elem((16, 16), 100.0f32), Some(&header))
            .unwrap();
        let model = photometry_gain_model_cmd(path).await.unwrap();
        assert_eq!(model["effective_gain"], 0.25, "{model}");
        assert_eq!(model["poisson_route"], "header_gain", "{model}");
        assert_eq!(model["gain_e_per_adu"], 0.25);
        assert_eq!(model["source"], "EGAIN");
        assert_eq!(model["unit_class"], "counts");
        assert_eq!(model["combine_scaled"], false);
        assert!(model["fallback_gain"].is_null());
    }

    #[tokio::test]
    async fn photometry_gain_model_cmd_routes_a_rate_file_with_an_err_plane_to_err_plane() {
        let dir = tempfile::tempdir().unwrap();
        let size = 16;
        let path = dir.path().join("rate_err.fits");
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu {
                    extname: Some("SCI"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: HduData::F32(vec![100.0; size * size]),
                    extra_cards: vec![("BUNIT", "'DN/s'".into())],
                },
                TestHdu {
                    extname: Some("ERR"),
                    extver: Some(1),
                    cols: size,
                    rows: size,
                    data: HduData::F32(vec![0.5; size * size]),
                    extra_cards: vec![("BUNIT", "'DN/s'".into())],
                },
            ],
        );
        let plain = path.to_str().unwrap().to_string();
        let model = photometry_gain_model_cmd(plain.clone()).await.unwrap();
        assert_eq!(model["poisson_route"], "err_plane", "{model}");
        assert!(model["effective_gain"].is_null(), "{model}");
        assert!(model["fallback_gain"].is_null(), "{model}");
        assert_eq!(model["unit_class"], "count_rate", "{model}");
        assert_eq!(model["note"], "Poisson noise from the ERR plane", "{model}");
        let keyed = photometry_gain_model_cmd(format!("{plain}#hdu=1")).await.unwrap();
        assert_eq!(keyed["poisson_route"], "err_plane", "{keyed}");
    }

    fn star_cube_hdu(extname: &'static str, cols: usize, rows: usize, planes: usize, amp: f32) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let (cards, _) = cube_hdu(extname, cols, rows, planes, &[("BUNIT", "'MJy/sr'".into())]);
        let plane: Vec<f32> = (0..cols * rows)
            .map(|i| {
                let x = (i % cols) as f32 - (cols / 2) as f32;
                let y = (i / cols) as f32 - (rows / 2) as f32;
                0.5 + amp * (-(x * x + y * y) / 8.0).exp()
            })
            .collect();
        let data = (0..planes).flat_map(|_| plane.iter().flat_map(|v| v.to_be_bytes())).collect();
        (cards, data)
    }

    #[tokio::test]
    async fn a_three_d_err_cube_is_not_the_err_plane_photometry_uses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calints.fits");
        let (cols, rows, planes) = (24, 16, 5);
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                star_cube_hdu("SCI", cols, rows, planes, 50.0),
                star_cube_hdu("ERR", cols, rows, planes, 0.0),
            ],
        );
        let plain = format!("{}#hdu=1", path.to_str().unwrap());
        let model = photometry_gain_model_cmd(plain.clone()).await.unwrap();
        assert_ne!(model["poisson_route"], "err_plane", "{model}");
        assert_eq!(model["poisson_route"], "unavailable", "{model}");
        assert_eq!(model["unit_class"], "calibrated", "{model}");
        let out = photometry_for_path(&plain, 12.0, 8.0, Some(3.0), None, false, false, None).unwrap();
        assert_eq!(out[RES_PHOTOMETRY]["err_used"], false, "{}", out[RES_PHOTOMETRY]);
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(
            !warnings.iter().any(|w| w.as_str().unwrap().contains("Poisson noise from the ERR plane")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap().ends_with("JWST/Roman calibrated units (MJy/sr): no gain applies")),
            "{warnings:?}"
        );
        assert!(!has_err_extension(&plain, (rows, cols)));
    }

    #[test]
    fn has_err_extension_sees_the_err_plane_of_a_gzip_compressed_mef() {
        use crate::infra::fits::gzip::test_support::gzip_copy;
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("cal.fits");
        sci_err_dq_mef_with_dq_cards(&plain, 6, 5, vec![0; 30], vec![]);
        let gz = gzip_copy(&plain, "cal.fits.gz");
        assert!(has_err_extension(plain.to_str().unwrap(), (5, 6)));
        assert!(has_err_extension(gz.to_str().unwrap(), (5, 6)));
    }

    #[tokio::test]
    async fn photometry_gain_model_cmd_counts_an_asdf_err_array() {
        let dir = tempfile::tempdir().unwrap();
        let tree = "data: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\nerr: !core/ndarray-1.0.0\n  data: [[0.1, 0.2], [0.3, 0.4]]\n  datatype: float32\n";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(tree.as_bytes());
        bytes.extend_from_slice(b"...\n");
        let path = dir.path().join("with_err.asdf");
        std::fs::write(&path, bytes).unwrap();
        let plain = path.to_str().unwrap().to_string();
        assert!(!has_err_extension(&plain, (2, 2)));
        let model = photometry_gain_model_cmd(plain).await.unwrap();
        assert_eq!(model["poisson_route"], "err_plane", "{model}");
        assert_eq!(model["note"], "Poisson noise from the ERR plane", "{model}");
    }

    #[tokio::test]
    async fn photometry_gain_model_cmd_accepts_a_headerless_in_memory_key() {
        let key = "__r2_polish_headerless_gain_probe";
        let arr = ndarray::Array2::<f32>::from_elem((8, 8), 10.0);
        crate::infra::cache::GLOBAL_IMAGE_CACHE.insert_synthetic(key, std::sync::Arc::new(arr.clone()), compute_image_stats(&arr));
        let model = photometry_gain_model_cmd(key.to_string()).await;
        crate::infra::cache::GLOBAL_IMAGE_CACHE.remove(key);
        let model = model.unwrap();
        assert_eq!(model["poisson_route"], "unavailable", "{model}");
        assert_eq!(model["note"], "no gain in the header", "{model}");
        assert!(model["gain_e_per_adu"].is_null(), "{model}");
    }

    #[tokio::test]
    async fn photometry_results_carry_the_pixel_unit_and_the_unit_of_a_sum() {
        let dir = tempfile::tempdir().unwrap();
        let jwst = jwst_star_mef(&dir.path().join("jwst_units.fits"), vec![]);
        let plain = gaussian_fits(dir.path(), "plain_units.fits");
        let adu = dir.path().join("adu_units.fits").to_str().unwrap().to_string();
        let arr = ndarray::Array2::from_shape_vec((64, 64), gaussian_pixels(64, 1000.0, 2.0, 100.0)).unwrap();
        let mut header = HduHeader::empty();
        header.set("BUNIT", "adu".to_string());
        crate::infra::fits::writer::write_fits_mono(&adu, &arr, Some(&header)).unwrap();
        let cases = [
            (jwst, json!("MJy/sr"), json!("MJy/sr x pix")),
            (plain, serde_json::Value::Null, serde_json::Value::Null),
            (adu, json!("adu"), json!("adu")),
        ];
        for (path, bunit, summed_unit) in cases {
            let single = photometry_for_path(&path, 32.0, 32.0, None, None, false, false, None).unwrap();
            let batch = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], None, None, None, None, None, None, None)
                .await
                .unwrap();
            for out in [&single, &batch] {
                assert_eq!(out["bunit"], bunit, "{path}");
                assert_eq!(out["summed_unit"], summed_unit, "{path}");
            }
        }
    }

    #[test]
    fn the_scale_helpers_read_a_rotated_cd_and_refuse_a_singular_one() {
        let (n, c) = 30f64.to_radians().sin_cos();
        let s = 0.5 / 3600.0;
        let cd = [[-c * s, -n * s], [-n * s, c * s]];
        let wcs = WcsTransform::from_header(&crate::core::imaging::region::test_support::header_with_cd(cd)).unwrap();
        let (sx, sy) = axis_scales_arcsec(&wcs);
        assert!((sx - 0.5).abs() < 1e-12, "{sx}");
        assert!((sy - 0.5).abs() < 1e-12, "{sy}");
        let area = pixel_area_scale_arcsec(&wcs).unwrap();
        assert!((area - 0.5).abs() < 1e-12, "{area}");
        assert_eq!(axis_scale_warning(&wcs), None);
        assert_eq!(area_scale_of_cd([[0.0, 0.0], [0.0, 0.0]]), None);
        assert_eq!(area_scale_of_cd([[1e-4, 1e-4], [1e-4, 1e-4]]), None);
    }

    const NO_DQ_PLANE_WARNING: &str = "DQ exclusion requested but not applied: this image has no DQ plane";
    const LAYER_STAR_SIZE: usize = 32;

    fn layer_star_sci(extver: Option<i64>) -> TestHdu {
        TestHdu {
            extname: Some("SCI"),
            extver,
            cols: LAYER_STAR_SIZE,
            rows: LAYER_STAR_SIZE,
            data: HduData::F32(gaussian_pixels(LAYER_STAR_SIZE, 1000.0, 2.0, 100.0)),
            extra_cards: vec![("BUNIT", "'DN/s'".into())],
        }
    }

    fn layer_star_companion(extname: &'static str, extver: Option<i64>, data: HduData) -> TestHdu {
        TestHdu { extname: Some(extname), extver, cols: LAYER_STAR_SIZE, rows: LAYER_STAR_SIZE, data, extra_cards: vec![] }
    }

    fn float_dq_mef(dir: &std::path::Path) -> String {
        let path = dir.join("float_dq.fits");
        write_test_mef(
            &path,
            &[("TELESCOP", "'JWST'".into())],
            &[
                layer_star_sci(Some(1)),
                layer_star_companion("DQ", Some(1), HduData::F32(vec![0.0; LAYER_STAR_SIZE * LAYER_STAR_SIZE])),
            ],
        );
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn unknown_flags_mef(dir: &std::path::Path) -> String {
        let path = dir.join("unknown_flags.fits");
        let mut dq = vec![0i32; LAYER_STAR_SIZE * LAYER_STAR_SIZE];
        dq[5] = 1;
        write_test_mef(
            &path,
            &[("TELESCOP", "'Subaru'".into())],
            &[layer_star_sci(None), layer_star_companion("DQ", None, HduData::I32(dq))],
        );
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    fn dq_not_applied_warnings(out: &serde_json::Value) -> Vec<String> {
        out[RES_WARNINGS]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap().to_string())
            .filter(|w| w.starts_with(DQ_NOT_APPLIED_PREFIX))
            .collect()
    }

    fn layer_star_photometry(key: &str, exclude_dq: bool) -> serde_json::Value {
        let centre = (LAYER_STAR_SIZE / 2) as f64;
        photometry_for_path(key, centre, centre, Some(5.0), None, false, exclude_dq, None).unwrap()
    }

    #[tokio::test]
    async fn an_err_plane_of_another_extver_is_not_the_err_plane_of_this_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two_sci.fits");
        sci_err_dq_mef_with_dq_cards(&path, 6, 5, vec![0; 30], vec![]);
        let dims = (5, 6);
        let key4 = format!("{}#hdu=4", path.to_str().unwrap());
        assert!(photometry_planes(&key4, dims).err.is_none());
        assert!(!has_err_extension(&key4, dims), "SCI,2 has no ERR of its own");
        let model4 = photometry_gain_model_cmd(key4).await.unwrap();
        assert_ne!(model4["poisson_route"], "err_plane", "{model4}");

        let key1 = format!("{}#hdu=1", path.to_str().unwrap());
        assert!(has_err_extension(&key1, dims));
        let model1 = photometry_gain_model_cmd(key1).await.unwrap();
        assert_eq!(model1["poisson_route"], "err_plane", "{model1}");
    }

    #[tokio::test]
    async fn the_gain_model_and_the_measurement_agree_on_the_err_plane_when_extver_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("err_without_extver.fits");
        let mut err = layer_star_companion("ERR", None, HduData::F32(vec![0.5; LAYER_STAR_SIZE * LAYER_STAR_SIZE]));
        err.extra_cards = vec![("BUNIT", "'DN/s'".into())];
        write_test_mef(&path, &[], &[layer_star_sci(Some(1)), err]);
        let key = format!("{}#hdu=1", path.to_str().unwrap());
        let used = layer_star_photometry(&key, false)[RES_PHOTOMETRY]["err_used"] == true;
        let said = photometry_gain_model_cmd(key).await.unwrap()["poisson_route"] == "err_plane";
        assert_eq!(said, used, "gain model says ERR plane: {said}; measurement used one: {used}");
    }

    fn error_reference(path: &str) -> Option<crate::types::image_ref::ImageRef> {
        let entry = load_cached_full(path).or_else(|_| load_cached(path)).ok()?;
        entry.companions().and_then(|c| c.err.clone())
    }

    fn star_photometry(path: &str) -> serde_json::Value {
        photometry_for_path(path, 32.0, 32.0, Some(5.0), None, false, false, None).unwrap()
    }

    #[tokio::test]
    async fn the_gain_model_sees_an_error_layer_that_sits_in_a_sibling_file() {
        let dir = tempfile::tempdir().unwrap();
        let image = legacy_brick(dir.path(), Some(SIZE));
        let out = star_photometry(&image);
        assert_eq!(out[RES_PHOTOMETRY]["err_used"], true, "the sibling pairing of SURVEY-LAYERS-R3: {}", out[RES_WARNINGS]);
        assert!(has_err_extension(&image, (SIZE, SIZE)), "error reference {:?}", error_reference(&image));
        let model = photometry_gain_model_cmd(image).await.unwrap();
        assert_eq!(model["poisson_route"], "err_plane", "{model}");
    }

    #[tokio::test]
    async fn the_gain_model_and_the_measurement_agree_when_a_sibling_cannot_be_used() {
        for invvar_size in [Some(SIZE / 2), None] {
            let dir = tempfile::tempdir().unwrap();
            let image = legacy_brick(dir.path(), invvar_size);
            let out = star_photometry(&image);
            assert_eq!(out[RES_PHOTOMETRY]["err_used"], false, "invvar {invvar_size:?}: {}", out[RES_WARNINGS]);
            assert!(!has_err_extension(&image, (SIZE, SIZE)), "invvar {invvar_size:?}: {:?}", error_reference(&image));
            let model = photometry_gain_model_cmd(image).await.unwrap();
            assert_ne!(model["poisson_route"], "err_plane", "invvar {invvar_size:?}: {model}");
        }
    }

    #[test]
    fn the_gain_model_sees_the_error_plane_of_one_band_of_a_cutout_cube() {
        let dir = tempfile::tempdir().unwrap();
        let key = legacy_band_cutout(&dir.path().join("cut.fits"), SIZE);
        let out = star_photometry(&key);
        assert_eq!(out[RES_PHOTOMETRY]["err_used"], true, "the band-plane pairing of SURVEY-LAYERS-R7: {}", out[RES_WARNINGS]);
        assert!(has_err_extension(&key, (SIZE, SIZE)), "error reference {:?}", error_reference(&key));
        let small = legacy_band_cutout(&dir.path().join("small_error_cut.fits"), SIZE / 2);
        assert!(!has_err_extension(&small, (SIZE, SIZE)), "error reference {:?}", error_reference(&small));
    }

    #[test]
    fn dq_exclusion_that_cannot_be_applied_is_reported_in_the_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let key = float_dq_mef(dir.path());
        let out = layer_star_photometry(&key, true);
        assert_eq!(out[RES_MASKED], false);
        let reason = load_companions(&key).unwrap().dq_unavailable.expect("a floating-point DQ plane says why it is no mask");
        assert_eq!(
            dq_not_applied_warnings(&out),
            vec![format!("{DQ_NOT_APPLIED_PREFIX}{reason}")],
            "{}",
            out[RES_WARNINGS]
        );
        let not_requested = layer_star_photometry(&key, false);
        assert!(dq_not_applied_warnings(&not_requested).is_empty(), "{}", not_requested[RES_WARNINGS]);
    }

    #[test]
    fn dq_exclusion_on_an_image_without_a_dq_plane_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "plain.fits");
        let out = photometry_for_path(&path, 32.0, 32.0, Some(5.0), None, false, true, None).unwrap();
        assert_eq!(dq_not_applied_warnings(&out), vec![NO_DQ_PLANE_WARNING.to_string()], "{}", out[RES_WARNINGS]);
        assert_eq!(out[RES_MASKED], false);
    }

    #[test]
    fn dq_exclusion_with_unknown_flag_definitions_says_that_no_bit_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let key = unknown_flags_mef(dir.path());
        let out = layer_star_photometry(&key, true);
        assert_eq!(dq_not_applied_warnings(&out), vec![unnamed_flags_warning(&key)], "{}", out[RES_WARNINGS]);
        assert_eq!(out[RES_MASKED], false);
    }

    #[test]
    fn a_usable_dq_plane_raises_no_warning_and_the_layer_report_names_both_planes() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(&dir.path().join("cal.fits"), &[], cal_sci_cards(), CalPlanes::ALL, vec![]);
        let ctx = photometry_context(&fixture.key, true).unwrap();
        let loaded = load_companions(&fixture.key).unwrap();
        let err_source = loaded.err_origin.map(|origin| origin.source);
        assert!(err_source.is_some() && loaded.dq_origin.is_some());
        assert_eq!(
            ctx.layers,
            LayerReport {
                err_source: err_source.clone(),
                dq_source: loaded.dq_origin,
                dq_flags: Some("jwst".into()),
                dq_exclusion_available: true,
                dq_gap: None,
                err_note: None,
                err_gap: None,
            }
        );
        assert!(ctx.mask.is_some());
        assert!(!ctx.warnings.iter().any(|w| w.starts_with(DQ_NOT_APPLIED_PREFIX)), "{:?}", ctx.warnings);

        let without_dq = jwst_cal_mef(&dir.path().join("cal_sci_err.fits"), &[], cal_sci_cards(), CalPlanes::SCI_ERR, vec![]);
        let ctx = photometry_context(&without_dq.key, true).unwrap();
        assert_eq!(
            ctx.layers,
            LayerReport {
                err_source,
                dq_source: None,
                dq_flags: None,
                dq_exclusion_available: false,
                dq_gap: Some("this image has no DQ plane".into()),
                err_note: None,
                err_gap: None,
            }
        );
        assert!(ctx.mask.is_none());
        assert_eq!(ctx.warnings.last().map(String::as_str), Some(NO_DQ_PLANE_WARNING), "{:?}", ctx.warnings);
    }

    #[test]
    fn the_layer_report_and_resolve_dq_mask_agree_on_every_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let keys = [
            float_dq_mef(dir.path()),
            gaussian_fits(dir.path(), "plain.fits"),
            unknown_flags_mef(dir.path()),
            jwst_cal_mef(&dir.path().join("cal.fits"), &[], cal_sci_cards(), CalPlanes::ALL, vec![]).key,
            afw_star_mef(&dir.path().join("calexp.fits"), flat("VARIANCE", 4.0)),
            unnamed_flag_star_mef(&dir.path().join("flag.fits")),
        ];
        let available: Vec<bool> = keys
            .iter()
            .map(|key| {
                let entry = load_cached_full(key).unwrap();
                let report = layer_report(key, &entry);
                assert_eq!(
                    report.dq_exclusion_available,
                    resolve_dq_mask(key, true, entry.arr().dim()).is_some(),
                    "{key}: {report:?}"
                );
                assert_eq!(report.dq_gap.is_none(), report.dq_exclusion_available, "{key}: {report:?}");
                report.dq_exclusion_available
            })
            .collect();
        assert_eq!(available, vec![false, false, false, true, true, false]);
    }

    #[tokio::test]
    async fn the_warning_order_keeps_batch_and_single_equal() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "plain.fits");
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, Some(5.0), None, None, Some(false), Some(true), Some(2.0), None)
            .await
            .unwrap();
        let batch = measure_photometry_batch_cmd(path, vec![(32.0, 32.0)], Some(5.0), None, None, Some(2.0), Some(true), None, None)
            .await
            .unwrap();
        assert_eq!(batch[RES_WARNINGS], single[RES_WARNINGS]);
        assert_eq!(single[RES_WARNINGS].as_array().unwrap().last().unwrap(), NO_DQ_PLANE_WARNING, "{}", single[RES_WARNINGS]);
    }

    #[tokio::test]
    async fn a_sky_annulus_inside_the_aperture_is_refused_by_both_commands() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "refused.fits");
        let sentence = "sky annulus inner radius 5 must be larger than the aperture radius 5";
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, Some(5.0), Some(5.0), Some(10.0), Some(false), None, None, None)
            .await
            .unwrap_err();
        assert!(single.contains(sentence), "{single}");
        let batch = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], Some(5.0), Some(5.0), Some(10.0), None, None, None, None)
            .await
            .unwrap_err();
        assert!(batch.contains(sentence), "{batch}");

        let auto_radius = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], None, Some(2.0), Some(4.0), None, None, None, None)
            .await
            .unwrap();
        assert_eq!(auto_radius[RES_N_FAILED], 1);
        assert!(auto_radius[RES_ROWS][0][RES_ERROR].as_str().unwrap().contains("must be larger than the aperture radius"));
        assert!(auto_radius[RES_ROWS][0][RES_PHOTOMETRY].is_null());

        let half = measure_photometry_cmd(path.clone(), 32.0, 32.0, None, Some(8.0), None, Some(false), None, None, None)
            .await
            .unwrap_err();
        assert!(half.contains("sky annulus needs both an inner and an outer radius"), "{half}");
        let inverted = measure_photometry_cmd(path, 32.0, 32.0, None, Some(12.0), Some(8.0), Some(false), None, None, None)
            .await
            .unwrap_err();
        assert!(inverted.contains("sky annulus outer radius 8 must be larger than the inner radius 12"), "{inverted}");
    }

    #[tokio::test]
    async fn batch_photometry_refuses_too_many_empty_or_non_finite_points() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "limits.fits");
        let too_many = vec![(32.0, 32.0); 6000];
        let err = measure_photometry_batch_cmd(path.clone(), too_many, None, None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("6000 points exceeds the limit of 5000"), "{err}");
        let empty = measure_photometry_batch_cmd(path.clone(), vec![], None, None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(empty.contains("at least one point"), "{empty}");
        let nan = measure_photometry_batch_cmd(path, vec![(32.0, f64::NAN)], None, None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(nan.contains("point 0"), "{nan}");
    }

    #[tokio::test]
    async fn batch_photometry_without_growth_curves_still_reports_encircled_energy_radii() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "growth.fits");
        let out = measure_photometry_batch_cmd(path, vec![(32.0, 32.0)], Some(6.0), None, None, None, None, Some(false), None)
            .await
            .unwrap();
        let phot = &out[RES_ROWS][0][RES_PHOTOMETRY];
        assert_eq!(phot["growth_curve"].as_array().unwrap().len(), 0);
        assert!(phot["flux_total"].is_number(), "{phot}");
        let ee50 = phot["ee50_radius"].as_f64().expect("EE50");
        let ee80 = phot["ee80_radius"].as_f64().expect("EE80");
        assert!((ee50 - 1.1774 * 2.0).abs() / (1.1774 * 2.0) < 0.05, "EE50 {ee50}");
        assert!(ee80 > ee50);
    }

    #[tokio::test]
    async fn batch_photometry_reports_positions_outside_the_image_as_failed_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "outside.fits");
        let points = vec![(32.0, 32.0), (3000.0, 3000.0), (-500.0, 10.0), (70.0, 32.0)];
        let out = measure_photometry_batch_cmd(path, points, Some(5.0), None, None, None, None, None, None).await.unwrap();
        assert_eq!(out[RES_N_MEASURED], 1);
        assert_eq!(out[RES_N_FAILED], 3);
        for i in [1usize, 2, 3] {
            let row = &out[RES_ROWS][i];
            assert!(row[RES_PHOTOMETRY].is_null(), "{row}");
            assert!(row[RES_ERROR].as_str().unwrap().contains("lies outside the 64 x 64 image"), "{row}");
        }
        assert_eq!(out[RES_ROWS][2][RES_ERROR], "position (-500, 10) lies outside the 64 x 64 image");
    }

    #[tokio::test]
    async fn a_sky_annulus_wider_than_the_limit_is_refused_by_every_photometry_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "wide_annulus.fits");
        let sentence = "sky annulus outer radius 3000 must be at most 512 pixels";
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, None, Some(20.0), Some(3000.0), Some(false), None, None, None)
            .await
            .unwrap_err();
        assert!(single.contains(sentence), "{single}");
        let batch = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0); 100], Some(5.0), Some(20.0), Some(3000.0), None, None, None, None)
            .await
            .unwrap_err();
        assert!(batch.contains(sentence), "{batch}");
        assert_eq!(annulus_arg(Some(20.0), Some(MAX_SKY_ANNULUS_RADIUS)).unwrap(), Some((20.0, MAX_SKY_ANNULUS_RADIUS)));
        assert!(annulus_arg(Some(20.0), Some(512.5)).is_err());
    }

    #[tokio::test]
    async fn an_aperture_radius_outside_the_measurable_range_is_refused_once_by_both_commands() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "aperture_range.fits");
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, Some(1.5), None, None, Some(false), None, None, None)
            .await
            .unwrap_err();
        assert!(single.contains("aperture radius 1.5 must be between 2 and 60 pixels"), "{single}");
        let small = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0); 3], Some(1.5), Some(1.8), Some(4.0), None, None, None, None)
            .await
            .unwrap_err();
        assert!(small.contains("aperture radius 1.5 must be between 2 and 60 pixels"), "{small}");
        let large = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], Some(100.0), None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(large.contains("aperture radius 100 must be between 2 and 60 pixels"), "{large}");
        let single_large = measure_photometry_cmd(path.clone(), 32.0, 32.0, Some(100.0), None, None, Some(false), None, None, None)
            .await
            .unwrap_err();
        assert!(single_large.contains("aperture radius 100 must be between 2 and 60 pixels"), "{single_large}");
        let nan = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], Some(f64::NAN), None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(nan.contains("aperture radius NaN"), "{nan}");
        let edge = measure_photometry_batch_cmd(path, vec![(32.0, 32.0)], Some(2.0), None, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(edge[RES_N_MEASURED], 1);
        assert_eq!(edge[RES_ROWS][0][RES_PHOTOMETRY]["aperture_radius"], 2.0);
    }

    #[tokio::test]
    async fn batch_photometry_reports_a_sky_annulus_with_no_usable_pixel_as_a_failed_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "thin_annulus.fits");
        let out = measure_photometry_batch_cmd(path, vec![(32.0, 32.0)], Some(5.0), Some(10.0), Some(10.3), None, None, None, None)
            .await
            .unwrap();
        assert_eq!(out[RES_N_FAILED], 1);
        assert!(out[RES_ROWS][0][RES_PHOTOMETRY].is_null());
        let error = out[RES_ROWS][0][RES_ERROR].as_str().unwrap();
        assert!(error.starts_with("sky annulus 10.0 - 10.3 px around (32.0, 32.0) has no usable pixel"), "{error}");
    }

    #[tokio::test]
    async fn batch_photometry_with_dq_exclusion_equals_the_single_measurement_for_every_point() {
        let dir = tempfile::tempdir().unwrap();
        let key = jwst_star_mef(&dir.path().join("jwst_batch_dq.fits"), vec![]);
        let points: Vec<(f64, f64)> = (0..70).map(|i| (24.0 + (i % 10) as f64 * 1.7, 24.0 + (i / 10) as f64 * 2.3)).collect();
        assert!(points.len() > PAR_BATCH_POINTS);
        let batch = measure_photometry_batch_cmd(key.clone(), points.clone(), None, None, None, None, Some(true), Some(true), None)
            .await
            .unwrap();
        assert_eq!(batch[RES_MASKED], true);
        assert_eq!(batch[RES_N_MEASURED], 70);
        for (i, &(x, y)) in points.iter().enumerate() {
            let single = photometry_for_path(&key, x, y, None, None, false, true, None).unwrap();
            assert_eq!(batch[RES_ROWS][i][RES_PHOTOMETRY], single[RES_PHOTOMETRY], "point {i}");
        }
        assert_eq!(batch[RES_ROWS][0][RES_PHOTOMETRY]["n_masked"], 1);
    }

    #[test]
    fn a_capped_detection_reports_the_count_before_the_cap() {
        let star = |flux: f64| crate::core::analysis::star_detection::DetectedStar {
            x: 1.0,
            y: 1.0,
            flux,
            fwhm: 2.0,
            eccentricity: 0.0,
            peak: flux,
            npix: 5,
            snr: 10.0,
        };
        let result = DetectionResult {
            stars: (0..5).map(|i| star(100.0 - i as f64)).collect(),
            background_median: 0.0,
            background_sigma: 1.0,
            threshold_sigma: 5.0,
            image_width: 4,
            image_height: 4,
        };
        let capped = capped_detection_json(result.clone(), 2, Instant::now()).unwrap();
        assert_eq!(capped[RES_N_DETECTED], 5);
        assert_eq!(capped["stars"].as_array().unwrap().len(), 2);
        assert_eq!(capped["stars"][0]["flux"], 100.0);
        assert!(capped[RES_ELAPSED_MS].is_number());
        let uncapped = capped_detection_json(result, 200, Instant::now()).unwrap();
        assert_eq!(uncapped[RES_N_DETECTED], 5);
        assert_eq!(uncapped["stars"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn the_fft_header_carries_the_padded_size_per_axis_and_the_downsampled_flag() {
        let fft = FftResult {
            spectrum: ndarray::Array2::from_elem((2, 3), 0.5f32),
            display_width: 3,
            display_height: 2,
            padded_rows: 2048,
            padded_cols: 4096,
            downsampled: true,
            image_rows: 1025,
            image_cols: 20,
        };
        let header = fft_header(&fft, 1.5, 7.0, 42);
        assert_eq!((header.len(), FFT_HEADER_BYTES), (40, 40));
        let u32_at = |o: usize| u32::from_le_bytes(header[o..o + 4].try_into().unwrap());
        let f32_at = |o: usize| f32::from_le_bytes(header[o..o + 4].try_into().unwrap());
        assert_eq!((u32_at(0), u32_at(4)), (3, 2));
        assert_eq!((f32_at(8), f32_at(12)), (1.5, 7.0));
        assert_eq!(u32_at(16), 42);
        assert_eq!((u32_at(20), u32_at(24)), (4096, 2048));
        assert_eq!(u32_at(28), FFT_WINDOWED_FLAG | FFT_DOWNSAMPLED_FLAG);
        assert_eq!((u32_at(32), u32_at(36)), (20, 1025));
        let plain = fft_header(&FftResult { downsampled: false, ..fft }, 1.5, 7.0, 42);
        assert_eq!(u32::from_le_bytes(plain[28..32].try_into().unwrap()), FFT_WINDOWED_FLAG);
    }

    #[tokio::test]
    async fn a_histogram_window_bins_only_the_pixels_inside_it_and_keeps_the_full_statistics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.fits").to_str().unwrap().to_string();
        let mut data = ndarray::Array2::from_shape_vec((64, 64), noise(64 * 64, 3.0, 21)).unwrap();
        for (y, x) in [(3usize, 5usize), (40, 41), (50, 12), (60, 60)] {
            data[[y, x]] = 1000.0;
        }
        crate::infra::fits::writer::write_fits_mono(&path, &data, None).unwrap();

        let full = compute_histogram(path.clone(), None, None, None).await.unwrap();
        let windowed = compute_histogram(path, None, Some(-15.0), Some(15.0)).await.unwrap();
        assert_eq!(windowed[RES_MIN], -15.0);
        assert_eq!(windowed[RES_MAX], 15.0);
        assert_eq!(full[RES_MAX], 1000.0);
        for key in [RES_DATA_MIN, RES_DATA_MAX, RES_MEDIAN, RES_SIGMA, RES_MAD, RES_TOTAL_PIXELS, RES_AUTO_STF] {
            assert_eq!(windowed[key], full[key], "{key}");
        }
        let count = |out: &serde_json::Value| out[RES_BINS].as_array().unwrap().iter().map(|b| b.as_u64().unwrap()).sum::<u64>();
        assert_eq!(count(&full), 64 * 64);
        assert_eq!(count(&windowed), 64 * 64 - 4, "pixels outside the window were piled into the edge bins");
        assert_eq!(windowed[RES_BIN_COUNT], HISTOGRAM_BINS_DISPLAY);
    }

    #[test]
    fn a_windowed_histogram_bins_in_place_exactly_what_a_filtered_copy_would_bin() {
        let mut values = noise(64 * 64, 3.0, 7);
        values[0] = f32::NAN;
        values[1] = 0.0;
        values[2] = f32::INFINITY;
        values[3] = -15.0;
        values[4] = 15.0;
        values[5] = 15.000001;
        values[6] = -400.0;
        values[7] = 400.0;
        let arr = ndarray::Array2::from_shape_vec((64, 64), values.clone()).unwrap();
        let hist = windowed_histogram(&arr, -15.0, 15.0);
        let copied: Vec<f32> = values.iter().copied().filter(|&v| (-15.0..=15.0).contains(&v)).collect();
        let expected = build_histogram(&copied, HISTOGRAM_BINS, -15.0, 15.0);
        assert_eq!(hist.bins, expected.bins);
        assert_eq!(hist.bin_edges, expected.bin_edges);
        assert_eq!((hist.min, hist.max), (-15.0, 15.0));
        let inside = values.iter().filter(|&&v| v.is_finite() && v != 0.0 && (-15.0..=15.0).contains(&v)).count();
        assert_eq!(hist.bins.iter().map(|&b| b as usize).sum::<usize>(), inside);
        assert!(hist.bins[0] >= 1, "the lower edge belongs to the first bin");
        assert!(hist.bins[hist.bins.len() - 1] >= 1, "the upper edge belongs to the last bin");
        let transposed = arr.reversed_axes();
        assert!(transposed.as_slice().is_none());
        assert_eq!(windowed_histogram(&transposed, -15.0, 15.0).bins, hist.bins);
        let degenerate = windowed_histogram(&transposed, 1.0, 1.0 + 1e-12);
        assert!(degenerate.bins.iter().all(|&b| b == 0));
        assert_eq!(degenerate.bins.len(), HISTOGRAM_BINS);
    }

    #[tokio::test]
    async fn a_half_specified_inverted_or_non_finite_histogram_window_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "window_refused.fits");
        let half = compute_histogram(path.clone(), None, Some(1.0), None).await.unwrap_err();
        assert!(half.contains("histogram window needs both lo and hi"), "{half}");
        let inverted = compute_histogram(path.clone(), None, Some(5.0), Some(5.0)).await.unwrap_err();
        assert!(inverted.contains("histogram window hi 5 must be larger than lo 5"), "{inverted}");
        let nan = compute_histogram(path, None, Some(f64::NAN), Some(1.0)).await.unwrap_err();
        assert!(nan.contains("histogram window (NaN, 1) must be finite"), "{nan}");
    }

    #[tokio::test]
    async fn a_histogram_window_narrower_than_the_minimum_is_refused_before_binning() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "window_narrow.fits");
        let narrow = compute_histogram(path, None, Some(2.0), Some(2.0 + 1e-11)).await.unwrap_err();
        assert!(narrow.contains("histogram window from lo 2 to hi 2.00000000001 is narrower than"), "{narrow}");
    }

    fn gaia_row(ra: f64, dec: f64, g: f64) -> CatalogRow {
        CatalogRow {
            id: format!("{ra}_{dec}"),
            ra,
            dec,
            ra_epoch: ra,
            dec_epoch: dec,
            pm_ra_masyr: None,
            pm_dec_masyr: None,
            g: Some(g),
            bp: None,
            rp: None,
            bp_rp: Some(0.8),
            parallax_mas: None,
            teff_k: None,
        }
    }

    #[test]
    fn a_gaia_failure_or_an_empty_cone_becomes_a_warning_and_the_nearest_star_within_five_arcsec_matches() {
        let (value, warning) = gaia_match_outcome(10.0, 20.0, &[]);
        assert!(value.is_null());
        assert_eq!(warning.as_deref(), Some("Gaia: no G<17 star within 5\""));
        let far = gaia_row(10.0, 20.0 + 10.0 / 3600.0, 11.0);
        let (value, warning) = gaia_match_outcome(10.0, 20.0, &[&far]);
        assert!(value.is_null());
        assert_eq!(warning.as_deref(), Some("Gaia: no G<17 star within 5\""));
        let near = gaia_row(10.0 + 3.0 / 3600.0 / 20.0f64.to_radians().cos(), 20.0, 12.5);
        let nearer = gaia_row(10.0, 20.0 - 2.0 / 3600.0, 13.0);
        let (value, warning) = gaia_match_outcome(10.0, 20.0, &[&far, &near, &nearer]);
        assert!(warning.is_none(), "{warning:?}");
        assert_eq!(value[RES_GMAG], 13.0);
        assert_eq!(value[RES_BP_RP], 0.8);
        assert!((value[RES_SEPARATION_ARCSEC].as_f64().unwrap() - 2.0).abs() < 1e-6, "{value}");
        let across_zero = gaia_row(0.0005, 0.0, 9.0);
        let (wrapped, _) = gaia_match_outcome(359.9995, 0.0, &[&across_zero]);
        assert!((wrapped[RES_SEPARATION_ARCSEC].as_f64().unwrap() - 3.6).abs() < 1e-6, "{wrapped}");
    }

    fn catalog_from_map(
        primed: &HashMap<CatalogCacheKey, Vec<CatalogRow>>,
        seen: &mut Vec<ConeQuery>,
        q: &ConeQuery,
    ) -> Result<CatalogHit, String> {
        seen.push(q.clone());
        let rows = primed
            .get(&catalog_cache_key(q))
            .unwrap_or_else(|| panic!("unexpected catalog query {q:?}"));
        Ok(CatalogHit { rows: Arc::new(rows.clone()), source: CatalogSource::Memory })
    }

    #[test]
    fn gaia_match_uses_the_cached_field_cone() {
        let dir = tempfile::tempdir().unwrap();
        let path = wcs_gaussian_fits_at(dir.path(), "field_cone.fits", 212.5, -17.25);
        let header = cached_header(&path).unwrap();
        let wcs = WcsTransform::from_header(&header).unwrap();
        let geometry = field_geometry(&wcs, 64, 64);
        let field = gaia_match_field(&geometry);
        let half_diagonal = cone_query_for_field(&geometry, None, Some(17.0), None);
        assert!(
            ((field.radius_deg - half_diagonal.radius_deg) * 3600.0 - GAIA_MATCH_RADIUS_ARCSEC).abs() < 1e-9,
            "{} vs {}",
            field.radius_deg,
            half_diagonal.radius_deg
        );
        assert_eq!((field.ra, field.dec, field.mag_limit, field.max_rows), (half_diagonal.ra, half_diagonal.dec, Some(17.0), 5000));
        let star = wcs.pixel_to_world(32.0, 32.0);
        let mut primed = HashMap::new();
        primed.insert(catalog_cache_key(&field), vec![gaia_row(star.ra, star.dec, 12.0)]);
        let mut seen = Vec::new();
        let mut cached = |q: &ConeQuery| catalog_from_map(&primed, &mut seen, q);
        let out = photometry_for_path_with_catalog(&path, 32.0, 32.0, None, None, true, false, None, None, &mut cached).unwrap();
        assert_eq!(out[RES_GAIA][RES_BP_RP], 0.8, "{}", out[RES_GAIA]);
        assert_eq!(out[RES_GAIA][RES_GMAG], 12.0);
        assert!(out[RES_GAIA][RES_SEPARATION_ARCSEC].as_f64().unwrap() < 1.0, "{}", out[RES_GAIA]);
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(!warnings.iter().any(|w| w.as_str().unwrap().starts_with("Gaia")), "{warnings:?}");
        assert_eq!(seen, vec![field]);
    }

    #[test]
    fn a_capped_field_cone_falls_back_to_the_per_star_cone() {
        let dir = tempfile::tempdir().unwrap();
        let path = wcs_gaussian_fits_at(dir.path(), "capped_cone.fits", 213.0, -17.75);
        let header = cached_header(&path).unwrap();
        let wcs = WcsTransform::from_header(&header).unwrap();
        let field = gaia_match_field(&field_geometry(&wcs, 64, 64));
        let star = wcs.pixel_to_world(32.0, 32.0);
        let per_star = ConeQuery { ra: star.ra, dec: star.dec, radius_deg: 0.01, mag_limit: Some(17.0), max_rows: 500 };
        let far = gaia_row(star.ra + 0.5, star.dec, 9.0);
        let mut primed = HashMap::new();
        primed.insert(catalog_cache_key(&field), vec![far; field.max_rows]);
        primed.insert(catalog_cache_key(&per_star), vec![gaia_row(star.ra, star.dec, 12.0)]);
        let mut seen = Vec::new();
        let mut cached = |q: &ConeQuery| catalog_from_map(&primed, &mut seen, q);
        let out = photometry_for_path_with_catalog(&path, 32.0, 32.0, None, None, true, false, None, None, &mut cached).unwrap();
        assert_eq!(out[RES_GAIA][RES_BP_RP], 0.8, "{}", out[RES_GAIA]);
        assert_eq!(out[RES_GAIA][RES_GMAG], 12.0);
        assert_eq!(field.max_rows, 5000);
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(seen[0], field);
        assert_eq!(seen[1].radius_deg, GAIA_MATCH_CONE_DEG);
        assert_eq!(seen[1].mag_limit, Some(GAIA_MATCH_MAG_LIMIT));
        assert_eq!(seen[1].max_rows, GAIA_MATCH_FALLBACK_ROWS);
    }

    #[test]
    fn a_gaia_query_failure_becomes_a_warning_at_the_caller() {
        let dir = tempfile::tempdir().unwrap();
        let path = wcs_gaussian_fits_at(dir.path(), "offline.fits", 213.5, -18.25);
        let mut cached = |_: &ConeQuery| Err("VizieR request failed: timeout".to_string());
        let out = photometry_for_path_with_catalog(&path, 32.0, 32.0, None, None, true, false, None, None, &mut cached).unwrap();
        assert!(out[RES_GAIA].is_null(), "{}", out[RES_GAIA]);
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap() == "Gaia query failed: VizieR request failed: timeout"),
            "{warnings:?}"
        );
    }

    fn wcs_gaussian_fits(dir: &std::path::Path, name: &str) -> String {
        wcs_gaussian_fits_at(dir, name, 180.0, 45.0)
    }

    fn wcs_gaussian_fits_at(dir: &std::path::Path, name: &str, crval1: f64, crval2: f64) -> String {
        let size = 64;
        let mut arr = ndarray::Array2::<f32>::zeros((size, size));
        for (i, v) in gaussian_pixels(size, 1000.0, 2.0, 100.0).into_iter().enumerate() {
            arr[[i / size, i % size]] = v;
        }
        let mut header = crate::types::header::HduHeader::empty();
        header.set("CTYPE1", "RA---TAN".to_string());
        header.set("CTYPE2", "DEC--TAN".to_string());
        header.set_f64("CRPIX1", 32.5);
        header.set_f64("CRPIX2", 32.5);
        header.set_f64("CRVAL1", crval1);
        header.set_f64("CRVAL2", crval2);
        header.set_f64("CD1_1", -2.7778e-4);
        header.set_f64("CD1_2", 0.0);
        header.set_f64("CD2_1", 0.0);
        header.set_f64("CD2_2", 2.7778e-4);
        let path = dir.join(name);
        crate::infra::fits::writer::write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn click_photometry_skips_the_gaia_match_unless_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = wcs_gaussian_fits(dir.path(), "wcs.fits");
        let out = measure_photometry_cmd(path, 32.0, 32.0, None, None, None, None, None, None, None).await.unwrap();
        assert!(out[RES_SKY][RES_RA].is_number(), "{}", out[RES_SKY]);
        assert!(out[RES_GAIA].is_null());
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(!warnings.iter().any(|w| w.as_str().unwrap().starts_with("Gaia")), "{warnings:?}");
    }

    #[test]
    fn a_gaia_match_without_a_wcs_is_reported_as_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "nowcs.fits");
        let out = photometry_for_path(&path, 32.0, 32.0, None, None, true, false, None).unwrap();
        assert!(out[RES_GAIA].is_null());
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap() == "Gaia match skipped: the file has no celestial WCS"),
            "{warnings:?}"
        );
    }

    fn cal_photometry_with_gain(key: &str, exclude_dq: bool, gain: Option<f64>) -> serde_json::Value {
        photometry_for_path(key, 32.0, 32.0, Some(CAL_APERTURE_RADIUS), None, false, exclude_dq, gain).unwrap()
    }

    fn cal_photometry(key: &str, exclude_dq: bool) -> serde_json::Value {
        cal_photometry_with_gain(key, exclude_dq, None)
    }

    fn plain_cal(dir: &std::path::Path, name: &str, planes: CalPlanes, exclude_dq: bool, gain: Option<f64>) -> serde_json::Value {
        let plain = jwst_cal_mef(&dir.join(name), &[], cal_sci_cards(), planes, vec![]);
        cal_photometry_with_gain(&plain.key, exclude_dq, gain)
    }

    const FLUX_FIELDS: [&str; 8] =
        ["net_flux", "flux_err", "snr", "flux_jy", "flux_err_jy", "mag_ab", "mag_ab_err", "mag_ab_total"];

    fn flux_bits(phot: &serde_json::Value) -> Vec<u64> {
        let mut bits: Vec<u64> = FLUX_FIELDS.iter().map(|f| phot[f].as_f64().unwrap().to_bits()).collect();
        bits.extend(phot["growth_curve"].as_array().unwrap().iter().map(|p| p["flux"].as_f64().unwrap().to_bits()));
        bits
    }

    const HEAD_15AFA22_FLUX_BITS: [u64; 48] = [
        0x40d75c70674d66e1, 0x400fc29c5ad9e12b, 0x40b78997fc2dd42a, 0x3f41f85858818451,
        0x3e786e5372411c0e, 0x40310d602c12372e, 0x3f279e162f820bf8, 0x4030ffbd755aac0d,
        0x408a393148ebcb56, 0x40a556062cd496c9, 0x40b805056b3f574c, 0x40c2c2bd8a6c847b,
        0x40ca5f6f7f358ad2, 0x40d04d371bace26e, 0x40d33f33a038ba1b, 0x40d506b4dd20abd0,
        0x40d6883460052aa6, 0x40d75c70674d66e1, 0x40d7f39cca6557ae, 0x40d83ca7d26b0ca2,
        0x40d8684e461bcf10, 0x40d87be8f02a60aa, 0x40d88429e75192a3, 0x40d8878eea6c1ba0,
        0x40d88924aa49fcac, 0x40d8898ee2ac7720, 0x40d88a8720604db2, 0x40d88ad549dff961,
        0x40d88accb6fc3754, 0x40d88a25e40dcede, 0x40d888dbc6d4d012, 0x40d8891ff82b1b3c,
        0x40d8895afa6c920e, 0x40d8896100069a3e, 0x40d88a559d158afe, 0x40d88a4a24dbd3e4,
        0x40d888dbb2e5f9ac, 0x40d8883f5b43948c, 0x40d887f3a33a88a4, 0x40d8882b0f7e0564,
        0x40d888a5603f7c50, 0x40d888c1277c1cb4, 0x40d889229160b098, 0x40d888110c29d574,
        0x40d886b8036f7328, 0x40d886d1e9ced978, 0x40d886f28331d198, 0x40d886ed3199d5c4,
    ];
    const HEAD_LABEL: &str = "JWST MJy/sr, PIXAR_SR from header";
    const AREA_LABEL: &str = "JWST MJy/sr, PIXAR_SR from header, AREA pixel-area map applied";
    const JWST_MJY_SR_NOTE: &str = "JWST MJy/sr: flux [Jy] = sum(MJy/sr) * PIXAR_SR * 1e6";
    const AREA_NOTE: &str =
        "AREA pixel-area map applied (HDU 4, map mean 1.0001): flux [Jy] = sum((SB - sky) * AREA) * PIXAR_SR * 1e6";

    fn native_bits(phot: &serde_json::Value) -> Vec<u64> {
        let mut bits = vec![phot["net_flux"].as_f64().unwrap().to_bits(), phot["flux_err"].as_f64().unwrap().to_bits()];
        bits.extend(phot["growth_curve"].as_array().unwrap().iter().map(|p| p["flux"].as_f64().unwrap().to_bits()));
        bits
    }

    fn relative_gap(value: f64, expected: f64) -> f64 {
        (value - expected).abs() / expected.abs()
    }

    fn number(phot: &serde_json::Value, field: &str) -> f64 {
        phot[field].as_f64().unwrap_or_else(|| panic!("{field}: {phot}"))
    }

    fn mentions_area(values: &serde_json::Value) -> bool {
        values.as_array().map_or(false, |items| items.iter().any(|v| v.as_str().unwrap_or("").contains("AREA")))
    }

    fn area_warnings(out: &serde_json::Value) -> Vec<String> {
        out[RES_WARNINGS]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap().to_string())
            .filter(|w| w.contains("AREA"))
            .collect()
    }

    fn notes_of(out: &serde_json::Value) -> Vec<String> {
        out[RES_PHOTCAL]["notes"].as_array().map_or(Vec::new(), |n| n.iter().map(|v| v.as_str().unwrap().to_string()).collect())
    }

    fn tan_wcs_cards(arcsec_per_pixel: f64) -> Vec<(&'static str, String)> {
        let deg = arcsec_per_pixel / 3600.0;
        vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", "32.5".into()),
            ("CRPIX2", "32.5".into()),
            ("CRVAL1", "150.0".into()),
            ("CRVAL2", "2.0".into()),
            ("CD1_1", format!("{:.12E}", -deg)),
            ("CD1_2", "0.0".into()),
            ("CD2_1", "0.0".into()),
            ("CD2_2", format!("{:.12E}", deg)),
        ]
    }

    fn sci_cards_with(cards: Vec<(&'static str, String)>) -> Vec<(&'static str, String)> {
        let mut all = cal_sci_cards();
        all.extend(cards);
        all
    }

    fn sci_cards_without(key: &str) -> Vec<(&'static str, String)> {
        cal_sci_cards().into_iter().filter(|(k, _)| *k != key).collect()
    }

    fn sci_cards_with_pixar(value: &str) -> Vec<(&'static str, String)> {
        let mut cards = sci_cards_without("PIXAR_SR");
        cards.push(("PIXAR_SR", value.to_string()));
        cards
    }

    #[test]
    fn a_jwst_cal_without_an_area_extension_keeps_the_head_photometry_bit_for_bit() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(&dir.path().join("cal.fits"), &[], cal_sci_cards(), CalPlanes::ALL, vec![]);
        let out = cal_photometry(&fixture.key, false);
        let phot = &out[RES_PHOTOMETRY];
        assert_eq!(flux_bits(phot), HEAD_15AFA22_FLUX_BITS.to_vec());
        assert_eq!(out[RES_PHOTCAL][RES_LABEL], HEAD_LABEL);
        assert!(notes_of(&out).iter().any(|n| n == JWST_MJY_SR_NOTE), "{:?}", notes_of(&out));
        assert!(!mentions_area(&out[RES_PHOTCAL]["notes"]), "{}", out[RES_PHOTCAL]["notes"]);
        assert!(!mentions_area(&out[RES_WARNINGS]), "{}", out[RES_WARNINGS]);
        assert_eq!(phot["area_applied"], false);
    }

    #[test]
    fn an_i2d_with_an_r_area_card_but_no_area_extension_matches_the_plain_cal_bit_for_bit() {
        let dir = tempfile::tempdir().unwrap();
        let size = CAL_SIZE;
        let primary = [
            ("S_RESAMP", "'COMPLETE'".to_string()),
            ("R_AREA", "'crds://jwst_nircam_area_0312.fits'".to_string()),
        ];
        let resampled_products = vec![
            TestHdu {
                extname: Some("CON"),
                extver: Some(1),
                cols: size,
                rows: size,
                data: HduData::I32(vec![1; size * size]),
                extra_cards: vec![],
            },
            TestHdu {
                extname: Some("WHT"),
                extver: Some(1),
                cols: size,
                rows: size,
                data: HduData::F32(vec![1.0; size * size]),
                extra_cards: vec![],
            },
        ];
        let fixture = jwst_cal_mef(&dir.path().join("i2d.fits"), &primary, cal_sci_cards(), CalPlanes::SCI_ERR, resampled_products);
        let out = cal_photometry(&fixture.key, false);
        let reference = plain_cal(dir.path(), "plain.fits", CalPlanes::SCI_ERR, false, None);
        let phot = &out[RES_PHOTOMETRY];
        assert_eq!(flux_bits(phot), flux_bits(&reference[RES_PHOTOMETRY]));
        assert_eq!(out[RES_PHOTCAL][RES_LABEL], HEAD_LABEL);
        assert!(!mentions_area(&out[RES_PHOTCAL]["notes"]), "{}", out[RES_PHOTCAL]["notes"]);
        assert!(!mentions_area(&out[RES_WARNINGS]), "{}", out[RES_WARNINGS]);
        assert_eq!(phot["area_applied"], false);
    }

    #[test]
    fn area_map_fluxes_match_a_hand_sum_over_the_masked_aperture() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(
            &dir.path().join("cal_area.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::ALL,
            vec![area_hdu(blocked_area(), CAL_SIZE, Some(1))],
        );
        let area = square(&blocked_area());
        let out = cal_photometry(&fixture.key, true);
        let phot = &out[RES_PHOTOMETRY];
        let mask = dq_exclusion(&fixture.key).unwrap().expect("DQ exclusion map");
        assert_ne!(mask[[CAL_EXCLUDED_PIXEL.0, CAL_EXCLUDED_PIXEL.1]], 0);
        assert_eq!(phot["n_masked"], 1);

        let (x, y, r) = (number(phot, "x"), number(phot, "y"), number(phot, "aperture_radius"));
        let bg_mean = number(phot, "bg_mean");
        let bg_sigma = number(phot, "bg_sigma");
        let bg_pixels = number(phot, "bg_pixels");
        assert!(bg_sigma > 0.0);
        let mut net = 0.0f64;
        let mut variance = 0.0f64;
        let mut area_weight = 0.0f64;
        let mut touched_block = false;
        for p in circular_aperture(CAL_SIZE, CAL_SIZE, x, y, r, 5) {
            if mask[[p.y, p.x]] != 0 {
                continue;
            }
            let a = area[[p.y, p.x]] as f64;
            touched_block |= a != 1.0;
            let w = p.weight as f64 * a;
            net += w * (fixture.sci[[p.y, p.x]] as f64 - bg_mean);
            variance += (w * fixture.err[[p.y, p.x]] as f64).powi(2);
            area_weight += w;
        }
        assert!(touched_block, "the 1.02 block must overlap the aperture");
        variance += area_weight * area_weight * bg_sigma * bg_sigma / bg_pixels;
        let expected_jy = JY_PER_MJY * CAL_PIXAR_SR * net;
        let expected_err_jy = JY_PER_MJY * CAL_PIXAR_SR * variance.sqrt();

        let flux_jy = number(phot, "flux_jy");
        assert!(relative_gap(flux_jy, expected_jy) < 1e-9, "flux_jy {flux_jy} vs hand sum {expected_jy}");
        let flux_err_jy = number(phot, "flux_err_jy");
        assert!(relative_gap(flux_err_jy, expected_err_jy) < 1e-9, "flux_err_jy {flux_err_jy} vs hand sum {expected_err_jy}");
        let mag_ab = number(phot, "mag_ab");
        assert!((mag_ab - (-2.5 * flux_jy.log10() + AB_MAG_ZERO_POINT)).abs() < 1e-12, "mag_ab {mag_ab}");
        assert!(relative_gap(number(phot, "net_flux"), net) < 1e-9);
        assert_eq!(phot["area_applied"], true);
        assert_eq!(out[RES_PHOTCAL][RES_LABEL], AREA_LABEL);
        let notes = notes_of(&out);
        assert!(notes.iter().any(|n| n == AREA_NOTE), "{notes:?}");
        assert!(!notes.iter().any(|n| n == JWST_MJY_SR_NOTE), "the generic MJy/sr formula must be replaced: {notes:?}");
        assert!(!mentions_area(&out[RES_WARNINGS]), "{}", out[RES_WARNINGS]);
    }

    #[test]
    fn an_area_map_of_ones_is_bit_identical_and_a_uniform_map_scales_the_flux_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let reference = plain_cal(dir.path(), "cal_plain.fits", CalPlanes::ALL, false, None);
        let ones = jwst_cal_mef(
            &dir.path().join("cal_ones.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::ALL,
            vec![area_hdu(uniform_area(1.0), CAL_SIZE, Some(1))],
        );
        let out = cal_photometry(&ones.key, false);
        assert_eq!(flux_bits(&out[RES_PHOTOMETRY]), flux_bits(&reference[RES_PHOTOMETRY]));
        assert_eq!(out[RES_PHOTOMETRY]["area_applied"], true);
        assert_eq!(out[RES_PHOTCAL][RES_LABEL], AREA_LABEL);

        let scaled = jwst_cal_mef(
            &dir.path().join("cal_102.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::ALL,
            vec![area_hdu(uniform_area(AREA_BLOCK_VALUE), CAL_SIZE, Some(1))],
        );
        let out = cal_photometry(&scaled.path, false);
        let phot = &out[RES_PHOTOMETRY];
        let plain = &reference[RES_PHOTOMETRY];
        assert_eq!(out[RES_PHOTCAL][RES_LABEL], AREA_LABEL, "an Auto plane resolved to SCI takes the AREA map");
        let scale = f64::from(AREA_BLOCK_VALUE);
        for field in ["net_flux", "flux_err", "flux_jy", "flux_err_jy"] {
            let value = number(phot, field);
            let expected = scale * number(plain, field);
            assert!(relative_gap(value, expected) < 1e-12, "{field} {value} vs {expected}");
        }
        let mag_ab = number(phot, "mag_ab");
        assert!((mag_ab - (number(plain, "mag_ab") - 2.5 * scale.log10())).abs() < 1e-12, "mag_ab {mag_ab}");
        assert!(relative_gap(number(phot, "snr"), number(plain, "snr")) < 1e-12);
        for field in ["x", "y", "peak", "fwhm", "aperture_radius", "aperture_pixels", "aperture_area", "bg_mean", "bg_sigma", "bg_pixels", "n_masked", "sky_inner", "sky_outer"] {
            assert_eq!(phot[field], plain[field], "{field}");
        }
        assert!(relative_gap(number(phot, "aperture_correction"), number(plain, "aperture_correction")) < 1e-12);
    }

    #[test]
    fn a_non_finite_or_non_positive_area_pixel_gets_unit_weight() {
        let dir = tempfile::tempdir().unwrap();
        let reference = plain_cal(dir.path(), "cal_plain.fits", CalPlanes::ALL, true, None);
        let mut area = uniform_area(1.0);
        area[32 * CAL_SIZE + 33] = f32::NAN;
        area[31 * CAL_SIZE + 31] = 0.0;
        area[33 * CAL_SIZE + 30] = -1.0;
        let holed = jwst_cal_mef(&dir.path().join("cal_holes.fits"), &[], cal_sci_cards(), CalPlanes::ALL, vec![area_hdu(area, CAL_SIZE, Some(1))]);
        let out = cal_photometry(&holed.key, true);
        assert_eq!(flux_bits(&out[RES_PHOTOMETRY]), flux_bits(&reference[RES_PHOTOMETRY]));
        assert_eq!(out[RES_PHOTOMETRY]["area_applied"], true);
        assert_eq!(out[RES_PHOTCAL][RES_LABEL], AREA_LABEL);
        assert!(area_warnings(&out).is_empty(), "{:?}", area_warnings(&out));
    }

    #[test]
    fn without_an_err_plane_the_area_map_enters_the_variance_once_like_photon_counts() {
        let dir = tempfile::tempdir().unwrap();
        let scale = f64::from(AREA_BLOCK_VALUE);
        for (name, gain) in [("nogain", None), ("gain", Some(2.0))] {
            let reference = plain_cal(dir.path(), &format!("plain_{name}.fits"), CalPlanes::SCI_DQ, false, gain);
            let scaled = jwst_cal_mef(
                &dir.path().join(format!("uniform_{name}.fits")),
                &[],
                cal_sci_cards(),
                CalPlanes::SCI_DQ,
                vec![area_hdu(uniform_area(AREA_BLOCK_VALUE), CAL_SIZE, Some(1))],
            );
            let out = cal_photometry_with_gain(&scaled.key, false, gain);
            let (phot, plain) = (&out[RES_PHOTOMETRY], &reference[RES_PHOTOMETRY]);
            assert_eq!(phot["err_used"], false);
            assert_eq!(phot["area_applied"], true);
            for field in ["aperture_area", "bg_mean", "bg_sigma", "bg_pixels"] {
                assert_eq!(phot[field], plain[field], "{name}: {field}");
            }
            let (n_ap, sigma, n_bg) = (number(plain, "aperture_area"), number(plain, "bg_sigma"), number(plain, "bg_pixels"));
            let poisson = gain.map_or(0.0, |g| number(plain, "net_flux").max(0.0) / g);
            let expected = (scale * (poisson + n_ap * sigma * sigma) + scale * scale * n_ap * n_ap * sigma * sigma / n_bg).sqrt();
            let flux_err = number(phot, "flux_err");
            assert!(
                relative_gap(flux_err, expected) < 1e-9,
                "{name}: flux_err {flux_err} vs {expected} (a uniform map of {scale} multiplies the photon and sky variance by {scale}, the sky-mean term by {scale}^2)"
            );
            let (snr, plain_snr) = (number(phot, "snr"), number(plain, "snr"));
            assert!(relative_gap(snr, number(phot, "net_flux") / flux_err) < 1e-12, "{name}: snr {snr}");
            assert!(snr > plain_snr && snr < scale * plain_snr, "{name}: more collecting area gives more signal to noise, less than linearly: {snr} vs {plain_snr}");
        }

        let gain = 2.0;
        let blocked = jwst_cal_mef(
            &dir.path().join("blocked_noerr.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::SCI_DQ,
            vec![area_hdu(blocked_area(), CAL_SIZE, Some(1))],
        );
        let area = square(&blocked_area());
        let out = cal_photometry_with_gain(&blocked.key, true, Some(gain));
        let phot = &out[RES_PHOTOMETRY];
        let mask = dq_exclusion(&blocked.key).unwrap().expect("DQ exclusion map");
        let (x, y, r) = (number(phot, "x"), number(phot, "y"), number(phot, "aperture_radius"));
        let (bg_mean, bg_sigma, bg_pixels) = (number(phot, "bg_mean"), number(phot, "bg_sigma"), number(phot, "bg_pixels"));
        let mut net = 0.0f64;
        let mut area_weight = 0.0f64;
        for p in circular_aperture(CAL_SIZE, CAL_SIZE, x, y, r, 5) {
            if mask[[p.y, p.x]] != 0 {
                continue;
            }
            let (w, a) = (p.weight as f64, area[[p.y, p.x]] as f64);
            let signal = blocked.sci[[p.y, p.x]] as f64 - bg_mean;
            net += w * a * signal;
            area_weight += w * a;
        }
        let variance = net.max(0.0) / gain + area_weight * bg_sigma * bg_sigma + area_weight * area_weight * bg_sigma * bg_sigma / bg_pixels;
        let flux_err = number(phot, "flux_err");
        assert!(relative_gap(number(phot, "net_flux"), net) < 1e-9);
        assert!(relative_gap(flux_err, variance.sqrt()) < 1e-9, "flux_err {flux_err} vs hand sum {}", variance.sqrt());
        assert_eq!(phot["err_used"], false);
    }

    #[test]
    fn an_area_extension_that_fails_a_gate_is_ignored_with_a_named_warning() {
        let dir = tempfile::tempdir().unwrap();
        let reference = plain_cal(dir.path(), "plain.fits", CalPlanes::ALL, false, None);
        let plain_native = native_bits(&reference[RES_PHOTOMETRY]);
        let map = |value: f32| vec![area_hdu(uniform_area(value), CAL_SIZE, Some(1))];
        let steradians = jwst_cal_mef(&dir.path().join("sr.fits"), &[], cal_sci_cards(), CalPlanes::ALL, map(2.29249e-14));
        let wrong_size = jwst_cal_mef(&dir.path().join("dims.fits"), &[], cal_sci_cards(), CalPlanes::ALL, vec![area_hdu(vec![1.0; 32 * 32], 32, Some(1))]);
        let other_extver = jwst_cal_mef(&dir.path().join("extver.fits"), &[], cal_sci_cards(), CalPlanes::ALL, vec![area_hdu(uniform_area(1.02), CAL_SIZE, Some(2))]);
        let resampled = jwst_cal_mef(&dir.path().join("resampled.fits"), &[], sci_cards_with(tan_wcs_cards(0.0625)), CalPlanes::ALL, map(1.02));
        let mut no_pixar_cards = sci_cards_without("PIXAR_SR");
        no_pixar_cards.extend(tan_wcs_cards(0.0312));
        let no_pixar = jwst_cal_mef(&dir.path().join("nopixar.fits"), &[], no_pixar_cards, CalPlanes::ALL, map(1.02));
        let mut zero_pixar_cards = sci_cards_with_pixar("0.0");
        zero_pixar_cards.extend(tan_wcs_cards(0.0312));
        let zero_pixar = jwst_cal_mef(&dir.path().join("zeropixar.fits"), &[], zero_pixar_cards, CalPlanes::ALL, map(1.02));
        let uncalibrated = jwst_cal_mef(&dir.path().join("uncal.fits"), &[], sci_cards_without("PIXAR_SR"), CalPlanes::ALL, map(1.02));
        let mut rate_cards = sci_cards_without("BUNIT");
        rate_cards.push(("BUNIT", "'DN/s'".into()));
        let rate = jwst_cal_mef(&dir.path().join("rate.fits"), &[], rate_cards, CalPlanes::ALL, map(1.02));
        let derived = "JWST MJy/sr, PIXAR_SR derived from the WCS";
        let cases = [
            (&steradians, "AREA extension ignored: map mean 2.29e-14 is not a relative pixel area (expected about 1)", Some(HEAD_LABEL)),
            (&wrong_size, "AREA extension ignored: its size 32 x 32 differs from the image 64 x 64", Some(HEAD_LABEL)),
            (&other_extver, "AREA extension ignored: EXTVER 2 differs from SCI EXTVER 1", Some(HEAD_LABEL)),
            (&resampled, "AREA extension ignored: PIXAR_SR was replaced by the WCS pixel area (resampled image)", Some(derived)),
            (&no_pixar, "AREA extension ignored: PIXAR_SR is missing, so the pixel area comes from the WCS", Some(derived)),
            (&zero_pixar, "AREA extension ignored: PIXAR_SR is missing, so the pixel area comes from the WCS", Some(derived)),
            (&uncalibrated, "AREA extension ignored: the header has no photometric calibration", None),
            (&rate, "AREA extension ignored: JWST DN/s via PHOTMJSR and PIXAR_SR is not JWST MJy/sr", Some("JWST DN/s via PHOTMJSR and PIXAR_SR")),
        ];
        for (fixture, expected_warning, label) in cases {
            let out = cal_photometry(&fixture.key, false);
            let phot = &out[RES_PHOTOMETRY];
            assert_eq!(native_bits(phot), plain_native, "{}", fixture.key);
            assert_eq!(phot["area_applied"], false, "{}", fixture.key);
            assert_eq!(out[RES_PHOTCAL][RES_LABEL].as_str(), label, "{}", fixture.key);
            assert!(!mentions_area(&out[RES_PHOTCAL]["notes"]), "{}: {}", fixture.key, out[RES_PHOTCAL]["notes"]);
            assert_eq!(area_warnings(&out), vec![expected_warning.to_string()], "{}", fixture.key);
        }
        let out = cal_photometry(&steradians.key, false);
        assert_eq!(flux_bits(&out[RES_PHOTOMETRY]), flux_bits(&reference[RES_PHOTOMETRY]));
    }

    #[test]
    fn an_area_hdu_without_an_extver_card_is_the_map_of_sci_extver_1() {
        let dir = tempfile::tempdir().unwrap();
        let reference = plain_cal(dir.path(), "plain.fits", CalPlanes::ALL, false, None);
        let fixture = jwst_cal_mef(
            &dir.path().join("noextver.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::ALL,
            vec![area_hdu(uniform_area(AREA_BLOCK_VALUE), CAL_SIZE, None)],
        );
        let out = cal_photometry(&fixture.key, false);
        assert_eq!(out[RES_PHOTOMETRY]["area_applied"], true);
        assert_eq!(out[RES_PHOTCAL][RES_LABEL], AREA_LABEL);
        assert!(area_warnings(&out).is_empty(), "{:?}", area_warnings(&out));
        let scale = f64::from(AREA_BLOCK_VALUE);
        let (flux, plain) = (number(&out[RES_PHOTOMETRY], "net_flux"), number(&reference[RES_PHOTOMETRY], "net_flux"));
        assert!(relative_gap(flux, scale * plain) < 1e-12, "{flux} vs {scale} x {plain}");
    }

    #[tokio::test]
    async fn batch_photometry_with_an_area_map_equals_the_single_measurement() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(
            &dir.path().join("batch_area.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::ALL,
            vec![area_hdu(blocked_area(), CAL_SIZE, Some(1))],
        );
        let single = measure_photometry_cmd(fixture.key.clone(), 32.0, 32.0, Some(CAL_APERTURE_RADIUS), None, None, Some(false), Some(true), None, None)
            .await
            .unwrap();
        let batch = measure_photometry_batch_cmd(fixture.key.clone(), vec![(32.0, 32.0)], Some(CAL_APERTURE_RADIUS), None, None, None, Some(true), Some(true), None)
            .await
            .unwrap();
        let rows = batch[RES_ROWS].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(single[RES_PHOTOMETRY]["area_applied"], true);
        assert_eq!(rows[0][RES_PHOTOMETRY], single[RES_PHOTOMETRY]);
        assert_eq!(batch[RES_PHOTCAL], single[RES_PHOTCAL]);
        assert_eq!(batch[RES_PHOTCAL][RES_LABEL], AREA_LABEL);
        assert_eq!(batch[RES_WARNINGS], single[RES_WARNINGS]);
    }

    #[test]
    fn a_time_series_applies_the_area_map_frame_by_frame() {
        let dir = tempfile::tempdir().unwrap();
        let with_area = jwst_cal_mef(
            &dir.path().join("frame_area.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::ALL,
            vec![area_hdu(uniform_area(AREA_BLOCK_VALUE), CAL_SIZE, Some(1))],
        );
        let plain = jwst_cal_mef(&dir.path().join("frame_plain.fits"), &[], cal_sci_cards(), CalPlanes::ALL, vec![]);
        let refused = jwst_cal_mef(
            &dir.path().join("frame_sr.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::ALL,
            vec![area_hdu(uniform_area(2.29249e-14), CAL_SIZE, Some(1))],
        );
        let single_plain = cal_photometry(&plain.key, false);
        let targets = vec![TimeSeriesTarget { x: 32.0, y: 32.0, label: "T".into(), role: "target".into() }];
        let config = PhotometryConfig { aperture_radius: Some(CAL_APERTURE_RADIUS), ..PhotometryConfig::default() };
        let out = measure_time_series(&[with_area.key.clone(), plain.key.clone(), refused.key.clone()], &targets, &config, false, false, None).unwrap();
        assert_eq!(out.n_skipped, 0, "{:?}", out.frames.iter().map(|f| f.skipped.clone()).collect::<Vec<_>>());
        let first = serde_json::to_value(out.frames[0].targets[0].as_ref().expect("target on the AREA frame")).unwrap();
        let second = serde_json::to_value(out.frames[1].targets[0].as_ref().expect("target on the plain frame")).unwrap();
        let third = serde_json::to_value(out.frames[2].targets[0].as_ref().expect("target on the refused frame")).unwrap();
        assert_eq!(first["area_applied"], true);
        assert_eq!(second["area_applied"], false);
        assert_eq!(third["area_applied"], false);
        for field in ["net_flux", "flux_err", "flux_jy", "mag_ab"] {
            assert_eq!(number(&second, field).to_bits(), number(&single_plain[RES_PHOTOMETRY], field).to_bits(), "{field}");
            assert_eq!(number(&third, field).to_bits(), number(&single_plain[RES_PHOTOMETRY], field).to_bits(), "{field}");
        }
        let (a, b) = (number(&first, "flux_jy"), number(&second, "flux_jy"));
        let scale = f64::from(AREA_BLOCK_VALUE);
        assert!(relative_gap(a, scale * b) < 1e-12, "{a} vs {scale} x {b}");
        assert_eq!(out.frames[0].photcal_label.as_deref(), Some(AREA_LABEL));
        assert_eq!(out.frames[1].photcal_label.as_deref(), Some(HEAD_LABEL));
        assert_eq!(out.frames[2].photcal_label.as_deref(), Some(HEAD_LABEL));
        let area_warnings: Vec<&String> = out.warnings.iter().filter(|w| w.contains("AREA")).collect();
        assert_eq!(
            area_warnings,
            vec![
                "frame_sr.fits: AREA extension ignored: map mean 2.29e-14 is not a relative pixel area (expected about 1)",
                "AREA pixel-area map applied on 1 of 3 measured frames (only on frame_area.fits): fluxes with and without the map differ by the aperture mean AREA, a step of a few hundredths of a magnitude in the light curve",
            ],
            "{:?}",
            out.warnings
        );
        let sent = serde_json::to_value(&out).unwrap();
        assert_eq!(sent["area_mixed"], true, "the exports need the flag, not only the warning text");

        let consistent = measure_time_series(&[plain.key.clone(), refused.key.clone()], &targets, &config, false, false, None).unwrap();
        assert!(
            !consistent.warnings.iter().any(|w| w.contains("of 2 measured frames")),
            "frames that agree on the map raise no series warning: {:?}",
            consistent.warnings
        );
        assert_eq!(serde_json::to_value(&consistent).unwrap()["area_mixed"], false);
    }

    #[test]
    fn a_cal_opened_from_a_zip_gets_the_same_area_map_and_err_plane_as_the_plain_file() {
        use crate::infra::fits::dispatcher::test_support::write_zip;
        let dir = tempfile::tempdir().unwrap();
        let plain = jwst_cal_mef(
            &dir.path().join("cal.fits"),
            &[],
            cal_sci_cards(),
            CalPlanes::ALL,
            vec![area_hdu(uniform_area(AREA_BLOCK_VALUE), CAL_SIZE, Some(1))],
        );
        let zip = dir.path().join("bundle.zip");
        write_zip(&zip, &[("cal.fits", &std::fs::read(&plain.path).unwrap())]);
        let zipped = format!("{}#hdu=1", zip.display());

        let from_zip = cal_photometry(&zipped, true);
        let from_plain = cal_photometry(&plain.key, true);
        assert_eq!(from_zip[RES_PHOTOMETRY]["area_applied"], true, "{}", from_zip[RES_WARNINGS]);
        assert_eq!(from_zip[RES_PHOTCAL][RES_LABEL], AREA_LABEL);
        assert_eq!(flux_bits(&from_zip[RES_PHOTOMETRY]), flux_bits(&from_plain[RES_PHOTOMETRY]));
        assert_eq!(from_zip[RES_WARNINGS], from_plain[RES_WARNINGS]);
        assert!(has_err_extension(&zipped, (CAL_SIZE, CAL_SIZE)), "the ERR plane of the zipped _cal feeds the gain model");
    }

    #[test]
    fn photometry_on_a_tan_pv_header_warns_that_the_pv_distortion_is_not_applied() {
        use crate::infra::wcs_source::test_fixtures::PV_DISTORTION_CARDS;
        let dir = tempfile::tempdir().unwrap();
        let mut cards = sci_cards_with(tan_wcs_cards(0.0312));
        cards.extend(PV_DISTORTION_CARDS.iter().map(|&(key, value)| (key, value.to_string())));
        let fixture = jwst_cal_mef(&dir.path().join("tanpv.fits"), &[], cards, CalPlanes::ALL, vec![]);
        let out = cal_photometry(&fixture.key, false);
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap() == "PV distortion not applied: PV1_0, PV1_1, PV1_2, PV1_4, PV2_0, PV2_2, PV2_10"),
            "{warnings:?}"
        );
        assert!(out[RES_SKY][RES_RA].is_number(), "the linear TAN still gives a sky position: {}", out[RES_SKY]);
    }

    #[test]
    #[ignore = "reads jw01475001001_02101_00001_nrca1_cal.fits (and the i2d and crf when present) from ASTROBURST_MAST_DIR"]
    fn real_jw01475_cal_flux_ratio_equals_the_flux_weighted_aperture_mean_area() {
        use crate::core::astrometry::gwcs::test_support::{mast_dir, MAST_DIR_VAR};
        let dir = mast_dir();
        let cal = dir.join("jw01475001001_02101_00001_nrca1_cal.fits");
        assert!(cal.is_file(), "set {MAST_DIR_VAR} to the folder that holds the jw01475 products: {} is not a file", cal.display());
        let ctx = photometry_context(&format!("{}#hdu=1", cal.display()), true).unwrap();
        let area = ctx.area.map().expect("AREA map attached to the _cal frame");
        assert_eq!(ctx.photcal.as_ref().unwrap().label(), AREA_LABEL);
        assert!(!ctx.warnings.iter().any(|w| w.contains("AREA")), "{:?}", ctx.warnings);
        let (h, w) = ctx.entry.arr().dim();
        let mut stars = detect_stars_core(ctx.entry.arr(), 20.0).stars;
        stars.sort_by(|a, b| b.flux.total_cmp(&a.flux));
        let config = ctx.config(Some(CAL_APERTURE_RADIUS), None, None);
        let mask = ctx.mask.as_ref().map(|m| &m.map);
        let bare_source = MaskedImage::new(ctx.entry.arr(), mask).unwrap();
        let with_area_source = ctx.masked_image().unwrap();
        let mut checked = 0;
        for star in stars.iter().filter(|s| s.x > 40.0 && s.y > 40.0 && s.x < (w - 40) as f64 && s.y < (h - 40) as f64) {
            let err = ctx.planes.err.as_ref().map(|e| e.arr());
            let Ok(bare) = measure_star_prepared(&bare_source, err, ctx.planes.saturated.as_ref(), star.x, star.y, &config) else {
                continue;
            };
            if bare.saturated || bare.net_flux <= 0.0 {
                continue;
            }
            let with = ctx.measure(&with_area_source, star.x, star.y, &config).unwrap();
            assert!(with.area_applied && !bare.area_applied);
            assert_eq!((with.x, with.y, with.bg_mean, with.aperture_area), (bare.x, bare.y, bare.bg_mean, bare.aperture_area));
            let mut numerator = 0.0f64;
            let mut denominator = 0.0f64;
            for p in circular_aperture(h, w, bare.x, bare.y, bare.aperture_radius, 5) {
                if mask.is_some_and(|m| m[[p.y, p.x]] != 0) {
                    continue;
                }
                let v = ctx.entry.arr()[[p.y, p.x]] as f64;
                if !v.is_finite() {
                    continue;
                }
                let weight = p.weight as f64 * (v - bare.bg_mean);
                numerator += weight * area[[p.y, p.x]] as f64;
                denominator += weight;
            }
            let expected = numerator / denominator;
            let ratio = with.net_flux / bare.net_flux;
            assert!((ratio - expected).abs() < 1e-6, "star ({:.1}, {:.1}): ratio {ratio} vs aperture mean AREA {expected}", bare.x, bare.y);
            eprintln!(
                "star ({:.1}, {:.1}): net {:.6e} -> {:.6e}, ratio {ratio:.6}, flux-weighted mean AREA {expected:.6}, delta mag_ab {:+.4}, mag_ab {:.4}",
                bare.x,
                bare.y,
                bare.net_flux,
                with.net_flux,
                -2.5 * ratio.log10(),
                with.mag_ab.unwrap_or(f64::NAN)
            );
            checked += 1;
            if checked == 5 {
                break;
            }
        }
        assert!(checked >= 3, "only {checked} stars could be measured");

        let i2d = dir.join("jw01475001001_02101_00001_nrca1_i2d.fits");
        if i2d.is_file() {
            let ctx = photometry_context(&format!("{}#hdu=1", i2d.display()), false).unwrap();
            assert!(ctx.area.map().is_none(), "an i2d has no AREA extension");
            assert!(!ctx.photcal.as_ref().unwrap().label().contains("AREA"));
            assert!(!ctx.warnings.iter().any(|w| w.contains("AREA")), "{:?}", ctx.warnings);
            eprintln!("i2d: {} ({:?})", ctx.photcal.as_ref().unwrap().label(), ctx.warnings);
        }
        let crf = dir.join("jw01475001001_02101_00001_nrca1_o001_crf.fits");
        if crf.is_file() {
            let ctx = photometry_context(&format!("{}#hdu=1", crf.display()), false).unwrap();
            assert!(ctx.area.map().is_some(), "the crf carries the same AREA extension as the cal");
            assert_eq!(ctx.photcal.as_ref().unwrap().label(), AREA_LABEL);
            eprintln!("crf: {}", ctx.photcal.as_ref().unwrap().notes.join(" | "));
        }
    }

    const USER_AB_25_JY_PER_UNIT: f64 = 3.63078054770101e-07;
    const USER_AB_25_PER_SECOND_600_JY_PER_COUNT: f64 = 6.051300912835017e-10;
    const TWO_AND_A_HALF_LOG10_600: f64 = 6.945378125959109;
    const PER_SECOND_REFUSAL_PREFIX: &str =
        "a zero point per second needs EXPTIME to turn the pixel values into a rate, and this header has none";

    fn counts_star(dir: &std::path::Path, name: &str, cards: &[(&str, &str)]) -> String {
        let size = 64;
        let arr = ndarray::Array2::from_shape_vec((size, size), gaussian_pixels(size, 1000.0, 2.0, 100.0)).unwrap();
        let mut header = crate::types::header::HduHeader::empty();
        for (key, value) in cards {
            header.set(key, value.to_string());
        }
        let path = dir.join(name);
        crate::infra::fits::writer::write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn user_zero_point(zp: f64, system: UserMagSystem, per_second: bool) -> UserZeroPoint {
        UserZeroPoint { zp, system, per_second }
    }

    fn measured_with(path: &str, zero_point: Option<&UserZeroPoint>) -> anyhow::Result<serde_json::Value> {
        photometry_for_path_with_catalog(path, 32.0, 32.0, Some(5.0), None, false, false, None, zero_point, &mut |q| {
            query_gaia_cached(q)
        })
    }

    fn warning_texts(out: &serde_json::Value) -> Vec<String> {
        out[RES_WARNINGS].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect()
    }

    fn assert_relative(measured: f64, expected: f64, tolerance: f64) {
        assert!(
            (measured - expected).abs() <= tolerance * expected.abs(),
            "measured {measured:e} expected {expected:e} within {tolerance:e} relative"
        );
    }

    #[test]
    fn a_typed_ab_zero_point_calibrates_an_image_without_calibration() {
        let dir = tempfile::tempdir().unwrap();
        let path = counts_star(dir.path(), "adu.fits", &[("BUNIT", "adu")]);
        let zp = user_zero_point(25.0, UserMagSystem::Ab, false);
        let out = measured_with(&path, Some(&zp)).unwrap();
        let phot = &out[RES_PHOTOMETRY];
        let net = phot["net_flux"].as_f64().unwrap();
        let flux_err = phot["flux_err"].as_f64().unwrap();
        assert_relative(phot["flux_jy"].as_f64().unwrap(), net * USER_AB_25_JY_PER_UNIT, 1e-12);
        assert!((phot["mag_ab"].as_f64().unwrap() - (25.0 - 2.5 * net.log10())).abs() <= 1e-9, "{phot}");
        assert_relative(phot["flux_err_jy"].as_f64().unwrap(), flux_err * USER_AB_25_JY_PER_UNIT, 1e-12);
        assert_eq!(out[RES_PHOTCAL]["mag_system"], "ab");
        let origin = out[RES_PHOTCAL]["origin"].as_str().unwrap();
        assert!(!origin.is_empty() && origin.contains("25"), "{origin}");
        let warnings = warning_texts(&out);
        assert_eq!(warnings.first().map(String::as_str), Some(USER_ZERO_POINT_WARNING), "{warnings:?}");
        assert_eq!(warnings.iter().filter(|w| w.as_str() == USER_ZERO_POINT_WARNING).count(), 1, "{warnings:?}");
        assert!(!warnings.iter().any(|w| w.contains("no photometric calibration")), "{warnings:?}");

        let uncalibrated = measured_with(&path, None).unwrap();
        assert!(uncalibrated[RES_PHOTOMETRY]["flux_jy"].is_null());
        assert!(
            warning_texts(&uncalibrated).iter().any(|w| w.contains("no photometric calibration")),
            "{:?}",
            warning_texts(&uncalibrated)
        );
    }

    #[test]
    fn a_per_second_zero_point_divides_counts_by_the_exposure_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = counts_star(dir.path(), "adu_600.fits", &[("BUNIT", "adu"), ("EXPTIME", "600")]);
        let out = measured_with(&path, Some(&user_zero_point(25.0, UserMagSystem::Ab, true))).unwrap();
        let phot = &out[RES_PHOTOMETRY];
        let net = phot["net_flux"].as_f64().unwrap();
        assert_relative(phot["flux_jy"].as_f64().unwrap(), net * USER_AB_25_PER_SECOND_600_JY_PER_COUNT, 1e-12);
        assert!(
            (phot["mag_ab"].as_f64().unwrap() - (25.0 + TWO_AND_A_HALF_LOG10_600 - 2.5 * net.log10())).abs() <= 1e-9,
            "{phot}"
        );
        let header = cached_header(&path).unwrap();
        let expected = PhotCal::user_zero_point_for(Some(&header), 25.0, MagSystem::Ab, true).warnings;
        assert!(expected
            .iter()
            .any(|w| w == "the user zero point is defined for one count per second: the pixel values are divided by EXPTIME = 600.0 s"));
        let warnings = warning_texts(&out);
        assert!(warnings.len() >= expected.len(), "{warnings:?}");
        assert_eq!(warnings[..expected.len()], expected[..], "{warnings:?}");
        assert_eq!(warnings.iter().filter(|w| w.contains("EXPTIME")).count(), 1, "{warnings:?}");
    }

    #[test]
    fn a_per_second_zero_point_on_a_rate_image_applies_no_exposure_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = counts_star(dir.path(), "adu_s.fits", &[("BUNIT", "adu/s"), ("EXPTIME", "600")]);
        let out = measured_with(&path, Some(&user_zero_point(25.0, UserMagSystem::Ab, true))).unwrap();
        let phot = &out[RES_PHOTOMETRY];
        let net = phot["net_flux"].as_f64().unwrap();
        assert_relative(phot["flux_jy"].as_f64().unwrap(), net * USER_AB_25_JY_PER_UNIT, 1e-12);
        let warnings = warning_texts(&out);
        assert!(!warnings.iter().any(|w| w.contains("EXPTIME")), "{warnings:?}");
    }

    #[test]
    fn a_per_second_zero_point_without_exptime_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let per_second = user_zero_point(25.0, UserMagSystem::Ab, true);
        let with_unit = counts_star(dir.path(), "adu_no_exptime.fits", &[("BUNIT", "adu")]);
        let error = measured_with(&with_unit, Some(&per_second)).unwrap_err();
        assert_eq!(format!("{error:#}"), format!("{PER_SECOND_REFUSAL_PREFIX} (BUNIT 'adu')"));
        let without_unit = counts_star(dir.path(), "no_unit.fits", &[]);
        let error = measured_with(&without_unit, Some(&per_second)).unwrap_err();
        assert_eq!(format!("{error:#}"), format!("{PER_SECOND_REFUSAL_PREFIX} (BUNIT 'missing')"));
        assert_eq!(
            format!("{:#}", user_photcal(None, &per_second).unwrap_err()),
            format!("{PER_SECOND_REFUSAL_PREFIX} (BUNIT 'missing')")
        );
    }

    #[test]
    fn a_vega_zero_point_gives_no_jansky_and_no_ab_magnitude() {
        let dir = tempfile::tempdir().unwrap();
        let path = counts_star(dir.path(), "adu_vega.fits", &[("BUNIT", "adu")]);
        let out = measured_with(&path, Some(&user_zero_point(25.0, UserMagSystem::Vega, false))).unwrap();
        let uncalibrated = measured_with(&path, None).unwrap();
        let phot = &out[RES_PHOTOMETRY];
        for key in ["flux_jy", "mag_ab", "mag_ab_err", "mag_ab_total"] {
            assert!(phot[key].is_null(), "{key}: {}", phot[key]);
        }
        assert_eq!(phot["mag_inst"], uncalibrated[RES_PHOTOMETRY]["mag_inst"]);
        assert!(phot["mag_inst"].is_number(), "{phot}");
        assert_eq!(out[RES_PHOTCAL]["mag_system"], "vega");
        let warnings = warning_texts(&out);
        let position = warnings.iter().position(|w| w == USER_ZERO_POINT_WARNING).expect("user zero point warning");
        assert!(warnings.len() > position + 1, "{warnings:?}");
    }

    #[test]
    fn a_user_zero_point_is_not_applied_when_the_header_calibrates() {
        let dir = tempfile::tempdir().unwrap();
        let key = jwst_star_mef(&dir.path().join("calibrated.fits"), vec![]);
        let typed = measured_with(&key, Some(&user_zero_point(25.0, UserMagSystem::Ab, false))).unwrap();
        let plain = measured_with(&key, None).unwrap();
        assert_eq!(typed[RES_PHOTOMETRY], plain[RES_PHOTOMETRY]);
        assert_eq!(typed[RES_PHOTCAL], plain[RES_PHOTCAL]);
        let prefix = "user zero point not applied: the header calibrates this image (";
        let warnings = warning_texts(&typed);
        assert_eq!(warnings.iter().filter(|w| w.starts_with(prefix)).count(), 1, "{warnings:?}");
        let origin = plain[RES_PHOTCAL]["origin"].as_str().unwrap();
        assert!(warnings.iter().any(|w| *w == format!("{prefix}{origin})")), "{warnings:?}");
        assert!(!warning_texts(&plain).iter().any(|w| w.starts_with(prefix)));
    }

    #[tokio::test]
    async fn a_non_finite_zero_point_is_refused_by_the_commands() {
        let path = "C:/nope/missing_user_zero_point.fits".to_string();
        let nan = || Some(user_zero_point(f64::NAN, UserMagSystem::Ab, false));
        let expected = "user zero point NaN must be a finite magnitude";
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, None, None, None, None, None, None, nan()).await.unwrap_err();
        assert!(single.contains(expected), "{single}");
        let batch = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], None, None, None, None, None, None, nan())
            .await
            .unwrap_err();
        assert!(batch.contains(expected), "{batch}");
        let status = photometry_status_cmd(path, None, nan()).await.unwrap_err();
        assert!(status.contains(expected), "{status}");
    }

    #[tokio::test]
    async fn batch_and_single_agree_with_a_typed_zero_point() {
        let dir = tempfile::tempdir().unwrap();
        let path = counts_star(dir.path(), "adu_batch.fits", &[("BUNIT", "adu"), ("EXPTIME", "600")]);
        let zp = || Some(user_zero_point(25.0, UserMagSystem::Ab, true));
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, Some(5.0), None, None, Some(false), None, None, zp())
            .await
            .unwrap();
        let batch = measure_photometry_batch_cmd(path, vec![(32.0, 32.0)], Some(5.0), None, None, None, None, Some(true), zp())
            .await
            .unwrap();
        assert!(single[RES_PHOTOMETRY]["flux_jy"].is_number(), "{}", single[RES_PHOTOMETRY]);
        assert_eq!(batch[RES_ROWS][0][RES_PHOTOMETRY], single[RES_PHOTOMETRY]);
        assert_eq!(batch[RES_WARNINGS], single[RES_WARNINGS]);
        assert_eq!(batch[RES_PHOTCAL], single[RES_PHOTCAL]);
    }

    fn sorted_keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn the_wire_names_of_the_calibration_are_the_ones_the_frontend_mirrors() {
        let dir = tempfile::tempdir().unwrap();
        let path = counts_star(dir.path(), "adu_wire.fits", &[("BUNIT", "adu")]);
        let header = cached_header(&path).unwrap();
        let ab = serde_json::to_value(user_photcal(Some(&header), &user_zero_point(25.0, UserMagSystem::Ab, false)).unwrap().0).unwrap();
        let vega =
            serde_json::to_value(user_photcal(Some(&header), &user_zero_point(25.0, UserMagSystem::Vega, false)).unwrap().0).unwrap();
        let mut njy_header = crate::types::header::HduHeader::empty();
        njy_header.set("BUNIT", "nJy".to_string());
        let njy = serde_json::to_value(PhotCal::from_header(&njy_header, None).unwrap()).unwrap();
        let mut fluxmag0_header = crate::types::header::HduHeader::empty();
        fluxmag0_header.set_f64("FLUXMAG0", 6.31e10);
        let survey = serde_json::to_value(PhotCal::from_header(&fluxmag0_header, None).unwrap()).unwrap();
        let photcal_keys = ["area_applied", "bunit", "convention", "jy_per_native_unit", "mag_system", "notes", "origin", "warnings"];
        for value in [&ab, &vega, &njy, &survey] {
            assert_eq!(sorted_keys(value), photcal_keys, "{value}");
        }
        let generic_keys = ["exptime", "exptime_applied", "keyword", "kind", "per_second", "system", "zp"];
        assert_eq!(ab["convention"]["kind"], "generic_zero_point");
        assert_eq!(sorted_keys(&ab["convention"]), generic_keys);
        assert_eq!(ab["convention"]["system"], "ab");
        assert_eq!(ab["mag_system"], "ab");
        assert!(ab["jy_per_native_unit"].is_number());
        assert_eq!(vega["convention"]["kind"], "generic_zero_point");
        assert_eq!(sorted_keys(&vega["convention"]), generic_keys);
        assert_eq!(vega["convention"]["system"], "vega");
        assert_eq!(vega["mag_system"], "vega");
        assert!(vega["jy_per_native_unit"].is_null());
        assert_eq!(njy["convention"]["kind"], "flux_density_per_pixel");
        assert_eq!(sorted_keys(&njy["convention"]), ["jy_per_unit", "kind", "unit"]);
        assert_eq!(njy["mag_system"], "ab");
        assert_eq!(survey["convention"]["kind"], "survey_zero_point");
        assert_eq!(sorted_keys(&survey["convention"]), ["exptime_applied", "kind", "source", "zp_ab"]);
        assert!(survey["convention"]["exptime_applied"].is_null());
    }

    #[test]
    fn the_user_zero_point_deserialises_from_the_wire_shape() {
        let parsed = serde_json::from_str::<UserZeroPoint>(r#"{"zp":25.0,"system":"vega","per_second":true}"#).unwrap();
        assert_eq!(parsed, user_zero_point(25.0, UserMagSystem::Vega, true));
        let ab = serde_json::from_str::<UserZeroPoint>(r#"{"zp":25.0,"system":"ab","per_second":false}"#).unwrap();
        assert_eq!(ab, user_zero_point(25.0, UserMagSystem::Ab, false));
        assert!(serde_json::from_str::<UserZeroPoint>(r#"{"zp":25.0,"system":"st","per_second":true}"#).is_err());
    }

    const ERROR_LAYER_NOTE_PREFIX: &str = "error layer: ";

    fn survey_star_photometry(key: &str, exclude_dq: bool, gain: Option<f64>) -> serde_json::Value {
        photometry_for_path(key, 32.0, 32.0, Some(5.0), None, false, exclude_dq, gain).unwrap()
    }

    fn unnamed_flags_warning(key: &str) -> String {
        let source = load_companions(key).unwrap().flags.expect("the flag layer is the mask of this image").source;
        format!(
            "{DQ_NOT_APPLIED_PREFIX}the flag layer has no usable bit names ({source}), so nothing is excluded until bits are chosen"
        )
    }

    fn dq_twin_of_the_inverse_variance_star(path: &std::path::Path) -> String {
        write_test_mef(
            path,
            &[("TELESCOP", "'JWST'".into())],
            &[star("SCI"), layer("DQ", HduData::I32(flagged(&NO_DATA_PIXELS, 1)), vec![])],
        );
        format!("{}#hdu=1", path.to_str().unwrap())
    }

    #[test]
    fn a_mask_with_named_planes_excludes_and_flags_saturation_in_photometry() {
        let dir = tempfile::tempdir().unwrap();
        let key = afw_star_mef(&dir.path().join("calexp.fits"), flat("VARIANCE", 4.0));
        let exclusion_mask = load_companions(&key).unwrap().exclusion_mask();
        assert_eq!(exclusion_mask, 3, "BAD and SAT are excluded by default, DETECTED never is");

        let excluded = survey_star_photometry(&key, true, None);
        assert_eq!(excluded[RES_MASKED], true, "{}", excluded[RES_WARNINGS]);
        assert_eq!(excluded[RES_PHOTOMETRY]["n_masked"], (exclusion_mask & 3).count_ones(), "{}", excluded[RES_PHOTOMETRY]);

        let kept = survey_star_photometry(&key, false, None);
        let phot = &kept[RES_PHOTOMETRY];
        assert_eq!(phot["saturated"], true, "{phot}");
        assert_eq!(phot["n_saturated"], 1, "{phot}");
        assert_eq!(phot["saturation_source"], "DQ SATURATED", "{phot}");
        assert_eq!(phot["err_used"], true, "{phot}");
        let sigma_twin = afw_star_mef(&dir.path().join("calexp_sigma.fits"), flat("ERR", 2.0));
        let twin_err = number(&survey_star_photometry(&sigma_twin, false, None)[RES_PHOTOMETRY], "flux_err");
        let gap = relative_gap(number(phot, "flux_err"), twin_err);
        assert!(gap <= 1e-6, "a variance of 4 is a sigma of 2: flux_err {} against {twin_err}", phot["flux_err"]);
    }

    #[test]
    fn an_inverse_variance_layer_is_named_once_and_its_pixels_without_data_leave_the_sum() {
        let dir = tempfile::tempdir().unwrap();
        let key = inverse_variance_star_mef(&dir.path().join("ivar.fits"));
        let origin = load_companions(&key).unwrap().err_origin.expect("the IVAR layer is the error layer of this image");
        let out = survey_star_photometry(&key, false, None);
        let named: Vec<String> = warning_texts(&out).into_iter().filter(|w| w.starts_with(ERROR_LAYER_NOTE_PREFIX)).collect();
        assert_eq!(
            named,
            vec![format!("{ERROR_LAYER_NOTE_PREFIX}{}, 3 pixels without data", origin.source)],
            "{}",
            out[RES_WARNINGS]
        );
        let phot = &out[RES_PHOTOMETRY];
        assert_eq!(phot["err_used"], true, "{phot}");
        let flux_err = number(phot, "flux_err");
        assert!(flux_err.is_finite() && flux_err > 0.0, "{flux_err}");
        assert_eq!(out[RES_MASKED], false);
        assert_eq!(phot["n_masked"], 3, "{phot}");

        let twin = dq_twin_of_the_inverse_variance_star(&dir.path().join("dq_twin.fits"));
        let through_dq = survey_star_photometry(&twin, true, None);
        assert_eq!(through_dq[RES_MASKED], true, "{}", through_dq[RES_WARNINGS]);
        assert_eq!(through_dq[RES_PHOTOMETRY]["n_masked"], 3);
        assert_eq!(
            number(phot, "net_flux").to_bits(),
            number(&through_dq[RES_PHOTOMETRY], "net_flux").to_bits(),
            "net flux {} against {} with the same three pixels left out through the DQ route",
            phot["net_flux"],
            through_dq[RES_PHOTOMETRY]["net_flux"]
        );

        let jwst_key = jwst_star_mef(&dir.path().join("jwst_star.fits"), vec![]);
        let jwst = survey_star_photometry(&jwst_key, false, None);
        assert!(!warning_texts(&jwst).iter().any(|w| w.starts_with("error layer")), "{}", jwst[RES_WARNINGS]);
        assert_eq!(jwst[RES_PHOTOMETRY]["n_masked"], 0);
        assert!(photometry_context(&jwst_key, false).unwrap().exclusion.is_none(), "a pass-through ERR plane gets no map");
        let no_data_map = photometry_context(&key, false).unwrap().exclusion.expect("the pixels without an error value");
        assert_eq!(no_data_map.iter().filter(|&&excluded| excluded != 0).count(), NO_DATA_PIXELS.len());
        assert!(NO_DATA_PIXELS.iter().all(|&(row, col)| no_data_map[[row, col]] == 1));
    }

    #[test]
    fn a_flag_layer_without_bit_names_excludes_nothing_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let key = unnamed_flag_star_mef(&dir.path().join("flag.fits"));
        let sentence = unnamed_flags_warning(&key);
        let out = survey_star_photometry(&key, true, Some(2.0));
        assert_eq!(out[RES_MASKED], false);
        assert_eq!(warning_texts(&out).last(), Some(&sentence), "{}", out[RES_WARNINGS]);
        assert_eq!(photometry_context(&key, true).unwrap().warnings.last(), Some(&sentence));
    }

    #[test]
    fn jwst_exclusion_and_saturation_are_bit_identical_with_or_without_a_flag_set() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = jwst_cal_mef(&dir.path().join("cal.fits"), &[], cal_sci_cards(), CalPlanes::ALL, vec![]);
        let loaded = load_companions(&fixture.key).unwrap();
        let table = loaded.dq.as_ref().map(|(_, table)| *table).expect("a cal file has a DQ plane");
        assert_eq!(loaded.exclusion_mask(), table.exclusion_mask());
        assert_eq!(Ok(loaded.saturated_mask()), table.mask_from_names(&[PHOTOMETRY_SATURATED_FLAG]));
        let mask = resolve_dq_mask(&fixture.key, true, (CAL_SIZE, CAL_SIZE)).expect("mask");
        assert_eq!(mask.excluded, 1);
        assert_eq!(mask.map[[CAL_EXCLUDED_PIXEL.0, CAL_EXCLUDED_PIXEL.1]], 1);
    }

    const LAYER_REFUSAL_PREFIX: &str = "no calibration applied: this plane is";

    fn plane_basis(key: &str) -> String {
        load_cached_full(key)
            .unwrap()
            .plane_info()
            .and_then(|info| info.pairing.layer.clone())
            .expect("a FITS plane has a layer role")
            .basis
    }

    fn assert_not_calibrated(key: &str, expected: &str) {
        assert!(expected.starts_with(LAYER_REFUSAL_PREFIX), "{expected}");
        assert!(expected.ends_with(&format!("({})", plane_basis(key))), "{expected} against the basis {}", plane_basis(key));
        let status = photometry_status_for_path(key, false, None).unwrap();
        assert!(status.photcal.is_null(), "{}", status.photcal);
        assert_eq!(status.calibration_source, None);
        assert_eq!(status.missing_reason.as_deref(), Some(expected));
        assert!(!status.warnings.iter().any(|w| w == expected), "{:?}", status.warnings);
        let ctx = photometry_context(key, false).unwrap();
        assert!(ctx.photcal.is_none());
        assert_eq!(ctx.calibration_gap.as_deref(), Some(expected));
        assert_eq!(ctx.warnings.iter().filter(|w| *w == expected).count(), 1, "{:?}", ctx.warnings);
        let typed = photometry_context_with(key, false, Some(&user_zero_point(25.0, UserMagSystem::Ab, false))).unwrap();
        assert!(typed.photcal.is_none() && typed.calibration_source.is_none(), "{:?}", typed.warnings);
        assert_eq!(typed.calibration_gap.as_deref(), Some(expected));
    }

    #[test]
    fn a_mask_plane_is_not_calibrated() {
        let dir = tempfile::tempdir().unwrap();
        let zero_point = ("MAGZERO", "24.6");
        let mask_path = dir.path().join("maskbits.fits");
        write_raw_hdus(
            &mask_path,
            &[(
                vec![
                    ("SIMPLE", "T".into()),
                    ("BITPIX", "32".into()),
                    ("NAXIS", "2".into()),
                    ("NAXIS1", SIZE.to_string()),
                    ("NAXIS2", SIZE.to_string()),
                    ("IMAGETYP", "'MASKBITS'".into()),
                    ("MAGZERO", "24.6".into()),
                ],
                vec![0u8; SIZE * SIZE * 4],
            )],
        );
        assert_not_calibrated(mask_path.to_str().unwrap(), "no calibration applied: this plane is a mask layer (IMAGETYP MASKBITS)");

        let science = counts_star(dir.path(), "science.fits", &[zero_point]);
        let calibrated = photometry_context(&science, false).unwrap();
        assert_eq!(calibrated.calibration_gap, None, "{:?}", calibrated.warnings);
        assert!(calibrated.photcal.is_some());
        assert_eq!(calibrated.calibration_source, Some("header"));

        let inverse_variance = counts_star(dir.path(), "invvar.fits", &[("IMAGETYP", "INVVAR"), zero_point]);
        assert_not_calibrated(
            &inverse_variance,
            "no calibration applied: this plane is an inverse-variance layer (IMAGETYP INVVAR)",
        );

        let layered = |name: &str, science: &'static str, second: &'static str, value: f32| {
            let path = dir.path().join(name);
            let mut second_plane = flat(second, value);
            second_plane.extra_cards = vec![("MAGZERO", "24.6".into())];
            write_test_mef(&path, &[], &[star(science), second_plane]);
            format!("{}#hdu=2", path.to_str().unwrap())
        };
        assert_not_calibrated(
            &layered("variance.fits", "IMAGE", "VARIANCE", 4.0),
            "no calibration applied: this plane is a variance layer (EXTNAME VARIANCE)",
        );

        let sigma = photometry_context(&layered("rms.fits", "SCI", "RMS", 0.25), false).unwrap();
        assert!(
            !sigma.calibration_gap.as_deref().is_some_and(|gap| gap.starts_with(LAYER_REFUSAL_PREFIX)),
            "a sigma layer is in the unit of its image and keeps its calibration: {:?}",
            sigma.calibration_gap
        );

        let named_only = dir.path().join("legacysurvey-0001m002-invvar-r.fits");
        let mut header = crate::types::header::HduHeader::empty();
        header.set("MAGZERO", "24.6".to_string());
        crate::infra::fits::writer::write_fits_mono(
            named_only.to_str().unwrap(),
            &ndarray::Array2::from_elem((SIZE, SIZE), 0.25f32),
            Some(&header),
        )
        .unwrap();
        assert_not_calibrated(
            named_only.to_str().unwrap(),
            "no calibration applied: this plane is an inverse-variance layer (file name -invvar-)",
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1, the Euclid VIS cutout with its RMS and FLAG siblings"]
    fn real_euclid_e1_status_names_the_rms_map_and_the_unnamed_flag_layer() {
        let science = crate::core::astrometry::gwcs::test_support::env_dir("ASTROBURST_SCIENCE_DATA_DIR")
            .join("survey")
            .join("E1")
            .join("EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits");
        assert!(science.is_file(), "{} is missing", science.display());
        let science = science.to_str().unwrap();

        let status = photometry_status_for_path(science, true, None).unwrap();
        println!("euclid e1 status: err {:?}, dq {:?}, warnings {:?}", status.err, status.dq, status.warnings);
        let err = status.err.expect("the RMS sibling is the error layer");
        assert!(err.origin.contains("MOSAIC-VIS-RMS"), "{}", err.origin);
        let dq = status.dq.expect("the FLAG sibling is the mask layer");
        assert!(dq.source.contains("MOSAIC-VIS-FLAG"), "{}", dq.source);
        assert!(!dq.exclusion_available, "bit 24 of FLAG lies on the sources: nothing may be excluded by default");
        assert!(dq.unavailable_reason.is_some());
        let not_applied: Vec<&String> = status.warnings.iter().filter(|w| w.starts_with(DQ_NOT_APPLIED_PREFIX)).collect();
        assert_eq!(not_applied.len(), 1, "{:?}", status.warnings);

        let (x, y) = (337.17675170716575, 127.33561326757126);
        let out = photometry_for_path(science, x, y, Some(4.0), None, false, true, None).unwrap();
        let phot = &out[RES_PHOTOMETRY];
        println!(
            "euclid e1 photometry at ({x}, {y}), r = 4 px: centroid ({}, {}) px, net_flux {} +/- {} (image unit x pix), err_used {}, masked {}, n_masked {}, warnings {}",
            phot["x"], phot["y"], phot["net_flux"], phot["flux_err"], phot["err_used"], out[RES_MASKED], phot["n_masked"], out[RES_WARNINGS]
        );
        let golden = "plan/goldens/survey/phot_euclid_e1.json point_like_sources[0]";
        assert_eq!(phot["err_used"], true, "{golden}");
        assert_eq!(out[RES_MASKED], false, "{golden}");
        assert_eq!(phot["n_masked"], 0, "{golden}: none of the 22 sentinel pixels of the RMS map lies on a source");
        let offset = (number(phot, "x") - x).hypot(number(phot, "y") - y);
        assert!(offset <= 1.0, "{golden}: centroid {offset} px from the requested position");
    }

    fn assert_real_gain_model_agrees(relative: &str, (x, y): (f64, f64), radius: f64) {
        let path = crate::core::astrometry::gwcs::test_support::env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join(relative);
        assert!(path.is_file(), "{} is missing", path.display());
        let path = path.to_str().unwrap();
        let out = photometry_for_path(path, x, y, Some(radius), None, false, false, None).unwrap();
        let dims = load_cached_full(path).or_else(|_| load_cached(path)).unwrap().arr().dim();
        let has_err = has_err_extension(path, dims);
        let phot = &out[RES_PHOTOMETRY];
        println!(
            "{relative}: error reference {:?}, dims {dims:?}, has_err_extension {has_err}, err_used {}, net_flux {} +/- {} (image unit x pix) at ({x}, {y}) px, r = {radius} px",
            error_reference(path),
            phot["err_used"],
            phot["net_flux"],
            phot["flux_err"]
        );
        assert_eq!(phot["err_used"], true, "{}", out[RES_WARNINGS]);
        assert!(has_err);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz"]
    fn real_legacy_l2_r_gain_model_agrees_with_the_measurement() {
        assert_real_gain_model_agrees(
            "L2/legacysurvey-0532m280-image-r.fits.fz",
            (3333.811483943494, 625.4191790671039),
            7.633587786259541,
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits"]
    fn real_euclid_e1_gain_model_agrees_with_the_measurement() {
        assert_real_gain_model_agrees(
            "E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits",
            (337.17675170716575, 127.33561326757126),
            4.0,
        );
    }
}
