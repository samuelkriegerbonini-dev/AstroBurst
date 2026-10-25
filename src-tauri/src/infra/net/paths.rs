use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(feature = "mast")]
use crate::infra::mast::settings::MastSettings;

pub const ENV_DATA_DIR: &str = "ASTROBURST_DATA_DIR";
#[cfg(feature = "mast")]
pub const DEFAULT_MAST_QUOTA_GB: f64 = 20.0;
#[cfg(feature = "mast")]
const BYTES_PER_GB: f64 = 1e9;
const APP_DIR: &str = "astroburst";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataRootSource {
    Env,
    TestData,
    Configured,
    Platform,
}

impl DataRootSource {
    pub const fn chosen_by_user(self) -> bool {
        matches!(self, DataRootSource::Env | DataRootSource::Configured)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoot {
    pub path: PathBuf,
    pub source: DataRootSource,
}

impl DataRoot {
    pub fn require_present(&self) -> Result<&Path, String> {
        if self.source.chosen_by_user() && !self.path.is_dir() {
            return Err(format!(
                "the Data folder {} does not exist; connect its drive or choose another folder in Settings",
                self.path.display()
            ));
        }
        Ok(&self.path)
    }
}

pub fn data_root() -> PathBuf {
    try_data_root().unwrap_or_else(|error| {
        let fallback = resolve_data_root(
            std::env::var_os(ENV_DATA_DIR),
            test_data_dir(),
            None,
            platform_data_dir(),
        )
        .path;
        log::warn!(
            "the Data folder setting could not be read ({error:#}); using {}",
            fallback.display()
        );
        fallback
    })
}

pub fn try_data_root() -> anyhow::Result<PathBuf> {
    resolved_data_root().map(|root| root.path)
}

pub fn resolved_data_root() -> anyhow::Result<DataRoot> {
    data_root_from(
        std::env::var_os(ENV_DATA_DIR),
        test_data_dir(),
        configured_data_dir,
        platform_data_dir(),
    )
}

fn configured_data_dir() -> anyhow::Result<Option<String>> {
    Ok(crate::infra::config::load_config()?.data_dir_override)
}

fn data_root_from(
    env: Option<OsString>,
    test: Option<PathBuf>,
    configured: impl FnOnce() -> anyhow::Result<Option<String>>,
    platform: Option<PathBuf>,
) -> anyhow::Result<DataRoot> {
    let decided = env
        .as_deref()
        .is_some_and(|dir| Path::new(dir).is_absolute())
        || test.as_deref().is_some_and(Path::is_absolute);
    if decided {
        return Ok(resolve_data_root(env, test, None, platform));
    }
    let configured = configured()?;
    Ok(resolve_data_root(env, test, configured.as_deref(), platform))
}

#[cfg(feature = "mast")]
fn test_data_dir() -> Option<PathBuf> {
    test_data_dir_in(crate::infra::mast::settings::settings())
}

#[cfg(feature = "mast")]
fn test_data_dir_in(settings: &MastSettings) -> Option<PathBuf> {
    settings
        .test_data_dir
        .as_ref()
        .map(|dir| dir.join(crate::infra::mast::settings::TEST_DATA_DIR))
}

#[cfg(not(feature = "mast"))]
fn test_data_dir() -> Option<PathBuf> {
    None
}

pub fn cache_root() -> PathBuf {
    app_dir_under(dirs::cache_dir())
}

#[cfg(feature = "mast")]
pub fn mast_root() -> PathBuf {
    mast_root_in(&data_root(), crate::infra::mast::settings::settings())
}

#[cfg(feature = "mast")]
pub fn mast_quota_bytes(configured_gb: Option<f64>) -> u64 {
    let gb = configured_gb
        .filter(|gb| gb.is_finite() && *gb > 0.0)
        .unwrap_or(DEFAULT_MAST_QUOTA_GB);
    (gb * BYTES_PER_GB).round() as u64
}

pub fn free_space(path: &Path) -> io::Result<u64> {
    let dir = path
        .ancestors()
        .find(|candidate| candidate.is_dir())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no existing folder at or above {}", path.display()),
            )
        })?;
    free_bytes(dir)
}

fn resolve_data_root(
    env: Option<OsString>,
    test: Option<PathBuf>,
    configured: Option<&str>,
    platform: Option<PathBuf>,
) -> DataRoot {
    let env = env
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .map(|path| DataRoot {
            path,
            source: DataRootSource::Env,
        });
    let test = test
        .filter(|dir| dir.is_absolute())
        .map(|path| DataRoot {
            path,
            source: DataRootSource::TestData,
        });
    let configured = configured
        .map(|text| PathBuf::from(text.trim()))
        .filter(|dir| dir.is_absolute())
        .map(|path| DataRoot {
            path,
            source: DataRootSource::Configured,
        });
    env.or(test).or(configured).unwrap_or_else(|| DataRoot {
        path: app_dir_under(platform),
        source: DataRootSource::Platform,
    })
}

fn app_dir_under(base: Option<PathBuf>) -> PathBuf {
    base.unwrap_or_else(std::env::temp_dir).join(APP_DIR)
}

