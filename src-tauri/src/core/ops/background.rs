use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::imaging::background::{
    deband, detect_band_axis, dispersed_exposure, BackgroundConfig, BackgroundMode, DebandAxis, DebandConfig,
};
use crate::core::imaging::dbe::DbeConfig;
use crate::core::stacking::cfa_guard::cfa_pattern;
use crate::types::constants::{
    MAX_GRID_SIZE, MAX_ITERATIONS, MAX_POLY_DEGREE, MIN_GRID_SIZE, MIN_ITERATIONS, MIN_POLY_DEGREE, MODE_DIVIDE,
};
use crate::types::header::HduHeader;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebandSettings {
    pub sigma_clip: f32,
    pub iterations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "model", content = "config", rename_all = "snake_case")]
pub enum BackgroundModel {
    Polynomial(BackgroundConfig),
    Spline(DbeConfig),
    DebandRows(DebandSettings),
    DebandCols(DebandSettings),
    DebandBoth(DebandSettings),
    DebandAuto(DebandSettings),
}

impl BackgroundModel {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Polynomial(_) => "polynomial",
            Self::Spline(_) => "spline",
            Self::DebandRows(_) => "deband_rows",
            Self::DebandCols(_) => "deband_cols",
            Self::DebandBoth(_) => "deband_both",
            Self::DebandAuto(_) => "deband_auto",
        }
    }
}

pub fn model_for_mode(mode: &str, config: BackgroundConfig, iterations: usize) -> BackgroundModel {
    let settings = DebandSettings { sigma_clip: config.sigma_clip, iterations };
    match mode {
        "deband_rows" => BackgroundModel::DebandRows(settings),
        "deband_cols" => BackgroundModel::DebandCols(settings),
        "deband_both" => BackgroundModel::DebandBoth(settings),
        "deband_auto" => BackgroundModel::DebandAuto(settings),
        _ => BackgroundModel::Polynomial(config),
    }
}

pub enum DebandRequest {
    Fixed(DebandAxis),
    Auto,
}

pub fn deband_request(mode: &str) -> Option<DebandRequest> {
    match mode {
        "deband_rows" => Some(DebandRequest::Fixed(DebandAxis::Rows)),
        "deband_cols" => Some(DebandRequest::Fixed(DebandAxis::Columns)),
        "deband_both" => Some(DebandRequest::Fixed(DebandAxis::Both)),
        "deband_auto" => Some(DebandRequest::Auto),
        _ => None,
    }
}

pub fn deband_axis_token(axis: DebandAxis) -> &'static str {
    match axis {
        DebandAxis::Rows => "rows",
        DebandAxis::Columns => "cols",
        DebandAxis::Both => "both",
    }
}

pub fn deband_history(axis: DebandAxis, detected: bool) -> String {
    let token = deband_axis_token(axis);
    if detected { format!("deband {token} (auto)") } else { format!("deband {token}") }
}

fn dispersed_error(exp_type: &str) -> String {
    format!(
        "De-banding is for imaging frames; this is a dispersed exposure (EXP_TYPE={exp_type}) whose rows and columns carry spectra, so a band model would subtract signal."
    )
}

fn cfa_error(pattern: &str) -> String {
    format!(
        "De-banding is for debayered frames; this is a Bayer mosaic (BAYERPAT={pattern}) whose row and column medians alternate by colour, so a band model would tint the image. Debayer it first (Processing > Debayer)."
    )
}

pub fn deband_guard<'a>(mut headers: impl Iterator<Item = &'a HduHeader> + Clone) -> anyhow::Result<()> {
    if let Some(exp_type) = headers.clone().find_map(dispersed_exposure) {
        anyhow::bail!("{}", dispersed_error(&exp_type));
    }
    if let Some(pattern) = headers.find_map(cfa_pattern) {
        anyhow::bail!("{}", cfa_error(pattern.name()));
    }
    Ok(())
}

pub struct SingleOutput {
    pub corrected: Array2<f32>,
    pub model: Array2<f32>,
    pub deband: Option<(DebandAxis, bool)>,
    pub sample_count: Option<usize>,
    pub rms_residual: Option<f64>,
    pub elapsed_ms: u64,
}

