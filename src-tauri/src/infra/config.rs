use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use crate::types::config::AppConfig;

fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("Could not determine config directory")?
        .join("astroburst");
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

fn api_key_path(service: &str) -> Result<PathBuf> {
    Ok(config_dir()?.join(format!("{}_api_key.txt", service)))
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        let default = AppConfig::default();
        save_config(&default)?;
        return Ok(default);
    }
    let content = fs::read_to_string(&path).context("Failed to read config")?;
    let config: AppConfig = serde_json::from_str(&content).context("Failed to parse config")?;
    Ok(config)
}

pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path()?;
    let content = serde_json::to_string_pretty(config).context("Failed to serialize config")?;
    fs::write(&path, content).context("Failed to write config")?;
    Ok(())
}

pub fn update_config_field(field: &str, value: serde_json::Value) -> Result<AppConfig> {
    let mut config = load_config()?;
    let mut map = serde_json::to_value(&config).context("Failed to serialize config")?;

    if let Some(obj) = map.as_object_mut() {
        obj.insert(field.to_string(), value);
    }

    config = serde_json::from_value(map).context("Failed to deserialize updated config")?;
    save_config(&config)?;
    Ok(config)
}

pub fn save_api_key(key: &str, service: &str) -> Result<()> {
    let path = api_key_path(service)?;
    fs::write(&path, key).context("Failed to write API key")?;
    Ok(())
}

pub fn load_api_key(service: &str) -> Result<Option<String>> {
    let path = api_key_path(service)?;
    if !path.exists() {
        return Ok(None);
    }
    let key = fs::read_to_string(&path).context("Failed to read API key")?;
    let trimmed = key.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}
