use ndarray::{Array2, Zip};

use crate::core::imaging::stats::compute_image_stats;

pub fn arcsinh_stretch(data: &Array2<f32>, factor: f32) -> Array2<f32> {
    let stats = compute_image_stats(data);
    arcsinh_stretch_with_stats(data, stats.min as f32, stats.max as f32, factor, 1.0)
}

pub fn arcsinh_stretch_with_stats(
    data: &Array2<f32>,
    dmin: f32,
    dmax: f32,
    factor: f32,
    gamma: f32,
) -> Array2<f32> {
    if factor.abs() < 1e-10 {
        return data.clone();
    }

    let range = dmax - dmin;
    if range < 1e-10 {
        return Array2::zeros(data.raw_dim());
    }

    let inv_range = 1.0 / range;
    let inv_denom = 1.0 / factor.asinh();
    let apply_gamma = (gamma - 1.0).abs() > 1e-6;

    let mut result = Array2::zeros(data.raw_dim());

    Zip::from(&mut result)
        .and(data)
        .par_for_each(|out, &val| {
            if !val.is_finite() {
                *out = 0.0;
            } else {
                let norm = ((val - dmin) * inv_range).clamp(0.0, 1.0);
                let stretched = (norm * factor).asinh() * inv_denom;
                *out = if apply_gamma { stretched.powf(gamma) } else { stretched };
            }
        });

    result
}

pub fn arcsinh_stretch_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    factor: f32,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    arcsinh_stretch_rgb_with_stats(r, g, b, None, None, factor, 1.0)
}

pub fn arcsinh_stretch_rgb_with_stats(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    global_min: Option<f32>,
    global_max: Option<f32>,
    factor: f32,
    gamma: f32,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    if factor.abs() < 1e-10 {
        return (r.clone(), g.clone(), b.clone());
    }

    let (gmin, gmax) = match (global_min, global_max) {
        (Some(mn), Some(mx)) => (mn, mx),
        _ => {
            let sr = compute_image_stats(r);
            let sg = compute_image_stats(g);
            let sb = compute_image_stats(b);
            (
                (sr.min as f32).min(sg.min as f32).min(sb.min as f32),
                (sr.max as f32).max(sg.max as f32).max(sb.max as f32),
            )
        }
    };

    let (ro, (go, bo)) = rayon::join(
        || arcsinh_stretch_with_stats(r, gmin, gmax, factor, gamma),
        || rayon::join(
            || arcsinh_stretch_with_stats(g, gmin, gmax, factor, gamma),
            || arcsinh_stretch_with_stats(b, gmin, gmax, factor, gamma),
        ),
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
        assert!(result[[0, 2]] > 0.0);
    }

    #[test]
    fn test_arcsinh_stretch_negative_values_not_destroyed() {
        let data = Array2::from_shape_vec((1, 4), vec![-0.1, 0.0, 0.5, 1.0]).unwrap();
        let result = arcsinh_stretch(&data, 10.0);
        assert!((result[[0, 0]]).abs() < 1e-6);
        assert!(result[[0, 2]] > 0.0);
        assert!((result[[0, 3]] - 1.0).abs() < 1e-4);
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

    #[test]
    fn test_arcsinh_stretch_values_above_one() {
        let data = Array2::from_shape_vec((1, 4), vec![0.0, 0.5, 1.5, 3.0]).unwrap();
        let result = arcsinh_stretch(&data, 50.0);
        assert!((result[[0, 0]]).abs() < 1e-6);
        assert!((result[[0, 3]] - 1.0).abs() < 1e-4);
        assert!(result[[0, 1]] < result[[0, 2]]);
        assert!(result[[0, 2]] < result[[0, 3]]);
    }

    #[test]
    fn test_arcsinh_stretch_rgb_global_norm() {
        let r = Array2::from_shape_vec((1, 2), vec![0.5, 2.0]).unwrap();
        let g = Array2::from_shape_vec((1, 2), vec![0.3, 1.0]).unwrap();
        let b = Array2::from_shape_vec((1, 2), vec![0.1, 0.5]).unwrap();
        let (ro, go, bo) = arcsinh_stretch_rgb(&r, &g, &b, 20.0);
        assert!(ro[[0, 1]] > go[[0, 1]]);
        assert!(go[[0, 1]] > bo[[0, 1]]);
        for ch in [&ro, &go, &bo] {
            for &v in ch.iter() {
                assert!(v >= 0.0 && v <= 1.0);
            }
        }
    }

    #[test]
    fn test_arcsinh_stretch_gamma() {
        let data = Array2::from_shape_vec((1, 3), vec![0.0, 0.5, 1.0]).unwrap();
        let no_gamma = arcsinh_stretch_with_stats(&data, 0.0, 1.0, 10.0, 1.0);
        let with_gamma = arcsinh_stretch_with_stats(&data, 0.0, 1.0, 10.0, 0.5);
        assert!(with_gamma[[0, 1]] > no_gamma[[0, 1]]);
    }
}
