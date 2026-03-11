use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, render_and_save, resolve_output_dir};
use crate::core::analysis::deconvolution::{generate_gaussian_psf, richardson_lucy};
use crate::core::imaging::psf_estimation::{estimate_psf, psf_to_kernel, PsfEstimationConfig};
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
    use_empirical_psf: Option<bool>,
    psf_num_stars: Option<usize>,
    psf_cutout_radius: Option<usize>,
) -> Result<serde_json::Value, String> {
    let progress_clone = ProgressHandle::new(&app, EVENT_DECONV_PROGRESS, iterations as u64).clone();

    blocking_cmd!({
        resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let psf_kernel = if use_empirical_psf.unwrap_or(false) {
            let psf_config = PsfEstimationConfig {
                num_stars: psf_num_stars.unwrap_or(3),
                cutout_radius: psf_cutout_radius.unwrap_or(15),
                ..PsfEstimationConfig::default()
            };

            let psf_result = estimate_psf(image, &psf_config)
                .map_err(|e| anyhow::anyhow!("PSF estimation failed: {}", e))?;

            psf_to_kernel(&psf_result)
        } else {
            generate_gaussian_psf(psf_size, psf_sigma as f32)
        };

        let rl_config = RLConfig {
            iterations,
            psf_sigma,
            psf_size,
            regularization,
            deringing,
            deringing_threshold: dering_threshold as f32,
        };

        let rl_result = richardson_lucy(image, &psf_kernel, &rl_config, Some(&progress_clone))?;

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
