use super::inverse::newton_exact;
pub use super::inverse::NewtonOptions;
use super::transform::{Kind, Node};
pub use super::transform::Vals;
use super::GwcsError;

#[derive(Debug, Clone, PartialEq)]
pub enum FrameKind {
    Frame2D,
    Celestial { reference: String },
    Other(String),
}

#[derive(Debug, Clone)]
pub struct GwcsFrame {
    pub name: String,
    pub kind: FrameKind,
    pub unit: Vec<String>,
    pub axes_names: Vec<String>,
    pub axis_physical_types: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GwcsStep {
    pub frame: GwcsFrame,
    pub transform: Option<Node>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub intervals: [[f64; 2]; 2],
    pub ignore: [bool; 2],
}

impl BoundingBox {
    pub fn square(lo: f64, hi: f64) -> Self {
        Self { intervals: [[lo, hi], [lo, hi]], ignore: [false, false] }
    }

    fn outside(&self, axis: usize, v: f64) -> bool {
        !self.ignore[axis] && (v < self.intervals[axis][0] || v > self.intervals[axis][1])
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InverseResult {
    pub pixel: [f64; 2],
    pub iterations: u8,
    pub converged: bool,
    pub residual_px: f64,
}

#[derive(Debug, Clone)]
pub struct GwcsPipeline {
    pub steps: Vec<GwcsStep>,
    pub bounding_box: Option<BoundingBox>,
    pub name: String,
    pub source_key: String,
    forward: Node,
    backward: Result<Node, GwcsError>,
}

impl GwcsPipeline {
    pub fn new(
        steps: Vec<GwcsStep>,
        bounding_box: Option<BoundingBox>,
        name: String,
        source_key: String,
    ) -> Result<Self, GwcsError> {
        let transforms: Vec<&Node> = steps.iter().filter_map(|s| s.transform.as_ref()).collect();
        if transforms.is_empty() {
            return Err(GwcsError::Shape { path: "steps".into(), reason: "no step has a transform".into() });
        }
        let mut forward = transforms[0].clone();
        for (k, next) in transforms.iter().enumerate().skip(1) {
            if forward.n_outputs != next.n_inputs {
                return Err(GwcsError::Shape {
                    path: format!("steps[{k}].transform"),
                    reason: format!("step takes {} inputs but the previous step produces {}", next.n_inputs, forward.n_outputs),
                });
            }
            forward = Node::new(Kind::Compose(Box::new(forward), Box::new((*next).clone())), "compose", "steps");
        }
        if forward.n_inputs != 2 || forward.n_outputs != 2 {
            return Err(GwcsError::Shape {
                path: "steps".into(),
                reason: format!("forward chain maps {} inputs to {} outputs; 2 -> 2 is required", forward.n_inputs, forward.n_outputs),
            });
        }
        let backward = Self::build_backward(&transforms);
        Ok(Self { steps, bounding_box, name, source_key, forward, backward })
    }

    fn build_backward(transforms: &[&Node]) -> Result<Node, GwcsError> {
        let mut acc: Option<Node> = None;
        for t in transforms.iter().rev() {
            let inv = t.inverse()?;
            acc = Some(match acc {
                None => inv,
                Some(prev) => {
                    if prev.n_outputs != inv.n_inputs {
                        return Err(GwcsError::Shape {
                            path: t.path.clone(),
                            reason: format!("inverse takes {} inputs but the previous inverse produces {}", inv.n_inputs, prev.n_outputs),
                        });
                    }
                    Node::new(Kind::Compose(Box::new(prev), Box::new(inv)), "compose", "steps")
                }
            });
        }
        acc.ok_or_else(|| GwcsError::Shape { path: "steps".into(), reason: "no step has a transform".into() })
    }

    pub fn n_steps(&self) -> usize {
        self.steps.iter().filter(|s| s.transform.is_some()).count()
    }

    pub fn frame_names(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.frame.name.clone()).collect()
    }

    pub fn frames_summary(&self) -> String {
        self.frame_names().join("->")
    }

    pub fn backward_error(&self) -> Option<&GwcsError> {
        self.backward.as_ref().err()
    }

    pub fn forward_node(&self) -> &Node {
        &self.forward
    }

    pub fn in_bbox(&self, x: f64, y: f64) -> bool {
        match &self.bounding_box {
            None => true,
            Some(b) => !(b.outside(0, x) || b.outside(1, y)),
        }
    }

    pub fn forward(&self, x: f64, y: f64, with_bbox: bool) -> [f64; 2] {
        if with_bbox && !self.in_bbox(x, y) {
            return [f64::NAN, f64::NAN];
        }
        let out = self.forward.eval(Vals::pair(x, y));
        [out.v[0], out.v[1]]
    }

    pub fn forward_frames(&self, x: f64, y: f64) -> Vec<(String, Vals)> {
        let mut out = Vec::with_capacity(self.steps.len());
        let mut vals = Vals::pair(x, y);
        for k in 0..self.steps.len() {
            let Some(t) = &self.steps[k].transform else { continue };
            vals = t.eval(vals);
            let name = self.steps.get(k + 1).map(|s| s.frame.name.clone()).unwrap_or_default();
            out.push((name, vals));
        }
        out
    }

    pub fn backward_analytic(&self, lon: f64, lat: f64) -> Result<[f64; 2], GwcsError> {
        let backward = self.backward.as_ref().map_err(Clone::clone)?;
        let out = backward.eval(Vals::pair(lon, lat));
        Ok([out.v[0], out.v[1]])
    }

    pub fn backward_frames(&self, lon: f64, lat: f64) -> Result<Vec<(String, Vals)>, GwcsError> {
        let mut out = Vec::with_capacity(self.steps.len());
        let mut vals = Vals::pair(lon, lat);
        for step in self.steps.iter().rev() {
            let Some(t) = &step.transform else { continue };
            vals = t.inverse()?.eval(vals);
            out.push((step.frame.name.clone(), vals));
        }
        Ok(out)
    }

    pub fn backward_exact(&self, lon: f64, lat: f64, opts: &NewtonOptions) -> InverseResult {
        let start = self
            .backward_analytic(lon, lat)
            .ok()
            .or_else(|| self.bbox_centre())
            .unwrap_or([f64::NAN, f64::NAN]);
        newton_exact(&|p| self.forward(p[0], p[1], false), [lon, lat], start, opts)
    }

    pub fn out_of_bounds(&self, p: [f64; 2]) -> [f64; 2] {
        let Some(b) = &self.bounding_box else { return p };
        [
            if b.outside(0, p[0]) { f64::NAN } else { p[0] },
            if b.outside(1, p[1]) { f64::NAN } else { p[1] },
        ]
    }

    pub fn invert_gwcs_semantics(&self, lon: f64, lat: f64) -> Result<[f64; 2], GwcsError> {
        let Some(footprint) = self.footprint() else {
            return self.backward_analytic(lon, lat);
        };
        let mut world = [lon, lat];
        for (axis, w) in world.iter_mut().enumerate() {
            let values: Vec<f64> = footprint.iter().map(|c| c[axis]).collect();
            let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let outside = if axis == 0 && max - min > 180.0 {
                let d = 0.5 * (min + max);
                let lower = values.iter().cloned().filter(|v| *v <= d).fold(f64::NEG_INFINITY, f64::max);
                let upper = values.iter().cloned().filter(|v| *v > d).fold(f64::INFINITY, f64::min);
                *w > lower && *w < upper
            } else {
                *w < min || *w > max
            };
            if outside {
                *w = f64::NAN;
            }
        }
        let pixel = self.backward_analytic(world[0], world[1])?;
        Ok(self.out_of_bounds(pixel))
    }

    pub fn footprint(&self) -> Option<[[f64; 2]; 4]> {
        let b = self.bounding_box?;
        let [[x0, x1], [y0, y1]] = b.intervals;
        Some([
            self.forward(x0, y0, false),
            self.forward(x0, y1, false),
            self.forward(x1, y1, false),
            self.forward(x1, y0, false),
        ])
    }

    pub fn bbox_centre(&self) -> Option<[f64; 2]> {
        let b = self.bounding_box?;
        Some([0.5 * (b.intervals[0][0] + b.intervals[0][1]), 0.5 * (b.intervals[1][0] + b.intervals[1][1])])
    }

    pub fn local_jacobian_deg_per_px(&self, x: f64, y: f64, h_px: f64) -> [[f64; 2]; 2] {
        let centre = self.forward(x, y, false);
        let cos_dec = centre[1].to_radians().cos();
        let px = self.forward(x + h_px, y, false);
        let mx = self.forward(x - h_px, y, false);
        let py = self.forward(x, y + h_px, false);
        let my = self.forward(x, y - h_px, false);
        let scale = 1.0 / (2.0 * h_px);
        [
            [wrap_delta_deg(px[0] - mx[0]) * scale * cos_dec, wrap_delta_deg(py[0] - my[0]) * scale * cos_dec],
            [(px[1] - mx[1]) * scale, (py[1] - my[1]) * scale],
        ]
    }
}

fn wrap_delta_deg(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::super::parse::{find_wcs_node, parse_gwcs, wcs_node_at, WcsNodeClass};
    use super::super::test_support::*;
    use super::super::transform::Axis;
    use super::*;
    use crate::infra::asdf::parser::AsdfFile;

    const WORLD_TOL: f64 = 1e-10;
    const ARCSEC_TOL: f64 = 1e-9;
    const ANALYTIC_TOL: f64 = 1e-7;
    const EXACT_TOL: f64 = 1e-6;

    fn check_forward(pipeline: &GwcsPipeline, truth: &TruthFile) {
        assert_eq!(pipeline.bounding_box.map(|b| b.intervals), truth.bounding_box, "{} bounding box", truth.source_id);
        for p in &truth.points {
            let [x, y] = p.pixel;
            let what = format!("{} {} ({x}, {y})", truth.source_id, p.label);
            assert_eq!(pipeline.in_bbox(x, y), p.inside_bbox, "{what} inside_bbox");
            assert_nan_pattern(pipeline.forward(x, y, true), pair(&p.world), WORLD_TOL, &format!("{what} world"));
            assert_nan_pattern(pipeline.forward(x, y, false), pair(&p.world_no_bbox), WORLD_TOL, &format!("{what} world_no_bbox"));
            for (frame, vals) in pipeline.forward_frames(x, y) {
                let Some(exp) = p.frames_forward.get(&frame) else { panic!("{what}: truth lacks frame {frame}") };
                let tol = if frame == *truth.frames.last().unwrap() { WORLD_TOL } else { ARCSEC_TOL };
                assert_close_slice(vals.as_slice(), &exp.iter().map(|v| v.unwrap_or(f64::NAN)).collect::<Vec<_>>(), tol, &format!("{what} frame {frame}"));
            }
        }
    }

    fn check_inverse(pipeline: &GwcsPipeline, truth: &TruthFile) {
        for p in &truth.points {
            let what = format!("{} {}", truth.source_id, p.label);
            let [lon, lat] = unwrap_pair(&p.world_no_bbox);
            let analytic = pipeline.backward_analytic(lon, lat).unwrap_or_else(|e| panic!("{what}: {e:#}"));
            assert_nan_pattern(analytic, pair(p.invert_no_bbox.as_ref().unwrap()), ANALYTIC_TOL, &format!("{what} invert_no_bbox"));
            let own = pipeline.forward(p.pixel[0], p.pixel[1], false);
            let gwcs = pipeline.invert_gwcs_semantics(own[0], own[1]).unwrap();
            assert_nan_pattern(gwcs, pair(p.invert.as_ref().unwrap()), ANALYTIC_TOL, &format!("{what} invert"));
            if let Some(frames) = &p.frames_backward_from_world_no_bbox {
                for (frame, vals) in pipeline.backward_frames(lon, lat).unwrap() {
                    let exp: Vec<f64> = frames[&frame].iter().map(|v| v.unwrap_or(f64::NAN)).collect();
                    let tol = if frame == truth.frames[0] { ANALYTIC_TOL } else { ARCSEC_TOL };
                    assert_close_slice(vals.as_slice(), &exp, tol, &format!("{what} backward frame {frame}"));
                }
            }
            let exact = pipeline.backward_exact(lon, lat, &NewtonOptions::default());
            assert!(exact.converged, "{what}: Newton did not converge: {exact:?}");
            assert!(exact.iterations <= 4, "{what}: {exact:?}");
            assert_nan_pattern(exact.pixel, pair(p.exact_inverse_newton.as_ref().unwrap()), EXACT_TOL, &format!("{what} exact_inverse_newton"));
        }
    }

    #[test]
    fn nircam_cal300_matches_truth_forward() {
        let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        assert_eq!(pipeline.n_steps(), 3);
        assert_eq!(pipeline.frames_summary(), "detector->v2v3->v2v3vacorr->world");
        assert_eq!(pipeline.bounding_box, Some(BoundingBox::square(-0.5, 2047.5)));
        let truth = truth("jw02739001001_02105_00001_nrca1");
        assert_eq!(truth.points.len(), 122);
        check_forward(&pipeline, &truth);
        assert_close_slice(&pipeline.forward(0.0, 0.0, true), &[274.7253135310639, -13.875903855344635], WORLD_TOL, "(0,0)");
        assert_close_slice(&pipeline.forward(1023.5, 1023.5, true), &[274.7348047011612, -13.867263013140171], WORLD_TOL, "centre");
        assert_close_slice(&pipeline.forward(2047.5, 2047.5, true), &[274.7442670347447, -13.858761062760378], WORLD_TOL, "corner");
    }

    #[test]
    fn nircam_cal300_matches_truth_inverse() {
        let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let truth = truth("jw02739001001_02105_00001_nrca1");
        check_inverse(&pipeline, &truth);
        let corner = pipeline.backward_analytic(274.72530887834534, -13.875908119051436).unwrap();
        assert_close_slice(&corner, &[-0.5000749041269721, -0.4995578095400175], ANALYTIC_TOL, "analytic corner");
        let w = pipeline.forward(-0.5, -0.5, false);
        let gwcs = pipeline.invert_gwcs_semantics(w[0], w[1]).unwrap();
        assert_nan_pattern(gwcs, [None, Some(-0.4995578095400175)], ANALYTIC_TOL, "invert (-0.5,-0.5)");
        let w = pipeline.forward(2047.5, 2047.5, false);
        let far = pipeline.invert_gwcs_semantics(w[0], w[1]).unwrap();
        assert_nan_pattern(far, [Some(2047.4999660011813), None], ANALYTIC_TOL, "invert (2047.5,2047.5)");
        let inside = pipeline.invert_gwcs_semantics(274.7348047011612, -13.867263013140171).unwrap();
        assert_close_slice(&inside, &[1023.5, 1023.5], 1e-6, "invert of the centre is finite");
        let outside = pipeline.invert_gwcs_semantics(274.9, -13.0).unwrap();
        assert!(outside[0].is_nan() && outside[1].is_nan(), "world outside the footprint is blanked before inverting: {outside:?}");
    }

    #[test]
    fn miri_cal300_matches_truth_forward() {
        let pipeline = fixture_pipeline("wcs_jwst_miri_cal300.asdf");
        assert_eq!(pipeline.n_steps(), 3);
        check_forward(&pipeline, &truth("jw02739002001_02101_00001_mirimage"));
    }

    #[test]
    fn miri_cal300_matches_truth_inverse() {
        let pipeline = fixture_pipeline("wcs_jwst_miri_cal300.asdf");
        let truth = truth("jw02739002001_02101_00001_mirimage");
        check_inverse(&pipeline, &truth);
        let origin = truth.points.iter().find(|p| p.pixel == [0.0, 0.0]).unwrap();
        let [lon, lat] = unwrap_pair(&origin.world_no_bbox);
        let analytic = pipeline.backward_analytic(lon, lat).unwrap();
        assert_close_slice(&analytic, &[-0.017061996746065233, 0.11028247995545826], ANALYTIC_TOL, "MIRI analytic inverse is 0.11 px off");
        let exact = pipeline.backward_exact(lon, lat, &NewtonOptions::default());
        assert!(exact.converged && exact.pixel[0].abs() < 1e-8 && exact.pixel[1].abs() < 1e-8, "{exact:?}");
    }

    #[test]
    fn fgs_cal300_matches_truth_forward() {
        let pipeline = fixture_pipeline("wcs_jwst_fgs_cal300.asdf");
        check_forward(&pipeline, &truth("jw01476003004_02201_00001_guider2"));
    }

    #[test]
    fn fgs_cal300_matches_truth_inverse() {
        let pipeline = fixture_pipeline("wcs_jwst_fgs_cal300.asdf");
        check_inverse(&pipeline, &truth("jw01476003004_02201_00001_guider2"));
    }

    #[test]
    fn nircam_gwcs_fixture_matches_truth_forward() {
        for name in ["wcs_nircam_gwcs_fixture.asdf", "wcs_nircam_gwcs_fixture_std150.asdf"] {
            let pipeline = fixture_pipeline(name);
            assert_eq!(pipeline.n_steps(), 2, "{name}");
            assert_eq!(pipeline.frames_summary(), "detector->v2v3->world");
            check_forward(&pipeline, &truth("gwcs_nircamwcs"));
        }
    }

    #[test]
    fn nircam_gwcs_fixture_matches_truth_inverse() {
        for name in ["wcs_nircam_gwcs_fixture.asdf", "wcs_nircam_gwcs_fixture_std150.asdf"] {
            check_inverse(&fixture_pipeline(name), &truth("gwcs_nircamwcs"));
        }
    }

    #[test]
    fn miri_gwcs_fixture_matches_truth() {
        let pipeline = fixture_pipeline("wcs_miri_gwcs_fixture.asdf");
        assert_eq!(pipeline.forward_node().count_stored_inverses(), 8);
        let truth = truth("gwcs_miriwcs");
        check_forward(&pipeline, &truth);
        check_inverse(&pipeline, &truth);
    }

    #[test]
    fn gwcs_examples_tan_matches_truth_without_a_bbox() {
        let pipeline = fixture_pipeline("wcs_gwcs_examples_tan.asdf");
        assert_eq!(pipeline.n_steps(), 1);
        assert_eq!(pipeline.bounding_box, None);
        assert_eq!(pipeline.footprint(), None);
        let truth = truth("gwcs_examples_image_wcs");
        check_forward(&pipeline, &truth);
        check_inverse(&pipeline, &truth);
        for p in truth.points.iter().step_by(7) {
            assert_eq!(pipeline.forward(p.pixel[0], p.pixel[1], true), pipeline.forward(p.pixel[0], p.pixel[1], false));
        }
        assert_close_slice(&pipeline.forward(0.0, 0.0, true), &[29.980362905902044, 44.986109428818445], WORLD_TOL, "(0,0)");
    }

    #[test]
    fn bbox_blanks_all_outputs_outside_and_keeps_edges() {
        let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let edge = pipeline.forward(-0.5, -0.5, true);
        assert!(edge[0].is_finite() && edge[1].is_finite(), "{edge:?}");
        let outside = pipeline.forward(-0.5000001, 100.0, true);
        assert!(outside[0].is_nan() && outside[1].is_nan(), "{outside:?}");
        let unbounded = pipeline.forward(-0.5000001, 100.0, false);
        assert!(unbounded[0].is_finite() && unbounded[1].is_finite(), "{unbounded:?}");
        assert!(pipeline.in_bbox(2047.5, 2047.5));
        assert!(!pipeline.in_bbox(2047.5000001, 0.0));
        assert_eq!(pipeline.bbox_centre(), Some([1023.5, 1023.5]));
    }

    #[test]
    fn out_of_bounds_is_per_axis() {
        let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let out = pipeline.out_of_bounds([-0.50007, -0.4996]);
        assert!(out[0].is_nan(), "{out:?}");
        assert_eq!(out[1], -0.4996);
        let inside = pipeline.out_of_bounds([10.0, 2047.5]);
        assert_eq!(inside, [10.0, 2047.5]);
    }

    fn plate_scale_arcsec(cd: &[[f64; 2]; 2]) -> f64 {
        (cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]).abs().sqrt() * 3600.0
    }

