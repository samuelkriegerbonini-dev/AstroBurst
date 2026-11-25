use std::path::{Path, PathBuf};

use serde_json::json;
use tauri::Manager;

use crate::cmd::common::blocking_cmd;
use crate::infra::config;
use crate::types::constants::{RES_SAVED, RES_SERVICE, RES_WARNINGS};

const UI_API_KEY_SERVICES: [&str; 1] = ["astrometry"];
const RES_STORED: &str = "stored";
const RES_HINT: &str = "hint";
const KEY_HINT_EDGE: usize = 4;
const KEY_HINT_MIN_LEN: usize = 12;
const SHORT_KEY_HINT: &str = "****";

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
        let mut value = serde_json::to_value(&cfg)?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(RES_WARNINGS.to_owned(), json!(config::settings_warnings()));
        }
        Ok(value)
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
        Ok(json!({ RES_STORED: key.is_some(), RES_HINT: key.as_deref().map(key_hint), RES_SERVICE: svc }))
    })
}

fn key_hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() < KEY_HINT_MIN_LEN {
        return SHORT_KEY_HINT.to_owned();
    }
    let head: String = chars[..KEY_HINT_EDGE].iter().collect();
    let tail: String = chars[chars.len() - KEY_HINT_EDGE..].iter().collect();
    format!("{head}...{tail}")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use super::{get_api_key_in, save_api_key_in};

    static KEY_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();

    fn key_dir() -> anyhow::Result<PathBuf> {
        Ok(KEY_DIR.get_or_init(|| tempfile::tempdir().unwrap()).path().to_path_buf())
    }

    static EMPTY_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    static SHORT_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();

    fn short_dir() -> anyhow::Result<PathBuf> {
        Ok(SHORT_DIR.get_or_init(|| tempfile::tempdir().unwrap()).path().to_path_buf())
    }

    fn empty_dir() -> anyhow::Result<PathBuf> {
        Ok(EMPTY_DIR.get_or_init(|| tempfile::tempdir().unwrap()).path().to_path_buf())
    }

    #[tokio::test]
    async fn get_api_key_returns_only_whether_a_key_is_stored_and_a_hint() {
        crate::infra::config::save_api_key_in(&key_dir().unwrap(), "0123456789abcdef", "astrometry").unwrap();
        let result = get_api_key_in(key_dir, None).await.unwrap();
        let text = result.to_string();
        assert!(!text.contains("456789ab"), "the key reached the interface: {text}");
        assert_eq!(result["stored"], serde_json::json!(true), "{text}");
        assert_eq!(result["hint"], serde_json::json!("0123...cdef"), "{text}");
        crate::infra::config::save_api_key_in(&short_dir().unwrap(), "abcdefgh", "astrometry").unwrap();
        let short = get_api_key_in(short_dir, None).await.unwrap();
        assert!(!short.to_string().contains("abcd") && !short.to_string().contains("efgh"), "{short}");
        assert_eq!(short["stored"], serde_json::json!(true), "{short}");
        let none = get_api_key_in(empty_dir, None).await.unwrap();
        assert_eq!(none["stored"], serde_json::json!(false), "{none}");
        assert!(none["hint"].is_null(), "{none}");
    }

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
