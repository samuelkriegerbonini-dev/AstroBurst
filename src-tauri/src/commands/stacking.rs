use std::path::Path;
use std::time::Instant;

use anyhow::Result;

use crate::domain::drizzle::{self, DrizzleConfig, DrizzleKernel};
use crate::domain::drizzle_rgb::{self, DrizzleRgbConfig};
use crate::domain::normalize::asinh_normalize;
use crate::domain::pipeline;
use crate::domain::rgb_compose::{self, RgbComposeConfig, WhiteBalance};
use crate::domain::scnr::{ScnrConfig, ScnrMethod};
use crate::utils::render::render_grayscale;

use super::helpers::*;

#[tauri::command]
pub async fn calibrate(
    science_path: String,
    output_dir: String,
    bias_paths: Option<Vec<String>>,
    dark_paths: Option<Vec<String>>,
    flat_paths: Option<Vec<String>>,
    dark_exposure_ratio: Option<f32>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();

        let calibrated = crate::domain::calibration::calibrate_from_paths(
            &science_path,
            bias_paths.as_deref(),
            dark_paths.as_deref(),
            flat_paths.as_deref(),
            dark_exposure_ratio.unwrap_or(1.0),
        )?;

        let out = resolve_output_dir(&output_dir)?;
        let normalized = asinh_normalize(&calibrated);

        let stem = Path::new(&science_path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let png_path = out.join(format!("{}_calibrated.png", stem));
        render_grayscale(&normalized, png_path.to_str().unwrap())?;

        let dims = calibrated.dim();
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "png_path": png_path.to_string_lossy(),
            "dimensions": [dims.1, dims.0],
            "has_bias": bias_paths.is_some(),
            "has_dark": dark_paths.is_some(),
            "has_flat": flat_paths.is_some(),
            "elapsed_ms": elapsed,
        }))
    })
        .await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(map_anyhow)
}

#[tauri::command]
pub async fn stack(
    paths: Vec<String>,
    output_dir: String,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
    max_iterations: Option<usize>,
    align: Option<bool>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();

        let config = crate::domain::stacking::StackConfig {
            sigma_low: sigma_low.unwrap_or(3.0),
            sigma_high: sigma_high.unwrap_or(3.0),
            max_iterations: max_iterations.unwrap_or(5),
            align: align.unwrap_or(true),
        };

        let stack_result = crate::domain::stacking::stack_from_paths(&paths, &config, None)?;

        let out = resolve_output_dir(&output_dir)?;
        let normalized = asinh_normalize(&stack_result.image);
        let png_path = out.join("stacked.png");
        render_grayscale(&normalized, png_path.to_str().unwrap())?;

        let dims = stack_result.image.dim();
        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "png_path": png_path.to_string_lossy(),
            "dimensions": [dims.1, dims.0],
            "frame_count": stack_result.frame_count,
            "rejected_pixels": stack_result.rejected_pixels,
            "offsets": stack_result.offsets.iter().map(|(dy, dx)| serde_json::json!({"dy": dy, "dx": dx})).collect::<Vec<_>>(),
            "elapsed_ms": elapsed,
        }))
    })
        .await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(map_anyhow)
}

