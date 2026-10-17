use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use reqwest::{Client, Url};
use tokio::sync::Semaphore;
use tokio::task::AbortHandle;

use super::cache::dir_info;
use super::download::ProgressThrottle;
use super::transfer::{self, Storage, Target, MAX_PRODUCT_BYTES, QUOTA_FOLDERS};
use crate::core::mast::errors::{bad_input, internal};
use crate::core::mast::ids::validate_request_id;
use crate::core::mast::types::{
    MastCmdError, MastDownloadAction, MastDownloadInput, MastDownloadItem, MastDownloadState,
    MastErrorCode, MastErrorContext, MastMissionId,
};
use crate::infra::net::download::{
    download, DownloadError, DownloadObserver, DownloadOutcome, DownloadRequest,
};

pub const DOWNLOAD_PERMITS: usize = 2;
pub const DOWNLOAD_EVENT: &str = "mast:download";
pub const MAX_ITEMS_PER_REQUEST: usize = 500;
pub const KEPT_FINISHED: usize = 200;
const SIZE_UNITS: [(f64, &str); 4] = [(1e12, "TB"), (1e9, "GB"), (1e6, "MB"), (1e3, "KB")];
const TWO_DECIMALS_FROM: f64 = 1e9;
const ONE_DECIMAL_BELOW: f64 = 10.0;
const PART_SUFFIX: &str = ".part";
const RECORD_SUFFIX: &str = ".done.json";

pub type Emit = Arc<dyn Fn(&MastDownloadItem) + Send + Sync>;
pub type StorageSource = Arc<dyn Fn() -> Result<Storage, MastCmdError> + Send + Sync>;
pub type FreeSpace = fn(&Path) -> io::Result<u64>;

