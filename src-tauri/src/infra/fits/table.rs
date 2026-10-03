use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use anyhow::{bail, Context, Result};

use crate::infra::fits::reader::{list_extensions, read_header_blocks, ParsedHdu};
use crate::types::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnSpan {
    Fixed {
        offset: usize,
        type_code: char,
        repeat: usize,
    },
    VarArray {
        offset: usize,
        elem_bytes: usize,
        wide: bool,
    },
}

pub(crate) struct BintableLayout {
    pub row_width: usize,
    pub n_rows: usize,
    pub heap_base: usize,
    pub columns: HashMap<String, ColumnSpan>,
}

pub(crate) fn tform_elem_bytes(type_code: char) -> Result<usize> {
    Ok(match type_code {
        'L' | 'B' | 'A' => 1,
        'I' => 2,
        'J' | 'E' => 4,
        'K' | 'D' | 'C' => 8,
        'M' => 16,
        other => bail!("Unsupported BINTABLE column type code '{other}'"),
    })
}

pub(crate) enum TformKind {
    Fixed { repeat: usize, type_code: char },
    VarArray { elem_type: char, wide: bool },
}

pub(crate) fn parse_tform(tform: &str) -> Result<TformKind> {
    let tform = tform.trim();
    let digit_end = tform
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tform.len());
    let repeat: usize = if digit_end == 0 {
        1
    } else {
        tform[..digit_end].parse().unwrap_or(1)
    };
    let mut chars = tform[digit_end..].chars();
    let type_code = chars.next().context("empty TFORM type code")?;
    if type_code == 'P' || type_code == 'Q' {
        let elem_type = chars
            .next()
            .context("variable-length TFORM missing element type code")?;
        Ok(TformKind::VarArray {
            elem_type,
            wide: type_code == 'Q',
        })
    } else {
        Ok(TformKind::Fixed { repeat, type_code })
    }
}

const MAX_TFIELDS: i64 = 999;

fn checked_tfields(header: &HduHeader, label: &str) -> Result<usize> {
    let tfields = header.get_i64("TFIELDS").unwrap_or(0);
    if tfields < 0 {
        bail!("{label}: TFIELDS={tfields} is negative");
    }
    if tfields > MAX_TFIELDS {
        bail!("{label}: TFIELDS={tfields} exceeds the FITS limit of {MAX_TFIELDS}");
    }
    Ok(tfields as usize)
}

pub(crate) fn build_bintable_layout(header: &HduHeader, data_start: usize) -> Result<BintableLayout> {
    let naxis1 = header.get_i64("NAXIS1").context("Missing NAXIS1")? as usize;
    let naxis2 = header.get_i64("NAXIS2").context("Missing NAXIS2")? as usize;
    let tfields = checked_tfields(header, &hdu_label(header))?;

    let mut offset = 0usize;
    let mut columns = HashMap::new();

    for i in 1..=tfields {
        let tform = header
            .get(&format!("TFORM{i}"))
            .with_context(|| format!("Missing TFORM{i}"))?;
        let ttype = header
            .get(&format!("TTYPE{i}"))
            .map(|s| s.trim().to_uppercase());

        let (span, width) = match parse_tform(tform)? {
            TformKind::VarArray { elem_type, wide } => {
                let elem_bytes = tform_elem_bytes(elem_type)?;
                let width = if wide { 16 } else { 8 };
                (
                    ColumnSpan::VarArray {
                        offset,
                        elem_bytes,
                        wide,
                    },
                    width,
                )
            }
            TformKind::Fixed { repeat, type_code } => {
                let width = if type_code == 'X' {
                    repeat.div_ceil(8)
                } else {
                    repeat * tform_elem_bytes(type_code)?
                };
                (ColumnSpan::Fixed { offset, type_code, repeat }, width)
            }
        };

        if let Some(name) = ttype {
            columns.insert(name, span);
        }
        offset += width;
    }

    if offset != naxis1 {
        bail!(
            "BINTABLE row width mismatch: TFORM columns sum to {} bytes, NAXIS1={}",
            offset,
            naxis1
        );
    }

    let theap = header.get_i64("THEAP").unwrap_or((naxis1 * naxis2) as i64) as usize;
    let heap_base = data_start + theap;

    Ok(BintableLayout {
        row_width: naxis1,
        n_rows: naxis2,
        heap_base,
        columns,
    })
}