    #[test]
    fn local_jacobian_gives_the_plate_scale() {
        let nircam = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let cd = nircam.local_jacobian_deg_per_px(1023.5, 1023.5, 0.5);
        assert_close(plate_scale_arcsec(&cd), 0.031227126739074955, 1e-7, "NIRCam scale at the bbox centre, gwcs 1.0.3 central differences h = 0.5 px");
        assert_close_slice(&cd[0], &[2.8424385001916973e-7, 8.695862277324659e-6], 1e-12, "NIRCam dRA cos(dec)/d(x, y)");
        assert_close_slice(&cd[1], &[8.643508859762505e-6, -2.7798482982177575e-7], 1e-12, "NIRCam dDec/d(x, y)");
        let fgs = fixture_pipeline("wcs_jwst_fgs_cal300.asdf");
        let cd = fgs.local_jacobian_deg_per_px(1023.5, 1023.5, 0.5);
        assert_close(fgs.forward(1023.5, 1023.5, false)[1].to_radians().cos(), 0.34991721595707936, 1e-9, "FGS cos(dec)");
        assert_close(plate_scale_arcsec(&cd), 0.06880870224069112, 1e-7, "FGS scale at the bbox centre (0.1163 without cos(dec))");
        let miri = fixture_pipeline("wcs_jwst_miri_cal300.asdf");
        let cd = miri.local_jacobian_deg_per_px(515.5, 511.5, 0.5);
        assert_close(plate_scale_arcsec(&cd), 0.11088396380052763, 1e-7, "MIRI scale at the bbox centre");
    }

