use ndarray::Array2;
use rayon::prelude::*;

use crate::core::alignment::phase_correlation;
use crate::core::alignment::triangles::{self, StarPair, TriangleParams};
use crate::core::analysis::star_detection::detect_stars;
use crate::core::imaging::sampling::bicubic_sample;
use crate::math::median::exact_median_f64;

const MIN_MATCHES_AFFINE: usize = 6;
const MIN_MATCHES_RIGID: usize = 4;
const DETECTION_SIGMA: f64 = 3.5;
const MAX_OFFSET_FRACTION: f64 = 0.40;
const MIN_SCALE: f64 = 0.70;
const MAX_SCALE: f64 = 1.40;
const MIN_REFLECTION_INLIERS: usize = 5;
const TRANSLATION_AGREE_PX: f64 = 1.0;
const TRANSLATION_AGREE_FWHM_FRACTION: f64 = 0.5;
const FIXED_SOURCE_RADIUS_PX: f64 = 0.5;

#[derive(Debug)]
pub enum TranslationCheck {
    Inconclusive,
    Agrees,
    Prefer(AffineAlignResult),
    Refused(AffineAlignResult),
    FitRejected(String),
}

#[derive(Debug, Clone, Default)]
pub struct AlignmentStars {
    pub positions: Vec<(f64, f64)>,
    pub median_fwhm_px: f64,
}

impl AlignmentStars {
    pub fn agreement_tolerance_px(&self) -> f64 {
        TRANSLATION_AGREE_PX.max(TRANSLATION_AGREE_FWHM_FRACTION * self.median_fwhm_px)
    }
}

pub fn alignment_stars(image: &Array2<f32>) -> AlignmentStars {
    let (rows, cols) = image.dim();
    let detected = detect_stars(&normalize_for_detection(image), DETECTION_SIGMA);
    let fwhm: Vec<f64> = detected.stars.iter().map(|star| star.fwhm).collect();
    AlignmentStars {
        positions: triangles::select_alignment_stars(&detected.stars, TriangleParams::ALIGNMENT.max_stars, rows, cols),
        median_fwhm_px: exact_median_f64(&fwhm),
    }
}

pub fn check_translation_against_stars(
    reference: &AlignmentStars,
    target: &Array2<f32>,
    dy: f64,
    dx: f64,
    rows: usize,
    cols: usize,
) -> TranslationCheck {
    let target_stars = alignment_stars(target).positions;
    check_translation_against_star_lists(reference, &target_stars, dy, dx, rows, cols)
}

fn check_translation_against_star_lists(
    reference: &AlignmentStars,
    target_stars: &[(f64, f64)],
    dy: f64,
    dx: f64,
    rows: usize,
    cols: usize,
) -> TranslationCheck {
    let params = &TriangleParams::ALIGNMENT;
    let tolerance = reference.agreement_tolerance_px();
    let judge = |fit| judge_against_translation(fit, dy, dx, rows, cols, tolerance, params);
    let verdict = judge(register_star_lists(&reference.positions, target_stars, rows, cols, params));
    if matches!(verdict, TranslationCheck::Agrees | TranslationCheck::Refused(_)) || dx.hypot(dy) <= tolerance {
        return verdict;
    }
    let (moving_reference, moving_target) = without_fixed_sources(&reference.positions, target_stars, FIXED_SOURCE_RADIUS_PX);
    let fixed = reference.positions.len() - moving_reference.len();
    if fixed < params.min_inliers {
        return verdict;
    }
    let contaminated = |without_them: String| {
        TranslationCheck::FitRejected(format!(
            "the star match is contaminated by {fixed} of {} reference sources that sit at the same pixels in both frames despite the ({dx:.1}, {dy:.1}) px shift; sources fixed on the sensor, such as hot pixels, are being matched as stars, and without them {without_them}",
            reference.positions.len()
        ))
    };
    match judge(register_star_lists(&moving_reference, &moving_target, rows, cols, params)) {
        TranslationCheck::Inconclusive => contaminated("too few sources remain for a star match".to_string()),
        TranslationCheck::FitRejected(reason) => contaminated(reason),
        clean => clean,
    }
}

fn judge_against_translation(
    fit: Result<AffineAlignResult, Option<SanityRejection>>,
    dy: f64,
    dx: f64,
    rows: usize,
    cols: usize,
    tolerance: f64,
    params: &TriangleParams,
) -> TranslationCheck {
    match fit {
        Ok(result) if result.method == AffineAlignMethod::Identity => TranslationCheck::Refused(result),
        Ok(result) if agrees_with_translation(&result.transform, dy, dx, rows, cols, tolerance) => TranslationCheck::Agrees,
        Ok(result) if !params.is_wide_rotation_fit(result.matched_stars, result.inliers, result.rms_px) => {
            TranslationCheck::FitRejected(format!(
                "the star match disagrees with the ({:.1}, {:.1}) px shift but is not decisive: {} of {} inliers at RMS {:.2} px, while overriding a confident shift needs at least {} inliers and RMS <= {:.1} px",
                dx,
                dy,
                result.inliers,
                result.matched_stars,
                result.rms_px,
                params.wide_rotation_inliers_needed(result.matched_stars),
                params.wide_rotation_max_rms_px
            ))
        }
        Ok(result) => TranslationCheck::Prefer(result),
        Err(Some(rejected)) if rejected.kind == SanityKind::WideRotation => TranslationCheck::Refused(unregistered(Some(rejected.reason))),
        Err(Some(rejected)) => TranslationCheck::FitRejected(rejected.reason),
        Err(None) => TranslationCheck::Inconclusive,
    }
}

fn without_fixed_sources(
    reference_stars: &[(f64, f64)],
    target_stars: &[(f64, f64)],
    radius_px: f64,
) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
    let mut fixed_target = vec![false; target_stars.len()];
    let moving_reference: Vec<(f64, f64)> = reference_stars
        .iter()
        .copied()
        .filter(|&(x, y)| match target_stars.iter().position(|&(tx, ty)| (tx - x).hypot(ty - y) < radius_px) {
            Some(j) => {
                fixed_target[j] = true;
                false
            }
            None => true,
        })
        .collect();
    let moving_target = target_stars.iter().zip(&fixed_target).filter(|(_, fixed)| !**fixed).map(|(&star, _)| star).collect();
    (moving_reference, moving_target)
}

