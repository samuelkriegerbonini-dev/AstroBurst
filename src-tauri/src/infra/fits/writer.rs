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
}
