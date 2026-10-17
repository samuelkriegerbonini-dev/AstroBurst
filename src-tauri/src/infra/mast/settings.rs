use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Url;

use super::client::{RedirectAllowlist, DEFAULT_BASE_URL};
use crate::core::mast::errors::OpKind;

pub const DEFAULT_AUTH_URL: &str = "https://auth.mast.stsci.edu";
pub const ENV_MAST_URL: &str = "ASTROBURST_MAST_URL";
pub const ENV_MAST_AUTH_URL: &str = "ASTROBURST_MAST_AUTH_URL";
pub const ENV_MAST_TIMEOUT_MS: &str = "ASTROBURST_MAST_TIMEOUT_MS";
pub const ENV_MAST_RETRY_MS: &str = "ASTROBURST_MAST_RETRY_MS";
pub const ENV_TEST_DATA_DIR: &str = "ASTROBURST_TEST_DATA_DIR";
pub const TEST_CONFIG_DIR: &str = "config";
pub const TEST_LOCAL_DIR: &str = "local";
pub const DEFAULT_RETRY_BACKOFF: Duration = Duration::from_secs(1);
pub const CACHE_DIR: &str = "mast";
pub const TEST_CACHE_DIR: &str = "mast-test";
pub const LOOPBACK_HOST: &str = "127.0.0.1";
pub const STSCI_DOMAIN: &str = "stsci.edu";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostOverride {
    Https(String),
    Loopback(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MastSettings {
    pub base_url: String,
    pub auth_url: String,
    pub base_override: Option<HostOverride>,
    pub auth_override: Option<HostOverride>,
    pub timeout_override: Option<Duration>,
    pub retry_backoff: Duration,
    pub test_data_dir: Option<PathBuf>,
}

impl Default for MastSettings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            auth_url: DEFAULT_AUTH_URL.to_owned(),
            base_override: None,
            auth_override: None,
            timeout_override: None,
            retry_backoff: DEFAULT_RETRY_BACKOFF,
            test_data_dir: None,
        }
    }
}

pub fn parse_override(value: &str) -> Option<(String, HostOverride)> {
    let url = Url::parse(value.trim()).ok()?;
    let clean = url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/");
    if !clean {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    match url.scheme() {
        "https" if host == STSCI_DOMAIN || host.ends_with(&format!(".{STSCI_DOMAIN}")) => {
            let base = match url.port() {
                Some(port) => format!("https://{host}:{port}"),
                None => format!("https://{host}"),
            };
            Some((base, HostOverride::Https(host)))
        }
        "http" if host == LOOPBACK_HOST => {
            let port = url.port()?;
            Some((format!("http://{LOOPBACK_HOST}:{port}"), HostOverride::Loopback(port)))
        }
        _ => None,
    }
}

pub fn parse_timeout_ms(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().filter(|ms| *ms > 0).map(Duration::from_millis)
}

pub const fn default_timeout(kind: OpKind) -> Duration {
    let secs = match kind {
        OpKind::Search | OpKind::Preview | OpKind::Products => 120,
        OpKind::ImageList => 60,
        OpKind::Probe | OpKind::Lookup | OpKind::Thumbnail | OpKind::AuthCheck | OpKind::Info => 30,
    };
    Duration::from_secs(secs)
}

fn read_var(lookup: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    lookup(name).map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

impl MastSettings {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let mut settings = Self::default();
        if let Some(raw) = read_var(&lookup, ENV_MAST_URL) {
            match parse_override(&raw) {
                Some((base, host)) => {
                    settings.base_url = base;
                    settings.base_override = Some(host);
                }
                None => log::warn!("{ENV_MAST_URL} ignored: expected https://<host>.stsci.edu or http://127.0.0.1:<port>"),
            }
        }
        if let Some(raw) = read_var(&lookup, ENV_MAST_AUTH_URL) {
            match parse_override(&raw) {
                Some((base, host)) => {
                    settings.auth_url = base;
                    settings.auth_override = Some(host);
                }
                None => log::warn!("{ENV_MAST_AUTH_URL} ignored: expected https://<host>.stsci.edu or http://127.0.0.1:<port>"),
            }
        }
        if let Some(raw) = read_var(&lookup, ENV_MAST_TIMEOUT_MS) {
            match parse_timeout_ms(&raw) {
                Some(timeout) => settings.timeout_override = Some(timeout),
                None => log::warn!("{ENV_MAST_TIMEOUT_MS} ignored: expected a positive number of milliseconds"),
            }
        }
        if let Some(raw) = read_var(&lookup, ENV_MAST_RETRY_MS) {
            match parse_timeout_ms(&raw) {
                Some(backoff) => settings.retry_backoff = backoff,
                None => log::warn!("{ENV_MAST_RETRY_MS} ignored: expected a positive number of milliseconds"),
            }
        }
        if let Some(raw) = read_var(&lookup, ENV_TEST_DATA_DIR) {
            let dir = PathBuf::from(raw);
            match (&settings.base_override, dir.is_absolute()) {
                (Some(_), true) => settings.test_data_dir = Some(dir),
                (Some(_), false) => log::warn!("{ENV_TEST_DATA_DIR} ignored: expected an absolute path"),
                (None, _) => log::warn!("{ENV_TEST_DATA_DIR} ignored: it is honoured only together with {ENV_MAST_URL}"),
            }
        }
        settings
    }

    pub fn config_dir(&self) -> Option<PathBuf> {
        self.test_data_dir.as_ref().map(|dir| dir.join(TEST_CONFIG_DIR))
    }

    pub fn local_data_dir(&self, app_local: Option<PathBuf>) -> Option<PathBuf> {
        self.test_data_dir.as_ref().map(|dir| dir.join(TEST_LOCAL_DIR)).or(app_local)
    }

    pub fn timeout(&self, kind: OpKind) -> Duration {
        self.timeout_override.unwrap_or(default_timeout(kind))
    }

    pub fn cache_dir_name(&self) -> &'static str {
        if self.base_override.is_some() {
            TEST_CACHE_DIR
        } else {
            CACHE_DIR
        }
    }

    pub fn redirect_allowlist(&self) -> RedirectAllowlist {
        let list = RedirectAllowlist::mast();
        match &self.base_override {
            Some(HostOverride::Https(host)) => list.allow_https_host(host),
            Some(HostOverride::Loopback(port)) => list.allow_loopback(LOOPBACK_HOST, *port),
            None => list,
        }
    }
}

