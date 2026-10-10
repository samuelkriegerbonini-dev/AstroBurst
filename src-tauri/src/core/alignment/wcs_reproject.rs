use ndarray::{s, Array2};
use rayon::prelude::*;

use crate::core::analysis::star_detection::detect_stars;
use crate::core::astrometry::wcs::{WcsKind, WcsTransform};
use crate::core::imaging::resample::{box_reduce, compute_wcs_updates};
use crate::core::imaging::sampling::bicubic_sample;
use crate::math::exact_median_f64;
use crate::types::header::HduHeader;

pub const MAP_STEP_PX: usize = 8;
pub const REF_TIE_TOLERANCE: f64 = 0.005;
pub const PREFILTER_MIN_RATIO: f64 = 1.5;
const RESIDUAL_MIN_STARS: usize = 4;
const RESIDUAL_WINDOW_PX: usize = 512;
const RESIDUAL_WINDOW_GRID: usize = 3;
const RESIDUAL_DETECTION_SIGMA: f64 = 5.0;
const RESIDUAL_REFERENCE_STARS: usize = 30;
const RESIDUAL_MATCH_PX: f64 = 3.0;
const RESIDUAL_AGREEMENT_PX: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReprojectReport {
    pub scale_ratio: f64,
    pub rotation_deg: f64,
    pub area_ratio: f64,
    pub coverage: f64,
    pub prefilter_k: usize,
    pub wcs_kind: WcsKind,
}

struct CoarseMap {
    nodes_x: usize,
    nodes_y: usize,
    sx: Vec<f64>,
    sy: Vec<f64>,
}

fn node_count(len: usize) -> usize {
    if len == 0 {
        1
    } else {
        (len - 1).div_ceil(MAP_STEP_PX) + 1
    }
}

#[inline]
fn weighted(value: f64, weight: f64) -> f64 {
    if weight == 0.0 {
        0.0
    } else {
        value * weight
    }
}

impl CoarseMap {
    fn build(target_wcs: &WcsTransform, reference_wcs: &WcsTransform, out_rows: usize, out_cols: usize) -> CoarseMap {
        let nodes_x = node_count(out_cols);
        let nodes_y = node_count(out_rows);
        let nodes: Vec<(f64, f64)> = (0..nodes_y)
            .flat_map(|j| (0..nodes_x).map(move |i| ((i * MAP_STEP_PX) as f64, (j * MAP_STEP_PX) as f64)))
            .collect();
        let sky: Vec<(f64, f64)> = reference_wcs
            .pixel_to_world_batch(&nodes)
            .into_iter()
            .map(|c| (c.ra, c.dec))
            .collect();
        let (sx, sy) = target_wcs.world_to_pixel_batch(&sky).into_iter().unzip();
        CoarseMap { nodes_x, nodes_y, sx, sy }
    }

    fn sample(&self, x: f64, y: f64) -> (f64, f64) {
        let step = MAP_STEP_PX as f64;
        let gx = x / step;
        let gy = y / step;
        let i0 = (gx.floor().max(0.0) as usize).min(self.nodes_x - 1);
        let j0 = (gy.floor().max(0.0) as usize).min(self.nodes_y - 1);
        let i1 = (i0 + 1).min(self.nodes_x - 1);
        let j1 = (j0 + 1).min(self.nodes_y - 1);
        let fx = gx - i0 as f64;
        let fy = gy - j0 as f64;
        let w00 = (1.0 - fx) * (1.0 - fy);
        let w10 = fx * (1.0 - fy);
        let w01 = (1.0 - fx) * fy;
        let w11 = fx * fy;
        let k00 = j0 * self.nodes_x + i0;
        let k10 = j0 * self.nodes_x + i1;
        let k01 = j1 * self.nodes_x + i0;
        let k11 = j1 * self.nodes_x + i1;
        let lerp = |v: &[f64]| {
            weighted(v[k00], w00) + weighted(v[k10], w10) + weighted(v[k01], w01) + weighted(v[k11], w11)
        };
        (lerp(&self.sx), lerp(&self.sy))
    }
}

