use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use tauri::ipc::Response;

use crate::cmd::common::{blocking_cmd, extract_image_resolved, load_cached, load_cached_full, resolve_output_dir, save_preview_png, try_extract_rgb_resolved, save_rgb_preview_png};
use crate::core::imaging::stats::{compute_histogram_with_stats, compute_image_stats, downsample_histogram};
use crate::core::imaging::stf::StfParams;
use crate::core::imaging::stf::{apply_stf, apply_stf_f32, auto_stf, AutoStfConfig};
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::infra::fits::writer::{filter_header, write_fits_mono_bitpix, write_fits_rgb};
use crate::infra::ipc::encode_with_header_downsampled;
use crate::infra::render::grayscale::{render_grayscale_16bit, render_stretched_8bit, render_stretched_16bit};
use crate::infra::render::rgb::{render_rgb, render_rgb_16bit};
use crate::types::constants::{COPY_WCS, COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B, COMPOSITE_ORIG_R, COMPOSITE_ORIG_G, COMPOSITE_ORIG_B, HISTOGRAM_BINS_DISPLAY, RES_APPLY_STF, RES_AUTO_STF, RES_BINS, RES_BIN_COUNT, RES_BIT_DEPTH, RES_BITPIX, RES_COPY_METADATA, RES_DATA_MAX, RES_DATA_MIN, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FILE_SIZE_BYTES, RES_HEADER, RES_HIGHLIGHT, RES_HISTOGRAM, RES_MAD, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIDTONE, RES_MIN, RES_OUTPUT_PATH, RES_PNG_PATH, RES_SHADOW, RES_SIGMA, RES_STATS, RES_STF, RES_TOTAL_PIXELS};
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

fn process_rgb_fits(
    path: &str,
    output_dir: &str,
    t0: Instant,
    full: bool,
) -> anyhow::Result<Option<serde_json::Value>> {
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
    save_rgb_preview_png(&r_stretched, &g_stretched, &b_stretched, &png_path)?;

    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_R, Arc::new(rgb.r.clone()), stats_r.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_G, Arc::new(rgb.g.clone()), stats_g.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_KEY_B, Arc::new(rgb.b.clone()), stats_b.clone());

    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_R, Arc::new(rgb.r.clone()), stats_r.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_G, Arc::new(rgb.g.clone()), stats_g.clone());
    GLOBAL_IMAGE_CACHE.insert_synthetic(COMPOSITE_ORIG_B, Arc::new(rgb.b.clone()), stats_b.clone());

    let mut result = json!({
        RES_PNG_PATH: png_path,
        RES_DIMENSIONS: [cols, rows],
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        RES_STATS: stats_json_full(&stats_r),
        RES_STF: stf_json(&stf_r),
        "is_rgb": true,
        "stf_r": stf_json(&stf_r),
        "stf_g": stf_json(&stf_g),
        "stf_b": stf_json(&stf_b),
    });

    if full {
        let hist = compute_histogram_with_stats(&rgb.r, &stats_r);
        let display_bins = downsample_histogram(&hist, HISTOGRAM_BINS_DISPLAY);
        let header_json = serde_json::to_value(&rgb.header.index)?;

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
                RES_AUTO_STF: stf_json(&stf_r),
            }));
        }
    }

    Ok(Some(result))
}

