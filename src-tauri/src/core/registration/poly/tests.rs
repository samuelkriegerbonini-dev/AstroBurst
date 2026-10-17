use std::ops::RangeInclusive;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::*;
use crate::core::astrometry::gwcs::sip_fit::NativePlane;
use crate::core::astrometry::gwcs::{GwcsError, SipFit, SipFitOptions};
use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
use crate::types::HduHeader;

type Pair = (f64, f64, f64, f64);
type SipExport =
    fn(&PolyModel, &TanParams, [[f64; 2]; 2], &SipFitOptions) -> Result<SipFit, GwcsError>;

const SEEDS: [u64; 8] = [1, 2, 3, 5, 8, 13, 21, 34];

fn term_count(order: u8) -> usize {
    let n = order as usize;
    (n + 1) * (n + 2) / 2
}

struct Field {
    amplitudes: Vec<f64>,
    xc: f64,
    yc: f64,
    scale: f64,
}

impl Field {
    fn new(amplitudes: &[f64], width: f64, height: f64) -> Self {
        Self {
            amplitudes: amplitudes.to_vec(),
            xc: 0.43 * width,
            yc: 0.57 * height,
            scale: 0.6 * width.max(height),
        }
    }

    fn weight(degree: usize, j: usize, axis: usize) -> f64 {
        let magnitude = 0.4 + 0.06 * ((degree * 31 + j * 17 + axis * 7) % 11) as f64;
        if (degree + j + axis).is_multiple_of(3) {
            -magnitude
        } else {
            magnitude
        }
    }

    fn at(&self, x: f64, y: f64) -> (f64, f64) {
        let t = (x - self.xc) / self.scale;
        let s = (y - self.yc) / self.scale;
        let mut dx = 0.0;
        let mut dy = 0.0;
        for (degree, amplitude) in self.amplitudes.iter().enumerate() {
            for j in 0..=degree {
                let i = degree - j;
                let monomial = t.powi(i as i32) * s.powi(j as i32);
                dx += amplitude * Self::weight(degree, j, 0) * monomial;
                dy += amplitude * Self::weight(degree, j, 1) * monomial;
            }
        }
        (x + dx, y + dy)
    }
}

fn radial_field(k_px: f64, width: f64, height: f64) -> impl Fn(f64, f64) -> (f64, f64) {
    let (xc, yc) = ((width - 1.0) / 2.0, (height - 1.0) / 2.0);
    let scale = 0.5 * width.max(height);
    move |x, y| {
        let t = (x - xc) / scale;
        let s = (y - yc) / scale;
        let r2 = t * t + s * s;
        (x + 4.2 + k_px * r2 * t, y - 1.3 + k_px * r2 * s)
    }
}

fn scattered_points(seed: u64, n: usize, width: f64, height: f64) -> Vec<(f64, f64)> {
    let mut rng = StdRng::seed_from_u64(seed);
    let (xmax, ymax) = (width - 1.0, height - 1.0);
    let mut points = vec![(0.0, 0.0), (xmax, 0.0), (0.0, ymax), (xmax, ymax)];
    while points.len() < n {
        points.push((rng.gen::<f64>() * xmax, rng.gen::<f64>() * ymax));
    }
    points.truncate(n);
    points
}

fn gaussian(rng: &mut StdRng) -> f64 {
    let u1 = 1.0 - rng.gen::<f64>();
    let u2 = rng.gen::<f64>();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn pairs_through(points: &[(f64, f64)], map: impl Fn(f64, f64) -> (f64, f64)) -> Vec<Pair> {
    points
        .iter()
        .map(|&(x, y)| {
            let (u, v) = map(x, y);
            (x, y, u, v)
        })
        .collect()
}

fn with_noise(pairs: &[Pair], sigma_px: f64, seed: u64) -> Vec<Pair> {
    let mut rng = StdRng::seed_from_u64(seed ^ 0x9e37_79b9_7f4a_7c15);
    pairs
        .iter()
        .map(|&(x, y, u, v)| {
            (
                x,
                y,
                u + sigma_px * gaussian(&mut rng),
                v + sigma_px * gaussian(&mut rng),
            )
        })
        .collect()
}

fn swapped(pairs: &[Pair]) -> Vec<Pair> {
    pairs.iter().map(|&(x, y, u, v)| (u, v, x, y)).collect()
}

fn grid(width: f64, height: f64, n: usize) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(n * n);
    for row in 0..n {
        for col in 0..n {
            let x = (width - 1.0) * col as f64 / (n - 1) as f64;
            let y = (height - 1.0) * row as f64 / (n - 1) as f64;
            out.push((x, y));
        }
    }
    out
}

fn max_model_error(
    model: &PolyModel,
    points: &[(f64, f64)],
    truth: impl Fn(f64, f64) -> (f64, f64),
) -> f64 {
    points
        .iter()
        .map(|&(x, y)| {
            let (ex, ey) = truth(x, y);
            let (gx, gy) = model.eval(x, y);
            (gx - ex).hypot(gy - ey)
        })
        .fold(0.0, |m, d| if d.is_nan() || d > m { d } else { m })
}

fn holdout_rms_by_hand(pairs: &[Pair], order: u8, folds: usize) -> f64 {
    let fold_of = fold_assignment(pairs.len(), folds);
    let mut sum = 0.0;
    for fold in 0..folds {
        let training: Vec<Pair> = pairs
            .iter()
            .zip(&fold_of)
            .filter(|(_, &f)| f != fold)
            .map(|(p, _)| *p)
            .collect();
        let held: Vec<Pair> = pairs
            .iter()
            .zip(&fold_of)
            .filter(|(_, &f)| f == fold)
            .map(|(p, _)| *p)
            .collect();
        let model = fit(&training, order)
            .unwrap_or_else(|| panic!("fold {fold} of order {order} did not fit"));
        sum += model.residuals(&held).iter().map(|r| r * r).sum::<f64>();
    }
    (sum / pairs.len() as f64).sqrt()
}

fn bic_by_hand(pairs: &[Pair], order: u8) -> f64 {
    let model = fit(pairs, order).unwrap_or_else(|| panic!("order {order} did not fit"));
    let rss: f64 = model.residuals(pairs).iter().map(|r| r * r).sum();
    let n = pairs.len() as f64;
    n * (rss / n).ln() + (2 * term_count(order)) as f64 * n.ln()
}

fn replay_selection(considered: &[(u8, f64, f64)]) -> Option<u8> {
    let mut validated = considered.iter().copied().filter(|c| c.1.is_finite());
    let mut best = validated.next()?;
    for (order, holdout, bic) in validated {
        let gain = best.1 - holdout;
        let relative_ok = holdout <= (1.0 - HOLDOUT_MIN_RELATIVE_GAIN) * best.1;
        if gain >= HOLDOUT_MIN_ABSOLUTE_GAIN_PX && relative_ok && bic < best.2 {
            best = (order, holdout, bic);
        }
    }
    Some(best.0)
}

fn considered_entry(selection: &PolySelection, order: u8) -> (u8, f64, f64) {
    *selection
        .considered
        .iter()
        .find(|c| c.0 == order)
        .unwrap_or_else(|| panic!("order {order} missing from {:?}", selection.considered))
}

