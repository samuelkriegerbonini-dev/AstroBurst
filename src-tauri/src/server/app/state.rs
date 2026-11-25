// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

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
    create_lock: Arc<Mutex<()>>,
}

impl AppState {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        let job_semaphore = Arc::new(Semaphore::new(config.jobs_max));
        Self {
            sessions: Arc::new(DashMap::new()),
            job_semaphore,
            config,
            created_total: Arc::new(AtomicU64::new(0)),
            create_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn create_session(&self) -> Option<Arc<Session>> {
        let _only_one_create_at_a_time = self.create_lock.lock().unwrap_or_else(|e| e.into_inner());
        let session = SessionManager::new(
            Arc::clone(&self.sessions),
            Arc::clone(&self.config),
        )
        .create()?;
        self.created_total.fetch_add(1, Ordering::Relaxed);
        Some(session)
    }
}