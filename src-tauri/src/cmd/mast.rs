use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_fs::FsExt;

use crate::core::mast::errors::{bad_input, internal};
use crate::core::mast::types::{
    MastCacheInfo, MastCacheKind, MastCmdError, MastErrorCode, MastImageRef, MastImageSet, MastLevelClass, MastMission, MastMissionId,
    MastPreviewRef, MastProductList, MastProgressEvent, MastQuery, MastResolved, MastSearchPage, MastTokenStatus, PreviewSize,
};
use crate::infra::mast::auth::TokenStore;
use crate::infra::mast::cache::cache_root;
use crate::infra::mast::settings::{self, MastSettings};
use crate::infra::mast::{service, MastHub};

pub const PROGRESS_EVENT: &str = "mast:progress";
pub const RESTORE_REQUEST_ID: &str = "token:restore";
const REQUEST_ID: &str = "requestId";
const REASON_LIMIT: usize = 200;
const SECRET_REASON: &str = "must be a string";

pub struct Arg<T>(Result<T, String>);

impl<'de, T: DeserializeOwned> Deserialize<'de> for Arg<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Option::<Value>::deserialize(deserializer)?.unwrap_or(Value::Null);
        Ok(Self(serde_json::from_value(raw).map_err(|error| error.to_string())))
    }
}

impl<T> Arg<T> {
    pub fn get(self, field: &str) -> Result<T, MastCmdError> {
        self.0.map_err(|reason| bad_input(field, &reason.chars().take(REASON_LIMIT).collect::<String>()))
    }

    pub fn get_secret(self, field: &str) -> Result<T, MastCmdError> {
        self.0.map_err(|_| bad_input(field, SECRET_REASON))
    }
}

pub struct MastState(Result<Arc<MastHub>, MastCmdError>);

impl MastState {
    pub fn open(settings: MastSettings, local_data: Option<PathBuf>, store: Option<TokenStore>) -> Self {
        let Some(local_data) = local_data else {
            log::warn!("MAST is unavailable: no local data folder");
            return Self(Err(MastCmdError::new(MastErrorCode::Cache, "the local data folder is not available")));
        };
        let root = cache_root(&local_data, &settings);
        match MastHub::new(settings, root) {
            Ok(hub) => Self(Ok(Arc::new(match store {
                Some(store) => hub.with_token_store(store),
                None => hub,
            }))),
            Err(error) => {
                log::warn!("MAST is unavailable: {error}");
                Self(Err(MastCmdError::new(MastErrorCode::Network, "the MAST client could not be created")))
            }
        }
    }

    pub fn hub(&self) -> Result<Arc<MastHub>, MastCmdError> {
        self.0.clone()
    }
}

pub fn install(app: &tauri::App) {
    let store = match TokenStore::system() {
        Ok(store) => Some(store),
        Err(error) => {
            log::warn!("MAST token storage is unavailable: {error}");
            None
        }
    };
    let settings = settings::settings();
    close_test_config(app, settings);
    let state = MastState::open(settings.clone(), settings.local_data_dir(app.path().app_local_data_dir().ok()), store);
    if let Ok(hub) = state.hub() {
        tauri::async_runtime::spawn(warm_up(hub));
    }
    app.manage(state);
}

fn close_test_config(app: &tauri::App, settings: &MastSettings) {
    let Some(dir) = settings.config_dir() else { return };
    let closed = std::fs::create_dir_all(&dir).and_then(|_| std::fs::canonicalize(&dir)).unwrap_or(dir);
    if let Err(error) = app.fs_scope().forbid_directory(&closed, true) {
        log::warn!("the test config folder could not be closed to the interface: {error}");
    }
}

pub async fn warm_up(hub: Arc<MastHub>) {
    let evicting = Arc::clone(&hub);
    let eviction = tokio::task::spawn_blocking(move || service::evict_cache(&evicting));
    if let Err(error) = service::token_status(&hub, RESTORE_REQUEST_ID, false).await {
        log::warn!("MAST token was not restored: {error}");
    }
    if eviction.await.is_err() {
        log::warn!("MAST cache eviction did not finish");
    }
}

async fn registered<T, Fut>(hub: Arc<MastHub>, request_id: String, work: impl FnOnce(Arc<MastHub>, String) -> Fut) -> Result<T, MastCmdError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, MastCmdError>> + Send + 'static,
{
    let task = work(Arc::clone(&hub), request_id.clone());
    hub.registry().run(&request_id, task).await
}

