use std::io::{BufRead, BufReader};
use std::fs::File;
use std::path::Path;

use serde_yaml::Value;

use super::blocks::{BlockHeader, BlockData};

const ASDF_MAGIC: &str = "#ASDF";
const YAML_DOC_END: &str = "...";

#[derive(Debug)]
pub struct AsdfFile {
    pub version: String,
    pub standard_version: Option<String>,
    pub tree: Value,
    pub blocks: Vec<BlockData>,
}

impl AsdfFile {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, AsdfError> {
        let file = File::open(path.as_ref())?;
        let mut reader = BufReader::new(file);

        let (version, standard_version) = Self::read_preamble(&mut reader)?;
        let tree = Self::read_yaml_tree(&mut reader)?;
        let blocks = Self::read_blocks(&mut reader)?;

        Ok(Self {
            version,
            standard_version,
            tree,
            blocks,
        })
    }

    fn read_preamble<R: BufRead>(reader: &mut R) -> Result<(String, Option<String>), AsdfError> {
        let mut line = String::new();
        reader.read_line(&mut line)?;

        if !line.starts_with(ASDF_MAGIC) {
            return Err(AsdfError::InvalidMagic);
        }

        let version = line
            .trim()
            .strip_prefix("#ASDF ")
            .unwrap_or("1.0.0")
            .to_string();

        let mut standard_version = None;
        let mut peek_line = String::new();
        let bytes_read = reader.read_line(&mut peek_line)?;

        if bytes_read > 0 && peek_line.starts_with("#ASDF_STANDARD") {
            standard_version = peek_line
                .trim()
                .strip_prefix("#ASDF_STANDARD ")
                .map(|s| s.to_string());
        }

        Ok((version, standard_version))
    }

    fn read_yaml_tree<R: BufRead>(reader: &mut R) -> Result<Value, AsdfError> {
        let mut yaml_content = String::new();
        let mut in_document = false;

        for line_result in reader.by_ref().lines() {
            let line = line_result?;

            if line.starts_with("---") {
                in_document = true;
                continue;
            }

            if line == YAML_DOC_END {
                break;
            }

            if line.starts_with("%YAML") || line.starts_with("%TAG") || line.starts_with('#') {
                continue;
            }

            if in_document {
                yaml_content.push_str(&line);
                yaml_content.push('\n');
            }
        }

        if yaml_content.is_empty() {
            return Err(AsdfError::NoYamlTree);
        }

        let tree: Value = serde_yaml::from_str(&yaml_content)
            .map_err(|e| AsdfError::YamlParse(e.to_string()))?;

        Ok(tree)
    }

    fn read_blocks<R: BufRead>(reader: &mut R) -> Result<Vec<BlockData>, AsdfError> {
        let mut blocks = Vec::new();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;

        let mut offset = 0;
        while offset < buf.len() {
            offset = skip_padding(&buf, offset);

            if offset + 4 > buf.len() {
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
            let data_end = data_start + header.allocated_size as usize;

            if data_end > buf.len() {
                return Err(AsdfError::BlockTruncated);
            }

            let raw = &buf[data_start..data_start + header.used_size as usize];
            let decompressed = header.decompress(raw)?;

            blocks.push(BlockData {
                index: blocks.len(),
                data: decompressed,
                original_size: header.data_size as usize,
            });

            offset = data_end;
        }

        Ok(blocks)
    }

}

fn skip_padding(buf: &[u8], mut offset: usize) -> usize {
    while offset < buf.len() && buf[offset] == 0 {
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
        }
    }
}

impl std::error::Error for AsdfError {}
