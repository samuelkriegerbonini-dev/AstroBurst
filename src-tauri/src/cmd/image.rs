use std::time::Instant;

use serde_json::json;
use tauri::ipc::Response;

use crate::cmd::common::{blocking_cmd, extract_image_resolved, load_cached, load_cached_full, load_fits_array, resolve_output_dir, save_preview_png};
use crate::core::imaging::stats::{compute_histogram_with_stats, compute_image_stats, downsample_histogram};
use crate::core::imaging::stf::StfParams;
use crate::core::imaging::stf::{apply_stf, apply_stf_f32, auto_stf, AutoStfConfig};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::writer::{filter_header, write_fits_mono, write_fits_rgb};
use crate::infra::ipc::{encode_with_header, encode_with_header_downsampled};
use crate::types::constants::{
    HISTOGRAM_BINS_DISPLAY,
    RES_AUTO_STF, RES_BINS, RES_BIN_COUNT, RES_BITPIX, RES_DATA_MAX, RES_DATA_MIN,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_HEADER, RES_HIGHLIGHT,
    RES_HISTOGRAM, RES_MAD, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIDTONE, RES_MIN,
    RES_OUTPUT_PATH, RES_PNG_PATH, RES_SHADOW, RES_SIGMA,
    RES_STATS, RES_STF, RES_TOTAL_PIXELS,
};
use crate::types::image::ImageStats;

fn png_path_for(path: &str, output_dir: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    format!("{}/{}.png", output_dir, stem)
}

fn render_to_png(cached: &ImageEntry, png_path: &str) -> anyhow::Result<(StfParams, usize, usize)> {
    let arr = cached.arr();
    let stats = cached.stats();
    let stf_params = auto_stf(stats, &AutoStfConfig::default());
    let rendered = apply_stf(arr, &stf_params, stats);
    let (rows, cols) = arr.dim();
    save_preview_png(rendered, cols, rows, png_path)?;
    Ok((stf_params, rows, cols))
}

fn stats_json(stats: &ImageStats) -> serde_json::Value {
    json!({
        RES_MIN: stats.min,
        RES_MAX: stats.max,
        RES_MEAN: stats.mean,
        RES_SIGMA: stats.sigma,
        RES_MEDIAN: stats.median,
    })
}

fn stats_json_full(stats: &ImageStats) -> serde_json::Value {
    json!({
        RES_MIN: stats.min,
        RES_MAX: stats.max,
        RES_MEAN: stats.mean,
        RES_SIGMA: stats.sigma,
        RES_MEDIAN: stats.median,
        RES_MAD: stats.mad,
    })
}

fn stf_json(stf: &StfParams) -> serde_json::Value {
    json!({
        RES_SHADOW: stf.shadow,
        RES_MIDTONE: stf.midtone,
        RES_HIGHLIGHT: stf.highlight,
    })
}

#[tauri::command]
pub async fn process_fits(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let cached = load_cached(&path)?;
        let png_path = png_path_for(&path, &output_dir);
        let (stf_params, rows, cols) = render_to_png(&cached, &png_path)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: stats_json(cached.stats()),
            RES_STF: stf_json(&stf_params),
        }))
    })
}

#[tauri::command]
pub async fn process_fits_full(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let cached = load_cached_full(&path)?;
        let png_path = png_path_for(&path, &output_dir);
        let (stf_params, rows, cols) = render_to_png(&cached, &png_path)?;

        let stats = cached.stats();
        let hist = compute_histogram_with_stats(cached.arr(), stats);
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);

        let header_json = match cached.header() {
            Some(h) => serde_json::to_value(&h.index)?,
            None => json!(null),
        };

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: stats_json_full(stats),
            RES_STF: stf_json(&stf_params),
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
                RES_AUTO_STF: stf_json(&stf_params),
            },
        }))
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
    apply_stf_stretch: Option<bool>,
    shadow: Option<f64>,
    midtone: Option<f64>,
    highlight: Option<f64>,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
    bitpix: Option<i32>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let resolved = extract_image_resolved(&path)?;
        let do_stf = apply_stf_stretch.unwrap_or(false);
        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);

        let export_arr = if do_stf {
            let stf = StfParams {
                shadow: shadow.unwrap_or(0.0),
                midtone: midtone.unwrap_or(0.5),
                highlight: highlight.unwrap_or(1.0),
            };
            let stats = compute_image_stats(&resolved.arr);
            apply_stf_f32(&resolved.arr, &stf, &stats)
        } else {
            resolved.arr
        };

        let filtered = filter_header(&resolved.header, do_wcs, do_meta);
        write_fits_mono(&output_path, &export_arr, filtered.as_ref())?;

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BITPIX: bitpix.unwrap_or(-32),
            "apply_stf": do_stf,
            "copy_wcs": do_wcs,
            "copy_metadata": do_meta,
            "file_size_bytes": file_size,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
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
    blocking_cmd!({
        let t0 = Instant::now();
        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);

        let r_resolved = extract_image_resolved(
            r_path.as_deref().ok_or_else(|| anyhow::anyhow!("R channel path required"))?
        )?;
        let g_arr = load_fits_array(
            g_path.as_deref().ok_or_else(|| anyhow::anyhow!("G channel path required"))?
        )?;
        let b_arr = load_fits_array(
            b_path.as_deref().ok_or_else(|| anyhow::anyhow!("B channel path required"))?
        )?;

        let filtered = filter_header(&r_resolved.header, do_wcs, do_meta);
        write_fits_rgb(&output_path, &r_resolved.arr, &g_arr, &b_arr, filtered.as_ref())?;

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            "copy_wcs": do_wcs,
            "copy_metadata": do_meta,
            "file_size_bytes": file_size,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}
