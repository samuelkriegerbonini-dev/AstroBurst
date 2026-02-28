use std::time::Instant;

use anyhow::Result;

use crate::domain::cube::process_cube;
use crate::domain::lazy_cube::{process_cube_lazy, LazyCube};
use crate::utils::render::render_grayscale;

use super::helpers::*;

#[tauri::command]
pub async fn process_cube_cmd(
    path: String,
    output_dir: String,
    frame_step: Option<usize>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let step = frame_step.unwrap_or(5);
        let (fits_path, _tmp) = resolve_fits(&path)?;
        let fits_str = fits_path.to_string_lossy().to_string();

        let cube_result = process_cube(&fits_str, &output_dir, step)?;
        let elapsed = start.elapsed().as_millis() as u64;

        let wavelengths: serde_json::Value = match cube_result.wavelengths {
            Some(w) => serde_json::json!(w),
            None => serde_json::Value::Null,
        };

        Ok(serde_json::json!({
            "dimensions": cube_result.dimensions,
            "collapsed_path": cube_result.collapsed_path,
            "collapsed_median_path": cube_result.collapsed_median_path,
            "frames_dir": cube_result.frames_dir,
            "frame_count": cube_result.frame_count,
            "center_spectrum": cube_result.center_spectrum,
            "wavelengths": wavelengths,
            "elapsed_ms": elapsed
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn process_cube_lazy_cmd(
    path: String,
    output_dir: String,
    frame_step: Option<usize>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let step = frame_step.unwrap_or(5);
        let (fits_path, _tmp) = resolve_fits(&path)?;
        let fits_str = fits_path.to_string_lossy().to_string();

        let cube_result = process_cube_lazy(&fits_str, &output_dir, step)?;
        let elapsed = start.elapsed().as_millis() as u64;

        let wavelengths: serde_json::Value = match cube_result.wavelengths {
            Some(w) => serde_json::json!(w),
            None => serde_json::Value::Null,
        };

        Ok(serde_json::json!({
            "dimensions": cube_result.dimensions,
            "collapsed_path": cube_result.collapsed_path,
            "collapsed_median_path": cube_result.collapsed_median_path,
            "frames_dir": cube_result.frames_dir,
            "frame_count": cube_result.frame_count,
            "total_frames": cube_result.total_frames,
            "center_spectrum": cube_result.center_spectrum,
            "wavelengths": wavelengths,
            "elapsed_ms": elapsed
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_cube_info(path: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (fits_path, _tmp) = resolve_fits(&path)?;
        let fits_str = fits_path.to_string_lossy().to_string();
        let lazy = LazyCube::open(&fits_str)?;
        let g = &lazy.geometry;
        let wavelengths = crate::domain::cube::build_wavelength_axis(&lazy.header);

        Ok(serde_json::json!({
            "naxis1": g.naxis1,
            "naxis2": g.naxis2,
            "naxis3": g.naxis3,
            "bitpix": g.bitpix,
            "bytes_per_pixel": g.bytes_per_pixel,
            "frame_bytes": g.frame_bytes,
            "total_data_bytes": g.frame_bytes * g.naxis3,
            "wavelengths": wavelengths.map(|w| serde_json::json!(w)).unwrap_or(serde_json::Value::Null),
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_cube_frame(
    path: String,
    frame_index: usize,
    output_path: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (fits_path, _tmp) = resolve_fits(&path)?;
        let fits_str = fits_path.to_string_lossy().to_string();

        let lazy = LazyCube::open(&fits_str)?;
        let frame = lazy.get_frame(frame_index)?;
        let stats = lazy.compute_global_stats_streaming()?;
        let normalized = crate::domain::lazy_cube::normalize_frame_with_stats(&frame, &stats);

        render_grayscale(&normalized, &output_path)?;
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "frame_index": frame_index,
            "output_path": output_path,
            "elapsed_ms": elapsed
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_cube_spectrum(
    path: String,
    x: usize,
    y: usize,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (fits_path, _tmp) = resolve_fits(&path)?;
        let fits_str = fits_path.to_string_lossy().to_string();

        let lazy = LazyCube::open(&fits_str)?;
        let spectrum = lazy.extract_spectrum_at(y, x)?;
        let wavelengths = crate::domain::cube::build_wavelength_axis(&lazy.header);
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "x": x,
            "y": y,
            "spectrum": spectrum,
            "wavelengths": wavelengths.map(|w| serde_json::json!(w)).unwrap_or(serde_json::Value::Null),
            "elapsed_ms": elapsed
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}
