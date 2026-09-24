use std::borrow::Cow;
use std::fs::File;
use std::path::Path;

use memmap2::Mmap;
use serde_yaml::Value;

use super::blocks::{BlockHeader, BlockRef};

const ASDF_MAGIC: &[u8] = b"#ASDF";
const BLOCK_INDEX_MAGIC: &[u8] = b"#ASDF BLOCK INDEX";
const STREAMED_FLAG: u32 = 0x1;

enum Backing {
    Mapped(Mmap),
    #[cfg(test)]
    Owned(Vec<u8>),
}

impl Backing {
    fn as_slice(&self) -> &[u8] {
        match self {
            Backing::Mapped(m) => &m[..],
            #[cfg(test)]
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
}

impl AsdfFile {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, AsdfError> {
        let file = File::open(path.as_ref())?;
        if file.metadata()?.len() == 0 {
            return Err(AsdfError::InvalidMagic);
        }
        let mmap = unsafe { Mmap::map(&file)? };
        Self::parse(Backing::Mapped(mmap))
    }

    #[cfg(test)]
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
        let tree = parse_tree(text)?;
        let blocks = read_blocks(bytes, blocks_start)?;
        Ok(Self {
            tree,
            blocks,
            backing,
        })
    }

    pub fn block_data(&self, index: usize) -> Result<Cow<'_, [u8]>, AsdfError> {
        let block = self
            .blocks
            .get(index)
            .ok_or(AsdfError::BlockOutOfRange(index))?;
        let bytes = self.backing.as_slice();
        let raw = &bytes[block.data_start..block.data_start + block.used_size];
        block.header.decompress(raw)
    }
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

fn parse_tree(text: &str) -> Result<Value, AsdfError> {
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

    serde_yaml::from_str(&yaml_content).map_err(|e| AsdfError::YamlParse(e.to_string()))
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
        }
    }
}

impl std::error::Error for AsdfError {}
