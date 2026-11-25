use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use super::text_table::{TableFormat, TextTable};
use super::{csv, ecsv};

pub const MAX_TEXT_TABLE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_TABLE_CELLS: usize = 20_000_000;
pub(crate) const NO_ROWS_NOTE: &str = "the table has no data rows";
pub(super) const BYTE_ORDER_MARK: char = '\u{feff}';
pub(super) const COMMENT: char = '#';

const CSV_EXTENSION: &str = "csv";
const ECSV_EXTENSION: &str = "ecsv";
pub(super) const ECSV_SIGNATURE: &str = "%ECSV";

pub fn read_text_table(path: &str) -> Result<TextTable> {
    read_text_table_capped(path, MAX_TEXT_TABLE_BYTES)
}

pub(super) fn read_text_table_capped(path: &str, max_bytes: u64) -> Result<TextTable> {
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
