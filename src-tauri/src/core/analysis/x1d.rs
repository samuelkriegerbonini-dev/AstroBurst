use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::core::astrometry::spectral::wavelength_unit_to_um;
use crate::core::imaging::dq_flags::DqTable;
use crate::infra::fits::reader::{read_primary_header, ParsedHdu};
use crate::infra::fits::table::{list_tables, load_bintable, Column, ColumnValues, TableHdu};
use crate::types::constants::DEFAULT_WAVELENGTH_UNIT;
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

pub const EXTNAME_EXTRACT1D: &str = "EXTRACT1D";
pub const X1D_SUFFIX: &str = "_x1d";
pub const X1D_SIBLING_SUFFIXES: [&str; 2] = ["_s3d", "_cal"];
pub const COL_WAVELENGTH: &str = "WAVELENGTH";
pub const COL_FLUX: &str = "FLUX";
pub const COL_FLUX_ERROR: &str = "FLUX_ERROR";
pub const COL_SURF_BRIGHT: &str = "SURF_BRIGHT";
pub const COL_BACKGROUND: &str = "BACKGROUND";
pub const COL_NPIXELS: &str = "NPIXELS";
pub const COL_DQ: &str = "DQ";

const DEFAULT_FLUX_UNIT: &str = "Jy";
const OPTIONAL_COLUMNS: [&str; 5] = [COL_FLUX_ERROR, COL_SURF_BRIGHT, COL_BACKGROUND, COL_NPIXELS, COL_DQ];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct X1dTableRef {
    pub hdu: usize,
    pub extver: Option<i64>,
    pub n_rows: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct X1dSpectrum {
    pub path: String,
    pub hdu: usize,
    pub extver: Option<i64>,
    pub n_rows: usize,
    pub wavelength_um: Vec<f64>,
    pub wavelength_unit: String,
    pub flux: Vec<f64>,
    pub flux_error: Option<Vec<f64>>,
    pub flux_unit: String,
    pub surf_bright: Option<Vec<f64>>,
    pub surf_bright_unit: Option<String>,
    pub background: Option<Vec<f64>>,
    pub npixels: Option<Vec<f64>>,
    pub dq: Option<Vec<u32>>,
    pub dq_table: DqTable,
    pub dq_flagged_rows: usize,
    pub srctype: Option<String>,
    pub grating: Option<String>,
    pub filter: Option<String>,
    pub detector: Option<String>,
    pub instrument: Option<String>,
    pub target: Option<String>,
    pub other_tables: Vec<X1dTableRef>,
    pub notes: Vec<String>,
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn resolve_x1d_path(path: &str) -> Result<PathBuf, String> {
    let bare = ImageRef::parse(path).path;
    let candidate = Path::new(&bare);
    let name = file_name(candidate);
    let stem = candidate.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = candidate.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = stem.to_lowercase();
    let Some(suffix) = X1D_SIBLING_SUFFIXES.iter().find(|s| lower.ends_with(*s)) else {
        return Ok(candidate.to_path_buf());
    };
    let base = &stem[..stem.len() - suffix.len()];
    let sibling_name = if ext.is_empty() {
        format!("{base}{X1D_SUFFIX}")
    } else {
        format!("{base}{X1D_SUFFIX}.{ext}")
    };
    let sibling = candidate.with_file_name(&sibling_name);
    if sibling.is_file() {
        Ok(sibling)
    } else {
        Err(format!("no pipeline x1d next to {name}: expected {sibling_name}"))
    }
}

fn is_extract1d(table: &TableHdu) -> bool {
    table
        .extname
        .as_deref()
        .is_some_and(|e| e.trim().eq_ignore_ascii_case(EXTNAME_EXTRACT1D))
}

fn table_ref(table: &TableHdu) -> X1dTableRef {
    X1dTableRef { hdu: table.index, extver: table.extver, n_rows: table.n_rows }
}

fn extract1d_refs(tables: &[(TableHdu, ParsedHdu)]) -> Vec<X1dTableRef> {
    tables.iter().filter(|(t, _)| is_extract1d(t)).map(|(t, _)| table_ref(t)).collect()
}

pub fn extract1d_tables(file: &File) -> Result<Vec<X1dTableRef>> {
    Ok(extract1d_refs(&list_tables(file)?))
}

pub fn pick_table(tables: &[X1dTableRef], hdu: Option<usize>) -> Result<X1dTableRef, String> {
    match hdu {
        Some(n) => tables
            .iter()
            .find(|t| t.hdu == n)
            .cloned()
            .ok_or_else(|| format!("HDU {n} is not an {EXTNAME_EXTRACT1D} table")),
        None => tables
            .iter()
            .find(|t| t.extver == Some(1))
            .or_else(|| tables.first())
            .cloned()
            .ok_or_else(|| format!("no {EXTNAME_EXTRACT1D} table in the file")),
    }
}

fn inventory(tables: &[(TableHdu, ParsedHdu)]) -> String {
    if tables.is_empty() {
        return "none".to_string();
    }
    tables
        .iter()
        .map(|(t, _)| format!("[{}] {} ({} rows)", t.index, t.extname.as_deref().unwrap_or("(none)"), t.n_rows))
        .collect::<Vec<_>>()
        .join(", ")
}

fn card(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn as_f64(column: &Column) -> Vec<f64> {
    match &column.values {
        ColumnValues::F64(v) => v.clone(),
        ColumnValues::I64(v) => v.iter().map(|&x| x as f64).collect(),
    }
}

fn as_u32(column: &Column) -> Vec<u32> {
    match &column.values {
        ColumnValues::I64(v) => v.iter().map(|&x| x.clamp(0, u32::MAX as i64) as u32).collect(),
        ColumnValues::F64(v) => v
            .iter()
            .map(|&x| if x.is_finite() { x.clamp(0.0, u32::MAX as f64) as u32 } else { 0 })
            .collect(),
    }
}

pub fn read_x1d_spectrum(path: &str, hdu: Option<usize>) -> Result<X1dSpectrum> {
    let resolved = resolve_x1d_path(path).map_err(anyhow::Error::msg)?;
    let resolved_key = resolved.to_string_lossy().into_owned();
    let name = file_name(&resolved);
    let file = File::open(&resolved).with_context(|| format!("Failed to open {resolved_key}"))?;
    let tables = list_tables(&file)?;
    let refs = extract1d_refs(&tables);
    if refs.is_empty() {
        bail!("no {EXTNAME_EXTRACT1D} table in {name}; tables in this file: {}", inventory(&tables));
    }
    if let Some(n) = hdu.filter(|n| !refs.iter().any(|r| r.hdu == *n)) {
        let label = tables
            .iter()
            .find(|(t, _)| t.index == n)
            .map(|(t, _)| t.extname.clone().unwrap_or_else(|| "(none)".to_string()))
            .unwrap_or_else(|| "not a table".to_string());
        bail!("HDU {n} ({label}) is not an {EXTNAME_EXTRACT1D} table; tables in this file: {}", inventory(&tables));
    }
    let chosen = pick_table(&refs, hdu).map_err(anyhow::Error::msg)?;
    let (table, parsed) = tables
        .iter()
        .find(|(t, _)| t.index == chosen.hdu)
        .with_context(|| format!("HDU {} of {name} is not listed as a table", chosen.hdu))?;

    let mut notes = Vec::new();
    let data = load_bintable(&resolved_key, chosen.hdu)?;
    let required = data.columns(&[COL_WAVELENGTH, COL_FLUX])?;
    let mut columns = Vec::with_capacity(required.len() + OPTIONAL_COLUMNS.len());
    columns.extend(required);
    for optional in OPTIONAL_COLUMNS {
        if !table.columns.iter().any(|c| c.name == optional) {
            notes.push(format!("{optional} column missing"));
            continue;
        }
        match data.column(optional) {
            Ok(column) => columns.push(column),
            Err(e) => notes.push(format!("{optional} column unreadable: {e:#}")),
        }
    }
    let column = |col: &str| columns.iter().find(|c| c.name == col);

    let wavelength = column(COL_WAVELENGTH).with_context(|| format!("{COL_WAVELENGTH} column missing"))?;
    let scale = match wavelength.unit.as_deref() {
        None => {
            notes.push(format!("TUNIT of {COL_WAVELENGTH} missing: assumed {DEFAULT_WAVELENGTH_UNIT}"));
            1.0
        }
        Some(unit) => wavelength_unit_to_um(unit).with_context(|| format!("unknown wavelength unit '{unit}'"))?,
    };
    let wavelength_um = as_f64(wavelength).into_iter().map(|w| w * scale).collect();

    let flux = column(COL_FLUX).with_context(|| format!("{COL_FLUX} column missing"))?;
    let flux_unit = match flux.unit.clone() {
        Some(unit) => unit,
        None => {
            notes.push(format!("TUNIT of {COL_FLUX} missing: assumed {DEFAULT_FLUX_UNIT}"));
            DEFAULT_FLUX_UNIT.to_string()
        }
    };
    let optional = |col: &str| column(col).map(as_f64);
    let dq = column(COL_DQ).map(as_u32);
    let dq_flagged_rows = dq.as_ref().map_or(0, |d| d.iter().filter(|&&v| v != 0).count());
    let primary = read_primary_header(&resolved_key)?;

    Ok(X1dSpectrum {
        path: resolved_key,
        hdu: chosen.hdu,
        extver: chosen.extver,
        n_rows: table.n_rows,
        wavelength_um,
        wavelength_unit: DEFAULT_WAVELENGTH_UNIT.to_string(),
        flux: as_f64(flux),
        flux_error: optional(COL_FLUX_ERROR),
        flux_unit,
        surf_bright: optional(COL_SURF_BRIGHT),
        surf_bright_unit: column(COL_SURF_BRIGHT).and_then(|c| c.unit.clone()),
        background: optional(COL_BACKGROUND),
        npixels: optional(COL_NPIXELS),
        dq,
        dq_table: DqTable::select(&primary),
        dq_flagged_rows,
        srctype: card(&parsed.header, "SRCTYPE").or_else(|| card(&primary, "SRCTYPE")),
        grating: card(&primary, "GRATING"),
        filter: card(&primary, "FILTER"),
        detector: card(&primary, "DETECTOR"),
        instrument: card(&primary, "INSTRUME"),
        target: card(&primary, "TARGNAME"),
        other_tables: refs.into_iter().filter(|r| r.hdu != chosen.hdu).collect(),
        notes,
    })
}

#[cfg(test)]
pub mod test_fixtures {
    use std::path::{Path, PathBuf};

    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, write_raw_hdus};

    pub const FIXTURE_FLAGGED_ROW: usize = 3;

    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    pub fn fixture_wavelength(row: usize, unit: &str) -> f64 {
        if unit.eq_ignore_ascii_case("angstrom") {
            20000.0 + 10.0 * row as f64
        } else {
            2.0 + 0.001 * row as f64
        }
    }

    pub fn fixture_flux(row: usize) -> f64 {
        10.0 + row as f64
    }

    fn extract1d_hdu(rows: usize, extver: i64, wavelength_unit: &str) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let columns: [(&str, &str, Option<&str>); 7] = [
            ("WAVELENGTH", "1D", if wavelength_unit.is_empty() { None } else { Some(wavelength_unit) }),
            ("FLUX", "1D", Some("Jy")),
            ("FLUX_ERROR", "1D", Some("Jy")),
            ("SURF_BRIGHT", "1D", Some("MJy/sr")),
            ("BACKGROUND", "1D", Some("Jy")),
            ("NPIXELS", "1D", None),
            ("DQ", "1J", None),
        ];
        let mut cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "52".into()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "7".into()),
            ("EXTNAME", "'EXTRACT1D'".into()),
            ("EXTVER", extver.to_string()),
            ("SRCTYPE", "'POINT   '".into()),
        ];
        for (i, (name, tform, unit)) in columns.iter().enumerate() {
            let n = i + 1;
            cards.push((leak(format!("TTYPE{n}")), format!("'{name:<8}'")));
            cards.push((leak(format!("TFORM{n}")), format!("'{tform:<8}'")));
            if let Some(unit) = unit {
                cards.push((leak(format!("TUNIT{n}")), format!("'{unit:<8}'")));
            }
        }
        let mut data = Vec::with_capacity(52 * rows);
        for row in 0..rows {
            data.extend_from_slice(&fixture_wavelength(row, wavelength_unit).to_be_bytes());
            data.extend_from_slice(&fixture_flux(row).to_be_bytes());
            data.extend_from_slice(&(0.1 * (row as f64 + 1.0)).to_be_bytes());
            data.extend_from_slice(&(100.0 + row as f64).to_be_bytes());
            data.extend_from_slice(&1.0f64.to_be_bytes());
            data.extend_from_slice(&9.0f64.to_be_bytes());
            let dq: i32 = if row == FIXTURE_FLAGGED_ROW { 1 } else { 0 };
            data.extend_from_slice(&dq.to_be_bytes());
        }
        (cards, data)
    }

    fn asdf_hdu() -> (Vec<(&'static str, String)>, Vec<u8>) {
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "1".into()),
            ("NAXIS2", "1".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("TTYPE1", "'ASDF_METADATA'".into()),
            ("TFORM1", "'1B      '".into()),
            ("EXTNAME", "'ASDF    '".into()),
        ];
        (cards, vec![0u8])
    }

    pub fn write_x1d_fixture(dir: &Path, name: &str, rows: usize, extvers: &[i64], wavelength_unit: &str) -> PathBuf {
        let mut primary = empty_primary_cards();
        primary.extend([
            ("TELESCOP", "'JWST    '".to_string()),
            ("INSTRUME", "'NIRSPEC '".to_string()),
            ("GRATING", "'G235H   '".to_string()),
            ("FILTER", "'F170LP  '".to_string()),
            ("DETECTOR", "'NRS1    '".to_string()),
            ("TARGNAME", "'T       '".to_string()),
        ]);
        let mut hdus = vec![(primary, Vec::new())];
        for &extver in extvers {
            hdus.push(extract1d_hdu(rows, extver, wavelength_unit));
        }
        hdus.push(asdf_hdu());
        let path = dir.join(name);
        write_raw_hdus(&path, &hdus);
        path
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;
    use crate::infra::fits::table::test_support::write_table;

    fn key(path: &std::path::Path) -> String {
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn read_x1d_spectrum_returns_the_columns_cards_and_decoded_dq() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_x1d_fixture(dir.path(), "jw_nrs1_x1d.fits", 6, &[1], "um");
        let s = read_x1d_spectrum(&key(&path), None).unwrap();
        assert!(s.path.ends_with("jw_nrs1_x1d.fits"), "{}", s.path);
        assert_eq!(s.hdu, 1);
        assert_eq!(s.extver, Some(1));
        assert_eq!(s.n_rows, 6);
        let expected: Vec<f64> = (0..6).map(|i| fixture_wavelength(i, "um")).collect();
        assert_eq!(s.wavelength_um, expected);
        assert_eq!(s.wavelength_unit, "um");
        assert_eq!(s.flux, (0..6).map(fixture_flux).collect::<Vec<_>>());
        assert_eq!(s.flux_unit, "Jy");
        assert_eq!(s.flux_error.as_ref().map(|v| v.len()), Some(6));
        assert_eq!(s.surf_bright.as_ref().map(|v| v[0]), Some(100.0));
        assert_eq!(s.surf_bright_unit.as_deref(), Some("MJy/sr"));
        assert_eq!(s.background.as_ref().map(|v| v[0]), Some(1.0));
        assert_eq!(s.npixels.as_ref().map(|v| v[0]), Some(9.0));
        assert_eq!(s.dq, Some(vec![0, 0, 0, 1, 0, 0]));
        assert_eq!(s.dq_table, DqTable::Jwst);
        assert_eq!(s.dq_flagged_rows, 1);
        assert_eq!(s.srctype.as_deref(), Some("POINT"));
        assert_eq!(s.grating.as_deref(), Some("G235H"));
        assert_eq!(s.filter.as_deref(), Some("F170LP"));
        assert_eq!(s.detector.as_deref(), Some("NRS1"));
        assert_eq!(s.instrument.as_deref(), Some("NIRSPEC"));
        assert_eq!(s.target.as_deref(), Some("T"));
        assert!(s.other_tables.is_empty(), "{:?}", s.other_tables);
        assert!(s.notes.is_empty(), "{:?}", s.notes);
    }

    #[test]
    fn read_x1d_spectrum_from_the_s3d_or_cal_path_opens_the_sibling_and_names_it_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_x1d_fixture(dir.path(), "foo_x1d.fits", 4, &[1], "um");
        let s3d = format!("{}#hdu=1", key(&dir.path().join("foo_s3d.fits")));
        let resolved = resolve_x1d_path(&s3d).unwrap();
        assert!(resolved.ends_with("foo_x1d.fits"), "{}", resolved.display());
        let s = read_x1d_spectrum(&s3d, None).unwrap();
        assert!(s.path.ends_with("foo_x1d.fits"), "{}", s.path);
        assert_eq!(s.n_rows, 4);

        let cal = key(&dir.path().join("bar_cal.fits"));
        let err = resolve_x1d_path(&cal).unwrap_err();
        assert!(err.starts_with("no pipeline x1d next to bar_cal.fits: expected bar_x1d.fits"), "{err}");
        let err = format!("{:#}", read_x1d_spectrum(&cal, None).unwrap_err());
        assert!(err.contains("bar_x1d.fits"), "{err}");

        let rate = key(&dir.path().join("baz_rate.fits"));
        assert_eq!(resolve_x1d_path(&rate).unwrap(), dir.path().join("baz_rate.fits"));
        let plain = format!("{}#hdu=2", key(&dir.path().join("spectrum.fits")));
        assert_eq!(resolve_x1d_path(&plain).unwrap(), dir.path().join("spectrum.fits"));

        let direct = key(&dir.path().join("foo_x1d.fits"));
        assert_eq!(resolve_x1d_path(&direct).unwrap(), dir.path().join("foo_x1d.fits"));
    }

    #[test]
    fn read_x1d_spectrum_accepts_a_table_file_without_a_pipeline_suffix_and_still_checks_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let table = write_x1d_fixture(dir.path(), "spectrum.fits", 4, &[1], "um");
        let s = read_x1d_spectrum(&key(&table), None).unwrap();
        assert!(s.path.ends_with("spectrum.fits"), "{}", s.path);
        assert_eq!(s.n_rows, 4);
        assert_eq!(s.flux, (0..4).map(fixture_flux).collect::<Vec<_>>());

        let no_table = write_x1d_fixture(dir.path(), "plain.fits", 4, &[], "um");
        let err = format!("{:#}", read_x1d_spectrum(&key(&no_table), None).unwrap_err());
        assert!(err.starts_with("no EXTRACT1D table in plain.fits; tables in this file: [1] ASDF (1 rows)"), "{err}");

        let missing = key(&dir.path().join("nowhere.fits"));
        let err = format!("{:#}", read_x1d_spectrum(&missing, None).unwrap_err());
        assert!(err.contains("nowhere.fits"), "{err}");
    }

    #[test]
    fn read_x1d_spectrum_drops_an_unreadable_optional_column_with_a_note_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = Vec::new();
        for row in 0..3u32 {
            data.extend_from_slice(&(2.0 + 0.001 * row as f64).to_be_bytes());
            data.extend_from_slice(&(10.0 + row as f64).to_be_bytes());
            data.push(b'T');
            data.extend_from_slice(&(row as i32 % 2).to_be_bytes());
        }
        let path = write_table(
            dir.path(),
            "odd_x1d.fits",
            &[("WAVELENGTH", "1D", Some("um")), ("FLUX", "1D", Some("Jy")), ("NPIXELS", "1L", None), ("DQ", "1J", None)],
            3,
            data,
            &[],
        );
        let s = read_x1d_spectrum(&key(&path), None).unwrap();
        assert_eq!(s.flux, vec![10.0, 11.0, 12.0]);
        assert_eq!(s.npixels, None);
        assert_eq!(s.dq, Some(vec![0, 1, 0]));
        assert_eq!(s.dq_flagged_rows, 1);
        assert!(s.flux_error.is_none() && s.surf_bright.is_none() && s.background.is_none());
        let note = s.notes.iter().find(|n| n.starts_with("NPIXELS column unreadable: ")).unwrap_or_else(|| panic!("{:?}", s.notes));
        assert!(note.contains("unsupported scalar type 'L'"), "{note}");
        assert!(s.notes.iter().any(|n| n == "FLUX_ERROR column missing"), "{:?}", s.notes);

        let flux_as_logical = write_table(
            dir.path(),
            "bad_x1d.fits",
            &[("WAVELENGTH", "1D", Some("um")), ("FLUX", "1L", None)],
            1,
            vec![0u8; 9],
            &[],
        );
        let err = format!("{:#}", read_x1d_spectrum(&key(&flux_as_logical), None).unwrap_err());
        assert!(err.contains("column 'FLUX': unsupported scalar type 'L'"), "{err}");
    }

    #[test]
    fn read_x1d_spectrum_refuses_a_table_whose_tfields_exceeds_the_fits_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_table(
            dir.path(),
            "corrupt_x1d.fits",
            &[("WAVELENGTH", "1D", Some("um")), ("FLUX", "1D", Some("Jy"))],
            1,
            vec![0u8; 16],
            &[("TFIELDS", "10000000000".into())],
        );
        let expected = "HDU 1 (EXTRACT1D): TFIELDS=10000000000 exceeds the FITS limit of 999";
        let err = format!("{:#}", read_x1d_spectrum(&key(&path), None).unwrap_err());
        assert!(err.contains(expected), "{err}");
        let via_sibling = key(&dir.path().join("corrupt_s3d.fits"));
        let err = format!("{:#}", read_x1d_spectrum(&via_sibling, None).unwrap_err());
        assert!(err.contains(expected), "{err}");
    }

    #[test]
    fn read_x1d_spectrum_picks_extver_1_by_default_and_lists_the_other_extract1d_hdus() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_x1d_fixture(dir.path(), "mos_x1d.fits", 5, &[2, 1], "um");
        let file = File::open(&path).unwrap();
        let tables = extract1d_tables(&file).unwrap();
        assert_eq!(
            tables,
            vec![
                X1dTableRef { hdu: 1, extver: Some(2), n_rows: 5 },
                X1dTableRef { hdu: 2, extver: Some(1), n_rows: 5 },
            ]
        );
        assert_eq!(pick_table(&tables, None).unwrap().hdu, 2);
        assert_eq!(pick_table(&tables, Some(1)).unwrap().extver, Some(2));
        assert!(pick_table(&tables, Some(3)).is_err());
        assert_eq!(pick_table(&tables[..1], None).unwrap().hdu, 1);
        assert!(pick_table(&[], None).is_err());

        let default = read_x1d_spectrum(&key(&path), None).unwrap();
        assert_eq!(default.hdu, 2);
        assert_eq!(default.extver, Some(1));
        assert_eq!(default.other_tables, vec![X1dTableRef { hdu: 1, extver: Some(2), n_rows: 5 }]);

        let second = read_x1d_spectrum(&key(&path), Some(1)).unwrap();
        assert_eq!(second.hdu, 1);
        assert_eq!(second.extver, Some(2));
        assert_eq!(second.other_tables, vec![X1dTableRef { hdu: 2, extver: Some(1), n_rows: 5 }]);
    }

    #[test]
    fn read_x1d_spectrum_refuses_a_non_extract1d_hdu_with_the_table_inventory() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_x1d_fixture(dir.path(), "inv_x1d.fits", 6, &[1], "um");
        let err = format!("{:#}", read_x1d_spectrum(&key(&path), Some(2)).unwrap_err());
        assert!(err.starts_with("HDU 2 (ASDF) is not an EXTRACT1D table; tables in this file: "), "{err}");
        assert!(err.contains("[1] EXTRACT1D (6 rows)"), "{err}");
        assert!(err.contains("[2] ASDF (1 rows)"), "{err}");
        let err = format!("{:#}", read_x1d_spectrum(&key(&path), Some(7)).unwrap_err());
        assert!(err.contains("HDU 7"), "{err}");
        assert!(err.contains("EXTRACT1D"), "{err}");
    }

    #[test]
    fn read_x1d_spectrum_converts_angstrom_wavelengths_to_um() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_x1d_fixture(dir.path(), "ang_x1d.fits", 3, &[1], "Angstrom");
        let s = read_x1d_spectrum(&key(&path), None).unwrap();
        assert!((s.wavelength_um[0] - 2.0).abs() < 1e-12, "{:?}", s.wavelength_um);
        assert!((s.wavelength_um[2] - 2.002).abs() < 1e-12, "{:?}", s.wavelength_um);
        assert_eq!(s.wavelength_unit, "um");
        assert!(s.notes.is_empty(), "{:?}", s.notes);
    }

    #[test]
    fn read_x1d_spectrum_assumes_um_when_tunit_is_missing_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_x1d_fixture(dir.path(), "nounit_x1d.fits", 3, &[1], "");
        let s = read_x1d_spectrum(&key(&path), None).unwrap();
        assert_eq!(s.wavelength_um, (0..3).map(|i| fixture_wavelength(i, "")).collect::<Vec<_>>());
        assert_eq!(s.wavelength_unit, "um");
        assert!(s.notes.iter().any(|n| n == "TUNIT of WAVELENGTH missing: assumed um"), "{:?}", s.notes);
    }
}
