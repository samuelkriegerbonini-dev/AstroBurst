use std::cmp::Ordering;
use std::collections::HashMap;

use rayon::prelude::*;

use crate::core::alignment::affine::{AffineAlignMethod, AffineAlignResult, AffineTransform};
use crate::core::analysis::star_detection::DetectedStar;

pub(crate) type StarPair = (f64, f64, f64, f64);

const SELECTION_GRID: usize = 4;
const SINGULAR_RELATIVE_DET: f64 = 1e-12;
const RANSAC_SEED: u64 = 0xDEAD_BEEF_CAFE_BABE;
const RANSAC_CHUNK_STRIDE: u64 = 0x9E37_79B9_7F4A_7C15;
const RANSAC_SAMPLE_ATTEMPTS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Parity {
    Same,
    Either,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TriangleParams {
    pub(crate) max_stars: usize,
    pub(crate) tolerance: f64,
    pub(crate) tie_margin: f64,
    pub(crate) min_side_px: f64,
    pub(crate) min_votes: u32,
    pub(crate) ransac_iterations: usize,
    pub(crate) ransac_chunks: usize,
    pub(crate) inlier_px: f64,
    pub(crate) min_inliers: usize,
    pub(crate) min_inlier_ratio: f64,
    pub(crate) max_residual_px: f64,
    pub(crate) wide_rotation_above_deg: f64,
    pub(crate) wide_rotation_min_inliers: usize,
    pub(crate) wide_rotation_min_inlier_ratio: f64,
    pub(crate) wide_rotation_max_rms_px: f64,
    pub(crate) parity: Parity,
}

impl TriangleParams {
    pub(crate) const ALIGNMENT: TriangleParams = TriangleParams {
        max_stars: 60,
        tolerance: 0.02,
        tie_margin: 0.03,
        min_side_px: 15.0,
        min_votes: 1,
        ransac_iterations: 2000,
        ransac_chunks: 16,
        inlier_px: 3.0,
        min_inliers: 4,
        min_inlier_ratio: 0.20,
        max_residual_px: 5.0,
        wide_rotation_above_deg: 30.0,
        wide_rotation_min_inliers: 8,
        wide_rotation_min_inlier_ratio: 0.25,
        wide_rotation_max_rms_px: 1.0,
        parity: Parity::Either,
    };

    pub(crate) fn wide_rotation_inliers_needed(&self, matched: usize) -> usize {
        let by_ratio = (self.wide_rotation_min_inlier_ratio * matched as f64).ceil() as usize;
        self.wide_rotation_min_inliers.max(by_ratio)
    }

    pub(crate) fn is_wide_rotation_fit(&self, matched: usize, inliers: usize, rms_px: f64) -> bool {
        inliers >= self.wide_rotation_inliers_needed(matched)
            && rms_px <= self.wide_rotation_max_rms_px
    }
}

pub(crate) struct TriangleDesc {
    sorted_vertices: [usize; 3],
    ratio_mid: f64,
    ratio_long: f64,
    orientation: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TriangleMatch {
    pub(crate) pairs: Vec<StarPair>,
    pub(crate) votes: u64,
    pub(crate) flipped: bool,
}

pub(crate) fn select_alignment_stars(
    stars: &[DetectedStar],
    n: usize,
    rows: usize,
    cols: usize,
) -> Vec<(f64, f64)> {
    if stars.len() <= n {
        return stars.iter().map(|s| (s.x, s.y)).collect();
    }
    let cell_w = (cols as f64 / SELECTION_GRID as f64).max(1.0);
    let cell_h = (rows as f64 / SELECTION_GRID as f64).max(1.0);
    let per_cell = (n / (SELECTION_GRID * SELECTION_GRID)).max(1);
    let mut cell_counts = [0usize; SELECTION_GRID * SELECTION_GRID];
    let mut selected: Vec<usize> = Vec::with_capacity(n);
    let mut skipped: Vec<usize> = Vec::new();
    for (idx, s) in stars.iter().enumerate() {
        let gx = ((s.x / cell_w) as usize).min(SELECTION_GRID - 1);
        let gy = ((s.y / cell_h) as usize).min(SELECTION_GRID - 1);
        let cell = gy * SELECTION_GRID + gx;
        if cell_counts[cell] < per_cell && selected.len() < n {
            cell_counts[cell] += 1;
            selected.push(idx);
        } else {
            skipped.push(idx);
        }
    }
    for idx in skipped {
        if selected.len() >= n {
            break;
        }
        selected.push(idx);
    }
    selected.sort_unstable();
    selected
        .into_iter()
        .filter_map(|i| stars.get(i).map(|s| (s.x, s.y)))
        .collect()
}

pub(crate) fn build_triangles(stars: &[(f64, f64)], params: &TriangleParams) -> Vec<TriangleDesc> {
    let n = stars.len();
    if n < 3 {
        return Vec::new();
    }
    let limit = n.min(params.max_stars);

    (0..limit)
        .into_par_iter()
        .flat_map(|i| {
            let mut local = Vec::new();
            for j in (i + 1)..limit {
                for k in (j + 1)..limit {
                    let mut sides = [
                        dist(stars[i], stars[j]),
                        dist(stars[j], stars[k]),
                        dist(stars[i], stars[k]),
                    ];
                    sides.sort_by(|a, b| a.total_cmp(b));

                    if sides[0] < params.min_side_px {
                        continue;
                    }

                    let ratio_mid = sides[1] / sides[0];
                    let ratio_long = sides[2] / sides[0];

                    if (ratio_mid - 1.0).abs() < params.tie_margin
                        || (ratio_long - ratio_mid).abs() < params.tie_margin
                    {
                        continue;
                    }

                    let sorted = sort_triangle_vertices(stars, &[i, j, k]);
                    let (ax, ay) = stars[sorted[0]];
                    let (bx, by) = stars[sorted[1]];
                    let (cx, cy) = stars[sorted[2]];
                    let orientation = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax) >= 0.0;

                    local.push(TriangleDesc {
                        sorted_vertices: sorted,
                        ratio_mid,
                        ratio_long,
                        orientation,
                    });
                }
            }
            local
        })
        .collect()
}

fn ratio_bucket(triangle: &TriangleDesc, tolerance: f64) -> (i64, i64) {
    (
        (triangle.ratio_mid / tolerance).floor() as i64,
        (triangle.ratio_long / tolerance).floor() as i64,
    )
}

pub(crate) fn match_triangles(
    ref_stars: &[(f64, f64)],
    tgt_stars: &[(f64, f64)],
    ref_tris: &[TriangleDesc],
    tgt_tris: &[TriangleDesc],
    params: &TriangleParams,
) -> TriangleMatch {
    let tolerance = params.tolerance;
    let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (idx, tt) in tgt_tris.iter().enumerate() {
        buckets
            .entry(ratio_bucket(tt, tolerance))
            .or_default()
            .push(idx);
    }

    let local_votes: Vec<HashMap<(usize, usize), u32>> = ref_tris
        .par_iter()
        .map(|rt| {
            let mut votes = HashMap::new();
            let (km, kl) = ratio_bucket(rt, tolerance);
            for dm in -1..=1 {
                for dl in -1..=1 {
                    let Some(indices) = buckets.get(&(km + dm, kl + dl)) else {
                        continue;
                    };
                    for tt in indices.iter().filter_map(|&idx| tgt_tris.get(idx)) {
                        if rt.orientation != tt.orientation {
                            continue;
                        }
                        let d_mid = (rt.ratio_mid - tt.ratio_mid).abs();
                        let d_long = (rt.ratio_long - tt.ratio_long).abs();

                        if d_mid > tolerance || d_long > tolerance {
                            continue;
                        }

                        for (&ri, &ti) in rt.sorted_vertices.iter().zip(&tt.sorted_vertices) {
                            *votes.entry((ri, ti)).or_insert(0) += 1;
                        }
                    }
                }
            }
            votes
        })
        .collect();

    let mut vote_map: HashMap<(usize, usize), u32> = HashMap::new();
    for local in local_votes {
        for (k, v) in local {
            *vote_map.entry(k).or_insert(0) += v;
        }
    }

    let mut ranked: Vec<((usize, usize), u32)> = vote_map.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut used_ref = vec![false; ref_stars.len()];
    let mut used_tgt = vec![false; tgt_stars.len()];
    let mut pairs = Vec::new();
    let mut total_votes = 0u64;

    for ((ri, ti), votes) in ranked {
        if votes < params.min_votes {
            break;
        }
        let (Some(&(rx, ry)), Some(&(tx, ty))) = (ref_stars.get(ri), tgt_stars.get(ti)) else {
            continue;
        };
        if used_ref[ri] || used_tgt[ti] {
            continue;
        }
        used_ref[ri] = true;
        used_tgt[ti] = true;
        pairs.push((rx, ry, tx, ty));
        total_votes += u64::from(votes);
    }

    TriangleMatch {
        pairs,
        votes: total_votes,
        flipped: false,
    }
}

pub(crate) fn match_star_lists(
    ref_stars: &[(f64, f64)],
    tgt_stars: &[(f64, f64)],
    params: &TriangleParams,
) -> TriangleMatch {
    let ref_tris = build_triangles(ref_stars, params);
    let tgt_tris = build_triangles(tgt_stars, params);
    let same = match_triangles(ref_stars, tgt_stars, &ref_tris, &tgt_tris, params);
    if params.parity == Parity::Same {
        return same;
    }
    let mirrored = match_mirrored(ref_stars, tgt_stars, &ref_tris, params);
    if mirrored.votes <= same.votes {
        return same;
    }
    mirrored
}

fn match_mirrored(
    ref_stars: &[(f64, f64)],
    tgt_stars: &[(f64, f64)],
    ref_tris: &[TriangleDesc],
    params: &TriangleParams,
) -> TriangleMatch {
    let mirrored_stars: Vec<(f64, f64)> = tgt_stars.iter().map(|&(x, y)| (-x, y)).collect();
    let mirrored_tris = build_triangles(&mirrored_stars, params);
    let mirrored = match_triangles(ref_stars, &mirrored_stars, ref_tris, &mirrored_tris, params);
    TriangleMatch {
        pairs: mirrored
            .pairs
            .into_iter()
            .map(|(rx, ry, tx, ty)| (rx, ry, -tx, ty))
            .collect(),
        votes: mirrored.votes,
        flipped: true,
    }
}

fn sort_triangle_vertices(stars: &[(f64, f64)], indices: &[usize; 3]) -> [usize; 3] {
    let [i, j, k] = *indices;
    let mut verts = [
        (i, dist(stars[j], stars[k])),
        (j, dist(stars[i], stars[k])),
        (k, dist(stars[i], stars[j])),
    ];
    verts.sort_by(|a, b| a.1.total_cmp(&b.1));
    [verts[0].0, verts[1].0, verts[2].0]
}

pub(crate) fn ransac_affine(
    matches: &[StarPair],
    method: AffineAlignMethod,
    params: &TriangleParams,
) -> Option<AffineAlignResult> {
    let n = matches.len();
    let min_sample = if method == AffineAlignMethod::Affine {
        3
    } else {
        2
    };
    if n < min_sample {
        return None;
    }

    let chunks = params.ransac_chunks.max(1);
    let chunk_size = params.ransac_iterations.div_ceil(chunks);

    let chunk_results: Vec<(usize, AffineTransform, Vec<bool>)> = (0..chunks)
        .into_par_iter()
        .map(|chunk| {
            let mut rng_state =
                RANSAC_SEED.wrapping_add((chunk as u64).wrapping_mul(RANSAC_CHUNK_STRIDE));
            let inline_rand = |state: &mut u64| -> usize {
                *state ^= *state << 13;
                *state ^= *state >> 7;
                *state ^= *state << 17;
                (*state as usize) % n
            };

            let mut local_best_inliers = 0usize;
            let mut local_best_transform = AffineTransform::identity();
            let mut local_best_mask = vec![false; n];
            let mut mask = vec![false; n];

            for _ in 0..chunk_size {
                let mut sample = Vec::with_capacity(min_sample);
                let mut attempts = 0;
                while sample.len() < min_sample && attempts < RANSAC_SAMPLE_ATTEMPTS {
                    let idx = inline_rand(&mut rng_state);
                    if !sample.contains(&idx) {
                        sample.push(idx);
                    }
                    attempts += 1;
                }
                if sample.len() < min_sample {
                    continue;
                }

                let sample_matches: Vec<StarPair> = sample
                    .iter()
                    .filter_map(|&i| matches.get(i).copied())
                    .collect();

                let transform = match method {
                    AffineAlignMethod::Affine => fit_affine(&sample_matches),
                    _ => fit_rigid(&sample_matches),
                };
                let Some(transform) = transform else {
                    continue;
                };

                mask.fill(false);
                let mut inlier_count = 0;
                for (slot, &(rx, ry, tx, ty)) in mask.iter_mut().zip(matches) {
                    let (px, py) = transform.map(rx, ry);
                    let err = ((px - tx).powi(2) + (py - ty).powi(2)).sqrt();
                    if err < params.inlier_px {
                        inlier_count += 1;
                        *slot = true;
                    }
                }

                if inlier_count > local_best_inliers {
                    local_best_inliers = inlier_count;
                    local_best_transform = transform;
                    local_best_mask.copy_from_slice(&mask);
                }
            }

            (local_best_inliers, local_best_transform, local_best_mask)
        })
        .collect();

    let (best_inliers, best_transform, best_inlier_mask) = chunk_results
        .into_iter()
        .reduce(|a, b| if b.0 > a.0 { b } else { a })
        .unwrap_or((0, AffineTransform::identity(), vec![false; n]));

    if best_inliers < params.min_inliers {
        log::debug!(
            "RANSAC: best_inliers {} < min_inliers {}",
            best_inliers,
            params.min_inliers
        );
        return None;
    }

    let inlier_ratio = best_inliers as f64 / n as f64;
    if inlier_ratio < params.min_inlier_ratio {
        log::debug!(
            "RANSAC: inlier_ratio {:.3} < min_inlier_ratio {:.2}",
            inlier_ratio,
            params.min_inlier_ratio
        );
        return None;
    }

    let inlier_matches: Vec<StarPair> = matches
        .iter()
        .zip(&best_inlier_mask)
        .filter(|(_, &keep)| keep)
        .map(|(&pair, _)| pair)
        .collect();

    let refined = match method {
        AffineAlignMethod::Affine => fit_affine(&inlier_matches),
        _ => fit_rigid(&inlier_matches),
    }
    .unwrap_or(best_transform);

    let residual = compute_residual(&inlier_matches, &refined);
    if residual > params.max_residual_px {
        log::debug!(
            "RANSAC: refined residual {:.3}px > max_residual_px {:.1}",
            residual,
            params.max_residual_px
        );
        return None;
    }

    Some(AffineAlignResult {
        transform: refined,
        matched_stars: n,
        inliers: best_inliers,
        residual_px: residual,
        rms_px: compute_rms(&inlier_matches, &refined),
        method,
        confidence: 1.0,
        rejection: None,
    })
}

pub(crate) fn fit_affine(matches: &[StarPair]) -> Option<AffineTransform> {
    if matches.len() < 3 {
        return None;
    }

    let (ab, tx) = solve_3x3_ls(matches, true)?;
    let (cd, ty) = solve_3x3_ls(matches, false)?;

    Some(AffineTransform {
        a: ab.0,
        b: ab.1,
        tx,
        c: cd.0,
        d: cd.1,
        ty,
    })
}

pub(crate) fn solve_3x3_ls(matches: &[StarPair], solve_x: bool) -> Option<((f64, f64), f64)> {
    let count = matches.len() as f64;
    let (sum_x, sum_y) = matches
        .iter()
        .fold((0.0, 0.0), |(sx, sy), &(rx, ry, _, _)| (sx + rx, sy + ry));
    let (mean_x, mean_y) = (sum_x / count, sum_y / count);
    let mut ata = [[0.0f64; 3]; 3];
    let mut atb = [0.0f64; 3];

    for &(rx, ry, tx, ty) in matches {
        let target = if solve_x { tx } else { ty };
        let row = [rx - mean_x, ry - mean_y, 1.0];

        for i in 0..3 {
            for j in 0..3 {
                ata[i][j] += row[i] * row[j];
            }
            atb[i] += row[i] * target;
        }
    }

    let x = solve_3x3(ata, atb)?;
    Some(((x[0], x[1]), x[2] - x[0] * mean_x - x[1] * mean_y))
}

pub(crate) fn solve_3x3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);

    let hadamard_bound: f64 = a
        .iter()
        .map(|row| row.iter().map(|v| v * v).sum::<f64>().sqrt())
        .product();
    let singular_below = SINGULAR_RELATIVE_DET * hadamard_bound;
    if !det.is_finite() || det.abs().partial_cmp(&singular_below) != Some(Ordering::Greater) {
        return None;
    }

    let inv_det = 1.0 / det;

    let inv = [
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) * inv_det,
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * inv_det,
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * inv_det,
        ],
        [
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) * inv_det,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * inv_det,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * inv_det,
        ],
        [
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) * inv_det,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * inv_det,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * inv_det,
        ],
    ];

    Some([
        inv[0][0] * b[0] + inv[0][1] * b[1] + inv[0][2] * b[2],
        inv[1][0] * b[0] + inv[1][1] * b[1] + inv[1][2] * b[2],
        inv[2][0] * b[0] + inv[2][1] * b[1] + inv[2][2] * b[2],
    ])
}

