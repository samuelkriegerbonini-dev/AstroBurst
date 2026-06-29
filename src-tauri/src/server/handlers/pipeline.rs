// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::imaging::calibration_pipeline::{
    BatchPipelineConfig, BatchStackConfig, CalibrationMasters, ChannelInput,
    run_batch_pipeline,
};
use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::core::stacking::calibration::{
    create_master_bias, create_master_dark, create_master_flat,
};
use astroburst_lib::infra::fits::reader::{load_fits_image, read_primary_header};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::job::{new_job, SseEvent};
use crate::state::AppState;

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
    pub sigma_low: Option<f32>,
    pub sigma_high: Option<f32>,
    pub normalize: Option<bool>,
    pub result_prefix: Option<String>,
}

fn read_exposure_s(path: &str) -> Option<f64> {
    let h = read_primary_header(path).ok()?;
    h.get_f64("EXPTIME")
        .or_else(|| h.get_f64("EXPOSURE"))
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn median_exposure(paths: &[String]) -> Option<f64> {
    let mut vals: Vec<f64> = paths.iter().filter_map(|p| read_exposure_s(p)).collect();
    if vals.is_empty() {
        return None;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(vals[vals.len() / 2])
}

fn dark_scales(light_paths: &[String], dark_exp: Option<f64>) -> Vec<f32> {
    let Some(dexp) = dark_exp else {
        return vec![1.0; light_paths.len()];
    };
    light_paths
        .iter()
        .map(|p| match read_exposure_s(p) {
            Some(lexp) => {
                let r = (lexp / dexp) as f32;
                if (r - 1.0).abs() < 0.01 { 1.0 } else { r.clamp(0.05, 20.0) }
            }
            None => 1.0,
        })
        .collect()
}

fn load_batch(paths: &[String]) -> anyhow::Result<Vec<ndarray::Array2<f32>>> {
    paths.iter().map(|p| load_fits_image(p)).collect()
}

pub async fn run(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
    Json(params): Json<PipelineParams>,
) -> Result<(StatusCode, Json<Value>)> {
    if params.channels.is_empty() {
        return Err(AppError::BadRequest("channels must not be empty".into()));
    }

    let permit = state
        .job_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests)?;

    let bias_paths = params.bias_paths.unwrap_or_default();
    let dark_paths = params.dark_paths.unwrap_or_default();
    let flat_paths = params.flat_paths.unwrap_or_default();
    let prefix = params.result_prefix.unwrap_or_default();

    let config = BatchPipelineConfig {
        stack: BatchStackConfig {
            sigma_low: params.sigma_low.unwrap_or(2.5),
            sigma_high: params.sigma_high.unwrap_or(3.0),
            max_iterations: 5,
            normalize_before_stack: params.normalize.unwrap_or(true),
        },
    };

    let slot_names: Vec<String> = params
        .channels
        .iter()
        .map(|ch| format!("{}{}", prefix, ch.label))
        .collect();

    let (job, tx) = new_job("pipeline");
    let job_id = job.id.clone();
    session.jobs.insert(job_id.clone(), Arc::clone(&job));

    let cache = Arc::clone(&session.cache);
    let channels_spec = params.channels;
    let slot_names_resp = slot_names.clone();

    tokio::task::spawn_blocking(move || {
        let _permit = permit;

        tx.blocking_send(SseEvent::Progress { pct: 5, stage: "building masters".into() }).ok();

        let master_bias = if bias_paths.is_empty() {
            None
        } else {
            match create_master_bias(&bias_paths) {
                Ok(b) => Some(b),
                Err(e) => {
                    job.set_error();
                    tx.blocking_send(SseEvent::Error { message: format!("bias: {:#}", e) }).ok();
                    return;
                }
            }
        };

        let master_dark = if dark_paths.is_empty() {
            None
        } else {
            match create_master_dark(&dark_paths, master_bias.as_ref()) {
                Ok(d) => Some(d),
                Err(e) => {
                    job.set_error();
                    tx.blocking_send(SseEvent::Error { message: format!("dark: {:#}", e) }).ok();
                    return;
                }
            }
        };

        let master_flat = if flat_paths.is_empty() {
            None
        } else {
            match create_master_flat(&flat_paths, master_bias.as_ref(), master_dark.as_ref(), median_exposure(&dark_paths)) {
                Ok(f) => Some(f),
                Err(e) => {
                    job.set_error();
                    tx.blocking_send(SseEvent::Error { message: format!("flat: {:#}", e) }).ok();
                    return;
                }
            }
        };

        let dark_exp = if dark_paths.is_empty() || master_bias.is_none() {
            None
        } else {
            median_exposure(&dark_paths)
        };

        let masters = CalibrationMasters { dark: master_dark, flat: master_flat, bias: master_bias };

        tx.blocking_send(SseEvent::Progress { pct: 15, stage: "loading lights".into() }).ok();

        let channel_inputs: Vec<ChannelInput> = {
            let mut out = Vec::with_capacity(channels_spec.len());
            for ch in &channels_spec {
                let lights = match load_batch(&ch.paths) {
                    Ok(l) => l,
                    Err(e) => {
                        job.set_error();
                        tx.blocking_send(SseEvent::Error {
                            message: format!("channel '{}': {:#}", ch.label, e),
                        }).ok();
                        return;
                    }
                };
                let scales = dark_scales(&ch.paths, dark_exp);
                out.push(ChannelInput { lights, label: ch.label.clone(), dark_scales: scales });
            }
            out
        };

        tx.blocking_send(SseEvent::Progress { pct: 25, stage: "stacking".into() }).ok();

        let pipeline_result = match run_batch_pipeline(channel_inputs, &masters, &config) {
            Ok(r) => r,
            Err(e) => {
                job.set_error();
                tx.blocking_send(SseEvent::Error { message: e }).ok();
                return;
            }
        };

        tx.blocking_send(SseEvent::Progress { pct: 90, stage: "storing".into() }).ok();

        for ((label, arr), slot) in pipeline_result.master_channels.iter().zip(&slot_names) {
            let stats = compute_image_stats(arr);
            cache.insert_synthetic(slot, Arc::new(arr.clone()), stats);
            log::debug!("pipeline: stored channel '{}' → slot '{}'", label, slot);
        }

        job.set_done();
        tx.blocking_send(SseEvent::Complete).ok();
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "job_id": job_id,
            "status": "running",
            "slots": slot_names_resp,
        })),
    ))
}
