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
