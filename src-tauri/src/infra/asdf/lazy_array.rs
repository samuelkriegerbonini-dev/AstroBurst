use std::borrow::Cow;
use std::ops::Deref;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ndarray::Array2;

use super::converter::{plane_geometry, AsdfImage, PlaneGeometry};
use super::parser::{AsdfError, AsdfFile};
use super::tree::{ArraySource, NdArrayMeta};

enum BlockBytes<'a> {
    InFile(Cow<'a, [u8]>),
    Held(Arc<Vec<u8>>),
}

impl Deref for BlockBytes<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            BlockBytes::InFile(bytes) => bytes,
            BlockBytes::Held(bytes) => bytes.as_slice(),
        }
    }
}

#[derive(Debug)]
pub struct AsdfLazyArray {
    file: AsdfFile,
    key: String,
    meta: NdArrayMeta,
    geometry: PlaneGeometry,
    decompressed: Mutex<Option<Arc<Vec<u8>>>>,
}

impl AsdfLazyArray {
    pub fn open<P: AsRef<Path>>(path: P, key: &str) -> Result<Self, AsdfError> {
        Self::from_file(AsdfFile::open(path)?, key)
    }

    pub fn from_file(file: AsdfFile, key: &str) -> Result<Self, AsdfError> {
        let meta = AsdfImage::array_meta(&file, key)?;
        let geometry = plane_geometry(&meta.shape);
        Ok(Self { file, key: key.to_string(), meta, geometry, decompressed: Mutex::new(None) })
    }

