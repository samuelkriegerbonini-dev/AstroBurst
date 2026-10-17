use std::collections::BTreeMap;

use serde::Serialize;

use crate::core::analysis::x1d::{X1dSpectrum, COL_WAVELENGTH, EXTNAME_EXTRACT1D};
use crate::core::astrometry::spectral::{
    air_formula_applies, is_non_linear_spectral_ctype, is_spacecraft, spectral_axis_on,
    wavelength_unit_to_um, AxisKind, SpectralAxis,
};
use crate::core::imaging::dq_flags::DqTable;
use crate::math::exact_median_f64;
use crate::types::constants::{DEFAULT_WAVELENGTH_UNIT, HEADER_BUNIT};
use crate::types::header::HduHeader;

pub const MORTON_AIR_FORMULA: &str = "Morton (2000, ApJS 130, 403)";
pub const IRAF_WAT_CARD_WIDTH: usize = 68;
pub const CSV_WAVELENGTH_VACUUM_COLUMN: &str = "wavelength_vacuum_um";
pub const CSV_WAVELENGTH_AIR_COLUMN: &str = "wavelength_air_um";
pub const CSV_FLUX_COLUMN: &str = "flux";
pub const CSV_FLUX_UNIT_COLUMN: &str = "flux_unit";
pub const CSV_ERR_COLUMN: &str = "err";
pub const CSV_DQ_COLUMN: &str = "dq";
pub const CSV_DQ_TABLE_KEY: &str = "dq_table";

const CSV_FLUX_UNIT_KEY: &str = "flux_unit";
const CSV_NATIVE_FLUX_UNIT: &str = "native";
const CSV_PRECISION_KEY: &str = "precision";
const CSV_F32_WIDENED: &str = "f32_widened";
const CSV_FILE_KEY: &str = "file";
const CSV_COMMENT: char = '#';
const CSV_QUOTE: char = '"';
const CSV_SEPARATOR: char = ',';
const BYTE_ORDER_MARK: char = '\u{feff}';

const MORTON_CONSTANT: f64 = 8.34254e-5;
const MORTON_UV_TERM: f64 = 2.406147e-2;
const MORTON_UV_POLE: f64 = 130.0;
const MORTON_IR_TERM: f64 = 1.5998e-4;
const MORTON_IR_POLE: f64 = 38.9;

