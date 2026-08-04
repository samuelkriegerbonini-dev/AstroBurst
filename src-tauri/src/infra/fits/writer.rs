use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use ndarray::Array2;
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::infra::fits::compress::gzip::gzip2_encode;
use crate::infra::fits::compress::quantize::{quantize_tile, QuantizeResult, NULL_VALUE};
use crate::infra::fits::compress::rice::RiceParams;
use crate::infra::fits::compress::rice_encode::rice_encode;
use crate::types::header::HduHeader;

const FITS_BLOCK_SIZE: usize = 2880;

const WCS_PREFIXES: &[&str] = &[
    "CRPIX", "CRVAL", "CDELT", "CTYPE", "CUNIT", "CROTA",
    "CD1_1", "CD1_2", "CD2_1", "CD2_2",
    "PC1_1", "PC1_2", "PC2_1", "PC2_2",
    "LONPOLE", "LATPOLE", "RADESYS", "EQUINOX", "EPOCH",
    "A_ORDER", "B_ORDER", "AP_ORDER", "BP_ORDER",
    "A_", "B_", "AP_", "BP_",
    "PV1_", "PV2_",
    "WCSAXES", "WCSNAME",
];

fn is_wcs_card(key: &str) -> bool {
    WCS_PREFIXES.iter().any(|p| key.starts_with(p))
}

pub fn filter_header(header: &HduHeader, copy_wcs: bool, copy_metadata: bool) -> Option<HduHeader> {
    if !copy_wcs && !copy_metadata {
        return None;
    }
    if copy_wcs && copy_metadata {
        return Some(header.clone());
    }
    let filtered_cards: Vec<_> = header
        .cards
        .iter()
        .filter(|card| {
            let key = card.0.trim();
            if copy_wcs && !copy_metadata {
                is_wcs_card(key)
            } else {
                !is_wcs_card(key)
            }
        })
        .cloned()
        .collect();

    if filtered_cards.is_empty() {
        return None;
    }

    let mut filtered = header.clone();
    filtered.cards = filtered_cards;
    Some(filtered)
}

fn pad_to_block(writer: &mut BufWriter<File>, bytes_written: usize, fill: u8) -> Result<()> {
    let remainder = bytes_written % FITS_BLOCK_SIZE;
    if remainder != 0 {
        let padding = FITS_BLOCK_SIZE - remainder;
        writer.write_all(&vec![fill; padding])?;
    }
    Ok(())
}

fn build_value_card(key: &str, value: &str, comment: &str) -> [u8; 80] {
    let mut card = format!("{:<8}= {:>20}", key, value);
    if !comment.is_empty() {
        card = format!("{} / {}", card, comment);
    }
    let truncated = truncate_str_bytes(&card, 80);
    let mut out = [b' '; 80];
    out[..truncated.len()].copy_from_slice(truncated);
    out
}

fn write_header_card(writer: &mut BufWriter<File>, key: &str, value: &str, comment: &str) -> Result<usize> {
    writer.write_all(&build_value_card(key, value, comment))?;
    Ok(80)
}

fn write_header_end(writer: &mut BufWriter<File>, bytes_written: usize) -> Result<usize> {
    let end_card = format!("{:<80}", "END");
    writer.write_all(end_card.as_bytes())?;
    let total = bytes_written + 80;
    pad_to_block(writer, total, b' ')?;
    Ok(total)
}

const I16_BLANK: i16 = i16::MIN;

fn write_f32_slice_as_be(writer: &mut BufWriter<File>, slice: &[f32]) -> Result<()> {
    const CHUNK: usize = 16384;
    let mut be_buf = vec![0u8; CHUNK * 4];

    for chunk in slice.chunks(CHUNK) {
        let buf = &mut be_buf[..chunk.len() * 4];
        chunk
            .iter()
            .zip(buf.chunks_exact_mut(4))
            .for_each(|(&val, out)| {
                out.copy_from_slice(&val.to_be_bytes());
            });
        writer.write_all(buf)?;
    }

    Ok(())
}

fn write_i16_slice_as_be(writer: &mut BufWriter<File>, data: &[f32], bzero: f64, bscale: f64) -> Result<()> {
    const CHUNK: usize = 16384;
    let mut be_buf = vec![0u8; CHUNK * 2];

    for chunk in data.chunks(CHUNK) {
        let buf = &mut be_buf[..chunk.len() * 2];
        chunk
            .iter()
            .zip(buf.chunks_exact_mut(2))
            .for_each(|(&val, out)| {
                let raw = if val.is_finite() {
                    let physical = ((val as f64) - bzero) / bscale;
                    physical.clamp((I16_BLANK as f64) + 1.0, i16::MAX as f64).round() as i16
                } else {
                    I16_BLANK
                };
                out.copy_from_slice(&raw.to_be_bytes());
            });
        writer.write_all(buf)?;
    }

    Ok(())
}

fn write_f64_slice_as_be(writer: &mut BufWriter<File>, data: &[f32]) -> Result<()> {
    const CHUNK: usize = 16384;
    let mut be_buf = vec![0u8; CHUNK * 8];

    for chunk in data.chunks(CHUNK) {
        let buf = &mut be_buf[..chunk.len() * 8];
        chunk
            .iter()
            .zip(buf.chunks_exact_mut(8))
            .for_each(|(&val, out)| {
                out.copy_from_slice(&(val as f64).to_be_bytes());
            });
        writer.write_all(buf)?;
    }

    Ok(())
}

fn compute_bzero_bscale(data: &[f32]) -> (f64, f64) {
    compute_bzero_bscale_slices(&[data])
}

