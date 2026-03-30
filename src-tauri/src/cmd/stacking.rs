use serde_json::json;

use crate::cmd::common::{blocking_cmd, render_asinh_and_save, resolve_output_dir};
use crate::core::imaging::stats::compute_image_stats;
use crate::domain::calibration::calibrate_from_paths;
use crate::domain::stacking::stack_from_paths;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_CALIBRATE_PROGRESS, EVENT_STACK_PROGRESS, STAGE_RENDER, STAGE_SAVE,
    RES_DIMENSIONS, RES_DX, RES_DY, RES_FITS_PATH, RES_FRAME_COUNT,
    RES_HAS_BIAS, RES_HAS_DARK, RES_HAS_FLAT, RES_MAX, RES_MEAN, RES_MIN,
    RES_OFFSETS, RES_PNG_PATH, RES_REJECTED_PIXELS, RES_SIGMA, RES_STATS,
};
use crate::types::stacking::StackConfig;

#[tauri::command]
pub async fn calibrate(
    app: tauri::AppHandle,
    science_path: String,
    output_dir: String,
    bias_paths: Option<Vec<String>>,
    dark_paths: Option<Vec<String>>,
    flat_paths: Option<Vec<String>>,
    dark_exposure_ratio: Option<f32>,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, EVENT_CALIBRATE_PROGRESS, 4);
    let progress_clone = progress.clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let calibrated = calibrate_from_paths(
            &science_path,
            bias_paths.as_deref(),
            dark_paths.as_deref(),
            flat_paths.as_deref(),
            dark_exposure_ratio.unwrap_or(1.0),
        )?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let stem = std::path::Path::new(&science_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("calibrated");

        let (png_path, fits_path) = render_asinh_and_save(
            &calibrated,
            &output_dir,
            &format!("{}_calibrated", stem),
            true,
        )?;

        let (rows, cols) = calibrated.dim();
        let stats = compute_image_stats(&calibrated);

        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: [cols, rows],
            RES_HAS_BIAS: bias_paths.is_some(),
            RES_HAS_DARK: dark_paths.is_some(),
            RES_HAS_FLAT: flat_paths.is_some(),
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
pub async fn stack(
    app: tauri::AppHandle,
    paths: Vec<String>,
    output_dir: String,
    sigma_low: Option<f32>,
    sigma_high: Option<f32>,
    max_iterations: Option<usize>,
    align: Option<bool>,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    let frame_count = paths.len() as u64;
    let progress = ProgressHandle::new(&app, EVENT_STACK_PROGRESS, frame_count + 2);
    let progress_clone = progress.clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let config = StackConfig {
            sigma_low: sigma_low.unwrap_or(3.0),
            sigma_high: sigma_high.unwrap_or(3.0),
            max_iterations: max_iterations.unwrap_or(5),
            align: align.unwrap_or(true),
        };

        let result = stack_from_paths(&paths, &config, None)?;

        progress_clone.tick_with_stage(STAGE_RENDER);

        let stem = name.as_deref().unwrap_or("stacked");

        let (png_path, fits_path) = render_asinh_and_save(
            &result.image,
            &output_dir,
            stem,
            true,
        )?;

        let (rows, cols) = result.image.dim();
        let stats = compute_image_stats(&result.image);

        progress_clone.tick_with_stage(STAGE_SAVE);
        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: [cols, rows],
            RES_FRAME_COUNT: result.frame_count,
            RES_REJECTED_PIXELS: result.rejected_pixels,
            RES_OFFSETS: result.offsets.iter().map(|(dy, dx)| json!({RES_DY: dy, RES_DX: dx})).collect::<Vec<_>>(),
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}
