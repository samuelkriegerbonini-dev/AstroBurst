use anyhow::{bail, Result};
use ndarray::Array2;
use serde::Serialize;

use crate::core::astrometry::spectral::{
    air_formula_applies, air_refractive_index, air_to_vacuum_um, velocity_kms, wavelength_um_from_frequency_ghz,
    AxisKind, SpectralAxis, VelocityConvention, SPEED_OF_LIGHT_KMS,
};
use crate::core::astrometry::wcs::{position_angle_deg, WcsTransform};
use crate::core::cube::lazy::LazyCube;
use crate::math::exact_median_f64;

pub const MAX_PV_OFFSETS: usize = 8192;
pub const MAX_PV_ACROSS: usize = 256;
pub const MAX_PV_CELLS: usize = 64 << 20;
pub const MAX_PV_SAMPLES: usize = 1 << 28;
pub const PV_REST_REQUIRED: &str =
    "rest wavelength required for a velocity axis: set rest_um or add RESTWAV/RESTFRQ to the header";
pub const OFFSET_UNIT_ARCSEC: &str = "arcsec";
pub const OFFSET_UNIT_PIXEL: &str = "pixel";
pub const CHANNEL_UNIT: &str = "ch";

const PV_BAND_BYTES: usize = 64 << 20;
const PV_MAX_CHANNELS_PER_BATCH: usize = 32;
const COUNT_EPSILON: f64 = 1e-9;
const ARCSEC_PER_DEG: f64 = 3600.0;
const UNIT_UM: &str = "um";
const UNIT_GHZ: &str = "GHz";
const UNIT_KMS: &str = "km/s";
const HEADER_BUNIT: &str = "BUNIT";
const NOTE_NO_WCS: &str = "no celestial WCS: offsets in pixels";
const NOTE_DEGENERATE_CD: &str = "CD matrix gives no finite positive scale along the slit: offsets in pixels";
const NOTE_SIP: &str = "SIP distortion in the header is ignored: offsets use the linear CD scale at the reference pixel";
const NOTE_DQ: &str = "DQ flags are not applied: the PV samples the science array only";
const NOTE_ZEROS: &str = "only non-finite samples are skipped: exact zeros count as flux (a zero-padded edge is sampled as 0)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvSpectralMode {
    WavelengthVac,
    WavelengthAir,
    Frequency,
    Velocity,
}

