use std::collections::HashMap;
use std::fs::File;

use anyhow::{bail, Context, Result};
use memmap2::{Mmap, MmapOptions};
use ndarray::{Array2, Array3};
use rayon::prelude::*;

use crate::types::HduHeader;
use crate::types::constants::BLOCK_SIZE;

use super::compress;
use super::file_bytes::read_file_bytes;

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

#[inline]
fn blank_value(header: &HduHeader) -> Option<i64> {
    header.get_i64("BLANK")
}

#[inline]
fn is_identity_scaling(bscale: f64, bzero: f64) -> bool {
    (bscale - 1.0).abs() < 1e-15 && bzero.abs() < 1e-15
}

pub fn decode_pixels(data: &[u8], bitpix: i64, bscale: f64, bzero: f64) -> Vec<f32> {
    decode_pixels_blank(data, bitpix, bscale, bzero, None)
}

pub fn decode_pixels_blank(
    data: &[u8],
    bitpix: i64,
    bscale: f64,
    bzero: f64,
    blank: Option<i64>,
) -> Vec<f32> {
    let identity = is_identity_scaling(bscale, bzero);

    match bitpix {
        8 => {
            data.par_iter()
                .map(|&b| {
                    if blank == Some(b as i64) {
                        f32::NAN
                    } else if identity {
                        b as f32
                    } else {
                        (b as f64 * bscale + bzero) as f32
                    }
                })
                .collect()
        }
        16 => data
            .par_chunks_exact(2)
            .map(|c| {
                let v = i16::from_be_bytes([c[0], c[1]]);
                if blank == Some(v as i64) {
                    f32::NAN
                } else if identity {
                    v as f32
                } else {
                    (v as f64 * bscale + bzero) as f32
                }
            })
            .collect(),
        32 => data
            .par_chunks_exact(4)
            .map(|c| {
                let v = i32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                if blank == Some(v as i64) {
                    f32::NAN
                } else if identity {
                    v as f32
                } else {
                    (v as f64 * bscale + bzero) as f32
                }
            })
            .collect(),
        64 => data
            .par_chunks_exact(8)
            .map(|c| {
                let v = i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
                if blank == Some(v) {
                    f32::NAN
                } else if identity {
                    v as f32
                } else {
                    (v as f64 * bscale + bzero) as f32
                }
            })
            .collect(),
        -32 => {
            if identity {
                data.par_chunks_exact(4)
                    .map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]]))
                    .collect()
            } else {
                data.par_chunks_exact(4)
                    .map(|c| {
                        let v = f32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                        (v as f64 * bscale + bzero) as f32
                    })
                    .collect()
            }
        }
        -64 => {
            if identity {
                data.par_chunks_exact(8)
                    .map(|c| {
                        f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32
                    })
                    .collect()
            } else {
                data.par_chunks_exact(8)
                    .map(|c| {
                        let v = f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
                        (v * bscale + bzero) as f32
                    })
                    .collect()
            }
        }
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
        64 => {
            let v = i64::from_be_bytes([
                raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
            ]);
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
    if let Some(rest) = trimmed.strip_prefix('\'') {
        let mut out = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    out.push('\'');
                    chars.next();
                } else {
                    break;
                }
            } else {
                out.push(c);
            }
        }
        return out.trim_end().to_string();
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
    is_compressed: bool,
}

fn build_scanned_hdu(parsed: ParsedHdu, idx: usize) -> ScannedHdu {
    let h = &parsed.header;

    let extname = h.get("EXTNAME").map(|s| s.to_string());
    let extver = h.get_i64("EXTVER");
    let is_image_hdu = h
        .get("XTENSION")
        .is_none_or(|x| x.trim().eq_ignore_ascii_case("IMAGE"));
    let is_compressed = compress::is_compressed_image_hdu(h);

    // For a compressed-image BINTABLE, the Z-prefixed keywords (ZNAXIS,
    // ZNAXISn, ZBITPIX) describe the *decompressed* image shape; NAXIS/
    // NAXISn/BITPIX describe the BINTABLE storage itself (row bytes, row
    // count, BITPIX=8) and are irrelevant to callers picking an image HDU.
    // v1 scope: only 2D (ZNAXIS=2) compressed images are selectable --
    // 3D compressed cubes fall through as has_data=false rather than
    // being mis-selected into the plain-image/cube/RGB byte-slicing paths.
    let (naxis, naxis1, naxis2, naxis3, bitpix, has_data) = if is_compressed {
        let shape = compress::read_compressed_shape(h);
        let has_data = (shape.znaxis == 2 || (shape.znaxis == 3 && shape.znaxis3 >= 1))
            && shape.znaxis1 > 1
            && shape.znaxis2 > 1;
        let naxis3 = if shape.znaxis == 3 { shape.znaxis3 } else { 0 };
        (shape.znaxis, shape.znaxis1, shape.znaxis2, naxis3, shape.zbitpix, has_data)
    } else {
        let naxis = h.get_i64("NAXIS").unwrap_or(0);
        let naxis1 = h.get_i64("NAXIS1").unwrap_or(0);
        let naxis2 = h.get_i64("NAXIS2").unwrap_or(0);
        let naxis3 = h.get_i64("NAXIS3").unwrap_or(0);
        let bitpix = h.get_i64("BITPIX").unwrap_or(0);
        let has_data = is_image_hdu && naxis >= 2 && naxis1 > 1 && naxis2 > 1;
        (naxis, naxis1, naxis2, naxis3, bitpix, has_data)
    };

    ScannedHdu {
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
        is_compressed,
    }
}

