use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::cmd::common::blocking_cmd;
#[cfg(feature = "net")]
use crate::infra::catalog_store::catalog_client;
use crate::infra::catalog_store::{
    chosen_data_root_missing, CatalogStore, InstallObserver, InstallOutcome, InstallStage,
    StoreError, StoreRoots, FIELD_EXTERNAL_CATALOGS, FIELD_SIRIL_CATALOG_DIR,
    INSTALL_PROGRESS_EVENT,
};
use crate::infra::config;
use crate::infra::net::paths::ENV_DATA_DIR;
use crate::infra::progress::ProgressHandle;
use crate::types::config::AppConfig;

const COMPLETE_STAGE: &str = "complete";

static CATALOG_CHOICES: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize)]
pub(crate) struct InstallProgress {
    id: String,
    current: u64,
    total: u64,
    percent: u32,
    stage: String,
}

pub(crate) fn install_progress_payload(
    id: &str,
    stage: &str,
    done: u64,
    total: u64,
) -> InstallProgress {
    InstallProgress {
        id: id.to_owned(),
        current: done,
        total,
        percent: percent_of(done, total),
        stage: stage.to_owned(),
    }
}

pub(crate) fn install_cancel_key(id: &str) -> String {
    format!("{INSTALL_PROGRESS_EVENT}:{id}")
}

struct ProgressObserver {
    id: String,
    handle: ProgressHandle,
    app: tauri::AppHandle,
}

impl ProgressObserver {
    fn emit(&self, stage: &str, done: u64, total: u64) {
        let payload = install_progress_payload(&self.id, stage, done, total);
        if let Err(error) = self.app.emit(INSTALL_PROGRESS_EVENT, payload) {
            log::debug!("catalogue install progress was not delivered: {error}");
        }
    }
}

impl InstallObserver for ProgressObserver {
    fn progress(&self, stage: InstallStage, done: u64, total: Option<u64>) {
        self.emit(stage.name(), done, total.unwrap_or(0));
    }

    fn is_cancelled(&self) -> bool {
        self.handle.is_cancelled()
    }
}

fn percent_of(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    let percent = u128::from(done.min(total)) * 100 / u128::from(total);
    u32::try_from(percent).unwrap_or(100)
}

fn resource_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().resource_dir().ok()
}

fn app_store(resource_dir: Option<PathBuf>) -> anyhow::Result<CatalogStore> {
    Ok(CatalogStore::new(StoreRoots::from_app(
        resource_dir.as_deref(),
    )?))
}

fn writable_store(resource_dir: Option<PathBuf>) -> anyhow::Result<CatalogStore> {
    let store = app_store(resource_dir)?;
    let config = config::load_config()?;
    let data = &store.roots().data;
    if chosen_data_root_missing(std::env::var_os(ENV_DATA_DIR), &config, data) {
        anyhow::bail!(
            "the data folder {} is not reachable; reconnect its drive or choose another data folder in Settings",
            data.display()
        );
    }
    Ok(store)
}

pub(crate) fn status_json(store: &CatalogStore, config: &AppConfig) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(store.report(config))?)
}

#[cfg(feature = "net")]
pub(crate) async fn install_dataset(
    store: &CatalogStore,
    id: &str,
    observer: Arc<dyn InstallObserver>,
) -> Result<InstallOutcome, String> {
    store.spec(id).map_err(|error| error.to_string())?;
    let client =
        catalog_client().map_err(|error| format!("the download client did not start: {error}"))?;
    store
        .install(id, &client, observer)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "net"))]
pub(crate) async fn install_dataset(
    store: &CatalogStore,
    id: &str,
    observer: Arc<dyn InstallObserver>,
) -> Result<InstallOutcome, String> {
    store
        .install(id, observer)
        .await
        .map_err(|error| error.to_string())
}

fn update_choices<T>(
    change: impl FnOnce(&mut AppConfig) -> Result<T, StoreError>,
) -> anyhow::Result<T> {
    let _serialised = CATALOG_CHOICES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let before = config::load_config()?;
    let mut after = before.clone();
    let value = change(&mut after)?;
    if before.external_catalogs != after.external_catalogs {
        config::update_config_field(
            FIELD_EXTERNAL_CATALOGS,
            serde_json::to_value(&after.external_catalogs)?,
            &[],
        )?;
    }
    if before.siril_catalog_dir != after.siril_catalog_dir {
        config::update_config_field(
            FIELD_SIRIL_CATALOG_DIR,
            serde_json::to_value(&after.siril_catalog_dir)?,
            &[],
        )?;
    }
    Ok(value)
}

