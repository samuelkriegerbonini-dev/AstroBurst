mod time_series;
pub use time_series::*;

use std::time::Instant;

use serde_json::json;
use tauri::ipc::Response;
use rayon::prelude::*;

use crate::cmd::common::{
    blocking_cmd, cached_header, dq_exclusion, load_cached, load_cached_full, load_companions, HEADER_DISPLAY_REFERRED,
};
use crate::types::constants::{
    HISTOGRAM_BINS, HISTOGRAM_BINS_DISPLAY, RES_BINS, RES_BIN_COUNT, RES_MIN, RES_MAX,
    RES_DATA_MIN, RES_DATA_MAX, RES_MEDIAN, RES_MEAN, RES_SIGMA, RES_MAD, RES_TOTAL_PIXELS,
    RES_AUTO_STF, RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT, RES_ELAPSED_MS,
    RES_RA, RES_DEC, RES_GMAG, RES_BP_RP, RES_SEPARATION_ARCSEC,
    RES_PHOTOMETRY, RES_SKY, RES_GAIA,
    RES_SUBFRAMES, RES_TOTAL, RES_ACCEPTED, RES_REJECTED,
    RES_MASKED, RES_DQ_EXCLUDED, RES_LABEL, RES_WARNINGS,
    RES_ROWS, RES_INDEX, RES_ERROR, RES_N_MEASURED, RES_N_FAILED, RES_N_DETECTED,
};
use crate::types::image::{AutoStfConfig, Histogram, ImageStats, StfParams};
use crate::core::analysis::fft::{compute_power_spectrum, FftResult};
use crate::core::analysis::photometry::{
    measure_star_prepared, saturation_level, MaskedImage, PhotometryConfig, StarPhotometry, MAX_APERTURE_RADIUS,
    MIN_APERTURE_RADIUS,
};
use crate::core::analysis::star_detection::{detect_stars as detect_stars_core, DetectionResult};
use crate::core::astrometry::spcc::{query_gaia_vizier, CatalogStar};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::dq_flags::{apply_exclusion, exclusion_map};
use crate::core::imaging::stats::{
    build_histogram, compute_histogram_with_stats, compute_image_stats, downsample_histogram, is_valid_pixel,
};
use crate::core::imaging::stf::auto_stf;
use crate::core::metadata::photcal::{missing_calibration_reason, PhotCal};
use crate::infra::cache::ImageEntry;
use crate::types::header::HduHeader;

const PAR_THRESHOLD: usize = 1_000_000;
const MAX_BATCH_POINTS: usize = 5000;
const PAR_BATCH_POINTS: usize = 64;
const MAX_SKY_ANNULUS_RADIUS: f64 = 512.0;
const FFT_WINDOWED_FLAG: u32 = 1;
const FFT_DOWNSAMPLED_FLAG: u32 = 2;
const FFT_HEADER_BYTES: usize = 40;
const GAIA_MATCH_RADIUS_ARCSEC: f64 = 5.0;
const GAIA_MATCH_CONE_DEG: f64 = 0.01;
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
    let map = match dq_exclusion(path) {
        Ok(Some(m)) if m.dim() == dims => m,
        Ok(_) => return None,
        Err(e) => {
            log::warn!("DQ exclusion unavailable for {}: {:#}", path, e);
            return None;
        }
    };
    let excluded = map.iter().filter(|&&v| v != 0).count() as u64;
    Some(DqMask { map, excluded })
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
        let (er, eg, eb) = crate::cmd::io::rgb_source_planes(path.as_deref())?;

        let r = er.as_ref();
        let g = eg.as_ref();
        let b = eb.as_ref();
        let (rows, cols) = r.dim();

        let r_s = r.as_slice().unwrap();
        let g_s = g.as_slice().unwrap();
        let b_s = b.as_slice().unwrap();

        let n = rows * cols;
        let lum_vec: Vec<f32> = if n > PAR_THRESHOLD {
            (0..n).into_par_iter()
                .map(|i| r_s[i] * 0.2126 + g_s[i] * 0.7152 + b_s[i] * 0.0722)
                .collect()
        } else {
            (0..n)
                .map(|i| r_s[i] * 0.2126 + g_s[i] * 0.7152 + b_s[i] * 0.0722)
                .collect()
        };

        let mut lum_min = f32::INFINITY;
        let mut lum_max = f32::NEG_INFINITY;
        for &v in &lum_vec {
            if v.is_finite() {
                if v < lum_min { lum_min = v; }
                if v > lum_max { lum_max = v; }
            }
        }

        let range = lum_max - lum_min;
        let normalized: Vec<f32> = if range > 1e-10 {
            let inv = 1.0 / range;
            if n > PAR_THRESHOLD {
                lum_vec.par_iter()
                    .map(|&v| if v.is_finite() { ((v - lum_min) * inv).clamp(0.0, 1.0) } else { 0.0 })
                    .collect()
            } else {
                lum_vec.iter()
                    .map(|&v| if v.is_finite() { ((v - lum_min) * inv).clamp(0.0, 1.0) } else { 0.0 })
                    .collect()
            }
        } else {
            vec![0.0; n]
        };

        let lum = ndarray::Array2::from_shape_vec((rows, cols), normalized)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        capped_detection_json(detect_stars_core(&lum, sigma), max_stars, t0)
    })
}

