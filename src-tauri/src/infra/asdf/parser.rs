use std::borrow::Cow;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use memmap2::Mmap;
use serde_yaml::Value;

use super::blocks::{BlockHeader, BlockRef, Compression};
use super::tree::STREAMED_BLOCK;
use crate::core::stacking::memory::{format_bytes, MEMORY_BUDGET_ENV};

const ASDF_MAGIC: &[u8] = b"#ASDF";
const BLOCK_INDEX_MAGIC: &[u8] = b"#ASDF BLOCK INDEX";
const STREAMED_FLAG: u32 = 0x1;
const FORMAT_VERSION_PREFIX: &str = "#ASDF ";
const STANDARD_VERSION_PREFIX: &str = "#ASDF_STANDARD ";
const KNOWN_MAJOR_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOrigin {
    pub path: PathBuf,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub tree_digest: u64,
}

enum Backing {
    Mapped(Mmap),
    Owned(Vec<u8>),
}

impl Backing {
    fn as_slice(&self) -> &[u8] {
        match self {
            Backing::Mapped(m) => &m[..],
            Backing::Owned(v) => v.as_slice(),
        }
    }
}

impl std::fmt::Debug for Backing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Backing({} bytes)", self.as_slice().len())
    }
}

#[derive(Debug)]
pub struct AsdfFile {
    pub tree: Value,
    pub blocks: Vec<BlockRef>,
    backing: Backing,
    origin: Option<FileOrigin>,
}

impl AsdfFile {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, AsdfError> {
        let file = File::open(path.as_ref())?;
        let metadata = file.metadata()?;
        if metadata.len() == 0 {
            return Err(AsdfError::InvalidMagic);
        }
        let mmap = unsafe { Mmap::map(&file)? };
        let mut parsed = Self::parse(Backing::Mapped(mmap))?;
        parsed.origin = Some(FileOrigin {
            path: path.as_ref().to_path_buf(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
            tree_digest: parsed.tree_digest(),
        });
        Ok(parsed)
    }

    pub fn origin(&self) -> Option<&FileOrigin> {
        self.origin.as_ref()
    }

    fn tree_digest(&self) -> u64 {
        let bytes = self.backing.as_slice();
        let (yaml_end, _) = find_tree_end(bytes);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes[..yaml_end].hash(&mut hasher);
        hasher.finish()
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, AsdfError> {
        Self::parse(Backing::Owned(bytes))
    }

    fn parse(backing: Backing) -> Result<Self, AsdfError> {
        let bytes = backing.as_slice();
        if !bytes.starts_with(ASDF_MAGIC) {
            return Err(AsdfError::InvalidMagic);
        }
        let (yaml_end, blocks_start) = find_tree_end(bytes);
        let text = std::str::from_utf8(&bytes[..yaml_end])
            .map_err(|e| AsdfError::YamlParse(e.to_string()))?;
        check_versions(text)?;
        let tree = parse_tree(text)?;
        let blocks = read_blocks(bytes, blocks_start)?;
        Ok(Self {
            tree,
            blocks,
            backing,
            origin: None,
        })
    }

    fn block(&self, index: usize) -> Result<&BlockRef, AsdfError> {
        if index != STREAMED_BLOCK {
            return self.blocks.get(index).ok_or(AsdfError::BlockOutOfRange(index));
        }
        self.blocks
            .last()
            .filter(|block| block.header.flags & STREAMED_FLAG != 0)
            .ok_or(AsdfError::BlockOutOfRange(self.blocks.len()))
    }

    pub fn block_data(&self, index: usize) -> Result<Cow<'_, [u8]>, AsdfError> {
        let block = self.block(index)?;
        let raw = block
            .data_start
            .checked_add(block.used_size)
            .and_then(|end| self.backing.as_slice().get(block.data_start..end))
            .ok_or(AsdfError::BlockTruncated)?;
        block.header.decompress(raw)
    }

    pub fn block_len(&self, index: usize) -> Result<usize, AsdfError> {
        let block = self.block(index)?;
        if block.header.compression == Compression::None {
            return Ok(block.used_size);
        }
        usize::try_from(block.header.data_size)
            .map_err(|_| AsdfError::ShapeOverflow(format!("block declares data_size {}", block.header.data_size)))
    }

