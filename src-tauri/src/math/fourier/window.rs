use ndarray::Array2;
use serde::{Deserialize, Serialize};

use super::fft::FftFloat;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "window", rename_all = "snake_case")]
pub enum TaperWindow {
    TopHat { beta: f64 },
    CosineBell { alpha: f64 },
    SplitCosineBell { alpha: f64, beta: f64 },
    Hanning,
    Tukey { alpha: f64 },
}

impl TaperWindow {
    pub fn name(self) -> &'static str {
        match self {
            Self::TopHat { .. } => "top_hat",
            Self::CosineBell { .. } => "cosine_bell",
            Self::SplitCosineBell { .. } => "split_cosine_bell",
            Self::Hanning => "hanning",
            Self::Tukey { .. } => "tukey",
        }
    }

    pub fn alpha_beta(self) -> (f64, f64) {
        match self {
            Self::TopHat { beta } => (0.0, beta),
            Self::CosineBell { alpha } => (alpha, 0.0),
            Self::SplitCosineBell { alpha, beta } => (alpha, beta),
            Self::Hanning => (1.0, 0.0),
            Self::Tukey { alpha } => (alpha, 1.0 - alpha),
        }
    }

    pub fn values(self, shape: (usize, usize)) -> Array2<f64> {
        let (alpha, beta) = self.alpha_beta();
        split_cosine_bell(shape, alpha, beta)
    }
}

pub fn normalized_radius(shape: (usize, usize)) -> Array2<f64> {
    let (rows, cols) = shape;
    let centre = |len: usize| (len as f64 - 1.0) / 2.0;
    let (yc, xc) = (centre(rows), centre(cols));
    let scale = |half: f64| if half == 0.0 { 1.0 } else { half };
    let (sy, sx) = (scale(yc), scale(xc));
    Array2::from_shape_fn(shape, |(r, c)| ((c as f64 - xc) / sx).hypot((r as f64 - yc) / sy))
}

pub fn split_cosine_bell(shape: (usize, usize), alpha: f64, beta: f64) -> Array2<f64> {
    normalized_radius(shape).mapv(|dist| {
        if dist < beta {
            1.0
        } else if dist > beta + alpha {
            0.0
        } else if alpha > 0.0 {
            0.5 * (1.0 + (std::f64::consts::PI * (dist - beta) / alpha).cos())
        } else {
            1.0
        }
    })
}

pub fn hann_periodic<T: FftFloat>(n: usize) -> Vec<T> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![T::one()];
    }
    let two_pi = T::two() * T::pi();
    let nf = <T as FftFloat>::from_usize(n);
    (0..n)
        .map(|i| {
            let phase = two_pi * <T as FftFloat>::from_usize(i) / nf;
            T::half() * (T::one() - phase.cos_val())
        })
        .collect()
}