const CTYPE1: &str = "CTYPE1";
const CUNIT1: &str = "CUNIT1";
const DC_FLAG: &str = "DC-FLAG";
const DC_FLAG_LINEAR: f64 = 0.0;
const DC_FLAG_LOG: f64 = 1.0;
const IRAF_LINEAR_CTYPE: &str = "LINEAR";
const MULTISPEC_CTYPE: &str = "MULTISPE";
const MULTISPEC_MARKER: &str = "multispec";
const LOG_SUFFIX: &str = "-LOG";
const AIR_CODE: &str = "AWAV";
const VACUUM_CODE: &str = "WAVE";
const WAT_AXES: [usize; 3] = [0, 1, 2];
const WAT_SPECTRAL_AXIS: usize = 1;
const MAX_WAT_CARDS: usize = 999;
const WAT_WTYPE: &str = "wtype";
const WAT_LABEL: &str = "label";
const WAT_UNITS: &str = "units";
const IRAF_LINEAR_WTYPE: &str = "linear";
const IRAF_WAVELENGTH_LABEL: &str = "wavelength";
const IRAF_DEFAULT_UNIT: &str = "Angstrom";
const IRAF_WAVELENGTH_UNITS: [&str; 10] = [
    "angstrom",
    "angstroms",
    "nm",
    "nanometer",
    "nanometers",
    "um",
    "micron",
    "microns",
    "micrometer",
    "micrometers",
];
const SPACE_MISSIONS: &str = "JWST, HST or Roman";
const CARDS_OF_INTEREST: [&str; 11] = [
    "OBJECT", "TARGNAME", "TELESCOP", "INSTRUME", "DETECTOR", "GRATING", "FILTER", "SRCTYPE",
    "DATE-OBS", DC_FLAG, "WAT0_001",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Spectrum1dSource {
    X1d {
        path: String,
        hdu: usize,
        extver: Option<i64>,
    },
    Wcs1d {
        path: String,
        hdu: usize,
    },
    Csv {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WavelengthFrame {
    Air,
    Vacuum,
    NotWavelength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    F64,
    F32Widened,
}

#[derive(Debug, Clone, Serialize)]
pub struct Spectrum1d {
    pub source: Spectrum1dSource,
    pub axis: SpectralAxis,
    pub wavelength_frame: WavelengthFrame,
    pub precision: Precision,
    pub flux: Vec<f64>,
    pub err: Option<Vec<f64>>,
    pub dq: Option<Vec<u32>>,
    pub dq_table: DqTable,
    pub flux_unit: Option<String>,
    pub n_rows: usize,
    pub cards: BTreeMap<String, String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wcs1dArrays {
    pub flux: Vec<f64>,
    pub precision: Precision,
    pub err: Option<Vec<f64>>,
    pub dq: Option<Vec<u32>>,
}

pub fn morton_air_refractive_index(lambda_vacuum_um: f64) -> f64 {
    let sigma2 = 1.0 / (lambda_vacuum_um * lambda_vacuum_um);
    1.0 + MORTON_CONSTANT
        + MORTON_UV_TERM / (MORTON_UV_POLE - sigma2)
        + MORTON_IR_TERM / (MORTON_IR_POLE - sigma2)
}

pub fn vacuum_to_air_morton_um(lambda_vacuum_um: f64) -> f64 {
    if !air_formula_applies(lambda_vacuum_um) {
        return lambda_vacuum_um;
    }
    lambda_vacuum_um / morton_air_refractive_index(lambda_vacuum_um)
}

impl Spectrum1d {
    pub fn from_x1d(x1d: X1dSpectrum) -> Spectrum1d {
        let mut notes = x1d.notes;
        notes.push(format!(
            "{EXTNAME_EXTRACT1D} table of a space-mission pipeline: vacuum wavelengths"
        ));
        let cards = [
            ("SRCTYPE", x1d.srctype),
            ("GRATING", x1d.grating),
            ("FILTER", x1d.filter),
            ("DETECTOR", x1d.detector),
            ("INSTRUME", x1d.instrument),
            ("TARGNAME", x1d.target),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|v| (key.to_string(), v)))
        .collect();
        Spectrum1d {
            source: Spectrum1dSource::X1d {
                path: x1d.path,
                hdu: x1d.hdu,
                extver: x1d.extver,
            },
            axis: tabulated_axis(x1d.wavelength_um, AxisKind::Wave, COL_WAVELENGTH),
            wavelength_frame: WavelengthFrame::Vacuum,
            precision: Precision::F64,
            n_rows: x1d.n_rows,
            flux: x1d.flux,
            err: x1d.flux_error,
            dq: x1d.dq,
            dq_table: x1d.dq_table,
            flux_unit: Some(x1d.flux_unit),
            cards,
            notes,
        }
    }

    pub fn from_wcs1d(
        path: &str,
        hdu: usize,
        header: &HduHeader,
        arrays: Wcs1dArrays,
    ) -> Result<Spectrum1d, String> {
        check_shape(header, &arrays)?;
        refuse_multispec(header)?;
        refuse_log_lambda(header)?;
        let len = arrays.flux.len();
        let mut notes = Vec::new();
        let (axis, wavelength_frame) = match card_upper(header, CTYPE1) {
            Some(ctype) if ctype != IRAF_LINEAR_CTYPE => {
                ctype_axis(header, len, &ctype, &mut notes)?
            }
            ctype => linear_layout_axis(header, len, ctype, &mut notes)?,
        };
        if arrays.precision == Precision::F32Widened {
            notes.push("BITPIX = -64 values decoded at f32 precision and widened to f64: about 7 significant digits kept".to_string());
        }
        let flux_unit = card(header, HEADER_BUNIT);
        if flux_unit.is_none() {
            notes.push(format!("{HEADER_BUNIT} missing: flux in native units"));
        }
        Ok(Spectrum1d {
            source: Spectrum1dSource::Wcs1d {
                path: path.to_string(),
                hdu,
            },
            axis,
            wavelength_frame,
            precision: arrays.precision,
            n_rows: len,
            flux: arrays.flux,
            err: arrays.err,
            dq: arrays.dq,
            dq_table: DqTable::select(header),
            flux_unit,
            cards: cards_of_interest(header),
            notes,
        })
    }

    pub fn from_csv(path: &str, text: &str) -> Result<Spectrum1d, String> {
        let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(text);
        let mut provenance = BTreeMap::new();
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line))
            .filter(|(_, line)| !line.trim().is_empty());
        let (header_line, header) = loop {
            match lines.next() {
                None => {
                    return Err(format!(
                        "{path} holds no header row: a spectrum CSV needs a header naming {CSV_WAVELENGTH_VACUUM_COLUMN} (or {CSV_WAVELENGTH_AIR_COLUMN}) and {CSV_FLUX_COLUMN}"
                    ))
                }
                Some((_, line)) if is_csv_comment(line) => record_provenance(line, &mut provenance),
                Some(found) => break found,
            }
        };
        let names = csv_fields(header, header_line)?;
        let columns = CsvColumns::find(&names)?;
        let mut rows = CsvRows::default();
        for (number, line) in lines.filter(|(_, line)| !is_csv_comment(line)) {
            let fields = csv_fields(line, number)?;
            if fields.len() != names.len() {
                return Err(format!(
                    "line {number}: {} fields, the header has {}",
                    fields.len(),
                    names.len()
                ));
            }
            rows.push(&columns, &fields, number)?;
        }
        if rows.flux.is_empty() {
            return Err(format!("{path} holds no data rows below its header"));
        }
        let dq_table = provenance
            .get(CSV_DQ_TABLE_KEY)
            .map(|name| csv_dq_table(name))
            .transpose()?
            .unwrap_or(DqTable::Unknown);
        let mut notes = vec![format!(
            "CSV file, column {}: {}",
            columns.wavelength_name,
            match columns.frame {
                WavelengthFrame::Air => "air wavelengths",
                _ => "vacuum wavelengths",
            }
        )];
        if let Some(file) = provenance.get(CSV_FILE_KEY) {
            notes.push(format!("exported from {file}"));
        }
        let flux_unit = rows
            .flux_unit
            .or_else(|| provenance.get(CSV_FLUX_UNIT_KEY).cloned())
            .filter(|unit| !unit.is_empty() && !unit.eq_ignore_ascii_case(CSV_NATIVE_FLUX_UNIT));
        if flux_unit.is_none() {
            notes.push(format!(
                "{CSV_FLUX_UNIT_COLUMN} missing or '{CSV_NATIVE_FLUX_UNIT}': flux in native units"
            ));
        }
        let precision = match provenance.get(CSV_PRECISION_KEY) {
            Some(name) if name.eq_ignore_ascii_case(CSV_F32_WIDENED) => {
                notes.push(format!("'# {CSV_PRECISION_KEY}: {CSV_F32_WIDENED}' line: the exported flux was decoded at f32 precision, about 7 significant digits kept"));
                Precision::F32Widened
            }
            _ => Precision::F64,
        };
        if rows.dq.is_some() && !provenance.contains_key(CSV_DQ_TABLE_KEY) {
            notes.push(format!(
                "{CSV_DQ_COLUMN} column without a '# {CSV_DQ_TABLE_KEY}:' line: no channel is excluded by its flags"
            ));
        }
        let kind = match columns.frame {
            WavelengthFrame::Air => AxisKind::Awav,
            _ => AxisKind::Wave,
        };
        Ok(Spectrum1d {
            source: Spectrum1dSource::Csv {
                path: path.to_string(),
            },
            axis: tabulated_axis(rows.wavelength, kind, columns.wavelength_name),
            wavelength_frame: columns.frame,
            precision,
            n_rows: rows.flux.len(),
            flux: rows.flux,
            err: rows.err,
            dq: rows.dq,
            dq_table,
            flux_unit,
            cards: provenance,
            notes,
        })
    }
}

struct CsvColumns {
    wavelength: usize,
    wavelength_name: &'static str,
    frame: WavelengthFrame,
    flux: usize,
    flux_unit: Option<usize>,
    err: Option<usize>,
    dq: Option<usize>,
}

impl CsvColumns {
    fn find(names: &[String]) -> Result<CsvColumns, String> {
        let column = |name: &str| {
            names
                .iter()
                .position(|header| header.trim().eq_ignore_ascii_case(name))
        };
        let (wavelength, wavelength_name, frame) = match (
            column(CSV_WAVELENGTH_AIR_COLUMN),
            column(CSV_WAVELENGTH_VACUUM_COLUMN),
        ) {
            (Some(i), _) => (i, CSV_WAVELENGTH_AIR_COLUMN, WavelengthFrame::Air),
            (None, Some(i)) => (i, CSV_WAVELENGTH_VACUUM_COLUMN, WavelengthFrame::Vacuum),
            (None, None) => {
                return Err(format!(
                    "the CSV header names no {CSV_WAVELENGTH_VACUUM_COLUMN} or {CSV_WAVELENGTH_AIR_COLUMN} column: a spectrum CSV needs wavelengths in micrometres"
                ))
            }
        };
        let flux = column(CSV_FLUX_COLUMN)
            .ok_or_else(|| format!("the CSV header names no {CSV_FLUX_COLUMN} column"))?;
        Ok(CsvColumns {
            wavelength,
            wavelength_name,
            frame,
            flux,
            flux_unit: column(CSV_FLUX_UNIT_COLUMN),
            err: column(CSV_ERR_COLUMN),
            dq: column(CSV_DQ_COLUMN),
        })
    }
}

#[derive(Default)]
struct CsvRows {
    wavelength: Vec<f64>,
    flux: Vec<f64>,
    err: Option<Vec<f64>>,
    dq: Option<Vec<u32>>,
    flux_unit: Option<String>,
}

impl CsvRows {
    fn push(&mut self, columns: &CsvColumns, fields: &[String], line: usize) -> Result<(), String> {
        self.wavelength.push(csv_number(
            &fields[columns.wavelength],
            line,
            columns.wavelength_name,
        )?);
        self.flux
            .push(csv_number(&fields[columns.flux], line, CSV_FLUX_COLUMN)?);
        if let Some(i) = columns.err {
            let value = csv_number(&fields[i], line, CSV_ERR_COLUMN)?;
            self.err.get_or_insert_with(Vec::new).push(value);
        }
        if let Some(i) = columns.dq {
            let flags = csv_flags(&fields[i], line)?;
            self.dq.get_or_insert_with(Vec::new).push(flags);
        }
        if let Some(i) = columns.flux_unit.filter(|_| self.flux_unit.is_none()) {
            let unit = fields[i].trim();
            if !unit.is_empty() {
                self.flux_unit = Some(unit.to_string());
            }
        }
        Ok(())
    }
}

fn is_csv_comment(line: &str) -> bool {
    line.trim_start().starts_with(CSV_COMMENT)
}

fn record_provenance(line: &str, provenance: &mut BTreeMap<String, String>) {
    let comment = line.trim_start().trim_start_matches(CSV_COMMENT);
    if let Some((key, value)) = comment.split_once(':') {
        let key = key.trim();
        if !key.is_empty() {
            provenance
                .entry(key.to_string())
                .or_insert_with(|| value.trim().to_string());
        }
    }
}

fn csv_fields(line: &str, number: usize) -> Result<Vec<String>, String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, CSV_QUOTE) if chars.peek() == Some(&CSV_QUOTE) => {
                field.push(CSV_QUOTE);
                chars.next();
            }
            (true, CSV_QUOTE) => quoted = false,
            (false, CSV_QUOTE) if field.trim().is_empty() => {
                field.clear();
                quoted = true;
            }
            (false, CSV_SEPARATOR) => fields.push(std::mem::take(&mut field)),
            (_, c) => field.push(c),
        }
    }
    if quoted {
        return Err(format!("line {number}: unterminated quoted field"));
    }
    fields.push(field);
    Ok(fields)
}