impl PvSpectralMode {
    pub fn parse(name: &str) -> std::result::Result<Self, String> {
        match name.trim().to_lowercase().as_str() {
            "wavelength_vac" => Ok(PvSpectralMode::WavelengthVac),
            "wavelength_air" => Ok(PvSpectralMode::WavelengthAir),
            "frequency" => Ok(PvSpectralMode::Frequency),
            "velocity" => Ok(PvSpectralMode::Velocity),
            other => Err(format!(
                "unknown spectral mode '{}': use wavelength_vac, wavelength_air, frequency or velocity",
                other
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PvSpectralMode::WavelengthVac => "wavelength_vac",
            PvSpectralMode::WavelengthAir => "wavelength_air",
            PvSpectralMode::Frequency => "frequency",
            PvSpectralMode::Velocity => "velocity",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PvConfig {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub z0: usize,
    pub z1: usize,
    pub step_px: f64,
    pub width_px: f64,
    pub mode: PvSpectralMode,
    pub rest_um: Option<f64>,
    pub convention: VelocityConvention,
    pub velocity_shift_kms: f64,
}

#[derive(Debug, Clone)]
pub struct PvNativeAxis {
    pub ctype: Option<String>,
    pub cunit: Option<String>,
    pub crval: f64,
    pub cdelt: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PvSummary {
    pub n_offsets: usize,
    pub n_channels: usize,
    pub n_across: usize,
    pub n_off_image: usize,
    pub offset_step: f64,
    pub offset_unit: &'static str,
    pub slit_length_px: f64,
    pub slit_length: f64,
    pub slit_pa_deg: Option<f64>,
    pub pixel_scale_arcsec: Option<f64>,
    pub ridge_gradient: Option<f64>,
    pub ridge_gradient_unit: String,
    pub ridge_span: Option<f64>,
    pub n_ridge_valid: usize,
    pub peak_value: Option<f64>,
    pub peak_offset: Option<f64>,
    pub peak_spectral: Option<f64>,
    pub bunit: Option<String>,
}

#[derive(Debug)]
pub struct PvDiagram {
    pub data: Array2<f32>,
    pub offsets: Vec<f64>,
    pub offset_unit: &'static str,
    pub offset_step: f64,
    pub spectral_values: Vec<f64>,
    pub spectral_unit: &'static str,
    pub convention_applies: bool,
    pub rest_um: Option<f64>,
    pub native: PvNativeAxis,
    pub ridge: Vec<f64>,
    pub ridge_channel: Vec<f64>,
    pub peak_channel: Vec<Option<usize>>,
    pub peak_value: Vec<f64>,
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub n_across: usize,
    pub summary: PvSummary,
    pub notes: Vec<String>,
}

struct SpectralResolution {
    values: Vec<f64>,
    unit: &'static str,
    convention_applies: bool,
    rest_um: Option<f64>,
    native: PvNativeAxis,
    notes: Vec<String>,
}

fn channel_axis(z0: usize, n_channels: usize, reason: &str, mut notes: Vec<String>) -> SpectralResolution {
    notes.push(format!("no spectral WCS ({}): the PV spectral axis is the channel index", reason));
    SpectralResolution {
        values: (0..n_channels).map(|k| (z0 + k) as f64).collect(),
        unit: CHANNEL_UNIT,
        convention_applies: false,
        rest_um: None,
        native: PvNativeAxis { ctype: None, cunit: None, crval: z0 as f64, cdelt: 1.0 },
        notes,
    }
}

fn native_axis(axis: &SpectralAxis, z0: usize) -> PvNativeAxis {
    let scale = if axis.header_scale.is_finite() && axis.header_scale != 0.0 { axis.header_scale } else { 1.0 };
    let cunit = if axis.header_unit.is_empty() { axis.kind.fits_default_unit().to_string() } else { axis.header_unit.clone() };
    PvNativeAxis {
        ctype: Some(axis.ctype.clone()),
        cunit: Some(cunit),
        crval: axis.values.get(z0).map(|v| v / scale).unwrap_or(f64::NAN),
        cdelt: axis.cdelt / scale,
    }
}

fn frame_description(axis: &SpectralAxis) -> String {
    match axis.specsys.as_deref() {
        Some(frame) => format!("axis frame {}", frame),
        None => "axis frame, which the header does not state (no SPECSYS)".to_string(),
    }
}

fn resolve_spectral(cube: &LazyCube, cfg: &PvConfig, n_channels: usize) -> Result<SpectralResolution> {
    let axis = match cube.spectral_axis() {
        Ok(axis) => axis,
        Err(reason) => {
            if cfg.mode == PvSpectralMode::Velocity {
                bail!("no spectral WCS ({}): a velocity PV needs a wavelength, frequency or velocity axis", reason);
            }
            return Ok(channel_axis(cfg.z0, n_channels, &reason, Vec::new()));
        }
    };
    let mut notes = axis.notes.clone();
    let stored: Vec<f64> = axis.values.get(cfg.z0..=cfg.z1).map(<[f64]>::to_vec).unwrap_or_default();
    if stored.len() != n_channels {
        bail!("spectral axis has {} values but the cube has {} channels", axis.values.len(), n_channels);
    }
    let (values, unit, convention_applies, rest_um) = match axis.kind {
        AxisKind::Unknown => {
            if cfg.mode == PvSpectralMode::Velocity {
                bail!(
                    "CTYPE3 '{}' is not a spectral axis: a velocity PV needs a wavelength, frequency or velocity axis",
                    axis.ctype
                );
            }
            let reason = if axis.ctype.is_empty() {
                "CTYPE3 missing".to_string()
            } else {
                format!("CTYPE3 '{}' is not a spectral axis", axis.ctype)
            };
            return Ok(channel_axis(cfg.z0, n_channels, &reason, notes));
        }
        AxisKind::Vrad | AxisKind::Vopt | AxisKind::Velo => {
            notes.push(format!(
                "velocity axis CTYPE3 '{}' returned as stored in the header's convention: the panel convention is ignored",
                axis.ctype
            ));
            (stored, UNIT_KMS, false, None)
        }
        AxisKind::Zopt => (
            stored.iter().map(|&z| velocity_kms(1.0 + z, 1.0, cfg.convention)).collect(),
            UNIT_KMS,
            true,
            None,
        ),
        AxisKind::Wave | AxisKind::Awav | AxisKind::Freq => {
            let vacuum: Vec<f64> = match axis.kind {
                AxisKind::Awav => stored.iter().map(|&w| air_to_vacuum_um(w)).collect(),
                AxisKind::Freq => stored.iter().map(|&f| wavelength_um_from_frequency_ghz(f)).collect(),
                _ => stored.clone(),
            };
            match cfg.mode {
                PvSpectralMode::WavelengthVac => (vacuum, UNIT_UM, false, None),
                PvSpectralMode::WavelengthAir => {
                    let air = if axis.kind == AxisKind::Awav {
                        stored
                    } else {
                        vacuum
                            .iter()
                            .map(|&l| if air_formula_applies(l) { l / air_refractive_index(l) } else { l })
                            .collect()
                    };
                    (air, UNIT_UM, false, None)
                }
                PvSpectralMode::Frequency => {
                    let frequency = if axis.kind == AxisKind::Freq {
                        stored
                    } else {
                        vacuum.iter().map(|&l| SPEED_OF_LIGHT_KMS / l).collect()
                    };
                    (frequency, UNIT_GHZ, false, None)
                }
                PvSpectralMode::Velocity => {
                    let rest = match cfg.rest_um.filter(|r| r.is_finite() && *r > 0.0).or(axis.rest_wavelength_um) {
                        Some(rest) => rest,
                        None => bail!("{}", PV_REST_REQUIRED),
                    };
                    let velocities = vacuum.iter().map(|&l| velocity_kms(l, rest, cfg.convention)).collect();
                    (velocities, UNIT_KMS, true, Some(rest))
                }
            }
        }
    };
    let mut values = values;
    if cfg.mode == PvSpectralMode::Velocity {
        let frame = frame_description(&axis);
        let shift = cfg.velocity_shift_kms;
        if shift != 0.0 {
            for v in &mut values {
                *v += shift;
            }
            notes.push(format!(
                "PV velocities are in the {}, shifted by {:+.3} km/s (frame correction from the panel)",
                frame, shift
            ));
        } else {
            notes.push(format!(
                "PV velocities are in the {}: no barycentric or heliocentric correction is applied",
                frame
            ));
        }
    }
    Ok(SpectralResolution { values, unit, convention_applies, rest_um, native: native_axis(&axis, cfg.z0), notes })
}

pub fn row_band(y_min: f64, y_max: f64, naxis2: usize) -> (usize, usize) {
    if naxis2 < 2 {
        return (0, naxis2);
    }
    let top = (naxis2 - 1) as f64;
    let lo = if y_min.is_finite() { y_min.clamp(0.0, top) } else { 0.0 };
    let hi = if y_max.is_finite() { y_max.clamp(0.0, top) } else { top };
    let row_end = (hi.floor() as usize + 2).min(naxis2);
    let row_start = (lo.floor() as usize).min(row_end - 2);
    (row_start, row_end - row_start)
}

pub fn channels_per_batch(row_count: usize, naxis1: usize) -> usize {
    match row_count.checked_mul(naxis1).and_then(|n| n.checked_mul(std::mem::size_of::<f32>())) {
        Some(bytes) if bytes > 0 => (PV_BAND_BYTES / bytes).clamp(1, PV_MAX_CHANNELS_PER_BATCH),
        _ => 1,
    }
}

pub fn sample_budget(n_offsets: usize, n_across: usize, n_channels: usize) -> Result<usize> {
    match n_offsets.checked_mul(n_across).and_then(|n| n.checked_mul(n_channels)) {
        Some(n) if n <= MAX_PV_SAMPLES => Ok(n),
        _ => bail!(
            "PV of {} x {} x {} bilinear samples exceeds the {} allowed: reduce the width, raise the step or narrow the channel range",
            n_offsets,
            n_across,
            n_channels,
            MAX_PV_SAMPLES
        ),
    }
}

fn bilinear_base(c: f64, n: usize) -> Option<(usize, f64)> {
    if n < 2 || !c.is_finite() || c < 0.0 || c > (n - 1) as f64 {
        return None;
    }
    if c == (n - 1) as f64 {
        return Some((n - 2, 1.0));
    }
    let c0 = c.floor();
    Some((c0 as usize, c - c0))
}

pub fn bilinear_at(band: &[f32], rows: usize, cols: usize, x: f64, y: f64) -> Option<f64> {
    let (x0, fx) = bilinear_base(x, cols)?;
    let (y0, fy) = bilinear_base(y, rows)?;
    let mut corners = [0.0f64; 4];
    for (slot, (ux, uy)) in [(x0, y0), (x0 + 1, y0), (x0, y0 + 1), (x0 + 1, y0 + 1)].into_iter().enumerate() {
        let v = *band.get(uy.checked_mul(cols)?.checked_add(ux)?)?;
        if !v.is_finite() {
            return None;
        }
        corners[slot] = v as f64;
    }
    let top = corners[0] * (1.0 - fx) + corners[1] * fx;
    let bottom = corners[2] * (1.0 - fx) + corners[3] * fx;
    Some(top * (1.0 - fy) + bottom * fy)
}

pub fn median_subtracted_centroid(values: &[f64], axis: &[f64], z0: usize) -> (f64, f64) {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    let median = exact_median_f64(&finite);
    let (mut weight_sum, mut spectral_sum, mut channel_sum) = (0.0f64, 0.0f64, 0.0f64);
    for (k, (&v, &s)) in values.iter().zip(axis).enumerate() {
        if !v.is_finite() {
            continue;
        }
        let w = (v - median).max(0.0);
        weight_sum += w;
        spectral_sum += w * s;
        channel_sum += w * (z0 + k) as f64;
    }
    if weight_sum > 0.0 {
        (spectral_sum / weight_sum, channel_sum / weight_sum)
    } else {
        (f64::NAN, f64::NAN)
    }
}

fn least_squares_slope(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0usize, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in xs.iter().zip(ys) {
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        n += 1;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let denominator = nf * sxx - sx * sx;
    if !(denominator.is_finite() && denominator > 0.0) {
        return None;
    }
    let slope = (nf * sxy - sx * sy) / denominator;
    slope.is_finite().then_some(slope)
}

fn finite_option(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

fn card_text(cube: &LazyCube, key: &str) -> Option<String> {
    cube.header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

struct OffsetScale {
    step: f64,
    unit: &'static str,
    slit_pa_deg: Option<f64>,
    pixel_scale_arcsec: Option<f64>,
    notes: Vec<String>,
}

fn offset_scale(cube: &LazyCube, cfg: &PvConfig, along: (f64, f64)) -> OffsetScale {
    let Ok(wcs) = WcsTransform::from_header(&cube.header) else {
        return OffsetScale {
            step: cfg.step_px,
            unit: OFFSET_UNIT_PIXEL,
            slit_pa_deg: None,
            pixel_scale_arcsec: None,
            notes: vec![NOTE_NO_WCS.to_string()],
        };
    };
    let cd = wcs.raw_params().4;
    let scale_deg = (cd[0][0] * along.0 + cd[0][1] * along.1).hypot(cd[1][0] * along.0 + cd[1][1] * along.1);
    let step_arcsec = cfg.step_px * scale_deg * ARCSEC_PER_DEG;
    let start = wcs.pixel_to_world(cfg.x0, cfg.y0);
    let end = wcs.pixel_to_world(cfg.x1, cfg.y1);
    let slit_pa_deg = [start.ra, start.dec, end.ra, end.dec]
        .iter()
        .all(|v| v.is_finite())
        .then(|| position_angle_deg(start.ra, start.dec, end.ra, end.dec));
    let mut notes = Vec::new();
    let (step, unit) = if step_arcsec.is_finite() && step_arcsec > 0.0 {
        (step_arcsec, OFFSET_UNIT_ARCSEC)
    } else {
        notes.push(NOTE_DEGENERATE_CD.to_string());
        (cfg.step_px, OFFSET_UNIT_PIXEL)
    };
    if wcs.orientation(cube.geometry.naxis1, cube.geometry.naxis2).sip_present {
        notes.push(NOTE_SIP.to_string());
    }
    OffsetScale { step, unit, slit_pa_deg, pixel_scale_arcsec: finite_option(wcs.pixel_scale_arcsec()), notes }
}

pub fn pv_diagram(cube: &LazyCube, cfg: &PvConfig) -> Result<PvDiagram> {
    cube.check_channel_range(cfg.z0, cfg.z1)?;
    let (naxis1, naxis2) = (cube.geometry.naxis1, cube.geometry.naxis2);
    if naxis1 < 2 || naxis2 < 2 {
        bail!("cube spatial plane is {}x{}: bilinear sampling needs at least 2x2 pixels", naxis1, naxis2);
    }
    for (name, value) in [("x0", cfg.x0), ("y0", cfg.y0), ("x1", cfg.x1), ("y1", cfg.y1)] {
        if !value.is_finite() {
            bail!("slit endpoint {} must be finite", name);
        }
    }
    let length = (cfg.x1 - cfg.x0).hypot(cfg.y1 - cfg.y0);
    if !length.is_finite() {
        bail!("slit length is not finite");
    }
    if length <= 0.0 {
        bail!("slit has zero length: the two endpoints coincide");
    }
    let step = cfg.step_px;
    if !(step.is_finite() && step > 0.0) {
        bail!("step_px must be a positive finite number of pixels, got {}", step);
    }
    let width = cfg.width_px;
    if !(width.is_finite() && width >= 0.0) {
        bail!("width_px must be a finite number of pixels >= 0, got {}", width);
    }
    let n_offsets_f = (length / step + COUNT_EPSILON).floor() + 1.0;
    if n_offsets_f > MAX_PV_OFFSETS as f64 {
        bail!(
            "slit of {:.2} px at step {} px gives {:.0} offsets, more than the {} allowed",
            length,
            step,
            n_offsets_f,
            MAX_PV_OFFSETS
        );
    }
    let n_across_f = (width / step + COUNT_EPSILON).floor().max(1.0);
    if n_across_f > MAX_PV_ACROSS as f64 {
        bail!(
            "width {} px at step {} px gives {:.0} across samples, more than the {} allowed",
            width,
            step,
            n_across_f,
            MAX_PV_ACROSS
        );
    }
    let n_offsets = n_offsets_f as usize;
    let n_across = n_across_f as usize;
    if n_offsets < 2 {
        bail!("a PV needs at least 2 offsets, got {}: lengthen the slit or reduce step_px", n_offsets);
    }
    let n_channels = cfg.z1 - cfg.z0 + 1;
    if n_channels < 2 {
        bail!("a PV needs at least 2 channels, got z0..=z1 = {}..={}", cfg.z0, cfg.z1);
    }
    if n_offsets.checked_mul(n_channels).is_none_or(|cells| cells > MAX_PV_CELLS) {
        bail!("PV of {} x {} cells exceeds the {} allowed", n_offsets, n_channels, MAX_PV_CELLS);
    }
    sample_budget(n_offsets, n_across, n_channels)?;

    let midpoint = ((cfg.x0 + cfg.x1) / 2.0, (cfg.y0 + cfg.y1) / 2.0);
    let along = ((cfg.x1 - cfg.x0) / length, (cfg.y1 - cfg.y0) / length);
    let across = (-along.1, along.0);
    let half_offsets = (n_offsets as f64 - 1.0) / 2.0;
    let half_across = (n_across as f64 - 1.0) / 2.0;
    let along_distance = |i: usize| (i as f64 - half_offsets) * step;
    let xs: Vec<f64> = (0..n_offsets).map(|i| midpoint.0 + along.0 * along_distance(i)).collect();
    let ys: Vec<f64> = (0..n_offsets).map(|i| midpoint.1 + along.1 * along_distance(i)).collect();

    let top_right = ((naxis1 - 1) as f64, (naxis2 - 1) as f64);
    let mut points: Vec<Option<(f64, f64)>> = Vec::with_capacity(n_offsets * n_across);
    let mut n_off_image = 0usize;
    let (mut y_min, mut y_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..n_offsets {
        for j in 0..n_across {
            let distance = (j as f64 - half_across) * step;
            let x = xs[i] + across.0 * distance;
            let y = ys[i] + across.1 * distance;
            if (0.0..=top_right.0).contains(&x) && (0.0..=top_right.1).contains(&y) {
                y_min = y_min.min(y);
                y_max = y_max.max(y);
                points.push(Some((x, y)));
            } else {
                n_off_image += 1;
                points.push(None);
            }
        }
    }
    if n_off_image == points.len() {
        bail!("slit lies entirely outside the {}x{} image", naxis1, naxis2);
    }

    let spectral = resolve_spectral(cube, cfg, n_channels)?;

    let (row_start, row_count) = row_band(y_min, y_max, naxis2);
    let row_origin = row_start as f64;
    let per_batch = channels_per_batch(row_count, naxis1);
    let mut data = Array2::<f32>::from_elem((n_channels, n_offsets), f32::NAN);
    let mut z = cfg.z0;
    while z <= cfg.z1 {
        let z_end = z.saturating_add(per_batch).min(cfg.z1.saturating_add(1));
        let bands = cube.decode_row_band_batch(z, z_end, row_start, row_count)?;
        for (k, band) in bands.iter().enumerate() {
            if band.len() != row_count * naxis1 {
                bail!("decoded band of {} values does not match {} rows x {} columns", band.len(), row_count, naxis1);
            }
            let row = z + k - cfg.z0;
            for i in 0..n_offsets {
                let slit_points = points.get(i * n_across..(i + 1) * n_across).unwrap_or(&[]);
                let mut acc = 0.0f64;
                let mut count = 0usize;
                for point in slit_points.iter().flatten() {
                    if let Some(v) = bilinear_at(band, row_count, naxis1, point.0, point.1 - row_origin) {
                        acc += v;
                        count += 1;
                    }
                }
                if count > 0 {
                    if let Some(cell) = data.get_mut((row, i)) {
                        *cell = (acc / count as f64) as f32;
                    }
                }
            }
        }
        z = z_end;
    }

    let mut ridge = vec![f64::NAN; n_offsets];
    let mut ridge_channel = vec![f64::NAN; n_offsets];
    let mut peak_channel = vec![None; n_offsets];
    let mut peak_value = vec![f64::NAN; n_offsets];
    let mut column = Vec::with_capacity(n_channels);
    let mut brightest: Option<(usize, usize, f64)> = None;
    for i in 0..n_offsets {
        column.clear();
        column.extend((0..n_channels).map(|k| data[[k, i]] as f64));
        let (r, rc) = median_subtracted_centroid(&column, &spectral.values, cfg.z0);
        ridge[i] = r;
        ridge_channel[i] = rc;
        let mut best: Option<(usize, f64)> = None;
        for (k, &v) in column.iter().enumerate() {
            if v.is_finite() && best.is_none_or(|(_, b)| v > b) {
                best = Some((k, v));
            }
        }
        if let Some((k, v)) = best {
            peak_channel[i] = Some(cfg.z0 + k);
            peak_value[i] = v;
            if brightest.is_none_or(|(_, _, b)| v > b) {
                brightest = Some((k, i, v));
            }
        }
    }

    let scale = offset_scale(cube, cfg, along);
    let offsets: Vec<f64> = (0..n_offsets).map(|i| (i as f64 - half_offsets) * scale.step).collect();
    let finite_ridge: Vec<f64> = ridge.iter().copied().filter(|v| v.is_finite()).collect();
    let ridge_span = if finite_ridge.is_empty() {
        None
    } else {
        let lo = finite_ridge.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = finite_ridge.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        finite_option(hi - lo)
    };
    let summary = PvSummary {
        n_offsets,
        n_channels,
        n_across,
        n_off_image,
        offset_step: scale.step,
        offset_unit: scale.unit,
        slit_length_px: length,
        slit_length: length / step * scale.step,
        slit_pa_deg: scale.slit_pa_deg,
        pixel_scale_arcsec: scale.pixel_scale_arcsec,
        ridge_gradient: least_squares_slope(&offsets, &ridge),
        ridge_gradient_unit: format!("{}/{}", spectral.unit, scale.unit),
        ridge_span,
        n_ridge_valid: finite_ridge.len(),
        peak_value: brightest.map(|(_, _, v)| v),
        peak_offset: brightest.and_then(|(_, i, _)| offsets.get(i).copied()),
        peak_spectral: brightest.and_then(|(k, _, _)| spectral.values.get(k).copied()).and_then(finite_option),
        bunit: card_text(cube, HEADER_BUNIT),
    };

    let mut notes = spectral.notes;
    notes.push(format!(
        "ridge = intensity-weighted spectral centroid per offset after subtracting the per-offset median over channels {}..={} (negative residuals get zero weight): choose a range where the line covers less than half of the channels",
        cfg.z0, cfg.z1
    ));
    notes.extend(scale.notes);
    if n_off_image > 0 {
        notes.push(format!(
            "{} of {} slit sample points fall outside the image and are NaN",
            n_off_image,
            n_offsets * n_across
        ));
    }
    notes.push(NOTE_DQ.to_string());
    notes.push(NOTE_ZEROS.to_string());

    Ok(PvDiagram {
        data,
        offsets,
        offset_unit: scale.unit,
        offset_step: scale.step,
        spectral_values: spectral.values,
        spectral_unit: spectral.unit,
        convention_applies: spectral.convention_applies,
        rest_um: spectral.rest_um,
        native: spectral.native,
        ridge,
        ridge_channel,
        peak_channel,
        peak_value,
        xs,
        ys,
        n_across,
        summary,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cube::lazy::test_support::*;
    use crate::core::imaging::region::line_cut;

    const SLIT_H: (f64, f64, f64, f64) = (2.0, 16.0, 29.0, 16.0);
    const SLIT_V: (f64, f64, f64, f64) = (16.0, 2.0, 16.0, 29.0);
    const GRADIENT_SIGMA: f64 = 1.5;

    fn config(slit: (f64, f64, f64, f64), step_px: f64, width_px: f64, z0: usize, z1: usize) -> PvConfig {
        PvConfig {
            x0: slit.0,
            y0: slit.1,
            x1: slit.2,
            y1: slit.3,
            z0,
            z1,
            step_px,
            width_px,
            mode: PvSpectralMode::Velocity,
            rest_um: None,
            convention: VelocityConvention::Optical,
            velocity_shift_kms: 0.0,
        }
    }

    fn full_config(slit: (f64, f64, f64, f64), step_px: f64, width_px: f64) -> PvConfig {
        config(slit, step_px, width_px, 0, LINE_CUBE_DEPTH - 1)
    }

    fn gradient_centre(x: usize) -> f64 {
        10.0 + 0.5 * x as f64
    }

    fn gradient_value(z: usize, x: usize) -> f64 {
        let d = z as f64 - gradient_centre(x);
        (-d * d / (2.0 * GRADIENT_SIGMA * GRADIENT_SIGMA)).exp()
    }

    fn write_gradient_cube(path: &std::path::Path, continuum: f32) {
        write_cube(path, LINE_CUBE_SIZE, LINE_CUBE_SIZE, LINE_CUBE_DEPTH, &line_cube_cards(), |z, _y, x| {
            continuum + gradient_value(z, x) as f32
        });
    }

    fn open(path: &std::path::Path) -> LazyCube {
        LazyCube::open(path.to_str().unwrap()).unwrap()
    }

    fn channel_width_kms() -> f64 {
        SPEED_OF_LIGHT_KMS * LINE_CDELT_UM / LINE_REST_UM
    }

    fn optical_velocity_of_channel(c: f64) -> f64 {
        SPEED_OF_LIGHT_KMS * (1.0 + LINE_CDELT_UM * c - LINE_REST_UM) / LINE_REST_UM
    }

    fn cards_without(keys: &[&str]) -> Vec<(&'static str, &'static str)> {
        line_cube_cards().into_iter().filter(|(k, _)| !keys.contains(k)).collect()
    }

    fn cards_with(overrides: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut all = line_cube_cards();
        for (key, value) in overrides {
            all.retain(|(k, _)| k != key);
            all.push((key, value));
        }
        all
    }

    #[test]
    fn a_linear_velocity_gradient_along_x_gives_a_straight_ridge_within_a_tenth_of_a_channel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gradient.fits");
        write_gradient_cube(&path, 0.0);
        let cube = open(&path);
        let pv = pv_diagram(&cube, &full_config(SLIT_H, 1.0, 1.0)).unwrap();
        assert_eq!(pv.summary.n_offsets, 28);
        assert_eq!(pv.spectral_unit, "km/s");
        let dv = channel_width_kms();
        for i in 0..28 {
            let x = 2 + i;
            let centre = gradient_centre(x);
            assert!((pv.ridge_channel[i] - centre).abs() < 0.1, "offset {}: ridge channel {} vs {}", i, pv.ridge_channel[i], centre);
            let expected = optical_velocity_of_channel(centre);
            assert!((pv.ridge[i] - expected).abs() < 0.1 * dv, "offset {}: ridge {} vs {}", i, pv.ridge[i], expected);
            for z in 0..LINE_CUBE_DEPTH {
                let value = pv.data[[z, i]] as f64;
                assert!((value - gradient_value(z, x)).abs() < 1e-6, "z={} i={} {}", z, i, value);
            }
        }
        let gradient = pv.summary.ridge_gradient.unwrap();
        let expected = 0.5 * dv / 0.36;
        assert!((gradient - expected).abs() < 1e-3 * expected, "{} vs {}", gradient, expected);
        assert_eq!(pv.summary.ridge_gradient_unit, "km/s/arcsec");
    }

    #[test]
    fn a_slit_perpendicular_to_the_gradient_gives_a_flat_ridge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gradient.fits");
        write_gradient_cube(&path, 0.0);
        let cube = open(&path);
        let pv = pv_diagram(&cube, &full_config(SLIT_V, 1.0, 1.0)).unwrap();
        assert_eq!(pv.ridge_channel.len(), 28);
        for (i, c) in pv.ridge_channel.iter().enumerate() {
            assert!((c - 18.0).abs() < 0.01, "offset {}: {}", i, c);
        }
    }

    #[test]
    fn the_per_offset_median_removes_a_constant_continuum_from_the_ridge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gradient_continuum.fits");
        write_gradient_cube(&path, LINE_CONTINUUM);
        let cube = open(&path);
        let pv = pv_diagram(&cube, &full_config(SLIT_H, 1.0, 1.0)).unwrap();
        assert!((pv.ridge_channel[0] - 11.0).abs() < 0.05, "{}", pv.ridge_channel[0]);
        assert!(pv.notes.iter().any(|n| n.contains("per-offset median")), "{:?}", pv.notes);
    }

    #[test]
    fn width_averaging_takes_the_mean_of_the_across_samples() {
        let dir = tempfile::tempdir().unwrap();
        let ramp = dir.path().join("ramp.fits");
        write_cube(&ramp, LINE_CUBE_SIZE, LINE_CUBE_SIZE, 4, &line_cube_cards(), |_, y, _| 1.0 + y as f32);
        let cube = open(&ramp);
        for (width, n_across) in [(3.0, 3), (1.0, 1), (2.0, 2)] {
            let pv = pv_diagram(&cube, &config(SLIT_H, 1.0, width, 0, 3)).unwrap();
            assert_eq!(pv.n_across, n_across, "width {}", width);
            for v in pv.data.iter() {
                assert!((*v - 17.0).abs() < 1e-6, "width {}: {}", width, v);
            }
        }
        let bowl = dir.path().join("bowl.fits");
        write_cube(&bowl, LINE_CUBE_SIZE, LINE_CUBE_SIZE, 4, &line_cube_cards(), |_, y, _| (y as f32 - 16.0).powi(2));
        let cube = open(&bowl);
        for (width, expected) in [(3.0, 2.0 / 3.0), (1.0, 0.0), (2.0, 0.5)] {
            let pv = pv_diagram(&cube, &config(SLIT_H, 1.0, width, 0, 3)).unwrap();
            for v in pv.data.iter() {
                assert!((*v as f64 - expected).abs() < 1e-6, "width {}: {} vs {}", width, v, expected);
            }
        }
    }

    #[test]
    fn width_averaging_reduces_noise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noise.fits");
        write_cube(&path, LINE_CUBE_SIZE, LINE_CUBE_SIZE, 4, &line_cube_cards(), |z, y, x| 1.0 + deterministic_noise(z, y, x, 0.2));
        let cube = open(&path);
        let std_of = |width: f64| {
            let pv = pv_diagram(&cube, &config(SLIT_H, 1.0, width, 0, 3)).unwrap();
            let row: Vec<f64> = (0..pv.summary.n_offsets).map(|i| pv.data[[0, i]] as f64).collect();
            let mean = row.iter().sum::<f64>() / row.len() as f64;
            (row.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / row.len() as f64).sqrt()
        };
        let wide = std_of(5.0);
        let narrow = std_of(1.0);
        assert!(wide < 0.7 * narrow, "wide {} narrow {}", wide, narrow);
    }

    #[test]
    fn offsets_are_centred_on_the_slit_midpoint_and_positive_toward_the_second_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let pv = pv_diagram(&cube, &full_config(SLIT_H, 1.0, 1.0)).unwrap();
        assert_eq!(pv.offset_unit, OFFSET_UNIT_ARCSEC);
        assert!(pv.offsets.iter().sum::<f64>().abs() < 1e-9);
        assert!(pv.offsets.windows(2).all(|w| w[1] > w[0]));
        assert!((pv.offsets[27] - pv.offsets[0] - 27.0 * 0.36).abs() < 1e-9);
        assert_eq!(pv.xs[0], 2.0);
        assert_eq!(pv.xs[27], 29.0);
        let reversed = pv_diagram(&cube, &full_config((29.0, 16.0, 2.0, 16.0), 1.0, 1.0)).unwrap();
        assert_eq!(reversed.xs[0], 29.0);
        let coarse = pv_diagram(&cube, &full_config((2.0, 16.0, 16.0, 16.0), 5.0, 1.0)).unwrap();
        assert_eq!(coarse.summary.n_offsets, 3);
        assert_eq!(coarse.xs, vec![4.0, 9.0, 14.0]);
        assert_eq!(coarse.ys, vec![16.0; 3]);
        for (offset, expected) in coarse.offsets.iter().zip([-1.8, 0.0, 1.8]) {
            assert!((offset - expected).abs() < 1e-9, "{} vs {}", offset, expected);
        }
    }

    #[test]
    fn the_offset_step_projects_the_cd_matrix_along_the_slit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aniso.fits");
        write_cube(&path, LINE_CUBE_SIZE, LINE_CUBE_SIZE, 4, &cards_with(&[("CD2_2", "2.0E-4")]), |_, _, _| 1.0);
        let cube = open(&path);
        let vertical = pv_diagram(&cube, &config(SLIT_V, 1.0, 1.0, 0, 3)).unwrap();
        assert!((vertical.offset_step - 0.72).abs() < 1e-9, "{}", vertical.offset_step);
        let horizontal = pv_diagram(&cube, &config(SLIT_H, 1.0, 1.0, 0, 3)).unwrap();
        assert!((horizontal.offset_step - 0.36).abs() < 1e-9, "{}", horizontal.offset_step);

        let bare = dir.path().join("nowcs.fits");
        let cards = cards_without(&["CTYPE1", "CTYPE2", "CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2", "CD1_1", "CD1_2", "CD2_1", "CD2_2"]);
        write_cube(&bare, LINE_CUBE_SIZE, LINE_CUBE_SIZE, 4, &cards, |_, _, _| 1.0);
        let cube = open(&bare);
        let pv = pv_diagram(&cube, &config(SLIT_H, 1.0, 1.0, 0, 3)).unwrap();
        assert_eq!(pv.offset_unit, OFFSET_UNIT_PIXEL);
        assert_eq!(pv.offset_step, 1.0);
        assert!(pv.summary.slit_pa_deg.is_none());
        assert!(pv.summary.pixel_scale_arcsec.is_none());
        assert!(pv.notes.iter().any(|n| n == "no celestial WCS: offsets in pixels"), "{:?}", pv.notes);
    }

    #[test]
    fn the_slit_position_angle_is_east_of_north() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let north = pv_diagram(&cube, &full_config((16.0, 16.0, 16.0, 26.0), 1.0, 1.0)).unwrap();
        let pa = north.summary.slit_pa_deg.unwrap();
        assert!(pa.min(360.0 - pa) < 0.01, "{}", pa);
        let east = pv_diagram(&cube, &full_config((16.0, 16.0, 6.0, 16.0), 1.0, 1.0)).unwrap();
        let pa = east.summary.slit_pa_deg.unwrap();
        assert!((pa - 90.0).abs() < 0.01, "{}", pa);
    }

    #[test]
    fn band_sampling_equals_line_cut_at_every_sample_of_an_integer_length_slit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.05);
        let cube = open(&path);
        let slit = (2.0, 3.0, 26.0, 21.0);
        let pv = pv_diagram(&cube, &full_config(slit, 1.0, 1.0)).unwrap();
        assert_eq!(pv.summary.n_offsets, 31);
        for z in [0usize, 20, 39] {
            let frame = cube.get_frame(z).unwrap();
            let cut = line_cut(&frame, slit.0, slit.1, slit.2, slit.3, None).unwrap();
            assert_eq!(cut.n_samples, 31);
            for (i, sample) in cut.values.iter().enumerate() {
                let ours = pv.data[[z, i]] as f64;
                match sample {
                    Some(v) => assert!((ours - v).abs() <= 1e-6 * v.abs().max(1.0), "z={} i={} {} vs {}", z, i, ours, v),
                    None => assert!(ours.is_nan(), "z={} i={}", z, i),
                }
            }
        }
    }

    #[test]
    fn a_horizontal_slit_on_an_integer_row_and_on_the_top_row_samples_without_nan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plane.fits");
        write_cube(&path, LINE_CUBE_SIZE, LINE_CUBE_SIZE, 4, &line_cube_cards(), |_, y, x| (x + 10 * y) as f32);
        let cube = open(&path);
        for y in [16.0, 31.0] {
            let pv = pv_diagram(&cube, &config((2.0, y, 29.0, y), 1.0, 1.0, 0, 3)).unwrap();
            for i in 0..pv.summary.n_offsets {
                let expected = (2 + i) as f64 + 10.0 * y;
                for z in 0..4 {
                    let v = pv.data[[z, i]] as f64;
                    assert!(v.is_finite() && (v - expected).abs() < 1e-6, "y={} z={} i={} {}", y, z, i, v);
                }
            }
        }
        assert_eq!(row_band(16.0, 16.0, 32), (16, 2));
        assert_eq!(row_band(31.0, 31.0, 32), (30, 2));
        assert_eq!(row_band(3.2, 7.9, 32), (3, 6));
    }

    #[test]
    fn an_endpoint_off_the_image_gives_nan_only_for_the_outside_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let pv = pv_diagram(&cube, &full_config((-3.0, 16.0, 10.0, 16.0), 1.0, 1.0)).unwrap();
        assert_eq!(pv.summary.n_offsets, 14);
        for z in 0..LINE_CUBE_DEPTH {
            for i in 0..3 {
                assert!(pv.data[[z, i]].is_nan(), "z={} i={}", z, i);
            }
            for i in 3..14 {
                assert!(pv.data[[z, i]].is_finite(), "z={} i={}", z, i);
            }
        }
        assert_eq!(pv.summary.n_off_image, 3);
        assert!(pv.notes.iter().any(|n| n == "3 of 14 slit sample points fall outside the image and are NaN"), "{:?}", pv.notes);
    }

    #[test]
    fn spectral_values_follow_the_requested_mode_and_shift() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let mut cfg = full_config(SLIT_H, 1.0, 1.0);
        let lambda = |z: usize| 1.0 + LINE_CDELT_UM * z as f64;

        cfg.mode = PvSpectralMode::WavelengthVac;
        let pv = pv_diagram(&cube, &cfg).unwrap();
        assert_eq!(pv.spectral_unit, "um");
        for z in 0..LINE_CUBE_DEPTH {
            assert!((pv.spectral_values[z] - lambda(z)).abs() < 1e-12);
        }
        cfg.mode = PvSpectralMode::Frequency;
        let pv = pv_diagram(&cube, &cfg).unwrap();
        assert_eq!(pv.spectral_unit, "GHz");
        for z in 0..LINE_CUBE_DEPTH {
            assert!((pv.spectral_values[z] - SPEED_OF_LIGHT_KMS / lambda(z)).abs() < 1e-9);
        }
        cfg.mode = PvSpectralMode::WavelengthAir;
        let pv = pv_diagram(&cube, &cfg).unwrap();
        assert_eq!(pv.spectral_unit, "um");
        for z in 0..LINE_CUBE_DEPTH {
            assert!((pv.spectral_values[z] - lambda(z) / air_refractive_index(lambda(z))).abs() < 1e-12);
        }
        cfg.mode = PvSpectralMode::Velocity;
        cfg.velocity_shift_kms = 12.5;
        let pv = pv_diagram(&cube, &cfg).unwrap();
        assert_eq!(pv.spectral_unit, "km/s");
        assert!(pv.convention_applies);
        for z in 0..LINE_CUBE_DEPTH {
            let expected = optical_velocity_of_channel(z as f64) + 12.5;
            assert!((pv.spectral_values[z] - expected).abs() < 1e-9, "z={} {} vs {}", z, pv.spectral_values[z], expected);
        }
        assert!(pv.notes.iter().any(|n| n.contains("shifted by +12.500 km/s")), "{:?}", pv.notes);
        cfg.velocity_shift_kms = 0.0;

        let no_rest = dir.path().join("no_rest.fits");
        write_line_cube_with_cards(&no_rest, &[("RESTWAV", "")]);
        let err = pv_diagram(&open(&no_rest), &cfg).unwrap_err().to_string();
        assert!(err.contains(PV_REST_REQUIRED), "{}", err);

        let foo = dir.path().join("foo.fits");
        write_line_cube_with_cards(&foo, &[("CTYPE3", "'FOO'")]);
        let cube = open(&foo);
        cfg.mode = PvSpectralMode::WavelengthVac;
        let pv = pv_diagram(&cube, &cfg).unwrap();
        assert_eq!(pv.spectral_unit, CHANNEL_UNIT);
        for z in 0..LINE_CUBE_DEPTH {
            assert_eq!(pv.spectral_values[z], z as f64);
        }
        assert!(
            pv.notes.iter().any(|n| n == "no spectral WCS (CTYPE3 'FOO' is not a spectral axis): the PV spectral axis is the channel index"),
            "{:?}",
            pv.notes
        );
        cfg.mode = PvSpectralMode::Velocity;
        let err = pv_diagram(&cube, &cfg).unwrap_err().to_string();
        assert!(err.contains("CTYPE3 'FOO' is not a spectral axis: a velocity PV needs"), "{}", err);

        let log = dir.path().join("log.fits");
        write_line_cube_with_cards(&log, &[("CTYPE3", "'WAVE-LOG'")]);
        let cube = open(&log);
        cfg.mode = PvSpectralMode::WavelengthVac;
        let pv = pv_diagram(&cube, &cfg).unwrap();
        assert_eq!(pv.spectral_unit, CHANNEL_UNIT);
        assert!(
            pv.notes.iter().any(|n| n == "no spectral WCS (non-linear spectral axis (WAVE-LOG) is not supported): the PV spectral axis is the channel index"),
            "{:?}",
            pv.notes
        );
        cfg.mode = PvSpectralMode::Velocity;
        let err = pv_diagram(&cube, &cfg).unwrap_err().to_string();
        assert!(err.contains("no spectral WCS (non-linear spectral axis (WAVE-LOG) is not supported): a velocity PV needs"), "{}", err);

        let vrad = dir.path().join("vrad.fits");
        write_line_cube_with_cards(&vrad, &[("CTYPE3", "'VRAD'"), ("CUNIT3", "'km/s'")]);
        let cube = open(&vrad);
        cfg.convention = VelocityConvention::Radio;
        let pv = pv_diagram(&cube, &cfg).unwrap();
        assert_eq!(pv.spectral_unit, "km/s");
        assert!(!pv.convention_applies);
        for z in 0..LINE_CUBE_DEPTH {
            assert!((pv.spectral_values[z] - lambda(z)).abs() < 1e-12);
        }
        assert!(
            pv.notes.iter().any(|n| n.contains("velocity axis CTYPE3 'VRAD' returned as stored in the header's convention")),
            "{:?}",
            pv.notes
        );
    }

