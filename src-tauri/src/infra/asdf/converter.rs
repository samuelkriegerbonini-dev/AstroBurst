use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;
use serde_yaml::Value;

use super::parser::{AsdfError, AsdfFile};
use super::tree::{untag, ArraySource, ByteOrder, DType, NdArrayMeta, WcsInfo};

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
            data: Vec::new(),
            wcs,
            metadata,
        }
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

        let (height, width, channels, layout) = match Self::interpret_shape(&meta.shape) {
            Ok(dims) => dims,
            Err(_) => return Ok(Self::empty(wcs, Self::extract_metadata(&asdf.tree, key))),
        };

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
            data,
            wcs,
            metadata: Self::extract_metadata(&asdf.tree, key),
        })
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

    pub fn into_array2(self) -> ndarray::Array2<f32> {
        if !self.has_image() {
            return ndarray::Array2::zeros((0, 0));
        }
        let (height, width) = (self.height, self.width);
        let mut plane = self.data;
        plane.truncate(width * height);
        ndarray::Array2::from_shape_vec((height, width), plane)
            .unwrap_or_else(|_| ndarray::Array2::zeros((height, width)))
    }

    fn find_data_array(tree: &Value) -> Result<Option<(String, NdArrayMeta)>, AsdfError> {
        let candidates = ["data", "sci", "SCI", "science", "image"];

        if let Some(mapping) = tree.as_mapping() {
            for key in &candidates {
                if let Some(node) = mapping.get(Value::String(key.to_string())) {
                    if let Some(meta) = Self::try_meta(node, true)? {
                        return Ok(Some((key.to_string(), meta)));
                    }
                    if let Some(data_node) = node.get("data") {
                        if let Some(meta) = Self::try_meta(data_node, true)? {
                            return Ok(Some((key.to_string(), meta)));
                        }
                    }
                }
            }
        }

        if let Some(roman) = tree.get("roman") {
            let roman_paths = ["data", "science", "sci"];
            for rp in &roman_paths {
                if let Some(node) = roman.get(*rp) {
                    if let Some(meta) = Self::try_meta(node, true)? {
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

    fn interpret_shape(shape: &[usize]) -> Result<(usize, usize, usize, PixelLayout), AsdfError> {
        match shape.len() {
            0 => Ok((1, 1, 1, PixelLayout::Planar)),
            1 => Ok((1, shape[0], 1, PixelLayout::Planar)),
            2 => Ok((shape[0], shape[1], 1, PixelLayout::Planar)),
            3 if shape[0] <= 4 => Ok((shape[1], shape[2], shape[0], PixelLayout::Planar)),
            3 if shape[2] <= 4 => Ok((shape[0], shape[1], shape[2], PixelLayout::Interleaved)),
            3 => Ok((shape[1], shape[2], shape[0], PixelLayout::Planar)),
            n => Err(AsdfError::UnsupportedRank(n)),
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
    fn cube_selects_first_plane_and_reports_channels() {
        let tree = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [5, 2, 6]\n";
        let values: Vec<f32> = (0..60).map(|i| i as f32).collect();
        let img = load_bytes(asdf_bytes(tree, &[raw_block(&f32_le(&values))])).unwrap();
        assert_eq!((img.height, img.width, img.channels), (2, 6, 5));
        let arr = img.into_array2();
        assert_eq!(arr.dim(), (2, 6));
        assert_eq!(arr.as_slice().unwrap(), &values[..12]);
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
