use std::collections::HashMap;

use anyhow::{bail, Result};

use super::{
    beyond_exact_integers, cell_cap_error, exact_integer_note, is_integer_literal, TableCells,
    TableColumn, TableFormat, TextTable, BYTE_ORDER_MARK, COMMENT, NO_ROWS_NOTE,
};

const QUOTE: char = '"';
const SEPARATOR: char = ',';
const LINE_FEED: char = '\n';
const CARRIAGE_RETURN: char = '\r';
const KEY_SEPARATOR: char = ':';
const UNIT_KEY_PREFIX: &str = "unit ";
const DATATYPE_FLOAT: &str = "float64";
const DATATYPE_INTEGER: &str = "int64";
const DATATYPE_BOOL: &str = "bool";
const DATATYPE_STRING: &str = "string";
const BOOL_LITERALS: [&str; 2] = ["true", "false"];

pub(crate) fn parse_csv(text: &str, label: &str, max_cells: usize) -> Result<TextTable> {
    let mut cursor = Cursor {
        rest: text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(text),
        line: 1,
    };
    let mut comments = Vec::new();
    let mut comments_without_pair = 0_usize;
    let header = loop {
        match cursor.next_line(label)? {
            None => bail!("{label} holds no header row"),
            Some(Line::Comment(comment)) => match comment_pair(comment) {
                Some(pair) => comments.push(pair),
                None => comments_without_pair += 1,
            },
            Some(Line::Record { fields, .. }) => break fields,
        }
    };
    let mut notes = Vec::new();
    if comments_without_pair > 0 {
        notes.push(format!(
            "comment lines before the header without a 'key: value' pair: {comments_without_pair} (ignored)"
        ));
    }
    let names = column_names(header, label, &mut notes)?;
    let mut cells: Vec<Vec<String>> = vec![Vec::new(); names.len()];
    let mut n_rows = 0_usize;
    while let Some(line) = cursor.next_line(label)? {
        let Line::Record { fields, line } = line else {
            continue;
        };
        if fields.len() != names.len() {
            bail!(
                "{label}, line {line}: {} fields, the header has {}",
                fields.len(),
                names.len()
            );
        }
        n_rows += 1;
        if n_rows.saturating_mul(names.len()) > max_cells {
            return Err(cell_cap_error(label, max_cells));
        }
        for (column, field) in cells.iter_mut().zip(fields) {
            column.push(field);
        }
    }
    let columns = names
        .into_iter()
        .zip(cells)
        .map(|(name, cells)| typed_column(name, cells, &mut notes))
        .collect();
    if n_rows == 0 {
        notes.push(NO_ROWS_NOTE.to_string());
    }
    let mut table = TextTable {
        format: TableFormat::Csv,
        columns,
        n_rows,
        meta: serde_json::Value::Null,
        comments,
        notes,
    };
    apply_unit_comments(&mut table);
    Ok(table)
}

enum Line<'a> {
    Comment(&'a str),
    Record { fields: Vec<String>, line: usize },
}

struct Cursor<'a> {
    rest: &'a str,
    line: usize,
}

