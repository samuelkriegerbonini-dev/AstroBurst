use serde::Deserialize;
use serde_json::json;

use super::cards::{stack_output_cards, StackCards};
use super::combine::{output_name, reference_header, ABPROC_STACKED};
use crate::cmd::common::{derived_output_header, render_named_and_save, resolve_output_dir, OutputValues};
use crate::core::imaging::calibration_pipeline::{
    lights_with_dq_planes, run_batch_pipeline, validate_cosmetic_config, BatchPipelineConfig,
    BatchPipelineResult, BatchPipelineStats, BatchStackConfig, CalibrationMasters, ChannelInput,
    DQ_COSMETIC_WARNING,
};
use crate::core::imaging::cosmetic::CosmeticConfig;
use crate::core::stacking::calibration::load_fits_image;
use crate::core::stacking::cfa_guard::{refuse_cfa_frames, CfaStep};
use crate::core::stacking::frame_cards::{read_frame_cards, FrameCards};
use crate::core::stacking::masters::{
    assign_dark_groups, build_masters_from_frames, check_channel_lights, read_master_frames, MasterRequest,
};
use crate::core::stacking::never_cancelled;
use crate::types::constants::{
    RES_CHANNEL_PREVIEWS, RES_DIMENSIONS, RES_FITS_PATH, RES_HEIGHT, RES_INPUT_PATH, RES_LABEL, RES_MASTERS,
    RES_PIXELS_B64, RES_PNG_PATH, RES_RGB_DIMENSIONS, RES_RGB_PNG_PATH, RES_RGB_PREVIEW, RES_STATS, RES_WARNINGS,
    RES_WIDTH,
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
    #[serde(default)]
    pub flat_dark_paths: Vec<String>,
    pub sigma_low: Option<f32>,
    pub sigma_high: Option<f32>,
    pub normalize: Option<bool>,
    pub align: Option<bool>,
    pub rejection: Option<String>,
    pub combine: Option<String>,
    #[serde(default)]
    pub cosmetic: Option<CosmeticConfig>,
    #[serde(default)]
    pub dark_optimize: bool,
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

fn dark_scales_for_cards(lights: &[FrameCards], dark_exposure: Option<f64>) -> Vec<f32> {
    let Some(dark_exp) = dark_exposure else {
        return vec![1.0; lights.len()];
    };
    lights
        .iter()
        .map(|card| match card.exposure_s {
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

fn rgb_preview_bytes(rgb: &ndarray::Array3<f32>) -> (Vec<u8>, usize, usize) {
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
    (buf, w, h)
}

struct MasterOutput {
    label: String,
    png_path: String,
    fits_path: String,
    width: usize,
    height: usize,
    input_path: String,
}

struct RgbOutput {
    png_path: String,
    preview_b64: String,
    width: usize,
    height: usize,
}

struct PipelineOutputs {
    masters: Vec<MasterOutput>,
    rgb: Option<RgbOutput>,
}

const NORMALIZATION_FRAME: &str = "frame";
const NORMALIZATION_NONE: &str = "none";

fn save_master(
    label: &str,
    arr: &ndarray::Array2<f32>,
    spec: &ChannelFilesInput,
    output_dir: &str,
    stem: &str,
    stack: &BatchStackConfig,
    excluded: &[usize],
) -> anyhow::Result<MasterOutput> {
    let mut header = derived_output_header(reference_header(&spec.paths).as_ref(), ABPROC_STACKED, OutputValues::Linear);
    let included: Vec<&str> = spec
        .paths
        .iter()
        .enumerate()
        .filter(|(i, _)| !excluded.contains(i))
        .map(|(_, p)| p.as_str())
        .collect();
    stack_output_cards(
        &mut header,
        &StackCards {
            included_paths: &included,
            combine: stack.combine,
            rejection: stack.rejection.name(),
            normalization: Some(if stack.normalize_before_stack { NORMALIZATION_FRAME } else { NORMALIZATION_NONE }),
            drizzle: None,
        },
    );
    let name = format!("{stem}_{}", output_name(Some(label), "channel"));
    let (png_path, fits_path) = render_named_and_save(arr, output_dir, &name, true, Some(&header))?;
    let fits_path = fits_path.ok_or_else(|| anyhow::anyhow!("Master '{label}' FITS was not written"))?;
    let (rows, cols) = arr.dim();
    Ok(MasterOutput {
        label: label.to_string(),
        png_path,
        fits_path,
        width: cols,
        height: rows,
        input_path: spec.paths.first().cloned().unwrap_or_default(),
    })
}

fn save_rgb_preview(rgb: &ndarray::Array3<f32>, output_dir: &str, stem: &str) -> anyhow::Result<RgbOutput> {
    let (bytes, width, height) = rgb_preview_bytes(rgb);
    let png_path = format!("{output_dir}/{stem}_rgb.png");
    image::save_buffer(&png_path, &bytes, width as u32, height as u32, image::ColorType::Rgb8)
        .map_err(|e| anyhow::anyhow!("Failed to save RGB PNG: {e}"))?;
    use base64::Engine;
    let preview_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(RgbOutput { png_path, preview_b64, width, height })
}

fn save_pipeline_outputs(
    result: &BatchPipelineResult,
    specs: &[ChannelFilesInput],
    output_dir: &str,
    stem: &str,
    stack: &BatchStackConfig,
) -> anyhow::Result<PipelineOutputs> {
    if result.master_channels.len() != specs.len() || result.stats.channels.len() != specs.len() {
        anyhow::bail!(
            "Pipeline produced {} masters and {} channel stats for {} channels",
            result.master_channels.len(),
            result.stats.channels.len(),
            specs.len()
        );
    }
    let masters = result
        .master_channels
        .iter()
        .zip(specs)
        .zip(&result.stats.channels)
        .map(|(((label, arr), spec), stats)| save_master(label, arr, spec, output_dir, stem, stack, &stats.excluded_frames))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let rgb = result.rgb.as_ref().map(|rgb| save_rgb_preview(rgb, output_dir, stem)).transpose()?;
    Ok(PipelineOutputs { masters, rgb })
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
            cfa: false,
            dark_group: Vec::new(),
            light_temp_c: Vec::new(),
        })
        .collect();
    let no_masters = CalibrationMasters::default();
    let passthrough = BatchPipelineConfig {
        stack: BatchStackConfig {
            normalize_before_stack: false,
            ..BatchStackConfig::default()
        },
        align: false,
        cosmetic: None,
        dark_optimize: false,
    };
    run_batch_pipeline(channels, &no_masters, &passthrough)
}

#[derive(Debug, Clone, Copy, Default)]
struct MasterFrameCounts {
    darks: usize,
    flats: usize,
    bias: usize,
    flat_darks: usize,
}

impl MasterFrameCounts {
    fn of(request: &PipelineRequest) -> Self {
        Self {
            darks: request.dark_paths.len(),
            flats: request.flat_paths.len(),
            bias: request.bias_paths.len(),
            flat_darks: request.flat_dark_paths.len(),
        }
    }
}

fn stack_channels_one_at_a_time<F>(
    specs: &[ChannelFilesInput],
    mut load_channel: F,
    masters: &CalibrationMasters,
    counts: MasterFrameCounts,
    config: &BatchPipelineConfig,
) -> Result<BatchPipelineResult, String>
where
    F: FnMut(usize, &ChannelFilesInput) -> Result<ChannelInput, String>,
{
    if specs.is_empty() {
        return Err("No channels provided".into());
    }

    let mut master_channels = Vec::with_capacity(specs.len());
    let mut channel_stats = Vec::with_capacity(specs.len());
    let mut dark_groups = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        let channel = load_channel(index, spec)?;
        let mut single = run_batch_pipeline(vec![channel], masters, config)?;
        master_channels.append(&mut single.master_channels);
        channel_stats.append(&mut single.stats.channels);
        dark_groups = single.stats.dark_groups;
    }

    let stats = BatchPipelineStats {
        darks_combined: counts.darks,
        flats_combined: counts.flats,
        bias_combined: counts.bias,
        flat_darks_combined: counts.flat_darks,
        dark_groups,
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
    output_dir: String,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || run_pipeline_request(request, &output_dir, name.as_deref()))
        .await
        .map_err(|e| format!("Task panic: {e}"))?
}

fn run_pipeline_request(
    request: PipelineRequest,
    output_dir: &str,
    name: Option<&str>,
) -> Result<serde_json::Value, String> {
    let rejection = crate::cmd::helpers::parse_rejection_method(request.rejection.as_deref())
        .map_err(|e| format!("{:#}", e))?;
    let combine = crate::cmd::helpers::parse_combine_method(request.combine.as_deref())
        .map_err(|e| format!("{:#}", e))?;
    if let Some(cosmetic) = &request.cosmetic {
        validate_cosmetic_config(cosmetic)?;
    }
    if request.align.unwrap_or(true) {
        let lights: Vec<String> = request.channels.iter().flat_map(|ch| ch.paths.iter().cloned()).collect();
        refuse_cfa_frames(&lights, CfaStep::Align).map_err(|e| format!("{:#}", e))?;
    }

    let cosmetic_enabled = request.cosmetic.is_some();
    let master_request = MasterRequest {
        bias: &request.bias_paths,
        darks: &request.dark_paths,
        flats: &request.flat_paths,
        flat_darks: &request.flat_dark_paths,
    };
    let master_frames = read_master_frames(&master_request);
    let channel_cards: Vec<Vec<FrameCards>> = request
        .channels
        .iter()
        .map(|ch| ch.paths.iter().map(|p| read_frame_cards(p)).collect())
        .collect();
    let mut warnings = pipeline_warnings(&request.channels, cosmetic_enabled);
    for (ch, cards) in request.channels.iter().zip(&channel_cards) {
        warnings.extend(
            check_channel_lights(&master_frames, &ch.label, cards, cosmetic_enabled).map_err(|e| format!("{:#}", e))?,
        );
    }

    let (masters, report) = build_masters_from_frames(&master_request, &master_frames, &never_cancelled)
        .map_err(|e| format!("{:#}", e))?;
    warnings.extend(report.warnings);
    let dark_exposure = report.dark_exposure_s.filter(|_| masters.bias.is_some());

    let mut channel_warnings: Vec<String> = Vec::new();
    let load_channel = |index: usize, ch: &ChannelFilesInput| -> Result<ChannelInput, String> {
        let cards = &channel_cards[index];
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

        let dark_scales = dark_scales_for_cards(cards, dark_exposure);
        if dark_scales.iter().any(|s| (*s - 1.0).abs() >= 0.01) {
            let min_s = dark_scales.iter().cloned().fold(f32::INFINITY, f32::min);
            let max_s = dark_scales.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            log::info!(
                "Channel '{}': scaling master dark by exposure ratio (range {:.3}..{:.3}, dark median {:.1}s)",
                ch.label, min_s, max_s, dark_exposure.unwrap_or(0.0)
            );
        }

        let (dark_group, temp_warnings) = assign_dark_groups(&masters.dark_groups, cards, &ch.label);
        channel_warnings.extend(temp_warnings);

        Ok(ChannelInput {
            lights,
            label: ch.label.clone(),
            dark_scales,
            cfa: cards.iter().any(|c| c.cfa),
            dark_group,
            light_temp_c: cards.iter().map(|c| c.temp_c).collect(),
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
        dark_optimize: request.dark_optimize,
    };

    let counts = MasterFrameCounts::of(&request);
    let result = stack_channels_one_at_a_time(&request.channels, load_channel, &masters, counts, &config)?;
    warnings.extend(channel_warnings);
    warnings.extend(result.stats.channels.iter().flat_map(|c| c.warnings.iter().cloned()));

    let output_dir = resolve_output_dir(output_dir).map_err(|e| format!("{:#}", e))?;
    let stem = output_name(name, "pipeline");
    let outputs = save_pipeline_outputs(&result, &request.channels, &output_dir, &stem, &config.stack)
        .map_err(|e| format!("{:#}", e))?;

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

    let masters_json: Vec<serde_json::Value> = outputs
        .masters
        .iter()
        .map(|m| {
            json!({
                RES_LABEL: m.label,
                RES_PNG_PATH: m.png_path,
                RES_FITS_PATH: m.fits_path,
                RES_DIMENSIONS: [m.width, m.height],
                RES_INPUT_PATH: m.input_path,
            })
        })
        .collect();
    let rgb = outputs.rgb.as_ref();

    Ok(json!({
        RES_STATS: result.stats,
        RES_CHANNEL_PREVIEWS: channel_previews,
        RES_RGB_PREVIEW: rgb.map(|o| &o.preview_b64),
        RES_WARNINGS: warnings,
        RES_MASTERS: masters_json,
        RES_RGB_PNG_PATH: rgb.map(|o| &o.png_path),
        RES_RGB_DIMENSIONS: rgb.map(|o| [o.width, o.height]),
    }))
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
            cfa: false,
            dark_group: Vec::new(),
            light_temp_c: Vec::new(),
        }
    }

    fn test_config() -> BatchPipelineConfig {
        BatchPipelineConfig {
            stack: BatchStackConfig::default(),
            align: false,
            cosmetic: None,
            dark_optimize: false,
        }
    }

    #[test]
    fn pipeline_request_accepts_a_partial_cosmetic_object_and_defaults_to_off() {
        let with: PipelineRequest = serde_json::from_str(
            r#"{"channels":[],"dark_paths":[],"flat_paths":[],"bias_paths":[],"cosmetic":{"use_master_dark":true,"dark_hot_sigma":5},"dark_optimize":true}"#,
        )
        .unwrap();
        let cosmetic = with.cosmetic.expect("cosmetic parsed");
        assert!(cosmetic.use_master_dark);
        assert_eq!(cosmetic.dark_hot_sigma, Some(5.0));
        assert!(cosmetic.defects.is_empty());
        assert!(with.dark_optimize);

        let without: PipelineRequest =
            serde_json::from_str(r#"{"channels":[],"dark_paths":[],"flat_paths":[],"bias_paths":[]}"#).unwrap();
        assert!(without.cosmetic.is_none());
        assert!(!without.dark_optimize);
    }

    #[test]
    fn pipeline_request_accepts_flat_dark_paths_and_defaults_to_empty() {
        let with: PipelineRequest = serde_json::from_str(
            r#"{"channels":[],"dark_paths":[],"flat_paths":[],"bias_paths":[],"flat_dark_paths":["C:/cal/fd0.fits","C:/cal/fd1.fits"]}"#,
        )
        .unwrap();
        assert_eq!(with.flat_dark_paths, vec!["C:/cal/fd0.fits".to_string(), "C:/cal/fd1.fits".to_string()]);

        let without: PipelineRequest =
            serde_json::from_str(r#"{"channels":[],"dark_paths":[],"flat_paths":[],"bias_paths":[]}"#).unwrap();
        assert!(without.flat_dark_paths.is_empty());
    }

    fn write_exposure_frames(dir: &tempfile::TempDir, prefix: &str, count: usize, level: f32, cards: &[(&str, &str)]) -> Vec<String> {
        let mut header = crate::types::header::HduHeader::empty();
        for (key, value) in cards {
            header.set(key, (*value).to_string());
        }
        (0..count)
            .map(|i| {
                let path = dir.path().join(format!("{prefix}{i}.fits")).to_str().unwrap().to_string();
                let frame = Array2::from_shape_fn((12, 12), |(y, x)| level + ((x * 7 + y * 3 + i) % 5) as f32);
                crate::infra::fits::writer::write_fits_mono(&path, &frame, Some(&header)).unwrap();
                path
            })
            .collect()
    }

    fn calibrated_request(dir: &tempfile::TempDir, darks: &[(f32, &[(&str, &str)])]) -> PipelineRequest {
        let mut dark_paths = Vec::new();
        for (i, (level, cards)) in darks.iter().enumerate() {
            dark_paths.extend(write_exposure_frames(dir, &format!("dark{i}_"), 1, *level, cards));
        }
        PipelineRequest {
            channels: vec![ChannelFilesInput {
                label: "R".into(),
                paths: write_exposure_frames(dir, "light", 2, 500.0, &[("EXPTIME", "120"), ("CCD-TEMP", "-9.5")]),
            }],
            dark_paths,
            flat_paths: write_exposure_frames(dir, "flat", 2, 1000.0, &[("EXPTIME", "2")]),
            bias_paths: vec![],
            flat_dark_paths: write_exposure_frames(dir, "flatdark", 2, 15.0, &[("EXPTIME", "2")]),
            sigma_low: None,
            sigma_high: None,
            normalize: Some(false),
            align: Some(false),
            rejection: Some("none".into()),
            combine: None,
            cosmetic: None,
            dark_optimize: false,
        }
    }

    #[test]
    fn pipeline_reports_flat_darks_dark_groups_and_consistency_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let cold: (f32, &[(&str, &str)]) = (20.0, &[("EXPTIME", "300"), ("CCD-TEMP", "-20.0")]);
        let warm: (f32, &[(&str, &str)]) = (60.0, &[("EXPTIME", "300"), ("CCD-TEMP", "-10.0")]);
        let request = calibrated_request(&dir, &[cold, cold, cold, cold, cold, warm, warm, warm, warm, warm]);
        let response = run_pipeline_request(request, &out_dir(&dir), None).unwrap();

        let stats = &response[RES_STATS];
        assert_eq!(stats["darks_combined"], 10, "{stats}");
        assert_eq!(stats["flat_darks_combined"], 2, "{stats}");
        assert_eq!(stats["dark_groups"], json!([{ "temp_c": -20.0, "frames": 5 }, { "temp_c": -10.0, "frames": 5 }]), "{stats}");
        assert_eq!(stats["channels"][0]["dark_group_temp_c"], -10.0, "{stats}");
        assert!((stats["channels"][0]["dark_temp_delta_max_c"].as_f64().unwrap() - 0.5).abs() < 1e-9, "{stats}");

        let warnings: Vec<&str> = response[RES_WARNINGS].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].starts_with("Channel 'R': lights are 120 s but the darks are 300 s and there is no master bias"),
            "{}",
            warnings[0]
        );

        let master = crate::infra::fits::reader::load_fits_image(response[RES_MASTERS][0][RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert_eq!(master.dim(), (12, 12));
        let mut values: Vec<f32> = master.iter().copied().collect();
        values.sort_by(f32::total_cmp);
        let median = values[values.len() / 2];
        assert!(
            (median - 440.0).abs() < 5.0,
            "master median {median}: the -9.5 C lights must get the -10 C dark group (level 60), not the -20 C group (level 20)"
        );
    }

    #[test]
    fn pipeline_refuses_mixed_dark_exposures_before_building_masters() {
        let dir = tempfile::tempdir().unwrap();
        let request = calibrated_request(&dir, &[(20.0, &[("EXPTIME", "60")]), (20.0, &[("EXPTIME", "300")])]);
        let out = out_dir(&dir);
        let err = run_pipeline_request(request, &out, Some("mixed")).unwrap_err();
        assert!(err.starts_with("Darks have mixed exposures (60 s to 300 s)"), "{err}");
        assert!(err.contains("dark0_0.fits") && err.contains("dark1_0.fits"), "{err}");
        assert!(!std::path::Path::new(&out).exists(), "the output dir was created before the consistency check");
    }

    #[test]
    fn invalid_cosmetic_config_fails_before_any_master_is_built() {
        let dir = tempfile::tempdir().unwrap();
        let missing_bias = dir.path().join("missing_bias.fits");
        let request = PipelineRequest {
            channels: vec![spec("R", 2)],
            dark_paths: vec![],
            flat_paths: vec![],
            bias_paths: vec![missing_bias.to_str().unwrap().to_string()],
            flat_dark_paths: vec![],
            sigma_low: None,
            sigma_high: None,
            normalize: None,
            align: None,
            rejection: None,
            combine: None,
            cosmetic: Some(CosmeticConfig { amount: 1.5, ..Default::default() }),
            dark_optimize: false,
        };

        let out = out_dir(&dir);
        let err = run_pipeline_request(request, &out, Some("cosmetic")).unwrap_err();
        assert!(err.starts_with("Cosmetic correction"), "{err}");
        assert!(err.contains("Amount"), "{err}");
        assert!(!err.contains("missing_bias"), "{err}");
        assert!(!std::path::Path::new(&out).exists(), "the output dir was created before validation");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
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
            dark_groups: Vec::new(),
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

        let counts = MasterFrameCounts { darks: 0, flats: 0, bias: 7, flat_darks: 0 };
        let sequential = stack_channels_one_at_a_time(&specs, |_, s| Ok(synthetic_channel(s)), &masters, counts, &config)
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

        assert_eq!(sequential.stats.bias_combined, 7);
        assert_eq!(sequential.stats.darks_combined, 0);
        assert_eq!(sequential.stats.flats_combined, 0);
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
            |_, s| {
                calls.set(calls.get() + 1);
                if s.label == "G" {
                    Err("boom".to_string())
                } else {
                    Ok(synthetic_channel(s))
                }
            },
            &test_masters(),
            MasterFrameCounts::default(),
            &test_config(),
        )
        .unwrap_err();
        assert_eq!(err, "boom");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn per_channel_stacking_rejects_empty_request() {
        let err = stack_channels_one_at_a_time(
            &[],
            |_, s| Ok(synthetic_channel(s)),
            &test_masters(),
            MasterFrameCounts::default(),
            &test_config(),
        )
        .unwrap_err();
        assert_eq!(err, "No channels provided");
    }

    fn out_dir(dir: &tempfile::TempDir) -> String {
        dir.path().join("out").to_str().unwrap().replace('\\', "/")
    }

    fn write_frames_sized(dir: &tempfile::TempDir, prefix: &str, count: usize, level: f32, size: usize) -> Vec<String> {
        (0..count)
            .map(|i| {
                let path = dir.path().join(format!("{prefix}{i}.fits")).to_str().unwrap().to_string();
                let frame = Array2::from_shape_fn((size, size), |(y, x)| level + ((x * 7 + y * 3 + i) % 5) as f32);
                crate::infra::fits::writer::write_fits_mono(&path, &frame, None).unwrap();
                path
            })
            .collect()
    }

    fn write_frames(dir: &tempfile::TempDir, prefix: &str, count: usize, level: f32) -> Vec<String> {
        write_frames_sized(dir, prefix, count, level, 12)
    }

    fn rgb_request(dir: &tempfile::TempDir, sizes: [usize; 3]) -> PipelineRequest {
        PipelineRequest {
            channels: vec![
                ChannelFilesInput { label: "R".into(), paths: write_frames_sized(dir, "r", 2, 500.0, sizes[0]) },
                ChannelFilesInput { label: "G".into(), paths: write_frames_sized(dir, "g", 1, 300.0, sizes[1]) },
                ChannelFilesInput { label: "B".into(), paths: write_frames_sized(dir, "b", 1, 200.0, sizes[2]) },
            ],
            dark_paths: vec![],
            flat_paths: vec![],
            bias_paths: vec![],
            flat_dark_paths: vec![],
            sigma_low: None,
            sigma_high: None,
            normalize: None,
            align: Some(false),
            rejection: None,
            combine: None,
            cosmetic: None,
            dark_optimize: false,
        }
    }

    fn decode_b64(value: &serde_json::Value) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode(value.as_str().expect("base64 string")).expect("valid base64")
    }

    fn write_cfa_frames(dir: &tempfile::TempDir, prefix: &str, count: usize) -> Vec<String> {
        let mut header = crate::types::header::HduHeader::empty();
        header.set("BAYERPAT", "RGGB".to_string());
        (0..count)
            .map(|i| {
                let path = dir.path().join(format!("{prefix}{i}.fits")).to_str().unwrap().to_string();
                let frame = Array2::from_shape_fn((12, 12), |(y, x)| 500.0 + ((x * 7 + y * 3 + i) % 5) as f32);
                crate::infra::fits::writer::write_fits_mono(&path, &frame, Some(&header)).unwrap();
                path
            })
            .collect()
    }

    fn cfa_request(dir: &tempfile::TempDir, align: bool, bias_paths: Vec<String>) -> PipelineRequest {
        PipelineRequest {
            channels: vec![ChannelFilesInput { label: "R".into(), paths: write_cfa_frames(dir, "cfa", 2) }],
            dark_paths: vec![],
            flat_paths: vec![],
            bias_paths,
            flat_dark_paths: vec![],
            sigma_low: None,
            sigma_high: None,
            normalize: None,
            align: Some(align),
            rejection: None,
            combine: None,
            cosmetic: None,
            dark_optimize: false,
        }
    }

    #[test]
    fn pipeline_refuses_cfa_lights_before_building_masters() {
        let dir = tempfile::tempdir().unwrap();
        let missing_bias = dir.path().join("missing_bias.fits").to_str().unwrap().to_string();
        let request = cfa_request(&dir, true, vec![missing_bias]);
        let out = out_dir(&dir);
        let err = run_pipeline_request(request, &out, Some("cfa")).unwrap_err();
        assert!(err.contains("2 of 2 lights"), "{err}");
        assert!(err.contains("BAYERPAT=RGGB"), "{err}");
        assert!(!err.contains("missing_bias"), "{err}");
        assert!(!std::path::Path::new(&out).exists(), "the output dir was created before the guard ran");
    }

    fn write_bayer_lights(dir: &tempfile::TempDir, count: usize) -> Vec<String> {
        let mut header = crate::types::header::HduHeader::empty();
        header.set("BAYERPAT", "'RGGB'".to_string());
        (0..count)
            .map(|i| {
                let path = dir.path().join(format!("bayer{i}.fits")).to_str().unwrap().to_string();
                let mut frame = Array2::from_shape_fn((12, 12), |(y, x)| if y % 2 == 0 && x % 2 == 0 { 1000.0 } else { 100.0 });
                frame[[4, 4]] = 60000.0;
                crate::infra::fits::writer::write_fits_mono(&path, &frame, Some(&header)).unwrap();
                path
            })
            .collect()
    }

    fn master_pixel(response: &serde_json::Value, y: usize, x: usize) -> f32 {
        let fits = response[RES_MASTERS][0][RES_FITS_PATH].as_str().unwrap();
        crate::infra::fits::reader::load_fits_image(fits).unwrap()[[y, x]]
    }

    #[test]
    fn pipeline_cosmetic_uses_the_cfa_lattice_on_bayer_lights() {
        let dir = tempfile::tempdir().unwrap();
        let lights = write_bayer_lights(&dir, 2);
        let cosmetic = |cfa: Option<bool>| CosmeticConfig {
            cfa,
            defects: vec![crate::core::imaging::cosmetic::Defect::Point { x: 4, y: 4 }],
            use_master_dark: false,
            auto_hot_sigma: None,
            auto_cold_sigma: None,
            ..Default::default()
        };
        let request = |cfa: Option<bool>| PipelineRequest {
            channels: vec![ChannelFilesInput { label: "R".into(), paths: lights.clone() }],
            dark_paths: vec![],
            flat_paths: vec![],
            bias_paths: vec![],
            flat_dark_paths: vec![],
            sigma_low: None,
            sigma_high: None,
            normalize: Some(false),
            align: Some(false),
            rejection: Some("none".into()),
            combine: None,
            cosmetic: Some(cosmetic(cfa)),
            dark_optimize: false,
        };

        let auto = run_pipeline_request(request(None), &out_dir(&dir), Some("auto")).unwrap();
        let replaced = master_pixel(&auto, 4, 4);
        assert!((replaced - 1000.0).abs() < 1e-3, "cfa None must use the stride-2 R lattice: {replaced}");
        assert!((master_pixel(&auto, 4, 6) - 1000.0).abs() < 1e-3);

        let mono = run_pipeline_request(request(Some(false)), &out_dir(&dir), Some("mono")).unwrap();
        let replaced = master_pixel(&mono, 4, 4);
        assert!((replaced - 100.0).abs() < 1e-3, "an explicit cfa=false keeps the stride-1 median: {replaced}");
    }

    #[test]
    fn pipeline_without_alignment_accepts_cfa_lights() {
        let dir = tempfile::tempdir().unwrap();
        let request = cfa_request(&dir, false, vec![]);
        let response = run_pipeline_request(request, &out_dir(&dir), Some("cfa")).unwrap();
        assert_eq!(response[RES_MASTERS].as_array().map(|m| m.len()), Some(1));
    }

    #[test]
    fn pipeline_stats_report_how_many_frames_built_each_master() {
        let dir = tempfile::tempdir().unwrap();
        let request = PipelineRequest {
            channels: vec![ChannelFilesInput { label: "L".into(), paths: write_frames(&dir, "light", 2, 500.0) }],
            dark_paths: write_frames(&dir, "dark", 3, 10.0),
            flat_paths: write_frames(&dir, "flat", 2, 1000.0),
            bias_paths: vec![],
            flat_dark_paths: vec![],
            sigma_low: None,
            sigma_high: None,
            normalize: None,
            align: Some(false),
            rejection: None,
            combine: None,
            cosmetic: None,
            dark_optimize: false,
        };

        let response = run_pipeline_request(request, &out_dir(&dir), None).unwrap();
        let stats = &response[RES_STATS];
        assert_eq!(stats["darks_combined"], 3, "{stats}");
        assert_eq!(stats["flats_combined"], 2, "{stats}");
        assert_eq!(stats["bias_combined"], 0, "{stats}");
    }

    #[test]
    fn pipeline_writes_fits_and_png_per_master_and_the_rgb_png_with_matching_dims() {
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let request = rgb_request(&dir, [12, 12, 12]);
        let first_lights: Vec<String> = request.channels.iter().map(|ch| ch.paths[0].clone()).collect();
        let stem = "M42_pipeline4_20260926-101010-123";

        let response = run_pipeline_request(request, &out, Some(stem)).unwrap();

        let masters = response[RES_MASTERS].as_array().unwrap();
        let previews = response[RES_CHANNEL_PREVIEWS].as_array().unwrap();
        assert_eq!(masters.len(), 3);
        for (i, label) in ["R", "G", "B"].into_iter().enumerate() {
            let master = &masters[i];
            assert_eq!(master[RES_LABEL], label);
            assert_eq!(previews[i][RES_LABEL], label);
            let fits = master[RES_FITS_PATH].as_str().unwrap();
            let png = master[RES_PNG_PATH].as_str().unwrap();
            assert_eq!(fits, format!("{out}/{stem}_{label}.fits"));
            assert_eq!(png, format!("{out}/{stem}_{label}.png"));
            assert_eq!(crate::infra::fits::reader::load_fits_image(fits).unwrap().dim(), (12, 12));
            assert_eq!(image::image_dimensions(png).unwrap(), (12, 12));
            assert_eq!(master[RES_DIMENSIONS], json!([12, 12]));
            assert_eq!(master[RES_INPUT_PATH], first_lights[i]);
            let header = crate::infra::fits::reader::read_primary_header(fits).unwrap();
            assert_eq!(header.get("ABPROC").map(str::trim), Some("stacked"));
        }

        let rgb_png = response[RES_RGB_PNG_PATH].as_str().unwrap();
        assert_eq!(rgb_png, format!("{out}/{stem}_rgb.png"));
        assert_eq!(response[RES_RGB_DIMENSIONS], json!([12, 12]));
        let saved = image::open(rgb_png).unwrap().into_rgb8();
        assert_eq!(saved.dimensions(), (12, 12));
        assert_eq!(saved.into_raw(), decode_b64(&response[RES_RGB_PREVIEW]));
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 7);
    }

    #[test]
    fn pipeline_channel_masters_carry_ncombine() {
        use crate::types::constants::{
            HEADER_COMBINE_METHOD, HEADER_NCOMBINE, HEADER_NORMALIZATION_METHOD, HEADER_REJECTION_METHOD, HEADER_TOTEXP,
        };
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let request = rgb_request(&dir, [12, 12, 12]);

        let response = run_pipeline_request(request, &out, Some("masters")).unwrap();

        let masters = response[RES_MASTERS].as_array().unwrap();
        for (master, expected) in masters.iter().zip([2, 1, 1]) {
            let fits = master[RES_FITS_PATH].as_str().unwrap();
            let header = crate::infra::fits::reader::read_primary_header(fits).unwrap();
            assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(expected), "{}", master[RES_LABEL]);
            assert_eq!(header.get(HEADER_COMBINE_METHOD).map(str::trim), Some("mean"));
            assert_eq!(header.get(HEADER_REJECTION_METHOD).map(str::trim), Some("sigma_clip"));
            assert_eq!(header.get(HEADER_NORMALIZATION_METHOD).map(str::trim), Some("frame"));
            assert_eq!(header.get(HEADER_TOTEXP), None, "the test lights carry no exposure card");
        }
        let r_header = crate::infra::fits::reader::read_primary_header(masters[0][RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert!(
            r_header
                .cards
                .iter()
                .any(|(k, v)| k.trim() == "HISTORY" && v == "AstroBurst stack: 2 frames, sigma_clip, mean, frame"),
            "{:?}",
            r_header.cards
        );
    }

    fn blob_frame(cx: f32) -> Array2<f32> {
        Array2::from_shape_fn((300, 300), |(y, x)| {
            let (dy, dx) = (y as f32 - 150.0, x as f32 - cx);
            100.0 + 1000.0 * (-(dy * dy + dx * dx) / 18.0).exp()
        })
    }

    fn write_blob_lights(dir: &tempfile::TempDir, prefix: &str, centers_x: &[f32]) -> Vec<String> {
        let mut header = crate::types::header::HduHeader::empty();
        header.set("EXPTIME", "300".to_string());
        centers_x
            .iter()
            .enumerate()
            .map(|(i, &cx)| {
                let path = dir.path().join(format!("{prefix}{i}.fits")).to_str().unwrap().to_string();
                crate::infra::fits::writer::write_fits_mono(&path, &blob_frame(cx), Some(&header)).unwrap();
                path
            })
            .collect()
    }

    fn aligned_request(channels: Vec<ChannelFilesInput>) -> PipelineRequest {
        PipelineRequest {
            channels,
            dark_paths: vec![],
            flat_paths: vec![],
            bias_paths: vec![],
            flat_dark_paths: vec![],
            sigma_low: None,
            sigma_high: None,
            normalize: Some(false),
            align: Some(true),
            rejection: Some("none".into()),
            combine: None,
            cosmetic: None,
            dark_optimize: false,
        }
    }

    #[test]
    fn a_frame_left_out_by_alignment_is_shown_to_the_user_and_not_counted_in_the_master_cards() {
        use crate::types::constants::{HEADER_NCOMBINE, HEADER_TOTEXP};
        let dir = tempfile::tempdir().unwrap();
        let paths = write_blob_lights(&dir, "light", &[50.0, 50.0, 250.0]);
        let request = aligned_request(vec![ChannelFilesInput { label: "L".into(), paths }]);

        let response = run_pipeline_request(request, &out_dir(&dir), Some("excluded")).unwrap();

        let warnings: Vec<&str> = response[RES_WARNINGS].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(
            warnings.iter().any(|w| w.starts_with("Channel 'L': frame 3 of 3 was left out of the stack")),
            "{warnings:?}"
        );
        assert_eq!(response[RES_STATS]["channels"][0]["excluded_frames"], json!([2]));
        let header = crate::infra::fits::reader::read_primary_header(response[RES_MASTERS][0][RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(2), "NCOMBINE counts a frame that is not in the stack");
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(600.0), "TOTEXP counts a frame that is not in the stack");
        assert!(
            header.cards.iter().any(|(k, v)| k.trim() == "HISTORY" && v.starts_with("AstroBurst stack: 2 frames")),
            "{:?}",
            header.cards
        );
    }

    fn blob_channel(label: &str, centers_x: &[f32]) -> ChannelInput {
        ChannelInput {
            lights: centers_x.iter().map(|&cx| blob_frame(cx)).collect(),
            label: label.to_string(),
            dark_scales: vec![1.0; centers_x.len()],
            cfa: false,
            dark_group: Vec::new(),
            light_temp_c: Vec::new(),
        }
    }

    fn aligned_config() -> BatchPipelineConfig {
        BatchPipelineConfig {
            stack: BatchStackConfig {
                normalize_before_stack: false,
                rejection: crate::types::stacking::RejectionMethod::None,
                ..BatchStackConfig::default()
            },
            align: true,
            cosmetic: None,
            dark_optimize: false,
        }
    }

    #[test]
    fn per_channel_stacking_keeps_alignment_exclusions_in_their_own_channel() {
        let specs = vec![spec("R", 3), spec("L", 2)];
        let centers = |label: &str| -> Vec<f32> { if label == "R" { vec![50.0, 50.0, 250.0] } else { vec![50.0, 50.0] } };
        let result = stack_channels_one_at_a_time(
            &specs,
            |_, s| Ok(blob_channel(&s.label, &centers(&s.label))),
            &CalibrationMasters::default(),
            MasterFrameCounts::default(),
            &aligned_config(),
        )
        .unwrap();
        let (r, l) = (&result.stats.channels[0], &result.stats.channels[1]);
        assert_eq!((r.label.as_str(), l.label.as_str()), ("R", "L"));
        assert_eq!(r.excluded_frames, vec![2]);
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
        assert!(r.warnings[0].starts_with("Channel 'R': frame 3 of 3 was left out"), "{}", r.warnings[0]);
        assert!(l.excluded_frames.is_empty() && l.warnings.is_empty(), "{l:?}");

        let err = stack_channels_one_at_a_time(
            &[spec("L", 2), spec("R", 2)],
            |_, s| Ok(blob_channel(&s.label, if s.label == "R" { &[50.0, 250.0] } else { &[50.0, 50.0] })),
            &CalibrationMasters::default(),
            MasterFrameCounts::default(),
            &aligned_config(),
        )
        .unwrap_err();
        assert!(err.starts_with("Channel 'R': no frame could be aligned to the reference frame (frame 1)"), "{err}");
    }

    #[test]
    fn pipeline_output_name_and_labels_cannot_escape_the_output_dir() {
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let mut request = rgb_request(&dir, [12, 12, 12]);
        request.channels[0].label = "../../R".into();
        request.channels[1].label = "G / green".into();
        request.channels[2].label = "..".into();

        let response = run_pipeline_request(request, &out, Some("../evil")).unwrap();

        let mut written = Vec::new();
        for master in response[RES_MASTERS].as_array().unwrap() {
            written.push(master[RES_FITS_PATH].as_str().unwrap().to_string());
            written.push(master[RES_PNG_PATH].as_str().unwrap().to_string());
        }
        assert!(written[0].ends_with("/_evil__.._R.fits"), "{}", written[0]);
        assert!(written[2].ends_with("/_evil_G_green.fits"), "{}", written[2]);
        assert!(written[4].ends_with("/_evil_channel.fits"), "{}", written[4]);
        let canonical_out = std::fs::canonicalize(&out).unwrap();
        for path in &written {
            let canonical = std::fs::canonicalize(path).unwrap();
            assert_eq!(canonical.parent(), Some(canonical_out.as_path()), "{path}");
        }
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), written.len());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().filter(|e| e.as_ref().unwrap().path().is_dir()).count(), 1);
        assert!(response[RES_RGB_PNG_PATH].is_null());
        assert!(response[RES_RGB_DIMENSIONS].is_null());
        assert!(response[RES_RGB_PREVIEW].is_null());
    }

    #[test]
    fn rgb_dimensions_follow_the_composed_buffer_when_channel_sizes_differ() {
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);

        let response = run_pipeline_request(rgb_request(&dir, [12, 10, 10]), &out, None).unwrap();

        assert_eq!(response[RES_CHANNEL_PREVIEWS][0][RES_WIDTH], 12);
        assert_eq!(response[RES_MASTERS][0][RES_DIMENSIONS], json!([12, 12]));
        assert_eq!(response[RES_MASTERS][1][RES_DIMENSIONS], json!([10, 10]));
        assert_eq!(response[RES_RGB_DIMENSIONS], json!([10, 10]));
        assert_eq!(decode_b64(&response[RES_RGB_PREVIEW]).len(), 10 * 10 * 3);
        let rgb_png = response[RES_RGB_PNG_PATH].as_str().unwrap();
        assert_eq!(rgb_png, format!("{out}/pipeline_rgb.png"));
        assert_eq!(image::image_dimensions(rgb_png).unwrap(), (10, 10));
        assert!(std::path::Path::new(&format!("{out}/pipeline_R.fits")).is_file());
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
    fn dark_scales_read_the_exposure_of_hdu_refs() {
        use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("light.fits");
        write_test_mef(
            &path,
            &[("EXPTIME", "30.0".to_string())],
            &[TestHdu { extname: Some("SCI"), extver: Some(1), cols: 2, rows: 2, data: HduData::F32(vec![1.0, 2.0, 3.0, 4.0]), extra_cards: vec![] }],
        );
        let reference = crate::types::image_ref::ImageRef::hdu(path.to_str().unwrap(), 1).cache_key();
        let scales = dark_scales_for_cards(&[read_frame_cards(&reference)], Some(300.0));
        assert!((scales[0] - 0.1).abs() < 1e-6, "{scales:?}");
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
        let (bytes, rw, rh) = rgb_preview_bytes(&rgb);
        let (_, pw, ph) = array2_to_b64_u16(&Array2::<f32>::zeros((1, w)));
        assert_eq!((rw, rh), (pw, ph));
        assert_eq!(bytes.len(), pw * ph * 3);
        assert!(bytes.iter().step_by(3).all(|&r| r == 255));
    }
}
