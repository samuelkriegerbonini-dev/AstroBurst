// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use ndarray::Array2;
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::imaging::calibration_pipeline::{
    lights_with_dq_planes, run_batch_pipeline_cancellable, validate_cosmetic_config, BatchPipelineConfig,
    BatchStackConfig, ChannelInput, DQ_COSMETIC_WARNING,
};
use astroburst_lib::core::imaging::cosmetic::CosmeticConfig;
use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::core::stacking::calibration::read_exposure_seconds;
use astroburst_lib::core::stacking::cfa_guard::CfaStep;
use astroburst_lib::core::stacking::frame_cards::{read_frame_cards, FrameCards};
use astroburst_lib::core::stacking::masters::{
    assign_dark_groups, build_calibration_masters, check_channel_lights, check_master_frames, dark_group_warnings,
    read_master_frames, MasterRequest,
};
use astroburst_lib::core::stacking::CancelCheck;
use astroburst_lib::infra::cache::ImageCache;
use astroburst_lib::infra::fits::reader::load_fits_image;
use astroburst_lib::types::error::AppError as CoreError;
use astroburst_lib::types::stacking::{CombineMethod, RejectionMethod};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::handlers::stacking::{refuse_cfa_lights, require_positive, stop_if_cancelled};
use crate::job::{new_job, spawn_job, Job};
use crate::state::AppState;

const STACKING_PCT: u32 = 25;

fn store_channels(
    job: &Job,
    cache: &ImageCache,
    channels: Vec<(String, Array2<f32>)>,
    slot_names: &[String],
) {
    job.progress(90, "storing");
    if !job.begin_commit() {
        return;
    }

    for ((label, arr), slot) in channels.into_iter().zip(slot_names) {
        let stats = compute_image_stats(&arr);
        cache.insert_synthetic(slot, Arc::new(arr), stats);
        log::debug!("pipeline: stored channel '{}' → slot '{}'", label, slot);
    }

    job.set_done();
}

#[derive(Deserialize)]
pub struct ChannelSpec {
    pub label: String,
    pub paths: Vec<String>,
}

#[derive(Deserialize)]
pub struct PipelineParams {
    pub channels: Vec<ChannelSpec>,
    pub dark_paths: Option<Vec<String>>,
    pub flat_paths: Option<Vec<String>>,
    pub bias_paths: Option<Vec<String>>,
    pub flat_dark_paths: Option<Vec<String>>,
    pub sigma_low: Option<f32>,
    pub sigma_high: Option<f32>,
    pub normalize: Option<bool>,
    pub align: Option<bool>,
    pub result_prefix: Option<String>,
    pub rejection: Option<String>,
    pub combine: Option<String>,
    #[serde(default)]
    pub cosmetic: Option<CosmeticConfig>,
    #[serde(default)]
    pub dark_optimize: bool,
}

fn cosmetic_warnings(light_paths: Vec<String>) -> Vec<String> {
    if lights_with_dq_planes(light_paths.iter()).is_empty() {
        Vec::new()
    } else {
        vec![DQ_COSMETIC_WARNING.to_string()]
    }
}

fn pre_run_checks(
    inputs: &PipelineInputs,
    channels: &[(String, Vec<String>)],
    cosmetic_enabled: bool,
) -> std::result::Result<Vec<String>, String> {
    let frames = read_master_frames(&inputs.master_request());
    let mut warnings = check_master_frames(&frames).map_err(|e| format!("{:#}", e))?;
    for (label, paths) in channels {
        let cards = channel_cards(paths);
        warnings.extend(check_channel_lights(&frames, label, &cards, cosmetic_enabled).map_err(|e| format!("{:#}", e))?);
        warnings.extend(dark_group_warnings(&frames, &cards, label));
    }
    if cosmetic_enabled {
        let light_paths: Vec<String> = channels.iter().flat_map(|(_, paths)| paths.iter().cloned()).collect();
        warnings.extend(cosmetic_warnings(light_paths));
    }
    Ok(warnings)
}

fn dark_scales(light_paths: &[String], dark_exp: Option<f64>) -> Vec<f32> {
    let Some(dexp) = dark_exp else {
        return vec![1.0; light_paths.len()];
    };
    light_paths
        .iter()
        .map(|p| match read_exposure_seconds(p) {
            Some(lexp) => {
                let r = (lexp / dexp) as f32;
                if (r - 1.0).abs() < 0.01 { 1.0 } else { r.clamp(0.05, 20.0) }
            }
            None => 1.0,
        })
        .collect()
}

