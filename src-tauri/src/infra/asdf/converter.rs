use std::path::Path;
use std::collections::HashMap;

use serde_yaml::Value;

use super::parser::{AsdfFile, AsdfError};
use super::tree::{NdArrayMeta, DType, ByteOrder, WcsInfo};

#[derive(Debug)]
pub struct AsdfImage {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub data: Vec<f32>,
    pub wcs: Option<WcsInfo>,
    pub metadata: HashMap<String, String>,
}

impl AsdfImage {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, AsdfError> {
        let asdf = AsdfFile::open(path)?;

        let (array_key, array_node) = Self::find_data_array(&asdf.tree)?;
        let meta = NdArrayMeta::from_yaml(array_node)?;

        let block = asdf
            .blocks
            .get(meta.source)
            .ok_or_else(|| AsdfError::MissingField(format!("block {}", meta.source)))?;

        let pixels = Self::to_f32_pixels(&block.data, &meta);

        let (height, width, channels) = Self::interpret_shape(&meta.shape);

        let wcs = WcsInfo::from_yaml(&asdf.tree)
            .or_else(|| WcsInfo::from_gwcs(&asdf.tree));

        let metadata = Self::extract_metadata(&asdf.tree, &array_key);

        Ok(Self {
            width,
            height,
            channels,
            data: pixels,
            wcs,
            metadata,
        })
    }

    pub fn to_array2(&self) -> ndarray::Array2<f32> {
        let plane = if self.channels == 1 {
            self.data.clone()
        } else {
            self.data[..self.width * self.height].to_vec()
        };
        ndarray::Array2::from_shape_vec((self.height, self.width), plane)
            .unwrap_or_else(|_| ndarray::Array2::zeros((self.height, self.width)))
    }

    fn find_data_array(tree: &Value) -> Result<(String, &Value), AsdfError> {
        let candidates = ["data", "sci", "SCI", "science", "image"];

        if let Some(mapping) = tree.as_mapping() {
            for key in &candidates {
                if let Some(node) = mapping.get(Value::String(key.to_string())) {
                    if node.get("source").is_some() && node.get("shape").is_some() {
                        return Ok((key.to_string(), node));
                    }
                    if let Some(data_node) = node.get("data") {
                        if data_node.get("source").is_some() {
                            return Ok((key.to_string(), data_node));
                        }
                    }
                }
            }
        }

        if let Some(roman) = tree.get("roman") {
            let roman_paths = ["data", "science", "sci"];
            for rp in &roman_paths {
                if let Some(node) = roman.get(*rp) {
                    if node.get("source").is_some() && node.get("shape").is_some() {
                        return Ok((format!("roman.{}", rp), node));
                    }
                }
            }
        }

        if let Some(mapping) = tree.as_mapping() {
            for (k, v) in mapping.iter() {
                if let Some(found) = Self::deep_find_ndarray(v, 0) {
                    let key_str = k.as_str().unwrap_or("unknown").to_string();
                    return Ok((key_str, found));
                }
            }
        }

        Err(AsdfError::MissingField("data array".into()))
    }

    fn deep_find_ndarray(node: &Value, depth: usize) -> Option<&Value> {
        if depth > 4 {
            return None;
        }
        if node.get("source").is_some() && node.get("shape").is_some() {
            return Some(node);
        }
        if let Some(mapping) = node.as_mapping() {
            for (_, v) in mapping.iter() {
                if let Some(found) = Self::deep_find_ndarray(v, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn to_f32_pixels(raw: &[u8], meta: &NdArrayMeta) -> Vec<f32> {
        match (&meta.dtype, &meta.byteorder) {
            (DType::Float32, ByteOrder::Big) => raw
                .chunks_exact(4)
                .map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),

            (DType::Float32, ByteOrder::Little) => raw
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),

            (DType::Float64, ByteOrder::Big) => raw
                .chunks_exact(8)
                .map(|c| {
                    f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32
                })
                .collect(),

            (DType::Float64, ByteOrder::Little) => raw
                .chunks_exact(8)
                .map(|c| {
                    f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32
                })
                .collect(),

            (DType::Int16, ByteOrder::Big) => raw
                .chunks_exact(2)
                .map(|c| i16::from_be_bytes([c[0], c[1]]) as f32)
                .collect(),

            (DType::Int16, ByteOrder::Little) => raw
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32)
                .collect(),

            (DType::UInt16, ByteOrder::Big) => raw
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]) as f32)
                .collect(),

