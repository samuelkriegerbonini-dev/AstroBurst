use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::domain::normalize::asinh_normalize;
use crate::domain::stf::{self, StfParams};
use crate::domain::stats;
use crate::domain::fits_writer::{self, FitsWriteConfig};
use crate::utils::ipc;
use crate::utils::render::render_grayscale;

use super::helpers::*;

#[tauri::command]
pub async fn process_fits(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (header, arr, _tmp) = extract_image_resolved(&path)?;
        let dims = arr.dim();
        let normalized = asinh_normalize(&arr);

        let stem = Path::new(&path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let out_dir = resolve_output_dir(&output_dir)?;
        let png_path = out_dir.join(format!("{}.png", stem));
        render_grayscale(&normalized, png_path.to_str().unwrap())?;

        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "png_path": png_path.to_string_lossy(),
            "dimensions": [dims.1, dims.0],
            "elapsed_ms": elapsed
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn process_batch(
    paths: Vec<String>,
    output_dir: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let out = resolve_output_dir(&output_dir)?;

        let results: Vec<serde_json::Value> = paths
            .par_iter()
            .map(|path| {
                let file_start = Instant::now();

                let process = || -> Result<(String, [usize; 2], u64)> {
                    let (_, arr, _tmp) = extract_image_resolved(path)?;
                    let dims = arr.dim();
                    let normalized = asinh_normalize(&arr);

                    let stem = Path::new(path)
                        .file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .to_string();

                    let png_path = out.join(format!("{}.png", stem));
                    render_grayscale(&normalized, png_path.to_str().unwrap())?;

                    Ok((
                        png_path.to_string_lossy().to_string(),
                        [dims.1, dims.0],
                        file_start.elapsed().as_millis() as u64,
                    ))
                };

                match process() {
                    Ok((png_path, dims, elapsed)) => serde_json::json!({
                        "path": path,
                        "png_path": png_path,
                        "dimensions": dims,
                        "elapsed_ms": elapsed,
                        "status": "done"
                    }),
                    Err(e) => serde_json::json!({
                        "path": path,
                        "status": "error",
                        "error": format!("{:#}", e)
                    }),
                }
            })
            .collect();

        let processed = results.iter().filter(|r| r["status"] == "done").count();
        let failed = results.iter().filter(|r| r["status"] == "error").count();
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "processed": processed,
            "failed": failed,
            "elapsed_ms": elapsed,
            "results": results
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_raw_pixels(path: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (_, arr, _tmp) = extract_image_resolved(&path)?;
        let (rows, cols) = arr.dim();
        let slice = arr.as_slice().context("contiguous")?;

        let mut finite_vals: Vec<f32> = slice.iter().filter(|v| v.is_finite()).copied().collect();
        let n = finite_vals.len();
        let (data_min, data_max) = if n > 0 {
            finite_vals
                .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            (finite_vals[0] as f64, finite_vals[n - 1] as f64)
        } else {
            (0.0, 1.0)
        };

        let byte_len = slice.len() * 4;
        let mut bytes = Vec::with_capacity(byte_len);
        for &v in slice {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "width": cols,
            "height": rows,
            "data_b64": b64,
            "data_min": data_min,
            "data_max": data_max,
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub fn get_raw_pixels_binary(path: String) -> std::result::Result<tauri::ipc::Response, String> {
    let (_, arr, _tmp) = extract_image_resolved(&path).map_err(map_anyhow)?;
    let data = ipc::encode_with_header(&arr).map_err(map_anyhow)?;
    Ok(tauri::ipc::Response::new(data))
}

#[tauri::command]
pub async fn export_fits(
    path: String,
    output_path: String,
    apply_stf: Option<bool>,
    shadow: Option<f64>,
    midtone: Option<f64>,
    highlight: Option<f64>,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();
        let (header, arr, _tmp) = extract_image_resolved(&path)?;
        let (rows, cols) = arr.dim();

        let output_image = if apply_stf.unwrap_or(false) {
            let params = StfParams {
                shadow: shadow.unwrap_or(0.0),
                midtone: midtone.unwrap_or(0.5),
                highlight: highlight.unwrap_or(1.0),
            };
            let st = stats::compute_image_stats(&arr);
            stf::apply_stf_f32(&arr, &params, &st)
        } else {
            arr
        };

        let config = FitsWriteConfig {
            copy_wcs: copy_wcs.unwrap_or(true),
            copy_obs_metadata: copy_metadata.unwrap_or(true),
            software: Some("AstroKit".into()),
            ..Default::default()
        };

        let written_path =
            fits_writer::write_fits_image(&output_image, &output_path, Some(&header), &config)?;

        let file_size = std::fs::metadata(&written_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "output_path": written_path,
            "dimensions": [cols, rows],
            "bitpix": -32,
            "file_size_bytes": file_size,
            "stf_applied": apply_stf.unwrap_or(false),
            "wcs_copied": copy_wcs.unwrap_or(true),
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn export_fits_rgb(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_path: String,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();

        let r_data = r_path
            .as_ref()
            .map(|p| extract_image_resolved(p))
            .transpose()?;
        let g_data = g_path
            .as_ref()
            .map(|p| extract_image_resolved(p))
            .transpose()?;
        let b_data = b_path
            .as_ref()
            .map(|p| extract_image_resolved(p))
            .transpose()?;

        let present = [r_data.is_some(), g_data.is_some(), b_data.is_some()];
        let count = present.iter().filter(|&&b| b).count();
        if count < 2 {
            anyhow::bail!("Need at least 2 channels for RGB FITS export");
        }

        let ref_dim = r_data
            .as_ref()
            .or(g_data.as_ref())
            .or(b_data.as_ref())
            .unwrap()
            .1
            .dim();

        let zeros = ndarray::Array2::zeros(ref_dim);

        let r_arr = r_data
            .as_ref()
            .map(|(_, a, _)| a.clone())
            .unwrap_or_else(|| zeros.clone());
        let g_arr = g_data
            .as_ref()
            .map(|(_, a, _)| a.clone())
            .unwrap_or_else(|| zeros.clone());
        let b_arr = b_data
            .as_ref()
            .map(|(_, a, _)| a.clone())
            .unwrap_or_else(|| zeros.clone());

        let source_header = r_data
            .as_ref()
            .or(g_data.as_ref())
            .or(b_data.as_ref())
            .map(|(h, _, _)| h);

        let config = FitsWriteConfig {
            copy_wcs: copy_wcs.unwrap_or(true),
            copy_obs_metadata: copy_metadata.unwrap_or(true),
            software: Some("AstroKit".into()),
            ..Default::default()
        };

        let written_path = fits_writer::write_fits_rgb(
            &r_arr,
            &g_arr,
            &b_arr,
            &output_path,
            source_header.as_ref(),
            &config,
        )?;

        let file_size = std::fs::metadata(&written_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "output_path": written_path,
            "dimensions": [ref_dim.1, ref_dim.0, 3],
            "bitpix": -32,
            "file_size_bytes": file_size,
            "elapsed_ms": elapsed,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}
