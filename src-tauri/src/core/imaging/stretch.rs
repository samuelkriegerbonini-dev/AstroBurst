use ndarray::{Array2, Zip};
use rayon::prelude::*;

fn finite_min_max(data: &Array2<f32>) -> (f32, f32) {
    let mut mn = f32::INFINITY;
    let mut mx = f32::NEG_INFINITY;
    match data.as_slice() {
        Some(s) => {
            let pairs: Vec<(f32, f32)> = s
                .par_chunks(1 << 16)
                .map(|chunk| {
                    let mut cmn = f32::INFINITY;
                    let mut cmx = f32::NEG_INFINITY;
                    for &v in chunk {
                        if v.is_finite() {
                            if v < cmn { cmn = v; }
                            if v > cmx { cmx = v; }
                        }
                    }
                    (cmn, cmx)
                })
                .collect();
            for (cmn, cmx) in pairs {
                if cmn < mn { mn = cmn; }
                if cmx > mx { mx = cmx; }
            }
        }
        None => {
            for &v in data.iter() {
                if v.is_finite() {
                    if v < mn { mn = v; }
                    if v > mx { mx = v; }
                }
            }
        }
    }
    if mn <= mx { (mn, mx) } else { (0.0, 1.0) }
}

pub fn arcsinh_stretch(data: &Array2<f32>, factor: f32) -> Array2<f32> {
    let (dmin, dmax) = finite_min_max(data);
    arcsinh_stretch_with_stats(data, dmin, dmax, factor, 1.0)
}

pub fn arcsinh_stretch_with_stats(
    data: &Array2<f32>,
    dmin: f32,
    dmax: f32,
    factor: f32,
    gamma: f32,
) -> Array2<f32> {
    if factor.abs() < 1e-10 {
        return data.clone();
    }

    let range = dmax - dmin;
    if range < 1e-10 {
        return Array2::zeros(data.raw_dim());
    }

    let inv_range = 1.0 / range;
    let inv_denom = 1.0 / factor.asinh();
    let apply_gamma = (gamma - 1.0).abs() > 1e-6;

    let mut result = Array2::zeros(data.raw_dim());

    Zip::from(&mut result)
        .and(data)
        .par_for_each(|out, &val| {
            if !val.is_finite() {
                *out = 0.0;
            } else {
                let norm = ((val - dmin) * inv_range).clamp(0.0, 1.0);
                let stretched = (norm * factor).asinh() * inv_denom;
                *out = if apply_gamma { stretched.powf(gamma) } else { stretched };
            }
        });

    result
}

pub fn arcsinh_stretch_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    factor: f32,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    arcsinh_stretch_rgb_with_stats(r, g, b, None, None, factor, 1.0)
}

pub fn arcsinh_stretch_rgb_with_stats(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    global_min: Option<f32>,
    global_max: Option<f32>,
    factor: f32,
    gamma: f32,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    if factor.abs() < 1e-10 {
        return (r.clone(), g.clone(), b.clone());
    }

    let (gmin, gmax) = match (global_min, global_max) {
        (Some(mn), Some(mx)) => (mn, mx),
        _ => {
            let (rmn, rmx) = finite_min_max(r);
            let (gmn, gmx) = finite_min_max(g);
            let (bmn, bmx) = finite_min_max(b);
            (rmn.min(gmn).min(bmn), rmx.max(gmx).max(bmx))
        }
    };

    let range = gmax - gmin;
    if range < 1e-10 {
        return (
            Array2::zeros(r.raw_dim()),
            Array2::zeros(g.raw_dim()),
            Array2::zeros(b.raw_dim()),
        );
    }

    let inv_range = 1.0 / range;
    let inv_denom = 1.0 / factor.asinh();
    let apply_gamma = (gamma - 1.0).abs() > 1e-6;

    let mut ro = Array2::zeros(r.raw_dim());
    let mut go = Array2::zeros(g.raw_dim());
    let mut bo = Array2::zeros(b.raw_dim());

    Zip::from(&mut ro)
        .and(&mut go)
        .and(&mut bo)
        .and(r)
        .and(g)
        .and(b)
        .par_for_each(|rout, gout, bout, &rv, &gv, &bv| {
            if !rv.is_finite() || !gv.is_finite() || !bv.is_finite() {
                *rout = 0.0;
                *gout = 0.0;
                *bout = 0.0;
                return;
            }
            let rn = ((rv - gmin) * inv_range).clamp(0.0, 1.0);
            let gn = ((gv - gmin) * inv_range).clamp(0.0, 1.0);
            let bn = ((bv - gmin) * inv_range).clamp(0.0, 1.0);
            let lum = 0.2126 * rn + 0.7152 * gn + 0.0722 * bn;
            if lum < 1e-12 {
                *rout = 0.0;
                *gout = 0.0;
                *bout = 0.0;
                return;
            }
            let mut stretched = (lum * factor).asinh() * inv_denom;
            if apply_gamma {
                stretched = stretched.powf(gamma);
            }
            let mult = stretched / lum;
            *rout = (rn * mult).clamp(0.0, 1.0);
            *gout = (gn * mult).clamp(0.0, 1.0);
            *bout = (bn * mult).clamp(0.0, 1.0);
        });

    (ro, go, bo)
}

