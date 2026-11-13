use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_2;

use serde::{Deserialize, Serialize};

use crate::core::astrometry::wcs::angular_separation;
use crate::core::metadata::photcal::{AB_MAG_ZERO_POINT, MAG_ERR_PER_RELATIVE_FLUX_ERR};
use crate::infra::tables::{TableCells, TableColumn, TextTable};
use crate::math::exact_median_f64;
use crate::types::constants::MAD_TO_SIGMA;

pub const NANOMAGGY_ZERO_POINT_AB: f64 = 22.5;
pub const MAX_COMPARE_POSITIONS: usize = 5000;
pub const FEW_ROWS_BELOW: usize = 5;
pub const EXACT_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogueFluxUnit {
    #[serde(rename = "nanomaggy")]
    Nanomaggy,
    #[serde(rename = "Jy")]
    Jy,
    #[serde(rename = "mJy")]
    MilliJy,
    #[serde(rename = "uJy")]
    MicroJy,
    #[serde(rename = "nJy")]
    NanoJy,
    #[serde(rename = "zero_point")]
    ZeroPoint,
    #[serde(rename = "ab_mag")]
    AbMag,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareMapping {
    #[serde(default)]
    pub id: Option<String>,
    pub ra: String,
    pub dec: String,
    pub flux: String,
    pub flux_unit: CatalogueFluxUnit,
    #[serde(default)]
    pub zero_point: Option<f64>,
    #[serde(default)]
    pub flux_err: Option<String>,
    #[serde(default)]
    pub flux_ivar: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub missing_value: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CutOp {
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = ">")]
    Gt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CutValue {
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareCut {
    pub column: String,
    pub op: CutOp,
    pub value: CutValue,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareAperture {
    #[serde(default)]
    pub radius_arcsec: Option<f64>,
    #[serde(default)]
    pub radius_px: Option<f64>,
    #[serde(default)]
    pub annulus_arcsec: Option<[f64; 2]>,
    #[serde(default)]
    pub annulus_px: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompareSelection {
    pub mapping: CompareMapping,
    pub cuts: Vec<CompareCut>,
    pub min_snr: Option<f64>,
    pub max_snr: Option<f64>,
    pub min_neighbour_arcsec: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rejection {
    pub stage: u8,
    pub order: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueEntry {
    pub row: usize,
    pub id: String,
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub class: Option<String>,
    pub flux_cat_jy: Option<f64>,
    pub flux_err_cat_jy: Option<f64>,
    pub snr_cat: Option<f64>,
    pub rejection: Option<Rejection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueSide {
    pub entries: Vec<CatalogueEntry>,
    pub n_table: usize,
    pub n_no_position: usize,
    pub n_off_image: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    pub x: f64,
    pub y: f64,
    pub on_image: bool,
    pub inside: bool,
    pub error: Option<String>,
    pub flux_native: Option<f64>,
    pub flux_err_native: Option<f64>,
    pub flux_jy: Option<f64>,
    pub flux_err_jy: Option<f64>,
    pub masked_fraction: Option<f64>,
    pub saturated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RobustStat {
    pub median: f64,
    pub nmad: f64,
    pub median_err: Option<f64>,
    pub n: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompareRow {
    pub row: usize,
    pub id: String,
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub class: Option<String>,
    pub flux_cat_jy: Option<f64>,
    pub flux_err_cat_jy: Option<f64>,
    pub snr_cat: Option<f64>,
    pub flux_native: Option<f64>,
    pub flux_err_native: Option<f64>,
    pub flux_jy: Option<f64>,
    pub flux_err_jy: Option<f64>,
    pub ratio: Option<f64>,
    pub masked_fraction: Option<f64>,
    pub used: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RejectedCount {
    pub reason: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompareSummary {
    pub n_table: usize,
    pub n_on_image: usize,
    pub n_after_cuts: usize,
    pub n_measured: usize,
    pub n_used: usize,
    pub ratio: Option<RobustStat>,
    pub ratio_offset_mag: Option<f64>,
    pub err_ratio: Option<RobustStat>,
    pub implied_zero_point_ab: Option<RobustStat>,
    pub rejected: Vec<RejectedCount>,
    pub rows: Vec<CompareRow>,
    pub warnings: Vec<String>,
}

const ARCSEC_PER_DEGREE: f64 = 3600.0;
const MAX_LISTED_COLUMNS: usize = 40;
const STAGE_NO_POSITION: u8 = 0;
const STAGE_OFF_IMAGE: u8 = 1;
const STAGE_MISSING_VALUE: u8 = 2;
const STAGE_CUT: u8 = 3;
const STAGE_SNR_BELOW: u8 = 4;
const STAGE_SNR_ABOVE: u8 = 5;
const STAGE_NEIGHBOUR: u8 = 6;
const STAGE_NOT_POSITIVE: u8 = 7;
const STAGE_EDGE: u8 = 8;
const STAGE_NOT_MEASURED: u8 = 9;
const STAGE_NO_DATA: u8 = 10;
const STAGE_SATURATED: u8 = 11;
const REASON_NO_POSITION: &str = "no position";
const REASON_OFF_IMAGE: &str = "off the image";
const REASON_NOT_POSITIVE: &str = "catalogue flux is not positive";
const REASON_EDGE: &str = "aperture crosses the image edge";
const REASON_NO_FINITE_FLUX: &str = "not measured: no finite flux";
const REASON_NO_DATA: &str = "aperture touches pixels without data";
const REASON_SATURATED: &str = "saturated in the image";
const WARNING_NO_ROW: &str = "no row is left to compare: see the rejected rows";
const WARNING_NO_JY_CALIBRATION: &str = "the image has no flux calibration in Jy: the ratio is not available; the zero point implied by the catalogue is given instead";

impl CutOp {
    fn symbol(self) -> &'static str {
        match self {
            CutOp::Lt => "<",
            CutOp::Le => "<=",
            CutOp::Eq => "==",
            CutOp::Ne => "!=",
            CutOp::Ge => ">=",
            CutOp::Gt => ">",
        }
    }

    fn holds(self, cell: f64, wanted: f64) -> bool {
        match self {
            CutOp::Lt => cell < wanted,
            CutOp::Le => cell <= wanted,
            CutOp::Eq => cell == wanted,
            CutOp::Ne => cell != wanted,
            CutOp::Ge => cell >= wanted,
            CutOp::Gt => cell > wanted,
        }
    }
}

fn jy_at_ab_magnitude(magnitude: f64) -> f64 {
    10f64.powf((AB_MAG_ZERO_POINT - magnitude) / 2.5)
}

fn jy_per_linear_unit(unit: CatalogueFluxUnit, zero_point: Option<f64>) -> Option<f64> {
    match unit {
        CatalogueFluxUnit::Nanomaggy => Some(jy_at_ab_magnitude(NANOMAGGY_ZERO_POINT_AB)),
        CatalogueFluxUnit::Jy => Some(1.0),
        CatalogueFluxUnit::MilliJy => Some(1e-3),
        CatalogueFluxUnit::MicroJy => Some(1e-6),
        CatalogueFluxUnit::NanoJy => Some(1e-9),
        CatalogueFluxUnit::ZeroPoint => zero_point.filter(|zp| zp.is_finite()).map(jy_at_ab_magnitude),
        CatalogueFluxUnit::AbMag => None,
    }
}

fn above_zero(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

pub fn catalogue_flux_jy(value: f64, unit: CatalogueFluxUnit, zero_point: Option<f64>) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    match unit {
        CatalogueFluxUnit::AbMag => Some(jy_at_ab_magnitude(value)),
        _ => jy_per_linear_unit(unit, zero_point).map(|factor| value * factor),
    }
}

pub fn catalogue_flux_err_jy(
    flux: f64,
    err: Option<f64>,
    ivar: Option<f64>,
    unit: CatalogueFluxUnit,
    zero_point: Option<f64>,
) -> Option<f64> {
    if let Some(err) = err {
        let err = above_zero(err)?;
        return match unit {
            CatalogueFluxUnit::AbMag => catalogue_flux_jy(flux, unit, zero_point)
                .map(|flux_jy| flux_jy * err / MAG_ERR_PER_RELATIVE_FLUX_ERR),
            _ => jy_per_linear_unit(unit, zero_point).map(|factor| err * factor),
        };
    }
    let ivar = above_zero(ivar?)?;
    jy_per_linear_unit(unit, zero_point).map(|factor| factor / ivar.sqrt())
}

fn catalogue_snr(flux: f64, err: Option<f64>, ivar: Option<f64>, unit: CatalogueFluxUnit) -> Option<f64> {
    if let Some(err) = err {
        let err = above_zero(err)?;
        return Some(match unit {
            CatalogueFluxUnit::AbMag => MAG_ERR_PER_RELATIVE_FLUX_ERR / err,
            _ => flux / err,
        });
    }
    above_zero(ivar?).map(|ivar| flux * ivar.sqrt())
}

pub fn cut_text(cut: &CompareCut) -> String {
    let (column, op) = (&cut.column, cut.op.symbol());
    match &cut.value {
        CutValue::Number(number) => format!("cut {column} {op} {number}"),
        CutValue::Text(text) => format!("cut {column} {op} {}", text.trim()),
    }
}

fn column_named<'a>(table: &'a TextTable, name: &str) -> Result<&'a TableColumn, String> {
    table.column(name).ok_or_else(|| {
        let names: Vec<&str> = table.columns.iter().map(|column| column.name.as_str()).collect();
        let listed = names[..names.len().min(MAX_LISTED_COLUMNS)].join(", ");
        match names.len().saturating_sub(MAX_LISTED_COLUMNS) {
            0 => format!("column '{name}' is not in the table; it has: {listed}"),
            more => format!("column '{name}' is not in the table; it has: {listed} and {more} more"),
        }
    })
}

fn numeric_cells<'a>(table: &'a TextTable, name: &str, role: &str) -> Result<&'a [f64], String> {
    match &column_named(table, name)?.cells {
        TableCells::F64(cells) => Ok(cells),
        TableCells::Text(_) => Err(format!("column '{name}' is text; {role} needs a numeric column")),
    }
}

struct MappedColumns<'a> {
    id: Option<&'a TableColumn>,
    ra: &'a [f64],
    dec: &'a [f64],
    flux: &'a [f64],
    flux_err: Option<&'a [f64]>,
    flux_ivar: Option<&'a [f64]>,
    class: Option<&'a TableColumn>,
}

fn mapped_columns<'a>(table: &'a TextTable, mapping: &CompareMapping) -> Result<MappedColumns<'a>, String> {
    let any_column = |name: &Option<String>| name.as_deref().map(|name| column_named(table, name)).transpose();
    let numeric = |name: &Option<String>, role: &str| {
        name.as_deref()
            .map(|name| numeric_cells(table, name, role))
            .transpose()
    };
    let columns = MappedColumns {
        id: any_column(&mapping.id)?,
        ra: numeric_cells(table, &mapping.ra, "ra")?,
        dec: numeric_cells(table, &mapping.dec, "dec")?,
        flux: numeric_cells(table, &mapping.flux, "flux")?,
        flux_err: numeric(&mapping.flux_err, "flux_err")?,
        flux_ivar: numeric(&mapping.flux_ivar, "flux_ivar")?,
        class: any_column(&mapping.class)?,
    };
    if columns.flux_err.is_some() && columns.flux_ivar.is_some() {
        return Err("map flux_err or flux_ivar, not both".to_string());
    }
    if mapping.flux_unit == CatalogueFluxUnit::ZeroPoint && !mapping.zero_point.is_some_and(f64::is_finite) {
        return Err("flux_unit zero_point needs a finite zero_point".to_string());
    }
    if mapping.flux_unit == CatalogueFluxUnit::AbMag && columns.flux_ivar.is_some() {
        return Err(
            "flux_ivar cannot be used with flux_unit ab_mag; map the magnitude error as flux_err".to_string(),
        );
    }
    Ok(columns)
}

enum ResolvedCut<'a> {
    Number {
        cells: &'a [f64],
        op: CutOp,
        wanted: f64,
    },
    Text {
        cells: &'a [String],
        wanted: String,
        equal: bool,
    },
}

impl ResolvedCut<'_> {
    fn keeps(&self, row: usize) -> bool {
        match self {
            ResolvedCut::Number { cells, op, wanted } => !cells[row].is_nan() && op.holds(cells[row], *wanted),
            ResolvedCut::Text { cells, wanted, equal } => {
                let cell = cells[row].trim();
                !cell.is_empty() && cell.eq_ignore_ascii_case(wanted) == *equal
            }
        }
    }
}

fn resolved_cut<'a>(table: &'a TextTable, cut: &CompareCut) -> Result<ResolvedCut<'a>, String> {
    let (column, op) = (&cut.column, cut.op);
    match (&column_named(table, column)?.cells, &cut.value) {
        (TableCells::F64(cells), CutValue::Number(wanted)) => Ok(ResolvedCut::Number {
            cells,
            op,
            wanted: *wanted,
        }),
        (TableCells::F64(cells), CutValue::Text(text)) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|wanted| !wanted.is_nan())
            .map(|wanted| ResolvedCut::Number { cells, op, wanted })
            .ok_or_else(|| {
                format!(
                    "cut on '{column}': the column is numeric and '{}' is not a number",
                    text.trim()
                )
            }),
        (TableCells::Text(_), _) if !matches!(op, CutOp::Eq | CutOp::Ne) => Err(format!(
            "cut on '{column}': {} needs a numeric column",
            op.symbol()
        )),
        (TableCells::Text(_), CutValue::Number(_)) => {
            Err(format!("cut on '{column}': the column is text; give a text value"))
        }
        (TableCells::Text(cells), CutValue::Text(text)) => Ok(ResolvedCut::Text {
            cells,
            wanted: text.trim().to_string(),
            equal: op == CutOp::Eq,
        }),
    }
}

