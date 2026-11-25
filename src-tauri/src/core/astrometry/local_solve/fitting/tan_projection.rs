use crate::core::registration::poly::TanParams;

use crate::core::astrometry::local_solve::constants::{ARCSEC_PER_DEG, TAN_LON_POLE, SCALE_TEXT_DIGITS};
use crate::core::astrometry::local_solve::types::ImageParity;

pub(crate) fn destination(
    (ra, dec): (f64, f64),
    bearing_deg: f64,
    distance_deg: f64,
) -> (f64, f64) {
    let (sin_ra, cos_ra) = ra.to_radians().sin_cos();
    let (sin_dec, cos_dec) = dec.to_radians().sin_cos();
    let (sin_bearing, cos_bearing) = bearing_deg.to_radians().sin_cos();
    let (sin_dist, cos_dist) = distance_deg.to_radians().sin_cos();
    let north = sin_dist * cos_bearing;
    let east = sin_dist * sin_bearing;
    let x = cos_dist * cos_dec * cos_ra - north * sin_dec * cos_ra - east * sin_ra;
    let y = cos_dist * cos_dec * sin_ra - north * sin_dec * sin_ra + east * cos_ra;
    let z = cos_dist * sin_dec + north * cos_dec;
    (
        wrap_longitude_deg(y.atan2(x).to_degrees()),
        z.atan2(x.hypot(y)).to_degrees(),
    )
}

fn wrap_longitude_deg(value: f64) -> f64 {
    let wrapped = value.rem_euclid(360.0);
    if wrapped >= 360.0 {
        0.0
    } else {
        wrapped
    }
}

pub(in crate::core::astrometry::local_solve) fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

pub(in crate::core::astrometry::local_solve) fn scale_text(arcsec: f64) -> String {
    if !is_positive(arcsec) {
        return arcsec.to_string();
    }
    let decimals = (SCALE_TEXT_DIGITS - 1.0 - arcsec.log10().floor()).max(0.0) as usize;
    format!("{arcsec:.decimals$}")
}

pub(in crate::core::astrometry::local_solve) fn centre_crpix((width, height): (usize, usize)) -> (f64, f64) {
    ((width as f64 + 1.0) / 2.0, (height as f64 + 1.0) / 2.0)
}

pub(in crate::core::astrometry::local_solve) fn tan_plane((ra0, dec0): (f64, f64), ra: f64, dec: f64) -> Option<(f64, f64)> {
    let (sin_dec, cos_dec) = dec.to_radians().sin_cos();
    let (sin_dec0, cos_dec0) = dec0.to_radians().sin_cos();
    let (sin_dra, cos_dra) = (ra - ra0).to_radians().sin_cos();
    let depth = sin_dec * sin_dec0 + cos_dec * cos_dec0 * cos_dra;
    if !is_positive(depth) {
        return None;
    }
    let x = cos_dec * sin_dra / depth;
    let y = (sin_dec * cos_dec0 - cos_dec * sin_dec0 * cos_dra) / depth;
    Some((x.to_degrees(), y.to_degrees()))
}

pub(in crate::core::astrometry::local_solve) fn tan_sky((ra0, dec0): (f64, f64), x_deg: f64, y_deg: f64) -> (f64, f64) {
    let (x, y) = (x_deg.to_radians(), y_deg.to_radians());
    let (sin_dec0, cos_dec0) = dec0.to_radians().sin_cos();
    let norm = (1.0 + x * x + y * y).sqrt();
    let dec = ((sin_dec0 + y * cos_dec0) / norm).clamp(-1.0, 1.0).asin();
    let ra = ra0.to_radians() + x.atan2(cos_dec0 - y * sin_dec0);
    (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
}

pub(in crate::core::astrometry::local_solve) fn cd_inverse(cd: &[[f64; 2]; 2]) -> Option<[[f64; 2]; 2]> {
    let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
    if !det.is_finite() || det == 0.0 {
        return None;
    }
    Some([
        [cd[1][1] / det, -cd[0][1] / det],
        [-cd[1][0] / det, cd[0][0] / det],
    ])
}

pub(in crate::core::astrometry::local_solve) fn pixel_to_sky(tan: &TanParams, x: f64, y: f64) -> (f64, f64) {
    let dx = x + 1.0 - tan.crpix.0;
    let dy = y + 1.0 - tan.crpix.1;
    let plane_x = tan.cd[0][0] * dx + tan.cd[0][1] * dy;
    let plane_y = tan.cd[1][0] * dx + tan.cd[1][1] * dy;
    tan_sky(tan.crval, plane_x, plane_y)
}

pub(in crate::core::astrometry::local_solve) fn sky_to_pixel(tan: &TanParams, ra: f64, dec: f64) -> Option<(f64, f64)> {
    let (plane_x, plane_y) = tan_plane(tan.crval, ra, dec)?;
    let inverse = cd_inverse(&tan.cd)?;
    let dx = inverse[0][0] * plane_x + inverse[0][1] * plane_y;
    let dy = inverse[1][0] * plane_x + inverse[1][1] * plane_y;
    Some((dx + tan.crpix.0 - 1.0, dy + tan.crpix.1 - 1.0))
}

pub(in crate::core::astrometry::local_solve) fn scale_arcsec(cd: &[[f64; 2]; 2]) -> f64 {
    (cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]).abs().sqrt() * ARCSEC_PER_DEG
}

pub(in crate::core::astrometry::local_solve) fn rotation_deg(cd: &[[f64; 2]; 2]) -> f64 {
    (-cd[0][1]).atan2(cd[1][1]).to_degrees()
}

pub(in crate::core::astrometry::local_solve) fn parity_of(cd: &[[f64; 2]; 2]) -> ImageParity {
    if cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0] > 0.0 {
        ImageParity::Flipped
    } else {
        ImageParity::Normal
    }
}

pub(in crate::core::astrometry::local_solve) fn tan_cards(tan: &TanParams) -> Vec<(String, String)> {
    [
        ("WCSAXES", "2".to_string()),
        ("CTYPE1", "RA---TAN".to_string()),
        ("CTYPE2", "DEC--TAN".to_string()),
        ("CRPIX1", tan.crpix.0.to_string()),
        ("CRPIX2", tan.crpix.1.to_string()),
        ("CRVAL1", tan.crval.0.to_string()),
        ("CRVAL2", tan.crval.1.to_string()),
        ("CUNIT1", "deg".to_string()),
        ("CUNIT2", "deg".to_string()),
        ("CD1_1", tan.cd[0][0].to_string()),
        ("CD1_2", tan.cd[0][1].to_string()),
        ("CD2_1", tan.cd[1][0].to_string()),
        ("CD2_2", tan.cd[1][1].to_string()),
        ("LONPOLE", TAN_LON_POLE.to_string()),
        ("RADESYS", "ICRS".to_string()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect()
}
