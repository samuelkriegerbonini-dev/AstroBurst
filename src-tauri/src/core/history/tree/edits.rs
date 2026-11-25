use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::core::history::doc::{node_id, node_number, HistoryDoc, ImageInfo, NewNode, Node, NodeOutput, APP_VERSION, FITS_EXTENSION, PNG_EXTENSION};
use crate::core::history::error::HistoryError;
use crate::core::history::sidecar::save;
use crate::core::history::stamp::utc_timestamp;

const BYTES_PER_SAMPLE: u64 = 4;

fn node_files_exist(image_dir: &Path, id: &str) -> bool {
    [FITS_EXTENSION, PNG_EXTENSION].iter().any(|ext| image_dir.join(format!("{id}.{ext}")).exists())
}

pub fn next_node_id(image_dir: &Path, doc: &HistoryDoc) -> String {
    let mut number = doc.nodes.iter().filter_map(|n| node_number(&n.id)).max().unwrap_or(0).max(doc.last_issued) + 1;
    while node_files_exist(image_dir, &node_id(number)) {
        number += 1;
    }
    node_id(number)
}

fn measure(image_dir: &Path, output: &NodeOutput) -> (bool, u64) {
    output.files().fold((true, 0), |(present, bytes), file| match fs::metadata(image_dir.join(file)) {
        Ok(meta) => (present, bytes + meta.len()),
        Err(_) => (false, bytes),
    })
}

fn save_or_restore(image_dir: &Path, doc: &mut HistoryDoc, restore: impl FnOnce(&mut HistoryDoc)) -> Result<(), HistoryError> {
    let saved = save(image_dir, doc);
    if saved.is_err() {
        restore(doc);
    }
    saved
}

pub fn append(image_dir: &Path, doc: &mut HistoryDoc, node: NewNode) -> Result<(), HistoryError> {
    doc.ensure_writable()?;
    if doc.node(&node.id).is_some() {
        return Err(HistoryError::NodeExists(node.id));
    }
    let depth = match node.parent.as_deref() {
        Some(parent) => match doc.ancestry(parent).len() {
            0 => return Err(HistoryError::UnknownNode(parent.to_string())),
            above => above + 1,
        },
        None => 1,
    };
    let (present, bytes) = measure(image_dir, &node.output);
    let every = doc.policy.k.max(1) as usize;
    let previous_head = doc.head.replace(node.id.clone());
    let previous_issued = doc.last_issued;
    doc.last_issued = previous_issued.max(node_number(&node.id).unwrap_or(0));
    doc.nodes.push(Node {
        id: node.id,
        parent: node.parent,
        op: node.op,
        mask: node.mask,
        output: node.output,
        at: utc_timestamp(SystemTime::now()),
        elapsed_ms: node.elapsed_ms,
        app: APP_VERSION.to_string(),
        threads: rayon::current_num_threads(),
        present,
        bytes,
        checkpoint: depth % every == 0,
        replayable: node.replayable,
        rebuilt_app: None,
        rebuilt_at: None,
    });
    save_or_restore(image_dir, doc, |doc| {
        doc.nodes.pop();
        doc.head = previous_head;
        doc.last_issued = previous_issued;
    })
}

pub fn set_head(image_dir: &Path, doc: &mut HistoryDoc, head: Option<&str>) -> Result<(), HistoryError> {
    doc.ensure_writable()?;
    if let Some(id) = head.filter(|id| doc.node(id).is_none()) {
        return Err(HistoryError::UnknownNode(id.to_string()));
    }
    let previous = std::mem::replace(&mut doc.head, head.map(str::to_string));
    save_or_restore(image_dir, doc, |doc| doc.head = previous)
}

pub fn restamp(doc: &mut HistoryDoc, image: ImageInfo) {
    for node in &mut doc.nodes {
        node.replayable = false;
    }
    doc.image = image;
}

pub fn estimate_next_node_bytes(dims: [usize; 2], planes: usize) -> u64 {
    dims[0] as u64 * dims[1] as u64 * planes as u64 * BYTES_PER_SAMPLE
}

pub fn mark_restored(image_dir: &Path, doc: &mut HistoryDoc, id: &str) -> Result<(), HistoryError> {
    doc.ensure_writable()?;
    let index = doc.nodes.iter().position(|n| n.id == id).ok_or_else(|| HistoryError::UnknownNode(id.to_string()))?;
    let (present, bytes) = measure(image_dir, &doc.nodes[index].output);
    let node = &mut doc.nodes[index];
    let previous = (node.present, node.bytes, node.rebuilt_app.clone(), node.rebuilt_at.clone());
    (node.present, node.bytes) = (present, bytes);
    node.rebuilt_app = Some(APP_VERSION.to_string());
    node.rebuilt_at = Some(utc_timestamp(SystemTime::now()));
    save_or_restore(image_dir, doc, |doc| {
        let node = &mut doc.nodes[index];
        (node.present, node.bytes, node.rebuilt_app, node.rebuilt_at) = previous;
    })
}

pub fn remove_subtree(image_dir: &Path, doc: &mut HistoryDoc, id: &str) -> Result<Vec<Node>, HistoryError> {
    doc.ensure_writable()?;
    let parent = doc.node(id).ok_or_else(|| HistoryError::UnknownNode(id.to_string()))?.parent.clone();
    let mut doomed = vec![id.to_string()];
    let mut cursor = 0;
    while cursor < doomed.len() {
        let children = doc.nodes.iter().filter(|n| n.parent.as_deref() == Some(doomed[cursor].as_str())).map(|n| n.id.clone());
        let fresh: Vec<String> = children.filter(|child| !doomed.contains(child)).collect();
        doomed.extend(fresh);
        cursor += 1;
    }
    let previous = (doc.nodes.clone(), doc.head.clone());
    let (removed, kept): (Vec<Node>, Vec<Node>) = std::mem::take(&mut doc.nodes).into_iter().partition(|n| doomed.contains(&n.id));
    doc.nodes = kept;
    if doc.head.as_ref().is_some_and(|head| doomed.contains(head)) {
        doc.head = parent;
    }
    save_or_restore(image_dir, doc, |doc| (doc.nodes, doc.head) = previous)?;
    Ok(removed)
}