    #[test]
    fn local_jacobian_crosses_the_ra_wrap() {
        let wrap = synthetic_v23tosky(359.99995, 10.0, 0.0312);
        let x0 = 1023.5 + 0.00005 * 3600.0 * 10f64.to_radians().cos() / 0.0312;
        let minus = wrap.forward(x0 - 0.5, 1023.5, false);
        let plus = wrap.forward(x0 + 0.5, 1023.5, false);
        assert!(minus[0] > 359.9999 && plus[0] < 0.0001, "the h = 0.5 px step crosses 0/360: {minus:?} {plus:?}");
        let cd = wrap.local_jacobian_deg_per_px(x0, 1023.5, 0.5);
        assert!(cd[0][0].is_finite() && cd[0][0] > 0.0, "dRA cos(dec)/dx across the wrap: {cd:?}");
        assert_close(cd[0][0] * 3600.0, 0.0312, 1e-8, "dRA cos(dec)/dx equals the v2 scale (x maps to east)");
        assert_close(cd[1][1] * 3600.0, 0.0312, 1e-8, "dDec/dy equals the v3 scale");
        assert!(cd[0][1].abs() * 3600.0 < 1e-7 && cd[1][0].abs() * 3600.0 < 1e-7, "{cd:?}");
        assert_close(plate_scale_arcsec(&cd), 0.0312, 1e-8, "scale with cos(10 deg) = 0.985 applied");
    }

