use ndarray::Array2;
use rayon::prelude::*;

#[derive(Debug, Clone)]
pub struct LevelsParams {
    pub black: f64,
    pub gamma: f64,
    pub white: f64,
}

impl Default for LevelsParams {
    fn default() -> Self {
        Self { black: 0.0, gamma: 1.0, white: 1.0 }
    }
}

impl LevelsParams {
    pub fn is_identity(&self) -> bool {
        (self.black).abs() < 1e-7
            && (self.gamma - 1.0).abs() < 1e-7
            && (self.white - 1.0).abs() < 1e-7
    }
}

#[inline(always)]
fn levels_pixel(v: f32, black: f64, inv_range: f64, inv_gamma: f64) -> f32 {
    let norm = ((v as f64 - black) * inv_range).clamp(0.0, 1.0);
    norm.powf(inv_gamma) as f32
}

pub fn apply_levels(data: &Array2<f32>, params: &LevelsParams) -> Array2<f32> {
    if params.is_identity() {
        return data.clone();
    }

    let black = params.black;
    let range = (params.white - params.black).max(1e-15);
    let inv_range = 1.0 / range;
    let inv_gamma = 1.0 / params.gamma.clamp(0.01, 10.0);

    let (rows, cols) = data.dim();
    let src = data.as_slice().expect("contiguous");
    let pixels: Vec<f32> = src
        .par_iter()
        .map(|&v| {
            if !v.is_finite() || v < 0.0 { return 0.0f32; }
            levels_pixel(v, black, inv_range, inv_gamma)
        })
        .collect();

    Array2::from_shape_vec((rows, cols), pixels).unwrap()
}

pub fn apply_levels_rgb(
    r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>,
    lr: &LevelsParams, lg: &LevelsParams, lb: &LevelsParams,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let (ro, (go, bo)) = rayon::join(
        || apply_levels(r, lr),
        || rayon::join(|| apply_levels(g, lg), || apply_levels(b, lb)),
    );
    (ro, go, bo)
}

pub struct SplineLut {
    lut: [f32; 4096],
}

impl SplineLut {
    pub fn from_points(points: &[(f64, f64)]) -> Self {
        let mut sorted: Vec<(f64, f64)> = points.to_vec();
        sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        sorted.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);

        if sorted.is_empty() || sorted[0].0 > 1e-6 {
            sorted.insert(0, (0.0, 0.0));
        }
        if sorted.last().map_or(true, |p| p.0 < 1.0 - 1e-6) {
            sorted.push((1.0, 1.0));
        }

        let n = sorted.len();
        let tangents = fritsch_carlson_tangents(&sorted);

        let mut lut = [0.0f32; 4096];
        for i in 0..4096 {
            let t = i as f64 / 4095.0;
            lut[i] = hermite_eval(&sorted, &tangents, n, t).clamp(0.0, 1.0) as f32;
        }

        Self { lut }
    }

    pub fn is_identity(points: &[(f64, f64)]) -> bool {
        if points.len() > 2 { return false; }
        if points.is_empty() { return true; }
        if points.len() == 1 {
            return (points[0].0 - points[0].1).abs() < 1e-6;
        }
        let near_start = (points[0].0).abs() < 1e-6 && (points[0].1).abs() < 1e-6;
        let near_end = (points[1].0 - 1.0).abs() < 1e-6 && (points[1].1 - 1.0).abs() < 1e-6;
        near_start && near_end
    }

    #[inline(always)]
    pub fn apply(&self, v: f32) -> f32 {
        let idx = (v.clamp(0.0, 1.0) * 4095.0) as usize;
        unsafe { *self.lut.get_unchecked(idx.min(4095)) }
    }
}

fn fritsch_carlson_tangents(pts: &[(f64, f64)]) -> Vec<f64> {
    let n = pts.len();
    if n < 2 { return vec![0.0; n]; }
    if n == 2 {
        let slope = (pts[1].1 - pts[0].1) / (pts[1].0 - pts[0].0).max(1e-15);
        return vec![slope, slope];
    }

    let mut deltas = Vec::with_capacity(n - 1);
    let mut slopes = Vec::with_capacity(n - 1);
    for i in 0..n - 1 {
        let dx = (pts[i + 1].0 - pts[i].0).max(1e-15);
        deltas.push(dx);
        slopes.push((pts[i + 1].1 - pts[i].1) / dx);
    }

    let mut m = vec![0.0; n];
    m[0] = slopes[0];
    m[n - 1] = slopes[n - 2];
    for i in 1..n - 1 {
        if slopes[i - 1].signum() != slopes[i].signum() {
            m[i] = 0.0;
        } else {
            m[i] = (slopes[i - 1] + slopes[i]) * 0.5;
        }
    }

    for i in 0..n - 1 {
        if slopes[i].abs() < 1e-15 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
            continue;
        }
        let alpha = m[i] / slopes[i];
        let beta = m[i + 1] / slopes[i];
        let tau = alpha * alpha + beta * beta;
        if tau > 9.0 {
            let s = 3.0 / tau.sqrt();
            m[i] = s * alpha * slopes[i];
            m[i + 1] = s * beta * slopes[i];
        }
    }

    m
}