    pub fn block_is_compressed(&self, index: usize) -> bool {
        self.block(index).is_ok_and(|block| block.header.compression != Compression::None)
    }
}

fn major_version(line: &str, prefix: &str) -> Option<u32> {
    line.strip_prefix(prefix)?.trim().split('.').next()?.parse().ok()
}

fn check_versions(text: &str) -> Result<(), AsdfError> {
    let head = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .take_while(|line| line.starts_with('#') || line.starts_with('%'));
    for line in head {
        let major = major_version(line, STANDARD_VERSION_PREFIX).or_else(|| major_version(line, FORMAT_VERSION_PREFIX));
        if major.is_some_and(|major| major > KNOWN_MAJOR_VERSION) {
            return Err(AsdfError::UnsupportedVersion(format!(
                "{} (this reader knows major version {KNOWN_MAJOR_VERSION})",
                line.trim()
            )));
        }
    }
    Ok(())
}

fn find_tree_end(bytes: &[u8]) -> (usize, usize) {
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i] == b'\n' && &bytes[i + 1..i + 4] == b"..." {
            let after = i + 4;
            if after == bytes.len() {
                return (i + 1, after);
            }
            if bytes[after] == b'\n' {
                return (i + 1, after + 1);
            }
            if bytes[after] == b'\r' && after + 1 < bytes.len() && bytes[after + 1] == b'\n' {
                return (i + 1, after + 2);
            }
            if bytes[after] == b'\r' && after + 1 == bytes.len() {
                return (i + 1, after + 1);
            }
        }
        i += 1;
    }
    let first_block = find_bytes(bytes, BlockHeader::MAGIC, 0).unwrap_or(bytes.len());
    (first_block, first_block)
}

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

pub(crate) fn parse_tree(text: &str) -> Result<Value, AsdfError> {
    let lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    let mut yaml_content = String::new();
    let mut in_document = false;

    for line in lines.skip(1) {
        if line.starts_with("---") {
            in_document = true;
            continue;
        }
        if line == "..." {
            break;
        }
        if line.starts_with("%YAML") || line.starts_with("%TAG") || line.starts_with('#') {
            continue;
        }
        if in_document {
            yaml_content.push_str(line);
            yaml_content.push('\n');
        }
    }

    if yaml_content.trim().is_empty() {
        return Err(AsdfError::NoYamlTree);
    }

    let yaml_content = shorten_verbatim_tags(&yaml_content);
    serde_yaml::from_str(&yaml_content).map_err(|e| AsdfError::YamlParse(e.to_string()))
}

const VERBATIM_TAG_PREFIXES: [&str; 4] = [
    "tag:stsci.edu:gwcs/",
    "tag:astropy.org:",
    "tag:stsci.edu:jwst_pipeline/",
    "asdf://",
];

fn verbatim_tag_is_shortenable(uri: &str) -> bool {
    VERBATIM_TAG_PREFIXES.iter().any(|p| uri.starts_with(p))
        && !uri.contains(|c: char| c.is_whitespace() || "{}[],".contains(c))
}

fn shorten_verbatim_tags(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find("!<") {
        let start = cursor + rel;
        let body = start + 2;
        let in_tag_position =
            start == 0 || matches!(bytes[start - 1], b' ' | b'\t' | b'\n' | b'\r' | b'[' | b'{' | b',');
        match text[body..].find('>') {
            Some(len) if in_tag_position && verbatim_tag_is_shortenable(&text[body..body + len]) => {
                out.push_str(&text[copied..start]);
                out.push('!');
                out.push_str(&text[body..body + len]);
                copied = body + len + 1;
                cursor = copied;
            }
            _ => cursor = body,
        }
    }
    out.push_str(&text[copied..]);
    out
}

fn read_blocks(buf: &[u8], start: usize) -> Result<Vec<BlockRef>, AsdfError> {
    let mut blocks = Vec::new();
    let mut offset = start;
    while offset < buf.len() {
        offset = skip_padding(buf, offset);
        if offset + 4 > buf.len() {
            break;
        }
        if buf[offset..].starts_with(BLOCK_INDEX_MAGIC) {
            break;
        }
        if &buf[offset..offset + 4] != BlockHeader::MAGIC {
            if blocks.is_empty() {
                offset += 1;
                continue;
            }
            break;
        }

        let (header, header_end) = BlockHeader::parse(&buf[offset..])?;
        let data_start = offset + header_end;

        if header.flags & STREAMED_FLAG != 0 {
            let used = buf.len() - data_start;
            blocks.push(BlockRef {
                header,
                data_start,
                used_size: used,
            });
            break;
        }

        let data_end = data_start
            .checked_add(header.allocated_size as usize)
            .ok_or(AsdfError::BlockTruncated)?;
        if data_end > buf.len() || header.used_size > header.allocated_size {
            return Err(AsdfError::BlockTruncated);
        }
        let used_size = header.used_size as usize;
        blocks.push(BlockRef {
            header,
            data_start,
            used_size,
        });
        offset = data_end;
    }
    Ok(blocks)
}

fn skip_padding(buf: &[u8], mut offset: usize) -> usize {
    while offset < buf.len() && (buf[offset] == 0 || buf[offset] == b' ' || buf[offset] == b'\n' || buf[offset] == b'\r') {
        offset += 1;
    }
    offset
}

