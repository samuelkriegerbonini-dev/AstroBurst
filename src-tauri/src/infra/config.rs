use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::types::config::AppConfig;
use crate::types::constants::DEFAULT_ASTROMETRY_API_URL;

pub const FIELD_ASTROMETRY_API_URL: &str = "astrometry_api_url";
pub const FIELD_MAST_TOKEN_TRANSPORT: &str = "mast_token_transport";
pub const MAST_TOKEN_TRANSPORTS: [&str; 2] = ["header", "cookie"];
pub const FIELD_DATA_DIR_OVERRIDE: &str = "data_dir_override";
pub const FIELD_MAST_QUOTA_GB: &str = "mast_quota_gb";
const API_KEY_SERVICE_MAX_LEN: usize = 32;
const API_KEY_FILE_SUFFIX: &str = "_api_key.txt";

const CONFIG_FILE: &str = "config.json";
const CONFIG_TEMP_EXTENSION: &str = "json.tmp";
const CONFIG_BACKUP_EXTENSION: &str = "json.bad";

const APP_DIR: &str = "astroburst";

static CONFIG_LOCK: Mutex<()> = Mutex::new(());
static REDIRECTED_DIR: OnceLock<PathBuf> = OnceLock::new();

fn lock_config() -> MutexGuard<'static, ()> {
    CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn redirect_config_dir(dir: PathBuf) {
    if REDIRECTED_DIR.set(dir).is_err() {
        log::warn!("the config folder was already chosen; the redirect is ignored");
    }
}

fn config_dir_from(redirected: Option<&Path>, system: Option<PathBuf>) -> Option<PathBuf> {
    match redirected {
        Some(dir) => Some(dir.to_path_buf()),
        None => system.map(|dir| dir.join(APP_DIR)),
    }
}

pub(crate) fn config_dir() -> Result<PathBuf> {
    let dir = config_dir_from(REDIRECTED_DIR.get().map(PathBuf::as_path), dirs::config_dir())
        .context("Could not determine config directory")?;
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

pub(crate) fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(CONFIG_FILE))
}

pub fn validate_api_key_service(service: &str) -> Result<()> {
    let valid = (1..=API_KEY_SERVICE_MAX_LEN).contains(&service.len())
        && service.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    if valid {
        Ok(())
    } else {
        bail!(
            "Invalid API key service name {:?}: expected 1 to {} characters from a-z and _",
            service,
            API_KEY_SERVICE_MAX_LEN
        )
    }
}

fn api_key_path_in(dir: &Path, service: &str) -> Result<PathBuf> {
    validate_api_key_service(service)?;
    Ok(dir.join(format!("{}{}", service, API_KEY_FILE_SUFFIX)))
}

pub fn is_api_key_file(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.to_ascii_lowercase().ends_with(API_KEY_FILE_SUFFIX)
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

fn refuse_managed_field(field: &str) -> Result<()> {
    if field == FIELD_MAST_TOKEN_TRANSPORT {
        bail!("{} is managed by the MAST token commands and cannot be set directly", FIELD_MAST_TOKEN_TRANSPORT);
    }
    Ok(())
}

fn validated_data_dir(value: serde_json::Value, reserved: &[&Path]) -> Result<serde_json::Value> {
    let text = match &value {
        serde_json::Value::Null => return Ok(value),
        serde_json::Value::String(text) => text.trim(),
        _ => bail!(
            "{} must be a folder path or null, got {}",
            FIELD_DATA_DIR_OVERRIDE,
            value
        ),
    };
    if text.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    if !Path::new(text).is_absolute() {
        bail!(
            "{} must be an absolute folder path, got {:?}",
            FIELD_DATA_DIR_OVERRIDE,
            text
        );
    }
    if let Some(root) = reserved
        .iter()
        .find(|root| is_same_or_inside(Path::new(text), root))
    {
        bail!(
            "{} {:?} is inside {}, which the app clears; choose a folder outside it",
            FIELD_DATA_DIR_OVERRIDE,
            text,
            root.display()
        );
    }
    fs::create_dir_all(text)
        .with_context(|| format!("{} {:?} cannot be created", FIELD_DATA_DIR_OVERRIDE, text))?;
    tempfile::tempfile_in(text)
        .with_context(|| format!("{} {:?} is not writable", FIELD_DATA_DIR_OVERRIDE, text))?;
    Ok(serde_json::Value::String(text.to_owned()))
}

fn is_same_or_inside(candidate: &Path, root: &Path) -> bool {
    let root = comparable_components(root);
    !root.is_empty() && comparable_components(candidate).starts_with(&root)
}

fn comparable_components(path: &Path) -> Vec<String> {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut anchor = Vec::new();
    let mut names = Vec::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => anchor.push(folded(&prefix_text(prefix.kind()))),
            Component::RootDir => anchor.push(folded(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                names.pop();
            }
            Component::Normal(name) => names.push(folded(&name.to_string_lossy())),
        }
    }
    anchor.extend(names);
    anchor
}