pub(crate) fn read_var_descriptor(row: &[u8], span: &ColumnSpan) -> Result<(usize, usize)> {
    match *span {
        ColumnSpan::VarArray {
            offset,
            elem_bytes,
            wide,
        } => {
            let (nelem, rel) = if wide {
                let nelem = i64::from_be_bytes(row[offset..offset + 8].try_into().unwrap());
                let rel = i64::from_be_bytes(row[offset + 8..offset + 16].try_into().unwrap());
                (nelem as usize, rel as usize)
            } else {
                let nelem = i32::from_be_bytes(row[offset..offset + 4].try_into().unwrap());
                let rel = i32::from_be_bytes(row[offset + 4..offset + 8].try_into().unwrap());
                (nelem as usize, rel as usize)
            };
            Ok((nelem * elem_bytes, rel * elem_bytes))
        }
        _ => bail!("column is not a variable-length array"),
    }
}

pub(crate) fn read_fixed_f64(row: &[u8], span: &ColumnSpan) -> Result<f64> {
    match *span {
        ColumnSpan::Fixed {
            offset,
            type_code: 'D',
            ..
        } => Ok(f64::from_be_bytes(
            row[offset..offset + 8].try_into().unwrap(),
        )),
        ColumnSpan::Fixed {
            offset,
            type_code: 'E',
            ..
        } => Ok(f32::from_be_bytes(row[offset..offset + 4].try_into().unwrap()) as f64),
        _ => bail!("column is not a fixed D/E scalar"),
    }
}

fn hdu_label(header: &HduHeader) -> String {
    if let Some(name) = header.get("EXTNAME").map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    header
        .get("XTENSION")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "PRIMARY".to_string())
}

