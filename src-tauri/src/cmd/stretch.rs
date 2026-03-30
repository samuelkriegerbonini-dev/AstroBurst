use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, render_and_save, resolve_output_dir};
use crate::core::imaging::stretch::arcsinh_stretch;
use crate::core::imaging::masked_stretch::{masked_stretch, MaskedStretchConfig};
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH,
    RES_STRETCH_FACTOR, RES_ITERATIONS_RUN, RES_STARS_MASKED,
    RES_MASK_COVERAGE, RES_FINAL_BACKGROUND, RES_CONVERGED,
    SUFFIX_MASKED_STRETCH,
};

#[tauri::command]
pub async fn apply_arcsinh_stretch_cmd(
    path: String,
    output_dir: String,
    factor: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let clamped_factor = (factor as f32).clamp(1.0, 500.0);

        let t0 = std::time::Instant::now();
        let stretched = arcsinh_stretch(image, clamped_factor);
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_and_save(&stretched, &path, &output_dir, "arcsinh", true)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_STRETCH_FACTOR: clamped_factor,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn masked_stretch_cmd(
    path: String,
    output_dir: String,
    iterations: Option<usize>,
    target_background: Option<f64>,
    mask_growth: Option<f64>,
    mask_softness: Option<f64>,
    protection_amount: Option<f64>,
    luminance_protect: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let config = MaskedStretchConfig {
            iterations: iterations.unwrap_or(10),
            target_background: target_background.unwrap_or(0.25),
            mask_growth: mask_growth.unwrap_or(2.5),
            mask_softness: mask_softness.unwrap_or(4.0),
            protection_amount: protection_amount.unwrap_or(0.85),
            luminance_protect: luminance_protect.unwrap_or(true),
            ..MaskedStretchConfig::default()
        };

        let t0 = std::time::Instant::now();
        let result = masked_stretch(image, &config)?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_and_save(&result.image, &path, &output_dir, SUFFIX_MASKED_STRETCH, true)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_ITERATIONS_RUN: result.iterations_run,
            RES_FINAL_BACKGROUND: result.final_background,
            RES_STARS_MASKED: result.stars_masked,
            RES_MASK_COVERAGE: result.mask_coverage,
            RES_CONVERGED: result.converged,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}