    fn poly_yaml(c00: f64, c01: f64, c10: f64, c11: f64) -> String {
        format!(
            "!transform/polynomial-1.3.0 {{coefficients: !core/ndarray-1.1.0 {{data: [[{c00}, {c01}], [{c10}, {c11}]], datatype: float64, shape: [2, 2]}}, domain: [[-1, 1], [-1, 1]], window: [[-1, 1], [-1, 1]], inputs: [x, y], outputs: [z]}}"
        )
    }

    fn roman_l2_core(indent: &str) -> String {
        let p0 = poly_yaml(0.0, 2.0e-6, 0.11, 1.0e-7);
        let p1 = poly_yaml(0.0, 0.108, 3.0e-4, -2.0e-7);
        [
            "!transform/compose-1.4.0",
            "  bounding_box: !transform/property/bounding_box-1.2.0 {ignore: [], intervals: {x0: [-0.5, 4087.5], x1: [-0.5, 4087.5]}, order: F}",
            "  forward:",
            "  - !transform/concatenate-1.4.0",
            "    forward:",
            "    - &id001 !transform/shift-1.4.0 {inputs: [x], offset: -2043.5, outputs: [y]}",
            "    - *id001",
            "    inputs: [x0, x1]",
            "    outputs: [y0, y1]",
            "  - !transform/compose-1.4.0",
            "    forward:",
            "    - !transform/remap_axes-1.5.0 {inputs: [x0, x1], mapping: [0, 1, 0, 1], outputs: [x0, x1, x2, x3]}",
            "    - !transform/concatenate-1.4.0",
            &format!("      forward: [{p0}, {p1}]"),
            "      inputs: [x0, y0, x1, y1]",
            "      outputs: [z0, z1]",
            "    inputs: [x0, x1]",
            "    outputs: [z0, z1]",
            "  inputs: [x0, x1]",
            "  outputs: [z0, z1]",
        ]
        .iter()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn roman_shaped_wcs(with_outer_shift: bool) -> String {
        let step0 = if with_outer_shift {
            format!(
                "    transform: !transform/compose-1.4.0\n      bounding_box: !transform/property/bounding_box-1.2.0 {{ignore: [], intervals: {{x0: [-0.5, 4095.5], x1: [-0.5, 4095.5]}}, order: F}}\n      forward:\n      - !transform/concatenate-1.4.0\n        forward:\n        - !transform/shift-1.4.0 {{inputs: [x], offset: -4.0, outputs: [y]}}\n        - !transform/shift-1.4.0 {{inputs: [x], offset: -4.0, outputs: [y]}}\n        inputs: [x0, x1]\n        outputs: [y0, y1]\n      - {}\n      inputs: [x0, x1]\n      outputs: [y0, y1]\n",
                roman_l2_core("        ").trim_start()
            )
        } else {
            format!("    transform: {}\n", roman_l2_core("      ").trim_start())
        };
        format!(
            "wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>\n  name: ''\n  steps:\n  - !<tag:stsci.edu:gwcs/step-1.3.0>\n    frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0> {{axes_names: [x, y], axes_order: [0, 1], axis_physical_types: ['custom:x', 'custom:y'], name: detector, unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]}}\n{step0}  - !<tag:stsci.edu:gwcs/step-1.3.0>\n    frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0> {{axes_names: [v2, v3], axes_order: [0, 1], axis_physical_types: ['custom:v2', 'custom:v3'], name: v2v3, unit: [!unit/unit-1.0.0 arcsec, !unit/unit-1.0.0 arcsec]}}\n    transform: !transform/identity-1.4.0 {{inputs: [x0, x1], n_dims: 2, outputs: [x0, x1]}}\n  - !<tag:stsci.edu:gwcs/step-1.3.0>\n    frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0> {{axes_names: [v2, v3], axes_order: [0, 1], axis_physical_types: ['custom:v2', 'custom:v3'], name: v2v3vacorr, unit: [!unit/unit-1.0.0 arcsec, !unit/unit-1.0.0 arcsec]}}\n    transform: !transform/compose-1.4.0\n      forward:\n      - !transform/compose-1.4.0\n        forward:\n        - !transform/compose-1.4.0\n          forward:\n          - !transform/concatenate-1.4.0\n            forward:\n            - !transform/scale-1.4.0 {{factor: 0.0002777777777777778, inputs: [x], outputs: [y]}}\n            - !transform/scale-1.4.0 {{factor: 0.0002777777777777778, inputs: [x], outputs: [y]}}\n            inputs: [x0, x1]\n            outputs: [y0, y1]\n          - !<tag:stsci.edu:gwcs/spherical_cartesian-1.3.0> {{inputs: [lon, lat], outputs: [x, y, z], transform_type: spherical_to_cartesian, wrap_lon_at: 180}}\n          inputs: [x0, x1]\n          outputs: [x, y, z]\n        - !transform/rotate_sequence_3d-1.2.0 {{angles: [0.36, -0.29, 60.04, 30.664375673955533, -72.42091897428809], axes_order: zyxyz, inputs: [x, y, z], outputs: [x, y, z], rotation_type: cartesian}}\n        inputs: [x0, x1]\n        outputs: [x, y, z]\n      - !<tag:stsci.edu:gwcs/spherical_cartesian-1.3.0> {{inputs: [x, y, z], outputs: [lon, lat], transform_type: cartesian_to_spherical, wrap_lon_at: 360}}\n      inputs: [x0, x1]\n      name: v23tosky\n      outputs: [lon, lat]\n  - !<tag:stsci.edu:gwcs/step-1.3.0>\n    frame: !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>\n      axes_names: [lon, lat]\n      axes_order: [0, 1]\n      axis_physical_types: [pos.eq.ra, pos.eq.dec]\n      name: world\n      reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0> {{frame_attributes: {{}}}}\n      unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]\n    transform: null\n"
        )
    }

    fn synthetic_pipeline(body: &str) -> GwcsPipeline {
        let tree = parse_real_syntax(body);
        let (key, node) = find_wcs_node(&tree).expect("wcs node");
        assert_eq!(key.0, "wcs");
        parse_gwcs(node, &no_arrays, &key.0).unwrap_or_else(|e| panic!("{e:#}\n{body}"))
    }

    #[test]
    fn roman_l1_structure_ignores_the_inner_bbox_and_shifts_by_four() {
        let l1 = synthetic_pipeline(&roman_shaped_wcs(true));
        let l2 = synthetic_pipeline(&roman_shaped_wcs(false));
        assert_eq!(l1.bounding_box, Some(BoundingBox::square(-0.5, 4095.5)));
        assert_eq!(l2.bounding_box, Some(BoundingBox::square(-0.5, 4087.5)));
        assert_eq!(l1.frames_summary(), "detector->v2v3->v2v3vacorr->world");
        let reference_pixel = l1.forward(2.0, 2.0, true);
        assert!(reference_pixel[0].is_finite() && reference_pixel[1].is_finite(), "{reference_pixel:?}");
        assert!(l2.forward(2.0 - 4.0, 2.0 - 4.0, true)[0].is_nan());
        let mut seed = 12345u64;
        for _ in 0..20 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let x = (seed >> 33) as f64 / (1u64 << 31) as f64 * 4095.0;
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let y = (seed >> 33) as f64 / (1u64 << 31) as f64 * 4095.0;
            let a = l1.forward(x, y, false);
            let b = l2.forward(x - 4.0, y - 4.0, false);
            assert_close_slice(&a, &b, 1e-12, &format!("l1({x}, {y}) == l2(x-4, y-4)"));
        }
        let frames = l1.forward_frames(100.0, 200.0);
        let v2v3 = frames.iter().find(|(n, _)| n == "v2v3").unwrap().1;
        let vacorr = frames.iter().find(|(n, _)| n == "v2v3vacorr").unwrap().1;
        assert_eq!(v2v3.as_slice(), vacorr.as_slice(), "Identity step keeps v2v3");
        let (u, v) = (100.0 - 4.0 - 2043.5, 200.0 - 4.0 - 2043.5);
        assert_close_slice(v2v3.as_slice(), &[0.11 * u + 2.0e-6 * v, 3.0e-4 * u + 0.108 * v], 1e-9, "degree-1 polynomial pair; the stored c11 entry (i+j > degree) is ignored");
        let exact = l1.backward_exact(reference_pixel[0], reference_pixel[1], &NewtonOptions::default());
        assert!(exact.converged, "{exact:?}");
        assert_close_slice(&exact.pixel, &[2.0, 2.0], 1e-6, "round trip at the reference pixel");
    }

