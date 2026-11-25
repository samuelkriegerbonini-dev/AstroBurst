use std::collections::HashSet;
use std::fs::File;
use std::ops::Range;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::core::analysis::tso::{
    SeriesKey, TsoFile, TsoLayout, WhiteLightLog, DATAMODEL_EXTENSIONS, DATAMODEL_ROWS, DQ_DO_NOT_USE, UNKNOWN_DETECTOR,
};
use crate::core::analysis::x1d::{COL_DQ, COL_FLUX, COL_FLUX_ERROR, COL_NPIXELS, COL_WAVELENGTH, EXTNAME_EXTRACT1D};
use crate::core::astrometry::spectral::wavelength_unit_to_um;
use crate::infra::fits::asdf_hdu::{find_asdf_hdu, read_asdf_hdu_bytes};
use crate::infra::fits::gzip::{resolve_gzip, split_gz_suffix};
use crate::infra::fits::reader::{read_header_blocks, ParsedHdu};
use crate::infra::fits::table::{
    bintable_reader, card_text, list_tables, BintableReader, Column, ColumnInfo, ColumnValues, TableHdu,
};
use crate::types::constants::DEFAULT_WAVELENGTH_UNIT;
use crate::types::header::HduHeader;

pub const COL_INT_NUM: &str = "INT_NUM";
pub const COL_N_ALONGDISP: &str = "N_ALONGDISP";
pub const COL_SEGMENT: &str = "SEGMENT";
pub const COL_MJD_AVG: &str = "MJD-AVG";
pub const COL_TDB_MID: &str = "TDB-MID";

pub const TSO_SET_BYTE_CAP: u64 = 1 << 30;
pub const READ_CHUNK_BYTES: usize = 8 << 20;
pub const UNSIGNED_TZERO: i64 = 2_147_483_648;

const CARD_DATAMODL: &str = "DATAMODL";
const CARD_DETECTOR: &str = "DETECTOR";
const CARD_SPORDER: &str = "SPORDER";
const CARD_S_OUTLIR: &str = "S_OUTLIR";
const CARD_ASNTABLE: &str = "ASNTABLE";
const CARD_EXSEGNUM: &str = "EXSEGNUM";
const CARD_PCOUNT: &str = "PCOUNT";
const LEVEL3_ASSOCIATION: &str = "tso3";
const X1DINTS_ENDING: &str = "_x1dints.fits";
const WHTLT_ENDING: &str = "_whtlt.ecsv";
const LOG_MIN_WAVELENGTH: &str = "min_wavelength:";
const LOG_MAX_WAVELENGTH: &str = "max_wavelength:";
const LOG_LIMIT_NOT_SET: &str = "None";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X1dIntsShape {
    pub layout: TsoLayout,
    pub n_series: usize,
    pub n_integrations: usize,
    pub n_channels: usize,
}

pub fn series_key(extension: &HduHeader, primary: &HduHeader) -> SeriesKey {
    let detector = card_text(extension, CARD_DETECTOR)
        .or_else(|| card_text(primary, CARD_DETECTOR))
        .unwrap_or_else(|| UNKNOWN_DETECTOR.to_string());
    SeriesKey { detector, sporder: extension.get_i64(CARD_SPORDER) }
}

fn is_extract1d(table: &TableHdu) -> bool {
    table
        .extname
        .as_deref()
        .is_some_and(|name| name.trim().eq_ignore_ascii_case(EXTNAME_EXTRACT1D))
}

pub fn is_rows_table(table: &TableHdu) -> bool {
    [COL_INT_NUM, COL_N_ALONGDISP, COL_WAVELENGTH]
        .iter()
        .all(|name| table.columns.iter().any(|column| column.name == *name))
}

pub fn x1dints_shape(primary: &HduHeader, tables: &[(TableHdu, ParsedHdu)]) -> Option<X1dIntsShape> {
    let extract1d: Vec<&(TableHdu, ParsedHdu)> = tables.iter().filter(|(table, _)| is_extract1d(table)).collect();
    let (first_table, _) = extract1d.first()?;
    let datamodl = card_text(primary, CARD_DATAMODL);
    let every_table_has_rows = extract1d.iter().all(|(table, _)| is_rows_table(table));
    let every_header_has_int_num = extract1d.iter().all(|(_, parsed)| parsed.header.get_i64(COL_INT_NUM).is_some());
    let layout = match datamodl.as_deref() {
        None | Some(DATAMODEL_ROWS | DATAMODEL_EXTENSIONS) if every_table_has_rows => TsoLayout::Rows,
        Some(DATAMODEL_ROWS) => TsoLayout::Rows,
        Some(DATAMODEL_EXTENSIONS) if every_header_has_int_num => TsoLayout::Extensions,
        _ => return None,
    };
    let keys: Vec<SeriesKey> = extract1d.iter().map(|(_, parsed)| series_key(&parsed.header, primary)).collect();
    let first_key = &keys[0];
    let n_series = keys.iter().collect::<HashSet<_>>().len();
    let tables_of_first_key = extract1d.iter().zip(&keys).filter(|(_, key)| *key == first_key).map(|(entry, _)| &entry.0);
    let (n_integrations, n_channels) = match layout {
        TsoLayout::Rows => (
            tables_of_first_key.map(|table| table.n_rows).sum(),
            first_table.columns.iter().find(|column| column.name == COL_WAVELENGTH).map_or(0, |column| column.repeat),
        ),
        TsoLayout::Extensions => (tables_of_first_key.count(), first_table.n_rows),
    };
    Some(X1dIntsShape { layout, n_series, n_integrations, n_channels })
}

