use std::fs::{self, File};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ndarray::{Array2, Array3};
use rayon::prelude::*;

use crate::domain::normalize::asinh_normalize;
use crate::model::HduHeader;
use crate::utils::mmap::extract_cube_mmap;
use crate::utils::render::render_grayscale;
use crate::utils::simd::collapse_mean_simd;

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
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
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

pub fn build_wavelength_axis(header: &HduHeader) -> Option<Vec<f64>> {
    let crval3 = header.get_f64("CRVAL3")?;
    let cdelt3 = header.get_f64("CDELT3")?;
    let crpix3 = header.get_f64("CRPIX3").unwrap_or(1.0);
    let naxis3 = header.get_i64("NAXIS3")? as usize;

    let axis: Vec<f64> = (0..naxis3)
        .map(|i| crval3 + (i as f64 - crpix3 + 1.0) * cdelt3)
        .collect();

    Some(axis)
}

struct GlobalStats {
    median: f32,
    sigma: f32,
    low: f32,
    high: f32,
}

fn compute_global_stats(cube: &Array3<f32>) -> GlobalStats {
    let mut finite: Vec<f32> = cube
        .iter()
        .filter(|v| v.is_finite() && **v != 0.0)
        .copied()
        .collect();

    if finite.is_empty() {
        return GlobalStats {
            median: 0.0,
            sigma: 1.0,
            low: 0.0,
            high: 1.0,
        };
    }

    let n = finite.len();
    let mid = n / 2;
    finite.select_nth_unstable_by(mid, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    let median = finite[mid];

    let mut deviations: Vec<f32> = finite.iter().map(|v| (v - median).abs()).collect();
    let dev_mid = deviations.len() / 2;
    deviations.select_nth_unstable_by(dev_mid, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    let sigma = (deviations[dev_mid] * 1.4826).max(1e-10);

    finite.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let low = finite[(n as f64 * 0.01) as usize];
    let high = finite[((n as f64 * 0.999) as usize).min(n - 1)];

    GlobalStats {
        median,
        sigma,
        low,
        high,
    }
}

fn normalize_with_global(data: &Array2<f32>, g: &GlobalStats) -> Array2<f32> {
    let alpha: f32 = 10.0;
    let inv_sigma_alpha = alpha / g.sigma;

    data.mapv(|v| {
        if !v.is_finite() {
            return 0.0;
        }
        let clamped = v.clamp(g.low, g.high);
        let scaled = inv_sigma_alpha * (clamped - g.median);
        scaled.asinh()
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
        render_grayscale(&normalized, &path)
    })?;

    Ok(indices.len())
}

pub fn process_cube(
    input_path: &str,
    output_dir: &str,
    frame_step: usize,
) -> Result<CubeResult> {
    let (actual_fits_path, _tmp_holder) = if input_path.to_lowercase().ends_with(".zip") {
        let resolved = crate::utils::dispatcher::resolve_input(std::path::Path::new(input_path))
            .with_context(|| format!("Failed to resolve ZIP input {}", input_path))?;
        match resolved {
            crate::utils::dispatcher::ResolvedInput::ExtractedFromZip { files, _tmp } => {
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
    let collapsed_norm = asinh_normalize(&collapsed);
    let collapsed_path = format!("{}/collapsed_mean.png", output_dir);
    render_grayscale(&collapsed_norm, &collapsed_path)?;

    let collapsed_med = collapse_median(&cube);
    let collapsed_med_norm = asinh_normalize(&collapsed_med);
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
    })
}

#[derive(Debug, Clone)]
pub struct CubeResult {
    pub dimensions: [usize; 3],
    pub collapsed_path: String,
    pub collapsed_median_path: String,
    pub frames_dir: String,
    pub frame_count: usize,
    pub center_spectrum: Vec<f32>,
    pub wavelengths: Option<Vec<f64>>,
}
