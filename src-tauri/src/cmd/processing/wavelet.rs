use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir};
use crate::cmd::processing::local_contrast::render_linear_output;
use crate::core::imaging::wavelet::{check_layer_bias, WaveletConfig};
use crate::core::ops::{run_op, OpInput, OpSpec, OpVersion};
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_WAVELET_PROGRESS,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_NOISE_ESTIMATE,
    RES_PNG_PATH, RES_SCALES_PROCESSED,
};

pub(crate) const SUFFIX_DENOISED: &str = "denoised";

#[tauri::command]
pub async fn wavelet_denoise_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    num_scales: usize,
    thresholds: Vec<f64>,
    linear: bool,
    layer_bias: Option<Vec<f32>>,
) -> Result<serde_json::Value, String> {
    let n_scales = num_scales.clamp(1, 8);
    let progress = ProgressHandle::new(&app, EVENT_WAVELET_PROGRESS, (n_scales * 2 + 1) as u64);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        check_layer_bias(layer_bias.as_deref())?;

        let entry = load_from_cache_or_disk(&path)?;

        let config = WaveletConfig {
            num_scales: n_scales,
            thresholds: thresholds.iter().map(|&t| t as f32).collect(),
            linear_denoise: linear,
            layer_bias,
        };

        let t0 = std::time::Instant::now();
        let denoised = run_op(&OpSpec::WaveletDenoise { v: OpVersion, config }, &OpInput::mono(entry.arr(), None), Some(&progress_clone))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_linear_output(&denoised.planes[0], &path, &entry, &output_dir, SUFFIX_DENOISED)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_SCALES_PROCESSED: denoised.extras[RES_SCALES_PROCESSED],
            RES_NOISE_ESTIMATE: denoised.extras[RES_NOISE_ESTIMATE],
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}
