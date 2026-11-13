use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

mod csv;
mod ecsv;

pub const MAX_TEXT_TABLE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_TABLE_CELLS: usize = 20_000_000;
pub(crate) const NO_ROWS_NOTE: &str = "the table has no data rows";
const BYTE_ORDER_MARK: char = '\u{feff}';
const COMMENT: char = '#';
const SIGNS: [char; 2] = ['+', '-'];
const LARGEST_EXACT_INTEGER: u128 = 1 << 53;
const CSV_EXTENSION: &str = "csv";
const ECSV_EXTENSION: &str = "ecsv";
const ECSV_SIGNATURE: &str = "%ECSV";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TableFormat {
    Csv,
    Ecsv,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableCells {
    F64(Vec<f64>),
    Text(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableColumn {
    pub name: String,
    pub unit: Option<String>,
    pub datatype: String,
    pub cells: TableCells,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextTable {
    pub format: TableFormat,
    pub columns: Vec<TableColumn>,
    pub n_rows: usize,
    pub meta: serde_json::Value,
    pub comments: Vec<(String, String)>,
    pub notes: Vec<String>,
}

pub fn read_text_table(path: &str) -> Result<TextTable> {
    read_text_table_capped(path, MAX_TEXT_TABLE_BYTES)
}

fn read_text_table_capped(path: &str, max_bytes: u64) -> Result<TextTable> {
    let file = File::open(path).with_context(|| format!("Failed to open {path}"))?;
    let size = file
        .metadata()
        .with_context(|| format!("Failed to read the size of {path}"))?
        .len();
    if size > max_bytes {
        bail!("{path} holds {size} bytes, more than the {max_bytes} bytes a text table may hold");
    }
    let mut bytes = Vec::new();
    file.take(max_bytes)
        .read_to_end(&mut bytes)
        .with_context(|| format!("Failed to read {path}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|e| anyhow!("{path} is not UTF-8 text ({e}): a text table must be UTF-8"))?;
    let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(&text);
    let (format, format_note) = table_format_of(path, text)?;
    let mut table = parse_text_table(text, format, path)?;
    if let Some(note) = format_note {
        table.notes.insert(0, note);
    }
    Ok(table)
}

pub fn parse_text_table(text: &str, format: TableFormat, label: &str) -> Result<TextTable> {
    match format {
        TableFormat::Csv => csv::parse_csv(text, label, MAX_TABLE_CELLS),
        TableFormat::Ecsv => ecsv::parse_ecsv(text, label, MAX_TABLE_CELLS),
    }
}

pub(crate) fn is_integer_literal(cell: &str) -> bool {
    let digits = cell.strip_prefix(SIGNS).unwrap_or(cell);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

pub(crate) fn beyond_exact_integers(cell: &str) -> bool {
    cell.parse::<i128>()
        .map_or(true, |value| value.unsigned_abs() > LARGEST_EXACT_INTEGER)
}

pub(crate) fn exact_integer_note(column: &str) -> String {
    format!("column '{column}': integers beyond 2^53 are kept as text so that identifiers stay exact")
}

pub(crate) fn cell_cap_error(label: &str, max_cells: usize) -> anyhow::Error {
    anyhow!("{label} holds more than {max_cells} cells (rows x columns): the table is too large to load")
}

pub(crate) fn table_format_of(path: &str, text: &str) -> Result<(TableFormat, Option<String>)> {
    let extension = Path::new(path).extension().and_then(|ext| ext.to_str());
    let has_extension = |wanted: &str| extension.is_some_and(|ext| ext.eq_ignore_ascii_case(wanted));
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let starts_as_ecsv = first
        .strip_prefix(COMMENT)
        .is_some_and(|rest| rest.trim_start_matches(' ').starts_with(ECSV_SIGNATURE));
    if starts_as_ecsv {
        let note = (!has_extension(ECSV_EXTENSION)).then(|| {
            let written = extension.map_or_else(|| "no extension".to_string(), |ext| format!(".{ext}"));
            format!("read as ECSV: the file starts with '# %ECSV' although its extension is {written}")
        });
        return Ok((TableFormat::Ecsv, note));
    }
    if has_extension(CSV_EXTENSION) {
        return Ok((TableFormat::Csv, None));
    }
    if has_extension(ECSV_EXTENSION) {
        bail!("{path} has the .ecsv extension but does not start with '# %ECSV'");
    }
    bail!("{path}: text tables are read from .csv and .ecsv files")
}

impl TextTable {
    pub fn column(&self, name: &str) -> Option<&TableColumn> {
        self.column_index(name).map(|index| &self.columns[index])
    }

    pub fn f64_column(&self, name: &str) -> Result<&[f64]> {
        let column = self.existing_column(name)?;
        match &column.cells {
            TableCells::F64(values) => Ok(values),
            TableCells::Text(_) => bail!(
                "column '{}' is text ({}), not numeric",
                column.name,
                column.datatype
            ),
        }
    }

    pub fn text_column(&self, name: &str) -> Result<&[String]> {
        let column = self.existing_column(name)?;
        match &column.cells {
            TableCells::Text(values) => Ok(values),
            TableCells::F64(_) => bail!(
                "column '{}' is numeric ({}), not text",
                column.name,
                column.datatype
            ),
        }
    }

    fn column_index(&self, name: &str) -> Option<usize> {
        let names = || self.columns.iter().map(|column| column.name.as_str());
        names()
            .position(|found| found == name)
            .or_else(|| names().position(|found| found.eq_ignore_ascii_case(name)))
    }

    fn existing_column(&self, name: &str) -> Result<&TableColumn> {
        self.column(name).ok_or_else(|| {
            let names: Vec<&str> = self.columns.iter().map(|column| column.name.as_str()).collect();
            anyhow!("table has no column '{name}'; columns: {}", names.join(", "))
        })
    }
}

impl TableColumn {
    pub fn is_numeric(&self) -> bool {
        matches!(self.cells, TableCells::F64(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;

    const SCIENCE_DATA_ENV: &str = "ASTROBURST_SCIENCE_DATA_DIR";
    const LABEL: &str = "t.csv";
    pub(super) const MIXED_TYPES_CSV: &str =
        "name,ra,dec,n,ok,flux\na,10.5,-20.25,3,true,1.5e-17\nb, 11 ,+0.5,-4,False,\n";
    const EXACT_INTEGER_NOTE_LS_ID: &str =
        "column 'ls_id': integers beyond 2^53 are kept as text so that identifiers stay exact";
    const ONE_IGNORED_COMMENT_NOTE: &str =
        "comment lines before the header without a 'key: value' pair: 1 (ignored)";

    pub(super) fn assert_f64_cells(column: &TableColumn, expected: &[f64]) {
        let TableCells::F64(cells) = &column.cells else {
            panic!("column '{}' holds text, numbers were expected", column.name);
        };
        assert_eq!(cells.len(), expected.len(), "column '{}'", column.name);
        for (row, (got, want)) in cells.iter().zip(expected).enumerate() {
            assert!(
                (got.is_nan() && want.is_nan()) || got.to_bits() == want.to_bits(),
                "column '{}', row {row}: {got:e} is not {want:e}",
                column.name
            );
        }
    }

    pub(super) fn assert_text_cells(column: &TableColumn, expected: &[&str]) {
        let TableCells::Text(cells) = &column.cells else {
            panic!("column '{}' holds numbers, text was expected", column.name);
        };
        assert_eq!(cells.as_slice(), expected, "column '{}'", column.name);
    }

    pub(super) fn comment_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn count_equal(values: &[f64], wanted: f64) -> usize {
        values.iter().filter(|&&value| value == wanted).count()
    }

    fn count_nan(values: &[f64]) -> usize {
        values.iter().filter(|value| value.is_nan()).count()
    }

    #[test]
    fn columns_are_found_by_exact_name_first_and_the_accessors_say_what_is_wrong() {
        let table = parse_text_table(MIXED_TYPES_CSV, TableFormat::Csv, LABEL).unwrap();
        assert_eq!(table.column("RA").unwrap().name, "ra");
        assert!(table.column("nope").is_none());
        assert!(table.column("ra").unwrap().is_numeric());
        assert!(!table.column("ok").unwrap().is_numeric());
        assert_eq!(table.f64_column("flux").unwrap().len(), 2);
        assert_eq!(
            table.f64_column("nope").unwrap_err().to_string(),
            "table has no column 'nope'; columns: name, ra, dec, n, ok, flux"
        );
        assert_eq!(
            table.text_column("nope").unwrap_err().to_string(),
            "table has no column 'nope'; columns: name, ra, dec, n, ok, flux"
        );
        assert_eq!(
            table.f64_column("NAME").unwrap_err().to_string(),
            "column 'name' is text (string), not numeric"
        );
        assert_eq!(
            table.text_column("ra").unwrap_err().to_string(),
            "column 'ra' is numeric (float64), not text"
        );
        assert_eq!(table.text_column("ok").unwrap(), ["true", "False"]);

        let cased = parse_text_table("Flux,flux\n1,2\n", TableFormat::Csv, LABEL).unwrap();
        assert_f64_cells(cased.column("flux").unwrap(), &[2.0]);
        assert_f64_cells(cased.column("FLUX").unwrap(), &[1.0]);
    }

    #[test]
    fn read_text_table_picks_the_format_and_refuses_what_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path.to_str().unwrap().to_string()
        };
        let refusal = |path: &str| read_text_table(path).unwrap_err().to_string();

        let csv = write("t.csv", MIXED_TYPES_CSV.as_bytes());
        let table = read_text_table(&csv).unwrap();
        assert_eq!(table.format, TableFormat::Csv);
        assert_eq!(table.n_rows, 2);
        assert!(table.notes.is_empty(), "{:?}", table.notes);

        let upper_case = write("T.CSV", MIXED_TYPES_CSV.as_bytes());
        assert_eq!(read_text_table(&upper_case).unwrap().format, TableFormat::Csv);

        let err = refusal(&write("t.txt", MIXED_TYPES_CSV.as_bytes()));
        assert!(err.contains("text tables are read from .csv and .ecsv files"), "{err}");

        let err = refusal(&write("t.ecsv", MIXED_TYPES_CSV.as_bytes()));
        assert!(
            err.contains("has the .ecsv extension but does not start with '# %ECSV'"),
            "{err}"
        );

        let ecsv_text = "# %ECSV 1.0\n# ---\n# datatype:\n# - {name: a, datatype: float64}\na\n1.0\n";
        let as_ecsv = read_text_table(&write("e.csv", ecsv_text.as_bytes())).unwrap();
        assert_eq!(as_ecsv.format, TableFormat::Ecsv);
        assert_eq!(as_ecsv.columns.len(), 1);
        assert_eq!(as_ecsv.columns[0].name, "a");
        assert_eq!(as_ecsv.columns[0].datatype, "float64");
        assert_f64_cells(&as_ecsv.columns[0], &[1.0]);
        assert_eq!(
            as_ecsv.notes.first().map(String::as_str),
            Some("read as ECSV: the file starts with '# %ECSV' although its extension is .csv")
        );

        let size = MIXED_TYPES_CSV.len() as u64;
        let limit = size - 1;
        let err = read_text_table_capped(&csv, limit).unwrap_err().to_string();
        assert!(
            err.contains(&size.to_string()) && err.contains(&limit.to_string()),
            "{err}"
        );
        assert_eq!(read_text_table_capped(&csv, size).unwrap().n_rows, 2);

        let err = refusal(&write("bin.csv", &[0xff, 0xfe, 0x00]));
        assert!(err.contains("is not UTF-8 text"), "{err}");

        let missing = dir.path().join("missing.csv");
        let err = refusal(missing.to_str().unwrap());
        assert!(err.contains("Failed to open"), "{err}");

        let with_mark = write("mark.csv", format!("\u{feff}{MIXED_TYPES_CSV}").as_bytes());
        assert_eq!(read_text_table(&with_mark).unwrap().columns[0].name, "name");

        assert_eq!(
            table_format_of("x.dat", "# %ECSV 1.0\n").unwrap(),
            (
                TableFormat::Ecsv,
                Some(
                    "read as ECSV: the file starts with '# %ECSV' although its extension is .dat"
                        .to_string()
                )
            )
        );
        assert_eq!(
            table_format_of("x", "\n#%ECSV 0.9\n").unwrap(),
            (
                TableFormat::Ecsv,
                Some(
                    "read as ECSV: the file starts with '# %ECSV' although its extension is no extension"
                        .to_string()
                )
            )
        );
        assert_eq!(
            table_format_of("x.ECSV", "# %ECSV 1.0\n").unwrap(),
            (TableFormat::Ecsv, None)
        );
        assert_eq!(
            table_format_of("x.Csv", "# file: a.fits\na\n1\n").unwrap(),
            (TableFormat::Csv, None)
        );
    }

    #[test]
    fn a_csv_with_the_provenance_lines_of_the_exports_reads_back_with_units() {
        let text = [
            "# software: AstroBurst v0.8.0",
            "# exported_utc: 2026-11-10T03:04:05.000Z",
            "# file: a.fits; b.fits",
            "# aperture_radius_arcsec: 1.5",
            "# note:",
            "# unit flux_ujy: uJy",
            "# unit flux_err_ujy: uJy",
            "id,flux_ujy,flux_err_ujy,note",
            "s1,2.5,0.25,\"a, b\"",
            "s2,,0.5,\"q\"\"x\"",
            "",
        ]
        .join("\r\n");
        let table = parse_text_table(&text, TableFormat::Csv, LABEL).unwrap();
        assert_eq!(
            table.comments,
            comment_pairs(&[
                ("software", "AstroBurst v0.8.0"),
                ("exported_utc", "2026-11-10T03:04:05.000Z"),
                ("file", "a.fits; b.fits"),
                ("aperture_radius_arcsec", "1.5"),
                ("note", ""),
                ("unit flux_ujy", "uJy"),
                ("unit flux_err_ujy", "uJy"),
            ])
        );
        assert!(table.notes.is_empty(), "{:?}", table.notes);
        assert_eq!(table.n_rows, 2);
        assert_eq!(table.columns.len(), 4);
        assert_text_cells(&table.columns[0], &["s1", "s2"]);
        assert_eq!(table.columns[0].name, "id");
        assert_eq!(table.columns[0].unit, None);
        assert_eq!(table.columns[1].name, "flux_ujy");
        assert_f64_cells(&table.columns[1], &[2.5, f64::NAN]);
        assert_eq!(table.columns[1].unit.as_deref(), Some("uJy"));
        assert_eq!(table.columns[2].name, "flux_err_ujy");
        assert_f64_cells(&table.columns[2], &[0.25, 0.5]);
        assert_eq!(table.columns[2].unit.as_deref(), Some("uJy"));
        assert_eq!(table.columns[3].name, "note");
        assert_text_cells(&table.columns[3], &["a, b", "q\"x"]);
        assert_eq!(table.columns[3].unit, None);
    }

    #[derive(Debug, PartialEq)]
    struct Census {
        rows: usize,
        columns: usize,
        text_columns: Vec<String>,
        f64_cells: usize,
        nan: usize,
        bits: u64,
    }

    fn census(table: &TextTable) -> Census {
        let mut census = Census {
            rows: table.n_rows,
            columns: table.columns.len(),
            text_columns: Vec::new(),
            f64_cells: 0,
            nan: 0,
            bits: 0,
        };
        for column in &table.columns {
            match &column.cells {
                TableCells::Text(_) => census.text_columns.push(column.name.clone()),
                TableCells::F64(values) => {
                    census.f64_cells += values.len();
                    census.nan += count_nan(values);
                    census.bits = values
                        .iter()
                        .filter(|value| !value.is_nan())
                        .fold(census.bits, |sum, value| sum.wrapping_add(value.to_bits()));
                }
            }
        }
        census
    }

    fn expected_census(
        rows: usize,
        columns: usize,
        text_columns: &[&str],
        f64_cells: usize,
        nan: usize,
        bits: u64,
    ) -> Census {
        Census {
            rows,
            columns,
            text_columns: text_columns.iter().map(|name| name.to_string()).collect(),
            f64_cells,
            nan,
            bits,
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/catalogs/*.csv, survey/S1/C4_*.csv, survey/P1/C5_*.csv"]
    fn real_the_six_saved_catalogues_read_with_the_counts_and_bits_of_the_oracle() {
        let data_dir = env_dir(SCIENCE_DATA_ENV);
        let expected = [
            (
                "survey/catalogs/C1_ls_dr10_box_A.csv",
                expected_census(297, 17, &["ls_id", "type", "brickname"], 4158, 0, 12702290954235689341),
            ),
            (
                "survey/catalogs/C1b_ls_dr10_brick_0532m280.csv",
                expected_census(69, 30, &["ls_id", "type", "brickname"], 1863, 0, 5594772081365853458),
            ),
            (
                "survey/catalogs/C2_des_dr2_box_A.csv",
                expected_census(148, 18, &[], 2664, 0, 677871704936781548),
            ),
            (
                "survey/catalogs/C3_euclid_q1_mer_cone_A.csv",
                expected_census(692, 21, &["object_id"], 13840, 1037, 1927497413521413116),
            ),
            (
                "survey/S1/C4_sdss_dr17_field_1462_4_165.csv",
                expected_census(808, 16, &["objID"], 12120, 0, 8025279126763919628),
            ),
            (
                "survey/P1/C5_ps1_dr2_cone_A.csv",
                expected_census(75, 13, &["objid"], 900, 0, 10229542862595792151),
            ),
        ];
        let tables: Vec<TextTable> = expected
            .iter()
            .map(|(file, _)| read_text_table(data_dir.join(file).to_str().unwrap()).unwrap())
            .collect();
        let measured: Vec<Census> = tables.iter().map(census).collect();
        for ((file, _), census) in expected.iter().zip(&measured) {
            println!("{file}: {census:?}");
        }
        for ((file, wanted), census) in expected.iter().zip(&measured) {
            assert_eq!(census, wanted, "{file} (oracle: plan/pacotes/BASE-R1-csv_oracle.py)");
        }

        let c1b = &tables[1];
        let reference_row = c1b
            .text_column("ls_id")
            .unwrap()
            .iter()
            .position(|id| id == "10995300315432397");
        assert_eq!(reference_row, Some(8), "catalogues.C1b.reference_star_row");
        assert_eq!(c1b.f64_column("ra").unwrap()[8], 53.13873891283809);
        assert_eq!(c1b.f64_column("dec").unwrap()[8], -28.08538886599608);
        assert_eq!(c1b.text_column("type").unwrap()[8], "PSF");
        assert_eq!(c1b.f64_column("maskbits").unwrap()[8], 0.0);
        assert_eq!(c1b.f64_column("apflux_r_5").unwrap()[8], 79.09336);
        assert_eq!(c1b.column("ls_id").unwrap().datatype, "int64");
        assert!(
            c1b.notes.iter().any(|note| note == EXACT_INTEGER_NOTE_LS_ID),
            "{:?}",
            c1b.notes
        );

        let c3 = &tables[3];
        for (name, empty_cells) in [("point_like_prob", 154), ("extended_prob", 692)] {
            let column = c3.column(name).unwrap();
            assert!(column.is_numeric(), "{name}");
            assert_eq!(column.datatype, "float64", "{name}");
            assert_eq!(
                count_nan(c3.f64_column(name).unwrap()),
                empty_cells,
                "catalogues.C3 empty cells of {name}"
            );
        }

        let c4 = &tables[4];
        assert!(
            c4.notes.iter().any(|note| note == ONE_IGNORED_COMMENT_NOTE),
            "{:?}",
            c4.notes
        );
        assert!(c4.comments.is_empty(), "{:?}", c4.comments);
        assert_eq!(c4.column("type").unwrap().datatype, "int64");
        let object_types = c4.f64_column("type").unwrap();
        assert_eq!(count_equal(object_types, 3.0), 496, "catalogues.C4.type_values");
        assert_eq!(count_equal(object_types, 6.0), 312, "catalogues.C4.type_values");

        let c5 = &tables[5];
        let kron_flux = c5.f64_column("rkronflux").unwrap();
        assert_eq!(count_equal(kron_flux, -999.0), 36, "catalogues.C5.n_minus_999.rkronflux");
        assert_eq!(count_nan(kron_flux), 0);
        let fill = c5.f64_column("rflxr5fill").unwrap();
        assert_eq!(count_equal(fill, 0.9748240113258362), 74);
        assert_eq!(count_equal(fill, -999.0), 1);
    }

    const WHTLT_FILES: [(&str, usize, usize, &str, u64); 16] = [
        (
            "jw01201-o103_t012_niriss_clear-gr700xd-substrip256",
            121,
            5,
            "-, -, MJy, MJy, MJy",
            8118970195848548794,
        ),
        (
            "jw01274-o013_t008_miri_p750l-slitlessprism",
            10,
            2,
            "-, -",
            675643576017680837,
        ),
        (
            "jw01366-o001_t001_niriss_clear-gr700xd-substrip256",
            537,
            5,
            "-, -, MJy, MJy, MJy",
            17035172115456278754,
        ),
        (
            "jw01366-o002_t001_nircam_f322w2-grismr-subgrism256",
            366,
            3,
            "-, -, DN / s",
            14546036033265677056,
        ),
        (
            "jw01366-o003_t001_nirspec_f290lp-g395h-s1600a1-sub2048",
            465,
            6,
            "-, -, Jy, -, -, Jy",
            17268563156647984689,
        ),
        (
            "jw01538-o067_t013_niriss_clear-gr700xd-substrip96",
            1,
            4,
            "-, -, MJy, MJy",
            219530550047830999,
        ),
        (
            "jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048",
            44,
            3,
            "-, -, Jy",
            6972171530579839937,
        ),
        (
            "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048",
            240,
            3,
            "-, -, Jy",
            9465661922402805722,
        ),
        (
            "jw02459-o013_t007_nircam_f322w2-grismr-subgrism64",
            1,
            3,
            "-, -, DN / s",
            14080931099942723615,
        ),
        (
            "jw02965-o002_t001_miri_p750l-slitlessprism",
            10,
            2,
            "-, -",
            675493082251539156,
        ),
        (
            "jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048",
            8,
            6,
            "-, -, Jy, -, -, Jy",
            2584386345839431242,
        ),
        (
            "jw04498-o013_t008_miri_p750l-slitlessprism",
            64,
            3,
            "-, -, Jy",
            10232748650099767796,
        ),
        (
            "jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512",
            65,
            3,
            "-, -, Jy",
            2033138272249231122,
        ),
        (
            "jw06828-o003_t001_nirspec_clear-prism-s1600a1-sub512",
            1,
            3,
            "-, -, -",
            13971034474081771101,
        ),
        (
            "jw07073-o004_t001_niriss_clear-gr700xd-substrip256",
            1,
            5,
            "-, -, MJy, MJy, MJy",
            4864263965144878714,
        ),
        (
            "jw07849-o002_t002_miri_p750l-slitlessprism",
            100,
            3,
            "-, -, Jy",
            11649246784507264331,
        ),
    ];
    const NO_UNIT: &str = "-";

    fn units_in_column_order(table: &TextTable) -> String {
        let units: Vec<&str> = table
            .columns
            .iter()
            .map(|column| column.unit.as_deref().unwrap_or(NO_UNIT))
            .collect();
        units.join(", ")
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/whtlt/*.ecsv"]
    fn real_the_sixteen_whtlt_files_read_bit_for_bit_like_astropy() {
        let folder = env_dir(SCIENCE_DATA_ENV).join("tso").join("whtlt");
        let tables: Vec<TextTable> = WHTLT_FILES
            .iter()
            .map(|(name, ..)| {
                let path = folder.join(format!("{name}_whtlt.ecsv"));
                read_text_table(path.to_str().unwrap()).unwrap()
            })
            .collect();
        for ((name, ..), table) in WHTLT_FILES.iter().zip(&tables) {
            println!(
                "{name}: {:?} units [{}]",
                census(table),
                units_in_column_order(table)
            );
        }
        for (&(name, rows, columns, units, bits), table) in WHTLT_FILES.iter().zip(&tables) {
            assert_eq!(
                census(table),
                expected_census(rows, columns, &[], rows * columns, 0, bits),
                "{name} (oracle: plan/pacotes/BASE-R2-ecsv_oracle.py)"
            );
            assert_eq!(units_in_column_order(table), units, "{name}");
            assert_eq!(
                table.meta["number_of_integrations"],
                serde_json::json!(table.n_rows),
                "{name}: number_of_integrations is the number of table rows"
            );
            assert!(table.notes.is_empty(), "{name}: {:?}", table.notes);
        }
        let table_of = |program: &str| {
            let index = WHTLT_FILES
                .iter()
                .position(|(name, ..)| name.starts_with(program))
                .unwrap();
            &tables[index]
        };

        let two_detectors = table_of("jw03375-o001");
        let names: Vec<&str> = two_detectors
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "MJD_UTC_NRS1",
                "BJD_TDB_NRS1",
                "whitelight_flux_NRS1",
                "MJD_UTC_NRS2",
                "BJD_TDB_NRS2",
                "whitelight_flux_NRS2"
            ],
            "goldens/tso/whtlt_jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048.json"
        );
        let flux_nrs1 = two_detectors.f64_column("whitelight_flux_NRS1").unwrap();
        assert_eq!(flux_nrs1[0], -5953.629162915719, "series[0].flux_first3[0]");
        assert_eq!(flux_nrs1[7], -6114.179097844157, "series[0].flux_last3[2]");
        assert_eq!(
            two_detectors.f64_column("MJD_UTC_NRS1").unwrap()[0],
            60496.52120635712,
            "time_first"
        );
        assert_eq!(
            two_detectors.f64_column("whitelight_flux_NRS2").unwrap()[0],
            3.7127694054398646
        );
        assert_eq!(
            two_detectors.meta,
            serde_json::json!({
                "instrument": "NIRSPEC",
                "exp_type": "NRS_BRIGHTOBJ",
                "subarray": "SUB2048",
                "filter": "F290LP",
                "pupil": null,
                "target_name": "SIPS J1256-1257B",
                "number_of_integrations": 8
            })
        );

        let one_row = table_of("jw06828-o003");
        assert_eq!(one_row.n_rows, 1);
        assert_eq!(
            one_row.f64_column("MJD_UTC_NRS1").unwrap(),
            [60599.140564409725]
        );
        assert_eq!(
            one_row.f64_column("whitelight_flux_NRS1").unwrap(),
            [2.988304267333806]
        );
        assert!(one_row.columns.iter().all(|column| column.unit.is_none()));

        let legacy = table_of("jw01274-o013");
        let names: Vec<&str> = legacy
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect();
        assert_eq!(names, ["MJD", "whitelight_flux"]);
        assert_eq!(legacy.meta["detector"], "MIRIMAGE");
    }
}
