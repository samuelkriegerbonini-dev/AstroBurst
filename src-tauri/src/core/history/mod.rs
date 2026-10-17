pub mod id;
pub mod kernels;
pub mod stamp;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::ops::{OpSpec, OutputValues};
use crate::types::image_ref::ImageRef;

use self::id::{canonical_ref, image_id, rebase_ref};
use self::stamp::{file_stamp, utc_timestamp};

pub const HISTORY_FORMAT: &str = "astroburst-history";
pub const HISTORY_VERSION: u32 = 1;
pub const HISTORY_DIR_NAME: &str = "history";
pub const SIDECAR_NAME: &str = "history.json";
pub const KERNELS_DIR_NAME: &str = "kernels";
pub const MASKS_DIR_NAME: &str = "masks";
pub const DEFAULT_CHECKPOINT_EVERY: u32 = 4;
pub const HEADER_ABHIST: &str = "ABHIST";
pub const HEADER_ABNODE: &str = "ABNODE";

const TEMP_SUFFIX: &str = ".tmp";
const STALE_TEMP_AGE: Duration = Duration::from_secs(300);
const NODE_PREFIX: &str = "n";
const FITS_EXTENSION: &str = "fits";
const PNG_EXTENSION: &str = "png";
const BYTES_PER_SAMPLE: u64 = 4;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("{0}")]
    ReadOnly(String),
    #[error("{path} cannot be read as an AstroBurst history: {reason}")]
    Unreadable { path: String, reason: String },
    #[error("no node {0} in this history")]
    UnknownNode(String),
    #[error("node {0} is already in this history; node files are never overwritten")]
    NodeExists(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFingerprint {
    #[serde(rename = "ref")]
    pub file_ref: String,
    pub size: u64,
    pub mtime: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    #[serde(rename = "ref")]
    pub source_ref: String,
    pub id: String,
    pub size: u64,
    pub mtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<FileFingerprint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinLevel {
    Full,
    HeadAndParent,
    Floor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub k: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self { k: DEFAULT_CHECKPOINT_EVERY }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaskRef {
    Id(String),
    Recipe(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeOutput {
    pub path: String,
    pub png: String,
    pub extras: Vec<String>,
    pub dims: [usize; 2],
    pub planes: usize,
    pub domain: OutputValues,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub parent: Option<String>,
    pub op: OpSpec,
    pub mask: Option<MaskRef>,
    pub output: NodeOutput,
    pub at: String,
    pub elapsed_ms: u64,
    pub app: String,
    pub threads: usize,
    pub present: bool,
    pub bytes: u64,
    pub checkpoint: bool,
    #[serde(default = "replayable_by_default", skip_serializing_if = "is_replayable")]
    pub replayable: bool,
}

fn replayable_by_default() -> bool {
    true
}

fn is_replayable(replayable: &bool) -> bool {
    *replayable
}

fn is_unissued(last_issued: &u32) -> bool {
    *last_issued == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryDoc {
    pub format: String,
    pub version: u32,
    pub image: ImageInfo,
    pub image_key: String,
    pub base_path: Option<String>,
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "is_unissued")]
    pub last_issued: u32,
    pub policy: Policy,
    pub app: String,
    pub threads: usize,
    pub masks: BTreeMap<String, Value>,
    pub nodes: Vec<Node>,
}

pub struct NewNode {
    pub id: String,
    pub parent: Option<String>,
    pub op: OpSpec,
    pub mask: Option<MaskRef>,
    pub output: NodeOutput,
    pub elapsed_ms: u64,
    pub replayable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayPlan {
    pub from: Option<String>,
    pub steps: Vec<String>,
}

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

fn temp_path(path: &Path) -> PathBuf {
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

fn mtime_text(time: Option<SystemTime>) -> Option<String> {
    time.map(utc_timestamp)
}

fn stamp_of(file_key: &str) -> io::Result<(u64, Option<String>)> {
    let (size, mtime) = file_stamp(&fs::metadata(ImageRef::parse(file_key).path)?);
    Ok((size, mtime_text(mtime)))
}

impl FileFingerprint {
    fn of(file_key: &str, file_ref: String) -> io::Result<Self> {
        let (size, mtime) = stamp_of(file_key)?;
        Ok(Self { file_ref, size, mtime })
    }

    fn matches(&self, file_key: &str) -> bool {
        stamp_of(file_key).is_ok_and(|(size, mtime)| size == self.size && mtime == self.mtime)
    }
}

impl ImageInfo {
    pub fn of(image_key: &str, base_path: Option<&str>) -> io::Result<Self> {
        let (size, mtime) = stamp_of(image_key)?;
        let base = match (rebase_ref(image_key, base_path), base_path) {
            (Some(base_ref), Some(base)) => Some(FileFingerprint::of(base, base_ref)?),
            _ => None,
        };
        Ok(Self { source_ref: canonical_ref(image_key), id: image_id(image_key, base_path), size, mtime, base })
    }

    pub fn fingerprint_matches(&self, image_key: &str, base_path: Option<&str>) -> bool {
        let source_matches = stamp_of(image_key).is_ok_and(|(size, mtime)| size == self.size && mtime == self.mtime);
        let base_matches = match (&self.base, rebase_ref(image_key, base_path), base_path) {
            (None, None, _) => true,
            (Some(stamp), Some(_), Some(base)) => stamp.matches(base),
            _ => false,
        };
        self.id == image_id(image_key, base_path) && source_matches && base_matches
    }
}

fn node_id(number: u32) -> String {
    format!("{NODE_PREFIX}{number:04}")
}

fn node_number(id: &str) -> Option<u32> {
    id.strip_prefix(NODE_PREFIX)?.parse().ok()
}

impl NodeOutput {
    pub fn named(id: &str, dims: [usize; 2], planes: usize, domain: OutputValues, extra_suffixes: &[&str]) -> Self {
        Self {
            path: format!("{id}.{FITS_EXTENSION}"),
            png: format!("{id}.{PNG_EXTENSION}"),
            extras: extra_suffixes.iter().map(|suffix| format!("{id}_{suffix}.{PNG_EXTENSION}")).collect(),
            dims,
            planes,
            domain,
        }
    }

    pub fn files(&self) -> impl Iterator<Item = &str> {
        [self.path.as_str(), self.png.as_str()].into_iter().chain(self.extras.iter().map(String::as_str))
    }
}

impl HistoryDoc {
    pub fn new(image: ImageInfo, image_key: &str, base_path: Option<&str>) -> Self {
        Self {
            format: HISTORY_FORMAT.to_string(),
            version: HISTORY_VERSION,
            image,
            image_key: image_key.to_string(),
            base_path: base_path.map(str::to_string),
            head: None,
            last_issued: 0,
            policy: Policy::default(),
            app: APP_VERSION.to_string(),
            threads: rayon::current_num_threads(),
            masks: BTreeMap::new(),
            nodes: Vec::new(),
        }
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn read_only_reason(&self) -> Option<String> {
        read_only_reason(self.version)
    }

    fn ensure_writable(&self) -> Result<(), HistoryError> {
        self.read_only_reason().map_or(Ok(()), |reason| Err(HistoryError::ReadOnly(reason)))
    }

    pub fn is_rebased(&self) -> bool {
        rebase_ref(&self.image_key, self.base_path.as_deref()).is_some()
    }

    pub fn redo_target(&self) -> Option<&Node> {
        self.nodes.iter().filter(|n| n.parent == self.head).max_by_key(|n| node_number(&n.id))
    }

    pub fn ancestry(&self, id: &str) -> Vec<&Node> {
        let by_id: HashMap<&str, &Node> = self.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut path = Vec::new();
        let mut cursor = by_id.get(id).copied();
        while let Some(node) = cursor {
            if path.len() == self.nodes.len() {
                break;
            }
            path.push(node);
            cursor = node.parent.as_deref().and_then(|p| by_id.get(p).copied());
        }
        path.reverse();
        path
    }
}

fn read_only_reason(version: u32) -> Option<String> {
    (version != HISTORY_VERSION).then(|| {
        format!("this history was written with history format version {version}; this build writes version {HISTORY_VERSION}, so it opens read-only")
    })
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
    serde_json::from_slice(&bytes).map(Some).map_err(|e| unreadable(&sidecar, e))
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

pub fn mark_restored(image_dir: &Path, doc: &mut HistoryDoc, id: &str) -> Result<(), HistoryError> {
    doc.ensure_writable()?;
    let index = doc.nodes.iter().position(|n| n.id == id).ok_or_else(|| HistoryError::UnknownNode(id.to_string()))?;
    let (present, bytes) = measure(image_dir, &doc.nodes[index].output);
    let node = &mut doc.nodes[index];
    let previous = (node.present, node.bytes);
    (node.present, node.bytes) = (present, bytes);
    save_or_restore(image_dir, doc, |doc| (doc.nodes[index].present, doc.nodes[index].bytes) = previous)
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

fn newest_descent<'a>(doc: &'a HistoryDoc, from: Option<&str>) -> Vec<&'a Node> {
    let mut tail: Vec<&Node> = Vec::new();
    let mut cursor = from.map(str::to_string);
    while let Some(child) = doc.nodes.iter().filter(|n| n.parent == cursor).max_by_key(|n| node_number(&n.id)) {
        if tail.len() == doc.nodes.len() {
            break;
        }
        cursor = Some(child.id.clone());
        tail.push(child);
    }
    tail
}

pub fn downstream_of(doc: &HistoryDoc, id: &str) -> Vec<Node> {
    let line = doc.head.as_deref().map(|head| doc.ancestry(head)).unwrap_or_default();
    let tail: Vec<&Node> = match line.iter().position(|n| n.id == id) {
        Some(at) => line[at + 1..].iter().copied().chain(newest_descent(doc, doc.head.as_deref())).collect(),
        None => newest_descent(doc, Some(id)),
    };
    tail.into_iter().cloned().collect()
}

pub fn replay_plan(doc: &HistoryDoc, target: &str) -> Result<ReplayPlan, HistoryError> {
    let path = doc.ancestry(target);
    if path.is_empty() {
        return Err(HistoryError::UnknownNode(target.to_string()));
    }
    let start = path.iter().rposition(|node| node.present);
    let from = start.map(|i| path[i].id.clone());
    let steps = path[start.map_or(0, |i| i + 1)..].iter().map(|node| node.id.clone()).collect();
    Ok(ReplayPlan { from, steps })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use std::time::Duration;

    pub(crate) const STAGES: [&str; 5] = ["background", "wavelet_denoise", "rl_deconv", "arcsinh", "tone"];
    pub(crate) const PNG_BYTES: usize = 1024;

    pub(crate) fn op(name: &str) -> OpSpec {
        let value = match name {
            "background" => serde_json::json!({"op": "background", "v": 1, "model": "polynomial",
                "config": {"gridSize": 8, "polyDegree": 3, "sigmaClip": 2.5, "iterations": 3, "mode": "subtract"}}),
            "wavelet_denoise" => serde_json::json!({"op": "wavelet_denoise", "v": 1, "numScales": 5,
                "thresholds": [3.0, 2.5, 2.0, 1.5, 1.0], "linearDenoise": true, "layerBias": null}),
            "rl_deconv" => serde_json::json!({"op": "rl_deconv", "v": 1, "iterations": 20, "regularization": 0.001,
                "deringing": true, "deringingThreshold": 0.1, "psf": {"kind": "gaussian", "sigma": 2.0, "size": 15}}),
            "arcsinh" => serde_json::json!({"op": "arcsinh", "v": 1, "factor": 50.0}),
            "tone" => serde_json::json!({"op": "tone", "v": 1, "levels": {"black": 0.1, "gamma": 1.0, "white": 0.9}}),
            "lhe" => serde_json::json!({"op": "lhe", "v": 1, "kernelRadius": 64, "contrastLimit": 2.0, "amount": 1.0,
                "histBits": 8, "circular": true}),
            other => panic!("no fixture op {other}"),
        };
        serde_json::from_value(value).unwrap()
    }

    pub(crate) fn write_bytes(path: &Path, bytes: usize, seconds_ago: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
        age_file(path, seconds_ago);
    }

    pub(crate) fn new_doc(image_key: &str) -> HistoryDoc {
        rebased_doc(image_key, None)
    }

    pub(crate) fn rebased_doc(image_key: &str, base_path: Option<&str>) -> HistoryDoc {
        let image = ImageInfo { source_ref: canonical_ref(image_key), id: image_id(image_key, base_path), size: 0, mtime: None, base: None };
        HistoryDoc::new(image, image_key, base_path)
    }

    pub(crate) fn age_file(path: &Path, seconds_ago: u64) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago)).unwrap();
    }

    pub(crate) fn record(dir: &Path, doc: &mut HistoryDoc, parent: Option<&str>, op_name: &str, node_bytes: usize) -> String {
        let id = next_node_id(dir, doc);
        let output = NodeOutput::named(&id, [64, 32], 1, OutputValues::Linear, &[]);
        let age = 100_000 - 10 * doc.nodes.len() as u64;
        write_bytes(&dir.join(&output.path), node_bytes, age);
        write_bytes(&dir.join(&output.png), PNG_BYTES, age);
        let node = NewNode { id: id.clone(), parent: parent.map(str::to_string), op: op(op_name), mask: None, output, elapsed_ms: 5, replayable: true };
        append(dir, doc, node).unwrap();
        id
    }

    pub(crate) fn chain(dir: &Path, doc: &mut HistoryDoc, from: Option<&str>, ops: &[&str], node_bytes: usize) -> Vec<String> {
        let mut parent = from.map(str::to_string);
        ops.iter()
            .map(|name| {
                let id = record(dir, doc, parent.as_deref(), name, node_bytes);
                parent = Some(id.clone());
                id
            })
            .collect()
    }

    pub(crate) fn three_branches(dir: &Path, node_bytes: usize) -> HistoryDoc {
        let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
        let first = chain(dir, &mut doc, None, &STAGES, node_bytes);
        chain(dir, &mut doc, Some(&first[0]), &STAGES[1..], node_bytes);
        chain(dir, &mut doc, Some(&first[0]), &STAGES[1..], node_bytes);
        doc
    }

    pub(crate) fn ids_of(paths: &[String]) -> Vec<String> {
        let mut ids: Vec<String> =
            paths.iter().filter(|p| p.ends_with(".fits")).map(|p| p.trim_end_matches(".fits").to_string()).collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use std::time::{Duration, Instant};

    const GOLDEN_V1: &str = include_str!("golden/history-v1.json");

    fn unix(text: &str) -> String {
        text.replace("\r\n", "\n")
    }

    #[test]
    fn golden_history_v1_round_trips() {
        let golden = unix(GOLDEN_V1);
        let doc: HistoryDoc = serde_json::from_str(&golden).unwrap();
        assert_eq!(doc.format, HISTORY_FORMAT);
        assert_eq!(doc.version, HISTORY_VERSION);
        assert_eq!(doc.read_only_reason(), None);
        assert_eq!(doc.nodes.len(), 5);
        assert_eq!(doc.head.as_deref(), Some("n0004"));
        assert_eq!(doc.nodes[3].mask, Some(MaskRef::Id("m01".to_string())));
        assert!(matches!(doc.nodes[4].mask, Some(MaskRef::Recipe(_))));
        assert_eq!(doc.nodes[0].output.extras, ["n0001_model.png"]);
        assert!(!doc.nodes[1].present);
        assert_eq!(serde_json::to_string_pretty(&doc).unwrap(), golden, "the v1 sidecar did not re-serialise byte-identically");
    }

    #[test]
    fn append_then_set_head_survive_a_crash_between_tmp_write_and_rename() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = new_doc("/data/m42.fits");
        let ids = chain(dir.path(), &mut doc, None, &["background", "wavelet_denoise"], 64);
        set_head(dir.path(), &mut doc, Some(&ids[0])).unwrap();

        let sidecar = sidecar_path(dir.path());
        let before = fs::read(&sidecar).unwrap();
        let temp = temp_path(&sidecar);
        fs::write(&temp, &before[..before.len() / 2]).unwrap();
        age_file(&temp, STALE_TEMP_AGE.as_secs() + 1);

        let loaded = load(dir.path()).unwrap().expect("the sidecar vanished after a crash");
        assert_eq!(fs::read(&sidecar).unwrap(), before, "the old sidecar was touched by the interrupted write");
        assert_eq!(loaded.nodes.len(), 2);
        assert_eq!(loaded.head.as_deref(), Some(ids[0].as_str()));
        assert!(!temp.exists(), "a stale temp sidecar was left behind");
        assert!(load(tempfile::tempdir().unwrap().path()).unwrap().is_none());
    }

    #[test]
    fn a_version_2_sidecar_opens_read_only_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let bumped = unix(GOLDEN_V1).replace("\"version\": 1", "\"version\": 2");
        fs::write(sidecar_path(dir.path()), &bumped).unwrap();

        let Err(HistoryError::ReadOnly(reason)) = load(dir.path()) else {
            panic!("a v2 sidecar was treated as writable");
        };
        assert!(reason.contains("version 2") && reason.contains("read-only"), "{reason}");

        let mut doc: HistoryDoc = serde_json::from_str(&bumped).unwrap();
        assert_eq!(doc.read_only_reason().as_deref(), Some(reason.as_str()));
        let err = set_head(dir.path(), &mut doc, Some("n0001")).unwrap_err();
        assert!(matches!(err, HistoryError::ReadOnly(_)), "{err}");
        let node = NewNode {
            id: "n0009".to_string(),
            parent: None,
            op: op("arcsinh"),
            mask: None,
            output: NodeOutput::named("n0009", [4, 4], 1, OutputValues::DisplayReferred, &[]),
            elapsed_ms: 1,
            replayable: true,
        };
        assert!(matches!(append(dir.path(), &mut doc, node), Err(HistoryError::ReadOnly(_))));
        assert_eq!(fs::read_to_string(sidecar_path(dir.path())).unwrap(), bumped, "a read-only sidecar was rewritten");

        fs::write(sidecar_path(dir.path()), b"{\"format\": \"something-else\"}").unwrap();
        let err = load(dir.path()).unwrap_err();
        assert!(matches!(err, HistoryError::Unreadable { .. }), "{err}");
    }

    #[test]
    fn a_sidecar_from_a_newer_build_opens_read_only_even_when_its_ops_do_not_parse() {
        let dir = tempfile::tempdir().unwrap();
        let mut newer: Value = serde_json::from_str(&unix(GOLDEN_V1)).unwrap();
        newer["version"] = 2.into();
        newer["nodes"][0]["op"]["v"] = 2.into();
        newer["nodes"][1]["op"]["op"] = "star_removal".into();
        let bytes = serde_json::to_vec_pretty(&newer).unwrap();
        fs::write(sidecar_path(dir.path()), &bytes).unwrap();

        match load(dir.path()) {
            Err(HistoryError::ReadOnly(reason)) => {
                assert!(reason.contains("version 2") && reason.contains("read-only"), "{reason}")
            }
            other => panic!("a version 2 sidecar did not open read-only: {other:?}"),
        }
        assert_eq!(fs::read(sidecar_path(dir.path())).unwrap(), bytes);

        fs::write(sidecar_path(dir.path()), &bytes[..bytes.len() / 2]).unwrap();
        assert!(matches!(load(dir.path()), Err(HistoryError::Unreadable { .. })), "a truncated sidecar was not reported unreadable");
    }

    #[test]
    fn mark_missing_and_save_leave_a_read_only_sidecar_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let bumped = unix(GOLDEN_V1).replace("\"version\": 1", "\"version\": 2");
        fs::write(sidecar_path(dir.path()), &bumped).unwrap();
        let mut doc: HistoryDoc = serde_json::from_str(&bumped).unwrap();
        let present_before = doc.nodes.iter().filter(|n| n.present).count();

        assert!(mark_missing(dir.path(), &mut doc).is_empty(), "eviction marks were recorded in a read-only history");
        assert_eq!(doc.nodes.iter().filter(|n| n.present).count(), present_before);
        assert!(matches!(save(dir.path(), &doc), Err(HistoryError::ReadOnly(_))), "a read-only history was saved");
        assert_eq!(fs::read_to_string(sidecar_path(dir.path())).unwrap(), bumped, "a read-only sidecar was rewritten");
    }

    #[test]
    fn concurrent_writers_never_share_a_temp_file_and_load_spares_one_in_flight() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = new_doc("/data/m42.fits");
        chain(dir.path(), &mut doc, None, &["background"], 64);
        let sidecar = sidecar_path(dir.path());
        let first = temp_path(&sidecar);
        let second = temp_path(&sidecar);
        assert_ne!(first, second, "two writers would rename each other's half-written sidecar");

        fs::write(&first, b"{\"half\":").unwrap();
        load(dir.path()).unwrap().unwrap();
        assert!(first.exists(), "load deleted the temp file of a write still in flight");

        write_bytes(&second, 8, 3600);
        load(dir.path()).unwrap().unwrap();
        assert!(!second.exists(), "an hour-old interrupted write was left behind");
        assert!(first.exists());
    }

    #[test]
    fn a_head_at_the_root_still_pins_the_redo_target() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = three_branches(dir.path(), 1024);
        set_head(dir.path(), &mut doc, None).unwrap();
        assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Full)), ["n0001"], "Revert to original left the redo target unpinned");
        assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::HeadAndParent)), ["n0001"]);
        assert_eq!(doc.redo_target().map(|n| n.id.as_str()), Some("n0001"));
        assert_eq!(ids_of(&active_paths(&doc)), ["n0001"]);

        set_head(dir.path(), &mut doc, Some("n0001")).unwrap();
        assert_eq!(doc.redo_target().map(|n| n.id.as_str()), Some("n0010"), "redo is not the most recent child");
        assert_eq!(ids_of(&active_paths(&doc)), ["n0001", "n0010"]);
    }

    #[test]
    fn a_rebased_history_never_releases_its_first_node() {
        let dir = tempfile::tempdir().unwrap();
        let key = "/data/m42.fits#hdu=0";
        let mut doc = rebased_doc(key, Some("/app/output/m42_calibrated.fits"));
        assert!(doc.is_rebased());
        chain(dir.path(), &mut doc, None, &STAGES, 1 << 20);
        let shrunk = pin_levels(&[(&doc, SystemTime::now())], 0);
        assert_eq!(ids_of(&pinned_paths(&doc, shrunk[0])), ["n0001", "n0004", "n0005"], "the only bridge from a wiped chain base was released");
        assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Floor)), ["n0001"]);

        let plain_dir = tempfile::tempdir().unwrap();
        let mut plain = new_doc(key);
        assert!(!plain.is_rebased());
        chain(plain_dir.path(), &mut plain, None, &STAGES, 1 << 20);
        assert_eq!(ids_of(&pinned_paths(&plain, shrunk[0])), ["n0004", "n0005"]);
        assert!(pinned_paths(&plain, PinLevel::Floor).is_empty());
    }

    #[test]
    fn node_paths_are_unique_and_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = new_doc("/data/m42.fits");
        let a = record(dir.path(), &mut doc, None, "arcsinh", 64);
        let b = record(dir.path(), &mut doc, None, "arcsinh", 64);
        assert_eq!((a.as_str(), b.as_str()), ("n0001", "n0002"));
        assert_ne!(doc.nodes[0].output.path, doc.nodes[1].output.path);

        write_bytes(&dir.path().join("n0003.fits"), 8, 1);
        assert_eq!(next_node_id(dir.path(), &doc), "n0004", "the allocator reused a file already on disk");
        write_bytes(&dir.path().join("n0004.png"), 8, 1);
        assert_eq!(next_node_id(dir.path(), &doc), "n0005");

        let duplicate = NewNode {
            id: a.clone(),
            parent: None,
            op: op("arcsinh"),
            mask: None,
            output: NodeOutput::named(&a, [64, 32], 1, OutputValues::DisplayReferred, &[]),
            elapsed_ms: 1,
            replayable: true,
        };
        assert!(matches!(append(dir.path(), &mut doc, duplicate), Err(HistoryError::NodeExists(_))));
        assert_eq!(doc.nodes.len(), 2);

        let orphan = NewNode {
            id: "n0010".to_string(),
            parent: Some("n0042".to_string()),
            op: op("arcsinh"),
            mask: None,
            output: NodeOutput::named("n0010", [64, 32], 1, OutputValues::DisplayReferred, &[]),
            elapsed_ms: 1,
            replayable: true,
        };
        assert!(matches!(append(dir.path(), &mut doc, orphan), Err(HistoryError::UnknownNode(_))));
        assert_eq!(load(dir.path()).unwrap().unwrap().nodes.len(), 2);
    }

    #[test]
    fn node_ids_are_never_handed_out_twice_after_a_branch_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
        chain(dir.path(), &mut doc, None, &["background", "wavelet_denoise", "arcsinh", "tone"], 64);
        for gone in remove_subtree(dir.path(), &mut doc, "n0003").unwrap() {
            for file in gone.output.files() {
                fs::remove_file(dir.path().join(file)).unwrap();
            }
        }
        assert_eq!(next_node_id(dir.path(), &doc), "n0005", "the id of a deleted step was handed out again");
        let reloaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(next_node_id(dir.path(), &reloaded), "n0005", "the issued ids were lost on reload");

        let golden: HistoryDoc = serde_json::from_str(&unix(GOLDEN_V1)).unwrap();
        assert_eq!(next_node_id(tempfile::tempdir().unwrap().path(), &golden), "n0006");
    }

    #[test]
    fn node_output_names_follow_the_node_id() {
        let out = NodeOutput::named("n0007", [2058, 2058], 1, OutputValues::Linear, &["model"]);
        assert_eq!((out.path.as_str(), out.png.as_str()), ("n0007.fits", "n0007.png"));
        assert_eq!(out.extras, ["n0007_model.png"]);
        assert_eq!(out.files().collect::<Vec<_>>(), ["n0007.fits", "n0007.png", "n0007_model.png"]);
    }

    #[test]
    fn node_paths_have_no_dot_prefixed_component() {
        let app_data = tempfile::tempdir().unwrap();
        let root = history_root(app_data.path());
        assert_eq!(root, app_data.path().join("history"));
        let dir = image_dir(&root, &image_id("/data/m42.fits#hdu=0", None));
        let mut doc = new_doc("/data/m42.fits#hdu=0");
        let id = record(&dir, &mut doc, None, "background", 64);
        let model = NodeOutput::named(&id, [4, 4], 1, OutputValues::Linear, &["model"]);
        let hash = kernels::write_kernel(&dir, &ndarray::Array2::<f32>::ones((3, 3))).unwrap();

        let mut produced = vec![root.clone(), dir.clone(), sidecar_path(&dir), kernels_dir(&dir), masks_dir(&dir)];
        produced.push(kernels::kernel_path(&dir, &hash).unwrap());
        produced.extend(model.files().map(|f| dir.join(f)));
        produced.extend(doc.nodes.iter().flat_map(|n| n.output.files().map(|f| dir.join(f)).collect::<Vec<_>>()));
        produced.extend(protected_paths(&doc, &dir, PinLevel::Full));
        produced.push(temp_path(&sidecar_path(&dir)));

        for path in &produced {
            let below = path.strip_prefix(app_data.path()).unwrap();
            for part in below.components() {
                let name = part.as_os_str().to_string_lossy();
                assert!(!name.starts_with('.'), "{} has the dot-prefixed component {name}", path.display());
            }
        }
    }

    #[test]
    fn fingerprint_matches_until_the_source_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("m42.fits");
        write_bytes(&source, 2880, 60);
        let key = format!("{}#hdu=0", source.display());
        let info = ImageInfo::of(&key, None).unwrap();
        assert_eq!(info.size, 2880);
        assert_eq!(info.id, image_id(&key, None));
        assert_eq!(info.source_ref, canonical_ref(&key));
        assert!(info.mtime.is_some());
        assert!(info.fingerprint_matches(&key, None));

        write_bytes(&source, 2880, 5);
        assert!(!info.fingerprint_matches(&key, None), "a rewritten source of the same size kept its fingerprint");
        write_bytes(&source, 5760, 60);
        assert!(!info.fingerprint_matches(&key, None));
        fs::remove_file(&source).unwrap();
        assert!(!info.fingerprint_matches(&key, None));
    }

    #[test]
    fn fingerprint_notices_a_rewritten_chain_base() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("m42_light.fits");
        let base = dir.path().join("output").join("m42_light_calibrated.fits");
        write_bytes(&source, 2880, 600);
        write_bytes(&base, 5760, 300);
        let key = format!("{}#hdu=0", source.display());
        let base_key = base.to_string_lossy().to_string();

        let info = ImageInfo::of(&key, Some(&base_key)).unwrap();
        let stamp = info.base.clone().expect("a rebased history did not stamp its chain base");
        assert_eq!((stamp.file_ref, stamp.size), (canonical_ref(&base_key), 5760));
        assert!(info.fingerprint_matches(&key, Some(&base_key)), "an untouched source and base lost their fingerprint");
        assert!(!info.fingerprint_matches(&key, None));

        write_bytes(&base, 5760, 5);
        assert!(!info.fingerprint_matches(&key, Some(&base_key)), "a recalibrated base of the same size kept the old history");
        fs::remove_file(&base).unwrap();
        assert!(!info.fingerprint_matches(&key, Some(&base_key)));

        let plain = ImageInfo::of(&key, None).unwrap();
        assert_eq!(plain.base, None);
        assert!(!serde_json::to_string(&plain).unwrap().contains("\"base\""), "a plain history serialised a base stamp");
        assert!(plain.fingerprint_matches(&key, Some(&key)));
    }

    #[test]
    fn estimate_next_node_bytes_is_four_bytes_per_sample_and_plane() {
        assert_eq!(estimate_next_node_bytes([14344, 8589], 1), 492_802_464);
        assert_eq!(estimate_next_node_bytes([2058, 2058], 1), 16_941_456);
        assert_eq!(estimate_next_node_bytes([7065, 4177], 3), 354_126_060);
    }

    #[test]
    fn pinned_set_keeps_head_parent_root_and_every_fourth_node() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = three_branches(dir.path(), 1024);
        assert_eq!(doc.nodes.len(), 13);
        assert_eq!(doc.head.as_deref(), Some("n0013"));
        assert_eq!(doc.nodes.iter().filter(|n| n.checkpoint).map(|n| n.id.as_str()).collect::<Vec<_>>(), ["n0004", "n0008", "n0012"]);
        assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Full)), ["n0001", "n0012", "n0013"]);

        set_head(dir.path(), &mut doc, Some("n0011")).unwrap();
        assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Full)), ["n0001", "n0010", "n0011"]);

        let long = tempfile::tempdir().unwrap();
        let mut seven = new_doc("/data/long.fits");
        chain(long.path(), &mut seven, None, &["background", "wavelet_denoise", "rl_deconv", "lhe", "arcsinh", "tone", "tone"], 64);
        let pinned = pinned_paths(&seven, PinLevel::Full);
        assert_eq!(ids_of(&pinned), ["n0001", "n0004", "n0006", "n0007"]);
        assert!(pinned.contains(&"n0004.png".to_string()), "{pinned:?}");
    }

    #[test]
    fn pinned_set_always_keeps_nodes_that_cannot_be_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = three_branches(dir.path(), 1 << 20);
        doc.nodes[2].replayable = false;
        for level in [PinLevel::Full, PinLevel::HeadAndParent, PinLevel::Floor] {
            assert!(ids_of(&pinned_paths(&doc, level)).contains(&"n0003".to_string()), "{level:?} released a step replay cannot rebuild");
        }
        assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Floor)), ["n0003"]);
        let shrunk = pin_levels(&[(&doc, SystemTime::now())], 0);
        assert_eq!(ids_of(&pinned_paths(&doc, shrunk[0])), ["n0003", "n0012", "n0013"]);

        let text = serde_json::to_value(&doc).unwrap();
        assert_eq!(text["nodes"][2]["replayable"], false);
        assert!(text["nodes"][0].get("replayable").is_none(), "a replayable node serialised the flag: {}", text["nodes"][0]);
        let back: HistoryDoc = serde_json::from_value(text).unwrap();
        assert!(!back.nodes[2].replayable && back.nodes[0].replayable);
    }

    #[test]
    fn pinned_set_shrinks_to_head_and_parent_when_it_exceeds_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let doc = three_branches(dir.path(), 1 << 20);
        let full = 3 * ((1 << 20) + PNG_BYTES as u64);
        let now = SystemTime::now();
        assert_eq!(pin_levels(&[(&doc, now)], full), [PinLevel::Full]);
        assert_eq!(pin_levels(&[(&doc, now)], full - 1), [PinLevel::HeadAndParent]);
        assert_eq!(pin_levels(&[(&doc, now)], 0), [PinLevel::HeadAndParent], "the only history lost its current step");
        assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::HeadAndParent)), ["n0012", "n0013"]);
        assert!(pinned_paths(&doc, PinLevel::Floor).is_empty());
        assert_eq!(doc.nodes[0].bytes, (1 << 20) + PNG_BYTES as u64);
    }

    #[test]
    fn pin_levels_release_the_oldest_images_first_and_keep_the_newest_head() {
        let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
        let docs: Vec<HistoryDoc> = dirs
            .iter()
            .zip(["/data/a.fits", "/data/b.fits", "/data/c.fits"])
            .map(|(dir, key)| {
                let mut doc = new_doc(key);
                chain(dir.path(), &mut doc, None, &STAGES, 1 << 20);
                doc
            })
            .collect();
        let now = SystemTime::now();
        let used = [now - Duration::from_secs(300), now - Duration::from_secs(200), now - Duration::from_secs(100)];
        let tracked: Vec<(&HistoryDoc, SystemTime)> = docs.iter().zip(used).collect();
        let shuffled = [tracked[2], tracked[0], tracked[1]];
        use PinLevel::{Floor, Full, HeadAndParent};

        assert_eq!(pin_levels(&tracked, u64::MAX), [Full, Full, Full]);
        assert_eq!(pin_levels(&tracked, 8 << 20), [HeadAndParent, HeadAndParent, Full]);
        assert_eq!(pin_levels(&tracked, 4 << 20), [Floor, Floor, HeadAndParent]);
        assert_eq!(pin_levels(&tracked, 0), [Floor, Floor, HeadAndParent]);
        assert_eq!(pin_levels(&shuffled, 4 << 20), [HeadAndParent, Floor, Floor], "release followed list order, not last use");
        assert!(pin_levels(&[], 0).is_empty());
    }

    #[test]
    fn protected_paths_cover_the_sidecar_and_side_files() {
        let dir = tempfile::tempdir().unwrap();
        let doc = three_branches(dir.path(), 64);
        let protected = protected_paths(&doc, dir.path(), PinLevel::Full);
        for expected in [sidecar_path(dir.path()), kernels_dir(dir.path()), masks_dir(dir.path())] {
            assert!(protected.contains(&expected), "{} is not protected", expected.display());
        }
        assert!(protected.contains(&dir.path().join("n0013.fits")));
        assert!(!protected.contains(&dir.path().join("n0002.fits")));
    }

    #[test]
    fn replay_plan_starts_at_the_nearest_present_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = new_doc("/data/m42.fits");
        let ids = chain(dir.path(), &mut doc, None, &["background", "wavelet_denoise", "rl_deconv", "arcsinh"], 64);
        for id in &ids[1..] {
            fs::remove_file(dir.path().join(format!("{id}.fits"))).unwrap();
        }
        let missing = mark_missing(dir.path(), &mut doc);
        assert_eq!(missing, ["n0002", "n0003", "n0004"]);
        assert_eq!(load(dir.path()).unwrap().unwrap().nodes.iter().filter(|n| !n.present).count(), 3, "eviction marks were not saved");
        assert!(mark_missing(dir.path(), &mut doc).is_empty());

        let plan = replay_plan(&doc, "n0004").unwrap();
        assert_eq!(plan, ReplayPlan { from: Some("n0001".to_string()), steps: vec!["n0002".into(), "n0003".into(), "n0004".into()] });
        assert_eq!(replay_plan(&doc, "n0001").unwrap(), ReplayPlan { from: Some("n0001".to_string()), steps: vec![] });

        fs::remove_file(dir.path().join("n0001.png")).unwrap();
        mark_missing(dir.path(), &mut doc);
        assert_eq!(replay_plan(&doc, "n0002").unwrap(), ReplayPlan { from: None, steps: vec!["n0001".into(), "n0002".into()] });
        assert!(matches!(replay_plan(&doc, "n0042"), Err(HistoryError::UnknownNode(_))));
    }

    #[test]
    #[ignore = "timing: checked locally in A10, three CI runners make the bound flaky"]
    fn recorder_overhead_under_15_ms_for_a_200_node_doc() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
        let mut parent: Option<String> = None;
        for i in 0..199 {
            let id = format!("n{:04}", i + 1);
            let output = NodeOutput::named(&id, [2058, 2058], 1, OutputValues::Linear, &[]);
            doc.nodes.push(Node {
                id: id.clone(),
                parent: parent.clone(),
                op: op(STAGES[i % STAGES.len()]),
                mask: None,
                output,
                at: utc_timestamp(SystemTime::now()),
                elapsed_ms: 812,
                app: APP_VERSION.to_string(),
                threads: 16,
                present: true,
                bytes: 16_941_456,
                checkpoint: false,
                replayable: true,
            });
            parent = if i % 7 == 6 { None } else { Some(id) };
        }
        save(dir.path(), &doc).unwrap();
        let id = next_node_id(dir.path(), &doc);
        let output = NodeOutput::named(&id, [2058, 2058], 1, OutputValues::Linear, &[]);
        write_bytes(&dir.path().join(&output.path), 64, 1);
        write_bytes(&dir.path().join(&output.png), 64, 1);
        let node = NewNode { id, parent, op: op("tone"), mask: None, output, elapsed_ms: 3, replayable: true };

        let started = Instant::now();
        append(dir.path(), &mut doc, node).unwrap();
        let elapsed = started.elapsed();
        let written = fs::metadata(sidecar_path(dir.path())).unwrap().len();
        println!("recorder: 200-node append {:.2} ms, sidecar {written} bytes", elapsed.as_secs_f64() * 1000.0);
        assert!(elapsed < Duration::from_millis(15), "append took {elapsed:?}");
        assert!(written <= 100 * 1024, "the sidecar rewrite is {written} bytes");
    }

    fn downstream_ids(doc: &HistoryDoc, id: &str) -> Vec<String> {
        downstream_of(doc, id).into_iter().map(|n| n.id).collect()
    }

    #[test]
    fn downstream_of_follows_the_line_the_history_tool_shows() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
        let first = chain(dir.path(), &mut doc, None, &["background", "wavelet_denoise", "arcsinh", "tone"], 64);
        assert_eq!(downstream_ids(&doc, "n0002"), ["n0003", "n0004"]);

        set_head(dir.path(), &mut doc, Some("n0002")).unwrap();
        assert_eq!(downstream_ids(&doc, "n0002"), ["n0003", "n0004"], "re-run downstream on the head dropped its undone steps");
        set_head(dir.path(), &mut doc, Some("n0003")).unwrap();
        assert_eq!(downstream_ids(&doc, "n0002"), ["n0003", "n0004"], "a step undone after the head was dropped");
        set_head(dir.path(), &mut doc, None).unwrap();
        assert_eq!(downstream_ids(&doc, "n0001"), ["n0002", "n0003", "n0004"]);

        chain(dir.path(), &mut doc, Some(&first[0]), &["wavelet_denoise", "arcsinh"], 64);
        assert_eq!(doc.head.as_deref(), Some("n0006"));
        assert_eq!(downstream_ids(&doc, "n0001"), ["n0005", "n0006"]);
        assert_eq!(downstream_ids(&doc, "n0002"), ["n0003", "n0004"], "a step off the head path did not follow its newest descent");
        assert!(downstream_ids(&doc, "n0006").is_empty());
        assert!(downstream_ids(&doc, "n0042").is_empty());
    }

    #[test]
    fn remove_subtree_drops_descendants_and_reparents_the_head() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
        let first = chain(dir.path(), &mut doc, None, &STAGES[..3], 64);
        let side = chain(dir.path(), &mut doc, Some(&first[0]), &STAGES[1..3], 64);
        assert_eq!(doc.head.as_deref(), Some(side[1].as_str()));

        let removed: Vec<String> = remove_subtree(dir.path(), &mut doc, &first[1]).unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(removed, [first[1].clone(), first[2].clone()]);
        assert_eq!(doc.head.as_deref(), Some(side[1].as_str()), "removing another branch moved the head");
        assert_eq!(load(dir.path()).unwrap().unwrap().nodes.len(), 3);

        let removed: Vec<String> = remove_subtree(dir.path(), &mut doc, &side[0]).unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(removed, side);
        assert_eq!(doc.head.as_deref(), Some(first[0].as_str()), "a head inside the removed branch did not move to its parent");

        remove_subtree(dir.path(), &mut doc, &first[0]).unwrap();
        assert!(doc.nodes.is_empty());
        assert_eq!(doc.head, None);
        assert!(matches!(remove_subtree(dir.path(), &mut doc, "n0042"), Err(HistoryError::UnknownNode(id)) if id == "n0042"));
        assert_eq!(load(dir.path()).unwrap().unwrap().head, None);
    }
}