fn hermite_eval(pts: &[(f64, f64)], tangents: &[f64], n: usize, x: f64) -> f64 {
    if x <= pts[0].0 { return pts[0].1; }
    if x >= pts[n - 1].0 { return pts[n - 1].1; }

    let mut seg = 0;
    for i in 1..n {
        if x < pts[i].0 {
            seg = i - 1;
            break;
        }
    }

    let dx = (pts[seg + 1].0 - pts[seg].0).max(1e-15);
    let t = (x - pts[seg].0) / dx;
    let t2 = t * t;
    let t3 = t2 * t;

    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;

    h00 * pts[seg].1
        + h10 * dx * tangents[seg]
        + h01 * pts[seg + 1].1
        + h11 * dx * tangents[seg + 1]
}

pub fn apply_curve(data: &Array2<f32>, lut: &SplineLut) -> Array2<f32> {
    let (rows, cols) = data.dim();
    let src = data.as_slice().expect("contiguous");
    let pixels: Vec<f32> = src
        .par_iter()
        .map(|&v| {
            if !v.is_finite() || v < 0.0 { return 0.0f32; }
            lut.apply(v)
        })
        .collect();
    Array2::from_shape_vec((rows, cols), pixels).unwrap()
}

pub fn apply_curve_rgb(
    r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>,
    lr: &SplineLut, lg: &SplineLut, lb: &SplineLut,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let (ro, (go, bo)) = rayon::join(
        || apply_curve(r, lr),
        || rayon::join(|| apply_curve(g, lg), || apply_curve(b, lb)),
    );
    (ro, go, bo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levels_identity() {
        let data = Array2::from_shape_fn((10, 10), |(r, c)| (r + c) as f32 / 20.0);
        let result = apply_levels(&data, &LevelsParams::default());
        for r in 0..10 {
            for c in 0..10 {
                assert!((result[[r, c]] - data[[r, c]]).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn test_levels_black_clip() {
        let data = Array2::from_shape_vec((1, 4), vec![0.0, 0.1, 0.5, 1.0]).unwrap();
        let params = LevelsParams { black: 0.2, gamma: 1.0, white: 1.0 };
        let result = apply_levels(&data, &params);
        assert_eq!(result[[0, 0]], 0.0);
        assert_eq!(result[[0, 1]], 0.0);
        assert!(result[[0, 2]] > 0.0 && result[[0, 2]] < 1.0);
        assert!((result[[0, 3]] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_levels_gamma() {
        let data = Array2::from_shape_vec((1, 1), vec![0.5]).unwrap();
        let bright = apply_levels(&data, &LevelsParams { black: 0.0, gamma: 2.0, white: 1.0 });
        let dark = apply_levels(&data, &LevelsParams { black: 0.0, gamma: 0.5, white: 1.0 });
        assert!(bright[[0, 0]] > 0.5);
        assert!(dark[[0, 0]] < 0.5);
    }

    #[test]
    fn test_spline_identity() {
        let lut = SplineLut::from_points(&[(0.0, 0.0), (1.0, 1.0)]);
        for i in 0..=100 {
            let v = i as f32 / 100.0;
            assert!((lut.apply(v) - v).abs() < 0.01, "v={} got={}", v, lut.apply(v));
        }
    }

    #[test]
    fn test_spline_s_curve() {
        let lut = SplineLut::from_points(&[
            (0.0, 0.0), (0.25, 0.15), (0.5, 0.5), (0.75, 0.85), (1.0, 1.0),
        ]);
        assert!(lut.apply(0.0) < 0.01);
        assert!((lut.apply(1.0) - 1.0).abs() < 0.01);
        assert!(lut.apply(0.25) < 0.25);
        assert!(lut.apply(0.75) > 0.75);
    }

    #[test]
    fn test_spline_monotonic() {
        let lut = SplineLut::from_points(&[
            (0.0, 0.0), (0.3, 0.1), (0.5, 0.5), (0.7, 0.9), (1.0, 1.0),
        ]);
        let mut prev = 0.0f32;
        for i in 0..=4095 {
            let v = i as f32 / 4095.0;
            let out = lut.apply(v);
            assert!(out >= prev - 1e-6, "non-monotonic at v={}: {}>{}", v, prev, out);
            prev = out;
        }
    }
}
