use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

use crate::cmd::common::blocking_cmd;
use crate::infra::fits::gzip::resolve_gzip;
use crate::infra::fits::table::{list_tables, ColumnInfo, TableHdu};
use crate::infra::tables::{read_text_table, TableCells, TableColumn, TableFormat, TextTable};

pub const EMPTY_PATH_ERROR: &str = "path must name a table file, got an empty string";
pub(crate) const MAX_WIRE_CELLS: usize = 2_000_000;
const TEXT_TABLE_EXTENSIONS: [&str; 2] = ["csv", "ecsv"];
const FITS_LISTED_ONLY_NOTE: &str = "binary tables of a FITS file are listed only: this version reads the values of .csv and .ecsv tables";
const SPECTRUM_WAVELENGTH_COLUMNS: [&str; 2] = ["wavelength_vacuum_um", "wavelength_air_um"];
const SPECTRUM_FLUX_COLUMN: &str = "flux";
const TIME_COLUMNS: [&str; 5] = ["mjd", "jd_mid", "jd_utc", "hjd_utc", "bmjd_tdb"];
const TIME_COLUMN_PREFIXES: [&str; 2] = ["mjd_utc", "bjd_tdb"];
const VALUE_COLUMN_PARTS: [&str; 2] = ["flux", "mag"];
const SKY_POSITION_PAIRS: [(&str, &str); 3] = [
    ("ra", "dec"),
    ("alphawin_j2000", "deltawin_j2000"),
    ("ramean", "decmean"),
];
const DOTTED_RA_SUFFIX: &str = ".ra";
const DOTTED_DEC_SUFFIX: &str = ".dec";
const NO_GUESS_REASON: &str = "no column set of a spectrum, a light curve or a catalogue";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TableGuessKind {
    Spectrum,
    LightCurve,
    Catalogue,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TableGuess {
    pub kind: TableGuessKind,
    pub reason: String,
}

fn checked_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(EMPTY_PATH_ERROR.to_string());
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ProbeFormat {
    Csv,
    Ecsv,
    FitsBintable,
}

#[derive(Serialize)]
struct ProbeColumn {
    name: String,
    unit: Option<String>,
    datatype: String,
    numeric: bool,
}

#[derive(Serialize)]
struct ProbeHdu {
    hdu: usize,
    extname: Option<String>,
    n_rows: usize,
    n_columns: usize,
}

#[derive(Serialize)]
struct TableProbe {
    format: ProbeFormat,
    n_rows: usize,
    columns: Vec<ProbeColumn>,
    tables: Option<Vec<ProbeHdu>>,
    meta: serde_json::Value,
    comments: Vec<(String, String)>,
    guess: TableGuess,
    notes: Vec<String>,
}

#[derive(Serialize)]
struct ResultColumn<'a> {
    name: &'a str,
    unit: Option<&'a str>,
    datatype: &'a str,
    values: Option<&'a [f64]>,
    text: Option<&'a [String]>,
}

#[derive(Serialize)]
struct TableResult<'a> {
    format: TableFormat,
    n_rows: usize,
    columns: Vec<ResultColumn<'a>>,
    meta: &'a serde_json::Value,
    comments: &'a [(String, String)],
    notes: &'a [String],
}

fn is_time_column(lower_name: &str) -> bool {
    TIME_COLUMNS.contains(&lower_name)
        || TIME_COLUMN_PREFIXES
            .iter()
            .any(|prefix| lower_name.starts_with(prefix))
}

fn is_value_column(lower_name: &str) -> bool {
    !is_time_column(lower_name)
        && VALUE_COLUMN_PARTS
            .iter()
            .any(|part| lower_name.contains(part))
}

