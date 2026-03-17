use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, resolve_output_dir, downsample_u8_rgb, MAX_PREVIEW_DIM};
use crate::core::compose::rgb::process_rgb;
use crate::core::compose::lrgb::apply_lrgb;
use crate::core::imaging::resample::resample_image;
use crate::infra::render::rgb::render_rgb;
use crate::types::compose::{RgbComposeConfig, RgbComposeResult, WhiteBalance};
use crate::types::constants::{DEFAULT_DIMENSION_TOLERANCE, DEFAULT_RGB_COMPOSITE_FILENAME, DEFAULT_SCNR_AMOUNT, DEFAULT_WB_VALUE, SCNR_METHOD_MAXIMUM, WB_MODE_MANUAL, WB_MODE_NONE, RES_DIMENSIONS, RES_DIMENSION_CROP, RES_ELAPSED_MS, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIN, RES_OFFSET_B, RES_OFFSET_G, RES_PNG_PATH, RES_SCNR_APPLIED, RES_STATS_B, RES_STATS_G, RES_STATS_R, LRGB_APPLIED, RESAMPLED};
use crate::types::image::{ScnrConfig, ScnrMethod};

fn load_channel(path: &Option<String>) -> anyhow::Result<Option<Array2<f32>>> {
    match path {
        Some(p) => Ok(Some(load_cached(p)?.arr().to_owned())),
        None => Ok(None),
    }
}

fn resample_to_largest(
    l: Option<Array2<f32>>,
    r: Option<Array2<f32>>,
    g: Option<Array2<f32>>,
    b: Option<Array2<f32>>,
) -> anyhow::Result<(Option<Array2<f32>>, Option<Array2<f32>>, Option<Array2<f32>>, Option<Array2<f32>>, bool)> {
    let dims: Vec<(usize, usize)> = [l.as_ref(), r.as_ref(), g.as_ref(), b.as_ref()]
        .iter()
        .filter_map(|ch| ch.map(|a| a.dim()))
        .collect();

    if dims.len() < 2 {
        return Ok((l, r, g, b, false));
    }

    let max_rows = dims.iter().map(|d| d.0).max().unwrap();
    let max_cols = dims.iter().map(|d| d.1).max().unwrap();
    let min_rows = dims.iter().map(|d| d.0).min().unwrap();
    let min_cols = dims.iter().map(|d| d.1).min().unwrap();

    let ratio_rows = max_rows as f64 / min_rows as f64;
    let ratio_cols = max_cols as f64 / min_cols as f64;

    if ratio_rows < 1.1 && ratio_cols < 1.1 {
        return Ok((l, r, g, b, false));
    }

    log::info!(
        "Auto-resample: harmonizing channels to {}x{} (ratio {:.1}x{:.1})",
        max_cols, max_rows, ratio_cols, ratio_rows
    );

    let resample_if_needed = |ch: Option<Array2<f32>>| -> anyhow::Result<Option<Array2<f32>>> {
        match ch {
            Some(arr) => {
                let (rows, cols) = arr.dim();
                if rows == max_rows && cols == max_cols {
                    Ok(Some(arr))
                } else {
                    Ok(Some(resample_image(&arr, max_rows, max_cols)?))
                }
            }
            None => Ok(None),
        }
    };

    Ok((
        resample_if_needed(l)?,
        resample_if_needed(r)?,
        resample_if_needed(g)?,
        resample_if_needed(b)?,
        true,
    ))
}