    pub fn file(&self) -> &AsdfFile {
        &self.file
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn shape(&self) -> &[usize] {
        &self.meta.shape
    }

    pub fn width(&self) -> usize {
        self.geometry.width
    }

    pub fn height(&self) -> usize {
        self.geometry.height
    }

    pub fn plane_count(&self) -> usize {
        self.geometry.plane_count
    }

    pub fn holds_decompressed_bytes(&self) -> bool {
        self.decompressed.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    fn block_bytes(&self, index: usize) -> Result<BlockBytes<'_>, AsdfError> {
        if !self.file.block_is_compressed(index) {
            return self.file.block_data(index).map(BlockBytes::InFile);
        }
        let mut held = self.decompressed.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(bytes) = held.as_ref() {
            return Ok(BlockBytes::Held(Arc::clone(bytes)));
        }
        let bytes = Arc::new(self.file.block_data(index)?.into_owned());
        *held = Some(Arc::clone(&bytes));
        Ok(BlockBytes::Held(bytes))
    }

    pub fn plane(&self, index: usize) -> Result<Array2<f32>, AsdfError> {
        let (height, width) = (self.geometry.height, self.geometry.width);
        let expected = width * height;
        let mut pixels = match &self.meta.source {
            ArraySource::Inline(values) => AsdfImage::inline_plane(values, &self.meta.shape, index)?,
            ArraySource::Block(block) => {
                let bytes = self.block_bytes(*block)?;
                let raw = AsdfImage::gather_plane_bytes(&bytes, &self.meta, index)?;
                AsdfImage::to_f32_pixels(&raw, &self.meta)
            }
        };
        if pixels.len() < expected {
            return Err(AsdfError::ShapeMismatch { got: pixels.len(), expected });
        }
        pixels.truncate(expected);
        Array2::from_shape_vec((height, width), pixels).map_err(|_| AsdfError::ShapeMismatch { got: expected, expected })
    }

    pub fn pixel_series(&self, x: usize, y: usize) -> Result<Option<Vec<f32>>, AsdfError> {
        if x >= self.geometry.width || y >= self.geometry.height {
            return Ok(None);
        }
        let planes = self.geometry.plane_count;
        let series = match &self.meta.source {
            ArraySource::Inline(values) => (0..planes)
                .map(|plane| AsdfImage::inline_sample(values, &self.meta.shape, plane, y, x))
                .collect::<Result<Vec<f32>, AsdfError>>()?,
            ArraySource::Block(block) => {
                let bytes = self.block_bytes(*block)?;
                let element = self.meta.byte_size_per_element().max(1);
                let mut raw = Vec::with_capacity(planes * element);
                for plane in 0..planes {
                    raw.extend_from_slice(AsdfImage::sample_bytes(&bytes, &self.meta, plane, y, x)?);
                }
                AsdfImage::to_f32_pixels(&raw, &self.meta)
            }
        };
        Ok(Some(series))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn block(compression: &[u8; 4], stored: &[u8], data_size: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0xd3, 0x42, 0x4c, 0x4b]);
        out.extend_from_slice(&48u16.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(compression);
        out.extend_from_slice(&(stored.len() as u64).to_be_bytes());
        out.extend_from_slice(&(stored.len() as u64).to_be_bytes());
        out.extend_from_slice(&(data_size as u64).to_be_bytes());
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(stored);
        out
    }

    fn zlib(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    const RESULTANTS: usize = 6;
    const HEIGHT: usize = 5;
    const WIDTH: usize = 7;

    fn ramp_value(index: usize) -> u16 {
        (index * 37 % 60_000) as u16
    }

    fn ramp_bytes() -> Vec<u8> {
        (0..RESULTANTS * HEIGHT * WIDTH).flat_map(|i| ramp_value(i).to_be_bytes()).collect()
    }

    fn ramp_tree(extra: &str) -> String {
        format!(
            "roman:\n  meta:\n    exposure: {{frame_time: 3.04}}\n  data: !core/ndarray-1.0.0\n    source: 0\n    datatype: uint16\n    byteorder: big\n    shape: [{RESULTANTS}, {HEIGHT}, {WIDTH}]\n{extra}"
        )
    }

    fn check_against_the_full_decode(lazy: &AsdfLazyArray, file: &AsdfFile, key: &str) {
        let full = AsdfImage::load_array(file, key).unwrap();
        assert_eq!((lazy.width(), lazy.height(), lazy.plane_count()), (full.width, full.height, full.plane_count()));
        for index in 0..full.plane_count() {
            assert_eq!(lazy.plane(index).unwrap(), full.plane(index).unwrap(), "{key} plane {index}");
        }
        assert!(lazy.plane(full.plane_count()).is_err(), "{key}: there is no plane past the last");
        let plane_len = full.width * full.height;
        for (x, y) in [(0, 0), (full.width - 1, 0), (0, full.height - 1), (full.width / 2, full.height / 2), (full.width - 1, full.height - 1)] {
            let series = lazy.pixel_series(x, y).unwrap().expect("inside the array");
            let expected: Vec<f32> = (0..full.plane_count()).map(|g| full.data[g * plane_len + y * full.width + x]).collect();
            assert_eq!(series, expected, "{key} pixel ({x}, {y})");
        }
        assert_eq!(lazy.pixel_series(full.width, 0).unwrap(), None);
        assert_eq!(lazy.pixel_series(0, full.height).unwrap(), None);
    }

    #[test]
    fn a_plain_block_serves_frames_and_series_without_a_full_decode() {
        let bytes = asdf_bytes(&ramp_tree(""), &[block(b"\0\0\0\0", &ramp_bytes(), ramp_bytes().len())]);
        let reference = AsdfFile::from_bytes(bytes.clone()).unwrap();
        let lazy = AsdfLazyArray::from_file(AsdfFile::from_bytes(bytes).unwrap(), "roman.data").unwrap();
        assert_eq!(lazy.shape(), &[RESULTANTS, HEIGHT, WIDTH]);
        assert_eq!(lazy.key(), "roman.data");
        check_against_the_full_decode(&lazy, &reference, "roman.data");
        let third = lazy.plane(2).unwrap();
        assert_eq!(third[[1, 3]], f32::from(ramp_value(2 * HEIGHT * WIDTH + WIDTH + 3)));
        assert_eq!(lazy.pixel_series(3, 1).unwrap().unwrap()[4], f32::from(ramp_value(4 * HEIGHT * WIDTH + WIDTH + 3)));
        assert!(!lazy.holds_decompressed_bytes(), "a block stored plain is read in place");
        assert_eq!(lazy.file().tree["roman"]["meta"]["exposure"]["frame_time"].as_f64(), Some(3.04), "the parsed tree stays with the reader");
    }

    #[test]
    fn a_compressed_block_is_decompressed_once_and_kept_in_its_stored_type() {
        let raw = ramp_bytes();
        let bytes = asdf_bytes(&ramp_tree(""), &[block(b"zlib", &zlib(&raw), raw.len())]);
        let reference = AsdfFile::from_bytes(bytes.clone()).unwrap();
        let lazy = AsdfLazyArray::from_file(AsdfFile::from_bytes(bytes).unwrap(), "roman.data").unwrap();
        assert!(!lazy.holds_decompressed_bytes(), "nothing is decompressed before the first request");
        check_against_the_full_decode(&lazy, &reference, "roman.data");
        assert!(lazy.holds_decompressed_bytes());
        let held = lazy.decompressed.lock().unwrap().as_ref().map(|b| b.len());
        assert_eq!(held, Some(raw.len()), "the held bytes are the stored uint16 samples, half the size of an f32 cube");
    }

    #[test]
    fn inline_interleaved_and_strided_arrays_are_served_too() {
        let tree = "inline: !core/ndarray-1.0.0\n  data: [[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]], [[13, 14], [15, 16]], [[17, 18], [19, 20]]]\n  datatype: float32\nwoven: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [5, 4, 3]\nflipped: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [5, 2, 6]\n  offset: 192\n  strides: [-48, 24, 4]\nflat: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [6, 10]\n";
        let values: Vec<u8> = (0..60).flat_map(|i| (i as f32).to_le_bytes()).collect();
        let bytes = asdf_bytes(tree, &[block(b"\0\0\0\0", &values, values.len())]);
        let reference = AsdfFile::from_bytes(bytes.clone()).unwrap();
        for key in ["inline", "woven", "flipped", "flat"] {
            let lazy = AsdfLazyArray::from_file(AsdfFile::from_bytes(bytes.clone()).unwrap(), key).unwrap();
            check_against_the_full_decode(&lazy, &reference, key);
        }
        assert!(matches!(
            AsdfLazyArray::from_file(AsdfFile::from_bytes(bytes).unwrap(), "missing"),
            Err(AsdfError::MissingField(key)) if key == "missing"
        ));
    }

    #[test]
    fn a_block_shorter_than_the_declared_array_is_an_error_not_a_panic() {
        let short = &ramp_bytes()[..RESULTANTS * HEIGHT * WIDTH];
        let bytes = asdf_bytes(&ramp_tree(""), &[block(b"\0\0\0\0", short, short.len())]);
        let lazy = AsdfLazyArray::from_file(AsdfFile::from_bytes(bytes).unwrap(), "roman.data").unwrap();
        assert!(lazy.plane(0).is_ok(), "the planes the block does hold are served");
        assert!(matches!(lazy.plane(RESULTANTS - 1), Err(AsdfError::ShapeMismatch { .. })));
        assert!(matches!(lazy.pixel_series(0, 0), Err(AsdfError::ShapeMismatch { .. })));
    }

    #[test]
    fn opening_from_disk_keeps_the_file_mapped_for_later_requests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ramp.asdf");
        let raw = ramp_bytes();
        std::fs::write(&path, asdf_bytes(&ramp_tree(""), &[block(b"\0\0\0\0", &raw, raw.len())])).unwrap();
        let lazy = AsdfLazyArray::open(&path, "roman.data").unwrap();
        let first = lazy.pixel_series(6, 4).unwrap().unwrap();
        let again = lazy.pixel_series(6, 4).unwrap().unwrap();
        assert_eq!(first, again);
        assert_eq!(first.len(), RESULTANTS);
        assert_eq!(first[5], f32::from(ramp_value(5 * HEIGHT * WIDTH + 4 * WIDTH + 6)));
    }
}