fn csv_number(field: &str, line: usize, column: &str) -> Result<f64, String> {
    let text = field.trim();
    if text.is_empty() {
        return Ok(f64::NAN);
    }
    text.parse::<f64>()
        .map_err(|_| format!("line {line}: {column} value '{text}' is not a number"))
}

fn csv_flags(field: &str, line: usize) -> Result<u32, String> {
    let text = field.trim();
    text.parse::<u32>().map_err(|_| {
        format!("line {line}: {CSV_DQ_COLUMN} value '{text}' is not an unsigned 32-bit integer")
    })
}

fn csv_dq_table(name: &str) -> Result<DqTable, String> {
    match name.to_ascii_lowercase().as_str() {
        "jwst" => Ok(DqTable::Jwst),
        "roman" => Ok(DqTable::Roman),
        "hst" => Ok(DqTable::Hst),
        "unknown" => Ok(DqTable::Unknown),
        _ => Err(format!(
            "{CSV_DQ_TABLE_KEY} '{name}' is not one of jwst, roman, hst, unknown"
        )),
    }
}

fn card(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn card_upper(header: &HduHeader, key: &str) -> Option<String> {
    card(header, key).map(|s| s.to_uppercase())
}

fn tabulated_axis(values: Vec<f64>, kind: AxisKind, ctype: &str) -> SpectralAxis {
    let steps: Vec<f64> = values
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| d.is_finite())
        .collect();
    SpectralAxis {
        kind,
        ctype: ctype.to_string(),
        unit: DEFAULT_WAVELENGTH_UNIT.to_string(),
        header_unit: DEFAULT_WAVELENGTH_UNIT.to_string(),
        header_scale: 1.0,
        crval: values.first().copied().unwrap_or(f64::NAN),
        cdelt: exact_median_f64(&steps),
        crpix: 1.0,
        values,
        rest_wavelength_um: None,
        rest_frequency_hz: None,
        specsys: None,
        velosys: None,
        notes: Vec::new(),
        tabulated: true,
    }
}

fn check_shape(header: &HduHeader, arrays: &Wcs1dArrays) -> Result<(), String> {
    match header.get_i64("NAXIS") {
        Some(1) => {}
        Some(n) => return Err(format!("NAXIS = {n}: a 1D spectrum needs NAXIS = 1")),
        None => return Err("NAXIS missing: a 1D spectrum needs NAXIS = 1".to_string()),
    }
    let len = arrays.flux.len();
    if len == 0 {
        return Err("the spectrum holds no values".to_string());
    }
    if let Some(naxis1) = header.get_i64("NAXIS1").filter(|&n| n != len as i64) {
        return Err(format!("NAXIS1 = {naxis1} but the data hold {len} values"));
    }
    let companions = [
        ("ERR", arrays.err.as_ref().map(Vec::len)),
        ("DQ", arrays.dq.as_ref().map(Vec::len)),
    ];
    for (name, count) in companions {
        if let Some(count) = count.filter(|&n| n != len) {
            return Err(format!(
                "{name} holds {count} values but the spectrum has {len}"
            ));
        }
    }
    Ok(())
}

fn wat_cards(header: &HduHeader, axis: usize) -> Vec<(String, String)> {
    (1..=MAX_WAT_CARDS)
        .map(|n| format!("WAT{axis}_{n:03}"))
        .map_while(|key| {
            let raw = header.get(&key)?.trim_end();
            let text = raw
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .unwrap_or(raw)
                .trim_end();
            Some((key, text.to_string()))
        })
        .collect()
}

fn wat_text(cards: &[(String, String)]) -> Option<String> {
    let last = cards.len().checked_sub(1)?;
    Some(
        cards
            .iter()
            .enumerate()
            .map(|(i, (_, text))| {
                if i < last {
                    format!("{text:<IRAF_WAT_CARD_WIDTH$}")
                } else {
                    text.clone()
                }
            })
            .collect(),
    )
}

fn wat_attributes(text: &str) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    let mut rest = text.trim_start();
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().to_lowercase();
        let after = rest[eq + 1..].trim_start();
        let (value, remainder) = match after.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').unwrap_or((quoted, "")),
            None => after.split_at(after.find(char::is_whitespace).unwrap_or(after.len())),
        };
        attributes.push((key, value.trim().to_string()));
        rest = remainder.trim_start();
    }
    attributes
}

fn wat_attribute<'a>(attributes: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn refuse_multispec(header: &HduHeader) -> Result<(), String> {
    if let Some(ctype) = card_upper(header, CTYPE1).filter(|c| c.starts_with(MULTISPEC_CTYPE)) {
        return Err(format!(
            "{CTYPE1} = '{ctype}': IRAF multispec spectra are not supported"
        ));
    }
    let flagged = WAT_AXES
        .iter()
        .flat_map(|&axis| wat_cards(header, axis))
        .find(|(_, text)| text.to_lowercase().contains(MULTISPEC_MARKER));
    match flagged {
        Some((key, text)) => Err(format!(
            "{key} = '{}': IRAF multispec spectra are not supported",
            text.trim()
        )),
        None => Ok(()),
    }
}

fn refuse_log_lambda(header: &HduHeader) -> Result<(), String> {
    match header.get_f64(DC_FLAG) {
        Some(flag) if flag == DC_FLAG_LOG => {
            return Err(format!(
                "{DC_FLAG} = 1: log-lambda dispersion is not supported"
            ));
        }
        Some(flag) if flag != DC_FLAG_LINEAR => {
            return Err(format!(
                "{DC_FLAG} = {flag}: the spectrum carries no dispersion solution"
            ));
        }
        _ => {}
    }
    if let Some(ctype) = card_upper(header, CTYPE1).filter(|c| c.ends_with(LOG_SUFFIX)) {
        return Err(format!(
            "{CTYPE1} = '{ctype}': log-lambda dispersion is not supported"
        ));
    }
    Ok(())
}

fn space_mission(header: &HduHeader) -> Option<String> {
    card_upper(header, "TELESCOP").filter(|telescop| is_spacecraft(telescop))
}

fn mission_frame(header: &HduHeader, described: &str, notes: &mut Vec<String>) -> WavelengthFrame {
    match space_mission(header) {
        Some(telescop) => {
            notes.push(format!(
                "{described} on a space-mission file (TELESCOP '{telescop}'): vacuum wavelengths"
            ));
            WavelengthFrame::Vacuum
        }
        None => {
            notes.push(format!(
                "{described} with no {SPACE_MISSIONS} TELESCOP: read as air wavelengths, the ground-based default"
            ));
            WavelengthFrame::Air
        }
    }
}

