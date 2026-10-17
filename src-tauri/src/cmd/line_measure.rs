use std::time::Instant;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::cmd::common::blocking_cmd;
use crate::cmd::cube::{err_source, APERTURE_SUBSAMPLES, ERR_SOURCE_NONE};
use crate::core::analysis::line_measure::{
    measure_line_with_errors, resolve_windows, with_velocity, ContinuumWindows, LineMeasurement, LineModel,
};
use crate::core::analysis::spectrum1d::{
    vacuum_to_air_morton_um, Spectrum1d, Spectrum1dSource, WavelengthFrame, CSV_ERR_COLUMN, MORTON_AIR_FORMULA,
};
use crate::core::analysis::x1d::{COL_FLUX_ERROR, EXTNAME_EXTRACT1D};
use crate::core::astrometry::spectral::{
    air_formula_applies, air_to_vacuum_um, AxisKind, SpectralAxis, VelocityConvention, AIR_FORMULA_MIN_UM,
    AIR_VACUUM_FORMULA,
};
use crate::core::cube::companions::{open_cube_set, CubeSet};
use crate::core::cube::lazy::{ApertureSpectrum, LazyCube};
use crate::core::cube::linefit::dq_flagged;
use crate::core::imaging::region::RegionShape;
use crate::infra::fits::spectrum1d::load_spectrum1d;
use crate::types::constants::{EXTNAME_DQ, EXTNAME_ERR, HEADER_BUNIT, RES_SOURCE};

pub const MAX_LINE_CHANNEL: usize = 1 << 24;
pub const NATIVE_FLUX_UNIT: &str = "native";
pub const REGION_SUM_UNIT_SUFFIX: &str = " x pix";
pub const DEFAULT_CONVENTION: &str = "optical";
pub const FRAME_AIR: &str = "air";
pub const FRAME_VACUUM: &str = "vacuum";
pub const KEY_ERR_SOURCE: &str = "err_source";

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
    File {
        #[serde(default)]
        hdu: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasureOptions {
    pub use_err: bool,
    pub use_dq: bool,
    pub wavelength_frame: Option<WavelengthFrame>,
}

impl Default for MeasureOptions {
    fn default() -> Self {
        MeasureOptions { use_err: true, use_dq: true, wavelength_frame: None }
    }
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

struct CubeSpectrum {
    flux: Vec<f32>,
    err: Option<Vec<f64>>,
    excluded: Option<Vec<bool>>,
    err_source: String,
}

fn widened(values: Vec<f32>) -> Vec<f64> {
    values.into_iter().map(f64::from).collect()
}

fn used_companions<'a>(
    set: &'a CubeSet,
    options: &MeasureOptions,
    notes: &mut Vec<String>,
) -> (Option<&'a LazyCube>, Option<&'a LazyCube>) {
    let err = match (set.err.as_deref(), options.use_err) {
        (Some(err), true) => {
            notes.push(format!("flux error from the {} cube (HDU {})", EXTNAME_ERR, err.hdu_index));
            Some(err)
        }
        (Some(_), false) => {
            notes.push("ERR present but not used (use_err = false): flux error from the continuum scatter".to_string());
            None
        }
        (None, _) => None,
    };
    let dq = match (set.dq.as_deref(), options.use_dq) {
        (Some(dq), true) => Some(dq),
        (Some(_), false) => {
            notes.push("DQ present but not used (use_dq = false): no channel excluded by its flags".to_string());
            None
        }
        (None, _) => None,
    };
    (err, dq)
}

fn measured_channels(request: &LineRequest, depth: usize) -> Vec<usize> {
    let line = (request.z0, request.z1);
    let windows = (line.0 <= line.1 && line.1 < depth)
        .then(|| resolve_windows(request.continuum, line, depth, &mut Vec::new()).ok())
        .flatten();
    let mut channels: Vec<usize> = (line.0..=line.1)
        .chain(windows.into_iter().flat_map(|(a, b)| (a.0..=a.1).chain(b.0..=b.1)))
        .filter(|&z| z < depth)
        .collect();
    channels.sort_unstable();
    channels.dedup();
    channels
}

fn region_area_note(shape: &RegionShape, spectrum: &ApertureSpectrum, measured: &[usize], incomplete: &[bool]) -> String {
    let persistent = spectrum.dropouts.dropped_in_every(measured);
    let area = if persistent == 0 {
        format!("{:.1} pixels", spectrum.npix)
    } else {
        let kept = measured.iter().find(|&&z| !incomplete[z]).map_or(spectrum.npix, |&z| spectrum.used_weight[z]);
        format!(
            "{:.1} of {:.1} pixels (spaxels dropped in every measured channel by DQ bit 0 or non-finite flux: {})",
            kept, spectrum.npix, persistent
        )
    };
    let sky = if spectrum.bg_per_pixel.is_some() {
        format!(", sky from {} background pixels subtracted per channel", spectrum.n_bg)
    } else {
        String::new()
    };
    format!("{} region: flux summed over {}{}", shape.kind(), area, sky)
}

fn aperture_exclusions(
    incomplete: Vec<bool>,
    measured: &[usize],
    request: &LineRequest,
    dq: Option<&LazyCube>,
    notes: &mut Vec<String>,
) -> anyhow::Result<Option<Vec<bool>>> {
    let excluded = measured.iter().filter(|&&z| incomplete[z]).count();
    if let Some(dq) = dq {
        notes.push(format!(
            "{} cube (HDU {}): aperture spaxels with bit 0 (DO_NOT_USE) dropped per channel",
            EXTNAME_DQ, dq.hdu_index
        ));
    }
    if dq.is_none() && excluded == 0 {
        return Ok(None);
    }
    notes.push(format!(
        "{} of the {} line and continuum channels lost aperture spaxels (DQ bit 0 or non-finite flux) that other measured channels kept: excluded",
        excluded,
        measured.len()
    ));
    let mut line = (request.z0..=request.z1).filter(|&z| z < incomplete.len()).peekable();
    if line.peek().is_some() && line.all(|z| incomplete[z]) {
        bail!(
            "every line channel {}..={} lost aperture spaxels (DQ bit 0 or non-finite flux) that other measured channels kept: the region does not cover one spaxel set across the line and continuum channels; move the region off the dropped spaxels or measure with use_dq = false",
            request.z0,
            request.z1
        );
    }
    Ok(Some(incomplete))
}

fn extract_spectrum(
    set: &CubeSet,
    source: &SpectrumSource,
    request: &LineRequest,
    options: &MeasureOptions,
    notes: &mut Vec<String>,
) -> anyhow::Result<CubeSpectrum> {
    let mut companion_notes = Vec::new();
    let (err, dq) = used_companions(set, options, &mut companion_notes);
    let spectrum = match source {
        SpectrumSource::File { .. } => bail!("a spectrum file source has no cube spectrum to extract"),
        SpectrumSource::Pixel { x, y } => {
            let flux = set.sci.extract_spectrum_at(*y, *x)?;
            let errors = err.map(|c| c.extract_spectrum_at(*y, *x)).transpose()?.map(widened);
            let flags = dq.map(|c| c.extract_spectrum_at(*y, *x)).transpose()?;
            if let Some(dq) = dq {
                companion_notes.push(format!(
                    "{} cube (HDU {}): channels of this spaxel with bit 0 (DO_NOT_USE) set are excluded",
                    EXTNAME_DQ, dq.hdu_index
                ));
            }
            let excluded = flags.map(|flags| flags.into_iter().map(dq_flagged).collect());
            CubeSpectrum { flux, err: errors, excluded, err_source: err_source(err) }
        }
        SpectrumSource::Region { shape, background } => {
            let spectrum =
                set.sci.extract_spectrum_aperture_with(shape, background.as_ref(), APERTURE_SUBSAMPLES, err, dq)?;
            let measured = measured_channels(request, spectrum.sum.len());
            let incomplete = spectrum.dropouts.incomplete_channels(&measured);
            notes.push(region_area_note(shape, &spectrum, &measured, &incomplete));
            let excluded = aperture_exclusions(incomplete, &measured, request, dq, &mut companion_notes)?;
            CubeSpectrum { flux: spectrum.sum, err: spectrum.err.map(widened), excluded, err_source: err_source(err) }
        }
    };
    notes.append(&mut companion_notes);
    Ok(spectrum)
}

fn add_velocity(m: &mut LineMeasurement, axis: &SpectralAxis, rest_um: Option<f64>, request: &LineRequest) {
    match (rest_um, axis.kind) {
        (Some(rest), AxisKind::Wave | AxisKind::Awav | AxisKind::Freq) => {
            with_velocity(m, rest, request.convention, axis.kind, axis.specsys.as_deref(), request.velocity_shift_kms);
        }
        (None, AxisKind::Wave | AxisKind::Awav | AxisKind::Freq) => {
            m.notes.push("no rest wavelength: velocities not computed".to_string());
        }
        (_, kind) if kind.is_velocity() => {
            m.notes.push(format!("axis {} is already a velocity axis: centroid and sigma are in km/s", axis.ctype));
        }
        _ => {}
    }
}

fn measure_cube_line(
    path: &str,
    source: &SpectrumSource,
    request: &LineRequest,
    options: &MeasureOptions,
) -> anyhow::Result<(LineMeasurement, String)> {
    if options.wavelength_frame.is_some() {
        bail!("wavelength_frame applies to spectrum files only: a cube's air or vacuum frame comes from its CTYPE3");
    }
    let set = open_cube_set(path)?;
    let cube = &set.sci;
    let axis = cube.spectral_axis().map_err(anyhow::Error::msg)?;
    if !axis.kind.is_spectral() {
        bail!(
            "CTYPE3 '{}' is not a spectral axis: the line measurement needs a wavelength, frequency or velocity axis",
            axis.ctype
        );
    }
    let mut notes = set.notes.clone();
    let axis_values = measured_axis_values(&axis, &mut notes);
    let spectrum = extract_spectrum(&set, source, request, options, &mut notes)?;
    let mut m = measure_line_with_errors(
        &axis_values,
        &spectrum.flux,
        spectrum.err.as_deref(),
        spectrum.excluded.as_deref(),
        (request.z0, request.z1),
        request.continuum,
        request.model,
    )
    .map_err(anyhow::Error::msg)?;
    m.axis_unit = axis.unit.clone();
    let native_unit = bunit(cube).unwrap_or_else(|| NATIVE_FLUX_UNIT.to_string());
    let spectrum_unit = if matches!(source, SpectrumSource::Region { .. }) {
        format!("{}{}", native_unit, REGION_SUM_UNIT_SUFFIX)
    } else {
        native_unit
    };
    m.flux_unit = format!("{} x {}", spectrum_unit, axis.unit);
    notes.append(&mut m.notes);
    m.notes = notes;
    add_velocity(&mut m, &axis, request.rest_um.or(axis.rest_wavelength_um), request);
    Ok((m, spectrum.err_source))
}

fn parse_wavelength_frame(name: &str) -> Result<WavelengthFrame, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        FRAME_AIR => Ok(WavelengthFrame::Air),
        FRAME_VACUUM => Ok(WavelengthFrame::Vacuum),
        _ => Err(format!("unknown wavelength frame '{}': use '{}' or '{}'", name, FRAME_AIR, FRAME_VACUUM)),
    }
}

