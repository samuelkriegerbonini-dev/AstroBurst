use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use serde_json::Value as Json;
use serde_yaml::Value as Yaml;

use super::{
    beyond_exact_integers, cell_cap_error, exact_integer_note, is_integer_literal, TableCells,
    TableColumn, TableFormat, TextTable, BYTE_ORDER_MARK, COMMENT, ECSV_SIGNATURE, NO_ROWS_NOTE,
};
use crate::infra::asdf::tree::untag;

const QUOTE: char = '"';
const SPACE: char = ' ';
const COMMA: char = ',';
const VERSION_SEPARATOR: char = '.';
const VERSIONS_READ_SILENTLY: [&str; 2] = ["1.0", "0.9"];
const KEY_DATATYPE: &str = "datatype";
const KEY_DELIMITER: &str = "delimiter";
const KEY_META: &str = "meta";
const KEY_NAME: &str = "name";
const KEY_UNIT: &str = "unit";
const KEY_SUBTYPE: &str = "subtype";
const SERIALIZED_COLUMNS_KEY: &str = "__serialized_columns__";
const FLOAT_DATATYPES: [&str; 4] = ["float16", "float32", "float64", "float128"];
const INTEGER_DATATYPES: [&str; 8] = [
    "int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64",
];
const TEXT_DATATYPES: [&str; 2] = ["bool", "string"];
const META_DROPPED_NOTE: &str = "the meta of the ECSV header is not a mapping and was dropped";