    fn frame(name: &str, kind: FrameKind, unit: &str) -> GwcsFrame {
        GwcsFrame {
            name: name.into(),
            kind,
            unit: vec![unit.into(), unit.into()],
            axes_names: vec!["a".into(), "b".into()],
            axis_physical_types: vec!["pos.eq.ra".into(), "pos.eq.dec".into()],
        }
    }

    fn pair_node(a: Kind, b: Kind, tag: &str) -> Node {
        Node::new(Kind::Concatenate(Box::new(Node::new(a.clone(), tag, "s")), Box::new(Node::new(b, tag, "s"))), "concatenate", "s")
    }

    fn compose(a: Node, b: Node) -> Node {
        Node::new(Kind::Compose(Box::new(a), Box::new(b)), "compose", "s")
    }

    fn synthetic_v23tosky(ra_ref: f64, dec_ref: f64, scale_arcsec: f64) -> GwcsPipeline {
        let det_to_v2v3 = compose(
            pair_node(Kind::Shift(-1023.5), Kind::Shift(-1023.5), "shift"),
            pair_node(Kind::Scale(scale_arcsec), Kind::Scale(scale_arcsec), "scale"),
        );
        let to_sphere = compose(
            pair_node(Kind::Scale(1.0 / 3600.0), Kind::Scale(1.0 / 3600.0), "scale"),
            Node::new(Kind::SphericalToCartesian { wrap_lon_at: 180 }, "spherical_cartesian", "s"),
        );
        let rotation = Node::new(
            Kind::rotation_sequence(vec![0.0, 0.0, 0.0, dec_ref, -ra_ref], vec![Axis::Z, Axis::Y, Axis::X, Axis::Y, Axis::Z], false),
            "rotate_sequence_3d",
            "s",
        );
        let v23tosky = compose(compose(to_sphere, rotation), Node::new(Kind::CartesianToSpherical { wrap_lon_at: 360 }, "spherical_cartesian", "s"));
        let steps = vec![
            GwcsStep { frame: frame("detector", FrameKind::Frame2D, "pixel"), transform: Some(det_to_v2v3) },
            GwcsStep { frame: frame("v2v3", FrameKind::Frame2D, "arcsec"), transform: Some(Node::new(Kind::Identity(2), "identity", "s")) },
            GwcsStep { frame: frame("v2v3vacorr", FrameKind::Frame2D, "arcsec"), transform: Some(v23tosky) },
            GwcsStep { frame: frame("world", FrameKind::Celestial { reference: "icrs".into() }, "deg"), transform: None },
        ];
        GwcsPipeline::new(steps, Some(BoundingBox::square(-0.5, 2047.5)), "synthetic".into(), "wcs".into()).unwrap()
    }

    fn round_trip(pipeline: &GwcsPipeline, p: [f64; 2], what: &str) -> InverseResult {
        let w = pipeline.forward(p[0], p[1], false);
        let r = pipeline.backward_exact(w[0], w[1], &NewtonOptions::default());
        assert!(r.converged, "{what}: {r:?} for pixel {p:?} world {w:?}");
        assert_close_slice(&r.pixel, &p, 1e-6, what);
        r
    }