pub(crate) fn fit_rigid(matches: &[StarPair]) -> Option<AffineTransform> {
    let n = matches.len();
    if n < 2 {
        return None;
    }

    let (mut rcx, mut rcy, mut tcx, mut tcy) = (0.0, 0.0, 0.0, 0.0);
    for &(rx, ry, tx, ty) in matches {
        rcx += rx;
        rcy += ry;
        tcx += tx;
        tcy += ty;
    }
    let nf = n as f64;
    rcx /= nf;
    rcy /= nf;
    tcx /= nf;
    tcy /= nf;

    let mut num = 0.0;
    let mut den = 0.0;
    for &(rx, ry, tx, ty) in matches {
        let drx = rx - rcx;
        let dry = ry - rcy;
        let dtx = tx - tcx;
        let dty = ty - tcy;
        num += drx * dty - dry * dtx;
        den += drx * dtx + dry * dty;
    }

    let theta = num.atan2(den);
    let cos_t = theta.cos();
    let sin_t = theta.sin();

    let tx = tcx - cos_t * rcx + sin_t * rcy;
    let ty = tcy - sin_t * rcx - cos_t * rcy;

    Some(AffineTransform {
        a: cos_t,
        b: -sin_t,
        tx,
        c: sin_t,
        d: cos_t,
        ty,
    })
}

