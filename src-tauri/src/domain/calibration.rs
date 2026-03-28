use std::fs::File;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;
use crate::math::median::f32_cmp;

use crate::infra::fits::reader::extract_image_mmap;

pub use crate::core::stacking::calibration::{
    CalibrationConfig, subtract_bias, subtract_dark, calibrate_image,
};

pub fn load_fits_image(path: &str) -> Result<Array2<f32>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open {}", path))?;
    let result = extract_image_mmap(&file)
        .with_context(|| format!("Failed to load {}", path))?;
    Ok(result.image)
}

fn median_combine_row_major(
    frames: Vec<Array2<f32>>,
    rows: usize,
    cols: usize,
) -> Vec<f32> {
    let n = frames.len();
    let npix = rows * cols;

    let slices: Vec<&[f32]> = frames
        .iter()
        .map(|f| f.as_slice().expect("contiguous"))
        .collect();

    let mut result = vec![0.0f32; npix];

    result
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(y, row_buf)| {
            let mut vals = Vec::with_capacity(n);
            let base = y * cols;
            for x in 0..cols {
                vals.clear();
                let idx = base + x;
                for s in &slices {
                    let v = s[idx];
                    if v.is_finite() {
                        vals.push(v);
                    }
                }
                if vals.is_empty() {
                    row_buf[x] = 0.0;
                } else {
                    let mid = vals.len() / 2;
                    vals.select_nth_unstable_by(mid, |a, b| f32_cmp(a, b));
                    row_buf[x] = vals[mid];
                }
            }
        });

    result
}

pub fn create_master_bias(bias_paths: &[String]) -> Result<Array2<f32>> {
    if bias_paths.is_empty() {
        bail!("No bias frames provided");
    }

    let first = load_fits_image(&bias_paths[0])?;
    let (rows, cols) = first.dim();

    let mut frames = Vec::with_capacity(bias_paths.len());
    frames.push(first);

    for path in &bias_paths[1..] {
        let frame = load_fits_image(path)?;
        if frame.dim() != (rows, cols) {
            bail!(
                "Dimension mismatch: expected ({}, {}), got {:?}",
                rows, cols, frame.dim()
            );
        }
        frames.push(frame);
    }

    let result = median_combine_row_major(frames, rows, cols);

    Ok(Array2::from_shape_vec((rows, cols), result)
        .context("Failed to reshape master bias")?)
}

pub fn create_master_dark(
    dark_paths: &[String],
    master_bias: Option<&Array2<f32>>,
) -> Result<Array2<f32>> {
    if dark_paths.is_empty() {
        bail!("No dark frames provided");
    }

    let first = load_fits_image(&dark_paths[0])?;
    let (rows, cols) = first.dim();

    let first = match master_bias {
        Some(bias) => subtract_bias(&first, bias),
        None => first,
    };

    let mut frames = Vec::with_capacity(dark_paths.len());
    frames.push(first);

    for path in &dark_paths[1..] {
        let mut frame = load_fits_image(path)?;
        if frame.dim() != (rows, cols) {
            bail!(
                "Dimension mismatch: expected ({}, {}), got {:?}",
                rows, cols, frame.dim()
            );
        }
        if let Some(bias) = master_bias {
            frame = subtract_bias(&frame, bias);
        }
        frames.push(frame);
    }

    let result = median_combine_row_major(frames, rows, cols);

    Ok(Array2::from_shape_vec((rows, cols), result)
        .context("Failed to reshape master dark")?)
}

pub fn create_master_flat(
    flat_paths: &[String],
    master_bias: Option<&Array2<f32>>,
    master_dark: Option<&Array2<f32>>,
) -> Result<Array2<f32>> {
    if flat_paths.is_empty() {
        bail!("No flat frames provided");
    }

    let first = load_fits_image(&flat_paths[0])?;
    let (rows, cols) = first.dim();

    let preprocess = |mut frame: Array2<f32>| -> Array2<f32> {
        if let Some(bias) = master_bias {
            frame = subtract_bias(&frame, bias);
        }
        if let Some(dark) = master_dark {
            frame = subtract_dark(&frame, dark, 1.0);
        }
        frame
    };

    let mut frames = Vec::with_capacity(flat_paths.len());
    frames.push(preprocess(first));

    for path in &flat_paths[1..] {
        let frame = load_fits_image(path)?;
        if frame.dim() != (rows, cols) {
            bail!(
                "Dimension mismatch: expected ({}, {}), got {:?}",
                rows, cols, frame.dim()
            );
        }
        frames.push(preprocess(frame));
    }

    let mut result = median_combine_row_major(frames, rows, cols);

    let sum: f64 = result.iter()
        .filter(|v| v.is_finite() && **v > 0.0)
        .map(|v| *v as f64)
        .sum();
    let count = result.iter()
        .filter(|v| v.is_finite() && **v > 0.0)
        .count();

    if count > 0 {
        let mean = sum / count as f64;
        let inv_mean = if mean.abs() > 1e-10 { 1.0 / mean as f32 } else { 1.0 };

        result.par_iter_mut().for_each(|v| {
            if v.is_finite() && *v > 0.0 {
                *v *= inv_mean;
            } else {
                *v = 1.0;
            }
        });
    }

    Ok(Array2::from_shape_vec((rows, cols), result)
        .context("Failed to reshape normalized master flat")?)
}

pub fn calibrate_from_paths(
    science_path: &str,
    bias_paths: Option<&[String]>,
    dark_paths: Option<&[String]>,
    flat_paths: Option<&[String]>,
    dark_exposure_ratio: f32,
) -> Result<Array2<f32>> {
    let science = load_fits_image(science_path)?;

    let master_bias = match bias_paths {
        Some(paths) if !paths.is_empty() => Some(create_master_bias(paths)?),
        _ => None,
    };

    let master_dark = match dark_paths {
        Some(paths) if !paths.is_empty() => {
            Some(create_master_dark(paths, master_bias.as_ref())?)
        }
        _ => None,
    };

    let master_flat = match flat_paths {
        Some(paths) if !paths.is_empty() => Some(create_master_flat(
            paths,
            master_bias.as_ref(),
            master_dark.as_ref(),
        )?),
        _ => None,
    };

    let config = CalibrationConfig {
        master_bias,
        master_dark,
        master_flat,
        dark_exposure_ratio,
    };

    Ok(calibrate_image(&science, &config))
}
