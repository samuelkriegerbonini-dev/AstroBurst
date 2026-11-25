use anyhow::{anyhow, bail, Result};
use serde::Serialize;

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

    pub(super) fn column_index(&self, name: &str) -> Option<usize> {
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
