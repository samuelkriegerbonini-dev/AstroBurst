use std::collections::HashSet;
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use super::api::MissionEndpoint;
use super::auth;
use super::cache::{
    cached_image, dataset_dir, dir_info, evict_to_cap, list_file, read_stored, read_stored_at, remove_file, remove_tree, representative_file,
    safe_filename, stored_name, sweep_stale_temps, touch, write_atomic, write_stored, DiskBudget, ListKind, IMG_DIR, LISTS_DIR, LIST_TTL, REPRESENTATIVE_NAME,
    REPRESENTATIVE_STEM,
};
use super::client::ImageOp;
use super::download::{fetch, Fetched, ProgressThrottle, HEAD_KEEP};
use super::error::MastError;
use super::hub::{CachedSearch, MastHub, ProbeResult, StoredProducts};
use super::lookup::{self, NAMES_FILE};
use super::models::{DatasetIds, DatasetProduct, FieldMap, ImageListResult, SearchParams};
use super::ratelimit::is_rate_limit;
use crate::core::mast::errors::{bad_input, classify, internal, io_cause, OpKind, Operation};
use crate::core::mast::ids::validate_request_id;
use crate::core::mast::images::{merge_lists, needs_list_after_redirect, prefer_filter, product_stem, representative_entry};
use crate::core::mast::jpeg::{dimensions, quick_wcs, scale_wcs};
use crate::core::mast::local_level;
use crate::core::mast::missions::{merge_probe, mission, missions as static_missions, utc_date, utc_datetime};
use crate::core::mast::normalize::{dropped_conditions, normalize_page, PageRequest};
use crate::core::mast::object_filters::is_object_column;
use crate::core::mast::preview::{plan, prepared_name, resize_jpeg, PreviewPlan};
use crate::core::mast::products::classify as classify_products;
use crate::core::mast::query::{build_params, canonical_key, probe_params, validate, ValidQuery, PAGE_LIMIT};
use crate::core::mast::types::{
    MastAvailability, MastCacheInfo, MastCacheKind, MastCmdError, MastErrorCode, MastErrorContext, MastImageRef, MastImageSet, MastLevelClass,
    MastMission, MastMissionId, MastPreviewRef, MastProductList, MastProgressEvent, MastQuery, MastResolved, MastSearchPage, MastTokenStatus,
    MissionCapabilities, PreviewSize, PreviewSource, ProgressStage,
};

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

pub fn missions(hub: &MastHub) -> Vec<MastMission> {
    let probes = hub.probes();
    static_missions()
        .into_iter()
        .map(|entry| match probes.get(&entry.id) {
            Some(probe) => merge_probe(entry, probe.availability, &probe.checked_on),
            None => entry,
        })
        .collect()
}

fn failure(hub: &MastHub, error: &MastError, op: &Operation, sent: Instant) -> MastCmdError {
    let consecutive = hub.limiter().record(error, op, sent, Instant::now());
    classify(error, op, consecutive)
}

const IMAGE_ATTEMPTS: u32 = 2;

struct Tried<T> {
    result: Result<T, MastError>,
    first_sent: Instant,
    last_sent: Instant,
    attempts: u32,
}

impl<T> Tried<T> {
    fn classified(self, hub: &MastHub, op: &Operation) -> Result<T, MastCmdError> {
        let Tried { result, first_sent, last_sent, attempts } = self;
        result.map_err(|error| {
            if worth_retrying(&error) {
                hub.limiter().note_server_trouble(Instant::now());
            }
            let mut failed = failure(hub, &error, op, last_sent);
            failed.context.attempts = Some(attempts);
            failed.context.elapsed_ms = Some(elapsed_ms(first_sent));
            failed
        })
    }
}

fn worth_retrying(error: &MastError) -> bool {
    match error {
        MastError::Transport { .. } => true,
        MastError::Status { status, .. } => *status >= 500 && !is_rate_limit(error),
        _ => false,
    }
}

fn jittered(base: Duration) -> Duration {
    let spread = (base.as_millis() / 4) as u64;
    base + Duration::from_millis(rand::random::<u64>() % (spread + 1))
}

fn may_retry(hub: &MastHub) -> bool {
    let now = Instant::now();
    hub.limiter().open_window(now).is_none() && !hub.limiter().server_trouble(now)
}

async fn with_retry<T, F, Fut>(hub: &MastHub, mut call: F) -> Tried<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, MastError>>,
{
    let first_sent = Instant::now();
    let mut tried = Tried { result: call().await, first_sent, last_sent: first_sent, attempts: 1 };
    while tried.attempts < IMAGE_ATTEMPTS && tried.result.as_ref().is_err_and(worth_retrying) && may_retry(hub) {
        tokio::time::sleep(jittered(hub.settings().retry_backoff)).await;
        if !may_retry(hub) {
            break;
        }
        tried.last_sent = Instant::now();
        tried.attempts += 1;
        tried.result = call().await;
    }
    tried
}

pub async fn probe_mission(hub: &MastHub, id: MastMissionId) -> Result<MastMission, MastCmdError> {
    let availability = match hub.endpoint(id, OpKind::Probe) {
        Err(error) if error.code == MastErrorCode::Unauthorized => MastAvailability::Token,
        Err(error) => return Err(error),
        Ok(endpoint) => {
            let op = Operation::mission(id, OpKind::Probe, "search");
            let sent = Instant::now();
            match endpoint.search::<FieldMap>(&probe_params()).await {
                Ok(_) => {
                    hub.limiter().on_success();
                    MastAvailability::Open
                }
                Err(error) => {
                    let classified = failure(hub, &error, &op, sent);
                    match classified.code {
                        MastErrorCode::NotDeployed => MastAvailability::NotDeployed,
                        MastErrorCode::Unauthorized => MastAvailability::Token,
                        _ => return Err(classified),
                    }
                }
            }
        }
    };
    let checked_on = utc_date(SystemTime::now());
    hub.record_probe(id, ProbeResult { availability, checked_on: checked_on.clone() });
    Ok(merge_probe(mission(id), availability, &checked_on))
}

pub async fn resolve(hub: &MastHub, name: &str) -> Result<Option<MastResolved>, MastCmdError> {
    lookup::resolve(hub, name).await
}

pub async fn search(hub: &MastHub, request_id: &str, query: &MastQuery, refresh: bool) -> Result<MastSearchPage, MastCmdError> {
    auth::screen_unauthorized(hub, run_search(hub, request_id, query, refresh)).await
}

async fn run_search(hub: &MastHub, request_id: &str, query: &MastQuery, refresh: bool) -> Result<MastSearchPage, MastCmdError> {
    let started = Instant::now();
    validate_request_id(request_id)?;
    let valid = validate(query)?;
    let endpoint = hub.endpoint(valid.mission, OpKind::Search)?;
    let unfiltered_total = if refresh { None } else { unfiltered_total(hub, &valid) };
    if let Some(plan) = local_level::plan(&valid) {
        if let Some((wide, from_cache)) = wide_answer(hub, &endpoint, request_id, &plan.wide, refresh, started).await? {
            if let Some(mut page) = local_level::page(&wide, &valid, &plan) {
                page.request_id = request_id.to_owned();
                page.elapsed_ms = elapsed_ms(started);
                page.from_cache = from_cache;
                page.unfiltered_total = unfiltered_total;
                hub.remember_row_filters(valid.mission, &page.rows);
                return Ok(page);
            }
        }
    }
    let mut page = search_page(hub, &endpoint, request_id, &valid, refresh, started).await?;
    if !page.from_cache {
        hub.remember_row_filters(valid.mission, &page.rows);
    }
    page.unfiltered_total = unfiltered_total;
    Ok(page)
}

async fn wide_answer(
    hub: &MastHub,
    endpoint: &MissionEndpoint,
    request_id: &str,
    wide: &ValidQuery,
    refresh: bool,
    started: Instant,
) -> Result<Option<(Arc<MastSearchPage>, bool)>, MastCmdError> {
    let params = build_params(wide);
    let key = canonical_key(wide.mission, &params);
    if !refresh {
        let cached = hub.searches().get(&key, Instant::now());
        match cached {
            Some(CachedSearch::Page(page)) => return Ok(Some((page, true))),
            Some(CachedSearch::FullWideAnswer) => return Ok(None),
            None => {}
        }
    }
    let page = fetched_page(hub, endpoint, request_id, wide, &params, started).await?;
    if !local_level::is_complete(&page) {
        hub.searches().insert(key, CachedSearch::FullWideAnswer, Instant::now());
        return Ok(None);
    }
    let page = Arc::new(page);
    hub.searches().insert(key, CachedSearch::Page(Arc::clone(&page)), Instant::now());
    Ok(Some((page, false)))
}

async fn search_page(
    hub: &MastHub,
    endpoint: &MissionEndpoint,
    request_id: &str,
    valid: &ValidQuery,
    refresh: bool,
    started: Instant,
) -> Result<MastSearchPage, MastCmdError> {
    let params = build_params(valid);
    let key = canonical_key(valid.mission, &params);
    if !refresh {
        let cached = hub.searches().get(&key, Instant::now());
        if let Some(CachedSearch::Page(stored)) = cached {
            let mut page = MastSearchPage::clone(&stored);
            page.request_id = request_id.to_owned();
            page.from_cache = true;
            page.elapsed_ms = elapsed_ms(started);
            page.dropped_conditions = dropped_conditions(valid, &page.applied);
            return Ok(page);
        }
    }
    let page = fetched_page(hub, endpoint, request_id, valid, &params, started).await?;
    hub.searches().insert(key, CachedSearch::Page(Arc::new(page.clone())), Instant::now());
    Ok(page)
}

async fn fetched_page(
    hub: &MastHub,
    endpoint: &MissionEndpoint,
    request_id: &str,
    valid: &ValidQuery,
    params: &SearchParams,
    started: Instant,
) -> Result<MastSearchPage, MastCmdError> {
    let op = Operation::mission(valid.mission, OpKind::Search, "search");
    let sent = Instant::now();
    let results = match endpoint.search::<FieldMap>(params).await {
        Ok(results) => results,
        Err(error) => return Err(failure(hub, &error, &op, sent)),
    };
    hub.limiter().on_success();
    let request_body = serde_json::to_value(params).unwrap_or(Value::Null);
    Ok(normalize_page(
        valid.mission,
        results,
        PageRequest { request_id, query: valid, request_body, elapsed_ms: elapsed_ms(started) },
    ))
}

fn unfiltered_total(hub: &MastHub, valid: &ValidQuery) -> Option<i64> {
    let object = |column: &str| is_object_column(valid.mission, column);
    if !valid.conditions.iter().any(|(column, _)| object(column)) {
        return None;
    }
    let plain = ValidQuery {
        conditions: valid.conditions.iter().filter(|(column, _)| !object(column)).cloned().collect(),
        offset: 0,
        limit: PAGE_LIMIT,
        ..valid.clone()
    };
    let key = canonical_key(plain.mission, &build_params(&plain));
    let cached = hub.searches().get(&key, Instant::now());
    match cached {
        Some(CachedSearch::Page(page)) if page.total_reliable => page.total_reported,
        _ => None,
    }
}

pub type ProgressSink<'a> = dyn Fn(MastProgressEvent) + Send + Sync + 'a;

const THUMBNAIL_LIST: &str = "THUMBNAIL";
const PREVIEW_LIST: &str = "PREVIEW";

fn capabilities(hub: &MastHub, id: MastMissionId) -> MissionCapabilities {
    let entry = match hub.probes().get(&id) {
        Some(probe) => merge_probe(mission(id), probe.availability, &probe.checked_on),
        None => mission(id),
    };
    entry.capabilities
}

const NO_IMAGES: &str = "no image endpoint for this mission";

fn not_deployed(op: &Operation, message: &str) -> MastCmdError {
    MastCmdError::new(MastErrorCode::NotDeployed, message).with_context(MastErrorContext {
        mission: op.mission,
        operation: Some(op.path.clone()),
        ..Default::default()
    })
}

fn not_image(op: &Operation) -> MastCmdError {
    MastCmdError::new(MastErrorCode::NotImage, "response is not an image").with_context(MastErrorContext {
        operation: Some(op.path.clone()),
        ..Default::default()
    })
}

fn no_thumbnail(op: &Operation) -> MastCmdError {
    MastCmdError::new(MastErrorCode::NoImage, "no thumbnail listed for this dataset").with_context(MastErrorContext {
        operation: Some(op.path.clone()),
        ..Default::default()
    })
}

fn cache_failure(error: &std::io::Error) -> MastCmdError {
    MastCmdError::new(MastErrorCode::Cache, io_cause(error))
}

fn with_operation(mut error: MastCmdError, op: &Operation) -> MastCmdError {
    error.context.operation.get_or_insert_with(|| op.path.clone());
    error
}

fn window_check(hub: &MastHub, op: &Operation) -> Result<(), MastCmdError> {
    match hub.limiter().open_window(Instant::now()) {
        None => Ok(()),
        Some(window) => Err(MastCmdError::new(MastErrorCode::RateLimited, "rate limited, waiting before the next image request")
            .with_context(MastErrorContext {
                retry_after_sec: Some(window.remaining_secs),
                consecutive: Some(window.consecutive),
                operation: Some(op.path.clone()),
                ..Default::default()
            })),
    }
}

fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(HEAD_KEEP);
    std::fs::File::open(path)?.take(HEAD_KEEP as u64).read_to_end(&mut head)?;
    Ok(head)
}

fn spawn_eviction(budget: Arc<DiskBudget>, dir: PathBuf) {
    let job = move || {
        let remaining = run_eviction(&budget, &dir);
        budget.finish(remaining);
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => drop(handle.spawn_blocking(job)),
        Err(_) => job(),
    }
}

fn run_eviction(budget: &DiskBudget, dir: &Path) -> u64 {
    match evict_to_cap(dir, budget.cap(), budget.target()) {
        Ok(remaining) => remaining,
        Err(error) => {
            log::warn!("MAST cache eviction stopped early: {error}");
            dir_info(dir).bytes
        }
    }
}

fn note_image_write(hub: &MastHub, bytes: u64) {
    let budget = hub.image_budget();
    if budget.note_write(bytes) {
        spawn_eviction(budget, hub.cache_root().join(IMG_DIR));
    }
}

pub(crate) fn note_list_write(hub: &MastHub, bytes: u64) {
    let budget = hub.list_budget();
    if budget.note_write(bytes) {
        spawn_eviction(budget, hub.cache_root().join(LISTS_DIR));
    }
}

pub fn evict_cache(hub: &MastHub) {
    let longest = hub.settings().timeout(OpKind::Preview);
    sweep_stale_temps(hub.cache_root(), std::process::id(), SystemTime::now(), longest);
    for (budget, dir) in [
        (hub.image_budget(), hub.cache_root().join(IMG_DIR)),
        (hub.list_budget(), hub.cache_root().join(LISTS_DIR)),
    ] {
        if budget.begin() {
            let remaining = run_eviction(&budget, &dir);
            budget.finish(remaining);
        }
    }
}

struct ImageCall<'a> {
    hub: &'a MastHub,
    request_id: &'a str,
    id: MastMissionId,
    dataset: &'a str,
    dir: PathBuf,
    started: Instant,
}

fn worth_storing(set: &MastImageSet) -> bool {
    set.entries.iter().any(|entry| entry.thumbnail_filename.is_some()) && set.entries.iter().any(|entry| entry.preview_filename.is_some())
}

fn cached_set(hub: &MastHub, id: MastMissionId, dataset: &str) -> Option<MastImageSet> {
    let key = (id, dataset.to_owned());
    if let Some(set) = hub.image_sets().get(&key, Instant::now()) {
        return Some(set);
    }
    let path = list_file(hub.cache_root(), id, dataset, ListKind::Images).ok()?;
    let set: MastImageSet = read_stored(&path, LIST_TTL, SystemTime::now())?;
    if set.mission != id || set.dataset_id != dataset || !worth_storing(&set) {
        return None;
    }
    touch(&path);
    hub.image_sets().insert(key, set.clone(), Instant::now());
    Some(set)
}