fn prefix_text(prefix: Prefix) -> String {
    match prefix {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => format!("{}:", char::from(drive)),
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            format!(
                r"\\{}\{}",
                server.to_string_lossy(),
                share.to_string_lossy()
            )
        }
        Prefix::Verbatim(name) | Prefix::DeviceNS(name) => name.to_string_lossy().into_owned(),
    }
}

fn folded(text: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        text.to_lowercase()
    } else {
        text.to_owned()
    }
}

fn validated_mast_quota(value: serde_json::Value) -> Result<serde_json::Value> {
    if value.is_null() || value.as_f64().is_some_and(|gb| gb > 0.0) {
        Ok(value)
    } else {
        bail!(
            "{} must be a positive number of GB or null, got {}",
            FIELD_MAST_QUOTA_GB,
            value
        )
    }
}

fn validated_value(
    field: &str,
    value: serde_json::Value,
    reserved: &[&Path],
) -> Result<serde_json::Value> {
    match field {
        FIELD_DATA_DIR_OVERRIDE => validated_data_dir(value, reserved),
        FIELD_MAST_QUOTA_GB => validated_mast_quota(value),
        _ => Ok(value),
    }
}

fn validated_update(
    field: &str,
    value: serde_json::Value,
    reserved: &[&Path],
) -> Result<serde_json::Value> {
    refuse_managed_field(field)?;
    if field == FIELD_ASTROMETRY_API_URL {
        let candidate = value
            .as_str()
            .ok_or_else(|| anyhow!("{} must be a string", FIELD_ASTROMETRY_API_URL))?;
        ensure_https_api_url(candidate)?;
    }
    validated_value(field, value, reserved)
}

#[cfg(test)]
fn update_config_field_at(path: &Path, field: &str, value: serde_json::Value) -> Result<AppConfig> {
    let value = validated_update(field, value, &[])?;
    write_config_field_at(path, field, value)
}

fn write_config_field_at(path: &Path, field: &str, value: serde_json::Value) -> Result<AppConfig> {
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

pub fn update_config_field(
    field: &str,
    value: serde_json::Value,
    reserved: &[&Path],
) -> Result<AppConfig> {
    let value = validated_update(field, value, reserved)?;
    let path = config_path()?;
    let _guard = lock_config();
    write_config_field_at(&path, field, value)
}

pub fn set_mast_token_transport(value: Option<&str>) -> Result<AppConfig> {
    set_mast_token_transport_in(&config_dir()?, value)
}

pub fn set_mast_token_transport_in(dir: &Path, value: Option<&str>) -> Result<AppConfig> {
    fs::create_dir_all(dir).context("Failed to create config directory")?;
    let _guard = lock_config();
    set_mast_token_transport_at(&dir.join(CONFIG_FILE), value)
}

pub fn mast_token_transport_in(dir: &Path) -> Result<Option<String>> {
    let path = dir.join(CONFIG_FILE);
    if !path.exists() {
        return Ok(None);
    }
    Ok(load_config_from(&path)?.mast_token_transport)
}

pub(crate) fn set_mast_token_transport_at(path: &Path, value: Option<&str>) -> Result<AppConfig> {
    let stored = match value {
        Some(transport) if MAST_TOKEN_TRANSPORTS.contains(&transport) => serde_json::Value::String(transport.to_owned()),
        Some(transport) => bail!("Unknown MAST token transport {:?}", transport),
        None => serde_json::Value::Null,
    };
    write_config_field_at(path, FIELD_MAST_TOKEN_TRANSPORT, stored)
}

pub fn save_api_key(key: &str, service: &str) -> Result<()> {
    save_api_key_in(&config_dir()?, key, service)
}

pub fn save_api_key_in(dir: &Path, key: &str, service: &str) -> Result<()> {
    let path = api_key_path_in(dir, service)?;
    fs::create_dir_all(dir).context("Failed to create config directory")?;
    write_private(&path, key)
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .context("Failed to write API key")?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .context("Failed to restrict API key file permissions")?;
    file.write_all(contents.as_bytes()).context("Failed to write API key")
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).context("Failed to write API key")
}

