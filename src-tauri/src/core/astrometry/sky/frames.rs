use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkyFrame {
    Icrs,
    Fk5J2000,
    Galactic,
    EclipticJ2000,
}

impl SkyFrame {
    pub const ALL: [SkyFrame; 4] = [
        SkyFrame::Icrs,
        SkyFrame::Fk5J2000,
        SkyFrame::Galactic,
        SkyFrame::EclipticJ2000,
    ];

    pub fn from_name(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "icrs" => Ok(SkyFrame::Icrs),
            "fk5" => Ok(SkyFrame::Fk5J2000),
            "galactic" => Ok(SkyFrame::Galactic),
            "ecliptic" => Ok(SkyFrame::EclipticJ2000),
            other => Err(format!(
                "unknown sky frame '{other}' (supported: icrs, fk5, galactic, ecliptic)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SkyFrame::Icrs => "icrs",
            SkyFrame::Fk5J2000 => "fk5",
            SkyFrame::Galactic => "galactic",
            SkyFrame::EclipticJ2000 => "ecliptic",
        }
    }

    pub fn lon_in_hours(self) -> bool {
        matches!(self, SkyFrame::Icrs | SkyFrame::Fk5J2000)
    }
}

pub type Mat3 = [[f64; 3]; 3];

pub fn rot_x(angle_deg: f64) -> Mat3 {
    let (s, c) = angle_deg.to_radians().sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]]
}

pub fn rot_y(angle_deg: f64) -> Mat3 {
    let (s, c) = angle_deg.to_radians().sin_cos();
    [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]]
}

