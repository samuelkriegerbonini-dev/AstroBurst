use anyhow::{Context, Result};
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
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

    write_png_l8(&pixels, cols, rows, path)
}

pub fn render_grayscale_16bit(data: &Array2<f32>, path: &str) -> Result<()> {
    let (rows, cols) = data.dim();
    let slice = data.as_slice().context("Array not contiguous")?;
    let (min, max) = find_minmax_valid(slice);
    let range = (max - min).max(1e-10);
    let inv_range = 65535.0 / range;

    let pixels: Vec<u16> = slice
        .par_iter()
        .map(|&v| {
            if v.is_finite() && v > PADDING_THRESHOLD {
                ((v - min) * inv_range).clamp(0.0, 65535.0) as u16
            } else {
                0
            }
        })
        .collect();

    write_png_l16(&pixels, cols, rows, path)
}

pub fn render_stretched_8bit(data: &Array2<f32>, path: &str) -> Result<()> {
    let (rows, cols) = data.dim();
    let slice = data.as_slice().context("Array not contiguous")?;

    let pixels: Vec<u8> = slice
        .par_iter()
        .map(|&v| (v.clamp(0.0, 1.0) * 255.0) as u8)
        .collect();

    write_png_l8(&pixels, cols, rows, path)
}

pub fn render_stretched_16bit(data: &Array2<f32>, path: &str) -> Result<()> {
    let (rows, cols) = data.dim();
    let slice = data.as_slice().context("Array not contiguous")?;

    let pixels: Vec<u16> = slice
        .par_iter()
        .map(|&v| (v.clamp(0.0, 1.0) * 65535.0) as u16)
        .collect();

    write_png_l16(&pixels, cols, rows, path)
}

pub fn save_stf_png(pixels: &[u8], width: usize, height: usize, path: &str) -> Result<()> {
    write_png_l8(pixels, width, height, path)
}

pub fn save_stf_png_owned(pixels: Vec<u8>, width: usize, height: usize, path: &str) -> Result<()> {
    write_png_l8(&pixels, width, height, path)
}

fn write_png_l8(pixels: &[u8], width: usize, height: usize, path: &str) -> Result<()> {
    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Sub,
    );
    encoder
        .write_image(pixels, width as u32, height as u32, ColorType::L8.into())
        .context("Failed to write PNG")?;
    Ok(())
}

fn write_png_l16(pixels: &[u16], width: usize, height: usize, path: &str) -> Result<()> {
    let bytes: Vec<u8> = pixels.iter().flat_map(|&v| v.to_be_bytes()).collect();
    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Sub,
    );
    encoder
        .write_image(&bytes, width as u32, height as u32, ColorType::L16.into())
        .context("Failed to write 16-bit PNG")?;
    Ok(())
}
