use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::history::doc::{HistoryDoc, Node};
use super::sidecar::{kernels_dir, masks_dir, sidecar_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinLevel {
    Full,
    HeadAndParent,
    Floor,
}

fn head_path(doc: &HistoryDoc) -> Vec<&Node> {
    match doc.head.as_deref() {
        Some(head) => doc.ancestry(head),
        None => doc.redo_target().into_iter().collect(),
    }
}

fn replay_cannot_rebuild(doc: &HistoryDoc, node: &Node) -> bool {
    !node.replayable || (node.parent.is_none() && doc.is_rebased())
}

fn pinned_nodes(doc: &HistoryDoc, level: PinLevel) -> Vec<&Node> {
    let path = head_path(doc);
    let head_and_parent = path.len().saturating_sub(2);
    let on_path: Vec<&str> = path
        .into_iter()
        .enumerate()
        .filter(|&(depth, node)| {
            (depth == 0 && level == PinLevel::Full)
                || (level != PinLevel::Floor && depth >= head_and_parent)
                || (level == PinLevel::Full && node.checkpoint)
        })
        .map(|(_, node)| node.id.as_str())
        .collect();
    doc.nodes.iter().filter(|node| replay_cannot_rebuild(doc, node) || on_path.contains(&node.id.as_str())).collect()
}

fn pinned_bytes(doc: &HistoryDoc, level: PinLevel) -> u64 {
    pinned_nodes(doc, level).iter().filter(|n| n.present).map(|n| n.bytes).sum()
}

pub fn pin_levels(docs: &[(&HistoryDoc, SystemTime)], cap_bytes: u64) -> Vec<PinLevel> {
    let mut levels = vec![PinLevel::Full; docs.len()];
    let mut pinned: Vec<u64> = docs.iter().map(|(doc, _)| pinned_bytes(doc, PinLevel::Full)).collect();
    let mut total: u64 = pinned.iter().sum();
    let mut oldest_first: Vec<usize> = (0..docs.len()).collect();
    oldest_first.sort_by_key(|&i| docs[i].1);
    let newest = oldest_first.last().copied();
    for level in [PinLevel::HeadAndParent, PinLevel::Floor] {
        for &i in &oldest_first {
            if total <= cap_bytes {
                return levels;
            }
            if level == PinLevel::Floor && Some(i) == newest {
                continue;
            }
            let lowered = pinned_bytes(docs[i].0, level);
            total = total - pinned[i] + lowered;
            (levels[i], pinned[i]) = (level, lowered);
        }
    }
    levels
}

fn node_files(node: &Node) -> Vec<String> {
    node.output.files().map(str::to_string).collect()
}

pub fn pinned_paths(doc: &HistoryDoc, level: PinLevel) -> Vec<String> {
    pinned_nodes(doc, level).into_iter().flat_map(node_files).collect()
}

pub fn active_paths(doc: &HistoryDoc) -> Vec<String> {
    let redo = doc.head.as_ref().and(doc.redo_target());
    head_path(doc).into_iter().chain(redo).flat_map(node_files).collect()
}

pub fn protected_paths(doc: &HistoryDoc, image_dir: &Path, level: PinLevel) -> Vec<PathBuf> {
    let side_files = [sidecar_path(image_dir), kernels_dir(image_dir), masks_dir(image_dir)];
    side_files.into_iter().chain(pinned_paths(doc, level).into_iter().map(|file| image_dir.join(file))).collect()
}
