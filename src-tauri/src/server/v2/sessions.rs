// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};

use crate::error::Result;
use crate::extractors::SessionExtractor;
use crate::state::AppState;

pub async fn status(SessionExtractor(session): SessionExtractor) -> Result<Json<Value>> {
    let active = session.v2.active_ref.read().await.clone();
    Ok(Json(json!({
        "session_id": session.id,
        "active_ref": active,
        "image_count": session.v2.meta.len(),
        "cache_bytes": session.cache.memory_estimate_bytes(),
    })))
}

pub async fn delete(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
) -> Result<StatusCode> {
    state.sessions.remove(&session.id);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn keepalive(SessionExtractor(session): SessionExtractor) -> Result<Json<Value>> {
    Ok(Json(json!({ "session_id": session.id, "status": "ok" })))
}
