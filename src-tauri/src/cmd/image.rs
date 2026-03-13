use std::time::Instant;

use rayon::prelude::*;
use serde_json::json;
use tauri::ipc::Response;

use crate::cmd::common::{blocking_cmd, extract_image_resolved, load_cached, load_cached_full, load_fits_array, render_and_save, resolve_output_dir, save_preview_png, MAX_PREVIEW_DIM};
use crate::core::imaging::stats::{downsample_histogram, compute_histogram_with_stats};
use crate::core::imaging::stf::{auto_stf, apply_stf, AutoStfConfig};
use crate::infra::ipc::{encode_with_header, encode_with_header_downsampled};
use crate::types::constants::{
    HISTOGRAM_BINS_DISPLAY,
    RES_AUTO_STF, RES_BINS, RES_BIN_COUNT, RES_BITPIX, RES_DATA_MAX, RES_DATA_MIN,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_ERROR, RES_HEADER, RES_HIGHLIGHT,
    RES_HISTOGRAM, RES_MAD, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIDTONE, RES_MIN,
    RES_OUTPUT_PATH, RES_PATH, RES_PNG_PATH, RES_RESULTS, RES_SHADOW, RES_SIGMA,
    RES_STATS, RES_STF, RES_TOTAL_PIXELS,
};

#[tauri::command]
pub async fn process_fits(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;
        let arr = cached.arr();
        let stats = cached.stats();
        let stf_params = auto_stf(stats, &AutoStfConfig::default());
        let rendered = apply_stf(arr, &stf_params, stats);

        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let png_path = format!("{}/{}.png", output_dir, stem);
        let (rows, cols) = arr.dim();
        save_preview_png(rendered, cols, rows, &png_path)?;

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: elapsed,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
                RES_MEDIAN: stats.median,
            },
            RES_STF: {
                RES_SHADOW: stf_params.shadow,
                RES_MIDTONE: stf_params.midtone,
                RES_HIGHLIGHT: stf_params.highlight,
            },
        }))
    })
}

#[tauri::command]
pub async fn process_fits_full(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let cached = load_cached_full(&path)?;
        let arr = cached.arr();
        let stats = cached.stats();
        let header = cached.header();

        let stf_params = auto_stf(stats, &AutoStfConfig::default());
        let rendered = apply_stf(arr, &stf_params, stats);

        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let png_path = format!("{}/{}.png", output_dir, stem);
        let (rows, cols) = arr.dim();
        save_preview_png(rendered, cols, rows, &png_path)?;

        let hist = compute_histogram_with_stats(arr, stats);
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);

        let header_json = match header {
            Some(h) => serde_json::to_value(&h.index)?,
            None => json!(null),
        };

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: elapsed,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
                RES_MEDIAN: stats.median,
                RES_MAD: stats.mad,
            },
            RES_STF: {
                RES_SHADOW: stf_params.shadow,
                RES_MIDTONE: stf_params.midtone,
                RES_HIGHLIGHT: stf_params.highlight,
            },
            RES_HEADER: header_json,
            RES_HISTOGRAM: {
                RES_BINS: display_bins,
                RES_BIN_COUNT: display_bins.len(),
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
            },
        }))
    })
}

#[tauri::command]
pub async fn process_batch(paths: Vec<String>, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let results: Vec<serde_json::Value> = paths.par_iter().map(|path| {
            match (|| -> anyhow::Result<serde_json::Value> {

                let ro = render_and_save(
                    load_cached(path)?.arr(),
                    path,
                    &output_dir,
                    "",
                    false,
                )?;
                let (rows, cols) = ro.dims;
                Ok(json!({
                    RES_PATH: path,
                    RES_PNG_PATH: ro.png_path,
                    RES_DIMENSIONS: [cols, rows],
                    RES_STATS: {
                        RES_MIN: ro.stats.min,
                        RES_MAX: ro.stats.max,
                        RES_MEAN: ro.stats.mean,
                        RES_SIGMA: ro.stats.sigma,
                    },
                }))
            })() {
                Ok(r) => r,
                Err(e) => json!({
                    RES_PATH: path,
                    RES_ERROR: format!("{:#}", e),
                }),
            }
        }).collect();

        Ok(json!({ RES_RESULTS: results }))
    })
}

#[tauri::command]
pub async fn get_raw_pixels_binary(path: String) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let data = encode_with_header(&extract_image_resolved(&path)?.arr)?;
        Ok(Response::new(data))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}

#[tauri::command]
pub async fn get_raw_pixels_preview(path: String, max_dim: Option<u32>) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let dim = max_dim.unwrap_or(2048) as usize;
        let data = encode_with_header_downsampled(&extract_image_resolved(&path)?.arr, dim)?;
        Ok(Response::new(data))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}

#[tauri::command]
pub async fn export_fits(
    path: String,
    output_path: String,
    bitpix: Option<i32>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        crate::infra::fits::writer::write_fits_mono(&output_path, &extract_image_resolved(&path)?.arr, None)?;
        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BITPIX: bitpix.unwrap_or(-32),
        }))
    })
}

#[tauri::command]
pub async fn export_fits_rgb(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_path: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let load_channel = |p: &Option<String>| -> anyhow::Result<ndarray::Array2<f32>> {
            match p {
                Some(path) => load_fits_array(path),
                None => anyhow::bail!("Channel path is required"),
            }
        };

        let r = load_channel(&r_path)?;
        let g = load_channel(&g_path)?;
        let b = load_channel(&b_path)?;

        crate::infra::fits::writer::write_fits_rgb(&output_path, &r, &g, &b, None)?;
        Ok(json!({ RES_OUTPUT_PATH: output_path }))
    })
}
