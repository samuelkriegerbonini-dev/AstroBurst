use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::Query;
use axum::{Extension, Json};
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};

use super::error::ScienceError;
use super::guard::resolve_under_roots;
use super::state::ScienceState;
use crate::error::AppError;

const MAX_LISTED_ENTRIES: usize = 5000;
const FITS_SUFFIXES: [&str; 7] = [
    ".fits", ".fit", ".fts", ".fz", ".fits.gz", ".fit.gz", ".fts.gz",
];
const ASDF_SUFFIX: &str = ".asdf";

#[derive(Deserialize)]
pub(crate) struct FsQuery {
    dir: Option<String>,
    glob: Option<String>,
}

pub(crate) async fn list(
    Extension(science): Extension<Arc<ScienceState>>,
    Query(query): Query<FsQuery>,
) -> Result<Json<Value>, ScienceError> {
    let requested = query
        .dir
        .filter(|dir| !dir.trim().is_empty())
        .unwrap_or_else(|| science.cfg.data_root.to_string_lossy().into_owned());
    let glob = query.glob.filter(|glob| !glob.is_empty());
    let listing = tokio::task::spawn_blocking(move || {
        let dir = resolve_under_roots(&science.cfg.roots(), &requested)?;
        let pattern = glob.as_deref().map(glob_pattern).transpose()?;
        listing(&requested, &dir, pattern.as_ref())
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))??;
    Ok(Json(listing))
}

fn glob_pattern(glob: &str) -> Result<Regex, ScienceError> {
    let body: String = glob
        .chars()
        .map(|c| match c {
            '*' => ".*".to_owned(),
            '?' => ".".to_owned(),
            other => regex::escape(&other.to_string()),
        })
        .collect();
    Regex::new(&format!("(?i)^{body}$"))
        .map_err(|e| AppError::BadRequest(format!("glob {glob:?} cannot be used: {e}")).into())
}

fn listing(requested: &str, dir: &Path, pattern: Option<&Regex>) -> Result<Value, ScienceError> {
    if !dir.exists() {
        return Err(AppError::NotFound(format!("folder {requested} not found")).into());
    }
    if !dir.is_dir() {
        return Err(AppError::BadRequest(format!("{requested} is not a folder")).into());
    }
    let read = std::fs::read_dir(dir)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("cannot list {requested}: {e}")))?;
    let mut entries: Vec<(String, Value)> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok().filter(|name| !name.starts_with('.'))?;
            let meta = entry.metadata().ok()?;
            let kind = kind_of(&name, meta.is_dir());
            let listed = kind == "dir" || pattern.is_none_or(|p| p.is_match(&name));
            listed.then(|| {
                let modified_ms = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
                let bytes = (!meta.is_dir()).then_some(meta.len());
                (name.clone(), json!({ "name": name, "kind": kind, "bytes": bytes, "modified_ms": modified_ms }))
            })
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let truncated = entries.len() > MAX_LISTED_ENTRIES;
    entries.truncate(MAX_LISTED_ENTRIES);
    Ok(json!({
        "dir": dir.to_string_lossy(),
        "count": entries.len(),
        "truncated": truncated,
        "entries": entries.into_iter().map(|(_, entry)| entry).collect::<Vec<_>>(),
    }))
}

fn kind_of(name: &str, is_dir: bool) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if is_dir {
        "dir"
    } else if FITS_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix)) {
        "fits"
    } else if lower.ends_with(ASDF_SUFFIX) {
        "asdf"
    } else {
        "other"
    }
}
