pub mod bins;
pub mod cache;
pub mod pick;
pub mod series;
pub mod stitch;
pub mod white_light;
pub mod whtlt;
pub mod transit_spectrum;

pub use cache::open_tso;
pub use pick::{find_series, series_spectrum, TsoPick, MEDIAN_ERROR_FACTOR};
pub use series::series_label;

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

#[cfg(test)]
pub(crate) mod test_support {
    use super::{sample_is_valid, SeriesKey, TsoFile, TsoLayout, TsoSeries, TsoTimeScale, DATAMODEL_ROWS};

    pub struct SynthIntegration {
        pub int_num: i64,
        pub mjd_utc: f64,
        pub bmjd_tdb: f64,
        pub flux: Vec<f64>,
        pub flux_error: Vec<f64>,
        pub dq: Vec<u32>,
        pub npixels: Vec<f64>,
    }

    pub fn synth_integration(int_num: i64, time: f64, flux: &[f64]) -> SynthIntegration {
        SynthIntegration {
            int_num,
            mjd_utc: time,
            bmjd_tdb: time + 0.002,
            flux: flux.to_vec(),
            flux_error: vec![0.5; flux.len()],
            dq: vec![0; flux.len()],
            npixels: vec![7.0; flux.len()],
        }
    }

    pub fn synth_series(
        detector: &str,
        sporder: Option<i64>,
        wavelength_um: &[f64],
        rows: Vec<SynthIntegration>,
        time_scale: TsoTimeScale,
    ) -> TsoSeries {
        let time_source = match time_scale {
            TsoTimeScale::BmjdTdb => "TDB-MID column",
            TsoTimeScale::MjdUtc => "MJD-AVG (UTC, not barycentric)",
            TsoTimeScale::Index => "integration number (no usable times)",
        };
        let n_valid = rows
            .iter()
            .map(|row| {
                let valid = wavelength_um
                    .iter()
                    .zip(&row.flux)
                    .zip(&row.dq)
                    .filter(|((wavelength, flux), dq)| sample_is_valid(**wavelength, **flux, **dq));
                valid.count() as u32
            })
            .collect();
        TsoSeries {
            key: SeriesKey { detector: detector.to_string(), sporder },
            n_integrations: rows.len(),
            n_channels: wavelength_um.len(),
            int_num: rows.iter().map(|row| row.int_num).collect(),
            segment: vec![1; rows.len()],
            file_index: vec![0; rows.len()],
            n_valid,
            mjd_utc_mid: rows.iter().map(|row| row.mjd_utc).collect(),
            bmjd_tdb_mid: rows.iter().map(|row| row.bmjd_tdb).collect(),
            time_scale,
            time_source: time_source.to_string(),
            wavelength_um: wavelength_um.repeat(rows.len()),
            flux: rows.iter().flat_map(|row| row.flux.iter().copied()).collect(),
            flux_error: rows.iter().flat_map(|row| row.flux_error.iter().copied()).collect(),
            dq: rows.iter().flat_map(|row| row.dq.iter().copied()).collect(),
            npixels: rows.iter().flat_map(|row| row.npixels.iter().copied()).collect(),
            grid_um: wavelength_um.to_vec(),
            flux_unit: "Jy".to_string(),
            common_grid: true,
            outlier_detection: None,
            completeness: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn synth_file(file_name: &str, exp_type: &str, filter: &str, level: u8) -> TsoFile {
        TsoFile {
            path: file_name.to_string(),
            file_name: file_name.to_string(),
            sha256: "0".repeat(64),
            level,
            datamodel: Some(DATAMODEL_ROWS.to_string()),
            cal_ver: None,
            exp_type: Some(exp_type.to_string()),
            layout: TsoLayout::Rows,
            nints: None,
            intstart: None,
            intend: None,
            exsegnum: None,
            exsegtot: None,
            instrument: None,
            detector: None,
            filter: Some(filter.to_string()),
            pupil: None,
            grating: None,
            subarray: None,
            target: None,
            expstart: None,
            outlier_detection: None,
            whitelight_log: None,
            whtlt_path: None,
            n_extract1d: 1,
            data_bytes: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::test_support::{synth_file, synth_integration, synth_series};
    use super::*;

    #[test]
    fn tso_types_serialise_with_the_names_the_frontend_reads() {
        let key = SeriesKey { detector: "NRS1".into(), sporder: Some(-1) };
        assert_eq!(serde_json::to_value(&key).unwrap(), json!({"detector": "NRS1", "sporder": -1}));
        let without_order = SeriesKey { detector: "NRS1".into(), sporder: None };
        assert_eq!(serde_json::to_value(&without_order).unwrap(), json!({"detector": "NRS1", "sporder": null}));
        assert_eq!(serde_json::to_value(TsoLayout::Rows).unwrap(), json!("rows"));
        assert_eq!(serde_json::to_value(TsoLayout::Extensions).unwrap(), json!("extensions"));
        assert_eq!(serde_json::to_value(TsoTimeScale::BmjdTdb).unwrap(), json!("bmjd_tdb"));
        assert_eq!(serde_json::to_value(TsoTimeScale::MjdUtc).unwrap(), json!("mjd_utc"));
        assert_eq!(serde_json::to_value(TsoTimeScale::Index).unwrap(), json!("index"));
        let parsed: SeriesKey = serde_json::from_value(json!({"detector": "NIS", "sporder": 2})).unwrap();
        assert_eq!(parsed, SeriesKey { detector: "NIS".into(), sporder: Some(2) });

        let file = serde_json::to_value(synth_file("jw_x1dints.fits", "NRS_BRIGHTOBJ", "CLEAR", 3)).unwrap();
        assert_eq!(file["path"], json!("jw_x1dints.fits"));
        assert_eq!(file["file_name"], json!("jw_x1dints.fits"));
        assert_eq!(file["sha256"], json!("0".repeat(64)));
        assert_eq!(file["level"], json!(3));
        assert_eq!(file["datamodel"], json!("TSOMultiSpecModel"));
        assert_eq!(file["layout"], json!("rows"));
        assert_eq!(file["exp_type"], json!("NRS_BRIGHTOBJ"));
        assert_eq!(file["filter"], json!("CLEAR"));
        assert_eq!(file["instrument"], json!(null));
        assert_eq!(file["detector"], json!(null));
        assert_eq!(file["whitelight_log"], json!(null));
        assert_eq!(file["whtlt_path"], json!(null));
        assert_eq!(file["n_extract1d"], json!(1));
        assert_eq!(file["data_bytes"], json!(0));
        let log = WhiteLightLog { min_um: Some(2.47), max_um: None };
        assert_eq!(serde_json::to_value(&log).unwrap(), json!({"min_um": 2.47, "max_um": null}));
    }

    #[test]
    fn a_sample_is_valid_only_with_a_finite_wavelength_a_finite_flux_and_dq_bit_0_clear() {
        assert!(sample_is_valid(1.0, 2.0, 0));
        assert!(!sample_is_valid(f64::NAN, 2.0, 0));
        assert!(!sample_is_valid(1.0, f64::NAN, 0));
        assert!(!sample_is_valid(1.0, f64::INFINITY, 0));
        assert!(!sample_is_valid(1.0, 2.0, 1));
        assert!(!sample_is_valid(1.0, 2.0, 3));
        assert!(sample_is_valid(1.0, 2.0, 2));
        assert!(sample_is_valid(1.0, -5.0, 0));
    }

    fn two_rows() -> Vec<test_support::SynthIntegration> {
        vec![synth_integration(1, 60000.0, &[1.0, 2.0, 3.0]), synth_integration(2, 60000.5, &[4.0, f64::NAN, 6.0])]
    }

    #[test]
    fn the_synthetic_series_builder_fills_counts_times_and_valid_samples() {
        let series = synth_series("NRS1", Some(0), &[1.0, 1.1, 1.2], two_rows(), TsoTimeScale::BmjdTdb);
        assert_eq!(series.key, SeriesKey { detector: "NRS1".into(), sporder: Some(0) });
        assert_eq!(series.n_integrations, 2);
        assert_eq!(series.n_channels, 3);
        assert_eq!(series.n_valid, vec![3, 2]);
        assert_eq!(series.time(), vec![60000.002, 60000.502]);
        assert_eq!(series.int_num, vec![1, 2]);
        assert!(!series.is_valid(1, 1));
        assert!(series.is_valid(1, 2));
        assert_eq!(series.sample(1, 2), 5);
        assert_eq!(series.grid_um, vec![1.0, 1.1, 1.2]);
        assert!(series.common_grid);
        assert_eq!(series.wavelength_um.len(), 6);
        assert_eq!(series.wavelength_um[3..], [1.0, 1.1, 1.2]);
        assert_eq!(series.flux[..3], [1.0, 2.0, 3.0]);
        assert_eq!(series.flux_error, vec![0.5; 6]);
        assert_eq!(series.dq, vec![0; 6]);
        assert_eq!(series.npixels, vec![7.0; 6]);
        assert_eq!(series.mjd_utc_mid, vec![60000.0, 60000.5]);
        assert_eq!(series.segment, vec![1, 1]);
        assert_eq!(series.file_index, vec![0, 0]);
        assert_eq!(series.flux_unit, "Jy");
        assert_eq!(series.time_source, "TDB-MID column");
        assert!(series.completeness.is_empty() && series.notes.is_empty() && series.warnings.is_empty());

        let by_index = synth_series("NRS1", Some(0), &[1.0, 1.1, 1.2], two_rows(), TsoTimeScale::Index);
        assert_eq!(by_index.time(), vec![1.0, 2.0]);
        assert_eq!(by_index.time_source, "integration number (no usable times)");
        let by_utc = synth_series("NRS1", Some(0), &[1.0, 1.1, 1.2], two_rows(), TsoTimeScale::MjdUtc);
        assert_eq!(by_utc.time(), vec![60000.0, 60000.5]);
        assert_eq!(by_utc.time_source, "MJD-AVG (UTC, not barycentric)");
    }
}
