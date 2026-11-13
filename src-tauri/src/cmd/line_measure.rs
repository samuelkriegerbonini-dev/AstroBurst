use std::time::Instant;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::cmd::common::blocking_cmd;
use crate::cmd::cube::{err_source, APERTURE_SUBSAMPLES, ERR_SOURCE_NONE};
use crate::core::analysis::line_measure::{
    line_redshift, measure_line_physical, resolve_windows, with_velocity, ContinuumWindows, LineMeasurement, LineModel,
    CGS_FLUX_PER_SI_FLUX,
};
use crate::core::analysis::spectrum1d::{Spectrum1d, Spectrum1dSource, WavelengthFrame, CSV_ERR_COLUMN};
use crate::core::analysis::tso::{find_series, open_tso, series_label, series_spectrum, SeriesKey, TsoPick};
use crate::core::analysis::x1d::{COL_FLUX_ERROR, EXTNAME_EXTRACT1D};
use crate::core::astrometry::spectral::{
    air_formula_applies, air_to_vacuum_um, vacuum_to_air_um, AxisKind, SpectralAxis, VelocityConvention,
    AIR_FORMULA_MIN_UM, AIR_VACUUM_FORMULA,
};
use crate::core::cube::companions::{open_cube_set, CubeSet};
use crate::core::cube::lazy::{ApertureSpectrum, LazyCube};
use crate::core::cube::linefit::dq_flagged;
use crate::core::cube::moments::channel_widths;
use crate::core::imaging::region::RegionShape;
use crate::core::metadata::flux_units::{fnu_jy_to_flambda_si_um, parse_spectral_flux_unit, SpectralFluxUnit};
use crate::core::metadata::photcal::{FluxConvention, PhotCal, JY_PER_MJY};
use crate::infra::fits::spectrum1d::load_spectrum1d;
use crate::infra::wcs_source::load_wcs;
use crate::types::constants::{EXTNAME_DQ, EXTNAME_ERR, HEADER_BUNIT, RES_SOURCE};

pub const MAX_LINE_CHANNEL: usize = 1 << 24;
pub const NATIVE_FLUX_UNIT: &str = "native";
pub const REGION_SUM_UNIT_SUFFIX: &str = " x pix";
pub const DEFAULT_CONVENTION: &str = "optical";
pub const FRAME_AIR: &str = "air";
pub const FRAME_VACUUM: &str = "vacuum";
pub const KEY_ERR_SOURCE: &str = "err_source";
pub const KEY_LINE_ID: &str = "line_id";
pub const KEY_LINE_LABEL: &str = "line_label";
pub const KEY_REDSHIFT: &str = "redshift";
pub const KEY_WAVELENGTH_FRAME: &str = "wavelength_frame";
pub const MAX_LINE_ID_CHARS: usize = 64;
pub const MAX_LINE_LABEL_CHARS: usize = 120;
const ASSUMED_FRAME_SUFFIX: &str = " assumed, no header card names the frame";
const NO_PHYSICAL_FLUX_PREFIX: &str = "no physical flux: ";
const SOSS_FLUX_LABEL: &str = "MJy";
const PER_BEAM_MARKER: &str = "beam";
const ANGSTROM_PER_UM: f64 = 1e4;
const SI_FLUX_PER_JY_GHZ: f64 = 1e-17;
const SERIES_NOTES_RESTATED: usize = 2;

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
    Series {
        paths: Vec<String>,
        detector: String,
        #[serde(default)]
        sporder: Option<i64>,
        pick: TsoPick,
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
    pub redshift: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineIdentity {
    pub id: Option<String>,
    pub label: Option<String>,
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

enum MeasuredAxis<'a> {
    VacuumWavelength,
    AirWavelength,
    VacuumFromAir(&'a [f64]),
    Frequency,
    Other,
}

enum FluxSource<'a> {
    File,
    Spaxel,
    Region(&'a LazyCube),
}

fn cube_measured_axis(axis: &SpectralAxis) -> MeasuredAxis<'_> {
    match axis.kind {
        AxisKind::Wave => MeasuredAxis::VacuumWavelength,
        AxisKind::Awav => MeasuredAxis::VacuumFromAir(&axis.values),
        AxisKind::Freq => MeasuredAxis::Frequency,
        _ => MeasuredAxis::Other,
    }
}

fn document_measured_axis(frame: WavelengthFrame, kind: AxisKind) -> MeasuredAxis<'static> {
    match (frame, kind) {
        (WavelengthFrame::Vacuum, _) => MeasuredAxis::VacuumWavelength,
        (WavelengthFrame::Air, _) => MeasuredAxis::AirWavelength,
        (WavelengthFrame::NotWavelength, AxisKind::Freq) => MeasuredAxis::Frequency,
        (WavelengthFrame::NotWavelength, _) => MeasuredAxis::Other,
    }
}

fn mjy_per_sr_pixel_solid_angle(cube: &LazyCube) -> Option<f64> {
    let wcs = load_wcs(&cube.source_path, &cube.header).ok();
    match PhotCal::from_header(&cube.header, wcs.as_ref())?.convention {
        FluxConvention::JwstMjySr { pixar_sr, .. } => Some(pixar_sr),
        _ => None,
    }
}

fn width_ratios(numerator_axis: &[f64], denominator_axis: &[f64]) -> Vec<f64> {
    channel_widths(numerator_axis).into_iter().zip(channel_widths(denominator_axis)).map(|(n, d)| n / d).collect()
}

fn physical_flux_factor(
    unit: Option<&str>,
    axis: &MeasuredAxis,
    measured: &[f64],
    source: &FluxSource,
) -> (Option<Vec<f64>>, String) {
    let refused = |reason: String| (None, format!("{}{}", NO_PHYSICAL_FLUX_PREFIX, reason));
    let Some(unit) = unit else {
        return refused("the spectrum has no flux unit".to_string());
    };
    if unit == SOSS_FLUX_LABEL {
        return refused(format!(
            "{} is the label JWST NIRISS SOSS spectra carry on values that are not on a MJy scale, so the unit cannot be trusted",
            unit
        ));
    }
    if unit.to_lowercase().contains(PER_BEAM_MARKER) {
        return refused(format!("{} is a brightness per beam and this version does not read the beam area", unit));
    }
    let (jy_per_unit, f_nu) = match parse_spectral_flux_unit(unit) {
        SpectralFluxUnit::Unknown => {
            return refused(format!("unit '{}' is not a flux density this version converts", unit));
        }
        SpectralFluxUnit::Rate => return refused(format!("{} is a count rate without a flux calibration", unit)),
        SpectralFluxUnit::FluxDensityLambda { cgs_per_unit } => {
            let si_per_unit = cgs_per_unit * ANGSTROM_PER_UM / CGS_FLUX_PER_SI_FLUX;
            return match axis {
                MeasuredAxis::VacuumWavelength | MeasuredAxis::AirWavelength => (
                    Some(vec![si_per_unit; measured.len()]),
                    format!("F_lambda in {} integrated over wavelength", unit),
                ),
                MeasuredAxis::VacuumFromAir(air) => (
                    Some(width_ratios(air, measured).into_iter().map(|ratio| si_per_unit * ratio).collect()),
                    format!(
                        "F_lambda in {} integrated over air wavelengths (axis converted with {} for the measurement)",
                        unit, AIR_VACUUM_FORMULA
                    ),
                ),
                MeasuredAxis::Frequency | MeasuredAxis::Other => {
                    refused("F_lambda is integrated over wavelength only".to_string())
                }
            };
        }
        SpectralFluxUnit::FluxDensityNu { jy_per_unit } => (jy_per_unit, format!("F_nu in {}", unit)),
        SpectralFluxUnit::SurfaceBrightness => match source {
            FluxSource::File => {
                return refused(format!("{} is a surface brightness and a spectrum file carries no solid angle", unit));
            }
            FluxSource::Spaxel => {
                return refused(format!("one spaxel in {} is an intensity, not a flux; measure a region", unit));
            }
            FluxSource::Region(cube) => match mjy_per_sr_pixel_solid_angle(cube) {
                Some(pixar_sr) => (
                    pixar_sr * JY_PER_MJY,
                    format!("{} summed over the region with a pixel solid angle of {:.4e} sr, then F_nu", unit, pixar_sr),
                ),
                None => {
                    return refused(format!(
                        "{} needs MJy/sr and a pixel solid angle (PIXAR_SR or a celestial WCS), which this cube does not give",
                        unit
                    ));
                }
            },
        },
    };
    match axis {
        MeasuredAxis::VacuumWavelength | MeasuredAxis::VacuumFromAir(_) => (
            Some(measured.iter().map(|&x| fnu_jy_to_flambda_si_um(jy_per_unit, x)).collect()),
            format!("{} integrated per channel with c / lambda^2", f_nu),
        ),
        MeasuredAxis::AirWavelength => {
            let vacuum: Vec<f64> = measured.iter().map(|&x| air_to_vacuum_um(x)).collect();
            let factor = vacuum
                .iter()
                .zip(width_ratios(&vacuum, measured))
                .map(|(&x, ratio)| fnu_jy_to_flambda_si_um(jy_per_unit, x) * ratio)
                .collect();
            (
                Some(factor),
                format!(
                    "{} integrated per channel with c / lambda^2 on vacuum wavelengths (air axis converted with {})",
                    f_nu, AIR_VACUUM_FORMULA
                ),
            )
        }
        MeasuredAxis::Frequency => (
            Some(vec![jy_per_unit * SI_FLUX_PER_JY_GHZ; measured.len()]),
            format!("{} integrated over frequency", f_nu),
        ),
        MeasuredAxis::Other => refused("the axis is not a wavelength or frequency axis".to_string()),
    }
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
        SpectrumSource::File { .. } | SpectrumSource::Series { .. } => {
            bail!("a spectrum file source has no cube spectrum to extract")
        }
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
            with_velocity(
                m,
                rest,
                request.redshift,
                request.convention,
                axis.kind,
                axis.specsys.as_deref(),
                request.velocity_shift_kms,
            );
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
) -> anyhow::Result<(LineMeasurement, String, Option<&'static str>)> {
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
    let unit = bunit(cube);
    let flux_source = match source {
        SpectrumSource::Region { .. } => FluxSource::Region(cube),
        _ => FluxSource::Spaxel,
    };
    let (channel_factor, physical_basis) =
        physical_flux_factor(unit.as_deref(), &cube_measured_axis(&axis), &axis_values, &flux_source);
    let mut m = measure_line_physical(
        &axis_values,
        &spectrum.flux,
        spectrum.err.as_deref(),
        spectrum.excluded.as_deref(),
        (request.z0, request.z1),
        request.continuum,
        request.model,
        channel_factor.as_deref(),
    )
    .map_err(anyhow::Error::msg)?;
    m.axis_unit = axis.unit.clone();
    m.flux_physical_basis = physical_basis;
    let native_unit = unit.unwrap_or_else(|| NATIVE_FLUX_UNIT.to_string());
    let spectrum_unit = if matches!(source, SpectrumSource::Region { .. }) {
        format!("{}{}", native_unit, REGION_SUM_UNIT_SUFFIX)
    } else {
        native_unit
    };
    m.flux_unit = format!("{} x {}", spectrum_unit, axis.unit);
    notes.append(&mut m.notes);
    m.notes = notes;
    add_velocity(&mut m, &axis, request.rest_um.or(axis.rest_wavelength_um), request);
    let measured_frame = matches!(axis.kind, AxisKind::Wave | AxisKind::Awav).then_some(FRAME_VACUUM);
    Ok((m, spectrum.err_source, measured_frame))
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

fn frame_name(frame: WavelengthFrame) -> Option<&'static str> {
    match frame {
        WavelengthFrame::Air => Some(FRAME_AIR),
        WavelengthFrame::Vacuum => Some(FRAME_VACUUM),
        WavelengthFrame::NotWavelength => None,
    }
}