fn store_set(hub: &MastHub, set: &MastImageSet) {
    hub.image_sets().insert((set.mission, set.dataset_id.clone()), set.clone(), Instant::now());
    if !worth_storing(set) {
        return;
    }
    let Ok(path) = list_file(hub.cache_root(), set.mission, &set.dataset_id, ListKind::Images) else { return };
    match write_stored(&path, set, SystemTime::now()) {
        Ok(bytes) => note_list_write(hub, bytes),
        Err(error) => log::warn!("MAST image list was not cached: {error}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caller {
    Card,
    Pane,
}

async fn fetch_list(hub: &MastHub, endpoint: &MissionEndpoint, dataset: &str, category: &str, caller: Caller) -> Tried<Vec<ImageListResult>> {
    let _permit = match caller {
        Caller::Pane => hub.list_permits().acquire().await,
        Caller::Card => {
            let _turn = hub.card_list_turn().lock().await;
            hub.list_permits().acquire().await
        }
    };
    with_retry(hub, move || endpoint.get_image_list(dataset, Some(category))).await
}

async fn load_set(hub: &MastHub, id: MastMissionId, dataset: &str, caller: Caller) -> Result<(MastImageSet, bool), MastCmdError> {
    if let Some(set) = cached_set(hub, id, dataset) {
        return Ok((set, true));
    }
    let op = Operation::mission(id, OpKind::ImageList, "get_image_list");
    let endpoint = hub.endpoint(id, OpKind::ImageList)?;
    window_check(hub, &op)?;
    let (thumbs, previews) = tokio::join!(
        fetch_list(hub, &endpoint, dataset, THUMBNAIL_LIST, caller),
        fetch_list(hub, &endpoint, dataset, PREVIEW_LIST, caller)
    );
    let thumbs = thumbs.classified(hub, &op)?;
    let previews = match previews.classified(hub, &op) {
        Ok(previews) => previews,
        Err(_) if caller == Caller::Card => return Ok((merge_lists(id, dataset, &thumbs, &[]), false)),
        Err(error) => return Err(error),
    };
    hub.limiter().on_success();
    let set = merge_lists(id, dataset, &thumbs, &previews);
    store_set(hub, &set);
    Ok((set, false))
}

fn hinted(hub: &MastHub, mut set: MastImageSet) -> MastImageSet {
    if let Some(filter) = hub.row_filter(set.mission, &set.dataset_id) {
        prefer_filter(&mut set, &filter);
    }
    set
}

fn stamp(hub: &MastHub, set: MastImageSet, started: Instant, from_cache: bool) -> MastImageSet {
    let mut set = hinted(hub, set);
    set.elapsed_ms = elapsed_ms(started);
    set.from_cache = from_cache;
    set
}

pub async fn image_list(hub: &MastHub, id: MastMissionId, dataset_id: &str) -> Result<MastImageSet, MastCmdError> {
    auth::screen_unauthorized(hub, run_image_list(hub, id, dataset_id)).await
}

async fn run_image_list(hub: &MastHub, id: MastMissionId, dataset_id: &str) -> Result<MastImageSet, MastCmdError> {
    let started = Instant::now();
    dataset_dir(hub.cache_root(), id, dataset_id)?;
    if !capabilities(hub, id).images {
        return Err(not_deployed(&Operation::mission(id, OpKind::ImageList, "get_image_list"), NO_IMAGES));
    }
    hub.client_for(id, OpKind::ImageList)?;
    let (set, from_cache) = load_set(hub, id, dataset_id, Caller::Pane).await?;
    Ok(stamp(hub, set, started, from_cache))
}

impl ImageCall<'_> {
    fn image_ref(&self, path: &Path, head: &[u8], bytes: u64, filename: &str, image_count: Option<usize>, from_cache: bool) -> Option<MastImageRef> {
        let (width, height, _) = dimensions(head)?;
        Some(MastImageRef {
            request_id: self.request_id.to_owned(),
            path: display_path(path),
            width,
            height,
            bytes,
            filename: filename.to_owned(),
            wcs: quick_wcs(head).and_then(|wcs| wcs.summary(width, height)),
            image_count,
            elapsed_ms: elapsed_ms(self.started),
            from_cache,
        })
    }

    fn cached(&self, path: &Path, filename: &str, image_count: Option<usize>) -> Option<MastImageRef> {
        let head = read_head(path).ok()?;
        let bytes = std::fs::metadata(path).ok()?.len();
        match self.image_ref(path, &head, bytes, filename, image_count, true) {
            Some(found) => {
                touch(path);
                Some(found)
            }
            None => {
                let _ = std::fs::remove_file(path);
                None
            }
        }
    }

    fn cached_named(&self, filename: &str, image_count: Option<usize>) -> Option<MastImageRef> {
        let path = cached_image(&self.dir, filename)?;
        self.cached(&path, filename, image_count)
    }

    fn commit(&self, fetched: Fetched, name: &str, op: &Operation) -> Result<(PathBuf, Vec<u8>), MastCmdError> {
        if dimensions(&fetched.head).is_none() {
            return Err(not_image(op));
        }
        let path = self.dir.join(stored_name(name, fetched.download.kind));
        let bytes = fetched.download.received;
        let head = fetched.head.clone();
        fetched.commit(&path).map_err(|error| match error {
            MastError::Io(source) => cache_failure(&source),
            other => classify(&other, op, 0),
        })?;
        note_image_write(self.hub, bytes);
        Ok((path, head))
    }

    fn finish(&self, fetched: Fetched, name: &str, image_count: Option<usize>, op: &Operation) -> Result<MastImageRef, MastCmdError> {
        let bytes = fetched.download.received;
        let shown = if name == REPRESENTATIVE_STEM { stored_name(name, fetched.download.kind) } else { name.to_owned() };
        let (path, head) = self.commit(fetched, name, op)?;
        self.image_ref(&path, &head, bytes, &shown, image_count, false).ok_or_else(|| not_image(op))
    }

    async fn download_thumbnail(&self, endpoint: &MissionEndpoint, filename: Option<&str>, op: &Operation) -> Result<Fetched, MastCmdError> {
        self.sent_thumbnail(endpoint, filename, op).await.0
    }

    async fn sent_thumbnail(&self, endpoint: &MissionEndpoint, filename: Option<&str>, op: &Operation) -> (Result<Fetched, MastCmdError>, bool) {
        if let Err(error) = window_check(self.hub, op) {
            return (Err(error), false);
        }
        let _permit = self.hub.thumb_permits().acquire().await;
        let calm = !self.hub.limiter().server_trouble(Instant::now());
        let (dataset, dir) = (self.dataset, self.dir.as_path());
        let tried = with_retry(self.hub, move || async move {
            fetch(endpoint, ImageOp::Thumbnail, filename, Some(dataset), ImageOp::Thumbnail.cap(), dir, &mut |_, _| {}).await
        })
        .await;
        let fetched = tried.classified(self.hub, op);
        if fetched.is_ok() {
            self.hub.limiter().on_success();
        }
        (fetched, calm)
    }

    async fn named_thumbnail(&self, endpoint: &MissionEndpoint, filename: &str, op: &Operation) -> Result<MastImageRef, MastCmdError> {
        let (set, _) = load_set(self.hub, self.id, self.dataset, Caller::Card).await?;
        if !set.entries.iter().any(|entry| entry.thumbnail_filename.as_deref() == Some(filename)) {
            return Err(bad_input("filename", "is not in the thumbnail list of this dataset"));
        }
        let count = Some(set.entries.len());
        if let Some(found) = self.cached_named(filename, count) {
            return Ok(found);
        }
        let fetched = self.download_thumbnail(endpoint, Some(filename), op).await?;
        self.finish(fetched, filename, count, op)
    }

    fn keep_representative_name(&self, name: &str) {
        match write_atomic(&self.dir.join(REPRESENTATIVE_NAME), name.as_bytes()) {
            Ok(bytes) => note_image_write(self.hub, bytes),
            Err(error) => log::warn!("MAST representative name was not kept: {error}"),
        }
    }

    fn kept_representative_name(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.dir.join(REPRESENTATIVE_NAME)).ok()?;
        safe_filename(text.trim()).ok().map(str::to_owned)
    }

    fn cached_representative(&self, set: Option<&MastImageSet>) -> Option<MastImageRef> {
        let count = set.map(|set| set.entries.len());
        if let Some(name) = self.hub.chosen_representative(self.id, self.dataset) {
            if let Some(found) = self.cached_named(&name, count) {
                touch(&self.dir.join(REPRESENTATIVE_NAME));
                return Some(found);
            }
        }
        if let Some(name) = self.kept_representative_name() {
            if let Some(found) = self.cached_named(&name, count) {
                touch(&self.dir.join(REPRESENTATIVE_NAME));
                self.hub.remember_representative(self.id, self.dataset, &name);
                return Some(found);
            }
        }
        if let Some(path) = representative_file(&self.dir) {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(found) = self.cached(&path, &name, count) {
                return Some(found);
            }
        }
        let name = representative_entry(set?)?.thumbnail_filename.clone()?;
        self.cached_named(&name, count)
    }

    async fn representative_thumbnail(&self, endpoint: &MissionEndpoint, level_class: MastLevelClass, op: &Operation) -> Result<MastImageRef, MastCmdError> {
        let cached = cached_set(self.hub, self.id, self.dataset).map(|set| hinted(self.hub, set));
        if let Some(found) = self.cached_representative(cached.as_ref()) {
            return Ok(found);
        }
        let known = cached.as_ref().map(|set| set.entries.len());
        let mut first_error = None;
        let mut stand_in = None;
        if level_class != MastLevelClass::Exposure && !self.hub.fileset_failed(self.id, level_class) {
            let (sent, calm) = self.sent_thumbnail(endpoint, None, op).await;
            match sent {
                Ok(fetched) => match served_name(&fetched.download.final_url) {
                    Some(name) if self.id == MastMissionId::Hst && needs_list_after_redirect(&name) => {
                        let bytes = fetched.download.received;
                        match self.commit(fetched, &name, op) {
                            Ok((path, head)) => stand_in = self.image_ref(&path, &head, bytes, &name, None, false),
                            Err(error) => log::warn!("MAST exposure thumbnail was not kept: {}", error.message),
                        }
                    }
                    Some(name) => {
                        let found = self.finish(fetched, &name, known, op)?;
                        self.hub.remember_representative(self.id, self.dataset, &name);
                        self.keep_representative_name(&name);
                        return Ok(found);
                    }
                    None => return self.finish(fetched, REPRESENTATIVE_STEM, known, op),
                },
                Err(error) if falls_back_to_list(&error, calm) && self.hub.limiter().open_window(Instant::now()).is_none() => {
                    if error.code == MastErrorCode::NotImage {
                        self.hub.remember_fileset_failure(self.id, level_class);
                    }
                    first_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        match (self.listed_representative(endpoint, first_error, op).await, stand_in) {
            (Err(error), Some(kept)) => {
                log::warn!("MAST representative thumbnail fell back to the exposure thumbnail: {}", error.message);
                Ok(kept)
            }
            (listed, _) => listed,
        }
    }

    async fn listed_representative(&self, endpoint: &MissionEndpoint, first_error: Option<MastCmdError>, op: &Operation) -> Result<MastImageRef, MastCmdError> {
        let set = match load_set(self.hub, self.id, self.dataset, Caller::Card).await {
            Ok((set, _)) => hinted(self.hub, set),
            Err(error) => return Err(first_cause(first_error, error)),
        };
        let count = Some(set.entries.len());
        let Some(name) = representative_entry(&set).and_then(|entry| entry.thumbnail_filename.clone()) else {
            return Err(first_error.unwrap_or_else(|| no_thumbnail(op)));
        };
        if let Some(found) = self.cached_named(&name, count) {
            return Ok(found);
        }
        let fetched = self.download_thumbnail(endpoint, Some(&name), op).await?;
        self.finish(fetched, &name, count, op)
    }

    fn cached_original(&self, filename: &str) -> Option<(PathBuf, Vec<u8>)> {
        let path = cached_image(&self.dir, filename)?;
        let head = read_head(&path).ok()?;
        if dimensions(&head).is_some() {
            Some((path, head))
        } else {
            let _ = std::fs::remove_file(&path);
            None
        }
    }

    async fn download_preview(
        &self,
        endpoint: &MissionEndpoint,
        filename: &str,
        throttle: &mut ProgressThrottle,
        progress: &ProgressSink<'_>,
        op: &Operation,
    ) -> Result<(PathBuf, Vec<u8>, bool), MastCmdError> {
        window_check(self.hub, op)?;
        let _permit = self.hub.preview_permits().acquire().await;
        if let Some((path, head)) = self.cached_original(filename) {
            return Ok((path, head, false));
        }
        let sent = Instant::now();
        let fetched = fetch(endpoint, ImageOp::Preview, Some(filename), Some(self.dataset), ImageOp::Preview.cap(), &self.dir, &mut |received, total| {
            if let Some(event) = throttle.download(received, total, Instant::now()) {
                progress(event);
            }
        })
        .await
        .map_err(|error| failure(self.hub, &error, op, sent))?;
        self.hub.limiter().on_success();
        let (path, head) = self.commit(fetched, filename, op)?;
        Ok((path, head, true))
    }

    async fn prepare(&self, original: &Path, target: &Path, step: PreviewPlan, op: &Operation) -> Result<(), MastCmdError> {
        let permit = self.hub.decode_permits().acquire_owned().await.map_err(|_| internal())?;
        let (source, destination) = (original.to_path_buf(), target.to_path_buf());
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let bytes = std::fs::read(&source).map_err(|error| cache_failure(&error))?;
            let prepared = resize_jpeg(&bytes, step)?;
            write_atomic(&destination, &prepared).map_err(|error| cache_failure(&error))
        });
        let written = job.await.map_err(|_| internal())?.map_err(|error| with_operation(error, op))?;
        note_image_write(self.hub, written);
        Ok(())
    }
}

fn falls_back_to_list(error: &MastCmdError, calm: bool) -> bool {
    match error.code {
        MastErrorCode::NoImage | MastErrorCode::NotImage => true,
        MastErrorCode::Server => calm,
        _ => false,
    }
}

fn first_cause(first: Option<MastCmdError>, listed: MastCmdError) -> MastCmdError {
    match first {
        Some(first) if first.code == MastErrorCode::Server || listed.code == MastErrorCode::NoImage => first,
        _ => listed,
    }
}

fn served_name(final_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(final_url).ok()?;
    let name = url.path_segments()?.next_back()?.to_owned();
    let lower = name.to_ascii_lowercase();
    let image = lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png");
    (image && safe_filename(&name).is_ok()).then_some(name)
}

pub async fn thumbnail(
    hub: &MastHub,
    request_id: &str,
    id: MastMissionId,
    dataset_id: &str,
    level_class: MastLevelClass,
    filename: Option<&str>,
) -> Result<MastImageRef, MastCmdError> {
    auth::screen_unauthorized(hub, run_thumbnail(hub, request_id, id, dataset_id, level_class, filename)).await
}

async fn run_thumbnail(
    hub: &MastHub,
    request_id: &str,
    id: MastMissionId,
    dataset_id: &str,
    level_class: MastLevelClass,
    filename: Option<&str>,
) -> Result<MastImageRef, MastCmdError> {
    let started = Instant::now();
    validate_request_id(request_id)?;
    let dir = dataset_dir(hub.cache_root(), id, dataset_id)?;
    if let Some(name) = filename {
        safe_filename(name)?;
    }
    let op = Operation::mission(id, OpKind::Thumbnail, "get_image_thumbnail");
    if !capabilities(hub, id).images {
        return Err(not_deployed(&op, NO_IMAGES));
    }
    let endpoint = hub.endpoint(id, OpKind::Thumbnail)?;
    let mut flight = hub.thumbnail_flight(id, dataset_id, filename).await;
    if let Some(answer) = flight.earlier_failure() {
        return Err(answer);
    }
    let call = ImageCall { hub, request_id, id, dataset: dataset_id, dir, started };
    let result = match filename {
        Some(name) => call.named_thumbnail(&endpoint, name, &op).await,
        None => call.representative_thumbnail(&endpoint, level_class, &op).await,
    };
    if let Err(error) = &result {
        if error.code != MastErrorCode::Cancelled {
            flight.share_failure(error);
        }
    }
    result
}

fn prepared_matches(path: &Path, width: u32, height: u32) -> bool {
    read_head(path).ok().and_then(|head| dimensions(&head)).is_some_and(|(w, h, _)| (w, h) == (width, height))
}

pub async fn preview(
    hub: &MastHub,
    request_id: &str,
    id: MastMissionId,
    dataset_id: &str,
    filename: &str,
    size: PreviewSize,
    progress: &ProgressSink<'_>,
) -> Result<MastPreviewRef, MastCmdError> {
    auth::screen_unauthorized(hub, run_preview(hub, request_id, id, dataset_id, filename, size, progress)).await
}

async fn run_preview(
    hub: &MastHub,
    request_id: &str,
    id: MastMissionId,
    dataset_id: &str,
    filename: &str,
    size: PreviewSize,
    progress: &ProgressSink<'_>,
) -> Result<MastPreviewRef, MastCmdError> {
    let started = Instant::now();
    validate_request_id(request_id)?;
    let dir = dataset_dir(hub.cache_root(), id, dataset_id)?;
    safe_filename(filename)?;
    let op = Operation::mission(id, OpKind::Preview, "get_image_preview");
    if !capabilities(hub, id).images {
        return Err(not_deployed(&op, NO_IMAGES));
    }
    let endpoint = hub.endpoint(id, OpKind::Preview)?;
    let call = ImageCall { hub, request_id, id, dataset: dataset_id, dir, started };
    let (set, _) = load_set(hub, id, dataset_id, Caller::Pane).await?;
    if !set.entries.iter().any(|entry| entry.preview_filename.as_deref() == Some(filename)) {
        return Err(bad_input("filename", "is not in the preview list of this dataset"));
    }
    let mut throttle = ProgressThrottle::new(request_id);
    let (original, head, downloaded) = match call.cached_original(filename) {
        Some((path, head)) => (path, head, false),
        None => call.download_preview(&endpoint, filename, &mut throttle, progress, &op).await?,
    };
    let (width, height, channels) = dimensions(&head).ok_or_else(|| not_image(&op))?;
    let source_bytes = std::fs::metadata(&original).map_err(|error| cache_failure(&error))?.len();
    let wcs = quick_wcs(&head);
    let step = plan(width, height, size);
    let (path, shown_width, shown_height, shown_wcs) = match step {
        PreviewPlan::PassThrough => (original.clone(), width, height, wcs.and_then(|w| w.summary(width, height))),
        PreviewPlan::TooLarge => {
            let _ = std::fs::remove_file(&original);
            return Err(MastCmdError::new(MastErrorCode::TooLarge, "image is too large to prepare a preview").with_context(MastErrorContext {
                bytes: Some(source_bytes),
                operation: Some(op.path.clone()),
                ..Default::default()
            }));
        }
        PreviewPlan::Resize { width: target_width, height: target_height, factor } => {
            let target = call.dir.join(prepared_name(&product_stem(filename), size));
            if !prepared_matches(&target, target_width, target_height) {
                progress(throttle.stage(ProgressStage::Decode, Instant::now()));
                if let Err(error) = call.prepare(&original, &target, step, &op).await {
                    if error.code == MastErrorCode::Decode {
                        let _ = std::fs::remove_file(&original);
                    }
                    return Err(error);
                }
            }
            let scaled = wcs.map(|w| scale_wcs(w, factor)).and_then(|w| w.summary(target_width, target_height));
            (target, target_width, target_height, scaled)
        }
    };
    touch(&original);
    if path != original {
        touch(&path);
    }
    let bytes = std::fs::metadata(&path).map_err(|error| cache_failure(&error))?.len();
    progress(throttle.stage(ProgressStage::Ready, Instant::now()));
    Ok(MastPreviewRef {
        image: MastImageRef {
            request_id: request_id.to_owned(),
            path: display_path(&path),
            width: shown_width,
            height: shown_height,
            bytes,
            filename: filename.to_owned(),
            wcs: shown_wcs,
            image_count: Some(set.entries.len()),
            elapsed_ms: elapsed_ms(started),
            from_cache: !downloaded,
        },
        size,
        channels,
        source: PreviewSource { width, height, bytes: source_bytes },
    })
}

pub const PRODUCT_CHUNK: usize = 50;
pub const PRODUCT_IDS_MAX: usize = 2000;

fn requested_ids(hub: &MastHub, id: MastMissionId, dataset_ids: &[String]) -> Result<Vec<String>, MastCmdError> {
    if dataset_ids.is_empty() || dataset_ids.len() > PRODUCT_IDS_MAX {
        return Err(bad_input("datasetIds", &format!("send 1 to {PRODUCT_IDS_MAX} dataset ids")));
    }
    let mut seen = HashSet::new();
    let mut ids = Vec::with_capacity(dataset_ids.len());
    for dataset in dataset_ids {
        list_file(hub.cache_root(), id, dataset, ListKind::Products)?;
        if seen.insert(dataset.as_str()) {
            ids.push(dataset.clone());
        }
    }
    Ok(ids)
}

fn cached_products(hub: &MastHub, id: MastMissionId, dataset: &str) -> Option<MastProductList> {
    let key = (id, dataset.to_owned());
    let now = SystemTime::now();
    if let Some((list, expires)) = hub.product_lists().get(&key, Instant::now()) {
        if now < expires {
            return Some(list);
        }
    }
    let path = list_file(hub.cache_root(), id, dataset, ListKind::Products).ok()?;
    let (list, stored_at): (MastProductList, SystemTime) = read_stored_at(&path, LIST_TTL, now)?;
    if list.mission != id || list.dataset_ids != [dataset] {
        return None;
    }
    touch(&path);
    let stored: StoredProducts = (list.clone(), stored_at + LIST_TTL);
    hub.product_lists().insert(key, stored, Instant::now());
    Some(list)
}

fn store_products(hub: &MastHub, list: &MastProductList, stored_at: SystemTime) {
    let Some(dataset) = list.dataset_ids.first() else { return };
    hub.product_lists().insert((list.mission, dataset.clone()), (list.clone(), stored_at + LIST_TTL), Instant::now());
    if list.products.is_empty() {
        return;
    }
    let Ok(path) = list_file(hub.cache_root(), list.mission, dataset, ListKind::Products) else { return };
    match write_stored(&path, list, stored_at) {
        Ok(bytes) => note_list_write(hub, bytes),
        Err(error) => log::warn!("MAST product list was not cached: {error}"),
    }
}

fn stamped(mut list: MastProductList, started: Instant, from_cache: bool) -> MastProductList {
    list.elapsed_ms = elapsed_ms(started);
    list.from_cache = from_cache;
    list
}

pub async fn products(hub: &MastHub, request_id: &str, id: MastMissionId, dataset_ids: &[String], refresh: bool) -> Result<MastProductList, MastCmdError> {
    auth::screen_unauthorized(hub, run_products(hub, request_id, id, dataset_ids, refresh)).await
}

async fn run_products(hub: &MastHub, request_id: &str, id: MastMissionId, dataset_ids: &[String], refresh: bool) -> Result<MastProductList, MastCmdError> {
    let started = Instant::now();
    validate_request_id(request_id)?;
    let ids = requested_ids(hub, id, dataset_ids)?;
    let op = Operation::mission(id, OpKind::Products, "post_list_products");
    if !capabilities(hub, id).products {
        return Err(not_deployed(&op, "no product listing for this mission"));
    }
    let endpoint = hub.endpoint(id, OpKind::Products)?;
    let single = match ids.as_slice() {
        [dataset] => Some(dataset.as_str()),
        _ => None,
    };
    let cached = || single.filter(|_| !refresh).and_then(|dataset| cached_products(hub, id, dataset));
    if let Some(list) = cached() {
        return Ok(stamped(list, started, true));
    }
    let _permit = hub.product_permits().acquire().await;
    if let Some(list) = cached() {
        return Ok(stamped(list, started, true));
    }
    let mut listing = Vec::new();
    for chunk in ids.chunks(PRODUCT_CHUNK) {
        let sent = Instant::now();
        let page = endpoint
            .post_list_products::<DatasetProduct>(&DatasetIds::new(chunk.iter().cloned()))
            .await
            .map_err(|error| failure(hub, &error, &op, sent))?;
        listing.extend(page.products);
    }
    hub.limiter().on_success();
    let mut list = classify_products(id, &ids, listing);
    if single.is_some() {
        let stored_at = SystemTime::now();
        list.cached_until = Some(utc_datetime(stored_at + LIST_TTL));
        store_products(hub, &list, stored_at);
    }
    Ok(stamped(list, started, false))
}

pub async fn token_status(hub: &MastHub, request_id: &str, check: bool) -> Result<MastTokenStatus, MastCmdError> {
    validate_request_id(request_id)?;
    if check {
        auth::check(hub).await
    } else {
        auth::stored_status(hub).await
    }
}

pub async fn token_save(hub: &MastHub, request_id: &str, token: &str) -> Result<MastTokenStatus, MastCmdError> {
    validate_request_id(request_id)?;
    auth::save(hub, token).await
}

pub async fn token_clear(hub: &MastHub) -> Result<MastTokenStatus, MastCmdError> {
    auth::clear(hub).await
}

pub fn cache_info(hub: &MastHub) -> MastCacheInfo {
    dir_info(hub.cache_root())
}

pub fn cache_clear(hub: &MastHub, kind: MastCacheKind) -> Result<MastCacheInfo, MastCmdError> {
    let root = hub.cache_root();
    let (freed, error) = match kind {
        MastCacheKind::Images => remove_tree(&root.join(IMG_DIR)),
        MastCacheKind::Lists => remove_tree(&root.join(LISTS_DIR)),
        MastCacheKind::Names => match remove_file(&root.join(NAMES_FILE)) {
            Ok(freed) => (freed, None),
            Err(error) => (MastCacheInfo::default(), Some(error)),
        },
        MastCacheKind::All => remove_tree(root),
    };
    if matches!(kind, MastCacheKind::Images | MastCacheKind::All) {
        hub.forget_images();
        hub.image_budget().reset(dir_info(&root.join(IMG_DIR)).bytes);
    }
    if matches!(kind, MastCacheKind::Lists | MastCacheKind::All) {
        hub.image_sets().clear();
        hub.product_lists().clear();
        hub.list_budget().reset(dir_info(&root.join(LISTS_DIR)).bytes);
    }
    if matches!(kind, MastCacheKind::Names | MastCacheKind::All) {
        hub.names().clear_negative();
    }
    if kind == MastCacheKind::All {
        hub.searches().clear();
    }
    match error {
        Some(error) if freed.files == 0 => Err(cache_failure(&error)),
        Some(error) => {
            log::warn!("MAST cache was cleared in part: {error}");
            Ok(freed)
        }
        None => Ok(freed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::ids::validate_dataset_id;
    use crate::core::mast::missions::{
        CLASSY_PRODUCTS_OK, HST_DATASET_CONDITION_OK, IUE_PRODUCTS_OK, JWST_DATE_SORT_OK, JWST_FILTER_WILDCARD_OK, ULLYSES_PRODUCTS_OK,
    };
    use crate::core::mast::normalize::normalize_row;
    use crate::core::mast::types::MastProductClass;
    use crate::infra::mast::models::DatasetProducts;
    use crate::core::mast::query::{MAX_ROWS, PAGE_LIMIT};
    use crate::core::mast::types::{LevelFilter, MastFilterValues, MastWhere, ObjectCategory, ObjectType};
    use crate::infra::mast::api::MissionEndpoint;
    use crate::infra::mast::lookup::NAMES_FILE;
    use crate::infra::mast::models::{SearchParams, SearchResults};
    use crate::infra::mast::settings::{MastSettings, ENV_MAST_RETRY_MS, ENV_MAST_TIMEOUT_MS, ENV_MAST_URL};
    use crate::core::mast::types::{ProgressStage, WcsOrientation};
    use crate::infra::mast::testing::{form_value, serve_routes, synthetic_jpeg, Recorded, Route, TestServer};
    use serde_json::{json, Value};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name)).unwrap()
    }

    const RETRY_MS: u64 = 50;

    fn hub_with(server: &TestServer, dir: &Path, timeout_ms: u64, retry_ms: u64) -> Arc<MastHub> {
        let base = server.base.clone();
        let settings = MastSettings::from_lookup(move |name| match name {
            ENV_MAST_URL => Some(base.clone()),
            ENV_MAST_TIMEOUT_MS => Some(timeout_ms.to_string()),
            ENV_MAST_RETRY_MS => Some(retry_ms.to_string()),
            _ => None,
        });
        Arc::new(MastHub::new(settings, dir.to_path_buf()).unwrap())
    }

    fn hub_with_timeout(server: &TestServer, dir: &Path, timeout_ms: u64) -> Arc<MastHub> {
        hub_with(server, dir, timeout_ms, RETRY_MS)
    }

    fn hub_for(server: &TestServer, dir: &Path) -> Arc<MastHub> {
        hub_with_timeout(server, dir, 5000)
    }

    fn live_hub(dir: &Path) -> Arc<MastHub> {
        Arc::new(MastHub::new(MastSettings::default(), dir.to_path_buf()).unwrap())
    }

    fn m16(mission: MastMissionId) -> MastQuery {
        MastQuery {
            mission,
            where_: MastWhere::Position { ra_deg: 274.688, dec_deg: -13.792, radius_arcmin: 3.0, label: Some("M 16".into()) },
            program: None,
            dataset_id: None,
            filters: MastFilterValues::default(),
            preset: None,
            sort: Vec::new(),
            offset: 0,
            limit: PAGE_LIMIT,
        }
    }

    fn lookup_body(request: &crate::infra::mast::testing::Recorded) -> Value {
        serde_json::from_str(&form_value(&request.body, "request").unwrap()).unwrap()
    }

    async fn start_search(hub: &Arc<MastHub>, id: &str, query: MastQuery) -> tokio::task::JoinHandle<Result<MastSearchPage, MastCmdError>> {
        let hub = Arc::clone(hub);
        let id = id.to_owned();
        tokio::spawn(async move {
            let worker = Arc::clone(&hub);
            let request_id = id.clone();
            hub.registry().run(&id, async move { search(&worker, &request_id, &query, false).await }).await
        })
    }

    async fn wait_registered(hub: &MastHub, id: &str) {
        for _ in 0..400 {
            if hub.registry().contains(id) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("{id} never registered");
    }

    #[tokio::test]
    async fn lookup_hit_and_miss() {
        let server = serve_routes(vec![
            Route::post("/api/v0/invoke").when(|r| r.body.contains("M16")).json(fixture("lookup_m16.json")),
            Route::post("/api/v0/invoke").json(fixture("nl.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let hit = resolve(&hub, " M16 ").await.unwrap().unwrap();
        assert_eq!(hit.query, "M16");
        assert_eq!(hit.canonical_name, "M 16");
        assert_eq!(hit.resolver, "SIMBAD");
        assert_eq!((hit.ra_deg, hit.dec_deg), (274.688, -13.792));
        assert_eq!(hit.object_type.as_deref(), Some("OpenCluster"));
        assert!(!hit.from_cache);
        let miss = resolve(&hub, "Qzxv-Not-A-Star-77").await.unwrap();
        assert!(miss.is_none());
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        for request in &requests {
            assert_eq!(request.method, "POST");
            assert_eq!(request.path(), "/api/v0/invoke");
            assert_eq!(request.header("content-type"), Some("application/x-www-form-urlencoded"));
            assert_eq!(request.header("authorization"), None);
            assert_eq!(request.header("cookie"), None);
        }
        assert_eq!(
            lookup_body(&requests[0]),
            json!({"service": "Mast.Name.Lookup", "params": {"input": "M16", "format": "json"}, "format": "json"})
        );
        assert_eq!(lookup_body(&requests[1])["params"]["input"], json!("Qzxv-Not-A-Star-77"));
        let error = resolve(&hub, "  ").await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput);
        assert_eq!(server.requests().len(), 2);
    }

    #[tokio::test]
    async fn lookup_negative_cached_in_memory_positive_on_disk() {
        let server = serve_routes(vec![
            Route::post("/api/v0/invoke").when(|r| r.body.contains("M16")).json(fixture("lookup_m16.json")),
            Route::post("/api/v0/invoke").json(fixture("nl.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        assert!(!resolve(&hub, "M16").await.unwrap().unwrap().from_cache);
        assert!(resolve(&hub, "nowhere").await.unwrap().is_none());
        assert_eq!(server.requests().len(), 2);
        let again = resolve(&hub, "m16").await.unwrap().unwrap();
        assert!(again.from_cache);
        assert_eq!(again.query, "m16");
        assert_eq!(again.canonical_name, "M 16");
        assert!(resolve(&hub, "Nowhere").await.unwrap().is_none());
        assert_eq!(server.requests().len(), 2);
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join(NAMES_FILE)).unwrap()).unwrap();
        assert_eq!(stored["m16"]["canonicalName"], json!("M 16"));
        assert!(stored.get("nowhere").is_none());
        let restarted = hub_for(&server, dir.path());
        assert!(resolve(&restarted, "M16").await.unwrap().unwrap().from_cache);
        assert!(resolve(&restarted, "nowhere").await.unwrap().is_none());
        assert_eq!(server.requests().len(), 3);
    }

    #[tokio::test]
    async fn search_sends_the_normalised_request() {
        let server = serve_routes(vec![Route::post("/search/jwst/api/v0.1/search").json(fixture("jwst_m16_l3_optical.json"))]).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let mut query = m16(MastMissionId::Jwst);
        query.filters.level = Some(LevelFilter::Combined);
        let page = search(&hub, "g1:search", &query, false).await.unwrap();
        assert_eq!(page.request_id, "g1:search");
        assert_eq!(page.mission, MastMissionId::Jwst);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].filters.len(), 6);
        assert!(!page.from_cache);
        let request = &server.requests()[0];
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(request.header("authorization"), None);
        let sent: Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(sent, page.request_body);
        assert_eq!(sent["target"], json!(["274.688 -13.792"]));
        assert_eq!(sent["conditions"], json!([{"productLevel": "3"}]));
        assert_eq!(sent["limit"], json!(200));
        assert_eq!(sent["select_cols"].as_array().unwrap().len(), 19);
        assert_eq!(sent["select_cols"][16], json!("targtype"));
        assert_eq!(sent["select_cols"][18], json!("targdesc"));
        let bad = MastQuery { where_: MastWhere::None, ..m16(MastMissionId::Jwst) };
        assert_eq!(search(&hub, "g1:search", &bad, false).await.unwrap_err().code, MastErrorCode::BadInput);
        assert_eq!(search(&hub, "bad id", &query, false).await.unwrap_err().code, MastErrorCode::BadInput);
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn search_cache_hit_and_refresh_bypass() {
        let server = serve_routes(vec![Route::post("/search/jwst/api/v0.1/search").json(fixture("jwst_m16_l3.json"))]).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let query = m16(MastMissionId::Jwst);
        let first = search(&hub, "g1:search", &query, false).await.unwrap();
        assert!(!first.from_cache);
        let cached = search(&hub, "g2:search", &query, false).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.request_id, "g2:search");
        assert_eq!(cached.rows, first.rows);
        assert_eq!(server.count("/search/jwst"), 1);
        let refreshed = search(&hub, "g3:search", &query, true).await.unwrap();
        assert!(!refreshed.from_cache);
        assert_eq!(server.count("/search/jwst"), 2);
        assert!(search(&hub, "g4:search", &query, false).await.unwrap().from_cache);
        assert_eq!(server.count("/search/jwst"), 2);
        let next_page = MastQuery { offset: 200, ..query.clone() };
        assert!(!search(&hub, "g4:page:200", &next_page, false).await.unwrap().from_cache);
        let hst = m16(MastMissionId::Hst);
        let server_hst = serve_routes(vec![Route::post("/search/hst/").json(fixture("hst_m16.json"))]).await;
        let hub_hst = hub_for(&server_hst, dir.path());
        assert!(!search(&hub_hst, "g1:search", &hst, false).await.unwrap().from_cache);
        assert_eq!(server.count("/search/jwst"), 3);
    }

    #[tokio::test]
    async fn preset_searches_name_dropped_conditions_for_the_query_that_reads_the_page() {
        let mut silent: Value = serde_json::from_str(&fixture("jwst_m16_l3.json")).unwrap();
        silent["search_params"]["conditions"] = json!([]);
        let server = serve_routes(vec![Route::post("/search/jwst/api/v0.1/search").json(silent.to_string())]).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let mut plain = m16(MastMissionId::Jwst);
        plain.filters.level = Some(LevelFilter::Combined);
        let catalogues = MastQuery { preset: Some("pipeline_catalogs".into()), ..plain.clone() };
        let first = search(&hub, "g1:search", &catalogues, false).await.unwrap();
        assert!(!first.from_cache);
        assert_eq!(first.dropped_conditions, ["productLevel"]);
        let sent: Value = serde_json::from_str(&server.requests()[0].body).unwrap();
        assert_eq!(sent["conditions"], json!([{"productLevel": "3"}]));
        let reused = search(&hub, "g2:search", &plain, false).await.unwrap();
        assert!(reused.from_cache);
        assert_eq!(reused.dropped_conditions, Vec::<String>::new());
        let again = search(&hub, "g3:search", &catalogues, false).await.unwrap();
        assert!(again.from_cache);
        assert_eq!(again.dropped_conditions, ["productLevel"]);
        let tso = MastQuery { preset: Some("tso".into()), ..plain.clone() };
        let fresh = search(&hub, "g4:search", &tso, false).await.unwrap();
        assert!(!fresh.from_cache);
        assert_eq!(fresh.dropped_conditions, ["tsovisit", "exp_type"]);
        let sent: Value = serde_json::from_str(&server.requests()[1].body).unwrap();
        assert_eq!(
            sent["conditions"],
            json!([{"productLevel": "3"}, {"tsovisit": "t"}, {"exp_type": "NRS_BRIGHTOBJ,NIS_SOSS,MIR_LRS-SLITLESS,NRC_TSGRISM"}])
        );
        assert_eq!(server.count("/search/jwst"), 2);
    }

    fn ism_answer(copies: usize, total: usize) -> String {
        let mut answer: Value = serde_json::from_str(&fixture("jwst_m16_ism.json")).unwrap();
        let rows = answer["results"].as_array().unwrap().clone();
        let copied: Vec<Value> = (0..copies)
            .flat_map(|copy| {
                rows.iter().map(move |row| {
                    let mut row = row.clone();
                    if copy > 0 {
                        row["fileSetName"] = json!(format!("{}_{copy}", row["fileSetName"].as_str().unwrap()));
                    }
                    row
                })
            })
            .take(total)
            .collect();
        answer["totalResults"] = json!(copied.len());
        answer["results"] = Value::Array(copied);
        answer.to_string()
    }

    fn ism(level: LevelFilter, offset: i64) -> MastQuery {
        let mut query = m16(MastMissionId::Jwst);
        query.filters.level = Some(level);
        query.filters.categories = vec![ObjectCategory::Ism];
        query.offset = offset;
        query
    }

    fn body_of(request: &Recorded) -> Value {
        serde_json::from_str(&request.body).unwrap()
    }

    #[tokio::test]
    async fn local_level_service_path() {
        let server = serve_routes(vec![
            Route::post("/search/jwst/api/v0.1/search").when(|r| r.body.contains("\"productLevel\":")).json(fixture("jwst_m16_l3.json")),
            Route::post("/search/jwst/api/v0.1/search").json(ism_answer(4, usize::MAX)),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let first = search(&hub, "g1:search", &ism(LevelFilter::Exposures, 0), false).await.unwrap();
        assert_eq!(server.count("/search/jwst"), 1);
        let sent = body_of(&server.requests()[0]);
        assert_eq!(sent["conditions"], json!([{"targcat": "ISM"}]));
        assert_eq!((&sent["offset"], &sent["limit"]), (&json!(0), &json!(2000)));
        assert_eq!(sent["select_cols"].as_array().unwrap().len(), 19);
        assert_eq!(first.level_applied_locally.as_deref(), Some("productLevel in 1b, 2a, 2b, 2c"));
        assert!(!first.from_cache);
        assert_eq!(first.request_id, "g1:search");
        assert_eq!((first.rows.len(), first.total_reported, first.total_reliable, first.has_more), (200, Some(444), true, true));
        assert_eq!((first.offset, first.limit), (0, 200));
        assert!(first.rows.iter().all(|row| row.level.as_deref() == Some("1b, 2a, 2b, 2c")));
        assert_eq!(first.request_body, sent);
        assert_eq!(first.dropped_conditions, Vec::<String>::new());
        assert_eq!(first.unfiltered_total, None);
        let second = search(&hub, "g1:page:200", &ism(LevelFilter::Exposures, 200), false).await.unwrap();
        assert!(second.from_cache && second.level_applied_locally.is_some());
        assert_eq!((second.rows.len(), second.offset, second.has_more), (200, 200, true));
        assert_ne!(second.rows[0].dataset_id, first.rows[0].dataset_id);
        let last = search(&hub, "g1:page:400", &ism(LevelFilter::Exposures, 400), false).await.unwrap();
        assert_eq!((last.rows.len(), last.has_more), (44, false));
        let combined = search(&hub, "g2:search", &ism(LevelFilter::Combined, 0), false).await.unwrap();
        assert!(combined.from_cache && combined.level_applied_locally.is_some());
        assert_eq!((combined.rows.len(), combined.total_reported, combined.has_more), (8, Some(8), false));
        assert_eq!(server.count("/search/jwst"), 1, "later pages and the other level come from the one wide answer");
        assert_eq!(hub.row_filter(MastMissionId::Jwst, "jw02739-o002_t001_miri_3").as_deref(), Some("F770W"));
        let refreshed = search(&hub, "g3:search", &ism(LevelFilter::Combined, 0), true).await.unwrap();
        assert!(!refreshed.from_cache && refreshed.level_applied_locally.is_some());
        assert_eq!(server.count("/search/jwst"), 2);
        let all_levels = search(&hub, "g4:search", &ism(LevelFilter::All, 0), false).await.unwrap();
        assert_eq!(all_levels.level_applied_locally, None);
        assert_eq!(body_of(&server.requests()[2])["limit"], json!(200));
        let mut hst = m16(MastMissionId::Hst);
        hst.filters.level = Some(LevelFilter::Combined);
        hst.filters.categories = vec![ObjectCategory::Ism];
        let server_hst = serve_routes(vec![Route::post("/search/hst/").json(fixture("hst_m16_ism.json"))]).await;
        let hub_hst = hub_for(&server_hst, dir.path());
        let page = search(&hub_hst, "g1:search", &hst, false).await.unwrap();
        assert!(page.level_applied_locally.is_none());
        assert_eq!(body_of(&server_hst.requests()[0])["limit"], json!(200));
    }

    #[tokio::test]
    async fn a_full_wide_answer_falls_back_to_the_server_level() {
        let mut level_answer: Value = serde_json::from_str(&fixture("jwst_m16_l3.json")).unwrap();
        level_answer["search_params"]["conditions"] = json!([{"productLevel": "3"}, {"targcat": "ISM"}]);
        let server = serve_routes(vec![
            Route::post("/search/jwst/api/v0.1/search").when(|r| r.body.contains("\"productLevel\":")).json(level_answer.to_string()),
            Route::post("/search/jwst/api/v0.1/search").json(ism_answer(18, 2000)),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let page = search(&hub, "g1:search", &ism(LevelFilter::Combined, 0), false).await.unwrap();
        assert_eq!(page.level_applied_locally, None);
        assert_eq!(server.count("/search/jwst"), 2);
        let requests = server.requests();
        assert_eq!(body_of(&requests[0])["limit"], json!(2000));
        let fallback = body_of(&requests[1]);
        assert_eq!(fallback["conditions"], json!([{"productLevel": "3"}, {"targcat": "ISM"}]));
        assert_eq!(fallback["limit"], json!(200));
        assert_eq!(page.request_body, fallback);
        assert_eq!((page.rows.len(), page.total_reported), (2, Some(2)));
        assert_eq!(page.dropped_conditions, Vec::<String>::new());
        let again = search(&hub, "g2:search", &ism(LevelFilter::Combined, 0), false).await.unwrap();
        assert!(again.from_cache && again.level_applied_locally.is_none());
        assert_eq!(server.count("/search/jwst"), 2);
        let plan = local_level::plan(&validate(&ism(LevelFilter::Combined, 0)).unwrap()).unwrap();
        let wide_key = canonical_key(MastMissionId::Jwst, &build_params(&plan.wide));
        assert!(matches!(hub.searches().get(&wide_key, Instant::now()), Some(CachedSearch::FullWideAnswer)));
        assert_eq!(hub.searches().weight(), 3, "the rows of the full wide answer stayed in memory");
    }

    fn chip(category: ObjectCategory) -> MastQuery {
        let mut query = ism(LevelFilter::Combined, 0);
        query.filters.categories = vec![category];
        query
    }

    #[tokio::test]
    async fn wide_answers_share_a_row_budget_and_the_least_recent_is_dropped() {
        let server = serve_routes(vec![Route::post("/search/jwst/api/v0.1/search").json(ism_answer(18, MAX_ROWS as usize - 1))]).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let chips = [ObjectCategory::Ism, ObjectCategory::Galaxies, ObjectCategory::Stars, ObjectCategory::StarClusters];
        for (n, category) in chips.into_iter().enumerate() {
            let page = search(&hub, &format!("g{n}:search"), &chip(category), false).await.unwrap();
            assert!(page.level_applied_locally.is_some() && !page.from_cache, "{category:?}");
        }
        assert_eq!(server.count("/search/jwst"), 4);
        let again = search(&hub, "g5:search", &chip(ObjectCategory::Ism), false).await.unwrap();
        assert!(!again.from_cache, "four answers of 1,999 rows stayed in memory");
        assert_eq!(server.count("/search/jwst"), 5);
        let recent = search(&hub, "g6:search", &chip(ObjectCategory::StarClusters), false).await.unwrap();
        assert!(recent.from_cache);
        assert_eq!(server.count("/search/jwst"), 5);
    }

    #[tokio::test]
    async fn a_refreshed_search_reports_no_cached_unfiltered_total() {
        let server = serve_routes(vec![
            Route::post("/search/jwst/api/v0.1/search").when(|r| r.body.contains("targcat\":")).json(ism_answer(1, usize::MAX)),
            Route::post("/search/jwst/api/v0.1/search").json(fixture("jwst_m16_l3.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let mut plain = m16(MastMissionId::Jwst);
        plain.filters.level = Some(LevelFilter::Combined);
        assert_eq!(search(&hub, "g1:search", &plain, false).await.unwrap().total_reported, Some(2));
        let cached = search(&hub, "g2:search", &ism(LevelFilter::Combined, 0), false).await.unwrap();
        assert_eq!(cached.unfiltered_total, Some(2));
        let refreshed = search(&hub, "g3:search", &ism(LevelFilter::Combined, 0), true).await.unwrap();
        assert!(!refreshed.from_cache);
        assert_eq!(refreshed.unfiltered_total, None, "a refresh showed the plain total cached before it");
        assert_eq!(server.count("/search/jwst"), 3);
    }

    #[tokio::test]
    async fn unfiltered_totals_come_from_the_search_cache_only() {
        let server = serve_routes(vec![
            Route::post("/search/jwst/api/v0.1/search").when(|r| r.body.contains("targcat\":")).json(ism_answer(1, usize::MAX)),
            Route::post("/search/jwst/api/v0.1/search").json(fixture("jwst_m16_l3.json")),
            Route::post("/search/hst/").when(|r| r.body.contains("sci_broad_category\":")).json(fixture("hst_m16_ism.json")),
            Route::post("/search/hst/").json(fixture("hst_m16.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let cold = search(&hub, "g1:search", &ism(LevelFilter::Combined, 0), false).await.unwrap();
        assert!(cold.level_applied_locally.is_some());
        assert_eq!(cold.unfiltered_total, None);
        assert_eq!(server.count("/search/jwst"), 1);
        let mut plain = m16(MastMissionId::Jwst);
        plain.filters.level = Some(LevelFilter::Combined);
        let baseline = search(&hub, "g2:search", &plain, false).await.unwrap();
        assert_eq!((baseline.total_reported, baseline.total_reliable, baseline.unfiltered_total), (Some(2), true, None));
        assert_eq!(server.count("/search/jwst"), 2);
        let warm = search(&hub, "g3:search", &ism(LevelFilter::Combined, 0), false).await.unwrap();
        assert_eq!(warm.unfiltered_total, Some(2));
        assert_eq!(server.count("/search/jwst"), 2);
        let all_levels = search(&hub, "g4:search", &ism(LevelFilter::All, 0), false).await.unwrap();
        assert_eq!(all_levels.unfiltered_total, None);
        assert_eq!(server.count("/search/jwst"), 3);
        let everything = m16(MastMissionId::Jwst);
        assert_eq!(search(&hub, "g5:search", &everything, false).await.unwrap().total_reported, Some(2));
        let cached = search(&hub, "g6:search", &ism(LevelFilter::All, 0), false).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.unfiltered_total, Some(2));
        assert_eq!(server.count("/search/jwst"), 4);
        let mut hst = m16(MastMissionId::Hst);
        hst.filters.categories = vec![ObjectCategory::Ism];
        assert_eq!(search(&hub, "g7:search", &hst, false).await.unwrap().unfiltered_total, None);
        assert_eq!(search(&hub, "g8:search", &m16(MastMissionId::Hst), false).await.unwrap().total_reported, Some(265));
        let filtered = search(&hub, "g9:search", &hst, false).await.unwrap();
        assert!(filtered.from_cache);
        assert_eq!((filtered.unfiltered_total, filtered.total_reported), (Some(265), Some(37)));
        assert_eq!(server.count("/search/hst"), 2);
        let serialised = serde_json::to_value(&filtered).unwrap();
        assert_eq!((&serialised["unfilteredTotal"], &serialised["levelAppliedLocally"]), (&json!(265), &json!(null)));
    }

    #[tokio::test]
    async fn search_400_maps_to_rejected_with_detail() {
        let long_detail = "y".repeat(700);
        let server = serve_routes(vec![
            Route::post("/search/hst/").status(400).json(format!(r#"{{"detail":"{long_detail}"}}"#)),
            Route::post("/search/jwst/").status(400).json(fixture("radius60.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let mut query = m16(MastMissionId::Jwst);
        query.where_ = MastWhere::Position { ra_deg: 274.688, dec_deg: -13.792, radius_arcmin: 60.0, label: None };
        let error = search(&hub, "g1:search", &query, false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Rejected);
        assert_eq!(error.message, "Radius greater than 30 arcminutes is not supported");
        assert_eq!(error.context.status, Some(400));
        assert_eq!(error.context.detail.as_deref(), Some("Radius greater than 30 arcminutes is not supported"));
        assert_eq!(error.context.operation.as_deref(), Some("search/jwst/api/v0.1/search"));
        let sent: Value = serde_json::from_str(&server.requests()[0].body).unwrap();
        assert_eq!(sent["radius"], json!(30.0));
        let long = search(&hub, "g2:search", &m16(MastMissionId::Hst), false).await.unwrap_err();
        assert_eq!(long.code, MastErrorCode::Rejected);
        assert_eq!(long.context.detail.as_deref().map(str::len), Some(300));
        assert!(long.context.detail.as_deref().unwrap().chars().all(|c| c == 'y'));
    }

    #[tokio::test]
    async fn nginx_404_and_fastapi_404_are_not_deployed() {
        let server = serve_routes(vec![
            Route::post("/search/tess/").status(404).html(fixture("m_tess.json")),
            Route::post("/search/classy/").status(404).json(fixture("classy_il.json")),
            Route::post("/search/iue/").status(404).json(r#"{"detail":"Dataset gone"}"#),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let tess = search(&hub, "g1:search", &m16(MastMissionId::Tess), false).await.unwrap_err();
        assert_eq!(tess.code, MastErrorCode::NotDeployed);
        assert_eq!(tess.context.mission, Some(MastMissionId::Tess));
        assert_eq!(tess.context.status, Some(404));
        assert_eq!(tess.context.operation.as_deref(), Some("search/tess/api/v0.1/search"));
        let classy = search(&hub, "g2:search", &m16(MastMissionId::Classy), false).await.unwrap_err();
        assert_eq!(classy.code, MastErrorCode::NotDeployed);
        assert_eq!(classy.context.mission, Some(MastMissionId::Classy));
        let iue = search(&hub, "g3:search", &m16(MastMissionId::Iue), false).await.unwrap_err();
        assert_eq!(iue.code, MastErrorCode::Server);
    }

    #[tokio::test]
    async fn rate_limited_429_reads_retry_after() {
        let server = serve_routes(vec![
            Route::post("/search/jwst/").status(429).header("Retry-After", "5").times(2).json(r#"{"detail":"slow down"}"#),
            Route::post("/search/jwst/").json(fixture("jwst_m16_l3.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let query = m16(MastMissionId::Jwst);
        let error = search(&hub, "g1:search", &query, false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::RateLimited);
        assert_eq!(error.context.retry_after_sec, Some(5));
        assert_eq!(error.context.consecutive, Some(1));
        assert_eq!(error.context.status, Some(429));
        assert_eq!(error.context.operation.as_deref(), Some("search/jwst/api/v0.1/search"));
        let second = search(&hub, "g2:search", &query, false).await.unwrap_err();
        assert_eq!(second.context.consecutive, Some(2));
        assert_eq!(server.count("/search/jwst"), 2);
        assert!(search(&hub, "g3:search", &query, false).await.is_ok());
        assert_eq!(hub.limiter().consecutive(), 0);
    }

    #[tokio::test]
    async fn registry_cancel_by_id_and_prefix() {
        let server = serve_routes(vec![
            Route::post("/search/jwst/").delay(Duration::from_secs(20)).json(fixture("jwst_m16_l3.json")),
            Route::post("/search/hst/").delay(Duration::from_secs(20)).json(fixture("hst_m16.json")),
            Route::post("/api/v0/invoke").delay(Duration::from_secs(20)).json(fixture("lookup_m16.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let single = start_search(&hub, "g1:search", m16(MastMissionId::Jwst)).await;
        wait_registered(&hub, "g1:search").await;
        server.wait_for(|r| r.len() == 1).await;
        assert_eq!(hub.registry().cancel(Some(&["g1:search".to_owned()]), None).unwrap(), 1);
        let error = single.await.unwrap().unwrap_err();
        assert_eq!(error.code, MastErrorCode::Cancelled);
        assert_eq!(error.context, Default::default());
        let recorded = server.wait_for(|r| r.iter().any(|x| x.aborted)).await;
        assert!(recorded[0].aborted);
        assert!(!hub.registry().contains("g1:search"));
        let a = start_search(&hub, "g2:search", m16(MastMissionId::Jwst)).await;
        let b = start_search(&hub, "g2:page:200", MastQuery { offset: 200, ..m16(MastMissionId::Hst) }).await;
        let keep = {
            let worker = Arc::clone(&hub);
            let hub = Arc::clone(&hub);
            tokio::spawn(async move { hub.registry().run("g3:resolve:m16", async move { resolve(&worker, "M16").await }).await })
        };
        for id in ["g2:search", "g2:page:200", "g3:resolve:m16"] {
            wait_registered(&hub, id).await;
        }
        server.wait_for(|r| r.len() == 4).await;
        assert_eq!(hub.registry().cancel(None, Some("g2:")).unwrap(), 2);
        assert_eq!(a.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        assert_eq!(b.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        assert!(hub.registry().contains("g3:resolve:m16"));
        assert_eq!(hub.registry().cancel(Some(&["g3:resolve:m16".to_owned(), "g9:none".to_owned()]), Some("g3:")).unwrap(), 1);
        assert_eq!(keep.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        assert!(hub.registry().is_empty());
        let recorded = server.wait_for(|r| r.iter().filter(|x| x.aborted).count() == 4).await;
        assert_eq!(recorded.iter().filter(|x| x.aborted).count(), 4);
    }

    #[tokio::test]
    async fn duplicate_request_id_aborts_previous() {
        let server = serve_routes(vec![
            Route::post("/search/jwst/").delay(Duration::from_secs(20)).json(fixture("jwst_m16_l3.json")),
            Route::post("/search/hst/").json(fixture("hst_m16.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let first = start_search(&hub, "g1:search", m16(MastMissionId::Jwst)).await;
        wait_registered(&hub, "g1:search").await;
        server.wait_for(|r| r.len() == 1).await;
        let second = start_search(&hub, "g1:search", m16(MastMissionId::Hst)).await;
        assert_eq!(first.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        let page = second.await.unwrap().unwrap();
        assert_eq!(page.mission, MastMissionId::Hst);
        assert!(hub.registry().is_empty());
        let recorded = server.wait_for(|r| r.iter().any(|x| x.aborted)).await;
        assert!(recorded.iter().any(|x| x.aborted && x.path().starts_with("/search/jwst/")));
    }

    #[tokio::test]
    async fn probe_mission_classifies_404_401_200() {
        let server = serve_routes(vec![
            Route::post("/search/tess/").status(404).html(fixture("m_tess.json")),
            Route::post("/search/multi_mission/").status(404).json(r#"{"detail":"Not Found"}"#),
            Route::post("/search/roman_spectra/").status(401).json(r#"{"detail":"Only authorized users can make search API calls"}"#),
            Route::post("/search/jwst/").json(r#"{"totalResults": 3, "results": []}"#),
            Route::post("/search/hst/").close_without_answer(),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let today = utc_date(SystemTime::now());
        let tess = probe_mission(&hub, MastMissionId::Tess).await.unwrap();
        assert_eq!(tess.availability, MastAvailability::NotDeployed);
        assert_eq!(tess.checked_on, today);
        assert_eq!(probe_mission(&hub, MastMissionId::MultiMission).await.unwrap().availability, MastAvailability::NotDeployed);
        let spectra = probe_mission(&hub, MastMissionId::RomanSpectra).await.unwrap();
        assert_eq!(spectra.availability, MastAvailability::Token);
        let jwst = probe_mission(&hub, MastMissionId::Jwst).await.unwrap();
        assert_eq!(jwst.availability, MastAvailability::Open);
        assert!(jwst.capabilities.images);
        let offline = probe_mission(&hub, MastMissionId::Hst).await.unwrap_err();
        assert_eq!(offline.code, MastErrorCode::Network);
        let probe_request = server.requests().into_iter().find(|r| r.path().starts_with("/search/jwst/")).unwrap();
        let sent: Value = serde_json::from_str(&probe_request.body).unwrap();
        assert_eq!(sent, json!({"target": ["0 0"], "radius": 0.1, "radius_units": "arcminutes", "count_only": true}));
        let merged = missions(&hub);
        let find = |id: MastMissionId| merged.iter().find(|m| m.id == id).unwrap().clone();
        assert_eq!(find(MastMissionId::Tess).checked_on, today);
        assert_eq!(find(MastMissionId::RomanSpectra).availability, MastAvailability::Token);
        assert_eq!(find(MastMissionId::Hst).availability, MastAvailability::Open);
        assert_eq!(find(MastMissionId::Hst).checked_on, "2026-10-16");
        assert_eq!(merged.len(), 10);
    }

    #[tokio::test]
    async fn roman_without_token_fails_locally() {
        let server = serve_routes(vec![Route::post("/search/").json(fixture("jwst_m16_l3.json"))]).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        for mission in [MastMissionId::Roman, MastMissionId::RomanCgi] {
            let error = search(&hub, "g1:search", &m16(mission), false).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::Unauthorized);
            assert_eq!(error.context.mission, Some(mission));
            assert_eq!(error.context.status, None);
            let probed = probe_mission(&hub, mission).await.unwrap();
            assert_eq!(probed.availability, MastAvailability::Token);
        }
        assert!(server.requests().is_empty());
    }

    const L3: &str = "jw02739-o001_t001_nircam";
    const MIRI: &str = "jw02739-o002_t001_miri";
    const EXPOSURE: &str = "jw02739001002_02101_00001";
    const ASSOCIATION: &str = "ICK909030";

    fn fixture_bytes(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name)).unwrap()
    }

    fn list_route(mission: &str, dataset: &str, category: &str, body: String) -> Route {
        Route::get(&format!("/search/{mission}/api/v0.1/get_image_list?fileset_name={dataset}&category={category}")).json(body)
    }

    fn l3_list_routes(dataset: &str) -> Vec<Route> {
        ["THUMBNAIL", "PREVIEW"]
            .into_iter()
            .map(|category| list_route("jwst", dataset, category, fixture(&format!("il_{L3}_{category}.json")).replace(L3, dataset)))
            .collect()
    }

    fn hst_list_routes() -> Vec<Route> {
        ["THUMBNAIL", "PREVIEW"]
            .into_iter()
            .map(|category| list_route("hst", ASSOCIATION, category, fixture(&format!("il_{ASSOCIATION}_{category}.json"))))
            .collect()
    }

    fn files_under(dir: &Path) -> Vec<String> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    out.push(path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    fn dataset_folder(root: &Path, mission: &str, dataset: &str) -> std::path::PathBuf {
        root.join("img").join(mission).join(dataset)
    }

    fn assert_cache_path(path: &str, root: &Path) {
        assert!(Path::new(path).is_absolute(), "{path}");
        assert!(!path.starts_with(r"\\?\"), "{path}");
        assert!(Path::new(path).starts_with(root), "{path}");
        assert!(path.ends_with(".jpg") || path.ends_with(".png"), "{path}");
        assert!(Path::new(path).is_file(), "{path}");
    }

    fn collector() -> (Arc<std::sync::Mutex<Vec<MastProgressEvent>>>, impl Fn(MastProgressEvent) + Send + Sync) {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = {
            let events = Arc::clone(&events);
            move |event: MastProgressEvent| events.lock().unwrap().push(event)
        };
        (events, sink)
    }

    fn quiet(_event: MastProgressEvent) {}

    fn wcs_comment(crpix: [f64; 2], cd: f64) -> String {
        format!(
            "CTYPE1  = 'RA---TAN'\nCTYPE2  = 'DEC--TAN'\nCRPIX1  = {}\nCRPIX2  = {}\nCRVAL1  = 274.7\nCRVAL2  = -13.8\nCD1_1   = {}\nCD1_2   = 0\nCD2_1   = 0\nCD2_2   = {}\n",
            crpix[0], crpix[1], -cd, cd
        )
    }

    async fn settle(budget: &crate::infra::mast::cache::DiskBudget) {
        for _ in 0..400 {
            if !budget.busy() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("eviction never finished");
    }

    #[tokio::test]
    async fn image_list_is_cached_in_memory_and_on_disk() {
        let mut routes = l3_list_routes(L3);
        routes.push(list_route("jwst", MIRI, "THUMBNAIL", fixture(&format!("il_{L3}_THUMBNAIL.json")).replace(L3, MIRI)));
        routes.push(Route::get(&format!("/search/jwst/api/v0.1/get_image_list?fileset_name={MIRI}&category=PREVIEW")).status(500).html("<html>boom</html>"));
        routes.push(Route::post("/search/jwst/api/v0.1/search").json(fixture("jwst_m16_l3_optical.json")));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let set = image_list(&hub, MastMissionId::Jwst, L3).await.unwrap();
        assert_eq!(set.entries.len(), 6);
        assert_eq!(set.representative, Some(0));
        assert!(!set.from_cache);
        let requests = server.requests();
        let mut targets: Vec<&str> = requests.iter().map(|r| r.target.as_str()).collect();
        targets.sort();
        assert_eq!(
            targets,
            [
                "/search/jwst/api/v0.1/get_image_list?fileset_name=jw02739-o001_t001_nircam&category=PREVIEW",
                "/search/jwst/api/v0.1/get_image_list?fileset_name=jw02739-o001_t001_nircam&category=THUMBNAIL",
            ]
        );
        assert!(requests.iter().all(|r| r.header("authorization").is_none() && r.header("cookie").is_none()));
        let again = image_list(&hub, MastMissionId::Jwst, L3).await.unwrap();
        assert!(again.from_cache);
        assert_eq!(again.entries, set.entries);
        assert_eq!(server.requests().len(), 2);
        assert!(dir.path().join("lists").join("jwst").join(format!("{L3}.images.json")).is_file());

        let restarted = hub_for(&server, dir.path());
        search(&restarted, "g1:search", &m16(MastMissionId::Jwst), false).await.unwrap();
        let hinted = image_list(&restarted, MastMissionId::Jwst, L3).await.unwrap();
        assert!(hinted.from_cache);
        assert_eq!(hinted.representative, Some(1), "the row filter F187N picks the representative");
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_list"), 2);

        let failed = image_list(&hub, MastMissionId::Jwst, MIRI).await.unwrap_err();
        assert_eq!(failed.code, MastErrorCode::Server);
        assert_eq!(failed.context.operation.as_deref(), Some("search/jwst/api/v0.1/get_image_list"));
        assert!(!dir.path().join("lists").join("jwst").join(format!("{MIRI}.images.json")).exists());

        let before = server.requests().len();
        let iue = image_list(&hub, MastMissionId::Iue, "lwp12345").await.unwrap_err();
        assert_eq!(iue.code, MastErrorCode::NotDeployed);
        assert_eq!(iue.context.mission, Some(MastMissionId::Iue));
        assert_eq!(iue.context.operation.as_deref(), Some("search/iue/api/v0.1/get_image_list"));
        assert_eq!(iue.context.status, None);
        let roman = image_list(&hub, MastMissionId::Roman, "r0001").await.unwrap_err();
        assert_eq!(roman.code, MastErrorCode::Unauthorized);
        for bad in ["../x", "CON", ""] {
            assert_eq!(image_list(&hub, MastMissionId::Jwst, bad).await.unwrap_err().code, MastErrorCode::BadInput, "{bad}");
        }
        assert_eq!(server.requests().len(), before);
    }

    fn images_file(root: &Path, mission: &str, dataset: &str) -> std::path::PathBuf {
        root.join("lists").join(mission).join(format!("{dataset}.images.json"))
    }

    #[tokio::test]
    async fn blank_image_list_body_is_an_error_and_is_not_stored() {
        let server = serve_routes(vec![
            Route::get("/search/hst/api/v0.1/get_image_list?fileset_name=EMPTY1&category=THUMBNAIL").bytes(Vec::new()),
            Route::get("/search/hst/api/v0.1/get_image_list?fileset_name=EMPTY1&category=PREVIEW").bytes(Vec::new()),
            Route::get("/search/hst/api/v0.1/get_image_list?fileset_name=EMPTY2&category=").status(204),
            Route::get("/search/hst/api/v0.1/get_image_list?fileset_name=EMPTY3&category=").bytes(b" 
 ".to_vec()),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        for dataset in ["EMPTY1", "EMPTY2", "EMPTY3"] {
            let error = image_list(&hub, MastMissionId::Hst, dataset).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::Decode, "{dataset}");
            assert_eq!(error.message, "empty answer", "{dataset}");
            assert_eq!(error.context.operation.as_deref(), Some("search/hst/api/v0.1/get_image_list"), "{dataset}");
            assert!(!images_file(dir.path(), "hst", dataset).exists(), "{dataset}");
        }
        let card = thumbnail(&hub, "g1:thumb:e", MastMissionId::Hst, "EMPTY1", MastLevelClass::Exposure, None).await.unwrap_err();
        assert_eq!(card.code, MastErrorCode::Decode);
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_list?fileset_name=EMPTY1"), 4, "a blank answer is never cached");
        assert!(!dir.path().join("lists").exists());
    }

    #[tokio::test]
    async fn empty_image_lists_stay_in_memory() {
        let mut routes = vec![
            list_route("hst", "NONE1", "THUMBNAIL", "[]".into()),
            list_route("hst", "NONE1", "PREVIEW", "[]".into()),
            list_route("hst", "HALF1", "THUMBNAIL", fixture(&format!("il_{ASSOCIATION}_THUMBNAIL.json")).replace(ASSOCIATION, "HALF1")),
            list_route("hst", "HALF1", "PREVIEW", "[]".into()),
        ];
        routes.extend(hst_list_routes());
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let none = image_list(&hub, MastMissionId::Hst, "NONE1").await.unwrap();
        assert!(none.entries.is_empty());
        assert!(!none.from_cache);
        let half = image_list(&hub, MastMissionId::Hst, "HALF1").await.unwrap();
        assert!(!half.entries.is_empty());
        for dataset in ["NONE1", "HALF1"] {
            assert!(!images_file(dir.path(), "hst", dataset).exists(), "{dataset}: a set with an empty list stays off the disk");
            assert!(image_list(&hub, MastMissionId::Hst, dataset).await.unwrap().from_cache, "{dataset}: kept for the session");
        }
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_list"), 4);
        let full = image_list(&hub, MastMissionId::Hst, ASSOCIATION).await.unwrap();
        assert!(images_file(dir.path(), "hst", ASSOCIATION).is_file());

        let legacy = merge_lists(MastMissionId::Hst, "OLD1", &[], &[]);
        write_stored(&images_file(dir.path(), "hst", "OLD1"), &legacy, SystemTime::now()).unwrap();
        let restarted = hub_for(&server, dir.path());
        for dataset in ["NONE1", "HALF1"] {
            assert!(!image_list(&restarted, MastMissionId::Hst, dataset).await.unwrap().from_cache, "{dataset}");
        }
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_list"), 10);
        assert!(image_list(&restarted, MastMissionId::Hst, ASSOCIATION).await.unwrap().from_cache);
        assert_eq!(image_list(&restarted, MastMissionId::Hst, ASSOCIATION).await.unwrap().entries, full.entries);
        let old = image_list(&restarted, MastMissionId::Hst, "OLD1").await.unwrap_err();
        assert_eq!(old.code, MastErrorCode::Server, "an empty set an older build stored is fetched again");
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_list?fileset_name=OLD1"), 4);
    }

    #[tokio::test]
    async fn pane_list_is_not_queued_behind_card_lists() {
        let wave = Duration::from_millis(500);
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let cards = ["jw02739001002_02101_00002", "jw02739001002_02101_00003", "jw02739001002_02101_00004", "jw02739001002_02101_00005"];
        let pane = "jw02739001002_02101_00006";
        let mut routes = vec![Route::get("/search/jwst/api/v0.1/get_image_thumbnail?filename=").bytes(thumb.clone())];
        for dataset in cards.iter().chain([&pane]) {
            for category in ["THUMBNAIL", "PREVIEW"] {
                routes.push(list_route("jwst", dataset, category, fixture(&format!("il_{EXPOSURE}_{category}.json"))).delay(wave));
            }
        }
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let started: Vec<_> = cards
            .iter()
            .map(|dataset| {
                let (hub, dataset) = (Arc::clone(&hub), dataset.to_string());
                tokio::spawn(async move { thumbnail(&hub, "g1:thumb:x", MastMissionId::Jwst, &dataset, MastLevelClass::Exposure, None).await })
            })
            .collect();
        server.wait_for(|r| r.iter().filter(|x| x.target.contains("get_image_list")).count() >= 2).await;
        let set = image_list(&hub, MastMissionId::Jwst, pane).await.unwrap();
        assert_eq!(set.entries.len(), 60);
        let lists: Vec<String> = server.requests().iter().filter(|r| r.target.contains("get_image_list")).map(|r| r.target.clone()).collect();
        let last_pane = lists.iter().rposition(|t| t.contains(pane)).unwrap();
        assert!(last_pane <= 5, "the pane list went out behind {last_pane} card lists: {lists:#?}");
        for card in started {
            assert!(card.await.unwrap().is_ok());
        }
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_list"), 10);
    }

    #[tokio::test]
    async fn representative_jwst_l3_uses_fileset_only() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let mut routes = vec![Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={L3}"))
            .header("Content-Type", "application/octet-stream")
            .bytes(thumb.clone())];
        routes.extend(l3_list_routes(L3));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let first = thumbnail(&hub, "g1:thumb:jw02739-o001_t001_nircam", MastMissionId::Jwst, L3, MastLevelClass::Combined, None)
            .await
            .unwrap();
        assert_eq!(first.request_id, "g1:thumb:jw02739-o001_t001_nircam");
        assert_cache_path(&first.path, dir.path());
        assert!(Path::new(&first.path).starts_with(dataset_folder(dir.path(), "jwst", L3)));
        assert!(first.path.ends_with("_representative.jpg"), "{}", first.path);
        assert_eq!(first.filename, "_representative.jpg");
        assert_eq!((first.width, first.height, first.bytes), (128, 76, thumb.len() as u64));
        assert_eq!(first.wcs.as_ref().unwrap().orientation, WcsOrientation::Unreliable);
        assert_eq!(first.image_count, None);
        assert!(!first.from_cache);
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].target, "/search/jwst/api/v0.1/get_image_thumbnail?fileset_name=jw02739-o001_t001_nircam");
        assert_eq!(requests[0].header("authorization"), None);
        assert_eq!(files_under(&dataset_folder(dir.path(), "jwst", L3)), ["_representative.jpg"]);
        let cached = thumbnail(&hub, "g2:thumb:x", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.path, first.path);
        assert_eq!(cached.request_id, "g2:thumb:x");
        assert_eq!(server.requests().len(), 1);
        image_list(&hub, MastMissionId::Jwst, L3).await.unwrap();
        let counted = thumbnail(&hub, "g3:thumb:x", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        assert!(counted.from_cache);
        assert_eq!(counted.image_count, Some(6));
        assert_eq!(server.requests().len(), 3);
    }

    #[tokio::test]
    async fn representative_falls_back_to_list_on_401_without_remembering() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let mut routes = vec![
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name=")
                .status(401)
                .json(r#"{"detail":"UNAUTHORIZED USER: anonymous"}"#),
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?filename=").bytes(thumb.clone()),
        ];
        routes.extend(l3_list_routes(L3));
        routes.extend(l3_list_routes(MIRI));
        routes.push(list_route("jwst", "jw02739-o003_t002_nircam", "THUMBNAIL", "[]".into()));
        routes.push(list_route("jwst", "jw02739-o003_t002_nircam", "PREVIEW", "[]".into()));
        for category in ["THUMBNAIL", "PREVIEW"] {
            routes.push(list_route("jwst", EXPOSURE, category, fixture(&format!("il_{EXPOSURE}_{category}.json"))));
        }
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let first = thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        assert_eq!(first.filename, "jw02739-o001_t001_nircam_clear-f090w_i2d_thumb.jpg");
        assert!(first.path.ends_with("jw02739-o001_t001_nircam_clear-f090w_i2d_thumb.jpg"), "{}", first.path);
        assert_cache_path(&first.path, dir.path());
        assert_eq!(first.image_count, Some(6));
        assert!(!hub.fileset_failed(MastMissionId::Jwst, MastLevelClass::Combined), "a 401 is about this dataset, not the level class");
        let named = server.requests().into_iter().find(|r| r.target.contains("?filename=")).unwrap();
        assert_eq!(
            named.target,
            "/search/jwst/api/v0.1/get_image_thumbnail?filename=jw02739-o001_t001_nircam_clear-f090w_i2d_thumb.jpg&fileset_name=jw02739-o001_t001_nircam"
        );
        let row: FieldMap = serde_json::from_value(json!({"fileSetName": MIRI, "productLevel": "3", "filter": "F187N"})).unwrap();
        hub.remember_row_filters(MastMissionId::Jwst, &[crate::core::mast::normalize::normalize_row(MastMissionId::Jwst, &row, 0)]);
        let second = thumbnail(&hub, "g1:thumb:b", MastMissionId::Jwst, MIRI, MastLevelClass::Combined, None).await.unwrap();
        assert_eq!(second.filename, "jw02739-o002_t001_miri_clear-f187n_i2d_thumb.jpg", "the list path follows the row filter");
        assert_eq!(second.image_count, Some(6));
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name="), 2, "each dataset tries the fileset-only call");
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_list"), 4);
        let empty = thumbnail(&hub, "g1:thumb:c", MastMissionId::Jwst, "jw02739-o003_t002_nircam", MastLevelClass::Combined, None)
            .await
            .unwrap_err();
        assert_eq!(empty.code, MastErrorCode::NoImage);
        let exposure = thumbnail(&hub, "g1:thumb:d", MastMissionId::Jwst, EXPOSURE, MastLevelClass::Exposure, None).await.unwrap();
        assert_eq!(exposure.filename, "jw02739001002_02101_00001_nrcalong_cal_thumb.jpg");
        assert_eq!(exposure.image_count, Some(60));
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name="), 3);
    }

    #[tokio::test]
    async fn representative_falls_back_to_the_list_after_a_fileset_5xx() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let fileset = format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={L3}");
        let mut routes = vec![
            Route::get(&fileset).status(500).header("Content-Type", "text/plain; charset=utf-8").bytes(b"Internal Server Error".to_vec()),
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?filename=").bytes(thumb.clone()),
        ];
        routes.extend(l3_list_routes(L3));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let found = thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        assert_eq!(found.filename, "jw02739-o001_t001_nircam_clear-f090w_i2d_thumb.jpg");
        assert_eq!(found.image_count, Some(6));
        assert!(!found.from_cache);
        let kinds: Vec<&str> = server
            .requests()
            .iter()
            .map(|r| if r.target.starts_with(&fileset) { "fileset" } else if r.target.contains("get_image_list") { "list" } else { "named" })
            .collect();
        assert_eq!(kinds, ["fileset", "fileset", "list", "list", "named"]);
        assert!(!hub.fileset_failed(MastMissionId::Jwst, MastLevelClass::Combined), "a 500 is about this request, not the level class");
    }

    #[tokio::test]
    async fn a_fileset_timeout_is_shown_without_a_retry_or_a_list_fallback() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let fileset = format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={L3}");
        let mut routes = vec![
            Route::get(&fileset).delay(Duration::from_secs(3)).bytes(thumb.clone()),
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?filename=").bytes(thumb.clone()),
        ];
        routes.extend(l3_list_routes(L3));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_with_timeout(&server, dir.path(), 300);
        let failed = thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap_err();
        assert_eq!(failed.code, MastErrorCode::Timeout);
        assert_eq!(failed.context.timeout_sec, Some(1));
        assert_eq!(failed.context.attempts, Some(1));
        assert_eq!(failed.context.operation.as_deref(), Some("search/jwst/api/v0.1/get_image_thumbnail"));
        assert_eq!(server.count(&fileset), 1);
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_list"), 0);
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_thumbnail?filename="), 0);
    }

    #[tokio::test]
    async fn a_5xx_outage_stops_retries_and_list_fallbacks_for_the_next_cards() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let fileset = "/search/jwst/api/v0.1/get_image_thumbnail?fileset_name=";
        let lists = "/search/jwst/api/v0.1/get_image_list";
        let server = serve_routes(vec![
            Route::get(fileset).times(3).status(500).header("Content-Type", "text/plain; charset=utf-8").bytes(b"Internal Server Error".to_vec()),
            Route::get(fileset).bytes(thumb.clone()),
            Route::get(lists).status(500).json(r#"{"detail":"image index down"}"#),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let first = thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap_err();
        assert_eq!(first.code, MastErrorCode::Server);
        assert_eq!((server.count(fileset), server.count(lists)), (2, 2), "the first card retries once and tries the lists once");

        let second = thumbnail(&hub, "g1:thumb:b", MastMissionId::Jwst, MIRI, MastLevelClass::Combined, None).await.unwrap_err();
        assert_eq!(second.code, MastErrorCode::Server);
        assert_eq!(second.context.attempts, Some(1));
        assert_eq!((server.count(fileset), server.count(lists)), (3, 2), "during the outage a card costs one request");

        let recovered = thumbnail(&hub, "g1:thumb:c", MastMissionId::Jwst, "jw02739-o003_t002_nircam", MastLevelClass::Combined, None).await.unwrap();
        assert!(!recovered.from_cache);
        assert!(!hub.limiter().server_trouble(Instant::now()), "an answer ends the outage");
    }

    #[tokio::test]
    async fn a_failed_list_fallback_reports_the_fileset_error() {
        let server = serve_routes(vec![
            Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={L3}")).status(500).json(r#"{"detail":"fileset index down"}"#),
            Route::get("/search/jwst/api/v0.1/get_image_list").status(502).json(r#"{"detail":"list index down"}"#),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let failed = thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap_err();
        assert_eq!(failed.code, MastErrorCode::Server);
        assert_eq!(failed.context.status, Some(500));
        assert_eq!(failed.context.detail.as_deref(), Some("fileset index down"));
        assert_eq!(failed.context.operation.as_deref(), Some("search/jwst/api/v0.1/get_image_thumbnail"));
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_list"), 2, "the list fallback still ran");
    }

    #[tokio::test]
    async fn card_thumbnail_survives_a_failed_preview_list() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let server = serve_routes(vec![
            list_route("jwst", EXPOSURE, "THUMBNAIL", fixture(&format!("il_{EXPOSURE}_THUMBNAIL.json"))),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_list?fileset_name={EXPOSURE}&category=PREVIEW"))
                .status(500)
                .json(r#"{"detail":"preview index unavailable"}"#),
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?filename=").bytes(thumb.clone()),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let card = thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, EXPOSURE, MastLevelClass::Exposure, None).await.unwrap();
        assert_eq!(card.filename, "jw02739001002_02101_00001_nrcalong_cal_thumb.jpg");
        assert_eq!(card.image_count, Some(60));
        let strip_name = "jw02739001002_02101_00001_nrcb1_cal_thumb.jpg";
        let strip = thumbnail(&hub, "g1:thumb:b", MastMissionId::Jwst, EXPOSURE, MastLevelClass::Exposure, Some(strip_name)).await.unwrap();
        assert_eq!(strip.filename, strip_name);
        let pane = image_list(&hub, MastMissionId::Jwst, EXPOSURE).await.unwrap_err();
        assert_eq!(pane.code, MastErrorCode::Server);
        assert_eq!(pane.context.status, Some(500));
        assert_eq!(pane.context.detail.as_deref(), Some("preview index unavailable"));
        assert!(!images_file(dir.path(), "jwst", EXPOSURE).exists());
        assert!(hub.image_sets().is_empty(), "a set missing its preview list is not kept");
    }

    #[tokio::test]
    async fn hst_redirect_thumbnail_is_served_from_disk_after_restart() {
        let thumb = fixture_bytes("thumb_hst.bin");
        let server = serve_routes(vec![
            Route::get("/search/hst/api/v0.1/get_image_thumbnail?fileset_name=IDNM20010")
                .status(307)
                .header("Location", "/s3/hst/public/idnm/idnm20010/idnm20010_drz_thumb.jpg"),
            Route::get("/s3/hst/public/idnm/idnm20010/idnm20010_drz_thumb.jpg").header("Content-Type", "image/jpeg").bytes(thumb.clone()),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let first = thumbnail(&hub, "g1:thumb:IDNM20010", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap();
        assert_eq!(first.filename, "idnm20010_drz_thumb.jpg");
        assert!(!first.from_cache);
        let before = server.requests().len();
        assert_eq!(before, 2);

        let restarted = hub_for(&server, dir.path());
        let again = thumbnail(&restarted, "g3:thumb:IDNM20010", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap();
        assert!(again.from_cache);
        assert_eq!(again.filename, "idnm20010_drz_thumb.jpg");
        assert_eq!(again.path, first.path);
        assert_eq!(server.requests().len(), before, "the redirect name survives a restart");

        let folder = dataset_folder(dir.path(), "hst", "IDNM20010");
        std::fs::write(folder.join("_representative.name"), "../escape_thumb.jpg").unwrap();
        let tampered = hub_for(&server, dir.path());
        let refetched = thumbnail(&tampered, "g4:thumb:IDNM20010", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap();
        assert!(!refetched.from_cache, "an unusable name is ignored");
        assert_eq!(std::fs::read_to_string(folder.join("_representative.name")).unwrap(), "idnm20010_drz_thumb.jpg");

        std::fs::remove_file(folder.join("idnm20010_drz_thumb.jpg")).unwrap();
        let evicted = hub_for(&server, dir.path());
        let downloaded = thumbnail(&evicted, "g5:thumb:IDNM20010", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap();
        assert!(!downloaded.from_cache, "a name without its file falls through to MAST");
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_thumbnail?fileset_name=IDNM20010"), 3);
    }

    #[tokio::test]
    async fn serving_a_kept_representative_name_keeps_it_fresh_for_eviction() {
        let thumb = fixture_bytes("thumb_hst.bin");
        let server = serve_routes(vec![
            Route::get("/search/hst/api/v0.1/get_image_thumbnail?fileset_name=IDNM20010")
                .status(307)
                .header("Location", "/s3/hst/public/idnm/idnm20010/idnm20010_drz_thumb.jpg"),
            Route::get("/s3/hst/public/idnm/idnm20010/idnm20010_drz_thumb.jpg").header("Content-Type", "image/jpeg").bytes(thumb.clone()),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        thumbnail(&hub, "g1:thumb:IDNM20010", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap();
        let sidecar = dataset_folder(dir.path(), "hst", "IDNM20010").join("_representative.name");
        let stale = SystemTime::now() - Duration::from_secs(3 * 24 * 3600);
        std::fs::File::options().write(true).open(&sidecar).unwrap().set_modified(stale).unwrap();
        let restarted = hub_for(&server, dir.path());
        let again = thumbnail(&restarted, "g2:thumb:IDNM20010", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap();
        assert!(again.from_cache);
        let modified = std::fs::metadata(&sidecar).unwrap().modified().unwrap();
        assert!(modified > stale + Duration::from_secs(24 * 3600), "the name file keeps its old age and is evicted before its image");
    }

    #[tokio::test]
    async fn hst_redirect_keeps_the_raw_thumbnail_when_the_list_fails() {
        let thumb = fixture_bytes("thumb_hst.bin");
        let server = serve_routes(vec![
            Route::get(&format!("/search/hst/api/v0.1/get_image_thumbnail?fileset_name={ASSOCIATION}"))
                .status(307)
                .header("Location", "/s3/hst/public/ick9/ick909c1q/ick909c1q_raw_thumb.jpg"),
            Route::get("/s3/hst/public/ick9/ick909c1q/ick909c1q_raw_thumb.jpg").header("Content-Type", "image/jpeg").bytes(thumb.clone()),
            Route::get("/search/hst/api/v0.1/get_image_list").status(502).html("<html>bad gateway</html>"),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let kept = thumbnail(&hub, "g1:thumb:ICK909030", MastMissionId::Hst, ASSOCIATION, MastLevelClass::Unknown, None).await.unwrap();
        assert_eq!(kept.filename, "ick909c1q_raw_thumb.jpg");
        assert_eq!((kept.width, kept.height), (117, 128));
        assert!(!kept.from_cache);
        assert_cache_path(&kept.path, dir.path());
        assert_eq!(hub.chosen_representative(MastMissionId::Hst, ASSOCIATION), None, "an exposure thumbnail is a stand-in, not the representative");
        assert_eq!(files_under(&dataset_folder(dir.path(), "hst", ASSOCIATION)), ["ick909c1q_raw_thumb.jpg"]);
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_list"), 4);
    }

    #[tokio::test]
    async fn representative_remembers_a_non_image_fileset_answer() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let mut routes = vec![
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name=").html("<html>not an image</html>"),
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?filename=").bytes(thumb.clone()),
        ];
        routes.extend(l3_list_routes(L3));
        routes.extend(l3_list_routes(MIRI));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        assert!(hub.fileset_failed(MastMissionId::Jwst, MastLevelClass::Combined));
        assert!(!hub.fileset_failed(MastMissionId::Hst, MastLevelClass::Combined));
        thumbnail(&hub, "g1:thumb:b", MastMissionId::Jwst, MIRI, MastLevelClass::Combined, None).await.unwrap();
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name="), 1, "the failure is remembered");
    }

    #[tokio::test]
    async fn representative_hst_raw_redirect_uses_list() {
        let thumb = fixture_bytes("thumb_hst.bin");
        let mut routes = vec![
            Route::get("/search/hst/api/v0.1/get_image_thumbnail?fileset_name=ICK909030")
                .status(307)
                .header("Location", "/s3/hst/public/ick9/ick909c1q/ick909c1q_raw_thumb.jpg"),
            Route::get("/s3/hst/public/ick9/ick909c1q/ick909c1q_raw_thumb.jpg").header("Content-Type", "image/jpeg").bytes(thumb.clone()),
            Route::get("/search/hst/api/v0.1/get_image_thumbnail?fileset_name=IDNM20010")
                .status(307)
                .header("Location", "/s3/hst/public/idnm/idnm20010/idnm20010_drz_thumb.jpg"),
            Route::get("/s3/hst/public/idnm/idnm20010/idnm20010_drz_thumb.jpg").header("Content-Type", "image/jpeg").bytes(thumb.clone()),
            Route::get("/search/hst/api/v0.1/get_image_thumbnail?filename=ick909030_drc_thumb.jpg").bytes(thumb.clone()),
        ];
        routes.extend(hst_list_routes());
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let rep = thumbnail(&hub, "g1:thumb:ICK909030", MastMissionId::Hst, ASSOCIATION, MastLevelClass::Unknown, None).await.unwrap();
        assert_eq!(rep.filename, "ick909030_drc_thumb.jpg");
        assert_eq!(rep.image_count, Some(17));
        assert_eq!((rep.width, rep.height), (117, 128));
        assert_eq!(rep.wcs.as_ref().unwrap().orientation, WcsOrientation::CdMatrix);
        assert_cache_path(&rep.path, dir.path());
        assert_eq!(
            files_under(&dataset_folder(dir.path(), "hst", ASSOCIATION)),
            ["ick909030_drc_thumb.jpg", "ick909c1q_raw_thumb.jpg"]
        );
        let s3 = server.requests().into_iter().find(|r| r.path().starts_with("/s3/")).unwrap();
        assert_eq!(s3.header("authorization"), None);
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_list?fileset_name=ICK909030"), 2);
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_thumbnail?filename=ick909030_drc_thumb.jpg"), 1);
        assert!(!hub.fileset_failed(MastMissionId::Hst, MastLevelClass::Unknown));

        let drz = thumbnail(&hub, "g1:thumb:IDNM20010", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap();
        assert_eq!(drz.filename, "idnm20010_drz_thumb.jpg");
        assert!(drz.path.ends_with("idnm20010_drz_thumb.jpg"), "{}", drz.path);
        assert_eq!(drz.image_count, None);
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_list?fileset_name=IDNM20010"), 0);
        assert_eq!(files_under(&dataset_folder(dir.path(), "hst", "IDNM20010")), ["_representative.name", "idnm20010_drz_thumb.jpg"]);
        let before = server.requests().len();
        let cached = thumbnail(&hub, "g2:thumb:IDNM20010", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.path, drz.path);
        let again = thumbnail(&hub, "g2:thumb:ICK909030", MastMissionId::Hst, ASSOCIATION, MastLevelClass::Unknown, None).await.unwrap();
        assert!(again.from_cache);
        assert_eq!(again.filename, "ick909030_drc_thumb.jpg");
        assert_eq!(server.requests().len(), before);
    }

    const NGINX_504: &str = "<html><head><title>504 Gateway Time-out</title></head><body><center><h1>504 Gateway Time-out</h1></center></body></html>";

    #[tokio::test]
    async fn thumbnails_and_image_lists_retry_once_after_a_5xx_or_a_dropped_connection() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let broken = "jw02739-o001_t001_nircam_clear-f187n_i2d_thumb.jpg";
        let slow = "jw02739-o001_t001_nircam_clear-f200w_i2d_thumb.jpg";
        let dropped = "jw02739-o001_t001_nircam_clear-f335m_i2d_thumb.jpg";
        let fileset = format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={L3}");
        let mut routes = vec![
            Route::get(&fileset).status(500).times(1).html("<html>busy</html>"),
            Route::get(&fileset).bytes(thumb.clone()),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={broken}")).status(502).json(r#"{"detail":"Bad Gateway"}"#),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={slow}")).delay(Duration::from_secs(3)).bytes(thumb.clone()),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={dropped}")).times(1).close_without_answer(),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={dropped}")).bytes(thumb.clone()),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_list?fileset_name={MIRI}&category=THUMBNAIL")).status(504).times(1).html(NGINX_504),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_preview?filename=jw02739-o001_t001_nircam_clear-f090w_i2d.jpg")).status(500).html("<html>x</html>"),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename=jw02739-o001_t001_nircam_clear-f444w_i2d_thumb.jpg"))
                .status(404)
                .json(r#"{"detail":"Image not found"}"#),
        ];
        routes.extend(l3_list_routes(L3));
        routes.extend(l3_list_routes(MIRI));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());

        let recovered = thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        assert!(!recovered.from_cache);
        assert_eq!(server.count(&fileset), 2, "one retry after the 500");
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_list"), 0);

        let listed = image_list(&hub, MastMissionId::Jwst, MIRI).await.unwrap();
        assert_eq!(listed.entries.len(), 6);
        assert_eq!(server.count(&format!("/search/jwst/api/v0.1/get_image_list?fileset_name={MIRI}&category=THUMBNAIL")), 2, "one retry after the 504");

        let started = Instant::now();
        let failed = thumbnail(&hub, "g1:thumb:b", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(broken)).await.unwrap_err();
        assert_eq!(failed.code, MastErrorCode::Server);
        assert_eq!(failed.context.status, Some(502));
        assert_eq!(failed.context.detail.as_deref(), Some("Bad Gateway"));
        assert_eq!(failed.context.attempts, Some(2));
        let elapsed = failed.context.elapsed_ms.expect("elapsedMs on a retried failure");
        assert!(elapsed >= RETRY_MS && elapsed <= elapsed_ms(started), "{elapsed}");
        assert_eq!(server.count(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={broken}")), 2);
        let value = serde_json::to_value(&failed).unwrap();
        assert_eq!(value["context"]["attempts"], json!(2));
        assert_eq!(value["context"]["elapsedMs"], json!(elapsed));

        let missing = thumbnail(&hub, "g1:thumb:c", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some("jw02739-o001_t001_nircam_clear-f444w_i2d_thumb.jpg"))
            .await
            .unwrap_err();
        assert_eq!(missing.code, MastErrorCode::NoImage);
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_thumbnail?filename=jw02739-o001_t001_nircam_clear-f444w"), 1, "a 404 is an answer, not a hiccup");

        let pane = preview(&hub, "g1:preview:a", MastMissionId::Jwst, L3, "jw02739-o001_t001_nircam_clear-f090w_i2d.jpg", PreviewSize::Pane, &quiet).await.unwrap_err();
        assert_eq!(pane.code, MastErrorCode::Server);
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_preview"), 1, "previews are not retried");

        let calm = hub_for(&server, dir.path());
        let reconnected = thumbnail(&calm, "g1:thumb:e", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(dropped)).await.unwrap();
        assert_eq!(reconnected.filename, dropped);
        assert_eq!(server.count(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={dropped}")), 2, "one retry after a dropped connection");

        let impatient = hub_with_timeout(&server, dir.path(), 300);
        let timed_out = thumbnail(&impatient, "g1:thumb:d", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(slow)).await.unwrap_err();
        assert_eq!(timed_out.code, MastErrorCode::Timeout);
        assert_eq!(timed_out.context.timeout_sec, Some(1));
        assert_eq!(timed_out.context.attempts, Some(1), "a timeout is shown at once, not retried");
        let slow_requests = server.wait_for(|r| r.iter().any(|x| x.target.contains(slow) && x.aborted)).await;
        assert_eq!(slow_requests.iter().filter(|x| x.target.contains(slow)).count(), 1);
    }

    #[tokio::test]
    async fn a_retry_is_dropped_when_a_rate_limit_opens_during_its_backoff() {
        let broken = "jw02739-o001_t001_nircam_clear-f187n_i2d_thumb.jpg";
        let limited = "jw02739-o001_t001_nircam_clear-f200w_i2d_thumb.jpg";
        let mut routes = vec![
            Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={broken}")).status(500).json(r#"{"detail":"busy"}"#),
            Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={limited}"))
                .status(429)
                .header("Retry-After", "5")
                .json(r#"{"detail":"slow down"}"#),
        ];
        routes.extend(l3_list_routes(L3));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_with(&server, dir.path(), 5000, 1500);
        image_list(&hub, MastMissionId::Jwst, L3).await.unwrap();
        let first = {
            let hub = Arc::clone(&hub);
            tokio::spawn(async move { thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(broken)).await })
        };
        server.wait_for(|r| r.iter().any(|x| x.target.contains(broken))).await;
        let paused = thumbnail(&hub, "g1:thumb:b", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(limited)).await.unwrap_err();
        assert_eq!(paused.code, MastErrorCode::RateLimited);
        let failed = first.await.unwrap().unwrap_err();
        assert_eq!(failed.code, MastErrorCode::Server);
        assert_eq!(failed.context.attempts, Some(1));
        assert_eq!(server.count(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={broken}")), 1, "no retry into an open rate-limit window");
    }

    #[test]
    fn retry_backoff_spreads_retries_over_a_quarter_of_the_wait() {
        let base = Duration::from_millis(1000);
        let waits: HashSet<Duration> = (0..64).map(|_| jittered(base)).collect();
        assert!(waits.iter().all(|wait| *wait >= base && *wait <= base + base / 4), "{waits:?}");
        assert!(waits.len() > 1, "every retry left at the same instant");
    }

    #[tokio::test]
    async fn concurrent_requests_for_one_thumbnail_share_one_download() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let fileset = format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={L3}");
        let stuck = format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={MIRI}");
        let server = serve_routes(vec![
            Route::get(&fileset).delay(Duration::from_millis(400)).bytes(thumb.clone()),
            Route::get(&stuck).delay(Duration::from_secs(20)).bytes(thumb.clone()),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let run = |id: &'static str, dataset: &'static str| {
            let hub = Arc::clone(&hub);
            tokio::spawn(async move {
                let worker = Arc::clone(&hub);
                hub.registry()
                    .run(id, async move { thumbnail(&worker, id, MastMissionId::Jwst, dataset, MastLevelClass::Combined, None).await })
                    .await
            })
        };
        let earlier = run("g3:thumb:jw02739-o001_t001_nircam", L3);
        server.wait_for(|r| r.len() == 1).await;
        let later = run("g4:thumb:jw02739-o001_t001_nircam", L3);
        let earlier = earlier.await.unwrap().unwrap();
        let later = later.await.unwrap().unwrap();
        assert!(!earlier.from_cache);
        assert!(later.from_cache, "the next generation reuses the download already in flight");
        assert_eq!(later.request_id, "g4:thumb:jw02739-o001_t001_nircam");
        assert_eq!(later.path, earlier.path);
        assert_eq!(server.count(&fileset), 1);

        let holder = run("g5:thumb:jw02739-o002_t001_miri", MIRI);
        server.wait_for(|r| r.iter().any(|x| x.target.starts_with(&stuck))).await;
        let waiter = run("g6:thumb:jw02739-o002_t001_miri", MIRI);
        wait_registered(&hub, "g6:thumb:jw02739-o002_t001_miri").await;
        assert_eq!(server.count(&stuck), 1, "the second request waits for the first");
        assert_eq!(hub.registry().cancel(None, Some("g5:")).unwrap(), 1);
        assert_eq!(holder.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        let taken_over = server.wait_for(|r| r.iter().filter(|x| x.target.starts_with(&stuck)).count() == 2).await;
        assert_eq!(taken_over.iter().filter(|x| x.target.starts_with(&stuck)).count(), 2, "a cancelled holder hands over to the waiter");
        assert_eq!(hub.registry().cancel(None, Some("g6:")).unwrap(), 1);
        assert_eq!(waiter.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        assert!(hub.registry().is_empty());
        assert_eq!(hub.thumbnails_in_flight(), 0);
    }

    #[tokio::test]
    async fn requests_queued_behind_a_failing_thumbnail_get_its_answer() {
        let broken = "jw02739-o001_t001_nircam_clear-f187n_i2d_thumb.jpg";
        let route = format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={broken}");
        let mut routes = vec![Route::get(&route).delay(Duration::from_millis(600)).status(500).json(r#"{"detail":"thumbnail store down"}"#)];
        routes.extend(l3_list_routes(L3));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        image_list(&hub, MastMissionId::Jwst, L3).await.unwrap();
        let run = |id: &'static str| {
            let hub = Arc::clone(&hub);
            tokio::spawn(async move {
                let worker = Arc::clone(&hub);
                hub.registry()
                    .run(id, async move { thumbnail(&worker, id, MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(broken)).await })
                    .await
            })
        };
        let holder = run("g1:thumb:jw02739-o001_t001_nircam:f187n");
        server.wait_for(|r| r.iter().any(|x| x.target.starts_with(&route))).await;
        let waiter = run("g2:thumb:jw02739-o001_t001_nircam:f187n");
        wait_registered(&hub, "g2:thumb:jw02739-o001_t001_nircam:f187n").await;
        let held = holder.await.unwrap().unwrap_err();
        let shared = waiter.await.unwrap().unwrap_err();
        assert_eq!(held.code, MastErrorCode::Server);
        assert_eq!(shared.code, MastErrorCode::Server);
        assert_eq!(shared.context.detail.as_deref(), Some("thumbnail store down"));
        assert_eq!(server.count(&route), 2, "the queued request takes the holder's answer instead of asking MAST again");
        assert_eq!(hub.thumbnails_in_flight(), 0);

        let later = thumbnail(&hub, "g3:thumb:jw02739-o001_t001_nircam:f187n", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(broken)).await.unwrap_err();
        assert_eq!(later.code, MastErrorCode::Server);
        assert!(server.count(&route) > 2, "a request made after the flight ended asks MAST again");
    }

    #[tokio::test]
    async fn named_thumbnail_must_be_listed_and_follows_sniffed_type() {
        let mut png = Vec::new();
        image::DynamicImage::ImageLuma8(image::GrayImage::new(32, 20))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let name = "jw02739-o001_t001_nircam_clear-f187n_i2d_thumb.jpg";
        let mut routes = vec![Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={name}")).bytes(png.clone())];
        routes.extend(l3_list_routes(L3));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let strip = thumbnail(&hub, "g1:thumb:f", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(name)).await.unwrap();
        assert!(strip.path.ends_with("_thumb.jpg.png"), "{}", strip.path);
        assert_cache_path(&strip.path, dir.path());
        assert_eq!((strip.width, strip.height, strip.bytes), (32, 20, png.len() as u64));
        assert_eq!(strip.filename, name);
        assert_eq!(strip.wcs, None);
        assert_eq!(strip.image_count, Some(6));
        let request = server.requests().into_iter().find(|r| r.target.contains("get_image_thumbnail")).unwrap();
        assert_eq!(request.target, format!("/search/jwst/api/v0.1/get_image_thumbnail?filename={name}&fileset_name={L3}"));
        let again = thumbnail(&hub, "g2:thumb:f", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(name)).await.unwrap();
        assert!(again.from_cache);
        assert_eq!(again.path, strip.path);
        let before = server.requests().len();
        let missing = thumbnail(&hub, "g1:thumb:x", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some("other_thumb.jpg")).await.unwrap_err();
        assert_eq!(missing.code, MastErrorCode::BadInput);
        assert!(missing.message.starts_with("filename: "), "{}", missing.message);
        for bad in ["../x_thumb.jpg", "a b.jpg", ".hidden"] {
            let error = thumbnail(&hub, "g1:thumb:y", MastMissionId::Jwst, L3, MastLevelClass::Combined, Some(bad)).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{bad}");
        }
        let iue = thumbnail(&hub, "g1:thumb:z", MastMissionId::Iue, "lwp1", MastLevelClass::Unknown, None).await.unwrap_err();
        assert_eq!(iue.code, MastErrorCode::NotDeployed);
        let bad_id = thumbnail(&hub, "bad id", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap_err();
        assert_eq!(bad_id.code, MastErrorCode::BadInput);
        assert_eq!(server.requests().len(), before);
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name="), 0);
    }

    #[tokio::test]
    async fn preview_passes_small_images_through_and_resizes_large_ones() {
        let big = synthetic_jpeg(3000, 1000, false, Some(&wcs_comment([1500.5, 500.5], 1.0e-5)));
        let small = synthetic_jpeg(1000, 800, true, None);
        let mut routes = vec![
            Route::get("/search/hst/api/v0.1/get_image_preview?filename=ick909030_drc.jpg").header("Content-Type", "application/octet-stream").bytes(big.clone()),
            Route::get("/search/hst/api/v0.1/get_image_preview?filename=ick909030_drz.jpg").bytes(small.clone()),
            Route::get("/search/hst/api/v0.1/get_image_preview?filename=ick909c1q_flc.jpg").html("<html>oops</html>"),
            Route::get("/search/hst/api/v0.1/get_image_preview?filename=ick909cwq_flc.jpg")
                .bytes(vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x03, 0xE8, 0x0B, 0xB8, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xD9]),
        ];
        routes.extend(hst_list_routes());
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let folder = dataset_folder(dir.path(), "hst", ASSOCIATION);
        let (events, sink) = collector();
        let pane = preview(&hub, "g1:preview:p", MastMissionId::Hst, ASSOCIATION, "ick909030_drc.jpg", PreviewSize::Pane, &sink).await.unwrap();
        assert_eq!(pane.size, PreviewSize::Pane);
        assert_eq!(pane.image.request_id, "g1:preview:p");
        assert_eq!((pane.image.width, pane.image.height), (2048, 683));
        assert_eq!(pane.channels, 1);
        assert_eq!(pane.source, crate::core::mast::types::PreviewSource { width: 3000, height: 1000, bytes: big.len() as u64 });
        assert!(pane.image.path.ends_with("ick909030_drc.2048.jpg"), "{}", pane.image.path);
        assert_cache_path(&pane.image.path, dir.path());
        assert_eq!(pane.image.filename, "ick909030_drc.jpg");
        assert_eq!(pane.image.image_count, Some(17));
        assert!(!pane.image.from_cache);
        let written = std::fs::read(&pane.image.path).unwrap();
        assert_eq!(crate::core::mast::jpeg::dimensions(&written), Some((2048, 683, 1)));
        assert_eq!(pane.image.bytes, written.len() as u64);
        let wcs = pane.image.wcs.clone().unwrap();
        assert_eq!(wcs.orientation, WcsOrientation::CdMatrix);
        assert!((wcs.pixel_scale_arcsec - 0.036 * 3000.0 / 2048.0).abs() < 1e-9, "{wcs:?}");
        assert!((wcs.field_arcmin[0] - 1.8).abs() < 1e-6, "{wcs:?}");
        let seen = events.lock().unwrap().clone();
        assert_eq!(seen.first().map(|e| e.stage), Some(ProgressStage::Download));
        assert!(seen.iter().any(|e| e.stage == ProgressStage::Decode));
        assert_eq!(seen.last().map(|e| e.stage), Some(ProgressStage::Ready));
        assert!(seen.iter().all(|e| e.request_id == "g1:preview:p"));
        assert_eq!(seen.iter().filter(|e| e.stage == ProgressStage::Ready).count(), 1);
        assert_eq!(files_under(&folder), ["ick909030_drc.2048.jpg", "ick909030_drc.jpg"]);
        let request = server.requests().into_iter().find(|r| r.target.contains("get_image_preview")).unwrap();
        assert_eq!(request.target, "/search/hst/api/v0.1/get_image_preview?filename=ick909030_drc.jpg&fileset_name=ICK909030");

        let (events, sink) = collector();
        let again = preview(&hub, "g2:preview:p", MastMissionId::Hst, ASSOCIATION, "ick909030_drc.jpg", PreviewSize::Pane, &sink).await.unwrap();
        assert!(again.image.from_cache);
        assert_eq!(again.image.path, pane.image.path);
        assert_eq!(again.image.wcs, pane.image.wcs);
        assert_eq!(again.source, pane.source);
        assert_eq!(events.lock().unwrap().iter().map(|e| e.stage).collect::<Vec<_>>(), [ProgressStage::Ready]);
        let large = preview(&hub, "g2:preview:l", MastMissionId::Hst, ASSOCIATION, "ick909030_drc.jpg", PreviewSize::Large, &quiet).await.unwrap();
        assert!(large.image.path.ends_with("ick909030_drc.jpg"), "{}", large.image.path);
        assert_eq!((large.image.width, large.image.height), (3000, 1000));
        assert!((large.image.wcs.as_ref().unwrap().pixel_scale_arcsec - 0.036).abs() < 1e-9);
        assert_eq!(server.count("/search/hst/api/v0.1/get_image_preview"), 1);

        let (events, sink) = collector();
        let pass = preview(&hub, "g3:preview:z", MastMissionId::Hst, ASSOCIATION, "ick909030_drz.jpg", PreviewSize::Pane, &sink).await.unwrap();
        assert!(pass.image.path.ends_with("ick909030_drz.jpg"), "{}", pass.image.path);
        assert_eq!((pass.image.width, pass.image.height, pass.channels), (1000, 800, 3));
        assert_eq!(pass.source.width, 1000);
        assert_eq!(pass.image.wcs, None);
        assert!(!events.lock().unwrap().iter().any(|e| e.stage == ProgressStage::Decode));

        let html = preview(&hub, "g3:preview:h", MastMissionId::Hst, ASSOCIATION, "ick909c1q_flc.jpg", PreviewSize::Pane, &quiet).await.unwrap_err();
        assert_eq!(html.code, MastErrorCode::NotImage);
        assert!(!files_under(&folder).iter().any(|f| f.contains("flc") || f.ends_with(".part")));
        let corrupt = preview(&hub, "g3:preview:c", MastMissionId::Hst, ASSOCIATION, "ick909cwq_flc.jpg", PreviewSize::Pane, &quiet).await.unwrap_err();
        assert_eq!(corrupt.code, MastErrorCode::Decode);
        assert_eq!(corrupt.context.operation.as_deref(), Some("search/hst/api/v0.1/get_image_preview"));
        assert!(!files_under(&folder).iter().any(|f| f.contains("flc") || f.ends_with(".part")), "{:?}", files_under(&folder));
        let before = server.requests().len();
        let missing = preview(&hub, "g3:preview:m", MastMissionId::Hst, ASSOCIATION, "ick909030_drc_thumb.jpg", PreviewSize::Pane, &quiet).await.unwrap_err();
        assert_eq!(missing.code, MastErrorCode::BadInput);
        assert!(missing.message.starts_with("filename: "), "{}", missing.message);
        let traversal = preview(&hub, "g3:preview:t", MastMissionId::Hst, ASSOCIATION, "../x.jpg", PreviewSize::Pane, &quiet).await.unwrap_err();
        assert_eq!(traversal.code, MastErrorCode::BadInput);
        assert_eq!(server.requests().len(), before);
    }

    #[tokio::test]
    async fn preview_cancel_deletes_part_file() {
        let head = fixture_bytes("prev_hst_head_sos.bin");
        let mut routes = vec![Route::get("/search/hst/api/v0.1/get_image_preview?filename=ick909030_drc.jpg").stall(head.clone(), 5_000_000)];
        routes.extend(hst_list_routes());
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let folder = dataset_folder(dir.path(), "hst", ASSOCIATION);
        let id = "g1:preview:ICK909030:ick909030_drc.jpg:pane";
        let (events, sink) = collector();
        let task = {
            let hub = Arc::clone(&hub);
            tokio::spawn(async move {
                let worker = Arc::clone(&hub);
                hub.registry()
                    .run(id, async move {
                        preview(&worker, id, MastMissionId::Hst, ASSOCIATION, "ick909030_drc.jpg", PreviewSize::Pane, &sink).await
                    })
                    .await
            })
        };
        for _ in 0..400 {
            if !events.lock().unwrap().is_empty() && files_under(&folder).iter().any(|f| f.ends_with(".part")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(files_under(&folder).iter().any(|f| f.ends_with(".part")), "{:?}", files_under(&folder));
        let first = events.lock().unwrap()[0].clone();
        assert_eq!(first.stage, ProgressStage::Download);
        assert_eq!(first.total, Some(5_000_000));
        assert!(first.received >= head.len() as u64);
        assert_eq!(hub.registry().cancel(Some(&[id.to_owned()]), None).unwrap(), 1);
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code, MastErrorCode::Cancelled);
        let recorded = server.wait_for(|r| r.iter().any(|x| x.aborted && x.path().ends_with("get_image_preview"))).await;
        assert!(recorded.iter().any(|x| x.aborted && x.path().ends_with("get_image_preview")));
        assert!(files_under(&folder).is_empty(), "{:?}", files_under(&folder));
        assert!(hub.registry().is_empty());
        assert_eq!(hub.preview_permits().available_permits(), 1);
    }

    #[tokio::test]
    async fn preview_over_64mb_is_too_large_and_leaves_nothing() {
        let head = fixture_bytes("prev_hst_head_sos.bin");
        let over = 70 * 1024 * 1024;
        let mut routes = vec![
            Route::get("/search/hst/api/v0.1/get_image_preview?filename=ick909030_drc.jpg").oversized(head.clone(), over, true),
            Route::get("/search/hst/api/v0.1/get_image_preview?filename=ick909030_drz.jpg").oversized(head.clone(), over, false),
        ];
        routes.extend(hst_list_routes());
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let folder = dataset_folder(dir.path(), "hst", ASSOCIATION);
        let declared = preview(&hub, "g1:preview:a", MastMissionId::Hst, ASSOCIATION, "ick909030_drc.jpg", PreviewSize::Pane, &quiet).await.unwrap_err();
        assert_eq!(declared.code, MastErrorCode::TooLarge);
        assert_eq!(declared.context.bytes, Some(over as u64));
        assert_eq!(declared.context.operation.as_deref(), Some("search/hst/api/v0.1/get_image_preview"));
        assert!(files_under(&folder).is_empty(), "{:?}", files_under(&folder));
        let streamed = preview(&hub, "g1:preview:b", MastMissionId::Hst, ASSOCIATION, "ick909030_drz.jpg", PreviewSize::Large, &quiet).await.unwrap_err();
        assert_eq!(streamed.code, MastErrorCode::TooLarge);
        assert!(streamed.context.bytes.unwrap() > 64 * 1024 * 1024);
        assert!(files_under(&folder).is_empty(), "{:?}", files_under(&folder));
        assert_eq!(hub.preview_permits().available_permits(), 1);
    }

    #[tokio::test]
    async fn rate_limit_window_fails_fast_without_request() {
        let server = serve_routes(vec![
            Route::get("/search/hst/api/v0.1/get_image_thumbnail?fileset_name=").status(429).header("Retry-After", "5").json(r#"{"detail":"slow down"}"#),
            Route::post("/search/hst/api/v0.1/search").json(fixture("hst_m16.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let first = thumbnail(&hub, "g1:thumb:a", MastMissionId::Hst, ASSOCIATION, MastLevelClass::Unknown, None).await.unwrap_err();
        assert_eq!(first.code, MastErrorCode::RateLimited);
        assert_eq!(first.context.retry_after_sec, Some(5));
        assert_eq!(first.context.consecutive, Some(1));
        assert_eq!(first.context.status, Some(429));
        assert_eq!(first.context.operation.as_deref(), Some("search/hst/api/v0.1/get_image_thumbnail"));
        assert_eq!(server.requests().len(), 1);
        let second = thumbnail(&hub, "g1:thumb:b", MastMissionId::Hst, "IDNM20010", MastLevelClass::Unknown, None).await.unwrap_err();
        assert_eq!(second.code, MastErrorCode::RateLimited);
        assert!((1..=5).contains(&second.context.retry_after_sec.unwrap()), "{second:?}");
        assert_eq!(second.context.consecutive, Some(1));
        assert_eq!(second.context.status, None);
        assert_eq!(second.context.operation.as_deref(), Some("search/hst/api/v0.1/get_image_thumbnail"));
        let list = image_list(&hub, MastMissionId::Hst, ASSOCIATION).await.unwrap_err();
        assert_eq!(list.code, MastErrorCode::RateLimited);
        assert_eq!(list.context.operation.as_deref(), Some("search/hst/api/v0.1/get_image_list"));
        let pane = preview(&hub, "g1:preview:c", MastMissionId::Hst, ASSOCIATION, "ick909030_drc.jpg", PreviewSize::Pane, &quiet).await.unwrap_err();
        assert_eq!(pane.code, MastErrorCode::RateLimited);
        assert_eq!(server.requests().len(), 1, "image calls inside the window send nothing");
        search(&hub, "g2:search", &m16(MastMissionId::Hst), false).await.unwrap();
        assert_eq!(server.requests().len(), 2, "searches never check the window");
    }

    #[tokio::test]
    async fn rate_limit_ladder_and_consecutive() {
        use crate::infra::mast::ratelimit::RateLimiter;
        let limiter = RateLimiter::new();
        let op = Operation::mission(MastMissionId::Jwst, OpKind::Thumbnail, "get_image_thumbnail");
        let limited = MastError::Status {
            status: 429,
            url: "u".into(),
            body: String::new(),
            detail: None,
            content_type: None,
            retry_after_secs: None,
        };
        let t0 = Instant::now();
        let mut now = t0;
        for (expected_consecutive, expected_wait) in [(1u32, 10u64), (2, 30), (3, 60), (4, 60)] {
            assert!(limiter.open_window(now).is_none());
            assert_eq!(limiter.record(&limited, &op, now, now), expected_consecutive);
            let window = limiter.open_window(now).unwrap();
            assert_eq!((window.remaining_secs, window.consecutive), (expected_wait, expected_consecutive));
            let classified = classify(&limited, &op, expected_consecutive);
            assert_eq!(classified.context.retry_after_sec, Some(expected_wait));
            now += Duration::from_secs(expected_wait);
        }
        limiter.on_success();
        assert_eq!(limiter.consecutive(), 0);
        assert!(limiter.open_window(now).is_none());

        let server = serve_routes(vec![
            Route::get("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name=").status(503).html("<html>busy</html>"),
            Route::get("/search/jwst/api/v0.1/get_image_list").status(404).json(r#"{"detail":"Image not found"}"#),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        for (i, (dataset, attempts)) in [("jw01", 2), ("jw02", 1), ("jw03", 1)].into_iter().enumerate() {
            let error = thumbnail(&hub, "g1:thumb:x", MastMissionId::Jwst, dataset, MastLevelClass::Combined, None).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::Server, "{dataset}: the 503 is what the card shows");
            assert_eq!(error.context.status, Some(503));
            assert_eq!(error.context.operation.as_deref(), Some("search/jwst/api/v0.1/get_image_thumbnail"));
            assert_eq!(error.context.attempts, Some(attempts), "{dataset}: only the first busy answer is retried, the next cards see the outage");
            assert_eq!(hub.limiter().consecutive(), i as u32 + 1, "a retried request counts once");
        }
        let breaker = thumbnail(&hub, "g1:thumb:y", MastMissionId::Jwst, "jw04", MastLevelClass::Combined, None).await.unwrap_err();
        assert_eq!(breaker.code, MastErrorCode::RateLimited);
        assert_eq!(breaker.context.consecutive, Some(3));
        assert!((59..=60).contains(&breaker.context.retry_after_sec.unwrap()), "{breaker:?}");
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_thumbnail"), 4);
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_list"), 2, "only the first card tries the lists; the others fail during the outage");
        assert!(!hub.fileset_failed(MastMissionId::Jwst, MastLevelClass::Combined), "a 503 is not a missing image");
    }

    #[tokio::test]
    async fn one_burst_of_rate_limits_is_one_episode() {
        let server = serve_routes(vec![Route::get("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name=")
            .status(429)
            .header("Retry-After", "5")
            .delay(Duration::from_millis(300))
            .html("<html>slow down</html>")])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let call = |dataset: &'static str| thumbnail(&hub, "g1:thumb:x", MastMissionId::Jwst, dataset, MastLevelClass::Combined, None);
        let (a, b, c, d) = tokio::join!(call("jw01"), call("jw02"), call("jw03"), call("jw04"));
        for error in [a, b, c, d].map(Result::unwrap_err) {
            assert_eq!(error.code, MastErrorCode::RateLimited, "{error:?}");
            assert_eq!(error.context.consecutive, Some(1), "{error:?}");
            assert_eq!(error.context.retry_after_sec, Some(5), "{error:?}");
        }
        let blocked = call("jw05").await.unwrap_err();
        assert_eq!(blocked.code, MastErrorCode::RateLimited);
        assert_eq!(blocked.context.consecutive, Some(1), "{blocked:?}");
        assert_eq!(server.requests().len(), 4);
    }

    #[tokio::test]
    async fn cache_info_and_clear_by_kind() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let mut routes = vec![Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={L3}")).bytes(thumb.clone())];
        routes.extend(l3_list_routes(L3));
        routes.push(Route::post("/search/jwst/api/v0.1/search").json(fixture("jwst_m16_l3.json")));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        image_list(&hub, MastMissionId::Jwst, L3).await.unwrap();
        let stored = crate::infra::mast::lookup::StoredName {
            canonical_name: "M 16".into(),
            resolver: "SIMBAD".into(),
            ra_deg: 274.688,
            dec_deg: -13.792,
            object_type: None,
            stored_at: SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        };
        hub.names().write_positive(hub.cache_root(), "m16", &stored, SystemTime::now()).unwrap();
        let list_bytes = std::fs::metadata(dir.path().join("lists").join("jwst").join(format!("{L3}.images.json"))).unwrap().len();
        let names_bytes = std::fs::metadata(dir.path().join(NAMES_FILE)).unwrap().len();
        assert_eq!(cache_info(&hub), MastCacheInfo { bytes: thumb.len() as u64 + list_bytes + names_bytes, files: 3 });

        assert_eq!(cache_clear(&hub, MastCacheKind::Images).unwrap(), MastCacheInfo { bytes: thumb.len() as u64, files: 1 });
        assert!(!dir.path().join("img").exists());
        assert_eq!(cache_info(&hub).files, 2);
        let refetched = thumbnail(&hub, "g2:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        assert!(!refetched.from_cache);
        assert_eq!(refetched.image_count, Some(6));
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_thumbnail"), 2);

        assert_eq!(cache_clear(&hub, MastCacheKind::Lists).unwrap(), MastCacheInfo { bytes: list_bytes, files: 1 });
        assert!(hub.image_sets().is_empty());
        assert!(!image_list(&hub, MastMissionId::Jwst, L3).await.unwrap().from_cache);
        assert_eq!(server.count("/search/jwst/api/v0.1/get_image_list"), 4);

        hub.names().remember_negative("nowhere", Instant::now());
        assert_eq!(cache_clear(&hub, MastCacheKind::Names).unwrap(), MastCacheInfo { bytes: names_bytes, files: 1 });
        assert!(!dir.path().join(NAMES_FILE).exists());
        assert!(!hub.names().is_negative("nowhere", Instant::now()));
        assert_eq!(cache_clear(&hub, MastCacheKind::Names).unwrap(), MastCacheInfo::default());

        search(&hub, "g3:search", &m16(MastMissionId::Jwst), false).await.unwrap();
        let everything = cache_info(&hub);
        assert_eq!(everything.files, 2);
        assert_eq!(cache_clear(&hub, MastCacheKind::All).unwrap(), everything);
        assert_eq!(cache_info(&hub), MastCacheInfo::default());
        assert!(hub.image_sets().is_empty());
        assert!(!search(&hub, "g4:search", &m16(MastMissionId::Jwst), false).await.unwrap().from_cache);
    }

    struct Undeletable {
        #[cfg(unix)]
        folder: PathBuf,
        #[cfg(windows)]
        _open: std::fs::File,
    }

    impl Undeletable {
        #[cfg(windows)]
        fn new(path: &Path) -> Self {
            use std::os::windows::fs::OpenOptionsExt;
            Self { _open: std::fs::OpenOptions::new().read(true).share_mode(0).open(path).unwrap() }
        }

        #[cfg(unix)]
        fn new(path: &Path) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let folder = path.parent().unwrap().to_path_buf();
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
            Self { folder }
        }
    }

    #[cfg(unix)]
    impl Drop for Undeletable {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.folder, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[tokio::test]
    async fn clearing_resets_memory_and_reports_what_was_freed_when_a_file_stays() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let mut routes = vec![Route::get(&format!("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name={L3}")).bytes(thumb.clone())];
        routes.extend(l3_list_routes(L3));
        routes.push(Route::post("/search/jwst/api/v0.1/search").json(fixture("jwst_m16_l3.json")));
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        thumbnail(&hub, "g1:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        image_list(&hub, MastMissionId::Jwst, L3).await.unwrap();
        search(&hub, "g1:search", &m16(MastMissionId::Jwst), false).await.unwrap();
        let before = cache_info(&hub);
        let stuck = dataset_folder(dir.path(), "jwst", "jw00001-o001_t001_nircam").join("stuck_thumb.jpg");
        std::fs::create_dir_all(stuck.parent().unwrap()).unwrap();
        std::fs::write(&stuck, b"locked").unwrap();
        let pinned = Undeletable::new(&stuck);
        let freed = cache_clear(&hub, MastCacheKind::All).unwrap();
        assert_eq!(freed, before, "every other file is removed and reported");
        assert!(stuck.exists());
        assert!(hub.image_sets().is_empty());
        assert_eq!(hub.image_budget().used(), Some(6), "the budget counts what stayed on disk");
        assert!(!search(&hub, "g2:search", &m16(MastMissionId::Jwst), false).await.unwrap().from_cache);
        let refetched = thumbnail(&hub, "g2:thumb:a", MastMissionId::Jwst, L3, MastLevelClass::Combined, None).await.unwrap();
        assert!(!refetched.from_cache);
        drop(pinned);
        let only_stuck = Undeletable::new(&stuck);
        cache_clear(&hub, MastCacheKind::Images).unwrap();
        let error = cache_clear(&hub, MastCacheKind::Images).unwrap_err();
        assert_eq!(error.code, MastErrorCode::Cache);
        let expected = if cfg!(windows) { "file in use by another program" } else { "permission denied" };
        assert_eq!(error.message, expected);
        drop(only_stuck);
    }

    #[tokio::test]
    async fn image_writes_evict_least_recent_files_over_the_cap() {
        let thumb = fixture_bytes("t_clear-f187n.bin");
        let server = serve_routes(vec![Route::get("/search/jwst/api/v0.1/get_image_thumbnail?fileset_name=").bytes(thumb.clone())]).await;
        let dir = tempfile::tempdir().unwrap();
        let base = server.base.clone();
        let settings = MastSettings::from_lookup(move |name| (name == ENV_MAST_URL).then(|| base.clone()));
        let cap = thumb.len() as u64 * 2 + 1000;
        let hub = Arc::new(MastHub::new(settings, dir.path().to_path_buf()).unwrap().with_disk_caps(cap, 1 << 20));
        evict_cache(&hub);
        assert_eq!(hub.image_budget().used(), Some(0));
        let datasets = ["jw00001-o001_t001_nircam", "jw00002-o001_t001_nircam", "jw00003-o001_t001_nircam", "jw00004-o001_t001_nircam"];
        for dataset in datasets {
            thumbnail(&hub, "g1:thumb:x", MastMissionId::Jwst, dataset, MastLevelClass::Combined, None).await.unwrap();
            settle(&hub.image_budget()).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let left = files_under(&dir.path().join("img"));
        assert_eq!(
            left,
            [
                "jwst/jw00003-o001_t001_nircam/_representative.jpg".to_owned(),
                "jwst/jw00004-o001_t001_nircam/_representative.jpg".to_owned(),
            ]
        );
        assert_eq!(hub.image_budget().used(), Some(thumb.len() as u64 * 2));
        std::fs::write(dir.path().join("img").join("stale.jpg"), vec![0u8; cap as usize]).unwrap();
        let restarted = MastHub::new(MastSettings::default(), dir.path().to_path_buf()).unwrap().with_disk_caps(cap, 1 << 20);
        evict_cache(&restarted);
        assert!(dir_info_bytes(&dir.path().join("img")) <= restarted.image_budget().target());
    }

    const PRODUCTS_PATH: &str = "/search/jwst/api/v0.1/post_list_products";

    fn product_entry(dataset: &str, filename: &str, category: &str, suffix: &str, size: i64) -> Value {
        json!({
            "product_key": format!("{dataset}_{filename}"),
            "access": "PUBLIC",
            "dataset": dataset,
            "filename": filename,
            "uri": format!("{dataset}/{filename}"),
            "authz_primary_identifier": filename,
            "authz_secondary_identifier": filename,
            "file_suffix": suffix,
            "category": category,
            "size": size,
            "type": "science"
        })
    }

    fn synthetic_listing(ids: &[String]) -> String {
        let products: Vec<Value> = ids.iter().map(|id| product_entry(id, &format!("{id}_clear-f090w_i2d.fits"), "3", "_i2d", 1000)).collect();
        json!({ "products": products }).to_string()
    }

    fn sent_ids(request: &Recorded) -> Vec<String> {
        serde_json::from_str::<Value>(&request.body).unwrap()["dataset_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    }

    fn find_uri<'a>(list: &'a MastProductList, filename: &str) -> &'a str {
        &list.products.iter().find(|p| p.filename == filename).unwrap().mast_uri
    }

    #[tokio::test]
    async fn products_post_in_chunks_of_50() {
        let ids: Vec<String> = (1..=120).map(|n| format!("jw{n:05}-o001_t001_nircam")).collect();
        let routes = ids
            .chunks(PRODUCT_CHUNK)
            .map(|chunk| {
                let marker = format!("\"{}\"", chunk[0]);
                Route::post(PRODUCTS_PATH).when(move |r| r.body.contains(&marker)).json(synthetic_listing(chunk))
            })
            .collect();
        let server = serve_routes(routes).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let list = products(&hub, "g1:products", MastMissionId::Jwst, &ids, false).await.unwrap();
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        let chunks: Vec<Vec<String>> = requests.iter().map(sent_ids).collect();
        assert_eq!(chunks, vec![ids[..50].to_vec(), ids[50..100].to_vec(), ids[100..].to_vec()]);
        for request in &requests {
            assert_eq!(request.method, "POST");
            assert_eq!(request.target, PRODUCTS_PATH);
            assert_eq!(request.header("content-type"), Some("application/json"));
            assert_eq!(request.header("authorization"), None);
            assert_eq!(request.header("cookie"), None);
        }
        assert_eq!(list.mission, MastMissionId::Jwst);
        assert_eq!(list.dataset_ids, ids);
        assert_eq!(list.total_count, 120);
        assert_eq!(list.total_bytes, 120_000);
        let owners: std::collections::BTreeSet<&str> = list.products.iter().map(|p| p.dataset_id.as_str()).collect();
        assert_eq!(owners, ids.iter().map(String::as_str).collect());
        assert!(list.products.iter().all(|p| p.cls == MastProductClass::Image && p.mast_uri.starts_with("mast:JWST/product/")));
        assert_eq!(list.groups.len(), 1);
        assert_eq!((list.groups[0].count, list.groups[0].bytes), (120, 120_000));
        assert_eq!((list.from_cache, list.cached_until.as_deref()), (false, None));
        assert!(!dir.path().join("lists").exists(), "multi-id listings are not cached");
        assert!(hub.product_lists().is_empty());

        let mut doubled = ids.clone();
        doubled.push(ids[0].clone());
        doubled.push(ids[119].clone());
        let again = products(&hub, "g2:products", MastMissionId::Jwst, &doubled, false).await.unwrap();
        assert!(!again.from_cache);
        assert_eq!(again.dataset_ids, ids);
        assert_eq!(again.total_count, 120);
        let requests = server.requests();
        assert_eq!(requests.len(), 6);
        assert_eq!(sent_ids(&requests[3]), ids[..50].to_vec());
        assert_eq!(sent_ids(&requests[5]), ids[100..].to_vec());
        assert!(requests.iter().all(|r| !r.target.contains("retrieve_product")));
    }

    #[tokio::test]
    async fn products_cache_single_id_only() {
        let broken = "jw09999-o001_t001_nircam";
        let miri = json!({ "products": [product_entry(MIRI, "jw02739-o002_t001_miri_f770w_i2d.fits", "3", "_i2d", 2000)] }).to_string();
        let server = serve_routes(vec![
            Route::post(PRODUCTS_PATH).when(move |r| r.body.contains(broken)).status(500).html("<html>boom</html>"),
            Route::post(PRODUCTS_PATH).when(|r| r.body.contains(MIRI)).json(miri),
            Route::post(PRODUCTS_PATH).json(fixture("prod_jw.json")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let single = [L3.to_owned()];
        let before = SystemTime::now();
        let first = products(&hub, "g1:products:a", MastMissionId::Jwst, &single, false).await.unwrap();
        assert!(!first.from_cache);
        assert_eq!(first.total_count, 108);
        let until = first.cached_until.clone().unwrap();
        assert!(until >= utc_datetime(before + LIST_TTL) && until <= utc_datetime(SystemTime::now() + LIST_TTL), "{until}");
        assert_eq!(until.len(), "2026-10-17T04:10:00Z".len());
        let file = dir.path().join("lists").join("jwst").join(format!("{L3}.products.json"));
        assert!(file.is_file());
        assert_eq!(server.count(PRODUCTS_PATH), 1);

        let cached = products(&hub, "g2:products:a", MastMissionId::Jwst, &single, false).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.products, first.products);
        assert_eq!(cached.groups, first.groups);
        assert_eq!(cached.cached_until.as_deref(), Some(until.as_str()));
        assert_eq!(server.count(PRODUCTS_PATH), 1);

        let restarted = hub_for(&server, dir.path());
        let from_disk = products(&restarted, "g1:products:a", MastMissionId::Jwst, &single, false).await.unwrap();
        assert!(from_disk.from_cache);
        assert_eq!(from_disk.products, first.products);
        assert_eq!(from_disk.cached_until.as_deref(), Some(until.as_str()));
        assert_eq!(server.count(PRODUCTS_PATH), 1);

        let refreshed = products(&hub, "g3:products:a", MastMissionId::Jwst, &single, true).await.unwrap();
        assert!(!refreshed.from_cache);
        assert!(refreshed.cached_until.clone().unwrap() >= until);
        assert_eq!(server.count(PRODUCTS_PATH), 2);

        let pair = [L3.to_owned(), MIRI.to_owned()];
        let combined = products(&hub, "g3:products", MastMissionId::Jwst, &pair, false).await.unwrap();
        assert!(!combined.from_cache);
        assert_eq!(combined.dataset_ids, pair.to_vec());
        assert_eq!(combined.cached_until, None);
        assert_eq!(combined.products[0].dataset_id, MIRI);
        assert_eq!(server.count(PRODUCTS_PATH), 3);
        assert!(!products(&hub, "g4:products", MastMissionId::Jwst, &pair, false).await.unwrap().from_cache);
        assert_eq!(server.count(PRODUCTS_PATH), 4);
        assert!(!dir.path().join("lists").join("jwst").join(format!("{MIRI}.products.json")).exists());
        assert_eq!(hub.product_lists().len(), 1);
        assert!(products(&hub, "g4:products:a", MastMissionId::Jwst, &single, false).await.unwrap().from_cache);
        assert_eq!(server.count(PRODUCTS_PATH), 4);

        write_stored(&file, &from_disk, SystemTime::now() - LIST_TTL - Duration::from_secs(60)).unwrap();
        let expired = hub_for(&server, dir.path());
        let renewed = products(&expired, "g1:products:a", MastMissionId::Jwst, &single, false).await.unwrap();
        assert!(!renewed.from_cache, "a listing older than 24 h is fetched again");
        assert_eq!(server.count(PRODUCTS_PATH), 5);

        hub.product_lists().insert((MastMissionId::Jwst, L3.to_owned()), (first.clone(), SystemTime::now() - Duration::from_secs(1)), Instant::now());
        let past_memory = products(&hub, "g5:products:a", MastMissionId::Jwst, &single, false).await.unwrap();
        assert!(past_memory.from_cache);
        assert_eq!(past_memory.cached_until, renewed.cached_until, "an expired memory entry falls through to the newer file");
        assert_eq!(server.count(PRODUCTS_PATH), 5);

        cache_clear(&hub, MastCacheKind::Lists).unwrap();
        assert!(hub.product_lists().is_empty());
        assert!(!file.exists());
        assert!(!products(&hub, "g6:products:a", MastMissionId::Jwst, &single, false).await.unwrap().from_cache);
        assert_eq!(server.count(PRODUCTS_PATH), 6);

        let error = products(&hub, "g7:products:b", MastMissionId::Jwst, &[broken.to_owned()], false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Server);
        assert_eq!(error.context.status, Some(500));
        assert_eq!(error.context.operation.as_deref(), Some("search/jwst/api/v0.1/post_list_products"));
        assert!(!dir.path().join("lists").join("jwst").join(format!("{broken}.products.json")).exists());
        assert_eq!(server.count(PRODUCTS_PATH), 7);
        assert!(server.requests().iter().all(|r| !r.target.contains("retrieve_product")));
    }

    #[tokio::test]
    async fn blank_product_listing_is_an_error_and_is_not_stored() {
        let blank = "jw09001-o001_t001_nircam";
        let no_content = "jw09002-o001_t001_nircam";
        let spaces = "jw09003-o001_t001_nircam";
        let server = serve_routes(vec![
            Route::post(PRODUCTS_PATH).when(move |r| r.body.contains(blank)).bytes(Vec::new()),
            Route::post(PRODUCTS_PATH).when(move |r| r.body.contains(no_content)).status(204),
            Route::post(PRODUCTS_PATH).when(move |r| r.body.contains(spaces)).bytes(b" \r\n ".to_vec()),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        for dataset in [blank, no_content, spaces] {
            let error = products(&hub, "g1:products:e", MastMissionId::Jwst, &[dataset.to_owned()], false).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::Decode, "{dataset}");
            assert_eq!(error.message, "empty answer", "{dataset}");
            assert_eq!(error.context.operation.as_deref(), Some("search/jwst/api/v0.1/post_list_products"), "{dataset}");
            assert!(!dir.path().join("lists").join("jwst").join(format!("{dataset}.products.json")).exists(), "{dataset}");
        }
        assert!(products(&hub, "g2:products:e", MastMissionId::Jwst, &[blank.to_owned()], false).await.is_err());
        assert_eq!(server.count(PRODUCTS_PATH), 4, "a blank answer is never cached");
        assert!(hub.product_lists().is_empty());
    }

    #[tokio::test]
    async fn empty_product_listing_is_not_stored_on_disk() {
        let server = serve_routes(vec![Route::post(PRODUCTS_PATH).json(r#"{"products": []}"#)]).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let single = [L3.to_owned()];
        let first = products(&hub, "g1:products:z", MastMissionId::Jwst, &single, false).await.unwrap();
        assert_eq!(first.total_count, 0);
        assert!(!first.from_cache);
        assert!(!dir.path().join("lists").join("jwst").join(format!("{L3}.products.json")).exists());
        assert!(products(&hub, "g2:products:z", MastMissionId::Jwst, &single, false).await.unwrap().from_cache);
        assert_eq!(server.count(PRODUCTS_PATH), 1);
        let restarted = hub_for(&server, dir.path());
        let again = products(&restarted, "g1:products:z", MastMissionId::Jwst, &single, false).await.unwrap();
        assert!(!again.from_cache, "an empty listing is asked for again after a restart");
        assert_eq!(server.count(PRODUCTS_PATH), 2);
    }

    #[tokio::test]
    async fn products_refused_for_unverified_missions() {
        let classy = json!({
            "products": [product_entry("J0036-3333", "hlsp_classy_hst_cos_j0036-3333_multi_v1_coadded.fits", "SCIENCE", "coadded", 14_469_120)]
        })
        .to_string();
        let server = serve_routes(vec![
            Route::post("/search/iue/api/v0.1/post_list_products").json(fixture("prod_iue.json")),
            Route::post("/search/classy/api/v0.1/post_list_products").json(classy),
            Route::post("/search/").json(r#"{"products": []}"#),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = hub_for(&server, dir.path());
        let mut expected_requests = 0;
        for (id, dataset, verified) in [
            (MastMissionId::Iue, "LWP00501", IUE_PRODUCTS_OK),
            (MastMissionId::Classy, "J0036-3333", CLASSY_PRODUCTS_OK),
            (MastMissionId::Ullyses, "453", ULLYSES_PRODUCTS_OK),
        ] {
            assert_eq!(mission(id).capabilities.products, verified, "{id:?}");
            let result = products(&hub, "g1:products", id, &[dataset.to_owned()], false).await;
            if verified {
                let list = result.unwrap();
                expected_requests += 1;
                assert_eq!(list.mission, id);
                assert!(list.products.iter().all(|p| p.dataset_id == dataset), "{id:?}");
                if id == MastMissionId::Iue {
                    assert_eq!(list.total_count, 6);
                    assert_eq!(find_uri(&list, "lwp00501.lihi.gz"), "mast:IUE/url//pub/iue/data/lwp/00000/lwp00501.lihi.gz");
                }
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.code, MastErrorCode::NotDeployed, "{id:?}");
                assert_eq!(error.context.mission, Some(id));
                assert_eq!(error.context.operation, Some(format!("search/{}/api/v0.1/post_list_products", id.slug())));
                assert_eq!(error.context.status, None);
            }
            assert_eq!(server.requests().len(), expected_requests, "{id:?}");
        }
        hub.record_probe(MastMissionId::Tess, ProbeResult { availability: MastAvailability::Open, checked_on: "2026-10-16".into() });
        for id in [MastMissionId::RomanCgi, MastMissionId::RomanSpectra, MastMissionId::Tess, MastMissionId::MultiMission] {
            let error = products(&hub, "g1:products", id, &["x1".to_owned()], false).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::NotDeployed, "{id:?}");
            assert_eq!(error.context.mission, Some(id));
        }
        let roman = products(&hub, "g1:products", MastMissionId::Roman, &["r0001".to_owned()], false).await.unwrap_err();
        assert_eq!(roman.code, MastErrorCode::Unauthorized);
        assert_eq!(roman.context.mission, Some(MastMissionId::Roman));
        assert_eq!(roman.context.status, None);
        let too_many: Vec<String> = (0..=PRODUCT_IDS_MAX).map(|n| format!("jw{n:05}")).collect();
        for bad in [Vec::new(), vec!["../x".to_owned()], vec!["CON".to_owned()], vec![L3.to_owned(), "a b".to_owned()], too_many] {
            let error = products(&hub, "g1:products", MastMissionId::Jwst, &bad, false).await.unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{:?}", bad.first());
            assert!(error.message.starts_with("datasetId"), "{}", error.message);
        }
        let error = products(&hub, "bad id", MastMissionId::Jwst, &[L3.to_owned()], false).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput);
        assert_eq!(server.requests().len(), expected_requests);
        assert!(server.requests().iter().all(|r| !r.target.contains("retrieve_product")));
    }

    fn dir_info_bytes(dir: &Path) -> u64 {
        crate::infra::mast::cache::dir_info(dir).bytes
    }

    fn observed_dates(results: &SearchResults<FieldMap>) -> Vec<String> {
        results
            .results
            .iter()
            .filter_map(|row| row.get("date_obs").and_then(Value::as_str).map(str::to_owned))
            .collect()
    }

    fn descending(dates: &[String]) -> bool {
        dates.windows(2).all(|pair| pair[0] >= pair[1])
    }

    fn live_cone(mission: MastMissionId, ra_deg: f64, dec_deg: f64, level: LevelFilter) -> MastQuery {
        let mut query = m16(mission);
        query.where_ = MastWhere::Position { ra_deg, dec_deg, radius_arcmin: 3.0, label: None };
        query.filters.level = Some(level);
        query
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_object_filters_and_local_level() {
        let dir = tempfile::tempdir().unwrap();
        let hub = live_hub(dir.path());
        let mut m16_ism = live_cone(MastMissionId::Jwst, 274.688, -13.792, LevelFilter::Combined);
        m16_ism.filters.categories = vec![ObjectCategory::Ism];
        let started = Instant::now();
        let page = search(&hub, "live:1", &m16_ism, false).await.unwrap();
        let seconds = started.elapsed().as_secs_f64();
        eprintln!(
            "L2 M16 Combined ISM: rows {} total {:?} local {} dropped {:?} {:.1} s levels {:?}",
            page.rows.len(),
            page.total_reported,
            page.level_applied_locally.is_some(),
            page.dropped_conditions,
            seconds,
            page.rows.iter().map(|row| row.level.clone()).collect::<Vec<_>>()
        );
        assert!(page.level_applied_locally.is_some());
        assert_eq!(page.rows.len(), 2);
        assert!(page.rows.iter().all(|row| row.categories == [ObjectCategory::Ism]));
        assert!(seconds < 15.0);
        let mut all_levels = m16_ism.clone();
        all_levels.filters.level = Some(LevelFilter::All);
        let wide = search(&hub, "live:2", &all_levels, false).await.unwrap();
        eprintln!("L1 M16 All ISM: rows {} total {:?} reliable {}", wide.rows.len(), wide.total_reported, wide.total_reliable);
        assert!(wide.rows.iter().all(|row| row.values["targcat"] == "ISM"));
        let mut trappist = live_cone(MastMissionId::Jwst, 346.6223, -5.0414, LevelFilter::Combined);
        trappist.filters.object_types = vec![ObjectType::Exoplanets];
        let started = Instant::now();
        let planets = search(&hub, "live:3", &trappist, false).await.unwrap();
        let seconds = started.elapsed().as_secs_f64();
        eprintln!("L8 TRAPPIST-1 Combined Exoplanets: rows {} local {} {:.1} s", planets.rows.len(), planets.level_applied_locally.is_some(), seconds);
        assert!(planets.level_applied_locally.is_some() && seconds < 15.0);
        assert!(planets.rows.iter().all(|row| row.target_description.as_deref().is_some_and(|text| text.contains("Exoplanet"))));
        let mut hst = live_cone(MastMissionId::Hst, 274.688, -13.792, LevelFilter::All);
        hst.filters.categories = vec![ObjectCategory::Ism];
        let hst_page = search(&hub, "live:4", &hst, false).await.unwrap();
        eprintln!(
            "L5 HST M16 ISM: rows {} total {:?} reliable {} dropped {:?}",
            hst_page.rows.len(),
            hst_page.total_reported,
            hst_page.total_reliable,
            hst_page.dropped_conditions
        );
        assert_eq!(hst_page.total_reported, Some(hst_page.rows.len() as i64));
        assert!(hst_page.rows.iter().all(|row| row.categories.contains(&ObjectCategory::Ism)));
        let mut iue = live_cone(MastMissionId::Iue, 274.688, -13.792, LevelFilter::All);
        iue.filters.categories = vec![ObjectCategory::Stars];
        let iue_page = search(&hub, "live:5", &iue, false).await.unwrap();
        eprintln!("L9 IUE M16 Stars: rows {} dropped {:?}", iue_page.rows.len(), iue_page.dropped_conditions);
        assert!(!iue_page.rows.is_empty());
        assert!(iue_page.rows.iter().all(|row| row.categories == [ObjectCategory::Stars]));
        assert!(iue_page.dropped_conditions.is_empty());
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_jwst_optical_elements_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        let hub = live_hub(dir.path());
        let endpoint: MissionEndpoint = hub.endpoint(MastMissionId::Jwst, OpKind::Search).unwrap();
        let params = |pattern: &str| {
            SearchParams::new()
                .target("274.688 -13.792")
                .radius(3.0, "arcminutes")
                .page(0, 50)
                .select(["fileSetName", "filter", "opticalElements"])
                .condition("productLevel", "3")
                .condition("opticalElements", pattern)
        };
        let started = Instant::now();
        let hit: SearchResults<FieldMap> = endpoint.search(&params("*F470N*")).await.unwrap();
        let hit_ms = elapsed_ms(started);
        let ids: Vec<String> = hit.results.iter().filter_map(|r| r.get("fileSetName").and_then(Value::as_str).map(str::to_owned)).collect();
        let all_match = hit
            .results
            .iter()
            .all(|r| r.get("opticalElements").and_then(Value::as_str).is_some_and(|text| text.contains("F470N")));
        let none: SearchResults<FieldMap> = endpoint.search(&params("*F999W*")).await.unwrap();
        eprintln!(
            "live_jwst_optical_elements_wildcard: *F470N* -> rows {} total {} ids {:?} all_match {} ({} ms); *F999W* -> rows {} total {}; echo {:?}",
            hit.results.len(),
            hit.total_results,
            ids,
            all_match,
            hit_ms,
            none.results.len(),
            none.total_results,
            hit.search_params.conditions
        );
        let works = !hit.results.is_empty() && all_match && ids.iter().any(|id| id == "jw02739-o001_t001_nircam") && none.results.is_empty();
        assert_eq!(works, JWST_FILTER_WILDCARD_OK);
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_hst_dataset_condition() {
        let dir = tempfile::tempdir().unwrap();
        let hub = live_hub(dir.path());
        let endpoint = hub.endpoint(MastMissionId::Hst, OpKind::Search).unwrap();
        let params = SearchParams::new()
            .page(0, 10)
            .select(["sci_data_set_name", "sci_targname", "sci_instrume"])
            .condition("sci_data_set_name", "ICK909030");
        let started = Instant::now();
        let raw: SearchResults<FieldMap> = endpoint.search(&params).await.unwrap();
        let ids: Vec<String> = raw
            .results
            .iter()
            .filter_map(|r| r.get("sci_data_set_name").and_then(Value::as_str).map(|s| s.trim().to_owned()))
            .collect();
        eprintln!(
            "live_hst_dataset_condition: rows {} total {} ids {:?} ({} ms) echo {:?}",
            raw.results.len(),
            raw.total_results,
            ids,
            elapsed_ms(started),
            raw.search_params.conditions
        );
        let works = ids == vec!["ICK909030".to_owned()];
        assert_eq!(works, HST_DATASET_CONDITION_OK);
        let query = MastQuery { where_: MastWhere::None, dataset_id: Some("ick909030".into()), ..m16(MastMissionId::Hst) };
        let page = search(&hub, "live:hst", &query, false).await.unwrap();
        let found: Vec<&str> = page.rows.iter().map(|r| r.dataset_id.as_str()).collect();
        eprintln!("live_hst_dataset_condition via service (lowercase input): rows {:?} applied {:?}", found, page.applied.conditions);
        if HST_DATASET_CONDITION_OK {
            assert_eq!(found, vec!["ICK909030"]);
        }
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_jwst_date_obs_server_sort() {
        let dir = tempfile::tempdir().unwrap();
        let hub = live_hub(dir.path());
        let endpoint = hub.endpoint(MastMissionId::Jwst, OpKind::Search).unwrap();
        let around = SearchParams::new()
            .target("274.688 -13.792")
            .radius(3.0, "arcminutes")
            .page(0, 60)
            .select(["fileSetName", "date_obs"])
            .sort("date_obs", true);
        let programme = SearchParams::new()
            .page(0, 60)
            .select(["fileSetName", "date_obs"])
            .condition("program", 2739)
            .condition("productLevel", "3")
            .sort("date_obs", true);
        let started = Instant::now();
        let near: SearchResults<FieldMap> = endpoint.search(&around).await.unwrap();
        let near_ms = elapsed_ms(started);
        let started = Instant::now();
        let prog: SearchResults<FieldMap> = endpoint.search(&programme).await.unwrap();
        let prog_ms = elapsed_ms(started);
        let near_dates = observed_dates(&near);
        let prog_dates = observed_dates(&prog);
        eprintln!(
            "live_jwst_date_obs_server_sort: cone rows {} sorted {} first {:?} ({} ms) echo {:?} {:?}; programme rows {} sorted {} first {:?} ({} ms)",
            near_dates.len(),
            descending(&near_dates),
            near_dates.iter().take(6).collect::<Vec<_>>(),
            near_ms,
            near.search_params.sort_by,
            near.search_params.sort_desc,
            prog_dates.len(),
            descending(&prog_dates),
            prog_dates.iter().take(6).collect::<Vec<_>>(),
            prog_ms
        );
        assert!(near_dates.len() > 10 && prog_dates.len() > 5);
        assert_eq!(descending(&near_dates) && descending(&prog_dates), JWST_DATE_SORT_OK);
    }

    fn live_value(row: &FieldMap, column: &str) -> String {
        match row.get(column) {
            Some(Value::String(text)) => text.trim().to_ascii_lowercase(),
            Some(other) => other.to_string(),
            None => String::new(),
        }
    }

    async fn live_preset_holds(endpoint: &MissionEndpoint, id: &str, program: i64) -> bool {
        let preset = crate::core::mast::presets::find(MastMissionId::Jwst, id).unwrap();
        let query = MastQuery {
            mission: MastMissionId::Jwst,
            where_: MastWhere::None,
            program: Some(program),
            dataset_id: None,
            filters: MastFilterValues { level: Some(LevelFilter::All), ..Default::default() },
            preset: Some(id.to_owned()),
            sort: Vec::new(),
            offset: 0,
            limit: PAGE_LIMIT,
        };
        let valid = validate(&query).unwrap();
        let params = build_params(&valid).select(preset.conditions.iter().map(|(column, _)| *column));
        let started = Instant::now();
        let hit: SearchResults<FieldMap> = endpoint.search(&params).await.unwrap();
        let hit_ms = elapsed_ms(started);
        let rows = hit.results.len();
        let all_match = hit.results.iter().all(|row| {
            preset.conditions.iter().all(|(column, value)| {
                let found = live_value(row, column);
                value.split(',').any(|part| part.eq_ignore_ascii_case(&found))
            })
        });
        let total = hit.total_results;
        let page = normalize_page(
            MastMissionId::Jwst,
            hit,
            PageRequest { request_id: "live:preset", query: &valid, request_body: Value::Null, elapsed_ms: hit_ms },
        );
        let mut controls = Vec::new();
        for (column, _) in preset.conditions {
            let control = SearchParams::new().page(0, 50).select(["fileSetName"]).condition("program", program).condition(*column, "zz_none");
            let started = Instant::now();
            let none: SearchResults<FieldMap> = endpoint.search(&control).await.unwrap();
            controls.push((*column, none.results.len(), none.total_results, elapsed_ms(started)));
        }
        eprintln!(
            "live_jwst_presets {id} (programme {program}): rows {rows} total {total} all_match {all_match} dropped {:?} echo {:?} ({hit_ms} ms); controls (column, rows, total, ms) {controls:?}",
            page.dropped_conditions, page.applied.conditions
        );
        rows > 0 && all_match && page.dropped_conditions.is_empty() && controls.iter().all(|(_, rows, _, _)| *rows == 0)
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_jwst_presets() {
        let dir = tempfile::tempdir().unwrap();
        let hub = live_hub(dir.path());
        let endpoint: MissionEndpoint = hub.endpoint(MastMissionId::Jwst, OpKind::Search).unwrap();
        let mut failed = Vec::new();
        for (id, program) in [
            ("tso", 1366),
            ("ifu_cubes", 1266),
            ("slit_spectra", 2736),
            ("psf_calibration", 1386),
            ("pure_parallel", 2514),
            ("pipeline_catalogs", 2739),
        ] {
            if !live_preset_holds(&endpoint, id, program).await {
                failed.push(id);
            }
        }
        assert_eq!(failed, Vec::<&str>::new());
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_server_error_reaches_the_card_with_what_mast_said() {
        let dir = tempfile::tempdir().unwrap();
        let hub = live_hub(dir.path());
        let started = Instant::now();
        let error = thumbnail(&hub, "live:thumb:ICK909C1Q", MastMissionId::Hst, "ICK909C1Q", MastLevelClass::Unknown, None).await.unwrap_err();
        eprintln!("live_server_error_reaches_the_card_with_what_mast_said: {error:?} ({} ms)", elapsed_ms(started));
        assert_eq!(error.code, MastErrorCode::Server);
        assert_eq!(error.context.status, Some(500));
        assert_eq!(error.context.detail.as_deref(), Some("Internal Server Error"));
        assert_eq!(error.context.attempts, Some(2));
        assert!(!dir.path().join("lists").exists());
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_name_lookup_m16() {
        let dir = tempfile::tempdir().unwrap();
        let hub = live_hub(dir.path());
        let raw: Value = hub
            .anon_client(OpKind::Lookup)
            .post_form(crate::infra::mast::lookup::LOOKUP_PATH, &[("request", &crate::infra::mast::lookup::lookup_request("M16"))])
            .await
            .unwrap();
        let entry = &raw["resolvedCoordinate"][0];
        let keys: Vec<&str> = entry.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
        eprintln!("live_name_lookup_m16 raw keys {keys:?} entry {entry}");
        for key in ["ra", "decl", "canonicalName", "resolver", "objectType"] {
            assert!(entry.get(key).is_some(), "{key} missing");
        }
        let started = Instant::now();
        let resolved = resolve(&hub, "M16").await.unwrap().unwrap();
        eprintln!("live_name_lookup_m16 resolved {resolved:?} ({} ms)", elapsed_ms(started));
        assert_eq!(resolved.canonical_name, "M 16");
        assert!((resolved.ra_deg - 274.688).abs() < 0.01 && (resolved.dec_deg + 13.792).abs() < 0.01);
        assert!(!resolved.resolver.is_empty());
        assert!(resolve(&hub, "M16").await.unwrap().unwrap().from_cache);
    }

    async fn live_products_mapping(id: MastMissionId) {
        let dir = tempfile::tempdir().unwrap();
        let hub = live_hub(dir.path());
        let started = Instant::now();
        let found: SearchResults<FieldMap> = hub.endpoint(id, OpKind::Search).unwrap().search(&SearchParams::new().page(0, 50)).await.unwrap();
        let search_ms = elapsed_ms(started);
        let ids: Vec<String> = found.results.iter().enumerate().map(|(i, raw)| normalize_row(id, raw, i as i64).dataset_id).collect();
        let invalid: Vec<&String> = ids.iter().filter(|v| validate_dataset_id(v).is_err()).collect();
        let listings = hub.endpoint(id, OpKind::Products).unwrap();
        let mut listed = Vec::new();
        for dataset in ids.iter().filter(|v| validate_dataset_id(v).is_ok()).take(3) {
            let started = Instant::now();
            let page: DatasetProducts<DatasetProduct> = listings.post_list_products(&DatasetIds::new([dataset.clone()])).await.unwrap();
            let list = classify_products(id, std::slice::from_ref(dataset), page.products);
            let classes: Vec<(MastProductClass, usize)> = list.groups.iter().map(|g| (g.cls, g.count)).collect();
            eprintln!(
                "live_products_mapping {id:?}: {dataset} -> {} products {classes:?}, first uri {:?}, owner {:?} ({} ms)",
                list.total_count,
                list.products.first().map(|p| p.mast_uri.as_str()),
                list.products.first().map(|p| p.dataset_id.as_str()),
                elapsed_ms(started)
            );
            listed.push(list.total_count);
        }
        eprintln!(
            "live_products_mapping {id:?}: rows {} total {} first ids {:?} invalid {} {:?} (search {search_ms} ms)",
            ids.len(),
            found.total_results,
            ids.iter().take(5).collect::<Vec<_>>(),
            invalid.len(),
            invalid.iter().take(5).collect::<Vec<_>>()
        );
        let works = !ids.is_empty() && invalid.is_empty() && !listed.is_empty() && listed.iter().all(|count| *count > 0);
        assert_eq!(works, mission(id).capabilities.products, "{id:?}");
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_iue_products_mapping() {
        live_products_mapping(MastMissionId::Iue).await;
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_classy_products_mapping() {
        live_products_mapping(MastMissionId::Classy).await;
    }

    #[tokio::test]
    #[ignore = "live MAST request"]
    async fn live_ullyses_products_mapping() {
        live_products_mapping(MastMissionId::Ullyses).await;
    }
}
