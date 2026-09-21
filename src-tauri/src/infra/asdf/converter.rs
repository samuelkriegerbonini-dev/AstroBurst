use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;
use serde_yaml::Value;

use super::parser::{AsdfError, AsdfFile};
use super::tree::{untag, ArraySource, ByteOrder, DType, NdArrayMeta, WcsInfo};
use crate::types::image::IntPlane;

enum PixelLayout {
    Planar,
    Interleaved,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsdfArrayInfo {
    pub key: String,
    pub shape: Vec<usize>,
    pub dtype: DType,
    pub bitpix: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneGeometry {
    pub height: usize,
    pub width: usize,
    pub plane_count: usize,
}

pub fn plane_geometry(shape: &[usize]) -> PlaneGeometry {
    let (height, width, plane_count, _) = AsdfImage::interpret_shape(shape);
    PlaneGeometry { height, width, plane_count }
}

pub fn is_interleaved_layout(shape: &[usize]) -> bool {
    matches!(AsdfImage::interpret_shape(shape).3, PixelLayout::Interleaved)
}

pub fn shape_label(shape: &[usize]) -> String {
    shape
        .iter()
        .map(usize::to_string)
        .collect::<Vec<String>>()
        .join("x")
}

pub fn bitpix_for_dtype(d: &DType) -> i64 {
    match d {
        DType::Int8 | DType::UInt8 | DType::Bool8 => 8,
        DType::Int16 | DType::UInt16 => 16,
        DType::Int32 | DType::UInt32 => 32,
        DType::Int64 | DType::UInt64 => 64,
        DType::Float32 => -32,
        DType::Float64 => -64,
        DType::Complex64 | DType::Complex128 => 0,
    }
}

fn array_info_at(node: &Value, key: String) -> Option<AsdfArrayInfo> {
    let meta = AsdfImage::try_meta_or_wrapped(node).ok().flatten()?;
    if meta.shape.len() < 2 {
        return None;
    }
    let bitpix = bitpix_for_dtype(&meta.dtype);
    Some(AsdfArrayInfo { key, shape: meta.shape, dtype: meta.dtype, bitpix })
}

pub fn auto_data_key(asdf: &AsdfFile) -> Option<String> {
    AsdfImage::find_data_array(&asdf.tree)
        .ok()
        .flatten()
        .map(|(key, _)| key)
}

pub fn list_arrays(asdf: &AsdfFile) -> Vec<AsdfArrayInfo> {
    let mut out = Vec::new();
    let Some(mapping) = untag(&asdf.tree).as_mapping() else {
        return out;
    };
    for (k, v) in mapping.iter() {
        let Some(key) = k.as_str() else { continue };
        if let Some(info) = array_info_at(v, key.to_string()) {
            out.push(info);
        }
    }
    if let Some(roman) = asdf.tree.get("roman").and_then(|r| untag(r).as_mapping()) {
        for (k, v) in roman.iter() {
            let Some(key) = k.as_str() else { continue };
            if let Some(info) = array_info_at(v, format!("roman.{}", key)) {
                out.push(info);
            }
        }
    }
    out
}

#[derive(Debug)]
pub struct AsdfImage {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
    pub wcs: Option<WcsInfo>,
    pub metadata: HashMap<String, String>,
    pub unit: Option<String>,
}

impl AsdfImage {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, AsdfError> {
        let asdf = AsdfFile::open(path)?;
        Self::from_file(&asdf)
    }

    pub fn from_file(asdf: &AsdfFile) -> Result<Self, AsdfError> {
        let wcs = Self::resolve_wcs(&asdf.tree);

        match Self::find_data_array(&asdf.tree)? {
            Some((key, meta)) => Self::from_array(asdf, &key, meta, wcs),
            None => Ok(Self::empty(wcs, Self::extract_metadata(&asdf.tree, ""))),
        }
    }

    fn resolve_wcs(tree: &Value) -> Option<WcsInfo> {
        WcsInfo::from_yaml(tree).or_else(|| WcsInfo::from_gwcs(tree))
    }

    fn empty(wcs: Option<WcsInfo>, metadata: HashMap<String, String>) -> Self {
        Self {
            width: 0,
            height: 0,
            channels: 0,
            shape: Vec::new(),
            data: Vec::new(),
            wcs,
            metadata,
            unit: None,
        }
    }

    fn node_at<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
        let mut node = tree;
        for part in key.split('.') {
            node = untag(node).as_mapping()?.get(Value::String(part.to_string()))?;
        }
        Some(node)
    }

    fn extract_unit(tree: &Value, key: &str) -> Option<String> {
        let node = Self::node_at(tree, key)?;
        let unit = untag(node)
            .as_mapping()?
            .get(Value::String("unit".to_string()))?;
        let text = match untag(unit) {
            Value::String(s) => s.trim().to_string(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };
        (!text.is_empty()).then_some(text)
    }

    fn from_array(
        asdf: &AsdfFile,
        key: &str,
        mut meta: NdArrayMeta,
        wcs: Option<WcsInfo>,
    ) -> Result<Self, AsdfError> {
        let mut pixels = match &meta.source {
            ArraySource::Inline(values) => values.clone(),
            ArraySource::Block(index) => {
                let block = asdf.block_data(*index)?;
                meta.resolve_streamed_shape(block.len());
                let raw = Self::gather_array_bytes(&block, &meta);
                Self::to_f32_pixels(&raw, &meta)
            }
        };

        let (height, width, channels, layout) = Self::interpret_shape(&meta.shape);

        let expected: usize = meta.shape.iter().product();
        if pixels.len() < expected {
            return Err(AsdfError::ShapeMismatch {
                got: pixels.len(),
                expected,
            });
        }
        pixels.truncate(expected);

        let data = match layout {
            PixelLayout::Interleaved if channels > 1 => {
                Self::deinterleave(&pixels, width, height, channels)
            }
            _ => pixels,
        };

        Ok(Self {
            width,
            height,
            channels,
            shape: meta.shape,
            data,
            wcs,
            metadata: Self::extract_metadata(&asdf.tree, key),
            unit: Self::extract_unit(&asdf.tree, key),
        })
    }

    pub fn load_array(asdf: &AsdfFile, key: &str) -> Result<Self, AsdfError> {
        let node = Self::node_at(&asdf.tree, key)
            .ok_or_else(|| AsdfError::MissingField(key.to_string()))?;
        let meta = Self::try_meta_or_wrapped(node)?
            .ok_or_else(|| AsdfError::MissingField(key.to_string()))?;
        let wcs = Self::resolve_wcs(&asdf.tree);
        Self::from_array(asdf, key, meta, wcs)
    }

    pub fn load_array_int(asdf: &AsdfFile, key: &str) -> Result<Option<IntPlane>, AsdfError> {
        let node = Self::node_at(&asdf.tree, key)
            .ok_or_else(|| AsdfError::MissingField(key.to_string()))?;
        let mut meta = Self::try_meta_or_wrapped(node)?
            .ok_or_else(|| AsdfError::MissingField(key.to_string()))?;
        let signed = match meta.dtype {
            DType::Int8 | DType::Int16 | DType::Int32 => true,
            DType::UInt8 | DType::UInt16 | DType::UInt32 | DType::Bool8 => false,
            _ => return Ok(None),
        };
        let bits: Vec<u32> = match &meta.source {
            ArraySource::Inline(values) => values
                .iter()
                .map(|&v| if signed { v as i32 as u32 } else { v as u32 })
                .collect(),
            ArraySource::Block(index) => {
                let block = asdf.block_data(*index)?;
                meta.resolve_streamed_shape(block.len());
                let raw = Self::gather_array_bytes(&block, &meta);
                match Self::to_int_pixels(&raw, &meta) {
                    Some((bits, _)) => bits,
                    None => return Ok(None),
                }
            }
        };
        if meta.shape.len() < 2 {
            return Ok(None);
        }
        let geometry = plane_geometry(&meta.shape);
        if geometry.plane_count > 1 && is_interleaved_layout(&meta.shape) {
            return Ok(None);
        }
        let (height, width) = (geometry.height, geometry.width);
        let expected = width * height;
        if bits.len() < expected {
            return Err(AsdfError::ShapeMismatch { got: bits.len(), expected });
        }
        let mut plane = bits;
        plane.truncate(expected);
        let arr = ndarray::Array2::from_shape_vec((height, width), plane)
            .map_err(|_| AsdfError::ShapeMismatch { got: expected, expected })?;
        Ok(Some(IntPlane { bits: arr, signed }))
    }

    pub fn array_exists(tree: &Value, key: &str) -> bool {
        Self::node_at(tree, key)
            .and_then(|n| Self::try_meta_or_wrapped(n).ok().flatten())
            .is_some()
    }

    fn to_int_pixels(raw: &[u8], meta: &NdArrayMeta) -> Option<(Vec<u32>, bool)> {
        let order = meta.byteorder;
        macro_rules! decode_int {
            ($n:literal, $ty:ty, $signed:expr) => {{
                let v: Vec<u32> = raw
                    .chunks_exact($n)
                    .map(|c| {
                        let b: [u8; $n] = c.try_into().unwrap_or([0u8; $n]);
                        let x = match order {
                            ByteOrder::Big => <$ty>::from_be_bytes(b),
                            ByteOrder::Little => <$ty>::from_le_bytes(b),
                        };
                        x as i64 as u32
                    })
                    .collect();
                Some((v, $signed))
            }};
        }
        match &meta.dtype {
            DType::Int8 => Some((raw.iter().map(|&c| c as i8 as i32 as u32).collect(), true)),
            DType::UInt8 => Some((raw.iter().map(|&c| c as u32).collect(), false)),
            DType::Bool8 => Some((raw.iter().map(|&c| (c != 0) as u32).collect(), false)),
            DType::Int16 => decode_int!(2, i16, true),
            DType::UInt16 => decode_int!(2, u16, false),
            DType::Int32 => decode_int!(4, i32, true),
            DType::UInt32 => decode_int!(4, u32, false),
            _ => None,
        }
    }

    pub fn has_image(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    pub fn plane_count(&self) -> usize {
        self.channels
    }

    fn plane_bounds(&self, index: usize) -> Option<(usize, usize)> {
        if !self.has_image() || index >= self.channels.max(1) {
            return None;
        }
        let size = self.width.checked_mul(self.height)?;
        let start = index.checked_mul(size)?;
        let end = start.checked_add(size)?;
        (end <= self.data.len()).then_some((start, end))
    }

    pub fn plane(&self, index: usize) -> Option<ndarray::Array2<f32>> {
        let (start, end) = self.plane_bounds(index)?;
        ndarray::Array2::from_shape_vec((self.height, self.width), self.data[start..end].to_vec()).ok()
    }

    pub fn into_plane(mut self, index: usize) -> Option<ndarray::Array2<f32>> {
        let (start, end) = self.plane_bounds(index)?;
        self.data.truncate(end);
        self.data.drain(..start);
        ndarray::Array2::from_shape_vec((self.height, self.width), self.data).ok()
    }

    fn find_data_array(tree: &Value) -> Result<Option<(String, NdArrayMeta)>, AsdfError> {
        let candidates = ["data", "sci", "SCI", "science", "image"];

        if let Some(mapping) = tree.as_mapping() {
            for key in &candidates {
                if let Some(node) = mapping.get(Value::String(key.to_string())) {
                    if let Some(meta) = Self::try_meta_or_wrapped(node)? {
                        return Ok(Some((key.to_string(), meta)));
                    }
                }
            }
        }

        if let Some(roman) = tree.get("roman") {
            let roman_paths = ["data", "science", "sci"];
            for rp in &roman_paths {
                if let Some(node) = roman.get(*rp) {
                    if let Some(meta) = Self::try_meta_or_wrapped(node)? {
                        return Ok(Some((format!("roman.{}", rp), meta)));
                    }
                }
            }
        }

        if let Some(mapping) = tree.as_mapping() {
            for (k, v) in mapping.iter() {
                if let Some(meta) = Self::deep_find_ndarray(v, 0)? {
                    let key_str = k.as_str().unwrap_or("unknown").to_string();
                    return Ok(Some((key_str, meta)));
                }
            }
        }

        Ok(None)
    }

    fn try_meta_or_wrapped(node: &Value) -> Result<Option<NdArrayMeta>, AsdfError> {
        if let Some(meta) = Self::try_meta(node, true)? {
            return Ok(Some(meta));
        }
        for wrapper in ["data", "value"] {
            if let Some(inner) = node.get(wrapper) {
                if let Some(meta) = Self::try_meta(inner, true)? {
                    return Ok(Some(meta));
                }
            }
        }
        Ok(None)
    }

    fn try_meta(node: &Value, strict: bool) -> Result<Option<NdArrayMeta>, AsdfError> {
        let is_block_array = node.get("source").is_some() && node.get("shape").is_some();
        let is_inline_array = node
            .get("data")
            .map(|d| d.as_sequence().is_some())
            .unwrap_or(false)
            && node.get("source").is_none();
        if !is_block_array && !is_inline_array {
            return Ok(None);
        }
        match NdArrayMeta::from_yaml(node) {
            Ok(meta) => Ok(Some(meta)),
            Err(e) if strict => Err(e),
            Err(_) => Ok(None),
        }
    }

    fn deep_find_ndarray(node: &Value, depth: usize) -> Result<Option<NdArrayMeta>, AsdfError> {
        if depth > 4 {
            return Ok(None);
        }
        if let Some(meta) = Self::try_meta(node, false)? {
            return Ok(Some(meta));
        }
        if let Some(mapping) = node.as_mapping() {
            for (_, v) in mapping.iter() {
                if let Some(meta) = Self::deep_find_ndarray(v, depth + 1)? {
                    return Ok(Some(meta));
                }
            }
        }
        Ok(None)
    }

    fn gather_array_bytes<'a>(block: &'a [u8], meta: &NdArrayMeta) -> Cow<'a, [u8]> {
        Self::gather_elements(
            block,
            &meta.shape,
            meta.byte_size_per_element(),
            meta.offset,
            &meta.effective_strides(),
        )
    }

    fn gather_elements<'a>(
        block: &'a [u8],
        shape: &[usize],
        element_size: usize,
        offset: usize,
        strides: &[isize],
    ) -> Cow<'a, [u8]> {
        let elem = element_size.max(1);
        let count: usize = shape.iter().product();
        let expected = count.saturating_mul(elem);