pub(crate) fn guess_table_kind(columns: &[(String, bool)]) -> TableGuess {
    let numeric: Vec<(&str, String)> = columns
        .iter()
        .filter(|(_, numeric)| *numeric)
        .map(|(name, _)| (name.as_str(), name.to_ascii_lowercase()))
        .collect();
    let first = |matches: &dyn Fn(&str) -> bool| {
        numeric
            .iter()
            .find(|(_, lower)| matches(lower))
            .map(|(name, _)| *name)
    };
    let named = |wanted: &str| first(&|lower| lower == wanted);
    let guess = |kind: TableGuessKind, reason: String| TableGuess { kind, reason };

    let wavelength = first(&|lower| SPECTRUM_WAVELENGTH_COLUMNS.contains(&lower));
    if let (Some(wavelength), Some(flux)) = (wavelength, named(SPECTRUM_FLUX_COLUMN)) {
        return guess(
            TableGuessKind::Spectrum,
            format!("columns {wavelength} and {flux}: a spectrum exported by AstroBurst"),
        );
    }
    if let (Some(time), Some(value)) = (first(&is_time_column), first(&is_value_column)) {
        return guess(
            TableGuessKind::LightCurve,
            format!("time column {time} and value column {value}"),
        );
    }
    let listed_pair = SKY_POSITION_PAIRS
        .iter()
        .find_map(|(ra, dec)| Some((named(ra)?, named(dec)?)));
    let dotted_pair = || {
        numeric.iter().find_map(|(name, lower)| {
            let prefix = lower.strip_suffix(DOTTED_RA_SUFFIX)?;
            Some((*name, named(&format!("{prefix}{DOTTED_DEC_SUFFIX}"))?))
        })
    };
    match listed_pair.or_else(dotted_pair) {
        Some((ra, dec)) => guess(
            TableGuessKind::Catalogue,
            format!("sky position columns {ra} and {dec}"),
        ),
        None => guess(TableGuessKind::Unknown, NO_GUESS_REASON.to_string()),
    }
}

fn guess_of(columns: &[ProbeColumn]) -> TableGuess {
    let described: Vec<(String, bool)> = columns
        .iter()
        .map(|column| (column.name.clone(), column.numeric))
        .collect();
    guess_table_kind(&described)
}

fn fits_datatype(type_code: char, repeat: usize) -> String {
    let name = match type_code {
        'L' => "bool",
        'X' => "bit",
        'B' => "uint8",
        'I' => "int16",
        'J' => "int32",
        'K' => "int64",
        'A' => "string",
        'E' => "float32",
        'D' => "float64",
        'C' => "complex64",
        'M' => "complex128",
        'P' | 'Q' => "vararray",
        _ => "unknown",
    };
    if repeat == 1 || matches!(type_code, 'A' | 'P' | 'Q') {
        name.to_string()
    } else {
        format!("{name}[{repeat}]")
    }
}

fn fits_numeric(type_code: char, repeat: usize) -> bool {
    repeat == 1 && matches!(type_code, 'B' | 'I' | 'J' | 'K' | 'E' | 'D')
}

fn fits_column(column: &ColumnInfo) -> ProbeColumn {
    ProbeColumn {
        name: column.name.clone(),
        unit: column.unit.clone(),
        datatype: fits_datatype(column.type_code, column.repeat),
        numeric: fits_numeric(column.type_code, column.repeat),
    }
}

fn is_text_table_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            TEXT_TABLE_EXTENSIONS
                .iter()
                .any(|wanted| extension.eq_ignore_ascii_case(wanted))
        })
}

fn probe_text_table(table: TextTable) -> TableProbe {
    let columns: Vec<ProbeColumn> = table
        .columns
        .into_iter()
        .map(|column| ProbeColumn {
            numeric: column.is_numeric(),
            name: column.name,
            unit: column.unit,
            datatype: column.datatype,
        })
        .collect();
    TableProbe {
        format: match table.format {
            TableFormat::Csv => ProbeFormat::Csv,
            TableFormat::Ecsv => ProbeFormat::Ecsv,
        },
        n_rows: table.n_rows,
        guess: guess_of(&columns),
        columns,
        tables: None,
        meta: table.meta,
        comments: table.comments,
        notes: table.notes,
    }
}

fn fits_binary_tables(path: &str) -> Result<Vec<TableHdu>> {
    let (readable, _decompressed) = resolve_gzip(Path::new(path))?;
    let file = File::open(&readable)?;
    Ok(list_tables(&file)?
        .into_iter()
        .map(|(table, _)| table)
        .collect())
}