fn with_code(ctype: &str, code: &str) -> String {
    match ctype.find('-') {
        Some(pos) => format!("{code}{}", &ctype[pos..]),
        None => code.to_string(),
    }
}

fn frame_code(frame: WavelengthFrame) -> &'static str {
    match frame {
        WavelengthFrame::Air => AIR_CODE,
        _ => VACUUM_CODE,
    }
}

fn normalised_axis(
    header: &HduHeader,
    len: usize,
    ctype: &str,
    unit: Option<&str>,
    label: &str,
) -> Result<SpectralAxis, String> {
    let mut normalised = header.clone();
    normalised.set(CTYPE1, ctype.to_string());
    if let Some(unit) = unit {
        normalised.set(CUNIT1, unit.to_string());
    }
    spectral_axis_on(&normalised, 1, len).map_err(|e| format!("{label}: {e}"))
}

fn ctype_axis(
    header: &HduHeader,
    len: usize,
    ctype: &str,
    notes: &mut Vec<String>,
) -> Result<(SpectralAxis, WavelengthFrame), String> {
    let label = format!("{CTYPE1} = '{ctype}'");
    if is_non_linear_spectral_ctype(ctype) {
        return Err(format!(
            "{label}: non-linear and tabulated spectral axes are not supported in a 1D spectrum"
        ));
    }
    let probe = spectral_axis_on(header, 1, len).map_err(|e| format!("{label}: {e}"))?;
    match probe.kind {
        AxisKind::Unknown => Err(format!("{label} is not a spectral axis")),
        AxisKind::Awav => {
            notes.push(format!("{CTYPE1} '{ctype}': air wavelengths"));
            Ok((probe, WavelengthFrame::Air))
        }
        AxisKind::Wave => {
            let frame = mission_frame(header, &format!("{CTYPE1} '{ctype}'"), notes);
            let mut axis = normalised_axis(
                header,
                len,
                &with_code(ctype, frame_code(frame)),
                None,
                &label,
            )?;
            axis.ctype = ctype.to_string();
            Ok((axis, frame))
        }
        _ => {
            notes.push(format!(
                "{CTYPE1} '{ctype}' is not a wavelength axis: air and vacuum do not apply"
            ));
            Ok((probe, WavelengthFrame::NotWavelength))
        }
    }
}

fn iraf_wavelength_unit(
    header: &HduHeader,
    text: &str,
    notes: &mut Vec<String>,
) -> Result<String, String> {
    let attributes = wat_attributes(text);
    let wtype = wat_attribute(&attributes, WAT_WTYPE).unwrap_or_default();
    if !wtype.eq_ignore_ascii_case(IRAF_LINEAR_WTYPE) {
        return Err(format!("WAT1 {WAT_WTYPE}={wtype}: only {WAT_WTYPE}={IRAF_LINEAR_WTYPE} IRAF axes are supported"));
    }
    let axis_label = wat_attribute(&attributes, WAT_LABEL).unwrap_or_default();
    if !axis_label.eq_ignore_ascii_case(IRAF_WAVELENGTH_LABEL) {
        return Err(format!(
            "WAT1 {WAT_LABEL}={axis_label}: the IRAF axis is not a wavelength axis"
        ));
    }
    match wat_attribute(&attributes, WAT_UNITS) {
        Some(unit) if IRAF_WAVELENGTH_UNITS.contains(&unit.to_lowercase().as_str()) => {
            Ok(unit.to_string())
        }
        Some(unit) => Err(format!(
            "WAT1 {WAT_UNITS}={unit} is not a supported wavelength unit (angstroms, nm, microns)"
        )),
        None => {
            match card(header, CUNIT1) {
                Some(unit) => {
                    notes.push(format!("WAT1 has no {WAT_UNITS}=: {CUNIT1} '{unit}' used"));
                    Ok(unit)
                }
                None => {
                    notes.push(format!("WAT1 has no {WAT_UNITS}= and {CUNIT1} is missing: {IRAF_DEFAULT_UNIT} assumed"));
                    Ok(IRAF_DEFAULT_UNIT.to_string())
                }
            }
        }
    }
}

fn linear_layout_axis(
    header: &HduHeader,
    len: usize,
    ctype: Option<String>,
    notes: &mut Vec<String>,
) -> Result<(SpectralAxis, WavelengthFrame), String> {
    let label = match &ctype {
        Some(c) => format!("{CTYPE1} = '{c}'"),
        None => format!("{CTYPE1} missing"),
    };
    let (unit, described) = match wat_text(&wat_cards(header, WAT_SPECTRAL_AXIS)) {
        Some(text) => {
            let unit = iraf_wavelength_unit(header, &text, notes)?;
            let described = format!("IRAF linear wavelength axis ({label}, WAT1 {WAT_UNITS}={unit})");
            (unit, described)
        }
        None => match card(header, CUNIT1).filter(|u| wavelength_unit_to_um(u).is_some()) {
            Some(unit) => {
                let described = format!("{label} with no IRAF WAT1 cards: {CUNIT1} '{unit}' read as a linear wavelength axis");
                (unit, described)
            }
            None => {
                return Err(format!(
                    "{label}, no IRAF WAT1 wavelength label and no wavelength {CUNIT1}: the array has no wavelength solution"
                ))
            }
        },
    };
    let frame = mission_frame(header, &described, notes);
    let mut axis = normalised_axis(header, len, frame_code(frame), Some(&unit), &label)?;
    axis.ctype = ctype.unwrap_or_default();
    Ok((axis, frame))
}