pub struct QueueConfig {
    pub client: reqwest::Result<Client>,
    pub base_url: String,
    pub storage: StorageSource,
    pub free_space: FreeSpace,
    pub emit: Emit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueRequest {
    pub request_id: String,
    pub mission: MastMissionId,
    pub items: Vec<MastDownloadInput>,
    pub open: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Space {
    pub used: u64,
    pub free: Option<u64>,
}

struct Entry {
    item: MastDownloadItem,
    url: Url,
    dest: PathBuf,
    replace: bool,
    on_disk: u64,
    cancel: Arc<AtomicBool>,
    task: Option<AbortHandle>,
}

struct Inner {
    client: Result<Client, String>,
    base_url: String,
    storage: StorageSource,
    free_space: FreeSpace,
    emit: Emit,
    permits: Arc<Semaphore>,
    entries: Mutex<Vec<Entry>>,
}

#[derive(Clone)]
pub struct DownloadQueue {
    inner: Arc<Inner>,
}

struct Planned {
    index: usize,
    input: MastDownloadInput,
    target: Target,
    url: Url,
}

struct Placed {
    planned: Planned,
    dest: PathBuf,
    on_disk: u64,
    incoming: Option<u64>,
}

struct Survey {
    placed: Vec<Placed>,
    space: Space,
    quota: u64,
}

struct Accepted {
    snapshot: Vec<MastDownloadItem>,
    started: Vec<String>,
    merged: Vec<MastDownloadItem>,
}

struct Job {
    request: DownloadRequest,
    cancel: Arc<AtomicBool>,
    replace: bool,
    mission: MastMissionId,
}

fn size_text(bytes: u64) -> String {
    let value = bytes as f64;
    for (size, unit) in SIZE_UNITS {
        if value < size {
            continue;
        }
        let scaled = value / size;
        return if size >= TWO_DECIMALS_FROM {
            format!("{scaled:.2} {unit}")
        } else if scaled < ONE_DECIMAL_BELOW {
            format!("{scaled:.1} {unit}")
        } else {
            format!("{} {unit}", scaled.round())
        };
    }
    format!("{bytes} B")
}

fn shared_place(first: &str, second: &str) -> MastCmdError {
    bad_input(
        "uri",
        &format!("{first} and {second} would be saved as the same file"),
    )
}

pub fn admit(
    space: Space,
    pending: u64,
    incoming: Option<u64>,
    quota: u64,
) -> Result<(), MastCmdError> {
    let Some(incoming) = incoming else {
        return Ok(());
    };
    let total = space
        .used
        .checked_add(pending)
        .and_then(|sum| sum.checked_add(incoming));
    match total {
        Some(total) if total <= quota => {}
        _ => {
            let total = total.unwrap_or(u64::MAX);
            return Err(MastCmdError::new(
                MastErrorCode::Quota,
                format!(
                    "the MAST download quota ({}) would be exceeded: {} downloaded or queued, {} more requested",
                    size_text(quota),
                    size_text(space.used.saturating_add(pending)),
                    size_text(incoming)
                ),
            )
            .with_context(MastErrorContext {
                bytes: Some(total),
                ..Default::default()
            }));
        }
    }
    let needed = pending.saturating_add(incoming);
    match space.free {
        Some(free) if free < needed => Err(MastCmdError::new(
            MastErrorCode::Quota,
            format!(
                "not enough free disk space: {} free, {} needed",
                size_text(free),
                size_text(needed)
            ),
        )
        .with_context(MastErrorContext {
            bytes: Some(needed),
            ..Default::default()
        })),
        _ => Ok(()),
    }
}

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

#[cfg(any(windows, target_os = "macos"))]
fn place_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn place_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn same_place(a: &Path, b: &Path) -> bool {
    place_key(a) == place_key(b)
}

fn pending_bytes(entry: &Entry) -> u64 {
    let written = match entry.item.received {
        0 => entry.on_disk,
        received => received,
    };
    entry.item.size_bytes.saturating_sub(written)
}

fn prune(entries: &mut Vec<Entry>) {
    let finished = entries.iter().filter(|e| !e.item.state.is_active()).count();
    let mut excess = finished.saturating_sub(KEPT_FINISHED);
    entries.retain(|entry| {
        if excess > 0 && !entry.item.state.is_active() {
            excess -= 1;
            false
        } else {
            true
        }
    });
}

struct Watch<'a> {
    inner: &'a Inner,
    id: &'a str,
    cancel: Arc<AtomicBool>,
    throttle: Mutex<ProgressThrottle>,
}

impl DownloadObserver for Watch<'_> {
    fn progress(&self, received: u64, total: Option<u64>) {
        let due = guard(&self.throttle)
            .download(received, total, Instant::now())
            .is_some();
        self.inner.progress(self.id, received, total, due);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

impl Inner {
    fn emit_all(&self, items: &[MastDownloadItem]) {
        for item in items {
            (self.emit)(item);
        }
    }

    fn survey(&self, planned: Vec<Planned>) -> Result<Survey, MastCmdError> {
        let storage = (self.storage)()?;
        let mut seen: HashMap<String, String> = HashMap::new();
        let mut placed = Vec::with_capacity(planned.len());
        for planned in planned {
            let dest = transfer::destination(&storage.root, &planned.target)?;
            let key = place_key(&dest);
            if let Some(first) = seen.get(&key) {
                if *first != planned.target.uri {
                    return Err(shared_place(first, &planned.target.uri));
                }
                continue;
            }
            seen.insert(key, planned.target.uri.clone());
            let part = file_len(&beside(&dest, PART_SUFFIX));
            let size = planned.input.size_bytes;
            let (on_disk, incoming) = if planned.input.replace || !dest.exists() {
                (part, Some(size.saturating_sub(part)))
            } else {
                (size, None)
            };
            placed.push(Placed {
                planned,
                dest,
                on_disk,
                incoming,
            });
        }
        let used = QUOTA_FOLDERS
            .iter()
            .map(|folder| dir_info(&storage.root.join(folder)).bytes)
            .sum();
        let free = match (self.free_space)(&storage.root) {
            Ok(free) => Some(free),
            Err(error) => {
                log::warn!("free disk space for MAST downloads is unknown: {error}");
                None
            }
        };
        Ok(Survey {
            placed,
            space: Space { used, free },
            quota: storage.quota_bytes,
        })
    }

    fn accept(&self, request: &EnqueueRequest, survey: Survey) -> Result<Accepted, MastCmdError> {
        let mut entries = guard(&self.entries);
        let mut joined = Vec::new();
        let mut fresh = Vec::new();
        for placed in survey.placed {
            let running = entries
                .iter()
                .position(|e| e.item.state.is_active() && same_place(&e.dest, &placed.dest));
            match running {
                Some(at) if entries[at].item.uri != placed.planned.target.uri => {
                    return Err(shared_place(
                        &entries[at].item.uri,
                        &placed.planned.target.uri,
                    ));
                }
                Some(at) => joined.push(at),
                None => fresh.push(placed),
            }
        }
        let ids: Vec<String> = fresh
            .iter()
            .map(|placed| format!("{}:{}", request.request_id, placed.planned.index))
            .collect();
        for id in &ids {
            validate_request_id(id)?;
            if entries
                .iter()
                .any(|e| e.item.state.is_active() && &e.item.id == id)
            {
                return Err(bad_input(
                    "requestId",
                    "is already used by a running download",
                ));
            }
        }
        let pending = entries
            .iter()
            .filter(|e| e.item.state.is_active())
            .map(pending_bytes)
            .fold(0, u64::saturating_add);
        let incoming = fresh
            .iter()
            .filter_map(|placed| placed.incoming)
            .reduce(u64::saturating_add);
        admit(survey.space, pending, incoming, survey.quota)?;
        for at in &joined {
            entries[*at].cancel.store(false, Ordering::SeqCst);
        }
        let mut merged = Vec::new();
        if request.open {
            for at in joined {
                let item = &mut entries[at].item;
                if !item.open {
                    item.open = true;
                    merged.push(item.clone());
                }
            }
        }
        entries.retain(|e| {
            e.item.state.is_active()
                || !(ids.contains(&e.item.id) || fresh.iter().any(|p| same_place(&e.dest, &p.dest)))
        });
        let mut started = Vec::with_capacity(fresh.len());
        for (placed, id) in fresh.into_iter().zip(ids) {
            let Placed {
                planned,
                dest,
                on_disk,
                ..
            } = placed;
            let item = MastDownloadItem {
                id: id.clone(),
                request_id: request.request_id.clone(),
                mission: request.mission,
                dataset_id: planned.input.dataset_id.clone(),
                filename: planned.target.filename.clone(),
                program: planned.target.program.clone(),
                uri: planned.target.uri.clone(),
                size_bytes: planned.input.size_bytes,
                opens_as: planned.input.opens_as,
                open: request.open,
                replace: planned.input.replace,
                state: MastDownloadState::Queued,
                received: 0,
                total: None,
                path: dest.to_string_lossy().into_owned(),
                resumed_from: 0,
                reused: false,
                verified: None,
                action: None,
                error: None,
            };
            entries.push(Entry {
                item,
                url: planned.url,
                dest,
                replace: planned.input.replace,
                on_disk,
                cancel: Arc::new(AtomicBool::new(false)),
                task: None,
            });
            started.push(id);
        }
        prune(&mut entries);
        Ok(Accepted {
            snapshot: entries.iter().map(|e| e.item.clone()).collect(),
            started,
            merged,
        })
    }

    fn attach(&self, id: &str, task: AbortHandle) {
        if let Some(entry) = guard(&self.entries).iter_mut().find(|e| e.item.id == id) {
            entry.task = Some(task);
        }
    }

    fn begin(&self, id: &str) -> Option<Job> {
        let (job, item) = {
            let mut entries = guard(&self.entries);
            let entry = entries.iter_mut().find(|e| e.item.id == id)?;
            if entry.item.state != MastDownloadState::Queued || entry.cancel.load(Ordering::SeqCst)
            {
                return None;
            }
            entry.item.state = MastDownloadState::Downloading;
            let job = Job {
                request: DownloadRequest {
                    url: entry.url.clone(),
                    dest: entry.dest.clone(),
                    expected_size: Some(entry.item.size_bytes).filter(|size| *size > 0),
                    expected_sha256: None,
                    expected_etag: None,
                    headers: Vec::new(),
                    max_bytes: MAX_PRODUCT_BYTES,
                    overwrite: entry.replace,
                },
                cancel: Arc::clone(&entry.cancel),
                replace: entry.replace,
                mission: entry.item.mission,
            };
            (job, entry.item.clone())
        };
        (self.emit)(&item);
        Some(job)
    }

    fn progress(&self, id: &str, received: u64, total: Option<u64>, emit: bool) {
        let item = {
            let mut entries = guard(&self.entries);
            let Some(entry) = entries.iter_mut().find(|e| e.item.id == id) else {
                return;
            };
            if entry.item.state != MastDownloadState::Downloading {
                return;
            }
            entry.item.received = received;
            entry.item.total = total.or(entry.item.total);
            emit.then(|| entry.item.clone())
        };
        if let Some(item) = item {
            (self.emit)(&item);
        }
    }

    fn finish(
        &self,
        id: &str,
        mission: MastMissionId,
        result: Result<DownloadOutcome, DownloadError>,
        reused: bool,
    ) -> bool {
        let item = {
            let mut entries = guard(&self.entries);
            let Some(entry) = entries.iter_mut().find(|e| e.item.id == id) else {
                return true;
            };
            if matches!(result, Err(DownloadError::Cancelled))
                && !entry.cancel.load(Ordering::SeqCst)
            {
                return false;
            }
            entry.task = None;
            let item = &mut entry.item;
            match result {
                Ok(outcome) => {
                    item.state = MastDownloadState::Done;
                    item.received = outcome.bytes;
                    item.total = Some(outcome.bytes);
                    item.path = outcome.path.to_string_lossy().into_owned();
                    item.resumed_from = outcome.resumed_from;
                    item.reused = reused;
                    item.verified = Some(!reused || outcome.revalidated);
                    item.action = None;
                    item.error = None;
                }
                Err(DownloadError::Cancelled) => {
                    item.state = MastDownloadState::Cancelled;
                    item.action = Some(MastDownloadAction::Resume);
                    item.error = None;
                }
                Err(error) => {
                    let (error, action) = transfer::failure(error, mission);
                    item.state = MastDownloadState::Error;
                    item.action = action;
                    item.error = Some(error);
                }
            }
            let item = item.clone();
            prune(&mut entries);
            item
        };
        (self.emit)(&item);
        true
    }
}

async fn run(inner: Arc<Inner>, id: String, client: Client) {
    let Ok(_permit) = Arc::clone(&inner.permits).acquire_owned().await else {
        return;
    };
    let Some(job) = inner.begin(&id) else {
        return;
    };
    let dest = job.request.dest.clone();
    let record = beside(&dest, RECORD_SUFFIX);
    let existed = tokio::fs::metadata(&dest)
        .await
        .is_ok_and(|meta| meta.is_file());
    let kept_record = match job.replace {
        true => tokio::fs::read(&record).await.ok(),
        false => None,
    };
    let observer = Watch {
        inner: &inner,
        id: &id,
        cancel: Arc::clone(&job.cancel),
        throttle: Mutex::new(ProgressThrottle::new(&id)),
    };
    loop {
        if job.replace {
            if let Err(error) = tokio::fs::remove_file(&record).await {
                if error.kind() != io::ErrorKind::NotFound {
                    inner.finish(&id, job.mission, Err(DownloadError::Io(error)), false);
                    return;
                }
            }
        }
        let result = download(&client, &job.request, &observer).await;
        if let (Err(_), Some(bytes)) = (&result, &kept_record) {
            restore_record(&dest, &record, bytes).await;
        }
        if inner.finish(&id, job.mission, result, existed && !job.replace) {
            return;
        }
    }
}

async fn restore_record(dest: &Path, record: &Path, bytes: &[u8]) {
    let record_gone = tokio::fs::metadata(record).await.is_err();
    let file_kept = tokio::fs::metadata(dest)
        .await
        .is_ok_and(|meta| meta.is_file());
    if !(record_gone && file_kept) {
        return;
    }
    if let Err(error) = tokio::fs::write(record, bytes).await {
        log::warn!(
            "the download record of {} could not be restored: {error}",
            dest.display()
        );
    }
}

impl DownloadQueue {
    pub fn new(config: QueueConfig) -> Self {
        let client = config.client.map_err(|error| {
            log::warn!("the MAST download client could not be created: {error}");
            error.to_string()
        });
        Self {
            inner: Arc::new(Inner {
                client,
                base_url: config.base_url,
                storage: config.storage,
                free_space: config.free_space,
                emit: config.emit,
                permits: Arc::new(Semaphore::new(DOWNLOAD_PERMITS)),
                entries: Mutex::new(Vec::new()),
            }),
        }
    }

    pub async fn enqueue(
        &self,
        request: EnqueueRequest,
    ) -> Result<Vec<MastDownloadItem>, MastCmdError> {
        validate_request_id(&request.request_id)?;
        if request.items.is_empty() || request.items.len() > MAX_ITEMS_PER_REQUEST {
            return Err(bad_input("items", "send 1 to 500 files"));
        }
        let client = self.inner.client.clone().map_err(|_| {
            MastCmdError::new(
                MastErrorCode::Network,
                "the download client could not be created",
            )
        })?;
        let planned = request
            .items
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let target = transfer::plan(request.mission, input)?;
                let url = transfer::download_url(&self.inner.base_url, &target.uri)?;
                Ok(Planned {
                    index,
                    input: input.clone(),
                    target,
                    url,
                })
            })
            .collect::<Result<Vec<_>, MastCmdError>>()?;
        let surveyor = Arc::clone(&self.inner);
        let survey = tokio::task::spawn_blocking(move || surveyor.survey(planned))
            .await
            .map_err(|_| internal())??;
        let Accepted {
            snapshot,
            started,
            mut merged,
        } = self.inner.accept(&request, survey)?;
        for id in started {
            let task = tokio::spawn(run(Arc::clone(&self.inner), id.clone(), client.clone()));
            self.inner.attach(&id, task.abort_handle());
            if let Some(item) = snapshot.iter().find(|item| item.id == id) {
                merged.push(item.clone());
            }
        }
        self.inner.emit_all(&merged);
        Ok(snapshot)
    }

