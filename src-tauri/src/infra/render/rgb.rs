use anyhow::{Context, Result};
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use ndarray::Array2;
use rayon::prelude::*;

pub fn render_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    path: &str,
) -> Result<()> {
    let (rows, cols) = r.dim();
    let r_slice = r.as_slice().context("R channel not contiguous")?;
    let g_slice = g.as_slice().context("G channel not contiguous")?;
    let b_slice = b.as_slice().context("B channel not contiguous")?;

    let npix = rows * cols;
    let mut pixels = vec![0u8; npix * 3];

    pixels
        .par_chunks_mut(cols * 3)
        .enumerate()
        .for_each(|(y, row_buf)| {
            let base = y * cols;
            for x in 0..cols {
                let i = base + x;
                let o = x * 3;
                row_buf[o] = (r_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
                row_buf[o + 1] = (g_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
                row_buf[o + 2] = (b_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
            }
        });

    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Sub,
    );
    encoder
        .write_image(&pixels, cols as u32, rows as u32, ColorType::Rgb8.into())
        .context("Failed to write RGB PNG")?;

    Ok(())
}

pub fn render_rgb_16bit(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    path: &str,
) -> Result<()> {
    let (rows, cols) = r.dim();
    let r_slice = r.as_slice().context("R channel not contiguous")?;
    let g_slice = g.as_slice().context("G channel not contiguous")?;
    let b_slice = b.as_slice().context("B channel not contiguous")?;

    let npix = rows * cols;
    let mut pixels = vec![0u16; npix * 3];

    pixels
        .par_chunks_mut(cols * 3)
        .enumerate()
        .for_each(|(y, row_buf)| {
            let base = y * cols;
            for x in 0..cols {
                let i = base + x;
                let o = x * 3;
                row_buf[o] = (r_slice[i].clamp(0.0, 1.0) * 65535.0) as u16;
                row_buf[o + 1] = (g_slice[i].clamp(0.0, 1.0) * 65535.0) as u16;
                row_buf[o + 2] = (b_slice[i].clamp(0.0, 1.0) * 65535.0) as u16;
            }
        });

    let bytes: Vec<u8> = pixels.iter().flat_map(|&v| v.to_be_bytes()).collect();

    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(4 * 1024 * 1024, file);
    let encoder = PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Sub,
    );
    encoder
        .write_image(&bytes, cols as u32, rows as u32, ColorType::Rgb16.into())
        .context("Failed to write 16-bit RGB PNG")?;

    Ok(())
}
