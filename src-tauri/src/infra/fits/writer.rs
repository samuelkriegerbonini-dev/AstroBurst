use anyhow::{bail, Context, Result};
use ndarray::Array2;
use std::fs::File;
use std::io::{BufWriter, Write};

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

fn pad_to_block(writer: &mut BufWriter<File>, bytes_written: usize) -> Result<()> {
    let remainder = bytes_written % FITS_BLOCK_SIZE;
    if remainder != 0 {
        let padding = FITS_BLOCK_SIZE - remainder;
        writer.write_all(&vec![0u8; padding])?;
    }
    Ok(())
}

fn write_header_card(writer: &mut BufWriter<File>, key: &str, value: &str, comment: &str) -> Result<usize> {
    let mut card = format!("{:<8}= {:>20}", key, value);
    if !comment.is_empty() {
        card = format!("{} / {}", card, comment);
    }
    let padded = format!("{:<80}", &card[..card.len().min(80)]);
    writer.write_all(padded.as_bytes())?;
    Ok(80)
}

fn write_header_end(writer: &mut BufWriter<File>, bytes_written: usize) -> Result<usize> {
    let end_card = format!("{:<80}", "END");
    writer.write_all(end_card.as_bytes())?;
    let total = bytes_written + 80;
    pad_to_block(writer, total)?;
    Ok(total)
}

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
                let physical = ((val as f64) - bzero) / bscale;
                let clamped = physical.clamp(i16::MIN as f64, i16::MAX as f64).round() as i16;
                out.copy_from_slice(&clamped.to_be_bytes());
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

fn write_f32_array_as_be(writer: &mut BufWriter<File>, data: &Array2<f32>) -> Result<usize> {
    let slice = data.as_slice().context("Array not contiguous")?;
    write_f32_slice_as_be(writer, slice)?;
    Ok(slice.len() * 4)
}

fn compute_bzero_bscale(data: &Array2<f32>) -> (f64, f64) {
    let slice = data.as_slice().unwrap_or(&[]);
    let mut dmin = f64::INFINITY;
    let mut dmax = f64::NEG_INFINITY;
    for &v in slice {
        let v = v as f64;
        if v.is_finite() {
            if v < dmin { dmin = v; }
            if v > dmax { dmax = v; }
        }
    }
    if !dmin.is_finite() || !dmax.is_finite() || (dmax - dmin).abs() < 1e-30 {
        return (32768.0, 1.0);
    }
    let bscale = (dmax - dmin) / 65535.0;
    let bzero = dmin + bscale * 32768.0;
    (bzero, bscale)
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

fn write_extra_header_cards(
    writer: &mut BufWriter<File>,
    hdr: &HduHeader,
    skip: &[&str],
) -> Result<usize> {
    let mut bytes = 0;
    for card in &hdr.cards {
        let key = card.0.trim();
        if skip.iter().any(|&s| s == key) {
            continue;
        }
        let card_str = format!("{:<80}", format!("{:<8}= {:>20}", key, card.1));
        writer.write_all(card_str[..80].as_bytes())?;
        bytes += 80;
    }
    Ok(bytes)
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
        compute_bzero_bscale(data)
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

    if let Some(hdr) = header {
        static SKIP_MONO: &[&str] = &[
            "SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "BZERO", "BSCALE", "END",
        ];
        bytes += write_extra_header_cards(&mut writer, hdr, SKIP_MONO)?;
    }

    write_header_end(&mut writer, bytes)?;

    let data_bytes = write_array_with_bitpix(&mut writer, data, bitpix, bzero, bscale)?;
    pad_to_block(&mut writer, data_bytes)?;

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

    bytes += write_header_card(&mut writer, "SIMPLE", "T", "FITS standard")?;
    bytes += write_header_card(&mut writer, "BITPIX", "-32", "32-bit float")?;
    bytes += write_header_card(&mut writer, "NAXIS", "3", "3D RGB cube")?;
    bytes += write_header_card(&mut writer, "NAXIS1", &cols.to_string(), "width")?;
    bytes += write_header_card(&mut writer, "NAXIS2", &rows.to_string(), "height")?;
    bytes += write_header_card(&mut writer, "NAXIS3", "3", "RGB channels")?;
    bytes += write_header_card(&mut writer, "BZERO", "0.0", "")?;
    bytes += write_header_card(&mut writer, "BSCALE", "1.0", "")?;

    if let Some(hdr) = header {
        static SKIP_RGB: &[&str] = &[
            "SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "NAXIS3",
            "BZERO", "BSCALE", "END",
        ];
        bytes += write_extra_header_cards(&mut writer, hdr, SKIP_RGB)?;
    }

    write_header_end(&mut writer, bytes)?;

    let mut data_bytes = 0;
    for channel in [r, g, b] {
        data_bytes += write_f32_array_as_be(&mut writer, channel)?;
    }
    pad_to_block(&mut writer, data_bytes)?;

    writer.flush()?;
    Ok(())
}
