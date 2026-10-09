// FITS tile-compression support (Rice/GZIP) — contributed by Jae-Joon Lee <https://github.com/leejjoon>
//! FITS Tiled Image Compression Convention: recognising a compressed image HDU
//! and decoding it.
//!
//! Reference: https://fits.gsfc.nasa.gov/registry/tilecompression.html
//!
//! Decoding is done by `fitsio-pure`, a pure-Rust port of cfitsio's decoders:
//! RICE_1, GZIP_1, GZIP_2, HCOMPRESS_1, PLIO_1 and NOCOMPRESS, with any tile
//! shape, `SUBTRACTIVE_DITHER_1`/`_2` and `NO_DITHER` requantization from
//! `ZSCALE`/`ZZERO` columns or keywords, and `BLANK`/`ZBLANK` nulls as NaN.

use anyhow::{bail, Context, Result};
use fitsio_pure::header::Card;
use fitsio_pure::value::Value;
use ndarray::Array2;

use crate::types::header::parse_fits_float;
use crate::types::HduHeader;

pub fn is_compressed_image_hdu(header: &HduHeader) -> bool {
    let is_bintable = header
        .get("XTENSION")
        .map(|x| x.trim().eq_ignore_ascii_case("BINTABLE"))
        .unwrap_or(false);
    let is_zimage = header
        .get("ZIMAGE")
        .map(|v| v.trim().eq_ignore_ascii_case("T"))
        .unwrap_or(false);
    is_bintable && is_zimage
}

/// Decompressed-image shape/type as advertised by the Z-prefixed keywords,
/// used by the caller (reader.rs) to populate HduInfo without decoding.
pub struct CompressedImageShape {
    pub znaxis: i64,
    pub znaxis1: i64,
    pub znaxis2: i64,
    pub znaxis3: i64,
    pub zbitpix: i64,
}

pub fn read_compressed_shape(header: &HduHeader) -> CompressedImageShape {
    CompressedImageShape {
        znaxis: header.get_i64("ZNAXIS").unwrap_or(0),
        znaxis1: header.get_i64("ZNAXIS1").unwrap_or(0),
        znaxis2: header.get_i64("ZNAXIS2").unwrap_or(0),
        znaxis3: header.get_i64("ZNAXIS3").unwrap_or(0),
        zbitpix: header.get_i64("ZBITPIX").unwrap_or(0),
    }
}

fn fitsio_pure_cards(header: &HduHeader) -> Vec<Card> {
    header
        .cards
        .iter()
        .filter_map(|(key, raw)| {
            let v = raw.trim();
            let value = if header.is_string_value(key) == Some(true) {
                Value::String(raw.clone())
            } else if let Ok(n) = v.parse::<i64>() {
                Value::Integer(n)
            } else if v == "T" || v == "F" {
                Value::Logical(v == "T")
            } else if let Some(f) = parse_fits_float(v) {
                Value::Float(f)
            } else {
                Value::String(raw.clone())
            };
            Card::new(key, value).ok()
        })
        .collect()
}

#[cfg(test)]
pub fn decode_compressed_image(
    mmap: &[u8],
    header: &HduHeader,
    data_start: usize,
) -> Result<Array2<f32>> {
    let mut planes = decode_compressed_planes(mmap, header, data_start)?;
    if planes.len() != 1 {
        bail!(
            "decode_compressed_image expects a single-plane (ZNAXIS=2) compressed \
             image, found {} planes -- use decode_compressed_planes for cubes",
            planes.len()
        );
    }
    Ok(planes.remove(0))
}

