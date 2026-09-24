// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use tokio::sync::{mpsc, OwnedSemaphorePermit};
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

const STORING: u8 = 4;
const RUNNING: u8 = JobStatus::Running as u8;

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    Progress { pct: u32, stage: String },
    Complete,
    Error { message: String },
    Cancelled,
}

pub struct Job {
    pub id: JobId,
    pub action: String,
    pub pct: AtomicU32,
    status: AtomicU8,
    pub cancel: CancellationToken,
    pub rx: Mutex<Option<mpsc::Receiver<SseEvent>>>,
    pub started_at: u64,
    pub completed_at: Mutex<Option<u64>>,
    tx: Mutex<Option<mpsc::Sender<SseEvent>>>,
    worker_active: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Job {
    fn new(id: JobId, action: impl Into<String>, tx: mpsc::Sender<SseEvent>, rx: mpsc::Receiver<SseEvent>) -> Arc<Self> {
        Arc::new(Self {
            id,
            action: action.into(),
            pct: AtomicU32::new(0),
            status: AtomicU8::new(RUNNING),
            cancel: CancellationToken::new(),
            rx: Mutex::new(Some(rx)),
            started_at: now_ms(),
            completed_at: Mutex::new(None),
            tx: Mutex::new(Some(tx)),
            worker_active: AtomicBool::new(false),
        })
    }

    pub fn is_running(&self) -> bool {
        matches!(self.status.load(Ordering::Acquire), RUNNING | STORING)
    }

    pub fn is_active(&self) -> bool {
        self.is_running() || self.worker_active.load(Ordering::Acquire)
    }

    pub fn current_status(&self) -> JobStatus {
        match self.status.load(Ordering::Acquire) {
            RUNNING | STORING => JobStatus::Running,
            1 => JobStatus::Done,
            2 => JobStatus::Error,
            _ => JobStatus::Cancelled,
        }
    }

    pub fn completed_at(&self) -> Option<u64> {
        *lock(&self.completed_at)
    }

    pub fn progress(&self, pct: u32, stage: &str) {
        if self.status.load(Ordering::Acquire) != RUNNING {
            return;
        }
        let pct = pct.min(99);
        self.pct.fetch_max(pct, Ordering::AcqRel);
        if let Some(tx) = lock(&self.tx).as_ref() {
            if tx.capacity() > 1 {
                let _ = tx.try_send(SseEvent::Progress { pct, stage: stage.to_string() });
            }
        }
    }

    pub fn begin_commit(&self) -> bool {
        self.status
            .compare_exchange(RUNNING, STORING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn finish(&self, from: &[u8], next: JobStatus, event: SseEvent) -> bool {
        let won = from.iter().any(|&current| {
            self.status
                .compare_exchange(current, next as u8, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        });
        if !won {
            return false;
        }
        if next == JobStatus::Done {
            self.pct.store(100, Ordering::Release);
        }
        if next == JobStatus::Cancelled {
            self.cancel.cancel();
        }
        *lock(&self.completed_at) = Some(now_ms());
        if let Some(tx) = lock(&self.tx).take() {
            let _ = tx.try_send(event);
        }
        true
    }

    pub fn set_done(&self) -> bool {
        self.finish(&[STORING, RUNNING], JobStatus::Done, SseEvent::Complete)
    }

    pub fn set_error(&self, message: impl Into<String>) -> bool {
        self.finish(&[RUNNING, STORING], JobStatus::Error, SseEvent::Error { message: message.into() })
    }

    pub fn set_cancelled(&self) -> bool {
        self.finish(&[RUNNING], JobStatus::Cancelled, SseEvent::Cancelled)
    }
}

pub fn new_job(action: &str) -> Arc<Job> {
    let (tx, rx) = mpsc::channel(256);
    Job::new(Uuid::new_v4().to_string(), action, tx, rx)
}

fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

pub fn run_job<F: FnOnce(&Job)>(job: &Job, body: F) {
    if job.cancel.is_cancelled() {
        return;
    }
    match std::panic::catch_unwind(AssertUnwindSafe(|| body(job))) {
        Ok(()) => {
            if job.is_running() {
                job.set_error("job worker ended without a result");
            }
        }
        Err(panic) => {
            job.set_error(format!("job worker panicked: {}", panic_message(panic.as_ref())));
        }
    }
}

struct Worker {
    job: Arc<Job>,
    _permit: OwnedSemaphorePermit,
}

impl Worker {
    fn start(job: Arc<Job>, permit: OwnedSemaphorePermit) -> Self {
        job.worker_active.store(true, Ordering::Release);
        Self { job, _permit: permit }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.job.worker_active.store(false, Ordering::Release);
    }
}

pub fn spawn_job<F>(job: Arc<Job>, permit: OwnedSemaphorePermit, body: F)
where
    F: FnOnce(&Job) + Send + 'static,
{
    let worker = Worker::start(job, permit);
    tokio::task::spawn_blocking(move || run_job(&worker.job, body));
}

#[cfg(test)]
pub(crate) async fn spawn_waiting_worker(job: &Arc<Job>, permit: OwnedSemaphorePermit) -> std::sync::mpsc::Sender<()> {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    spawn_job(Arc::clone(job), permit, move |_| {
        let _ = started_tx.send(());
        let _ = release_rx.recv();
    });
    let _ = started_rx.await;
    release_tx
}

#[cfg(test)]
pub(crate) fn cancel_after_stage_started(job: &Job, stage_pct: u32) -> impl Fn() -> bool + Sync + '_ {
    let polls_in_stage = std::sync::atomic::AtomicUsize::new(0);
    move || job.pct.load(Ordering::Acquire) >= stage_pct && polls_in_stage.fetch_add(1, Ordering::SeqCst) >= 1
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc::error::TryRecvError;
    use tokio::sync::Semaphore;

    use super::*;

    fn events(job: &Job) -> mpsc::Receiver<SseEvent> {
        lock(&job.rx).take().expect("receiver present")
    }

    #[test]
    fn late_worker_cannot_overwrite_cancelled() {
        let job = new_job("stack");
        assert!(job.set_cancelled());
        assert!(!job.set_done());
        assert_eq!(job.current_status(), JobStatus::Cancelled);
        assert!(!job.set_error("late"));
        assert_eq!(job.current_status(), JobStatus::Cancelled);
    }

    #[test]
    fn running_job_finishes_once() {
        let job = new_job("stack");
        assert!(job.is_running());
        assert!(job.set_done());
        assert_eq!(job.current_status(), JobStatus::Done);
        assert_eq!(job.pct.load(Ordering::Acquire), 100);
        assert!(!job.set_error("late"));
        assert_eq!(job.current_status(), JobStatus::Done);
    }

    #[test]
    fn cancel_after_completion_does_not_relabel_a_finished_job() {
        let job = new_job("stack");
        let mut rx = events(&job);
        assert!(job.set_done());
        let done_at = job.completed_at();
        assert!(!job.set_cancelled());
        assert_eq!(job.current_status(), JobStatus::Done);
        assert_eq!(job.completed_at(), done_at);
        assert!(!job.cancel.is_cancelled());
        assert!(matches!(rx.try_recv(), Ok(SseEvent::Complete)));
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn cancel_that_arrives_while_the_result_is_stored_is_refused() {
        let job = new_job("stack");
        let mut rx = events(&job);
        assert!(job.begin_commit());
        assert!(job.is_running());
        assert_eq!(job.current_status(), JobStatus::Running);
        assert!(!job.set_cancelled());
        assert!(job.set_done());
        assert_eq!(job.current_status(), JobStatus::Done);
        assert!(matches!(rx.try_recv(), Ok(SseEvent::Complete)));
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn cancelled_job_cannot_start_storing() {
        let job = new_job("stack");
        assert!(job.set_cancelled());
        assert!(!job.begin_commit());
    }

    async fn wait_for_permit(semaphore: &Semaphore) {
        let permit = tokio::time::timeout(Duration::from_secs(5), semaphore.acquire())
            .await
            .expect("the worker must give its permit back when it exits")
            .expect("semaphore open");
        drop(permit);
    }

    #[tokio::test]
    async fn cancel_closes_the_stream_at_once_but_the_permit_returns_only_when_the_worker_exits() {
        let semaphore = Arc::new(Semaphore::new(1));
        let job = new_job("stack");
        let mut rx = events(&job);
        let release = spawn_waiting_worker(&job, Arc::clone(&semaphore).try_acquire_owned().unwrap()).await;
        job.progress(40, "loading");

        assert!(job.set_cancelled());
        assert!(job.cancel.is_cancelled());
        assert!(matches!(rx.try_recv(), Ok(SseEvent::Progress { pct: 40, .. })));
        assert!(matches!(rx.try_recv(), Ok(SseEvent::Cancelled)));
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
        assert_eq!(semaphore.available_permits(), 0, "the worker still runs, so it keeps its permit");
        assert!(job.is_active());

        job.progress(80, "stacking");
        assert_eq!(job.pct.load(Ordering::Acquire), 40);

        drop(release);
        wait_for_permit(&semaphore).await;
        assert!(!job.is_active());
        assert_eq!(job.current_status(), JobStatus::Cancelled);
    }

    #[tokio::test]
    async fn panicking_worker_gives_its_permit_back() {
        let semaphore = Arc::new(Semaphore::new(1));
        let job = new_job("stack");
        spawn_job(Arc::clone(&job), Arc::clone(&semaphore).try_acquire_owned().unwrap(), |_| panic!("boom"));
        wait_for_permit(&semaphore).await;
        assert_eq!(job.current_status(), JobStatus::Error);
        assert!(!job.is_active());
    }

    #[test]
    fn cancelled_job_never_starts_its_body() {
        let job = new_job("stack");
        assert!(job.set_cancelled());
        let mut ran = false;
        run_job(&job, |_| ran = true);
        assert!(!ran);
        assert_eq!(job.current_status(), JobStatus::Cancelled);
    }

    #[test]
    fn full_channel_still_delivers_the_terminal_event() {
        let job = new_job("stack");
        let mut rx = events(&job);
        for i in 0..1000 {
            job.progress(i % 99, "loading");
        }
        assert!(job.set_error("boom"));
        let mut last = None;
        while let Ok(evt) = rx.try_recv() {
            last = Some(evt);
        }
        assert!(matches!(last, Some(SseEvent::Error { .. })));
    }

    #[test]
    fn panicking_worker_marks_the_job_failed_and_reports_it() {
        let job = new_job("stack");
        let mut rx = events(&job);
        run_job(&job, |_| panic!("attempt to add with overflow"));
        assert_eq!(job.current_status(), JobStatus::Error);
        match rx.try_recv() {
            Ok(SseEvent::Error { message }) => assert!(message.contains("overflow"), "{message}"),
            _ => panic!("expected an error event"),
        }
    }

    #[test]
    fn worker_that_returns_without_an_outcome_is_marked_failed() {
        let job = new_job("stack");
        run_job(&job, |_| {});
        assert_eq!(job.current_status(), JobStatus::Error);

        let cancelled = new_job("stack");
        cancelled.set_cancelled();
        run_job(&cancelled, |_| {});
        assert_eq!(cancelled.current_status(), JobStatus::Cancelled);
    }
}
