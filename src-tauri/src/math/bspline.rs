use ndarray::Array2;
use rayon::prelude::*;

const POLE: f64 = -0.267_949_192_431_122_7;
const NEAREST_PAD: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplineMode {
    Constant(f64),
    Nearest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineBoundary {
    Mirror,
    Reflect,
}

fn mirrored_causal_start(line: &[f64]) -> f64 {
    let n = line.len();
    let period = 2 * n - 2;
    let mut power = 1.0;
    let mut sum = 0.0;
    for k in 0..period {
        let index = if k < n { k } else { period - k };
        sum += power * line[index];
        power *= POLE;
    }
    sum / (1.0 - POLE.powi(period as i32))
}

fn reflected_causal_start(line: &[f64]) -> f64 {
    let n = line.len();
    let period = 2 * n;
    let mut power = POLE;
    let mut sum = 0.0;
    for k in 1..=period {
        let index = if k <= n { k - 1 } else { period - k };
        sum += power * line[index];
        power *= POLE;
    }
    line[0] + sum / (1.0 - POLE.powi(period as i32))
}

pub fn prefilter_line(line: &mut [f64], boundary: LineBoundary) {
    let n = line.len();
    if n < 2 {
        return;
    }
    let gain = (1.0 - POLE) * (1.0 - 1.0 / POLE);
    line.iter_mut().for_each(|value| *value *= gain);
    line[0] = match boundary {
        LineBoundary::Mirror => mirrored_causal_start(line),
        LineBoundary::Reflect => reflected_causal_start(line),
    };
    for k in 1..n {
        line[k] += POLE * line[k - 1];
    }
    line[n - 1] = match boundary {
        LineBoundary::Mirror => POLE / (POLE * POLE - 1.0) * (line[n - 1] + POLE * line[n - 2]),
        LineBoundary::Reflect => POLE / (POLE - 1.0) * line[n - 1],
    };
    for k in (0..n - 1).rev() {
        line[k] = POLE * (line[k + 1] - line[k]);
    }
}

fn prefilter_2d(data: &mut Array2<f64>, boundary: LineBoundary) {
    let cols = data.ncols();
    let columns: Vec<Vec<f64>> = (0..cols)
        .into_par_iter()
        .map(|col| {
            let mut line = data.column(col).to_vec();
            prefilter_line(&mut line, boundary);
            line
        })
        .collect();
    for (col, line) in columns.iter().enumerate() {
        data.column_mut(col).iter_mut().zip(line).for_each(|(dst, &value)| *dst = value);
    }
    data.as_slice_mut()
        .expect("standard layout")
        .par_chunks_mut(cols.max(1))
        .for_each(|row| prefilter_line(row, boundary));
}

fn edge_padded(data: &Array2<f64>, pad: usize) -> Array2<f64> {
    let (rows, cols) = data.dim();
    Array2::from_shape_fn((rows + 2 * pad, cols + 2 * pad), |(r, c)| {
        data[[r.saturating_sub(pad).min(rows - 1), c.saturating_sub(pad).min(cols - 1)]]
    })
}

fn extended_index(index: isize, len: usize, mode: SplineMode) -> usize {
    if len < 2 {
        return 0;
    }
    let len = len as isize;
    match mode {
        SplineMode::Constant(_) => {
            let period = 2 * (len - 1);
            let folded = index.rem_euclid(period);
            (if folded < len { folded } else { period - folded }) as usize
        }
        SplineMode::Nearest => index.clamp(0, len - 1) as usize,
    }
}

fn basis_weights(t: f64) -> [f64; 4] {
    let u = 1.0 - t;
    [
        u * u * u / 6.0,
        (t * t * (t - 2.0) * 3.0 + 4.0) / 6.0,
        (u * u * (u - 2.0) * 3.0 + 4.0) / 6.0,
        t * t * t / 6.0,
    ]
}

fn taps(coordinate: f64, len: usize, mode: SplineMode) -> ([usize; 4], [f64; 4]) {
    let coordinate = coordinate.clamp(-2.0, len as f64 + 1.0);
    let floor = coordinate.floor();
    let start = floor as isize - 1;
    let indices = [0, 1, 2, 3].map(|k| extended_index(start + k, len, mode));
    (indices, basis_weights(coordinate - floor))
}

pub struct CubicSpline {
    coefficients: Array2<f64>,
    rows: usize,
    cols: usize,
    pad: usize,
    mode: SplineMode,
}

impl CubicSpline {
    pub fn new(data: &Array2<f64>, mode: SplineMode) -> Self {
        let (rows, cols) = data.dim();
        let (mut coefficients, pad, boundary) = match mode {
            SplineMode::Nearest if rows > 0 && cols > 0 => (edge_padded(data, NEAREST_PAD), NEAREST_PAD, LineBoundary::Reflect),
            SplineMode::Nearest => (data.as_standard_layout().into_owned(), 0, LineBoundary::Reflect),
            SplineMode::Constant(_) => (data.as_standard_layout().into_owned(), 0, LineBoundary::Mirror),
        };
        prefilter_2d(&mut coefficients, boundary);
        Self { coefficients, rows, cols, pad, mode }
    }

    pub fn coefficients(&self) -> &Array2<f64> {
        &self.coefficients
    }

    pub fn value(&self, y: f64, x: f64) -> f64 {
        let (y, x) = match self.mode {
            SplineMode::Constant(cval) => {
                let inside = |v: f64, len: usize| len > 0 && (0.0..=(len - 1) as f64).contains(&v);
                if !inside(y, self.rows) || !inside(x, self.cols) {
                    return cval;
                }
                (y, x)
            }
            SplineMode::Nearest => {
                if self.rows == 0 || self.cols == 0 || y.is_nan() || x.is_nan() {
                    return f64::NAN;
                }
                let pad = self.pad as f64;
                (y + pad, x + pad)
            }
        };
        let (rows, cols) = self.coefficients.dim();
        let (row_taps, row_weights) = taps(y, rows, self.mode);
        let (col_taps, col_weights) = taps(x, cols, self.mode);
        let mut total = 0.0;
        for (&r, &wy) in row_taps.iter().zip(&row_weights) {
            let mut line = 0.0;
            for (&c, &wx) in col_taps.iter().zip(&col_weights) {
                line += wx * self.coefficients[[r, c]];
            }
            total += wy * line;
        }
        total
    }
}

fn sample_grid(spline: &CubicSpline, shape: (usize, usize), coordinate: impl Fn(usize, usize) -> (f64, f64) + Sync) -> Array2<f64> {
    let (rows, cols) = shape;
    let mut out = vec![0.0; rows * cols];
    out.par_chunks_mut(cols.max(1)).enumerate().for_each(|(r, row)| {
        for (c, value) in row.iter_mut().enumerate() {
            let (y, x) = coordinate(r, c);
            *value = spline.value(y, x);
        }
    });
    Array2::from_shape_vec(shape, out).expect("shape matches the buffer")
}

pub fn interpolate_line(values: &[f64], coordinates: &[f64], mode: SplineMode) -> Vec<f64> {
    let n = values.len();
    let (coefficients, pad) = match mode {
        SplineMode::Nearest if n > 0 => {
            let mut padded: Vec<f64> = (0..n + 2 * NEAREST_PAD).map(|i| values[i.saturating_sub(NEAREST_PAD).min(n - 1)]).collect();
            prefilter_line(&mut padded, LineBoundary::Reflect);
            (padded, NEAREST_PAD)
        }
        _ => {
            let mut line = values.to_vec();
            prefilter_line(&mut line, LineBoundary::Mirror);
            (line, 0)
        }
    };
    coordinates
        .iter()
        .map(|&x| {
            let x = match mode {
                SplineMode::Constant(cval) => {
                    if n == 0 || !(0.0..=(n - 1) as f64).contains(&x) {
                        return cval;
                    }
                    x
                }
                SplineMode::Nearest => {
                    if n == 0 || x.is_nan() {
                        return f64::NAN;
                    }
                    x + pad as f64
                }
            };
            let (indices, weights) = taps(x, coefficients.len(), mode);
            indices.iter().zip(&weights).map(|(&i, &w)| w * coefficients[i]).sum()
        })
        .collect()
}

pub fn zoomed_len(len: usize, factor: f64) -> usize {
    (len as f64 * factor).round_ties_even().max(0.0) as usize
}

pub fn shift(data: &Array2<f64>, shift: [f64; 2], mode: SplineMode) -> Array2<f64> {
    let spline = CubicSpline::new(data, mode);
    sample_grid(&spline, data.dim(), |r, c| (r as f64 - shift[0], c as f64 - shift[1]))
}

pub fn affine_transform(
    data: &Array2<f64>,
    matrix: [[f64; 2]; 2],
    offset: [f64; 2],
    output_shape: (usize, usize),
    mode: SplineMode,
) -> Array2<f64> {
    let spline = CubicSpline::new(data, mode);
    sample_grid(&spline, output_shape, |r, c| {
        let (r, c) = (r as f64, c as f64);
        (matrix[0][0] * r + matrix[0][1] * c + offset[0], matrix[1][0] * r + matrix[1][1] * c + offset[1])
    })
}

pub fn zoom(data: &Array2<f64>, factors: [f64; 2], mode: SplineMode) -> Array2<f64> {
    let (rows, cols) = data.dim();
    let shape = (zoomed_len(rows, factors[0]), zoomed_len(cols, factors[1]));
    let step = |input: usize, output: usize| if output > 1 { (input as f64 - 1.0) / (output as f64 - 1.0) } else { 1.0 };
    let (step_y, step_x) = (step(rows, shape.0), step(cols, shape.1));
    let spline = CubicSpline::new(data, mode);
    sample_grid(&spline, shape, |r, c| (r as f64 * step_y, c as f64 * step_x))
}

#[cfg(test)]
pub(crate) mod scipy_golden {
    use std::path::{Path, PathBuf};

    use ndarray::Array2;
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    pub fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack")
    }

    fn read_json(path: &Path) -> Value {
        let text = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn stale_input(fixtures: &Path, folder: &str) -> Result<(), String> {
        let name = format!("{folder}/manifest.json");
        let manifest = read_json(&fixtures.join(folder).join("manifest.json"));
        if manifest["kind"] != "scipy_reference" {
            return Err(format!("{name}: kind {} is not scipy_reference", manifest["kind"]));
        }
        let inputs = manifest["inputs"].as_array().filter(|inputs| !inputs.is_empty()).ok_or_else(|| format!("{name}: no inputs list"))?;
        let regenerate = manifest["regenerate"].as_str().unwrap_or("its generator");
        for input in inputs {
            let path = input["path"].as_str().ok_or_else(|| format!("{name}: an input has no path"))?;
            let bytes = std::fs::read(fixtures.join(path)).map_err(|error| format!("{name}: input {path}: {error}"))?;
            if Some(bytes.len() as u64) != input["bytes"].as_u64() || Some(sha256_hex(&bytes).as_str()) != input["sha256"].as_str() {
                return Err(format!("{name}: input {path} differs from the manifest; the goldens are stale, re-run {regenerate}"));
            }
        }
        Ok(())
    }

    pub fn checked_in(fixtures: &Path, folder: &str, file: &str) -> Value {
        if let Err(message) = stale_input(fixtures, folder) {
            panic!("{message}");
        }
        read_json(&fixtures.join(folder).join(file))
    }

    pub fn checked(folder: &str, file: &str) -> Value {
        checked_in(&fixtures_dir(), folder, file)
    }

    pub fn load() -> Value {
        checked("scipy", "scipy_spline.json")
    }

    pub fn number(value: &Value) -> f64 {
        value.as_f64().unwrap_or(f64::NAN)
    }

    pub fn vector(value: &Value) -> Vec<f64> {
        value.as_array().unwrap().iter().map(number).collect()
    }

    pub fn array2(value: &Value) -> Array2<f64> {
        let rows: Vec<Vec<f64>> = value.as_array().unwrap().iter().map(vector).collect();
        let cols = rows.first().map_or(0, Vec::len);
        Array2::from_shape_vec((rows.len(), cols), rows.concat()).unwrap()
    }

    pub fn max_abs(values: impl IntoIterator<Item = f64>) -> f64 {
        values.into_iter().filter(|v| v.is_finite()).fold(0.0, |acc, v| acc.max(v.abs()))
    }

    pub fn assert_close(got: &Array2<f64>, expected: &Array2<f64>, relative: f64, what: &str) {
        assert_eq!(got.dim(), expected.dim(), "{what}: shape");
        let scale = max_abs(expected.iter().copied());
        let mut worst = 0.0f64;
        for ((index, g), e) in got.indexed_iter().zip(expected.iter()) {
            assert_eq!(g.is_nan(), e.is_nan(), "{what}: NaN pattern at {index:?}: {g} against {e}");
            if e.is_finite() {
                worst = worst.max((g - e).abs());
            }
        }
        assert!(worst <= relative * scale, "{what}: max difference {worst:e} against {:e} ({relative:e} of {scale:e})", relative * scale);
    }
}

#[cfg(test)]
mod tests {
    use super::scipy_golden::{array2, assert_close, load, number, vector};
    use super::*;
    use serde_json::Value;

    const PARITY: f64 = 1e-12;

    fn grid() -> Array2<f64> {
        Array2::from_shape_fn((7, 9), |(r, c)| ((r * 9 + c) * 37 % 23) as f64 - 11.0 + 0.25 * r as f64)
    }

    fn mode_of(case: &Value) -> SplineMode {
        match case["mode"].as_str().unwrap() {
            "nearest" => SplineMode::Nearest,
            "constant" => SplineMode::Constant(number(&case["cval"])),
            other => panic!("unexpected mode {other}"),
        }
    }

    fn pair(value: &Value) -> [f64; 2] {
        let values = vector(value);
        [values[0], values[1]]
    }

    #[test]
    fn bspline_shift_matches_scipy_reference_values() {
        let golden = load();
        let data = grid();
        assert_eq!(data, array2(&golden["grid"]), "the test grid differs from the one scipy saw");

        let filters = &golden["spline_filter1d"];
        for (boundary, key) in [(LineBoundary::Mirror, "mirror"), (LineBoundary::Reflect, "reflect")] {
            let mut line = vector(&filters["line"]);
            prefilter_line(&mut line, boundary);
            let got = Array2::from_shape_vec((1, line.len()), line).unwrap();
            let expected = Array2::from_shape_vec((1, got.ncols()), vector(&filters[key])).unwrap();
            assert_close(&got, &expected, PARITY, &format!("spline_filter1d {key}"));
        }
        let coefficients = CubicSpline::new(&data, SplineMode::Constant(0.0));
        assert_close(coefficients.coefficients(), &array2(&golden["spline_filter_mirror"]), PARITY, "spline_filter mirror");

        for case in golden["shift"].as_array().unwrap() {
            let got = shift(&data, pair(&case["shift"]), mode_of(case));
            assert_close(&got, &array2(&case["output"]), PARITY, &format!("shift {} {}", case["shift"], case["mode"]));
        }
    }

    #[test]
    fn spline_values_follow_scipy_edge_rules_for_constant_and_nearest_modes() {
        let golden = load();
        let data = grid();
        let cases = &golden["map_coordinates"];
        let points: Vec<[f64; 2]> = cases["points"].as_array().unwrap().iter().map(pair).collect();
        let constant = CubicSpline::new(&data, SplineMode::Constant(number(&cases["constant"]["cval"])));
        let nearest = CubicSpline::new(&data, SplineMode::Nearest);
        let expected_constant = vector(&cases["constant"]["values"]);
        let expected_nearest = vector(&cases["nearest"]);
        for (i, [y, x]) in points.iter().copied().enumerate() {
            let got = constant.value(y, x);
            assert!((got - expected_constant[i]).abs() <= 1e-12 * 12.0, "constant at ({y}, {x}): {got} against {}", expected_constant[i]);
            let got = nearest.value(y, x);
            assert!((got - expected_nearest[i]).abs() <= 1e-12 * 14.0, "nearest at ({y}, {x}): {got} against {}", expected_nearest[i]);
        }
    }

    #[test]
    fn line_interpolation_matches_scipy_map_coordinates_in_one_dimension() {
        let golden = load();
        let case = &golden["line"];
        let values = vector(&case["values"]);
        let coordinates = vector(&case["coordinates"]);
        let nearest = interpolate_line(&values, &coordinates, SplineMode::Nearest);
        let constant = interpolate_line(&values, &coordinates, SplineMode::Constant(number(&case["constant"]["cval"])));
        for (got, expected, what) in [(nearest, vector(&case["nearest"]), "nearest"), (constant, vector(&case["constant"]["values"]), "constant")] {
            let got = Array2::from_shape_vec((1, got.len()), got).unwrap();
            let expected = Array2::from_shape_vec((1, expected.len()), expected).unwrap();
            assert_close(&got, &expected, PARITY, &format!("1-D {what}"));
        }
    }

    #[test]
    fn affine_transform_with_a_rotation_matches_scipy() {
        let golden = load();
        let case = &golden["affine_rotation"];
        let matrix = array2(&case["matrix"]);
        let shape = vector(&case["output_shape"]);
        let got = affine_transform(
            &grid(),
            [[matrix[[0, 0]], matrix[[0, 1]]], [matrix[[1, 0]], matrix[[1, 1]]]],
            pair(&case["offset"]),
            (shape[0] as usize, shape[1] as usize),
            SplineMode::Constant(0.0),
        );
        assert_close(&got, &array2(&case["output"]), PARITY, "affine_transform rotation");
    }

    #[test]
    fn checked_loader_fails_on_a_stale_input() {
        let fixtures = scipy_golden::fixtures_dir();
        let scratch = tempfile::tempdir().unwrap();
        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(fixtures.join("kernels").join("manifest.json")).unwrap()).unwrap();
        std::fs::create_dir_all(scratch.path().join("kernels")).unwrap();
        for file in ["manifest.json", "kernel_goldens.json"] {
            std::fs::copy(fixtures.join("kernels").join(file), scratch.path().join("kernels").join(file)).unwrap();
        }
        let inputs: Vec<&str> = manifest["inputs"].as_array().unwrap().iter().map(|input| input["path"].as_str().unwrap()).collect();
        assert_eq!(inputs[0], "d1/psf/d1_psf_JWST_F444W_40mas.fits");
        for input in &inputs {
            let copy = scratch.path().join(input);
            std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
            std::fs::copy(fixtures.join(input), &copy).unwrap();
        }
        assert_eq!(scipy_golden::stale_input(scratch.path(), "kernels"), Ok(()), "a faithful copy verifies");
        assert_eq!(scipy_golden::checked_in(scratch.path(), "kernels", "kernel_goldens.json")["target"], scipy_golden::checked("kernels", "kernel_goldens.json")["target"]);

        let flipped = scratch.path().join(inputs[0]);
        let mut bytes = std::fs::read(&flipped).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&flipped, &bytes).unwrap();
        let message = scipy_golden::stale_input(scratch.path(), "kernels").unwrap_err();
        let expected = format!(
            "kernels/manifest.json: input d1/psf/d1_psf_JWST_F444W_40mas.fits differs from the manifest; the goldens are stale, re-run {}",
            manifest["regenerate"].as_str().unwrap()
        );
        assert_eq!(message, expected);
        let root = scratch.path();
        let payload = std::panic::catch_unwind(|| scipy_golden::checked_in(root, "kernels", "kernel_goldens.json")).expect_err("a stale input must stop the loader");
        let panicked = payload.downcast_ref::<String>().map(String::as_str).or_else(|| payload.downcast_ref::<&str>().copied());
        assert_eq!(panicked, Some(expected.as_str()));
    }

    #[test]
    fn zoom_matches_scipy_shapes_and_values() {
        let golden = load();
        for case in golden["zoom"].as_array().unwrap() {
            let factor = number(&case["factor"]);
            let got = zoom(&grid(), [factor, factor], mode_of(case));
            assert_close(&got, &array2(&case["output"]), PARITY, &format!("zoom {factor} {}", case["mode"]));
        }
        assert_eq!(zoomed_len(61, 0.5), 30, "Python rounds half to even");
        assert_eq!(zoomed_len(63, 0.5), 32);
        assert_eq!(zoomed_len(21, 2.0), 42);
    }
}
