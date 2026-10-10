use std::cmp::Ordering;

use super::pipeline::GwcsPipeline;
use super::transform::{Kind, Node, Vals};
use super::GwcsError;

pub const SIP_FIT_MAX_DEGREE: u8 = 6;
const SIP_DEGREE_LIMIT: u8 = 9;
const TAN_LON_POLE: f64 = 180.0;
const SINGULAR_CD_DET: f64 = 1e-300;
const ACCURACY_WARNING: &str = "Failed to achieve requested SIP approximation accuracy.";
const DOUBLE_SAMPLING_WARNING: &str =
    "Double sampling check FAILED: Sampling may be too coarse for the distortion model being fitted.";
const NOT_FINITE_FIT: &str = "Failed to fit SIP. Computed coefficients are not finite.";

#[derive(Debug, Clone)]
pub struct SipFitOptions {
    pub max_pix_error: f64,
    pub degree: Option<Vec<u8>>,
    pub max_inv_pix_error: Option<f64>,
    pub inv_degree: Option<Vec<u8>>,
    pub npoints: usize,
    pub crpix: Option<[f64; 2]>,
}

impl Default for SipFitOptions {
    fn default() -> Self {
        Self {
            max_pix_error: 0.01,
            degree: None,
            max_inv_pix_error: Some(0.01),
            inv_degree: None,
            npoints: 32,
            crpix: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SipFit {
    pub crpix: [f64; 2],
    pub crval: [f64; 2],
    pub cd: [[f64; 2]; 2],
    pub a_order: u8,
    pub a: Vec<(u8, u8, f64)>,
    pub b: Vec<(u8, u8, f64)>,
    pub ap_order: Option<u8>,
    pub ap: Vec<(u8, u8, f64)>,
    pub bp: Vec<(u8, u8, f64)>,
    pub max_err_px: f64,
    pub inv_err_px: Option<f64>,
    pub plate_scale_deg: f64,
    pub warnings: Vec<String>,
}

impl SipFit {
    pub fn has_sip(&self) -> bool {
        self.a_order > 1
    }

    pub fn header_cards(&self) -> Vec<(String, String)> {
        let suffix = if self.has_sip() { "-SIP" } else { "" };
        let mut cards: Vec<(String, String)> = vec![
            ("WCSAXES".into(), "2".into()),
            ("CTYPE1".into(), format!("RA---TAN{suffix}")),
            ("CTYPE2".into(), format!("DEC--TAN{suffix}")),
            ("CRPIX1".into(), self.crpix[0].to_string()),
            ("CRPIX2".into(), self.crpix[1].to_string()),
            ("CRVAL1".into(), self.crval[0].to_string()),
            ("CRVAL2".into(), self.crval[1].to_string()),
            ("CUNIT1".into(), "deg".into()),
            ("CUNIT2".into(), "deg".into()),
            ("CD1_1".into(), self.cd[0][0].to_string()),
            ("CD1_2".into(), self.cd[0][1].to_string()),
            ("CD2_1".into(), self.cd[1][0].to_string()),
            ("CD2_2".into(), self.cd[1][1].to_string()),
            ("LONPOLE".into(), TAN_LON_POLE.to_string()),
            ("RADESYS".into(), "ICRS".into()),
        ];
        if !self.has_sip() {
            return cards;
        }
        cards.push(("A_ORDER".into(), self.a_order.to_string()));
        cards.push(("B_ORDER".into(), self.a_order.to_string()));
        push_terms(&mut cards, "A", &self.a);
        push_terms(&mut cards, "B", &self.b);
        cards.push(("SIPMXERR".into(), exponent_form(self.max_err_px)));
        if let (Some(order), Some(err)) = (self.ap_order, self.inv_err_px) {
            cards.push(("AP_ORDER".into(), order.to_string()));
            cards.push(("BP_ORDER".into(), order.to_string()));
            push_terms(&mut cards, "AP", &self.ap);
            push_terms(&mut cards, "BP", &self.bp);
            cards.push(("SIPIVERR".into(), exponent_form(err)));
        }
        cards
    }
}

fn exponent_form(v: f64) -> String {
    format!("{v:E}")
}

fn push_terms(cards: &mut Vec<(String, String)>, prefix: &str, terms: &[(u8, u8, f64)]) {
    for (i, j, c) in terms {
        cards.push((format!("{prefix}_{i}_{j}"), exponent_form(*c)));
    }
}

fn refusal(text: &str) -> GwcsError {
    GwcsError::Parse(text.to_string())
}

pub fn fit_tan_sip(
    eval: &dyn Fn(f64, f64) -> [f64; 2],
    bbox: [[f64; 2]; 2],
    opts: &SipFitOptions,
) -> Result<SipFit, GwcsError> {
    if opts.npoints < 8 {
        return Err(refusal("Number of sampling points is too small. 'npoints' must be >= 8."));
    }
    let [[xmin, xmax], [ymin, ymax]] = bbox;
    if !(xmax - xmin >= 1.0 && ymax - ymin >= 1.0) {
        return Err(refusal("Bounding box is too small for fitting a SIP polynomial"));
    }
    let degrees = degree_list(opts.degree.as_deref())?;
    let inv_degrees = degree_list(opts.inv_degree.as_deref())?;
    let crpix = match opts.crpix {
        Some([x, y]) => [x - 1.0, y - 1.0],
        None => [round_tenth((xmin + xmax) / 2.0), round_tenth((ymin + ymax) / 2.0)],
    };
    let [lon, lat] = eval(crpix[0], crpix[1]);
    if !(lon.is_finite() && lat.is_finite()) {
        return Err(refusal("gWCS is not finite at the fit reference pixel"));
    }
    let native = NativePlane::new(lon, lat);
    let ntransform = |u: f64, v: f64| native.project(eval(u + crpix[0], v + crpix[1]));

    let single = Samples::on_grid(opts.npoints, bbox, crpix, &ntransform);
    let double = Samples::on_grid(2 * opts.npoints, bbox, crpix, &ntransform);

    let x0 = ntransform(0.0, 0.0);
    let xx = ntransform(1.0, 0.0);
    let yx = ntransform(0.0, 1.0);
    let pixarea = ((xx[0] - x0[0]) * (yx[1] - x0[1]) - (xx[1] - x0[1]) * (yx[0] - x0[0])).abs();
    let plate_scale = pixarea.sqrt();
    if !(plate_scale.is_finite() && plate_scale > 0.0) {
        return Err(refusal("SIP fit produced a singular CD matrix"));
    }

    let forward = fit_2d_poly(&degrees, opts.max_pix_error, plate_scale, &single, &double)?;
    let (cd, a, b) = reform(&forward)?;
    let mut warnings = forward.warnings.clone();
    let mut inverse = None;
    if forward.degree > 1 {
        if let Some(target) = opts.max_inv_pix_error.filter(|t| *t > 0.0) {
            let fit = fit_2d_poly(&inv_degrees, target, 1.0, &single.inverse_samples(&cd), &double.inverse_samples(&cd))?;
            warnings.extend(fit.warnings.iter().cloned());
            inverse = Some(fit);
        }
    }
    let terms = |fit: &PolyFit, c: &[f64]| -> Vec<(u8, u8, f64)> {
        fit.powers.iter().zip(c).map(|(&(i, j), &c)| (i, j, c)).collect()
    };
    Ok(SipFit {
        crpix: [crpix[0] + 1.0, crpix[1] + 1.0],
        crval: [normalise_ra(lon), lat],
        cd,
        a_order: forward.degree,
        a,
        b,
        ap_order: inverse.as_ref().map(|f| f.degree),
        ap: inverse.as_ref().map(|f| terms(f, &f.cx)).unwrap_or_default(),
        bp: inverse.as_ref().map(|f| terms(f, &f.cy)).unwrap_or_default(),
        max_err_px: forward.max_err_px,
        inv_err_px: inverse.as_ref().map(|f| f.max_err_px),
        plate_scale_deg: plate_scale,
        warnings,
    })
}

pub fn fit_pipeline_tan_sip(
    pipeline: &GwcsPipeline,
    opts: &SipFitOptions,
    naxis: Option<(usize, usize)>,
) -> Result<SipFit, GwcsError> {
    let from_naxis = |axis: usize| -> Result<[f64; 2], GwcsError> {
        let (n1, n2) = naxis.ok_or_else(|| {
            GwcsError::Bbox("a bounding box or the array dimensions are needed to fit a SIP header".into())
        })?;
        let n = if axis == 0 { n1 } else { n2 };
        Ok([-0.5, n as f64 - 0.5])
    };
    let mut bbox = [[0.0; 2]; 2];
    for axis in 0..2 {
        bbox[axis] = match pipeline.bounding_box {
            Some(b) if !b.ignore[axis] => b.intervals[axis],
            _ => from_naxis(axis)?,
        };
    }
    fit_tan_sip(&|x, y| pipeline.forward(x, y, false), bbox, opts)
}

fn degree_list(requested: Option<&[u8]>) -> Result<Vec<u8>, GwcsError> {
    let Some(list) = requested else {
        return Ok((1..=SIP_FIT_MAX_DEGREE).collect());
    };
    let mut degrees = list.to_vec();
    degrees.sort_unstable();
    degrees.dedup();
    match (degrees.first(), degrees.last()) {
        (Some(&lo), Some(&hi)) if lo >= 1 && hi <= SIP_DEGREE_LIMIT => Ok(degrees),
        _ => Err(refusal("Allowed values for SIP degree are [1...9]")),
    }
}

fn round_tenth(x: f64) -> f64 {
    (x * 10.0).round_ties_even() / 10.0
}

fn normalise_ra(ra: f64) -> f64 {
    if ra >= 360.0 {
        ra - 360.0
    } else if ra < 0.0 {
        ra + 360.0
    } else {
        ra
    }
}

struct NativePlane {
    rotate: Node,
    project: Node,
}

impl NativePlane {
    fn new(lon: f64, lat: f64) -> Self {
        Self {
            rotate: Node::new(Kind::rotate_celestial2native(lon, lat, TAN_LON_POLE), "rotate3d", "sip_fit"),
            project: Node::new(Kind::Sky2PixTan, "gnomonic", "sip_fit"),
        }
    }

    fn project(&self, sky: [f64; 2]) -> [f64; 2] {
        let out = self.project.eval(self.rotate.eval(Vals::pair(sky[0], sky[1])));
        [out.v[0], out.v[1]]
    }
}

struct Samples {
    u: Vec<f64>,
    v: Vec<f64>,
    x: Vec<f64>,
    y: Vec<f64>,
}

impl Samples {
    fn on_grid(npoints: usize, bbox: [[f64; 2]; 2], crpix: [f64; 2], ntransform: &dyn Fn(f64, f64) -> [f64; 2]) -> Self {
        let (u, v) = sampling_grid(npoints, bbox, crpix);
        let mut x = Vec::with_capacity(u.len());
        let mut y = Vec::with_capacity(u.len());
        for (uu, vv) in u.iter().zip(&v) {
            let p = ntransform(*uu, *vv);
            x.push(p[0]);
            y.push(p[1]);
        }
        Self { u, v, x, y }
    }

    fn inverse_samples(&self, cd: &[[f64; 2]; 2]) -> Self {
        let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
        let n = self.u.len();
        let mut out = Self { u: Vec::with_capacity(n), v: Vec::with_capacity(n), x: Vec::with_capacity(n), y: Vec::with_capacity(n) };
        for k in 0..n {
            let lin_u = (cd[1][1] * self.x[k] - cd[0][1] * self.y[k]) / det;
            let lin_v = (-cd[1][0] * self.x[k] + cd[0][0] * self.y[k]) / det;
            out.u.push(lin_u);
            out.v.push(lin_v);
            out.x.push(self.u[k] - lin_u);
            out.y.push(self.v[k] - lin_v);
        }
        out
    }
}

fn sampling_grid(npoints: usize, bbox: [[f64; 2]; 2], crpix: [f64; 2]) -> (Vec<f64>, Vec<f64>) {
    let axis = |[lo, hi]: [f64; 2], c: f64| -> Vec<f64> {
        let step = (lo - hi) / (1.0 - npoints as f64);
        let size = ((hi + step - lo) / step).ceil().max(0.0) as usize;
        (0..size).map(|i| i as f64 * step + lo - c).collect()
    };
    let xs = axis(bbox[0], crpix[0]);
    let ys = axis(bbox[1], crpix[1]);
    let mut u = Vec::with_capacity(xs.len() * ys.len());
    let mut v = Vec::with_capacity(xs.len() * ys.len());
    for y in &ys {
        for x in &xs {
            u.push(*x);
            v.push(*y);
        }
    }
    (u, v)
}

struct PolyFit {
    cx: Vec<f64>,
    cy: Vec<f64>,
    powers: Vec<(u8, u8)>,
    degree: u8,
    max_err_px: f64,
    warnings: Vec<String>,
}

fn fit_2d_poly(degrees: &[u8], max_pix_error: f64, plate_scale: f64, single: &Samples, double: &Samples) -> Result<PolyFit, GwcsError> {
    let single_degree = degrees.len() == 1;
    let max_error = max_pix_error * plate_scale;
    let mut best: Option<(Vec<f64>, Vec<f64>, Vec<(u8, u8)>, u8)> = None;
    let mut fit_error = f64::INFINITY;
    let mut met = false;
    for &deg in degrees {
        let Some((cx, cy, powers, resid)) = poly_fit_lu(single, deg) else {
            if single_degree {
                return Err(refusal(NOT_FINITE_FIT));
            }
            break;
        };
        if resid >= fit_error {
            break;
        }
        best = Some((cx, cy, powers, deg));
        fit_error = resid;
        if fit_error <= max_error {
            met = true;
            break;
        }
    }
    let Some((cx, cy, powers, degree)) = best else {
        return Err(refusal(NOT_FINITE_FIT));
    };
    let mut warnings = Vec::new();
    if !met {
        warnings.push(ACCURACY_WARNING.to_string());
    }
    if met || single_degree {
        let dense = max_residual(double, &powers, &cx, &cy);
        if dense > (5.0 * fit_error).min(max_error) {
            warnings.push(DOUBLE_SAMPLING_WARNING.to_string());
        }
        fit_error = fit_error.max(dense);
    }
    Ok(PolyFit { cx, cy, powers, degree, max_err_px: fit_error / plate_scale, warnings })
}

fn power_pairs(degree: u8) -> Vec<(u8, u8)> {
    let mut powers = Vec::new();
    for i in 0..=degree {
        for j in 0..=(degree - i) {
            if i + j > 0 {
                powers.push((i, j));
            }
        }
    }
    powers
}

fn power_table(values: &[f64], max_pow: usize) -> Vec<f64> {
    let width = max_pow + 1;
    let mut table = vec![1.0; values.len() * width];
    for (k, &x) in values.iter().enumerate() {
        let row = &mut table[k * width..(k + 1) * width];
        for p in 1..width {
            row[p] = row[p - 1] * x;
        }
    }
    table
}

fn poly_fit_lu(s: &Samples, degree: u8) -> Option<(Vec<f64>, Vec<f64>, Vec<(u8, u8)>, f64)> {
    let powers = power_pairs(degree);
    let nterms = powers.len();
    let n = s.u.len();
    let width = 2 * degree as usize + 1;
    let up = power_table(&s.u, width - 1);
    let vp = power_table(&s.v, width - 1);
    let mut buf = vec![0.0; n];
    let mut a = vec![vec![0.0; nterms]; nterms];
    let mut bx = vec![0.0; nterms];
    let mut by = vec![0.0; nterms];
    for i in 0..nterms {
        let (pi, qi) = (powers[i].0 as usize, powers[i].1 as usize);
        for k in 0..n {
            buf[k] = s.x[k] * (up[k * width + pi] * vp[k * width + qi]);
        }
        bx[i] = pairwise_sum(&buf);
        for k in 0..n {
            buf[k] = s.y[k] * (up[k * width + pi] * vp[k * width + qi]);
        }
        by[i] = pairwise_sum(&buf);
        for j in i..nterms {
            let (pj, qj) = (powers[j].0 as usize, powers[j].1 as usize);
            for k in 0..n {
                buf[k] = up[k * width + pi + pj] * vp[k * width + qi + qj];
            }
            a[i][j] = pairwise_sum(&buf);
            a[j][i] = a[i][j];
        }
    }
    let mut a_y = a.clone();
    lu_solve(&mut a, &mut bx)?;
    lu_solve(&mut a_y, &mut by)?;
    let resid = max_residual(s, &powers, &bx, &by);
    resid.is_finite().then_some((bx, by, powers, resid))
}

fn poly_eval(powers: &[(u8, u8)], c: &[f64], u: f64, v: f64) -> f64 {
    powers.iter().zip(c).map(|(&(p, q), &c)| c * u.powi(p as i32) * v.powi(q as i32)).sum()
}

fn max_residual(s: &Samples, powers: &[(u8, u8)], cx: &[f64], cy: &[f64]) -> f64 {
    (0..s.u.len())
        .map(|k| {
            let dx = s.x[k] - poly_eval(powers, cx, s.u[k], s.v[k]);
            let dy = s.y[k] - poly_eval(powers, cy, s.u[k], s.v[k]);
            dx.hypot(dy)
        })
        .fold(0.0, |m, d| if d.is_nan() || d > m { d } else { m })
}

fn reform(fit: &PolyFit) -> Result<([[f64; 2]; 2], Vec<(u8, u8, f64)>, Vec<(u8, u8, f64)>), GwcsError> {
    let coeff = |c: &[f64], p: (u8, u8)| fit.powers.iter().position(|&q| q == p).map_or(0.0, |i| c[i]);
    let cd = [
        [coeff(&fit.cx, (1, 0)), coeff(&fit.cx, (0, 1))],
        [coeff(&fit.cy, (1, 0)), coeff(&fit.cy, (0, 1))],
    ];
    let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
    if !(det.abs() >= SINGULAR_CD_DET) {
        return Err(refusal("SIP fit produced a singular CD matrix"));
    }
    let inv = [[cd[1][1] / det, -cd[0][1] / det], [-cd[1][0] / det, cd[0][0] / det]];
    let mut a = Vec::new();
    let mut b = Vec::new();
    for (k, &(i, j)) in fit.powers.iter().enumerate() {
        if i + j < 2 {
            continue;
        }
        let (x, y) = (fit.cx[k], fit.cy[k]);
        a.push((i, j, inv[0][0] * x + inv[0][1] * y));
        b.push((i, j, inv[1][0] * x + inv[1][1] * y));
    }
    Ok((cd, a, b))
}

pub fn lu_solve(a: &mut [Vec<f64>], b: &mut [f64]) -> Option<()> {
    let n = b.len();
    if a.len() != n || a.iter().any(|row| row.len() != n) {
        return None;
    }
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().partial_cmp(&a[j][col].abs()).unwrap_or(Ordering::Equal))?;
        let p = a[pivot][col];
        if !(p.is_finite() && p != 0.0) {
            return None;
        }
        if pivot != col {
            a.swap(pivot, col);
            b.swap(pivot, col);
        }
        for row in col + 1..n {
            let factor = a[row][col] / p;
            if factor == 0.0 {
                continue;
            }
            for k in col..n {
                a[row][k] -= factor * a[col][k];
            }
            b[row] -= factor * b[col];
        }
    }
    for i in (0..n).rev() {
        let mut s = b[i];
        for k in i + 1..n {
            s -= a[i][k] * b[k];
        }
        b[i] = s / a[i][i];
    }
    b.iter().all(|v| v.is_finite()).then_some(())
}

fn pairwise_sum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        return a.iter().fold(0.0, |s, v| s + v);
    }
    if n <= 128 {
        let mut r = [0.0; 8];
        r.copy_from_slice(&a[..8]);
        let mut i = 8;
        while i < n - (n % 8) {
            for j in 0..8 {
                r[j] += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        return res;
    }
    let mut n2 = n / 2;
    n2 -= n2 % 8;
    pairwise_sum(&a[..n2]) + pairwise_sum(&a[n2..])
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{assert_close, fixture_pipeline, truth, unwrap_pair};
    use super::*;
    use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
    use crate::types::HduHeader;

    fn header_from(fit: &SipFit, naxis: (usize, usize)) -> HduHeader {
        let mut h = HduHeader::empty();
        h.set("NAXIS1", naxis.0.to_string());
        h.set("NAXIS2", naxis.1.to_string());
        for (k, v) in fit.header_cards() {
            h.set(&k, v);
        }
        h
    }

    fn pipeline_bbox(p: &GwcsPipeline) -> [[f64; 2]; 2] {
        p.bounding_box.expect("fixture has a bounding box").intervals
    }

    fn naxis_of(bbox: [[f64; 2]; 2]) -> (usize, usize) {
        ((bbox[0][1] + 0.5).round() as usize, (bbox[1][1] + 0.5).round() as usize)
    }

    fn linspace(lo: f64, hi: f64, n: usize) -> Vec<f64> {
        (0..n).map(|k| lo + (hi - lo) * k as f64 / (n - 1) as f64).collect()
    }

    fn assert_rel(got: f64, gwcs: f64, rel: f64, what: &str) {
        let diff = ((got - gwcs) / gwcs).abs();
        assert!(diff <= rel, "{what}: got {got:e}, gwcs 1.0.3 {gwcs:e} (relative diff {diff:e} > {rel:e})");
    }

    #[derive(Default)]
    struct RoundTrip {
        max_forward_px: f64,
        max_inverse_px: f64,
    }

    impl RoundTrip {
        fn check(&mut self, wcs: &WcsTransform, fit: &SipFit, pixel: [f64; 2], world: [f64; 2], what: &str) {
            let [x, y] = pixel;
            let sky = wcs.pixel_to_world(x, y);
            let sep_px = angular_separation(sky.ra, sky.dec, world[0], world[1]) / fit.plate_scale_deg;
            assert!(sep_px.is_finite(), "{what}: header SIP at ({x}, {y}) gave {sky:?} for {world:?}");
            self.max_forward_px = self.max_forward_px.max(sep_px);
            let (bx, by) = wcs.world_to_pixel(world[0], world[1]);
            let back_px = (bx - x).hypot(by - y);
            assert!(back_px.is_finite(), "{what}: header inverse at ({x}, {y}) gave ({bx}, {by})");
            self.max_inverse_px = self.max_inverse_px.max(back_px);
        }

        fn assert_within(&self, fit: &SipFit, what: &str) {
            let forward_bound = (1.5 * fit.max_err_px).max(1e-6);
            assert!(self.max_forward_px <= forward_bound, "{what}: forward {} > {forward_bound}", self.max_forward_px);
            let inv = fit.inv_err_px.unwrap_or_else(|| panic!("{what}: no inverse fit"));
            let inverse_bound = (1.5 * inv).max(1e-6);
            assert!(self.max_inverse_px <= inverse_bound, "{what}: inverse {} > {inverse_bound}", self.max_inverse_px);
        }
    }

    fn round_trip(fit: &SipFit, p: &GwcsPipeline, bbox: [[f64; 2]; 2], n: usize, what: &str) -> RoundTrip {
        let wcs = WcsTransform::from_header(&header_from(fit, naxis_of(bbox))).unwrap_or_else(|e| panic!("{what}: {e:#}"));
        let mut rt = RoundTrip::default();
        for y in linspace(bbox[1][0], bbox[1][1], n) {
            for x in linspace(bbox[0][0], bbox[0][1], n) {
                rt.check(&wcs, fit, [x, y], p.forward(x, y, false), what);
            }
        }
        rt
    }

    fn err_text(r: Result<SipFit, GwcsError>) -> String {
        match r {
            Err(e) => format!("{e:#}"),
            Ok(fit) => panic!("expected a refusal, got {fit:?}"),
        }
    }

    #[test]
    fn lu_solve_solves_a_well_conditioned_system() {
        let mut a = vec![vec![4.0, -2.0, 1.0], vec![-2.0, 4.0, -2.0], vec![1.0, -2.0, 4.0]];
        let mut b = [11.0, -16.0, 17.0];
        assert!(lu_solve(&mut a, &mut b).is_some());
        for (got, exp) in b.iter().zip([1.0, -2.0, 3.0]) {
            assert_close(*got, exp, 1e-13, "lu solution");
        }
        let mut needs_pivot = vec![vec![0.0, 1.0], vec![1.0, 0.0]];
        let mut rhs = [2.0, 3.0];
        assert!(lu_solve(&mut needs_pivot, &mut rhs).is_some());
        assert_eq!(rhs, [3.0, 2.0]);
        let mut singular = vec![vec![1.0, 2.0], vec![2.0, 4.0]];
        let mut rhs = [1.0, 2.0];
        assert!(lu_solve(&mut singular, &mut rhs).is_none());
    }

    #[test]
    fn pairwise_sum_groups_terms_like_numpy() {
        let a: Vec<f64> = (0..1000).map(|k| k as f64 * 0.1 + 1e15 * (k % 7) as f64).collect();
        let naive = a.iter().fold(0.0, |s, v| s + v);
        assert_eq!(pairwise_sum(&a), 2.99700000000005e18);
        assert_ne!(naive, 2.99700000000005e18);
        let b = [1.0, 1e16, -1e16, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        assert_eq!(pairwise_sum(&b), 44.0);
        assert_eq!(pairwise_sum(&[0.5, 0.25]), 0.75);
        assert_eq!(pairwise_sum(&[]), 0.0);
    }

    #[test]
    fn the_sampling_grid_matches_numpy_mgrid() {
        let (u, v) = sampling_grid(32, [[-0.5, 2047.5], [-0.5, 1023.5]], [1023.5, 511.5]);
        assert_eq!((u.len(), v.len()), (1024, 1024));
        assert_eq!((u[0], v[0]), (-1024.0, -512.0));
        assert_eq!((u[1], v[1]), (-1024.0 + 2048.0 / 31.0, -512.0));
        assert_eq!((u[32], v[32]), (-1024.0, -512.0 + 1024.0 / 31.0));
        assert_close(u[1023], 1024.0, 1e-12, "last u");
        assert_close(v[1023], 512.0, 1e-12, "last v");
        let (ud, _) = sampling_grid(64, [[-0.5, 4087.5], [-0.5, 4087.5]], [2043.5, 2043.5]);
        assert_eq!(ud.len(), 4096);
    }

    #[test]
    fn a_pure_tan_chain_fits_at_degree_one() {
        let p = fixture_pipeline("wcs_gwcs_examples_tan.asdf");
        let bbox = [[-0.5, 999.5], [-0.5, 999.5]];
        let fit = fit_tan_sip(&|x, y| p.forward(x, y, false), bbox, &SipFitOptions::default())
            .unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(fit.a_order, 1);
        assert!(!fit.has_sip());
        assert_rel(fit.max_err_px, 8.310360364150786e-8, 0.1, "degree-1 max_err_px");
        assert_eq!(fit.ap_order, None);
        assert_eq!(fit.inv_err_px, None);
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);
        let centre = p.forward(499.5, 499.5, false);
        assert_close(centre[0], 29.99998035814972, 1e-9, "fixture centre ra");
        assert_close(centre[1], 44.999986111109436, 1e-9, "fixture centre dec");
        assert_close(fit.crval[0], centre[0], 1e-9, "crval1");
        assert_close(fit.crval[1], centre[1], 1e-9, "crval2");
        assert_eq!(fit.crpix, [500.5, 500.5]);
        let header = header_from(&fit, (1000, 1000));
        assert_eq!(header.get("CTYPE1"), Some("RA---TAN"));
        assert_eq!(header.get("CTYPE2"), Some("DEC--TAN"));
        assert_eq!(header.get("A_ORDER"), None);
        assert_eq!(header.get("SIPMXERR"), None);
        let wcs = WcsTransform::from_header(&header).unwrap_or_else(|e| panic!("{e:#}"));
        for (x, y) in [(-0.5, -0.5), (999.5, -0.5), (999.5, 999.5), (-0.5, 999.5)] {
            let truth = p.forward(x, y, false);
            let sky = wcs.pixel_to_world(x, y);
            assert_close(sky.ra, truth[0], 1e-9, &format!("ra at ({x}, {y})"));
            assert_close(sky.dec, truth[1], 1e-9, &format!("dec at ({x}, {y})"));
        }
    }

    #[test]
    fn nircam_chain_needs_degree_five_at_0_01_px() {
        let p = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let fit = fit_pipeline_tan_sip(&p, &SipFitOptions::default(), None).unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(fit.a_order, 5);
        assert_eq!(fit.ap_order, Some(5));
        assert_rel(fit.max_err_px, 9.220195712301538e-9, 0.1, "nircam max_err_px");
        let inv = fit.inv_err_px.expect("inverse fitted");
        assert_rel(inv, 3.593143551386226e-4, 1e-4, "nircam inv_err_px");
        assert!(fit.warnings.is_empty(), "{:?}", fit.warnings);
        assert_eq!((fit.a.len(), fit.b.len(), fit.ap.len(), fit.bp.len()), (18, 18, 20, 20));
        round_trip(&fit, &p, pipeline_bbox(&p), 21, "nircam").assert_within(&fit, "nircam");
    }

    #[test]
    fn miri_chain_fits_within_the_target() {
        let p = fixture_pipeline("wcs_jwst_miri_cal300.asdf");
        let fit = fit_pipeline_tan_sip(&p, &SipFitOptions::default(), None).unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(fit.a_order, 4);
        assert_rel(fit.max_err_px, 5.637965119758063e-7, 0.1, "miri max_err_px");
        assert_eq!(fit.ap_order, Some(6));
        let inv = fit.inv_err_px.expect("inverse fitted");
        assert_rel(inv, 3.6081930089466386e-3, 1e-4, "miri inv_err_px");
        assert_eq!(fit.crpix, [516.5, 512.5]);
    }

    #[test]
    fn a_fixed_low_degree_reports_its_residual_and_warns() {
        let opts = SipFitOptions { degree: Some(vec![3]), ..SipFitOptions::default() };
        for (name, gwcs_max_err_px, gwcs_inv_err_px, inv_order) in [
            ("wcs_jwst_nircam_cal300.asdf", 0.052884467263122854, 3.593143551242954e-4, 5),
            ("wcs_jwst_miri_cal300.asdf", 1.242761975086825, 3.6081930097327463e-3, 6),
        ] {
            let p = fixture_pipeline(name);
            let fit = fit_pipeline_tan_sip(&p, &opts, None).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            assert_eq!(fit.a_order, 3, "{name}");
            assert_rel(fit.max_err_px, gwcs_max_err_px, 1e-4, &format!("{name} degree-3 max_err_px"));
            assert_eq!(fit.ap_order, Some(inv_order), "{name}");
            let inv = fit.inv_err_px.unwrap_or_else(|| panic!("{name}: no inverse fit"));
            assert_rel(inv, gwcs_inv_err_px, 1e-4, &format!("{name} degree-3 inv_err_px"));
            assert!(fit.warnings.iter().any(|w| w == ACCURACY_WARNING), "{name}: {:?}", fit.warnings);
            assert!(fit.warnings.iter().any(|w| w == DOUBLE_SAMPLING_WARNING), "{name}: {:?}", fit.warnings);
            let header = header_from(&fit, (2048, 2048));
            assert_eq!(header.get_f64("SIPMXERR"), Some(fit.max_err_px), "{name}");
            assert_eq!(header.get_f64("SIPIVERR"), Some(inv), "{name}");
        }
    }

    #[test]
    fn crpix_rounds_half_to_even_like_python() {
        let eval = |x: f64, y: f64| [30.0 + 1e-5 * x, 45.0 + 1e-5 * y];
        let bbox = [[-0.5, 2047.0], [-0.5, 2047.0]];
        let fit = fit_tan_sip(&eval, bbox, &SipFitOptions::default()).unwrap_or_else(|e| panic!("{e:#}"));
        assert_close(fit.crpix[0], 1024.2, 1e-9, "crpix1 from a 1023.25 centre");
        assert_close(fit.crpix[1], 1024.2, 1e-9, "crpix2 from a 1023.25 centre");
        let given = SipFitOptions { crpix: Some([100.0, 200.0]), ..SipFitOptions::default() };
        let fit = fit_tan_sip(&eval, bbox, &given).unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(fit.crpix, [100.0, 200.0]);
        assert_close(fit.crval[0], 30.0 + 1e-5 * 99.0, 1e-12, "crval1 at the given 0-based crpix");
        assert_close(fit.crval[1], 45.0 + 1e-5 * 199.0, 1e-12, "crval2 at the given 0-based crpix");
    }

    #[test]
    fn header_cards_round_trip_through_the_app_sip_code() {
        let p = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let fit = fit_pipeline_tan_sip(&p, &SipFitOptions::default(), None).unwrap_or_else(|e| panic!("{e:#}"));
        let header = header_from(&fit, (2048, 2048));
        for (key, value) in [
            ("CTYPE1", "RA---TAN-SIP"),
            ("CTYPE2", "DEC--TAN-SIP"),
            ("WCSAXES", "2"),
            ("RADESYS", "ICRS"),
            ("LONPOLE", "180"),
            ("CUNIT1", "deg"),
            ("CUNIT2", "deg"),
            ("CRPIX1", "1024.5"),
            ("CRPIX2", "1024.5"),
            ("A_ORDER", "5"),
            ("B_ORDER", "5"),
            ("AP_ORDER", "5"),
            ("BP_ORDER", "5"),
        ] {
            assert_eq!(header.get(key), Some(value), "{key}");
        }
        let sipmxerr = header.get("SIPMXERR").expect("SIPMXERR card");
        assert!(sipmxerr.contains("E-"), "{sipmxerr}");
        assert_eq!(header.get_f64("SIPMXERR"), Some(fit.max_err_px));
        assert_eq!(header.get_f64("SIPIVERR"), fit.inv_err_px);
        assert!(header.get("A_2_0").expect("A_2_0").contains('E'));
        let count = |prefix: &str| header.cards.iter().filter(|(k, _)| k.starts_with(prefix) && k.chars().nth(prefix.len()).is_some_and(|c| c.is_ascii_digit())).count();
        assert_eq!((count("A_"), count("B_"), count("AP_"), count("BP_")), (18, 18, 20, 20));
        for (k, v) in &header.cards {
            assert!(k.is_ascii() && v.is_ascii() && v.len() <= 70, "{k} = {v}");
        }
        let wcs = WcsTransform::from_header(&header).unwrap_or_else(|e| panic!("{e:#}"));
        let o = wcs.orientation(2048, 2048);
        assert!(o.sip_present);
        assert_eq!(o.projection, "TAN");
        assert_eq!(wcs.sip_fit_residuals(), (Some(fit.max_err_px), fit.inv_err_px));
    }

    #[test]
    fn too_small_a_box_or_too_few_points_is_refused() {
        let eval = |x: f64, y: f64| [30.0 + 1e-5 * x, 45.0 + 1e-5 * y];
        let box_100 = [[-0.5, 99.5], [-0.5, 99.5]];
        let few = SipFitOptions { npoints: 7, ..SipFitOptions::default() };
        let text = err_text(fit_tan_sip(&eval, box_100, &few));
        assert!(text.contains("Number of sampling points is too small. 'npoints' must be >= 8."), "{text}");
        let text = err_text(fit_tan_sip(&eval, [[-0.5, 0.4], [-0.5, 99.5]], &SipFitOptions::default()));
        assert!(text.contains("Bounding box is too small for fitting a SIP polynomial"), "{text}");
        let nan = |_: f64, _: f64| [f64::NAN, f64::NAN];
        let text = err_text(fit_tan_sip(&nan, box_100, &SipFitOptions::default()));
        assert!(text.contains("gWCS is not finite at the fit reference pixel"), "{text}");
        let bad_degree = SipFitOptions { degree: Some(vec![0]), ..SipFitOptions::default() };
        let text = err_text(fit_tan_sip(&eval, box_100, &bad_degree));
        assert!(text.contains("Allowed values for SIP degree are [1...9]"), "{text}");
        let p = fixture_pipeline("wcs_gwcs_examples_tan.asdf");
        let text = err_text(fit_pipeline_tan_sip(&p, &SipFitOptions::default(), None));
        assert!(text.contains("a bounding box or the array dimensions are needed to fit a SIP header"), "{text}");
        let fit = fit_pipeline_tan_sip(&p, &SipFitOptions::default(), Some((1000, 1000))).unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(fit.crpix, [500.5, 500.5]);
    }

    #[test]
    #[ignore]
    fn gwcs_bench_sip_fit_cost() {
        let p = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let start = std::time::Instant::now();
        let runs = 5;
        let mut fit = None;
        for _ in 0..runs {
            fit = Some(fit_pipeline_tan_sip(&p, &SipFitOptions::default(), None).unwrap_or_else(|e| panic!("{e:#}")));
        }
        let per_fit = start.elapsed() / runs;
        let fit = fit.expect("fitted");
        println!(
            "sip fit (NIRCam cal300, npoints 32, degree {} / inverse {:?}): {:.1} ms per fit",
            fit.a_order,
            fit.ap_order,
            per_fit.as_secs_f64() * 1e3
        );
        assert!(per_fit.as_millis() < 2000, "{per_fit:?}");
    }

    #[test]
    fn derived_fits_round_trip_fit_evaluate_compare() {
        for (name, truth_name, orders, gwcs_max_err_px, gwcs_inv_err_px) in [
            ("wcs_jwst_nircam_cal300.asdf", "jw02739001001_02105_00001_nrca1", (5, 5), 9.220195712301538e-9, 3.593143551386226e-4),
            ("wcs_jwst_miri_cal300.asdf", "jw02739002001_02101_00001_mirimage", (4, 6), 5.637965119758063e-7, 3.6081930089466386e-3),
            ("wcs_jwst_fgs_cal300.asdf", "jw01476003004_02201_00001_guider2", (4, 5), 3.8900196174779237e-7, 1.8852911657631644e-3),
            ("wcs_nircam_gwcs_fixture.asdf", "gwcs_nircamwcs", (5, 5), 7.220155855680387e-8, 3.8013234248764735e-3),
        ] {
            let p = fixture_pipeline(name);
            let fit = fit_pipeline_tan_sip(&p, &SipFitOptions::default(), Some((2048, 2048)))
                .unwrap_or_else(|e| panic!("{name}: {e:#}"));
            let inv = fit.inv_err_px.unwrap_or_else(|| panic!("{name}: no inverse fit"));
            assert_eq!((fit.a_order, fit.ap_order), (orders.0, Some(orders.1)), "{name}");
            assert_rel(fit.max_err_px, gwcs_max_err_px, 0.1, &format!("{name} max_err_px"));
            assert_rel(inv, gwcs_inv_err_px, 1e-4, &format!("{name} inv_err_px"));
            assert!(fit.warnings.is_empty(), "{name}: {:?}", fit.warnings);
            let bbox = p.bounding_box.map(|b| b.intervals).unwrap_or([[-0.5, 2047.5], [-0.5, 2047.5]]);
            round_trip(&fit, &p, bbox, 33, name).assert_within(&fit, name);
            let wcs = WcsTransform::from_header(&header_from(&fit, naxis_of(bbox))).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            let mut against_gwcs = RoundTrip::default();
            let truth = truth(truth_name);
            for point in &truth.points {
                let world = unwrap_pair(&point.world_no_bbox);
                if !(world[0].is_finite() && world[1].is_finite()) {
                    continue;
                }
                against_gwcs.check(&wcs, &fit, point.pixel, world, &format!("{name} vs gwcs truth {}", point.label));
            }
            against_gwcs.assert_within(&fit, &format!("{name} vs gwcs truth"));
        }
    }

    #[test]
    fn a_nan_sample_makes_the_residual_nan_instead_of_vanishing() {
        let s = Samples { u: vec![0.0, 1.0, 2.0], v: vec![0.0, 1.0, 0.5], x: vec![0.0, f64::NAN, 2.0], y: vec![0.0, 1.0, 0.5] };
        let powers = [(1u8, 0u8), (0, 1)];
        let finite = Samples { u: s.u.clone(), v: s.v.clone(), x: vec![0.0, 1.0, 2.0], y: s.y.clone() };
        assert_eq!(max_residual(&finite, &powers, &[1.0, 0.0], &[0.0, 1.0]), 0.0, "an exact linear fit has no residual");
        assert!(max_residual(&s, &powers, &[1.0, 0.0], &[0.0, 1.0]).is_nan(), "numpy's dist.max() is NaN when one sample is NaN");
    }
}
