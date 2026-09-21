use std::collections::HashMap;
use std::fs::File;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::analysis::aperture::{annulus, circular_aperture, total_weight, AperturePixel};
use crate::core::astrometry::spectral::SpectralAxis;
use crate::core::imaging::region::RegionShape;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::HduHeader;
use crate::math::median::f32_cmp;
use crate::math::{exact_median_mut, sigma_clipped_stats};
use crate::infra::fits::reader::{create_mmap_random, decode_pixels, decode_single_pixel, parse_header_at};

#[derive(Debug, Clone)]
pub struct CubeGeometry {
    pub naxis1: usize,
    pub naxis2: usize,
    pub naxis3: usize,
    pub bitpix: i64,
    pub bytes_per_pixel: usize,
    pub bzero: f64,
    pub bscale: f64,
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

    pub fn clear(&mut self) {
        self.entries.clear();
        self.current_bytes = 0;
        self.access_counter = 0;
    }
}

pub use crate::core::cube::eager::GlobalCubeStats;

pub fn normalize_frame_with_stats(data: &Array2<f32>, stats: &GlobalCubeStats) -> Array2<f32> {
    crate::core::cube::eager::normalize_with_global(data, stats)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LazyCubeResult {
    pub dimensions: [usize; 3],
    pub collapsed_path: String,
    pub collapsed_median_path: String,
    pub frames_dir: String,
    pub frame_count: usize,
    pub total_frames: usize,
    pub center_spectrum: Vec<f32>,
    pub wavelengths: Option<Vec<f64>>,
    pub elapsed_ms: u64,
}

const DEFAULT_CACHE_BYTES: usize = 256 << 20;
const BATCH_SIZE: usize = 32;
const STATS_SAMPLE_FRAMES: usize = 32;
const STATS_TARGET_SAMPLES: usize = 4_000_000;

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

fn median_or_nan(values: &mut Vec<f32>) -> f32 {
    if values.is_empty() {
        f32::NAN
    } else {
        exact_median_mut(values) as f32
    }
}

pub struct LazyCube {
    _file: File,
    mmap: Mmap,
    pub header: HduHeader,
    pub geometry: CubeGeometry,
    cache: Mutex<LruFrameCache>,
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
    pub fn open(path: &str) -> Result<Self> {
        Self::open_with_cache(path, DEFAULT_CACHE_BYTES)
    }

    pub fn open_with_cache(path: &str, cache_bytes: usize) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open FITS file {}", path))?;
        let mmap = create_mmap_random(&file)
            .context("mmap failed for lazy cube")?;

        let mut offset: usize = 0;
        while offset < mmap.len() {
            let parsed = parse_header_at(&mmap, offset)
                .context("Header parse failed in lazy cube")?;
            let header = parsed.header;

            let naxis = header.get_i64("NAXIS").unwrap_or(0);
            let naxis3 = header.get_i64("NAXIS3").unwrap_or(0);

            if naxis == 3 && naxis3 > 1 {
                let naxis1_i = header.get_i64("NAXIS1").unwrap_or(0);
                let naxis2_i = header.get_i64("NAXIS2").unwrap_or(0);
                if naxis1_i <= 0 || naxis2_i <= 0 {
                    bail!("Invalid cube dimensions NAXIS1={}, NAXIS2={}", naxis1_i, naxis2_i);
                }
                let naxis1 = naxis1_i as usize;
                let naxis2 = naxis2_i as usize;
                let naxis3 = naxis3 as usize;

                let bitpix = header.get_i64("BITPIX")
                    .context("Missing BITPIX")?;
                let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
                if bytes_per_pixel == 0 {
                    bail!("Unsupported BITPIX={}", bitpix);
                }
                let (frame_bytes, total_bytes) =
                    checked_cube_bytes(naxis1, naxis2, naxis3, bytes_per_pixel)?;
                let data_offset = parsed.data_start;

                let data_end = data_offset
                    .checked_add(total_bytes)
                    .context("Cube data end overflow")?;
                if data_end > mmap.len() {
                    bail!(
                        "Cube data [{}, {}) exceeds file size {}",
                        data_offset, data_end, mmap.len()
                    );
                }

                let bzero = header.get_f64("BZERO").unwrap_or(0.0);
                let bscale = header.get_f64("BSCALE").unwrap_or(1.0);

                let geometry = CubeGeometry {
                    naxis1, naxis2, naxis3,
                    bitpix, bytes_per_pixel, bzero, bscale,
                    data_offset, frame_bytes,
                };

                return Ok(LazyCube {
                    _file: file, mmap, header, geometry,
                    cache: Mutex::new(LruFrameCache::new(cache_bytes)),
                });
            }

            offset = parsed.next_hdu_offset;
        }

        bail!("No 3D data block found in FITS file")
    }

    pub fn get_frame(&self, z: usize) -> Result<Arc<Array2<f32>>> {
        if z >= self.geometry.naxis3 {
            bail!("Frame index {} out of range (depth={})", z, self.geometry.naxis3);
        }

        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(frame) = cache.get(z) {
                return Ok(frame);
            }
        }

        let frame = Arc::new(self.frame_uncached(z)?);

        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(z, Arc::clone(&frame));
        }

        Ok(frame)
    }

    pub fn frame_uncached(&self, z: usize) -> Result<Array2<f32>> {
        if z >= self.geometry.naxis3 {
            bail!("Frame index {} out of range (depth={})", z, self.geometry.naxis3);
        }
        let g = &self.geometry;
        let start = g.data_offset + z * g.frame_bytes;
        let end = start + g.frame_bytes;
        let raw = &self.mmap[start..end];

        let pixels = decode_pixels(raw, g.bitpix, g.bscale, g.bzero);
        Array2::from_shape_vec((g.naxis2, g.naxis1), pixels)
            .context("Failed to reshape frame pixels")
    }

    fn decode_frame_nocache(&self, z: usize) -> Vec<f32> {
        let g = &self.geometry;
        let start = g.data_offset + z * g.frame_bytes;
        let end = start + g.frame_bytes;
        let raw = &self.mmap[start..end];
        decode_pixels(raw, g.bitpix, g.bscale, g.bzero)
    }

    pub fn extract_spectrum_at(&self, y: usize, x: usize) -> Result<Vec<f32>> {
        let g = &self.geometry;
        if y >= g.naxis2 || x >= g.naxis1 {
            bail!("Pixel ({}, {}) out of bounds", y, x);
        }

        let pixel_offset_in_frame = (y * g.naxis1 + x) * g.bytes_per_pixel;
        let mut spectrum = Vec::with_capacity(g.naxis3);

        for z in 0..g.naxis3 {
            let abs_offset = g.data_offset + z * g.frame_bytes + pixel_offset_in_frame;
            let raw = &self.mmap[abs_offset..abs_offset + g.bytes_per_pixel];
            let val = decode_single_pixel(raw, g.bitpix, g.bscale, g.bzero);
            spectrum.push(val);
        }

        Ok(spectrum)
    }

    pub fn clear_cache(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
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
        if z >= g.naxis3 {
            bail!("Frame index {} out of range (depth={})", z, g.naxis3);
        }
        if row_start + row_count > g.naxis2 {
            bail!("rows {}..{} exceed the frame height {}", row_start, row_start + row_count, g.naxis2);
        }
        let start = g.data_offset + z * g.frame_bytes + row_start * g.naxis1 * g.bytes_per_pixel;
        let end = start + row_count * g.naxis1 * g.bytes_per_pixel;
        Ok(decode_pixels(&self.mmap[start..end], g.bitpix, g.bscale, g.bzero))
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
        if pixels.is_empty() {
            bail!("{} region covers no image pixels", shape.kind());
        }
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
        let (mut y0, mut y1) = row_span(&pixels).expect("aperture has pixels");
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
                for i in 0..npix {
                    let v = pixels[i];
                    if v.is_finite() {
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
                    for i in 0..band_npix {
                        let v = pixels[i];
                        if v.is_finite() {
                            samples[i].push(v);
                        }
                    }
                }
            }

            let band_result: Vec<f32> = samples.into_par_iter().map(|mut vals| median_or_nan(&mut vals)).collect();
            data.extend_from_slice(&band_result);
        }

        Array2::from_shape_vec((rows, cols), data).context("Failed to reshape collapsed median range")
    }

    pub fn collapse_mean_lazy(&self) -> Result<Array2<f32>> {
        let g = &self.geometry;
        let (rows, cols) = (g.naxis2, g.naxis1);
        let npix = rows * cols;
        let depth = g.naxis3;

        let mut sum = vec![0.0f64; npix];
        let mut count = vec![0u32; npix];

        for batch_start in (0..depth).step_by(BATCH_SIZE) {
            let batch_end = (batch_start + BATCH_SIZE).min(depth);
            let batch_count = batch_end - batch_start;

            let frames: Vec<Vec<f32>> = (batch_start..batch_end)
                .into_par_iter()
                .map(|z| self.decode_frame_nocache(z))
                .collect();

            for frame_idx in 0..batch_count {
                let pixels = &frames[frame_idx];
                for i in 0..npix {
                    let v = pixels[i];
                    if v.is_finite() && v != 0.0 {
                        sum[i] += v as f64;
                        count[i] += 1;
                    }
                }
            }
        }

        let result_data: Vec<f32> = sum
            .into_par_iter()
            .zip(count.into_par_iter())
            .map(|(s, c)| if c > 0 { (s / c as f64) as f32 } else { 0.0 })
            .collect();

        Ok(Array2::from_shape_vec((rows, cols), result_data)
            .context("Failed to reshape collapsed mean")?)
    }

    pub fn collapse_median_lazy(&self) -> Result<Array2<f32>> {
        let g = &self.geometry;
        let (rows, cols) = (g.naxis2, g.naxis1);
        let depth = g.naxis3;

        const TARGET_BAND_BYTES: usize = 256 * 1024 * 1024;
        let bytes_per_row = cols.max(1) * depth.max(1) * 4;
        let band_rows = (TARGET_BAND_BYTES / bytes_per_row.max(1)).clamp(1, rows.max(1));

        let mut result_data: Vec<f32> = Vec::with_capacity(rows * cols);

        for band_start in (0..rows).step_by(band_rows) {
            let band_end = (band_start + band_rows).min(rows);
            let band_npix = (band_end - band_start) * cols;

            let mut pixel_vals: Vec<Vec<f32>> = vec![Vec::new(); band_npix];

            for batch_start in (0..depth).step_by(BATCH_SIZE) {
                let batch_end = (batch_start + BATCH_SIZE).min(depth);

                let frames: Vec<Vec<f32>> = (batch_start..batch_end)
                    .into_par_iter()
                    .map(|z| {
                        let start = g.data_offset
                            + z * g.frame_bytes
                            + band_start * cols * g.bytes_per_pixel;
                        let end = start + band_npix * g.bytes_per_pixel;
                        decode_pixels(&self.mmap[start..end], g.bitpix, g.bscale, g.bzero)
                    })
                    .collect();

                for pixels in &frames {
                    for i in 0..band_npix {
                        let v = pixels[i];
                        if v.is_finite() && v != 0.0 {
                            pixel_vals[i].push(v);
                        }
                    }
                }
            }

            let band_result: Vec<f32> = pixel_vals
                .into_par_iter()
                .map(|mut vals| {
                    if vals.is_empty() {
                        return 0.0;
                    }
                    let mid = vals.len() / 2;
                    vals.select_nth_unstable_by(mid, |a, b| f32_cmp(a, b));
                    vals[mid]
                })
                .collect();
            result_data.extend_from_slice(&band_result);
        }

        Ok(Array2::from_shape_vec((rows, cols), result_data)
            .context("Failed to reshape collapsed median")?)
    }

    pub fn compute_global_stats_streaming(&self) -> Result<GlobalCubeStats> {
        let g = &self.geometry;

        let (step, stride) = stats_sample_plan(g.naxis3, g.naxis1 * g.naxis2);

        let indices: Vec<usize> = (0..g.naxis3).step_by(step).collect();
        let frame_samples: Vec<Vec<f32>> = indices
            .par_iter()
            .map(|&z| {
                let pixels = self.decode_frame_nocache(z);
                pixels
                    .into_iter()
                    .step_by(stride)
                    .filter(|v| v.is_finite() && *v != 0.0)
                    .collect()
            })
            .collect();

        let total: usize = frame_samples.iter().map(Vec::len).sum();
        let mut sampled: Vec<f32> = Vec::with_capacity(total);
        for chunk in frame_samples {
            sampled.extend(chunk);
        }

        if sampled.is_empty() {
            return Ok(GlobalCubeStats { median: 0.0, sigma: 1.0, low: 0.0, high: 1.0 });
        }

        let n = sampled.len();
        let mid = n / 2;
        sampled.select_nth_unstable_by(mid, |a, b| f32_cmp(a, b));
        let median = sampled[mid];

        let mut deviations: Vec<f32> = sampled.iter().map(|v| (v - median).abs()).collect();
        let dev_mid = deviations.len() / 2;
        deviations.select_nth_unstable_by(dev_mid, |a, b| f32_cmp(a, b));
        let sigma = (deviations[dev_mid] * MAD_TO_SIGMA as f32).max(1e-10);

        let low_idx = (n as f64 * 0.01) as usize;
        let high_idx = ((n as f64 * 0.999) as usize).min(n - 1);
        sampled.select_nth_unstable_by(low_idx, |a, b| f32_cmp(a, b));
        let low = sampled[low_idx];
        sampled.select_nth_unstable_by(high_idx, |a, b| f32_cmp(a, b));
        let high = sampled[high_idx];

        Ok(GlobalCubeStats { median, sigma, low, high })
    }
}