pub fn probe_x1dints(file: &File) -> Result<Option<X1dIntsShape>> {
    let primary = read_header_blocks(file, 0)?.header;
    Ok(x1dints_shape(&primary, &list_tables(file)?))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadBudget {
    pub cap: u64,
    pub used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeOrigin {
    Column,
    Keyword,
    Absent,
}

#[derive(Debug, Clone)]
pub struct X1dIntsTable {
    pub hdu: usize,
    pub extver: Option<i64>,
    pub key: SeriesKey,
    pub n_channels: usize,
    pub flux_unit: Option<String>,
    pub time_origin: TimeOrigin,
    pub int_num: Vec<i64>,
    pub segment: Vec<i64>,
    pub n_alongdisp: Vec<u32>,
    pub mjd_utc_mid: Vec<f64>,
    pub bmjd_tdb_mid: Vec<f64>,
    pub wavelength_um: Vec<f64>,
    pub flux: Vec<f64>,
    pub flux_error: Vec<f64>,
    pub dq: Vec<u32>,
    pub npixels: Vec<f64>,
    pub rows_dropped_empty: usize,
}

#[derive(Debug, Clone)]
pub struct X1dIntsFile {
    pub file: TsoFile,
    pub tables: Vec<X1dIntsTable>,
    pub notes: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    Required,
    Optional,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Vector,
    Scalar,
}

struct ColumnRule {
    name: &'static str,
    type_code: char,
    need: Need,
    shape: Shape,
}

const fn rule(name: &'static str, type_code: char, need: Need, shape: Shape) -> ColumnRule {
    ColumnRule { name, type_code, need, shape }
}

const ROWS_COLUMNS: [ColumnRule; 10] = [
    rule(COL_INT_NUM, 'J', Need::Required, Shape::Scalar),
    rule(COL_N_ALONGDISP, 'J', Need::Required, Shape::Scalar),
    rule(COL_WAVELENGTH, 'D', Need::Required, Shape::Vector),
    rule(COL_FLUX, 'D', Need::Required, Shape::Vector),
    rule(COL_FLUX_ERROR, 'D', Need::Optional, Shape::Vector),
    rule(COL_DQ, 'J', Need::Optional, Shape::Vector),
    rule(COL_NPIXELS, 'D', Need::Optional, Shape::Vector),
    rule(COL_SEGMENT, 'J', Need::Optional, Shape::Scalar),
    rule(COL_MJD_AVG, 'D', Need::Optional, Shape::Scalar),
    rule(COL_TDB_MID, 'D', Need::Optional, Shape::Scalar),
];

const EXTENSION_COLUMNS: [ColumnRule; 5] = [
    rule(COL_WAVELENGTH, 'D', Need::Required, Shape::Scalar),
    rule(COL_FLUX, 'D', Need::Required, Shape::Scalar),
    rule(COL_FLUX_ERROR, 'D', Need::Optional, Shape::Scalar),
    rule(COL_DQ, 'J', Need::Optional, Shape::Scalar),
    rule(COL_NPIXELS, 'D', Need::Optional, Shape::Scalar),
];

struct Source<'a> {
    file: &'a File,
    primary: &'a HduHeader,
    layout: TsoLayout,
    chunk_bytes: usize,
}

struct Samples {
    wavelength_um: Vec<f64>,
    flux: Vec<f64>,
    flux_error: Vec<f64>,
    dq: Vec<u32>,
    npixels: Vec<f64>,
}

pub fn read_x1dints_file(path: &str, budget: &mut ReadBudget) -> Result<X1dIntsFile> {
    read_x1dints_file_with(path, budget, READ_CHUNK_BYTES)
}

pub(crate) fn read_x1dints_file_with(path: &str, budget: &mut ReadBudget, chunk_bytes: usize) -> Result<X1dIntsFile> {
    let given = Path::new(path);
    let name = given.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let (readable, _decompressed) = resolve_gzip(given)?;
    let file = File::open(&readable).with_context(|| format!("Failed to open {path}"))?;
    let (primary, tables) = read_header_blocks(&file, 0)
        .and_then(|parsed| Ok((parsed.header, list_tables(&file)?)))
        .map_err(|error| anyhow!("{name}: {error:#}"))?;

    let datamodel = card_text(&primary, CARD_DATAMODL);
    if let Some(found) = datamodel.as_deref().filter(|found| ![DATAMODEL_ROWS, DATAMODEL_EXTENSIONS].contains(found)) {
        bail!(
            "{name}: {CARD_DATAMODL} '{found}' is not a time series of spectra ({DATAMODEL_ROWS} or {DATAMODEL_EXTENSIONS} expected)"
        );
    }
    let extract1d: Vec<&(TableHdu, ParsedHdu)> = tables.iter().filter(|(table, _)| is_extract1d(table)).collect();
    if extract1d.is_empty() {
        bail!("{name}: no {EXTNAME_EXTRACT1D} binary table; tables in this file: {}", inventory(&tables));
    }
    let layout = match (x1dints_shape(&primary, &tables), &datamodel) {
        (Some(shape), _) => shape.layout,
        (None, Some(_)) => bail!(
            "{name}: its {EXTNAME_EXTRACT1D} tables carry no {COL_INT_NUM} card: this is a plain spectrum, not a time series"
        ),
        (None, None) => bail!(
            "{name}: {CARD_DATAMODL} card missing and its {EXTNAME_EXTRACT1D} tables are not one row per integration (columns {COL_INT_NUM}, {COL_N_ALONGDISP} and {COL_WAVELENGTH} expected): not a time series"
        ),
    };

    let data_bytes =
        extract1d.iter().fold(0u64, |total, (_, parsed)| total.saturating_add(declared_bytes(&parsed.header)));
    let wanted = budget.used.saturating_add(data_bytes);
    if wanted > budget.cap {
        bail!(
            "{name}: the {EXTNAME_EXTRACT1D} tables of the files given hold {wanted} bytes, more than the {} bytes this reader loads at once; open fewer files",
            budget.cap
        );
    }
    budget.used = wanted;

    let mut notes = Vec::new();
    if datamodel.is_none() {
        notes.push(format!("{CARD_DATAMODL} card missing: layout taken from the table columns"));
    }
    let source = Source { file: &file, primary: &primary, layout, chunk_bytes };
    let mut read = Vec::with_capacity(extract1d.len());
    for (table, parsed) in &extract1d {
        let label = format!("HDU {} ({EXTNAME_EXTRACT1D})", table.index);
        if layout == TsoLayout::Extensions && table.n_rows == 0 {
            notes.push(empty_rows_note(&label, 1));
            continue;
        }
        let spectra = read_table(&source, table, parsed, &label, &mut notes)
            .map_err(|error| table_error(&name, &label, error))?;
        read.push(spectra);
    }

    let after_outlier_detection = primary.get(CARD_S_OUTLIR).is_some();
    let level3_association = card_text(&primary, CARD_ASNTABLE)
        .is_some_and(|association| association.to_ascii_lowercase().contains(LEVEL3_ASSOCIATION));
    let level = if after_outlier_detection || level3_association { 3 } else { 2 };
    let text = |card: &str| card_text(&primary, card);
    Ok(X1dIntsFile {
        file: TsoFile {
            path: path.to_string(),
            file_name: name.clone(),
            sha256: sha256_hex(path)?,
            level,
            datamodel,
            cal_ver: text("CAL_VER"),
            exp_type: text("EXP_TYPE"),
            layout,
            nints: primary.get_i64("NINTS"),
            intstart: primary.get_i64("INTSTART"),
            intend: primary.get_i64("INTEND"),
            exsegnum: primary.get_i64(CARD_EXSEGNUM),
            exsegtot: primary.get_i64("EXSEGTOT"),
            instrument: text("INSTRUME"),
            detector: text(CARD_DETECTOR),
            filter: text("FILTER"),
            pupil: text("PUPIL"),
            grating: text("GRATING"),
            subarray: text("SUBARRAY"),
            target: text("TARGNAME"),
            expstart: primary.get_f64("EXPSTART"),
            outlier_detection: text(CARD_S_OUTLIR),
            whitelight_log: logged_limits(&file),
            whtlt_path: if level == 3 { sibling_whtlt(given, &name) } else { None },
            n_extract1d: extract1d.len(),
            data_bytes,
        },
        tables: read,
        notes,
    })
}

fn declared_bytes(header: &HduHeader) -> u64 {
    let axis = |card: &str| header.get_i64(card).unwrap_or(0).max(0) as u64;
    axis("NAXIS1").saturating_mul(axis("NAXIS2"))
}

fn inventory(tables: &[(TableHdu, ParsedHdu)]) -> String {
    if tables.is_empty() {
        return "none".to_string();
    }
    tables
        .iter()
        .map(|(table, _)| format!("[{}] {} ({} rows)", table.index, table.extname.as_deref().unwrap_or("(none)"), table.n_rows))
        .collect::<Vec<_>>()
        .join(", ")
}

fn empty_rows_note(label: &str, dropped: usize) -> String {
    format!("{label}: {dropped} row(s) with {COL_N_ALONGDISP} = 0 dropped")
}

fn table_error(name: &str, label: &str, error: anyhow::Error) -> anyhow::Error {
    let reason = format!("{error:#}");
    if reason.contains(label) {
        anyhow!("{name}: {reason}")
    } else {
        anyhow!("{name}: {label}: {reason}")
    }
}

fn sha256_hex(path: &str) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("Failed to open {path}"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).with_context(|| format!("Failed to read {path}"))?;
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn logged_limits(file: &File) -> Option<WhiteLightLog> {
    let hdu = find_asdf_hdu(file).ok()??;
    whitelight_log_limits(&read_asdf_hdu_bytes(file, &hdu).ok()?)
}

pub fn whitelight_log_limits(asdf_cell: &[u8]) -> Option<WhiteLightLog> {
    let text = String::from_utf8_lossy(asdf_cell);
    Some(WhiteLightLog {
        min_um: logged_limit(&text, LOG_MIN_WAVELENGTH)?,
        max_um: logged_limit(&text, LOG_MAX_WAVELENGTH)?,
    })
}

fn logged_limit(text: &str, key: &str) -> Option<Option<f64>> {
    let value = text[text.find(key)? + key.len()..].trim_start_matches(' ');
    let end = value
        .find(|c: char| matches!(c, '\\' | '"' | '\'' | ',') || c.is_whitespace())
        .unwrap_or(value.len());
    match &value[..end] {
        LOG_LIMIT_NOT_SET => Some(None),
        number => number.parse::<f64>().ok().filter(|limit| limit.is_finite()).map(Some),
    }
}

fn sibling_whtlt(path: &Path, name: &str) -> Option<String> {
    let (plain, _) = split_gz_suffix(name);
    let stem_len = plain.len().checked_sub(X1DINTS_ENDING.len()).filter(|&len| plain.is_char_boundary(len))?;
    if !plain[stem_len..].eq_ignore_ascii_case(X1DINTS_ENDING) {
        return None;
    }
    let sibling = path.with_file_name(format!("{}{WHTLT_ENDING}", &plain[..stem_len]));
    sibling.is_file().then(|| sibling.to_string_lossy().into_owned())
}

fn read_table(
    source: &Source,
    table: &TableHdu,
    parsed: &ParsedHdu,
    label: &str,
    notes: &mut Vec<String>,
) -> Result<X1dIntsTable> {
    let header = &parsed.header;
    let pcount = header.get_i64(CARD_PCOUNT).unwrap_or(0);
    if pcount != 0 {
        bail!("{CARD_PCOUNT} = {pcount}, variable-length data is not read");
    }
    let rules: &[ColumnRule] = match source.layout {
        TsoLayout::Rows => &ROWS_COLUMNS,
        TsoLayout::Extensions => &EXTENSION_COLUMNS,
    };
    check_columns(table, rules, source.layout)?;
    let column = |name: &str| table.columns.iter().find(|column| column.name == name);
    let scale = match column(COL_WAVELENGTH).and_then(|wavelength| wavelength.unit.as_deref()) {
        None => {
            notes.push(format!("{label}: TUNIT of {COL_WAVELENGTH} missing: assumed {DEFAULT_WAVELENGTH_UNIT}"));
            1.0
        }
        Some(unit) => wavelength_unit_to_um(unit).with_context(|| format!("unknown wavelength unit '{unit}'"))?,
    };
    let (present, missing): (Vec<&str>, Vec<&str>) =
        rules.iter().map(|rule| rule.name).partition(|name| column(*name).is_some());
    let (n_channels, time_origin) = match source.layout {
        TsoLayout::Rows => (
            column(COL_WAVELENGTH).map_or(0, |wavelength| wavelength.repeat),
            if column(COL_MJD_AVG).or(column(COL_TDB_MID)).is_some() { TimeOrigin::Column } else { TimeOrigin::Absent },
        ),
        TsoLayout::Extensions => (
            table.n_rows,
            if header.get_f64(COL_MJD_AVG).or(header.get_f64(COL_TDB_MID)).is_some() {
                TimeOrigin::Keyword
            } else {
                TimeOrigin::Absent
            },
        ),
    };

    let mut reader = bintable_reader(source.file, table.index, parsed)?;
    let mut spectra = X1dIntsTable {
        hdu: table.index,
        extver: table.extver,
        key: series_key(header, source.primary),
        n_channels,
        flux_unit: column(COL_FLUX).and_then(|flux| flux.unit.clone()),
        time_origin,
        int_num: Vec::new(),
        segment: Vec::new(),
        n_alongdisp: Vec::new(),
        mjd_utc_mid: Vec::new(),
        bmjd_tdb_mid: Vec::new(),
        wavelength_um: Vec::new(),
        flux: Vec::new(),
        flux_error: Vec::new(),
        dq: Vec::new(),
        npixels: Vec::new(),
        rows_dropped_empty: 0,
    };
    match source.layout {
        TsoLayout::Rows => fill_from_rows(&mut spectra, &mut reader, &present, scale, source.chunk_bytes)?,
        TsoLayout::Extensions => fill_from_extension(&mut spectra, &mut reader, &present, scale, source)?,
    }
    if spectra.rows_dropped_empty > 0 {
        notes.push(empty_rows_note(label, spectra.rows_dropped_empty));
    }
    notes.extend(missing.iter().map(|name| format!("{label}: column {name} missing")));
    Ok(spectra)
}

fn check_columns(table: &TableHdu, rules: &[ColumnRule], layout: TsoLayout) -> Result<()> {
    let column = |name: &str| table.columns.iter().find(|column| column.name == name);
    if let Some(absent) = rules.iter().find(|rule| rule.need == Need::Required && column(rule.name).is_none()) {
        let available = table.columns.iter().map(|column| column.name.as_str()).collect::<Vec<_>>().join(", ");
        bail!("column {} missing; columns: {available}", absent.name);
    }
    let present: Vec<(&ColumnRule, &ColumnInfo)> =
        rules.iter().filter_map(|rule| Some((rule, column(rule.name)?))).collect();
    for (rule, info) in &present {
        let scalar = rule.shape == Shape::Scalar;
        if info.type_code != rule.type_code || (scalar && info.repeat != 1) {
            let wording = if scalar && (info.repeat != 1 || layout == TsoLayout::Extensions) { "scalar " } else { "" };
            bail!("column {} has TFORM '{}', expected a {wording}{} column", info.name, info.tform, rule.type_code);
        }
    }
    let vectors: Vec<&ColumnInfo> =
        present.iter().filter(|(rule, _)| rule.shape == Shape::Vector).map(|(_, info)| *info).collect();
    if vectors.windows(2).any(|pair| pair[0].repeat != pair[1].repeat) {
        let lengths = vectors.iter().map(|info| format!("{} {}", info.name, info.repeat)).collect::<Vec<_>>().join(", ");
        bail!("vector columns differ in length ({lengths})");
    }
    Ok(())
}

fn fill_from_rows(
    spectra: &mut X1dIntsTable,
    reader: &mut BintableReader,
    names: &[&str],
    scale: f64,
    chunk_bytes: usize,
) -> Result<()> {
    let n_channels = spectra.n_channels;
    for_each_chunk(reader, names, chunk_bytes, |rows, mut columns| {
        let int_num = integers(required_values(&mut columns, COL_INT_NUM)?);
        let n_alongdisp = unsigned(required_values(&mut columns, COL_N_ALONGDISP)?);
        let segment = take_values(&mut columns, COL_SEGMENT).map_or_else(|| vec![1; rows.len()], integers);
        let mjd_avg = take_values(&mut columns, COL_MJD_AVG).map_or_else(|| vec![f64::NAN; rows.len()], floats);
        let tdb_mid = take_values(&mut columns, COL_TDB_MID).map_or_else(|| vec![f64::NAN; rows.len()], floats);
        let samples = take_samples(&mut columns, scale)?;
        for row in 0..rows.len() {
            let valid = n_alongdisp[row] as usize;
            if valid > n_channels {
                bail!(
                    "row {} has {COL_N_ALONGDISP} = {valid} beyond the {n_channels} elements of a row",
                    rows.start + row + 1
                );
            }
            if valid == 0 {
                spectra.rows_dropped_empty += 1;
                continue;
            }
            spectra.int_num.push(int_num[row]);
            spectra.segment.push(segment[row]);
            spectra.n_alongdisp.push(n_alongdisp[row]);
            spectra.mjd_utc_mid.push(mjd_avg[row]);
            spectra.bmjd_tdb_mid.push(tdb_mid[row]);
            let channels = row * n_channels..(row + 1) * n_channels;
            append_row(&mut spectra.wavelength_um, &samples.wavelength_um[channels.clone()], valid, f64::NAN);
            append_row(&mut spectra.flux, &samples.flux[channels.clone()], valid, f64::NAN);
            append_row(&mut spectra.flux_error, &samples.flux_error[channels.clone()], valid, f64::NAN);
            append_row(&mut spectra.dq, &samples.dq[channels.clone()], valid, DQ_DO_NOT_USE);
            append_row(&mut spectra.npixels, &samples.npixels[channels], valid, 0.0);
        }
        Ok(())
    })
}

fn fill_from_extension(
    spectra: &mut X1dIntsTable,
    reader: &mut BintableReader,
    names: &[&str],
    scale: f64,
    source: &Source,
) -> Result<()> {
    let header = reader.header();
    let int_num = header.get_i64(COL_INT_NUM).with_context(|| format!("no {COL_INT_NUM} card"))?;
    spectra.int_num.push(int_num);
    spectra.segment.push(source.primary.get_i64(CARD_EXSEGNUM).unwrap_or(1));
    spectra.n_alongdisp.push(u32::try_from(spectra.n_channels).unwrap_or(u32::MAX));
    spectra.mjd_utc_mid.push(header.get_f64(COL_MJD_AVG).unwrap_or(f64::NAN));
    spectra.bmjd_tdb_mid.push(header.get_f64(COL_TDB_MID).unwrap_or(f64::NAN));
    for_each_chunk(reader, names, source.chunk_bytes, |_, mut columns| {
        let samples = take_samples(&mut columns, scale)?;
        spectra.wavelength_um.extend(samples.wavelength_um);
        spectra.flux.extend(samples.flux);
        spectra.flux_error.extend(samples.flux_error);
        spectra.dq.extend(samples.dq);
        spectra.npixels.extend(samples.npixels);
        Ok(())
    })
}

fn for_each_chunk(
    reader: &mut BintableReader,
    names: &[&str],
    chunk_bytes: usize,
    mut handle: impl FnMut(Range<usize>, Vec<Column>) -> Result<()>,
) -> Result<()> {
    let rows_per_chunk = (chunk_bytes / reader.row_width().max(1)).max(1);
    let mut start = 0;
    while start < reader.n_rows() {
        let rows = start..start.saturating_add(rows_per_chunk).min(reader.n_rows());
        start = rows.end;
        handle(rows.clone(), reader.read_rows(rows, names)?)?;
    }
    Ok(())
}

fn take_values(columns: &mut Vec<Column>, name: &str) -> Option<ColumnValues> {
    let at = columns.iter().position(|column| column.name == name)?;
    Some(columns.swap_remove(at).values)
}

fn required_values(columns: &mut Vec<Column>, name: &str) -> Result<ColumnValues> {
    take_values(columns, name).with_context(|| format!("column {name} was not read"))
}

fn take_samples(columns: &mut Vec<Column>, scale: f64) -> Result<Samples> {
    let wavelength_um: Vec<f64> =
        floats(required_values(columns, COL_WAVELENGTH)?).into_iter().map(|wavelength| wavelength * scale).collect();
    let count = wavelength_um.len();
    Ok(Samples {
        flux: floats(required_values(columns, COL_FLUX)?),
        flux_error: take_values(columns, COL_FLUX_ERROR).map_or_else(|| vec![f64::NAN; count], floats),
        dq: take_values(columns, COL_DQ).map_or_else(|| vec![0; count], unsigned),
        npixels: take_values(columns, COL_NPIXELS).map_or_else(|| vec![f64::NAN; count], floats),
        wavelength_um,
    })
}

fn floats(values: ColumnValues) -> Vec<f64> {
    match values {
        ColumnValues::F64(values) => values,
        ColumnValues::I64(values) => values.into_iter().map(|value| value as f64).collect(),
    }
}

fn integers(values: ColumnValues) -> Vec<i64> {
    match values {
        ColumnValues::I64(values) => values,
        ColumnValues::F64(values) => values.into_iter().map(|value| value as i64).collect(),
    }
}

fn unsigned(values: ColumnValues) -> Vec<u32> {
    integers(values).into_iter().map(|value| value.clamp(0, i64::from(u32::MAX)) as u32).collect()
}

fn append_row<T: Copy>(target: &mut Vec<T>, row: &[T], valid: usize, padding: T) {
    target.extend_from_slice(&row[..valid]);
    target.resize(target.len() + row.len() - valid, padding);
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use std::path::{Path, PathBuf};

    use super::UNSIGNED_TZERO;
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, write_raw_hdus};
    use crate::infra::fits::table::test_support::bintable_cards;

    pub type RawHdu = (Vec<(&'static str, String)>, Vec<u8>);

    pub struct SynthRow {
        pub int_num: i32,
        pub n_alongdisp: u32,
        pub segment: i32,
        pub mjd_avg: f64,
        pub tdb_mid: f64,
        pub wavelength: Vec<f64>,
        pub flux: Vec<f64>,
        pub flux_error: Vec<f64>,
        pub dq: Vec<u32>,
        pub npixels: Vec<f64>,
    }

    pub struct SynthTable {
        pub extver: i64,
        pub detector: Option<&'static str>,
        pub sporder: Option<i64>,
        pub flux_unit: Option<&'static str>,
        pub rows: Vec<SynthRow>,
    }

    pub fn ramp_row(int_num: i32, time: f64, channels: usize) -> SynthRow {
        SynthRow {
            int_num,
            n_alongdisp: channels as u32,
            segment: 1,
            mjd_avg: time,
            tdb_mid: time + 0.002,
            wavelength: (0..channels).map(|channel| 1.0 + 0.1 * channel as f64).collect(),
            flux: (0..channels).map(|channel| 10.0 * f64::from(int_num) + channel as f64).collect(),
            flux_error: vec![0.5; channels],
            dq: vec![0; channels],
            npixels: vec![7.0; channels],
        }
    }

    pub fn series_primary(datamodl: &str, extra: &[(&'static str, String)]) -> Vec<(&'static str, String)> {
        let mut cards = empty_primary_cards();
        cards.push(("DATAMODL", format!("'{datamodl}'")));
        cards.extend(extra.iter().cloned());
        cards
    }

    fn stored_unsigned(value: u32) -> [u8; 4] {
        ((i64::from(value) - UNSIGNED_TZERO) as i32).to_be_bytes()
    }

    fn stored_floats(values: &[f64]) -> impl Iterator<Item = u8> + '_ {
        values.iter().flat_map(|value| value.to_be_bytes())
    }

    pub fn rows_hdu(table: &SynthTable) -> RawHdu {
        let channels = table.rows.first().map_or(0, |row| row.wavelength.len());
        let vector = format!("{channels}D");
        let flags = format!("{channels}J");
        let shape = format!("'({channels})'");
        let mut extra: Vec<(&'static str, String)> = vec![
            ("TZERO5", UNSIGNED_TZERO.to_string()),
            ("TZERO7", UNSIGNED_TZERO.to_string()),
            ("TDIM2", shape.clone()),
            ("TDIM3", shape.clone()),
            ("TDIM4", shape.clone()),
            ("TDIM5", shape.clone()),
            ("TDIM6", shape),
            ("EXTVER", table.extver.to_string()),
        ];
        if let Some(detector) = table.detector {
            extra.push(("DETECTOR", format!("'{detector}'")));
        }
        if let Some(sporder) = table.sporder {
            extra.push(("SPORDER", sporder.to_string()));
        }
        let cards = bintable_cards(
            &[
                ("INT_NUM", "1J", None),
                ("WAVELENGTH", &vector, Some("um")),
                ("FLUX", &vector, table.flux_unit),
                ("FLUX_ERROR", &vector, table.flux_unit),
                ("DQ", &flags, None),
                ("NPIXELS", &vector, None),
                ("N_ALONGDISP", "1J", None),
                ("SEGMENT", "1J", None),
                ("MJD-AVG", "1D", Some("d")),
                ("TDB-MID", "1D", Some("d")),
            ],
            table.rows.len(),
            &extra,
        );
        let mut data = Vec::new();
        for row in &table.rows {
            data.extend(row.int_num.to_be_bytes());
            data.extend(stored_floats(&row.wavelength));
            data.extend(stored_floats(&row.flux));
            data.extend(stored_floats(&row.flux_error));
            data.extend(row.dq.iter().flat_map(|&flags| stored_unsigned(flags)));
            data.extend(stored_floats(&row.npixels));
            data.extend(stored_unsigned(row.n_alongdisp));
            data.extend(row.segment.to_be_bytes());
            data.extend(row.mjd_avg.to_be_bytes());
            data.extend(row.tdb_mid.to_be_bytes());
        }
        (cards, data)
    }

    pub fn extension_hdu(extver: i64, cards: &[(&'static str, String)], row: &SynthRow) -> RawHdu {
        let mut extra: Vec<(&'static str, String)> =
            vec![("TZERO4", UNSIGNED_TZERO.to_string()), ("EXTVER", extver.to_string())];
        extra.extend(cards.iter().cloned());
        let header = bintable_cards(
            &[
                ("WAVELENGTH", "1D", Some("um")),
                ("FLUX", "1D", Some("Jy")),
                ("FLUX_ERROR", "1D", Some("Jy")),
                ("DQ", "1J", None),
                ("NPIXELS", "1D", None),
            ],
            row.wavelength.len(),
            &extra,
        );
        let mut data = Vec::new();
        for channel in 0..row.wavelength.len() {
            data.extend(row.wavelength[channel].to_be_bytes());
            data.extend(row.flux[channel].to_be_bytes());
            data.extend(row.flux_error[channel].to_be_bytes());
            data.extend(stored_unsigned(row.dq[channel]));
            data.extend(row.npixels[channel].to_be_bytes());
        }
        (header, data)
    }

    pub fn write_series_file(dir: &Path, name: &str, primary: Vec<(&'static str, String)>, hdus: Vec<RawHdu>) -> PathBuf {
        let path = dir.join(name);
        let mut all = vec![(primary, Vec::new())];
        all.extend(hdus);
        write_raw_hdus(&path, &all);
        path
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::path::PathBuf;
    use std::time::Instant;

    use super::test_fixtures::{
        extension_hdu, ramp_row, rows_hdu, series_primary, write_series_file, RawHdu, SynthRow, SynthTable,
    };
    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::asdf_hdu::test_fixtures::{asdf_bintable_hdu, set_card};
    use crate::infra::fits::gzip::test_support::gzip_copy;
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, write_raw_hdus};
    use crate::infra::fits::table::test_support::bintable_cards;

    type Hdu = (Vec<(&'static str, String)>, Vec<u8>);

    fn owned(cards: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        cards.iter().map(|(key, value)| (*key, value.to_string())).collect()
    }

    fn primary(cards: &[(&'static str, &str)]) -> Hdu {
        let mut all = empty_primary_cards();
        all.extend(owned(cards));
        (all, Vec::new())
    }

    fn table(columns: &[(&str, &str, Option<&str>)], rows: usize, extra: &[(&'static str, &str)]) -> Hdu {
        let cards = bintable_cards(columns, rows, &owned(extra));
        let width: usize = cards.iter().find(|(key, _)| *key == "NAXIS1").unwrap().1.parse().unwrap();
        (cards, vec![0u8; width * rows])
    }

    fn rows_table(rows: usize, channels: usize, extra: &[(&'static str, &str)]) -> Hdu {
        let vector = format!("{channels}D");
        table(
            &[("INT_NUM", "1J", None), ("WAVELENGTH", &vector, None), ("FLUX", &vector, None), ("N_ALONGDISP", "1J", None)],
            rows,
            extra,
        )
    }

    fn scalar_table(rows: usize, extra: &[(&'static str, &str)]) -> Hdu {
        table(&[("WAVELENGTH", "1D", None), ("FLUX", "1D", None)], rows, extra)
    }

    fn probe(hdus: &[Hdu]) -> Option<X1dIntsShape> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("series.fits");
        write_raw_hdus(&path, hdus);
        probe_x1dints(&File::open(&path).unwrap()).unwrap()
    }

    fn shape(layout: TsoLayout, n_series: usize, n_integrations: usize, n_channels: usize) -> Option<X1dIntsShape> {
        Some(X1dIntsShape { layout, n_series, n_integrations, n_channels })
    }

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for (key, value) in cards {
            header.set(key, value.to_string());
        }
        header
    }

    fn key(detector: &str, sporder: Option<i64>) -> SeriesKey {
        SeriesKey { detector: detector.to_string(), sporder }
    }

    #[test]
    fn a_tsomultispecmodel_file_is_a_rows_series_counted_from_the_first_key() {
        let found = probe(&[
            primary(&[("DATAMODL", "'TSOMultiSpecModel'"), ("DETECTOR", "'MULTIPLE'")]),
            rows_table(8, 5, &[("DETECTOR", "'NRS1'"), ("SPORDER", "-1")]),
            rows_table(4, 5, &[("DETECTOR", "'NRS1'"), ("SPORDER", "-1")]),
            rows_table(8, 7, &[("DETECTOR", "'NRS2'"), ("SPORDER", "-1")]),
        ]);
        assert_eq!(found, shape(TsoLayout::Rows, 2, 12, 5));
    }

    #[test]
    fn without_a_datamodl_card_the_columns_decide() {
        assert_eq!(probe(&[primary(&[]), rows_table(1, 3, &[])]), shape(TsoLayout::Rows, 1, 1, 3));
        let vectors_without_int_num = table(&[("WAVELENGTH", "3D", Some("um")), ("FLUX", "3D", Some("Jy"))], 2, &[]);
        assert_eq!(probe(&[primary(&[]), vectors_without_int_num]), None);
    }

    #[test]
    fn a_multispecmodel_file_is_a_series_only_when_every_extract1d_carries_int_num() {
        let legacy_primary = || primary(&[("DATAMODL", "'MultiSpecModel'"), ("DETECTOR", "'MIRIMAGE'")]);
        let found = probe(&[
            legacy_primary(),
            scalar_table(4, &[("INT_NUM", "1"), ("SPORDER", "1")]),
            scalar_table(4, &[("INT_NUM", "2"), ("SPORDER", "1")]),
            scalar_table(4, &[("INT_NUM", "3"), ("SPORDER", "1")]),
        ]);
        assert_eq!(found, shape(TsoLayout::Extensions, 1, 3, 4));
        let one_without_the_card = probe(&[
            legacy_primary(),
            scalar_table(4, &[("INT_NUM", "1"), ("SPORDER", "1")]),
            scalar_table(4, &[("SPORDER", "1")]),
            scalar_table(4, &[("INT_NUM", "3"), ("SPORDER", "1")]),
        ]);
        assert_eq!(one_without_the_card, None);
        assert_eq!(probe(&[legacy_primary(), scalar_table(4, &[("SPORDER", "1")])]), None);
    }

    #[test]
    fn other_data_models_and_files_without_an_extract1d_table_are_not_series() {
        assert_eq!(probe(&[primary(&[("DATAMODL", "'CubeModel'")]), rows_table(2, 3, &[])]), None);
        let other_tables = probe(&[
            primary(&[("DATAMODL", "'TSOMultiSpecModel'")]),
            table(&[("INTEGRATION_NUMBER", "1J", None), ("INT_MID_BJD_TDB", "1D", None)], 2, &[("EXTNAME", "'INT_TIMES'")]),
            table(&[("ASDF_METADATA", "4B", None)], 1, &[("EXTNAME", "'ASDF'")]),
        ]);
        assert_eq!(other_tables, None);
        let lower_case_name = probe(&[
            primary(&[("DATAMODL", "'TSOMultiSpecModel'")]),
            rows_table(2, 3, &[("EXTNAME", "'extract1d'")]),
        ]);
        assert_eq!(lower_case_name, shape(TsoLayout::Rows, 1, 2, 3));
    }

    #[test]
    fn the_series_key_takes_the_detector_of_the_extension_then_of_the_primary_and_keeps_sporder_signed() {
        let multiple = header(&[("DETECTOR", "'MULTIPLE'")]);
        assert_eq!(series_key(&header(&[("DETECTOR", "'NRS2'"), ("SPORDER", "-1")]), &multiple), key("NRS2", Some(-1)));
        assert_eq!(series_key(&header(&[("SPORDER", "0")]), &header(&[("DETECTOR", "'NIS     '")])), key("NIS", Some(0)));
        assert_eq!(series_key(&header(&[("SPORDER", "3")]), &header(&[])), key("UNKNOWN", Some(3)));
        assert_eq!(series_key(&header(&[]), &header(&[])), key("UNKNOWN", None));
    }

    fn real_file(relative: &str) -> File {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("tso").join(relative);
        File::open(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    fn golden_of(relative: &str) -> String {
        let name = relative.rsplit('/').next().unwrap().trim_end_matches("_x1dints.fits");
        format!("plan/goldens/tso/x1dints_{name}.json")
    }

    fn extract1d_keys(file: &File) -> (HduHeader, Vec<SeriesKey>, Vec<Option<String>>) {
        let primary = read_header_blocks(file, 0).unwrap().header;
        let tables = list_tables(file).unwrap();
        let extract1d: Vec<&HduHeader> = tables.iter().filter(|(t, _)| is_extract1d(t)).map(|(_, p)| &p.header).collect();
        let keys = extract1d.iter().map(|extension| series_key(extension, &primary)).collect();
        let detectors = extract1d.iter().map(|extension| card_text(extension, "DETECTOR")).collect();
        (primary, keys, detectors)
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/{A1,A3,A4,A5,A6,B1,C,E1}/*_x1dints.fits"]
    fn real_x1dints_files_are_identified_with_the_layout_and_counts_of_the_goldens() {
        let mut cases: Vec<(String, TsoLayout, usize, usize, usize, &str, i64)> = vec![
            ("A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits".into(), TsoLayout::Rows, 1, 4, 2040, "NRCALONG", 1),
            ("A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits".into(), TsoLayout::Rows, 3, 5, 2048, "NIS", 1),
            ("A4/jw04498-o013_t008_miri_p750l-slitlessprism_x1dints.fits".into(), TsoLayout::Rows, 1, 64, 388, "MIRIMAGE", 1),
            ("A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits".into(), TsoLayout::Rows, 2, 8, 1271, "NRS1", -1),
            ("A5/jw03375001001_03101_00001-seg001_nrs1_x1dints.fits".into(), TsoLayout::Rows, 1, 8, 1271, "NRS1", -1),
            ("A5/jw03375001001_03101_00001-seg001_nrs2_x1dints.fits".into(), TsoLayout::Rows, 1, 8, 2048, "NRS2", -1),
            ("A6/jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_x1dints.fits".into(), TsoLayout::Rows, 1, 65, 431, "NRS1", 0),
            ("B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits".into(), TsoLayout::Extensions, 1, 10, 388, "MIRIMAGE", 1),
            ("C/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits".into(), TsoLayout::Rows, 1, 44, 1299, "NRS1", -1),
            ("C/jw02327001001_03101_00001-seg006_nrs1_x1dints.fits".into(), TsoLayout::Rows, 1, 4, 1299, "NRS1", -1),
            ("E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits".into(), TsoLayout::Rows, 1, 240, 431, "NRS1", 0),
        ];
        for segment in 1..=5 {
            let name = format!("C/jw02327001001_03101_00001-seg00{segment}_nrs1_x1dints.fits");
            cases.push((name, TsoLayout::Rows, 1, 8, 1299, "NRS1", -1));
        }
        assert_eq!(cases.len(), 16);
        for (relative, layout, n_series, n_integrations, n_channels, detector, sporder) in &cases {
            let golden = golden_of(relative);
            let file = real_file(relative);
            let found = probe_x1dints(&file).unwrap();
            println!("{relative}: {found:?}");
            assert_eq!(found, shape(*layout, *n_series, *n_integrations, *n_channels), "{relative} against {golden}");
            let (_, keys, _) = extract1d_keys(&file);
            assert_eq!(keys[0], key(detector, Some(*sporder)), "{relative} against series_keys of {golden}");
        }

        let soss = "A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits";
        let (_, keys, detectors) = extract1d_keys(&real_file(soss));
        let golden = golden_of(soss);
        assert_eq!(keys, vec![key("NIS", Some(1)), key("NIS", Some(2)), key("NIS", Some(3))], "series_keys of {golden}");
        assert_eq!(detectors, vec![None, None, None], "DETECTOR cards of the EXTRACT1D headers, {golden}");

        let gratings = "A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits";
        let (primary, keys, _) = extract1d_keys(&real_file(gratings));
        let golden = golden_of(gratings);
        assert_eq!(keys, vec![key("NRS1", Some(-1)), key("NRS2", Some(-1))], "series_keys of {golden}");
        assert_eq!(card_text(&primary, "DETECTOR").as_deref(), Some("MULTIPLE"), "primary DETECTOR, {golden}");
    }

    fn f64s(column: &Column) -> &[f64] {
        match &column.values {
            ColumnValues::F64(values) => values,
            other => panic!("{}: expected F64 values, got {other:?}", column.name),
        }
    }

    fn i64s(column: &Column) -> &[i64] {
        match &column.values {
            ColumnValues::I64(values) => values,
            other => panic!("{}: expected I64 values, got {other:?}", column.name),
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits and tso/A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits"]
    fn real_e1_table_is_read_by_row_ranges_with_its_unsigned_columns() {
        let prism = "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits";
        let golden = golden_of(prism);
        let file = real_file(prism);
        let tables = list_tables(&file).unwrap();
        let (table, parsed) = tables.iter().find(|(t, _)| is_extract1d(t)).expect("an EXTRACT1D table");
        let mut reader = bintable_reader(&file, table.index, parsed).unwrap();
        println!("{prism}: {} rows of {} bytes, {} bytes of data", reader.n_rows(), reader.row_width(), reader.data_bytes());
        assert_eq!(reader.n_rows(), 240, "{golden}");
        assert_eq!(reader.row_width(), 60400, "{golden}");
        assert_eq!(reader.data_bytes(), 14_496_000, "{golden}");

        let names = [COL_N_ALONGDISP, COL_DQ, COL_INT_NUM, COL_WAVELENGTH, COL_MJD_AVG, COL_TDB_MID];
        let first = reader.read_rows(0..1, &names).unwrap();
        assert_eq!(i64s(&first[0]), [431], "N_ALONGDISP of row 0, {golden}");
        assert_eq!(i64s(&first[1]).len(), 431, "DQ of row 0, {golden}");
        assert_eq!(i64s(&first[1])[0], 1, "first DQ of row 0, {golden}");
        assert_eq!(i64s(&first[2]), [1], "INT_NUM of row 0, {golden}");
        assert_eq!(
            f64s(&first[3])[..3],
            [0.5530469695008425, 0.5564392656057195, 0.55989716986397],
            "first wavelengths of row 0 in um, {golden}"
        );
        assert_eq!(f64s(&first[4]), [60061.44133176829], "MJD-AVG of row 0 in days (UTC), {golden}");
        assert_eq!(f64s(&first[5]), [60061.4421733457], "TDB-MID of row 0 in days (BJD_TDB - 2400000.5), {golden}");

        let last = reader.read_rows(239..240, &names).unwrap();
        assert_eq!(i64s(&last[2]), [240], "INT_NUM of row 239, {golden}");
        assert_eq!(f64s(&last[5]), [60061.52706665472], "TDB-MID of row 239 in days, {golden}");
        assert_eq!(f64s(&last[3]).last().copied(), Some(5.370352185327354), "last wavelength of row 239 in um, {golden}");

        let grism = "A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits";
        let golden = golden_of(grism);
        let file = real_file(grism);
        let tables = list_tables(&file).unwrap();
        let (table, parsed) = tables.iter().find(|(t, _)| is_extract1d(t)).expect("an EXTRACT1D table");
        let mut reader = bintable_reader(&file, table.index, parsed).unwrap();
        assert_eq!(reader.row_width(), 285_660, "{golden}");
        assert_eq!(reader.n_rows(), 4, "{golden}");
        let n_alongdisp = reader.read_rows(0..1, &[COL_N_ALONGDISP]).unwrap();
        assert_eq!(i64s(&n_alongdisp[0]), [2040], "N_ALONGDISP of row 0 (stored 80 00 07 f8), {golden}");
    }

    const IDENTITY_CARDS: [(&str, &str); 14] = [
        ("NINTS", "2"),
        ("INTSTART", "1"),
        ("INTEND", "2"),
        ("EXSEGNUM", "1"),
        ("EXSEGTOT", "1"),
        ("EXP_TYPE", "'NRS_BRIGHTOBJ'"),
        ("INSTRUME", "'NIRSPEC'"),
        ("DETECTOR", "'NRS1'"),
        ("FILTER", "'CLEAR'"),
        ("GRATING", "'PRISM'"),
        ("SUBARRAY", "'SUB512'"),
        ("TARGNAME", "'T'"),
        ("CAL_VER", "'3.0.0'"),
        ("EXPSTART", "59999.99"),
    ];

    const WHITELIGHT_LOG_SAMPLE: &[u8] = br#"suffix: whtlt\n      search_output_file: True\n      input_dir:\n      level3\n      min_wavelength: 2.47\n      max_wavelength: 3.94", '2026-10-24T05:39:22.059"#;

    fn budget() -> ReadBudget {
        ReadBudget { cap: TSO_SET_BYTE_CAP, used: 0 }
    }

    fn read_series(path: &Path) -> X1dIntsFile {
        read_x1dints_file(path.to_str().unwrap(), &mut budget())
            .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()))
    }

    fn refusal(path: &Path) -> String {
        format!("{:#}", read_x1dints_file(path.to_str().unwrap(), &mut budget()).unwrap_err())
    }

    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|value| if value.is_nan() { f64::NAN.to_bits() } else { value.to_bits() }).collect()
    }

    fn sha256_of(path: &Path) -> String {
        Sha256::digest(std::fs::read(path).unwrap()).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn logged(min_um: Option<f64>, max_um: Option<f64>) -> Option<WhiteLightLog> {
        Some(WhiteLightLog { min_um, max_um })
    }

    fn nrs1_table(rows: Vec<SynthRow>) -> SynthTable {
        SynthTable { extver: 1, detector: Some("NRS1"), sporder: Some(-1), flux_unit: Some("Jy"), rows }
    }

    fn two_integrations() -> SynthTable {
        let mut second = ramp_row(2, 60000.5, 4);
        second.flux[1] = f64::NAN;
        second.dq[1] = 1;
        nrs1_table(vec![ramp_row(1, 60000.0, 4), second])
    }

    fn rows_primary(extra: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        series_primary(DATAMODEL_ROWS, &owned(extra))
    }

    fn write_two_integrations(dir: &Path, name: &str, extra: &[(&'static str, &str)], trailing: Vec<RawHdu>) -> PathBuf {
        let mut cards = IDENTITY_CARDS.to_vec();
        cards.extend_from_slice(extra);
        let mut hdus = vec![rows_hdu(&two_integrations())];
        hdus.extend(trailing);
        write_series_file(dir, name, rows_primary(&cards), hdus)
    }

    fn two_integrations_in_a_folder(
        name: &str,
        extra: &[(&'static str, &str)],
        trailing: Vec<RawHdu>,
    ) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = write_two_integrations(dir.path(), name, extra, trailing);
        (dir, path)
    }

    fn write_rows(dir: &Path, name: &str, rows: Vec<SynthRow>) -> PathBuf {
        write_series_file(dir, name, rows_primary(&[]), vec![rows_hdu(&nrs1_table(rows))])
    }

    #[test]
    fn a_rows_table_is_read_integration_by_integration_with_its_identity_cards() {
        let (_dir, path) = two_integrations_in_a_folder("two.fits", &[], Vec::new());
        let mut budget = budget();
        let found = read_x1dints_file(path.to_str().unwrap(), &mut budget).unwrap();
        assert_eq!(budget, ReadBudget { cap: TSO_SET_BYTE_CAP, used: 344 });

        assert_eq!(found.tables.len(), 1);
        let spectra = &found.tables[0];
        assert_eq!(spectra.hdu, 1);
        assert_eq!(spectra.extver, Some(1));
        assert_eq!(spectra.key, key("NRS1", Some(-1)));
        assert_eq!(spectra.n_channels, 4);
        assert_eq!(spectra.flux_unit.as_deref(), Some("Jy"));
        assert_eq!(spectra.time_origin, TimeOrigin::Column);
        assert_eq!(spectra.int_num, [1, 2]);
        assert_eq!(spectra.segment, [1, 1]);
        assert_eq!(spectra.n_alongdisp, [4, 4]);
        assert_eq!(spectra.mjd_utc_mid, [60000.0, 60000.5]);
        assert_eq!(spectra.bmjd_tdb_mid, [60000.002, 60000.502]);
        assert_eq!(spectra.wavelength_um, [1.0, 1.1, 1.2, 1.3, 1.0, 1.1, 1.2, 1.3]);
        assert_eq!(bits(&spectra.flux), bits(&[10.0, 11.0, 12.0, 13.0, 20.0, f64::NAN, 22.0, 23.0]));
        assert_eq!(spectra.dq, [0, 0, 0, 0, 0, 1, 0, 0]);
        assert_eq!(spectra.flux_error, [0.5; 8]);
        assert_eq!(spectra.npixels, [7.0; 8]);
        assert_eq!(spectra.rows_dropped_empty, 0);
        assert_eq!(found.notes, Vec::<String>::new());

        let expected = TsoFile {
            path: path.to_str().unwrap().to_string(),
            file_name: "two.fits".into(),
            sha256: sha256_of(&path),
            level: 2,
            datamodel: Some("TSOMultiSpecModel".into()),
            cal_ver: Some("3.0.0".into()),
            exp_type: Some("NRS_BRIGHTOBJ".into()),
            layout: TsoLayout::Rows,
            nints: Some(2),
            intstart: Some(1),
            intend: Some(2),
            exsegnum: Some(1),
            exsegtot: Some(1),
            instrument: Some("NIRSPEC".into()),
            detector: Some("NRS1".into()),
            filter: Some("CLEAR".into()),
            pupil: None,
            grating: Some("PRISM".into()),
            subarray: Some("SUB512".into()),
            target: Some("T".into()),
            expstart: Some(59999.99),
            outlier_detection: None,
            whitelight_log: None,
            whtlt_path: None,
            n_extract1d: 1,
            data_bytes: 344,
        };
        assert_eq!(found.file, expected);
        assert_eq!(found.file.sha256.len(), 64);
    }

    #[test]
    fn padding_beyond_n_alongdisp_is_normalised_and_empty_rows_are_dropped() {
        let mut padded = ramp_row(2, 60000.5, 4);
        padded.n_alongdisp = 2;
        for channel in 2..4 {
            padded.wavelength[channel] = 9.9;
            padded.flux[channel] = 99.0;
            padded.flux_error[channel] = 9.0;
            padded.dq[channel] = 0;
            padded.npixels[channel] = 5.0;
        }
        let mut empty = ramp_row(3, 60001.0, 4);
        empty.n_alongdisp = 0;
        let dir = tempfile::tempdir().unwrap();
        let found = read_series(&write_rows(dir.path(), "padded.fits", vec![ramp_row(1, 60000.0, 4), padded, empty]));

        let spectra = &found.tables[0];
        assert_eq!(spectra.int_num, [1, 2]);
        assert_eq!(spectra.n_alongdisp, [4, 2]);
        assert_eq!(spectra.mjd_utc_mid, [60000.0, 60000.5]);
        assert_eq!(spectra.wavelength_um[..6], [1.0, 1.1, 1.2, 1.3, 1.0, 1.1]);
        assert_eq!(spectra.flux[..6], [10.0, 11.0, 12.0, 13.0, 20.0, 21.0]);
        assert_eq!(spectra.flux_error[..6], [0.5; 6]);
        assert_eq!(spectra.npixels[..6], [7.0; 6]);
        assert_eq!(bits(&spectra.wavelength_um[6..]), bits(&[f64::NAN; 2]));
        assert_eq!(bits(&spectra.flux[6..]), bits(&[f64::NAN; 2]));
        assert_eq!(bits(&spectra.flux_error[6..]), bits(&[f64::NAN; 2]));
        assert_eq!(spectra.dq, [0, 0, 0, 0, 0, 0, DQ_DO_NOT_USE, DQ_DO_NOT_USE]);
        assert_eq!(spectra.npixels[6..], [0.0, 0.0]);
        assert_eq!(spectra.rows_dropped_empty, 1);
        assert_eq!(found.notes, ["HDU 1 (EXTRACT1D): 1 row(s) with N_ALONGDISP = 0 dropped"]);
    }

    #[test]
    fn several_extract1d_tables_keep_their_own_keys_units_and_lengths_and_other_hdus_are_ignored() {
        let spectra_of = |extver: i64, detector: Option<&'static str>, sporder: i64, flux_unit: &'static str, rows: i32, channels: usize| {
            let rows = (1..=rows).map(|number| ramp_row(number, 60000.0 + f64::from(number), channels)).collect();
            rows_hdu(&SynthTable { extver, detector, sporder: Some(sporder), flux_unit: Some(flux_unit), rows })
        };
        let hdus = vec![
            table(&[("INTEGRATION_NUMBER", "1J", None)], 2, &[("EXTNAME", "'INT_TIMES'")]),
            spectra_of(1, Some("NRS1"), -1, "Jy", 2, 4),
            spectra_of(2, Some("NRS2"), -1, "Jy", 2, 6),
            spectra_of(3, None, 2, "MJy", 1, 3),
            table(&[("FILENAME", "8A", None)], 1, &[("EXTNAME", "'HDRTAB'")]),
        ];
        let dir = tempfile::tempdir().unwrap();
        let found = read_series(&write_series_file(dir.path(), "multi.fits", rows_primary(&[("DETECTOR", "'MULTIPLE'")]), hdus));

        let summary: Vec<_> = found
            .tables
            .iter()
            .map(|spectra| (spectra.hdu, spectra.extver, spectra.key.clone(), spectra.n_channels, spectra.flux_unit.as_deref(), spectra.int_num.len()))
            .collect();
        assert_eq!(
            summary,
            [
                (2, Some(1), key("NRS1", Some(-1)), 4, Some("Jy"), 2),
                (3, Some(2), key("NRS2", Some(-1)), 6, Some("Jy"), 2),
                (4, Some(3), key("MULTIPLE", Some(2)), 3, Some("MJy"), 1),
            ]
        );
        assert_eq!(found.tables[1].flux, [10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 20.0, 21.0, 22.0, 23.0, 24.0, 25.0]);
        assert_eq!(found.tables[2].wavelength_um, [1.0, 1.1, 1.2]);
        assert_eq!(found.file.n_extract1d, 3);
        assert_eq!(found.file.data_bytes, 968);
        assert_eq!(found.file.detector.as_deref(), Some("MULTIPLE"));
    }

    fn legacy_row(integration: i32, channels: i32) -> SynthRow {
        SynthRow {
            wavelength: (0..channels).map(|channel| f64::from(14 - channel)).collect(),
            flux: (0..channels).map(|channel| f64::from(integration + channel)).collect(),
            ..ramp_row(integration, 0.0, channels as usize)
        }
    }

    fn write_legacy(dir: &Path, name: &str, tables: &[(i32, i32, &[(&'static str, &str)])]) -> PathBuf {
        let primary = series_primary(DATAMODEL_EXTENSIONS, &owned(&[("DETECTOR", "'MIRIMAGE'"), ("EXSEGNUM", "2")]));
        let hdus = tables
            .iter()
            .map(|(integration, channels, cards)| {
                let mut all = vec![("INT_NUM", integration.to_string()), ("SPORDER", "1".to_string())];
                all.extend(owned(cards));
                extension_hdu(i64::from(*integration), &all, &legacy_row(*integration, *channels))
            })
            .collect();
        write_series_file(dir, name, primary, hdus)
    }

    #[test]
    fn a_per_integration_file_takes_identity_and_times_from_the_cards_of_each_table() {
        let dir = tempfile::tempdir().unwrap();
        let timed = write_legacy(
            dir.path(),
            "timed.fits",
            &[
                (1, 5, &[("MJD-AVG", "60000.1"), ("TDB-MID", "60000.102")]),
                (2, 5, &[("MJD-AVG", "60000.2"), ("TDB-MID", "60000.202")]),
                (3, 5, &[("MJD-AVG", "60000.3"), ("TDB-MID", "60000.302")]),
            ],
        );
        let found = read_series(&timed);
        assert_eq!(found.file.layout, TsoLayout::Extensions);
        assert_eq!(found.file.datamodel.as_deref(), Some("MultiSpecModel"));
        assert_eq!(found.file.n_extract1d, 3);
        assert_eq!(found.file.data_bytes, 540);
        assert_eq!(found.notes, Vec::<String>::new());
        assert_eq!(found.tables.len(), 3);
        let times = [(60000.1, 60000.102), (60000.2, 60000.202), (60000.3, 60000.302)];
        for (spectra, (integration, (mjd, tdb))) in found.tables.iter().zip((1i64..).zip(times)) {
            let first = integration as f64;
            assert_eq!(spectra.hdu as i64, integration);
            assert_eq!(spectra.extver, Some(integration));
            assert_eq!(spectra.int_num, [integration]);
            assert_eq!(spectra.segment, [2]);
            assert_eq!(spectra.n_alongdisp, [5]);
            assert_eq!(spectra.mjd_utc_mid, [mjd]);
            assert_eq!(spectra.bmjd_tdb_mid, [tdb]);
            assert_eq!(spectra.time_origin, TimeOrigin::Keyword);
            assert_eq!(spectra.key, key("MIRIMAGE", Some(1)));
            assert_eq!(spectra.n_channels, 5);
            assert_eq!(spectra.wavelength_um, [14.0, 13.0, 12.0, 11.0, 10.0]);
            assert_eq!(spectra.flux, [first, first + 1.0, first + 2.0, first + 3.0, first + 4.0]);
            assert_eq!(spectra.flux_error, [0.5; 5]);
            assert_eq!(spectra.dq, [0; 5]);
            assert_eq!(spectra.npixels, [7.0; 5]);
            assert_eq!(spectra.flux_unit.as_deref(), Some("Jy"));
            assert_eq!(spectra.rows_dropped_empty, 0);
        }

        let untimed = read_series(&write_legacy(dir.path(), "untimed.fits", &[(1, 5, &[]), (2, 5, &[]), (3, 5, &[])]));
        assert_eq!(untimed.tables.len(), 3);
        for spectra in &untimed.tables {
            assert!(spectra.mjd_utc_mid[0].is_nan() && spectra.bmjd_tdb_mid[0].is_nan());
            assert_eq!(spectra.time_origin, TimeOrigin::Absent);
        }

        let with_an_empty_table = read_series(&write_legacy(dir.path(), "hollow.fits", &[(1, 5, &[]), (2, 0, &[]), (3, 5, &[])]));
        let kept: Vec<(usize, &[i64])> = with_an_empty_table.tables.iter().map(|spectra| (spectra.hdu, &spectra.int_num[..])).collect();
        assert_eq!(kept, [(1, &[1][..]), (3, &[3][..])]);
        assert_eq!(with_an_empty_table.notes, ["HDU 2 (EXTRACT1D): 1 row(s) with N_ALONGDISP = 0 dropped"]);
        assert_eq!(with_an_empty_table.file.n_extract1d, 3);
    }

    #[test]
    fn the_reader_refuses_what_it_cannot_trust_naming_the_file_the_hdu_and_the_reason() {
        let with_card = |(mut cards, data): RawHdu, card: &str, value: &str| -> RawHdu {
            set_card(&mut cards, card, value);
            (cards, data)
        };
        let one_row_of = |columns: &[(&str, &str, Option<&str>)]| table(columns, 1, &[]);
        let legacy_primary = || series_primary(DATAMODEL_EXTENSIONS, &[]);
        let mut beyond_the_row = ramp_row(1, 60000.0, 4);
        beyond_the_row.n_alongdisp = 5;

        let cases: Vec<(&str, Vec<(&'static str, String)>, Vec<RawHdu>, &str)> = vec![
            (
                "cube.fits",
                series_primary("CubeModel", &[]),
                vec![rows_hdu(&two_integrations())],
                "cube.fits: DATAMODL 'CubeModel' is not a time series of spectra (TSOMultiSpecModel or MultiSpecModel expected)",
            ),
            (
                "empty.fits",
                rows_primary(&[]),
                vec![table(&[("INTEGRATION_NUMBER", "1J", None)], 2, &[("EXTNAME", "'INT_TIMES'")])],
                "empty.fits: no EXTRACT1D binary table; tables in this file: [1] INT_TIMES (2 rows)",
            ),
            (
                "plain.fits",
                legacy_primary(),
                vec![extension_hdu(1, &[], &ramp_row(1, 60000.0, 3))],
                "plain.fits: its EXTRACT1D tables carry no INT_NUM card: this is a plain spectrum, not a time series",
            ),
            (
                "heap.fits",
                rows_primary(&[]),
                vec![with_card(rows_hdu(&two_integrations()), "PCOUNT", "8")],
                "heap.fits: HDU 1 (EXTRACT1D): PCOUNT = 8, variable-length data is not read",
            ),
            (
                "flux_e.fits",
                rows_primary(&[]),
                vec![one_row_of(&[("INT_NUM", "1J", None), ("WAVELENGTH", "4D", None), ("FLUX", "4E", None), ("N_ALONGDISP", "1J", None)])],
                "flux_e.fits: HDU 1 (EXTRACT1D): column FLUX has TFORM '4E', expected a D column",
            ),
            (
                "dq_d.fits",
                rows_primary(&[]),
                vec![with_card(rows_hdu(&two_integrations()), "TFORM5", "'4D'")],
                "dq_d.fits: HDU 1 (EXTRACT1D): column DQ has TFORM '4D', expected a J column",
            ),
            (
                "ragged.fits",
                rows_primary(&[]),
                vec![one_row_of(&[("INT_NUM", "1J", None), ("WAVELENGTH", "4D", None), ("FLUX", "3D", None), ("N_ALONGDISP", "1J", None)])],
                "ragged.fits: HDU 1 (EXTRACT1D): vector columns differ in length (WAVELENGTH 4, FLUX 3)",
            ),
            (
                "nocol.fits",
                rows_primary(&[]),
                vec![one_row_of(&[("WAVELENGTH", "4D", None), ("FLUX", "4D", None), ("N_ALONGDISP", "1J", None)])],
                "nocol.fits: HDU 1 (EXTRACT1D): column INT_NUM missing; columns: WAVELENGTH, FLUX, N_ALONGDISP",
            ),
            (
                "long.fits",
                rows_primary(&[]),
                vec![rows_hdu(&nrs1_table(vec![beyond_the_row]))],
                "long.fits: HDU 1 (EXTRACT1D): row 1 has N_ALONGDISP = 5 beyond the 4 elements of a row",
            ),
            (
                "unit.fits",
                rows_primary(&[]),
                vec![with_card(rows_hdu(&two_integrations()), "TUNIT2", "'furlong'")],
                "unit.fits: HDU 1 (EXTRACT1D): unknown wavelength unit 'furlong'",
            ),
            (
                "intstr.fits",
                legacy_primary(),
                vec![
                    extension_hdu(1, &owned(&[("INT_NUM", "1")]), &ramp_row(1, 60000.0, 3)),
                    extension_hdu(2, &owned(&[("INT_NUM", "'x'")]), &ramp_row(2, 60000.5, 3)),
                ],
                "intstr.fits: its EXTRACT1D tables carry no INT_NUM card: this is a plain spectrum, not a time series",
            ),
            (
                "nodm.fits",
                empty_primary_cards(),
                vec![table(&[("WAVELENGTH", "3D", None), ("FLUX", "3D", None)], 2, &[])],
                "nodm.fits: DATAMODL card missing and its EXTRACT1D tables are not one row per integration (columns INT_NUM, N_ALONGDISP and WAVELENGTH expected): not a time series",
            ),
            (
                "wide.fits",
                rows_primary(&[]),
                vec![one_row_of(&[("INT_NUM", "4J", None), ("WAVELENGTH", "4D", None), ("FLUX", "4D", None), ("N_ALONGDISP", "1J", None)])],
                "wide.fits: HDU 1 (EXTRACT1D): column INT_NUM has TFORM '4J', expected a scalar J column",
            ),
            (
                "legacy_e.fits",
                legacy_primary(),
                vec![table(&[("WAVELENGTH", "1D", None), ("FLUX", "1E", None)], 3, &[("INT_NUM", "1")])],
                "legacy_e.fits: HDU 1 (EXTRACT1D): column FLUX has TFORM '1E', expected a scalar D column",
            ),
        ];
        for (name, primary_cards, hdus, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            let message = refusal(&write_series_file(dir.path(), name, primary_cards, hdus));
            assert!(message.contains(expected), "{name}: {message}");
        }

        let dir = tempfile::tempdir().unwrap();
        let message = refusal(&dir.path().join("missing.fits"));
        assert!(message.contains("Failed to open") && message.contains("missing.fits"), "{message}");

        let not_fits = dir.path().join("text.fits");
        std::fs::write(&not_fits, b"wavelength,flux\n1.0,2.0\n").unwrap();
        let message = refusal(&not_fits);
        assert!(message.starts_with("text.fits: "), "{message}");
    }

    #[test]
    fn a_set_larger_than_the_budget_is_refused_before_any_table_data_is_read() {
        let (_dir, path) = two_integrations_in_a_folder("cut.fits", &[], Vec::new());
        let location = path.to_str().unwrap();
        let data_start = {
            let tables = list_tables(&File::open(&path).unwrap()).unwrap();
            let (_, parsed) = tables.iter().find(|(listed, _)| is_extract1d(listed)).expect("an EXTRACT1D table");
            parsed.data_start
        };
        OpenOptions::new().write(true).open(&path).unwrap().set_len(data_start as u64).unwrap();

        let mut nearly_full = ReadBudget { cap: 600, used: 300 };
        let over = format!("{:#}", read_x1dints_file(location, &mut nearly_full).unwrap_err());
        assert_eq!(
            over,
            "cut.fits: the EXTRACT1D tables of the files given hold 644 bytes, more than the 600 bytes this reader loads at once; open fewer files"
        );
        assert_eq!(nearly_full, ReadBudget { cap: 600, used: 300 });

        let truncated = refusal(&path);
        assert!(truncated.starts_with("cut.fits: HDU 1 (EXTRACT1D): header declares 344 bytes of table data"), "{truncated}");
        assert!(truncated.contains("only 0 bytes follow the header"), "{truncated}");
    }

    #[test]
    fn chunked_reading_gives_the_same_table_as_one_read() {
        let dir = tempfile::tempdir().unwrap();
        let rows = (1..=7).map(|number| ramp_row(number, 60000.0 + 0.5 * f64::from(number), 5)).collect();
        let path = write_rows(dir.path(), "seven.fits", rows);
        let location = path.to_str().unwrap();
        let at_once = read_x1dints_file_with(location, &mut budget(), READ_CHUNK_BYTES).unwrap();
        let whole = &at_once.tables[0];
        assert_eq!(whole.int_num, [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(whole.flux[30..], [70.0, 71.0, 72.0, 73.0, 74.0]);
        for chunk_bytes in [1, 417] {
            let in_chunks = read_x1dints_file_with(location, &mut budget(), chunk_bytes).unwrap();
            let pieces = &in_chunks.tables[0];
            assert_eq!(pieces.int_num, whole.int_num, "chunks of {chunk_bytes} bytes");
            assert_eq!(pieces.flux, whole.flux, "chunks of {chunk_bytes} bytes");
            assert_eq!(pieces.wavelength_um, whole.wavelength_um, "chunks of {chunk_bytes} bytes");
            assert_eq!(pieces.dq, whole.dq, "chunks of {chunk_bytes} bytes");
            assert_eq!(pieces.npixels, whole.npixels, "chunks of {chunk_bytes} bytes");
            assert_eq!(pieces.bmjd_tdb_mid, whole.bmjd_tdb_mid, "chunks of {chunk_bytes} bytes");
        }
    }

    #[test]
    fn whitelight_log_limits_reads_numbers_none_and_nothing() {
        assert_eq!(whitelight_log_limits(WHITELIGHT_LOG_SAMPLE), logged(Some(2.47), Some(3.94)));
        let miri = br#"suffix: whtlt\n      min_wavelength: 5.0\n      max_wavelength: 12.0", '2026"#;
        assert_eq!(whitelight_log_limits(miri), logged(Some(5.0), Some(12.0)));
        let not_set = br#"suffix: whtlt\n      min_wavelength: None\n      max_wavelength: None", '2026"#;
        assert_eq!(whitelight_log_limits(not_set), logged(None, None));
        let without_max = br#"level3\n      min_wavelength: 2.47\n      suffix: whtlt"#;
        assert_eq!(whitelight_log_limits(without_max), None);
        let not_a_number = br#"level3\n      min_wavelength: wide\n      max_wavelength: 3.94""#;
        assert_eq!(whitelight_log_limits(not_a_number), None);
        assert_eq!(whitelight_log_limits(b""), None);
        assert_eq!(whitelight_log_limits(&[0xff, 0xfe]), None);
    }

    #[test]
    fn the_level_the_logged_limits_and_the_sibling_whtlt_come_from_the_file_and_never_fail_the_load() {
        let (_dir, after_outliers) =
            two_integrations_in_a_folder("after_outlier_detection.fits", &[("S_OUTLIR", "'COMPLETE'")], Vec::new());
        let found = read_series(&after_outliers);
        assert_eq!(found.file.level, 3);
        assert_eq!(found.file.outlier_detection.as_deref(), Some("COMPLETE"));
        assert_eq!(found.file.whtlt_path, None);

        let tso3 = [("ASNTABLE", "'jw02327-o001_20260925t123033_tso3_00001_asn.json'")];
        let (_dir, level3_association) = two_integrations_in_a_folder("tso3.fits", &tso3, Vec::new());
        let found = read_series(&level3_association);
        assert_eq!(found.file.level, 3);
        assert_eq!(found.file.outlier_detection, None);
        let spec2 = [("ASNTABLE", "'jw02327-o001_20260925t123033_tso-spec2_00005_asn.json'")];
        let (_dir, level2_association) = two_integrations_in_a_folder("spec2.fits", &spec2, Vec::new());
        assert_eq!(read_series(&level2_association).file.level, 2);

        let mut cell = b"#ASDF 1.0.0\n".to_vec();
        cell.extend_from_slice(WHITELIGHT_LOG_SAMPLE);
        let (_dir, with_log) = two_integrations_in_a_folder("logged.fits", &[], vec![asdf_bintable_hdu(&cell)]);
        assert_eq!(read_series(&with_log).file.whitelight_log, logged(Some(2.47), Some(3.94)));
        let (_dir, without_magic) =
            two_integrations_in_a_folder("nomagic.fits", &[], vec![asdf_bintable_hdu(WHITELIGHT_LOG_SAMPLE)]);
        let found = read_series(&without_magic);
        assert_eq!(found.file.whitelight_log, None);
        assert_eq!(found.tables.len(), 1);
        assert_eq!(found.tables[0].int_num, [1, 2]);
        assert_eq!(bits(&found.tables[0].flux), bits(&[10.0, 11.0, 12.0, 13.0, 20.0, f64::NAN, 22.0, 23.0]));

        let (dir, level3) = two_integrations_in_a_folder("obs_x1dints.fits", &[("S_OUTLIR", "'COMPLETE'")], Vec::new());
        assert_eq!(read_series(&level3).file.whtlt_path, None);
        std::fs::write(dir.path().join("obs_whtlt.ecsv"), b"").unwrap();
        let sibling = read_series(&level3).file.whtlt_path.expect("the white-light file beside a level 3 product");
        assert!(sibling.ends_with("obs_whtlt.ecsv"), "{sibling}");
        let (dir, level2) = two_integrations_in_a_folder("seg_x1dints.fits", &[], Vec::new());
        std::fs::write(dir.path().join("seg_whtlt.ecsv"), b"").unwrap();
        assert_eq!(read_series(&level2).file.whtlt_path, None);
    }

    #[test]
    fn a_gzip_compressed_series_file_reads_like_the_plain_one() {
        let (dir, plain) = two_integrations_in_a_folder("t_x1dints.fits", &[("S_OUTLIR", "'COMPLETE'")], Vec::new());
        std::fs::write(dir.path().join("t_whtlt.ecsv"), b"").unwrap();
        let packed = gzip_copy(&plain, "t_x1dints.fits.gz");
        let (from_plain, from_gzip) = (read_series(&plain), read_series(&packed));
        let (wanted, spectra) = (&from_plain.tables[0], &from_gzip.tables[0]);
        assert_eq!(spectra.int_num, wanted.int_num);
        assert_eq!(spectra.int_num, [1, 2]);
        assert_eq!(bits(&spectra.flux), bits(&wanted.flux));
        assert_eq!(spectra.wavelength_um, wanted.wavelength_um);
        assert_eq!(spectra.dq, wanted.dq);
        assert_eq!(spectra.npixels, wanted.npixels);
        assert_eq!(spectra.bmjd_tdb_mid, wanted.bmjd_tdb_mid);
        assert_eq!(from_gzip.file.file_name, "t_x1dints.fits.gz");
        assert_eq!(from_gzip.file.path, packed.to_str().unwrap());
        assert_ne!(from_gzip.file.sha256, from_plain.file.sha256);
        assert_eq!(from_gzip.file.sha256, sha256_of(&packed));
        assert_eq!(from_plain.file.sha256, sha256_of(&plain));
        let sibling = from_gzip.file.whtlt_path.expect("the white-light file beside a compressed level 3 product");
        assert!(sibling.ends_with("t_whtlt.ecsv"), "{sibling}");
    }

    #[test]
    fn unsigned_columns_keep_values_above_the_signed_range() {
        let mut row = ramp_row(1, 60000.0, 4);
        row.dq[0] = 3_000_000_000;
        let dir = tempfile::tempdir().unwrap();
        let found = read_series(&write_rows(dir.path(), "unsigned.fits", vec![row]));
        assert_eq!(found.tables[0].dq, [3_000_000_000, 0, 0, 0]);
        assert_eq!(found.tables[0].n_alongdisp, [4]);
    }

    fn bare_rows_hdu(wavelength_unit: Option<&str>, wavelengths: [f64; 3]) -> RawHdu {
        let cards = bintable_cards(
            &[("INT_NUM", "1J", None), ("WAVELENGTH", "3D", wavelength_unit), ("FLUX", "3D", None), ("N_ALONGDISP", "1J", None)],
            2,
            &owned(&[("TZERO4", "2147483648")]),
        );
        let mut data = Vec::new();
        for integration in 1i32..=2 {
            data.extend(integration.to_be_bytes());
            data.extend(wavelengths.iter().flat_map(|value| value.to_be_bytes()));
            data.extend((0..3).flat_map(|channel| f64::from(10 * integration + channel).to_be_bytes()));
            data.extend(((3 - UNSIGNED_TZERO) as i32).to_be_bytes());
        }
        (cards, data)
    }

    #[test]
    fn missing_optional_columns_are_filled_and_named_in_the_notes() {
        let dir = tempfile::tempdir().unwrap();
        let bare = write_series_file(dir.path(), "bare.fits", rows_primary(&[]), vec![bare_rows_hdu(Some("um"), [1.0, 1.1, 1.2])]);
        let found = read_series(&bare);
        let spectra = &found.tables[0];
        assert_eq!(spectra.int_num, [1, 2]);
        assert_eq!(spectra.n_alongdisp, [3, 3]);
        assert_eq!(spectra.wavelength_um, [1.0, 1.1, 1.2, 1.0, 1.1, 1.2]);
        assert_eq!(spectra.flux, [10.0, 11.0, 12.0, 20.0, 21.0, 22.0]);
        assert_eq!(bits(&spectra.flux_error), bits(&[f64::NAN; 6]));
        assert_eq!(bits(&spectra.npixels), bits(&[f64::NAN; 6]));
        assert_eq!(spectra.dq, [0; 6]);
        assert_eq!(spectra.segment, [1, 1]);
        assert_eq!(bits(&spectra.mjd_utc_mid), bits(&[f64::NAN; 2]));
        assert_eq!(bits(&spectra.bmjd_tdb_mid), bits(&[f64::NAN; 2]));
        assert_eq!(spectra.time_origin, TimeOrigin::Absent);
        assert_eq!(spectra.flux_unit, None);
        assert_eq!(found.notes.len(), 6, "{:?}", found.notes);
        for column in ["FLUX_ERROR", "DQ", "NPIXELS", "SEGMENT", "MJD-AVG", "TDB-MID"] {
            let note = format!("HDU 1 (EXTRACT1D): column {column} missing");
            assert!(found.notes.contains(&note), "{note} in {:?}", found.notes);
        }

        let unlabelled =
            write_series_file(dir.path(), "unlabelled.fits", empty_primary_cards(), vec![bare_rows_hdu(None, [1.0, 1.1, 1.2])]);
        let found = read_series(&unlabelled);
        assert_eq!(found.file.datamodel, None);
        assert_eq!(found.file.layout, TsoLayout::Rows);
        assert_eq!(found.tables[0].wavelength_um, [1.0, 1.1, 1.2, 1.0, 1.1, 1.2]);
        assert_eq!(found.notes[..2], [
            "DATAMODL card missing: layout taken from the table columns",
            "HDU 1 (EXTRACT1D): TUNIT of WAVELENGTH missing: assumed um",
        ]);

        let nanometres = write_series_file(
            dir.path(),
            "nanometres.fits",
            rows_primary(&[]),
            vec![bare_rows_hdu(Some("nm"), [1000.0, 1100.0, 1200.0])],
        );
        let found = read_series(&nanometres);
        for (converted, expected) in found.tables[0].wavelength_um.iter().zip([1.0, 1.1, 1.2, 1.0, 1.1, 1.2]) {
            assert!((converted - expected).abs() < 1e-12, "{converted} um against {expected} um");
        }
    }

    const REAL_A1: &str = "A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits";
    const REAL_A3: &str = "A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits";
    const REAL_B1: &str = "B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits";
    const REAL_C1: &str = "C/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits";
    const REAL_E1: &str = "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits";

    const G395M_SEGMENT_TIMES: [([f64; 2], [f64; 2]); 6] = [
        ([60280.05657544178, 60280.12944272495], [60280.05386059504, 60280.126721581415]),
        ([60280.13985233657, 60280.21271961612], [60280.13713029375, 60280.20999128011]),
        ([60280.223129227226, 60280.29599650314], [60280.22039999245, 60280.29326097882]),
        ([60280.30640611373, 60280.379273385995], [60280.30366969116, 60280.37653067752]),
        ([60280.389682996065, 60280.46255026468], [60280.38693938986, 60280.45980037622]),
        ([60280.47295987422, 60280.50418870245], [60280.47020908856, 60280.501435225575]),
    ];

    struct RealTable {
        hdu: usize,
        detector: &'static str,
        sporder: i64,
        rows: usize,
        channels: usize,
        first_int_num: i64,
        segment: i64,
        tdb: [f64; 2],
        mjd: [f64; 2],
        first_wavelengths: [f64; 3],
        last_wavelengths: [f64; 3],
        nan_flux: usize,
        flagged: usize,
        nan_flux_unflagged: usize,
    }

    struct RealFile {
        relative: String,
        level: u8,
        nints: i64,
        flux_unit: &'static str,
        logged: Option<WhiteLightLog>,
        tables: Vec<RealTable>,
    }

    fn g395m_segment_file(segment: usize) -> String {
        format!("C/jw02327001001_03101_00001-seg00{segment}_nrs1_x1dints.fits")
    }

    fn g395m_segment(segment: usize, hdu: usize) -> RealTable {
        let (tdb, mjd) = G395M_SEGMENT_TIMES[segment - 1];
        let rows = if segment == 6 { 4 } else { 8 };
        RealTable {
            hdu,
            detector: "NRS1",
            sporder: -1,
            rows,
            channels: 1299,
            first_int_num: 8 * (segment as i64 - 1) + 1,
            segment: segment as i64,
            tdb,
            mjd,
            first_wavelengths: [2.848429652675818, 2.8502291545571627, 2.852028648687558],
            last_wavelengths: [5.17846349118785, 5.180257949448807, 5.182052338975267],
            nan_flux: 23 * rows,
            flagged: 23 * rows,
            nan_flux_unflagged: 0,
        }
    }

    fn g395h_nrs1(hdu: usize) -> RealTable {
        RealTable {
            hdu,
            detector: "NRS1",
            sporder: -1,
            rows: 8,
            channels: 1271,
            first_int_num: 1,
            segment: 1,
            tdb: [60496.522504382556, 60496.55911503738],
            mjd: [60496.52120635712, 60496.557820564216],
            first_wavelengths: [2.8617736352602416, 2.862452740024391, 2.863131742101342],
            last_wavelengths: [3.7191066310685548, 3.7197792427927103, 3.7204518540407396],
            nan_flux: 256,
            flagged: 256,
            nan_flux_unflagged: 0,
        }
    }

    fn g395h_nrs2(hdu: usize) -> RealTable {
        RealTable {
            hdu,
            detector: "NRS2",
            sporder: -1,
            rows: 8,
            channels: 2048,
            first_int_num: 1,
            segment: 1,
            tdb: [60496.52240268599, 60496.55901266559],
            mjd: [60496.52110465069, 60496.55771818249],
            first_wavelengths: [3.8194005627692933, 3.8200721495595147, 3.8207437699318807],
            last_wavelengths: [5.175868480698995, 5.176521506685293, 5.177174531991892],
            nan_flux: 72,
            flagged: 72,
            nan_flux_unflagged: 0,
        }
    }

    fn soss_order(hdu: usize, sporder: i64, channels: usize, first_wavelengths: [f64; 3], last_wavelengths: [f64; 3], nan_flux: usize) -> RealTable {
        RealTable {
            hdu,
            detector: "NIS",
            sporder,
            rows: 5,
            channels,
            first_int_num: 1,
            segment: 1,
            tdb: [61043.21592608525, 61043.21592608525],
            mjd: [61043.21309070602, f64::NAN],
            first_wavelengths,
            last_wavelengths,
            nan_flux,
            flagged: 0,
            nan_flux_unflagged: nan_flux,
        }
    }

    fn real_rows_files() -> Vec<RealFile> {
        let mut files = vec![
            RealFile {
                relative: REAL_A1.into(),
                level: 3,
                nints: 4,
                flux_unit: "DN/s",
                logged: logged(Some(2.47), Some(3.94)),
                tables: vec![RealTable {
                    hdu: 2,
                    detector: "NRCALONG",
                    sporder: 1,
                    rows: 4,
                    channels: 2040,
                    first_int_num: 1,
                    segment: 1,
                    tdb: [59829.99303618102, 59829.99303618102],
                    mjd: [59829.989846412034, f64::NAN],
                    first_wavelengths: [2.40455961227417, 2.4055089950561523, 2.4064581394195557],
                    last_wavelengths: [4.385996341705322, 4.386977195739746, 4.387957572937012],
                    nan_flux: 0,
                    flagged: 0,
                    nan_flux_unflagged: 0,
                }],
            },
            RealFile {
                relative: REAL_A3.into(),
                level: 3,
                nints: 5,
                flux_unit: "MJy",
                logged: logged(None, None),
                tables: vec![
                    soss_order(
                        2,
                        1,
                        2048,
                        [2.8329402405819173, 2.8319530344473756, 2.8309658283323555],
                        [0.8491389518812792, 0.8482612176785985, 0.8473834834322623],
                        50,
                    ),
                    soss_order(
                        3,
                        2,
                        1783,
                        [1.4138421961399272, 1.4133842641364875, 1.412926333659344],
                        [0.5886635230187085, 0.5882546711519454, 0.5878459442296309],
                        25,
                    ),
                    soss_order(
                        4,
                        3,
                        1134,
                        [0.9556302324967343, 0.955309643600813, 0.9549890562384763],
                        [0.633060454539978, 0.6328535742311957, 0.632646909646051],
                        25,
                    ),
                ],
            },
            RealFile {
                relative: "A4/jw04498-o013_t008_miri_p750l-slitlessprism_x1dints.fits".into(),
                level: 3,
                nints: 64,
                flux_unit: "Jy",
                logged: logged(Some(5.0), Some(12.0)),
                tables: vec![RealTable {
                    hdu: 2,
                    detector: "MIRIMAGE",
                    sporder: 1,
                    rows: 64,
                    channels: 388,
                    first_int_num: 1,
                    segment: 1,
                    tdb: [60463.278435296284, 60463.2791309261],
                    mjd: [60463.27446656057, 60463.27516218557],
                    first_wavelengths: [14.048406600952148, 14.034751892089844, 14.02108097076416],
                    last_wavelengths: [4.643184661865234, 4.44938325881958, 4.063209533691406],
                    nan_flux: 321,
                    flagged: 321,
                    nan_flux_unflagged: 0,
                }],
            },
            RealFile {
                relative: "A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits".into(),
                level: 3,
                nints: 8,
                flux_unit: "Jy",
                logged: logged(None, None),
                tables: vec![g395h_nrs1(2), g395h_nrs2(3)],
            },
            RealFile {
                relative: "A5/jw03375001001_03101_00001-seg001_nrs1_x1dints.fits".into(),
                level: 2,
                nints: 8,
                flux_unit: "Jy",
                logged: None,
                tables: vec![g395h_nrs1(2)],
            },
            RealFile {
                relative: "A5/jw03375001001_03101_00001-seg001_nrs2_x1dints.fits".into(),
                level: 2,
                nints: 8,
                flux_unit: "Jy",
                logged: None,
                tables: vec![g395h_nrs2(2)],
            },
            RealFile {
                relative: "A6/jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_x1dints.fits".into(),
                level: 3,
                nints: 65,
                flux_unit: "Jy",
                logged: logged(None, None),
                tables: vec![RealTable {
                    hdu: 2,
                    detector: "NRS1",
                    sporder: 0,
                    rows: 65,
                    channels: 431,
                    first_int_num: 1,
                    segment: 1,
                    tdb: [60796.95842671788, 60797.05911576449],
                    mjd: [60796.95485544182, 60797.055553154765],
                    first_wavelengths: [0.5524153961529221, 0.5557963153106529, 0.559246162366847],
                    last_wavelengths: [5.354063970613874, 5.361335788635605, 5.368596776621393],
                    nan_flux: 1590,
                    flagged: 1590,
                    nan_flux_unflagged: 0,
                }],
            },
            RealFile {
                relative: REAL_C1.into(),
                level: 3,
                nints: 44,
                flux_unit: "Jy",
                logged: logged(None, None),
                tables: (1..=6).map(|segment| g395m_segment(segment, segment + 1)).collect(),
            },
            RealFile {
                relative: REAL_E1.into(),
                level: 3,
                nints: 240,
                flux_unit: "Jy",
                logged: logged(None, None),
                tables: vec![RealTable {
                    hdu: 2,
                    detector: "NRS1",
                    sporder: 0,
                    rows: 240,
                    channels: 431,
                    first_int_num: 1,
                    segment: 1,
                    tdb: [60061.4421733457, 60061.52706665472],
                    mjd: [60061.44133176829, 60061.526222939145],
                    first_wavelengths: [0.5530469695008425, 0.5564392656057195, 0.55989716986397],
                    last_wavelengths: [5.35582173389611, 5.363092489601262, 5.370352185327354],
                    nan_flux: 5918,
                    flagged: 5918,
                    nan_flux_unflagged: 0,
                }],
            },
        ];
        files.extend((1..=6).map(|segment| RealFile {
            relative: g395m_segment_file(segment),
            level: 2,
            nints: 44,
            flux_unit: "Jy",
            logged: None,
            tables: vec![g395m_segment(segment, 2)],
        }));
        files
    }

    fn real_path(relative: &str) -> String {
        env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("tso").join(relative).to_string_lossy().into_owned()
    }

    fn read_real(relative: &str) -> X1dIntsFile {
        read_x1dints_file(&real_path(relative), &mut budget()).unwrap_or_else(|error| panic!("{relative}: {error:#}"))
    }

    fn ends(values: &[f64]) -> [f64; 2] {
        [values[0], values[values.len() - 1]]
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/{A1,A3,A4,A5,A6,C,E1}/*_x1dints.fits"]
    fn real_current_layout_files_read_with_the_tables_units_times_and_masks_of_the_goldens() {
        let files = real_rows_files();
        assert_eq!(files.len(), 15);
        for expected in &files {
            let relative = expected.relative.as_str();
            let golden = golden_of(relative);
            let found = read_real(relative);
            println!(
                "{relative}: level {}, {} table(s), logged limits {:?}",
                found.file.level,
                found.tables.len(),
                found.file.whitelight_log
            );
            assert_eq!(found.file.level, expected.level, "{relative}: product_level of {golden}");
            assert_eq!(found.file.layout, TsoLayout::Rows, "{relative}: layout of {golden}");
            assert_eq!(found.file.nints, Some(expected.nints), "{relative}: primary.NINTS of {golden}");
            assert_eq!(
                found.file.whitelight_log, expected.logged,
                "{relative}: series[0].white_light_as_archived.limits_source of {golden}"
            );
            assert_eq!(found.tables.len(), expected.tables.len(), "{relative}: n_extract1d of {golden}");
            for (spectra, wanted) in found.tables.iter().zip(&expected.tables) {
                let at = format!("{relative} HDU {}", wanted.hdu);
                let table_golden =
                    if relative == REAL_C1 { golden_of(&g395m_segment_file(wanted.hdu - 1)) } else { golden.clone() };
                assert_eq!(spectra.hdu, wanted.hdu, "{at}: series[].extract1d_hdus of {golden}");
                assert_eq!(spectra.key, key(wanted.detector, Some(wanted.sporder)), "{at}: series_keys of {golden}");
                assert_eq!(spectra.n_channels, wanted.channels, "{at}: series[].tform_repeat of {golden}");
                assert_eq!(spectra.flux_unit.as_deref(), Some(expected.flux_unit), "{at}: series[].units.FLUX of {golden}");
                assert_eq!(spectra.time_origin, TimeOrigin::Column, "{at}: series[].time.source of {golden}");
                let numbers: Vec<i64> = (wanted.first_int_num..).take(wanted.rows).collect();
                assert_eq!(spectra.int_num, numbers, "{at}: series[].rows_per_extract1d and int_num of {golden}");
                assert_eq!(spectra.segment, vec![wanted.segment; wanted.rows], "{at}: series[].segment_values of {golden}");
                assert_eq!(
                    spectra.n_alongdisp,
                    vec![wanted.channels as u32; wanted.rows],
                    "{at}: series[].n_alongdisp of {golden}"
                );
                assert_eq!(spectra.rows_dropped_empty, 0, "{at}: series[].n_alongdisp.rows_zero of {golden}");
                assert_eq!(
                    bits(&ends(&spectra.bmjd_tdb_mid)),
                    bits(&wanted.tdb),
                    "{at}: series[0].time.tdb_mid_first and tdb_mid_last of {table_golden}"
                );
                assert_eq!(
                    bits(&ends(&spectra.mjd_utc_mid)),
                    bits(&wanted.mjd),
                    "{at}: series[0].time.mjd_avg_first and mjd_avg_last of {table_golden}"
                );
                assert_eq!(
                    bits(&spectra.wavelength_um[..3]),
                    bits(&wanted.first_wavelengths),
                    "{at}: series[].first_valid_row.wavelength_first3 of {golden}"
                );
                assert_eq!(
                    bits(&spectra.wavelength_um[wanted.channels - 3..wanted.channels]),
                    bits(&wanted.last_wavelengths),
                    "{at}: series[].first_valid_row.wavelength_last3 of {golden}"
                );
                let nan_flux = spectra.flux.iter().filter(|flux| flux.is_nan()).count();
                let flagged = spectra.dq.iter().filter(|&&flags| flags != 0).count();
                let nan_flux_unflagged =
                    spectra.flux.iter().zip(&spectra.dq).filter(|(flux, &flags)| flux.is_nan() && flags == 0).count();
                println!(
                    "{at}: {} x {} {:?} in {:?}, NaN flux {nan_flux}, dq != 0 {flagged}, NaN flux with dq 0 {nan_flux_unflagged}",
                    spectra.int_num.len(),
                    spectra.n_channels,
                    spectra.key,
                    spectra.flux_unit
                );
                assert_eq!(nan_flux, wanted.nan_flux, "{at}: series[0].masking_counts.flux_nan of {table_golden}");
                assert_eq!(flagged, wanted.flagged, "{at}: series[0].masking_counts.dq_nonzero of {table_golden}");
                assert_eq!(
                    nan_flux_unflagged, wanted.nan_flux_unflagged,
                    "{at}: series[0].masking_counts.flux_nan_with_dq_zero of {table_golden}"
                );
            }
        }

        let grism = read_real(REAL_A1);
        let golden = golden_of(REAL_A1);
        assert_eq!(bits(&grism.tables[0].bmjd_tdb_mid), bits(&[59829.99303618102; 4]), "series[0].time of {golden}");
        assert_eq!(
            bits(&grism.tables[0].mjd_utc_mid),
            bits(&[59829.989846412034, f64::NAN, f64::NAN, f64::NAN]),
            "series[0].time.mjd_avg_nan_rows of {golden}"
        );
        let soss = read_real(REAL_A3);
        let golden = golden_of(REAL_A3);
        for order in &soss.tables {
            assert_eq!(bits(&order.bmjd_tdb_mid), bits(&[61043.21592608525; 5]), "HDU {}: series[].time of {golden}", order.hdu);
            assert_eq!(
                bits(&order.mjd_utc_mid),
                bits(&[61043.21309070602, f64::NAN, f64::NAN, f64::NAN, f64::NAN]),
                "HDU {}: series[].time.mjd_avg_nan_rows of {golden}",
                order.hdu
            );
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits"]
    fn real_legacy_file_reads_ten_integrations_from_ten_extensions() {
        let golden = golden_of(REAL_B1);
        let found = read_real(REAL_B1);
        assert_eq!(found.file.layout, TsoLayout::Extensions, "layout of {golden}");
        assert_eq!(found.file.level, 3, "product_level of {golden}");
        assert_eq!(found.file.cal_ver.as_deref(), Some("1.15.1"), "primary.CAL_VER of {golden}");
        assert_eq!(found.file.whitelight_log, None, "series[0].white_light_as_archived.limits_source of {golden}");
        assert_eq!(found.tables.len(), 10, "n_extract1d of {golden}");
        for (spectra, integration) in found.tables.iter().zip(1i64..) {
            let at = format!("{REAL_B1} HDU {}", spectra.hdu);
            assert_eq!(spectra.hdu as i64, integration + 1, "{at}: series[0].extract1d_hdus of {golden}");
            assert_eq!(spectra.extver, Some(integration), "{at}: hdus[].extver of {golden}");
            assert_eq!(spectra.key, key("MIRIMAGE", Some(1)), "{at}: series_keys of {golden}");
            assert_eq!(spectra.int_num, [integration], "{at}: series[0].int_num of {golden}");
            assert_eq!(spectra.segment, [1], "{at}: primary.EXSEGNUM of {golden}");
            assert_eq!(spectra.n_alongdisp, [388], "{at}: series[0].n_alongdisp of {golden}");
            assert_eq!(spectra.n_channels, 388, "{at}: series[0].tform_repeat of {golden}");
            assert_eq!(spectra.time_origin, TimeOrigin::Keyword, "{at}: series[0].time.source of {golden}");
            assert_eq!(spectra.flux_unit.as_deref(), Some("Jy"), "{at}: series[0].units.FLUX of {golden}");
            assert_eq!(
                bits(&spectra.wavelength_um[..3]),
                bits(&[13.993632316589355, 13.980284690856934, 13.966923713684082]),
                "{at}: series[0].first_valid_row.wavelength_first3 of {golden}"
            );
            assert_eq!(
                bits(&spectra.wavelength_um[385..]),
                bits(&[4.390594005584717, 4.174907207489014, 3.7063305377960205]),
                "{at}: series[0].first_valid_row.wavelength_last3 of {golden}"
            );
            let nan_flux = spectra.flux.iter().filter(|flux| flux.is_nan()).count();
            let nan_flux_unflagged =
                spectra.flux.iter().zip(&spectra.dq).filter(|(flux, &flags)| flux.is_nan() && flags == 0).count();
            println!(
                "{at}: integration {:?}, TDB-MID {:?}, MJD-AVG {:?}, NaN flux {nan_flux}, NaN flux with dq 0 {nan_flux_unflagged}",
                spectra.int_num, spectra.bmjd_tdb_mid, spectra.mjd_utc_mid
            );
            assert_eq!(nan_flux, 11, "{at}: series[0].masking_counts.flux_nan of {golden} (110 over ten integrations)");
            assert_eq!(nan_flux_unflagged, 11, "{at}: series[0].masking_counts.flux_nan_with_dq_zero of {golden}");
        }
        let (first, last) = (&found.tables[0], &found.tables[9]);
        assert_eq!(
            bits(&[first.bmjd_tdb_mid[0], last.bmjd_tdb_mid[0]]),
            bits(&[60126.80095917577, 60126.80143914741]),
            "series[0].time.tdb_mid_first and tdb_mid_last of {golden}"
        );
        assert_eq!(
            bits(&[first.mjd_utc_mid[0], last.mjd_utc_mid[0]]),
            bits(&[60126.800615279506, 60126.80109527951]),
            "series[0].time.mjd_avg_first and mjd_avg_last of {golden}"
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/{E1,B1,C}/*-o0*_x1dints.fits"]
    fn real_inputs_are_identified_by_their_sha256_and_their_table_bytes() {
        let started = Instant::now();
        let prism = read_real(REAL_E1);
        println!("{REAL_E1}: read in {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
        let golden = golden_of(REAL_E1);
        assert_eq!(
            prism.file.sha256, "a5e7022d22e779fdf43b14fba6e7d23644c9547c920b6a52c13435c63706bfbc",
            "produced_by.inputs[0].sha256 of {golden}"
        );
        assert_eq!(prism.file.data_bytes, 14_496_000, "hdus[2].naxis1 x naxis2 of {golden}");
        assert_eq!(prism.file.n_extract1d, 1, "n_extract1d of {golden}");

        let legacy = read_real(REAL_B1);
        let golden = golden_of(REAL_B1);
        assert_eq!(
            legacy.file.sha256, "1954b9b953605d7d4b55baceccd0df39a06c5b8638431d4dc5935a98ca7bba05",
            "produced_by.inputs[0].sha256 of {golden}"
        );
        assert_eq!(legacy.file.data_bytes, 543_200, "hdus[2..=11].naxis1 x naxis2 of {golden}");
        assert_eq!(legacy.file.n_extract1d, 10, "n_extract1d of {golden}");

        let segments = read_real(REAL_C1);
        assert_eq!(segments.file.data_bytes, 8_004_480, "hdus[2..=7].naxis1 x naxis2 of {}", golden_of(REAL_C1));

        let mut small = ReadBudget { cap: 1_000_000, used: 0 };
        let over = format!("{:#}", read_x1dints_file(&real_path(REAL_E1), &mut small).unwrap_err());
        println!("{over}");
        assert!(
            over.contains("hold 14496000 bytes, more than the 1000000 bytes this reader loads at once; open fewer files"),
            "{over}"
        );
        assert_eq!(small, ReadBudget { cap: 1_000_000, used: 0 });
    }
}
