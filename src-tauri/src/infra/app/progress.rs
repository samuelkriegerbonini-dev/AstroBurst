use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[cfg(feature = "tauri")]
use tauri::Emitter;

const MIN_EMIT_INTERVAL_MS: u128 = 50;

fn should_emit(last_stage: Option<&str>, stage: &str, elapsed_ms: u128, is_last: bool) -> bool {
    match last_stage {
        None => true,
        Some(last) => is_last || last != stage || elapsed_ms >= MIN_EMIT_INTERVAL_MS,
    }
}

#[derive(Clone, Serialize)]
pub struct ProgressPayload {
    pub current: u64,
    pub total: u64,
    pub percent: u32,
    pub stage: String,
}

// Headless build support (tauri-gated ProgressHandle + no-op stub below) — contributed by
// Jae-Joon Lee (https://github.com/leejjoon).
#[derive(Clone)]
pub struct ProgressHandle {
    current: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    cancel_source: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    sink: ProgressSink,
    last_emit: Arc<Mutex<(Instant, Option<String>)>>,
    part_index: u64,
    part_count: u64,
}

pub type ProgressSink = Arc<dyn Fn(&ProgressPayload) + Send + Sync>;

#[cfg(feature = "tauri")]
static CANCEL_FLAGS: std::sync::LazyLock<Mutex<std::collections::HashMap<String, Vec<std::sync::Weak<AtomicBool>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

#[cfg(feature = "tauri")]
pub fn cancel_event(event: &str) -> bool {
    let mut flags = CANCEL_FLAGS.lock().unwrap();
    match flags.get_mut(event) {
        Some(list) => {
            let mut any = false;
            list.retain(|w| match w.upgrade() {
                Some(flag) => {
                    flag.store(true, Ordering::Relaxed);
                    any = true;
                    true
                }
                None => false,
            });
            any
        }
        None => false,
    }
}

impl ProgressHandle {
    #[cfg(feature = "tauri")]
    pub fn new(app: &tauri::AppHandle, event: &str, total: u64) -> Self {
        let (app, event_name) = (app.clone(), event.to_string());
        let progress = Self::with_sink(
            Arc::new(move |payload: &ProgressPayload| {
                let _ = app.emit(&event_name, payload.clone());
            }),
            total,
        );
        {
            let mut flags = CANCEL_FLAGS.lock().unwrap();
            let list = flags.entry(event.to_string()).or_default();
            list.retain(|w| w.strong_count() > 0);
            list.push(Arc::downgrade(&progress.cancelled));
        }
        progress
    }

