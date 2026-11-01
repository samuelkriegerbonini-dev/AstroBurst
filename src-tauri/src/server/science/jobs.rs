use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ndarray::Array2;
use serde_json::{json, Value};

use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::infra::progress::ProgressHandle;
use astroburst_lib::types::error::CodedError;
use astroburst_lib::types::header::HduHeader;

use super::error::ScienceError;
use super::outcome::JobOutcome;
use super::progress::job_progress;
use super::state::ScienceState;
use crate::error::AppError;
use crate::job::{new_job, spawn_job, Job};
use crate::session::Session;
use crate::state::AppState;
use crate::v2::images::register_and_respond;

pub(crate) struct Product {
    pub(crate) name: String,
    pub(crate) image: Array2<f32>,
    pub(crate) header: HduHeader,
}

pub(crate) struct Finished {
    pub(crate) result: Value,
    pub(crate) warnings: Vec<String>,
    pub(crate) products: Vec<Product>,
}

pub(crate) fn publish_products(session: &Session, products: Vec<Product>) -> Vec<String> {
    products
        .into_iter()
        .filter_map(|product| {
            let stats = compute_image_stats(&product.image);
            session.cache.insert_synthetic_with_header(
                &product.name,
                Arc::new(product.image),
                stats,
                Some(product.header),
            );
            let entry = session.cache.get(&product.name)?;
            register_and_respond(
                session,
                product.name.clone(),
                None,
                None,
                &entry,
                Value::Null,
            );
            Some(product.name)
        })
        .collect()
}

pub(crate) fn run_science_job<F>(
    state: &AppState,
    session: &Arc<Session>,
    science: &Arc<ScienceState>,
    action: &str,
    total: u64,
    body: F,
) -> Result<(StatusCode, Json<Value>), ScienceError>
where
    F: FnOnce(&Job, &ProgressHandle) -> anyhow::Result<Finished> + Send + 'static,
{
    let permit = state
        .job_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests(state.config.jobs_max))?;
    let job = new_job(action);
    let job_id = job.id.clone();
    session.jobs.insert(job_id.clone(), Arc::clone(&job));
    let progress = job_progress(&job, total);
    let (state, session, science) = (state.clone(), Arc::clone(session), Arc::clone(science));
    spawn_job(job, permit, move |job| {
        let finished = body(job, &progress);
        settle(&state, &session, &science, job, finished);
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job_id, "status": "running" })),
    ))
}

fn settle(
    state: &AppState,
    session: &Session,
    science: &ScienceState,
    job: &Job,
    finished: anyhow::Result<Finished>,
) {
    if !job.begin_commit() {
        return;
    }
    let outcome = match finished {
        Ok(Finished {
            result,
            warnings,
            products,
        }) => {
            let products = publish_products(session, products);
            JobOutcome {
                result,
                warnings,
                products,
                error: None,
                error_code: None,
                error_hint: None,
            }
        }
        Err(e) => {
            let coded = e.downcast_ref::<CodedError>();
            JobOutcome {
                result: Value::Null,
                warnings: Vec::new(),
                products: Vec::new(),
                error: Some(format!("{e:#}")),
                error_code: coded.map(|coded| coded.code),
                error_hint: coded.and_then(|coded| coded.hint.clone()),
            }
        }
    };
    let error = outcome.error.clone();
    science
        .outcomes
        .record(state, &session.id, &job.id, outcome);
    match error {
        Some(message) => job.set_error(message),
        None => job.set_done(),
    };
}