fn document_frame(found: WavelengthFrame, requested: Option<WavelengthFrame>) -> Result<WavelengthFrame, String> {
    match requested {
        None => Ok(found),
        Some(WavelengthFrame::NotWavelength) => {
            Err(format!("wavelength_frame must be '{}' or '{}'", FRAME_AIR, FRAME_VACUUM))
        }
        Some(_) if found == WavelengthFrame::NotWavelength => {
            Err("the spectral axis is not a wavelength axis: an air or vacuum frame does not apply".to_string())
        }
        Some(frame) => Ok(frame),
    }
}

fn frame_text(frame: WavelengthFrame) -> &'static str {
    match frame {
        WavelengthFrame::Air => "air wavelengths",
        WavelengthFrame::Vacuum => "vacuum wavelengths",
        WavelengthFrame::NotWavelength => "a spectral axis that is not a wavelength",
    }
}

fn document_hdu(source: &Spectrum1dSource) -> Option<usize> {
    match source {
        Spectrum1dSource::X1d { hdu, .. } | Spectrum1dSource::Wcs1d { hdu, .. } => Some(*hdu),
        Spectrum1dSource::Csv { .. } => None,
    }
}

fn document_label(source: &Spectrum1dSource) -> String {
    match source {
        Spectrum1dSource::X1d { hdu, .. } => format!("{} table in HDU {}", EXTNAME_EXTRACT1D, hdu),
        Spectrum1dSource::Wcs1d { hdu, .. } => format!("1D array in HDU {}", hdu),
        Spectrum1dSource::Csv { .. } => "CSV file".to_string(),
    }
}

fn document_err_source(source: &Spectrum1dSource, err_used: bool) -> String {
    match (source, err_used) {
        (_, false) => ERR_SOURCE_NONE.to_string(),
        (Spectrum1dSource::X1d { hdu, .. }, true) => format!("{} column (HDU {})", COL_FLUX_ERROR, hdu),
        (Spectrum1dSource::Wcs1d { .. }, true) => format!("{} extension", EXTNAME_ERR),
        (Spectrum1dSource::Csv { .. }, true) => format!("{} column (CSV)", CSV_ERR_COLUMN),
    }
}

fn document_errors<'a>(doc: &'a Spectrum1d, use_err: bool, notes: &mut Vec<String>) -> Option<&'a [f64]> {
    match (doc.err.as_deref(), use_err) {
        (Some(err), true) => Some(err),
        (Some(_), false) => {
            notes.push("ERR present but not used (use_err = false): flux error from the continuum scatter".to_string());
            None
        }
        (None, true) => {
            notes.push("the file has no ERR or FLUX_ERROR: flux error from the continuum scatter".to_string());
            None
        }
        (None, false) => None,
    }
}

fn document_exclusions(doc: &Spectrum1d, use_dq: bool, notes: &mut Vec<String>) -> Option<Vec<bool>> {
    let dq = doc.dq.as_ref()?;
    if !use_dq {
        notes.push("DQ present but not used (use_dq = false): no channel excluded by its flags".to_string());
        return None;
    }
    let mask = doc.dq_table.exclusion_mask();
    if mask == 0 {
        notes.push("DQ present but its flag table is unknown: no channel excluded by its flags".to_string());
        return None;
    }
    Some(dq.iter().map(|&bits| bits & mask != 0).collect())
}

fn measured_flux(flux: &[f64], notes: &mut Vec<String>) -> Vec<f32> {
    let narrowed: Vec<f32> = flux.iter().map(|&v| v as f32).collect();
    if flux.iter().zip(&narrowed).any(|(&wide, &narrow)| wide.is_finite() && f64::from(narrow) != wide) {
        notes.push("flux measured at f32 precision: about 7 significant digits of the f64 values kept".to_string());
    }
    narrowed
}

fn air_rest_um(rest_um: f64, notes: &mut Vec<String>) -> f64 {
    if !air_formula_applies(rest_um) {
        notes.push(format!(
            "rest wavelength {} um is below {} um where {} does not apply: left as vacuum",
            rest_um, AIR_FORMULA_MIN_UM, MORTON_AIR_FORMULA
        ));
        return rest_um;
    }
    let air = vacuum_to_air_morton_um(rest_um);
    notes.push(format!(
        "vacuum rest wavelength {} um converted to {:.7} um in air with {} to match the air wavelength axis",
        rest_um, air, MORTON_AIR_FORMULA
    ));
    air
}

