use serde::{Deserialize, Serialize};

pub const DATAMODEL_ROWS: &str = "TSOMultiSpecModel";
pub const DATAMODEL_EXTENSIONS: &str = "MultiSpecModel";
pub const DQ_DO_NOT_USE: u32 = 1;
pub const GRID_TOLERANCE_UM: f64 = 1e-9;
pub const UNKNOWN_DETECTOR: &str = "UNKNOWN";
pub const UNKNOWN_UNIT: &str = "unknown";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SeriesKey {
    pub detector: String,
    pub sporder: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TsoLayout {
    Rows,
    Extensions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TsoTimeScale {
    BmjdTdb,
    MjdUtc,
    Index,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhiteLightLog {
    pub min_um: Option<f64>,
    pub max_um: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TsoFile {
    pub path: String,
    pub file_name: String,
    pub sha256: String,
    pub level: u8,
    pub datamodel: Option<String>,
    pub cal_ver: Option<String>,
    pub exp_type: Option<String>,
    pub layout: TsoLayout,
    pub nints: Option<i64>,
    pub intstart: Option<i64>,
    pub intend: Option<i64>,
    pub exsegnum: Option<i64>,
    pub exsegtot: Option<i64>,
    pub instrument: Option<String>,
    pub detector: Option<String>,
    pub filter: Option<String>,
    pub pupil: Option<String>,
    pub grating: Option<String>,
    pub subarray: Option<String>,
    pub target: Option<String>,
    pub expstart: Option<f64>,
    pub outlier_detection: Option<String>,
    pub whitelight_log: Option<WhiteLightLog>,
    pub whtlt_path: Option<String>,
    pub n_extract1d: usize,
    pub data_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExposureCompleteness {
    pub exposure: String,
    pub nints: Option<i64>,
    pub found: usize,
    pub missing: Vec<(i64, i64)>,
    pub complete: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct TsoSeries {
    pub key: SeriesKey,
    pub n_integrations: usize,
    pub n_channels: usize,
    pub int_num: Vec<i64>,
    pub segment: Vec<i64>,
    pub file_index: Vec<u32>,
    pub n_valid: Vec<u32>,
    pub mjd_utc_mid: Vec<f64>,
    pub bmjd_tdb_mid: Vec<f64>,
    pub time_scale: TsoTimeScale,
    pub time_source: String,
    pub wavelength_um: Vec<f64>,
    pub flux: Vec<f64>,
    pub flux_error: Vec<f64>,
    pub dq: Vec<u32>,
    pub npixels: Vec<f64>,
    pub grid_um: Vec<f64>,
    pub flux_unit: String,
    pub common_grid: bool,
    pub outlier_detection: Option<String>,
    pub completeness: Vec<ExposureCompleteness>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TsoSet {
    pub files: Vec<TsoFile>,
    pub series: Vec<TsoSeries>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn sample_is_valid(wavelength_um: f64, flux: f64, dq: u32) -> bool {
    wavelength_um.is_finite() && flux.is_finite() && dq & DQ_DO_NOT_USE == 0
}

impl TsoSeries {
    pub fn sample(&self, row: usize, channel: usize) -> usize {
        row * self.n_channels + channel
    }

    pub fn is_valid(&self, row: usize, channel: usize) -> bool {
        let sample = self.sample(row, channel);
        sample_is_valid(self.wavelength_um[sample], self.flux[sample], self.dq[sample])
    }

    pub fn time(&self) -> Vec<f64> {
        match self.time_scale {
            TsoTimeScale::BmjdTdb => self.bmjd_tdb_mid.clone(),
            TsoTimeScale::MjdUtc => self.mjd_utc_mid.clone(),
            TsoTimeScale::Index => self.int_num.iter().map(|&number| number as f64).collect(),
        }
    }
}
