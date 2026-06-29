// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::core::stacking::calibration::{drizzle_from_paths, stack_from_paths};
use astroburst_lib::types::compose::AlignMethod;
use astroburst_lib::types::stacking::{DrizzleConfig, DrizzleKernel, StackConfig};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::job::{new_job, SseEvent};
use crate::state::AppState;

fn parse_kernel(s: Option<&str>) -> DrizzleKernel {
    match s {
        Some("gaussian") => DrizzleKernel::Gaussian,
        Some("lanczos3") | Some("lanczos") => DrizzleKernel::Lanczos3,
        _ => DrizzleKernel::Square,
    }
}

#[derive(Deserialize)]
pub struct StackParams {
    pub paths: Vec<String>,
    pub result_slot: Option<String>,
    pub sigma_low: Option<f32>,
    pub sigma_high: Option<f32>,
    pub max_iterations: Option<usize>,
    pub align: Option<bool>,
    pub align_method: Option<String>,
    pub weights: Option<Vec<f64>>,
}

#[derive(Deserialize)]
pub struct DrizzleParams {
    pub paths: Vec<String>,
    pub result_slot: Option<String>,
    pub scale: Option<f64>,
    pub pixfrac: Option<f64>,
    pub kernel: Option<String>,
    pub sigma_low: Option<f32>,
    pub sigma_high: Option<f32>,
    pub align: Option<bool>,
}

pub async fn stack(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
    Json(params): Json<StackParams>,
) -> Result<(StatusCode, Json<Value>)> {
    if params.paths.is_empty() {
        return Err(AppError::BadRequest("paths must not be empty".into()));
    }

    let permit = state
        .job_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests)?;

    let config = StackConfig {
        sigma_low: params.sigma_low.unwrap_or(3.0),
        sigma_high: params.sigma_high.unwrap_or(3.0),
        max_iterations: params.max_iterations.unwrap_or(5),
        align: params.align.unwrap_or(true),
        align_method: match params.align_method.as_deref() {
            Some("affine") => AlignMethod::Affine,
            _ => AlignMethod::PhaseCorrelation,
        },
        weights: params.weights,
    };

    let result_slot = params.result_slot.unwrap_or_else(|| "stacked".into());
    let paths = params.paths;

    let (job, tx) = new_job("stack");
    let job_id = job.id.clone();
    session.jobs.insert(job_id.clone(), Arc::clone(&job));

    let cache = Arc::clone(&session.cache);
    let slot = result_slot.clone();

    tokio::task::spawn_blocking(move || {
        let _permit = permit;

        tx.blocking_send(SseEvent::Progress { pct: 0, stage: "loading".into() }).ok();

        match stack_from_paths(&paths, &config, None) {
            Ok(result) => {
                job.set_pct(90);
                tx.blocking_send(SseEvent::Progress { pct: 90, stage: "storing".into() }).ok();

                let stats = compute_image_stats(&result.image);
                cache.insert_synthetic(&slot, Arc::new(result.image), stats);

                job.set_done();
                tx.blocking_send(SseEvent::Complete).ok();
            }
            Err(e) => {
                job.set_error();
                tx.blocking_send(SseEvent::Error { message: format!("{:#}", e) }).ok();
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job_id, "status": "running", "slot": result_slot })),
    ))
}

pub async fn drizzle(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
    Json(params): Json<DrizzleParams>,
) -> Result<(StatusCode, Json<Value>)> {
    if params.paths.is_empty() {
        return Err(AppError::BadRequest("paths must not be empty".into()));
    }

    let permit = state
        .job_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests)?;

    let config = DrizzleConfig {
        scale: params.scale.unwrap_or(2.0),
        pixfrac: params.pixfrac.unwrap_or(0.7),
        kernel: parse_kernel(params.kernel.as_deref()),
        sigma_low: params.sigma_low.unwrap_or(3.0),
        sigma_high: params.sigma_high.unwrap_or(3.0),
        align: params.align.unwrap_or(true),
        ..DrizzleConfig::default()
    };

    let result_slot = params.result_slot.unwrap_or_else(|| "drizzled".into());
    let paths = params.paths;

    let (job, tx) = new_job("drizzle");
    let job_id = job.id.clone();
    session.jobs.insert(job_id.clone(), Arc::clone(&job));

    let cache = Arc::clone(&session.cache);
    let slot = result_slot.clone();

    tokio::task::spawn_blocking(move || {
        let _permit = permit;

        tx.blocking_send(SseEvent::Progress { pct: 0, stage: "loading".into() }).ok();

        match drizzle_from_paths(&paths, &config, None) {
            Ok(result) => {
                job.set_pct(90);
                tx.blocking_send(SseEvent::Progress { pct: 90, stage: "storing".into() }).ok();

                let stats = compute_image_stats(&result.image);
                cache.insert_synthetic(&slot, Arc::new(result.image), stats);

                job.set_done();
                tx.blocking_send(SseEvent::Complete).ok();
            }
            Err(e) => {
                job.set_error();
                tx.blocking_send(SseEvent::Error { message: format!("{:#}", e) }).ok();
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job_id, "status": "running", "slot": result_slot })),
    ))
}
