use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde_json::json;

use crate::cmd::common::{blocking_cmd, source_path};
use crate::cmd::ops::RunGuard;
use crate::core::history;
use crate::infra::config::{config_path, load_config_from};
use crate::types::constants::{
    DEFAULT_OUTPUT_MAX_BYTES, RES_CLEANED_BYTES, RES_CLEANED_FILES, RES_CLEANED_PATHS,
    RES_ELAPSED_MS, RES_FILE_COUNT, RES_HISTORY_ROOT, RES_MAX_SIZE, RES_OUTPUT_DIR, RES_REPLAYABLE_NODES,
    RES_TOTAL_SIZE,
};

const MAX_WALK_DEPTH: usize = 8;
const OUTPUT_DIR_NAME: &str = "output";

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

fn dir_prefix(path: &Path) -> String {
    format!("{}{}", normalized_key(path).trim_end_matches(['/', '\\']), std::path::MAIN_SEPARATOR)
}

struct KeepSet {
    files: HashSet<String>,
    dirs: Vec<String>,
    removable_under: Option<Vec<String>>,
}

impl KeepSet {
    fn new(keep: &[&str], removable_under: Option<&[&Path]>) -> Self {
        let mut files = HashSet::new();
        let mut dirs = Vec::new();
        for entry in keep.iter().filter(|p| !p.trim().is_empty()) {
            let file = source_path(entry);
            let path = Path::new(&file);
            if path.is_dir() {
                dirs.push(dir_prefix(path));
            } else {
                files.insert(normalized_key(path));
            }
        }
        let removable_under = removable_under.map(|roots| roots.iter().map(|root| dir_prefix(root)).collect());
        Self { files, dirs, removable_under }
    }

    fn protects(&self, path: &Path) -> bool {
        let key = normalized_key(path);
        let outside = |roots: &Vec<String>| !roots.iter().any(|root| key.starts_with(root.as_str()));
        self.files.contains(&key)
            || self.dirs.iter().any(|dir| key.starts_with(dir.as_str()))
            || self.removable_under.as_ref().is_some_and(outside)
    }
}

pub(crate) fn enforce_output_lru_keeping(
    dir: &Path,
    max_bytes: u64,
    keep: &[&str],
    best_effort: bool,
) -> Result<(usize, u64, Vec<String>)> {
    enforce_lru(dir, max_bytes, keep, best_effort, None)
}

