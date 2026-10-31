use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::{Extension, Json};
use dashmap::DashMap;
use serde_json::{json, Value};

use super::error::ScienceError;
use super::state::ScienceState;
use crate::extractors::SessionExtractor;
use crate::handlers::jobs::get_job;
use crate::job::JobId;
use crate::session::{Session, SessionId};
use crate::state::AppState;

pub(crate) const FINISHED_JOBS_PER_SESSION: usize = 256;

pub(crate) struct JobOutcome {
    pub(crate) result: Value,
    pub(crate) warnings: Vec<String>,
    pub(crate) products: Vec<String>,
    pub(crate) error: Option<String>,
}

#[derive(Default)]
pub(crate) struct OutcomeStore(DashMap<(SessionId, JobId), Arc<JobOutcome>>);

impl OutcomeStore {
    pub(crate) fn record(&self, state: &AppState, session: &str, job: &str, outcome: JobOutcome) {
        self.0
            .insert((session.to_owned(), job.to_owned()), Arc::new(outcome));
        self.prune(state);
    }

    pub(crate) fn get(&self, session: &str, job: &str) -> Option<Arc<JobOutcome>> {
        self.0
            .get(&(session.to_owned(), job.to_owned()))
            .map(|entry| Arc::clone(entry.value()))
    }

    pub(crate) fn prune(&self, state: &AppState) {
        self.0
            .retain(|(sid, _), _| state.sessions.contains_key(sid));
        let sessions: Vec<Arc<Session>> = state
            .sessions
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect();
        for session in &sessions {
            for jid in finished_jobs_over_the_cap(session) {
                session.jobs.remove(&jid);
                self.0.remove(&(session.id.clone(), jid));
            }
        }
    }
}

pub(crate) async fn outcome(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
    Extension(science): Extension<Arc<ScienceState>>,
    Path(params): Path<HashMap<String, String>>,
) -> Result<Json<Value>, ScienceError> {
    science.outcomes.prune(&state);
    let jid = params.get("jid").cloned().unwrap_or_default();
    let Json(job) = get_job(
        SessionExtractor(Arc::clone(&session)),
        State(state),
        Path(params),
    )
    .await?;
    let outcome = science.outcomes.get(&session.id, &jid).map(|recorded| {
        json!({
            "result": recorded.result,
            "warnings": recorded.warnings,
            "products": recorded
                .products
                .iter()
                .map(|name| json!({ "ref": name, "present": session.cache.contains(name) }))
                .collect::<Vec<_>>(),
            "error": recorded.error,
        })
    });
    Ok(Json(json!({ "job": job, "outcome": outcome })))
}

fn finished_jobs_over_the_cap(session: &Session) -> Vec<JobId> {
    let mut finished: Vec<(Option<u64>, u64, JobId)> = session
        .jobs
        .iter()
        .filter(|entry| !entry.value().is_active())
        .map(|entry| {
            (
                entry.value().completed_at(),
                entry.value().started_at,
                entry.key().clone(),
            )
        })
        .collect();
    let excess = finished.len().saturating_sub(FINISHED_JOBS_PER_SESSION);
    finished.sort();
    finished
        .into_iter()
        .take(excess)
        .map(|(_, _, jid)| jid)
        .collect()
}