#[tauri::command]
pub async fn drizzle_stack_cmd(
    paths: Vec<String>,
    output_dir: String,
    scale: Option<f64>,
    pixfrac: Option<f64>,
    kernel: Option<String>,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
    align: Option<bool>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();

        let drizzle_kernel = match kernel.as_deref() {
            Some("gaussian") => DrizzleKernel::Gaussian,
            Some("lanczos3") | Some("lanczos") => DrizzleKernel::Lanczos3,
            _ => DrizzleKernel::Square,
        };

        let config = DrizzleConfig {
            scale: scale.unwrap_or(2.0),
            pixfrac: pixfrac.unwrap_or(0.7),
            kernel: drizzle_kernel,
            sigma_low: sigma_low.unwrap_or(3.0),
            sigma_high: sigma_high.unwrap_or(3.0),
            sigma_iterations: 5,
            align: align.unwrap_or(true),
        };

        let drizzle_result = drizzle::drizzle_from_paths(&paths, &config, None)?;

        let out = resolve_output_dir(&output_dir)?;
        let normalized = asinh_normalize(&drizzle_result.image);
        let png_path = out.join("drizzle_result.png");
        render_grayscale(&normalized, png_path.to_str().unwrap())?;

        let wgt_normalized = {
            let max_w = drizzle_result
                .weight_map
                .iter()
                .cloned()
                .fold(0.0f32, f32::max);
            if max_w > 0.0 {
                drizzle_result.weight_map.mapv(|v| v / max_w)
            } else {
                drizzle_result.weight_map.clone()
            }
        };
        let wgt_path = out.join("drizzle_weights.png");
        render_grayscale(&wgt_normalized, wgt_path.to_str().unwrap())?;

        let elapsed = start.elapsed().as_millis() as u64;

        let offsets_json: Vec<serde_json::Value> = drizzle_result
            .offsets
            .iter()
            .map(|(dx, dy)| serde_json::json!({"dx": dx, "dy": dy}))
            .collect();

        Ok(serde_json::json!({
            "png_path": png_path.to_string_lossy(),
            "weight_map_path": wgt_path.to_string_lossy(),
            "input_dims": [drizzle_result.input_dims.1, drizzle_result.input_dims.0],
            "output_dims": [drizzle_result.output_dims.1, drizzle_result.output_dims.0],
            "scale": drizzle_result.output_scale,
            "frame_count": drizzle_result.frame_count,
            "rejected_pixels": drizzle_result.rejected_pixels,
            "offsets": offsets_json,
            "elapsed_ms": elapsed,
        }))
    })
        .await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(map_anyhow)
}

#[tauri::command]
pub async fn drizzle_rgb_cmd(
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
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let start = Instant::now();

        let drizzle_kernel = match kernel.as_deref() {
            Some("gaussian") => DrizzleKernel::Gaussian,
            Some("lanczos3") | Some("lanczos") => DrizzleKernel::Lanczos3,
            _ => DrizzleKernel::Square,
        };

        let drizzle_cfg = DrizzleConfig {
            scale: scale.unwrap_or(2.0),
            pixfrac: pixfrac.unwrap_or(0.7),
            kernel: drizzle_kernel,
            sigma_low: sigma_low.unwrap_or(3.0),
            sigma_high: sigma_high.unwrap_or(3.0),
            sigma_iterations: 5,
            align: align.unwrap_or(true),
        };

        let wb = match wb_mode.as_deref() {
            Some("manual") => WhiteBalance::Manual(
                wb_r.unwrap_or(1.0),
                wb_g.unwrap_or(1.0),
                wb_b.unwrap_or(1.0),
            ),
            Some("none") => WhiteBalance::None,
            _ => WhiteBalance::Auto,
        };

        let scnr_cfg = if scnr_enabled.unwrap_or(false) {
            let method = match scnr_method.as_deref() {
                Some("maximum") => ScnrMethod::MaximumNeutral,
                _ => ScnrMethod::AverageNeutral,
            };
            Some(ScnrConfig {
                method,
                amount: scnr_amount.unwrap_or(1.0) as f32,
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

        let out = resolve_output_dir(&output_dir)?;
        let png_path = out.join("drizzle_rgb.png");
        let png_str = png_path.to_string_lossy().to_string();

        let fits_out = if save_fits.unwrap_or(false) {
            Some(out.join("drizzle_rgb.fits").to_string_lossy().to_string())
        } else {
            None
        };

        let result = drizzle_rgb::drizzle_rgb(
            r_paths.as_deref(),
            g_paths.as_deref(),
            b_paths.as_deref(),
            &png_str,
            fits_out.as_deref(),
            &config,
        )?;

        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "png_path": result.png_path,
            "fits_path": result.fits_path,
            "input_dims": [result.input_dims.1, result.input_dims.0],
            "output_dims": [result.output_dims.1, result.output_dims.0],
            "scale": result.scale,
            "frame_count_r": result.frame_count_r,
            "frame_count_g": result.frame_count_g,
            "frame_count_b": result.frame_count_b,
            "rejected_pixels": result.rejected_pixels,
            "stf_r": { "shadow": result.stf_r.shadow, "midtone": result.stf_r.midtone, "highlight": result.stf_r.highlight },
            "stf_g": { "shadow": result.stf_g.shadow, "midtone": result.stf_g.midtone, "highlight": result.stf_g.highlight },
            "stf_b": { "shadow": result.stf_b.shadow, "midtone": result.stf_b.midtone, "highlight": result.stf_b.highlight },
            "stats_r": result.stats_r,
            "stats_g": result.stats_g,
            "stats_b": result.stats_b,
            "scnr_applied": result.scnr_applied,
            "elapsed_ms": elapsed,
        }))
    })
        .await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(map_anyhow)
}

