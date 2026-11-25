use std::sync::Arc;

use crate::science::config::ScienceConfig;
use crate::science::outcome::OutcomeStore;
use crate::state::AppState;

pub(crate) struct ScienceState {
    pub(crate) cfg: ScienceConfig,
    pub(crate) outcomes: OutcomeStore,
}

impl ScienceState {
    pub(crate) fn new(cfg: ScienceConfig) -> Self {
        match (cfg.guarded(), cfg.token.is_some()) {
            (true, false) => log::warn!(
                "ASTROBURST_BIND {} is not loopback and ASTROBURST_TOKEN is not set: every route answers 503 guard_unconfigured",
                cfg.bind
            ),
            (false, true) => log::warn!(
                "ASTROBURST_TOKEN is set but ASTROBURST_BIND {} is loopback: loopback stays open and the token is not checked",
                cfg.bind
            ),
            _ => {}
        }
        Self {
            cfg,
            outcomes: OutcomeStore::default(),
        }
    }

    pub(crate) fn from_env(state: &AppState) -> Arc<Self> {
        if let Some(warning) = panic_strategy_warning(cfg!(panic = "abort")) {
            log::warn!("{warning}");
        }
        Arc::new(Self::new(ScienceConfig::from_env(state.config.bind)))
    }
}

pub(crate) fn panic_strategy_warning(aborts: bool) -> Option<&'static str> {
    aborts.then_some(
        "this server was built with panic = \"abort\" (the release profile): a panic in a job or a request ends the process and every session; build it with --profile release-server, whose panic guard marks the job failed and keeps serving",
    )
}
