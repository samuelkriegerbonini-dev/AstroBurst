pub(crate) mod capabilities;
pub(crate) mod codes;
pub(crate) mod config;
pub(crate) mod cubeprep;
pub(crate) mod error;
pub(crate) mod fs;
pub(crate) mod guard;
pub(crate) mod images;
pub(crate) mod inspect;
pub(crate) mod jobs;
pub(crate) mod outcome;
pub(crate) mod progress;
pub(crate) mod resolve;
pub(crate) mod state;
pub(crate) mod synth;
#[cfg(test)]
mod tests;

use axum::routing::{delete, get, post};
use axum::Router;

use crate::handlers::jobs::{cancel_job, get_job, stream_job};
use crate::state::AppState;

pub(crate) use state::ScienceState;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v2/sessions/:sid/jobs/:jid",
            get(get_job).delete(cancel_job),
        )
        .route("/v2/sessions/:sid/jobs/:jid/stream", get(stream_job))
        .route("/v2/sessions/:sid/jobs/:jid/outcome", get(outcome::outcome))
        .route("/v2/sessions/:sid/planes", get(inspect::planes))
        .route("/v2/sessions/:sid/images/:ref", delete(images::delete))
        .route("/v2/sessions/:sid/images/:ref/fits", get(images::fits))
        .route("/v2/fs", get(fs::list))
        .route("/v2/capabilities", get(capabilities::capabilities))
        .route("/v2/sessions/:sid/synth", post(synth::synth))
        .route("/v2/cubeprep/filters", get(cubeprep::filters))
        .route("/v2/sessions/:sid/cubeprep/jobs", post(cubeprep::run))
}
