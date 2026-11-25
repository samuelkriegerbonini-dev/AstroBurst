use crate::core::registration::poly::TanParams;

use crate::core::astrometry::local_solve::constants::{TAN_HYPOTHESIS_PAIRS, POISSON_TERM_CUTOFF, POISSON_MAX_TERMS, COVERAGE_GRID};
use crate::core::astrometry::local_solve::types::{Detection, SkyPair, Verification, cell_covered, LocalSolveParams};
use super::tan_projection::{is_positive, sky_to_pixel, scale_arcsec};
use super::tan_fit::clipped_rms;
use crate::core::astrometry::local_solve::pairing::{match_radius, inside, nearest_pairs, corners_finite};

#[derive(Debug, Clone, Copy)]
pub(in crate::core::astrometry::local_solve) struct StageGate {
    pub(in crate::core::astrometry::local_solve) max_rms_px: f64,
    pub(in crate::core::astrometry::local_solve) trials: usize,
}

pub(in crate::core::astrometry::local_solve) fn false_alarm_log10(matched: usize, lambda: f64, trials: usize) -> f64 {
    let scored = matched.saturating_sub(TAN_HYPOTHESIS_PAIRS);
    (poisson_tail_log10(scored, lambda) + (trials.max(1) as f64).log10()).min(0.0)
}

type SkyToPixel<'a> = &'a dyn Fn(f64, f64) -> Option<(f64, f64)>;

