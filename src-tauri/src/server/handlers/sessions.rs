// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};

use crate::error::{AppError, Result};
use crate::state::AppState;

pub async fn create(State(state): State<AppState>) -> Result<(StatusCode, Json<Value>)> {
    let session = state.create_session().ok_or_else(|| {
        AppError::ServiceUnavailable(format!(
            "session cap reached (max {})",
            state.config.session_max
        ))
    })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "session_id": session.id })),
    ))
}
