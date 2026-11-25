use std::fs;
use std::path::Path;

use crate::core::history::doc::{HistoryDoc, Node};
use crate::core::history::error::HistoryError;
use super::sidecar::{last_used, load, save, sidecar_path};

pub fn mark_evicted(image_dir: &Path) -> Result<Vec<String>, HistoryError> {
    let Some(mut doc) = load(image_dir)? else {
        return Ok(Vec::new());
    };
    let previous_use = last_used(image_dir)?;
    let marked = mark_missing(image_dir, &mut doc);
    if !marked.is_empty() {
        fs::File::options().write(true).open(sidecar_path(image_dir))?.set_modified(previous_use)?;
    }
    Ok(marked)
}

pub fn mark_missing(image_dir: &Path, doc: &mut HistoryDoc) -> Vec<String> {
    mark_missing_where(image_dir, doc, |_| true)
}

pub fn mark_missing_on_path(image_dir: &Path, doc: &mut HistoryDoc, target: &str) -> Vec<String> {
    let path: Vec<String> = doc.ancestry(target).iter().map(|n| n.id.clone()).collect();
    mark_missing_where(image_dir, doc, |node| path.contains(&node.id))
}

fn mark_missing_where(image_dir: &Path, doc: &mut HistoryDoc, checked: impl Fn(&Node) -> bool) -> Vec<String> {
    if doc.read_only_reason().is_some() {
        return Vec::new();
    }
    let mut marked = Vec::new();
    for node in doc.nodes.iter_mut().filter(|n| n.present && checked(n)) {
        if !node.output.files().all(|file| image_dir.join(file).exists()) {
            node.present = false;
            marked.push(node.id.clone());
        }
    }
    if !marked.is_empty() {
        if let Err(e) = save(image_dir, doc) {
            log::warn!("history: could not record {} evicted nodes in {}: {e}", marked.len(), image_dir.display());
        }
    }
    marked
}
