use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use ndarray::Array2;
use rayon::prelude::*;

use crate::core::astrometry::solve_hints::SolveHints;
use crate::core::stacking::{CancelCheck, StageCallback};

use crate::core::astrometry::local_solve::constants::{
    MAX_NEAR_RADIUS_DEG, STAGE_DETECT, STAGE_SEARCH, STAGE_FIT, ARCSEC_PER_DEG, MAX_TAN_FIELD_DEG,
};
use crate::core::astrometry::local_solve::catalog_tiers::CatalogProvider;
use crate::core::astrometry::local_solve::types::{
    Detection, LocalSolveParams, LocalSolution, FailureReason, CandidateNumbers, SolveFailure,
};
use super::search_plan::{search_plan, scale_hypotheses};
use crate::core::astrometry::local_solve::tan_projection::{is_positive, scale_text, scale_arcsec, rotation_deg, parity_of};
use crate::core::astrometry::local_solve::detection::{
    detect_for_solve, usable_detections, dropped_saturated, disc_points, spread_detections,
    radial_spread,
};
use crate::core::astrometry::local_solve::triangle_match::MatchSide;
use super::centre_trial::{SearchContext, Hypothesis, Solved, try_centre};

pub fn solve_local(
    image: &Array2<f32>,
    hints: &SolveHints,
    catalog: &dyn CatalogProvider,
    params: &LocalSolveParams,
    cancel: CancelCheck,
    stage: StageCallback,
) -> Result<LocalSolution, SolveFailure> {
    let started = Instant::now();
    stage(STAGE_DETECT);
    let (height, width) = image.dim();
    let detections = detect_for_solve(image, hints, params);
    solve_detections(
        started,
        &detections,
        (width, height),
        hints,
        catalog,
        params,
        cancel,
        stage,
    )
}

pub fn solve_stars(
    detections: &[Detection],
    size: (usize, usize),
    hints: &SolveHints,
    catalog: &dyn CatalogProvider,
    params: &LocalSolveParams,
    cancel: CancelCheck,
    stage: StageCallback,
) -> Result<LocalSolution, SolveFailure> {
    solve_detections(
        Instant::now(),
        detections,
        size,
        hints,
        catalog,
        params,
        cancel,
        stage,
    )
}

fn better_candidate(a: &CandidateNumbers, b: &CandidateNumbers) -> bool {
    (a.matched, -a.false_alarm_log10, -a.rms_px) > (b.matched, -b.false_alarm_log10, -b.rms_px)
}

fn record_best(best: &Mutex<Option<CandidateNumbers>>, candidate: &CandidateNumbers) {
    let mut slot = best.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot
        .as_ref()
        .is_none_or(|current| better_candidate(candidate, current))
    {
        *slot = Some(candidate.clone());
    }
}

