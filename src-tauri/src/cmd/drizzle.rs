use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, resolve_output_dir};
use crate::core::imaging::normalize::robust_asinh_preview;
use crate::core::imaging::stats::compute_image_stats;
use crate::domain::drizzle::drizzle_from_paths;
use crate::domain::drizzle_rgb::drizzle_rgb;
use crate::infra::progress::ProgressHandle;
use crate::infra::render::grayscale::render_grayscale;
use crate::types::compose::{DrizzleRgbConfig, WhiteBalance};
use crate::types::constants::{
    DEFAULT_DRIZZLE_PIXFRAC, DEFAULT_DRIZZLE_SCALE, DEFAULT_DRIZZLE_SIGMA,
    DEFAULT_DRIZZLE_SIGMA_ITERS, DEFAULT_SCNR_AMOUNT, DEFAULT_WB_VALUE,
    EVENT_DRIZZLE_PROGRESS, EVENT_DRIZZLE_RGB_PROGRESS,
    FILE_DRIZZLE_RESULT_FITS, FILE_DRIZZLE_RESULT_PNG, FILE_DRIZZLE_RGB_FITS,
    FILE_DRIZZLE_RGB_PNG, FILE_DRIZZLE_WEIGHTS_PNG,
    KERNEL_GAUSSIAN, KERNEL_LANCZOS, KERNEL_LANCZOS3,
    SCNR_METHOD_MAXIMUM, STAGE_RENDER, STAGE_SAVE,
    WB_MODE_MANUAL, WB_MODE_NONE,
    RES_DIMENSIONS, RES_DX, RES_DY, RES_ELAPSED_MS, RES_FITS_PATH, RES_FRAME_COUNT,
    RES_FRAME_COUNT_B, RES_FRAME_COUNT_G, RES_FRAME_COUNT_R,
    RES_INPUT_DIMS, RES_MAX, RES_MEAN, RES_MIN, RES_OFFSETS, RES_OUTPUT_DIMS,
    RES_PNG_PATH, RES_REJECTED_PIXELS, RES_SCALE, RES_SIGMA, RES_STATS,
    RES_WEIGHT_MAP_PATH,
};
use crate::types::image::{ScnrConfig, ScnrMethod};
use crate::types::stacking::{DrizzleConfig, DrizzleKernel};

#[tauri::command]
pub async fn drizzle_stack_cmd(
    app: tauri::AppHandle,
    paths: Vec<String>,
    output_dir: String,
    scale: Option<f64>,
    pixfrac: Option<f64>,
    kernel: Option<String>,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
    align: Option<bool>,
) -> Result<serde_json::Value, String> {
    let progress_clone =
        ProgressHandle::new(&app, EVENT_DRIZZLE_PROGRESS,
                            paths.len() as u64 + 2)
            .clone();

    let scale_val = scale.unwrap_or(DEFAULT_DRIZZLE_SCALE);

    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let k = match kernel.as_deref() {
            Some(KERNEL_GAUSSIAN) => DrizzleKernel::Gaussian,
            Some(KERNEL_LANCZOS3) | Some(KERNEL_LANCZOS) => DrizzleKernel::Lanczos3,
            _ => DrizzleKernel::Square,
        };

        let config = DrizzleConfig {
            scale: scale_val,
            pixfrac: pixfrac.unwrap_or(DEFAULT_DRIZZLE_PIXFRAC),
            kernel: k,
            sigma_low: sigma_low.unwrap_or(DEFAULT_DRIZZLE_SIGMA),
            sigma_high: sigma_high.unwrap_or(DEFAULT_DRIZZLE_SIGMA),
            sigma_iterations: DEFAULT_DRIZZLE_SIGMA_ITERS,
            align: align.unwrap_or(true),
        };

        let result = drizzle_from_paths(&paths, &config, None)?;

        let normalized = robust_asinh_preview(&result.image);
        progress_clone.tick_with_stage(STAGE_RENDER);

        let png_path = format!("{}/{}", output_dir, FILE_DRIZZLE_RESULT_PNG);
        let fits_path = format!("{}/{}", output_dir, FILE_DRIZZLE_RESULT_FITS);
        let weight_path = format!("{}/{}", output_dir, FILE_DRIZZLE_WEIGHTS_PNG);

        render_grayscale(&normalized, &png_path)?;
        
        crate::infra::fits::writer::write_fits_mono(&fits_path, &result.image, None)?;
        
        render_grayscale(&{
            let max_w = result.weight_map.iter().cloned().fold(0.0f32, f32::max);
            if max_w > 0.0 {
                result.weight_map.mapv(|v| v / max_w)
            } else {
                result.weight_map.clone()
            }
        }, &weight_path)?;

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

        let (rows, cols) = result.image.dim();
        let stats = compute_image_stats(&result.image);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_WEIGHT_MAP_PATH: weight_path,
            RES_DIMENSIONS: [cols, rows],
            RES_OUTPUT_DIMS: [cols, rows],
            RES_INPUT_DIMS: [(cols as f64 / scale_val).round() as usize, (rows as f64 / scale_val).round() as usize],
            RES_FRAME_COUNT: result.frame_count,
            RES_REJECTED_PIXELS: result.rejected_pixels,
            RES_OFFSETS: result.offsets.iter().map(|(dy, dx)| json!({RES_DY: dy, RES_DX: dx})).collect::<Vec<_>>(),
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            RES_SCALE: scale_val,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}

