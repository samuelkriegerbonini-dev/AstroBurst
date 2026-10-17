use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::Semaphore;

use super::api::MissionEndpoint;
use super::auth::{AuthSession, TokenStore};
use super::cache::{
    DiskBudget, MemoryLru, IMAGE_CAP_BYTES, IMAGE_SET_ENTRIES, LIST_CAP_BYTES, LIST_TTL, PRODUCT_LIST_ENTRIES, SEARCH_CACHE_ENTRIES,
    SEARCH_CACHE_TTL,
};
use super::client::{MastClient, RedirectRules};
use super::error::MastResult;
use super::lookup::NameCache;
use super::ratelimit::RateLimiter;
use super::registry::Registry;
use super::settings::MastSettings;
use crate::core::mast::errors::{unauthorized_without_token, OpKind};
use crate::core::mast::types::{
    MastAvailability, MastCmdError, MastErrorCode, MastImageSet, MastLevelClass, MastMissionId, MastProductList, MastRow, MastSearchPage,
};

pub const THUMBNAIL_PERMITS: usize = 4;
pub const LIST_PERMITS: usize = 2;
pub const PREVIEW_PERMITS: usize = 1;
pub const DECODE_PERMITS: usize = 1;
pub const PRODUCT_PERMITS: usize = 1;
pub const ROW_HINT_ENTRIES: usize = 2000;
pub const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

pub type DatasetKey = (MastMissionId, String);
pub type StoredProducts = (MastProductList, SystemTime);
type FlightSlot = Arc<tokio::sync::Mutex<Option<MastCmdError>>>;
type Flights = Mutex<HashMap<String, FlightSlot>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub availability: MastAvailability,
    pub checked_on: String,
}

pub struct MastHub {
    settings: MastSettings,
    anon: MastClient,
    auth: Mutex<AuthSession>,
    auth_serial: tokio::sync::Mutex<()>,
    token_store: Option<TokenStore>,
    cache_root: PathBuf,
    registry: Registry,
    limiter: RateLimiter,
    searches: Mutex<MemoryLru<String, MastSearchPage>>,
    names: NameCache,
    probes: Mutex<HashMap<MastMissionId, ProbeResult>>,
    image_sets: Mutex<MemoryLru<DatasetKey, MastImageSet>>,
    product_lists: Mutex<MemoryLru<DatasetKey, StoredProducts>>,
    row_filters: Mutex<MemoryLru<DatasetKey, String>>,
    chosen: Mutex<MemoryLru<DatasetKey, String>>,
    fileset_failures: Mutex<HashSet<(MastMissionId, MastLevelClass)>>,
    thumb_flights: Flights,
    thumb_permits: Semaphore,
    list_permits: Semaphore,
    card_list_turn: tokio::sync::Mutex<()>,
    preview_permits: Semaphore,
    decode_permits: Arc<Semaphore>,
    product_permits: Semaphore,
    image_budget: Arc<DiskBudget>,
    list_budget: Arc<DiskBudget>,
}

impl fmt::Debug for MastHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MastHub")
            .field("settings", &self.settings)
            .field("anon", &self.anon)
            .field("has_token_client", &self.has_token_client())
            .field("token_authorized", &self.auth_session().authorized())
            .field("token_store", &self.token_store)
            .field("cache_root", &self.cache_root)
            .field("registry", &self.registry)
            .finish()
    }
}

pub fn needs_token(mission: MastMissionId) -> bool {
    matches!(mission, MastMissionId::Roman | MastMissionId::RomanCgi)
}

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Flight<'a> {
    flights: &'a Flights,
    key: String,
    held: Option<tokio::sync::OwnedMutexGuard<Option<MastCmdError>>>,
}

impl Flight<'_> {
    pub fn earlier_failure(&self) -> Option<MastCmdError> {
        self.held.as_deref().and_then(Option::clone)
    }

    pub fn share_failure(&mut self, error: &MastCmdError) {
        if let Some(slot) = self.held.as_deref_mut() {
            *slot = Some(error.clone());
        }
    }
}

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        drop(self.held.take());
        let mut flights = guard(self.flights);
        if flights.get(&self.key).is_some_and(|lock| Arc::strong_count(lock) == 1) {
            flights.remove(&self.key);
        }
    }
}

