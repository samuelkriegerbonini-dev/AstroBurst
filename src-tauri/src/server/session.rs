// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use tokio::sync::RwLock;
use uuid::Uuid;

use astroburst_lib::infra::cache::ImageCache;

use super::config::ServerConfig;
use super::job::{Job, JobId};

pub type SessionId = String;

pub struct Session {
    pub id: SessionId,
    pub cache: Arc<ImageCache>,
    pub jobs: DashMap<JobId, Arc<Job>>,
    last_accessed: RwLock<Instant>,
}

impl Session {
    pub fn new(id: SessionId, cfg: &ServerConfig) -> Arc<Self> {
        Arc::new(Self {
            id,
            cache: Arc::new(ImageCache::new(cfg.cache_max_entries, cfg.cache_max_bytes)),
            jobs: DashMap::new(),
            last_accessed: RwLock::new(Instant::now()),
        })
    }

    pub async fn touch(&self) {
        *self.last_accessed.write().await = Instant::now();
    }

    pub fn has_active_jobs(&self) -> bool {
        self.jobs.iter().any(|e| e.value().is_running())
    }
}

pub struct SessionManager {
    sessions: Arc<DashMap<SessionId, Arc<Session>>>,
    config: Arc<ServerConfig>,
}

impl SessionManager {
    pub fn new(
        sessions: Arc<DashMap<SessionId, Arc<Session>>>,
        config: Arc<ServerConfig>,
    ) -> Self {
        Self { sessions, config }
    }

    pub fn create(&self) -> Option<Arc<Session>> {
        if self.sessions.len() >= self.config.session_max {
            return None;
        }
        let id = Uuid::new_v4().to_string();
        let session = Session::new(id.clone(), &self.config);
        self.sessions.insert(id, Arc::clone(&session));
        Some(session)
    }

    pub fn start_ttl_cleaner(
        sessions: Arc<DashMap<SessionId, Arc<Session>>>,
        config: Arc<ServerConfig>,
    ) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.cleanup_interval);
            loop {
                interval.tick().await;
                let now = Instant::now();
                let mut expired: Vec<SessionId> = Vec::new();
                for entry in sessions.iter() {
                    let s = entry.value();
                    if s.has_active_jobs() {
                        continue;
                    }
                    let last = *s.last_accessed.read().await;
                    if now.duration_since(last) > config.session_ttl {
                        expired.push(entry.key().clone());
                    }
                }
                for id in &expired {
                    sessions.remove(id);
                    log::info!("session {} evicted (idle TTL)", id);
                }
            }
        });
    }
}
