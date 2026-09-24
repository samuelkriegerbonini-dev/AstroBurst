use anyhow::Result;
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::alignment::affine;
use crate::core::alignment::phase_correlation;
use crate::core::imaging::sampling::bicubic_sample;
use crate::types::compose::AlignMethod;

const MAX_OFFSET_FRACTION: f64 = 0.40;

#[derive(Debug, Clone)]
pub struct AlignPairResult {
    pub aligned: Array2<f32>,
    pub offset: (f64, f64),
    pub center_offset: (f64, f64),
    pub registered: bool,
    pub confidence: f64,
    pub method_used: String,
    pub matched_stars: usize,
    pub inliers: usize,
    pub residual_px: f64,
}

pub fn offset_within_limits(dy: f64, dx: f64, rows: usize, cols: usize) -> bool {
    dy.is_finite()
        && dx.is_finite()
        && dx.abs() <= cols as f64 * MAX_OFFSET_FRACTION
        && dy.abs() <= rows as f64 * MAX_OFFSET_FRACTION
}

fn center_displacement(transform: &affine::AffineTransform, rows: usize, cols: usize) -> (f64, f64) {
    let cx = (cols as f64 - 1.0) * 0.5;
    let cy = (rows as f64 - 1.0) * 0.5;
    let (sx, sy) = transform.map(cx, cy);
    (sy - cy, sx - cx)
}

fn ensure_contiguous(image: &Array2<f32>) -> Array2<f32> {
    if image.is_standard_layout() {
        image.clone()
    } else {
        image.as_standard_layout().into_owned()
    }
}

