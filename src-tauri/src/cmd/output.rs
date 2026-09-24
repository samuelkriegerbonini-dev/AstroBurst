use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde_json::json;

use crate::cmd::common::{blocking_cmd, source_path};
use crate::infra::config::{config_path, load_config_from};
use crate::types::constants::{
    DEFAULT_OUTPUT_MAX_BYTES, RES_CLEANED_BYTES, RES_CLEANED_FILES, RES_CLEANED_PATHS,
    RES_ELAPSED_MS, RES_FILE_COUNT, RES_MAX_SIZE, RES_OUTPUT_DIR, RES_TOTAL_SIZE,
};

const MAX_WALK_DEPTH: usize = 8;

static SWEEP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn output_cap_bytes(configured_mb: Option<u64>) -> u64 {
    configured_mb
        .map(|mb| mb.saturating_mul(1024 * 1024))
        .unwrap_or(DEFAULT_OUTPUT_MAX_BYTES)
}

fn max_bytes_from_config(config_file: &Path) -> u64 {
    output_cap_bytes(load_config_from(config_file).ok().and_then(|c| c.output_max_size_mb))
}

fn get_max_bytes() -> u64 {
    config_path().map_or(DEFAULT_OUTPUT_MAX_BYTES, |file| max_bytes_from_config(&file))
}

struct FileEntry {
    path: PathBuf,
    size: u64,
    mtime: SystemTime,
}

