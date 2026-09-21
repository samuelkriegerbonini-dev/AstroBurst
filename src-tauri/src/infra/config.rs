use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::config::AppConfig;
use crate::types::constants::DEFAULT_ASTROMETRY_API_URL;

pub const FIELD_ASTROMETRY_API_URL: &str = "astrometry_api_url";

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

fn https_host_of(url: &str) -> Option<&str> {
    let trimmed = url.trim();
    let rest = trimmed
        .get(..8)
        .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
        .map(|_| &trimmed[8..])?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() || host.contains(char::is_whitespace) {
        None
    } else {
        Some(host)
    }
}

pub fn ensure_https_api_url(url: &str) -> Result<()> {
    if https_host_of(url).is_some() {
        Ok(())
    } else {
        Err(anyhow!(
            "Refusing to use astrometry API URL {:?}: the API key is only sent over https:// URLs with a host (default: {})",
            url.trim(),
            DEFAULT_ASTROMETRY_API_URL
        ))
    }
}

pub fn astrometry_api_url() -> Result<String> {
    let configured = load_config()
        .map(|cfg| cfg.astrometry_api_url)
        .unwrap_or_else(|_| DEFAULT_ASTROMETRY_API_URL.to_string());
    ensure_https_api_url(&configured)?;
    Ok(configured.trim().to_string())
}

pub fn update_config_field(field: &str, value: serde_json::Value) -> Result<AppConfig> {
    if field == FIELD_ASTROMETRY_API_URL {
        let candidate = value
            .as_str()
            .ok_or_else(|| anyhow!("{} must be a string", FIELD_ASTROMETRY_API_URL))?;
        ensure_https_api_url(candidate)?;
    }

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
    restrict_to_owner(&path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .context("Failed to restrict API key file permissions")
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::{ensure_https_api_url, update_config_field, FIELD_ASTROMETRY_API_URL};
    use serde_json::json;

    #[test]
    fn accepts_https_urls_with_a_host() {
        assert!(ensure_https_api_url("https://nova.astrometry.net").is_ok());
        assert!(ensure_https_api_url("  https://nova.astrometry.net/api/  ").is_ok());
        assert!(ensure_https_api_url("HTTPS://nova.astrometry.net").is_ok());
    }

    #[test]
    fn rejects_non_https_or_hostless_urls() {
        for bad in [
            "http://nova.astrometry.net",
            "http://",
            "https://",
            "https:///api",
            "ftp://nova.astrometry.net",
            "nova.astrometry.net",
            "",
        ] {
            assert!(
                ensure_https_api_url(bad).is_err(),
                "expected {:?} to be rejected",
                bad
            );
        }
    }

    #[test]
    fn rejected_url_appears_in_the_error() {
        let err = ensure_https_api_url("http://evil.example").unwrap_err().to_string();
        assert!(err.contains("http://evil.example"), "error was: {}", err);
    }

    #[test]
    fn update_config_field_refuses_to_persist_a_non_https_api_url() {
        let err = update_config_field(FIELD_ASTROMETRY_API_URL, json!("http://evil.example"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("http://evil.example"), "error was: {}", err);
    }

    #[test]
    fn update_config_field_refuses_a_non_string_api_url() {
        assert!(update_config_field(FIELD_ASTROMETRY_API_URL, json!(42)).is_err());
    }
}
