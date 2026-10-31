use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::types::config::CustomFilter;
use crate::types::header::HduHeader;

pub const FILTER_WAVELENGTHS_NM: &[(&str, u32)] = &[
    ("HA", 656),
    ("HALPHA", 656),
    ("H_ALPHA", 656),
    ("OIII", 501),
    ("O3", 501),
    ("SII", 673),
    ("S2", 673),
    ("NII", 658),
    ("HB", 486),
    ("HBETA", 486),
    ("F656N", 656),
    ("F657N", 657),
    ("F658N", 658),
    ("F673N", 673),
    ("F501N", 501),
    ("F502N", 501),
    ("F503N", 503),
    ("F487N", 487),
    ("F469N", 469),
    ("F631N", 631),
    ("F070W", 700),
    ("F090W", 900),
    ("F115W", 1150),
    ("F140M", 1400),
    ("F150W", 1500),
    ("F150W2", 1500),
    ("F162M", 1620),
    ("F164N", 1640),
    ("F182M", 1820),
    ("F187N", 1870),
    ("F200W", 2000),
    ("F210M", 2100),
    ("F212N", 2120),
    ("F250M", 2500),
    ("F277W", 2770),
    ("F300M", 3000),
    ("F322W2", 3220),
    ("F323N", 3230),
    ("F335M", 3350),
    ("F356W", 3560),
    ("F360M", 3600),
    ("F405N", 4050),
    ("F410M", 4100),
    ("F430M", 4300),
    ("F444W", 4440),
    ("F460M", 4600),
    ("F466N", 4660),
    ("F470N", 4700),
    ("F480M", 4800),
    ("F560W", 5600),
    ("F770W", 7700),
    ("F1000W", 10000),
    ("F1130W", 11300),
    ("F1280W", 12800),
    ("F1500W", 15000),
    ("F1800W", 18000),
    ("F2100W", 21000),
    ("F2550W", 25500),
    ("F158M", 1580),
    ("F380M", 3800),
    ("F336W", 336),
    ("F390W", 390),
    ("F435W", 435),
    ("F438W", 438),
    ("F439W", 439),
    ("F450W", 450),
    ("F475W", 475),
    ("F555W", 555),
    ("F606W", 606),
    ("F625W", 625),
    ("F675W", 675),
    ("F702W", 702),
    ("F775W", 775),
    ("F814W", 814),
    ("F062", 629),
    ("F087", 870),
    ("F106", 1057),
    ("F129", 1290),
    ("F146", 1438),
    ("F158", 1575),
    ("F184", 1839),
    ("F213", 2123),
];

const FILTER_HEADER_KEYS: [&str; 6] = ["FILTER", "FILTER1", "FILTER2", "FILTNAM1", "FILTNAM2", "PUPIL"];
const PUPIL_FIRST_KEYS: [&str; 6] = ["PUPIL", "FILTER", "FILTER1", "FILTER2", "FILTNAM1", "FILTNAM2"];
const PUPIL_KEY: &str = "PUPIL";
const CLEAR_TOKEN: &str = "CLEAR";

fn table_hit(code: &str) -> Option<(&'static str, u32)> {
    FILTER_WAVELENGTHS_NM.iter().copied().find(|(known, _)| *known == code)
}

pub fn filter_code_and_wavelength_nm(value: &str) -> Option<(&'static str, u32)> {
    let upper = value.to_uppercase();
    if let Some(hit) = table_hit(upper.trim()) {
        return Some(hit);
    }
    upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty() && *token != CLEAR_TOKEN)
        .find_map(table_hit)
}

fn is_wheel_position(value: &str) -> bool {
    (1..=2).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
}

fn header_filter_value(header: &HduHeader, key: &str) -> Option<String> {
    let value = header.get(key)?.trim().trim_matches('\'').trim();
    (!value.is_empty() && !is_wheel_position(value)).then(|| value.to_string())
}

pub fn effective_filter(header: &HduHeader) -> Option<String> {
    let pupil_resolves = header_filter_value(header, PUPIL_KEY).is_some_and(|v| filter_code_and_wavelength_nm(&v).is_some());
    let keys: &[&str] = if pupil_resolves { &PUPIL_FIRST_KEYS } else { &FILTER_HEADER_KEYS };
    let values: Vec<String> = keys.iter().filter_map(|key| header_filter_value(header, key)).collect();
    values
        .iter()
        .find(|v| filter_code_and_wavelength_nm(v).is_some())
        .or_else(|| values.first())
        .cloned()
}

struct BuiltinFilter {
    dataset: &'static str,
    filter: &'static str,
    pivot_angstrom: f64,
    instrument: &'static str,
    group: &'static str,
    aliases: &'static [&'static str],
}

const fn row(
    dataset: &'static str,
    filter: &'static str,
    pivot_angstrom: f64,
    instrument: &'static str,
    group: &'static str,
    aliases: &'static [&'static str],
) -> BuiltinFilter {
    BuiltinFilter { dataset, filter, pivot_angstrom, instrument, group, aliases }
}