    #[test]
    fn newton_handles_the_pole_and_ra_wrap() {
        let wrap = synthetic_v23tosky(359.99995, 10.0, 0.0312);
        let pole = synthetic_v23tosky(359.99995, 90.0 - 5e-6, 0.0312);
        let centre = wrap.forward(1023.5, 1023.5, false);
        assert_close_slice(&centre, &[359.99995, 10.0], 1e-9, "wrap chain centre");
        for chain in [&wrap, &pole] {
            for i in 0..5 {
                for j in 0..5 {
                    let p = [i as f64 * 2047.0 / 4.0, j as f64 * 2047.0 / 4.0];
                    round_trip(chain, p, "grid point");
                }
            }
        }
        let pole_centre = pole.forward(1023.5, 1023.5, false);
        assert!((pole_centre[1] - 90.0).abs() < 1e-5, "{pole_centre:?}");
        round_trip(&pole, [1023.5, 1023.5], "centre pixel 0.018 arcsec from the pole");
        let near_pole = pole.backward_analytic(123.0, 89.999999).unwrap();
        let w = pole.forward(near_pole[0], near_pole[1], false);
        assert!((w[1] - 89.999999).abs() < 1e-9, "{w:?}");
        round_trip(&pole, near_pole, "pixel whose forward is (lon, 89.999999)");
        let across = wrap.backward_analytic(0.00002, 10.0).unwrap();
        assert!(wrap.in_bbox(across[0], across[1]), "{across:?}");
        let r = wrap.backward_exact(0.00002, 10.0, &NewtonOptions::default());
        assert!(r.converged && r.iterations <= 3, "{r:?}");
        assert_close_slice(&r.pixel, &across, 1e-6, "RA 0.00002 against ra_ref 359.99995");
        let w = wrap.forward(r.pixel[0], r.pixel[1], false);
        assert_close(w[0], 0.00002, 1e-9, "forward of the solution");
    }

    #[test]
    fn newton_returns_nan_instead_of_a_folded_start() {
        let pipeline = fixture_pipeline("wcs_jwst_fgs_cal300.asdf");
        let (lon, lat) = (79.43014558211632, -69.59900426111466);
        let start = pipeline.backward_analytic(lon, lat).unwrap();
        assert!(start[0].is_finite() && start[1].is_finite(), "{start:?}");
        assert!(pipeline.in_bbox(start[0], start[1]), "the folded analytic start lies inside the box: {start:?}");
        assert!((start[0] - 359.04).abs() < 1.0 && (start[1] - 88.32).abs() < 1.0, "{start:?}");
        let r = pipeline.backward_exact(lon, lat, &NewtonOptions::default());
        assert!(!r.converged, "{r:?}");
        let far_outside = r.pixel.iter().any(|v| !v.is_finite())
            || r.pixel[0] < -8.5
            || r.pixel[0] > 2055.5
            || r.pixel[1] < -8.5
            || r.pixel[1] > 2055.5;
        assert!(far_outside, "a finite pixel inside the box is the failure: {r:?}");
    }

    #[test]
    fn backward_frames_walk_the_steps_in_reverse() {
        let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let frames = pipeline.backward_frames(274.72530887834534, -13.875908119051436).unwrap();
        let names: Vec<&str> = frames.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["v2v3vacorr", "v2v3", "detector"]);
        assert_close_slice(frames[0].1.as_slice(), &[153.14120464613708, -559.5022831896379], ARCSEC_TOL, "v2v3vacorr");
        assert_close_slice(frames[2].1.as_slice(), &[-0.5000749041269721, -0.4995578095400175], ANALYTIC_TOL, "detector");
    }

    #[test]
    #[ignore]
    fn gwcs_bench_forward_and_inverse_cost() {
        use rand::{Rng, SeedableRng};
        use std::time::Instant;
        let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let points: Vec<[f64; 2]> = (0..100_000).map(|_| [rng.gen_range(-0.5..2047.5), rng.gen_range(-0.5..2047.5)]).collect();
        let t = Instant::now();
        let mut acc = 0.0;
        let worlds: Vec<[f64; 2]> = points.iter().map(|p| pipeline.forward(p[0], p[1], false)).collect();
        let forward_ns = t.elapsed().as_nanos() as f64 / points.len() as f64;
        let t = Instant::now();
        for w in &worlds {
            acc += pipeline.backward_analytic(w[0], w[1]).unwrap()[0];
        }
        let analytic_ns = t.elapsed().as_nanos() as f64 / points.len() as f64;
        let t = Instant::now();
        let mut max_err: f64 = 0.0;
        let mut iterations = 0u64;
        for (p, w) in points.iter().zip(&worlds) {
            let r = pipeline.backward_exact(w[0], w[1], &NewtonOptions::default());
            iterations += u64::from(r.iterations);
            max_err = max_err.max((r.pixel[0] - p[0]).abs()).max((r.pixel[1] - p[1]).abs());
        }
        let exact_ns = t.elapsed().as_nanos() as f64 / points.len() as f64;
        println!(
            "gwcs_bench forward_ns={forward_ns:.1} analytic_ns={analytic_ns:.1} exact_ns={exact_ns:.1} mean_iterations={:.2} max_err_px={max_err:.2e} checksum={acc:.3}",
            iterations as f64 / points.len() as f64
        );
        assert!(max_err < 1e-6, "max_err {max_err}");
        assert!(forward_ns <= 1000.0, "forward {forward_ns} ns/pt over budget");
        assert!(analytic_ns <= 1000.0, "analytic inverse {analytic_ns} ns/pt over budget");
        assert!(exact_ns <= 6000.0, "exact inverse {exact_ns} ns/pt over budget");
    }

    #[derive(serde::Deserialize)]
    struct InventorySource {
        file: String,
        wcs_key_path: Vec<String>,
    }

    #[derive(serde::Deserialize)]
    struct Inventory {
        sources: std::collections::BTreeMap<String, InventorySource>,
    }

