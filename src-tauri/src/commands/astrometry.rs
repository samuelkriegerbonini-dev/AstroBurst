use std::fs::File;
use std::time::Instant;

use anyhow::Result;

use crate::domain::config_manager;
use crate::domain::plate_solve::{self, SolveConfig};
use crate::utils::mmap::extract_image_mmap;

use super::helpers::*;

#[tauri::command]
pub async fn pixel_to_world(path: String, x: f64, y: f64) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (header, _, _tmp) = extract_image_resolved(&path)?;
        let wcs = crate::domain::wcs::WcsTransform::from_header(&header)?;

        let coord = wcs.pixel_to_world(x, y);
        let scale = wcs.pixel_scale_arcsec();

        Ok(serde_json::json!({
            "ra": coord.ra,
            "dec": coord.dec,
            "ra_dec_str": format!("{}", coord),
            "pixel_scale_arcsec": scale,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn world_to_pixel(path: String, ra: f64, dec: f64) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (header, _, _tmp) = extract_image_resolved(&path)?;
        let wcs = crate::domain::wcs::WcsTransform::from_header(&header)?;

        let (px, py) = wcs.world_to_pixel(ra, dec);
        Ok(serde_json::json!({ "x": px, "y": py }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_wcs_info(path: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (header, _, _tmp) = extract_image_resolved(&path)?;
        let wcs = crate::domain::wcs::WcsTransform::from_header(&header)?;

        let naxis1 = header.get_i64("NAXIS1").unwrap_or(0) as usize;
        let naxis2 = header.get_i64("NAXIS2").unwrap_or(0) as usize;

        let center = wcs.pixel_to_world(naxis1 as f64 / 2.0, naxis2 as f64 / 2.0);
        let (fov_x, fov_y) = wcs.field_of_view(naxis1, naxis2);

        let corners = wcs.pixel_to_world_batch(&[
            (0.0, 0.0),
            (naxis1 as f64, 0.0),
            (naxis1 as f64, naxis2 as f64),
            (0.0, naxis2 as f64),
        ]);

        Ok(serde_json::json!({
            "center_ra": center.ra,
            "center_dec": center.dec,
            "center_str": format!("{}", center),
            "pixel_scale_arcsec": wcs.pixel_scale_arcsec(),
            "fov_arcmin": [fov_x, fov_y],
            "corners": corners.iter().map(|c| serde_json::json!({"ra": c.ra, "dec": c.dec})).collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[cfg(feature = "astrometry-net")]
#[tauri::command]
pub async fn plate_solve_cmd(
    path: String,
    sigma: Option<f64>,
    max_stars: Option<usize>,
    ra_hint: Option<f64>,
    dec_hint: Option<f64>,
    radius_hint: Option<f64>,
    scale_low: Option<f64>,
    scale_high: Option<f64>,
) -> Result<serde_json::Value, String> {
    let api_key = config_manager::get_api_key()
        .ok_or_else(|| "No API key configured. Use save_api_key first.".to_string())?;

    let cfg = config_manager::load_config();

    let (detection, image_width, image_height, resolved_path) =
        tokio::task::spawn_blocking(move || -> Result<(plate_solve::DetectionResult, usize, usize, String)> {
            let (fits_path, _tmp) = resolve_fits(&path)?;
            let fits_str = fits_path.to_string_lossy().to_string();
            let file = File::open(&fits_path)?;
            let mmap_result = extract_image_mmap(&file)?;
            let sigma_thresh = sigma.unwrap_or(5.0);
            let mut det = plate_solve::detect_stars(&mmap_result.image, sigma_thresh);
            let limit = max_stars.unwrap_or(cfg.plate_solve_max_stars);
            if det.stars.len() > limit {
                det.stars.truncate(limit);
            }
            let w = det.image_width;
            let h = det.image_height;
            Ok((det, w, h, fits_str))
        })
        .await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(map_anyhow)?;

    if detection.stars.is_empty() {
        return Err("No stars detected — cannot plate solve".into());
    }

    let solve_config = SolveConfig {
        api_url: cfg.astrometry_api_url.clone(),
        api_key,
        ra_hint,
        dec_hint,
        radius_hint: radius_hint.or(Some(10.0)),
        scale_low,
        scale_high,
        max_stars: Some(cfg.plate_solve_max_stars),
    };

    let solve_result = plate_solve::solve_astrometry_net(
        &resolved_path,
        &detection.stars,
        image_width,
        image_height,
        &solve_config,
    )
    .await
    .map_err(map_anyhow)?;

    Ok(serde_json::json!({
        "success": solve_result.success,
        "ra_center": solve_result.ra_center,
        "dec_center": solve_result.dec_center,
        "orientation": solve_result.orientation,
        "pixel_scale": solve_result.pixel_scale,
        "field_w_arcmin": solve_result.field_w_arcmin,
        "field_h_arcmin": solve_result.field_h_arcmin,
        "stars_detected": detection.stars.len(),
        "stars_used": solve_result.stars_used,
        "wcs_headers": solve_result.wcs_headers,
        "wcs_matrix": {
            "cd1_1": solve_result.wcs_headers.get("CD1_1").and_then(|v| v.parse::<f64>().ok()),
            "cd1_2": solve_result.wcs_headers.get("CD1_2").and_then(|v| v.parse::<f64>().ok()),
            "cd2_1": solve_result.wcs_headers.get("CD2_1").and_then(|v| v.parse::<f64>().ok()),
            "cd2_2": solve_result.wcs_headers.get("CD2_2").and_then(|v| v.parse::<f64>().ok()),
            "crpix1": solve_result.wcs_headers.get("CRPIX1").and_then(|v| v.parse::<f64>().ok()),
            "crpix2": solve_result.wcs_headers.get("CRPIX2").and_then(|v| v.parse::<f64>().ok()),
            "crval1": solve_result.wcs_headers.get("CRVAL1").and_then(|v| v.parse::<f64>().ok()),
            "crval2": solve_result.wcs_headers.get("CRVAL2").and_then(|v| v.parse::<f64>().ok()),
        },
    }))
}

#[cfg(not(feature = "astrometry-net"))]
#[tauri::command]
pub async fn plate_solve_cmd(
    _path: String,
    _sigma: Option<f64>,
    _max_stars: Option<usize>,
    _ra_hint: Option<f64>,
    _dec_hint: Option<f64>,
    _radius_hint: Option<f64>,
    _scale_low: Option<f64>,
    _scale_high: Option<f64>,
) -> Result<serde_json::Value, String> {
    Err("Plate solving requires the 'astrometry-net' feature. Rebuild with: cargo build --features astrometry-net".into())
}