fn agrees_with_translation(transform: &AffineTransform, dy: f64, dx: f64, rows: usize, cols: usize, tolerance_px: f64) -> bool {
    largest_probe_displacement(transform, &AffineTransform::translation(dx, dy), rows, cols) <= tolerance_px
}

pub(crate) fn largest_probe_displacement(found: &AffineTransform, expected: &AffineTransform, rows: usize, cols: usize) -> f64 {
    let (right, bottom) = (cols as f64 - 1.0, rows as f64 - 1.0);
    [(0.0, 0.0), (right, 0.0), (0.0, bottom), (right, bottom), (right * 0.5, bottom * 0.5)]
        .into_iter()
        .map(|(x, y)| {
            let (fx, fy) = found.map(x, y);
            let (ex, ey) = expected.map(x, y);
            (fx - ex).hypot(fy - ey)
        })
        .fold(0.0, f64::max)
}

fn normalize_for_detection(image: &Array2<f32>) -> Array2<f32> {
    let len = image.len();
    if len == 0 {
        return image.clone();
    }

    let sample_step = (len / 100_000).max(1);
    let mut samples: Vec<f32> = image.iter().step_by(sample_step).copied().filter(|v| v.is_finite()).collect();
    if samples.len() < 100 {
        return image.clone();
    }
    samples.sort_by(|a, b| a.total_cmp(b));

    let lo = samples[samples.len() / 100] as f64;
    let hi = samples[samples.len() * 999 / 1000] as f64;
    let range = hi - lo;
    if range < 1e-15 {
        return image.clone();
    }
    let inv_range = 1.0 / range;

    let mut result = Array2::zeros(image.dim());
    ndarray::Zip::from(&mut result)
        .and(image)
        .par_for_each(|o, &v| {
            *o = ((v as f64 - lo) * inv_range).max(0.0) as f32;
        });
    result
}

#[derive(Debug, Clone, Copy)]
pub struct AffineTransform {
    pub a: f64,
    pub b: f64,
    pub tx: f64,
    pub c: f64,
    pub d: f64,
    pub ty: f64,
}

impl AffineTransform {
    pub fn identity() -> Self {
        Self { a: 1.0, b: 0.0, tx: 0.0, c: 0.0, d: 1.0, ty: 0.0 }
    }

    pub fn translation(tx: f64, ty: f64) -> Self {
        Self { a: 1.0, b: 0.0, tx, c: 0.0, d: 1.0, ty }
    }

    #[inline(always)]
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.b * y + self.tx,
            self.c * x + self.d * y + self.ty,
        )
    }

    pub fn rotation_deg(&self) -> f64 {
        self.c.atan2(self.a).to_degrees()
    }

    pub fn scale_x(&self) -> f64 {
        (self.a * self.a + self.c * self.c).sqrt()
    }

    pub fn scale_y(&self) -> f64 {
        (self.b * self.b + self.d * self.d).sqrt()
    }
}

#[derive(Debug, Clone)]
pub struct AffineAlignResult {
    pub transform: AffineTransform,
    pub matched_stars: usize,
    pub inliers: usize,
    pub residual_px: f64,
    pub rms_px: f64,
    pub method: AffineAlignMethod,
    pub confidence: f64,
    pub rejection: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AffineAlignMethod {
    Affine,
    Rigid,
    PhaseCorrelation,
    Identity,
}

impl std::fmt::Display for AffineAlignMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AffineAlignMethod::Affine => write!(f, "affine"),
            AffineAlignMethod::Rigid => write!(f, "rigid"),
            AffineAlignMethod::PhaseCorrelation => write!(f, "phase_correlation"),
            AffineAlignMethod::Identity => write!(f, "identity"),
        }
    }
}

pub fn align_channel_affine(
    reference: &Array2<f32>,
    target: &Array2<f32>,
) -> AffineAlignResult {
    align_channel_affine_with(reference, &alignment_stars(reference), target)
}

pub fn align_channel_affine_with(
    reference: &Array2<f32>,
    reference_stars: &AlignmentStars,
    target: &Array2<f32>,
) -> AffineAlignResult {
    let (rows, cols) = reference.dim();
    let target_stars = alignment_stars(target).positions;

    log::info!(
        "Affine: {} ref stars, {} tgt stars selected (image {}x{})",
        reference_stars.positions.len(), target_stars.len(), cols, rows
    );

    match register_star_lists(&reference_stars.positions, &target_stars, rows, cols, &TriangleParams::ALIGNMENT) {
        Ok(result) => result,
        Err(rejection) => fallback_phase_correlation(reference, target, rows, cols, rejection.map(|rejected| rejected.reason)),
    }
}

fn register_star_lists(
    ref_stars: &[(f64, f64)],
    tgt_stars: &[(f64, f64)],
    rows: usize,
    cols: usize,
    params: &TriangleParams,
) -> Result<AffineAlignResult, Option<SanityRejection>> {
    if ref_stars.len() < MIN_MATCHES_RIGID || tgt_stars.len() < MIN_MATCHES_RIGID {
        log::info!(
            "Affine: insufficient stars (ref={}, tgt={}), fallback to phase correlation",
            ref_stars.len(), tgt_stars.len()
        );
        return Err(None);
    }

    let ref_tris = triangles::build_triangles(ref_stars, params);
    let tgt_tris = triangles::build_triangles(tgt_stars, params);
    let same = triangles::match_triangles(ref_stars, tgt_stars, &ref_tris, &tgt_tris, params);

    log::info!(
        "Affine: {} triangle matches ({} votes) from stars ref={}, tgt={}",
        same.pairs.len(), same.votes, ref_stars.len(), tgt_stars.len()
    );

    let rejection = match fit_same_parity(&same.pairs, rows, cols, params) {
        Ok(result) => return Ok(result),
        Err(rejection) => rejection,
    };

    let more_voted = triangles::match_star_lists(ref_stars, tgt_stars, params);
    if !more_voted.flipped {
        return Err(rejection);
    }
    match reflection_refusal(&more_voted.pairs, rows, cols, params) {
        Some(refused) => Ok(refused),
        None => Err(rejection),
    }
}

