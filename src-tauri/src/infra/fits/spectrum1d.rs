use std::fs::File;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::core::analysis::spectrum1d::{Precision, Spectrum1d, Wcs1dArrays};
use crate::core::analysis::x1d::{
    extract1d_tables, pick_table, read_x1d_spectrum, X1dTableRef, EXTNAME_EXTRACT1D,
};
use crate::core::astrometry::spectral::{is_spectral_ctype, wavelength_unit_to_um};
use crate::infra::fits::file_bytes::read_file_bytes;
use crate::infra::fits::gzip::resolve_gzip;
use crate::infra::fits::reader::{
    decode_pixels_blank, decode_pixels_int, parse_header_at, IntPixels, ParsedHdu,
};
use crate::infra::image_source::{is_dq_name, is_err_name};
use crate::types::constants::BLOCK_SIZE;
use crate::types::header::{parse_fits_float, HduHeader};

const MIN_SPECTRUM_CHANNELS: i64 = 2;
const IRAF_LINEAR_CTYPE: &str = "LINEAR";
const MULTISPEC_CTYPE: &str = "MULTISPE";
const MAX_WAT_CARDS: usize = 999;
const WAT1_WAVELENGTH_LABEL: &str = "label=wavelength";
const SUPPORTED_BITPIX: [i64; 6] = [8, 16, 32, 64, -32, -64];
const F32_WIDENED_BITPIX: i64 = -64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Spectrum1dKind {
    X1d { hdu: usize },
    Wcs1d { hdu: usize },
}

pub fn classify(file: &File) -> Result<Option<Spectrum1dKind>> {
    classify_with(file, &extract1d_tables(file)?)
}

fn classify_with(file: &File, tables: &[X1dTableRef]) -> Result<Option<Spectrum1dKind>> {
    if !tables.is_empty() {
        let table = pick_table(tables, None).map_err(anyhow::Error::msg)?;
        return Ok(Some(Spectrum1dKind::X1d { hdu: table.hdu }));
    }
    let bytes = read_file_bytes(file)?;
    let hdus = scan_headers(&bytes)?;
    Ok(hdus
        .iter()
        .position(|parsed| is_spectrum_array(&parsed.header))
        .map(|hdu| Spectrum1dKind::Wcs1d { hdu }))
}

pub fn read_1d_array(path: &str, hdu: usize) -> Result<(HduHeader, Wcs1dArrays)> {
    let (readable, _decompressed) = resolve_gzip(Path::new(path))?;
    let file = File::open(&readable).with_context(|| format!("Failed to open {path}"))?;
    let bytes = read_file_bytes(&file)?;
    let hdus = scan_headers(&bytes)?;
    let chosen = hdus
        .get(hdu)
        .with_context(|| format!("HDU {hdu} out of range (file has {} HDUs)", hdus.len()))?;
    check_1d_array(chosen, hdu)?;
    let (flux, flux_precision) = decode_values(&bytes, chosen, &label(chosen, hdu))?;
    let err = companion(&hdus, hdu, is_err_name)
        .map(|i| decode_values(&bytes, &hdus[i], &label(&hdus[i], i)))
        .transpose()?;
    let dq = companion(&hdus, hdu, is_dq_name)
        .map(|i| decode_dq(&bytes, &hdus[i], &label(&hdus[i], i)))
        .transpose()?;
    let widened = flux_precision == Precision::F32Widened
        || err
            .as_ref()
            .is_some_and(|(_, p)| *p == Precision::F32Widened);
    let header = if hdu == 0 {
        chosen.header.clone()
    } else {
        hdus[0].header.merge_with(&chosen.header)
    };
    Ok((
        header,
        Wcs1dArrays {
            flux,
            precision: if widened {
                Precision::F32Widened
            } else {
                Precision::F64
            },
            err: err.map(|(values, _)| values),
            dq,
        },
    ))
}

pub fn load_spectrum1d(path: &str, hdu: Option<usize>) -> Result<Spectrum1d> {
    let (readable, _decompressed) = resolve_gzip(Path::new(path))?;
    let file = File::open(&readable).with_context(|| format!("Failed to open {path}"))?;
    let tables = extract1d_tables(&file)?;
    let kind = match hdu {
        Some(n) if tables.iter().any(|t| t.hdu == n) => Spectrum1dKind::X1d { hdu: n },
        Some(n) => Spectrum1dKind::Wcs1d { hdu: n },
        None => classify_with(&file, &tables)?.with_context(|| {
            format!("{path} holds no 1D spectrum: no {EXTNAME_EXTRACT1D} table and no 1D array with a spectral axis")
        })?,
    };
    match kind {
        Spectrum1dKind::X1d { hdu } => {
            Ok(Spectrum1d::from_x1d(read_x1d_spectrum(path, Some(hdu))?))
        }
        Spectrum1dKind::Wcs1d { hdu } => {
            let (header, arrays) = read_1d_array(path, hdu)?;
            Spectrum1d::from_wcs1d(path, hdu, &header, arrays).map_err(anyhow::Error::msg)
        }
    }
}

