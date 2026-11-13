use std::f64::consts::{PI, TAU};

pub const TRANSIT_MODEL_NAME: &str = "Mandel and Agol 2002, quadratic limb darkening, circular orbit";
pub const ECLIPSE_MODEL_NAME: &str = "eclipse of a uniform planet disc by the star, circular orbit";
pub const MAX_RADIUS_RATIO: f64 = 100.0;
pub const CARLSON_TOLERANCE: f64 = 2.5e-4;

const CARLSON_MAX_ITERATIONS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitShape {
    pub t0: f64,
    pub period: f64,
    pub rp: f64,
    pub a_rs: f64,
    pub b: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LimbDarkening {
    pub u1: f64,
    pub u2: f64,
}

fn carlson_rc1(e: f64) -> f64 {
    if e == 0.0 {
        return 1.0;
    }
    let root = e.abs().sqrt();
    if e > 0.0 {
        root.atan() / root
    } else {
        root.atanh() / root
    }
}

fn largest_offset(mean: f64, members: &[f64]) -> f64 {
    members.iter().map(|member| (member - mean).abs()).fold(0.0, f64::max)
}

fn fifth_order_series(e2: f64, e3: f64, e4: f64, e5: f64) -> f64 {
    1.0 - 3.0 * e2 / 14.0 + e3 / 6.0 + 9.0 * e2 * e2 / 88.0 - 3.0 * e4 / 22.0 - 9.0 * e2 * e3 / 52.0 + 3.0 * e5 / 26.0
}

pub fn carlson_rf(x: f64, y: f64, z: f64) -> f64 {
    let (mut x, mut y, mut z) = (x, y, z);
    for _ in 0..CARLSON_MAX_ITERATIONS {
        let mean = (x + y + z) / 3.0;
        if largest_offset(mean, &[x, y, z]) <= CARLSON_TOLERANCE * mean.abs() {
            break;
        }
        let (root_x, root_y, root_z) = (x.sqrt(), y.sqrt(), z.sqrt());
        let lambda = root_x * root_y + root_y * root_z + root_z * root_x;
        x = (x + lambda) / 4.0;
        y = (y + lambda) / 4.0;
        z = (z + lambda) / 4.0;
    }
    let mean = (x + y + z) / 3.0;
    let dx = 1.0 - x / mean;
    let dy = 1.0 - y / mean;
    let dz = -dx - dy;
    let e2 = dx * dy - dz * dz;
    let e3 = dx * dy * dz;
    (1.0 - e2 / 10.0 + e3 / 14.0 + e2 * e2 / 24.0 - 3.0 * e2 * e3 / 44.0) / mean.sqrt()
}

pub fn carlson_rd(x: f64, y: f64, z: f64) -> f64 {
    let (mut x, mut y, mut z) = (x, y, z);
    let mut sum = 0.0;
    let mut scale = 1.0;
    for _ in 0..CARLSON_MAX_ITERATIONS {
        let mean = (x + y + 3.0 * z) / 5.0;
        if largest_offset(mean, &[x, y, z]) <= CARLSON_TOLERANCE * mean.abs() {
            break;
        }
        let (root_x, root_y, root_z) = (x.sqrt(), y.sqrt(), z.sqrt());
        let lambda = root_x * root_y + root_y * root_z + root_z * root_x;
        sum += scale / (root_z * (z + lambda));
        scale /= 4.0;
        x = (x + lambda) / 4.0;
        y = (y + lambda) / 4.0;
        z = (z + lambda) / 4.0;
    }
    let mean = (x + y + 3.0 * z) / 5.0;
    let dx = (mean - x) / mean;
    let dy = (mean - y) / mean;
    let dz = -(dx + dy) / 3.0;
    let e2 = dx * dy - 6.0 * dz * dz;
    let e3 = (3.0 * dx * dy - 8.0 * dz * dz) * dz;
    let e4 = 3.0 * (dx * dy - dz * dz) * dz * dz;
    let e5 = dx * dy * dz * dz * dz;
    3.0 * sum + scale * fifth_order_series(e2, e3, e4, e5) / (mean * mean.sqrt())
}

pub fn carlson_rj(x: f64, y: f64, z: f64, p: f64) -> f64 {
    let delta = (p - x) * (p - y) * (p - z);
    let (mut x, mut y, mut z, mut p) = (x, y, z, p);
    let mut sum = 0.0;
    let mut scale = 1.0;
    for _ in 0..CARLSON_MAX_ITERATIONS {
        let mean = (x + y + z + 2.0 * p) / 5.0;
        if largest_offset(mean, &[x, y, z, p]) <= CARLSON_TOLERANCE * mean.abs() {
            break;
        }
        let (root_x, root_y, root_z, root_p) = (x.sqrt(), y.sqrt(), z.sqrt(), p.sqrt());
        let lambda = root_x * root_y + root_y * root_z + root_z * root_x;
        let d = (root_p + root_x) * (root_p + root_y) * (root_p + root_z);
        sum += scale / d * carlson_rc1(scale * scale * scale * delta / (d * d));
        scale /= 4.0;
        x = (x + lambda) / 4.0;
        y = (y + lambda) / 4.0;
        z = (z + lambda) / 4.0;
        p = (p + lambda) / 4.0;
    }
    let mean = (x + y + z + 2.0 * p) / 5.0;
    let dx = (mean - x) / mean;
    let dy = (mean - y) / mean;
    let dz = (mean - z) / mean;
    let dp = -(dx + dy + dz) / 2.0;
    let e2 = dx * dy + dx * dz + dy * dz - 3.0 * dp * dp;
    let e3 = dx * dy * dz + 2.0 * e2 * dp + 4.0 * dp * dp * dp;
    let e4 = (2.0 * dx * dy * dz + e2 * dp + 3.0 * dp * dp * dp) * dp;
    let e5 = dx * dy * dz * dp * dp;
    scale * fifth_order_series(e2, e3, e4, e5) / (mean * mean.sqrt()) + 6.0 * sum
}

fn ellip_k(mc: f64) -> f64 {
    carlson_rf(0.0, mc, 1.0)
}

fn ellip_e(mc: f64) -> f64 {
    carlson_rf(0.0, mc, 1.0) - (1.0 - mc) / 3.0 * carlson_rd(0.0, mc, 1.0)
}

fn limb_crossing_angles(p: f64, z: f64) -> (f64, f64, f64) {
    let kite = ((1.0 - z + p) * (1.0 + z - p) * (z + p - 1.0) * (z + p + 1.0)).max(0.0).sqrt();
    (kite, kite.atan2(p * p + z * z - 1.0), kite.atan2(1.0 - p * p + z * z))
}

fn mu_term_inside_the_disc(p: f64, z: f64) -> f64 {
    let near_gap = 1.0 - (z - p) * (z - p);
    let far_gap = 1.0 - (z + p) * (z + p);
    let q = p * p - z * z;
    let mc = far_gap / near_gap;
    let bracket = (1.0 - 5.0 * z * z + p * p + q * q) * ellip_k(mc) + near_gap * (z * z + 7.0 * p * p - 4.0) * ellip_e(mc)
        - q / near_gap * carlson_rj(0.0, mc, 1.0, 1.0 / near_gap);
    1.0 / 3.0 + 2.0 / (9.0 * PI * near_gap.sqrt()) * bracket
}

fn mu_term_at_inner_contact(p: f64) -> f64 {
    2.0 / (3.0 * PI) * (1.0 - 2.0 * p).acos() - 4.0 / (9.0 * PI) * (3.0 + 2.0 * p - 8.0 * p * p) * (p * (1.0 - p)).sqrt()
}

fn mu_term_across_the_limb(p: f64, z: f64) -> f64 {
    let near_sq = (z - p) * (z - p);
    let far_sq = (z + p) * (z + p);
    let q = p * p - z * z;
    let zp = z * p;
    let mc = (z + p - 1.0) * (z + p + 1.0) / (4.0 * zp);
    let k_factor = (1.0 - far_sq) * (2.0 * far_sq + near_sq - 3.0) - 3.0 * q * (far_sq - 2.0);
    let e_factor = 4.0 * zp * (z * z + 7.0 * p * p - 4.0);
    1.0 / 3.0 + (k_factor * ellip_k(mc) + e_factor * ellip_e(mc)) / (9.0 * PI * zp.sqrt())
        - q * carlson_rj(0.0, mc, 1.0, far_sq / (4.0 * zp)) / (36.0 * PI * zp * zp.sqrt())
}

pub fn uniform_overlap(p: f64, z: f64) -> f64 {
    if z >= 1.0 + p {
        return 0.0;
    }
    if z <= p - 1.0 {
        return 1.0;
    }
    if z <= 1.0 - p {
        return p * p;
    }
    let (kite, planet_angle, star_angle) = limb_crossing_angles(p, z);
    (p * p * planet_angle + star_angle - kite / 2.0) / PI
}

pub fn hidden_terms(p: f64, z: f64) -> (f64, f64, f64) {
    if z >= 1.0 + p {
        return (0.0, 0.0, 0.0);
    }
    if z <= p - 1.0 {
        return (1.0, 2.0 / 3.0, 0.5);
    }
    if z + p <= 1.0 {
        let mu_term = if z + p < 1.0 { mu_term_inside_the_disc(p, z) } else { mu_term_at_inner_contact(p) };
        return (p * p, mu_term, p * p * (p * p + 2.0 * z * z) / 2.0);
    }
    let (kite, planet_angle, star_angle) = limb_crossing_angles(p, z);
    let area = (p * p * planet_angle + star_angle - kite / 2.0) / PI;
    let r2_term = (star_angle + p * p * (p * p + 2.0 * z * z) * planet_angle - (1.0 + 5.0 * p * p + z * z) * kite / 4.0) / (2.0 * PI);
    (area, mu_term_across_the_limb(p, z), r2_term)
}

pub fn quadratic_flux(p: f64, z: f64, limb: &LimbDarkening) -> f64 {
    if z >= 1.0 + p {
        return 1.0;
    }
    let (area, mu_term, r2_term) = hidden_terms(p, z);
    let hidden = (1.0 - limb.u1 - 2.0 * limb.u2) * area + (limb.u1 + 2.0 * limb.u2) * mu_term + limb.u2 * r2_term;
    1.0 - hidden / (1.0 - limb.u1 / 3.0 - limb.u2 / 6.0)
}

pub fn sky_separation(time: f64, shape: &TransitShape) -> (f64, bool) {
    let cycles = (time - shape.t0) / shape.period;
    let (sin_phase, cos_phase) = (TAU * (cycles - cycles.round())).sin_cos();
    ((shape.a_rs * sin_phase).hypot(shape.b * cos_phase), cos_phase > 0.0)
}

pub fn transit_flux(time: f64, shape: &TransitShape, limb: &LimbDarkening) -> f64 {
    let (z, near) = sky_separation(time, shape);
    if !near || z >= 1.0 + shape.rp {
        return 1.0;
    }
    quadratic_flux(shape.rp, z, limb)
}

pub fn transit_duration_days(shape: &TransitShape) -> Option<f64> {
    let orbit = shape.a_rs * shape.a_rs - shape.b * shape.b;
    if orbit <= 0.0 {
        return None;
    }
    let outer = 1.0 + shape.rp;
    let arg = (outer * outer - shape.b * shape.b) / orbit;
    if arg <= 0.0 {
        None
    } else if arg >= 1.0 {
        Some(shape.period / 2.0)
    } else {
        Some(shape.period / PI * arg.sqrt().asin())
    }
}

pub fn full_transit_duration_days(shape: &TransitShape) -> Option<f64> {
    let orbit = shape.a_rs * shape.a_rs - shape.b * shape.b;
    if shape.rp >= 1.0 || shape.b >= 1.0 - shape.rp || orbit <= 0.0 {
        return None;
    }
    let inner = 1.0 - shape.rp;
    Some(shape.period / PI * ((inner * inner - shape.b * shape.b) / orbit).sqrt().asin())
}

fn check_finite(members: &[(&str, f64)]) -> Result<(), String> {
    match members.iter().find(|(_, value)| !value.is_finite()) {
        Some((name, _)) => Err(format!("{name} is not a finite number")),
        None => Ok(()),
    }
}

fn check_orbit(shape: &TransitShape) -> Result<(), String> {
    let TransitShape { period, rp, a_rs, b, .. } = *shape;
    if period <= 0.0 {
        return Err(format!("period {period} d must be above 0"));
    }
    if rp <= 0.0 || rp > MAX_RADIUS_RATIO {
        return Err(format!("rp {rp} must be above 0 and at most {MAX_RADIUS_RATIO} (planet radius over star radius)"));
    }
    if a_rs <= 1.0 {
        return Err(format!("a_rs {a_rs} must be above 1 (orbit radius over star radius)"));
    }
    if b < 0.0 {
        return Err(format!("b {b} must be 0 or above"));
    }
    if b >= a_rs {
        return Err(format!("b {b} must be below a_rs {a_rs}"));
    }
    Ok(())
}

pub fn check_transit_shape(shape: &TransitShape, limb: &LimbDarkening) -> Result<(), String> {
    let TransitShape { t0, period, rp, a_rs, b } = *shape;
    let LimbDarkening { u1, u2 } = *limb;
    check_finite(&[("t0", t0), ("period", period), ("rp", rp), ("a_rs", a_rs), ("b", b), ("u1", u1), ("u2", u2)])?;
    check_orbit(shape)?;
    if b >= 1.0 + rp {
        return Err(format!("no transit: b {b} is at or above 1 + rp = {}, so the planet never crosses the star", 1.0 + rp));
    }
    if rp >= 1.0 + b {
        return Err(format!(
            "the planet covers the whole star at mid-transit (rp {rp} is at or above 1 + b = {}): a total occultation is not fitted by this model",
            1.0 + b
        ));
    }
    if !(0.0..=2.0).contains(&u1) || !(-1.0..=1.0).contains(&u2) {
        return Err(format!("limb darkening out of range: u1 {u1} must be between 0 and 2 and u2 {u2} between -1 and 1"));
    }
    Ok(())
}

pub fn eclipse_flux(time: f64, shape: &TransitShape, flux_ratio: f64) -> f64 {
    let (z, near) = sky_separation(time, shape);
    if !near || z >= 1.0 + shape.rp {
        return 1.0;
    }
    1.0 - flux_ratio / (1.0 + flux_ratio) * uniform_overlap(shape.rp, z) / (shape.rp * shape.rp)
}

pub fn check_eclipse_shape(shape: &TransitShape, flux_ratio: f64) -> Result<(), String> {
    let TransitShape { t0, period, rp, a_rs, b } = *shape;
    check_finite(&[("t0", t0), ("period", period), ("rp", rp), ("a_rs", a_rs), ("b", b), ("flux_ratio", flux_ratio)])?;
    check_orbit(shape)?;
    if b >= 1.0 + rp {
        return Err(format!(
            "no eclipse: b {b} is at or above 1 + rp = {}, so the planet never passes behind the star",
            1.0 + rp
        ));
    }
    if flux_ratio <= -0.5 || flux_ratio > 10.0 {
        return Err(format!("flux_ratio {flux_ratio} must be above -0.5 and at most 10 (planet flux over star flux)"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const LD: LimbDarkening = LimbDarkening { u1: 0.4, u2: 0.25 };
    const SHAPE: TransitShape = TransitShape { t0: 0.0, period: 3.0, rp: 0.1, a_rs: 9.0, b: 0.3 };
    const WHITE_DWARF_SHAPE: TransitShape = TransitShape { t0: 0.0, period: 1.4079405, rp: 7.28, a_rs: 336.0, b: 7.16 };
    const ECLIPSE_SHAPE: TransitShape = TransitShape { t0: 10.0, period: 3.0, rp: 0.1, a_rs: 9.0, b: 0.3 };

    fn assert_close(got: f64, want: f64, tolerance: f64, what: &str) {
        assert!((got - want).abs() <= tolerance, "{what}: got {got:?}, want {want:?}, off by {:e}", (got - want).abs());
    }

    fn assert_relative(got: f64, want: f64, tolerance: f64, what: &str) {
        let relative = ((got - want) / want).abs();
        assert!(relative <= tolerance, "{what}: got {got:?}, want {want:?}, relative difference {relative:e}");
    }

    fn central_transit_flux(p: f64, limb: &LimbDarkening) -> f64 {
        let hidden = (1.0 - limb.u1 - 2.0 * limb.u2) * p * p
            + (limb.u1 + 2.0 * limb.u2) * (2.0 / 3.0) * (1.0 - (1.0 - p * p).powf(1.5))
            + limb.u2 * p.powi(4) / 2.0;
        1.0 - hidden / (1.0 - limb.u1 / 3.0 - limb.u2 / 6.0)
    }

    fn refusal(shape: TransitShape, limb: LimbDarkening) -> String {
        check_transit_shape(&shape, &limb).unwrap_err()
    }

    fn eclipse_refusal(shape: TransitShape, flux_ratio: f64) -> String {
        check_eclipse_shape(&shape, flux_ratio).unwrap_err()
    }

    #[test]
    fn carlson_integrals_match_reference_values() {
        let references = [
            ("RF(0, 0.5, 1)", carlson_rf(0.0, 0.5, 1.0), 1.8540746773013719),
            ("RD(0, 0.5, 1)", carlson_rd(0.0, 0.5, 1.0), 3.0205847775221786),
            ("RJ(0, 0.5, 1, 2)", carlson_rj(0.0, 0.5, 1.0, 2.0), 1.7428419316550683),
            ("RJ(2, 3, 4, 5)", carlson_rj(2.0, 3.0, 4.0, 5.0), 0.1429757966715675),
            ("RF(1, 2, 4)", carlson_rf(1.0, 2.0, 4.0), 0.6850858166334359),
            ("RD(0, 2, 1)", carlson_rd(0.0, 2.0, 1.0), 1.7972103521033884),
            ("RJ(0, 1e-6, 1, 3.5)", carlson_rj(0.0, 1e-6, 1.0, 3.5), 6.563367688097384),
            ("RJ(0, 0.9, 1, 1.0001)", carlson_rj(0.0, 0.9, 1.0, 1.0001), 2.4503267697785547),
        ];
        for (what, got, want) in references {
            assert_relative(got, want, 1e-13, what);
        }
        assert_close(carlson_rf(0.0, 1.0, 1.0), PI / 2.0, 1e-14, "RF(0, 1, 1)");
        assert_close(carlson_rd(0.0, 1.0, 1.0), 3.0 * PI / 4.0, 1e-14, "RD(0, 1, 1)");
        assert_close(carlson_rj(0.0, 1.0, 1.0, 1.0), 3.0 * PI / 4.0, 1e-14, "RJ(0, 1, 1, 1)");
    }

    #[test]
    fn the_uniform_overlap_follows_its_closed_forms() {
        assert_eq!(uniform_overlap(0.1, 0.3).to_bits(), (0.1_f64 * 0.1).to_bits());
        assert_eq!(uniform_overlap(0.1, 1.1), 0.0);
        assert_eq!(uniform_overlap(0.1, 2.0), 0.0);
        assert_eq!(uniform_overlap(1.5, 0.2), 1.0);
        let two_unit_discs_one_radius_apart = 2.0 / 3.0 - 3.0_f64.sqrt() / (2.0 * PI);
        assert_close(two_unit_discs_one_radius_apart, 0.3910022189557706, 1e-14, "closed form of two unit discs");
        assert_close(uniform_overlap(1.0, 1.0), two_unit_discs_one_radius_apart, 1e-14, "overlap(1, 1)");
        assert_close(uniform_overlap(7.28, 7.16), 0.5617640649892655, 1e-11, "overlap(7.28, 7.16)");
    }

    #[test]
    fn the_uniform_source_is_one_minus_the_overlap() {
        for p in [0.05, 0.1, 0.5, 1.0, 1.5] {
            for z in [0.0, 0.2, 0.7, 1.2, 2.2] {
                let flux = quadratic_flux(p, z, &LimbDarkening::default());
                assert_close(flux, 1.0 - uniform_overlap(p, z), 1e-15, &format!("p {p} z {z}"));
            }
        }
    }

    #[test]
    fn a_central_transit_matches_the_closed_form() {
        let steep = LimbDarkening { u1: 0.9, u2: -0.2 };
        let pinned = [
            (0.05, LD, 0.9969704552563485),
            (0.05, steep, 0.9965928271488542),
            (0.1, LD, 0.9878909547167664),
            (0.1, steep, 0.986394346697979),
            (0.5, LD, 0.7053320384278756),
            (0.5, steep, 0.6787586603810587),
            (0.9, LD, 0.1353684217652894),
            (0.9, steep, 0.12029503633057859),
        ];
        for (p, limb, value) in pinned {
            let what = format!("p {p} u1 {} u2 {}", limb.u1, limb.u2);
            let expected = central_transit_flux(p, &limb);
            assert_close(expected, value, 1e-13, &format!("closed form, {what}"));
            assert_close(quadratic_flux(p, 0.0, &limb), expected, 1e-13, &what);
        }
    }

    #[test]
    fn the_inner_contact_and_the_centre_crossing_are_continuous() {
        let (area, mu_term, r2_term) = hidden_terms(0.5, 0.5);
        assert_close(area, 0.25, 1e-14, "area(0.5, 0.5)");
        assert_close(1.0 / 3.0 - 4.0 / (9.0 * PI), 0.1918622728072041, 1e-14, "closed form of the mu term");
        assert_close(mu_term, 1.0 / 3.0 - 4.0 / (9.0 * PI), 1e-14, "mu_term(0.5, 0.5)");
        assert_close(r2_term, 3.0 / 32.0, 1e-14, "r2_term(0.5, 0.5)");
        for p in [0.05, 0.1, 0.3, 0.5, 0.8] {
            let contact = quadratic_flux(p, 1.0 - p, &LD);
            let outside = quadratic_flux(p, 1.0 - p + 1e-9, &LD);
            let inside = quadratic_flux(p, 1.0 - p - 1e-9, &LD);
            assert!((outside - contact).abs() < 1e-8, "p {p}: step of {:e} just outside inner contact", (outside - contact).abs());
            assert!((inside - contact).abs() < 1e-8, "p {p}: step of {:e} just inside inner contact", (inside - contact).abs());
            let step = (quadratic_flux(p, p + 1e-12, &LD) - quadratic_flux(p, p - 1e-12, &LD)).abs();
            assert!(step < 1e-10, "p {p}: step of {step:e} where the planet's edge crosses the centre");
        }
    }

    #[test]
    fn the_model_matches_the_numerical_integration_on_the_grid() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transit/model_grid.json")).unwrap();
        let doc = serde_json::from_str::<serde_json::Value>(&text).unwrap();
        let cases = doc["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 550);
        let mut worst = 0.0_f64;
        let mut worst_row = [0.0_f64; 5];
        for case in cases {
            let row: Vec<f64> = case.as_array().unwrap().iter().map(|member| member.as_f64().unwrap()).collect();
            let [p, z, u1, u2, flux] = row[..] else { panic!("a row of {} members", row.len()) };
            let difference = (quadratic_flux(p, z, &LimbDarkening { u1, u2 }) - flux).abs();
            assert!(difference.is_finite(), "p {p} z {z} u1 {u1} u2 {u2}: the model is not finite");
            if difference > worst {
                worst = difference;
                worst_row = [p, z, u1, u2, flux];
            }
        }
        println!("largest difference from the numerical integration: {worst:e} at [p, z, u1, u2, flux] = {worst_row:?}");
        assert!(worst <= 1e-9, "largest difference {worst:e} at [p, z, u1, u2, flux] = {worst_row:?}");
    }

    #[test]
    fn limits_of_the_geometry() {
        for (p, z) in [(0.1, 1.1), (0.1, 5.0), (7.28, 8.3), (7.28, 100.0)] {
            assert_eq!(quadratic_flux(p, z, &LD).to_bits(), 1.0_f64.to_bits(), "p {p} z {z}");
        }
        assert_eq!(hidden_terms(1.5, 0.2), (1.0, 2.0 / 3.0, 0.5));
        assert_close(quadratic_flux(1.5, 0.2, &LD), 0.0, 1e-15, "the hidden disc");
    }

    #[test]
    fn the_sky_separation_follows_the_circular_orbit() {
        assert_eq!(sky_separation(0.0, &SHAPE), (0.3, true));
        let (z, near) = sky_separation(0.03, &SHAPE);
        assert_close(z, (9.0 * (0.02 * PI).sin()).hypot(0.3 * (0.02 * PI).cos()), 1e-13, "z at 0.03 d");
        assert_close(z, 0.6395308892639974, 1e-13, "z at 0.03 d");
        assert!(near);
        let (far_z, far_near) = sky_separation(1.5, &SHAPE);
        assert_close(far_z, 0.3, 1e-12, "z at half a period");
        assert!(!far_near);
        assert_close(sky_separation(3.03, &SHAPE).0, z, 1e-11, "one period later");
        assert_close(sky_separation(-2.97, &SHAPE).0, z, 1e-11, "one period earlier");
        let dated = TransitShape { t0: 2460310.5, ..SHAPE };
        assert!(!sky_separation(2460310.5 + 1.5, &dated).1);
    }

    #[test]
    fn the_transit_curve_is_symmetric_and_flat_outside() {
        let mid = transit_flux(0.0, &SHAPE, &LD);
        assert_close(mid, 0.9881234631537438, 1e-11, "mid-transit");
        assert_close(transit_flux(0.03, &SHAPE, &LD), 0.989194961170402, 1e-11, "0.03 d after mid-transit");
        assert_close(transit_flux(-0.03, &SHAPE, &LD), 0.989194961170402, 1e-11, "0.03 d before mid-transit");
        assert_eq!(transit_flux(0.057, &SHAPE, &LD).to_bits(), 1.0_f64.to_bits());
        assert_eq!(transit_flux(1.5, &SHAPE, &LD).to_bits(), 1.0_f64.to_bits());
        assert_close(transit_flux(3.0, &SHAPE, &LD), mid, 1e-12, "one period after mid-transit");
    }

    #[test]
    fn durations_follow_the_contact_geometry() {
        let total = transit_duration_days(&SHAPE).unwrap();
        let full = full_transit_duration_days(&SHAPE).unwrap();
        assert_close(total, 3.0 / PI * ((1.1_f64.powi(2) - 0.09) / (81.0 - 0.09)).sqrt().asin(), 1e-13, "T14 closed form");
        assert_close(total, 0.1126124414167327, 1e-13, "T14");
        assert_close(full, 3.0 / PI * ((0.9_f64.powi(2) - 0.09) / (81.0 - 0.09)).sqrt().asin(), 1e-13, "T23 closed form");
        assert_close(full, 0.09021583157004577, 1e-13, "T23");
        assert!(transit_flux(total / 2.0 - 1e-6, &SHAPE, &LD) < 1.0);
        assert_eq!(transit_flux(total / 2.0 + 1e-6, &SHAPE, &LD).to_bits(), 1.0_f64.to_bits());
        let grazing = TransitShape { b: 0.95, ..SHAPE };
        assert_close(transit_duration_days(&grazing).unwrap(), 0.059205592919331106, 1e-13, "T14 at b 0.95");
        assert_eq!(full_transit_duration_days(&grazing), None);
        assert_close(transit_duration_days(&WHITE_DWARF_SHAPE).unwrap(), 0.0055480137900789, 1e-14, "T14 of the white dwarf shape");
        assert_eq!(full_transit_duration_days(&WHITE_DWARF_SHAPE), None);
        let missing = TransitShape { b: 1.2, ..SHAPE };
        assert_eq!(transit_duration_days(&missing), None);
        assert_eq!(full_transit_duration_days(&missing), None);
    }

    #[test]
    fn shapes_that_can_not_be_fitted_are_refused_with_the_reason() {
        assert_eq!(refusal(TransitShape { rp: f64::NAN, ..SHAPE }, LD), "rp is not a finite number");
        assert_eq!(refusal(TransitShape { period: 0.0, ..SHAPE }, LD), "period 0 d must be above 0");
        assert_eq!(refusal(TransitShape { rp: 0.0, ..SHAPE }, LD), "rp 0 must be above 0 and at most 100 (planet radius over star radius)");
        assert_eq!(refusal(TransitShape { rp: 150.0, ..SHAPE }, LD), "rp 150 must be above 0 and at most 100 (planet radius over star radius)");
        assert_eq!(refusal(TransitShape { a_rs: 1.0, ..SHAPE }, LD), "a_rs 1 must be above 1 (orbit radius over star radius)");
        assert_eq!(refusal(TransitShape { b: -0.1, ..SHAPE }, LD), "b -0.1 must be 0 or above");
        assert_eq!(refusal(TransitShape { a_rs: 1.05, b: 1.08, ..SHAPE }, LD), "b 1.08 must be below a_rs 1.05");
        assert_eq!(refusal(TransitShape { a_rs: 1.05, b: 1.05, ..SHAPE }, LD), "b 1.05 must be below a_rs 1.05");
        assert_eq!(
            refusal(TransitShape { b: 1.2, ..SHAPE }, LD),
            "no transit: b 1.2 is at or above 1 + rp = 1.1, so the planet never crosses the star"
        );
        assert_eq!(
            refusal(TransitShape { rp: 1.5, b: 0.2, ..SHAPE }, LD),
            "the planet covers the whole star at mid-transit (rp 1.5 is at or above 1 + b = 1.2): a total occultation is not fitted by this model"
        );
        assert_eq!(
            refusal(SHAPE, LimbDarkening { u1: 2.5, ..LD }),
            "limb darkening out of range: u1 2.5 must be between 0 and 2 and u2 0.25 between -1 and 1"
        );
        assert_eq!(
            refusal(SHAPE, LimbDarkening { u2: -1.5, ..LD }),
            "limb darkening out of range: u1 0.4 must be between 0 and 2 and u2 -1.5 between -1 and 1"
        );
        assert_eq!(check_transit_shape(&SHAPE, &LD), Ok(()));
        assert_eq!(check_transit_shape(&WHITE_DWARF_SHAPE, &LimbDarkening::default()), Ok(()));
    }

    #[test]
    fn a_planet_larger_than_its_star_grazes_it() {
        assert_close(quadratic_flux(7.28, 7.16, &LimbDarkening { u1: 0.1, u2: 0.3 }), 0.4351096408173768, 1e-10, "limb-darkened");
        assert_close(quadratic_flux(7.28, 7.16, &LimbDarkening::default()), 0.43823593501074565, 1e-10, "uniform");
    }

    #[test]
    fn the_eclipse_depth_is_the_flux_ratio_over_one_plus_it() {
        assert_close(1.0 / 1.002, 0.998003992015968, 1e-15, "closed form of the depth");
        assert_close(eclipse_flux(10.0, &ECLIPSE_SHAPE, 0.002), 1.0 / 1.002, 1e-15, "mid-eclipse");
        assert_eq!(eclipse_flux(10.07, &ECLIPSE_SHAPE, 0.002).to_bits(), 1.0_f64.to_bits(), "after last contact");
        assert_eq!(eclipse_flux(11.5, &ECLIPSE_SHAPE, 0.002).to_bits(), 1.0_f64.to_bits(), "the transit side of the orbit");
        let (z, near) = sky_separation(10.052, &ECLIPSE_SHAPE);
        assert!(near);
        assert_close(z, 1.0226881221237396, 1e-12, "z at 10.052 d");
        let partly_hidden = 1.0 - 0.002 / 1.002 * uniform_overlap(0.1, z) / 0.01;
        assert_close(partly_hidden, 0.9993070427539474, 1e-12, "closed form of the partly hidden planet");
        assert_close(eclipse_flux(10.052, &ECLIPSE_SHAPE, 0.002), 0.9993070427539474, 1e-12, "0.052 d after mid-eclipse");
        assert_close(eclipse_flux(9.948, &ECLIPSE_SHAPE, 0.002), 0.9993070427539474, 1e-12, "0.052 d before mid-eclipse");
        for time in [10.0, 10.07, 11.5, 10.052, 9.948] {
            assert_eq!(eclipse_flux(time, &ECLIPSE_SHAPE, 0.0).to_bits(), 1.0_f64.to_bits(), "no planet flux at {time}");
        }
    }

    #[test]
    fn a_planet_larger_than_the_star_is_never_fully_hidden() {
        let giant = TransitShape { rp: 2.0, b: 0.0, ..ECLIPSE_SHAPE };
        assert_close(eclipse_flux(10.0, &giant, 0.1), 1.0 - 0.1 / 1.1 / 4.0, 1e-15, "a quarter of the planet's disc hidden");
    }

    #[test]
    fn eclipse_shapes_that_can_not_be_fitted_are_refused_with_the_reason() {
        assert_eq!(check_eclipse_shape(&ECLIPSE_SHAPE, 0.002), Ok(()));
        assert_eq!(eclipse_refusal(ECLIPSE_SHAPE, f64::NAN), "flux_ratio is not a finite number");
        assert_eq!(eclipse_refusal(TransitShape { rp: f64::INFINITY, ..ECLIPSE_SHAPE }, f64::NAN), "rp is not a finite number");
        assert_eq!(
            eclipse_refusal(TransitShape { b: 1.2, ..ECLIPSE_SHAPE }, 0.002),
            "no eclipse: b 1.2 is at or above 1 + rp = 1.1, so the planet never passes behind the star"
        );
        assert_eq!(
            eclipse_refusal(ECLIPSE_SHAPE, -0.6),
            "flux_ratio -0.6 must be above -0.5 and at most 10 (planet flux over star flux)"
        );
        assert_eq!(
            eclipse_refusal(ECLIPSE_SHAPE, 11.0),
            "flux_ratio 11 must be above -0.5 and at most 10 (planet flux over star flux)"
        );
        assert_eq!(
            eclipse_refusal(ECLIPSE_SHAPE, -0.5),
            "flux_ratio -0.5 must be above -0.5 and at most 10 (planet flux over star flux)"
        );
        assert_eq!(check_eclipse_shape(&ECLIPSE_SHAPE, 10.0), Ok(()));
        assert_eq!(check_eclipse_shape(&ECLIPSE_SHAPE, -0.25), Ok(()));
        assert_eq!(check_eclipse_shape(&TransitShape { rp: 1.5, b: 0.2, ..ECLIPSE_SHAPE }, 0.002), Ok(()));
        assert_eq!(eclipse_refusal(TransitShape { period: 0.0, ..ECLIPSE_SHAPE }, 0.002), "period 0 d must be above 0");
        let refused_by_both = [
            TransitShape { period: -2.0, ..ECLIPSE_SHAPE },
            TransitShape { rp: 0.0, ..ECLIPSE_SHAPE },
            TransitShape { rp: 150.0, ..ECLIPSE_SHAPE },
            TransitShape { a_rs: 1.0, ..ECLIPSE_SHAPE },
            TransitShape { b: -0.1, ..ECLIPSE_SHAPE },
            TransitShape { a_rs: 1.05, b: 1.08, ..ECLIPSE_SHAPE },
        ];
        for shape in refused_by_both {
            assert_eq!(eclipse_refusal(shape, 0.002), refusal(shape, LD), "{shape:?}");
        }
    }
}
