use super::fft::FftFloat;

#[derive(Debug, Clone, Copy)]
pub struct SubpixelShift<T: FftFloat> {
    pub dx: T,
    pub dy: T,
}

fn axis_neighbors<T: FftFloat>(
    surface: &[T],
    rows: usize,
    cols: usize,
    peak_y: usize,
    peak_x: usize,
    axis_y: bool,
) -> (T, T, T) {
    if axis_y {
        let py = if peak_y == 0 { rows - 1 } else { peak_y - 1 };
        let ny = if peak_y == rows - 1 { 0 } else { peak_y + 1 };
        (
            surface[peak_y * cols + peak_x],
            surface[py * cols + peak_x],
            surface[ny * cols + peak_x],
        )
    } else {
        let px = if peak_x == 0 { cols - 1 } else { peak_x - 1 };
        let nx = if peak_x == cols - 1 { 0 } else { peak_x + 1 };
        (
            surface[peak_y * cols + peak_x],
            surface[peak_y * cols + px],
            surface[peak_y * cols + nx],
        )
    }
}

pub fn foroosh_refine_1d<T: FftFloat>(
    surface: &[T],
    rows: usize,
    cols: usize,
    peak_y: usize,
    peak_x: usize,
    axis_y: bool,
) -> T {
    let (center, prev, next) = axis_neighbors(surface, rows, cols, peak_y, peak_x, axis_y);

    let zero = T::zero();
    let half = T::half();
    let toward_next = next >= prev;
    let side = if toward_next { next } else { prev };
    let denom = center + side;
    if denom.abs_val() < T::epsilon_val() {
        return zero;
    }
    let magnitude = side / denom;
    let signed = if toward_next { magnitude } else { zero - magnitude };
    signed.max_of(zero - half).min_of(half)
}

pub fn foroosh_refine_2d<T: FftFloat>(
    surface: &[T],
    rows: usize,
    cols: usize,
    peak_y: usize,
    peak_x: usize,
) -> SubpixelShift<T> {
    SubpixelShift {
        dy: foroosh_refine_1d(surface, rows, cols, peak_y, peak_x, true),
        dx: foroosh_refine_1d(surface, rows, cols, peak_y, peak_x, false),
    }
}

pub fn unwrap_circular_peak<T: FftFloat>(peak: usize, fft_size: usize) -> T {
    if peak > fft_size / 2 {
        <T as FftFloat>::from_usize(peak) - <T as FftFloat>::from_usize(fft_size)
    } else {
        <T as FftFloat>::from_usize(peak)
    }
}