fn load_batch(paths: &[String], cancelled: CancelCheck) -> anyhow::Result<Vec<ndarray::Array2<f32>>> {
    paths
        .iter()
        .map(|p| {
            stop_if_cancelled(cancelled)?;
            load_fits_image(p)
        })
        .collect()
}

struct PipelineInputs {
    bias_paths: Vec<String>,
    dark_paths: Vec<String>,
    flat_paths: Vec<String>,
    flat_dark_paths: Vec<String>,
}

impl PipelineInputs {
    fn master_request(&self) -> MasterRequest<'_> {
        MasterRequest {
            bias: &self.bias_paths,
            darks: &self.dark_paths,
            flats: &self.flat_paths,
            flat_darks: &self.flat_dark_paths,
        }
    }
}

fn channel_cards(paths: &[String]) -> Vec<FrameCards> {
    paths.iter().map(|p| read_frame_cards(p)).collect()
}

fn checkpoint(cancelled: CancelCheck) -> std::result::Result<(), String> {
    if cancelled() {
        Err(CoreError::Cancelled.to_string())
    } else {
        Ok(())
    }
}

fn run_pipeline(
    job: &Job,
    inputs: &PipelineInputs,
    channels_spec: &[ChannelSpec],
    config: &BatchPipelineConfig,
    cancelled: CancelCheck,
) -> std::result::Result<Vec<(String, Array2<f32>)>, String> {
    job.progress(5, "building masters");

    let (masters, report) =
        build_calibration_masters(&inputs.master_request(), cancelled).map_err(|e| format!("{:#}", e))?;
    checkpoint(cancelled)?;
    for warning in &report.warnings {
        log::warn!("pipeline: {warning}");
    }
    let dark_exp = report.dark_exposure_s.filter(|_| masters.bias.is_some());

    job.progress(15, "loading lights");
    let mut channel_inputs: Vec<ChannelInput> = Vec::with_capacity(channels_spec.len());
    for ch in channels_spec {
        checkpoint(cancelled)?;
        let cards = channel_cards(&ch.paths);
        let lights = load_batch(&ch.paths, cancelled).map_err(|e| format!("channel '{}': {:#}", ch.label, e))?;
        let scales = dark_scales(&ch.paths, dark_exp);
        let (dark_group, temp_warnings) = assign_dark_groups(&masters.dark_groups, &cards, &ch.label);
        for warning in &temp_warnings {
            log::warn!("pipeline: {warning}");
        }
        channel_inputs.push(ChannelInput {
            lights,
            label: ch.label.clone(),
            dark_scales: scales,
            cfa: cards.iter().any(|c| c.cfa),
            dark_group,
            light_temp_c: cards.iter().map(|c| c.temp_c).collect(),
        });
    }
    checkpoint(cancelled)?;

    job.progress(STACKING_PCT, "stacking");
    let result = run_batch_pipeline_cancellable(channel_inputs, &masters, config, cancelled)?;
    checkpoint(cancelled)?;
    Ok(result.master_channels)
}

