use anyhow::anyhow;

const SIGNS: [char; 2] = ['+', '-'];
const LARGEST_EXACT_INTEGER: u128 = 1 << 53;

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