fn cards_of_interest(header: &HduHeader) -> BTreeMap<String, String> {
    let wat1 = wat_cards(header, WAT_SPECTRAL_AXIS)
        .into_iter()
        .map(|(key, _)| key);
    CARDS_OF_INTEREST
        .iter()
        .map(|key| key.to_string())
        .chain(wat1)
        .filter_map(|key| card(header, &key).map(|value| (key, value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::x1d::read_x1d_spectrum;
    use crate::core::analysis::x1d::test_fixtures::write_x1d_fixture;
    use crate::core::astrometry::spectral::AxisKind;

    const LEN: usize = 64;
    const PATH: &str = "spec.fits";
    const IRAF_WAT1: &str = "wtype=linear label=Wavelength units=angstroms";

    fn header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        h.set("NAXIS", "1".to_string());
        h.set("NAXIS1", LEN.to_string());
        for (k, v) in pairs {
            h.set(k, v.to_string());
        }
        h
    }

    fn arrays() -> Wcs1dArrays {
        Wcs1dArrays {
            flux: (0..LEN).map(|i| 1.0 + i as f64).collect(),
            precision: Precision::F64,
            err: None,
            dq: None,
        }
    }

    fn open(pairs: &[(&str, &str)]) -> Result<Spectrum1d, String> {
        Spectrum1d::from_wcs1d(PATH, 0, &header(pairs), arrays())
    }

    fn iraf_cards(units: &str) -> Vec<(&'static str, String)> {
        vec![
            ("CTYPE1", "LINEAR".to_string()),
            ("WAT0_001", "system=equispec".to_string()),
            (
                "WAT1_001",
                format!("wtype=linear label=Wavelength units={units}"),
            ),
            ("DC-FLAG", "0".to_string()),
            ("CRVAL1", "4000.0".to_string()),
            ("CDELT1", "2.5".to_string()),
            ("CD1_1", "2.5".to_string()),
            ("CRPIX1", "1.0".to_string()),
        ]
    }

    fn open_owned(cards: &[(&str, String)]) -> Result<Spectrum1d, String> {
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (*k, v.as_str())).collect();
        open(&pairs)
    }

    fn assert_relative(actual: f64, expected: f64, tolerance: f64) {
        let relative = ((actual - expected) / expected).abs();
        assert!(
            relative <= tolerance,
            "actual {actual} expected {expected} relative {relative:e}"
        );
    }

    #[test]
    fn crval_cdelt_crpix_angstrom_axis_gives_wavelengths_exact_to_1e9_relative() {
        let crval = 6512.345678_9;
        let cdelt = 0.047_31;
        let crpix = 3.5;
        let s = open(&[
            ("CTYPE1", "WAVE"),
            ("CUNIT1", "Angstrom"),
            ("CRVAL1", "6512.3456789"),
            ("CDELT1", "0.04731"),
            ("CRPIX1", "3.5"),
            ("BUNIT", "erg/s/cm2/A"),
        ])
        .unwrap();
        assert_eq!(s.axis.values.len(), LEN);
        assert_eq!(s.n_rows, LEN);
        assert_eq!(s.axis.unit, "um");
        assert_eq!(s.axis.header_unit, "Angstrom");
        for (i, &value) in s.axis.values.iter().enumerate() {
            let angstrom = crval + (i as f64 + 1.0 - crpix) * cdelt;
            assert_relative(value, angstrom * 1e-4, 1e-9);
        }
        assert_eq!(s.flux, arrays().flux);
        assert_eq!(s.flux_unit.as_deref(), Some("erg/s/cm2/A"));
        assert_eq!(s.precision, Precision::F64);
        assert_eq!(
            s.source,
            Spectrum1dSource::Wcs1d {
                path: PATH.to_string(),
                hdu: 0
            }
        );
    }

    #[test]
    fn iraf_linear_wat1_layout_opens_as_an_angstrom_air_wavelength_axis() {
        let s = open_owned(&iraf_cards("angstroms")).unwrap();
        assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
        assert_eq!(s.axis.kind, AxisKind::Awav);
        assert_eq!(s.axis.ctype, "LINEAR");
        assert_eq!(s.axis.unit, "um");
        assert_eq!(s.axis.header_unit, "angstroms");
        assert_relative(s.axis.values[0], 0.4, 1e-12);
        assert_relative(s.axis.values[10], (4000.0 + 25.0) * 1e-4, 1e-12);
        assert_eq!(s.cards.get("WAT1_001").map(String::as_str), Some(IRAF_WAT1));
        assert_eq!(s.cards.get("DC-FLAG").map(String::as_str), Some("0"));
        assert!(
            s.notes.iter().any(|n| n.contains("air wavelengths")),
            "{:?}",
            s.notes
        );
        assert!(s.flux_unit.is_none());
        assert!(
            s.notes.iter().any(|n| n.contains("BUNIT missing")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn iraf_linear_axis_with_an_unknown_wat1_unit_is_refused_naming_the_unit() {
        let err = open_owned(&iraf_cards("furlongs")).unwrap_err();
        assert!(err.contains("units=furlongs"), "{err}");
    }

    #[test]
    fn iraf_wat1_split_at_a_blank_across_cards_is_rejoined_at_68_characters() {
        let mut cards = iraf_cards("angstroms");
        cards.retain(|(k, _)| *k != "WAT1_001");
        cards.push(("WAT1_001", "wtype=linear label=Wavelength".to_string()));
        cards.push(("WAT1_002", "units=nm".to_string()));
        let s = open_owned(&cards).unwrap();
        assert_eq!(s.axis.header_unit, "nm");
        assert_relative(s.axis.values[0], 4.0, 1e-12);
        assert!(s.cards.contains_key("WAT1_002"), "{:?}", s.cards);
    }

    #[test]
    fn iraf_linear_axis_falls_back_to_cunit1_then_to_angstrom_with_a_note() {
        let mut cards = iraf_cards("angstroms");
        cards.retain(|(k, _)| *k != "WAT1_001");
        cards.push(("WAT1_001", "wtype=linear label=Wavelength".to_string()));
        let mut with_cunit = cards.clone();
        with_cunit.push(("CUNIT1", "nm".to_string()));
        let s = open_owned(&with_cunit).unwrap();
        assert_eq!(s.axis.header_unit, "nm");
        assert!(
            s.notes.iter().any(|n| n.contains("CUNIT1 'nm'")),
            "{:?}",
            s.notes
        );

        let s = open_owned(&cards).unwrap();
        assert_relative(s.axis.values[0], 0.4, 1e-12);
        assert!(
            s.notes.iter().any(|n| n.contains("Angstrom assumed")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn linear_or_absent_ctype_without_a_wavelength_solution_is_refused() {
        let err = open(&[("CTYPE1", "LINEAR"), ("CRVAL1", "1"), ("CDELT1", "1")]).unwrap_err();
        assert!(
            err.contains("CTYPE1 = 'LINEAR'") && err.contains("no wavelength solution"),
            "{err}"
        );

        let err = open(&[
            ("CTYPE1", "LINEAR"),
            ("WAT1_001", "wtype=linear label=Pixel"),
            ("CRVAL1", "1"),
            ("CDELT1", "1"),
        ])
        .unwrap_err();
        assert!(err.contains("label=Pixel"), "{err}");

        let s = open(&[("CUNIT1", "nm"), ("CRVAL1", "500"), ("CDELT1", "0.5")]).unwrap();
        assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
        assert_eq!(s.axis.ctype, "");
        assert!(
            s.notes.iter().any(|n| n.contains("CTYPE1 missing")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn bess_shaped_wave_header_without_a_mission_card_opens_as_air_with_the_note() {
        let s = open(&[
            ("CTYPE1", "WAVE"),
            ("CUNIT1", "Angstrom"),
            ("CRVAL1", "6400.0"),
            ("CDELT1", "0.1"),
            ("CRPIX1", "1.0"),
            ("OBJECT", "HD 37795"),
            ("DATE-OBS", "2024-01-02T20:10:00"),
            ("BSS_VHEL", "0.0"),
            ("BSS_INST", "C11 LHIRES III 2400"),
            ("BSS_SITE", "Observatory"),
        ])
        .unwrap();
        assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
        assert_eq!(s.axis.kind, AxisKind::Awav);
        assert_eq!(s.axis.ctype, "WAVE");
        let note = s
            .notes
            .iter()
            .find(|n| n.contains("air wavelengths"))
            .unwrap_or_else(|| panic!("{:?}", s.notes));
        assert!(note.contains("ground-based"), "{note}");
        assert!(
            !s.axis.notes.iter().any(|n| n.contains("vacuum")),
            "{:?}",
            s.axis.notes
        );
        assert_eq!(s.cards.get("OBJECT").map(String::as_str), Some("HD 37795"));
    }

    #[test]
    fn wave_with_a_space_mission_telescope_is_vacuum_and_awav_is_always_air() {
        let jwst = open(&[
            ("CTYPE1", "WAVE"),
            ("CUNIT1", "um"),
            ("CRVAL1", "2.0"),
            ("CDELT1", "0.001"),
            ("TELESCOP", "JWST"),
        ])
        .unwrap();
        assert_eq!(jwst.wavelength_frame, WavelengthFrame::Vacuum);
        assert_eq!(jwst.axis.kind, AxisKind::Wave);
        assert!(
            jwst.notes
                .iter()
                .any(|n| n.contains("vacuum wavelengths") && n.contains("JWST")),
            "{:?}",
            jwst.notes
        );
        assert_eq!(jwst.dq_table, DqTable::Jwst);

        let synonym = open(&[
            ("CTYPE1", "WAVELENGTH"),
            ("CUNIT1", "um"),
            ("CRVAL1", "2.0"),
            ("CDELT1", "0.001"),
            ("TELESCOP", "HST"),
        ])
        .unwrap();
        assert_eq!(synonym.wavelength_frame, WavelengthFrame::Vacuum);
        assert_eq!(synonym.axis.ctype, "WAVELENGTH");
        assert!(
            !synonym.axis.notes.iter().any(|n| n.contains("CTYPE3")),
            "{:?}",
            synonym.axis.notes
        );

        let air = open(&[
            ("CTYPE1", "AWAV"),
            ("CUNIT1", "Angstrom"),
            ("CRVAL1", "5000"),
            ("CDELT1", "1"),
            ("TELESCOP", "JWST"),
        ])
        .unwrap();
        assert_eq!(air.wavelength_frame, WavelengthFrame::Air);
        assert_eq!(air.axis.kind, AxisKind::Awav);

        let freq = open(&[
            ("CTYPE1", "FREQ"),
            ("CUNIT1", "GHz"),
            ("CRVAL1", "100"),
            ("CDELT1", "0.01"),
        ])
        .unwrap();
        assert_eq!(freq.wavelength_frame, WavelengthFrame::NotWavelength);
        assert_eq!(freq.axis.kind, AxisKind::Freq);
    }

    #[test]
    fn multispec_log_lambda_and_tabulated_headers_are_refused_naming_the_card() {
        let err = open(&[
            ("CTYPE1", "MULTISPE"),
            ("WAT0_001", "system=multispec"),
            (
                "WAT1_001",
                "wtype=multispec label=Wavelength units=angstroms",
            ),
            (
                "WAT2_001",
                "wtype=multispec spec1 = \"1 1 0 4000. 2.5 64 0. 0. 0. 1. 0.\"",
            ),
        ])
        .unwrap_err();
        assert!(
            err.contains("CTYPE1 = 'MULTISPE'") && err.contains("multispec"),
            "{err}"
        );

        let mut cards = iraf_cards("angstroms");
        cards.push((
            "WAT2_001",
            "wtype=multispec spec1 = \"1 1 0 4000. 2.5 64\"".to_string(),
        ));
        let err = open_owned(&cards).unwrap_err();
        assert!(
            err.contains("WAT2_001") && err.contains("multispec"),
            "{err}"
        );

        let mut cards = iraf_cards("angstroms");
        cards.retain(|(k, _)| *k != "DC-FLAG");
        cards.push(("DC-FLAG", "1".to_string()));
        let err = open_owned(&cards).unwrap_err();
        assert!(
            err.contains("DC-FLAG = 1") && err.contains("log-lambda"),
            "{err}"
        );

        let err = open(&[
            ("CTYPE1", "WAVE-LOG"),
            ("CUNIT1", "m"),
            ("CRVAL1", "1e-6"),
            ("CDELT1", "1e-9"),
        ])
        .unwrap_err();
        assert!(
            err.contains("CTYPE1 = 'WAVE-LOG'") && err.contains("log-lambda"),
            "{err}"
        );

        let err = open(&[
            ("CTYPE1", "WAVE-TAB"),
            ("CUNIT1", "m"),
            ("CRVAL1", "0"),
            ("CDELT1", "1"),
        ])
        .unwrap_err();
        assert!(err.contains("CTYPE1 = 'WAVE-TAB'"), "{err}");

        let err = open(&[
            ("CTYPE1", "RA---TAN"),
            ("CRVAL1", "10"),
            ("CDELT1", "0.001"),
        ])
        .unwrap_err();
        assert!(
            err.contains("CTYPE1 = 'RA---TAN'") && err.contains("not a spectral axis"),
            "{err}"
        );
    }

    #[test]
    fn shape_mismatches_and_non_1d_headers_are_refused() {
        let mut h = header(&[("CTYPE1", "WAVE"), ("CRVAL1", "1e-6"), ("CDELT1", "1e-9")]);
        h.set("NAXIS", "2".to_string());
        let err = Spectrum1d::from_wcs1d(PATH, 0, &h, arrays()).unwrap_err();
        assert!(err.contains("NAXIS = 2"), "{err}");

        let h = header(&[("CTYPE1", "WAVE"), ("CRVAL1", "1e-6"), ("CDELT1", "1e-9")]);
        let mut short = arrays();
        short.flux.pop();
        let err = Spectrum1d::from_wcs1d(PATH, 0, &h, short).unwrap_err();
        assert!(
            err.contains(&format!("NAXIS1 = {LEN}"))
                && err.contains(&format!("{} values", LEN - 1)),
            "{err}"
        );

        let mut bad_err = arrays();
        bad_err.err = Some(vec![1.0; LEN + 1]);
        let err = Spectrum1d::from_wcs1d(PATH, 0, &h, bad_err).unwrap_err();
        assert!(err.contains("ERR holds"), "{err}");

        let mut bad_dq = arrays();
        bad_dq.dq = Some(vec![0; 3]);
        let err = Spectrum1d::from_wcs1d(PATH, 0, &h, bad_dq).unwrap_err();
        assert!(err.contains("DQ holds 3 values"), "{err}");
    }

    #[test]
    fn widened_f32_precision_and_companion_arrays_are_carried_with_a_note() {
        let h = header(&[
            ("CTYPE1", "WAVE"),
            ("CUNIT1", "um"),
            ("CRVAL1", "2.0"),
            ("CDELT1", "0.001"),
            ("TELESCOP", "JWST"),
        ]);
        let mut input = arrays();
        input.precision = Precision::F32Widened;
        input.err = Some(vec![0.5; LEN]);
        input.dq = Some((0..LEN as u32).map(|i| i % 2).collect());
        let s = Spectrum1d::from_wcs1d("cal.fits", 1, &h, input.clone()).unwrap();
        assert_eq!(s.precision, Precision::F32Widened);
        assert!(
            s.notes
                .iter()
                .any(|n| n.contains("BITPIX = -64") && n.contains("f32")),
            "{:?}",
            s.notes
        );
        assert_eq!(s.err, input.err);
        assert_eq!(s.dq, input.dq);
        assert_eq!(
            s.source,
            Spectrum1dSource::Wcs1d {
                path: "cal.fits".to_string(),
                hdu: 1
            }
        );
    }

    #[test]
    fn morton_2000_converts_vacuum_h_alpha_to_its_standard_air_wavelength() {
        let air_angstrom = vacuum_to_air_morton_um(6564.61e-4) * 1e4;
        assert!((air_angstrom - 6562.79).abs() <= 0.01, "{air_angstrom}");
        assert!(
            morton_air_refractive_index(0.65646) > 1.000_27,
            "{}",
            morton_air_refractive_index(0.65646)
        );
        assert_eq!(vacuum_to_air_morton_um(0.15), 0.15);
    }

    #[test]
    fn from_x1d_keeps_the_table_arrays_bit_identical_and_states_vacuum() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_x1d_fixture(dir.path(), "jw_nrs1_x1d.fits", 6, &[1], "um");
        let x1d = read_x1d_spectrum(path.to_str().unwrap(), None).unwrap();
        let expected = x1d.clone();
        let s = Spectrum1d::from_x1d(x1d);
        assert_eq!(
            s.source,
            Spectrum1dSource::X1d {
                path: expected.path.clone(),
                hdu: 1,
                extver: Some(1)
            }
        );
        assert_eq!(s.axis.values, expected.wavelength_um);
        assert_eq!(s.axis.kind, AxisKind::Wave);
        assert_eq!(s.axis.unit, "um");
        assert!(s.axis.tabulated);
        assert_eq!(s.flux, expected.flux);
        assert_eq!(s.err, expected.flux_error);
        assert_eq!(s.dq, expected.dq);
        assert_eq!(s.dq_table, DqTable::Jwst);
        assert_eq!(s.flux_unit.as_deref(), Some("Jy"));
        assert_eq!(s.n_rows, 6);
        assert_eq!(s.wavelength_frame, WavelengthFrame::Vacuum);
        assert_eq!(s.precision, Precision::F64);
        assert_eq!(s.cards.get("GRATING").map(String::as_str), Some("G235H"));
        assert_eq!(s.cards.get("SRCTYPE").map(String::as_str), Some("POINT"));
        assert!(
            s.notes.iter().any(|n| n.contains("vacuum wavelengths")),
            "{:?}",
            s.notes
        );
    }

    const CSV_PATH: &str = "C:/data/jw_test_x1d_spectrum.csv";
    const CSV_ROWS: usize = 12;
    const CSV_NAN_FLUX_ROW: usize = 5;
    const CSV_NAN_ERR_ROW: usize = 7;
    const CSV_ROUND_TRIP_TOLERANCE: f64 = 1e-12;

    fn exported_csv() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("spectrum1d")
            .join("document_export.csv");
        std::fs::read_to_string(path).unwrap()
    }

    fn exported_wavelength(i: usize) -> f64 {
        1.8 + i as f64 * 0.00137
    }

    fn exported_flux(i: usize) -> f64 {
        if i == CSV_NAN_FLUX_ROW {
            f64::NAN
        } else {
            0.0123 + i as f64 * 4.56e-5
        }
    }

    fn exported_err(i: usize) -> f64 {
        if i == CSV_NAN_ERR_ROW {
            f64::NAN
        } else {
            1e-4 + i as f64 * 1e-6
        }
    }

    fn exported_dq() -> Vec<u32> {
        let mut dq = vec![0; CSV_ROWS];
        dq[3] = 1;
        dq[9] = 1 << 31;
        dq[10] = 2;
        dq
    }

    fn assert_round_trip(what: &str, actual: &[f64], expected: impl Fn(usize) -> f64) {
        assert_eq!(actual.len(), CSV_ROWS, "{what}");
        for (i, &value) in actual.iter().enumerate() {
            let want = expected(i);
            if want.is_nan() {
                assert!(value.is_nan(), "{what}[{i}] = {value}, expected NaN");
            } else {
                assert!(
                    ((value - want) / want).abs() <= CSV_ROUND_TRIP_TOLERANCE,
                    "{what}[{i}] = {value}, expected {want}"
                );
            }
        }
    }

    #[test]
    fn from_csv_reads_the_frontend_document_export_back_to_1e12() {
        let lf = exported_csv().replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        let with_bom = format!("\u{feff}{crlf}");
        for text in [lf, crlf, with_bom] {
            let s = Spectrum1d::from_csv(CSV_PATH, &text).unwrap();
            assert_eq!(s.n_rows, CSV_ROWS);
            assert_round_trip("wavelength", &s.axis.values, exported_wavelength);
            assert_round_trip("flux", &s.flux, exported_flux);
            assert_round_trip("err", s.err.as_deref().unwrap(), exported_err);
            assert_eq!(s.dq, Some(exported_dq()));
            assert_eq!(s.dq_table, DqTable::Jwst);
            assert_eq!(s.flux_unit.as_deref(), Some("Jy"));
            assert_eq!(s.wavelength_frame, WavelengthFrame::Vacuum);
            assert_eq!(s.precision, Precision::F64);
            assert_eq!(s.axis.kind, AxisKind::Wave);
            assert_eq!(s.axis.unit, "um");
            assert!(s.axis.tabulated);
            assert_eq!(
                s.source,
                Spectrum1dSource::Csv {
                    path: CSV_PATH.to_string()
                }
            );
            assert_eq!(
                s.cards.get("file").map(String::as_str),
                Some("jw_test_x1d.fits")
            );
            assert!(
                s.notes
                    .iter()
                    .any(|n| n.contains("wavelength_vacuum_um") && n.contains("vacuum wavelengths")),
                "{:?}",
                s.notes
            );
        }
    }

    #[test]
    fn from_csv_reads_an_air_wavelength_column_a_quoted_unit_and_dq_without_a_table() {
        let text = "# file: ha.fits\nchannel,wavelength_air_um,flux,flux_unit,dq\n0,0.6562,1.5,\"erg/s/cm2/A, x1e-17\",0\n1,0.6563,2.5,\"erg/s/cm2/A, x1e-17\",4\n";
        let s = Spectrum1d::from_csv(CSV_PATH, text).unwrap();
        assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
        assert_eq!(s.axis.kind, AxisKind::Awav);
        assert_eq!(s.axis.values, vec![0.6562, 0.6563]);
        assert_eq!(s.flux, vec![1.5, 2.5]);
        assert_eq!(s.flux_unit.as_deref(), Some("erg/s/cm2/A, x1e-17"));
        assert!(s.err.is_none());
        assert_eq!(s.dq, Some(vec![0, 4]));
        assert_eq!(s.dq_table, DqTable::Unknown);
        assert!(
            s.notes.iter().any(|n| n.contains("dq_table")),
            "{:?}",
            s.notes
        );
        assert!(
            s.notes.iter().any(|n| n.contains("air wavelengths")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn from_csv_takes_the_flux_unit_from_the_provenance_line_without_a_unit_column() {
        let text = "# flux_unit: MJy/sr\nwavelength_vacuum_um,flux\n2.0,1\n2.1,2\n";
        let s = Spectrum1d::from_csv(CSV_PATH, text).unwrap();
        assert_eq!(s.flux_unit.as_deref(), Some("MJy/sr"));
        let s = Spectrum1d::from_csv(CSV_PATH, "wavelength_vacuum_um,flux\n2.0,1\n").unwrap();
        assert!(s.flux_unit.is_none());
        assert!(
            s.notes.iter().any(|n| n.contains("native units")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn from_csv_reads_an_air_document_export_on_its_own_air_column_not_the_greisen_vacuum_one() {
        let text = "# file: ha.fits\n# wavelength_air_note: wavelength_vacuum_um converted from air\nchannel,wavelength_vacuum_um,wavelength_air_um,flux,flux_unit\n0,0.6564712848862345,0.65628,1.5,Jy\n1,0.6564812876247376,0.65629,2.5,Jy\n";
        let s = Spectrum1d::from_csv(CSV_PATH, text).unwrap();
        assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
        assert_eq!(s.axis.kind, AxisKind::Awav);
        assert_eq!(s.axis.ctype, CSV_WAVELENGTH_AIR_COLUMN);
        assert_eq!(s.axis.values, vec![0.65628, 0.65629]);
        assert!(
            s.notes
                .iter()
                .any(|n| n.contains("wavelength_air_um") && n.contains("air wavelengths")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn from_csv_reads_the_frontend_air_export_back_in_air_without_a_unit_at_f32_precision() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("spectrum1d")
            .join("air_document_export.csv");
        let lf = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        for text in [lf.clone(), lf.replace('\n', "\r\n")] {
            let s = Spectrum1d::from_csv(CSV_PATH, &text).unwrap();
            assert_eq!(s.wavelength_frame, WavelengthFrame::Air);
            assert_eq!(s.axis.kind, AxisKind::Awav);
            assert_eq!(s.axis.values, vec![0.65628, 0.65629, 0.6563]);
            assert_eq!(s.flux, vec![1.5, 2.5, 3.5]);
            assert!(s.flux_unit.is_none(), "{:?}", s.flux_unit);
            assert_eq!(s.precision, Precision::F32Widened);
            assert!(s.err.is_none() && s.dq.is_none());
        }
    }

    const REGION_ROWS: usize = 6;
    const REGION_WEIGHT: f64 = 12.5;
    const REGION_JY_PER_SUM: f64 = 2.35e-5;
    const REGION_NAN_FLUX_ROW: usize = 2;
    const REGION_NAN_ERR_ROW: usize = 4;

    fn region_sum(i: usize) -> f64 {
        if i == REGION_NAN_FLUX_ROW {
            f64::NAN
        } else {
            3.5 + i as f64 * 0.25
        }
    }

    fn region_err(i: usize) -> f64 {
        if i == REGION_NAN_ERR_ROW {
            f64::NAN
        } else {
            0.02 + i as f64 * 0.001
        }
    }

    fn region_export(view: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("spectrum1d")
            .join(format!("region_{view}_export.csv"));
        std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
    }

    fn assert_region_column(what: &str, actual: &[f64], expected: impl Fn(usize) -> f64) {
        assert_eq!(actual.len(), REGION_ROWS, "{what}");
        for (i, &value) in actual.iter().enumerate() {
            let want = expected(i);
            if want.is_nan() {
                assert!(value.is_nan(), "{what}[{i}] = {value}, expected NaN");
            } else {
                assert!(
                    ((value - want) / want).abs() <= CSV_ROUND_TRIP_TOLERANCE,
                    "{what}[{i}] = {value}, expected {want}"
                );
            }
        }
    }

    #[test]
    fn from_csv_pairs_the_flux_of_each_region_view_export_with_the_error_of_that_view() {
        let views: [(&str, &str, fn(usize) -> f64, fn(usize) -> f64); 3] = [
            ("sum", "MJy/sr x pix", region_sum, region_err),
            (
                "mean",
                "MJy/sr",
                |i| region_sum(i) / REGION_WEIGHT,
                |i| region_err(i) / REGION_WEIGHT,
            ),
            (
                "jy",
                "Jy",
                |i| region_sum(i) * REGION_JY_PER_SUM,
                |i| region_err(i) * REGION_JY_PER_SUM,
            ),
        ];
        for (view, unit, flux, err) in views {
            let s = Spectrum1d::from_csv(CSV_PATH, &region_export(view)).unwrap();
            assert_region_column(&format!("{view} wavelength"), &s.axis.values, |i| {
                2.0 + i as f64 * 0.001
            });
            assert_region_column(&format!("{view} flux"), &s.flux, flux);
            assert_region_column(&format!("{view} err"), s.err.as_deref().unwrap(), err);
            assert_eq!(s.flux_unit.as_deref(), Some(unit), "{view}");
            assert_eq!(s.cards.get("view").map(String::as_str), Some(view));
            assert_eq!(
                s.cards.get("err_source").map(String::as_str),
                Some("ERR HDU 2")
            );
        }
    }

    #[test]
    fn from_csv_reads_the_native_unit_placeholder_of_the_export_as_no_unit() {
        let text = "# flux_unit: native\nwavelength_vacuum_um,flux,flux_unit\n2.0,1,native\n2.1,2,native\n";
        let s = Spectrum1d::from_csv(CSV_PATH, text).unwrap();
        assert!(s.flux_unit.is_none(), "{:?}", s.flux_unit);
        assert!(
            s.notes.iter().any(|n| n.contains("native units")),
            "{:?}",
            s.notes
        );
        let s = Spectrum1d::from_csv(
            CSV_PATH,
            "wavelength_vacuum_um,flux,flux_unit\n2.0,1,Native\n",
        )
        .unwrap();
        assert!(s.flux_unit.is_none(), "{:?}", s.flux_unit);
    }

    #[test]
    fn from_csv_keeps_the_f32_widened_precision_its_provenance_line_names() {
        let text =
            "# precision: f32_widened\nwavelength_vacuum_um,flux\n2.0,0.012300000339746475\n";
        let s = Spectrum1d::from_csv(CSV_PATH, text).unwrap();
        assert_eq!(s.precision, Precision::F32Widened);
        assert!(
            s.notes.iter().any(|n| n.contains("f32 precision")),
            "{:?}",
            s.notes
        );
        let s = Spectrum1d::from_csv(
            CSV_PATH,
            "# precision: f64\nwavelength_vacuum_um,flux\n2.0,1\n",
        )
        .unwrap();
        assert_eq!(s.precision, Precision::F64);
        assert!(
            !s.notes.iter().any(|n| n.contains("f32 precision")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn from_csv_refuses_a_csv_it_cannot_read_naming_the_line_and_the_column() {
        let refusals: [(&str, &[&str]); 9] = [
            ("# only provenance\n", &["no header row"]),
            (
                "channel,flux\n0,1\n",
                &["wavelength_vacuum_um", "wavelength_air_um"],
            ),
            ("channel,wavelength_vacuum_um\n0,1\n", &["flux column"]),
            ("wavelength_vacuum_um,flux\n", &["no data rows"]),
            (
                "wavelength_vacuum_um,flux\n1.0,2.0,3.0\n",
                &["line 2", "3 fields", "header has 2"],
            ),
            (
                "wavelength_vacuum_um,flux\n1.0,abc\n",
                &["line 2", "flux", "'abc'"],
            ),
            (
                "wavelength_vacuum_um,flux,dq\n1.0,2.0,-1\n",
                &["line 2", "dq", "'-1'"],
            ),
            (
                "wavelength_vacuum_um,flux,flux_unit\n1.0,2.0,\"Jy\n",
                &["line 2", "unterminated"],
            ),
            (
                "# dq_table: kepler\nwavelength_vacuum_um,flux,dq\n1.0,2.0,0\n",
                &["dq_table 'kepler'"],
            ),
        ];
        for (text, expected) in refusals {
            let err = Spectrum1d::from_csv(CSV_PATH, text).unwrap_err();
            for part in expected {
                assert!(err.contains(part), "{text:?}: {err}");
            }
        }
    }

    #[test]
    fn serialised_csv_source_is_tagged_csv_with_its_path() {
        let s = Spectrum1d::from_csv(CSV_PATH, "wavelength_vacuum_um,flux\n2.0,1\n").unwrap();
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json["source"],
            serde_json::json!({ "kind": "csv", "path": CSV_PATH })
        );
        assert_eq!(json["wavelength_frame"], "vacuum");
    }

    #[test]
    fn serialised_document_uses_the_tagged_shapes_the_frontend_reads() {
        let s = open_owned(&iraf_cards("angstroms")).unwrap();
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["source"]["kind"], "wcs1d");
        assert_eq!(json["source"]["path"], PATH);
        assert_eq!(json["source"]["hdu"], 0);
        assert_eq!(json["wavelength_frame"], "air");
        assert_eq!(json["precision"], "f64");
        assert_eq!(json["axis"]["kind"], "awav");
        assert_eq!(json["dq_table"], "unknown");
        assert!(json["err"].is_null() && json["dq"].is_null() && json["flux_unit"].is_null());
        assert_eq!(json["cards"]["WAT0_001"], "system=equispec");
        assert_eq!(
            serde_json::to_value(Precision::F32Widened).unwrap(),
            "f32_widened"
        );
        assert_eq!(
            serde_json::to_value(WavelengthFrame::NotWavelength).unwrap(),
            "not_wavelength"
        );
    }
}