    fn open_real_source(path: &std::path::Path) -> Result<AsdfFile, String> {
        if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("fits")) {
            asdf_from_fits(path)
        } else {
            AsdfFile::open(path).or_else(|e| asdf_tree_only(path).map_err(|e2| format!("{e}; yaml-only fallback: {e2}")))
        }
    }

    #[test]
    #[ignore]
    fn gwcs_real_all_23_truth_sources_match() {
        let truth_dir = truth_dir();
        if skip_if_absent(&truth_dir) {
            return;
        }
        let inventory_path = truth_dir.parent().unwrap().join("inventory.json");
        if skip_if_absent(&inventory_path) {
            return;
        }
        let inventory_text = std::fs::read_to_string(&inventory_path)
            .unwrap()
            .replace(": NaN", ": null")
            .replace(" NaN,", " null,")
            .replace("[NaN", "[null")
            .replace(" NaN]", " null]")
            .replace("-Infinity", "null")
            .replace("Infinity", "null");
        let inventory: Inventory = serde_json::from_str(&inventory_text).unwrap_or_else(|e| panic!("{}: {e}", inventory_path.display()));
        assert_eq!(inventory.sources.len(), 24, "23 evaluable sources plus slit_wcs");
        let refused = ["gwcs_miri_lrs_wcs", "gwcs_examples_slit_wcs"];
        let mut evaluated = 0;
        let mut refusals = 0;
        let mut skipped = 0;
        println!("{:<40} {:>12} {:>12} {:>12} {:>12} {:>6}", "source", "world_deg", "frames_as", "analytic_px", "exact_px", "iters");
        for (id, src) in &inventory.sources {
            let file = std::path::PathBuf::from(&src.file);
            if skip_if_absent(&file) {
                skipped += 1;
                continue;
            }
            let asdf = open_real_source(&file).unwrap_or_else(|e| panic!("{id}: {e}"));
            let key = src.wcs_key_path.join(".");
            let node = wcs_node_at(&asdf.tree, &key).unwrap_or_else(|| panic!("{id}: no node at {key}"));
            let result = parse_gwcs(node, &|n| resolve_arrays(&asdf, n), &key);
            if refused.contains(&id.as_str()) {
                let err = match result {
                    Err(e) => format!("{e:#}"),
                    Ok(_) => panic!("{id} must refuse"),
                };
                if id == "gwcs_miri_lrs_wcs" {
                    assert!(err.contains("is not supported by the evaluator"), "{id}: {err}");
                    assert!(err.contains("'tabular'") && !err.contains("'constant'"), "{id}: Const1D evaluates; the wavelength lookup table refuses: {err}");
                    assert!(matches!(super::super::parse::classify_wcs_node(node), WcsNodeClass::NotImaging2D(_)), "{id} classifies as NotImaging2D");
                }
                println!("{id:<40} refused: {err}");
                refusals += 1;
                continue;
            }
            let pipeline = result.unwrap_or_else(|e| panic!("{id}: {e:#}"));
            let truth_file = truth_dir.join(format!("{id}.json"));
            if skip_if_absent(&truth_file) {
                skipped += 1;
                continue;
            }
            let truth = truth_from(&truth_file);
            assert_eq!(truth.points.len(), 122, "{id}");
            println!("{}", compare_with_truth(id, &pipeline, &truth).row(id));
            evaluated += 1;
        }
        println!("evaluated {evaluated}, refused {refusals}, skipped {skipped}");
        assert_eq!(evaluated + refusals + skipped, 24);
        assert!(skipped > 0 || (evaluated == 22 && refusals == 2), "evaluated {evaluated}, refused {refusals}");
    }

    #[derive(Default)]
    struct TruthErrors {
        world_deg: f64,
        frames_arcsec: f64,
        analytic_px: f64,
        exact_px: f64,
        iterations: u8,
    }

    impl TruthErrors {
        fn row(&self, id: &str) -> String {
            format!(
                "{id:<40} {:>12.2e} {:>12.2e} {:>12.2e} {:>12.2e} {:>6}",
                self.world_deg, self.frames_arcsec, self.analytic_px, self.exact_px, self.iterations
            )
        }
    }

    fn compare_with_truth(id: &str, pipeline: &GwcsPipeline, truth: &TruthFile) -> TruthErrors {
        let mut errors = TruthErrors::default();
        for p in &truth.points {
            let [x, y] = p.pixel;
            let what = format!("{id} {}", p.label);
            let got = pipeline.forward(x, y, true);
            assert_nan_pattern(got, pair(&p.world), WORLD_TOL, &format!("{what} world"));
            for (i, e) in p.world.iter().enumerate() {
                if let Some(e) = e {
                    errors.world_deg = errors.world_deg.max((got[i] - e).abs());
                }
            }
            for (frame, vals) in pipeline.forward_frames(x, y) {
                let exp: Vec<f64> = p.frames_forward[&frame].iter().map(|v| v.unwrap_or(f64::NAN)).collect();
                let tol = if frame == *truth.frames.last().unwrap() { WORLD_TOL } else { ARCSEC_TOL };
                assert_close_slice(vals.as_slice(), &exp, tol, &format!("{what} frame {frame}"));
                if tol == ARCSEC_TOL {
                    for (g, e) in vals.as_slice().iter().zip(&exp) {
                        errors.frames_arcsec = errors.frames_arcsec.max((g - e).abs());
                    }
                }
            }
            let [lon, lat] = unwrap_pair(&p.world_no_bbox);
            let analytic = pipeline.backward_analytic(lon, lat).unwrap_or_else(|e| panic!("{what}: {e:#}"));
            let exp = pair(p.invert_no_bbox.as_ref().unwrap());
            assert_nan_pattern(analytic, exp, ANALYTIC_TOL, &format!("{what} invert_no_bbox"));
            for i in 0..2 {
                if let Some(e) = exp[i] {
                    errors.analytic_px = errors.analytic_px.max((analytic[i] - e).abs());
                }
            }
            let own = pipeline.forward(x, y, false);
            let gwcs = pipeline.invert_gwcs_semantics(own[0], own[1]).unwrap();
            assert_nan_pattern(gwcs, pair(p.invert.as_ref().unwrap()), ANALYTIC_TOL, &format!("{what} invert"));
            if let Some(frames) = &p.frames_backward_from_world_no_bbox {
                for (frame, vals) in pipeline.backward_frames(lon, lat).unwrap() {
                    let exp: Vec<f64> = frames[&frame].iter().map(|v| v.unwrap_or(f64::NAN)).collect();
                    let tol = if frame == truth.frames[0] { ANALYTIC_TOL } else { ARCSEC_TOL };
                    assert_close_slice(vals.as_slice(), &exp, tol, &format!("{what} backward frame {frame}"));
                }
            }
            let exact = pipeline.backward_exact(lon, lat, &NewtonOptions::default());
            assert!(exact.converged && exact.iterations <= 4, "{what}: {exact:?}");
            let exp = pair(p.exact_inverse_newton.as_ref().unwrap());
            assert_nan_pattern(exact.pixel, exp, EXACT_TOL, &format!("{what} exact_inverse_newton"));
            for i in 0..2 {
                if let Some(e) = exp[i] {
                    errors.exact_px = errors.exact_px.max((exact.pixel[i] - e).abs());
                }
            }
            errors.iterations = errors.iterations.max(exact.iterations);
        }
        errors
    }

    const CRF_DETECTORS: [&str; 10] = ["nrca1", "nrca2", "nrca3", "nrca4", "nrcb1", "nrcb2", "nrcb3", "nrcb4", "nrcalong", "nrcblong"];
    const CRF_FRAMES: &str = "detector->v2v3->v2v3vacorr->v2v3corr->world";

    #[test]
    #[ignore]
    fn gwcs_real_crf_tweakreg_chains_match_truth() {
        use crate::core::astrometry::wcs::WcsKind;
        use crate::infra::image_source::load_plane_header;
        use crate::infra::wcs_source::load_wcs;
        use crate::types::image_ref::ImageRef;
        let (mast, truth_dir) = (mast_dir(), truth_dir());
        if skip_if_absent(&mast) || skip_if_absent(&truth_dir) {
            return;
        }
        let (mut evaluated, mut skipped) = (0, 0);
        println!("{:<40} {:>12} {:>12} {:>12} {:>12} {:>6} {:>12}", "source", "world_deg", "frames_as", "analytic_px", "exact_px", "iters", "vs_sip_mas");
        for det in CRF_DETECTORS {
            let stem = format!("jw01475001001_02101_00002_{det}");
            let file = mast.join(format!("{stem}_o001_crf.fits"));
            let truth_file = truth_dir.join(format!("crf_{stem}.json"));
            if skip_if_absent(&file) || skip_if_absent(&truth_file) {
                skipped += 1;
                continue;
            }
            let asdf = asdf_from_fits(&file).unwrap_or_else(|e| panic!("{e}"));
            let (key, node) = find_wcs_node(&asdf.tree).unwrap_or_else(|| panic!("{stem}: no wcs node"));
            assert_eq!(key.0, "meta.wcs");
            let pipeline = parse_gwcs(node, &|n| resolve_arrays(&asdf, n), &key.0).unwrap_or_else(|e| panic!("{stem}: {e:#}"));
            assert_eq!(pipeline.n_steps(), 4, "{stem}");
            assert_eq!(pipeline.frames_summary(), CRF_FRAMES, "{stem}");
            let mut tags = Vec::new();
            tags_in(pipeline.steps[2].transform.as_ref().unwrap(), &mut tags);
            assert!(tags.iter().any(|t| t == "divide") && tags.iter().any(|t| t == "constant"), "{stem}: {tags:?}");
            let truth = truth_from(&truth_file);
            assert_eq!(truth.points.len(), 122, "{stem}");
            assert_eq!(truth.frames.join("->"), CRF_FRAMES, "{stem}");
            let errors = compare_with_truth(&truth.source_id, &pipeline, &truth);
            let path = file.to_str().unwrap();
            let header = load_plane_header(&ImageRef::parse(path)).unwrap_or_else(|e| panic!("{stem}: {e:#}"));
            let wcs = load_wcs(path, &header).unwrap_or_else(|e| panic!("{stem}: {e:#}"));
            assert_eq!((wcs.kind(), wcs.gwcs_refusal()), (WcsKind::Gwcs, None), "{stem}: the app loader uses the gWCS instead of the header SIP");
            let info = wcs.gwcs_info().unwrap();
            assert_eq!((info.n_steps, info.frames.join("->")), (4, CRF_FRAMES.to_string()), "{stem}");
            let origin = truth.points.iter().find(|p| p.pixel == [0.0, 0.0]).unwrap();
            let sky = wcs.pixel_to_world(0.0, 0.0);
            assert_nan_pattern([sky.ra, sky.dec], pair(&origin.world_no_bbox), WORLD_TOL, &format!("{stem} load_wcs pixel_to_world(0, 0)"));
            let vs_sip = info.vs_header_sip_max_mas.unwrap_or(f64::NAN);
            println!("{} {vs_sip:>12.2e}", errors.row(&truth.source_id));
            evaluated += 1;
        }
        println!("evaluated {evaluated}, skipped {skipped}");
        assert_eq!(evaluated + skipped, CRF_DETECTORS.len());
        assert!(skipped > 0 || evaluated == CRF_DETECTORS.len(), "evaluated {evaluated}");
    }

    fn tags_in(node: &Node, out: &mut Vec<String>) {
        out.push(node.tag.clone());
        if let Kind::Compose(a, b) | Kind::Concatenate(a, b) | Kind::Arithmetic(_, a, b) = &node.kind {
            tags_in(a, out);
            tags_in(b, out);
        }
        if let Some(inverse) = &node.stored_inverse {
            tags_in(inverse, out);
        }
    }

    #[test]
    fn nircam_crf_tweakreg_chain_evaluates_divide_and_constant() {
        let pipeline = fixture_pipeline("wcs_jwst_nircam_crf_tweakreg.asdf");
        assert_eq!(pipeline.n_steps(), 4);
        assert_eq!(pipeline.frames_summary(), CRF_FRAMES);
        assert_eq!(pipeline.bounding_box, Some(BoundingBox::square(-0.5, 2047.5)));
        let correction = pipeline.steps[2].transform.as_ref().unwrap();
        assert_eq!(correction.name.as_deref(), Some("JWST tangent-plane linear correction. v1"));
        assert!(correction.stored_inverse.is_some());
        let mut tags = Vec::new();
        tags_in(correction, &mut tags);
        assert!(tags.iter().any(|t| t == "divide") && tags.iter().any(|t| t == "constant"), "{tags:?}");
        assert_close_slice(&pipeline.forward(0.0, 0.0, true), &[9.347013884760797, 1.4835463426539917], WORLD_TOL, "(0,0)");
        let frames = pipeline.forward_frames(1023.5, 1023.5);
        let names: Vec<&str> = frames.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["v2v3", "v2v3vacorr", "v2v3corr", "world"]);
        let (vacorr, corr) = (frames[1].1, frames[2].1);
        let moved = (corr.v[0] - vacorr.v[0]).hypot(corr.v[1] - vacorr.v[1]);
        assert!(moved > 0.01 && moved < 0.2, "the tweakreg correction moves v2v3 by {moved} arcsec");
        let (lon, lat) = (9.342372139487859, 1.471796122968886);
        let back: Vec<String> = pipeline.backward_frames(lon, lat).unwrap().into_iter().map(|(n, _)| n).collect();
        assert_eq!(back, ["v2v3corr", "v2v3vacorr", "v2v3", "detector"]);
        assert_close_slice(&pipeline.backward_analytic(lon, lat).unwrap(), &[1023.500000003738, 1023.4999999989781], ANALYTIC_TOL, "analytic centre");
        let r = pipeline.backward_exact(lon, lat, &NewtonOptions::default());
        assert!(r.converged && r.iterations <= 4, "{r:?}");
        assert_close_slice(&r.pixel, &[1023.4999999996751, 1023.5000000007885], EXACT_TOL, "exact centre");
    }

    #[test]
    #[ignore]
    fn gwcs_real_roman_far_sky_point_is_nan_not_a_ghost() {
        let path = real_data_dir().join("r9999901001001001001_0001_wfi01_f129_cal.asdf");
        if skip_if_absent(&path) {
            return;
        }
        let asdf = AsdfFile::open(&path).unwrap();
        let node = wcs_node_at(&asdf.tree, "roman.meta.wcs").unwrap();
        let pipeline = parse_gwcs(node, &|n| resolve_arrays(&asdf, n), "roman.meta.wcs").unwrap();
        assert_close_slice(&pipeline.forward(0.0, 0.0, true), &[72.49379934026463, -30.725719347990214], WORLD_TOL, "Roman (0,0)");
        let start = pipeline.backward_analytic(72.83404951829527, -31.252367086918532).unwrap();
        assert!(pipeline.in_bbox(start[0], start[1]), "{start:?}");
        let r = pipeline.backward_exact(72.83404951829527, -31.252367086918532, &NewtonOptions::default());
        assert!(!r.converged, "{r:?}");
        assert!(r.pixel.iter().any(|v| !v.is_finite()) || !pipeline.in_bbox(r.pixel[0], r.pixel[1]), "{r:?}");
        let l1_path = real_data_dir().join("r9999901001001001001_0001_wfi01_f129_wcs.asdf");
        if skip_if_absent(&l1_path) {
            return;
        }
        let sidecar = AsdfFile::open(&l1_path).unwrap();
        let (key, _) = find_wcs_node(&sidecar.tree).unwrap();
        assert_eq!(key.0, "roman.wcs_l2");
        let l1_node = wcs_node_at(&sidecar.tree, "roman.wcs_l1").unwrap();
        let l1 = parse_gwcs(l1_node, &|n| resolve_arrays(&sidecar, n), "roman.wcs_l1").unwrap();
        assert_eq!(l1.bounding_box, Some(BoundingBox::square(-0.5, 4095.5)));
        assert_close_slice(&l1.forward(0.0, 0.0, true), &[72.49394172327753, -30.725838737618616], WORLD_TOL, "wcs_l1 (0,0) through lz4 blocks");
        assert_close_slice(&l1.forward(2.0, 2.0, true), &pipeline.forward(-2.0, -2.0, false), 1e-12, "wcs_l1 = cal(x-4, y-4), inner bbox ignored");
    }
}