pub fn shift_image_subpixel(image: &Array2<f32>, dy: f64, dx: f64) -> Array2<f32> {
    if dy.abs() < 1e-12 && dx.abs() < 1e-12 {
        return image.clone();
    }
    let owned = ensure_contiguous(image);
    let (rows, cols) = owned.dim();
    let src = owned.as_slice().expect("contiguous after ensure_contiguous");
    let mut out = vec![f32::NAN; rows * cols];
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
            let reject = !offset_within_limits(pc.dy, pc.dx, rows, cols)
                || phase_correlation::is_low_confidence(pc.confidence);
            if reject {
                Ok(AlignPairResult {
                    aligned: target.clone(),
                    offset: (0.0, 0.0),
                    center_offset: (0.0, 0.0),
                    registered: false,
                    confidence: pc.confidence,
                    method_used: "phase_correlation_identity".into(),
                    matched_stars: 0,
                    inliers: 0,
                    residual_px: 0.0,
                })
            } else {
                let shifted = shift_image_subpixel(target, pc.dy, pc.dx);
                Ok(AlignPairResult {
                    aligned: shifted,
                    offset: (pc.dy, pc.dx),
                    center_offset: (pc.dy, pc.dx),
                    registered: true,
                    confidence: pc.confidence,
                    method_used: "phase_correlation".into(),
                    matched_stars: 0,
                    inliers: 0,
                    residual_px: 0.0,
                })
            }
        }
        AlignMethod::Affine => {
            let result = affine::align_channel_affine(reference, target);
            let warped = affine::warp_image(target, &result.transform, rows, cols);
            Ok(AlignPairResult {
                aligned: warped,
                offset: (result.transform.ty, result.transform.tx),
                center_offset: center_displacement(&result.transform, rows, cols),
                registered: result.method != affine::AffineAlignMethod::Identity,
                confidence: result.confidence,
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

#[cfg(test)]
fn xorshift_unit(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
pub(crate) fn star_field(size: usize, angle_deg: f64, noise_seed: u64) -> Array2<f32> {
    let mut positions = 0x9E37_79B9_7F4A_7C15u64;
    let center = (size as f64 - 1.0) * 0.5;
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let span = size as f64 - 60.0;
    let stars: Vec<(f64, f64, f64)> = (0..40)
        .map(|_| {
            let x = 30.0 + xorshift_unit(&mut positions) * span;
            let y = 30.0 + xorshift_unit(&mut positions) * span;
            let amp = 400.0 + xorshift_unit(&mut positions) * 4000.0;
            let (dx, dy) = (x - center, y - center);
            (center + cos * dx - sin * dy, center + sin * dx + cos * dy, amp)
        })
        .collect();
    let mut noise = noise_seed.max(1);
    Array2::from_shape_fn((size, size), |(y, x)| {
        let mut v = 100.0 + (xorshift_unit(&mut noise) - 0.5) * 6.0;
        for &(sx, sy, amp) in &stars {
            let d2 = (x as f64 - sx).powi(2) + (y as f64 - sy).powi(2);
            if d2 < 100.0 {
                v += amp * (-d2 / (2.0 * 1.6 * 1.6)).exp();
            }
        }
        v as f32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pattern(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            let h = (y.wrapping_mul(73856093) ^ x.wrapping_mul(19349663)) & 0xffff;
            let texture = (h as f32 / 65535.0 - 0.5) * 400.0;
            ((y as f32 * 0.3).sin() * (x as f32 * 0.2).cos() * 1000.0) + 500.0
                + ((y * 7 + x * 13) as f32 * 0.01).sin() * 200.0
                + texture
        })
    }

    fn shift_array(img: &Array2<f32>, dy: i32, dx: i32) -> Array2<f32> {
        let (rows, cols) = img.dim();
        let mut out = Array2::<f32>::zeros((rows, cols));
        for y in 0..rows {
            let sy = y as i32 - dy;
            if sy < 0 || sy >= rows as i32 { continue; }
            for x in 0..cols {
                let sx = x as i32 - dx;
                if sx < 0 || sx >= cols as i32 { continue; }
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
        ).unwrap();

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

    fn blob_scene(rows: usize, cols: usize, cy: f32, cx: f32) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            let dy = y as f32 - cy;
            let dx = x as f32 - cx;
            100.0 + 1000.0 * (-(dy * dy + dx * dx) / 18.0).exp()
        })
    }

    #[test]
    fn out_of_range_pc_shift_falls_back_to_identity() {
        let reference = blob_scene(300, 300, 150.0, 50.0);
        let target = blob_scene(300, 300, 150.0, 250.0);
        let result = align_pair(
            &reference,
            &target,
            AlignMethod::PhaseCorrelation,
            300,
            300,
        ).unwrap();

        assert_eq!(result.offset, (0.0, 0.0));
        assert_eq!(result.method_used, "phase_correlation_identity");
        assert!(!result.registered);
        assert!(result.aligned.iter().all(|v| v.is_finite()));
        assert_eq!(result.aligned, target);

        let affine = align_pair(&reference, &target, AlignMethod::Affine, 300, 300).unwrap();
        assert_eq!(affine.method_used, "identity");
        assert!(!affine.registered);
    }

    #[test]
    fn registered_phase_correlation_reports_its_shift_as_centre_offset() {
        let reference = make_pattern(128, 128);
        let target = shift_array(&reference, 6, -4);
        let result = align_pair(&reference, &target, AlignMethod::PhaseCorrelation, 128, 128).unwrap();
        assert!(result.registered);
        assert_eq!(result.center_offset, result.offset);
        assert!((result.offset.0 - 6.0).abs() < 0.5 && (result.offset.1 + 4.0).abs() < 0.5, "{:?}", result.offset);
    }

    #[test]
    fn affine_centre_offset_is_the_displacement_of_the_image_centre() {
        let reference = star_field(400, 0.0, 11);
        let target = star_field(400, 3.0, 12);
        let result = align_pair(&reference, &target, AlignMethod::Affine, 400, 400).unwrap();
        assert!(result.registered, "method {}", result.method_used);
        assert!(result.center_offset.0.abs() < 0.5 && result.center_offset.1.abs() < 0.5, "{:?}", result.center_offset);
        assert!(result.offset.1 > 8.0 && result.offset.0 < -8.0, "origin translation {:?}", result.offset);
    }

    #[test]
    fn offset_limits_reject_non_finite_and_far_shifts() {
        assert!(offset_within_limits(10.0, -10.0, 100, 100));
        assert!(offset_within_limits(40.0, 40.0, 100, 100));
        assert!(!offset_within_limits(40.1, 0.0, 100, 100));
        assert!(!offset_within_limits(0.0, -40.1, 100, 100));
        assert!(!offset_within_limits(f64::NAN, 0.0, 100, 100));
        assert!(!offset_within_limits(0.0, f64::INFINITY, 100, 100));
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
    fn test_shift_subpixel_integer_shift_samples_the_shifted_source() {
        let img = make_pattern(64, 64);
        let shifted = shift_image_subpixel(&img, 2.0, 3.0);
        assert_eq!(shifted.dim(), (64, 64));
        for y in 0..62 {
            for x in 0..61 {
                let expected = img[[y + 2, x + 3]];
                assert!((shifted[[y, x]] - expected).abs() < 1e-3 * expected.abs().max(1.0), "({y},{x}) {} vs {expected}", shifted[[y, x]]);
            }
        }
        assert!(shifted[[63, 63]].is_nan());
    }

    #[test]
    fn test_shift_subpixel_non_contiguous_input() {
        let base = make_pattern(64, 64);
        let view = base.t().to_owned();
        let shifted = shift_image_subpixel(&view, 1.0, 1.0);
        assert_eq!(shifted.dim(), (64, 64));
        for y in 0..63 {
            for x in 0..63 {
                let expected = view[[y + 1, x + 1]];
                assert!((shifted[[y, x]] - expected).abs() < 1e-3 * expected.abs().max(1.0));
            }
        }
    }
}
