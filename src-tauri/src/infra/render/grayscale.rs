use anyhow::{Context, Result};
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use ndarray::Array2;
use rayon::prelude::*;

use crate::math::simd::find_minmax_simd;
use crate::types::constants::PADDING_THRESHOLD;

pub fn render_grayscale(data: &Array2<f32>, path: &str) -> Result<()> {
    let (rows, cols) = data.dim();
    let slice = data.as_slice().context("Array not contiguous")?;
    let (min, max) = find_minmax_simd(slice);
    let range = (max - min).max(1e-10);
    let inv_range = 255.0 / range;

    let pixels: Vec<u8> = slice
        .par_iter()
        .map(|&v| {
            if v.is_finite() && v > PADDING_THRESHOLD {
                ((v - min) * inv_range).round().clamp(0.0, 255.0) as u8
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
    let (min, max) = find_minmax_simd(slice);
    let range = (max - min).max(1e-10);
    let inv_range = 65535.0 / range;

    let mut pixels = vec![0u8; slice.len() * 2];
    pixels
        .par_chunks_exact_mut(2)
        .zip(slice.par_iter())
        .for_each(|(out, &v)| {
            let q = if v.is_finite() && v > PADDING_THRESHOLD {
                ((v - min) * inv_range).round().clamp(0.0, 65535.0) as u16
            } else {
                0
            };
            out.copy_from_slice(&q.to_ne_bytes());
        });

    write_png_l16(&pixels, cols, rows, path)
}

pub fn render_stretched_8bit(data: &Array2<f32>, path: &str) -> Result<()> {
    let (rows, cols) = data.dim();
    let slice = data.as_slice().context("Array not contiguous")?;

    let pixels: Vec<u8> = slice
        .par_iter()
        .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();

    write_png_l8(&pixels, cols, rows, path)
}

pub fn render_stretched_16bit(data: &Array2<f32>, path: &str) -> Result<()> {
    let (rows, cols) = data.dim();
    let slice = data.as_slice().context("Array not contiguous")?;

    let mut pixels = vec![0u8; slice.len() * 2];
    pixels
        .par_chunks_exact_mut(2)
        .zip(slice.par_iter())
        .for_each(|(out, &v)| {
            let q = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
            out.copy_from_slice(&q.to_ne_bytes());
        });

    write_png_l16(&pixels, cols, rows, path)
}

pub fn save_stf_png(pixels: Vec<u8>, width: usize, height: usize, path: &str) -> Result<()> {
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

fn write_png_l16(ne_bytes: &[u8], width: usize, height: usize, path: &str) -> Result<()> {
    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Sub,
    );
    encoder
        .write_image(ne_bytes, width as u32, height as u32, ColorType::L16.into())
        .context("Failed to write 16-bit PNG")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use image::codecs::png::PngEncoder;
    use image::{ColorType, ImageEncoder};

    fn encode_l16(bytes: &[u8], w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let encoder = PngEncoder::new_with_quality(
            std::io::Cursor::new(&mut out),
            image::codecs::png::CompressionType::Default,
            image::codecs::png::FilterType::Sub,
        );
        encoder
            .write_image(bytes, w, h, ColorType::L16.into())
            .unwrap();
        out
    }

    #[test]
    fn test_l16_native_endian_roundtrip() {
        let vals: [u16; 4] = [0, 1000, 30000, 65535];
        let mut ne = Vec::new();
        for v in vals {
            ne.extend_from_slice(&v.to_ne_bytes());
        }
        let png = encode_l16(&ne, 4, 1);
        let decoded = image::load_from_memory(&png).unwrap().into_luma16();
        assert_eq!(decoded.as_raw().as_slice(), &vals);
    }

    #[test]
    fn test_render_stretched_16bit_roundtrip() {
        use ndarray::Array2;

        let img = Array2::from_shape_fn((4, 8), |(y, x)| (y * 8 + x) as f32 / 31.0);
        let dir = std::env::temp_dir().join("astroburst_l16_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("grad16.png");
        let path_str = path.to_str().unwrap();

        super::render_stretched_16bit(&img, path_str).unwrap();

        let decoded = image::open(path_str).unwrap().into_luma16();
        assert_eq!(decoded.dimensions(), (8, 4));
        for (y, x) in [(0usize, 1usize), (1, 3), (2, 5), (3, 7)] {
            let expected = ((y * 8 + x) as f32 / 31.0 * 65535.0).round() as u16;
            let got = decoded.get_pixel(x as u32, y as u32).0[0];
            assert_eq!(got, expected, "pixel ({},{})", y, x);
        }

        let _ = std::fs::remove_file(path);
    }
}