    pub fn cancel(&self, request_id: &str) -> Result<usize, MastCmdError> {
        validate_request_id(request_id)?;
        let prefix = format!("{request_id}:");
        let mut count = 0;
        let mut stopped = Vec::new();
        {
            let mut entries = guard(&self.inner.entries);
            for entry in entries.iter_mut() {
                let hit = entry.item.id == request_id || entry.item.id.starts_with(&prefix);
                if !hit || !entry.item.state.is_active() {
                    continue;
                }
                count += 1;
                entry.cancel.store(true, Ordering::SeqCst);
                if entry.item.state == MastDownloadState::Queued {
                    if let Some(task) = entry.task.take() {
                        task.abort();
                    }
                    entry.item.state = MastDownloadState::Cancelled;
                    entry.item.action = Some(MastDownloadAction::Retry);
                    stopped.push(entry.item.clone());
                }
            }
        }
        self.inner.emit_all(&stopped);
        Ok(count)
    }

    pub fn snapshot(&self) -> Vec<MastDownloadItem> {
        guard(&self.inner.entries)
            .iter()
            .map(|entry| entry.item.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use crate::core::mast::types::{MastDownloadAction, MastDownloadState, MastErrorCode, OpensAs};
    use crate::infra::mast::transfer::HLSP_FOLDER;
    use crate::infra::net::http::{build_client, HttpProfile};
    use crate::infra::net::test_server::{serve, Resource, TestServer};

    const KIB: u64 = 1 << 10;
    const PATIENCE: Duration = Duration::from_secs(30);
    const CEERS: &str = "mast:HLSP/ceers/nircam1/ceers_nircam1_f200w_v0.5_i2d.fits";
    const CEERS_FILE: &str = "ceers_nircam1_f200w_v0.5_i2d.fits";

    fn body(len: u64, seed: u64) -> Vec<u8> {
        (0..len)
            .map(|i| ((i * 31 + i / 4099 + seed) % 251) as u8)
            .collect()
    }

    fn plenty(_: &Path) -> io::Result<u64> {
        Ok(u64::MAX / 4)
    }

    fn cramped(_: &Path) -> io::Result<u64> {
        Ok(1000)
    }

    fn unknown(_: &Path) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no free space here",
        ))
    }

    struct Rig {
        queue: DownloadQueue,
        server: TestServer,
        root: PathBuf,
        events: Arc<Mutex<Vec<MastDownloadItem>>>,
        _dir: tempfile::TempDir,
    }

    fn rig_with(resource: Resource, quota_bytes: u64, free_space: FreeSpace) -> Rig {
        let server = serve(resource);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data").join("mast-test");
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&events);
        let storage_root = root.clone();
        let queue = DownloadQueue::new(QueueConfig {
            client: build_client(&HttpProfile::transfer()),
            base_url: server.base.clone(),
            storage: Arc::new(move || {
                Ok(Storage {
                    root: storage_root.clone(),
                    quota_bytes,
                })
            }),
            free_space,
            emit: Arc::new(move |item: &MastDownloadItem| seen.lock().unwrap().push(item.clone())),
        });
        Rig {
            queue,
            server,
            root,
            events,
            _dir: dir,
        }
    }

    fn rig(resource: Resource) -> Rig {
        rig_with(resource, 20_000_000_000, plenty)
    }

    fn item(filename: &str, size: u64) -> MastDownloadInput {
        MastDownloadInput {
            dataset_id: "jw02739001001_02105_00001_nrca1".to_owned(),
            filename: filename.to_owned(),
            program: Some("2739".to_owned()),
            size_bytes: size,
            uri: None,
            opens_as: Some(OpensAs::Image),
            replace: false,
        }
    }

    fn hlsp(size: u64) -> MastDownloadInput {
        MastDownloadInput {
            dataset_id: "ceers_nircam1".to_owned(),
            filename: CEERS_FILE.to_owned(),
            program: None,
            size_bytes: size,
            uri: Some(CEERS.to_owned()),
            opens_as: Some(OpensAs::Image),
            replace: false,
        }
    }

    fn request(id: &str, items: Vec<MastDownloadInput>) -> EnqueueRequest {
        EnqueueRequest {
            request_id: id.to_owned(),
            mission: MastMissionId::Jwst,
            items,
            open: false,
        }
    }

    fn find(queue: &DownloadQueue, id: &str) -> Option<MastDownloadItem> {
        queue.snapshot().into_iter().find(|item| item.id == id)
    }

    async fn settle(queue: &DownloadQueue, id: &str) -> MastDownloadItem {
        let start = Instant::now();
        loop {
            if let Some(item) = find(queue, id).filter(|item| !item.state.is_active()) {
                return item;
            }
            assert!(
                start.elapsed() < PATIENCE,
                "{id} did not finish: {:?}",
                find(queue, id)
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn until(
        queue: &DownloadQueue,
        id: &str,
        test: impl Fn(&MastDownloadItem) -> bool,
    ) -> MastDownloadItem {
        let start = Instant::now();
        loop {
            if let Some(item) = find(queue, id).filter(|item| test(item)) {
                return item;
            }
            assert!(
                start.elapsed() < PATIENCE,
                "{id} never reached the state: {:?}",
                find(queue, id)
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn part_of(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(".part");
        PathBuf::from(name)
    }

    #[tokio::test]
    async fn two_downloads_run_at_once_and_the_third_waits() {
        let payload = body(256 * KIB, 1);
        let rig = rig(Resource::new(payload.clone()).slow_steady(512 * KIB));
        let names = ["a_cal.fits", "b_cal.fits", "c_cal.fits"];
        let items = names.iter().map(|name| item(name, 256 * KIB)).collect();
        let first = rig.queue.enqueue(request("dl1", items)).await.unwrap();
        assert_eq!(
            first.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["dl1:0", "dl1:1", "dl1:2"]
        );
        assert!(first
            .iter()
            .all(|i| i.state.is_active() && i.request_id == "dl1"));
        assert_eq!(
            first[0].path,
            rig.root
                .join("JWST")
                .join("2739")
                .join("a_cal.fits")
                .to_string_lossy()
        );
        assert_eq!(first[0].program.as_deref(), Some("2739"));
        assert_eq!(first[0].uri, "mast:JWST/product/a_cal.fits");

        let start = Instant::now();
        let (mut most, mut waited) = (0, false);
        loop {
            let items = rig.queue.snapshot();
            let downloading = items
                .iter()
                .filter(|i| i.state == MastDownloadState::Downloading)
                .count();
            let queued = items
                .iter()
                .filter(|i| i.state == MastDownloadState::Queued)
                .count();
            most = most.max(downloading);
            waited |= downloading == 2 && queued == 1;
            if items.iter().all(|i| i.state == MastDownloadState::Done) {
                break;
            }
            assert!(start.elapsed() < PATIENCE, "{items:?}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(most, 2);
        assert!(
            waited,
            "the third item never waited behind two running ones"
        );

        for (n, name) in names.iter().enumerate() {
            let done = find(&rig.queue, &format!("dl1:{n}")).unwrap();
            let path = rig.root.join("JWST").join("2739").join(name);
            assert_eq!(std::fs::read(&path).unwrap(), payload, "{name}");
            assert_eq!(done.path, path.to_string_lossy());
            assert_eq!(
                (done.received, done.total, done.verified, done.reused),
                (256 * KIB, Some(256 * KIB), Some(true), false)
            );
            assert_eq!((done.action, done.error.clone()), (None, None));
        }
        let requests = rig.server.requests();
        assert_eq!(requests.len(), 3);
        assert!(
            requests
                .iter()
                .any(|r| r.target
                    == "/api/v0.1/Download/file?uri=mast%3AJWST%2Fproduct%2Fa_cal.fits"),
            "{requests:?}"
        );
        let events = rig.events.lock().unwrap().clone();
        assert!(events
            .iter()
            .any(|e| e.id == "dl1:2" && e.state == MastDownloadState::Downloading));
        assert!(events
            .iter()
            .any(|e| e.state == MastDownloadState::Downloading
                && e.received > 0
                && e.received < 256 * KIB));
        for n in 0..3 {
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.id == format!("dl1:{n}") && e.state == MastDownloadState::Done)
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn quota_and_free_space_refuse_before_any_request() {
        let small = rig_with(Resource::new(body(4 * KIB, 2)), 100, plenty);
        let error = small
            .queue
            .enqueue(request("q1", vec![item("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap_err();
        assert_eq!(error.code, MastErrorCode::Quota);
        assert!(error.message.contains("quota"), "{}", error.message);
        assert!(small.server.requests().is_empty());
        assert!(small.queue.snapshot().is_empty());

        let full = rig_with(Resource::new(body(4 * KIB, 2)), 20_000_000_000, cramped);
        let error = full
            .queue
            .enqueue(request("q2", vec![item("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap_err();
        assert_eq!(error.code, MastErrorCode::Quota);
        assert!(error.message.contains("free"), "{}", error.message);
        assert!(full.server.requests().is_empty());

        let counted = rig_with(Resource::new(body(4 * KIB, 2)), 4 * KIB + 250, plenty);
        std::fs::create_dir_all(counted.root.join("JWST").join("1")).unwrap();
        std::fs::write(
            counted.root.join("JWST").join("1").join("old.fits"),
            vec![0u8; 200],
        )
        .unwrap();
        std::fs::create_dir_all(counted.root.join("lists")).unwrap();
        std::fs::write(
            counted.root.join("lists").join("not_counted.bin"),
            vec![0u8; 100_000],
        )
        .unwrap();
        std::fs::write(counted.root.join("not_counted.bin"), vec![0u8; 100_000]).unwrap();
        let fine = counted
            .queue
            .enqueue(request("q3", vec![item("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap();
        assert_eq!(fine.len(), 1);
        assert_eq!(
            settle(&counted.queue, "q3:0").await.state,
            MastDownloadState::Done
        );
        let again = counted
            .queue
            .enqueue(request("q4", vec![item("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap();
        assert_eq!(
            again.len(),
            1,
            "a file already on disk adds nothing to the quota"
        );
        assert_eq!(
            settle(&counted.queue, "q4:0").await.state,
            MastDownloadState::Done
        );

        let hlsp_counted = rig_with(Resource::new(body(4 * KIB, 2)), 4 * KIB + 250, plenty);
        std::fs::create_dir_all(hlsp_counted.root.join(HLSP_FOLDER).join("x")).unwrap();
        std::fs::write(
            hlsp_counted.root.join(HLSP_FOLDER).join("x").join("y.fits"),
            vec![0u8; 300],
        )
        .unwrap();
        let over = hlsp_counted
            .queue
            .enqueue(request("q5", vec![item("b_cal.fits", 4 * KIB)]))
            .await
            .unwrap_err();
        assert_eq!(over.code, MastErrorCode::Quota);
        assert!(hlsp_counted.server.requests().is_empty());

        let blind = rig_with(Resource::new(body(4 * KIB, 2)), 20_000_000_000, unknown);
        blind
            .queue
            .enqueue(request("q6", vec![item("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap();
        assert_eq!(
            settle(&blind.queue, "q6:0").await.state,
            MastDownloadState::Done
        );
    }

    #[test]
    fn admission_counts_disk_queue_and_incoming_bytes() {
        let roomy = Space {
            used: 600,
            free: Some(10_000),
        };
        assert!(admit(roomy, 200, Some(200), 1000).is_ok());
        let error = admit(roomy, 200, Some(201), 1000).unwrap_err();
        assert_eq!(error.code, MastErrorCode::Quota);
        assert_eq!(error.context.bytes, Some(1001));
        let tight = Space {
            used: 0,
            free: Some(399),
        };
        assert_eq!(
            admit(tight, 200, Some(200), 1000).unwrap_err().code,
            MastErrorCode::Quota
        );
        assert!(admit(
            Space {
                used: 0,
                free: Some(400)
            },
            200,
            Some(200),
            1000
        )
        .is_ok());
        assert!(admit(
            Space {
                used: 0,
                free: None
            },
            200,
            Some(200),
            1000
        )
        .is_ok());
        assert!(admit(
            Space {
                used: u64::MAX,
                free: None
            },
            u64::MAX,
            Some(1),
            u64::MAX
        )
        .is_err());
        let over = Space {
            used: 5000,
            free: Some(0),
        };
        assert!(admit(over, 0, None, 1000).is_ok());
        let unknown_size = admit(over, 0, Some(0), 1000).unwrap_err();
        assert_eq!(
            (unknown_size.code, unknown_size.context.bytes),
            (MastErrorCode::Quota, Some(5000))
        );
    }

    fn place_foreign(rig: &Rig, name: &str, len: usize) -> PathBuf {
        let path = rig.root.join("JWST").join("2739").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![1u8; len]).unwrap();
        path
    }

    fn replacing(filename: &str, size: u64) -> MastDownloadInput {
        MastDownloadInput {
            replace: true,
            ..item(filename, size)
        }
    }

    #[tokio::test]
    async fn a_replace_counts_its_whole_new_copy_and_an_unknown_size_still_meets_the_quota() {
        let quota = rig_with(Resource::new(body(4 * KIB, 13)), 4 * KIB + 250, plenty);
        place_foreign(&quota, "a_cal.fits", 4 * KIB as usize);
        let error = quota
            .queue
            .enqueue(request("r1", vec![replacing("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap_err();
        assert_eq!(error.code, MastErrorCode::Quota);
        assert!(error.message.contains("quota"), "{}", error.message);
        assert_eq!(error.context.bytes, Some(8 * KIB));
        let kept = quota
            .queue
            .enqueue(request("r2", vec![item("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap();
        assert_eq!(kept.len(), 1, "a file left in place adds nothing");
        assert_eq!(
            settle(&quota.queue, "r2:0").await.action,
            Some(MastDownloadAction::Replace)
        );

        let full = rig_with(Resource::new(body(4 * KIB, 13)), 20_000_000_000, cramped);
        place_foreign(&full, "a_cal.fits", 4 * KIB as usize);
        let error = full
            .queue
            .enqueue(request("r3", vec![replacing("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap_err();
        assert_eq!(error.code, MastErrorCode::Quota);
        assert!(error.message.contains("free"), "{}", error.message);

        let over = rig_with(Resource::new(body(4 * KIB, 13)), 1000, plenty);
        place_foreign(&over, "old.fits", 5000);
        let error = over
            .queue
            .enqueue(request("r4", vec![item("b_cal.fits", 0)]))
            .await
            .unwrap_err();
        assert_eq!(error.code, MastErrorCode::Quota);

        for rig in [&quota, &full, &over] {
            assert!(rig.server.requests().is_empty());
        }
    }

    #[tokio::test]
    async fn bytes_already_on_disk_for_a_queued_item_count_once() {
        let rig = rig_with(Resource::new(body(4 * KIB, 14)), 1100, plenty);
        let held = Arc::clone(&rig.queue.inner.permits)
            .acquire_many_owned(DOWNLOAD_PERMITS as u32)
            .await
            .unwrap();
        let folder = rig.root.join("JWST").join("2739");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("c_cal.fits.part"), vec![0u8; 900]).unwrap();
        rig.queue
            .enqueue(request("p1", vec![item("c_cal.fits", 1000)]))
            .await
            .unwrap();
        assert_eq!(
            find(&rig.queue, "p1:0").unwrap().state,
            MastDownloadState::Queued
        );
        let second = rig
            .queue
            .enqueue(request("p2", vec![item("d_cal.fits", 100)]))
            .await;
        assert!(second.is_ok(), "{second:?}");
        let error = rig
            .queue
            .enqueue(request("p3", vec![item("e_cal.fits", 1)]))
            .await
            .unwrap_err();
        assert_eq!(
            (error.code, error.context.bytes),
            (MastErrorCode::Quota, Some(1101))
        );
        assert_eq!(rig.queue.cancel("p1").unwrap(), 1);
        assert_eq!(rig.queue.cancel("p2").unwrap(), 1);
        drop(held);
        assert!(rig.server.requests().is_empty());
    }

    #[tokio::test]
    async fn a_failed_replace_stays_a_replace_when_it_resumes() {
        let payload = body(256 * KIB, 15);
        let rig = rig(Resource::new(payload.clone()).drop_after(100 * KIB));
        let path = place_foreign(&rig, "a_cal.fits", 19);
        rig.queue
            .enqueue(request("x1", vec![replacing("a_cal.fits", 256 * KIB)]))
            .await
            .unwrap();
        let dropped = settle(&rig.queue, "x1:0").await;
        assert_eq!(
            (dropped.state, dropped.action, dropped.replace),
            (
                MastDownloadState::Error,
                Some(MastDownloadAction::Resume),
                true
            )
        );
        assert_eq!(std::fs::metadata(part_of(&path)).unwrap().len(), 100 * KIB);

        rig.queue
            .enqueue(request("x2", vec![item("a_cal.fits", 256 * KIB)]))
            .await
            .unwrap();
        assert_eq!(
            settle(&rig.queue, "x2:0").await.action,
            Some(MastDownloadAction::Replace),
            "without replace the resume meets the old file"
        );

        let resume = MastDownloadInput {
            replace: dropped.replace,
            ..item("a_cal.fits", 256 * KIB)
        };
        rig.queue
            .enqueue(request("x3", vec![resume]))
            .await
            .unwrap();
        let done = settle(&rig.queue, "x3:0").await;
        assert_eq!(
            (done.state, done.resumed_from, done.replace),
            (MastDownloadState::Done, 100 * KIB, true)
        );
        assert_eq!(std::fs::read(&path).unwrap(), payload);
    }

    #[tokio::test]
    async fn a_duplicate_request_adds_open_to_the_waiting_download() {
        let rig = rig(Resource::new(body(4 * KIB, 16)));
        let held = Arc::clone(&rig.queue.inner.permits)
            .acquire_many_owned(DOWNLOAD_PERMITS as u32)
            .await
            .unwrap();
        rig.queue
            .enqueue(request("m1", vec![item("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap();
        let mut opened = request("m2", vec![item("a_cal.fits", 4 * KIB)]);
        opened.open = true;
        let snapshot = rig.queue.enqueue(opened).await.unwrap();
        let merged = snapshot.iter().find(|i| i.id == "m1:0").unwrap();
        assert_eq!(
            (merged.open, merged.state, snapshot.len()),
            (true, MastDownloadState::Queued, 1)
        );
        assert!(rig
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.id == "m1:0" && e.open));
        drop(held);
        let done = settle(&rig.queue, "m1:0").await;
        assert_eq!((done.state, done.open), (MastDownloadState::Done, true));
    }

    #[tokio::test]
    async fn bad_items_refuse_the_whole_request_before_any_io() {
        let rig = rig(Resource::new(body(4 * KIB, 3)));
        let mut crafted = item("b_cal.fits", 4 * KIB);
        crafted.program = Some("..\\..\\x".to_owned());
        let error = rig
            .queue
            .enqueue(request("b1", vec![item("a_cal.fits", 4 * KIB), crafted]))
            .await
            .unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput);
        assert!(error.message.starts_with("program: "), "{}", error.message);
        assert!(rig.queue.snapshot().is_empty());
        assert!(!rig.root.exists());

        let empty = rig
            .queue
            .enqueue(request("b2", Vec::new()))
            .await
            .unwrap_err();
        assert!(empty.message.starts_with("items: "), "{}", empty.message);
        let many = rig
            .queue
            .enqueue(request(
                "b3",
                vec![item("a_cal.fits", 1); MAX_ITEMS_PER_REQUEST + 1],
            ))
            .await
            .unwrap_err();
        assert!(many.message.starts_with("items: "), "{}", many.message);
        let bad_id = rig
            .queue
            .enqueue(request("b 4", vec![item("a_cal.fits", 1)]))
            .await
            .unwrap_err();
        assert!(
            bad_id.message.starts_with("requestId: "),
            "{}",
            bad_id.message
        );
        let mut roman = request("b5", vec![item("r_cal.asdf", 1)]);
        roman.mission = MastMissionId::Roman;
        assert_eq!(
            rig.queue.enqueue(roman).await.unwrap_err().code,
            MastErrorCode::UriRefused
        );
        assert!(rig.server.requests().is_empty());
        assert!(!rig.root.exists());

        let refused_root = DownloadQueue::new(QueueConfig {
            client: build_client(&HttpProfile::transfer()),
            base_url: rig.server.base.clone(),
            storage: Arc::new(|| {
                Err(MastCmdError::new(
                    MastErrorCode::Cache,
                    "the Data folder does not exist",
                ))
            }),
            free_space: plenty,
            emit: Arc::new(|_: &MastDownloadItem| {}),
        });
        let error = refused_root
            .enqueue(request("b6", vec![item("a_cal.fits", 1)]))
            .await
            .unwrap_err();
        assert_eq!(
            (error.code, error.message.as_str()),
            (MastErrorCode::Cache, "the Data folder does not exist")
        );

        let no_client = DownloadQueue::new(QueueConfig {
            client: Client::builder().user_agent("bad\nagent").build(),
            base_url: rig.server.base.clone(),
            storage: Arc::new(|| {
                Ok(Storage {
                    root: PathBuf::from("unused"),
                    quota_bytes: 1,
                })
            }),
            free_space: plenty,
            emit: Arc::new(|_: &MastDownloadItem| {}),
        });
        assert_eq!(
            no_client
                .enqueue(request("b7", vec![item("a_cal.fits", 1)]))
                .await
                .unwrap_err()
                .code,
            MastErrorCode::Network
        );
        assert!(rig.server.requests().is_empty());
    }

    #[tokio::test]
    async fn an_hlsp_item_is_revalidated_without_a_transfer_and_a_changed_etag_offers_redownload() {
        let first = body(64 * KIB, 4);
        let second = body(64 * KIB, 5);
        let rig = rig(Resource::new(first.clone()).change_etag_after(2, second.clone()));
        let path = rig.root.join(HLSP_FOLDER).join("ceers").join(CEERS_FILE);

        rig.queue
            .enqueue(request("h1", vec![hlsp(64 * KIB)]))
            .await
            .unwrap();
        let done = settle(&rig.queue, "h1:0").await;
        assert_eq!(
            (done.state, done.reused, done.verified),
            (MastDownloadState::Done, false, Some(true))
        );
        assert_eq!(done.path, path.to_string_lossy());
        assert_eq!(done.program, None);
        assert_eq!(std::fs::read(&path).unwrap(), first);

        rig.queue
            .enqueue(request("h2", vec![hlsp(64 * KIB)]))
            .await
            .unwrap();
        let reused = settle(&rig.queue, "h2:0").await;
        assert_eq!(
            (reused.state, reused.reused, reused.verified),
            (MastDownloadState::Done, true, Some(true))
        );
        let probe = rig.server.requests()[1].clone();
        assert_eq!(probe.header("range"), Some("bytes=0-0"));
        assert_eq!((probe.status, probe.body_bytes), (304, 0));
        assert_eq!(
            rig.queue
                .snapshot()
                .iter()
                .filter(|i| i.path == done.path)
                .count(),
            1,
            "a finished row is replaced, not repeated"
        );

        rig.queue
            .enqueue(request("h3", vec![hlsp(64 * KIB)]))
            .await
            .unwrap();
        let stale = settle(&rig.queue, "h3:0").await;
        assert_eq!(stale.state, MastDownloadState::Error);
        assert_eq!(
            stale.error.as_ref().map(|e| e.code),
            Some(MastErrorCode::Stale)
        );
        assert_eq!(stale.action, Some(MastDownloadAction::Redownload));
        assert_eq!(std::fs::read(&path).unwrap(), first);

        let mut again = hlsp(64 * KIB);
        again.replace = true;
        rig.queue.enqueue(request("h4", vec![again])).await.unwrap();
        let fresh = settle(&rig.queue, "h4:0").await;
        assert_eq!(
            (fresh.state, fresh.reused, fresh.verified, fresh.action),
            (MastDownloadState::Done, false, Some(true), None)
        );
        assert_eq!(std::fs::read(&path).unwrap(), second);
    }

    #[tokio::test]
    async fn a_foreign_file_waits_for_replace() {
        let payload = body(16 * KIB, 6);
        let rig = rig(Resource::new(payload.clone()));
        let path = rig.root.join("JWST").join("2739").join("a_cal.fits");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"someone else's file").unwrap();

        rig.queue
            .enqueue(request("f1", vec![item("a_cal.fits", 16 * KIB)]))
            .await
            .unwrap();
        let refused = settle(&rig.queue, "f1:0").await;
        assert_eq!(refused.state, MastDownloadState::Error);
        assert_eq!(
            refused.error.as_ref().map(|e| e.code),
            Some(MastErrorCode::BadInput)
        );
        assert_eq!(refused.action, Some(MastDownloadAction::Replace));
        assert_eq!(std::fs::read(&path).unwrap(), b"someone else's file");
        assert!(rig.server.requests().is_empty());

        let mut replace = item("a_cal.fits", 16 * KIB);
        replace.replace = true;
        rig.queue
            .enqueue(request("f2", vec![replace]))
            .await
            .unwrap();
        let done = settle(&rig.queue, "f2:0").await;
        assert_eq!(done.state, MastDownloadState::Done);
        assert_eq!(std::fs::read(&path).unwrap(), payload);
    }

    #[tokio::test]
    async fn a_cancelled_download_keeps_its_part_and_resumes() {
        let payload = body(1024 * KIB, 7);
        let rig = rig(Resource::new(payload.clone()).stall_after(512 * KIB));
        rig.queue
            .enqueue(request("c1", vec![item("a_cal.fits", 1024 * KIB)]))
            .await
            .unwrap();
        until(&rig.queue, "c1:0", |i| {
            i.state == MastDownloadState::Downloading
        })
        .await;
        let start = Instant::now();
        while rig.server.requests().is_empty() {
            assert!(start.elapsed() < PATIENCE);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
        assert_eq!(rig.queue.cancel("c1").unwrap(), 1);
        let cancelled = settle(&rig.queue, "c1:0").await;
        assert_eq!(cancelled.state, MastDownloadState::Cancelled);
        assert_eq!(cancelled.action, Some(MastDownloadAction::Resume));
        let path = rig.root.join("JWST").join("2739").join("a_cal.fits");
        let kept = std::fs::metadata(part_of(&path)).unwrap().len();
        assert!(kept > 0 && kept <= 512 * KIB, "{kept}");
        assert!(!path.exists());

        rig.queue
            .enqueue(request("c2", vec![item("a_cal.fits", 1024 * KIB)]))
            .await
            .unwrap();
        let done = settle(&rig.queue, "c2:0").await;
        assert_eq!(
            (done.state, done.resumed_from),
            (MastDownloadState::Done, kept)
        );
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        let resumed = rig.server.requests().last().cloned().unwrap();
        let range = format!("bytes={kept}-");
        assert_eq!(resumed.header("range"), Some(range.as_str()));
        assert_eq!(rig.queue.cancel("c2").unwrap(), 0);
        assert_eq!(
            rig.queue.cancel("bad id").unwrap_err().code,
            MastErrorCode::BadInput
        );
    }

    #[tokio::test]
    async fn a_queued_item_is_cancelled_without_a_request() {
        let rig = rig(Resource::new(body(2048 * KIB, 8)).slow_steady(256 * KIB));
        let items = ["a_cal.fits", "b_cal.fits", "c_cal.fits"]
            .iter()
            .map(|n| item(n, 2048 * KIB))
            .collect();
        rig.queue.enqueue(request("w1", items)).await.unwrap();
        until(&rig.queue, "w1:1", |i| {
            i.state == MastDownloadState::Downloading
        })
        .await;
        assert_eq!(
            find(&rig.queue, "w1:2").unwrap().state,
            MastDownloadState::Queued
        );
        assert_eq!(rig.queue.cancel("w1:2").unwrap(), 1);
        let waiting = find(&rig.queue, "w1:2").unwrap();
        assert_eq!(
            (waiting.state, waiting.action),
            (
                MastDownloadState::Cancelled,
                Some(MastDownloadAction::Retry)
            )
        );
        assert_eq!(rig.queue.cancel("w1").unwrap(), 2);
        settle(&rig.queue, "w1:0").await;
        settle(&rig.queue, "w1:1").await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let requests = rig.server.requests();
        assert_eq!(requests.len(), 2, "{requests:?}");
        assert!(requests.iter().all(|r| !r.target.contains("c_cal")));
    }

    #[tokio::test]
    async fn a_dropped_connection_offers_resume_and_continues_from_the_part() {
        let payload = body(256 * KIB, 9);
        let rig = rig(Resource::new(payload.clone()).drop_after(100 * KIB));
        rig.queue
            .enqueue(request("d1", vec![item("a_cal.fits", 256 * KIB)]))
            .await
            .unwrap();
        let dropped = settle(&rig.queue, "d1:0").await;
        assert_eq!(dropped.state, MastDownloadState::Error);
        assert_eq!(
            dropped.error.as_ref().map(|e| e.code),
            Some(MastErrorCode::Network)
        );
        assert_eq!(dropped.action, Some(MastDownloadAction::Resume));
        rig.queue
            .enqueue(request("d2", vec![item("a_cal.fits", 256 * KIB)]))
            .await
            .unwrap();
        let done = settle(&rig.queue, "d2:0").await;
        assert_eq!(
            (done.state, done.resumed_from),
            (MastDownloadState::Done, 100 * KIB)
        );
        assert_eq!(
            std::fs::read(rig.root.join("JWST").join("2739").join("a_cal.fits")).unwrap(),
            payload
        );
    }

    #[tokio::test]
    async fn a_listing_size_off_by_one_byte_leaves_nothing_behind() {
        let rig = rig(Resource::new(body(32 * KIB, 10)));
        rig.queue
            .enqueue(request("s1", vec![item("a_cal.fits", 32 * KIB + 1)]))
            .await
            .unwrap();
        let failed = settle(&rig.queue, "s1:0").await;
        assert_eq!(
            failed.error.as_ref().map(|e| e.code),
            Some(MastErrorCode::SizeMismatch)
        );
        assert_eq!(failed.action, Some(MastDownloadAction::Retry));
        let folder = rig.root.join("JWST").join("2739");
        let left: Vec<_> = std::fs::read_dir(&folder)
            .map(|d| d.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
    }

    #[tokio::test]
    async fn a_copy_without_validators_is_served_unverified() {
        let payload = body(8 * KIB, 11);
        let rig = rig(Resource::new(payload).no_etag().no_last_modified());
        rig.queue
            .enqueue(request("v1", vec![item("a_cal.fits", 8 * KIB)]))
            .await
            .unwrap();
        assert_eq!(settle(&rig.queue, "v1:0").await.verified, Some(true));
        rig.queue
            .enqueue(request("v2", vec![item("a_cal.fits", 8 * KIB)]))
            .await
            .unwrap();
        let reused = settle(&rig.queue, "v2:0").await;
        assert_eq!(
            (reused.state, reused.reused, reused.verified),
            (MastDownloadState::Done, true, Some(false))
        );
        assert_eq!(rig.server.requests().len(), 1);
    }

    #[tokio::test]
    async fn the_same_file_is_not_queued_twice_and_open_is_kept() {
        let rig = rig(Resource::new(body(2048 * KIB, 12)).slow_steady(512 * KIB));
        let mut opened = request(
            "o1",
            vec![
                item("a_cal.fits", 2048 * KIB),
                item("a_cal.fits", 2048 * KIB),
            ],
        );
        opened.open = true;
        let first = rig.queue.enqueue(opened).await.unwrap();
        assert_eq!(first.len(), 1);
        assert!(first[0].open);
        assert_eq!(first[0].opens_as, Some(OpensAs::Image));
        let second = rig
            .queue
            .enqueue(request("o2", vec![item("a_cal.fits", 2048 * KIB)]))
            .await
            .unwrap();
        assert_eq!(
            second.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["o1:0"]
        );
        assert_eq!(rig.queue.cancel("o1").unwrap(), 1);
        settle(&rig.queue, "o1:0").await;
    }

    #[test]
    fn quota_messages_give_sizes_in_the_unit_that_fits() {
        let tiny = admit(
            Space {
                used: 0,
                free: None,
            },
            0,
            Some(117_573_120),
            1_000_000,
        )
        .unwrap_err();
        assert_eq!(
            tiny.message,
            "the MAST download quota (1.0 MB) would be exceeded: 0 B downloaded or queued, 118 MB more requested"
        );
        let large = admit(
            Space {
                used: 19_500_000_000,
                free: None,
            },
            0,
            Some(1_200_000_000),
            20_000_000_000,
        )
        .unwrap_err();
        assert_eq!(
            large.message,
            "the MAST download quota (20.00 GB) would be exceeded: 19.50 GB downloaded or queued, 1.20 GB more requested"
        );
        let disk = admit(
            Space {
                used: 0,
                free: Some(3_400),
            },
            400,
            Some(5_000),
            u64::MAX,
        )
        .unwrap_err();
        assert_eq!(
            disk.message,
            "not enough free disk space: 3.4 KB free, 5.4 KB needed"
        );
    }

    fn hlsp_at(uri: &str) -> MastDownloadInput {
        MastDownloadInput {
            uri: Some(uri.to_owned()),
            ..hlsp(4096 * KIB)
        }
    }

    #[tokio::test]
    async fn hlsp_files_that_would_share_one_local_name_are_refused_not_merged() {
        let rig = rig(Resource::new(body(4096 * KIB, 21)).slow_steady(1024 * KIB));
        let v1 = &format!("mast:HLSP/ceers/v1/{CEERS_FILE}");
        let v2 = &format!("mast:HLSP/ceers/v2/{CEERS_FILE}");
        let error = rig
            .queue
            .enqueue(request("u1", vec![hlsp_at(v1), hlsp_at(v2)]))
            .await
            .unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput);
        assert!(
            error.message.contains(v1) && error.message.contains(v2),
            "{}",
            error.message
        );
        assert!(rig.queue.snapshot().is_empty());
        assert!(rig.server.requests().is_empty());

        let same = rig
            .queue
            .enqueue(request("u2", vec![hlsp_at(v1), hlsp_at(v1)]))
            .await
            .unwrap();
        assert_eq!(same.len(), 1);
        let later = rig
            .queue
            .enqueue(request("u3", vec![hlsp_at(v2)]))
            .await
            .unwrap_err();
        assert_eq!(later.code, MastErrorCode::BadInput);
        assert!(
            later.message.contains(v1) && later.message.contains(v2),
            "{}",
            later.message
        );
        assert_eq!(
            rig.queue
                .snapshot()
                .iter()
                .map(|i| (i.id.as_str().to_owned(), i.uri.clone()))
                .collect::<Vec<_>>(),
            [("u2:0".to_owned(), v1.clone())]
        );
        assert_eq!(rig.queue.cancel("u2").unwrap(), 1);
        settle(&rig.queue, "u2:0").await;
    }

    struct HeldServer {
        base: String,
        arrived: std::sync::mpsc::Receiver<()>,
        release: std::sync::mpsc::Sender<()>,
    }

    fn held_server(payload: Vec<u8>) -> HeldServer {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (arrived_tx, arrived) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut buffer = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => head.extend_from_slice(&buffer[..read]),
                }
            }
            let _ = arrived_tx.send(());
            let _ = release_rx.recv();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nETag: \"held\"\r\nContent-Length: {}\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.write_all(&payload);
        });
        HeldServer {
            base,
            arrived,
            release,
        }
    }

    fn queue_at(base: &str, root: PathBuf) -> DownloadQueue {
        DownloadQueue::new(QueueConfig {
            client: build_client(&HttpProfile::transfer()),
            base_url: base.to_owned(),
            storage: Arc::new(move || {
                Ok(Storage {
                    root: root.clone(),
                    quota_bytes: 20_000_000_000,
                })
            }),
            free_space: plenty,
            emit: Arc::new(|_: &MastDownloadItem| {}),
        })
    }

    #[tokio::test]
    async fn a_new_request_takes_back_a_cancel_the_stalled_transfer_has_not_seen() {
        let payload = body(64 * KIB, 22);
        let held = held_server(payload.clone());
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data").join("mast-test");
        let queue = queue_at(&held.base, root.clone());
        queue
            .enqueue(request("k1", vec![item("a_cal.fits", 64 * KIB)]))
            .await
            .unwrap();
        let start = Instant::now();
        while held.arrived.try_recv().is_err() {
            assert!(
                start.elapsed() < PATIENCE,
                "the request never reached the server"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(queue.cancel("k1").unwrap(), 1);
        let again = queue
            .enqueue(request("k2", vec![item("a_cal.fits", 64 * KIB)]))
            .await
            .unwrap();
        assert_eq!(
            again.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["k1:0"]
        );
        held.release.send(()).unwrap();
        let done = settle(&queue, "k1:0").await;
        assert_eq!((done.state, done.action), (MastDownloadState::Done, None));
        assert_eq!(
            std::fs::read(root.join("JWST").join("2739").join("a_cal.fits")).unwrap(),
            payload
        );
    }

    #[tokio::test]
    async fn a_cancel_taken_back_after_the_transfer_stopped_starts_it_again() {
        let rig = rig(Resource::new(body(4 * KIB, 24)));
        let held = Arc::clone(&rig.queue.inner.permits)
            .acquire_many_owned(DOWNLOAD_PERMITS as u32)
            .await
            .unwrap();
        rig.queue
            .enqueue(request("t1", vec![item("a_cal.fits", 4 * KIB)]))
            .await
            .unwrap();
        let cancel = {
            let mut entries = guard(&rig.queue.inner.entries);
            let entry = entries.iter_mut().find(|e| e.item.id == "t1:0").unwrap();
            entry.item.state = MastDownloadState::Downloading;
            Arc::clone(&entry.cancel)
        };
        let before = rig.events.lock().unwrap().len();
        assert!(!rig.queue.inner.finish(
            "t1:0",
            MastMissionId::Jwst,
            Err(DownloadError::Cancelled),
            false
        ));
        let item = find(&rig.queue, "t1:0").unwrap();
        assert_eq!(
            (item.state, item.action),
            (MastDownloadState::Downloading, None)
        );
        assert_eq!(rig.events.lock().unwrap().len(), before);
        cancel.store(true, Ordering::SeqCst);
        assert!(rig.queue.inner.finish(
            "t1:0",
            MastMissionId::Jwst,
            Err(DownloadError::Cancelled),
            false
        ));
        let item = find(&rig.queue, "t1:0").unwrap();
        assert_eq!(
            (item.state, item.action),
            (
                MastDownloadState::Cancelled,
                Some(MastDownloadAction::Resume)
            )
        );
        drop(held);
    }

    #[tokio::test]
    async fn a_replace_that_stops_keeps_the_record_of_the_copy_still_on_disk() {
        let payload = body(256 * KIB, 23);
        let rig = rig(Resource::new(payload.clone()).slow_steady(256 * KIB));
        let path = rig.root.join("JWST").join("2739").join("a_cal.fits");
        rig.queue
            .enqueue(request("r1", vec![item("a_cal.fits", 256 * KIB)]))
            .await
            .unwrap();
        assert_eq!(
            settle(&rig.queue, "r1:0").await.state,
            MastDownloadState::Done
        );
        rig.queue
            .enqueue(request("r2", vec![replacing("a_cal.fits", 256 * KIB)]))
            .await
            .unwrap();
        until(&rig.queue, "r2:0", |i| i.received > 0).await;
        assert_eq!(rig.queue.cancel("r2").unwrap(), 1);
        assert_eq!(
            settle(&rig.queue, "r2:0").await.state,
            MastDownloadState::Cancelled
        );
        rig.queue
            .enqueue(request("r3", vec![item("a_cal.fits", 256 * KIB)]))
            .await
            .unwrap();
        let kept = settle(&rig.queue, "r3:0").await;
        assert_eq!(
            (kept.state, kept.reused, kept.action),
            (MastDownloadState::Done, true, None)
        );
        assert_eq!(std::fs::read(&path).unwrap(), payload);
    }
}
