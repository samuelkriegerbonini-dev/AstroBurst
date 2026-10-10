use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use memmap2::Mmap;
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::analysis::aperture::{annulus, circular_aperture, total_weight, AperturePixel};
use crate::core::astrometry::spectral::SpectralAxis;
use crate::core::cube::eager::DISPLAY_ASINH_ALPHA;
use crate::core::imaging::region::RegionShape;
use crate::core::imaging::stats::is_valid_pixel;
use crate::core::ramp::info::{ramp_info, RampInfo};
use crate::infra::fits::compress::is_compressed_image_hdu;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::file_bytes::{io_mode, prefer_mmap, IoMode};
use crate::infra::fits::reader::{create_mmap_random, decode_pixels_blank, parse_header_at, read_header_blocks, ParsedHdu};
use crate::infra::fits::table::FileTabTables;
use crate::infra::render::grayscale::render_stretched_8bit;
use crate::math::median::f32_cmp;
use crate::math::{exact_median_mut, sigma_clipped_stats};
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::{HduHeader, ImageRef, PlaneSelector};

#[derive(Debug, Clone)]
pub struct CubeGeometry {
    pub naxis1: usize,
    pub naxis2: usize,
    pub naxis3: usize,
    pub naxis4: usize,
    pub depth: usize,
    pub bitpix: i64,
    pub bytes_per_pixel: usize,
    pub bzero: f64,
    pub bscale: f64,
    pub blank: Option<i64>,
    pub data_offset: usize,
    pub frame_bytes: usize,
}

pub struct LruFrameCache {
    entries: HashMap<usize, CacheEntry>,
    max_bytes: usize,
    current_bytes: usize,
    access_counter: u64,
}

struct CacheEntry {
    frame: Arc<Array2<f32>>,
    bytes: usize,
    last_access: u64,
}

impl LruFrameCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_bytes,
            current_bytes: 0,
            access_counter: 0,
        }
    }

    pub fn get(&mut self, frame_idx: usize) -> Option<Arc<Array2<f32>>> {
        if let Some(entry) = self.entries.get_mut(&frame_idx) {
            self.access_counter += 1;
            entry.last_access = self.access_counter;
            Some(Arc::clone(&entry.frame))
        } else {
            None
        }
    }

    pub fn insert(&mut self, frame_idx: usize, frame: Arc<Array2<f32>>) {
        let bytes = frame.len() * std::mem::size_of::<f32>();
        if bytes > self.max_bytes {
            return;
        }
        if let Some(old) = self.entries.remove(&frame_idx) {
            self.current_bytes -= old.bytes;
        }
        while self.current_bytes + bytes > self.max_bytes {
            let victim = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(&k, _)| k);
            match victim {
                Some(k) => {
                    if let Some(old) = self.entries.remove(&k) {
                        self.current_bytes -= old.bytes;
                    }
                }
                None => break,
            }
        }
        self.access_counter += 1;
        self.current_bytes += bytes;
        self.entries.insert(
            frame_idx,
            CacheEntry {
                frame,
                bytes,
                last_access: self.access_counter,
            },
        );
    }
}

pub use crate::core::cube::eager::GlobalCubeStats;

pub fn normalize_frame_with_stats(data: &Array2<f32>, stats: &GlobalCubeStats) -> Array2<f32> {
    crate::core::cube::eager::normalize_with_global(data, stats)
}

const DEFAULT_CACHE_BYTES: usize = 256 << 20;
const BATCH_SIZE: usize = 32;
const STATS_SAMPLE_FRAMES: usize = 32;
const STATS_TARGET_SAMPLES: usize = 4_000_000;
const MAX_FITS_AXES: i64 = 999;
const SUPPORTED_BITPIX: [i64; 6] = [8, 16, 32, 64, -32, -64];

fn stats_sample_plan(naxis3: usize, npix: usize) -> (usize, usize) {
    let sample_frames = STATS_SAMPLE_FRAMES.min(naxis3).max(1);
    let step = naxis3.div_ceil(sample_frames);
    let frames = (0..naxis3).step_by(step.max(1)).count();
    let stride = ((frames * npix) / STATS_TARGET_SAMPLES).max(1);
    (step.max(1), stride)
}

const BACKGROUND_CLIP_SIGMA: f32 = 3.0;
const BACKGROUND_CLIP_ITERATIONS: usize = 5;
const RANGE_MEDIAN_SINGLE_PASS_BYTES: usize = 1 << 30;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApertureSpectrum {
    pub sum: Vec<f32>,
    pub mean: Vec<f32>,
    pub npix: f64,
    pub bg_per_pixel: Option<Vec<f32>>,
    pub n_bg: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollapseMode {
    Sum,
    Mean,
    Median,
}

impl CollapseMode {
    pub fn parse(name: &str) -> Result<CollapseMode> {
        match name.trim().to_lowercase().as_str() {
            "sum" => Ok(CollapseMode::Sum),
            "mean" | "average" => Ok(CollapseMode::Mean),
            "median" => Ok(CollapseMode::Median),
            other => bail!("unknown collapse mode '{}': use sum, mean or median", other),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CollapseMode::Sum => "sum",
            CollapseMode::Mean => "mean",
            CollapseMode::Median => "median",
        }
    }
}

fn lattice_pixels(shape: &RegionShape, rows: usize, cols: usize) -> Vec<AperturePixel> {
    let b = shape.bounds();
    let x0 = b.x0.max(0);
    let y0 = b.y0.max(0);
    let x1 = b.x1.min(cols as i64 - 1);
    let y1 = b.y1.min(rows as i64 - 1);
    let mut out = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            if shape.contains(x as f64, y as f64) {
                out.push(AperturePixel { y: y as usize, x: x as usize, weight: 1.0 });
            }
        }
    }
    out
}

pub fn region_pixels(
    shape: &RegionShape,
    rows: usize,
    cols: usize,
    subsamples: u8,
) -> Result<Vec<AperturePixel>> {
    shape.validate()?;
    match shape {
        RegionShape::Circle { x, y, r } => Ok(circular_aperture(rows, cols, *x, *y, *r, subsamples)),
        RegionShape::Annulus { x, y, r_inner, r_outer } => {
            Ok(annulus(rows, cols, *x, *y, *r_inner, *r_outer, subsamples))
        }
        RegionShape::Box { .. } | RegionShape::Ellipse { .. } | RegionShape::Polygon { .. } => {
            Ok(lattice_pixels(shape, rows, cols))
        }
        RegionShape::Line { .. } | RegionShape::Point { .. } => bail!(
            "{} regions have no area: use a circle, box, ellipse or polygon",
            shape.kind()
        ),
    }
}

fn row_span(pixels: &[AperturePixel]) -> Option<(usize, usize)> {
    let lo = pixels.iter().map(|p| p.y).min()?;
    let hi = pixels.iter().map(|p| p.y).max()?;
    Some((lo, hi))
}

fn clipped_background_level(mut samples: Vec<f32>) -> f32 {
    if samples.is_empty() {
        return f32::NAN;
    }
    sigma_clipped_stats(&mut samples, BACKGROUND_CLIP_SIGMA, BACKGROUND_CLIP_ITERATIONS).0 as f32
}

pub fn median_band_rows(range_len: usize, cols: usize, rows: usize, limit_bytes: usize) -> usize {
    let bytes_per_row = range_len.max(1).saturating_mul(cols.max(1)).saturating_mul(4);
    let total = bytes_per_row.saturating_mul(rows.max(1));
    if total <= limit_bytes {
        return rows.max(1);
    }
    (limit_bytes / bytes_per_row).clamp(1, rows.max(1))
}

fn usable_display_sigma(sigma: f32) -> bool {
    sigma.is_finite() && sigma > 0.0 && (DISPLAY_ASINH_ALPHA / sigma).is_finite()
}

fn display_sigma(samples: &[f32], median: f32, mad_sigma: f32) -> f32 {
    if usable_display_sigma(mad_sigma) {
        return mad_sigma;
    }
    if samples.len() < 2 {
        return 1.0;
    }
    let centre = median as f64;
    let variance = samples
        .iter()
        .map(|&v| {
            let d = v as f64 - centre;
            d * d
        })
        .sum::<f64>()
        / (samples.len() - 1) as f64;
    let std = variance.sqrt() as f32;
    if usable_display_sigma(std) {
        std
    } else {
        1.0
    }
}

fn median_or_nan(values: &mut Vec<f32>) -> f32 {
    if values.is_empty() {
        f32::NAN
    } else {
        exact_median_mut(values) as f32
    }
}

enum CubeData {
    Mapped { map: Mmap, _file: File },
    Positioned { file: File, len: usize },
}

#[cfg(unix)]
fn read_exact_at(file: &File, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        match file.seek_read(&mut buf[done..], offset + done as u64) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => done += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl CubeData {
    fn open(file: File, mode: IoMode) -> Result<CubeData> {
        if prefer_mmap(&file, mode) {
            let map = create_mmap_random(&file).context("mmap failed for lazy cube")?;
            return Ok(CubeData::Mapped { map, _file: file });
        }
        let len = usize::try_from(file.metadata().context("stat failed for lazy cube")?.len())
            .context("cube file is larger than the address space")?;
        Ok(CubeData::Positioned { file, len })
    }

    fn len(&self) -> usize {
        match self {
            CubeData::Mapped { map, .. } => map.len(),
            CubeData::Positioned { len, .. } => *len,
        }
    }

    fn header_at(&self, offset: usize) -> Result<ParsedHdu> {
        match self {
            CubeData::Mapped { map, .. } => parse_header_at(map, offset),
            CubeData::Positioned { file, .. } => read_header_blocks(file, offset),
        }
    }

    fn bytes(&self, start: usize, len: usize) -> Result<Cow<'_, [u8]>> {
        let end = start.checked_add(len).context("cube read range overflow")?;
        if end > self.len() {
            bail!("cube bytes [{}, {}) exceed the file size {}", start, end, self.len());
        }
        match self {
            CubeData::Mapped { map, .. } => map
                .get(start..end)
                .map(Cow::Borrowed)
                .with_context(|| format!("cube bytes [{}, {}) are not mapped", start, end)),
            CubeData::Positioned { file, .. } => {
                let mut buf = vec![0u8; len];
                read_exact_at(file, start as u64, &mut buf)
                    .with_context(|| format!("read of cube bytes [{}, {}) failed", start, end))?;
                Ok(Cow::Owned(buf))
            }
        }
    }
}

