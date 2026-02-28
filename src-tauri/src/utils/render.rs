use anyhow::{Context, Result};
use image::{GrayImage, Luma};
use ndarray::Array2;

use crate::utils::simd::find_minmax_simd;

pub fn render_grayscale(data: &Array2<f32>, path: &str) -> Result<()> {
    let (rows, cols) = data.dim();

    let slice = data.as_slice().expect("Array2 must be contiguous");
    let (min, max) = find_minmax_simd(slice);
    let range = (max - min).max(1e-10);
    let inv_range = 255.0 / range;

    let mut img = GrayImage::new(cols as u32, rows as u32);
    for y in 0..rows {
        for x in 0..cols {
            let v = data[[y, x]];
            let byte = if v.is_finite() {
                ((v - min) * inv_range).clamp(0.0, 255.0) as u8
            } else {
                0
            };
            img.put_pixel(x as u32, y as u32, Luma([byte]));
        }
    }

    img.save(path)
        .with_context(|| format!("Failed to save grayscale image to {}", path))?;
    Ok(())
}
