// mef_writer — contributed by Jae-Joon Lee <https://github.com/leejjoon>

use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::{bail, Context, Result};
use ndarray::Array2;

use crate::types::constants::BLOCK_SIZE;
use crate::types::HduHeader;

use super::compress::is_compressed_image_hdu;
use super::file_bytes::read_file_bytes;
use super::reader::{decode_pixels, parse_header_at};
use super::writer::{
    write_planes_gzip2_lossless, write_planes_lossless_int, write_planes_quantized,
    write_primary_hdu_stub,
};

struct SourceHdu {
    header: HduHeader,
    header_start: usize,
    data_start: usize,
    next_hdu_offset: usize,
}

fn scan_source_hdus(mmap: &[u8]) -> Result<Vec<SourceHdu>> {
    let mut hdus = Vec::new();
    let mut offset = 0usize;
    while offset + BLOCK_SIZE <= mmap.len() {
        let parsed = parse_header_at(mmap, offset)?;
        let next = parsed.next_hdu_offset;
        hdus.push(SourceHdu {
            header: parsed.header,
            header_start: parsed.header_start,
            data_start: parsed.data_start,
            next_hdu_offset: next,
        });
        offset = next;
    }
    if hdus.is_empty() {
        bail!("No HDUs found in source file");
    }
    Ok(hdus)
}

fn read_raw_ints_be(data: &[u8], bitpix: i64) -> Result<Vec<i64>> {
    match bitpix {
        8 => Ok(data.iter().map(|&b| b as i64).collect()),
        16 => Ok(data
            .chunks_exact(2)
            .map(|c| i16::from_be_bytes([c[0], c[1]]) as i64)
            .collect()),
        32 => Ok(data
            .chunks_exact(4)
            .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]) as i64)
            .collect()),
        other => bail!("read_raw_ints_be: unsupported integer BITPIX {other}"),
    }
}

fn is_image_extension(header: &HduHeader) -> bool {
    match header.get("XTENSION") {
        None => header.get_i64("NAXIS").unwrap_or(0) > 0,
        Some(x) => x.trim().eq_ignore_ascii_case("IMAGE"),
    }
}

fn copy_verbatim(mmap: &[u8], writer: &mut BufWriter<File>, hdu: &SourceHdu) -> Result<()> {
    writer.write_all(&mmap[hdu.header_start..hdu.next_hdu_offset])?;
    Ok(())
}

pub enum CompressMode {
    Lossy { quantize_level: f64 },
    Lossless,
}

pub struct CompressOptions {
    pub mode: CompressMode,
    pub drop_extnames: Vec<String>,
    pub raw_extnames: Vec<String>,
}

pub struct MefReport {
    pub dropped: Vec<String>,
    pub kept_raw: Vec<String>,
}

fn normalize_extname(s: &str) -> String {
    s.trim().trim_matches('\'').trim().to_ascii_uppercase()
}

