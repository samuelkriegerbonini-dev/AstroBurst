use anyhow::Result;
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::alignment::affine;
use crate::core::alignment::phase_correlation;
use crate::core::imaging::resample::bicubic_sample;
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

fn shift_image_subpixel(image: &Array2<f32>, dy: f64, dx: f64) -> Array2<f32> {
    if dy.abs() < 1e-12 && dx.abs() < 1e-12 {
        return image.clone();
    }
    let (rows, cols) = image.dim();
    let src = image.as_slice().expect("contiguous");
    let mut out = vec![f32::NAN; rows * cols];
    out.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        for x in 0..cols {
            let sy = y as f64 - dy;
            let sx = x as f64 - dx;
            if sy < -0.5 || sy > (rows as f64 - 0.5) || sx < -0.5 || sx > (cols as f64 - 0.5) {
                continue;
            }
            row[x] = bicubic_sample(src, rows, cols, sy, sx);
        }
    });
    Array2::from_shape_vec((rows, cols), out).unwrap()
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
                label, result.offset.0, result.offset.1, result.confidence,
            );
        }
        AlignMethod::Affine => {
            log::info!(
                "{} alignment: method={}, stars={}, inliers={}, residual={:.3}px, tx={:.2}, ty={:.2}",
                label, result.method_used, result.matched_stars, result.inliers,
                result.residual_px, result.offset.1, result.offset.0,
            );
        }
    }
    Ok(result)
}
