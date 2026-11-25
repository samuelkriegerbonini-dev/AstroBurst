use crate::core::imaging::boundary::clamp_index;

pub(crate) struct SampledPixel {
    pub value: f32,
    taps: [(usize, f64); 16],
    count: usize,
}

impl SampledPixel {
    fn empty(value: f32) -> Self {
        Self { value, taps: [(0, 0.0); 16], count: 0 }
    }

    fn add(&mut self, index: usize, weight: f64) {
        if weight == 0.0 {
            return;
        }
        if let Some((_, total)) = self.taps[..self.count].iter_mut().find(|(i, _)| *i == index) {
            *total += weight;
        } else {
            self.taps[self.count] = (index, weight);
            self.count += 1;
        }
    }

    pub fn weights(&self) -> impl Iterator<Item = (usize, f64)> + '_ {
        self.taps[..self.count].iter().copied().filter(|(_, weight)| *weight != 0.0)
    }
}

#[inline]
pub fn catmull_rom(t: f64) -> f64 {
    let abs_t = t.abs();
    if abs_t <= 1.0 {
        abs_t * abs_t * (1.5 * abs_t - 2.5) + 1.0
    } else if abs_t <= 2.0 {
        abs_t * (abs_t * (2.5 - 0.5 * abs_t) - 4.0) + 2.0
    } else {
        0.0
    }
}

