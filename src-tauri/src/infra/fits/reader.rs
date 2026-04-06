use std::collections::HashMap;
use std::fs::File;

use anyhow::{bail, Context, Result};
use memmap2::{Mmap, MmapOptions};
use ndarray::{Array2, Array3};
use rayon::prelude::*;

use crate::types::HduHeader;
use crate::types::constants::BLOCK_SIZE;

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
            .par_iter()
            .map(|&b| (b as f64 * bscale + bzero) as f32)
            .collect(),
        16 => data
            .par_chunks_exact(2)
            .map(|c| {
                let v = i16::from_be_bytes([c[0], c[1]]);
                (v as f64 * bscale + bzero) as f32
            })
            .collect(),
        32 => data
            .par_chunks_exact(4)
            .map(|c| {
                let v = i32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                (v as f64 * bscale + bzero) as f32
            })
            .collect(),
        -32 => data
            .par_chunks_exact(4)
            .map(|c| {
                let v = f32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                (v as f64 * bscale + bzero) as f32
            })
            .collect(),
        -64 => data
            .par_chunks_exact(8)
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct HduInfo {
    pub index: usize,
    pub extname: Option<String>,
    pub extver: Option<i64>,
    pub naxis: i64,
    pub naxis1: i64,
    pub naxis2: i64,
    pub naxis3: i64,
    pub bitpix: i64,
    pub has_data: bool,
    #[serde(skip)]
    pub header_start: usize,
    #[serde(skip)]
    pub data_start: usize,
}

struct ScannedHdu {
    info: HduInfo,
    header: HduHeader,
}

fn scan_all_hdus(mmap: &[u8]) -> Result<Vec<ScannedHdu>> {
    let mut hdus = Vec::new();
    let mut offset: usize = 0;
    let mut idx: usize = 0;

    while offset < mmap.len() {
        if offset + BLOCK_SIZE > mmap.len() {
            if hdus.is_empty() {
                bail!("FITS file too small to contain a valid header");
            }
            break;
        }

        let parsed = match parse_header_at(mmap, offset) {
            Ok(p) => p,
            Err(_) if !hdus.is_empty() => break,
            Err(e) => return Err(e),
        };
        let h = &parsed.header;

        let naxis = h.get_i64("NAXIS").unwrap_or(0);
        let naxis1 = h.get_i64("NAXIS1").unwrap_or(0);
        let naxis2 = h.get_i64("NAXIS2").unwrap_or(0);
        let naxis3 = h.get_i64("NAXIS3").unwrap_or(0);
        let bitpix = h.get_i64("BITPIX").unwrap_or(0);
        let extname = h.get("EXTNAME").map(|s| s.to_string());
        let extver = h.get_i64("EXTVER");

        let has_data = naxis >= 2 && naxis1 > 1 && naxis2 > 1;

        hdus.push(ScannedHdu {
            info: HduInfo {
                index: idx,
                extname,
                extver,
                naxis,
                naxis1,
                naxis2,
                naxis3,
                bitpix,
                has_data,
                header_start: parsed.header_start,
                data_start: parsed.data_start,
            },
            header: parsed.header,
        });

        offset = parsed.next_hdu_offset;
        idx += 1;
    }

    Ok(hdus)
}

fn select_best_image_hdu(hdus: &[ScannedHdu]) -> Option<usize> {
    if hdus.len() == 1 && hdus[0].info.has_data {
        return Some(0);
    }

    for (i, hdu) in hdus.iter().enumerate() {
        if let Some(ref name) = hdu.info.extname {
            if name.eq_ignore_ascii_case("SCI") && hdu.info.has_data {
                return Some(i);
            }
        }
    }

    for (i, hdu) in hdus.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if hdu.info.has_data {
            return Some(i);
        }
    }

    if hdus.first().map(|h| h.info.has_data).unwrap_or(false) {
        return Some(0);
    }

    None
}

