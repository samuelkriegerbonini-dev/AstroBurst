use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir};
use crate::cmd::processing::local_contrast::render_linear_output;
use crate::core::imaging::wavelet::{wavelet_denoise, WaveletConfig};
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_WAVELET_PROGRESS,
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_NOISE_ESTIMATE,
    RES_PNG_PATH, RES_SCALES_PROCESSED,
};

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

        if let Some(bias) = &layer_bias {
            if bias.iter().any(|b| !b.is_finite()) {
                anyhow::bail!("layer_bias must contain only finite values");
            }
        }

        let entry = load_from_cache_or_disk(&path)?;

        let config = WaveletConfig {
            num_scales: n_scales,
            thresholds: thresholds.iter().map(|&t| t as f32).collect(),
            linear_denoise: linear,
            layer_bias,
        };

        let wav_result = wavelet_denoise(entry.arr(), &config, Some(&progress_clone))?;

        let ro = render_linear_output(&wav_result.denoised, &path, &entry, &output_dir, "denoised")?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_SCALES_PROCESSED: wav_result.scales_processed,
            RES_NOISE_ESTIMATE: wav_result.noise_estimate,
            RES_ELAPSED_MS: wav_result.elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}