        if strides == NdArrayMeta::contiguous_strides(shape, elem).as_slice()
            && offset <= block.len()
        {
            let end = offset.saturating_add(expected).min(block.len());
            let usable = ((end - offset) / elem) * elem;
            return Cow::Borrowed(&block[offset..offset + usable]);
        }

        let ndim = shape.len();
        let mut out = Vec::with_capacity(expected.min(block.len()));
        let mut idx = vec![0usize; ndim];
        for _ in 0..count {
            let mut pos = offset as isize;
            for axis in 0..ndim {
                pos = pos.saturating_add((idx[axis] as isize).saturating_mul(strides[axis]));
            }
            if pos < 0 {
                break;
            }
            let start = pos as usize;
            let end = start.saturating_add(elem);
            if end > block.len() {
                break;
            }
            out.extend_from_slice(&block[start..end]);
            for axis in (0..ndim).rev() {
                idx[axis] += 1;
                if idx[axis] < shape[axis] {
                    break;
                }
                idx[axis] = 0;
            }
        }
        Cow::Owned(out)
    }

    fn read_f32(c: &[u8], order: ByteOrder) -> f32 {
        let b = [c[0], c[1], c[2], c[3]];
        match order {
            ByteOrder::Big => f32::from_be_bytes(b),
            ByteOrder::Little => f32::from_le_bytes(b),
        }
    }

    fn read_f64(c: &[u8], order: ByteOrder) -> f64 {
        let b = [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]];
        match order {
            ByteOrder::Big => f64::from_be_bytes(b),
            ByteOrder::Little => f64::from_le_bytes(b),
        }
    }

    fn decode_par<F>(raw: &[u8], elem: usize, f: F) -> Vec<f32>
    where
        F: Fn(&[u8]) -> f32 + Sync + Send,
    {
        const PAR_MIN_ELEMS: usize = 1 << 16;
        if raw.len() / elem.max(1) >= PAR_MIN_ELEMS {
            raw.par_chunks_exact(elem).map(|c| f(c)).collect()
        } else {
            raw.chunks_exact(elem).map(|c| f(c)).collect()
        }
    }

    fn to_f32_pixels(raw: &[u8], meta: &NdArrayMeta) -> Vec<f32> {
        let order = meta.byteorder;

        macro_rules! decode {
            ($n:literal, $ty:ty) => {
                Self::decode_par(raw, $n, |c| {
                    let b: [u8; $n] = c.try_into().unwrap();
                    (match order {
                        ByteOrder::Big => <$ty>::from_be_bytes(b),
                        ByteOrder::Little => <$ty>::from_le_bytes(b),
                    }) as f32
                })
            };
        }

        match &meta.dtype {
            DType::Int8 => Self::decode_par(raw, 1, |c| c[0] as i8 as f32),
            DType::UInt8 => Self::decode_par(raw, 1, |c| c[0] as f32),
            DType::Bool8 => Self::decode_par(raw, 1, |c| (c[0] != 0) as u8 as f32),
            DType::Int16 => decode!(2, i16),
            DType::UInt16 => decode!(2, u16),
            DType::Int32 => decode!(4, i32),
            DType::UInt32 => decode!(4, u32),
            DType::Int64 => decode!(8, i64),
            DType::UInt64 => decode!(8, u64),
            DType::Float32 => Self::decode_par(raw, 4, |c| Self::read_f32(c, order)),
            DType::Float64 => Self::decode_par(raw, 8, |c| Self::read_f64(c, order) as f32),
            DType::Complex64 => Self::decode_par(raw, 8, |c| {
                Self::read_f32(&c[0..4], order).hypot(Self::read_f32(&c[4..8], order))
            }),
            DType::Complex128 => Self::decode_par(raw, 16, |c| {
                (Self::read_f64(&c[0..8], order).hypot(Self::read_f64(&c[8..16], order))) as f32
            }),
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

    fn interpret_shape(shape: &[usize]) -> (usize, usize, usize, PixelLayout) {
        match shape.len() {
            0 => (1, 1, 1, PixelLayout::Planar),
            1 => (1, shape[0], 1, PixelLayout::Planar),
            2 => (shape[0], shape[1], 1, PixelLayout::Planar),
            3 if shape[0] <= 4 => (shape[1], shape[2], shape[0], PixelLayout::Planar),
            3 if shape[2] <= 4 => (shape[0], shape[1], shape[2], PixelLayout::Interleaved),
            3 => (shape[1], shape[2], shape[0], PixelLayout::Planar),
            n => (
                shape[n - 2],
                shape[n - 1],
                shape[..n - 2].iter().product(),
                PixelLayout::Planar,
            ),
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
        match untag(val) {
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
    use super::super::tree::NdArrayMeta;

    fn fixtures_dir() -> Option<std::path::PathBuf> {
        if let Ok(d) = std::env::var("ASDF_FIXTURES") {
            return Some(std::path::PathBuf::from(d));
        }
        let bundled = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("asdf");
        bundled.is_dir().then_some(bundled)
    }

    fn block(flags: u32, compression: &[u8; 4], data: &[u8], data_size: usize, extra_alloc: usize, extra_header: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0xd3, 0x42, 0x4c, 0x4b]);
        let header_size = 48 + extra_header;
        out.extend_from_slice(&(header_size as u16).to_be_bytes());
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(compression);
        let allocated = if flags & 1 != 0 { 0 } else { data.len() + extra_alloc };
        let used = if flags & 1 != 0 { 0 } else { data.len() };
        out.extend_from_slice(&(allocated as u64).to_be_bytes());
        out.extend_from_slice(&(used as u64).to_be_bytes());
        out.extend_from_slice(&(data_size as u64).to_be_bytes());
        out.extend_from_slice(&[0u8; 16]);
        out.extend(std::iter::repeat(0u8).take(extra_header));
        out.extend_from_slice(data);
        out.extend(std::iter::repeat(0u8).take(extra_alloc));
        out
    }

    fn raw_block(data: &[u8]) -> Vec<u8> {
        block(0, b"\0\0\0\0", data, data.len(), 0, 0)
    }

    fn asdf_bytes(tree_yaml: &str, blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        out.extend_from_slice(tree_yaml.as_bytes());
        out.extend_from_slice(b"...\n");
        for b in blocks {
            out.extend_from_slice(b);
        }
        out
    }

    fn f32_le(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn load_bytes(bytes: Vec<u8>) -> Result<AsdfImage, AsdfError> {
        let file = AsdfFile::from_bytes(bytes)?;
        AsdfImage::from_file(&file)
    }

    const TREE_2X3: &str = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [2, 3]\n";

    #[test]
    fn synthetic_minimal_image_loads() {
        let img = load_bytes(asdf_bytes(TREE_2X3, &[raw_block(&f32_le(&[0., 1., 2., 3., 4., 5.]))])).unwrap();
        assert_eq!((img.height, img.width, img.channels), (2, 3, 1));
        assert_eq!(img.data, vec![0., 1., 2., 3., 4., 5.]);
    }

    #[test]
    fn crlf_line_endings_accepted() {
        let mut out = Vec::new();
        out.extend_from_slice(b"#ASDF 1.0.0\r\n#ASDF_STANDARD 1.5.0\r\n%YAML 1.1\r\n--- !core/asdf-1.1.0\r\n");
        out.extend_from_slice(TREE_2X3.replace('\n', "\r\n").as_bytes());
        out.extend_from_slice(b"...\r\n");
        out.extend_from_slice(&raw_block(&f32_le(&[1., 1., 1., 1., 1., 1.])));
        let img = load_bytes(out).unwrap();
        assert_eq!(img.data.iter().sum::<f32>(), 6.0);
    }

    #[test]
    fn missing_asdf_standard_line_accepted() {
        let mut out = Vec::new();
        out.extend_from_slice(b"#ASDF 1.0.0\n%YAML 1.1\n--- !core/asdf-1.1.0\n");
        out.extend_from_slice(TREE_2X3.as_bytes());
        out.extend_from_slice(b"...\n");
        out.extend_from_slice(&raw_block(&f32_le(&[2.; 6])));
        let file = AsdfFile::from_bytes(out).unwrap();
        assert_eq!(file.standard_version, None);
        assert_eq!(file.version, "1.0.0");
        assert_eq!(AsdfImage::from_file(&file).unwrap().data.len(), 6);
    }

    #[test]
    fn tree_without_directives_or_document_marker_line_2() {
        let mut out = Vec::new();
        out.extend_from_slice(b"#ASDF 1.0.0\n--- !core/asdf-1.1.0\n");
        out.extend_from_slice(TREE_2X3.as_bytes());
        out.extend_from_slice(b"...\n");
        out.extend_from_slice(&raw_block(&f32_le(&[3.; 6])));
        assert_eq!(load_bytes(out).unwrap().data.len(), 6);
    }

    #[test]
    fn block_index_at_eof_is_ignored() {
        let mut bytes = asdf_bytes(TREE_2X3, &[raw_block(&f32_le(&[1., 2., 3., 4., 5., 6.]))]);
        bytes.extend_from_slice(b"#ASDF BLOCK INDEX\n%YAML 1.1\n---\n- 200\n...\n");
        let file = AsdfFile::from_bytes(bytes).unwrap();
        assert_eq!(file.blocks.len(), 1);
        assert_eq!(AsdfImage::from_file(&file).unwrap().data.iter().sum::<f32>(), 21.0);
    }

    #[test]
    fn padding_with_spaces_after_tree_is_skipped() {
        let mut out = Vec::new();
        out.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n--- !core/asdf-1.1.0\n");
        out.extend_from_slice(TREE_2X3.as_bytes());
        out.extend_from_slice(b"...\n");
        out.extend(std::iter::repeat(b' ').take(37));
        out.extend_from_slice(&raw_block(&f32_le(&[1.; 6])));
        assert_eq!(load_bytes(out).unwrap().data.len(), 6);
    }

    #[test]
    fn header_size_larger_than_48_and_allocated_padding() {
        let tree = "data: !core/ndarray-1.0.0\n  source: 1\n  datatype: float32\n  byteorder: little\n  shape: [2, 3]\n";
        let first = block(0, b"\0\0\0\0", &f32_le(&[9.; 4]), 16, 24, 8);
        let second = block(0, b"\0\0\0\0", &f32_le(&[1., 2., 3., 4., 5., 6.]), 24, 0, 16);
        let file = AsdfFile::from_bytes(asdf_bytes(tree, &[first, second])).unwrap();
        assert_eq!(file.blocks.len(), 2);
        assert_eq!(file.blocks[0].header.header_size, 56);
        let img = AsdfImage::from_file(&file).unwrap();
        assert_eq!(img.data, vec![1., 2., 3., 4., 5., 6.]);
    }

    #[test]
    fn streamed_block_extends_to_eof() {
        let tree = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: ['*', 4]\n";
        let streamed = block(1, b"\0\0\0\0", &f32_le(&[1.; 12]), 0, 0, 0);
        let img = load_bytes(asdf_bytes(tree, &[streamed])).unwrap();
        assert_eq!((img.height, img.width), (3, 4));
        assert_eq!(img.data.iter().sum::<f32>(), 12.0);
    }

    #[test]
    fn negative_strides_reverse_rows() {
        let tree = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [2, 3]\n  offset: 12\n  strides: [-12, 4]\n";
        let img = load_bytes(asdf_bytes(tree, &[raw_block(&f32_le(&[0., 1., 2., 3., 4., 5.]))])).unwrap();
        assert_eq!(img.data, vec![3., 4., 5., 0., 1., 2.]);
    }

    #[test]
    fn column_view_with_strides() {
        let tree = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [3]\n  offset: 4\n  strides: [12]\n";
        let img = load_bytes(asdf_bytes(tree, &[raw_block(&f32_le(&[0., 1., 2., 3., 4., 5., 6., 7., 8.]))])).unwrap();
        assert_eq!(img.data, vec![1., 4., 7.]);
    }

    #[test]
    fn external_source_fails_loudly() {
        let tree = "data: !core/ndarray-1.0.0\n  source: file0.asdf\n  datatype: float32\n  byteorder: little\n  shape: [2, 3]\n";
        match load_bytes(asdf_bytes(tree, &[])) {
            Err(AsdfError::ExternalBlock(uri)) => assert_eq!(uri, "file0.asdf"),
            other => panic!("expected ExternalBlock error, got {:?}", other.map(|i| i.data)),
        }
    }

    #[test]
    fn inline_data_array_loads() {
        let tree = "data: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\n";
        let img = load_bytes(asdf_bytes(tree, &[])).unwrap();
        assert_eq!((img.height, img.width), (2, 2));
        assert_eq!(img.data, vec![1., 2., 3., 4.]);
    }

    #[test]
    fn plain_ndarray_node_has_no_unit() {
        let img = load_bytes(asdf_bytes(TREE_2X3, &[raw_block(&f32_le(&[1.; 6]))])).unwrap();
        assert_eq!(img.unit, None);
    }

    #[test]
    fn quantity_tagged_data_node_yields_unit() {
        let tree = "data: !unit/quantity-1.1.0\n  value: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  unit: !unit/unit-1.0.0 DN / s\n";
        let img = load_bytes(asdf_bytes(tree, &[])).unwrap();
        assert_eq!((img.height, img.width), (2, 2));
        assert_eq!(img.data, vec![1., 2., 3., 4.]);
        assert_eq!(img.metadata.get("ASDF_DATA_KEY").map(String::as_str), Some("data"));
        assert_eq!(img.unit.as_deref(), Some("DN / s"));
    }

    #[test]
    fn try_meta_or_wrapped_unwraps_data_and_value_containers() {
        let inline: Value = serde_yaml::from_str("data: [[1, 2], [3, 4]]\ndatatype: float32\n").unwrap();
        assert_eq!(AsdfImage::try_meta_or_wrapped(&inline).unwrap().unwrap().shape, vec![2, 2]);

        let wrapped_data: Value =
            serde_yaml::from_str("data:\n  data: [[1, 2, 3]]\n  datatype: float32\n").unwrap();
        assert_eq!(AsdfImage::try_meta_or_wrapped(&wrapped_data).unwrap().unwrap().shape, vec![1, 3]);

        let wrapped_value: Value =
            serde_yaml::from_str("value:\n  data: [[1], [2], [3]]\n  datatype: float32\nunit: electron\n").unwrap();
        assert_eq!(AsdfImage::try_meta_or_wrapped(&wrapped_value).unwrap().unwrap().shape, vec![3, 1]);

        let scalar: Value = serde_yaml::from_str("value: 7\nunit: electron\n").unwrap();
        assert!(AsdfImage::try_meta_or_wrapped(&scalar).unwrap().is_none());
    }

    #[test]
    fn roman_quantity_node_yields_unit_through_dotted_key() {
        let tree = "roman:\n  meta:\n    instrument: {name: WFI}\n  data: !unit/quantity-1.1.0\n    value: !core/ndarray-1.0.0\n      data: [[5, 6], [7, 8]]\n      datatype: float32\n    unit: electron\n";
        let img = load_bytes(asdf_bytes(tree, &[])).unwrap();
        assert_eq!(img.metadata.get("ASDF_DATA_KEY").map(String::as_str), Some("roman.data"));
        assert_eq!(img.unit.as_deref(), Some("electron"));
    }

    #[test]
    fn empty_unit_is_treated_as_absent() {
        let tree = "data: !unit/quantity-1.1.0\n  value: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  unit: '  '\n";
        let img = load_bytes(asdf_bytes(tree, &[])).unwrap();
        assert_eq!(img.unit, None);
    }

    #[test]
    fn zlib_block_decompressed() {
        use std::io::Write;
        let payload = f32_le(&[7., 7., 7., 7., 7., 7.]);
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&payload).unwrap();
        let compressed = enc.finish().unwrap();
        let b = block(0, b"zlib", &compressed, payload.len(), 0, 0);
        let img = load_bytes(asdf_bytes(TREE_2X3, &[b])).unwrap();
        assert_eq!(img.data.iter().sum::<f32>(), 42.0);
    }

    #[test]
    fn unused_block_with_unknown_compression_does_not_block_load() {
        let tree = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [2, 3]\nother: !core/ndarray-1.0.0\n  source: 1\n  datatype: float32\n  byteorder: little\n  shape: [2]\n";
        let good = raw_block(&f32_le(&[1., 2., 3., 4., 5., 6.]));
        let odd = block(0, b"blsc", &[1, 2, 3, 4, 5, 6, 7, 8], 8, 0, 0);
        let file = AsdfFile::from_bytes(asdf_bytes(tree, &[good, odd])).unwrap();
        assert_eq!(file.blocks.len(), 2);
        assert!(matches!(file.block_data(1), Err(AsdfError::UnsupportedCompression(_))));
        assert_eq!(AsdfImage::from_file(&file).unwrap().data.iter().sum::<f32>(), 21.0);
    }

    #[test]
    fn uint16_big_endian_decoded() {
        let tree = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: uint16\n  byteorder: big\n  shape: [1, 3]\n";
        let payload: Vec<u8> = [1u16, 256, 65535].iter().flat_map(|v| v.to_be_bytes()).collect();
        let img = load_bytes(asdf_bytes(tree, &[raw_block(&payload)])).unwrap();
        assert_eq!(img.data, vec![1., 256., 65535.]);
    }

    #[test]
    fn cube_keeps_every_plane_and_reports_the_plane_count() {
        let tree = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [5, 2, 6]\n";
        let values: Vec<f32> = (0..60).map(|i| i as f32).collect();
        let img = load_bytes(asdf_bytes(tree, &[raw_block(&f32_le(&values))])).unwrap();
        assert_eq!((img.height, img.width), (2, 6));
        assert_eq!(img.plane_count(), 5);
        assert_eq!(img.shape, vec![5, 2, 6]);
        assert_eq!(img.data, values, "every plane must survive the load");

        let first = img.plane(0).expect("plane 1 of 5");
        assert_eq!(first.dim(), (2, 6));
        assert_eq!(first.as_slice().unwrap(), &values[..12]);
        let last = img.plane(4).expect("plane 5 of 5");
        assert_eq!(last.as_slice().unwrap(), &values[48..]);
        assert!(img.plane(5).is_none(), "there is no sixth plane");

        let owned = img.into_plane(3).expect("plane 4 of 5");
        assert_eq!(owned.as_slice().unwrap(), &values[36..48]);
    }

    #[test]
    fn into_plane_declines_an_image_without_pixels() {
        let tree = "meta:\n  instrument: {name: WFI}\n";
        let file = AsdfFile::from_bytes(asdf_bytes(tree, &[])).unwrap();
        let empty = AsdfImage::from_file(&file).unwrap();
        assert!(!empty.has_image());
        assert!(empty.plane(0).is_none());
        assert!(empty.into_plane(0).is_none());
    }

    #[test]
    fn plane_geometry_counts_planes_for_every_layout() {
        let single = |height, width| PlaneGeometry { height, width, plane_count: 1 };
        assert_eq!(plane_geometry(&[2, 6]), single(2, 6));
        assert_eq!(plane_geometry(&[1, 2, 6]), single(2, 6));
        assert_eq!(plane_geometry(&[8, 6, 1]), single(8, 6));
        assert_eq!(plane_geometry(&[1, 1, 2, 6]), single(2, 6));
        assert_eq!(plane_geometry(&[5, 2, 6]), PlaneGeometry { height: 2, width: 6, plane_count: 5 });
        assert_eq!(plane_geometry(&[3, 2, 6]), PlaneGeometry { height: 2, width: 6, plane_count: 3 });
        assert_eq!(plane_geometry(&[8, 6, 3]), PlaneGeometry { height: 8, width: 6, plane_count: 3 });
        assert_eq!(plane_geometry(&[7, 2, 2, 6]), PlaneGeometry { height: 2, width: 6, plane_count: 14 });
        assert_eq!(shape_label(&[6, 4096, 4096]), "6x4096x4096");
    }

    #[test]
    fn load_array_int_takes_the_first_plane_of_a_planar_cube_and_declines_interleaved() {
        let tree = "dq: !core/ndarray-1.0.0\n  source: 0\n  datatype: uint32\n  byteorder: little\n  shape: [5, 2, 6]\nflat: !core/ndarray-1.0.0\n  source: 0\n  datatype: uint32\n  byteorder: little\n  shape: [2, 3]\nwoven: !core/ndarray-1.0.0\n  source: 0\n  datatype: uint32\n  byteorder: little\n  shape: [5, 4, 3]\n";
        let bits: Vec<u8> = (0..60u32).flat_map(|v| v.to_le_bytes()).collect();
        let file = AsdfFile::from_bytes(asdf_bytes(tree, &[raw_block(&bits)])).unwrap();

        let dq = AsdfImage::load_array_int(&file, "dq").unwrap().unwrap();
        assert_eq!(dq.bits.dim(), (2, 6));
        assert_eq!(dq.bits.as_slice().unwrap(), &(0..12u32).collect::<Vec<u32>>()[..]);

        let flat = AsdfImage::load_array_int(&file, "flat").unwrap().unwrap();
        assert_eq!(flat.bits.dim(), (2, 3));

        assert!(
            AsdfImage::load_array_int(&file, "woven").unwrap().is_none(),
            "an interleaved multi-channel mask cannot be sliced without de-interleaving"
        );
    }

    #[test]
    fn roman_layout_resolves_data_and_gwcs_and_tagged_meta() {
        let tree = "roman: !<asdf://stsci.edu/datamodels/roman/tags/wfi_image-1.0.0>\n  meta:\n    exposure: !<asdf://stsci.edu/datamodels/roman/tags/exposure-1.0.0>\n      exposure_time: 107.0\n    instrument: {name: WFI, detector: WFI01}\n    wcs:\n      steps:\n        - transform: {transform_type: Shift, offset: -2043.5}\n        - transform: {transform_type: Shift, offset: -2043.5}\n        - frame: {name: world}\n  data: !core/ndarray-1.0.0\n    source: 0\n    datatype: float32\n    byteorder: little\n    shape: [2, 3]\n";
        let img = load_bytes(asdf_bytes(tree, &[raw_block(&f32_le(&[1.; 6]))])).unwrap();
        assert_eq!(img.metadata.get("ASDF_DATA_KEY").map(String::as_str), Some("roman.data"));
        assert_eq!(img.metadata.get("roman.meta.exposure.exposure_time").map(String::as_str), Some("107.0"));
        assert_eq!(img.metadata.get("roman.meta.instrument.name").map(String::as_str), Some("WFI"));
        let wcs = img.wcs.expect("roman gwcs resolved");
        assert_eq!(wcs.crpix, [2044.5, 2044.5]);
    }

    #[test]
    fn truncated_block_is_error() {
        let mut bytes = asdf_bytes(TREE_2X3, &[raw_block(&f32_le(&[1.; 6]))]);
        bytes.truncate(bytes.len() - 5);
        assert!(matches!(AsdfFile::from_bytes(bytes), Err(AsdfError::BlockTruncated)));
    }

    #[test]
    fn short_block_is_shape_mismatch() {
        let bytes = asdf_bytes(TREE_2X3, &[raw_block(&f32_le(&[1.; 4]))]);
        assert!(matches!(load_bytes(bytes), Err(AsdfError::ShapeMismatch { got: 4, expected: 6 })));
    }

    #[test]
    fn asdf_reference_suite() {
        let dir = match fixtures_dir() {
            Some(d) => d,
            None => return,
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
            if !path.exists() {
                eprintln!("SKIP {} (not bundled)", name);
                continue;
            }
            match AsdfImage::load(&path) {
                Ok(img) => {
                    if no_image.contains(&name) && img.has_image() {
                        eprintln!(
                            "FAIL {} (expected no image, got {}x{})",
                            name, img.width, img.height
                        );
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

        assert!(
            failures.is_empty(),
            "ASDF reference suite failures: {:?}",
            failures
        );
    }

    #[test]
    fn asdf_view_offset_strides() {
        let dir = match fixtures_dir() {
            Some(d) => d,
            None => return,
        };

        let path = dir.join("12_shared_view.asdf");
        if !path.exists() {
            return;
        }
        let asdf = AsdfFile::open(&path).expect("open 12_shared_view.asdf");

        let cases: &[(&str, f64)] = &[
            ("full", 4950.0),
            ("first_half", 1225.0),
            ("strided", 2450.0),
        ];

        let mut failures = Vec::new();
        for (key, expected) in cases {
            let node = asdf.tree.get(*key).expect("array node present");
            let meta = NdArrayMeta::from_yaml(node).expect("parse ndarray meta");
            let index = match meta.source {
                ArraySource::Block(i) => i,
                ArraySource::Inline(_) => panic!("fixture arrays are block-backed"),
            };
            let block = asdf.block_data(index).expect("source block present");
            let raw = AsdfImage::gather_array_bytes(&block, &meta);
            let pixels = AsdfImage::to_f32_pixels(&raw, &meta);
            let sum: f64 = pixels.iter().map(|&v| v as f64).sum();
            if (sum - expected).abs() > 1e-6 || pixels.len() != meta.element_count() {
                eprintln!(
                    "FAIL view {} (len {} sum {} expected {})",
                    key,
                    pixels.len(),
                    sum,
                    expected
                );
                failures.push(*key);
            } else {
                eprintln!("PASS view {} (len {} sum {})", key, pixels.len(), sum);
            }
        }

        assert!(failures.is_empty(), "ASDF view failures: {:?}", failures);
    }

    const MULTI_TREE: &str = "meta:\n  telescope: JWST\ndata: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [2, 3]\ndq: !core/ndarray-1.0.0\n  source: 1\n  datatype: uint32\n  byteorder: big\n  shape: [2, 3]\nerr: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [2, 3]\nwave: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [6]\nroman:\n  meta:\n    instrument: {name: WFI}\n  dq: !core/ndarray-1.0.0\n    source: 2\n    datatype: int16\n    byteorder: little\n    shape: [2, 3]\n";

    fn multi_file() -> AsdfFile {
        let dq: Vec<u8> = [0u32, 0x8000_0001, 3, 0, 0, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let rdq: Vec<u8> = [-5i16, 7, 0, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let blocks = vec![raw_block(&f32_le(&[1., 2., 3., 4., 5., 6.])), raw_block(&dq), raw_block(&rdq)];
        AsdfFile::from_bytes(asdf_bytes(MULTI_TREE, &blocks)).unwrap()
    }

    #[test]
    fn list_arrays_reports_rank2_arrays_in_tree_order() {
        let file = multi_file();
        let arrays = list_arrays(&file);
        let keys: Vec<&str> = arrays.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, vec!["data", "dq", "err", "roman.dq"]);
        assert_eq!(arrays[1].bitpix, 32);
        assert_eq!(arrays[1].dtype, DType::UInt32);
        assert_eq!(arrays[1].shape, vec![2, 3]);
        assert_eq!(arrays[3].bitpix, 16);
        assert_eq!(arrays[0].bitpix, -32);
    }

    #[test]
    fn load_array_by_dotted_key() {
        let file = multi_file();
        let dq = AsdfImage::load_array(&file, "dq").unwrap();
        assert_eq!((dq.height, dq.width), (2, 3));
        assert_eq!(dq.data[1], 0x8000_0001u32 as f32);
        assert_eq!(dq.metadata.get("ASDF_DATA_KEY").map(String::as_str), Some("dq"));
        let rdq = AsdfImage::load_array(&file, "roman.dq").unwrap();
        assert_eq!(rdq.data, vec![-5., 7., 0., 0., 0., 0.]);
        assert_eq!(rdq.metadata.get("ASDF_DATA_KEY").map(String::as_str), Some("roman.dq"));
        assert!(matches!(AsdfImage::load_array(&file, "nope"), Err(AsdfError::MissingField(k)) if k == "nope"));
        assert!(matches!(AsdfImage::load_array(&file, "meta"), Err(AsdfError::MissingField(_))));
    }

    #[test]
    fn load_array_int_decodes_integer_dtypes_losslessly() {
        let file = multi_file();
        let dq = AsdfImage::load_array_int(&file, "dq").unwrap().unwrap();
        assert!(!dq.signed);
        assert_eq!(dq.bits.dim(), (2, 3));
        assert_eq!(dq.bits[[0, 1]], 0x8000_0001);
        assert_eq!(dq.value_at(0, 1), 2147483649);
        assert_eq!(dq.bits[[0, 2]], 3);

        let rdq = AsdfImage::load_array_int(&file, "roman.dq").unwrap().unwrap();
        assert!(rdq.signed);
        assert_eq!(rdq.value_at(0, 0), -5);
        assert_eq!(rdq.value_at(0, 1), 7);

        assert!(AsdfImage::load_array_int(&file, "data").unwrap().is_none());
        assert!(matches!(AsdfImage::load_array_int(&file, "missing"), Err(AsdfError::MissingField(_))));
    }

    #[test]
    fn load_array_int_little_endian_uint32_and_bool8() {
        let tree = "dq: !core/ndarray-1.0.0\n  source: 0\n  datatype: uint32\n  byteorder: little\n  shape: [1, 2]\nmask: !core/ndarray-1.0.0\n  source: 1\n  datatype: bool8\n  shape: [1, 3]\n";
        let dq: Vec<u8> = [0x8000_0001u32, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
        let file = AsdfFile::from_bytes(asdf_bytes(tree, &[raw_block(&dq), raw_block(&[0, 1, 5])])).unwrap();
        let p = AsdfImage::load_array_int(&file, "dq").unwrap().unwrap();
        assert_eq!(p.bits[[0, 0]], 0x8000_0001);
        assert_eq!(p.bits[[0, 1]], 2);
        let m = AsdfImage::load_array_int(&file, "mask").unwrap().unwrap();
        assert!(!m.signed);
        assert_eq!(m.bits.as_slice().unwrap(), &[0, 1, 1]);
    }

    #[test]
    fn load_array_int_inline_source_casts_values() {
        let tree = "dq: !core/ndarray-1.0.0\n  data: [[0, 3], [1, 0]]\n  datatype: uint32\n";
        let file = AsdfFile::from_bytes(asdf_bytes(tree, &[])).unwrap();
        let p = AsdfImage::load_array_int(&file, "dq").unwrap().unwrap();
        assert_eq!(p.bits.as_slice().unwrap(), &[0, 3, 1, 0]);
    }

    #[test]
    fn bitpix_for_dtype_table() {
        assert_eq!(bitpix_for_dtype(&DType::Int8), 8);
        assert_eq!(bitpix_for_dtype(&DType::UInt8), 8);
        assert_eq!(bitpix_for_dtype(&DType::Bool8), 8);
        assert_eq!(bitpix_for_dtype(&DType::Int16), 16);
        assert_eq!(bitpix_for_dtype(&DType::UInt16), 16);
        assert_eq!(bitpix_for_dtype(&DType::Int32), 32);
        assert_eq!(bitpix_for_dtype(&DType::UInt32), 32);
        assert_eq!(bitpix_for_dtype(&DType::Int64), 64);
        assert_eq!(bitpix_for_dtype(&DType::UInt64), 64);
        assert_eq!(bitpix_for_dtype(&DType::Float32), -32);
        assert_eq!(bitpix_for_dtype(&DType::Float64), -64);
        assert_eq!(bitpix_for_dtype(&DType::Complex64), 0);
        assert_eq!(bitpix_for_dtype(&DType::Complex128), 0);
    }

    #[test]
    fn array_exists_checks_dotted_keys() {
        let file = multi_file();
        assert!(AsdfImage::array_exists(&file.tree, "dq"));
        assert!(AsdfImage::array_exists(&file.tree, "roman.dq"));
        assert!(AsdfImage::array_exists(&file.tree, "wave"));
        assert!(!AsdfImage::array_exists(&file.tree, "roman.err"));
        assert!(!AsdfImage::array_exists(&file.tree, "meta"));
        assert!(!AsdfImage::array_exists(&file.tree, "nothing"));
    }

    #[test]
    fn to_f32_pixels_parallel_matches_sequential() {
        let n = 100_000usize;
        let mut raw = Vec::with_capacity(n * 4);
        for i in 0..n {
            raw.extend_from_slice(&(i as f32).to_le_bytes());
        }
        let meta = NdArrayMeta {
            source: ArraySource::Block(0),
            shape: vec![n],
            streamed_first_dim: false,
            dtype: DType::Float32,
            byteorder: ByteOrder::Little,
            offset: 0,
            strides: None,
        };
        let px = AsdfImage::to_f32_pixels(&raw, &meta);
        assert_eq!(px.len(), n);
        assert_eq!(px[0], 0.0);
        assert_eq!(px[1], 1.0);
        assert_eq!(px[n - 1], (n - 1) as f32);
        let sum: f64 = px.iter().map(|&v| v as f64).sum();
        let expected: f64 = (0..n).map(|i| i as f64).sum();
        assert_eq!(sum, expected);
    }
}