pub fn unwrap_and_refine<T: FftFloat>(
    surface: &[T],
    fft_rows: usize,
    fft_cols: usize,
    peak_y: usize,
    peak_x: usize,
) -> SubpixelShift<T> {
    let raw_dy = unwrap_circular_peak::<T>(peak_y, fft_rows);
    let raw_dx = unwrap_circular_peak::<T>(peak_x, fft_cols);

    let sub = foroosh_refine_2d(surface, fft_rows, fft_cols, peak_y, peak_x);

    SubpixelShift {
        dy: raw_dy + sub.dy,
        dx: raw_dx + sub.dx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foroosh(prev: f64, center: f64, next: f64) -> f64 {
        foroosh_refine_1d(&[prev, center, next], 1, 3, 0, 1, false)
    }

    #[test]
    fn test_unwrap_circular_peak_no_wrap() {
        let result: f64 = unwrap_circular_peak(5, 64);
        assert!((result - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_unwrap_circular_peak_wrap() {
        let result: f64 = unwrap_circular_peak(60, 64);
        assert!((result - (-4.0)).abs() < 1e-10);
    }

    #[test]
    fn test_unwrap_and_refine_centered() {
        let size = 16;
        let mut surface = vec![0.0f64; size * size];
        surface[0] = 1.0;
        surface[1] = 0.5;
        surface[size] = 0.5;
        let result = unwrap_and_refine(&surface, size, size, 0, 0);
        assert!(result.dx.abs() < 0.5);
        assert!(result.dy.abs() < 0.5);
    }

    #[test]
    fn test_foroosh_refine_2d_keeps_axes_apart() {
        let size = 8;
        let mut surface = vec![0.0f64; size * size];
        surface[3 * size + 3] = 1.0;
        surface[3 * size + 2] = 0.6;
        surface[3 * size + 4] = 0.4;
        surface[2 * size + 3] = 0.3;
        surface[4 * size + 3] = 0.7;
        let result = foroosh_refine_2d(&surface, size, size, 3, 3);
        assert!((result.dx - (-0.6 / 1.6)).abs() < 1e-12, "dx {}", result.dx);
        assert!((result.dy - 0.7 / 1.7).abs() < 1e-12, "dy {}", result.dy);
    }

    #[test]
    fn test_f32_subpixel() {
        let mut surface = vec![0.0f32; 16];
        surface[5] = 0.0;
        surface[6] = 1.0;
        surface[7] = 1.0 / 3.0;
        let result = foroosh_refine_1d(&surface, 4, 4, 1, 2, false);
        assert!((result - 0.25).abs() < 1e-6);
    }

    #[test]
    fn test_wrap_around_y() {
        let size = 8;
        let mut surface = vec![0.0f64; size * size];
        surface[3] = 1.0;
        surface[(size - 1) * size + 3] = 0.6;
        surface[size + 3] = 0.4;
        let result = foroosh_refine_1d(&surface, size, size, 0, 3, true);
        assert!((result - (-0.6 / 1.6)).abs() < 1e-12, "{result}");
    }

    #[test]
    fn test_wrap_around_x() {
        let size = 8;
        let mut surface = vec![0.0f64; size * size];
        surface[3 * size] = 1.0;
        surface[3 * size + (size - 1)] = 0.6;
        surface[3 * size + 1] = 0.4;
        let result = foroosh_refine_1d(&surface, size, size, 3, 0, false);
        assert!((result - (-0.6 / 1.6)).abs() < 1e-12, "{result}");
    }

    #[test]
    fn test_foroosh_integer_peak() {
        assert!(foroosh(0.0, 1.0, 0.0).abs() < 1e-12);
    }

    #[test]
    fn test_foroosh_half_pixel() {
        assert!((foroosh(0.0, 1.0, 1.0) - 0.5).abs() < 1e-12);
        assert!((foroosh(1.0, 1.0, 0.0) - (-0.5)).abs() < 1e-12);
    }

    #[test]
    fn test_foroosh_quarter_pixel() {
        let third = 1.0 / 3.0;
        assert!((foroosh(0.0, 1.0, third) - 0.25).abs() < 1e-12);
        assert!((foroosh(third, 1.0, 0.0) - (-0.25)).abs() < 1e-12);
    }

    #[test]
    fn test_foroosh_direction_toward_larger_neighbor() {
        assert!(foroosh(0.2, 1.0, 0.6) > 0.0);
        assert!(foroosh(0.6, 1.0, 0.2) < 0.0);
    }

    #[test]
    fn test_foroosh_clamped_and_bounded() {
        let r = foroosh(0.0, 1.0, 1000.0);
        assert!((-0.5..=0.5).contains(&r));
        let r2 = foroosh(1000.0, 1.0, 0.0);
        assert!((-0.5..=0.5).contains(&r2));
    }

    #[test]
    fn test_foroosh_degenerate_denom() {
        assert!(foroosh(0.0, 0.0, 0.0).abs() < 1e-12);
    }

    fn dirichlet_pc_surface_1d(n: usize, d: f64) -> Vec<f64> {
        let nn = n as f64;
        let mut spectrum_re = vec![0.0f64; n];
        let mut spectrum_im = vec![0.0f64; n];
        for k in 0..n {
            let freq = if k > n / 2 { k as f64 - nn } else { k as f64 };
            let ang = -2.0 * std::f64::consts::PI * freq * d / nn;
            spectrum_re[k] = ang.cos();
            spectrum_im[k] = ang.sin();
        }
        spectrum_re[0] = 1.0;
        spectrum_im[0] = 0.0;
        let mut surface = vec![0.0f64; n];
        for sample in 0..n {
            let mut acc = 0.0f64;
            for k in 0..n {
                let ang = 2.0 * std::f64::consts::PI * (k as f64) * (sample as f64) / nn;
                acc += spectrum_re[k] * ang.cos() - spectrum_im[k] * ang.sin();
            }
            surface[sample] = acc / nn;
        }
        surface
    }

    #[test]
    fn test_foroosh_recovers_dirichlet_peak_shift() {
        let n = 64;
        for &d in &[0.1f64, 0.2, 0.25, 0.3, 0.35, -0.2, -0.35] {
            let surface = dirichlet_pc_surface_1d(n, d);
            let mut peak = 0usize;
            for i in 1..n {
                if surface[i] > surface[peak] {
                    peak = i;
                }
            }
            let raw = if peak > n / 2 { peak as f64 - n as f64 } else { peak as f64 };

            let foroosh = raw + foroosh_refine_1d(&surface, 1, n, 0, peak, false);
            let foroosh_err = (foroosh - d).abs();

            assert!(
                foroosh_err < 0.03,
                "foroosh error {:.4} too large at d={}",
                foroosh_err,
                d
            );
        }
    }
}