const BUILTIN_FILTERS: &[BuiltinFilter] = &[
    row("JWST", "F070W", 7039.12, "NIRCam", "SW", &[]),
    row("JWST", "F090W", 9021.53, "NIRCam", "SW", &[]),
    row("JWST", "F115W", 11542.61, "NIRCam", "SW", &[]),
    row("JWST", "F140M", 14053.23, "NIRCam", "SW", &[]),
    row("JWST", "F150W", 15007.44, "NIRCam", "SW", &[]),
    row("JWST", "F162M", 16272.47, "NIRCam", "SW", &[]),
    row("JWST", "F164N", 16445.36, "NIRCam", "SW", &[]),
    row("JWST", "F150W2", 16592.11, "NIRCam", "SW", &[]),
    row("JWST", "F182M", 18451.67, "NIRCam", "SW", &[]),
    row("JWST", "F187N", 18738.99, "NIRCam", "SW", &[]),
    row("JWST", "F200W", 19886.48, "NIRCam", "SW", &[]),
    row("JWST", "F210M", 20954.51, "NIRCam", "SW", &[]),
    row("JWST", "F212N", 21213.18, "NIRCam", "SW", &[]),
    row("JWST", "F250M", 25032.33, "NIRCam", "LW", &[]),
    row("JWST", "F277W", 27617.40, "NIRCam", "LW", &[]),
    row("JWST", "F300M", 29891.21, "NIRCam", "LW", &[]),
    row("JWST", "F323N", 32368.53, "NIRCam", "LW", &[]),
    row("JWST", "F322W2", 32319.76, "NIRCam", "LW", &[]),
    row("JWST", "F335M", 33620.67, "NIRCam", "LW", &[]),
    row("JWST", "F356W", 35683.62, "NIRCam", "LW", &[]),
    row("JWST", "F360M", 36241.76, "NIRCam", "LW", &[]),
    row("JWST", "F405N", 40516.53, "NIRCam", "LW", &[]),
    row("JWST", "F410M", 40822.38, "NIRCam", "LW", &[]),
    row("JWST", "F430M", 42812.58, "NIRCam", "LW", &[]),
    row("JWST", "F444W", 44043.15, "NIRCam", "LW", &[]),
    row("JWST", "F460M", 46299.28, "NIRCam", "LW", &[]),
    row("JWST", "F466N", 46544.30, "NIRCam", "LW", &[]),
    row("JWST", "F470N", 47077.91, "NIRCam", "LW", &[]),
    row("JWST", "F480M", 48181.95, "NIRCam", "LW", &[]),
    row("HST", "F435W", 4329.85, "ACS", "ACS", &[]),
    row("HST", "F475W", 4746.94, "ACS", "ACS", &[]),
    row("HST", "F555W", 5361.03, "ACS", "ACS", &[]),
    row("HST", "F606W", 5921.88, "ACS", "ACS", &[]),
    row("HST", "F625W", 6311.85, "ACS", "ACS", &[]),
    row("HST", "F660N", 6599.46, "ACS", "ACS", &[]),
    row("HST", "F775W", 7693.47, "ACS", "ACS", &[]),
    row("HST", "F814W", 8045.53, "ACS", "ACS", &[]),
    row("HST", "F850LP", 9031.48, "ACS", "ACS", &[]),
    row("HST", "F225W", 2357.65, "WFC3", "UVIS", &["F225WU"]),
    row("HST", "F275W", 2702.92, "WFC3", "UVIS", &["F275WU"]),
    row("HST", "F336W", 3354.60, "WFC3", "UVIS", &["F336WU"]),
    row("HST", "F390W", 3920.70, "WFC3", "UVIS", &[]),
    row("HST", "F680N", 6877.61, "WFC3", "UVIS", &[]),
    row("HST", "F098M", 9862.72, "WFC3", "IR", &[]),
    row("HST", "F105W", 10550.25, "WFC3", "IR", &[]),
    row("HST", "F110W", 11534.46, "WFC3", "IR", &[]),
    row("HST", "F125W", 12486.07, "WFC3", "IR", &[]),
    row("HST", "F127M", 12741.07, "WFC3", "IR", &[]),
    row("HST", "F139M", 13841.81, "WFC3", "IR", &[]),
    row("HST", "F140W", 13923.21, "WFC3", "IR", &[]),
    row("HST", "F153M", 15332.75, "WFC3", "IR", &[]),
    row("HST", "F160W", 15370.34, "WFC3", "IR", &[]),
    row("LSST", "u", 3680.04, "LSST", "LSST", &[]),
    row("LSST", "g", 4782.26, "LSST", "LSST", &[]),
    row("LSST", "r", 6217.82, "LSST", "LSST", &[]),
    row("LSST", "i", 7532.28, "LSST", "LSST", &[]),
    row("LSST", "z", 8685.06, "LSST", "LSST", &[]),
    row("LSST", "y", 9730.05, "LSST", "LSST", &[]),
    row("JPAS", "uJAVA", 3545.16, "JPAS", "broad", &[]),
    row("JPAS", "u", 3696.25, "JPAS", "broad", &[]),
    row("JPAS", "gSDSS", 4767.98, "JPAS", "broad", &[]),
    row("JPAS", "rSDSS", 6245.36, "JPAS", "broad", &[]),
    row("JPAS", "iSDSS", 7656.20, "JPAS", "broad", &[]),
    row("JPAS", "J0378", 3796.72, "JPAS", "narrow", &[]),
    row("JPAS", "J0390", 3904.35, "JPAS", "narrow", &[]),
    row("JPAS", "J0400", 4007.05, "JPAS", "narrow", &[]),
    row("JPAS", "J0410", 4116.21, "JPAS", "narrow", &[]),
    row("JPAS", "J0420", 4209.93, "JPAS", "narrow", &[]),
    row("JPAS", "J0430", 4310.85, "JPAS", "narrow", &[]),
    row("JPAS", "J0440", 4408.62, "JPAS", "narrow", &[]),
    row("JPAS", "J0450", 4515.05, "JPAS", "narrow", &[]),
    row("JPAS", "J0460", 4608.91, "JPAS", "narrow", &[]),
    row("JPAS", "J0470", 4705.05, "JPAS", "narrow", &[]),
    row("JPAS", "J0480", 4808.56, "JPAS", "narrow", &[]),
    row("JPAS", "J0490", 4906.78, "JPAS", "narrow", &[]),
    row("JPAS", "J0500", 5005.77, "JPAS", "narrow", &[]),
    row("JPAS", "J0510", 5102.18, "JPAS", "narrow", &[]),
    row("JPAS", "J0520", 5208.42, "JPAS", "narrow", &[]),
    row("JPAS", "J0530", 5304.05, "JPAS", "narrow", &[]),
    row("JPAS", "J0540", 5397.35, "JPAS", "narrow", &[]),
    row("JPAS", "J0550", 5502.11, "JPAS", "narrow", &[]),
    row("JPAS", "J0560", 5604.63, "JPAS", "narrow", &[]),
    row("JPAS", "J0570", 5710.16, "JPAS", "narrow", &[]),
    row("JPAS", "J0580", 5811.75, "JPAS", "narrow", &[]),
    row("JPAS", "J0590", 5920.18, "JPAS", "narrow", &[]),
    row("JPAS", "J0600", 6010.29, "JPAS", "narrow", &[]),
    row("JPAS", "J0610", 6117.67, "JPAS", "narrow", &[]),
    row("JPAS", "J0620", 6209.24, "JPAS", "narrow", &[]),
    row("JPAS", "J0630", 6312.87, "JPAS", "narrow", &[]),
    row("JPAS", "J0640", 6410.68, "JPAS", "narrow", &[]),
    row("JPAS", "J0650", 6507.56, "JPAS", "narrow", &[]),
    row("JPAS", "J0660", 6607.63, "JPAS", "narrow", &[]),
    row("JPAS", "J0670", 6713.15, "JPAS", "narrow", &[]),
    row("JPAS", "J0680", 6810.86, "JPAS", "narrow", &[]),
    row("JPAS", "J0690", 6916.25, "JPAS", "narrow", &[]),
    row("JPAS", "J0700", 7008.56, "JPAS", "narrow", &[]),
    row("JPAS", "J0710", 7121.87, "JPAS", "narrow", &[]),
    row("JPAS", "J0720", 7212.05, "JPAS", "narrow", &[]),
    row("JPAS", "J0730", 7312.16, "JPAS", "narrow", &[]),
    row("JPAS", "J0740", 7416.10, "JPAS", "narrow", &[]),
    row("JPAS", "J0750", 7501.82, "JPAS", "narrow", &[]),
    row("JPAS", "J0760", 7599.48, "JPAS", "narrow", &[]),
    row("JPAS", "J0770", 7720.67, "JPAS", "narrow", &[]),
    row("JPAS", "J0780", 7808.15, "JPAS", "narrow", &[]),
    row("JPAS", "J0790", 7908.32, "JPAS", "narrow", &[]),
    row("JPAS", "J0800", 8007.07, "JPAS", "narrow", &[]),
    row("JPAS", "J0810", 8114.29, "JPAS", "narrow", &[]),
    row("JPAS", "J0820", 8216.72, "JPAS", "narrow", &[]),
    row("JPAS", "J0830", 8318.81, "JPAS", "narrow", &[]),
    row("JPAS", "J0840", 8410.94, "JPAS", "narrow", &[]),
    row("JPAS", "J0850", 8509.07, "JPAS", "narrow", &[]),
    row("JPAS", "J0860", 8598.06, "JPAS", "narrow", &[]),
    row("JPAS", "J0870", 8709.04, "JPAS", "narrow", &[]),
    row("JPAS", "J0880", 8819.03, "JPAS", "narrow", &[]),
    row("JPAS", "J0890", 8910.48, "JPAS", "narrow", &[]),
    row("JPAS", "J0900", 9003.26, "JPAS", "narrow", &[]),
    row("JPAS", "J0910", 9095.89, "JPAS", "narrow", &[]),
    row("JPAS", "J1007", 9590.54, "JPAS", "narrow", &[]),
    row("JPLUS", "uJAVA", 3491.44, "JPLUS", "broad", &[]),
    row("JPLUS", "J0378", 3782.37, "JPLUS", "narrow", &[]),
    row("JPLUS", "J0395", 3939.37, "JPLUS", "narrow", &[]),
    row("JPLUS", "J0410", 4108.12, "JPLUS", "narrow", &[]),
    row("JPLUS", "J0430", 4303.29, "JPLUS", "narrow", &[]),
    row("JPLUS", "gSDSS", 4742.94, "JPLUS", "broad", &[]),
    row("JPLUS", "J0515", 5140.89, "JPLUS", "narrow", &[]),
    row("JPLUS", "rSDSS", 6247.64, "JPLUS", "broad", &[]),
    row("JPLUS", "J0660", 6603.91, "JPLUS", "narrow", &[]),
    row("JPLUS", "iSDSS", 7656.20, "JPLUS", "broad", &[]),
    row("JPLUS", "J0861", 8610.49, "JPLUS", "narrow", &[]),
    row("JPLUS", "zSDSS", 8964.70, "JPLUS", "broad", &[]),
    row("SWIFT", "UVW2", 2033.0, "UVOT", "UVOT", &[]),
    row("SWIFT", "UVM2", 2229.0, "UVOT", "UVOT", &[]),
    row("SWIFT", "UVW1", 2591.0, "UVOT", "UVOT", &[]),
    row("FDS", "u", 3540.0, "OmegaCAM", "OmegaCAM", &[]),
    row("FDS", "g", 4750.0, "OmegaCAM", "OmegaCAM", &[]),
    row("FDS", "r", 6220.0, "OmegaCAM", "OmegaCAM", &[]),
    row("FDS", "i", 7630.0, "OmegaCAM", "OmegaCAM", &[]),
    row("ROMAN", "F062", 6291.0, "WFI", "WFI", &[]),
    row("ROMAN", "F087", 8696.0, "WFI", "WFI", &[]),
    row("ROMAN", "F106", 10567.0, "WFI", "WFI", &[]),
    row("ROMAN", "F129", 12901.0, "WFI", "WFI", &[]),
    row("ROMAN", "F146", 14378.0, "WFI", "WFI", &[]),
    row("ROMAN", "F158", 15749.0, "WFI", "WFI", &[]),
    row("ROMAN", "F184", 18394.0, "WFI", "WFI", &[]),
    row("ROMAN", "F213", 21230.0, "WFI", "WFI", &[]),
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FilterEntry {
    pub dataset: String,
    pub filter: String,
    pub instrument: String,
    pub group: String,
    pub pivot_angstrom: f64,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FilterSource {
    Header { keyword: String },
    FileName,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FilterIdentification {
    pub entry: FilterEntry,
    pub source: FilterSource,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterSelection {
    pub dataset: String,
    #[serde(default)]
    pub instrument: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct FilterCandidate<'a> {
    pub path: &'a str,
    pub header: Option<&'a HduHeader>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectedFilter {
    pub path: String,
    pub identification: FilterIdentification,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FilterError {
    #[error("unknown dataset '{dataset}'; the registry has {}", known.join(", "))]
    UnknownDataset { dataset: String, known: Vec<String> },
    #[error("'{file_name}': the dataset {dataset} is not a word of the file name; name the file like '{dataset}_<filter>.fits', with the dataset and the filter delimited by '_', '-' or '.'")]
    DatasetNotInFileName { dataset: String, file_name: String },
    #[error("'{file_name}': no {dataset} filter is a word of the file name")]
    NoFilterInFileName { dataset: String, file_name: String },
    #[error("'{file_name}': the file name names several {dataset} filters ({})", filters.join(", "))]
    SeveralFiltersInFileName { dataset: String, file_name: String, filters: Vec<String> },
    #[error("'{file_name}': TELESCOP says {header} but the dataset {requested} was asked for")]
    DatasetMismatch { file_name: String, header: String, requested: String },
    #[error("'{file_name}': no TELESCOP card names a registry dataset, so the dataset must be given to read the filter from the file name")]
    DatasetRequired { file_name: String },
    #[error("no file matched the filter selection")]
    NothingSelected,
    #[error("dataset '{dataset}': {reason}")]
    InvalidDataset { dataset: String, reason: String },
}

const HEADER_INSTRUMENT_ALIASES: [(&str, &str); 1] = [("UVOTA", "UVOT")];

impl FilterEntry {
    fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.filter.as_str()).chain(self.aliases.iter().map(String::as_str))
    }

    fn is_named(&self, code: &str) -> bool {
        self.names().any(|name| name.to_uppercase() == code)
    }
}

fn file_name_of(path: &str) -> String {
    Path::new(path).file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

fn file_stem_of(path: &str) -> String {
    Path::new(path).file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned())
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

fn has_word(text: &str, word: &str) -> bool {
    let (text, word) = (text.to_uppercase(), word.to_uppercase());
    !word.is_empty()
        && (0..=text.len()).filter(|&start| text.is_char_boundary(start)).any(|start| {
            text[start..].starts_with(&word)
                && !text[..start].chars().next_back().is_some_and(is_word_char)
                && !text[start + word.len()..].chars().next().is_some_and(is_word_char)
        })
}

fn header_text(header: &HduHeader, key: &str) -> Option<String> {
    let value = header.get(key)?.trim().trim_matches('\'').trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn header_instrument(header: &HduHeader) -> Option<String> {
    let raw = header_text(header, "INSTRUME")?.to_uppercase();
    let alias = HEADER_INSTRUMENT_ALIASES.iter().find(|(name, _)| *name == raw).map(|(_, registry)| registry.to_string());
    Some(alias.unwrap_or(raw))
}

fn resolve_value<'a>(candidates: &[&'a FilterEntry], value: &str) -> Option<&'a FilterEntry> {
    let upper = value.to_uppercase();
    let lookup = |code: &str| candidates.iter().copied().find(|entry| entry.is_named(code));
    lookup(upper.trim()).or_else(|| {
        upper
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty() && *token != CLEAR_TOKEN)
            .find_map(lookup)
    })
}

fn matches_criterion(value: &str, wanted: Option<&str>) -> bool {
    wanted.is_none_or(|wanted| value.eq_ignore_ascii_case(wanted.trim()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilterRegistry {
    entries: Vec<FilterEntry>,
}

impl FilterRegistry {
    pub fn builtin() -> Self {
        let entries = BUILTIN_FILTERS
            .iter()
            .map(|row| FilterEntry {
                dataset: row.dataset.to_string(),
                filter: row.filter.to_string(),
                instrument: row.instrument.to_string(),
                group: row.group.to_string(),
                pivot_angstrom: row.pivot_angstrom,
                aliases: row.aliases.iter().map(|alias| alias.to_string()).collect(),
            })
            .collect();
        Self { entries }
    }

    pub fn with_datasets(datasets: &BTreeMap<String, BTreeMap<String, CustomFilter>>) -> Result<Self, FilterError> {
        let mut registry = Self::builtin();
        for (dataset, filters) in datasets {
            registry.add_dataset(dataset, filters, false)?;
        }
        Ok(registry)
    }

    pub fn entries(&self) -> &[FilterEntry] {
        &self.entries
    }

    pub fn datasets(&self) -> Vec<String> {
        let mut datasets: Vec<String> = Vec::new();
        for entry in &self.entries {
            if !datasets.contains(&entry.dataset) {
                datasets.push(entry.dataset.clone());
            }
        }
        datasets
    }

    fn dataset_key(&self, name: &str) -> Option<&str> {
        self.entries.iter().map(|entry| entry.dataset.as_str()).find(|dataset| dataset.eq_ignore_ascii_case(name.trim()))
    }

    fn require_dataset(&self, name: &str) -> Result<&str, FilterError> {
        self.dataset_key(name)
            .ok_or_else(|| FilterError::UnknownDataset { dataset: name.trim().to_uppercase(), known: self.datasets() })
    }

    fn entries_of<'a>(&'a self, dataset: &'a str) -> impl Iterator<Item = &'a FilterEntry> + 'a {
        self.entries.iter().filter(move |entry| entry.dataset == dataset)
    }

    pub fn entry(&self, dataset: &str, filter: &str) -> Option<&FilterEntry> {
        let dataset = self.dataset_key(dataset)?;
        let code = filter.trim().to_uppercase();
        self.entries_of(dataset).find(|entry| entry.is_named(&code))
    }

    pub fn add_dataset(&mut self, dataset: &str, filters: &BTreeMap<String, CustomFilter>, overwrite: bool) -> Result<(), FilterError> {
        let key = dataset.trim().to_uppercase();
        let invalid = |reason: String| FilterError::InvalidDataset { dataset: key.clone(), reason };
        if key.is_empty() {
            return Err(invalid("the dataset has no name".into()));
        }
        if filters.is_empty() {
            return Err(invalid("the dataset has no filter".into()));
        }
        if self.dataset_key(&key).is_some() && !overwrite {
            return Err(invalid("the dataset already exists; replace it with overwrite".into()));
        }
        let mut owners: BTreeMap<String, &str> = BTreeMap::new();
        for (filter, spec) in filters {
            if !(spec.pivot_angstrom.is_finite() && spec.pivot_angstrom > 0.0) {
                return Err(invalid(format!("filter '{filter}': lambda {} is not a positive wavelength in Angstrom", spec.pivot_angstrom)));
            }
            for name in std::iter::once(filter).chain(&spec.aliases) {
                let code = name.trim().to_uppercase();
                if code.is_empty() {
                    return Err(invalid(format!("filter '{filter}' has an empty name or alias")));
                }
                match owners.get(&code) {
                    Some(owner) if *owner != filter.as_str() => {
                        return Err(invalid(format!(
                            "name '{name}' of filter '{filter}' collides with filter '{owner}' (names are case-insensitive)"
                        )));
                    }
                    _ => {
                        owners.insert(code, filter.as_str());
                    }
                }
            }
        }
        self.entries.retain(|entry| entry.dataset != key);
        self.entries.extend(filters.iter().map(|(filter, spec)| FilterEntry {
            dataset: key.clone(),
            filter: filter.trim().to_string(),
            instrument: spec.instrument.clone(),
            group: spec.group.clone(),
            pivot_angstrom: spec.pivot_angstrom,
            aliases: spec.aliases.iter().map(|alias| alias.trim().to_string()).collect(),
        }));
        Ok(())
    }

    pub fn identify_file_name(&self, path: &str, dataset: &str) -> Result<FilterIdentification, FilterError> {
        let dataset = self.require_dataset(dataset)?;
        let file_name = file_name_of(path);
        let stem = file_stem_of(path);
        if !has_word(&stem, dataset) {
            return Err(FilterError::DatasetNotInFileName { dataset: dataset.to_string(), file_name });
        }
        let matches: Vec<&FilterEntry> = self.entries_of(dataset).filter(|entry| entry.names().any(|name| has_word(&stem, name))).collect();
        match matches.as_slice() {
            [] => Err(FilterError::NoFilterInFileName { dataset: dataset.to_string(), file_name }),
            [entry] => Ok(FilterIdentification { entry: (*entry).clone(), source: FilterSource::FileName, warnings: Vec::new() }),
            several => Err(FilterError::SeveralFiltersInFileName {
                dataset: dataset.to_string(),
                file_name,
                filters: several.iter().map(|entry| entry.filter.clone()).collect(),
            }),
        }
    }

    fn identify_in_header(&self, header: &HduHeader, dataset: &str) -> Result<FilterIdentification, Option<String>> {
        let in_dataset: Vec<&FilterEntry> = self.entries_of(dataset).collect();
        let instrument = header_instrument(header);
        let candidates: Vec<&FilterEntry> = match instrument.as_deref() {
            Some(name) if in_dataset.iter().any(|entry| entry.instrument.eq_ignore_ascii_case(name)) => {
                in_dataset.iter().copied().filter(|entry| entry.instrument.eq_ignore_ascii_case(name)).collect()
            }
            Some(name) => return Err(Some(format!("the registry has no {dataset} {name} filters"))),
            None => in_dataset.clone(),
        };
        let pupil_resolves = header_filter_value(header, PUPIL_KEY).is_some_and(|value| resolve_value(&candidates, &value).is_some());
        let keys: &[&str] = if pupil_resolves { &PUPIL_FIRST_KEYS } else { &FILTER_HEADER_KEYS };
        let values: Vec<(&str, String)> = keys.iter().filter_map(|key| header_filter_value(header, key).map(|value| (*key, value))).collect();
        if let Some((keyword, entry)) = values.iter().find_map(|(key, value)| resolve_value(&candidates, value).map(|entry| (*key, entry))) {
            return Ok(FilterIdentification {
                entry: entry.clone(),
                source: FilterSource::Header { keyword: keyword.to_string() },
                warnings: Vec::new(),
            });
        }
        if values.is_empty() {
            return Err(None);
        }
        let named: Vec<String> = values.iter().map(|(key, value)| format!("{key} {value}")).collect();
        let elsewhere: Vec<String> = values
            .iter()
            .filter_map(|(_, value)| resolve_value(&in_dataset, value))
            .map(|entry| format!("{} is listed for {}", entry.filter, entry.instrument))
            .collect();
        let scope = instrument.map_or_else(|| dataset.to_string(), |name| format!("{dataset} {name}"));
        let mut reason = format!("{} names no {scope} filter of the registry", named.join(", "));
        if !elsewhere.is_empty() {
            reason.push_str(&format!(" ({})", elsewhere.join(", ")));
        }
        Err(Some(reason))
    }

    fn instrument_dataset(&self, instrument: &str) -> Option<&str> {
        let mut owners = self.entries.iter().filter(|entry| entry.instrument.eq_ignore_ascii_case(instrument)).map(|entry| entry.dataset.as_str());
        let first = owners.next()?;
        owners.all(|dataset| dataset == first).then_some(first)
    }

    fn header_dataset(&self, header: &HduHeader) -> Option<&str> {
        header_text(header, "TELESCOP")
            .and_then(|telescope| self.dataset_key(&telescope))
            .or_else(|| header_instrument(header).and_then(|instrument| self.instrument_dataset(&instrument)))
    }

    pub fn identify(&self, header: Option<&HduHeader>, path: &str, dataset: Option<&str>) -> Result<FilterIdentification, FilterError> {
        let requested = dataset.map(|name| self.require_dataset(name)).transpose()?;
        let from_header = header.and_then(|header| self.header_dataset(header));
        if let (Some(requested), Some(from_header)) = (requested, from_header) {
            if requested != from_header {
                return Err(FilterError::DatasetMismatch {
                    file_name: file_name_of(path),
                    header: from_header.to_string(),
                    requested: requested.to_string(),
                });
            }
        }
        let dataset = requested.or(from_header).ok_or_else(|| FilterError::DatasetRequired { file_name: file_name_of(path) })?;
        let mut notes = Vec::new();
        if let Some(header) = header {
            match self.identify_in_header(header, dataset) {
                Ok(mut found) => {
                    if let Ok(named) = self.identify_file_name(path, dataset) {
                        if named.entry.filter != found.entry.filter {
                            found.warnings.push(format!(
                                "the file name names {} but the header names {}; the header wins",
                                named.entry.filter, found.entry.filter
                            ));
                        }
                    }
                    return Ok(found);
                }
                Err(Some(reason)) => notes.push(format!("header: {reason}; the file name rule decides")),
                Err(None) => {}
            }
        }
        let mut found = self.identify_file_name(path, dataset)?;
        if notes.is_empty() {
            if let Some(instrument) = header.and_then(header_instrument) {
                if !found.entry.instrument.eq_ignore_ascii_case(&instrument) {
                    notes.push(format!(
                        "INSTRUME {instrument} differs from the {} entry of {}; its pivot wavelength is used",
                        found.entry.instrument, found.entry.filter
                    ));
                }
            }
        }
        found.warnings.extend(notes);
        Ok(found)
    }

    pub fn select(&self, files: &[FilterCandidate<'_>], selection: &[FilterSelection]) -> Result<Vec<SelectedFilter>, FilterError> {
        let mut selected = Vec::new();
        for criteria in selection {
            let dataset = self.require_dataset(&criteria.dataset)?;
            for file in files {
                let Ok(found) = self.identify(file.header, file.path, Some(dataset)) else { continue };
                if matches_criterion(&found.entry.instrument, criteria.instrument.as_deref())
                    && matches_criterion(&found.entry.group, criteria.group.as_deref())
                {
                    selected.push(SelectedFilter { path: file.path.to_string(), identification: found });
                }
            }
        }
        selected.sort_by(|a, b| a.identification.entry.pivot_angstrom.total_cmp(&b.identification.entry.pivot_angstrom));
        if selected.is_empty() {
            return Err(FilterError::NothingSelected);
        }
        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (key, value) in cards {
            h.set(key, value.to_string());
        }
        h
    }

    #[test]
    fn effective_filter_prefers_a_pupil_filter_code() {
        let pupil = header(&[("FILTER", "F444W"), ("PUPIL", "F470N")]);
        assert_eq!(effective_filter(&pupil).as_deref(), Some("F470N"));
        let clear_pupil = header(&[("FILTER", "F444W"), ("PUPIL", "CLEAR")]);
        assert_eq!(effective_filter(&clear_pupil).as_deref(), Some("F444W"));
        let niriss = header(&[("FILTER", "CLEAR"), ("PUPIL", "F158M")]);
        assert_eq!(effective_filter(&niriss).as_deref(), Some("F158M"));
        let wfpc2 = header(&[("FILTER1", "23"), ("FILTNAM1", "F502N")]);
        assert_eq!(effective_filter(&wfpc2).as_deref(), Some("F502N"));
        let acs = header(&[("FILTER1", "CLEAR1L"), ("FILTER2", "F814W")]);
        assert_eq!(effective_filter(&acs).as_deref(), Some("F814W"));
        let acs_unknown = header(&[("FILTER1", "CLEAR1L"), ("FILTER2", "F850LP")]);
        assert_eq!(effective_filter(&acs_unknown).as_deref(), Some("CLEAR1L"));
        let nrm = header(&[("PUPIL", "NRM")]);
        assert_eq!(effective_filter(&nrm).as_deref(), Some("NRM"));
        let quoted = header(&[("FILTER", "'F444W   '"), ("PUPIL", "'F470N   '")]);
        assert_eq!(effective_filter(&quoted).as_deref(), Some("F470N"));
        let unresolvable = header(&[("FILTER", "CLEAR"), ("PUPIL", "NRM")]);
        assert_eq!(effective_filter(&unresolvable).as_deref(), Some("CLEAR"));
        assert_eq!(effective_filter(&HduHeader::empty()), None);
    }

    #[test]
    fn tokenised_lookup_matches_the_frontend() {
        assert_eq!(filter_code_and_wavelength_nm("CLEAR/F090W"), Some(("F090W", 900)));
        assert_eq!(filter_code_and_wavelength_nm("F444W;CLEAR"), Some(("F444W", 4440)));
        assert_eq!(filter_code_and_wavelength_nm("CLEAR"), None);
        assert_eq!(filter_code_and_wavelength_nm("CLEAR/CLEAR"), None);
        assert_eq!(filter_code_and_wavelength_nm(" ha "), Some(("HA", 656)));
        assert_eq!(filter_code_and_wavelength_nm("f090w"), Some(("F090W", 900)));
        assert_eq!(filter_code_and_wavelength_nm("F158M"), Some(("F158M", 1580)));
        assert_eq!(filter_code_and_wavelength_nm("F380M"), Some(("F380M", 3800)));
        assert_eq!(filter_code_and_wavelength_nm("F814W"), Some(("F814W", 814)));
        assert_eq!(filter_code_and_wavelength_nm("XYZ123"), None);
        assert_eq!(filter_code_and_wavelength_nm(""), None);
    }

    const ROMAN_PIVOT_SOURCE: &str = "GalSim Roman_effarea_20210614 pivot; roman-docs WFI Optical Elements table (read 2026-10-16) agrees to the integer nm";

    #[test]
    fn roman_wfi_pivots_resolve_exactly() {
        let pivots: [(&str, u32, f64, &str); 8] = [
            ("F062", 629, 0.6291, ROMAN_PIVOT_SOURCE),
            ("F087", 870, 0.8696, ROMAN_PIVOT_SOURCE),
            ("F106", 1057, 1.0567, ROMAN_PIVOT_SOURCE),
            ("F129", 1290, 1.2901, ROMAN_PIVOT_SOURCE),
            ("F146", 1438, 1.4378, ROMAN_PIVOT_SOURCE),
            ("F158", 1575, 1.5749, ROMAN_PIVOT_SOURCE),
            ("F184", 1839, 1.8394, ROMAN_PIVOT_SOURCE),
            ("F213", 2123, 2.1230, ROMAN_PIVOT_SOURCE),
        ];
        for (code, nm, documented_um, source) in pivots {
            assert_eq!(filter_code_and_wavelength_nm(code), Some((code, nm)), "{code} ({source})");
            assert!((f64::from(nm) - documented_um * 1000.0).abs() <= 0.5, "{code}: table {nm} nm vs roman-docs {documented_um} um");
        }
        assert_eq!(filter_code_and_wavelength_nm("F158"), Some(("F158", 1575)));
        assert_eq!(filter_code_and_wavelength_nm("F158M"), Some(("F158M", 1580)));
        assert_eq!(filter_code_and_wavelength_nm("f129"), Some(("F129", 1290)));
        assert_eq!(filter_code_and_wavelength_nm("F129;CLEAR"), Some(("F129", 1290)));
        assert_eq!(filter_code_and_wavelength_nm("WFI/f213"), Some(("F213", 2123)));
        let roman = header(&[("TELESCOP", "ROMAN"), ("INSTRUME", "WFI"), ("FILTER", "F129")]);
        assert_eq!(effective_filter(&roman).as_deref(), Some("F129"));
    }

    #[test]
    fn the_table_has_82_unique_codes() {
        assert_eq!(FILTER_WAVELENGTHS_NM.len(), 82);
        let mut codes: Vec<&str> = FILTER_WAVELENGTHS_NM.iter().map(|(code, _)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 82);
        assert!(FILTER_WAVELENGTHS_NM.iter().all(|(code, nm)| *code == code.to_ascii_uppercase() && *nm > 0));
    }

    const TUTORIAL_1_FILES: [&str; 30] = [
        "hst-abell2744clu-grizli-v7.0-f105w_drz_sci.fits",
        "hst-abell2744clu-grizli-v7.0-f125w_drz_sci.fits",
        "hst-abell2744clu-grizli-v7.0-f140w_drz_sci.fits",
        "hst-abell2744clu-grizli-v7.0-f160w_drz_sci.fits",
        "hst-abell2744clu-grizli-v7.1-f225wu_drc_sci.fits",
        "hst-abell2744clu-grizli-v7.1-f275wu_drc_sci.fits",
        "hst-abell2744clu-grizli-v7.1-f336wu_drc_sci.fits",
        "hst-abell2744clu-grizli-v7.1-f435w_drc_sci.fits",
        "hst-abell2744clu-grizli-v7.1-f606w_drc_sci.fits",
        "hst-abell2744clu-grizli-v7.1-f814w_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f070w-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f090w-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f115w-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f140m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f150w-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f162m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f182m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f200w-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f210m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f250m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f277w-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f300m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f335m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f356w-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f360m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f410m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f430m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f444w-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f460m-clear_drc_sci.fits",
        "jwst-abell2744clu-grizli-v7.2-f480m-clear_drc_sci.fits",
    ];
    const TUTORIAL_1_SIMPLIFIED: [&str; 30] = [
        "HST_F105W.fits", "HST_F125W.fits", "HST_F140W.fits", "HST_F160W.fits", "HST_F225W.fits",
        "HST_F275W.fits", "HST_F336W.fits", "HST_F435W.fits", "HST_F606W.fits", "HST_F814W.fits",
        "JWST_F070W.fits", "JWST_F090W.fits", "JWST_F115W.fits", "JWST_F140M.fits", "JWST_F150W.fits",
        "JWST_F162M.fits", "JWST_F182M.fits", "JWST_F200W.fits", "JWST_F210M.fits", "JWST_F250M.fits",
        "JWST_F277W.fits", "JWST_F300M.fits", "JWST_F335M.fits", "JWST_F356W.fits", "JWST_F360M.fits",
        "JWST_F410M.fits", "JWST_F430M.fits", "JWST_F444W.fits", "JWST_F460M.fits", "JWST_F480M.fits",
    ];
    const NIRCAM_SW: [&str; 9] = ["F070W", "F090W", "F115W", "F140M", "F150W", "F162M", "F182M", "F200W", "F210M"];
    const NIRCAM_LW: [&str; 11] = ["F250M", "F277W", "F300M", "F335M", "F356W", "F360M", "F410M", "F430M", "F444W", "F460M", "F480M"];
    const NIRCAM_PRINTED_ANGSTROM: [f64; 20] = [
        7039.12, 9021.53, 11542.61, 14053.23, 15007.44, 16272.47, 18451.67, 19886.48, 20954.51, 25032.33, 27617.40, 29891.21,
        33620.67, 35683.62, 36241.76, 40822.38, 42812.58, 44043.15, 46299.28, 48181.95,
    ];

    fn selection(dataset: &str, instrument: Option<&str>, group: Option<&str>) -> FilterSelection {
        FilterSelection { dataset: dataset.into(), instrument: instrument.map(String::from), group: group.map(String::from) }
    }

    fn selected(registry: &FilterRegistry, paths: &[String], criteria: &[FilterSelection]) -> Vec<SelectedFilter> {
        let files: Vec<FilterCandidate> = paths.iter().map(|path| FilterCandidate { path, header: None }).collect();
        registry.select(&files, criteria).unwrap()
    }

    fn filters_of(chosen: &[SelectedFilter]) -> Vec<String> {
        chosen.iter().map(|item| item.identification.entry.filter.clone()).collect()
    }

    fn pivots_of(chosen: &[SelectedFilter]) -> Vec<f64> {
        chosen.iter().map(|item| item.identification.entry.pivot_angstrom).collect()
    }

    fn custom(value: serde_json::Value) -> BTreeMap<String, CustomFilter> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn grizli_uvis_alias_f225wu_is_f225w() {
        let registry = FilterRegistry::builtin();
        for (name, filter) in [
            ("hst-abell2744clu-grizli-v7.1-f225wu_drc_sci.fits", "F225W"),
            ("hst-abell2744clu-grizli-v7.1-f275wu_drc_sci.fits", "F275W"),
            ("hst-abell2744clu-grizli-v7.1-f336wu_drc_sci.fits", "F336W"),
            ("HST_F225W.fits", "F225W"),
        ] {
            let found = registry.identify_file_name(name, "HST").unwrap();
            assert_eq!(found.entry.filter, filter, "{name}");
            assert_eq!((found.entry.instrument.as_str(), found.entry.group.as_str()), ("WFC3", "UVIS"), "{name}");
            assert_eq!(found.source, FilterSource::FileName);
            assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        }
        assert_eq!(registry.entry("hst", "f225wu").map(|found| found.filter.as_str()), Some("F225W"));
        assert_eq!(registry.entry("HST", "F225W").map(|found| found.pivot_angstrom), Some(2357.65));
    }

    #[test]
    fn filename_with_two_filters_is_refused() {
        let registry = FilterRegistry::builtin();
        assert_eq!(
            registry.identify_file_name("data/JWST_F200W_F444W.fits", "JWST").unwrap_err(),
            FilterError::SeveralFiltersInFileName {
                dataset: "JWST".into(),
                file_name: "JWST_F200W_F444W.fits".into(),
                filters: vec!["F200W".into(), "F444W".into()],
            }
        );
        let refused = registry.identify_file_name("JWST_F150W_F150W2.fits", "JWST").unwrap_err();
        assert!(matches!(&refused, FilterError::SeveralFiltersInFileName { filters, .. } if filters == &["F150W", "F150W2"]), "{refused}");
        assert!(refused.to_string().contains("F150W, F150W2"), "{refused}");
        assert_eq!(registry.identify_file_name("HST_F225W_F225WU.fits", "HST").unwrap().entry.filter, "F225W");
        assert_eq!(registry.identify_file_name("JWST_F444W_f444w.fits", "jwst").unwrap().entry.filter, "F444W");
    }

    #[test]
    fn header_filter_wins_over_filename() {
        let registry = FilterRegistry::builtin();
        let nircam = header(&[("TELESCOP", "JWST"), ("INSTRUME", "NIRCAM"), ("FILTER", "F444W"), ("PUPIL", "CLEAR")]);
        let found = registry.identify(Some(&nircam), "mosaics/JWST_F200W.fits", None).unwrap();
        assert_eq!((found.entry.dataset.as_str(), found.entry.filter.as_str()), ("JWST", "F444W"));
        assert_eq!(found.source, FilterSource::Header { keyword: "FILTER".into() });
        assert!(found.warnings.iter().any(|warning| warning.contains("F200W") && warning.contains("F444W")), "{:?}", found.warnings);
        let agreeing = registry.identify(Some(&nircam), "JWST_F444W.fits", None).unwrap();
        assert!(agreeing.warnings.is_empty(), "{:?}", agreeing.warnings);

        let pupil = header(&[("TELESCOP", "JWST"), ("INSTRUME", "NIRCAM"), ("FILTER", "F444W"), ("PUPIL", "F470N")]);
        let found = registry.identify(Some(&pupil), "x.fits", None).unwrap();
        assert_eq!((found.entry.filter.as_str(), found.source.clone()), ("F470N", FilterSource::Header { keyword: "PUPIL".into() }));
        let acs = header(&[("TELESCOP", "HST"), ("INSTRUME", "ACS"), ("FILTER1", "CLEAR1L"), ("FILTER2", "F850LP")]);
        let found = registry.identify(Some(&acs), "j8xi01_drz.fits", None).unwrap();
        assert_eq!((found.entry.filter.as_str(), found.source.clone()), ("F850LP", FilterSource::Header { keyword: "FILTER2".into() }));
        let uvot = header(&[("TELESCOP", "SWIFT"), ("INSTRUME", "UVOTA"), ("FILTER", "UVW1")]);
        assert_eq!(registry.identify(Some(&uvot), "sw00031049001uw1_sk.img", None).unwrap().entry.filter, "UVW1");

        let no_telescope = header(&[("FILTER", "F444W")]);
        let found = registry.identify(Some(&no_telescope), "JWST_F200W.fits", Some("JWST")).unwrap();
        assert_eq!(found.entry.filter, "F444W");
        assert_eq!(
            registry.identify(Some(&no_telescope), "JWST_F200W.fits", None).unwrap_err(),
            FilterError::DatasetRequired { file_name: "JWST_F200W.fits".into() }
        );
        assert_eq!(registry.identify(None, "JWST_F200W.fits", Some("JWST")).unwrap().entry.filter, "F200W");
        assert!(matches!(registry.identify(Some(&nircam), "JWST_F444W.fits", Some("HST")), Err(FilterError::DatasetMismatch { .. })));
    }

    #[test]
    fn the_header_instrument_keys_the_lookup() {
        let registry = FilterRegistry::builtin();
        let wfc3 = header(&[("TELESCOP", "HST"), ("INSTRUME", "WFC3"), ("FILTER", "F814W")]);
        let found = registry.identify(Some(&wfc3), "hst_wfc3_uvis_f814w.fits", None).unwrap();
        assert_eq!((found.entry.instrument.as_str(), found.entry.filter.as_str()), ("ACS", "F814W"));
        assert_eq!(found.source, FilterSource::FileName);
        assert!(found.warnings.iter().any(|warning| warning.contains("WFC3") && warning.contains("ACS")), "{:?}", found.warnings);
        assert!(registry.identify(Some(&wfc3), "wfc3_image.fits", None).is_err(), "the header alone never borrows the ACS entry");

        let niriss = header(&[("TELESCOP", "JWST"), ("INSTRUME", "NIRISS"), ("FILTER", "CLEAR"), ("PUPIL", "F150W")]);
        assert_eq!(
            registry.identify(Some(&niriss), "jw01234_nis_i2d.fits", None).unwrap_err(),
            FilterError::DatasetNotInFileName { dataset: "JWST".into(), file_name: "jw01234_nis_i2d.fits".into() }
        );
        let named = registry.identify(Some(&niriss), "JWST_NIRISS_F150W.fits", None).unwrap();
        assert_eq!(named.entry.instrument, "NIRCam");
        assert!(named.warnings.iter().any(|warning| warning.contains("NIRISS")), "{:?}", named.warnings);
    }

    #[test]
    fn the_instrument_names_the_dataset_when_telescop_is_missing() {
        let registry = FilterRegistry::builtin();
        let nircam = header(&[("INSTRUME", "NIRCAM"), ("FILTER", "F200W"), ("PUPIL", "CLEAR")]);
        let found = registry.identify(Some(&nircam), "abell2744_sw.fits", None).unwrap();
        assert_eq!((found.entry.dataset.as_str(), found.entry.filter.as_str()), ("JWST", "F200W"));
        let uvot = header(&[("INSTRUME", "UVOTA"), ("FILTER", "UVM2")]);
        assert_eq!(registry.identify(Some(&uvot), "ngc1399.img", None).unwrap().entry.dataset, "SWIFT");
        let unknown = header(&[("INSTRUME", "MIRI"), ("FILTER", "F770W")]);
        assert_eq!(registry.identify(Some(&unknown), "m104.fits", None).unwrap_err(), FilterError::DatasetRequired { file_name: "m104.fits".into() });
    }

    #[test]
    fn the_file_name_rule_follows_lightstack_word_boundaries() {
        let registry = FilterRegistry::builtin();
        let found = |name: &str, dataset: &str| registry.identify_file_name(name, dataset).map(|hit| hit.entry.filter);
        assert_eq!(found("LSST_r.fits", "LSST"), Ok("r".to_string()));
        assert_eq!(found("abell370_LSST_r_drz.fits", "LSST"), Ok("r".to_string()));
        assert!(matches!(found("abell370_LSST_rband.fits", "LSST"), Err(FilterError::NoFilterInFileName { .. })));
        assert!(matches!(found("JWSTF200W.fits", "JWST"), Err(FilterError::DatasetNotInFileName { .. })));
        assert_eq!(found("jwst_f150w2.fits", "JWST"), Ok("F150W2".to_string()));
        assert_eq!(found("JPAS_uJAVA.fits", "JPAS"), Ok("uJAVA".to_string()));
        assert_eq!(found("jpas-u.fits", "jpas"), Ok("u".to_string()));
        assert_eq!(found("mytarget.JWST.F444W.drz.fits", "JWST"), Ok("F444W".to_string()));
        assert_eq!(found("JWST_F200W", "JWST"), Ok("F200W".to_string()));
        assert_eq!(found("JWST_F200W.fits.gz", "JWST"), Ok("F200W".to_string()));
        assert_eq!(found("swift_uvw1.fits", "Swift"), Ok("UVW1".to_string()));
        assert_eq!(
            found("/data/JWST_F444W/HST_F814W.fits", "JWST"),
            Err(FilterError::DatasetNotInFileName { dataset: "JWST".into(), file_name: "HST_F814W.fits".into() })
        );
        assert_eq!(
            found("JWST_F444W.fits", "euclid"),
            Err(FilterError::UnknownDataset {
                dataset: "EUCLID".into(),
                known: ["JWST", "HST", "LSST", "JPAS", "JPLUS", "SWIFT", "FDS", "ROMAN"].map(String::from).to_vec(),
            })
        );
    }

    #[test]
    fn cube_order_matches_select_filters_on_tutorial_names() {
        let registry = FilterRegistry::builtin();
        let grizli: Vec<String> = TUTORIAL_1_FILES.iter().map(|name| format!("./../outputs/datacubes/reg_cut/region_0/{name}")).collect();
        let simplified: Vec<String> =
            TUTORIAL_1_SIMPLIFIED.iter().map(|name| format!("./../outputs/datacubes/reg_cut/region_0_simple/{name}")).collect();
        let mixed: Vec<&str> = NIRCAM_SW.iter().chain(&NIRCAM_LW).copied().collect();
        let groups: [(&str, Option<&str>, Option<&str>, &[&str]); 6] = [
            ("JWST", Some("NIRCam"), Some("SW"), &NIRCAM_SW),
            ("JWST", Some("NIRCam"), Some("LW"), &NIRCAM_LW),
            ("HST", Some("ACS"), None, &["F435W", "F606W", "F814W"]),
            ("HST", Some("WFC3"), Some("IR"), &["F105W", "F125W", "F140W", "F160W"]),
            ("HST", Some("WFC3"), Some("UVIS"), &["F225W", "F275W", "F336W"]),
            ("JWST", Some("NIRCam"), None, &mixed),
        ];
        for paths in [&grizli, &simplified] {
            for (dataset, instrument, group, expected) in groups {
                let chosen = selected(&registry, paths, &[selection(dataset, instrument, group)]);
                assert_eq!(filters_of(&chosen), expected, "{dataset} {instrument:?} {group:?}");
            }
        }
        assert_eq!(pivots_of(&selected(&registry, &simplified, &[selection("JWST", Some("NIRCam"), None)])), NIRCAM_PRINTED_ANGSTROM);
        assert_eq!(pivots_of(&selected(&registry, &grizli, &[selection("HST", Some("ACS"), None)])), [4329.85, 5921.88, 8045.53]);
        assert_eq!(
            pivots_of(&selected(&registry, &grizli, &[selection("HST", Some("WFC3"), Some("IR"))])),
            [10550.25, 12486.07, 13923.21, 15370.34]
        );
        assert_eq!(pivots_of(&selected(&registry, &grizli, &[selection("HST", Some("WFC3"), Some("UVIS"))])), [2357.65, 2702.92, 3354.60]);

        let tutorial_2: Vec<String> = ["hst_acs_f475w", "hst_acs_f606w", "hst_acs_f850lp", "swift_uvm2", "swift_uvw1", "swift_uvw2"]
            .iter()
            .map(|stem| format!("fits/cut/{stem}.fits"))
            .collect();
        let chosen = selected(&registry, &tutorial_2, &[selection("HST", None, None), selection("SWIFT", None, None)]);
        let printed: Vec<(String, String, String)> = chosen
            .into_iter()
            .map(|item| (item.identification.entry.dataset, item.identification.entry.filter, item.path))
            .collect();
        let expected = [
            ("SWIFT", "UVW2", "fits/cut/swift_uvw2.fits"),
            ("SWIFT", "UVM2", "fits/cut/swift_uvm2.fits"),
            ("SWIFT", "UVW1", "fits/cut/swift_uvw1.fits"),
            ("HST", "F475W", "fits/cut/hst_acs_f475w.fits"),
            ("HST", "F606W", "fits/cut/hst_acs_f606w.fits"),
            ("HST", "F850LP", "fits/cut/hst_acs_f850lp.fits"),
        ]
        .map(|(dataset, filter, path)| (dataset.to_string(), filter.to_string(), path.to_string()));
        assert_eq!(printed, expected);
    }

    #[test]
    fn the_registry_holds_every_lightstack_dataset_and_the_roman_filters() {
        let registry = FilterRegistry::builtin();
        let datasets = registry.datasets();
        assert_eq!(datasets, ["JWST", "HST", "LSST", "JPAS", "JPLUS", "SWIFT", "FDS", "ROMAN"]);
        let count = |dataset: &str| registry.entries().iter().filter(|found| found.dataset == dataset).count();
        assert_eq!(datasets.iter().map(|dataset| count(dataset)).collect::<Vec<_>>(), [29, 23, 6, 60, 12, 3, 4, 8]);
        for dataset in &datasets {
            let mut names: Vec<String> = registry
                .entries()
                .iter()
                .filter(|found| &found.dataset == dataset)
                .flat_map(|found| std::iter::once(&found.filter).chain(&found.aliases).map(|name| name.to_uppercase()))
                .collect();
            let total = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), total, "{dataset}: a name is shared by two filters");
        }
        assert!(registry.entries().iter().all(|found| found.pivot_angstrom.is_finite() && found.pivot_angstrom > 0.0));
        for (dataset, filter, angstrom) in [
            ("JWST", "F444W", 44043.15),
            ("JWST", "F150W2", 16592.11),
            ("HST", "F850LP", 9031.48),
            ("LSST", "y", 9730.05),
            ("JPAS", "J1007", 9590.54),
            ("JPLUS", "zSDSS", 8964.70),
            ("SWIFT", "UVM2", 2229.0),
            ("FDS", "i", 7630.0),
        ] {
            assert_eq!(registry.entry(dataset, filter).map(|found| found.pivot_angstrom), Some(angstrom), "{dataset} {filter}");
        }
        assert_eq!(registry.entry("lsst", "R").map(|found| found.filter.as_str()), Some("r"));
        assert!(registry.entry("JWST", "F770W").is_none(), "MIRI imaging is not in the registry");
        assert_eq!(filter_code_and_wavelength_nm("F444W"), Some(("F444W", 4440)), "the colour view keeps its nominal table");
        for (code, micron) in [
            ("F062", 0.6291),
            ("F087", 0.8696),
            ("F106", 1.0567),
            ("F129", 1.2901),
            ("F146", 1.4378),
            ("F158", 1.5749),
            ("F184", 1.8394),
            ("F213", 2.1230),
        ] {
            let pivot = registry.entry("ROMAN", code).map(|found| found.pivot_angstrom).unwrap();
            assert!((pivot - micron * 1e4).abs() < 1e-6, "{code}: {pivot} ({ROMAN_PIVOT_SOURCE})");
        }
    }

    #[test]
    fn custom_datasets_follow_the_add_custom_dataset_rules() {
        let euclid = custom(serde_json::json!({
            "VIS": {"lambda": 7180.0, "instrument": "VIS", "group": "optical"},
            "Y": {"lambda": 10800.0, "instrument": "NISP", "group": "nir", "aliases": ["NISP_Y"]},
        }));
        let mut registry = FilterRegistry::builtin();
        registry.add_dataset("euclid", &euclid, false).unwrap();
        assert_eq!(registry.identify_file_name("EUCLID_VIS.fits", "Euclid").unwrap().entry.pivot_angstrom, 7180.0);
        assert_eq!(registry.identify_file_name("ngc1399_euclid_nisp_y.fits", "EUCLID").unwrap().entry.filter, "Y");
        assert!(matches!(registry.add_dataset("EUCLID", &euclid, false), Err(FilterError::InvalidDataset { .. })));

        let replaced = custom(serde_json::json!({"VIS": {"lambda": 7150.0, "instrument": "VIS", "group": "optical"}}));
        registry.add_dataset("EUCLID", &replaced, true).unwrap();
        assert_eq!(registry.entry("EUCLID", "VIS").map(|found| found.pivot_angstrom), Some(7150.0));
        assert!(registry.entry("EUCLID", "Y").is_none());

        let colliding = custom(serde_json::json!({
            "H": {"lambda": 17700.0, "instrument": "NISP", "group": "nir"},
            "H_ALT": {"lambda": 17800.0, "instrument": "NISP", "group": "nir", "aliases": ["h"]},
        }));
        let refused = registry.add_dataset("TEST", &colliding, false).unwrap_err();
        assert!(refused.to_string().contains("collides"), "{refused}");
        for invalid in [
            serde_json::json!({"X": {"lambda": -1.0, "instrument": "I", "group": "g"}}),
            serde_json::json!({"X": {"lambda": 5000.0, "instrument": "I", "group": "g", "aliases": [" "]}}),
            serde_json::json!({}),
        ] {
            assert!(matches!(registry.add_dataset("TEST", &custom(invalid.clone()), false), Err(FilterError::InvalidDataset { .. })), "{invalid}");
        }
        assert!(registry.entry("TEST", "X").is_none());

        let from_config = FilterRegistry::with_datasets(&BTreeMap::from([("EUCLID".to_string(), euclid.clone())])).unwrap();
        assert_eq!(from_config.datasets().last().map(String::as_str), Some("EUCLID"));
        let clash = BTreeMap::from([("JWST".to_string(), euclid)]);
        assert!(matches!(FilterRegistry::with_datasets(&clash), Err(FilterError::InvalidDataset { .. })));
    }

    #[test]
    fn a_selection_of_an_unknown_dataset_or_of_nothing_is_an_error() {
        let registry = FilterRegistry::builtin();
        let files = [FilterCandidate { path: "fits/cut/swift_uvw1.fits", header: None }];
        assert_eq!(registry.select(&files, &[selection("LSST", None, None)]).unwrap_err(), FilterError::NothingSelected);
        assert!(matches!(registry.select(&files, &[selection("EUCLID", None, None)]), Err(FilterError::UnknownDataset { .. })));
        let twice = selected(&registry, &["LSST_FDS_g.fits".to_string()], &[selection("LSST", None, None), selection("FDS", None, None)]);
        let found: Vec<(String, f64)> = twice.iter().map(|item| (item.identification.entry.dataset.clone(), item.identification.entry.pivot_angstrom)).collect();
        assert_eq!(found, [("FDS".to_string(), 4750.0), ("LSST".to_string(), 4782.26)]);
        let lower_case = selected(&registry, &["JWST_F444W.fits".to_string()], &[selection("jwst", Some("nircam"), Some("lw"))]);
        assert_eq!(filters_of(&lower_case), ["F444W"]);
    }
}
