// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub bind: SocketAddr,

    pub session_ttl: Duration,

    pub session_max: usize,

    pub jobs_max: usize,

    pub cache_max_entries: usize,

    pub cache_max_bytes: usize,

    pub cleanup_interval: Duration,

    pub log_level: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".parse().expect("default bind is valid"),
            session_ttl: Duration::from_secs(900),
            session_max: 8,
            jobs_max: 4,
            cache_max_entries: 32,
            cache_max_bytes: 2 * 1024 * 1024 * 1024,
            cleanup_interval: Duration::from_secs(60),
            log_level: "info".to_owned(),
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            bind: env_or("ASTROBURST_BIND", d.bind),
            session_ttl: Duration::from_secs(env_or::<u64>(
                "ASTROBURST_SESSION_TTL",
                d.session_ttl.as_secs(),
            )),
            session_max: env_or("ASTROBURST_SESSION_MAX", d.session_max),
            jobs_max: env_or("ASTROBURST_JOBS_MAX", d.jobs_max),
            cache_max_entries: env_or("ASTROBURST_CACHE_MAX_ENTRIES", d.cache_max_entries),
            cache_max_bytes: env_or("ASTROBURST_CACHE_MAX_BYTES", d.cache_max_bytes),
            cleanup_interval: Duration::from_secs(env_or::<u64>(
                "ASTROBURST_CLEANUP_INTERVAL",
                d.cleanup_interval.as_secs(),
            )),
            log_level: env_or("ASTROBURST_LOG_LEVEL", d.log_level),
        }
    }
}

fn env_or<T>(var: &str, default: T) -> T
where
    T: FromStr + std::fmt::Debug,
    T::Err: std::fmt::Debug,
{
    match std::env::var(var) {
        Err(_) => default,
        Ok(raw) => raw.parse().unwrap_or_else(|e| {
            eprintln!(
                "WARN: {var}={raw:?} is invalid ({e:?}); using default {default:?}"
            );
            default
        }),
    }
}
