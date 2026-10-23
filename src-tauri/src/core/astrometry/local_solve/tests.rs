use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

use super::synthetic::{
    at_most, destination, detection, detections_from, exact_pairs, flat_frame, gradient_frame,
    hot_pixel_frame, image_centre_crpix, noise_frame, random_detections, random_in_cap,
    render_field, wcs_from_cards, FrameStyle, PixelDistortion, SyntheticProvider, TruthField,
    ARCSEC_PER_DEG, TIER1_CUT_STARS_PER_DEG2,
};
use super::*;
use crate::core::astrometry::solve_hints::{ScaleRange, SCALE_HINT_MARGIN};
use crate::core::astrometry::wcs::angular_separation;
use crate::core::stacking::{never_cancelled, no_stage};

const MAS_PER_DEG: f64 = 3_600_000.0;
const TAN_FIT_MAX_PX: f64 = 1e-4;
const TAN_FIT_MAX_MAS: f64 = 1.0;
const TAN_FIT_SIZE: (usize, usize) = (2000, 1400);
const TAN_FIT_LONG_SIDES_DEG: [f64; 3] = [0.25, 2.0, 20.0];
const SOLVED_WORST_CORNER_PX: f64 = 0.5;
const RANDOM_TRIALS: usize = 1000;
const DENSE_RANDOM_TRIALS: usize = 40;
const ODDS_SIZE: usize = 1000;
const ODDS_RADIUS_PX: f64 = 1.5;
const CANCEL_AFTER_CHECKS: usize = 4;
const SIP_PAIRS: usize = 300;
const SIP_NOISE_PX: f64 = 0.02;
const SIP_MAX_ERROR_PX: f64 = 0.05;
const SIP_EXTRAPOLATION_MAX_ERROR_PX: f64 = 0.5;
const SIP_CARD_MAX_ERR_PX: f64 = 0.01;
const SPIRAL_RANDOM_TRIALS: usize = 8;
const END_TO_END_MIN_PREDICTED: usize = 150;
const DISTORTED_MAX_ERROR_PX: f64 = 0.5;
const BINNING_FRAME: (usize, usize) = (1920, 1080);
const BINNING_SCALE_ARCSEC: f64 = 1.5;
const SMALL_FIELD_LONG_SIDE_DEG: f64 = 0.25;
const TIER0_LABEL: &str = "tier0-synthetic";
const TIER1_LABEL: &str = "tier1-synthetic";
const TIER2_LABEL: &str = "tier2-synthetic";
const TIER_CENTRE: (f64, f64) = (56.75, 24.12);
const FRAME: (usize, usize) = (1536, 1024);

fn grid(width: usize, height: usize, n: usize) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(n * n);
    for row in 0..n {
        for col in 0..n {
            out.push((
                -0.5 + width as f64 * col as f64 / (n - 1) as f64,
                -0.5 + height as f64 * row as f64 / (n - 1) as f64,
            ));
        }
    }
    out
}

fn worse(worst: f64, value: f64) -> f64 {
    if value.is_nan() || value > worst {
        value
    } else {
        worst
    }
}

fn tan_fit_errors(truth: &TruthField, fit: &TanFit, inliers: usize, label: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let (width, height) = truth.size();
    let centre = image_centre_crpix(width, height);
    if (fit.tan.crpix.0 - centre.0).abs() > 1e-9 || (fit.tan.crpix.1 - centre.1).abs() > 1e-9 {
        errors.push(format!(
            "{label}: CRPIX {:?}, expected the image centre {centre:?}",
            fit.tan.crpix
        ));
    }
    let fitted = TruthField::from_tan(fit.tan, width, height);
    let mut worst_mas: f64 = 0.0;
    let mut worst_px: f64 = 0.0;
    let tangent_point = (centre.0 - 1.0, centre.1 - 1.0);
    for (x, y) in grid(width, height, 11)
        .into_iter()
        .filter(|&node| node != tangent_point)
    {
        let (ra, dec) = truth.to_sky(x, y);
        let (fitted_ra, fitted_dec) = fitted.to_sky(x, y);
        worst_mas = worse(
            worst_mas,
            angular_separation(ra, dec, fitted_ra, fitted_dec) * MAS_PER_DEG,
        );
        let (fx, fy) = fitted.to_pixel(ra, dec);
        worst_px = worse(worst_px, (fx - x).hypot(fy - y));
    }
    if !at_most(worst_mas, TAN_FIT_MAX_MAS) {
        errors.push(format!(
            "{label}: sky error {worst_mas:.4} mas > {TAN_FIT_MAX_MAS} mas"
        ));
    }
    if !at_most(worst_px, TAN_FIT_MAX_PX) {
        errors.push(format!(
            "{label}: pixel error {worst_px:.3e} px > {TAN_FIT_MAX_PX} px"
        ));
    }
    if !at_most(fit.rms_px, TAN_FIT_MAX_PX) {
        errors.push(format!(
            "{label}: rms {:.3e} px > {TAN_FIT_MAX_PX} px",
            fit.rms_px
        ));
    }
    let kept = fit
        .inliers
        .iter()
        .take(inliers)
        .filter(|&&kept| kept)
        .count();
    if kept != inliers {
        errors.push(format!("{label}: {kept} of {inliers} exact pairs kept"));
    }
    errors
}

