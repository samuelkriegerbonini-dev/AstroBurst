// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use ndarray::Array2;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::core::stacking::calibration::{drizzle_from_paths, stack_from_paths};
use astroburst_lib::infra::cache::ImageCache;
use astroburst_lib::types::compose::AlignMethod;
use astroburst_lib::types::stacking::{
    CombineMethod, DrizzleConfig, DrizzleKernel, NormalizationMethod, RejectionMethod,
    RejectionNormalization, StackConfig,
};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::job::{new_job, Job, SseEvent};
use crate::state::AppState;

fn parse_kernel(s: Option<&str>) -> DrizzleKernel {
    match s {
        Some("gaussian") => DrizzleKernel::Gaussian,
        Some("lanczos3") | Some("lanczos") => DrizzleKernel::Lanczos3,
        _ => DrizzleKernel::Square,
    }
}

fn publish_result(
    job: &Job,
    tx: &mpsc::Sender<SseEvent>,
    cache: &ImageCache,
    slot: &str,
    outcome: anyhow::Result<Array2<f32>>,
) {
    if job.cancel.is_cancelled() {
        return;
    }
    match outcome {
        Ok(image) => {
            job.set_pct(90);
            tx.blocking_send(SseEvent::Progress { pct: 90, stage: "storing".into() }).ok();

            let stats = compute_image_stats(&image);
            cache.insert_synthetic(slot, Arc::new(image), stats);

            job.set_done();
            tx.blocking_send(SseEvent::Complete).ok();
        }
        Err(e) => {
            job.set_error();
            tx.blocking_send(SseEvent::Error { message: format!("{:#}", e) }).ok();
        }
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
    pub rejection: Option<String>,
    pub combine: Option<String>,
    pub normalization: Option<String>,
    pub rejection_normalization: Option<String>,
    pub winsor_cutoff: Option<f32>,
    pub percentile_low: Option<f32>,
    pub percentile_high: Option<f32>,
    pub minmax_low: Option<usize>,
    pub minmax_high: Option<usize>,
    pub rejection_maps: Option<bool>,
}

fn parse_named<T>(name: Option<&str>, parse: fn(&str) -> std::result::Result<T, String>, default: T) -> Result<T> {
    match name {
        Some(n) => parse(n).map_err(AppError::BadRequest),
        None => Ok(default),
    }
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
    pub rejection: Option<String>,
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

    let defaults = StackConfig::default();
    let config = StackConfig {
        sigma_low: params.sigma_low.unwrap_or(defaults.sigma_low),
        sigma_high: params.sigma_high.unwrap_or(defaults.sigma_high),
        max_iterations: params.max_iterations.unwrap_or(defaults.max_iterations),
        align: params.align.unwrap_or(true),
        align_method: match params.align_method.as_deref() {
            Some("affine") => AlignMethod::Affine,
            _ => AlignMethod::PhaseCorrelation,
        },
        weights: params.weights,
        rejection: parse_named(params.rejection.as_deref(), RejectionMethod::from_name, defaults.rejection)?,
        combine: parse_named(params.combine.as_deref(), CombineMethod::from_name, defaults.combine)?,
        normalization: parse_named(
            params.normalization.as_deref(),
            NormalizationMethod::from_name,
            defaults.normalization,
        )?,
        rejection_normalization: parse_named(
            params.rejection_normalization.as_deref(),
            RejectionNormalization::from_name,
            defaults.rejection_normalization,
        )?,
        winsor_cutoff: params.winsor_cutoff.unwrap_or(defaults.winsor_cutoff),
        percentile_low: params.percentile_low.unwrap_or(defaults.percentile_low),
        percentile_high: params.percentile_high.unwrap_or(defaults.percentile_high),
        minmax_low: params.minmax_low.unwrap_or(defaults.minmax_low),
        minmax_high: params.minmax_high.unwrap_or(defaults.minmax_high),
        rejection_maps: params.rejection_maps.unwrap_or(false),
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

        let outcome = stack_from_paths(&paths, &config, None).map(|r| r.image);
        publish_result(&job, &tx, &cache, &slot, outcome);
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
        rejection: parse_named(
            params.rejection.as_deref(),
            RejectionMethod::from_name,
            RejectionMethod::default(),
        )?,
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

        let outcome = drizzle_from_paths(&paths, &config, None).map(|r| r.image);
        publish_result(&job, &tx, &cache, &slot, outcome);
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job_id, "status": "running", "slot": result_slot })),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::job::JobStatus;

    fn setup() -> (Arc<Job>, mpsc::Sender<SseEvent>, mpsc::Receiver<SseEvent>, ImageCache) {
        let (job, tx) = new_job("stack");
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

    #[test]
    fn cancelled_job_ignores_ok_result() {
        let (job, tx, mut rx, cache) = setup();
        job.cancel.cancel();
        job.set_cancelled();

        publish_result(&job, &tx, &cache, "stacked", Ok(Array2::<f32>::zeros((4, 4))));

        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert_ne!(job.pct.load(Ordering::Relaxed), 100);
        assert!(cache.get("stacked").is_none());
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn cancelled_job_ignores_error_result() {
        let (job, tx, mut rx, cache) = setup();
        job.cancel.cancel();
        job.set_cancelled();

        publish_result(&job, &tx, &cache, "stacked", Err(anyhow::anyhow!("boom")));

        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn running_job_stores_result_and_completes() {
        let (job, tx, mut rx, cache) = setup();

        publish_result(&job, &tx, &cache, "stacked", Ok(Array2::<f32>::ones((4, 4))));

        assert_eq!(job.current_status(), JobStatus::Done);
        assert_eq!(job.pct.load(Ordering::Relaxed), 100);
        assert!(cache.get("stacked").is_some());
        assert!(matches!(drain(&mut rx).last(), Some(SseEvent::Complete)));
    }

    #[test]
    fn running_job_reports_error() {
        let (job, tx, mut rx, cache) = setup();

        publish_result(&job, &tx, &cache, "stacked", Err(anyhow::anyhow!("boom")));

        assert_eq!(job.current_status(), JobStatus::Error);
        assert!(cache.get("stacked").is_none());
        assert!(matches!(drain(&mut rx).last(), Some(SseEvent::Error { .. })));
    }
}