fn select(pairs: &[Pair], orders: RangeInclusive<u8>, what: &str) -> PolySelection {
    let selection =
        fit_select(pairs, orders, 5).unwrap_or_else(|| panic!("{what}: fit_select refused"));
    let replayed = replay_selection(&selection.considered);
    assert_eq!(
        Some(selection.model.order),
        replayed,
        "{what}: chosen order disagrees with the documented rule over {:?}",
        selection.considered
    );
    selection
}

fn set_cards(header: &mut HduHeader, cards: &[(&str, String)]) {
    for (key, value) in cards {
        header.set(key, value.clone());
    }
}

fn tan_header(tan: &TanParams, naxis: (usize, usize)) -> HduHeader {
    let mut h = HduHeader::empty();
    set_cards(
        &mut h,
        &[
            ("NAXIS", "2".to_string()),
            ("NAXIS1", naxis.0.to_string()),
            ("NAXIS2", naxis.1.to_string()),
            ("WCSAXES", "2".to_string()),
            ("CTYPE1", "RA---TAN".to_string()),
            ("CTYPE2", "DEC--TAN".to_string()),
            ("CUNIT1", "deg".to_string()),
            ("CUNIT2", "deg".to_string()),
            ("CRPIX1", tan.crpix.0.to_string()),
            ("CRPIX2", tan.crpix.1.to_string()),
            ("CRVAL1", tan.crval.0.to_string()),
            ("CRVAL2", tan.crval.1.to_string()),
            ("CD1_1", tan.cd[0][0].to_string()),
            ("CD1_2", tan.cd[0][1].to_string()),
            ("CD2_1", tan.cd[1][0].to_string()),
            ("CD2_2", tan.cd[1][1].to_string()),
            ("LONPOLE", "180".to_string()),
            ("RADESYS", "ICRS".to_string()),
        ],
    );
    h
}

fn sip_header(fit: &SipFit, naxis: (usize, usize)) -> HduHeader {
    let mut h = HduHeader::empty();
    h.set("NAXIS", "2".to_string());
    h.set("NAXIS1", naxis.0.to_string());
    h.set("NAXIS2", naxis.1.to_string());
    for (k, v) in fit.header_cards() {
        h.set(&k, v);
    }
    h
}

fn field_tan() -> TanParams {
    let scale_deg = 1.2 / 3600.0;
    let (sin, cos) = 20f64.to_radians().sin_cos();
    TanParams {
        crval: (83.82, -5.39),
        crpix: (1024.5, 768.5),
        cd: [
            [-scale_deg * cos, scale_deg * sin],
            [scale_deg * sin, scale_deg * cos],
        ],
    }
}

fn plate_scale_deg(tan: &TanParams) -> f64 {
    (tan.cd[0][0] * tan.cd[1][1] - tan.cd[0][1] * tan.cd[1][0])
        .abs()
        .sqrt()
}

struct SkyRoundTrip {
    forward_px: f64,
    inverse_px: f64,
}

fn sky_round_trip(
    exported: &WcsTransform,
    reference: &WcsTransform,
    truth: impl Fn(f64, f64) -> (f64, f64),
    points: &[(f64, f64)],
    scale_deg: f64,
) -> SkyRoundTrip {
    let mut out = SkyRoundTrip {
        forward_px: 0.0,
        inverse_px: 0.0,
    };
    for &(x, y) in points {
        let (ix, iy) = truth(x, y);
        let expected = reference.pixel_to_world(ix, iy);
        let got = exported.pixel_to_world(x, y);
        let forward = angular_separation(got.ra, got.dec, expected.ra, expected.dec) / scale_deg;
        assert!(
            forward.is_finite(),
            "forward at ({x}, {y}): {got:?} vs {expected:?}"
        );
        out.forward_px = out.forward_px.max(forward);
        let (bx, by) = exported.world_to_pixel(expected.ra, expected.dec);
        let inverse = (bx - x).hypot(by - y);
        assert!(
            inverse.is_finite(),
            "inverse at ({x}, {y}) gave ({bx}, {by})"
        );
        out.inverse_px = out.inverse_px.max(inverse);
    }
    out
}

#[test]
fn the_public_api_has_the_shape_the_plan_names() {
    let _fit: fn(&[Pair], u8) -> Option<PolyModel> = fit;
    let _fit_pair: fn(&[Pair], u8) -> Option<PolyPair> = fit_pair;
    let _fit_select: fn(&[Pair], RangeInclusive<u8>, usize) -> Option<PolySelection> = fit_select;
    let _eval: fn(&PolyModel, f64, f64) -> (f64, f64) = PolyModel::eval;
    let _residuals: fn(&PolyModel, &[Pair]) -> Vec<f64> = PolyModel::residuals;
    let _quadrants: fn(&PolyModel, &[Pair]) -> [Option<f64>; 4] = PolyModel::max_quadrant_residual;
    let _sip: SipExport = PolyModel::to_sip_cards;
    let model = PolyModel {
        order: 1,
        norm: Normalisation {
            x0: 0.0,
            y0: 0.0,
            sx: 1.0,
            sy: 1.0,
        },
        cx: vec![0.0, 1.0, 0.0],
        cy: vec![0.0, 0.0, 1.0],
    };
    let pair = PolyPair {
        forward: model.clone(),
        inverse: model.clone(),
    };
    let selection = PolySelection {
        model: pair.forward.clone(),
        holdout_rms_px: 0.0,
        considered: vec![(1u8, 0.0f64, 0.0f64)],
    };
    let tan = TanParams {
        crval: (0.0, 0.0),
        crpix: (1.0, 1.0),
        cd: [[1.0, 0.0], [0.0, 1.0]],
    };
    assert_eq!(selection.model, pair.inverse);
    assert_eq!(tan.crpix, (1.0, 1.0));
    assert_eq!(MIN_PAIRS_PER_TERM, 4);
    assert_eq!(HOLDOUT_MIN_RELATIVE_GAIN, 0.10);
    assert_eq!(HOLDOUT_MIN_ABSOLUTE_GAIN_PX, 0.03);
}

#[test]
fn a_hand_built_model_evaluates_its_monomials_in_graded_order() {
    let norm = Normalisation {
        x0: 100.0,
        y0: 50.0,
        sx: 10.0,
        sy: 5.0,
    };
    let order_2 = PolyModel {
        order: 2,
        norm,
        cx: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        cy: vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    };
    let (gx, gy) = order_2.eval(115.0, 52.5);
    assert!(
        (gx - 19.75).abs() <= 1e-12,
        "1 + 2u + 3v + 4u^2 + 5uv + 6v^2 at u=1.5, v=0.5 gave {gx}"
    );
    assert!((gy - 0.25).abs() <= 1e-12, "v^2 at v=0.5 gave {gy}");
    let mut cy = vec![0.0; 10];
    cy[7] = 1.0;
    let order_3 = PolyModel {
        order: 3,
        norm,
        cx: vec![0.0; 10],
        cy,
    };
    let (gx, gy) = order_3.eval(115.0, 52.5);
    assert!(gx.abs() <= 1e-12, "{gx}");
    assert!(
        (gy - 1.125).abs() <= 1e-12,
        "index 7 must be u^2 v (graded: 1, u, v, u2, uv, v2, u3, u2v, uv2, v3), got {gy}"
    );
}

