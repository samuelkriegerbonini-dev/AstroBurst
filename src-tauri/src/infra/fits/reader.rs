use std::collections::{HashMap, HashSet};
use std::fs::File;

use anyhow::{bail, Context, Result};
use memmap2::{Mmap, MmapOptions};
use ndarray::Array2;
use rayon::prelude::*;

use crate::types::constants::BLOCK_SIZE;
use crate::types::header::{is_commentary_key, parse_fits_float, MAX_FITS_AXES};
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

fn scale_card(header: &HduHeader, key: &str, default: f64) -> Result<f64> {
    match header.get(key).map(str::trim).filter(|raw| !raw.is_empty()) {
        None => Ok(default),
        Some(raw) => parse_fits_float(raw)
            .with_context(|| format!("{key} = '{raw}' is not a number; the pixel values cannot be scaled")),
    }
}

fn scaling(header: &HduHeader) -> Result<(f64, f64)> {
    Ok((scale_card(header, "BZERO", 0.0)?, scale_card(header, "BSCALE", 1.0)?))
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
        8 if bzero < 0.0 => {
            if bzero < i32::MIN as f64 {
                bail!("BZERO {} out of range for BITPIX 8", bzero);
            }
            let off = bzero as i32;
            Ok(IntPixels::I32(data.par_iter().map(|&b| b as i32 + off).collect()))
        }
        8 => {
            if bzero > u32::MAX as f64 {
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
    let (bzero, bscale) = scaling(h)?;
    let px = decode_pixels_int(&mmap[hdu.info.data_start..data_end], bitpix, bscale, bzero)?;
    int_pixels_to_plane(px, naxis2, naxis1)
}

pub fn auto_hdu_index(file: &File) -> Result<usize> {
    let hdus = scan_hdu_headers(file)?;
    if hdus.is_empty() {
        bail!("No HDUs found in FITS file");
    }
    select_auto_image_hdu(&hdus)
}

pub fn extract_header_by_index_merged(file: &File, hdu_index: usize) -> Result<HduHeader> {
    let hdus = scan_hdu_headers(file)?;
    if hdu_index >= hdus.len() {
        bail!("HDU index {} out of range (file has {} HDUs)", hdu_index, hdus.len());
    }
    Ok(build_merged_header(&hdus, hdu_index))
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

enum HeaderCard {
    Value { key: String, value: String, quoted: bool, continues: bool },
    Continue { value: String, continues: bool },
    Commentary { key: String, text: String },
    Ignored,
}

fn card_value(raw: &str) -> (String, bool, bool) {
    let quoted = raw.trim_start().starts_with('\'');
    let value = extract_header_value(raw);
    let continues = quoted && value.ends_with('&');
    (value, quoted, continues)
}

fn parse_card(card: &[u8]) -> HeaderCard {
    let keyword = String::from_utf8_lossy(&card[..8]).trim().to_string();
    let body = String::from_utf8_lossy(&card[8..]);
    if is_commentary_key(&keyword) {
        return HeaderCard::Commentary { key: keyword, text: body.trim().to_string() };
    }
    match keyword.as_str() {
        "CONTINUE" if body.trim_start().starts_with('\'') => {
            let (value, _, continues) = card_value(&body);
            HeaderCard::Continue { value, continues }
        }
        "HIERARCH" => match body.split_once('=') {
            Some((name, raw)) if !name.trim().is_empty() => {
                let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
                let (value, quoted, continues) = card_value(raw);
                HeaderCard::Value { key: format!("HIERARCH {name}"), value, quoted, continues }
            }
            _ => HeaderCard::Ignored,
        },
        _ if body.starts_with("= ") => {
            let (value, quoted, continues) = card_value(&body[2..]);
            HeaderCard::Value { key: keyword, value, quoted, continues }
        }
        _ => HeaderCard::Ignored,
    }
}

const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];

fn reject_gzip(head: &[u8]) -> Result<()> {
    if head.starts_with(&GZIP_MAGIC) {
        bail!("gzip-compressed FITS is not supported; decompress first");
    }
    Ok(())
}

fn reject_gzip_file(file: &File) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};

    let mut f = file;
    f.seek(SeekFrom::Start(0)).context("seek to the start of the file failed")?;
    let mut head = Vec::with_capacity(GZIP_MAGIC.len());
    f.take(GZIP_MAGIC.len() as u64)
        .read_to_end(&mut head)
        .context("reading the start of the file failed")?;
    reject_gzip(&head)
}

