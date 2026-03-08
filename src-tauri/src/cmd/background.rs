use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir};
use crate::core::imaging::background::{extract_background, BackgroundConfig, BackgroundMode};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{auto_stf, apply_stf, AutoStfConfig};
use crate::infra::progress::ProgressHandle;
use crate::infra::render::grayscale::save_stf_png;
use crate::types::constants::{
    MODE_DIVIDE, PROGRESS_EVENT, PROGRESS_STEPS, DEFAULT_STEM,
    MIN_GRID_SIZE, MAX_GRID_SIZE, MIN_POLY_DEGREE, MAX_POLY_DEGREE, MIN_ITERATIONS, MAX_ITERATIONS,
    RES_CORRECTED_PNG, RES_MODEL_PNG, RES_CORRECTED_FITS, RES_SAMPLE_COUNT,
    RES_RMS_RESIDUAL, RES_ELAPSED_MS, RES_DIMENSIONS,
};

#[tauri::command]
pub async fn extract_background_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: String,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, PROGRESS_EVENT, PROGRESS_STEPS as u64);
    let progress_clone = progress.clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;

        let bg_mode = match mode.as_str() {
            MODE_DIVIDE => BackgroundMode::Divide,
            _ => BackgroundMode::Subtract,
        };

        let config = BackgroundConfig {
            grid_size: grid_size.clamp(MIN_GRID_SIZE, MAX_GRID_SIZE),
            poly_degree: poly_degree.clamp(MIN_POLY_DEGREE, MAX_POLY_DEGREE),
            sigma_clip: sigma_clip as f32,
            iterations: iterations.clamp(MIN_ITERATIONS, MAX_ITERATIONS),
            mode: bg_mode,
        };

        let bg_result = extract_background(entry.arr(), &config, Some(&progress_clone))?;

        let stats = compute_image_stats(&bg_result.corrected);
        let stf_params = auto_stf(&stats, &AutoStfConfig::default());
        let rendered = apply_stf(&bg_result.corrected, &stf_params, &stats);

        let model_stats = compute_image_stats(&bg_result.model);
        let model_stf = auto_stf(&model_stats, &AutoStfConfig::default());
        let model_rendered = apply_stf(&bg_result.model, &model_stf, &model_stats);

        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(DEFAULT_STEM);

        let corrected_png = format!("{}/{}_bg_corrected.png", output_dir, stem);
        let model_png = format!("{}/{}_bg_model.png", output_dir, stem);
        let corrected_fits = format!("{}/{}_bg_corrected.fits", output_dir, stem);

        let (rows, cols) = bg_result.corrected.dim();
        save_stf_png(&rendered, cols, rows, &corrected_png)?;
        save_stf_png(&model_rendered, cols, rows, &model_png)?;
        crate::infra::fits::writer::write_fits_mono(&corrected_fits, &bg_result.corrected, None)?;

        Ok(json!({
            RES_CORRECTED_PNG: corrected_png,
            RES_MODEL_PNG: model_png,
            RES_CORRECTED_FITS: corrected_fits,
            RES_SAMPLE_COUNT: bg_result.sample_count,
            RES_RMS_RESIDUAL: bg_result.rms_residual,
            RES_ELAPSED_MS: bg_result.elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}