async fn blocking<T: Send + 'static>(job: impl FnOnce() -> Result<T, MastCmdError> + Send + 'static) -> Result<T, MastCmdError> {
    tokio::task::spawn_blocking(job).await.unwrap_or_else(|_| Err(internal()))
}

pub async fn resolve(hub: Arc<MastHub>, request_id: String, name: String) -> Result<Option<MastResolved>, MastCmdError> {
    registered(hub, request_id, |hub, _| async move { service::resolve(&hub, &name).await }).await
}

pub async fn search(hub: Arc<MastHub>, request_id: String, query: MastQuery, refresh: bool) -> Result<MastSearchPage, MastCmdError> {
    registered(hub, request_id, |hub, id| async move { service::search(&hub, &id, &query, refresh).await }).await
}

pub async fn image_list(hub: Arc<MastHub>, request_id: String, mission: MastMissionId, dataset_id: String) -> Result<MastImageSet, MastCmdError> {
    registered(hub, request_id, |hub, _| async move { service::image_list(&hub, mission, &dataset_id).await }).await
}

pub async fn thumbnail(
    hub: Arc<MastHub>,
    request_id: String,
    mission: MastMissionId,
    dataset_id: String,
    level_class: MastLevelClass,
    filename: Option<String>,
) -> Result<MastImageRef, MastCmdError> {
    registered(hub, request_id, |hub, id| async move {
        service::thumbnail(&hub, &id, mission, &dataset_id, level_class, filename.as_deref()).await
    })
    .await
}

pub async fn preview(
    hub: Arc<MastHub>,
    request_id: String,
    mission: MastMissionId,
    dataset_id: String,
    filename: String,
    size: PreviewSize,
    emit: impl Fn(MastProgressEvent) + Send + Sync + 'static,
) -> Result<MastPreviewRef, MastCmdError> {
    registered(hub, request_id, |hub, id| async move {
        service::preview(&hub, &id, mission, &dataset_id, &filename, size, &emit).await
    })
    .await
}

pub async fn products(
    hub: Arc<MastHub>,
    request_id: String,
    mission: MastMissionId,
    dataset_ids: Vec<String>,
    refresh: bool,
) -> Result<MastProductList, MastCmdError> {
    registered(hub, request_id, |hub, id| async move { service::products(&hub, &id, mission, &dataset_ids, refresh).await }).await
}

pub fn cancel(hub: &MastHub, request_ids: Option<Vec<String>>, prefix: Option<String>) -> Result<usize, MastCmdError> {
    hub.registry().cancel(request_ids.as_deref(), prefix.as_deref())
}

pub async fn token_status(hub: Arc<MastHub>, request_id: String, check: bool) -> Result<MastTokenStatus, MastCmdError> {
    if check {
        return registered(hub, request_id, |hub, id| async move { service::token_status(&hub, &id, true).await }).await;
    }
    service::token_status(&hub, &request_id, false).await
}

pub async fn token_save(hub: Arc<MastHub>, request_id: String, token: String) -> Result<MastTokenStatus, MastCmdError> {
    registered(hub, request_id, |hub, id| async move { service::token_save(&hub, &id, &token).await }).await
}

pub async fn cache_info(hub: Arc<MastHub>) -> Result<MastCacheInfo, MastCmdError> {
    blocking(move || Ok(service::cache_info(&hub))).await
}

pub async fn cache_clear(hub: Arc<MastHub>, kind: MastCacheKind) -> Result<MastCacheInfo, MastCmdError> {
    blocking(move || service::cache_clear(&hub, kind)).await
}

#[tauri::command]
pub async fn mast_missions_cmd(state: State<'_, MastState>) -> Result<Vec<MastMission>, MastCmdError> {
    let hub = state.hub()?;
    Ok(service::missions(&hub))
}

#[tauri::command]
pub async fn mast_probe_mission_cmd(state: State<'_, MastState>, mission: Arg<MastMissionId>) -> Result<MastMission, MastCmdError> {
    let hub = state.hub()?;
    service::probe_mission(&hub, mission.get("mission")?).await
}

#[tauri::command]
pub async fn mast_resolve_cmd(state: State<'_, MastState>, request_id: Arg<String>, name: Arg<String>) -> Result<Option<MastResolved>, MastCmdError> {
    resolve(state.hub()?, request_id.get(REQUEST_ID)?, name.get("name")?).await
}

