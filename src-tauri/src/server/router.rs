// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{
    extract::{Request, State},
    http::HeaderValue,
    middleware::{self, Next},
    response::Response,
    routing::{self, get},
    Json, Router,
};
use serde_json::json;
use std::sync::atomic::Ordering;
use uuid::Uuid;

use super::handlers::{io, jobs, pipeline, render, sessions, stacking};
use super::state::AppState;
use super::v2;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/sessions", routing::post(sessions::create))
        .route("/sessions/:sid/fits/open",   routing::post(io::fits_open))
        .route("/sessions/:sid/fits/header", routing::post(io::fits_header))
        .route("/sessions/:sid/image/render",   routing::post(render::render))
        .route("/sessions/:sid/image/stf",      routing::post(render::auto_render))
        .route("/sessions/:sid/image/viewport", routing::post(render::viewport))
        .route(
            "/sessions/:sid/jobs/:jid",
            routing::get(jobs::get_job).delete(jobs::cancel_job),
        )
        .route("/sessions/:sid/jobs/:jid/stream", routing::get(jobs::stream_job))
        .route("/sessions/:sid/stacking/stack",   routing::post(stacking::stack))
        .route("/sessions/:sid/stacking/drizzle", routing::post(stacking::drizzle))
        .route("/sessions/:sid/pipeline/run", routing::post(pipeline::run))
        .route("/v2/sessions", routing::post(sessions::create))
        .route(
            "/v2/sessions/:sid",
            get(v2::sessions::status).delete(v2::sessions::delete),
        )
        .route("/v2/sessions/:sid/keepalive", routing::post(v2::sessions::keepalive))
        .route("/v2/sessions/:sid/open", routing::post(v2::images::open))
        .route("/v2/sessions/:sid/hdu", routing::post(v2::images::switch_hdu))
        .route("/v2/sessions/:sid/images", get(v2::images::list_images))
        .route("/v2/sessions/:sid/structure", get(v2::inspect::structure))
        .route("/v2/sessions/:sid/header", get(v2::inspect::header))
        .route("/v2/sessions/:sid/wcs", get(v2::inspect::wcs))
        .route("/v2/sessions/:sid/cutout", routing::post(v2::cutout::cutout))
        .route("/v2/sessions/:sid/wcs/pix2sky", routing::post(v2::wcs::pix2sky))
        .route("/v2/sessions/:sid/wcs/sky2pix", routing::post(v2::wcs::sky2pix))
        .route("/v2/sessions/:sid/wcs/separation", routing::post(v2::wcs::separation))
        .route("/v2/sessions/:sid/bin", routing::post(v2::bin::bin))
        .route("/v2/sessions/:sid/pixel", routing::post(v2::pixel::pixel))
        .route("/v2/sessions/:sid/stats", routing::post(v2::stats::stats))
        .route("/v2/sessions/:sid/histogram", routing::post(v2::histogram::histogram))
        .route("/v2/sessions/:sid/render", routing::post(v2::render::handler::render))
        .with_state(state)
        .layer(middleware::from_fn(request_id))
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "sessions_active": state.sessions.len(),
        "sessions_total": state.created_total.load(Ordering::Relaxed),
    }))
}

async fn request_id(req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut res = next.run(req).await;
    if let Ok(val) = HeaderValue::from_str(&id) {
        res.headers_mut().insert("x-request-id", val);
    }
    res
}
