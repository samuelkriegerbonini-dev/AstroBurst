use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use crate::types::config::AppConfig;
use crate::types::constants::DEFAULT_ASTROMETRY_API_URL;

pub const FIELD_ASTROMETRY_API_URL: &str = "astrometry_api_url";

const CONFIG_TEMP_EXTENSION: &str = "json.tmp";
const CONFIG_BACKUP_EXTENSION: &str = "json.bad";

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn lock_config() -> MutexGuard<'static, ()> {
    CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("Could not determine config directory")?
        .join("astroburst");
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

pub(crate) fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

fn api_key_path(service: &str) -> Result<PathBuf> {
    Ok(config_dir()?.join(format!("{}_api_key.txt", service)))
}

fn parse_config(content: &str) -> Result<AppConfig> {
    let stored: serde_json::Value = serde_json::from_str(content).context("Failed to parse config")?;
    let stored = stored
        .as_object()
        .ok_or_else(|| anyhow!("config is not a JSON object"))?;
    let mut merged = serde_json::to_value(AppConfig::default()).context("Failed to serialize config")?;
    let base = merged
        .as_object_mut()
        .ok_or_else(|| anyhow!("default config is not a JSON object"))?;
    for (key, value) in stored {
        base.insert(key.clone(), value.clone());
    }
    serde_json::from_value(merged).context("Failed to parse config")
}

fn write_config_file(path: &Path, config: &AppConfig) -> Result<()> {
    let content = serde_json::to_string_pretty(config).context("Failed to serialize config")?;
    let temp = path.with_extension(CONFIG_TEMP_EXTENSION);
    fs::write(&temp, content).context("Failed to write config")?;
    fs::rename(&temp, path).context("Failed to replace config")?;
    Ok(())
}

fn load_config_at(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        let default = AppConfig::default();
        write_config_file(path, &default)?;
        return Ok(default);
    }
    let bytes = fs::read(path).context("Failed to read config")?;
    match parse_config(&String::from_utf8_lossy(&bytes)) {
        Ok(config) => Ok(config),
        Err(e) => {
            let backup = path.with_extension(CONFIG_BACKUP_EXTENSION);
            match fs::rename(path, &backup) {
                Ok(()) => log::warn!(
                    "config {} was unreadable ({:#}); kept it as {} and continued with defaults",
                    path.display(),
                    e,
                    backup.display()
                ),
                Err(move_err) => log::warn!(
                    "config {} was unreadable ({:#}) and could not be moved aside ({}); continued with defaults",
                    path.display(),
                    e,
                    move_err
                ),
            }
            Ok(AppConfig::default())
        }
    }
}

fn update_config_field_at(path: &Path, field: &str, value: serde_json::Value) -> Result<AppConfig> {
    let config = load_config_at(path)?;
    let mut map = serde_json::to_value(&config).context("Failed to serialize config")?;
    let obj = map
        .as_object_mut()
        .ok_or_else(|| anyhow!("config is not a JSON object"))?;
    if !obj.contains_key(field) {
        let mut valid: Vec<&str> = obj.keys().map(String::as_str).collect();
        valid.sort_unstable();
        bail!("Unknown config field {:?}; valid fields: {}", field, valid.join(", "));
    }
    obj.insert(field.to_string(), value);
    let updated: AppConfig = serde_json::from_value(map).context("Failed to deserialize updated config")?;
    write_config_file(path, &updated)?;
    Ok(updated)
}

pub fn load_config() -> Result<AppConfig> {
    load_config_from(&config_path()?)
}

pub(crate) fn load_config_from(path: &Path) -> Result<AppConfig> {
    let _guard = lock_config();
    load_config_at(path)
}

pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path()?;
    let _guard = lock_config();
    write_config_file(&path, config)
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
    let path = config_path()?;
    let _guard = lock_config();
    update_config_field_at(&path, field, value)
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
    use super::{
        ensure_https_api_url, load_config_at, update_config_field, update_config_field_at, write_config_file,
        CONFIG_BACKUP_EXTENSION, CONFIG_TEMP_EXTENSION, FIELD_ASTROMETRY_API_URL,
    };
    use crate::types::config::AppConfig;
    use serde_json::json;

    fn config_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("config.json")
    }

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

    #[test]
    fn a_truncated_config_is_set_aside_and_the_app_keeps_working() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        let torn = "{\n  \"plate_solve_timeout_secs\": 300\n}\n}tail of an older write";
        std::fs::write(&path, torn).unwrap();

        let loaded = load_config_at(&path).expect("a corrupt config must not break get_config");
        assert_eq!(loaded.plate_solve_timeout_secs, AppConfig::default().plate_solve_timeout_secs);
        let backup = path.with_extension(CONFIG_BACKUP_EXTENSION);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), torn, "the unreadable file was not kept");

        let updated = update_config_field_at(&path, "plate_solve_timeout_secs", json!(250)).unwrap();
        assert_eq!(updated.plate_solve_timeout_secs, 250);
        assert_eq!(load_config_at(&path).unwrap().plate_solve_timeout_secs, 250);
    }

    #[test]
    fn a_config_written_before_a_field_existed_keeps_its_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        std::fs::write(&path, "{\"plate_solve_timeout_secs\": 300, \"retired_setting\": true}").unwrap();
        let loaded = load_config_at(&path).unwrap();
        assert_eq!(loaded.plate_solve_timeout_secs, 300);
        assert_eq!(loaded.astrometry_api_url, AppConfig::default().astrometry_api_url);
        assert!(!path.with_extension(CONFIG_BACKUP_EXTENSION).exists());
    }

    #[test]
    fn an_unknown_field_is_an_error_that_names_the_valid_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        let err = update_config_field_at(&path, "plate_solve_timeout", json!(300))
            .unwrap_err()
            .to_string();
        assert!(err.contains("plate_solve_timeout"), "{err}");
        assert!(err.contains("plate_solve_timeout_secs"), "{err}");
        assert_eq!(load_config_at(&path).unwrap().plate_solve_timeout_secs, AppConfig::default().plate_solve_timeout_secs);
    }

    #[test]
    fn saving_replaces_the_file_through_a_temporary_and_leaves_no_residue() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        let mut config = AppConfig::default();
        config.plate_solve_timeout_secs = 42;
        write_config_file(&path, &config).unwrap();
        config.plate_solve_timeout_secs = 7;
        write_config_file(&path, &config).unwrap();
        assert_eq!(load_config_at(&path).unwrap().plate_solve_timeout_secs, 7);
        assert!(!path.with_extension(CONFIG_TEMP_EXTENSION).exists());
    }
}