#[tauri::command]
pub async fn process_fits(path: String, output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        if let Some(result) = process_rgb_fits(&path, &output_dir, t0, false)? {
            return Ok(result);
        }

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

        let target_bitpix = bitpix.unwrap_or(-32);
        let filtered = filter_header(&resolved.header, do_wcs, do_meta);
        write_fits_mono_bitpix(&output_path, &export_arr, filtered.as_ref(), target_bitpix)?;

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BITPIX: target_bitpix,
            RES_APPLY_STF: do_stf,
            COPY_WCS: do_wcs,
            RES_COPY_METADATA: do_meta,
            RES_FILE_SIZE_BYTES: file_size,
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

        let cache_r = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R);
        let cache_g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G);
        let cache_b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B);

        let (r_arr, g_arr, b_arr, header_source) = if let (Some(cr), Some(cg), Some(cb)) = (cache_r, cache_g, cache_b) {
            let r_hdr = r_path.as_deref()
                .and_then(|p| extract_image_resolved(p).ok())
                .map(|r| r.header);
            (cr.arr().to_owned(), cg.arr().to_owned(), cb.arr().to_owned(), r_hdr)
        } else {
            let r_resolved = extract_image_resolved(
                r_path.as_deref().ok_or_else(|| anyhow::anyhow!("R channel path required"))?
            )?;
            let g_resolved = extract_image_resolved(
                g_path.as_deref().ok_or_else(|| anyhow::anyhow!("G channel path required"))?
            )?;
            let b_resolved = extract_image_resolved(
                b_path.as_deref().ok_or_else(|| anyhow::anyhow!("B channel path required"))?
            )?;
            let g_raw = g_resolved.arr;
            let b_raw = b_resolved.arr;

            let (r_rows, r_cols) = r_resolved.arr.dim();
            let g_ok = g_raw.dim() == (r_rows, r_cols);
            let b_ok = b_raw.dim() == (r_rows, r_cols);

            let (g_final, b_final) = if g_ok && b_ok {
                (g_raw, b_raw)
            } else {
                let max_rows = r_rows.max(g_raw.dim().0).max(b_raw.dim().0);
                let max_cols = r_cols.max(g_raw.dim().1).max(b_raw.dim().1);
                let resample_if = |arr: ndarray::Array2<f32>| -> anyhow::Result<ndarray::Array2<f32>> {
                    if arr.dim() == (max_rows, max_cols) { return Ok(arr); }
                    crate::core::imaging::resample::resample_image(&arr, max_rows, max_cols)
                };
                (resample_if(g_raw)?, resample_if(b_raw)?)
            };

            let r_final = if r_resolved.arr.dim() != g_final.dim() {
                let (tr, tc) = g_final.dim();
                crate::core::imaging::resample::resample_image(&r_resolved.arr, tr, tc)?
            } else {
                r_resolved.arr
            };

            (r_final, g_final, b_final, Some(r_resolved.header))
        };

        let filtered = header_source
            .as_ref()
            .and_then(|h| filter_header(h, do_wcs, do_meta));
        write_fits_rgb(&output_path, &r_arr, &g_arr, &b_arr, filtered.as_ref())?;

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            COPY_WCS: do_wcs,
            RES_COPY_METADATA: do_meta,
            RES_FILE_SIZE_BYTES: file_size,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn export_png(
    path: String,
    output_path: String,
    bit_depth: Option<u8>,
    apply_stf_stretch: Option<bool>,
    shadow: Option<f64>,
    midtone: Option<f64>,
    highlight: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let depth = bit_depth.unwrap_or(16);
        let do_stf = apply_stf_stretch.unwrap_or(false);

        if let Some(rgb) = try_extract_rgb_resolved(&path)? {
            let sr = compute_image_stats(&rgb.r);
            let sg = compute_image_stats(&rgb.g);
            let sb = compute_image_stats(&rgb.b);

            let (r_out, g_out, b_out) = if do_stf {
                let stf = StfParams {
                    shadow: shadow.unwrap_or(0.0),
                    midtone: midtone.unwrap_or(0.5),
                    highlight: highlight.unwrap_or(1.0),
                };
                (
                    apply_stf_f32(&rgb.r, &stf, &sr),
                    apply_stf_f32(&rgb.g, &stf, &sg),
                    apply_stf_f32(&rgb.b, &stf, &sb),
                )
            } else {
                let ar = auto_stf(&sr, &AutoStfConfig::default());
                let ag = auto_stf(&sg, &AutoStfConfig::default());
                let ab = auto_stf(&sb, &AutoStfConfig::default());
                (
                    apply_stf_f32(&rgb.r, &ar, &sr),
                    apply_stf_f32(&rgb.g, &ag, &sg),
                    apply_stf_f32(&rgb.b, &ab, &sb),
                )
            };

            if depth == 16 {
                render_rgb_16bit(&r_out, &g_out, &b_out, &output_path)?;
            } else {
                render_rgb(&r_out, &g_out, &b_out, &output_path)?;
            }

            let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
            let (rows, cols) = rgb.r.dim();

            return Ok(json!({
                RES_OUTPUT_PATH: output_path,
                RES_BIT_DEPTH: depth,
                RES_APPLY_STF: true,
                RES_FILE_SIZE_BYTES: file_size,
                RES_DIMENSIONS: [cols, rows],
                RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            }));
        }

        let resolved = extract_image_resolved(&path)?;

        if do_stf {
            let stf = StfParams {
                shadow: shadow.unwrap_or(0.0),
                midtone: midtone.unwrap_or(0.5),
                highlight: highlight.unwrap_or(1.0),
            };
            let stats = compute_image_stats(&resolved.arr);
            let stretched = apply_stf_f32(&resolved.arr, &stf, &stats);
            if depth == 16 {
                render_stretched_16bit(&stretched, &output_path)?;
            } else {
                render_stretched_8bit(&stretched, &output_path)?;
            }
        } else if depth == 16 {
            render_grayscale_16bit(&resolved.arr, &output_path)?;
        } else {
            crate::infra::render::grayscale::render_grayscale(&resolved.arr, &output_path)?;
        }

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let (rows, cols) = resolved.arr.dim();

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BIT_DEPTH: depth,
            RES_APPLY_STF: do_stf,
            RES_FILE_SIZE_BYTES: file_size,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn export_rgb_png(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_path: String,
    bit_depth: Option<u8>,
    apply_stf_stretch: Option<bool>,
    shadow_r: Option<f64>,
    midtone_r: Option<f64>,
    highlight_r: Option<f64>,
    shadow_g: Option<f64>,
    midtone_g: Option<f64>,
    highlight_g: Option<f64>,
    shadow_b: Option<f64>,
    midtone_b: Option<f64>,
    highlight_b: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let depth = bit_depth.unwrap_or(16);
        let do_stf = apply_stf_stretch.unwrap_or(false);

        let cache_r = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R);
        let cache_g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G);
        let cache_b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B);

        if let (Some(cr), Some(cg), Some(cb)) = (&cache_r, &cache_g, &cache_b) {
            let (r_out, g_out, b_out);
            if do_stf {
                let stf_r = StfParams {
                    shadow: shadow_r.unwrap_or(0.0),
                    midtone: midtone_r.unwrap_or(0.5),
                    highlight: highlight_r.unwrap_or(1.0),
                };
                let stf_g = StfParams {
                    shadow: shadow_g.unwrap_or(0.0),
                    midtone: midtone_g.unwrap_or(0.5),
                    highlight: highlight_g.unwrap_or(1.0),
                };
                let stf_b = StfParams {
                    shadow: shadow_b.unwrap_or(0.0),
                    midtone: midtone_b.unwrap_or(0.5),
                    highlight: highlight_b.unwrap_or(1.0),
                };
                r_out = apply_stf_f32(cr.arr(), &stf_r, cr.stats());
                g_out = apply_stf_f32(cg.arr(), &stf_g, cg.stats());
                b_out = apply_stf_f32(cb.arr(), &stf_b, cb.stats());
            } else {
                let auto_r = auto_stf(cr.stats(), &AutoStfConfig::default());
                let auto_g = auto_stf(cg.stats(), &AutoStfConfig::default());
                let auto_b = auto_stf(cb.stats(), &AutoStfConfig::default());
                r_out = apply_stf_f32(cr.arr(), &auto_r, cr.stats());
                g_out = apply_stf_f32(cg.arr(), &auto_g, cg.stats());
                b_out = apply_stf_f32(cb.arr(), &auto_b, cb.stats());
            }
            if depth == 16 {
                render_rgb_16bit(&r_out, &g_out, &b_out, &output_path)?;
            } else {
                render_rgb(&r_out, &g_out, &b_out, &output_path)?;
            }
            let (rows, cols) = cr.arr().dim();
            let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
            return Ok(json!({
                RES_OUTPUT_PATH: output_path,
                RES_BIT_DEPTH: depth,
                RES_APPLY_STF: true,
                RES_FILE_SIZE_BYTES: file_size,
                RES_DIMENSIONS: [cols, rows],
                RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            }));
        }

        let r_entry = load_cached(
            r_path.as_deref().ok_or_else(|| anyhow::anyhow!("R channel path required"))?
        )?;
        let g_entry = load_cached(
            g_path.as_deref().ok_or_else(|| anyhow::anyhow!("G channel path required"))?
        )?;
        let b_entry = load_cached(
            b_path.as_deref().ok_or_else(|| anyhow::anyhow!("B channel path required"))?
        )?;

        let ra = r_entry.arr();
        let ga = g_entry.arr();
        let ba = b_entry.arr();

        let has_explicit_stf = shadow_r.is_some() || shadow_g.is_some() || shadow_b.is_some();

        let stretch_and_render = |r: &ndarray::Array2<f32>, g: &ndarray::Array2<f32>, b: &ndarray::Array2<f32>| -> anyhow::Result<serde_json::Value> {
            if do_stf {
                let sr = compute_image_stats(r);
                let sg = compute_image_stats(g);
                let sb = compute_image_stats(b);
                let (stf_r, stf_g, stf_b) = if has_explicit_stf {
                    (
                        StfParams { shadow: shadow_r.unwrap_or(0.0), midtone: midtone_r.unwrap_or(0.5), highlight: highlight_r.unwrap_or(1.0) },
                        StfParams { shadow: shadow_g.unwrap_or(0.0), midtone: midtone_g.unwrap_or(0.5), highlight: highlight_g.unwrap_or(1.0) },
                        StfParams { shadow: shadow_b.unwrap_or(0.0), midtone: midtone_b.unwrap_or(0.5), highlight: highlight_b.unwrap_or(1.0) },
                    )
                } else {
                    (
                        auto_stf(&sr, &AutoStfConfig::default()),
                        auto_stf(&sg, &AutoStfConfig::default()),
                        auto_stf(&sb, &AutoStfConfig::default()),
                    )
                };
                let ro = apply_stf_f32(r, &stf_r, &sr);
                let go = apply_stf_f32(g, &stf_g, &sg);
                let bo = apply_stf_f32(b, &stf_b, &sb);
                if depth == 16 { render_rgb_16bit(&ro, &go, &bo, &output_path)?; }
                else { render_rgb(&ro, &go, &bo, &output_path)?; }
            } else {
                if depth == 16 { render_rgb_16bit(r, g, b, &output_path)?; }
                else { render_rgb(r, g, b, &output_path)?; }
            }
            let (rows, cols) = r.dim();
            let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
            Ok(json!({
                RES_OUTPUT_PATH: output_path,
                RES_BIT_DEPTH: depth,
                RES_APPLY_STF: do_stf,
                RES_FILE_SIZE_BYTES: file_size,
                RES_DIMENSIONS: [cols, rows],
                RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            }))
        };

        if ra.dim() != ga.dim() || ra.dim() != ba.dim() {
            let max_rows = ra.dim().0.max(ga.dim().0).max(ba.dim().0);
            let max_cols = ra.dim().1.max(ga.dim().1).max(ba.dim().1);
            let resample_if = |arr: &ndarray::Array2<f32>| -> anyhow::Result<ndarray::Array2<f32>> {
                if arr.dim() == (max_rows, max_cols) { return Ok(arr.to_owned()); }
                crate::core::imaging::resample::resample_image(arr, max_rows, max_cols)
            };
            let ro = resample_if(ra)?;
            let go = resample_if(ga)?;
            let bo = resample_if(ba)?;
            return stretch_and_render(&ro, &go, &bo);
        }

        stretch_and_render(ra, ga, ba)
    })
}
