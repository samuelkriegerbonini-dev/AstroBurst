use std::collections::HashMap;
use std::fs::File;

use anyhow::{bail, Context, Result};
use memmap2::{Mmap, MmapOptions};
use ndarray::{Array2, Array3};
use rayon::prelude::*;

use crate::types::constants::BLOCK_SIZE;
use crate::types::image::IntPlane;
use crate::types::{HduHeader, ImageRef, PlaneSelector};

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

pub enum IntPixels {
    U32(Vec<u32>),
    I32(Vec<i32>),
}

pub fn decode_pixels_int(data: &[u8], bitpix: i64, bscale: f64, bzero: f64) -> Result<IntPixels> {
    if bscale != 1.0 || !bzero.is_finite() || bzero.fract() != 0.0 {
        bail!("scaled integer data (BSCALE={}, BZERO={})", bscale, bzero);
    }
    match bitpix {
        8 => {
            if bzero < 0.0 || bzero > u32::MAX as f64 {
                bail!("BZERO {} out of range for BITPIX 8", bzero);
            }
            let off = bzero as u32;
            let out: Result<Vec<u32>> = data
                .par_iter()
                .map(|&b| (b as u32).checked_add(off).context("BITPIX 8 value overflow"))
                .collect();
            Ok(IntPixels::U32(out?))
        }
        16 => {
            if bzero == 32768.0 {
                Ok(IntPixels::U32(
                    data.par_chunks_exact(2)
                        .map(|c| (i16::from_be_bytes([c[0], c[1]]) as i64 + 32768) as u32)
                        .collect(),
                ))
            } else {
                if bzero < i32::MIN as f64 || bzero > i32::MAX as f64 {
                    bail!("BZERO {} out of range for BITPIX 16", bzero);
                }
                let off = bzero as i32;
                let out: Result<Vec<i32>> = data
                    .par_chunks_exact(2)
                    .map(|c| {
                        (i16::from_be_bytes([c[0], c[1]]) as i32)
                            .checked_add(off)
                            .context("BITPIX 16 value overflow")
                    })
                    .collect();
                Ok(IntPixels::I32(out?))
            }
        }
        32 => {
            if bzero == 2147483648.0 {
                Ok(IntPixels::U32(
                    data.par_chunks_exact(4)
                        .map(|c| (i32::from_be_bytes([c[0], c[1], c[2], c[3]]) as i64 + 2147483648) as u32)
                        .collect(),
                ))
            } else {
                if bzero < i32::MIN as f64 || bzero > i32::MAX as f64 {
                    bail!("BZERO {} out of range for BITPIX 32", bzero);
                }
                let off = bzero as i32;
                let out: Result<Vec<i32>> = data
                    .par_chunks_exact(4)
                    .map(|c| {
                        i32::from_be_bytes([c[0], c[1], c[2], c[3]])
                            .checked_add(off)
                            .context("BITPIX 32 value overflow")
                    })
                    .collect();
                Ok(IntPixels::I32(out?))
            }
        }
        other => bail!("not an integer plane (BITPIX={})", other),
    }
}

pub fn int_pixels_to_plane(px: IntPixels, rows: usize, cols: usize) -> Result<IntPlane> {
    let (bits, signed) = match px {
        IntPixels::U32(v) => (v, false),
        IntPixels::I32(v) => (v.into_iter().map(|x| x as u32).collect(), true),
    };
    let bits = Array2::from_shape_vec((rows, cols), bits)
        .context("Failed to reshape integer plane")?;
    Ok(IntPlane { bits, signed })
}

pub fn extract_int_plane_by_index(file: &File, hdu_index: usize) -> Result<IntPlane> {
    let mmap = read_file_bytes(file)?;
    let hdus = scan_all_hdus(&mmap)?;
    if hdu_index >= hdus.len() {
        bail!("HDU index {} out of range (file has {} HDUs)", hdu_index, hdus.len());
    }
    let hdu = &hdus[hdu_index];
    if hdu.is_compressed {
        bail!("HDU {} is a compressed image; lossless integer read is not supported", hdu_index);
    }
    let h = &hdu.header;
    let naxis = h.get_i64("NAXIS").unwrap_or(0);
    let naxis1_i = h.get_i64("NAXIS1").unwrap_or(0);
    let naxis2_i = h.get_i64("NAXIS2").unwrap_or(0);
    if naxis < 2 || naxis1_i <= 0 || naxis2_i <= 0 {
        bail!("HDU {} is not a 2D image (NAXIS={})", hdu_index, naxis);
    }
    let planes = declared_plane_count(h, naxis);
    if planes != 1 {
        bail!(
            "HDU {} is a {}D cube of {} planes, not a 2D integer plane",
            hdu_index, naxis, planes
        );
    }
    let bitpix = h.get_i64("BITPIX").context("Missing BITPIX")?;
    if !matches!(bitpix, 8 | 16 | 32) {
        bail!("not an integer plane (BITPIX={})", bitpix);
    }
    let (naxis1, naxis2) = (naxis1_i as usize, naxis2_i as usize);
    let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
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
    let (bzero, bscale) = scaling(h);
    let px = decode_pixels_int(&mmap[hdu.info.data_start..data_end], bitpix, bscale, bzero)?;
    int_pixels_to_plane(px, naxis2, naxis1)
}

pub fn auto_hdu_index(file: &File) -> Result<usize> {
    let hdus = scan_hdu_headers(file)?;
    if hdus.is_empty() {
        bail!("No HDUs found in FITS file");
    }
    select_best_image_hdu(&hdus).with_context(|| no_image_hdu_error(&hdus))
}