pub(crate) fn compute_residual(matches: &[StarPair], transform: &AffineTransform) -> f64 {
    if matches.is_empty() {
        return 0.0;
    }
    let sum: f64 = matches
        .iter()
        .map(|&(rx, ry, tx, ty)| {
            let (px, py) = transform.map(rx, ry);
            ((px - tx).powi(2) + (py - ty).powi(2)).sqrt()
        })
        .sum();
    sum / matches.len() as f64
}

pub(crate) fn compute_rms(matches: &[StarPair], transform: &AffineTransform) -> f64 {
    if matches.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = matches
        .iter()
        .map(|&(rx, ry, tx, ty)| {
            let (px, py) = transform.map(rx, ry);
            (px - tx).powi(2) + (py - ty).powi(2)
        })
        .sum();
    (sum_sq / matches.len() as f64).sqrt()
}

#[inline]
pub(crate) fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

#[cfg(test)]
pub(crate) const A1_SIZE: usize = 2048;

#[cfg(test)]
pub(crate) const A1_STARS: usize = 300;

#[cfg(test)]
pub(crate) const A1_ROTATIONS_DEG: [f64; 5] = [0.0, 37.0, 90.0, 179.5, 180.0];

#[cfg(test)]
const A1_SHIFT: (f64, f64) = (12.25, -7.5);

