use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

use super::synthetic::{
    at_most, coloured_rows, destination, detection, detections_from, exact_pairs, flat_frame,
    flux_for, gradient_frame, hot_pixel_frame, image_centre_crpix, noise_frame, random_detections,
    random_in_cap, render_field, wcs_from_cards, ColourLocus, FrameStyle, PixelDistortion,
    SyntheticProvider, TruthField, ARCSEC_PER_DEG, TIER1_CUT_STARS_PER_DEG2,
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
const CLIPPED_FIELD_SCALE_ARCSEC: f64 = 6.0;
const CLIPPED_FIELD_FLUX_AT_DEPTH: f64 = 5_000.0;
const CLIPPED_FIELD_BACKGROUNDS: [f64; 2] = [1_000.0, 3_000.0];
const CLIPPED_FIELD_MIN_PIXELS: usize = 40;
const CLIP_ADU: f32 = 65_000.0;
const BLEED_HALF_LENGTH_PX: usize = 12;
const OFF_FRAME_GROUP_STARS: usize = 80;
const OFF_FRAME_GROUP_MAGS: (f64, f64) = (5.0, 7.0);
const OFF_FRAME_GROUP_SHORT_SIDES: (f64, f64) = (0.85, 1.05);
const CHANCE_PAIR_FWHM_PX: f64 = 2.0;
const CHANCE_PAIR_TRUE: usize = 55;
const CHANCE_PAIR_CHANCE: usize = 15;
const CHANCE_PAIR_SCATTER_PX: f64 = 0.3;
const CHANCE_PAIR_OFFSETS_PX: (f64, f64) = (2.0, 2.8);
const BEYOND_RADIUS_CASES: [(f64, f64, u8); 2] = [(12.0, 60.0, 4), (6.0, 40.0, 3)];
const BEYOND_RADIUS_STARS_IN_FRAME: f64 = 500.0;
const INVERSE_CUBIC_NEWTON_STEPS: usize = 30;
const ELONGATED_SMEAR_HALF_WIDTH: usize = 2;
const COARSE_TAN_STARS: usize = 200;
const COARSE_TAN_JITTER_PX: f64 = 0.1;
const COARSE_TAN_SHIFT_PX: f64 = 1.5;
const COARSE_TAN_ROTATION_DEG: f64 = 0.1;
const COLOUR_FIELD_STARS_PER_DEG2: f64 = 1600.0;
const COLOUR_FIELD_OPTICAL_MAGS: (f64, f64) = (7.0, 14.0);
const COLOUR_LOCUS: ColourLocus = ColourLocus {
    intrinsic_colour: (0.1, 0.9),
    intrinsic_slope: 2.2,
    intrinsic_offset: 0.0,
    reddening: (0.0, 0.7),
    reddening_slope: 2.7,
    scatter: 0.15,
};
const COLOUR_FIELD_BLOATED_BRIGHTER_THAN: f64 = 11.5;
const COLOUR_FIELD_DEPTH_R: f64 = 14.0;
const TESS_LIKE_FRAME: (usize, usize) = (2136, 2078);
const TESS_LIKE_SCALE_ARCSEC: f64 = 20.5;
const TESS_LIKE_STARS_IN_FRAME: f64 = 3000.0;
const TESS_LIKE_DISTORTION_PX: (f64, f64, f64) = (35.0, 30.0, 8.0);
const GALAXY_SIGMA_PX: f64 = 400.0;
const GALAXY_PEAK_ADU: f64 = 3000.0;
const GALAXY_NUCLEUS_SIGMA_PX: f64 = 8.0;
const GALAXY_NUCLEUS_PEAK_ADU: f64 = 60_000.0;
const GALAXY_MIN_RECOVERED_FRACTION: f64 = 0.5;
const PARTIAL_FIELD_STARS_IN_FRAME: f64 = 500.0;
const PARTIAL_FIELD_DISC_FRACTION: f64 = 0.35;
const PARTIAL_FIELD_SHIFT_PX: f64 = 40.0;
const EXTINCTION_PATCH_SURVIVING_FRACTION: f64 = 0.15;
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
    best.matched < 8 || best.false_alarm_log10 >= -9.0 || best.rms_px > 1.0 || !best.covered()
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
        near_radius_fields: 0.0,
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
        near_radius_fields: 0.0,
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
fn poisson_lower_tail_log10_matches_a_direct_sum() {
    let cases = [(0, 11.0), (1, 11.0), (2, 26.0), (5, 5.0), (30, 20.0), (0, 0.5)];
    let mut failures = Vec::new();
    for (k, lambda) in cases {
        let got = poisson_lower_tail_log10(k, lambda);
        let direct: f64 = (0..=k)
            .map(|i| {
                let ln_factorial: f64 = (1..=i).map(|j| (j as f64).ln()).sum();
                (-lambda + i as f64 * lambda.ln() - ln_factorial).exp()
            })
            .sum();
        let expected = direct.log10();
        if !at_most((got - expected).abs(), 1e-9 * expected.abs().max(1.0)) {
            failures.push(format!("k {k}, lambda {lambda}: {got} vs {expected}"));
        }
    }
    if poisson_lower_tail_log10(3, 0.0) != 0.0 {
        failures.push("lambda 0 must give log10 P = 0".to_string());
    }
    if poisson_lower_tail_log10(500, 3.0) != 0.0 {
        failures.push("a count far above lambda must give log10 P = 0".to_string());
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

fn rendered_by_inverse_radial_cubic(
    truth: &TruthField,
    corner_px: f64,
) -> impl Fn(f64, f64) -> (f64, f64) {
    let (xc, yc) = (
        (truth.width as f64 - 1.0) / 2.0,
        (truth.height as f64 - 1.0) / 2.0,
    );
    let half_diagonal = (truth.width as f64 / 2.0).hypot(truth.height as f64 / 2.0);
    let k = corner_px / half_diagonal;
    move |x, y| {
        let (u, v) = ((x - xc) / half_diagonal, (y - yc) / half_diagonal);
        let tau = u.hypot(v);
        if tau == 0.0 {
            return (x, y);
        }
        let mut rho = tau;
        for _ in 0..INVERSE_CUBIC_NEWTON_STEPS {
            rho -= (rho + k * rho * rho * rho - tau) / (1.0 + 3.0 * k * rho * rho);
        }
        let shrink = rho / tau;
        (
            xc + u * shrink * half_diagonal,
            yc + v * shrink * half_diagonal,
        )
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
        near_radius_fields: 0.0,
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
fn a_wide_field_with_clipped_bright_stars_solves_on_a_dark_and_a_bright_sky() {
    let truth = TruthField::new(
        (90.0, -30.0),
        CLIPPED_FIELD_SCALE_ARCSEC,
        30.0,
        false,
        FRAME.0,
        FRAME.1,
    );
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        5.0,
        TIER1_CUT_STARS_PER_DEG2,
        (6.0, 15.5),
        500,
    );
    let hints = truth.hints(0.3, 60.0);
    let params = LocalSolveParams::default();
    let mut failures = Vec::new();
    for background in CLIPPED_FIELD_BACKGROUNDS {
        let mut image = render_field(
            &truth,
            provider.rows(),
            15.5,
            &FrameStyle {
                background,
                noise_sigma: 10.0,
                flux_at_depth: CLIPPED_FIELD_FLUX_AT_DEPTH,
                seed: 501,
                ..FrameStyle::default()
            },
        );
        image.mapv_inplace(|value| value.min(CLIP_ADU));
        let clipped = image.iter().filter(|&&value| value >= CLIP_ADU).count();
        let flagged = detect_for_solve(&image, &hints, &params)
            .iter()
            .filter(|d| d.saturated)
            .count();
        let label = format!(
            "{:.2} deg, background {background} ADU, {clipped} pixels clipped at {CLIP_ADU}, {flagged} detections flagged saturated",
            truth.long_side_deg()
        );
        assert!(
            clipped >= CLIPPED_FIELD_MIN_PIXELS,
            "{label}: the fixture must clip its bright stars"
        );
        let outcome = solve_local(
            &image,
            &hints,
            &provider,
            &params,
            &never_cancelled,
            &no_stage,
        );
        match outcome {
            Err(failure) => failures.push(format!("{label}: {failure:?}")),
            Ok(solution) => {
                let corner = truth.worst_corner_px(&solution.wcs_cards);
                eprintln!(
                    "{label}: {} matched of {} predicted, rms {:.3} px, worst corner {corner:.3} px, {} centres",
                    solution.matched, solution.predicted, solution.rms_px, solution.centres_tried
                );
                if !at_most(corner, SOLVED_WORST_CORNER_PX) {
                    failures.push(format!("{label}: worst corner {corner:.3} px"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn bleed_clipped_columns(image: &mut Array2<f32>) {
    let (rows, _) = image.dim();
    let clipped: Vec<(usize, usize)> = image
        .indexed_iter()
        .filter(|(_, &value)| value >= CLIP_ADU)
        .map(|((row, col), _)| (row, col))
        .collect();
    for (row, col) in clipped {
        let top = row.saturating_sub(BLEED_HALF_LENGTH_PX);
        let bottom = (row + BLEED_HALF_LENGTH_PX).min(rows - 1);
        for bleed_row in top..=bottom {
            image[[bleed_row, col]] = CLIP_ADU;
        }
    }
}

#[test]
fn saturated_stars_with_bleed_columns_are_counted_before_the_eccentricity_filter() {
    let truth = TruthField::new(
        (90.0, -30.0),
        CLIPPED_FIELD_SCALE_ARCSEC,
        30.0,
        false,
        FRAME.0,
        FRAME.1,
    );
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        5.0,
        TIER1_CUT_STARS_PER_DEG2,
        (6.0, 15.5),
        500,
    );
    let hints = truth.hints(0.3, 60.0);
    let params = LocalSolveParams::default();
    let mut image = render_field(
        &truth,
        provider.rows(),
        15.5,
        &FrameStyle {
            background: CLIPPED_FIELD_BACKGROUNDS[0],
            noise_sigma: 10.0,
            flux_at_depth: CLIPPED_FIELD_FLUX_AT_DEPTH,
            seed: 501,
            ..FrameStyle::default()
        },
    );
    image.mapv_inplace(|value| value.min(CLIP_ADU));
    bleed_clipped_columns(&mut image);
    let detections = detect_for_solve(&image, &hints, &params);
    let flagged = detections.iter().filter(|d| d.saturated).count();
    let bleeding = detections
        .iter()
        .filter(|d| d.saturated && d.star.eccentricity > params.max_eccentricity)
        .count();
    let label = format!(
        "{:.2} deg with bleed columns: {flagged} detections flagged saturated, {bleeding} of them eccentric",
        truth.long_side_deg()
    );
    let mut failures = Vec::new();
    if bleeding == 0 {
        failures.push(format!(
            "{label}: bleeding saturated stars must be flagged and counted before the eccentricity filter drops them"
        ));
    }
    match solve_local(
        &image,
        &hints,
        &provider,
        &params,
        &never_cancelled,
        &no_stage,
    ) {
        Err(failure) => failures.push(format!("{label}: {failure:?}")),
        Ok(solution) => {
            let corner = truth.worst_corner_px(&solution.wcs_cards);
            eprintln!(
                "{label}: {} matched of {} predicted, rms {:.3} px, worst corner {corner:.3} px, {} centres",
                solution.matched, solution.predicted, solution.rms_px, solution.centres_tried
            );
            if !at_most(corner, SOLVED_WORST_CORNER_PX) {
                failures.push(format!("{label}: worst corner {corner:.3} px"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_triangle_stars_come_from_inside_the_frame_not_from_the_cone_around_it() {
    let truth = TruthField::with_long_side((200.0, -35.0), 5.0, 20.0, false, FRAME.0, FRAME.1);
    let field = truth.field();
    let diagonal = field.width_deg.hypot(field.height_deg);
    let uniform = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        1.2 * diagonal,
        truth.density_for(200.0),
        (8.0, 13.0),
        610,
    );
    let mut rows = uniform.rows().to_vec();
    let mut rng = StdRng::seed_from_u64(611);
    let (xc, yc) = (
        (truth.width as f64 - 1.0) / 2.0,
        (truth.height as f64 - 1.0) / 2.0,
    );
    let short_side = truth.height.min(truth.width) as f64;
    for index in 0..OFF_FRAME_GROUP_STARS {
        let above = OFF_FRAME_GROUP_SHORT_SIDES.0
            + (OFF_FRAME_GROUP_SHORT_SIDES.1 - OFF_FRAME_GROUP_SHORT_SIDES.0) * rng.gen::<f64>();
        let x = xc + 0.3 * truth.width as f64 * (rng.gen::<f64>() - 0.5);
        let y = yc - above * short_side;
        let (ra, dec) = truth.to_sky(x, y);
        let mag = OFF_FRAME_GROUP_MAGS.0
            + (OFF_FRAME_GROUP_MAGS.1 - OFF_FRAME_GROUP_MAGS.0) * rng.gen::<f64>();
        rows.push(super::synthetic::catalog_row(
            100_000 + index,
            ra,
            dec,
            Some(mag),
        ));
    }
    let provider = SyntheticProvider::new(TIER1_LABEL, rows, 13.0);
    let image = render_field(
        &truth,
        provider.rows(),
        13.0,
        &FrameStyle {
            seed: 612,
            ..FrameStyle::default()
        },
    );
    let hints = truth.hints(0.0, 0.0);
    let params = LocalSolveParams {
        near_radius_deg: 0.0,
        near_radius_fields: 0.0,
        ..LocalSolveParams::default()
    };
    let label = format!(
        "{} deg field with {OFF_FRAME_GROUP_STARS} bright stars above the frame, inside the old cone",
        truth.long_side_deg()
    );
    let outcome = solve_local(
        &image,
        &hints,
        &provider,
        &params,
        &never_cancelled,
        &no_stage,
    );
    let failures = end_to_end_errors(&label, &truth, outcome, false, 0.0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_rms_gate_ignores_chance_pairs_inside_the_match_radius() {
    let params = LocalSolveParams::default();
    let truth = TruthField::new((150.0, 2.2), 2.0, 31.0, false, ODDS_SIZE, ODDS_SIZE);
    let total = CHANCE_PAIR_TRUE + CHANCE_PAIR_CHANCE;
    let radius = params
        .match_radius_px
        .max(params.match_radius_fwhm * CHANCE_PAIR_FWHM_PX);
    let exclusion = 3.0 * radius;
    let mut rng = StdRng::seed_from_u64(620);
    let margin = 5.0;
    let span = ODDS_SIZE as f64 - 1.0 - 2.0 * margin;
    let mut catalogue_px: Vec<(f64, f64)> = Vec::with_capacity(total);
    while catalogue_px.len() < total {
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
    let detections: Vec<Detection> = catalogue_px
        .iter()
        .enumerate()
        .map(|(index, &(x, y))| {
            let (dx, dy) = if index < CHANCE_PAIR_TRUE {
                (
                    CHANCE_PAIR_SCATTER_PX * super::synthetic::gaussian(&mut rng),
                    CHANCE_PAIR_SCATTER_PX * super::synthetic::gaussian(&mut rng),
                )
            } else {
                let distance = CHANCE_PAIR_OFFSETS_PX.0
                    + (CHANCE_PAIR_OFFSETS_PX.1 - CHANCE_PAIR_OFFSETS_PX.0) * rng.gen::<f64>();
                let angle = rng.gen::<f64>() * std::f64::consts::TAU;
                (distance * angle.cos(), distance * angle.sin())
            };
            detection(x + dx, y + dy, 1000.0, CHANCE_PAIR_FWHM_PX)
        })
        .collect();
    let catalogue: Vec<(f64, f64)> = catalogue_px
        .iter()
        .map(|&(x, y)| truth.to_sky(x, y))
        .collect();
    let v = verify(
        &truth.tan,
        &detections,
        &catalogue,
        (ODDS_SIZE, ODDS_SIZE),
        Some((1.8, 2.2)),
        &params,
    );
    eprintln!(
        "{CHANCE_PAIR_TRUE} pairs at {CHANCE_PAIR_SCATTER_PX} px and {CHANCE_PAIR_CHANCE} chance pairs at {:?} px inside r = {radius} px: {v:?}",
        CHANCE_PAIR_OFFSETS_PX
    );
    assert_eq!(v.matched, total, "{v:?}");
    assert!((v.radius_px - radius).abs() < 1e-12, "{v:?}");
    assert!(v.false_alarm_log10 < -9.0, "{v:?}");
    assert!(
        at_most(v.rms_px, 2.0 * CHANCE_PAIR_SCATTER_PX),
        "rms must describe the matched core, not the chance pairs: {v:?}"
    );
    assert!(v.accepted, "{v:?}");
}

#[test]
fn distortion_beyond_the_match_radius_is_recovered_by_matching_through_the_provisional_sip() {
    let params = LocalSolveParams::default();
    let mut failures = Vec::new();
    for (index, &(long_side, corner_px, max_order)) in BEYOND_RADIUS_CASES.iter().enumerate() {
        let seed = 630 + index as u64;
        let truth = TruthField::with_long_side(
            (40.0 + 100.0 * index as f64, -20.0 + 50.0 * index as f64),
            long_side,
            15.0 + 70.0 * index as f64,
            index == 1,
            FRAME.0,
            FRAME.1,
        );
        let field = truth.field();
        let diagonal = field.width_deg.hypot(field.height_deg);
        let provider = SyntheticProvider::uniform(
            TIER1_LABEL,
            truth.centre(),
            1.2 * diagonal,
            truth.density_for(BEYOND_RADIUS_STARS_IN_FRAME),
            (8.0, 13.0),
            seed,
        );
        let distortion = rendered_by_inverse_radial_cubic(&truth, corner_px);
        let image = render_field(
            &truth,
            provider.rows(),
            13.0,
            &FrameStyle {
                seed,
                distortion: Some(Box::new(rendered_by_inverse_radial_cubic(
                    &truth, corner_px,
                ))),
                ..FrameStyle::default()
            },
        );
        let hints = truth.hints(0.2, 30.0 + 90.0 * index as f64);
        let label = format!(
            "{long_side} deg, {corner_px} px corner distortion beyond the match radius, order up to {max_order}"
        );
        let solution = match solve_local(
            &image,
            &hints,
            &provider,
            &params,
            &never_cancelled,
            &no_stage,
        ) {
            Ok(solution) => solution,
            Err(failure) => {
                failures.push(format!("{label}: {failure:?}"));
                continue;
            }
        };
        let (forward, inverse) =
            rendered_distortion_error_px(&truth, &distortion, &solution.wcs_cards);
        eprintln!(
            "{label}: {} matched of {} predicted, rms {:.3} px, log10 P {:.1}, sip {:?}, forward {forward:.3} px, inverse {inverse:.3} px, {} centres",
            solution.matched,
            solution.predicted,
            solution.rms_px,
            solution.false_alarm_log10,
            solution.sip_order,
            solution.centres_tried
        );
        if !(at_most(forward, DISTORTED_MAX_ERROR_PX) && at_most(inverse, DISTORTED_MAX_ERROR_PX)) {
            failures.push(format!(
                "{label}: forward {forward:.3} px, inverse {inverse:.3} px over the distorted grid"
            ));
        }
        if !matches!(solution.sip_order, Some(order) if (2..=max_order).contains(&order)) {
            failures.push(format!("{label}: sip order {:?}", solution.sip_order));
        }
        let expected_parity = if index == 1 {
            ImageParity::Flipped
        } else {
            ImageParity::Normal
        };
        if solution.parity != expected_parity {
            failures.push(format!("{label}: parity {:?}", solution.parity));
        }
        if !(solution.matched >= 8 && solution.rms_px <= 1.0 && solution.false_alarm_log10 < -9.0) {
            failures.push(format!(
                "{label}: matched {}, rms {}, log10 P {}",
                solution.matched, solution.rms_px, solution.false_alarm_log10
            ));
        }
        if solution.matched * 2 < solution.predicted {
            failures.push(format!(
                "{label}: only {} of {} predicted stars matched, the corners stayed outside the match radius",
                solution.matched, solution.predicted
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn smear_horizontally(image: &mut Array2<f32>, half_width: usize) {
    let (rows, cols) = image.dim();
    let window = (2 * half_width + 1) as f32;
    for row in 0..rows {
        let line: Vec<f32> = (0..cols).map(|col| image[[row, col]]).collect();
        for col in 0..cols {
            let lo = col.saturating_sub(half_width);
            let hi = (col + half_width).min(cols - 1);
            let sum: f32 = line[lo..=hi].iter().sum();
            image[[row, col]] = sum / window;
        }
    }
}

#[test]
fn an_elongated_psf_keeps_its_stars_for_the_triangle_stage() {
    let truth = TruthField::with_long_side((200.0, -35.0), 5.0, 20.0, false, FRAME.0, FRAME.1);
    let field = truth.field();
    let diagonal = field.width_deg.hypot(field.height_deg);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        1.2 * diagonal,
        truth.density_for(200.0),
        (8.0, 13.0),
        640,
    );
    let mut image = render_field(
        &truth,
        provider.rows(),
        13.0,
        &FrameStyle {
            seed: 641,
            ..FrameStyle::default()
        },
    );
    smear_horizontally(&mut image, ELONGATED_SMEAR_HALF_WIDTH);
    let hints = truth.hints(0.1, 45.0);
    let params = LocalSolveParams::default();
    let detections = detect_for_solve(&image, &hints, &params);
    let elongated = detections
        .iter()
        .filter(|d| d.star.eccentricity > params.max_eccentricity)
        .count();
    let label = format!(
        "{} deg field smeared over {} px: {elongated} of {} detections above eccentricity {}",
        truth.long_side_deg(),
        2 * ELONGATED_SMEAR_HALF_WIDTH + 1,
        detections.len(),
        params.max_eccentricity
    );
    eprintln!("{label}");
    assert!(
        elongated * 2 > detections.len(),
        "the smear must elongate most stars past the fixed cut: {label}"
    );
    let outcome = solve_local(
        &image,
        &hints,
        &provider,
        &params,
        &never_cancelled,
        &no_stage,
    );
    let failures = end_to_end_errors(&label, &truth, outcome, false, 0.0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_coarse_tan_above_the_rms_gate_is_refined_before_the_gate_applies() {
    let params = LocalSolveParams::default();
    let truth = TruthField::new((150.0, 2.2), 2.0, 31.0, false, FRAME.0, FRAME.1);
    let pairs = exact_pairs(&truth, COARSE_TAN_STARS, 650);
    let mut rng = StdRng::seed_from_u64(651);
    let detections: Vec<Detection> = pairs
        .iter()
        .map(|p| {
            detection(
                p.x + COARSE_TAN_JITTER_PX * super::synthetic::gaussian(&mut rng),
                p.y + COARSE_TAN_JITTER_PX * super::synthetic::gaussian(&mut rng),
                1000.0,
                3.0,
            )
        })
        .collect();
    let catalogue: Vec<(f64, f64)> = pairs.iter().map(|p| (p.ra, p.dec)).collect();
    let rows: Vec<CatalogRow> = pairs
        .iter()
        .enumerate()
        .map(|(i, p)| super::synthetic::catalog_row(i, p.ra, p.dec, Some(10.0)))
        .collect();
    let provider = SyntheticProvider::new(TIER1_LABEL, rows, 13.0);
    let shifted = destination(truth.centre(), 90.0, COARSE_TAN_SHIFT_PX * 2.0 / ARCSEC_PER_DEG);
    let coarse = TruthField::new(
        shifted,
        2.0,
        31.0 + COARSE_TAN_ROTATION_DEG,
        false,
        FRAME.0,
        FRAME.1,
    )
    .tan;
    let before = verify(
        &coarse,
        &detections,
        &catalogue,
        truth.size(),
        Some((1.8, 2.2)),
        &params,
    );
    eprintln!("coarse TAN on the footprint set: {before:?}");
    assert!(
        before.matched >= params.min_matches
            && before.false_alarm_log10 < params.max_false_alarm.log10()
            && before.rms_px > params.max_rms_px
            && !before.accepted,
        "the coarse TAN must be significant but above the RMS gate: {before:?}"
    );
    let ctx = SearchContext {
        detections: detections.clone(),
        sip_detections: detections.clone(),
        saturated: 0,
        size: truth.size(),
        catalog: &provider,
        ranking: RankingKey::MAGNITUDE,
        params: &params,
        epoch_year: None,
        scale_range: (1.8, 2.2),
        trials: 1,
    };
    let gate = StageGate {
        max_rms_px: params.max_rms_px,
        trials: 1,
    };
    let (refined, after, kept) = refine_on(coarse, &catalogue, &ctx, gate);
    let corner = truth.worst_corner_px(&tan_cards(&refined));
    eprintln!(
        "refined: {after:?}, {} pairs kept, worst corner {corner:.3} px",
        kept.len()
    );
    assert!(after.accepted, "the refit must pass the gate: {after:?}");
    assert!(
        at_most(after.rms_px, 3.0 * COARSE_TAN_JITTER_PX) && after.matched >= COARSE_TAN_STARS * 9 / 10,
        "{after:?}"
    );
    assert!(at_most(corner, 0.2), "worst corner {corner:.3} px");
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
                    if !refused_for_a_named_reason(best) {
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

#[test]
fn a_solution_that_matches_only_the_inner_third_of_the_frame_is_refused() {
    let params = LocalSolveParams::default();
    let truth = TruthField::with_long_side((140.0, -12.0), 2.0, 20.0, false, FRAME.0, FRAME.1);
    let field = truth.field();
    let diagonal = field.width_deg.hypot(field.height_deg);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        1.2 * diagonal,
        truth.density_for(PARTIAL_FIELD_STARS_IN_FRAME),
        (8.0, 13.0),
        700,
    );
    let whole = detections_from(&truth, provider.rows(), 13.0, 0.05, 20, 701);
    let hints = truth.hints(0.05, 10.0);
    let control = solve_detections(&truth, &provider, &whole, &hints, &params)
        .unwrap_or_else(|failure| panic!("the undisturbed frame must solve: {failure:?}"));
    let control_corner = truth.worst_corner_px(&control.wcs_cards);
    assert!(
        at_most(control_corner, SOLVED_WORST_CORNER_PX),
        "control worst corner {control_corner:.3} px"
    );
    let (xc, yc) = (
        (truth.width as f64 - 1.0) / 2.0,
        (truth.height as f64 - 1.0) / 2.0,
    );
    let disc = PARTIAL_FIELD_DISC_FRACTION * truth.width.min(truth.height) as f64;
    let partial: Vec<Detection> = whole
        .into_iter()
        .map(|mut d| {
            if (d.star.x - xc).hypot(d.star.y - yc) > disc {
                d.star.x += PARTIAL_FIELD_SHIFT_PX;
                d.star.y += PARTIAL_FIELD_SHIFT_PX;
            }
            d
        })
        .filter(|d| truth.inside(d.star.x, d.star.y))
        .collect();
    match solve_detections(&truth, &provider, &partial, &hints, &params) {
        Ok(solution) => panic!(
            "a solution right only inside {disc:.0} px of the centre was accepted: {} matched of {} predicted, log10 P {:.1}, rms {:.3} px, sip {:?}, worst corner {:.1} px",
            solution.matched,
            solution.predicted,
            solution.false_alarm_log10,
            solution.rms_px,
            solution.sip_order,
            truth.worst_corner_px(&solution.wcs_cards)
        ),
        Err(failure) => {
            assert_eq!(failure.reason, FailureReason::NoSolution, "{failure:?}");
            let best = failure
                .best
                .as_deref()
                .unwrap_or_else(|| panic!("the refused candidate keeps its numbers: {failure:?}"));
            assert!(
                best.matched >= params.min_matches
                    && best.false_alarm_log10 < params.max_false_alarm.log10()
                    && best.rms_px <= params.max_rms_px,
                "the partial solution must be refused for its coverage, not for its odds: {best:?}"
            );
            assert!(
                !best.covered(),
                "the refusal must name the empty cells: {best:?}"
            );
            assert!(
                failure.message.contains("coverage"),
                "the message must say why: {}",
                failure.message
            );
        }
    }
}

type CellCounts = (usize, usize, usize);

fn coverage_of_cells(cells: &[CellCounts; 9], radius: f64) -> (f64, f64) {
    let side = 300.0;
    let size = (3 * side as usize, 3 * side as usize);
    let mut positions: Vec<(f64, f64)> = Vec::new();
    let mut detections: Vec<Detection> = Vec::new();
    let mut matched: Vec<(usize, usize, f64)> = Vec::new();
    for (cell, &(predicted, detected, found)) in cells.iter().enumerate() {
        let origin = ((cell % 3) as f64 * side, (cell / 3) as f64 * side);
        let columns = (predicted.max(detected) as f64).sqrt().ceil().max(1.0) as usize;
        let step = side / (columns as f64 + 1.0);
        let point = |i: usize, shift: f64| {
            (
                origin.0 + step * ((i % columns) as f64 + 1.0) + shift,
                origin.1 + step * ((i / columns) as f64 + 1.0) + shift,
            )
        };
        let first = positions.len();
        positions.extend((0..predicted).map(|i| point(i, 0.0)));
        for j in 0..detected {
            let (x, y) = if j < found {
                positions[first + j]
            } else {
                point(j, step / 2.0)
            };
            if j < found {
                matched.push((first + j, detections.len(), 0.0));
            }
            detections.push(detection(x, y, 1000.0, 3.0));
        }
    }
    coverage_log10(&positions, &detections, &matched, size, radius)
}

#[test]
fn a_cell_is_uncovered_only_when_its_matches_are_both_deficient_and_at_chance_level() {
    let clear = (100, 33, 30);
    let dusty = coverage_of_cells(
        &[clear, clear, clear, clear, (100, 33, 6), clear, clear, clear, clear],
        1.5,
    );
    let dense = (300, 100, 60);
    let at_chance = coverage_of_cells(
        &[dense, dense, dense, dense, dense, dense, dense, dense, (300, 100, 13)],
        2.5,
    );
    let thin = (50, 30, 28);
    let empty = coverage_of_cells(&[(50, 30, 0), thin, thin, thin, thin, thin, thin, thin, thin], 1.5);
    let sparse = coverage_of_cells(&[thin, thin, (3, 2, 1), thin, thin, thin, thin, thin, thin], 1.5);
    let uniform = coverage_of_cells(&[clear; 9], 1.5);
    eprintln!(
        "(cell coverage, matches above chance) log10 P: dusty {dusty:?}, at chance {at_chance:?}, empty {empty:?}, sparse {sparse:?}, uniform {uniform:?}"
    );
    let mut failures = Vec::new();
    if !(dusty.0 < COVERAGE_MIN_LOG10 && dusty.1 < COVERAGE_MIN_LOG10 && cell_covered(dusty.0, dusty.1)) {
        failures.push(format!("a cell with 6 matches of 27 expected over a chance level of 0.26 must stay covered: {dusty:?}"));
    }
    if !(at_chance.0 < COVERAGE_MIN_LOG10 && at_chance.1 >= COVERAGE_MIN_LOG10 && !cell_covered(at_chance.0, at_chance.1)) {
        failures.push(format!("a cell with 13 matches of 55 expected over a chance level of 6.5 must be uncovered: {at_chance:?}"));
    }
    if !(empty.0 < COVERAGE_MIN_LOG10 && empty.1 == 0.0 && !cell_covered(empty.0, empty.1)) {
        failures.push(format!("a cell with no match of 25 expected must be uncovered: {empty:?}"));
    }
    if !(sparse.0 >= COVERAGE_MIN_LOG10 && cell_covered(sparse.0, sparse.1)) {
        failures.push(format!("a cell with 1 match of 3 predicted stars carries no deficit: {sparse:?}"));
    }
    if !(uniform.0 > -1.0 && cell_covered(uniform.0, uniform.1)) {
        failures.push(format!("a uniform match fraction must be covered with margin: {uniform:?}"));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_correct_solution_with_one_cell_behind_an_extinction_patch_is_accepted() {
    let params = LocalSolveParams::default();
    let truth = TruthField::with_long_side((140.0, -12.0), 2.0, 20.0, false, FRAME.0, FRAME.1);
    let field = truth.field();
    let diagonal = field.width_deg.hypot(field.height_deg);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        1.2 * diagonal,
        truth.density_for(PARTIAL_FIELD_STARS_IN_FRAME),
        (8.0, 13.0),
        700,
    );
    let whole = detections_from(&truth, provider.rows(), 13.0, 0.05, 20, 701);
    let hints = truth.hints(0.05, 10.0);
    let cell = (
        truth.width as f64 / COVERAGE_GRID as f64,
        truth.height as f64 / COVERAGE_GRID as f64,
    );
    let mut rng = StdRng::seed_from_u64(702);
    let mut hidden = 0usize;
    let mut patched: Vec<Detection> = whole
        .into_iter()
        .filter(|d| {
            let behind_the_patch = d.star.x < cell.0 && d.star.y < cell.1;
            if behind_the_patch && rng.gen::<f64>() >= EXTINCTION_PATCH_SURVIVING_FRACTION {
                hidden += 1;
                false
            } else {
                true
            }
        })
        .collect();
    patched.extend(random_detections(hidden, cell.0 as usize, cell.1 as usize, 703));
    let in_patch = |x: f64, y: f64| x < cell.0 && y < cell.1;
    let spread = spread_detections(&usable_detections(&patched, &params), truth.size(), params.detection_stars);
    let radius = match_radius(&spread, &params);
    let predicted_in_patch: Vec<(f64, f64)> = provider
        .rows()
        .iter()
        .map(|row| truth.to_pixel(row.ra, row.dec))
        .filter(|&(x, y)| in_patch(x, y))
        .collect();
    let detected_in_patch = spread.iter().filter(|d| in_patch(d.star.x, d.star.y)).count();
    let matched_in_patch = predicted_in_patch
        .iter()
        .filter(|&&(x, y)| spread.iter().any(|d| (d.star.x - x).hypot(d.star.y - y) <= radius))
        .count();
    let label = format!(
        "{hidden} catalogue stars hidden behind an extinction patch over one of the {} cells and {hidden} uncatalogued stars detected in their place (patch cell: {} predicted, {detected_in_patch} of the {} verification detections, {matched_in_patch} within {radius:.1} px)",
        COVERAGE_GRID * COVERAGE_GRID,
        predicted_in_patch.len(),
        spread.len()
    );
    match solve_detections(&truth, &provider, &patched, &hints, &params) {
        Err(failure) => panic!(
            "{label}: the correct solution was refused: {} (best {:?})",
            failure.message, failure.best
        ),
        Ok(solution) => {
            let corner = truth.worst_corner_px(&solution.wcs_cards);
            eprintln!(
                "{label}: {} matched of {} predicted, rms {:.3} px, worst corner {corner:.3} px, {} centres",
                solution.matched, solution.predicted, solution.rms_px, solution.centres_tried
            );
            assert!(at_most(corner, SOLVED_WORST_CORNER_PX), "worst corner {corner:.3} px");
        }
    }
}

fn true_pairs_in_best_window(
    truth: &TruthField,
    side: &MatchSide,
    rows: &[CatalogRow],
    ranking: RankingKey,
    params: &LocalSolveParams,
) -> (usize, usize) {
    let sky = ranked_sky(rows, 0, params.catalogue_stars, ranking);
    let coincidence = 1.5;
    let true_ranks: Vec<usize> = sky
        .iter()
        .enumerate()
        .filter(|(_, &(ra, dec))| {
            let (tx, ty) = truth.to_pixel(ra, dec);
            side.stars
                .iter()
                .any(|&(x, y)| (x - tx).hypot(y - ty) <= coincidence)
        })
        .map(|(rank, _)| rank)
        .collect();
    let window = ((side.params.max_stars as f64 * MATCH_WINDOW_FRACTION).round() as usize)
        .max(side.params.min_inliers);
    let step = (window / 2).max(1);
    let best = (0..MATCH_WINDOWS)
        .map(|index| {
            let start = index * step;
            true_ranks
                .iter()
                .filter(|&&rank| rank >= start && rank < start + window)
                .count()
        })
        .max()
        .unwrap_or(0);
    (true_ranks.len(), best)
}

#[test]
fn a_2mass_tier_ranks_its_cone_by_the_optical_equivalent_of_j_and_j_ks() {
    let params = LocalSolveParams::default();
    let truth = TruthField::new((300.0, 40.0), 2.4, 0.0, false, FRAME.0, FRAME.1);
    let field = truth.field();
    let diagonal = field.width_deg.hypot(field.height_deg);
    let optical = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        1.2 * diagonal,
        COLOUR_FIELD_STARS_PER_DEG2,
        COLOUR_FIELD_OPTICAL_MAGS,
        720,
    );
    let coloured = coloured_rows(optical.rows(), &COLOUR_LOCUS, 721);
    let by_j_alone = SyntheticProvider::new(TIER1_LABEL, coloured.infrared.clone(), 16.0);
    let provider = SyntheticProvider::new(TIER1_LABEL, coloured.infrared.clone(), 16.0)
        .with_ranking(RankingKey::TWO_MASS_J_KS);
    let bloated_flux = flux_for(COLOUR_FIELD_BLOATED_BRIGHTER_THAN, COLOUR_FIELD_DEPTH_R, 1000.0);
    let detections: Vec<Detection> =
        detections_from(&truth, &coloured.optical, COLOUR_FIELD_DEPTH_R, 0.05, 30, 722)
            .into_iter()
            .filter(|d| d.star.flux < bloated_flux)
            .collect();
    let hints = truth.hints(0.3, 240.0);
    let hint = hints.centre.as_ref().map(|c| (c.ra, c.dec)).expect("hint");
    let usable = usable_detections(&detections, &params);
    let disc_radius_px = params.cone_short_side_fraction * truth.width.min(truth.height) as f64;
    let points = disc_points(&usable, truth.size(), disc_radius_px);
    let inlier_px = ransac_inlier_px(truth.width.max(truth.height), truth.long_side_deg());
    let side = MatchSide::new(&points, params.hinted_match_stars, inlier_px);
    let cone_radius = params.cone_short_side_fraction * field.width_deg.min(field.height_deg);
    let (rows, _) = provider.cone(hint.0, hint.1, cone_radius, None).expect("cone");
    let by_j = true_pairs_in_best_window(&truth, &side, &rows, RankingKey::MAGNITUDE, &params);
    let by_key = true_pairs_in_best_window(&truth, &side, &rows, RankingKey::TWO_MASS_J_KS, &params);
    eprintln!(
        "cone of {} 2MASS-like stars, {} compact image stars in the disc: true pairs (all / best window) by J {:?}, by J + 2 clip(J - Ks) {:?}",
        rows.len(),
        side.stars.len(),
        by_j,
        by_key
    );
    assert!(
        by_j.1 < by_key.1,
        "the fixture must rank the image stars better with the colour key: {by_j:?} vs {by_key:?}"
    );
    match solve_detections(&truth, &by_j_alone, &detections, &hints, &params) {
        Ok(solution) => panic!(
            "ranking by J alone must not reach the triangle stage on this cone: {} matched of {} predicted, worst corner {:.3} px",
            solution.matched,
            solution.predicted,
            truth.worst_corner_px(&solution.wcs_cards)
        ),
        Err(failure) => {
            eprintln!("by J alone: {}", failure.message);
            assert_eq!(failure.reason, FailureReason::NoSolution, "{failure:?}");
        }
    }
    let solution = solve_detections(&truth, &provider, &detections, &hints, &params)
        .unwrap_or_else(|failure| panic!("the 2MASS-like cone must solve with the colour key: {failure:?}"));
    let corner = truth.worst_corner_px(&solution.wcs_cards);
    eprintln!(
        "with the colour key: {} matched of {} predicted, rms {:.3} px, worst corner {corner:.3} px, {} centres",
        solution.matched, solution.predicted, solution.rms_px, solution.centres_tried
    );
    assert!(at_most(corner, SOLVED_WORST_CORNER_PX), "worst corner {corner:.3} px");
}

fn add_galaxy(image: &mut Array2<f32>, centre: (f64, f64), sigma_px: f64, peak: f64) {
    let (rows, cols) = image.dim();
    for row in 0..rows {
        for col in 0..cols {
            let (dx, dy) = (col as f64 - centre.0, row as f64 - centre.1);
            let r2 = dx * dx + dy * dy;
            image[[row, col]] += (peak * (-r2 / (2.0 * sigma_px * sigma_px)).exp()) as f32;
        }
    }
}

#[test]
fn stars_on_a_bright_extended_source_are_detected_and_the_frame_solves() {
    let params = LocalSolveParams::default();
    let truth = TruthField::new((10.68, 41.27), 3.2, 0.0, false, FRAME.0, FRAME.1);
    let field = truth.field();
    let diagonal = field.width_deg.hypot(field.height_deg);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        1.2 * diagonal,
        TIER1_CUT_STARS_PER_DEG2,
        (6.0, 15.5),
        730,
    );
    let mut image = render_field(
        &truth,
        provider.rows(),
        15.5,
        &FrameStyle {
            seed: 731,
            ..FrameStyle::default()
        },
    );
    let galaxy_centre = (
        (truth.width as f64 - 1.0) / 2.0 + 90.0,
        (truth.height as f64 - 1.0) / 2.0 - 50.0,
    );
    add_galaxy(&mut image, galaxy_centre, GALAXY_SIGMA_PX, GALAXY_PEAK_ADU);
    add_galaxy(
        &mut image,
        galaxy_centre,
        GALAXY_NUCLEUS_SIGMA_PX,
        GALAXY_NUCLEUS_PEAK_ADU,
    );
    let hints = truth.hints(0.05, 320.0);
    let on_the_disc = |x: f64, y: f64| {
        (x - galaxy_centre.0).hypot(y - galaxy_centre.1) <= GALAXY_SIGMA_PX
    };
    let catalogued: Vec<(f64, f64)> = provider
        .rows()
        .iter()
        .map(|row| truth.to_pixel(row.ra, row.dec))
        .filter(|&(x, y)| truth.inside(x, y))
        .collect();
    let recovered = |detections: &[Detection], region: &dyn Fn(f64, f64) -> bool| {
        let stars: Vec<&(f64, f64)> = catalogued.iter().filter(|&&(x, y)| region(x, y)).collect();
        let found = stars
            .iter()
            .filter(|&&&(x, y)| {
                detections
                    .iter()
                    .any(|d| (d.star.x - x).hypot(d.star.y - y) <= params.match_radius_px)
            })
            .count();
        (found, stars.len())
    };
    let global_background = LocalSolveParams {
        background_tile_px: 0,
        ..LocalSolveParams::default()
    };
    let with_global = detect_for_solve(&image, &hints, &global_background);
    let control = recovered(&with_global, &on_the_disc);
    let control_clear = recovered(&with_global, &|x, y| !on_the_disc(x, y));
    eprintln!(
        "one global background: {} detections, {} of the {} catalogue stars on the disc recovered within {} px, {} of {} off it",
        with_global.len(),
        control.0,
        control.1,
        params.match_radius_px,
        control_clear.0,
        control_clear.1
    );
    let detections = detect_for_solve(&image, &hints, &params);
    let disc = recovered(&detections, &on_the_disc);
    let clear = recovered(&detections, &|x, y| !on_the_disc(x, y));
    let label = format!(
        "{:.2} deg frame with a {GALAXY_PEAK_ADU} ADU, sigma {GALAXY_SIGMA_PX} px galaxy over a {} ADU sky: {} detections, {} of the {} catalogue stars on the disc recovered, {} of {} off it",
        truth.long_side_deg(),
        FrameStyle::default().background,
        detections.len(),
        disc.0,
        disc.1,
        clear.0,
        clear.1
    );
    eprintln!("{label}");
    let floor = GALAXY_MIN_RECOVERED_FRACTION * disc.1 as f64;
    let mut failures = Vec::new();
    if (control.0 as f64) >= floor {
        failures.push(format!(
            "{label}: the control with one global background recovers {} of {} stars on the disc, so this fixture does not show the defect",
            control.0, control.1
        ));
    }
    if (disc.0 as f64) < floor {
        failures.push(format!(
            "{label}: at least {GALAXY_MIN_RECOVERED_FRACTION} of the stars on the galaxy must survive detection"
        ));
    }
    match solve_local(
        &image,
        &hints,
        &provider,
        &params,
        &never_cancelled,
        &no_stage,
    ) {
        Err(failure) => failures.push(format!("{label}: {failure:?}")),
        Ok(solution) => {
            let corner = truth.worst_corner_px(&solution.wcs_cards);
            eprintln!(
                "{label}: {} matched of {} predicted, rms {:.3} px, worst corner {corner:.3} px, {} centres",
                solution.matched, solution.predicted, solution.rms_px, solution.centres_tried
            );
            if !at_most(corner, SOLVED_WORST_CORNER_PX) {
                failures.push(format!("{label}: worst corner {corner:.3} px"));
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
fn tess_like(truth: &TruthField, quadratic_px: f64, cubic_px: f64, quartic_px: f64) -> impl Fn(f64, f64) -> (f64, f64) {
    let (xc, yc) = (
        (truth.width as f64 - 1.0) / 2.0,
        (truth.height as f64 - 1.0) / 2.0,
    );
    let half_diagonal = (truth.width as f64 / 2.0).hypot(truth.height as f64 / 2.0);
    move |x, y| {
        let (u, v) = ((x - xc) / half_diagonal, (y - yc) / half_diagonal);
        let r2 = u * u + v * v;
        (
            x + quadratic_px * (0.8 * u * u - 0.5 * u * v + 0.3 * v * v) + cubic_px * r2 * u + quartic_px * (u.powi(4) + 3.0 * u * u * v * v),
            y + quadratic_px * (0.4 * v * v + 0.6 * u * v - 0.2 * u * u) + cubic_px * r2 * v + quartic_px * (v.powi(4) - 2.0 * u * u * v * v),
        )
    }
}

fn inverse_of(
    forward: impl Fn(f64, f64) -> (f64, f64) + Send + Sync + 'static,
) -> impl Fn(f64, f64) -> (f64, f64) + Send + Sync {
    move |x, y| {
        let (mut mx, mut my) = (x, y);
        for _ in 0..INVERSE_CUBIC_NEWTON_STEPS {
            let (fx, fy) = forward(mx, my);
            let step = 0.5;
            let (fxx, fyx) = forward(mx + step, my);
            let (fxy, fyy) = forward(mx, my + step);
            let j = [
                [(fxx - fx) / step, (fxy - fx) / step],
                [(fyx - fy) / step, (fyy - fy) / step],
            ];
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if det.abs() < 1e-12 {
                break;
            }
            let (rx, ry) = (fx - x, fy - y);
            mx -= (j[1][1] * rx - j[0][1] * ry) / det;
            my -= (-j[1][0] * rx + j[0][0] * ry) / det;
        }
        (mx, my)
    }
}

fn worst_error_by_radial_fifth(
    truth: &TruthField,
    undistort: &dyn Fn(f64, f64) -> (f64, f64),
    cards: &[(String, String)],
) -> [f64; 5] {
    let wcs = wcs_from_cards(cards, truth.width, truth.height);
    let scale_deg = truth.scale_arcsec() / ARCSEC_PER_DEG;
    let half_diagonal = (truth.width as f64 / 2.0).hypot(truth.height as f64 / 2.0);
    let (xc, yc) = (
        (truth.width as f64 - 1.0) / 2.0,
        (truth.height as f64 - 1.0) / 2.0,
    );
    let mut by_band = [0.0f64; 5];
    for (mx, my) in grid(truth.width - 1, truth.height - 1, 17) {
        let (mx, my) = (mx + 0.5, my + 0.5);
        let (x, y) = undistort(mx, my);
        let (ra, dec) = truth.to_sky(x, y);
        let got = wcs.pixel_to_world(mx, my);
        let error = angular_separation(ra, dec, got.ra, got.dec) / scale_deg;
        let band = (((mx - xc).hypot(my - yc) / half_diagonal * 5.0).floor() as usize).min(4);
        by_band[band] = worse(by_band[band], error);
    }
    by_band
}

#[test]
fn a_tess_like_order_4_distortion_is_recovered_out_to_the_corners() {
    let truth = TruthField::new(
        (70.0, -60.0),
        TESS_LIKE_SCALE_ARCSEC,
        15.0,
        false,
        TESS_LIKE_FRAME.0,
        TESS_LIKE_FRAME.1,
    );
    let field = truth.field();
    let diagonal = field.width_deg.hypot(field.height_deg);
    let provider = SyntheticProvider::uniform(
        TIER1_LABEL,
        truth.centre(),
        1.2 * diagonal,
        truth.density_for(TESS_LIKE_STARS_IN_FRAME),
        (6.0, 12.0),
        740,
    );
    let (quadratic, cubic, quartic) = TESS_LIKE_DISTORTION_PX;
    let undistort = tess_like(&truth, quadratic, cubic, quartic);
    let image = render_field(
        &truth,
        provider.rows(),
        12.0,
        &FrameStyle {
            seed: 741,
            uncatalogued: 2000,
            distortion: Some(Box::new(inverse_of(tess_like(
                &truth, quadratic, cubic, quartic,
            )))),
            ..FrameStyle::default()
        },
    );
    let hints = truth.hints(2.0 / truth.long_side_deg(), 50.0);
    let triangle_list_only = LocalSolveParams {
        sip_stars_per_band: 0,
        ..LocalSolveParams::default()
    };
    let params = LocalSolveParams::default();
    let mut outcomes = Vec::new();
    for (name, params) in [("triangle list", &triangle_list_only), ("radial bands", &params)] {
        let solution = solve_local(
            &image,
            &hints,
            &provider,
            params,
            &never_cancelled,
            &no_stage,
        )
        .unwrap_or_else(|failure| panic!("the TESS-like frame must solve ({name}): {failure:?}"));
        let by_band = worst_error_by_radial_fifth(&truth, &undistort, &solution.wcs_cards);
        eprintln!(
            "{:.1} deg frame with {quadratic} + {cubic} + {quartic} px of quadratic, cubic and quartic distortion at the corner, SIP stars from the {name}: sip {:?}, {} matched of {} predicted, rms {:.3} px, {} centres, worst error by radial fifth of the half diagonal {:?} px",
            truth.long_side_deg(),
            solution.sip_order,
            solution.matched,
            solution.predicted,
            solution.rms_px,
            solution.centres_tried,
            by_band
        );
        outcomes.push((name, solution.sip_order, solution.centres_tried, by_band));
    }
    let mut failures = Vec::new();
    for (name, order, centres, by_band) in &outcomes {
        let worst = by_band.iter().copied().fold(0.0, worse);
        if !at_most(worst, DISTORTED_MAX_ERROR_PX) {
            failures.push(format!(
                "{name}: the corners must be fitted, not extrapolated: {by_band:?}"
            ));
        }
        if *order != Some(4) {
            failures.push(format!("{name}: sip order {order:?}"));
        }
        if *centres != 1 {
            failures.push(format!(
                "{name}: the hint 2 deg off must solve at the first centre of the fine spiral, not after {centres}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn projective_residual_px(
    centre: (f64, f64),
    offset_deg: f64,
    cone_radius_deg: f64,
    scale_deg: f64,
) -> f64 {
    let candidate = destination(centre, 33.0, offset_deg);
    let mut points = vec![centre];
    for step in 0..12 {
        points.push(destination(centre, 30.0 * step as f64, cone_radius_deg));
    }
    let pairs: Vec<(f64, f64, f64, f64)> = points
        .iter()
        .map(|&(ra, dec)| {
            let (tx, ty) = tan_plane(centre, ra, dec).expect("inside the hemisphere");
            let (cx, cy) = tan_plane(candidate, ra, dec).expect("inside the hemisphere");
            (-cx / scale_deg, cy / scale_deg, -tx / scale_deg, ty / scale_deg)
        })
        .collect();
    let fit = crate::core::alignment::triangles::fit_affine(&pairs).expect("affine");
    pairs
        .iter()
        .map(|&(rx, ry, tx, ty)| {
            let (px, py) = fit.map(rx, ry);
            (px - tx).hypot(py - ty)
        })
        .fold(0.0, worse)
}

#[test]
fn the_spiral_spacing_keeps_the_projective_residual_of_a_wrong_centre_under_half_the_pixel_tolerance(
) {
    let params = LocalSolveParams::default();
    let shipped = LocalSolveParams {
        spiral_projective_px: f64::INFINITY,
        ..LocalSolveParams::default()
    };
    let budget = TriangleParams::SOLVER.inlier_px / 2.0;
    assert_eq!(params.spiral_projective_px, budget);
    let mut failures = Vec::new();
    for &(label, width, height, scale_arcsec, wide) in &[
        ("1 deg", 3000usize, 2000usize, 1.2f64, false),
        ("2.5 deg", 3000, 2000, 3.0, false),
        ("5 deg", 3000, 2000, 6.0, false),
        (
            "TESS 12 deg",
            TESS_LIKE_FRAME.0,
            TESS_LIKE_FRAME.1,
            TESS_LIKE_SCALE_ARCSEC,
            true,
        ),
        ("30 deg", 3000, 2000, 36.85, true),
    ] {
        let truth = TruthField::new((120.0, -30.0), scale_arcsec, 0.0, false, width, height);
        let field = truth.field();
        let short_deg = field.width_deg.min(field.height_deg);
        let scale_deg = scale_arcsec / ARCSEC_PER_DEG;
        let cone_radius_deg = params.cone_short_side_fraction * short_deg;
        let coarse = spiral_spacing_deg(scale_deg, short_deg, cone_radius_deg, &shipped);
        let spacing = spiral_spacing_deg(scale_deg, short_deg, cone_radius_deg, &params);
        let worst_offset = spacing / 3f64.sqrt();
        let residual =
            projective_residual_px(truth.centre(), worst_offset, cone_radius_deg, scale_deg);
        let coarse_residual = projective_residual_px(
            truth.centre(),
            coarse / 3f64.sqrt(),
            cone_radius_deg,
            scale_deg,
        );
        let radius = params
            .near_radius_deg
            .max(params.near_radius_fields * truth.long_side_deg())
            .clamp(0.0, MAX_NEAR_RADIUS_DEG);
        let plan = search_plan(truth.centre(), &[spacing], radius, params.max_centres);
        eprintln!(
            "{label}: coarse spacing {coarse:.3} deg leaves {coarse_residual:.2} px at its worst centre; spacing {spacing:.3} deg leaves {residual:.2} px; {} centres to {:.2} deg{}",
            plan.centres,
            plan.radius_reached_deg,
            if plan.capped { " (capped)" } else { "" }
        );
        if coarse != params.spiral_spacing_fraction * short_deg {
            failures.push(format!(
                "{label}: the shipped spacing must be {} of the short side",
                params.spiral_spacing_fraction
            ));
        }
        if !at_most(residual, 1.05 * budget) {
            failures.push(format!(
                "{label}: {residual:.2} px of projective residual at the worst centre, budget {budget} px"
            ));
        }
        if wide {
            if !(spacing < coarse && residual >= 0.5 * budget) {
                failures.push(format!(
                    "{label}: a wide field must use a finer spacing that spends its budget: {spacing:.3} vs {coarse:.3} deg, {residual:.2} px"
                ));
            }
            if coarse_residual <= budget {
                failures.push(format!(
                    "{label}: the fixture must show the coarse spacing over budget: {coarse_residual:.2} px"
                ));
            }
        } else if spacing != coarse {
            failures.push(format!(
                "{label}: a narrow field must keep the coarse spacing: {spacing:.3} vs {coarse:.3} deg"
            ));
        }
        if label == "TESS 12 deg" && plan.capped {
            failures.push(format!(
                "{label}: the finer spiral must still cover the search disc within {} centres",
                params.max_centres
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