pub(crate) fn parse_ecsv(text: &str, label: &str, max_cells: usize) -> Result<TextTable> {
    let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(text);
    let mut lines = text
        .lines()
        .map(str::trim)
        .zip(1_usize..)
        .filter(|(line, _)| !line.is_empty())
        .peekable();
    let version = lines
        .next()
        .and_then(|(line, _)| ecsv_version(line))
        .ok_or_else(|| anyhow!("{label}: the first line must be '# %ECSV <version>'"))?;
    let mut notes = Vec::new();
    if !VERSIONS_READ_SILENTLY.contains(&version) {
        notes.push(format!("ECSV version {version}: read with the 1.0 rules"));
    }
    let mut header_lines = Vec::new();
    while let Some((line, _)) = lines.next_if(|(line, _)| line.starts_with(COMMENT)) {
        header_lines.push(&line[COMMENT.len_utf8()..]);
    }
    let Header {
        mut columns,
        delimiter,
        meta,
    } = parse_header(&header_lines, label, &mut notes)?;

    let (names_line, names_line_number) = lines
        .next()
        .ok_or_else(|| anyhow!("{label}: the ECSV header is followed by no column-name line"))?;
    let listed = split_fields(names_line, delimiter, label, names_line_number)?;
    if !listed.iter().eq(columns.iter().map(|column| &column.name)) {
        let declared: Vec<&str> = columns.iter().map(|column| column.name.as_str()).collect();
        bail!(
            "{label}: the column-name line lists [{}] but the header declares [{}]",
            listed.join(", "),
            declared.join(", ")
        );
    }

    let mut n_rows = 0_usize;
    for (line, line_number) in lines {
        if line.starts_with(COMMENT) {
            continue;
        }
        let fields = split_fields(line, delimiter, label, line_number)?;
        if fields.len() != columns.len() {
            bail!(
                "{label}, line {line_number}: {} fields, the header declares {} columns",
                fields.len(),
                columns.len()
            );
        }
        n_rows += 1;
        if n_rows.saturating_mul(columns.len()) > max_cells {
            return Err(cell_cap_error(label, max_cells));
        }
        for (column, cell) in columns.iter_mut().zip(fields) {
            if column.kind != CellKind::Text {
                let number = if cell.is_empty() {
                    f64::NAN
                } else {
                    cell.parse().map_err(|_| {
                        anyhow!(
                            "{label}, line {line_number}: column '{}' ({}) holds '{cell}', which is not a number",
                            column.name,
                            column.datatype
                        )
                    })?
                };
                column.numbers.push(number);
            }
            if column.kind != CellKind::Float {
                column.written.push(cell);
            }
        }
    }

    let columns = columns
        .into_iter()
        .map(|column| column.finished(&mut notes))
        .collect();
    if n_rows == 0 {
        notes.push(NO_ROWS_NOTE.to_string());
    }
    Ok(TextTable {
        format: TableFormat::Ecsv,
        columns,
        n_rows,
        meta,
        comments: Vec::new(),
        notes,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum CellKind {
    Float,
    Integer,
    Text,
}

struct EcsvColumn {
    name: String,
    unit: Option<String>,
    datatype: String,
    kind: CellKind,
    numbers: Vec<f64>,
    written: Vec<String>,
}

struct Header {
    columns: Vec<EcsvColumn>,
    delimiter: char,
    meta: Json,
}

impl EcsvColumn {
    fn finished(self, notes: &mut Vec<String>) -> TableColumn {
        let beyond_exact =
            |cell: &String| is_integer_literal(cell) && beyond_exact_integers(cell);
        let cells = match self.kind {
            CellKind::Float => TableCells::F64(self.numbers),
            CellKind::Integer if self.written.iter().any(beyond_exact) => {
                notes.push(exact_integer_note(&self.name));
                TableCells::Text(self.written)
            }
            CellKind::Integer => TableCells::F64(self.numbers),
            CellKind::Text => {
                if !TEXT_DATATYPES.contains(&self.datatype.as_str()) {
                    notes.push(format!(
                        "column '{}' has datatype {}, which is kept as text",
                        self.name, self.datatype
                    ));
                }
                TableCells::Text(self.written)
            }
        };
        TableColumn {
            name: self.name,
            unit: self.unit,
            datatype: self.datatype,
            cells,
        }
    }
}

fn ecsv_version(first_line: &str) -> Option<&str> {
    let version = first_line
        .strip_prefix(COMMENT)?
        .trim_start_matches(SPACE)
        .strip_prefix(ECSV_SIGNATURE)?
        .strip_prefix(SPACE)?
        .trim_start_matches(SPACE);
    let (major, minor) = version.split_once(VERSION_SEPARATOR)?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    (digits(major) && digits(minor)).then_some(version)
}

fn dedented(lines: &[&str]) -> String {
    let common_indentation = lines
        .iter()
        .filter(|line| !line.is_empty())
        .map(|line| line.len() - line.trim_start_matches(SPACE).len())
        .min()
        .unwrap_or(0);
    let dedented: Vec<&str> = lines
        .iter()
        .map(|line| line.get(common_indentation..).unwrap_or_default())
        .collect();
    dedented.join("\n")
}

fn parse_header(lines: &[&str], label: &str, notes: &mut Vec<String>) -> Result<Header> {
    let document: Yaml = serde_yaml::from_str(&dedented(lines))
        .map_err(|e| anyhow!("{label}: the ECSV header is not valid YAML ({e})"))?;
    let document = untag(&document);
    let entries = document
        .get(KEY_DATATYPE)
        .map(untag)
        .and_then(Yaml::as_sequence)
        .ok_or_else(|| anyhow!("{label}: the ECSV header has no 'datatype' list"))?;
    let mut columns = Vec::with_capacity(entries.len());
    let mut names = HashSet::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let entry = untag(entry);
        let text_of = |key: &str| entry.get(key).map(untag).and_then(Yaml::as_str);
        let name = text_of(KEY_NAME)
            .ok_or_else(|| anyhow!("{label}: datatype entry {} has no name", index + 1))?;
        let datatype = text_of(KEY_DATATYPE)
            .ok_or_else(|| anyhow!("{label}: column '{name}' has no datatype"))?;
        if let Some(subtype) = entry.get(KEY_SUBTYPE) {
            bail!(
                "{label}: column '{name}' has subtype '{}': multi-dimensional and variable-length columns are not read",
                yaml_text(subtype)
            );
        }
        if !names.insert(name) {
            bail!("{label}: column name '{name}' appears twice");
        }
        columns.push(EcsvColumn {
            name: name.to_string(),
            unit: text_of(KEY_UNIT).map(str::to_string),
            datatype: datatype.to_string(),
            kind: cell_kind(datatype),
            numbers: Vec::new(),
            written: Vec::new(),
        });
    }
    let delimiter = match document.get(KEY_DELIMITER).map(yaml_text).as_deref() {
        None | Some(" ") => SPACE,
        Some(",") => COMMA,
        Some(other) => bail!("{label}: ECSV delimiter must be a space or a comma, found '{other}'"),
    };
    let meta = document
        .get(KEY_META)
        .map_or(Json::Null, |meta| meta_json(meta, notes));
    Ok(Header {
        columns,
        delimiter,
        meta,
    })
}

fn cell_kind(datatype: &str) -> CellKind {
    if FLOAT_DATATYPES.contains(&datatype) {
        CellKind::Float
    } else if INTEGER_DATATYPES.contains(&datatype) {
        CellKind::Integer
    } else {
        CellKind::Text
    }
}

fn split_fields(line: &str, delimiter: char, label: &str, line_number: usize) -> Result<Vec<String>> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut inside_quotes = false;
    let mut was_quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (inside_quotes, c) {
            (true, QUOTE) if chars.peek() == Some(&QUOTE) => {
                field.push(QUOTE);
                chars.next();
            }
            (true, QUOTE) => inside_quotes = false,
            (false, QUOTE) if field.is_empty() && !was_quoted => {
                inside_quotes = true;
                was_quoted = true;
            }
            (false, c) if c == delimiter => {
                fields.push(closed_field(&field, was_quoted));
                field.clear();
                was_quoted = false;
                while chars.next_if_eq(&SPACE).is_some() {}
            }
            (_, c) => field.push(c),
        }
    }
    if inside_quotes {
        bail!("{label}, line {line_number}: unterminated quoted field");
    }
    fields.push(closed_field(&field, was_quoted));
    Ok(fields)
}