fn measure_file_line(
    path: &str,
    hdu: Option<usize>,
    request: &LineRequest,
    options: &MeasureOptions,
) -> anyhow::Result<(LineMeasurement, SpectrumSource, String)> {
    let doc = load_spectrum1d(path, hdu)?;
    let frame = document_frame(doc.wavelength_frame, options.wavelength_frame).map_err(anyhow::Error::msg)?;
    let label = document_label(&doc.source);
    let mut notes = vec![if frame == doc.wavelength_frame {
        format!("spectrum file, {}: {}", label, frame_text(frame))
    } else {
        format!(
            "spectrum file, {}: {} set by the request (the file reads as {})",
            label,
            frame_text(frame),
            frame_text(doc.wavelength_frame)
        )
    }];
    let err = document_errors(&doc, options.use_err, &mut notes);
    let err_label = document_err_source(&doc.source, err.is_some());
    let excluded = document_exclusions(&doc, options.use_dq, &mut notes);
    let flux = measured_flux(&doc.flux, &mut notes);
    let mut m = measure_line_with_errors(
        &doc.axis.values,
        &flux,
        err,
        excluded.as_deref(),
        (request.z0, request.z1),
        request.continuum,
        request.model,
    )
    .map_err(anyhow::Error::msg)?;
    m.axis_unit = doc.axis.unit.clone();
    m.flux_unit = format!("{} x {}", doc.flux_unit.as_deref().unwrap_or(NATIVE_FLUX_UNIT), doc.axis.unit);
    notes.append(&mut m.notes);
    m.notes = notes;
    let rest = match (request.rest_um.or(doc.axis.rest_wavelength_um), frame) {
        (Some(rest), WavelengthFrame::Air) => Some(air_rest_um(rest, &mut m.notes)),
        (rest, _) => rest,
    };
    add_velocity(&mut m, &doc.axis, rest, request);
    Ok((m, SpectrumSource::File { hdu: document_hdu(&doc.source) }, err_label))
}

#[cfg(test)]
pub(crate) fn measure_line_json(path: &str, source: &SpectrumSource, request: &LineRequest) -> anyhow::Result<serde_json::Value> {
    measure_line_json_with(path, source, request, &MeasureOptions::default())
}

