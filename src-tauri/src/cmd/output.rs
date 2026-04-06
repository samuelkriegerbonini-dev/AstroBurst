use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::infra::config::load_config;
use crate::types::constants::{
    DEFAULT_OUTPUT_MAX_BYTES, RES_CLEANED_BYTES, RES_CLEANED_FILES,
    RES_ELAPSED_MS, RES_FILE_COUNT, RES_OUTPUT_DIR, RES_TOTAL_SIZE,
};

static OUTPUT_MAX_BYTES: OnceLock<u64> = OnceLock::new();

fn get_max_bytes() -> u64 {
    *OUTPUT_MAX_BYTES.get_or_init(|| {
        load_config()
            .ok()
            .and_then(|c| c.output_max_size_mb)
            .map(|mb| mb.saturating_mul(1024 * 1024))
            .unwrap_or(DEFAULT_OUTPUT_MAX_BYTES)
    })
}

struct FileEntry {
    path: std::path::PathBuf,
    size: u64,
    mtime: SystemTime,
}

fn walk_dir_entries(dir: &Path) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    if !dir.exists() {
        return Ok(entries);
    }
    for entry in fs::read_dir(dir).context("Failed to read output directory")? {
        let entry = entry?;
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        entries.push(FileEntry {
            path: entry.path(),
            size: meta.len(),
            mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    Ok(entries)
}

pub(crate) fn enforce_output_lru(dir: &Path, max_bytes: u64) -> Result<(usize, u64)> {
    let mut entries = walk_dir_entries(dir)?;
    let total: u64 = entries.iter().map(|e| e.size).sum();

    if total <= max_bytes {
        return Ok((0, 0));
    }

    entries.sort_by(|a, b| a.mtime.cmp(&b.mtime));

    let mut current = total;
    let mut removed_count = 0usize;
    let mut removed_bytes = 0u64;

    for entry in &entries {
        if current <= max_bytes {
            break;
        }
        match fs::remove_file(&entry.path) {
            Ok(()) => {
                current = current.saturating_sub(entry.size);
                removed_count += 1;
                removed_bytes += entry.size;
                log::info!(
                    "LRU cleanup: removed {} ({} bytes)",
                    entry.path.display(),
                    entry.size
                );
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

    Ok((removed_count, removed_bytes))
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
            RES_FILE_COUNT: file_count,
        }))
    })
}

#[tauri::command]
pub async fn cleanup_output_cmd(
    output_dir: String,
    max_size_mb: Option<u64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = std::time::Instant::now();
        let path = Path::new(&output_dir);
        let max_bytes = max_size_mb
            .map(|mb| mb.saturating_mul(1024 * 1024))
            .unwrap_or_else(get_max_bytes);

        let (cleaned_files, cleaned_bytes) = enforce_output_lru(path, max_bytes)?;
        let (total_size, file_count) = dir_info(path)?;

        Ok(json!({
            RES_CLEANED_FILES: cleaned_files,
            RES_CLEANED_BYTES: cleaned_bytes,
            RES_TOTAL_SIZE: total_size,
            RES_FILE_COUNT: file_count,
            RES_OUTPUT_DIR: output_dir,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}