fn limit_text(limit: Option<f64>) -> String {
    limit.map_or_else(|| "none".to_string(), |value| value.to_string())
}

fn resolved_selection<'a>(
    table: &'a TextTable,
    selection: &CompareSelection,
) -> Result<(MappedColumns<'a>, Vec<ResolvedCut<'a>>), String> {
    let columns = mapped_columns(table, &selection.mapping)?;
    let (min_snr, max_snr) = (selection.min_snr, selection.max_snr);
    if min_snr.is_some() || max_snr.is_some() {
        if columns.flux_err.is_none() && columns.flux_ivar.is_none() {
            return Err("min_snr and max_snr need a catalogue error column (flux_err or flux_ivar)".to_string());
        }
        let all_finite = [min_snr, max_snr].into_iter().flatten().all(f64::is_finite);
        let reversed = min_snr.zip(max_snr).is_some_and(|(min, max)| min > max);
        if !all_finite || reversed {
            return Err(format!(
                "min_snr {} and max_snr {} must be finite and in order",
                limit_text(min_snr),
                limit_text(max_snr)
            ));
        }
    }
    if let Some(limit) = selection.min_neighbour_arcsec {
        if above_zero(limit).is_none() {
            return Err(format!("min_neighbour_arcsec {limit} must be a positive number"));
        }
    }
    let cuts = selection
        .cuts
        .iter()
        .map(|cut| resolved_cut(table, cut))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((columns, cuts))
}

pub fn validate_selection(table: &TextTable, selection: &CompareSelection) -> Result<(), String> {
    resolved_selection(table, selection).map(|_| ())
}

fn sky_positions(ra: &[f64], dec: &[f64], missing_value: Option<f64>) -> Vec<Option<(f64, f64)>> {
    ra.iter()
        .zip(dec)
        .map(|(&ra, &dec)| {
            let on_the_sky = (0.0..=360.0).contains(&ra) && (-90.0..=90.0).contains(&dec);
            let given = missing_value != Some(ra) && missing_value != Some(dec);
            (on_the_sky && given).then_some((ra, dec))
        })
        .collect()
}

pub fn catalogue_positions(
    table: &TextTable,
    mapping: &CompareMapping,
) -> Result<Vec<Option<(f64, f64)>>, String> {
    let ra = numeric_cells(table, &mapping.ra, "ra")?;
    let dec = numeric_cells(table, &mapping.dec, "dec")?;
    Ok(sky_positions(ra, dec, mapping.missing_value))
}

fn rows_with_a_close_neighbour(positions: &[Option<(f64, f64)>], limit_arcsec: f64) -> Vec<bool> {
    let mut by_dec: Vec<(usize, f64, f64)> = positions
        .iter()
        .enumerate()
        .filter_map(|(row, position)| position.map(|(ra, dec)| (row, ra, dec)))
        .collect();
    by_dec.sort_by(|a, b| a.2.total_cmp(&b.2));
    let limit_deg = limit_arcsec / ARCSEC_PER_DEGREE;
    let mut close = vec![false; positions.len()];
    for (first, &(row, ra, dec)) in by_dec.iter().enumerate() {
        for &(other, other_ra, other_dec) in &by_dec[first + 1..] {
            if other_dec > dec + limit_deg {
                break;
            }
            if angular_separation(ra, dec, other_ra, other_dec) * ARCSEC_PER_DEGREE < limit_arcsec {
                close[row] = true;
                close[other] = true;
            }
        }
    }
    close
}

fn holds_exact_ids(column: &TableColumn) -> bool {
    match &column.cells {
        TableCells::F64(cells) => !cells
            .iter()
            .any(|value| value.is_finite() && value.abs() >= EXACT_INTEGER_LIMIT),
        TableCells::Text(_) => true,
    }
}

fn cell_text(column: &TableColumn, row: usize) -> Option<String> {
    match &column.cells {
        TableCells::F64(cells) => cells[row].is_finite().then(|| cells[row].to_string()),
        TableCells::Text(cells) => Some(cells[row].trim())
            .filter(|text| !text.is_empty())
            .map(str::to_string),
    }
}

fn rejection_at(stage: u8, order: usize, reason: impl Into<String>) -> Rejection {
    Rejection {
        stage,
        order,
        reason: reason.into(),
    }
}

pub fn evaluate_catalogue(
    table: &TextTable,
    selection: &CompareSelection,
    on_image: &[bool],
) -> Result<CatalogueSide, String> {
    let (columns, cuts) = resolved_selection(table, selection)?;
    if on_image.len() != table.n_rows {
        return Err(format!(
            "on_image has {} flags for {} rows",
            on_image.len(),
            table.n_rows
        ));
    }
    let mapping = &selection.mapping;
    let (unit, zero_point, missing_value) = (mapping.flux_unit, mapping.zero_point, mapping.missing_value);
    let positions = sky_positions(columns.ra, columns.dec, missing_value);
    let crowded = selection
        .min_neighbour_arcsec
        .map(|limit| (limit, rows_with_a_close_neighbour(&positions, limit)));
    let snr_is_limited = selection.min_snr.is_some() || selection.max_snr.is_some();
    let mut warnings = Vec::new();
    let id_column = columns.id.filter(|column| holds_exact_ids(column));
    if let (Some(name), None) = (&mapping.id, id_column) {
        warnings.push(format!(
            "column '{name}' holds integers above 2^53 that arrived as floating point: rows are named by their row number"
        ));
    }
    let flagged_missing = |name: &str, cell: f64| {
        (missing_value == Some(cell))
            .then(|| rejection_at(STAGE_MISSING_VALUE, 0, format!("missing value ({cell}) in {name}")))
    };
    let not_finite = |name: &str, cell: f64| {
        (!cell.is_finite()).then(|| rejection_at(STAGE_MISSING_VALUE, 0, format!("missing value in {name}")))
    };
    let missing_error = |name: &Option<String>, cell: Option<f64>| {
        name.as_deref().zip(cell).and_then(|(name, cell)| {
            flagged_missing(name, cell).or_else(|| not_finite(name, cell).filter(|_| snr_is_limited))
        })
    };

    let mut entries = Vec::new();
    let (mut n_no_position, mut n_off_image) = (0, 0);
    for row in 0..table.n_rows {
        let Some((ra_deg, dec_deg)) = positions[row] else {
            n_no_position += 1;
            continue;
        };
        if !on_image[row] {
            n_off_image += 1;
            continue;
        }
        let flux = columns.flux[row];
        let err_cell = columns.flux_err.map(|cells| cells[row]);
        let ivar_cell = columns.flux_ivar.map(|cells| cells[row]);
        let missing_flux = flagged_missing(&mapping.flux, flux).or_else(|| not_finite(&mapping.flux, flux));
        let flux_is_given = missing_flux.is_none();
        let err = err_cell.filter(|cell| missing_value != Some(*cell));
        let ivar = ivar_cell.filter(|cell| missing_value != Some(*cell));
        let snr_cat = flux_is_given
            .then(|| catalogue_snr(flux, err, ivar, unit))
            .flatten();
        let rejection = missing_flux
            .or_else(|| missing_error(&mapping.flux_err, err_cell))
            .or_else(|| missing_error(&mapping.flux_ivar, ivar_cell))
            .or_else(|| {
                cuts.iter()
                    .position(|cut| !cut.keeps(row))
                    .map(|order| rejection_at(STAGE_CUT, order, cut_text(&selection.cuts[order])))
            })
            .or_else(|| {
                selection
                    .min_snr
                    .filter(|min| !snr_cat.is_some_and(|snr| snr >= *min))
                    .map(|min| rejection_at(STAGE_SNR_BELOW, 0, format!("catalogue signal-to-noise below {min}")))
            })
            .or_else(|| {
                selection
                    .max_snr
                    .filter(|max| !snr_cat.is_some_and(|snr| snr <= *max))
                    .map(|max| rejection_at(STAGE_SNR_ABOVE, 0, format!("catalogue signal-to-noise above {max}")))
            })
            .or_else(|| {
                crowded
                    .as_ref()
                    .filter(|(_, close)| close[row])
                    .map(|(limit, _)| rejection_at(STAGE_NEIGHBOUR, 0, format!("neighbour within {limit} arcsec")))
            })
            .or_else(|| {
                (unit != CatalogueFluxUnit::AbMag && flux <= 0.0)
                    .then(|| rejection_at(STAGE_NOT_POSITIVE, 0, REASON_NOT_POSITIVE))
            });
        entries.push(CatalogueEntry {
            row: row + 1,
            id: id_column
                .and_then(|column| cell_text(column, row))
                .unwrap_or_else(|| format!("row {}", row + 1)),
            ra_deg,
            dec_deg,
            class: columns.class.and_then(|column| cell_text(column, row)),
            flux_cat_jy: flux_is_given
                .then(|| catalogue_flux_jy(flux, unit, zero_point))
                .flatten(),
            flux_err_cat_jy: flux_is_given
                .then(|| catalogue_flux_err_jy(flux, err, ivar, unit, zero_point))
                .flatten(),
            snr_cat,
            rejection,
        });
    }

    let n_to_measure = entries.iter().filter(|entry| entry.rejection.is_none()).count();
    if n_to_measure > MAX_COMPARE_POSITIONS {
        return Err(format!(
            "{n_to_measure} catalogue rows are left to measure; the comparison measures at most {MAX_COMPARE_POSITIONS}: cut the table or add cuts"
        ));
    }
    Ok(CatalogueSide {
        entries,
        n_table: table.n_rows,
        n_no_position,
        n_off_image,
        warnings,
    })
}