#[cfg(windows)]
fn platform_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
}

#[cfg(not(windows))]
fn platform_data_dir() -> Option<PathBuf> {
    dirs::data_dir()
}

#[cfg(feature = "mast")]
fn mast_root_in(data_root: &Path, settings: &MastSettings) -> PathBuf {
    data_root.join(settings.cache_dir_name())
}

#[cfg(windows)]
fn free_bytes(dir: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    let separators = [u16::from(b'\\'), u16::from(b'/')];
    if !wide.last().is_some_and(|last| separators.contains(last)) {
        wide.push(separators[0]);
    }
    wide.push(0);
    let mut available: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(available)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn c_path(dir: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

#[cfg(target_os = "linux")]
fn free_bytes(dir: &Path) -> io::Result<u64> {
    let path = c_path(dir)?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(u64::from(stats.f_bavail).saturating_mul(u64::from(stats.f_frsize)))
}

#[cfg(target_os = "macos")]
fn free_bytes(dir: &Path) -> io::Result<u64> {
    let path = c_path(dir)?;
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut stats) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(stats.f_bavail.saturating_mul(u64::from(stats.f_bsize)))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn free_bytes(_dir: &Path) -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "free disk space is not available on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn root_of(env: Option<OsString>, configured: Option<&str>, platform: Option<PathBuf>) -> PathBuf {
        resolve_data_root(env, None, configured, platform).path
    }

    #[test]
    fn a_chosen_data_folder_that_is_gone_is_refused_and_a_default_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("unplugged");
        let root = |path: &Path, source| DataRoot {
            path: path.to_path_buf(),
            source,
        };
        assert_eq!(root(dir.path(), DataRootSource::Configured).require_present(), Ok(dir.path()));
        for source in [DataRootSource::Platform, DataRootSource::TestData] {
            assert_eq!(root(&gone, source).require_present(), Ok(gone.as_path()), "{source:?}");
        }
        for source in [DataRootSource::Configured, DataRootSource::Env] {
            assert_eq!(
                root(&gone, source).require_present(),
                Err(format!(
                    "the Data folder {} does not exist; connect its drive or choose another folder in Settings",
                    gone.display()
                )),
                "{source:?}"
            );
        }
        assert!(!gone.exists());
    }

    #[test]
    fn the_env_folder_wins_over_the_configured_one_and_the_platform_default() {
        let env = absolute("ab-env-root");
        let configured = absolute("ab-configured-root");
        let configured_text = configured.to_string_lossy().into_owned();
        let platform = Some(absolute("ab-local-app-data"));
        let default_root = absolute("ab-local-app-data").join("astroburst");

        assert_eq!(
            root_of(
                Some(env.clone().into_os_string()),
                Some(&configured_text),
                platform.clone()
            ),
            env
        );
        assert_eq!(
            root_of(None, Some(&configured_text), platform.clone()),
            configured
        );
        assert_eq!(
            root_of(
                None,
                Some(&format!("  {configured_text} ")),
                platform.clone()
            ),
            configured
        );
        assert_eq!(
            root_of(None, None, platform.clone()),
            default_root
        );

        assert_eq!(
            root_of(
                Some(OsString::from("relative/env")),
                Some(&configured_text),
                platform.clone()
            ),
            configured
        );
        assert_eq!(
            root_of(Some(OsString::new()), None, platform.clone()),
            default_root
        );
        assert_eq!(
            root_of(None, Some("relative/config"), platform.clone()),
            default_root
        );
        assert_eq!(root_of(None, Some(""), platform), default_root);

        let fallback = root_of(None, None, None);
        assert!(fallback.is_absolute(), "{}", fallback.display());
        assert!(fallback.ends_with("astroburst"), "{}", fallback.display());
    }

    #[test]
    fn an_unreadable_config_is_an_error_unless_the_env_folder_already_decides() {
        let env = absolute("ab-env-root");
        let configured = absolute("ab-configured-root");
        let platform = Some(absolute("ab-local-app-data"));
        let unreadable = || -> anyhow::Result<Option<String>> {
            anyhow::bail!("config.json is locked by another process")
        };
        let never_read = || -> anyhow::Result<Option<String>> {
            panic!("the config was read although the env folder decides")
        };

        assert_eq!(
            data_root_from(
                Some(env.clone().into_os_string()),
                None,
                never_read,
                platform.clone()
            )
            .unwrap()
            .path,
            env
        );
        let err = data_root_from(None, None, unreadable, platform.clone()).unwrap_err();
        assert!(format!("{err:#}").contains("locked"), "{err:#}");
        assert!(data_root_from(
            Some(OsString::from("relative/env")),
            None,
            unreadable,
            platform.clone()
        )
        .is_err());
        let configured_text = configured.to_string_lossy().into_owned();
        assert_eq!(
            data_root_from(None, None, || Ok(Some(configured_text)), platform.clone())
                .unwrap()
                .path,
            configured
        );
        assert_eq!(
            data_root_from(None, None, || Ok(None), platform)
                .unwrap()
                .path,
            absolute("ab-local-app-data").join("astroburst")
        );
    }

    #[test]
    fn the_test_data_folder_sits_below_the_env_folder_and_above_the_configured_one() {
        let env = absolute("ab-env-root");
        let test = absolute("ab-cdp-canned").join("data");
        let configured = absolute("ab-configured-root");
        let configured_text = configured.to_string_lossy().into_owned();
        let platform = Some(absolute("ab-local-app-data"));
        let never_read = || -> anyhow::Result<Option<String>> {
            panic!("the config was read although the test folder decides")
        };
        let root = |path: &PathBuf, source| DataRoot {
            path: path.clone(),
            source,
        };

        assert_eq!(
            resolve_data_root(
                Some(env.clone().into_os_string()),
                Some(test.clone()),
                Some(&configured_text),
                platform.clone()
            ),
            root(&env, DataRootSource::Env)
        );
        assert_eq!(
            data_root_from(None, Some(test.clone()), never_read, platform.clone()).unwrap(),
            root(&test, DataRootSource::TestData)
        );
        assert_eq!(
            resolve_data_root(
                None,
                Some(PathBuf::from("relative/data")),
                Some(&configured_text),
                platform.clone()
            ),
            root(&configured, DataRootSource::Configured)
        );
        assert_eq!(
            resolve_data_root(None, None, None, platform).source,
            DataRootSource::Platform
        );
        assert!(DataRootSource::Env.chosen_by_user());
        assert!(DataRootSource::Configured.chosen_by_user());
        assert!(!DataRootSource::TestData.chosen_by_user());
        assert!(!DataRootSource::Platform.chosen_by_user());
    }

    #[cfg(feature = "mast")]
    #[test]
    fn the_test_data_folder_follows_the_canned_mast_settings() {
        use crate::infra::mast::settings::{ENV_MAST_URL, ENV_TEST_DATA_DIR};
        let scratch = absolute("ab-cdp-canned");
        let text = scratch.to_string_lossy().into_owned();
        let canned = MastSettings::from_lookup(|name| match name {
            ENV_MAST_URL => Some("http://127.0.0.1:43123".to_owned()),
            ENV_TEST_DATA_DIR => Some(text.clone()),
            _ => None,
        });
        assert_eq!(test_data_dir_in(&canned), Some(scratch.join("data")));
        assert_eq!(test_data_dir_in(&MastSettings::default()), None);
    }

    #[test]
    fn the_default_data_root_is_the_non_roaming_local_folder_on_windows() {
        #[cfg(windows)]
        assert_eq!(platform_data_dir(), dirs::data_local_dir());
        #[cfg(not(windows))]
        assert_eq!(platform_data_dir(), dirs::data_dir());
    }

    #[test]
    fn cache_root_is_the_folder_that_holds_the_catalog_cache() {
        let root = cache_root();
        assert!(root.is_absolute(), "{}", root.display());
        if let Some(cache) = dirs::cache_dir() {
            assert_eq!(root, cache.join("astroburst"));
        }
        #[cfg(windows)]
        assert_eq!(root, root_of(None, None, platform_data_dir()));
    }

    #[cfg(feature = "mast")]
    #[test]
    fn mast_root_switches_to_mast_test_under_the_url_override() {
        let root = absolute("ab-data-root");
        assert_eq!(
            mast_root_in(&root, &MastSettings::default()),
            root.join("mast")
        );
        let canned = MastSettings::from_lookup(|name| {
            (name == crate::infra::mast::settings::ENV_MAST_URL)
                .then(|| "http://127.0.0.1:43123".to_owned())
        });
        assert_eq!(mast_root_in(&root, &canned), root.join("mast-test"));
        let refused = MastSettings::from_lookup(|name| {
            (name == crate::infra::mast::settings::ENV_MAST_URL)
                .then(|| "https://mirror.example.org".to_owned())
        });
        assert_eq!(mast_root_in(&root, &refused), root.join("mast"));
    }

    #[cfg(feature = "mast")]
    #[test]
    fn the_mast_quota_defaults_to_twenty_gigabytes_and_ignores_invalid_values() {
        assert_eq!(mast_quota_bytes(None), 20_000_000_000);
        assert_eq!(mast_quota_bytes(Some(0.5)), 500_000_000);
        assert_eq!(mast_quota_bytes(Some(0.001)), 1_000_000);
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(mast_quota_bytes(Some(invalid)), 20_000_000_000, "{invalid}");
        }
    }

    #[test]
    fn free_space_is_measured_on_the_nearest_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        let here = free_space(dir.path()).unwrap();
        assert!(
            here > 0,
            "no free space reported for {}",
            dir.path().display()
        );
        assert!(free_space(&dir.path().join("mast").join("JWST").join("2739")).unwrap() > 0);
        let file = dir.path().join("x.fits");
        std::fs::write(&file, b"SIMPLE").unwrap();
        assert!(free_space(&file).unwrap() > 0);
        let err = free_space(Path::new("ab-no-such-folder").join("deeper").as_path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