#[tauri::command]
pub async fn mast_search_cmd(
    state: State<'_, MastState>,
    request_id: Arg<String>,
    query: Arg<MastQuery>,
    refresh: Arg<bool>,
) -> Result<MastSearchPage, MastCmdError> {
    search(state.hub()?, request_id.get(REQUEST_ID)?, query.get("query")?, refresh.get("refresh")?).await
}

#[tauri::command]
pub async fn mast_image_list_cmd(
    state: State<'_, MastState>,
    request_id: Arg<String>,
    mission: Arg<MastMissionId>,
    dataset_id: Arg<String>,
) -> Result<MastImageSet, MastCmdError> {
    image_list(state.hub()?, request_id.get(REQUEST_ID)?, mission.get("mission")?, dataset_id.get("datasetId")?).await
}

#[tauri::command]
pub async fn mast_thumbnail_cmd(
    state: State<'_, MastState>,
    request_id: Arg<String>,
    mission: Arg<MastMissionId>,
    dataset_id: Arg<String>,
    level_class: Arg<MastLevelClass>,
    filename: Arg<Option<String>>,
) -> Result<MastImageRef, MastCmdError> {
    thumbnail(
        state.hub()?,
        request_id.get(REQUEST_ID)?,
        mission.get("mission")?,
        dataset_id.get("datasetId")?,
        level_class.get("levelClass")?,
        filename.get("filename")?,
    )
    .await
}

#[tauri::command]
pub async fn mast_preview_cmd(
    state: State<'_, MastState>,
    app: AppHandle,
    request_id: Arg<String>,
    mission: Arg<MastMissionId>,
    dataset_id: Arg<String>,
    filename: Arg<String>,
    size: Arg<PreviewSize>,
) -> Result<MastPreviewRef, MastCmdError> {
    let emit = move |event: MastProgressEvent| {
        let _ = app.emit(PROGRESS_EVENT, event);
    };
    preview(
        state.hub()?,
        request_id.get(REQUEST_ID)?,
        mission.get("mission")?,
        dataset_id.get("datasetId")?,
        filename.get("filename")?,
        size.get("size")?,
        emit,
    )
    .await
}

#[tauri::command]
pub async fn mast_products_cmd(
    state: State<'_, MastState>,
    request_id: Arg<String>,
    mission: Arg<MastMissionId>,
    dataset_ids: Arg<Vec<String>>,
    refresh: Arg<bool>,
) -> Result<MastProductList, MastCmdError> {
    products(state.hub()?, request_id.get(REQUEST_ID)?, mission.get("mission")?, dataset_ids.get("datasetIds")?, refresh.get("refresh")?).await
}

#[tauri::command]
pub async fn mast_cancel_cmd(
    state: State<'_, MastState>,
    request_ids: Arg<Option<Vec<String>>>,
    prefix: Arg<Option<String>>,
) -> Result<usize, MastCmdError> {
    let hub = state.hub()?;
    cancel(&hub, request_ids.get("requestIds")?, prefix.get("prefix")?)
}

#[tauri::command]
pub async fn mast_token_status_cmd(state: State<'_, MastState>, request_id: Arg<String>, check: Arg<bool>) -> Result<MastTokenStatus, MastCmdError> {
    token_status(state.hub()?, request_id.get(REQUEST_ID)?, check.get("check")?).await
}

#[tauri::command]
pub async fn mast_token_save_cmd(state: State<'_, MastState>, request_id: Arg<String>, token: Arg<String>) -> Result<MastTokenStatus, MastCmdError> {
    token_save(state.hub()?, request_id.get(REQUEST_ID)?, token.get_secret("token")?).await
}

#[tauri::command]
pub async fn mast_token_clear_cmd(state: State<'_, MastState>) -> Result<MastTokenStatus, MastCmdError> {
    let hub = state.hub()?;
    service::token_clear(&hub).await
}

#[tauri::command]
pub async fn mast_cache_info_cmd(state: State<'_, MastState>) -> Result<MastCacheInfo, MastCmdError> {
    cache_info(state.hub()?).await
}

