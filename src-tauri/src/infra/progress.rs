use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Emitter;

const MIN_EMIT_INTERVAL_MS: u128 = 50;

#[derive(Clone)]
pub struct ProgressHandle {
    current: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    app_handle: tauri::AppHandle,
    event_name: String,
    last_emit: Arc<Mutex<Instant>>,
}

#[derive(Clone, Serialize)]
pub struct ProgressPayload {
    pub current: u64,
    pub total: u64,
    pub percent: u32,
    pub stage: String,
}

impl ProgressHandle {
    pub fn new(app: &tauri::AppHandle, event: &str, total: u64) -> Self {
        Self {
            current: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(total)),
            cancelled: Arc::new(AtomicBool::new(false)),
            app_handle: app.clone(),
            event_name: event.to_string(),
            last_emit: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub fn tick_with_stage(&self, stage: &str) {
        let cur = self.current.fetch_add(1, Ordering::Relaxed) + 1;
        let tot = self.total.load(Ordering::Relaxed);

        let is_last = cur >= tot;
        if !is_last {
            let mut last = self.last_emit.lock().unwrap();
            let now = Instant::now();
            if now.duration_since(*last).as_millis() < MIN_EMIT_INTERVAL_MS {
                return;
            }
            *last = now;
        }

        let percent = if tot > 0 {
            (cur as f64 / tot as f64 * 100.0) as u32
        } else {
            0
        };
        let _ = self.app_handle.emit(
            &self.event_name,
            ProgressPayload {
                current: cur,
                total: tot,
                percent,
                stage: stage.to_string(),
            },
        );
    }

    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn emit_complete(&self) {
        let tot = self.total.load(Ordering::Relaxed);
        let _ = self.app_handle.emit(
            &self.event_name,
            ProgressPayload {
                current: tot,
                total: tot,
                percent: 100,
                stage: "complete".to_string(),
            },
        );
    }
}