#[tauri::command]
pub async fn catalog_status_cmd(app: tauri::AppHandle) -> Result<Value, String> {
    let resource = resource_dir(&app);
    blocking_cmd!({
        let store = app_store(resource)?;
        status_json(&store, &config::load_config()?)
    })
}

#[tauri::command]
pub async fn catalog_install_cmd(app: tauri::AppHandle, id: String) -> Result<Value, String> {
    let resource = resource_dir(&app);
    let store = tokio::task::spawn_blocking(move || writable_store(resource))
        .await
        .map_err(|error| format!("Task join failed: {error}"))?
        .map_err(|error| format!("{error:#}"))?;
    let observer = Arc::new(ProgressObserver {
        id: id.clone(),
        handle: ProgressHandle::new(&app, &install_cancel_key(&id), 1),
        app: app.clone(),
    });
    let outcome = install_dataset(&store, &id, observer.clone()).await?;
    observer.emit(COMPLETE_STAGE, outcome.bytes, outcome.bytes);
    serde_json::to_value(outcome).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn catalog_use_file_cmd(
    app: tauri::AppHandle,
    id: String,
    path: String,
) -> Result<Value, String> {
    let resource = resource_dir(&app);
    blocking_cmd!({
        let store = app_store(resource)?;
        let used = update_choices(|config| store.use_external_file(&id, Path::new(&path), config))?;
        Ok(json!({ "id": id, "path": used }))
    })
}

#[tauri::command]
pub async fn catalog_use_siril_dir_cmd(
    app: tauri::AppHandle,
    dir: String,
) -> Result<Value, String> {
    let resource = resource_dir(&app);
    blocking_cmd!({
        let store = app_store(resource)?;
        let file = update_choices(|config| store.use_siril_dir(Path::new(&dir), config))?;
        Ok(json!({ "dir": dir, "path": file }))
    })
}

#[tauri::command]
pub async fn catalog_remove_cmd(app: tauri::AppHandle, id: String) -> Result<Value, String> {
    let resource = resource_dir(&app);
    blocking_cmd!({
        let store = writable_store(resource)?;
        let removed = update_choices(|config| store.remove(&id, config))?;
        Ok(serde_json::to_value(removed)?)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use regex::Regex;

    use serde_json::{json, Value};

    use super::{
        install_cancel_key, install_dataset, install_progress_payload, percent_of, status_json,
    };
    use crate::infra::catalog_store::{
        CatalogStore, InstallObserver, InstallStage, StoreRoots, SIRIL_GAIA_BZ2_BYTES,
        SIRIL_GAIA_ID, TWOMASS_ID, TYCHO2_ID,
    };
    use crate::types::config::AppConfig;

    const CATALOG_COMMANDS: [&str; 5] = [
        "catalog_status_cmd",
        "catalog_install_cmd",
        "catalog_use_file_cmd",
        "catalog_use_siril_dir_cmd",
        "catalog_remove_cmd",
    ];
    const FREE_BYTES: u64 = 123_456_789;

    struct Silent;

    impl InstallObserver for Silent {
        fn progress(&self, _stage: InstallStage, _done: u64, _total: Option<u64>) {}

        fn is_cancelled(&self) -> bool {
            false
        }
    }

    fn temp_store(dir: &Path) -> CatalogStore {
        CatalogStore::new(StoreRoots::under(&dir.join("data"), &dir.join("cache")))
            .with_free_space(Arc::new(|_: &Path| Ok(FREE_BYTES)))
    }

    #[test]
    fn catalog_status_cmd_serialises_size_licence_credit_and_free_space() {
        let dir = tempfile::tempdir().unwrap();
        let value = status_json(&temp_store(dir.path()), &AppConfig::default()).unwrap();
        let rows = value.as_array().unwrap();
        let ids: Vec<&str> = rows.iter().filter_map(|row| row["id"].as_str()).collect();
        for id in [TYCHO2_ID, TWOMASS_ID, SIRIL_GAIA_ID] {
            assert!(ids.contains(&id), "{id} missing from {ids:?}");
        }
        let siril = rows.iter().find(|row| row["id"] == SIRIL_GAIA_ID).unwrap();
        assert_eq!(siril["size_bytes"], SIRIL_GAIA_BZ2_BYTES);
        assert_eq!(siril["kind"], "external");
        assert_eq!(siril["compressed"], "bzip2");
        let host = if cfg!(feature = "net") {
            json!("zenodo.org")
        } else {
            Value::Null
        };
        assert_eq!(siril["url_host"], host);
        assert_eq!(siril["free_space_bytes"], FREE_BYTES);
        assert_eq!(siril["installable"], false);
        assert_eq!(siril["status"]["state"], "missing");
        assert!(siril["credit"].as_str().unwrap().contains("ESA/Gaia/DPAC"));
        let siril_licence = siril["licence"].as_str().unwrap();
        assert!(
            siril_licence.contains("Siril catalogue format specification: CC BY 4.0"),
            "{siril_licence}"
        );
        assert!(
            siril_licence.contains("data: Gaia DR3, ESA/Gaia/DPAC"),
            "{siril_licence}"
        );
        for row in rows {
            assert!(!row["licence"].as_str().unwrap().is_empty(), "{row}");
            assert!(!row["credit"].as_str().unwrap().is_empty(), "{row}");
            assert!(
                row["folder"].as_str().unwrap().contains("catalogs"),
                "{row}"
            );
        }
        let tycho = rows.iter().find(|row| row["id"] == TYCHO2_ID).unwrap();
        assert_eq!(tycho["kind"], "tier0_bundled");
        assert_eq!(tycho["size_bytes"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn catalog_install_cmd_refuses_an_unknown_id_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let refused = install_dataset(
            &temp_store(dir.path()),
            "no-such-catalogue",
            Arc::new(Silent),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("\"no-such-catalogue\""), "{refused}");
        assert!(
            refused.contains(TYCHO2_ID) && refused.contains(SIRIL_GAIA_ID),
            "{refused}"
        );
        assert!(!dir.path().join("data").exists());
    }

    #[test]
    fn install_progress_percent_is_bounded() {
        assert_eq!(percent_of(0, 0), 0);
        assert_eq!(percent_of(50, 200), 25);
        assert_eq!(percent_of(300, 200), 100);
        assert_eq!(percent_of(u64::MAX, u64::MAX), 100);
    }

    #[test]
    fn each_install_reports_its_dataset_and_cancels_under_its_own_key() {
        assert_eq!(
            install_cancel_key(TWOMASS_ID),
            "catalog-install-progress:2mass-l8-v1"
        );
        assert_ne!(
            install_cancel_key(TWOMASS_ID),
            install_cancel_key(SIRIL_GAIA_ID)
        );
        let payload =
            serde_json::to_value(install_progress_payload(SIRIL_GAIA_ID, "decode", 50, 200))
                .unwrap();
        assert_eq!(payload["id"], SIRIL_GAIA_ID);
        assert_eq!(payload["stage"], "decode");
        assert_eq!(
            (
                payload["current"].as_u64(),
                payload["total"].as_u64(),
                payload["percent"].as_u64()
            ),
            (Some(50), Some(200), Some(25))
        );
    }

    fn assert_wiring(lib: &str, cmd_mod: &str, infra_mod: &str) {
        let lib = lib.replace("\r\n", "\n");
        let cmd_mod = cmd_mod.replace("\r\n", "\n");
        let infra_mod = infra_mod.replace("\r\n", "\n");
        let listed = Regex::new(r"(?m)^\s*cmd::catalog_store::(catalog_\w+),").unwrap();
        let registered: Vec<String> = listed
            .captures_iter(&lib)
            .map(|found| found[1].to_owned())
            .collect();
        assert_eq!(registered, CATALOG_COMMANDS);
        let gate = "#[cfg(feature = \"net\")]";
        assert!(!lib.contains(&format!("{gate}\n            cmd::catalog_store::")));
        for registry in [&cmd_mod, &infra_mod] {
            assert!(registry.contains("\npub mod catalog_store;"));
            assert!(!registry.contains(&format!("{gate}\npub mod catalog_store;")));
        }
        let defined = Regex::new(r"#\[tauri::command\]\s*pub async fn (\w+)\(").unwrap();
        let commands: Vec<String> = defined
            .captures_iter(include_str!("catalog_store.rs"))
            .map(|found| found[1].to_owned())
            .collect();
        assert_eq!(commands, CATALOG_COMMANDS);
    }

    #[test]
    fn catalog_store_commands_are_registered_in_every_build() {
        assert_wiring(
            include_str!("../lib.rs"),
            include_str!("mod.rs"),
            include_str!("../infra/mod.rs"),
        );
    }
}
