use std::sync::Arc;

use super::config::ScienceConfig;
use super::outcome::OutcomeStore;
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
        Arc::new(Self::new(ScienceConfig::from_env(state.config.bind)))
    }
}
