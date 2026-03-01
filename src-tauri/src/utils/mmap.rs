use std::collections::HashMap;
use std::fs::File;

use anyhow::{bail, Context, Result};
use memmap2::{Mmap, MmapOptions};
use ndarray::{Array2, Array3};

use crate::model::HduHeader;
use crate::utils::constants::BLOCK_SIZE;

pub fn create_mmap(file: &File) -> Result<Mmap> {
    let mmap = unsafe { MmapOptions::new().map(file).context("mmap failed")? };
    #[cfg(unix)]
    {
        let _ = mmap.advise(memmap2::Advice::Sequential);
    }
    Ok(mmap)
}

pub fn create_mmap_random(file: &File) -> Result<Mmap> {
    let mmap = unsafe { MmapOptions::new().map(file).context("mmap random failed")? };
    #[cfg(unix)]
    {
        let _ = mmap.advise(memmap2::Advice::Random);
    }
    Ok(mmap)
}

#[inline]
fn scaling(header: &HduHeader) -> (f64, f64) {
    let bzero = header.get_f64("BZERO").unwrap_or(0.0);
    let bscale = header.get_f64("BSCALE").unwrap_or(1.0);
    (bzero, bscale)
}

pub fn decode_pixels(data: &[u8], bitpix: i64, bscale: f64, bzero: f64) -> Vec<f32> {
    match bitpix {
        8 => data
            .iter()
            .map(|&b| (b as f64 * bscale + bzero) as f32)
            .collect(),
        16 => data
            .chunks_exact(2)
            .map(|c| {
                let v = i16::from_be_bytes([c[0], c[1]]);
                (v as f64 * bscale + bzero) as f32
            })
            .collect(),
        32 => data
            .chunks_exact(4)
            .map(|c| {
                let v = i32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                (v as f64 * bscale + bzero) as f32
            })
            .collect(),
        -32 => data
            .chunks_exact(4)
            .map(|c| {
                let v = f32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                (v as f64 * bscale + bzero) as f32
            })
            .collect(),
        -64 => data
            .chunks_exact(8)
            .map(|c| {
                let v = f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
                (v * bscale + bzero) as f32
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn decode_single_pixel(raw: &[u8], bitpix: i64, bscale: f64, bzero: f64) -> f32 {
    match bitpix {
        8 => (raw[0] as f64 * bscale + bzero) as f32,
        16 => {
            let v = i16::from_be_bytes([raw[0], raw[1]]);
            (v as f64 * bscale + bzero) as f32
        }
        32 => {
            let v = i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
            (v as f64 * bscale + bzero) as f32
        }
        -32 => {
            let v = f32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
            (v as f64 * bscale + bzero) as f32
        }
        -64 => {
            let v = f64::from_be_bytes([
                raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
            ]);
            (v * bscale + bzero) as f32
        }
        _ => 0.0,
    }
}

fn extract_header_value(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('\'') {
        if let Some(end) = trimmed[1..].find('\'') {
            return trimmed[1..1 + end].trim_end().to_string();
        }
    }
    match trimmed.find('/') {
        Some(pos) => trimmed[..pos].trim().to_string(),
        None => trimmed.to_string(),
    }
}


pub struct ParsedHdu {
    pub header: HduHeader,
    pub header_start: usize,
    pub data_start: usize,
    pub next_hdu_offset: usize,
}

pub fn parse_header_at(mmap: &[u8], offset: usize) -> Result<ParsedHdu> {
    let mut cards = Vec::new();
    let mut index = HashMap::new();
    let mut pos = offset;
    let mut end_found = false;

    while !end_found {
        if pos + BLOCK_SIZE > mmap.len() {
            bail!("Unexpected end of file while reading header at offset {}", offset);
        }

        let block = &mmap[pos..pos + BLOCK_SIZE];
        pos += BLOCK_SIZE;

        for card_bytes in block.chunks_exact(80) {
            let keyword_bytes = &card_bytes[0..8];
            let keyword = String::from_utf8_lossy(keyword_bytes).trim().to_string();

            if keyword == "END" {
                end_found = true;
                break;
            }

            if card_bytes.len() < 10 || &card_bytes[8..10] != b"= " {
                continue;
            }

            let value_raw_bytes = &card_bytes[10..];
            let value_str = String::from_utf8_lossy(value_raw_bytes);

            let value = extract_header_value(&value_str);

            cards.push((keyword.clone(), value.clone()));
            index.insert(keyword, value);
        }
    }

    let header = HduHeader { cards, index };
    let data_start = pos;
    let data_bytes_padded = header.padded_data_bytes();
    let next_hdu = data_start + data_bytes_padded;

    Ok(ParsedHdu {
        header,
        header_start: offset,
        data_start,
        next_hdu_offset: next_hdu,
    })
}

pub struct MmapImageResult {
    pub header: HduHeader,
    pub image: Array2<f32>,
}

pub struct MmapCubeResult {
    pub header: HduHeader,
    pub cube: Array3<f32>,
}

pub fn extract_image_mmap(file: &File) -> Result<MmapImageResult> {
    let mmap = create_mmap(file)?;
    let mut offset: usize = 0;

    while offset < mmap.len() {
        let parsed = parse_header_at(&mmap, offset)?;
        let header = &parsed.header;

        let naxis = header.get_i64("NAXIS").unwrap_or(0);
        let naxis1 = header.get_i64("NAXIS1").unwrap_or(0);
        let naxis2 = header.get_i64("NAXIS2").unwrap_or(0);

        if naxis >= 2 && naxis1 > 1 && naxis2 > 1 {
            let data_offset = parsed.data_start;
            let bitpix = header
                .get_i64("BITPIX")
                .context("Missing BITPIX in image HDU")?;
            let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
            let slice_bytes = naxis1 as usize * naxis2 as usize * bytes_per_pixel;

            let data_end = data_offset + slice_bytes;
            if data_end > mmap.len() {
                bail!("Image data exceeds file size");
            }

            let raw = &mmap[data_offset..data_end];
            let (bzero, bscale) = scaling(header);
            let pixels = decode_pixels(raw, bitpix, bscale, bzero);
            let image = Array2::from_shape_vec((naxis2 as usize, naxis1 as usize), pixels)
                .context("Failed to reshape image pixels")?;

            return Ok(MmapImageResult {
                header: parsed.header,
                image,
            });
        }

        offset = parsed.next_hdu_offset;
    }

    bail!("No 2D image block found")
}

pub fn extract_cube_mmap(file: &File) -> Result<MmapCubeResult> {
    let mmap = create_mmap(file)?;
    let mut offset: usize = 0;

    while offset < mmap.len() {
        let parsed = parse_header_at(&mmap, offset)?;
        let header = &parsed.header;

        let naxis = header.get_i64("NAXIS").unwrap_or(0);
        let naxis3 = header.get_i64("NAXIS3").unwrap_or(0);

        if naxis == 3 && naxis3 > 1 {
            let naxis1 = header.get_i64("NAXIS1").unwrap_or(0) as usize;
            let naxis2 = header.get_i64("NAXIS2").unwrap_or(0) as usize;
            let naxis3 = naxis3 as usize;

            let data_offset = parsed.data_start;
            let bitpix = header
                .get_i64("BITPIX")
                .context("Missing BITPIX in cube HDU")?;
            let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
            let total_bytes = naxis1 * naxis2 * naxis3 * bytes_per_pixel;

            let data_end = data_offset + total_bytes;
            if data_end > mmap.len() {
                bail!("Cube data exceeds file size");
            }

            let raw = &mmap[data_offset..data_end];
            let (bzero, bscale) = scaling(header);
            let pixels = decode_pixels(raw, bitpix, bscale, bzero);
            let cube = Array3::from_shape_vec((naxis3, naxis2, naxis1), pixels)
                .context("Failed to reshape cube pixels")?;

            return Ok(MmapCubeResult {
                header: parsed.header,
                cube,
            });
        }

        offset = parsed.next_hdu_offset;
    }

    bail!("No 3D data block found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_pixels_i16() {
        let data: &[u8] = &[0x01, 0x00, 0xFF, 0xFF];
        let pixels = decode_pixels(data, 16, 1.0, 0.0);
        assert_eq!(pixels.len(), 2);
        assert!((pixels[0] - 256.0).abs() < 1e-6);
        assert!((pixels[1] - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_decode_pixels_f32() {
        let data: &[u8] = &[0x3F, 0x80, 0x00, 0x00];
        let pixels = decode_pixels(data, -32, 1.0, 0.0);
        assert_eq!(pixels.len(), 1);
        assert!((pixels[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_decode_pixels_with_scaling() {
        let data: &[u8] = &[100];
        let pixels = decode_pixels(data, 8, 2.0, 10.0);
        assert!((pixels[0] - 210.0).abs() < 1e-6);
    }

    #[test]
    fn test_decode_single_pixel_f32() {
        let bytes = 1.0f32.to_be_bytes();
        let val = decode_single_pixel(&bytes, -32, 1.0, 0.0);
        assert!((val - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_decode_single_pixel_i16() {
        let bytes = 256i16.to_be_bytes();
        let val = decode_single_pixel(&bytes, 16, 1.0, 0.0);
        assert!((val - 256.0).abs() < 1e-6);
    }
}