#[tauri::command]
pub async fn compose_rgb_cmd(
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

        let r_arr = r_data.as_ref().map(|(_, arr, _)| arr);
        let g_arr = g_data.as_ref().map(|(_, arr, _)| arr);
        let b_arr = b_data.as_ref().map(|(_, arr, _)| arr);

        let wb = match wb_mode.as_deref() {
            Some("manual") => WhiteBalance::Manual(
                wb_r.unwrap_or(1.0),
                wb_g.unwrap_or(1.0),
                wb_b.unwrap_or(1.0),
            ),
            Some("none") => WhiteBalance::None,
            _ => WhiteBalance::Auto,
        };

        let scnr_cfg = if scnr_enabled.unwrap_or(false) {
            let method = match scnr_method.as_deref() {
                Some("maximum") => ScnrMethod::MaximumNeutral,
                _ => ScnrMethod::AverageNeutral,
            };
            Some(ScnrConfig {
                method,
                amount: scnr_amount.unwrap_or(1.0) as f32,
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
            ..Default::default()
        };

        let out = resolve_output_dir(&output_dir)?;
        let png_path = out.join("rgb_composite.png");
        let png_str = png_path.to_string_lossy().to_string();

        let result = rgb_compose::compose_rgb(r_arr, g_arr, b_arr, &png_str, &config)?;

        let elapsed = start.elapsed().as_millis() as u64;

        Ok(serde_json::json!({
            "png_path": result.png_path,
            "width": result.width,
            "height": result.height,
            "stf_r": { "shadow": result.stf_r.shadow, "midtone": result.stf_r.midtone, "highlight": result.stf_r.highlight },
            "stf_g": { "shadow": result.stf_g.shadow, "midtone": result.stf_g.midtone, "highlight": result.stf_g.highlight },
            "stf_b": { "shadow": result.stf_b.shadow, "midtone": result.stf_b.midtone, "highlight": result.stf_b.highlight },
            "stats_r": result.stats_r,
            "stats_g": result.stats_g,
            "stats_b": result.stats_b,
            "offset_g": [result.offset_g.0, result.offset_g.1],
            "offset_b": [result.offset_b.0, result.offset_b.1],
            "scnr_applied": result.scnr_applied,
            "elapsed_ms": elapsed,
        }))
    })
        .await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(map_anyhow)
}

#[tauri::command]
pub async fn run_pipeline_cmd(
    input_path: String,
    output_dir: String,
    frame_step: Option<usize>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let step = frame_step.unwrap_or(5);
        let pipeline_result = pipeline::run_pipeline(&input_path, &output_dir, step)?;

        let results_json: Vec<serde_json::Value> = pipeline_result
            .results
            .iter()
            .map(|r| match r {
                pipeline::SingleResult::Ok {
                    path,
                    cube,
                    elapsed_ms,
                } => serde_json::json!({
                    "path": path,
                    "status": "done",
                    "dimensions": cube.dimensions,
                    "collapsed_path": cube.collapsed_path,
                    "collapsed_median_path": cube.collapsed_median_path,
                    "frame_count": cube.frame_count,
                    "elapsed_ms": elapsed_ms,
                }),
                pipeline::SingleResult::Err { path, error } => serde_json::json!({
                    "path": path,
                    "status": "error",
                    "error": error,
                }),
            })
            .collect();

        Ok(serde_json::json!({
            "total_files": pipeline_result.total_files,
            "succeeded": pipeline_result.succeeded,
            "failed": pipeline_result.failed,
            "elapsed_ms": pipeline_result.elapsed_ms,
            "results": results_json,
        }))
    })
        .await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(map_anyhow)
}