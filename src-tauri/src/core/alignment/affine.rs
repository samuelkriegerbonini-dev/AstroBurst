use ndarray::Array2;
use rayon::prelude::*;

use crate::core::alignment::phase_correlation;
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::imaging::resample::bicubic_sample;

const MAX_STARS: usize = 80;
const TRIANGLE_TOLERANCE: f64 = 0.005;
const MIN_MATCHES_AFFINE: usize = 8;
const MIN_MATCHES_RIGID: usize = 5;
const RANSAC_ITERATIONS: usize = 500;
const RANSAC_INLIER_PX: f64 = 2.0;
const DETECTION_SIGMA: f64 = 5.0;
const MIN_TRIANGLE_SIDE: f64 = 20.0;
const MIN_VOTES: u32 = 3;
const MIN_INLIER_RATIO: f64 = 0.3;
const MAX_RESIDUAL_PX: f64 = 3.0;
const MAX_OFFSET_FRACTION: f64 = 0.25;
const MAX_ROTATION_DEG: f64 = 5.0;
const MIN_SCALE: f64 = 0.85;
const MAX_SCALE: f64 = 1.15;

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
    pub method: AffineAlignMethod,
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

struct TriangleDesc {
    star_indices: [usize; 3],
    ratio_mid: f64,
    ratio_long: f64,
    perimeter: f64,
}

pub fn align_channel_affine(
    reference: &Array2<f32>,
    target: &Array2<f32>,
) -> AffineAlignResult {
    let (rows, cols) = reference.dim();
    let ref_det = detect_stars(reference, DETECTION_SIGMA);
    let tgt_det = detect_stars(target, DETECTION_SIGMA);

    let ref_stars = top_n_stars(&ref_det.stars, MAX_STARS);
    let tgt_stars = top_n_stars(&tgt_det.stars, MAX_STARS);

    if ref_stars.len() < MIN_MATCHES_RIGID || tgt_stars.len() < MIN_MATCHES_RIGID {
        return fallback_phase_correlation(reference, target, rows, cols);
    }

    let ref_tris = build_triangles(&ref_stars);
    let tgt_tris = build_triangles(&tgt_stars);

    if ref_tris.is_empty() || tgt_tris.is_empty() {
        return fallback_phase_correlation(reference, target, rows, cols);
    }

    let matches = match_triangles(&ref_stars, &tgt_stars, &ref_tris, &tgt_tris);

    if matches.len() < MIN_MATCHES_RIGID {
        return fallback_phase_correlation(reference, target, rows, cols);
    }

    if matches.len() >= MIN_MATCHES_AFFINE {
        if let Some(result) = ransac_affine(&matches, AffineAlignMethod::Affine) {
            if is_sane_transform(&result, rows, cols) {
                return result;
            }
        }
    }

    if matches.len() >= MIN_MATCHES_RIGID {
        if let Some(result) = ransac_affine(&matches, AffineAlignMethod::Rigid) {
            if is_sane_transform(&result, rows, cols) {
                return result;
            }
        }
    }

    fallback_phase_correlation(reference, target, rows, cols)
}

fn is_sane_transform(result: &AffineAlignResult, rows: usize, cols: usize) -> bool {
    let t = &result.transform;

    let max_tx = cols as f64 * MAX_OFFSET_FRACTION;
    let max_ty = rows as f64 * MAX_OFFSET_FRACTION;
    if t.tx.abs() > max_tx || t.ty.abs() > max_ty {
        return false;
    }

    if t.rotation_deg().abs() > MAX_ROTATION_DEG {
        return false;
    }

    let sx = t.scale_x();
    let sy = t.scale_y();
    if sx < MIN_SCALE || sx > MAX_SCALE || sy < MIN_SCALE || sy > MAX_SCALE {
        return false;
    }

    if result.residual_px > MAX_RESIDUAL_PX {
        return false;
    }

    let ratio = result.inliers as f64 / result.matched_stars.max(1) as f64;
    if ratio < MIN_INLIER_RATIO {
        return false;
    }

    true
}

fn fallback_phase_correlation(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    rows: usize,
    cols: usize,
) -> AffineAlignResult {
    let pc = phase_correlation::phase_correlate(reference, target);

    let max_tx = cols as f64 * MAX_OFFSET_FRACTION;
    let max_ty = rows as f64 * MAX_OFFSET_FRACTION;
    if pc.dx.abs() > max_tx || pc.dy.abs() > max_ty || pc.confidence < 1.5 {
        return AffineAlignResult {
            transform: AffineTransform::identity(),
            matched_stars: 0,
            inliers: 0,
            residual_px: 0.0,
            method: AffineAlignMethod::Identity,
        };
    }

    AffineAlignResult {
        transform: AffineTransform::translation(pc.dx, pc.dy),
        matched_stars: 0,
        inliers: 0,
        residual_px: 0.0,
        method: AffineAlignMethod::PhaseCorrelation,
    }
}

