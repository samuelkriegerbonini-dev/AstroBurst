// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use ndarray::Array2;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use astroburst_lib::core::imaging::calibration_pipeline::{
    lights_with_dq_planes, run_batch_pipeline, validate_cosmetic_config, BatchPipelineConfig,
    BatchStackConfig, CalibrationMasters, ChannelInput, DQ_COSMETIC_WARNING,
};
use astroburst_lib::core::imaging::cosmetic::CosmeticConfig;
use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::core::stacking::calibration::{
    create_master_bias, create_master_dark, create_master_flat,
};
use astroburst_lib::infra::cache::ImageCache;
use astroburst_lib::infra::fits::reader::{load_fits_image, read_primary_header};
use astroburst_lib::types::stacking::{CombineMethod, RejectionMethod};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::job::{new_job, Job, SseEvent};
use crate::state::AppState;

fn fail(job: &Job, tx: &mpsc::Sender<SseEvent>, message: String) {
    if job.cancel.is_cancelled() {
        return;
    }
    job.set_error();
    tx.blocking_send(SseEvent::Error { message }).ok();
}

fn store_channels(
    job: &Job,
    tx: &mpsc::Sender<SseEvent>,
    cache: &ImageCache,
    channels: &[(String, Array2<f32>)],
    slot_names: &[String],
) {
    if job.cancel.is_cancelled() {
        return;
    }
    tx.blocking_send(SseEvent::Progress { pct: 90, stage: "storing".into() }).ok();

    for ((label, arr), slot) in channels.iter().zip(slot_names) {
        let stats = compute_image_stats(arr);
        cache.insert_synthetic(slot, Arc::new(arr.clone()), stats);
        log::debug!("pipeline: stored channel '{}' → slot '{}'", label, slot);
    }

    job.set_done();
    tx.blocking_send(SseEvent::Complete).ok();
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

    let warnings = if config.cosmetic.is_some() {
        let light_paths: Vec<String> = params.channels.iter().flat_map(|ch| ch.paths.iter().cloned()).collect();
        tokio::task::spawn_blocking(move || cosmetic_warnings(light_paths))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("dq inspection panicked: {e}")))?
    } else {
        Vec::new()
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
                    fail(&job, &tx, format!("bias: {:#}", e));
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
                    fail(&job, &tx, format!("dark: {:#}", e));
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
                    fail(&job, &tx, format!("flat: {:#}", e));
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

        if job.cancel.is_cancelled() {
            return;
        }
        tx.blocking_send(SseEvent::Progress { pct: 15, stage: "loading lights".into() }).ok();

        let channel_inputs: Vec<ChannelInput> = {
            let mut out = Vec::with_capacity(channels_spec.len());
            for ch in &channels_spec {
                let lights = match load_batch(&ch.paths) {
                    Ok(l) => l,
                    Err(e) => {
                        fail(&job, &tx, format!("channel '{}': {:#}", ch.label, e));
                        return;
                    }
                };
                let scales = dark_scales(&ch.paths, dark_exp);
                out.push(ChannelInput { lights, label: ch.label.clone(), dark_scales: scales });
            }
            out
        };

        if job.cancel.is_cancelled() {
            return;
        }
        tx.blocking_send(SseEvent::Progress { pct: 25, stage: "stacking".into() }).ok();

        let pipeline_result = match run_batch_pipeline(channel_inputs, &masters, &config) {
            Ok(r) => r,
            Err(e) => {
                fail(&job, &tx, e);
                return;
            }
        };

        store_channels(&job, &tx, &cache, &pipeline_result.master_channels, &slot_names);
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

    use super::*;
    use crate::job::JobStatus;

    fn setup() -> (Arc<Job>, mpsc::Sender<SseEvent>, mpsc::Receiver<SseEvent>, ImageCache) {
        let (job, tx) = new_job("pipeline");
        let rx = job.rx.lock().unwrap().take().unwrap();
        (job, tx, rx, ImageCache::new(4, 1 << 20))
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
        let (job, tx, mut rx, cache) = setup();
        job.cancel.cancel();
        job.set_cancelled();

        store_channels(&job, &tx, &cache, &channels(), &["p_R".to_string(), "p_G".to_string()]);

        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert_ne!(job.pct.load(Ordering::Relaxed), 100);
        assert!(cache.get("p_R").is_none());
        assert!(cache.get("p_G").is_none());
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn cancelled_job_ignores_failure() {
        let (job, tx, mut rx, _cache) = setup();
        job.cancel.cancel();
        job.set_cancelled();

        fail(&job, &tx, "bias: boom".into());

        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn running_job_stores_channels_and_completes() {
        let (job, tx, mut rx, cache) = setup();

        store_channels(&job, &tx, &cache, &channels(), &["p_R".to_string(), "p_G".to_string()]);

        assert_eq!(job.current_status(), JobStatus::Done);
        assert!(cache.get("p_R").is_some());
        assert!(cache.get("p_G").is_some());
        assert!(matches!(drain(&mut rx).last(), Some(SseEvent::Complete)));
    }

    #[test]
    fn running_job_reports_failure() {
        let (job, tx, mut rx, _cache) = setup();

        fail(&job, &tx, "dark: boom".into());

        assert_eq!(job.current_status(), JobStatus::Error);
        assert!(matches!(drain(&mut rx).last(), Some(SseEvent::Error { .. })));
    }
}