#[test]
fn a_malformed_hand_built_model_evaluates_to_nan_instead_of_panicking() {
    let norm = Normalisation {
        x0: 0.0,
        y0: 0.0,
        sx: 1.0,
        sy: 1.0,
    };
    let short = PolyModel {
        order: 3,
        norm,
        cx: vec![1.0, 2.0],
        cy: vec![1.0; 10],
    };
    let zero_scale = PolyModel {
        order: 1,
        norm: Normalisation { sx: 0.0, ..norm },
        cx: vec![0.0, 1.0, 0.0],
        cy: vec![0.0, 0.0, 1.0],
    };
    let huge_order = PolyModel {
        order: u8::MAX,
        norm,
        cx: vec![1.0; 3],
        cy: vec![1.0; 3],
    };
    let pairs = [(1.0, 2.0, 1.0, 2.0), (-3.0, 4.0, -3.0, 4.0)];
    let tan = field_tan();
    let bbox = [[-0.5, 2047.5], [-0.5, 1535.5]];
    for (what, model) in [
        ("short coefficients", &short),
        ("zero scale", &zero_scale),
        ("order 255", &huge_order),
    ] {
        let (gx, gy) = model.eval(1.0, 2.0);
        assert!(gx.is_nan() && gy.is_nan(), "{what}: eval gave ({gx}, {gy})");
        let residuals = model.residuals(&pairs);
        assert_eq!(residuals.len(), pairs.len(), "{what}");
        assert!(
            residuals.iter().all(|r| r.is_nan()),
            "{what}: {residuals:?}"
        );
        let _ = model.max_quadrant_residual(&pairs);
        assert!(
            model
                .to_sip_cards(&tan, bbox, &SipFitOptions::default())
                .is_err(),
            "{what}: to_sip_cards accepted it"
        );
    }
}

#[test]
fn polynomials_of_orders_1_to_4_are_recovered_to_1e_9_relative_on_noiseless_pairs() {
    let (width, height): (f64, f64) = (4096.0, 3072.0);
    let extent = width.max(height);
    let check = grid(width, height, 25);
    for order in 1..=4u8 {
        let amplitudes = [3.7, 25.0, 6.0, 4.0, 2.5];
        let field = Field::new(&amplitudes[..=order as usize], width, height);
        let points = scattered_points(40 + order as u64, 300, width, height);
        let pairs = pairs_through(&points, |x, y| field.at(x, y));
        let model = fit(&pairs, order).unwrap_or_else(|| panic!("order {order} refused"));
        assert_eq!(model.order, order);
        assert_eq!(model.cx.len(), term_count(order), "order {order} cx terms");
        assert_eq!(model.cy.len(), term_count(order), "order {order} cy terms");
        let on_pairs = max_model_error(&model, &points, |x, y| field.at(x, y));
        let on_grid = max_model_error(&model, &check, |x, y| field.at(x, y));
        assert!(
            on_pairs <= 1e-9 * extent,
            "order {order}: {on_pairs:e} px on the pairs > {:e}",
            1e-9 * extent
        );
        assert!(
            on_grid <= 1e-9 * extent,
            "order {order}: {on_grid:e} px on the frame grid > {:e}",
            1e-9 * extent
        );
        let residuals = model.residuals(&pairs);
        assert_eq!(residuals.len(), pairs.len());
        assert!(
            residuals.iter().all(|r| *r <= 1e-9 * extent),
            "order {order} residuals"
        );
    }
}

#[test]
fn order_3_synthetic_mapping_is_recovered_to_1e_9_px() {
    let (width, height) = (2048.0, 2048.0);
    let field = Field::new(&[3.7, 25.0, 6.0, 4.0], width, height);
    let points = scattered_points(7, 200, width, height);
    let pairs = pairs_through(&points, |x, y| field.at(x, y));
    let model = fit(&pairs, 3).expect("order 3 fits 200 noiseless pairs");
    let on_pairs = max_model_error(&model, &points, |x, y| field.at(x, y));
    let on_grid = max_model_error(&model, &grid(width, height, 33), |x, y| field.at(x, y));
    assert!(on_pairs <= 1e-9, "{on_pairs:e} px on the pairs");
    assert!(on_grid <= 1e-9, "{on_grid:e} px on the frame grid");
}

#[test]
fn orders_2_to_4_stay_conditioned_on_a_6000_by_4000_frame() {
    let (width, height) = (6000.0, 4000.0);
    let check = grid(width, height, 31);
    for order in 2..=4u8 {
        let amplitudes = [-11.0, 40.0, 8.0, 5.0, 3.0];
        let field = Field::new(&amplitudes[..=order as usize], width, height);
        let points = scattered_points(60 + order as u64, 500, width, height);
        let pairs = pairs_through(&points, |x, y| field.at(x, y));
        let model = fit(&pairs, order)
            .unwrap_or_else(|| panic!("order {order} refused on a 6000 x 4000 frame"));
        let error = max_model_error(&model, &check, |x, y| field.at(x, y));
        assert!(
            error <= 1e-9 * width,
            "order {order}: {error:e} px > {:e} px",
            1e-9 * width
        );
    }
}

#[test]
fn normalised_coordinates_of_the_fitted_points_span_minus_one_to_one() {
    let (width, height) = (6000.0, 4000.0);
    let field = Field::new(&[1.0, 10.0, 3.0, 2.0, 1.0], width, height);
    let points: Vec<(f64, f64)> = scattered_points(9, 400, width, height)
        .into_iter()
        .map(|(x, y)| (x + 20000.0, y + 15000.0))
        .collect();
    let pairs = pairs_through(&points, |x, y| field.at(x - 20000.0, y - 15000.0));
    let model = fit(&pairs, 4).expect("order 4 fits an offset 6000 x 4000 canvas");
    let Normalisation { x0, y0, sx, sy } = model.norm;
    assert!(sx > 0.0 && sy > 0.0, "{:?}", model.norm);
    let us: Vec<f64> = points.iter().map(|p| (p.0 - x0) / sx).collect();
    let vs: Vec<f64> = points.iter().map(|p| (p.1 - y0) / sy).collect();
    for (axis, values) in [("u", &us), ("v", &vs)] {
        let lo = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (lo + 1.0).abs() <= 1e-12 && (hi - 1.0).abs() <= 1e-12,
            "{axis} spans [{lo}, {hi}], expected [-1, 1]"
        );
    }
    let error = max_model_error(&model, &points, |x, y| field.at(x - 20000.0, y - 15000.0));
    assert!(error <= 1e-9 * width, "offset canvas: {error:e} px");
}

#[test]
fn too_few_pairs_for_the_order_are_refused() {
    let (width, height) = (3000.0, 2000.0);
    for order in 1..=4u8 {
        let field = Field::new(
            &[2.0, 15.0, 3.0, 2.0, 1.0][..=order as usize],
            width,
            height,
        );
        let enough = MIN_PAIRS_PER_TERM * term_count(order);
        let points = scattered_points(80 + order as u64, enough, width, height);
        let pairs = pairs_through(&points, |x, y| field.at(x, y));
        assert!(
            fit(&pairs, order).is_some(),
            "order {order} with {enough} pairs (terms x 4) must fit"
        );
        assert!(
            fit_pair(&pairs, order).is_some(),
            "fit_pair order {order} with {enough} pairs"
        );
        let short = &pairs[..enough - 1];
        assert!(
            fit(short, order).is_none(),
            "order {order} with {} pairs must be refused",
            enough - 1
        );
        assert!(
            fit_pair(short, order).is_none(),
            "fit_pair order {order} with {} pairs",
            enough - 1
        );
        assert!(
            fit_select(short, order..=order, 5).is_none(),
            "fit_select order {order} with {} pairs",
            enough - 1
        );
    }
    assert!(fit(&[], 1).is_none());
    assert!(fit_select(&[], 1..=3, 5).is_none());
}