#[tauri::command]
pub async fn mast_cache_clear_cmd(state: State<'_, MastState>, kind: Arg<MastCacheKind>) -> Result<MastCacheInfo, MastCmdError> {
    cache_clear(state.hub()?, kind.get("kind")?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::query::PAGE_LIMIT;
    use crate::core::mast::types::{MastErrorCode, MastFilterValues, MastWhere, ProgressStage};
    use crate::infra::mast::cache::dir_info;
    use crate::infra::mast::settings::{CACHE_DIR, ENV_MAST_AUTH_URL, ENV_MAST_TIMEOUT_MS, ENV_MAST_URL, TEST_CACHE_DIR};
    use crate::infra::mast::testing::{serve_routes, Route, TestServer, MARKER_TOKEN};
    use crate::infra::mast::SecretToken;
    use regex::Regex;
    use serde_json::json;
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    const MAST_COMMANDS: [&str; 14] = [
        "mast_missions_cmd",
        "mast_probe_mission_cmd",
        "mast_resolve_cmd",
        "mast_search_cmd",
        "mast_image_list_cmd",
        "mast_thumbnail_cmd",
        "mast_preview_cmd",
        "mast_products_cmd",
        "mast_cancel_cmd",
        "mast_token_status_cmd",
        "mast_token_save_cmd",
        "mast_token_clear_cmd",
        "mast_cache_info_cmd",
        "mast_cache_clear_cmd",
    ];

    const CONTRACT_ARGS: [(&str, &[&str]); 14] = [
        ("mast_missions_cmd", &[]),
        ("mast_probe_mission_cmd", &["mission"]),
        ("mast_resolve_cmd", &["requestId", "name"]),
        ("mast_search_cmd", &["requestId", "query", "refresh"]),
        ("mast_image_list_cmd", &["requestId", "mission", "datasetId"]),
        ("mast_thumbnail_cmd", &["requestId", "mission", "datasetId", "levelClass", "filename"]),
        ("mast_preview_cmd", &["requestId", "mission", "datasetId", "filename", "size"]),
        ("mast_products_cmd", &["requestId", "mission", "datasetIds", "refresh"]),
        ("mast_cancel_cmd", &["requestIds", "prefix"]),
        ("mast_token_status_cmd", &["requestId", "check"]),
        ("mast_token_save_cmd", &["requestId", "token"]),
        ("mast_token_clear_cmd", &[]),
        ("mast_cache_info_cmd", &[]),
        ("mast_cache_clear_cmd", &["kind"]),
    ];

    const SLOW: Duration = Duration::from_secs(20);
    const L3: &str = "jw02739-o001_t001_nircam";
    const ASSOCIATION: &str = "ICK909030";

    fn camel(name: &str) -> String {
        let mut out = String::new();
        let mut upper = false;
        for c in name.chars() {
            if c == '_' {
                upper = true;
            } else if upper {
                out.push(c.to_ascii_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
        }
        out
    }

    fn defined_commands() -> Vec<(String, Vec<(String, String)>)> {
        let signature = Regex::new(r"#\[tauri::command\]\s*pub async fn (\w+)\(([^)]*)\)").unwrap();
        let param = Regex::new(r"(\w+): (\w+)").unwrap();
        signature
            .captures_iter(include_str!("mast.rs"))
            .map(|found| {
                let params = param.captures_iter(&found[2]).map(|p| (p[1].to_owned(), p[2].to_owned())).collect();
                (found[1].to_owned(), params)
            })
            .collect()
    }

    fn assert_wiring(lib: &str, cmd_mod: &str) {
        let lib = lib.replace("\r\n", "\n");
        let cmd_mod = cmd_mod.replace("\r\n", "\n");
        let gated = Regex::new(r#"#\[cfg\(feature = "mast"\)\]\s*cmd::mast::(mast_\w+),"#).unwrap();
        let registered: Vec<String> = gated.captures_iter(&lib).map(|found| found[1].to_owned()).collect();
        assert_eq!(registered, MAST_COMMANDS);
        assert_eq!(lib.matches("cmd::mast::mast_").count(), MAST_COMMANDS.len());
        assert!(lib.contains("#[cfg(feature = \"mast\")]\n    crate::infra::mast::settings::init_from_env();"));
        assert!(lib.contains("#[cfg(feature = \"mast\")]\n            cmd::mast::install(app);"));
        assert!(cmd_mod.contains("#[cfg(feature = \"mast\")]\npub mod mast;"));
    }

    #[test]
    fn command_names_match_contract() {
        let contract: Vec<&str> = CONTRACT_ARGS.iter().map(|(name, _)| *name).collect();
        assert_eq!(contract, MAST_COMMANDS);
        let defined: Vec<String> = defined_commands().into_iter().map(|(name, _)| name).collect();
        assert_eq!(defined, MAST_COMMANDS);
        assert_wiring(include_str!("../lib.rs"), include_str!("mod.rs"));
    }

    #[test]
    fn wiring_check_holds_on_crlf_checkouts() {
        let crlf = |text: &str| text.replace("\r\n", "\n").replace('\n', "\r\n");
        assert_wiring(&crlf(include_str!("../lib.rs")), &crlf(include_str!("mod.rs")));
    }

    #[test]
    fn command_args_match_contract() {
        for ((name, params), (contract_name, contract_args)) in defined_commands().into_iter().zip(CONTRACT_ARGS) {
            assert_eq!(name, contract_name);
            let args: Vec<&(String, String)> = params.iter().filter(|(_, kind)| kind != "State" && kind != "AppHandle").collect();
            let keys: Vec<String> = args.iter().map(|(param, _)| camel(param)).collect();
            assert_eq!(keys, contract_args.iter().map(|key| key.to_string()).collect::<Vec<_>>(), "{name}");
            for (param, kind) in args {
                assert_eq!(kind, "Arg", "{name}: {param} must reject a wrong shape with mast_bad_input");
            }
            assert_eq!(params.iter().filter(|(_, kind)| kind == "State").count(), 1, "{name}");
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Sample {
        request_id: Arg<String>,
        mission: Arg<MastMissionId>,
        filename: Arg<Option<String>>,
        request_ids: Arg<Option<Vec<String>>>,
        token: Arg<String>,
    }

    fn sample(value: serde_json::Value) -> Sample {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn args_of_the_wrong_shape_reject_with_bad_input() {
        let good = sample(json!({
            "requestId": "g1:search",
            "mission": "roman_cgi",
            "filename": "a_thumb.jpg",
            "requestIds": ["g1:a", "g1:b"],
            "token": MARKER_TOKEN
        }));
        assert_eq!(good.request_id.get(REQUEST_ID).unwrap(), "g1:search");
        assert_eq!(good.mission.get("mission").unwrap(), MastMissionId::RomanCgi);
        assert_eq!(good.filename.get("filename").unwrap().as_deref(), Some("a_thumb.jpg"));
        assert_eq!(good.request_ids.get("requestIds").unwrap(), Some(vec!["g1:a".to_owned(), "g1:b".to_owned()]));
        assert_eq!(good.token.get_secret("token").unwrap(), MARKER_TOKEN);

        let absent = sample(json!({"filename": null, "requestIds": null}));
        for error in [absent.request_id.get(REQUEST_ID).unwrap_err(), absent.mission.get("mission").unwrap_err()] {
            assert_eq!(error.code, MastErrorCode::BadInput);
            assert!(error.message.starts_with("requestId: ") || error.message.starts_with("mission: "), "{}", error.message);
        }
        assert_eq!(absent.filename.get("filename").unwrap(), None);
        assert_eq!(absent.request_ids.get("requestIds").unwrap(), None);
        assert_eq!(absent.token.get_secret("token").unwrap_err().message, "token: must be a string");
        let missing = sample(json!({}));
        assert_eq!(missing.filename.get("filename").unwrap(), None);
        assert!(missing.request_id.get(REQUEST_ID).unwrap_err().message.starts_with("requestId: "));

        let wrong = sample(json!({
            "requestId": 7,
            "mission": "tess2",
            "filename": ["a"],
            "requestIds": "g1:",
            "token": 98765432109876543u64
        }));
        let error = wrong.mission.get("mission").unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput);
        assert!(error.message.starts_with("mission: ") && error.message.contains("tess2"), "{}", error.message);
        assert!(error.message.chars().count() <= "mission: ".len() + 200, "{}", error.message);
        assert_eq!(error.context, Default::default());
        assert!(wrong.request_id.get(REQUEST_ID).unwrap_err().message.starts_with("requestId: "));
        assert!(wrong.filename.get("filename").unwrap_err().message.starts_with("filename: "));
        assert!(wrong.request_ids.get("requestIds").unwrap_err().message.starts_with("requestIds: "));
        let secret = wrong.token.get_secret("token").unwrap_err();
        assert_eq!(secret.code, MastErrorCode::BadInput);
        assert_eq!(secret.message, "token: must be a string");
        let nested = sample(json!({"token": {"value": MARKER_TOKEN}}));
        assert!(!nested.token.get_secret("token").unwrap_err().message.contains(MARKER_TOKEN));
    }

    fn settings_for(base: &str) -> MastSettings {
        let base = base.to_owned();
        MastSettings::from_lookup(move |name| match name {
            ENV_MAST_URL | ENV_MAST_AUTH_URL => Some(base.clone()),
            ENV_MAST_TIMEOUT_MS => Some("30000".to_owned()),
            _ => None,
        })
    }

    struct Rig {
        server: TestServer,
        cache: TempDir,
        config: TempDir,
        hub: Arc<MastHub>,
    }

    async fn rig(routes: Vec<Route>) -> Rig {
        let server = serve_routes(routes).await;
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let state = MastState::open(settings_for(&server.base), Some(cache.path().to_path_buf()), Some(TokenStore::new(config.path())));
        let hub = state.hub().unwrap();
        Rig { server, cache, config, hub }
    }

    fn store_marker(config: &Path) {
        TokenStore::new(config).save(&SecretToken::new(MARKER_TOKEN).unwrap()).unwrap();
    }

    fn present(authorized: Option<bool>) -> MastTokenStatus {
        MastTokenStatus { present: true, authorized }
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name)).unwrap()
    }

    fn fixture_bytes(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name)).unwrap()
    }

    async fn wait_until(hub: &MastHub, count: usize) {
        for _ in 0..400 {
            if hub.registry().len() == count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("expected {count} registered tasks, found {}", hub.registry().len());
    }

    fn m16() -> MastQuery {
        MastQuery {
            mission: MastMissionId::Jwst,
            where_: MastWhere::Position { ra_deg: 274.688, dec_deg: -13.792, radius_arcmin: 3.0, label: None },
            program: None,
            dataset_id: None,
            filters: MastFilterValues::default(),
            sort: Vec::new(),
            offset: 0,
            limit: PAGE_LIMIT,
        }
    }

    #[tokio::test]
    async fn state_follows_the_url_override_and_reports_a_missing_folder() {
        let missing = MastState::open(MastSettings::default(), None, None).hub().unwrap_err();
        assert_eq!(missing.code, MastErrorCode::Cache);
        assert!(missing.message.contains("local data folder"), "{}", missing.message);

        let local = tempfile::tempdir().unwrap();
        let real = MastState::open(MastSettings::default(), Some(local.path().to_path_buf()), None).hub().unwrap();
        assert_eq!(real.cache_root(), local.path().join(CACHE_DIR));
        let no_store = token_status(Arc::clone(&real), "token:check".to_owned(), false).await.unwrap_err();
        assert_eq!(no_store.code, MastErrorCode::Cache);

        let canned = MastState::open(settings_for("http://127.0.0.1:9"), Some(local.path().to_path_buf()), None).hub().unwrap();
        assert_eq!(canned.cache_root(), local.path().join(TEST_CACHE_DIR));
        assert_eq!(canned.settings().base_url, "http://127.0.0.1:9");
        assert!(std::fs::read_dir(local.path()).unwrap().next().is_none(), "opening the state writes nothing");
    }

    #[tokio::test]
    async fn registered_commands_are_cancelled_by_id_or_prefix() {
        let rig = rig(vec![
            Route::get("/info").delay(SLOW).json(r#"{"anon":false}"#),
            Route::get("/search/").delay(SLOW).json("{}"),
            Route::post("/search/").delay(SLOW).json("{}"),
            Route::post("/api/v0/invoke").delay(SLOW).json("{}"),
        ])
        .await;
        store_marker(rig.config.path());
        let hub = &rig.hub;
        let check = tokio::spawn(token_status(Arc::clone(hub), "token:check".to_owned(), true));
        wait_until(hub, 1).await;
        let save = tokio::spawn(token_save(Arc::clone(hub), "token:save".to_owned(), MARKER_TOKEN.to_owned()));
        let lookup = tokio::spawn(resolve(Arc::clone(hub), "g7:resolve:m16".to_owned(), "M16".to_owned()));
        let page = tokio::spawn(search(Arc::clone(hub), "g7:search".to_owned(), m16(), false));
        let list = tokio::spawn(image_list(Arc::clone(hub), "g7:list:a".to_owned(), MastMissionId::Jwst, L3.to_owned()));
        let thumb = tokio::spawn(thumbnail(Arc::clone(hub), "g7:thumb:a".to_owned(), MastMissionId::Jwst, L3.to_owned(), MastLevelClass::Combined, None));
        let listing = tokio::spawn(products(Arc::clone(hub), "g7:products:a".to_owned(), MastMissionId::Jwst, vec![L3.to_owned()], false));
        wait_until(hub, 7).await;
        for id in ["token:check", "token:save", "g7:resolve:m16", "g7:search", "g7:list:a", "g7:thumb:a", "g7:products:a"] {
            assert!(hub.registry().contains(id), "{id}");
        }
        assert_eq!(cancel(hub, Some(vec!["token:save".to_owned(), "token:check".to_owned()]), Some("g7:".to_owned())).unwrap(), 7);
        let codes = [
            check.await.unwrap().unwrap_err().code,
            save.await.unwrap().unwrap_err().code,
            lookup.await.unwrap().unwrap_err().code,
            page.await.unwrap().unwrap_err().code,
            list.await.unwrap().unwrap_err().code,
            thumb.await.unwrap().unwrap_err().code,
            listing.await.unwrap().unwrap_err().code,
        ];
        assert_eq!(codes, [MastErrorCode::Cancelled; 7]);
        assert!(hub.registry().is_empty());
        assert_eq!(cancel(hub, None, None).unwrap(), 0);
        assert_eq!(cancel(hub, None, Some("g7".to_owned())).unwrap_err().code, MastErrorCode::BadInput);
        assert_eq!(std::fs::read_to_string(rig.config.path().join("mast_api_key.txt")).unwrap(), MARKER_TOKEN);
        assert!(rig.server.requests().iter().all(|r| r.path() != "/search/roman/api/v0.1/auth_check"));
    }

    #[tokio::test]
    async fn stored_status_read_never_aborts_a_running_check() {
        let rig = rig(vec![
            Route::get("/info").delay(Duration::from_millis(400)).json(r#"{"anon":false}"#),
            Route::get("/search/roman/api/v0.1/auth_check").json(r#"{"authorized":true}"#),
        ])
        .await;
        store_marker(rig.config.path());
        let check = tokio::spawn(token_status(Arc::clone(&rig.hub), "token:check".to_owned(), true));
        wait_until(&rig.hub, 1).await;
        let read = token_status(Arc::clone(&rig.hub), "token:check".to_owned(), false).await.unwrap();
        assert_eq!(check.await.unwrap().unwrap(), present(Some(true)));
        assert_eq!(read, present(Some(true)));
        assert!(rig.hub.registry().is_empty());
        let bad = token_status(Arc::clone(&rig.hub), "bad id".to_owned(), false).await.unwrap_err();
        assert_eq!(bad.code, MastErrorCode::BadInput);
        assert_eq!(rig.server.count("/info"), 1);
    }

    #[tokio::test]
    async fn preview_command_emits_progress_events_and_cancels_by_id() {
        let head = fixture_bytes("prev_hst_head_sos.bin");
        let mut routes = vec![Route::get("/search/hst/api/v0.1/get_image_preview?filename=ick909030_drc.jpg").stall(head, 5_000_000)];
        for category in ["THUMBNAIL", "PREVIEW"] {
            routes.push(
                Route::get(&format!("/search/hst/api/v0.1/get_image_list?fileset_name={ASSOCIATION}&category={category}"))
                    .json(fixture(&format!("il_{ASSOCIATION}_{category}.json"))),
            );
        }
        let rig = rig(routes).await;
        let events = Arc::new(std::sync::Mutex::new(Vec::<MastProgressEvent>::new()));
        let emit = {
            let events = Arc::clone(&events);
            move |event: MastProgressEvent| events.lock().unwrap().push(event)
        };
        let id = "g4:preview:ICK909030:ick909030_drc.jpg:pane";
        let task = tokio::spawn(preview(
            Arc::clone(&rig.hub),
            id.to_owned(),
            MastMissionId::Hst,
            ASSOCIATION.to_owned(),
            "ick909030_drc.jpg".to_owned(),
            PreviewSize::Pane,
            emit,
        ));
        for _ in 0..400 {
            if !events.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let first = events.lock().unwrap().first().cloned().expect("a progress event");
        assert_eq!(first.request_id, id);
        assert_eq!(first.stage, ProgressStage::Download);
        assert_eq!(first.total, Some(5_000_000));
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            json!({"requestId": id, "received": first.received, "total": 5_000_000, "stage": "download"})
        );
        assert_eq!(cancel(&rig.hub, Some(vec![id.to_owned()]), None).unwrap(), 1);
        assert_eq!(task.await.unwrap().unwrap_err().code, MastErrorCode::Cancelled);
        assert!(rig.hub.registry().is_empty());
        let folder = rig.cache.path().join(TEST_CACHE_DIR).join("img").join("hst").join(ASSOCIATION);
        let left: Vec<_> = std::fs::read_dir(&folder).map(|entries| entries.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
    }

    fn write_aged(path: &Path, bytes: usize, seconds_ago: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago)).unwrap();
    }

    #[tokio::test]
    async fn warm_up_evicts_over_cap_and_restores_the_token_without_a_request() {
        let server = serve_routes(Vec::new()).await;
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        store_marker(config.path());
        let root = cache.path().join(TEST_CACHE_DIR);
        let hub = Arc::new(
            MastHub::new(settings_for(&server.base), root.clone())
                .unwrap()
                .with_token_store(TokenStore::new(config.path()))
                .with_disk_caps(1000, 1000),
        );
        let images = root.join("img").join("jwst").join(L3);
        write_aged(&images.join("old_thumb.jpg"), 600, 300);
        write_aged(&images.join("mid_thumb.jpg"), 600, 200);
        write_aged(&images.join("new_thumb.jpg"), 600, 100);
        let lists = root.join("lists").join("jwst");
        write_aged(&lists.join("a.images.json"), 700, 300);
        write_aged(&lists.join("b.images.json"), 700, 100);
        let foreign = u32::MAX - 1;
        let stale = [images.join(format!("dl-{foreign}-0.part")), root.join(format!("names.json.{foreign}.tmp"))];
        for path in &stale {
            write_aged(path, 100, 10);
        }
        let own = images.join(format!("dl-{}-0.part", std::process::id()));
        write_aged(&own, 100, 0);
        assert!(!hub.has_token_client());
        warm_up(Arc::clone(&hub)).await;
        assert!(stale.iter().all(|path| !path.exists()), "stale temporary files survived the warm-up");
        assert!(own.is_file(), "a part file of this process was swept");
        assert!(dir_info(&root.join("img")).bytes <= 1000);
        assert!(images.join("new_thumb.jpg").is_file());
        assert!(!images.join("old_thumb.jpg").exists());
        assert!(dir_info(&root.join("lists")).bytes <= 1000);
        assert!(lists.join("b.images.json").is_file());
        assert!(hub.has_token_client());
        assert_eq!(hub.auth_session().authorized(), None);
        assert!(server.requests().is_empty());

        let bare = Arc::new(MastHub::new(settings_for(&server.base), cache.path().join("other")).unwrap());
        warm_up(Arc::clone(&bare)).await;
        assert!(!bare.has_token_client());
        assert!(server.requests().is_empty());
    }

    #[tokio::test]
    async fn cache_commands_report_and_clear_by_kind() {
        let rig = rig(Vec::new()).await;
        let root = rig.hub.cache_root().to_path_buf();
        write_aged(&root.join("img").join("hst").join(ASSOCIATION).join("a_thumb.jpg"), 10, 10);
        write_aged(&root.join("names.json"), 5, 10);
        assert_eq!(cache_info(Arc::clone(&rig.hub)).await.unwrap(), MastCacheInfo { bytes: 15, files: 2 });
        assert_eq!(cache_clear(Arc::clone(&rig.hub), MastCacheKind::Images).await.unwrap(), MastCacheInfo { bytes: 10, files: 1 });
        assert_eq!(cache_info(Arc::clone(&rig.hub)).await.unwrap(), MastCacheInfo { bytes: 5, files: 1 });
        assert_eq!(cache_clear(Arc::clone(&rig.hub), MastCacheKind::All).await.unwrap(), MastCacheInfo { bytes: 5, files: 1 });
        assert_eq!(cache_info(Arc::clone(&rig.hub)).await.unwrap(), MastCacheInfo::default());
        assert!(rig.server.requests().is_empty());
    }
}
