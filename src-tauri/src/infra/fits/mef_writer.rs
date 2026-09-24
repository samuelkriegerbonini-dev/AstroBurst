// mef_writer — contributed by Jae-Joon Lee <https://github.com/leejjoon>

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use ndarray::Array2;

use crate::types::constants::BLOCK_SIZE;
use crate::types::HduHeader;

use super::compress::is_compressed_image_hdu;
use super::file_bytes::read_file_bytes;
use super::reader::{decode_pixels, parse_header_at};
use super::writer::{
    validate_quantize_level, write_planes_gzip2_lossless, write_planes_lossless_int,
    write_planes_quantized, write_primary_hdu_stub,
};

struct SourceHdu {
    header: HduHeader,
    header_start: usize,
    data_start: usize,
    data_bytes: usize,
    data_end: Option<usize>,
    padded_end: usize,
}

fn hdu_label(index: usize, header: &HduHeader) -> String {
    header
        .get("EXTNAME")
        .map(normalize_extname)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("HDU{index}"))
}

fn truncation_error(index: usize, hdu: &SourceHdu, file_len: usize) -> anyhow::Error {
    anyhow::anyhow!(
        "HDU {index} ({}) is truncated: its data needs {} bytes from offset {} but the file ends at {file_len}",
        hdu_label(index, &hdu.header),
        hdu.data_bytes,
        hdu.data_start
    )
}