fn compressed_axes(header: &HduHeader) -> Option<i64> {
    is_compressed_image_hdu(header).then(|| header.get_i64("ZNAXIS").unwrap_or(0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CubeShape {
    pub naxis3: usize,
    pub naxis4: usize,
}

fn unsupported_axis(axis: i64, len: i64) -> String {
    format!(
        "has NAXIS{}={}: only cubes whose axes beyond the third have length 1 are supported",
        axis, len
    )
}

fn fourth_axis_is_a_ramp(header: &HduHeader, primary: Option<&HduHeader>, naxis3: i64, naxis4: i64) -> bool {
    let merged = primary.map_or_else(|| header.clone(), |p| p.merge_with(header));
    ramp_info(&merged).is_some_and(|r| r.ngroups as i64 == naxis3 && r.nints as i64 == naxis4)
}

fn cube_depth(header: &HduHeader, primary: Option<&HduHeader>) -> std::result::Result<CubeShape, String> {
    if let Some(znaxis) = compressed_axes(header) {
        return Err(if znaxis >= 3 {
            format!(
                "is a tile-compressed cube (ZNAXIS={}); compressed cubes are not supported, decompress the file (funpack) first",
                znaxis
            )
        } else {
            format!("is a tile-compressed image with ZNAXIS={}, not a data cube", znaxis)
        });
    }
    let naxis = header.get_i64("NAXIS").unwrap_or(0);
    if !(3..=MAX_FITS_AXES).contains(&naxis) {
        return Err(format!("has NAXIS={}, not a data cube", naxis));
    }
    let naxis3 = header.get_i64("NAXIS3").unwrap_or(0);
    let naxis4 = if naxis >= 4 { header.get_i64("NAXIS4").unwrap_or(1) } else { 1 };
    let integrations_are_a_ramp = naxis4 > 1 && fourth_axis_is_a_ramp(header, primary, naxis3, naxis4);
    if naxis3 <= 1 && !(naxis3 == 1 && integrations_are_a_ramp) {
        return Err(format!("has NAXIS3={}: a cube needs at least two planes", naxis3));
    }
    if naxis4 != 1 && !integrations_are_a_ramp {
        return Err(unsupported_axis(4, naxis4));
    }
    for axis in 5..=naxis {
        let len = header.get_i64(&format!("NAXIS{}", axis)).unwrap_or(1);
        if len != 1 {
            return Err(unsupported_axis(axis, len));
        }
    }
    let naxis3 = usize::try_from(naxis3).map_err(|_| format!("has NAXIS3={}, too large for this platform", naxis3))?;
    let naxis4 = usize::try_from(naxis4).map_err(|_| format!("has NAXIS4={}, too large for this platform", naxis4))?;
    Ok(CubeShape { naxis3, naxis4 })
}

fn positive_axis(header: &HduHeader, key: &str) -> Result<usize> {
    let value = header.get_i64(key).unwrap_or(0);
    if value <= 0 {
        bail!("Invalid cube dimension {}={}", key, value);
    }
    usize::try_from(value).with_context(|| format!("{}={} is too large", key, value))
}

struct FoundCube {
    index: usize,
    parsed: ParsedHdu,
    shape: CubeShape,
    primary: Option<HduHeader>,
}

const CONTEXT_EXTNAMES: [&str; 2] = ["CON", "CTX"];

fn is_resample_context_hdu(header: &HduHeader) -> bool {
    header
        .get("EXTNAME")
        .is_some_and(|name| CONTEXT_EXTNAMES.iter().any(|known| name.trim().eq_ignore_ascii_case(known)))
}

fn find_cube_hdu(data: &CubeData, plane: &PlaneSelector) -> Result<FoundCube> {
    let mut offset = 0usize;
    let mut index = 0usize;
    let mut compressed_cube: Option<String> = None;
    let mut primary: Option<HduHeader> = None;
    while offset < data.len() {
        let parsed = data
            .header_at(offset)
            .with_context(|| format!("Header parse failed in lazy cube at HDU {}", index))?;
        match plane {
            PlaneSelector::Hdu(n) if *n == index => {
                let shape = cube_depth(&parsed.header, primary.as_ref()).map_err(|reason| anyhow!("HDU {} {}", index, reason))?;
                return Ok(FoundCube { index, parsed, shape, primary });
            }
            PlaneSelector::Auto if is_resample_context_hdu(&parsed.header) => {}
            PlaneSelector::Auto => match cube_depth(&parsed.header, primary.as_ref()) {
                Ok(shape) => return Ok(FoundCube { index, parsed, shape, primary }),
                Err(reason) => {
                    if compressed_cube.is_none() && compressed_axes(&parsed.header).is_some_and(|n| n >= 3) {
                        compressed_cube = Some(format!("HDU {} {}", index, reason));
                    }
                }
            },
            _ => {}
        }
        if parsed.next_hdu_offset <= offset {
            bail!("HDU {} has an invalid data size", index);
        }
        if index == 0 {
            primary = Some(parsed.header);
        }
        offset = parsed.next_hdu_offset;
        index += 1;
    }
    match (plane, compressed_cube) {
        (PlaneSelector::Hdu(n), _) => bail!("HDU index {} out of range (file has {} HDUs)", n, index),
        (_, Some(reason)) => bail!("No uncompressed 3D data block found in FITS file: {}", reason),
        (_, None) => bail!("No 3D data block found in FITS file"),
    }
}

pub struct LazyCube {
    data: CubeData,
    _tmp: Option<tempfile::TempDir>,
    pub source_path: String,
    pub header: HduHeader,
    pub primary_header: Option<HduHeader>,
    pub geometry: CubeGeometry,
    pub hdu_index: usize,
    cache: Mutex<LruFrameCache>,
    stats: OnceLock<GlobalCubeStats>,
}

fn checked_cube_bytes(
    naxis1: usize,
    naxis2: usize,
    naxis3: usize,
    bytes_per_pixel: usize,
) -> Result<(usize, usize)> {
    let frame_bytes = naxis1
        .checked_mul(naxis2)
        .and_then(|v| v.checked_mul(bytes_per_pixel))
        .context("Cube frame size overflow")?;
    let total_bytes = frame_bytes
        .checked_mul(naxis3)
        .context("Cube data size overflow")?;
    Ok((frame_bytes, total_bytes))
}

impl LazyCube {
    pub fn open(key: &str) -> Result<Self> {
        Self::open_with_mode(key, io_mode())
    }

    pub fn open_with_mode(key: &str, mode: IoMode) -> Result<Self> {
        let reference = ImageRef::parse(key);
        if let PlaneSelector::Array(name) = &reference.plane {
            bail!("{} names the ASDF array '{}': only FITS cubes can be opened as a cube", key, name);
        }
        let (fits_path, tmp) = resolve_single_image(&reference.path)?;
        let file = File::open(&fits_path)
            .with_context(|| format!("Failed to open FITS file {}", reference.path))?;
        let data = CubeData::open(file, mode)?;
        let found = find_cube_hdu(&data, &reference.plane).with_context(|| format!("Cannot open {} as a cube", key))?;
        let header = found.parsed.header;
        let naxis1 = positive_axis(&header, "NAXIS1")?;
        let naxis2 = positive_axis(&header, "NAXIS2")?;
        let CubeShape { naxis3, naxis4 } = found.shape;
        let depth = naxis3.checked_mul(naxis4).context("Cube depth overflow")?;

        let bitpix = header.get_i64("BITPIX").context("Missing BITPIX")?;
        if !SUPPORTED_BITPIX.contains(&bitpix) {
            bail!("Unsupported BITPIX={}", bitpix);
        }
        let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
        let (frame_bytes, total_bytes) = checked_cube_bytes(naxis1, naxis2, depth, bytes_per_pixel)?;
        let data_offset = found.parsed.data_start;
        let data_end = data_offset
            .checked_add(total_bytes)
            .context("Cube data end overflow")?;
        if data_end > data.len() {
            bail!("Cube data [{}, {}) exceeds file size {}", data_offset, data_end, data.len());
        }

        let geometry = CubeGeometry {
            naxis1,
            naxis2,
            naxis3,
            naxis4,
            depth,
            bitpix,
            bytes_per_pixel,
            bzero: header.get_f64("BZERO").unwrap_or(0.0),
            bscale: header.get_f64("BSCALE").unwrap_or(1.0),
            blank: if bitpix > 0 { header.get_i64("BLANK") } else { None },
            data_offset,
            frame_bytes,
        };

        Ok(LazyCube {
            data,
            _tmp: tmp,
            source_path: fits_path.to_string_lossy().into_owned(),
            header,
            primary_header: found.primary,
            geometry,
            hdu_index: found.index,
            cache: Mutex::new(LruFrameCache::new(DEFAULT_CACHE_BYTES)),
            stats: OnceLock::new(),
        })
    }

    #[cfg(test)]
    fn is_memory_mapped(&self) -> bool {
        matches!(self.data, CubeData::Mapped { .. })
    }

    fn frame_cache(&self) -> MutexGuard<'_, LruFrameCache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn decode(&self, raw: &[u8]) -> Vec<f32> {
        let g = &self.geometry;
        decode_pixels_blank(raw, g.bitpix, g.bscale, g.bzero, g.blank)
    }

    fn check_frame(&self, z: usize) -> Result<()> {
        if z >= self.geometry.depth {
            bail!("Frame index {} out of range (depth={})", z, self.geometry.depth);
        }
        Ok(())
    }

    fn sample_at(&self, z: usize, pixel_offset_in_frame: usize) -> Result<f32> {
        let g = &self.geometry;
        let raw = self.data.bytes(g.data_offset + z * g.frame_bytes + pixel_offset_in_frame, g.bytes_per_pixel)?;
        Ok(self.decode(&raw).first().copied().unwrap_or(f32::NAN))
    }

    fn pixel_offset_in_frame(&self, y: usize, x: usize) -> Result<usize> {
        let g = &self.geometry;
        if y >= g.naxis2 || x >= g.naxis1 {
            bail!("Pixel ({}, {}) out of bounds", y, x);
        }
        Ok((y * g.naxis1 + x) * g.bytes_per_pixel)
    }

    pub fn get_frame(&self, z: usize) -> Result<Arc<Array2<f32>>> {
        self.check_frame(z)?;
        if let Some(frame) = self.frame_cache().get(z) {
            return Ok(frame);
        }
        let frame = Arc::new(self.frame_uncached(z)?);
        self.frame_cache().insert(z, Arc::clone(&frame));
        Ok(frame)
    }

    pub fn frame_uncached(&self, z: usize) -> Result<Array2<f32>> {
        let g = &self.geometry;
        let pixels = self.decode_rows(z, 0, g.naxis2)?;
        Array2::from_shape_vec((g.naxis2, g.naxis1), pixels)
            .context("Failed to reshape frame pixels")
    }

    pub fn extract_spectrum_at(&self, y: usize, x: usize) -> Result<Vec<f32>> {
        let pixel_offset_in_frame = self.pixel_offset_in_frame(y, x)?;
        (0..self.geometry.depth).map(|z| self.sample_at(z, pixel_offset_in_frame)).collect()
    }

    pub fn merged_header(&self) -> HduHeader {
        match &self.primary_header {
            Some(primary) => primary.merge_with(&self.header),
            None => self.header.clone(),
        }
    }

    pub fn ramp(&self) -> Option<RampInfo> {
        let g = &self.geometry;
        ramp_info(&self.merged_header()).filter(|r| r.ngroups == g.naxis3 && r.nints == g.naxis4)
    }

    pub fn extract_group_series(&self, y: usize, x: usize, integration: usize) -> Result<Vec<f32>> {
        let g = &self.geometry;
        if integration >= g.naxis4 {
            bail!("Integration {} out of range (nints={})", integration, g.naxis4);
        }
        let pixel_offset_in_frame = self.pixel_offset_in_frame(y, x)?;
        let z0 = integration * g.naxis3;
        (z0..z0 + g.naxis3).map(|z| self.sample_at(z, pixel_offset_in_frame)).collect()
    }

    pub fn check_channel_range(&self, z0: usize, z1: usize) -> Result<()> {
        if z0 > z1 {
            bail!("channel range start {} is after its end {}", z0, z1);
        }
        if z1 >= self.geometry.depth {
            bail!("channel {} is out of range (depth={})", z1, self.geometry.depth);
        }
        Ok(())
    }

    pub fn decode_rows(&self, z: usize, row_start: usize, row_count: usize) -> Result<Vec<f32>> {
        let g = &self.geometry;
        self.check_frame(z)?;
        if row_start.checked_add(row_count).map_or(true, |end| end > g.naxis2) {
            bail!("rows {}..+{} exceed the frame height {}", row_start, row_count, g.naxis2);
        }
        let row_bytes = g.naxis1 * g.bytes_per_pixel;
        let start = g.data_offset + z * g.frame_bytes + row_start * row_bytes;
        let raw = self.data.bytes(start, row_count * row_bytes)?;
        Ok(self.decode(&raw))
    }

    pub fn decode_row_band_batch(
        &self,
        z_start: usize,
        z_end: usize,
        row_start: usize,
        row_count: usize,
    ) -> Result<Vec<Vec<f32>>> {
        (z_start..z_end)
            .into_par_iter()
            .map(|z| self.decode_rows(z, row_start, row_count))
            .collect()
    }

    pub fn decode_frames(&self, z_start: usize, z_end: usize) -> Result<Vec<Vec<f32>>> {
        self.decode_row_band_batch(z_start, z_end, 0, self.geometry.naxis2)
    }

    pub fn spectral_axis(&self) -> std::result::Result<SpectralAxis, String> {
        let tables = FileTabTables { path: &self.source_path };
        crate::core::astrometry::spectral::spectral_axis_with(&self.header, self.geometry.naxis3, &tables)
    }

    pub fn extract_spectrum_aperture(
        &self,
        shape: &RegionShape,
        background: Option<&RegionShape>,
        subsamples: u8,
    ) -> Result<ApertureSpectrum> {
        let g = &self.geometry;
        let (rows, cols, depth) = (g.naxis2, g.naxis1, g.depth);
        let pixels = region_pixels(shape, rows, cols, subsamples)?;
        let Some((mut y0, mut y1)) = row_span(&pixels) else {
            bail!("{} region covers no image pixels", shape.kind());
        };
        let bg_pixels = match background {
            Some(bg) => {
                let found = region_pixels(bg, rows, cols, subsamples)?;
                if found.is_empty() {
                    bail!("background {} region covers no image pixels", bg.kind());
                }
                Some(found)
            }
            None => None,
        };
        if let Some((bg_y0, bg_y1)) = bg_pixels.as_deref().and_then(row_span) {
            y0 = y0.min(bg_y0);
            y1 = y1.max(bg_y1);
        }
        let row_count = y1 - y0 + 1;

        let mut sum = Vec::with_capacity(depth);
        let mut mean = Vec::with_capacity(depth);
        let mut bg_levels: Option<Vec<f32>> = bg_pixels.as_ref().map(|_| Vec::with_capacity(depth));

        for batch_start in (0..depth).step_by(BATCH_SIZE) {
            let batch_end = (batch_start + BATCH_SIZE).min(depth);
            let bands = self.decode_row_band_batch(batch_start, batch_end, y0, row_count)?;
            for band in &bands {
                let sample = |p: &AperturePixel| band[(p.y - y0) * cols + p.x];
                let mut acc = 0.0f64;
                let mut weight = 0.0f64;
                for p in &pixels {
                    let v = sample(p);
                    if v.is_finite() {
                        acc += p.weight as f64 * v as f64;
                        weight += p.weight as f64;
                    }
                }
                if let (Some(bg), Some(levels)) = (&bg_pixels, bg_levels.as_mut()) {
                    let samples: Vec<f32> = bg.iter().map(sample).filter(|v| v.is_finite()).collect();
                    let level = clipped_background_level(samples);
                    if level.is_finite() {
                        acc -= level as f64 * weight;
                    } else {
                        acc = f64::NAN;
                    }
                    levels.push(level);
                }
                if weight > 0.0 {
                    sum.push(acc as f32);
                    mean.push((acc / weight) as f32);
                } else {
                    sum.push(f32::NAN);
                    mean.push(f32::NAN);
                }
            }
        }

        Ok(ApertureSpectrum {
            sum,
            mean,
            npix: total_weight(&pixels),
            bg_per_pixel: bg_levels,
            n_bg: bg_pixels.map_or(0, |p| p.len()),
        })
    }

    pub fn collapse_range(&self, z0: usize, z1: usize, mode: CollapseMode) -> Result<Array2<f32>> {
        self.check_channel_range(z0, z1)?;
        match mode {
            CollapseMode::Sum | CollapseMode::Mean => self.collapse_range_linear(z0, z1, mode),
            CollapseMode::Median => {
                let g = &self.geometry;
                let band_rows = median_band_rows(z1 - z0 + 1, g.naxis1, g.naxis2, RANGE_MEDIAN_SINGLE_PASS_BYTES);
                self.collapse_range_median_banded(z0, z1, band_rows)
            }
        }
    }

    fn collapse_range_linear(&self, z0: usize, z1: usize, mode: CollapseMode) -> Result<Array2<f32>> {
        let g = &self.geometry;
        let (rows, cols) = (g.naxis2, g.naxis1);
        let npix = rows * cols;
        let mut sum = vec![0.0f64; npix];
        let mut count = vec![0u32; npix];

        for batch_start in (z0..=z1).step_by(BATCH_SIZE) {
            let batch_end = (batch_start + BATCH_SIZE).min(z1 + 1);
            let frames = self.decode_frames(batch_start, batch_end)?;
            for pixels in &frames {
                for (i, &v) in pixels.iter().enumerate().take(npix) {
                    if is_valid_pixel(v) {
                        sum[i] += v as f64;
                        count[i] += 1;
                    }
                }
            }
        }

        let data: Vec<f32> = sum
            .into_par_iter()
            .zip(count.into_par_iter())
            .map(|(s, c)| {
                if c == 0 {
                    f32::NAN
                } else if mode == CollapseMode::Mean {
                    (s / c as f64) as f32
                } else {
                    s as f32
                }
            })
            .collect();
        Array2::from_shape_vec((rows, cols), data).context("Failed to reshape collapsed range")
    }

    pub fn collapse_range_median_banded(&self, z0: usize, z1: usize, band_rows: usize) -> Result<Array2<f32>> {
        self.check_channel_range(z0, z1)?;
        let g = &self.geometry;
        let (rows, cols) = (g.naxis2, g.naxis1);
        let band_rows = band_rows.clamp(1, rows.max(1));
        let mut data: Vec<f32> = Vec::with_capacity(rows * cols);

        for band_start in (0..rows).step_by(band_rows) {
            let band_end = (band_start + band_rows).min(rows);
            let row_count = band_end - band_start;
            let band_npix = row_count * cols;
            let mut samples: Vec<Vec<f32>> = vec![Vec::with_capacity(z1 - z0 + 1); band_npix];

            for batch_start in (z0..=z1).step_by(BATCH_SIZE) {
                let batch_end = (batch_start + BATCH_SIZE).min(z1 + 1);
                let bands = self.decode_row_band_batch(batch_start, batch_end, band_start, row_count)?;
                for pixels in &bands {
                    for (slot, &v) in samples.iter_mut().zip(pixels.iter()) {
                        if is_valid_pixel(v) {
                            slot.push(v);
                        }
                    }
                }
            }

            let band_result: Vec<f32> = samples.into_par_iter().map(|mut vals| median_or_nan(&mut vals)).collect();
            data.extend_from_slice(&band_result);
        }

        Array2::from_shape_vec((rows, cols), data).context("Failed to reshape collapsed median range")
    }

    pub fn compute_global_stats_streaming(&self) -> Result<GlobalCubeStats> {
        let g = &self.geometry;

        let (step, stride) = stats_sample_plan(g.depth, g.naxis1 * g.naxis2);

        let indices: Vec<usize> = (0..g.depth).step_by(step).collect();
        let frame_samples: Vec<Vec<f32>> = indices
            .par_iter()
            .map(|&z| {
                let pixels = self.decode_rows(z, 0, g.naxis2)?;
                Ok(pixels
                    .into_iter()
                    .step_by(stride)
                    .filter(|v| is_valid_pixel(*v))
                    .collect())
            })
            .collect::<Result<_>>()?;

        let mut sampled: Vec<f32> = frame_samples.into_iter().flatten().collect();

        if sampled.is_empty() {
            return Ok(GlobalCubeStats { median: 0.0, sigma: 1.0, low: 0.0, high: 1.0 });
        }

        let n = sampled.len();
        let mid = n / 2;
        sampled.select_nth_unstable_by(mid, f32_cmp);
        let median = sampled[mid];

        let mut deviations: Vec<f32> = sampled.iter().map(|v| (v - median).abs()).collect();
        let dev_mid = deviations.len() / 2;
        deviations.select_nth_unstable_by(dev_mid, f32_cmp);
        let sigma = display_sigma(&sampled, median, deviations[dev_mid] * MAD_TO_SIGMA as f32);

        let low_idx = (n as f64 * 0.01) as usize;
        let high_idx = ((n as f64 * 0.999) as usize).min(n - 1);
        sampled.select_nth_unstable_by(low_idx, f32_cmp);
        let low = sampled[low_idx];
        sampled.select_nth_unstable_by(high_idx, f32_cmp);
        let high = sampled[high_idx];

        Ok(GlobalCubeStats { median, sigma, low, high })
    }

    pub fn global_stats(&self) -> Result<GlobalCubeStats> {
        if let Some(stats) = self.stats.get() {
            return Ok(stats.clone());
        }
        let stats = self.compute_global_stats_streaming()?;
        Ok(self.stats.get_or_init(|| stats).clone())
    }

    pub fn save_display_png(&self, frame: &Array2<f32>, path: &str) -> Result<()> {
        let stats = self.global_stats()?;
        render_stretched_8bit(&normalize_frame_with_stats(frame, &stats), path)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io::Write;

    use crate::types::constants::BLOCK_SIZE;

    pub const LINE_CUBE_SIZE: usize = 32;
    pub const LINE_CUBE_DEPTH: usize = 40;
    pub const LINE_CENTRE_CHANNEL: f64 = 20.0;
    pub const LINE_SIGMA_CHANNELS: f64 = 3.0;
    pub const LINE_DISK_CENTRE: f64 = 16.0;
    pub const LINE_DISK_RADIUS: f64 = 6.0;
    pub const LINE_CONTINUUM: f32 = 1.0;
    pub const LINE_CDELT_UM: f64 = 0.001;
    pub const LINE_REST_UM: f64 = 1.020;

    fn push_card(header: &mut Vec<u8>, key: &str, value: &str) {
        let mut card = format!("{key:<8}= {value:>20}").into_bytes();
        card.resize(80, b' ');
        header.extend_from_slice(&card);
    }

    fn close_header(header: &mut Vec<u8>) {
        let mut end = b"END".to_vec();
        end.resize(80, b' ');
        header.extend_from_slice(&end);
        while header.len() % BLOCK_SIZE != 0 {
            header.push(b' ');
        }
    }

    fn pad_data(data: &mut Vec<u8>) {
        while data.len() % BLOCK_SIZE != 0 {
            data.push(0);
        }
    }

    pub fn image_hdu(first: &str, axes: &[usize], bitpix: i64, cards: &[(&str, &str)], mut data: Vec<u8>) -> Vec<u8> {
        let mut header = Vec::new();
        if first == "SIMPLE" {
            push_card(&mut header, "SIMPLE", "T");
        } else {
            push_card(&mut header, "XTENSION", "'IMAGE   '");
        }
        push_card(&mut header, "BITPIX", &bitpix.to_string());
        push_card(&mut header, "NAXIS", &axes.len().to_string());
        for (i, n) in axes.iter().enumerate() {
            push_card(&mut header, &format!("NAXIS{}", i + 1), &n.to_string());
        }
        if first != "SIMPLE" {
            push_card(&mut header, "PCOUNT", "0");
            push_card(&mut header, "GCOUNT", "1");
        }
        for (key, value) in cards {
            push_card(&mut header, key, value);
        }
        close_header(&mut header);
        pad_data(&mut data);
        header.extend_from_slice(&data);
        header
    }

    pub fn bintable_hdu(cards: &[(&str, &str)], row_bytes: usize, rows: usize, mut data: Vec<u8>) -> Vec<u8> {
        let mut header = Vec::new();
        push_card(&mut header, "XTENSION", "'BINTABLE'");
        for (key, value) in [("BITPIX", "8"), ("NAXIS", "2")] {
            push_card(&mut header, key, value);
        }
        push_card(&mut header, "NAXIS1", &row_bytes.to_string());
        push_card(&mut header, "NAXIS2", &rows.to_string());
        for (key, value) in [("PCOUNT", "0"), ("GCOUNT", "1")] {
            push_card(&mut header, key, value);
        }
        for (key, value) in cards {
            push_card(&mut header, key, value);
        }
        close_header(&mut header);
        pad_data(&mut data);
        header.extend_from_slice(&data);
        header
    }

    pub fn wcs_table_hdu(values: &[f32]) -> Vec<u8> {
        let tform = format!("'{}E'", values.len());
        let tdim = format!("'(1,{})'", values.len());
        let cards = [
            ("TFIELDS", "1"),
            ("TTYPE1", "'wavelength'"),
            ("TFORM1", tform.as_str()),
            ("TDIM1", tdim.as_str()),
            ("TUNIT1", "'um'"),
            ("EXTNAME", "'WCS-TABLE'"),
            ("EXTVER", "1"),
        ];
        let data = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        bintable_hdu(&cards, 4 * values.len(), 1, data)
    }

    pub fn tab_cube_cards() -> Vec<(&'static str, &'static str)> {
        vec![
            ("CTYPE3", "'WAVE-TAB'"),
            ("PS3_0", "'WCS-TABLE'"),
            ("PS3_1", "'wavelength'"),
            ("CRPIX3", "0"),
            ("CRVAL3", "0"),
            ("CDELT3", "1"),
            ("CUNIT3", "'um'"),
        ]
    }

    pub fn write_cube_with_wcs_table(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        cube_cards: &[(&str, &str)],
        table_values: &[f32],
        value: impl Fn(usize, usize, usize) -> f32,
    ) {
        let depth = table_values.len();
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        let mut sci: Vec<(&str, &str)> = vec![("EXTNAME", "'SCI'")];
        sci.extend_from_slice(cube_cards);
        bytes.extend(image_hdu("XTENSION", &[cols, rows, depth], -32, &sci, f32_samples(cols, rows, depth, value)));
        bytes.extend(wcs_table_hdu(table_values));
        write_bytes(path, &bytes);
    }

    pub fn empty_bintable_hdu(cards: &[(&str, &str)]) -> Vec<u8> {
        let mut header = Vec::new();
        push_card(&mut header, "XTENSION", "'BINTABLE'");
        for (key, value) in [("BITPIX", "8"), ("NAXIS", "2"), ("NAXIS1", "8"), ("NAXIS2", "0"), ("PCOUNT", "0"), ("GCOUNT", "1"), ("TFIELDS", "1")] {
            push_card(&mut header, key, value);
        }
        for (key, value) in cards {
            push_card(&mut header, key, value);
        }
        close_header(&mut header);
        header
    }

    pub fn f32_samples(cols: usize, rows: usize, depth: usize, value: impl Fn(usize, usize, usize) -> f32) -> Vec<u8> {
        let mut data = Vec::with_capacity(cols * rows * depth * 4);
        for z in 0..depth {
            for y in 0..rows {
                for x in 0..cols {
                    data.extend_from_slice(&value(z, y, x).to_be_bytes());
                }
            }
        }
        data
    }

    pub fn write_bytes(path: &std::path::Path, bytes: &[u8]) {
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    pub fn write_cube(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        depth: usize,
        cards: &[(&str, &str)],
        value: impl Fn(usize, usize, usize) -> f32,
    ) {
        let data = f32_samples(cols, rows, depth, value);
        write_bytes(path, &image_hdu("SIMPLE", &[cols, rows, depth], -32, cards, data));
    }

    pub fn write_i16_cube(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        depth: usize,
        cards: &[(&str, &str)],
        value: impl Fn(usize, usize, usize) -> i16,
    ) {
        let mut data = Vec::with_capacity(cols * rows * depth * 2);
        for z in 0..depth {
            for y in 0..rows {
                for x in 0..cols {
                    data.extend_from_slice(&value(z, y, x).to_be_bytes());
                }
            }
        }
        write_bytes(path, &image_hdu("SIMPLE", &[cols, rows, depth], 16, cards, data));
    }

    pub const RAMP_U16_BZERO: i64 = 32768;

    pub fn write_u16_ramp_mef(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        ngroups: usize,
        nints: usize,
        primary_cards: &[(&str, &str)],
        sci_cards: &[(&str, &str)],
        value: impl Fn(usize, usize, usize, usize) -> u16,
    ) {
        let mut primary: Vec<(&str, &str)> = vec![("EXTEND", "T")];
        primary.extend_from_slice(primary_cards);
        let mut bytes = image_hdu("SIMPLE", &[], 8, &primary, Vec::new());
        let bzero = RAMP_U16_BZERO.to_string();
        let mut sci: Vec<(&str, &str)> = vec![("BZERO", bzero.as_str()), ("BSCALE", "1"), ("EXTNAME", "'SCI'")];
        sci.extend_from_slice(sci_cards);
        let mut data = Vec::with_capacity(cols * rows * ngroups * nints * 2);
        for i in 0..nints {
            for g in 0..ngroups {
                for y in 0..rows {
                    for x in 0..cols {
                        let stored = (value(i, g, y, x) as i64 - RAMP_U16_BZERO) as i16;
                        data.extend_from_slice(&stored.to_be_bytes());
                    }
                }
            }
        }
        bytes.extend(image_hdu("XTENSION", &[cols, rows, ngroups, nints], 16, &sci, data));
        write_bytes(path, &bytes);
    }

    pub fn write_mef_cube(path: &std::path::Path, cols: usize, rows: usize, depth: usize, extensions: &[(&str, f32)]) {
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        for (extname, base) in extensions {
            let quoted = format!("'{}'", extname);
            let data = f32_samples(cols, rows, depth, |z, _, _| base + z as f32);
            bytes.extend(image_hdu("XTENSION", &[cols, rows, depth], -32, &[("EXTNAME", quoted.as_str())], data));
        }
        write_bytes(path, &bytes);
    }

    pub fn line_profile(z: usize) -> f32 {
        let d = z as f64 - LINE_CENTRE_CHANNEL;
        (-d * d / (2.0 * LINE_SIGMA_CHANNELS * LINE_SIGMA_CHANNELS)).exp() as f32
    }

    pub fn inside_disk(y: usize, x: usize) -> bool {
        let dx = x as f64 - LINE_DISK_CENTRE;
        let dy = y as f64 - LINE_DISK_CENTRE;
        dx * dx + dy * dy <= LINE_DISK_RADIUS * LINE_DISK_RADIUS
    }

    pub fn disk_pixel_count() -> usize {
        (0..LINE_CUBE_SIZE)
            .flat_map(|y| (0..LINE_CUBE_SIZE).map(move |x| (y, x)))
            .filter(|&(y, x)| inside_disk(y, x))
            .count()
    }

    pub fn deterministic_noise(z: usize, y: usize, x: usize, amplitude: f32) -> f32 {
        if amplitude == 0.0 {
            return 0.0;
        }
        let mut h = (z as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ (x as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        let unit = (h % 2001) as f32 / 1000.0 - 1.0;
        amplitude * unit
    }

    pub fn line_cube_cards() -> Vec<(&'static str, &'static str)> {
        vec![
            ("WCSAXES", "3"),
            ("CTYPE1", "'RA---TAN'"),
            ("CTYPE2", "'DEC--TAN'"),
            ("CTYPE3", "'WAVE'"),
            ("CUNIT3", "'um'"),
            ("CRVAL1", "10.0"),
            ("CRVAL2", "-20.0"),
            ("CRVAL3", "1.0"),
            ("CRPIX1", "16.0"),
            ("CRPIX2", "16.0"),
            ("CRPIX3", "1.0"),
            ("CD1_1", "-1.0E-4"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "1.0E-4"),
            ("CD3_3", "0.001"),
            ("RESTWAV", "1.020E-6"),
            ("SPECSYS", "'BARYCENT'"),
            ("BUNIT", "'Jy/beam'"),
        ]
    }

    pub fn write_line_cube(path: &std::path::Path, noise_amplitude: f32) {
        write_cube(
            path,
            LINE_CUBE_SIZE,
            LINE_CUBE_SIZE,
            LINE_CUBE_DEPTH,
            &line_cube_cards(),
            |z, y, x| {
                let line = if inside_disk(y, x) { line_profile(z) } else { 0.0 };
                LINE_CONTINUUM + line + deterministic_noise(z, y, x, noise_amplitude)
            },
        );
    }

    pub fn write_line_cube_with_cards(
        path: &std::path::Path,
        cards: &[(&'static str, &'static str)],
    ) {
        let mut all = line_cube_cards();
        for (key, value) in cards {
            all.retain(|(k, _)| k != key);
            all.push((key, value));
        }
        write_cube(path, LINE_CUBE_SIZE, LINE_CUBE_SIZE, LINE_CUBE_DEPTH, &all, |z, y, x| {
            let line = if inside_disk(y, x) { line_profile(z) } else { 0.0 };
            LINE_CONTINUUM + line
        });
    }

    pub fn context_hdu(cols: usize, rows: usize, planes: usize) -> Vec<u8> {
        context_hdu_named("'CON     '", cols, rows, planes)
    }

    pub fn context_hdu_named(extname: &str, cols: usize, rows: usize, planes: usize) -> Vec<u8> {
        let data: Vec<u8> = (0..cols * rows * planes).flat_map(|i| (i as i32 % 4).to_be_bytes()).collect();
        image_hdu("XTENSION", &[cols, rows, planes], 32, &[("EXTNAME", extname)], data)
    }

    pub fn write_i2d_like_mef(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        primary_cards: &[(&str, &str)],
        sci_cards: &[(&str, &str)],
    ) {
        let mut primary: Vec<(&str, &str)> = vec![("EXTEND", "T")];
        primary.extend_from_slice(primary_cards);
        let mut bytes = image_hdu("SIMPLE", &[], 8, &primary, Vec::new());
        let mut sci: Vec<(&str, &str)> = vec![("EXTNAME", "'SCI     '")];
        sci.extend_from_slice(sci_cards);
        bytes.extend(image_hdu("XTENSION", &[cols, rows], -32, &sci, f32_samples(cols, rows, 1, |_, y, x| (y * cols + x) as f32)));
        bytes.extend(context_hdu(cols, rows, 2));
        write_bytes(path, &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_support::*;

    #[test]
    fn test_lru_cache() {
        let mut cache = LruFrameCache::new(72);
        let frame1 = Array2::<f32>::zeros((3, 3));
        let frame2 = Array2::<f32>::ones((3, 3));
        let frame3 = Array2::<f32>::from_elem((3, 3), 2.0);

        cache.insert(0, Arc::new(frame1));
        cache.insert(1, Arc::new(frame2));
        cache.insert(2, Arc::new(frame3));

        assert!(cache.get(0).is_none());
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_some());
    }

    #[test]
    fn test_normalize_frame_with_stats() {
        let frame = Array2::from_shape_vec((2, 2), vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        let stats = GlobalCubeStats {
            median: 2.5,
            sigma: 1.0,
            low: 1.0,
            high: 4.0,
        };
        let normalized = normalize_frame_with_stats(&frame, &stats);
        assert_eq!(normalized.dim(), (2, 2));
        for &v in normalized.iter() {
            assert!(is_valid_pixel(v) && v > 0.0 && v <= 1.0, "{}", v);
        }
        assert!(normalized[[0, 0]] < normalized[[0, 1]]);
        assert!(normalized[[0, 1]] < normalized[[1, 0]]);
        assert!(normalized[[1, 0]] < normalized[[1, 1]]);
    }

    #[test]
    fn stats_sample_plan_bounds_frames_and_samples() {
        for naxis3 in 2..300usize {
            let (step, stride) = stats_sample_plan(naxis3, 100);
            let frames = (0..naxis3).step_by(step).count();
            assert!(frames >= 1 && frames <= STATS_SAMPLE_FRAMES, "naxis3={} frames={}", naxis3, frames);
            assert_eq!(stride, 1);
        }

        let npix = 4096 * 4096;
        let (step, stride) = stats_sample_plan(40, npix);
        let frames = (0..40).step_by(step).count();
        assert_eq!(frames, 20);
        let samples = frames * ((npix + stride - 1) / stride);
        assert!(samples <= 2 * STATS_TARGET_SAMPLES, "samples={}", samples);
        assert!(samples >= STATS_TARGET_SAMPLES / 2, "samples={}", samples);

        let (step, stride) = stats_sample_plan(0, npix);
        assert_eq!((step, stride), (1, 1));
    }

    #[test]
    fn global_stats_streaming_samples_at_most_32_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube40.fits");
        write_cube(&path, 4, 4, 40, &[], |z, _, _| (z + 1) as f32);
        let lazy = LazyCube::open(path.to_str().unwrap()).unwrap();

        let stats = lazy.compute_global_stats_streaming().unwrap();
        assert_eq!(stats.median, 21.0);
        assert_eq!(stats.low, 1.0);
        assert_eq!(stats.high, 39.0);
        let cached = lazy.global_stats().unwrap();
        assert_eq!((cached.median, cached.low, cached.high), (21.0, 1.0, 39.0));
    }

    #[test]
    fn cube_bytes_overflow_is_rejected() {
        assert_eq!(checked_cube_bytes(2, 2, 2, 4).unwrap(), (16, 32));
        assert!(checked_cube_bytes(usize::MAX, 2, 1, 1).is_err());
        assert!(checked_cube_bytes(1 << 40, 1 << 40, 1, 1).is_err());
        assert!(checked_cube_bytes(4, 4, usize::MAX, 1).is_err());
    }

    fn open_line_cube(dir: &tempfile::TempDir, noise: f32) -> LazyCube {
        let path = dir.path().join("line_cube.fits");
        write_line_cube(&path, noise);
        LazyCube::open(path.to_str().unwrap()).unwrap()
    }

    fn disk_circle() -> RegionShape {
        RegionShape::Circle { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r: LINE_DISK_RADIUS }
    }

    #[test]
    fn aperture_spectrum_over_the_disk_matches_npix_times_the_line() {
        let dir = tempfile::tempdir().unwrap();
        let cube = open_line_cube(&dir, 0.0);
        let spectrum = cube.extract_spectrum_aperture(&disk_circle(), None, 1).unwrap();
        let npix = disk_pixel_count() as f64;
        assert_eq!(spectrum.npix, npix);
        assert_eq!(spectrum.sum.len(), LINE_CUBE_DEPTH);
        assert!(spectrum.bg_per_pixel.is_none());
        assert_eq!(spectrum.n_bg, 0);
        for z in 0..LINE_CUBE_DEPTH {
            let expected = npix * (1.0 + line_profile(z) as f64);
            assert!(
                (spectrum.sum[z] as f64 - expected).abs() < 1e-3,
                "z={} sum={} expected={}",
                z,
                spectrum.sum[z],
                expected
            );
            let expected_mean = 1.0 + line_profile(z) as f64;
            assert!((spectrum.mean[z] as f64 - expected_mean).abs() < 1e-5, "z={} mean={}", z, spectrum.mean[z]);
        }
    }

    #[test]
    fn annulus_background_removes_the_continuum() {
        let dir = tempfile::tempdir().unwrap();
        let cube = open_line_cube(&dir, 0.0);
        let background = RegionShape::Annulus { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r_inner: 9.0, r_outer: 13.0 };
        let spectrum = cube.extract_spectrum_aperture(&disk_circle(), Some(&background), 1).unwrap();
        let npix = disk_pixel_count() as f64;
        let levels = spectrum.bg_per_pixel.as_ref().expect("background levels");
        assert_eq!(levels.len(), LINE_CUBE_DEPTH);
        assert!(spectrum.n_bg > 100, "n_bg={}", spectrum.n_bg);
        for z in 0..LINE_CUBE_DEPTH {
            assert!((levels[z] - LINE_CONTINUUM).abs() < 1e-6, "z={} level={}", z, levels[z]);
            let expected = npix * line_profile(z) as f64;
            assert!(
                (spectrum.sum[z] as f64 - expected).abs() < 1e-3,
                "z={} sum={} expected={}",
                z,
                spectrum.sum[z],
                expected
            );
        }
    }

    #[test]
    fn box_ellipse_and_polygon_apertures_use_unit_weights_on_lattice_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let cube = open_line_cube(&dir, 0.0);
        let c = LINE_DISK_CENTRE;
        let boxed = RegionShape::Box { x: c, y: c, width: 5.0, height: 5.0, angle: 0.0 };
        let polygon = RegionShape::Polygon {
            points: vec![[c - 2.5, c - 2.5], [c + 2.5, c - 2.5], [c + 2.5, c + 2.5], [c - 2.5, c + 2.5]],
        };
        let ellipse = RegionShape::Ellipse { x: c, y: c, rx: 2.0, ry: 2.0, angle: 0.0 };
        let from_box = cube.extract_spectrum_aperture(&boxed, None, 5).unwrap();
        let from_polygon = cube.extract_spectrum_aperture(&polygon, None, 5).unwrap();
        let from_ellipse = cube.extract_spectrum_aperture(&ellipse, None, 5).unwrap();
        assert_eq!(from_box.npix, 25.0);
        assert_eq!(from_polygon.npix, 25.0);
        assert_eq!(from_ellipse.npix, 13.0);
        for z in 0..LINE_CUBE_DEPTH {
            let expected = 25.0 * (1.0 + line_profile(z) as f64);
            assert!((from_box.sum[z] as f64 - expected).abs() < 1e-3, "z={}", z);
            assert_eq!(from_box.sum[z], from_polygon.sum[z]);
            assert!((from_ellipse.mean[z] as f64 - (1.0 + line_profile(z) as f64)).abs() < 1e-5);
        }
    }

    #[test]
    fn aperture_spectrum_rejects_regions_without_area() {
        let dir = tempfile::tempdir().unwrap();
        let cube = open_line_cube(&dir, 0.0);
        let line = RegionShape::Line { x1: 0.0, y1: 0.0, x2: 5.0, y2: 5.0 };
        let point = RegionShape::Point { x: 3.0, y: 3.0 };
        let off_image = RegionShape::Circle { x: 500.0, y: 500.0, r: 2.0 };
        assert!(cube.extract_spectrum_aperture(&line, None, 1).unwrap_err().to_string().contains("no area"));
        assert!(cube.extract_spectrum_aperture(&point, None, 1).unwrap_err().to_string().contains("no area"));
        assert!(cube.extract_spectrum_aperture(&off_image, None, 1).unwrap_err().to_string().contains("no image pixels"));
        let bad_background = RegionShape::Annulus { x: 500.0, y: 500.0, r_inner: 1.0, r_outer: 3.0 };
        assert!(cube
            .extract_spectrum_aperture(&disk_circle(), Some(&bad_background), 1)
            .unwrap_err()
            .to_string()
            .contains("background"));
    }

    #[test]
    fn collapse_range_sum_over_the_line_matches_the_analytic_sum() {
        let dir = tempfile::tempdir().unwrap();
        let cube = open_line_cube(&dir, 0.0);
        let (z0, z1) = (15usize, 25usize);
        let sum = cube.collapse_range(z0, z1, CollapseMode::Sum).unwrap();
        let mean = cube.collapse_range(z0, z1, CollapseMode::Mean).unwrap();
        let median = cube.collapse_range(z0, z1, CollapseMode::Median).unwrap();
        assert_eq!(sum.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
        let n = (z1 - z0 + 1) as f64;
        let line_sum: f64 = (z0..=z1).map(|z| line_profile(z) as f64).sum();
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        assert!((sum[centre] as f64 - (n + line_sum)).abs() < 1e-3, "{}", sum[centre]);
        assert!((sum[[0, 0]] as f64 - n).abs() < 1e-5, "{}", sum[[0, 0]]);
        assert!((mean[centre] as f64 - (n + line_sum) / n).abs() < 1e-5);
        assert!((mean[[0, 0]] as f64 - 1.0).abs() < 1e-6);
        assert!((median[centre] - (1.0 + line_profile(17))).abs() < 1e-6, "{}", median[centre]);
        assert!((median[[0, 0]] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn collapse_range_median_in_row_bands_matches_the_single_pass_result() {
        let dir = tempfile::tempdir().unwrap();
        let cube = open_line_cube(&dir, 0.05);
        let full = cube.collapse_range(10, 30, CollapseMode::Median).unwrap();
        let banded = cube.collapse_range_median_banded(10, 30, 5).unwrap();
        assert_eq!(full, banded);
        assert_eq!(median_band_rows(21, LINE_CUBE_SIZE, LINE_CUBE_SIZE, 1 << 30), LINE_CUBE_SIZE);
        assert_eq!(median_band_rows(21, LINE_CUBE_SIZE, LINE_CUBE_SIZE, 21 * LINE_CUBE_SIZE * 4 * 3), 3);
        assert_eq!(median_band_rows(1000, 4096, 4096, 1 << 30), 65);
        assert_eq!(median_band_rows(1000, 1 << 30, 10, 1 << 30), 1);
    }

    #[test]
    fn collapse_range_rejects_bad_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let cube = open_line_cube(&dir, 0.0);
        assert!(cube.collapse_range(5, 4, CollapseMode::Sum).unwrap_err().to_string().contains("after"));
        assert!(cube.collapse_range(0, LINE_CUBE_DEPTH, CollapseMode::Mean).unwrap_err().to_string().contains("out of range"));
        assert_eq!(CollapseMode::parse("Median").unwrap(), CollapseMode::Median);
        assert!(CollapseMode::parse("max").is_err());
    }

    fn zeros_fixture_value(z: usize, y: usize, x: usize) -> Option<f32> {
        if z == 2 {
            Some(0.0)
        } else if z == 4 && y == 0 && x == 0 {
            None
        } else {
            Some(2.0)
        }
    }

    fn assert_zeros_fixture(cube: &LazyCube) {
        let whole = RegionShape::Box { x: 1.5, y: 1.5, width: 4.0, height: 4.0, angle: 0.0 };
        let spectrum = cube.extract_spectrum_aperture(&whole, None, 1).unwrap();
        assert_eq!(spectrum.npix, 16.0);
        assert_eq!(spectrum.sum[2], 0.0);
        assert_eq!(spectrum.mean[2], 0.0);
        assert_eq!(spectrum.sum[4], 30.0);
        assert_eq!(spectrum.mean[4], 2.0);
        assert_eq!(spectrum.sum[0], 32.0);

        let mean = cube.collapse_range(0, 5, CollapseMode::Mean).unwrap();
        assert_eq!(mean[[1, 1]], 2.0);
        assert_eq!(mean[[0, 0]], 2.0);
        let sum = cube.collapse_range(0, 5, CollapseMode::Sum).unwrap();
        assert_eq!(sum[[0, 0]], 8.0);
        let median = cube.collapse_range(0, 5, CollapseMode::Median).unwrap();
        assert_eq!(median[[0, 0]], 2.0);
        assert_eq!(median[[1, 1]], 2.0);
        let only_zeros = cube.collapse_range(2, 2, CollapseMode::Mean).unwrap();
        assert!(only_zeros[[3, 3]].is_nan(), "{}", only_zeros[[3, 3]]);
        let only_missing = cube.collapse_range(4, 4, CollapseMode::Median).unwrap();
        assert!(only_missing[[0, 0]].is_nan());
        assert_eq!(only_missing[[0, 1]], 2.0);
    }

    #[test]
    fn range_collapses_skip_zero_padding_and_nan_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zeros.fits");
        write_cube(&path, 4, 4, 6, &[], |z, y, x| zeros_fixture_value(z, y, x).unwrap_or(f32::NAN));
        assert_zeros_fixture(&LazyCube::open(path.to_str().unwrap()).unwrap());
    }

    #[test]
    fn a_full_range_collapse_skips_padded_channels_and_keeps_negative_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("padded_channels.fits");
        let padded = [0.0, 0.0, 0.0, 10.0, 10.0];
        let negative = [-3.0, 0.0, -5.0, -4.0, f32::NAN];
        write_cube(&path, 3, 2, 5, &[], |z, y, x| match (y, x) {
            (1, 2) => padded[z],
            (0, 1) => negative[z],
            _ => 1.0 + z as f32,
        });
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let mean = cube.collapse_range(0, 4, CollapseMode::Mean).unwrap();
        let median = cube.collapse_range(0, 4, CollapseMode::Median).unwrap();
        let sum = cube.collapse_range(0, 4, CollapseMode::Sum).unwrap();
        assert_eq!(mean[[1, 2]], 10.0);
        assert_eq!(median[[1, 2]], 10.0);
        assert_eq!(sum[[1, 2]], 20.0);
        assert_eq!(mean[[0, 1]], -4.0);
        assert_eq!(median[[0, 1]], -4.0);
        assert_eq!(mean[[0, 0]], 3.0);
        assert_eq!(median[[0, 0]], 3.0);
    }

    #[test]
    fn blank_integer_samples_are_missing_data_in_every_lazy_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zeros_i16.fits");
        let cards = [("BLANK", "-32768"), ("BSCALE", "0.5"), ("BZERO", "0.0")];
        write_i16_cube(&path, 4, 4, 6, &cards, |z, y, x| match zeros_fixture_value(z, y, x) {
            Some(v) => (v * 2.0) as i16,
            None => i16::MIN,
        });
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert_eq!(cube.geometry.blank, Some(-32768));
        assert_zeros_fixture(&cube);

        let spectrum = cube.extract_spectrum_at(0, 0).unwrap();
        assert!(spectrum[4].is_nan(), "BLANK decoded as {}", spectrum[4]);
        assert_eq!(spectrum[0], 2.0);
        assert!(cube.get_frame(4).unwrap()[[0, 0]].is_nan());
        assert!(cube.frame_uncached(4).unwrap()[[0, 0]].is_nan());
        assert!(cube.decode_rows(4, 0, 1).unwrap()[0].is_nan());
        assert!(cube.compute_global_stats_streaming().unwrap().low > 0.0);

        let float_blank = dir.path().join("float_blank.fits");
        write_cube(&float_blank, 2, 2, 2, &[("BLANK", "0")], |_, _, _| 0.0);
        let float_cube = LazyCube::open(float_blank.to_str().unwrap()).unwrap();
        assert_eq!(float_cube.geometry.blank, None, "BLANK only applies to integer data");
    }

    #[test]
    fn a_degenerate_fourth_axis_opens_as_a_cube_and_a_real_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let stokes = dir.path().join("stokes.fits");
        let data = f32_samples(3, 2, 5, |z, y, x| (z * 100 + y * 10 + x) as f32);
        write_bytes(&stokes, &image_hdu("SIMPLE", &[3, 2, 5, 1], -32, &[("CTYPE3", "'FREQ'"), ("CTYPE4", "'STOKES'")], data));
        let cube = LazyCube::open(stokes.to_str().unwrap()).unwrap();
        assert_eq!((cube.geometry.naxis1, cube.geometry.naxis2, cube.geometry.naxis3), (3, 2, 5));
        assert_eq!(cube.get_frame(4).unwrap()[[1, 2]], 412.0);
        assert_eq!(cube.extract_spectrum_at(1, 2).unwrap(), vec![12.0, 112.0, 212.0, 312.0, 412.0]);

        assert_eq!((cube.geometry.naxis4, cube.geometry.depth), (1, 5));
        assert!(cube.primary_header.is_none());
        assert!(cube.ramp().is_none());
    }

    #[test]
    fn a_fourth_axis_above_one_without_ramp_cards_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        let polarised = dir.path().join("polarised.fits");
        let data = f32_samples(3, 2, 10, |z, _, _| z as f32);
        write_bytes(&polarised, &image_hdu("SIMPLE", &[3, 2, 5, 2], -32, &[], data));
        let err = format!("{:#}", LazyCube::open(polarised.to_str().unwrap()).err().expect("NAXIS4=2 must be refused"));
        assert!(err.contains("No 3D data block"), "{}", err);
        let err = format!("{:#}", LazyCube::open(&format!("{}#hdu=0", polarised.to_str().unwrap())).err().unwrap());
        assert!(err.contains("NAXIS4=2"), "{}", err);

        let inconsistent = dir.path().join("inconsistent.fits");
        write_u16_ramp_mef(&inconsistent, 3, 2, 5, 2, &[("NGROUPS", "4"), ("NINTS", "2")], &[], |_, _, _, _| 1);
        let err = format!("{:#}", LazyCube::open(inconsistent.to_str().unwrap()).err().expect("NGROUPS 4 on NAXIS3 5 is not a ramp"));
        assert!(err.contains("No 3D data block"), "{}", err);
    }

    #[test]
    fn a_fourth_axis_whose_product_matches_the_cards_but_whose_axes_disagree_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let swapped = dir.path().join("swapped.fits");
        let primary = [("NGROUPS", "4"), ("NINTS", "3"), ("DATAMODL", "'Level1bModel'")];
        write_u16_ramp_mef(&swapped, 3, 2, 6, 2, &primary, &[], |_, _, _, _| 1);
        let err = format!("{:#}", LazyCube::open(swapped.to_str().unwrap()).err().expect("NGROUPS 4 x NINTS 3 on [6, 2] is not a ramp"));
        assert!(err.contains("No 3D data block"), "{}", err);
        let err = format!("{:#}", LazyCube::open(&format!("{}#hdu=1", swapped.to_str().unwrap())).err().unwrap());
        assert!(err.contains("NAXIS4=2"), "{}", err);
    }

    #[test]
    fn a_ramp_whose_cards_disagree_with_the_axes_of_a_degenerate_fourth_axis_opens_as_a_plain_cube() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flat_integrations.fits");
        let primary = [("NGROUPS", "5"), ("NINTS", "2"), ("TGROUP", "10.0"), ("DATAMODL", "'Level1bModel'")];
        write_u16_ramp_mef(&path, 3, 2, 10, 1, &primary, &[], |_, g, _, _| (30000 + g) as u16);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert_eq!((cube.geometry.naxis3, cube.geometry.naxis4, cube.geometry.depth), (10, 1, 10));
        assert!(ramp_info(&cube.merged_header()).is_some(), "the product check alone accepts 5 x 2 on 10 x 1");
        assert!(cube.ramp().is_none(), "the cube reports no ramp when the axes disagree with the cards");
        assert_eq!(cube.extract_spectrum_at(0, 0).unwrap().len(), 10);
        assert_eq!(cube.extract_group_series(0, 0, 0).unwrap().len(), 10);
    }

    #[test]
    fn a_ramp_with_a_single_group_and_several_integrations_opens_as_a_cube_of_integrations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("one_group.fits");
        let primary = [("NGROUPS", "1"), ("NINTS", "5"), ("TGROUP", "10.0"), ("DATAMODL", "'Level1bModel'")];
        write_u16_ramp_mef(&path, 3, 2, 1, 5, &primary, &[], |i, _, y, x| (30000 + i * 100 + y * 10 + x) as u16);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert_eq!((cube.geometry.naxis3, cube.geometry.naxis4, cube.geometry.depth), (1, 5, 5));
        let ramp = cube.ramp().expect("NGROUPS 1 x NINTS 5 is a ramp");
        assert_eq!((ramp.ngroups, ramp.nints), (1, 5));
        assert_eq!(cube.extract_group_series(1, 2, 3).unwrap(), vec![30312.0]);
        assert_eq!(cube.get_frame(4).unwrap()[[0, 0]], 30400.0);
        assert_eq!(cube.extract_spectrum_at(0, 0).unwrap().len(), 5);

        let flat = dir.path().join("flat.fits");
        write_u16_ramp_mef(&flat, 3, 2, 1, 1, &[("NGROUPS", "1"), ("NINTS", "1")], &[], |_, _, _, _| 1);
        let err = format!("{:#}", LazyCube::open(&format!("{}#hdu=1", flat.to_str().unwrap())).err().expect("a single plane is not a cube"));
        assert!(err.contains("at least two planes"), "{}", err);
    }

    fn ramp_value(i: usize, g: usize, y: usize, x: usize) -> u16 {
        (30000 + i * 1000 + g * 100 + y * 10 + x) as u16
    }

    #[test]
    fn a_four_dimensional_ramp_with_several_integrations_opens_when_the_primary_header_names_ngroups_and_nints() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jw_uncal.fits");
        let primary = [("NGROUPS", "3"), ("NINTS", "2"), ("TGROUP", "10.0"), ("DATAMODL", "'Level1bModel'")];
        write_u16_ramp_mef(&path, 4, 3, 3, 2, &primary, &[], ramp_value);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert_eq!(cube.hdu_index, 1);
        assert_eq!((cube.geometry.naxis1, cube.geometry.naxis2), (4, 3));
        assert_eq!((cube.geometry.naxis3, cube.geometry.naxis4, cube.geometry.depth), (3, 2, 6));
        assert_eq!(cube.get_frame(4).unwrap()[[1, 2]], 31112.0);
        assert_eq!(cube.get_frame(5).unwrap()[[2, 3]], 31223.0);
        assert_eq!(cube.extract_group_series(1, 2, 1).unwrap(), vec![31012.0, 31112.0, 31212.0]);
        assert_eq!(cube.extract_group_series(0, 0, 0).unwrap(), vec![30000.0, 30100.0, 30200.0]);
        assert_eq!(cube.extract_spectrum_at(1, 2).unwrap().len(), 6);
        assert_eq!(cube.extract_spectrum_at(1, 2).unwrap()[4], 31112.0);
        assert!(cube.extract_group_series(1, 2, 2).unwrap_err().to_string().contains("out of range"));
        assert!(cube.get_frame(6).is_err());
        assert_eq!(cube.decode_frames(0, 6).unwrap().len(), 6);
        assert_eq!(cube.collapse_range(0, 5, CollapseMode::Mean).unwrap().dim(), (3, 4));

        let ramp = cube.ramp().expect("the merged header carries NGROUPS and NINTS");
        assert_eq!((ramp.ngroups, ramp.nints), (3, 2));
        assert_eq!(ramp.tgroup_s, Some(10.0));
        assert_eq!(ramp.datamodl.as_deref(), Some("Level1bModel"));
        assert!(cube.primary_header.as_ref().is_some_and(|p| p.get("NGROUPS") == Some("3")));
        assert_eq!(cube.header.get("NGROUPS"), None);

        let explicit = LazyCube::open(&format!("{}#hdu=1", path.to_str().unwrap())).unwrap();
        assert_eq!(explicit.geometry.depth, 6);
        assert!(explicit.ramp().is_some());
    }

    #[test]
    fn the_merged_header_prefers_extension_cards_and_geometry_ignores_a_primary_bzero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("merged.fits");
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T"), ("TELESCOP", "'JWST'"), ("BUNIT", "'wrong'"), ("BZERO", "1000")], Vec::new());
        let data = f32_samples(3, 2, 4, |z, y, x| (z * 100 + y * 10 + x) as f32);
        bytes.extend(image_hdu("XTENSION", &[3, 2, 4], -32, &[("EXTNAME", "'SCI'"), ("BUNIT", "'DN'")], data));
        write_bytes(&path, &bytes);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert_eq!(cube.header.get("TELESCOP"), None);
        let merged = cube.merged_header();
        assert_eq!(merged.get("TELESCOP"), Some("JWST"));
        assert_eq!(merged.get("BUNIT"), Some("DN"));
        assert_eq!(merged.get("EXTNAME"), Some("SCI"));
        assert_eq!(merged.get_i64("NAXIS"), Some(3));
        assert_eq!(cube.geometry.bzero, 0.0);
        assert_eq!(cube.get_frame(0).unwrap()[[1, 2]], 12.0);
        assert_eq!(cube.get_frame(3).unwrap()[[0, 0]], 300.0);
        assert!(cube.ramp().is_none());
    }

    #[test]
    fn a_ramp_with_one_integration_still_opens_and_reports_a_ramp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner_like_uncal.fits");
        let primary = [
            ("NGROUPS", "10"),
            ("NINTS", "1"),
            ("TGROUP", "14.589"),
            ("DATAMODL", "'Level1bModel'"),
            ("DETECTOR", "'NRS1'"),
            ("NRS_NORM", "16"),
            ("NRS_REF", "4"),
        ];
        write_u16_ramp_mef(&path, 8, 20, 10, 1, &primary, &[("BUNIT", "'DN'")], |_, g, y, x| (1000 + g * 50 + y + x) as u16);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert_eq!((cube.geometry.naxis3, cube.geometry.naxis4, cube.geometry.depth), (10, 1, 10));
        let ramp = cube.ramp().expect("NINTS 1 is still a ramp");
        assert_eq!((ramp.ngroups, ramp.nints), (10, 1));
        assert_eq!(ramp.detector.as_deref(), Some("NRS1"));
        assert_eq!(ramp.irs2.as_ref().map(|i| (i.nrs_norm, i.nrs_ref, i.noutputs)), Some((16, 4, 5)));
        assert_eq!((ramp.frame_width, ramp.frame_height), (8, 20));
        assert_eq!(cube.extract_group_series(3, 2, 0).unwrap()[4], 1205.0);
        assert_eq!(cube.get_frame(9).unwrap()[[19, 7]], 1476.0);
    }

    #[test]
    fn a_plane_ref_opens_the_named_hdu_of_the_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jw_s3d.fits");
        write_mef_cube(&path, 4, 3, 6, &[("SCI", 10.0), ("ERR", 500.0)]);
        let source = path.to_str().unwrap();

        let auto = LazyCube::open(source).unwrap();
        assert_eq!(auto.hdu_index, 1);
        assert_eq!(auto.get_frame(2).unwrap()[[0, 0]], 12.0);

        let sci = LazyCube::open(&format!("{}#hdu=1", source)).unwrap();
        assert_eq!(sci.hdu_index, 1);
        assert_eq!(sci.header.get("EXTNAME"), Some("SCI"));
        assert_eq!(sci.extract_spectrum_at(2, 3).unwrap()[5], 15.0);

        let err = LazyCube::open(&format!("{}#hdu=2", source)).unwrap();
        assert_eq!(err.hdu_index, 2);
        assert_eq!(err.header.get("EXTNAME"), Some("ERR"));
        assert_eq!(err.get_frame(0).unwrap()[[1, 1]], 500.0);

        let primary = format!("{:#}", LazyCube::open(&format!("{}#hdu=0", source)).err().unwrap());
        assert!(primary.contains("HDU 0") && primary.contains("NAXIS=0"), "{}", primary);
        let missing = format!("{:#}", LazyCube::open(&format!("{}#hdu=7", source)).err().unwrap());
        assert!(missing.contains("out of range"), "{}", missing);
        let asdf = format!("{:#}", LazyCube::open(&format!("{}#array=data", source)).err().unwrap());
        assert!(asdf.contains("ASDF"), "{}", asdf);
    }

    #[test]
    fn a_cube_with_a_wave_tab_axis_reads_its_wcs_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tab_cube.fits");
        write_cube_with_wcs_table(&path, 4, 4, &tab_cube_cards(), &[1.0, 1.5, 2.25], |z, _, _| z as f32 + 1.0);
        let source = path.to_str().unwrap();
        let cube = LazyCube::open(source).unwrap();
        assert_eq!(cube.source_path, source);
        assert_eq!(cube.geometry.naxis3, 3);
        let axis = cube.spectral_axis().unwrap();
        assert!(axis.tabulated);
        assert_eq!(axis.values.len(), 3);
        for (got, want) in axis.values.iter().zip([1.0, 1.5, 2.25]) {
            assert!((got - want).abs() < 1e-7, "{:?}", axis.values);
        }
        assert_eq!(axis.unit, "um");
        assert!(axis.notes.iter().any(|n| n.starts_with("WAVE-TAB from WCS-TABLE[1] column 'wavelength' (3 entries)")), "{:?}", axis.notes);
    }

    #[test]
    fn a_linear_wave_axis_ignores_an_unreferenced_wcs_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("miri_like.fits");
        let cards = [("CTYPE3", "'WAVE'"), ("CRVAL3", "1.0"), ("CDELT3", "0.5"), ("CRPIX3", "1"), ("CUNIT3", "'um'")];
        write_cube_with_wcs_table(&path, 4, 4, &cards, &[1.0, 1.5, 2.25], |z, _, _| z as f32);
        let axis = LazyCube::open(path.to_str().unwrap()).unwrap().spectral_axis().unwrap();
        assert!(!axis.tabulated);
        assert_eq!(axis.values, vec![1.0, 1.5, 2.0]);
        assert!(axis.notes.is_empty(), "{:?}", axis.notes);
    }

    const MIRI_MRS_DIR_VAR: &str = "ASTROBURST_MIRI_MRS_DIR";

    #[test]
    #[ignore]
    fn miri_mrs_wcs_table_matches_its_linear_wave_axis_within_one_f32_ulp() {
        use crate::core::astrometry::spectral::spectral_axis_on_with;
        use crate::infra::fits::table::{read_bintable_column_by_name, ColumnValues, FileTabTables};

        let cases = [
            ("jw02016-c1012_t023_miri_ch1-long_s3d.fits", 1400usize, 6.5304003f64, 7.6496000f64),
            ("jw02016-c1012_t023_miri_ch4-long_s3d.fits", 717, 24.4030, 28.6990),
        ];
        let Some(dir) = std::env::var_os(MIRI_MRS_DIR_VAR) else {
            eprintln!("skipped: {MIRI_MRS_DIR_VAR} unset");
            return;
        };
        for (name, count, first, last) in cases {
            let path = std::path::Path::new(&dir).join(name);
            if !path.exists() {
                eprintln!("skipped: {} absent", path.display());
                return;
            }
            let path = path.to_string_lossy().into_owned();
            let column = read_bintable_column_by_name(&path, "WCS-TABLE", 1, "wavelength").unwrap();
            assert_eq!(column.repeat, count, "{name}");
            assert_eq!(column.tdim, Some(vec![1, count]), "{name}");
            let ColumnValues::F64(table) = &column.values else { panic!("{name}: wavelength column is not F64") };
            assert_eq!(table.len(), count, "{name}");
            assert!((table[0] - first).abs() < 1e-6, "{name}: {}", table[0]);
            assert!((table[count - 1] - last).abs() < 1e-6, "{name}: {}", table[count - 1]);
            assert!(table.windows(2).all(|w| w[1] > w[0]), "{name}: the table is not strictly increasing");

            let cube = LazyCube::open(&path).unwrap();
            let linear = cube.spectral_axis().unwrap();
            assert!(!linear.tabulated, "{name}");
            assert_eq!(linear.values.len(), count, "{name}");
            let mut derived = cube.header.clone();
            derived.set("CTYPE3", "WAVE-TAB".to_string());
            derived.set("PS3_0", "WCS-TABLE".to_string());
            derived.set("PS3_1", "wavelength".to_string());
            derived.set_f64("CRPIX3", 0.0);
            derived.set_f64("CRVAL3", 0.0);
            derived.set_f64("CDELT3", 1.0);
            let tabulated = spectral_axis_on_with(&derived, 3, count, &FileTabTables { path: &path }).unwrap();
            assert!(tabulated.tabulated, "{name}");
            assert_eq!(tabulated.values.len(), count, "{name}");
            let mut worst = 0.0f64;
            for (k, (tab, lin)) in tabulated.values.iter().zip(&linear.values).enumerate() {
                let delta = (tab - lin).abs();
                let bound = 2.0 * f32::EPSILON as f64 * lin.abs();
                assert!(delta <= bound, "{name} channel {k}: table {tab} vs linear {lin} (|delta| {delta:.3e} > {bound:.3e})");
                worst = worst.max(delta);
            }
            println!("{name}: {count} channels {first}..{last} um, max |table - linear| = {worst:.3e} um");
        }
    }

    #[test]
    fn auto_detection_skips_a_jwst_con_context_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jw_i2d.fits");
        write_i2d_like_mef(
            &path,
            6,
            4,
            &[("TELESCOP", "'JWST'")],
            &[("CTYPE1", "'RA---TAN'"), ("CTYPE2", "'DEC--TAN'"), ("CRVAL1", "150.0"), ("CRVAL2", "2.0")],
        );
        let key = path.to_str().unwrap();

        let auto = format!("{:#}", LazyCube::open(key).err().expect("an i2d with a CON context extension is not a cube"));
        assert!(auto.contains("No 3D data block"), "{}", auto);

        let explicit = LazyCube::open(&format!("{}#hdu=2", key)).unwrap();
        assert_eq!(explicit.hdu_index, 2);
        assert_eq!(explicit.header.get("EXTNAME"), Some("CON"));
        assert_eq!(explicit.geometry.depth, 2);
    }

    #[test]
    fn auto_detection_skips_an_hst_drizzle_ctx_context_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hst_drc.fits");
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T"), ("TELESCOP", "'HST'")], Vec::new());
        bytes.extend(image_hdu("XTENSION", &[5, 3], -32, &[("EXTNAME", "'SCI     '")], f32_samples(5, 3, 1, |_, y, x| (y * 5 + x) as f32)));
        bytes.extend(context_hdu_named("'CTX     '", 5, 3, 3));
        write_bytes(&path, &bytes);
        let key = path.to_str().unwrap();

        let auto = format!("{:#}", LazyCube::open(key).err().expect("a drizzled product with a 3D CTX extension is not a cube"));
        assert!(auto.contains("No 3D data block"), "{}", auto);

        let explicit = LazyCube::open(&format!("{}#hdu=2", key)).unwrap();
        assert_eq!(explicit.hdu_index, 2);
        assert_eq!(explicit.header.get("EXTNAME"), Some("CTX"));
        assert_eq!(explicit.geometry.depth, 3);
    }

    #[test]
    fn auto_detection_still_finds_a_science_cube_behind_a_con_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("con_then_sci.fits");
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        bytes.extend(context_hdu(3, 2, 2));
        let data = f32_samples(3, 2, 4, |z, y, x| (z * 100 + y * 10 + x) as f32);
        bytes.extend(image_hdu("XTENSION", &[3, 2, 4], -32, &[("EXTNAME", "'SCI'")], data));
        write_bytes(&path, &bytes);

        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert_eq!(cube.hdu_index, 2);
        assert_eq!(cube.header.get("EXTNAME"), Some("SCI"));
        assert_eq!(cube.geometry.depth, 4);
        assert_eq!(cube.get_frame(3).unwrap()[[1, 2]], 312.0);
    }

    #[test]
    fn a_zipped_cube_opens_through_the_zip_path() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner.fits");
        write_line_cube(&inner, 0.0);
        let zip_path = dir.path().join("cube.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        writer.start_file("inner.fits", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&std::fs::read(&inner).unwrap()).unwrap();
        writer.finish().unwrap();
        let cube = LazyCube::open(&format!("{}#hdu=0", zip_path.to_str().unwrap())).unwrap();
        assert_eq!(cube.geometry.naxis3, LINE_CUBE_DEPTH);
        assert_eq!(cube.get_frame(0).unwrap()[[0, 0]], LINE_CONTINUUM);
    }

    #[test]
    fn positioned_reads_match_the_mapping_and_leave_the_file_replaceable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("positioned.fits");
        write_line_cube(&path, 0.05);
        let key = path.to_str().unwrap();
        let mapped = LazyCube::open_with_mode(key, IoMode::Mmap).unwrap();
        let positioned = LazyCube::open_with_mode(key, IoMode::Read).unwrap();
        assert!(mapped.is_memory_mapped());
        assert!(!positioned.is_memory_mapped());

        assert_eq!(*mapped.get_frame(7).unwrap(), *positioned.get_frame(7).unwrap());
        assert_eq!(mapped.extract_spectrum_at(16, 16).unwrap(), positioned.extract_spectrum_at(16, 16).unwrap());
        assert_eq!(
            mapped.collapse_range(3, 30, CollapseMode::Median).unwrap(),
            positioned.collapse_range(3, 30, CollapseMode::Median).unwrap()
        );
        let a = mapped.extract_spectrum_aperture(&disk_circle(), None, 5).unwrap();
        let b = positioned.extract_spectrum_aperture(&disk_circle(), None, 5).unwrap();
        assert_eq!(a.sum, b.sum);
        let (sa, sb) = (mapped.global_stats().unwrap(), positioned.global_stats().unwrap());
        assert_eq!((sa.median, sa.sigma, sa.low, sa.high), (sb.median, sb.sigma, sb.low, sb.high));
        drop(mapped);

        std::fs::write(&path, b"replaced while a positioned cube was open").unwrap();
        assert!(positioned.frame_uncached(3).is_err(), "a read past the new end of file must fail, not crash");
    }

    #[test]
    fn display_png_uses_one_cube_wide_scale_for_every_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hot.fits");
        write_cube(&path, 8, 8, 4, &[], |z, y, x| {
            if z == 1 && y == 0 && x == 0 {
                1.0e5
            } else {
                100.0 + deterministic_noise(0, y, x, 2.0)
            }
        });
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let quiet = dir.path().join("quiet.png");
        let hot = dir.path().join("hot.png");
        cube.save_display_png(&cube.get_frame(0).unwrap(), quiet.to_str().unwrap()).unwrap();
        cube.save_display_png(&cube.get_frame(1).unwrap(), hot.to_str().unwrap()).unwrap();
        let quiet = image::open(&quiet).unwrap().to_luma8();
        let hot = image::open(&hot).unwrap().to_luma8();

        let sky = &hot.as_raw()[1..];
        let lit = sky.iter().filter(|&&v| v > 0).count();
        assert!(lit > sky.len() / 2, "the hot pixel crushed the sky to black: {:?}", sky);
        assert!(sky.iter().collect::<std::collections::HashSet<_>>().len() > 3);
        assert_eq!(&quiet.as_raw()[1..], sky, "the same sky rendered with a different scale in another frame");
        assert_eq!(hot.as_raw()[0], 255);
    }

    fn modal_value(z: usize, y: usize, x: usize) -> f32 {
        let i = y * 8 + x;
        if i % 4 == 0 {
            101.0 + ((i + z) % 9) as f32
        } else {
            100.0
        }
    }

    #[test]
    fn a_cube_where_one_value_fills_most_pixels_keeps_a_graded_display_scale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modal.fits");
        write_cube(&path, 8, 8, 4, &[], modal_value);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let stats = cube.compute_global_stats_streaming().unwrap();
        assert_eq!(stats.median, 100.0);
        assert!(stats.sigma > 1.0 && stats.sigma < 5.0, "sigma={}", stats.sigma);

        let png = dir.path().join("modal.png");
        cube.save_display_png(&cube.get_frame(0).unwrap(), png.to_str().unwrap()).unwrap();
        let grey = image::open(&png).unwrap().to_luma8();
        let at = |y: usize, x: usize| grey.as_raw()[y * 8 + x];
        let one_above = (0..64).find(|&i| modal_value(0, i / 8, i % 8) == 101.0).unwrap();
        let top = (0..64).find(|&i| modal_value(0, i / 8, i % 8) == 109.0).unwrap();
        assert!(at(one_above / 8, one_above % 8) < 192, "1 ADU above the mode rendered at {}", at(one_above / 8, one_above % 8));
        assert_eq!(at(top / 8, top % 8), 255);
        assert_eq!(display_sigma(&[5.0, 5.0, 5.0], 5.0, 0.0), 1.0);
        assert_eq!(display_sigma(&[1.0, 2.0], 1.5, 0.7), 0.7);
    }

    #[test]
    fn a_tile_compressed_cube_is_refused_with_a_decompress_hint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube.fits.fz");
        let compressed = [
            ("ZIMAGE", "T"),
            ("TTYPE1", "'COMPRESSED_DATA'"),
            ("TFORM1", "'1PB(0)'"),
            ("ZBITPIX", "-32"),
            ("ZNAXIS", "3"),
            ("ZNAXIS1", "4"),
            ("ZNAXIS2", "4"),
            ("ZNAXIS3", "6"),
            ("ZCMPTYPE", "'RICE_1'"),
        ];
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        bytes.extend(empty_bintable_hdu(&compressed));
        write_bytes(&path, &bytes);
        let key = path.to_str().unwrap();

        let auto = format!("{:#}", LazyCube::open(key).err().expect("a compressed cube must be refused"));
        assert!(auto.contains("HDU 1 is a tile-compressed cube (ZNAXIS=3)") && auto.contains("funpack"), "{}", auto);
        let explicit = format!("{:#}", LazyCube::open(&format!("{}#hdu=1", key)).err().unwrap());
        assert!(explicit.contains("compressed cubes are not supported"), "{}", explicit);

        let image_path = dir.path().join("image.fits.fz");
        let mut image = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        image.extend(empty_bintable_hdu(&[("ZIMAGE", "T"), ("ZNAXIS", "2"), ("ZNAXIS1", "4"), ("ZNAXIS2", "4")]));
        write_bytes(&image_path, &image);
        let image_key = image_path.to_str().unwrap();
        let auto = format!("{:#}", LazyCube::open(image_key).err().unwrap());
        assert!(auto.contains("No 3D data block found") && !auto.contains("funpack"), "{}", auto);
        let explicit = format!("{:#}", LazyCube::open(&format!("{}#hdu=1", image_key)).err().unwrap());
        assert!(explicit.contains("tile-compressed image with ZNAXIS=2, not a data cube"), "{}", explicit);
    }

    #[test]
    fn spectral_axis_comes_from_the_cube_header() {
        let dir = tempfile::tempdir().unwrap();
        let cube = open_line_cube(&dir, 0.0);
        let axis = cube.spectral_axis().unwrap();
        assert_eq!(axis.kind, crate::core::astrometry::spectral::AxisKind::Wave);
        assert_eq!(axis.unit, "um");
        assert_eq!(axis.values.len(), LINE_CUBE_DEPTH);
        assert!((axis.values[20] - LINE_REST_UM).abs() < 1e-12);
        assert!((axis.rest_wavelength_um.unwrap() - LINE_REST_UM).abs() < 1e-9);
        assert_eq!(axis.specsys.as_deref(), Some("BARYCENT"));
    }
}
