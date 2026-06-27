// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;

use super::error::AppError;
use super::session::Session;
use super::state::AppState;

pub struct SessionExtractor(pub Arc<Session>);

#[axum::async_trait]
impl FromRequestParts<AppState> for SessionExtractor {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Path(params) = Path::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map_err(|e| AppError::BadRequest(format!("invalid path: {e}")))?;

        let sid = params
            .get("sid")
            .ok_or_else(|| AppError::BadRequest("route missing :sid segment".into()))?
            .clone();

        let session = state
            .sessions
            .get(&sid)
            .map(|e| Arc::clone(e.value()))
            .ok_or_else(|| AppError::NotFound(format!("session {sid} not found")))?;

        session.touch().await;
        Ok(SessionExtractor(session))
    }
}