pub fn extract_header_by_index_merged(file: &File, hdu_index: usize) -> Result<HduHeader> {
    let hdus = scan_hdu_headers(file)?;
    if hdu_index >= hdus.len() {
        bail!("HDU index {} out of range (file has {} HDUs)", hdu_index, hdus.len());
    }
    Ok(build_merged_header(&hdus, hdu_index))
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
    is_image_hdu: bool,
    plane_count: i64,
    is_single_plane_image: bool,
}

const MAX_FITS_AXES: i64 = 999;

fn declared_plane_count_with_prefix(header: &HduHeader, naxis: i64, prefix: &str) -> i64 {
    (3..=naxis.clamp(0, MAX_FITS_AXES))
        .map(|axis| header.get_i64(&format!("{prefix}NAXIS{axis}")).unwrap_or(1))
        .fold(1i64, |acc, len| acc.saturating_mul(len))
}

fn declared_plane_count(header: &HduHeader, naxis: i64) -> i64 {
    declared_plane_count_with_prefix(header, naxis, "")
}

fn build_scanned_hdu(parsed: ParsedHdu, idx: usize) -> ScannedHdu {
    let h = &parsed.header;

    let extname = h.get("EXTNAME").map(|s| s.to_string());
    let extver = h.get_i64("EXTVER");
    let is_image_hdu = h
        .get("XTENSION")
        .is_none_or(|x| x.trim().eq_ignore_ascii_case("IMAGE"));
    let is_compressed = compress::is_compressed_image_hdu(h);

    // A compressed-image BINTABLE describes its decompressed image in ZNAXIS/ZNAXISn/ZBITPIX; NAXIS/NAXISn/BITPIX describe the table storage.
    let (naxis, naxis1, naxis2, naxis3, bitpix, plane_count) = if is_compressed {
        let shape = compress::read_compressed_shape(h);
        let planes = declared_plane_count_with_prefix(h, shape.znaxis, "Z");
        let naxis3 = if shape.znaxis == 3 { shape.znaxis3 } else { 0 };
        (shape.znaxis, shape.znaxis1, shape.znaxis2, naxis3, shape.zbitpix, planes)
    } else {
        let naxis = h.get_i64("NAXIS").unwrap_or(0);
        let naxis1 = h.get_i64("NAXIS1").unwrap_or(0);
        let naxis2 = h.get_i64("NAXIS2").unwrap_or(0);
        let naxis3 = h.get_i64("NAXIS3").unwrap_or(0);
        let bitpix = h.get_i64("BITPIX").unwrap_or(0);
        (naxis, naxis1, naxis2, naxis3, bitpix, declared_plane_count(h, naxis))
    };

    let decodable = if is_compressed { naxis <= 3 } else { is_image_hdu };
    let has_data = decodable && naxis >= 2 && naxis1 > 1 && naxis2 > 1 && plane_count >= 1;
    let is_single_plane_image = has_data && plane_count == 1;

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
        is_image_hdu,
        plane_count,
        is_single_plane_image,
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
    if hdus.len() == 1 && hdus[0].is_single_plane_image {
        return Some(0);
    }

    for (i, hdu) in hdus.iter().enumerate() {
        if let Some(ref name) = hdu.info.extname {
            if name.eq_ignore_ascii_case("SCI") && hdu.is_single_plane_image {
                return Some(i);
            }
        }
    }

    for (i, hdu) in hdus.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if hdu.is_single_plane_image {
            return Some(i);
        }
    }

    if hdus.first().map(|h| h.is_single_plane_image).unwrap_or(false) {
        return Some(0);
    }

    None
}

fn xtension_label(header: &HduHeader) -> &str {
    header
        .get("XTENSION")
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .unwrap_or("PRIMARY")
}

fn shape_label(hdu: &ScannedHdu) -> String {
    let prefix = if hdu.is_compressed { "Z" } else { "" };
    let naxis = hdu.info.naxis;
    if naxis <= 0 {
        return format!("{prefix}NAXIS=0");
    }
    let dims: Vec<String> = (1..=naxis.clamp(1, MAX_FITS_AXES))
        .map(|axis| {
            hdu.header
                .get_i64(&format!("{prefix}NAXIS{axis}"))
                .unwrap_or(0)
                .to_string()
        })
        .collect();
    format!("{prefix}NAXIS={naxis} [{}]", dims.join("x"))
}

fn hdu_usability_reason(hdu: &ScannedHdu) -> String {
    if hdu.is_single_plane_image {
        return "loadable 2D image".to_string();
    }
    if !hdu.is_image_hdu && !hdu.is_compressed {
        return format!("{} extension, holds no image pixels", xtension_label(&hdu.header));
    }
    if hdu.info.naxis <= 0 {
        return "header only, no pixel data".to_string();
    }
    if hdu.info.naxis == 1 {
        return "1D array, not an image".to_string();
    }
    if hdu.plane_count > 1 {
        return format!(
            "{}D cube of {} planes, not a single 2D image",
            hdu.info.naxis, hdu.plane_count
        );
    }
    if hdu.plane_count <= 0 {
        return "declared axis of length zero, no pixel data".to_string();
    }
    if hdu.is_compressed && hdu.info.naxis > 3 {
        return format!(
            "compressed {}D block, only 2D and 3D compressed images are supported",
            hdu.info.naxis
        );
    }
    format!(
        "degenerate image dimensions {}x{}",
        hdu.info.naxis1, hdu.info.naxis2
    )
}

