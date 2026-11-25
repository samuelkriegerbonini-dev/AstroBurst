use std::sync::Arc;
use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, image_ref, load_cached, load_cached_full, output_stem, plane_info_json, resolve_output_dir, save_stf_preview_png, try_extract_rgb_resolved, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::imaging::stats::{compute_histogram_with_stats, compute_image_stats, downsample_histogram};
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig};
use crate::infra::cache::ImageEntry;
use crate::types::constants::{HISTOGRAM_BINS_DISPLAY, RES_AUTO_STF, RES_BINS, RES_BIN_COUNT, RES_DATA_MAX, RES_DATA_MIN, RES_DIMENSIONS, RES_ELAPSED_MS, RES_HEADER, RES_HISTOGRAM, RES_IMAGE_REF, RES_MAD, RES_MEAN, RES_MEDIAN, RES_PLANE, RES_PNG_PATH, RES_SIGMA, RES_STATS, RES_STF, RES_TOTAL_PIXELS, RES_IS_RGB, STF_R, STF_G, STF_B};
use crate::types::image::StfParams;

use super::rgb_planes::{remember_rgb_planes, rgb_stamp};

pub(super) fn png_path_for(path: &str, output_dir: &str) -> String {
    format!("{}/{}.png", output_dir, output_stem(path))
}

pub(super) fn render_to_png(cached: &ImageEntry, png_path: &str) -> anyhow::Result<(StfParams, usize, usize)> {
    let stats = cached.stats();
    let stf_params = auto_stf(stats, &AutoStfConfig::default());
    save_stf_preview_png(cached.arr(), &stf_params, stats, png_path)?;
    let (rows, cols) = cached.arr().dim();
    Ok((stf_params, rows, cols))
}

pub(super) fn process_rgb_fits(
    path: &str,
    output_dir: &str,
    t0: Instant,
    full: bool,
) -> anyhow::Result<Option<serde_json::Value>> {
    let stamp = rgb_stamp(path);
    let rgb = match try_extract_rgb_resolved(path)? {
        Some(r) => r,
        None => return Ok(None),
    };

    let stats_r = compute_image_stats(&rgb.r);
    let stats_g = compute_image_stats(&rgb.g);
    let stats_b = compute_image_stats(&rgb.b);

    let stf_r = auto_stf(&stats_r, &AutoStfConfig::default());
    let stf_g = auto_stf(&stats_g, &AutoStfConfig::default());
    let stf_b = auto_stf(&stats_b, &AutoStfConfig::default());

    let r_stretched = apply_stf_f32(&rgb.r, &stf_r, &stats_r);
    let g_stretched = apply_stf_f32(&rgb.g, &stf_g, &stats_g);
    let b_stretched = apply_stf_f32(&rgb.b, &stf_b, &stats_b);

    let (rows, cols) = rgb.r.dim();
    let png_path = png_path_for(path, output_dir);
    helpers::render_rgb_preview(&r_stretched, &g_stretched, &b_stretched, &png_path, MAX_PREVIEW_DIM)?;

    let full_data = if full {
        let hist = compute_histogram_with_stats(&rgb.r, &stats_r);
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);
        let header_json = serde_json::to_value(&rgb.header.index)?;
        Some((display_bins, header_json))
    } else {
        None
    };

    if let Some(stamp) = stamp {
        remember_rgb_planes(path, stamp, &(Arc::new(rgb.r), Arc::new(rgb.g), Arc::new(rgb.b)));
    }

    let mut result = json!({
        RES_PNG_PATH: png_path,
        RES_DIMENSIONS: [cols, rows],
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        RES_STATS: helpers::stats_json_full(&stats_r),
        RES_STF: helpers::stf_json(&stf_r),
        RES_IS_RGB: true,
        STF_R: helpers::stf_json(&stf_r),
        STF_G: helpers::stf_json(&stf_g),
        STF_B: helpers::stf_json(&stf_b),
    });

    if let Some((display_bins, header_json)) = full_data {
        if let Some(obj) = result.as_object_mut() {
            obj.insert(RES_HEADER.to_string(), header_json);
            obj.insert(RES_HISTOGRAM.to_string(), json!({
                RES_BINS: display_bins,
                RES_BIN_COUNT: display_bins.len(),
                RES_DATA_MIN: stats_r.min,
                RES_DATA_MAX: stats_r.max,
                RES_MEDIAN: stats_r.median,
                RES_MEAN: stats_r.mean,
                RES_SIGMA: stats_r.sigma,
                RES_MAD: stats_r.mad,
                RES_TOTAL_PIXELS: stats_r.valid_count,
                RES_AUTO_STF: helpers::stf_json(&stf_r),
            }));
        }
    }

    Ok(Some(result))
}

#[tauri::command]
pub async fn process_fits(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let output_dir = resolve_output_dir(&output_dir)?;

        if let Some(result) = process_rgb_fits(&path, &output_dir, t0, false)? {
            return Ok(result);
        }

        let cached = load_cached(&path)?;
        let png_path = png_path_for(&path, &output_dir);
        let (stf_params, rows, cols) = render_to_png(&cached, &png_path)?;
        let plane = plane_json_for(&path, &cached);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: helpers::stats_json(cached.stats()),
            RES_STF: helpers::stf_json(&stf_params),
            RES_IMAGE_REF: image_ref(&path).cache_key(),
            RES_PLANE: plane,
        }))
    })
}

fn plane_json_for(path: &str, cached: &ImageEntry) -> serde_json::Value {
    match plane_info_json(path, cached) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("plane info unavailable for {}: {:#}", path, e);
            serde_json::Value::Null
        }
    }
}

#[tauri::command]
pub async fn process_fits_full(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let output_dir = resolve_output_dir(&output_dir)?;

        if let Some(result) = process_rgb_fits(&path, &output_dir, t0, true)? {
            return Ok(result);
        }

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
        let plane = plane_json_for(&path, &cached);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_STATS: helpers::stats_json_full(stats),
            RES_STF: helpers::stf_json(&stf_params),
            RES_HEADER: header_json,
            RES_IMAGE_REF: image_ref(&path).cache_key(),
            RES_PLANE: plane,
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
                RES_AUTO_STF: helpers::stf_json(&stf_params),
            },
        }))
    })
}

#[tauri::command]
pub async fn release_cube_cmd(path: String) -> Result<(), String> {
    GLOBAL_CUBE_CACHE.invalidate(&path);
    crate::cmd::ramp::inspect::release_asdf_ramp(&path);
    Ok(())
}