fn compute_bzero_bscale_slices(slices: &[&[f32]]) -> (f64, f64) {
    let mut dmin = f64::INFINITY;
    let mut dmax = f64::NEG_INFINITY;
    for data in slices {
        for &v in *data {
            let v = v as f64;
            if v.is_finite() {
                if v < dmin { dmin = v; }
                if v > dmax { dmax = v; }
            }
        }
    }
    if !dmin.is_finite() || !dmax.is_finite() || (dmax - dmin).abs() < 1e-30 {
        return (32768.0, 1.0);
    }
    let bscale = (dmax - dmin) / 65534.0;
    let bzero = dmin + bscale * 32767.0;
    (bzero, bscale)
}

fn compute_bzero_bscale_array(data: &Array2<f32>) -> (f64, f64) {
    let slice = data.as_slice().unwrap_or(&[]);
    compute_bzero_bscale(slice)
}

fn write_array_with_bitpix(
    writer: &mut BufWriter<File>,
    data: &Array2<f32>,
    bitpix: i32,
    bzero: f64,
    bscale: f64,
) -> Result<usize> {
    let slice = data.as_slice().context("Array not contiguous")?;
    match bitpix {
        16 => {
            write_i16_slice_as_be(writer, slice, bzero, bscale)?;
            Ok(slice.len() * 2)
        }
        -64 => {
            write_f64_slice_as_be(writer, slice)?;
            Ok(slice.len() * 8)
        }
        _ => {
            write_f32_slice_as_be(writer, slice)?;
            Ok(slice.len() * 4)
        }
    }
}

fn write_slice_with_bitpix(
    writer: &mut BufWriter<File>,
    slice: &[f32],
    bitpix: i32,
    bzero: f64,
    bscale: f64,
) -> Result<usize> {
    match bitpix {
        16 => {
            write_i16_slice_as_be(writer, slice, bzero, bscale)?;
            Ok(slice.len() * 2)
        }
        -64 => {
            write_f64_slice_as_be(writer, slice)?;
            Ok(slice.len() * 8)
        }
        _ => {
            write_f32_slice_as_be(writer, slice)?;
            Ok(slice.len() * 4)
        }
    }
}

fn truncate_str_bytes(s: &str, max: usize) -> &[u8] {
    if s.len() <= max {
        return s.as_bytes();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s.as_bytes()[..end]
}

fn is_fits_numeric(value: &str) -> bool {
    value == "T" || value == "F" || value.parse::<f64>().is_ok()
}

fn push_header_card(out: &mut Vec<u8>, key: &str, value: &str) {
    let mut card = [b' '; 80];
    let kb = key.as_bytes();
    let klen = kb.len().min(8);
    card[..klen].copy_from_slice(&kb[..klen]);

    if matches!(key, "COMMENT" | "HISTORY" | "") {
        let tb = truncate_str_bytes(value, 72);
        card[8..8 + tb.len()].copy_from_slice(tb);
        out.extend_from_slice(&card);
        return;
    }

    card[8] = b'=';
    card[9] = b' ';

    if is_fits_numeric(value) {
        let vb = truncate_str_bytes(value, 20);
        let start = 30 - vb.len();
        card[start..30].copy_from_slice(vb);
    } else {
        card[10] = b'\'';
        let vb = truncate_str_bytes(value, 67);
        card[11..11 + vb.len()].copy_from_slice(vb);
        let close = (11 + vb.len()).max(19);
        card[close] = b'\'';
    }

    out.extend_from_slice(&card);
}

fn write_extra_header_cards(
    writer: &mut BufWriter<File>,
    hdr: &HduHeader,
    skip: &[&str],
) -> Result<usize> {
    let mut buf: Vec<u8> = Vec::new();
    for card in &hdr.cards {
        let key = card.0.trim();
        if skip.iter().any(|&s| s == key) {
            continue;
        }
        push_header_card(&mut buf, key, &card.1);
    }
    writer.write_all(&buf)?;
    Ok(buf.len())
}

fn write_provenance(writer: &mut BufWriter<File>) -> Result<usize> {
    let version = env!("CARGO_PKG_VERSION");
    let mut buf = Vec::new();
    push_header_card(&mut buf, "PROGRAM", &format!("AstroBurst {}", version));
    push_header_card(&mut buf, "HISTORY", &format!("Processed with AstroBurst {}", version));
    writer.write_all(&buf)?;
    Ok(buf.len())
}

pub fn write_fits_mono(
    path: &str,
    data: &Array2<f32>,
    header: Option<&HduHeader>,
) -> Result<()> {
    write_fits_mono_bitpix(path, data, header, -32)
}

pub fn write_fits_mono_bitpix(
    path: &str,
    data: &Array2<f32>,
    header: Option<&HduHeader>,
    bitpix: i32,
) -> Result<()> {
    let (rows, cols) = data.dim();
    let file = File::create(path).context("Failed to create FITS file")?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file);
    let mut bytes = 0;

    let (bitpix_str, bitpix_comment) = match bitpix {
        16 => ("16", "16-bit signed integer"),
        -64 => ("-64", "64-bit double"),
        _ => ("-32", "32-bit float"),
    };

    let (bzero, bscale) = if bitpix == 16 {
        compute_bzero_bscale_array(data)
    } else {
        (0.0, 1.0)
    };

    bytes += write_header_card(&mut writer, "SIMPLE", "T", "FITS standard")?;
    bytes += write_header_card(&mut writer, "BITPIX", bitpix_str, bitpix_comment)?;
    bytes += write_header_card(&mut writer, "NAXIS", "2", "2D image")?;
    bytes += write_header_card(&mut writer, "NAXIS1", &cols.to_string(), "width")?;
    bytes += write_header_card(&mut writer, "NAXIS2", &rows.to_string(), "height")?;
    bytes += write_header_card(&mut writer, "BZERO", &format!("{:.10E}", bzero), "")?;
    bytes += write_header_card(&mut writer, "BSCALE", &format!("{:.10E}", bscale), "")?;
    if bitpix == 16 {
        bytes += write_header_card(&mut writer, "BLANK", &I16_BLANK.to_string(), "undefined pixel value")?;
    }

    if let Some(hdr) = header {
        static SKIP_MONO: &[&str] = &[
            "SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "BZERO", "BSCALE", "BLANK", "END",
        ];
        bytes += write_extra_header_cards(&mut writer, hdr, SKIP_MONO)?;
    }

    bytes += write_provenance(&mut writer)?;

    write_header_end(&mut writer, bytes)?;

    let data_bytes = write_array_with_bitpix(&mut writer, data, bitpix, bzero, bscale)?;
    pad_to_block(&mut writer, data_bytes, 0u8)?;

    writer.flush()?;
    Ok(())
}