#[cfg(test)]
const A1_MIN_SEPARATION_PX: f64 = 24.0;

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum FrameView {
    Rotated(f64),
    Mirrored,
}

#[cfg(test)]
fn fixture_unit(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
pub(crate) fn a1_truth(view: FrameView, size: usize) -> AffineTransform {
    let centre = (size as f64 - 1.0) * 0.5;
    match view {
        FrameView::Rotated(deg) => {
            let (sin, cos) = deg.to_radians().sin_cos();
            AffineTransform {
                a: cos,
                b: -sin,
                tx: centre - cos * centre + sin * centre + A1_SHIFT.0,
                c: sin,
                d: cos,
                ty: centre - sin * centre - cos * centre + A1_SHIFT.1,
            }
        }
        FrameView::Mirrored => AffineTransform {
            a: -1.0,
            b: 0.0,
            tx: 2.0 * centre + A1_SHIFT.0,
            c: 0.0,
            d: 1.0,
            ty: A1_SHIFT.1,
        },
    }
}

#[cfg(test)]
pub(crate) fn a1_sky(size: usize, count: usize) -> Vec<(f64, f64, f64)> {
    let centre = (size as f64 - 1.0) * 0.5;
    let radius = size as f64 * 0.45;
    let min_separation_sq = A1_MIN_SEPARATION_PX * A1_MIN_SEPARATION_PX;
    let mut state = 0xA1F1_E1D5_7A25_0D1Fu64;
    let mut sky: Vec<(f64, f64, f64)> = Vec::with_capacity(count);
    for _ in 0..count * 1000 {
        if sky.len() == count {
            break;
        }
        let x = centre + (fixture_unit(&mut state) * 2.0 - 1.0) * radius;
        let y = centre + (fixture_unit(&mut state) * 2.0 - 1.0) * radius;
        let flux = 2000.0 + fixture_unit(&mut state) * 18000.0;
        let inside = (x - centre).hypot(y - centre) <= radius;
        let isolated = sky
            .iter()
            .all(|&(sx, sy, _)| (sx - x).powi(2) + (sy - y).powi(2) >= min_separation_sq);
        if inside && isolated {
            sky.push((x, y, flux));
        }
    }
    sky
}

#[cfg(test)]
pub(crate) fn render_sky(
    sky: &[(f64, f64, f64)],
    size: usize,
    view: &AffineTransform,
) -> ndarray::Array2<f32> {
    use crate::core::synth::psf::{render_stars, GaussianPsf};
    use crate::core::synth::star_field::Star;
    let stars: Vec<Star> = sky
        .iter()
        .map(|&(x, y, flux)| {
            let (vx, vy) = view.map(x, y);
            Star {
                x: vx,
                y: vy,
                z: 0.0,
                flux,
                temperature: 5800.0,
            }
        })
        .collect();
    render_stars(
        &stars,
        &GaussianPsf::from_fwhm(3.0),
        size as u32,
        size as u32,
    )
}

#[cfg(test)]
pub(crate) fn a1_reference(size: usize, count: usize) -> ndarray::Array2<f32> {
    render_sky(&a1_sky(size, count), size, &AffineTransform::identity())
}

#[cfg(test)]
pub(crate) fn a1_frame(size: usize, count: usize, view: FrameView) -> ndarray::Array2<f32> {
    render_sky(&a1_sky(size, count), size, &a1_truth(view, size))
}

#[cfg(test)]
pub(crate) fn a1_jittered_frame(
    size: usize,
    count: usize,
    view: FrameView,
    jitter_sigma_px: f64,
) -> ndarray::Array2<f32> {
    let half_width = jitter_sigma_px * 3f64.sqrt();
    let mut state = 0x7E57_0FF3u64;
    let jittered: Vec<(f64, f64, f64)> = a1_sky(size, count)
        .into_iter()
        .map(|(x, y, flux)| {
            let dx = (fixture_unit(&mut state) * 2.0 - 1.0) * half_width;
            let dy = (fixture_unit(&mut state) * 2.0 - 1.0) * half_width;
            (x + dx, y + dy, flux)
        })
        .collect();
    render_sky(&jittered, size, &a1_truth(view, size))
}

#[cfg(test)]
pub(crate) fn registration_rms(
    found: &AffineTransform,
    truth: &AffineTransform,
    sky: &[(f64, f64, f64)],
) -> f64 {
    if sky.is_empty() {
        return f64::INFINITY;
    }
    let sum_sq: f64 = sky
        .iter()
        .map(|&(x, y, _)| {
            let (fx, fy) = found.map(x, y);
            let (tx, ty) = truth.map(x, y);
            (fx - tx).powi(2) + (fy - ty).powi(2)
        })
        .sum();
    (sum_sq / sky.len() as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Xorshift(u64);

    impl Xorshift {
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    const PARAMS: TriangleParams = TriangleParams::ALIGNMENT;

    #[test]
    fn test_fit_rigid_pure_translation() {
        let matches = vec![
            (0.0, 0.0, 2.0, 3.0),
            (10.0, 0.0, 12.0, 3.0),
            (0.0, 10.0, 2.0, 13.0),
            (10.0, 10.0, 12.0, 13.0),
        ];
        let t = fit_rigid(&matches).unwrap();
        assert!((t.tx - 2.0).abs() < 0.01, "tx={}", t.tx);
        assert!((t.ty - 3.0).abs() < 0.01, "ty={}", t.ty);
        assert!(t.rotation_deg().abs() < 0.01);
    }

    #[test]
    fn test_fit_rigid_rotation() {
        let angle = 2.0f64.to_radians();
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        let ref_pts = [
            (100.0, 100.0),
            (200.0, 100.0),
            (100.0, 200.0),
            (200.0, 200.0),
            (150.0, 150.0),
        ];

        let matches: Vec<StarPair> = ref_pts
            .iter()
            .map(|&(rx, ry)| {
                let tx = cos_a * rx - sin_a * ry;
                let ty = sin_a * rx + cos_a * ry;
                (rx, ry, tx, ty)
            })
            .collect();

        let t = fit_rigid(&matches).unwrap();
        assert!(
            (t.rotation_deg() - 2.0).abs() < 0.1,
            "rotation={:.3}",
            t.rotation_deg()
        );
    }

    #[test]
    fn test_fit_affine_translation() {
        let matches = vec![
            (0.0, 0.0, 5.0, -2.0),
            (100.0, 0.0, 105.0, -2.0),
            (0.0, 100.0, 5.0, 98.0),
            (100.0, 100.0, 105.0, 98.0),
        ];
        let t = fit_affine(&matches).unwrap();
        assert!((t.tx - 5.0).abs() < 0.01);
        assert!((t.ty - (-2.0)).abs() < 0.01);
        assert!((t.a - 1.0).abs() < 0.01);
        assert!((t.d - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_solve_3x3_identity() {
        let a = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let b = [3.0, 5.0, 7.0];
        let x = solve_3x3(a, b).unwrap();
        assert!((x[0] - 3.0).abs() < 1e-10);
        assert!((x[1] - 5.0).abs() < 1e-10);
        assert!((x[2] - 7.0).abs() < 1e-10);
    }

    #[test]
    fn test_triangle_matching_identical() {
        let stars = vec![
            (10.0, 10.0),
            (50.0, 10.0),
            (30.0, 40.0),
            (80.0, 20.0),
            (60.0, 70.0),
        ];
        let tris = build_triangles(&stars, &PARAMS);
        let matched = match_triangles(&stars, &stars, &tris, &tris, &PARAMS);
        assert!(
            matched.pairs.len() >= 4,
            "got {} matches",
            matched.pairs.len()
        );
    }

    #[test]
    fn fit_affine_rejects_collinear_samples_at_image_scale() {
        let mut rng = Xorshift(0x1234_5678_9ABC_DEF1);
        for _ in 0..500 {
            let (x0, y0) = (1000.0 + rng.unit() * 2000.0, 1000.0 + rng.unit() * 2000.0);
            let angle = rng.unit() * std::f64::consts::TAU;
            let (ux, uy) = (angle.cos(), angle.sin());
            let matches: Vec<StarPair> = [0.0, 317.37 + rng.unit(), 811.13 + rng.unit()]
                .iter()
                .map(|t| {
                    let (rx, ry) = (x0 + t * ux, y0 + t * uy);
                    (rx, ry, rx + 5.0, ry - 3.0)
                })
                .collect();
            assert!(
                fit_affine(&matches).is_none(),
                "collinear sample solved: {matches:?}"
            );
        }
        let triangle = [(1200.3, 1100.7), (2900.1, 1300.2), (1800.6, 2950.9)];
        let matches: Vec<StarPair> = triangle
            .iter()
            .map(|&(x, y)| (x, y, x + 5.0, y - 3.0))
            .collect();
        let t = fit_affine(&matches).expect("triangle solves");
        assert!(
            (t.tx - 5.0).abs() < 1e-6 && (t.ty + 3.0).abs() < 1e-6,
            "{t:?}"
        );
        assert!(
            (t.a - 1.0).abs() < 1e-9
                && t.b.abs() < 1e-9
                && t.c.abs() < 1e-9
                && (t.d - 1.0).abs() < 1e-9
        );
    }

    #[test]
    fn match_triangles_is_deterministic_across_calls() {
        let stars = vec![
            (12.0, 15.0),
            (97.0, 22.0),
            (41.0, 88.0),
            (150.0, 61.0),
            (73.0, 140.0),
            (180.0, 170.0),
            (25.0, 190.0),
            (130.0, 115.0),
        ];
        let tris = build_triangles(&stars, &PARAMS);
        let first = match_triangles(&stars, &stars, &tris, &tris, &PARAMS);
        assert!(first.pairs.len() >= 4);
        for _ in 0..24 {
            assert_eq!(
                match_triangles(&stars, &stars, &tris, &tris, &PARAMS),
                first
            );
        }
    }

    #[test]
    fn ransac_result_does_not_depend_on_thread_count() {
        let mut rng = Xorshift(0x0DDB_A11C_AFE0_F00D);
        let mut matches = Vec::new();
        for i in 0..40 {
            let (rx, ry) = (50.0 + rng.unit() * 900.0, 50.0 + rng.unit() * 900.0);
            let noise = if i % 5 == 0 { 40.0 } else { 6.0 };
            let tx = rx * 1.001 + 0.02 * ry + 7.0 + (rng.unit() - 0.5) * noise;
            let ty = -0.02 * rx + ry * 0.999 - 4.0 + (rng.unit() - 0.5) * noise;
            matches.push((rx, ry, tx, ty));
        }
        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| ransac_affine(&matches, AffineAlignMethod::Affine, &PARAMS))
                .map(|r| {
                    let t = r.transform;
                    (r.inliers, [t.a, t.b, t.tx, t.c, t.d, t.ty])
                })
        };
        let single = run(1);
        assert!(single.is_some());
        for threads in [2, 3, 4, 8] {
            let other = run(threads);
            assert_eq!(
                other.map(|(n, t)| (n, t.map(f64::to_bits))),
                single.map(|(n, t)| (n, t.map(f64::to_bits))),
                "{threads} threads"
            );
        }
    }
    fn scattered_stars(count: usize, seed: u64) -> Vec<(f64, f64)> {
        let mut rng = Xorshift(seed);
        (0..count)
            .map(|_| (40.0 + rng.unit() * 920.0, 40.0 + rng.unit() * 920.0))
            .collect()
    }

    fn correct_pairs(matched: &TriangleMatch, truth: impl Fn(f64, f64) -> (f64, f64)) -> usize {
        matched
            .pairs
            .iter()
            .filter(|&&(rx, ry, tx, ty)| {
                let (ex, ey) = truth(rx, ry);
                (ex - tx).abs() < 1e-9 && (ey - ty).abs() < 1e-9
            })
            .count()
    }

    #[test]
    fn mirrored_target_matches_only_with_either_parity() {
        let stars = scattered_stars(40, 0x5EED_0F_4057_A125);
        let mirror = |x: f64, y: f64| (999.0 - x, y);
        let mirrored: Vec<(f64, f64)> = stars.iter().map(|&(x, y)| mirror(x, y)).collect();

        let same = TriangleParams {
            parity: Parity::Same,
            ..PARAMS
        };
        let either = TriangleParams {
            parity: Parity::Either,
            ..PARAMS
        };
        let same_match = match_star_lists(&stars, &mirrored, &same);
        let either_match = match_star_lists(&stars, &mirrored, &either);
        let same_true = correct_pairs(&same_match, mirror);
        let either_true = correct_pairs(&either_match, mirror);

        assert!(!same_match.flipped);
        assert!(
            either_match.flipped,
            "the mirrored parity did not win: votes {}, {either_true} true pairs",
            either_match.votes
        );
        assert!(
            either_true >= 30,
            "either parity found {either_true} true pairs"
        );
        assert!(
            same_true * 2 < either_true,
            "same parity found {same_true} true pairs, either {either_true}"
        );
        assert!(
            either_match.votes > 2 * same_match.votes,
            "votes {} vs {}",
            either_match.votes,
            same_match.votes
        );

        let unflipped = match_star_lists(&stars, &stars, &either);
        assert!(!unflipped.flipped);
        assert!(correct_pairs(&unflipped, |x, y| (x, y)) >= 30);
        assert_eq!(unflipped, match_star_lists(&stars, &stars, &same));
    }
    fn registered(method: AffineAlignMethod) -> bool {
        matches!(method, AffineAlignMethod::Affine | AffineAlignMethod::Rigid)
    }

    #[test]
    fn rotations_of_0_37_90_179_5_and_180_degrees_register_within_0_05_px() {
        use crate::core::alignment::affine::align_channel_affine;
        let sky = a1_sky(A1_SIZE, A1_STARS);
        assert_eq!(sky.len(), A1_STARS);
        let reference = a1_reference(A1_SIZE, A1_STARS);
        let report: Vec<(bool, String)> = A1_ROTATIONS_DEG
            .iter()
            .map(|&deg| {
                let view = FrameView::Rotated(deg);
                let result = align_channel_affine(&reference, &a1_frame(A1_SIZE, A1_STARS, view));
                let rms = registration_rms(&result.transform, &a1_truth(view, A1_SIZE), &sky);
                let ok = registered(result.method)
                    && rms <= 0.05
                    && result.rms_px <= 0.05
                    && result.rejection.is_none();
                let line = format!(
                    "{deg} deg: method {}, rotation {:.3} deg, error vs truth {rms:.4} px RMS, fit {:.4} px RMS, inliers {}/{}, rejection {:?}",
                    result.method,
                    result.transform.rotation_deg(),
                    result.rms_px,
                    result.inliers,
                    result.matched_stars,
                    result.rejection
                );
                (ok, line)
            })
            .collect();
        let lines: Vec<&str> = report.iter().map(|(_, line)| line.as_str()).collect();
        assert!(report.iter().all(|(ok, _)| *ok), "{}", lines.join("\n"));
    }

    #[test]
    fn a_mirrored_frame_returns_identity_with_a_reflection_rejection_and_no_pc_fallback() {
        use crate::core::alignment::affine::align_channel_affine;
        let reference = a1_reference(A1_SIZE, A1_STARS);
        let mirrored = a1_frame(A1_SIZE, A1_STARS, FrameView::Mirrored);
        let result = align_channel_affine(&reference, &mirrored);
        assert_eq!(result.method, AffineAlignMethod::Identity, "{result:?}");
        assert_eq!(result.confidence, 0.0, "{result:?}");
        assert!(
            result.inliers >= 8 && result.matched_stars >= result.inliers,
            "the identity did not come from the star fit: {result:?}"
        );
        let t = result.transform;
        assert_eq!(
            [t.a, t.b, t.tx, t.c, t.d, t.ty],
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        );
        let rejection = result.rejection.as_deref().unwrap_or_default();
        assert!(rejection.starts_with("reflection (det="), "{result:?}");
    }

    #[test]
    fn a_repeated_grid_pattern_is_refused_not_mis_registered() {
        use crate::core::alignment::affine::align_channel_affine;
        let centre = (A1_SIZE as f64 - 1.0) * 0.5;
        let spacing = 96.0;
        let mut state = 0x6E1D_1A77_1CE5_0002u64;
        let cells: Vec<(f64, f64)> = (0..20)
            .flat_map(|row| (0..20).map(move |col| (row as f64, col as f64)))
            .collect();
        let lattice: Vec<(f64, f64, f64)> = cells
            .into_iter()
            .map(|(row, col)| {
                let jitter_x = (fixture_unit(&mut state) * 2.0 - 1.0) * 2.0;
                let jitter_y = (fixture_unit(&mut state) * 2.0 - 1.0) * 2.0;
                let flux = 4000.0 + fixture_unit(&mut state) * 12000.0;
                (
                    centre + (col - 9.5) * spacing + jitter_x,
                    centre + (row - 9.5) * spacing + jitter_y,
                    flux,
                )
            })
            .collect();
        let view = FrameView::Rotated(90.0);
        let truth = a1_truth(view, A1_SIZE);
        let reference = render_sky(&lattice, A1_SIZE, &AffineTransform::identity());
        let target = render_sky(&lattice, A1_SIZE, &truth);
        let result = align_channel_affine(&reference, &target);
        let rms = registration_rms(&result.transform, &truth, &lattice);
        if result.method == AffineAlignMethod::Identity {
            assert!(
                result.rejection.is_some(),
                "excluded without a reason: {result:?}"
            );
        } else {
            assert!(
                rms <= 0.05,
                "a wrong solution was accepted: rotation {:.3} deg, error vs truth {rms:.3} px RMS, {result:?}",
                result.transform.rotation_deg()
            );
        }
    }

    const SPARSE_REFERENCE: [(f64, f64); 17] = [
        (540.66, 420.96),
        (689.97, 948.04),
        (248.4, 734.66),
        (426.19, 676.77),
        (76.55, 617.75),
        (138.24, 686.54),
        (183.06, 768.9),
        (139.28, 498.71),
        (316.54, 940.3),
        (472.64, 491.81),
        (879.12, 637.33),
        (943.58, 846.67),
        (287.34, 96.71),
        (459.17, 855.4),
        (140.22, 790.54),
        (956.51, 198.05),
        (241.8, 112.15),
    ];

    const SPARSE_TARGET: [(f64, f64); 17] = [
        (523.88, 401.11),
        (681.57, 925.59),
        (236.49, 719.22),
        (413.57, 658.33),
        (63.03, 604.65),
        (810.82, 808.61),
        (556.46, 240.75),
        (914.49, 503.63),
        (304.37, 103.54),
        (130.64, 536.56),
        (140.02, 116.46),
        (432.71, 624.48),
        (328.38, 434.6),
        (709.6, 432.88),
        (463.01, 291.54),
        (471.39, 742.93),
        (604.93, 634.42),
    ];

    const SPARSE_COMMON_STARS: usize = 5;

    fn rendered(list: &[(f64, f64)]) -> ndarray::Array2<f32> {
        let sky: Vec<(f64, f64, f64)> = list.iter().map(|&(x, y)| (x, y, 8000.0)).collect();
        render_sky(&sky, 1000, &AffineTransform::identity())
    }

    #[test]
    fn a_sparse_field_keeps_its_same_parity_match_when_the_mirrored_votes_are_higher() {
        use crate::core::alignment::affine::align_channel_affine;
        let same = TriangleParams {
            parity: Parity::Same,
            ..PARAMS
        };
        let same_match = match_star_lists(&SPARSE_REFERENCE, &SPARSE_TARGET, &same);
        let either_match = match_star_lists(&SPARSE_REFERENCE, &SPARSE_TARGET, &PARAMS);
        assert!(
            either_match.flipped && either_match.votes > same_match.votes,
            "the fixture lost its higher mirrored vote total: {} vs {}",
            either_match.votes,
            same_match.votes
        );

        let result = align_channel_affine(&rendered(&SPARSE_REFERENCE), &rendered(&SPARSE_TARGET));
        assert!(
            registered(result.method) && result.rejection.is_none(),
            "{result:?}"
        );
        for (&(rx, ry), &(tx, ty)) in SPARSE_REFERENCE
            .iter()
            .zip(&SPARSE_TARGET)
            .take(SPARSE_COMMON_STARS)
        {
            let (px, py) = result.transform.map(rx, ry);
            assert!(
                (px - tx).hypot(py - ty) < 1.0,
                "({rx}, {ry}) maps to ({px:.2}, {py:.2}), expected ({tx}, {ty}): {result:?}"
            );
        }
    }

    #[test]
    fn a_sparse_mirrored_frame_is_refused_by_its_star_fit_without_phase_correlation() {
        use crate::core::alignment::affine::align_channel_affine;
        let sky = a1_sky(1000, 7);
        assert_eq!(sky.len(), 7);
        let reference = render_sky(&sky, 1000, &AffineTransform::identity());
        let mirrored = render_sky(&sky, 1000, &a1_truth(FrameView::Mirrored, 1000));
        let result = align_channel_affine(&reference, &mirrored);
        assert_eq!(result.method, AffineAlignMethod::Identity, "{result:?}");
        assert_eq!(result.confidence, 0.0, "{result:?}");
        assert!(
            result.inliers >= 6 && result.matched_stars >= result.inliers,
            "the refusal did not come from the star fit: {result:?}"
        );
        let rejection = result.rejection.as_deref().unwrap_or_default();
        assert!(rejection.starts_with("reflection (det="), "{result:?}");
    }
}