#[test]
fn order_zero_and_absurd_orders_are_refused_without_panicking() {
    let field = Field::new(&[1.0, 10.0, 2.0, 1.0], 1000.0, 1000.0);
    let pairs = pairs_through(&scattered_points(3, 2000, 1000.0, 1000.0), |x, y| {
        field.at(x, y)
    });
    assert!(
        fit(&pairs, 0).is_none(),
        "order 0 is not a registration model"
    );
    assert!(fit(&pairs, u8::MAX).is_none());
    assert!(fit_pair(&pairs, u8::MAX).is_none());
    let selection =
        fit_select(&pairs, 0..=2, 5).expect("order 0 is skipped, 1 and 2 are considered");
    let orders: Vec<u8> = selection.considered.iter().map(|c| c.0).collect();
    assert_eq!(orders, vec![1, 2]);
    let few = &pairs[..40];
    let selection = fit_select(few, 1..=u8::MAX, 5).expect("orders up to the cap are considered");
    let orders: Vec<u8> = selection.considered.iter().map(|c| c.0).collect();
    assert_eq!(
        orders,
        vec![1, 2, 3],
        "40 pairs cap the order at 3 (10 terms x 4)"
    );
}

#[test]
fn collinear_pairs_return_none() {
    let mut rng = StdRng::seed_from_u64(11);
    let line: Vec<Pair> = (0..300)
        .map(|_| {
            let y = rng.gen::<f64>() * 2000.0;
            let x = 1.7 * y + 13.2;
            (x, y, x + 0.25, y - 0.5)
        })
        .collect();
    for order in 1..=3u8 {
        assert!(
            fit(&line, order).is_none(),
            "order {order} on a slanted line"
        );
        assert!(
            fit_pair(&line, order).is_none(),
            "fit_pair order {order} on a slanted line"
        );
    }
    assert!(
        fit_select(&line, 1..=3, 5).is_none(),
        "fit_select on a slanted line"
    );
    let column: Vec<Pair> = (0..300)
        .map(|k| (512.0, k as f64 * 6.5, 513.0, k as f64 * 6.5 + 2.0))
        .collect();
    assert!(
        fit(&column, 1).is_none(),
        "points on one column have no x extent"
    );
    assert!(fit(&column, 2).is_none());
}

#[test]
fn points_on_two_columns_fit_order_1_but_not_order_2() {
    let mut rng = StdRng::seed_from_u64(12);
    let pairs: Vec<Pair> = (0..200)
        .map(|k| {
            let x = if k % 2 == 0 { 100.0 } else { 1900.0 };
            let y = rng.gen::<f64>() * 1500.0;
            (x, y, 0.999 * x - 0.02 * y + 3.0, 0.02 * x + 0.999 * y - 1.0)
        })
        .collect();
    assert!(
        fit(&pairs, 1).is_some(),
        "two columns determine an affine model"
    );
    assert!(
        fit(&pairs, 2).is_none(),
        "u^2 is constant on two columns, order 2 is rank deficient"
    );
}

#[test]
fn non_finite_inputs_are_refused() {
    let points = scattered_points(13, 120, 2000.0, 2000.0);
    let field = Field::new(&[1.0, 10.0, 2.0], 2000.0, 2000.0);
    let good = pairs_through(&points, |x, y| field.at(x, y));
    assert!(fit(&good, 2).is_some());
    for slot in 0..4 {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut pairs = good.clone();
            let p = &mut pairs[57];
            match slot {
                0 => p.0 = bad,
                1 => p.1 = bad,
                2 => p.2 = bad,
                _ => p.3 = bad,
            }
            assert!(
                fit(&pairs, 2).is_none(),
                "fit accepted {bad} in slot {slot}"
            );
            assert!(
                fit_pair(&pairs, 2).is_none(),
                "fit_pair accepted {bad} in slot {slot}"
            );
            assert!(
                fit_select(&pairs, 1..=3, 5).is_none(),
                "fit_select accepted {bad} in slot {slot}"
            );
        }
    }
}

#[test]
fn residuals_are_euclidean_distances_in_pair_order() {
    let points = scattered_points(14, 100, 2000.0, 1500.0);
    let field = Field::new(&[2.0, 12.0, 3.0], 2000.0, 1500.0);
    let pairs = pairs_through(&points, |x, y| field.at(x, y));
    let model = fit(&pairs, 2).expect("order 2 fits");
    let mut probe = pairs.clone();
    probe[42].2 += 0.3;
    probe[42].3 -= 0.4;
    let residuals = model.residuals(&probe);
    assert_eq!(residuals.len(), probe.len());
    for (k, r) in residuals.iter().enumerate() {
        let expected = if k == 42 { 0.5 } else { 0.0 };
        assert!(
            (r - expected).abs() <= 1e-9,
            "residual {k} = {r}, expected {expected}"
        );
    }
}

#[test]
fn max_quadrant_residual_reports_the_worst_corner() {
    let (width, height) = (2000.0, 1600.0);
    let field = Field::new(&[2.0, 12.0, 3.0], width, height);
    let points = scattered_points(15, 200, width, height);
    let model = fit(&pairs_through(&points, |x, y| field.at(x, y)), 2).expect("order 2 fits");
    let Normalisation { x0, y0, .. } = model.norm;
    assert!(
        (x0 - (width - 1.0) / 2.0).abs() <= 1e-9 && (y0 - (height - 1.0) / 2.0).abs() <= 1e-9,
        "{:?}",
        model.norm
    );
    let exact = |x: f64, y: f64| {
        let (u, v) = field.at(x, y);
        (x, y, u, v)
    };
    let shifted = |x: f64, y: f64, dx: f64, dy: f64| {
        let (u, v) = field.at(x, y);
        (x, y, u + dx, v + dy)
    };
    let probe = vec![
        exact(100.0, 100.0),
        shifted(300.0, 200.0, 0.18, -0.24),
        exact(1500.0, 300.0),
        exact(1900.0, 50.0),
        shifted(200.0, 1500.0, 0.06, 0.08),
        exact(1800.0, 1200.0),
        shifted(1950.0, 1550.0, -0.48, 0.64),
        shifted(1200.0, 900.0, 0.3, 0.4),
    ];
    let quadrants = model.max_quadrant_residual(&probe);
    let expected = [0.3, 0.0, 0.1, 0.8];
    for (index, (got, want)) in quadrants.iter().zip(expected).enumerate() {
        let got = got.unwrap_or_else(|| panic!("quadrant {index} has pairs but reported None"));
        assert!(
            (got - want).abs() <= 1e-9,
            "quadrant {index}: {got} vs {want} (index = (x >= x0) + 2 (y >= y0))"
        );
    }
    let worst = quadrants
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.unwrap_or(0.0).total_cmp(&b.1.unwrap_or(0.0)))
        .map(|(i, _)| i);
    assert_eq!(
        worst,
        Some(3),
        "the upper-right corner (x >= x0, y >= y0) is the worst"
    );
    let left_only = vec![exact(100.0, 100.0), shifted(150.0, 1500.0, 0.0, 0.2)];
    let quadrants = model.max_quadrant_residual(&left_only);
    assert!(
        quadrants[1].is_none() && quadrants[3].is_none(),
        "empty quadrants report None: {quadrants:?}"
    );
    assert!(quadrants[0].is_some_and(|r| r <= 1e-9), "{quadrants:?}");
    assert!(
        quadrants[2].is_some_and(|r| (r - 0.2).abs() <= 1e-9),
        "{quadrants:?}"
    );
}