pub fn parse_header_at(mmap: &[u8], offset: usize) -> Result<ParsedHdu> {
    if offset == 0 {
        reject_gzip(mmap)?;
    }
    parse_header_blocks(mmap, offset)
}

fn parse_header_blocks(mmap: &[u8], offset: usize) -> Result<ParsedHdu> {
    let mut cards: Vec<(String, String)> = Vec::new();
    let mut index = HashMap::new();
    let mut string_keys = HashSet::new();
    let mut long_string: Option<usize> = None;
    let mut pos = offset;
    let mut end_found = false;

    while !end_found {
        let block = pos
            .checked_add(BLOCK_SIZE)
            .and_then(|end| mmap.get(pos..end))
            .with_context(|| format!("Unexpected end of file while reading header at offset {}", offset))?;
        pos += BLOCK_SIZE;

        for card_bytes in block.chunks_exact(80) {
            if String::from_utf8_lossy(&card_bytes[..8]).trim() == "END" {
                end_found = true;
                break;
            }

            let open = long_string.take();
            match parse_card(card_bytes) {
                HeaderCard::Value { key, value, quoted, continues } => {
                    long_string = continues.then_some(cards.len());
                    if quoted {
                        string_keys.insert(key.clone());
                    } else {
                        string_keys.remove(&key);
                    }
                    index.insert(key.clone(), value.clone());
                    cards.push((key, value));
                }
                HeaderCard::Continue { value, continues } => {
                    if let Some((key, text)) = open.and_then(|i| cards.get_mut(i)) {
                        text.pop();
                        text.push_str(&value);
                        index.insert(key.clone(), text.clone());
                        long_string = open.filter(|_| continues);
                    }
                }
                HeaderCard::Commentary { key, text } => cards.push((key, text)),
                HeaderCard::Ignored => {}
            }
        }
    }

    let header = HduHeader { cards, index, string_keys: Some(string_keys) };
    let data_start = pos;
    let next_hdu_offset = header
        .checked_padded_data_bytes()
        .and_then(|padded| data_start.checked_add(padded).context("data unit end overflows"))
        .with_context(|| format!("Invalid data unit size in the header at offset {}", offset))?;

    Ok(ParsedHdu {
        header,
        header_start: offset,
        data_start,
        next_hdu_offset,
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
    reject_gzip(mmap)?;
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

    if offset == 0 {
        reject_gzip_file(file)?;
    }
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

    let parsed = parse_header_blocks(&buf, 0)?;
    let shift = |relative: usize| {
        offset
            .checked_add(relative)
            .with_context(|| format!("HDU offsets overflow in the header at offset {offset}"))
    };
    Ok(ParsedHdu {
        data_start: shift(parsed.data_start)?,
        next_hdu_offset: shift(parsed.next_hdu_offset)?,
        header: parsed.header,
        header_start: offset,
    })
}

fn scan_hdu_headers(file: &File) -> Result<Vec<ScannedHdu>> {
    let file_len = file.metadata().context("stat failed")?.len() as usize;
    if file_len == 0 {
        bail!("File is empty (0 bytes) — the download or copy may have failed");
    }
    reject_gzip_file(file)?;
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

fn select_auto_image_hdu(hdus: &[ScannedHdu]) -> Result<usize> {
    let selected = select_best_image_hdu(hdus).with_context(|| no_image_hdu_error(hdus))?;
    if let Some(rgb) = select_rgb_cube_hdu(hdus).and_then(|i| hdus.get(i)) {
        bail!(
            "{} holds an RGB colour cube, so this file opens as a colour composite and has no single mono image; {} is a separate plane, open it explicitly with #hdu={}",
            hdu_label(rgb),
            hdu_label(&hdus[selected]),
            selected
        );
    }
    Ok(selected)
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

fn has_rgb_cube_shape(hdu: &ScannedHdu) -> bool {
    (hdu.is_image_hdu || hdu.is_compressed)
        && hdu.info.naxis == 3
        && (3..=4).contains(&hdu.info.naxis3)
        && hdu.info.naxis1 > 1
        && hdu.info.naxis2 > 1
}

fn is_science_extension(hdu: &ScannedHdu) -> bool {
    hdu.info
        .extname
        .as_deref()
        .is_some_and(|n| n.trim().eq_ignore_ascii_case("SCI"))
}

fn select_rgb_cube_hdu(hdus: &[ScannedHdu]) -> Option<usize> {
    if let Some(marked) = hdus.iter().position(|hdu| has_rgb_cube_shape(hdu) && has_rgb_marker(hdu)) {
        return Some(marked);
    }
    let has_2d_science = hdus.iter().any(|hdu| hdu.is_single_plane_image && is_science_extension(hdu));
    if has_2d_science {
        return None;
    }
    let has_2d_image = hdus.iter().any(|hdu| hdu.is_single_plane_image);
    hdus.iter().position(|hdu| {
        has_rgb_cube_shape(hdu)
            && !third_axis_is_not_colour(hdu)
            && (!has_2d_image || hdu.info.index == 0)
    })
}

fn image_view_header(hdu: &ScannedHdu) -> HduHeader {
    let mut header = hdu.header.clone();
    if !hdu.is_compressed {
        return header;
    }
    let storage_axes = hdu.header.get_i64("NAXIS").unwrap_or(0).clamp(0, MAX_FITS_AXES);
    let image_axes = hdu.info.naxis.clamp(0, MAX_FITS_AXES);
    header.set("BITPIX", hdu.info.bitpix.to_string());
    header.set("NAXIS", image_axes.to_string());
    for axis in 1..=storage_axes.max(image_axes) {
        let key = format!("NAXIS{axis}");
        match hdu.header.get(&format!("ZNAXIS{axis}")).filter(|_| axis <= image_axes) {
            Some(len) => header.set(&key, len.trim().to_string()),
            None => header.remove(&key),
        }
    }
    header
}

fn build_merged_header(hdus: &[ScannedHdu], selected_idx: usize) -> HduHeader {
    let selected = image_view_header(&hdus[selected_idx]);
    if selected_idx == 0 || hdus.len() == 1 {
        return selected;
    }
    hdus[0].header.merge_with(&selected)
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
    let (bzero, bscale) = scaling(h)?;
    let pixels = decode_pixels_blank(raw, bitpix, bscale, bzero, blank_value(h));
    let image = Array2::from_shape_vec((naxis2, naxis1), pixels)
        .context("Failed to reshape image pixels")?;

    Ok(image)
}

pub struct MmapImageResult {
    pub header: HduHeader,
    pub image: Array2<f32>,
    pub selected_extension: Option<String>,
    pub extensions: Vec<HduInfo>,
}

pub struct MmapRgbResult {
    pub header: HduHeader,
    pub r: Array2<f32>,
    pub g: Array2<f32>,
    pub b: Array2<f32>,
}

pub fn extract_image_mmap(file: &File) -> Result<MmapImageResult> {
    let mmap = read_file_bytes(file)?;
    let hdus = scan_all_hdus(&mmap)?;

    if hdus.is_empty() {
        bail!("No HDUs found in FITS file");
    }

    let selected_idx = select_auto_image_hdu(&hdus)?;

    let image = extract_image_from_hdu(&mmap, &hdus[selected_idx])?;
    let header = build_merged_header(&hdus, selected_idx);

    let selected_extension = if selected_idx > 0 {
        hdus[selected_idx].info.extname.clone()
            .or_else(|| Some(format!("HDU {}", selected_idx)))
    } else {
        None
    };

    let extensions: Vec<HduInfo> = hdus.iter().map(|h| h.info.clone()).collect();

    Ok(MmapImageResult {
        header,
        image,
        selected_extension,
        extensions,
    })
}

pub fn extract_header_mmap(file: &File) -> Result<HduHeader> {
    let hdus = scan_hdu_headers(file)?;

    if hdus.is_empty() {
        bail!("No HDUs found in FITS file");
    }

    let selected_idx = match select_rgb_cube_hdu(&hdus) {
        Some(rgb) => rgb,
        None => select_auto_image_hdu(&hdus)?,
    };

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

    Ok(MmapImageResult {
        header,
        image,
        selected_extension,
        extensions,
    })
}

pub fn try_extract_rgb_mmap(file: &File) -> Result<Option<MmapRgbResult>> {
    let mmap = read_file_bytes(file)?;
    let hdus = scan_all_hdus(&mmap)?;

    if hdus.is_empty() {
        bail!("No HDUs found in FITS file");
    }

    let Some(hdu) = select_rgb_cube_hdu(&hdus).and_then(|i| hdus.get(i)) else {
        return Ok(None);
    };

    let (r, g, b) = if hdu.is_compressed {
        let mut planes = compress::decode_compressed_planes(&mmap, &hdu.header, hdu.info.data_start)?;
        if planes.len() < 3 {
            bail!("Compressed RGB HDU decoded {} planes, expected at least 3", planes.len());
        }
        let b = planes.swap_remove(2);
        let g = planes.swap_remove(1);
        let r = planes.swap_remove(0);
        (r, g, b)
    } else {
        (
            decode_hdu_plane(&mmap, hdu, 0).context("Failed to decode the R channel")?,
            decode_hdu_plane(&mmap, hdu, 1).context("Failed to decode the G channel")?,
            decode_hdu_plane(&mmap, hdu, 2).context("Failed to decode the B channel")?,
        )
    };

    Ok(Some(MmapRgbResult {
        header: build_merged_header(&hdus, hdu.info.index),
        r,
        g,
        b,
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

    pub fn plane_hdu(
        extname: &'static str,
        cols: usize,
        rows: usize,
    ) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", cols.to_string()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", format!("'{extname:<8}'")),
        ];
        let data: Vec<u8> = (0..cols * rows).flat_map(|i| (i as f32).to_be_bytes()).collect();
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
        let px = decode_pixels_int(&[0, 127, 128, 255], 8, 1.0, -128.0).unwrap();
        assert!(matches!(px, IntPixels::I32(ref v) if v == &[-128, -1, 0, 127]));
        let px = decode_pixels_int(&[1], 8, 1.0, -1.0).unwrap();
        assert!(matches!(px, IntPixels::I32(ref v) if v == &[0]));
        assert!(decode_pixels_int(&[1], 8, 1.0, -3.0e9).is_err());
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
            string_keys: None,
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

        let header = extract_header_mmap(&file).unwrap();
        assert_eq!(header.get_i64("NAXIS1"), Some(37));
        assert_eq!(header.get_i64("NAXIS2"), Some(23));
        assert_eq!(header.get_i64("BITPIX"), Some(16));
        assert_eq!(extract_header_by_index_merged(&file, 1).unwrap().get_i64("NAXIS1"), Some(37));
        let loaded = extract_image_mmap(&file).unwrap();
        assert_eq!(loaded.header.get_i64("NAXIS1"), Some(loaded.image.ncols() as i64));
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

    fn raw_header_block(cards: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for card in cards.iter().chain(std::iter::once(&"END")) {
            let mut bytes = card.as_bytes().to_vec();
            assert!(bytes.len() <= 80, "card longer than 80 bytes: {card}");
            bytes.resize(80, b' ');
            out.extend_from_slice(&bytes);
        }
        while out.len() % BLOCK_SIZE != 0 {
            out.push(b' ');
        }
        out
    }

    fn primary_image_cards(bitpix: &str, axes: &[&str], extra: &[(&'static str, String)]) -> Vec<(&'static str, String)> {
        const AXIS_KEYS: [&str; 4] = ["NAXIS1", "NAXIS2", "NAXIS3", "NAXIS4"];
        let mut cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", bitpix.into()),
            ("NAXIS", axes.len().to_string()),
        ];
        cards.extend(AXIS_KEYS.iter().zip(axes).map(|(k, v)| (*k, v.to_string())));
        cards.push(("EXTEND", "T".into()));
        cards.extend(extra.iter().cloned());
        cards
    }

    fn compressed_image_hdu(znaxis: &[usize]) -> (Vec<(&'static str, String)>, Vec<u8>) {
        const ZAXIS_KEYS: [&str; 3] = ["ZNAXIS1", "ZNAXIS2", "ZNAXIS3"];
        let mut cards: Vec<(&'static str, String)> = vec![
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
            ("ZBITPIX", "16".into()),
            ("ZNAXIS", znaxis.len().to_string()),
        ];
        cards.extend(ZAXIS_KEYS.iter().zip(znaxis).map(|(k, v)| (*k, v.to_string())));
        cards.push(("EXTNAME", "'SCI     '".into()));
        (cards, vec![0u8; 24])
    }

    #[test]
    fn header_keeps_commentary_hierarch_and_continued_strings() {
        let bytes = raw_header_block(&[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
            "FILENAME= 'jw01234_very_long_&'",
            "CONTINUE  'middle_part_&'",
            "CONTINUE  'rest.fits'          / tail comment",
            "HIERARCH ESO DET DIT = 10.0 / integration time",
            "HIERARCH ESO INS FILT1 NAME = 'Ks' / filter",
            "HISTORY   calibrated with pipeline 1.2",
            "COMMENT   first note",
            "COMMENT   second note",
            "DANGLING= 'ends with &'",
            "OBJECT  = 'M31'",
            "CONTINUE  'orphan'",
        ]);
        let parsed = parse_header_at(&bytes, 0).unwrap();
        let h = &parsed.header;

        assert_eq!(h.get("FILENAME"), Some("jw01234_very_long_middle_part_rest.fits"));
        assert_eq!(h.get_f64("HIERARCH ESO DET DIT"), Some(10.0));
        assert_eq!(h.get("HIERARCH ESO INS FILT1 NAME"), Some("Ks"));
        let commentary = |key: &str| -> Vec<&str> {
            h.cards.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
        };
        assert_eq!(commentary("HISTORY"), vec!["calibrated with pipeline 1.2"]);
        assert_eq!(commentary("COMMENT"), vec!["first note", "second note"]);
        assert_eq!(h.get("HISTORY"), None, "commentary cards never shadow a keyword lookup");
        assert_eq!(h.get("DANGLING"), Some("ends with &"));
        assert_eq!(h.get("OBJECT"), Some("M31"));
        assert!(!h.cards.iter().any(|(k, _)| k == "CONTINUE"));
        assert_eq!(parsed.next_hdu_offset, BLOCK_SIZE);
    }

    #[test]
    fn fortran_d_exponents_scale_integer_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fortran_scaled.fits");
        let data: Vec<u8> = [-32768i16, 0, 1, 32767].iter().flat_map(|v| v.to_be_bytes()).collect();
        let cards = primary_image_cards(
            "16",
            &["2", "2"],
            &[("BZERO", "3.2768D+04".into()), ("BSCALE", "1.0D+00".into())],
        );
        write_raw_hdus(&path, &[(cards, data.clone())]);

        let image = load_fits_image(path.to_str().unwrap()).unwrap();
        assert_eq!(image.iter().copied().collect::<Vec<f32>>(), vec![0.0, 32768.0, 32769.0, 65535.0]);
        let plane = extract_int_plane_by_index(&File::open(&path).unwrap(), 0).unwrap();
        assert!(!plane.signed);
        assert_eq!(plane.value_at(1, 1), 65535);

        let bad = dir.path().join("bad_scale.fits");
        let cards = primary_image_cards("16", &["2", "2"], &[("BZERO", "'abc'".into())]);
        write_raw_hdus(&bad, &[(cards, data)]);
        let err = format!("{:#}", load_fits_image(bad.to_str().unwrap()).unwrap_err());
        assert!(err.contains("BZERO = 'abc' is not a number"), "{err}");
    }

    #[test]
    fn signed_byte_dq_keeps_its_integer_plane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("int8_dq.fits");
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "2".into()),
            ("NAXIS2", "2".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("BZERO", "-128".into()),
            ("BSCALE", "1".into()),
            ("EXTNAME", "'DQ      '".into()),
        ];
        write_raw_hdus(&path, &[(empty_primary_cards(), Vec::new()), (cards, vec![0, 127, 128, 255])]);
        let plane = extract_int_plane_by_index(&File::open(&path).unwrap(), 1).unwrap();
        assert!(plane.signed);
        assert_eq!(plane.value_at(0, 0), -128);
        assert_eq!(plane.value_at(0, 1), -1);
        assert_eq!(plane.value_at(1, 0), 0);
        assert_eq!(plane.value_at(1, 1), 127);
    }

    #[test]
    fn a_context_cube_after_a_2d_science_image_is_not_read_as_rgb() {
        let dir = tempfile::tempdir().unwrap();

        for planes in [3, 4] {
            let i2d = dir.path().join(format!("i2d_con{planes}.fits"));
            write_raw_hdus(
                &i2d,
                &[
                    (empty_primary_cards(), Vec::new()),
                    plane_hdu("SCI", 4, 3),
                    plane_hdu("ERR", 4, 3),
                    cube_hdu("CON", 4, 3, planes, &[]),
                ],
            );
            let file = File::open(&i2d).unwrap();
            assert!(
                try_extract_rgb_mmap(&file).unwrap().is_none(),
                "a {planes}-plane context cube must not hide the 2D SCI image"
            );
            assert_eq!(auto_hdu_index(&file).unwrap(), 1);
            assert_eq!(extract_image_mmap(&file).unwrap().image.dim(), (3, 4));
        }

        let weight_then_cube = dir.path().join("wht_cube.fits");
        write_raw_hdus(
            &weight_then_cube,
            &[
                (empty_primary_cards(), Vec::new()),
                plane_hdu("WHT", 4, 3),
                cube_hdu("STACK", 4, 3, 3, &[]),
            ],
        );
        assert!(try_extract_rgb_mmap(&File::open(&weight_then_cube).unwrap()).unwrap().is_none());

        let marked = dir.path().join("sci_and_marked_rgb.fits");
        write_raw_hdus(
            &marked,
            &[
                (empty_primary_cards(), Vec::new()),
                plane_hdu("SCI", 4, 3),
                cube_hdu("COLOUR", 4, 3, 3, &[("CTYPE3", "'RGB     '".into())]),
            ],
        );
        let rgb = try_extract_rgb_mmap(&File::open(&marked).unwrap()).unwrap().expect("an explicit colour marker wins");
        assert_eq!(rgb.header.get("EXTNAME"), Some("COLOUR"));
    }

    #[test]
    fn auto_selection_refuses_a_mono_plane_beside_an_rgb_cube() {
        let dir = tempfile::tempdir().unwrap();

        let marked = dir.path().join("rgb_with_mask.fits");
        write_raw_hdus(
            &marked,
            &[
                (empty_primary_cards(), Vec::new()),
                cube_hdu("SCI", 4, 3, 3, &[("CTYPE3", "'RGB     '".into())]),
                plane_hdu("MASK", 4, 3),
            ],
        );
        let file = File::open(&marked).unwrap();
        assert!(try_extract_rgb_mmap(&file).unwrap().is_some());
        let msg = format!("{:#}", auto_hdu_index(&file).unwrap_err());
        assert!(msg.contains("HDU 1 (SCI) holds an RGB colour cube"), "{msg}");
        assert!(msg.contains("#hdu=2"), "{msg}");
        assert!(extract_image_mmap(&file).is_err());
        assert!(load_fits_image(marked.to_str().unwrap()).is_err());
        assert_eq!(extract_image_mmap_by_index(&file, 2).unwrap().image.dim(), (3, 4));

        let primary_cube = dir.path().join("primary_rgb_with_weight.fits");
        let data: Vec<u8> = (0..4 * 3 * 3).flat_map(|i| (i as f32).to_be_bytes()).collect();
        write_raw_hdus(
            &primary_cube,
            &[(primary_image_cards("-32", &["4", "3", "3"], &[]), data), plane_hdu("WHT", 4, 3)],
        );
        let file = File::open(&primary_cube).unwrap();
        let rgb = try_extract_rgb_mmap(&file).unwrap().expect("an unmarked primary cube is the colour image");
        assert_eq!(rgb.b[[0, 0]], 24.0);
        assert!(auto_hdu_index(&file).is_err());
    }

    #[test]
    fn the_header_of_a_colour_file_is_its_rgb_cube_header() {
        let dir = tempfile::tempdir().unwrap();

        let beside_mask = dir.path().join("rgb_beside_mask.fits");
        let mut primary = empty_primary_cards();
        primary.push(("TELESCOP", "'RC8     '".into()));
        write_raw_hdus(
            &beside_mask,
            &[
                (primary, Vec::new()),
                cube_hdu("SCI", 4, 3, 3, &[("CTYPE3", "'RGB     '".into()), ("CRVAL1", "150.0".into())]),
                plane_hdu("MASK", 4, 3),
            ],
        );
        let header = extract_header_mmap(&File::open(&beside_mask).unwrap()).unwrap();
        assert_eq!(header.get("EXTNAME"), Some("SCI"), "the mask header must not stand in for the colour image");
        assert_eq!(header.get_i64("NAXIS3"), Some(3));
        assert_eq!(header.get_f64("CRVAL1"), Some(150.0));
        assert_eq!(header.get("TELESCOP"), Some("RC8"));

        let lone_cube = dir.path().join("lone_rgb.fits");
        let data: Vec<u8> = (0..5 * 3 * 3).flat_map(|i| (i as f32).to_be_bytes()).collect();
        write_raw_hdus(&lone_cube, &[(primary_image_cards("-32", &["5", "3", "3"], &[]), data)]);
        let file = File::open(&lone_cube).unwrap();
        assert!(try_extract_rgb_mmap(&file).unwrap().is_some());
        let header = extract_header_mmap(&file).unwrap();
        assert_eq!(header.get_i64("NAXIS1"), Some(5));
        assert_eq!(header.get_i64("NAXIS3"), Some(3));
        assert!(auto_hdu_index(&file).is_err(), "a header read never makes a colour file loadable as mono");

        let i2d = dir.path().join("i2d_con.fits");
        write_raw_hdus(
            &i2d,
            &[
                (empty_primary_cards(), Vec::new()),
                plane_hdu("SCI", 4, 3),
                cube_hdu("CON", 4, 3, 3, &[]),
            ],
        );
        assert_eq!(extract_header_mmap(&File::open(&i2d).unwrap()).unwrap().get("EXTNAME"), Some("SCI"));
    }

    #[test]
    fn compressed_headers_describe_the_decompressed_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("z2d.fits");
        write_raw_hdus(&path, &[(empty_primary_cards(), Vec::new()), compressed_image_hdu(&[37, 23])]);
        let file = File::open(&path).unwrap();

        for header in [extract_header_mmap(&file).unwrap(), extract_header_by_index_merged(&file, 1).unwrap()] {
            assert_eq!(header.get_i64("NAXIS"), Some(2));
            assert_eq!(header.get_i64("NAXIS1"), Some(37));
            assert_eq!(header.get_i64("NAXIS2"), Some(23));
            assert_eq!(header.get_i64("BITPIX"), Some(16));
        }
        let raw = extract_header_by_index(&file, 1).unwrap();
        assert_eq!(raw.get_i64("NAXIS1"), Some(8), "the raw card view keeps the table storage shape");
    }

    #[test]
    fn corrupt_sizes_are_errors_instead_of_wrapped_offsets() {
        let negative = raw_header_block(&[
            "SIMPLE  =                    T",
            "BITPIX  =                   16",
            "NAXIS   =                    2",
            "NAXIS1  =                   -1",
            "NAXIS2  =                  100",
        ]);
        let err = parse_header_at(&negative, 0).err().expect("a negative axis has no data size");
        assert!(format!("{err:#}").contains("NAXIS1 = -1 is negative"), "{err:#}");

        let overflowing = raw_header_block(&[
            "SIMPLE  =                    T",
            "BITPIX  =                   16",
            "NAXIS   =                    1",
            "NAXIS1  =  9223372036854775807",
        ]);
        assert!(parse_header_at(&overflowing, 0).is_err());

        let runaway = raw_header_block(&[
            "SIMPLE  =                    T",
            "BITPIX  =                   16",
            "NAXIS   =         999999999999",
        ]);
        assert!(parse_header_at(&runaway, 0).is_err());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt_tail.fits");
        let mut bad_ext = plane_hdu("BAD", 4, 3);
        bad_ext.0.retain(|(k, _)| *k != "NAXIS1");
        bad_ext.0.push(("NAXIS1", "-1".into()));
        write_raw_hdus(
            &path,
            &[(empty_primary_cards(), Vec::new()), plane_hdu("SCI", 4, 3), bad_ext],
        );
        let file = File::open(&path).unwrap();
        assert_eq!(list_extensions(&file).unwrap().len(), 2, "the scan stops at the corrupt HDU");
        assert_eq!(scan_all_hdus(&read_file_bytes(&file).unwrap()).unwrap().len(), 2);
        assert_eq!(extract_image_mmap(&file).unwrap().image.dim(), (3, 4));
    }

    #[test]
    fn a_crafted_three_plane_header_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crafted_rgb.fits");
        let cards = primary_image_cards("16", &["1024819115206086201", "3", "3"], &[]);
        write_raw_hdus(&path, &[(cards, vec![0u8; 16])]);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 2 * BLOCK_SIZE as u64);
        let file = File::open(&path).unwrap();
        assert!(try_extract_rgb_mmap(&file).is_err());
        assert!(extract_image_mmap(&file).is_err());
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn a_gzip_compressed_fits_is_refused_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let large = dir.path().join("large.fits");
        let mut noisy = plane_hdu("SCI", 64, 64);
        let mut state: u32 = 12345;
        noisy.1 = (0..64 * 64)
            .flat_map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                state.to_be_bytes()
            })
            .collect();
        write_raw_hdus(&large, &[(empty_primary_cards(), Vec::new()), noisy]);
        let small = dir.path().join("small.fits");
        write_raw_hdus(&small, &[(empty_primary_cards(), Vec::new())]);

        let large_gz = gzip(&std::fs::read(&large).unwrap());
        let small_gz = gzip(&std::fs::read(&small).unwrap());
        assert!(large_gz.len() >= BLOCK_SIZE, "one archive must pass the one-block size check");
        assert!(small_gz.len() < BLOCK_SIZE, "one archive must be shorter than a FITS block");

        for (name, bytes) in [("large.fits.gz", large_gz), ("small.fits.gz", small_gz)] {
            let gz = dir.path().join(name);
            std::fs::write(&gz, &bytes).unwrap();
            let file = File::open(&gz).unwrap();
            let errors = [
                extract_image_mmap(&file).err(),
                extract_image_mmap_by_index(&file, 1).err(),
                extract_int_plane_by_index(&file, 1).err(),
                try_extract_rgb_mmap(&file).err(),
                extract_header_mmap(&file).err(),
                extract_header_by_index(&file, 0).err(),
                list_extensions(&file).err(),
                auto_hdu_index(&file).err(),
                read_primary_header(gz.to_str().unwrap()).err(),
                parse_header_at(&bytes, 0).err(),
            ];
            for (i, err) in errors.into_iter().enumerate() {
                let msg = format!("{:#}", err.unwrap_or_else(|| panic!("{name}: reader {i} accepted gzip bytes")));
                assert!(msg.contains("gzip-compressed FITS is not supported; decompress first"), "{name}: reader {i}: {msg}");
            }
        }

        assert_eq!(extract_image_mmap(&File::open(&large).unwrap()).unwrap().image.dim(), (64, 64));
    }

    #[test]
    fn string_keys_record_which_values_were_fits_strings() {
        let block = raw_header_block(&[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
            "PROGRAM = '01234   '           / program id",
            "VISIT   =                  001",
            "LINENUM = '01.001  '",
            "FLAG    = 'T       '",
            "LOGIC   =                    F",
            "RATIO   =            1.234D-05",
            "LONGSTR = 'first part&'",
            "CONTINUE  'second part'",
            "HIERARCH ESO DET CHIP = '12' / hierarch string",
            "HIERARCH ESO DET NX = 2048",
            "DUP     = '1       '",
            "DUP     =                    2",
            "REDO    =                    3",
            "REDO    = 'three   '",
            "HISTORY 'not a value card'",
            "COMMENT = 'still commentary'",
        ]);
        let parsed = parse_header_at(&block, 0).unwrap();
        let string_keys = parsed.header.string_keys.as_ref().expect("a header read from a file knows its value types");
        let mut quoted: Vec<&str> = string_keys.iter().map(String::as_str).collect();
        quoted.sort_unstable();
        assert_eq!(quoted, vec!["FLAG", "HIERARCH ESO DET CHIP", "LINENUM", "LONGSTR", "PROGRAM", "REDO"]);
        assert_eq!(parsed.header.get("PROGRAM"), Some("01234"));
        assert_eq!(parsed.header.get("VISIT"), Some("001"));
        assert_eq!(parsed.header.get("LONGSTR"), Some("first partsecond part"));
        assert_eq!(parsed.header.get("DUP"), Some("2"));
        assert_eq!(parsed.header.get("REDO"), Some("three"));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quoted.fits");
        std::fs::write(&path, &block).unwrap();
        let positioned = read_header_blocks(&File::open(&path).unwrap(), 0).unwrap();
        assert_eq!(positioned.header.string_keys, parsed.header.string_keys);
    }
}
