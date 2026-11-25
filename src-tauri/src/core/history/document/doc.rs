use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use ndarray::Array2;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::mask::{check_dims, MaskError, MaskSpec};
use crate::core::ops::{OpSpec, OutputValues};
use crate::types::image_ref::ImageRef;

use super::error::HistoryError;
use super::id::{canonical_ref, image_id, rebase_ref};
use super::stamp::{file_sha256, file_stamp, utc_timestamp};

pub const HISTORY_FORMAT: &str = "astroburst-history";
pub const HISTORY_VERSION: u32 = 1;
pub const DEFAULT_CHECKPOINT_EVERY: u32 = 4;
pub const HEADER_ABHIST: &str = "ABHIST";
pub const HEADER_ABNODE: &str = "ABNODE";
const NODE_PREFIX: &str = "n";
pub(in crate::core::history) const FITS_EXTENSION: &str = "fits";
pub(in crate::core::history) const PNG_EXTENSION: &str = "png";
pub(in crate::core::history) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
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

impl MaskRef {
    pub fn label(&self) -> &str {
        match self {
            Self::Id(id) => id,
            Self::Recipe(recipe) => recipe.get("kind").and_then(Value::as_str).unwrap_or("inline"),
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebuilt_app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebuilt_at: Option<String>,
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
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub deleted_masks: BTreeSet<String>,
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
        let sha256 = Some(file_sha256(Path::new(&ImageRef::parse(image_key).path))?);
        Ok(Self { source_ref: canonical_ref(image_key), id: image_id(image_key, base_path), size, mtime, base, sha256 })
    }

    pub fn content_matches(&self, image_key: &str) -> bool {
        self.sha256.as_ref().is_none_or(|recorded| {
            file_sha256(Path::new(&ImageRef::parse(image_key).path)).is_ok_and(|current| &current == recorded)
        })
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

pub(in crate::core::history) fn node_id(number: u32) -> String {
    format!("{NODE_PREFIX}{number:04}")
}

pub(in crate::core::history) fn node_number(id: &str) -> Option<u32> {
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
            deleted_masks: BTreeSet::new(),
            nodes: Vec::new(),
        }
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn mask_spec(&self, mask: &MaskRef) -> Result<MaskSpec, MaskError> {
        let (label, stored) = match mask {
            MaskRef::Id(id) => (format!("mask {id}"), self.masks.get(id).ok_or_else(|| MaskError::Unknown(id.clone()))?),
            MaskRef::Recipe(recipe) => ("inline mask".to_string(), recipe),
        };
        MaskSpec::deserialize(stored).map_err(|e| MaskError::Invalid(format!("{label}: {e}")))
    }

    pub fn listed_mask_spec(&self, mask: &MaskRef) -> Result<MaskSpec, MaskError> {
        match mask {
            MaskRef::Id(id) if self.deleted_masks.contains(id) => Err(MaskError::Deleted(id.clone())),
            _ => self.mask_spec(mask),
        }
    }

    pub fn mask_raster(&self, mask: &MaskRef, planes: &[&Array2<f32>]) -> Result<Array2<f32>, MaskError> {
        let raster = self.mask_spec(mask)?.rasterize(planes)?;
        let image = planes.first().map_or((0, 0), |plane| plane.dim());
        check_dims(mask.label(), raster.dim(), image)?;
        Ok(raster)
    }

    pub fn read_only_reason(&self) -> Option<String> {
        read_only_reason(self.version)
    }

    pub(in crate::core::history) fn ensure_writable(&self) -> Result<(), HistoryError> {
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

pub(in crate::core::history) fn read_only_reason(version: u32) -> Option<String> {
    (version != HISTORY_VERSION).then(|| {
        format!("this history was written with history format version {version}; this build writes version {HISTORY_VERSION}, so it opens read-only")
    })
}