#[test]
fn fit_pair_fits_forward_on_the_pairs_and_inverse_on_the_swapped_pairs() {
    let points = scattered_points(16, 150, 2048.0, 2048.0);
    let field = Field::new(&[5.0, 30.0, 2.0, 1.0], 2048.0, 2048.0);
    let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.05, 16);
    let pair = fit_pair(&pairs, 3).expect("order 3 pair fits");
    assert_eq!(pair.forward, fit(&pairs, 3).expect("forward"));
    assert_eq!(pair.inverse, fit(&swapped(&pairs), 3).expect("inverse"));
    assert_eq!(pair.inverse.order, 3);
}

#[test]
fn an_affine_pair_inverts_exactly() {
    let (sin, cos) = 37f64.to_radians().sin_cos();
    let affine = |x: f64, y: f64| {
        (
            12.5 + 1.003 * (cos * x - sin * y),
            -7.25 + 1.003 * (sin * x + cos * y),
        )
    };
    let points = scattered_points(17, 100, 3000.0, 2000.0);
    let pairs = pairs_through(&points, affine);
    let pair = fit_pair(&pairs, 1).expect("affine pair");
    for (x, y) in grid(3000.0, 2000.0, 11) {
        let (u, v) = pair.forward.eval(x, y);
        let (bx, by) = pair.inverse.eval(u, v);
        assert!(
            (bx - x).hypot(by - y) <= 1e-9 * 3000.0,
            "({x}, {y}) -> ({u}, {v}) -> ({bx}, {by})"
        );
    }
}

#[test]
fn fit_pair_inverse_composes_to_identity_within_1e_6_px() {
    let (width, height) = (2048.0, 2048.0);
    let (sin, cos) = 37f64.to_radians().sin_cos();
    let quadratic = Field::new(&[0.0, 0.0, 0.2], width, height);
    let map = |x: f64, y: f64| {
        let (qx, qy) = quadratic.at(x, y);
        (
            12.5 + 1.003 * (cos * qx - sin * qy),
            -7.25 + 1.003 * (sin * qx + cos * qy),
        )
    };
    let points = scattered_points(18, 400, width, height);
    let pairs = pairs_through(&points, map);
    let pair = fit_pair(&pairs, 3).expect("order 3 pair");
    let mut worst: f64 = 0.0;
    for (x, y) in grid(width, height, 21) {
        let (u, v) = pair.forward.eval(x, y);
        let (bx, by) = pair.inverse.eval(u, v);
        worst = worst.max((bx - x).hypot(by - y));
    }
    assert!(
        worst <= 1e-6,
        "inverse(forward(p)) - p reached {worst:e} px"
    );
}

#[test]
fn fit_select_keeps_order_2_on_30_noisy_pairs_of_a_quadratic_field() {
    for seed in SEEDS {
        let field = Field::new(&[3.0, 20.0, 3.0], 2000.0, 2000.0);
        let points = scattered_points(100 + seed, 30, 2000.0, 2000.0);
        let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.05, seed);
        let selection = select(&pairs, 1..=4, &format!("seed {seed}"));
        assert_eq!(
            selection.model.order, 2,
            "seed {seed}: {:?}",
            selection.considered
        );
        let orders: Vec<u8> = selection.considered.iter().map(|c| c.0).collect();
        assert_eq!(
            orders,
            vec![1, 2],
            "seed {seed}: orders 3 and 4 need 40 and 60 pairs"
        );
    }
}

#[test]
fn fit_select_picks_the_true_order_1_to_4_under_gaussian_noise_for_eight_seeds() {
    let (width, height) = (3000.0, 2000.0);
    for order in 1..=4u8 {
        for seed in SEEDS {
            let amplitudes = [3.7, 25.0, 2.0, 2.0, 2.0];
            let field = Field::new(&amplitudes[..=order as usize], width, height);
            let points = scattered_points(200 + seed, 200, width, height);
            let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.05, seed);
            let what = format!("true order {order}, seed {seed}");
            let selection = select(&pairs, 1..=5, &what);
            assert_eq!(
                selection.model.order, order,
                "{what}: {:?}",
                selection.considered
            );
            assert!(
                selection.holdout_rms_px < 0.1,
                "{what}: holdout {} px for 0.05 px noise",
                selection.holdout_rms_px
            );
        }
    }
}

#[test]
fn fit_select_raises_to_order_4_only_when_holdout_improves_by_10_percent_and_0_03_px() {
    let (width, height) = (3000.0, 3000.0);
    for seed in SEEDS {
        let field = Field::new(&[3.0, 20.0, 3.0, 2.0, 1.0], width, height);
        let points = scattered_points(300 + seed, 300, width, height);
        let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.05, seed);
        let selection = select(&pairs, 2..=5, &format!("seed {seed}"));
        assert_eq!(
            selection.model.order, 4,
            "seed {seed}: {:?}",
            selection.considered
        );
        let (_, h3, _) = considered_entry(&selection, 3);
        let (_, h4, _) = considered_entry(&selection, 4);
        assert!(
            h3 - h4 >= HOLDOUT_MIN_ABSOLUTE_GAIN_PX && h4 <= 0.9 * h3,
            "seed {seed}: {h3} -> {h4}"
        );
    }
}

#[test]
fn fit_select_keeps_order_3_when_the_holdout_gain_is_below_0_03_px() {
    let (width, height) = (3000.0, 3000.0);
    for seed in SEEDS {
        let field = Field::new(&[3.0, 20.0, 3.0, 2.0, 0.03], width, height);
        let points = scattered_points(400 + seed, 300, width, height);
        let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.002, seed);
        let selection = select(&pairs, 2..=5, &format!("seed {seed}"));
        assert_eq!(
            selection.model.order, 3,
            "seed {seed}: {:?}",
            selection.considered
        );
        let (_, h3, _) = considered_entry(&selection, 3);
        let (_, h4, _) = considered_entry(&selection, 4);
        assert!(
            h4 <= 0.9 * h3,
            "seed {seed}: the relative gate alone would raise ({h3} -> {h4})"
        );
        assert!(
            h3 - h4 < HOLDOUT_MIN_ABSOLUTE_GAIN_PX,
            "seed {seed}: {h3} -> {h4}"
        );
    }
}

