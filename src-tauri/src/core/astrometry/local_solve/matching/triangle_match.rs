use crate::core::alignment::affine::{AffineAlignMethod, AffineTransform};
use crate::core::alignment::triangles::{
    build_triangles, match_triangles, ransac_affine, StarPair, TriangleDesc, TriangleMatch,
    TriangleParams,
};

use crate::core::astrometry::local_solve::constants::{
    MIN_TAN_PAIRS, RANSAC_INLIER_FIELD_FACTOR, MATCH_WINDOWS, MATCH_WINDOW_FRACTION,
};

pub(in crate::core::astrometry::local_solve) struct MatchSide {
    pub(in crate::core::astrometry::local_solve) stars: Vec<(f64, f64)>,
    tris: Vec<TriangleDesc>,
    mirrored_stars: Vec<(f64, f64)>,
    mirrored_tris: Vec<TriangleDesc>,
    pub(in crate::core::astrometry::local_solve) params: TriangleParams,
}

impl MatchSide {
    pub(in crate::core::astrometry::local_solve) fn new(points: &[(f64, f64)], match_stars: usize, inlier_px: f64) -> Self {
        let params = TriangleParams {
            max_stars: match_stars.max(MIN_TAN_PAIRS),
            inlier_px,
            max_residual_px: inlier_px.max(TriangleParams::SOLVER.max_residual_px),
            ..TriangleParams::SOLVER
        };
        let stars = points[..points.len().min(params.max_stars)].to_vec();
        let mirrored_stars: Vec<(f64, f64)> = stars.iter().map(|&(x, y)| (-x, y)).collect();
        Self {
            tris: build_triangles(&stars, &params),
            mirrored_tris: build_triangles(&mirrored_stars, &params),
            stars,
            mirrored_stars,
            params,
        }
    }

    pub(in crate::core::astrometry::local_solve) fn match_catalogue(&self, catalogue: &[(f64, f64)]) -> (TriangleMatch, TriangleMatch) {
        let cat_tris = build_triangles(catalogue, &self.params);
        let same = match_triangles(catalogue, &self.stars, &cat_tris, &self.tris, &self.params);
        let mirrored = match_triangles(
            catalogue,
            &self.mirrored_stars,
            &cat_tris,
            &self.mirrored_tris,
            &self.params,
        );
        let unmirrored = TriangleMatch {
            pairs: mirrored
                .pairs
                .into_iter()
                .map(|(rx, ry, tx, ty)| (rx, ry, -tx, ty))
                .collect(),
            votes: mirrored.votes,
            flipped: true,
        };
        (same, unmirrored)
    }
}

fn window_inliers(side: &MatchSide, catalogue: &[(f64, f64)]) -> Option<Vec<StarPair>> {
    let (same, mirrored) = side.match_catalogue(catalogue);
    let [same_inliers, mirrored_inliers] = [&same, &mirrored].map(|m| {
        ransac_affine(&m.pairs, AffineAlignMethod::Affine, &side.params)
            .map(|fit| inlier_pairs(&m.pairs, &fit.transform, side.params.inlier_px))
    });
    match (same_inliers, mirrored_inliers) {
        (Some(a), Some(b)) if b.len() > a.len() => Some(b),
        (Some(a), _) => Some(a),
        (None, b) => b,
    }
}

pub(in crate::core::astrometry::local_solve) fn best_window_inliers(side: &MatchSide, catalogue: &[(f64, f64)]) -> Option<Vec<StarPair>> {
    let window = ((side.params.max_stars as f64 * MATCH_WINDOW_FRACTION).round() as usize)
        .max(side.params.min_inliers);
    let step = (window / 2).max(1);
    let mut best: Option<Vec<StarPair>> = None;
    for index in 0..MATCH_WINDOWS {
        let start = index * step;
        if start + side.params.min_inliers > catalogue.len() {
            break;
        }
        let end = (start + window).min(catalogue.len());
        if let Some(inliers) = window_inliers(side, &catalogue[start..end]) {
            if best.as_ref().is_none_or(|b| inliers.len() > b.len()) {
                best = Some(inliers);
            }
        }
    }
    best.filter(|inliers| inliers.len() >= side.params.min_inliers)
}

pub(in crate::core::astrometry::local_solve) fn ransac_inlier_px(long_side_px: usize, long_side_deg: f64) -> f64 {
    let long_side_rad = long_side_deg.to_radians();
    TriangleParams::SOLVER
        .inlier_px
        .max(RANSAC_INLIER_FIELD_FACTOR * long_side_px as f64 * long_side_rad * long_side_rad)
}

fn inlier_pairs(
    matched: &[StarPair],
    transform: &AffineTransform,
    inlier_px: f64,
) -> Vec<StarPair> {
    matched
        .iter()
        .copied()
        .filter(|&(rx, ry, tx, ty)| {
            let (px, py) = transform.map(rx, ry);
            (px - tx).hypot(py - ty) < inlier_px
        })
        .collect()
}