fn walk_dir_entries(dir: &Path) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    if !dir.exists() {
        return Ok(entries);
    }
    let mut pending: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((current, depth)) = pending.pop() {
        let listing = match fs::read_dir(&current) {
            Ok(listing) => listing,
            Err(e) if depth == 0 => return Err(e).context("Failed to read output directory"),
            Err(_) => continue,
        };
        for entry in listing.flatten() {
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_dir() {
                if depth < MAX_WALK_DEPTH {
                    pending.push((entry.path(), depth + 1));
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            entries.push(FileEntry {
                path: entry.path(),
                size: meta.len(),
                mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    Ok(entries)
}

fn normalized_key(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

struct KeepSet {
    files: HashSet<String>,
    dirs: Vec<String>,
}

impl KeepSet {
    fn new(keep: &[&str]) -> Self {
        let mut files = HashSet::new();
        let mut dirs = Vec::new();
        for entry in keep.iter().filter(|p| !p.trim().is_empty()) {
            let file = source_path(entry);
            let path = Path::new(&file);
            let key = normalized_key(path);
            if path.is_dir() {
                dirs.push(format!("{}{}", key.trim_end_matches(['/', '\\']), std::path::MAIN_SEPARATOR));
            } else {
                files.insert(key);
            }
        }
        Self { files, dirs }
    }

    fn protects(&self, path: &Path) -> bool {
        let key = normalized_key(path);
        self.files.contains(&key) || self.dirs.iter().any(|dir| key.starts_with(dir.as_str()))
    }
}

pub(crate) fn enforce_output_lru_keeping(
    dir: &Path,
    max_bytes: u64,
    keep: &[&str],
) -> Result<(usize, u64, Vec<String>)> {
    let _guard = SWEEP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let protected = KeepSet::new(keep);
    let mut entries = walk_dir_entries(dir)?;
    let total: u64 = entries.iter().map(|e| e.size).sum();

    if total <= max_bytes {
        return Ok((0, 0, Vec::new()));
    }

    entries.retain(|e| !protected.protects(&e.path));
    entries.sort_by(|a, b| a.mtime.cmp(&b.mtime));

    let removable: u64 = entries.iter().map(|e| e.size).sum();
    let floor = total - removable;
    if floor >= max_bytes {
        log::warn!(
            "LRU cleanup: {} bytes are kept for files still in use, above the {} byte cap; nothing removed",
            floor,
            max_bytes
        );
        return Ok((0, 0, Vec::new()));
    }

    let mut current = total;
    let mut removed_count = 0usize;
    let mut removed_bytes = 0u64;
    let mut removed_paths: Vec<String> = Vec::new();

    for entry in &entries {
        if current <= max_bytes {
            break;
        }
        match fs::remove_file(&entry.path) {
            Ok(()) => {
                current = current.saturating_sub(entry.size);
                removed_count += 1;
                removed_bytes += entry.size;
                removed_paths.push(entry.path.to_string_lossy().to_string());
                log::info!(
                    "LRU cleanup: removed {} ({} bytes)",
                    entry.path.display(),
                    entry.size
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                current = current.saturating_sub(entry.size);
            }
            Err(e) => {
                log::warn!("LRU cleanup: failed to remove {}: {}", entry.path.display(), e);
            }
        }
    }

    log::info!(
        "LRU cleanup complete: removed {} files, freed {} bytes, remaining {} bytes",
        removed_count,
        removed_bytes,
        current
    );

    Ok((removed_count, removed_bytes, removed_paths))
}

fn dir_info(dir: &Path) -> Result<(u64, usize)> {
    let entries = walk_dir_entries(dir)?;
    let total_size: u64 = entries.iter().map(|e| e.size).sum();
    Ok((total_size, entries.len()))
}

#[tauri::command]
pub async fn get_output_dir_info(output_dir: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let path = Path::new(&output_dir);
        let (total_size, file_count) = dir_info(path)?;
        Ok(json!({
            RES_OUTPUT_DIR: output_dir,
            RES_TOTAL_SIZE: total_size,
            RES_MAX_SIZE: get_max_bytes(),
            RES_FILE_COUNT: file_count,
        }))
    })
}

#[tauri::command]
pub async fn cleanup_output_cmd(
    output_dir: String,
    max_size_mb: Option<u64>,
    keep: Option<Vec<String>>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = std::time::Instant::now();
        let path = Path::new(&output_dir);
        let max_bytes = max_size_mb
            .map(|mb| output_cap_bytes(Some(mb)))
            .unwrap_or_else(get_max_bytes);
        let keep = keep.unwrap_or_default();
        let keep_refs: Vec<&str> = keep.iter().map(String::as_str).collect();

        let (cleaned_files, cleaned_bytes, cleaned_paths) = enforce_output_lru_keeping(path, max_bytes, &keep_refs)?;
        let (total_size, file_count) = dir_info(path)?;

        Ok(json!({
            RES_CLEANED_FILES: cleaned_files,
            RES_CLEANED_BYTES: cleaned_bytes,
            RES_CLEANED_PATHS: cleaned_paths,
            RES_TOTAL_SIZE: total_size,
            RES_FILE_COUNT: file_count,
            RES_OUTPUT_DIR: output_dir,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn cancel_progress_cmd(event: String) -> Result<bool, String> {
    Ok(crate::infra::progress::cancel_event(&event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    fn write_file(dir: &Path, name: &str, bytes: usize) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(&vec![b'x'; bytes]).unwrap();
        f.flush().unwrap();
        path
    }

    fn age(path: &Path, seconds_ago: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago)).unwrap();
    }

    #[test]
    fn lru_never_removes_a_protected_file() {
        let dir = tempfile::tempdir().unwrap();
        let oldest = write_file(dir.path(), "slot_input.fits", 4096);
        let middle = write_file(dir.path(), "stale_result.fits", 4096);
        let newest = write_file(dir.path(), "fresh_result.fits", 4096);
        age(&oldest, 30);
        age(&middle, 20);
        age(&newest, 10);

        let keep = [oldest.to_str().unwrap()];
        let (removed, _, paths) = enforce_output_lru_keeping(dir.path(), 8192, &keep).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("stale_result.fits"), "{paths:?}");

        assert_eq!(removed, 1);
        assert!(oldest.exists(), "protected input was deleted");
        assert!(!middle.exists(), "oldest unprotected file should go first");
        assert!(newest.exists());
    }

    #[test]
    fn lru_removes_nothing_when_the_protected_files_alone_exceed_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let master_a = write_file(dir.path(), "master_a.fits", 6000);
        let master_b = write_file(dir.path(), "master_b.fits", 6000);
        let old_result = write_file(dir.path(), "old_result.fits", 1000);
        age(&master_a, 30);
        age(&master_b, 30);
        age(&old_result, 10);

        let keep = [master_a.to_str().unwrap(), master_b.to_str().unwrap()];
        let (removed, bytes, paths) = enforce_output_lru_keeping(dir.path(), 8192, &keep).unwrap();

        assert_eq!((removed, bytes, paths.len()), (0, 0, 0));
        assert!(
            old_result.exists(),
            "scorched an unprotected file for a target the keep-set makes unreachable"
        );
        assert!(master_a.exists() && master_b.exists());
    }

    #[test]
    fn lru_is_a_no_op_below_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let only = write_file(dir.path(), "result.fits", 100);
        let (removed, bytes, paths) = enforce_output_lru_keeping(dir.path(), 8192, &[]).unwrap();
        assert_eq!((removed, bytes, paths.len()), (0, 0, 0));
        assert!(only.exists());
    }

    #[test]
    fn tiles_in_subdirectories_are_counted_and_evicted() {
        let dir = tempfile::tempdir().unwrap();
        let tile = write_file(dir.path(), "tiles/slot0/0/0_0.png", 4096);
        let result = write_file(dir.path(), "result.fits", 4096);
        age(&tile, 30);
        age(&result, 10);

        assert_eq!(dir_info(dir.path()).unwrap(), (8192, 2), "tile bytes are missing from the storage report");
        let (removed, bytes, paths) = enforce_output_lru_keeping(dir.path(), 4096, &[]).unwrap();
        assert_eq!((removed, bytes), (1, 4096));
        assert!(paths[0].ends_with("0_0.png"), "{paths:?}");
        assert!(!tile.exists());
        assert!(result.exists());
    }

    #[test]
    fn a_kept_directory_protects_everything_under_it() {
        let dir = tempfile::tempdir().unwrap();
        let active_tile = write_file(dir.path(), "tiles/slot0/0/0_0.png", 4096);
        let stale_tile = write_file(dir.path(), "tiles/slot1/0/0_0.png", 4096);
        let result = write_file(dir.path(), "result.fits", 4096);
        age(&active_tile, 40);
        age(&stale_tile, 30);
        age(&result, 10);

        let slot0 = dir.path().join("tiles").join("slot0");
        let keep = [slot0.to_str().unwrap()];
        let (removed, _, _) = enforce_output_lru_keeping(dir.path(), 8192, &keep).unwrap();
        assert_eq!(removed, 1);
        assert!(active_tile.exists(), "a tile of the pyramid on screen was deleted");
        assert!(!stale_tile.exists());
        assert!(result.exists());
    }

    #[tokio::test]
    async fn manual_cleanup_never_deletes_paths_the_frontend_still_references() {
        let dir = tempfile::tempdir().unwrap();
        let chunk = 600 * 1024;
        let ingest_png = write_file(dir.path(), "m42.png", chunk);
        let processed = write_file(dir.path(), "m42_denoised.fits", chunk);
        let stale = write_file(dir.path(), "old_arcsinh.fits", chunk);
        let fresh = write_file(dir.path(), "new_result.fits", chunk);
        age(&ingest_png, 50);
        age(&processed, 40);
        age(&stale, 30);
        age(&fresh, 10);

        let keep = vec![
            ingest_png.to_str().unwrap().to_string(),
            format!("{}#hdu=0", processed.to_str().unwrap()),
        ];
        let res = cleanup_output_cmd(dir.path().to_str().unwrap().to_string(), Some(2), Some(keep))
            .await
            .unwrap();
        assert!(ingest_png.exists(), "the ingest PNG of a loaded file was deleted");
        assert!(processed.exists(), "a processed source the preview still shows was deleted");
        assert!(!stale.exists());
        assert!(fresh.exists());
        assert_eq!(res[RES_CLEANED_FILES], 1);
        let cleaned: Vec<String> = serde_json::from_value(res[RES_CLEANED_PATHS].clone()).unwrap();
        assert_eq!(cleaned.len(), 1);
        assert!(cleaned[0].ends_with("old_arcsinh.fits"), "{cleaned:?}");
    }

    #[test]
    fn the_output_cap_conversion_saturates_instead_of_wrapping() {
        assert_eq!(output_cap_bytes(None), DEFAULT_OUTPUT_MAX_BYTES);
        assert_eq!(output_cap_bytes(Some(1)), 1024 * 1024);
        assert_eq!(output_cap_bytes(Some(1 << 44)), u64::MAX, "a huge cap wrapped to a small one");
        assert_eq!(output_cap_bytes(Some(u64::MAX)), u64::MAX);
    }

    #[test]
    fn a_cap_edited_in_the_config_applies_without_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        fs::write(&config, r#"{"output_max_size_mb": 5}"#).unwrap();
        assert_eq!(max_bytes_from_config(&config), 5 * 1024 * 1024);

        fs::write(&config, r#"{"output_max_size_mb": 7}"#).unwrap();
        assert_eq!(
            max_bytes_from_config(&config),
            7 * 1024 * 1024,
            "the cap read first was kept after the config changed"
        );

        fs::write(&config, r#"{"output_max_size_mb": null}"#).unwrap();
        assert_eq!(max_bytes_from_config(&config), DEFAULT_OUTPUT_MAX_BYTES);
    }
}