#[derive(Debug, Clone, Copy)]
pub struct GhsParams {
    pub d: f64,
    pub b: f64,
    pub sp: f64,
    pub lp: f64,
    pub hp: f64,
}

impl Default for GhsParams {
    fn default() -> Self {
        Self {
            d: 2.0,
            b: 0.0,
            sp: 0.05,
            lp: 0.0,
            hp: 1.0,
        }
    }
}

enum GhsKernel {
    Exp,
    Log,
    Pow,
}

struct GhsCoeffs {
    lp: f64,
    sp: f64,
    hp: f64,
    kernel: GhsKernel,
    b1: f64,
    a2: f64,
    b2: f64,
    c2: f64,
    d2: f64,
    e2: f64,
    a3: f64,
    b3: f64,
    c3: f64,
    d3: f64,
    e3: f64,
    a4: f64,
    b4: f64,
}

impl GhsCoeffs {
    fn new(params: &GhsParams) -> Option<Self> {
        let d = params.d.clamp(0.0, 50.0);
        if d < 1e-9 {
            return None;
        }
        let sp = params.sp.clamp(0.0, 1.0);
        let lp = params.lp.clamp(0.0, sp);
        let hp = params.hp.clamp(sp, 1.0);
        let b = params.b.clamp(-10.0, 15.0);

        if b.abs() < 1e-9 {
            let elp = (-d * (sp - lp)).exp();
            let ehp = (-d * (hp - sp)).exp();
            let qlp = elp;
            let q0 = qlp - d * lp * elp;
            let qwp = 2.0 - ehp;
            let q1 = qwp + d * (1.0 - hp) * ehp;
            let q = 1.0 / (q1 - q0);
            Some(Self {
                lp,
                sp,
                hp,
                kernel: GhsKernel::Exp,
                b1: d * elp * q,
                a2: -q0 * q,
                b2: q,
                c2: -d * sp,
                d2: d,
                e2: 0.0,
                a3: (2.0 - q0) * q,
                b3: -q,
                c3: d * sp,
                d3: -d,
                e3: 0.0,
                a4: (qwp - q0 - d * hp * ehp) * q,
                b4: d * ehp * q,
            })
        } else if (b + 1.0).abs() < 1e-4 {
            let klp = 1.0 + d * (sp - lp);
            let khp = 1.0 + d * (hp - sp);
            let qlp = -klp.ln();
            let q0 = qlp - d * lp / klp;
            let qwp = khp.ln();
            let q1 = qwp + d * (1.0 - hp) / khp;
            let q = 1.0 / (q1 - q0);
            Some(Self {
                lp,
                sp,
                hp,
                kernel: GhsKernel::Log,
                b1: d / klp * q,
                a2: -q0 * q,
                b2: -q,
                c2: 1.0 + d * sp,
                d2: -d,
                e2: 0.0,
                a3: -q0 * q,
                b3: q,
                c3: 1.0 - d * sp,
                d3: d,
                e3: 0.0,
                a4: (qwp - q0 - d * hp / khp) * q,
                b4: q * d / khp,
            })
        } else if b < 0.0 {
            let bn = -b;
            let klp = 1.0 + d * bn * (sp - lp);
            let khp = 1.0 + d * bn * (hp - sp);
            let qlp = (1.0 - klp.powf((bn - 1.0) / bn)) / (bn - 1.0);
            let q0 = qlp - d * lp * klp.powf(-1.0 / bn);
            let qwp = (khp.powf((bn - 1.0) / bn) - 1.0) / (bn - 1.0);
            let q1 = qwp + d * (1.0 - hp) * khp.powf(-1.0 / bn);
            let q = 1.0 / (q1 - q0);
            Some(Self {
                lp,
                sp,
                hp,
                kernel: GhsKernel::Pow,
                b1: d * klp.powf(-1.0 / bn) * q,
                a2: (1.0 / (bn - 1.0) - q0) * q,
                b2: -q / (bn - 1.0),
                c2: 1.0 + d * bn * sp,
                d2: -d * bn,
                e2: (bn - 1.0) / bn,
                a3: (-1.0 / (bn - 1.0) - q0) * q,
                b3: q / (bn - 1.0),
                c3: 1.0 - d * bn * sp,
                d3: d * bn,
                e3: (bn - 1.0) / bn,
                a4: (qwp - q0 - d * hp * khp.powf(-1.0 / bn)) * q,
                b4: d * khp.powf(-1.0 / bn) * q,
            })
        } else {
            let klp = 1.0 + d * b * (sp - lp);
            let khp = 1.0 + d * b * (hp - sp);
            let qlp = klp.powf(-1.0 / b);
            let q0 = qlp - d * lp * klp.powf(-(1.0 + b) / b);
            let qwp = 2.0 - khp.powf(-1.0 / b);
            let q1 = qwp + d * (1.0 - hp) * khp.powf(-(1.0 + b) / b);
            let q = 1.0 / (q1 - q0);
            Some(Self {
                lp,
                sp,
                hp,
                kernel: GhsKernel::Pow,
                b1: d * klp.powf(-(1.0 + b) / b) * q,
                a2: -q0 * q,
                b2: q,
                c2: 1.0 + d * b * sp,
                d2: -d * b,
                e2: -1.0 / b,
                a3: (2.0 - q0) * q,
                b3: -q,
                c3: 1.0 - d * b * sp,
                d3: d * b,
                e3: -1.0 / b,
                a4: (qwp - q0 - d * hp * khp.powf(-(1.0 + b) / b)) * q,
                b4: d * khp.powf(-(1.0 + b) / b) * q,
            })
        }
    }

