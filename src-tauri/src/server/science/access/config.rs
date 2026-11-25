use std::fmt;
use std::net::{Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use super::guard::host_name;

#[derive(Clone)]
pub(crate) struct Secret(String);

impl Secret {
    fn new(raw: String) -> Option<Self> {
        let value = raw.trim();
        (!value.is_empty()).then(|| Self(value.to_owned()))
    }

    pub(crate) fn matches(&self, presented: &[u8]) -> bool {
        let expected = Sha256::digest(self.0.as_bytes());
        let given = Sha256::digest(presented);
        expected
            .iter()
            .zip(given.iter())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScienceConfig {
    pub(crate) bind: SocketAddr,
    pub(crate) token: Option<Secret>,
    pub(crate) data_root: PathBuf,
    pub(crate) output_root: PathBuf,
    pub(crate) allowed_hosts: Vec<String>,
    #[cfg(feature = "mast")]
    pub(crate) mast: MastServerConfig,
}

#[cfg(feature = "mast")]
#[derive(Clone, Debug)]
pub(crate) struct MastServerConfig {
    pub(crate) download_dir: PathBuf,
    pub(crate) token: Option<Secret>,
}

impl ScienceConfig {
    pub(crate) fn from_env(bind: SocketAddr) -> Self {
        Self::from_lookup(bind, |name| std::env::var(name).ok())
    }

    pub(crate) fn from_lookup(bind: SocketAddr, lookup: impl Fn(&str) -> Option<String>) -> Self {
        let set = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
        let data_root = server_root(set("ASTROBURST_SERVER_ROOT"));
        let dir = |name: &str, default: &str| {
            set(name)
                .map(PathBuf::from)
                .unwrap_or_else(|| data_root.join(default))
        };
        Self {
            bind,
            token: set("ASTROBURST_TOKEN").and_then(Secret::new),
            output_root: dir("ASTROBURST_OUTPUT_ROOT", "out"),
            allowed_hosts: allowed_hosts(set("ASTROBURST_ALLOWED_HOSTS").unwrap_or_default()),
            #[cfg(feature = "mast")]
            mast: MastServerConfig {
                download_dir: dir("ASTROBURST_MAST_DOWNLOAD_DIR", "mast"),
                token: set("ASTROBURST_MAST_TOKEN").and_then(Secret::new),
            },
            data_root,
        }
    }

    pub(crate) fn guarded(&self) -> bool {
        !self.bind.ip().is_loopback()
    }

    pub(crate) fn roots(&self) -> Vec<PathBuf> {
        [
            &self.data_root,
            &self.output_root,
            #[cfg(feature = "mast")]
            &self.mast.download_dir,
        ]
        .into_iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .filter(|root| root.is_dir())
        .collect()
    }
}

fn server_root(configured: Option<String>) -> PathBuf {
    let path = configured
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    match std::fs::canonicalize(&path) {
        Ok(root) if root.is_dir() => root,
        _ => {
            log::warn!(
                "ASTROBURST_SERVER_ROOT {} is not a usable folder; no path is accepted until it is fixed",
                path.display()
            );
            path
        }
    }
}

fn allowed_hosts(raw: String) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let host = match entry.parse::<Ipv6Addr>() {
                Ok(ip) => Some(format!("[{ip}]")),
                Err(_) => host_name(entry),
            };
            if host.is_none() {
                log::warn!("ASTROBURST_ALLOWED_HOSTS entry {entry:?} is not a host name; ignored");
            }
            host
        })
        .collect()
}