fn probe_fits_tables(path: &str) -> Result<TableProbe> {
    let tables = fits_binary_tables(path).with_context(|| {
        format!("{path}: not a .csv or .ecsv table and not readable as FITS")
    })?;
    let Some(shown) = tables.first() else {
        bail!("{path} holds no binary table");
    };
    let columns: Vec<ProbeColumn> = shown.columns.iter().map(fits_column).collect();
    let mut notes = vec![FITS_LISTED_ONLY_NOTE.to_string()];
    if tables.len() > 1 {
        notes.push(format!(
            "{} binary tables: the columns shown are those of HDU {}",
            tables.len(),
            shown.index
        ));
    }
    Ok(TableProbe {
        format: ProbeFormat::FitsBintable,
        n_rows: shown.n_rows,
        guess: guess_of(&columns),
        columns,
        tables: Some(
            tables
                .iter()
                .map(|table| ProbeHdu {
                    hdu: table.index,
                    extname: table.extname.clone(),
                    n_rows: table.n_rows,
                    n_columns: table.columns.len(),
                })
                .collect(),
        ),
        meta: serde_json::Value::Null,
        comments: Vec::new(),
        notes,
    })
}

fn probe_table_json(path: &str) -> Result<serde_json::Value> {
    let probe = if is_text_table_path(path) {
        probe_text_table(read_text_table(path)?)
    } else {
        probe_fits_tables(path)?
    };
    Ok(serde_json::to_value(&probe)?)
}

fn requested_column<'a>(table: &'a TextTable, name: &str) -> Result<&'a TableColumn> {
    table.column(name).ok_or_else(|| {
        let names: Vec<&str> = table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect();
        anyhow!("table has no column '{name}'; columns: {}", names.join(", "))
    })
}

fn result_column(column: &TableColumn) -> ResultColumn<'_> {
    let (values, text) = match &column.cells {
        TableCells::F64(values) => (Some(values.as_slice()), None),
        TableCells::Text(text) => (None, Some(text.as_slice())),
    };
    ResultColumn {
        name: &column.name,
        unit: column.unit.as_deref(),
        datatype: &column.datatype,
        values,
        text,
    }
}

fn read_table_json(
    path: &str,
    wanted: Option<&[String]>,
    max_cells: usize,
) -> Result<serde_json::Value> {
    let table = read_text_table(path)?;
    let returned: Vec<&TableColumn> = match wanted {
        None => table.columns.iter().collect(),
        Some(names) => names
            .iter()
            .map(|name| requested_column(&table, name))
            .collect::<Result<_>>()?,
    };
    if table.n_rows.saturating_mul(returned.len()) > max_cells {
        bail!(
            "{path}: {} rows x {} columns is more than the {max_cells} cells sent to the interface at once: ask for fewer columns",
            table.n_rows,
            returned.len()
        );
    }
    let result = TableResult {
        format: table.format,
        n_rows: table.n_rows,
        columns: returned.into_iter().map(result_column).collect(),
        meta: &table.meta,
        comments: &table.comments,
        notes: &table.notes,
    };
    Ok(serde_json::to_value(&result)?)
}

#[tauri::command]
pub async fn probe_table_cmd(path: String) -> Result<serde_json::Value, String> {
    checked_path(&path)?;
    blocking_cmd!(probe_table_json(&path))
}

