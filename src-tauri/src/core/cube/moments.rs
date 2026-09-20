use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;
use serde::Deserialize;

use crate::core::astrometry::spectral::{velocity_axis, SpectralAxis, VelocityConvention};
use crate::core::cube::lazy::{median_band_rows, LazyCube};
use crate::math::{exact_mad_mut, exact_median_mut};
use crate::types::constants::MAD_TO_SIGMA;

pub const DEFAULT_SNR_THRESHOLD: f64 = 3.0;
pub const VELOCITY_UNIT: &str = "km/s";
pub const REST_REQUIRED_MESSAGE: &str = "rest wavelength required for velocity moments";

const CONTINUUM_BAND_BYTES: usize = 256 << 20;
const MOMENT_BATCH_SIZE: usize = 16;

pub type ContinuumWindows = ((usize, usize), (usize, usize));

#[derive(Debug, Clone, Deserialize)]
struct MomentConfigWire {
    z0: usize,
    z1: usize,
    #[serde(default)]
    rest_um: Option<f64>,
    #[serde(default)]
    convention: Option<String>,
    #[serde(default)]
    continuum: Option<ContinuumWindows>,
    #[serde(default)]
    snr_threshold: Option<f64>,
    #[serde(default)]
    mask_below_threshold: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "MomentConfigWire")]
pub struct MomentConfig {
    pub z0: usize,
    pub z1: usize,
    pub rest_um: Option<f64>,
    pub convention: VelocityConvention,
    pub continuum: Option<ContinuumWindows>,
    pub snr_threshold: f64,
    pub mask_below_threshold: bool,
}

impl TryFrom<MomentConfigWire> for MomentConfig {
    type Error = String;

    fn try_from(wire: MomentConfigWire) -> std::result::Result<Self, Self::Error> {
        let convention = match wire.convention.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(name) => VelocityConvention::parse(name)?,
            None => VelocityConvention::Optical,
        };
        let snr_threshold = wire.snr_threshold.unwrap_or(DEFAULT_SNR_THRESHOLD);
        if !snr_threshold.is_finite() || snr_threshold < 0.0 {
            return Err(format!("snr_threshold must be a non-negative number, got {}", snr_threshold));
        }
        Ok(MomentConfig {
            z0: wire.z0,
            z1: wire.z1,
            rest_um: wire.rest_um,
            convention,
            continuum: wire.continuum,
            snr_threshold,
            mask_below_threshold: wire.mask_below_threshold.unwrap_or(true),
        })
    }
}

