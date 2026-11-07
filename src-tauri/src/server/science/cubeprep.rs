use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde_json::{json, Value};

use astroburst_lib::infra::paths::plain;
use astroburst_lib::service::cubeprep::filter_registry;
use astroburst_lib::service::cubeprep_recipe::{run_recipe, Recipe, FILE_NOT_FOUND};
use astroburst_lib::types::error::CodedError;

use super::error::ScienceError;
use super::guard::confine;
use super::jobs::{run_science_job, Finished};
use super::state::ScienceState;
use crate::error::AppError;
use crate::extractors::SessionExtractor;
use crate::state::AppState;

const ACTION: &str = "cubeprep";

pub(crate) async fn filters() -> Result<Json<Value>, ScienceError> {
    let registry = tokio::task::spawn_blocking(filter_registry)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?;
    Ok(Json(json!({
        "datasets": registry.datasets(),
        "filters": registry.entries(),
    })))
}

pub(crate) async fn run(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
    Extension(science): Extension<Arc<ScienceState>>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ScienceError> {
    let recipe = Recipe::from_value(&body)
        .map_err(refused)?
        .resolved(&plain(&science.cfg.data_root));
    let guarded = science.cfg.guarded();
    if guarded {
        let paths = recipe.paths().into_iter().map(str::to_owned).collect();
        confine(&science.cfg, paths).await?;
    }
    let (recipe, found, advisories) = tokio::task::spawn_blocking(move || {
        let advisories = recipe.validate()?;
        let found = if guarded {
            recipe.empirical_files()?
        } else {
            Vec::new()
        };
        Ok::<_, CodedError>((recipe, found, advisories))
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(refused)?;
    if guarded {
        confine(&science.cfg, found).await?;
    }
    let output_root = plain(&science.cfg.output_root);
    let job_root = output_root.clone();
    let steps = recipe.steps.len() as u64;
    let (status, Json(mut accepted)) = run_science_job(
        &state,
        &session,
        &science,
        ACTION,
        steps,
        move |job, progress| {
            let report = run_recipe(&recipe, &output_dir(&job_root, &job.id), progress)?;
            Ok(Finished {
                warnings: report.warnings.clone(),
                result: serde_json::to_value(&report)?,
                products: Vec::new(),
            })
        },
    )?;
    if let Some(job_id) = accepted["job_id"].as_str().map(str::to_owned) {
        accepted["output_dir"] = json!(output_dir(&output_root, &job_id).to_string_lossy());
    }
    accepted["warnings"] = json!(advisories);
    Ok((status, Json(accepted)))
}

fn output_dir(root: &Path, job_id: &str) -> PathBuf {
    root.join(format!("{ACTION}-{job_id}"))
}

fn refused(error: CodedError) -> ScienceError {
    let CodedError {
        code,
        message,
        hint,
    } = error;
    if code == FILE_NOT_FOUND {
        ScienceError::not_found(code, message, hint)
    } else {
        ScienceError::bad_request(code, message, hint)
    }
}