#[test]
fn fit_select_keeps_order_3_when_the_holdout_gain_is_below_10_percent() {
    let (width, height) = (3000.0, 3000.0);
    for seed in SEEDS.iter().take(4).copied() {
        let field = Field::new(&[3.0, 20.0, 3.0, 2.0, 5.5], width, height);
        let points = scattered_points(500 + seed, 2000, width, height);
        let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 1.06, seed);
        let selection = select(&pairs, 3..=4, &format!("seed {seed}"));
        assert_eq!(
            selection.model.order, 3,
            "seed {seed}: {:?}",
            selection.considered
        );
        let (_, h3, _) = considered_entry(&selection, 3);
        let (_, h4, _) = considered_entry(&selection, 4);
        assert!(
            h3 - h4 >= HOLDOUT_MIN_ABSOLUTE_GAIN_PX,
            "seed {seed}: the absolute gate alone would raise ({h3} -> {h4})"
        );
        assert!(h4 > 0.9 * h3, "seed {seed}: {h3} -> {h4}");
    }
}

#[test]
fn fit_select_reaches_order_3_for_radial_distortion_even_though_order_2_adds_nothing() {
    let (width, height) = (3000.0, 3000.0);
    for seed in SEEDS {
        let points = scattered_points(600 + seed, 300, width, height);
        let pairs = with_noise(
            &pairs_through(&points, radial_field(3.0, width, height)),
            0.05,
            seed,
        );
        let selection = select(&pairs, 1..=4, &format!("seed {seed}"));
        assert_eq!(
            selection.model.order, 3,
            "seed {seed}: {:?}",
            selection.considered
        );
        let (_, h1, _) = considered_entry(&selection, 1);
        let (_, h2, _) = considered_entry(&selection, 2);
        assert!(
            h2 > 0.9 * h1,
            "seed {seed}: order 2 was meant to add nothing ({h1} -> {h2})"
        );
    }
}

#[test]
fn fit_select_caps_orders_on_the_full_pair_count_not_the_training_folds() {
    let field = Field::new(&[2.0, 15.0, 3.0, 2.0], 2500.0, 2500.0);
    let points = scattered_points(19, 40, 2500.0, 2500.0);
    let pairs = pairs_through(&points, |x, y| field.at(x, y));
    let selection = select(&pairs, 1..=4, "40 noiseless pairs");
    let orders: Vec<u8> = selection.considered.iter().map(|c| c.0).collect();
    assert_eq!(
        orders,
        vec![1, 2, 3],
        "order 3 has 10 terms x 4 = 40 pairs; its folds train on 32"
    );
    assert_eq!(selection.model.order, 3);
    assert!(
        selection.holdout_rms_px <= 1e-6,
        "{}",
        selection.holdout_rms_px
    );
}

#[test]
fn considered_holdout_and_bic_follow_the_documented_formulas() {
    let field = Field::new(&[3.0, 20.0, 2.0, 1.0], 2400.0, 1800.0);
    let points = scattered_points(20, 200, 2400.0, 1800.0);
    let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.08, 20);
    let selection = select(&pairs, 1..=4, "formulas");
    let orders: Vec<u8> = selection.considered.iter().map(|c| c.0).collect();
    assert_eq!(orders, vec![1, 2, 3, 4]);
    for &(order, holdout, bic) in &selection.considered {
        let expected_holdout = holdout_rms_by_hand(&pairs, order, 5);
        assert!(
            (holdout - expected_holdout).abs() <= 1e-6 * expected_holdout.max(1e-3),
            "order {order}: holdout {holdout} vs {expected_holdout} (pair i held out in fold fold_of[i], RMS of residual distances)"
        );
        let expected_bic = bic_by_hand(&pairs, order);
        assert!(
            (bic - expected_bic).abs() <= 1e-6 * expected_bic.abs().max(1.0),
            "order {order}: bic {bic} vs {expected_bic} (n ln(RSS/n) + 2 terms ln n)"
        );
    }
    let chosen = considered_entry(&selection, selection.model.order);
    assert_eq!(selection.holdout_rms_px, chosen.1);
    let refit = fit(&pairs, selection.model.order).expect("refit");
    assert_eq!(selection.model.norm, refit.norm);
    let drift = max_model_error(&selection.model, &grid(2400.0, 1800.0, 15), |x, y| {
        refit.eval(x, y)
    });
    assert!(
        drift <= 1e-9,
        "the selected model is the full-data fit of the chosen order (drift {drift:e} px)"
    );
}

#[test]
fn fit_select_refuses_fewer_than_two_folds_and_empty_order_ranges() {
    let field = Field::new(&[2.0, 15.0, 3.0], 2000.0, 2000.0);
    let pairs = pairs_through(&scattered_points(21, 100, 2000.0, 2000.0), |x, y| {
        field.at(x, y)
    });
    assert!(fit_select(&pairs, 1..=3, 5).is_some());
    assert!(
        fit_select(&pairs, 1..=3, 1).is_none(),
        "one fold has no holdout"
    );
    assert!(fit_select(&pairs, 1..=3, 0).is_none());
    assert!(
        fit_select(&pairs, RangeInclusive::new(3, 2), 5).is_none(),
        "empty order range"
    );
    assert!(
        fit_select(&pairs, 9..=12, 5).is_none(),
        "no order in range fits 100 pairs"
    );
}

#[test]
fn an_identity_model_exports_the_tan_params_themselves() {
    let tan = field_tan();
    let (width, height) = (2048usize, 1536usize);
    let bbox = [[-0.5, width as f64 - 0.5], [-0.5, height as f64 - 0.5]];
    let identity = |x: f64, y: f64| (x, y);
    let points = scattered_points(22, 100, width as f64, height as f64);
    let model = fit(&pairs_through(&points, identity), 1).expect("identity fit");
    let opts = SipFitOptions {
        crpix: Some([tan.crpix.0, tan.crpix.1]),
        ..SipFitOptions::default()
    };
    let fit = model
        .to_sip_cards(&tan, bbox, &opts)
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(fit.a_order, 1, "an identity model needs no SIP terms");
    assert_eq!(
        fit.crpix,
        [tan.crpix.0, tan.crpix.1],
        "TanParams.crpix is FITS 1-based like SipFit.crpix"
    );
    assert!(
        (fit.crval[0] - tan.crval.0).abs() <= 1e-9 && (fit.crval[1] - tan.crval.1).abs() <= 1e-9,
        "pixels are 0-based: the identity model must land CRVAL on TanParams.crval, got {:?}",
        fit.crval
    );
    let scale = plate_scale_deg(&tan);
    for row in 0..2 {
        for col in 0..2 {
            let diff = (fit.cd[row][col] - tan.cd[row][col]).abs();
            assert!(
                diff <= 1e-9 * scale,
                "CD{}_{}: {:?} vs {:?}",
                row + 1,
                col + 1,
                fit.cd,
                tan.cd
            );
        }
    }
    let exported = WcsTransform::from_header(&sip_header(&fit, (width, height)))
        .unwrap_or_else(|e| panic!("{e:#}"));
    let reference = WcsTransform::from_header(&tan_header(&tan, (width, height)))
        .unwrap_or_else(|e| panic!("{e:#}"));
    let check = grid(width as f64, height as f64, 16);
    let trip = sky_round_trip(&exported, &reference, identity, &check, scale);
    assert!(trip.forward_px <= 1e-6, "forward {:e} px", trip.forward_px);
    assert!(trip.inverse_px <= 1e-6, "inverse {:e} px", trip.inverse_px);
}