#[derive(Debug, Clone)]
pub struct MomentMaps {
    pub m0: Array2<f32>,
    pub m1: Array2<f32>,
    pub m2: Array2<f32>,
    pub m0_unit: String,
    pub velocity_unit: &'static str,
    pub noise_per_channel: Option<f64>,
    pub n_channels: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PixelFit {
    intercept: f32,
    slope: f32,
    sigma: f32,
}

fn robust_sigma(residuals: &mut Vec<f32>) -> f32 {
    if residuals.len() < 2 {
        return f32::NAN;
    }
    let median = exact_median_mut(residuals) as f32;
    (exact_mad_mut(residuals, median) as f64 * MAD_TO_SIGMA) as f32
}

fn fit_pixel(xs: &[f64], ys: &[f32], constant_only: bool) -> PixelFit {
    let mut n = 0usize;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in xs.iter().zip(ys) {
        if !y.is_finite() {
            continue;
        }
        let y = y as f64;
        n += 1;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    if n == 0 {
        return PixelFit { intercept: f32::NAN, slope: f32::NAN, sigma: f32::NAN };
    }
    let nf = n as f64;
    let denominator = nf * sxx - sx * sx;
    let (intercept, slope) = if constant_only || n < 3 || denominator.abs() <= 1e-12 * nf * sxx.max(1.0) {
        (sy / nf, 0.0)
    } else {
        let slope = (nf * sxy - sx * sy) / denominator;
        ((sy - slope * sx) / nf, slope)
    };
    let mut residuals: Vec<f32> = xs
        .iter()
        .zip(ys)
        .filter(|(_, y)| y.is_finite())
        .map(|(&x, &y)| (y as f64 - (intercept + slope * x)) as f32)
        .collect();
    PixelFit { intercept: intercept as f32, slope: slope as f32, sigma: robust_sigma(&mut residuals) }
}

pub fn channel_widths(velocities: &[f64]) -> Vec<f64> {
    let n = velocities.len();
    (0..n)
        .map(|i| {
            if n < 2 {
                0.0
            } else if i == 0 {
                (velocities[1] - velocities[0]).abs()
            } else if i + 1 == n {
                (velocities[n - 1] - velocities[n - 2]).abs()
            } else {
                (velocities[i + 1] - velocities[i - 1]).abs() / 2.0
            }
        })
        .collect()
}

fn velocities_for(axis: &SpectralAxis, cfg: &MomentConfig, notes: &mut Vec<String>) -> Result<Vec<f64>> {
    if !axis.kind.is_spectral() {
        bail!(
            "CTYPE3 '{}' is not a spectral axis: moments need a wavelength, frequency or velocity axis",
            axis.ctype
        );
    }
    if axis.kind.is_velocity() {
        notes.push(format!(
            "velocity axis {} used as stored in {} (rest wavelength and convention ignored)",
            axis.ctype, axis.unit
        ));
        return Ok(axis.values.clone());
    }
    let rest_um = match cfg.rest_um.filter(|r| r.is_finite() && *r > 0.0).or(axis.rest_wavelength_um) {
        Some(rest) => rest,
        None => bail!("{}", REST_REQUIRED_MESSAGE),
    };
    let velocity = velocity_axis(axis, rest_um, cfg.convention).map_err(anyhow::Error::msg)?;
    notes.extend(velocity.notes);
    notes.push(format!(
        "velocities from the {} convention with rest wavelength {} um",
        cfg.convention.name(),
        rest_um
    ));
    Ok(velocity.values_kms)
}

fn check_window(window: (usize, usize), depth: usize) -> Result<()> {
    if window.0 > window.1 || window.1 >= depth {
        bail!(
            "continuum window {}..={} is invalid for a cube with {} channels",
            window.0,
            window.1,
            depth
        );
    }
    Ok(())
}

fn window_channels(windows: ContinuumWindows, depth: usize) -> Result<Vec<usize>> {
    check_window(windows.0, depth)?;
    check_window(windows.1, depth)?;
    let mut channels: Vec<usize> = (windows.0 .0..=windows.0 .1).chain(windows.1 .0..=windows.1 .1).collect();
    channels.sort_unstable();
    channels.dedup();
    Ok(channels)
}

struct ContinuumFit {
    intercept: Vec<f32>,
    slope: Vec<f32>,
    sigma: Vec<f32>,
    linear: bool,
}

fn fit_continuum(cube: &LazyCube, windows: ContinuumWindows, notes: &mut Vec<String>) -> Result<ContinuumFit> {
    let g = &cube.geometry;
    let (rows, cols, depth) = (g.naxis2, g.naxis1, g.naxis3);
    let channels = window_channels(windows, depth)?;
    let linear = windows.0 != windows.1 && channels.len() >= 3;
    let xs: Vec<f64> = channels.iter().map(|&z| z as f64).collect();
    let band_rows = median_band_rows(channels.len(), cols, rows, CONTINUUM_BAND_BYTES);
    let npix = rows * cols;
    let mut intercept = vec![f32::NAN; npix];
    let mut slope = vec![f32::NAN; npix];
    let mut sigma = vec![f32::NAN; npix];

    for band_start in (0..rows).step_by(band_rows) {
        let band_end = (band_start + band_rows).min(rows);
        let row_count = band_end - band_start;
        let planes: Vec<Vec<f32>> = channels
            .par_iter()
            .map(|&z| cube.decode_rows(z, band_start, row_count))
            .collect::<Result<_>>()?;
        let band_npix = row_count * cols;
        let fits: Vec<PixelFit> = (0..band_npix)
            .into_par_iter()
            .map(|i| {
                let ys: Vec<f32> = planes.iter().map(|p| p[i]).collect();
                fit_pixel(&xs, &ys, !linear)
            })
            .collect();
        let offset = band_start * cols;
        for (i, fit) in fits.into_iter().enumerate() {
            intercept[offset + i] = fit.intercept;
            slope[offset + i] = fit.slope;
            sigma[offset + i] = fit.sigma;
        }
    }

    notes.push(format!(
        "continuum: {} fitted per pixel over channels {}..={} and {}..={}, subtracted before the moments",
        if linear { "first-order line" } else { "constant" },
        windows.0 .0,
        windows.0 .1,
        windows.1 .0,
        windows.1 .1
    ));
    Ok(ContinuumFit { intercept, slope, sigma, linear })
}

fn median_finite(values: &[f32]) -> Option<f64> {
    let mut finite: Vec<f32> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        None
    } else {
        Some(exact_median_mut(&mut finite))
    }
}

fn line_free_channel(cfg: &MomentConfig, depth: usize) -> Option<usize> {
    if cfg.z0 > 0 {
        Some(cfg.z0 - 1)
    } else if cfg.z1 + 1 < depth {
        Some(cfg.z1 + 1)
    } else {
        None
    }
}

fn global_noise_from_channel(cube: &LazyCube, cfg: &MomentConfig, notes: &mut Vec<String>) -> Result<Option<f64>> {
    let Some(z) = line_free_channel(cfg, cube.geometry.naxis3) else {
        notes.push("no line-free channel outside the range: noise unknown, SNR mask not applied".to_string());
        return Ok(None);
    };
    let plane = cube.decode_rows(z, 0, cube.geometry.naxis2)?;
    let mut finite: Vec<f32> = plane.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.len() < 2 {
        notes.push(format!("channel {} has no finite pixels: noise unknown, SNR mask not applied", z));
        return Ok(None);
    }
    let median = exact_median_mut(&mut finite) as f32;
    let sigma = exact_mad_mut(&mut finite, median) as f64 * MAD_TO_SIGMA;
    notes.push(format!(
        "no continuum windows: moments use the raw channel values; noise = robust sigma of channel {} ({:.4e})",
        z, sigma
    ));
    Ok(Some(sigma))
}

#[derive(Clone, Copy, Default)]
struct PixelAccumulator {
    m0: f64,
    weight: f64,
    weighted_v: f64,
    weighted_v2: f64,
    count: u32,
}

fn bunit(cube: &LazyCube) -> Option<String> {
    cube.header
        .get("BUNIT")
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn moment_maps(cube: &LazyCube, cfg: &MomentConfig) -> Result<MomentMaps> {
    let g = &cube.geometry;
    let (rows, cols, depth) = (g.naxis2, g.naxis1, g.naxis3);
    cube.check_channel_range(cfg.z0, cfg.z1)?;
    let n_channels = cfg.z1 - cfg.z0 + 1;
    if n_channels < 2 {
        bail!("moment maps need at least two channels in the range, got {}", n_channels);
    }
    let axis = cube.spectral_axis().map_err(anyhow::Error::msg)?;
    let mut notes = Vec::new();
    let velocities = velocities_for(&axis, cfg, &mut notes)?;
    if velocities.len() != depth {
        bail!("spectral axis has {} values for a cube with {} channels", velocities.len(), depth);
    }
    let widths = channel_widths(&velocities);
    let dv_mean = widths[cfg.z0..=cfg.z1].iter().sum::<f64>() / n_channels as f64;
    if !(dv_mean > 0.0) {
        bail!("channel velocity width is zero: the spectral axis has no usable step");
    }

    let continuum = match cfg.continuum {
        Some(windows) => Some(fit_continuum(cube, windows, &mut notes)?),
        None => None,
    };
    let global_noise = match &continuum {
        Some(fit) => {
            let level = median_finite(&fit.sigma).filter(|s| *s > 0.0);
            match level {
                Some(s) => notes.push(format!(
                    "noise per channel = robust sigma of the {} residuals per pixel, floored at the map median {:.4e}",
                    if fit.linear { "linear-fit" } else { "constant-fit" },
                    s
                )),
                None => notes.push("continuum residuals have zero scatter: per-pixel noise unavailable".to_string()),
            }
            level
        }
        None => global_noise_from_channel(cube, cfg, &mut notes)?,
    };

    let npix = rows * cols;
    let mut acc = vec![PixelAccumulator::default(); npix];
    for batch_start in (cfg.z0..=cfg.z1).step_by(MOMENT_BATCH_SIZE) {
        let batch_end = (batch_start + MOMENT_BATCH_SIZE).min(cfg.z1 + 1);
        let frames = cube.decode_frames(batch_start, batch_end)?;
        for (k, frame) in frames.iter().enumerate() {
            let z = batch_start + k;
            let v = velocities[z];
            let dv = widths[z];
            let xz = z as f64;
            acc.par_iter_mut().enumerate().for_each(|(i, a)| {
                let raw = frame[i];
                if !raw.is_finite() {
                    return;
                }
                let value = match &continuum {
                    Some(fit) => {
                        let baseline = fit.intercept[i] as f64 + fit.slope[i] as f64 * xz;
                        if !baseline.is_finite() {
                            return;
                        }
                        raw as f64 - baseline
                    }
                    None => raw as f64,
                };
                a.count += 1;
                a.m0 += value * dv;
                if value > 0.0 {
                    a.weight += value;
                    a.weighted_v += value * v;
                    a.weighted_v2 += value * v * v;
                }
            });
        }
    }

    let sqrt_n = (n_channels as f64).sqrt();
    let threshold = cfg.snr_threshold;
    let mut m0 = vec![f32::NAN; npix];
    let mut m1 = vec![f32::NAN; npix];
    let mut m2 = vec![f32::NAN; npix];
    let mut masked = 0usize;
    let mut unmaskable = 0usize;
    for i in 0..npix {
        let a = acc[i];
        if a.count == 0 {
            continue;
        }
        let per_pixel = continuum
            .as_ref()
            .map(|fit| fit.sigma[i] as f64)
            .filter(|s| s.is_finite() && *s > 0.0);
        let noise = match (per_pixel, global_noise) {
            (Some(local), Some(floor)) => Some(local.max(floor)),
            (local, floor) => local.or(floor),
        };
        let passes = match noise {
            Some(sigma) => a.m0 / (sigma * sqrt_n * dv_mean) >= threshold,
            None => {
                unmaskable += 1;
                true
            }
        };
        if !passes {
            masked += 1;
        }
        if passes || !cfg.mask_below_threshold {
            m0[i] = a.m0 as f32;
        }
        if passes && a.weight > 0.0 {
            let mean_v = a.weighted_v / a.weight;
            let variance = (a.weighted_v2 / a.weight - mean_v * mean_v).max(0.0);
            m1[i] = mean_v as f32;
            m2[i] = variance.sqrt() as f32;
        }
    }
    notes.push(format!(
        "SNR threshold {}: {} of {} pixels below threshold (M1/M2 set to NaN{})",
        threshold,
        masked,
        npix,
        if cfg.mask_below_threshold { ", M0 too" } else { "" }
    ));
    if unmaskable > 0 {
        notes.push(format!("{} pixels had no noise estimate and were kept unmasked", unmaskable));
    }

    let m0_unit = match bunit(cube) {
        Some(unit) => format!("{} {}", unit, VELOCITY_UNIT),
        None => VELOCITY_UNIT.to_string(),
    };
    let shape = (rows, cols);
    Ok(MomentMaps {
        m0: Array2::from_shape_vec(shape, m0)?,
        m1: Array2::from_shape_vec(shape, m1)?,
        m2: Array2::from_shape_vec(shape, m2)?,
        m0_unit,
        velocity_unit: VELOCITY_UNIT,
        noise_per_channel: global_noise,
        n_channels,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::spectral::SPEED_OF_LIGHT_KMS;
    use crate::core::cube::lazy::test_support::*;

    fn channel_width_kms() -> f64 {
        SPEED_OF_LIGHT_KMS * LINE_CDELT_UM / LINE_REST_UM
    }

    fn line_config() -> MomentConfig {
        MomentConfig {
            z0: 8,
            z1: 32,
            rest_um: None,
            convention: VelocityConvention::Optical,
            continuum: Some(((0, 5), (34, 39))),
            snr_threshold: DEFAULT_SNR_THRESHOLD,
            mask_below_threshold: true,
        }
    }

    fn distance_from_disk_centre(y: usize, x: usize) -> f64 {
        (x as f64 - LINE_DISK_CENTRE).hypot(y as f64 - LINE_DISK_CENTRE)
    }

    #[test]
    fn moment_maps_recover_the_line_centre_and_width_and_mask_the_continuum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_noisy.fits");
        write_line_cube(&path, 0.01);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let maps = moment_maps(&cube, &line_config()).unwrap();

        assert_eq!(maps.n_channels, 25);
        assert_eq!(maps.m0_unit, "Jy/beam km/s");
        assert_eq!(maps.velocity_unit, "km/s");
        assert_eq!(maps.m0.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
        let noise = maps.noise_per_channel.expect("noise estimate");
        assert!(noise > 0.002 && noise < 0.02, "noise={}", noise);

        let dv = channel_width_kms();
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        let m1 = maps.m1[centre] as f64;
        let m2 = maps.m2[centre] as f64;
        let m0 = maps.m0[centre] as f64;
        assert!(m1.abs() < 0.2 * dv, "m1={} dv={}", m1, dv);
        let expected_m2 = LINE_SIGMA_CHANNELS * dv;
        assert!((m2 - expected_m2).abs() < 0.1 * expected_m2, "m2={} expected={}", m2, expected_m2);
        let expected_m0: f64 = (8..=32).map(|z| line_profile(z) as f64).sum::<f64>() * dv;
        assert!((m0 - expected_m0).abs() < 0.03 * expected_m0, "m0={} expected={}", m0, expected_m0);

        let mut outside = 0usize;
        let mut outside_masked = 0usize;
        for y in 0..LINE_CUBE_SIZE {
            for x in 0..LINE_CUBE_SIZE {
                let d = distance_from_disk_centre(y, x);
                if d <= 4.0 {
                    assert!(maps.m1[[y, x]].is_finite(), "inside pixel ({}, {}) masked", y, x);
                    assert!(maps.m2[[y, x]].is_finite());
                    assert!(maps.m0[[y, x]].is_finite());
                } else if d > 8.0 {
                    outside += 1;
                    let masked = maps.m1[[y, x]].is_nan() && maps.m2[[y, x]].is_nan() && maps.m0[[y, x]].is_nan();
                    if masked {
                        outside_masked += 1;
                    }
                }
            }
        }
        assert!(outside > 500);
        assert!(
            outside_masked as f64 >= 0.95 * outside as f64,
            "masked {} of {} outside pixels",
            outside_masked,
            outside
        );
        assert!(maps.notes.iter().any(|n| n.contains("continuum")));
    }

    #[test]
    fn unmasked_m0_keeps_the_continuum_pixels_when_masking_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line_noisy.fits");
        write_line_cube(&path, 0.01);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let mut cfg = line_config();
        cfg.mask_below_threshold = false;
        let maps = moment_maps(&cube, &cfg).unwrap();
        let corner = maps.m0[[0, 0]];
        assert!(corner.is_finite());
        assert!(corner.abs() < 100.0, "corner m0={}", corner);
        assert!(maps.m1[[0, 0]].is_nan());
    }

    #[test]
    fn moment_maps_require_a_rest_wavelength_when_the_header_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no_rest.fits");
        write_line_cube_with_cards(&path, &[("RESTWAV", "")]);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        assert!(cube.spectral_axis().unwrap().rest_wavelength_um.is_none());
        let err = moment_maps(&cube, &line_config()).unwrap_err().to_string();
        assert!(err.contains(REST_REQUIRED_MESSAGE), "{}", err);

        let mut cfg = line_config();
        cfg.rest_um = Some(LINE_REST_UM);
        let maps = moment_maps(&cube, &cfg).unwrap();
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        assert!((maps.m1[centre] as f64).abs() < 0.2 * channel_width_kms());
    }

    #[test]
    fn a_velocity_axis_is_used_directly_without_a_rest_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vrad.fits");
        let cards = [
            ("CTYPE3", "'VRAD'"),
            ("CUNIT3", "'km/s'"),
            ("CRVAL3", "-300.0"),
            ("CDELT3", "40.0"),
            ("CRPIX3", "1.0"),
            ("BUNIT", "'K'"),
        ];
        write_cube(&path, 8, 8, 16, &cards, |z, _, _| {
            let d = z as f64 - 7.0;
            (2.0 * (-d * d / (2.0 * 1.5 * 1.5)).exp()) as f32
        });
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let cfg = MomentConfig {
            z0: 1,
            z1: 13,
            rest_um: None,
            convention: VelocityConvention::Radio,
            continuum: None,
            snr_threshold: DEFAULT_SNR_THRESHOLD,
            mask_below_threshold: true,
        };
        let maps = moment_maps(&cube, &cfg).unwrap();
        assert_eq!(maps.m0_unit, "K km/s");
        assert!((maps.m1[[3, 3]] as f64 + 20.0).abs() < 0.2 * 40.0, "m1={}", maps.m1[[3, 3]]);
        assert!((maps.m2[[3, 3]] as f64 - 60.0).abs() < 6.0, "m2={}", maps.m2[[3, 3]]);
        assert!(maps.m0[[3, 3]].is_finite());
        assert!(maps.notes.iter().any(|n| n.contains("velocity axis")));
    }

    #[test]
    fn moment_config_deserializes_with_defaults_and_rejects_bad_conventions() {
        let cfg: MomentConfig = serde_json::from_str(r#"{"z0":8,"z1":32}"#).unwrap();
        assert_eq!(cfg.convention, VelocityConvention::Optical);
        assert_eq!(cfg.snr_threshold, DEFAULT_SNR_THRESHOLD);
        assert!(cfg.mask_below_threshold);
        assert!(cfg.rest_um.is_none());
        assert!(cfg.continuum.is_none());

        let cfg: MomentConfig = serde_json::from_str(
            r#"{"z0":1,"z1":2,"rest_um":0.65646,"convention":"radio","continuum":[[0,0],[5,6]],"snr_threshold":5,"mask_below_threshold":false}"#,
        )
        .unwrap();
        assert_eq!(cfg.convention, VelocityConvention::Radio);
        assert_eq!(cfg.continuum, Some(((0, 0), (5, 6))));
        assert_eq!(cfg.snr_threshold, 5.0);
        assert!(!cfg.mask_below_threshold);
        assert_eq!(cfg.rest_um, Some(0.65646));

        assert!(serde_json::from_str::<MomentConfig>(r#"{"z0":1,"z1":2,"convention":"sideways"}"#).is_err());
        assert!(serde_json::from_str::<MomentConfig>(r#"{"z0":1,"z1":2,"snr_threshold":-1}"#).is_err());
    }

    #[test]
    fn per_pixel_fit_handles_lines_constants_and_nan_samples() {
        let xs = [0.0, 1.0, 2.0, 10.0, 11.0, 12.0];
        let ys: Vec<f32> = xs.iter().map(|x| (0.5 + 0.02 * x) as f32).collect();
        let fit = fit_pixel(&xs, &ys, false);
        assert!((fit.intercept - 0.5).abs() < 1e-5, "{:?}", fit);
        assert!((fit.slope - 0.02).abs() < 1e-6, "{:?}", fit);
        assert!(fit.sigma.abs() < 1e-5);

        let constant = fit_pixel(&xs, &ys, true);
        let mean_y = ys.iter().map(|v| *v as f64).sum::<f64>() / ys.len() as f64;
        assert!((constant.intercept as f64 - mean_y).abs() < 1e-6);
        assert_eq!(constant.slope, 0.0);
        assert!(constant.sigma > 0.0);

        let mut with_nan = ys.clone();
        with_nan[3] = f32::NAN;
        let fit = fit_pixel(&xs, &with_nan, false);
        assert!((fit.intercept - 0.5).abs() < 1e-5);
        assert!((fit.slope - 0.02).abs() < 1e-6);

        let all_nan = vec![f32::NAN; xs.len()];
        let fit = fit_pixel(&xs, &all_nan, false);
        assert!(fit.intercept.is_nan() && fit.slope.is_nan() && fit.sigma.is_nan());

        let two = fit_pixel(&xs[..2], &ys[..2], false);
        assert!((two.intercept - 0.51).abs() < 1e-5, "{:?}", two);
        assert_eq!(two.slope, 0.0);
    }

    #[test]
    fn channel_widths_use_centred_differences_with_one_sided_edges() {
        let widths = channel_widths(&[0.0, 10.0, 30.0, 60.0]);
        assert_eq!(widths, vec![10.0, 15.0, 25.0, 30.0]);
        let decreasing = channel_widths(&[60.0, 30.0, 10.0, 0.0]);
        assert_eq!(decreasing, vec![30.0, 25.0, 15.0, 10.0]);
        assert_eq!(channel_widths(&[5.0]), vec![0.0]);
        assert!(channel_widths(&[]).is_empty());
    }

    #[test]
    fn moment_maps_reject_bad_ranges_and_windows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let mut cfg = line_config();
        cfg.z1 = LINE_CUBE_DEPTH;
        assert!(moment_maps(&cube, &cfg).unwrap_err().to_string().contains("out of range"));
        let mut cfg = line_config();
        cfg.z0 = 20;
        cfg.z1 = 20;
        assert!(moment_maps(&cube, &cfg).unwrap_err().to_string().contains("two channels"));
        let mut cfg = line_config();
        cfg.continuum = Some(((0, 5), (38, 45)));
        assert!(moment_maps(&cube, &cfg).unwrap_err().to_string().contains("continuum window"));

        let legacy = dir.path().join("legacy.fits");
        write_cube(&legacy, 4, 4, 8, &[("CRVAL3", "1.0"), ("CDELT3", "0.1")], |z, _, _| z as f32);
        let legacy_cube = LazyCube::open(legacy.to_str().unwrap()).unwrap();
        let mut cfg = line_config();
        cfg.z0 = 1;
        cfg.z1 = 6;
        cfg.continuum = None;
        cfg.rest_um = Some(1.3);
        let err = moment_maps(&legacy_cube, &cfg).unwrap_err().to_string();
        assert!(err.contains("not a spectral axis"), "{}", err);
    }
}