fn fit_same_parity(
    matches: &[StarPair],
    rows: usize,
    cols: usize,
    params: &TriangleParams,
) -> Result<AffineAlignResult, Option<SanityRejection>> {
    if matches.len() < MIN_MATCHES_RIGID {
        log::info!(
            "Affine: too few triangle matches ({}), fallback to phase correlation",
            matches.len()
        );
        return Err(None);
    }

    let mut rejection: Option<SanityRejection> = None;

    if matches.len() >= MIN_MATCHES_AFFINE {
        if let Some(result) = triangles::ransac_affine(matches, AffineAlignMethod::Affine, params) {
            match check_transform_sanity(&result, rows, cols, params) {
                Ok(()) => return Ok(result),
                Err(rejected) => {
                    log::warn!(
                        "Affine transform rejected: {}. stars={}, inliers={}, residual={:.2}px, rms={:.2}px. Trying rigid.",
                        rejected.reason, result.matched_stars, result.inliers, result.residual_px, result.rms_px
                    );
                    rejection = Some(rejected);
                }
            }
        } else {
            log::info!("Affine RANSAC returned no result with {} matches, trying rigid", matches.len());
        }
    }

    if let Some(result) = triangles::ransac_affine(matches, AffineAlignMethod::Rigid, params) {
        match check_transform_sanity(&result, rows, cols, params) {
            Ok(()) => return Ok(result),
            Err(rejected) => {
                log::warn!(
                    "Rigid transform rejected: {}. stars={}, inliers={}, residual={:.2}px.",
                    rejected.reason, result.matched_stars, result.inliers, result.residual_px
                );
                rejection.get_or_insert(rejected);
            }
        }
    } else {
        log::info!("Rigid RANSAC returned no result with {} matches", matches.len());
    }

    Err(rejection)
}

fn reflection_refusal(
    mirrored_pairs: &[StarPair],
    rows: usize,
    cols: usize,
    params: &TriangleParams,
) -> Option<AffineAlignResult> {
    if mirrored_pairs.len() < MIN_MATCHES_AFFINE {
        return None;
    }
    let fit = triangles::ransac_affine(mirrored_pairs, AffineAlignMethod::Affine, params)?;
    let rejected = check_transform_sanity(&fit, rows, cols, params).err()?;
    if rejected.kind != SanityKind::Reflection || !is_decisive_reflection(&fit) {
        return None;
    }
    log::warn!(
        "Affine: mirrored star pattern, {} of {} mirrored matches agree: {}",
        fit.inliers, fit.matched_stars, rejected.reason
    );
    Some(AffineAlignResult {
        transform: AffineTransform::identity(),
        method: AffineAlignMethod::Identity,
        confidence: 0.0,
        rejection: Some(rejected.reason),
        ..fit
    })
}

