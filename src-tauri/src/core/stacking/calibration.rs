use ndarray::Array2;
use rayon::prelude::*;

use crate::math::median::median_f32_mut;

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

    let mut finite_vals: Vec<f32> = flat_slice
        .iter()
        .filter(|v| v.is_finite() && v.abs() > 1e-4)
        .copied()
        .collect();
    let flat_median = if finite_vals.is_empty() {
        1.0f32
    } else {
        median_f32_mut(&mut finite_vals)
    };
    let inv_median = if flat_median.abs() > 1e-10 { 1.0 / flat_median } else { 1.0 };

    let result: Vec<f32> = img_slice
        .par_iter()
        .zip(flat_slice.par_iter())
        .map(|(&iv, &fv)| {
            let fv_norm = fv * inv_median;
            if fv_norm.is_finite() && fv_norm.abs() > 1e-4 {
                iv / fv_norm
            } else {
                iv
            }
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result).unwrap()
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

    calibrated.mapv_inplace(|v| if v < 0.0 { 0.0 } else { v });
    calibrated
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
        let expected_00 = 100.0 / (0.5 / 1.25);
        let expected_01 = 200.0 / (1.0 / 1.25);
        assert!((result[[0, 0]] - expected_00).abs() < 1.0);
        assert!((result[[0, 1]] - expected_01).abs() < 1.0);
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

        let result = calibrate_image(&raw, &config);
        assert!((result[[0, 0]] - 95.0).abs() < 1e-4);
        assert!((result[[2, 2]] - 175.0).abs() < 1e-4);
    }
}
