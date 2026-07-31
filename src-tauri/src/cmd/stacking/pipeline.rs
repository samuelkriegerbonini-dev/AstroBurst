use serde::Deserialize;
use serde_json::json;

use crate::core::imaging::calibration_pipeline::{
    run_batch_pipeline, BatchPipelineConfig, BatchStackConfig,
    CalibrationMasters, ChannelInput,
};
use crate::core::stacking::calibration::{
    create_master_bias, create_master_dark, create_master_flat, load_fits_image,
};
use crate::infra::fits::reader::read_primary_header;
use crate::types::constants::{
    RES_LABEL, RES_PIXELS_B64, RES_WIDTH, RES_HEIGHT,
    RES_STATS, RES_CHANNEL_PREVIEWS, RES_RGB_PREVIEW,
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
    pub align: Option<bool>,
}

fn load_batch(paths: &[String]) -> Result<Vec<ndarray::Array2<f32>>, anyhow::Error> {
    paths.iter().map(|p| load_fits_image(p)).collect()
}

fn read_exposure_seconds(path: &str) -> Option<f64> {
    let header = read_primary_header(path).ok()?;
    header
        .get_f64("EXPTIME")
        .or_else(|| header.get_f64("EXPOSURE"))
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn median_exposure(paths: &[String]) -> Option<f64> {
    let mut vals: Vec<f64> = paths.iter().filter_map(|p| read_exposure_seconds(p)).collect();
    if vals.is_empty() {
        return None;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(vals[vals.len() / 2])
}

fn compute_dark_scales(light_paths: &[String], dark_exposure: Option<f64>) -> Vec<f32> {
    let Some(dark_exp) = dark_exposure else {
        return vec![1.0; light_paths.len()];
    };
    light_paths
        .iter()
        .map(|p| match read_exposure_seconds(p) {
            Some(light_exp) => {
                let ratio = (light_exp / dark_exp) as f32;
                if (ratio - 1.0).abs() < 0.01 {
                    1.0
                } else {
                    ratio.clamp(0.05, 20.0)
                }
            }
            None => 1.0,
        })
        .collect()
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
            Some(create_master_flat(&request.flat_paths, master_bias.as_ref(), master_dark.as_ref(), median_exposure(&request.dark_paths)).map_err(|e| format!("{:#}", e))?)
        };

        let dark_exposure = if request.dark_paths.is_empty() || master_bias.is_none() {
            None
        } else {
            median_exposure(&request.dark_paths)
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

                if lights.len() > 1 {
                    let ref_dim = lights[0].dim();
                    for (i, l) in lights.iter().enumerate().skip(1) {
                        if l.dim() != ref_dim {
                            return Err(format!(
                                "Channel '{}': frame {} has shape {:?} but frame 0 has {:?}. All frames must match.",
                                ch.label, i, l.dim(), ref_dim
                            ));
                        }
                    }
                }

                let dark_scales = compute_dark_scales(&ch.paths, dark_exposure);
                if dark_scales.iter().any(|s| (*s - 1.0).abs() >= 0.01) {
                    let min_s = dark_scales.iter().cloned().fold(f32::INFINITY, f32::min);
                    let max_s = dark_scales.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                    log::info!(
                        "Channel '{}': scaling master dark by exposure ratio (range {:.3}..{:.3}, dark median {:.1}s)",
                        ch.label, min_s, max_s, dark_exposure.unwrap_or(0.0)
                    );
                }

                Ok(ChannelInput {
                    lights,
                    label: ch.label.clone(),
                    dark_scales,
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
            align: request.align.unwrap_or(true),
        };

        let result = run_batch_pipeline(channels, &masters, &config)?;

        let channel_previews: Vec<serde_json::Value> = result
            .master_channels
            .iter()
            .map(|(label, arr)| {
                let (h, w) = arr.dim();
                json!({
                    RES_LABEL: label,
                    RES_PIXELS_B64: array2_to_b64_u16(arr),
                    RES_WIDTH: w,
                    RES_HEIGHT: h,
                })
            })
            .collect();

        let rgb_preview = result.rgb.as_ref().map(|rgb| rgb_to_b64_u8(rgb));

        Ok(json!({
            RES_STATS: result.stats,
            RES_CHANNEL_PREVIEWS: channel_previews,
            RES_RGB_PREVIEW: rgb_preview,
        }))
    })
        .await
        .map_err(|e| format!("Task panic: {e}"))?
}