pub const PHOTOMETRY_SATURATED_FLAG: &str = "SATURATED";
pub const HEADER_PROCESSING_PROVENANCE: &str = "ABPROC";
pub const RES_PHOTCAL: &str = "photcal";

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
    let err = comps.err.filter(|entry| entry.arr().dim() == dims);
    let saturated = comps.dq.and_then(|(entry, table)| {
        let bits = table.mask_from_names(&[PHOTOMETRY_SATURATED_FLAG]).ok()?;
        let plane = entry.int_plane()?;
        (plane.bits.dim() == dims).then(|| exclusion_map(&plane.bits, bits))
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

fn gaia_match_outcome(
    coord_ra: f64,
    coord_dec: f64,
    query: Result<Vec<CatalogStar>, String>,
) -> (serde_json::Value, Option<String>) {
    let stars = match query {
        Ok(stars) => stars,
        Err(reason) => return (serde_json::Value::Null, Some(format!("Gaia query failed: {reason}"))),
    };
    let cos_dec = coord_dec.to_radians().cos();
    let mut best: Option<(f64, usize)> = None;
    for (i, s) in stars.iter().enumerate() {
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
                RES_GMAG: stars[i].gmag,
                RES_BP_RP: stars[i].bp_rp,
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

pub(crate) struct PhotometryContext {
    pub entry: ImageEntry,
    pub mask: Option<DqMask>,
    pub planes: PhotometryPlanes,
    pub wcs: Option<WcsTransform>,
    pub photcal: Option<PhotCal>,
    pub warnings: Vec<String>,
}

pub(crate) fn photometry_context(path: &str, exclude_dq: bool) -> anyhow::Result<PhotometryContext> {
    let entry = load_cached_full(path).or_else(|_| load_cached(path))?;
    let dims = entry.arr().dim();
    let mask = resolve_dq_mask(path, exclude_dq, dims);
    let planes = photometry_planes(path, dims);
    let header = entry.header();
    let wcs = header.and_then(|h| WcsTransform::from_header(h).ok());
    let photcal = header.and_then(|h| PhotCal::from_header(h, wcs.as_ref()));
    let mut warnings: Vec<String> = Vec::new();
    match &photcal {
        Some(cal) => warnings.extend(cal.warnings.iter().cloned()),
        None => warnings.push(missing_calibration_reason(header)),
    }
    warnings.extend(processed_data_warning(header));
    Ok(PhotometryContext { entry, mask, planes, wcs, photcal, warnings })
}

pub(crate) fn processed_data_warning(header: Option<&HduHeader>) -> Option<String> {
    let provenance = header?.get(HEADER_PROCESSING_PROVENANCE)?;
    Some(format!("photometry on processed data ({})", provenance.trim().trim_matches('\'').trim()))
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
        MaskedImage::new(self.entry.arr(), self.mask.as_ref().map(|m| &m.map))
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
    let t0 = Instant::now();
    let ctx = photometry_context(path, exclude_dq)?;
    let config = ctx.config(aperture_radius, sky_annulus, gain);
    let source = ctx.masked_image().map_err(anyhow::Error::msg)?;
    let phot = ctx.measure(&source, x, y, &config).map_err(|e| anyhow::anyhow!(e))?;

    let sky = ctx.sky_json(&phot);
    let mut warnings = ctx.warnings.clone();
    let gaia = match (&ctx.wcs, gaia_match) {
        (Some(wcs), true) => {
            let coord = wcs.pixel_to_world(phot.x, phot.y);
            if coord.ra.is_finite() && coord.dec.is_finite() {
                let query = query_gaia_vizier(coord.ra, coord.dec, GAIA_MATCH_CONE_DEG, 0);
                let (gaia, warning) = gaia_match_outcome(coord.ra, coord.dec, query);
                warnings.extend(warning);
                gaia
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

    Ok(json!({
        RES_PHOTOMETRY: serde_json::to_value(&phot)?,
        RES_SKY: sky,
        RES_GAIA: gaia,
        RES_PHOTCAL: ctx.photcal_json()?,
        RES_WARNINGS: warnings,
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
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        if !x.is_finite() || !y.is_finite() {
            anyhow::bail!("photometry position ({x}, {y}) must be finite");
        }
        let aperture_radius = aperture_radius_arg(aperture_radius)?;
        let sky_annulus = annulus_arg(annulus_inner, annulus_outer)?;
        check_annulus_clears_aperture(aperture_radius, sky_annulus)?;
        photometry_for_path(
            &path,
            x,
            y,
            aperture_radius,
            sky_annulus,
            gaia_match.unwrap_or(false),
            exclude_dq.unwrap_or(false),
            gain,
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
        let with_growth_curve = with_growth_curve.unwrap_or(false);

        let ctx = photometry_context(&path, exclude_dq.unwrap_or(false))?;
        let config = ctx.config(aperture_radius, sky_annulus, gain);
        let source = ctx.masked_image().map_err(anyhow::Error::msg)?;
        let measure = |&(x, y): &(f64, f64)| ctx.measure(&source, x, y, &config);
        let measured: Vec<Result<StarPhotometry, String>> = if points.len() > PAR_BATCH_POINTS {
            points.par_iter().map(measure).collect()
        } else {
            points.iter().map(measure).collect()
        };

        let mut n_measured = 0usize;
        let mut rows = Vec::with_capacity(measured.len());
        for (index, outcome) in measured.into_iter().enumerate() {
            match outcome {
                Ok(mut phot) => {
                    n_measured += 1;
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

        Ok(json!({
            RES_ROWS: rows,
            RES_PHOTCAL: ctx.photcal_json()?,
            RES_WARNINGS: ctx.warnings,
            RES_MASKED: ctx.mask.is_some(),
            RES_N_MEASURED: n_measured,
            RES_N_FAILED: n_failed,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
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
mod tests {
    use super::*;
    use crate::infra::fits::reader::test_fixtures::{sci_err_dq_mef, write_test_mef, HduData, TestHdu};

    fn gaussian_pixels(size: usize, amp: f32, sigma: f32, bg: f32) -> Vec<f32> {
        let c = (size / 2) as f32;
        (0..size * size)
            .map(|i| {
                let x = (i % size) as f32 - c;
                let y = (i / size) as f32 - c;
                bg + amp * (-(x * x + y * y) / (2.0 * sigma * sigma)).exp()
            })
            .collect()
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
        let header = crate::cmd::common::derived_output_header(
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
    fn resolve_dq_mask_counts_excluded_pixels_on_mef() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hist.fits");
        let mut dq = vec![-2147483648i32; 16];
        dq[1] = -2147483647;
        dq[6] = -2147483645;
        dq[9] = -2147483646;
        sci_err_dq_mef(&path, 4, 4, dq);
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
        let single = measure_photometry_cmd(path.clone(), 31.0, 33.0, Some(5.0), Some(10.0), Some(15.0), Some(false), None, Some(2.0))
            .await
            .unwrap();
        let batch = measure_photometry_batch_cmd(path.clone(), vec![(31.0, 33.0)], Some(5.0), Some(10.0), Some(15.0), Some(2.0), None, Some(true))
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

        let mixed = measure_photometry_batch_cmd(path, vec![(32.0, 32.0), (5.0, 5.0)], None, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(mixed[RES_N_MEASURED], 2);
        assert_eq!(mixed[RES_ROWS][1][RES_INDEX], 1);
    }

    #[tokio::test]
    async fn a_sky_annulus_inside_the_aperture_is_refused_by_both_commands() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "refused.fits");
        let sentence = "sky annulus inner radius 5 must be larger than the aperture radius 5";
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, Some(5.0), Some(5.0), Some(10.0), Some(false), None, None)
            .await
            .unwrap_err();
        assert!(single.contains(sentence), "{single}");
        let batch = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], Some(5.0), Some(5.0), Some(10.0), None, None, None)
            .await
            .unwrap_err();
        assert!(batch.contains(sentence), "{batch}");

        let auto_radius = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], None, Some(2.0), Some(4.0), None, None, None)
            .await
            .unwrap();
        assert_eq!(auto_radius[RES_N_FAILED], 1);
        assert!(auto_radius[RES_ROWS][0][RES_ERROR].as_str().unwrap().contains("must be larger than the aperture radius"));
        assert!(auto_radius[RES_ROWS][0][RES_PHOTOMETRY].is_null());

        let half = measure_photometry_cmd(path.clone(), 32.0, 32.0, None, Some(8.0), None, Some(false), None, None)
            .await
            .unwrap_err();
        assert!(half.contains("sky annulus needs both an inner and an outer radius"), "{half}");
        let inverted = measure_photometry_cmd(path, 32.0, 32.0, None, Some(12.0), Some(8.0), Some(false), None, None)
            .await
            .unwrap_err();
        assert!(inverted.contains("sky annulus outer radius 8 must be larger than the inner radius 12"), "{inverted}");
    }

    #[tokio::test]
    async fn batch_photometry_refuses_too_many_empty_or_non_finite_points() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "limits.fits");
        let too_many = vec![(32.0, 32.0); 6000];
        let err = measure_photometry_batch_cmd(path.clone(), too_many, None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("6000 points exceeds the limit of 5000"), "{err}");
        let empty = measure_photometry_batch_cmd(path.clone(), vec![], None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(empty.contains("at least one point"), "{empty}");
        let nan = measure_photometry_batch_cmd(path, vec![(32.0, f64::NAN)], None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(nan.contains("point 0"), "{nan}");
    }

    #[tokio::test]
    async fn batch_photometry_without_growth_curves_still_reports_encircled_energy_radii() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "growth.fits");
        let out = measure_photometry_batch_cmd(path, vec![(32.0, 32.0)], Some(6.0), None, None, None, None, Some(false))
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
        let out = measure_photometry_batch_cmd(path, points, Some(5.0), None, None, None, None, None).await.unwrap();
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
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, None, Some(20.0), Some(3000.0), Some(false), None, None)
            .await
            .unwrap_err();
        assert!(single.contains(sentence), "{single}");
        let batch = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0); 100], Some(5.0), Some(20.0), Some(3000.0), None, None, None)
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
        let single = measure_photometry_cmd(path.clone(), 32.0, 32.0, Some(1.5), None, None, Some(false), None, None)
            .await
            .unwrap_err();
        assert!(single.contains("aperture radius 1.5 must be between 2 and 60 pixels"), "{single}");
        let small = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0); 3], Some(1.5), Some(1.8), Some(4.0), None, None, None)
            .await
            .unwrap_err();
        assert!(small.contains("aperture radius 1.5 must be between 2 and 60 pixels"), "{small}");
        let large = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], Some(100.0), None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(large.contains("aperture radius 100 must be between 2 and 60 pixels"), "{large}");
        let single_large = measure_photometry_cmd(path.clone(), 32.0, 32.0, Some(100.0), None, None, Some(false), None, None)
            .await
            .unwrap_err();
        assert!(single_large.contains("aperture radius 100 must be between 2 and 60 pixels"), "{single_large}");
        let nan = measure_photometry_batch_cmd(path.clone(), vec![(32.0, 32.0)], Some(f64::NAN), None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(nan.contains("aperture radius NaN"), "{nan}");
        let edge = measure_photometry_batch_cmd(path, vec![(32.0, 32.0)], Some(2.0), None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(edge[RES_N_MEASURED], 1);
        assert_eq!(edge[RES_ROWS][0][RES_PHOTOMETRY]["aperture_radius"], 2.0);
    }

    #[tokio::test]
    async fn batch_photometry_reports_a_sky_annulus_with_no_usable_pixel_as_a_failed_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = gaussian_fits(dir.path(), "thin_annulus.fits");
        let out = measure_photometry_batch_cmd(path, vec![(32.0, 32.0)], Some(5.0), Some(10.0), Some(10.3), None, None, None)
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
        let batch = measure_photometry_batch_cmd(key.clone(), points.clone(), None, None, None, None, Some(true), Some(true))
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

    #[test]
    fn a_gaia_failure_or_an_empty_cone_becomes_a_warning_and_the_nearest_star_within_five_arcsec_matches() {
        let star = |ra: f64, dec: f64, gmag: f64| CatalogStar { ra, dec, bp_rp: 0.8, gmag: Some(gmag) };
        let (value, warning) = gaia_match_outcome(10.0, 20.0, Err("VizieR request failed: timeout".into()));
        assert!(value.is_null());
        assert_eq!(warning.as_deref(), Some("Gaia query failed: VizieR request failed: timeout"));
        let (value, warning) = gaia_match_outcome(10.0, 20.0, Ok(vec![]));
        assert!(value.is_null());
        assert_eq!(warning.as_deref(), Some("Gaia: no G<17 star within 5\""));
        let far = star(10.0, 20.0 + 10.0 / 3600.0, 11.0);
        let (value, warning) = gaia_match_outcome(10.0, 20.0, Ok(vec![far.clone()]));
        assert!(value.is_null());
        assert_eq!(warning.as_deref(), Some("Gaia: no G<17 star within 5\""));
        let near = star(10.0 + 3.0 / 3600.0 / 20.0f64.to_radians().cos(), 20.0, 12.5);
        let nearer = star(10.0, 20.0 - 2.0 / 3600.0, 13.0);
        let (value, warning) = gaia_match_outcome(10.0, 20.0, Ok(vec![far, near, nearer]));
        assert!(warning.is_none(), "{warning:?}");
        assert_eq!(value[RES_GMAG], 13.0);
        assert!((value[RES_SEPARATION_ARCSEC].as_f64().unwrap() - 2.0).abs() < 1e-6, "{value}");
        let (wrapped, _) = gaia_match_outcome(359.9995, 0.0, Ok(vec![star(0.0005, 0.0, 9.0)]));
        assert!((wrapped[RES_SEPARATION_ARCSEC].as_f64().unwrap() - 3.6).abs() < 1e-6, "{wrapped}");
    }

    fn wcs_gaussian_fits(dir: &std::path::Path, name: &str) -> String {
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
        header.set_f64("CRVAL1", 180.0);
        header.set_f64("CRVAL2", 45.0);
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
        let out = measure_photometry_cmd(path, 32.0, 32.0, None, None, None, None, None, None).await.unwrap();
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
}