fn scan_all_hdus(mmap: &[u8]) -> Result<Vec<ScannedHdu>> {
    if mmap.is_empty() {
        bail!("File is empty (0 bytes) — the download or copy may have failed");
    }
    let mut hdus = Vec::new();
    let mut offset: usize = 0;

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

        offset = parsed.next_hdu_offset;
        let idx = hdus.len();
        hdus.push(build_scanned_hdu(parsed, idx));
    }

    Ok(hdus)
}

pub(crate) fn read_header_blocks(file: &File, offset: usize) -> Result<ParsedHdu> {
    use std::io::{Read, Seek, SeekFrom};

    let mut f = file;
    f.seek(SeekFrom::Start(offset as u64))
        .with_context(|| format!("seek to header at offset {offset} failed"))?;

    let mut buf: Vec<u8> = Vec::with_capacity(BLOCK_SIZE);
    'blocks: loop {
        let start = buf.len();
        buf.resize(start + BLOCK_SIZE, 0);
        f.read_exact(&mut buf[start..]).with_context(|| {
            format!("Unexpected end of file while reading header at offset {offset}")
        })?;
        for card in buf[start..].chunks_exact(80) {
            if String::from_utf8_lossy(&card[0..8]).trim() == "END" {
                break 'blocks;
            }
        }
    }

    let parsed = parse_header_at(&buf, 0)?;
    Ok(ParsedHdu {
        header: parsed.header,
        header_start: offset,
        data_start: offset + parsed.data_start,
        next_hdu_offset: offset + parsed.next_hdu_offset,
    })
}

