use std::time::Instant;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::cmd::common::blocking_cmd;
use crate::cmd::cube::APERTURE_SUBSAMPLES;
use crate::core::analysis::line_measure::{measure_line, with_velocity, ContinuumWindows, LineMeasurement, LineModel};
use crate::core::astrometry::spectral::{
    air_formula_applies, air_to_vacuum_um, AxisKind, SpectralAxis, VelocityConvention, AIR_FORMULA_MIN_UM,
    AIR_VACUUM_FORMULA,
};
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::cube::lazy::LazyCube;
use crate::core::imaging::region::RegionShape;
use crate::types::constants::{HEADER_BUNIT, RES_SOURCE};

pub const MAX_LINE_CHANNEL: usize = 1 << 24;
pub const NATIVE_FLUX_UNIT: &str = "native";
pub const DEFAULT_CONVENTION: &str = "optical";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SpectrumSource {
    Pixel {
        x: usize,
        y: usize,
    },
    Region {
        shape: RegionShape,
        #[serde(default)]
        background: Option<RegionShape>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineRequest {
    pub z0: usize,
    pub z1: usize,
    pub continuum: Option<ContinuumWindows>,
    pub rest_um: Option<f64>,
    pub convention: VelocityConvention,
    pub model: LineModel,
    pub velocity_shift_kms: Option<f64>,
}

fn bunit(cube: &LazyCube) -> Option<String> {
    cube.header
        .get(HEADER_BUNIT)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn measured_axis_values(axis: &SpectralAxis, notes: &mut Vec<String>) -> Vec<f64> {
    if axis.kind != AxisKind::Awav {
        return axis.values.clone();
    }
    if axis.values.iter().any(|&w| w.is_finite() && !air_formula_applies(w)) {
        notes.push(format!(
            "some air wavelengths are below {} um where {} does not apply: left unchanged",
            AIR_FORMULA_MIN_UM, AIR_VACUUM_FORMULA
        ));
    }
    notes.push(format!("air wavelengths converted to vacuum with {}", AIR_VACUUM_FORMULA));
    axis.values.iter().map(|&w| air_to_vacuum_um(w)).collect()
}

fn extract_spectrum(cube: &LazyCube, source: &SpectrumSource, notes: &mut Vec<String>) -> anyhow::Result<Vec<f32>> {
    match source {
        SpectrumSource::Pixel { x, y } => cube.extract_spectrum_at(*y, *x),
        SpectrumSource::Region { shape, background } => {
            let spectrum = cube.extract_spectrum_aperture(shape, background.as_ref(), APERTURE_SUBSAMPLES)?;
            notes.push(format!(
                "{} region: flux summed over {:.1} pixels{}",
                shape.kind(),
                spectrum.npix,
                if spectrum.bg_per_pixel.is_some() {
                    format!(", sky from {} background pixels subtracted per channel", spectrum.n_bg)
                } else {
                    String::new()
                }
            ));
            Ok(spectrum.sum)
        }
    }
}

pub(crate) fn measure_line_json(path: &str, source: &SpectrumSource, request: &LineRequest) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let cube = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    let axis = cube.spectral_axis().map_err(anyhow::Error::msg)?;
    if !axis.kind.is_spectral() {
        bail!(
            "CTYPE3 '{}' is not a spectral axis: the line measurement needs a wavelength, frequency or velocity axis",
            axis.ctype
        );
    }
    let mut notes = Vec::new();
    let axis_values = measured_axis_values(&axis, &mut notes);
    let spectrum = extract_spectrum(&cube, source, &mut notes)?;
    let mut m: LineMeasurement =
        measure_line(&axis_values, &spectrum, (request.z0, request.z1), request.continuum, request.model)
            .map_err(anyhow::Error::msg)?;
    m.axis_unit = axis.unit.clone();
    m.flux_unit = format!("{} x {}", bunit(&cube).unwrap_or_else(|| NATIVE_FLUX_UNIT.to_string()), axis.unit);
    notes.append(&mut m.notes);
    m.notes = notes;
    match (request.rest_um.or(axis.rest_wavelength_um), axis.kind) {
        (Some(rest), AxisKind::Wave | AxisKind::Awav | AxisKind::Freq) => {
            with_velocity(&mut m, rest, request.convention, axis.kind, request.velocity_shift_kms);
        }
        (None, AxisKind::Wave | AxisKind::Awav | AxisKind::Freq) => {
            m.notes.push("no rest wavelength: velocities not computed".to_string());
        }
        (_, kind) if kind.is_velocity() => {
            m.notes.push(format!("axis {} is already a velocity axis: centroid and sigma are in km/s", axis.ctype));
        }
        _ => {}
    }
    m.elapsed_ms = t0.elapsed().as_millis() as u64;
    let mut value = serde_json::to_value(&m)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(RES_SOURCE.to_string(), serde_json::to_value(source)?);
    }
    Ok(value)
}

fn validate_window(window: (usize, usize)) -> Result<(), String> {
    if window.0 > window.1 || window.1 >= MAX_LINE_CHANNEL {
        return Err(format!("continuum window {}..={} is not a valid channel range", window.0, window.1));
    }
    Ok(())
}

#[tauri::command]
pub async fn measure_spectral_line_cmd(
    path: String,
    source: SpectrumSource,
    z0: usize,
    z1: usize,
    continuum: Option<ContinuumWindows>,
    rest_um: Option<f64>,
    convention: Option<String>,
    model: Option<String>,
    velocity_shift_kms: Option<f64>,
) -> Result<serde_json::Value, String> {
    if path.trim().is_empty() {
        return Err("path must name a cube file, got an empty string".to_string());
    }
    if z0 > z1 || z1 >= MAX_LINE_CHANNEL {
        return Err(format!("line range z0..=z1 must satisfy z0 <= z1 < {}, got {}..={}", MAX_LINE_CHANNEL, z0, z1));
    }
    if let Some(windows) = continuum {
        validate_window(windows.0)?;
        validate_window(windows.1)?;
    }
    if let Some(rest) = rest_um {
        if !(rest.is_finite() && rest > 0.0) {
            return Err(format!("rest_um must be a positive number of micrometres, got {}", rest));
        }
    }
    if let Some(shift) = velocity_shift_kms {
        if !shift.is_finite() {
            return Err(format!("velocity_shift_kms must be a finite number of km/s, got {}", shift));
        }
    }
    let convention = VelocityConvention::parse(convention.as_deref().unwrap_or(DEFAULT_CONVENTION))?;
    let model = LineModel::parse(model.as_deref().unwrap_or(LineModel::None.name()))?;
    let request = LineRequest { z0, z1, continuum, rest_um, convention, model, velocity_shift_kms };
    blocking_cmd!(measure_line_json(&path, &source, &request))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cube::lazy::{region_pixels, test_support::*};

    const LINE_RANGE: (usize, usize) = (8, 32);
    const WINDOWS: ContinuumWindows = ((0, 5), (34, 39));
    const EXPECTED_FLUX: f64 = 7.5199e-3;
    const CONTINUUM_LEVEL_TOLERANCE: f64 = 5e-6;

    fn request(model: LineModel) -> LineRequest {
        LineRequest {
            z0: LINE_RANGE.0,
            z1: LINE_RANGE.1,
            continuum: Some(WINDOWS),
            rest_um: None,
            convention: VelocityConvention::Optical,
            model,
            velocity_shift_kms: None,
        }
    }

    fn centre_pixel() -> SpectrumSource {
        SpectrumSource::Pixel { x: LINE_DISK_CENTRE as usize, y: LINE_DISK_CENTRE as usize }
    }

    fn write(dir: &tempfile::TempDir, name: &str, noise: f32) -> String {
        let path = dir.path().join(name);
        write_line_cube(&path, noise);
        path.to_str().unwrap().to_string()
    }

    fn number(value: &serde_json::Value, key: &str) -> f64 {
        value[key].as_f64().unwrap_or_else(|| panic!("{} missing in {}", key, value))
    }

    #[test]
    fn the_centre_pixel_reports_the_analytic_line_with_velocity_and_fit() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "line.fits", 0.0);
        let value = measure_line_json(&path, &centre_pixel(), &request(LineModel::Gaussian)).unwrap();
        assert_eq!(value["z0"], 8);
        assert_eq!(value["z1"], 32);
        assert_eq!(value["n_channels"], 25);
        assert_eq!(value["axis_unit"], "um");
        assert_eq!(value["flux_unit"], "Jy/beam x um");
        assert!((number(&value, "centroid") - LINE_REST_UM).abs() < 1e-6);
        assert!((number(&value, "flux") - EXPECTED_FLUX).abs() < 0.01 * EXPECTED_FLUX);
        assert!((number(&value, "equivalent_width") + EXPECTED_FLUX).abs() < 0.01 * EXPECTED_FLUX);
        assert!((number(&value, "sigma") - 0.003).abs() < 0.02 * 0.003);
        assert!((number(&value, "continuum_level") - 1.0).abs() < CONTINUUM_LEVEL_TOLERANCE);
        let velocity = &value["velocity"];
        assert!(number(velocity, "centroid_kms").abs() < 0.1, "{}", velocity);
        assert_eq!(velocity["convention"], "optical");
        assert_eq!(number(velocity, "rest_um"), LINE_REST_UM);
        assert_eq!(number(velocity, "shift_applied_kms"), 0.0);
        let fit = &value["fit"];
        assert_eq!(fit["converged"], true);
        assert!((number(fit, "amplitude") - 1.0).abs() < 1e-4, "{}", fit);
        assert!((number(fit, "centre") - LINE_REST_UM).abs() < 1e-4, "{}", fit);
        assert!((number(fit, "sigma") - 0.003).abs() < 1e-4, "{}", fit);
        assert_eq!(value[RES_SOURCE]["kind"], "pixel");
        assert_eq!(value[RES_SOURCE]["x"], 16);
        assert!(value["elapsed_ms"].is_u64());
        assert!(value["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("rest wavelength")));
    }

    #[test]
    fn a_noisy_pixel_is_fitted_within_three_formal_sigma_and_shifted_velocities_are_noted() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "noisy.fits", 0.05);
        let mut req = request(LineModel::Gaussian);
        req.velocity_shift_kms = Some(-7.25);
        req.convention = VelocityConvention::Relativistic;
        let value = measure_line_json(&path, &centre_pixel(), &req).unwrap();
        let fit = &value["fit"];
        assert_eq!(fit["converged"], true, "{}", fit);
        for (key, err_key, truth) in [("amplitude", "amplitude_err", 1.0), ("centre", "centre_err", LINE_REST_UM), ("sigma", "sigma_err", 0.003)] {
            let pull = (number(fit, key) - truth) / number(fit, err_key);
            assert!(pull.is_finite() && pull.abs() < 3.0, "{} pull {} in {}", key, pull, fit);
        }
        let velocity = &value["velocity"];
        assert_eq!(number(velocity, "shift_applied_kms"), -7.25);
        assert_eq!(velocity["convention"], "relativistic");
        assert!(value["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("shifted by")));
    }

    #[test]
    fn a_pixel_outside_the_disc_has_no_significant_flux() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "noisy.fits", 0.05);
        let value = measure_line_json(&path, &SpectrumSource::Pixel { x: 2, y: 2 }, &request(LineModel::None)).unwrap();
        let flux = number(&value, "flux");
        let flux_err = number(&value, "flux_err");
        assert!(flux_err > 0.0);
        assert!(flux.abs() < 3.0 * flux_err, "flux={} err={}", flux, flux_err);
        assert!(number(&value, "snr").abs() < 3.0);
        assert!(value["fit"].is_null());
    }

    #[test]
    fn a_region_source_sums_the_disc_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "line.fits", 0.0);
        let pixel = measure_line_json(&path, &centre_pixel(), &request(LineModel::None)).unwrap();
        let disk = RegionShape::Circle { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r: LINE_DISK_RADIUS };
        let source = SpectrumSource::Region { shape: disk.clone(), background: None };
        let region = measure_line_json(&path, &source, &request(LineModel::None)).unwrap();
        let line_weight: f64 = region_pixels(&disk, LINE_CUBE_SIZE, LINE_CUBE_SIZE, APERTURE_SUBSAMPLES)
            .unwrap()
            .iter()
            .filter(|p| inside_disk(p.y, p.x))
            .map(|p| p.weight as f64)
            .sum();
        let expected = number(&pixel, "flux") * line_weight;
        let flux = number(&region, "flux");
        assert!((flux - expected).abs() < 0.01 * expected, "flux={} expected={}", flux, expected);
        assert!((flux - number(&pixel, "flux") * disk_pixel_count() as f64).abs() < 0.05 * expected);
        assert!((number(&region, "centroid") - LINE_REST_UM).abs() < 1e-6);
        assert_eq!(region[RES_SOURCE]["kind"], "region");
        assert_eq!(region[RES_SOURCE]["shape"]["shape"], "circle");
        assert!(region["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("flux summed over")));
    }

    #[test]
    fn bad_ranges_models_conventions_and_sources_are_refused_with_a_sentence() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "line.fits", 0.0);
        let mut req = request(LineModel::None);
        req.z1 = LINE_CUBE_DEPTH;
        let err = measure_line_json(&path, &centre_pixel(), &req).unwrap_err().to_string();
        assert!(err.contains("invalid for a spectrum"), "{}", err);
        let err = measure_line_json(&path, &SpectrumSource::Pixel { x: 99, y: 0 }, &request(LineModel::None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("out of bounds"), "{}", err);
        let point = SpectrumSource::Region { shape: RegionShape::Point { x: 1.0, y: 1.0 }, background: None };
        let err = measure_line_json(&path, &point, &request(LineModel::None)).unwrap_err().to_string();
        assert!(err.contains("no area"), "{}", err);
        assert!(LineModel::parse("lorentz").unwrap_err().contains("unknown line model"));
        assert!(VelocityConvention::parse("sideways").unwrap_err().contains("unknown velocity convention"));
        assert!(validate_window((5, 2)).unwrap_err().contains("continuum window"));
    }

    #[test]
    fn a_spectrum_source_deserializes_from_the_frontend_shape() {
        let pixel: SpectrumSource = serde_json::from_str(r#"{"kind":"pixel","x":3,"y":4}"#).unwrap();
        assert_eq!(pixel, SpectrumSource::Pixel { x: 3, y: 4 });
        let region: SpectrumSource =
            serde_json::from_str(r#"{"kind":"region","shape":{"shape":"circle","x":1.0,"y":2.0,"r":3.0},"background":null}"#)
                .unwrap();
        assert_eq!(
            region,
            SpectrumSource::Region { shape: RegionShape::Circle { x: 1.0, y: 2.0, r: 3.0 }, background: None }
        );
        assert!(serde_json::from_str::<SpectrumSource>(r#"{"kind":"line","x":1}"#).is_err());
    }

    #[test]
    fn a_non_spectral_axis_is_refused_and_air_wavelengths_are_converted() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("legacy.fits");
        write_cube(&legacy, 4, 4, 8, &[("CRVAL3", "1.0"), ("CDELT3", "0.1")], |z, _, _| z as f32);
        let mut req = request(LineModel::None);
        req.z0 = 1;
        req.z1 = 6;
        req.continuum = None;
        let err = measure_line_json(legacy.to_str().unwrap(), &SpectrumSource::Pixel { x: 1, y: 1 }, &req)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a spectral axis"), "{}", err);

        let air = dir.path().join("air.fits");
        write_line_cube_with_cards(&air, &[("CTYPE3", "'AWAV'")]);
        let value = measure_line_json(air.to_str().unwrap(), &centre_pixel(), &request(LineModel::None)).unwrap();
        let centroid = number(&value, "centroid");
        assert!((centroid - air_to_vacuum_um(LINE_REST_UM)).abs() < 1e-6, "centroid={}", centroid);
        assert!(value["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("converted to vacuum")));
    }
}
