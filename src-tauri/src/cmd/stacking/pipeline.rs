use serde::Deserialize;
use serde_json::json;

use crate::core::imaging::calibration_pipeline::{
    lights_with_dq_planes, run_batch_pipeline, BatchPipelineConfig, BatchPipelineResult,
    BatchPipelineStats, BatchStackConfig, CalibrationMasters, ChannelInput, DQ_COSMETIC_WARNING,
};
use crate::core::imaging::cosmetic::CosmeticConfig;
use crate::core::stacking::calibration::{
    create_master_bias, create_master_dark, create_master_flat, load_fits_image,
};
use crate::infra::fits::reader::read_primary_header;
use crate::types::constants::{
    RES_LABEL, RES_PIXELS_B64, RES_WIDTH, RES_HEIGHT,
    RES_STATS, RES_CHANNEL_PREVIEWS, RES_RGB_PREVIEW, RES_WARNINGS,
};

const PIPELINE_PREVIEW_DIM: usize = 2048;

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
    pub rejection: Option<String>,
    pub combine: Option<String>,
    #[serde(default)]
    pub cosmetic: Option<CosmeticConfig>,
}

fn load_batch(paths: &[String]) -> Result<Vec<ndarray::Array2<f32>>, anyhow::Error> {
    paths.iter().map(|p| load_fits_image(p)).collect()
}

