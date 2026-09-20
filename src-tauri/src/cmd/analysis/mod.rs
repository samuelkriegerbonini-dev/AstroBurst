use std::time::Instant;

use serde_json::json;
use tauri::ipc::Response;
use rayon::prelude::*;

use crate::cmd::common::{blocking_cmd, dq_exclusion, load_cached, load_cached_full, load_companions};
use crate::types::constants::{
    HISTOGRAM_BINS_DISPLAY, RES_BINS, RES_BIN_COUNT, RES_MIN, RES_MAX,
    RES_DATA_MIN, RES_DATA_MAX, RES_MEDIAN, RES_MEAN, RES_SIGMA, RES_MAD, RES_TOTAL_PIXELS,
    RES_AUTO_STF, RES_SHADOW, RES_MIDTONE, RES_HIGHLIGHT, RES_ELAPSED_MS,
    RES_RA, RES_DEC, RES_GMAG, RES_BP_RP, RES_SEPARATION_ARCSEC,
    RES_PHOTOMETRY, RES_SKY, RES_GAIA,
    RES_SUBFRAMES, RES_TOTAL, RES_ACCEPTED, RES_REJECTED,
    RES_MASKED, RES_DQ_EXCLUDED, RES_LABEL, RES_WARNINGS,
};
use crate::types::image::AutoStfConfig;
use crate::core::analysis::fft::compute_power_spectrum;
use crate::core::analysis::photometry::{measure_star_full, saturation_level, PhotometryConfig, StarPhotometry};
use crate::core::analysis::star_detection::detect_stars as detect_stars_core;
use crate::core::astrometry::spcc::query_gaia_vizier;
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::dq_flags::{apply_exclusion, exclusion_map};
use crate::core::imaging::stats::{compute_histogram_with_stats, compute_image_stats, downsample_histogram};
use crate::core::imaging::stf::auto_stf;
use crate::core::metadata::photcal::{missing_calibration_reason, PhotCal};
use crate::infra::cache::ImageEntry;

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

pub const PHOTOMETRY_SATURATED_FLAG: &str = "SATURATED";
pub const HEADER_PROCESSING_PROVENANCE: &str = "ABPROC";
pub const RES_PHOTCAL: &str = "photcal";

pub(crate) struct PhotometryPlanes {
    pub err: Option<ImageEntry>,
    pub saturated: Option<ndarray::Array2<u8>>,
}

pub(crate) fn photometry_planes(path: &str, dims: (usize, usize), warnings: &mut Vec<String>) -> PhotometryPlanes {
    let comps = match load_companions(path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("companion planes unavailable for {}: {:#}", path, e);
            return PhotometryPlanes { err: None, saturated: None };
        }
    };
    let err = comps.err.and_then(|entry| {
        let err_dims = entry.arr().dim();
        if err_dims == dims {
            Some(entry)
        } else {
            warnings.push(format!(
                "ERR plane {}x{} does not match the image {}x{}; ignored",
                err_dims.1, err_dims.0, dims.1, dims.0
            ));
            None
        }
    });
    let saturated = comps.dq.and_then(|(entry, table)| {
        let bits = table.mask_from_names(&[PHOTOMETRY_SATURATED_FLAG]).ok()?;
        let plane = entry.int_plane()?;
        (plane.bits.dim() == dims).then(|| exclusion_map(&plane.bits, bits))
    });
    PhotometryPlanes { err, saturated }
}