pub(in crate::core::astrometry::local_solve) fn verify_pairs(
    tan: &TanParams,
    detections: &[Detection],
    catalogue: &[(f64, f64)],
    size: (usize, usize),
    scale_range_arcsec: Option<(f64, f64)>,
    params: &LocalSolveParams,
    gate: StageGate,
) -> (Verification, Vec<SkyPair>) {
    let predict = |ra: f64, dec: f64| sky_to_pixel(tan, ra, dec);
    verify_with(
        &predict,
        match_radius(detections, params),
        tan,
        detections,
        catalogue,
        size,
        scale_range_arcsec,
        params,
        gate,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::core::astrometry::local_solve) fn verify_with(
    predict: SkyToPixel,
    radius: f64,
    tan: &TanParams,
    detections: &[Detection],
    catalogue: &[(f64, f64)],
    size: (usize, usize),
    scale_range_arcsec: Option<(f64, f64)>,
    params: &LocalSolveParams,
    gate: StageGate,
) -> (Verification, Vec<SkyPair>) {
    let (width, height) = size;
    let projected: Vec<((f64, f64), (f64, f64))> = catalogue
        .iter()
        .filter_map(|&(ra, dec)| {
            let pixel = predict(ra, dec)?;
            (pixel.0.is_finite() && pixel.1.is_finite() && inside(size, pixel))
                .then_some((pixel, (ra, dec)))
        })
        .collect();
    let positions: Vec<(f64, f64)> = projected.iter().map(|(pixel, _)| *pixel).collect();
    let matched = nearest_pairs(&positions, detections, radius);
    let distances: Vec<f64> = matched.iter().map(|&(_, _, d)| d).collect();
    let rms = clipped_rms(&distances, params.clip_sigma);
    let (coverage_log10, coverage_chance_log10) =
        coverage_log10(&positions, detections, &matched, size, radius);
    let lambda =
        projected.len() as f64 * detections.len() as f64 * std::f64::consts::PI * radius * radius
            / (width as f64 * height as f64);
    let false_alarm_log10 = false_alarm_log10(matched.len(), lambda, gate.trials);
    let corners = corners_finite(tan, size);
    let scale = scale_arcsec(&tan.cd);
    let scale_in_range = scale_range_arcsec.is_none_or(|(low, high)| scale >= low && scale <= high);
    let accepted = matched.len() >= params.min_matches
        && false_alarm_log10 < params.max_false_alarm.log10()
        && rms <= gate.max_rms_px
        && corners
        && scale_in_range;
    let pairs = matched
        .iter()
        .map(|&(i, j, _)| {
            let (ra, dec) = projected[i].1;
            SkyPair {
                x: detections[j].star.x,
                y: detections[j].star.y,
                ra,
                dec,
            }
        })
        .collect();
    (
        Verification {
            matched: matched.len(),
            predicted: projected.len(),
            detections: detections.len(),
            radius_px: radius,
            lambda,
            false_alarm_log10,
            rms_px: rms,
            coverage_log10,
            coverage_chance_log10,
            corners_finite: corners,
            scale_in_range,
            accepted,
        },
        pairs,
    )
}

fn coverage_cell((width, height): (usize, usize), (x, y): (f64, f64)) -> usize {
    let last = (COVERAGE_GRID - 1) as f64;
    let column = ((x + 0.5) * COVERAGE_GRID as f64 / width as f64)
        .floor()
        .clamp(0.0, last) as usize;
    let row = ((y + 0.5) * COVERAGE_GRID as f64 / height as f64)
        .floor()
        .clamp(0.0, last) as usize;
    row * COVERAGE_GRID + column
}

pub(in crate::core::astrometry::local_solve) fn coverage_log10(
    positions: &[(f64, f64)],
    detections: &[Detection],
    matched: &[(usize, usize, f64)],
    size: (usize, usize),
    radius: f64,
) -> (f64, f64) {
    const CELLS: usize = COVERAGE_GRID * COVERAGE_GRID;
    if positions.is_empty() || detections.is_empty() || matched.is_empty() {
        return (0.0, 0.0);
    }
    let mut predicted = [0usize; CELLS];
    let mut detected = [0usize; CELLS];
    let mut found = [0usize; CELLS];
    for &position in positions {
        predicted[coverage_cell(size, position)] += 1;
    }
    for detection in detections {
        detected[coverage_cell(size, (detection.star.x, detection.star.y))] += 1;
    }
    for &(i, _, _) in matched {
        found[coverage_cell(size, positions[i])] += 1;
    }
    let by_predicted = matched.len() as f64 / positions.len() as f64;
    let by_detected = matched.len() as f64 / detections.len() as f64;
    let cell_area = size.0 as f64 * size.1 as f64 / CELLS as f64;
    let chance_per_pair = std::f64::consts::PI * radius * radius / cell_area;
    let mut worst = (0.0, 0.0);
    let mut worst_uncovered: Option<(f64, f64)> = None;
    for cell in 0..CELLS {
        let expected =
            (predicted[cell] as f64 * by_predicted).min(detected[cell] as f64 * by_detected);
        let chance = predicted[cell] as f64 * detected[cell] as f64 * chance_per_pair;
        let deficit = poisson_lower_tail_log10(found[cell], expected);
        let above_chance = poisson_tail_log10(found[cell], chance);
        if deficit < worst.0 {
            worst = (deficit, above_chance);
        }
        if !cell_covered(deficit, above_chance)
            && worst_uncovered.is_none_or(|(d, _)| deficit < d)
        {
            worst_uncovered = Some((deficit, above_chance));
        }
    }
    worst_uncovered.unwrap_or(worst)
}

pub(in crate::core::astrometry::local_solve) fn poisson_lower_tail_log10(k: usize, lambda: f64) -> f64 {
    if !is_positive(lambda) {
        return 0.0;
    }
    let ln_lambda = lambda.ln();
    let mut ln_term = -lambda;
    let mut ln_sum = ln_term;
    for i in 1..=k {
        ln_term += ln_lambda - (i as f64).ln();
        let peak = ln_sum.max(ln_term);
        ln_sum = peak + ((ln_sum - peak).exp() + (ln_term - peak).exp()).ln();
    }
    (ln_sum / std::f64::consts::LN_10).min(0.0)
}

#[cfg(test)]
pub(in crate::core::astrometry::local_solve) fn verify(
    tan: &TanParams,
    detections: &[Detection],
    catalogue: &[(f64, f64)],
    size: (usize, usize),
    scale_range_arcsec: Option<(f64, f64)>,
    params: &LocalSolveParams,
) -> Verification {
    let gate = StageGate {
        max_rms_px: params.max_rms_px,
        trials: 1,
    };
    verify_pairs(
        tan,
        detections,
        catalogue,
        size,
        scale_range_arcsec,
        params,
        gate,
    )
    .0
}

pub(crate) fn poisson_tail_log10(k: usize, lambda: f64) -> f64 {
    if k == 0 {
        return 0.0;
    }
    if !is_positive(lambda) {
        return f64::NEG_INFINITY;
    }
    let ln_lambda = lambda.ln();
    let ln_factorial: f64 = (1..=k).map(|i| (i as f64).ln()).sum();
    let mut ln_term = -lambda + k as f64 * ln_lambda - ln_factorial;
    let mut ln_sum = ln_term;
    for i in k..k + POISSON_MAX_TERMS {
        ln_term += ln_lambda - ((i + 1) as f64).ln();
        let peak = ln_sum.max(ln_term);
        ln_sum = peak + ((ln_sum - peak).exp() + (ln_term - peak).exp()).ln();
        if (i + 1) as f64 > lambda && ln_term - ln_sum < POISSON_TERM_CUTOFF.ln() {
            break;
        }
    }
    ln_sum / std::f64::consts::LN_10
}

pub(in crate::core::astrometry::local_solve) fn quadrants(pairs: &[SkyPair], (width, height): (usize, usize)) -> usize {
    let (xc, yc) = ((width as f64 - 1.0) / 2.0, (height as f64 - 1.0) / 2.0);
    let mut seen = [false; 4];
    for pair in pairs {
        seen[usize::from(pair.x >= xc) + 2 * usize::from(pair.y >= yc)] = true;
    }
    seen.iter().filter(|&&s| s).count()
}