#[test]
fn an_affine_model_exports_a_plain_tan_header_on_the_same_sky() {
    let tan = field_tan();
    let (width, height) = (2048usize, 1536usize);
    let bbox = [[-0.5, width as f64 - 0.5], [-0.5, height as f64 - 0.5]];
    let affine = |x: f64, y: f64| (x + 0.004 * y + 2.5, y - 0.004 * x - 1.75);
    let points = scattered_points(22, 100, width as f64, height as f64);
    let model = fit(&pairs_through(&points, affine), 1).expect("affine fit");
    let opts = SipFitOptions::default();
    let fit = model
        .to_sip_cards(&tan, bbox, &opts)
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(
        fit.a_order, 1,
        "an affine model meets 0.01 px without SIP terms"
    );
    assert!(!fit.has_sip());
    let exported = WcsTransform::from_header(&sip_header(&fit, (width, height)))
        .unwrap_or_else(|e| panic!("{e:#}"));
    let reference = WcsTransform::from_header(&tan_header(&tan, (width, height)))
        .unwrap_or_else(|e| panic!("{e:#}"));
    let check = grid(width as f64, height as f64, 16);
    let trip = sky_round_trip(&exported, &reference, affine, &check, plate_scale_deg(&tan));
    let bound = (1.5 * fit.max_err_px).max(1e-6);
    assert!(
        trip.forward_px <= bound && trip.forward_px <= opts.max_pix_error,
        "forward {:e} px against the reported {:e} px",
        trip.forward_px,
        fit.max_err_px
    );
    assert!(
        trip.inverse_px <= opts.max_pix_error,
        "inverse {:e} px",
        trip.inverse_px
    );
}

#[test]
fn to_sip_cards_round_trips_through_wcs_from_header_within_1e_6_px() {
    let tan = field_tan();
    let (width, height) = (2048usize, 1536usize);
    let bbox = [[-0.5, width as f64 - 0.5], [-0.5, height as f64 - 0.5]];
    let field = Field::new(&[0.7, 4.0, 0.6, 1.0], width as f64, height as f64);
    let points = scattered_points(23, 400, width as f64, height as f64);
    let model = fit(&pairs_through(&points, |x, y| field.at(x, y)), 3).expect("order 3 fit");
    let opts = SipFitOptions {
        max_pix_error: 1e-9,
        degree: Some(vec![3]),
        inv_degree: Some(vec![4]),
        ..SipFitOptions::default()
    };
    let fit = model
        .to_sip_cards(&tan, bbox, &opts)
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(fit.a_order, 3);
    assert_eq!(fit.ap_order, Some(4));
    assert!(fit.max_err_px <= 1e-6, "SIPMXERR {:e}", fit.max_err_px);
    let header = sip_header(&fit, (width, height));
    assert_eq!(header.get("CTYPE1"), Some("RA---TAN-SIP"));
    assert_eq!(header.get("A_ORDER"), Some("3"));
    assert_eq!(header.get("AP_ORDER"), Some("4"));
    let exported = WcsTransform::from_header(&header).unwrap_or_else(|e| panic!("{e:#}"));
    let reference = WcsTransform::from_header(&tan_header(&tan, (width, height)))
        .unwrap_or_else(|e| panic!("{e:#}"));
    let truth = |x: f64, y: f64| field.at(x, y);
    let mut check = grid(width as f64, height as f64, 32);
    check.extend([
        (-0.5, -0.5),
        (width as f64 - 0.5, -0.5),
        (-0.5, height as f64 - 0.5),
        (width as f64 - 0.5, height as f64 - 0.5),
    ]);
    let trip = sky_round_trip(&exported, &reference, truth, &check, plate_scale_deg(&tan));
    assert!(trip.forward_px <= 1e-6, "forward {:e} px", trip.forward_px);
    assert!(trip.inverse_px < 0.01, "inverse {:e} px", trip.inverse_px);
}

#[test]
fn to_sip_cards_refuses_a_singular_cd_or_non_finite_tan() {
    let points = scattered_points(24, 50, 1000.0, 1000.0);
    let model = fit(&pairs_through(&points, |x, y| (x + 1.0, y + 2.0)), 1).expect("shift fit");
    let bbox = [[-0.5, 999.5], [-0.5, 999.5]];
    let good = field_tan();
    assert!(model
        .to_sip_cards(&good, bbox, &SipFitOptions::default())
        .is_ok());
    let singular = TanParams {
        cd: [[1e-4, 2e-4], [2e-4, 4e-4]],
        ..good
    };
    assert!(
        model
            .to_sip_cards(&singular, bbox, &SipFitOptions::default())
            .is_err(),
        "singular CD"
    );
    let nan_crval = TanParams {
        crval: (f64::NAN, 10.0),
        ..good
    };
    assert!(
        model
            .to_sip_cards(&nan_crval, bbox, &SipFitOptions::default())
            .is_err(),
        "NaN CRVAL"
    );
    let inf_crpix = TanParams {
        crpix: (f64::INFINITY, 1.0),
        ..good
    };
    assert!(
        model
            .to_sip_cards(&inf_crpix, bbox, &SipFitOptions::default())
            .is_err(),
        "infinite CRPIX"
    );
}

#[test]
fn order_10_is_refused_by_the_order_cap_even_with_enough_pairs() {
    let field = Field::new(&[1.0, 10.0, 2.0, 1.0], 3000.0, 3000.0);
    let pairs = pairs_through(&scattered_points(26, 400, 3000.0, 3000.0), |x, y| {
        field.at(x, y)
    });
    assert!(
        fit(&pairs, 9).is_some(),
        "order 9 fits 400 pairs (55 terms x 4 = 220)"
    );
    assert!(
        MIN_PAIRS_PER_TERM * term_count(10) <= pairs.len(),
        "the pair cap alone would accept order 10 (66 terms x 4 = 264)"
    );
    assert!(
        fit(&pairs, 10).is_none(),
        "order 10 exceeds MAX_ORDER = {MAX_ORDER}"
    );
    assert!(fit_pair(&pairs, 10).is_none());
    let selection = fit_select(&pairs, 1..=u8::MAX, 5).expect("orders 1 to 9 fit");
    assert_eq!(
        selection.considered.last().map(|c| c.0),
        Some(9),
        "{:?}",
        selection.considered
    );
}

#[test]
#[ignore = "timing bench: cargo test --release --lib registration::poly -- --ignored"]
fn bench_fit_select_2000_pairs_order_5_five_folds_within_15_ms() {
    let field = Field::new(&[3.0, 20.0, 3.0, 2.0, 1.0, 0.5], 6000.0, 4000.0);
    let points = scattered_points(25, 2000, 6000.0, 4000.0);
    let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.05, 25);
    let mut times = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        let selection = fit_select(&pairs, 5..=5, 5).expect("order 5 fits 2000 pairs");
        times.push(start.elapsed().as_secs_f64() * 1e3);
        assert_eq!(selection.model.order, 5);
    }
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    eprintln!("fit_select 2000 pairs order 5, 5 folds: median {median:.2} ms over {times:?}");
    assert!(median <= 15.0, "median {median:.2} ms > 15 ms");
}

fn row_major_grid(columns: usize, rows: usize, width: f64, height: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for col in 0..columns {
            let x = (width - 1.0) * col as f64 / (columns - 1) as f64;
            let y = (height - 1.0) * row as f64 / (rows - 1) as f64;
            out.push((x, y));
        }
    }
    out
}

