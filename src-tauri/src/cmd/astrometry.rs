use std::fs::File;

use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
use crate::infra::config;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::{extract_header_mmap, extract_image_mmap};
use crate::types::constants::{
    DEFAULT_API_KEY_SERVICE, DEFAULT_ASTROMETRY_API_URL, HEADER_NAXIS1,
    HEADER_NAXIS2, RES_CENTER_DEC, RES_CENTER_RA, RES_FOV_ARCMIN,
    RES_FOV_H_ARCMIN, RES_FOV_W_ARCMIN, RES_NAXIS1, RES_NAXIS2,
    RES_PIXEL_SCALE_ARCSEC,
};

const MAX_UPLOAD_DIM: usize = 2048;

fn load_header_and_wcs(path: &str) -> anyhow::Result<(crate::types::header::HduHeader, WcsTransform)> {
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let header = extract_header_mmap(&file)?;
    let wcs = WcsTransform::from_header(&header)?;
    Ok((header, wcs))
}

fn load_wcs_only(path: &str) -> anyhow::Result<WcsTransform> {
    let (fits_path, _tmp) = resolve_single_image(path)?;
    let file = File::open(&fits_path)?;
    let header = extract_header_mmap(&file)?;
    WcsTransform::from_header(&header)
}

type WcsStamp = (u64, Option<std::time::SystemTime>);

static WCS_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, (WcsStamp, std::sync::Arc<WcsTransform>)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn load_wcs_cached(path: &str) -> anyhow::Result<std::sync::Arc<WcsTransform>> {
    let stamp: Option<WcsStamp> = std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()));

    if let Some(st) = &stamp {
        let cache = WCS_CACHE.lock().unwrap();
        if let Some((cached_stamp, wcs)) = cache.get(path) {
            if cached_stamp == st {
                return Ok(std::sync::Arc::clone(wcs));
            }
        }
    }

    let wcs = std::sync::Arc::new(load_wcs_only(path)?);

    if let Some(st) = stamp {
        let mut cache = WCS_CACHE.lock().unwrap();
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(path.to_string(), (st, std::sync::Arc::clone(&wcs)));
    }

    Ok(wcs)
}

fn resolve_api_key(provided: Option<String>) -> Option<String> {
    if let Some(ref k) = provided {
        if !k.is_empty() {
            return provided;
        }
    }
    config::load_api_key(DEFAULT_API_KEY_SERVICE).ok().flatten()
}