fn enforce_lru(
    dir: &Path,
    max_bytes: u64,
    keep: &[&str],
    best_effort: bool,
    removable_under: Option<&[&Path]>,
) -> Result<(usize, u64, Vec<String>)> {
    let _guard = SWEEP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let protected = KeepSet::new(keep, removable_under);
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
        if !best_effort {
            log::warn!(
                "LRU cleanup: {} bytes are kept for files still in use, above the {} byte cap; nothing removed",
                floor,
                max_bytes
            );
            return Ok((0, 0, Vec::new()));
        }
        log::warn!(
            "LRU cleanup: {} bytes are kept for files still in use, above the {} byte cap; removing every unprotected file",
            floor,
            max_bytes
        );
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

pub(crate) fn dir_info(dir: &Path) -> Result<(u64, usize)> {
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

        let (cleaned_files, cleaned_bytes, cleaned_paths) =
            enforce_output_lru_keeping(path, max_bytes, &keep_refs, true)?;
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

pub(crate) fn clear_output_dir(data_dir: &Path) {
    let output_dir = data_dir.join(OUTPUT_DIR_NAME);
    if let Err(e) = fs::remove_dir_all(&output_dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log::warn!("launch: could not clear {}: {e}", output_dir.display());
        }
    }
    if let Err(e) = fs::create_dir_all(&output_dir) {
        log::warn!("launch: could not create {}: {e}", output_dir.display());
    }
}

#[derive(Default)]
struct HistorySweep {
    removed_files: usize,
    removed_bytes: u64,
    removed_paths: Vec<String>,
    replayable_nodes: usize,
}

impl HistorySweep {
    fn add(&mut self, (files, bytes, paths): (usize, u64, Vec<String>)) {
        self.removed_files += files;
        self.removed_bytes += bytes;
        self.removed_paths.extend(paths);
    }
}

struct TrackedHistory {
    dir: PathBuf,
    doc: history::HistoryDoc,
    last_used: SystemTime,
}

#[derive(Default)]
struct HistoryScan {
    unreadable: Vec<String>,
    tracked: Vec<TrackedHistory>,
    held: Vec<(PathBuf, RunGuard)>,
}

fn is_history_root(root: &Path) -> bool {
    root.is_absolute() && root.file_name().is_some_and(|name| name == history::HISTORY_DIR_NAME)
}

fn scan_history_root(root: &Path) -> HistoryScan {
    let mut scan = HistoryScan::default();
    let Ok(listing) = fs::read_dir(root) else {
        return scan;
    };
    for entry in listing.flatten() {
        let dir = entry.path();
        let is_image_dir = entry.file_type().is_ok_and(|kind| kind.is_dir())
            && history::id::is_hex_id(&entry.file_name().to_string_lossy());
        if !is_image_dir {
            continue;
        }
        let Ok(guard) = RunGuard::for_sweep(&dir) else {
            log::info!("history sweep: {} is kept whole while a step runs on it", dir.display());
            continue;
        };
        match history::load(&dir) {
            Ok(Some(doc)) => {
                let last_used = history::last_used(&dir).unwrap_or(SystemTime::UNIX_EPOCH);
                scan.tracked.push(TrackedHistory { dir: dir.clone(), doc, last_used });
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("history sweep: {} is kept whole: {e}", dir.display());
                scan.unreadable.push(dir.to_string_lossy().to_string());
            }
        }
        scan.held.push((dir, guard));
    }
    scan
}

fn sweep_history(root: &Path, max_bytes: u64) -> Result<HistorySweep> {
    sweep_scanned(root, max_bytes, scan_history_root(root))
}

fn sweep_scanned(root: &Path, max_bytes: u64, scan: HistoryScan) -> Result<HistorySweep> {
    let usage: Vec<(&history::HistoryDoc, SystemTime)> = scan.tracked.iter().map(|t| (&t.doc, t.last_used)).collect();
    let levels = history::pin_levels(&usage, max_bytes);
    let mut pinned = scan.unreadable.clone();
    let mut on_head_paths = Vec::new();
    for (tracked, level) in scan.tracked.iter().zip(levels) {
        pinned.extend(history::protected_paths(&tracked.doc, &tracked.dir, level).iter().map(|p| p.to_string_lossy().to_string()));
        on_head_paths.extend(history::active_paths(&tracked.doc).iter().map(|file| tracked.dir.join(file).to_string_lossy().to_string()));
    }
    let off_path_first: Vec<&str> = pinned.iter().chain(&on_head_paths).map(String::as_str).collect();
    let pinned_only: Vec<&str> = pinned.iter().map(String::as_str).collect();
    let held: Vec<&Path> = scan.held.iter().map(|(dir, _)| dir.as_path()).collect();

    let mut sweep = HistorySweep::default();
    sweep.add(enforce_lru(root, max_bytes, &off_path_first, true, Some(&held))?);
    sweep.add(enforce_lru(root, max_bytes, &pinned_only, true, Some(&held))?);
    for tracked in &scan.tracked {
        match history::mark_evicted(&tracked.dir) {
            Ok(marked) => sweep.replayable_nodes += marked.len(),
            Err(e) => log::warn!("history sweep: could not record evicted nodes in {}: {e}", tracked.dir.display()),
        }
    }
    Ok(sweep)
}

#[tauri::command]
pub async fn cleanup_history_cmd(history_root: String, max_size_mb: Option<u64>) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = std::time::Instant::now();
        let root = Path::new(&history_root);
        if !is_history_root(root) {
            anyhow::bail!(
                "refusing to sweep {history_root}: the history sweep only runs on an absolute folder named \"{}\"",
                history::HISTORY_DIR_NAME
            );
        }
        let max_bytes = max_size_mb.map(|mb| output_cap_bytes(Some(mb))).unwrap_or_else(get_max_bytes);
        let sweep = sweep_history(root, max_bytes)?;
        let (total_size, file_count) = dir_info(root)?;
        Ok(json!({
            RES_CLEANED_FILES: sweep.removed_files,
            RES_CLEANED_BYTES: sweep.removed_bytes,
            RES_CLEANED_PATHS: sweep.removed_paths,
            RES_REPLAYABLE_NODES: sweep.replayable_nodes,
            RES_TOTAL_SIZE: total_size,
            RES_FILE_COUNT: file_count,
            RES_HISTORY_ROOT: history_root,
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
        let (removed, _, paths) = enforce_output_lru_keeping(dir.path(), 8192, &keep, false).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("stale_result.fits"), "{paths:?}");

        assert_eq!(removed, 1);
        assert!(oldest.exists(), "protected input was deleted");
        assert!(!middle.exists(), "oldest unprotected file should go first");
        assert!(newest.exists());
    }

    #[test]
    fn a_strict_sweep_removes_nothing_when_the_protected_files_alone_exceed_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let master_a = write_file(dir.path(), "master_a.fits", 6000);
        let master_b = write_file(dir.path(), "master_b.fits", 6000);
        let old_result = write_file(dir.path(), "old_result.fits", 1000);
        age(&master_a, 30);
        age(&master_b, 30);
        age(&old_result, 10);

        let keep = [master_a.to_str().unwrap(), master_b.to_str().unwrap()];
        let (removed, bytes, paths) = enforce_output_lru_keeping(dir.path(), 8192, &keep, false).unwrap();

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
        let (removed, bytes, paths) = enforce_output_lru_keeping(dir.path(), 8192, &[], false).unwrap();
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
        let (removed, bytes, paths) = enforce_output_lru_keeping(dir.path(), 4096, &[], false).unwrap();
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
        let (removed, _, _) = enforce_output_lru_keeping(dir.path(), 8192, &keep, false).unwrap();
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

    #[tokio::test]
    async fn manual_cleanup_removes_unprotected_files_even_when_the_cap_is_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let master_a = write_file(dir.path(), "master_a.fits", 600 * 1024);
        let master_b = write_file(dir.path(), "master_b.fits", 600 * 1024);
        let older = write_file(dir.path(), "old_stretch.png", 100 * 1024);
        let newer = write_file(dir.path(), "old_result.fits", 100 * 1024);
        age(&master_a, 50);
        age(&master_b, 40);
        age(&older, 30);
        age(&newer, 20);

        let keep = vec![
            master_a.to_str().unwrap().to_string(),
            master_b.to_str().unwrap().to_string(),
        ];
        let res = cleanup_output_cmd(dir.path().to_str().unwrap().to_string(), Some(1), Some(keep))
            .await
            .unwrap();

        assert_eq!(res[RES_CLEANED_FILES], 2, "{res}");
        assert_eq!(res[RES_CLEANED_BYTES], 200 * 1024);
        let cleaned: Vec<String> = serde_json::from_value(res[RES_CLEANED_PATHS].clone()).unwrap();
        assert!(cleaned[0].ends_with("old_stretch.png") && cleaned[1].ends_with("old_result.fits"), "{cleaned:?}");
        assert!(!older.exists() && !newer.exists());
        assert!(master_a.exists() && master_b.exists(), "a protected master was deleted");
        assert_eq!(res[RES_TOTAL_SIZE], 1200 * 1024);
    }

    #[tokio::test]
    async fn manual_cleanup_with_a_zero_cap_removes_every_unprotected_file() {
        let dir = tempfile::tempdir().unwrap();
        let in_use = write_file(dir.path(), "m42.png", 4096);
        let stale_a = write_file(dir.path(), "a_result.fits", 4096);
        let stale_b = write_file(dir.path(), "tiles/slot1/0/0_0.png", 4096);
        age(&in_use, 30);
        age(&stale_a, 20);
        age(&stale_b, 10);

        let keep = vec![in_use.to_str().unwrap().to_string()];
        let res = cleanup_output_cmd(dir.path().to_str().unwrap().to_string(), Some(0), Some(keep))
            .await
            .unwrap();

        assert_eq!(res[RES_CLEANED_FILES], 2, "{res}");
        assert!(!stale_a.exists() && !stale_b.exists(), "a cap of 0 left unprotected outputs on disk");
        assert!(in_use.exists(), "the ingest PNG of a loaded file was deleted");
        assert_eq!((res[RES_TOTAL_SIZE].as_u64(), res[RES_FILE_COUNT].as_u64()), (Some(4096), Some(1)));

        let empty = tempfile::tempdir().unwrap();
        let lone = write_file(empty.path(), "lone.fits", 4096);
        let res = cleanup_output_cmd(empty.path().to_str().unwrap().to_string(), Some(0), None)
            .await
            .unwrap();
        assert_eq!(res[RES_CLEANED_FILES], 1, "{res}");
        assert!(!lone.exists());
    }

    #[test]
    fn the_output_cap_conversion_saturates_instead_of_wrapping() {
        assert_eq!(output_cap_bytes(None), DEFAULT_OUTPUT_MAX_BYTES);
        assert_eq!(output_cap_bytes(Some(1)), 1024 * 1024);
        assert_eq!(output_cap_bytes(Some(1 << 44)), u64::MAX, "a huge cap wrapped to a small one");
        assert_eq!(output_cap_bytes(Some(u64::MAX)), u64::MAX);
    }

    #[test]
    fn startup_cleanup_keeps_history() {
        let data = tempfile::tempdir().unwrap();
        let stale = write_file(&data.path().join(OUTPUT_DIR_NAME), "a.fits", 4096);
        let tile = write_file(&data.path().join(OUTPUT_DIR_NAME), "tiles/slot0/0/0_0.png", 512);
        let image = history::image_dir(&history::history_root(data.path()), "0123456789abcdef");
        let sidecar = write_file(&image, history::SIDECAR_NAME, 128);
        let node = write_file(&image, "n0001.fits", 2880);

        clear_output_dir(data.path());

        let output = data.path().join(OUTPUT_DIR_NAME);
        assert!(output.is_dir(), "the output folder was not recreated");
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0, "launch left outputs behind");
        assert!(!stale.exists() && !tile.exists());
        assert_eq!(fs::read(&sidecar).unwrap().len(), 128, "the launch wipe reached the history sidecar");
        assert_eq!(fs::read(&node).unwrap().len(), 2880, "the launch wipe reached a history node");

        let fresh = tempfile::tempdir().unwrap();
        let first_launch = fresh.path().join("com.astroburst.app");
        clear_output_dir(&first_launch);
        assert!(first_launch.join(OUTPUT_DIR_NAME).is_dir());
    }

    fn present_ids(doc: &history::HistoryDoc) -> Vec<&str> {
        doc.nodes.iter().filter(|n| n.present).map(|n| n.id.as_str()).collect()
    }

    #[tokio::test]
    async fn history_sweep_never_removes_protected_files_and_evicts_old_branches_first() {
        use crate::core::history::fixtures::{new_doc, record, three_branches, write_bytes};
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let key = "/data/nrca1_i2d.fits#hdu=1";
        let dir_a = history::image_dir(&root, &history::id::image_id(key, None));
        let doc_a = three_branches(&dir_a, 1 << 20);
        assert_eq!(doc_a.image_key, key);
        let kernel = history::kernels::write_kernel(&dir_a, &ndarray::Array2::<f32>::ones((31, 31))).unwrap();
        let kernel_path = history::kernels::kernel_path(&dir_a, &kernel).unwrap();
        age(&kernel_path, 500_000);
        let mask = history::masks_dir(&dir_a).join("m01.json");
        write_bytes(&mask, 512, 500_000);

        let dir_b = history::image_dir(&root, &history::id::image_id("/data/other.fits", None));
        let mut doc_b = new_doc("/data/other.fits");
        let only_b = record(&dir_b, &mut doc_b, None, "arcsinh", 1 << 20);
        age(&dir_b.join(format!("{only_b}.fits")), 600_000);

        let res = cleanup_history_cmd(root.to_string_lossy().to_string(), Some(6)).await.unwrap();

        for kept in [history::sidecar_path(&dir_a), history::sidecar_path(&dir_b), kernel_path, mask] {
            assert!(kept.exists(), "{} was swept", kept.display());
        }
        let after_a = history::load(&dir_a).unwrap().unwrap();
        assert_eq!(present_ids(&after_a), ["n0001", "n0011", "n0012", "n0013"], "{res}");
        for gone in &after_a.nodes[1..10] {
            assert!(!dir_a.join(&gone.output.path).exists() || !dir_a.join(&gone.output.png).exists(), "{} kept its files", gone.id);
        }
        for pinned in ["n0001", "n0012", "n0013"] {
            assert!(dir_a.join(format!("{pinned}.fits")).exists() && dir_a.join(format!("{pinned}.png")).exists(), "pinned {pinned} was evicted");
        }
        assert_eq!(present_ids(&history::load(&dir_b).unwrap().unwrap()), [only_b.as_str()], "the head of another image was evicted");
        assert_eq!(res[RES_REPLAYABLE_NODES], 9, "{res}");
        assert!(res[RES_TOTAL_SIZE].as_u64().unwrap() <= 6 << 20, "{res}");
        assert_eq!(res[RES_HISTORY_ROOT], root.to_string_lossy().to_string());
        let cleaned: Vec<String> = serde_json::from_value(res[RES_CLEANED_PATHS].clone()).unwrap();
        assert!(cleaned.iter().any(|p| p.ends_with("n0002.fits")), "{cleaned:?}");
        assert!(!cleaned.iter().any(|p| p.ends_with("history.json") || p.contains("kernels") || p.contains("masks")), "{cleaned:?}");
    }

    #[tokio::test]
    async fn history_sweep_keeps_every_file_of_a_history_it_cannot_read() {
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let newer = history::image_dir(&root, "00000000000000aa");
        let golden = include_str!("../core/history/golden/history-v1.json").replace("\"version\": 1", "\"version\": 2");
        fs::create_dir_all(&newer).unwrap();
        fs::write(history::sidecar_path(&newer), golden).unwrap();
        let newer_node = write_file(&newer, "n0001.fits", 4096);
        age(&newer_node, 900_000);
        let orphan = write_file(&history::image_dir(&root, "00000000000000bb"), "n0001.fits", 4096);
        age(&orphan, 10);

        let res = cleanup_history_cmd(root.to_string_lossy().to_string(), Some(0)).await.unwrap();

        assert!(newer_node.exists(), "a node of a history written by a newer build was swept");
        assert!(history::sidecar_path(&newer).exists());
        assert!(!orphan.exists(), "a node file with no sidecar was kept");
        assert_eq!(res[RES_CLEANED_FILES], 1, "{res}");

        let missing = data.path().join("never_created").join(history::HISTORY_DIR_NAME);
        let res = cleanup_history_cmd(missing.to_string_lossy().to_string(), None).await.unwrap();
        assert_eq!((res[RES_CLEANED_FILES].as_u64(), res[RES_TOTAL_SIZE].as_u64()), (Some(0), Some(0)));
    }

    #[tokio::test]
    async fn history_sweep_keeps_every_file_of_a_history_whose_sidecar_is_truncated() {
        use crate::core::history::fixtures::three_branches;
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let dir = history::image_dir(&root, &history::id::image_id("/data/damaged.fits", None));
        three_branches(&dir, 4096);
        let sidecar = history::sidecar_path(&dir);
        let bytes = fs::read(&sidecar).unwrap();
        fs::write(&sidecar, &bytes[..bytes.len() / 3]).unwrap();
        let files_before = fs::read_dir(&dir).unwrap().count();

        let res = cleanup_history_cmd(root.to_string_lossy().to_string(), Some(0)).await.unwrap();

        assert_eq!(res[RES_CLEANED_FILES], 0, "{res}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), files_before, "a node of a damaged history was swept");
    }

    #[tokio::test]
    async fn history_sweep_evicts_abandoned_branches_before_the_head_path() {
        use crate::core::history::fixtures::three_branches;
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let dir = history::image_dir(&root, &history::id::image_id("/data/nrca1_i2d.fits#hdu=1", None));
        let mut doc = three_branches(&dir, 1 << 20);
        history::set_head(&dir, &mut doc, Some("n0005")).unwrap();

        let res = cleanup_history_cmd(root.to_string_lossy().to_string(), Some(6)).await.unwrap();

        let after = history::load(&dir).unwrap().unwrap();
        assert_eq!(present_ids(&after), ["n0001", "n0002", "n0003", "n0004", "n0005"], "{res}");
        assert!(res[RES_TOTAL_SIZE].as_u64().unwrap() <= 6 << 20, "{res}");
        assert_eq!(res[RES_REPLAYABLE_NODES], 8, "{res}");
    }

    fn aged_image(root: &Path, key: &str, seconds_ago: u64) -> PathBuf {
        use crate::core::history::fixtures::{chain, new_doc, STAGES};
        let dir = history::image_dir(root, &history::id::image_id(key, None));
        let mut doc = new_doc(key);
        let ids = chain(&dir, &mut doc, None, &STAGES, 1 << 20);
        for (i, id) in ids.iter().enumerate() {
            let node_age = seconds_ago + 10 * (ids.len() - i) as u64;
            age(&dir.join(format!("{id}.fits")), node_age);
            age(&dir.join(format!("{id}.png")), node_age);
        }
        age(&history::sidecar_path(&dir), seconds_ago);
        dir
    }

    #[tokio::test]
    async fn history_sweep_reaches_the_cap_across_images_by_releasing_the_oldest_pins_first() {
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let oldest = aged_image(&root, "/data/a.fits", 300_000);
        let middle = aged_image(&root, "/data/b.fits", 200_000);
        let newest = aged_image(&root, "/data/c.fits", 100_000);

        let res = cleanup_history_cmd(root.to_string_lossy().to_string(), Some(4)).await.unwrap();

        assert!(res[RES_TOTAL_SIZE].as_u64().unwrap() <= 4 << 20, "the sweep stopped above the cap: {res}");
        assert_eq!(present_ids(&history::load(&newest).unwrap().unwrap()), ["n0003", "n0004", "n0005"], "{res}");
        assert!(present_ids(&history::load(&middle).unwrap().unwrap()).is_empty(), "{res}");
        assert!(present_ids(&history::load(&oldest).unwrap().unwrap()).is_empty(), "{res}");
        for dir in [&oldest, &middle, &newest] {
            assert!(history::sidecar_path(dir).exists());
        }
        assert_eq!(res[RES_REPLAYABLE_NODES], 12, "{res}");
    }

    #[tokio::test]
    async fn history_sweep_refuses_a_root_that_is_not_an_absolute_history_folder() {
        let data = tempfile::tempdir().unwrap();
        let output = data.path().join(OUTPUT_DIR_NAME);
        let user_file = write_file(&output.join("0123456789abcdef"), "n0001.fits", 4096);

        for root in [output.to_string_lossy().to_string(), history::HISTORY_DIR_NAME.to_string(), "./history".to_string()] {
            let err = cleanup_history_cmd(root.clone(), Some(0)).await.unwrap_err();
            assert!(err.contains("history"), "{root}: {err}");
        }
        assert!(user_file.exists(), "the history sweep deleted a file outside the history root");
    }

    #[tokio::test]
    async fn history_sweep_only_touches_image_id_folders() {
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let loose = write_file(&root, "notes.txt", 4096);
        let foreign = write_file(&root.join("Presets"), "n0001.fits", 4096);
        let upper = write_file(&root.join("FEDCBA9876543210"), "n0001.fits", 4096);
        let orphan = write_file(&root.join("0123456789abcdef"), "n0001.fits", 4096);

        let res = cleanup_history_cmd(root.to_string_lossy().to_string(), Some(0)).await.unwrap();

        assert!(!orphan.exists(), "an orphan node file in an image folder was kept");
        assert!(loose.exists() && foreign.exists() && upper.exists(), "the sweep reached outside the image folders: {res}");
        assert_eq!(res[RES_CLEANED_FILES], 1, "{res}");
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

    #[tokio::test]
    async fn history_sweep_leaves_an_image_alone_while_a_run_holds_it() {
        use crate::cmd::ops::RunGuard;
        use crate::core::history::fixtures::three_branches;
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let id = history::id::image_id("/data/busy.fits", None);
        let dir = history::image_dir(&root, &id);
        three_branches(&dir, 4096);
        let in_flight = write_file(&dir, "n0014.fits", 4096);
        age(&in_flight, 900_000);
        let sidecar = fs::read(history::sidecar_path(&dir)).unwrap();
        let files_before = fs::read_dir(&dir).unwrap().count();

        let guard = RunGuard::acquire(&dir).unwrap();
        let res = cleanup_history_cmd(root.to_string_lossy().to_string(), Some(0)).await.unwrap();
        assert_eq!(res[RES_CLEANED_FILES], 0, "{res}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), files_before, "the sweep removed files of an image whose run is in flight");
        assert_eq!(fs::read(history::sidecar_path(&dir)).unwrap(), sidecar, "the sweep rewrote a sidecar a run is appending to");
        drop(guard);

        let res = cleanup_history_cmd(root.to_string_lossy().to_string(), Some(0)).await.unwrap();
        assert!(res[RES_CLEANED_FILES].as_u64().unwrap() > 0, "{res}");
        assert!(!in_flight.exists());
        assert!(RunGuard::acquire(&dir).is_ok(), "the sweep kept the image locked");
    }

    #[test]
    fn history_sweep_never_touches_an_image_folder_that_appears_after_it_listed_the_root() {
        use crate::cmd::ops::RunGuard;
        use crate::core::history::fixtures::{new_doc, record, three_branches};
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let old = history::image_dir(&root, &history::id::image_id("/data/nrca1_i2d.fits#hdu=1", None));
        three_branches(&old, 4096);

        let scan = scan_history_root(&root);
        let fresh = history::image_dir(&root, &history::id::image_id("/data/fresh.fits", None));
        let run = RunGuard::acquire(&fresh).unwrap();
        let mut doc = new_doc("/data/fresh.fits");
        let first = record(&fresh, &mut doc, None, "arcsinh", 4096);
        let node = fresh.join(format!("{first}.fits"));
        age(&node, 900_000);
        let sweep = sweep_scanned(&root, 0, scan).unwrap();
        drop(run);

        assert!(history::sidecar_path(&fresh).exists(), "the sweep deleted the sidecar of a run that started after it listed the root");
        assert!(node.exists() && fresh.join(format!("{first}.png")).exists(), "the sweep deleted the first node of a new image");
        assert!(sweep.removed_files > 0, "the old image was not swept at all");
        assert!(!sweep.removed_paths.iter().any(|p| Path::new(p).starts_with(&fresh)), "{:?}", sweep.removed_paths);
    }

    #[test]
    fn a_step_during_the_history_sweep_is_refused_with_the_cleanup_reason() {
        use crate::cmd::ops::{move_head, HeadMove};
        use crate::core::history::fixtures::three_branches;
        let data = tempfile::tempdir().unwrap();
        let root = history::history_root(data.path());
        let key = "/data/nrca1_i2d.fits#hdu=1";
        three_branches(&history::image_dir(&root, &history::id::image_id(key, None)), 64);

        let scan = scan_history_root(&root);
        let err = move_head(&root, key, None, HeadMove::Undo).unwrap_err().to_string();
        assert!(err.contains("history cleanup is running"), "{err}");
        drop(scan);
        assert_eq!(move_head(&root, key, None, HeadMove::Undo).unwrap()["head"], "n0012");
    }
}