fn render_rgb_preview(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    path: &str,
    max_dim: usize,
) -> anyhow::Result<()> {
    use rayon::prelude::*;
    use anyhow::Context;

    let (rows, cols) = r.dim();

    if rows <= max_dim && cols <= max_dim {
        return render_rgb(r, g, b, path);
    }

    let r_slice = r.as_slice().context("R not contiguous")?;
    let g_slice = g.as_slice().context("G not contiguous")?;
    let b_slice = b.as_slice().context("B not contiguous")?;

    let npix = rows * cols;
    let mut pixels = vec![0u8; npix * 3];

    pixels
        .par_chunks_mut(cols * 3)
        .enumerate()
        .for_each(|(y, row_buf)| {
            let base = y * cols;
            for x in 0..cols {
                let i = base + x;
                let o = x * 3;
                row_buf[o] = (r_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
                row_buf[o + 1] = (g_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
                row_buf[o + 2] = (b_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
            }
        });

    let (preview, pw, ph) = downsample_u8_rgb(&pixels, cols, rows, max_dim);
    drop(pixels);

    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Sub,
    );
    use image::ImageEncoder;
    encoder
        .write_image(&preview, pw as u32, ph as u32, image::ColorType::Rgb8.into())
        .context("Failed to write RGB preview PNG")?;

    Ok(())
}

#[tauri::command]
pub async fn compose_rgb_cmd(
    l_path: Option<String>,
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_dir: String,
    auto_stretch: Option<bool>,
    linked_stf: Option<bool>,
    align: Option<bool>,
    wb_mode: Option<String>,
    wb_r: Option<f64>,
    wb_g: Option<f64>,
    wb_b: Option<f64>,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    dimension_tolerance: Option<usize>,
    lrgb_lightness: Option<f64>,
    lrgb_chrominance: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let l_arr = load_channel(&l_path)?;
        let r_arr = load_channel(&r_path)?;
        let g_arr = load_channel(&g_path)?;
        let b_arr = load_channel(&b_path)?;

        let (l_arr, r_arr, g_arr, b_arr, resampled) =
            resample_to_largest(l_arr, r_arr, g_arr, b_arr)?;

        let wb = match wb_mode.as_deref() {
            Some(WB_MODE_MANUAL) => WhiteBalance::Manual(
                wb_r.unwrap_or(DEFAULT_WB_VALUE),
                wb_g.unwrap_or(DEFAULT_WB_VALUE),
                wb_b.unwrap_or(DEFAULT_WB_VALUE),
            ),
            Some(WB_MODE_NONE) => WhiteBalance::None,
            _ => WhiteBalance::Auto,
        };

        let scnr_cfg = if scnr_enabled.unwrap_or(false) {
            let method = match scnr_method.as_deref() {
                Some(SCNR_METHOD_MAXIMUM) => ScnrMethod::MaximumNeutral,
                _ => ScnrMethod::AverageNeutral,
            };
            Some(ScnrConfig {
                method,
                amount: scnr_amount.unwrap_or(DEFAULT_SCNR_AMOUNT as f64) as f32,
                preserve_luminance: false,
            })
        } else {
            None
        };

        let config = RgbComposeConfig {
            white_balance: wb,
            auto_stretch: auto_stretch.unwrap_or(true),
            linked_stf: linked_stf.unwrap_or(false),
            align: align.unwrap_or(true),
            scnr: scnr_cfg,
            dimension_tolerance: dimension_tolerance.unwrap_or(DEFAULT_DIMENSION_TOLERANCE),
            ..RgbComposeConfig::default()
        };

        let mut processed = process_rgb(
            r_arr.as_ref(),
            g_arr.as_ref(),
            b_arr.as_ref(),
            &config,
        )?;

        let lrgb_applied = if let Some(l_data) = l_arr.as_ref() {
            let lightness = lrgb_lightness.unwrap_or(1.0) as f32;
            let chrominance = lrgb_chrominance.unwrap_or(1.0) as f32;

            let l_stretched = if config.auto_stretch {
                use crate::core::imaging::stf::{auto_stf, apply_stf_f32, analyze};
                use crate::types::image::AutoStfConfig;
                let (stats, _) = analyze(l_data);
                let stf = auto_stf(&stats, &AutoStfConfig::default());
                apply_stf_f32(l_data, &stf, &stats)
            } else {
                l_data.clone()
            };

            apply_lrgb(
                &l_stretched,
                &mut processed.r,
                &mut processed.g,
                &mut processed.b,
                lightness,
                chrominance,
            )?;
            true
        } else {
            false
        };

        let png_path = format!("{}/{}", output_dir, DEFAULT_RGB_COMPOSITE_FILENAME);

        render_rgb_preview(
            &processed.r,
            &processed.g,
            &processed.b,
            &png_path,
            MAX_PREVIEW_DIM,
        )?;

        let result = RgbComposeResult {
            png_path: png_path.clone(),
            stf_r: processed.stf_r,
            stf_g: processed.stf_g,
            stf_b: processed.stf_b,
            stats_r: processed.stats_r.clone(),
            stats_g: processed.stats_g.clone(),
            stats_b: processed.stats_b.clone(),
            offset_g: processed.offset_g,
            offset_b: processed.offset_b,
            width: processed.cols,
            height: processed.rows,
            scnr_applied: processed.scnr_applied,
            dimension_crop: processed.dimension_crop,
        };

        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: result.png_path,
            RES_DIMENSIONS: [result.width, result.height],
            RES_SCNR_APPLIED: result.scnr_applied,
            RES_OFFSET_G: [result.offset_g.0, result.offset_g.1],
            RES_OFFSET_B: [result.offset_b.0, result.offset_b.1],
            RES_DIMENSION_CROP: result.dimension_crop,
            RESAMPLED: resampled,
            LRGB_APPLIED: lrgb_applied,
            RES_STATS_R: { RES_MEDIAN: result.stats_r.median, RES_MEAN: result.stats_r.mean, RES_MIN: result.stats_r.min, RES_MAX: result.stats_r.max },
            RES_STATS_G: { RES_MEDIAN: result.stats_g.median, RES_MEAN: result.stats_g.mean, RES_MIN: result.stats_g.min, RES_MAX: result.stats_g.max },
            RES_STATS_B: { RES_MEDIAN: result.stats_b.median, RES_MEAN: result.stats_b.mean, RES_MIN: result.stats_b.min, RES_MAX: result.stats_b.max },
            RES_ELAPSED_MS: elapsed,
        }))
    })
}