fn apply_calibration(phot: &mut StarPhotometry, cal: &PhotCal) {
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

fn gaia_match_json(coord_ra: f64, coord_dec: f64) -> serde_json::Value {
    let Ok(stars) = query_gaia_vizier(coord_ra, coord_dec, 0.01, 1) else {
        return serde_json::Value::Null;
    };
    let cos_dec = coord_dec.to_radians().cos();
    let mut best: Option<(f64, usize)> = None;
    for (i, s) in stars.iter().enumerate() {
        let mut dra = (coord_ra - s.ra).abs();
        if dra > 180.0 {
            dra = 360.0 - dra;
        }
        let sep = ((dra * cos_dec).powi(2) + (coord_dec - s.dec).powi(2)).sqrt() * 3600.0;
        if sep < 5.0 && best.map_or(true, |(bd, _)| sep < bd) {
            best = Some((sep, i));
        }
    }
    match best {
        Some((sep, i)) => json!({
            RES_GMAG: stars[i].gmag,
            RES_BP_RP: stars[i].bp_rp,
            RES_SEPARATION_ARCSEC: sep,
        }),
        None => serde_json::Value::Null,
    }
}

pub(crate) fn photometry_for_path(
    path: &str,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    gaia_match: bool,
    exclude_dq: bool,
    gain: Option<f64>,
) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let entry = load_cached_full(path).or_else(|_| load_cached(path))?;
    let dims = entry.arr().dim();
    let mask = resolve_dq_mask(path, exclude_dq, dims);
    let mut warnings: Vec<String> = Vec::new();
    let planes = photometry_planes(path, dims, &mut warnings);
    let header = entry.header();
    let wcs = header.and_then(|h| WcsTransform::from_header(h).ok());
    let photcal = header.and_then(|h| PhotCal::from_header(h, wcs.as_ref()));

    let config = PhotometryConfig {
        aperture_radius: aperture_radius.filter(|r| r.is_finite() && *r > 0.0),
        saturation: Some(saturation_level(header, entry.stats().max)),
        gain: gain.filter(|g| g.is_finite() && *g > 0.0),
        ..PhotometryConfig::default()
    };

    let mut phot = measure_star_full(
        entry.arr(),
        planes.err.as_ref().map(|e| e.arr()),
        mask.as_ref().map(|m| &m.map),
        planes.saturated.as_ref(),
        x,
        y,
        &config,
    )
    .map_err(|e| anyhow::anyhow!(e))?;

    match &photcal {
        Some(cal) => {
            apply_calibration(&mut phot, cal);
            warnings.extend(cal.warnings.iter().cloned());
        }
        None => warnings.push(missing_calibration_reason(header)),
    }
    if let Some(provenance) = header.and_then(|h| h.get(HEADER_PROCESSING_PROVENANCE)) {
        warnings.push(format!("photometry on processed data ({})", provenance.trim().trim_matches('\'').trim()));
    }

    let mut sky = serde_json::Value::Null;
    let mut gaia = serde_json::Value::Null;
    if let Some(wcs) = &wcs {
        let coord = wcs.pixel_to_world(phot.x, phot.y);
        sky = json!({ RES_RA: coord.ra, RES_DEC: coord.dec });
        if gaia_match {
            gaia = gaia_match_json(coord.ra, coord.dec);
        }
    }

    let photcal_value = match &photcal {
        Some(cal) => photcal_json(cal)?,
        None => serde_json::Value::Null,
    };

    Ok(json!({
        RES_PHOTOMETRY: serde_json::to_value(&phot)?,
        RES_SKY: sky,
        RES_GAIA: gaia,
        RES_PHOTCAL: photcal_value,
        RES_WARNINGS: warnings,
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        RES_MASKED: mask.is_some(),
    }))
}

#[tauri::command]
pub async fn measure_photometry_cmd(
    path: String,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    gaia_match: Option<bool>,
    exclude_dq: Option<bool>,
    gain: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        photometry_for_path(
            &path,
            x,
            y,
            aperture_radius,
            gaia_match.unwrap_or(true),
            exclude_dq.unwrap_or(false),
            gain,
        )
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
        let out = photometry_for_path(&key, 32.0, 32.0, None, false, true, None).unwrap();
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

        let unmasked = photometry_for_path(&key, 32.0, 32.0, None, false, false, None).unwrap();
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
        let out = photometry_for_path(&key, 32.0, 32.0, Some(5.0), false, false, None).unwrap();
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
        let out = photometry_for_path(plain.to_str().unwrap(), 32.0, 32.0, None, false, false, Some(2.0)).unwrap();
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
        let without = photometry_for_path(plain.to_str().unwrap(), 32.0, 32.0, None, false, false, None).unwrap();
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
        let out = photometry_for_path(path.to_str().unwrap(), 32.0, 32.0, None, false, false, None).unwrap();
        let phot = &out[RES_PHOTOMETRY];
        assert_eq!(phot["peak"], 1100.0);
        assert_eq!(phot["saturated"], true);
        assert_eq!(phot["saturation_source"], "SATURATE");
        assert_eq!(phot["n_saturated"], 0);

        let key = jwst_star_mef(&dir.path().join("dq.fits"), vec![("SATURATE", "1000".to_string())]);
        let with_dq = photometry_for_path(&key, 32.0, 32.0, None, false, false, None).unwrap();
        assert_eq!(with_dq[RES_PHOTOMETRY]["saturation_source"], "DQ SATURATED");
        assert_eq!(with_dq[RES_PHOTOMETRY]["n_saturated"], 1);
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
}