    #[test]
    fn the_peak_channel_is_the_brightest_finite_sample() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gradient.fits");
        write_gradient_cube(&path, 0.0);
        let cube = open(&path);
        let pv = pv_diagram(&cube, &full_config(SLIT_H, 1.0, 1.0)).unwrap();
        for i in 0..28 {
            let centre = gradient_centre(2 + i);
            let peak = pv.peak_channel[i].unwrap();
            assert!(peak == centre.floor() as usize || peak == centre.ceil() as usize, "offset {}: peak {} centre {}", i, peak, centre);
            assert!((pv.peak_value[i] - pv.data[[peak, i]] as f64).abs() < 1e-6);
        }
    }

    #[test]
    fn refusals_name_the_parameter_and_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = open(&path);
        let err = |cfg: &PvConfig| pv_diagram(&cube, cfg).unwrap_err().to_string();

        assert!(err(&full_config((5.0, 5.0, 5.0, 5.0), 1.0, 1.0)).contains("slit has zero length"));
        assert!(err(&full_config(SLIT_H, 0.0, 1.0)).contains("step_px must be a positive finite number of pixels, got 0"));
        assert!(err(&full_config(SLIT_H, f64::NAN, 1.0)).contains("step_px must be a positive finite number of pixels, got NaN"));
        assert!(err(&full_config(SLIT_H, 1.0, -1.0)).contains("width_px must be a finite number of pixels >= 0, got -1"));
        assert!(err(&config(SLIT_H, 1.0, 1.0, 5, 3)).contains("channel range start 5 is after its end 3"));
        assert!(err(&config(SLIT_H, 1.0, 1.0, 0, 40)).contains("channel 40 is out of range"));
        assert!(err(&config(SLIT_H, 1.0, 1.0, 5, 5)).contains("a PV needs at least 2 channels, got z0..=z1 = 5..=5"));
        assert!(err(&full_config((-1e308, 16.0, 1e308, 16.0), 1.0, 1.0)).contains("slit length is not finite"));
        assert!(err(&full_config((40.0, 40.0, 50.0, 50.0), 1.0, 1.0)).contains("slit lies entirely outside the 32x32 image"));
        let e = err(&full_config(SLIT_H, 0.001, 1.0));
        assert!(e.contains("27001 offsets, more than the 8192 allowed"), "{}", e);
        let e = err(&full_config(SLIT_H, 0.05, 4096.0));
        assert!(e.contains("81920 across samples, more than the 256 allowed"), "{}", e);

        let thin = dir.path().join("thin.fits");
        write_cube(&thin, 1, 32, 4, &[], |_, _, _| 1.0);
        let e = pv_diagram(&open(&thin), &config((0.0, 2.0, 0.0, 20.0), 1.0, 1.0, 0, 3)).unwrap_err().to_string();
        assert!(e.contains("cube spatial plane is 1x32: bilinear sampling needs at least 2x2 pixels"), "{}", e);

        let deep = dir.path().join("deep.fits");
        write_cube(&deep, 2, 2, 4096, &[], |_, _, _| 1.0);
        let e = pv_diagram(&open(&deep), &config((-500.0, 0.5, 500.0, 0.5), 0.125, 32.0, 0, 4095)).unwrap_err().to_string();
        assert!(e.contains("8001 x 256 x 4096 bilinear samples exceeds the 268435456 allowed"), "{}", e);
    }

    #[test]
    fn channel_batches_and_the_sample_budget_are_bounded() {
        assert_eq!(channels_per_batch(2, 32), 32);
        assert_eq!(channels_per_batch(2, 16 << 20), 1);
        assert_eq!(channels_per_batch(usize::MAX / 2, 8), 1);
        assert_eq!(sample_budget(8192, 256, 40).unwrap(), 83_886_080);
        let e = sample_budget(8001, 256, 4096).unwrap_err().to_string();
        assert!(e.contains("8001 x 256 x 4096 bilinear samples"), "{}", e);
        assert!(sample_budget(usize::MAX, 2, 2).is_err());
    }

    #[test]
    fn bilinear_at_matches_the_region_rule() {
        let band = [0.0f32, 1.0, 2.0, 3.0];
        assert_eq!(bilinear_at(&band, 2, 2, 0.5, 0.5), Some(1.5));
        assert_eq!(bilinear_at(&band, 2, 2, 1.0, 1.0), Some(3.0));
        assert_eq!(bilinear_at(&band, 2, 2, 1.5, 0.0), None);
        let holed = [0.0f32, f32::NAN, 2.0, 3.0];
        assert_eq!(bilinear_at(&holed, 2, 2, 0.5, 0.5), None);
    }
}
