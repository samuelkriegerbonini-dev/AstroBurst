// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use tokio::sync::Semaphore;

use super::config::ServerConfig;
use super::session::{Session, SessionId, SessionManager};

#[derive(Clone)]
pub struct AppState {
    pub sessions: Arc<DashMap<SessionId, Arc<Session>>>,
    pub job_semaphore: Arc<Semaphore>,
    pub config: Arc<ServerConfig>,
    pub created_total: Arc<AtomicU64>,
}

impl AppState {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        let job_semaphore = Arc::new(Semaphore::new(config.jobs_max));
        Self {
            sessions: Arc::new(DashMap::new()),
            job_semaphore,
            config,
            created_total: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn create_session(&self) -> Option<Arc<Session>> {
        let session = SessionManager::new(
            Arc::clone(&self.sessions),
            Arc::clone(&self.config),
        )
        .create()?;
        self.created_total.fetch_add(1, Ordering::Relaxed);
        Some(session)
    }
}