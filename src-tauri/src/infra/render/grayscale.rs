use anyhow::{Context, Result};
use image::GrayImage;
use ndarray::Array2;
use rayon::prelude::*;

use crate::math::simd::find_minmax_valid;
use crate::types::constants::PADDING_THRESHOLD;

pub fn render_grayscale(data: &Array2<f32>, path: &str) -> Result<()> {
    let (rows, cols) = data.dim();
    let slice = data.as_slice().context("Array not contiguous")?;
    let (min, max) = find_minmax_valid(slice);
    let range = (max - min).max(1e-10);
    let inv_range = 255.0 / range;

    let pixels: Vec<u8> = slice
        .par_iter()
        .map(|&v| {
            if v.is_finite() && v > PADDING_THRESHOLD {
                ((v - min) * inv_range).clamp(0.0, 255.0) as u8
            } else {
                0
            }
        })
        .collect();

    let img = GrayImage::from_raw(cols as u32, rows as u32, pixels)
        .context("Failed to create grayscale image from raw pixels")?;
    img.save(path).context("Failed to save grayscale PNG")?;
    Ok(())
}

pub fn save_stf_png(pixels: &[u8], width: usize, height: usize, path: &str) -> Result<()> {
    let img = GrayImage::from_raw(width as u32, height as u32, pixels.to_vec())
        .context("Failed to create grayscale image from STF pixels")?;
    img.save(path).context("Failed to save STF PNG")?;
    Ok(())
}