pub fn reproject_to_grid(
    target: &Array2<f32>,
    target_wcs: &WcsTransform,
    reference_wcs: &WcsTransform,
    out_rows: usize,
    out_cols: usize,
) -> (Array2<f32>, ReprojectReport) {
    let (rows, cols) = target.dim();
    let total = out_rows * out_cols;
    let mut buf = vec![f32::NAN; total];
    let mut finite = 0usize;
    if rows > 0 && cols > 0 && total > 0 {
        let standard = target.as_standard_layout();
        let slice = standard.as_slice().expect("standard layout is contiguous");
        let map = CoarseMap::build(target_wcs, reference_wcs, out_rows, out_cols);
        let max_x = cols as f64 - 0.5;
        let max_y = rows as f64 - 0.5;
        finite = buf
            .par_chunks_mut(out_cols)
            .enumerate()
            .map(|(y, row)| {
                let mut count = 0usize;
                for (x, px) in row.iter_mut().enumerate() {
                    let (sx, sy) = map.sample(x as f64, y as f64);
                    if sx >= -0.5 && sy >= -0.5 && sx <= max_x && sy <= max_y {
                        let v = bicubic_sample(slice, rows, cols, sy, sx);
                        *px = v;
                        if v.is_finite() {
                            count += 1;
                        }
                    }
                }
                count
            })
            .sum();
    }
    let image = Array2::from_shape_vec((out_rows, out_cols), buf).expect("buffer matches the output shape");
    let coverage = if total == 0 { 0.0 } else { finite as f64 / total as f64 };
    let report = ReprojectReport {
        scale_ratio: target_wcs.pixel_scale_arcsec() / reference_wcs.pixel_scale_arcsec(),
        rotation_deg: relative_rotation_deg(target_wcs, reference_wcs),
        area_ratio: pixel_area_ratio(reference_wcs, target_wcs),
        coverage,
        prefilter_k: 1,
        wcs_kind: target_wcs.kind(),
    };
    (image, report)
}

pub fn prefilter_coarse_target(
    target: &Array2<f32>,
    target_header: &HduHeader,
    ratio: f64,
) -> Option<(Array2<f32>, HduHeader, usize)> {
    if !ratio.is_finite() || ratio < PREFILTER_MIN_RATIO {
        return None;
    }
    let k = (ratio.floor() as usize).max(2);
    let (rows, cols) = target.dim();
    let (out_rows, out_cols) = (rows / k, cols / k);
    if out_rows == 0 || out_cols == 0 {
        return None;
    }
    let (crop_rows, crop_cols) = (out_rows * k, out_cols * k);
    let reduced = box_reduce(target.slice(s![..crop_rows, ..crop_cols]), out_rows, out_cols).ok()?;
    let mut header = target_header.clone();
    for (key, value) in compute_wcs_updates(target_header, (crop_rows, crop_cols), (out_rows, out_cols)) {
        if key == "NAXIS1" || key == "NAXIS2" {
            header.set(&key, (value as i64).to_string());
        } else {
            header.set_f64(&key, value);
        }
    }
    Some((reduced, header, k))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarResidual {
    pub dy: f64,
    pub dx: f64,
    pub matched: usize,
    pub agreeing: usize,
    pub rms_px: f64,
}

type WindowStars = (Vec<(f64, f64)>, Vec<(f64, f64)>);

fn residual_window_spans(len: usize) -> Vec<(usize, usize)> {
    let count = (len / RESIDUAL_WINDOW_PX).clamp(1, RESIDUAL_WINDOW_GRID);
    let size = (len / count).min(RESIDUAL_WINDOW_PX);
    (0..count).map(|i| (((2 * i + 1) * len / (2 * count)).saturating_sub(size / 2), size)).collect()
}

fn window_stars(image: &Array2<f32>, (r0, rows): (usize, usize), (c0, cols): (usize, usize), limit: usize) -> Vec<(f64, f64)> {
    let window = image.slice(s![r0..r0 + rows, c0..c0 + cols]).to_owned();
    detect_stars(&window, RESIDUAL_DETECTION_SIGMA)
        .stars
        .into_iter()
        .take(limit)
        .map(|star| (c0 as f64 + star.x, r0 as f64 + star.y))
        .collect()
}

fn median_displacement(displacements: &[(f64, f64)]) -> (f64, f64) {
    let dy: Vec<f64> = displacements.iter().map(|d| d.0).collect();
    let dx: Vec<f64> = displacements.iter().map(|d| d.1).collect();
    (exact_median_f64(&dy), exact_median_f64(&dx))
}

fn residual_from_seed(windows: &[WindowStars], (seed_dy, seed_dx): (f64, f64)) -> Option<StarResidual> {
    let displacements: Vec<(f64, f64)> = windows
        .iter()
        .flat_map(|(references, targets)| {
            references.iter().filter_map(move |&(x, y)| {
                let (px, py) = (x + seed_dx, y + seed_dy);
                targets
                    .iter()
                    .map(|&(tx, ty)| ((tx - px).hypot(ty - py), (ty - y, tx - x)))
                    .filter(|(distance, _)| *distance <= RESIDUAL_MATCH_PX)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, displacement)| displacement)
            })
        })
        .collect();
    if displacements.is_empty() {
        return None;
    }
    let first = median_displacement(&displacements);
    let agreeing: Vec<(f64, f64)> = displacements
        .iter()
        .copied()
        .filter(|d| (d.0 - first.0).hypot(d.1 - first.1) <= RESIDUAL_AGREEMENT_PX)
        .collect();
    if agreeing.len() < RESIDUAL_MIN_STARS {
        return None;
    }
    let (dy, dx) = median_displacement(&agreeing);
    let rms_px = (agreeing.iter().map(|d| (d.0 - dy).powi(2) + (d.1 - dx).powi(2)).sum::<f64>() / agreeing.len() as f64).sqrt();
    Some(StarResidual { dy, dx, matched: displacements.len(), agreeing: agreeing.len(), rms_px })
}

