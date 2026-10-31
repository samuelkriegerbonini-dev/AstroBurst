#[cfg(feature = "mast")]
use std::sync::Arc;

use axum::extract::State;
#[cfg(feature = "mast")]
use axum::Extension;
use axum::Json;
use serde_json::{json, Value};

#[cfg(feature = "mast")]
use super::state::ScienceState;
use crate::state::AppState;

const ROUTES: [(&str, &str); 43] = [
    ("GET", "/health"),
    ("POST", "/sessions"),
    ("POST", "/sessions/:sid/fits/open"),
    ("POST", "/sessions/:sid/fits/header"),
    ("POST", "/sessions/:sid/image/render"),
    ("POST", "/sessions/:sid/image/stf"),
    ("POST", "/sessions/:sid/image/viewport"),
    ("GET", "/sessions/:sid/jobs/:jid"),
    ("DELETE", "/sessions/:sid/jobs/:jid"),
    ("GET", "/sessions/:sid/jobs/:jid/stream"),
    ("POST", "/sessions/:sid/stacking/stack"),
    ("POST", "/sessions/:sid/stacking/drizzle"),
    ("POST", "/sessions/:sid/pipeline/run"),
    ("POST", "/v2/sessions"),
    ("GET", "/v2/sessions/:sid"),
    ("DELETE", "/v2/sessions/:sid"),
    ("POST", "/v2/sessions/:sid/keepalive"),
    ("POST", "/v2/sessions/:sid/open"),
    ("POST", "/v2/sessions/:sid/hdu"),
    ("GET", "/v2/sessions/:sid/images"),
    ("GET", "/v2/sessions/:sid/structure"),
    ("GET", "/v2/sessions/:sid/header"),
    ("GET", "/v2/sessions/:sid/wcs"),
    ("POST", "/v2/sessions/:sid/cutout"),
    ("POST", "/v2/sessions/:sid/wcs/pix2sky"),
    ("POST", "/v2/sessions/:sid/wcs/sky2pix"),
    ("POST", "/v2/sessions/:sid/wcs/separation"),
    ("POST", "/v2/sessions/:sid/wcs/grid"),
    ("POST", "/v2/sessions/:sid/bin"),
    ("POST", "/v2/sessions/:sid/pixel"),
    ("POST", "/v2/sessions/:sid/stats"),
    ("POST", "/v2/sessions/:sid/histogram"),
    ("POST", "/v2/sessions/:sid/render"),
    ("GET", "/v2/sessions/:sid/jobs/:jid"),
    ("DELETE", "/v2/sessions/:sid/jobs/:jid"),
    ("GET", "/v2/sessions/:sid/jobs/:jid/stream"),
    ("GET", "/v2/sessions/:sid/jobs/:jid/outcome"),
    ("GET", "/v2/sessions/:sid/planes"),
    ("DELETE", "/v2/sessions/:sid/images/:ref"),
    ("GET", "/v2/sessions/:sid/images/:ref/fits"),
    ("GET", "/v2/fs"),
    ("GET", "/v2/capabilities"),
    ("POST", "/v2/sessions/:sid/synth"),
];

pub(crate) async fn capabilities(
    State(state): State<AppState>,
    #[cfg(feature = "mast")] Extension(science): Extension<Arc<ScienceState>>,
) -> Json<Value> {
    let config = &state.config;
    let body = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "features": {
            "mast": cfg!(feature = "mast"),
            "astrometry_net": cfg!(feature = "astrometry-net"),
            "vizier": cfg!(feature = "vizier"),
            "asdf_full": cfg!(feature = "asdf-full"),
        },
        "limits": {
            "session_max": config.session_max,
            "session_ttl_seconds": config.session_ttl.as_secs(),
            "jobs_max": config.jobs_max,
            "cache_max_entries": config.cache_max_entries,
            "cache_max_bytes": config.cache_max_bytes,
        },
        "routes": ROUTES
            .iter()
            .map(|(method, path)| json!({ "method": method, "path": path }))
            .collect::<Vec<_>>(),
    });
    #[cfg(feature = "mast")]
    let body = with_mast_token(body, &science);
    Json(body)
}

#[cfg(feature = "mast")]
fn with_mast_token(mut body: Value, science: &ScienceState) -> Value {
    body["mast_token"] = json!({ "present": science.cfg.mast.token.is_some() });
    body
}