pub fn hann_symmetric<T: FftFloat>(n: usize) -> Vec<T> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![T::one()];
    }
    let two_pi = T::two() * T::pi();
    let denom = <T as FftFloat>::from_usize(n - 1).max_of(T::one());
    (0..n)
        .map(|i| {
            let phase = two_pi * <T as FftFloat>::from_usize(i) / denom;
            T::half() * (T::one() - phase.cos_val())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hann_periodic_length() {
        let w: Vec<f64> = hann_periodic(128);
        assert_eq!(w.len(), 128);
    }

    #[test]
    fn test_hann_periodic_first_zero() {
        let w: Vec<f64> = hann_periodic(128);
        assert!(w[0].abs() < 1e-10);
    }

    #[test]
    fn test_hann_periodic_not_zero_at_end() {
        let w: Vec<f64> = hann_periodic(128);
        assert!(w[127] > 1e-5);
    }

    #[test]
    fn test_hann_periodic_peak() {
        let w: Vec<f64> = hann_periodic(128);
        let max_val = w.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!((max_val - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_hann_symmetric_endpoints_zero() {
        let w: Vec<f64> = hann_symmetric(128);
        assert!(w[0].abs() < 1e-10);
        assert!(w[127].abs() < 1e-10);
    }

    #[test]
    fn test_hann_symmetric_peak_at_center() {
        let w: Vec<f64> = hann_symmetric(129);
        let center = 64;
        assert!((w[center] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_hann_symmetric_symmetry() {
        let w: Vec<f64> = hann_symmetric(128);
        for i in 0..64 {
            assert!(
                (w[i] - w[127 - i]).abs() < 1e-10,
                "asymmetry at i={}: {} vs {}",
                i,
                w[i],
                w[127 - i]
            );
        }
    }

    #[test]
    fn test_hann_periodic_symmetry() {
        let w: Vec<f64> = hann_periodic(128);
        for i in 1..64 {
            assert!(
                (w[i] - w[128 - i]).abs() < 1e-10,
                "asymmetry at i={}: {} vs {}",
                i,
                w[i],
                w[128 - i]
            );
        }
    }

    #[test]
    fn test_hann_periodic_f32() {
        let w: Vec<f32> = hann_periodic(64);
        assert_eq!(w.len(), 64);
        assert!(w[0].abs() < 1e-6);
    }

    #[test]
    fn test_hann_empty() {
        let w: Vec<f64> = hann_periodic(0);
        assert!(w.is_empty());
    }

    #[test]
    fn test_hann_single() {
        let w: Vec<f64> = hann_periodic(1);
        assert_eq!(w.len(), 1);
        assert!((w[0] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_periodic_vs_symmetric_differ() {
        let p: Vec<f64> = hann_periodic(128);
        let s: Vec<f64> = hann_symmetric(128);
        let mut any_diff = false;
        for (a, b) in p.iter().zip(s.iter()) {
            if (a - b).abs() > 1e-10 {
                any_diff = true;
                break;
            }
        }
        assert!(any_diff);
    }

    #[test]
    fn test_hann_values_in_range() {
        let w: Vec<f64> = hann_periodic(256);
        for v in &w {
            assert!(*v >= -1e-10);
            assert!(*v <= 1.0 + 1e-10);
        }
    }

    fn assert_row(got: &[f64], expected: &[f64], tolerance: f64, what: &str) {
        assert_eq!(got.len(), expected.len(), "{what}: length");
        for (i, (g, e)) in got.iter().zip(expected).enumerate() {
            assert!((g - e).abs() <= tolerance, "{what}[{i}]: {g} against {e}");
        }
    }

    fn numpy_windows() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack/kernels/kernel_goldens.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["windows"].clone()
    }

    fn numbers(value: &serde_json::Value) -> Vec<f64> {
        value.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
    }

    #[test]
    fn taper_windows_match_the_photutils_reference_rows() {
        use TaperWindow::*;
        let row = |window: TaperWindow, shape: (usize, usize), r: usize| window.values(shape).row(r).to_vec();
        assert_row(&row(Hanning, (5, 5), 1), &[0.0, 0.19715007, 0.5, 0.19715007, 0.0], 1e-8, "Hanning 5x5 row 1");
        assert_row(&row(Tukey { alpha: 0.5 }, (5, 5), 1), &[0.0, 0.63312767, 1.0, 0.63312767, 0.0], 1e-8, "Tukey(0.5) 5x5 row 1");
        assert_row(
            &row(CosineBell { alpha: 0.8 }, (7, 7), 2),
            &[0.0, 0.011467736745367552, 0.36162762260757253, 0.6294095225512605, 0.36162762260757253, 0.011467736745367552, 0.0],
            1e-14,
            "CosineBell(0.8) 7x7 row 2",
        );
        assert_row(
            &row(SplitCosineBell { alpha: 0.8, beta: 0.2 }, (5, 5), 2),
            &[0.0, 0.6913417161825449, 1.0, 0.6913417161825449, 0.0],
            1e-14,
            "SplitCosineBell(0.8, 0.2) 5x5 row 2",
        );
        assert_row(&row(TopHat { beta: 0.5 }, (5, 5), 2), &[0.0, 1.0, 1.0, 1.0, 0.0], 0.0, "TopHat(0.5) 5x5 row 2");
        assert_eq!(Hanning.values((51, 25))[[25, 12]], 1.0);
        let serialized = serde_json::to_value(SplitCosineBell { alpha: 0.3, beta: 0.1 }).unwrap();
        assert_eq!(serialized, serde_json::json!({ "window": "split_cosine_bell", "alpha": 0.3, "beta": 0.1 }));
        assert_eq!(serde_json::from_value::<TaperWindow>(serde_json::json!({ "window": "hanning" })).unwrap(), Hanning);
    }

    #[test]
    fn taper_window_rows_match_numpy_hanning_and_scipy_tukey() {
        use TaperWindow::*;
        let golden = numpy_windows();
        let hanning = Hanning.values((101, 101));
        assert_row(&hanning.row(50).to_vec(), &numbers(&golden["hanning_101"]), 1e-14, "Hanning 101 central row");
        assert_row(&hanning.row(50).to_vec(), &hann_symmetric::<f64>(101), 1e-14, "Hanning 101 against hann_symmetric");
        let tukey = Tukey { alpha: 0.4 }.values((51, 101));
        assert_row(&tukey.row(25).to_vec(), &numbers(&golden["tukey_101_0.4"]), 1e-14, "Tukey(0.4) 51x101 central row");
        assert_row(&tukey.column(50).to_vec(), &numbers(&golden["tukey_51_0.4"]), 1e-14, "Tukey(0.4) 51x101 central column");
        let radius = normalized_radius((51, 101));
        for (index, &dist) in radius.indexed_iter() {
            if dist > 1.0 {
                assert_eq!(tukey[index], 0.0, "{index:?} lies outside the unit radius");
            }
            if dist < 0.6 {
                assert_eq!(tukey[index], 1.0, "{index:?} lies inside 1 - alpha");
            }
        }
        assert_row(&Tukey { alpha: 0.5 }.values((1, 11)).row(0).to_vec(), &numbers(&golden["tukey_11_0.5"]), 1e-14, "Tukey(0.5) 1x11");
    }
}