fn pipeline_warnings(channels: &[ChannelFilesInput], cosmetic_enabled: bool) -> Vec<String> {
    if !cosmetic_enabled {
        return Vec::new();
    }
    let flagged = lights_with_dq_planes(channels.iter().flat_map(|ch| ch.paths.iter()));
    if flagged.is_empty() {
        Vec::new()
    } else {
        vec![DQ_COSMETIC_WARNING.to_string()]
    }
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

fn preview_stride(height: usize, width: usize, max_dim: usize) -> usize {
    let largest = height.max(width);
    if largest <= max_dim {
        1
    } else {
        largest.div_ceil(max_dim)
    }
}

fn array2_to_b64_u16(arr: &ndarray::Array2<f32>) -> (String, usize, usize) {
    let (full_h, full_w) = arr.dim();
    let step = preview_stride(full_h, full_w, PIPELINE_PREVIEW_DIM);
    let view = arr.slice(ndarray::s![..;step, ..;step]);
    let (h, w) = view.dim();

    let min_val = view.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_val = view.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range = max_val - min_val;

    let mut buf = Vec::with_capacity(h * w * 2);
    for &v in view.iter() {
        let norm = if range > 0.0 {
            ((v - min_val) / range * 65535.0) as u16
        } else {
            0u16
        };
        buf.extend_from_slice(&norm.to_le_bytes());
    }

    use base64::Engine;
    (base64::engine::general_purpose::STANDARD.encode(&buf), w, h)
}

fn rgb_to_b64_u8(rgb: &ndarray::Array3<f32>) -> String {
    let (full_h, full_w, _) = rgb.dim();
    let step = preview_stride(full_h, full_w, PIPELINE_PREVIEW_DIM);
    let view = rgb.slice(ndarray::s![..;step, ..;step, ..]);
    let (h, w, _) = view.dim();
    let mut buf = Vec::with_capacity(h * w * 3);
    for y in 0..h {
        for x in 0..w {
            buf.push((view[[y, x, 0]].clamp(0.0, 1.0) * 255.0) as u8);
            buf.push((view[[y, x, 1]].clamp(0.0, 1.0) * 255.0) as u8);
            buf.push((view[[y, x, 2]].clamp(0.0, 1.0) * 255.0) as u8);
        }
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(&buf)
}

fn compose_rgb_from_stacked_masters(
    master_channels: Vec<(String, ndarray::Array2<f32>)>,
) -> Result<BatchPipelineResult, String> {
    let channels: Vec<ChannelInput> = master_channels
        .into_iter()
        .map(|(label, master)| ChannelInput {
            lights: vec![master],
            label,
            dark_scales: vec![1.0],
        })
        .collect();
    let no_masters = CalibrationMasters {
        dark: None,
        flat: None,
        bias: None,
    };
    let passthrough = BatchPipelineConfig {
        stack: BatchStackConfig {
            normalize_before_stack: false,
            ..BatchStackConfig::default()
        },
        align: false,
        cosmetic: None,
    };
    run_batch_pipeline(channels, &no_masters, &passthrough)
}

fn stack_channels_one_at_a_time<F>(
    specs: &[ChannelFilesInput],
    mut load_channel: F,
    masters: &CalibrationMasters,
    config: &BatchPipelineConfig,
) -> Result<BatchPipelineResult, String>
where
    F: FnMut(&ChannelFilesInput) -> Result<ChannelInput, String>,
{
    if specs.is_empty() {
        return Err("No channels provided".into());
    }

    let mut master_channels = Vec::with_capacity(specs.len());
    let mut channel_stats = Vec::with_capacity(specs.len());
    for spec in specs {
        let channel = load_channel(spec)?;
        let mut single = run_batch_pipeline(vec![channel], masters, config)?;
        master_channels.append(&mut single.master_channels);
        channel_stats.append(&mut single.stats.channels);
    }

    let stats = BatchPipelineStats {
        darks_combined: if masters.dark.is_some() { 1 } else { 0 },
        flats_combined: if masters.flat.is_some() { 1 } else { 0 },
        bias_combined: if masters.bias.is_some() { 1 } else { 0 },
        channels: channel_stats,
    };

    let composed = compose_rgb_from_stacked_masters(master_channels)?;
    Ok(BatchPipelineResult {
        master_channels: composed.master_channels,
        rgb: composed.rgb,
        stats,
    })
}

#[tauri::command]
pub async fn run_pipeline_cmd(
    request: PipelineRequest,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let rejection = crate::cmd::helpers::parse_rejection_method(request.rejection.as_deref())
            .map_err(|e| format!("{:#}", e))?;
        let combine = crate::cmd::helpers::parse_combine_method(request.combine.as_deref())
            .map_err(|e| format!("{:#}", e))?;

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

        let load_channel = |ch: &ChannelFilesInput| -> Result<ChannelInput, String> {
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
        };

        let config = BatchPipelineConfig {
            stack: BatchStackConfig {
                sigma_low: request.sigma_low.unwrap_or(2.5),
                sigma_high: request.sigma_high.unwrap_or(3.0),
                max_iterations: 5,
                normalize_before_stack: request.normalize.unwrap_or(true),
                rejection,
                combine,
            },
            align: request.align.unwrap_or(true),
            cosmetic: request.cosmetic.clone(),
        };

        let warnings = pipeline_warnings(&request.channels, config.cosmetic.is_some());
        let result = stack_channels_one_at_a_time(&request.channels, load_channel, &masters, &config)?;

        let channel_previews: Vec<serde_json::Value> = result
            .master_channels
            .iter()
            .map(|(label, arr)| {
                let (b64, w, h) = array2_to_b64_u16(arr);
                json!({
                    RES_LABEL: label,
                    RES_PIXELS_B64: b64,
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
            RES_WARNINGS: warnings,
        }))
    })
        .await
        .map_err(|e| format!("Task panic: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ndarray::{Array2, Array3};
    use std::cell::Cell;

    fn spec(label: &str, n: usize) -> ChannelFilesInput {
        ChannelFilesInput {
            label: label.to_string(),
            paths: (0..n).map(|i| format!("{label}_{i}.fits")).collect(),
        }
    }

    fn synthetic_light(seed: f32, index: usize) -> Array2<f32> {
        let mut a = Array2::from_elem((12, 12), 10.0 + seed);
        for y in 0..12 {
            for x in 0..12 {
                a[[y, x]] += ((x * 7 + y * 3 + index) % 5) as f32;
            }
        }
        a[[5, 5]] = 500.0 + seed;
        a
    }

    fn synthetic_channel(spec: &ChannelFilesInput) -> ChannelInput {
        let seed = match spec.label.as_str() {
            "R" => 1.0,
            "G" => 2.0,
            _ => 3.0,
        };
        ChannelInput {
            lights: (0..spec.paths.len()).map(|i| synthetic_light(seed, i)).collect(),
            label: spec.label.clone(),
            dark_scales: vec![1.0; spec.paths.len()],
        }
    }

    fn test_config() -> BatchPipelineConfig {
        BatchPipelineConfig {
            stack: BatchStackConfig::default(),
            align: false,
            cosmetic: None,
        }
    }

    #[test]
    fn pipeline_request_accepts_a_partial_cosmetic_object_and_defaults_to_off() {
        let with: PipelineRequest = serde_json::from_str(
            r#"{"channels":[],"dark_paths":[],"flat_paths":[],"bias_paths":[],"cosmetic":{"use_master_dark":true,"dark_hot_sigma":5}}"#,
        )
        .unwrap();
        let cosmetic = with.cosmetic.expect("cosmetic parsed");
        assert!(cosmetic.use_master_dark);
        assert_eq!(cosmetic.dark_hot_sigma, Some(5.0));
        assert!(cosmetic.defects.is_empty());

        let without: PipelineRequest =
            serde_json::from_str(r#"{"channels":[],"dark_paths":[],"flat_paths":[],"bias_paths":[]}"#).unwrap();
        assert!(without.cosmetic.is_none());
    }

    #[test]
    fn pipeline_warnings_flag_dq_planes_only_when_cosmetic_is_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mef = dir.path().join("light_dq.fits");
        crate::infra::fits::reader::test_fixtures::sci_err_dq_mef(&mef, 4, 4, vec![0i32; 16]);
        let plain = dir.path().join("light_plain.fits");
        crate::infra::fits::writer::write_fits_mono(plain.to_str().unwrap(), &Array2::from_elem((4, 4), 1.0f32), None)
            .unwrap();

        let dq_channel = vec![ChannelFilesInput {
            label: "L".into(),
            paths: vec![plain.to_str().unwrap().to_string(), mef.to_str().unwrap().to_string()],
        }];
        assert_eq!(pipeline_warnings(&dq_channel, true), vec![DQ_COSMETIC_WARNING.to_string()]);
        assert!(pipeline_warnings(&dq_channel, false).is_empty());

        let plain_channel = vec![ChannelFilesInput { label: "L".into(), paths: vec![plain.to_str().unwrap().to_string()] }];
        assert!(pipeline_warnings(&plain_channel, true).is_empty());
    }

    fn test_masters() -> CalibrationMasters {
        CalibrationMasters {
            dark: None,
            flat: None,
            bias: Some(Array2::from_elem((12, 12), 1.0)),
        }
    }

    fn assert_close(a: &Array2<f32>, b: &Array2<f32>, what: &str) {
        assert_eq!(a.dim(), b.dim(), "{what}: shape mismatch");
        for (va, vb) in a.iter().zip(b.iter()) {
            assert!((va - vb).abs() <= 1e-4 * va.abs().max(1.0), "{what}: {va} vs {vb}");
        }
    }

    #[test]
    fn per_channel_stacking_matches_single_batch_run() {
        let specs = vec![spec("R", 3), spec("G", 3), spec("B", 3)];
        let masters = test_masters();
        let config = test_config();

        let sequential = stack_channels_one_at_a_time(&specs, |s| Ok(synthetic_channel(s)), &masters, &config)
            .expect("sequential run");
        let batch = run_batch_pipeline(specs.iter().map(synthetic_channel).collect(), &masters, &config)
            .expect("batch run");

        assert_eq!(sequential.master_channels.len(), 3);
        for ((ls, ms), (lb, mb)) in sequential.master_channels.iter().zip(batch.master_channels.iter()) {
            assert_eq!(ls, lb);
            assert_close(ms, mb, &format!("master {ls}"));
        }

        let rgb_s = sequential.rgb.expect("sequential rgb");
        let rgb_b = batch.rgb.expect("batch rgb");
        assert_eq!(rgb_s.dim(), rgb_b.dim());
        for (a, b) in rgb_s.iter().zip(rgb_b.iter()) {
            assert!((a - b).abs() <= 1e-4, "rgb {a} vs {b}");
        }

        assert_eq!(sequential.stats.bias_combined, batch.stats.bias_combined);
        assert_eq!(sequential.stats.darks_combined, batch.stats.darks_combined);
        assert_eq!(sequential.stats.flats_combined, batch.stats.flats_combined);
        assert_eq!(sequential.stats.channels.len(), 3);
        for (cs, cb) in sequential.stats.channels.iter().zip(batch.stats.channels.iter()) {
            assert_eq!(cs.label, cb.label);
            assert_eq!(cs.lights_input, cb.lights_input);
            assert_eq!(cs.lights_input, 3);
            assert_eq!(cs.lights_after_rejection, cb.lights_after_rejection);
            assert!((cs.mean - cb.mean).abs() < 1e-6);
        }
    }

    #[test]
    fn per_channel_stacking_loads_one_channel_per_call_and_stops_on_error() {
        let specs = vec![spec("R", 2), spec("G", 2), spec("B", 2)];
        let calls = Cell::new(0usize);
        let err = stack_channels_one_at_a_time(
            &specs,
            |s| {
                calls.set(calls.get() + 1);
                if s.label == "G" {
                    Err("boom".to_string())
                } else {
                    Ok(synthetic_channel(s))
                }
            },
            &test_masters(),
            &test_config(),
        )
        .unwrap_err();
        assert_eq!(err, "boom");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn per_channel_stacking_rejects_empty_request() {
        let err = stack_channels_one_at_a_time(&[], |s| Ok(synthetic_channel(s)), &test_masters(), &test_config())
            .unwrap_err();
        assert_eq!(err, "No channels provided");
    }

    #[test]
    fn compose_pass_leaves_masters_untouched() {
        let masters = vec![
            ("R".to_string(), synthetic_light(1.0, 0)),
            ("G".to_string(), synthetic_light(2.0, 1)),
            ("B".to_string(), synthetic_light(3.0, 2)),
        ];
        let expected: Vec<_> = masters.clone();
        let composed = compose_rgb_from_stacked_masters(masters).expect("compose");
        assert!(composed.rgb.is_some());
        for ((le, me), (lc, mc)) in expected.iter().zip(composed.master_channels.iter()) {
            assert_eq!(le, lc);
            assert_close(me, mc, &format!("master {le}"));
        }
    }

    #[test]
    fn preview_stride_caps_largest_dimension() {
        assert_eq!(preview_stride(100, 200, 2048), 1);
        assert_eq!(preview_stride(2048, 10, 2048), 1);
        assert_eq!(preview_stride(2049, 10, 2048), 2);
        assert_eq!(preview_stride(10, 4096, 2048), 2);
        assert_eq!(preview_stride(6000, 4000, 2048), 3);
    }

    #[test]
    fn channel_preview_is_downsampled_before_encoding() {
        let w = 5000usize;
        let mut arr = Array2::<f32>::zeros((2, w));
        for x in 0..w {
            arr[[0, x]] = x as f32;
            arr[[1, x]] = x as f32;
        }
        let (b64, pw, ph) = array2_to_b64_u16(&arr);
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).expect("valid base64");
        assert_eq!(pw, 1667);
        assert_eq!(ph, 1);
        assert_eq!(bytes.len(), pw * ph * 2);
        assert!(bytes.len() < w * 2 * 2);
        let first = u16::from_le_bytes([bytes[0], bytes[1]]);
        let last = u16::from_le_bytes([bytes[bytes.len() - 2], bytes[bytes.len() - 1]]);
        assert_eq!(first, 0);
        assert_eq!(last, 65535);
    }

    #[test]
    fn rgb_preview_is_downsampled_with_same_stride_as_channels() {
        let w = 5000usize;
        let mut rgb = Array3::<f32>::zeros((1, w, 3));
        for x in 0..w {
            rgb[[0, x, 0]] = if x % 3 == 0 { 1.0 } else { 0.0 };
        }
        let b64 = rgb_to_b64_u8(&rgb);
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).expect("valid base64");
        let (_, pw, ph) = array2_to_b64_u16(&Array2::<f32>::zeros((1, w)));
        assert_eq!(bytes.len(), pw * ph * 3);
        assert!(bytes.iter().step_by(3).all(|&r| r == 255));
    }
}