#[tauri::command]
pub async fn plate_solve_cmd(
    path: String,
    api_key: Option<String>,
    scale_lower: Option<f64>,
    scale_upper: Option<f64>,
    scale_units: Option<String>,
    downsample_factor: Option<u32>,
    center_ra: Option<f64>,
    center_dec: Option<f64>,
    radius: Option<f64>,
) -> Result<serde_json::Value, String> {
    let (upload_path, _tmp, _tmp_ds, stars, width, height, ds_factor, cfg) = tokio::task::spawn_blocking(
        move || -> anyhow::Result<_> {
            let resolved_key = resolve_api_key(api_key);

            let (resolved_path, tmp) = resolve_single_image(&path)?;
            let file = File::open(&resolved_path)?;
            let result = extract_image_mmap(&file)?;

            let naxis1 = result.header.get_i64(HEADER_NAXIS1).unwrap_or(0) as usize;
            let naxis2 = result.header.get_i64(HEADER_NAXIS2).unwrap_or(0) as usize;

            let detection = crate::core::analysis::star_detection::detect_stars(
                &result.image,
                5.0,
            );

            let target_dims: Option<(usize, usize)> = if let Some(f) = downsample_factor.filter(|&f| f > 1) {
                let ds_cols = (naxis1 / f as usize).max(1);
                let ds_rows = (naxis2 / f as usize).max(1);
                Some((ds_rows, ds_cols))
            } else if naxis1 > MAX_UPLOAD_DIM || naxis2 > MAX_UPLOAD_DIM {
                let scale = MAX_UPLOAD_DIM as f64 / naxis1.max(naxis2) as f64;
                Some((
                    (naxis2 as f64 * scale).round() as usize,
                    (naxis1 as f64 * scale).round() as usize,
                ))
            } else {
                None
            };

            let (upload_fits, tmp_ds, ds_factor) = if let Some((ds_rows, ds_cols)) = target_dims {
                log::info!(
                    "Plate solve: downsampling {}x{} to {}x{} for upload",
                    naxis1, naxis2, ds_cols, ds_rows
                );

                let downsampled = crate::core::alignment::downsample::area_downsample(
                    &result.image, ds_rows, ds_cols,
                );

                let tmp_file = tempfile::Builder::new()
                    .suffix(".fits")
                    .tempfile()?;
                let tmp_path = tmp_file.path().to_string_lossy().to_string();

                crate::infra::fits::writer::write_fits_mono(
                    &tmp_path,
                    &downsampled,
                    Some(&result.header),
                )?;

                let fx = naxis1 as f64 / ds_cols as f64;
                let fy = naxis2 as f64 / ds_rows as f64;
                (tmp_path, Some(tmp_file), Some((fx, fy)))
            } else {
                (resolved_path.to_string_lossy().to_string(), None, None)
            };

            let is_pixel_scale = scale_units.as_deref().map_or(true, |u| u.contains("pix"));
            let hint_scale = if is_pixel_scale {
                ds_factor.map_or(1.0, |(fx, fy)| (fx * fy).sqrt())
            } else {
                1.0
            };

            let cfg = crate::infra::astrometry::plate_solve::SolveConfig {
                api_url: config::load_config()
                    .map(|c| c.astrometry_api_url)
                    .unwrap_or_else(|_| DEFAULT_ASTROMETRY_API_URL.into()),
                api_key: resolved_key.unwrap_or_default(),
                ra_hint: center_ra,
                dec_hint: center_dec,
                radius_hint: radius,
                scale_low: scale_lower.map(|v| v * hint_scale),
                scale_high: scale_upper.map(|v| v * hint_scale),
                scale_units,
                max_stars: config::load_config()
                    .map(|c| Some(c.plate_solve_max_stars))
                    .unwrap_or(Some(100)),
            };

            Ok((upload_fits, tmp, tmp_ds, detection.stars, naxis1, naxis2, ds_factor, cfg))
        },
    )
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e: anyhow::Error| e.to_string())?;

    #[cfg(feature = "astrometry-net")]
    {
        let mut solve_result = crate::infra::astrometry::plate_solve::solve_astrometry_net(
            &upload_path, &stars, width, height, &cfg,
        )
            .await
            .map_err(|e| e.to_string())?;

        if let Some((fx, fy)) = ds_factor {
            rescale_solve_to_original(&mut solve_result, fx, fy, width, height);
        }

        drop((_tmp, _tmp_ds));
        return serde_json::to_value(&solve_result).map_err(|e| e.to_string());
    }

    #[cfg(not(feature = "astrometry-net"))]
    {
        drop((_tmp, _tmp_ds, upload_path, stars, width, height, ds_factor, cfg));
        let result = crate::infra::astrometry::plate_solve::solve_offline_placeholder()
            .map_err(|e| e.to_string())?;
        serde_json::to_value(&result).map_err(|e| e.to_string())
    }
}

#[cfg(feature = "astrometry-net")]
fn rescale_solve_to_original(
    result: &mut crate::infra::astrometry::plate_solve::SolveResult,
    fx: f64,
    fy: f64,
    width: usize,
    height: usize,
) {
    let f_mean = (fx * fy).sqrt();
    result.pixel_scale /= f_mean;
    result.field_w_arcmin = result.pixel_scale * width as f64 / 60.0;
    result.field_h_arcmin = result.pixel_scale * height as f64 / 60.0;

    result
        .wcs_headers
        .retain(|k, _| !["A_", "B_", "AP_", "BP_"].iter().any(|p| k.starts_with(p)));

    let mut updates: Vec<(String, String)> = Vec::new();
    for (key, value) in &result.wcs_headers {
        match key.as_str() {
            "CRPIX1" | "CRPIX2" | "CD1_1" | "CD1_2" | "CD2_1" | "CD2_2" | "CDELT1" | "CDELT2" => {
                if let Ok(num) = value.trim().parse::<f64>() {
                    let scaled = match key.as_str() {
                        "CRPIX1" => fx * (num - 0.5) + 0.5,
                        "CRPIX2" => fy * (num - 0.5) + 0.5,
                        "CD1_1" | "CD2_1" | "CDELT1" => num / fx,
                        _ => num / fy,
                    };
                    updates.push((key.clone(), format!("{:.12E}", scaled)));
                }
            }
            "IMAGEW" => updates.push((key.clone(), width.to_string())),
            "IMAGEH" => updates.push((key.clone(), height.to_string())),
            "CTYPE1" | "CTYPE2" => {
                if value.contains("-SIP") {
                    updates.push((key.clone(), value.replace("-SIP", "")));
                }
            }
            _ => {}
        }
    }
    for (key, value) in updates {
        result.wcs_headers.insert(key, value);
    }

    for ann in &mut result.annotations {
        ann.pixelx = fx * (ann.pixelx - 0.5) + 0.5;
        ann.pixely = fy * (ann.pixely - 0.5) + 0.5;
        if let Some(r) = ann.radius.as_mut() {
            *r *= f_mean;
        }
    }
}