pub async fn run(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
    Json(params): Json<PipelineParams>,
) -> Result<(StatusCode, Json<Value>)> {
    if params.channels.is_empty() {
        return Err(AppError::BadRequest("channels must not be empty".into()));
    }

    let bias_paths = params.bias_paths.unwrap_or_default();
    let dark_paths = params.dark_paths.unwrap_or_default();
    let flat_paths = params.flat_paths.unwrap_or_default();
    let flat_dark_paths = params.flat_dark_paths.unwrap_or_default();
    let prefix = params.result_prefix.unwrap_or_default();

    let rejection = match params.rejection.as_deref() {
        Some(name) => RejectionMethod::from_name(name).map_err(AppError::BadRequest)?,
        None => RejectionMethod::default(),
    };
    let combine = match params.combine.as_deref() {
        Some(name) => CombineMethod::from_name(name).map_err(AppError::BadRequest)?,
        None => CombineMethod::default(),
    };

    if let Some(cosmetic) = &params.cosmetic {
        validate_cosmetic_config(cosmetic).map_err(AppError::BadRequest)?;
    }

    let config = BatchPipelineConfig {
        stack: BatchStackConfig {
            sigma_low: params.sigma_low.unwrap_or(2.5),
            sigma_high: params.sigma_high.unwrap_or(3.0),
            max_iterations: 5,
            normalize_before_stack: params.normalize.unwrap_or(true),
            rejection,
            combine,
        },
        align: params.align.unwrap_or(true),
        cosmetic: params.cosmetic,
        dark_optimize: params.dark_optimize,
    };
    require_positive("sigma_low", config.stack.sigma_low as f64)?;
    require_positive("sigma_high", config.stack.sigma_high as f64)?;
    if config.align {
        let lights: Vec<String> = params.channels.iter().flat_map(|ch| ch.paths.iter().cloned()).collect();
        refuse_cfa_lights(lights, CfaStep::Align).await?;
    }

    let permit = state
        .job_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests(state.config.jobs_max))?;

    let inputs = PipelineInputs { bias_paths, dark_paths, flat_paths, flat_dark_paths };
    let warnings = {
        let inputs = PipelineInputs {
            bias_paths: inputs.bias_paths.clone(),
            dark_paths: inputs.dark_paths.clone(),
            flat_paths: inputs.flat_paths.clone(),
            flat_dark_paths: inputs.flat_dark_paths.clone(),
        };
        let channels: Vec<(String, Vec<String>)> =
            params.channels.iter().map(|ch| (ch.label.clone(), ch.paths.clone())).collect();
        let cosmetic_enabled = config.cosmetic.is_some();
        tokio::task::spawn_blocking(move || pre_run_checks(&inputs, &channels, cosmetic_enabled))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("frame inspection panicked: {e}")))?
            .map_err(AppError::BadRequest)?
    };

    let slot_names: Vec<String> = params
        .channels
        .iter()
        .map(|ch| format!("{}{}", prefix, ch.label))
        .collect();

    let job = new_job("pipeline");
    let job_id = job.id.clone();
    session.jobs.insert(job_id.clone(), Arc::clone(&job));

    let cache = Arc::clone(&session.cache);
    let channels_spec = params.channels;
    let slot_names_resp = slot_names.clone();

    spawn_job(job, permit, move |job| {
        let cancelled = || job.cancel.is_cancelled();
        match run_pipeline(job, &inputs, &channels_spec, &config, &cancelled) {
            Ok(channels) => store_channels(job, &cache, channels, &slot_names),
            Err(message) => {
                job.set_error(message);
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "job_id": job_id,
            "status": "running",
            "slots": slot_names_resp,
            "warnings": warnings,
        })),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use tokio::sync::mpsc;

    use super::*;
    use crate::job::{cancel_after_stage_started, run_job, JobStatus, SseEvent};
    use astroburst_lib::core::stacking::calibration::median_exposure_seconds;

    fn setup() -> (Arc<Job>, mpsc::Receiver<SseEvent>, ImageCache) {
        let job = new_job("pipeline");
        let rx = job.rx.lock().unwrap().take().unwrap();
        (job, rx, ImageCache::new(4, 1 << 20))
    }

    fn drain(rx: &mut mpsc::Receiver<SseEvent>) -> Vec<SseEvent> {
        let mut out = Vec::new();
        while let Ok(evt) = rx.try_recv() {
            out.push(evt);
        }
        out
    }

    fn channels() -> Vec<(String, Array2<f32>)> {
        vec![
            ("R".to_string(), Array2::<f32>::ones((4, 4))),
            ("G".to_string(), Array2::<f32>::ones((4, 4))),
        ]
    }

    #[test]
    fn cancelled_job_does_not_store_channels() {
        let (job, mut rx, cache) = setup();
        job.set_cancelled();

        store_channels(&job, &cache, channels(), &["p_R".to_string(), "p_G".to_string()]);

        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert_ne!(job.pct.load(Ordering::Relaxed), 100);
        assert!(cache.get("p_R").is_none());
        assert!(cache.get("p_G").is_none());
        assert!(matches!(drain(&mut rx).as_slice(), [SseEvent::Cancelled]));
    }

    #[test]
    fn cancelled_job_ignores_failure() {
        let (job, mut rx, _cache) = setup();
        job.set_cancelled();

        assert!(!job.set_error("bias: boom"));

        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert!(matches!(drain(&mut rx).as_slice(), [SseEvent::Cancelled]));
    }

    #[test]
    fn running_job_stores_channels_and_completes() {
        let (job, mut rx, cache) = setup();

        store_channels(&job, &cache, channels(), &["p_R".to_string(), "p_G".to_string()]);

        assert_eq!(job.current_status(), JobStatus::Done);
        assert!(cache.get("p_R").is_some());
        assert!(cache.get("p_G").is_some());
        assert!(matches!(drain(&mut rx).last(), Some(SseEvent::Complete)));
    }

    #[test]
    fn running_job_reports_failure() {
        let (job, mut rx, _cache) = setup();

        run_job(&job, |job| {
            job.set_error("dark: boom");
        });

        assert_eq!(job.current_status(), JobStatus::Error);
        assert!(matches!(drain(&mut rx).last(), Some(SseEvent::Error { .. })));
    }

    #[test]
    fn exposure_scaling_reads_hdu_refs() {
        let dir = tempfile::tempdir().unwrap();
        let light = dir.path().join("light.fits");
        let dark = dir.path().join("dark.fits");
        crate::tests::v2_fixtures::write_exposure_fits(&light, 4, 4, 150.0);
        crate::tests::v2_fixtures::write_exposure_fits(&dark, 4, 4, 300.0);
        let light_ref = format!("{}#hdu=0", light.to_str().unwrap());
        let dark_ref = format!("{}#hdu=0", dark.to_str().unwrap());

        let dark_exp = median_exposure_seconds(&[dark_ref]);
        assert_eq!(dark_exp, Some(300.0));
        assert_eq!(dark_scales(&[light_ref], dark_exp), vec![0.5]);
    }

    fn plain_config() -> BatchPipelineConfig {
        BatchPipelineConfig {
            stack: BatchStackConfig {
                sigma_low: 2.5,
                sigma_high: 3.0,
                max_iterations: 5,
                normalize_before_stack: true,
                rejection: RejectionMethod::default(),
                combine: CombineMethod::default(),
            },
            align: false,
            cosmetic: None,
            dark_optimize: false,
        }
    }

    fn write_frames(dir: &tempfile::TempDir, prefix: &str, count: usize) -> Vec<String> {
        (0..count)
            .map(|i| {
                let path = dir.path().join(format!("{prefix}{i}.fits"));
                crate::tests::v2_fixtures::write_pixels_fits(&path, 4, 4, &[100.0 + i as f32; 16]);
                path.to_str().unwrap().to_string()
            })
            .collect()
    }

    #[test]
    fn a_cancel_stops_the_master_build_before_its_next_frame() {
        let dir = tempfile::tempdir().unwrap();
        let (job, _rx, _cache) = setup();
        job.set_cancelled();
        let mut bias_paths = write_frames(&dir, "bias", 1);
        let missing = dir.path().join("missing.fits").to_str().unwrap().to_string();
        bias_paths.push(missing.clone());
        let inputs = PipelineInputs { bias_paths, dark_paths: vec![missing.clone()], flat_paths: Vec::new(), flat_dark_paths: Vec::new() };
        let channels = [ChannelSpec { label: "R".into(), paths: vec![missing] }];
        let err = run_pipeline(&job, &inputs, &channels, &plain_config(), &|| job.cancel.is_cancelled()).unwrap_err();
        assert_eq!(err, format!("bias: {}", CoreError::Cancelled));
    }

    #[test]
    fn a_cancel_raised_while_the_channels_are_stacked_stops_the_combine() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = PipelineInputs { bias_paths: Vec::new(), dark_paths: Vec::new(), flat_paths: Vec::new(), flat_dark_paths: Vec::new() };
        let channels = [ChannelSpec { label: "R".into(), paths: write_frames(&dir, "light", 3) }];
        let (job, _rx, _cache) = setup();
        let err = run_pipeline(&job, &inputs, &channels, &plain_config(), &cancel_after_stage_started(&job, STACKING_PCT))
            .unwrap_err();
        assert_eq!(err, CoreError::Cancelled.to_string());

        let (fresh, _rx, _cache) = setup();
        let stacked = run_pipeline(&fresh, &inputs, &channels, &plain_config(), &|| false).unwrap();
        assert_eq!(stacked.len(), 1);
    }
}