fn hdu_label(hdu: &ScannedHdu) -> String {
    let name = hdu
        .info
        .extname
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| xtension_label(&hdu.header).to_string());
    format!("HDU {} ({})", hdu.info.index, name)
}

fn most_image_like_hdu(hdus: &[ScannedHdu]) -> Option<&ScannedHdu> {
    let rank = |hdu: &ScannedHdu| {
        (
            u8::from((hdu.is_image_hdu || hdu.is_compressed) && hdu.info.naxis >= 2),
            u8::from(
                hdu.info
                    .extname
                    .as_deref()
                    .is_some_and(|n| n.trim().eq_ignore_ascii_case("SCI")),
            ),
            hdu.info.naxis1.saturating_mul(hdu.info.naxis2),
        )
    };
    hdus.iter()
        .filter(|hdu| hdu.info.naxis > 0)
        .fold(None, |best: Option<&ScannedHdu>, hdu| match best {
            Some(current) if rank(current) >= rank(hdu) => Some(current),
            _ => Some(hdu),
        })
        .or_else(|| hdus.first())
}

const MAX_INVENTORY_ENTRIES: usize = 12;

fn no_image_hdu_error(hdus: &[ScannedHdu]) -> String {
    let mut entries: Vec<String> = hdus
        .iter()
        .take(MAX_INVENTORY_ENTRIES)
        .map(|hdu| {
            format!(
                "[{}] EXTNAME={} XTENSION={} {} -- {}",
                hdu.info.index,
                hdu.info
                    .extname
                    .as_deref()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .unwrap_or("(none)"),
                xtension_label(&hdu.header),
                shape_label(hdu),
                hdu_usability_reason(hdu),
            )
        })
        .collect();
    if hdus.len() > MAX_INVENTORY_ENTRIES {
        entries.push(format!("... {} more HDUs", hdus.len() - MAX_INVENTORY_ENTRIES));
    }
    let verdict = match most_image_like_hdu(hdus) {
        Some(hdu) => format!("{}: {}; ", hdu_label(hdu), hdu_usability_reason(hdu)),
        None => String::new(),
    };
    format!(
        "{verdict}no HDU in this file holds a 2D image. HDUs: {}",
        entries.join("; ")
    )
}

const COLOUR_AXIS_NAMES: [&str; 3] = ["RGB", "COLOR", "COLOUR"];

const NON_COLOUR_AXIS_TYPES: [&str; 12] = [
    "WAVE", "AWAV", "FREQ", "VELO", "VRAD", "VOPT", "ENER", "WAVN", "ZOPT", "BETA", "STOKES",
    "TIME",
];

fn normalised_card(value: &str) -> String {
    value.trim().trim_matches('\'').trim().to_ascii_uppercase()
}

fn is_colour_marker(value: &str) -> bool {
    let normalised = normalised_card(value);
    COLOUR_AXIS_NAMES.contains(&normalised.as_str())
}

fn is_non_colour_axis_type(value: &str) -> bool {
    let normalised = normalised_card(value);
    let head = normalised.split('-').next().unwrap_or("");
    NON_COLOUR_AXIS_TYPES.contains(&head)
}

fn has_rgb_marker(hdu: &ScannedHdu) -> bool {
    hdu.header.get("CTYPE3").is_some_and(is_colour_marker)
        || hdu.info.extname.as_deref().is_some_and(is_colour_marker)
}

fn third_axis_is_not_colour(hdu: &ScannedHdu) -> bool {
    let h = &hdu.header;
    h.get("CTYPE3").is_some_and(is_non_colour_axis_type)
        || h.get("CUNIT3").is_some_and(|u| !normalised_card(u).is_empty())
        || (h.get_f64("CRVAL3").is_some() && h.get_f64("CDELT3").is_some())
}

fn select_rgb_cube_hdu(hdus: &[ScannedHdu]) -> Option<usize> {
    hdus.iter().position(|hdu| {
        (hdu.is_image_hdu || hdu.is_compressed)
            && hdu.info.naxis == 3
            && (3..=4).contains(&hdu.info.naxis3)
            && hdu.info.naxis1 > 1
            && hdu.info.naxis2 > 1
            && (has_rgb_marker(hdu) || !third_axis_is_not_colour(hdu))
    })
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
    if !hdu.is_single_plane_image {
        bail!(
            "HDU {} cannot be loaded as a 2D image: {}",
            hdu.info.index,
            hdu_usability_reason(hdu)
        );
    }
    decode_hdu_plane(mmap, hdu, 0)
}

