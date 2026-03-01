use std::time::Instant;

use anyhow::Result;

use crate::domain::fft::compute_power_spectrum;
use crate::domain::plate_solve;
use crate::domain::stf::{self, AutoStfConfig};

use super::helpers::*;

#[tauri::command]
pub async fn compute_histogram(path: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (_, arr, _tmp) = extract_image_resolved(&path)?;
        let (st, hist) = stf::analyze(&arr);
        let auto_params = stf::auto_stf(&st, &AutoStfConfig::default());
        let bins_512 = stf::downsample_histogram(&hist, 512);
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "bins": bins_512,
            "bin_count": 512,
            "data_min": hist.data_min,
            "data_max": hist.data_max,
            "median": st.median,
            "mean": st.mean,
            "sigma": st.sigma,
            "mad": st.mad,
            "total_pixels": hist.total_pixels,
            "auto_stf": {
                "shadow": auto_params.shadow,
                "midtone": auto_params.midtone,
                "highlight": auto_params.highlight,
            },
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn compute_fft_spectrum(path: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (_, arr, _tmp) = extract_image_resolved(&path)?;
        let fft_result = compute_power_spectrum(&arr);

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&fft_result.pixels);
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "width": fft_result.width,
            "height": fft_result.height,
            "pixels_b64": b64,
            "dc_magnitude": fft_result.dc_magnitude,
            "max_magnitude": fft_result.max_magnitude,
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn detect_stars(
    path: String,
    sigma: Option<f64>,
    max_stars: Option<usize>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (_, arr, _tmp) = extract_image_resolved(&path)?;
        let sigma_thresh = sigma.unwrap_or(5.0);
        let mut detection = plate_solve::detect_stars(&arr, sigma_thresh);

        let limit = max_stars.unwrap_or(500);
        if detection.stars.len() > limit {
            detection.stars.truncate(limit);
        }

        let elapsed = start.elapsed().as_millis() as u64;

        let star_json: Vec<serde_json::Value> = detection
            .stars
            .iter()
            .map(|s| {
                serde_json::json!({
                    "x": s.x,
                    "y": s.y,
                    "flux": s.flux,
                    "fwhm": s.fwhm,
                    "peak": s.peak,
                    "npix": s.npix,
                    "snr": s.snr,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "stars": star_json,
            "count": detection.stars.len(),
            "background_median": detection.background_median,
            "background_sigma": detection.background_sigma,
            "threshold_sigma": detection.threshold_sigma,
            "image_width": detection.image_width,
            "image_height": detection.image_height,
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}
