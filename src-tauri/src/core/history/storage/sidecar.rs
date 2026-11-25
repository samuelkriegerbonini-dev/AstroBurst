use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use serde_json::Value;

use crate::core::history::doc::{read_only_reason, HistoryDoc, HISTORY_FORMAT};
use crate::core::ops::OpSpec;
use crate::core::history::error::HistoryError;

pub const HISTORY_DIR_NAME: &str = "history";
pub const SIDECAR_NAME: &str = "history.json";
pub const KERNELS_DIR_NAME: &str = "kernels";
pub const MASKS_DIR_NAME: &str = "masks";
const TEMP_SUFFIX: &str = ".tmp";
pub(in crate::core::history) const STALE_TEMP_AGE: Duration = Duration::from_secs(300);

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn history_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(HISTORY_DIR_NAME)
}

pub fn image_dir(root: &Path, image_id: &str) -> PathBuf {
    root.join(image_id)
}

pub fn sidecar_path(image_dir: &Path) -> PathBuf {
    image_dir.join(SIDECAR_NAME)
}

pub fn kernels_dir(image_dir: &Path) -> PathBuf {
    image_dir.join(KERNELS_DIR_NAME)
}

pub fn masks_dir(image_dir: &Path) -> PathBuf {
    image_dir.join(MASKS_DIR_NAME)
}

fn temp_prefix(path: &Path) -> String {
    format!("{}.", path.file_name().unwrap_or_default().to_string_lossy())
}

pub(in crate::core::history) fn temp_path(path: &Path) -> PathBuf {
    let writer = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!("{}{}-{writer}{TEMP_SUFFIX}", temp_prefix(path), std::process::id()))
}

fn is_temp_of(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).and_then(|rest| rest.strip_suffix(TEMP_SUFFIX)).is_some_and(|writer| !writer.is_empty())
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = temp_path(path);
    let renamed = fs::write(&temp, bytes).and_then(|()| fs::rename(&temp, path));
    if renamed.is_err() {
        let _ = fs::remove_file(&temp);
    }
    renamed
}

fn remove_stale_temps(path: &Path) {
    let Some(Ok(listing)) = path.parent().map(fs::read_dir) else {
        return;
    };
    let prefix = temp_prefix(path);
    let now = SystemTime::now();
    for entry in listing.flatten().filter(|entry| is_temp_of(&entry.file_name().to_string_lossy(), &prefix)) {
        let age = entry.metadata().and_then(|meta| meta.modified()).ok().and_then(|at| now.duration_since(at).ok());
        if age.is_some_and(|age| age >= STALE_TEMP_AGE) {
            if let Err(e) = fs::remove_file(entry.path()) {
                log::warn!("history: could not remove the interrupted write {}: {e}", entry.path().display());
            }
        }
    }
}

fn unreadable(path: &Path, reason: impl std::fmt::Display) -> HistoryError {
    HistoryError::Unreadable { path: path.display().to_string(), reason: reason.to_string() }
}

#[derive(Deserialize)]
struct Envelope {
    format: String,
    version: u32,
}

pub fn load(image_dir: &Path) -> Result<Option<HistoryDoc>, HistoryError> {
    let sidecar = sidecar_path(image_dir);
    remove_stale_temps(&sidecar);
    let bytes = match fs::read(&sidecar) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|e| unreadable(&sidecar, e))?;
    if envelope.format != HISTORY_FORMAT {
        return Err(unreadable(&sidecar, format!("format is \"{}\", expected \"{HISTORY_FORMAT}\"", envelope.format)));
    }
    if let Some(reason) = read_only_reason(envelope.version) {
        return Err(HistoryError::ReadOnly(reason));
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|e| match unrunnable_step(&bytes) {
        Some(reason) => HistoryError::ReadOnly(reason),
        None => unreadable(&sidecar, e),
    })
}

fn unrunnable_step(bytes: &[u8]) -> Option<String> {
    let doc: Value = serde_json::from_slice(bytes).ok()?;
    doc.get("nodes")?.as_array()?.iter().find_map(|node| {
        let err = OpSpec::deserialize(node.get("op")?).err()?;
        let id = node.get("id").and_then(Value::as_str).unwrap_or("a step");
        Some(format!("{id} holds a step this build cannot run ({err}); the history opens read-only"))
    })
}

pub fn last_used(image_dir: &Path) -> io::Result<SystemTime> {
    fs::metadata(sidecar_path(image_dir))?.modified()
}

pub fn save(image_dir: &Path, doc: &HistoryDoc) -> Result<(), HistoryError> {
    doc.ensure_writable()?;
    fs::create_dir_all(image_dir)?;
    write_atomic(&sidecar_path(image_dir), &serde_json::to_vec(doc)?)?;
    Ok(())
}