#[tauri::command]
pub async fn get_wcs_info(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (header, wcs) = load_header_and_wcs(&path)?;
        let naxis1 = header.get_i64(HEADER_NAXIS1).unwrap_or(0) as usize;
        let naxis2 = header.get_i64(HEADER_NAXIS2).unwrap_or(0) as usize;
        let pixel_scale = wcs.pixel_scale_arcsec();
        let (fov_w, fov_h) = wcs.field_of_view(naxis1, naxis2);
        let center = wcs.pixel_to_world(naxis1 as f64 / 2.0, naxis2 as f64 / 2.0);

        // Per-pixel wcs_params (crpix/crval/cd/projection/sip) used to be emitted
        // here so the frontend could do its own pix->sky math client-side
        // (src/utils/wcstransform.ts). That TS twin is retired in favor of the
        // `pixel_to_world_cmd` IPC command, which drives the real engine (full
        // projection coverage, not just TAN/SIN/ARC/CAR) -- so this endpoint now
        // only reports the static summary fields.
        Ok(json!({
            RES_CENTER_RA: center.ra,
            RES_CENTER_DEC: center.dec,
            RES_PIXEL_SCALE_ARCSEC: pixel_scale,
            RES_FOV_W_ARCMIN: fov_w,
            RES_FOV_H_ARCMIN: fov_h,
            RES_FOV_ARCMIN: [fov_w, fov_h],
            RES_NAXIS1: naxis1,
            RES_NAXIS2: naxis2,
        }))
    })
}

/// Batched pixel->sky conversion for the frontend cursor RA/Dec readout, backing
/// the full projection coverage wcs-rs gives us (the old readout did its own
/// client-side math in `src/utils/wcstransform.ts`, limited to TAN/SIN/ARC/CAR).
/// `points` are 0-based image-array pixel coordinates. Each result entry is
/// `[ra, dec]` in degrees, or `null` if that point has no valid sky position
/// (e.g. a singular CD matrix) -- NaN cannot round-trip through JSON.
#[tauri::command]
pub async fn pixel_to_world_cmd(
    path: String,
    points: Vec<(f64, f64)>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let wcs = load_wcs_cached(&path)?;
        let coords = wcs.pixel_to_world_batch(&points);
        let out: Vec<serde_json::Value> = coords
            .into_iter()
            .map(|c| {
                if c.ra.is_finite() && c.dec.is_finite() {
                    json!([c.ra, c.dec])
                } else {
                    serde_json::Value::Null
                }
            })
            .collect();
        Ok(json!({ "points": out }))
    })
}

struct SkyFootprint {
    center_ra: f64,
    center_dec: f64,
    corners: [(f64, f64); 4],
    fov_w_arcmin: f64,
    fov_h_arcmin: f64,
}

fn sky_footprint(path: &str) -> anyhow::Result<SkyFootprint> {
    let (header, wcs) = load_header_and_wcs(path)?;
    let n1 = header.get_i64(HEADER_NAXIS1).unwrap_or(0);
    let n2 = header.get_i64(HEADER_NAXIS2).unwrap_or(0);
    if n1 <= 0 || n2 <= 0 {
        anyhow::bail!("Missing NAXIS1/NAXIS2 in header");
    }
    let (w, h) = (n1 as f64, n2 as f64);
    let pts = [
        (0.0, 0.0),
        (w - 1.0, 0.0),
        (w - 1.0, h - 1.0),
        (0.0, h - 1.0),
    ];
    let mut corners = [(0.0f64, 0.0f64); 4];
    for (i, &(x, y)) in pts.iter().enumerate() {
        let c = wcs.pixel_to_world(x, y);
        if !c.ra.is_finite() || !c.dec.is_finite() {
            anyhow::bail!("Image corner does not project to a valid sky position");
        }
        corners[i] = (c.ra, c.dec);
    }
    let center = wcs.pixel_to_world(w / 2.0, h / 2.0);
    if !center.ra.is_finite() || !center.dec.is_finite() {
        anyhow::bail!("Image center does not project to a valid sky position");
    }
    let (fov_w, fov_h) = wcs.field_of_view(n1 as usize, n2 as usize);
    Ok(SkyFootprint {
        center_ra: center.ra,
        center_dec: center.dec,
        corners,
        fov_w_arcmin: fov_w,
        fov_h_arcmin: fov_h,
    })
}