fn scan_hdu_headers(file: &File) -> Result<Vec<ScannedHdu>> {
    let file_len = file.metadata().context("stat failed")?.len() as usize;
    if file_len == 0 {
        bail!("File is empty (0 bytes) — the download or copy may have failed");
    }
    let mut hdus = Vec::new();
    let mut offset: usize = 0;

    while offset < file_len {
        if offset + BLOCK_SIZE > file_len {
            if hdus.is_empty() {
                bail!("FITS file too small to contain a valid header");
            }
            break;
        }

        let parsed = match read_header_blocks(file, offset) {
            Ok(p) => p,
            Err(_) if !hdus.is_empty() => break,
            Err(e) => return Err(e),
        };

        offset = parsed.next_hdu_offset;
        let idx = hdus.len();
        hdus.push(build_scanned_hdu(parsed, idx));
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
    if hdu.is_compressed {
        let mut planes = compress::decode_compressed_planes(mmap, &hdu.header, hdu.info.data_start)?;
        if planes.is_empty() {
            bail!("Compressed image HDU decoded to zero planes");
        }
        return Ok(planes.swap_remove(0));
    }

    let h = &hdu.header;
    let naxis1_i = h.get_i64("NAXIS1").unwrap_or(0);
    let naxis2_i = h.get_i64("NAXIS2").unwrap_or(0);
    if naxis1_i <= 0 || naxis2_i <= 0 {
        bail!("Invalid image dimensions NAXIS1={}, NAXIS2={}", naxis1_i, naxis2_i);
    }
    let naxis1 = naxis1_i as usize;
    let naxis2 = naxis2_i as usize;
    let bitpix = h.get_i64("BITPIX").context("Missing BITPIX")?;
    let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
    if bytes_per_pixel == 0 {
        bail!("Unsupported BITPIX={}", bitpix);
    }
    let slice_bytes = naxis1
        .checked_mul(naxis2)
        .and_then(|v| v.checked_mul(bytes_per_pixel))
        .context("Image size overflow")?;

    let data_end = hdu
        .info
        .data_start
        .checked_add(slice_bytes)
        .context("Image data end overflow")?;
    if data_end > mmap.len() {
        bail!("Image data exceeds file size");
    }

    let raw = &mmap[hdu.info.data_start..data_end];
    let (bzero, bscale) = scaling(h);
    let pixels = decode_pixels_blank(raw, bitpix, bscale, bzero, blank_value(h));
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
    let mmap = read_file_bytes(file)?;
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

/// Same HDU selection as `extract_image_mmap`, but skips `extract_image_from_hdu`
/// (pixel decode/decompress) entirely -- for callers that only need the merged
/// header (e.g. a WCS lookup driven by mouse movement, where re-decoding the
/// whole image on every call would be far too slow).
pub fn extract_header_mmap(file: &File) -> Result<HduHeader> {
    let hdus = scan_hdu_headers(file)?;

    if hdus.is_empty() {
        bail!("No HDUs found in FITS file");
    }

    let selected_idx = select_best_image_hdu(&hdus)
        .context("No 2D image block found in any HDU")?;

    Ok(build_merged_header(&hdus, selected_idx))
}

pub fn extract_image_mmap_by_index(file: &File, hdu_index: usize) -> Result<MmapImageResult> {
    let mmap = read_file_bytes(file)?;
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
    let mmap = read_file_bytes(file)?;
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
    let naxis = hdu.info.naxis;
    let naxis3 = hdu.info.naxis3;

    if naxis != 3 || naxis3 < 3 || naxis3 > 4 {
        return Ok(None);
    }

    let (r, g, b) = if hdu.is_compressed {
        let mut planes = compress::decode_compressed_planes(&mmap, h, hdu.info.data_start)?;
        if planes.len() < 3 {
            bail!("Compressed RGB HDU decoded {} planes, expected at least 3", planes.len());
        }
        let b = planes.swap_remove(2);
        let g = planes.swap_remove(1);
        let r = planes.swap_remove(0);
        (r, g, b)
    } else {
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
        let blank = blank_value(h);
        let r_pixels = decode_pixels_blank(&mmap[base..base + plane_size], bitpix, bscale, bzero, blank);
        let g_pixels = decode_pixels_blank(&mmap[base + plane_size..base + 2 * plane_size], bitpix, bscale, bzero, blank);
        let b_pixels = decode_pixels_blank(&mmap[base + 2 * plane_size..base + 3 * plane_size], bitpix, bscale, bzero, blank);

        let r = Array2::from_shape_vec((naxis2, naxis1), r_pixels)
            .context("Failed to reshape R channel")?;
        let g = Array2::from_shape_vec((naxis2, naxis1), g_pixels)
            .context("Failed to reshape G channel")?;
        let b = Array2::from_shape_vec((naxis2, naxis1), b_pixels)
            .context("Failed to reshape B channel")?;
        (r, g, b)
    };

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
    let hdus = scan_hdu_headers(file)?;
    Ok(hdus.into_iter().map(|h| h.info).collect())
}

pub fn extract_header_by_index(file: &File, hdu_index: usize) -> Result<HduHeader> {
    let mut hdus = scan_hdu_headers(file)?;
    if hdu_index >= hdus.len() {
        bail!("HDU index {} out of range (file has {} HDUs)", hdu_index, hdus.len());
    }
    Ok(hdus.swap_remove(hdu_index).header)
}

pub fn extract_cube_mmap(file: &File) -> Result<MmapCubeResult> {
    let mmap = read_file_bytes(file)?;
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
            let pixels = decode_pixels_blank(raw, bitpix, bscale, bzero, blank_value(header));
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

pub fn read_primary_header(path: &str) -> Result<HduHeader> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open {}", path))?;
    Ok(read_header_blocks(&file, 0)?.header)
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
    fn test_decode_pixels_identity_f32_fast_path() {
        let val = std::f32::consts::PI;
        let data = val.to_be_bytes();
        let pixels = decode_pixels(&data, -32, 1.0, 0.0);
        assert_eq!(pixels[0], val);
    }

    #[test]
    fn test_is_identity_scaling() {
        assert!(is_identity_scaling(1.0, 0.0));
        assert!(!is_identity_scaling(2.0, 0.0));
        assert!(!is_identity_scaling(1.0, 32768.0));
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

    fn compressed_fixtures_dir() -> Option<std::path::PathBuf> {
        if let Ok(d) = std::env::var("FITS_COMPRESSED_FIXTURES") {
            return Some(std::path::PathBuf::from(d));
        }
        let bundled = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("fits_compressed");
        bundled.is_dir().then_some(bundled)
    }

    #[test]
    fn compressed_image_reference_suite() {
        let dir = match compressed_fixtures_dir() {
            Some(d) => d,
            None => return,
        };

        // (filename, expected sum of finite pixels, absolute tolerance).
        // Expected values computed once via astropy's own decompression
        // (`uv run --with astropy`); RICE_1/GZIP_1/GZIP_2 are lossless for
        // integer data so those match to a tight tolerance, while the
        // quantized-float case carries float32-rounding-level slack.
        let cases: &[(&str, f64, f64)] = &[
            ("rice_i16_default_tile.fits", 261136.0, 1e-6),
            ("rice_i32_default_tile.fits", 111456500.0, 1e-3),
            ("rice_u8_default_tile.fits", 90752.0, 1e-6),
            // same source data as rice_i16_default_tile, but tiled 6x16
            // instead of row-tiled -- cross-checks the 2D tile-geometry math
            // (tiles_x>1 and tiles_y>1, partial edge tiles) independent of
            // BSCALE/BZERO/dtype correctness.
            ("rice_i16_2d_tiles.fits", 261136.0, 1e-6),
            ("rice_f32_quantized.fits", -650.953536, 1e-2),
            // every row here is internally uniform, so the quantizer falls
            // back to GZIP_COMPRESSED_DATA (raw physical float bytes) for
            // every tile instead of quantized RICE_1 -- exercises that
            // fallback path and its "use global BSCALE/BZERO, not per-tile
            // ZSCALE/ZZERO" scaling rule.
            ("rice_f32_uniform_rows_gzip_fallback.fits", -58506.25, 1e-2),
            ("gzip1_i16.fits", 261136.0, 1e-6),
            ("gzip2_i32.fits", 111456500.0, 1e-3),
            ("gzip2_i16_2d_tiles.fits", 261136.0, 1e-6),
        ];

        let mut failures = Vec::new();
        for (name, expected, tol) in cases {
            let path = dir.join(name);
            if !path.exists() {
                eprintln!("SKIP {name} (not bundled)");
                continue;
            }
            let path_str = path.to_string_lossy().to_string();
            match load_fits_image(&path_str) {
                Ok(image) => {
                    let sum: f64 = image.iter().filter(|v| v.is_finite()).map(|&v| v as f64).sum();
                    if (sum - expected).abs() > *tol {
                        eprintln!("FAIL {name} (sum {sum} expected {expected})");
                        failures.push(*name);
                    } else {
                        eprintln!("PASS {name} (sum {sum:.4})");
                    }
                }
                Err(e) => {
                    eprintln!("FAIL {name} (load error: {e})");
                    failures.push(*name);
                }
            }
        }

        assert!(failures.is_empty(), "compressed-image reference suite failures: {:?}", failures);
    }

    #[test]
    fn compressed_image_hdu_info_reports_decompressed_shape() {
        let dir = match compressed_fixtures_dir() {
            Some(d) => d,
            None => return,
        };
        let path = dir.join("rice_i16_default_tile.fits");
        if !path.exists() {
            return;
        }
        let file = File::open(&path).unwrap();
        let exts = list_extensions(&file).unwrap();
        // HDU 1 is the compressed-image BINTABLE; HduInfo should surface the
        // *decompressed* image shape (ZNAXISn/ZBITPIX), not the BINTABLE's
        // own row-bytes/row-count/BITPIX=8 storage shape.
        let compressed = &exts[1];
        assert!(compressed.has_data);
        assert_eq!(compressed.naxis1, 37);
        assert_eq!(compressed.naxis2, 23);
        assert_eq!(compressed.bitpix, 16);
    }

    #[test]
    fn seek_scanner_matches_full_scan_on_every_fixture() {
        let dir = match compressed_fixtures_dir() {
            Some(d) => d,
            None => return,
        };
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "fits") {
                continue;
            }
            let file = File::open(&path).unwrap();
            let bytes = read_file_bytes(&file).unwrap();
            let full = scan_all_hdus(&bytes).unwrap();
            let seek = scan_hdu_headers(&file).unwrap();

            assert_eq!(full.len(), seek.len(), "{path:?}: HDU count");
            for (f, s) in full.iter().zip(seek.iter()) {
                assert_eq!(f.info.index, s.info.index, "{path:?}");
                assert_eq!(f.info.extname, s.info.extname, "{path:?}");
                assert_eq!(f.info.naxis, s.info.naxis, "{path:?}");
                assert_eq!(f.info.naxis1, s.info.naxis1, "{path:?}");
                assert_eq!(f.info.naxis2, s.info.naxis2, "{path:?}");
                assert_eq!(f.info.naxis3, s.info.naxis3, "{path:?}");
                assert_eq!(f.info.bitpix, s.info.bitpix, "{path:?}");
                assert_eq!(f.info.has_data, s.info.has_data, "{path:?}");
                assert_eq!(f.info.header_start, s.info.header_start, "{path:?}");
                assert_eq!(f.info.data_start, s.info.data_start, "{path:?}");
                assert_eq!(f.is_compressed, s.is_compressed, "{path:?}");
                assert_eq!(f.header.cards, s.header.cards, "{path:?}");
            }
            checked += 1;
        }
        assert!(checked > 0, "no fixtures found to check");
    }
}