pub fn deband_single(image: &Array2<f32>, request: DebandRequest, sigma_clip: f32, iterations: usize) -> SingleOutput {
    let t0 = std::time::Instant::now();
    let base = DebandConfig { axis: DebandAxis::Both, sigma_clip, iterations: iterations.clamp(1, 5) };
    let (axis, detected) = match request {
        DebandRequest::Fixed(axis) => (axis, false),
        DebandRequest::Auto => (detect_band_axis(image, &base), true),
    };
    let corrected = deband(image, &DebandConfig { axis, ..base });
    let model = Array2::from_shape_fn(image.dim(), |ix| {
        let v = image[ix];
        if v.is_finite() { v - corrected[ix] } else { f32::NAN }
    });
    SingleOutput {
        corrected,
        model,
        deband: Some((axis, detected)),
        sample_count: None,
        rms_residual: None,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    }
}

fn checked_sigma_clip(sigma_clip: f64) -> anyhow::Result<f32> {
    let clip = sigma_clip as f32;
    if !(clip.is_finite() && clip > 0.0) {
        anyhow::bail!("sigma_clip must be a finite number greater than 0, got {}", sigma_clip);
    }
    Ok(clip)
}

fn clamped_background(grid_size: usize, poly_degree: usize, sigma_clip: f32, iterations: usize, mode: BackgroundMode) -> BackgroundConfig {
    BackgroundConfig {
        grid_size: grid_size.clamp(MIN_GRID_SIZE, MAX_GRID_SIZE),
        poly_degree: poly_degree.clamp(MIN_POLY_DEGREE, MAX_POLY_DEGREE),
        sigma_clip,
        iterations: iterations.clamp(MIN_ITERATIONS, MAX_ITERATIONS),
        mode,
    }
}

pub fn background_config(
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: &str,
) -> anyhow::Result<BackgroundConfig> {
    let mode = match mode {
        MODE_DIVIDE => BackgroundMode::Divide,
        _ => BackgroundMode::Subtract,
    };
    Ok(clamped_background(grid_size, poly_degree, checked_sigma_clip(sigma_clip)?, iterations, mode))
}

pub fn checked_background_config(config: &BackgroundConfig) -> anyhow::Result<BackgroundConfig> {
    let clip = checked_sigma_clip(config.sigma_clip as f64)?;
    Ok(clamped_background(config.grid_size, config.poly_degree, clip, config.iterations, config.mode.clone()))
}

pub fn checked_deband_settings(settings: &DebandSettings) -> anyhow::Result<DebandSettings> {
    Ok(DebandSettings {
        sigma_clip: checked_sigma_clip(settings.sigma_clip as f64)?,
        iterations: settings.iterations.clamp(MIN_ITERATIONS, MAX_ITERATIONS),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_for_mode_maps_every_single_file_mode() {
        let config = background_config(6, 2, 2.5, 4, "divide").unwrap();
        let named: Vec<(&str, &str)> = ["subtract", "divide", "neutralize", "deband_rows", "deband_cols", "deband_both", "deband_auto"]
            .into_iter()
            .map(|mode| (mode, model_for_mode(mode, config.clone(), 7).name()))
            .collect();
        assert_eq!(
            named,
            [
                ("subtract", "polynomial"),
                ("divide", "polynomial"),
                ("neutralize", "polynomial"),
                ("deband_rows", "deband_rows"),
                ("deband_cols", "deband_cols"),
                ("deband_both", "deband_both"),
                ("deband_auto", "deband_auto"),
            ]
        );
        let BackgroundModel::DebandAuto(settings) = model_for_mode("deband_auto", config.clone(), 7) else { unreachable!() };
        assert_eq!((settings.sigma_clip, settings.iterations), (2.5, 7));
        let BackgroundModel::Polynomial(kept) = model_for_mode("divide", config, 7) else { unreachable!() };
        assert!(matches!(kept.mode, BackgroundMode::Divide));
        assert_eq!((kept.grid_size, kept.poly_degree, kept.iterations), (6, 2, 4));
    }
}