pub fn write_compressed_mef(
    source_path: &str,
    output_path: &str,
    opts: &CompressOptions,
) -> Result<MefReport> {
    let file = File::open(source_path).with_context(|| format!("Failed to open {source_path}"))?;
    let mmap = read_file_bytes(&file)?;
    let hdus = scan_source_hdus(&mmap)?;

    let drop_set: Vec<String> = opts.drop_extnames.iter().map(|s| normalize_extname(s)).collect();
    let raw_set: Vec<String> = opts.raw_extnames.iter().map(|s| normalize_extname(s)).collect();
    let mut dropped: Vec<String> = Vec::new();
    let mut kept_raw: Vec<String> = Vec::new();

    let out_file = File::create(output_path).context("Failed to create output FITS file")?;
    let mut writer = BufWriter::with_capacity(4 * 1024 * 1024, out_file);

    write_primary_hdu_stub(&mut writer, Some(&hdus[0].header))?;

    for (hdu_idx, hdu) in hdus.iter().enumerate() {
        let header = &hdu.header;

        if hdu_idx == 0 && header.get_i64("NAXIS").unwrap_or(0) == 0 {
            continue;
        }

        if !drop_set.is_empty() || !raw_set.is_empty() {
            if let Some(name) = header.get("EXTNAME") {
                let norm = normalize_extname(name);
                if drop_set.contains(&norm) {
                    dropped.push(norm);
                    continue;
                }
                if raw_set.contains(&norm) {
                    kept_raw.push(norm);
                    copy_verbatim(&mmap, &mut writer, hdu)?;
                    continue;
                }
            }
        }

        if is_compressed_image_hdu(header) {
            copy_verbatim(&mmap, &mut writer, hdu)?;
            continue;
        }

        if !is_image_extension(header) {
            copy_verbatim(&mmap, &mut writer, hdu)?;
            continue;
        }

        let naxis = header.get_i64("NAXIS").unwrap_or(0);
        if naxis < 2 || naxis > 3 {
            copy_verbatim(&mmap, &mut writer, hdu)?;
            continue;
        }

        let naxis1 = header.get_i64("NAXIS1").unwrap_or(0) as usize;
        let naxis2 = header.get_i64("NAXIS2").unwrap_or(0) as usize;
        let nplanes = if naxis == 3 {
            header.get_i64("NAXIS3").unwrap_or(1) as usize
        } else {
            1
        };
        let bitpix = header.get_i64("BITPIX").context("Missing BITPIX")?;
        let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
        let plane_len = naxis1 * naxis2;
        let total_len = plane_len * nplanes;
        let data_end = hdu.data_start + total_len * bytes_per_pixel;
        if data_end > mmap.len() {
            bail!(
                "HDU data exceeds file size (EXTNAME={:?})",
                header.get("EXTNAME")
            );
        }
        let raw = &mmap[hdu.data_start..data_end];

        if bitpix < 0 {
            let bzero = header.get_f64("BZERO").unwrap_or(0.0);
            let bscale = header.get_f64("BSCALE").unwrap_or(1.0);
            match opts.mode {
                CompressMode::Lossless => {
                    let plane_byte_len = plane_len * bytes_per_pixel;
                    let plane_slices: Vec<&[u8]> = (0..nplanes)
                        .map(|p| &raw[p * plane_byte_len..(p + 1) * plane_byte_len])
                        .collect();
                    write_planes_gzip2_lossless(
                        &mut writer, &plane_slices, naxis1, naxis2, bitpix as i32, bzero, bscale,
                        Some(header),
                    )?;
                }
                CompressMode::Lossy { quantize_level } => {
                    let pixels = decode_pixels(raw, bitpix, bscale, bzero);
                    let mut planes = Vec::with_capacity(nplanes);
                    for p in 0..nplanes {
                        let chunk = pixels[p * plane_len..(p + 1) * plane_len].to_vec();
                        planes.push(
                            Array2::from_shape_vec((naxis2, naxis1), chunk)
                                .context("Failed to reshape plane")?,
                        );
                    }
                    write_planes_quantized(&mut writer, &planes, Some(header), quantize_level)?;
                }
            }
        } else {
            let all_ints = read_raw_ints_be(raw, bitpix)?;
            let mut planes = Vec::with_capacity(nplanes);
            for p in 0..nplanes {
                planes.push(all_ints[p * plane_len..(p + 1) * plane_len].to_vec());
            }
            let bzero = header.get_f64("BZERO").unwrap_or(0.0);
            let bscale = header.get_f64("BSCALE").unwrap_or(1.0);
            let blank = header.get_i64("BLANK");
            write_planes_lossless_int(
                &mut writer, &planes, naxis1, naxis2, bitpix as i32, blank, bzero, bscale,
                Some(header),
            )?;
        }
    }

    writer.flush()?;
    Ok(MefReport { dropped, kept_raw })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::compress::{decode_compressed_image, decode_compressed_planes};

    const BLOCK: usize = BLOCK_SIZE;

    fn card(key: &str, value: &str) -> Vec<u8> {
        let text = format!("{key:<8}= {value}");
        let mut bytes = text.into_bytes();
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
        while out.len() % BLOCK != 0 {
            out.push(b' ');
        }
        out
    }

    fn f32_data_block(pixels: &[f32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(pixels.len() * 4);
        for p in pixels {
            out.extend_from_slice(&p.to_be_bytes());
        }
        while out.len() % BLOCK != 0 {
            out.push(0);
        }
        out
    }

    fn i32_data_block(pixels: &[i32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(pixels.len() * 4);
        for p in pixels {
            out.extend_from_slice(&p.to_be_bytes());
        }
        while out.len() % BLOCK != 0 {
            out.push(0);
        }
        out
    }

    fn write_synthetic_source(path: &std::path::Path) {
        let (w, h) = (24usize, 16usize);
        let mut buf = Vec::new();

        buf.extend_from_slice(&header_block(&[
            ("SIMPLE", "T".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "0".into()),
            ("EXTEND", "T".into()),
            ("VERSION", "'6.4     '".into()),
        ]));

        let sci: Vec<f32> = (0..w * h).map(|i| 100.0 + (i as f32 * 0.07).sin() * 10.0).collect();
        buf.extend_from_slice(&header_block(&[
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'SCI     '".into()),
            ("CRPIX1", "12.5".into()),
            ("CRPIX2", "8.5".into()),
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
        ]));
        buf.extend_from_slice(&f32_data_block(&sci));

        let mut flags = vec![0i32; w * h];
        flags[5] = 1 << 3;
        flags[40] = (1 << 7) | (1 << 2);
        flags[200] = 1 << 15;
        buf.extend_from_slice(&header_block(&[
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'FLAGS   '".into()),
        ]));
        buf.extend_from_slice(&i32_data_block(&flags));

        let (cw, ch, cn) = (8usize, 6usize, 3usize);
        let cube: Vec<f32> = (0..cw * ch * cn).map(|i| (i as f32) * 0.5 - 3.0).collect();
        buf.extend_from_slice(&header_block(&[
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", cw.to_string()),
            ("NAXIS2", ch.to_string()),
            ("NAXIS3", cn.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'CUBE    '".into()),
        ]));
        buf.extend_from_slice(&f32_data_block(&cube));

        buf.extend_from_slice(&header_block(&[
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "1".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("EXTNAME", "'AUX     '".into()),
            ("TTYPE1", "'X       '".into()),
            ("TFORM1", "'1J      '".into()),
        ]));
        buf.extend_from_slice(&i32_data_block(&[42]));

        std::fs::write(path, &buf).unwrap();
    }

    #[test]
    fn write_compressed_mef_full_roundtrip() {
        let dir = std::env::temp_dir();
        let source_path = dir.join("ab_mef_source.fits");
        let output_path = dir.join("ab_mef_output.fits");
        write_synthetic_source(&source_path);

        write_compressed_mef(
            source_path.to_str().unwrap(),
            output_path.to_str().unwrap(),
            &CompressOptions {
                mode: CompressMode::Lossy { quantize_level: 16.0 },
                drop_extnames: vec![],
                raw_extnames: vec![],
            },
        )
        .unwrap();

        let source_bytes = std::fs::read(&source_path).unwrap();
        let out_bytes = std::fs::read(&output_path).unwrap();

        let primary = parse_header_at(&out_bytes, 0).unwrap();
        assert_eq!(primary.header.get("SIMPLE").map(|v| v.trim()), Some("T"));
        assert_eq!(primary.header.get("VERSION").map(|v| v.trim()), Some("6.4"));

        let hdu1 = parse_header_at(&out_bytes, primary.next_hdu_offset).unwrap();
        assert_eq!(hdu1.header.get("ZCMPTYPE"), Some("RICE_1"));
        assert_eq!(hdu1.header.get("ZQUANTIZ"), Some("SUBTRACTIVE_DITHER_1"));
        assert_eq!(hdu1.header.get("EXTNAME").map(|v| v.trim()), Some("SCI"));
        assert_eq!(hdu1.header.get_f64("CRPIX1"), Some(12.5));
        let sci_decoded = decode_compressed_image(&out_bytes, &hdu1.header, hdu1.data_start).unwrap();
        let sci_original: Vec<f32> =
            (0..24 * 16).map(|i| 100.0 + (i as f32 * 0.07).sin() * 10.0).collect();
        for (o, d) in sci_original.iter().zip(sci_decoded.iter()) {
            assert!((o - d).abs() < 2.0, "SCI mismatch: {o} vs {d}");
        }

        let hdu2 = parse_header_at(&out_bytes, hdu1.next_hdu_offset).unwrap();
        assert_eq!(hdu2.header.get("ZCMPTYPE"), Some("RICE_1"));
        assert!(hdu2.header.get("ZQUANTIZ").is_none(), "FLAGS must not be quantized");
        assert_eq!(hdu2.header.get_i64("ZBITPIX"), Some(32));
        assert_eq!(hdu2.header.get("EXTNAME").map(|v| v.trim()), Some("FLAGS"));
        let flags_decoded = decode_compressed_image(&out_bytes, &hdu2.header, hdu2.data_start).unwrap();
        let mut flags_original = vec![0.0f32; 24 * 16];
        flags_original[5] = (1 << 3) as f32;
        flags_original[40] = ((1 << 7) | (1 << 2)) as f32;
        flags_original[200] = (1 << 15) as f32;
        for (o, d) in flags_original.iter().zip(flags_decoded.iter()) {
            assert_eq!(*o, *d, "FLAGS must round-trip exactly (lossless int path)");
        }

        let hdu3 = parse_header_at(&out_bytes, hdu2.next_hdu_offset).unwrap();
        assert_eq!(hdu3.header.get_i64("ZNAXIS"), Some(3));
        assert_eq!(hdu3.header.get_i64("ZNAXIS3"), Some(3));
        assert_eq!(hdu3.header.get("EXTNAME").map(|v| v.trim()), Some("CUBE"));
        let planes = decode_compressed_planes(&out_bytes, &hdu3.header, hdu3.data_start).unwrap();
        assert_eq!(planes.len(), 3);
        let cube_original: Vec<f32> = (0..8 * 6 * 3).map(|i| (i as f32) * 0.5 - 3.0).collect();
        for (p, plane) in planes.iter().enumerate() {
            for (i, &d) in plane.iter().enumerate() {
                let o = cube_original[p * 8 * 6 + i];
                assert!((o - d).abs() < 2.0, "CUBE plane {p} mismatch: {o} vs {d}");
            }
        }

        let hdu4 = parse_header_at(&out_bytes, hdu3.next_hdu_offset).unwrap();
        assert_eq!(hdu4.header.get("XTENSION").map(|v| v.trim()), Some("BINTABLE"));
        assert_eq!(hdu4.header.get("EXTNAME").map(|v| v.trim()), Some("AUX"));

        let src_primary = parse_header_at(&source_bytes, 0).unwrap();
        let src_hdu1 = parse_header_at(&source_bytes, src_primary.next_hdu_offset).unwrap();
        let src_hdu2 = parse_header_at(&source_bytes, src_hdu1.next_hdu_offset).unwrap();
        let src_hdu3 = parse_header_at(&source_bytes, src_hdu2.next_hdu_offset).unwrap();
        let src_hdu4 = parse_header_at(&source_bytes, src_hdu3.next_hdu_offset).unwrap();
        assert_eq!(
            &out_bytes[hdu4.header_start..hdu4.next_hdu_offset],
            &source_bytes[src_hdu4.header_start..src_hdu4.next_hdu_offset],
            "AUX BinTable must be passed through byte-identical"
        );

        let _ = std::fs::remove_file(&source_path);
        let _ = std::fs::remove_file(&output_path);
    }

    #[test]
    fn write_compressed_mef_lossless_roundtrips_floats_exactly() {
        let dir = std::env::temp_dir();
        let source_path = dir.join("ab_mef_lossless_source.fits");
        let output_path = dir.join("ab_mef_lossless_output.fits");
        let output_path2 = dir.join("ab_mef_lossless_output2.fits");
        write_synthetic_source(&source_path);

        let opts = CompressOptions {
            mode: CompressMode::Lossless,
            drop_extnames: vec![],
            raw_extnames: vec![],
        };
        let report = write_compressed_mef(
            source_path.to_str().unwrap(),
            output_path.to_str().unwrap(),
            &opts,
        )
        .unwrap();
        assert!(report.dropped.is_empty() && report.kept_raw.is_empty());

        let out_bytes = std::fs::read(&output_path).unwrap();
        let primary = parse_header_at(&out_bytes, 0).unwrap();

        let hdu1 = parse_header_at(&out_bytes, primary.next_hdu_offset).unwrap();
        assert_eq!(hdu1.header.get("ZCMPTYPE"), Some("GZIP_2"));
        assert!(hdu1.header.get("ZQUANTIZ").is_none(), "lossless float must not be quantized");
        assert_eq!(hdu1.header.get_i64("ZBITPIX"), Some(-32));
        let sci_decoded = decode_compressed_image(&out_bytes, &hdu1.header, hdu1.data_start).unwrap();
        let sci_original: Vec<f32> =
            (0..24 * 16).map(|i| 100.0 + (i as f32 * 0.07).sin() * 10.0).collect();
        assert_eq!(sci_decoded.len(), sci_original.len());
        for (o, d) in sci_original.iter().zip(sci_decoded.iter()) {
            assert_eq!(*d, *o, "SCI must round-trip bit-exact under GZIP_2");
        }

        let hdu2 = parse_header_at(&out_bytes, hdu1.next_hdu_offset).unwrap();
        assert_eq!(hdu2.header.get("ZCMPTYPE"), Some("RICE_1"));

        let hdu3 = parse_header_at(&out_bytes, hdu2.next_hdu_offset).unwrap();
        assert_eq!(hdu3.header.get("ZCMPTYPE"), Some("GZIP_2"));
        let planes = decode_compressed_planes(&out_bytes, &hdu3.header, hdu3.data_start).unwrap();
        let cube_original: Vec<f32> = (0..8 * 6 * 3).map(|i| (i as f32) * 0.5 - 3.0).collect();
        for (p, plane) in planes.iter().enumerate() {
            for (i, &d) in plane.iter().enumerate() {
                assert_eq!(d, cube_original[p * 8 * 6 + i], "CUBE plane {p} idx {i} not exact");
            }
        }

        write_compressed_mef(source_path.to_str().unwrap(), output_path2.to_str().unwrap(), &opts).unwrap();
        assert_eq!(out_bytes, std::fs::read(&output_path2).unwrap(), "lossless output must be deterministic");

        let _ = std::fs::remove_file(&source_path);
        let _ = std::fs::remove_file(&output_path);
        let _ = std::fs::remove_file(&output_path2);
    }

    #[test]
    fn write_compressed_mef_drops_named_hdus() {
        let dir = std::env::temp_dir();
        let source_path = dir.join("ab_mef_drop_source.fits");
        let output_path = dir.join("ab_mef_drop_output.fits");
        write_synthetic_source(&source_path);

        let report = write_compressed_mef(
            source_path.to_str().unwrap(),
            output_path.to_str().unwrap(),
            &CompressOptions {
                mode: CompressMode::Lossy { quantize_level: 16.0 },
                drop_extnames: vec!["flags".into(), "NOPE".into()],
                raw_extnames: vec![],
            },
        )
        .unwrap();
        assert_eq!(report.dropped, vec!["FLAGS".to_string()]);

        let out_bytes = std::fs::read(&output_path).unwrap();
        let mut names = Vec::new();
        let mut off = 0usize;
        while off + BLOCK <= out_bytes.len() {
            let h = parse_header_at(&out_bytes, off).unwrap();
            if let Some(n) = h.header.get("EXTNAME") {
                names.push(n.trim().to_string());
            }
            if h.next_hdu_offset <= off {
                break;
            }
            off = h.next_hdu_offset;
        }
        assert!(!names.iter().any(|n| n == "FLAGS"), "FLAGS should be dropped, got {names:?}");
        assert!(names.iter().any(|n| n == "SCI"), "SCI should remain, got {names:?}");
        assert!(names.iter().any(|n| n == "AUX"), "AUX should remain, got {names:?}");

        let _ = std::fs::remove_file(&source_path);
        let _ = std::fs::remove_file(&output_path);
    }

    #[test]
    fn write_compressed_mef_keeps_named_hdus_raw() {
        let dir = std::env::temp_dir();
        let source_path = dir.join("ab_mef_raw_source.fits");
        let output_path = dir.join("ab_mef_raw_output.fits");
        write_synthetic_source(&source_path);

        let report = write_compressed_mef(
            source_path.to_str().unwrap(),
            output_path.to_str().unwrap(),
            &CompressOptions {
                mode: CompressMode::Lossless,
                drop_extnames: vec![],
                raw_extnames: vec!["sci".into()],
            },
        )
        .unwrap();
        assert_eq!(report.kept_raw, vec!["SCI".to_string()]);
        assert!(report.dropped.is_empty());

        let source_bytes = std::fs::read(&source_path).unwrap();
        let out_bytes = std::fs::read(&output_path).unwrap();

        let primary = parse_header_at(&out_bytes, 0).unwrap();
        let hdu1 = parse_header_at(&out_bytes, primary.next_hdu_offset).unwrap();
        assert_eq!(hdu1.header.get("EXTNAME").map(|v| v.trim()), Some("SCI"));
        assert_eq!(hdu1.header.get("XTENSION").map(|v| v.trim()), Some("IMAGE"));
        assert!(hdu1.header.get("ZCMPTYPE").is_none(), "SCI must not be compressed");
        let src_primary = parse_header_at(&source_bytes, 0).unwrap();
        let src_hdu1 = parse_header_at(&source_bytes, src_primary.next_hdu_offset).unwrap();
        assert_eq!(
            &out_bytes[hdu1.header_start..hdu1.next_hdu_offset],
            &source_bytes[src_hdu1.header_start..src_hdu1.next_hdu_offset],
            "SCI must be passed through byte-identical"
        );

        let mut found_cube_gzip2 = false;
        let mut off = hdu1.next_hdu_offset;
        while off + BLOCK <= out_bytes.len() {
            let h = parse_header_at(&out_bytes, off).unwrap();
            if h.header.get("EXTNAME").map(|v| v.trim()) == Some("CUBE") {
                assert_eq!(h.header.get("ZCMPTYPE"), Some("GZIP_2"));
                found_cube_gzip2 = true;
            }
            if h.next_hdu_offset <= off {
                break;
            }
            off = h.next_hdu_offset;
        }
        assert!(found_cube_gzip2, "CUBE should still be GZIP_2-compressed");

        let _ = std::fs::remove_file(&source_path);
        let _ = std::fs::remove_file(&output_path);
    }
}