pub fn decode_compressed_planes(
    mmap: &[u8],
    header: &HduHeader,
    data_start: usize,
) -> Result<Vec<Array2<f32>>> {
    let hdu = fitsio_pure::hdu::Hdu::from_cards(fitsio_pure_cards(header), data_start, data_start)
        .map_err(|e| anyhow::anyhow!("compressed image header: {e}"))?;
    let fitsio_pure::hdu::HduInfo::CompressedImage { znaxes, .. } = &hdu.info else {
        bail!("HDU at byte {data_start} is not a tile-compressed image");
    };
    let (w, h) = (znaxes[0], znaxes.get(1).copied().unwrap_or(1));
    let values = fitsio_pure::image::read_image_physical_f32(mmap, &hdu)
        .map_err(|e| anyhow::anyhow!("decoding compressed image: {e}"))?;
    if w * h == 0 {
        bail!("compressed image has an empty plane ({w}x{h})");
    }
    if values.len() == w * h {
        return Ok(vec![
            Array2::from_shape_vec((h, w), values).context("plane shape")?
        ]);
    }
    values
        .chunks(w * h)
        .map(|plane| Array2::from_shape_vec((h, w), plane.to_vec()).context("plane shape"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    const NULL_VALUE: i32 = -2_147_483_647;

    fn make_header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for &(k, v) in pairs {
            header.index.insert(k.to_string(), v.to_string());
            header.cards.push((k.to_string(), v.to_string()));
        }
        header
    }

    fn gzip1(raw: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(raw).unwrap();
        enc.finish().unwrap()
    }

    fn gzip2(raw: &[u8], bytepix: usize) -> Vec<u8> {
        let pixels = raw.len() / bytepix;
        let mut shuffled = vec![0u8; raw.len()];
        for (i, pixel) in raw.chunks_exact(bytepix).enumerate() {
            for (j, &b) in pixel.iter().enumerate() {
                shuffled[j * pixels + i] = b;
            }
        }
        gzip1(&shuffled)
    }

    fn be_i32(values: &[i32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn be_i16(values: &[i16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn rice_tile<T: fitsio_pure::compress::CompressPixel>(values: &[T]) -> Vec<u8> {
        use fitsio_pure::compress::{compress_image_hdu, TileCompression};
        let hdu =
            compress_image_hdu(&[values.len()], values, &TileCompression::rice(), &[]).unwrap();
        let end = hdu
            .chunks(80)
            .position(|c| c.starts_with(b"END     "))
            .unwrap();
        let data = &hdu[(end * 80 + 80).div_ceil(2880) * 2880..];
        let len = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
        let offset = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
        let heap = &data[8..];
        heap[offset..offset + len].to_vec()
    }

    struct SingleTile<'a> {
        zcmptype: &'a str,
        zbitpix: i64,
        npix: usize,
        heap: Vec<u8>,
        quant: Option<(f64, f64, &'a str)>,
        extra: Vec<(&'a str, &'a str)>,
    }

    fn tile<'a>(
        zcmptype: &'a str,
        zbitpix: i64,
        npix: usize,
        heap: Vec<u8>,
        quant: Option<(f64, f64, &'a str)>,
    ) -> SingleTile<'a> {
        SingleTile {
            zcmptype,
            zbitpix,
            npix,
            heap,
            quant,
            extra: Vec::new(),
        }
    }

    fn single_tile_hdu(t: &SingleTile) -> (Vec<u8>, HduHeader) {
        let row_width: usize = if t.quant.is_some() { 24 } else { 8 };
        let mut mmap = Vec::new();
        mmap.extend_from_slice(&(t.heap.len() as i32).to_be_bytes());
        mmap.extend_from_slice(&0i32.to_be_bytes());
        if let Some((zscale, zzero, _)) = t.quant {
            mmap.extend_from_slice(&zscale.to_be_bytes());
            mmap.extend_from_slice(&zzero.to_be_bytes());
        }
        assert_eq!(mmap.len(), row_width);
        mmap.extend_from_slice(&t.heap);

        let npix = t.npix.to_string();
        let zbitpix = t.zbitpix.to_string();
        let naxis1 = row_width.to_string();
        let pcount = t.heap.len().to_string();
        let tfields = if t.quant.is_some() { "3" } else { "1" };
        let mut pairs: Vec<(&str, &str)> = vec![
            ("XTENSION", "BINTABLE"),
            ("BITPIX", "8"),
            ("NAXIS", "2"),
            ("NAXIS1", &naxis1),
            ("NAXIS2", "1"),
            ("PCOUNT", &pcount),
            ("TFIELDS", tfields),
            ("TTYPE1", "COMPRESSED_DATA"),
            ("TFORM1", "1PB(0)"),
            ("ZIMAGE", "T"),
            ("ZCMPTYPE", t.zcmptype),
            ("ZBITPIX", &zbitpix),
            ("ZNAXIS", "2"),
            ("ZNAXIS1", &npix),
            ("ZNAXIS2", "1"),
            ("ZTILE1", &npix),
            ("ZTILE2", "1"),
        ];
        if let Some((_, _, zquantiz)) = t.quant {
            pairs.extend([
                ("TTYPE2", "ZSCALE"),
                ("TFORM2", "1D"),
                ("TTYPE3", "ZZERO"),
                ("TFORM3", "1D"),
                ("ZBLANK", "-2147483647"),
            ]);
            if !zquantiz.is_empty() {
                pairs.extend([("ZQUANTIZ", zquantiz), ("ZDITHER0", "1")]);
            }
        }
        pairs.extend(t.extra.iter().copied());
        let mut header = make_header(&pairs);
        header.string_keys = Some(
            [
                "XTENSION", "TTYPE1", "TFORM1", "ZCMPTYPE", "TTYPE2", "TFORM2", "TTYPE3", "TFORM3",
                "ZQUANTIZ", "ZNAME1", "ZNAME2",
            ]
            .iter()
            .map(|k| k.to_string())
            .collect(),
        );
        (mmap, header)
    }

    #[test]
    fn quantized_columns_without_zquantiz_decode_as_no_dither() {
        for (zcmptype, heap) in [
            ("RICE_1", rice_tile(&QUANT_INTS)),
            ("GZIP_1", gzip1(&be_i32(&QUANT_INTS))),
            ("NOCOMPRESS", be_i32(&QUANT_INTS)),
        ] {
            let pixels = decode_single_tile(&tile(
                zcmptype,
                -32,
                QUANT_INTS.len(),
                heap,
                Some((0.5, 100.0, "")),
            ))
            .unwrap();
            assert_quantized_no_dither(&pixels);
        }
    }

    #[test]
    fn zscale_and_zzero_header_keywords_scale_every_tile() {
        let (mmap, mut header) = single_tile_hdu(&tile(
            "RICE_1",
            -32,
            QUANT_INTS.len(),
            rice_tile(&QUANT_INTS),
            None,
        ));
        header.set("ZSCALE", "0.5".to_string());
        header.set("ZZERO", "100.0".to_string());
        header.set("ZBLANK", NULL_VALUE.to_string());
        let pixels: Vec<f32> = decode_compressed_image(&mmap, &header, 0)
            .unwrap()
            .iter()
            .copied()
            .collect();
        assert_quantized_no_dither(&pixels);
    }

    #[test]
    fn a_rice_float_image_without_any_scale_is_refused_instead_of_returning_codes() {
        let err = decode_single_tile(&tile(
            "RICE_1",
            -32,
            QUANT_INTS.len(),
            rice_tile(&QUANT_INTS),
            None,
        ))
        .expect_err("integer codes must not be passed off as float pixels");
        assert!(err.to_string().contains("ZSCALE/ZZERO"), "{err}");
    }

    #[test]
    fn zquantiz_none_keeps_a_lossless_float_tile_unscaled() {
        let values = [1.5f32, -2.25, 1e10, 0.0, 3.0];
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        let pixels = decode_single_tile(&tile(
            "GZIP_2",
            -32,
            values.len(),
            gzip2(&raw, 4),
            Some((0.5, 100.0, "NONE")),
        ))
        .unwrap();
        assert_eq!(pixels, values.to_vec());
    }

    fn decode_single_tile(t: &SingleTile) -> Result<Vec<f32>> {
        let (mmap, header) = single_tile_hdu(t);
        let image = decode_compressed_image(&mmap, &header, 0)?;
        Ok(image.iter().copied().collect())
    }

    const QUANT_INTS: [i32; 6] = [0, 1, 2, -3, 40, NULL_VALUE];
    const QUANT_EXPECTED: [f32; 5] = [100.0, 100.5, 101.0, 98.5, 120.0];

    fn assert_quantized_no_dither(pixels: &[f32]) {
        assert_eq!(pixels.len(), QUANT_INTS.len());
        assert_eq!(&pixels[..5], &QUANT_EXPECTED[..]);
        assert!(
            pixels[5].is_nan(),
            "ZBLANK pixel must decode to NaN, got {}",
            pixels[5]
        );
    }

    #[test]
    fn gzip2_lossless_float64_tile_decodes() {
        let values = [1.5f64, -2.25, 1e10, 0.0, 3.0];
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        let pixels =
            decode_single_tile(&tile("GZIP_2", -64, values.len(), gzip2(&raw, 8), None)).unwrap();
        let expected: Vec<f32> = values.iter().map(|&v| v as f32).collect();
        assert_eq!(pixels, expected);
    }

    #[test]
    fn gzip1_lossless_int64_tile_decodes() {
        let values = [7i64, -8, 1 << 40, 0];
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        let pixels =
            decode_single_tile(&tile("GZIP_1", 64, values.len(), gzip1(&raw), None)).unwrap();
        let expected: Vec<f32> = values.iter().map(|&v| v as f32).collect();
        assert_eq!(pixels, expected);
    }

    #[test]
    fn gzip1_quantized_tile_payload_is_int32() {
        let pixels = decode_single_tile(&tile(
            "GZIP_1",
            -32,
            QUANT_INTS.len(),
            gzip1(&be_i32(&QUANT_INTS)),
            Some((0.5, 100.0, "NO_DITHER")),
        ))
        .unwrap();
        assert_quantized_no_dither(&pixels);
    }

    #[test]
    fn nocompress_quantized_tile_payload_is_int32() {
        let pixels = decode_single_tile(&tile(
            "NOCOMPRESS",
            -32,
            QUANT_INTS.len(),
            be_i32(&QUANT_INTS),
            Some((0.5, 100.0, "NO_DITHER")),
        ))
        .unwrap();
        assert_quantized_no_dither(&pixels);
    }

    const QUANT_INTS_I16: [i16; 5] = [0, 1, 2, -3, 40];

    #[test]
    fn gzip1_quantized_tile_int16_payload() {
        let pixels = decode_single_tile(&tile(
            "GZIP_1",
            -32,
            QUANT_INTS_I16.len(),
            gzip1(&be_i16(&QUANT_INTS_I16)),
            Some((0.5, 100.0, "NO_DITHER")),
        ))
        .unwrap();
        assert_eq!(pixels, QUANT_EXPECTED.to_vec());
    }

    #[test]
    fn gzip2_quantized_tile_int16_payload() {
        let pixels = decode_single_tile(&tile(
            "GZIP_2",
            -32,
            QUANT_INTS_I16.len(),
            gzip2(&be_i16(&QUANT_INTS_I16), 2),
            Some((0.5, 100.0, "NO_DITHER")),
        ))
        .unwrap();
        assert_eq!(pixels, QUANT_EXPECTED.to_vec());
    }

    #[test]
    fn quantized_tile_with_bad_byte_count_is_an_error() {
        let err = decode_single_tile(&tile(
            "NOCOMPRESS",
            -32,
            6,
            vec![1u8; 7],
            Some((0.5, 100.0, "NO_DITHER")),
        ))
        .expect_err("7 bytes for 6 pixels must be reported as an error");
        assert!(err.to_string().contains("byte count"), "{err}");
    }

    #[test]
    fn gzip2_quantized_tile_applies_subtractive_dither() {
        let pixels = decode_single_tile(&tile(
            "GZIP_2",
            -64,
            QUANT_INTS.len(),
            gzip2(&be_i32(&QUANT_INTS), 4),
            Some((0.5, 100.0, "SUBTRACTIVE_DITHER_1")),
        ))
        .unwrap();
        assert_eq!(pixels.len(), QUANT_INTS.len());
        for (i, (got, want)) in pixels.iter().zip(QUANT_EXPECTED.iter()).enumerate() {
            assert!((got - want).abs() <= 0.25 + 1e-6, "pixel {i}: {got}");
            assert_ne!(*got, *want, "pixel {i} was not dithered");
        }
        assert!(
            pixels[5].is_nan(),
            "ZBLANK pixel must decode to NaN, got {}",
            pixels[5]
        );
    }

    #[test]
    fn rice_quantized_float64_tile_defaults_to_bytepix_4() {
        let pixels = decode_single_tile(&tile(
            "RICE_1",
            -64,
            QUANT_INTS.len(),
            rice_tile(&QUANT_INTS),
            Some((0.5, 100.0, "NO_DITHER")),
        ))
        .unwrap();
        assert_quantized_no_dither(&pixels);
    }

    #[test]
    fn truncated_rice_tile_is_an_error_not_a_panic() {
        let values: Vec<i16> = (0..64).map(|i| ((i * 37 % 101) - 50) as i16).collect();
        let encoded = rice_tile(&values);
        let mut t = tile(
            "RICE_1",
            16,
            values.len(),
            encoded[..encoded.len() / 2].to_vec(),
            None,
        );
        t.extra = vec![
            ("ZNAME1", "BLOCKSIZE"),
            ("ZVAL1", "32"),
            ("ZNAME2", "BYTEPIX"),
            ("ZVAL2", "2"),
        ];
        let err =
            decode_single_tile(&t).expect_err("half a Rice stream must be reported as an error");
        assert!(err.to_string().contains("truncated"), "{err}");
    }
}