fn scan_headers(bytes: &[u8]) -> Result<Vec<ParsedHdu>> {
    let mut hdus = Vec::new();
    let mut offset = 0usize;
    while offset
        .checked_add(BLOCK_SIZE)
        .is_some_and(|end| end <= bytes.len())
    {
        match parse_header_at(bytes, offset) {
            Ok(parsed) => {
                offset = parsed.next_hdu_offset;
                hdus.push(parsed);
            }
            Err(_) if !hdus.is_empty() => break,
            Err(e) => return Err(e),
        }
    }
    if hdus.is_empty() {
        bail!("no FITS header: the file is shorter than one {BLOCK_SIZE}-byte block");
    }
    Ok(hdus)
}

fn extname(header: &HduHeader) -> Option<&str> {
    header
        .get("EXTNAME")
        .map(str::trim)
        .filter(|n| !n.is_empty())
}

fn label(parsed: &ParsedHdu, index: usize) -> String {
    match extname(&parsed.header) {
        Some(name) => format!("HDU {index} ({name})"),
        None => format!("HDU {index}"),
    }
}

fn is_image_hdu(header: &HduHeader) -> bool {
    header
        .get("XTENSION")
        .is_none_or(|x| x.trim().eq_ignore_ascii_case("IMAGE"))
}

fn is_1d_image(header: &HduHeader) -> bool {
    is_image_hdu(header) && header.get_i64("NAXIS") == Some(1)
}

fn finite_card(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|v| v.is_finite())
}

fn has_linear_dispersion(header: &HduHeader) -> bool {
    finite_card(header, "CRVAL1").is_some()
        && ["CDELT1", "CD1_1"]
            .iter()
            .any(|key| finite_card(header, key).is_some_and(|step| step != 0.0))
}

fn has_wavelength_unit(header: &HduHeader) -> bool {
    header
        .get("CUNIT1")
        .is_some_and(|unit| wavelength_unit_to_um(unit.trim()).is_some())
}

fn wat1_text(header: &HduHeader) -> Option<String> {
    let joined: String = (1..=MAX_WAT_CARDS)
        .map_while(|n| header.get(&format!("WAT1_{n:03}")))
        .collect();
    (!joined.is_empty()).then(|| joined.split_whitespace().collect::<String>().to_lowercase())
}

fn has_spectral_evidence(header: &HduHeader) -> bool {
    let ctype = header
        .get("CTYPE1")
        .map(|c| c.trim().to_ascii_uppercase())
        .filter(|c| !c.is_empty() && c != IRAF_LINEAR_CTYPE);
    match ctype {
        Some(ctype) => ctype.starts_with(MULTISPEC_CTYPE) || is_spectral_ctype(&ctype),
        None => match wat1_text(header) {
            Some(wat) => wat.contains(WAT1_WAVELENGTH_LABEL),
            None => has_wavelength_unit(header) || has_linear_dispersion(header),
        },
    }
}

fn is_spectrum_array(header: &HduHeader) -> bool {
    is_1d_image(header)
        && header
            .get_i64("NAXIS1")
            .is_some_and(|n| n >= MIN_SPECTRUM_CHANNELS)
        && !extname(header).is_some_and(|n| is_err_name(n) || is_dq_name(n))
        && has_spectral_evidence(header)
}

fn check_1d_array(parsed: &ParsedHdu, index: usize) -> Result<()> {
    let header = &parsed.header;
    if !is_image_hdu(header) {
        bail!(
            "HDU {index} is a {} extension, not a 1D array",
            header.get("XTENSION").map(str::trim).unwrap_or_default()
        );
    }
    match header.get_i64("NAXIS") {
        Some(1) => Ok(()),
        Some(n) => bail!("HDU {index} has NAXIS = {n}, not a 1D array"),
        None => bail!("HDU {index} has no NAXIS card"),
    }
}