impl<'a> Cursor<'a> {
    fn next_line(&mut self, label: &str) -> Result<Option<Line<'a>>> {
        while !self.rest.is_empty() {
            let (physical, after) = self
                .rest
                .split_once(LINE_FEED)
                .unwrap_or((self.rest, ""));
            let trimmed = physical.trim();
            if !trimmed.is_empty() && !trimmed.starts_with(COMMENT) {
                let line = self.line;
                let fields = self.record(label)?;
                return Ok(Some(Line::Record { fields, line }));
            }
            self.rest = after;
            self.line += 1;
            if !trimmed.is_empty() {
                return Ok(Some(Line::Comment(trimmed)));
            }
        }
        Ok(None)
    }

    fn record(&mut self, label: &str) -> Result<Vec<String>> {
        let first_line = self.line;
        let mut fields = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        let mut end = self.rest.len();
        let mut chars = self.rest.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            let next = chars.peek().map(|&(_, next)| next);
            match (quoted, c) {
                (true, QUOTE) if next == Some(QUOTE) => {
                    field.push(QUOTE);
                    chars.next();
                }
                (true, QUOTE) => quoted = false,
                (true, CARRIAGE_RETURN) if next == Some(LINE_FEED) => {}
                (true, LINE_FEED) => {
                    field.push(LINE_FEED);
                    self.line += 1;
                }
                (false, QUOTE) if field.trim().is_empty() => {
                    field.clear();
                    quoted = true;
                }
                (false, SEPARATOR) => {
                    fields.push(field.trim().to_string());
                    field.clear();
                }
                (false, LINE_FEED) => {
                    end = at + LINE_FEED.len_utf8();
                    break;
                }
                (_, c) => field.push(c),
            }
        }
        if quoted {
            bail!("{label}, line {first_line}: unterminated quoted field");
        }
        fields.push(field.trim().to_string());
        self.rest = &self.rest[end..];
        self.line += 1;
        Ok(fields)
    }
}

fn comment_pair(comment: &str) -> Option<(String, String)> {
    let (key, value) = comment
        .trim_start_matches(COMMENT)
        .split_once(KEY_SEPARATOR)?;
    let key = key.trim();
    (!key.is_empty()).then(|| (key.to_string(), value.trim().to_string()))
}

fn column_names(header: Vec<String>, label: &str, notes: &mut Vec<String>) -> Result<Vec<String>> {
    let names: Vec<String> = header
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            if !name.is_empty() {
                return name;
            }
            let position = index + 1;
            notes.push(format!(
                "column {position} has no name in the header and is called 'column_{position}'"
            ));
            format!("column_{position}")
        })
        .collect();
    let mut positions = HashMap::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let position = index + 1;
        if let Some(first) = positions.insert(name.as_str(), position) {
            bail!("{label}: column name '{name}' appears twice (columns {first} and {position})");
        }
    }
    Ok(names)
}

fn filled(cells: &[String]) -> impl Iterator<Item = &String> {
    cells.iter().filter(|cell| !cell.is_empty())
}

fn is_bool_literal(cell: &str) -> bool {
    BOOL_LITERALS
        .iter()
        .any(|literal| cell.eq_ignore_ascii_case(literal))
}

fn typed_column(name: String, cells: Vec<String>, notes: &mut Vec<String>) -> TableColumn {
    let numbers: Option<Vec<f64>> = cells
        .iter()
        .map(|cell| {
            if cell.is_empty() {
                Some(f64::NAN)
            } else {
                cell.parse().ok()
            }
        })
        .collect();
    let integers = filled(&cells).next().is_some()
        && filled(&cells).all(|cell| is_integer_literal(cell));
    let (datatype, cells) = match numbers {
        Some(_) if integers && filled(&cells).any(|cell| beyond_exact_integers(cell)) => {
            notes.push(exact_integer_note(&name));
            (DATATYPE_INTEGER, TableCells::Text(cells))
        }
        Some(values) if integers => (DATATYPE_INTEGER, TableCells::F64(values)),
        Some(values) => (DATATYPE_FLOAT, TableCells::F64(values)),
        None if filled(&cells).all(|cell| is_bool_literal(cell)) => {
            (DATATYPE_BOOL, TableCells::Text(cells))
        }
        None => (DATATYPE_STRING, TableCells::Text(cells)),
    };
    TableColumn {
        name,
        unit: None,
        datatype: datatype.to_string(),
        cells,
    }
}

