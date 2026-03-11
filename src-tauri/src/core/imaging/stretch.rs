use ndarray::Array2;
use rayon::prelude::*;

pub fn arcsinh_stretch(data: &Array2<f32>, factor: f32) -> Array2<f32> {
    if factor.abs() < 1e-10 {
        return data.clone();
    }

    let inv_denom = 1.0 / factor.asinh();
    let (rows, cols) = data.dim();

    let pixels: Vec<f32> = data
        .as_slice()
        .expect("contiguous")
        .par_iter()
        .map(|&v| {
            if !v.is_finite() || v <= 0.0 {
                return 0.0;
            }
            let clamped = v.clamp(0.0, 1.0);
            (clamped * factor).asinh() * inv_denom
        })
        .collect();

    Array2::from_shape_vec((rows, cols), pixels).unwrap()
}

pub fn arcsinh_stretch_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    factor: f32,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    (
        arcsinh_stretch(r, factor),
        arcsinh_stretch(g, factor),
        arcsinh_stretch(b, factor),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arcsinh_stretch_boundaries() {
        let data = Array2::from_shape_vec((1, 3), vec![0.0, 0.5, 1.0]).unwrap();
        let result = arcsinh_stretch(&data, 10.0);
        assert!((result[[0, 0]]).abs() < 1e-6);
        assert!((result[[0, 2]] - 1.0).abs() < 1e-4);
        assert!(result[[0, 1]] > 0.5);
    }

    #[test]
    fn test_arcsinh_stretch_monotonic() {
        let data = Array2::from_shape_fn((1, 100), |(_, x)| (x + 1) as f32 / 100.0);
        let result = arcsinh_stretch(&data, 50.0);
        for x in 1..100 {
            assert!(result[[0, x]] >= result[[0, x - 1]]);
        }
    }

    #[test]
    fn test_arcsinh_stretch_factor_effect() {
        let data = Array2::from_shape_vec((1, 1), vec![0.1]).unwrap();
        let mild = arcsinh_stretch(&data, 5.0);
        let strong = arcsinh_stretch(&data, 100.0);
        assert!(strong[[0, 0]] > mild[[0, 0]]);
    }

    #[test]
    fn test_arcsinh_stretch_zero_factor() {
        let data = Array2::from_shape_vec((2, 2), vec![0.1, 0.5, 0.8, 1.0]).unwrap();
        let result = arcsinh_stretch(&data, 0.0);
        assert_eq!(result, data);
    }

    #[test]
    fn test_arcsinh_stretch_nan_safe() {
        let data = Array2::from_shape_vec((1, 3), vec![f32::NAN, -0.5, 0.5]).unwrap();
        let result = arcsinh_stretch(&data, 10.0);
        assert_eq!(result[[0, 0]], 0.0);
        assert_eq!(result[[0, 1]], 0.0);
        assert!(result[[0, 2]] > 0.0);
    }
}