    #[inline]
    fn apply(&self, x: f64) -> f64 {
        let y = if x < self.lp {
            self.b1 * x
        } else if x < self.sp {
            match self.kernel {
                GhsKernel::Exp => self.a2 + self.b2 * (self.c2 + self.d2 * x).exp(),
                GhsKernel::Log => self.a2 + self.b2 * (self.c2 + self.d2 * x).max(1e-30).ln(),
                GhsKernel::Pow => self.a2 + self.b2 * (self.c2 + self.d2 * x).max(0.0).powf(self.e2),
            }
        } else if x < self.hp {
            match self.kernel {
                GhsKernel::Exp => self.a3 + self.b3 * (self.c3 + self.d3 * x).exp(),
                GhsKernel::Log => self.a3 + self.b3 * (self.c3 + self.d3 * x).max(1e-30).ln(),
                GhsKernel::Pow => self.a3 + self.b3 * (self.c3 + self.d3 * x).max(0.0).powf(self.e3),
            }
        } else {
            self.a4 + self.b4 * x
        };
        y.clamp(0.0, 1.0)
    }
}

pub fn ghs_stretch(data: &Array2<f32>, params: &GhsParams) -> Array2<f32> {
    let (dmin, dmax) = finite_min_max(data);
    ghs_stretch_with_stats(data, dmin, dmax, params)
}