fn same_extver(chosen: Option<i64>, other: Option<i64>) -> bool {
    match (chosen, other) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

fn companion(hdus: &[ParsedHdu], chosen: usize, is_named: fn(&str) -> bool) -> Option<usize> {
    let extver = hdus[chosen].header.get_i64("EXTVER");
    hdus.iter()
        .enumerate()
        .find(|(i, parsed)| {
            *i != chosen
                && is_1d_image(&parsed.header)
                && extname(&parsed.header).is_some_and(is_named)
                && same_extver(extver, parsed.header.get_i64("EXTVER"))
        })
        .map(|(i, _)| i)
}

fn scale_card(header: &HduHeader, key: &str, default: f64, label: &str) -> Result<f64> {
    match header.get(key).map(str::trim).filter(|raw| !raw.is_empty()) {
        None => Ok(default),
        Some(raw) => parse_fits_float(raw).with_context(|| {
            format!("{label}: {key} = '{raw}' is not a number; the values cannot be scaled")
        }),
    }
}

fn data_unit<'a>(bytes: &'a [u8], parsed: &ParsedHdu, label: &str) -> Result<(&'a [u8], i64)> {
    let header = &parsed.header;
    let bitpix = header
        .get_i64("BITPIX")
        .filter(|b| SUPPORTED_BITPIX.contains(b))
        .with_context(|| {
            format!(
                "{label}: BITPIX '{}' is not a FITS pixel type",
                header.get("BITPIX").unwrap_or_default().trim()
            )
        })?;
    let len = header
        .get_i64("NAXIS1")
        .and_then(|n| usize::try_from(n).ok())
        .with_context(|| format!("{label}: NAXIS1 is missing or negative"))?;
    let size = len
        .checked_mul(bitpix.unsigned_abs() as usize / 8)
        .with_context(|| format!("{label}: the data unit size overflows"))?;
    let end = parsed
        .data_start
        .checked_add(size)
        .filter(|&end| end <= bytes.len())
        .with_context(|| {
            format!("{label}: the data unit ({size} bytes) ends past the end of the file")
        })?;
    Ok((&bytes[parsed.data_start..end], bitpix))
}

fn decode_values(bytes: &[u8], parsed: &ParsedHdu, label: &str) -> Result<(Vec<f64>, Precision)> {
    let header = &parsed.header;
    let (raw, bitpix) = data_unit(bytes, parsed, label)?;
    let bzero = scale_card(header, "BZERO", 0.0, label)?;
    let bscale = scale_card(header, "BSCALE", 1.0, label)?;
    let values = decode_pixels_blank(raw, bitpix, bscale, bzero, header.get_i64("BLANK"));
    let precision = if bitpix == F32_WIDENED_BITPIX {
        Precision::F32Widened
    } else {
        Precision::F64
    };
    Ok((values.into_iter().map(f64::from).collect(), precision))
}