pub fn aperture_inside(x: f64, y: f64, reach_px: f64, cols: usize, rows: usize) -> bool {
    x.is_finite()
        && y.is_finite()
        && reach_px.is_finite()
        && x - reach_px >= -0.5
        && x + reach_px <= cols as f64 - 0.5
        && y - reach_px >= -0.5
        && y + reach_px <= rows as f64 - 0.5
}

pub fn robust_stat(values: &[f64]) -> Option<RobustStat> {
    let finite: Vec<f64> = values.iter().copied().filter(|value| value.is_finite()).collect();
    if finite.is_empty() {
        return None;
    }
    let n = finite.len();
    let median = exact_median_f64(&finite);
    let deviations: Vec<f64> = finite.iter().map(|value| (value - median).abs()).collect();
    let nmad = MAD_TO_SIGMA * exact_median_f64(&deviations);
    Some(RobustStat {
        median,
        nmad,
        median_err: (n >= 2).then(|| FRAC_PI_2.sqrt() * nmad / (n as f64).sqrt()),
        n,
    })
}

fn measurement_rejection(measurement: Option<&Measured>) -> Option<Rejection> {
    let Some(measured) = measurement else {
        return Some(rejection_at(STAGE_NOT_MEASURED, 0, REASON_NO_FINITE_FLUX));
    };
    if !measured.on_image {
        Some(rejection_at(STAGE_OFF_IMAGE, 0, REASON_OFF_IMAGE))
    } else if !measured.inside {
        Some(rejection_at(STAGE_EDGE, 0, REASON_EDGE))
    } else if let Some(error) = &measured.error {
        Some(rejection_at(STAGE_NOT_MEASURED, 0, format!("not measured: {error}")))
    } else if !measured.flux_native.is_some_and(f64::is_finite) {
        Some(rejection_at(STAGE_NOT_MEASURED, 0, REASON_NO_FINITE_FLUX))
    } else if measured.masked_fraction.is_some_and(|fraction| fraction > 0.0) {
        Some(rejection_at(STAGE_NO_DATA, 0, REASON_NO_DATA))
    } else if measured.saturated {
        Some(rejection_at(STAGE_SATURATED, 0, REASON_SATURATED))
    } else {
        None
    }
}

fn few_rows_warning(n_used: usize) -> String {
    let subject = if n_used == 1 {
        "1 row is".to_string()
    } else {
        format!("{n_used} rows are")
    };
    format!("only {subject} used: the median of fewer than {FEW_ROWS_BELOW} rows is indicative")
}