fn closed_field(field: &str, was_quoted: bool) -> String {
    if was_quoted {
        field.to_string()
    } else {
        field.trim().to_string()
    }
}

fn yaml_text(value: &Yaml) -> String {
    match untag(value) {
        Yaml::String(text) => text.clone(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn meta_entries(meta: &Yaml) -> Option<Vec<(&Yaml, &Yaml)>> {
    match untag(meta) {
        Yaml::Mapping(mapping) => Some(mapping.iter().collect()),
        Yaml::Sequence(items) => items
            .iter()
            .map(|item| match untag(item) {
                Yaml::Mapping(mapping) if mapping.len() == 1 => mapping.iter().next(),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

fn meta_json(meta: &Yaml, notes: &mut Vec<String>) -> Json {
    let Some(entries) = meta_entries(meta) else {
        notes.push(META_DROPPED_NOTE.to_string());
        return Json::Null;
    };
    let mut object = serde_json::Map::new();
    for (key, value) in entries {
        let key = yaml_text(key);
        if key == SERIALIZED_COLUMNS_KEY {
            continue;
        }
        if object.contains_key(&key) {
            notes.push(format!(
                "meta key '{key}' appears more than once: the first value is kept"
            ));
        } else {
            object.insert(key, json_of(value));
        }
    }
    Json::Object(object)
}

fn json_of(value: &Yaml) -> Json {
    match value {
        Yaml::Null => Json::Null,
        Yaml::Bool(flag) => Json::Bool(*flag),
        Yaml::String(text) => Json::String(text.clone()),
        Yaml::Number(number) => number
            .as_i64()
            .map(Json::from)
            .or_else(|| number.as_u64().map(Json::from))
            .or_else(|| {
                number
                    .as_f64()
                    .and_then(serde_json::Number::from_f64)
                    .map(Json::Number)
            })
            .unwrap_or(Json::Null),
        Yaml::Sequence(items) => Json::Array(items.iter().map(json_of).collect()),
        Yaml::Mapping(mapping) => Json::Object(
            mapping
                .iter()
                .map(|(key, value)| (yaml_text(key), json_of(value)))
                .collect(),
        ),
        Yaml::Tagged(tagged) => json_of(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::{assert_f64_cells, assert_text_cells};
    use super::super::{TableFormat, TextTable, MAX_TABLE_CELLS};
    use super::parse_ecsv;

    const LABEL: &str = "t.ecsv";
    const WHITE_LIGHT_BY_ORDER: &str = "\
# %ECSV 1.0
# ---
# datatype:
# - {name: MJD_UTC, datatype: float64}
# - {name: BJD_TDB, datatype: float64}
# - {name: whitelight_flux_order_1, unit: MJy, datatype: float64}
# - {name: whitelight_flux_order_2, unit: MJy, datatype: float64}
# meta: !!omap
# - {instrument: NIRISS}
# - {exp_type: NIS_SOSS}
# - {pupil: null}
# - {target_name: TEST STAR}
# - {number_of_integrations: 2}
# - __serialized_columns__:
#     whitelight_flux_order_1:
#       __class__: astropy.units.quantity.Quantity
#       unit: &id001 !astropy.units.Unit {unit: MJy}
#       value: !astropy.table.SerializedColumn {name: whitelight_flux_order_1}
#     whitelight_flux_order_2:
#       __class__: astropy.units.quantity.Quantity
#       unit: *id001
#       value: !astropy.table.SerializedColumn {name: whitelight_flux_order_2}
# schema: astropy-2.0
MJD_UTC BJD_TDB whitelight_flux_order_1 whitelight_flux_order_2
60000.5 60000.50125 1500.25 14000.125
60000.501 60000.50225 1500.5 -14000.25
";
    const MASKED_QUOTED_AND_BIG_INTEGERS: &str = r#"# %ECSV 1.0
# ---
# datatype:
# - {name: id, datatype: int64}
# - {name: small, datatype: int32}
# - {name: flux, unit: Jy, datatype: float64}
# - {name: name, datatype: string}
# - {name: ok, datatype: bool}
# - {name: inf, datatype: float64}
# meta: !!omap
# - {target: X 1}
# - {n: 3}
# schema: astropy-2.0
id small flux name ok inf
12345678901234567 1 1.5 "a b" True inf
7 2 nan "q""uote" False -inf
-3 3 "" "" True 0.1
"#;
    const COMMA_DELIMITED: &str = r#"# %ECSV 1.0
# ---
# datatype:
# - {name: id, datatype: int64}
# - {name: flux, unit: Jy, datatype: float64}
# - {name: name, datatype: string}
# delimiter: ','
# schema: astropy-2.0
id,flux,name
1,1.5,a b
2,,"x, y"
3, 2.5 ,
"#;
    const BLOCK_STYLE_VERSION_0_9: &str = "\
# %ECSV 0.9
# ---
# datatype:
# - name: t
#   datatype: float64
#   unit: d
#   description: mid time
# - name: f
#   datatype: float32
# meta:
#   observer: someone
#   nested: {a: 1, b: [1, 2]}
t f
1.5 2.5
";
    const LEGACY_HEADER: [&str; 5] = [
        "datatype:",
        "- {name: MJD, datatype: float64}",
        "- {name: whitelight_flux, datatype: float64}",
        "meta: !!omap",
        "- {detector: MIRIMAGE}",
    ];
    const TWO_FLOAT_COLUMNS: [&str; 3] = [
        "datatype:",
        "- {name: a, datatype: float64}",
        "- {name: b, datatype: float64}",
    ];

    fn parsed(text: &str) -> TextTable {
        parse_ecsv(text, LABEL, MAX_TABLE_CELLS).unwrap()
    }

    fn refusal(text: &str) -> String {
        parse_ecsv(text, LABEL, MAX_TABLE_CELLS)
            .unwrap_err()
            .to_string()
    }

    fn document(header: &[&str], rows: &[&str]) -> String {
        let mut lines = vec!["# %ECSV 1.0".to_string(), "# ---".to_string()];
        lines.extend(header.iter().map(|line| format!("# {line}")));
        lines.extend(rows.iter().map(|row| row.to_string()));
        lines.join("\n") + "\n"
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

    fn units(table: &TextTable) -> Vec<Option<&str>> {
        table
            .columns
            .iter()
            .map(|column| column.unit.as_deref())
            .collect()
    }

    #[test]
    fn an_astropy_header_with_an_ordered_map_units_anchors_and_class_tags_reads_like_astropy() {
        let table = parsed(WHITE_LIGHT_BY_ORDER);
        assert_eq!(table.format, TableFormat::Ecsv);
        assert_eq!(
            names(&table),
            [
                "MJD_UTC",
                "BJD_TDB",
                "whitelight_flux_order_1",
                "whitelight_flux_order_2"
            ]
        );
        assert_eq!(
            datatypes(&table),
            ["float64", "float64", "float64", "float64"]
        );
        assert_eq!(units(&table), [None, None, Some("MJy"), Some("MJy")]);
        assert_eq!(table.n_rows, 2);
        assert_f64_cells(&table.columns[0], &[60000.5, 60000.501]);
        assert_f64_cells(&table.columns[1], &[60000.50125, 60000.50225]);
        assert_f64_cells(&table.columns[2], &[1500.25, 1500.5]);
        assert_f64_cells(&table.columns[3], &[14000.125, -14000.25]);
        assert_eq!(
            table.meta,
            json!({
                "instrument": "NIRISS",
                "exp_type": "NIS_SOSS",
                "pupil": null,
                "target_name": "TEST STAR",
                "number_of_integrations": 2
            })
        );
        assert!(table.comments.is_empty(), "{:?}", table.comments);
        assert!(table.notes.is_empty(), "{:?}", table.notes);

        let crlf = WHITE_LIGHT_BY_ORDER.replace('\n', "\r\n");
        assert_eq!(parsed(&crlf), table);
        assert_eq!(parsed(&format!("\u{feff}{crlf}")), table);
        assert_eq!(parsed(&format!("\u{feff}{WHITE_LIGHT_BY_ORDER}")), table);
    }

    #[test]
    fn the_legacy_layout_without_units_and_a_one_row_table_are_plain_tables() {
        let table = parsed(&document(
            &LEGACY_HEADER,
            &["MJD whitelight_flux", "60126.5 2.25", "60126.75 2.5"],
        ));
        assert_eq!(table.n_rows, 2);
        assert_eq!(names(&table), ["MJD", "whitelight_flux"]);
        assert_eq!(units(&table), [None, None]);
        assert_f64_cells(&table.columns[0], &[60126.5, 60126.75]);
        assert_f64_cells(&table.columns[1], &[2.25, 2.5]);
        assert_eq!(table.meta, json!({"detector": "MIRIMAGE"}));
        assert!(table.notes.is_empty(), "{:?}", table.notes);

        let one_row = parsed(&document(
            &LEGACY_HEADER,
            &["MJD whitelight_flux", "60599.25 3.0"],
        ));
        assert_eq!(one_row.n_rows, 1);
        assert_f64_cells(&one_row.columns[0], &[60599.25]);
        assert_f64_cells(&one_row.columns[1], &[3.0]);
        assert!(one_row.notes.is_empty(), "{:?}", one_row.notes);

        let spaced = parsed(&document(
            &LEGACY_HEADER,
            &[
                "",
                "MJD whitelight_flux",
                "",
                "60126.5   2.25",
                "# between the rows",
                "  60126.75 2.5  ",
            ],
        ));
        assert_eq!(spaced, table);
    }

    #[test]
    fn masked_cells_quoted_strings_booleans_and_big_integers_follow_astropy() {
        let table = parsed(MASKED_QUOTED_AND_BIG_INTEGERS);
        assert_eq!(table.n_rows, 3);
        assert_eq!(names(&table), ["id", "small", "flux", "name", "ok", "inf"]);
        assert_eq!(
            datatypes(&table),
            ["int64", "int32", "float64", "string", "bool", "float64"]
        );
        assert_eq!(units(&table), [None, None, Some("Jy"), None, None, None]);
        assert_text_cells(&table.columns[0], &["12345678901234567", "7", "-3"]);
        assert_f64_cells(&table.columns[1], &[1.0, 2.0, 3.0]);
        assert_f64_cells(&table.columns[2], &[1.5, f64::NAN, f64::NAN]);
        assert_text_cells(&table.columns[3], &["a b", "q\"uote", ""]);
        assert_text_cells(&table.columns[4], &["True", "False", "True"]);
        assert_f64_cells(&table.columns[5], &[f64::INFINITY, f64::NEG_INFINITY, 0.1]);
        assert_eq!(table.meta, json!({"target": "X 1", "n": 3}));
        assert_eq!(
            table.notes,
            ["column 'id': integers beyond 2^53 are kept as text so that identifiers stay exact"]
        );
    }

    #[test]
    fn a_comma_delimiter_trims_fields_and_reads_empty_cells_as_missing() {
        let table = parsed(COMMA_DELIMITED);
        assert_eq!(table.n_rows, 3);
        assert_eq!(datatypes(&table), ["int64", "float64", "string"]);
        assert_eq!(units(&table), [None, Some("Jy"), None]);
        assert_f64_cells(&table.columns[0], &[1.0, 2.0, 3.0]);
        assert_f64_cells(&table.columns[1], &[1.5, f64::NAN, 2.5]);
        assert_text_cells(&table.columns[2], &["a b", "x, y", ""]);
        assert_eq!(table.meta, serde_json::Value::Null);
        assert!(table.notes.is_empty(), "{:?}", table.notes);
    }

    #[test]
    fn a_block_style_header_version_0_9_and_a_mapping_meta_are_read() {
        let table = parsed(BLOCK_STYLE_VERSION_0_9);
        assert_eq!(table.n_rows, 1);
        assert_eq!(names(&table), ["t", "f"]);
        assert_eq!(datatypes(&table), ["float64", "float32"]);
        assert_eq!(units(&table), [Some("d"), None]);
        assert_f64_cells(&table.columns[0], &[1.5]);
        assert_f64_cells(&table.columns[1], &[2.5]);
        assert_eq!(
            table.meta,
            json!({"observer": "someone", "nested": {"a": 1, "b": [1, 2]}})
        );
        assert!(table.notes.is_empty(), "{:?}", table.notes);

        let later_version = parsed(&BLOCK_STYLE_VERSION_0_9.replace("%ECSV 0.9", "%ECSV 1.1"));
        assert_eq!(
            later_version.notes,
            ["ECSV version 1.1: read with the 1.0 rules"]
        );
        assert_eq!(later_version.columns, table.columns);
    }

    #[test]
    fn sky_coordinate_columns_written_by_astropy_keep_their_dotted_names() {
        let table = parsed(&document(
            &[
                "datatype:",
                "- {name: label, datatype: int64}",
                "- {name: sky_centroid.ra, unit: deg, datatype: float64}",
                "- {name: sky_centroid.dec, unit: deg, datatype: float64}",
                "meta: !!omap",
                "- __serialized_columns__:",
                "    sky_centroid:",
                "      __class__: astropy.coordinates.sky_coordinate.SkyCoord",
                "      dec: !astropy.table.SerializedColumn",
                "        __class__: astropy.coordinates.angles.core.Latitude",
                "        unit: &id001 !astropy.units.Unit {unit: deg}",
                "        value: !astropy.table.SerializedColumn {name: sky_centroid.dec}",
                "      frame: icrs",
                "      ra: !astropy.table.SerializedColumn",
                "        __class__: astropy.coordinates.angles.core.Longitude",
                "        unit: *id001",
                "        value: !astropy.table.SerializedColumn {name: sky_centroid.ra}",
                "        wrap_angle: !astropy.coordinates.Angle",
                "          unit: *id001",
                "          value: 360.0",
                "      representation_type: spherical",
                "schema: astropy-2.0",
            ],
            &[
                "label sky_centroid.ra sky_centroid.dec",
                "1 10.5 -20.25",
                "2 10.6 -20.2",
            ],
        ));
        assert_eq!(
            names(&table),
            ["label", "sky_centroid.ra", "sky_centroid.dec"]
        );
        assert_eq!(units(&table), [None, Some("deg"), Some("deg")]);
        assert_f64_cells(&table.columns[0], &[1.0, 2.0]);
        assert_f64_cells(&table.columns[1], &[10.5, 10.6]);
        assert_f64_cells(&table.columns[2], &[-20.25, -20.2]);
        assert_eq!(table.meta, json!({}));
        assert!(table.notes.is_empty(), "{:?}", table.notes);
    }

    #[test]
    fn a_meta_that_is_not_a_mapping_is_dropped_with_a_note_and_a_repeated_key_keeps_the_first() {
        let one_column = "- {name: a, datatype: float64}";
        let rows = ["a", "1.0"];

        let sequence = parsed(&document(&["datatype:", one_column, "meta: [1, 2]"], &rows));
        assert_eq!(sequence.meta, serde_json::Value::Null);
        assert_eq!(
            sequence.notes,
            ["the meta of the ECSV header is not a mapping and was dropped"]
        );

        let repeated = parsed(&document(
            &["datatype:", one_column, "meta: !!omap", "- {a: 1}", "- {a: 2}"],
            &rows,
        ));
        assert_eq!(repeated.meta, json!({"a": 1}));
        assert_eq!(
            repeated.notes,
            ["meta key 'a' appears more than once: the first value is kept"]
        );

        let converted = parsed(&document(
            &[
                "datatype:",
                one_column,
                "meta:",
                "  1: one",
                "  ratio: 0.5",
                "  open: .inf",
                "  flag: true",
                "  tagged: !astropy.units.Unit {unit: Jy}",
            ],
            &rows,
        ));
        assert_eq!(
            converted.meta,
            json!({
                "1": "one",
                "ratio": 0.5,
                "open": null,
                "flag": true,
                "tagged": {"unit": "Jy"}
            })
        );
        assert!(converted.notes.is_empty(), "{:?}", converted.notes);
    }

    #[test]
    fn a_complex_column_is_kept_as_text_with_a_note() {
        let table = parsed(&document(
            &["datatype:", "- {name: z, datatype: complex128}"],
            &["z", "(1+2j)"],
        ));
        assert_text_cells(&table.columns[0], &["(1+2j)"]);
        assert_eq!(table.columns[0].datatype, "complex128");
        assert_eq!(
            table.notes,
            ["column 'z' has datatype complex128, which is kept as text"]
        );
    }

    #[test]
    fn broken_headers_are_refused_with_the_reason() {
        let rows = ["a", "1.0"];
        let first_line = "t.ecsv: the first line must be '# %ECSV <version>'";
        assert_eq!(
            refusal("# datatype:\n# - {name: a, datatype: float64}\na\n1.0\n"),
            first_line
        );
        assert_eq!(refusal(""), first_line);
        assert_eq!(
            refusal("# %ECSV\n# datatype:\n# - {name: a, datatype: float64}\na\n1.0\n"),
            first_line
        );
        assert_eq!(
            refusal("# %ECSV 1.x\n# datatype:\n# - {name: a, datatype: float64}\na\n1.0\n"),
            first_line
        );

        let err = refusal(&document(&["datatype: ["], &rows));
        assert!(
            err.starts_with("t.ecsv: the ECSV header is not valid YAML ("),
            "{err}"
        );
        assert_eq!(
            refusal(&document(&["meta: {a: 1}"], &rows)),
            "t.ecsv: the ECSV header has no 'datatype' list"
        );
        assert_eq!(
            refusal(&document(&["datatype:", "- {datatype: float64}"], &rows)),
            "t.ecsv: datatype entry 1 has no name"
        );
        assert_eq!(
            refusal(&document(&["datatype:", "- {name: a}"], &rows)),
            "t.ecsv: column 'a' has no datatype"
        );
        assert_eq!(
            refusal(&document(
                &[
                    "datatype:",
                    "- {name: a, datatype: float64}",
                    "- {name: a, datatype: float64}"
                ],
                &["a a", "1.0 2.0"]
            )),
            "t.ecsv: column name 'a' appears twice"
        );
        assert_eq!(
            refusal(&document(
                &[
                    "datatype:",
                    "- {name: v, datatype: string, subtype: 'float64[2]'}"
                ],
                &["v", "[1.0,2.0]"]
            )),
            "t.ecsv: column 'v' has subtype 'float64[2]': multi-dimensional and variable-length columns are not read"
        );
        assert_eq!(
            refusal(&document(
                &["datatype:", "- {name: a, datatype: float64}", "delimiter: ';'"],
                &rows
            )),
            "t.ecsv: ECSV delimiter must be a space or a comma, found ';'"
        );
        assert_eq!(
            refusal(&document(&TWO_FLOAT_COLUMNS, &[])),
            "t.ecsv: the ECSV header is followed by no column-name line"
        );
        assert_eq!(
            refusal(&document(&TWO_FLOAT_COLUMNS, &["b a", "1.0 2.0"])),
            "t.ecsv: the column-name line lists [b, a] but the header declares [a, b]"
        );
    }

    #[test]
    fn broken_data_lines_are_refused_with_their_line_number() {
        assert_eq!(
            refusal(&document(&TWO_FLOAT_COLUMNS, &["a b", "1.0"])),
            "t.ecsv, line 7: 1 fields, the header declares 2 columns"
        );
        assert_eq!(
            refusal(&document(&TWO_FLOAT_COLUMNS, &["a b", "1.0 abc"])),
            "t.ecsv, line 7: column 'b' (float64) holds 'abc', which is not a number"
        );
        assert_eq!(
            refusal(&document(&TWO_FLOAT_COLUMNS, &["a b", "1.0 \"open"])),
            "t.ecsv, line 7: unterminated quoted field"
        );
        assert_eq!(
            refusal(&document(
                &TWO_FLOAT_COLUMNS,
                &["a b", "1.0 2.0", "", "# skipped", "1.0\t2.0"]
            )),
            "t.ecsv, line 10: 1 fields, the header declares 2 columns"
        );
    }

    #[test]
    fn a_table_without_rows_and_the_cell_cap() {
        let table = parsed(&document(&TWO_FLOAT_COLUMNS, &["a b"]));
        assert_eq!(table.n_rows, 0);
        assert_eq!(names(&table), ["a", "b"]);
        for column in &table.columns {
            assert_f64_cells(column, &[]);
        }
        assert_eq!(table.notes, ["the table has no data rows"]);

        let two_rows = document(&TWO_FLOAT_COLUMNS, &["a b", "1.0 2.0", "3.0 4.0"]);
        assert_eq!(
            parse_ecsv(&two_rows, LABEL, 3).unwrap_err().to_string(),
            "t.ecsv holds more than 3 cells (rows x columns): the table is too large to load"
        );
        assert_eq!(parse_ecsv(&two_rows, LABEL, 4).unwrap().n_rows, 2);
    }
}
