use ndarray::Array2;
use rayon::prelude::*;

use crate::core::alignment::phase_correlation;
use crate::core::alignment::triangles::{self, StarPair, TriangleParams};
use crate::core::analysis::star_detection::detect_stars;
use crate::core::imaging::sampling::bicubic_sample;

const MIN_MATCHES_AFFINE: usize = 6;
const MIN_MATCHES_RIGID: usize = 4;
const DETECTION_SIGMA: f64 = 3.5;
const MAX_OFFSET_FRACTION: f64 = 0.40;
const MIN_SCALE: f64 = 0.70;
const MAX_SCALE: f64 = 1.40;
const MIN_REFLECTION_INLIERS: usize = 5;

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
    let (rows, cols) = reference.dim();

    let ref_norm = normalize_for_detection(reference);
    let tgt_norm = normalize_for_detection(target);

    let ref_det = detect_stars(&ref_norm, DETECTION_SIGMA);
    let tgt_det = detect_stars(&tgt_norm, DETECTION_SIGMA);

    log::info!(
        "Affine: detected {} ref stars, {} tgt stars (image {}x{})",
        ref_det.stars.len(), tgt_det.stars.len(), cols, rows
    );

    let params = &TriangleParams::ALIGNMENT;
    let ref_stars = triangles::select_alignment_stars(&ref_det.stars, params.max_stars, rows, cols);
    let tgt_stars = triangles::select_alignment_stars(&tgt_det.stars, params.max_stars, rows, cols);

    match register_star_lists(&ref_stars, &tgt_stars, rows, cols, params) {
        Ok(result) => result,
        Err(rejection) => fallback_phase_correlation(reference, target, rows, cols, rejection),
    }
}

fn register_star_lists(
    ref_stars: &[(f64, f64)],
    tgt_stars: &[(f64, f64)],
    rows: usize,
    cols: usize,
    params: &TriangleParams,
) -> Result<AffineAlignResult, Option<String>> {
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
) -> Result<AffineAlignResult, Option<String>> {
    if matches.len() < MIN_MATCHES_RIGID {
        log::info!(
            "Affine: too few triangle matches ({}), fallback to phase correlation",
            matches.len()
        );
        return Err(None);
    }

    let mut rejection: Option<String> = None;

    if matches.len() >= MIN_MATCHES_AFFINE {
        if let Some(result) = triangles::ransac_affine(matches, AffineAlignMethod::Affine, params) {
            match check_transform_sanity(&result, rows, cols, params) {
                Ok(()) => return Ok(result),
                Err(rejected) => {
                    log::warn!(
                        "Affine transform rejected: {}. stars={}, inliers={}, residual={:.2}px, rms={:.2}px. Trying rigid.",
                        rejected.reason, result.matched_stars, result.inliers, result.residual_px, result.rms_px
                    );
                    rejection = Some(rejected.reason);
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
                rejection.get_or_insert(rejected.reason);
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
    if !rejected.reflection || !is_decisive_reflection(&fit) {
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

struct SanityRejection {
    reason: String,
    reflection: bool,
}

impl SanityRejection {
    fn other(reason: String) -> Self {
        Self { reason, reflection: false }
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
            reflection: true,
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
        return Err(SanityRejection::other(format!(
            "rotation {:.2} deg needs at least {} inliers and RMS <= {:.1} px, got {} of {} matches at RMS {:.2} px",
            rotation,
            params.wide_rotation_inliers_needed(result.matched_stars),
            params.wide_rotation_max_rms_px,
            result.inliers,
            result.matched_stars,
            result.rms_px
        )));
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
        return AffineAlignResult {
            transform: AffineTransform::identity(),
            matched_stars: 0,
            inliers: 0,
            residual_px: 0.0,
            rms_px: 0.0,
            method: AffineAlignMethod::Identity,
            confidence: 0.0,
            rejection,
        };
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
            .is_err_and(|rejected| rejected.reflection)
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
        let edge = size as f64 - 1.0;
        let centre = edge * 0.5;
        [(0.0, 0.0), (edge, 0.0), (0.0, edge), (edge, edge), (centre, centre)]
            .iter()
            .map(|&(x, y)| {
                let (fx, fy) = found.map(x, y);
                let (tx, ty) = truth.map(x, y);
                (fx - tx).hypot(fy - ty)
            })
            .fold(0.0, f64::max)
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