#[derive(Debug)]
pub enum AsdfError {
    Io(std::io::Error),
    InvalidMagic,
    NoYamlTree,
    YamlParse(String),
    InvalidBlockHeader,
    BlockTruncated,
    UnsupportedCompression(String),
    DecompressionFailed(String),
    InvalidDtype(String),
    MissingField(String),
    BlockOutOfRange(usize),
    ExternalBlock(String),
    ShapeMismatch { got: usize, expected: usize },
    TooLarge { what: String, needed: u64, available: u64 },
    ShapeOverflow(String),
    InvalidStrides(String),
    UnsupportedDtype(String),
    UnsupportedVersion(String),
}

impl From<std::io::Error> for AsdfError {
    fn from(e: std::io::Error) -> Self {
        AsdfError::Io(e)
    }
}

impl std::fmt::Display for AsdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsdfError::Io(e) => write!(f, "IO error: {}", e),
            AsdfError::InvalidMagic => write!(f, "Not a valid ASDF file"),
            AsdfError::NoYamlTree => write!(f, "No YAML tree found"),
            AsdfError::YamlParse(e) => write!(f, "YAML parse error: {}", e),
            AsdfError::InvalidBlockHeader => write!(f, "Invalid block header"),
            AsdfError::BlockTruncated => write!(f, "Block data truncated"),
            AsdfError::UnsupportedCompression(c) => write!(f, "Unsupported compression: {}", c),
            AsdfError::DecompressionFailed(e) => write!(f, "Decompression failed: {}", e),
            AsdfError::InvalidDtype(d) => write!(f, "Invalid dtype: {}", d),
            AsdfError::MissingField(field) => write!(f, "Missing field: {}", field),
            AsdfError::BlockOutOfRange(i) => write!(f, "Block index out of range: {}", i),
            AsdfError::ExternalBlock(uri) => write!(
                f,
                "External (exploded) ASDF blocks are not supported: {}",
                uri
            ),
            AsdfError::ShapeMismatch { got, expected } => {
                write!(
                    f,
                    "Array element count mismatch: got {} expected {}",
                    got, expected
                )
            }
            AsdfError::TooLarge { what, needed, available } => write!(
                f,
                "{} needs about {} of memory but only {} is available. If the system can page this much, set {} to the memory to allow (in MB) and restart.",
                what,
                format_bytes(*needed),
                format_bytes(*available),
                MEMORY_BUDGET_ENV
            ),
            AsdfError::ShapeOverflow(what) => {
                write!(f, "Array size overflows the addressable memory: {}", what)
            }
            AsdfError::InvalidStrides(reason) => write!(f, "Invalid strides: {}", reason),
            AsdfError::UnsupportedDtype(d) => write!(f, "Unsupported datatype: {}", d),
            AsdfError::UnsupportedVersion(v) => write!(f, "Unsupported version: {}", v),
        }
    }
}

impl std::error::Error for AsdfError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_of(v: Option<&Value>) -> Option<String> {
        match v? {
            Value::Tagged(t) => Some(t.tag.to_string()),
            _ => None,
        }
    }

    const REAL_SYNTAX: &str = r##"#ASDF 1.0.0
#ASDF_STANDARD 1.6.0
%YAML 1.1
%TAG ! tag:stsci.edu:asdf/
--- !core/asdf-1.1.0
asdf_library: !core/software-1.0.0 {author: The ASDF Developers, name: asdf, version: 5.3.1}
meta:
  description: "!<tag:stsci.edu:gwcs/not-a-tag-1.0.0>"
  flow: {k: !<tag:stsci.edu:gwcs/frame2d-1.2.0> {name: x}, m: [!<tag:stsci.edu:gwcs/step-1.3.0> {q: 1}]}
  wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>
    name: ''
    pixel_shape: null
    steps:
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0>
        axes_names: [x, y]
        axes_order: [0, 1]
        axis_physical_types: ['custom:x', 'custom:y']
        name: detector
        unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]
      transform: !transform/compose-1.4.0
        forward:
        - !transform/concatenate-1.4.0
          forward:
          - &id001 !transform/shift-1.4.0
            inputs: [x]
            offset: 1.0
            outputs: [y]
          - *id001
          inputs: [x0, x1]
          outputs: [y0, y1]
        - !<tag:stsci.edu:gwcs/spherical_cartesian-1.3.0>
          inputs: [lon, lat]
          outputs: [x, y, z]
          transform_type: spherical_to_cartesian
          wrap_lon_at: 180
        inputs: [x0, x1]
        outputs: [x, y, z]
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>
        axes_names: [lon, lat]
        axes_order: [0, 1]
        axis_physical_types: [pos.eq.ra, pos.eq.dec]
        name: world
        reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0>
          frame_attributes: {}
        unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]
      transform: null