fn document_hdu(source: &Spectrum1dSource) -> Option<usize> {
    match source {
        Spectrum1dSource::X1d { hdu, .. } | Spectrum1dSource::Wcs1d { hdu, .. } => Some(*hdu),
        Spectrum1dSource::Csv { .. } | Spectrum1dSource::X1dInts { .. } => None,
    }
}

fn document_label(source: &Spectrum1dSource) -> String {
    match source {
        Spectrum1dSource::X1d { hdu, .. } => format!("{} table in HDU {}", EXTNAME_EXTRACT1D, hdu),
        Spectrum1dSource::Wcs1d { hdu, .. } => format!("1D array in HDU {}", hdu),
        Spectrum1dSource::Csv { .. } => "CSV file".to_string(),
        Spectrum1dSource::X1dInts { .. } => "time-series pick".to_string(),
    }
}

fn document_err_source(source: &Spectrum1dSource, err_used: bool) -> String {
    match (source, err_used) {
        (_, false) => ERR_SOURCE_NONE.to_string(),
        (Spectrum1dSource::X1d { hdu, .. }, true) => format!("{} column (HDU {})", COL_FLUX_ERROR, hdu),
        (Spectrum1dSource::Wcs1d { .. }, true) => format!("{} extension", EXTNAME_ERR),
        (Spectrum1dSource::Csv { .. }, true) => format!("{} column (CSV)", CSV_ERR_COLUMN),
        (Spectrum1dSource::X1dInts { .. }, true) => format!("{} of the series pick", COL_FLUX_ERROR),
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

fn air_rest_um(rest_um: f64, redshift: Option<f64>, notes: &mut Vec<String>) -> f64 {
    let stretch = 1.0 + redshift.unwrap_or(0.0);
    let observed_um = rest_um * stretch;
    if !air_formula_applies(observed_um) {
        let located = match redshift {
            Some(z) => format!("rest wavelength {} um, at {} um for redshift {},", rest_um, observed_um, z),
            None => format!("rest wavelength {} um", rest_um),
        };
        notes.push(format!(
            "{} is below {} um where {} does not apply: left as vacuum",
            located, AIR_FORMULA_MIN_UM, AIR_VACUUM_FORMULA
        ));
        return rest_um;
    }
    let air = vacuum_to_air_um(observed_um) / stretch;
    notes.push(match redshift {
        Some(z) => format!(
            "vacuum rest wavelength {} um converted to {:.7} um in air with {} evaluated at {:.7} um, where the line falls at redshift {}, to match the air wavelength axis",
            rest_um, air, AIR_VACUUM_FORMULA, observed_um, z
        ),
        None => format!(
            "vacuum rest wavelength {} um converted to {:.7} um in air with {} to match the air wavelength axis",
            rest_um, air, AIR_VACUUM_FORMULA
        ),
    });
    air
}

fn measured_frame_text(doc: &Spectrum1d, frame: WavelengthFrame) -> String {
    let file_frame = format!(
        "{}{}",
        frame_text(doc.wavelength_frame),
        if doc.frame_assumed() { ASSUMED_FRAME_SUFFIX } else { "" }
    );
    if frame == doc.wavelength_frame {
        file_frame
    } else {
        format!("{} set by the request (the file reads as {})", frame_text(frame), file_frame)
    }
}

fn measure_document_line(
    doc: &Spectrum1d,
    frame: WavelengthFrame,
    mut notes: Vec<String>,
    request: &LineRequest,
    options: &MeasureOptions,
) -> anyhow::Result<(LineMeasurement, String, Option<&'static str>)> {
    let err = document_errors(doc, options.use_err, &mut notes);
    let err_label = document_err_source(&doc.source, err.is_some());
    let excluded = document_exclusions(doc, options.use_dq, &mut notes);
    let flux = measured_flux(&doc.flux, &mut notes);
    let (channel_factor, physical_basis) = physical_flux_factor(
        doc.flux_unit.as_deref().map(str::trim).filter(|unit| !unit.is_empty()),
        &document_measured_axis(frame, doc.axis.kind),
        &doc.axis.values,
        &FluxSource::File,
    );
    let mut m = measure_line_physical(
        &doc.axis.values,
        &flux,
        err,
        excluded.as_deref(),
        (request.z0, request.z1),
        request.continuum,
        request.model,
        channel_factor.as_deref(),
    )
    .map_err(anyhow::Error::msg)?;
    m.axis_unit = doc.axis.unit.clone();
    m.flux_physical_basis = physical_basis;
    m.flux_unit = format!("{} x {}", doc.flux_unit.as_deref().unwrap_or(NATIVE_FLUX_UNIT), doc.axis.unit);
    notes.append(&mut m.notes);
    m.notes = notes;
    let rest_vacuum = request.rest_um.or(doc.axis.rest_wavelength_um);
    let rest = match (rest_vacuum, frame) {
        (Some(rest), WavelengthFrame::Air) => Some(air_rest_um(rest, request.redshift, &mut m.notes)),
        (rest, _) => rest,
    };
    add_velocity(&mut m, &doc.axis, rest, request);
    if let (WavelengthFrame::Air, Some(rest_vacuum), Some(velocity)) = (frame, rest_vacuum, m.velocity.as_mut()) {
        let centroid_vacuum = air_to_vacuum_um(m.centroid);
        let centroid_err_vacuum = m.centroid_err * centroid_vacuum / m.centroid;
        (velocity.z_line, velocity.z_line_err) =
            line_redshift(centroid_vacuum, centroid_err_vacuum, rest_vacuum, velocity.shift_applied_kms);
    }
    Ok((m, err_label, frame_name(frame)))
}

fn measure_file_line(
    path: &str,
    hdu: Option<usize>,
    request: &LineRequest,
    options: &MeasureOptions,
) -> anyhow::Result<(LineMeasurement, SpectrumSource, String, Option<&'static str>)> {
    let doc = load_spectrum1d(path, hdu)?;
    let frame = document_frame(doc.wavelength_frame, options.wavelength_frame).map_err(anyhow::Error::msg)?;
    let notes =
        vec![format!("spectrum file, {}: {}", document_label(&doc.source), measured_frame_text(&doc, frame))];
    let (m, err_label, measured_frame) = measure_document_line(&doc, frame, notes, request, options)?;
    Ok((m, SpectrumSource::File { hdu: document_hdu(&doc.source) }, err_label, measured_frame))
}

fn pick_text(pick: &TsoPick, n_integrations: usize) -> String {
    match *pick {
        TsoPick::Integration { index } => format!("integration {}", index + 1),
        TsoPick::Median => format!("median of {} integrations", n_integrations),
        TsoPick::Mean { first, last } => format!("mean of integrations {} to {}", first + 1, last + 1),
    }
}

fn measure_series_line(
    paths: &[String],
    key: &SeriesKey,
    pick: &TsoPick,
    request: &LineRequest,
    options: &MeasureOptions,
) -> anyhow::Result<(LineMeasurement, String, Option<&'static str>)> {
    let set = open_tso(paths)?;
    let series = find_series(&set, key)?;
    let doc = series_spectrum(&set, key, pick)?;
    let frame = document_frame(doc.wavelength_frame, options.wavelength_frame).map_err(anyhow::Error::msg)?;
    let keys: Vec<SeriesKey> = set.series.iter().map(|found| found.key.clone()).collect();
    let mut notes = vec![format!(
        "time series {}, {}: {}",
        series_label(key, &keys),
        pick_text(pick, series.n_integrations),
        measured_frame_text(&doc, frame)
    )];
    notes.extend(doc.notes.iter().skip(SERIES_NOTES_RESTATED).cloned());
    notes.extend(series.warnings.iter().cloned());
    measure_document_line(&doc, frame, notes, request, options)
}

#[cfg(test)]
pub(crate) fn measure_line_json(path: &str, source: &SpectrumSource, request: &LineRequest) -> anyhow::Result<serde_json::Value> {
    measure_line_json_with(path, source, request, &MeasureOptions::default())
}

#[cfg(test)]
pub(crate) fn measure_line_json_with(
    path: &str,
    source: &SpectrumSource,
    request: &LineRequest,
    options: &MeasureOptions,
) -> anyhow::Result<serde_json::Value> {
    measure_line_json_identified(path, source, request, options, &LineIdentity::default())
}

pub(crate) fn measure_line_json_identified(
    path: &str,
    source: &SpectrumSource,
    request: &LineRequest,
    options: &MeasureOptions,
    identity: &LineIdentity,
) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let (mut m, measured, err_label, measured_frame) = match source {
        SpectrumSource::File { hdu } => measure_file_line(path, *hdu, request, options)?,
        SpectrumSource::Series { paths, detector, sporder, pick } => {
            let key = SeriesKey { detector: detector.clone(), sporder: *sporder };
            let (m, err_label, measured_frame) = measure_series_line(paths, &key, pick, request, options)?;
            (m, source.clone(), err_label, measured_frame)
        }
        SpectrumSource::Pixel { .. } | SpectrumSource::Region { .. } => {
            let (m, err_label, measured_frame) = measure_cube_line(path, source, request, options)?;
            (m, source.clone(), err_label, measured_frame)
        }
    };
    m.elapsed_ms = t0.elapsed().as_millis() as u64;
    let mut value = serde_json::to_value(&m)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(RES_SOURCE.to_string(), serde_json::to_value(&measured)?);
        object.insert(KEY_ERR_SOURCE.to_string(), serde_json::Value::String(err_label));
        object.insert(KEY_LINE_ID.to_string(), serde_json::to_value(&identity.id)?);
        object.insert(KEY_LINE_LABEL.to_string(), serde_json::to_value(&identity.label)?);
        object.insert(KEY_REDSHIFT.to_string(), serde_json::to_value(request.redshift)?);
        object.insert(KEY_WAVELENGTH_FRAME.to_string(), serde_json::to_value(measured_frame)?);
    }
    Ok(value)
}