#[tauri::command]
pub async fn drizzle_rgb_cmd(
    app: tauri::AppHandle,
    r_paths: Option<Vec<String>>,
    g_paths: Option<Vec<String>>,
    b_paths: Option<Vec<String>>,
    output_dir: String,
    scale: Option<f64>,
    pixfrac: Option<f64>,
    kernel: Option<String>,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
    align: Option<bool>,
    wb_mode: Option<String>,
    wb_r: Option<f64>,
    wb_g: Option<f64>,
    wb_b: Option<f64>,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    save_fits: Option<bool>,
) -> Result<serde_json::Value, String> {
    let total_frames = r_paths.as_ref().map_or(0, |v| v.len())
        + g_paths.as_ref().map_or(0, |v| v.len())
        + b_paths.as_ref().map_or(0, |v| v.len());
    let progress_clone = ProgressHandle::new(&app, EVENT_DRIZZLE_RGB_PROGRESS, total_frames as u64 + 2)
        .clone();
    let scale_val = scale.unwrap_or(DEFAULT_DRIZZLE_SCALE);

    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let k = match kernel.as_deref() {
            Some(KERNEL_GAUSSIAN) => DrizzleKernel::Gaussian,
            Some(KERNEL_LANCZOS3) | Some(KERNEL_LANCZOS) => DrizzleKernel::Lanczos3,
            _ => DrizzleKernel::Square,
        };

        let drizzle_cfg = DrizzleConfig {
            scale: scale_val,
            pixfrac: pixfrac.unwrap_or(DEFAULT_DRIZZLE_PIXFRAC),
            kernel: k,
            sigma_low: sigma_low.unwrap_or(DEFAULT_DRIZZLE_SIGMA),
            sigma_high: sigma_high.unwrap_or(DEFAULT_DRIZZLE_SIGMA),
            sigma_iterations: DEFAULT_DRIZZLE_SIGMA_ITERS,
            align: align.unwrap_or(true),
        };

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

        let config = DrizzleRgbConfig {
            drizzle: drizzle_cfg,
            white_balance: wb,
            auto_stretch: true,
            linked_stf: false,
            scnr: scnr_cfg,
        };

        let png_path = format!("{}/{}", output_dir, FILE_DRIZZLE_RGB_PNG);
        let fits_out = if save_fits.unwrap_or(false) {
            Some(format!("{}/{}", output_dir, FILE_DRIZZLE_RGB_FITS))
        } else {
            None
        };

        let result = drizzle_rgb(
            r_paths.as_deref(),
            g_paths.as_deref(),
            b_paths.as_deref(),
            &png_path,
            fits_out.as_deref(),
            &config,
        )?;

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

        let (out_h, out_w) = result.output_dims;
        let in_w = (out_w as f64 / scale_val).round() as usize;
        let in_h = (out_h as f64 / scale_val).round() as usize;
        let elapsed = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_PNG_PATH: result.png_path,
            RES_FITS_PATH: result.fits_path,
            RES_DIMENSIONS: [out_w, out_h],
            RES_OUTPUT_DIMS: [out_w, out_h],
            RES_INPUT_DIMS: [in_w, in_h],
            RES_FRAME_COUNT_R: result.frame_count_r,
            RES_FRAME_COUNT_G: result.frame_count_g,
            RES_FRAME_COUNT_B: result.frame_count_b,
            RES_REJECTED_PIXELS: result.rejected_pixels,
            RES_ELAPSED_MS: elapsed,
            RES_SCALE: scale_val,
        }))
    })
}