...
"##;

    #[test]
    fn a_future_major_version_of_the_format_or_the_standard_is_refused_by_name() {
        let body = "--- !core/asdf-1.1.0\nmeta: {a: 1}\n...\n";
        let open = |head: &str| AsdfFile::from_bytes(format!("{head}{body}").into_bytes());
        assert!(open("#ASDF 1.0.0\n#ASDF_STANDARD 1.6.0\n%YAML 1.1\n").is_ok());
        assert!(open("#ASDF 1.0.0\n%YAML 1.1\n").is_ok(), "the standard line is optional");
        match open("#ASDF 2.0.0\n#ASDF_STANDARD 1.6.0\n%YAML 1.1\n") {
            Err(AsdfError::UnsupportedVersion(what)) => assert!(what.contains("#ASDF 2.0.0"), "{what}"),
            other => panic!("expected a version refusal, got {:?}", other.map(|f| f.blocks.len())),
        }
        match open("#ASDF 1.0.0\n#ASDF_STANDARD 2.1.0\n%YAML 1.1\n") {
            Err(AsdfError::UnsupportedVersion(what)) => assert!(what.contains("#ASDF_STANDARD 2.1.0"), "{what}"),
            other => panic!("expected a version refusal, got {:?}", other.map(|f| f.blocks.len())),
        }
    }

    #[test]
    fn verbatim_tags_survive_parse_tree() {
        let tree = parse_tree(REAL_SYNTAX).expect("parse");
        let meta = tree.get("meta").expect("meta");
        let wcs = meta.get("wcs").expect("wcs");
        assert_eq!(tag_of(Some(wcs)).as_deref(), Some("!tag:stsci.edu:gwcs/wcs-1.4.0"));
        let steps = wcs.get("steps").and_then(Value::as_sequence).expect("steps");
        assert_eq!(tag_of(steps.first()).as_deref(), Some("!tag:stsci.edu:gwcs/step-1.3.0"));
        assert_eq!(
            tag_of(steps[0].get("frame")).as_deref(),
            Some("!tag:stsci.edu:gwcs/frame2d-1.2.0")
        );
        let forward = steps[0]
            .get("transform")
            .and_then(|t| t.get("forward"))
            .and_then(Value::as_sequence)
            .expect("forward");
        assert_eq!(
            tag_of(forward.get(1)).as_deref(),
            Some("!tag:stsci.edu:gwcs/spherical_cartesian-1.3.0")
        );
        let shifts = forward[0].get("forward").and_then(Value::as_sequence).expect("shifts");
        assert_eq!(tag_of(shifts.get(0)).as_deref(), Some("!transform/shift-1.4.0"));
        assert_eq!(tag_of(shifts.get(1)).as_deref(), Some("!transform/shift-1.4.0"));
        assert_eq!(shifts[1].get("offset").and_then(Value::as_f64), Some(1.0));
        assert_eq!(
            tag_of(steps[1].get("frame")).as_deref(),
            Some("!tag:stsci.edu:gwcs/celestial_frame-1.2.0")
        );
        assert_eq!(
            tag_of(steps[1].get("frame").and_then(|f| f.get("reference_frame"))).as_deref(),
            Some("!tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0")
        );
        assert_eq!(
            meta.get("description").and_then(Value::as_str),
            Some("!<tag:stsci.edu:gwcs/not-a-tag-1.0.0>")
        );
        let flow = meta.get("flow").expect("flow");
        assert_eq!(tag_of(flow.get("k")).as_deref(), Some("!tag:stsci.edu:gwcs/frame2d-1.2.0"));
        assert_eq!(
            tag_of(flow.get("m").and_then(|m| m.get(0))).as_deref(),
            Some("!tag:stsci.edu:gwcs/step-1.3.0")
        );
        let spaced = "x: !<tag:stsci.edu:gwcs/x y-1.0.0>\n  a: 1\n";
        assert_eq!(shorten_verbatim_tags(spaced), spaced);
        let quoted = "d: \"!<tag:stsci.edu:gwcs/q-1.0.0>\"\n";
        assert_eq!(shorten_verbatim_tags(quoted), quoted);
        let other = "o: !<tag:example.org:thing-1.0.0>\n  a: 1\n";
        assert_eq!(shorten_verbatim_tags(other), other);
        assert_eq!(
            shorten_verbatim_tags("r: !<asdf://stsci.edu/datamodels/roman/tags/wfi_wcs-2.0.0>\n  a: 1\n"),
            "r: !asdf://stsci.edu/datamodels/roman/tags/wfi_wcs-2.0.0\n  a: 1\n"
        );
    }
}