fn scan_source_hdus(mmap: &[u8]) -> Result<Vec<SourceHdu>> {
    let mut hdus = Vec::new();
    let mut offset = 0usize;
    while offset + BLOCK_SIZE <= mmap.len() {
        let parsed = parse_header_at(mmap, offset)?;
        let data_bytes = parsed.header.data_byte_count();
        let data_end = parsed
            .data_start
            .checked_add(data_bytes)
            .filter(|&end| end <= mmap.len());
        let padded_end = parsed.next_hdu_offset;
        hdus.push(SourceHdu {
            header: parsed.header,
            header_start: parsed.header_start,
            data_start: parsed.data_start,
            data_bytes,
            data_end,
            padded_end,
        });
        offset = padded_end;
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

fn is_ascii_table(header: &HduHeader) -> bool {
    header
        .get("XTENSION")
        .is_some_and(|x| x.trim().eq_ignore_ascii_case("TABLE"))
}

fn copy_verbatim(mmap: &[u8], writer: &mut BufWriter<File>, hdu: &SourceHdu) -> Result<()> {
    if hdu.data_end.is_none() {
        bail!("HDU data at offset {} runs past the end of the source file", hdu.data_start);
    }
    let available_end = hdu.padded_end.min(mmap.len());
    let bytes = mmap
        .get(hdu.header_start..available_end)
        .context("HDU lies outside the source file")?;
    writer.write_all(bytes)?;
    let missing_padding = hdu.padded_end - available_end;
    if missing_padding > 0 {
        let fill = if is_ascii_table(&hdu.header) { b' ' } else { 0u8 };
        writer.write_all(&vec![fill; missing_padding])?;
    }
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

#[derive(Debug)]
pub struct MefReport {
    pub dropped: Vec<String>,
    pub kept_raw: Vec<String>,
    pub uncompressed: Vec<String>,
}

fn normalize_extname(s: &str) -> String {
    s.trim().trim_matches('\'').trim().to_ascii_uppercase()
}

struct ImageGeometry {
    naxis1: usize,
    naxis2: usize,
    nplanes: usize,
    bitpix: i64,
    bytes: usize,
}

fn image_geometry(header: &HduHeader) -> Result<Option<ImageGeometry>> {
    let naxis = header.get_i64("NAXIS").unwrap_or(0);
    if !(2..=3).contains(&naxis) {
        return Ok(None);
    }
    let bitpix = header.get_i64("BITPIX").context("Missing BITPIX")?;
    if !matches!(bitpix, 8 | 16 | 32 | -32 | -64) {
        return Ok(None);
    }
    let axis = |key: &str| {
        header
            .get_i64(key)
            .and_then(|v| usize::try_from(v).ok())
            .filter(|&v| v > 0)
    };
    let (Some(naxis1), Some(naxis2)) = (axis("NAXIS1"), axis("NAXIS2")) else {
        return Ok(None);
    };
    let nplanes = if naxis == 3 {
        match axis("NAXIS3") {
            Some(n) => n,
            None => return Ok(None),
        }
    } else {
        1
    };
    let bytes = naxis1
        .checked_mul(naxis2)
        .and_then(|n| n.checked_mul(nplanes))
        .and_then(|n| n.checked_mul(bitpix.unsigned_abs() as usize / 8));
    Ok(bytes.map(|bytes| ImageGeometry { naxis1, naxis2, nplanes, bitpix, bytes }))
}

enum HduPlan {
    InPrimaryStub,
    Drop(String),
    KeepRaw(String),
    Verbatim,
    Uncompressed(String),
    Compress(ImageGeometry),
}

fn plan_hdu(
    index: usize,
    hdu: &SourceHdu,
    drop_set: &[String],
    raw_set: &[String],
    file_len: usize,
) -> Result<HduPlan> {
    let header = &hdu.header;
    let has_data = hdu.data_bytes > 0;
    if index == 0 && (header.get_i64("NAXIS").unwrap_or(0) == 0 || !has_data) {
        return Ok(HduPlan::InPrimaryStub);
    }
    let name = header.get("EXTNAME").map(normalize_extname);
    if let Some(name) = name.as_ref().filter(|name| drop_set.contains(name)) {
        return Ok(HduPlan::Drop(name.clone()));
    }
    let Some(data_end) = hdu.data_end else {
        return Err(truncation_error(index, hdu, file_len));
    };
    if let Some(name) = name.filter(|name| raw_set.contains(name)) {
        return Ok(HduPlan::KeepRaw(name));
    }
    if !has_data || is_compressed_image_hdu(header) || !is_image_extension(header) {
        return Ok(HduPlan::Verbatim);
    }
    Ok(match image_geometry(header)? {
        Some(geometry)
            if hdu
                .data_start
                .checked_add(geometry.bytes)
                .is_some_and(|end| end <= data_end) =>
        {
            HduPlan::Compress(geometry)
        }
        _ => HduPlan::Uncompressed(hdu_label(index, header)),
    })
}

fn compress_image(
    mmap: &[u8],
    writer: &mut BufWriter<File>,
    hdu: &SourceHdu,
    geometry: &ImageGeometry,
    mode: &CompressMode,
) -> Result<()> {
    let header = &hdu.header;
    let raw = hdu
        .data_start
        .checked_add(geometry.bytes)
        .and_then(|end| mmap.get(hdu.data_start..end))
        .with_context(|| format!("HDU data exceeds file size (EXTNAME={:?})", header.get("EXTNAME")))?;
    let (ncols, nrows) = (geometry.naxis1, geometry.naxis2);
    let plane_len = ncols * nrows;
    let bzero = header.get_f64("BZERO").unwrap_or(0.0);
    let bscale = header.get_f64("BSCALE").unwrap_or(1.0);

    if geometry.bitpix > 0 {
        let planes: Vec<Vec<i64>> = read_raw_ints_be(raw, geometry.bitpix)?
            .chunks_exact(plane_len)
            .map(<[i64]>::to_vec)
            .collect();
        let blank = header.get_i64("BLANK");
        return write_planes_lossless_int(
            writer, &planes, ncols, nrows, geometry.bitpix as i32, blank, bzero, bscale, Some(header),
        );
    }
    match mode {
        CompressMode::Lossless => {
            let plane_slices: Vec<&[u8]> = raw.chunks_exact(geometry.bytes / geometry.nplanes).collect();
            write_planes_gzip2_lossless(
                writer, &plane_slices, ncols, nrows, geometry.bitpix as i32, bzero, bscale, Some(header),
            )
        }
        CompressMode::Lossy { quantize_level } => {
            let planes = decode_pixels(raw, geometry.bitpix, bscale, bzero)
                .chunks_exact(plane_len)
                .map(|chunk| {
                    Array2::from_shape_vec((nrows, ncols), chunk.to_vec()).context("Failed to reshape plane")
                })
                .collect::<Result<Vec<_>>>()?;
            write_planes_quantized(writer, &planes, Some(header), *quantize_level)
        }
    }
}

fn is_same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

static PARTIAL_OUTPUTS: AtomicU64 = AtomicU64::new(0);

struct PartialOutput {
    path: PathBuf,
    committed: bool,
}

impl PartialOutput {
    fn create_beside(target: &Path) -> Result<(Self, File)> {
        let name = target
            .file_name()
            .context("The output path has no file name")?
            .to_string_lossy()
            .into_owned();
        let path = target.with_file_name(format!(
            ".{name}.{}-{}.partial",
            std::process::id(),
            PARTIAL_OUTPUTS.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .context("Failed to create output FITS file")?;
        Ok((Self { path, committed: false }, file))
    }

    fn commit(mut self, writer: BufWriter<File>, target: &Path) -> Result<()> {
        let file = writer
            .into_inner()
            .map_err(|e| e.into_error())
            .context("Failed to write the output FITS file")?;
        drop(file);
        std::fs::rename(&self.path, target)
            .with_context(|| format!("Failed to replace {}", target.display()))?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for PartialOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn write_compressed_mef(
    source_path: &str,
    output_path: &str,
    opts: &CompressOptions,
) -> Result<MefReport> {
    if let CompressMode::Lossy { quantize_level } = opts.mode {
        validate_quantize_level(quantize_level)?;
    }
    if is_same_file(source_path, output_path) {
        bail!("The output {output_path} is the source file itself; choose a different output file");
    }
    let file = File::open(source_path).with_context(|| format!("Failed to open {source_path}"))?;
    let mmap = read_file_bytes(&file)?;
    let hdus = scan_source_hdus(&mmap)?;

    let drop_set: Vec<String> = opts.drop_extnames.iter().map(|s| normalize_extname(s)).collect();
    let raw_set: Vec<String> = opts.raw_extnames.iter().map(|s| normalize_extname(s)).collect();
    let plans = hdus
        .iter()
        .enumerate()
        .map(|(index, hdu)| plan_hdu(index, hdu, &drop_set, &raw_set, mmap.len()))
        .collect::<Result<Vec<_>>>()?;
    let (Some(primary), Some(primary_plan)) = (hdus.first(), plans.first()) else {
        bail!("No HDUs found in source file");
    };

    let target = Path::new(output_path);
    let (partial, out_file) = PartialOutput::create_beside(target)?;
    let mut writer = BufWriter::with_capacity(4 * 1024 * 1024, out_file);
    let mut report = MefReport { dropped: Vec::new(), kept_raw: Vec::new(), uncompressed: Vec::new() };

    match primary_plan {
        HduPlan::InPrimaryStub | HduPlan::Drop(_) => {
            write_primary_hdu_stub(&mut writer, Some(&primary.header))?
        }
        HduPlan::Compress(_) => write_primary_hdu_stub(&mut writer, None)?,
        HduPlan::KeepRaw(_) | HduPlan::Verbatim | HduPlan::Uncompressed(_) => {}
    }

    for (hdu, plan) in hdus.iter().zip(&plans) {
        match plan {
            HduPlan::InPrimaryStub => {}
            HduPlan::Drop(name) => report.dropped.push(name.clone()),
            HduPlan::KeepRaw(name) => {
                report.kept_raw.push(name.clone());
                copy_verbatim(&mmap, &mut writer, hdu)?;
            }
            HduPlan::Verbatim => copy_verbatim(&mmap, &mut writer, hdu)?,
            HduPlan::Uncompressed(label) => {
                report.uncompressed.push(label.clone());
                copy_verbatim(&mmap, &mut writer, hdu)?;
            }
            HduPlan::Compress(geometry) => compress_image(&mmap, &mut writer, hdu, geometry, &opts.mode)?,
        }
    }

    partial.commit(writer, target)?;
    Ok(report)
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
        std::fs::write(path, synthetic_source_bytes()).unwrap();
    }

    fn synthetic_source_bytes() -> Vec<u8> {
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
        buf
    }

    #[test]
    fn write_compressed_mef_full_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("ab_mef_source.fits");
        let output_path = dir.path().join("ab_mef_output.fits");
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
    }

    #[test]
    fn write_compressed_mef_lossless_roundtrips_floats_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("ab_mef_lossless_source.fits");
        let output_path = dir.path().join("ab_mef_lossless_output.fits");
        let output_path2 = dir.path().join("ab_mef_lossless_output2.fits");
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
    }

    #[test]
    fn write_compressed_mef_drops_named_hdus() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("ab_mef_drop_source.fits");
        let output_path = dir.path().join("ab_mef_drop_output.fits");
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
    }

    #[test]
    fn write_compressed_mef_keeps_named_hdus_raw() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("ab_mef_raw_source.fits");
        let output_path = dir.path().join("ab_mef_raw_output.fits");
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
    }

    fn lossless() -> CompressOptions {
        CompressOptions { mode: CompressMode::Lossless, drop_extnames: vec![], raw_extnames: vec![] }
    }

    fn all_hdus(bytes: &[u8]) -> Vec<crate::infra::fits::reader::ParsedHdu> {
        let mut out = Vec::new();
        let mut off = 0usize;
        while off + BLOCK <= bytes.len() {
            let parsed = parse_header_at(bytes, off).unwrap();
            off = parsed.next_hdu_offset;
            out.push(parsed);
        }
        out
    }

    fn i64_data_block(values: &[i64]) -> Vec<u8> {
        let mut out: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        out.resize(out.len().next_multiple_of(BLOCK), 0);
        out
    }

    fn sci_extension(w: usize, h: usize) -> (Vec<u8>, Vec<f32>) {
        let sci: Vec<f32> = (0..w * h).map(|i| 50.0 + (i as f32 * 0.11).cos() * 4.0).collect();
        let mut bytes = header_block(&[
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'SCI     '".into()),
        ]);
        bytes.extend_from_slice(&f32_data_block(&sci));
        (bytes, sci)
    }

    #[test]
    fn a_source_whose_last_hdu_lacks_its_padding_is_copied_and_padded() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("unpadded.fits");
        let output_path = dir.path().join("unpadded_out.fits");
        let mut source = synthetic_source_bytes();
        source.truncate(source.len() - (BLOCK - 4));
        std::fs::write(&source_path, &source).unwrap();

        write_compressed_mef(source_path.to_str().unwrap(), output_path.to_str().unwrap(), &lossless()).unwrap();

        let out = std::fs::read(&output_path).unwrap();
        assert_eq!(out.len() % BLOCK, 0);
        let hdus = all_hdus(&out);
        let aux = hdus.last().unwrap();
        assert_eq!(aux.header.get("EXTNAME"), Some("AUX"));
        assert_eq!(&out[aux.data_start..aux.data_start + 4], &42i32.to_be_bytes());
        assert!(out[aux.data_start + 4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn a_truncated_source_is_refused_before_the_output_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("truncated.fits");
        let output_path = dir.path().join("truncated_out.fits");
        let full = synthetic_source_bytes();
        let source_hdus = all_hdus(&full);
        let cube = &source_hdus[3];
        assert_eq!(cube.header.get("EXTNAME"), Some("CUBE"));
        std::fs::write(&source_path, &full[..cube.data_start + 100]).unwrap();
        std::fs::write(&output_path, b"previous export").unwrap();

        for opts in [lossless(), CompressOptions { mode: CompressMode::Lossy { quantize_level: 16.0 }, drop_extnames: vec![], raw_extnames: vec![] }] {
            let err = write_compressed_mef(source_path.to_str().unwrap(), output_path.to_str().unwrap(), &opts)
                .unwrap_err();
            assert!(err.to_string().contains("truncated"), "{err}");
            assert!(err.to_string().contains("CUBE"), "{err}");
            assert_eq!(std::fs::read(&output_path).unwrap(), b"previous export");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2, "no partial output may be left behind");
        }
    }

    #[test]
    fn a_truncated_hdu_the_user_drops_is_skipped_and_the_rest_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("truncated_aux.fits");
        let output_path = dir.path().join("truncated_aux_out.fits");
        let full = synthetic_source_bytes();
        let aux = all_hdus(&full).pop().unwrap();
        assert_eq!(aux.header.get("EXTNAME"), Some("AUX"));
        std::fs::write(&source_path, &full[..aux.data_start + 2]).unwrap();

        let drop_aux = CompressOptions {
            mode: CompressMode::Lossless,
            drop_extnames: vec!["aux".into()],
            raw_extnames: vec![],
        };
        let report =
            write_compressed_mef(source_path.to_str().unwrap(), output_path.to_str().unwrap(), &drop_aux).unwrap();
        assert_eq!(report.dropped, vec!["AUX".to_string()]);
        let out = std::fs::read(&output_path).unwrap();
        let names: Vec<String> =
            all_hdus(&out).iter().filter_map(|h| h.header.get("EXTNAME").map(str::to_string)).collect();
        assert_eq!(names, ["SCI", "FLAGS", "CUBE"]);

        let keep_aux_raw = CompressOptions {
            mode: CompressMode::Lossless,
            drop_extnames: vec![],
            raw_extnames: vec!["AUX".into()],
        };
        for opts in [lossless(), keep_aux_raw] {
            let err = write_compressed_mef(source_path.to_str().unwrap(), output_path.to_str().unwrap(), &opts)
                .unwrap_err();
            assert!(err.to_string().contains("truncated"), "{err}");
            assert!(err.to_string().contains("AUX"), "{err}");
            assert_eq!(std::fs::read(&output_path).unwrap(), out);
        }
    }

    #[test]
    fn an_image_hdu_the_codec_cannot_compress_is_copied_verbatim_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("bitpix64.fits");
        let output_path = dir.path().join("bitpix64_out.fits");
        let (w, h) = (4usize, 3usize);
        let mut source = header_block(&[
            ("SIMPLE", "T".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "0".into()),
            ("EXTEND", "T".into()),
        ]);
        source.extend_from_slice(&header_block(&[
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "64".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'TIME    '".into()),
        ]));
        source.extend_from_slice(&i64_data_block(
            &(0..(w * h) as i64).map(|i| 1_700_000_000_000 + i).collect::<Vec<_>>(),
        ));
        let (sci_bytes, sci) = sci_extension(24, 16);
        source.extend_from_slice(&sci_bytes);
        std::fs::write(&source_path, &source).unwrap();

        let report =
            write_compressed_mef(source_path.to_str().unwrap(), output_path.to_str().unwrap(), &lossless()).unwrap();
        assert_eq!(report.uncompressed, vec!["TIME".to_string()]);
        assert!(report.kept_raw.is_empty());

        let out = std::fs::read(&output_path).unwrap();
        let out_hdus = all_hdus(&out);
        let src_hdus = all_hdus(&source);
        assert_eq!(out_hdus.len(), 3);
        assert_eq!(
            &out[out_hdus[1].header_start..out_hdus[1].next_hdu_offset],
            &source[src_hdus[1].header_start..src_hdus[1].next_hdu_offset]
        );
        assert_eq!(out_hdus[2].header.get("ZCMPTYPE"), Some("GZIP_2"));
        let decoded = decode_compressed_image(&out, &out_hdus[2].header, out_hdus[2].data_start).unwrap();
        assert_eq!(decoded.iter().copied().collect::<Vec<_>>(), sci);
    }

    #[test]
    fn a_primary_image_is_compressed_once_without_primary_only_cards_in_the_table() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("primary_image.fits");
        let output_path = dir.path().join("primary_image_out.fits");
        let (w, h) = (24usize, 16usize);
        let pixels: Vec<f32> = (0..w * h).map(|i| 1000.0 + (i as f32 * 0.3).sin() * 20.0).collect();
        let mut source = header_block(&[
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("EXTEND", "T".into()),
            ("OBJECT", "'M16     '".into()),
            ("CRPIX1", "12.5".into()),
            ("CHECKSUM", "'9aHCA7GB9aGBA7GB'".into()),
            ("DATASUM", "'3141592653'".into()),
        ]);
        source.extend_from_slice(&f32_data_block(&pixels));
        std::fs::write(&source_path, &source).unwrap();

        let report =
            write_compressed_mef(source_path.to_str().unwrap(), output_path.to_str().unwrap(), &lossless()).unwrap();
        assert!(report.uncompressed.is_empty());

        let out = std::fs::read(&output_path).unwrap();
        let hdus = all_hdus(&out);
        assert_eq!(hdus.len(), 2);
        let stub = &hdus[0].header;
        assert_eq!(stub.get_i64("NAXIS"), Some(0));
        assert_eq!(stub.get("OBJECT"), None, "metadata must not be duplicated into the stub");
        assert_eq!(stub.get("CRPIX1"), None);

        let table = &hdus[1].header;
        for key in ["SIMPLE", "EXTEND", "CHECKSUM", "DATASUM"] {
            assert_eq!(table.get(key), None, "{key} must not reach the compressed BINTABLE");
        }
        assert_eq!(table.get("XTENSION"), Some("BINTABLE"));
        assert_eq!(table.get("OBJECT"), Some("M16"));
        assert_eq!(table.get_f64("CRPIX1"), Some(12.5));
        let decoded = decode_compressed_image(&out, table, hdus[1].data_start).unwrap();
        assert_eq!(decoded.iter().copied().collect::<Vec<_>>(), pixels);
    }

    #[test]
    fn a_primary_image_that_cannot_be_compressed_stays_the_primary() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("spectrum.fits");
        let output_path = dir.path().join("spectrum_out.fits");
        let spectrum: Vec<f32> = (0..10).map(|i| i as f32 * 1.5).collect();
        let mut source = header_block(&[
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "1".into()),
            ("NAXIS1", "10".into()),
            ("EXTEND", "T".into()),
        ]);
        source.extend_from_slice(&f32_data_block(&spectrum));
        let (sci_bytes, _) = sci_extension(24, 16);
        source.extend_from_slice(&sci_bytes);
        std::fs::write(&source_path, &source).unwrap();

        let report =
            write_compressed_mef(source_path.to_str().unwrap(), output_path.to_str().unwrap(), &lossless()).unwrap();
        assert_eq!(report.uncompressed, vec!["HDU0".to_string()]);

        let out = std::fs::read(&output_path).unwrap();
        let hdus = all_hdus(&out);
        assert_eq!(hdus.len(), 2);
        let src_primary = parse_header_at(&source, 0).unwrap();
        assert_eq!(&out[..hdus[0].next_hdu_offset], &source[..src_primary.next_hdu_offset]);
        assert_eq!(hdus[1].header.get("XTENSION"), Some("BINTABLE"));
        assert_eq!(hdus[1].header.get("EXTNAME"), Some("SCI"));
    }

    #[test]
    fn a_write_that_fails_after_the_output_was_started_leaves_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.fits");
        write_synthetic_source(&source_path);
        let occupied = dir.path().join("occupied");
        std::fs::create_dir(&occupied).unwrap();
        std::fs::write(occupied.join("keep.txt"), b"kept").unwrap();

        let err = write_compressed_mef(source_path.to_str().unwrap(), occupied.to_str().unwrap(), &lossless())
            .unwrap_err();
        assert!(err.to_string().contains("Failed to replace"), "{err}");
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["occupied", "source.fits"]);
        assert_eq!(std::fs::read(occupied.join("keep.txt")).unwrap(), b"kept");
    }

    #[test]
    fn the_source_itself_is_refused_as_the_output() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("self.fits");
        write_synthetic_source(&source_path);
        let before = std::fs::read(&source_path).unwrap();
        let same_file = dir.path().join(".").join("self.fits");

        for opts in [lossless(), CompressOptions { mode: CompressMode::Lossy { quantize_level: 16.0 }, drop_extnames: vec![], raw_extnames: vec![] }] {
            let err = write_compressed_mef(source_path.to_str().unwrap(), same_file.to_str().unwrap(), &opts)
                .unwrap_err();
            assert!(err.to_string().contains("source file"), "{err}");
            assert_eq!(std::fs::read(&source_path).unwrap(), before);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    fn raw_card(header_bytes: &[u8], key: &str) -> String {
        header_bytes
            .chunks_exact(80)
            .map(|card| String::from_utf8_lossy(card).into_owned())
            .find(|card| card[..8].trim_end() == key)
            .unwrap_or_else(|| panic!("card {key} not found"))
    }

    fn assert_string_card(header_bytes: &[u8], key: &str) {
        let card = raw_card(header_bytes, key);
        assert_eq!(card.as_bytes()[10], b'\'', "{key} was a string in the source: {card:?}");
    }

    fn assert_literal_card(header_bytes: &[u8], key: &str, text: &str) {
        let card = raw_card(header_bytes, key);
        assert_eq!(&card[30 - text.len()..30], text, "{key} was a number in the source: {card:?}");
    }

    #[test]
    fn compressed_output_keeps_the_string_or_number_type_of_every_source_card() {
        let dir = tempfile::tempdir().unwrap();
        let (w, h) = (24usize, 16usize);
        let pixels: Vec<f32> = (0..w * h).map(|i| 10.0 + (i as f32 * 0.2).sin()).collect();

        let extension_source = dir.path().join("typed_mef.fits");
        let mut source = header_block(&[
            ("SIMPLE", "T".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "0".into()),
            ("EXTEND", "T".into()),
            ("SEQ_ID", "'1       '".into()),
            ("VERSION", "'6.4     '".into()),
            ("FLAGSTR", "'T       '".into()),
            ("CCD-TEMP", "-010.00".into()),
        ]);
        source.extend_from_slice(&header_block(&[
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'SCI     '".into()),
            ("EXPOSURE", "'2       '".into()),
            ("OBSNUM", "007".into()),
            ("EXPTIME", "12.5".into()),
        ]));
        source.extend_from_slice(&f32_data_block(&pixels));
        std::fs::write(&extension_source, &source).unwrap();

        let lossy = CompressOptions { mode: CompressMode::Lossy { quantize_level: 16.0 }, drop_extnames: vec![], raw_extnames: vec![] };
        for opts in [lossless(), lossy] {
            let output = dir.path().join("typed_mef_out.fits");
            write_compressed_mef(extension_source.to_str().unwrap(), output.to_str().unwrap(), &opts).unwrap();
            let out = std::fs::read(&output).unwrap();
            let hdus = all_hdus(&out);
            let stub = &out[hdus[0].header_start..hdus[0].data_start];
            for key in ["SEQ_ID", "VERSION", "FLAGSTR"] {
                assert_string_card(stub, key);
            }
            assert_literal_card(stub, "CCD-TEMP", "-010.00");
            assert_eq!(hdus[0].header.get("SEQ_ID"), Some("1"));
            assert_eq!(hdus[0].header.get("FLAGSTR"), Some("T"));

            let table = &out[hdus[1].header_start..hdus[1].data_start];
            assert_string_card(table, "EXPOSURE");
            assert_string_card(table, "EXTNAME");
            assert_literal_card(table, "OBSNUM", "007");
            assert_literal_card(table, "EXPTIME", "12.5");
            assert_literal_card(table, "ZIMAGE", "T");
        }

        let primary_source = dir.path().join("typed_primary.fits");
        let mut source = header_block(&[
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", w.to_string()),
            ("NAXIS2", h.to_string()),
            ("EXTEND", "T".into()),
            ("SEQ_ID", "'1       '".into()),
            ("CCD-TEMP", "-010.00".into()),
        ]);
        source.extend_from_slice(&f32_data_block(&pixels));
        std::fs::write(&primary_source, &source).unwrap();
        let output = dir.path().join("typed_primary_out.fits");
        write_compressed_mef(primary_source.to_str().unwrap(), output.to_str().unwrap(), &lossless()).unwrap();
        let out = std::fs::read(&output).unwrap();
        let hdus = all_hdus(&out);
        let table = &out[hdus[1].header_start..hdus[1].data_start];
        assert_string_card(table, "SEQ_ID");
        assert_literal_card(table, "CCD-TEMP", "-010.00");
    }

    #[test]
    fn a_lossy_level_that_is_not_positive_and_finite_is_refused_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("q_source.fits");
        let output_path = dir.path().join("q_out.fits");
        write_synthetic_source(&source_path);
        for quantize_level in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let opts = CompressOptions { mode: CompressMode::Lossy { quantize_level }, drop_extnames: vec![], raw_extnames: vec![] };
            let err = write_compressed_mef(source_path.to_str().unwrap(), output_path.to_str().unwrap(), &opts)
                .unwrap_err();
            assert!(err.to_string().contains("quantize_level"), "{err}");
            assert!(!output_path.exists());
        }
    }
}