fn top_n_stars(stars: &[DetectedStar], n: usize) -> Vec<(f64, f64)> {
    stars.iter()
        .take(n)
        .map(|s| (s.x, s.y))
        .collect()
}

fn build_triangles(stars: &[(f64, f64)]) -> Vec<TriangleDesc> {
    let n = stars.len();
    if n < 3 {
        return Vec::new();
    }
    let limit = n.min(50);
    let mut tris = Vec::with_capacity(limit * (limit - 1) * (limit - 2) / 6);

    for i in 0..limit {
        for j in (i + 1)..limit {
            for k in (j + 1)..limit {
                let mut sides = [
                    dist(stars[i], stars[j]),
                    dist(stars[j], stars[k]),
                    dist(stars[i], stars[k]),
                ];
                sides.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                if sides[0] < MIN_TRIANGLE_SIDE {
                    continue;
                }

                let ratio_mid = sides[1] / sides[0];
                let ratio_long = sides[2] / sides[0];
                let perimeter = sides[0] + sides[1] + sides[2];

                tris.push(TriangleDesc {
                    star_indices: [i, j, k],
                    ratio_mid,
                    ratio_long,
                    perimeter,
                });
            }
        }
    }
    tris
}

fn match_triangles(
    ref_stars: &[(f64, f64)],
    tgt_stars: &[(f64, f64)],
    ref_tris: &[TriangleDesc],
    tgt_tris: &[TriangleDesc],
) -> Vec<(f64, f64, f64, f64)> {
    let mut vote_map: std::collections::HashMap<(usize, usize), u32> =
        std::collections::HashMap::new();

    for rt in ref_tris {
        for tt in tgt_tris {
            let d_mid = (rt.ratio_mid - tt.ratio_mid).abs();
            let d_long = (rt.ratio_long - tt.ratio_long).abs();

            if d_mid > TRIANGLE_TOLERANCE || d_long > TRIANGLE_TOLERANCE {
                continue;
            }

            let ref_sorted = sort_triangle_vertices(ref_stars, &rt.star_indices);
            let tgt_sorted = sort_triangle_vertices(tgt_stars, &tt.star_indices);

            for p in 0..3 {
                let ri = ref_sorted[p];
                let ti = tgt_sorted[p];
                *vote_map.entry((ri, ti)).or_insert(0) += 1;
            }
        }
    }

    let mut pairs: Vec<((usize, usize), u32)> = vote_map.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1));

    let mut used_ref = vec![false; ref_stars.len()];
    let mut used_tgt = vec![false; tgt_stars.len()];
    let mut matches = Vec::new();

    for ((ri, ti), votes) in &pairs {
        if *votes < MIN_VOTES {
            break;
        }
        if used_ref[*ri] || used_tgt[*ti] {
            continue;
        }
        used_ref[*ri] = true;
        used_tgt[*ti] = true;
        matches.push((
            ref_stars[*ri].0,
            ref_stars[*ri].1,
            tgt_stars[*ti].0,
            tgt_stars[*ti].1,
        ));
    }

    matches
}

