use std::fs::File;
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use ndarray::{Array2, Array3};
use rayon::prelude::*;

use crate::types::HduHeader;
use crate::math::median::f32_cmp;
use crate::core::imaging::stats;
use crate::infra::fits::reader::{create_mmap_random, decode_pixels, decode_single_pixel, parse_header_at};

pub use crate::core::cube::lazy::{
    CubeGeometry, LruFrameCache, LazyCubeResult,
    normalize_frame_with_stats,
};
pub use crate::core::cube::eager::GlobalCubeStats;

const DEFAULT_CACHE_SIZE: usize = 64;
const BATCH_SIZE: usize = 32;

pub struct LazyCube {
    _file: File,
    mmap: Mmap,
    pub header: HduHeader,
    pub geometry: CubeGeometry,
    cache: Mutex<LruFrameCache>,
}

impl LazyCube {
    pub fn open(path: &str) -> Result<Self> {
        Self::open_with_cache(path, DEFAULT_CACHE_SIZE)
    }

    pub fn open_with_cache(path: &str, cache_frames: usize) -> Result<Self> {
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
                let naxis1 = header.get_i64("NAXIS1").unwrap_or(0) as usize;
                let naxis2 = header.get_i64("NAXIS2").unwrap_or(0) as usize;
                let naxis3 = naxis3 as usize;

                let bitpix = header.get_i64("BITPIX")
                    .context("Missing BITPIX")?;
                let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
                let frame_bytes = naxis1 * naxis2 * bytes_per_pixel;
                let data_offset = header.data_offset(parsed.header_start);

                let total_bytes = frame_bytes * naxis3;
                let data_end = data_offset + total_bytes;
                if data_end > mmap.len() {
                    bail!(
                        "Cube data [{}, {}) exceeds file size {}",
                        data_offset,
                        data_end,
                        mmap.len()
                    );
                }

                let bzero = header.get_f64("BZERO").unwrap_or(0.0);
                let bscale = header.get_f64("BSCALE").unwrap_or(1.0);

                let geometry = CubeGeometry {
                    naxis1,
                    naxis2,
                    naxis3,
                    bitpix,
                    bytes_per_pixel,
                    bzero,
                    bscale,
                    data_offset,
                    frame_bytes,
                };

                return Ok(LazyCube {
                    _file: file,
                    mmap,
                    header,
                    geometry,
                    cache: Mutex::new(LruFrameCache::new(cache_frames)),
                });
            }

