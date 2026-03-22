use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::infra::config;
use crate::types::constants::{RES_KEY, RES_SAVED, RES_SERVICE};

#[tauri::command]
pub async fn get_config() -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cfg = config::load_config()?;
        Ok(serde_json::to_value(&cfg)?)
    })
}

#[tauri::command]
pub async fn update_config(field: String, value: serde_json::Value) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cfg = config::update_config_field(&field, value)?;
        Ok(serde_json::to_value(&cfg)?)
    })
}

#[tauri::command]
pub async fn save_api_key(key: String, service: Option<String>) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let svc = service.as_deref().unwrap_or("astrometry");
        config::save_api_key(&key, svc)?;
        Ok(json!({ RES_SAVED: true, RES_SERVICE: svc }))
    })
}

#[tauri::command]
pub async fn get_api_key(service: Option<String>) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let svc = service.as_deref().unwrap_or("astrometry");
        let key = config::load_api_key(svc)?;
        Ok(json!({ RES_KEY: key, RES_SERVICE: svc }))
    })
}
