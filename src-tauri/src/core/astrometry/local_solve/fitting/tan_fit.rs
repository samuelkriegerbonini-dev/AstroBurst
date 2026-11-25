use crate::core::alignment::triangles::{solve_3x3_ls, StarPair};
use crate::core::registration::poly::TanParams;

use crate::core::astrometry::local_solve::constants::{
    MIN_TAN_PAIRS, CLIP_FLOOR_PX, MAX_CLIP_ROUNDS, CRVAL_CONVERGED_DEG, MAX_CRVAL_ITERATIONS,
    RADIAL_RMS_PER_MEDIAN,
};
use crate::core::astrometry::local_solve::types::{SkyPair, TanFit, LocalSolveParams};
use super::tan_projection::{centre_crpix, tan_plane, tan_sky, cd_inverse};

struct LinearFit {
    m: [[f64; 2]; 2],
    c: (f64, f64),
}

impl LinearFit {
    fn predict(&self, plane: (f64, f64)) -> (f64, f64) {
        (
            self.m[0][0] * plane.0 + self.m[0][1] * plane.1 + self.c.0,
            self.m[1][0] * plane.0 + self.m[1][1] * plane.1 + self.c.1,
        )
    }
}

fn pixel_offset(pair: &SkyPair, crpix: (f64, f64)) -> (f64, f64) {
    (pair.x + 1.0 - crpix.0, pair.y + 1.0 - crpix.1)
}

fn linear_fit(
    pairs: &[SkyPair],
    mask: &[bool],
    crpix: (f64, f64),
    crval: (f64, f64),
) -> Option<LinearFit> {
    let rows: Vec<StarPair> = pairs
        .iter()
        .zip(mask)
        .filter(|(_, &kept)| kept)
        .filter_map(|(pair, _)| {
            let (plane_x, plane_y) = tan_plane(crval, pair.ra, pair.dec)?;
            let (dx, dy) = pixel_offset(pair, crpix);
            Some((plane_x, plane_y, dx, dy))
        })
        .collect();
    if rows.len() < MIN_TAN_PAIRS {
        return None;
    }
    let ((m11, m12), cx) = solve_3x3_ls(&rows, true)?;
    let ((m21, m22), cy) = solve_3x3_ls(&rows, false)?;
    Some(LinearFit {
        m: [[m11, m12], [m21, m22]],
        c: (cx, cy),
    })
}

fn tan_pass(
    pairs: &[SkyPair],
    mask: &[bool],
    crpix: (f64, f64),
    start: (f64, f64),
    iterations: usize,
) -> Option<(TanParams, Vec<f64>)> {
    let mut crval = start;
    let mut fit = linear_fit(pairs, mask, crpix, crval)?;
    for iteration in 0..MAX_CRVAL_ITERATIONS {
        let cd = cd_inverse(&fit.m)?;
        let centre_x = -(cd[0][0] * fit.c.0 + cd[0][1] * fit.c.1);
        let centre_y = -(cd[1][0] * fit.c.0 + cd[1][1] * fit.c.1);
        let shift = centre_x.hypot(centre_y);
        if iteration >= iterations && shift < CRVAL_CONVERGED_DEG {
            break;
        }
        crval = tan_sky(crval, centre_x, centre_y);
        fit = linear_fit(pairs, mask, crpix, crval)?;
    }
    let cd = cd_inverse(&fit.m)?;
    let residuals = pairs
        .iter()
        .map(|pair| match tan_plane(crval, pair.ra, pair.dec) {
            Some(plane) => {
                let (px, py) = fit.predict(plane);
                let (dx, dy) = pixel_offset(pair, crpix);
                (px - dx).hypot(py - dy)
            }
            None => f64::INFINITY,
        })
        .collect();
    Some((TanParams { crval, crpix, cd }, residuals))
}

fn median_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

pub(in crate::core::astrometry::local_solve) fn plain_rms(distances: &[f64]) -> f64 {
    if distances.is_empty() {
        0.0
    } else {
        (distances.iter().map(|d| d * d).sum::<f64>() / distances.len() as f64).sqrt()
    }
}

pub(in crate::core::astrometry::local_solve) fn clipped_rms(distances: &[f64], clip_sigma: f64) -> f64 {
    if distances.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = distances
        .iter()
        .copied()
        .filter(|d| d.is_finite())
        .collect();
    if sorted.is_empty() {
        return f64::INFINITY;
    }
    sorted.sort_by(f64::total_cmp);
    let mut threshold =
        (clip_sigma * RADIAL_RMS_PER_MEDIAN * median_sorted(&sorted)).max(CLIP_FLOOR_PX);
    let mut rms = plain_rms(&sorted);
    for _ in 0..MAX_CLIP_ROUNDS {
        let kept: Vec<f64> = sorted
            .iter()
            .copied()
            .take_while(|d| *d <= threshold)
            .collect();
        if kept.is_empty() {
            break;
        }
        rms = plain_rms(&kept);
        let next = (clip_sigma * rms).max(CLIP_FLOOR_PX);
        if next >= threshold {
            break;
        }
        threshold = next;
    }
    rms
}

fn masked_rms(residuals: &[f64], mask: &[bool]) -> f64 {
    let (sum, count) = residuals
        .iter()
        .zip(mask)
        .filter(|(_, &kept)| kept)
        .fold((0.0, 0usize), |(sum, count), (r, _)| {
            (sum + r * r, count + 1)
        });
    if count == 0 {
        0.0
    } else {
        (sum / count as f64).sqrt()
    }
}

pub(in crate::core::astrometry::local_solve) fn fit_tan(
    pairs: &[SkyPair],
    size: (usize, usize),
    start: (f64, f64),
    params: &LocalSolveParams,
) -> Option<TanFit> {
    let finite = pairs
        .iter()
        .all(|p| [p.x, p.y, p.ra, p.dec].iter().all(|v| v.is_finite()));
    if pairs.len() < MIN_TAN_PAIRS || !finite || !start.0.is_finite() || !start.1.is_finite() {
        return None;
    }
    let crpix = centre_crpix(size);
    let mut mask = vec![true; pairs.len()];
    let mut crval = start;
    for round in 0..MAX_CLIP_ROUNDS {
        let (tan, residuals) = tan_pass(pairs, &mask, crpix, crval, params.crval_iterations)?;
        crval = tan.crval;
        let rms = masked_rms(&residuals, &mask);
        let threshold = (params.clip_sigma * rms).max(CLIP_FLOOR_PX);
        let next: Vec<bool> = residuals.iter().map(|r| *r <= threshold).collect();
        let kept = next.iter().filter(|&&k| k).count();
        if next == mask || kept < MIN_TAN_PAIRS || round + 1 == MAX_CLIP_ROUNDS {
            return Some(TanFit {
                tan,
                rms_px: rms,
                inliers: mask,
            });
        }
        mask = next;
    }
    None
}
