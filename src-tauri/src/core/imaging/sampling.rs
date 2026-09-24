use crate::core::imaging::boundary::clamp_index;

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
    if rows == 0 || cols == 0 || slice.is_empty() {
        return 0.0;
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
        }
        val += row_val * wy[j as usize];
    }

    if support_finite {
        return val as f32;
    }

    bilinear_finite(slice, rows, cols, iy, ix, fx, fy)
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
fn bilinear_finite(
    slice: &[f32],
    rows: usize,
    cols: usize,
    iy: i64,
    ix: i64,
    fx: f64,
    fy: f64,
) -> f32 {
    let r0 = clamp_index(iy, rows);
    let r1 = clamp_index(iy + 1, rows);
    let c0 = clamp_index(ix, cols);
    let c1 = clamp_index(ix + 1, cols);

    let taps = [
        (slice[r0 * cols + c0] as f64, (1.0 - fx) * (1.0 - fy)),
        (slice[r0 * cols + c1] as f64, fx * (1.0 - fy)),
        (slice[r1 * cols + c0] as f64, (1.0 - fx) * fy),
        (slice[r1 * cols + c1] as f64, fx * fy),
    ];

    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for (value, weight) in taps {
        if value.is_finite() {
            num += value * weight;
            den += weight;
        }
    }

    if den > 0.0 {
        (num / den) as f32
    } else {
        f32::NAN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