pub fn process_cube_lazy(
    fits_path: &str,
    output_dir: &str,
    frame_step: usize,
) -> Result<LazyCubeResult> {
    use std::fs;

    let t0 = std::time::Instant::now();
    let lazy = LazyCube::open(fits_path)?;
    let g = &lazy.geometry;
    let (depth, rows, cols) = (g.naxis3, g.naxis2, g.naxis1);

    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output dir {}", output_dir))?;

    let collapsed = lazy.collapse_mean_lazy()?;
    let collapsed_norm = crate::core::imaging::normalize::robust_asinh_preview(&collapsed);
    let collapsed_path = format!("{}/collapsed_mean.png", output_dir);
    crate::infra::render::render_grayscale(&collapsed_norm, &collapsed_path)?;

    lazy.clear_cache();

    let collapsed_med = lazy.collapse_median_lazy()?;
    let collapsed_med_norm = crate::core::imaging::normalize::robust_asinh_preview(&collapsed_med);
    let collapsed_med_path = format!("{}/collapsed_median.png", output_dir);
    crate::infra::render::render_grayscale(&collapsed_med_norm, &collapsed_med_path)?;

    lazy.clear_cache();

    let center_y = rows / 2;
    let center_x = cols / 2;
    let spectrum = lazy.extract_spectrum_at(center_y, center_x)?;
    let wavelengths = crate::core::cube::eager::build_wavelength_axis(&lazy.header);
    let frames_dir = format!("{}/frames", output_dir);

    fs::create_dir_all(&frames_dir)
        .with_context(|| format!("Failed to create frames dir {}", frames_dir))?;

    let stats = lazy.compute_global_stats_streaming()?;
    let step = frame_step.max(1);
    let mut frame_count = 0;

    for z in (0..depth).step_by(step) {
        let frame = lazy.frame_uncached(z)?;
        let normalized = normalize_frame_with_stats(&frame, &stats);
        let path = format!("{}/frame_{:04}.png", frames_dir, frame_count);
        crate::infra::render::render_grayscale(&normalized, &path)?;
        frame_count += 1;
    }

    Ok(LazyCubeResult {
        dimensions: [cols, rows, depth],
        collapsed_path,
        collapsed_median_path: collapsed_med_path,
        frames_dir,
        frame_count,
        total_frames: depth,
        center_spectrum: spectrum,
        wavelengths,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
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

    pub fn write_cube(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        depth: usize,
        cards: &[(&str, &str)],
        value: impl Fn(usize, usize, usize) -> f32,
    ) {
        let mut header = Vec::new();
        let mut push = |key: &str, value: &str| {
            let mut card = format!("{key:<8}= {value:>20}").into_bytes();
            card.resize(80, b' ');
            header.extend_from_slice(&card);
        };
        push("SIMPLE", "T");
        push("BITPIX", "-32");
        push("NAXIS", "3");
        push("NAXIS1", &cols.to_string());
        push("NAXIS2", &rows.to_string());
        push("NAXIS3", &depth.to_string());
        for (key, value) in cards {
            push(key, value);
        }
        let mut end = b"END".to_vec();
        end.resize(80, b' ');
        header.extend_from_slice(&end);
        while header.len() % BLOCK_SIZE != 0 {
            header.push(b' ');
        }
        let mut data = Vec::with_capacity(cols * rows * depth * 4);
        for z in 0..depth {
            for y in 0..rows {
                for x in 0..cols {
                    data.extend_from_slice(&value(z, y, x).to_be_bytes());
                }
            }
        }
        while data.len() % BLOCK_SIZE != 0 {
            data.push(0);
        }
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(&header).unwrap();
        file.write_all(&data).unwrap();
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
            assert!(v.is_finite());
            assert!(v > crate::types::constants::PADDING_THRESHOLD && v <= 1.0, "{}", v);
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

    fn write_synthetic_cube(path: &std::path::Path, naxis1: usize, naxis2: usize, naxis3: usize) {
        use std::io::Write;
        let mut header = String::new();
        for card in [
            "SIMPLE  =                    T".to_string(),
            "BITPIX  =                  -32".to_string(),
            "NAXIS   =                    3".to_string(),
            format!("NAXIS1  = {:>20}", naxis1),
            format!("NAXIS2  = {:>20}", naxis2),
            format!("NAXIS3  = {:>20}", naxis3),
            "END".to_string(),
        ] {
            header.push_str(&format!("{:<80}", card));
        }
        let mut bytes = header.into_bytes();
        bytes.resize(2880, b' ');
        for z in 0..naxis3 {
            for _ in 0..(naxis1 * naxis2) {
                bytes.extend_from_slice(&((z + 1) as f32).to_be_bytes());
            }
        }
        let padded = (bytes.len() + 2879) / 2880 * 2880;
        bytes.resize(padded, 0);
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&bytes).unwrap();
    }

    #[test]
    fn global_stats_streaming_samples_at_most_32_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube40.fits");
        write_synthetic_cube(&path, 4, 4, 40);
        let lazy = LazyCube::open(path.to_str().unwrap()).unwrap();

        let stats = lazy.compute_global_stats_streaming().unwrap();
        assert_eq!(stats.median, 21.0);
        assert_eq!(stats.low, 1.0);
        assert_eq!(stats.high, 39.0);
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

    #[test]
    fn zero_valued_samples_count_and_nan_samples_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zeros.fits");
        write_cube(&path, 4, 4, 6, &[], |z, y, x| {
            if z == 2 {
                0.0
            } else if z == 4 && y == 0 && x == 0 {
                f32::NAN
            } else {
                2.0
            }
        });
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let whole = RegionShape::Box { x: 1.5, y: 1.5, width: 4.0, height: 4.0, angle: 0.0 };
        let spectrum = cube.extract_spectrum_aperture(&whole, None, 1).unwrap();
        assert_eq!(spectrum.npix, 16.0);
        assert_eq!(spectrum.sum[2], 0.0);
        assert_eq!(spectrum.mean[2], 0.0);
        assert_eq!(spectrum.sum[4], 30.0);
        assert_eq!(spectrum.mean[4], 2.0);
        assert_eq!(spectrum.sum[0], 32.0);

        let mean = cube.collapse_range(0, 5, CollapseMode::Mean).unwrap();
        assert!((mean[[1, 1]] - 10.0 / 6.0).abs() < 1e-6, "{}", mean[[1, 1]]);
        assert!((mean[[0, 0]] - 1.6).abs() < 1e-6, "{}", mean[[0, 0]]);
        let sum = cube.collapse_range(0, 5, CollapseMode::Sum).unwrap();
        assert_eq!(sum[[0, 0]], 8.0);
        let median = cube.collapse_range(0, 5, CollapseMode::Median).unwrap();
        assert_eq!(median[[0, 0]], 2.0);
        assert_eq!(median[[1, 1]], 2.0);
        let only_zeros = cube.collapse_range(2, 2, CollapseMode::Mean).unwrap();
        assert_eq!(only_zeros[[3, 3]], 0.0);
        let only_nan = cube.collapse_range(4, 4, CollapseMode::Median).unwrap();
        assert!(only_nan[[0, 0]].is_nan());
        assert_eq!(only_nan[[0, 1]], 2.0);
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
