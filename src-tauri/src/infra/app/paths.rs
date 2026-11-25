use std::path::{Component, Path, PathBuf, Prefix};

const MAX_WIN32_PATH: usize = 260;
const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
    "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub fn plain(path: &Path) -> PathBuf {
    match without_verbatim_prefix(path) {
        Some(simplified) if win32_can_open(path) => simplified,
        _ => path.to_path_buf(),
    }
}

fn without_verbatim_prefix(path: &Path) -> Option<PathBuf> {
    let text = path.to_str()?;
    let Some(Component::Prefix(prefix)) = path.components().next() else { return None };
    match prefix.kind() {
        Prefix::VerbatimDisk(_) => Some(PathBuf::from(&text[4..])),
        Prefix::VerbatimUNC(server, share) => {
            let (server, share) = (server.to_str()?, share.to_str()?);
            let rest = &text[prefix.as_os_str().len()..];
            (valid_win32_name(server) && valid_win32_name(share)).then(|| PathBuf::from(format!(r"\\{server}\{share}{rest}")))
        }
        _ => None,
    }
}

fn win32_can_open(path: &Path) -> bool {
    path.as_os_str().len() <= MAX_WIN32_PATH
        && path.components().skip(1).all(|component| match component {
            Component::RootDir => true,
            Component::Normal(name) => name.to_str().is_some_and(valid_win32_name),
            _ => false,
        })
}

fn valid_win32_name(name: &str) -> bool {
    let device = name.split('.').next().unwrap_or(name).trim_end();
    !name.is_empty()
        && !name.ends_with(' ')
        && !name.ends_with('.')
        && !name.bytes().any(|byte| byte < b' ' || matches!(byte, b'<' | b'>' | b':' | b'"' | b'/' | b'\\' | b'|' | b'?' | b'*'))
        && !RESERVED_DEVICE_NAMES.iter().any(|reserved| device.eq_ignore_ascii_case(reserved))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_without_a_verbatim_disk_prefix_pass_through() {
        for given in ["relative/x.fits", "../d1/x.fits", "C:\\astrokit\\x.fits", "/home/user/x.fits", r"\\server\share\x.fits", ""] {
            assert_eq!(plain(Path::new(given)), PathBuf::from(given), "{given}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_verbatim_disk_and_unc_prefixes_are_stripped_only_when_win32_can_open_the_result() {
        let cases = [
            (r"\\?\C:\astrokit\d1\x.fits", r"C:\astrokit\d1\x.fits"),
            (r"\\?\C:\astrokit\d1", r"C:\astrokit\d1"),
            (r"\\?\C:\", r"C:\"),
            (r"\\?\C:\astrokit\con\x.fits", r"\\?\C:\astrokit\con\x.fits"),
            (r"\\?\C:\astrokit\LPT1.txt", r"\\?\C:\astrokit\LPT1.txt"),
            (r"\\?\C:\astrokit\trailing.\x.fits", r"\\?\C:\astrokit\trailing.\x.fits"),
            (r"\\?\C:\astrokit\trailing \x.fits", r"\\?\C:\astrokit\trailing \x.fits"),
            (r"\\?\C:\astrokit\console.log", r"C:\astrokit\console.log"),
            (r"\\?\UNC\server\share\x.fits", r"\\server\share\x.fits"),
            (r"\\?\UNC\nas.example.org\data$\d1\x.fits", r"\\nas.example.org\data$\d1\x.fits"),
            (r"\\?\UNC\server\share", r"\\server\share"),
            (r"\\?\UNC\server\share\con\x.fits", r"\\?\UNC\server\share\con\x.fits"),
            (r"\\?\UNC\server\share.\x.fits", r"\\?\UNC\server\share.\x.fits"),
            (r"\\?\UNC\server\aux\x.fits", r"\\?\UNC\server\aux\x.fits"),
            (r"\\.\pipe\astroburst", r"\\.\pipe\astroburst"),
            (r"\\?\Volume{0b1e5f4a-0000-0000-0000-100000000000}\x.fits", r"\\?\Volume{0b1e5f4a-0000-0000-0000-100000000000}\x.fits"),
        ];
        for (given, expected) in cases {
            assert_eq!(plain(Path::new(given)), PathBuf::from(expected), "{given}");
        }
        let long = format!(r"\\?\C:\{}\x.fits", "a".repeat(MAX_WIN32_PATH));
        assert_eq!(plain(Path::new(&long)), PathBuf::from(&long));
        let long_unc = format!(r"\\?\UNC\server\share\{}\x.fits", "b".repeat(MAX_WIN32_PATH - 20));
        assert_eq!(plain(Path::new(&long_unc)), PathBuf::from(&long_unc));
        let canonical = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert!(canonical.to_str().unwrap().starts_with(r"\\?\"), "{}", canonical.display());
        let plain_dir = plain(&canonical);
        assert!(!plain_dir.to_str().unwrap().starts_with(r"\\?\"), "{}", plain_dir.display());
        assert_eq!(std::fs::canonicalize(&plain_dir).unwrap(), canonical);
    }
}
