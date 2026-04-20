use anyhow::Result;
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::alignment::affine;
use crate::core::alignment::phase_correlation;
use crate::core::imaging::sampling::bicubic_sample;
use crate::types::compose::AlignMethod;

#[derive(Debug, Clone)]
pub struct AlignPairResult {
    pub aligned: Array2<f32>,
    pub offset: (f64, f64),
    pub confidence: f64,
    pub method_used: String,
    pub matched_stars: usize,
    pub inliers: usize,
    pub residual_px: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct OffsetEstimate {
    pub dy: f64,
    pub dx: f64,
    pub confidence: f64,
}

fn ensure_contiguous(image: &Array2<f32>) -> Array2<f32> {
    if image.is_standard_layout() {
        image.clone()
    } else {
        image.to_owned()
    }
}

pub fn shift_image_subpixel(image: &Array2<f32>, dy: f64, dx: f64) -> Array2<f32> {
    if dy.abs() < 1e-12 && dx.abs() < 1e-12 {
        return image.clone();
    }
    let owned = ensure_contiguous(image);
    let (rows, cols) = owned.dim();
    let src = owned.as_slice().expect("contiguous after ensure_contiguous");
    let mut out = vec![0.0f32; rows * cols];
    let rows_f = rows as f64;
    let cols_f = cols as f64;
    out.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        for x in 0..cols {
            let sy = y as f64 + dy;
            let sx = x as f64 + dx;
            if sy < -0.5 || sy > rows_f - 0.5 || sx < -0.5 || sx > cols_f - 0.5 {
                continue;
            }
            row[x] = bicubic_sample(src, rows, cols, sy, sx);
        }
    });
    Array2::from_shape_vec((rows, cols), out).unwrap()
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
                confidence: if result.inliers > 0 { 1.0 } else { 0.0 },
            }
        }
    }
}

pub fn align_pair(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    method: AlignMethod,
    rows: usize,
    cols: usize,
) -> Result<AlignPairResult> {
    match method {
        AlignMethod::PhaseCorrelation => {
            let pc = phase_correlation::phase_correlate(reference, target);
            let shifted = shift_image_subpixel(target, pc.dy, pc.dx);
            Ok(AlignPairResult {
                aligned: shifted,
                offset: (pc.dy, pc.dx),
                confidence: pc.confidence,
                method_used: "phase_correlation".into(),
                matched_stars: 0,
                inliers: 0,
                residual_px: 0.0,
            })
        }
        AlignMethod::Affine => {
            let result = affine::align_channel_affine(reference, target);
            let warped = affine::warp_image(target, &result.transform, rows, cols);
            Ok(AlignPairResult {
                aligned: warped,
                offset: (result.transform.ty, result.transform.tx),
                confidence: if result.inliers > 0 { 1.0 } else { 0.0 },
                method_used: result.method.to_string(),
                matched_stars: result.matched_stars,
                inliers: result.inliers,
                residual_px: result.residual_px,
            })
        }
    }
}

pub fn align_pair_with_label(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    method: AlignMethod,
    rows: usize,
    cols: usize,
    label: &str,
) -> Result<AlignPairResult> {
    let result = align_pair(reference, target, method, rows, cols)?;
    match method {
        AlignMethod::PhaseCorrelation => {
            log::info!(
                "{} alignment: phase_correlation, offset=({:.2}, {:.2}), confidence={:.4}",
                label,
                result.offset.0,
                result.offset.1,
                result.confidence,
            );
        }
        AlignMethod::Affine => {
            log::info!(
                "{} alignment: method={}, stars={}, inliers={}, residual={:.3}px, tx={:.2}, ty={:.2}",
                label,
                result.method_used,
                result.matched_stars,
                result.inliers,
                result.residual_px,
                result.offset.1,
                result.offset.0,
            );
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pattern(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            ((y as f32 * 0.3).sin() * (x as f32 * 0.2).cos() * 1000.0)
                + 500.0
                + ((y * 7 + x * 13) as f32 * 0.01).sin() * 200.0
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
}
