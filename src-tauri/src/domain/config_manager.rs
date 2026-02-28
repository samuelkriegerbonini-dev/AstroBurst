use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

const CONFIG_FILENAME: &str = "astrokit_config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub astrometry_api_key: Option<String>,
    pub astrometry_api_url: String,
    pub default_output_dir: Option<String>,
    pub plate_solve_timeout_secs: u64,
    pub plate_solve_max_stars: usize,
    pub auto_stretch_target_bg: f64,
    pub auto_stretch_shadow_k: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            astrometry_api_key: None,
            astrometry_api_url: "https://nova.astrometry.net".into(),
            default_output_dir: None,
            plate_solve_timeout_secs: 120,
            plate_solve_max_stars: 100,
            auto_stretch_target_bg: 0.25,
            auto_stretch_shadow_k: -2.8,
        }
    }
}

pub fn init_config_dir(app_data_dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(app_data_dir);
    let _ = CONFIG_DIR.set(app_data_dir.to_path_buf());

    let target = app_data_dir.join(CONFIG_FILENAME);
    if !target.exists() {
        let default_cfg = AppConfig {
            astrometry_api_key: Some("rkiguzwstjshftaj".into()),
            ..Default::default()
        };
        let _ = save_config(&default_cfg);
    }
}

fn config_path() -> PathBuf {
    CONFIG_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("com.astrokit.app")
        })
        .join(CONFIG_FILENAME)
}

pub fn load_config() -> AppConfig {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config dir: {:?}", parent))?;
    }
    let json = serde_json::to_string_pretty(config)
        .context("Failed to serialize config")?;
    std::fs::write(&path, json)
        .with_context(|| format!("Failed to write config to {:?}", path))?;
    Ok(())
}

pub fn save_api_key(key: &str) -> Result<()> {
    let mut config = load_config();
    config.astrometry_api_key = if key.trim().is_empty() {
        None
    } else {
        Some(key.trim().to_string())
    };
    save_config(&config)
}

pub fn get_api_key() -> Option<String> {
    load_config().astrometry_api_key
}

pub fn update_config_field(field: &str, value: serde_json::Value) -> Result<()> {
    let mut config = load_config();
    match field {
        "astrometry_api_key" => {
            config.astrometry_api_key = value.as_str().map(|s| s.to_string());
        }
        "astrometry_api_url" => {
            if let Some(s) = value.as_str() {
                config.astrometry_api_url = s.to_string();
            }
        }
        "default_output_dir" => {
            config.default_output_dir = value.as_str().map(|s| s.to_string());
        }
        "plate_solve_timeout_secs" => {
            if let Some(n) = value.as_u64() {
                config.plate_solve_timeout_secs = n;
            }
        }
        "plate_solve_max_stars" => {
            if let Some(n) = value.as_u64() {
                config.plate_solve_max_stars = n as usize;
            }
        }
        "auto_stretch_target_bg" => {
            if let Some(n) = value.as_f64() {
                config.auto_stretch_target_bg = n;
            }
        }
        "auto_stretch_shadow_k" => {
            if let Some(n) = value.as_f64() {
                config.auto_stretch_shadow_k = n;
            }
        }
        _ => anyhow::bail!("Unknown config field: {}", field),
    }
    save_config(&config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = AppConfig::default();
        assert!(cfg.astrometry_api_key.is_none());
        assert_eq!(cfg.plate_solve_max_stars, 100);
        assert_eq!(cfg.astrometry_api_url, "https://nova.astrometry.net");
    }

    #[test]
    fn test_roundtrip_serialization() {
        let mut cfg = AppConfig::default();
        cfg.astrometry_api_key = Some("test_key_123".into());
        cfg.plate_solve_timeout_secs = 60;

        let json = serde_json::to_string(&cfg).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.astrometry_api_key.as_deref(), Some("test_key_123"));
        assert_eq!(restored.plate_solve_timeout_secs, 60);
    }

    #[test]
    fn test_save_and_load_config() {
        let tmp = tempfile::tempdir().unwrap();
        init_config_dir(tmp.path());

        let mut cfg = AppConfig::default();
        cfg.astrometry_api_key = Some("roundtrip_key".into());
        save_config(&cfg).unwrap();

        let loaded = load_config();
        assert_eq!(loaded.astrometry_api_key.as_deref(), Some("roundtrip_key"));
    }

    #[test]
    fn test_save_api_key_shortcut() {
        let tmp = tempfile::tempdir().unwrap();
        init_config_dir(tmp.path());

        save_api_key("my_secret_key").unwrap();
        assert_eq!(get_api_key().as_deref(), Some("my_secret_key"));

        save_api_key("").unwrap();
        assert!(get_api_key().is_none());
    }

    #[test]
    fn test_update_field() {
        let tmp = tempfile::tempdir().unwrap();
        init_config_dir(tmp.path());

        save_config(&AppConfig::default()).unwrap();

        update_config_field("plate_solve_max_stars", serde_json::json!(200)).unwrap();
        let cfg = load_config();
        assert_eq!(cfg.plate_solve_max_stars, 200);

        update_config_field("astrometry_api_key", serde_json::json!("new_key")).unwrap();
        let cfg = load_config();
        assert_eq!(cfg.astrometry_api_key.as_deref(), Some("new_key"));
    }
}
