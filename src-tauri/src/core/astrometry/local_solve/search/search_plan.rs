use crate::core::astrometry::solve_hints::SCALE_HINT_MARGIN;

use crate::core::astrometry::local_solve::constants::SCALE_WINDOW_TOLERANCE;
use crate::core::astrometry::local_solve::tan_projection::{destination, is_positive};

fn ring_step_deg(spacing_deg: f64) -> f64 {
    spacing_deg * 3f64.sqrt() / 2.0
}

pub(in crate::core::astrometry::local_solve) fn last_ring(spacing_deg: f64, radius_deg: f64) -> usize {
    if !is_positive(spacing_deg) || !is_positive(radius_deg) {
        return 0;
    }
    (radius_deg / ring_step_deg(spacing_deg)).ceil() as usize
}

fn spiral_ring(hint: (f64, f64), spacing_deg: f64, k: usize) -> Vec<(f64, f64)> {
    let mut ring = Vec::with_capacity(6 * k);
    for corner in 0..6 {
        let (sin0, cos0) = (corner as f64 * 60.0f64).to_radians().sin_cos();
        let (sin1, cos1) = ((corner + 1) as f64 * 60.0f64).to_radians().sin_cos();
        let reach = k as f64 * spacing_deg;
        let (x0, y0) = (reach * cos0, reach * sin0);
        let (x1, y1) = (reach * cos1, reach * sin1);
        for step in 0..k {
            let t = step as f64 / k as f64;
            let (x, y) = (x0 + t * (x1 - x0), y0 + t * (y1 - y0));
            let bearing = x.atan2(y).to_degrees();
            ring.push(destination(hint, bearing, x.hypot(y)));
        }
    }
    ring
}

#[cfg(test)]
pub(in crate::core::astrometry::local_solve) fn spiral_centres(hint: (f64, f64), spacing_deg: f64, radius_deg: f64) -> Vec<Vec<(f64, f64)>> {
    let mut rings = vec![vec![hint]];
    for k in 1..=last_ring(spacing_deg, radius_deg) {
        rings.push(spiral_ring(hint, spacing_deg, k));
    }
    rings
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::core::astrometry::local_solve) struct SearchPlan {
    pub(in crate::core::astrometry::local_solve) rings: Vec<Vec<(usize, (f64, f64))>>,
    pub(in crate::core::astrometry::local_solve) centres: usize,
    pub(in crate::core::astrometry::local_solve) radius_reached_deg: f64,
    pub(in crate::core::astrometry::local_solve) capped: bool,
}

pub(in crate::core::astrometry::local_solve) fn search_plan(
    hint: (f64, f64),
    spacings_deg: &[f64],
    radius_deg: f64,
    max_centres: usize,
) -> SearchPlan {
    let first: Vec<(usize, (f64, f64))> = (0..spacings_deg.len()).map(|h| (h, hint)).collect();
    let mut centres = first.len();
    let mut rings = vec![first];
    let mut reached = vec![0.0f64; spacings_deg.len()];
    let mut capped = false;
    for k in 1.. {
        let mut ring = Vec::new();
        for (h, &spacing) in spacings_deg.iter().enumerate() {
            if k <= last_ring(spacing, radius_deg) {
                ring.extend(spiral_ring(hint, spacing, k).into_iter().map(|c| (h, c)));
            }
        }
        if ring.is_empty() {
            break;
        }
        if centres + ring.len() > max_centres {
            capped = true;
            break;
        }
        centres += ring.len();
        for &(h, _) in &ring {
            reached[h] = radius_deg.min(k as f64 * ring_step_deg(spacings_deg[h]));
        }
        rings.push(ring);
    }
    let radius_reached_deg = if capped {
        reached.iter().copied().fold(f64::INFINITY, f64::min)
    } else {
        radius_deg.max(0.0)
    };
    SearchPlan {
        rings,
        centres,
        radius_reached_deg,
        capped,
    }
}

pub(in crate::core::astrometry::local_solve) fn scale_hypotheses(low_arcsec: f64, high_arcsec: f64) -> Vec<f64> {
    let (below, above) = (1.0 - SCALE_HINT_MARGIN, 1.0 + SCALE_HINT_MARGIN);
    let window = above / below;
    let single = high_arcsec / low_arcsec <= window * (1.0 + SCALE_WINDOW_TOLERANCE);
    if !is_positive(low_arcsec) || !is_positive(high_arcsec) || single {
        return vec![(low_arcsec * high_arcsec / (below * above)).sqrt()];
    }
    let first = low_arcsec / below;
    let last = high_arcsec / above;
    let count = ((high_arcsec / low_arcsec).ln() / window.ln())
        .ceil()
        .max(2.0) as usize;
    (0..count)
        .map(|i| first * (last / first).powf(i as f64 / (count - 1) as f64))
        .collect()
}