pub fn ghs_stretch_with_stats(
    data: &Array2<f32>,
    dmin: f32,
    dmax: f32,
    params: &GhsParams,
) -> Array2<f32> {
    let coeffs = match GhsCoeffs::new(params) {
        Some(c) => c,
        None => return data.clone(),
    };

    let range = dmax - dmin;
    if range < 1e-10 {
        return Array2::zeros(data.raw_dim());
    }
    let inv_range = 1.0 / range;

    let mut result = Array2::zeros(data.raw_dim());
    Zip::from(&mut result)
        .and(data)
        .par_for_each(|out, &val| {
            if !val.is_finite() {
                *out = 0.0;
            } else {
                let norm = (((val - dmin) * inv_range) as f64).clamp(0.0, 1.0);
                *out = coeffs.apply(norm) as f32;
            }
        });

    result
}

pub fn ghs_stretch_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    params: &GhsParams,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let (rmn, rmx) = finite_min_max(r);
    let (gmn, gmx) = finite_min_max(g);
    let (bmn, bmx) = finite_min_max(b);
    let gmin = rmn.min(gmn).min(bmn);
    let gmax = rmx.max(gmx).max(bmx);

    let (ro, (go, bo)) = rayon::join(
        || ghs_stretch_with_stats(r, gmin, gmax, params),
        || rayon::join(
            || ghs_stretch_with_stats(g, gmin, gmax, params),
            || ghs_stretch_with_stats(b, gmin, gmax, params),
        ),
    );
    (ro, go, bo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arcsinh_stretch_boundaries() {
        let data = Array2::from_shape_vec((1, 3), vec![0.0, 0.5, 1.0]).unwrap();
        let result = arcsinh_stretch(&data, 10.0);
        assert!((result[[0, 0]]).abs() < 1e-6);
        assert!((result[[0, 2]] - 1.0).abs() < 1e-4);
        assert!(result[[0, 1]] > 0.5);
    }

    #[test]
    fn test_arcsinh_stretch_monotonic() {
        let data = Array2::from_shape_fn((1, 100), |(_, x)| (x + 1) as f32 / 100.0);
        let result = arcsinh_stretch(&data, 50.0);
        for x in 1..100 {
            assert!(result[[0, x]] >= result[[0, x - 1]]);
        }
    }

    #[test]
    fn test_arcsinh_stretch_factor_effect() {
        // Use a ranged array (a 1x1 constant has no range to normalize against).
        let data = Array2::from_shape_vec((1, 3), vec![0.0, 0.1, 1.0]).unwrap();
        let mild = arcsinh_stretch(&data, 5.0);
        let strong = arcsinh_stretch(&data, 100.0);
        assert!(strong[[0, 1]] > mild[[0, 1]]);
    }

    #[test]
    fn test_arcsinh_stretch_zero_factor() {
        let data = Array2::from_shape_vec((2, 2), vec![0.1, 0.5, 0.8, 1.0]).unwrap();
        let result = arcsinh_stretch(&data, 0.0);
        assert_eq!(result, data);
    }

    #[test]
    fn test_arcsinh_stretch_nan_safe() {
        let data = Array2::from_shape_vec((1, 3), vec![f32::NAN, -0.5, 0.5]).unwrap();
        let result = arcsinh_stretch(&data, 10.0);
        assert_eq!(result[[0, 0]], 0.0);
        assert!(result[[0, 2]] > 0.0);
    }

    #[test]
    fn test_arcsinh_stretch_negative_values_not_destroyed() {
        let data = Array2::from_shape_vec((1, 4), vec![-0.1, 0.0, 0.5, 1.0]).unwrap();
        let result = arcsinh_stretch(&data, 10.0);
        assert!((result[[0, 0]]).abs() < 1e-6);
        assert!(result[[0, 2]] > 0.0);
        assert!((result[[0, 3]] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_arcsinh_stretch_rgb_consistency() {
        let ch = Array2::from_shape_fn((10, 10), |(r, c)| (r + c) as f32 / 20.0);
        let single = arcsinh_stretch(&ch, 20.0);
        let (ro, go, bo) = arcsinh_stretch_rgb(&ch, &ch, &ch, 20.0);
        assert_eq!(ro, go);
        assert_eq!(go, bo);
        for (a, b) in single.iter().zip(ro.iter()) {
            assert!((a - b).abs() < 1e-4, "gray channel diverged from mono: {} vs {}", a, b);
        }
    }

    #[test]
    fn test_arcsinh_stretch_values_above_one() {
        let data = Array2::from_shape_vec((1, 4), vec![0.0, 0.5, 1.5, 3.0]).unwrap();
        let result = arcsinh_stretch(&data, 50.0);
        assert!((result[[0, 0]]).abs() < 1e-6);
        assert!((result[[0, 3]] - 1.0).abs() < 1e-4);
        assert!(result[[0, 1]] < result[[0, 2]]);
        assert!(result[[0, 2]] < result[[0, 3]]);
    }

    #[test]
    fn test_arcsinh_stretch_rgb_global_norm() {
        let r = Array2::from_shape_vec((1, 2), vec![0.5, 2.0]).unwrap();
        let g = Array2::from_shape_vec((1, 2), vec![0.3, 1.0]).unwrap();
        let b = Array2::from_shape_vec((1, 2), vec![0.1, 0.5]).unwrap();
        let (ro, go, bo) = arcsinh_stretch_rgb(&r, &g, &b, 20.0);
        assert!(ro[[0, 1]] > go[[0, 1]]);
        assert!(go[[0, 1]] > bo[[0, 1]]);
        for ch in [&ro, &go, &bo] {
            for &v in ch.iter() {
                assert!(v >= 0.0 && v <= 1.0);
            }
        }
    }

    #[test]
    fn arcsinh_rgb_preserves_color_ratio() {
        let r = Array2::from_shape_vec((2, 2), vec![0.0, 0.2, 0.8, 0.2]).unwrap();
        let g = Array2::from_shape_vec((2, 2), vec![0.0, 0.1, 0.0, 0.1]).unwrap();
        let b = Array2::from_shape_vec((2, 2), vec![0.0, 0.05, 0.0, 0.05]).unwrap();
        let (ro, go, bo) = arcsinh_stretch_rgb(&r, &g, &b, 20.0);
        let rv = ro[[0, 1]];
        let gv = go[[0, 1]];
        let bv = bo[[0, 1]];
        assert!((rv / gv - 2.0).abs() < 0.15, "R/G ratio not preserved: {}", rv / gv);
        assert!((gv / bv - 2.0).abs() < 0.15, "G/B ratio not preserved: {}", gv / bv);
    }

    #[test]
    fn test_arcsinh_stretch_gamma() {
        let data = Array2::from_shape_vec((1, 3), vec![0.0, 0.5, 1.0]).unwrap();
        let no_gamma = arcsinh_stretch_with_stats(&data, 0.0, 1.0, 10.0, 1.0);
        let with_gamma = arcsinh_stretch_with_stats(&data, 0.0, 1.0, 10.0, 0.5);
        assert!(with_gamma[[0, 1]] > no_gamma[[0, 1]]);
    }

    fn ghs_param_grid() -> Vec<GhsParams> {
        let mut out = Vec::new();
        for &b in &[-2.0, -1.0, -0.5, 0.0, 1.0, 6.0] {
            for &(lp, sp, hp) in &[(0.0, 0.0, 1.0), (0.0, 0.1, 1.0), (0.05, 0.2, 0.8)] {
                for &d in &[0.5, 2.0, 8.0] {
                    out.push(GhsParams { d, b, sp, lp, hp });
                }
            }
        }
        out
    }

    #[test]
    fn test_ghs_endpoints_all_branches() {
        for p in ghs_param_grid() {
            let c = GhsCoeffs::new(&p).unwrap();
            let t0 = c.apply(0.0);
            let t1 = c.apply(1.0);
            assert!(t0.abs() < 1e-9, "T(0)={} for {:?}", t0, p);
            assert!((t1 - 1.0).abs() < 1e-9, "T(1)={} for {:?}", t1, p);
        }
    }

    #[test]
    fn test_ghs_monotonic_all_branches() {
        for p in ghs_param_grid() {
            let c = GhsCoeffs::new(&p).unwrap();
            let mut prev = c.apply(0.0);
            for i in 1..=1000 {
                let x = i as f64 / 1000.0;
                let y = c.apply(x);
                assert!(
                    y >= prev - 1e-12,
                    "non-monotonic at x={} ({} < {}) for {:?}",
                    x,
                    y,
                    prev,
                    p
                );
                prev = y;
            }
        }
    }

    #[test]
    fn test_ghs_continuous_at_joints() {
        for p in ghs_param_grid() {
            let c = GhsCoeffs::new(&p).unwrap();
            let eps = 1e-7;
            for &joint in &[p.lp, p.sp, p.hp] {
                if joint <= eps || joint >= 1.0 - eps {
                    continue;
                }
                let lo = c.apply(joint - eps);
                let hi = c.apply(joint + eps);
                assert!(
                    (hi - lo).abs() < 1e-4,
                    "discontinuity {} at joint {} for {:?}",
                    (hi - lo).abs(),
                    joint,
                    p
                );
            }
        }
    }

    #[test]
    fn test_ghs_zero_d_identity() {
        let data = Array2::from_shape_vec((2, 2), vec![0.1, 0.5, 0.8, 1.0]).unwrap();
        let params = GhsParams { d: 0.0, ..GhsParams::default() };
        let result = ghs_stretch(&data, &params);
        assert_eq!(result, data);
    }

    #[test]
    fn test_ghs_stretch_brightens_shadows() {
        let data = Array2::from_shape_vec((1, 3), vec![0.0, 0.05, 1.0]).unwrap();
        let params = GhsParams { d: 8.0, b: 0.0, sp: 0.05, lp: 0.0, hp: 1.0 };
        let result = ghs_stretch(&data, &params);
        assert!(result[[0, 0]].abs() < 1e-6);
        assert!((result[[0, 2]] - 1.0).abs() < 1e-6);
        assert!(result[[0, 1]] > 0.2);
        assert!(result[[0, 1]] > 0.05 * 3.0);
    }

    #[test]
    fn test_ghs_nan_safe() {
        let data = Array2::from_shape_vec((1, 3), vec![f32::NAN, 0.2, 1.0]).unwrap();
        let params = GhsParams::default();
        let result = ghs_stretch(&data, &params);
        assert_eq!(result[[0, 0]], 0.0);
        assert!(result[[0, 1]].is_finite());
    }

    #[test]
    fn test_ghs_rgb_shared_normalization() {
        let ch = Array2::from_shape_fn((8, 8), |(r, c)| (r * 8 + c) as f32 / 63.0);
        let params = GhsParams { d: 3.0, b: 1.0, sp: 0.1, lp: 0.0, hp: 1.0 };
        let single = ghs_stretch(&ch, &params);
        let (ro, go, bo) = ghs_stretch_rgb(&ch, &ch, &ch, &params);
        assert_eq!(single, ro);
        assert_eq!(single, go);
        assert_eq!(single, bo);
    }

    #[test]
    fn test_ghs_higher_d_stretches_more() {
        let c_mild = GhsCoeffs::new(&GhsParams { d: 1.0, b: 0.0, sp: 0.0, lp: 0.0, hp: 1.0 }).unwrap();
        let c_strong = GhsCoeffs::new(&GhsParams { d: 8.0, b: 0.0, sp: 0.0, lp: 0.0, hp: 1.0 }).unwrap();
        assert!(c_strong.apply(0.05) > c_mild.apply(0.05));
    }
}