fn build_merged_header(hdus: &[ScannedHdu], selected_idx: usize) -> HduHeader {
    if selected_idx == 0 || hdus.len() == 1 {
        return hdus[selected_idx].header.clone();
    }

    let primary = &hdus[0].header;
    let extension = &hdus[selected_idx].header;
    primary.merge_with(extension)
}

fn extract_image_from_hdu(
    mmap: &[u8],
    hdu: &ScannedHdu,
) -> Result<Array2<f32>> {
    let h = &hdu.header;
    let naxis1 = h.get_i64("NAXIS1").unwrap_or(0) as usize;
    let naxis2 = h.get_i64("NAXIS2").unwrap_or(0) as usize;
    let bitpix = h.get_i64("BITPIX").context("Missing BITPIX")?;
    let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
    let slice_bytes = naxis1 * naxis2 * bytes_per_pixel;

    let data_end = hdu.info.data_start + slice_bytes;
    if data_end > mmap.len() {
        bail!("Image data exceeds file size");
    }

    let raw = &mmap[hdu.info.data_start..data_end];
    let (bzero, bscale) = scaling(h);
    let pixels = decode_pixels(raw, bitpix, bscale, bzero);
    let image = Array2::from_shape_vec((naxis2, naxis1), pixels)
        .context("Failed to reshape image pixels")?;

    Ok(image)
}

pub struct MmapImageResult {
    pub header: HduHeader,
    pub image: Array2<f32>,
    pub is_mef: bool,
    pub selected_extension: Option<String>,
    pub extension_count: usize,
    pub extensions: Vec<HduInfo>,
}

pub struct MmapCubeResult {
    pub header: HduHeader,
    pub cube: Array3<f32>,
}

pub struct MmapRgbResult {
    pub header: HduHeader,
    pub r: Array2<f32>,
    pub g: Array2<f32>,
    pub b: Array2<f32>,
    pub is_mef: bool,
    pub selected_extension: Option<String>,
    pub extension_count: usize,
    pub extensions: Vec<HduInfo>,
}

pub fn extract_image_mmap(file: &File) -> Result<MmapImageResult> {
    let mmap = create_mmap(file)?;
    let hdus = scan_all_hdus(&mmap)?;

    if hdus.is_empty() {
        bail!("No HDUs found in FITS file");
    }

    let is_mef = hdus.len() > 1;

    let selected_idx = select_best_image_hdu(&hdus)
        .context("No 2D image block found in any HDU")?;

    let image = extract_image_from_hdu(&mmap, &hdus[selected_idx])?;
    let header = build_merged_header(&hdus, selected_idx);

    let selected_extension = if selected_idx > 0 {
        hdus[selected_idx].info.extname.clone()
            .or_else(|| Some(format!("HDU {}", selected_idx)))
    } else {
        None
    };

    let extensions: Vec<HduInfo> = hdus.iter().map(|h| h.info.clone()).collect();
    let extension_count = hdus.len();

    Ok(MmapImageResult {
        header,
        image,
        is_mef,
        selected_extension,
        extension_count,
        extensions,
    })
}

pub fn extract_image_mmap_by_index(file: &File, hdu_index: usize) -> Result<MmapImageResult> {
    let mmap = create_mmap(file)?;
    let hdus = scan_all_hdus(&mmap)?;

    if hdu_index >= hdus.len() {
        bail!("HDU index {} out of range (file has {} HDUs)", hdu_index, hdus.len());
    }

    if !hdus[hdu_index].info.has_data {
        bail!("HDU {} has no image data", hdu_index);
    }

    let image = extract_image_from_hdu(&mmap, &hdus[hdu_index])?;
    let header = build_merged_header(&hdus, hdu_index);

    let selected_extension = if hdu_index > 0 {
        hdus[hdu_index].info.extname.clone()
            .or_else(|| Some(format!("HDU {}", hdu_index)))
    } else {
        None
    };

    let extensions: Vec<HduInfo> = hdus.iter().map(|h| h.info.clone()).collect();
    let is_mef = hdus.len() > 1;
    let extension_count = hdus.len();

    Ok(MmapImageResult {
        header,
        image,
        is_mef,
        selected_extension,
        extension_count,
        extensions,
    })
}

