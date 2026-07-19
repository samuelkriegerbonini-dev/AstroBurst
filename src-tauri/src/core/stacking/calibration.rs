use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::math::median::f32_cmp;
pub(crate) use crate::infra::fits::reader::load_fits_image;

pub struct CalibrationConfig {
    pub master_bias: Option<Array2<f32>>,
    pub master_dark: Option<Array2<f32>>,
    pub master_flat: Option<Array2<f32>>,
    pub dark_exposure_ratio: f32,
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
    let img_slice = image.as_slice().expect("contiguous");
    let flat_slice = master_flat.as_slice().expect("contiguous");

    let result: Vec<f32> = img_slice
        .par_iter()
        .zip(flat_slice.par_iter())
        .map(|(&iv, &fv)| {
            if fv.is_finite() && fv.abs() > 1e-4 {
                iv / fv
            } else {
                iv
            }
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result).unwrap()
}

fn ensure_master_dims(
    name: &str,
    master: Option<&Array2<f32>>,
    rows: usize,
    cols: usize,
) -> Result<()> {
    if let Some(m) = master {
        if m.dim() != (rows, cols) {
            bail!(
                "master {} shape {:?} does not match science frame {:?}",
                name,
                m.dim(),
                (rows, cols)
            );
        }
    }
    Ok(())
}

pub fn calibrate_image(raw: &Array2<f32>, config: &CalibrationConfig) -> Result<Array2<f32>> {
    let (rows, cols) = raw.dim();
    ensure_master_dims("bias", config.master_bias.as_ref(), rows, cols)?;
    ensure_master_dims("dark", config.master_dark.as_ref(), rows, cols)?;
    ensure_master_dims("flat", config.master_flat.as_ref(), rows, cols)?;
    let npix = rows * cols;
    let src = raw.as_slice().expect("contiguous");

    let bias_slice = config.master_bias.as_ref().and_then(|b| b.as_slice());
    let dark_slice = config.master_dark.as_ref().and_then(|d| d.as_slice());
    let flat_slice = config.master_flat.as_ref().and_then(|f| f.as_slice());
    let dark_ratio = config.dark_exposure_ratio;

    let result: Vec<f32> = (0..npix)
        .into_par_iter()
        .map(|i| {
            let mut v = src[i];

            if let Some(bias) = bias_slice {
                v -= bias[i];
            }

            if let Some(dark) = dark_slice {
                v -= dark[i] * dark_ratio;
            }

            if let Some(flat) = flat_slice {
                let fv = flat[i];
                if fv.is_finite() && fv.abs() > 1e-4 {
                    v /= fv;
                }
            }

            v
        })
        .collect();

    Ok(Array2::from_shape_vec((rows, cols), result).unwrap())
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

fn median_exposure_seconds(paths: &[String]) -> Option<f64> {
    let mut vals: Vec<f64> = paths
        .iter()
        .filter_map(|p| {
            let header = crate::infra::fits::reader::read_primary_header(p).ok()?;
            header
                .get_f64("EXPTIME")
                .or_else(|| header.get_f64("EXPOSURE"))
                .filter(|v| v.is_finite() && *v > 0.0)
        })
        .collect();
    if vals.is_empty() {
        return None;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(vals[vals.len() / 2])
}

fn flat_dark_scale(flat_exposure: Option<f64>, dark_exposure: Option<f64>) -> f32 {
    match (flat_exposure, dark_exposure) {
        (Some(flat), Some(dark)) if dark > 0.0 && flat.is_finite() && flat >= 0.0 => {
            ((flat / dark) as f32).clamp(0.0, 20.0)
        }
        _ => 1.0,
    }
}

pub fn create_master_flat(
    flat_paths: &[String],
    master_bias: Option<&Array2<f32>>,
    master_dark: Option<&Array2<f32>>,
    dark_exposure_seconds: Option<f64>,
) -> Result<Array2<f32>> {
    if flat_paths.is_empty() {
        bail!("No flat frames provided");
    }

    let dark_scale = if master_dark.is_some() && master_bias.is_some() {
        flat_dark_scale(median_exposure_seconds(flat_paths), dark_exposure_seconds)
    } else {
        1.0
    };

    let first = load_fits_image(&flat_paths[0])?;
    let (rows, cols) = first.dim();

    let preprocess = |mut frame: Array2<f32>| -> Array2<f32> {
        if let Some(bias) = master_bias {
            frame = subtract_bias(&frame, bias);
        }
        if let Some(dark) = master_dark {
            frame = subtract_dark(&frame, dark, dark_scale);
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

    let mut positives: Vec<f32> = result
        .iter()
        .filter(|v| v.is_finite() && **v > 0.0)
        .copied()
        .collect();

    if !positives.is_empty() {
        let mid = positives.len() / 2;
        positives.select_nth_unstable_by(mid, |a, b| f32_cmp(a, b));
        let median = positives[mid] as f64;
        let inv_median = if median.abs() > 1e-10 { 1.0 / median as f32 } else { 1.0 };

        result.par_iter_mut().for_each(|v| {
            if v.is_finite() && *v > 0.0 {
                *v *= inv_median;
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
        Some(paths) if !paths.is_empty() => {
            let dark_exposure = dark_paths.and_then(median_exposure_seconds);
            Some(create_master_flat(
                paths,
                master_bias.as_ref(),
                master_dark.as_ref(),
                dark_exposure,
            )?)
        }
        _ => None,
    };

    let config = CalibrationConfig {
        master_bias,
        master_dark,
        master_flat,
        dark_exposure_ratio,
    };

    calibrate_image(&science, &config)
}

pub fn stack_from_paths(
    paths: &[String],
    config: &crate::types::stacking::StackConfig,
    calibration: Option<&CalibrationConfig>,
) -> Result<crate::types::stacking::StackResult> {
    if paths.is_empty() {
        bail!("No image paths provided");
    }

    let images: Vec<Array2<f32>> = paths
        .iter()
        .map(|path| {
            let img = load_fits_image(path)?;
            match calibration {
                Some(cal) => calibrate_image(&img, cal),
                None => Ok(img),
            }
        })
        .collect::<Result<_>>()?;

    let result = crate::core::stacking::combine::stack_images(&images, config)?;
    Ok(result)
}

pub fn drizzle_from_paths(
    paths: &[String],
    config: &crate::types::stacking::DrizzleConfig,
    calibration: Option<&CalibrationConfig>,
) -> Result<crate::types::stacking::DrizzleResult> {
    if paths.is_empty() {
        bail!("No image paths provided");
    }

    let mut images: Vec<Array2<f32>> = Vec::with_capacity(paths.len());
    for path in paths {
        let mut img = load_fits_image(path)?;
        if let Some(cal) = calibration {
            img = calibrate_image(&img, cal)?;
        }
        images.push(img);
    }

    let result = crate::core::stacking::drizzle::drizzle_stack(&images, config)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_dark_scale_scales_by_exposure_ratio() {
        let s = flat_dark_scale(Some(5.0), Some(300.0));
        assert!((s - (5.0 / 300.0)).abs() < 1e-6, "got {}", s);
    }

    #[test]
    fn flat_dark_scale_identity_on_equal_exposure() {
        assert!((flat_dark_scale(Some(300.0), Some(300.0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn flat_dark_scale_falls_back_to_one_when_unknown() {
        assert_eq!(flat_dark_scale(None, Some(300.0)), 1.0);
        assert_eq!(flat_dark_scale(Some(5.0), None), 1.0);
        assert_eq!(flat_dark_scale(Some(5.0), Some(0.0)), 1.0);
    }

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
        assert!((result[[1, 0]] - 200.0).abs() < 1e-4);
        assert!((result[[1, 1]] - 200.0).abs() < 1e-4);
    }

    #[test]
    fn test_divide_flat_zero_safe() {
        let image =
            Array2::from_shape_vec((2, 2), vec![100.0, 200.0, 300.0, 400.0]).unwrap();
        let flat =
            Array2::from_shape_vec((2, 2), vec![0.0, 1.0, f32::NAN, 2.0]).unwrap();
        let result = divide_flat(&image, &flat);
        assert!((result[[0, 0]] - 100.0).abs() < 1e-4);
        assert!(result[[0, 1]].is_finite());
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

        let result = calibrate_image(&raw, &config).unwrap();
        assert!((result[[0, 0]] - 95.0).abs() < 1e-4);
        assert!((result[[2, 2]] - 175.0).abs() < 1e-4);
    }

    #[test]
    fn test_calibrate_rejects_mismatched_master() {
        let raw = Array2::from_elem((4, 4), 100.0f32);
        let config = CalibrationConfig {
            master_bias: Some(Array2::from_elem((3, 3), 10.0f32)),
            master_dark: None,
            master_flat: None,
            dark_exposure_ratio: 1.0,
        };
        assert!(calibrate_image(&raw, &config).is_err());
    }
}
