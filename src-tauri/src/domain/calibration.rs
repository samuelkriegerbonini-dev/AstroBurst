use std::fs::File;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::utils::mmap::extract_image_mmap;

fn load_fits_image(path: &str) -> Result<Array2<f32>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open {}", path))?;
    let result = extract_image_mmap(&file)
        .with_context(|| format!("Failed to load {}", path))?;
    Ok(result.image)
}

pub fn create_master_bias(bias_paths: &[String]) -> Result<Array2<f32>> {
    if bias_paths.is_empty() {
        bail!("No bias frames provided");
    }

    let first = load_fits_image(&bias_paths[0])?;
    let (rows, cols) = first.dim();
    let npix = rows * cols;
    let n = bias_paths.len();

    let mut columns: Vec<Vec<f32>> = vec![Vec::with_capacity(n); npix];
    for path in bias_paths {
        let frame = load_fits_image(path)?;
        if frame.dim() != (rows, cols) {
            bail!(
                "Dimension mismatch: expected ({}, {}), got {:?}",
                rows,
                cols,
                frame.dim()
            );
        }
        let slice = frame.as_slice().expect("contiguous");
        for i in 0..npix {
            if slice[i].is_finite() {
                columns[i].push(slice[i]);
            }
        }
    }

    let result: Vec<f32> = columns
        .into_par_iter()
        .map(|mut vals| {
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
    let npix = rows * cols;
    let n = dark_paths.len();

    let mut columns: Vec<Vec<f32>> = vec![Vec::with_capacity(n); npix];
    for path in dark_paths {
        let mut frame = load_fits_image(path)?;
        if frame.dim() != (rows, cols) {
            bail!(
                "Dimension mismatch: expected ({}, {}), got {:?}",
                rows,
                cols,
                frame.dim()
            );
        }
        if let Some(bias) = master_bias {
            frame = subtract_bias(&frame, bias);
        }
        let slice = frame.as_slice().expect("contiguous");
        for i in 0..npix {
            if slice[i].is_finite() {
                columns[i].push(slice[i]);
            }
        }
    }

    let result: Vec<f32> = columns
        .into_par_iter()
        .map(|mut vals| {
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
    let npix = rows * cols;
    let n = flat_paths.len();

    let mut columns: Vec<Vec<f32>> = vec![Vec::with_capacity(n); npix];
    for path in flat_paths {
        let mut frame = load_fits_image(path)?;
        if frame.dim() != (rows, cols) {
            bail!(
                "Dimension mismatch: expected ({}, {}), got {:?}",
                rows,
                cols,
                frame.dim()
            );
        }
        if let Some(bias) = master_bias {
            frame = subtract_bias(&frame, bias);
        }
        if let Some(dark) = master_dark {
            frame = subtract_dark(&frame, dark, 1.0);
        }
        let slice = frame.as_slice().expect("contiguous");
        for i in 0..npix {
            if slice[i].is_finite() {
                columns[i].push(slice[i]);
            }
        }
    }

    let mut result: Vec<f32> = columns
        .into_par_iter()
        .map(|mut vals| {
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

    let finite_vals: Vec<f32> = result
        .iter()
        .filter(|v| v.is_finite() && **v > 0.0)
        .copied()
        .collect();
    if finite_vals.is_empty() {
        return Ok(Array2::from_shape_vec((rows, cols), result)
            .context("Failed to reshape master flat")?);
    }

    let mean = finite_vals.iter().map(|v| *v as f64).sum::<f64>() / finite_vals.len() as f64;
    let inv_mean = if mean.abs() > 1e-10 {
        1.0 / mean as f32
    } else {
        1.0
    };

    for v in &mut result {
        if v.is_finite() && *v > 0.0 {
            *v *= inv_mean;
        } else {
            *v = 1.0;
        }
    }

    Ok(Array2::from_shape_vec((rows, cols), result)
        .context("Failed to reshape normalized master flat")?)
}

pub fn subtract_bias(image: &Array2<f32>, master_bias: &Array2<f32>) -> Array2<f32> {
    image - master_bias
}

pub fn subtract_dark(
    image: &Array2<f32>,
    master_dark: &Array2<f32>,
    exposure_ratio: f32,
) -> Array2<f32> {
    image - &(master_dark * exposure_ratio)
}

pub fn divide_flat(image: &Array2<f32>, master_flat: &Array2<f32>) -> Array2<f32> {
    let (rows, cols) = image.dim();
    let mut result = Array2::<f32>::zeros((rows, cols));
    for y in 0..rows {
        for x in 0..cols {
            let flat_val = master_flat[[y, x]];
            result[[y, x]] = if flat_val.is_finite() && flat_val.abs() > 0.01 {
                image[[y, x]] / flat_val
            } else {
                image[[y, x]]
            };
        }
    }
    result
}

pub struct CalibrationConfig {
    pub master_bias: Option<Array2<f32>>,
    pub master_dark: Option<Array2<f32>>,
    pub master_flat: Option<Array2<f32>>,
    pub dark_exposure_ratio: f32,
}

pub fn calibrate_image(raw: &Array2<f32>, config: &CalibrationConfig) -> Array2<f32> {
    let mut calibrated = raw.clone();

    if let Some(ref bias) = config.master_bias {
        calibrated = subtract_bias(&calibrated, bias);
    }
    if let Some(ref dark) = config.master_dark {
        calibrated = subtract_dark(&calibrated, dark, config.dark_exposure_ratio);
    }
    if let Some(ref flat) = config.master_flat {
        calibrated = divide_flat(&calibrated, flat);
    }

    calibrated
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtract_bias() {
        let image =
            Array2::from_shape_vec((2, 2), vec![110.0, 120.0, 130.0, 140.0]).unwrap();
        let bias = Array2::from_shape_vec((2, 2), vec![10.0, 10.0, 10.0, 10.0]).unwrap();
        let result = subtract_bias(&image, &bias);
        assert!((result[[0, 0]] - 100.0).abs() < 1e-6);
        assert!((result[[1, 1]] - 130.0).abs() < 1e-6);
    }

    #[test]
    fn test_subtract_dark_with_ratio() {
        let image =
            Array2::from_shape_vec((2, 2), vec![200.0, 200.0, 200.0, 200.0]).unwrap();
        let dark = Array2::from_shape_vec((2, 2), vec![20.0, 20.0, 20.0, 20.0]).unwrap();
        let result = subtract_dark(&image, &dark, 2.0);
        assert!((result[[0, 0]] - 160.0).abs() < 1e-6);
    }

    #[test]
    fn test_divide_flat() {
        let image =
            Array2::from_shape_vec((2, 2), vec![100.0, 200.0, 300.0, 400.0]).unwrap();
        let flat = Array2::from_shape_vec((2, 2), vec![0.5, 1.0, 1.5, 2.0]).unwrap();
        let result = divide_flat(&image, &flat);
        assert!((result[[0, 0]] - 200.0).abs() < 1e-4);
        assert!((result[[0, 1]] - 200.0).abs() < 1e-4);
    }

    #[test]
    fn test_divide_flat_zero_safe() {
        let image =
            Array2::from_shape_vec((2, 2), vec![100.0, 200.0, 300.0, 400.0]).unwrap();
        let flat =
            Array2::from_shape_vec((2, 2), vec![0.0, 1.0, f32::NAN, 2.0]).unwrap();
        let result = divide_flat(&image, &flat);
        assert!((result[[0, 0]] - 100.0).abs() < 1e-4);
        assert!((result[[0, 1]] - 200.0).abs() < 1e-4);
        assert!((result[[1, 0]] - 300.0).abs() < 1e-4);
    }

    #[test]
    fn test_full_calibration_pipeline() {
        let raw = Array2::from_shape_vec(
            (3, 3),
            vec![
                110.0, 120.0, 130.0, 140.0, 150.0, 160.0, 170.0, 180.0, 190.0,
            ],
        )
        .unwrap();
        let bias = Array2::from_shape_vec((3, 3), vec![10.0; 9]).unwrap();
        let dark = Array2::from_shape_vec((3, 3), vec![5.0; 9]).unwrap();
        let flat = Array2::from_shape_vec((3, 3), vec![1.0; 9]).unwrap();

        let config = CalibrationConfig {
            master_bias: Some(bias),
            master_dark: Some(dark),
            master_flat: Some(flat),
            dark_exposure_ratio: 1.0,
        };

        let result = calibrate_image(&raw, &config);
        assert!((result[[0, 0]] - 95.0).abs() < 1e-4);
        assert!((result[[2, 2]] - 175.0).abs() < 1e-4);
    }
}
