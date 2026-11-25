use std::path::{Path, PathBuf};

const APP_ORIGINS: [&str; 3] = ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];

pub fn asset_path_allowed(
    requested: &Path,
    roots: &[PathBuf],
    config_dir: Option<&Path>,
    granted: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(requested).ok().filter(|path| path.is_file())?;
    if crate::infra::config::is_api_key_file(&canonical.to_string_lossy()) {
        return None;
    }
    let settings = config_dir.and_then(|dir| std::fs::canonicalize(dir).ok());
    if settings.is_some_and(|dir| canonical.starts_with(dir)) {
        return None;
    }
    let inside_root = roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .any(|root| canonical.starts_with(root));
    (inside_root || granted(&canonical)).then_some(canonical)
}

pub fn allowed_origin(origin: Option<&str>, dev_origin: Option<&str>) -> Option<String> {
    let origin = origin?.trim_end_matches('/');
    let known = APP_ORIGINS.contains(&origin) || dev_origin.is_some_and(|dev| dev.trim_end_matches('/') == origin);
    known.then(|| origin.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{allowed_origin, asset_path_allowed};
    use std::path::{Path, PathBuf};

    #[test]
    fn only_the_app_s_own_origins_get_a_cors_answer() {
        for own in ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"] {
            assert_eq!(allowed_origin(Some(own), None).as_deref(), Some(own));
        }
        assert_eq!(allowed_origin(Some("http://localhost:1420"), Some("http://localhost:1420/")).as_deref(), Some("http://localhost:1420"));
        assert_eq!(allowed_origin(Some("http://localhost:1420"), None), None);
        assert_eq!(allowed_origin(Some("https://evil.example"), Some("http://localhost:1420")), None);
        assert_eq!(allowed_origin(None, None), None);
    }

    fn write(path: &Path) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
        path.to_path_buf()
    }

    fn never(_: &Path) -> bool {
        false
    }

    #[test]
    fn only_files_inside_the_app_folders_are_served() {
        let app = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let roots = vec![app.path().to_path_buf()];
        let preview = write(&app.path().join("output").join("m42.png"));
        let outside = write(&other.path().join("notes.png"));

        let served = asset_path_allowed(&preview, &roots, None, never).expect("an output preview was refused");
        assert_eq!(served, std::fs::canonicalize(&preview).unwrap());
        assert_eq!(asset_path_allowed(&outside, &roots, None, never), None, "a file outside the app folders was served");
        let escaped = app.path().join("output").join("..").join("..").join(other.path().file_name().unwrap()).join("notes.png");
        assert_eq!(asset_path_allowed(&escaped, &roots, None, never), None, "a .. path left the app folder");
        assert_eq!(asset_path_allowed(&app.path().join("output").join("missing.png"), &roots, None, never), None);
        let granted = |path: &Path| path == std::fs::canonicalize(&outside).unwrap();
        assert!(asset_path_allowed(&outside, &roots, None, granted).is_some(), "a file the user opened was refused");
    }

    #[test]
    fn the_settings_folder_and_key_files_are_never_served() {
        let app = tempfile::tempdir().unwrap();
        let roots = vec![app.path().to_path_buf()];
        let config_dir = app.path().join("astroburst");
        let settings = write(&config_dir.join("config.json"));
        let key = write(&app.path().join("output").join("mast_api_key.txt"));
        let always = |_: &Path| true;
        assert_eq!(asset_path_allowed(&settings, &roots, Some(&config_dir), always), None, "the settings file was served");
        assert_eq!(asset_path_allowed(&key, &roots, Some(&config_dir), always), None, "a key file was served");
    }

    #[cfg(windows)]
    #[test]
    fn windows_aliases_of_a_key_file_are_refused() {
        let app = tempfile::tempdir().unwrap();
        let roots = vec![app.path().to_path_buf()];
        let key = write(&app.path().join("mast_api_key.txt"));
        let text = key.to_string_lossy().into_owned();
        for alias in [format!("{text}."), format!("{text} "), format!("{text}::$DATA"), format!("{text}. .")] {
            assert_eq!(asset_path_allowed(Path::new(&alias), &roots, None, |_: &Path| true), None, "{alias:?} was served");
        }
    }
}
