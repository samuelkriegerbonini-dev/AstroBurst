use super::GwcsError;

pub const MAX_LANES: usize = 4;

const DEG_PER_RAD: f64 = 180.0 / std::f64::consts::PI;
const AFFINE_SINGULAR_DET: f64 = 1e-300;

#[derive(Debug, Clone, Copy)]
pub struct Vals {
    pub v: [f64; MAX_LANES],
    pub n: usize,
}

impl Vals {
    pub fn from_slice(values: &[f64]) -> Self {
        let n = values.len().min(MAX_LANES);
        let mut v = [f64::NAN; MAX_LANES];
        v[..n].copy_from_slice(&values[..n]);
        Self { v, n }
    }

    pub fn one(x: f64) -> Self {
        Self::from_slice(&[x])
    }

    pub fn pair(x: f64, y: f64) -> Self {
        Self::from_slice(&[x, y])
    }

    pub fn triple(x: f64, y: f64, z: f64) -> Self {
        Self::from_slice(&[x, y, z])
    }

    pub fn as_slice(&self) -> &[f64] {
        &self.v[..self.n]
    }

    fn concat(self, other: Vals) -> Vals {
        let mut v = [f64::NAN; MAX_LANES];
        let n = (self.n + other.n).min(MAX_LANES);
        v[..self.n].copy_from_slice(&self.v[..self.n]);
        let tail = n - self.n;
        v[self.n..n].copy_from_slice(&other.v[..tail]);
        Vals { v, n }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn from_char(c: char) -> Option<Axis> {
        match c.to_ascii_lowercase() {
            'x' => Some(Axis::X),
            'y' => Some(Axis::Y),
            'z' => Some(Axis::Z),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
}

impl ArithOp {
    pub fn from_tag(tag: &str) -> Option<ArithOp> {
        match tag {
            "add" => Some(ArithOp::Add),
            "subtract" => Some(ArithOp::Subtract),
            "multiply" => Some(ArithOp::Multiply),
            "divide" => Some(ArithOp::Divide),
            "power" => Some(ArithOp::Power),
            _ => None,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            ArithOp::Add => "add",
            ArithOp::Subtract => "subtract",
            ArithOp::Multiply => "multiply",
            ArithOp::Divide => "divide",
            ArithOp::Power => "power",
        }
    }

    pub fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            ArithOp::Add => a + b,
            ArithOp::Subtract => a - b,
            ArithOp::Multiply => a * b,
            ArithOp::Divide => a / b,
            ArithOp::Power => a.powf(b),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Kind {
    Compose(Box<Node>, Box<Node>),
    Concatenate(Box<Node>, Box<Node>),
    Arithmetic(ArithOp, Box<Node>, Box<Node>),
    Constant { value: f64, n_inputs: usize },
    Shift(f64),
    Scale(f64),
    Poly1D { coeffs: Vec<f64>, domain: Option<[f64; 2]>, window: Option<[f64; 2]> },
    Poly2D { degree: usize, coeffs: Vec<f64>, domain: Option<[[f64; 2]; 2]>, window: Option<[[f64; 2]; 2]> },
    Mapping { mapping: Vec<usize>, n_inputs: usize },
    Identity(usize),
    Affine2D { matrix: [[f64; 2]; 2], translation: [f64; 2] },
    Rotation2D { angle_deg: f64 },
    RotationSequence3D { angles_deg: Vec<f64>, axes: Vec<Axis>, matrix: [[f64; 3]; 3], spherical: bool },
    SphericalToCartesian { wrap_lon_at: u16 },
    CartesianToSpherical { wrap_lon_at: u16 },
    Pix2SkyTan,
    Sky2PixTan,
    RotateNative2Celestial { lon: f64, lat: f64, lon_pole: f64, matrix: [[f64; 3]; 3] },
    RotateCelestial2Native { lon: f64, lat: f64, lon_pole: f64, matrix: [[f64; 3]; 3] },
}

impl Kind {
    pub fn rotation_sequence(angles_deg: Vec<f64>, axes: Vec<Axis>, spherical: bool) -> Kind {
        let matrix = sequence_matrix(&angles_deg, &axes);
        Kind::RotationSequence3D { angles_deg, axes, matrix, spherical }
    }

    pub fn rotate_native2celestial(lon: f64, lat: f64, lon_pole: f64) -> Kind {
        Kind::RotateNative2Celestial { lon, lat, lon_pole, matrix: rotate3d_matrix(lon, lat, lon_pole, true) }
    }

    pub fn rotate_celestial2native(lon: f64, lat: f64, lon_pole: f64) -> Kind {
        Kind::RotateCelestial2Native { lon, lat, lon_pole, matrix: rotate3d_matrix(lon, lat, lon_pole, false) }
    }

    fn lanes(&self) -> (usize, usize) {
        match self {
            Kind::Compose(a, b) => (a.n_inputs, b.n_outputs),
            Kind::Concatenate(a, b) => (a.n_inputs + b.n_inputs, a.n_outputs + b.n_outputs),
            Kind::Arithmetic(_, a, _) => (a.n_inputs, a.n_outputs),
            Kind::Constant { n_inputs, .. } => (*n_inputs, 1),
            Kind::Shift(_) | Kind::Scale(_) | Kind::Poly1D { .. } => (1, 1),
            Kind::Poly2D { .. } => (2, 1),
            Kind::Mapping { mapping, n_inputs } => (*n_inputs, mapping.len()),
            Kind::Identity(n) => (*n, *n),
            Kind::Affine2D { .. } | Kind::Rotation2D { .. } => (2, 2),
            Kind::RotationSequence3D { spherical, .. } => {
                if *spherical {
                    (2, 2)
                } else {
                    (3, 3)
                }
            }
            Kind::SphericalToCartesian { .. } => (2, 3),
            Kind::CartesianToSpherical { .. } => (3, 2),
            Kind::Pix2SkyTan | Kind::Sky2PixTan => (2, 2),
            Kind::RotateNative2Celestial { .. } | Kind::RotateCelestial2Native { .. } => (2, 2),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    pub stored_inverse: Option<Box<Node>>,
    pub stored_inverse_error: Option<String>,
    pub tag: String,
    pub name: Option<String>,
    pub path: String,
    pub n_inputs: usize,
    pub n_outputs: usize,
}

impl Node {
    pub fn new(kind: Kind, tag: &str, path: &str) -> Node {
        let (n_inputs, n_outputs) = kind.lanes();
        Node {
            kind,
            stored_inverse: None,
            stored_inverse_error: None,
            tag: tag.to_string(),
            name: None,
            path: path.to_string(),
            n_inputs,
            n_outputs,
        }
    }

    pub fn eval(&self, input: Vals) -> Vals {
        let [x, y, z, _] = input.v;
        match &self.kind {
            Kind::Compose(a, b) => b.eval(a.eval(input)),
            Kind::Concatenate(a, b) => {
                let k = a.n_inputs.min(input.n);
                let left = a.eval(Vals::from_slice(&input.v[..k]));
                let right = b.eval(Vals::from_slice(&input.v[k..input.n]));
                left.concat(right)
            }
            Kind::Arithmetic(op, a, b) => {
                let left = a.eval(input);
                let right = b.eval(input);
                let mut v = [f64::NAN; MAX_LANES];
                for (slot, (l, r)) in v.iter_mut().zip(left.as_slice().iter().zip(right.as_slice())) {
                    *slot = op.apply(*l, *r);
                }
                Vals { v, n: left.n }
            }
            Kind::Constant { value, .. } => Vals::one(*value),
            Kind::Shift(offset) => Vals::one(x + offset),
            Kind::Scale(factor) => Vals::one(x * factor),
            Kind::Poly1D { coeffs, domain, window } => Vals::one(horner(coeffs, map_domain(x, *domain, *window))),
            Kind::Poly2D { degree, coeffs, domain, window } => {
                let xm = map_domain(x, domain.map(|d| d[0]), window.map(|w| w[0]));
                let ym = map_domain(y, domain.map(|d| d[1]), window.map(|w| w[1]));
                Vals::one(poly2d(*degree, coeffs, xm, ym))
            }
            Kind::Mapping { mapping, .. } => {
                let mut v = [f64::NAN; MAX_LANES];
                let n = mapping.len().min(MAX_LANES);
                for (slot, m) in v.iter_mut().zip(mapping) {
                    *slot = input.v.get(*m).copied().unwrap_or(f64::NAN);
                }
                Vals { v, n }
            }
            Kind::Identity(n) => Vals::from_slice(&input.v[..(*n).min(MAX_LANES)]),
            Kind::Affine2D { matrix, translation } => Vals::pair(
                matrix[0][0] * x + matrix[0][1] * y + translation[0],
                matrix[1][0] * x + matrix[1][1] * y + translation[1],
            ),
            Kind::Rotation2D { angle_deg } => {
                let (s, c) = angle_deg.to_radians().sin_cos();
                Vals::pair(x * c - y * s, x * s + y * c)
            }
            Kind::RotationSequence3D { matrix, spherical, .. } => {
                if *spherical {
                    let [lon, lat] = cartesian_to_spherical_astropy(matvec(matrix, spherical_to_cartesian(x, y)));
                    Vals::pair(lon, lat)
                } else {
                    let v = matvec(matrix, [x, y, z]);
                    Vals::triple(v[0], v[1], v[2])
                }
            }
            Kind::SphericalToCartesian { .. } => {
                let v = spherical_to_cartesian(x, y);
                Vals::triple(v[0], v[1], v[2])
            }
            Kind::CartesianToSpherical { wrap_lon_at } => {
                let [lon, lat] = cartesian_to_spherical_gwcs([x, y, z], *wrap_lon_at);
                Vals::pair(lon, lat)
            }
            Kind::Pix2SkyTan => {
                let r = x.hypot(y);
                let phi = if r == 0.0 { 0.0 } else { x.atan2(-y) * DEG_PER_RAD };
                let theta = DEG_PER_RAD.atan2(r) * DEG_PER_RAD;
                Vals::pair(phi, theta)
            }
            Kind::Sky2PixTan => {
                if !(y > 0.0) {
                    return Vals::pair(f64::NAN, f64::NAN);
                }
                let (sin_theta, cos_theta) = y.to_radians().sin_cos();
                let r = DEG_PER_RAD * cos_theta / sin_theta;
                let (sin_phi, cos_phi) = x.to_radians().sin_cos();
                Vals::pair(r * sin_phi, -r * cos_phi)
            }
            Kind::RotateNative2Celestial { matrix, .. } | Kind::RotateCelestial2Native { matrix, .. } => {
                let [mut alpha, delta] = cartesian_to_spherical_astropy(matvec(matrix, spherical_to_cartesian(x, y)));
                if alpha < 0.0 {
                    alpha += 360.0;
                }
                Vals::pair(alpha, delta)
            }
        }
    }

    pub fn inverse(&self) -> Result<Node, GwcsError> {
        if let Some(inv) = &self.stored_inverse {
            return Ok((**inv).clone());
        }
        match (self.derived_inverse(), &self.stored_inverse_error) {
            (Err(GwcsError::NoInverse { tag, path }), Some(err)) if path == self.path => {
                Err(GwcsError::NoInverse { tag, path: format!("{path} (stored inverse: {err})") })
            }
            (other, _) => other,
        }
    }

    pub fn derived_inverse(&self) -> Result<Node, GwcsError> {
        let kind = match &self.kind {
            Kind::Compose(a, b) => {
                let ia = a.inverse()?;
                let ib = b.inverse()?;
                if ib.n_outputs != ia.n_inputs {
                    return Err(GwcsError::Shape {
                        path: self.path.clone(),
                        reason: format!(
                            "the inverse of '{}' produces {} lanes but the inverse of '{}' takes {}",
                            b.tag, ib.n_outputs, a.tag, ia.n_inputs
                        ),
                    });
                }
                Kind::Compose(Box::new(ib), Box::new(ia))
            }
            Kind::Concatenate(a, b) => Kind::Concatenate(Box::new(a.inverse()?), Box::new(b.inverse()?)),
            Kind::Arithmetic(..) | Kind::Constant { .. } => return Err(self.no_inverse()),
            Kind::Shift(offset) => Kind::Shift(-offset),
            Kind::Scale(factor) => Kind::Scale(1.0 / factor),
            Kind::Poly1D { .. } | Kind::Poly2D { .. } => return Err(self.no_inverse()),
            Kind::Mapping { mapping, n_inputs } => {
                let mut inverse = Vec::with_capacity(*n_inputs);
                for i in 0..*n_inputs {
                    match mapping.iter().position(|m| *m == i) {
                        Some(p) => inverse.push(p),
                        None => return Err(self.no_inverse()),
                    }
                }
                Kind::Mapping { mapping: inverse, n_inputs: mapping.len() }
            }
            Kind::Identity(n) => Kind::Identity(*n),
            Kind::Affine2D { matrix, translation } => affine_inverse(matrix, translation).ok_or_else(|| self.no_inverse())?,
            Kind::Rotation2D { angle_deg } => Kind::Rotation2D { angle_deg: -angle_deg },
            Kind::RotationSequence3D { angles_deg, axes, spherical, .. } => Kind::rotation_sequence(
                angles_deg.iter().rev().map(|a| -a).collect(),
                axes.iter().rev().copied().collect(),
                *spherical,
            ),
            Kind::SphericalToCartesian { wrap_lon_at } => Kind::CartesianToSpherical { wrap_lon_at: *wrap_lon_at },
            Kind::CartesianToSpherical { wrap_lon_at } => Kind::SphericalToCartesian { wrap_lon_at: *wrap_lon_at },
            Kind::Pix2SkyTan => Kind::Sky2PixTan,
            Kind::Sky2PixTan => Kind::Pix2SkyTan,
            Kind::RotateNative2Celestial { lon, lat, lon_pole, .. } => Kind::rotate_celestial2native(*lon, *lat, *lon_pole),
            Kind::RotateCelestial2Native { lon, lat, lon_pole, .. } => Kind::rotate_native2celestial(*lon, *lat, *lon_pole),
        };
        Ok(self.derived(kind))
    }

    fn derived(&self, kind: Kind) -> Node {
        Node::new(kind, &self.tag, &self.path)
    }

    fn no_inverse(&self) -> GwcsError {
        GwcsError::NoInverse { tag: self.tag.clone(), path: self.path.clone() }
    }

    pub fn count_stored_inverses(&self) -> usize {
        let own = usize::from(self.stored_inverse.is_some());
        let children = match &self.kind {
            Kind::Compose(a, b) | Kind::Concatenate(a, b) | Kind::Arithmetic(_, a, b) => {
                a.count_stored_inverses() + b.count_stored_inverses()
            }
            _ => 0,
        };
        own + children
    }
}

pub fn passive_rotation(angle_deg: f64, axis: Axis) -> [[f64; 3]; 3] {
    let (s, c) = angle_deg.to_radians().sin_cos();
    let i = axis.index();
    let a1 = (i + 1) % 3;
    let a2 = (i + 2) % 3;
    let mut m = [[0.0; 3]; 3];
    m[i][i] = 1.0;
    m[a1][a1] = c;
    m[a1][a2] = s;
    m[a2][a1] = -s;
    m[a2][a2] = c;
    m
}

pub fn sequence_matrix(angles_deg: &[f64], axes: &[Axis]) -> [[f64; 3]; 3] {
    let mut m = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for (angle, axis) in angles_deg.iter().zip(axes) {
        m = matmul(&passive_rotation(*angle, *axis), &m);
    }
    m
}

pub fn rotate3d_matrix(lon: f64, lat: f64, lon_pole: f64, native2celestial: bool) -> [[f64; 3]; 3] {
    let angles = if native2celestial {
        [lon_pole - 90.0, -(90.0 - lat), -(90.0 + lon)]
    } else {
        [90.0 + lon, 90.0 - lat, -(lon_pole - 90.0)]
    };
    sequence_matrix(&angles, &[Axis::Z, Axis::X, Axis::Z])
}

fn matmul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

fn matvec(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub fn spherical_to_cartesian(lon_deg: f64, lat_deg: f64) -> [f64; 3] {
    let (sin_lon, cos_lon) = lon_deg.to_radians().sin_cos();
    let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
    [cos_lat * cos_lon, cos_lat * sin_lon, sin_lat]
}

fn cartesian_to_spherical_astropy(v: [f64; 3]) -> [f64; 2] {
    let h = v[0].hypot(v[1]);
    [v[1].atan2(v[0]) * DEG_PER_RAD, v[2].atan2(h) * DEG_PER_RAD]
}

fn cartesian_to_spherical_gwcs(v: [f64; 3], wrap_lon_at: u16) -> [f64; 2] {
    let h = v[0].hypot(v[1]);
    let lat = v[2].atan2(h) * DEG_PER_RAD;
    let mut lon = if h == 0.0 { 0.0 } else { v[1].atan2(v[0]) * DEG_PER_RAD };
    if wrap_lon_at != 180 && lon.is_finite() {
        lon = lon.rem_euclid(360.0);
    }
    [lon, lat]
}

fn map_domain(x: f64, domain: Option<[f64; 2]>, window: Option<[f64; 2]>) -> f64 {
    let Some([d0, d1]) = domain.or(window.map(|_| [-1.0, 1.0])) else { return x };
    let [w0, w1] = window.unwrap_or([-1.0, 1.0]);
    let scl = (w1 - w0) / (d1 - d0);
    let off = (w0 * d1 - w1 * d0) / (d1 - d0);
    off + scl * x
}

fn horner(coeffs: &[f64], x: f64) -> f64 {
    coeffs.iter().rev().fold(0.0, |acc, c| acc * x + c)
}

fn poly2d(degree: usize, coeffs: &[f64], x: f64, y: f64) -> f64 {
    let width = degree + 1;
    let mut result = 0.0;
    for i in (0..=degree).rev() {
        let row = &coeffs[i * width..i * width + (degree + 1 - i)];
        result = result * x + horner(row, y);
    }
    result
}

fn affine_inverse(matrix: &[[f64; 2]; 2], translation: &[f64; 2]) -> Option<Kind> {
    let det = matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0];
    if !(det.abs() >= AFFINE_SINGULAR_DET) {
        return None;
    }
    let inv = [[matrix[1][1] / det, -matrix[0][1] / det], [-matrix[1][0] / det, matrix[0][0] / det]];
    let t = [
        -(inv[0][0] * translation[0] + inv[0][1] * translation[1]),
        -(inv[1][0] * translation[0] + inv[1][1] * translation[1]),
    ];
    Some(Kind::Affine2D { matrix: inv, translation: t })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    const TOL: f64 = 1e-12;

    fn run_cases(name: &str, forward_tol: f64, inverse_tol: f64) {
        let node = fixture_node(name);
        for case in expected_cases(name) {
            let input = json_f64s(&case["input"]);
            let out = node.eval(Vals::from_slice(&input));
            assert_close_slice(out.as_slice(), &json_f64s(&case["output"]), forward_tol, &format!("{name} forward {input:?}"));
            if let Some(inv_in) = case.get("inverse_input") {
                let inv = node.inverse().unwrap_or_else(|e| panic!("{name}: {e:#}"));
                let got = inv.eval(Vals::from_slice(&json_f64s(inv_in)));
                assert_close_slice(got.as_slice(), &json_f64s(&case["inverse_output"]), inverse_tol, &format!("{name} inverse {inv_in}"));
            }
        }
    }

    #[test]
    fn shift_and_scale_match_expected() {
        run_cases("shift.asdf", TOL, TOL);
        run_cases("scale.asdf", TOL, TOL);
        let shift = fixture_node("shift.asdf");
        assert_close(shift.eval(Vals::one(1023.5)).v[0], 1020.724, TOL, "shift forward");
        assert_close(shift.inverse().unwrap().eval(Vals::one(1020.724)).v[0], 1023.5, TOL, "shift inverse");
        assert_close(fixture_node("scale.asdf").eval(Vals::one(3600.0)).v[0], 1.0, TOL, "scale forward");
    }

    #[test]
    fn concatenate_splits_lanes() {
        run_cases("concat_shift_shift.asdf", TOL, TOL);
        run_cases("dva_correction.asdf", TOL, 1e-11);
        let dva = fixture_node("dva_correction.asdf");
        assert_eq!((dva.n_inputs, dva.n_outputs), (2, 2));
        let inv = dva.derived_inverse().unwrap();
        let back = inv.eval(Vals::pair(0.008592537736433575, -0.03759078209925696));
        assert_close_slice(back.as_slice(), &[0.0, 0.0], 1e-12, "dva derived inverse");
    }

    #[test]
    fn poly2d_pair_with_stored_inverse() {
        for name in ["poly2d_distortion_pair_with_inverse.asdf", "poly2d_distortion_pair_inline.asdf", "poly2d_linear_pair_with_inverse.asdf"] {
            run_cases(name, TOL, TOL);
            let node = fixture_node(name);
            assert!(node.stored_inverse.is_some(), "{name} has a stored inverse");
        }
        let node = fixture_node("poly2d_distortion_pair_with_inverse.asdf");
        let out = node.eval(Vals::pair(500.25, -250.75));
        assert_close_slice(out.as_slice(), &[15.600976847810873, -7.838021106967654], TOL, "distortion forward");
        let back = node.inverse().unwrap().eval(out);
        assert_close_slice(back.as_slice(), &[500.24998889669905, -250.75001373100406], TOL, "stored inverse is approximate");
    }

    #[test]
    fn poly1d_with_stored_inverse() {
        run_cases("poly1d_with_inverse.asdf", TOL, 1e-12);
        let node = fixture_node("poly1d_with_inverse.asdf");
        match &node.kind {
            Kind::Poly1D { coeffs, .. } => assert_close_slice(coeffs, &[-12.7875, 0.025], 1e-12, "coefficients"),
            other => panic!("expected Poly1D, got {other:?}"),
        }
        let inv = node.inverse().unwrap();
        assert_close(inv.eval(Vals::one(0.0)).v[0], 511.5, TOL, "inverse of 0");
        assert_close(inv.eval(Vals::one(-11.7875)).v[0], 40.0, 1e-9, "inverse of -11.7875");
    }

    #[test]
    fn poly2d_concat4_takes_four_inputs() {
        let node = fixture_node("poly2d_concat4_with_inverse.asdf");
        assert_eq!((node.n_inputs, node.n_outputs), (4, 2));
        run_cases("poly2d_concat4_with_inverse.asdf", TOL, TOL);
        let inv = node.inverse().unwrap();
        assert_eq!(inv.n_inputs, 4);
    }

    #[test]
    fn mapping_and_identity_stored_inverses_change_lane_count() {
        let identity = fixture_node("identity_inverse_mapping.asdf");
        let inv = identity.inverse().unwrap().eval(Vals::pair(1.5, -2.5));
        assert_eq!(inv.n, 4);
        assert_close_slice(inv.as_slice(), &[1.5, -2.5, 1.5, -2.5], 0.0, "identity stored inverse");
        run_cases("identity_inverse_mapping.asdf", 0.0, 0.0);

        let mapping = fixture_node("mapping_0101_inverse_identity.asdf");
        run_cases("mapping_0101_inverse_identity.asdf", 0.0, 0.0);
        assert_eq!(mapping.inverse().unwrap().eval(Vals::from_slice(&[1.5, -2.5, 1.5, -2.5])).n, 2);
        let derived = mapping.derived_inverse().unwrap();
        assert_eq!(derived.n_inputs, 4, "mappings.py:118-122: the inverse of Mapping([0,1,0,1]) takes 4 inputs");
        assert_eq!(derived.n_outputs, 2);
        let back = derived.eval(Vals::from_slice(&[7.0, 8.0, 7.0, 8.0]));
        assert_close_slice(back.as_slice(), &[7.0, 8.0], 0.0, "derived mapping inverse");

        run_cases("mapping_swap.asdf", 0.0, 0.0);
        let swap = fixture_node("mapping_swap.asdf");
        assert_close_slice(swap.eval(Vals::pair(1.0, 2.0)).as_slice(), &[2.0, 1.0], 0.0, "swap");
        assert_close_slice(swap.derived_inverse().unwrap().eval(Vals::pair(2.0, 1.0)).as_slice(), &[1.0, 2.0], 0.0, "swap inverse");

        let drop = fixture_node("mapping_drop_n_inputs.asdf");
        assert_eq!((drop.n_inputs, drop.n_outputs), (2, 1));
        run_cases("mapping_drop_n_inputs.asdf", 0.0, 0.0);
        match drop.inverse() {
            Err(GwcsError::NoInverse { tag, .. }) => assert_eq!(tag, "remap_axes"),
            other => panic!("expected NoInverse, got {other:?}"),
        }
    }

    #[test]
    fn v23tosky_round_trips() {
        run_cases("v23tosky_jwst.asdf", TOL, 1e-9);
        let node = fixture_node("v23tosky_jwst.asdf");
        let out = node.eval(Vals::pair(153.14120464614305, -559.502283189643));
        assert_close_slice(out.as_slice(), &[274.72530887834534, -13.875908119051436], TOL, "v23tosky forward");
        let back = node.derived_inverse().unwrap().eval(out);
        assert_close_slice(back.as_slice(), &[153.14120464613708, -559.5022831896379], 1e-9, "v23tosky inverse");
        let centre = node.eval(Vals::pair(0.0, 0.0));
        assert_close_slice(centre.as_slice(), &[274.8870583990748, -13.839823482758614], TOL, "v23tosky origin");
    }

    #[test]
    fn spherical_cartesian_wraps_as_gwcs() {
        run_cases("spherical_to_cartesian_wrap180.asdf", TOL, TOL);
        run_cases("cartesian_to_spherical_wrap360.asdf", TOL, TOL);
        let c2s360 = Node::new(Kind::CartesianToSpherical { wrap_lon_at: 360 }, "spherical_cartesian", "synthetic");
        assert_close_slice(c2s360.eval(Vals::triple(-1.0, 0.0, 0.0)).as_slice(), &[180.0, 0.0], TOL, "wrap360 at -x");
        let near = c2s360.eval(Vals::triple(1.0, -1e-9, 0.0));
        assert_close(near.v[0], 360.0 - 5.729577951308232e-8, 1e-12, "wrap360 just below 360");
        assert!(near.v[0] < 360.0 && near.v[0] > 359.9999999);
        assert_close_slice(c2s360.eval(Vals::triple(-0.0, 0.0, 1.0)).as_slice(), &[0.0, 90.0], 0.0, "pole lon 0 where a plain atan2(+0, -0) gives 180");
        let c2s180 = Node::new(Kind::CartesianToSpherical { wrap_lon_at: 180 }, "spherical_cartesian", "synthetic");
        assert_close_slice(c2s180.eval(Vals::triple(-0.0, -0.0, -1.0)).as_slice(), &[0.0, -90.0], 0.0, "south pole lon 0 where a plain atan2(-0, -0) gives -180");
        assert_close(c2s180.eval(Vals::triple(1.0, -1e-9, 0.0)).v[0], -5.729577951308232e-8, 1e-20, "wrap180 stays negative");
        let edge = c2s360.eval(Vals::triple(1.0, -1e-17, 0.0));
        assert_eq!(edge.v[0], 360.0, "np.mod(-1e-17, 360) == 360.0 is reproduced");
        let nan = c2s360.eval(Vals::triple(f64::NAN, 0.0, 0.0));
        assert!(nan.v[0].is_nan() && nan.v[1].is_nan());
    }

    #[test]
    fn rotation_sequence_cartesian_and_spherical() {
        run_cases("rotation_sequence_3d_cartesian.asdf", TOL, TOL);
        run_cases("rotation_sequence_spherical_miri.asdf", TOL, 1e-9);
        let axes = vec![Axis::Z, Axis::Y, Axis::X, Axis::Y, Axis::Z];
        let seq = Node::new(Kind::rotation_sequence(vec![10.0, 0.0, 0.0, 0.0, 0.0], axes.clone(), true), "rotate_sequence_3d", "synthetic");
        assert_close_slice(seq.eval(Vals::pair(5.0, 10.0)).as_slice(), &[-5.0, 10.0], 1e-12, "no wrap (5, 10)");
        assert_close_slice(seq.eval(Vals::pair(-30.0, 10.0)).as_slice(), &[-40.0, 10.0], 1e-12, "no wrap (-30, 10)");
        let zero = Node::new(Kind::rotation_sequence(vec![0.0; 5], axes, true), "rotate_sequence_3d", "synthetic");
        assert_close_slice(zero.eval(Vals::pair(-30.0, 10.0)).as_slice(), &[-30.0, 10.0], 1e-12, "identity sequence keeps -30");
        let inv = seq.derived_inverse().unwrap();
        match &inv.kind {
            Kind::RotationSequence3D { angles_deg, axes, .. } => {
                assert_close_slice(angles_deg, &[0.0, 0.0, 0.0, 0.0, -10.0], 0.0, "reversed negated angles");
                assert_eq!(axes, &[Axis::Z, Axis::Y, Axis::X, Axis::Y, Axis::Z]);
            }
            other => panic!("unexpected inverse kind {other:?}"),
        }
        assert_close_slice(inv.eval(Vals::pair(-5.0, 10.0)).as_slice(), &[5.0, 10.0], 1e-12, "sequence inverse");
    }

    #[test]
    fn rotation2d_affine_gnomonic_rotate3d() {
        run_cases("rotation2d.asdf", TOL, TOL);
        run_cases("affine2d.asdf", TOL, TOL);
        run_cases("pix2sky_gnomonic.asdf", TOL, TOL);
        run_cases("rotate_native2celestial.asdf", TOL, 1e-9);
        let tan = fixture_node("pix2sky_gnomonic.asdf");
        assert_eq!(tan.eval(Vals::pair(0.0, 0.0)).as_slice(), &[0.0, 90.0]);
        assert_close_slice(tan.eval(Vals::pair(0.01, -0.02)).as_slice(), &[26.56505117707799, 89.97763932136026], TOL, "tan");
        let n2c = fixture_node("rotate_native2celestial.asdf");
        assert_close_slice(n2c.eval(Vals::pair(10.0, 80.0)).as_slice(), &[32.11296235752141, 35.130581725170536], TOL, "n2c");
        let c2n = n2c.inverse().unwrap();
        assert_close_slice(c2n.eval(Vals::pair(30.009998766018143, 44.992928495927615)).as_slice(), &[45.00000000000762, 89.99], 1e-9, "c2n");
        assert_close_slice(c2n.eval(Vals::pair(29.97999506224482, 45.014140389719955)).as_slice(), &[224.9999999999793, 89.98], 1e-9, "c2n wraps a negative longitude by +360");
        let pole = n2c.eval(Vals::pair(0.0, 90.0));
        assert_close_slice(pole.as_slice(), &[30.0, 45.0], 1e-12, "native pole");
        let sky2pix = Node::new(Kind::Sky2PixTan, "gnomonic", "synthetic");
        for theta in [-10.0, 0.0] {
            let out = sky2pix.eval(Vals::pair(30.0, theta));
            assert!(out.v[0].is_nan() && out.v[1].is_nan(), "theta {theta} gives NaN, got {:?}", out.as_slice());
        }
        let at_pole = sky2pix.eval(Vals::pair(30.0, 90.0));
        assert!(at_pole.v[0].abs() < 1e-12 && at_pole.v[1].abs() < 1e-12, "{:?}", at_pole.as_slice());
        let synthetic_c2n = Node::new(Kind::rotate_celestial2native(30.0, 45.0, 180.0), "rotate3d", "synthetic");
        assert_close_slice(synthetic_c2n.eval(Vals::pair(25.0, 44.0)).as_slice(), &[283.9012881374465, 86.29690254585657], 1e-9, "RotateCelestial2Native(30, 45, 180)(25, 44)");
        let singular = Node::new(Kind::Affine2D { matrix: [[1.0, 2.0], [2.0, 4.0]], translation: [0.0, 0.0] }, "affine", "synthetic");
        assert!(matches!(singular.inverse(), Err(GwcsError::NoInverse { .. })));
        let affine = Node::new(Kind::Affine2D { matrix: [[2.0, 1.0], [0.5, 3.0]], translation: [4.0, -1.0] }, "affine", "synthetic");
        let out = affine.eval(Vals::pair(1.5, -2.0));
        assert_close_slice(out.as_slice(), &[5.0, -6.25], 1e-12, "affine forward");
        let back = affine.inverse().unwrap().eval(out);
        assert_close_slice(back.as_slice(), &[1.5, -2.0], 1e-12, "affine inverse");
    }

    #[test]
    fn a_parent_stored_inverse_error_is_not_attached_to_a_child_failure() {
        let poly = Node::new(Kind::Poly1D { coeffs: vec![1.0, 2.0], domain: None, window: None }, "polynomial", "s/forward[0]");
        let shift = Node::new(Kind::Shift(1.0), "shift", "s/forward[1]");
        let mut parent = Node::new(Kind::Compose(Box::new(poly.clone()), Box::new(shift)), "compose", "s");
        parent.stored_inverse_error = Some("gWCS transform 'tabular' at s/inverse is not supported by the evaluator".into());
        let Err(GwcsError::NoInverse { tag, path }) = parent.inverse() else { panic!("expected NoInverse") };
        assert_eq!((tag.as_str(), path.as_str()), ("polynomial", "s/forward[0]"));
        assert_eq!(format!("{:#}", parent.inverse().unwrap_err()), "gWCS transform 'polynomial' at s/forward[0] has no inverse (none stored, none derivable)");
        assert!(parent.stored_inverse_error.as_deref().unwrap().contains("'tabular'"));
        let mut own = poly;
        own.stored_inverse_error = Some("gWCS transform 'tabular' at s/forward[0]/inverse is not supported by the evaluator".into());
        let text = format!("{:#}", own.inverse().unwrap_err());
        assert_eq!(text, "gWCS transform 'polynomial' at s/forward[0] (stored inverse: gWCS transform 'tabular' at s/forward[0]/inverse is not supported by the evaluator) has no inverse (none stored, none derivable)");
    }

    #[test]
    fn tag_versions_do_not_matter() {
        for (new, old) in [
            ("rotation2d.asdf", "rotation2d_std150.asdf"),
            ("pix2sky_gnomonic.asdf", "pix2sky_gnomonic_std150.asdf"),
            ("rotate_native2celestial.asdf", "rotate_native2celestial_std150.asdf"),
            ("affine2d.asdf", "affine2d_std150.asdf"),
        ] {
            let a = fixture_node(new);
            let b = fixture_node(old);
            assert_eq!(a.tag, b.tag, "{new} vs {old}");
            for case in expected_cases(old) {
                let input = Vals::from_slice(&json_f64s(&case["input"]));
                assert_close_slice(a.eval(input).as_slice(), b.eval(input).as_slice(), 0.0, &format!("{new} vs {old}"));
            }
            run_cases(old, TOL, 1e-9);
        }
    }

    #[test]
    fn nan_inputs_propagate() {
        let v23 = fixture_node("v23tosky_jwst.asdf");
        let out = v23.eval(Vals::pair(f64::NAN, 1.0));
        assert!(out.v[0].is_nan() && out.v[1].is_nan());
        let poly = fixture_node("poly2d_distortion_pair_with_inverse.asdf");
        let out = poly.eval(Vals::pair(1.0, f64::INFINITY));
        assert!(out.v[0].is_nan() || out.v[0].is_infinite());
    }

    #[test]
    fn passive_rotation_matches_astropy_convention() {
        let z = passive_rotation(90.0, Axis::Z);
        assert_close_slice(&z[0], &[0.0, 1.0, 0.0], 1e-15, "row 0");
        assert_close_slice(&z[1], &[-1.0, 0.0, 0.0], 1e-15, "row 1");
        assert_close_slice(&z[2], &[0.0, 0.0, 1.0], 0.0, "row 2");
        let seq = sequence_matrix(&[30.0, 40.0], &[Axis::Z, Axis::X]);
        let direct = matmul(&passive_rotation(40.0, Axis::X), &passive_rotation(30.0, Axis::Z));
        for i in 0..3 {
            assert_close_slice(&seq[i], &direct[i], 0.0, "R2 R1 order");
        }
    }

    #[test]
    fn a_window_without_a_domain_maps_from_the_default_domain() {
        assert_eq!(map_domain(0.25, None, None), 0.25, "no domain and no window: the input is used as is");
        assert_eq!(map_domain(0.5, Some([0.0, 2.0]), None), -0.5, "a domain alone maps onto the default window [-1, 1]");
        assert_eq!(map_domain(0.5, Some([0.0, 2.0]), Some([0.0, 10.0])), 2.5, "domain and window");
        assert_eq!(map_domain(0.5, None, Some([0.0, 10.0])), 7.5, "astropy keeps domain (-1, 1) when only a window is given");
    }

    fn arith(op: ArithOp, a: Node, b: Node) -> Node {
        Node::new(Kind::Arithmetic(op, Box::new(a), Box::new(b)), op.tag(), "synthetic")
    }

    fn constant(value: f64, n_inputs: usize) -> Node {
        Node::new(Kind::Constant { value, n_inputs }, "constant", "synthetic")
    }

    #[test]
    fn arithmetic_operators_combine_the_operand_outputs_lane_by_lane() {
        let shift = Node::new(Kind::Shift(1.5), "shift", "s");
        let scale = Node::new(Kind::Scale(2.0), "scale", "s");
        for (op, exp) in [
            (ArithOp::Add, 10.5),
            (ArithOp::Subtract, -1.5),
            (ArithOp::Multiply, 27.0),
            (ArithOp::Divide, 0.75),
            (ArithOp::Power, 8303.765625),
        ] {
            let node = arith(op, shift.clone(), scale.clone());
            assert_eq!((node.n_inputs, node.n_outputs), (1, 1), "{op:?}");
            assert_close(node.eval(Vals::one(3.0)).v[0], exp, 1e-9, &format!("{op:?} of Shift(1.5) and Scale(2) at 3"));
            match node.derived_inverse() {
                Err(GwcsError::NoInverse { tag, .. }) => assert_eq!(tag, op.tag()),
                other => panic!("{op:?}: astropy has no analytic inverse for arithmetic operators, got {other:?}"),
            }
        }
        let xyz = Node::new(Kind::Mapping { mapping: vec![0, 1, 2], n_inputs: 3 }, "remap_axes", "s");
        let xxx = Node::new(Kind::Mapping { mapping: vec![0, 0, 0], n_inputs: 3 }, "remap_axes", "s");
        let divide = arith(ArithOp::Divide, xyz, xxx);
        assert_eq!((divide.n_inputs, divide.n_outputs), (3, 3));
        assert_eq!(divide.eval(Vals::triple(2.0, 4.0, -6.0)).as_slice(), &[1.0, 2.0, -3.0]);
        let by_zero = divide.eval(Vals::triple(0.0, 1.0, -1.0));
        assert!(by_zero.v[0].is_nan(), "0/0 is NaN as in numpy: {:?}", by_zero.as_slice());
        assert_eq!(&by_zero.as_slice()[1..], &[f64::INFINITY, f64::NEG_INFINITY]);
        let cube_root = arith(ArithOp::Power, Node::new(Kind::Identity(1), "identity", "s"), constant(1.0 / 3.0, 1));
        assert!(cube_root.eval(Vals::one(-8.0)).v[0].is_nan(), "numpy power of a negative base to a fractional exponent is NaN");
        assert_close(cube_root.eval(Vals::one(8.0)).v[0], 2.0, 1e-15, "8 ** (1/3)");
    }

    #[test]
    fn constants_ignore_their_inputs_and_have_no_inverse() {
        let one_d = constant(299.7, 1);
        assert_eq!((one_d.n_inputs, one_d.n_outputs), (1, 1));
        for x in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(one_d.eval(Vals::one(x)).as_slice(), &[299.7], "Const1D fills the amplitude whatever the input ({x})");
        }
        let two_d = constant(-0.5, 2);
        assert_eq!((two_d.n_inputs, two_d.n_outputs), (2, 1));
        assert_eq!(two_d.eval(Vals::pair(1.0, f64::NAN)).as_slice(), &[-0.5]);
        for node in [one_d, two_d] {
            assert!(matches!(node.derived_inverse(), Err(GwcsError::NoInverse { ref tag, .. }) if tag == "constant"), "{:?}", node.derived_inverse());
        }
        let tan_to_cartesian = Node::new(
            Kind::Compose(
                Box::new(Node::new(Kind::Mapping { mapping: vec![0, 0, 1], n_inputs: 2 }, "remap_axes", "s")),
                Box::new(Node::new(
                    Kind::Concatenate(Box::new(constant(1.0, 1)), Box::new(Node::new(Kind::Identity(2), "identity", "s"))),
                    "concatenate",
                    "s",
                )),
            ),
            "compose",
            "s",
        );
        assert_eq!((tan_to_cartesian.n_inputs, tan_to_cartesian.n_outputs), (2, 3));
        assert_eq!(tan_to_cartesian.eval(Vals::pair(0.25, -0.5)).as_slice(), &[1.0, 0.25, -0.5], "Mapping((0,0,1)) | Const1D(1) & Identity(2)");
        let mut with_inverse = arith(ArithOp::Add, Node::new(Kind::Shift(1.0), "shift", "s"), Node::new(Kind::Scale(2.0), "scale", "s"));
        with_inverse.stored_inverse = Some(Box::new(Node::new(Kind::Shift(-1.0), "shift", "s")));
        let parent = arith(ArithOp::Multiply, with_inverse, constant(2.0, 1));
        assert_eq!(parent.count_stored_inverses(), 1, "stored inverses below an arithmetic node are counted");
    }

    #[test]
    fn arithmetic_and_constant_fixtures_match_astropy() {
        run_cases("const1d_with_inverse.asdf", 0.0, 0.0);
        run_cases("const1d_with_inverse_std150.asdf", 0.0, 0.0);
        let c1 = fixture_node("const1d_with_inverse.asdf");
        assert_eq!(c1.eval(Vals::one(f64::NAN)).as_slice(), &[299.7]);
        assert!(c1.stored_inverse.is_some(), "the LRS Const1D carries a stored Const1D inverse");
        run_cases("arithmetic_operators_1d.asdf", TOL, TOL);
        run_cases("arithmetic_add_with_inverse.asdf", TOL, TOL);
        run_cases("arithmetic_const2d.asdf", TOL, TOL);
        run_cases("arithmetic_two_outputs.asdf", TOL, TOL);
        run_cases("divide_cartesian_to_tan.asdf", TOL, TOL);
        run_cases("tp_correction_jwst.asdf", 1e-9, 1e-9);
        let ops = fixture_node("arithmetic_operators_1d.asdf");
        assert_eq!(ops.tag, "add");
        assert!(ops.eval(Vals::one(-4.0)).v[0].is_nan(), "(-4 + 3) ** 0.5 is NaN");
        assert!(matches!(ops.inverse(), Err(GwcsError::NoInverse { ref tag, .. }) if tag == "add"), "{:?}", ops.inverse());
        let lanes = fixture_node("arithmetic_two_outputs.asdf");
        assert_eq!((lanes.n_inputs, lanes.n_outputs), (2, 2));
        let tan = fixture_node("divide_cartesian_to_tan.asdf");
        assert_eq!(tan.name.as_deref(), Some("Cartesian 3D to TAN"));
        assert_eq!((tan.n_inputs, tan.n_outputs), (3, 2));
        assert!(matches!(tan.inverse(), Err(GwcsError::NoInverse { ref tag, .. }) if tag == "divide"), "{:?}", tan.inverse());
        let tp = fixture_node("tp_correction_jwst.asdf");
        assert_eq!(tp.name.as_deref(), Some("JWST tangent-plane linear correction. v1"));
        assert!(matches!(tp.derived_inverse(), Err(GwcsError::NoInverse { ref tag, .. }) if tag == "divide"), "{:?}", tp.derived_inverse());
        let stored = tp.inverse().unwrap();
        assert_eq!(stored.name.as_deref(), Some("Inverse JWST tangent-plane linear correction. v1"));
        let out = tp.eval(Vals::pair(120.66454051298493, -527.6382298732158));
        assert_close_slice(out.as_slice(), &[120.61638465078101, -527.5819370910275], 1e-9, "tweakreg correction of the nrca1 reference pixel");
    }
}