pub fn rot_z(angle_deg: f64) -> Mat3 {
    let (s, c) = angle_deg.to_radians().sin_cos();
    [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
}

pub fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

pub fn mat_transpose(a: &Mat3) -> Mat3 {
    [
        [a[0][0], a[1][0], a[2][0]],
        [a[0][1], a[1][1], a[2][1]],
        [a[0][2], a[1][2], a[2][2]],
    ]
}

pub fn mat_apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub fn spherical_to_cartesian(lon_deg: f64, lat_deg: f64) -> [f64; 3] {
    let (sl, cl) = lon_deg.to_radians().sin_cos();
    let (sb, cb) = lat_deg.to_radians().sin_cos();
    [cb * cl, cb * sl, sb]
}

pub fn cartesian_to_spherical(v: [f64; 3]) -> (f64, f64) {
    let mut lon = v[1].atan2(v[0]).to_degrees().rem_euclid(360.0);
    if lon >= 360.0 {
        lon = 0.0;
    }
    let lat = v[2].atan2(v[0].hypot(v[1])).to_degrees();
    (lon, lat)
}

const FK5_ETA0_DEG: f64 = -19.9e-3 / 3600.0;
const FK5_XI0_DEG: f64 = 9.1e-3 / 3600.0;
const FK5_DA0_DEG: f64 = -22.9e-3 / 3600.0;

const IAU2006_GAMMA_BAR_J2000_DEG: f64 = -0.052928 / 3600.0;
const IAU2006_PHI_BAR_J2000_DEG: f64 = 84381.412819 / 3600.0;
const IAU2006_PSI_BAR_J2000_DEG: f64 = -0.041775 / 3600.0;

const GAL_NGP_RA_J2000_DEG: f64 = 192.8594812065348;
const GAL_NGP_DEC_J2000_DEG: f64 = 27.12825118085622;
const GAL_LON0_J2000_DEG: f64 = 122.9319185680026;

pub static ICRS_TO_FK5_J2000: LazyLock<Mat3> = LazyLock::new(|| {
    mat_mul(
        &mat_mul(&rot_x(-FK5_ETA0_DEG), &rot_y(FK5_XI0_DEG)),
        &rot_z(FK5_DA0_DEG),
    )
});

pub static FK5_J2000_TO_GALACTIC: LazyLock<Mat3> = LazyLock::new(|| {
    mat_mul(
        &mat_mul(
            &rot_z(180.0 - GAL_LON0_J2000_DEG),
            &rot_y(90.0 - GAL_NGP_DEC_J2000_DEG),
        ),
        &rot_z(GAL_NGP_RA_J2000_DEG),
    )
});

pub static ICRS_TO_ECLIPTIC_J2000: LazyLock<Mat3> = LazyLock::new(|| {
    mat_mul(
        &mat_mul(&rot_z(-IAU2006_PSI_BAR_J2000_DEG), &rot_x(IAU2006_PHI_BAR_J2000_DEG)),
        &rot_z(IAU2006_GAMMA_BAR_J2000_DEG),
    )
});

static ICRS_TO_GALACTIC: LazyLock<Mat3> =
    LazyLock::new(|| mat_mul(&FK5_J2000_TO_GALACTIC, &ICRS_TO_FK5_J2000));

fn transform(m: &Mat3, lon: f64, lat: f64) -> (f64, f64) {
    cartesian_to_spherical(mat_apply(m, spherical_to_cartesian(lon, lat)))
}

pub fn icrs_to_fk5_j2000(ra: f64, dec: f64) -> (f64, f64) {
    transform(&ICRS_TO_FK5_J2000, ra, dec)
}

pub fn fk5_j2000_to_icrs(ra: f64, dec: f64) -> (f64, f64) {
    transform(&mat_transpose(&ICRS_TO_FK5_J2000), ra, dec)
}

pub fn icrs_to_galactic(ra: f64, dec: f64) -> (f64, f64) {
    transform(&ICRS_TO_GALACTIC, ra, dec)
}

pub fn icrs_to_ecliptic_j2000(ra: f64, dec: f64) -> (f64, f64) {
    transform(&ICRS_TO_ECLIPTIC_J2000, ra, dec)
}

pub fn convert_from_icrs(frame: SkyFrame, ra: f64, dec: f64) -> (f64, f64) {
    match frame {
        SkyFrame::Icrs => (ra.rem_euclid(360.0), dec),
        SkyFrame::Fk5J2000 => icrs_to_fk5_j2000(ra, dec),
        SkyFrame::Galactic => icrs_to_galactic(ra, dec),
        SkyFrame::EclipticJ2000 => icrs_to_ecliptic_j2000(ra, dec),
    }
}

pub fn galactic_to_icrs(lon: f64, lat: f64) -> (f64, f64) {
    transform(&mat_transpose(&ICRS_TO_GALACTIC), lon, lat)
}

pub fn ecliptic_j2000_to_icrs(lon: f64, lat: f64) -> (f64, f64) {
    transform(&mat_transpose(&ICRS_TO_ECLIPTIC_J2000), lon, lat)
}

pub fn convert_to_icrs(frame: SkyFrame, lon: f64, lat: f64) -> (f64, f64) {
    match frame {
        SkyFrame::Icrs => (lon.rem_euclid(360.0), lat),
        SkyFrame::Fk5J2000 => fk5_j2000_to_icrs(lon, lat),
        SkyFrame::Galactic => galactic_to_icrs(lon, lat),
        SkyFrame::EclipticJ2000 => ecliptic_j2000_to_icrs(lon, lat),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lon_diff(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    fn assert_orthonormal(m: &Mat3, name: &str) {
        let p = mat_mul(m, &mat_transpose(m));
        for i in 0..3 {
            for j in 0..3 {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (p[i][j] - want).abs() < 1e-12,
                    "{name}: (M*Mt)[{i}][{j}] = {}",
                    p[i][j]
                );
            }
        }
    }

    #[test]
    fn from_name_parses_the_four_frames_and_rejects_others() {
        assert_eq!(SkyFrame::from_name("icrs").unwrap(), SkyFrame::Icrs);
        assert_eq!(SkyFrame::from_name(" FK5 ").unwrap(), SkyFrame::Fk5J2000);
        assert_eq!(SkyFrame::from_name("Galactic").unwrap(), SkyFrame::Galactic);
        assert_eq!(SkyFrame::from_name("ecliptic").unwrap(), SkyFrame::EclipticJ2000);
        for frame in SkyFrame::ALL {
            assert_eq!(SkyFrame::from_name(frame.name()).unwrap(), frame);
        }
        let err = SkyFrame::from_name("supergalactic").unwrap_err();
        assert!(err.contains("supergalactic"), "{err}");
        for name in ["icrs", "fk5", "galactic", "ecliptic"] {
            assert!(err.contains(name), "{err} lacks {name}");
        }
        assert!(SkyFrame::from_name("").is_err());
    }

    #[test]
    fn only_equatorial_frames_report_longitude_in_hours() {
        assert!(SkyFrame::Icrs.lon_in_hours());
        assert!(SkyFrame::Fk5J2000.lon_in_hours());
        assert!(!SkyFrame::Galactic.lon_in_hours());
        assert!(!SkyFrame::EclipticJ2000.lon_in_hours());
    }

    #[test]
    fn rotation_matrices_follow_the_frame_rotation_convention() {
        let z = rot_z(90.0);
        let v = mat_apply(&z, [1.0, 0.0, 0.0]);
        assert!((v[0]).abs() < 1e-15 && (v[1] + 1.0).abs() < 1e-15, "{v:?}");
        let x = rot_x(90.0);
        let v = mat_apply(&x, [0.0, 1.0, 0.0]);
        assert!((v[1]).abs() < 1e-15 && (v[2] + 1.0).abs() < 1e-15, "{v:?}");
        let y = rot_y(90.0);
        let v = mat_apply(&y, [0.0, 0.0, 1.0]);
        assert!((v[0] + 1.0).abs() < 1e-15 && (v[2]).abs() < 1e-15, "{v:?}");
        for m in [rot_x(33.0), rot_y(-71.5), rot_z(200.0)] {
            assert_orthonormal(&m, "rot");
        }
    }

    #[test]
    fn spherical_round_trip_and_longitude_wrap() {
        for (lon, lat) in [(0.0, 0.0), (123.456, -45.0), (359.999, 89.0), (200.0, -89.9)] {
            let (l, b) = cartesian_to_spherical(spherical_to_cartesian(lon, lat));
            assert!(lon_diff(l, lon) < 1e-9 && (b - lat).abs() < 1e-9, "({lon},{lat}) -> ({l},{b})");
        }
        let (l, _) = cartesian_to_spherical([1.0, -1e-18, 0.0]);
        assert!((0.0..360.0).contains(&l), "lon {l} must be in [0,360)");
        let (l, b) = convert_from_icrs(SkyFrame::Icrs, -10.0, 5.0);
        assert!((l - 350.0).abs() < 1e-12 && b == 5.0);
        let (l, _) = convert_from_icrs(SkyFrame::Icrs, 370.0, 0.0);
        assert!((l - 10.0).abs() < 1e-12);
        let (l, _) = cartesian_to_spherical([1.0, 0.0, 0.0]);
        assert_eq!(l, 0.0);
    }

    #[test]
    fn constant_matrices_are_orthonormal() {
        assert_orthonormal(&ICRS_TO_FK5_J2000, "icrs->fk5");
        assert_orthonormal(&FK5_J2000_TO_GALACTIC, "fk5->gal");
        assert_orthonormal(&ICRS_TO_ECLIPTIC_J2000, "icrs->ecl");
        assert_orthonormal(&ICRS_TO_GALACTIC, "icrs->gal");
    }

    #[test]
    fn galactic_matches_astropy_goldens() {
        let (l, b) = icrs_to_galactic(266.4049882865447, -28.936177761791473);
        assert!(lon_diff(l, 0.0) < 1e-4, "galactic centre l = {l}");
        assert!(b.abs() < 1e-4, "galactic centre b = {b}");

        let (_, b) = icrs_to_galactic(192.85948, 27.12825);
        assert!((b - 90.0).abs() < 1e-4, "NGP b = {b}");

        let (l, b) = icrs_to_galactic(0.0, 0.0);
        assert!((l - 96.33726).abs() < 1e-3, "vernal equinox l = {l}");
        assert!((b + 60.18855).abs() < 1e-3, "vernal equinox b = {b}");

        let (l, b) = icrs_to_galactic(0.0, 90.0);
        assert!((l - 122.932).abs() < 1e-3, "NCP l = {l}");
        assert!((b - 27.128).abs() < 1e-3, "NCP b = {b}");
    }

    #[test]
    fn ecliptic_matches_reference_points() {
        let (l, b) = icrs_to_ecliptic_j2000(0.0, 0.0);
        assert!(lon_diff(l, 0.0) < 1e-5 && b.abs() < 1e-5, "equinox -> ({l},{b})");
        let (l, b) = icrs_to_ecliptic_j2000(90.0, 23.4392794);
        assert!((l - 90.0).abs() < 1e-5 && b.abs() < 1e-5, "solstice -> ({l},{b})");
        let (_, b) = icrs_to_ecliptic_j2000(270.0, 66.5607206);
        assert!((b - 90.0).abs() < 1e-5, "ecliptic pole b = {b}");
        let (l, b) = icrs_to_ecliptic_j2000(180.0, 0.0);
        assert!((l - 180.0).abs() < 1e-5 && b.abs() < 1e-5, "autumn equinox -> ({l},{b})");
    }

    #[test]
    fn ecliptic_j2000_uses_the_iau_2006_frame_bias() {
        let (ra, dec) = ecliptic_j2000_to_icrs(0.0, 0.0);
        let ra_mas = (ra + 180.0).rem_euclid(360.0) * 3.6e6 - 180.0 * 3.6e6;
        let dec_mas = dec * 3.6e6;
        assert!((ra_mas + 14.6).abs() < 0.05, "ICRS RA of the J2000 mean equinox = {ra_mas} mas, IERS dalpha0 = -14.6 mas");
        assert!((dec_mas - 16.617).abs() < 0.01, "ICRS Dec of the J2000 mean equinox = {dec_mas} mas, IERS -xi0 = 16.617 mas");

        let (l, b) = icrs_to_ecliptic_j2000(0.0, 0.0);
        assert!((l - 1.884_859_492e-6).abs() < 1e-12, "l = {l}");
        assert!((b + 5.848_205_841e-6).abs() < 1e-12, "b = {b}");
        let (l, b) = icrs_to_ecliptic_j2000(83.5, -5.25);
        assert!((l - 82.628_586_772_530_21).abs() < 1e-10 && (b + 28.523_108_557_033_04).abs() < 1e-10, "({l},{b})");
    }

    #[test]
    fn fk5_is_a_tiny_frame_bias_and_round_trips() {
        let (ra, dec) = icrs_to_fk5_j2000(10.0, 20.0);
        let sep = crate::core::astrometry::wcs::angular_separation(10.0, 20.0, ra, dec) * 3600.0;
        assert!(sep > 0.005 && sep < 0.03, "ICRS->FK5 separation {sep} arcsec");

        let back = mat_transpose(&ICRS_TO_FK5_J2000);
        let (r2, d2) = cartesian_to_spherical(mat_apply(&back, spherical_to_cartesian(ra, dec)));
        assert!(lon_diff(r2, 10.0) < 1e-9 && (d2 - 20.0).abs() < 1e-9, "({r2},{d2})");

        let (l, b) = icrs_to_galactic(83.5, -5.25);
        let back = mat_transpose(&ICRS_TO_GALACTIC);
        let (r2, d2) = cartesian_to_spherical(mat_apply(&back, spherical_to_cartesian(l, b)));
        assert!(lon_diff(r2, 83.5) < 1e-9 && (d2 + 5.25).abs() < 1e-9, "({r2},{d2})");

        let (l, b) = icrs_to_ecliptic_j2000(300.0, 40.0);
        let back = mat_transpose(&ICRS_TO_ECLIPTIC_J2000);
        let (r2, d2) = cartesian_to_spherical(mat_apply(&back, spherical_to_cartesian(l, b)));
        assert!(lon_diff(r2, 300.0) < 1e-9 && (d2 - 40.0).abs() < 1e-9, "({r2},{d2})");
    }

    #[test]
    fn fk5_to_icrs_inverts_icrs_to_fk5_at_several_positions() {
        for (ra, dec) in [(0.0, 0.0), (10.0, 20.0), (150.0, 2.0), (274.7, -13.8), (359.9, 89.5)] {
            let (fra, fdec) = icrs_to_fk5_j2000(ra, dec);
            let (r2, d2) = fk5_j2000_to_icrs(fra, fdec);
            assert!(lon_diff(r2, ra) < 1e-9 && (d2 - dec).abs() < 1e-9, "({ra},{dec}) -> ({r2},{d2})");
            let (r3, d3) = icrs_to_fk5_j2000(r2, d2);
            assert!(lon_diff(r3, fra) < 1e-9 && (d3 - fdec).abs() < 1e-9);
        }
        let sep = crate::core::astrometry::wcs::angular_separation(10.0, 20.0, fk5_j2000_to_icrs(10.0, 20.0).0, fk5_j2000_to_icrs(10.0, 20.0).1) * 3600.0;
        assert!(sep > 0.005 && sep < 0.03, "FK5->ICRS separation {sep} arcsec");
    }

    #[test]
    fn convert_from_icrs_dispatches_per_frame_and_passes_nan_through() {
        assert_eq!(convert_from_icrs(SkyFrame::Icrs, 12.5, -3.0), (12.5, -3.0));
        assert_eq!(
            convert_from_icrs(SkyFrame::Galactic, 12.5, -3.0),
            icrs_to_galactic(12.5, -3.0)
        );
        assert_eq!(
            convert_from_icrs(SkyFrame::EclipticJ2000, 12.5, -3.0),
            icrs_to_ecliptic_j2000(12.5, -3.0)
        );
        assert_eq!(
            convert_from_icrs(SkyFrame::Fk5J2000, 12.5, -3.0),
            icrs_to_fk5_j2000(12.5, -3.0)
        );
        for frame in SkyFrame::ALL {
            let (l, _) = convert_from_icrs(frame, f64::NAN, 1.0);
            assert!(l.is_nan(), "{frame:?} NaN ra -> lon {l}");
            let (_, b) = convert_from_icrs(frame, 1.0, f64::NAN);
            assert!(b.is_nan(), "{frame:?} NaN dec -> lat {b}");
        }
        let (l, b) = convert_from_icrs(SkyFrame::Galactic, f64::NAN, 1.0);
        assert!(l.is_nan() && b.is_nan());
    }

    #[test]
    fn convert_to_icrs_inverts_convert_from_icrs_for_every_frame() {
        let positions = [
            (0.0, 0.0),
            (10.0, 20.0),
            (83.5, -5.25),
            (192.85948, 27.12825),
            (266.40498, -28.93617),
            (359.99, 89.5),
            (300.0, -89.9),
        ];
        for frame in SkyFrame::ALL {
            for &(ra, dec) in &positions {
                let (lon, lat) = convert_from_icrs(frame, ra, dec);
                let (ra2, dec2) = convert_to_icrs(frame, lon, lat);
                assert!(
                    lon_diff(ra2, ra) < 1e-9 && (dec2 - dec).abs() < 1e-9,
                    "{frame:?}: ({ra},{dec}) -> ({lon},{lat}) -> ({ra2},{dec2})"
                );
                let (lon2, lat2) = convert_from_icrs(frame, ra2, dec2);
                let sep = crate::core::astrometry::wcs::angular_separation(lon, lat, lon2, lat2);
                assert!(sep < 1e-9, "{frame:?}: ({lon},{lat}) vs ({lon2},{lat2}) separated by {sep} deg");
            }
        }
        let (ra, dec) = convert_to_icrs(SkyFrame::Icrs, -30.0, 5.0);
        assert!((ra - 330.0).abs() < 1e-12 && dec == 5.0);
        let (ra, dec) = convert_to_icrs(SkyFrame::Galactic, 0.0, 0.0);
        assert!(lon_diff(ra, 266.40498).abs() < 1e-4 && (dec + 28.93617).abs() < 1e-4, "({ra},{dec})");
        let (ra, dec) = convert_to_icrs(SkyFrame::EclipticJ2000, 90.0, 0.0);
        assert!(lon_diff(ra, 90.0) < 1e-5 && (dec - 23.4392794).abs() < 1e-5, "({ra},{dec})");
        for frame in SkyFrame::ALL {
            let (ra, _) = convert_to_icrs(frame, f64::NAN, 1.0);
            assert!(ra.is_nan(), "{frame:?} NaN lon -> ra {ra}");
            let (_, dec) = convert_to_icrs(frame, 1.0, f64::NAN);
            assert!(dec.is_nan(), "{frame:?} NaN lat -> dec {dec}");
        }
        let (ra, dec) = convert_to_icrs(SkyFrame::Galactic, f64::NAN, 1.0);
        assert!(ra.is_nan() && dec.is_nan());
    }
}
