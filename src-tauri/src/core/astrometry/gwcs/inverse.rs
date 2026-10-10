use super::pipeline::InverseResult;

pub const GWCS_INVERSE_ACCEPT_PX: f64 = 0.5;
const JACOBIAN_SINGULAR_DET: f64 = 1e-30;
const DIVERGENCE_STEP_PX: f64 = 1e4;

#[derive(Debug, Clone, Copy)]
pub struct NewtonOptions {
    pub tol_px: f64,
    pub max_iter: u8,
    pub h_px: f64,
}

impl Default for NewtonOptions {
    fn default() -> Self {
        Self { tol_px: 1e-7, max_iter: 10, h_px: 0.01 }
    }
}

struct TangentBasis {
    target: [f64; 3],
    east: [f64; 3],
    north: [f64; 3],
}

fn unit_vector(lon_deg: f64, lat_deg: f64) -> [f64; 3] {
    let (sin_lon, cos_lon) = lon_deg.to_radians().sin_cos();
    let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
    [cos_lat * cos_lon, cos_lat * sin_lon, sin_lat]
}

fn unit_vector_of(world: [f64; 2]) -> [f64; 3] {
    unit_vector(world[0], world[1])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl TangentBasis {
    fn at(lon_deg: f64, lat_deg: f64) -> Self {
        let (sin_lon, cos_lon) = lon_deg.to_radians().sin_cos();
        let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
        Self {
            target: unit_vector(lon_deg, lat_deg),
            east: [-sin_lon, cos_lon, 0.0],
            north: [-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat],
        }
    }

    fn project(&self, world: [f64; 2]) -> [f64; 2] {
        let u = unit_vector(world[0], world[1]);
        [dot(self.east, u), dot(self.north, u)]
    }

    fn residual(&self, u: [f64; 3]) -> [f64; 2] {
        let d = [self.target[0] - u[0], self.target[1] - u[1], self.target[2] - u[2]];
        [dot(self.east, d), dot(self.north, d)]
    }

    fn same_hemisphere(&self, u: [f64; 3]) -> bool {
        dot(self.target, u) > 0.0
    }
}

pub fn newton_exact(
    forward: &dyn Fn([f64; 2]) -> [f64; 2],
    target_deg: [f64; 2],
    start: [f64; 2],
    opts: &NewtonOptions,
) -> InverseResult {
    let mut result = InverseResult { pixel: [f64::NAN, f64::NAN], iterations: 0, converged: false, residual_px: f64::NAN };
    if !(start[0].is_finite() && start[1].is_finite() && target_deg[0].is_finite() && target_deg[1].is_finite()) {
        return result;
    }
    let basis = TangentBasis::at(target_deg[0], target_deg[1]);
    let h = opts.h_px;
    let mut p = start;
    let mut last_det = f64::NAN;
    for _ in 0..opts.max_iter {
        let u = unit_vector_of(forward(p));
        if !basis.same_hemisphere(u) {
            break;
        }
        let r = basis.residual(u);
        let ux_p = basis.project(forward([p[0] + h, p[1]]));
        let ux_m = basis.project(forward([p[0] - h, p[1]]));
        let uy_p = basis.project(forward([p[0], p[1] + h]));
        let uy_m = basis.project(forward([p[0], p[1] - h]));
        let j = [
            [(ux_p[0] - ux_m[0]) / (2.0 * h), (uy_p[0] - uy_m[0]) / (2.0 * h)],
            [(ux_p[1] - ux_m[1]) / (2.0 * h), (uy_p[1] - uy_m[1]) / (2.0 * h)],
        ];
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        last_det = det;
        if !(det.abs() >= JACOBIAN_SINGULAR_DET) {
            break;
        }
        let delta = [(j[1][1] * r[0] - j[0][1] * r[1]) / det, (j[0][0] * r[1] - j[1][0] * r[0]) / det];
        let step = delta[0].hypot(delta[1]);
        if !step.is_finite() || step > DIVERGENCE_STEP_PX {
            break;
        }
        p = [p[0] + delta[0], p[1] + delta[1]];
        result.iterations += 1;
        if step < opts.tol_px {
            result.converged = true;
            break;
        }
    }
    let u = unit_vector_of(forward(p));
    let r = basis.residual(u);
    let near_side = basis.same_hemisphere(u);
    result.residual_px = if near_side && last_det.is_finite() && last_det != 0.0 {
        r[0].hypot(r[1]) / last_det.abs().sqrt()
    } else {
        f64::NAN
    };
    result.converged &= near_side;
    if result.converged || result.residual_px < GWCS_INVERSE_ACCEPT_PX {
        result.pixel = p;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    fn linear(p: [f64; 2]) -> [f64; 2] {
        [10.0 + 0.001 * p[0] - 0.0002 * p[1], 20.0 + 0.0011 * p[1] + 0.0001 * p[0]]
    }

    #[test]
    fn newton_converges_on_a_smooth_forward() {
        let target = linear([100.0, 200.0]);
        let result = newton_exact(&linear, target, [100.3, 199.6], &NewtonOptions::default());
        assert!(result.converged, "{result:?}");
        assert!(result.iterations <= 3, "{result:?}");
        assert_close(result.pixel[0], 100.0, 1e-7, "x");
        assert_close(result.pixel[1], 200.0, 1e-7, "y");
        assert!(result.residual_px < 1e-6, "{result:?}");
    }

    #[test]
    fn a_non_finite_start_returns_nan_without_iterating() {
        let result = newton_exact(&linear, [10.1, 20.2], [f64::NAN, 5.0], &NewtonOptions::default());
        assert!(!result.converged);
        assert_eq!(result.iterations, 0);
        assert!(result.pixel[0].is_nan() && result.pixel[1].is_nan());
    }

    #[test]
    fn a_singular_forward_returns_nan_not_the_start() {
        let flat = |_: [f64; 2]| [10.0, 20.0];
        let result = newton_exact(&flat, [11.0, 21.0], [3.0, 4.0], &NewtonOptions::default());
        assert!(!result.converged);
        assert!(result.pixel[0].is_nan() && result.pixel[1].is_nan(), "{result:?}");
    }

    #[test]
    fn a_slow_but_close_solution_is_kept_with_converged_false() {
        let target = linear([50.0, 60.0]);
        let opts = NewtonOptions { tol_px: 1e-7, max_iter: 1, h_px: 0.01 };
        let result = newton_exact(&linear, target, [50.2, 60.1], &opts);
        assert!(!result.converged);
        assert_eq!(result.iterations, 1);
        assert!(result.residual_px < GWCS_INVERSE_ACCEPT_PX, "{result:?}");
        assert!(result.pixel[0].is_finite(), "{result:?}");
        assert_close(result.pixel[0], 50.0, 1e-6, "x after one step of a linear map");
    }

    fn stepped(offset_deg: f64) -> impl Fn([f64; 2]) -> [f64; 2] {
        move |p| {
            let shift = if p[0] < 5.0 { offset_deg } else { 0.0 };
            [0.001 * p[0] + shift, 0.001 * p[1]]
        }
    }

    #[test]
    fn the_accept_threshold_is_half_a_pixel() {
        assert_eq!(GWCS_INVERSE_ACCEPT_PX, 0.5);
        let opts = NewtonOptions { tol_px: 1e-7, max_iter: 1, h_px: 0.01 };
        let below = newton_exact(&stepped(0.49e-3), [0.0, 0.0], [10.0, 0.0], &opts);
        assert!(!below.converged && below.iterations == 1, "{below:?}");
        assert_close(below.residual_px, 0.49, 1e-6, "residual after the single step lands on the offset side");
        assert_close_slice(&below.pixel, &[0.0, 0.0], 1e-6, "0.49 px is kept");
        let above = newton_exact(&stepped(0.51e-3), [0.0, 0.0], [10.0, 0.0], &opts);
        assert!(!above.converged && above.iterations == 1, "{above:?}");
        assert_close(above.residual_px, 0.51, 1e-6, "residual just above the threshold");
        assert!(above.pixel[0].is_nan() && above.pixel[1].is_nan(), "0.51 px is refused: {above:?}");
    }

    #[test]
    fn an_iterate_on_the_antipode_is_not_a_solution() {
        let far_side = |p: [f64; 2]| [180.0 + 0.001 * p[0], 0.001 * p[1]];
        let target = far_side([0.0, 0.0]);
        let antipode_start = [-180_000.0, 0.0];
        let u = far_side(antipode_start);
        assert_close_slice(&u, &[0.0, 0.0], 1e-12, "the start's forward is the antipode of the target");
        let result = newton_exact(&far_side, target, antipode_start, &NewtonOptions::default());
        assert!(!result.converged, "{result:?}");
        assert!(result.pixel[0].is_nan() && result.pixel[1].is_nan(), "{result:?}");
        assert!(!(result.residual_px < GWCS_INVERSE_ACCEPT_PX), "{result:?}");
        let good = newton_exact(&far_side, target, [3.0, -2.0], &NewtonOptions::default());
        assert!(good.converged, "{good:?}");
        assert_close_slice(&good.pixel, &[0.0, 0.0], 1e-7, "the near-side start still converges");
    }
}