fn sort_triangle_vertices(
    stars: &[(f64, f64)],
    indices: &[usize; 3],
) -> [usize; 3] {
    let mut sorted = *indices;
    let cx = (stars[sorted[0]].0 + stars[sorted[1]].0 + stars[sorted[2]].0) / 3.0;
    let cy = (stars[sorted[0]].1 + stars[sorted[1]].1 + stars[sorted[2]].1) / 3.0;

    sorted.sort_by(|&a, &b| {
        let ang_a = (stars[a].1 - cy).atan2(stars[a].0 - cx);
        let ang_b = (stars[b].1 - cy).atan2(stars[b].0 - cx);
        ang_a.partial_cmp(&ang_b).unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted
}

fn ransac_affine(
    matches: &[(f64, f64, f64, f64)],
    method: AffineAlignMethod,
) -> Option<AffineAlignResult> {
    let n = matches.len();
    let min_sample = if method == AffineAlignMethod::Affine { 3 } else { 2 };
    if n < min_sample {
        return None;
    }

    let mut best_inliers = 0;
    let mut best_transform = AffineTransform::identity();
    let mut best_inlier_mask = vec![false; n];

    let mut rng_state: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let inline_rand = |state: &mut u64| -> usize {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        (*state as usize) % n
    };

    let mut mask = vec![false; n];

    for _ in 0..RANSAC_ITERATIONS {
        let mut sample = Vec::with_capacity(min_sample);
        let mut attempts = 0;
        while sample.len() < min_sample && attempts < 20 {
            let idx = inline_rand(&mut rng_state);
            if !sample.contains(&idx) {
                sample.push(idx);
            }
            attempts += 1;
        }
        if sample.len() < min_sample {
            continue;
        }

        let sample_matches: Vec<(f64, f64, f64, f64)> =
            sample.iter().map(|&i| matches[i]).collect();

        let transform = match method {
            AffineAlignMethod::Affine => fit_affine(&sample_matches),
            _ => fit_rigid(&sample_matches),
        };
        let transform = match transform {
            Some(t) => t,
            None => continue,
        };

        mask.fill(false);
        let mut inlier_count = 0;
        for (i, &(rx, ry, tx, ty)) in matches.iter().enumerate() {
            let (px, py) = transform.map(rx, ry);
            let err = ((px - tx).powi(2) + (py - ty).powi(2)).sqrt();
            if err < RANSAC_INLIER_PX {
                inlier_count += 1;
                mask[i] = true;
            }
        }

        if inlier_count > best_inliers {
            best_inliers = inlier_count;
            best_transform = transform;
            best_inlier_mask.copy_from_slice(&mask);
        }
    }

    if best_inliers < MIN_MATCHES_RIGID {
        return None;
    }

    let inlier_ratio = best_inliers as f64 / n as f64;
    if inlier_ratio < MIN_INLIER_RATIO {
        return None;
    }

    let inlier_matches: Vec<(f64, f64, f64, f64)> = matches
        .iter()
        .zip(best_inlier_mask.iter())
        .filter(|(_, &m)| m)
        .map(|(&pt, _)| pt)
        .collect();

    let refined = match method {
        AffineAlignMethod::Affine => fit_affine(&inlier_matches),
        _ => fit_rigid(&inlier_matches),
    }
    .unwrap_or(best_transform);

    let residual = compute_residual(&inlier_matches, &refined);
    if residual > MAX_RESIDUAL_PX {
        return None;
    }

    Some(AffineAlignResult {
        transform: refined,
        matched_stars: matches.len(),
        inliers: best_inliers,
        residual_px: residual,
        method,
    })
}

fn fit_affine(matches: &[(f64, f64, f64, f64)]) -> Option<AffineTransform> {
    let n = matches.len();
    if n < 3 {
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

fn solve_3x3_ls(
    matches: &[(f64, f64, f64, f64)],
    solve_x: bool,
) -> Option<((f64, f64), f64)> {
    let mut ata = [[0.0f64; 3]; 3];
    let mut atb = [0.0f64; 3];

    for &(rx, ry, tx, ty) in matches {
        let target = if solve_x { tx } else { ty };
        let row = [rx, ry, 1.0];

        for i in 0..3 {
            for j in 0..3 {
                ata[i][j] += row[i] * row[j];
            }
            atb[i] += row[i] * target;
        }
    }

    let x = solve_3x3(ata, atb)?;
    Some(((x[0], x[1]), x[2]))
}

fn solve_3x3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
            - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);

    if det.abs() < 1e-12 {
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

fn fit_rigid(matches: &[(f64, f64, f64, f64)]) -> Option<AffineTransform> {
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

fn compute_residual(matches: &[(f64, f64, f64, f64)], transform: &AffineTransform) -> f64 {
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

#[inline]
fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn warp_image(
    image: &Array2<f32>,
    transform: &AffineTransform,
    out_rows: usize,
    out_cols: usize,
) -> Array2<f32> {
    let (src_rows, src_cols) = image.dim();
    let slice = image.as_slice().expect("contiguous");
    let total = out_rows * out_cols;
    let mut buf = vec![f32::NAN; total];

    buf.par_chunks_mut(out_cols)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..out_cols {
                let (sx, sy) = transform.map(x as f64, y as f64);
                if sx >= 0.0
                    && sy >= 0.0
                    && sx < (src_cols - 1) as f64
                    && sy < (src_rows - 1) as f64
                {
                    row[x] = bicubic_sample(slice, src_rows, src_cols, sy, sx);
                }
            }
        });

    Array2::from_shape_vec((out_rows, out_cols), buf).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let ref_pts = vec![
            (100.0, 100.0),
            (200.0, 100.0),
            (100.0, 200.0),
            (200.0, 200.0),
            (150.0, 150.0),
        ];

        let matches: Vec<(f64, f64, f64, f64)> = ref_pts
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
        let a = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let b = [3.0, 5.0, 7.0];
        let x = solve_3x3(a, b).unwrap();
        assert!((x[0] - 3.0).abs() < 1e-10);
        assert!((x[1] - 5.0).abs() < 1e-10);
        assert!((x[2] - 7.0).abs() < 1e-10);
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
        assert!(warped[[53, 55]] > 500.0);
        assert!(warped[[50, 50]] > 500.0 || warped[[53, 55]] > warped[[50, 50]]);
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
        let tris = build_triangles(&stars);
        let matches = match_triangles(&stars, &stars, &tris, &tris);
        assert!(matches.len() >= 4, "got {} matches", matches.len());
    }

    #[test]
    fn test_nan_fill_outside_bounds() {
        let img = Array2::from_elem((50, 50), 100.0f32);
        let t = AffineTransform::translation(1000.0, 1000.0);
        let warped = warp_image(&img, &t, 50, 50);
        assert!(warped[[25, 25]].is_nan());
    }
}
