use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::Manager;

pub const SAMPLE_DATA_DIR: &str = "sample-data";
pub const SAMPLE_FILES: [&str; 3] = ["502nmos.fits", "656nmos.fits", "673nmos.fits"];
pub const SAMPLE_CREDIT: &str =
    "HST/WFPC2 narrowband frames of the Eagle Nebula (M16): NASA, ESA, J. Hester and P. Scowen (Arizona State University), data via STScI";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SampleDataPaths {
    pub paths: Vec<String>,
    pub credit: String,
}

pub fn sample_data_roots(resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = resource_dir {
        roots.push(dir.join(crate::infra::catalog_store::BUNDLED_RESOURCES_DIR).join(SAMPLE_DATA_DIR));
        roots.push(dir.join(SAMPLE_DATA_DIR));
    }
    if cfg!(debug_assertions) {
        roots.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("exampleFits").join(SAMPLE_DATA_DIR));
    }
    roots
}

pub fn sample_data_paths(roots: &[PathBuf]) -> Option<Vec<PathBuf>> {
    roots.iter().find_map(|root| {
        let paths: Vec<PathBuf> = SAMPLE_FILES.iter().map(|name| root.join(name)).collect();
        paths.iter().all(|path| path.is_file()).then_some(paths)
    })
}

#[tauri::command]
pub async fn sample_data_paths_cmd(app: tauri::AppHandle) -> Result<SampleDataPaths, String> {
    let resource_dir = app.path().resource_dir().ok();
    let roots = sample_data_roots(resource_dir.as_deref());
    let paths = sample_data_paths(&roots).ok_or_else(|| "The sample data is not installed with this build".to_string())?;
    Ok(SampleDataPaths {
        paths: paths.iter().map(|path| path.to_string_lossy().into_owned()).collect(),
        credit: SAMPLE_CREDIT.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, names: &[&str]) {
        for name in names {
            std::fs::write(dir.join(name), b"SIMPLE  =                    T").unwrap();
        }
    }

    #[test]
    fn the_three_frames_are_found_in_the_first_root_that_holds_them_all() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write(first.path(), &SAMPLE_FILES[..2]);
        write(second.path(), &SAMPLE_FILES);
        let found = sample_data_paths(&[first.path().to_path_buf(), second.path().to_path_buf()]).unwrap();
        assert_eq!(found, SAMPLE_FILES.iter().map(|name| second.path().join(name)).collect::<Vec<_>>());
    }

    #[test]
    fn a_missing_frame_means_no_sample_data() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &SAMPLE_FILES[..2]);
        assert_eq!(sample_data_paths(&[dir.path().to_path_buf()]), None);
        assert_eq!(sample_data_paths(&[]), None);
    }

    #[test]
    fn the_bundled_resources_folder_is_probed_before_the_plain_one() {
        let resources = Path::new("C:/app");
        let roots = sample_data_roots(Some(resources));
        assert_eq!(roots[0], resources.join("resources").join("sample-data"));
        assert_eq!(roots[1], resources.join("sample-data"));
        assert!(roots.iter().all(|root| root.ends_with("sample-data")));
    }

    #[test]
    fn the_repository_samples_are_the_published_set() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("exampleFits").join(SAMPLE_DATA_DIR);
        let found = sample_data_paths(&[repo]).expect("exampleFits/sample-data holds the three frames");
        for path in found {
            let len = std::fs::metadata(&path).unwrap().len();
            assert_eq!(len, 10_298_880, "{}", path.display());
        }
    }
}