static SETTINGS: OnceLock<MastSettings> = OnceLock::new();

pub fn init_from_env() {
    let settings = SETTINGS.get_or_init(|| MastSettings::from_lookup(|name| std::env::var(name).ok()));
    if let Some(dir) = settings.config_dir() {
        crate::infra::config::redirect_config_dir(dir);
    }
}

pub fn settings() -> &'static MastSettings {
    SETTINGS.get_or_init(MastSettings::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn with(vars: &[(&str, &str)]) -> MastSettings {
        let map: HashMap<String, String> = vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        MastSettings::from_lookup(|name| map.get(name).cloned())
    }

    #[test]
    fn settings_accept_only_https_or_loopback_overrides() {
        let defaults = with(&[]);
        assert_eq!(defaults, MastSettings::default());
        assert_eq!(defaults.cache_dir_name(), CACHE_DIR);
        assert_eq!(defaults.timeout(OpKind::Search), Duration::from_secs(120));
        assert_eq!(defaults.timeout(OpKind::Lookup), Duration::from_secs(30));
        assert_eq!(defaults.timeout(OpKind::Probe), Duration::from_secs(30));
        assert_eq!(defaults.timeout(OpKind::ImageList), Duration::from_secs(60));
        assert_eq!(defaults.timeout(OpKind::Thumbnail), Duration::from_secs(30));
        assert_eq!(defaults.timeout(OpKind::Preview), Duration::from_secs(120));
        assert_eq!(defaults.timeout(OpKind::Products), Duration::from_secs(120));
        assert_eq!(defaults.timeout(OpKind::AuthCheck), Duration::from_secs(30));
        assert_eq!(defaults.timeout(OpKind::Info), Duration::from_secs(30));
        assert_eq!(defaults.retry_backoff, Duration::from_secs(1));

        let canned = with(&[
            (ENV_MAST_URL, "http://127.0.0.1:43123/"),
            (ENV_MAST_AUTH_URL, "http://127.0.0.1:43123"),
            (ENV_MAST_TIMEOUT_MS, "4000"),
            (ENV_MAST_RETRY_MS, "250"),
        ]);
        assert_eq!(canned.retry_backoff, Duration::from_millis(250));
        assert_eq!(canned.base_url, "http://127.0.0.1:43123");
        assert_eq!(canned.auth_url, "http://127.0.0.1:43123");
        assert_eq!(canned.base_override, Some(HostOverride::Loopback(43123)));
        assert_eq!(canned.cache_dir_name(), TEST_CACHE_DIR);
        for kind in [OpKind::Search, OpKind::Thumbnail, OpKind::Info] {
            assert_eq!(canned.timeout(kind), Duration::from_millis(4000));
        }
        let list = canned.redirect_allowlist();
        assert!(list.allows(&Url::parse("http://127.0.0.1:43123/s3/x_thumb.jpg").unwrap()));
        assert!(!list.allows(&Url::parse("http://127.0.0.1:43124/x").unwrap()));
        assert!(list.allows(&Url::parse("https://stpubdata.s3.us-east-1.amazonaws.com/x").unwrap()));

        let mirror = with(&[(ENV_MAST_URL, "https://MastDev.STScI.edu"), (ENV_MAST_AUTH_URL, "https://auth.mastdev.stsci.edu:8443")]);
        assert_eq!(mirror.base_url, "https://mastdev.stsci.edu");
        assert_eq!(mirror.base_override, Some(HostOverride::Https("mastdev.stsci.edu".into())));
        assert_eq!(mirror.cache_dir_name(), TEST_CACHE_DIR);
        assert!(mirror.redirect_allowlist().allows(&Url::parse("https://mastdev.stsci.edu/x").unwrap()));
        assert_eq!(mirror.auth_url, "https://auth.mastdev.stsci.edu:8443");

        for rejected in [
            "https://mirror.example.org",
            "https://stsci.edu.example.org",
            "https://evilstsci.edu",
            "http://mast.stsci.edu",
            "http://localhost:8080",
            "http://127.0.0.1",
            "http://127.0.0.2:8080",
            "https://user:pw@mast.example.org",
            "https://mast.example.org/path",
            "https://mast.example.org/?q=1",
            "ftp://mast.example.org",
            "file:///C:/x",
            "not a url",
            "",
        ] {
            let settings = with(&[(ENV_MAST_URL, rejected), (ENV_MAST_AUTH_URL, rejected)]);
            assert_eq!(settings.base_url, DEFAULT_BASE_URL, "{rejected}");
            assert_eq!(settings.auth_url, DEFAULT_AUTH_URL, "{rejected}");
            assert_eq!(settings.base_override, None, "{rejected}");
            assert_eq!(settings.cache_dir_name(), CACHE_DIR, "{rejected}");
        }
        for rejected in ["0", "-5", "abc", "1.5", ""] {
            assert_eq!(with(&[(ENV_MAST_TIMEOUT_MS, rejected)]).timeout_override, None, "{rejected}");
            assert_eq!(with(&[(ENV_MAST_RETRY_MS, rejected)]).retry_backoff, DEFAULT_RETRY_BACKOFF, "{rejected}");
        }
    }

    #[test]
    fn test_data_dir_is_honoured_only_for_a_test_server() {
        let scratch = std::env::temp_dir().join("ab-cdp-canned");
        let dir = scratch.to_string_lossy().into_owned();
        let app_local = Some(PathBuf::from("app-local"));
        for url in ["http://127.0.0.1:43123", "https://mastdev.stsci.edu", "https://mast.stsci.edu"] {
            let test = with(&[(ENV_MAST_URL, url), (ENV_TEST_DATA_DIR, &dir)]);
            assert_eq!(test.test_data_dir.as_deref(), Some(scratch.as_path()), "{url}");
            assert_eq!(test.config_dir(), Some(scratch.join(TEST_CONFIG_DIR)), "{url}");
            assert_eq!(test.local_data_dir(app_local.clone()), Some(scratch.join(TEST_LOCAL_DIR)), "{url}");
            assert_eq!(test.cache_dir_name(), TEST_CACHE_DIR, "{url}");
        }
        for (url, value) in [
            (None, dir.as_str()),
            (Some("http://localhost:8080"), dir.as_str()),
            (Some("https://mirror.example.org"), dir.as_str()),
            (Some("http://127.0.0.1:43123"), "relative/data"),
            (Some("http://127.0.0.1:43123"), ""),
        ] {
            let mut vars = vec![(ENV_TEST_DATA_DIR, value)];
            if let Some(url) = url {
                vars.push((ENV_MAST_URL, url));
            }
            let normal = with(&vars);
            assert_eq!(normal.test_data_dir, None, "{url:?} {value:?}");
            assert_eq!(normal.config_dir(), None, "{url:?} {value:?}");
            assert_eq!(normal.local_data_dir(app_local.clone()), app_local, "{url:?} {value:?}");
        }
    }
}
