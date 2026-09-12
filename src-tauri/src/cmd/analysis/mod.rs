use std::time::Instant;

use serde_json::json;
use tauri::ipc::Response;
use rayon::prelude::*;

use crate::cmd::common::{blocking_cmd, dq_exclusion, load_cached};
use crate::types::constants::{
    HISTOGRAM_BINS_DISPLAY, RES_BINS, RES_BIN_COUNT, RES_MIN, RES_MAX,
    RES_DATA_MIN, RES_DATA_MAX, RES_MEDIAN, RES_MEAN, RES_SIGMA, RES_MAD, RES_TOTAL_PIXELS,
    RES_AUTO_STF, RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT, RES_ELAPSED_MS,
    RES_RA, RES_DEC, RES_GMAG, RES_BP_RP, RES_SEPARATION_ARCSEC,
    RES_PHOTOMETRY, RES_SKY, RES_GAIA,
    RES_SUBFRAMES, RES_TOTAL, RES_ACCEPTED, RES_REJECTED,
    RES_MASKED, RES_DQ_EXCLUDED,
};
use crate::types::image::AutoStfConfig;
use crate::core::analysis::fft::compute_power_spectrum;
use crate::core::analysis::photometry::{measure_star_masked, PhotometryConfig};
use crate::core::analysis::star_detection::detect_stars as detect_stars_core;
use crate::core::astrometry::spcc::query_gaia_vizier;
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::dq_flags::apply_exclusion;
use crate::core::imaging::stats::{compute_histogram_with_stats, compute_image_stats, downsample_histogram};
use crate::core::imaging::stf::auto_stf;

const PAR_THRESHOLD: usize = 1_000_000;

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

#[tauri::command]
pub async fn compute_histogram(path: String, exclude_dq: Option<bool>) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let cached = load_cached(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), cached.arr().dim());

        let masked_arr = match &mask {
            Some(m) => Some(apply_exclusion(cached.arr(), &m.map)?),
            None => None,
        };
        let masked_stats = masked_arr.as_ref().map(compute_image_stats);
        let arr = masked_arr.as_ref().unwrap_or(cached.arr());
        let stats = masked_stats.as_ref().unwrap_or(cached.stats());

        let hist = compute_histogram_with_stats(arr, stats);
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);
        let stf_params = auto_stf(stats, &AutoStfConfig::default());

        Ok(json!({
            RES_BINS: display_bins,
            RES_BIN_COUNT: display_bins.len(),
            RES_MIN: hist.min,
            RES_MAX: hist.max,
            RES_DATA_MIN: stats.min,
            RES_DATA_MAX: stats.max,
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

        let header_size = 32;
        let mut buf = Vec::with_capacity(header_size + pixel_count);

        buf.extend_from_slice(&(cols as u32).to_le_bytes());
        buf.extend_from_slice(&(rows as u32).to_le_bytes());
        buf.extend_from_slice(&dc.to_le_bytes());
        buf.extend_from_slice(&max_val.to_le_bytes());
        buf.extend_from_slice(&elapsed_ms.to_le_bytes());
        buf.extend_from_slice(&(fft_result.original_size as u32).to_le_bytes());
        buf.extend_from_slice(&if fft_result.windowed { 1u32 } else { 0u32 }.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());

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

#[tauri::command]
pub async fn detect_stars(
    path: String,
    sigma: f64,
    max_stars: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let mut result = detect_stars_core(load_cached(&path)?.arr(), sigma);
        result.stars.truncate(max_stars);
        let mut val = serde_json::to_value(&result)?;
        if let Some(obj) = val.as_object_mut() {
            obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
        }
        Ok(val)
    })
}

#[tauri::command]
pub async fn detect_stars_composite(
    sigma: f64,
    max_stars: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let (er, eg, eb) = crate::cmd::helpers::load_composite_rgb()
            .map_err(|_| anyhow::anyhow!("RGB composite not available. Run Compose RGB first."))?;

        let r = er.arr();
        let g = eg.arr();
        let b = eb.arr();
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

        let mut result = detect_stars_core(&lum, sigma);
        result.stars.truncate(max_stars);
        let mut val = serde_json::to_value(&result)?;
        if let Some(obj) = val.as_object_mut() {
            obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
        }
        Ok(val)
    })
}

#[tauri::command]
pub async fn measure_photometry_cmd(
    path: String,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    gaia_match: Option<bool>,
    exclude_dq: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let entry = crate::cmd::common::load_cached_full(&path)
            .or_else(|_| load_cached(&path))?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());

        let config = PhotometryConfig {
            aperture_radius: aperture_radius.filter(|r| r.is_finite() && *r > 0.0),
            image_max: Some(entry.stats().max),
            ..PhotometryConfig::default()
        };

        let phot = measure_star_masked(entry.arr(), x, y, &config, mask.as_ref().map(|m| &m.map))
            .map_err(|e| anyhow::anyhow!(e))?;

        let mut sky = serde_json::Value::Null;
        let mut gaia = serde_json::Value::Null;

        if let Some(header) = entry.header() {
            if let Ok(wcs) = WcsTransform::from_header(header) {
                let coord = wcs.pixel_to_world(phot.x, phot.y);
                sky = json!({ RES_RA: coord.ra, RES_DEC: coord.dec });

                if gaia_match.unwrap_or(true) {
                    if let Ok(stars) = query_gaia_vizier(coord.ra, coord.dec, 0.01, 1) {
                        let cos_dec = coord.dec.to_radians().cos();
                        let mut best: Option<(f64, usize)> = None;
                        for (i, s) in stars.iter().enumerate() {
                            let mut dra = (coord.ra - s.ra).abs();
                            if dra > 180.0 {
                                dra = 360.0 - dra;
                            }
                            let sep = ((dra * cos_dec).powi(2) + (coord.dec - s.dec).powi(2))
                                .sqrt()
                                * 3600.0;
                            if sep < 5.0 && best.map_or(true, |(bd, _)| sep < bd) {
                                best = Some((sep, i));
                            }
                        }
                        if let Some((sep, i)) = best {
                            gaia = json!({
                                RES_GMAG: stars[i].gmag,
                                RES_BP_RP: stars[i].bp_rp,
                                RES_SEPARATION_ARCSEC: sep,
                            });
                        }
                    }
                }
            }
        }

        Ok(json!({
            RES_PHOTOMETRY: serde_json::to_value(&phot)?,
            RES_SKY: sky,
            RES_GAIA: gaia,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_MASKED: mask.is_some(),
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
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;

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
}