fn best_snapshot(best: &Mutex<Option<CandidateNumbers>>) -> Option<CandidateNumbers> {
    best.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn finish_solution(
    solved: Solved,
    detections: usize,
    centres_tried: usize,
    started: Instant,
) -> LocalSolution {
    let distortion = solved.distortion;
    let tan = distortion.tan;
    let sip_order = (distortion.order >= 2).then_some(distortion.order);
    LocalSolution {
        wcs_cards: distortion.wcs_cards,
        tan,
        centre: tan.crval,
        scale_arcsec: scale_arcsec(&tan.cd),
        rotation_deg: rotation_deg(&tan.cd),
        parity: parity_of(&tan.cd),
        matched: solved.verification.matched,
        predicted: solved.verification.predicted,
        rms_px: solved.rms_px,
        false_alarm_log10: solved.verification.false_alarm_log10,
        sip_order,
        sip_max_err_px: distortion.sip_max_err_px,
        detections,
        centres_tried,
        elapsed: started.elapsed(),
        warnings: solved.warnings,
    }
}

fn frozen_epoch_warning(catalog: &dyn CatalogProvider, observed_year: Option<f64>) -> Option<String> {
    let epoch = catalog.fixed_epoch_year().filter(|year| year.is_finite())?;
    let observed = observed_year
        .filter(|year| year.is_finite())
        .map_or_else(String::new, |year| format!(" (J{year:.1})"));
    Some(format!(
        "{} holds star positions at J{epoch:.1} without proper motions, so this solution is tied to the sky of J{epoch:.1}, not of the observation date{observed}; refine with Gaia DR3 for the observation epoch",
        catalog.label()
    ))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::core::astrometry::local_solve) fn solve_detections(
    started: Instant,
    detections: &[Detection],
    size: (usize, usize),
    hints: &SolveHints,
    catalog: &dyn CatalogProvider,
    params: &LocalSolveParams,
    cancel: CancelCheck,
    stage: StageCallback,
) -> Result<LocalSolution, SolveFailure> {
    let failure = |reason: FailureReason,
                   message: String,
                   detections: usize,
                   centres_tried: usize,
                   best: Option<CandidateNumbers>| SolveFailure {
        reason,
        message,
        detections,
        centres_tried,
        best: best.map(Box::new),
        elapsed: started.elapsed(),
    };
    let Some(centre) = hints.centre.as_ref() else {
        return Err(failure(
            FailureReason::MissingHint,
            "no pointing hint: the local solver needs an approximate centre".to_string(),
            detections.len(),
            0,
            None,
        ));
    };
    let Some(scale) = hints.scale.as_ref() else {
        return Err(failure(
            FailureReason::MissingHint,
            "no pixel scale hint: the local solver needs a scale range".to_string(),
            detections.len(),
            0,
            None,
        ));
    };
    let nominal = scale_hypotheses(scale.low_arcsec, scale.high_arcsec);
    let (width, height) = size;
    if !nominal.iter().all(|&s| is_positive(s))
        || !centre.ra.is_finite()
        || !centre.dec.is_finite()
        || width == 0
        || height == 0
    {
        return Err(failure(
            FailureReason::MissingHint,
            format!(
                "unusable hints: centre ({}, {}), scale {}..{} arcsec/px, size {width}x{height}",
                centre.ra, centre.dec, scale.low_arcsec, scale.high_arcsec
            ),
            detections.len(),
            0,
            None,
        ));
    }
    let hinted_long_side_deg = scale.high_arcsec * width.max(height) as f64 / ARCSEC_PER_DEG;
    if hinted_long_side_deg > MAX_TAN_FIELD_DEG {
        return Err(failure(
            FailureReason::MissingHint,
            format!(
                "unusable scale hint: {}..{} arcsec/px from {} gives a {hinted_long_side_deg:.0} deg field, wider than a TAN image can be (at most {MAX_TAN_FIELD_DEG:.0} deg)",
                scale.low_arcsec, scale.high_arcsec, scale.source
            ),
            detections.len(),
            0,
            None,
        ));
    }
    let usable = usable_detections(detections, params);
    let selected = spread_detections(&usable, size, params.detection_stars);
    let n_det = selected.len();
    if n_det < params.min_matches {
        return Err(failure(
            FailureReason::TooFewStars,
            format!(
                "{n_det} usable stars detected, at least {} needed",
                params.min_matches
            ),
            n_det,
            0,
            None,
        ));
    }
    if cancel() {
        return Err(failure(
            FailureReason::Cancelled,
            "cancelled before the search started".to_string(),
            n_det,
            0,
            None,
        ));
    }
    let disc_radius_px = params.cone_short_side_fraction * width.min(height) as f64;
    let points = disc_points(&usable, size, disc_radius_px);
    let saturated = dropped_saturated(detections, params);
    let mut hypotheses: Vec<Hypothesis> = nominal
        .iter()
        .map(|&arcsec| Hypothesis::new(arcsec, &points, saturated, size, params))
        .collect();
    let spacings: Vec<f64> = hypotheses.iter().map(|h| h.spacing_deg).collect();
    let widest_field_deg =
        nominal.iter().copied().fold(0.0f64, f64::max) * width.max(height) as f64 / ARCSEC_PER_DEG;
    let requested_radius = params
        .near_radius_deg
        .max(params.near_radius_fields * widest_field_deg)
        .clamp(0.0, MAX_NEAR_RADIUS_DEG);
    let plan = search_plan(
        (centre.ra, centre.dec),
        &spacings,
        requested_radius,
        params.max_centres.max(1),
    );
    if plan.rings.len() > 1 {
        for hypothesis in &mut hypotheses {
            let inlier_px = hypothesis.hinted.params.inlier_px;
            hypothesis.spiral = Some(MatchSide::new(
                &points,
                params.spiral_match_stars,
                inlier_px,
            ));
        }
    }
    let sip_detections = if params.sip_stars_per_band == 0 {
        selected.clone()
    } else {
        radial_spread(&usable, size, params.sip_stars_per_band)
    };
    let ctx = SearchContext {
        detections: selected,
        sip_detections,
        saturated,
        size,
        catalog,
        ranking: catalog.ranking_key(),
        params,
        epoch_year: hints.epoch_year,
        scale_range: (scale.low_arcsec, scale.high_arcsec),
        trials: 2 * plan.centres,
    };
    let tried = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let cancelled = AtomicBool::new(false);
    let best: Mutex<Option<CandidateNumbers>> = Mutex::new(None);
    let catalog_error: Mutex<Option<String>> = Mutex::new(None);
    let last_ring = plan.rings.len() - 1;
    for (k, ring) in plan.rings.iter().enumerate() {
        stage(&format!("{STAGE_SEARCH} ring {k}/{last_ring}"));
        let mut solved: Vec<(usize, Solved)> = ring
            .par_iter()
            .enumerate()
            .filter_map(|(index, &(h, candidate_centre))| {
                if stop.load(Ordering::SeqCst) {
                    return None;
                }
                if cancel() {
                    stop.store(true, Ordering::SeqCst);
                    cancelled.store(true, Ordering::SeqCst);
                    return None;
                }
                tried.fetch_add(1, Ordering::SeqCst);
                let hypothesis = &hypotheses[h];
                match try_centre(candidate_centre, hypothesis, hypothesis.side(k), &ctx) {
                    Ok(outcome) => {
                        if let Some(candidate) = &outcome.candidate {
                            record_best(&best, candidate);
                        }
                        outcome.solved.map(|s| (index, s))
                    }
                    Err(error) => {
                        let mut slot = catalog_error
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        slot.get_or_insert(error);
                        None
                    }
                }
            })
            .collect();
        let centres_tried = tried.load(Ordering::SeqCst);
        if cancelled.load(Ordering::SeqCst) {
            return Err(failure(
                FailureReason::Cancelled,
                format!("cancelled after {centres_tried} centres"),
                n_det,
                centres_tried,
                best_snapshot(&best),
            ));
        }
        solved.sort_by(|(ia, a), (ib, b)| {
            b.verification
                .matched
                .cmp(&a.verification.matched)
                .then(
                    a.verification
                        .false_alarm_log10
                        .total_cmp(&b.verification.false_alarm_log10),
                )
                .then(a.rms_px.total_cmp(&b.rms_px))
                .then(ia.cmp(ib))
        });
        if let Some((_, winner)) = solved.into_iter().next() {
            stage(STAGE_FIT);
            let mut solution = finish_solution(winner, n_det, centres_tried, started);
            solution.warnings.extend(frozen_epoch_warning(catalog, hints.epoch_year));
            return Ok(solution);
        }
    }
    let centres_tried = tried.load(Ordering::SeqCst);
    let best = best_snapshot(&best);
    let catalog_error = catalog_error
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match (best.as_ref(), catalog_error) {
        (None, Some(error)) => Err(failure(
            FailureReason::Catalog,
            format!("catalogue read failed: {error}"),
            n_det,
            centres_tried,
            None,
        )),
        (best, _) => {
            let detail = match best {
                Some(b) => format!(
                    "best candidate matched {} of {} predicted stars, rms {:.2} px, log10 P {:.1}, cell coverage log10 P {:.1}, cell matches above chance log10 P {:.1}",
                    b.matched,
                    b.predicted,
                    b.rms_px,
                    b.false_alarm_log10,
                    b.coverage_log10,
                    b.coverage_chance_log10
                ),
                None => format!(
                    "no centre produced {} consistent star matches (scale hint {}..{} arcsec/px from {})",
                    params.min_matches,
                    scale_text(scale.low_arcsec),
                    scale_text(scale.high_arcsec),
                    scale.source
                ),
            };
            let searched = if plan.capped {
                format!(
                    "{:.2} deg of the hint ({centres_tried} centres, search capped at {} centres; {requested_radius:.2} deg requested)",
                    plan.radius_reached_deg, params.max_centres
                )
            } else {
                format!("{requested_radius:.2} deg of the hint after {centres_tried} centres")
            };
            Err(failure(
                FailureReason::NoSolution,
                format!("no solution within {searched}: {detail}"),
                n_det,
                centres_tried,
                best.cloned(),
            ))
        }
    }
}