            (DType::UInt16, ByteOrder::Little) => raw
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]) as f32)
                .collect(),

            (DType::Int32, ByteOrder::Big) => raw
                .chunks_exact(4)
                .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]) as f32)
                .collect(),

            (DType::Int32, ByteOrder::Little) => raw
                .chunks_exact(4)
                .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32)
                .collect(),

            (DType::UInt8, _) => raw.iter().map(|&b| b as f32).collect(),

            (DType::Complex64, ByteOrder::Big) => raw
                .chunks_exact(8)
                .map(|c| {
                    let re = f32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                    let im = f32::from_be_bytes([c[4], c[5], c[6], c[7]]);
                    (re * re + im * im).sqrt()
                })
                .collect(),

            (DType::Complex64, ByteOrder::Little) => raw
                .chunks_exact(8)
                .map(|c| {
                    let re = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    let im = f32::from_le_bytes([c[4], c[5], c[6], c[7]]);
                    (re * re + im * im).sqrt()
                })
                .collect(),
        }
    }

    fn interpret_shape(shape: &[usize]) -> (usize, usize, usize) {
        match shape.len() {
            2 => (shape[0], shape[1], 1),
            3 if shape[0] <= 4 => (shape[1], shape[2], shape[0]),
            3 if shape[2] <= 4 => (shape[0], shape[1], shape[2]),
            3 => (shape[1], shape[2], shape[0]),
            _ => {
                let total: usize = shape.iter().product();
                let side = (total as f64).sqrt() as usize;
                (side, side, 1)
            }
        }
    }

    fn extract_metadata(tree: &Value, data_key: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();

        if let Some(meta) = tree.get("meta").and_then(|v| v.as_mapping()) {
            Self::flatten_yaml(&Value::Mapping(meta.clone()), "meta", &mut map);
        }

        if let Some(header) = tree.get("header").and_then(|v| v.as_mapping()) {
            Self::flatten_yaml(&Value::Mapping(header.clone()), "header", &mut map);
        }

        if let Some(roman_meta) = tree
            .get("roman")
            .and_then(|r| r.get("meta"))
            .and_then(|v| v.as_mapping())
        {
            Self::flatten_yaml(&Value::Mapping(roman_meta.clone()), "roman.meta", &mut map);
        }

        map.insert("ASDF_DATA_KEY".into(), data_key.to_string());

        map
    }

    fn flatten_yaml(val: &Value, prefix: &str, out: &mut HashMap<String, String>) {
        match val {
            Value::Mapping(m) => {
                for (k, v) in m.iter() {
                    let key_str = k.as_str().unwrap_or("?");
                    let full_key = if prefix.is_empty() {
                        key_str.to_string()
                    } else {
                        format!("{}.{}", prefix, key_str)
                    };
                    Self::flatten_yaml(v, &full_key, out);
                }
            }
            Value::String(s) => {
                out.insert(prefix.to_string(), s.clone());
            }
            Value::Number(n) => {
                out.insert(prefix.to_string(), n.to_string());
            }
            Value::Bool(b) => {
                out.insert(prefix.to_string(), b.to_string());
            }
            _ => {}
        }
    }

    pub fn channel_data(&self, ch: usize) -> Option<&[f32]> {
        if ch >= self.channels {
            return None;
        }
        let plane_size = self.width * self.height;
        let start = ch * plane_size;
        let end = start + plane_size;
        if end <= self.data.len() {
            Some(&self.data[start..end])
        } else {
            None
        }
    }

    pub fn pixel(&self, x: usize, y: usize, ch: usize) -> Option<f32> {
        if x >= self.width || y >= self.height || ch >= self.channels {
            return None;
        }
        let plane_size = self.width * self.height;
        let idx = ch * plane_size + y * self.width + x;
        self.data.get(idx).copied()
    }
}

pub fn is_asdf_file<P: AsRef<Path>>(path: P) -> bool {
    let path = path.as_ref();

    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        if ext == "asdf" {
            return true;
        }
    }

    if let Ok(mut f) = std::fs::File::open(path) {
        use std::io::Read;
        let mut buf = [0u8; 5];
        if f.read_exact(&mut buf).is_ok() {
            return &buf == b"#ASDF";
        }
    }

    false
}