            offset = parsed.next_hdu_offset;
        }

        bail!("No 3D data block found in FITS file")
    }

    pub fn get_frame(&self, z: usize) -> Result<Array2<f32>> {
        if z >= self.geometry.naxis3 {
            bail!(
                "Frame index {} out of range (depth={})",
                z,
                self.geometry.naxis3
            );
        }

        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(frame) = cache.get(z) {
                return Ok(frame);
            }
        }

        let g = &self.geometry;
        let start = g.data_offset + z * g.frame_bytes;
        let end = start + g.frame_bytes;
        let raw = &self.mmap[start..end];

        let pixels = decode_pixels(raw, g.bitpix, g.bscale, g.bzero);
        let frame = Array2::from_shape_vec((g.naxis2, g.naxis1), pixels)
            .context("Failed to reshape frame pixels")?;

        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(z, frame.clone());
        }

        Ok(frame)
    }

    fn decode_frame_nocache(&self, z: usize) -> Vec<f32> {
        let g = &self.geometry;
        let start = g.data_offset + z * g.frame_bytes;
        let end = start + g.frame_bytes;
        let raw = &self.mmap[start..end];
        decode_pixels(raw, g.bitpix, g.bscale, g.bzero)
    }

    pub fn get_frame_range(&self, start_z: usize, end_z: usize) -> Result<Array3<f32>> {
        let end_z = end_z.min(self.geometry.naxis3);
        if start_z >= end_z {
            bail!("Invalid frame range");
        }

        let count = end_z - start_z;
        let g = &self.geometry;
        let byte_start = g.data_offset + start_z * g.frame_bytes;
        let byte_end = byte_start + count * g.frame_bytes;
        let raw = &self.mmap[byte_start..byte_end];

        let pixels = decode_pixels(raw, g.bitpix, g.bscale, g.bzero);
        let cube = Array3::from_shape_vec((count, g.naxis2, g.naxis1), pixels)
            .context("Failed to reshape frame range")?;
        Ok(cube)
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
                    if stats::is_valid_pixel(v) {
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
        let npix = rows * cols;
        let depth = g.naxis3;

        let mut pixel_vals: Vec<Vec<f32>> = Vec::with_capacity(npix);
        for _ in 0..npix {
            pixel_vals.push(Vec::with_capacity(depth));
        }

        for batch_start in (0..depth).step_by(BATCH_SIZE) {
            let batch_end = (batch_start + BATCH_SIZE).min(depth);

            let frames: Vec<Vec<f32>> = (batch_start..batch_end)
                .into_par_iter()
                .map(|z| self.decode_frame_nocache(z))
                .collect();

            for pixels in &frames {
                for i in 0..npix {
                    let v = pixels[i];
                    if stats::is_valid_pixel(v) {
                        pixel_vals[i].push(v);
                    }
                }
            }
        }

        let result_data: Vec<f32> = pixel_vals
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

        Ok(Array2::from_shape_vec((rows, cols), result_data)
            .context("Failed to reshape collapsed median")?)
    }

    pub fn compute_global_stats_streaming(&self) -> Result<GlobalCubeStats> {
        let g = &self.geometry;

        let sample_frames = 32.min(g.naxis3);
        let step = if g.naxis3 > sample_frames {
            g.naxis3 / sample_frames
        } else {
            1
        };

        let indices: Vec<usize> = (0..g.naxis3).step_by(step).collect();
        let frame_samples: Vec<Vec<f32>> = indices
            .par_iter()
            .map(|&z| {
                let pixels = self.decode_frame_nocache(z);
                pixels.into_iter().filter(|v| stats::is_valid_pixel(*v)).collect()
            })
            .collect();

        let mut sampled: Vec<f32> = Vec::new();
        for chunk in frame_samples {
            sampled.extend(chunk);
        }

        if sampled.is_empty() {
            return Ok(GlobalCubeStats {
                median: 0.0,
                sigma: 1.0,
                low: 0.0,
                high: 1.0,
            });
        }

        let n = sampled.len();
        let mid = n / 2;
        sampled.select_nth_unstable_by(mid, |a, b| f32_cmp(a, b));
        let median = sampled[mid];

        let mut deviations: Vec<f32> = sampled.iter().map(|v| (v - median).abs()).collect();
        let dev_mid = deviations.len() / 2;
        deviations.select_nth_unstable_by(dev_mid, |a, b| f32_cmp(a, b));
        let sigma = (deviations[dev_mid] * 1.4826).max(1e-10);

        sampled.sort_unstable_by(|a, b| f32_cmp(a, b));
        let low = sampled[(n as f64 * 0.01) as usize];
        let high = sampled[((n as f64 * 0.999) as usize).min(n - 1)];

        Ok(GlobalCubeStats {
            median,
            sigma,
            low,
            high,
        })
    }
}

pub fn process_cube_lazy(
    fits_path: &str,
    output_dir: &str,
    frame_step: usize,
) -> Result<LazyCubeResult> {
    use std::fs;

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
    let wavelengths = crate::domain::cube::build_wavelength_axis(&lazy.header);
    let frames_dir = format!("{}/frames", output_dir);

    fs::create_dir_all(&frames_dir)
        .with_context(|| format!("Failed to create frames dir {}", frames_dir))?;

    let stats = lazy.compute_global_stats_streaming()?;
    let step = frame_step.max(1);
    let mut frame_count = 0;

    for z in (0..depth).step_by(step) {
        let frame = lazy.get_frame(z)?;
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
    })
}