fn decode_hdu_plane(
    mmap: &[u8],
    hdu: &ScannedHdu,
    plane_index: usize,
) -> Result<Array2<f32>> {
    if hdu.is_compressed {
        let mut planes = compress::decode_compressed_planes(mmap, &hdu.header, hdu.info.data_start)?;
        if plane_index >= planes.len() {
            bail!(
                "Compressed image HDU {} decoded to {} planes, plane {} was requested",
                hdu.info.index,
                planes.len(),
                plane_index
            );
        }
        return Ok(planes.swap_remove(plane_index));
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

    let plane_offset = slice_bytes
        .checked_mul(plane_index)
        .context("Plane offset overflow")?;
    let data_begin = hdu
        .info
        .data_start
        .checked_add(plane_offset)
        .context("Plane start overflow")?;
    let data_end = data_begin
        .checked_add(slice_bytes)
        .context("Image data end overflow")?;
    if data_end > mmap.len() {
        bail!("Image data exceeds file size");
    }

    let raw = &mmap[data_begin..data_end];
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
        .with_context(|| no_image_hdu_error(&hdus))?;

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
        .with_context(|| no_image_hdu_error(&hdus))?;

    Ok(build_merged_header(&hdus, selected_idx))
}

pub fn extract_image_mmap_by_index(file: &File, hdu_index: usize) -> Result<MmapImageResult> {
    let mmap = read_file_bytes(file)?;
    let hdus = scan_all_hdus(&mmap)?;

    if hdu_index >= hdus.len() {
        bail!("HDU index {} out of range (file has {} HDUs)", hdu_index, hdus.len());
    }

    let hdu = &hdus[hdu_index];
    if !hdu.info.has_data {
        bail!(
            "HDU {} cannot be loaded as a 2D image: {}",
            hdu_index,
            hdu_usability_reason(hdu)
        );
    }

    let image = decode_hdu_plane(&mmap, hdu, 0)?;
    let header = build_merged_header(&hdus, hdu_index);

    let base_extension = hdus[hdu_index].info.extname.clone()
        .or_else(|| Some(format!("HDU {}", hdu_index)));
    let selected_extension = if hdus[hdu_index].plane_count > 1 {
        base_extension.map(|name| {
            format!("{} (plane 1 of {})", name.trim(), hdus[hdu_index].plane_count)
        })
    } else if hdu_index > 0 {
        base_extension
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

    let selected_idx = match select_rgb_cube_hdu(&hdus) {
        Some(i) => i,
        None => return Ok(None),
    };

    let hdu = &hdus[selected_idx];
    let h = &hdu.header;
    let naxis3 = hdu.info.naxis3;

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
    let r = ImageRef::parse(path);
    let file = File::open(&r.path)
        .with_context(|| format!("Failed to open {}", r.path))?;
    let result = match &r.plane {
        PlaneSelector::Auto => extract_image_mmap(&file),
        PlaneSelector::Hdu(n) => extract_image_mmap_by_index(&file, *n),
        PlaneSelector::Array(_) => bail!("FITS files have no ASDF arrays; use #hdu=<n>: {}", path),
    }
    .with_context(|| format!("Failed to load {}", path))?;
    Ok(result.image)
}

pub fn read_primary_header(path: &str) -> Result<HduHeader> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open {}", path))?;
    Ok(read_header_blocks(&file, 0)?.header)
}

#[cfg(test)]
pub mod test_fixtures {
    use std::io::Write;

    use crate::types::constants::BLOCK_SIZE;

    pub enum HduData {
        F32(Vec<f32>),
        I32(Vec<i32>),
        I16(Vec<i16>),
    }

    pub struct TestHdu {
        pub extname: Option<&'static str>,
        pub extver: Option<i64>,
        pub cols: usize,
        pub rows: usize,
        pub data: HduData,
        pub extra_cards: Vec<(&'static str, String)>,
    }

    fn card(key: &str, value: &str) -> Vec<u8> {
        let mut bytes = format!("{key:<8}= {value}").into_bytes();
        bytes.resize(80, b' ');
        bytes
    }

    fn header_block(cards: &[(&str, String)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in cards {
            out.extend_from_slice(&card(k, v));
        }
        let mut end = b"END".to_vec();
        end.resize(80, b' ');
        out.extend_from_slice(&end);
        while out.len() % BLOCK_SIZE != 0 {
            out.push(b' ');
        }
        out
    }

    fn pad(mut out: Vec<u8>) -> Vec<u8> {
        while out.len() % BLOCK_SIZE != 0 {
            out.push(0);
        }
        out
    }

    pub fn write_raw_hdus(path: &std::path::Path, hdus: &[(Vec<(&'static str, String)>, Vec<u8>)]) {
        let mut buf = Vec::new();
        for (cards, data) in hdus {
            buf.extend_from_slice(&header_block(cards));
            buf.extend_from_slice(&pad(data.clone()));
        }
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    pub fn empty_primary_cards() -> Vec<(&'static str, String)> {
        vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "0".into()),
            ("EXTEND", "T".into()),
        ]
    }

    pub fn cube_hdu(
        extname: &'static str,
        cols: usize,
        rows: usize,
        planes: usize,
        extra: &[(&'static str, String)],
    ) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let mut cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", cols.to_string()),
            ("NAXIS2", rows.to_string()),
            ("NAXIS3", planes.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", format!("'{extname:<8}'")),
        ];
        cards.extend(extra.iter().cloned());
        let data: Vec<u8> = (0..cols * rows * planes)
            .flat_map(|i| (i as f32).to_be_bytes())
            .collect();
        (cards, data)
    }

    pub fn write_test_mef(path: &std::path::Path, primary_cards: &[(&str, String)], hdus: &[TestHdu]) {
        let mut buf = Vec::new();
        let mut primary: Vec<(&str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "0".into()),
            ("EXTEND", "T".into()),
        ];
        primary.extend(primary_cards.iter().cloned());
        buf.extend_from_slice(&header_block(&primary));
        for hdu in hdus {
            let (bitpix, data) = match &hdu.data {
                HduData::F32(v) => ("-32", v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>()),
                HduData::I32(v) => ("32", v.iter().flat_map(|x| x.to_be_bytes()).collect()),
                HduData::I16(v) => ("16", v.iter().flat_map(|x| x.to_be_bytes()).collect()),
            };
            let mut cards: Vec<(&str, String)> = vec![
                ("XTENSION", "'IMAGE   '".into()),
                ("BITPIX", bitpix.into()),
                ("NAXIS", "2".into()),
                ("NAXIS1", hdu.cols.to_string()),
                ("NAXIS2", hdu.rows.to_string()),
                ("PCOUNT", "0".into()),
                ("GCOUNT", "1".into()),
            ];
            if let Some(n) = hdu.extname {
                cards.push(("EXTNAME", format!("'{n:<8}'")));
            }
            if let Some(v) = hdu.extver {
                cards.push(("EXTVER", v.to_string()));
            }
            cards.extend(hdu.extra_cards.iter().cloned());
            buf.extend_from_slice(&header_block(&cards));
            buf.extend_from_slice(&pad(data));
        }
        std::fs::File::create(path).unwrap().write_all(&buf).unwrap();
    }

    pub fn ramp_f32(cols: usize, rows: usize) -> Vec<f32> {
        (0..cols * rows).map(|i| i as f32).collect()
    }

    pub fn sci_err_dq_mef(path: &std::path::Path, cols: usize, rows: usize, dq_bits: Vec<i32>) {
        sci_err_dq_mef_with_dq_cards(path, cols, rows, dq_bits, vec![]);
    }

    pub fn sci_err_dq_mef_with_dq_cards(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        dq_bits: Vec<i32>,
        dq_cards: Vec<(&'static str, String)>,
    ) {
        let mut dq_extra: Vec<(&'static str, String)> = vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into())];
        dq_extra.extend(dq_cards);
        write_test_mef(
            path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols, rows, data: HduData::F32(ramp_f32(cols, rows)), extra_cards: vec![("BUNIT", "'MJy/sr'".into())] },
                TestHdu { extname: Some("ERR"), extver: Some(1), cols, rows, data: HduData::F32(ramp_f32(cols, rows).iter().map(|v| v * 0.5).collect()), extra_cards: vec![("BUNIT", "'MJy/sr'".into())] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols, rows, data: HduData::I32(dq_bits), extra_cards: dq_extra },
                TestHdu { extname: Some("SCI"), extver: Some(2), cols, rows, data: HduData::F32(ramp_f32(cols, rows)), extra_cards: vec![] },
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_fixtures::*;

    #[test]
    fn decode_int_bitpix8_with_bzero() {
        let px = decode_pixels_int(&[0, 255, 7], 8, 1.0, 0.0).unwrap();
        assert!(matches!(px, IntPixels::U32(ref v) if v == &[0, 255, 7]));
        let px = decode_pixels_int(&[0, 255], 8, 1.0, 10.0).unwrap();
        assert!(matches!(px, IntPixels::U32(ref v) if v == &[10, 265]));
        assert!(decode_pixels_int(&[1], 8, 1.0, -1.0).is_err());
    }

    #[test]
    fn decode_int_bitpix16_unsigned_via_bzero_32768() {
        let data: Vec<u8> = [i16::MAX, i16::MIN, -1i16, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let px = decode_pixels_int(&data, 16, 1.0, 32768.0).unwrap();
        match px {
            IntPixels::U32(v) => assert_eq!(v, vec![65535, 0, 32767, 32768]),
            _ => panic!("expected U32"),
        }
    }

    #[test]
    fn decode_int_bitpix16_signed_negative() {
        let data: Vec<u8> = [-5i16, 300].iter().flat_map(|v| v.to_be_bytes()).collect();
        match decode_pixels_int(&data, 16, 1.0, 0.0).unwrap() {
            IntPixels::I32(v) => assert_eq!(v, vec![-5, 300]),
            _ => panic!("expected I32"),
        }
        match decode_pixels_int(&data, 16, 1.0, -10.0).unwrap() {
            IntPixels::I32(v) => assert_eq!(v, vec![-15, 290]),
            _ => panic!("expected I32"),
        }
    }

    #[test]
    fn decode_int_bitpix32_unsigned_via_bzero_keeps_bit31() {
        let data: Vec<u8> = [-1i32, i32::MIN, 1].iter().flat_map(|v| v.to_be_bytes()).collect();
        match decode_pixels_int(&data, 32, 1.0, 2147483648.0).unwrap() {
            IntPixels::U32(v) => {
                assert_eq!(v, vec![0x7FFF_FFFF, 0, 0x8000_0001]);
                assert_ne!(v[2] & (1 << 31), 0);
            }
            _ => panic!("expected U32"),
        }
    }

    #[test]
    fn decode_int_bitpix32_signed_and_overflow() {
        let data: Vec<u8> = [-7i32, i32::MAX].iter().flat_map(|v| v.to_be_bytes()).collect();
        match decode_pixels_int(&data, 32, 1.0, 0.0).unwrap() {
            IntPixels::I32(v) => assert_eq!(v, vec![-7, i32::MAX]),
            _ => panic!("expected I32"),
        }
        assert!(decode_pixels_int(&data, 32, 1.0, 1.0).is_err());
    }

    #[test]
    fn decode_int_rejects_scaled_and_float() {
        assert!(decode_pixels_int(&[1, 2], 16, 2.0, 0.0).is_err());
        assert!(decode_pixels_int(&[1, 2], 16, 1.0, 0.5).is_err());
        assert!(decode_pixels_int(&[0; 4], -32, 1.0, 0.0).is_err());
        assert!(decode_pixels_int(&[0; 8], 64, 1.0, 0.0).is_err());
        assert!(decode_pixels_int(&[0; 8], -64, 1.0, 0.0).is_err());
    }

    #[test]
    fn int_pixels_to_plane_marks_signedness() {
        let p = int_pixels_to_plane(IntPixels::I32(vec![-1, 2]), 1, 2).unwrap();
        assert!(p.signed);
        assert_eq!(p.bits[[0, 0]], u32::MAX);
        assert_eq!(p.value_at(0, 0), -1);
        let p = int_pixels_to_plane(IntPixels::U32(vec![u32::MAX]), 1, 1).unwrap();
        assert!(!p.signed);
        assert_eq!(p.value_at(0, 0), 4294967295);
        assert!(int_pixels_to_plane(IntPixels::U32(vec![1, 2, 3]), 2, 2).is_err());
    }

    #[test]
    fn extract_int_plane_by_index_preserves_high_bits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dq.fits");
        let mut dq = vec![0i32; 16];
        dq[5] = 1;
        dq[9] = ((2147483648u32 | 1) as i64 - 2147483648) as i32;
        sci_err_dq_mef(&path, 4, 4, dq);
        let file = File::open(&path).unwrap();
        let plane = extract_int_plane_by_index(&file, 3).unwrap();
        assert!(!plane.signed);
        assert_eq!(plane.bits.dim(), (4, 4));
        assert_eq!(plane.bits[[1, 1]], 2147483649);
        assert_eq!(plane.bits[[2, 1]], 2147483649);
        assert_eq!(plane.bits[[1, 1]] & (1 << 31), 1 << 31);
        assert_eq!(plane.value_at(2, 1), 2147483649);
        assert!(extract_int_plane_by_index(&file, 1).is_err());
        assert!(extract_int_plane_by_index(&file, 0).is_err());
        assert!(extract_int_plane_by_index(&file, 9).is_err());
    }

    #[test]
    fn extract_int_plane_by_index_rejects_a_cube() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dq_cube.fits");
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "3".into()),
            ("NAXIS3", "5".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'DQ      '".into()),
        ];
        let data: Vec<u8> = (0..4 * 3 * 5).flat_map(|i| (i as i32).to_be_bytes()).collect();
        write_raw_hdus(&path, &[(empty_primary_cards(), Vec::new()), (cards, data)]);
        let file = File::open(&path).unwrap();
        let err = extract_int_plane_by_index(&file, 1).unwrap_err();
        assert!(
            err.to_string().contains("3D cube of 5 planes"),
            "a DQ cube must not be silently truncated to plane 0: {err}"
        );
    }

    #[test]
    fn extract_int_plane_by_index_rejects_compressed() {
        let dir = match compressed_fixtures_dir() {
            Some(d) => d,
            None => return,
        };
        let path = dir.join("rice_i16_default_tile.fits");
        if !path.exists() {
            return;
        }
        let file = File::open(&path).unwrap();
        let err = extract_int_plane_by_index(&file, 1).unwrap_err();
        assert!(err.to_string().contains("compressed"), "{err}");
    }

    #[test]
    fn auto_hdu_index_prefers_sci() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mef.fits");
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("WEIGHT"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("SCI"), extver: None, cols: 6, rows: 3, data: HduData::F32(ramp_f32(6, 3)), extra_cards: vec![] },
            ],
        );
        let file = File::open(&path).unwrap();
        assert_eq!(auto_hdu_index(&file).unwrap(), 2);
        let merged = extract_header_by_index_merged(&file, 2).unwrap();
        assert_eq!(merged.get("EXTNAME"), Some("SCI"));
        assert_eq!(merged.get("EXTEND"), Some("T"));
        assert!(extract_header_by_index_merged(&file, 5).is_err());
    }

    fn bintable_hdu(
        extname: &'static str,
        row_bytes: usize,
        rows: usize,
    ) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", row_bytes.to_string()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "2".into()),
            ("TTYPE1", "'WAVELENGTH'".into()),
            ("TFORM1", "'1D      '".into()),
            ("TTYPE2", "'FLUX    '".into()),
            ("TFORM2", "'1D      '".into()),
            ("EXTNAME", format!("'{extname:<8}'")),
        ];
        (cards, vec![0u8; row_bytes * rows])
    }

    #[test]
    fn spectral_cube_is_rejected_instead_of_collapsing_to_plane_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s3d.fits");
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 5, &[("CTYPE3", "'WAVE    '".into())]),
            ],
        );
        let file = File::open(&path).unwrap();

        let err = auto_hdu_index(&file).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.starts_with("HDU 1 (SCI): 3D cube of 5 planes"), "{msg}");
        assert!(msg.contains("NAXIS=3 [4x3x5]"), "{msg}");

        assert!(extract_image_mmap(&file).is_err());
        assert!(extract_header_mmap(&file).is_err());
        assert!(load_fits_image(path.to_str().unwrap()).is_err());
    }

    #[test]
    fn spectral_cube_stays_openable_through_an_explicit_hdu_reference() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s3d_explicit.fits");
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 5, &[("CTYPE3", "'WAVE    '".into())]),
            ],
        );
        let file = File::open(&path).unwrap();

        let exts = list_extensions(&file).unwrap();
        assert!(exts[1].has_data, "the cube HDU still holds pixel data");
        assert_eq!(exts[1].naxis3, 5);

        let result = extract_image_mmap_by_index(&file, 1).expect("explicit #hdu= must open a cube");
        assert_eq!(result.image.dim(), (3, 4));
        assert_eq!(result.image[[0, 0]], 0.0);
        assert_eq!(result.image[[2, 3]], 11.0);
        assert_eq!(result.selected_extension.as_deref(), Some("SCI (plane 1 of 5)"));
        assert_eq!(result.header.get("NAXIS3"), Some("5"));

        let by_ref = format!("{}#hdu=1", path.to_str().unwrap());
        assert_eq!(load_fits_image(&by_ref).unwrap().dim(), (3, 4));
    }

    #[test]
    fn degenerate_third_and_fourth_axes_still_load_as_2d() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("degenerate.fits");
        let mut four_d = cube_hdu("SCI", 5, 2, 1, &[]);
        four_d.0.retain(|(k, _)| *k != "NAXIS");
        four_d.0.push(("NAXIS", "4".into()));
        four_d.0.push(("NAXIS4", "1".into()));
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 6, 4, 1, &[]),
            ],
        );
        let file = File::open(&path).unwrap();
        assert_eq!(auto_hdu_index(&file).unwrap(), 1);
        assert_eq!(extract_image_mmap(&file).unwrap().image.dim(), (4, 6));

        let path4 = dir.path().join("degenerate4.fits");
        write_raw_hdus(&path4, &[(empty_primary_cards(), Vec::new()), four_d]);
        let file4 = File::open(&path4).unwrap();
        assert_eq!(auto_hdu_index(&file4).unwrap(), 1);
        assert_eq!(extract_image_mmap(&file4).unwrap().image.dim(), (2, 5));
    }

    #[test]
    fn rejection_message_lists_the_hdu_inventory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x1d.fits");
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                bintable_hdu("EXTRACT1D", 16, 4),
            ],
        );
        let file = File::open(&path).unwrap();
        let msg = format!("{:#}", auto_hdu_index(&file).unwrap_err());

        assert!(msg.contains("EXTRACT1D"), "{msg}");
        assert!(msg.contains("BINTABLE"), "{msg}");
        assert!(msg.contains("holds no image pixels"), "{msg}");
        assert!(msg.contains("[0] EXTNAME=(none) XTENSION=PRIMARY"), "{msg}");
        assert!(!msg.contains("not found"), "{msg}");
        assert!(!msg.contains("No such file"), "{msg}");
        assert!(!msg.contains("Permission denied"), "{msg}");
        assert!(!msg.contains("Calibration reference file"), "{msg}");
    }

    #[test]
    fn rejection_message_front_loads_the_verdict_for_the_most_image_like_hdu() {
        let dir = tempfile::tempdir().unwrap();

        let x1d = dir.path().join("verdict_x1d.fits");
        write_raw_hdus(
            &x1d,
            &[
                (empty_primary_cards(), Vec::new()),
                bintable_hdu("EXTRACT1D", 16, 4),
            ],
        );
        let msg = format!("{:#}", auto_hdu_index(&File::open(&x1d).unwrap()).unwrap_err());
        assert!(
            msg.starts_with("HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels;"),
            "{msg}"
        );

        let s3d = dir.path().join("verdict_s3d.fits");
        write_raw_hdus(
            &s3d,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 6, 4, 1400, &[("CTYPE3", "'WAVE    '".into())]),
                cube_hdu("ERR", 6, 4, 1400, &[]),
                bintable_hdu("HDRTAB", 8, 2),
            ],
        );
        let msg = format!("{:#}", auto_hdu_index(&File::open(&s3d).unwrap()).unwrap_err());
        assert!(
            msg.starts_with("HDU 1 (SCI): 3D cube of 1400 planes, not a single 2D image;"),
            "{msg}"
        );
        assert!(msg[..50].contains("cube"), "the first 50 rendered characters must carry the verdict: {msg}");
    }

    #[test]
    fn rgb_cube_is_rejected_only_when_the_third_axis_is_not_a_colour_axis() {
        let dir = tempfile::tempdir().unwrap();

        let unmarked = dir.path().join("three_plane.fits");
        write_raw_hdus(
            &unmarked,
            &[(empty_primary_cards(), Vec::new()), cube_hdu("SCI", 4, 3, 3, &[])],
        );
        let stack = try_extract_rgb_mmap(&File::open(&unmarked).unwrap())
            .unwrap()
            .expect("an unmarked 3-plane stack is still a colour composite");
        assert_eq!(stack.r.dim(), (3, 4));
        assert_eq!(stack.b[[0, 0]], 24.0);

        let spectral = dir.path().join("three_plane_wave.fits");
        write_raw_hdus(
            &spectral,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 3, &[("CTYPE3", "'WAVE    '".into())]),
            ],
        );
        assert!(try_extract_rgb_mmap(&File::open(&spectral).unwrap()).unwrap().is_none());

        let velocity = dir.path().join("three_plane_velo.fits");
        write_raw_hdus(
            &velocity,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 3, &[("CTYPE3", "'VELO-LSR'".into())]),
            ],
        );
        assert!(try_extract_rgb_mmap(&File::open(&velocity).unwrap()).unwrap().is_none());

        let calibrated_axis = dir.path().join("three_plane_cunit.fits");
        write_raw_hdus(
            &calibrated_axis,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 3, &[("CUNIT3", "'um      '".into())]),
            ],
        );
        assert!(try_extract_rgb_mmap(&File::open(&calibrated_axis).unwrap()).unwrap().is_none());

        let sampled_axis = dir.path().join("three_plane_crval.fits");
        write_raw_hdus(
            &sampled_axis,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 3, &[("CRVAL3", "4.9".into()), ("CDELT3", "0.008".into())]),
            ],
        );
        assert!(try_extract_rgb_mmap(&File::open(&sampled_axis).unwrap()).unwrap().is_none());

        let colour_named_table = dir.path().join("colortab.fits");
        write_raw_hdus(
            &colour_named_table,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("COLORTAB", 4, 3, 3, &[("CTYPE3", "'WAVE    '".into())]),
            ],
        );
        assert!(
            try_extract_rgb_mmap(&File::open(&colour_named_table).unwrap()).unwrap().is_none(),
            "EXTNAME must match a colour name exactly, not merely contain one"
        );

        let ctype = dir.path().join("rgb_ctype.fits");
        write_raw_hdus(
            &ctype,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 3, &[("CTYPE3", "'RGB     '".into())]),
            ],
        );
        let rgb = try_extract_rgb_mmap(&File::open(&ctype).unwrap()).unwrap().unwrap();
        assert_eq!(rgb.r.dim(), (3, 4));
        assert_eq!(rgb.r[[0, 0]], 0.0);
        assert_eq!(rgb.g[[0, 0]], 12.0);
        assert_eq!(rgb.b[[0, 0]], 24.0);

        let named = dir.path().join("rgb_extname.fits");
        write_raw_hdus(
            &named,
            &[(empty_primary_cards(), Vec::new()), cube_hdu("RGB", 4, 3, 3, &[])],
        );
        assert!(try_extract_rgb_mmap(&File::open(&named).unwrap()).unwrap().is_some());

        let marked_with_axis_cards = dir.path().join("rgb_with_axis_cards.fits");
        write_raw_hdus(
            &marked_with_axis_cards,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu(
                    "SCI",
                    4,
                    3,
                    3,
                    &[("CTYPE3", "'RGB     '".into()), ("CUNIT3", "'        '".into())],
                ),
            ],
        );
        assert!(
            try_extract_rgb_mmap(&File::open(&marked_with_axis_cards).unwrap()).unwrap().is_some(),
            "an explicit colour CTYPE3 outranks the non-colour axis heuristics"
        );
    }

    #[test]
    fn compressed_3d_cube_is_not_selectable_as_an_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zcube.fits");
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "8".into()),
            ("NAXIS2", "3".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("TTYPE1", "'COMPRESSED_DATA'".into()),
            ("TFORM1", "'1PB     '".into()),
            ("ZIMAGE", "T".into()),
            ("ZCMPTYPE", "'RICE_1  '".into()),
            ("ZBITPIX", "-32".into()),
            ("ZNAXIS", "3".into()),
            ("ZNAXIS1", "4".into()),
            ("ZNAXIS2", "3".into()),
            ("ZNAXIS3", "5".into()),
            ("EXTNAME", "'SCI     '".into()),
        ];
        write_raw_hdus(
            &path,
            &[(empty_primary_cards(), Vec::new()), (cards, vec![0u8; 24])],
        );
        let file = File::open(&path).unwrap();
        let msg = format!("{:#}", auto_hdu_index(&file).unwrap_err());
        assert!(msg.starts_with("HDU 1 (SCI): 3D cube of 5 planes"), "{msg}");
        assert!(msg.contains("ZNAXIS=3 [4x3x5]"), "{msg}");
    }

    #[test]
    fn compressed_four_dimensional_block_reports_its_real_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("z4d.fits");
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "8".into()),
            ("NAXIS2", "3".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("TTYPE1", "'COMPRESSED_DATA'".into()),
            ("TFORM1", "'1PB     '".into()),
            ("ZIMAGE", "T".into()),
            ("ZCMPTYPE", "'RICE_1  '".into()),
            ("ZBITPIX", "-32".into()),
            ("ZNAXIS", "4".into()),
            ("ZNAXIS1", "100".into()),
            ("ZNAXIS2", "100".into()),
            ("ZNAXIS3", "1".into()),
            ("ZNAXIS4", "7".into()),
            ("EXTNAME", "'SCI     '".into()),
        ];
        write_raw_hdus(
            &path,
            &[(empty_primary_cards(), Vec::new()), (cards, vec![0u8; 24])],
        );
        let file = File::open(&path).unwrap();
        let msg = format!("{:#}", auto_hdu_index(&file).unwrap_err());
        assert!(msg.contains("4D cube of 7 planes"), "{msg}");
        assert!(!msg.contains("degenerate image dimensions"), "{msg}");
    }

    #[test]
    fn declared_plane_count_saturates_instead_of_overflowing() {
        let header = HduHeader {
            cards: vec![],
            index: HashMap::from([
                ("NAXIS".to_string(), "6".to_string()),
                ("NAXIS3".to_string(), "3037000500".to_string()),
                ("NAXIS4".to_string(), "3037000500".to_string()),
                ("NAXIS5".to_string(), "3037000500".to_string()),
                ("NAXIS6".to_string(), "3037000500".to_string()),
            ]),
        };
        assert_eq!(declared_plane_count(&header, 6), i64::MAX);
    }

    #[test]
    fn load_fits_image_honours_hdu_ref() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.fits");
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("A"), extver: None, cols: 5, rows: 2, data: HduData::F32(ramp_f32(5, 2)), extra_cards: vec![] },
                TestHdu { extname: Some("SCI"), extver: None, cols: 3, rows: 7, data: HduData::F32(ramp_f32(3, 7)), extra_cards: vec![] },
            ],
        );
        let key = format!("{}#hdu=1", path.to_str().unwrap());
        assert_eq!(load_fits_image(&key).unwrap().dim(), (2, 5));
        assert_eq!(load_fits_image(path.to_str().unwrap()).unwrap().dim(), (7, 3));
        assert!(load_fits_image(&format!("{}#array=dq", path.to_str().unwrap())).is_err());
    }

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