pub fn write_fits_rgb(
    path: &str,
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    header: Option<&HduHeader>,
) -> Result<()> {
    write_fits_rgb_bitpix(path, r, g, b, header, -32)
}

pub fn write_fits_rgb_bitpix(
    path: &str,
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    header: Option<&HduHeader>,
    bitpix: i32,
) -> Result<()> {
    let (rows, cols) = r.dim();
    if g.dim() != (rows, cols) || b.dim() != (rows, cols) {
        bail!(
            "RGB channel dimension mismatch: R={}x{}, G={}x{}, B={}x{}",
            cols, rows, g.dim().1, g.dim().0, b.dim().1, b.dim().0
        );
    }

    let file = File::create(path).context("Failed to create FITS file")?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file);
    let mut bytes = 0;

    let (bitpix_str, bitpix_comment) = match bitpix {
        16 => ("16", "16-bit signed integer"),
        -64 => ("-64", "64-bit double"),
        _ => ("-32", "32-bit float"),
    };

    let (bzero, bscale) = if bitpix == 16 {
        let r_sl = r.as_slice().unwrap_or(&[]);
        let g_sl = g.as_slice().unwrap_or(&[]);
        let b_sl = b.as_slice().unwrap_or(&[]);
        compute_bzero_bscale_slices(&[r_sl, g_sl, b_sl])
    } else {
        (0.0, 1.0)
    };

    bytes += write_header_card(&mut writer, "SIMPLE", "T", "FITS standard")?;
    bytes += write_header_card(&mut writer, "BITPIX", bitpix_str, bitpix_comment)?;
    bytes += write_header_card(&mut writer, "NAXIS", "3", "3D RGB cube")?;
    bytes += write_header_card(&mut writer, "NAXIS1", &cols.to_string(), "width")?;
    bytes += write_header_card(&mut writer, "NAXIS2", &rows.to_string(), "height")?;
    bytes += write_header_card(&mut writer, "NAXIS3", "3", "RGB channels")?;
    bytes += write_header_card(&mut writer, "BZERO", &format!("{:.10E}", bzero), "")?;
    bytes += write_header_card(&mut writer, "BSCALE", &format!("{:.10E}", bscale), "")?;
    if bitpix == 16 {
        bytes += write_header_card(&mut writer, "BLANK", &I16_BLANK.to_string(), "undefined pixel value")?;
    }

    if let Some(hdr) = header {
        static SKIP_RGB: &[&str] = &[
            "SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "NAXIS3",
            "BZERO", "BSCALE", "BLANK", "END",
        ];
        bytes += write_extra_header_cards(&mut writer, hdr, SKIP_RGB)?;
    }

    bytes += write_provenance(&mut writer)?;

    write_header_end(&mut writer, bytes)?;

    let mut data_bytes = 0;
    for channel in [r, g, b] {
        let sl = channel.as_slice().context("Channel not contiguous")?;
        data_bytes += write_slice_with_bitpix(&mut writer, sl, bitpix, bzero, bscale)?;
    }
    pad_to_block(&mut writer, data_bytes, 0u8)?;

    writer.flush()?;
    Ok(())
}



const RICE_BLOCKSIZE: u32 = 32;
const RICE_DITHER_SEED: i32 = 1;

pub(crate) fn write_primary_hdu_stub(
    writer: &mut BufWriter<File>,
    extra_header: Option<&HduHeader>,
) -> Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    push_header_card(&mut buf, "SIMPLE", "T");
    push_header_card(&mut buf, "BITPIX", "8");
    push_header_card(&mut buf, "NAXIS", "0");
    push_header_card(&mut buf, "EXTEND", "T");
    if let Some(hdr) = extra_header {
        for card in &hdr.cards {
            let key = card.0.trim();
            if matches!(key, "SIMPLE" | "BITPIX" | "EXTEND" | "END" | "PCOUNT" | "GCOUNT")
                || key.starts_with("NAXIS")
            {
                continue;
            }
            push_header_card(&mut buf, key, &card.1);
        }
    }
    writer.write_all(&buf)?;
    write_header_end(writer, buf.len())?;
    Ok(())
}

struct EncodedRow {
    compressed: Vec<u8>,
    gzip: Vec<u8>,
    zscale: f64,
    zzero: f64,
}