pub fn is_bintable_hdu(header: &HduHeader) -> bool {
    let is_bintable = header
        .get("XTENSION")
        .map(|x| x.trim().eq_ignore_ascii_case("BINTABLE"))
        .unwrap_or(false);
    let is_zimage = header
        .get("ZIMAGE")
        .map(|v| v.trim().eq_ignore_ascii_case("T"))
        .unwrap_or(false);
    is_bintable && !is_zimage
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ColumnInfo {
    pub index: usize,
    pub name: String,
    pub tform: String,
    pub type_code: char,
    pub repeat: usize,
    pub unit: Option<String>,
    pub tnull: Option<i64>,
    pub tzero: Option<f64>,
    pub tscal: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TableHdu {
    pub index: usize,
    pub extname: Option<String>,
    pub extver: Option<i64>,
    pub n_rows: usize,
    pub columns: Vec<ColumnInfo>,
}

fn column_infos(header: &HduHeader, label: &str) -> Result<Vec<ColumnInfo>> {
    let tfields = checked_tfields(header, label)?;
    let mut columns = Vec::with_capacity(tfields);
    for i in 1..=tfields {
        let tform = card_text(header, &format!("TFORM{i}")).with_context(|| format!("Missing TFORM{i}"))?;
        let (type_code, repeat) = match parse_tform(&tform)? {
            TformKind::Fixed { repeat, type_code } => (type_code, repeat),
            TformKind::VarArray { wide, .. } => (if wide { 'Q' } else { 'P' }, 0),
        };
        columns.push(ColumnInfo {
            index: i,
            name: card_text(header, &format!("TTYPE{i}")).map(|s| s.to_uppercase()).unwrap_or_default(),
            tform,
            type_code,
            repeat,
            unit: card_text(header, &format!("TUNIT{i}")),
            tnull: header.get_i64(&format!("TNULL{i}")),
            tzero: header.get_f64(&format!("TZERO{i}")),
            tscal: header.get_f64(&format!("TSCAL{i}")),
        });
    }
    Ok(columns)
}

pub fn describe_table(header: &HduHeader, index: usize) -> Result<TableHdu> {
    Ok(TableHdu {
        index,
        extname: card_text(header, "EXTNAME"),
        extver: header.get_i64("EXTVER"),
        n_rows: header.get_i64("NAXIS2").unwrap_or(0).max(0) as usize,
        columns: column_infos(header, &format!("HDU {index} ({})", hdu_label(header)))?,
    })
}

pub fn list_tables(file: &File) -> Result<Vec<(TableHdu, ParsedHdu)>> {
    let mut tables = Vec::new();
    for info in list_extensions(file)? {
        let parsed = read_header_blocks(file, info.header_start)?;
        if is_bintable_hdu(&parsed.header) {
            tables.push((describe_table(&parsed.header, info.index)?, parsed));
        }
    }
    Ok(tables)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColumnValues {
    F64(Vec<f64>),
    I64(Vec<i64>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub unit: Option<String>,
    pub values: ColumnValues,
    pub null_rows: usize,
}

fn read_raw_int(bytes: &[u8], type_code: char) -> i64 {
    match type_code {
        'B' => bytes[0] as i64,
        'I' => i16::from_be_bytes(bytes[..2].try_into().unwrap()) as i64,
        'J' => i32::from_be_bytes(bytes[..4].try_into().unwrap()) as i64,
        _ => i64::from_be_bytes(bytes[..8].try_into().unwrap()),
    }
}

fn read_one_column(data: &[u8], layout: &BintableLayout, info: &ColumnInfo) -> Result<Column> {
    let span = layout
        .columns
        .get(&info.name)
        .with_context(|| format!("column '{}' has no layout entry", info.name))?;
    let (offset, type_code) = match *span {
        ColumnSpan::Fixed { offset, type_code, repeat: 1 } => (offset, type_code),
        _ => bail!(
            "column '{}' is an array column (TFORM '{}'); only scalar columns are read",
            info.name,
            info.tform
        ),
    };
    let rows = (0..layout.n_rows).map(|i| &data[i * layout.row_width + offset..(i + 1) * layout.row_width]);
    let (values, null_rows) = match type_code {
        'D' => (ColumnValues::F64(rows.map(|r| f64::from_be_bytes(r[..8].try_into().unwrap())).collect()), 0),
        'E' => (ColumnValues::F64(rows.map(|r| f32::from_be_bytes(r[..4].try_into().unwrap()) as f64).collect()), 0),
        'B' | 'I' | 'J' | 'K' => {
            let raws: Vec<i64> = rows.map(|r| read_raw_int(r, type_code)).collect();
            let null_rows = info.tnull.map_or(0, |tnull| raws.iter().filter(|&&v| v == tnull).count());
            let tscal = info.tscal.unwrap_or(1.0);
            let tzero = info.tzero.unwrap_or(0.0);
            let promote = tscal != 1.0 || tzero.fract() != 0.0;
            let values = if promote {
                ColumnValues::F64(
                    raws.iter()
                        .map(|&raw| if info.tnull == Some(raw) { f64::NAN } else { raw as f64 * tscal + tzero })
                        .collect(),
                )
            } else {
                let shift = tzero as i128;
                let shifted: Vec<i128> = raws.iter().map(|&raw| raw as i128 + shift).collect();
                if shifted.iter().all(|&v| i64::try_from(v).is_ok()) {
                    ColumnValues::I64(shifted.iter().map(|&v| v as i64).collect())
                } else {
                    ColumnValues::F64(
                        raws.iter()
                            .zip(&shifted)
                            .map(|(&raw, &v)| if info.tnull == Some(raw) { f64::NAN } else { v as f64 })
                            .collect(),
                    )
                }
            };
            (values, null_rows)
        }
        other => bail!("column '{}': unsupported scalar type '{}'", info.name, other),
    };
    Ok(Column { name: info.name.clone(), unit: info.unit.clone(), values, null_rows })
}

fn read_columns_in(data: &[u8], header: &HduHeader, names: &[&str], label: &str) -> Result<Vec<Column>> {
    let infos = column_infos(header, label)?;
    let layout = build_bintable_layout(header, 0)?;
    let needed = layout
        .row_width
        .checked_mul(layout.n_rows)
        .with_context(|| format!("{label}: NAXIS1*NAXIS2 overflows"))?;
    if data.len() < needed {
        bail!("{label}: table data holds {} bytes but NAXIS1*NAXIS2 needs {needed}", data.len());
    }
    let available = infos.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");
    names
        .iter()
        .map(|requested| {
            let wanted = requested.trim().to_uppercase();
            let matches: Vec<&ColumnInfo> = infos.iter().filter(|c| c.name == wanted).collect();
            if matches.len() > 1 {
                let keys = matches.iter().map(|c| format!("TTYPE{}", c.index)).collect::<Vec<_>>().join(", ");
                bail!("column '{wanted}' appears {} times in {label} ({keys}); refusing the ambiguous name", matches.len());
            }
            let info = matches
                .first()
                .with_context(|| format!("column '{wanted}' not in {label}; columns: {available}"))?;
            read_one_column(data, &layout, info)
        })
        .collect()
}

pub fn read_columns(data: &[u8], header: &HduHeader, names: &[&str]) -> Result<Vec<Column>> {
    let label = card_text(header, "EXTNAME").map_or_else(|| "this table".to_string(), |n| format!("table {n}"));
    read_columns_in(data, header, names, &label)
}

pub struct BintableData {
    label: String,
    header: HduHeader,
    bytes: Vec<u8>,
}

impl BintableData {
    pub fn column(&self, name: &str) -> Result<Column> {
        let mut columns = read_columns_in(&self.bytes, &self.header, &[name], &self.label)?;
        Ok(columns.remove(0))
    }

    pub fn columns(&self, names: &[&str]) -> Result<Vec<Column>> {
        read_columns_in(&self.bytes, &self.header, names, &self.label)
    }
}

pub fn load_bintable(path: &str, hdu: usize) -> Result<BintableData> {
    let mut file = File::open(path).with_context(|| format!("Failed to open {path}"))?;
    let extensions = list_extensions(&file)?;
    let info = extensions
        .get(hdu)
        .with_context(|| format!("HDU index {hdu} out of range (file has {} HDUs)", extensions.len()))?;
    let parsed = read_header_blocks(&file, info.header_start)?;
    let label = format!("HDU {hdu} ({})", hdu_label(&parsed.header));
    if !is_bintable_hdu(&parsed.header) {
        bail!("{label} is not a BINTABLE");
    }
    let naxis1 = parsed.header.get_i64("NAXIS1").context("Missing NAXIS1")?.max(0) as usize;
    let naxis2 = parsed.header.get_i64("NAXIS2").context("Missing NAXIS2")?.max(0) as usize;
    let needed = naxis1
        .checked_mul(naxis2)
        .with_context(|| format!("{label}: NAXIS1*NAXIS2 overflows (NAXIS1={naxis1} x NAXIS2={naxis2})"))?;
    let file_len = file.metadata().with_context(|| format!("{label}: stat failed"))?.len();
    let available = file_len.saturating_sub(parsed.data_start as u64);
    if needed as u64 > available {
        bail!(
            "{label}: header declares {needed} bytes of table data (NAXIS1={naxis1} x NAXIS2={naxis2}) but only {available} bytes follow the header"
        );
    }
    let mut bytes = vec![0u8; needed];
    file.seek(SeekFrom::Start(parsed.data_start as u64))
        .with_context(|| format!("{label}: seek to table data failed"))?;
    file.read_exact(&mut bytes)
        .with_context(|| format!("{label}: table data truncated"))?;
    Ok(BintableData { label, header: parsed.header, bytes })
}

pub fn read_bintable_columns(path: &str, hdu: usize, names: &[&str]) -> Result<Vec<Column>> {
    load_bintable(path, hdu)?.columns(names)
}

#[cfg(test)]
pub mod test_support {
    use std::path::{Path, PathBuf};

    use super::{parse_tform, tform_elem_bytes, TformKind};
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, write_raw_hdus};

    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    pub fn bintable_cards(
        columns: &[(&str, &str, Option<&str>)],
        rows: usize,
        extra: &[(&'static str, String)],
    ) -> Vec<(&'static str, String)> {
        let mut width = 0usize;
        for (_, tform, _) in columns {
            width += match parse_tform(tform).unwrap() {
                TformKind::VarArray { wide, .. } => {
                    if wide {
                        16
                    } else {
                        8
                    }
                }
                TformKind::Fixed { repeat, type_code } => repeat * tform_elem_bytes(type_code).unwrap(),
            };
        }
        let mut cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", width.to_string()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", columns.len().to_string()),
            ("EXTNAME", "'EXTRACT1D'".into()),
        ];
        for (i, (name, tform, unit)) in columns.iter().enumerate() {
            let n = i + 1;
            cards.push((leak(format!("TTYPE{n}")), format!("'{name:<8}'")));
            cards.push((leak(format!("TFORM{n}")), format!("'{tform:<8}'")));
            if let Some(unit) = unit {
                cards.push((leak(format!("TUNIT{n}")), format!("'{unit:<8}'")));
            }
        }
        cards.extend(extra.iter().cloned());
        cards
    }

    pub fn write_table(
        dir: &Path,
        name: &str,
        columns: &[(&str, &str, Option<&str>)],
        rows: usize,
        data: Vec<u8>,
        extra: &[(&'static str, String)],
    ) -> PathBuf {
        let path = dir.join(name);
        write_raw_hdus(
            &path,
            &[(empty_primary_cards(), Vec::new()), (bintable_cards(columns, rows, extra), data)],
        );
        path
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::write_table;
    use super::*;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;

    fn make_header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for &(k, v) in pairs {
            header.index.insert(k.to_string(), v.to_string());
            header.cards.push((k.to_string(), v.to_string()));
        }
        header
    }

    fn f64s(column: &Column) -> Vec<f64> {
        match &column.values {
            ColumnValues::F64(v) => v.clone(),
            other => panic!("expected F64 values, got {other:?}"),
        }
    }

    fn i64s(column: &Column) -> Vec<i64> {
        match &column.values {
            ColumnValues::I64(v) => v.clone(),
            other => panic!("expected I64 values, got {other:?}"),
        }
    }

    #[test]
    fn read_columns_reads_d_e_and_j_scalars_with_units_and_applies_tzero() {
        let dir = tempfile::tempdir().unwrap();
        let wavelengths = [1.0f64, 1.1, 1.2];
        let fluxes = [10.0f32, 20.0, 30.0];
        let dq_raw = [i32::MIN, i32::MIN + 1, i32::MIN + 3];
        let npix = [5i16, 6, 7];
        let mut data = Vec::new();
        for i in 0..3 {
            data.extend_from_slice(&wavelengths[i].to_be_bytes());
            data.extend_from_slice(&fluxes[i].to_be_bytes());
            data.extend_from_slice(&dq_raw[i].to_be_bytes());
            data.extend_from_slice(&npix[i].to_be_bytes());
        }
        let path = write_table(
            dir.path(),
            "scalars.fits",
            &[("WAVELENGTH", "1D", Some("um")), ("FLUX", "1E", Some("Jy")), ("DQ", "1J", None), ("NPIX", "1I", None)],
            3,
            data,
            &[("TZERO3", "2147483648".into())],
        );

        let columns = read_bintable_columns(path.to_str().unwrap(), 1, &["WAVELENGTH", "FLUX", "DQ", "NPIX"]).unwrap();
        assert_eq!(columns.len(), 4);
        assert_eq!(columns[0].name, "WAVELENGTH");
        assert_eq!(columns[0].unit.as_deref(), Some("um"));
        assert_eq!(f64s(&columns[0]), wavelengths.to_vec());
        assert_eq!(columns[1].unit.as_deref(), Some("Jy"));
        assert_eq!(f64s(&columns[1]), vec![10.0, 20.0, 30.0]);
        assert_eq!(columns[2].unit, None);
        assert_eq!(i64s(&columns[2]), vec![0, 1, 3]);
        assert_eq!(columns[2].null_rows, 0);
        assert_eq!(i64s(&columns[3]), vec![5, 6, 7]);
        for column in &columns {
            assert_eq!(column.null_rows, 0, "{}", column.name);
        }
    }

    #[test]
    fn read_columns_refuses_an_unknown_column_and_lists_the_available_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_table(
            dir.path(),
            "unknown.fits",
            &[("WAVELENGTH", "1D", None), ("FLUX", "1D", None)],
            2,
            vec![0u8; 32],
            &[],
        );
        let err = read_bintable_columns(path.to_str().unwrap(), 1, &["FLUX", "NOPE"]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("column 'NOPE' not in HDU 1 (EXTRACT1D)"), "{msg}");
        assert!(msg.contains("WAVELENGTH, FLUX"), "{msg}");
    }

    #[test]
    fn read_columns_refuses_array_and_variable_length_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_table(
            dir.path(),
            "arrays.fits",
            &[("ARR", "3D", None), ("VAR", "1PD(10)", None), ("OK", "1D", None)],
            1,
            vec![0u8; 40],
            &[],
        );
        let key = path.to_str().unwrap();
        let arr = format!("{:#}", read_bintable_columns(key, 1, &["ARR"]).unwrap_err());
        assert!(arr.contains("column 'ARR' is an array column (TFORM '3D')"), "{arr}");
        assert!(arr.contains("only scalar columns are read"), "{arr}");
        let var = format!("{:#}", read_bintable_columns(key, 1, &["VAR"]).unwrap_err());
        assert!(var.contains("column 'VAR' is an array column (TFORM '1PD(10)')"), "{var}");
        let ok = read_bintable_columns(key, 1, &["OK"]).unwrap();
        assert_eq!(f64s(&ok[0]), vec![0.0]);
    }

    #[test]
    fn read_columns_counts_tnull_rows_in_integer_columns() {
        let dir = tempfile::tempdir().unwrap();
        let raws = [0i32, -1, 5, -1];
        let data: Vec<u8> = raws.iter().flat_map(|v| v.to_be_bytes()).collect();
        let path = write_table(dir.path(), "tnull.fits", &[("DQ", "1J", None)], 4, data, &[("TNULL1", "-1".into())]);
        let columns = read_bintable_columns(path.to_str().unwrap(), 1, &["DQ"]).unwrap();
        assert_eq!(columns[0].null_rows, 2);
        assert_eq!(i64s(&columns[0]), vec![0, -1, 5, -1]);
    }

    fn be_bytes<const N: usize>(rows: &[[u8; N]]) -> Vec<u8> {
        rows.iter().flat_map(|r| r.iter().copied()).collect()
    }

    #[test]
    fn read_columns_reads_unsigned_bytes_negative_shorts_and_negative_longs() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = [0u8, 200, 255];
        let shorts = [-5i16, 6, i16::MIN];
        let longs = [-1i64, i64::MIN, 42];
        let mut data = Vec::new();
        for i in 0..3 {
            data.push(bytes[i]);
            data.extend_from_slice(&shorts[i].to_be_bytes());
            data.extend_from_slice(&longs[i].to_be_bytes());
        }
        let path = write_table(
            dir.path(),
            "ints.fits",
            &[("U8", "1B", None), ("S16", "1I", None), ("S64", "1K", None)],
            3,
            data,
            &[],
        );
        let columns = read_bintable_columns(path.to_str().unwrap(), 1, &["U8", "S16", "S64"]).unwrap();
        assert_eq!(i64s(&columns[0]), vec![0, 200, 255]);
        assert_eq!(i64s(&columns[1]), vec![-5, 6, i16::MIN as i64]);
        assert_eq!(i64s(&columns[2]), vec![-1, i64::MIN, 42]);
    }

    #[test]
    fn read_columns_promotes_scaled_integers_to_f64_with_nan_at_tnull() {
        let dir = tempfile::tempdir().unwrap();
        let raws = [4i32, -1, 6];
        let data = be_bytes(&raws.map(|v| v.to_be_bytes()));
        let path = write_table(
            dir.path(),
            "scaled.fits",
            &[("SCALED", "1J", Some("ct"))],
            3,
            data,
            &[("TSCAL1", "0.5".into()), ("TZERO1", "10".into()), ("TNULL1", "-1".into())],
        );
        let column = &read_bintable_columns(path.to_str().unwrap(), 1, &["SCALED"]).unwrap()[0];
        let values = f64s(column);
        assert_eq!(values[0], 12.0);
        assert!(values[1].is_nan(), "{values:?}");
        assert_eq!(values[2], 13.0);
        assert_eq!(column.null_rows, 1);
        assert_eq!(column.unit.as_deref(), Some("ct"));
    }

    #[test]
    fn read_columns_refuses_an_unsupported_scalar_type() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_table(dir.path(), "logical.fits", &[("FLAG", "1L", None), ("OK", "1D", None)], 1, vec![b'T', 0, 0, 0, 0, 0, 0, 0, 0], &[]);
        let err = format!("{:#}", read_bintable_columns(path.to_str().unwrap(), 1, &["FLAG"]).unwrap_err());
        assert!(err.contains("column 'FLAG': unsupported scalar type 'L'"), "{err}");
        assert!(read_bintable_columns(path.to_str().unwrap(), 1, &["OK"]).is_ok());
    }

    #[test]
    fn read_columns_applies_a_2_63_tzero_to_k_columns_without_saturation() {
        let dir = tempfile::tempdir().unwrap();
        let small = [i64::MIN, i64::MIN + 1, -1];
        let big = [i64::MIN, 0, -1];
        let mut data = Vec::new();
        for i in 0..3 {
            data.extend_from_slice(&small[i].to_be_bytes());
            data.extend_from_slice(&big[i].to_be_bytes());
        }
        let two_63 = "9223372036854775808".to_string();
        let path = write_table(
            dir.path(),
            "u64.fits",
            &[("SMALL", "1K", None), ("BIG", "1K", None)],
            3,
            data,
            &[("TZERO1", two_63.clone()), ("TZERO2", two_63)],
        );
        let columns = read_bintable_columns(path.to_str().unwrap(), 1, &["SMALL", "BIG"]).unwrap();
        assert_eq!(i64s(&columns[0]), vec![0, 1, i64::MAX]);
        assert_eq!(f64s(&columns[1]), vec![0.0, 2f64.powi(63), 2f64.powi(63)]);
    }

    #[test]
    fn read_columns_refuses_a_requested_column_that_appears_twice() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = 1.5f64.to_be_bytes().to_vec();
        data.extend_from_slice(&2.5f32.to_be_bytes());
        data.extend_from_slice(&7i32.to_be_bytes());
        let path = write_table(
            dir.path(),
            "dup.fits",
            &[("FLUX", "1D", Some("Jy")), ("FLUX", "1E", None), ("OK", "1J", None)],
            1,
            data,
            &[],
        );
        let key = path.to_str().unwrap();
        let err = format!("{:#}", read_bintable_columns(key, 1, &["FLUX"]).unwrap_err());
        assert!(err.contains("column 'FLUX' appears 2 times in HDU 1 (EXTRACT1D)"), "{err}");
        assert!(err.contains("TTYPE1, TTYPE2"), "{err}");
        assert_eq!(i64s(&read_bintable_columns(key, 1, &["OK"]).unwrap()[0]), vec![7]);
    }

    #[test]
    fn load_bintable_refuses_a_table_that_declares_more_data_than_the_file_holds() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_table(
            dir.path(),
            "huge.fits",
            &[("A", "1D", None), ("B", "1D", None)],
            100_000_000_000,
            vec![0u8; 16],
            &[],
        );
        let err = format!("{:#}", load_bintable(path.to_str().unwrap(), 1).err().unwrap());
        assert!(err.contains("HDU 1 (EXTRACT1D): header declares 1600000000000 bytes of table data"), "{err}");
        assert!(err.contains("NAXIS1=16 x NAXIS2=100000000000"), "{err}");
        assert!(err.contains("follow the header"), "{err}");
    }

    #[test]
    fn describe_table_and_layout_refuse_a_tfields_outside_the_fits_range() {
        let too_many = make_header(&[
            ("XTENSION", "BINTABLE"),
            ("EXTNAME", "EXTRACT1D"),
            ("NAXIS1", "16"),
            ("NAXIS2", "1"),
            ("TFIELDS", "1000"),
            ("TTYPE1", "WAVELENGTH"),
            ("TFORM1", "1D"),
            ("TTYPE2", "FLUX"),
            ("TFORM2", "1D"),
        ]);
        let described = format!("{:#}", describe_table(&too_many, 1).unwrap_err());
        assert!(described.contains("HDU 1 (EXTRACT1D): TFIELDS=1000 exceeds the FITS limit of 999"), "{described}");
        let layout = format!("{:#}", build_bintable_layout(&too_many, 0).err().unwrap());
        assert!(layout.contains("EXTRACT1D: TFIELDS=1000 exceeds the FITS limit of 999"), "{layout}");

        let negative = make_header(&[("XTENSION", "BINTABLE"), ("NAXIS1", "0"), ("NAXIS2", "0"), ("TFIELDS", "-1")]);
        let described = format!("{:#}", describe_table(&negative, 2).unwrap_err());
        assert!(described.contains("HDU 2 (BINTABLE): TFIELDS=-1 is negative"), "{described}");
        let layout = format!("{:#}", build_bintable_layout(&negative, 0).err().unwrap());
        assert!(layout.contains("BINTABLE: TFIELDS=-1 is negative"), "{layout}");

        let at_limit = make_header(&[("XTENSION", "BINTABLE"), ("NAXIS1", "0"), ("NAXIS2", "0"), ("TFIELDS", "999")]);
        let described = format!("{:#}", describe_table(&at_limit, 3).unwrap_err());
        assert!(described.contains("Missing TFORM1"), "{described}");
        let layout = format!("{:#}", build_bintable_layout(&at_limit, 0).err().unwrap());
        assert!(layout.contains("Missing TFORM1"), "{layout}");
    }

    #[test]
    fn list_tables_refuses_a_table_whose_tfields_exceeds_the_fits_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_table(
            dir.path(),
            "tfields.fits",
            &[("WAVELENGTH", "1D", None), ("FLUX", "1D", None)],
            1,
            vec![0u8; 16],
            &[("TFIELDS", "10000000000".into())],
        );
        let file = File::open(&path).unwrap();
        let err = format!("{:#}", list_tables(&file).err().unwrap());
        assert!(err.contains("HDU 1 (EXTRACT1D): TFIELDS=10000000000 exceeds the FITS limit of 999"), "{err}");
        let err = format!("{:#}", read_bintable_columns(path.to_str().unwrap(), 1, &["FLUX"]).unwrap_err());
        assert!(err.contains("HDU 1 (EXTRACT1D): TFIELDS=10000000000 exceeds the FITS limit of 999"), "{err}");
    }

    #[test]
    fn read_bintable_columns_refuses_an_image_hdu_and_an_out_of_range_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mef.fits");
        sci_err_dq_mef(&path, 4, 4, vec![0i32; 16]);
        let key = path.to_str().unwrap();
        let image = format!("{:#}", read_bintable_columns(key, 1, &["X"]).unwrap_err());
        assert!(image.contains("HDU 1 (SCI) is not a BINTABLE"), "{image}");
        let range = format!("{:#}", read_bintable_columns(key, 9, &["X"]).unwrap_err());
        assert!(range.contains("out of range"), "{range}");
        assert!(range.contains("5 HDUs"), "{range}");
    }

    #[test]
    fn promoted_layout_still_sums_row_width_and_resolves_heap_base() {
        let header = make_header(&[
            ("XTENSION", "BINTABLE"),
            ("NAXIS1", "40"),
            ("NAXIS2", "2"),
            ("TFIELDS", "3"),
            ("TTYPE1", "COMPRESSED_DATA"),
            ("TFORM1", "1PB(0)"),
            ("TTYPE2", "ZSCALE"),
            ("TFORM2", "1D"),
            ("TTYPE3", "ARR"),
            ("TFORM3", "3D"),
            ("THEAP", "96"),
        ]);
        let layout = build_bintable_layout(&header, 2880).unwrap();
        assert_eq!(layout.row_width, 40);
        assert_eq!(layout.n_rows, 2);
        assert_eq!(layout.heap_base, 2976);
        assert_eq!(
            layout.columns["COMPRESSED_DATA"],
            ColumnSpan::VarArray { offset: 0, elem_bytes: 1, wide: false }
        );
        assert_eq!(layout.columns["ZSCALE"], ColumnSpan::Fixed { offset: 8, type_code: 'D', repeat: 1 });
        assert_eq!(layout.columns["ARR"], ColumnSpan::Fixed { offset: 16, type_code: 'D', repeat: 3 });
        let row = [0u8; 40];
        assert!(read_fixed_f64(&row, &layout.columns["ZSCALE"]).is_ok());
        assert!(read_fixed_f64(&row, &layout.columns["COMPRESSED_DATA"]).is_err());
        assert_eq!(read_var_descriptor(&row, &layout.columns["COMPRESSED_DATA"]).unwrap(), (0, 0));
    }
}