fn apply_unit_comments(table: &mut TextTable) {
    let units: Vec<(usize, String)> = table
        .comments
        .iter()
        .filter(|(_, unit)| !unit.is_empty())
        .filter_map(|(key, unit)| {
            let name = key.strip_prefix(UNIT_KEY_PREFIX)?.trim_start();
            Some((table.column_index(name)?, unit.clone()))
        })
        .collect();
    for (index, unit) in units {
        table.columns[index].unit = Some(unit);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{assert_f64_cells, assert_text_cells, comment_pairs, MIXED_TYPES_CSV};
    use super::super::{TableCells, TableColumn, TableFormat, TextTable, MAX_TABLE_CELLS};
    use super::parse_csv;
    use crate::core::analysis::spectrum1d::Spectrum1d;

    const LABEL: &str = "t.csv";
    const SPECTRUM_EXPORTS: [(&str, usize); 5] = [
        ("document_export.csv", 12),
        ("air_document_export.csv", 3),
        ("region_jy_export.csv", 6),
        ("region_mean_export.csv", 6),
        ("region_sum_export.csv", 6),
    ];
    const REGION_EXPORT_PREFIX: &str = "region_";

    fn parsed(text: &str) -> TextTable {
        parse_csv(text, LABEL, MAX_TABLE_CELLS).unwrap()
    }

    fn refusal(text: &str) -> String {
        parse_csv(text, LABEL, MAX_TABLE_CELLS)
            .unwrap_err()
            .to_string()
    }

    fn column<'a>(table: &'a TextTable, name: &str) -> &'a TableColumn {
        table
            .columns
            .iter()
            .find(|column| column.name == name)
            .unwrap_or_else(|| panic!("no column '{name}'"))
    }

    fn names(table: &TextTable) -> Vec<&str> {
        table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect()
    }

    fn datatypes(table: &TextTable) -> Vec<&str> {
        table
            .columns
            .iter()
            .map(|column| column.datatype.as_str())
            .collect()
    }

    fn assert_same_columns(left: &TextTable, right: &TextTable) {
        assert_eq!(left.n_rows, right.n_rows);
        assert_eq!(names(left), names(right));
        assert_eq!(datatypes(left), datatypes(right));
        for (a, b) in left.columns.iter().zip(&right.columns) {
            assert_eq!(a.unit, b.unit, "column '{}'", a.name);
            match (&a.cells, &b.cells) {
                (TableCells::F64(values), _) => assert_f64_cells(b, values),
                (TableCells::Text(_), _) => assert_eq!(a.cells, b.cells, "column '{}'", a.name),
            }
        }
    }

    fn spectrum_export(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("spectrum1d")
            .join(name);
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_csv_reads_float_integer_bool_and_text_columns_with_their_datatypes() {
        let table = parsed(MIXED_TYPES_CSV);
        assert_eq!(table.n_rows, 2);
        assert_eq!(table.format, TableFormat::Csv);
        assert_eq!(table.meta, serde_json::Value::Null);
        assert!(table.comments.is_empty(), "{:?}", table.comments);
        assert!(table.notes.is_empty(), "{:?}", table.notes);
        assert_eq!(names(&table), ["name", "ra", "dec", "n", "ok", "flux"]);
        assert_eq!(
            datatypes(&table),
            ["string", "float64", "float64", "int64", "bool", "float64"]
        );
        assert!(table.columns.iter().all(|column| column.unit.is_none()));
        assert_text_cells(&table.columns[0], &["a", "b"]);
        assert_f64_cells(&table.columns[1], &[10.5, 11.0]);
        assert_f64_cells(&table.columns[2], &[-20.25, 0.5]);
        assert_f64_cells(&table.columns[3], &[3.0, -4.0]);
        assert_text_cells(&table.columns[4], &["true", "False"]);
        assert_f64_cells(&table.columns[5], &[1.5e-17, f64::NAN]);
    }

    #[test]
    fn a_column_of_only_missing_cells_is_float64_and_nan_and_infinity_tokens_parse() {
        let table = parsed("a,b,c\n,nan,inf\n,NaN,-Infinity\n,1,2\n");
        assert_eq!(table.n_rows, 3);
        assert_eq!(datatypes(&table), ["float64", "float64", "float64"]);
        assert_f64_cells(column(&table, "a"), &[f64::NAN, f64::NAN, f64::NAN]);
        assert_f64_cells(column(&table, "b"), &[f64::NAN, f64::NAN, 1.0]);
        assert_f64_cells(
            column(&table, "c"),
            &[f64::INFINITY, f64::NEG_INFINITY, 2.0],
        );
    }

    #[test]
    fn integers_beyond_2_53_stay_text_so_that_identifiers_stay_exact() {
        let table = parsed(
            "big_id,small,edge\n12345678901234567,7,9007199254740992\n12345678901234568,8,-9007199254740992\n",
        );
        assert_text_cells(
            column(&table, "big_id"),
            &["12345678901234567", "12345678901234568"],
        );
        assert_f64_cells(column(&table, "small"), &[7.0, 8.0]);
        assert_f64_cells(
            column(&table, "edge"),
            &[9007199254740992.0, -9007199254740992.0],
        );
        assert_eq!(datatypes(&table), ["int64", "int64", "int64"]);
        assert_eq!(
            table.notes,
            ["column 'big_id': integers beyond 2^53 are kept as text so that identifiers stay exact"]
        );

        let above_the_limit = parsed("edge\n9007199254740993\n");
        assert_text_cells(column(&above_the_limit, "edge"), &["9007199254740993"]);
        assert_eq!(column(&above_the_limit, "edge").datatype, "int64");

        let above_i128 = parsed("id\n123456789012345678901234567890123456789012\n");
        assert_text_cells(
            column(&above_i128, "id"),
            &["123456789012345678901234567890123456789012"],
        );
        assert_eq!(column(&above_i128, "id").datatype, "int64");
    }

    #[test]
    fn comment_lines_before_the_header_become_key_value_pairs_and_the_others_are_counted() {
        let table = parsed(
            "#Table1\n# file: a.fits\n# note: x: y\n#\n# empty_value:\nra,dec\n1,2\n# trailing: skipped\n3,4\n",
        );
        assert_eq!(
            table.comments,
            comment_pairs(&[("file", "a.fits"), ("note", "x: y"), ("empty_value", "")])
        );
        assert_eq!(
            table.notes,
            ["comment lines before the header without a 'key: value' pair: 2 (ignored)"]
        );
        assert_eq!(table.n_rows, 2);
        assert_f64_cells(column(&table, "ra"), &[1.0, 3.0]);
        assert_f64_cells(column(&table, "dec"), &[2.0, 4.0]);
    }

    #[test]
    fn unit_comment_lines_give_their_column_a_unit() {
        let table = parsed(
            "# unit flux_ujy: uJy\n# unit FLUX_ERR_UJY: uJy\n# unit missing: K\n# unit note:\nid,flux_ujy,flux_err_ujy,note\ns1,2.5,0.25,x\n",
        );
        let units: Vec<Option<&str>> = table
            .columns
            .iter()
            .map(|column| column.unit.as_deref())
            .collect();
        assert_eq!(units, [None, Some("uJy"), Some("uJy"), None]);
        assert_eq!(
            table.comments,
            comment_pairs(&[
                ("unit flux_ujy", "uJy"),
                ("unit FLUX_ERR_UJY", "uJy"),
                ("unit missing", "K"),
                ("unit note", ""),
            ])
        );
        assert!(table.notes.is_empty(), "{:?}", table.notes);
    }

    #[test]
    fn quoted_fields_keep_commas_doubled_quotes_and_line_breaks() {
        let table = parsed(
            "id,source,notes\r\n1,\"{\"\"kind\"\":\"\"pixel\"\",\"\"x\"\":1}\",\"a, b\"\r\n2,\"line one\r\nline two\",z\r\n",
        );
        assert_eq!(table.n_rows, 2);
        assert_f64_cells(column(&table, "id"), &[1.0, 2.0]);
        assert_text_cells(
            column(&table, "source"),
            &["{\"kind\":\"pixel\",\"x\":1}", "line one\nline two"],
        );
        assert_text_cells(column(&table, "notes"), &["a, b", "z"]);
    }

    #[test]
    fn lf_crlf_and_a_byte_order_mark_give_the_same_table() {
        let crlf = MIXED_TYPES_CSV.replace('\n', "\r\n");
        let marked = format!("\u{feff}{crlf}");
        let from_lf = parsed(MIXED_TYPES_CSV);
        assert_eq!(from_lf.n_rows, 2);
        assert_same_columns(&from_lf, &parsed(&crlf));
        assert_same_columns(&from_lf, &parsed(&marked));
    }

    #[test]
    fn a_row_with_the_wrong_number_of_fields_is_refused_with_its_line() {
        assert_eq!(
            refusal("a,b,c\n1,2,3\n4,5\n"),
            "t.csv, line 3: 2 fields, the header has 3"
        );
        assert_eq!(
            refusal("# file: a.fits\n\na,b\n1,\"two\nlines\"\n3,4,5\n"),
            "t.csv, line 6: 3 fields, the header has 2"
        );
    }

    #[test]
    fn an_open_quote_a_missing_header_and_a_repeated_column_name_are_refused() {
        assert_eq!(
            refusal("a,b\n1,\"open\n2,3\n"),
            "t.csv, line 2: unterminated quoted field"
        );
        assert_eq!(refusal(""), "t.csv holds no header row");
        assert_eq!(refusal("# only: comments\n\n"), "t.csv holds no header row");
        assert_eq!(
            refusal("ra,dec,ra\n1,2,3\n"),
            "t.csv: column name 'ra' appears twice (columns 1 and 3)"
        );
        let cased = parsed("Flux,flux\n1,2\n");
        assert_eq!(names(&cased), ["Flux", "flux"]);
        assert_eq!(cased.n_rows, 1);
    }

    #[test]
    fn an_empty_header_name_is_numbered_and_a_table_without_rows_is_valid() {
        let table = parsed("a,,c\n");
        assert_eq!(names(&table), ["a", "column_2", "c"]);
        assert_eq!(table.n_rows, 0);
        assert_eq!(datatypes(&table), ["float64", "float64", "float64"]);
        for column in &table.columns {
            assert_f64_cells(column, &[]);
        }
        assert_eq!(
            table.notes,
            [
                "column 2 has no name in the header and is called 'column_2'",
                "the table has no data rows"
            ]
        );
    }

    #[test]
    fn the_cell_cap_refuses_a_table_while_it_is_read() {
        let text = "a,b\n1,2\n3,4\n";
        assert_eq!(
            parse_csv(text, LABEL, 3).unwrap_err().to_string(),
            "t.csv holds more than 3 cells (rows x columns): the table is too large to load"
        );
        assert_eq!(parse_csv(text, LABEL, 4).unwrap().n_rows, 2);
        assert_eq!(
            parse_csv("a,b\n1,2\n3,4\n5\n", LABEL, 3)
                .unwrap_err()
                .to_string(),
            "t.csv holds more than 3 cells (rows x columns): the table is too large to load"
        );
    }

    #[test]
    fn the_committed_spectrum_exports_read_like_spectrum1d_from_csv() {
        for (name, rows) in SPECTRUM_EXPORTS {
            let text = spectrum_export(name);
            let table = parse_csv(&text, name, MAX_TABLE_CELLS).unwrap();
            let spectrum = Spectrum1d::from_csv("x.csv", &text).unwrap();
            assert_eq!(table.n_rows, rows, "{name}");
            let wavelength = table
                .column("wavelength_air_um")
                .or_else(|| table.column("wavelength_vacuum_um"))
                .unwrap();
            assert_f64_cells(wavelength, &spectrum.axis.values);
            assert_f64_cells(column(&table, "flux"), &spectrum.flux);
            if let Some(err) = table.column("err") {
                assert_f64_cells(err, spectrum.err.as_deref().unwrap());
            } else {
                assert!(spectrum.err.is_none(), "{name}");
            }
            assert_eq!(table.comments[0].0, "file", "{name}");
            assert!(!column(&table, "flux_unit").is_numeric(), "{name}");
            if name.starts_with(REGION_EXPORT_PREFIX) {
                assert_eq!(column(&table, "bg_subtracted").datatype, "bool", "{name}");
            }
        }
    }
}
