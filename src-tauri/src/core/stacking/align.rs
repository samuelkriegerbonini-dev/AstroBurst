use ndarray::Array2;

use crate::core::alignment::affine;
use crate::core::alignment::phase_correlation;
use crate::types::compose::AlignMethod;

pub use crate::core::alignment::pair::{
    align_pair, align_pair_with_label, shift_image_subpixel, AlignPairResult,
};

#[derive(Debug, Clone, Copy)]
pub struct OffsetEstimate {
    pub dy: f64,
    pub dx: f64,
    pub confidence: f64,
}

pub fn estimate_offset(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    method: AlignMethod,
) -> OffsetEstimate {
    match method {
        AlignMethod::PhaseCorrelation => {
            let pc = phase_correlation::phase_correlate(reference, target);
            OffsetEstimate {
                dy: pc.dy,
                dx: pc.dx,
                confidence: pc.confidence,
            }
        }
        AlignMethod::Affine => {
            let result = affine::align_channel_affine(reference, target);
            OffsetEstimate {
                dy: result.transform.ty,
                dx: result.transform.tx,
                confidence: result.confidence,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pattern(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            let h = (y.wrapping_mul(73856093) ^ x.wrapping_mul(19349663)) & 0xffff;
            let texture = (h as f32 / 65535.0 - 0.5) * 400.0;
            ((y as f32 * 0.3).sin() * (x as f32 * 0.2).cos() * 1000.0)
                + 500.0
                + ((y * 7 + x * 13) as f32 * 0.01).sin() * 200.0
                + texture
        })
    }

    fn shift_array(img: &Array2<f32>, dy: i32, dx: i32) -> Array2<f32> {
        let (rows, cols) = img.dim();
        let mut out = Array2::<f32>::zeros((rows, cols));
        for y in 0..rows {
            let sy = y as i32 - dy;
            if sy < 0 || sy >= rows as i32 {
                continue;
            }
            for x in 0..cols {
                let sx = x as i32 - dx;
                if sx < 0 || sx >= cols as i32 {
                    continue;
                }
                out[[y, x]] = img[[sy as usize, sx as usize]];
            }
        }
        out
    }

    #[test]
    fn test_phase_correlation_actually_aligns() {
        let reference = make_pattern(128, 128);
        let target = shift_array(&reference, 6, -4);
        let result = align_pair(
            &reference,
            &target,
            AlignMethod::PhaseCorrelation,
            128,
            128,
        )
            .unwrap();

        let mut residual_sum = 0.0f64;
        let mut count = 0u32;
        for y in 20..108 {
            for x in 20..108 {
                let rv = reference[[y, x]];
                let av = result.aligned[[y, x]];
                if av.is_finite() && rv.is_finite() {
                    let d = (av - rv) as f64;
                    residual_sum += d * d;
                    count += 1;
                }
            }
        }
        let rmse = (residual_sum / count as f64).sqrt();
        assert!(rmse < 50.0, "RMSE too high after alignment: {}", rmse);
    }

    #[test]
    fn test_estimate_offset_phase_correlation() {
        let reference = make_pattern(128, 128);
        let target = shift_array(&reference, 5, -3);
        let est = estimate_offset(&reference, &target, AlignMethod::PhaseCorrelation);
        assert!((est.dy - 5.0).abs() < 1.5);
        assert!((est.dx - (-3.0)).abs() < 1.5);
    }

    #[test]
    fn test_shift_subpixel_zero() {
        let img = make_pattern(64, 64);
        let shifted = shift_image_subpixel(&img, 0.0, 0.0);
        for y in 0..64 {
            for x in 0..64 {
                assert!((img[[y, x]] - shifted[[y, x]]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn test_shift_subpixel_nonzero() {
        let img = make_pattern(64, 64);
        let shifted = shift_image_subpixel(&img, 2.0, 3.0);
        assert_eq!(shifted.dim(), (64, 64));
        assert!(shifted[[30, 30]].is_finite());
    }

    #[test]
    fn test_shift_subpixel_non_contiguous_input() {
        let base = make_pattern(64, 64);
        let view = base.t().to_owned();
        let shifted = shift_image_subpixel(&view, 1.0, 1.0);
        assert_eq!(shifted.dim(), (64, 64));
        assert!(shifted[[30, 30]].is_finite());
    }
}
