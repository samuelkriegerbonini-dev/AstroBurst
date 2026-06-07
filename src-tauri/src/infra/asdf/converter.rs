use std::path::Path;
use std::collections::HashMap;

use serde_yaml::Value;

use super::parser::{AsdfFile, AsdfError};
use super::tree::{NdArrayMeta, DType, ByteOrder, WcsInfo};

enum PixelLayout {
    Planar,
    Interleaved,
}

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

        let wcs = WcsInfo::from_yaml(&asdf.tree)
            .or_else(|| WcsInfo::from_gwcs(&asdf.tree));

        match Self::find_data_array(&asdf.tree) {
            Some((array_key, meta)) => {
                let block = asdf
                    .blocks
                    .get(meta.source)
                    .ok_or_else(|| AsdfError::MissingField(format!("block {}", meta.source)))?;

                let (height, width, channels, layout) = match Self::interpret_shape(&meta.shape) {
                    Ok(dims) => dims,
                    Err(_) => {
                        let metadata = Self::extract_metadata(&asdf.tree, &array_key);
                        return Ok(Self {
                            width: 0,
                            height: 0,
                            channels: 0,
                            data: Vec::new(),
                            wcs,
                            metadata,
                        });
                    }
                };

                let mut pixels = Self::to_f32_pixels(&block.data, &meta);

                let expected: usize = meta.shape.iter().product();
                if pixels.len() < expected {
                    return Err(AsdfError::MissingField(format!(
                        "array element count mismatch: got {} expected {}",
                        pixels.len(),
                        expected
                    )));
                }
                pixels.truncate(expected);

                let data = match layout {
                    PixelLayout::Interleaved if channels > 1 => {
                        Self::deinterleave(&pixels, width, height, channels)
                    }
                    _ => pixels,
                };

                let metadata = Self::extract_metadata(&asdf.tree, &array_key);

                Ok(Self {
                    width,
                    height,
                    channels,
                    data,
                    wcs,
                    metadata,
                })
            }
            None => {
                let metadata = Self::extract_metadata(&asdf.tree, "");
                Ok(Self {
                    width: 0,
                    height: 0,
                    channels: 0,
                    data: Vec::new(),
                    wcs,
                    metadata,
                })
            }
        }
    }

    pub fn has_image(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    pub fn to_array2(&self) -> ndarray::Array2<f32> {
        if !self.has_image() {
            return ndarray::Array2::zeros((0, 0));
        }
        let plane = if self.channels <= 1 {
            self.data.clone()
        } else {
            self.data[..self.width * self.height].to_vec()
        };
        ndarray::Array2::from_shape_vec((self.height, self.width), plane)
            .unwrap_or_else(|_| ndarray::Array2::zeros((self.height, self.width)))
    }

    fn find_data_array(tree: &Value) -> Option<(String, NdArrayMeta)> {
        let candidates = ["data", "sci", "SCI", "science", "image"];

        if let Some(mapping) = tree.as_mapping() {
            for key in &candidates {
                if let Some(node) = mapping.get(Value::String(key.to_string())) {
                    if let Some(meta) = Self::try_meta(node) {
                        return Some((key.to_string(), meta));
                    }
                    if let Some(data_node) = node.get("data") {
                        if let Some(meta) = Self::try_meta(data_node) {
                            return Some((key.to_string(), meta));
                        }
                    }
                }
            }
        }

        if let Some(roman) = tree.get("roman") {
            let roman_paths = ["data", "science", "sci"];
            for rp in &roman_paths {
                if let Some(node) = roman.get(*rp) {
                    if let Some(meta) = Self::try_meta(node) {
                        return Some((format!("roman.{}", rp), meta));
                    }
                }
            }
        }

        if let Some(mapping) = tree.as_mapping() {
            for (k, v) in mapping.iter() {
                if let Some(meta) = Self::deep_find_ndarray(v, 0) {
                    let key_str = k.as_str().unwrap_or("unknown").to_string();
                    return Some((key_str, meta));
                }
            }
        }

        None
    }

    fn try_meta(node: &Value) -> Option<NdArrayMeta> {
        if node.get("source").is_some() && node.get("shape").is_some() {
            return NdArrayMeta::from_yaml(node).ok();
        }
        None
    }

    fn deep_find_ndarray(node: &Value, depth: usize) -> Option<NdArrayMeta> {
        if depth > 4 {
            return None;
        }
        if let Some(meta) = Self::try_meta(node) {
            return Some(meta);
        }
        if let Some(mapping) = node.as_mapping() {
            for (_, v) in mapping.iter() {
                if let Some(meta) = Self::deep_find_ndarray(v, depth + 1) {
                    return Some(meta);
                }
            }
        }
        None
    }

    fn to_f32_pixels(raw: &[u8], meta: &NdArrayMeta) -> Vec<f32> {
        match (&meta.dtype, &meta.byteorder) {
            (DType::Int8, _) => raw.iter().map(|&b| b as i8 as f32).collect(),

            (DType::UInt8, _) => raw.iter().map(|&b| b as f32).collect(),

            (DType::Bool8, _) => raw.iter().map(|&b| if b != 0 { 1.0 } else { 0.0 }).collect(),

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

            (DType::UInt32, ByteOrder::Big) => raw
                .chunks_exact(4)
                .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as f32)
                .collect(),
            (DType::UInt32, ByteOrder::Little) => raw
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32)
                .collect(),

            (DType::Int64, ByteOrder::Big) => raw
                .chunks_exact(8)
                .map(|c| {
                    i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32
                })
                .collect(),
            (DType::Int64, ByteOrder::Little) => raw
                .chunks_exact(8)
                .map(|c| {
                    i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32
                })
                .collect(),

            (DType::UInt64, ByteOrder::Big) => raw
                .chunks_exact(8)
                .map(|c| {
                    u64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32
                })
                .collect(),
            (DType::UInt64, ByteOrder::Little) => raw
                .chunks_exact(8)
                .map(|c| {
                    u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32
                })
                .collect(),

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

            (DType::Complex64, ByteOrder::Big) => raw
                .chunks_exact(8)
                .map(|c| {
                    let re = f32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                    let im = f32::from_be_bytes([c[4], c[5], c[6], c[7]]);
                    re.hypot(im)
                })
                .collect(),
            (DType::Complex64, ByteOrder::Little) => raw
                .chunks_exact(8)
                .map(|c| {
                    let re = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    let im = f32::from_le_bytes([c[4], c[5], c[6], c[7]]);
                    re.hypot(im)
                })
                .collect(),

            (DType::Complex128, ByteOrder::Big) => raw
                .chunks_exact(16)
                .map(|c| {
                    let re = f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
                    let im = f64::from_be_bytes([c[8], c[9], c[10], c[11], c[12], c[13], c[14], c[15]]);
                    re.hypot(im) as f32
                })
                .collect(),
            (DType::Complex128, ByteOrder::Little) => raw
                .chunks_exact(16)
                .map(|c| {
                    let re = f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
                    let im = f64::from_le_bytes([c[8], c[9], c[10], c[11], c[12], c[13], c[14], c[15]]);
                    re.hypot(im) as f32
                })
                .collect(),
        }
    }

    fn deinterleave(data: &[f32], width: usize, height: usize, channels: usize) -> Vec<f32> {
        let plane = width * height;
        let mut out = vec![0.0f32; data.len()];
        for i in 0..plane {
            for c in 0..channels {
                out[c * plane + i] = data[i * channels + c];
            }
        }
        out
    }

    fn interpret_shape(shape: &[usize]) -> Result<(usize, usize, usize, PixelLayout), AsdfError> {
        match shape.len() {
            0 => Ok((1, 1, 1, PixelLayout::Planar)),
            1 => Ok((1, shape[0], 1, PixelLayout::Planar)),
            2 => Ok((shape[0], shape[1], 1, PixelLayout::Planar)),
            3 if shape[0] <= 4 => Ok((shape[1], shape[2], shape[0], PixelLayout::Planar)),
            3 if shape[2] <= 4 => Ok((shape[0], shape[1], shape[2], PixelLayout::Interleaved)),
            3 => Ok((shape[1], shape[2], shape[0], PixelLayout::Planar)),
            n => Err(AsdfError::MissingField(format!("unsupported array rank: {}", n))),
        }
    }

    fn extract_metadata(tree: &Value, data_key: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();

        if let Some(meta) = tree.get("meta") {
            Self::flatten_yaml(meta, "meta", &mut map);
        }

        if let Some(header) = tree.get("header") {
            Self::flatten_yaml(header, "header", &mut map);
        }

        if let Some(roman_meta) = tree.get("roman").and_then(|r| r.get("meta")) {
            Self::flatten_yaml(roman_meta, "roman.meta", &mut map);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asdf_reference_suite() {
        let dir = match std::env::var("ASDF_FIXTURES") {
            Ok(d) => std::path::PathBuf::from(d),
            Err(_) => return,
        };

        let no_image = [
            "01_minimal_no_arrays.asdf",
            "11_deep_nested_meta.asdf",
            "13_unicode_strings.asdf",
        ];

        let primary_sum: &[(&str, f64)] = &[
            ("02_single_float32.asdf", 120.0),
            ("05_compress_zlib.asdf", 32839.671875),
            ("06_compress_bzip2.asdf", 32760.8125),
            ("07_compress_lz4.asdf", 32691.3203125),
            ("08_highly_compressible_zlib.asdf", 100.0),
            ("10_multi_array_telescope.asdf", 524284.5),
            ("15_large_uncompressed.asdf", 2096870.875),
        ];

        let all = [
            "01_minimal_no_arrays.asdf",
            "02_single_float32.asdf",
            "03_dtypes_all.asdf",
            "04_byteorder_big.asdf",
            "05_compress_zlib.asdf",
            "06_compress_bzip2.asdf",
            "07_compress_lz4.asdf",
            "08_highly_compressible_zlib.asdf",
            "09_shapes_nd.asdf",
            "10_multi_array_telescope.asdf",
            "11_deep_nested_meta.asdf",
            "12_shared_view.asdf",
            "13_unicode_strings.asdf",
            "14_special_floats.asdf",
            "15_large_uncompressed.asdf",
        ];

        let mut failures = Vec::new();

        for name in all {
            let path = dir.join(name);
            match AsdfImage::load(&path) {
                Ok(img) => {
                    if no_image.contains(&name) && img.has_image() {
                        eprintln!("FAIL {} (expected no image, got {}x{})", name, img.width, img.height);
                        failures.push(name);
                        continue;
                    }
                    if let Some((_, expected)) = primary_sum.iter().find(|(n, _)| *n == name) {
                        let sum: f64 = img
                            .data
                            .iter()
                            .filter(|v| v.is_finite())
                            .map(|&v| v as f64)
                            .sum();
                        let tol = (expected.abs() * 1e-3).max(1e-3);
                        if (sum - expected).abs() > tol {
                            eprintln!("FAIL {} (sum {} expected {})", name, sum, expected);
                            failures.push(name);
                            continue;
                        }
                        eprintln!("PASS {} (load + sum {:.4})", name, sum);
                    } else {
                        eprintln!("PASS {} (load)", name);
                    }
                }
                Err(e) => {
                    eprintln!("FAIL {} (load error: {})", name, e);
                    failures.push(name);
                }
            }
        }

        assert!(failures.is_empty(), "ASDF reference suite failures: {:?}", failures);
    }
}
