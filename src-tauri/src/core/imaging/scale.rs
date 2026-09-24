use ndarray::Array2;
use rayon::prelude::*;

use crate::core::imaging::stats::{is_padding, is_valid_pixel, percentile};
use crate::core::imaging::zscale::{zscale_limits, DEFAULT_CONTRAST};
use crate::types::constants::{
    SCALE_MODE_MANUAL_ALIAS, SCALE_MODE_MINMAX, SCALE_MODE_PERCENTILE, SCALE_MODE_USER,
    SCALE_MODE_ZSCALE,
};

const LOG_A: f64 = 1000.0;

pub const DEFAULT_ASINH_A: f64 = 0.1;

pub const DEFAULT_POWER: f64 = 2.0;

pub const DEFAULT_PERCENTILES: [f64; 2] = [1.0, 99.5];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchKind {
    Linear,
    Log,
    Sqrt,
    Asinh,
    Power,
}

impl StretchKind {
    pub const ALL: [StretchKind; 5] = [
        StretchKind::Linear,
        StretchKind::Log,
        StretchKind::Sqrt,
        StretchKind::Asinh,
        StretchKind::Power,
    ];

    pub fn from_name(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "linear" => Ok(StretchKind::Linear),
            "log" => Ok(StretchKind::Log),
            "sqrt" => Ok(StretchKind::Sqrt),
            "asinh" => Ok(StretchKind::Asinh),
            "power" => Ok(StretchKind::Power),
            "histeq" => Err(
                "stretch 'histeq' is not supported: histogram equalization is a global \
                 transform, not a pointwise curve; use one of: linear, log, sqrt, asinh, power"
                    .into(),
            ),
            other => Err(format!(
                "unknown stretch '{other}' (supported: linear, log, sqrt, asinh, power)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            StretchKind::Linear => "linear",
            StretchKind::Log => "log",
            StretchKind::Sqrt => "sqrt",
            StretchKind::Asinh => "asinh",
            StretchKind::Power => "power",
        }
    }
}

pub fn apply_stretch(normalized: f32, kind: StretchKind, asinh_a: f64, power: f64) -> f32 {
    let x = (normalized as f64).clamp(0.0, 1.0);

    let y = match kind {
        StretchKind::Linear => x,
        StretchKind::Log => (LOG_A * x + 1.0).ln() / (LOG_A + 1.0).ln(),
        StretchKind::Sqrt => x.sqrt(),
        StretchKind::Asinh => {
            let a = if asinh_a > 0.0 { asinh_a } else { DEFAULT_ASINH_A };
            (x / a).asinh() / (1.0 / a).asinh()
        }
        StretchKind::Power => {
            let p = if power > 0.0 { power } else { DEFAULT_POWER };
            x.powf(p)
        }
    };

    (y as f32).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LimitMode {
    MinMax,
    ZScale { contrast: f64 },
    Percentile { low: f64, high: f64 },
    User { vmin: Option<f64>, vmax: Option<f64> },
}

impl LimitMode {
    pub fn from_parts(
        algorithm: &str,
        vmin: Option<f64>,
        vmax: Option<f64>,
        percentile: Option<[f64; 2]>,
        zscale_contrast: Option<f64>,
    ) -> Result<Self, String> {
        let name = algorithm.trim().to_ascii_lowercase();
        if name == SCALE_MODE_MINMAX {
            Ok(LimitMode::MinMax)
        } else if name == SCALE_MODE_ZSCALE {
            Ok(LimitMode::ZScale {
                contrast: zscale_contrast.unwrap_or(DEFAULT_CONTRAST),
            })
        } else if name == SCALE_MODE_PERCENTILE {
            let [low, high] = percentile.unwrap_or(DEFAULT_PERCENTILES);
            if !low.is_finite() || !high.is_finite() {
                return Err(format!("percentiles must be finite (got {low}, {high})"));
            }
            if !(0.0..=100.0).contains(&low) || !(0.0..=100.0).contains(&high) {
                return Err("percentiles must lie in 0..=100".into());
            }
            if low >= high {
                return Err(format!("percentile low must be < high (got {low}, {high})"));
            }
            Ok(LimitMode::Percentile { low, high })
        } else if name == SCALE_MODE_USER || name == SCALE_MODE_MANUAL_ALIAS {
            if vmin.is_some_and(|v| !v.is_finite()) || vmax.is_some_and(|v| !v.is_finite()) {
                return Err("vmin and vmax must be finite".into());
            }
            if let (Some(lo), Some(hi)) = (vmin, vmax) {
                if lo >= hi {
                    return Err(format!("vmin must be < vmax (got {lo}, {hi})"));
                }
            }
            Ok(LimitMode::User { vmin, vmax })
        } else {
            Err(format!(
                "unknown scale algorithm '{name}' (supported: minmax, zscale, percentile, user)"
            ))
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            LimitMode::MinMax => SCALE_MODE_MINMAX,
            LimitMode::ZScale { .. } => SCALE_MODE_ZSCALE,
            LimitMode::Percentile { .. } => SCALE_MODE_PERCENTILE,
            LimitMode::User { .. } => SCALE_MODE_USER,
        }
    }
}

pub fn valid_min_max(data: &Array2<f32>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &v in data.iter() {
        if is_valid_pixel(v) {
            let v = v as f64;
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
        }
    }
    if min.is_finite() && max.is_finite() {
        (min, max)
    } else {
        (0.0, 1.0)
    }
}

pub fn resolve_limits(data: &Array2<f32>, mode: LimitMode) -> (f64, f64) {
    match mode {
        LimitMode::ZScale { contrast } => {
            let (vmin, vmax) = zscale_limits(data, contrast);
            if vmin.is_finite() && vmax.is_finite() {
                (vmin, vmax)
            } else {
                (0.0, 1.0)
            }
        }
        LimitMode::MinMax => valid_min_max(data),
        LimitMode::User { vmin, vmax } => {
            let (dmin, dmax) = valid_min_max(data);
            (vmin.unwrap_or(dmin), vmax.unwrap_or(dmax))
        }
        LimitMode::Percentile { low, high } => {
            let mut valid: Vec<f32> = data.iter().copied().filter(|&v| is_valid_pixel(v)).collect();
            if valid.is_empty() {
                return (0.0, 1.0);
            }
            let vmin = percentile(&mut valid, (low / 100.0).clamp(0.0, 1.0)) as f64;
            let vmax = percentile(&mut valid, (high / 100.0).clamp(0.0, 1.0)) as f64;
            (vmin, vmax)
        }
    }
}

pub fn normalize_and_stretch(
    data: &[f32],
    vmin: f64,
    vmax: f64,
    kind: StretchKind,
    asinh_a: f64,
    power: f64,
) -> (Vec<f32>, u64, u64, u64) {
    let range = vmax - vmin;
    let norm: Vec<f32> = data
        .par_iter()
        .map(|&v| {
            if is_padding(v) {
                return f32::NAN;
            }
            let vd = v as f64;
            let n = if range > 0.0 {
                (((vd - vmin) / range) as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            apply_stretch(n, kind, asinh_a, power)
        })
        .collect();
    let (valid, below, above) = data
        .par_iter()
        .fold(
            || (0u64, 0u64, 0u64),
            |(va, be, ab), &v| {
                if is_padding(v) {
                    return (va, be, ab);
                }
                let vd = v as f64;
                (va + 1, be + (vd <= vmin) as u64, ab + (vd >= vmax) as u64)
            },
        )
        .reduce(|| (0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
    (norm, valid, below, above)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn from_name_accepts_the_five_kinds_case_insensitively() {
        assert_eq!(StretchKind::from_name("linear").unwrap(), StretchKind::Linear);
        assert_eq!(StretchKind::from_name("LOG").unwrap(), StretchKind::Log);
        assert_eq!(StretchKind::from_name(" Sqrt ").unwrap(), StretchKind::Sqrt);
        assert_eq!(StretchKind::from_name("asinh").unwrap(), StretchKind::Asinh);
        assert_eq!(StretchKind::from_name("Power").unwrap(), StretchKind::Power);
    }

    #[test]
    fn from_name_rejects_unknown_and_histeq_without_panicking() {
        for bad in ["histeq", "gamma", "zscale"] {
            let err = StretchKind::from_name(bad).unwrap_err();
            assert!(err.contains(bad), "message should name {bad:?}: {err}");
            assert!(err.contains("linear"), "message should list supported: {err}");
        }
        let err = StretchKind::from_name("histeq").unwrap_err();
        assert!(err.contains("histogram equalization"), "{err}");
        assert!(StretchKind::from_name("").is_err());
    }

    #[test]
    fn all_names_round_trip() {
        for kind in StretchKind::ALL {
            assert_eq!(StretchKind::from_name(kind.name()).unwrap(), kind);
        }
    }

    #[test]
    fn every_kind_maps_endpoints_to_zero_and_one() {
        for kind in StretchKind::ALL {
            let lo = apply_stretch(0.0, kind, DEFAULT_ASINH_A, DEFAULT_POWER);
            let hi = apply_stretch(1.0, kind, DEFAULT_ASINH_A, DEFAULT_POWER);
            assert!(lo.abs() < EPS, "{kind:?} T(0) = {lo}");
            assert!((hi - 1.0).abs() < EPS, "{kind:?} T(1) = {hi}");
        }
    }

    #[test]
    fn linear_is_identity() {
        for &v in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let y = apply_stretch(v, StretchKind::Linear, DEFAULT_ASINH_A, DEFAULT_POWER);
            assert!((y - v).abs() < EPS, "linear({v}) = {y}");
        }
    }

    #[test]
    fn sample_values_match_astropy_reference() {
        let log = [0.0, 0.799776, 0.899816, 0.958408, 1.0];
        let sqrt = [0.0, 0.5, 0.707107, 0.866025, 1.0];
        let asinh = [0.0, 0.549402, 0.77127, 0.904691, 1.0];
        let power = [0.0, 0.0625, 0.25, 0.5625, 1.0];
        let xs = [0.0f32, 0.25, 0.5, 0.75, 1.0];

        for (i, &x) in xs.iter().enumerate() {
            let a = apply_stretch(x, StretchKind::Log, DEFAULT_ASINH_A, DEFAULT_POWER);
            assert!((a - log[i]).abs() < EPS, "log({x}) = {a}, want {}", log[i]);
            let a = apply_stretch(x, StretchKind::Sqrt, DEFAULT_ASINH_A, DEFAULT_POWER);
            assert!((a - sqrt[i]).abs() < EPS, "sqrt({x}) = {a}, want {}", sqrt[i]);
            let a = apply_stretch(x, StretchKind::Asinh, DEFAULT_ASINH_A, DEFAULT_POWER);
            assert!((a - asinh[i]).abs() < EPS, "asinh({x}) = {a}, want {}", asinh[i]);
            let a = apply_stretch(x, StretchKind::Power, DEFAULT_ASINH_A, DEFAULT_POWER);
            assert!((a - power[i]).abs() < EPS, "power({x}) = {a}, want {}", power[i]);
        }
    }

    #[test]
    fn stretch_bytes_match_shared_golden_vectors() {
        let xs = [0.0f32, 0.25, 0.5, 0.75, 1.0];
        let golden: [(StretchKind, [u8; 5]); 5] = [
            (StretchKind::Linear, [0, 64, 128, 191, 255]),
            (StretchKind::Log, [0, 204, 229, 244, 255]),
            (StretchKind::Sqrt, [0, 128, 180, 221, 255]),
            (StretchKind::Asinh, [0, 140, 197, 231, 255]),
            (StretchKind::Power, [0, 16, 64, 143, 255]),
        ];
        for (kind, want) in golden {
            for (i, &x) in xs.iter().enumerate() {
                let got = (apply_stretch(x, kind, 0.1, 2.0) * 255.0).round() as u8;
                assert_eq!(got, want[i], "{kind:?}({x})");
            }
        }
    }

    #[test]
    fn asinh_incorporates_its_softening_parameter() {
        let y = apply_stretch(0.5, StretchKind::Asinh, 0.1, DEFAULT_POWER);
        assert!((y - 0.771270).abs() < EPS, "asinh(0.5, a=0.1) = {y}");

        let small_a = apply_stretch(0.5, StretchKind::Asinh, 0.01, DEFAULT_POWER);
        let large_a = apply_stretch(0.5, StretchKind::Asinh, 1.0, DEFAULT_POWER);
        assert!(small_a > large_a, "a=0.01 -> {small_a}, a=1.0 -> {large_a}");
    }

    #[test]
    fn power_incorporates_its_exponent() {
        let sq = apply_stretch(0.5, StretchKind::Power, DEFAULT_ASINH_A, 2.0);
        assert!((sq - 0.25).abs() < EPS, "power(0.5, p=2) = {sq}");
        let cube = apply_stretch(0.5, StretchKind::Power, DEFAULT_ASINH_A, 3.0);
        assert!((cube - 0.125).abs() < EPS, "power(0.5, p=3) = {cube}");
        let root = apply_stretch(0.5, StretchKind::Power, DEFAULT_ASINH_A, 0.5);
        assert!((root - 0.5f32.sqrt()).abs() < EPS, "power(0.5, p=0.5) = {root}");
    }

    #[test]
    fn every_kind_is_monotonic_nondecreasing() {
        for kind in StretchKind::ALL {
            let mut prev = apply_stretch(0.0, kind, DEFAULT_ASINH_A, DEFAULT_POWER);
            for i in 1..=1000 {
                let x = i as f32 / 1000.0;
                let y = apply_stretch(x, kind, DEFAULT_ASINH_A, DEFAULT_POWER);
                assert!(y >= prev - EPS, "{kind:?} not monotonic at x={x}: {y} < {prev}");
                prev = y;
            }
        }
    }

    #[test]
    fn out_of_range_input_is_clamped_not_nan() {
        for kind in StretchKind::ALL {
            let below = apply_stretch(-0.5, kind, DEFAULT_ASINH_A, DEFAULT_POWER);
            let above = apply_stretch(1.7, kind, DEFAULT_ASINH_A, DEFAULT_POWER);
            assert!(below.abs() < EPS, "{kind:?} below-range -> {below}");
            assert!((above - 1.0).abs() < EPS, "{kind:?} above-range -> {above}");
        }
    }

    #[test]
    fn degenerate_parameters_fall_back_to_defaults() {
        let a0 = apply_stretch(0.5, StretchKind::Asinh, 0.0, DEFAULT_POWER);
        assert!(a0.is_finite() && (0.0..=1.0).contains(&a0));
        let p0 = apply_stretch(0.0, StretchKind::Power, DEFAULT_ASINH_A, 0.0);
        assert!(p0.is_finite() && (0.0..=1.0).contains(&p0));
    }

    #[test]
    fn limit_mode_from_parts_accepts_known_names_and_manual_alias() {
        assert_eq!(
            LimitMode::from_parts("minmax", None, None, None, None).unwrap(),
            LimitMode::MinMax
        );
        assert_eq!(
            LimitMode::from_parts(" ZScale ", None, None, None, Some(0.4)).unwrap(),
            LimitMode::ZScale { contrast: 0.4 }
        );
        assert_eq!(
            LimitMode::from_parts("zscale", None, None, None, None).unwrap(),
            LimitMode::ZScale { contrast: DEFAULT_CONTRAST }
        );
        assert_eq!(
            LimitMode::from_parts("percentile", None, None, Some([5.0, 95.0]), None).unwrap(),
            LimitMode::Percentile { low: 5.0, high: 95.0 }
        );
        assert_eq!(
            LimitMode::from_parts("percentile", None, None, None, None).unwrap(),
            LimitMode::Percentile { low: 1.0, high: 99.5 }
        );
        let user = LimitMode::from_parts("user", Some(1.0), None, None, None).unwrap();
        assert_eq!(user, LimitMode::User { vmin: Some(1.0), vmax: None });
        let manual = LimitMode::from_parts("manual", Some(1.0), None, None, None).unwrap();
        assert_eq!(manual, user);
        assert_eq!(manual.name(), "user");
        assert_eq!(LimitMode::MinMax.name(), "minmax");
        assert_eq!(LimitMode::ZScale { contrast: 0.25 }.name(), "zscale");
        assert_eq!(LimitMode::Percentile { low: 0.0, high: 1.0 }.name(), "percentile");
    }

    #[test]
    fn limit_mode_from_parts_validates_percentiles_and_user_limits() {
        let err = LimitMode::from_parts("percentile", None, None, Some([99.0, 1.0]), None).unwrap_err();
        assert_eq!(err, "percentile low must be < high (got 99, 1)");
        let err = LimitMode::from_parts("percentile", None, None, Some([50.0, 50.0]), None).unwrap_err();
        assert!(err.starts_with("percentile low must be < high"), "{err}");
        let err = LimitMode::from_parts("percentile", None, None, Some([-1.0, 50.0]), None).unwrap_err();
        assert_eq!(err, "percentiles must lie in 0..=100");
        let err = LimitMode::from_parts("percentile", None, None, Some([0.0, 101.0]), None).unwrap_err();
        assert_eq!(err, "percentiles must lie in 0..=100");
        assert!(LimitMode::from_parts("percentile", None, None, Some([f64::NAN, 50.0]), None).is_err());
        assert!(LimitMode::from_parts("percentile", None, None, Some([1.0, f64::INFINITY]), None).is_err());
        assert!(LimitMode::from_parts("percentile", None, None, Some([0.0, 100.0]), None).is_ok());

        let err = LimitMode::from_parts("user", Some(5.0), Some(5.0), None, None).unwrap_err();
        assert_eq!(err, "vmin must be < vmax (got 5, 5)");
        let err = LimitMode::from_parts("user", Some(10.0), Some(2.5), None, None).unwrap_err();
        assert_eq!(err, "vmin must be < vmax (got 10, 2.5)");
        assert!(LimitMode::from_parts("user", Some(f64::NAN), None, None, None).is_err());
        assert!(LimitMode::from_parts("user", None, Some(3.0), None, None).is_ok());
        assert!(LimitMode::from_parts("user", Some(3.0), None, None, None).is_ok());
        assert!(LimitMode::from_parts("manual", None, None, None, None).is_ok());
        assert!(LimitMode::from_parts("user", Some(1.0), Some(2.0), None, None).is_ok());
    }

    #[test]
    fn limit_mode_from_parts_rejects_unknown_names() {
        for bad in ["bogus", "", "histeq", "auto"] {
            let err = LimitMode::from_parts(bad, None, None, None, None).unwrap_err();
            assert!(err.contains("unknown scale algorithm"), "{err}");
            assert!(err.contains("minmax") && err.contains("user"), "{err}");
        }
    }

    fn fixture() -> Array2<f32> {
        let mut arr = Array2::from_shape_fn((40, 50), |(y, x)| {
            ((y * 50 + x) as f32 * 0.37) - 100.0 + ((x * 7919) % 13) as f32
        });
        arr[[0, 0]] = f32::NAN;
        arr[[3, 4]] = f32::INFINITY;
        arr[[10, 10]] = 5000.0;
        arr[[20, 20]] = -5000.0;
        arr
    }

    #[test]
    fn resolve_limits_minmax_equals_valid_min_max() {
        let arr = fixture();
        assert_eq!(resolve_limits(&arr, LimitMode::MinMax), valid_min_max(&arr));
        assert_eq!(valid_min_max(&arr), (-5000.0, 5000.0));
    }

    #[test]
    fn resolve_limits_zscale_matches_zscale_limits() {
        let arr = fixture();
        let want = zscale_limits(&arr, 0.25);
        let got = resolve_limits(&arr, LimitMode::ZScale { contrast: 0.25 });
        assert_eq!(got, want);
        assert!(got.0.is_finite() && got.1.is_finite());
    }

    #[test]
    fn resolve_limits_percentile_full_range_equals_min_max() {
        let arr = fixture();
        let got = resolve_limits(&arr, LimitMode::Percentile { low: 0.0, high: 100.0 });
        assert_eq!(got, valid_min_max(&arr));
        let narrow = resolve_limits(&arr, LimitMode::Percentile { low: 1.0, high: 99.0 });
        assert!(narrow.0 > got.0 && narrow.1 < got.1, "{narrow:?} inside {got:?}");
    }

    #[test]
    fn resolve_limits_user_fills_missing_bounds_from_data() {
        let arr = fixture();
        let (dmin, dmax) = valid_min_max(&arr);
        assert_eq!(
            resolve_limits(&arr, LimitMode::User { vmin: None, vmax: None }),
            (dmin, dmax)
        );
        assert_eq!(
            resolve_limits(&arr, LimitMode::User { vmin: Some(-1.0), vmax: None }),
            (-1.0, dmax)
        );
        assert_eq!(
            resolve_limits(&arr, LimitMode::User { vmin: Some(-1.0), vmax: Some(2.0) }),
            (-1.0, 2.0)
        );
    }

    #[test]
    fn resolve_limits_all_nan_falls_back_to_unit_range() {
        let arr = Array2::from_elem((4, 4), f32::NAN);
        for mode in [
            LimitMode::MinMax,
            LimitMode::ZScale { contrast: 0.25 },
            LimitMode::Percentile { low: 1.0, high: 99.5 },
            LimitMode::User { vmin: None, vmax: None },
        ] {
            assert_eq!(resolve_limits(&arr, mode), (0.0, 1.0), "{mode:?}");
        }
        let empty = Array2::<f32>::zeros((0, 0));
        assert_eq!(resolve_limits(&empty, LimitMode::MinMax), (0.0, 1.0));
        let padding_only = Array2::<f32>::zeros((4, 4));
        for mode in [
            LimitMode::MinMax,
            LimitMode::ZScale { contrast: 0.25 },
            LimitMode::Percentile { low: 1.0, high: 99.5 },
        ] {
            assert_eq!(resolve_limits(&padding_only, mode), (0.0, 1.0), "{mode:?}");
        }
    }

    fn padded_sky() -> (Array2<f32>, Array2<f32>) {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut uniform = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 40) as f32 / (1u64 << 24) as f32
        };
        let sky = Array2::from_shape_fn((100, 100), |_| 1000.0 + 30.0 * ((0..12).map(|_| uniform()).sum::<f32>() - 6.0));
        let pad = |fill: f32| {
            let mut a = sky.clone();
            a.slice_mut(ndarray::s![.., 0..20]).fill(fill);
            a
        };
        (pad(0.0), pad(f32::NAN))
    }

    #[test]
    fn display_limits_ignore_zero_padding_like_nan() {
        let (zero_padded, nan_padded) = padded_sky();
        for mode in [
            LimitMode::MinMax,
            LimitMode::ZScale { contrast: 0.25 },
            LimitMode::Percentile { low: 1.0, high: 99.5 },
            LimitMode::User { vmin: None, vmax: None },
        ] {
            let got = resolve_limits(&zero_padded, mode);
            assert_eq!(got, resolve_limits(&nan_padded, mode), "{mode:?}");
            assert!(got.0 > 800.0, "{mode:?} vmin pulled to {}", got.0);
        }
    }

    #[test]
    fn normalize_and_stretch_treats_zero_padding_like_nan() {
        let data = [0.0f32, -2.0, 2.0, f32::NAN];
        let (norm, valid, below, above) = normalize_and_stretch(&data, -2.0, 2.0, StretchKind::Linear, 0.1, 2.0);
        assert!(norm[0].is_nan(), "padding mapped to {}", norm[0]);
        assert_eq!((norm[1], norm[2]), (0.0, 1.0));
        assert!(norm[3].is_nan());
        assert_eq!((valid, below, above), (2, 1, 1));
    }

    fn sequential_reference(
        data: &[f32],
        vmin: f64,
        vmax: f64,
        kind: StretchKind,
        asinh_a: f64,
        power: f64,
    ) -> (Vec<f32>, u64, u64, u64) {
        let range = vmax - vmin;
        let mut norm = Vec::with_capacity(data.len());
        let (mut valid, mut below, mut above) = (0u64, 0u64, 0u64);
        for &v in data {
            if is_valid_pixel(v) {
                valid += 1;
                let vd = v as f64;
                if vd <= vmin {
                    below += 1;
                }
                if vd >= vmax {
                    above += 1;
                }
                let n = if range > 0.0 {
                    (((vd - vmin) / range) as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                norm.push(apply_stretch(n, kind, asinh_a, power));
            } else {
                norm.push(f32::NAN);
            }
        }
        (norm, valid, below, above)
    }

    fn sample_data() -> Vec<f32> {
        let mut data: Vec<f32> = (0..20_000)
            .map(|i| ((i * 7919) % 1013) as f32 * 0.37 - 50.0)
            .collect();
        data[3] = f32::NAN;
        data[17] = f32::INFINITY;
        data[18] = f32::NEG_INFINITY;
        data[100] = 10.0;
        data[101] = 200.0;
        data
    }

    #[test]
    fn normalize_and_stretch_matches_sequential_reference_bitwise() {
        let data = sample_data();
        for kind in StretchKind::ALL {
            for (vmin, vmax) in [(10.0, 200.0), (0.0, 0.0), (-20.0, 150.5)] {
                let got = normalize_and_stretch(&data, vmin, vmax, kind, 0.05, 1.5);
                let want = sequential_reference(&data, vmin, vmax, kind, 0.05, 1.5);
                assert_eq!(got.0.len(), want.0.len());
                for (i, (g, w)) in got.0.iter().zip(want.0.iter()).enumerate() {
                    assert_eq!(
                        g.to_bits(),
                        w.to_bits(),
                        "pixel {i} differs for {kind:?} vmin={vmin} vmax={vmax}"
                    );
                }
                assert_eq!((got.1, got.2, got.3), (want.1, want.2, want.3));
            }
        }
    }

    #[test]
    fn normalize_and_stretch_counts_valid_and_clipped_pixels() {
        let data = [f32::NAN, 1.0, 5.0, 5.0, 10.0, 12.0, f32::INFINITY, 3.0];
        let (norm, valid, below, above) =
            normalize_and_stretch(&data, 5.0, 10.0, StretchKind::Linear, 0.1, 2.0);
        assert_eq!(valid, 6);
        assert_eq!(below, 4);
        assert_eq!(above, 2);
        assert!(norm[0].is_nan());
        assert!(norm[6].is_nan());
        assert_eq!(norm[1], 0.0);
        assert_eq!(norm[4], 1.0);
        assert_eq!(norm[5], 1.0);
    }

    #[test]
    fn normalize_and_stretch_empty_input() {
        let (norm, valid, below, above) =
            normalize_and_stretch(&[], 0.0, 1.0, StretchKind::Asinh, 0.1, 2.0);
        assert!(norm.is_empty());
        assert_eq!((valid, below, above), (0, 0, 0));
    }
}