#[inline]
pub fn bicubic_sample(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> f32 {
    bicubic_sampled::<false>(slice, rows, cols, y, x).value
}

pub(crate) fn bicubic_with_weights(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> SampledPixel {
    bicubic_sampled::<true>(slice, rows, cols, y, x)
}

fn bicubic_sampled<const TRACK_WEIGHTS: bool>(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> SampledPixel {
    if rows == 0 || cols == 0 || slice.is_empty() {
        return SampledPixel::empty(0.0);
    }
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - ix as f64;
    let fy = y - iy as f64;

    let wx = [
        catmull_rom(fx + 1.0),
        catmull_rom(fx),
        catmull_rom(fx - 1.0),
        catmull_rom(fx - 2.0),
    ];
    let wy = [
        catmull_rom(fy + 1.0),
        catmull_rom(fy),
        catmull_rom(fy - 1.0),
        catmull_rom(fy - 2.0),
    ];

    let mut val = 0.0f64;
    let mut result = SampledPixel::empty(f32::NAN);
    let mut support_finite = true;
    for j in 0..4i64 {
        let r = clamp_index(iy + j - 1, rows);
        let row_off = r * cols;
        let mut row_val = 0.0f64;
        for i in 0..4i64 {
            let c = clamp_index(ix + i - 1, cols);
            let sample = slice[row_off + c] as f64;
            support_finite &= sample.is_finite();
            row_val += sample * wx[i as usize];
            if TRACK_WEIGHTS {
                result.add(row_off + c, wx[i as usize] * wy[j as usize]);
            }
        }
        val += row_val * wy[j as usize];
    }

    if support_finite {
        result.value = val as f32;
        return result;
    }

    bilinear_finite::<TRACK_WEIGHTS>(slice, rows, cols, iy, ix, fx, fy)
}

#[inline]
pub fn bilinear_sample(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> f32 {
    bilinear_sampled::<false>(slice, rows, cols, y, x).value
}

pub(crate) fn bilinear_with_weights(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> SampledPixel {
    bilinear_sampled::<true>(slice, rows, cols, y, x)
}

fn bilinear_sampled<const TRACK_WEIGHTS: bool>(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> SampledPixel {
    if rows == 0 || cols == 0 || slice.is_empty() {
        return SampledPixel::empty(f32::NAN);
    }
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - ix as f64;
    let fy = y - iy as f64;
    let r0 = clamp_index(iy, rows);
    let r1 = clamp_index(iy + 1, rows);
    let c0 = clamp_index(ix, cols);
    let c1 = clamp_index(ix + 1, cols);
    let taps = [
        (r0 * cols + c0, (1.0 - fx) * (1.0 - fy)),
        (r0 * cols + c1, fx * (1.0 - fy)),
        (r1 * cols + c0, (1.0 - fx) * fy),
        (r1 * cols + c1, fx * fy),
    ];
    let mut value = 0.0f64;
    let mut result = SampledPixel::empty(f32::NAN);
    for (index, weight) in taps {
        if weight != 0.0 {
            value += slice[index] as f64 * weight;
            if TRACK_WEIGHTS {
                result.add(index, weight);
            }
        }
    }
    result.value = value as f32;
    result
}

pub fn preview_dims(rows: usize, cols: usize, max_dim: usize) -> (usize, usize) {
    if rows <= max_dim && cols <= max_dim {
        return (rows, cols);
    }
    let scale = max_dim as f64 / (rows.max(cols) as f64);
    let dst_rows = ((rows as f64) * scale).round().max(1.0) as usize;
    let dst_cols = ((cols as f64) * scale).round().max(1.0) as usize;
    (dst_rows, dst_cols)
}

#[inline]
pub fn cell_range(d: usize, src: usize, dst: usize) -> (usize, usize) {
    let start = d * src / dst;
    let end = (((d + 1) * src) / dst).max(start + 1).min(src);
    (start, end)
}

#[inline]
fn bilinear_finite<const TRACK_WEIGHTS: bool>(
    slice: &[f32],
    rows: usize,
    cols: usize,
    iy: i64,
    ix: i64,
    fx: f64,
    fy: f64,
) -> SampledPixel {
    let r0 = clamp_index(iy, rows);
    let r1 = clamp_index(iy + 1, rows);
    let c0 = clamp_index(ix, cols);
    let c1 = clamp_index(ix + 1, cols);

    let taps = [
        (r0 * cols + c0, (1.0 - fx) * (1.0 - fy)),
        (r0 * cols + c1, fx * (1.0 - fy)),
        (r1 * cols + c0, (1.0 - fx) * fy),
        (r1 * cols + c1, fx * fy),
    ];

    let mut num = 0.0f64;
    let mut den = 0.0f64;
    let mut result = SampledPixel::empty(f32::NAN);
    for (index, weight) in taps {
        let value = slice[index] as f64;
        if value.is_finite() {
            num += value * weight;
            den += weight;
            if TRACK_WEIGHTS {
                result.add(index, weight);
            }
        }
    }

    if den > 0.0 {
        result.value = (num / den) as f32;
        for (_, weight) in &mut result.taps[..result.count] {
            *weight /= den;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scientific_weights_merge_repeated_boundary_pixels_before_variance() {
        for sampled in [bilinear_with_weights(&[7.0], 1, 1, -0.3, 0.2), bicubic_with_weights(&[7.0], 1, 1, -0.3, 0.2)] {
            let weights: Vec<_> = sampled.weights().collect();
            assert_eq!(weights.len(), 1);
            assert_eq!(weights[0].0, 0);
            assert!((weights[0].1 - 1.0).abs() < 1e-14);
            let variance = weights.iter().map(|(_, weight)| weight * weight * 9.0).sum::<f64>();
            assert!((variance - 9.0).abs() < 1e-12);
        }
    }

    #[test]
    fn scientific_weights_follow_cubic_negative_lobes_and_finite_fallback() {
        let mut data = vec![2.0; 16];
        let cubic = bicubic_with_weights(&data, 4, 4, 1.5, 1.5);
        assert_eq!(cubic.weights().count(), 16);
        assert!(cubic.weights().any(|(_, weight)| weight < 0.0));
        let variance = cubic.weights().map(|(_, weight)| weight * weight * 4.0).sum::<f64>();
        assert_eq!(variance, 4.0 * (41.0f64 / 64.0).powi(2));
        data[0] = f32::NAN;
        let fallback = bicubic_with_weights(&data, 4, 4, 1.5, 1.5);
        assert_eq!(fallback.weights().collect::<Vec<_>>(), [(5, 0.25), (6, 0.25), (9, 0.25), (10, 0.25)]);
        data[5] = f32::NAN;
        let partial = bicubic_with_weights(&data, 4, 4, 1.5, 1.5);
        assert_eq!(partial.weights().count(), 3);
        assert!(partial.weights().all(|(index, weight)| index != 5 && (weight - 1.0 / 3.0).abs() < 1e-15));
        assert_eq!(partial.value, bicubic_sample(&data, 4, 4, 1.5, 1.5));
        assert!(bilinear_with_weights(&data, 4, 4, 1.5, 1.5).value.is_nan());
        let integer = bilinear_with_weights(&data, 4, 4, 2.0, 2.0);
        assert_eq!(integer.weights().collect::<Vec<_>>(), [(10, 1.0)]);
    }

    #[test]
    fn scientific_resampling_variance_matches_independent_noise_and_outputs_are_correlated() {
        use ndarray::Array2;
        use rand::{Rng, SeedableRng};
        use rand::rngs::StdRng;

        let mut rng = StdRng::seed_from_u64(20261102);
        let n = 2048usize;
        let mut moments = [[0.0, 0.0]; 3];
        let (mut adjacent_sum, mut cross_sum) = (0.0, 0.0);
        for _ in 0..n {
            let image = Array2::from_shape_fn((8, 8), |_| {
                let u1 = rng.gen::<f64>().max(f64::MIN_POSITIVE);
                let u2 = rng.gen::<f64>();
                (2.0 * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
            });
            let data = image.as_slice().unwrap();
            let reduced = crate::core::imaging::resample::box_reduce(image.view(), 4, 4).unwrap();
            let values = [
                bilinear_sample(data, 8, 8, 3.5, 3.5),
                bicubic_sample(data, 8, 8, 3.5, 3.5),
                bicubic_sample(reduced.as_slice().unwrap(), 4, 4, 1.5, 1.5),
            ];
            for (moment, value) in moments.iter_mut().zip(values) {
                moment[0] += f64::from(value);
                moment[1] += f64::from(value).powi(2);
            }
            let adjacent = f64::from(bicubic_sample(data, 8, 8, 3.5, 4.5));
            adjacent_sum += adjacent;
            cross_sum += f64::from(values[1]) * adjacent;
        }
        let n = n as f64;
        let six_standard_errors = 6.0 * (2.0 / (n - 1.0)).sqrt();
        for (moment, expected) in moments.iter().zip([1.0, 4.0 * (41.0f64 / 64.0).powi(2), (41.0f64 / 64.0).powi(2)]) {
            let observed = (moment[1] - moment[0].powi(2) / n) / (n - 1.0);
            assert!((observed / expected - 1.0).abs() < six_standard_errors, "variance {observed} vs {expected}");
        }
        let covariance = (cross_sum - moments[1][0] * adjacent_sum / n) / (n - 1.0);
        assert!(covariance > 0.2, "resampled pixels share input noise: covariance {covariance}");
    }

    #[test]
    fn test_catmull_rom_at_zero() {
        assert!((catmull_rom(0.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_at_one() {
        assert!(catmull_rom(1.0).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_symmetry() {
        assert!((catmull_rom(0.5) - catmull_rom(-0.5)).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_at_two() {
        assert!(catmull_rom(2.0).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_partition_of_unity() {
        for i in 0..=10 {
            let t = i as f64 / 10.0;
            let sum = catmull_rom(t + 1.0) + catmull_rom(t) + catmull_rom(t - 1.0) + catmull_rom(t - 2.0);
            assert!((sum - 1.0).abs() < 1e-10, "partition of unity failed at t={}: sum={}", t, sum);
        }
    }

    #[test]
    fn test_bicubic_on_integer() {
        let data: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let v = bicubic_sample(&data, 10, 10, 3.0, 4.0);
        assert!((v - 34.0).abs() < 1e-3);
    }

    #[test]
    fn test_bicubic_empty() {
        assert!((bicubic_sample(&[], 0, 0, 1.0, 1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_bicubic_constant_image() {
        let data = vec![42.0f32; 64];
        let v = bicubic_sample(&data, 8, 8, 3.5, 4.7);
        assert!((v - 42.0).abs() < 1e-3);
    }

    #[test]
    fn test_bicubic_single_nan_does_not_poison_neighborhood() {
        let mut data = vec![100.0f32; 64];
        data[4 * 8 + 4] = f32::NAN;
        let v = bicubic_sample(&data, 8, 8, 2.5, 2.5);
        assert!(v.is_finite(), "a distant NaN must not poison the sample");
        assert!((v - 100.0).abs() < 1e-3);
    }

    #[test]
    fn test_bicubic_nan_in_support_falls_back_to_finite_bilinear() {
        let mut data = vec![100.0f32; 64];
        data[3 * 8 + 3] = f32::NAN;
        let v = bicubic_sample(&data, 8, 8, 3.5, 3.5);
        assert!(v.is_finite(), "fallback must skip the NaN tap, got {}", v);
        assert!((v - 100.0).abs() < 1e-3);
    }

    #[test]
    fn test_bicubic_all_nan_support_returns_nan() {
        let data = vec![f32::NAN; 64];
        let v = bicubic_sample(&data, 8, 8, 3.5, 4.5);
        assert!(v.is_nan());
    }

    #[test]
    fn test_bicubic_finite_image_unchanged_by_nan_path() {
        let data: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let v = bicubic_sample(&data, 10, 10, 3.0, 4.0);
        assert!((v - 34.0).abs() < 1e-3);
    }

    fn legacy_dims(rows: usize, cols: usize, max_dim: usize) -> (usize, usize) {
        if rows <= max_dim && cols <= max_dim {
            return (rows, cols);
        }
        let scale = max_dim as f64 / (rows.max(cols) as f64);
        (
            ((rows as f64) * scale).round().max(1.0) as usize,
            ((cols as f64) * scale).round().max(1.0) as usize,
        )
    }

    #[test]
    fn preview_dims_matches_legacy_formula() {
        assert_eq!(preview_dims(100, 200, 2048), (100, 200));
        assert_eq!(preview_dims(4096, 4096, 2048), (2048, 2048));
        assert_eq!(preview_dims(3000, 1000, 2048), (2048, 683));
        assert_eq!(preview_dims(1000, 3000, 2048), (683, 2048));
        assert_eq!(preview_dims(1, 100000, 16), (1, 16));
        for (r, c, m) in [(4096, 4096, 1024), (12345, 6789, 2048), (7, 9000, 512), (2049, 2048, 2048), (2048, 2048, 2048)] {
            assert_eq!(preview_dims(r, c, m), legacy_dims(r, c, m), "{r}x{c}@{m}");
        }
    }

    #[test]
    fn bilinear_sample_weights_four_taps_and_lets_a_weighted_nan_through() {
        let data: Vec<f32> = (0..20).map(|i| (3 * (i % 5) + 7 * (i / 5)) as f32).collect();
        let plane = |y: f64, x: f64| (3.0 * x + 7.0 * y) as f32;
        for (y, x) in [(1.25, 2.5), (0.0, 0.0), (2.9, 3.1), (3.0, 4.0), (0.5, 3.75)] {
            assert!((bilinear_sample(&data, 4, 5, y, x) - plane(y, x)).abs() < 1e-5, "({y}, {x})");
        }
        assert_eq!(bilinear_sample(&data, 4, 5, -0.4, -0.3), data[0]);
        assert_eq!(bilinear_sample(&data, 4, 5, 3.45, 4.2), data[19]);

        let mut holed = data.clone();
        holed[5 + 2] = f32::NAN;
        assert!(bilinear_sample(&holed, 4, 5, 1.5, 2.5).is_nan());
        assert!(bilinear_sample(&holed, 4, 5, 1.0, 1.5).is_nan());
        assert_eq!(bilinear_sample(&holed, 4, 5, 1.0, 1.0), data[5 + 1]);
        assert!((bilinear_sample(&holed, 4, 5, 2.0, 2.5) - plane(2.0, 2.5)).abs() < 1e-5);
        assert!(bilinear_sample(&[], 0, 0, 0.0, 0.0).is_nan());
    }

    #[test]
    fn cell_range_is_monotone_and_covers_each_source_index_once() {
        for (src, dst) in [(4, 2), (10, 3), (4096, 2048), (7, 7), (2048, 683), (5, 2)] {
            let mut covered = vec![0u32; src];
            let mut prev_end = 0;
            for d in 0..dst {
                let (s, e) = cell_range(d, src, dst);
                assert!(s < e, "{src}/{dst} cell {d}");
                assert_eq!(s, prev_end, "{src}/{dst} cell {d} must start where the previous ended");
                for i in s..e {
                    covered[i] += 1;
                }
                prev_end = e;
            }
            assert_eq!(prev_end, src, "{src}/{dst}");
            assert!(covered.iter().all(|&c| c == 1), "{src}/{dst}: {covered:?}");
        }
        assert_eq!(cell_range(0, 4, 2), (0, 2));
        assert_eq!(cell_range(1, 4, 2), (2, 4));
    }
}
