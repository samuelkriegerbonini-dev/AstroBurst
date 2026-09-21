use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use ndarray::{Array2, Array3};
use rayon::prelude::*;

use crate::core::astrometry::spectral::spectral_axis;
use crate::math::simd::collapse_mean_simd;
use crate::math::median::f32_cmp;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::header::HduHeader;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CubeResult {
    pub dimensions: [usize; 3],
    pub collapsed_path: String,
    pub collapsed_median_path: String,
    pub frames_dir: String,
    pub frame_count: usize,
    pub center_spectrum: Vec<f32>,
    pub wavelengths: Option<Vec<f64>>,
    pub elapsed_ms: u64,
}

pub fn collapse_mean(cube: &Array3<f32>) -> Array2<f32> {
    collapse_mean_simd(cube)
}

pub fn collapse_median(cube: &Array3<f32>) -> Array2<f32> {
    let (depth, rows, cols) = cube.dim();
    let npix = rows * cols;

    let result_data: Vec<f32> = (0..npix)
        .into_par_iter()
        .map(|i| {
            let y = i / cols;
            let x = i % cols;
            let mut vals: Vec<f32> = (0..depth)
                .map(|z| cube[[z, y, x]])
                .filter(|v| v.is_finite() && *v != 0.0)
                .collect();

            if vals.is_empty() {
                return 0.0;
            }

            let mid = vals.len() / 2;
            vals.select_nth_unstable_by(mid, |a, b| {
                f32_cmp(a, b)
            });
            vals[mid]
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result_data).unwrap()
}

pub fn extract_spectrum(cube: &Array3<f32>, y: usize, x: usize) -> Vec<f32> {
    let depth = cube.dim().0;
    (0..depth).map(|z| cube[[z, y, x]]).collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpectralClassification {
    pub is_spectral: bool,
    pub reason: String,
    pub axis_type: Option<String>,
    pub axis_unit: Option<String>,
    pub axis_unit_assumed: bool,
    pub channel_count: usize,
}

fn assumed_axis_unit(header: &HduHeader, naxis3: usize) -> Option<String> {
    spectral_axis(header, naxis3)
        .ok()
        .filter(|axis| axis.kind.is_spectral() && !axis.header_unit.is_empty())
        .map(|axis| axis.header_unit.to_uppercase())
}

pub fn classify_spectral_cube(header: &HduHeader, naxis3: usize) -> SpectralClassification {
    let mut classification = classify_spectral_cube_from_cards(header, naxis3);
    if classification.is_spectral && classification.axis_unit.is_none() {
        if let Some(unit) = assumed_axis_unit(header, naxis3) {
            classification.axis_unit = Some(unit);
            classification.axis_unit_assumed = true;
        }
    }
    classification
}

fn classify_spectral_cube_from_cards(header: &HduHeader, naxis3: usize) -> SpectralClassification {
    let ctype3 = header.get("CTYPE3").map(|s| s.trim().trim_matches('\'').trim().to_uppercase());
    let cunit3 = header.get("CUNIT3").map(|s| s.trim().trim_matches('\'').trim().to_uppercase());
    let has_cdelt3 = header.get_f64("CDELT3").is_some();
    let has_crval3 = header.get_f64("CRVAL3").is_some();

    let spectral_ctypes = ["WAVE", "FREQ", "VELO", "AWAV", "VRAD", "VOPT", "ZOPT", "BETA", "ENER"];
    let spectral_units = ["M", "CM", "MM", "UM", "NM", "ANGSTROM", "A", "HZ", "KHZ", "MHZ", "GHZ", "M/S", "KM/S", "EV", "KEV"];

    let ctype_is_spectral = ctype3.as_ref().map_or(false, |ct| {
        spectral_ctypes.iter().any(|&s| ct.contains(s))
    });

    let cunit_is_spectral = cunit3.as_ref().map_or(false, |cu| {
        spectral_units.iter().any(|&s| cu == s || cu.contains(s))
    });

    if ctype_is_spectral {
        return SpectralClassification {
            is_spectral: true,
            reason: format!("CTYPE3 indicates spectral axis: {}", ctype3.as_deref().unwrap_or("")),
            axis_type: ctype3,
            axis_unit: cunit3,
            axis_unit_assumed: false,
            channel_count: naxis3,
        };
    }

    if cunit_is_spectral && has_cdelt3 {
        return SpectralClassification {
            is_spectral: true,
            reason: format!("CUNIT3 indicates spectral data: {}", cunit3.as_deref().unwrap_or("")),
            axis_type: ctype3,
            axis_unit: cunit3,
            axis_unit_assumed: false,
            channel_count: naxis3,
        };
    }

    if naxis3 <= 4 {
        return SpectralClassification {
            is_spectral: false,
            reason: format!("NAXIS3={} with no spectral keywords: likely RGB/RGBA composition", naxis3),
            axis_type: ctype3,
            axis_unit: cunit3,
            axis_unit_assumed: false,
            channel_count: naxis3,
        };
    }

    if has_cdelt3 && has_crval3 {
        return SpectralClassification {
            is_spectral: true,
            reason: format!("NAXIS3={} with CRVAL3/CDELT3 present: likely spectral cube", naxis3),
            axis_type: ctype3,
            axis_unit: cunit3,
            axis_unit_assumed: false,
            channel_count: naxis3,
        };
    }

    if naxis3 > 10 {
        return SpectralClassification {
            is_spectral: true,
            reason: format!("NAXIS3={}: high channel count suggests spectral data", naxis3),
            axis_type: ctype3,
            axis_unit: cunit3,
            axis_unit_assumed: false,
            channel_count: naxis3,
        };
    }

    SpectralClassification {
        is_spectral: false,
        reason: format!("NAXIS3={} with no spectral metadata: ambiguous, treating as non-spectral", naxis3),
        axis_type: ctype3,
        axis_unit: cunit3,
        axis_unit_assumed: false,
        channel_count: naxis3,
    }
}

pub fn build_wavelength_axis(header: &HduHeader) -> Option<Vec<f64>> {
    let naxis3 = header.get_i64("NAXIS3").filter(|n| *n > 0)? as usize;
    spectral_axis(header, naxis3).ok().map(|axis| axis.header_values())
}

#[derive(Debug, Clone)]
pub struct GlobalCubeStats {
    pub median: f32,
    pub sigma: f32,
    pub low: f32,
    pub high: f32,
}

pub fn compute_global_stats(cube: &Array3<f32>) -> GlobalCubeStats {
    let mut finite: Vec<f32> = cube
        .iter()
        .filter(|v| v.is_finite() && **v != 0.0)
        .copied()
        .collect();

    if finite.is_empty() {
        return GlobalCubeStats {
            median: 0.0,
            sigma: 1.0,
            low: 0.0,
            high: 1.0,
        };
    }

    let n = finite.len();
    let mid = n / 2;
    finite.select_nth_unstable_by(mid, |a, b| {
        f32_cmp(a, b)
    });
    let median = finite[mid];

    let mut deviations: Vec<f32> = finite.iter().map(|v| (v - median).abs()).collect();
    let dev_mid = deviations.len() / 2;
    deviations.select_nth_unstable_by(dev_mid, |a, b| {
        f32_cmp(a, b)
    });
    let sigma = (deviations[dev_mid] * MAD_TO_SIGMA as f32).max(1e-10);

    let low_idx = (n as f64 * 0.01) as usize;
    let high_idx = ((n as f64 * 0.999) as usize).min(n - 1);
    finite.select_nth_unstable_by(low_idx, |a, b| f32_cmp(a, b));
    let low = finite[low_idx];
    finite.select_nth_unstable_by(high_idx, |a, b| f32_cmp(a, b));
    let high = finite[high_idx];

    GlobalCubeStats {
        median,
        sigma,
        low,
        high,
    }
}

pub fn normalize_with_global(data: &Array2<f32>, g: &GlobalCubeStats) -> Array2<f32> {
    let alpha: f32 = 10.0;
    let inv_sigma_alpha = alpha / g.sigma;
    let lo = (inv_sigma_alpha * (g.low - g.median)).asinh();
    let hi = (inv_sigma_alpha * (g.high - g.median)).asinh();
    let inv = 1.0 / (hi - lo).max(1e-6);

    data.mapv(|v| {
        if !v.is_finite() {
            return 0.0;
        }
        let clamped = v.clamp(g.low, g.high);
        let scaled = inv_sigma_alpha * (clamped - g.median);
        ((scaled.asinh() - lo) * inv).clamp(1e-6, 1.0)
    })
}

pub fn export_cube_frames_sampled(
    cube: &Array3<f32>,
    output_dir: &str,
    step: usize,
) -> Result<usize> {
    let depth = cube.dim().0;
    let step = step.max(1);
    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create frames dir {}", output_dir))?;

    let global = compute_global_stats(cube);

    let indices: Vec<(usize, usize)> = (0..depth).step_by(step).enumerate().collect();

    indices.par_iter().try_for_each(|&(count, z)| -> Result<()> {
        let slice = cube.index_axis(ndarray::Axis(0), z).to_owned();
        let normalized = normalize_with_global(&slice, &global);
        let path = format!("{}/frame_{:04}.png", output_dir, count);
        crate::infra::render::render_grayscale(&normalized, &path)
    })?;

    Ok(indices.len())
}

pub fn process_cube(
    input_path: &str,
    output_dir: &str,
    frame_step: usize,
) -> Result<CubeResult> {
    use crate::core::imaging::normalize::robust_asinh_preview;
    use crate::infra::fits::reader::extract_cube_mmap;
    use crate::infra::render::render_grayscale;
    use std::fs::File;

    let t0 = std::time::Instant::now();

    let (actual_fits_path, _tmp_holder) = if input_path.to_lowercase().ends_with(".zip") {
        let resolved = crate::infra::fits::dispatcher::resolve_input(std::path::Path::new(input_path))
            .with_context(|| format!("Failed to resolve ZIP input {}", input_path))?;
        match resolved {
            crate::infra::fits::dispatcher::ResolvedInput::ExtractedFromZip { files, _tmp } => {
                let first = files
                    .into_iter()
                    .next()
                    .context("No .fits in ZIP")?;
                (first, Some(_tmp))
            }
            _ => unreachable!(),
        }
    } else {
        (PathBuf::from(input_path), None)
    };

    let file = File::open(&actual_fits_path)
        .with_context(|| format!("Failed to open FITS {:?}", actual_fits_path))?;
    let result = extract_cube_mmap(&file)
        .context("mmap cube extraction failed")?;

    let cube = result.cube;
    let header = result.header;
    let (depth, rows, cols) = cube.dim();

    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output dir {}", output_dir))?;

    let collapsed = collapse_mean(&cube);
    let collapsed_norm = robust_asinh_preview(&collapsed);
    let collapsed_path = format!("{}/collapsed_mean.png", output_dir);
    render_grayscale(&collapsed_norm, &collapsed_path)?;

    let collapsed_med = collapse_median(&cube);
    let collapsed_med_norm = robust_asinh_preview(&collapsed_med);
    let collapsed_med_path = format!("{}/collapsed_median.png", output_dir);
    render_grayscale(&collapsed_med_norm, &collapsed_med_path)?;

    let center_y = rows / 2;
    let center_x = cols / 2;
    let spectrum = extract_spectrum(&cube, center_y, center_x);
    let wavelengths = build_wavelength_axis(&header);

    let frames_dir = format!("{}/frames", output_dir);
    let frame_count = export_cube_frames_sampled(&cube, &frames_dir, frame_step)?;

    Ok(CubeResult {
        dimensions: [cols, rows, depth],
        collapsed_path,
        collapsed_median_path: collapsed_med_path,
        frames_dir,
        frame_count,
        center_spectrum: spectrum,
        wavelengths,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::make_header;
    use crate::types::constants::PADDING_THRESHOLD;

    #[test]
    fn normalize_with_global_keeps_pixels_below_median_visible() {
        let g = GlobalCubeStats { median: 100.0, sigma: 5.0, low: 88.0, high: 600.0 };
        let data = Array2::from_shape_vec(
            (2, 3),
            vec![88.0, 99.0, 100.0, 100.01, 600.0, f32::NAN],
        )
        .unwrap();
        let out = normalize_with_global(&data, &g);

        assert_eq!(out[[1, 2]], 0.0);
        for &v in out.iter().take(5) {
            assert!(v > PADDING_THRESHOLD && v <= 1.0, "{}", v);
        }
        assert!(out[[0, 0]] < out[[0, 1]]);
        assert!(out[[0, 1]] < out[[0, 2]]);
        assert!(out[[0, 2]] < out[[1, 0]]);
        assert!(out[[1, 0]] < out[[1, 1]]);
        assert!((out[[1, 1]] - 1.0).abs() < 1e-6);
        assert!(out[[0, 2]] > 0.3 && out[[0, 2]] < 0.4, "{}", out[[0, 2]]);
    }

    #[test]
    fn normalize_with_global_degenerate_range_stays_finite() {
        let g = GlobalCubeStats { median: 5.0, sigma: 1e-10, low: 5.0, high: 5.0 };
        let data = Array2::from_shape_vec((1, 2), vec![5.0, 7.0]).unwrap();
        let out = normalize_with_global(&data, &g);
        for &v in out.iter() {
            assert!(v.is_finite() && v > PADDING_THRESHOLD && v <= 1.0, "{}", v);
        }
    }

    #[test]
    fn wavelength_axis_reads_cd3_3_and_pc3_3_and_keeps_header_units() {
        let muse = make_header(&[
            ("NAXIS3", "3"),
            ("CTYPE3", "AWAV"),
            ("CUNIT3", "Angstrom"),
            ("CRVAL3", "4750.0"),
            ("CD3_3", "1.25"),
            ("CRPIX3", "1.0"),
        ]);
        assert_eq!(build_wavelength_axis(&muse).unwrap(), vec![4750.0, 4751.25, 4752.5]);
        let pc = make_header(&[
            ("NAXIS3", "2"),
            ("CTYPE3", "FREQ"),
            ("CUNIT3", "Hz"),
            ("CRVAL3", "2.3e11"),
            ("CDELT3", "1.0e6"),
            ("PC3_3", "2.0"),
            ("CRPIX3", "1.0"),
        ]);
        let axis = build_wavelength_axis(&pc).unwrap();
        assert!((axis[1] - 2.30002e11).abs() < 1.0, "{:?}", axis);
    }

    #[test]
    fn wavelength_axis_keeps_the_legacy_contract_and_drops_non_linear_axes() {
        let legacy = make_header(&[("NAXIS3", "4"), ("CRVAL3", "10.0"), ("CDELT3", "2.0"), ("CRPIX3", "2.0")]);
        assert_eq!(build_wavelength_axis(&legacy).unwrap(), vec![8.0, 10.0, 12.0, 14.0]);
        let log = make_header(&[("NAXIS3", "4"), ("CTYPE3", "WAVE-LOG"), ("CRVAL3", "1.0"), ("CDELT3", "0.1")]);
        assert!(build_wavelength_axis(&log).is_none());
        let no_depth = make_header(&[("CRVAL3", "1.0"), ("CDELT3", "0.1")]);
        assert!(build_wavelength_axis(&no_depth).is_none());
        let no_step = make_header(&[("NAXIS3", "4"), ("CRVAL3", "1.0")]);
        assert!(build_wavelength_axis(&no_step).is_none());
    }

    #[test]
    fn classification_assumes_the_fits_default_unit_when_cunit3_is_missing() {
        let bare = make_header(&[("CTYPE3", "WAVE"), ("CRVAL3", "4.7e-7"), ("CDELT3", "1.25e-10")]);
        let c = classify_spectral_cube(&bare, 3000);
        assert!(c.is_spectral);
        assert_eq!(c.axis_unit.as_deref(), Some("M"));
        assert!(c.axis_unit_assumed);
        let explicit = make_header(&[("CTYPE3", "WAVE"), ("CUNIT3", "um"), ("CRVAL3", "1.0"), ("CDELT3", "0.01")]);
        let c = classify_spectral_cube(&explicit, 3000);
        assert_eq!(c.axis_unit.as_deref(), Some("UM"));
        assert!(!c.axis_unit_assumed);
        let no_axis = make_header(&[("CTYPE3", "WAVE")]);
        let c = classify_spectral_cube(&no_axis, 3000);
        assert!(c.is_spectral);
        assert!(c.axis_unit.is_none());
        assert!(!c.axis_unit_assumed);
    }
}
