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
pub fn nearest_sample(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> f32 {
    if rows == 0 || cols == 0 || slice.is_empty() {
        return 0.0;
    }
    let iy = clamp_index(y.round() as i64, rows);
    let ix = clamp_index(x.round() as i64, cols);
    slice[iy * cols + ix]
}

#[inline]
pub fn bilinear_sample(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> f32 {
    if rows == 0 || cols == 0 || slice.is_empty() {
        return 0.0;
    }
    let ix0 = x.floor() as i64;
    let iy0 = y.floor() as i64;
    let fx = x - ix0 as f64;
    let fy = y - iy0 as f64;

    let r0 = clamp_index(iy0, rows);
    let r1 = clamp_index(iy0 + 1, rows);
    let c0 = clamp_index(ix0, cols);
    let c1 = clamp_index(ix0 + 1, cols);

    let v00 = slice[r0 * cols + c0] as f64;
    let v01 = slice[r0 * cols + c1] as f64;
    let v10 = slice[r1 * cols + c0] as f64;
    let v11 = slice[r1 * cols + c1] as f64;

    let top = v00 + (v01 - v00) * fx;
    let bot = v10 + (v11 - v10) * fx;
    (top + (bot - top) * fy) as f32
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

    let mut val = 0.0f64;
    for j in 0..4i64 {
        let r = clamp_index(iy + j - 1, rows);
        let row_off = r * cols;
        let mut row_val = 0.0f64;
        for i in 0..4i64 {
            let c = clamp_index(ix + i - 1, cols);
            row_val += slice[row_off + c] as f64 * wx[i as usize];
        }
        val += row_val * catmull_rom(fy - (j - 1) as f64);
    }

    val as f32
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
    fn test_nearest_center() {
        let data = vec![1.0, 2.0, 3.0, 4.0];
        assert!((nearest_sample(&data, 2, 2, 0.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((nearest_sample(&data, 2, 2, 0.0, 0.6) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_nearest_empty() {
        assert!((nearest_sample(&[], 0, 0, 0.0, 0.0)).abs() < 1e-6);
    }

    #[test]
    fn test_bilinear_center() {
        let data = vec![0.0, 10.0, 0.0, 10.0];
        let v = bilinear_sample(&data, 2, 2, 0.0, 0.5);
        assert!((v - 5.0).abs() < 1e-4);
    }

    #[test]
    fn test_bilinear_empty() {
        assert!((bilinear_sample(&[], 0, 0, 1.0, 1.0)).abs() < 1e-6);
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
}
