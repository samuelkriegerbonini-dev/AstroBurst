use std::fmt;

pub mod eval;
pub mod lexer;
pub mod parser;

pub use eval::{compile, evaluate, validate, OutputOptions, Program};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub fn new(start: usize, len: usize) -> Self {
        Self { start, len }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelMathError {
    pub message: String,
    pub position: Option<usize>,
    pub length: Option<usize>,
}

impl PixelMathError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            position: None,
            length: None,
        }
    }

    pub fn at(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            position: Some(span.start),
            length: Some(span.len.max(1)),
        }
    }
}

impl fmt::Display for PixelMathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some(position) => write!(f, "{} at {}", self.message, position),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for PixelMathError {}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    const EPS: f32 = 1e-5;

    fn names(extra: &[&str]) -> Vec<String> {
        std::iter::once("$T")
            .chain(extra.iter().copied())
            .map(String::from)
            .collect()
    }

    fn eval_on(
        expr: &str,
        slots: &[(&str, &Array2<f32>)],
        opts: &OutputOptions,
    ) -> Result<Array2<f32>, PixelMathError> {
        let slot_names: Vec<String> = slots.iter().map(|(n, _)| n.to_string()).collect();
        let arrays: Vec<&Array2<f32>> = slots.iter().map(|(_, a)| *a).collect();
        let program = compile(expr, &slot_names)?;
        evaluate(&program, &arrays, opts)
    }

    fn scalar(expr: &str) -> f32 {
        let target = Array2::<f32>::zeros((1, 1));
        eval_on(expr, &[("$T", &target)], &OutputOptions::default()).unwrap()[[0, 0]]
    }

    fn assert_close(actual: f32, expected: f32, what: &str) {
        let tol = EPS * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tol,
            "{what}: expected {expected}, got {actual}"
        );
    }

    fn image4x4() -> Array2<f32> {
        Array2::from_shape_vec((4, 4), (1..=16).map(|v| v as f32).collect()).unwrap()
    }

    fn image_with_nan_and_tiny() -> Array2<f32> {
        let mut img = image4x4();
        img[[0, 0]] = f32::NAN;
        img[[3, 3]] = 1e-9;
        img
    }

    fn finite_values(img: &Array2<f32>) -> Vec<f64> {
        img.iter().filter(|v| v.is_finite()).map(|&v| v as f64).collect()
    }

    #[test]
    fn precedence_and_associativity() {
        assert_close(scalar("2^3^2"), 512.0, "2^3^2");
        assert_close(scalar("-2^2"), -4.0, "-2^2");
        assert_close(scalar("1 + 2 * 3"), 7.0, "1 + 2 * 3");
        assert_close(scalar("~0.25"), 0.75, "~0.25");
        assert_close(scalar("(1 + 2) * 3"), 9.0, "(1 + 2) * 3");
        assert_close(scalar("2 * 3^2"), 18.0, "2 * 3^2");
        assert_close(scalar("10 - 4 - 3"), 3.0, "10 - 4 - 3");
        assert_close(scalar("8 / 4 / 2"), 1.0, "8 / 4 / 2");
        assert_close(scalar("7 % 3"), 1.0, "7 % 3");
        assert_close(scalar("-~0.25"), -0.75, "-~0.25");
        assert_close(scalar("2 * -3"), -6.0, "2 * -3");
        assert_close(scalar("2^-1"), 0.5, "2^-1");
        assert_close(scalar("1e3 + 2.5E-1 + .5"), 1000.75, "scientific numbers");
        assert_close(scalar("1 +\n 2\n"), 3.0, "newlines are whitespace");
    }

    #[test]
    fn comparisons_and_logic() {
        assert_close(scalar("1 < 2"), 1.0, "1 < 2");
        assert_close(scalar("2 <= 2"), 1.0, "2 <= 2");
        assert_close(scalar("3 > 4"), 0.0, "3 > 4");
        assert_close(scalar("3 >= 3"), 1.0, "3 >= 3");
        assert_close(scalar("1 == 1"), 1.0, "1 == 1");
        assert_close(scalar("1 != 1"), 0.0, "1 != 1");
        assert_close(scalar("1 && 0"), 0.0, "1 && 0");
        assert_close(scalar("1 || 0"), 1.0, "1 || 0");
        assert_close(scalar("!0"), 1.0, "!0");
        assert_close(scalar("!5"), 0.0, "!5");
        assert_close(scalar("1 + 1 == 2 && 3 > 2"), 1.0, "arithmetic binds tighter than comparison");
        assert_close(scalar("0 || 1 && 0"), 0.0, "&& binds tighter than ||");
        assert_close(scalar("iif(2 > 1, 10, 20)"), 10.0, "iif true");
        assert_close(scalar("iif(2 < 1, 10, 20)"), 20.0, "iif false");
    }

    #[test]
    fn per_pixel_functions_on_known_values() {
        assert_close(scalar("abs(-3)"), 3.0, "abs");
        assert_close(scalar("sqrt(16)"), 4.0, "sqrt");
        assert_close(scalar("exp(0)"), 1.0, "exp");
        assert_close(scalar("ln(e())"), 1.0, "ln");
        assert_close(scalar("log(1000)"), 3.0, "log");
        assert_close(scalar("log2(8)"), 3.0, "log2");
        assert_close(scalar("pow(2, 10)"), 1024.0, "pow");
        assert_close(scalar("min(3, 1, 2)"), 1.0, "min n-ary");
        assert_close(scalar("max(3, 1, 2)"), 3.0, "max n-ary");
        assert_close(scalar("min(4, 9)"), 4.0, "min binary");
        assert_close(scalar("floor(2.7)"), 2.0, "floor");
        assert_close(scalar("floor(-2.2)"), -3.0, "floor negative");
        assert_close(scalar("ceil(2.1)"), 3.0, "ceil");
        assert_close(scalar("round(2.5)"), 3.0, "round half away from zero");
        assert_close(scalar("round(-2.5)"), -3.0, "round negative");
        assert_close(scalar("trunc(-2.7)"), -2.0, "trunc");
        assert_close(scalar("sign(-4)"), -1.0, "sign negative");
        assert_close(scalar("sign(0)"), 0.0, "sign zero");
        assert_close(scalar("sign(4)"), 1.0, "sign positive");
        assert_close(scalar("clip(5, 0, 1)"), 1.0, "clip high");
        assert_close(scalar("clip(-1, 0, 1)"), 0.0, "clip low");
        assert_close(scalar("clip(0.5, 0, 1)"), 0.5, "clip inside");
        assert_close(scalar("rescale(5, 0, 10, 0, 1)"), 0.5, "rescale");
        assert_close(scalar("rescale(0, 0, 10, 100, 200)"), 100.0, "rescale offset");
        assert_close(scalar("rescale(10, 0, 10, 100, 200)"), 200.0, "rescale top");
        assert_close(scalar("pi()"), std::f32::consts::PI, "pi");
        assert_close(scalar("e()"), std::f32::consts::E, "e");
        assert_close(scalar("$T + 1"), 1.0, "target symbol");
    }

    #[test]
    fn reducers_ignore_nan_and_include_tiny_values() {
        let img = image_with_nan_and_tiny();
        let finite = finite_values(&img);
        assert_eq!(finite.len(), 15);
        let n = finite.len() as f64;
        let mean = finite.iter().sum::<f64>() / n;
        let sdev = (finite.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
        let adev = finite.iter().map(|v| (v - 8.0).abs()).sum::<f64>() / n;
        let opts = OutputOptions::default();
        let cases: [(&str, f32); 7] = [
            ("mean($T)", mean as f32),
            ("med($T)", 8.0),
            ("mdev($T)", 4.0),
            ("sdev($T)", sdev as f32),
            ("adev($T)", adev as f32),
            ("min($T)", 1e-9),
            ("max($T)", 15.0),
        ];
        for (expr, expected) in cases {
            let out = eval_on(expr, &[("$T", &img)], &opts).unwrap();
            assert_close(out[[0, 0]], expected, expr);
            assert_close(out[[3, 3]], expected, expr);
            assert!(out.iter().all(|v| (v - expected).abs() <= EPS * expected.abs().max(1.0)), "{expr} is not constant");
        }
        let per_pixel = eval_on("min($T, 3)", &[("$T", &img)], &opts).unwrap();
        assert!(per_pixel[[0, 0]].is_nan());
        assert_close(per_pixel[[0, 1]], 2.0, "min($T, 3) at 2");
        assert_close(per_pixel[[1, 3]], 3.0, "min($T, 3) at 8");
    }

    #[test]
    fn reducers_over_a_named_slot_and_mixed_with_pixels() {
        let target = image4x4();
        let flat = Array2::from_shape_vec((4, 4), vec![2.0; 16]).unwrap();
        let out = eval_on(
            "$T * (A / med(A)) - min(A)",
            &[("$T", &target), ("A", &flat)],
            &OutputOptions::default(),
        )
        .unwrap();
        assert_close(out[[0, 0]], -1.0, "1 * 1 - 2");
        assert_close(out[[3, 3]], 14.0, "16 * 1 - 2");
    }

    #[test]
    fn nan_semantics() {
        assert!(scalar("(0/0) + 1").is_nan(), "arithmetic propagates NaN");
        assert!(scalar("min(0/0, 1)").is_nan(), "min propagates NaN");
        assert!(scalar("max(1, 0/0)").is_nan(), "max propagates NaN");
        assert!(scalar("max(1, 2, 0/0)").is_nan(), "n-ary max propagates NaN");
        assert!(scalar("-(0/0)").is_nan(), "negation propagates NaN");
        assert!(scalar("~(0/0)").is_nan(), "inversion propagates NaN");
        assert!(scalar("abs(0/0)").is_nan(), "abs propagates NaN");
        assert!(scalar("clip(0/0, 0, 1)").is_nan(), "clip propagates NaN");
        assert!(scalar("sqrt(-1)").is_nan(), "sqrt of negative is NaN");
        assert_close(scalar("(0/0) < 1"), 0.0, "NaN < 1");
        assert_close(scalar("(0/0) <= 1"), 0.0, "NaN <= 1");
        assert_close(scalar("(0/0) > 1"), 0.0, "NaN > 1");
        assert_close(scalar("(0/0) >= 1"), 0.0, "NaN >= 1");
        assert_close(scalar("(0/0) == (0/0)"), 0.0, "NaN == NaN");
        assert_close(scalar("(0/0) != 1"), 1.0, "NaN != 1 (IEEE)");
        assert_close(scalar("(0/0) != (0/0)"), 1.0, "NaN != NaN (IEEE)");
        assert_close(scalar("iif(0/0, 1, 2)"), 2.0, "iif(NaN, a, b) yields b");
        assert_close(scalar("(0/0) && 1"), 0.0, "NaN is falsy in &&");
        assert_close(scalar("(0/0) || 1"), 1.0, "NaN is falsy in ||");
        assert_close(scalar("!(0/0)"), 1.0, "!NaN");
        assert_close(scalar("iif(1/0 > 1e30, 1, 0)"), 1.0, "division by zero yields +inf inside the expression");
        assert_close(scalar("iif(-1/0 < -1e30, 1, 0)"), 1.0, "division by zero yields -inf inside the expression");
        assert!(scalar("1/0").is_nan(), "non-finite output is written as NaN");
        assert!(scalar("ln(0)").is_nan(), "-inf output is written as NaN");
        assert!(scalar("0/0").is_nan(), "0/0 is NaN");
    }

    #[test]
    fn nan_fill_expression_replaces_nan_with_zero() {
        let img = image_with_nan_and_tiny();
        let out = eval_on("iif($T != $T, 0, $T)", &[("$T", &img)], &OutputOptions::default()).unwrap();
        assert_eq!(out[[0, 0]], 0.0);
        assert_close(out[[0, 1]], 2.0, "unchanged pixel");
        assert_close(out[[3, 3]], 1e-9, "tiny pixel kept");
        assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn dimension_mismatch_names_both_slots_and_sizes() {
        let target = image4x4();
        let other = Array2::<f32>::zeros((4, 5));
        let err = eval_on("$T + A", &[("$T", &target), ("A", &other)], &OutputOptions::default()).unwrap_err();
        assert!(err.message.contains("$T"), "{}", err.message);
        assert!(err.message.contains("A"), "{}", err.message);
        assert!(err.message.contains("4x4"), "{}", err.message);
        assert!(err.message.contains("5x4"), "{}", err.message);
    }

    #[test]
    fn unknown_symbol_lists_available_names() {
        let err = compile("$T + Q", &names(&["A", "B"])).unwrap_err();
        assert!(err.message.contains("'Q'"), "{}", err.message);
        assert!(err.message.contains("$T"), "{}", err.message);
        assert!(err.message.contains("A"), "{}", err.message);
        assert!(err.message.contains("B"), "{}", err.message);
        assert_eq!(err.position, Some(5));
        assert_eq!(err.length, Some(1));
        let long = compile("halpha * 2", &names(&["ha"])).unwrap_err();
        assert_eq!(long.position, Some(0));
        assert_eq!(long.length, Some(6));
    }

    #[test]
    fn parse_error_positions() {
        let err = validate("1 + * 2", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(4), "{}", err.message);
        assert_eq!(err.length, Some(1));
        let err = validate("foo(1)", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(0), "{}", err.message);
        assert_eq!(err.length, Some(3));
        assert!(err.message.contains("foo"));
        let err = validate("(1 + 2", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(6), "{}", err.message);
        let err = validate("1 +", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(3), "{}", err.message);
        let err = validate("abs(1, 2)", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(0), "{}", err.message);
        assert_eq!(err.length, Some(3));
        let err = validate("2 $T", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(2), "{}", err.message);
        assert_eq!(err.length, Some(2));
        let err = validate("1 = 2", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(2), "{}", err.message);
        let err = validate("1 # 2", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(2), "{}", err.message);
        let err = validate("", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(0), "{}", err.message);
        let err = validate("mean(1)", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(0), "{}", err.message);
        let err = validate("min(1)", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(0), "{}", err.message);
        let err = validate("pi(1)", &names(&[])).unwrap_err();
        assert_eq!(err.position, Some(0), "{}", err.message);
        assert!(validate("mean($T) + min(A, 2)", &names(&["A"])).is_ok());
    }

    #[test]
    fn slot_name_rules_in_compile() {
        let dup = compile("$T", &["$T".to_string(), "A".to_string(), "A".to_string()]).unwrap_err();
        assert!(dup.message.contains("duplicate"), "{}", dup.message);
        let bad = compile("$T", &["$T".to_string(), "1A".to_string()]).unwrap_err();
        assert!(bad.message.contains("'1A'"), "{}", bad.message);
        let empty = compile("$T", &["$T".to_string(), "".to_string()]).unwrap_err();
        assert!(empty.message.contains("empty"), "{}", empty.message);
        assert!(compile("(ha + oiii) / 2", &names(&["ha", "oiii"])).is_ok());
        assert!(compile("(ha + oiii) / 2", &["ha".to_string(), "oiii".to_string()]).is_ok());
    }

    #[test]
    fn evaluate_rejects_slot_count_mismatch() {
        let program = compile("$T + A", &names(&["A"])).unwrap();
        let target = image4x4();
        let err = evaluate(&program, &[&target], &OutputOptions::default()).unwrap_err();
        assert!(err.message.contains("2"), "{}", err.message);
        assert!(err.message.contains("1"), "{}", err.message);
        let none = compile("1", &[]).unwrap();
        assert!(evaluate(&none, &[], &OutputOptions::default()).is_err());
    }

    #[test]
    fn truncate_clamps_finite_values_to_unit_range() {
        let img = image_with_nan_and_tiny();
        let opts = OutputOptions { truncate: true, rescale: false };
        let out = eval_on("$T / 8 - 0.5", &[("$T", &img)], &opts).unwrap();
        assert!(out[[0, 0]].is_nan());
        assert_eq!(out[[0, 1]], 0.0);
        assert_close(out[[1, 3]], 0.5, "8/8 - 0.5");
        assert_eq!(out[[3, 2]], 1.0);
        let inf = eval_on("iif($T == 2, 1/0, $T)", &[("$T", &img)], &opts).unwrap();
        assert!(inf[[0, 1]].is_nan(), "inf is not clamped, it becomes NaN");
        assert_eq!(inf[[0, 2]], 1.0);
    }

    #[test]
    fn rescale_maps_finite_range_to_unit_and_keeps_nan() {
        let img = image_with_nan_and_tiny();
        let opts = OutputOptions { truncate: false, rescale: true };
        let out = eval_on("$T", &[("$T", &img)], &opts).unwrap();
        assert!(out[[0, 0]].is_nan());
        assert_close(out[[3, 3]], 0.0, "min maps to 0");
        assert_close(out[[3, 2]], 1.0, "max maps to 1");
        assert_close(out[[1, 1]], 6.0 / 15.0, "interior value");
        let with_inf = eval_on("iif($T == 15, 1/0, $T)", &[("$T", &img)], &opts).unwrap();
        assert!(with_inf[[3, 2]].is_nan(), "inf ignored by rescale and written as NaN");
        assert_close(with_inf[[3, 1]], 1.0, "14 is the finite max");
        let constant = eval_on("5", &[("$T", &img)], &opts).unwrap();
        assert_eq!(constant[[0, 0]], 0.0);
        let both = OutputOptions { truncate: true, rescale: true };
        let out = eval_on("$T * 100 - 50", &[("$T", &img)], &both).unwrap();
        assert_close(out[[3, 2]], 1.0, "rescale then truncate keeps the full range");
        assert_close(out[[3, 3]], 0.0, "rescale then truncate keeps the minimum");
    }

    #[test]
    fn large_image_evaluates_with_two_slots() {
        let n = 2048usize;
        let target = Array2::from_shape_fn((n, n), |(r, c)| ((r * n + c) % 1000) as f32);
        let ones = Array2::<f32>::ones((n, n));
        let out = eval_on("$T * 2 + A", &[("$T", &target), ("A", &ones)], &OutputOptions::default()).unwrap();
        assert_eq!(out.dim(), (n, n));
        assert_eq!(out[[0, 0]], 1.0);
        assert_eq!(out[[1, 0]], 97.0);
        assert_eq!(out[[n - 1, n - 1]], (((n * n - 1) % 1000) * 2 + 1) as f32);
    }

    #[test]
    fn non_contiguous_slots_are_supported() {
        let big = Array2::from_shape_fn((6, 6), |(r, c)| (r * 6 + c) as f32);
        let view = big.slice(ndarray::s![1..5, 1..5]).to_owned().reversed_axes();
        assert!(view.as_slice().is_none());
        let out = eval_on("$T + 1", &[("$T", &view)], &OutputOptions::default()).unwrap();
        assert_eq!(out[[0, 0]], 8.0);
        assert_eq!(out[[1, 0]], 9.0);
        assert_eq!(out[[0, 1]], 14.0);
    }

    #[test]
    fn error_display_includes_position() {
        let err = PixelMathError::at("unexpected token '*'", Span::new(4, 1));
        assert_eq!(err.to_string(), "unexpected token '*' at 4");
        let plain = PixelMathError::new("no slots");
        assert_eq!(plain.to_string(), "no slots");
        assert_eq!(PixelMathError::at("end", Span::new(3, 0)).length, Some(1));
    }
}