impl MastHub {
    pub fn new(settings: MastSettings, cache_root: PathBuf) -> MastResult<Self> {
        let anon = MastClient::builder()
            .base_url(&settings.base_url)
            .redirects(RedirectRules::Allowlist(settings.redirect_allowlist()))
            .build()?;
        Ok(Self {
            settings,
            anon,
            auth: Mutex::new(AuthSession::default()),
            auth_serial: tokio::sync::Mutex::new(()),
            token_store: None,
            cache_root,
            registry: Registry::new(),
            limiter: RateLimiter::new(),
            searches: Mutex::new(MemoryLru::new(SEARCH_CACHE_ENTRIES, SEARCH_CACHE_TTL)),
            names: NameCache::default(),
            probes: Mutex::new(HashMap::new()),
            image_sets: Mutex::new(MemoryLru::new(IMAGE_SET_ENTRIES, LIST_TTL)),
            product_lists: Mutex::new(MemoryLru::new(PRODUCT_LIST_ENTRIES, LIST_TTL)),
            row_filters: Mutex::new(MemoryLru::new(ROW_HINT_ENTRIES, SESSION_TTL)),
            chosen: Mutex::new(MemoryLru::new(ROW_HINT_ENTRIES, SESSION_TTL)),
            fileset_failures: Mutex::new(HashSet::new()),
            thumb_flights: Mutex::new(HashMap::new()),
            thumb_permits: Semaphore::new(THUMBNAIL_PERMITS),
            list_permits: Semaphore::new(LIST_PERMITS),
            card_list_turn: tokio::sync::Mutex::new(()),
            preview_permits: Semaphore::new(PREVIEW_PERMITS),
            decode_permits: Arc::new(Semaphore::new(DECODE_PERMITS)),
            product_permits: Semaphore::new(PRODUCT_PERMITS),
            image_budget: Arc::new(DiskBudget::new(IMAGE_CAP_BYTES)),
            list_budget: Arc::new(DiskBudget::new(LIST_CAP_BYTES)),
        })
    }

    pub fn with_token_store(mut self, store: TokenStore) -> Self {
        self.token_store = Some(store);
        self
    }

    pub fn token_store(&self) -> Result<&TokenStore, MastCmdError> {
        self.token_store
            .as_ref()
            .ok_or_else(|| MastCmdError::new(MastErrorCode::Cache, "token storage is not available"))
    }