#[tauri::command]
pub async fn read_table_cmd(
    path: String,
    columns: Option<Vec<String>>,
) -> Result<serde_json::Value, String> {
    checked_path(&path)?;
    blocking_cmd!(read_table_json(&path, columns.as_deref(), MAX_WIRE_CELLS))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, write_raw_hdus};
    use crate::infra::fits::table::test_support::{bintable_cards, write_table};

    const SCIENCE_DATA_ENV: &str = "ASTROBURST_SCIENCE_DATA_DIR";
    const CAT_CSV: &str = "# file: a.fits\n# unit flux_r: nanomaggy\nbig_id,ra,dec,type,flux_r\n12345678901234567,53.1,-28.1,PSF,1.5\n12345678901234568,53.2,-28.2,REX,\n";
    const BIG_ID_NOTE: &str =
        "column 'big_id': integers beyond 2^53 are kept as text so that identifiers stay exact";
    const LC_ECSV: &str = "# %ECSV 1.0\n# ---\n# datatype:\n# - {name: MJD_UTC_NRS1, datatype: float64}\n# - {name: BJD_TDB_NRS1, datatype: float64}\n# - {name: whitelight_flux_NRS1, unit: Jy, datatype: float64}\n# meta: !!omap\n# - {instrument: NIRSPEC}\n# - {number_of_integrations: 2}\n# schema: astropy-2.0\nMJD_UTC_NRS1 BJD_TDB_NRS1 whitelight_flux_NRS1\n60000.5 60000.50125 2.5\n60000.75 60000.75125 2.25\n";
    const LISTED_ONLY_NOTE: &str = "binary tables of a FITS file are listed only: this version reads the values of .csv and .ecsv tables";
    const NO_GUESS_REASON: &str = "no column set of a spectrum, a light curve or a catalogue";

    fn write_text(dir: &tempfile::TempDir, name: &str, text: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn path_in(dir: &tempfile::TempDir, name: &str) -> String {
        dir.path().join(name).to_str().unwrap().to_string()
    }

    fn catalogue_fits(dir: &tempfile::TempDir) -> String {
        write_table(
            dir.path(),
            "cat.fits",
            &[
                ("RA", "D", Some("deg")),
                ("DEC", "D", Some("deg")),
                ("FLUX", "E", Some("Jy")),
                ("NAME", "8A", None),
                ("SPEC", "3E", None),
            ],
            2,
            vec![0u8; 80],
            &[("EXTNAME", "'CATALOG '".into())],
        )
        .to_str()
        .unwrap()
        .to_string()
    }

    fn named(columns: &[(&str, bool)]) -> Vec<(String, bool)> {
        columns
            .iter()
            .map(|(name, numeric)| (name.to_string(), *numeric))
            .collect()
    }

    fn column_names(value: &serde_json::Value) -> Vec<&str> {
        value["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|column| column["name"].as_str().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn probe_table_cmd_describes_a_csv_with_its_columns_comments_and_guess() {
        let dir = tempfile::tempdir().unwrap();
        let value = probe_table_cmd(write_text(&dir, "cat.csv", CAT_CSV))
            .await
            .unwrap();
        assert_eq!(
            value,
            json!({
                "format": "csv",
                "n_rows": 2,
                "columns": [
                    {"name": "big_id", "unit": null, "datatype": "int64", "numeric": false},
                    {"name": "ra", "unit": null, "datatype": "float64", "numeric": true},
                    {"name": "dec", "unit": null, "datatype": "float64", "numeric": true},
                    {"name": "type", "unit": null, "datatype": "string", "numeric": false},
                    {"name": "flux_r", "unit": "nanomaggy", "datatype": "float64", "numeric": true}
                ],
                "tables": null,
                "meta": null,
                "comments": [["file", "a.fits"], ["unit flux_r", "nanomaggy"]],
                "guess": {"kind": "catalogue", "reason": "sky position columns ra and dec"},
                "notes": [BIG_ID_NOTE]
            })
        );
    }

    #[tokio::test]
    async fn probe_table_cmd_describes_an_ecsv_light_curve_with_its_meta() {
        let dir = tempfile::tempdir().unwrap();
        let value = probe_table_cmd(write_text(&dir, "lc.ecsv", LC_ECSV))
            .await
            .unwrap();
        assert_eq!(
            value,
            json!({
                "format": "ecsv",
                "n_rows": 2,
                "columns": [
                    {"name": "MJD_UTC_NRS1", "unit": null, "datatype": "float64", "numeric": true},
                    {"name": "BJD_TDB_NRS1", "unit": null, "datatype": "float64", "numeric": true},
                    {"name": "whitelight_flux_NRS1", "unit": "Jy", "datatype": "float64", "numeric": true}
                ],
                "tables": null,
                "meta": {"instrument": "NIRSPEC", "number_of_integrations": 2},
                "comments": [],
                "guess": {
                    "kind": "light_curve",
                    "reason": "time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1"
                },
                "notes": []
            })
        );
    }

    #[tokio::test]
    async fn probe_table_cmd_guesses_a_light_curve_for_the_transit_fit_csv_the_app_writes() {
        const FIT_CSV: &str = "# software: AstroBurst v0.9.0\n# time: BMJD (TDB)\n# unit bmjd_tdb: d\n# unit flux: DN / s\n# unit residual_ppm: ppm\nbmjd_tdb,flux,flux_err,baseline,model,residual,norm_flux,norm_model,residual_ppm\n59782.83,7.1,,7.2,7.15,-0.05,0.986,0.993,-6944\n59782.84,7.0,,7.2,7.05,-0.05,0.972,0.979,-6944\n";
        let dir = tempfile::tempdir().unwrap();
        let value = probe_table_cmd(write_text(&dir, "w39_transit_fit.csv", FIT_CSV))
            .await
            .unwrap();
        assert_eq!(
            value["guess"],
            json!({"kind": "light_curve", "reason": "time column bmjd_tdb and value column flux"})
        );
        assert_eq!(
            guess_table_kind(&named(&[("BMJD_TDB", true), ("rel_flux", true)])),
            TableGuess {
                kind: TableGuessKind::LightCurve,
                reason: "time column BMJD_TDB and value column rel_flux".to_string()
            }
        );
    }

    #[tokio::test]
    async fn probe_table_cmd_lists_the_binary_tables_of_a_fits_file_without_reading_them() {
        let dir = tempfile::tempdir().unwrap();
        let value = probe_table_cmd(catalogue_fits(&dir)).await.unwrap();
        assert_eq!(
            value,
            json!({
                "format": "fits_bintable",
                "n_rows": 2,
                "columns": [
                    {"name": "RA", "unit": "deg", "datatype": "float64", "numeric": true},
                    {"name": "DEC", "unit": "deg", "datatype": "float64", "numeric": true},
                    {"name": "FLUX", "unit": "Jy", "datatype": "float32", "numeric": true},
                    {"name": "NAME", "unit": null, "datatype": "string", "numeric": false},
                    {"name": "SPEC", "unit": null, "datatype": "float32[3]", "numeric": false}
                ],
                "tables": [{"hdu": 1, "extname": "CATALOG", "n_rows": 2, "n_columns": 5}],
                "meta": null,
                "comments": [],
                "guess": {"kind": "catalogue", "reason": "sky position columns RA and DEC"},
                "notes": [LISTED_ONLY_NOTE]
            })
        );

        let only_primary = dir.path().join("image.fits");
        write_raw_hdus(&only_primary, &[(empty_primary_cards(), Vec::new())]);
        let err = probe_table_cmd(only_primary.to_str().unwrap().to_string())
            .await
            .unwrap_err();
        assert!(err.contains("holds no binary table"), "{err}");

        let two_tables = dir.path().join("two.fits");
        write_raw_hdus(
            &two_tables,
            &[
                (empty_primary_cards(), Vec::new()),
                (
                    bintable_cards(
                        &[("TIME", "D", Some("d")), ("COUNTS", "1PE(5)", None)],
                        0,
                        &[("EXTNAME", "'FIRST   '".into())],
                    ),
                    Vec::new(),
                ),
                (
                    bintable_cards(&[("ID", "K", None)], 0, &[("EXTNAME", "'SECOND  '".into())]),
                    Vec::new(),
                ),
            ],
        );
        let value = probe_table_cmd(two_tables.to_str().unwrap().to_string())
            .await
            .unwrap();
        assert_eq!(
            value,
            json!({
                "format": "fits_bintable",
                "n_rows": 0,
                "columns": [
                    {"name": "TIME", "unit": "d", "datatype": "float64", "numeric": true},
                    {"name": "COUNTS", "unit": null, "datatype": "vararray", "numeric": false}
                ],
                "tables": [
                    {"hdu": 1, "extname": "FIRST", "n_rows": 0, "n_columns": 2},
                    {"hdu": 2, "extname": "SECOND", "n_rows": 0, "n_columns": 1}
                ],
                "meta": null,
                "comments": [],
                "guess": {"kind": "unknown", "reason": NO_GUESS_REASON},
                "notes": [
                    LISTED_ONLY_NOTE,
                    "2 binary tables: the columns shown are those of HDU 1"
                ]
            })
        );

        let described: Vec<(String, bool)> = [
            ('L', 1),
            ('X', 8),
            ('B', 1),
            ('I', 1),
            ('J', 1),
            ('K', 1),
            ('A', 8),
            ('E', 1),
            ('D', 1),
            ('C', 1),
            ('M', 1),
            ('P', 0),
            ('Q', 0),
            ('Z', 1),
            ('D', 4),
            ('J', 0),
        ]
        .iter()
        .map(|&(code, repeat)| (fits_datatype(code, repeat), fits_numeric(code, repeat)))
        .collect();
        assert_eq!(
            described,
            named(&[
                ("bool", false),
                ("bit[8]", false),
                ("uint8", true),
                ("int16", true),
                ("int32", true),
                ("int64", true),
                ("string", false),
                ("float32", true),
                ("float64", true),
                ("complex64", false),
                ("complex128", false),
                ("vararray", false),
                ("vararray", false),
                ("unknown", false),
                ("float64[4]", false),
                ("int32[0]", false),
            ])
        );
    }

    #[test]
    fn the_guess_follows_the_column_names() {
        const SPECTRUM_REASON: &str =
            "columns wavelength_vacuum_um and flux: a spectrum exported by AstroBurst";
        let cases: [(&[(&str, bool)], TableGuessKind, &str); 13] = [
            (
                &[
                    ("channel", true),
                    ("wavelength_vacuum_um", true),
                    ("flux", true),
                    ("flux_unit", false),
                ],
                TableGuessKind::Spectrum,
                SPECTRUM_REASON,
            ),
            (
                &[
                    ("channel", true),
                    ("wavelength_vacuum_um", true),
                    ("wavelength_air_um", true),
                    ("flux", true),
                ],
                TableGuessKind::Spectrum,
                SPECTRUM_REASON,
            ),
            (
                &[("MJD", true), ("whitelight_flux", true)],
                TableGuessKind::LightCurve,
                "time column MJD and value column whitelight_flux",
            ),
            (
                &[
                    ("MJD_UTC_NRS1", true),
                    ("BJD_TDB_NRS1", true),
                    ("whitelight_flux_NRS1", true),
                ],
                TableGuessKind::LightCurve,
                "time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1",
            ),
            (
                &[
                    ("index", true),
                    ("file", false),
                    ("jd_mid", true),
                    ("bjd_tdb", true),
                    ("diff_mag", true),
                    ("target_flux", true),
                ],
                TableGuessKind::LightCurve,
                "time column jd_mid and value column diff_mag",
            ),
            (
                &[("bjd_tdb", true), ("airmass", true)],
                TableGuessKind::Unknown,
                NO_GUESS_REASON,
            ),
            (
                &[("ra", true), ("dec", true), ("flux_r", true)],
                TableGuessKind::Catalogue,
                "sky position columns ra and dec",
            ),
            (
                &[
                    ("coadd_object_id", true),
                    ("alphawin_j2000", true),
                    ("deltawin_j2000", true),
                ],
                TableGuessKind::Catalogue,
                "sky position columns alphawin_j2000 and deltawin_j2000",
            ),
            (
                &[("objid", true), ("ramean", true), ("decmean", true)],
                TableGuessKind::Catalogue,
                "sky position columns ramean and decmean",
            ),
            (
                &[
                    ("label", true),
                    ("sky_centroid.ra", true),
                    ("sky_centroid.dec", true),
                ],
                TableGuessKind::Catalogue,
                "sky position columns sky_centroid.ra and sky_centroid.dec",
            ),
            (
                &[("ra", false), ("dec", true)],
                TableGuessKind::Unknown,
                NO_GUESS_REASON,
            ),
            (
                &[("ra", true), ("dec", true), ("mjd", true), ("mag", true)],
                TableGuessKind::LightCurve,
                "time column mjd and value column mag",
            ),
            (
                &[("a", true), ("b", true)],
                TableGuessKind::Unknown,
                NO_GUESS_REASON,
            ),
        ];
        for (columns, kind, reason) in cases {
            assert_eq!(
                guess_table_kind(&named(columns)),
                TableGuess {
                    kind,
                    reason: reason.to_string()
                },
                "{columns:?}"
            );
        }
    }

    #[tokio::test]
    async fn read_table_cmd_returns_numbers_with_null_for_missing_cells_and_text_columns_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_text(&dir, "cat.csv", CAT_CSV);
        let value = read_table_cmd(path.clone(), None).await.unwrap();
        assert_eq!(
            value,
            json!({
                "format": "csv",
                "n_rows": 2,
                "columns": [
                    {
                        "name": "big_id",
                        "unit": null,
                        "datatype": "int64",
                        "values": null,
                        "text": ["12345678901234567", "12345678901234568"]
                    },
                    {"name": "ra", "unit": null, "datatype": "float64", "values": [53.1, 53.2], "text": null},
                    {"name": "dec", "unit": null, "datatype": "float64", "values": [-28.1, -28.2], "text": null},
                    {"name": "type", "unit": null, "datatype": "string", "values": null, "text": ["PSF", "REX"]},
                    {"name": "flux_r", "unit": "nanomaggy", "datatype": "float64", "values": [1.5, null], "text": null}
                ],
                "meta": null,
                "comments": [["file", "a.fits"], ["unit flux_r", "nanomaggy"]],
                "notes": [BIG_ID_NOTE]
            })
        );

        let picked = read_table_cmd(path, Some(vec!["FLUX_R".to_string(), "ra".to_string()]))
            .await
            .unwrap();
        assert_eq!(column_names(&picked), ["flux_r", "ra"]);
        assert_eq!(picked["columns"][0]["values"], json!([1.5, null]));
        assert_eq!(picked["columns"][1]["values"], json!([53.1, 53.2]));
        assert_eq!(picked["n_rows"], 2);
    }

    #[tokio::test]
    async fn read_table_cmd_refuses_an_unknown_column_a_fits_path_an_empty_path_and_too_many_cells()
    {
        let dir = tempfile::tempdir().unwrap();
        let path = write_text(&dir, "cat.csv", CAT_CSV);
        assert_eq!(
            read_table_cmd(path.clone(), Some(vec!["nope".to_string()]))
                .await
                .unwrap_err(),
            "table has no column 'nope'; columns: big_id, ra, dec, type, flux_r"
        );

        let err = read_table_cmd(catalogue_fits(&dir), None)
            .await
            .unwrap_err();
        assert!(
            err.contains("text tables are read from .csv and .ecsv files"),
            "{err}"
        );

        assert_eq!(
            read_table_cmd("  ".to_string(), None).await.unwrap_err(),
            EMPTY_PATH_ERROR
        );

        let err = read_table_json(&path, None, 9).unwrap_err().to_string();
        assert!(
            err.contains("2 rows x 5 columns is more than the 9 cells"),
            "{err}"
        );
        assert_eq!(
            err,
            format!("{path}: 2 rows x 5 columns is more than the 9 cells sent to the interface at once: ask for fewer columns")
        );
        assert_eq!(read_table_json(&path, None, 10).unwrap()["n_rows"], 2);
        let four_cells = ["ra".to_string(), "dec".to_string()];
        assert_eq!(
            column_names(&read_table_json(&path, Some(&four_cells), 4).unwrap()),
            ["ra", "dec"]
        );
    }

    #[tokio::test]
    async fn probe_table_cmd_refuses_an_empty_path_and_names_a_missing_file() {
        assert_eq!(
            probe_table_cmd("  ".to_string()).await.unwrap_err(),
            EMPTY_PATH_ERROR
        );
        let dir = tempfile::tempdir().unwrap();
        let err = probe_table_cmd(path_in(&dir, "missing.csv"))
            .await
            .unwrap_err();
        assert!(err.contains("Failed to open"), "{err}");
        let missing_fits = path_in(&dir, "missing.fits");
        let err = probe_table_cmd(missing_fits.clone()).await.unwrap_err();
        assert!(
            err.starts_with(&format!(
                "{missing_fits}: not a .csv or .ecsv table and not readable as FITS"
            )),
            "{err}"
        );
    }

    const WHTLT_FILE_COUNT: usize = 16;
    const CATALOGUES: [(&str, usize, &str); 6] = [
        (
            "survey/catalogs/C1_ls_dr10_box_A.csv",
            297,
            "sky position columns ra and dec",
        ),
        (
            "survey/catalogs/C1b_ls_dr10_brick_0532m280.csv",
            69,
            "sky position columns ra and dec",
        ),
        (
            "survey/catalogs/C2_des_dr2_box_A.csv",
            148,
            "sky position columns ra and dec",
        ),
        (
            "survey/catalogs/C3_euclid_q1_mer_cone_A.csv",
            692,
            "sky position columns ra and dec",
        ),
        (
            "survey/S1/C4_sdss_dr17_field_1462_4_165.csv",
            808,
            "sky position columns ra and dec",
        ),
        (
            "survey/P1/C5_ps1_dr2_cone_A.csv",
            75,
            "sky position columns ramean and decmean",
        ),
    ];
    const WHTLT_PINS: [(&str, usize, Option<&str>); 3] = [
        (
            "jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_whtlt.ecsv",
            8,
            Some("time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1"),
        ),
        (
            "jw01274-o013_t008_miri_p750l-slitlessprism_whtlt.ecsv",
            10,
            Some("time column MJD and value column whitelight_flux"),
        ),
        (
            "jw06828-o003_t001_nirspec_clear-prism-s1600a1-sub512_whtlt.ecsv",
            1,
            None,
        ),
    ];

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/whtlt/*.ecsv, survey/catalogs/*.csv, survey/S1/C4_*.csv, survey/P1/C5_*.csv"]
    fn real_probe_guesses_a_light_curve_for_the_whtlt_files_and_a_catalogue_for_the_saved_tables() {
        let data_dir = env_dir(SCIENCE_DATA_ENV);
        let whtlt_dir = data_dir.join("tso").join("whtlt");
        let mut whtlt_names: Vec<String> = std::fs::read_dir(&whtlt_dir)
            .unwrap_or_else(|error| panic!("{}: {error}", whtlt_dir.display()))
            .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_string())
            .filter(|name| name.ends_with(".ecsv"))
            .collect();
        whtlt_names.sort();
        let probe = |path: std::path::PathBuf| probe_table_json(path.to_str().unwrap()).unwrap();
        let light_curves: Vec<(String, serde_json::Value)> = whtlt_names
            .into_iter()
            .map(|name| {
                let value = probe(whtlt_dir.join(&name));
                (name, value)
            })
            .collect();
        let catalogues: Vec<(String, serde_json::Value)> = CATALOGUES
            .iter()
            .map(|(file, ..)| (file.to_string(), probe(data_dir.join(file))))
            .collect();
        for (file, value) in light_curves.iter().chain(&catalogues) {
            println!(
                "{file}: {} | {} | {} rows",
                value["guess"]["kind"].as_str().unwrap(),
                value["guess"]["reason"].as_str().unwrap(),
                value["n_rows"]
            );
        }

        assert_eq!(light_curves.len(), WHTLT_FILE_COUNT);
        for (file, value) in &light_curves {
            assert_eq!(value["format"], "ecsv", "{file}");
            assert_eq!(value["guess"]["kind"], "light_curve", "{file}");
        }
        for (file, n_rows, reason) in WHTLT_PINS {
            let (_, value) = light_curves
                .iter()
                .find(|(name, _)| name == file)
                .unwrap_or_else(|| panic!("{file} is not in {}", whtlt_dir.display()));
            assert_eq!(value["n_rows"], n_rows, "{file}");
            if let Some(reason) = reason {
                assert_eq!(value["guess"]["reason"], reason, "{file}");
            }
        }
        for ((file, value), (_, n_rows, reason)) in catalogues.iter().zip(CATALOGUES) {
            assert_eq!(value["format"], "csv", "{file}");
            assert_eq!(
                value["guess"],
                json!({"kind": "catalogue", "reason": reason}),
                "{file}"
            );
            assert_eq!(
                value["n_rows"], n_rows,
                "{file} (plan/goldens/survey/catalogues.json, n_rows)"
            );
        }
    }
}