    pub fn with_sink(sink: ProgressSink, total: u64) -> Self {
        Self {
            current: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(total)),
            cancelled: Arc::new(AtomicBool::new(false)),
            cancel_source: None,
            sink,
            last_emit: Arc::new(Mutex::new((Instant::now(), None))),
            part_index: 0,
            part_count: 1,
        }
    }

    pub fn with_cancel_source(self, source: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self { cancel_source: Some(source), ..self }
    }

    pub fn part(&self, index: u64, count: u64) -> Self {
        Self {
            current: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(self.total.load(Ordering::Relaxed))),
            cancelled: Arc::clone(&self.cancelled),
            cancel_source: self.cancel_source.clone(),
            sink: self.sink.clone(),
            last_emit: Arc::clone(&self.last_emit),
            part_index: index,
            part_count: count.max(1),
        }
    }

    fn percent(&self, cur: u64, tot: u64) -> u32 {
        if tot == 0 {
            return 0;
        }
        let done = (self.part_index * tot + cur) as f64;
        (done / (self.part_count * tot) as f64 * 100.0) as u32
    }

    fn emit_payload(&self, cur: u64, tot: u64, stage: &str) {
        let percent = self.percent(cur, tot);
        (self.sink)(&ProgressPayload {
            current: cur,
            total: tot,
            percent,
            stage: stage.to_string(),
        });
    }

    fn claim_emit(&self, stage: &str, is_last: bool) -> bool {
        let mut last = self.last_emit.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(last.0).as_millis();
        if !should_emit(last.1.as_deref(), stage, elapsed, is_last) {
            return false;
        }
        *last = (now, Some(stage.to_string()));
        true
    }

    pub fn tick_with_stage(&self, stage: &str) {
        let cur = self.current.fetch_add(1, Ordering::Relaxed) + 1;
        let tot = self.total.load(Ordering::Relaxed);
        if self.claim_emit(stage, cur >= tot) {
            self.emit_payload(cur, tot, stage);
        }
    }

    pub fn emit_stage(&self, stage: &str) {
        let cur = self.current.load(Ordering::Relaxed);
        let tot = self.total.load(Ordering::Relaxed);
        *self.last_emit.lock().unwrap() = (Instant::now(), Some(stage.to_string()));
        self.emit_payload(cur, tot, stage);
    }

    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    pub fn report(&self, current: u64, total: u64, stage: &str) {
        self.current.store(current, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        if self.claim_emit(stage, current >= total) {
            self.emit_payload(current, total, stage);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || self.cancel_source.as_ref().is_some_and(|source| source())
    }

    pub fn emit_complete(&self) {
        if self.part_index + 1 < self.part_count {
            return;
        }
        let tot = self.total.load(Ordering::Relaxed);
        (self.sink)(&ProgressPayload {
            current: tot,
            total: tot,
            percent: 100,
            stage: "complete".to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Seen = Arc<Mutex<Vec<(u64, u64, u32, String)>>>;

    fn recorder() -> (ProgressSink, Seen) {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let sink: ProgressSink = {
            let seen = Arc::clone(&seen);
            Arc::new(move |p: &ProgressPayload| seen.lock().unwrap().push((p.current, p.total, p.percent, p.stage.clone())))
        };
        (sink, seen)
    }

    fn payload(current: u64, total: u64, percent: u32, stage: &str) -> (u64, u64, u32, String) {
        (current, total, percent, stage.to_string())
    }

    #[test]
    fn a_callback_sink_receives_the_first_tick_each_new_stage_the_last_tick_and_completion() {
        let (sink, seen) = recorder();
        let progress = ProgressHandle::with_sink(sink, 4);
        for stage in ["load_frame", "load_frame", "combine", "combine"] {
            progress.tick_with_stage(stage);
        }
        progress.emit_complete();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first(), Some(&payload(1, 4, 25, "load_frame")), "{seen:?}");
        assert!(seen.contains(&payload(3, 4, 75, "combine")), "{seen:?}");
        assert!(seen.contains(&payload(4, 4, 100, "combine")), "{seen:?}");
        assert_eq!(seen.last(), Some(&payload(4, 4, 100, "complete")), "{seen:?}");
    }

    #[test]
    fn parts_share_the_sink_and_only_the_last_part_reports_completion() {
        let (sink, seen) = recorder();
        let progress = ProgressHandle::with_sink(sink, 2);
        let first = progress.part(0, 2);
        first.report(2, 2, "ha");
        first.emit_complete();
        let second = progress.part(1, 2);
        second.report(1, 2, "oiii");
        second.emit_complete();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![payload(2, 2, 50, "ha"), payload(1, 2, 75, "oiii"), payload(2, 2, 100, "complete")]
        );
    }

    #[test]
    fn the_cancel_source_is_read_on_every_poll_by_the_handle_its_clones_and_its_parts() {
        let flag = Arc::new(AtomicBool::new(false));
        let source = {
            let flag = Arc::clone(&flag);
            Arc::new(move || flag.load(Ordering::SeqCst))
        };
        let (sink, _) = recorder();
        let progress = ProgressHandle::with_sink(sink, 2).with_cancel_source(source);
        let clone = progress.clone();
        let part = progress.part(1, 2);
        assert!(!progress.is_cancelled() && !clone.is_cancelled() && !part.is_cancelled());
        flag.store(true, Ordering::SeqCst);
        assert!(progress.is_cancelled() && clone.is_cancelled() && part.is_cancelled());
    }

    #[test]
    fn stage_changes_are_never_throttled() {
        assert!(should_emit(None, "load_frame", 0, false), "the first tick must be emitted");
        assert!(should_emit(Some("load_frame"), "align", 1, false), "a new stage within 50 ms must be emitted");
        assert!(!should_emit(Some("align"), "align", 49, false), "the same stage within 50 ms is throttled");
        assert!(should_emit(Some("align"), "align", 50, false), "the same stage after 50 ms is emitted");
        assert!(should_emit(Some("align"), "align", 0, true), "the last tick is always emitted");
    }
}
