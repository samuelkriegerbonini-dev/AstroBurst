// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use ndarray::Array2;
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::core::stacking::combine::{stack_images_cancellable, validate_frame_weights};
use astroburst_lib::core::stacking::drizzle::drizzle_stack_cancellable;
use astroburst_lib::core::stacking::CancelCheck;
use astroburst_lib::infra::cache::ImageCache;
use astroburst_lib::infra::fits::reader::load_fits_image;
use astroburst_lib::types::compose::AlignMethod;
use astroburst_lib::types::error::AppError as CoreError;
use astroburst_lib::types::stacking::{
    CombineMethod, DrizzleConfig, DrizzleKernel, NormalizationMethod, RejectionMethod,
    RejectionNormalization, StackConfig,
};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::job::{new_job, spawn_job, Job};
use crate::state::AppState;

const LOAD_PCT: usize = 60;

fn parse_kernel(s: Option<&str>) -> DrizzleKernel {
    match s {
        Some("gaussian") => DrizzleKernel::Gaussian,
        Some("lanczos3") | Some("lanczos") => DrizzleKernel::Lanczos3,
        _ => DrizzleKernel::Square,
    }
}

pub(crate) fn require_finite(name: &str, value: f64) -> Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("{name} must be a finite number")))
    }
}

pub(crate) fn require_positive(name: &str, value: f64) -> Result<()> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("{name} must be a finite number greater than 0, got {value}")))
    }
}

fn validate_stack_config(config: &StackConfig, frames: usize) -> Result<()> {
    require_positive("sigma_low", config.sigma_low as f64)?;
    require_positive("sigma_high", config.sigma_high as f64)?;
    require_finite("winsor_cutoff", config.winsor_cutoff as f64)?;
    require_finite("percentile_low", config.percentile_low as f64)?;
    require_finite("percentile_high", config.percentile_high as f64)?;
    validate_frame_weights(config.weights.as_deref(), frames).map_err(|e| AppError::BadRequest(format!("{e:#}")))?;
    let rejected = config.minmax_low.checked_add(config.minmax_high).ok_or_else(|| {
        AppError::BadRequest("minmax_low + minmax_high is too large".into())
    })?;
    if config.rejection == RejectionMethod::MinMax && rejected >= frames {
        return Err(AppError::BadRequest(format!(
            "minmax_low + minmax_high ({rejected}) must be smaller than the number of frames ({frames})"
        )));
    }
    Ok(())
}

fn validate_drizzle_config(config: &DrizzleConfig) -> Result<()> {
    require_positive("scale", config.scale)?;
    require_positive("pixfrac", config.pixfrac)?;
    require_positive("sigma_low", config.sigma_low as f64)?;
    require_positive("sigma_high", config.sigma_high as f64)
}

pub(crate) fn stop_if_cancelled(cancelled: CancelCheck) -> anyhow::Result<()> {
    if cancelled() {
        return Err(CoreError::Cancelled.into());
    }
    Ok(())
}

fn load_frames(job: &Job, paths: &[String], cancelled: CancelCheck) -> anyhow::Result<Vec<Array2<f32>>> {
    job.progress(0, "loading");
    let mut frames = Vec::with_capacity(paths.len());
    for (i, path) in paths.iter().enumerate() {
        stop_if_cancelled(cancelled)?;
        frames.push(load_fits_image(path)?);
        job.progress(((i + 1) * LOAD_PCT / paths.len().max(1)) as u32, "loading");
    }
    stop_if_cancelled(cancelled)?;
    Ok(frames)
}

fn stack_frames(job: &Job, paths: &[String], config: &StackConfig, cancelled: CancelCheck) -> anyhow::Result<Array2<f32>> {
    let frames = load_frames(job, paths, cancelled)?;
    job.progress(LOAD_PCT as u32, "stacking");
    Ok(stack_images_cancellable(&frames, config, cancelled)?.image)
}

fn drizzle_frames(job: &Job, paths: &[String], config: &DrizzleConfig, cancelled: CancelCheck) -> anyhow::Result<Array2<f32>> {
    let frames = load_frames(job, paths, cancelled)?;
    job.progress(LOAD_PCT as u32, "drizzling");
    Ok(drizzle_stack_cancellable(&frames, config, cancelled)?.image)
}