#[test]
fn tan_fit_recovers_a_synthetic_solution_to_1e_4_px_and_1_mas() {
    let params = LocalSolveParams::default();
    let mut failures = Vec::new();
    for (index, &long_side) in TAN_FIT_LONG_SIDES_DEG.iter().enumerate() {
        for flipped in [false, true] {
            let step = index as f64;
            let truth = TruthField::with_long_side(
                (44.0 + 150.0 * step, -30.0 + 35.0 * step),
                long_side,
                17.0 + 40.0 * step,
                flipped,
                TAN_FIT_SIZE.0,
                TAN_FIT_SIZE.1,
            );
            let pairs = exact_pairs(&truth, 200, 10 + index as u64);
            let start = destination(truth.centre(), 110.0, 0.25 * long_side);
            let label = format!("{long_side} deg, flipped {flipped}");
            match fit_tan(&pairs, truth.size(), start, &params) {
                None => failures.push(format!("{label}: fit_tan returned None")),
                Some(fit) => failures.extend(tan_fit_errors(&truth, &fit, pairs.len(), &label)),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn tan_fit_clips_gross_outliers_at_3_sigma() {
    let truth = TruthField::with_long_side((210.0, 12.0), 1.0, 75.0, false, 2000, 1400);
    let mut pairs = exact_pairs(&truth, 200, 3);
    let mut rng = StdRng::seed_from_u64(4);
    for _ in 0..12 {
        let x = rng.gen::<f64>() * 1999.0;
        let y = rng.gen::<f64>() * 1399.0;
        let shift = rng.gen_range(15.0..60.0);
        let angle = rng.gen::<f64>() * std::f64::consts::TAU;
        let (ra, dec) = truth.to_sky(x + shift * angle.cos(), y + shift * angle.sin());
        pairs.push(SkyPair { x, y, ra, dec });
    }
    let fit = fit_tan(
        &pairs,
        truth.size(),
        truth.centre(),
        &LocalSolveParams::default(),
    )
    .expect("fit_tan with 200 exact pairs and 12 outliers");
    let mut failures = tan_fit_errors(&truth, &fit, 200, "1 deg with 12 outliers");
    let flagged: Vec<usize> = (200..pairs.len())
        .filter(|&index| fit.inliers.get(index) != Some(&false))
        .collect();
    if fit.inliers.len() != pairs.len() || !flagged.is_empty() {
        failures.push(format!(
            "outliers kept as inliers: {flagged:?} (mask length {})",
            fit.inliers.len()
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn solve_detections(
    truth: &TruthField,
    provider: &SyntheticProvider,
    detections: &[Detection],
    hints: &SolveHints,
    params: &LocalSolveParams,
) -> Result<LocalSolution, SolveFailure> {
    solve_stars(
        detections,
        truth.size(),
        hints,
        provider,
        params,
        &never_cancelled,
        &no_stage,
    )
}

#[test]
fn a_flipped_synthetic_field_solves_with_parity_flipped() {
    let cases = [(0.0, false), (37.0, true), (179.5, true), (243.0, false)];
    let mut failures = Vec::new();
    for (index, &(rotation, flipped)) in cases.iter().enumerate() {
        let seed = 20 + index as u64;
        let truth = TruthField::with_long_side((210.8, 54.35), 1.0, rotation, flipped, 1600, 1066);
        let provider = SyntheticProvider::uniform(
            TIER1_LABEL,
            truth.centre(),
            2.5,
            truth.density_for(180.0),
            (7.0, 13.0),
            seed,
        );
        let detections = detections_from(&truth, provider.rows(), 13.0, 0.05, 25, seed);
        let hints = truth.hints(0.2, 70.0);
        let label = format!("rotation {rotation}, flipped {flipped}");
        match solve_detections(
            &truth,
            &provider,
            &detections,
            &hints,
            &LocalSolveParams::default(),
        ) {
            Err(failure) => failures.push(format!("{label}: {failure:?}")),
            Ok(solution) => {
                let expected = if flipped {
                    ImageParity::Flipped
                } else {
                    ImageParity::Normal
                };
                if solution.parity != expected {
                    failures.push(format!("{label}: parity {:?}", solution.parity));
                }
                let orientation = wcs_from_cards(&solution.wcs_cards, truth.width, truth.height)
                    .orientation(truth.width, truth.height);
                if orientation.flipped != flipped {
                    failures.push(format!("{label}: cards flipped {}", orientation.flipped));
                }
                let corner = truth.worst_corner_px(&solution.wcs_cards);
                if !at_most(corner, SOLVED_WORST_CORNER_PX) {
                    failures.push(format!("{label}: worst corner {corner:.3} px"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn refused_for_a_named_reason(best: &CandidateNumbers) -> bool {
    best.matched < 8 || best.false_alarm_log10 >= -9.0 || best.rms_px > 1.0
}

fn random_list_outcome(
    trial: usize,
    truth: &TruthField,
    provider: &SyntheticProvider,
    detections: &[Detection],
    params: &LocalSolveParams,
) -> Option<String> {
    let hints = truth.hints(0.0, 0.0);
    match solve_detections(truth, provider, detections, &hints, params) {
        Ok(solution) => Some(format!(
            "trial {trial}: accepted with {} matched of {} predicted, log10 P {:.2}, rms {:.3} px",
            solution.matched, solution.predicted, solution.false_alarm_log10, solution.rms_px
        )),
        Err(failure) if failure.reason != FailureReason::NoSolution => Some(format!(
            "trial {trial}: {:?} {}",
            failure.reason, failure.message
        )),
        Err(failure) => failure
            .best
            .as_ref()
            .filter(|best| !refused_for_a_named_reason(best))
            .map(|best| format!("trial {trial}: best candidate passes the gates {best:?}")),
    }
}

#[test]
fn a_random_star_list_is_refused_over_the_whole_default_spiral() {
    let truth = TruthField::with_long_side((83.8, -5.4), 1.0, 0.0, false, 1500, 1000);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        4.0,
        truth.density_for(60.0),
        (8.0, 14.0),
        7,
    );
    let params = LocalSolveParams::default();
    let accepted: Vec<String> = (0..SPIRAL_RANDOM_TRIALS)
        .into_par_iter()
        .filter_map(|trial| {
            let count = StdRng::seed_from_u64(500 + trial as u64).gen_range(10..=40);
            let detections =
                random_detections(count, truth.width, truth.height, 500 + trial as u64);
            random_list_outcome(trial, &truth, &provider, &detections, &params)
        })
        .collect();
    assert!(
        accepted.is_empty(),
        "{} of {SPIRAL_RANDOM_TRIALS} random lists were not refused over the spiral:\n{}",
        accepted.len(),
        accepted.join("\n")
    );
}

#[test]
fn a_random_star_list_never_passes_the_odds_gate_in_1000_trials() {
    let truth = TruthField::with_long_side((83.8, -5.4), 1.0, 0.0, false, 1500, 1000);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        2.0,
        truth.density_for(60.0),
        (8.0, 14.0),
        7,
    );
    let params = LocalSolveParams {
        near_radius_deg: 0.0,
        ..LocalSolveParams::default()
    };
    let accepted: Vec<String> = (0..RANDOM_TRIALS)
        .into_par_iter()
        .filter_map(|trial| {
            let count = StdRng::seed_from_u64(trial as u64).gen_range(10..=40);
            let detections = random_detections(count, truth.width, truth.height, trial as u64);
            random_list_outcome(trial, &truth, &provider, &detections, &params)
        })
        .collect();
    assert!(
        accepted.is_empty(),
        "{} of {RANDOM_TRIALS} random lists were not refused:\n{}",
        accepted.len(),
        accepted.join("\n")
    );
}

#[test]
fn a_dense_random_star_list_never_passes_the_odds_gate() {
    let truth = TruthField::with_long_side((266.4, -28.9), 1.0, 0.0, false, 1500, 1000);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        2.0,
        truth.density_for(400.0),
        (8.0, 14.0),
        8,
    );
    let params = LocalSolveParams {
        near_radius_deg: 0.0,
        ..LocalSolveParams::default()
    };
    let accepted: Vec<String> = (0..DENSE_RANDOM_TRIALS)
        .into_par_iter()
        .filter_map(|trial| {
            let count = StdRng::seed_from_u64(9000 + trial as u64).gen_range(80..=150);
            let detections =
                random_detections(count, truth.width, truth.height, 9000 + trial as u64);
            random_list_outcome(trial, &truth, &provider, &detections, &params)
        })
        .collect();
    assert!(accepted.is_empty(), "{}", accepted.join("\n"));
}

struct OddsCase {
    tan: crate::core::registration::poly::TanParams,
    detections: Vec<Detection>,
    catalogue: Vec<(f64, f64)>,
}

fn pixel_distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn odds_case(
    n_cat: usize,
    n_det: usize,
    true_matches: usize,
    offset_px: f64,
    fwhm: f64,
    seed: u64,
) -> OddsCase {
    let truth = TruthField::new((150.0, 2.2), 2.0, 31.0, false, ODDS_SIZE, ODDS_SIZE);
    let exclusion = 3.0 * ODDS_RADIUS_PX.max(1.5 * fwhm);
    let mut rng = StdRng::seed_from_u64(seed);
    let margin = 5.0;
    let span = ODDS_SIZE as f64 - 1.0 - 2.0 * margin;
    let mut catalogue_px: Vec<(f64, f64)> = Vec::with_capacity(n_cat);
    while catalogue_px.len() < n_cat {
        let p = (
            margin + span * rng.gen::<f64>(),
            margin + span * rng.gen::<f64>(),
        );
        if catalogue_px
            .iter()
            .all(|&q| pixel_distance(p, q) >= exclusion)
        {
            catalogue_px.push(p);
        }
    }
    let mut detections: Vec<Detection> = catalogue_px
        .iter()
        .take(true_matches)
        .map(|&(x, y)| detection(x + offset_px, y, 1000.0, fwhm))
        .collect();
    while detections.len() < n_det {
        let p = (
            rng.gen::<f64>() * (ODDS_SIZE as f64 - 1.0),
            rng.gen::<f64>() * (ODDS_SIZE as f64 - 1.0),
        );
        if catalogue_px
            .iter()
            .all(|&q| pixel_distance(p, q) >= exclusion)
        {
            detections.push(detection(p.0, p.1, 500.0, fwhm));
        }
    }
    OddsCase {
        tan: truth.tan,
        detections,
        catalogue: catalogue_px
            .iter()
            .map(|&(x, y)| truth.to_sky(x, y))
            .collect(),
    }
}

fn reference_tail_log10(k: usize, lambda: f64) -> f64 {
    if k == 0 {
        return 0.0;
    }
    if lambda <= 0.0 {
        return f64::NEG_INFINITY;
    }
    let ln_lambda = lambda.ln();
    let mut ln_factorial: f64 = (1..=k).map(|i| (i as f64).ln()).sum();
    let mut terms = Vec::with_capacity(600);
    for i in k..k + 600 {
        if i > k {
            ln_factorial += (i as f64).ln();
        }
        terms.push(-lambda + i as f64 * ln_lambda - ln_factorial);
    }
    let peak = terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let sum: f64 = terms.iter().map(|t| (t - peak).exp()).sum();
    (peak + sum.ln()) / std::f64::consts::LN_10
}

#[test]
fn poisson_tail_log10_matches_a_direct_sum() {
    let cases = [
        (8, 0.282),
        (8, 1.131),
        (12, 1.0),
        (1, 0.5),
        (3, 5.0),
        (30, 3.0),
        (100, 0.01),
    ];
    let mut failures = Vec::new();
    for (k, lambda) in cases {
        let got = poisson_tail_log10(k, lambda);
        let expected = reference_tail_log10(k, lambda);
        if !at_most((got - expected).abs(), 1e-9 * expected.abs().max(1.0)) {
            failures.push(format!("k {k}, lambda {lambda}: {got} vs {expected}"));
        }
    }
    if poisson_tail_log10(0, 3.0) != 0.0 {
        failures.push("k 0 must give log10 P = 0".to_string());
    }
    if poisson_tail_log10(1, 0.0) != f64::NEG_INFINITY {
        failures.push("lambda 0 with k 1 must give log10 P = -inf".to_string());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn eight_true_matches_in_a_dense_field_pass_only_when_p_below_1e_9() {
    let params = LocalSolveParams::default();
    let area = (ODDS_SIZE * ODDS_SIZE) as f64;
    let mut failures = Vec::new();
    let mut outcomes = BTreeSet::new();
    for n in (40..=400).step_by(18) {
        let case = odds_case(n, n, 8, 0.0, 1.0, n as u64);
        let v = verify(
            &case.tan,
            &case.detections,
            &case.catalogue,
            (ODDS_SIZE, ODDS_SIZE),
            Some((1.8, 2.2)),
            &params,
        );
        let lambda = (n * n) as f64 * std::f64::consts::PI * ODDS_RADIUS_PX.powi(2) / area;
        let expected_log10 = reference_tail_log10(8 - TAN_HYPOTHESIS_PAIRS, lambda);
        let expected_accept = expected_log10 < params.max_false_alarm.log10();
        outcomes.insert(expected_accept);
        let checks = [
            (v.matched == 8, format!("matched {}", v.matched)),
            (v.predicted == n, format!("predicted {}", v.predicted)),
            (v.detections == n, format!("detections {}", v.detections)),
            (
                (v.radius_px - ODDS_RADIUS_PX).abs() < 1e-12,
                format!("radius {}", v.radius_px),
            ),
            (
                (v.lambda - lambda).abs() <= 1e-9 * lambda,
                format!("lambda {} vs {lambda}", v.lambda),
            ),
            (
                (v.false_alarm_log10 - expected_log10).abs() <= 1e-6,
                format!("log10 P {} vs {expected_log10}", v.false_alarm_log10),
            ),
            (
                v.accepted == expected_accept,
                format!("accepted {} expected {expected_accept}", v.accepted),
            ),
        ];
        for (ok, what) in checks {
            if !ok {
                failures.push(format!("n {n}: {what}"));
            }
        }
    }
    assert_eq!(outcomes.len(), 2, "the sweep must cross the 1e-9 threshold");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn eight_matches_at_a_normal_density_are_refused_and_eleven_pass() {
    let params = LocalSolveParams::default();
    let mut failures = Vec::new();
    for (true_matches, expected) in [(8, false), (9, false), (11, true)] {
        let case = odds_case(45, 80, true_matches, 0.0, 3.0, 300 + true_matches as u64);
        let v = verify(
            &case.tan,
            &case.detections,
            &case.catalogue,
            (ODDS_SIZE, ODDS_SIZE),
            Some((1.8, 2.2)),
            &params,
        );
        if v.matched != true_matches || v.accepted != expected {
            failures.push(format!(
                "{true_matches} true matches, lambda {:.3}: matched {}, log10 P {:.2}, accepted {}",
                v.lambda, v.matched, v.false_alarm_log10, v.accepted
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn seven_matches_never_pass_even_when_p_is_tiny() {
    let case = odds_case(20, 20, 7, 0.0, 1.0, 70);
    let v = verify(
        &case.tan,
        &case.detections,
        &case.catalogue,
        (ODDS_SIZE, ODDS_SIZE),
        Some((1.8, 2.2)),
        &LocalSolveParams::default(),
    );
    assert_eq!(v.matched, 7, "{v:?}");
    assert!(v.false_alarm_log10 < -9.0, "{v:?}");
    assert!(!v.accepted, "{v:?}");
}

#[test]
fn an_rms_above_1_px_or_a_scale_outside_the_hint_is_refused() {
    let params = LocalSolveParams::default();
    let loose = odds_case(30, 30, 20, 1.2, 2.0, 71);
    let v = verify(
        &loose.tan,
        &loose.detections,
        &loose.catalogue,
        (ODDS_SIZE, ODDS_SIZE),
        Some((1.8, 2.2)),
        &params,
    );
    assert_eq!(v.matched, 20, "{v:?}");
    assert!((v.rms_px - 1.2).abs() < 1e-6, "{v:?}");
    assert!(!v.accepted, "rms above 1 px accepted: {v:?}");
    let tight = odds_case(30, 30, 20, 0.0, 1.0, 72);
    let off_scale = verify(
        &tight.tan,
        &tight.detections,
        &tight.catalogue,
        (ODDS_SIZE, ODDS_SIZE),
        Some((3.0, 4.0)),
        &params,
    );
    assert!(off_scale.corners_finite, "{off_scale:?}");
    assert!(!off_scale.scale_in_range, "{off_scale:?}");
    assert!(
        !off_scale.accepted,
        "scale outside the hint accepted: {off_scale:?}"
    );
}

#[test]
fn the_spiral_starts_at_the_hint_and_covers_the_search_disc() {
    let cases = [
        ((10.0, 20.0), 0.333, 3.0),
        ((200.0, 88.5), 0.5, 3.0),
        ((359.9, -45.0), 1.0, 15.0),
    ];
    let mut failures = Vec::new();
    for (hint, spacing, radius) in cases {
        let rings = spiral_centres(hint, spacing, radius);
        let label = format!("hint {hint:?}, spacing {spacing}, radius {radius}");
        if rings.first() != Some(&vec![hint]) {
            failures.push(format!("{label}: first ring {:?}", rings.first()));
        }
        let max_rings = (radius / (spacing * 3f64.sqrt() / 2.0)).ceil() as usize + 2;
        if rings.len() > max_rings {
            failures.push(format!(
                "{label}: {} rings, at most {max_rings}",
                rings.len()
            ));
        }
        for (k, ring) in rings.iter().enumerate().skip(1) {
            let full = 6 * k;
            let last = k + 1 == rings.len();
            if ring.len() > full || (!last && ring.len() != full) || ring.is_empty() {
                failures.push(format!("{label}: ring {k} has {} centres", ring.len()));
            }
            for &(ra, dec) in ring {
                let distance = angular_separation(hint.0, hint.1, ra, dec);
                let low = 0.85 * k as f64 * spacing * 3f64.sqrt() / 2.0;
                let high = 1.05 * k as f64 * spacing;
                let valid = (0.0..360.0).contains(&ra) && (-90.0..=90.0).contains(&dec);
                if !valid || !(low..=high).contains(&distance) {
                    failures.push(format!(
                        "{label}: ring {k} centre ({ra}, {dec}) at {distance} deg"
                    ));
                }
            }
        }
        let centres = rings.concat();
        let covering = 1.05 * spacing / 3f64.sqrt();
        let mut rng = StdRng::seed_from_u64(5);
        let uncovered = (0..2000)
            .map(|_| random_in_cap(&mut rng, hint, radius))
            .filter(|p| {
                centres
                    .iter()
                    .map(|c| angular_separation(p.0, p.1, c.0, c.1))
                    .fold(f64::INFINITY, f64::min)
                    > covering
            })
            .count();
        if uncovered > 0 {
            failures.push(format!("{label}: {uncovered} of 2000 points uncovered"));
        }
    }
    if spiral_centres((10.0, 20.0), 0.5, 0.0) != vec![vec![(10.0, 20.0)]] {
        failures.push("radius 0 must give the hint alone".to_string());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_hint_one_field_off_is_found_on_the_spiral() {
    let truth = TruthField::with_long_side((280.4, 38.8), 1.0, 63.0, false, 1500, 1000);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        3.5,
        truth.density_for(150.0),
        (8.0, 14.0),
        41,
    );
    let detections = detections_from(&truth, provider.rows(), 14.0, 0.05, 30, 42);
    let hints = truth.hints(1.0, 135.0);
    let params = LocalSolveParams {
        near_radius_deg: 1.5,
        ..LocalSolveParams::default()
    };
    let stages = Mutex::new(Vec::new());
    let record = |stage: &str| stages.lock().expect("stage log").push(stage.to_string());
    let solution = solve_stars(
        &detections,
        truth.size(),
        &hints,
        &provider,
        &params,
        &never_cancelled,
        &record,
    )
    .unwrap_or_else(|failure| panic!("one field off: {failure:?}"));
    let corner = truth.worst_corner_px(&solution.wcs_cards);
    assert!(corner <= SOLVED_WORST_CORNER_PX, "worst corner {corner} px");
    assert!(solution.centres_tried > 1, "{solution:?}");
    let stages = stages.into_inner().expect("stage log");
    let rings: BTreeSet<&String> = stages
        .iter()
        .filter(|stage| stage.starts_with(STAGE_SEARCH))
        .collect();
    assert!(rings.len() >= 2, "search rings reported: {stages:?}");
}

#[test]
#[ignore]
fn a_hint_2_5_degrees_off_is_found_on_the_spiral_within_3_s() {
    if cfg!(debug_assertions) {
        panic!("the 3 s budget is for the release profile: cargo test --release --lib -- --ignored a_hint_2_5_degrees_off");
    }
    let truth = TruthField::with_long_side((101.3, -16.7), 1.0, 25.0, false, 3000, 2000);
    let hints = truth.hints(2.5 / truth.long_side_deg(), 300.0);
    let hint = hints.centre.as_ref().map(|c| (c.ra, c.dec)).expect("hint");
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        hint,
        4.0,
        TIER1_CUT_STARS_PER_DEG2,
        (6.0, 15.5),
        77,
    );
    let image = render_field(
        &truth,
        provider.rows(),
        15.5,
        &FrameStyle {
            uncatalogued: 3000,
            seed: 78,
            ..FrameStyle::default()
        },
    );
    let started = Instant::now();
    let outcome = solve_local(
        &image,
        &hints,
        &provider,
        &LocalSolveParams::default(),
        &never_cancelled,
        &no_stage,
    );
    let elapsed = started.elapsed();
    let solution = outcome.unwrap_or_else(|failure| panic!("2.5 deg off: {failure:?}"));
    eprintln!(
        "near search 2.5 deg: {elapsed:?}, {} centres, {} matched, rms {:.3} px",
        solution.centres_tried, solution.matched, solution.rms_px
    );
    let corner = truth.worst_corner_px(&solution.wcs_cards);
    assert!(corner <= 1.0, "worst corner {corner} px");
    assert!(
        elapsed <= Duration::from_secs(3),
        "near search took {elapsed:?}"
    );
}

#[test]
fn cancel_stops_the_spiral_within_one_centre() {
    let truth = TruthField::with_long_side((30.0, -60.0), 1.0, 0.0, false, 1500, 1000);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        3.0,
        truth.density_for(120.0),
        (8.0, 14.0),
        3,
    );
    let detections = random_detections(40, truth.width, truth.height, 9);
    let hints = truth.hints(0.0, 0.0);
    let params = LocalSolveParams {
        near_radius_deg: MAX_NEAR_RADIUS_DEG,
        ..LocalSolveParams::default()
    };
    let checks = AtomicUsize::new(0);
    let first_true: Mutex<Option<Instant>> = Mutex::new(None);
    let cancel = || {
        if checks.fetch_add(1, Ordering::SeqCst) + 1 < CANCEL_AFTER_CHECKS {
            return false;
        }
        first_true
            .lock()
            .expect("cancel clock")
            .get_or_insert_with(Instant::now);
        true
    };
    let failure = solve_stars(
        &detections,
        truth.size(),
        &hints,
        &provider,
        &params,
        &cancel,
        &no_stage,
    )
    .expect_err("a cancelled search returns a failure");
    let returned = Instant::now();
    assert_eq!(failure.reason, FailureReason::Cancelled, "{failure:?}");
    let allowed = CANCEL_AFTER_CHECKS - 1 + rayon::current_num_threads();
    assert!(
        failure.centres_tried <= allowed,
        "{} centres tried after cancel, at most {allowed}",
        failure.centres_tried
    );
    let cancelled_at = first_true
        .into_inner()
        .expect("cancel clock")
        .expect("cancel was consulted");
    let reaction = returned.duration_since(cancelled_at);
    if !cfg!(debug_assertions) {
        assert!(
            reaction <= Duration::from_secs(1),
            "returned {reaction:?} after cancel"
        );
    }
    let already = solve_stars(
        &detections,
        truth.size(),
        &hints,
        &provider,
        &params,
        &|| true,
        &no_stage,
    )
    .expect_err("a search cancelled before it starts returns a failure");
    assert_eq!(already.reason, FailureReason::Cancelled, "{already:?}");
    assert_eq!(already.centres_tried, 0, "{already:?}");
}

fn radial_cubic(truth: &TruthField, corner_px: f64) -> impl Fn(f64, f64) -> (f64, f64) {
    let (xc, yc) = (
        (truth.width as f64 - 1.0) / 2.0,
        (truth.height as f64 - 1.0) / 2.0,
    );
    let half_diagonal = (truth.width as f64 / 2.0).hypot(truth.height as f64 / 2.0);
    move |x, y| {
        let (u, v) = ((x - xc) / half_diagonal, (y - yc) / half_diagonal);
        let r2 = u * u + v * v;
        (x + corner_px * r2 * u, y + corner_px * r2 * v)
    }
}

fn with_quartic(
    truth: &TruthField,
    corner_px: f64,
    quartic_px: f64,
) -> impl Fn(f64, f64) -> (f64, f64) {
    let cubic = radial_cubic(truth, corner_px);
    let (xc, yc) = (
        (truth.width as f64 - 1.0) / 2.0,
        (truth.height as f64 - 1.0) / 2.0,
    );
    let half_diagonal = (truth.width as f64 / 2.0).hypot(truth.height as f64 / 2.0);
    move |x, y| {
        let (u, v) = ((x - xc) / half_diagonal, (y - yc) / half_diagonal);
        let (dx, dy) = cubic(x, y);
        (
            dx + quartic_px * (u.powi(4) + 3.0 * u * u * v * v),
            dy + quartic_px * (v.powi(4) - 2.0 * u * u * v * v),
        )
    }
}

fn distorted_pairs(
    truth: &TruthField,
    distortion: &dyn Fn(f64, f64) -> (f64, f64),
    count: usize,
    seed: u64,
) -> Vec<SkyPair> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..count)
        .map(|_| {
            let x = rng.gen::<f64>() * (truth.width as f64 - 1.0);
            let y = rng.gen::<f64>() * (truth.height as f64 - 1.0);
            let (dx, dy) = distortion(x, y);
            let (ra, dec) = truth.to_sky(dx, dy);
            SkyPair {
                x: x + SIP_NOISE_PX * super::synthetic::gaussian(&mut rng),
                y: y + SIP_NOISE_PX * super::synthetic::gaussian(&mut rng),
                ra,
                dec,
            }
        })
        .collect()
}

fn distortion_error_px(
    truth: &TruthField,
    distortion: &dyn Fn(f64, f64) -> (f64, f64),
    cards: &[(String, String)],
) -> (f64, f64) {
    let wcs = wcs_from_cards(cards, truth.width, truth.height);
    let scale_deg = truth.scale_arcsec() / ARCSEC_PER_DEG;
    let mut forward: f64 = 0.0;
    let mut inverse: f64 = 0.0;
    for (x, y) in grid(truth.width - 1, truth.height - 1, 9) {
        let (x, y) = (x + 0.5, y + 0.5);
        let (dx, dy) = distortion(x, y);
        let (ra, dec) = truth.to_sky(dx, dy);
        let got = wcs.pixel_to_world(x, y);
        forward = worse(
            forward,
            angular_separation(ra, dec, got.ra, got.dec) / scale_deg,
        );
        let (px, py) = wcs.world_to_pixel(ra, dec);
        inverse = worse(inverse, (px - x).hypot(py - y));
    }
    (forward, inverse)
}

fn rendered_distortion_error_px(
    truth: &TruthField,
    distortion: &dyn Fn(f64, f64) -> (f64, f64),
    cards: &[(String, String)],
) -> (f64, f64) {
    let wcs = wcs_from_cards(cards, truth.width, truth.height);
    let scale_deg = truth.scale_arcsec() / ARCSEC_PER_DEG;
    let mut forward: f64 = 0.0;
    let mut inverse: f64 = 0.0;
    for (x, y) in grid(truth.width - 1, truth.height - 1, 9) {
        let (x, y) = (x + 0.5, y + 0.5);
        let (mx, my) = distortion(x, y);
        let (ra, dec) = truth.to_sky(x, y);
        let got = wcs.pixel_to_world(mx, my);
        forward = worse(
            forward,
            angular_separation(ra, dec, got.ra, got.dec) / scale_deg,
        );
        let (px, py) = wcs.world_to_pixel(ra, dec);
        inverse = worse(inverse, (px - mx).hypot(py - my));
    }
    (forward, inverse)
}

fn sip_case_errors(
    label: &str,
    truth: &TruthField,
    distortion: &dyn Fn(f64, f64) -> (f64, f64),
    pairs: &[SkyPair],
    expected_order: u8,
    expected_max_considered: u8,
    max_error_px: Option<f64>,
) -> Vec<String> {
    let fit = match fit_distortion(
        pairs,
        &truth.tan,
        truth.size(),
        &LocalSolveParams::default(),
    ) {
        Ok(fit) => fit,
        Err(error) => return vec![format!("{label}: {error}")],
    };
    let mut errors = Vec::new();
    if fit.order != expected_order {
        errors.push(format!(
            "{label}: order {} expected {expected_order}, considered {:?}",
            fit.order, fit.considered
        ));
    }
    let max_considered = fit.considered.iter().map(|c| c.0).max();
    if expected_order > 1 && max_considered != Some(expected_max_considered) {
        errors.push(format!(
            "{label}: highest order considered {max_considered:?}, expected {expected_max_considered}"
        ));
    }
    let has_sip = fit.wcs_cards.iter().any(|(key, _)| key == "A_ORDER");
    match (expected_order > 1, fit.sip_max_err_px) {
        (true, Some(err)) if err <= SIP_CARD_MAX_ERR_PX && has_sip => {}
        (false, None) if !has_sip => {}
        (_, err) => errors.push(format!(
            "{label}: SIPMXERR {err:?}, A_ORDER present {has_sip}"
        )),
    }
    if let Some(limit) = max_error_px {
        let (forward, inverse) = distortion_error_px(truth, distortion, &fit.wcs_cards);
        if !(forward <= limit && inverse <= limit) {
            errors.push(format!(
                "{label}: forward {forward:.4} px, inverse {inverse:.4} px, limit {limit} px"
            ));
        }
    }
    errors
}

type SipCase = (&'static str, f64, f64, f64, u8, u8, Option<f64>);

#[test]
fn order_3_distortion_is_recovered_and_order_4_is_chosen_only_above_5_degrees() {
    let mut failures = Vec::new();
    let cases: [SipCase; 5] = [
        (
            "2 deg, order 3",
            2.0,
            4.0,
            0.0,
            3,
            3,
            Some(SIP_MAX_ERROR_PX),
        ),
        ("3 deg, order 4 terms", 3.0, 4.0, 2.0, 3, 3, None),
        (
            "10 deg, order 4 terms",
            10.0,
            4.0,
            2.0,
            4,
            4,
            Some(SIP_MAX_ERROR_PX),
        ),
        (
            "10 deg, order 3",
            10.0,
            4.0,
            0.0,
            3,
            4,
            Some(SIP_MAX_ERROR_PX),
        ),
        (
            "2 deg, pure TAN",
            2.0,
            0.0,
            0.0,
            1,
            3,
            Some(SIP_MAX_ERROR_PX),
        ),
    ];
    for (index, &(label, long_side, cubic_px, quartic_px, order, considered, accuracy)) in
        cases.iter().enumerate()
    {
        let truth = TruthField::with_long_side(
            (120.0 + 30.0 * index as f64, 15.0),
            long_side,
            12.0,
            false,
            1500,
            1000,
        );
        let distortion = with_quartic(&truth, cubic_px, quartic_px);
        let pairs = distorted_pairs(&truth, &distortion, SIP_PAIRS, 60 + index as u64);
        failures.extend(sip_case_errors(
            label,
            &truth,
            &distortion,
            &pairs,
            order,
            considered,
            accuracy,
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn sip_needs_40_inliers_over_3_quadrants() {
    let truth = TruthField::with_long_side((300.0, -20.0), 2.0, 0.0, false, 1500, 1000);
    let distortion = radial_cubic(&truth, 4.0);
    let all = distorted_pairs(&truth, &distortion, 1200, 90);
    let (xc, yc) = (
        (truth.width as f64 - 1.0) / 2.0,
        (truth.height as f64 - 1.0) / 2.0,
    );
    let few: Vec<SkyPair> = all.iter().copied().take(39).collect();
    let left: Vec<SkyPair> = all.iter().copied().filter(|p| p.x < xc).take(80).collect();
    let three: Vec<SkyPair> = all
        .iter()
        .copied()
        .filter(|p| !(p.x >= xc && p.y >= yc))
        .take(80)
        .collect();
    let mut failures = Vec::new();
    failures.extend(sip_case_errors(
        "39 pairs",
        &truth,
        &distortion,
        &few,
        1,
        3,
        None,
    ));
    failures.extend(sip_case_errors(
        "80 pairs, 2 quadrants",
        &truth,
        &distortion,
        &left,
        1,
        3,
        None,
    ));
    failures.extend(sip_case_errors(
        "80 pairs, 3 quadrants",
        &truth,
        &distortion,
        &three,
        3,
        3,
        Some(SIP_EXTRAPOLATION_MAX_ERROR_PX),
    ));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

struct EndToEnd {
    long_side_deg: f64,
    centre: (f64, f64),
    rotation_deg: f64,
    flipped: bool,
    corner_px: f64,
}

const END_TO_END: [EndToEnd; 10] = [
    EndToEnd {
        long_side_deg: 0.25,
        centre: (10.68, 41.27),
        rotation_deg: 0.0,
        flipped: false,
        corner_px: 0.0,
    },
    EndToEnd {
        long_side_deg: 0.25,
        centre: (359.95, -0.5),
        rotation_deg: 123.0,
        flipped: true,
        corner_px: 0.0,
    },
    EndToEnd {
        long_side_deg: 1.0,
        centre: (83.82, -5.39),
        rotation_deg: 37.0,
        flipped: false,
        corner_px: 0.0,
    },
    EndToEnd {
        long_side_deg: 1.0,
        centre: (250.0, 85.0),
        rotation_deg: 211.0,
        flipped: true,
        corner_px: 0.0,
    },
    EndToEnd {
        long_side_deg: 5.0,
        centre: (192.86, 27.13),
        rotation_deg: 90.0,
        flipped: false,
        corner_px: 0.0,
    },
    EndToEnd {
        long_side_deg: 5.0,
        centre: (266.4, -28.9),
        rotation_deg: 300.0,
        flipped: true,
        corner_px: 0.0,
    },
    EndToEnd {
        long_side_deg: 20.0,
        centre: (150.0, 20.0),
        rotation_deg: 180.0,
        flipped: false,
        corner_px: 0.0,
    },
    EndToEnd {
        long_side_deg: 20.0,
        centre: (30.0, -70.0),
        rotation_deg: 17.0,
        flipped: true,
        corner_px: 0.0,
    },
    EndToEnd {
        long_side_deg: 12.0,
        centre: (95.0, 35.0),
        rotation_deg: 45.0,
        flipped: false,
        corner_px: 12.0,
    },
    EndToEnd {
        long_side_deg: 6.0,
        centre: (320.0, -42.0),
        rotation_deg: 200.0,
        flipped: true,
        corner_px: 8.0,
    },
];

fn angle_difference_deg(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn end_to_end_errors(
    label: &str,
    truth: &TruthField,
    outcome: Result<LocalSolution, SolveFailure>,
    flipped: bool,
    corner_px: f64,
) -> Vec<String> {
    let solution = match outcome {
        Ok(solution) => solution,
        Err(failure) => return vec![format!("{label}: {failure:?}")],
    };
    eprintln!(
        "{label}: {} matched of {} predicted, rms {:.3} px, log10 P {:.1}, sip {:?}, {} centres",
        solution.matched,
        solution.predicted,
        solution.rms_px,
        solution.false_alarm_log10,
        solution.sip_order,
        solution.centres_tried
    );
    let mut errors = Vec::new();
    let distorted = corner_px > 0.0;
    if distorted {
        let distortion = radial_cubic(truth, corner_px);
        let (forward, inverse) =
            rendered_distortion_error_px(truth, &distortion, &solution.wcs_cards);
        if !(at_most(forward, DISTORTED_MAX_ERROR_PX) && at_most(inverse, DISTORTED_MAX_ERROR_PX)) {
            errors.push(format!(
                "{label}: forward {forward:.3} px, inverse {inverse:.3} px over the distorted grid"
            ));
        }
    } else {
        let corner = truth.worst_corner_px(&solution.wcs_cards);
        if !at_most(corner, SOLVED_WORST_CORNER_PX) {
            errors.push(format!("{label}: worst corner {corner:.3} px"));
        }
        let scale_ratio = solution.scale_arcsec / truth.scale_arcsec();
        if !at_most((scale_ratio - 1.0).abs(), 1e-3) {
            errors.push(format!("{label}: scale ratio {scale_ratio}"));
        }
        let truth_rotation = truth
            .wcs()
            .orientation(truth.width, truth.height)
            .rotation_deg;
        if !at_most(
            angle_difference_deg(solution.rotation_deg, truth_rotation),
            0.05,
        ) {
            errors.push(format!(
                "{label}: rotation {} vs {truth_rotation}",
                solution.rotation_deg
            ));
        }
    }
    let expected_parity = if flipped {
        ImageParity::Flipped
    } else {
        ImageParity::Normal
    };
    if solution.parity != expected_parity {
        errors.push(format!("{label}: parity {:?}", solution.parity));
    }
    let centre_error = angular_separation(
        solution.centre.0,
        solution.centre.1,
        truth.centre().0,
        truth.centre().1,
    ) * ARCSEC_PER_DEG
        / truth.scale_arcsec();
    if !at_most(centre_error, SOLVED_WORST_CORNER_PX) {
        errors.push(format!("{label}: centre off by {centre_error:.3} px"));
    }
    let sip_as_expected = if distorted {
        matches!(solution.sip_order, Some(3 | 4))
    } else {
        solution.sip_order.is_none()
    };
    let gates = solution.matched >= 8
        && solution.rms_px <= 1.0
        && solution.false_alarm_log10 < -9.0
        && sip_as_expected;
    if !gates {
        errors.push(format!(
            "{label}: matched {}, rms {}, log10 P {}, sip {:?}",
            solution.matched, solution.rms_px, solution.false_alarm_log10, solution.sip_order
        ));
    }
    if solution.predicted < END_TO_END_MIN_PREDICTED {
        errors.push(format!(
            "{label}: {} predicted, at least {END_TO_END_MIN_PREDICTED} expected from the deep verification",
            solution.predicted
        ));
    }
    if solution.catalog_tier != TIER1_LABEL {
        errors.push(format!("{label}: catalogue tier {}", solution.catalog_tier));
    }
    errors
}

#[test]
fn synthetic_frames_from_0_25_to_20_degrees_solve_with_rotation_and_parity() {
    let params = LocalSolveParams::default();
    let mut failures = Vec::new();
    for (index, case) in END_TO_END.iter().enumerate() {
        let seed = 100 + index as u64;
        let truth = TruthField::with_long_side(
            case.centre,
            case.long_side_deg,
            case.rotation_deg,
            case.flipped,
            FRAME.0,
            FRAME.1,
        );
        let field = truth.field();
        let diagonal = field.width_deg.hypot(field.height_deg);
        let provider = SyntheticProvider::uniform(
            TIER1_LABEL,
            truth.centre(),
            1.2 * diagonal,
            truth.density_for(200.0),
            (8.0, 13.0),
            seed,
        );
        let distortion = (case.corner_px > 0.0)
            .then(|| Box::new(radial_cubic(&truth, case.corner_px)) as PixelDistortion);
        let image = render_field(
            &truth,
            provider.rows(),
            13.0,
            &FrameStyle {
                seed,
                distortion,
                ..FrameStyle::default()
            },
        );
        let hints = truth.hints(0.3, 40.0 + 70.0 * index as f64);
        let outcome = solve_local(
            &image,
            &hints,
            &provider,
            &params,
            &never_cancelled,
            &no_stage,
        );
        let label = format!(
            "{} deg at {:?}, rotation {}, flipped {}, corner distortion {} px",
            case.long_side_deg, case.centre, case.rotation_deg, case.flipped, case.corner_px
        );
        failures.extend(end_to_end_errors(
            &label,
            &truth,
            outcome,
            case.flipped,
            case.corner_px,
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_binning_wide_scale_hint_solves_at_both_ends_of_the_range() {
    let hints_low = BINNING_SCALE_ARCSEC * (1.0 - SCALE_HINT_MARGIN);
    let hints_high = 2.0 * BINNING_SCALE_ARCSEC * (1.0 + SCALE_HINT_MARGIN);
    let params = LocalSolveParams {
        near_radius_deg: 0.0,
        ..LocalSolveParams::default()
    };
    let mut failures = Vec::new();
    for (index, binning) in [1.0, 2.0].into_iter().enumerate() {
        let seed = 400 + index as u64;
        let truth = TruthField::new(
            (140.0, -12.0),
            BINNING_SCALE_ARCSEC * binning,
            33.0,
            false,
            BINNING_FRAME.0,
            BINNING_FRAME.1,
        );
        let provider = SyntheticProvider::uniform(
            TIER1_LABEL,
            truth.centre(),
            2.5,
            truth.density_for(150.0),
            (8.0, 14.0),
            seed,
        );
        let detections = detections_from(&truth, provider.rows(), 14.0, 0.05, 20, seed);
        let mut hints = truth.hints(0.0, 0.0);
        hints.scale = Some(ScaleRange {
            low_arcsec: hints_low,
            high_arcsec: hints_high,
            source: "XPIXSZ x 1 to 2 (XBINNING)".to_string(),
        });
        let label = format!(
            "truth {} arcsec/px with hints {hints_low}..{hints_high}",
            truth.scale_arcsec()
        );
        match solve_detections(&truth, &provider, &detections, &hints, &params) {
            Err(failure) => failures.push(format!("{label}: {failure:?}")),
            Ok(solution) => {
                let corner = truth.worst_corner_px(&solution.wcs_cards);
                if !at_most(corner, SOLVED_WORST_CORNER_PX) {
                    failures.push(format!("{label}: worst corner {corner:.3} px"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn scale_hypotheses_cover_the_hint_range_with_ten_percent_windows() {
    let window = (1.0 + SCALE_HINT_MARGIN) / (1.0 - SCALE_HINT_MARGIN);
    let mut failures = Vec::new();
    for (low, high) in [
        (0.9, 1.1),
        (0.92, 1.12),
        (1.35, 3.3),
        (0.9, 4.4),
        (1.0, 5.0),
    ] {
        let hypotheses = scale_hypotheses(low, high);
        let label = format!("{low}..{high}: {hypotheses:?}");
        let covered = |s: f64| {
            hypotheses
                .iter()
                .any(|h| s >= h * (1.0 - SCALE_HINT_MARGIN) && s <= h * (1.0 + SCALE_HINT_MARGIN))
        };
        let samples = (0..=100).map(|i| low * (high / low).powf(i as f64 / 100.0));
        if !samples.into_iter().all(covered) {
            failures.push(format!(
                "{label}: a scale in the range is outside every window"
            ));
        }
        let single = high / low <= window;
        let centred = (low * high / ((1.0 - SCALE_HINT_MARGIN) * (1.0 + SCALE_HINT_MARGIN))).sqrt();
        if single && hypotheses != vec![centred] {
            failures.push(format!(
                "{label}: one window must sit at {centred}, whose window is {}..{}",
                centred * (1.0 - SCALE_HINT_MARGIN),
                centred * (1.0 + SCALE_HINT_MARGIN)
            ));
        }
        let steps = hypotheses.windows(2).map(|w| w[1] / w[0]);
        if steps
            .clone()
            .any(|ratio| ratio > window + 1e-12 || ratio < 1.0)
        {
            failures.push(format!(
                "{label}: hypotheses must rise by at most {window:.4}"
            ));
        }
        let expected = if single {
            1
        } else {
            ((high / low).ln() / window.ln()).ceil() as usize
        };
        if hypotheses.len() != expected {
            failures.push(format!(
                "{label}: {} hypotheses, expected {expected}",
                hypotheses.len()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_search_plan_merges_hypotheses_per_ring_and_caps_the_centre_count() {
    let hint = (200.0, -15.0);
    let one = search_plan(hint, &[1.0 / 3.0], 3.0, 600);
    assert!(!one.capped, "{one:?}");
    assert_eq!(one.centres, 397, "1 deg field at 3 deg: rings 0..=11");
    assert_eq!(one.rings.len(), 12);
    assert_eq!(one.radius_reached_deg, 3.0);
    assert_eq!(
        one.rings
            .iter()
            .flatten()
            .map(|(_, c)| *c)
            .collect::<Vec<_>>(),
        spiral_centres(hint, 1.0 / 3.0, 3.0).concat()
    );
    let small = search_plan(hint, &[0.25 / 3.0], 3.0, 600);
    assert!(small.capped, "{}", small.centres);
    assert!(
        small.centres <= 600 && small.centres > 500,
        "{}",
        small.centres
    );
    let step = 0.25 / 3.0 * 3f64.sqrt() / 2.0;
    let rings = small.rings.len() - 1;
    assert!((small.radius_reached_deg - rings as f64 * step).abs() < 1e-12);
    let two = search_plan(hint, &[1.0 / 3.0, 2.0 / 3.0], 1.0, 10_000);
    assert_eq!(two.rings[0], vec![(0, hint), (1, hint)]);
    let per_hypothesis = |h: usize| two.rings.iter().flatten().filter(|(i, _)| *i == h).count();
    assert_eq!(
        per_hypothesis(0),
        spiral_centres(hint, 1.0 / 3.0, 1.0).concat().len()
    );
    assert_eq!(
        per_hypothesis(1),
        spiral_centres(hint, 2.0 / 3.0, 1.0).concat().len()
    );
    assert_eq!(two.centres, per_hypothesis(0) + per_hypothesis(1));
    assert_eq!(
        search_plan(hint, &[0.5], 0.0, 600).rings,
        vec![vec![(0, hint)]]
    );
}

#[test]
fn the_odds_score_excludes_the_tan_hypothesis_pairs_and_charges_the_trials() {
    let lambda = 0.146;
    assert_eq!(
        false_alarm_log10(8, lambda, 1),
        poisson_tail_log10(8 - TAN_HYPOTHESIS_PAIRS, lambda)
    );
    let charged = false_alarm_log10(11, lambda, 434);
    let single = false_alarm_log10(11, lambda, 1);
    assert!((charged - single - 434f64.log10()).abs() < 1e-12);
    assert!(
        false_alarm_log10(8, lambda, 1) > -9.0,
        "{}",
        false_alarm_log10(8, lambda, 1)
    );
    assert!(single < -9.0, "{single}");
    assert!(charged > -9.0, "{charged}");
    assert!(
        false_alarm_log10(12, lambda, 434) < -9.0,
        "{}",
        false_alarm_log10(12, lambda, 434)
    );
    assert_eq!(false_alarm_log10(3, lambda, 1), 0.0);
    assert_eq!(false_alarm_log10(2, lambda, 1000), 0.0);
    assert_eq!(false_alarm_log10(0, lambda, 1), 0.0);
}

#[test]
fn a_small_field_search_is_capped_at_max_centres_and_says_so() {
    let truth = TruthField::with_long_side(
        (12.5, 33.0),
        SMALL_FIELD_LONG_SIDE_DEG,
        0.0,
        false,
        1500,
        1000,
    );
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        3.5,
        TIER1_CUT_STARS_PER_DEG2,
        (8.0, 16.0),
        61,
    );
    let detections = random_detections(30, truth.width, truth.height, 62);
    let hints = truth.hints(0.0, 0.0);
    let params = LocalSolveParams::default();
    let failure = solve_stars(
        &detections,
        truth.size(),
        &hints,
        &provider,
        &params,
        &never_cancelled,
        &no_stage,
    )
    .expect_err("a random list over a capped spiral returns a failure");
    assert_eq!(failure.reason, FailureReason::NoSolution, "{failure:?}");
    assert!(
        failure.centres_tried <= params.max_centres,
        "{} centres tried, cap {}",
        failure.centres_tried,
        params.max_centres
    );
    assert!(
        failure.message.contains("capped"),
        "the failure must name the cap: {}",
        failure.message
    );
}

#[test]
fn cancel_stops_a_search_over_rendered_centres_within_one_centre() {
    let truth = TruthField::with_long_side((101.3, -16.7), 1.0, 25.0, false, 1500, 1000);
    let hints = truth.hints(2.0, 300.0);
    let hint = hints.centre.as_ref().map(|c| (c.ra, c.dec)).expect("hint");
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        hint,
        4.0,
        truth.density_for(150.0),
        (8.0, 14.0),
        77,
    );
    let detections = detections_from(&truth, provider.rows(), 14.0, 0.05, 30, 78);
    let params = LocalSolveParams::default();
    let checks = AtomicUsize::new(0);
    let first_true: Mutex<Option<Instant>> = Mutex::new(None);
    let cancel = || {
        if checks.fetch_add(1, Ordering::SeqCst) + 1 < CANCEL_AFTER_CHECKS {
            return false;
        }
        first_true
            .lock()
            .expect("cancel clock")
            .get_or_insert_with(Instant::now);
        true
    };
    let failure = solve_stars(
        &detections,
        truth.size(),
        &hints,
        &provider,
        &params,
        &cancel,
        &no_stage,
    )
    .expect_err("a cancelled search returns a failure");
    let returned = Instant::now();
    assert_eq!(failure.reason, FailureReason::Cancelled, "{failure:?}");
    let allowed = CANCEL_AFTER_CHECKS - 1 + rayon::current_num_threads();
    assert!(
        failure.centres_tried <= allowed,
        "{} centres tried after cancel, at most {allowed}",
        failure.centres_tried
    );
    let cancelled_at = first_true
        .into_inner()
        .expect("cancel clock")
        .expect("cancel was consulted");
    let reaction = returned.duration_since(cancelled_at);
    eprintln!(
        "cancel over rendered centres: {} centres tried, returned {reaction:?} after cancel",
        failure.centres_tried
    );
    if !cfg!(debug_assertions) {
        assert!(
            reaction <= Duration::from_secs(1),
            "returned {reaction:?} after cancel"
        );
    }
}

#[test]
fn c4_negatives_return_no_solution_with_the_best_numbers() {
    let truth = TruthField::with_long_side((266.4, -28.9), 1.0, 0.0, false, FRAME.0, FRAME.1);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        2.0,
        truth.density_for(250.0),
        (8.0, 14.0),
        51,
    );
    let mut hints = truth.hints(0.0, 0.0);
    hints.expected_tier0_stars = Some(37.0);
    let params = LocalSolveParams {
        near_radius_deg: 0.5,
        ..LocalSolveParams::default()
    };
    let frames = [
        ("noise", noise_frame(FRAME.0, FRAME.1, 52)),
        ("flat", flat_frame(FRAME.0, FRAME.1)),
        ("gradient", gradient_frame(FRAME.0, FRAME.1, 53)),
        ("hot pixels", hot_pixel_frame(FRAME.0, FRAME.1, 54)),
    ];
    let mut failures = Vec::new();
    for (name, image) in frames {
        match solve_local(
            &image,
            &hints,
            &provider,
            &params,
            &never_cancelled,
            &no_stage,
        ) {
            Ok(solution) => failures.push(format!("{name}: solved {solution:?}")),
            Err(failure) => {
                if failure.reason != FailureReason::NoSolution {
                    failures.push(format!("{name}: reason {:?}", failure.reason));
                }
                if failure.expected_tier0_stars != Some(37.0) {
                    failures.push(format!(
                        "{name}: expected tier-0 stars {:?}",
                        failure.expected_tier0_stars
                    ));
                }
                if let Some(best) = &failure.best {
                    if best.matched >= 8 && best.false_alarm_log10 < -9.0 && best.rms_px <= 1.0 {
                        failures.push(format!("{name}: best candidate passes the gates {best:?}"));
                    }
                }
                if name == "hot pixels" && (failure.detections == 0 || failure.centres_tried == 0) {
                    failures.push(format!(
                        "{name}: {} detections, {} centres tried",
                        failure.detections, failure.centres_tried
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn tier_chain(tier1: bool, tier2: bool) -> TierChain {
    let mut tiers: Vec<(CatalogTier, Box<dyn CatalogProvider>)> = Vec::new();
    if tier1 {
        tiers.push((
            CatalogTier::Tier1,
            Box::new(SyntheticProvider::uniform(
                TIER1_LABEL,
                TIER_CENTRE,
                1.0,
                TIER1_CUT_STARS_PER_DEG2,
                (6.0, 16.0),
                71,
            )),
        ));
    }
    tiers.push((
        CatalogTier::Tier0,
        Box::new(SyntheticProvider::uniform(
            TIER0_LABEL,
            TIER_CENTRE,
            4.0,
            TIER0_MEAN_STARS_PER_DEG2,
            (5.0, 12.0),
            72,
        )),
    ));
    if tier2 {
        tiers.push((
            CatalogTier::Tier2,
            Box::new(SyntheticProvider::uniform(
                TIER2_LABEL,
                TIER_CENTRE,
                1.0,
                2000.0,
                (6.0, 19.0),
                73,
            )),
        ));
    }
    TierChain { tiers }
}

fn three_by_two(long_side_deg: f64) -> FieldSize {
    FieldSize {
        width_deg: long_side_deg,
        height_deg: long_side_deg / 1.5,
    }
}

fn chosen(
    chain: &TierChain,
    long_side_deg: f64,
    expected: Option<f64>,
) -> Option<(CatalogTier, &'static str)> {
    chain
        .choose(three_by_two(long_side_deg), expected)
        .map(|(tier, provider)| {
            let (_, label) = provider
                .cone(TIER_CENTRE.0, TIER_CENTRE.1, 0.1, None)
                .expect("synthetic cone");
            (tier, label)
        })
}

#[test]
fn tier_chain_picks_tier_1_for_a_0_4_degree_field_and_tier_0_for_a_5_degree_field() {
    let chain = tier_chain(true, false);
    assert_eq!(
        chosen(&chain, 0.4, None),
        Some((CatalogTier::Tier1, TIER1_LABEL))
    );
    assert_eq!(
        chosen(&chain, 5.0, None),
        Some((CatalogTier::Tier0, TIER0_LABEL))
    );
}

#[test]
fn tier_chain_follows_the_expected_tier0_count_and_falls_back_in_order() {
    let full = tier_chain(true, true);
    assert_eq!(
        chosen(&full, 5.0, Some(20.0)),
        Some((CatalogTier::Tier1, TIER1_LABEL)),
        "a sparse 5 deg field near a galactic pole"
    );
    assert_eq!(
        chosen(&full, 0.4, Some(80.0)),
        Some((CatalogTier::Tier0, TIER0_LABEL)),
        "a 0.4 deg field on a cluster with 80 tier-0 stars"
    );
    assert_eq!(
        chosen(&full, 1.2, None),
        Some((CatalogTier::Tier0, TIER0_LABEL)),
        "1.2 x 0.8 deg holds about 60 tier-0 stars at the mean density"
    );
    assert_eq!(
        chosen(&full, 1.0, None),
        Some((CatalogTier::Tier1, TIER1_LABEL)),
        "1.0 x 0.67 deg holds about 41 tier-0 stars at the mean density"
    );
    assert_eq!(
        chosen(&tier_chain(false, true), 0.4, None),
        Some((CatalogTier::Tier2, TIER2_LABEL))
    );
    assert_eq!(
        chosen(&tier_chain(false, false), 0.4, None),
        Some((CatalogTier::Tier0, TIER0_LABEL))
    );
    assert!(TierChain::default()
        .choose(three_by_two(1.0), None)
        .is_none());
}

fn nearest_px(points: &[(f64, f64)], p: (f64, f64)) -> f64 {
    points
        .iter()
        .map(|&q| pixel_distance(p, q))
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn synthetic_fixtures_are_self_consistent() {
    let mut failures = Vec::new();
    for flipped in [false, true] {
        let truth = TruthField::with_long_side((359.95, 85.0), 5.0, 211.0, flipped, 1536, 1024);
        let mut worst: f64 = 0.0;
        for (x, y) in grid(truth.width, truth.height, 9) {
            let (ra, dec) = truth.to_sky(x, y);
            let (bx, by) = truth.to_pixel(ra, dec);
            worst = worse(worst, (bx - x).hypot(by - y));
        }
        if !at_most(worst, 1e-6) {
            failures.push(format!("flipped {flipped}: round trip {worst} px"));
        }
        let orientation = truth.wcs().orientation(truth.width, truth.height);
        if orientation.flipped != flipped {
            failures.push(format!(
                "flipped {flipped}: orientation says {}",
                orientation.flipped
            ));
        }
        if !at_most((truth.long_side_deg() - 5.0).abs(), 1e-9) {
            failures.push(format!("long side {}", truth.long_side_deg()));
        }
    }

    let truth = TruthField::with_long_side((83.82, -5.39), 1.0, 37.0, false, FRAME.0, FRAME.1);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        1.2,
        truth.density_for(200.0),
        (8.0, 13.0),
        5,
    );
    let image = render_field(&truth, provider.rows(), 13.0, &FrameStyle::default());
    let detected: Vec<(f64, f64)> =
        crate::core::analysis::star_detection::detect_stars(&image, 5.0)
            .stars
            .iter()
            .map(|s| (s.x, s.y))
            .collect();
    let bright: Vec<(f64, f64)> = provider
        .rows()
        .iter()
        .filter(|row| row.g.is_some_and(|g| g <= 12.5))
        .map(|row| truth.to_pixel(row.ra, row.dec))
        .filter(|&(x, y)| {
            x > 5.0 && y > 5.0 && x < FRAME.0 as f64 - 6.0 && y < FRAME.1 as f64 - 6.0
        })
        .collect();
    let found = bright
        .iter()
        .filter(|&&p| nearest_px(&detected, p) <= 0.5)
        .count();
    if bright.len() < 100 || (found as f64) < 0.9 * bright.len() as f64 {
        failures.push(format!(
            "rendered frame: {found} of {} catalogue stars brighter than 12.5 detected within 0.5 px ({} detections)",
            bright.len(),
            detected.len()
        ));
    }

    let case = odds_case(200, 200, 8, 0.0, 1.0, 3);
    let odds_truth = TruthField::from_tan(case.tan, ODDS_SIZE, ODDS_SIZE);
    let projected: Vec<(f64, f64)> = case
        .catalogue
        .iter()
        .map(|&(ra, dec)| odds_truth.to_pixel(ra, dec))
        .collect();
    let matches = projected
        .iter()
        .filter(|&&p| {
            let detections: Vec<(f64, f64)> = case
                .detections
                .iter()
                .map(|d| (d.star.x, d.star.y))
                .collect();
            nearest_px(&detections, p) <= ODDS_RADIUS_PX
        })
        .count();
    if matches != 8 {
        failures.push(format!(
            "odds case: {matches} catalogue stars matched, expected 8"
        ));
    }

    let expected = (1.0 - (-0.5f64).exp()).log10();
    if !at_most((reference_tail_log10(1, 0.5) - expected).abs(), 1e-12) {
        failures.push(format!(
            "reference tail {} vs {expected}",
            reference_tail_log10(1, 0.5)
        ));
    }

    let sip_cases: [(f64, f64, f64, u8, u8); 5] = [
        (2.0, 4.0, 0.0, 3, 3),
        (3.0, 4.0, 2.0, 3, 3),
        (10.0, 4.0, 2.0, 4, 4),
        (10.0, 4.0, 0.0, 4, 3),
        (2.0, 0.0, 0.0, 3, 1),
    ];
    for (index, &(long_side, cubic_px, quartic_px, max_order, expected_order)) in
        sip_cases.iter().enumerate()
    {
        let sip_truth = TruthField::with_long_side(
            (120.0 + 30.0 * index as f64, 15.0),
            long_side,
            12.0,
            false,
            1500,
            1000,
        );
        let distortion = with_quartic(&sip_truth, cubic_px, quartic_px);
        let pairs: Vec<(f64, f64, f64, f64)> =
            distorted_pairs(&sip_truth, &distortion, SIP_PAIRS, 60 + index as u64)
                .iter()
                .map(|p| {
                    let (tx, ty) = sip_truth.to_pixel(p.ra, p.dec);
                    (p.x, p.y, tx, ty)
                })
                .collect();
        let order = crate::core::registration::poly::fit_select(&pairs, 1..=max_order, 5)
            .map(|selection| selection.model.order);
        if order != Some(expected_order) {
            failures.push(format!(
                "poly::fit_select 1..={max_order} on SIP case {index}: {order:?}, expected {expected_order}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