    pub fn auth_session(&self) -> MutexGuard<'_, AuthSession> {
        guard(&self.auth)
    }

    pub fn auth_serial(&self) -> &tokio::sync::Mutex<()> {
        &self.auth_serial
    }

    #[cfg(test)]
    pub fn thumbnails_in_flight(&self) -> usize {
        guard(&self.thumb_flights).len()
    }

    #[cfg(test)]
    pub fn with_disk_caps(mut self, images: u64, lists: u64) -> Self {
        self.image_budget = Arc::new(DiskBudget::new(images));
        self.list_budget = Arc::new(DiskBudget::new(lists));
        self
    }

    pub fn image_sets(&self) -> MutexGuard<'_, MemoryLru<DatasetKey, MastImageSet>> {
        guard(&self.image_sets)
    }

    pub fn product_lists(&self) -> MutexGuard<'_, MemoryLru<DatasetKey, StoredProducts>> {
        guard(&self.product_lists)
    }

    pub fn product_permits(&self) -> &Semaphore {
        &self.product_permits
    }

    pub fn remember_row_filters(&self, mission: MastMissionId, rows: &[MastRow]) {
        if mission != MastMissionId::Jwst {
            return;
        }
        let now = Instant::now();
        let mut filters = guard(&self.row_filters);
        for row in rows.iter().filter(|row| row.level_class == MastLevelClass::Combined) {
            if let Some(filter) = row.filter.as_deref() {
                filters.insert((mission, row.dataset_id.clone()), filter.to_owned(), now);
            }
        }
    }

    pub fn row_filter(&self, mission: MastMissionId, dataset: &str) -> Option<String> {
        guard(&self.row_filters).get(&(mission, dataset.to_owned()), Instant::now())
    }

    pub fn chosen_representative(&self, mission: MastMissionId, dataset: &str) -> Option<String> {
        guard(&self.chosen).get(&(mission, dataset.to_owned()), Instant::now())
    }

    pub fn remember_representative(&self, mission: MastMissionId, dataset: &str, stored: &str) {
        guard(&self.chosen).insert((mission, dataset.to_owned()), stored.to_owned(), Instant::now());
    }

    pub fn forget_images(&self) {
        guard(&self.chosen).clear();
    }

    pub fn fileset_failed(&self, mission: MastMissionId, level: MastLevelClass) -> bool {
        guard(&self.fileset_failures).contains(&(mission, level))
    }

    pub fn remember_fileset_failure(&self, mission: MastMissionId, level: MastLevelClass) {
        guard(&self.fileset_failures).insert((mission, level));
    }

    pub async fn thumbnail_flight(&self, mission: MastMissionId, dataset: &str, filename: Option<&str>) -> Flight<'_> {
        let key = format!("{}/{dataset}/{}", mission.slug(), filename.unwrap_or_default());
        let lock = Arc::clone(guard(&self.thumb_flights).entry(key.clone()).or_default());
        let held = lock.lock_owned().await;
        Flight { flights: &self.thumb_flights, key, held: Some(held) }
    }

    pub fn thumb_permits(&self) -> &Semaphore {
        &self.thumb_permits
    }

    pub fn list_permits(&self) -> &Semaphore {
        &self.list_permits
    }

    pub fn card_list_turn(&self) -> &tokio::sync::Mutex<()> {
        &self.card_list_turn
    }

    pub fn preview_permits(&self) -> &Semaphore {
        &self.preview_permits
    }

    pub fn decode_permits(&self) -> Arc<Semaphore> {
        Arc::clone(&self.decode_permits)
    }

    pub fn image_budget(&self) -> Arc<DiskBudget> {
        Arc::clone(&self.image_budget)
    }

    pub fn list_budget(&self) -> Arc<DiskBudget> {
        Arc::clone(&self.list_budget)
    }

    pub fn settings(&self) -> &MastSettings {
        &self.settings
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }

    pub fn names(&self) -> &NameCache {
        &self.names
    }

    pub fn searches(&self) -> MutexGuard<'_, MemoryLru<String, MastSearchPage>> {
        guard(&self.searches)
    }

    pub fn anon_client(&self, kind: OpKind) -> MastClient {
        self.anon.clone().with_timeout(self.settings.timeout(kind))
    }

    pub fn has_token_client(&self) -> bool {
        self.auth_session().roman().is_some()
    }

    pub fn set_token_client(&self, client: Option<MastClient>) {
        self.auth_session().set_roman(client);
    }

    pub fn client_for(&self, mission: MastMissionId, kind: OpKind) -> Result<MastClient, MastCmdError> {
        if !needs_token(mission) {
            return Ok(self.anon_client(kind));
        }
        let session = self.auth_session();
        match session.roman() {
            Some(client) => Ok(client.clone().with_timeout(self.settings.timeout(kind))),
            None => Err(unauthorized_without_token(mission)),
        }
    }

    pub fn endpoint(&self, mission: MastMissionId, kind: OpKind) -> Result<MissionEndpoint, MastCmdError> {
        Ok(MissionEndpoint::new(self.client_for(mission, kind)?, mission))
    }

    pub fn record_probe(&self, mission: MastMissionId, result: ProbeResult) {
        guard(&self.probes).insert(mission, result);
    }

    pub fn probes(&self) -> HashMap<MastMissionId, ProbeResult> {
        guard(&self.probes).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::mast::testing::MARKER_TOKEN;
    use std::time::Duration;

    #[test]
    fn hub_hands_out_clients_per_mission_and_never_prints_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let settings = MastSettings { timeout_override: Some(Duration::from_millis(1500)), ..Default::default() };
        let hub = MastHub::new(settings, dir.path().to_path_buf()).unwrap();
        assert_eq!(hub.cache_root(), dir.path());
        let anon = hub.client_for(MastMissionId::Jwst, OpKind::Search).unwrap();
        assert!(!anon.has_token());
        assert_eq!(anon.timeout(), Duration::from_millis(1500));
        assert_eq!(anon.base_url(), "https://mast.stsci.edu");
        for mission in [MastMissionId::Roman, MastMissionId::RomanCgi] {
            assert!(needs_token(mission));
            let error = hub.client_for(mission, OpKind::Search).unwrap_err();
            assert_eq!(error.code, crate::core::mast::types::MastErrorCode::Unauthorized);
            assert_eq!(error.context.mission, Some(mission));
        }
        assert!(!needs_token(MastMissionId::RomanSpectra));
        let token_client = MastClient::builder().token(MARKER_TOKEN).redirects(RedirectRules::Never).build().unwrap();
        hub.set_token_client(Some(token_client));
        assert!(hub.has_token_client());
        let roman = hub.endpoint(MastMissionId::Roman, OpKind::Search).unwrap();
        assert!(roman.client().has_token());
        assert!(!hub.client_for(MastMissionId::Hst, OpKind::Search).unwrap().has_token());
        let printed = format!("{hub:?}");
        assert!(!printed.contains(MARKER_TOKEN), "{printed}");
        assert!(printed.contains("has_token_client: true"));
        hub.set_token_client(None);
        assert!(hub.client_for(MastMissionId::Roman, OpKind::Search).is_err());
        hub.record_probe(MastMissionId::Tess, ProbeResult { availability: MastAvailability::Open, checked_on: "2026-10-16".into() });
        assert_eq!(hub.probes().len(), 1);
    }

    fn jwst_row(dataset: &str, level: &str, filter: Option<&str>) -> MastRow {
        let raw: crate::infra::mast::models::FieldMap =
            serde_json::from_value(serde_json::json!({"fileSetName": dataset, "productLevel": level, "filter": filter})).unwrap();
        crate::core::mast::normalize::normalize_row(MastMissionId::Jwst, &raw, 0)
    }

    #[test]
    fn hub_keeps_session_image_memory() {
        let dir = tempfile::tempdir().unwrap();
        let hub = MastHub::new(MastSettings::default(), dir.path().to_path_buf()).unwrap();
        assert_eq!(hub.thumb_permits().available_permits(), THUMBNAIL_PERMITS);
        assert_eq!(hub.list_permits().available_permits(), LIST_PERMITS);
        assert_eq!(hub.preview_permits().available_permits(), PREVIEW_PERMITS);
        assert_eq!(hub.decode_permits().available_permits(), DECODE_PERMITS);
        assert_eq!(hub.product_permits().available_permits(), PRODUCT_PERMITS);
        assert_eq!(1 + THUMBNAIL_PERMITS + LIST_PERMITS + PREVIEW_PERMITS + PRODUCT_PERMITS, 9);
        assert!(hub.product_lists().is_empty());
        assert_eq!(hub.image_budget().cap(), IMAGE_CAP_BYTES);
        assert_eq!(hub.list_budget().cap(), LIST_CAP_BYTES);

        let rows = vec![
            jwst_row("jw02739-o001_t001_nircam", "3", Some("F187N")),
            jwst_row("jw02739001002_02101_00001", "2b", Some("F200W")),
            jwst_row("jw02739-o002_t001_miri", "3", None),
        ];
        assert_eq!(rows[0].level_class, MastLevelClass::Combined);
        assert_eq!(rows[1].level_class, MastLevelClass::Exposure);
        hub.remember_row_filters(MastMissionId::Jwst, &rows);
        assert_eq!(hub.row_filter(MastMissionId::Jwst, "jw02739-o001_t001_nircam").as_deref(), Some("F187N"));
        assert_eq!(hub.row_filter(MastMissionId::Jwst, "jw02739001002_02101_00001"), None);
        assert_eq!(hub.row_filter(MastMissionId::Jwst, "jw02739-o002_t001_miri"), None);
        assert_eq!(hub.row_filter(MastMissionId::Hst, "jw02739-o001_t001_nircam"), None);

        assert!(!hub.fileset_failed(MastMissionId::Jwst, MastLevelClass::Combined));
        hub.remember_fileset_failure(MastMissionId::Jwst, MastLevelClass::Combined);
        assert!(hub.fileset_failed(MastMissionId::Jwst, MastLevelClass::Combined));
        assert!(!hub.fileset_failed(MastMissionId::Hst, MastLevelClass::Combined));
        assert!(!hub.fileset_failed(MastMissionId::Jwst, MastLevelClass::Unknown));

        assert_eq!(hub.chosen_representative(MastMissionId::Hst, "ICK909030"), None);
        hub.remember_representative(MastMissionId::Hst, "ICK909030", "ick909030_drc_thumb.jpg");
        assert_eq!(hub.chosen_representative(MastMissionId::Hst, "ICK909030").as_deref(), Some("ick909030_drc_thumb.jpg"));
        hub.forget_images();
        assert_eq!(hub.chosen_representative(MastMissionId::Hst, "ICK909030"), None);
        assert!(hub.fileset_failed(MastMissionId::Jwst, MastLevelClass::Combined), "failure memory lasts the session");
    }
}