fn identity_text(name: &str, text: Option<String>, max_chars: usize) -> Result<Option<String>, String> {
    let Some(trimmed) = text.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let length = trimmed.chars().count();
    if length > max_chars {
        return Err(format!("{} must be at most {} characters, got {}", name, max_chars, length));
    }
    Ok(Some(trimmed.to_string()))
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
    redshift: Option<f64>,
    line_id: Option<String>,
    line_label: Option<String>,
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
    if let Some(z) = redshift {
        if !(z.is_finite() && z > -1.0) {
            return Err(format!("redshift must be a finite number above -1, got {}", z));
        }
    }
    let identity = LineIdentity {
        id: identity_text(KEY_LINE_ID, line_id, MAX_LINE_ID_CHARS)?,
        label: identity_text(KEY_LINE_LABEL, line_label, MAX_LINE_LABEL_CHARS)?,
    };
    let convention = VelocityConvention::parse(convention.as_deref().unwrap_or(DEFAULT_CONVENTION))?;
    let model = LineModel::parse(model.as_deref().unwrap_or(LineModel::None.name()))?;
    let wavelength_frame = wavelength_frame.as_deref().map(parse_wavelength_frame).transpose()?;
    let options = MeasureOptions { use_err: use_err.unwrap_or(true), use_dq: use_dq.unwrap_or(true), wavelength_frame };
    let request = LineRequest { z0, z1, continuum, rest_um, convention, model, velocity_shift_kms, redshift };
    blocking_cmd!(measure_line_json_identified(&path, &source, &request, &options, &identity))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::line_measure::measure_line_with_errors;
    use crate::core::analysis::line_measure::golden::{assert_matches_golden, fixtures_dir, without_elapsed};
    use crate::cmd::spectrum1d::test_fixtures::{iraf_linear_cards, write_wcs1d, write_x1d_columns};
    use crate::core::analysis::line_measure::golden::{golden_diffs, read_golden, unweighted_diffs};
    use crate::core::cube::lazy::{region_pixels, test_support::*};
    use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
    use crate::core::cube::companions::open_cube_set;
    use crate::core::cube::moments::channel_widths;
    use crate::core::analysis::tso::{DATAMODEL_ROWS, MEDIAN_ERROR_FACTOR};
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::x1dints::test_fixtures::{ramp_row, rows_hdu, series_primary, write_series_file, SynthRow, SynthTable};

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
            redshift: None,
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
            for key in [KEY_ERR_SOURCE, KEY_LINE_ID, KEY_LINE_LABEL, KEY_REDSHIFT, KEY_WAVELENGTH_FRAME] {
                value.as_object_mut().unwrap().remove(key);
            }
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

    const DOCUMENT_KEYS: [&str; 9] = [
        "axis_unit",
        "flux_unit",
        "notes",
        RES_SOURCE,
        KEY_ERR_SOURCE,
        KEY_LINE_ID,
        KEY_LINE_LABEL,
        KEY_REDSHIFT,
        KEY_WAVELENGTH_FRAME,
    ];
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
        let chi2_red = number(&value, "continuum_chi2_red");
        assert!(chi2_red > 0.0 && chi2_red < 1.0, "chi2_red={}", chi2_red);
        assert_eq!(value["n_channels_nominal"], 25);
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
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(refused.contains("unknown wavelength frame"), "{}", refused);
    }

    #[tokio::test]
    async fn the_command_reports_the_errors_of_every_quantity_of_a_file_measurement() {
        let dir = tempfile::tempdir().unwrap();
        let (axis, flux) = centre_arrays(0.0);
        let n = axis.len();
        let path = write_document(&dir, "errors_x1d.fits", &axis, &flux, &vec![DOCUMENT_ERR; n], &vec![0; n]);
        let value = measure_spectral_line_cmd(
            path,
            file_source(),
            LINE_RANGE.0,
            LINE_RANGE.1,
            Some(WINDOWS),
            Some(LINE_REST_UM),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let velocity = &value["velocity"];
        let oracle = [
            (&value, "equivalent_width_err", 5.319820064779271e-4),
            (&value, "centroid_err", 2.807258815067002e-4),
            (&value, "sigma_err", 4.913453585348932e-4),
            (&value, "fwhm_err", 1.1570298993108848e-3),
            (velocity, "centroid_err_kms", 82.50931572657882),
        ];
        for (object, key, expected) in oracle {
            let found = number(object, key);
            assert!((found - expected).abs() <= 1e-9 * expected, "{key}: found {found:?}, oracle {expected:?}");
        }
        assert_eq!(value["errors_reliable"], true);
        assert_eq!(value["width_errors_reliable"], false);
        let measurement_keys =
            ["equivalent_width_err", "centroid_err", "sigma_err", "fwhm_err", "errors_reliable", "width_errors_reliable"];
        let velocity_keys = [
            "centroid_err_kms",
            "sigma_err_kms",
            "fwhm_err_kms",
            "source_centroid_err_kms",
            "source_sigma_err_kms",
            "source_fwhm_err_kms",
        ];
        for key in measurement_keys {
            assert!(value.get(key).is_some_and(|v| !v.is_null()), "{key} missing in {value}");
        }
        for key in velocity_keys {
            assert!(velocity.get(key).is_some_and(|v| !v.is_null()), "{key} missing in {velocity}");
        }
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
        assert_eq!(value["n_channels_nominal"], 25);
        let dropped_inside_the_line = "6 of the 25 channels in the line range 8..=32 were dropped (3 DQ-flagged, 1 with non-finite flux, 2 with ERR <= 0 or not finite)";
        assert!(notes_of(&value).iter().any(|n| n.starts_with(dropped_inside_the_line)), "{:?}", notes_of(&value));
        assert_eq!(value[RES_SOURCE], serde_json::json!({ "kind": "file", "hdu": 1 }));

        let without_dq = MeasureOptions { use_dq: false, ..MeasureOptions::default() };
        let value = measure_line_json_with(&path, &source, &request(LineModel::None), &without_dq).unwrap();
        assert_eq!(value["n_dq_excluded"], 0);
        assert_eq!(value["n_channels"], 25 - 3);
        assert!(notes_of(&value).iter().any(|n| n.contains("use_dq = false")), "{:?}", notes_of(&value));
    }

    fn h_alpha_air_document(dir: &tempfile::TempDir) -> String {
        h_alpha_air_document_with(dir, "h_alpha_air.fits", &[])
    }

    fn h_alpha_air_flux() -> Vec<f32> {
        let centre = vacuum_to_air_um(H_ALPHA_VACUUM_UM) * 1e4;
        (0..H_ALPHA_CHANNELS)
            .map(|i| {
                let d = (H_ALPHA_START_ANGSTROM + H_ALPHA_STEP_ANGSTROM * i as f64 - centre) / H_ALPHA_SIGMA_ANGSTROM;
                (1.0 + (-0.5 * d * d).exp()) as f32
            })
            .collect()
    }

    fn h_alpha_air_document_with(dir: &tempfile::TempDir, name: &str, frame_cards: &[(&'static str, String)]) -> String {
        let mut cards = iraf_linear_cards(H_ALPHA_START_ANGSTROM, H_ALPHA_STEP_ANGSTROM, H_ALPHA_BUNIT);
        cards.extend(frame_cards.iter().cloned());
        write_wcs1d(dir.path(), name, &cards, &h_alpha_air_flux()).to_str().unwrap().to_string()
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
            redshift: None,
        }
    }

    #[test]
    fn an_air_document_converts_the_vacuum_rest_wavelength_with_morton_before_velocities() {
        let dir = tempfile::tempdir().unwrap();
        let path = h_alpha_air_document(&dir);
        let air_rest = vacuum_to_air_um(H_ALPHA_VACUUM_UM);
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
    fn a_measurement_on_a_file_whose_frame_no_card_names_says_that_the_frame_is_assumed() {
        let dir = tempfile::tempdir().unwrap();
        let assumed = h_alpha_air_document(&dir);
        let stated = h_alpha_air_document_with(&dir, "h_alpha_airorvac.fits", &[("AIRORVAC", "'air'".to_string())]);
        let assumed_air = "spectrum file, 1D array in HDU 0: air wavelengths assumed, no header card names the frame";
        let stated_air = "spectrum file, 1D array in HDU 0: air wavelengths";
        for (path, requested, note) in [
            (&assumed, None, assumed_air),
            (&assumed, Some(WavelengthFrame::Air), assumed_air),
            (
                &assumed,
                Some(WavelengthFrame::Vacuum),
                "spectrum file, 1D array in HDU 0: vacuum wavelengths set by the request (the file reads as air wavelengths assumed, no header card names the frame)",
            ),
            (&stated, None, stated_air),
            (&stated, Some(WavelengthFrame::Air), stated_air),
            (
                &stated,
                Some(WavelengthFrame::Vacuum),
                "spectrum file, 1D array in HDU 0: vacuum wavelengths set by the request (the file reads as air wavelengths)",
            ),
        ] {
            let options = MeasureOptions { wavelength_frame: requested, ..MeasureOptions::default() };
            let value = measure_line_json_with(path, &file_source(), &h_alpha_request(), &options).unwrap();
            assert_eq!(notes_of(&value)[0], note, "{path} with the request frame {requested:?}");
        }
    }

    const REDSHIFT: f64 = 0.5;
    const SOURCE_SIGMA_KMS: f64 = 100.0;
    const REDSHIFTED_LINE: (usize, usize) = (16, 64);
    const REDSHIFTED_WINDOWS: ContinuumWindows = ((0, 9), (71, 80));
    const SOURCE_VELOCITY_TOLERANCE_KMS: f64 = 0.05;
    const UNDERSTATED_ERR: f64 = 0.005;

    fn gaussian_line(axis: &[f64], centre: f64, sigma: f64) -> Vec<f32> {
        axis.iter()
            .map(|&x| {
                let d = (x - centre) / sigma;
                (1.0 + (-0.5 * d * d).exp()) as f32
            })
            .collect()
    }

    fn redshifted_h_alpha_x1d(dir: &tempfile::TempDir) -> String {
        let centre = H_ALPHA_VACUUM_UM * (1.0 + REDSHIFT);
        let sigma_um = centre * SOURCE_SIGMA_KMS / SPEED_OF_LIGHT_KMS;
        let axis: Vec<f64> = (0..81).map(|i| centre + (i as f64 - 40.0) * sigma_um / 4.0).collect();
        let flux = gaussian_line(&axis, centre, sigma_um);
        let n = axis.len();
        write_document(dir, "redshifted_x1d.fits", &axis, &flux, &vec![DOCUMENT_ERR; n], &vec![0; n])
    }

    #[tokio::test]
    async fn the_command_echoes_the_line_identity_and_the_requested_redshift() {
        let dir = tempfile::tempdir().unwrap();
        let path = redshifted_h_alpha_x1d(&dir);
        let measure = |redshift: Option<f64>, line_id: Option<&str>, line_label: Option<&str>| {
            measure_spectral_line_cmd(
                path.clone(),
                file_source(),
                REDSHIFTED_LINE.0,
                REDSHIFTED_LINE.1,
                Some(REDSHIFTED_WINDOWS),
                Some(H_ALPHA_VACUUM_UM),
                None,
                None,
                None,
                None,
                None,
                None,
                redshift,
                line_id.map(str::to_string),
                line_label.map(str::to_string),
            )
        };
        let null = serde_json::Value::Null;

        let value = measure(Some(REDSHIFT), Some("h_alpha"), Some(" Hα ")).await.unwrap();
        assert_eq!(value[KEY_LINE_ID], "h_alpha");
        assert_eq!(value[KEY_LINE_LABEL], "Hα");
        assert_eq!(value[KEY_REDSHIFT], REDSHIFT);
        assert_eq!(value[KEY_WAVELENGTH_FRAME], FRAME_VACUUM);
        let velocity = &value["velocity"];
        let z_line = number(velocity, "z_line");
        assert!((z_line - REDSHIFT).abs() < 1e-7, "z_line={z_line:?}");
        let z_line_err = number(velocity, "z_line_err");
        assert_relative("z_line_err", z_line_err, number(&value, "centroid_err") / H_ALPHA_VACUUM_UM, 1e-12);
        assert_relative("z_line_err against the oracle", z_line_err, 7.270589272315482e-5, 1e-9);

        let anonymous = measure(None, None, None).await.unwrap();
        for key in [KEY_LINE_ID, KEY_LINE_LABEL, KEY_REDSHIFT] {
            assert_eq!(anonymous.get(key), Some(&null), "{key} in {anonymous}");
        }
        assert_eq!(number(&anonymous["velocity"], "redshift"), 0.0);
        assert_eq!(anonymous["velocity"]["z_line"], velocity["z_line"]);

        let blank = measure(None, Some(""), Some("   ")).await.unwrap();
        assert_eq!(blank.get(KEY_LINE_ID), Some(&null));
        assert_eq!(blank.get(KEY_LINE_LABEL), Some(&null));

        let longest_id = "x".repeat(64);
        assert_eq!(measure(None, Some(&longest_id), None).await.unwrap()[KEY_LINE_ID], longest_id.as_str());
        let refused = measure(None, Some(&"x".repeat(65)), None).await.unwrap_err();
        assert_eq!(refused, "line_id must be at most 64 characters, got 65");
        let refused = measure(None, None, Some(&"x".repeat(121))).await.unwrap_err();
        assert_eq!(refused, "line_label must be at most 120 characters, got 121");

        let greek = "α".repeat(120);
        assert_eq!(greek.len(), 240);
        assert_eq!(measure(None, None, Some(&greek)).await.unwrap()[KEY_LINE_LABEL], greek.as_str());
        let refused = measure(None, None, Some(&"α".repeat(121))).await.unwrap_err();
        assert_eq!(refused, "line_label must be at most 120 characters, got 121");

        let letters = "x".repeat(120);
        let padded = format!(" {letters} ");
        assert_eq!(measure(None, None, Some(&padded)).await.unwrap()[KEY_LINE_LABEL], letters.as_str());
    }

    #[tokio::test]
    async fn the_command_takes_a_redshift_and_reports_source_frame_velocities_beside_the_observer_frame_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = redshifted_h_alpha_x1d(&dir);
        let measure = |redshift: Option<f64>| {
            measure_spectral_line_cmd(
                path.clone(),
                file_source(),
                REDSHIFTED_LINE.0,
                REDSHIFTED_LINE.1,
                Some(REDSHIFTED_WINDOWS),
                Some(H_ALPHA_VACUUM_UM),
                None,
                None,
                None,
                None,
                None,
                None,
                redshift,
                None,
                None,
            )
        };
        let value = measure(Some(REDSHIFT)).await.unwrap();
        let velocity = &value["velocity"];
        assert_eq!(number(velocity, "redshift"), REDSHIFT);
        assert_relative("source-frame sigma", number(velocity, "source_sigma_kms"), SOURCE_SIGMA_KMS, 1e-3);
        assert_relative("source-frame FWHM", number(velocity, "source_fwhm_kms"), SOURCE_SIGMA_KMS * 2.354_820_045, 1e-3);
        assert!(number(velocity, "source_centroid_kms").abs() < SOURCE_VELOCITY_TOLERANCE_KMS, "{}", velocity);
        assert_relative("observer-frame sigma", number(velocity, "sigma_kms"), 150.0, 1e-3);
        assert_relative("observer-frame centroid", number(velocity, "centroid_kms"), SPEED_OF_LIGHT_KMS * REDSHIFT, 1e-6);
        assert!(notes_of(&value).iter().any(|n| n.starts_with("redshift 0.5: source_centroid_kms")), "{:?}", notes_of(&value));

        let without = measure(None).await.unwrap();
        assert_eq!(number(&without["velocity"], "redshift"), 0.0);
        assert_eq!(without["velocity"]["source_centroid_kms"], without["velocity"]["centroid_kms"]);
        assert_eq!(without["velocity"]["centroid_kms"], velocity["centroid_kms"]);
        assert_eq!(without["velocity"]["sigma_kms"], velocity["sigma_kms"]);
        assert_relative("source-frame sigma", number(&without["velocity"], "source_sigma_kms"), SOURCE_SIGMA_KMS, 1e-3);
        assert!(notes_of(&without).iter().all(|n| !n.starts_with("redshift")), "{:?}", notes_of(&without));

        for (bad, shown) in [(-1.0, "-1"), (f64::NAN, "NaN"), (f64::INFINITY, "inf")] {
            let refused = measure(Some(bad)).await.unwrap_err();
            assert_eq!(refused, format!("redshift must be a finite number above -1, got {}", shown));
        }
    }

    fn redshifted_h_alpha_air_document(dir: &tempfile::TempDir) -> (String, f64) {
        let centre = vacuum_to_air_um(H_ALPHA_VACUUM_UM * (1.0 + REDSHIFT)) * 1e4;
        let sigma = centre * SOURCE_SIGMA_KMS / SPEED_OF_LIGHT_KMS;
        let (start, step) = (9796.0, 0.8);
        let axis: Vec<f64> = (0..121).map(|i| start + step * i as f64).collect();
        let cards = iraf_linear_cards(start, step, H_ALPHA_BUNIT);
        let path = write_wcs1d(dir.path(), "h_alpha_air_redshifted.fits", &cards, &gaussian_line(&axis, centre, sigma));
        (path.to_str().unwrap().to_string(), centre)
    }

    fn redshifted_air_request(redshift: Option<f64>) -> LineRequest {
        LineRequest { z0: 35, z1: 85, continuum: Some(((0, 20), (100, 120))), redshift, ..h_alpha_request() }
    }

    #[test]
    fn an_air_document_measured_at_a_redshift_converts_the_rest_wavelength_where_the_line_is_observed() {
        let dir = tempfile::tempdir().unwrap();
        let (path, centre) = redshifted_h_alpha_air_document(&dir);
        let value = measure_line_json(&path, &file_source(), &redshifted_air_request(Some(REDSHIFT))).unwrap();
        let velocity = &value["velocity"];
        assert!(number(velocity, "source_centroid_kms").abs() < SOURCE_VELOCITY_TOLERANCE_KMS, "{}", velocity);
        assert_relative("source-frame sigma", number(velocity, "source_sigma_kms"), SOURCE_SIGMA_KMS, 1e-3);
        assert_relative("observer-frame centroid", number(velocity, "centroid_kms"), SPEED_OF_LIGHT_KMS * REDSHIFT, 1e-6);
        let notes = notes_of(&value);
        assert!(notes.iter().any(|n| n.contains("where the line falls at redshift 0.5")), "{:?}", notes);
        let naive_rest = vacuum_to_air_um(H_ALPHA_VACUUM_UM) * (1.0 + REDSHIFT);
        let naive_kms = SPEED_OF_LIGHT_KMS * (centre * 1e-4 - naive_rest) / naive_rest;
        assert!(naive_kms.abs() > 4.0 * SOURCE_VELOCITY_TOLERANCE_KMS, "naive_kms={}", naive_kms);
    }

    #[test]
    fn the_redshift_of_a_line_on_an_air_document_does_not_depend_on_a_typed_redshift() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = redshifted_h_alpha_air_document(&dir);
        for typed_redshift in [Some(REDSHIFT), None] {
            let value = measure_line_json(&path, &file_source(), &redshifted_air_request(typed_redshift)).unwrap();
            let z_line = number(&value["velocity"], "z_line");
            assert!((z_line - REDSHIFT).abs() < 5e-7, "typed {typed_redshift:?}: z_line={z_line:?}");
        }
        let value = measure_line_json(&path, &file_source(), &redshifted_air_request(None)).unwrap();
        assert_eq!(value[KEY_WAVELENGTH_FRAME], FRAME_AIR);
        let as_vacuum = MeasureOptions { wavelength_frame: Some(WavelengthFrame::Vacuum), ..MeasureOptions::default() };
        let value = measure_line_json_with(&path, &file_source(), &redshifted_air_request(None), &as_vacuum).unwrap();
        assert_eq!(value[KEY_WAVELENGTH_FRAME], FRAME_VACUUM);
    }

    #[test]
    fn the_wavelength_frame_of_a_cube_is_vacuum_and_absent_on_other_axes() {
        let dir = tempfile::tempdir().unwrap();
        let wave = write(&dir, "line.fits", 0.0);
        let value = measure_line_json(&wave, &centre_pixel(), &request(LineModel::None)).unwrap();
        assert_eq!(value[KEY_WAVELENGTH_FRAME], FRAME_VACUUM);

        let air = dir.path().join("air.fits");
        write_line_cube_with_cards(&air, &[("CTYPE3", "'AWAV'")]);
        let value = measure_line_json(air.to_str().unwrap(), &centre_pixel(), &request(LineModel::None)).unwrap();
        assert_eq!(value[KEY_WAVELENGTH_FRAME], FRAME_VACUUM);
        let peak_axis = number(&value, "peak_axis");
        assert!((peak_axis - air_to_vacuum_um(LINE_REST_UM)).abs() < 1e-9, "peak_axis={peak_axis:?}");

        let frequency = dir.path().join("frequency.fits");
        write_line_cube_with_cards(&frequency, &[("CTYPE3", "'FREQ'"), ("CUNIT3", "'GHz'")]);
        let value = measure_line_json(frequency.to_str().unwrap(), &centre_pixel(), &request(LineModel::None)).unwrap();
        assert_eq!(value.get(KEY_WAVELENGTH_FRAME), Some(&serde_json::Value::Null), "{}", value);

        let (axis, flux) = centre_arrays(0.05);
        let csv = write_csv_document(&dir, &axis, &flux, &vec![DOCUMENT_ERR; axis.len()]);
        let value = measure_line_json(&csv, &file_source(), &request(LineModel::None)).unwrap();
        assert_eq!(value[KEY_WAVELENGTH_FRAME], FRAME_VACUUM);
    }

    #[test]
    fn a_window_overlapping_the_line_reaches_the_notes_of_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "line.fits", 0.0);
        let mut overlapping = request(LineModel::None);
        overlapping.continuum = Some(((0, 5), (30, 39)));
        let note = "continuum window 30..=39 overlaps the line range 8..=32 in 3 channels: line flux is fitted as continuum there, and the errors treat line and continuum channels as independent";
        for source in [centre_pixel(), disk_source()] {
            let notes = notes_of(&measure_line_json(&path, &source, &overlapping).unwrap());
            assert_eq!(notes.iter().filter(|n| *n == note).count(), 1, "{:?}", notes);
        }
    }

    #[test]
    fn a_file_whose_flux_error_understates_the_continuum_scatter_has_its_flux_error_scaled_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let (axis, flux) = centre_arrays(0.05);
        let n = axis.len();
        let err = vec![UNDERSTATED_ERR; n];
        let path = write_document(&dir, "understated_x1d.fits", &axis, &flux, &err, &vec![0; n]);
        let value = measure_line_json(&path, &file_source(), &request(LineModel::None)).unwrap();
        let scale = number(&value, "continuum_chi2_red").sqrt();
        assert!(scale > 5.0 && scale < 7.0, "scale={}", scale);
        assert_total_is_the_quadrature_of_the_terms(&value);
        let propagated_pixel = pixel_term(&err, &axis, LINE_RANGE.0..=LINE_RANGE.1);
        assert_relative("scaled pixel term", number(&value["flux_err_terms"], "pixel"), scale * propagated_pixel, 1e-9);
        let scatter_path = MeasureOptions { use_err: false, ..MeasureOptions::default() };
        let unweighted = measure_line_json_with(&path, &file_source(), &request(LineModel::None), &scatter_path).unwrap();
        assert_relative("scatter-path flux error", number(&value, "flux_err"), number(&unweighted, "flux_err"), 1e-9);
        let notes = notes_of(&value);
        assert!(notes.iter().any(|n| n.contains("ERR understates the scatter")), "{:?}", notes);
        assert!(notes.iter().any(|n| n.contains("each scaled by 6.0")), "{:?}", notes);
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
        assert!(value["continuum_chi2_red"].is_null(), "{}", value["continuum_chi2_red"]);
        assert_total_is_the_quadrature_of_the_terms(&value);
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
            assert!(value["continuum_chi2_red"].is_null(), "{}", value["continuum_chi2_red"]);
            assert_eq!(value["n_dq_excluded"], 0);
            assert_total_is_the_quadrature_of_the_terms(&value);
            let terms = &value["flux_err_terms"];
            let widths = channel_widths(&LazyCube::open(&path).unwrap().spectral_axis().unwrap().values);
            let line_widths = &widths[LINE_RANGE.0..=LINE_RANGE.1];
            let residual_rms = number(terms, "pixel") / line_widths.iter().map(|w| w * w).sum::<f64>().sqrt();
            let continuum_sigma = number(&value, "continuum_sigma");
            assert!(residual_rms > 0.5 * continuum_sigma && residual_rms < 2.0 * continuum_sigma, "{} {}", residual_rms, continuum_sigma);
            let level = residual_rms * line_widths.iter().sum::<f64>() / 12f64.sqrt();
            assert_relative("level term", number(terms, "level"), level, 1e-9);
            assert!(number(terms, "slope") > 0.0 && number(terms, "slope") < number(terms, "pixel"), "{}", terms);
            assert!(number(&value, "flux_err") > 1.7 * number(terms, "pixel"), "{}", value);
        }
        GLOBAL_CUBE_CACHE.invalidate(&path);
    }

    const PHYSICAL_FLUX_KEYS: [&str; 4] = ["flux_si", "flux_si_err", "flux_cgs", "flux_cgs_err"];
    const KEY_PHYSICAL_BASIS: &str = "flux_physical_basis";
    const F_NU_PER_CHANNEL_BASIS: &str = "F_nu in Jy integrated per channel with c / lambda^2";
    const CENTRE_PIXEL_FLUX_SI: f64 = 2.166841286254905e-14;
    const CENTRE_PIXEL_FLUX_SI_ERR: f64 = 1.2654046416104045e-15;
    const SURFACE_BRIGHTNESS_CARDS: [(&str, &str); 2] = [("BUNIT", "'MJy/sr'"), ("PIXAR_SR", "3.0461742E-12")];
    const JY_PER_SUMMED_MJY_PER_SR: f64 = 3.0461742e-6;

    fn write_with_cards(dir: &tempfile::TempDir, name: &str, cards: &[(&'static str, &'static str)]) -> String {
        let path = dir.path().join(name);
        write_line_cube_with_cards(&path, cards);
        path.to_str().unwrap().to_string()
    }

    fn measured(path: &str, source: &SpectrumSource) -> serde_json::Value {
        measure_line_json(path, source, &request(LineModel::None)).unwrap()
    }

    fn assert_no_physical_flux(value: &serde_json::Value, basis: &str) {
        for key in PHYSICAL_FLUX_KEYS {
            assert_eq!(value.get(key), Some(&serde_json::Value::Null), "{key} in {value}");
        }
        assert_eq!(value[KEY_PHYSICAL_BASIS], basis);
    }

    #[test]
    fn an_x1d_line_flux_in_jy_is_also_given_in_watts_per_square_metre() {
        let dir = tempfile::tempdir().unwrap();
        let (axis, flux) = centre_arrays(0.0);
        let n = axis.len();
        let path = write_document(&dir, "physical_x1d.fits", &axis, &flux, &vec![DOCUMENT_ERR; n], &vec![0; n]);
        let value = measured(&path, &file_source());
        let (flux_si, flux_si_err) = (number(&value, "flux_si"), number(&value, "flux_si_err"));
        assert_relative("flux_si", flux_si, CENTRE_PIXEL_FLUX_SI, 1e-9);
        assert_relative("flux_si_err", flux_si_err, CENTRE_PIXEL_FLUX_SI_ERR, 1e-9);
        assert_relative("flux_cgs", number(&value, "flux_cgs"), 1000.0 * flux_si, 1e-12);
        assert_relative("flux_cgs_err", number(&value, "flux_cgs_err"), 1000.0 * flux_si_err, 1e-12);
        assert_eq!(value[KEY_PHYSICAL_BASIS], F_NU_PER_CHANNEL_BASIS);
        assert_eq!(value["flux_unit"], "Jy x um");
    }

    #[test]
    fn an_f_lambda_spectrum_integrates_over_wavelength() {
        let dir = tempfile::tempdir().unwrap();
        let path = h_alpha_air_document(&dir);
        let value = measure_line_json(&path, &file_source(), &h_alpha_request()).unwrap();
        let (flux, flux_err) = (number(&value, "flux"), number(&value, "flux_err"));
        assert_relative("flux_cgs", number(&value, "flux_cgs"), flux * 1e4, 1e-12);
        assert_relative("flux_si", number(&value, "flux_si"), flux * 10.0, 1e-12);
        assert_relative("flux_cgs_err", number(&value, "flux_cgs_err"), flux_err * 1e4, 1e-12);
        assert_eq!(value[KEY_PHYSICAL_BASIS], "F_lambda in erg/cm2/s/A integrated over wavelength");
    }

    #[test]
    fn an_f_lambda_cube_on_an_air_axis_is_integrated_over_air_wavelengths() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_with_cards(&dir, "f_lambda_air.fits", &[("CTYPE3", "'AWAV'"), ("BUNIT", "'erg/cm2/s/A'")]);
        let value = measured(&path, &centre_pixel());
        let air = LazyCube::open(&path).unwrap().spectral_axis().unwrap().values;
        let vacuum: Vec<f64> = air.iter().map(|&w| air_to_vacuum_um(w)).collect();
        let centre = LINE_CENTRE_CHANNEL as usize;
        let air_over_vacuum_width = channel_widths(&air)[centre] / channel_widths(&vacuum)[centre];
        assert!((air_over_vacuum_width - 0.9997290).abs() < 1e-7, "air over vacuum width {air_over_vacuum_width:?}");
        let (flux, flux_cgs) = (number(&value, "flux"), number(&value, "flux_cgs"));
        let over_vacuum_widths = flux * 1e4;
        assert!(
            (flux_cgs / over_vacuum_widths - air_over_vacuum_width).abs() < 1e-7,
            "flux_cgs {flux_cgs:?} over {over_vacuum_widths:?}, widths {air_over_vacuum_width:?}"
        );
        assert!(
            (flux_cgs - over_vacuum_widths).abs() > 1e-4 * flux_cgs,
            "flux_cgs {flux_cgs:?} was integrated over the vacuum widths: {over_vacuum_widths:?}"
        );
        assert_relative("flux_si", number(&value, "flux_si"), flux_cgs / 1000.0, 1e-12);
        let (flux_err, flux_cgs_err) = (number(&value, "flux_err"), number(&value, "flux_cgs_err"));
        assert!(flux_err > 0.0, "flux_err {flux_err:?}");
        assert!(
            (flux_cgs_err / (flux_err * 1e4) - air_over_vacuum_width).abs() < 1e-7,
            "flux_cgs_err {flux_cgs_err:?} over {:?}, widths {air_over_vacuum_width:?}",
            flux_err * 1e4
        );
        let basis = value[KEY_PHYSICAL_BASIS].as_str().unwrap();
        assert!(
            basis.starts_with("F_lambda in erg/cm2/s/A integrated over air wavelengths (axis converted with ")
                && basis.ends_with(" for the measurement)"),
            "{basis}"
        );
        let per_beam_twin = write_with_cards(&dir, "air.fits", &[("CTYPE3", "'AWAV'")]);
        assert_eq!(flux.to_bits(), number(&measured(&per_beam_twin, &centre_pixel()), "flux").to_bits());
        assert_eq!(value["flux_unit"], "erg/cm2/s/A x um");
    }

    #[test]
    fn an_f_nu_spectrum_on_an_air_axis_is_integrated_on_vacuum_wavelengths() {
        let dir = tempfile::tempdir().unwrap();
        let flux = h_alpha_air_flux();
        let cards = iraf_linear_cards(H_ALPHA_START_ANGSTROM, H_ALPHA_STEP_ANGSTROM, "Jy");
        let air = write_wcs1d(dir.path(), "fnu_air.fits", &cards, &flux).to_str().unwrap().to_string();
        let vacuum_axis: Vec<f64> = (0..H_ALPHA_CHANNELS)
            .map(|i| air_to_vacuum_um((H_ALPHA_START_ANGSTROM + H_ALPHA_STEP_ANGSTROM * i as f64) * 1e-4))
            .collect();
        let n = H_ALPHA_CHANNELS;
        let vacuum = write_document(&dir, "fnu_vacuum_x1d.fits", &vacuum_axis, &flux, &vec![DOCUMENT_ERR; n], &vec![0; n]);
        let scatter_path = MeasureOptions { use_err: false, ..MeasureOptions::default() };
        let on_air = measure_line_json(&air, &file_source(), &h_alpha_request()).unwrap();
        let on_vacuum = measure_line_json_with(&vacuum, &file_source(), &h_alpha_request(), &scatter_path).unwrap();
        assert_eq!(on_air[KEY_WAVELENGTH_FRAME], FRAME_AIR);
        assert_eq!(on_vacuum[KEY_WAVELENGTH_FRAME], FRAME_VACUUM);
        assert_relative("flux_si on the air axis against its vacuum twin", number(&on_air, "flux_si"), number(&on_vacuum, "flux_si"), 1e-8);
        assert_eq!(on_vacuum[KEY_PHYSICAL_BASIS], F_NU_PER_CHANNEL_BASIS);
        let basis = on_air[KEY_PHYSICAL_BASIS].as_str().unwrap();
        assert!(basis.ends_with("on vacuum wavelengths (air axis converted with Morton (2000, ApJS 130, 403))"), "{basis}");
    }

    #[test]
    fn a_cube_in_jy_per_beam_or_one_spaxel_of_a_surface_brightness_cube_has_no_physical_flux() {
        let dir = tempfile::tempdir().unwrap();
        let per_beam = write(&dir, "line.fits", 0.0);
        for source in [centre_pixel(), disk_source()] {
            assert_no_physical_flux(
                &measured(&per_beam, &source),
                "no physical flux: Jy/beam is a brightness per beam and this version does not read the beam area",
            );
        }
        let surface_brightness = write_with_cards(&dir, "sb.fits", &SURFACE_BRIGHTNESS_CARDS);
        assert_no_physical_flux(
            &measured(&surface_brightness, &centre_pixel()),
            "no physical flux: one spaxel in MJy/sr is an intensity, not a flux; measure a region",
        );
    }

    #[test]
    fn a_region_of_a_surface_brightness_cube_uses_the_pixel_solid_angle() {
        let dir = tempfile::tempdir().unwrap();
        let surface_brightness = measured(&write_with_cards(&dir, "sb.fits", &SURFACE_BRIGHTNESS_CARDS), &disk_source());
        let in_jy = write_with_cards(&dir, "jy.fits", &[("BUNIT", "'Jy'")]);
        let summed_jy = measured(&in_jy, &disk_source());
        for key in ["flux_si", "flux_si_err"] {
            assert_relative(key, number(&surface_brightness, key) / number(&summed_jy, key), JY_PER_SUMMED_MJY_PER_SR, 1e-9);
        }
        assert_eq!(
            surface_brightness[KEY_PHYSICAL_BASIS],
            "MJy/sr summed over the region with a pixel solid angle of 3.0462e-12 sr, then F_nu integrated per channel with c / lambda^2"
        );
        assert_eq!(surface_brightness["flux_unit"], "MJy/sr x pix x um");
        assert_eq!(summed_jy[KEY_PHYSICAL_BASIS], F_NU_PER_CHANNEL_BASIS);
        let spaxel = measured(&in_jy, &centre_pixel());
        assert_relative("flux_si of the centre spaxel", number(&spaxel, "flux_si"), CENTRE_PIXEL_FLUX_SI, 1e-6);
        assert_eq!(spaxel[KEY_PHYSICAL_BASIS], F_NU_PER_CHANNEL_BASIS);
    }

    #[test]
    fn a_frequency_axis_in_jy_integrates_over_frequency() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_with_cards(&dir, "frequency.fits", &[("CTYPE3", "'FREQ'"), ("CUNIT3", "'GHz'"), ("BUNIT", "'Jy'")]);
        let value = measured(&path, &centre_pixel());
        assert_eq!(value["flux_unit"], "Jy x GHz");
        assert_relative("flux_si", number(&value, "flux_si"), number(&value, "flux") * 1e-17, 1e-12);
        assert_eq!(value[KEY_PHYSICAL_BASIS], "F_nu in Jy integrated over frequency");
    }

    #[test]
    fn a_spectrum_without_a_unit_with_a_count_rate_or_with_the_soss_label_says_why_there_is_no_physical_flux() {
        let dir = tempfile::tempdir().unwrap();
        let unitless = write_with_cards(&dir, "unitless.fits", &[("BUNIT", "''")]);
        assert_no_physical_flux(&measured(&unitless, &centre_pixel()), "no physical flux: the spectrum has no flux unit");
        let count_rate = write_with_cards(&dir, "rate.fits", &[("BUNIT", "'DN/s'")]);
        assert_no_physical_flux(
            &measured(&count_rate, &centre_pixel()),
            "no physical flux: DN/s is a count rate without a flux calibration",
        );
        let soss_label = measured(&write_with_cards(&dir, "soss.fits", &[("BUNIT", "'MJy'")]), &centre_pixel());
        assert_no_physical_flux(
            &soss_label,
            "no physical flux: MJy is the label JWST NIRISS SOSS spectra carry on values that are not on a MJy scale, so the unit cannot be trusted",
        );
        assert_eq!(soss_label["flux_unit"], "MJy x um");
        let in_mjy = measured(&write_with_cards(&dir, "millijansky.fits", &[("BUNIT", "'mJy'")]), &centre_pixel());
        let in_jy = measured(&write_with_cards(&dir, "jy.fits", &[("BUNIT", "'Jy'")]), &centre_pixel());
        assert_relative("flux_si in mJy over Jy", number(&in_mjy, "flux_si") / number(&in_jy, "flux_si"), 1e-3, 1e-9);
        assert_eq!(in_mjy[KEY_PHYSICAL_BASIS], "F_nu in mJy integrated per channel with c / lambda^2");
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
            redshift: None,
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
    fn real_g395h_br_alpha_line_flux_in_watts_per_square_metre() {
        let path = g395h_path();
        let request = g395h_request(BR_ALPHA_LINE, BR_ALPHA_CONTINUUM, LineModel::None);
        let scatter_path = MeasureOptions { use_err: false, ..MeasureOptions::default() };
        let cases = [
            ("with FLUX_ERROR", MeasureOptions::default(), [4.2943156309121404e-4, 7.840006575608761e-17, 3.798092195963517e-18]),
            ("without ERR", scatter_path, [4.2495643550283636e-4, 7.758325622903776e-17, 2.656171471621122e-18]),
        ];
        let quantities = [("flux", "Jy um"), ("flux_si", "W m^-2"), ("flux_si_err", "W m^-2")];
        let mut failures = Vec::new();
        for (label, options, oracle) in cases {
            let value = measure_line_json_with(&path, &file_source(), &request, &options).unwrap();
            for ((key, unit), expected) in quantities.into_iter().zip(oracle) {
                let found = value[key].as_f64().unwrap_or(f64::NAN);
                println!("{label}: {key} {found:?} {unit}, oracle {expected:?} {unit}");
                if !((found - expected).abs() <= 1e-9 * expected) {
                    failures.push(format!("{label}: {key} {found:?} {unit}, oracle {expected:?} {unit}"));
                }
            }
            println!("{label}: flux_cgs {} +- {} erg s^-1 cm^-2", value["flux_cgs"], value["flux_cgs_err"]);
            println!("{label}: {}", value[KEY_PHYSICAL_BASIS]);
            if value[KEY_PHYSICAL_BASIS] != F_NU_PER_CHANNEL_BASIS {
                failures.push(format!("{label}: basis {}", value[KEY_PHYSICAL_BASIS]));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
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

    const SERIES_DETECTOR: &str = "NRS1";
    const SERIES_CHANNELS: usize = 40;
    const SERIES_INTEGRATIONS: usize = 5;
    const SERIES_ERR: f64 = 0.05;
    const SERIES_MEDIAN_INTEGRATION: usize = 2;
    const FIRST_INTEGRATION_FLUX: f64 = 7.519612589399947e-3;
    const MEDIAN_OVER_FIRST_INTEGRATION: f64 = 1.02;
    const PICKED_FLUX_TOLERANCE: f64 = 1e-6;
    const SAME_ARRAYS_TOLERANCE: f64 = 1e-12;
    const SAME_ARRAYS_KEYS: [&str; 17] = [
        "flux",
        "flux_err",
        "equivalent_width",
        "centroid",
        "sigma",
        "fwhm",
        "snr",
        "continuum_level",
        "continuum_slope",
        "continuum_sigma",
        "continuum_chi2_red",
        "equivalent_width_err",
        "centroid_err",
        "sigma_err",
        "fwhm_err",
        "flux_si",
        "flux_si_err",
    ];
    const SERIES_ERR_SOURCE: &str = "FLUX_ERROR of the series pick";
    const MEDIAN_ERROR_CONVENTION: &str = "error of the median = 1.2533 x that";
    const PIPELINE_NOTE_START: &str = "EXTRACT1D table";

    fn series_axis() -> Vec<f64> {
        (0..SERIES_CHANNELS).map(|z| 1.0 + 0.001 * z as f64).collect()
    }

    fn series_flux(integration: usize) -> Vec<f64> {
        (0..SERIES_CHANNELS)
            .map(|z| {
                let offset = z as f64 - 20.0;
                (1.0 + (-offset * offset / 18.0).exp()) * (1.0 + 0.01 * integration as f64)
            })
            .collect()
    }

    fn series_rows() -> Vec<SynthRow> {
        (0..SERIES_INTEGRATIONS)
            .map(|integration| SynthRow {
                wavelength: series_axis(),
                flux: series_flux(integration),
                flux_error: vec![SERIES_ERR; SERIES_CHANNELS],
                ..ramp_row(integration as i32 + 1, 60000.0 + 0.01 * integration as f64, SERIES_CHANNELS)
            })
            .collect()
    }

    fn write_series_rows(dir: &tempfile::TempDir, name: &str, primary: &[(&'static str, String)], rows: Vec<SynthRow>) -> String {
        let table = SynthTable { extver: 1, detector: Some(SERIES_DETECTOR), sporder: Some(0), flux_unit: Some("Jy"), rows };
        write_series_file(dir.path(), name, series_primary(DATAMODEL_ROWS, primary), vec![rows_hdu(&table)])
            .to_str()
            .unwrap()
            .to_string()
    }

    fn write_series(dir: &tempfile::TempDir) -> String {
        write_series_rows(dir, "series_x1dints.fits", &[], series_rows())
    }

    fn series_key(detector: &str) -> SeriesKey {
        SeriesKey { detector: detector.to_string(), sporder: Some(0) }
    }

    fn series_pick(path: &str, detector: &str, pick: TsoPick) -> SpectrumSource {
        SpectrumSource::Series { paths: vec![path.to_string()], detector: detector.to_string(), sporder: Some(0), pick }
    }

    fn measured_pick(path: &str, pick: TsoPick) -> serde_json::Value {
        measured(path, &series_pick(path, SERIES_DETECTOR, pick))
    }

    #[test]
    fn a_series_source_deserializes_from_the_frontend_shape() {
        let median: SpectrumSource = serde_json::from_str(
            r#"{"kind":"series","paths":["a.fits"],"detector":"NRS1","sporder":0,"pick":{"kind":"median"}}"#,
        )
        .unwrap();
        assert_eq!(median, series_pick("a.fits", "NRS1", TsoPick::Median));
        let without_order: SpectrumSource = serde_json::from_str(
            r#"{"kind":"series","paths":["a.fits","b.fits"],"detector":"MIRIMAGE","pick":{"kind":"integration","index":3}}"#,
        )
        .unwrap();
        assert_eq!(
            without_order,
            SpectrumSource::Series {
                paths: vec!["a.fits".to_string(), "b.fits".to_string()],
                detector: "MIRIMAGE".to_string(),
                sporder: None,
                pick: TsoPick::Integration { index: 3 },
            }
        );
        let unknown_pick = r#"{"kind":"series","paths":["a.fits"],"detector":"NRS1","sporder":0,"pick":{"kind":"mode"}}"#;
        assert!(serde_json::from_str::<SpectrumSource>(unknown_pick).is_err());
        assert_eq!(
            serde_json::to_value(series_pick("a.fits", "NRS1", TsoPick::Mean { first: 0, last: 4 })).unwrap(),
            serde_json::json!({
                "kind": "series",
                "paths": ["a.fits"],
                "detector": "NRS1",
                "sporder": 0,
                "pick": { "kind": "mean", "first": 0, "last": 4 },
            })
        );
    }

    #[test]
    fn the_median_pick_of_a_series_is_measured_like_the_same_arrays_in_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let series = write_series(&dir);
        let on_series = measured_pick(&series, TsoPick::Median);
        let median: Vec<f32> = series_flux(SERIES_MEDIAN_INTEGRATION).iter().map(|&v| v as f32).collect();
        let median_err = MEDIAN_ERROR_FACTOR * SERIES_ERR / (SERIES_INTEGRATIONS as f64).sqrt();
        let document = write_document(
            &dir,
            "median_x1d.fits",
            &series_axis(),
            &median,
            &vec![median_err; SERIES_CHANNELS],
            &vec![0; SERIES_CHANNELS],
        );
        let on_file = measured(&document, &file_source());
        for key in SAME_ARRAYS_KEYS {
            assert_relative(key, number(&on_series, key), number(&on_file, key), SAME_ARRAYS_TOLERANCE);
        }
        assert_relative(
            "flux of the median pick",
            number(&on_series, "flux"),
            MEDIAN_OVER_FIRST_INTEGRATION * FIRST_INTEGRATION_FLUX,
            PICKED_FLUX_TOLERANCE,
        );
        assert_eq!(on_series["err_used"], true);
        assert_eq!(on_file[RES_SOURCE]["kind"], "file");
    }

    #[test]
    fn one_integration_and_a_mean_are_picked_by_their_indices() {
        let dir = tempfile::tempdir().unwrap();
        let series = write_series(&dir);
        let first = measured_pick(&series, TsoPick::Integration { index: 0 });
        assert_relative("flux of integration 1", number(&first, "flux"), FIRST_INTEGRATION_FLUX, PICKED_FLUX_TOLERANCE);
        assert_eq!(notes_of(&first)[0], "time series NRS1, integration 1: vacuum wavelengths");
        assert_eq!(first[RES_SOURCE]["pick"], serde_json::json!({ "kind": "integration", "index": 0 }));

        let mean = measured_pick(&series, TsoPick::Mean { first: 0, last: 4 });
        assert_relative(
            "flux of the mean of integrations 1 to 5",
            number(&mean, "flux"),
            MEDIAN_OVER_FIRST_INTEGRATION * FIRST_INTEGRATION_FLUX,
            PICKED_FLUX_TOLERANCE,
        );
        assert_eq!(notes_of(&mean)[0], "time series NRS1, mean of integrations 1 to 5: vacuum wavelengths");
        assert_eq!(mean[RES_SOURCE]["pick"], serde_json::json!({ "kind": "mean", "first": 0, "last": 4 }));

        let last = measured_pick(&series, TsoPick::Integration { index: 4 });
        assert_relative(
            "flux of integration 5",
            number(&last, "flux"),
            1.04 * FIRST_INTEGRATION_FLUX,
            PICKED_FLUX_TOLERANCE,
        );
        assert_eq!(notes_of(&last)[0], "time series NRS1, integration 5: vacuum wavelengths");
        let beyond = measure_line_json(
            &series,
            &series_pick(&series, SERIES_DETECTOR, TsoPick::Integration { index: 5 }),
            &request(LineModel::None),
        )
        .unwrap_err()
        .to_string();
        assert_eq!(beyond, "integration index 5 is out of range: the series NRS1 has 5 integrations (0 to 4)");
    }

    fn assert_notes_hold_in_order(notes: &[String], expected: &[String]) {
        let mut searched_from = 0;
        for sentence in expected {
            let found = notes[searched_from..]
                .iter()
                .position(|note| note == sentence)
                .unwrap_or_else(|| panic!("'{sentence}' is not in the notes after position {searched_from}: {notes:?}"));
            searched_from += found + 1;
        }
    }

    #[test]
    fn the_series_source_is_echoed_and_labelled() {
        let dir = tempfile::tempdir().unwrap();
        let series = write_series(&dir);
        let median = measured_pick(&series, TsoPick::Median);
        assert_eq!(
            median[RES_SOURCE],
            serde_json::json!({
                "kind": "series",
                "paths": [series.as_str()],
                "detector": "NRS1",
                "sporder": 0,
                "pick": { "kind": "median" },
            })
        );
        assert_eq!(median[KEY_ERR_SOURCE], SERIES_ERR_SOURCE);
        assert_eq!(median[KEY_WAVELENGTH_FRAME], FRAME_VACUUM);
        assert_eq!(median[KEY_PHYSICAL_BASIS], F_NU_PER_CHANNEL_BASIS);
        assert_eq!(median["flux_unit"], "Jy x um");
        assert_eq!(median["axis_unit"], "um");
        let notes = notes_of(&median);
        assert_eq!(notes[0], "time series NRS1, median of 5 integrations: vacuum wavelengths");
        assert!(notes[1].contains(MEDIAN_ERROR_CONVENTION), "{notes:?}");
        assert!(!notes.iter().any(|note| note.starts_with(PIPELINE_NOTE_START)), "{notes:?}");
        assert!(!notes.iter().any(|note| note.starts_with("time series NRS1:")), "{notes:?}");
        let set = open_tso(std::slice::from_ref(&series)).unwrap();
        let warnings = &find_series(&set, &series_key(SERIES_DETECTOR)).unwrap().warnings;
        assert_eq!(warnings.len(), 0, "{warnings:?}");
        assert_notes_hold_in_order(&notes, warnings);

        let scatter_path = MeasureOptions { use_err: false, ..MeasureOptions::default() };
        let source = series_pick(&series, SERIES_DETECTOR, TsoPick::Median);
        let without_err = measure_line_json_with(&series, &source, &request(LineModel::None), &scatter_path).unwrap();
        assert_eq!(without_err[KEY_ERR_SOURCE], ERR_SOURCE_NONE);
        assert_eq!(without_err["err_used"], false);
        assert!(notes_of(&without_err).iter().any(|note| note.contains("use_err = false")), "{:?}", notes_of(&without_err));

        let another_path = measure_line_json("not_read.fits", &source, &request(LineModel::None)).unwrap();
        assert_eq!(without_elapsed(another_path), without_elapsed(median));

        let air = MeasureOptions { wavelength_frame: Some(WavelengthFrame::Air), ..MeasureOptions::default() };
        let read_as_air = measure_line_json_with(&series, &source, &request(LineModel::None), &air).unwrap();
        assert_eq!(read_as_air[KEY_WAVELENGTH_FRAME], FRAME_AIR);
        assert!(
            notes_of(&read_as_air)[0].starts_with("time series NRS1, median of 5 integrations: air wavelengths set by the request"),
            "{:?}",
            notes_of(&read_as_air)
        );

        let first = measured_pick(&series, TsoPick::Integration { index: 0 });
        assert!(!notes_of(&first).iter().any(|note| note.contains("error of the median")), "{:?}", notes_of(&first));
        assert_eq!(first[KEY_ERR_SOURCE], SERIES_ERR_SOURCE);
    }

    #[test]
    fn the_warnings_of_the_series_follow_the_notes_of_the_pick() {
        let dir = tempfile::tempdir().unwrap();
        let untimed: Vec<SynthRow> =
            series_rows().into_iter().map(|row| SynthRow { mjd_avg: f64::NAN, tdb_mid: 60000.5, ..row }).collect();
        let series = write_series_rows(&dir, "untimed_x1dints.fits", &[("NINTS", "8".to_string())], untimed);
        let set = open_tso(std::slice::from_ref(&series)).unwrap();
        let warnings = find_series(&set, &series_key(SERIES_DETECTOR)).unwrap().warnings.clone();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].starts_with("no usable integration times"), "{warnings:?}");
        assert!(warnings[1].contains("5 of 8 integrations found"), "{warnings:?}");

        let notes = notes_of(&measured_pick(&series, TsoPick::Median));
        assert_eq!(notes[0], "time series NRS1, median of 5 integrations: vacuum wavelengths");
        assert!(notes[1].contains(MEDIAN_ERROR_CONVENTION), "{notes:?}");
        assert_eq!(notes[2..4], warnings[..], "{notes:?}");

        let notes = notes_of(&measured_pick(&series, TsoPick::Integration { index: 0 }));
        assert_eq!(notes[0], "time series NRS1, integration 1: vacuum wavelengths");
        assert_eq!(notes[1..3], warnings[..], "{notes:?}");
    }

    #[test]
    fn a_series_that_is_not_in_the_files_is_refused_with_the_ones_found() {
        let dir = tempfile::tempdir().unwrap();
        let series = write_series(&dir);
        let refused = measure_line_json(&series, &series_pick(&series, "NRS2", TsoPick::Median), &request(LineModel::None))
            .unwrap_err()
            .to_string();
        assert_eq!(refused, "no series NRS2, order 0 in these files; series found: NRS1");
        assert_eq!(refused, find_series(&open_tso(&[series]).unwrap(), &series_key("NRS2")).unwrap_err().to_string());
    }

    const A5_X1DINTS: &str = "tso/A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits";
    const A5_LINE: (usize, usize) = (1170, 1321);
    const A5_WINDOWS: ContinuumWindows = ((1110, 1149), (1342, 1381));
    const A5_ORACLE_TOLERANCE: f64 = 1e-6;
    const A5_ORACLE: [(&str, f64, &str); 5] = [
        ("flux", 1.0416364540562062e-5, "Jy um"),
        ("flux_err", 2.973799047632048e-6, "Jy um"),
        ("centroid", 4.661921647663075, "um"),
        ("continuum_chi2_red", 1365.084854130164, "(dimensionless)"),
        ("flux_si", 1.4369394397143645e-18, "W m^-2"),
    ];

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits"]
    fn real_a5_nrs2_median_pick_is_measured_like_its_arrays() {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join(A5_X1DINTS).to_str().unwrap().to_string();
        let key = SeriesKey { detector: "NRS2".to_string(), sporder: Some(-1) };
        let source = SpectrumSource::Series {
            paths: vec![path.clone()],
            detector: key.detector.clone(),
            sporder: key.sporder,
            pick: TsoPick::Median,
        };
        let request = g395h_request(A5_LINE, A5_WINDOWS, LineModel::None);
        let value = measure_line_json(&path, &source, &request).unwrap();
        for note in notes_of(&value) {
            println!("note: {note}");
        }
        println!("err_source: {}, basis: {}", value[KEY_ERR_SOURCE], value[KEY_PHYSICAL_BASIS]);

        let set = open_tso(std::slice::from_ref(&path)).unwrap();
        let doc = series_spectrum(&set, &key, &TsoPick::Median).unwrap();
        let mut step_notes = Vec::new();
        let flux = measured_flux(&doc.flux, &mut step_notes);
        let err = document_errors(&doc, true, &mut step_notes);
        let excluded = document_exclusions(&doc, true, &mut step_notes);
        let (channel_factor, basis) = physical_flux_factor(
            doc.flux_unit.as_deref(),
            &document_measured_axis(doc.wavelength_frame, doc.axis.kind),
            &doc.axis.values,
            &FluxSource::File,
        );
        assert_eq!(doc.flux_unit.as_deref(), Some("Jy"));
        assert_eq!(basis, F_NU_PER_CHANNEL_BASIS);
        let direct = measure_line_physical(
            &doc.axis.values,
            &flux,
            err,
            excluded.as_deref(),
            A5_LINE,
            Some(A5_WINDOWS),
            LineModel::None,
            channel_factor.as_deref(),
        )
        .unwrap();
        let direct = serde_json::to_value(&direct).unwrap();

        let mut failures = Vec::new();
        for key in SAME_ARRAYS_KEYS {
            let (found, expected) = (number(&value, key), number(&direct, key));
            println!("{key}: command {found:?}, same arrays {expected:?}");
            if !((found - expected).abs() <= SAME_ARRAYS_TOLERANCE * expected.abs()) {
                failures.push(format!("{key}: command {found:?}, same arrays {expected:?}"));
            }
        }
        for (key, expected, unit) in A5_ORACLE {
            let found = number(&value, key);
            let difference = (found - expected).abs() / expected.abs();
            println!("{key}: measured {found:?} {unit}, oracle {expected:?} {unit}, relative difference {difference:.1e}");
            if !(difference <= A5_ORACLE_TOLERANCE) {
                failures.push(format!("{key}: measured {found:?} {unit}, oracle {expected:?} {unit}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
