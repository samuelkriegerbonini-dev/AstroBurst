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

pub fn data_root() -> PathBuf {
    try_data_root().unwrap_or_else(|error| {
        let fallback = resolve_data_root(std::env::var_os(ENV_DATA_DIR), None, platform_data_dir());
        log::warn!(
            "the Data folder setting could not be read ({error:#}); using {}",
            fallback.display()
        );
        fallback
    })
}

pub fn try_data_root() -> anyhow::Result<PathBuf> {
    data_root_from(
        std::env::var_os(ENV_DATA_DIR),
        configured_data_dir,
        platform_data_dir(),
    )
}

fn configured_data_dir() -> anyhow::Result<Option<String>> {
    Ok(crate::infra::config::load_config()?.data_dir_override)
}

fn data_root_from(
    env: Option<OsString>,
    configured: impl FnOnce() -> anyhow::Result<Option<String>>,
    platform: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    if env
        .as_deref()
        .is_some_and(|dir| Path::new(dir).is_absolute())
    {
        return Ok(resolve_data_root(env, None, platform));
    }
    let configured = configured()?;
    Ok(resolve_data_root(env, configured.as_deref(), platform))
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
    configured: Option<&str>,
    platform: Option<PathBuf>,
) -> PathBuf {
    let env = env.map(PathBuf::from);
    let configured = configured.map(|text| PathBuf::from(text.trim()));
    env.into_iter()
        .chain(configured)
        .find(|dir| dir.is_absolute())
        .unwrap_or_else(|| app_dir_under(platform))
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

    #[test]
    fn the_env_folder_wins_over_the_configured_one_and_the_platform_default() {
        let env = absolute("ab-env-root");
        let configured = absolute("ab-configured-root");
        let configured_text = configured.to_string_lossy().into_owned();
        let platform = Some(absolute("ab-local-app-data"));
        let default_root = absolute("ab-local-app-data").join("astroburst");

        assert_eq!(
            resolve_data_root(
                Some(env.clone().into_os_string()),
                Some(&configured_text),
                platform.clone()
            ),
            env
        );
        assert_eq!(
            resolve_data_root(None, Some(&configured_text), platform.clone()),
            configured
        );
        assert_eq!(
            resolve_data_root(
                None,
                Some(&format!("  {configured_text} ")),
                platform.clone()
            ),
            configured
        );
        assert_eq!(
            resolve_data_root(None, None, platform.clone()),
            default_root
        );

        assert_eq!(
            resolve_data_root(
                Some(OsString::from("relative/env")),
                Some(&configured_text),
                platform.clone()
            ),
            configured
        );
        assert_eq!(
            resolve_data_root(Some(OsString::new()), None, platform.clone()),
            default_root
        );
        assert_eq!(
            resolve_data_root(None, Some("relative/config"), platform.clone()),
            default_root
        );
        assert_eq!(resolve_data_root(None, Some(""), platform), default_root);

        let fallback = resolve_data_root(None, None, None);
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
                never_read,
                platform.clone()
            )
            .unwrap(),
            env
        );
        let err = data_root_from(None, unreadable, platform.clone()).unwrap_err();
        assert!(format!("{err:#}").contains("locked"), "{err:#}");
        assert!(data_root_from(
            Some(OsString::from("relative/env")),
            unreadable,
            platform.clone()
        )
        .is_err());
        let configured_text = configured.to_string_lossy().into_owned();
        assert_eq!(
            data_root_from(None, || Ok(Some(configured_text)), platform.clone()).unwrap(),
            configured
        );
        assert_eq!(
            data_root_from(None, || Ok(None), platform).unwrap(),
            absolute("ab-local-app-data").join("astroburst")
        );
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
        assert_eq!(root, resolve_data_root(None, None, platform_data_dir()));
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
