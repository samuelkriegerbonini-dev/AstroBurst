use anyhow::{Context, Result};
use ndarray::Array2;
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::types::header::HduHeader;

const FITS_BLOCK_SIZE: usize = 2880;

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

pub fn write_fits_mono(
    path: &str,
    data: &Array2<f32>,
    header: Option<&HduHeader>,
) -> Result<()> {
    let (rows, cols) = data.dim();
    let file = File::create(path).context("Failed to create FITS file")?;
    let mut writer = BufWriter::new(file);
    let mut bytes = 0;

    bytes += write_header_card(&mut writer, "SIMPLE", "T", "FITS standard")?;
    bytes += write_header_card(&mut writer, "BITPIX", "-32", "32-bit float")?;
    bytes += write_header_card(&mut writer, "NAXIS", "2", "2D image")?;
    bytes += write_header_card(&mut writer, "NAXIS1", &cols.to_string(), "width")?;
    bytes += write_header_card(&mut writer, "NAXIS2", &rows.to_string(), "height")?;
    bytes += write_header_card(&mut writer, "BZERO", "0.0", "")?;
    bytes += write_header_card(&mut writer, "BSCALE", "1.0", "")?;

    if let Some(hdr) = header {
        for card in &hdr.cards {
            let key = card.0.trim();
            if matches!(key, "SIMPLE" | "BITPIX" | "NAXIS" | "NAXIS1" | "NAXIS2" | "BZERO" | "BSCALE" | "END") {
                continue;
            }
            let card_str = format!("{:<80}", format!("{:<8}= {:>20}", key, card.1));
            writer.write_all(card_str[..80].as_bytes())?;
            bytes += 80;
        }
    }

    write_header_end(&mut writer, bytes)?;

    let slice = data.as_slice().context("Array not contiguous")?;
    for &val in slice {
        writer.write_all(&val.to_be_bytes())?;
    }

    let data_bytes = slice.len() * 4;
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
    let file = File::create(path).context("Failed to create FITS file")?;
    let mut writer = BufWriter::new(file);
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
        for card in &hdr.cards {
            let key = card.0.trim();
            if matches!(key, "SIMPLE" | "BITPIX" | "NAXIS" | "NAXIS1" | "NAXIS2" | "NAXIS3" | "BZERO" | "BSCALE" | "END") {
                continue;
            }
            let card_str = format!("{:<80}", format!("{:<8}= {:>20}", key, card.1));
            writer.write_all(card_str[..80].as_bytes())?;
            bytes += 80;
        }
    }

    write_header_end(&mut writer, bytes)?;

    for channel in [r, g, b] {
        let slice = channel.as_slice().context("Channel not contiguous")?;
        for &val in slice {
            writer.write_all(&val.to_be_bytes())?;
        }
    }

    let data_bytes = rows * cols * 3 * 4;
    pad_to_block(&mut writer, data_bytes)?;

    writer.flush()?;
    Ok(())
}
