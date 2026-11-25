use crate::core::registration::poly::TanParams;

use crate::core::astrometry::local_solve::types::{Detection, LocalSolveParams};
use crate::core::astrometry::local_solve::tan_projection::pixel_to_sky;

pub(in crate::core::astrometry::local_solve) fn median_fwhm(detections: &[Detection]) -> f64 {
    let mut values: Vec<f64> = detections
        .iter()
        .map(|d| d.star.fwhm)
        .filter(|f| f.is_finite() && *f > 0.0)
        .collect();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

pub(in crate::core::astrometry::local_solve) fn match_radius(detections: &[Detection], params: &LocalSolveParams) -> f64 {
    params
        .match_radius_px
        .max(params.match_radius_fwhm * median_fwhm(detections))
}

pub(in crate::core::astrometry::local_solve) fn inside((width, height): (usize, usize), (x, y): (f64, f64)) -> bool {
    x >= -0.5 && y >= -0.5 && x <= width as f64 - 0.5 && y <= height as f64 - 0.5
}

pub(in crate::core::astrometry::local_solve) fn nearest_pairs(
    projected: &[(f64, f64)],
    detections: &[Detection],
    radius: f64,
) -> Vec<(usize, usize, f64)> {
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for (i, &(px, py)) in projected.iter().enumerate() {
        for (j, d) in detections.iter().enumerate() {
            let distance = (px - d.star.x).hypot(py - d.star.y);
            if distance <= radius {
                candidates.push((distance, i, j));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut used_projected = vec![false; projected.len()];
    let mut used_detection = vec![false; detections.len()];
    let mut pairs = Vec::new();
    for (distance, i, j) in candidates {
        if used_projected[i] || used_detection[j] {
            continue;
        }
        used_projected[i] = true;
        used_detection[j] = true;
        pairs.push((i, j, distance));
    }
    pairs
}

pub(in crate::core::astrometry::local_solve) fn corners_finite(tan: &TanParams, (width, height): (usize, usize)) -> bool {
    let right = width as f64 - 0.5;
    let top = height as f64 - 0.5;
    [(-0.5, -0.5), (right, -0.5), (right, top), (-0.5, top)]
        .iter()
        .all(|&(x, y)| {
            let (ra, dec) = pixel_to_sky(tan, x, y);
            ra.is_finite() && dec.is_finite()
        })
}