pub fn summarise(side: CatalogueSide, measured: &[Option<Measured>]) -> Result<CompareSummary, String> {
    if measured.len() != side.entries.len() {
        return Err(format!(
            "{} measurements for {} rows",
            measured.len(),
            side.entries.len()
        ));
    }
    let n_on_image = side.entries.len();
    let mut tally: BTreeMap<(u8, usize, String), usize> = BTreeMap::new();
    for (stage, reason, count) in [
        (STAGE_NO_POSITION, REASON_NO_POSITION, side.n_no_position),
        (STAGE_OFF_IMAGE, REASON_OFF_IMAGE, side.n_off_image),
    ] {
        if count > 0 {
            tally.insert((stage, 0, reason.to_string()), count);
        }
    }
    let (mut n_after_cuts, mut n_measured) = (0, 0);
    let mut rows = Vec::with_capacity(n_on_image);
    for (entry, measurement) in side.entries.into_iter().zip(measured) {
        let measurement = measurement.as_ref();
        let rejection = match entry.rejection {
            Some(cut_out) => Some(cut_out),
            None => {
                n_after_cuts += 1;
                let stopped = measurement_rejection(measurement);
                let was_measured = !stopped.as_ref().is_some_and(|rejection| {
                    matches!(rejection.stage, STAGE_OFF_IMAGE | STAGE_EDGE | STAGE_NOT_MEASURED)
                });
                if was_measured {
                    n_measured += 1;
                }
                stopped
            }
        };
        if let Some(Rejection { stage, order, reason }) = &rejection {
            *tally.entry((*stage, *order, reason.clone())).or_insert(0) += 1;
        }
        let flux_jy = measurement.and_then(|measured| measured.flux_jy);
        rows.push(CompareRow {
            row: entry.row,
            id: entry.id,
            ra_deg: entry.ra_deg,
            dec_deg: entry.dec_deg,
            x: measurement.map(|measured| measured.x),
            y: measurement.map(|measured| measured.y),
            class: entry.class,
            flux_cat_jy: entry.flux_cat_jy,
            flux_err_cat_jy: entry.flux_err_cat_jy,
            snr_cat: entry.snr_cat,
            flux_native: measurement.and_then(|measured| measured.flux_native),
            flux_err_native: measurement.and_then(|measured| measured.flux_err_native),
            flux_jy,
            flux_err_jy: measurement.and_then(|measured| measured.flux_err_jy),
            ratio: flux_jy.zip(entry.flux_cat_jy).map(|(flux, catalogue)| flux / catalogue),
            masked_fraction: measurement.and_then(|measured| measured.masked_fraction),
            used: rejection.is_none(),
            reason: rejection.map(|rejection| rejection.reason),
        });
    }

    let used: Vec<&CompareRow> = rows.iter().filter(|row| row.used).collect();
    let n_used = used.len();
    let ratios: Vec<f64> = used.iter().filter_map(|row| row.ratio).collect();
    let err_ratios: Vec<f64> = used
        .iter()
        .filter_map(|row| row.flux_err_jy.zip(row.flux_err_cat_jy))
        .filter(|(measured, catalogue)| *measured > 0.0 && *catalogue > 0.0)
        .map(|(measured, catalogue)| measured / catalogue)
        .collect();
    let implied_zero_points: Vec<f64> = used
        .iter()
        .filter_map(|row| row.flux_native.zip(row.flux_cat_jy))
        .filter(|(native, catalogue)| *native > 0.0 && *catalogue > 0.0)
        .map(|(native, catalogue)| 2.5 * (native / catalogue).log10() + AB_MAG_ZERO_POINT)
        .collect();
    let ratio = robust_stat(&ratios);

    let mut warnings = side.warnings;
    if n_used == 0 {
        warnings.push(WARNING_NO_ROW.to_string());
    } else if n_used < FEW_ROWS_BELOW {
        warnings.push(few_rows_warning(n_used));
    }
    if used.iter().any(|row| row.flux_native.is_some()) && !used.iter().any(|row| row.flux_jy.is_some()) {
        warnings.push(WARNING_NO_JY_CALIBRATION.to_string());
    }

    Ok(CompareSummary {
        n_table: side.n_table,
        n_on_image,
        n_after_cuts,
        n_measured,
        n_used,
        ratio,
        ratio_offset_mag: ratio
            .filter(|stat| stat.median > 0.0)
            .map(|stat| -2.5 * stat.median.log10()),
        err_ratio: robust_stat(&err_ratios),
        implied_zero_point_ab: robust_stat(&implied_zero_points),
        rejected: tally
            .into_iter()
            .map(|((_, _, reason), count)| RejectedCount { reason, count })
            .collect(),
        rows,
        warnings,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedPreset {
    pub id: String,
    pub label: String,
    pub band: String,
    pub mapping: CompareMapping,
    pub aperture: CompareAperture,
    pub cuts: Vec<CompareCut>,
    pub min_snr: Option<f64>,
    pub max_snr: Option<f64>,
    pub min_neighbour_arcsec: Option<f64>,
    pub notes: Vec<String>,
}

const BAND_MARK: &str = "<b>";

enum PresetCutValue {
    Number(f64),
    Text(&'static str),
}

struct PresetCut {
    column: &'static str,
    op: CutOp,
    value: PresetCutValue,
}

struct SurveyPreset {
    id: &'static str,
    label: &'static str,
    bands: &'static [&'static str],
    flux: &'static str,
    flux_unit: CatalogueFluxUnit,
    zero_point: Option<f64>,
    flux_err: Option<&'static str>,
    flux_ivar: Option<&'static str>,
    id_column: &'static str,
    ra: &'static str,
    dec: &'static str,
    class: Option<&'static str>,
    aperture: CompareAperture,
    cuts: &'static [PresetCut],
    min_snr: Option<f64>,
    missing_value: Option<f64>,
    min_neighbour_arcsec: Option<f64>,
    notes: &'static [&'static str],
}

static SURVEY_PRESETS: [SurveyPreset; 5] = [
    SurveyPreset {
        id: "legacy_dr10",
        label: "Legacy Surveys DR10",
        bands: &["g", "r", "i", "z"],
        flux: "apflux_<b>_5",
        flux_unit: CatalogueFluxUnit::Nanomaggy,
        zero_point: None,
        flux_err: None,
        flux_ivar: Some("apflux_ivar_<b>_5"),
        id_column: "ls_id",
        ra: "ra",
        dec: "dec",
        class: Some("type"),
        aperture: CompareAperture { radius_arcsec: Some(2.0), radius_px: None, annulus_arcsec: None, annulus_px: None },
        cuts: &[
            PresetCut { column: "type", op: CutOp::Eq, value: PresetCutValue::Text("PSF") },
            PresetCut { column: "maskbits", op: CutOp::Eq, value: PresetCutValue::Number(0.0) },
            PresetCut { column: "fracflux_<b>", op: CutOp::Lt, value: PresetCutValue::Number(0.01) },
        ],
        min_snr: None,
        missing_value: None,
        min_neighbour_arcsec: None,
        notes: &[
            "apflux_<b>_5: circular aperture of 2.0 arcsec radius, a plain sum with no local sky",
            "apflux_<b>_1 to apflux_<b>_8 are radii of 0.5, 0.75, 1.0, 1.5, 2.0, 3.5, 5.0 and 7.0 arcsec",
            "the catalogue leaves pixels of zero inverse variance out of the sum",
        ],
    },
    SurveyPreset {
        id: "des_dr2",
        label: "DES DR2",
        bands: &["g", "r", "i", "z", "y"],
        flux: "flux_aper_8_<b>",
        flux_unit: CatalogueFluxUnit::ZeroPoint,
        zero_point: Some(30.0),
        flux_err: Some("fluxerr_aper_8_<b>"),
        flux_ivar: None,
        id_column: "coadd_object_id",
        ra: "alphawin_j2000",
        dec: "deltawin_j2000",
        class: Some("extended_class_coadd"),
        aperture: CompareAperture { radius_arcsec: None, radius_px: Some(11.11), annulus_arcsec: None, annulus_px: None },
        cuts: &[
            PresetCut { column: "flags_<b>", op: CutOp::Lt, value: PresetCutValue::Number(4.0) },
            PresetCut { column: "imaflags_iso_<b>", op: CutOp::Eq, value: PresetCutValue::Number(0.0) },
        ],
        min_snr: Some(50.0),
        missing_value: None,
        min_neighbour_arcsec: None,
        notes: &[
            "flux_aper_8_<b>: diameter 22.22 px (5.84 arcsec at 0.263 arcsec per px), radius 11.11 px (2.92 arcsec), no local sky",
            "catalogue flux with zero point 30 and no exposure-time term",
            "catalogue errors of bright sources include the source Poisson term: compare errors on faint sources (signal-to-noise 5 to 20)",
        ],
    },
    SurveyPreset {
        id: "sdss_dr17",
        label: "SDSS DR17",
        bands: &["u", "g", "r", "i", "z"],
        flux: "aperFlux7_<b>",
        flux_unit: CatalogueFluxUnit::Nanomaggy,
        zero_point: None,
        flux_err: None,
        flux_ivar: None,
        id_column: "objID",
        ra: "ra",
        dec: "dec",
        class: Some("type"),
        aperture: CompareAperture {
            radius_arcsec: None,
            radius_px: Some(18.58),
            annulus_arcsec: None,
            annulus_px: Some([28.55, 44.21]),
        },
        cuts: &[
            PresetCut { column: "type", op: CutOp::Eq, value: PresetCutValue::Number(6.0) },
            PresetCut { column: "clean", op: CutOp::Eq, value: PresetCutValue::Number(1.0) },
            PresetCut { column: "psfMag_<b>", op: CutOp::Ge, value: PresetCutValue::Number(15.0) },
            PresetCut { column: "psfMag_<b>", op: CutOp::Le, value: PresetCutValue::Number(18.0) },
        ],
        min_snr: None,
        missing_value: None,
        min_neighbour_arcsec: None,
        notes: &[
            "aperFlux7_<b>: radius 18.58 px (7.36 arcsec at 0.396 arcsec per px), local sky from the annulus 28.55 to 44.21 px",
            "no error comparison: aperFlux7Ivar is not the pixel-noise error of this aperture",
        ],
    },
    SurveyPreset {
        id: "ps1_dr2",
        label: "Pan-STARRS1 DR2",
        bands: &["g", "r", "i", "z", "y"],
        flux: "<b>flxr5",
        flux_unit: CatalogueFluxUnit::Jy,
        zero_point: None,
        flux_err: Some("<b>flxr5err"),
        flux_ivar: None,
        id_column: "objid",
        ra: "ramean",
        dec: "decmean",
        class: None,
        aperture: CompareAperture { radius_arcsec: Some(3.0), radius_px: None, annulus_arcsec: None, annulus_px: None },
        cuts: &[PresetCut { column: "<b>flxr5fill", op: CutOp::Ge, value: PresetCutValue::Number(0.97) }],
        min_snr: Some(50.0),
        missing_value: Some(-999.0),
        min_neighbour_arcsec: None,
        notes: &[
            "<b>flxr5: radius 3.00 arcsec, flux in Jy, no local sky",
            "-999 marks a missing value",
            "<b>flxr5fill is 0.9748 for a complete aperture, so the fill cut is 0.97",
        ],
    },
    SurveyPreset {
        id: "euclid_q1",
        label: "Euclid Q1",
        bands: &["VIS"],
        flux: "flux_detection_total",
        flux_unit: CatalogueFluxUnit::MicroJy,
        zero_point: None,
        flux_err: None,
        flux_ivar: None,
        id_column: "object_id",
        ra: "ra",
        dec: "dec",
        class: Some("point_like_prob"),
        aperture: CompareAperture { radius_arcsec: Some(1.25), radius_px: None, annulus_arcsec: None, annulus_px: None },
        cuts: &[
            PresetCut { column: "point_like_prob", op: CutOp::Gt, value: PresetCutValue::Number(0.7) },
            PresetCut { column: "spurious_flag", op: CutOp::Eq, value: PresetCutValue::Number(0.0) },
        ],
        min_snr: None,
        missing_value: None,
        min_neighbour_arcsec: Some(2.0),
        notes: &[
            "Euclid Q1 has no fixed-aperture column: an aperture of 1.25 arcsec radius, on the plateau of the growth curve of point-like sources, is compared with flux_detection_total",
            "flux_vis_psf is not a total flux",
            "point_like_prob is the catalogue's probability, not a resolution verdict",
        ],
    },
];

fn table_spelling(columns: &[String], name: &str) -> Option<String> {
    columns
        .iter()
        .find(|column| column.as_str() == name)
        .or_else(|| columns.iter().find(|column| column.eq_ignore_ascii_case(name)))
        .cloned()
}

impl SurveyPreset {
    fn resolve(&self, columns: &[String], band: &str) -> Option<ResolvedPreset> {
        let named = |template: &str| template.replace(BAND_MARK, band);
        let found = |template: &str| table_spelling(columns, &named(template));
        let flux = found(self.flux)?;
        let ra = found(self.ra)?;
        let dec = found(self.dec)?;
        let mut notes: Vec<String> = self.notes.iter().map(|&note| named(note)).collect();

        let id = found(self.id_column);
        if id.is_none() {
            notes.push(format!(
                "column {} is not in the table: rows are named by their row number",
                named(self.id_column)
            ));
        }

        let mut min_snr = self.min_snr;
        if let Some(error) = self.flux_err.or(self.flux_ivar).filter(|&error| found(error).is_none()) {
            let lost_snr_cut = if min_snr.take().is_some() { " and no signal-to-noise cut" } else { "" };
            notes.push(format!("column {} is not in the table: no error comparison{lost_snr_cut}", named(error)));
        }

        let mut cuts = Vec::new();
        for preset_cut in self.cuts {
            let value = match preset_cut.value {
                PresetCutValue::Number(number) => CutValue::Number(number),
                PresetCutValue::Text(text) => CutValue::Text(text.to_string()),
            };
            let expected = named(preset_cut.column);
            match table_spelling(columns, &expected) {
                Some(column) => cuts.push(CompareCut { column, op: preset_cut.op, value }),
                None => {
                    let missing = CompareCut { column: expected, op: preset_cut.op, value };
                    notes.push(format!(
                        "{} left out: column {} is not in the table",
                        cut_text(&missing),
                        missing.column
                    ));
                }
            }
        }

        Some(ResolvedPreset {
            id: self.id.to_string(),
            label: self.label.to_string(),
            band: band.to_string(),
            mapping: CompareMapping {
                id,
                ra,
                dec,
                flux,
                flux_unit: self.flux_unit,
                zero_point: self.zero_point,
                flux_err: self.flux_err.and_then(found),
                flux_ivar: self.flux_ivar.and_then(found),
                class: self.class.and_then(found),
                missing_value: self.missing_value,
            },
            aperture: self.aperture.clone(),
            cuts,
            min_snr,
            max_snr: None,
            min_neighbour_arcsec: self.min_neighbour_arcsec,
            notes,
        })
    }
}

pub fn resolve_presets(columns: &[String]) -> Vec<ResolvedPreset> {
    SURVEY_PRESETS
        .iter()
        .flat_map(|preset| preset.bands.iter().filter_map(move |band| preset.resolve(columns, band)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::tables::{parse_text_table, TableFormat};

    const REASONS_CSV: &str = "id,ra,dec,flux,ivar,type,flag,frac\n\
a1,150.0000,2.0,100.0,400.0,PSF,0,0.001\n\
a2,150.0010,2.0,50.0,400.0,REX,0,0.001\n\
a3,150.0020,2.0,80.0,400.0,PSF,4,0.001\n\
a4,150.0030,2.0,,400.0,PSF,0,0.001\n\
a5,,2.0,60.0,400.0,PSF,0,0.001\n\
a6,150.0050,2.0,2.0,400.0,PSF,0,0.001\n\
a7,150.0060,2.0,-999,400.0,PSF,0,0.001\n\
a8,150.0070,2.0,90.0,400.0, psf ,0,0.5\n\
a9,150.0080,2.0,70.0,400.0,PSF,0,\n\
a10,150.0090,2.0,70.0,400.0,PSF,0,0.001\n";
    const REASONS_MAPPING_JSON: &str = r#"{"id":"id","ra":"ra","dec":"dec","flux":"flux","flux_unit":"nanomaggy","flux_ivar":"ivar","class":"type","missing_value":-999}"#;
    const REASONS_CUTS_JSON: &str = r#"[{"column":"type","op":"==","value":"PSF"},{"column":"flag","op":"==","value":0},{"column":"frac","op":"<","value":0.01}]"#;
    const REASONS_ON_IMAGE: [bool; 10] = [true, true, true, true, false, true, true, true, true, false];
    const REASONS_COLUMNS: &str = "id, ra, dec, flux, ivar, type, flag, frac";
    const FEW_ROWS_WARNING: &str = "only 3 rows are used: the median of fewer than 5 rows is indicative";
    const NO_ROW_WARNING: &str = "no row is left to compare: see the rejected rows";
    const SNR_CSV: &str = "ra,dec,flux,err\n\
150.00,2.0,8.0,2.0\n\
150.01,2.0,10.0,2.0\n\
150.02,2.0,40.0,2.0\n\
150.03,2.0,42.0,2.0\n\
150.04,2.0,30.0,\n\
150.05,2.0,0.0,2.0\n\
150.06,2.0,-3.0,2.0\n\
150.07,2.0,30.0,0.0\n";
    const REPORT_CSV: &str = "id,ra,dec,flux,err\n\
e1,150.001,2.0,1000,2.0\n\
e2,150.002,2.0,2000,2.0\n\
e3,150.003,2.0,3000,2.0\n\
e4,150.004,2.0,4000,2.0\n\
e5,150.005,2.0,5000,2.0\n\
e6,150.006,2.0,6000,2.0\n\
e7,150.007,2.0,7000,2.0\n\
e8,150.008,2.0,8000,2.0\n\
o1,150.009,2.0,100,2.0\n\
o2,150.010,2.0,100,2.0\n\
o3,150.011,2.0,100,2.0\n\
n1,,2.0,100,2.0\n";

    fn nanomaggy_jy() -> f64 {
        10f64.powf((8.90 - 22.5) / 2.5)
    }

    fn csv(text: &str) -> TextTable {
        parse_text_table(text, TableFormat::Csv, "inline").unwrap()
    }

    fn hand_table(columns: Vec<(&str, TableCells)>) -> TextTable {
        let n_rows = columns.first().map_or(0, |(_, cells)| match cells {
            TableCells::F64(values) => values.len(),
            TableCells::Text(values) => values.len(),
        });
        TextTable {
            format: TableFormat::Csv,
            columns: columns
                .into_iter()
                .map(|(name, cells)| TableColumn {
                    name: name.to_string(),
                    unit: None,
                    datatype: match cells {
                        TableCells::F64(_) => "float64".to_string(),
                        TableCells::Text(_) => "string".to_string(),
                    },
                    cells,
                })
                .collect(),
            n_rows,
            meta: serde_json::Value::Null,
            comments: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn mapping(flux_unit: CatalogueFluxUnit) -> CompareMapping {
        CompareMapping {
            id: None,
            ra: "ra".to_string(),
            dec: "dec".to_string(),
            flux: "flux".to_string(),
            flux_unit,
            zero_point: None,
            flux_err: None,
            flux_ivar: None,
            class: None,
            missing_value: None,
        }
    }

    fn selection(mapping: CompareMapping) -> CompareSelection {
        CompareSelection {
            mapping,
            cuts: Vec::new(),
            min_snr: None,
            max_snr: None,
            min_neighbour_arcsec: None,
        }
    }

    fn reasons_selection() -> CompareSelection {
        CompareSelection {
            cuts: serde_json::from_str(REASONS_CUTS_JSON).unwrap(),
            min_snr: Some(50.0),
            ..selection(serde_json::from_str(REASONS_MAPPING_JSON).unwrap())
        }
    }

    fn cut(column: &str, op: CutOp, value: CutValue) -> CompareCut {
        CompareCut {
            column: column.to_string(),
            op,
            value,
        }
    }

    fn text(value: &str) -> CutValue {
        CutValue::Text(value.to_string())
    }

    fn reasons(side: &CatalogueSide) -> Vec<Option<&str>> {
        side.entries
            .iter()
            .map(|entry| entry.rejection.as_ref().map(|rejection| rejection.reason.as_str()))
            .collect()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-12 * expected.abs(),
            "{actual:e} is not {expected:e} within a relative 1e-12"
        );
    }

    fn assert_stat(stat: Option<RobustStat>, median: f64, nmad: f64, n: usize) -> RobustStat {
        let stat = stat.expect("a statistic");
        assert_close(stat.median, median);
        assert_close(stat.nmad, nmad);
        assert_eq!(stat.n, n);
        stat
    }

    fn clean(flux: f64, err: f64) -> Measured {
        Measured {
            x: 10.0,
            y: 12.0,
            on_image: true,
            inside: true,
            error: None,
            flux_native: Some(flux),
            flux_err_native: Some(err),
            flux_jy: Some(flux * nanomaggy_jy()),
            flux_err_jy: Some(err * nanomaggy_jy()),
            masked_fraction: Some(0.0),
            saturated: false,
        }
    }

    fn unmeasured(on_image: bool, error: Option<&str>) -> Measured {
        Measured {
            on_image,
            error: error.map(str::to_string),
            flux_native: None,
            flux_err_native: None,
            flux_jy: None,
            flux_err_jy: None,
            masked_fraction: None,
            ..clean(0.0, 0.0)
        }
    }

    fn rejected(summary: &CompareSummary) -> Vec<(&str, usize)> {
        summary
            .rejected
            .iter()
            .map(|item| (item.reason.as_str(), item.count))
            .collect()
    }

    fn report_side() -> CatalogueSide {
        let table = csv(REPORT_CSV);
        let mut on_image = vec![true; 8];
        on_image.extend([false; 4]);
        let request = selection(CompareMapping {
            id: Some("id".to_string()),
            flux_err: Some("err".to_string()),
            ..mapping(CatalogueFluxUnit::Nanomaggy)
        });
        evaluate_catalogue(&table, &request, &on_image).unwrap()
    }

    fn two_row_side(id: Option<TableCells>) -> CatalogueSide {
        let mut columns = vec![
            ("ra", TableCells::F64(vec![150.0, 150.1])),
            ("dec", TableCells::F64(vec![2.0, 2.0])),
            ("flux", TableCells::F64(vec![1.0, 2.0])),
        ];
        let mut request = selection(mapping(CatalogueFluxUnit::Jy));
        if let Some(cells) = id {
            columns.push(("objid", cells));
            request.mapping.id = Some("objid".to_string());
        }
        evaluate_catalogue(&hand_table(columns), &request, &[true, true]).unwrap()
    }

    fn ids(side: &CatalogueSide) -> Vec<&str> {
        side.entries.iter().map(|entry| entry.id.as_str()).collect()
    }

    fn row_table(n: usize) -> TextTable {
        hand_table(vec![
            ("ra", TableCells::F64((0..n).map(|i| 10.0 + i as f64 * 0.001).collect())),
            ("dec", TableCells::F64(vec![0.0; n])),
            ("flux", TableCells::F64(vec![1.0; n])),
        ])
    }

    #[test]
    fn catalogue_fluxes_become_jansky_through_the_ab_zero_point_of_8_90() {
        use CatalogueFluxUnit::*;
        let fluxes = [
            (1.0, Nanomaggy, None, 3.6307805477010175e-06),
            (79.09336, Nanomaggy, None, 2.8717063294031377e-04),
            (445.29633, ZeroPoint, Some(30.0), 1.6167732529266465e-06),
            (2.0, MicroJy, None, 2e-6),
            (2.0, MilliJy, None, 2e-3),
            (2.0, NanoJy, None, 2e-9),
            (2.0, Jy, None, 2.0),
            (20.0, AbMag, None, 3.6307805477010174e-05),
        ];
        for (value, unit, zero_point, expected) in fluxes {
            assert_close(catalogue_flux_jy(value, unit, zero_point).unwrap(), expected);
        }
        assert_eq!(catalogue_flux_jy(f64::NAN, Nanomaggy, None), None);
        assert_eq!(catalogue_flux_jy(f64::NAN, AbMag, None), None);
        assert_eq!(catalogue_flux_jy(1.0, ZeroPoint, None), None);
        assert_eq!(catalogue_flux_jy(1.0, ZeroPoint, Some(f64::NAN)), None);

        assert_close(
            catalogue_flux_err_jy(79.09336, None, Some(1275.3608), Nanomaggy, None).unwrap(),
            1.0166780982761767e-07,
        );
        assert_close(
            catalogue_flux_err_jy(445.29633, Some(47.42379), None, ZeroPoint, Some(30.0)).unwrap(),
            1.7218537423025731e-07,
        );
        assert_close(
            catalogue_flux_err_jy(20.0, Some(0.02), None, AbMag, None).unwrap(),
            6.688144932055299e-07,
        );
        for ivar in [0.0, -1.0, f64::NAN] {
            assert_eq!(catalogue_flux_err_jy(79.09336, None, Some(ivar), Nanomaggy, None), None);
        }
        for err in [0.0, -1.0] {
            assert_eq!(catalogue_flux_err_jy(79.09336, Some(err), None, Nanomaggy, None), None);
        }
        assert_eq!(catalogue_flux_err_jy(79.09336, None, None, Nanomaggy, None), None);
    }

    #[test]
    fn a_request_that_cannot_be_read_is_refused_by_name() {
        let table = csv(REASONS_CSV);
        let base = reasons_selection();
        assert_eq!(validate_selection(&table, &base), Ok(()));
        let refusal = |change: &dyn Fn(&mut CompareSelection)| {
            let mut request = base.clone();
            change(&mut request);
            validate_selection(&table, &request).unwrap_err()
        };

        assert_eq!(
            refusal(&|request| request.mapping.flux = "flx".to_string()),
            format!("column 'flx' is not in the table; it has: {REASONS_COLUMNS}")
        );
        assert_eq!(
            refusal(&|request| request.cuts = vec![cut("nope", CutOp::Eq, CutValue::Number(1.0))]),
            format!("column 'nope' is not in the table; it has: {REASONS_COLUMNS}")
        );
        assert_eq!(
            refusal(&|request| request.mapping.ra = "type".to_string()),
            "column 'type' is text; ra needs a numeric column"
        );
        assert_eq!(
            refusal(&|request| request.mapping.flux_err = Some("frac".to_string())),
            "map flux_err or flux_ivar, not both"
        );
        assert_eq!(
            refusal(&|request| request.mapping.flux_unit = CatalogueFluxUnit::ZeroPoint),
            "flux_unit zero_point needs a finite zero_point"
        );
        assert_eq!(
            refusal(&|request| request.mapping.flux_unit = CatalogueFluxUnit::AbMag),
            "flux_ivar cannot be used with flux_unit ab_mag; map the magnitude error as flux_err"
        );
        assert_eq!(
            refusal(&|request| request.mapping.flux_ivar = None),
            "min_snr and max_snr need a catalogue error column (flux_err or flux_ivar)"
        );
        assert_eq!(
            refusal(&|request| {
                request.min_snr = Some(20.0);
                request.max_snr = Some(5.0);
            }),
            "min_snr 20 and max_snr 5 must be finite and in order"
        );
        assert_eq!(
            refusal(&|request| request.max_snr = Some(f64::INFINITY)),
            "min_snr 50 and max_snr inf must be finite and in order"
        );
        assert_eq!(
            refusal(&|request| request.min_neighbour_arcsec = Some(0.0)),
            "min_neighbour_arcsec 0 must be a positive number"
        );
        assert_eq!(
            refusal(&|request| request.cuts = vec![cut("type", CutOp::Lt, CutValue::Number(3.0))]),
            "cut on 'type': < needs a numeric column"
        );
        assert_eq!(
            refusal(&|request| request.cuts = vec![cut("flag", CutOp::Eq, text("abc"))]),
            "cut on 'flag': the column is numeric and 'abc' is not a number"
        );
        assert_eq!(
            refusal(&|request| request.cuts = vec![cut("type", CutOp::Eq, CutValue::Number(6.0))]),
            "cut on 'type': the column is text; give a text value"
        );

        let mut parsed_text = base.clone();
        parsed_text.cuts = vec![cut("flag", CutOp::Eq, text("0"))];
        assert_eq!(validate_selection(&table, &parsed_text), Ok(()));
        let side = evaluate_catalogue(&table, &parsed_text, &REASONS_ON_IMAGE).unwrap();
        assert_eq!(reasons(&side)[2], Some("cut flag == 0"));

        let names: Vec<String> = (1..=42).map(|n| format!("c{n}")).collect();
        let wide = hand_table(
            names
                .iter()
                .map(|name| (name.as_str(), TableCells::F64(Vec::new())))
                .collect(),
        );
        assert_eq!(
            validate_selection(&wide, &selection(mapping(CatalogueFluxUnit::Jy))).unwrap_err(),
            format!("column 'ra' is not in the table; it has: {} and 2 more", names[..40].join(", "))
        );
    }

    #[test]
    fn each_row_gets_the_first_reason_that_applies() {
        let table = csv(REASONS_CSV);
        let side = evaluate_catalogue(&table, &reasons_selection(), &REASONS_ON_IMAGE).unwrap();

        assert_eq!((side.n_table, side.n_no_position, side.n_off_image), (10, 1, 1));
        assert_eq!(side.warnings, Vec::<String>::new());
        let rows: Vec<usize> = side.entries.iter().map(|entry| entry.row).collect();
        assert_eq!(rows, vec![1, 2, 3, 4, 6, 7, 8, 9]);
        assert_eq!(
            reasons(&side),
            vec![
                None,
                Some("cut type == PSF"),
                Some("cut flag == 0"),
                Some("missing value in flux"),
                Some("catalogue signal-to-noise below 50"),
                Some("missing value (-999) in flux"),
                Some("cut frac < 0.01"),
                Some("cut frac < 0.01"),
            ]
        );
        let stages: Vec<(u8, usize)> = side
            .entries
            .iter()
            .filter_map(|entry| entry.rejection.as_ref().map(|rejection| (rejection.stage, rejection.order)))
            .collect();
        assert_eq!(stages, vec![(3, 0), (3, 1), (2, 0), (4, 0), (2, 0), (3, 2), (3, 2)]);

        let first = &side.entries[0];
        assert_eq!(first.id, "a1");
        assert_eq!((first.ra_deg, first.dec_deg), (150.0, 2.0));
        assert_close(first.flux_cat_jy.unwrap(), 100.0 * nanomaggy_jy());
        assert_close(first.flux_err_cat_jy.unwrap(), nanomaggy_jy() / 20.0);
        assert_close(first.snr_cat.unwrap(), 2000.0);
        assert_eq!(first.class.as_deref(), Some("PSF"));
        assert_eq!(side.entries[6].class.as_deref(), Some("psf"));
        assert_close(side.entries[4].snr_cat.unwrap(), 40.0);
        assert_eq!(side.entries[3].flux_cat_jy, None);
        assert_eq!(side.entries[5].flux_cat_jy, None);
    }

    #[test]
    fn ids_keep_every_digit_or_fall_back_to_the_row_number() {
        let survey_id = two_row_side(Some(TableCells::Text(vec![
            "10995300315432397".to_string(),
            String::new(),
        ])));
        assert_eq!(ids(&survey_id), vec!["10995300315432397", "row 2"]);
        assert_eq!(survey_id.warnings, Vec::<String>::new());

        let small = two_row_side(Some(TableCells::F64(vec![1394598529.0, 1.5])));
        assert_eq!(ids(&small), vec!["1394598529", "1.5"]);
        assert_eq!(small.warnings, Vec::<String>::new());

        let rounded = two_row_side(Some(TableCells::F64(vec![10995300315432396.0, 7.0])));
        assert_eq!(ids(&rounded), vec!["row 1", "row 2"]);
        assert_eq!(
            rounded.warnings,
            vec!["column 'objid' holds integers above 2^53 that arrived as floating point: rows are named by their row number".to_string()]
        );

        let unnamed = two_row_side(None);
        assert_eq!(ids(&unnamed), vec!["row 1", "row 2"]);
    }

    #[test]
    fn a_row_with_a_close_neighbour_is_left_out() {
        let table = hand_table(vec![
            ("ra", TableCells::F64(vec![150.0; 4])),
            (
                "dec",
                TableCells::F64(vec![2.0, 2.0 + 1.5 / 3600.0, 2.0 + 10.0 / 3600.0, 2.0 + 11.0 / 3600.0]),
            ),
            ("flux", TableCells::F64(vec![1.0; 4])),
            ("keep", TableCells::F64(vec![1.0, 1.0, 1.0, 0.0])),
        ]);
        let mut request = selection(mapping(CatalogueFluxUnit::Jy));
        request.cuts = vec![cut("keep", CutOp::Eq, CutValue::Number(1.0))];

        request.min_neighbour_arcsec = Some(2.0);
        let crowded = evaluate_catalogue(&table, &request, &[true; 4]).unwrap();
        assert_eq!(
            reasons(&crowded),
            vec![
                Some("neighbour within 2 arcsec"),
                Some("neighbour within 2 arcsec"),
                Some("neighbour within 2 arcsec"),
                Some("cut keep == 1"),
            ]
        );
        assert_eq!(crowded.entries[0].rejection.as_ref().map(|r| (r.stage, r.order)), Some((6, 0)));

        request.min_neighbour_arcsec = Some(0.5);
        let sparse = evaluate_catalogue(&table, &request, &[true; 4]).unwrap();
        assert_eq!(reasons(&sparse), vec![None, None, None, Some("cut keep == 1")]);
    }

    #[test]
    fn signal_to_noise_limits_and_non_positive_fluxes() {
        let table = csv(SNR_CSV);
        let mut request = selection(CompareMapping {
            flux_err: Some("err".to_string()),
            ..mapping(CatalogueFluxUnit::Jy)
        });
        let on_image = [true; 8];

        request.min_snr = Some(5.0);
        request.max_snr = Some(20.0);
        let limited = evaluate_catalogue(&table, &request, &on_image).unwrap();
        assert_eq!(
            reasons(&limited),
            vec![
                Some("catalogue signal-to-noise below 5"),
                None,
                None,
                Some("catalogue signal-to-noise above 20"),
                Some("missing value in err"),
                Some("catalogue signal-to-noise below 5"),
                Some("catalogue signal-to-noise below 5"),
                Some("catalogue signal-to-noise below 5"),
            ]
        );
        assert_eq!(limited.entries[0].rejection.as_ref().map(|r| r.stage), Some(4));
        assert_eq!(limited.entries[3].rejection.as_ref().map(|r| r.stage), Some(5));
        assert_eq!(limited.entries[1].snr_cat, Some(5.0));
        assert_eq!(limited.entries[2].snr_cat, Some(20.0));

        request.min_snr = None;
        let capped = evaluate_catalogue(&table, &request, &on_image).unwrap();
        assert_eq!(reasons(&capped)[7], Some("catalogue signal-to-noise above 20"));

        request.max_snr = None;
        let unlimited = evaluate_catalogue(&table, &request, &on_image).unwrap();
        assert_eq!(
            reasons(&unlimited),
            vec![
                None,
                None,
                None,
                None,
                None,
                Some("catalogue flux is not positive"),
                Some("catalogue flux is not positive"),
                None,
            ]
        );
        assert_eq!(unlimited.entries[5].rejection.as_ref().map(|r| r.stage), Some(7));
        assert_eq!(unlimited.entries[4].flux_cat_jy, Some(30.0));
        assert_eq!(unlimited.entries[4].flux_err_cat_jy, None);
        assert_eq!(unlimited.entries[4].snr_cat, None);
        assert_eq!(unlimited.entries[7].flux_err_cat_jy, None);
    }

    #[test]
    fn positions_outside_the_sky_are_no_positions() {
        let table = csv("ra,dec\n361,0\n10,-91\n,5\n10,\n99,5\n0,0\n360,90\n");
        let request = CompareMapping {
            missing_value: Some(99.0),
            ..mapping(CatalogueFluxUnit::Jy)
        };
        assert_eq!(
            catalogue_positions(&table, &request).unwrap(),
            vec![None, None, None, None, None, Some((0.0, 0.0)), Some((360.0, 90.0))]
        );
    }

    #[test]
    fn an_aperture_is_inside_when_no_part_leaves_the_pixel_grid() {
        assert!(aperture_inside(50.0, 50.0, 10.0, 100, 100));
        assert!(aperture_inside(9.5, 50.0, 10.0, 100, 100));
        assert!(!aperture_inside(9.4, 50.0, 10.0, 100, 100));
        assert!(aperture_inside(89.5, 50.0, 10.0, 100, 100));
        assert!(!aperture_inside(89.6, 50.0, 10.0, 100, 100));
        assert!(aperture_inside(50.0, 9.5, 10.0, 100, 100));
        assert!(!aperture_inside(50.0, 9.4, 10.0, 100, 100));
        assert!(aperture_inside(50.0, 89.5, 10.0, 100, 100));
        assert!(!aperture_inside(50.0, 89.6, 10.0, 100, 100));
        assert!(!aperture_inside(f64::NAN, 50.0, 10.0, 100, 100));
        assert!(!aperture_inside(50.0, f64::NAN, 10.0, 100, 100));
        assert!(!aperture_inside(50.0, 50.0, f64::NAN, 100, 100));
    }

    #[test]
    fn the_robust_statistics_are_the_median_and_1_4826_times_the_mad() {
        let five = assert_stat(robust_stat(&[0.98, 1.00, 1.01, 1.02, 1.50]), 1.01, 0.014826, 5);
        assert_close(five.median_err.unwrap(), 0.008309959977431575);

        let two = assert_stat(robust_stat(&[0.99, 1.01]), 1.0, 0.014826, 2);
        assert_close(two.median_err.unwrap(), 0.0131392003967626);

        assert_eq!(
            robust_stat(&[1.2]),
            Some(RobustStat {
                median: 1.2,
                nmad: 0.0,
                median_err: None,
                n: 1
            })
        );
        assert_eq!(robust_stat(&[]), None);
        assert_eq!(robust_stat(&[f64::NAN, f64::INFINITY]), None);
        assert_eq!(robust_stat(&[1.0, f64::NAN, 1.0]).map(|stat| stat.n), Some(2));
    }

    #[test]
    fn the_report_counts_every_stage_and_uses_only_clean_rows() {
        let side = report_side();
        assert_eq!((side.n_table, side.n_no_position, side.n_off_image), (12, 1, 3));
        assert_eq!(reasons(&side), vec![None; 8]);
        let outside = "position (3000, 32) lies outside the 64 x 64 image";
        let measured = vec![
            Some(clean(1010.0, 1.96)),
            Some(clean(2000.0, 2.0)),
            Some(clean(2970.0, 1.9)),
            Some(Measured {
                inside: false,
                ..clean(4000.0, 2.0)
            }),
            Some(Measured {
                masked_fraction: Some(0.25),
                ..clean(5000.0, 2.0)
            }),
            Some(Measured {
                saturated: true,
                ..clean(6000.0, 2.0)
            }),
            Some(unmeasured(true, Some(outside))),
            Some(unmeasured(false, None)),
        ];

        let summary = summarise(side, &measured).unwrap();

        assert_eq!(
            (summary.n_table, summary.n_on_image, summary.n_after_cuts, summary.n_measured, summary.n_used),
            (12, 8, 8, 5, 3)
        );
        let ratio = assert_stat(summary.ratio, 1.0, 0.014826, 3);
        assert_close(ratio.median_err.unwrap(), 1.2533141373155001 * 0.014826 / 3f64.sqrt());
        assert!(summary.ratio_offset_mag.unwrap().abs() < 1e-12);
        assert_stat(summary.err_ratio, 0.98, 0.029652, 3);
        assert_stat(summary.implied_zero_point_ab, 22.5, 0.01601717192536262, 3);
        assert_eq!(
            rejected(&summary),
            vec![
                ("no position", 1),
                ("off the image", 4),
                ("aperture crosses the image edge", 1),
                ("not measured: position (3000, 32) lies outside the 64 x 64 image", 1),
                ("aperture touches pixels without data", 1),
                ("saturated in the image", 1),
            ]
        );
        let used: Vec<bool> = summary.rows.iter().map(|row| row.used).collect();
        assert_eq!(used, vec![true, true, true, false, false, false, false, false]);
        let row_reasons: Vec<Option<&str>> = summary.rows.iter().map(|row| row.reason.as_deref()).collect();
        assert_eq!(
            row_reasons,
            vec![
                None,
                None,
                None,
                Some("aperture crosses the image edge"),
                Some("aperture touches pixels without data"),
                Some("saturated in the image"),
                Some("not measured: position (3000, 32) lies outside the 64 x 64 image"),
                Some("off the image"),
            ]
        );
        assert_close(summary.rows[0].ratio.unwrap(), 1.01);
        assert_close(summary.rows[4].ratio.unwrap(), 1.0);
        assert_close(summary.rows[5].ratio.unwrap(), 1.0);
        assert_eq!(summary.rows[6].ratio, None);
        assert_eq!(summary.rows[4].masked_fraction, Some(0.25));
        assert_eq!((summary.rows[0].row, summary.rows[0].id.as_str()), (1, "e1"));
        assert_eq!((summary.rows[0].x, summary.rows[0].y), (Some(10.0), Some(12.0)));
        assert_eq!(summary.rows[0].flux_native, Some(1010.0));
        assert_close(summary.rows[0].flux_err_cat_jy.unwrap(), 2.0 * nanomaggy_jy());
        assert_eq!(summary.warnings, vec![FEW_ROWS_WARNING.to_string()]);
    }

    #[test]
    fn an_image_without_a_jy_calibration_still_gives_the_implied_zero_point() {
        let table = csv(REPORT_CSV);
        let mut on_image = vec![true; 3];
        on_image.extend([false; 9]);
        let request = selection(CompareMapping {
            flux_err: Some("err".to_string()),
            ..mapping(CatalogueFluxUnit::Nanomaggy)
        });
        let side = evaluate_catalogue(&table, &request, &on_image).unwrap();
        let native_only = |flux: f64| {
            Some(Measured {
                flux_jy: None,
                flux_err_jy: None,
                ..clean(flux, 2.0)
            })
        };

        let summary = summarise(side, &[native_only(1010.0), native_only(2000.0), native_only(2970.0)]).unwrap();

        assert_eq!(summary.n_used, 3);
        assert_eq!(summary.ratio, None);
        assert_eq!(summary.err_ratio, None);
        assert_eq!(summary.ratio_offset_mag, None);
        assert_stat(summary.implied_zero_point_ab, 22.5, 0.01601717192536262, 3);
        assert!(summary.rows.iter().all(|row| row.used && row.ratio.is_none()));
        assert_eq!(
            summary.warnings,
            vec![
                FEW_ROWS_WARNING.to_string(),
                "the image has no flux calibration in Jy: the ratio is not available; the zero point implied by the catalogue is given instead".to_string(),
            ]
        );
    }

    #[test]
    fn the_report_orders_rejections_by_stage_then_cut_then_text() {
        let table = hand_table(vec![
            ("ra", TableCells::F64(vec![150.0, 150.1, 150.2, 150.2])),
            ("dec", TableCells::F64(vec![2.0, 2.0, 2.0, 2.0 + 1.0 / 3600.0])),
            ("flux", TableCells::F64(vec![100.0, 100.0, 4.0, 100.0])),
            ("err", TableCells::F64(vec![1.0; 4])),
            ("a", TableCells::F64(vec![0.0, 5.0, 0.0, 0.0])),
            ("b", TableCells::F64(vec![5.0, 0.0, 0.0, 0.0])),
        ]);
        let mut request = selection(CompareMapping {
            flux_err: Some("err".to_string()),
            ..mapping(CatalogueFluxUnit::Jy)
        });
        request.cuts = vec![
            cut("a", CutOp::Lt, CutValue::Number(1.0)),
            cut("b", CutOp::Lt, CutValue::Number(1.0)),
        ];
        request.min_snr = Some(10.0);
        request.min_neighbour_arcsec = Some(2.0);
        let side = evaluate_catalogue(&table, &request, &[true; 4]).unwrap();
        assert_eq!(
            reasons(&side),
            vec![
                Some("cut b < 1"),
                Some("cut a < 1"),
                Some("catalogue signal-to-noise below 10"),
                Some("neighbour within 2 arcsec"),
            ]
        );

        let summary = summarise(side, &[None, None, None, None]).unwrap();

        assert_eq!(
            rejected(&summary),
            vec![
                ("cut a < 1", 1),
                ("cut b < 1", 1),
                ("catalogue signal-to-noise below 10", 1),
                ("neighbour within 2 arcsec", 1),
            ]
        );
        assert_eq!((summary.n_on_image, summary.n_after_cuts, summary.n_measured, summary.n_used), (4, 0, 0, 0));
        assert_eq!(summary.warnings, vec![NO_ROW_WARNING.to_string()]);
    }

    #[test]
    fn empty_tables_single_rows_wrong_lengths_and_the_cap_of_5000_positions() {
        let request = selection(mapping(CatalogueFluxUnit::Jy));

        let empty = evaluate_catalogue(&csv("id,ra,dec,flux\n"), &request, &[]).unwrap();
        assert_eq!((empty.n_table, empty.entries.len()), (0, 0));
        let nothing = summarise(empty, &[]).unwrap();
        assert_eq!(
            (nothing.n_table, nothing.n_on_image, nothing.n_after_cuts, nothing.n_measured, nothing.n_used),
            (0, 0, 0, 0, 0)
        );
        assert_eq!(nothing.rejected, Vec::new());
        assert_eq!(nothing.rows, Vec::new());
        assert_eq!((nothing.ratio, nothing.err_ratio, nothing.implied_zero_point_ab), (None, None, None));
        assert_eq!(nothing.warnings, vec![NO_ROW_WARNING.to_string()]);

        let one_row = csv("ra,dec,flux\n150.0,2.0,3.0\n");
        let single = evaluate_catalogue(&one_row, &request, &[true]).unwrap();
        let only = summarise(single.clone(), &[Some(clean(3.0, 0.1))]).unwrap();
        assert_eq!((only.n_table, only.n_used), (1, 1));
        let ratio = assert_stat(only.ratio, nanomaggy_jy(), 0.0, 1);
        assert_eq!(ratio.median_err, None);
        assert_eq!(
            only.warnings,
            vec!["only 1 row is used: the median of fewer than 5 rows is indicative".to_string()]
        );

        assert_eq!(
            evaluate_catalogue(&one_row, &request, &[true, false]).unwrap_err(),
            "on_image has 2 flags for 1 rows"
        );
        assert_eq!(summarise(single, &[]).unwrap_err(), "0 measurements for 1 rows");

        let at_the_cap = evaluate_catalogue(&row_table(5000), &request, &vec![true; 5000]).unwrap();
        assert_eq!(at_the_cap.entries.len(), 5000);
        assert_eq!(
            evaluate_catalogue(&row_table(5001), &request, &vec![true; 5001]).unwrap_err(),
            "5001 catalogue rows are left to measure; the comparison measures at most 5000: cut the table or add cuts"
        );
    }

    fn names(columns: &[&str]) -> Vec<String> {
        columns.iter().map(|column| column.to_string()).collect()
    }

    fn number_cut(column: &str, op: CutOp, value: f64) -> CompareCut {
        cut(column, op, CutValue::Number(value))
    }

    fn preset_ids(presets: &[ResolvedPreset]) -> Vec<(&str, &str)> {
        presets.iter().map(|preset| (preset.id.as_str(), preset.band.as_str())).collect()
    }

    const LEGACY_R_COLUMNS: [&str; 8] =
        ["ls_id", "ra", "dec", "type", "maskbits", "fracflux_r", "apflux_r_5", "apflux_ivar_r_5"];

    fn legacy_notes(band: &str) -> Vec<String> {
        vec![
            format!("apflux_{band}_5: circular aperture of 2.0 arcsec radius, a plain sum with no local sky"),
            format!("apflux_{band}_1 to apflux_{band}_8 are radii of 0.5, 0.75, 1.0, 1.5, 2.0, 3.5, 5.0 and 7.0 arcsec"),
            "the catalogue leaves pixels of zero inverse variance out of the sum".to_string(),
        ]
    }

    #[test]
    fn a_legacy_table_resolves_one_preset_per_band_it_holds() {
        let presets = resolve_presets(&names(&LEGACY_R_COLUMNS));
        assert_eq!(
            presets,
            vec![ResolvedPreset {
                id: "legacy_dr10".to_string(),
                label: "Legacy Surveys DR10".to_string(),
                band: "r".to_string(),
                mapping: CompareMapping {
                    id: Some("ls_id".to_string()),
                    ra: "ra".to_string(),
                    dec: "dec".to_string(),
                    flux: "apflux_r_5".to_string(),
                    flux_unit: CatalogueFluxUnit::Nanomaggy,
                    zero_point: None,
                    flux_err: None,
                    flux_ivar: Some("apflux_ivar_r_5".to_string()),
                    class: Some("type".to_string()),
                    missing_value: None,
                },
                aperture: CompareAperture {
                    radius_arcsec: Some(2.0),
                    radius_px: None,
                    annulus_arcsec: None,
                    annulus_px: None,
                },
                cuts: vec![
                    cut("type", CutOp::Eq, text("PSF")),
                    number_cut("maskbits", CutOp::Eq, 0.0),
                    number_cut("fracflux_r", CutOp::Lt, 0.01),
                ],
                min_snr: None,
                max_snr: None,
                min_neighbour_arcsec: None,
                notes: legacy_notes("r"),
            }]
        );
    }

    #[test]
    fn column_names_match_without_regard_to_case_and_keep_the_table_spelling() {
        let mut columns = names(&LEGACY_R_COLUMNS);
        columns.push("APFLUX_G_5".to_string());
        let presets = resolve_presets(&columns);
        assert_eq!(preset_ids(&presets), vec![("legacy_dr10", "g"), ("legacy_dr10", "r")]);
        let g = &presets[0];
        assert_eq!(g.mapping.flux, "APFLUX_G_5");
        assert_eq!(g.mapping.flux_ivar, None);
        assert_eq!(g.mapping.id.as_deref(), Some("ls_id"));
        assert_eq!(
            g.cuts,
            vec![cut("type", CutOp::Eq, text("PSF")), number_cut("maskbits", CutOp::Eq, 0.0)]
        );
        let mut notes = legacy_notes("g");
        notes.push("column apflux_ivar_g_5 is not in the table: no error comparison".to_string());
        notes.push("cut fracflux_g < 0.01 left out: column fracflux_g is not in the table".to_string());
        assert_eq!(g.notes, notes);
        assert_eq!(presets[1].notes, legacy_notes("r"));

        let upper = resolve_presets(&names(&["LS_ID", "RA", "Dec", "TYPE", "apflux_r_5"]));
        assert_eq!(preset_ids(&upper), vec![("legacy_dr10", "r")]);
        assert_eq!(
            (upper[0].mapping.id.as_deref(), upper[0].mapping.ra.as_str(), upper[0].mapping.dec.as_str()),
            (Some("LS_ID"), "RA", "Dec")
        );
        assert_eq!(upper[0].mapping.class.as_deref(), Some("TYPE"));
        assert_eq!(upper[0].cuts, vec![cut("TYPE", CutOp::Eq, text("PSF"))]);
    }

    #[test]
    fn the_other_four_surveys_resolve_with_their_own_apertures_and_cuts() {
        let des_columns = [
            "coadd_object_id",
            "alphawin_j2000",
            "deltawin_j2000",
            "extended_class_coadd",
            "flags_r",
            "imaflags_iso_r",
            "flux_aper_8_r",
            "fluxerr_aper_8_r",
        ];
        let des = resolve_presets(&names(&des_columns));
        assert_eq!(preset_ids(&des), vec![("des_dr2", "r")]);
        let des = &des[0];
        assert_eq!(des.label, "DES DR2");
        assert_eq!(
            des.mapping,
            CompareMapping {
                id: Some("coadd_object_id".to_string()),
                ra: "alphawin_j2000".to_string(),
                dec: "deltawin_j2000".to_string(),
                flux: "flux_aper_8_r".to_string(),
                flux_unit: CatalogueFluxUnit::ZeroPoint,
                zero_point: Some(30.0),
                flux_err: Some("fluxerr_aper_8_r".to_string()),
                flux_ivar: None,
                class: Some("extended_class_coadd".to_string()),
                missing_value: None,
            }
        );
        assert_eq!(
            des.aperture,
            CompareAperture { radius_arcsec: None, radius_px: Some(11.11), annulus_arcsec: None, annulus_px: None }
        );
        assert_eq!(
            des.cuts,
            vec![number_cut("flags_r", CutOp::Lt, 4.0), number_cut("imaflags_iso_r", CutOp::Eq, 0.0)]
        );
        assert_eq!((des.min_snr, des.max_snr, des.min_neighbour_arcsec), (Some(50.0), None, None));
        assert_eq!(
            des.notes,
            vec![
                "flux_aper_8_r: diameter 22.22 px (5.84 arcsec at 0.263 arcsec per px), radius 11.11 px (2.92 arcsec), no local sky".to_string(),
                "catalogue flux with zero point 30 and no exposure-time term".to_string(),
                "catalogue errors of bright sources include the source Poisson term: compare errors on faint sources (signal-to-noise 5 to 20)".to_string(),
            ]
        );

        let des_without_error = resolve_presets(&names(&des_columns[..7]));
        assert_eq!(preset_ids(&des_without_error), vec![("des_dr2", "r")]);
        assert_eq!((des_without_error[0].mapping.flux_err.as_deref(), des_without_error[0].min_snr), (None, None));
        assert_eq!(
            des_without_error[0].notes.last().map(String::as_str),
            Some("column fluxerr_aper_8_r is not in the table: no error comparison and no signal-to-noise cut")
        );
        assert_eq!(des_without_error[0].notes.len(), 4);

        let sdss = resolve_presets(&names(&["objID", "ra", "dec", "type", "clean", "psfMag_r", "aperFlux7_r"]));
        assert_eq!(preset_ids(&sdss), vec![("sdss_dr17", "r")]);
        let sdss = &sdss[0];
        assert_eq!(sdss.label, "SDSS DR17");
        assert_eq!(
            (sdss.mapping.id.as_deref(), sdss.mapping.flux.as_str(), sdss.mapping.flux_unit),
            (Some("objID"), "aperFlux7_r", CatalogueFluxUnit::Nanomaggy)
        );
        assert_eq!((sdss.mapping.flux_err.as_deref(), sdss.mapping.flux_ivar.as_deref()), (None, None));
        assert_eq!(sdss.mapping.class.as_deref(), Some("type"));
        assert_eq!(
            sdss.aperture,
            CompareAperture { radius_arcsec: None, radius_px: Some(18.58), annulus_arcsec: None, annulus_px: Some([28.55, 44.21]) }
        );
        assert_eq!(
            sdss.cuts,
            vec![
                number_cut("type", CutOp::Eq, 6.0),
                number_cut("clean", CutOp::Eq, 1.0),
                number_cut("psfMag_r", CutOp::Ge, 15.0),
                number_cut("psfMag_r", CutOp::Le, 18.0),
            ]
        );
        assert_eq!((sdss.min_snr, sdss.max_snr, sdss.min_neighbour_arcsec), (None, None, None));
        assert_eq!(
            sdss.notes,
            vec![
                "aperFlux7_r: radius 18.58 px (7.36 arcsec at 0.396 arcsec per px), local sky from the annulus 28.55 to 44.21 px".to_string(),
                "no error comparison: aperFlux7Ivar is not the pixel-noise error of this aperture".to_string(),
            ]
        );

        let ps1 = resolve_presets(&names(&["objid", "ramean", "decmean", "rflxr5", "rflxr5err", "rflxr5fill"]));
        assert_eq!(preset_ids(&ps1), vec![("ps1_dr2", "r")]);
        let ps1 = &ps1[0];
        assert_eq!(ps1.label, "Pan-STARRS1 DR2");
        assert_eq!(
            ps1.mapping,
            CompareMapping {
                id: Some("objid".to_string()),
                ra: "ramean".to_string(),
                dec: "decmean".to_string(),
                flux: "rflxr5".to_string(),
                flux_unit: CatalogueFluxUnit::Jy,
                zero_point: None,
                flux_err: Some("rflxr5err".to_string()),
                flux_ivar: None,
                class: None,
                missing_value: Some(-999.0),
            }
        );
        assert_eq!(
            ps1.aperture,
            CompareAperture { radius_arcsec: Some(3.0), radius_px: None, annulus_arcsec: None, annulus_px: None }
        );
        assert_eq!(ps1.cuts, vec![number_cut("rflxr5fill", CutOp::Ge, 0.97)]);
        assert_eq!((ps1.min_snr, ps1.max_snr, ps1.min_neighbour_arcsec), (Some(50.0), None, None));
        assert_eq!(
            ps1.notes,
            vec![
                "rflxr5: radius 3.00 arcsec, flux in Jy, no local sky".to_string(),
                "-999 marks a missing value".to_string(),
                "rflxr5fill is 0.9748 for a complete aperture, so the fill cut is 0.97".to_string(),
            ]
        );

        let euclid = resolve_presets(&names(&[
            "object_id",
            "ra",
            "dec",
            "flux_detection_total",
            "point_like_prob",
            "spurious_flag",
        ]));
        assert_eq!(preset_ids(&euclid), vec![("euclid_q1", "VIS")]);
        let euclid = &euclid[0];
        assert_eq!(euclid.label, "Euclid Q1");
        assert_eq!(
            euclid.mapping,
            CompareMapping {
                id: Some("object_id".to_string()),
                ra: "ra".to_string(),
                dec: "dec".to_string(),
                flux: "flux_detection_total".to_string(),
                flux_unit: CatalogueFluxUnit::MicroJy,
                zero_point: None,
                flux_err: None,
                flux_ivar: None,
                class: Some("point_like_prob".to_string()),
                missing_value: None,
            }
        );
        assert_eq!(
            euclid.aperture,
            CompareAperture { radius_arcsec: Some(1.25), radius_px: None, annulus_arcsec: None, annulus_px: None }
        );
        assert_eq!(
            euclid.cuts,
            vec![number_cut("point_like_prob", CutOp::Gt, 0.7), number_cut("spurious_flag", CutOp::Eq, 0.0)]
        );
        assert_eq!((euclid.min_snr, euclid.max_snr, euclid.min_neighbour_arcsec), (None, None, Some(2.0)));
        assert_eq!(
            euclid.notes,
            vec![
                "Euclid Q1 has no fixed-aperture column: an aperture of 1.25 arcsec radius, on the plateau of the growth curve of point-like sources, is compared with flux_detection_total".to_string(),
                "flux_vis_psf is not a total flux".to_string(),
                "point_like_prob is the catalogue's probability, not a resolution verdict".to_string(),
            ]
        );

        let without_id = resolve_presets(&names(&["ra", "dec", "flux_detection_total"]));
        assert_eq!(preset_ids(&without_id), vec![("euclid_q1", "VIS")]);
        assert_eq!(without_id[0].mapping.id, None);
        assert_eq!(
            without_id[0].notes[3..],
            [
                "column object_id is not in the table: rows are named by their row number".to_string(),
                "cut point_like_prob > 0.7 left out: column point_like_prob is not in the table".to_string(),
                "cut spurious_flag == 0 left out: column spurious_flag is not in the table".to_string(),
            ]
        );
    }

    #[test]
    fn a_table_without_survey_columns_resolves_no_preset() {
        assert_eq!(resolve_presets(&names(&["ra", "dec", "flux"])), Vec::new());
        assert_eq!(resolve_presets(&names(&["ls_id", "dec", "apflux_r_5", "apflux_ivar_r_5"])), Vec::new());
        assert_eq!(resolve_presets(&[]), Vec::new());
    }

    #[test]
    fn a_resolved_preset_serialises_in_the_words_of_the_contract() {
        let des = resolve_presets(&names(&["alphawin_j2000", "deltawin_j2000", "flags_r", "flux_aper_8_r"]));
        let des = serde_json::to_value(&des[0]).unwrap();
        assert_eq!(des["mapping"]["flux_unit"], "zero_point");
        assert_eq!(des["cuts"][0], serde_json::json!({"column": "flags_r", "op": "<", "value": 4.0}));
        assert_eq!(des["aperture"], serde_json::json!({"radius_arcsec": null, "radius_px": 11.11, "annulus_arcsec": null, "annulus_px": null}));
        assert_eq!(des["max_snr"], serde_json::Value::Null);
        let keys: Vec<&str> = des.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["aperture", "band", "cuts", "id", "label", "mapping", "max_snr", "min_neighbour_arcsec", "min_snr", "notes"]
        );

        let legacy = serde_json::to_value(&resolve_presets(&names(&LEGACY_R_COLUMNS))[0]).unwrap();
        assert_eq!(legacy["cuts"][0]["value"], "PSF");
        assert_eq!(legacy["mapping"]["flux_unit"], "nanomaggy");

        let accepted: CompareMapping = serde_json::from_value(serde_json::json!({
            "id": null, "ra": "ra", "dec": "dec", "flux": "f", "flux_unit": "uJy", "zero_point": null,
            "flux_err": null, "flux_ivar": null, "class": null, "missing_value": null
        }))
        .unwrap();
        assert_eq!(accepted.flux_unit, CatalogueFluxUnit::MicroJy);
        assert_eq!(accepted.flux, "f");
        assert!(serde_json::from_value::<CompareMapping>(serde_json::json!({
            "id": null, "ra": "ra", "dec": "dec", "flux": "f", "flux_unit": "uJy", "zero_point": null,
            "flux_err": null, "flux_ivar": null, "class": null, "missing_value": null, "band": "r"
        }))
        .is_err());
    }
}