fn is_decisive_reflection(fit: &AffineAlignResult) -> bool {
    let t = &fit.transform;
    let det = t.a * t.d - t.b * t.c;
    fit.inliers >= MIN_REFLECTION_INLIERS && det.abs() >= MIN_SCALE * MIN_SCALE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SanityKind {
    Other,
    Reflection,
    WideRotation,
}

#[derive(Debug)]
struct SanityRejection {
    reason: String,
    kind: SanityKind,
}

impl SanityRejection {
    fn other(reason: String) -> Self {
        Self { reason, kind: SanityKind::Other }
    }
}

fn check_transform_sanity(
    result: &AffineAlignResult,
    rows: usize,
    cols: usize,
    params: &TriangleParams,
) -> Result<(), SanityRejection> {
    let t = &result.transform;
    if ![t.a, t.b, t.tx, t.c, t.d, t.ty].iter().all(|v| v.is_finite()) {
        return Err(SanityRejection::other("non-finite transform rejected".to_string()));
    }

    let (sx, sy) = (t.scale_x(), t.scale_y());
    let scale_range = MIN_SCALE..=MAX_SCALE;
    if !scale_range.contains(&sx) || !scale_range.contains(&sy) {
        return Err(SanityRejection::other(format!(
            "scale ({:.3}, {:.3}) outside [{:.2}, {:.2}] range",
            sx, sy, MIN_SCALE, MAX_SCALE
        )));
    }

    let det = t.a * t.d - t.b * t.c;
    if det <= 0.0 {
        return Err(SanityRejection {
            reason: format!("reflection (det={:.3}) rejected", det),
            kind: SanityKind::Reflection,
        });
    }

    let rotation = t.rotation_deg();
    let max_dx = cols as f64 * MAX_OFFSET_FRACTION;
    let max_dy = rows as f64 * MAX_OFFSET_FRACTION;
    if rotation.abs() <= params.wide_rotation_above_deg {
        if t.tx.abs() > max_dx || t.ty.abs() > max_dy {
            return Err(SanityRejection::other(format!(
                "translation ({:.1}, {:.1}) exceeds limit ({:.0}, {:.0})",
                t.tx, t.ty, max_dx, max_dy
            )));
        }
    } else {
        let (dy, dx) = center_displacement(t, rows, cols);
        if dx.abs() > max_dx || dy.abs() > max_dy {
            return Err(SanityRejection::other(format!(
                "centre displacement ({:.1}, {:.1}) exceeds limit ({:.0}, {:.0})",
                dx, dy, max_dx, max_dy
            )));
        }
    }

    let wide_fit = params.is_wide_rotation_fit(result.matched_stars, result.inliers, result.rms_px);
    if rotation.abs() > params.wide_rotation_above_deg && !wide_fit {
        return Err(SanityRejection {
            reason: format!(
                "rotation {:.2} deg needs at least {} inliers and RMS <= {:.1} px, got {} of {} matches at RMS {:.2} px",
                rotation,
                params.wide_rotation_inliers_needed(result.matched_stars),
                params.wide_rotation_max_rms_px,
                result.inliers,
                result.matched_stars,
                result.rms_px
            ),
            kind: SanityKind::WideRotation,
        });
    }

    Ok(())
}

pub(crate) fn center_displacement(transform: &AffineTransform, rows: usize, cols: usize) -> (f64, f64) {
    let cx = (cols as f64 - 1.0) * 0.5;
    let cy = (rows as f64 - 1.0) * 0.5;
    let (sx, sy) = transform.map(cx, cy);
    (sy - cy, sx - cx)
}

fn fallback_phase_correlation(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    rows: usize,
    cols: usize,
    rejection: Option<String>,
) -> AffineAlignResult {
    let pc = phase_correlation::phase_correlate(reference, target);

    let max_tx = cols as f64 * MAX_OFFSET_FRACTION;
    let max_ty = rows as f64 * MAX_OFFSET_FRACTION;
    if pc.dx.abs() > max_tx || pc.dy.abs() > max_ty || phase_correlation::is_low_confidence(pc.confidence) {
        return unregistered(rejection);
    }

    AffineAlignResult {
        transform: AffineTransform::translation(pc.dx, pc.dy),
        matched_stars: 0,
        inliers: 0,
        residual_px: 0.0,
        rms_px: 0.0,
        method: AffineAlignMethod::PhaseCorrelation,
        confidence: pc.confidence,
        rejection,
    }
}

fn unregistered(rejection: Option<String>) -> AffineAlignResult {
    AffineAlignResult {
        transform: AffineTransform::identity(),
        matched_stars: 0,
        inliers: 0,
        residual_px: 0.0,
        rms_px: 0.0,
        method: AffineAlignMethod::Identity,
        confidence: 0.0,
        rejection,
    }
}

pub fn warp_image(
    image: &Array2<f32>,
    transform: &AffineTransform,
    out_rows: usize,
    out_cols: usize,
) -> Array2<f32> {
    let (src_rows, src_cols) = image.dim();
    let total = out_rows * out_cols;
    if total == 0 {
        return Array2::zeros((out_rows, out_cols));
    }
    let mut buf = vec![f32::NAN; total];

    if src_rows == 0 || src_cols == 0 {
        return Array2::from_shape_vec((out_rows, out_cols), buf).unwrap();
    }

    let standard = image.as_standard_layout();
    let Some(slice) = standard.as_slice() else {
        return Array2::from_elem((out_rows, out_cols), f32::NAN);
    };
    let max_x = src_cols as f64 - 0.5;
    let max_y = src_rows as f64 - 0.5;

    buf.par_chunks_mut(out_cols)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..out_cols {
                let (sx, sy) = transform.map(x as f64, y as f64);
                if sx >= -0.5 && sy >= -0.5 && sx <= max_x && sy <= max_y {
                    row[x] = bicubic_sample(slice, src_rows, src_cols, sy, sx);
                }
            }
        });

    Array2::from_shape_vec((out_rows, out_cols), buf).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sanity(result: &AffineAlignResult, rows: usize, cols: usize) -> Result<(), String> {
        check_transform_sanity(result, rows, cols, &TriangleParams::ALIGNMENT).map_err(|rejected| rejected.reason)
    }

    fn named_reflection(result: &AffineAlignResult, rows: usize, cols: usize) -> bool {
        check_transform_sanity(result, rows, cols, &TriangleParams::ALIGNMENT)
            .is_err_and(|rejected| rejected.kind == SanityKind::Reflection)
    }

    fn rotation_about_centre(deg: f64, rows: usize, cols: usize) -> AffineTransform {
        let (cx, cy) = ((cols as f64 - 1.0) * 0.5, (rows as f64 - 1.0) * 0.5);
        let (sin, cos) = deg.to_radians().sin_cos();
        AffineTransform {
            a: cos,
            b: -sin,
            tx: cx - cos * cx + sin * cy,
            c: sin,
            d: cos,
            ty: cy - sin * cx - cos * cy,
        }
    }

    fn fitted(transform: AffineTransform, matched: usize, inliers: usize, rms_px: f64) -> AffineAlignResult {
        AffineAlignResult {
            transform,
            matched_stars: matched,
            inliers,
            residual_px: rms_px,
            rms_px,
            method: AffineAlignMethod::Affine,
            confidence: 1.0,
            rejection: None,
        }
    }

    #[test]
    fn a_180_degree_rotation_about_the_centre_passes_the_translation_rule() {
        let half_turn = rotation_about_centre(180.0, 2048, 2048);
        assert!(half_turn.tx > 2000.0 && half_turn.ty > 2000.0, "{half_turn:?}");
        let (dy, dx) = center_displacement(&half_turn, 2048, 2048);
        assert!(dy.abs() < 1e-9 && dx.abs() < 1e-9, "({dy}, {dx})");
        assert_eq!(sanity(&fitted(half_turn, 60, 40, 0.02), 2048, 2048), Ok(()));
    }

    #[test]
    fn a_small_rotation_keeps_todays_thresholds() {
        let ten_degrees = rotation_about_centre(10.0, 1000, 1000);
        assert_eq!(sanity(&fitted(ten_degrees, 12, 6, 2.5), 1000, 1000), Ok(()));
        let thirty_degrees = rotation_about_centre(30.0, 1000, 1000);
        assert_eq!(sanity(&fitted(thirty_degrees, 12, 6, 2.5), 1000, 1000), Ok(()));
    }

    #[test]
    fn a_wide_rotation_needs_eight_inliers_a_quarter_of_the_matches_and_one_pixel_rms() {
        for deg in [37.0, 90.0, -135.0, 179.5] {
            let turn = rotation_about_centre(deg, 1000, 1000);
            assert_eq!(sanity(&fitted(turn, 12, 8, 0.9), 1000, 1000), Ok(()), "{deg} deg");
            assert_eq!(sanity(&fitted(turn, 40, 10, 1.0), 1000, 1000), Ok(()), "{deg} deg");
            for (matched, inliers, rms) in [(12, 7, 0.5), (40, 9, 0.5), (12, 12, 1.01), (12, 6, 2.5)] {
                let verdict = sanity(&fitted(turn, matched, inliers, rms), 1000, 1000);
                let reason = verdict.expect_err("a weak wide rotation was accepted");
                assert!(reason.starts_with("rotation "), "{deg} deg: {reason}");
            }
        }
    }

    #[test]
    fn a_wide_rotation_is_judged_by_the_displacement_of_the_frame_centre() {
        let mut shifted = rotation_about_centre(90.0, 1000, 1000);
        shifted.tx += 401.0;
        let reason = sanity(&fitted(shifted, 40, 30, 0.2), 1000, 1000).expect_err("accepted");
        assert!(reason.starts_with("centre displacement"), "{reason}");
        shifted.tx -= 2.0;
        assert_eq!(sanity(&fitted(shifted, 40, 30, 0.2), 1000, 1000), Ok(()));
    }

    #[test]
    fn a_narrow_rotation_keeps_todays_raw_translation_limit() {
        let mut shifted = rotation_about_centre(5.0, 1000, 1000);
        shifted.tx += 360.0;
        let (_, dx) = center_displacement(&shifted, 1000, 1000);
        assert!(dx.abs() < 400.0 && shifted.tx > 400.0, "{shifted:?}");
        let reason = sanity(&fitted(shifted, 40, 30, 0.2), 1000, 1000).expect_err("accepted");
        assert!(reason.starts_with("translation"), "{reason}");
        shifted.tx -= 10.0;
        assert_eq!(sanity(&fitted(shifted, 40, 30, 0.2), 1000, 1000), Ok(()));
    }

    fn rotated_and_shifted(deg: f64, shift: (f64, f64), size: usize) -> AffineTransform {
        let mut truth = rotation_about_centre(deg, size, size);
        truth.tx += shift.0;
        truth.ty += shift.1;
        truth
    }

    fn largest_corner_error(found: &AffineTransform, truth: &AffineTransform, size: usize) -> f64 {
        largest_probe_displacement(found, truth, size, size)
    }

    fn sharp_tolerance() -> f64 {
        AlignmentStars::default().agreement_tolerance_px()
    }

    const CHANCE_MIRROR_REFERENCE: [(f64, f64); 17] = [
        (730.964859664504, 492.2078142278369),
        (447.49865217257377, 135.079590836146),
        (730.3194979306118, 514.8065453268692),
        (902.8712912289085, 817.3189681165184),
        (273.1386261867581, 696.8800897111793),
        (429.4341275258039, 694.6389248672868),
        (87.4108979128066, 448.15411197843036),
        (673.7122716700512, 189.9315935718026),
        (648.9151497476287, 778.02448543915),
        (930.3331210950173, 481.0783262006324),
        (88.44200179156066, 521.9742991631281),
        (852.2739485684973, 624.086211845046),
        (940.4529929478748, 958.4881009681876),
        (728.755290955321, 793.9306243264987),
        (607.4513525555768, 316.3515541408454),
        (740.5344785398265, 360.2618838614576),
        (555.221021202917, 622.1362674087133),
    ];

    const CHANCE_MIRROR_TARGET: [(f64, f64); 17] = [
        (259.23655768665276, 875.5269150169505),
        (152.17189370770544, 943.7614033508783),
        (772.7583090130126, 366.1450881995878),
        (625.5951467914987, 539.9161853901414),
        (460.1751239809228, 174.7968117191904),
        (698.2934163469951, 194.82192086759397),
        (652.682722731817, 152.00969985137743),
        (840.5260353521998, 150.82327302021736),
        (520.4922500995442, 362.2073484122256),
        (917.9971893616212, 1019.204723991537),
        (375.8448014015446, 344.15771226619984),
        (253.83760596800002, 187.09382661360598),
        (951.813320396572, 222.28732136543945),
        (727.8304330066561, 488.87860919437253),
        (567.6115426003923, 145.24337567468947),
        (203.01307463533382, 152.3018145547455),
        (395.326065192628, 155.2186058286517),
    ];

    #[test]
    fn a_fifth_of_a_degree_rotation_does_not_pass_as_a_translation() {
        let turned = rotated_and_shifted(0.2, (12.25, -7.5), 2048);
        let (dy, dx) = center_displacement(&turned, 2048, 2048);
        assert!((dx - 12.25).abs() < 1e-9 && (dy + 7.5).abs() < 1e-9, "({dy}, {dx})");
        let plain = AffineTransform::translation(12.25, -7.5);
        let corners = largest_probe_displacement(&turned, &plain, 2048, 2048);
        assert!((corners - 5.05).abs() < 0.05, "{corners}");
        assert_eq!(sharp_tolerance(), 1.0);
        assert!(!agrees_with_translation(&turned, dy, dx, 2048, 2048, sharp_tolerance()), "{turned:?}");
        let seeing_limited = AlignmentStars { positions: Vec::new(), median_fwhm_px: 6.0 }.agreement_tolerance_px();
        assert_eq!(seeing_limited, 3.0);
        assert!(!agrees_with_translation(&turned, dy, dx, 2048, 2048, seeing_limited), "{turned:?}");
        assert!(agrees_with_translation(&plain, -7.5, 12.25, 2048, 2048, sharp_tolerance()));
        let stretched = AffineTransform { a: 1.001, b: 0.0, tx: 12.25 - 0.001 * 1023.5, c: 0.0, d: 1.001, ty: -7.5 - 0.001 * 1023.5 };
        let (sy, sx) = center_displacement(&stretched, 2048, 2048);
        assert!((sx - 12.25).abs() < 1e-9 && (sy + 7.5).abs() < 1e-9, "({sy}, {sx})");
        assert!(
            !agrees_with_translation(&stretched, -7.5, 12.25, 2048, 2048, sharp_tolerance()),
            "a 0.1 % scale moves the corners by {} px",
            largest_probe_displacement(&stretched, &plain, 2048, 2048)
        );
    }

    #[test]
    fn a_negated_shift_does_not_pass_as_agreement() {
        let shift = AffineTransform::translation(12.25, -7.5);
        assert!(agrees_with_translation(&shift, -7.5, 12.25, 2048, 2048, sharp_tolerance()));
        assert!(!agrees_with_translation(&shift, 7.5, -12.25, 2048, 2048, sharp_tolerance()));
    }

    fn viewed(stars: &[(f64, f64)], deg: f64, shift: (f64, f64), size: usize) -> Vec<(f64, f64)> {
        let truth = rotated_and_shifted(deg, shift, size);
        stars.iter().map(|&(x, y)| truth.map(x, y)).collect()
    }

    const SPARSE_SKY: [(f64, f64); 5] = [(212.0, 305.0), (640.5, 180.25), (780.0, 690.0), (330.0, 760.5), (505.0, 450.0)];

    #[test]
    fn a_disagreeing_fit_with_too_few_inliers_keeps_the_shift_and_says_why() {
        let reference = AlignmentStars { positions: SPARSE_SKY.to_vec(), median_fwhm_px: 2.5 };
        let target = viewed(&SPARSE_SKY, 0.6, (6.0, -4.0), 1000);
        match check_translation_against_star_lists(&reference, &target, -4.0, 6.0, 1000, 1000) {
            TranslationCheck::FitRejected(reason) => {
                assert!(
                    reason.starts_with("the star match disagrees with the (6.0, -4.0) px shift but is not decisive: 5 of 5 inliers at RMS "),
                    "{reason}"
                );
                assert!(reason.ends_with("needs at least 8 inliers and RMS <= 1.0 px"), "{reason}");
            }
            other => panic!("a five-star fit overrode the shift: {other:?}"),
        }
    }

    #[test]
    fn a_decisive_disagreeing_fit_is_preferred_over_the_shift() {
        let sky = ring_sky();
        let reference = AlignmentStars { positions: sky.clone(), median_fwhm_px: 2.5 };
        let target = viewed(&sky, 0.6, (6.0, -4.0), 1000);
        match check_translation_against_star_lists(&reference, &target, -4.0, 6.0, 1000, 1000) {
            TranslationCheck::Prefer(result) => {
                assert!(result.inliers >= 8, "{result:?}");
                let truth = rotated_and_shifted(0.6, (6.0, -4.0), 1000);
                assert!(largest_probe_displacement(&result.transform, &truth, 1000, 1000) < 0.5, "{result:?}");
            }
            other => panic!("a decisive fit was not preferred: {other:?}"),
        }
    }

    const HOT_CLUSTERS: [(f64, f64); 14] = [
        (82.0, 93.0), (308.0, 72.0), (615.0, 113.0), (922.0, 62.0), (72.0, 410.0), (963.0, 359.0), (62.0, 666.0),
        (953.0, 635.0), (123.0, 922.0), (390.0, 963.0), (666.0, 902.0), (932.0, 943.0), (205.0, 257.0), (820.0, 205.0),
    ];

    const MOVING_STARS: [(f64, f64); 10] = [
        (350.3, 360.7), (480.2, 345.1), (640.6, 372.4), (372.9, 500.5), (520.4, 470.8),
        (668.1, 530.2), (410.7, 640.3), (560.9, 610.6), (690.2, 680.9), (455.5, 700.4),
    ];

    fn with_hot_clusters(stars: &[(f64, f64)]) -> Vec<(f64, f64)> {
        HOT_CLUSTERS.iter().chain(stars).copied().collect()
    }

    fn ring_sky() -> Vec<(f64, f64)> {
        (0..12)
            .map(|i| {
                let angle = i as f64 * 0.5236 + 0.3;
                (499.5 + 320.0 * angle.cos() + 17.0 * (i % 3) as f64, 499.5 + 320.0 * angle.sin() - 11.0 * (i % 4) as f64)
            })
            .collect()
    }

    #[test]
    fn sources_fixed_on_the_sensor_are_dropped_and_the_moving_stars_decide() {
        let (dx, dy) = (7.3, -4.1);
        let moved: Vec<(f64, f64)> = MOVING_STARS.iter().map(|&(x, y)| (x + dx, y + dy)).collect();
        let reference = AlignmentStars { positions: with_hot_clusters(&MOVING_STARS), median_fwhm_px: 2.5 };
        let verdict = check_translation_against_star_lists(&reference, &with_hot_clusters(&moved), dy, dx, 1024, 1024);
        assert!(matches!(verdict, TranslationCheck::Agrees), "the moving stars confirm the shift: {verdict:?}");

        let sky = ring_sky();
        let reference = AlignmentStars { positions: with_hot_clusters(&sky), median_fwhm_px: 2.5 };
        let turned = viewed(&sky, 0.6, (6.0, -4.0), 1000);
        match check_translation_against_star_lists(&reference, &with_hot_clusters(&turned), -4.0, 6.0, 1000, 1000) {
            TranslationCheck::Prefer(result) => {
                let truth = rotated_and_shifted(0.6, (6.0, -4.0), 1000);
                assert!(largest_probe_displacement(&result.transform, &truth, 1000, 1000) < 0.5, "{result:?}");
            }
            other => panic!("the rotation of the moving stars was not registered: {other:?}"),
        }

        let reference = AlignmentStars { positions: with_hot_clusters(&MOVING_STARS[..3]), median_fwhm_px: 2.5 };
        match check_translation_against_star_lists(&reference, &with_hot_clusters(&moved[..3]), dy, dx, 1024, 1024) {
            TranslationCheck::FitRejected(reason) => assert_eq!(
                reason,
                "the star match is contaminated by 14 of 17 reference sources that sit at the same pixels in both frames despite the (7.3, -4.1) px shift; sources fixed on the sensor, such as hot pixels, are being matched as stars, and without them too few sources remain for a star match"
            ),
            other => panic!("the fixed pattern won: {other:?}"),
        }
    }

    fn scattered_sky(count: usize, size: usize, mut state: u64) -> Vec<(f64, f64)> {
        let mut unit = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let span = size as f64 - 120.0;
        (0..count).map(|_| (60.0 + unit() * span, 60.0 + unit() * span)).collect()
    }

    #[test]
    fn a_rotated_frame_with_a_small_dither_is_registered_through_the_star_transform() {
        let sky = scattered_sky(60, 2048, 0x51A7_7E5B_D17E_0001);
        let reference = AlignmentStars { positions: sky.clone(), median_fwhm_px: 2.5 };
        for (deg, shift) in [(0.2, (4.0, 2.0)), (0.3, (4.0, 0.0)), (0.25, (-3.0, 4.0))] {
            let target = viewed(&sky, deg, shift, 2048);
            let truth = rotated_and_shifted(deg, shift, 2048);
            match check_translation_against_star_lists(&reference, &target, shift.1, shift.0, 2048, 2048) {
                TranslationCheck::Prefer(result) => assert!(
                    largest_probe_displacement(&result.transform, &truth, 2048, 2048) < 0.5,
                    "{deg} deg with a {shift:?} px dither: {result:?}"
                ),
                other => panic!("{deg} deg with a {shift:?} px dither was not registered through the stars: {other:?}"),
            }
        }
    }

    #[test]
    fn a_small_dither_with_centroid_noise_is_not_taken_for_fixed_sources() {
        let sky = scattered_sky(60, 2048, 0x3D17_4E12_0C0F_FEE1);
        let (dx, dy) = (2.9, 1.6);
        let mut state = 0xC3A7_0B1D_5E5E_D001u64;
        let mut jitter = move || {
            let mut sum = 0.0;
            for _ in 0..3 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                sum += (state >> 11) as f64 / (1u64 << 53) as f64;
            }
            (sum - 1.5) * 0.6
        };
        let positions: Vec<(f64, f64)> = sky.iter().map(|&(x, y)| (x + jitter(), y + jitter())).collect();
        let reference = AlignmentStars { positions, median_fwhm_px: 2.5 };
        let target: Vec<(f64, f64)> = sky.iter().map(|&(x, y)| (x + dx + jitter(), y + dy + jitter())).collect();
        let verdict = check_translation_against_star_lists(&reference, &target, dy, dx, 2048, 2048);
        assert!(matches!(verdict, TranslationCheck::Agrees), "{verdict:?}");
    }

    #[test]
    fn a_weak_wide_rotation_fit_leaves_the_frame_out_instead_of_keeping_the_shift() {
        let reference = AlignmentStars { positions: SPARSE_SKY.to_vec(), median_fwhm_px: 2.5 };
        let target = viewed(&SPARSE_SKY, 180.0, (6.0, -4.0), 1000);
        match check_translation_against_star_lists(&reference, &target, -4.0, 6.0, 1000, 1000) {
            TranslationCheck::Refused(result) => {
                assert_eq!(result.method, AffineAlignMethod::Identity, "{result:?}");
                let reason = result.rejection.as_deref().unwrap_or_default();
                assert!(reason.starts_with("rotation ") && reason.contains("needs at least 8 inliers and RMS <= 1.0 px, got 5 of 5 matches"), "{reason}");
            }
            other => panic!("a frame turned by 180 deg was not left out: {other:?}"),
        }
    }

    #[test]
    fn a_four_inlier_mirrored_fit_does_not_leave_an_unmirrored_sparse_frame_out() {
        let params = &TriangleParams::ALIGNMENT;
        assert!(triangles::match_star_lists(&CHANCE_MIRROR_REFERENCE, &CHANCE_MIRROR_TARGET, params).flipped);
        let verdict = register_star_lists(&CHANCE_MIRROR_REFERENCE, &CHANCE_MIRROR_TARGET, 1024, 1024, params);
        assert!(verdict.is_err(), "the frame was not handed to phase correlation: {verdict:?}");
    }

    const FALSE_AFFINE_REFERENCE: [(f64, f64); 17] = [
        (289.5656748740216, 245.99161755919764),
        (208.80009158561927, 612.8819209919176),
        (520.9701970485187, 695.5887457857672),
        (390.535297691627, 272.4214833092252),
        (138.7517723207108, 74.48271247447005),
        (497.00548627246724, 249.5601868247227),
        (141.69682606419076, 885.7322050557246),
        (861.2803016257042, 708.817015714098),
        (352.33828057441605, 497.3836287180795),
        (577.8435739638203, 528.5417873372335),
        (510.92514005811296, 458.1567615786247),
        (845.0016266454782, 475.03186941014616),
        (568.4583704258077, 146.38733107167224),
        (712.595600790411, 427.8035318786565),
        (463.9728036348406, 351.536127139502),
        (817.5934000179938, 382.30256371218593),
        (740.1244119570497, 764.2880809923537),
    ];

    const FALSE_AFFINE_TARGET: [(f64, f64); 17] = [
        (429.72936969631667, 318.0264189886992),
        (364.0311219786802, 403.79536413670127),
        (68.92096893201862, 243.36706389095315),
        (495.50769836405476, 649.9978003213804),
        (667.3073047207206, 426.5758765223702),
        (244.4789344113895, 186.92591769765005),
        (615.177718598002, 945.1175457323249),
        (686.7733419736691, 364.95647216933935),
        (511.2853516045316, 78.76318925906254),
        (559.3115512900995, 475.637167668307),
        (439.27904543990803, 180.94239943700023),
        (819.5909108885461, 404.90568787983625),
        (490.96652040339285, 236.3777469484976),
        (564.8765661903578, 419.07288027550845),
        (459.87216427378075, 200.0061657133784),
        (679.2155479038763, 801.0856600658628),
        (194.57263208816974, 584.3467631378174),
    ];

    #[test]
    fn a_false_four_inlier_affine_beyond_the_raw_shift_limit_gives_way_to_the_rigid_fit() {
        let params = &TriangleParams::ALIGNMENT;
        let truth = rotated_and_shifted(-3.695993724347916, (-20.368615948732526, -48.69650174302055), 1024);
        let result = register_star_lists(&FALSE_AFFINE_REFERENCE, &FALSE_AFFINE_TARGET, 1024, 1024, params)
            .unwrap_or_else(|rejection| panic!("the frame lost its star registration: {rejection:?}"));
        let error = largest_corner_error(&result.transform, &truth, 1024);
        assert!(error < 1.5, "registered {error:.1} px away from the truth: {result:?}");
        assert_eq!(result.method, AffineAlignMethod::Rigid, "{result:?}");
    }

    #[test]
    fn a_reflection_is_named_as_such_whatever_the_quality_of_its_fit() {
        let mirror = AffineTransform { a: -1.0, b: 0.0, tx: 999.0, c: 0.0, d: 1.0, ty: 0.0 };
        for fit in [fitted(mirror, 40, 30, 0.1), fitted(mirror, 40, 7, 0.1), fitted(mirror, 40, 30, 1.5)] {
            let reason = sanity(&fit, 1000, 1000).expect_err("a reflection was accepted");
            assert!(reason.starts_with("reflection (det=-1.000)"), "{reason}");
            assert!(named_reflection(&fit, 1000, 1000), "{fit:?}");
        }
        let squeezed = AffineTransform { a: -0.5, b: 0.0, tx: 600.0, c: 0.0, d: 0.5, ty: 0.0 };
        let reason = sanity(&fitted(squeezed, 40, 30, 0.1), 1000, 1000).expect_err("accepted");
        assert!(reason.starts_with("scale"), "{reason}");
        assert!(!named_reflection(&fitted(squeezed, 40, 30, 0.1), 1000, 1000));
        let half_turn = rotation_about_centre(180.0, 1000, 1000);
        assert!(!named_reflection(&fitted(half_turn, 12, 7, 0.5), 1000, 1000));
    }

    #[test]
    fn a_column_major_frame_aligns_and_warps_like_its_row_major_copy() {
        use crate::core::alignment::triangles::{a1_frame, a1_reference, FrameView};
        use ndarray::ShapeBuilder;
        let reference = a1_reference(512, 60);
        let target = a1_frame(512, 60, FrameView::Rotated(3.0));
        let mut column_major = Array2::<f32>::zeros((512, 512).f());
        column_major.assign(&target);
        assert!(column_major.as_slice().is_none());

        let expected = align_channel_affine(&reference, &target);
        let found = align_channel_affine(&reference, &column_major);
        assert_eq!(found.method, expected.method);
        assert_eq!(found.inliers, expected.inliers);
        let (e, f) = (expected.transform, found.transform);
        assert_eq!([f.a, f.b, f.tx, f.c, f.d, f.ty], [e.a, e.b, e.tx, e.c, e.d, e.ty]);

        let warped = warp_image(&column_major, &found.transform, 512, 512);
        let reference_warp = warp_image(&target, &found.transform, 512, 512);
        assert!(warped.iter().zip(reference_warp.iter()).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    #[test]
    fn a_non_finite_transform_is_refused() {
        let mut broken = AffineTransform::identity();
        broken.tx = f64::NAN;
        assert!(sanity(&fitted(broken, 40, 30, 0.1), 1000, 1000).is_err());
    }

    #[test]
    fn test_identity_transform() {
        let t = AffineTransform::identity();
        let (x, y) = t.map(10.0, 20.0);
        assert!((x - 10.0).abs() < 1e-10);
        assert!((y - 20.0).abs() < 1e-10);
    }

    #[test]
    fn test_translation_transform() {
        let t = AffineTransform::translation(5.0, -3.0);
        let (x, y) = t.map(10.0, 20.0);
        assert!((x - 15.0).abs() < 1e-10);
        assert!((y - 17.0).abs() < 1e-10);
    }

    #[test]
    fn test_warp_identity() {
        let img = Array2::from_shape_fn((50, 50), |(r, c)| (r * 50 + c) as f32);
        let t = AffineTransform::identity();
        let warped = warp_image(&img, &t, 50, 50);
        for r in 2..48 {
            for c in 2..48 {
                let diff = (warped[[r, c]] - img[[r, c]]).abs();
                assert!(diff < 0.5, "diff={} at ({},{})", diff, r, c);
            }
        }
    }

    #[test]
    fn test_warp_translation() {
        let img = Array2::from_shape_fn((100, 100), |(r, c)| {
            let dy = (r as f64 - 50.0).abs();
            let dx = (c as f64 - 50.0).abs();
            if dy < 10.0 && dx < 10.0 { 1000.0 } else { 100.0 }
        });
        let t = AffineTransform::translation(5.0, 3.0);
        let warped = warp_image(&img, &t, 100, 100);
        assert!(warped[[47, 45]] > 500.0);
        assert!(warped[[50, 50]] > 500.0 || warped[[53, 55]] > warped[[50, 50]]);
    }

    #[test]
    fn test_nan_fill_outside_bounds() {
        let img = Array2::from_elem((50, 50), 100.0f32);
        let t = AffineTransform::translation(1000.0, 1000.0);
        let warped = warp_image(&img, &t, 50, 50);
        assert!(warped[[25, 25]].is_nan());
    }

    #[test]
    fn test_warp_identity_fills_edges() {
        let img = Array2::from_shape_fn((20, 20), |(r, c)| (r * 20 + c) as f32);
        let warped = warp_image(&img, &AffineTransform::identity(), 20, 20);
        assert!(warped[[19, 19]].is_finite());
        assert!(warped[[0, 19]].is_finite());
        assert!(warped[[19, 0]].is_finite());
    }

    #[test]
    fn test_warp_empty_source_is_all_nan() {
        let img = Array2::<f32>::zeros((0, 0));
        let warped = warp_image(&img, &AffineTransform::identity(), 5, 5);
        assert!(warped.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn a_sparse_star_list_keeps_its_same_parity_fit_when_the_mirrored_votes_are_higher() {
        let reference = [
            (622.83, 677.42), (262.93, 697.45), (625.19, 704.36), (497.51, 91.19), (376.52, 945.85),
            (943.44, 401.77), (431.83, 532.71), (895.37, 743.31), (525.99, 782.11), (702.74, 572.82),
            (769.7, 509.03), (278.03, 770.21), (158.7, 60.51), (266.82, 116.45), (591.28, 566.06),
            (98.0, 811.62), (548.4, 731.32),
        ];
        let target = [
            (677.21, 649.38), (318.31, 681.63), (680.3, 676.39), (532.51, 67.7), (440.48, 926.23),
            (770.23, 388.67), (66.56, 367.63), (730.85, 118.41), (119.67, 435.12), (62.14, 89.26),
            (672.42, 809.96), (330.95, 649.6), (243.44, 119.31), (611.99, 464.62), (242.08, 399.45),
            (805.76, 495.05), (640.8, 203.16),
        ];
        let params = &TriangleParams::ALIGNMENT;
        assert!(triangles::match_star_lists(&reference, &target, params).flipped);

        let result = register_star_lists(&reference, &target, 1000, 1000, params)
            .unwrap_or_else(|rejection| panic!("the same-parity fit was discarded: {rejection:?}"));
        assert!(
            matches!(result.method, AffineAlignMethod::Affine | AffineAlignMethod::Rigid) && result.rejection.is_none(),
            "{result:?}"
        );
        for (&(rx, ry), &(tx, ty)) in reference.iter().zip(&target).take(5) {
            let (px, py) = result.transform.map(rx, ry);
            assert!((px - tx).hypot(py - ty) < 1.0, "({rx}, {ry}) maps to ({px:.2}, {py:.2}): {result:?}");
        }
    }

    #[test]
    fn warp_image_with_an_empty_output_returns_an_empty_frame() {
        let img = Array2::from_elem((5, 5), 1.0f32);
        assert_eq!(warp_image(&img, &AffineTransform::identity(), 5, 0).dim(), (5, 0));
        assert_eq!(warp_image(&img, &AffineTransform::identity(), 0, 5).dim(), (0, 5));
    }

    #[test]
    fn test_check_transform_sanity_rejects_reflection() {
        let reflection = AffineTransform { a: 1.0, b: 0.0, tx: 0.0, c: 0.0, d: -1.0, ty: 0.0 };
        let result = AffineAlignResult {
            transform: reflection,
            matched_stars: 10,
            inliers: 10,
            residual_px: 0.5,
            rms_px: 0.5,
            method: AffineAlignMethod::Affine,
            confidence: 1.0,
            rejection: None,
        };
        assert!(sanity(&result, 100, 100).is_err());
    }

    #[test]
    fn test_check_transform_sanity_accepts_identity() {
        let result = AffineAlignResult {
            transform: AffineTransform::identity(),
            matched_stars: 10,
            inliers: 10,
            residual_px: 0.5,
            rms_px: 0.5,
            method: AffineAlignMethod::Affine,
            confidence: 1.0,
            rejection: None,
        };
        assert!(sanity(&result, 100, 100).is_ok());
    }

}