#[test]
fn fit_select_validates_order_4_on_a_five_column_grid_listed_row_by_row() {
    let (width, height) = (3000.0, 3000.0);
    let field = Field::new(&[3.0, 20.0, 3.0, 2.0, 1.0], width, height);
    let points = row_major_grid(5, 20, width, height);
    let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.05, 27);
    assert!(
        fit(&pairs, 4).is_some(),
        "five distinct columns carry u^4 on the full data"
    );
    let selection = select(&pairs, 1..=4, "five-column grid");
    for &(order, holdout, _) in &selection.considered {
        assert!(
            holdout.is_finite(),
            "order {order}: a fold dropped a whole column ({:?})",
            selection.considered
        );
    }
    assert_eq!(selection.model.order, 4, "{:?}", selection.considered);
    assert!(
        selection.holdout_rms_px < 0.1,
        "{}",
        selection.holdout_rms_px
    );
}

#[test]
fn fit_select_returns_for_any_fold_count_by_clamping_it_to_the_pair_count() {
    let field = Field::new(&[2.0, 15.0, 3.0], 2000.0, 2000.0);
    let pairs = with_noise(
        &pairs_through(&scattered_points(28, 60, 2000.0, 2000.0), |x, y| {
            field.at(x, y)
        }),
        0.05,
        28,
    );
    let leave_one_out = fit_select(&pairs, 1..=2, pairs.len()).expect("leave-one-out selection");
    let (sender, receiver) = mpsc::channel();
    let worker_pairs = pairs.clone();
    thread::spawn(move || {
        let _ = sender.send(fit_select(&worker_pairs, 1..=2, usize::MAX));
    });
    let clamped = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("fit_select(.., usize::MAX folds) did not return within 30 s")
        .expect("selection with usize::MAX folds");
    assert_eq!(clamped, leave_one_out);
}

#[test]
fn fit_select_never_returns_an_order_whose_holdout_could_not_be_computed() {
    let field = Field::new(&[1.0, 5.0, 2.0], 1000.0, 1000.0);
    let mut points: Vec<(f64, f64)> = (0..23)
        .map(|i| {
            let x = if i % 2 == 0 { 0.0 } else { 999.0 };
            (x, 999.0 * i as f64 / 22.0)
        })
        .collect();
    points.push((499.5, 400.0));
    let pairs = pairs_through(&points, |x, y| field.at(x, y));
    assert!(
        fit(&pairs, 2).is_some(),
        "the single middle point makes u^2 resolvable on the full data"
    );
    assert_eq!(
        fit_select(&pairs, 2..=2, 5),
        None,
        "the fold that holds out the middle point cannot fit order 2"
    );
    let selection = fit_select(&pairs, 1..=2, 5).expect("order 1 validates");
    assert_eq!(selection.model.order, 1, "{:?}", selection.considered);
    assert!(selection.holdout_rms_px.is_finite());
    let (_, h2, _) = considered_entry(&selection, 2);
    assert!(
        h2.is_infinite(),
        "order 2 stays in considered with an unvalidated holdout"
    );
}

#[test]
fn fit_select_refuses_order_3_by_the_rule_on_40_noisy_pairs_of_a_quadratic_field() {
    for seed in SEEDS {
        let field = Field::new(&[3.0, 20.0, 3.0], 2000.0, 2000.0);
        let points = scattered_points(800 + seed, 40, 2000.0, 2000.0);
        let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.05, seed);
        let selection = select(&pairs, 1..=4, &format!("seed {seed}"));
        let orders: Vec<u8> = selection.considered.iter().map(|c| c.0).collect();
        assert_eq!(
            orders,
            vec![1, 2, 3],
            "seed {seed}: order 3 is offered, so the rule and not the cap must refuse it"
        );
        assert_eq!(
            selection.model.order, 2,
            "seed {seed}: {:?}",
            selection.considered
        );
    }
}

#[test]
fn fold_assignment_is_a_balanced_partition_that_ignores_the_input_order() {
    for (len, folds) in [(12, 5), (40, 5), (100, 5), (2000, 5), (7, 7), (60, 60)] {
        let fold_of = fold_assignment(len, folds);
        assert_eq!(fold_of.len(), len);
        assert_eq!(fold_of, fold_assignment(len, folds), "deterministic");
        let mut sizes = vec![0usize; folds];
        for &f in &fold_of {
            assert!(f < folds, "len {len}: fold {f} of {folds}");
            sizes[f] += 1;
        }
        let (smallest, largest) = (sizes.iter().min(), sizes.iter().max());
        assert_eq!(
            (smallest, largest),
            (Some(&(len / folds)), Some(&len.div_ceil(folds))),
            "len {len}, folds {folds}: {sizes:?}"
        );
    }
    let fold_of = fold_assignment(100, 5);
    for fold in 0..5 {
        for residue in 0..5 {
            let kept = (0..100).any(|i| i % 5 == residue && fold_of[i] != fold);
            assert!(
                kept,
                "fold {fold} holds out every index congruent to {residue} modulo 5"
            );
        }
    }
}

#[test]
fn fit_select_keeps_order_2_on_40_pairs_when_both_holdout_gates_pass_but_the_bic_rises() {
    for seed in [1, 5, 8, 13, 21] {
        let field = Field::new(&[3.0, 20.0, 3.0, 0.8], 3000.0, 3000.0);
        let points = scattered_points(700 + seed, 40, 3000.0, 3000.0);
        let pairs = with_noise(&pairs_through(&points, |x, y| field.at(x, y)), 0.2, seed);
        let selection = select(&pairs, 2..=3, &format!("seed {seed}"));
        let (_, h2, b2) = considered_entry(&selection, 2);
        let (_, h3, b3) = considered_entry(&selection, 3);
        assert!(
            h2 - h3 >= HOLDOUT_MIN_ABSOLUTE_GAIN_PX && h3 <= 0.9 * h2,
            "seed {seed}: both holdout gates pass ({h2} -> {h3})"
        );
        assert!(
            b3 >= b2,
            "seed {seed}: the BIC rises at 40 pairs ({b2} -> {b3})"
        );
        assert_eq!(
            selection.model.order, 2,
            "seed {seed}: the BIC is a co-gate, not a tie-break"
        );
    }
}

#[test]
fn to_sip_cards_deprojects_with_the_exact_inverse_of_the_native_plane_projection() {
    let tan = field_tan();
    let sky = TanSky::new(&tan).expect("regular TAN parameters");
    let plane = NativePlane::new(tan.crval.0, tan.crval.1);
    let scale = plate_scale_deg(&tan);
    let mut worst: f64 = 0.0;
    for (x, y) in grid(2048.0, 1536.0, 17) {
        let dx = x + 1.0 - tan.crpix.0;
        let dy = y + 1.0 - tan.crpix.1;
        let xi = tan.cd[0][0] * dx + tan.cd[0][1] * dy;
        let eta = tan.cd[1][0] * dx + tan.cd[1][1] * dy;
        let [px, py] = plane.project(sky.pixel_to_world(x, y));
        worst = worst.max((px - xi).hypot(py - eta) / scale);
    }
    assert!(
        worst <= 1e-9,
        "project(pixel_to_world(p)) left the intermediate plane by {worst:e} px"
    );
}