pub fn point_source_residual(reference: &Array2<f32>, reprojected: &Array2<f32>, seeds: &[(f64, f64)]) -> Option<StarResidual> {
    let (rows, cols) = reference.dim();
    if reprojected.dim() != (rows, cols) {
        return None;
    }
    let spans: Vec<((usize, usize), (usize, usize))> = residual_window_spans(rows)
        .into_iter()
        .flat_map(|r| residual_window_spans(cols).into_iter().map(move |c| (r, c)))
        .collect();
    let windows: Vec<WindowStars> = spans
        .par_iter()
        .map(|&(r, c)| (window_stars(reference, r, c, RESIDUAL_REFERENCE_STARS), window_stars(reprojected, r, c, usize::MAX)))
        .collect();
    seeds
        .iter()
        .filter_map(|&seed| residual_from_seed(&windows, seed))
        .reduce(|best, next| if next.agreeing > best.agreeing { next } else { best })
}

fn det(m: &[[f64; 2]; 2]) -> f64 {
    m[0][0] * m[1][1] - m[0][1] * m[1][0]
}

pub fn pixel_area_ratio(reference: &WcsTransform, target: &WcsTransform) -> f64 {
    det(&reference.raw_params().4).abs() / det(&target.raw_params().4).abs()
}

pub fn relative_rotation_deg(target: &WcsTransform, reference: &WcsTransform) -> f64 {
    let cd_ref = reference.raw_params().4;
    let cd_target = target.raw_params().4;
    let d = det(&cd_ref);
    let inv = [[cd_ref[1][1] / d, -cd_ref[0][1] / d], [-cd_ref[1][0] / d, cd_ref[0][0] / d]];
    let m00 = inv[0][0] * cd_target[0][0] + inv[0][1] * cd_target[1][0];
    let m01 = inv[0][0] * cd_target[0][1] + inv[0][1] * cd_target[1][1];
    let m10 = inv[1][0] * cd_target[0][0] + inv[1][1] * cd_target[1][0];
    let m11 = inv[1][0] * cd_target[0][1] + inv[1][1] * cd_target[1][1];
    let deg = (m10 - m01).atan2(m00 + m11).to_degrees();
    if deg <= -180.0 {
        deg + 360.0
    } else {
        deg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    const ARCSEC_DEG: f64 = 1.0 / 3600.0;
    const FWHM_TO_SIGMA: f64 = 0.424_660_900_144_009_5;

    fn header(numeric: &[(&str, f64)], text: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in text {
            h.set(k, v.to_string());
        }
        for (k, v) in numeric {
            h.set_f64(k, *v);
        }
        h
    }

    fn tan_header(naxis: (usize, usize), crpix: (f64, f64), crval: (f64, f64), cd: [[f64; 2]; 2]) -> HduHeader {
        header(
            &[
                ("CRPIX1", crpix.0),
                ("CRPIX2", crpix.1),
                ("CRVAL1", crval.0),
                ("CRVAL2", crval.1),
                ("CD1_1", cd[0][0]),
                ("CD1_2", cd[0][1]),
                ("CD2_1", cd[1][0]),
                ("CD2_2", cd[1][1]),
            ],
            &[
                ("NAXIS1", &naxis.0.to_string()),
                ("NAXIS2", &naxis.1.to_string()),
                ("CTYPE1", "RA---TAN"),
                ("CTYPE2", "DEC--TAN"),
            ],
        )
    }

    fn rotated_cd(scale_arcsec: f64, angle_deg: f64) -> [[f64; 2]; 2] {
        let s = scale_arcsec * ARCSEC_DEG;
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        [[-s * cos, s * sin], [s * sin, s * cos]]
    }

    fn wcs(h: &HduHeader) -> WcsTransform {
        WcsTransform::from_header(h).unwrap()
    }

    fn f200w_header() -> HduHeader {
        header(
            &[
                ("CRPIX1", 7185.307253487335),
                ("CRPIX2", 4294.478201664114),
                ("CRVAL1", 274.7298929226303),
                ("CRVAL2", -13.851728302049773),
                ("CDELT1", 0.0000086738920163002),
                ("CDELT2", 0.0000086738920163002),
                ("PC1_1", 0.03196958810575354),
                ("PC1_2", 0.9994888420769631),
                ("PC2_1", 0.9994888420769631),
                ("PC2_2", -0.03196958810575354),
            ],
            &[
                ("NAXIS1", "14344"),
                ("NAXIS2", "8589"),
                ("CTYPE1", "RA---TAN"),
                ("CTYPE2", "DEC--TAN"),
                ("RADESYS", "ICRS"),
            ],
        )
    }

    fn f444w_header() -> HduHeader {
        header(
            &[
                ("CRPIX1", 3545.3994741278593),
                ("CRPIX2", 2092.707324684303),
                ("CRVAL1", 274.7299961326976),
                ("CRVAL2", -13.851596271958135),
                ("CDELT1", 0.0000174739632912783),
                ("CDELT2", 0.0000174739632912783),
                ("PC1_1", 0.04044713241775692),
                ("PC1_2", 0.99918167991571),
                ("PC2_1", 0.99918167991571),
                ("PC2_2", -0.04044713241775692),
            ],
            &[
                ("NAXIS1", "7065"),
                ("NAXIS2", "4177"),
                ("CTYPE1", "RA---TAN"),
                ("CTYPE2", "DEC--TAN"),
                ("RADESYS", "ICRS"),
            ],
        )
    }

    fn render_stars(rows: usize, cols: usize, pedestal: f32, stars: &[(f64, f64, f64)], sigma_px: f64) -> Array2<f32> {
        let mut img = Array2::from_elem((rows, cols), pedestal);
        let reach = (6.0 * sigma_px).ceil() as i64;
        for &(x, y, peak) in stars {
            let cx = x.round() as i64;
            let cy = y.round() as i64;
            for r in (cy - reach)..=(cy + reach) {
                if r < 0 || r >= rows as i64 {
                    continue;
                }
                for c in (cx - reach)..=(cx + reach) {
                    if c < 0 || c >= cols as i64 {
                        continue;
                    }
                    let dx = c as f64 - x;
                    let dy = r as f64 - y;
                    let value = peak * (-(dx * dx + dy * dy) / (2.0 * sigma_px * sigma_px)).exp();
                    img[[r as usize, c as usize]] += value as f32;
                }
            }
        }
        img
    }

    fn window_sum(img: &Array2<f32>, x0: f64, y0: f64, half: i64, pedestal: f64) -> f64 {
        let (rows, cols) = img.dim();
        let cx = x0.round() as i64;
        let cy = y0.round() as i64;
        let mut sum = 0.0;
        for r in (cy - half)..=(cy + half) {
            for c in (cx - half)..=(cx + half) {
                if r < 0 || c < 0 || r >= rows as i64 || c >= cols as i64 {
                    continue;
                }
                let v = img[[r as usize, c as usize]] as f64;
                if v.is_finite() {
                    sum += v - pedestal;
                }
            }
        }
        sum
    }

    fn centroid(img: &Array2<f32>, x0: f64, y0: f64, half: i64, pedestal: f64) -> (f64, f64) {
        let (rows, cols) = img.dim();
        let cx = x0.round() as i64;
        let cy = y0.round() as i64;
        let (mut sw, mut sx, mut sy) = (0.0, 0.0, 0.0);
        for r in (cy - half)..=(cy + half) {
            for c in (cx - half)..=(cx + half) {
                if r < 0 || c < 0 || r >= rows as i64 || c >= cols as i64 {
                    continue;
                }
                let v = img[[r as usize, c as usize]] as f64;
                if !v.is_finite() {
                    continue;
                }
                let w = (v - pedestal).max(0.0);
                sw += w;
                sx += w * c as f64;
                sy += w * r as f64;
            }
        }
        (sx / sw, sy / sw)
    }

    fn finite_median(img: &Array2<f32>) -> f64 {
        let mut values: Vec<f32> = img.iter().copied().filter(|v| v.is_finite()).collect();
        assert!(!values.is_empty(), "no finite pixel");
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        values[values.len() / 2] as f64
    }

    fn sky_separation_deg(a: (f64, f64), b: (f64, f64)) -> f64 {
        let dra = (a.0 - b.0) * a.1.to_radians().cos();
        let ddec = a.1 - b.1;
        (dra * dra + ddec * ddec).sqrt()
    }

    fn half_scale_rotated_pair() -> (HduHeader, HduHeader) {
        let reference = tan_header((512, 512), (256.5, 256.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        let target = tan_header((256, 256), (128.5, 128.5), (83.0, 22.0 + 3.0 * ARCSEC_DEG), rotated_cd(0.2, 0.5));
        (reference, target)
    }

    const STAR_REF_POSITIONS: [(f64, f64); 12] = [
        (90.3, 100.7),
        (420.6, 95.2),
        (95.9, 415.4),
        (410.2, 430.8),
        (256.4, 256.6),
        (150.25, 300.75),
        (330.5, 160.5),
        (200.0, 420.0),
        (380.3, 300.1),
        (120.8, 210.9),
        (300.7, 390.3),
        (240.1, 130.6),
    ];

    #[test]
    fn reprojection_of_a_half_scale_rotated_channel_lands_stars_on_the_reference_grid() {
        let (ref_header, tgt_header) = half_scale_rotated_pair();
        let (ref_wcs, tgt_wcs) = (wcs(&ref_header), wcs(&tgt_header));
        let sigma_target = 2.5 * FWHM_TO_SIGMA;
        let sigma_reference = 5.0 * FWHM_TO_SIGMA;
        let pedestal = 100.0f32;
        let peak = 1000.0;
        let mut ref_stars = Vec::new();
        let mut tgt_stars = Vec::new();
        for &(x, y) in &STAR_REF_POSITIONS {
            let sky = ref_wcs.pixel_to_world(x, y);
            let (tx, ty) = tgt_wcs.world_to_pixel(sky.ra, sky.dec);
            assert!(tx > 12.0 && tx < 243.0 && ty > 12.0 && ty < 243.0, "star at ({x},{y}) maps to ({tx},{ty})");
            ref_stars.push((x, y, peak));
            tgt_stars.push((tx, ty, peak));
        }
        let reference = render_stars(512, 512, pedestal, &ref_stars, sigma_reference);
        let target = render_stars(256, 256, pedestal, &tgt_stars, sigma_target);

        let (out, report) = reproject_to_grid(&target, &tgt_wcs, &ref_wcs, 512, 512);

        assert_eq!(out.dim(), (512, 512));
        for (i, &(x, y)) in STAR_REF_POSITIONS.iter().enumerate() {
            let expected = centroid(&reference, x, y, 8, pedestal as f64);
            let got = centroid(&out, x, y, 8, pedestal as f64);
            let d = ((got.0 - expected.0).powi(2) + (got.1 - expected.1).powi(2)).sqrt();
            assert!(d < 0.15, "star {i}: centroid {got:?} vs {expected:?} ({d} px)");
            let flux_target = window_sum(&target, tgt_stars[i].0, tgt_stars[i].1, 6, pedestal as f64);
            let flux_out = window_sum(&out, x, y, 12, pedestal as f64);
            let ratio = flux_out / flux_target;
            assert!((3.9..=4.1).contains(&ratio), "star {i}: output flux / target flux = {ratio} (4 pixels per target pixel, values unscaled)");
        }
        assert!((finite_median(&out) - finite_median(&target)).abs() < 1e-3);
        assert!((report.scale_ratio - 2.0).abs() < 1e-6, "{}", report.scale_ratio);
        assert!((report.rotation_deg - 0.5).abs() < 1e-6, "{}", report.rotation_deg);
        assert!((report.area_ratio - 0.25).abs() < 1e-9, "{}", report.area_ratio);
        assert!(report.coverage > 0.9 && report.coverage < 1.0, "{}", report.coverage);
        assert_eq!(report.prefilter_k, 1);
    }

    #[test]
    fn the_report_states_the_wcs_kind_of_the_target_as_used() {
        use std::sync::Arc;

        use crate::core::astrometry::gwcs::test_support::fixture_pipeline;
        use crate::core::astrometry::gwcs::{GwcsOrigin, GwcsSource};

        let source = Arc::new(GwcsSource {
            pipeline: fixture_pipeline("wcs_jwst_nircam_cal300.asdf"),
            origin: GwcsOrigin::AsdfTree { key: "wcs".into() },
            wcsinfo_sip_max_err_px: None,
            wcsinfo_sip_inv_err_px: None,
        });
        let dims = header(&[], &[("NAXIS1", "2048"), ("NAXIS2", "2048")]);
        let gwcs = WcsTransform::from_gwcs(source, &dims).unwrap();
        let (crpix1, crpix2, crval1, crval2, cd, _) = gwcs.raw_params();
        let reference = wcs(&tan_header((64, 64), (32.5, 32.5), (crval1, crval2), cd));
        let target = Array2::<f32>::ones((2048, 2048));

        let (_, report) = reproject_to_grid(&target, &gwcs, &reference, 64, 64);
        assert_eq!(report.wcs_kind, WcsKind::Gwcs, "{report:?}");
        assert!(report.coverage > 0.99, "{}", report.coverage);
        assert_eq!(report.prefilter_k, 1);

        let header_target = wcs(&tan_header((2048, 2048), (crpix1, crpix2), (crval1, crval2), cd));
        let (_, report) = reproject_to_grid(&target, &header_target, &reference, 64, 64);
        assert_eq!(report.wcs_kind, WcsKind::Header, "{report:?}");
    }

    #[test]
    fn coarse_map_interpolation_matches_the_exact_mapping() {
        let (ref_header, tgt_header) = half_scale_rotated_pair();
        let (ref_wcs, tgt_wcs) = (wcs(&ref_header), wcs(&tgt_header));
        let map = CoarseMap::build(&tgt_wcs, &ref_wcs, 512, 512);
        let mut rng = StdRng::seed_from_u64(42);
        let mut worst = 0.0f64;
        for _ in 0..500 {
            let x = rng.gen_range(0.0..511.0);
            let y = rng.gen_range(0.0..511.0);
            let sky = ref_wcs.pixel_to_world(x, y);
            let exact = tgt_wcs.world_to_pixel(sky.ra, sky.dec);
            let interp = map.sample(x, y);
            let d = ((interp.0 - exact.0).powi(2) + (interp.1 - exact.1).powi(2)).sqrt();
            assert!(d < 1e-3, "({x},{y}): interpolated {interp:?} vs exact {exact:?}");
            worst = worst.max(d);
        }
        assert!(worst.is_finite());
    }

    #[test]
    fn pixels_outside_the_target_footprint_are_nan() {
        let reference = tan_header((512, 512), (256.5, 256.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        let target = tan_header((128, 128), (64.5, 64.5), (83.0, 22.0), rotated_cd(0.2, 0.0));
        let image = Array2::from_elem((128, 128), 7.0f32);
        let (out, report) = reproject_to_grid(&image, &wcs(&target), &wcs(&reference), 512, 512);
        for (x, y) in [(0usize, 0usize), (511, 0), (0, 511), (511, 511), (100, 256), (256, 400)] {
            assert!(out[[y, x]].is_nan(), "({x},{y}) = {}", out[[y, x]]);
        }
        for (x, y) in [(256usize, 256usize), (140, 256), (370, 256), (256, 140), (256, 370)] {
            assert!((out[[y, x]] - 7.0).abs() < 1e-4, "({x},{y}) = {}", out[[y, x]]);
        }
        assert!((report.coverage - 0.25).abs() < 0.02, "{}", report.coverage);
    }

    #[test]
    fn reprojected_values_are_never_scaled_whatever_bunit() {
        let reference = tan_header((256, 256), (128.5, 128.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        let ref_wcs = wcs(&reference);
        for bunit in ["ADU", "MJy/sr"] {
            let mut target = tan_header((128, 128), (64.5, 64.5), (83.0, 22.0), rotated_cd(0.2, 0.0));
            target.set("BUNIT", bunit.to_string());
            let image = Array2::from_elem((128, 128), 100.0f32);
            let (out, report) = reproject_to_grid(&image, &wcs(&target), &ref_wcs, 256, 256);
            let finite: Vec<f32> = out.iter().copied().filter(|v| v.is_finite()).collect();
            assert!(finite.len() > 60000, "{bunit}: {} finite pixels", finite.len());
            assert!(finite.iter().all(|v| (v - 100.0).abs() < 1e-3), "{bunit}: values were scaled");
            assert!((report.area_ratio - 0.25).abs() < 1e-9, "{bunit}: {}", report.area_ratio);
        }
    }

    #[test]
    fn pixel_area_ratio_is_det_cd_ref_over_det_cd_target() {
        let (f200w, f444w) = (wcs(&f200w_header()), wcs(&f444w_header()));
        let ratio = pixel_area_ratio(&f200w, &f444w);
        assert!((ratio - 0.246403).abs() < 1e-6, "{ratio}");
        let swapped = pixel_area_ratio(&f444w, &f200w);
        assert!((swapped - 4.0584).abs() < 1e-4, "{swapped}");
    }

    #[test]
    fn relative_rotation_is_signed_by_the_stated_convention() {
        let (f200w, f444w) = (wcs(&f200w_header()), wcs(&f444w_header()));
        let forward = relative_rotation_deg(&f444w, &f200w);
        assert!((forward - 0.486).abs() < 0.002, "{forward}");
        let swapped = relative_rotation_deg(&f200w, &f444w);
        assert!((swapped + 0.486).abs() < 0.002, "{swapped}");
    }

    #[test]
    fn coarse_reference_conserves_star_flux() {
        let reference = tan_header((128, 128), (64.5 + 3.37, 64.5 - 2.61), (83.0, 22.0), rotated_cd(0.2, 0.0));
        let tgt_header = tan_header((256, 256), (128.5, 128.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        let (ref_wcs, tgt_wcs) = (wcs(&reference), wcs(&tgt_header));
        let sigma = 1.2 * FWHM_TO_SIGMA;
        let mut rng = StdRng::seed_from_u64(7);
        let stars: Vec<(f64, f64, f64)> = (0..12)
            .map(|_| (rng.gen_range(40.0..216.0), rng.gen_range(40.0..216.0), 1000.0))
            .collect();
        let target = render_stars(256, 256, 0.0, &stars, sigma);

        let (reduced, adjusted_header, k) = prefilter_coarse_target(&target, &tgt_header, 2.0).expect("ratio 2 prefilters");
        assert_eq!(k, 2);
        assert_eq!(reduced.dim(), (128, 128));
        let adjusted = wcs(&adjusted_header);
        for (i, j) in [(0usize, 0usize), (37, 91), (127, 127), (64, 10)] {
            let got = adjusted.pixel_to_world(i as f64, j as f64);
            let want = tgt_wcs.pixel_to_world(2.0 * i as f64 + 0.5, 2.0 * j as f64 + 0.5);
            let sep = sky_separation_deg((got.ra, got.dec), (want.ra, want.dec));
            assert!(sep < 1e-9, "reduced ({i},{j}): {sep} deg");
        }

        let (out, report) = reproject_to_grid(&reduced, &adjusted, &ref_wcs, 128, 128);
        assert!(report.coverage > 0.9, "{}", report.coverage);
        for (i, &(x, y, _)) in stars.iter().enumerate() {
            let flux_target = window_sum(&target, x, y, 5, 0.0);
            let sky = tgt_wcs.pixel_to_world(x, y);
            let (rx, ry) = ref_wcs.world_to_pixel(sky.ra, sky.dec);
            let aperture = window_sum(&out, rx, ry, 2, 0.0);
            let expected = flux_target / 4.0;
            let rel = (aperture - expected).abs() / expected;
            assert!(rel < 0.03, "star {i} at ({x:.2},{y:.2}): aperture {aperture} vs {expected} ({rel:.4})");
        }
    }

    #[test]
    fn reference_without_overlap_yields_all_nan_and_zero_coverage() {
        let reference = tan_header((128, 128), (64.5, 64.5), (83.0, 22.0), rotated_cd(0.2, 0.0));
        let target = tan_header((128, 128), (64.5, 64.5), (83.0, 23.0), rotated_cd(0.2, 0.0));
        let image = Array2::from_elem((128, 128), 5.0f32);
        let (out, report) = reproject_to_grid(&image, &wcs(&target), &wcs(&reference), 128, 128);
        assert!(out.iter().all(|v| v.is_nan()));
        assert_eq!(report.coverage, 0.0);
    }

    #[test]
    fn prefilter_rescales_sip_so_block_centres_keep_their_sky_positions() {
        let mut tgt_header = tan_header((256, 256), (128.5, 128.5), (83.0, 22.0), rotated_cd(0.1, 0.0));
        tgt_header.set("CTYPE1", "RA---TAN-SIP".to_string());
        tgt_header.set("CTYPE2", "DEC--TAN-SIP".to_string());
        tgt_header.set_f64("A_ORDER", 2.0);
        tgt_header.set_f64("B_ORDER", 2.0);
        tgt_header.set_f64("A_2_0", 2.0e-5);
        tgt_header.set_f64("A_1_1", -1.0e-5);
        tgt_header.set_f64("B_0_2", 3.0e-5);
        tgt_header.set_f64("A_DMAX", 1.5);
        let original = wcs(&tgt_header);
        let image = Array2::zeros((256, 256));
        let (_, adjusted_header, k) = prefilter_coarse_target(&image, &tgt_header, 2.0).unwrap();
        assert_eq!(k, 2);
        assert_eq!(adjusted_header.get_i64("NAXIS1"), Some(128));
        assert_eq!(adjusted_header.get_i64("NAXIS2"), Some(128));
        assert!((adjusted_header.get_f64("A_DMAX").unwrap() - 0.75).abs() < 1e-12);
        let adjusted = wcs(&adjusted_header);
        for (i, j) in [(0usize, 0usize), (5, 120), (100, 100), (127, 3)] {
            let got = adjusted.pixel_to_world(i as f64, j as f64);
            let want = original.pixel_to_world(2.0 * i as f64 + 0.5, 2.0 * j as f64 + 0.5);
            let sep = sky_separation_deg((got.ra, got.dec), (want.ra, want.dec));
            assert!(sep < 1e-9, "reduced ({i},{j}): {sep} deg");
        }
    }

    fn random_stars(rows: usize, cols: usize, count: usize, seed: u64) -> Vec<(f64, f64, f64)> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..count)
            .map(|_| (rng.gen_range(20.0..cols as f64 - 20.0), rng.gen_range(20.0..rows as f64 - 20.0), rng.gen_range(400.0..2000.0)))
            .collect()
    }

    fn with_noise(img: Array2<f32>, amplitude: f32, seed: u64) -> Array2<f32> {
        let mut rng = StdRng::seed_from_u64(seed);
        img.mapv(|v| v + rng.gen_range(-amplitude..amplitude))
    }

    fn moved(stars: &[(f64, f64, f64)], dy: f64, dx: f64) -> Vec<(f64, f64, f64)> {
        stars.iter().map(|&(x, y, peak)| (x + dx, y + dy, peak)).collect()
    }

    #[test]
    fn point_sources_measure_the_residual_of_a_reprojection_whatever_the_seed() {
        let stars = random_stars(600, 600, 40, 11);
        let reference = with_noise(render_stars(600, 600, 100.0, &stars, 1.0), 2.0, 1);
        let reprojected = with_noise(render_stars(600, 600, 100.0, &moved(&stars, 0.6, -1.3), 2.0), 2.0, 2);

        let prior = point_source_residual(&reference, &reprojected, &[(0.0, 0.0)]).expect("40 point sources");
        assert!((prior.dy - 0.6).abs() < 0.05 && (prior.dx + 1.3).abs() < 0.05, "{prior:?}");
        assert!(prior.agreeing >= 30 && prior.agreeing <= prior.matched, "{prior:?}");
        assert!(prior.rms_px < 0.2, "{prior:?}");

        let spurious = point_source_residual(&reference, &reprojected, &[(0.0, 0.0), (-1.694, -1.309)]).expect("40 point sources");
        assert!((spurious.dy - 0.6).abs() < 0.05 && (spurious.dx + 1.3).abs() < 0.05, "a wrong seed does not move the point sources: {spurious:?}");
    }

    #[test]
    fn a_residual_beyond_the_match_radius_is_found_only_from_its_seed() {
        let stars = random_stars(600, 600, 40, 12);
        let reference = with_noise(render_stars(600, 600, 100.0, &stars, 1.0), 2.0, 3);
        let reprojected = with_noise(render_stars(600, 600, 100.0, &moved(&stars, 0.0, 8.2), 2.0), 2.0, 4);

        assert_eq!(point_source_residual(&reference, &reprojected, &[(0.0, 0.0)]), None);
        let seeded = point_source_residual(&reference, &reprojected, &[(0.0, 0.0), (0.3, 7.6)]).expect("seeded match");
        assert!(seeded.dy.abs() < 0.05 && (seeded.dx - 8.2).abs() < 0.05, "{seeded:?}");
    }

    #[test]
    fn extended_emission_with_three_point_sources_is_not_a_measurement() {
        let nebula = |dx: f64| {
            Array2::from_shape_fn((300, 300), |(y, x)| {
                let d2 = (x as f64 - 150.0 - dx).powi(2) + (y as f64 - 140.0).powi(2);
                (100.0 + 800.0 * (-d2 / (2.0 * 60.0 * 60.0)).exp()) as f32
            })
        };
        let few = [(40.0, 40.0, 1500.0), (250.0, 60.0, 1500.0), (60.0, 250.0, 1500.0)];
        let mut reference = render_stars(300, 300, 0.0, &few, 1.0);
        reference += &nebula(0.0);
        let mut reprojected = render_stars(300, 300, 0.0, &few, 2.0);
        reprojected += &nebula(2.5);

        assert_eq!(point_source_residual(&with_noise(reference, 2.0, 5), &with_noise(reprojected, 2.0, 6), &[(0.0, 0.0), (0.0, 2.5)]), None);
    }

    #[test]
    fn prefilter_block_size_follows_the_ratio() {
        let image = Array2::from_elem((7, 9), 1.0f32);
        let h = tan_header((9, 7), (4.0, 3.0), (83.0, 22.0), rotated_cd(0.1, 0.0));
        assert!(prefilter_coarse_target(&image, &h, 1.49).is_none());
        assert!(prefilter_coarse_target(&image, &h, f64::NAN).is_none());
        assert_eq!(prefilter_coarse_target(&image, &h, 1.5).map(|r| r.2), Some(2));
        assert_eq!(prefilter_coarse_target(&image, &h, 2.9).map(|r| r.2), Some(2));
        let (reduced, adjusted, k) = prefilter_coarse_target(&image, &h, 3.0).unwrap();
        assert_eq!(k, 3);
        assert_eq!(reduced.dim(), (2, 3));
        assert!(reduced.iter().all(|v| (v - 1.0).abs() < 1e-6));
        assert_eq!(adjusted.get_i64("NAXIS1"), Some(3));
        assert_eq!(adjusted.get_i64("NAXIS2"), Some(2));
        assert!((adjusted.get_f64("CRPIX1").unwrap() - ((4.0 - 0.5) / 3.0 + 0.5)).abs() < 1e-12);
        assert!((adjusted.get_f64("CRPIX2").unwrap() - ((3.0 - 0.5) / 3.0 + 0.5)).abs() < 1e-12);
        assert!((adjusted.get_f64("CD1_1").unwrap() - 3.0 * h.get_f64("CD1_1").unwrap()).abs() < 1e-18);
        assert!(prefilter_coarse_target(&image, &h, 8.0).is_none());
    }
}
