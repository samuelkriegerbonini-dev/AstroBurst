use std::path::{Path, PathBuf};

use serde_json::json;
use tauri::Manager;

use crate::cmd::common::blocking_cmd;
use crate::infra::config;
use crate::types::constants::{RES_KEY, RES_SAVED, RES_SERVICE};

const UI_API_KEY_SERVICES: [&str; 1] = ["astrometry"];

type ConfigDir = fn() -> anyhow::Result<PathBuf>;

fn ui_service(service: Option<String>) -> Result<String, String> {
    let service = service.unwrap_or_else(|| UI_API_KEY_SERVICES[0].to_owned());
    if UI_API_KEY_SERVICES.contains(&service.as_str()) {
        Ok(service)
    } else {
        Err(format!("API key service {service:?} is not available to the interface"))
    }
}

#[tauri::command]
pub async fn get_config() -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cfg = config::load_config()?;
        Ok(serde_json::to_value(&cfg)?)
    })
}

#[tauri::command]
pub async fn update_config(
    app: tauri::AppHandle,
    field: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let managed = app_managed_dirs(&app);
    blocking_cmd!({
        let reserved: Vec<&Path> = managed.iter().map(PathBuf::as_path).collect();
        let cfg = config::update_config_field(&field, value, &reserved)?;
        Ok(serde_json::to_value(&cfg)?)
    })
}

fn app_managed_dirs(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let paths = app.path();
    [paths.app_data_dir(), paths.app_local_data_dir()]
        .into_iter()
        .filter_map(Result::ok)
        .collect()
}

#[tauri::command]
pub async fn save_api_key(key: String, service: Option<String>) -> Result<serde_json::Value, String> {
    save_api_key_in(config::config_dir, key, service).await
}

async fn save_api_key_in(dir: ConfigDir, key: String, service: Option<String>) -> Result<serde_json::Value, String> {
    let svc = ui_service(service)?;
    blocking_cmd!({
        config::save_api_key_in(&dir()?, &key, &svc)?;
        Ok(json!({ RES_SAVED: true, RES_SERVICE: svc }))
    })
}

#[tauri::command]
pub async fn get_api_key(service: Option<String>) -> Result<serde_json::Value, String> {
    get_api_key_in(config::config_dir, service).await
}

async fn get_api_key_in(dir: ConfigDir, service: Option<String>) -> Result<serde_json::Value, String> {
    let svc = ui_service(service)?;
    blocking_cmd!({
        let key = config::load_api_key_in(&dir()?, &svc)?;
        Ok(json!({ RES_KEY: key, RES_SERVICE: svc }))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{get_api_key_in, save_api_key_in};

    const DIR_REACHED: &str = "config directory reached";

    fn unreachable_dir() -> anyhow::Result<PathBuf> {
        anyhow::bail!(DIR_REACHED)
    }

    #[tokio::test]
    async fn get_api_key_refuses_mast() {
        for service in ["mast", "../x", "MAST", ""] {
            let error = get_api_key_in(unreachable_dir, Some(service.to_owned())).await.unwrap_err();
            assert!(error.contains("not available"), "{service}: {error}");
        }
        for service in [None, Some("astrometry".to_owned())] {
            let error = get_api_key_in(unreachable_dir, service).await.unwrap_err();
            assert!(error.contains(DIR_REACHED), "{error}");
        }
    }

    #[tokio::test]
    async fn save_api_key_refuses_mast_and_traversal() {
        for service in ["mast", "../x", "../../evil", ""] {
            let error = save_api_key_in(unreachable_dir, "value".to_owned(), Some(service.to_owned())).await.unwrap_err();
            assert!(error.contains("not available"), "{service}: {error}");
        }
        let error = save_api_key_in(unreachable_dir, "value".to_owned(), None).await.unwrap_err();
        assert!(error.contains(DIR_REACHED), "{error}");
    }
}