pub fn try_extract_rgb_mmap(file: &File) -> Result<Option<MmapRgbResult>> {
    let mmap = create_mmap(file)?;
    let hdus = scan_all_hdus(&mmap)?;

    if hdus.is_empty() {
        bail!("No HDUs found in FITS file");
    }

    let selected_idx = match select_best_image_hdu(&hdus) {
        Some(i) => i,
        None => return Ok(None),
    };

    let hdu = &hdus[selected_idx];
    let h = &hdu.header;
    let naxis = h.get_i64("NAXIS").unwrap_or(0);
    let naxis3 = h.get_i64("NAXIS3").unwrap_or(0);

    if naxis != 3 || naxis3 < 3 || naxis3 > 4 {
        return Ok(None);
    }

    let naxis1 = h.get_i64("NAXIS1").unwrap_or(0) as usize;
    let naxis2 = h.get_i64("NAXIS2").unwrap_or(0) as usize;
    let bitpix = h.get_i64("BITPIX").context("Missing BITPIX in RGB HDU")?;
    let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
    let plane_size = naxis1 * naxis2 * bytes_per_pixel;
    let total_size = plane_size * naxis3 as usize;
    let (bzero, bscale) = scaling(h);

    let data_end = hdu.info.data_start + total_size;
    if data_end > mmap.len() {
        bail!("RGB data exceeds file size");
    }

    let base = hdu.info.data_start;
    let r_pixels = decode_pixels(&mmap[base..base + plane_size], bitpix, bscale, bzero);
    let g_pixels = decode_pixels(&mmap[base + plane_size..base + 2 * plane_size], bitpix, bscale, bzero);
    let b_pixels = decode_pixels(&mmap[base + 2 * plane_size..base + 3 * plane_size], bitpix, bscale, bzero);

    let r = Array2::from_shape_vec((naxis2, naxis1), r_pixels)
        .context("Failed to reshape R channel")?;
    let g = Array2::from_shape_vec((naxis2, naxis1), g_pixels)
        .context("Failed to reshape G channel")?;
    let b = Array2::from_shape_vec((naxis2, naxis1), b_pixels)
        .context("Failed to reshape B channel")?;

    let header = build_merged_header(&hdus, selected_idx);
    let is_mef = hdus.len() > 1;

    let selected_extension = if selected_idx > 0 {
        hdus[selected_idx].info.extname.clone()
            .or_else(|| Some(format!("HDU {}", selected_idx)))
    } else {
        None
    };

    let extensions: Vec<HduInfo> = hdus.iter().map(|h| h.info.clone()).collect();
    let extension_count = hdus.len();

    Ok(Some(MmapRgbResult {
        header,
        r,
        g,
        b,
        is_mef,
        selected_extension,
        extension_count,
        extensions,
    }))
}

pub fn list_extensions(file: &File) -> Result<Vec<HduInfo>> {
    let mmap = create_mmap(file)?;
    let hdus = scan_all_hdus(&mmap)?;
    Ok(hdus.into_iter().map(|h| h.info).collect())
}

pub fn extract_cube_mmap(file: &File) -> Result<MmapCubeResult> {
    let mmap = create_mmap(file)?;
    let mut offset: usize = 0;

    while offset + BLOCK_SIZE <= mmap.len() {
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

pub fn load_fits_image(path: &str) -> Result<Array2<f32>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open {}", path))?;
    let result = extract_image_mmap(&file)
        .with_context(|| format!("Failed to load {}", path))?;
    Ok(result.image)
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

    #[test]
    fn test_hdu_info_serializable() {
        let info = HduInfo {
            index: 0,
            extname: Some("SCI".to_string()),
            extver: Some(1),
            naxis: 2,
            naxis1: 100,
            naxis2: 100,
            naxis3: 0,
            bitpix: -32,
            has_data: true,
            header_start: 0,
            data_start: 2880,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["extname"], "SCI");
        assert!(json.get("header_start").is_none());
    }
}
