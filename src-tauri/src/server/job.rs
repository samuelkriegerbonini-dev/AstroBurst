// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub type JobId = String;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running = 0,
    Done = 1,
    Error = 2,
    Cancelled = 3,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    Progress { pct: u32, stage: String },
    Complete,
    Error { message: String },
}

pub struct Job {
    pub id: JobId,
    pub action: String,
    pub pct: AtomicU32,
    pub status: AtomicU8,
    pub cancel: CancellationToken,
    pub rx: Mutex<Option<mpsc::Receiver<SseEvent>>>,
    pub started_at: u64,
    pub completed_at: Mutex<Option<u64>>,
}

impl Job {
    fn new(id: JobId, action: impl Into<String>, rx: mpsc::Receiver<SseEvent>) -> Arc<Self> {
        Arc::new(Self {
            id,
            action: action.into(),
            pct: AtomicU32::new(0),
            status: AtomicU8::new(JobStatus::Running as u8),
            cancel: CancellationToken::new(),
            rx: Mutex::new(Some(rx)),
            started_at: now_ms(),
            completed_at: Mutex::new(None),
        })
    }

    pub fn is_running(&self) -> bool {
        self.status.load(Ordering::Acquire) == JobStatus::Running as u8
    }

    pub fn current_status(&self) -> JobStatus {
        match self.status.load(Ordering::Acquire) {
            0 => JobStatus::Running,
            1 => JobStatus::Done,
            2 => JobStatus::Error,
            _ => JobStatus::Cancelled,
        }
    }

    pub fn set_pct(&self, pct: u32) {
        self.pct.store(pct, Ordering::Release);
    }

    pub fn set_done(&self) {
        self.status.store(JobStatus::Done as u8, Ordering::Release);
        self.pct.store(100, Ordering::Release);
        *self.completed_at.lock().unwrap() = Some(now_ms());
    }

    pub fn set_error(&self) {
        self.status.store(JobStatus::Error as u8, Ordering::Release);
        *self.completed_at.lock().unwrap() = Some(now_ms());
    }

    pub fn set_cancelled(&self) {
        self.status.store(JobStatus::Cancelled as u8, Ordering::Release);
        *self.completed_at.lock().unwrap() = Some(now_ms());
    }
}

pub fn new_job(action: &str) -> (Arc<Job>, mpsc::Sender<SseEvent>) {
    let (tx, rx) = mpsc::channel(256);
    let id = Uuid::new_v4().to_string();
    (Job::new(id, action, rx), tx)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