fn decode_dq(bytes: &[u8], parsed: &ParsedHdu, label: &str) -> Result<Vec<u32>> {
    let header = &parsed.header;
    let (raw, bitpix) = data_unit(bytes, parsed, label)?;
    let bzero = scale_card(header, "BZERO", 0.0, label)?;
    let bscale = scale_card(header, "BSCALE", 1.0, label)?;
    let pixels = decode_pixels_int(raw, bitpix, bscale, bzero)
        .with_context(|| format!("{label}: the DQ array must hold integer flags"))?;
    Ok(match pixels {
        IntPixels::U32(bits) => bits,
        IntPixels::I32(bits) => bits.into_iter().map(|b| b as u32).collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::core::analysis::spectrum1d::{Precision, Spectrum1dSource, WavelengthFrame};
    use crate::core::analysis::x1d::read_x1d_spectrum;
    use crate::core::analysis::x1d::test_fixtures::write_x1d_fixture;
    use crate::core::astrometry::gwcs::test_support::documents_fits_dir;
    use crate::core::imaging::dq_flags::DqTable;
    use crate::infra::fits::reader::test_fixtures::{
        empty_primary_cards, ramp_f32, write_raw_hdus, write_test_mef, HduData, TestHdu,
    };
    use crate::infra::fits::reader::{extract_image_mmap, ONE_D_ARRAY_VERDICT};

    const LEN: usize = 24;
    const DQ_BZERO: i64 = 2_147_483_648;
    const WAVE_ANGSTROM: [(&str, &str); 5] = [
        ("CTYPE1", "'WAVE    '"),
        ("CUNIT1", "'Angstrom'"),
        ("CRVAL1", "6500.0"),
        ("CDELT1", "0.5"),
        ("CRPIX1", "1.0"),
    ];
    const WAVE_MICRON: [(&str, &str); 5] = [
        ("CTYPE1", "'WAVE    '"),
        ("CUNIT1", "'um      '"),
        ("CRVAL1", "2.0"),
        ("CDELT1", "0.001"),
        ("CRPIX1", "1.0"),
    ];
    const IRAF_LINEAR: [(&str, &str); 8] = [
        ("CTYPE1", "'LINEAR  '"),
        ("WAT0_001", "'system=equispec'"),
        (
            "WAT1_001",
            "'wtype=linear label=Wavelength units=angstroms'",
        ),
        ("DC-FLAG", "0"),
        ("CRVAL1", "4000.0"),
        ("CDELT1", "2.5"),
        ("CD1_1", "2.5"),
        ("CRPIX1", "1.0"),
    ];

    type RawHdu = (Vec<(&'static str, String)>, Vec<u8>);
    type Cards = Vec<(&'static str, &'static str)>;

    fn array_hdu(
        primary: bool,
        bitpix: i64,
        len: usize,
        extra: &[(&'static str, &str)],
        data: Vec<u8>,
    ) -> RawHdu {
        let mut cards: Vec<(&'static str, String)> = if primary {
            vec![("SIMPLE", "T".into())]
        } else {
            vec![("XTENSION", "'IMAGE   '".into())]
        };
        cards.push(("BITPIX", bitpix.to_string()));
        cards.push(("NAXIS", "1".into()));
        cards.push(("NAXIS1", len.to_string()));
        if primary {
            cards.push(("EXTEND", "T".into()));
        } else {
            cards.push(("PCOUNT", "0".into()));
            cards.push(("GCOUNT", "1".into()));
        }
        cards.extend(extra.iter().map(|(k, v)| (*k, v.to_string())));
        (cards, data)
    }

    fn with(
        base: &[(&'static str, &'static str)],
        extra: &[(&'static str, &'static str)],
    ) -> Cards {
        base.iter().chain(extra).copied().collect()
    }

    fn f32_bytes(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn f64_bytes(values: &[f64]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn i16_bytes(values: &[i16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn i32_bytes(values: &[i32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn flux() -> Vec<f32> {
        (0..LEN).map(|i| 100.0 + 0.25 * i as f32).collect()
    }

    fn err() -> Vec<f32> {
        (0..LEN).map(|i| 1.0 + 0.125 * i as f32).collect()
    }

    fn widen(values: &[f32]) -> Vec<f64> {
        values.iter().map(|&v| f64::from(v)).collect()
    }

    fn write(dir: &Path, name: &str, hdus: &[RawHdu]) -> PathBuf {
        let path = dir.join(name);
        write_raw_hdus(&path, hdus);
        path
    }

    fn kind_of(path: &Path) -> Option<Spectrum1dKind> {
        classify(&File::open(path).unwrap()).unwrap()
    }

    fn text(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    fn sci_err_file(dir: &Path) -> PathBuf {
        let sci = with(
            &WAVE_ANGSTROM,
            &[("EXTNAME", "'SCI     '"), ("BUNIT", "'erg/s/cm2/A'")],
        );
        write(
            dir,
            "sci_err_1d.fits",
            &[
                array_hdu(true, -32, LEN, &sci, f32_bytes(&flux())),
                array_hdu(
                    false,
                    -32,
                    LEN,
                    &[("EXTNAME", "'ERR     '")],
                    f32_bytes(&err()),
                ),
            ],
        )
    }

    #[test]
    fn sci_and_err_1d_file_classifies_as_wcs1d_at_hdu_0_and_reads_both_arrays() {
        let dir = tempfile::tempdir().unwrap();
        let path = sci_err_file(dir.path());
        assert_eq!(kind_of(&path), Some(Spectrum1dKind::Wcs1d { hdu: 0 }));

        let (header, arrays) = read_1d_array(text(&path), 0).unwrap();
        assert_eq!(arrays.flux, widen(&flux()));
        assert_eq!(arrays.err, Some(widen(&err())));
        assert_eq!(arrays.dq, None);
        assert_eq!(arrays.precision, Precision::F64);
        assert_eq!(header.get("CTYPE1"), Some("WAVE"));

        let s = load_spectrum1d(text(&path), None).unwrap();
        assert_eq!(
            s.source,
            Spectrum1dSource::Wcs1d {
                path: text(&path).to_string(),
                hdu: 0
            }
        );
        assert_eq!(s.err, Some(widen(&err())));
        assert_eq!(s.flux_unit.as_deref(), Some("erg/s/cm2/A"));
        assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
        assert!(
            (s.axis.values[2] - 6501.0e-4).abs() < 1e-12,
            "{}",
            s.axis.values[2]
        );
    }

    #[test]
    fn image_ingest_of_a_1d_spectrum_fails_with_the_one_d_array_verdict_the_frontend_matches() {
        assert_eq!(ONE_D_ARRAY_VERDICT, "1D array, not an image");
        let dir = tempfile::tempdir().unwrap();
        let path = sci_err_file(dir.path());
        let message = match extract_image_mmap(&File::open(&path).unwrap()) {
            Ok(_) => panic!("a 1D spectrum must not load as an image"),
            Err(e) => format!("{e:#}"),
        };
        assert!(
            message.contains(&format!("HDU 0 (SCI): {ONE_D_ARRAY_VERDICT};")),
            "{message}"
        );
        assert_eq!(kind_of(&path), Some(Spectrum1dKind::Wcs1d { hdu: 0 }));
    }

    #[test]
    fn iraf_linear_wat1_file_classifies_as_wcs1d_and_opens_as_angstrom_air() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "iraf.fits",
            &[array_hdu(true, -32, LEN, &IRAF_LINEAR, f32_bytes(&flux()))],
        );
        assert_eq!(kind_of(&path), Some(Spectrum1dKind::Wcs1d { hdu: 0 }));

        let s = load_spectrum1d(text(&path), None).unwrap();
        assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
        assert_eq!(s.axis.ctype, "LINEAR");
        assert_eq!(s.axis.header_unit, "angstroms");
        assert!(
            (s.axis.values[0] - 0.4).abs() < 1e-12,
            "{}",
            s.axis.values[0]
        );
        assert!(
            (s.axis.values[4] - 0.401).abs() < 1e-12,
            "{}",
            s.axis.values[4]
        );
        assert_eq!(s.flux, widen(&flux()));
    }

    #[test]
    fn bitpix_minus_64_array_is_decoded_through_f32_and_reported_as_widened() {
        let dir = tempfile::tempdir().unwrap();
        let values: Vec<f64> = (0..LEN).map(|i| 1.0 + 1e-10 * i as f64).collect();
        let path = write(
            dir.path(),
            "f64.fits",
            &[array_hdu(
                true,
                -64,
                LEN,
                &WAVE_ANGSTROM,
                f64_bytes(&values),
            )],
        );
        assert_eq!(kind_of(&path), Some(Spectrum1dKind::Wcs1d { hdu: 0 }));

        let (_, arrays) = read_1d_array(text(&path), 0).unwrap();
        assert_eq!(arrays.precision, Precision::F32Widened);
        let through_f32: Vec<f64> = values.iter().map(|&v| f64::from(v as f32)).collect();
        assert_eq!(arrays.flux, through_f32);

        let s = load_spectrum1d(text(&path), None).unwrap();
        assert_eq!(s.precision, Precision::F32Widened);
        assert!(
            s.notes.iter().any(|n| n.contains("BITPIX = -64")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn a_widened_err_companion_marks_the_document_as_widened() {
        let dir = tempfile::tempdir().unwrap();
        let errors: Vec<f64> = (0..LEN).map(|i| 0.5 + 1e-9 * i as f64).collect();
        let path = write(
            dir.path(),
            "f32_sci_f64_err.fits",
            &[
                array_hdu(true, -32, LEN, &WAVE_ANGSTROM, f32_bytes(&flux())),
                array_hdu(
                    false,
                    -64,
                    LEN,
                    &[("EXTNAME", "'ERR     '")],
                    f64_bytes(&errors),
                ),
            ],
        );
        let (_, arrays) = read_1d_array(text(&path), 0).unwrap();
        assert_eq!(arrays.precision, Precision::F32Widened);
        assert_eq!(
            arrays.err,
            Some(errors.iter().map(|&v| f64::from(v as f32)).collect())
        );
    }

    #[test]
    fn two_d_image_classifies_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.fits");
        write_test_mef(
            &path,
            &[],
            &[TestHdu {
                extname: Some("SCI"),
                extver: None,
                cols: 8,
                rows: 6,
                data: HduData::F32(ramp_f32(8, 6)),
                extra_cards: vec![],
            }],
        );
        assert_eq!(kind_of(&path), None);
    }

    #[test]
    fn eight_hdu_image_mef_still_classifies_as_none_and_refuses_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cal.fits");
        let names = [
            "SCI",
            "ERR",
            "DQ",
            "AREA",
            "VAR_POISSON",
            "VAR_RNOISE",
            "VAR_FLAT",
        ];
        let hdus: Vec<TestHdu> = names
            .iter()
            .map(|&name| TestHdu {
                extname: Some(name),
                extver: Some(1),
                cols: 6,
                rows: 4,
                data: if name == "DQ" {
                    HduData::I32(vec![0; 24])
                } else {
                    HduData::F32(ramp_f32(6, 4))
                },
                extra_cards: vec![],
            })
            .collect();
        write_test_mef(&path, &[("TELESCOP", "'JWST'".into())], &hdus);
        assert_eq!(kind_of(&path), None);

        let none = format!("{:#}", load_spectrum1d(text(&path), None).unwrap_err());
        assert!(none.contains("no 1D spectrum"), "{none}");
        let not_1d = format!("{:#}", load_spectrum1d(text(&path), Some(1)).unwrap_err());
        assert!(
            not_1d.contains("HDU 1") && not_1d.contains("NAXIS = 2"),
            "{not_1d}"
        );
        let out_of_range = format!("{:#}", read_1d_array(text(&path), 99).unwrap_err());
        assert!(
            out_of_range.contains("HDU 99") && out_of_range.contains("8 HDUs"),
            "{out_of_range}"
        );
    }

    #[test]
    fn one_d_arrays_without_a_wavelength_dispersion_classify_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [(&str, Cards, usize); 5] = [
            ("bare.fits", vec![], LEN),
            (
                "time.fits",
                vec![
                    ("CTYPE1", "'TIME    '"),
                    ("CUNIT1", "'s       '"),
                    ("CRVAL1", "0.0"),
                    ("CDELT1", "10.0"),
                ],
                LEN,
            ),
            (
                "ra.fits",
                vec![
                    ("CTYPE1", "'RA---TAN'"),
                    ("CRVAL1", "10.0"),
                    ("CDELT1", "0.001"),
                ],
                LEN,
            ),
            (
                "pixel.fits",
                vec![
                    ("CTYPE1", "'LINEAR  '"),
                    ("WAT1_001", "'wtype=linear label=Pixel'"),
                    ("CRVAL1", "1.0"),
                    ("CDELT1", "1.0"),
                ],
                LEN,
            ),
            ("one_channel.fits", WAVE_ANGSTROM.to_vec(), 1),
        ];
        for (name, cards, len) in cases {
            let values = vec![1.0f32; len];
            let path = write(
                dir.path(),
                name,
                &[array_hdu(true, -32, len, &cards, f32_bytes(&values))],
            );
            assert_eq!(kind_of(&path), None, "{name}");
        }
    }

    #[test]
    fn err_and_dq_named_1d_arrays_are_never_the_spectrum() {
        let dir = tempfile::tempdir().unwrap();
        let err_cards = with(&WAVE_ANGSTROM, &[("EXTNAME", "'ERR     '")]);
        let dq_cards = with(&WAVE_ANGSTROM, &[("EXTNAME", "'DQ      '")]);
        let path = write(
            dir.path(),
            "companions_only.fits",
            &[
                (empty_primary_cards(), Vec::new()),
                array_hdu(false, -32, LEN, &err_cards, f32_bytes(&err())),
                array_hdu(false, 16, LEN, &dq_cards, i16_bytes(&[0; LEN])),
            ],
        );
        assert_eq!(kind_of(&path), None);
    }

    #[test]
    fn spectral_evidence_without_a_usable_axis_classifies_so_the_refusal_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [(&str, Cards, [&str; 2]); 4] = [
            (
                "log.fits",
                vec![
                    ("CTYPE1", "'WAVE-LOG'"),
                    ("CUNIT1", "'m       '"),
                    ("CRVAL1", "1.0E-6"),
                    ("CDELT1", "1.0E-9"),
                ],
                ["WAVE-LOG", "log-lambda"],
            ),
            (
                "multispec.fits",
                vec![
                    ("CTYPE1", "'MULTISPE'"),
                    ("WAT0_001", "'system=multispec'"),
                    (
                        "WAT1_001",
                        "'wtype=multispec label=Wavelength units=angstroms'",
                    ),
                    ("CRVAL1", "1.0"),
                    ("CDELT1", "1.0"),
                ],
                ["MULTISPE", "multispec"],
            ),
            (
                "no_step.fits",
                vec![
                    ("CTYPE1", "'WAVE    '"),
                    ("CUNIT1", "'um      '"),
                    ("CRVAL1", "2.0"),
                ],
                ["CTYPE1 = 'WAVE'", "CDELT1"],
            ),
            (
                "unitless.fits",
                vec![("CRVAL1", "4000.0"), ("CDELT1", "2.0")],
                ["CTYPE1 missing", "no wavelength solution"],
            ),
        ];
        for (name, cards, expected) in cases {
            let path = write(
                dir.path(),
                name,
                &[array_hdu(true, -32, LEN, &cards, f32_bytes(&flux()))],
            );
            assert_eq!(
                kind_of(&path),
                Some(Spectrum1dKind::Wcs1d { hdu: 0 }),
                "{name}"
            );
            let refused = format!("{:#}", load_spectrum1d(text(&path), None).unwrap_err());
            assert!(
                expected.iter().all(|e| refused.contains(e)),
                "{name}: {refused}"
            );
        }
    }

    #[test]
    fn ctype_less_array_with_a_wavelength_cunit_classifies_and_opens() {
        let dir = tempfile::tempdir().unwrap();
        let cards = [
            ("CUNIT1", "'nm      '"),
            ("CRVAL1", "500.0"),
            ("CDELT1", "0.5"),
            ("OBJECT", "'HD 37795'"),
        ];
        let path = write(
            dir.path(),
            "bess_like.fits",
            &[array_hdu(true, -32, LEN, &cards, f32_bytes(&flux()))],
        );
        assert_eq!(kind_of(&path), Some(Spectrum1dKind::Wcs1d { hdu: 0 }));
        let s = load_spectrum1d(text(&path), None).unwrap();
        assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
        assert!(
            (s.axis.values[1] - 0.5005).abs() < 1e-12,
            "{}",
            s.axis.values[1]
        );
    }

    fn dq_stored(values: &[u32]) -> Vec<i32> {
        values
            .iter()
            .map(|&v| (i64::from(v) - DQ_BZERO) as i32)
            .collect()
    }

    #[test]
    fn mef_spectrum_reads_telescop_from_the_primary_and_pairs_err_dq_by_extver() {
        let dir = tempfile::tempdir().unwrap();
        let mut primary = empty_primary_cards();
        primary.push(("TELESCOP", "'JWST    '".into()));
        let sci = with(
            &WAVE_MICRON,
            &[
                ("EXTNAME", "'SCI     '"),
                ("EXTVER", "1"),
                ("BUNIT", "'Jy      '"),
            ],
        );
        let wrong_err: Vec<f32> = vec![99.0; LEN];
        let dq: Vec<u32> = (0..LEN as u32)
            .map(|i| if i == 5 { 1 | (1 << 31) } else { i % 3 })
            .collect();
        let path = write(
            dir.path(),
            "mef_1d.fits",
            &[
                (primary, Vec::new()),
                array_hdu(false, -32, LEN, &sci, f32_bytes(&flux())),
                array_hdu(
                    false,
                    -32,
                    LEN,
                    &[("EXTNAME", "'ERR     '"), ("EXTVER", "2")],
                    f32_bytes(&wrong_err),
                ),
                array_hdu(
                    false,
                    -32,
                    LEN,
                    &[("EXTNAME", "'ERR     '"), ("EXTVER", "1")],
                    f32_bytes(&err()),
                ),
                array_hdu(
                    false,
                    32,
                    LEN,
                    &[
                        ("EXTNAME", "'DQ      '"),
                        ("EXTVER", "1"),
                        ("BZERO", "2147483648"),
                        ("BSCALE", "1"),
                    ],
                    i32_bytes(&dq_stored(&dq)),
                ),
            ],
        );
        assert_eq!(kind_of(&path), Some(Spectrum1dKind::Wcs1d { hdu: 1 }));

        let s = load_spectrum1d(text(&path), None).unwrap();
        assert_eq!(
            s.source,
            Spectrum1dSource::Wcs1d {
                path: text(&path).to_string(),
                hdu: 1
            }
        );
        assert_eq!(s.wavelength_frame, WavelengthFrame::Vacuum);
        assert_eq!(s.err, Some(widen(&err())));
        assert_eq!(s.dq, Some(dq));
        assert_eq!(s.dq_table, DqTable::Jwst);
        assert_eq!(s.flux_unit.as_deref(), Some("Jy"));
        assert_eq!(s.cards.get("TELESCOP").map(String::as_str), Some("JWST"));
    }

    #[test]
    fn scaled_integer_spectrum_applies_bscale_bzero_and_blank() {
        let dir = tempfile::tempdir().unwrap();
        let raw: Vec<i16> = (0..LEN as i16)
            .map(|i| if i == 3 { -32768 } else { i * 4 - 20 })
            .collect();
        let cards = with(
            &WAVE_ANGSTROM,
            &[("BSCALE", "0.5"), ("BZERO", "100.0"), ("BLANK", "-32768")],
        );
        let path = write(
            dir.path(),
            "i16.fits",
            &[array_hdu(true, 16, LEN, &cards, i16_bytes(&raw))],
        );
        let (_, arrays) = read_1d_array(text(&path), 0).unwrap();
        assert_eq!(arrays.precision, Precision::F64);
        for (i, (&value, &stored)) in arrays.flux.iter().zip(&raw).enumerate() {
            if i == 3 {
                assert!(value.is_nan(), "BLANK decoded as {value}");
            } else {
                assert_eq!(value, f64::from(stored) * 0.5 + 100.0, "channel {i}");
            }
        }
    }

    #[test]
    fn unreadable_scaling_and_a_float_dq_are_refused_naming_the_hdu() {
        let dir = tempfile::tempdir().unwrap();
        let cards = with(&WAVE_ANGSTROM, &[("BSCALE", "'half'")]);
        let path = write(
            dir.path(),
            "bad_scale.fits",
            &[array_hdu(true, 16, LEN, &cards, i16_bytes(&[1; LEN]))],
        );
        let message = format!("{:#}", read_1d_array(text(&path), 0).unwrap_err());
        assert!(
            message.contains("HDU 0") && message.contains("BSCALE"),
            "{message}"
        );

        let path = write(
            dir.path(),
            "float_dq.fits",
            &[
                array_hdu(true, -32, LEN, &WAVE_ANGSTROM, f32_bytes(&flux())),
                array_hdu(
                    false,
                    -32,
                    LEN,
                    &[("EXTNAME", "'DQ      '")],
                    f32_bytes(&[0.0; LEN]),
                ),
            ],
        );
        let message = format!("{:#}", read_1d_array(text(&path), 0).unwrap_err());
        assert!(
            message.contains("HDU 1 (DQ)") && message.contains("BITPIX"),
            "{message}"
        );
    }

    #[test]
    fn truncated_data_unit_is_refused_naming_the_hdu() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "short.fits",
            &[array_hdu(
                true,
                -32,
                LEN,
                &WAVE_ANGSTROM,
                f32_bytes(&flux()),
            )],
        );
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(2880 + 8).unwrap();
        drop(file);
        let message = format!("{:#}", read_1d_array(text(&path), 0).unwrap_err());
        assert!(
            message.contains("HDU 0") && message.contains("past the end of the file"),
            "{message}"
        );
    }

    #[test]
    fn extract1d_table_classifies_as_x1d_and_loads_through_from_x1d() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_x1d_fixture(dir.path(), "jw_nrs1_x1d.fits", 6, &[1, 2], "um");
        assert_eq!(kind_of(&path), Some(Spectrum1dKind::X1d { hdu: 1 }));

        let expected = read_x1d_spectrum(text(&path), None).unwrap();
        let s = load_spectrum1d(text(&path), None).unwrap();
        assert_eq!(
            s.source,
            Spectrum1dSource::X1d {
                path: expected.path.clone(),
                hdu: 1,
                extver: Some(1)
            }
        );
        assert_eq!(s.axis.values, expected.wavelength_um);
        assert_eq!(s.flux, expected.flux);
        assert_eq!(s.err, expected.flux_error);
        assert_eq!(s.wavelength_frame, WavelengthFrame::Vacuum);

        let second = load_spectrum1d(text(&path), Some(2)).unwrap();
        assert_eq!(
            second.source,
            Spectrum1dSource::X1d {
                path: expected.path,
                hdu: 2,
                extver: Some(2)
            }
        );
    }

    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|v| v.to_bits()).collect()
    }

    #[test]
    #[ignore = "needs ASTROBURST_DOCUMENTS_FITS_DIR with the jw01266 NIRSpec x1d and s3d products"]
    fn real_nirspec_x1d_loads_bit_identical_and_its_s3d_cubes_classify_as_none() {
        let dir = documents_fits_dir();
        for grating in ["g395h-f290lp", "g235h-f170lp"] {
            let x1d = dir.join(format!("jw01266-o005_t001_nirspec_{grating}_x1d.fits"));
            assert!(x1d.is_file(), "{} is missing", x1d.display());
            let Some(Spectrum1dKind::X1d { hdu }) = kind_of(&x1d) else {
                panic!("{} did not classify as an x1d table", x1d.display());
            };
            let expected = read_x1d_spectrum(text(&x1d), None).unwrap();
            assert_eq!(hdu, expected.hdu);
            let s = load_spectrum1d(text(&x1d), None).unwrap();
            assert_eq!(
                bits(&s.axis.values),
                bits(&expected.wavelength_um),
                "{grating}"
            );
            assert_eq!(bits(&s.flux), bits(&expected.flux), "{grating}");
            assert_eq!(
                s.err.as_deref().map(bits),
                expected.flux_error.as_deref().map(bits),
                "{grating}"
            );
            assert_eq!(s.dq, expected.dq, "{grating}");
            assert_eq!(s.wavelength_frame, WavelengthFrame::Vacuum);

            let s3d = dir.join(format!("jw01266-o005_t001_nirspec_{grating}_s3d.fits"));
            assert!(s3d.is_file(), "{} is missing", s3d.display());
            assert_eq!(kind_of(&s3d), None, "{}", s3d.display());
        }
    }

    #[test]
    fn spectrum1d_kind_serialises_with_the_kind_tag_the_probe_returns() {
        assert_eq!(
            serde_json::to_value(Spectrum1dKind::X1d { hdu: 1 }).unwrap(),
            serde_json::json!({ "kind": "x1d", "hdu": 1 })
        );
        assert_eq!(
            serde_json::to_value(Spectrum1dKind::Wcs1d { hdu: 0 }).unwrap(),
            serde_json::json!({ "kind": "wcs1d", "hdu": 0 })
        );
    }
}
