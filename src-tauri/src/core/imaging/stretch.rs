use ndarray::Array2;
use rayon::prelude::*;

pub fn arcsinh_stretch(data: &Array2<f32>, factor: f32) -> Array2<f32> {
    if factor.abs() < 1e-10 {
        return data.clone();
    }

    let inv_denom = 1.0 / factor.asinh();
    let (rows, cols) = data.dim();
    let src = data.as_slice().expect("contiguous");

    let pixels: Vec<f32> = src
        .par_iter()
        .map(|&val| {
            if !val.is_finite() || val <= 0.0 {
                0.0
            } else {
                (val.clamp(0.0, 1.0) * factor).asinh() * inv_denom
            }
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
    let (ro, (go, bo)) = rayon::join(
        || arcsinh_stretch(r, factor),
        || {
            rayon::join(
                || arcsinh_stretch(g, factor),
                || arcsinh_stretch(b, factor),
            )
        },
    );
    (ro, go, bo)
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

    #[test]
    fn test_arcsinh_stretch_rgb_consistency() {
        let ch = Array2::from_shape_fn((10, 10), |(r, c)| (r + c) as f32 / 20.0);
        let single = arcsinh_stretch(&ch, 20.0);
        let (ro, go, bo) = arcsinh_stretch_rgb(&ch, &ch, &ch, 20.0);
        assert_eq!(single, ro);
        assert_eq!(single, go);
        assert_eq!(single, bo);
    }
}
