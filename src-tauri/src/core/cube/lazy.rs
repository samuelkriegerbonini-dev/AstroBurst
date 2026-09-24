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
use crate::infra::fits::compress::is_compressed_image_hdu;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::file_bytes::{io_mode, prefer_mmap, IoMode};
use crate::infra::fits::reader::{create_mmap_random, decode_pixels_blank, parse_header_at, read_header_blocks, ParsedHdu};
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

fn cube_depth(header: &HduHeader) -> std::result::Result<usize, String> {
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
    if naxis3 <= 1 {
        return Err(format!("has NAXIS3={}: a cube needs at least two planes", naxis3));
    }
    for axis in 4..=naxis {
        let len = header.get_i64(&format!("NAXIS{}", axis)).unwrap_or(1);
        if len != 1 {
            return Err(format!(
                "has NAXIS{}={}: only cubes whose axes beyond the third have length 1 are supported",
                axis, len
            ));
        }
    }
    usize::try_from(naxis3).map_err(|_| format!("has NAXIS3={}, too large for this platform", naxis3))
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
    depth: usize,
}

fn find_cube_hdu(data: &CubeData, plane: &PlaneSelector) -> Result<FoundCube> {
    let mut offset = 0usize;
    let mut index = 0usize;
    let mut compressed_cube: Option<String> = None;
    while offset < data.len() {
        let parsed = data
            .header_at(offset)
            .with_context(|| format!("Header parse failed in lazy cube at HDU {}", index))?;
        match plane {
            PlaneSelector::Hdu(n) if *n == index => {
                let depth = cube_depth(&parsed.header).map_err(|reason| anyhow!("HDU {} {}", index, reason))?;
                return Ok(FoundCube { index, parsed, depth });
            }
            PlaneSelector::Auto => match cube_depth(&parsed.header) {
                Ok(depth) => return Ok(FoundCube { index, parsed, depth }),
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
    pub header: HduHeader,
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
        let naxis3 = found.depth;

        let bitpix = header.get_i64("BITPIX").context("Missing BITPIX")?;
        if !SUPPORTED_BITPIX.contains(&bitpix) {
            bail!("Unsupported BITPIX={}", bitpix);
        }
        let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
        let (frame_bytes, total_bytes) = checked_cube_bytes(naxis1, naxis2, naxis3, bytes_per_pixel)?;
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
            header,
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
        if z >= self.geometry.naxis3 {
            bail!("Frame index {} out of range (depth={})", z, self.geometry.naxis3);
        }
        Ok(())
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
        let g = &self.geometry;
        if y >= g.naxis2 || x >= g.naxis1 {
            bail!("Pixel ({}, {}) out of bounds", y, x);
        }

        let pixel_offset_in_frame = (y * g.naxis1 + x) * g.bytes_per_pixel;
        let mut spectrum = Vec::with_capacity(g.naxis3);

        for z in 0..g.naxis3 {
            let raw = self.data.bytes(g.data_offset + z * g.frame_bytes + pixel_offset_in_frame, g.bytes_per_pixel)?;
            spectrum.push(self.decode(&raw).first().copied().unwrap_or(f32::NAN));
        }

        Ok(spectrum)
    }

    pub fn check_channel_range(&self, z0: usize, z1: usize) -> Result<()> {
        if z0 > z1 {
            bail!("channel range start {} is after its end {}", z0, z1);
        }
        if z1 >= self.geometry.naxis3 {
            bail!("channel {} is out of range (depth={})", z1, self.geometry.naxis3);
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
        crate::core::astrometry::spectral::spectral_axis(&self.header, self.geometry.naxis3)
    }

    pub fn extract_spectrum_aperture(
        &self,
        shape: &RegionShape,
        background: Option<&RegionShape>,
        subsamples: u8,
    ) -> Result<ApertureSpectrum> {
        let g = &self.geometry;
        let (rows, cols, depth) = (g.naxis2, g.naxis1, g.naxis3);
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

        let (step, stride) = stats_sample_plan(g.naxis3, g.naxis1 * g.naxis2);

        let indices: Vec<usize> = (0..g.naxis3).step_by(step).collect();
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

        let polarised = dir.path().join("polarised.fits");
        let data = f32_samples(3, 2, 10, |z, _, _| z as f32);
        write_bytes(&polarised, &image_hdu("SIMPLE", &[3, 2, 5, 2], -32, &[], data));
        let err = format!("{:#}", LazyCube::open(polarised.to_str().unwrap()).err().expect("NAXIS4=2 must be refused"));
        assert!(err.contains("No 3D data block"), "{}", err);
        let err = format!("{:#}", LazyCube::open(&format!("{}#hdu=0", polarised.to_str().unwrap())).err().unwrap());
        assert!(err.contains("NAXIS4=2"), "{}", err);
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
