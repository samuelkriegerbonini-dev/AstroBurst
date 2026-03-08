use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, render_and_save, resolve_output_dir};
use crate::core::analysis::deconvolution::{generate_gaussian_psf, richardson_lucy};
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_DECONV_PROGRESS, SUFFIX_DECONV,
    RES_CONVERGENCE, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH,
    RES_ITERATIONS_RUN, RES_PNG_PATH,
};
use crate::types::stacking::RLConfig;

#[tauri::command]
pub async fn deconvolve_rl_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    iterations: usize,
    psf_sigma: f64,
    psf_size: usize,
    regularization: f64,
    deringing: bool,
    dering_threshold: f64,
) -> Result<serde_json::Value, String> {
    let progress_clone = ProgressHandle::new(&app, EVENT_DECONV_PROGRESS, iterations as u64).clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let rl_result = richardson_lucy(
            load_from_cache_or_disk(&path)?.arr(),
            &generate_gaussian_psf(psf_size, psf_sigma as f32),
            &RLConfig {
                iterations,
                psf_sigma,
                psf_size,
                regularization,
                deringing,
                deringing_threshold: dering_threshold as f32,
            },
            Some(&progress_clone))?;

        let ro = render_and_save(&rl_result.image, &path, &output_dir, SUFFIX_DECONV, true)?;
        let (rows, cols) = ro.dims;

        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_ITERATIONS_RUN: rl_result.iterations_run,
            RES_CONVERGENCE: rl_result.convergence,
            RES_ELAPSED_MS: rl_result.elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}
