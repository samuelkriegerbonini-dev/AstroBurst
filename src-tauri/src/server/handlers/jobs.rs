// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt as _;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::job::{Job, JobStatus, SseEvent};
use crate::state::AppState;

fn job_json(job: &Arc<Job>) -> Value {
    json!({
        "id": job.id,
        "action": job.action,
        "status": job.current_status(),
        "pct": job.pct.load(std::sync::atomic::Ordering::Relaxed),
        "started_at": job.started_at,
        "completed_at": *job.completed_at.lock().unwrap(),
    })
}

fn resolve_job(session: &crate::session::Session, jid: &str) -> Result<Arc<Job>> {
    session
        .jobs
        .get(jid)
        .map(|e| Arc::clone(e.value()))
        .ok_or_else(|| AppError::NotFound(format!("job {jid} not found")))
}

fn sse_event_type(evt: &SseEvent) -> &'static str {
    match evt {
        SseEvent::Progress { .. } => "progress",
        SseEvent::Complete => "complete",
        SseEvent::Error { .. } => "error",
    }
}

pub async fn get_job(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Path(params): Path<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let jid = params
        .get("jid")
        .ok_or_else(|| AppError::BadRequest("missing :jid".into()))?;
    Ok(Json(job_json(&resolve_job(&session, jid)?)))
}

pub async fn cancel_job(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Path(params): Path<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let jid = params
        .get("jid")
        .ok_or_else(|| AppError::BadRequest("missing :jid".into()))?;
    let job = resolve_job(&session, jid)?;

    if job.current_status() == JobStatus::Running {
        job.cancel.cancel();
        job.set_cancelled();
    }

    Ok(Json(job_json(&job)))
}

pub async fn stream_job(
    SessionExtractor(session): SessionExtractor,
    State(_state): State<AppState>,
    Path(params): Path<HashMap<String, String>>,
) -> Result<Response> {
    let jid = params
        .get("jid")
        .ok_or_else(|| AppError::BadRequest("missing :jid".into()))?;
    let job = resolve_job(&session, jid)?;

    let rx = {
        let mut guard = job.rx.lock().unwrap();
        guard
            .take()
            .ok_or_else(|| AppError::Conflict("SSE stream already subscribed for this job".into()))?
    };

    let stream = ReceiverStream::new(rx).map(|evt: SseEvent| {
        let json = serde_json::to_string(&evt).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().event(sse_event_type(&evt)).data(json))
    });

    Ok(Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("keep-alive"),
        )
        .into_response())
}
