use serde::Deserialize;
use serde_json::json;

use crate::core::imaging::calibration_pipeline::{
    run_batch_pipeline, BatchPipelineConfig, BatchStackConfig,
    CalibrationMasters, ChannelInput,
};
use crate::domain::calibration::{
    create_master_bias, create_master_dark, create_master_flat, load_fits_image,
};

#[derive(Debug, Deserialize)]
pub struct ChannelFilesInput {
    pub label: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PipelineRequest {
    pub channels: Vec<ChannelFilesInput>,
    pub dark_paths: Vec<String>,
    pub flat_paths: Vec<String>,
    pub bias_paths: Vec<String>,
    pub sigma_low: Option<f32>,
    pub sigma_high: Option<f32>,
    pub normalize: Option<bool>,
}

fn load_batch(paths: &[String]) -> Result<Vec<ndarray::Array2<f32>>, anyhow::Error> {
    paths.iter().map(|p| load_fits_image(p)).collect()
}

fn array2_to_b64_u16(arr: &ndarray::Array2<f32>) -> String {
    let min_val = arr.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_val = arr.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range = max_val - min_val;

    let mut buf = Vec::with_capacity(arr.len() * 2);
    for &v in arr.iter() {
        let norm = if range > 0.0 {
            ((v - min_val) / range * 65535.0) as u16
        } else {
            0u16
        };
        buf.extend_from_slice(&norm.to_le_bytes());
    }

    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(&buf)
}

fn rgb_to_b64_u8(rgb: &ndarray::Array3<f32>) -> String {
    let (h, w, _) = rgb.dim();
    let mut buf = Vec::with_capacity(h * w * 3);
    for y in 0..h {
        for x in 0..w {
            buf.push((rgb[[y, x, 0]].clamp(0.0, 1.0) * 255.0) as u8);
            buf.push((rgb[[y, x, 1]].clamp(0.0, 1.0) * 255.0) as u8);
            buf.push((rgb[[y, x, 2]].clamp(0.0, 1.0) * 255.0) as u8);
        }
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(&buf)
}

#[tauri::command]
pub async fn run_pipeline_cmd(
    request: PipelineRequest,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let master_bias = if request.bias_paths.is_empty() {
            None
        } else {
            Some(create_master_bias(&request.bias_paths).map_err(|e| format!("{:#}", e))?)
        };

        let master_dark = if request.dark_paths.is_empty() {
            None
        } else {
            Some(create_master_dark(&request.dark_paths, master_bias.as_ref()).map_err(|e| format!("{:#}", e))?)
        };

        let master_flat = if request.flat_paths.is_empty() {
            None
        } else {
            Some(create_master_flat(&request.flat_paths, master_bias.as_ref(), master_dark.as_ref()).map_err(|e| format!("{:#}", e))?)
        };

        let masters = CalibrationMasters {
            dark: master_dark,
            flat: master_flat,
            bias: master_bias,
        };

        let channels: Vec<ChannelInput> = request
            .channels
            .iter()
            .map(|ch| {
                let lights = load_batch(&ch.paths).map_err(|e| format!("{:#}", e))?;
                Ok(ChannelInput {
                    lights,
                    label: ch.label.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let config = BatchPipelineConfig {
            stack: BatchStackConfig {
                sigma_low: request.sigma_low.unwrap_or(2.5),
                sigma_high: request.sigma_high.unwrap_or(3.0),
                max_iterations: 5,
                normalize_before_stack: request.normalize.unwrap_or(true),
            },
        };

        let result = run_batch_pipeline(channels, &masters, &config)?;

        let channel_previews: Vec<serde_json::Value> = result
            .master_channels
            .iter()
            .map(|(label, arr)| {
                let (h, w) = arr.dim();
                json!({
                    "label": label,
                    "pixels_b64": array2_to_b64_u16(arr),
                    "width": w,
                    "height": h,
                })
            })
            .collect();

        let rgb_preview = result.rgb.as_ref().map(|rgb| rgb_to_b64_u8(rgb));

        Ok(json!({
            "stats": result.stats,
            "channel_previews": channel_previews,
            "rgb_preview": rgb_preview,
        }))
    })
    .await
    .map_err(|e| format!("Task panic: {e}"))?
}