pub fn delete_api_key(service: &str) -> Result<bool> {
    delete_api_key_in(&config_dir()?, service)
}

pub fn delete_api_key_in(dir: &Path, service: &str) -> Result<bool> {
    let path = api_key_path_in(dir, service)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).context("Failed to delete API key"),
    }
}

pub fn load_api_key(service: &str) -> Result<Option<String>> {
    load_api_key_in(&config_dir()?, service)
}

pub fn load_api_key_in(dir: &Path, service: &str) -> Result<Option<String>> {
    let path = api_key_path_in(dir, service)?;
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
        config_dir_from, delete_api_key_in, ensure_https_api_url, is_api_key_file, load_api_key_in, load_config_at, save_api_key_in,
        set_mast_token_transport_at, update_config_field, update_config_field_at, validate_api_key_service,
        write_config_file, CONFIG_BACKUP_EXTENSION, CONFIG_TEMP_EXTENSION, FIELD_ASTROMETRY_API_URL,
        FIELD_MAST_TOKEN_TRANSPORT,
    };
    use super::{validated_data_dir, FIELD_DATA_DIR_OVERRIDE};
    use crate::types::config::AppConfig;
    use serde_json::json;

    fn config_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("config.json")
    }

    #[test]
    fn a_redirected_config_dir_replaces_the_system_one() {
        let system = Some(std::path::PathBuf::from("roaming"));
        assert_eq!(config_dir_from(None, system.clone()), Some(std::path::PathBuf::from("roaming").join("astroburst")));
        let scratch = std::path::Path::new("scratch").join("config");
        assert_eq!(config_dir_from(Some(&scratch), system), Some(scratch.clone()));
        assert_eq!(config_dir_from(Some(&scratch), None), Some(scratch));
        assert_eq!(config_dir_from(None, None), None);
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
        let err = update_config_field(FIELD_ASTROMETRY_API_URL, json!("http://evil.example"), &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("http://evil.example"), "error was: {}", err);
    }

    #[test]
    fn update_config_field_refuses_a_non_string_api_url() {
        assert!(update_config_field(FIELD_ASTROMETRY_API_URL, json!(42), &[]).is_err());
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

    #[test]
    fn api_key_files_are_recognised_in_any_spelling() {
        for refused in [
            "C:/Users/u/AppData/Roaming/astroburst/mast_api_key.txt",
            r"C:\Users\u\AppData\Roaming\astroburst\ASTROMETRY_API_KEY.TXT",
            "/home/u/.config/astroburst/mast_api_key.txt",
            "mast_api_key.txt",
        ] {
            assert!(is_api_key_file(refused), "{refused}");
        }
        for served in ["C:/x/m16_thumb.jpg", "C:/x/api_key.txt.png", "/tmp/notes_api_key.txt.jpg", ""] {
            assert!(!is_api_key_file(served), "{served}");
        }
        let dir = tempfile::tempdir().unwrap();
        save_api_key_in(dir.path(), "k", "mast").unwrap();
        let written = std::fs::read_dir(dir.path()).unwrap().next().unwrap().unwrap().path();
        assert!(is_api_key_file(&written.to_string_lossy()));
    }

    #[cfg(unix)]
    #[test]
    fn api_key_files_are_private_from_the_first_byte() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("mast_api_key.txt");
        std::fs::write(&existing, "old").unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o644)).unwrap();
        save_api_key_in(dir.path(), "new-key", "mast").unwrap();
        assert_eq!(std::fs::metadata(&existing).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "new-key");
        save_api_key_in(dir.path(), "fresh", "astrometry").unwrap();
        let fresh = dir.path().join("astrometry_api_key.txt");
        assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn api_key_service_name_is_validated() {
        for bad in ["../x", "MAST", "", "mast/../x", "a b", "mast.txt", "x\\y"] {
            assert!(validate_api_key_service(bad).is_err(), "{bad:?} was accepted");
        }
        assert!(validate_api_key_service(&"a".repeat(33)).is_err());
        assert!(validate_api_key_service(&"a".repeat(32)).is_ok());
        for good in ["astrometry", "mast", "a_b"] {
            assert!(validate_api_key_service(good).is_ok(), "{good:?} was refused");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(save_api_key_in(dir.path(), "k", "../x").is_err());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn api_keys_round_trip_and_delete_in_an_explicit_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_api_key_in(dir.path(), "mast").unwrap(), None);
        save_api_key_in(dir.path(), "  secret-value \n", "mast").unwrap();
        assert_eq!(load_api_key_in(dir.path(), "mast").unwrap().as_deref(), Some("secret-value"));
        assert!(dir.path().join("mast_api_key.txt").is_file());
        assert!(delete_api_key_in(dir.path(), "mast").unwrap());
        assert!(!delete_api_key_in(dir.path(), "mast").unwrap());
        assert_eq!(load_api_key_in(dir.path(), "mast").unwrap(), None);
    }

    #[test]
    fn update_config_refuses_mast_token_transport() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        let err = update_config_field_at(&path, FIELD_MAST_TOKEN_TRANSPORT, json!("cookie")).unwrap_err().to_string();
        assert!(err.contains(FIELD_MAST_TOKEN_TRANSPORT), "{err}");
        assert!(!path.exists(), "the refused update touched the config file");
        set_mast_token_transport_at(&path, Some("header")).unwrap();
        assert!(update_config_field_at(&path, FIELD_MAST_TOKEN_TRANSPORT, serde_json::Value::Null).is_err());
        assert_eq!(load_config_at(&path).unwrap().mast_token_transport.as_deref(), Some("header"));
    }

    #[test]
    fn mast_token_transport_is_written_only_through_its_setter() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        let updated = set_mast_token_transport_at(&path, Some("cookie")).unwrap();
        assert_eq!(updated.mast_token_transport.as_deref(), Some("cookie"));
        assert_eq!(load_config_at(&path).unwrap().mast_token_transport.as_deref(), Some("cookie"));
        assert!(set_mast_token_transport_at(&path, Some("query")).is_err());
        assert_eq!(load_config_at(&path).unwrap().mast_token_transport.as_deref(), Some("cookie"));
        set_mast_token_transport_at(&path, None).unwrap();
        assert_eq!(load_config_at(&path).unwrap().mast_token_transport, None);
    }

    #[test]
    fn default_capability_denies_api_key_files() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities").join("default.json");
        let capability: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let permissions = capability["permissions"].as_array().unwrap();
        let denied = permissions.iter().any(|permission| {
            permission["identifier"] == "fs:scope"
                && permission["deny"]
                    .as_array()
                    .is_some_and(|deny| deny.iter().any(|entry| entry["path"] == "$CONFIG/astroburst/*_api_key.txt"))
        });
        assert!(denied, "no fs:scope deny entry for the API key files");
    }

    fn stored_json(path: &std::path::Path) -> serde_json::Value {
        serde_json::to_value(load_config_at(path).unwrap()).unwrap()
    }

    #[test]
    fn the_data_folder_and_the_mast_quota_round_trip_through_update_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        let data = dir.path().join("data root").join("nested");
        let data_text = data.to_string_lossy().into_owned();
        update_config_field_at(
            &path,
            "data_dir_override",
            json!(format!("  {data_text}  ")),
        )
        .unwrap();
        update_config_field_at(&path, "mast_quota_gb", json!(0.5)).unwrap();
        assert!(data.is_dir(), "the data folder was not created");
        assert_eq!(
            std::fs::read_dir(&data).unwrap().count(),
            0,
            "the write check left a file behind"
        );
        let stored = stored_json(&path);
        assert_eq!(stored["data_dir_override"], json!(data_text));
        assert_eq!(stored["mast_quota_gb"], json!(0.5));

        update_config_field_at(&path, "data_dir_override", json!("")).unwrap();
        update_config_field_at(&path, "mast_quota_gb", serde_json::Value::Null).unwrap();
        let cleared = stored_json(&path);
        assert_eq!(cleared["data_dir_override"], serde_json::Value::Null);
        assert_eq!(cleared["mast_quota_gb"], serde_json::Value::Null);
        assert_eq!(
            serde_json::to_value(AppConfig::default()).unwrap()["mast_quota_gb"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn a_relative_or_uncreatable_data_folder_is_refused_before_the_config_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        let blocker = dir.path().join("a file");
        std::fs::write(&blocker, "x").unwrap();
        let under_a_file = blocker.join("data").to_string_lossy().into_owned();
        for refused in [
            json!("relative/data"),
            json!("data"),
            json!(under_a_file),
            json!(42),
            json!(["C:/data"]),
        ] {
            let err = update_config_field_at(&path, "data_dir_override", refused.clone())
                .unwrap_err()
                .to_string();
            assert!(err.contains("data_dir_override"), "{refused}: {err}");
            assert!(!path.exists(), "{refused} touched the config file");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let read_only = dir.path().join("read-only");
            std::fs::create_dir(&read_only).unwrap();
            std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o555)).unwrap();
            let writable_anyway = tempfile::tempfile_in(&read_only).is_ok();
            let outcome = update_config_field_at(
                &path,
                "data_dir_override",
                json!(read_only.to_string_lossy()),
            );
            std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o755)).unwrap();
            if !writable_anyway {
                let err = outcome.unwrap_err().to_string();
                assert!(err.contains("data_dir_override"), "{err}");
                assert!(
                    !path.exists(),
                    "the read-only folder touched the config file"
                );
            }
        }
    }

    #[test]
    fn a_negative_zero_or_non_numeric_mast_quota_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        for refused in [json!(-1.0), json!(0), json!("20"), json!(true)] {
            let err = update_config_field_at(&path, "mast_quota_gb", refused.clone())
                .unwrap_err()
                .to_string();
            assert!(err.contains("mast_quota_gb"), "{refused}: {err}");
            assert!(!path.exists(), "{refused} touched the config file");
        }
        update_config_field_at(&path, "mast_quota_gb", json!(20)).unwrap();
        assert_eq!(stored_json(&path)["mast_quota_gb"], json!(20.0));
    }

    #[test]
    fn a_data_folder_inside_a_folder_the_app_clears_is_refused() {
        let outer = tempfile::tempdir().unwrap();
        let app_data = outer.path().join("com.astroburst.desktop.roaming");
        let app_local = outer.path().join("com.astroburst.desktop");
        let reserved = [app_data.as_path(), app_local.as_path()];
        let text = |path: &std::path::Path| path.to_string_lossy().into_owned();

        let mut refused = vec![
            text(&app_local),
            text(&app_data.join("output")),
            text(&app_local.join("mast").join("JWST")),
            format!(
                "{}{}..",
                text(&app_local.join("mast")),
                std::path::MAIN_SEPARATOR
            ),
        ];
        if cfg!(windows) {
            refused.push(text(&app_local).to_uppercase());
            refused.push(text(&app_data.join("output")).replace('\\', "/"));
            refused.push(format!(r"\\?\{}", text(&app_local.join("mast"))));
            refused.push(format!("{}.", text(&app_local)));
        }
        for folder in refused {
            let err = validated_data_dir(json!(folder.clone()), &reserved)
                .unwrap_err()
                .to_string();
            assert!(err.contains(FIELD_DATA_DIR_OVERRIDE), "{folder}: {err}");
            let err = update_config_field(
                FIELD_DATA_DIR_OVERRIDE,
                json!(format!(" {folder} ")),
                &reserved,
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains(FIELD_DATA_DIR_OVERRIDE), "{folder}: {err}");
        }
        assert!(
            !app_local.exists() && !app_data.exists(),
            "a refused folder was created"
        );

        let sibling = outer.path().join("com.astroburst.desktop-downloads");
        assert_eq!(
            validated_data_dir(json!(text(&sibling)), &reserved).unwrap(),
            json!(text(&sibling))
        );
        let parent = text(outer.path());
        assert_eq!(
            validated_data_dir(json!(parent.clone()), &reserved).unwrap(),
            json!(parent)
        );
    }

    #[test]
    fn a_data_folder_check_runs_before_the_config_lock_is_taken() {
        let held = super::lock_config();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = update_config_field(FIELD_DATA_DIR_OVERRIDE, json!("relative/data"), &[]);
            let _ = sender.send(outcome.map(|_| ()).map_err(|e| e.to_string()));
        });
        let outcome = receiver.recv_timeout(std::time::Duration::from_secs(5));
        drop(held);
        let err = outcome
            .expect("the refused data folder waited for the config lock")
            .unwrap_err();
        assert!(err.contains(FIELD_DATA_DIR_OVERRIDE), "{err}");
    }

    #[test]
    fn an_unknown_field_lists_the_storage_settings_among_the_valid_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(&dir);
        let err = update_config_field_at(&path, "data_dir", json!("C:/data"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("data_dir_override"), "{err}");
        assert!(err.contains("mast_quota_gb"), "{err}");
    }
}