fn publish_result(job: &Job, cache: &ImageCache, slot: &str, outcome: anyhow::Result<Array2<f32>>) {
    match outcome {
        Ok(image) => {
            job.progress(90, "storing");
            if !job.begin_commit() {
                return;
            }
            let stats = compute_image_stats(&image);
            cache.insert_synthetic(slot, Arc::new(image), stats);
            job.set_done();
        }
        Err(e) => {
            job.set_error(format!("{:#}", e));
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
    validate_stack_config(&config, params.paths.len())?;

    let permit = state
        .job_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests(state.config.jobs_max))?;

    let result_slot = params.result_slot.unwrap_or_else(|| "stacked".into());
    let paths = params.paths;

    let job = new_job("stack");
    let job_id = job.id.clone();
    session.jobs.insert(job_id.clone(), Arc::clone(&job));

    let cache = Arc::clone(&session.cache);
    let slot = result_slot.clone();

    spawn_job(job, permit, move |job| {
        let cancelled = || job.cancel.is_cancelled();
        let outcome = stack_frames(job, &paths, &config, &cancelled);
        publish_result(job, &cache, &slot, outcome);
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
    validate_drizzle_config(&config)?;

    let permit = state
        .job_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests(state.config.jobs_max))?;

    let result_slot = params.result_slot.unwrap_or_else(|| "drizzled".into());
    let paths = params.paths;

    let job = new_job("drizzle");
    let job_id = job.id.clone();
    session.jobs.insert(job_id.clone(), Arc::clone(&job));

    let cache = Arc::clone(&session.cache);
    let slot = result_slot.clone();

    spawn_job(job, permit, move |job| {
        let cancelled = || job.cancel.is_cancelled();
        let outcome = drizzle_frames(job, &paths, &config, &cancelled);
        publish_result(job, &cache, &slot, outcome);
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job_id, "status": "running", "slot": result_slot })),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use tokio::sync::mpsc;

    use super::*;
    use crate::job::{cancel_after_stage_started, JobStatus, SseEvent};

    fn setup() -> (Arc<Job>, mpsc::Receiver<SseEvent>, ImageCache) {
        let job = new_job("stack");
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

    #[test]
    fn cancelled_job_ignores_ok_result() {
        let (job, mut rx, cache) = setup();
        job.set_cancelled();

        publish_result(&job, &cache, "stacked", Ok(Array2::<f32>::zeros((4, 4))));

        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert_ne!(job.pct.load(Ordering::Relaxed), 100);
        assert!(cache.get("stacked").is_none());
        assert!(matches!(drain(&mut rx).as_slice(), [SseEvent::Cancelled]));
    }

    #[test]
    fn cancelled_job_ignores_error_result() {
        let (job, mut rx, cache) = setup();
        job.set_cancelled();

        publish_result(&job, &cache, "stacked", Err(anyhow::anyhow!("boom")));

        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert!(matches!(drain(&mut rx).as_slice(), [SseEvent::Cancelled]));
    }

    #[test]
    fn running_job_stores_result_and_completes() {
        let (job, mut rx, cache) = setup();

        publish_result(&job, &cache, "stacked", Ok(Array2::<f32>::ones((4, 4))));

        assert_eq!(job.current_status(), JobStatus::Done);
        assert_eq!(job.pct.load(Ordering::Relaxed), 100);
        assert!(cache.get("stacked").is_some());
        assert!(matches!(drain(&mut rx).last(), Some(SseEvent::Complete)));
    }

    #[test]
    fn running_job_reports_error() {
        let (job, mut rx, cache) = setup();

        publish_result(&job, &cache, "stacked", Err(anyhow::anyhow!("boom")));

        assert_eq!(job.current_status(), JobStatus::Error);
        assert!(cache.get("stacked").is_none());
        assert!(matches!(drain(&mut rx).last(), Some(SseEvent::Error { .. })));
    }

    fn is_cancellation(err: &anyhow::Error) -> bool {
        matches!(err.downcast_ref::<CoreError>(), Some(CoreError::Cancelled))
    }

    fn write_frames(dir: &tempfile::TempDir, count: usize) -> Vec<String> {
        (0..count)
            .map(|i| {
                let path = dir.path().join(format!("frame{i}.fits"));
                crate::tests::v2_fixtures::write_pixels_fits(&path, 8, 8, &[100.0 + i as f32; 64]);
                path.to_str().unwrap().to_string()
            })
            .collect()
    }

    #[test]
    fn frame_loading_stops_at_the_next_frame_once_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let (job, _rx, _cache) = setup();
        job.set_cancelled();
        let missing = dir.path().join("missing.fits").to_str().unwrap().to_string();
        let err = load_frames(&job, &[missing], &|| job.cancel.is_cancelled()).unwrap_err();
        assert!(is_cancellation(&err), "{err:#}");
    }

    #[test]
    fn a_cancel_raised_while_frames_are_combined_stops_the_stack() {
        let dir = tempfile::tempdir().unwrap();
        let paths = write_frames(&dir, 3);
        let config = StackConfig { align: false, ..StackConfig::default() };
        let (job, _rx, _cache) = setup();
        let err = stack_frames(&job, &paths, &config, &cancel_after_stage_started(&job, LOAD_PCT as u32)).unwrap_err();
        assert!(is_cancellation(&err), "{err:#}");

        let (fresh, _rx, _cache) = setup();
        assert_eq!(stack_frames(&fresh, &paths, &config, &|| false).unwrap().dim(), (8, 8));
    }

    #[test]
    fn a_cancel_raised_while_frames_are_drizzled_stops_the_drizzle() {
        let dir = tempfile::tempdir().unwrap();
        let paths = write_frames(&dir, 3);
        let config = DrizzleConfig { align: false, ..DrizzleConfig::default() };
        let (job, _rx, _cache) = setup();
        let err = drizzle_frames(&job, &paths, &config, &cancel_after_stage_started(&job, LOAD_PCT as u32)).unwrap_err();
        assert!(is_cancellation(&err), "{err:#}");

        let (fresh, _rx, _cache) = setup();
        assert!(drizzle_frames(&fresh, &paths, &config, &|| false).is_ok());
    }

    #[test]
    fn minmax_counts_are_checked_before_any_work_starts() {
        let config = |low, high| StackConfig {
            rejection: RejectionMethod::MinMax,
            minmax_low: low,
            minmax_high: high,
            ..StackConfig::default()
        };
        assert!(validate_stack_config(&config(usize::MAX, 1), 3).is_err());
        assert!(validate_stack_config(&config(2, 1), 3).is_err());
        assert!(validate_stack_config(&config(1, 1), 3).is_ok());
        let other = StackConfig { minmax_low: 5, minmax_high: 5, ..StackConfig::default() };
        assert!(validate_stack_config(&other, 3).is_ok());
        let nan_sigma = StackConfig { sigma_low: f32::NAN, ..StackConfig::default() };
        assert!(validate_stack_config(&nan_sigma, 3).is_err());
    }

    #[test]
    fn weight_count_is_checked_before_any_frame_is_loaded() {
        let weighted = |weights: Vec<f64>| StackConfig { weights: Some(weights), ..StackConfig::default() };
        assert!(validate_stack_config(&weighted(vec![1.0, 1.0]), 3).is_err());
        assert!(validate_stack_config(&weighted(vec![1.0, 1.0, 1.0, 1.0]), 3).is_err());
        assert!(validate_stack_config(&weighted(vec![1.0, f64::NAN, 1.0]), 3).is_err());
        assert!(validate_stack_config(&weighted(vec![1.0, -1.0, 1.0]), 3).is_err());
        assert!(validate_stack_config(&weighted(vec![1.0, 0.5, 2.0]), 3).is_ok());
    }

    #[test]
    fn drizzle_scale_and_pixfrac_must_be_positive_numbers() {
        for (scale, pixfrac) in [(f64::NAN, 0.7), (2.0, f64::INFINITY), (0.0, 0.7), (2.0, -1.0)] {
            let config = DrizzleConfig { scale, pixfrac, ..DrizzleConfig::default() };
            assert!(validate_drizzle_config(&config).is_err(), "{scale} {pixfrac}");
        }
        assert!(validate_drizzle_config(&DrizzleConfig::default()).is_ok());
    }
}