fn encode_int16_row(row: &[f32], bzero: f64, bscale: f64) -> EncodedRow {
    let ints: Vec<i64> = row
        .iter()
        .map(|&val| {
            if val.is_finite() {
                let physical = (val as f64 - bzero) / bscale;
                physical.clamp(I16_BLANK as f64 + 1.0, i16::MAX as f64).round() as i64
            } else {
                I16_BLANK as i64
            }
        })
        .collect();
    let params = RiceParams { blocksize: RICE_BLOCKSIZE, bytepix: 2, signed: true };
    EncodedRow { compressed: rice_encode(&ints, &params), gzip: Vec::new(), zscale: 0.0, zzero: 0.0 }
}

fn gzip_raw_row(row: &[f32]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(row.len() * 4);
    for &v in row {
        raw.extend_from_slice(&v.to_be_bytes());
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&raw).expect("gzip encode into a Vec<u8> sink cannot fail");
    enc.finish().expect("gzip finish into a Vec<u8> sink cannot fail")
}

fn encode_lossless_int_row(raw: &[i64], bytepix: u32) -> EncodedRow {
    let signed = bytepix != 1;
    let params = RiceParams { blocksize: RICE_BLOCKSIZE, bytepix, signed };
    EncodedRow { compressed: rice_encode(raw, &params), gzip: Vec::new(), zscale: 0.0, zzero: 0.0 }
}

fn encode_quantized_row(row: &[f32], quantize_level: f64, tile_index: usize) -> (EncodedRow, bool) {
    match quantize_tile(row, quantize_level, RICE_DITHER_SEED, tile_index) {
        QuantizeResult::Ints { values, quant, has_null } => {
            let params = RiceParams { blocksize: RICE_BLOCKSIZE, bytepix: 4, signed: true };
            (
                EncodedRow {
                    compressed: rice_encode(&values, &params),
                    gzip: Vec::new(),
                    zscale: quant.scale,
                    zzero: quant.zero,
                },
                has_null,
            )
        }
        QuantizeResult::Overflow => (
            EncodedRow { compressed: Vec::new(), gzip: gzip_raw_row(row), zscale: 0.0, zzero: 0.0 },
            false,
        ),
    }
}

