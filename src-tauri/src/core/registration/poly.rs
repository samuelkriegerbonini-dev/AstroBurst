use std::cmp::Ordering;
use std::ops::RangeInclusive;

use crate::core::astrometry::gwcs::sip_fit::lu_solve;
use crate::core::astrometry::gwcs::{
    fit_tan_sip, GwcsError, Kind, Node, SipFit, SipFitOptions, Vals,
};

pub const MIN_PAIRS_PER_TERM: usize = 4;
pub const HOLDOUT_MIN_RELATIVE_GAIN: f64 = 0.10;
pub const HOLDOUT_MIN_ABSOLUTE_GAIN_PX: f64 = 0.03;
pub const MAX_ORDER: u8 = 9;

const MAX_TERMS: usize = (MAX_ORDER as usize + 1) * (MAX_ORDER as usize + 2) / 2;
const MAX_NORMAL_CONDITION: f64 = 1e12;
const SINGULAR_CD_RELATIVE_DET: f64 = 1e-12;
const TAN_LON_POLE: f64 = 180.0;

type Pair = (f64, f64, f64, f64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Normalisation {
    pub x0: f64,
    pub y0: f64,
    pub sx: f64,
    pub sy: f64,
}

impl Normalisation {
    fn spanning(pairs: &[Pair]) -> Option<Self> {
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for &(x, y, _, _) in pairs {
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            ymin = ymin.min(y);
            ymax = ymax.max(y);
        }
        let sx = (xmax - xmin) / 2.0;
        let sy = (ymax - ymin) / 2.0;
        (sx > 0.0 && sy > 0.0).then_some(Self {
            x0: (xmin + xmax) / 2.0,
            y0: (ymin + ymax) / 2.0,
            sx,
            sy,
        })
    }

    fn is_usable(&self) -> bool {
        [self.x0, self.y0, self.sx, self.sy]
            .iter()
            .all(|v| v.is_finite())
            && self.sx > 0.0
            && self.sy > 0.0
    }

    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.x0) / self.sx, (y - self.y0) / self.sy)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolyModel {
    pub order: u8,
    pub norm: Normalisation,
    pub cx: Vec<f64>,
    pub cy: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolyPair {
    pub forward: PolyModel,
    pub inverse: PolyModel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolySelection {
    pub model: PolyModel,
    pub holdout_rms_px: f64,
    pub considered: Vec<(u8, f64, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TanParams {
    pub crval: (f64, f64),
    pub crpix: (f64, f64),
    pub cd: [[f64; 2]; 2],
}

fn term_count(order: u8) -> usize {
    let n = usize::from(order);
    (n + 1) * (n + 2) / 2
}

fn all_finite(pairs: &[Pair]) -> bool {
    pairs
        .iter()
        .all(|&(x, y, tx, ty)| x.is_finite() && y.is_finite() && tx.is_finite() && ty.is_finite())
}

fn fill_monomials(order: u8, u: f64, v: f64, out: &mut [f64]) {
    if let Some(constant) = out.first_mut() {
        *constant = 1.0;
    }
    let mut start = 0;
    for degree in 0..usize::from(order) {
        let len = degree + 1;
        let Some((filled, rest)) = out.split_at_mut_checked(start + len) else {
            return;
        };
        let (Some(previous), Some(next)) = (filled.get(start..), rest.get_mut(..=len)) else {
            return;
        };
        for (target, &source) in next.iter_mut().zip(previous) {
            *target = source * u;
        }
        if let (Some(target), Some(&source)) = (next.last_mut(), previous.last()) {
            *target = source * v;
        }
        start += len;
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn add_in_place(target: &mut [f64], delta: &[f64]) {
    for (t, d) in target.iter_mut().zip(delta) {
        *t += d;
    }
}

struct Design {
    terms: usize,
    rows: Vec<f64>,
}

impl Design {
    fn build(pairs: &[Pair], order: u8, norm: &Normalisation) -> Self {
        let terms = term_count(order);
        let mut rows = Vec::with_capacity(terms * pairs.len());
        let mut row = vec![0.0; terms];
        for &(x, y, _, _) in pairs {
            let (u, v) = norm.apply(x, y);
            fill_monomials(order, u, v, &mut row);
            rows.extend_from_slice(&row);
        }
        Self { terms, rows }
    }

    fn rows(&self) -> impl Iterator<Item = &[f64]> {
        self.rows.chunks_exact(self.terms)
    }

    fn normal_equations(&self, pairs: &[Pair]) -> NormalEquations {
        let mut normal = NormalEquations::zero(self.terms);
        for (row, &(_, _, tx, ty)) in self.rows().zip(pairs) {
            normal.accumulate(row, tx, ty);
        }
        normal
    }

    fn gradient(&self, pairs: &[Pair], cx: &[f64], cy: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let mut gx = vec![0.0; self.terms];
        let mut gy = vec![0.0; self.terms];
        for (row, &(_, _, tx, ty)) in self.rows().zip(pairs) {
            let rx = tx - dot(row, cx);
            let ry = ty - dot(row, cy);
            for ((gx_i, gy_i), &phi) in gx.iter_mut().zip(gy.iter_mut()).zip(row) {
                *gx_i += phi * rx;
                *gy_i += phi * ry;
            }
        }
        (gx, gy)
    }
}

struct NormalEquations {
    a: Vec<Vec<f64>>,
    bx: Vec<f64>,
    by: Vec<f64>,
}

impl NormalEquations {
    fn zero(terms: usize) -> Self {
        Self {
            a: vec![vec![0.0; terms]; terms],
            bx: vec![0.0; terms],
            by: vec![0.0; terms],
        }
    }

    fn accumulate(&mut self, row: &[f64], tx: f64, ty: f64) {
        let lines = self.a.iter_mut().zip(&mut self.bx).zip(&mut self.by);
        for (((a_i, bx_i), by_i), &phi_i) in lines.zip(row) {
            for (a_ij, &phi_j) in a_i.iter_mut().zip(row) {
                *a_ij += phi_i * phi_j;
            }
            *bx_i += phi_i * tx;
            *by_i += phi_i * ty;
        }
    }
}

struct ScaledSystem {
    matrix: Vec<Vec<f64>>,
    scale: Vec<f64>,
}

impl ScaledSystem {
    fn from_normal(a: &[Vec<f64>]) -> Option<Self> {
        let scale = a
            .iter()
            .enumerate()
            .map(|(i, row)| {
                row.get(i)
                    .copied()
                    .filter(|d| d.is_finite() && *d > 0.0)
                    .map(|d| 1.0 / d.sqrt())
            })
            .collect::<Option<Vec<f64>>>()?;
        let matrix = a
            .iter()
            .zip(&scale)
            .map(|(row, &si)| {
                row.iter()
                    .zip(&scale)
                    .map(|(&a_ij, &sj)| a_ij * si * sj)
                    .collect()
            })
            .collect();
        let system = Self { matrix, scale };
        system.is_well_conditioned().then_some(system)
    }

    fn solve(&self, rhs: &[f64]) -> Option<Vec<f64>> {
        let mut a = self.matrix.clone();
        let mut b: Vec<f64> = rhs.iter().zip(&self.scale).map(|(r, s)| r * s).collect();
        lu_solve(&mut a, &mut b)?;
        Some(b.iter().zip(&self.scale).map(|(c, s)| c * s).collect())
    }

    fn is_well_conditioned(&self) -> bool {
        let n = self.matrix.len();
        let one_norm = (0..n)
            .map(|j| {
                self.matrix
                    .iter()
                    .map(|row| row.get(j).map_or(0.0, |v| v.abs()))
                    .sum::<f64>()
            })
            .fold(0.0, f64::max);
        let mut inverse_norm: f64 = 0.0;
        for j in 0..n {
            let mut a = self.matrix.clone();
            let mut column: Vec<f64> = (0..n).map(|i| if i == j { 1.0 } else { 0.0 }).collect();
            if lu_solve(&mut a, &mut column).is_none() {
                return false;
            }
            inverse_norm = inverse_norm.max(column.iter().map(|v| v.abs()).sum());
        }
        one_norm * inverse_norm < MAX_NORMAL_CONDITION
    }
}

fn fit_with_min_pairs(pairs: &[Pair], order: u8, min_pairs: usize) -> Option<PolyModel> {
    if order == 0 || order > MAX_ORDER || pairs.len() < min_pairs || !all_finite(pairs) {
        return None;
    }
    let norm = Normalisation::spanning(pairs)?;
    let design = Design::build(pairs, order, &norm);
    let normal = design.normal_equations(pairs);
    let system = ScaledSystem::from_normal(&normal.a)?;
    let mut cx = system.solve(&normal.bx)?;
    let mut cy = system.solve(&normal.by)?;
    let (gx, gy) = design.gradient(pairs, &cx, &cy);
    add_in_place(&mut cx, &system.solve(&gx)?);
    add_in_place(&mut cy, &system.solve(&gy)?);
    let model = PolyModel {
        order,
        norm,
        cx,
        cy,
    };
    model.coefficients_are_finite().then_some(model)
}

pub fn fit(pairs: &[Pair], order: u8) -> Option<PolyModel> {
    fit_with_min_pairs(pairs, order, MIN_PAIRS_PER_TERM * term_count(order))
}

pub fn fit_pair(pairs: &[Pair], order: u8) -> Option<PolyPair> {
    let forward = fit(pairs, order)?;
    let swapped: Vec<Pair> = pairs.iter().map(|&(x, y, tx, ty)| (tx, ty, x, y)).collect();
    let inverse = fit(&swapped, order)?;
    Some(PolyPair { forward, inverse })
}

struct Candidate {
    model: PolyModel,
    holdout: f64,
    bic: f64,
}

impl Candidate {
    fn improves_on(&self, incumbent: &Self) -> bool {
        let gain = incumbent.holdout - self.holdout;
        gain >= HOLDOUT_MIN_ABSOLUTE_GAIN_PX
            && self.holdout <= (1.0 - HOLDOUT_MIN_RELATIVE_GAIN) * incumbent.holdout
            && self.bic < incumbent.bic
    }
}

fn scrambled(index: usize) -> u64 {
    let mut z = (index as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn fold_assignment(len: usize, folds: usize) -> Vec<usize> {
    let mut by_scramble: Vec<usize> = (0..len).collect();
    by_scramble.sort_unstable_by_key(|&index| scrambled(index));
    let mut fold_of = vec![0; len];
    for (rank, index) in by_scramble.into_iter().enumerate() {
        if let Some(slot) = fold_of.get_mut(index) {
            *slot = rank.checked_rem(folds).unwrap_or(0);
        }
    }
    fold_of
}

fn holdout_rms(pairs: &[Pair], order: u8, fold_of: &[usize], folds: usize) -> f64 {
    let terms = term_count(order);
    let mut training = Vec::with_capacity(pairs.len());
    let mut held_out = Vec::new();
    let mut sum = 0.0;
    for fold in 0..folds {
        training.clear();
        held_out.clear();
        for (&pair, &pair_fold) in pairs.iter().zip(fold_of) {
            if pair_fold == fold {
                held_out.push(pair);
            } else {
                training.push(pair);
            }
        }
        let Some(model) = fit_with_min_pairs(&training, order, terms) else {
            return f64::INFINITY;
        };
        sum += held_out
            .iter()
            .map(|&(x, y, tx, ty)| {
                let (gx, gy) = model.eval(x, y);
                let distance = (gx - tx).hypot(gy - ty);
                distance * distance
            })
            .sum::<f64>();
    }
    (sum / pairs.len() as f64).sqrt()
}

fn bic(model: &PolyModel, pairs: &[Pair]) -> f64 {
    let n = pairs.len() as f64;
    let rss: f64 = model.residuals(pairs).iter().map(|r| r * r).sum();
    n * (rss / n).ln() + (2 * model.cx.len()) as f64 * n.ln()
}

pub fn fit_select(
    pairs: &[Pair],
    orders: RangeInclusive<u8>,
    folds: usize,
) -> Option<PolySelection> {
    if folds < 2 || pairs.is_empty() || !all_finite(pairs) {
        return None;
    }
    let folds = folds.min(pairs.len());
    let fold_of = fold_assignment(pairs.len(), folds);
    let mut candidates: Vec<Candidate> = Vec::new();
    for order in orders {
        if order == 0 {
            continue;
        }
        let Some(model) = fit(pairs, order) else {
            continue;
        };
        let holdout = holdout_rms(pairs, order, &fold_of, folds);
        let bic = bic(&model, pairs);
        candidates.push(Candidate {
            model,
            holdout,
            bic,
        });
    }
    let considered: Vec<(u8, f64, f64)> = candidates
        .iter()
        .map(|c| (c.model.order, c.holdout, c.bic))
        .collect();
    let mut validated = candidates.iter().filter(|c| c.holdout.is_finite());
    let mut chosen = validated.next()?;
    for candidate in validated {
        if candidate.improves_on(chosen) {
            chosen = candidate;
        }
    }
    Some(PolySelection {
        model: chosen.model.clone(),
        holdout_rms_px: chosen.holdout,
        considered,
    })
}

struct TanSky {
    crpix: (f64, f64),
    cd: [[f64; 2]; 2],
    deproject: Node,
    rotate: Node,
}

impl TanSky {
    fn new(tan: &TanParams) -> Result<Self, GwcsError> {
        let [[a, b], [c, d]] = tan.cd;
        let values = [
            tan.crval.0,
            tan.crval.1,
            tan.crpix.0,
            tan.crpix.1,
            a,
            b,
            c,
            d,
        ];
        if !values.iter().all(|v| v.is_finite()) {
            return Err(refusal("TAN parameters are not finite"));
        }
        let det = a * d - b * c;
        let magnitude = a * a + b * b + c * c + d * d;
        let regular = det
            .abs()
            .partial_cmp(&(SINGULAR_CD_RELATIVE_DET * magnitude))
            == Some(Ordering::Greater);
        if !regular {
            return Err(refusal("TAN CD matrix is singular"));
        }
        Ok(Self {
            crpix: tan.crpix,
            cd: tan.cd,
            deproject: Node::new(Kind::Pix2SkyTan, "gnomonic", "poly"),
            rotate: Node::new(
                Kind::rotate_native2celestial(tan.crval.0, tan.crval.1, TAN_LON_POLE),
                "rotate3d",
                "poly",
            ),
        })
    }

    fn pixel_to_world(&self, px: f64, py: f64) -> [f64; 2] {
        let dx = px + 1.0 - self.crpix.0;
        let dy = py + 1.0 - self.crpix.1;
        let xi = self.cd[0][0] * dx + self.cd[0][1] * dy;
        let eta = self.cd[1][0] * dx + self.cd[1][1] * dy;
        let sky = self.rotate.eval(self.deproject.eval(Vals::pair(xi, eta)));
        [sky.v[0], sky.v[1]]
    }
}

fn refusal(text: &str) -> GwcsError {
    GwcsError::Parse(text.to_string())
}

impl PolyModel {
    fn is_well_formed(&self) -> bool {
        let terms = term_count(self.order);
        self.order >= 1
            && self.order <= MAX_ORDER
            && self.norm.is_usable()
            && self.cx.len() == terms
            && self.cy.len() == terms
    }

    fn coefficients_are_finite(&self) -> bool {
        self.cx.iter().chain(&self.cy).all(|c| c.is_finite())
    }

    pub fn eval(&self, x: f64, y: f64) -> (f64, f64) {
        if !self.is_well_formed() {
            return (f64::NAN, f64::NAN);
        }
        let (u, v) = self.norm.apply(x, y);
        let mut buffer = [0.0; MAX_TERMS];
        let Some(row) = buffer.get_mut(..self.cx.len()) else {
            return (f64::NAN, f64::NAN);
        };
        fill_monomials(self.order, u, v, row);
        (dot(&self.cx, row), dot(&self.cy, row))
    }

    pub fn residuals(&self, pairs: &[Pair]) -> Vec<f64> {
        pairs
            .iter()
            .map(|&(x, y, tx, ty)| {
                let (gx, gy) = self.eval(x, y);
                (gx - tx).hypot(gy - ty)
            })
            .collect()
    }

    pub fn max_quadrant_residual(&self, pairs: &[Pair]) -> [Option<f64>; 4] {
        let mut out = [None; 4];
        for (&(x, y, _, _), residual) in pairs.iter().zip(self.residuals(pairs)) {
            let quadrant = usize::from(x >= self.norm.x0) + 2 * usize::from(y >= self.norm.y0);
            if let Some(slot) = out.get_mut(quadrant) {
                *slot = Some(slot.map_or(residual, |worst: f64| worst.max(residual)));
            }
        }
        out
    }

    pub fn to_sip_cards(
        &self,
        tan: &TanParams,
        bbox: [[f64; 2]; 2],
        opts: &SipFitOptions,
    ) -> Result<SipFit, GwcsError> {
        if !self.is_well_formed() {
            return Err(refusal(
                "polynomial model is malformed: order, normalisation and coefficient count disagree",
            ));
        }
        let sky = TanSky::new(tan)?;
        let eval = |x: f64, y: f64| {
            let (px, py) = self.eval(x, y);
            sky.pixel_to_world(px, py)
        };
        fit_tan_sip(&eval, bbox, opts)
    }
}

#[cfg(test)]
mod tests;
