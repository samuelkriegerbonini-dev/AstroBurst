use anyhow::Result;

use crate::domain::config_manager;

use super::helpers::map_anyhow;

#[tauri::command]
pub async fn get_config() -> Result<serde_json::Value, String> {
    let cfg = config_manager::load_config();
    let has_key = cfg.astrometry_api_key.is_some();
    Ok(serde_json::json!({
        "has_api_key": has_key,
        "astrometry_api_url": cfg.astrometry_api_url,
        "default_output_dir": cfg.default_output_dir,
        "plate_solve_timeout_secs": cfg.plate_solve_timeout_secs,
        "plate_solve_max_stars": cfg.plate_solve_max_stars,
        "auto_stretch_target_bg": cfg.auto_stretch_target_bg,
        "auto_stretch_shadow_k": cfg.auto_stretch_shadow_k,
    }))
}

#[tauri::command]
pub async fn update_config(
    field: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        config_manager::update_config_field(&field, value)?;
        Ok(serde_json::json!({ "updated": true }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn save_api_key(
    service: Option<String>,
    key: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let svc = service.as_deref().unwrap_or("astrometry");
        match svc {
            "astrometry" => {
                config_manager::save_api_key(&key)?;
                Ok(serde_json::json!({ "saved": true, "service": "astrometry" }))
            }
            _ => {
                config_manager::save_api_key(&key)?;
                Ok(serde_json::json!({ "saved": true, "service": svc }))
            }
        }
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_api_key() -> Result<serde_json::Value, String> {
    let key = config_manager::get_api_key();
    let is_set = key.is_some();
    let masked = key.as_deref().map(|k| {
        if k.len() <= 4 {
            "****".to_string()
        } else {
            format!("{}...{}", &k[..4], &k[k.len() - 4..])
        }
    });
    Ok(serde_json::json!({
        "is_set": is_set,
        "masked": masked,
    }))
}