fn is_reserved_bintable_key(key: &str) -> bool {
    if matches!(
        key,
        "XTENSION" | "BITPIX" | "NAXIS" | "NAXIS1" | "NAXIS2" | "PCOUNT" | "GCOUNT"
            | "TFIELDS" | "BZERO" | "BSCALE" | "BLANK" | "THEAP" | "END"
    ) || key.starts_with("TTYPE")
        || key.starts_with("TFORM")
    {
        return true;
    }
    if matches!(
        key,
        "ZIMAGE" | "ZCMPTYPE" | "ZBITPIX" | "ZNAXIS" | "ZQUANTIZ" | "ZDITHER0"
            | "ZMASKCMP" | "ZSIMPLE" | "ZTENSION" | "ZEXTEND" | "ZBLOCKED"
            | "ZPCOUNT" | "ZGCOUNT" | "ZHECKSUM" | "ZDATASUM"
    ) {
        return true;
    }
    for prefix in ["ZNAXIS", "ZTILE", "ZNAME", "ZVAL"] {
        if let Some(rest) = key.strip_prefix(prefix) {
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn write_compressed_bintable(
    writer: &mut BufWriter<File>,
    ncols: usize,
    nrows_per_plane: usize,
    nplanes: usize,
    zbitpix: i32,
    quantized: bool,
    has_null: bool,
    blank: Option<i64>,
    bzero: f64,
    bscale: f64,
    header: Option<&HduHeader>,
    zcmptype: &str,
    rows: &[EncodedRow],
) -> Result<()> {
    let tfields: usize = if quantized { 4 } else { 1 };
    let row_width: usize = if quantized { 32 } else { 8 };
    let n_bintable_rows = rows.len();
    debug_assert_eq!(n_bintable_rows, nrows_per_plane * nplanes);

    let heap_len: usize = rows.iter().map(|r| r.compressed.len() + r.gzip.len()).sum();

    let mut buf: Vec<u8> = Vec::new();
    push_header_card(&mut buf, "XTENSION", "BINTABLE");
    push_header_card(&mut buf, "BITPIX", "8");
    push_header_card(&mut buf, "NAXIS", "2");
    push_header_card(&mut buf, "NAXIS1", &row_width.to_string());
    push_header_card(&mut buf, "NAXIS2", &n_bintable_rows.to_string());
    push_header_card(&mut buf, "PCOUNT", &heap_len.to_string());
    push_header_card(&mut buf, "GCOUNT", "1");
    push_header_card(&mut buf, "TFIELDS", &tfields.to_string());

    push_header_card(&mut buf, "TTYPE1", "COMPRESSED_DATA");
    push_header_card(&mut buf, "TFORM1", "1PB");
    if quantized {
        push_header_card(&mut buf, "TTYPE2", "GZIP_COMPRESSED_DATA");
        push_header_card(&mut buf, "TFORM2", "1PB");
        push_header_card(&mut buf, "TTYPE3", "ZSCALE");
        push_header_card(&mut buf, "TFORM3", "1D");
        push_header_card(&mut buf, "TTYPE4", "ZZERO");
        push_header_card(&mut buf, "TFORM4", "1D");
    }

    push_header_card(&mut buf, "ZIMAGE", "T");
    push_header_card(&mut buf, "ZCMPTYPE", zcmptype);
    push_header_card(&mut buf, "ZBITPIX", &zbitpix.to_string());
    push_header_card(&mut buf, "ZNAXIS", if nplanes > 1 { "3" } else { "2" });
    push_header_card(&mut buf, "ZNAXIS1", &ncols.to_string());
    push_header_card(&mut buf, "ZNAXIS2", &nrows_per_plane.to_string());
    if nplanes > 1 {
        push_header_card(&mut buf, "ZNAXIS3", &nplanes.to_string());
    }
    push_header_card(&mut buf, "ZTILE1", &ncols.to_string());
    push_header_card(&mut buf, "ZTILE2", "1");
    if nplanes > 1 {
        push_header_card(&mut buf, "ZTILE3", "1");
    }
    if zcmptype == "RICE_1" {
        push_header_card(&mut buf, "ZNAME1", "BLOCKSIZE");
        push_header_card(&mut buf, "ZVAL1", &RICE_BLOCKSIZE.to_string());
        push_header_card(&mut buf, "ZNAME2", "BYTEPIX");
        let bytepix = (zbitpix.unsigned_abs() / 8).max(1);
        push_header_card(&mut buf, "ZVAL2", &bytepix.to_string());
    }
    if quantized {
        push_header_card(&mut buf, "ZQUANTIZ", "SUBTRACTIVE_DITHER_1");
        push_header_card(&mut buf, "ZDITHER0", &RICE_DITHER_SEED.to_string());
        if has_null {
            push_header_card(&mut buf, "ZBLANK", &NULL_VALUE.to_string());
        }
    } else if let Some(blank_value) = blank {
        push_header_card(&mut buf, "BLANK", &blank_value.to_string());
    }
    push_header_card(&mut buf, "BZERO", &format!("{:.10E}", bzero));
    push_header_card(&mut buf, "BSCALE", &format!("{:.10E}", bscale));
    push_header_card(&mut buf, "THEAP", &(row_width * n_bintable_rows).to_string());

    if let Some(hdr) = header {
        for card in &hdr.cards {
            let key = card.0.trim();
            if !is_reserved_bintable_key(key) {
                push_header_card(&mut buf, key, &card.1);
            }
        }
    }

    let version = env!("CARGO_PKG_VERSION");
    push_header_card(&mut buf, "PROGRAM", &format!("AstroBurst {}", version));
    push_header_card(&mut buf, "HISTORY", &format!("Processed with AstroBurst {}", version));

    writer.write_all(&buf)?;
    write_header_end(writer, buf.len())?;

    let mut table_buf = Vec::with_capacity(row_width * n_bintable_rows);
    let mut heap_buf = Vec::with_capacity(heap_len);
    let mut offset: i64 = 0;
    let push_desc = |table_buf: &mut Vec<u8>, len: usize, offset: i64| -> Result<i64> {
        let len_i32 = i32::try_from(len)
            .map_err(|_| anyhow::anyhow!("Compressed tile exceeds 2 GiB descriptor limit"))?;
        let off_i32 = i32::try_from(offset)
            .map_err(|_| anyhow::anyhow!("Compressed heap exceeds the 2 GiB FITS descriptor limit; export without compression"))?;
        table_buf.extend_from_slice(&len_i32.to_be_bytes());
        table_buf.extend_from_slice(&off_i32.to_be_bytes());
        Ok(offset + len as i64)
    };
    for row in rows {
        if !row.compressed.is_empty() {
            offset = push_desc(&mut table_buf, row.compressed.len(), offset)?;
            heap_buf.extend_from_slice(&row.compressed);
        } else {
            table_buf.extend_from_slice(&0i32.to_be_bytes());
            table_buf.extend_from_slice(&0i32.to_be_bytes());
        }
        if quantized {
            if !row.gzip.is_empty() {
                offset = push_desc(&mut table_buf, row.gzip.len(), offset)?;
                heap_buf.extend_from_slice(&row.gzip);
            } else {
                table_buf.extend_from_slice(&0i32.to_be_bytes());
                table_buf.extend_from_slice(&0i32.to_be_bytes());
            }
            table_buf.extend_from_slice(&row.zscale.to_be_bytes());
            table_buf.extend_from_slice(&row.zzero.to_be_bytes());
        }
    }

    writer.write_all(&table_buf)?;
    writer.write_all(&heap_buf)?;
    let data_bytes = table_buf.len() + heap_buf.len();
    pad_to_block(writer, data_bytes, 0u8)?;

    Ok(())
}

pub fn write_fits_mono_rice(
    path: &str,
    data: &Array2<f32>,
    header: Option<&HduHeader>,
    bitpix: i32,
    quantize_level: f64,
) -> Result<()> {
    if bitpix != 16 && bitpix != -32 {
        bail!("RICE_1 compression only supports bitpix 16 or -32 (got {bitpix})");
    }
    let (nrows, ncols) = data.dim();
    let slice = data.as_slice().context("Array not contiguous")?;
    let quantized = bitpix == -32;

    let (bzero, bscale) =
        if quantized { (0.0, 1.0) } else { compute_bzero_bscale_array(data) };

    let mut encoded_rows = Vec::with_capacity(nrows);
    let mut has_null = false;
    for r in 0..nrows {
        let row = &slice[r * ncols..(r + 1) * ncols];
        if quantized {
            let (enc, null) = encode_quantized_row(row, quantize_level, r);
            has_null |= null;
            encoded_rows.push(enc);
        } else {
            encoded_rows.push(encode_int16_row(row, bzero, bscale));
        }
    }

    let file = File::create(path).context("Failed to create FITS file")?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file);
    write_primary_hdu_stub(&mut writer, None)?;
    write_compressed_bintable(
        &mut writer, ncols, nrows, 1, bitpix, quantized, has_null, Some(I16_BLANK as i64),
        bzero, bscale, header, "RICE_1", &encoded_rows,
    )?;
    writer.flush()?;
    Ok(())
}

pub fn write_fits_rgb_rice(
    path: &str,
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    header: Option<&HduHeader>,
    bitpix: i32,
    quantize_level: f64,
) -> Result<()> {
    if bitpix != 16 && bitpix != -32 {
        bail!("RICE_1 compression only supports bitpix 16 or -32 (got {bitpix})");
    }
    let (nrows, ncols) = r.dim();
    if g.dim() != (nrows, ncols) || b.dim() != (nrows, ncols) {
        bail!(
            "RGB channel dimension mismatch: R={}x{}, G={}x{}, B={}x{}",
            ncols, nrows, g.dim().1, g.dim().0, b.dim().1, b.dim().0
        );
    }
    let quantized = bitpix == -32;

    let (bzero, bscale) = if quantized {
        (0.0, 1.0)
    } else {
        let r_sl = r.as_slice().unwrap_or(&[]);
        let g_sl = g.as_slice().unwrap_or(&[]);
        let b_sl = b.as_slice().unwrap_or(&[]);
        let mut combined = Vec::with_capacity(r_sl.len() + g_sl.len() + b_sl.len());
        combined.extend_from_slice(r_sl);
        combined.extend_from_slice(g_sl);
        combined.extend_from_slice(b_sl);
        compute_bzero_bscale(&combined)
    };

    let mut encoded_rows = Vec::with_capacity(nrows * 3);
    let mut has_null = false;
    let mut tile_index = 0usize;
    for channel in [r, g, b] {
        let slice = channel.as_slice().context("Channel not contiguous")?;
        for row_i in 0..nrows {
            let row = &slice[row_i * ncols..(row_i + 1) * ncols];
            if quantized {
                let (enc, null) = encode_quantized_row(row, quantize_level, tile_index);
                has_null |= null;
                encoded_rows.push(enc);
            } else {
                encoded_rows.push(encode_int16_row(row, bzero, bscale));
            }
            tile_index += 1;
        }
    }

    let file = File::create(path).context("Failed to create FITS file")?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file);
    write_primary_hdu_stub(&mut writer, None)?;
    write_compressed_bintable(
        &mut writer, ncols, nrows, 3, bitpix, quantized, has_null, Some(I16_BLANK as i64),
        bzero, bscale, header, "RICE_1", &encoded_rows,
    )?;
    writer.flush()?;
    Ok(())
}

pub(crate) fn write_planes_quantized(
    writer: &mut BufWriter<File>,
    planes: &[Array2<f32>],
    header: Option<&HduHeader>,
    quantize_level: f64,
) -> Result<()> {
    let (nrows, ncols) = planes.first().context("at least one plane required")?.dim();
    for p in &planes[1..] {
        if p.dim() != (nrows, ncols) {
            bail!("all planes must share the same dimensions ({ncols}x{nrows})");
        }
    }

    let slices: Vec<&[f32]> = planes
        .iter()
        .map(|p| p.as_slice().context("plane not contiguous"))
        .collect::<Result<_>>()?;

    let encoded: Vec<(EncodedRow, bool)> = (0..slices.len() * nrows)
        .into_par_iter()
        .map(|tile_index| {
            let slice = slices[tile_index / nrows];
            let row_i = tile_index % nrows;
            let row = &slice[row_i * ncols..(row_i + 1) * ncols];
            encode_quantized_row(row, quantize_level, tile_index)
        })
        .collect();
    let has_null = encoded.iter().any(|(_, null)| *null);
    let encoded_rows: Vec<EncodedRow> = encoded.into_iter().map(|(enc, _)| enc).collect();

    write_compressed_bintable(
        writer, ncols, nrows, planes.len(), -32, true, has_null, None, 0.0, 1.0, header,
        "RICE_1", &encoded_rows,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_planes_lossless_int(
    writer: &mut BufWriter<File>,
    raw_planes: &[Vec<i64>],
    ncols: usize,
    nrows: usize,
    zbitpix: i32,
    blank: Option<i64>,
    bzero: f64,
    bscale: f64,
    header: Option<&HduHeader>,
) -> Result<()> {
    let bytepix = match zbitpix {
        8 => 1,
        16 => 2,
        32 => 4,
        other => bail!("write_planes_lossless_int: unsupported ZBITPIX {other}"),
    };

    for plane in raw_planes {
        if plane.len() != nrows * ncols {
            bail!(
                "plane length {} does not match {ncols}x{nrows}",
                plane.len()
            );
        }
    }

    let encoded_rows: Vec<EncodedRow> = (0..raw_planes.len() * nrows)
        .into_par_iter()
        .map(|idx| {
            let plane = &raw_planes[idx / nrows];
            let row_i = idx % nrows;
            let row = &plane[row_i * ncols..(row_i + 1) * ncols];
            encode_lossless_int_row(row, bytepix)
        })
        .collect();

    write_compressed_bintable(
        writer, ncols, nrows, raw_planes.len(), zbitpix, false, false, blank, bzero, bscale,
        header, "RICE_1", &encoded_rows,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_planes_gzip2_lossless(
    writer: &mut BufWriter<File>,
    raw_plane_bytes: &[&[u8]],
    ncols: usize,
    nrows: usize,
    zbitpix: i32,
    bzero: f64,
    bscale: f64,
    header: Option<&HduHeader>,
) -> Result<()> {
    let bytepix = match zbitpix {
        -32 => 4usize,
        -64 => 8usize,
        other => bail!("write_planes_gzip2_lossless: unsupported ZBITPIX {other} (need -32/-64)"),
    };
    let row_bytes = ncols * bytepix;
    let plane_bytes = row_bytes * nrows;

    for plane in raw_plane_bytes {
        if plane.len() != plane_bytes {
            bail!(
                "plane byte length {} does not match {ncols}x{nrows}x{bytepix}",
                plane.len()
            );
        }
    }

    let encoded_rows: Vec<EncodedRow> = (0..raw_plane_bytes.len() * nrows)
        .into_par_iter()
        .map(|idx| {
            let plane = raw_plane_bytes[idx / nrows];
            let row_i = idx % nrows;
            let row = &plane[row_i * row_bytes..(row_i + 1) * row_bytes];
            EncodedRow {
                compressed: gzip2_encode(row, bytepix),
                gzip: Vec::new(),
                zscale: 0.0,
                zzero: 0.0,
            }
        })
        .collect();

    write_compressed_bintable(
        writer, ncols, nrows, raw_plane_bytes.len(), zbitpix, false, false, None, bzero, bscale,
        header, "GZIP_2", &encoded_rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::reader::parse_header_at;
    use std::collections::HashMap;

    fn mk_header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut index = HashMap::new();
        let mut cards = Vec::new();
        for (k, v) in pairs {
            index.insert(k.to_string(), v.to_string());
            cards.push((k.to_string(), v.to_string()));
        }
        HduHeader { cards, index }
    }

    #[test]
    fn header_string_and_numeric_roundtrip() {
        let path = std::env::temp_dir().join("ab_writer_roundtrip.fits");
        let p = path.to_str().unwrap();
        let data = Array2::<f32>::zeros((4, 4));
        let hdr = mk_header(&[
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
            ("RADESYS", "ICRS"),
            ("OBJECT", "M16"),
            ("CRVAL1", "202.4695"),
            ("CRPIX1", "512"),
        ]);
        write_fits_mono(p, &data, Some(&hdr)).unwrap();
        let bytes = std::fs::read(p).unwrap();
        let parsed = parse_header_at(&bytes, 0).unwrap();
        assert_eq!(parsed.header.get("CTYPE1"), Some("RA---TAN"));
        assert_eq!(parsed.header.get("CTYPE2"), Some("DEC--TAN"));
        assert_eq!(parsed.header.get("RADESYS"), Some("ICRS"));
        assert_eq!(parsed.header.get("OBJECT"), Some("M16"));
        assert_eq!(parsed.header.get_f64("CRVAL1"), Some(202.4695));
        assert_eq!(parsed.header.get_i64("CRPIX1"), Some(512));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn provenance_stamp_is_written() {
        let path = std::env::temp_dir().join("ab_writer_provenance.fits");
        let p = path.to_str().unwrap();
        let data = Array2::<f32>::zeros((4, 4));
        write_fits_mono(p, &data, None).unwrap();
        let bytes = std::fs::read(p).unwrap();
        let parsed = parse_header_at(&bytes, 0).unwrap();
        assert!(parsed.header.get("PROGRAM").unwrap().starts_with("AstroBurst"));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn header_card_bytes_are_valid_fits() {
        let mut buf = Vec::new();
        push_header_card(&mut buf, "CTYPE1", "RA---TAN");
        assert_eq!(buf.len(), 80);
        assert_eq!(&buf[0..6], b"CTYPE1");
        assert_eq!(buf[8], b'=');
        assert_eq!(buf[10], b'\'');
        let s = std::str::from_utf8(&buf).unwrap();
        assert!(s.contains("'RA---TAN'"));

        let mut num = Vec::new();
        push_header_card(&mut num, "CRVAL1", "202.4695");
        assert_eq!(num[8], b'=');
        assert_ne!(num[10], b'\'');
        assert_eq!(&num[22..30], b"202.4695");
    }

    #[test]
    fn header_card_no_panic_on_long_and_multibyte() {
        let mut buf = Vec::new();
        push_header_card(
            &mut buf,
            "OBJECT",
            "\u{00b5} Cephei \u{00c5}\u{00c5}\u{00c5}\u{00c5} a long value beyond eighty bytes \u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5} extra tail",
        );
        assert_eq!(buf.len(), 80);
    }

    #[test]
    fn structural_card_no_panic_on_long_multibyte_comment() {
        let card = build_value_card(
            "BITPIX",
            "16",
            "comment \u{00c5}\u{00c5}\u{00c5} far beyond eighty bytes \u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5}\u{00c5} tail tail tail tail",
        );
        assert_eq!(card.len(), 80);
    }

    use crate::infra::fits::compress::{decode_compressed_image, decode_compressed_planes};

    fn smooth_test_image(nrows: usize, ncols: usize) -> Array2<f32> {
        Array2::from_shape_fn((nrows, ncols), |(y, x)| {
            let i = (y * ncols + x) as f32;
            100.0 + 10.0 * (i * 0.05).sin()
        })
    }

    fn read_back_mono(path: &str) -> (HduHeader, Array2<f32>) {
        let bytes = std::fs::read(path).unwrap();
        let primary = parse_header_at(&bytes, 0).unwrap();
        assert_eq!(
            primary.header.get("SIMPLE").map(|v| v.trim()),
            Some("T"),
            "must start with a primary HDU"
        );
        let parsed = parse_header_at(&bytes, primary.next_hdu_offset).unwrap();
        let image = decode_compressed_image(&bytes, &parsed.header, parsed.data_start).unwrap();
        (parsed.header, image)
    }

    #[test]
    fn rice_mono_int16_roundtrip() {
        let path = std::env::temp_dir().join("ab_rice_mono_i16.fits");
        let p = path.to_str().unwrap();
        let data = smooth_test_image(16, 40);

        write_fits_mono_rice(p, &data, None, 16, 16.0).unwrap();
        let (header, decoded) = read_back_mono(p);

        assert_eq!(header.get("ZCMPTYPE"), Some("RICE_1"));
        assert_eq!(header.get("XTENSION"), Some("BINTABLE"));
        assert_eq!(header.get_i64("ZBITPIX"), Some(16));
        assert_eq!(decoded.dim(), data.dim());

        // int16 quantization step is BSCALE; allow a couple of steps of
        // slack for rounding at the clamp boundaries.
        let bscale = header.get_f64("BSCALE").unwrap();
        for (orig, dec) in data.iter().zip(decoded.iter()) {
            assert!(
                (orig - dec).abs() <= (bscale as f32) * 1.5 + 1e-4,
                "orig={orig} dec={dec} bscale={bscale}"
            );
        }
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn rice_mono_quantized_float_roundtrip() {
        let path = std::env::temp_dir().join("ab_rice_mono_f32.fits");
        let p = path.to_str().unwrap();
        let data = smooth_test_image(16, 40);

        write_fits_mono_rice(p, &data, None, -32, 16.0).unwrap();
        let (header, decoded) = read_back_mono(p);

        assert_eq!(header.get("ZCMPTYPE"), Some("RICE_1"));
        assert_eq!(header.get("ZQUANTIZ"), Some("SUBTRACTIVE_DITHER_1"));
        assert_eq!(header.get_i64("ZDITHER0"), Some(1));
        assert_eq!(decoded.dim(), data.dim());

        for (orig, dec) in data.iter().zip(decoded.iter()) {
            assert!((orig - dec).abs() < 2.0, "orig={orig} dec={dec}");
        }
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn rice_mono_nan_roundtrip_both_bitpix() {
        for bitpix in [16i32, -32] {
            let path = std::env::temp_dir()
                .join(format!("ab_rice_mono_nan_{}.fits", bitpix.abs()));
            let p = path.to_str().unwrap();
            let mut data = smooth_test_image(12, 32);
            data[[3, 5]] = f32::NAN;
            data[[10, 20]] = f32::NAN;

            write_fits_mono_rice(p, &data, None, bitpix, 16.0).unwrap();
            let (header, decoded) = read_back_mono(p);

            if bitpix == -32 {
                assert_eq!(header.get_i64("ZBLANK"), Some(NULL_VALUE));
            }
            assert!(decoded[[3, 5]].is_nan(), "bitpix={bitpix}");
            assert!(decoded[[10, 20]].is_nan(), "bitpix={bitpix}");
            let finite_count = decoded.iter().filter(|v| v.is_finite()).count();
            assert_eq!(finite_count, 12 * 32 - 2, "bitpix={bitpix}");
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn rice_mono_gzip_fallback_for_outlier_tile() {
        // One extreme outlier pixel blows up (max-min)/delta far past i32
        // range while the MAD-based noise estimate (robust to a single
        // spike) stays small -- forces `quantize_tile` to return
        // `Overflow`, exercising the GZIP_COMPRESSED_DATA fallback column.
        let path = std::env::temp_dir().join("ab_rice_mono_gzip_fallback.fits");
        let p = path.to_str().unwrap();
        let mut data = smooth_test_image(10, 40);
        data[[4, 20]] = 1.0e15;

        write_fits_mono_rice(p, &data, None, -32, 16.0).unwrap();
        let (header, decoded) = read_back_mono(p);
        assert_eq!(header.get("ZQUANTIZ"), Some("SUBTRACTIVE_DITHER_1"));

        // The fallback row is stored losslessly (raw gzip'd f32 bytes), so
        // it should reconstruct exactly, including the outlier itself.
        for (orig, dec) in data.row(4).iter().zip(decoded.row(4).iter()) {
            assert_eq!(orig, dec, "fallback row should round-trip exactly");
        }
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn rice_rgb_cube_roundtrip() {
        let path = std::env::temp_dir().join("ab_rice_rgb.fits");
        let p = path.to_str().unwrap();
        let r = smooth_test_image(10, 24);
        let g = r.mapv(|v| v * 0.5);
        let b = r.mapv(|v| v + 5.0);

        write_fits_rgb_rice(p, &r, &g, &b, None, -32, 16.0).unwrap();

        let bytes = std::fs::read(p).unwrap();
        let primary = parse_header_at(&bytes, 0).unwrap();
        let parsed = parse_header_at(&bytes, primary.next_hdu_offset).unwrap();
        assert_eq!(parsed.header.get_i64("ZNAXIS"), Some(3));
        assert_eq!(parsed.header.get_i64("ZNAXIS3"), Some(3));

        let planes = decode_compressed_planes(&bytes, &parsed.header, parsed.data_start).unwrap();
        assert_eq!(planes.len(), 3);
        for (plane, orig) in planes.iter().zip([&r, &g, &b]) {
            assert_eq!(plane.dim(), orig.dim());
            for (o, d) in orig.iter().zip(plane.iter()) {
                assert!((o - d).abs() < 2.0, "orig={o} dec={d}");
            }
        }
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn rice_rejects_unsupported_bitpix() {
        let data = Array2::<f32>::zeros((10, 10));
        let path = std::env::temp_dir().join("ab_rice_bad_bitpix.fits");
        let p = path.to_str().unwrap();
        assert!(write_fits_mono_rice(p, &data, None, -64, 16.0).is_err());
    }
}