fn wrap180(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

fn footprint_overlap_fraction(a: &SkyFootprint, b: &SkyFootprint) -> f64 {
    let ra0 = a.center_ra;
    let dec0 = a.center_dec;
    let cosd = dec0.to_radians().cos().max(1e-6);

    let bbox = |fp: &SkyFootprint| {
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for &(ra, dec) in &fp.corners {
            let x = wrap180(ra - ra0) * cosd;
            let y = dec - dec0;
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            ymin = ymin.min(y);
            ymax = ymax.max(y);
        }
        (xmin, xmax, ymin, ymax)
    };

    let (ax1, ax2, ay1, ay2) = bbox(a);
    let (bx1, bx2, by1, by2) = bbox(b);

    let ix = (ax2.min(bx2) - ax1.max(bx1)).max(0.0);
    let iy = (ay2.min(by2) - ay1.max(by1)).max(0.0);
    let area_a = (ax2 - ax1) * (ay2 - ay1);
    let area_b = (bx2 - bx1) * (by2 - by1);
    let min_area = area_a.min(area_b);
    if min_area <= 0.0 {
        return 0.0;
    }
    (ix * iy) / min_area
}

#[tauri::command]
pub async fn check_pointing_overlap_cmd(
    paths: Vec<String>,
    threshold: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let threshold = threshold.unwrap_or(0.05).clamp(0.0, 1.0);

        let footprints: Vec<(String, Result<SkyFootprint, String>)> = paths
            .iter()
            .map(|p| (p.clone(), sky_footprint(p).map_err(|e| format!("{:#}", e))))
            .collect();

        let files: Vec<serde_json::Value> = footprints
            .iter()
            .map(|(p, r)| match r {
                Ok(fp) => json!({
                    "path": p,
                    "has_wcs": true,
                    "center_ra": fp.center_ra,
                    "center_dec": fp.center_dec,
                    "fov_w_arcmin": fp.fov_w_arcmin,
                    "fov_h_arcmin": fp.fov_h_arcmin,
                }),
                Err(e) => json!({ "path": p, "has_wcs": false, "error": e }),
            })
            .collect();

        let mut pairs = Vec::new();
        let mut any_disjoint = false;
        for i in 0..footprints.len() {
            for j in (i + 1)..footprints.len() {
                match (&footprints[i].1, &footprints[j].1) {
                    (Ok(fa), Ok(fb)) => {
                        let fraction = footprint_overlap_fraction(fa, fb);
                        if fraction < threshold {
                            any_disjoint = true;
                            let sep = angular_separation(
                                fa.center_ra, fa.center_dec,
                                fb.center_ra, fb.center_dec,
                            ) * 60.0;
                            pairs.push(json!({
                                "a": footprints[i].0,
                                "b": footprints[j].0,
                                "status": "disjoint",
                                "fraction": fraction,
                                "separation_arcmin": sep,
                            }));
                        }
                    }
                    _ => {
                        pairs.push(json!({
                            "a": footprints[i].0,
                            "b": footprints[j].0,
                            "status": "unknown",
                            "fraction": serde_json::Value::Null,
                            "separation_arcmin": serde_json::Value::Null,
                        }));
                    }
                }
            }
        }

        Ok(json!({
            "files": files,
            "pairs": pairs,
            "any_disjoint": any_disjoint,
            "threshold": threshold,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::{footprint_overlap_fraction, wrap180, SkyFootprint};

    fn footprint(ra: f64, dec: f64, size_deg: f64) -> SkyFootprint {
        let half = size_deg / 2.0;
        let cosd = dec.to_radians().cos().max(1e-6);
        let dra = half / cosd;
        SkyFootprint {
            center_ra: ra,
            center_dec: dec,
            corners: [
                (ra - dra, dec - half),
                (ra + dra, dec - half),
                (ra + dra, dec + half),
                (ra - dra, dec + half),
            ],
            fov_w_arcmin: size_deg * 60.0,
            fov_h_arcmin: size_deg * 60.0,
        }
    }

    #[test]
    fn identical_footprints_fully_overlap() {
        let a = footprint(10.0, 5.0, 0.1);
        let b = footprint(10.0, 5.0, 0.1);
        assert!((footprint_overlap_fraction(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn disjoint_pointings_have_zero_overlap() {
        let a = footprint(6.95, 1.65, 0.04);
        let b = footprint(7.40, 1.81, 0.04);
        assert_eq!(footprint_overlap_fraction(&a, &b), 0.0);
    }

    #[test]
    fn ra_wraparound_is_handled() {
        let a = footprint(359.95, 0.0, 0.2);
        let b = footprint(0.05, 0.0, 0.2);
        let f = footprint_overlap_fraction(&a, &b);
        assert!(f > 0.4, "expected overlap across RA=0, got {}", f);
        assert!((wrap180(359.8) + 0.2).abs() < 1e-9);
    }

    #[test]
    fn high_declination_neighbors_overlap_partially() {
        let a = footprint(100.0, 80.0, 0.2);
        let b = footprint(100.0 + 0.1 / 80.0f64.to_radians().cos(), 80.0, 0.2);
        let f = footprint_overlap_fraction(&a, &b);
        assert!(f > 0.3 && f < 0.7, "expected partial overlap, got {}", f);
    }
}