pub(crate) fn measure_line_json_with(
    path: &str,
    source: &SpectrumSource,
    request: &LineRequest,
    options: &MeasureOptions,
) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let (mut m, measured, err_label) = match source {
        SpectrumSource::File { hdu } => measure_file_line(path, *hdu, request, options)?,
        SpectrumSource::Pixel { .. } | SpectrumSource::Region { .. } => {
            let (m, err_label) = measure_cube_line(path, source, request, options)?;
            (m, source.clone(), err_label)
        }
    };
    m.elapsed_ms = t0.elapsed().as_millis() as u64;
    let mut value = serde_json::to_value(&m)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(RES_SOURCE.to_string(), serde_json::to_value(&measured)?);
        object.insert(KEY_ERR_SOURCE.to_string(), serde_json::Value::String(err_label));
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
    use_err: Option<bool>,
    use_dq: Option<bool>,
    wavelength_frame: Option<String>,
) -> Result<serde_json::Value, String> {
    if path.trim().is_empty() {
        return Err("path must name a cube or spectrum file, got an empty string".to_string());
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
    let wavelength_frame = wavelength_frame.as_deref().map(parse_wavelength_frame).transpose()?;
    let options = MeasureOptions { use_err: use_err.unwrap_or(true), use_dq: use_dq.unwrap_or(true), wavelength_frame };
    let request = LineRequest { z0, z1, continuum, rest_um, convention, model, velocity_shift_kms };
    blocking_cmd!(measure_line_json_with(&path, &source, &request, &options))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::line_measure::golden::{assert_matches_golden, fixtures_dir, without_elapsed};
    use crate::cmd::spectrum1d::test_fixtures::{iraf_linear_cards, write_wcs1d, write_x1d_columns};
    use crate::core::analysis::line_measure::golden::{golden_diffs, read_golden, unweighted_diffs};
    use crate::core::cube::lazy::{region_pixels, test_support::*};
    use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
    use crate::core::cube::companions::open_cube_set;
    use crate::core::cube::moments::channel_widths;

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
    fn a_barycentric_cube_measured_without_a_shift_is_reported_in_its_own_frame_and_not_as_topocentric() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "bary.fits", 0.0);
        let value = measure_line_json(&path, &centre_pixel(), &request(LineModel::None)).unwrap();
        let notes: Vec<&str> = value["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
        let velocity_note = notes.iter().find(|n| n.starts_with("line velocity")).expect("velocity note");
        assert!(!velocity_note.contains("topocentric"), "{}", velocity_note);
        assert!(velocity_note.contains("axis frame BARYCENT"), "{}", velocity_note);
        assert_eq!(value["velocity"]["axis_frame"], "BARYCENT");
        assert_eq!(number(&value["velocity"], "shift_applied_kms"), 0.0);

        let undeclared = dir.path().join("undeclared.fits");
        write_line_cube_with_cards(&undeclared, &[("SPECSYS", "")]);
        let value = measure_line_json(undeclared.to_str().unwrap(), &centre_pixel(), &request(LineModel::None)).unwrap();
        assert!(value["velocity"]["axis_frame"].is_null(), "{}", value["velocity"]);
        let notes: Vec<&str> = value["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
        let velocity_note = notes.iter().find(|n| n.starts_with("line velocity")).expect("velocity note");
        assert!(!velocity_note.contains("topocentric"), "{}", velocity_note);
        assert!(velocity_note.contains("no SPECSYS"), "{}", velocity_note);
    }

    #[test]
    fn a_region_source_labels_its_flux_as_a_sum_over_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sb.fits");
        write_line_cube_with_cards(&path, &[("BUNIT", "'MJy/sr'"), ("PIXAR_SR", "3.0461742E-12")]);
        let path = path.to_str().unwrap();
        let disk = RegionShape::Circle { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r: LINE_DISK_RADIUS };
        let region =
            measure_line_json(path, &SpectrumSource::Region { shape: disk, background: None }, &request(LineModel::None))
                .unwrap();
        assert_eq!(region["flux_unit"], "MJy/sr x pix x um");
        let pixel = measure_line_json(path, &centre_pixel(), &request(LineModel::None)).unwrap();
        assert_eq!(pixel["flux_unit"], "MJy/sr x um");
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

    fn cube_golden_cases(dir: &tempfile::TempDir) -> Vec<(&'static str, serde_json::Value)> {
        let clean = write(dir, "golden_line.fits", 0.0);
        let noisy = write(dir, "golden_noisy.fits", 0.05);
        let air = dir.path().join("golden_air.fits");
        write_line_cube_with_cards(&air, &[("CTYPE3", "'AWAV'")]);
        let mut shifted = request(LineModel::Gaussian);
        shifted.velocity_shift_kms = Some(-7.25);
        shifted.convention = VelocityConvention::Relativistic;
        let disk = RegionShape::Circle { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r: LINE_DISK_RADIUS };
        let sky = RegionShape::Annulus {
            x: LINE_DISK_CENTRE,
            y: LINE_DISK_CENTRE,
            r_inner: LINE_DISK_RADIUS + 2.0,
            r_outer: LINE_DISK_RADIUS + 5.0,
        };
        let region = SpectrumSource::Region { shape: disk.clone(), background: None };
        let region_with_sky = SpectrumSource::Region { shape: disk, background: Some(sky) };
        let run = |path: &str, source: &SpectrumSource, req: &LineRequest| {
            let mut value = without_elapsed(measure_line_json(path, source, req).unwrap());
            assert_eq!(value[KEY_ERR_SOURCE], ERR_SOURCE_NONE, "{}", value);
            value.as_object_mut().unwrap().remove(KEY_ERR_SOURCE);
            value
        };
        vec![
            ("cube_centre_pixel_gaussian", run(&clean, &centre_pixel(), &request(LineModel::Gaussian))),
            ("cube_noisy_pixel_relativistic_shift", run(&noisy, &centre_pixel(), &shifted)),
            ("cube_outer_pixel_noisy", run(&noisy, &SpectrumSource::Pixel { x: 2, y: 2 }, &request(LineModel::None))),
            ("cube_region_disk", run(&clean, &region, &request(LineModel::None))),
            ("cube_region_disk_with_sky_noisy", run(&noisy, &region_with_sky, &request(LineModel::Gaussian))),
            ("cube_air_axis", run(air.to_str().unwrap(), &centre_pixel(), &request(LineModel::None))),
        ]
    }

    #[test]
    fn cube_line_measurements_are_bit_identical_to_the_goldens_recorded_before_the_error_aware_entry_point() {
        let dir = tempfile::tempdir().unwrap();
        let cases = cube_golden_cases(&dir);
        assert_eq!(cases.len(), 6);
        for (name, value) in cases {
            assert_matches_golden(&fixtures_dir().join(format!("{name}.json")), &value, 0);
        }
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

    const DOCUMENT_KEYS: [&str; 5] = ["axis_unit", "flux_unit", "notes", RES_SOURCE, KEY_ERR_SOURCE];
    const DOCUMENT_ERR: f64 = 0.05;
    const DO_NOT_USE: i32 = 1;
    const JUMP_DET: i32 = 4;
    const H_ALPHA_VACUUM_UM: f64 = 0.656461;
    const H_ALPHA_START_ANGSTROM: f64 = 6540.0;
    const H_ALPHA_STEP_ANGSTROM: f64 = 0.5;
    const H_ALPHA_SIGMA_ANGSTROM: f64 = 1.5;
    const H_ALPHA_CHANNELS: usize = 100;
    const H_ALPHA_LINE: (usize, usize) = (30, 61);
    const H_ALPHA_WINDOWS: ContinuumWindows = ((0, 20), (70, 99));
    const H_ALPHA_BUNIT: &str = "erg/cm2/s/A";
    const SPEED_OF_LIGHT_KMS: f64 = 299_792.458;
    const VELOCITY_TOLERANCE_KMS: f64 = 0.5;

    fn file_source() -> SpectrumSource {
        SpectrumSource::File { hdu: None }
    }

    fn notes_of(value: &serde_json::Value) -> Vec<String> {
        value["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect()
    }

    fn centre_arrays(noise: f32) -> (Vec<f64>, Vec<f32>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("arrays.fits");
        write_line_cube(&path, noise);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let centre = LINE_DISK_CENTRE as usize;
        (cube.spectral_axis().unwrap().values, cube.extract_spectrum_at(centre, centre).unwrap())
    }

    fn write_document(dir: &tempfile::TempDir, name: &str, axis: &[f64], flux: &[f32], err: &[f64], dq: &[i32]) -> String {
        let widened: Vec<f64> = flux.iter().map(|&v| f64::from(v)).collect();
        write_x1d_columns(dir.path(), name, axis, &widened, err, dq).to_str().unwrap().to_string()
    }

    fn without_document_keys(mut value: serde_json::Value) -> serde_json::Value {
        if let Some(object) = value.as_object_mut() {
            for key in DOCUMENT_KEYS {
                object.remove(key);
            }
        }
        value
    }

    #[tokio::test]
    async fn the_command_measures_an_x1d_file_source_with_its_flux_error() {
        let dir = tempfile::tempdir().unwrap();
        let (axis, flux) = centre_arrays(0.05);
        let n = axis.len();
        let path = write_document(&dir, "doc_x1d.fits", &axis, &flux, &vec![DOCUMENT_ERR; n], &vec![0; n]);
        let value = measure_spectral_line_cmd(
            path,
            file_source(),
            LINE_RANGE.0,
            LINE_RANGE.1,
            Some(WINDOWS),
            None,
            None,
            Some("gaussian".to_string()),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(value[RES_SOURCE]["kind"], "file");
        assert_eq!(value[RES_SOURCE]["hdu"], 1);
        assert_eq!(value["err_used"], true);
        assert_eq!(value[KEY_ERR_SOURCE], "FLUX_ERROR column (HDU 1)");
        assert!(number(&value["flux_err_terms"], "pixel") > 0.0, "{}", value["flux_err_terms"]);
        assert_eq!(value["axis_unit"], "um");
        assert_eq!(value["flux_unit"], "Jy x um");
        let flux_err = number(&value, "flux_err");
        assert!((number(&value, "flux") - EXPECTED_FLUX).abs() < 3.0 * flux_err, "flux {} err {}", value["flux"], flux_err);
        assert_eq!(value["fit"]["converged"], true, "{}", value["fit"]);
        assert!(notes_of(&value).iter().any(|n| n.contains("EXTRACT1D table in HDU 1: vacuum wavelengths")), "{:?}", notes_of(&value));
        let refused = measure_spectral_line_cmd(
            "doc.fits".to_string(),
            file_source(),
            LINE_RANGE.0,
            LINE_RANGE.1,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("sideways".to_string()),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("unknown wavelength frame"), "{}", refused);
    }

    #[test]
    fn a_file_source_with_use_err_false_reproduces_the_goldens_recorded_before_the_error_aware_entry_point() {
        let dir = tempfile::tempdir().unwrap();
        let (axis, clean) = centre_arrays(0.0);
        let (_, noisy) = centre_arrays(0.05);
        let mut blanked = noisy.clone();
        blanked[WINDOWS.1 .0..=WINDOWS.1 .1].fill(f32::NAN);
        let n = axis.len();
        let unweighted = MeasureOptions { use_err: false, ..MeasureOptions::default() };
        let cases = [
            ("gaussian_line_noiseless", &clean, LineModel::Gaussian, 0u64),
            ("gaussian_line_noisy", &noisy, LineModel::Gaussian, 0),
            ("constant_continuum_blanked_window", &blanked, LineModel::None, 6),
        ];
        let mut failures = Vec::new();
        for (name, flux, model, nonfinite) in cases {
            let path = write_document(&dir, &format!("{name}_x1d.fits"), &axis, flux, &vec![DOCUMENT_ERR; n], &vec![0; n]);
            let value = measure_line_json_with(&path, &file_source(), &request(model), &unweighted).unwrap();
            assert_eq!(value[RES_SOURCE]["kind"], "file");
            assert_eq!(value[KEY_ERR_SOURCE], ERR_SOURCE_NONE);
            assert!(notes_of(&value).iter().any(|n| n.contains("use_err = false")), "{:?}", notes_of(&value));
            let actual = without_document_keys(without_elapsed(value));
            let golden = without_document_keys(read_golden(&fixtures_dir().join(format!("{name}.json"))));
            let mut diffs = golden_diffs(&golden, &actual);
            diffs.extend(unweighted_diffs(&actual, nonfinite));
            if !diffs.is_empty() {
                failures.push(format!("{}:\n  {}", name, diffs.join("\n  ")));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn dq_flagged_zero_error_and_nan_rows_of_a_file_are_excluded_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let (axis, mut flux) = centre_arrays(0.05);
        let n = axis.len();
        let mut err = vec![DOCUMENT_ERR; n];
        let mut dq = vec![0; n];
        for z in [10, 12, 14] {
            dq[z] = DO_NOT_USE;
        }
        dq[16] = JUMP_DET;
        for z in [26, 28] {
            err[z] = 0.0;
        }
        flux[30] = f32::NAN;
        let path = write_document(&dir, "flagged_x1d.fits", &axis, &flux, &err, &dq);
        let source = SpectrumSource::File { hdu: Some(1) };
        let value = measure_line_json(&path, &source, &request(LineModel::None)).unwrap();
        assert_eq!(value["err_used"], true);
        assert_eq!(value["n_dq_excluded"], 3);
        assert_eq!(value["n_bad_err"], 2);
        assert_eq!(value["n_nonfinite"], 1);
        assert_eq!(value["n_channels"], 25 - 6);
        assert_eq!(value[RES_SOURCE], serde_json::json!({ "kind": "file", "hdu": 1 }));

        let without_dq = MeasureOptions { use_dq: false, ..MeasureOptions::default() };
        let value = measure_line_json_with(&path, &source, &request(LineModel::None), &without_dq).unwrap();
        assert_eq!(value["n_dq_excluded"], 0);
        assert_eq!(value["n_channels"], 25 - 3);
        assert!(notes_of(&value).iter().any(|n| n.contains("use_dq = false")), "{:?}", notes_of(&value));
    }

    fn h_alpha_air_document(dir: &tempfile::TempDir) -> String {
        let centre = vacuum_to_air_morton_um(H_ALPHA_VACUUM_UM) * 1e4;
        let flux: Vec<f32> = (0..H_ALPHA_CHANNELS)
            .map(|i| {
                let d = (H_ALPHA_START_ANGSTROM + H_ALPHA_STEP_ANGSTROM * i as f64 - centre) / H_ALPHA_SIGMA_ANGSTROM;
                (1.0 + (-0.5 * d * d).exp()) as f32
            })
            .collect();
        let cards = iraf_linear_cards(H_ALPHA_START_ANGSTROM, H_ALPHA_STEP_ANGSTROM, H_ALPHA_BUNIT);
        write_wcs1d(dir.path(), "h_alpha_air.fits", &cards, &flux).to_str().unwrap().to_string()
    }

    fn h_alpha_request() -> LineRequest {
        LineRequest {
            z0: H_ALPHA_LINE.0,
            z1: H_ALPHA_LINE.1,
            continuum: Some(H_ALPHA_WINDOWS),
            rest_um: Some(H_ALPHA_VACUUM_UM),
            convention: VelocityConvention::Optical,
            model: LineModel::None,
            velocity_shift_kms: None,
        }
    }

    #[test]
    fn an_air_document_converts_the_vacuum_rest_wavelength_with_morton_before_velocities() {
        let dir = tempfile::tempdir().unwrap();
        let path = h_alpha_air_document(&dir);
        let air_rest = vacuum_to_air_morton_um(H_ALPHA_VACUUM_UM);
        let value = measure_line_json(&path, &file_source(), &h_alpha_request()).unwrap();
        assert_eq!(value[RES_SOURCE], serde_json::json!({ "kind": "file", "hdu": 0 }));
        assert_eq!(value["err_used"], false);
        assert_eq!(value["flux_unit"], format!("{} x um", H_ALPHA_BUNIT));
        assert!((number(&value, "centroid") - air_rest).abs() < 1e-7, "centroid {}", value["centroid"]);
        let velocity = &value["velocity"];
        assert!(number(velocity, "centroid_kms").abs() < VELOCITY_TOLERANCE_KMS, "{}", velocity);
        assert_eq!(number(velocity, "rest_um"), air_rest);
        let notes = notes_of(&value);
        assert!(notes.iter().any(|n| n.contains("1D array in HDU 0: air wavelengths")), "{:?}", notes);
        assert!(notes.iter().any(|n| n.contains("Morton (2000")), "{:?}", notes);

        let as_vacuum = MeasureOptions { wavelength_frame: Some(WavelengthFrame::Vacuum), ..MeasureOptions::default() };
        let value = measure_line_json_with(&path, &file_source(), &h_alpha_request(), &as_vacuum).unwrap();
        let shifted = number(&value["velocity"], "centroid_kms");
        let expected = SPEED_OF_LIGHT_KMS * (air_rest - H_ALPHA_VACUUM_UM) / H_ALPHA_VACUUM_UM;
        assert!(expected < -80.0, "expected {}", expected);
        assert!((shifted - expected).abs() < VELOCITY_TOLERANCE_KMS, "shifted {} expected {}", shifted, expected);
        assert_eq!(number(&value["velocity"], "rest_um"), H_ALPHA_VACUUM_UM);
        let notes = notes_of(&value);
        assert!(notes.iter().any(|n| n.contains("vacuum wavelengths set by the request")), "{:?}", notes);
        assert!(notes.iter().all(|n| !n.contains("Morton (2000")), "{:?}", notes);
    }

    #[test]
    fn a_wavelength_frame_override_is_refused_on_cubes_and_on_axes_that_are_not_wavelengths() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "line.fits", 0.0);
        let options = MeasureOptions { wavelength_frame: Some(WavelengthFrame::Air), ..MeasureOptions::default() };
        let err = measure_line_json_with(&path, &centre_pixel(), &request(LineModel::None), &options).unwrap_err().to_string();
        assert!(err.contains("spectrum files only"), "{}", err);
        let err = measure_line_json(&path, &file_source(), &request(LineModel::None)).unwrap_err().to_string();
        assert!(err.contains("holds no 1D spectrum"), "{}", err);
        let err = document_frame(WavelengthFrame::NotWavelength, Some(WavelengthFrame::Air)).unwrap_err();
        assert!(err.contains("not a wavelength axis"), "{}", err);
        assert!(document_frame(WavelengthFrame::Air, Some(WavelengthFrame::NotWavelength)).is_err());
        assert_eq!(document_frame(WavelengthFrame::Air, None).unwrap(), WavelengthFrame::Air);
        assert_eq!(document_frame(WavelengthFrame::Air, Some(WavelengthFrame::Vacuum)).unwrap(), WavelengthFrame::Vacuum);
        assert_eq!(parse_wavelength_frame("Air").unwrap(), WavelengthFrame::Air);
        assert_eq!(parse_wavelength_frame("vacuum").unwrap(), WavelengthFrame::Vacuum);
        assert!(parse_wavelength_frame("not_wavelength").unwrap_err().contains("unknown wavelength frame"));
    }

    fn write_csv_document(dir: &tempfile::TempDir, axis: &[f64], flux: &[f32], err: &[f64]) -> String {
        let mut text = String::from("# flux_unit: Jy\nchannel,wavelength_vacuum_um,flux,flux_unit,err,dq\n");
        for (i, ((w, f), e)) in axis.iter().zip(flux).zip(err).enumerate() {
            text.push_str(&format!("{},{},{},Jy,{},0\n", i, w, f64::from(*f), e));
        }
        let path = dir.path().join("exported_spectrum.csv");
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn a_csv_spectrum_is_measured_with_its_err_column_and_refuses_an_hdu() {
        let dir = tempfile::tempdir().unwrap();
        let (axis, flux) = centre_arrays(0.05);
        let n = axis.len();
        let path = write_csv_document(&dir, &axis, &flux, &vec![DOCUMENT_ERR; n]);
        let value = measure_line_json(&path, &file_source(), &request(LineModel::Gaussian)).unwrap();
        assert_eq!(value[RES_SOURCE], serde_json::json!({ "kind": "file", "hdu": null }));
        assert_eq!(value["err_used"], true);
        assert_eq!(value[KEY_ERR_SOURCE], "err column (CSV)");
        assert_eq!(value["flux_unit"], "Jy x um");
        let flux_err = number(&value, "flux_err");
        assert!((number(&value, "flux") - EXPECTED_FLUX).abs() < 3.0 * flux_err, "flux {} err {}", value["flux"], flux_err);
        assert!(notes_of(&value).iter().any(|n| n.contains("CSV file: vacuum wavelengths")), "{:?}", notes_of(&value));
        let err = measure_line_json(&path, &SpectrumSource::File { hdu: Some(0) }, &request(LineModel::None)).unwrap_err().to_string();
        assert!(err.contains("HDU 0") && err.contains("CSV"), "{}", err);
    }

    #[test]
    fn a_file_source_deserializes_from_the_frontend_shape_and_echoes_the_file_kind() {
        let explicit: SpectrumSource = serde_json::from_str(r#"{"kind":"file","hdu":2}"#).unwrap();
        assert_eq!(explicit, SpectrumSource::File { hdu: Some(2) });
        let null: SpectrumSource = serde_json::from_str(r#"{"kind":"file","hdu":null}"#).unwrap();
        assert_eq!(null, file_source());
        let bare: SpectrumSource = serde_json::from_str(r#"{"kind":"file"}"#).unwrap();
        assert_eq!(bare, file_source());
        assert_eq!(serde_json::to_value(SpectrumSource::File { hdu: Some(1) }).unwrap(), serde_json::json!({ "kind": "file", "hdu": 1 }));
    }

    const MEF_NOISE: f32 = 0.01;
    const MEF_ERR_SOURCE: &str = "ERR HDU 2";
    const PIXEL_FLAGGED_CHANNELS: [usize; 3] = [2, 12, 36];
    const PIXEL_JUMP_CHANNEL: usize = 20;
    const PERSISTENT_SPAXEL: (usize, usize) = (16, 16);
    const TRANSIENT_SPAXEL: (usize, usize) = (15, 16);
    const TRANSIENT_CHANNELS: [usize; 3] = [3, 14, 15];
    const TRANSIENT_LINE_CHANNELS: usize = 2;
    const TERM_TOLERANCE: f64 = 1e-6;

    fn mef_err(z: usize) -> f32 {
        0.005 * (1.0 + 0.02 * z as f32)
    }

    fn write_mef(dir: &tempfile::TempDir, name: &str, dq: impl Fn(usize, usize, usize) -> i32) -> String {
        let path = dir.path().join(name);
        write_line_cube_mef(&path, MEF_NOISE, |z, _, _| mef_err(z), dq);
        path.to_str().unwrap().to_string()
    }

    fn disk_source() -> SpectrumSource {
        SpectrumSource::Region {
            shape: RegionShape::Circle { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r: LINE_DISK_RADIUS },
            background: None,
        }
    }

    fn pixel_term(err: &[f64], axis: &[f64], channels: impl Iterator<Item = usize>) -> f64 {
        let widths = channel_widths(axis);
        channels.map(|z| (err[z] * widths[z]).powi(2)).sum::<f64>().sqrt()
    }

    fn assert_relative(what: &str, found: f64, expected: f64, tolerance: f64) {
        assert!((found - expected).abs() <= tolerance * expected.abs(), "{}: {} vs {}", what, found, expected);
    }

    fn assert_total_is_the_quadrature_of_the_terms(value: &serde_json::Value) {
        let terms = &value["flux_err_terms"];
        let total = number(terms, "pixel").hypot(number(terms, "level")).hypot(number(terms, "slope"));
        assert_relative("flux_err against its three terms", number(value, "flux_err"), total, 1e-12);
    }

    fn invalidate_mef(key: &str) {
        for hdu in ["", "#hdu=1", "#hdu=2", "#hdu=3"] {
            GLOBAL_CUBE_CACHE.invalidate(&format!("{}{}", key, hdu));
        }
    }

    #[test]
    fn a_region_line_measurement_on_a_cube_with_err_takes_its_pixel_term_from_the_region_error_column() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_mef(&dir, "region_s3d.fits", |_, _, _| 0);
        let set = open_cube_set(&key).unwrap();
        let axis = set.sci.spectral_axis().unwrap().values;
        let weight_squares: f64 = region_pixels(&disk_shape(), LINE_CUBE_SIZE, LINE_CUBE_SIZE, APERTURE_SUBSAMPLES)
            .unwrap()
            .iter()
            .map(|p| f64::from(p.weight).powi(2))
            .sum();
        let err: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| f64::from(mef_err(z)) * weight_squares.sqrt()).collect();
        let expected_pixel = pixel_term(&err, &axis, LINE_RANGE.0..=LINE_RANGE.1);
        for continuum in [None, Some(WINDOWS)] {
            let mut req = request(LineModel::None);
            req.continuum = continuum;
            let value = measure_line_json(&key, &disk_source(), &req).unwrap();
            assert_eq!(value["err_used"], true, "{}", value);
            assert_eq!(value[KEY_ERR_SOURCE], MEF_ERR_SOURCE);
            assert_eq!(value["n_dq_excluded"], 0);
            assert_eq!(value["n_channels"], 25);
            assert_relative("pixel term", number(&value["flux_err_terms"], "pixel"), expected_pixel, TERM_TOLERANCE);
            assert_total_is_the_quadrature_of_the_terms(&value);
            assert!(number(&value, "flux_err") >= expected_pixel * (1.0 - TERM_TOLERANCE));
            let notes = notes_of(&value);
            assert!(notes.iter().any(|n| n.contains("flux summed over")), "{:?}", notes);
            assert!(notes.iter().any(|n| n.contains("ERR cube (HDU 2)")), "{:?}", notes);
        }
        invalidate_mef(&key);
    }

    fn disk_shape() -> RegionShape {
        RegionShape::Circle { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r: LINE_DISK_RADIUS }
    }

    fn centre_flags(z: usize, y: usize, x: usize) -> i32 {
        if (y, x) != (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize) {
            0
        } else if PIXEL_FLAGGED_CHANNELS.contains(&z) {
            DO_NOT_USE
        } else if z == PIXEL_JUMP_CHANNEL {
            JUMP_DET
        } else {
            0
        }
    }

    #[test]
    fn a_pixel_line_measurement_on_a_cube_with_err_and_dq_excludes_the_flagged_channels_of_that_spaxel() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_mef(&dir, "pixel_s3d.fits", centre_flags);
        let value = measure_line_json(&key, &centre_pixel(), &request(LineModel::None)).unwrap();
        assert_eq!(value["err_used"], true, "{}", value);
        assert_eq!(value[KEY_ERR_SOURCE], MEF_ERR_SOURCE);
        assert_eq!(value["n_dq_excluded"], PIXEL_FLAGGED_CHANNELS.len());
        assert_eq!(value["n_channels"], 25 - 1);
        let set = open_cube_set(&key).unwrap();
        let axis = set.sci.spectral_axis().unwrap().values;
        let err: Vec<f64> = (0..LINE_CUBE_DEPTH).map(|z| f64::from(mef_err(z))).collect();
        let usable = (LINE_RANGE.0..=LINE_RANGE.1).filter(|z| !PIXEL_FLAGGED_CHANNELS.contains(z));
        assert_relative("pixel term", number(&value["flux_err_terms"], "pixel"), pixel_term(&err, &axis, usable), TERM_TOLERANCE);
        assert_total_is_the_quadrature_of_the_terms(&value);

        let unflagged = MeasureOptions { use_dq: false, ..MeasureOptions::default() };
        let value = measure_line_json_with(&key, &centre_pixel(), &request(LineModel::None), &unflagged).unwrap();
        assert_eq!(value["n_dq_excluded"], 0);
        assert_eq!(value["n_channels"], 25);
        assert!(notes_of(&value).iter().any(|n| n.contains("use_dq = false")), "{:?}", notes_of(&value));

        let unweighted = MeasureOptions { use_err: false, ..MeasureOptions::default() };
        let value = measure_line_json_with(&key, &centre_pixel(), &request(LineModel::None), &unweighted).unwrap();
        assert_eq!(value["err_used"], false);
        assert_eq!(value[KEY_ERR_SOURCE], ERR_SOURCE_NONE);
        assert!(value["flux_err_terms"].is_null());
        assert_eq!(value["n_dq_excluded"], PIXEL_FLAGGED_CHANNELS.len());
        assert!(notes_of(&value).iter().any(|n| n.contains("use_err = false")), "{:?}", notes_of(&value));

        let refused = measure_line_json(&format!("{}#hdu=2", key), &centre_pixel(), &request(LineModel::None))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("cube products need the SCI cube"), "{}", refused);
        invalidate_mef(&key);
    }

    fn persistent_and_transient_flags(z: usize, y: usize, x: usize) -> i32 {
        let flagged = (y, x) == PERSISTENT_SPAXEL || ((y, x) == TRANSIENT_SPAXEL && TRANSIENT_CHANNELS.contains(&z));
        if flagged {
            DO_NOT_USE
        } else {
            0
        }
    }

    fn aperture_weights() -> Vec<(usize, usize, f64)> {
        region_pixels(&disk_shape(), LINE_CUBE_SIZE, LINE_CUBE_SIZE, APERTURE_SUBSAMPLES)
            .unwrap()
            .iter()
            .map(|p| (p.y, p.x, f64::from(p.weight)))
            .collect()
    }

    fn summed_area_note(dropped: &[(usize, usize)]) -> String {
        let weights = aperture_weights();
        let npix: f64 = weights.iter().map(|w| w.2).sum();
        let kept: f64 = weights.iter().filter(|w| !dropped.contains(&(w.0, w.1))).map(|w| w.2).sum();
        format!(
            "circle region: flux summed over {:.1} of {:.1} pixels (spaxels dropped in every measured channel by DQ bit 0 or non-finite flux: {})",
            kept,
            npix,
            dropped.len()
        )
    }

    fn excluded_note(excluded: usize, measured: usize) -> String {
        format!(
            "{} of the {} line and continuum channels lost aperture spaxels (DQ bit 0 or non-finite flux) that other measured channels kept: excluded",
            excluded, measured
        )
    }

    const MEASURED_CHANNELS: usize = 6 + 25 + 6;

    #[test]
    fn a_region_line_measurement_excludes_only_the_channels_that_lost_spaxels_other_measured_channels_kept() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_mef(&dir, "flags_s3d.fits", persistent_and_transient_flags);
        let value = measure_line_json(&key, &disk_source(), &request(LineModel::None)).unwrap();
        assert_eq!(value["err_used"], true, "{}", value);
        assert_eq!(value["n_dq_excluded"], TRANSIENT_CHANNELS.len());
        assert_eq!(value["n_channels"], 25 - TRANSIENT_LINE_CHANNELS);
        let notes = notes_of(&value);
        assert!(notes.contains(&excluded_note(TRANSIENT_CHANNELS.len(), MEASURED_CHANNELS)), "{:?}", notes);
        assert!(notes.contains(&summed_area_note(&[PERSISTENT_SPAXEL])), "{:?}", notes);

        let unflagged = MeasureOptions { use_dq: false, ..MeasureOptions::default() };
        let all = measure_line_json_with(&key, &disk_source(), &request(LineModel::None), &unflagged).unwrap();
        assert_eq!(all["n_dq_excluded"], 0);
        assert_eq!(all["n_channels"], 25);
        assert!(number(&all, "flux") > number(&value, "flux"), "{} vs {}", all["flux"], value["flux"]);
        invalidate_mef(&key);
    }

    const TRUNCATED_LINE: (usize, usize) = (8, 20);
    const NEAR_WINDOWS: ContinuumWindows = ((0, 5), (22, 27));
    const TRUNCATED_MEASURED_CHANNELS: usize = 6 + 13 + 6;
    const FOOTPRINT_EDGE_SPAXEL: (usize, usize) = (16, 20);
    const FOOTPRINT_EDGE_LAST_DROPPED: usize = 34;
    const SWAPPED_SPAXELS: [(usize, usize); 2] = [(16, 14), (16, 18)];
    const SWAP_CHANNEL: usize = 20;
    const NAN_SPAXEL: (usize, usize) = (17, 16);
    const NAN_LINE_CHANNELS: [usize; 2] = [14, 15];

    fn truncated_request() -> LineRequest {
        LineRequest { z0: TRUNCATED_LINE.0, z1: TRUNCATED_LINE.1, continuum: Some(NEAR_WINDOWS), ..request(LineModel::None) }
    }

    fn write_mef_from(
        dir: &tempfile::TempDir,
        name: &str,
        sci: impl Fn(usize, usize, usize) -> f32,
        dq: impl Fn(usize, usize, usize) -> i32,
    ) -> String {
        let path = dir.path().join(name);
        write_line_cube_mef_from(&path, &[], sci, |z, _, _| mef_err(z), dq);
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn a_footprint_edge_spaxel_dropped_in_every_measured_channel_keeps_the_measurement_and_shrinks_the_summed_area() {
        let dir = tempfile::tempdir().unwrap();
        let edge = |z: usize, y: usize, x: usize| (y, x) == FOOTPRINT_EDGE_SPAXEL && z <= FOOTPRINT_EDGE_LAST_DROPPED;
        let key = write_mef_from(
            &dir,
            "edge_s3d.fits",
            |z, y, x| if edge(z, y, x) { f32::NAN } else { line_cube_value(z, y, x, MEF_NOISE) },
            |z, y, x| if edge(z, y, x) { DO_NOT_USE } else { 0 },
        );
        let value = measure_line_json(&key, &disk_source(), &truncated_request()).unwrap();
        assert_eq!(value["err_used"], true, "{}", value);
        assert_eq!(value["n_dq_excluded"], 0);
        assert_eq!(value["n_channels"], TRUNCATED_LINE.1 - TRUNCATED_LINE.0 + 1);
        let notes = notes_of(&value);
        assert!(notes.contains(&excluded_note(0, TRUNCATED_MEASURED_CHANNELS)), "{:?}", notes);
        assert!(notes.contains(&summed_area_note(&[FOOTPRINT_EDGE_SPAXEL])), "{:?}", notes);
        invalidate_mef(&key);
    }

    #[test]
    fn a_region_whose_flagged_spaxel_moves_inside_the_measured_channels_is_refused_instead_of_switching_spaxel_sets() {
        let dir = tempfile::tempdir().unwrap();
        let swapped = |z: usize, y: usize, x: usize| {
            let moved = SWAPPED_SPAXELS[usize::from(z >= SWAP_CHANNEL)];
            if (y, x) == moved {
                DO_NOT_USE
            } else {
                0
            }
        };
        let key = write_mef(&dir, "swap_s3d.fits", swapped);
        let refused = measure_line_json(&key, &disk_source(), &truncated_request()).unwrap_err().to_string();
        assert!(
            refused.contains("every line channel 8..=20 lost aperture spaxels (DQ bit 0 or non-finite flux) that other measured channels kept"),
            "{}",
            refused
        );
        let unflagged = MeasureOptions { use_dq: false, ..MeasureOptions::default() };
        let all = measure_line_json_with(&key, &disk_source(), &truncated_request(), &unflagged).unwrap();
        assert_eq!(all["n_dq_excluded"], 0);
        assert_eq!(all["n_channels"], TRUNCATED_LINE.1 - TRUNCATED_LINE.0 + 1);
        invalidate_mef(&key);
    }

    #[test]
    fn channels_where_a_non_finite_spaxel_shrinks_the_aperture_are_excluded_with_or_without_dq() {
        let dir = tempfile::tempdir().unwrap();
        let dropout = |z: usize, y: usize, x: usize| (y, x) == NAN_SPAXEL && NAN_LINE_CHANNELS.contains(&z);
        let key = write_mef_from(
            &dir,
            "nan_s3d.fits",
            |z, y, x| if dropout(z, y, x) { f32::NAN } else { line_cube_value(z, y, x, MEF_NOISE) },
            |_, _, _| 0,
        );
        let unflagged = MeasureOptions { use_dq: false, ..MeasureOptions::default() };
        for options in [MeasureOptions::default(), unflagged] {
            let value = measure_line_json_with(&key, &disk_source(), &request(LineModel::None), &options).unwrap();
            assert_eq!(value["n_dq_excluded"], NAN_LINE_CHANNELS.len(), "{:?}", options);
            assert_eq!(value["n_channels"], 25 - NAN_LINE_CHANNELS.len(), "{:?}", options);
            let notes = notes_of(&value);
            assert!(notes.contains(&excluded_note(NAN_LINE_CHANNELS.len(), MEASURED_CHANNELS)), "{:?}", notes);
        }
        invalidate_mef(&key);
    }

    #[test]
    fn line_measurements_on_a_cube_without_err_keep_the_continuum_scatter_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "plain.fits", 0.05);
        for source in [centre_pixel(), disk_source()] {
            let value = measure_line_json(&path, &source, &request(LineModel::None)).unwrap();
            assert_eq!(value["err_used"], false, "{}", value);
            assert_eq!(value[KEY_ERR_SOURCE], ERR_SOURCE_NONE);
            assert!(value["flux_err_terms"].is_null());
            assert_eq!(value["n_dq_excluded"], 0);
            let continuum_sigma = number(&value, "continuum_sigma");
            let widths = channel_widths(&LazyCube::open(&path).unwrap().spectral_axis().unwrap().values);
            let mean_dx = widths[LINE_RANGE.0..=LINE_RANGE.1].iter().sum::<f64>() / 25.0;
            assert_relative("scatter error", number(&value, "flux_err"), continuum_sigma * 5.0 * mean_dx, 1e-12);
        }
        GLOBAL_CUBE_CACHE.invalidate(&path);
    }

    const G395H_X1D: &str = "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";
    const G395H_BR_ALPHA_GOLDEN: &str = "line_measure_goldens/jw01266_g395h_br_alpha.json";
    const BR_ALPHA_LINE: (usize, usize) = (1769, 1786);
    const BR_ALPHA_CONTINUUM: ContinuumWindows = ((1750, 1762), (1789, 1800));
    const G395H_DETECTOR_GAP: (usize, usize) = (1814, 1871);
    const G395H_GAP_CHANNELS: usize = 58;
    const ACROSS_GAP_LINE: (usize, usize) = (1801, 1900);
    const ACROSS_GAP_CONTINUUM: ContinuumWindows = ((1750, 1762), (1905, 1920));

    struct X1dArrays {
        axis: Vec<f64>,
        flux: Vec<f32>,
        err: Vec<f64>,
        excluded: Vec<bool>,
    }

    fn g395h_path() -> String {
        crate::core::astrometry::gwcs::test_support::documents_fits_dir().join(G395H_X1D).to_str().unwrap().to_string()
    }

    fn g395h_arrays(path: &str) -> X1dArrays {
        let x1d = crate::core::analysis::x1d::read_x1d_spectrum(path, None).unwrap();
        assert_eq!(x1d.dq_table, crate::core::imaging::dq_flags::DqTable::Jwst);
        X1dArrays {
            flux: x1d.flux.iter().map(|&v| v as f32).collect(),
            err: x1d.flux_error.expect("the G395H x1d has a FLUX_ERROR column"),
            excluded: x1d.dq.expect("the G395H x1d has a DQ column").iter().map(|&bits| bits & DO_NOT_USE as u32 != 0).collect(),
            axis: x1d.wavelength_um,
        }
    }

    fn g395h_request(line: (usize, usize), continuum: ContinuumWindows, model: LineModel) -> LineRequest {
        LineRequest {
            z0: line.0,
            z1: line.1,
            continuum: Some(continuum),
            rest_um: None,
            convention: VelocityConvention::Optical,
            model,
            velocity_shift_kms: None,
        }
    }

    fn considered_channels(line: (usize, usize), continuum: ContinuumWindows) -> Vec<usize> {
        let mut channels: Vec<usize> =
            (line.0..=line.1).chain(continuum.0 .0..=continuum.0 .1).chain(continuum.1 .0..=continuum.1 .1).collect();
        channels.sort_unstable();
        channels.dedup();
        channels
    }

    fn bad_err_channels(err: &[f64], channels: &[usize]) -> Vec<usize> {
        channels.iter().copied().filter(|&z| !(err[z].is_finite() && err[z] > 0.0)).collect()
    }

    #[test]
    #[ignore = "needs ASTROBURST_DOCUMENTS_FITS_DIR with the jw01266 G395H x1d and its local Br-alpha golden"]
    fn the_g395h_x1d_file_source_reproduces_the_br_alpha_golden_and_measures_it_with_its_flux_error() {
        let path = g395h_path();
        let source = SpectrumSource::File { hdu: None };
        let request = g395h_request(BR_ALPHA_LINE, BR_ALPHA_CONTINUUM, LineModel::Gaussian);
        let golden = read_golden(
            &crate::core::astrometry::gwcs::test_support::documents_fits_dir().join(G395H_BR_ALPHA_GOLDEN),
        );

        let plain = MeasureOptions { use_err: false, use_dq: false, wavelength_frame: None };
        let unweighted = without_elapsed(measure_line_json_with(&path, &source, &request, &plain).unwrap());
        let mut diffs = golden_diffs(&without_document_keys(golden.clone()), &without_document_keys(unweighted.clone()));
        diffs.extend(unweighted_diffs(&unweighted, 0));
        assert!(diffs.is_empty(), "{}", diffs.join("\n"));

        let value = measure_line_json(&path, &source, &request).unwrap();
        assert_eq!(value[RES_SOURCE], serde_json::json!({ "kind": "file", "hdu": 1 }));
        assert_eq!(value["err_used"], true);
        assert_eq!(value[KEY_ERR_SOURCE], "FLUX_ERROR column (HDU 1)");
        assert_eq!(value["flux_unit"], "Jy x um");
        assert_total_is_the_quadrature_of_the_terms(&value);

        let x1d = g395h_arrays(&path);
        let considered = considered_channels(BR_ALPHA_LINE, BR_ALPHA_CONTINUUM);
        assert_eq!(value["n_bad_err"], bad_err_channels(&x1d.err, &considered).len());
        assert_eq!(value["n_dq_excluded"], considered.iter().filter(|&&z| x1d.excluded[z]).count());
        assert_eq!(value["n_nonfinite"], considered.iter().filter(|&&z| !x1d.flux[z].is_finite()).count());
        assert_eq!((value["n_bad_err"].as_u64(), value["n_dq_excluded"].as_u64()), (Some(0), Some(0)));

        let same_arrays = measure_line_with_errors(
            &x1d.axis,
            &x1d.flux,
            Some(&x1d.err),
            Some(&x1d.excluded),
            BR_ALPHA_LINE,
            Some(BR_ALPHA_CONTINUUM),
            LineModel::Gaussian,
        )
        .unwrap();
        assert_eq!(number(&value, "flux").to_bits(), same_arrays.flux.to_bits());
        assert_eq!(number(&value, "centroid").to_bits(), same_arrays.centroid.to_bits());
        assert_eq!(number(&value, "flux_err").to_bits(), same_arrays.flux_err.to_bits());
        let scatter_error = number(&golden, "flux_err");
        assert!(
            (number(&value, "flux") - number(&golden, "flux")).abs() < scatter_error,
            "weighted flux {} against the unweighted golden {} +- {}",
            value["flux"],
            golden["flux"],
            scatter_error
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_DOCUMENTS_FITS_DIR with the jw01266 G395H x1d"]
    fn a_g395h_line_range_across_the_detector_gap_excludes_and_counts_its_58_zero_error_channels() {
        let path = g395h_path();
        let x1d = g395h_arrays(&path);
        let every_channel: Vec<usize> = (0..x1d.err.len()).collect();
        let gap: Vec<usize> = (G395H_DETECTOR_GAP.0..=G395H_DETECTOR_GAP.1).collect();
        assert_eq!(bad_err_channels(&x1d.err, &every_channel), gap);
        assert_eq!(gap.len(), G395H_GAP_CHANNELS);
        assert!(gap.iter().all(|&z| x1d.excluded[z]));

        let source = SpectrumSource::File { hdu: None };
        let request = g395h_request(ACROSS_GAP_LINE, ACROSS_GAP_CONTINUUM, LineModel::None);
        let line_channels = ACROSS_GAP_LINE.1 - ACROSS_GAP_LINE.0 + 1;
        let value = measure_line_json(&path, &source, &request).unwrap();
        assert_eq!(value["err_used"], true);
        assert_eq!(value["n_bad_err"], G395H_GAP_CHANNELS);
        assert_eq!(value["n_dq_excluded"], G395H_GAP_CHANNELS);
        assert_eq!(value["n_nonfinite"], 0);
        assert_eq!(value["n_channels"], line_channels - G395H_GAP_CHANNELS);
        let excluded_note = format!(
            "excluded among the {} line and continuum channels: {} DQ-flagged, 0 with non-finite flux, {} with ERR <= 0 or not finite; a channel can count under several reasons",
            considered_channels(ACROSS_GAP_LINE, ACROSS_GAP_CONTINUUM).len(),
            G395H_GAP_CHANNELS,
            G395H_GAP_CHANNELS
        );
        assert!(notes_of(&value).contains(&excluded_note), "{:?}", notes_of(&value));

        let plain = MeasureOptions { use_err: false, use_dq: false, wavelength_frame: None };
        let unweighted = measure_line_json_with(&path, &source, &request, &plain).unwrap();
        assert_eq!(unweighted["n_channels"], line_channels);
        assert_eq!((unweighted["n_bad_err"].as_u64(), unweighted["n_dq_excluded"].as_u64()), (Some(0), Some(0)));
    }
}
