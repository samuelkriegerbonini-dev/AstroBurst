use serde::Serialize;

use crate::core::astrometry::frames::{
    cartesian_to_spherical, mat_apply, mat_mul, rot_x, rot_y, rot_z, spherical_to_cartesian, Mat3,
};
use crate::core::astrometry::spectral::{
    earth_heliocentric_position_au, header_target_coordinates, invalid_time_cards, is_spacecraft,
    longitude_sign_is_ambiguous, mid_exposure_jd, moon_geocentric_ecliptic_au, no_site_in_header_message, site_location,
    sun_barycentric_position_au, sun_geometric_longitude_deg, target_unit_vector, SiteLocation, AU_KM, SPEED_OF_LIGHT_KMS,
    WGS84_A_M, WGS84_F,
};
use crate::core::astrometry::time::{gmst_deg, jd_from_gregorian, julian_centuries_j2000, JD_J2000, SECONDS_PER_DAY};
use crate::core::astrometry::wcs::{angular_separation, pixel_center};
use crate::infra::wcs_source::load_wcs;
use crate::types::header::HduHeader;

pub const AIRMASS_FORMULA: &str = "Kasten & Young 1989";
pub const TT_MINUS_TAI_SECONDS: f64 = 32.184;
pub const AU_LIGHT_SECONDS: f64 = AU_KM / SPEED_OF_LIGHT_KMS;
pub const LEAP_SECONDS: [(i32, u32, f64); 28] = [
    (1972, 1, 10.0),
    (1972, 7, 11.0),
    (1973, 1, 12.0),
    (1974, 1, 13.0),
    (1975, 1, 14.0),
    (1976, 1, 15.0),
    (1977, 1, 16.0),
    (1978, 1, 17.0),
    (1979, 1, 18.0),
    (1980, 1, 19.0),
    (1981, 7, 20.0),
    (1982, 7, 21.0),
    (1983, 7, 22.0),
    (1985, 7, 23.0),
    (1988, 1, 24.0),
    (1990, 1, 25.0),
    (1991, 1, 26.0),
    (1992, 7, 27.0),
    (1993, 7, 28.0),
    (1994, 7, 29.0),
    (1996, 1, 30.0),
    (1997, 7, 31.0),
    (1999, 1, 32.0),
    (2006, 1, 33.0),
    (2009, 1, 34.0),
    (2012, 7, 35.0),
    (2015, 7, 36.0),
    (2017, 1, 37.0),
];
pub const BJD_SOURCE_COMPUTED: &str = "computed";
pub const BJD_SOURCE_HEADER: &str = "header";
pub const SOURCE_USER: &str = "user";
pub const BARYCENTRIC_TIME_SOURCE: &str = "mid-exposure from BJDREFI + BJDREFF + (TSTART + TSTOP)/2";

const TAI_MINUS_UTC_BEFORE_TABLE_SECONDS: f64 = 10.0;
const TAI_MINUS_GPS_SECONDS: f64 = 19.0;
const IAU_L_G: f64 = 6.969290134e-10;
const IAU_L_B: f64 = 1.550519768e-8;
const IAU_TDB0_SECONDS: f64 = -6.55e-5;
const IAU_T0_JD: f64 = 2443144.5003725;
const BEFORE_TABLE_NOTE: &str = "before 1972-01-01: TAI-UTC taken as 10 s (leap-second table starts there)";
const NO_TIME_REASON: &str =
    "no observation time in the header (MJD-AVG, EXPMID, DATE-AVG, DATE-OBS, MJD-OBS or BJDREF with TSTART/TSTOP)";
const TDB_MINUS_TT_METHOD: &str = "TDB - TT from the two-term Fairhead & Bretagnon series (Astronomical Almanac, 30 us)";
const SCALE_UTC: &str = "UTC";
const SCALE_TT: &str = "TT";
const SCALE_TAI: &str = "TAI";
const SCALE_GPS: &str = "GPS";
const SCALE_TCG: &str = "TCG";
const SCALE_TDB: &str = "TDB";
const SCALE_TCB: &str = "TCB";
const BARYCENTRIC_SCALES: [&str; 2] = [SCALE_TDB, SCALE_TCB];
const TIMESYS_KEY: &str = "TIMESYS";
const TREFPOS_KEY: &str = "TREFPOS";
const SUPPORTED_TIME_REFERENCES: &str = "TREFPOS TOPOCENTER, GEOCENTER or BARYCENTER; TIMEREF LOCAL, GEOCENTRIC or SOLARSYSTEM";
const TIMEREF_KEY: &str = "TIMEREF";
const TIMEREF_BARYCENTRIC_PREFIX: &str = "SOLARSYSTEM";
const TREFPOS_SIGNIFICANT_CHARS: usize = 3;
const TCB_AT_OBSERVER: &str = "TIMESYS TCB is a barycentric time scale (FITS 4.0 Table 32 pairs it with BARYCENTER), but no TREFPOS, TIMEREF or BJDREFI places the time at the barycentre: the header time is not used, so no light-time correction is added or assumed";
const TDB_AT_OBSERVER_NOTE: &str = "TIMESYS TDB with no barycentric TREFPOS, TIMEREF or BJDREFI: read as TDB at the observer (FITS 4.0 Table 32 pairs TDB with BARYCENTER), so the light-time term is added";
const BJD_REFERENCE_INTEGER_KEY: &str = "BJDREFI";
const BJD_REFERENCE_FRACTION_KEY: &str = "BJDREFF";
const TIME_START_KEY: &str = "TSTART";
const TIME_STOP_KEY: &str = "TSTOP";
const TELESCOP_KEY: &str = "TELESCOP";
const IMAGE_CENTRE_LABEL: &str = "image centre";
const REFERENCE_FRAME_SUFFIX: &str = " on the reference frame";
const POLE_COS_LATITUDE_LIMIT: f64 = 1e-12;
const NO_TARGET_LIGHT_TIME_NOTE: &str = "no target coordinates: BJD_TDB and HJD_UTC need a WCS, RA/DEC keywords or a target override";
const NO_TARGET_HORIZONTAL_NOTE: &str = "no target coordinates: hour angle, altitude, azimuth, airmass, parallactic angle and the Moon separation need a target";
const NO_SITE_NOTE: &str = "no observatory location: local sidereal time, hour angle, altitude, azimuth, airmass, parallactic angle and the Sun and Moon altitude need a site";
const GEOCENTRIC_MOON_SEPARATION_NOTE: &str = "Moon separation from the geocentric Moon (no site for the topocentric one)";
const BARYCENTRIC_TIME_SITE_REASON: &str = "the header time is barycentric and JD_UTC still carries the light-time term (up to 8.3 min)";
const TOPOCENTRIC_METHOD_NOTE: &str = "topocentric block: mean equinox of date (Meeus ch. 21 precession, nutation omitted), unrefracted altitude, UTC used as UT1, airmass Kasten & Young 1989";
const LIGHT_TIME_METHOD_NOTE: &str = "light time: Meeus low-precision Earth with the Moon offset, Jupiter-Neptune circular reflex for the Sun; Shapiro and observer-offset terms omitted (< 0.03 s)";
const MOON_METHOD_NOTE: &str = "Moon: Meeus ch. 47 truncated series (about 0.3 deg), topocentric altitude, illumination from the phase angle (Meeus 48.3)";

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn card_string(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn card_string_upper(header: &HduHeader, key: &str) -> Option<String> {
    card_string(header, key).map(|s| s.to_uppercase())
}

fn finite_card(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|v| v.is_finite())
}

pub fn tai_minus_utc_seconds(jd_utc: f64) -> (f64, bool) {
    let mut found = None;
    for (year, month, offset_seconds) in LEAP_SECONDS {
        if jd_from_gregorian(year, month, 1.0) <= jd_utc {
            found = Some(offset_seconds);
        } else {
            break;
        }
    }
    match found {
        Some(offset_seconds) => (offset_seconds, false),
        None => (TAI_MINUS_UTC_BEFORE_TABLE_SECONDS, true),
    }
}

pub fn jd_tt_from_utc(jd_utc: f64) -> f64 {
    let (tai_minus_utc, _) = tai_minus_utc_seconds(jd_utc);
    jd_utc + (tai_minus_utc + TT_MINUS_TAI_SECONDS) / SECONDS_PER_DAY
}

pub fn jd_utc_from_tt(jd_tt: f64) -> f64 {
    let first_guess = jd_tt - (TT_MINUS_TAI_SECONDS + tai_minus_utc_seconds(jd_tt).0) / SECONDS_PER_DAY;
    jd_tt - (TT_MINUS_TAI_SECONDS + tai_minus_utc_seconds(first_guess).0) / SECONDS_PER_DAY
}

pub fn tdb_minus_tt_seconds(jd_tt: f64) -> f64 {
    let days = jd_tt - JD_J2000;
    let g = (357.53 + 0.9856003 * days).to_radians();
    let l_minus_l_jupiter = (246.11 + 0.90251792 * days).to_radians();
    0.001657 * g.sin() + 0.000022 * l_minus_l_jupiter.sin()
}

pub fn jd_tdb_from_tt(jd_tt: f64) -> f64 {
    jd_tt + tdb_minus_tt_seconds(jd_tt) / SECONDS_PER_DAY
}

fn jd_tt_from_tcg(jd_tcg: f64) -> f64 {
    jd_tcg - IAU_L_G * (jd_tcg - IAU_T0_JD)
}

fn jd_tdb_from_tcb(jd_tcb: f64) -> f64 {
    jd_tcb - IAU_L_B * (jd_tcb - IAU_T0_JD) + IAU_TDB0_SECONDS / SECONDS_PER_DAY
}

pub fn mean_obliquity_deg(jd_tt: f64) -> f64 {
    let t = julian_centuries_j2000(jd_tt);
    (84381.448 - 46.8150 * t - 0.00059 * t * t + 0.001813 * t * t * t) / 3600.0
}

pub fn precession_matrix_j2000_to_date(jd_tt: f64) -> Mat3 {
    let t = julian_centuries_j2000(jd_tt);
    let zeta = (2306.2181 * t + 0.30188 * t * t + 0.017998 * t * t * t) / 3600.0;
    let z = (2306.2181 * t + 1.09468 * t * t + 0.018203 * t * t * t) / 3600.0;
    let theta = (2004.3109 * t - 0.42665 * t * t - 0.041833 * t * t * t) / 3600.0;
    mat_mul(&mat_mul(&rot_z(-z), &rot_y(theta)), &rot_z(-zeta))
}

pub fn precess_j2000_to_date(v: [f64; 3], jd_tt: f64) -> [f64; 3] {
    mat_apply(&precession_matrix_j2000_to_date(jd_tt), v)
}

fn ecliptic_of_date_to_equatorial_of_date(v: [f64; 3], jd_tt: f64) -> [f64; 3] {
    mat_apply(&rot_x(-mean_obliquity_deg(jd_tt)), v)
}

pub fn local_sidereal_time_deg(jd_ut: f64, lon_east_deg: f64) -> f64 {
    (gmst_deg(jd_ut) + lon_east_deg).rem_euclid(360.0)
}

pub fn hour_angle_deg(lst_deg: f64, ra_deg: f64) -> f64 {
    180.0 - (ra_deg - lst_deg + 180.0).rem_euclid(360.0)
}

pub fn horizontal_coordinates(hour_angle_deg: f64, dec_deg: f64, lat_deg: f64) -> (f64, f64) {
    let (sin_h, cos_h) = hour_angle_deg.to_radians().sin_cos();
    let (sin_dec, cos_dec) = dec_deg.to_radians().sin_cos();
    let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
    let altitude = (sin_lat * sin_dec + cos_lat * cos_dec * cos_h).clamp(-1.0, 1.0).asin().to_degrees();
    let azimuth_from_south = (sin_h * cos_dec).atan2(cos_h * sin_lat * cos_dec - sin_dec * cos_lat).to_degrees();
    (altitude, (azimuth_from_south + 180.0).rem_euclid(360.0))
}

pub fn kasten_young_airmass(altitude_deg: f64) -> Option<f64> {
    if !altitude_deg.is_finite() || altitude_deg < 0.0 {
        return None;
    }
    let denominator = altitude_deg.to_radians().sin() + 0.50572 * (altitude_deg + 6.07995).powf(-1.6364);
    (denominator.is_finite() && denominator > 0.0).then(|| 1.0 / denominator)
}

pub fn parallactic_angle_deg(hour_angle_deg: f64, dec_deg: f64, lat_deg: f64) -> Option<f64> {
    let (sin_h, cos_h) = hour_angle_deg.to_radians().sin_cos();
    let (sin_dec, cos_dec) = dec_deg.to_radians().sin_cos();
    let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
    if cos_lat.abs() < POLE_COS_LATITUDE_LIMIT {
        return None;
    }
    Some(sin_h.atan2((sin_lat / cos_lat) * cos_dec - sin_dec * cos_h).to_degrees())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightTimeCentre {
    Heliocentre,
    Barycentre,
}

pub fn romer_delay_seconds(jd_tt: f64, ra_deg: f64, dec_deg: f64, centre: LightTimeCentre) -> f64 {
    let direction = target_unit_vector(ra_deg, dec_deg);
    let earth = earth_heliocentric_position_au(jd_tt);
    let observer = match centre {
        LightTimeCentre::Heliocentre => earth,
        LightTimeCentre::Barycentre => add(earth, sun_barycentric_position_au(jd_tt)),
    };
    dot(observer, direction) * AU_LIGHT_SECONDS
}

pub fn sun_equatorial_of_date(jd_tt: f64) -> (f64, f64, f64) {
    let (longitude, radius_au) = sun_geometric_longitude_deg(jd_tt);
    let (ra, dec) = cartesian_to_spherical(ecliptic_of_date_to_equatorial_of_date(spherical_to_cartesian(longitude, 0.0), jd_tt));
    (ra, dec, radius_au)
}

pub fn moon_equatorial_of_date_au(jd_tt: f64) -> [f64; 3] {
    ecliptic_of_date_to_equatorial_of_date(moon_geocentric_ecliptic_au(jd_tt), jd_tt)
}

pub fn observer_geocentric_au(site: &SiteLocation, lst_deg: f64) -> [f64; 3] {
    let lat = site.lat_deg.to_radians();
    let e2 = 2.0 * WGS84_F - WGS84_F * WGS84_F;
    let n = WGS84_A_M / (1.0 - e2 * lat.sin() * lat.sin()).sqrt();
    let axis_distance_m = (n + site.height_m) * lat.cos();
    let polar_m = (n * (1.0 - e2) + site.height_m) * lat.sin();
    let (sin_lst, cos_lst) = lst_deg.to_radians().sin_cos();
    let au_m = AU_KM * 1000.0;
    [axis_distance_m * cos_lst / au_m, axis_distance_m * sin_lst / au_m, polar_m / au_m]
}

pub fn moon_illuminated_fraction(jd_tt: f64) -> f64 {
    let moon = moon_geocentric_ecliptic_au(jd_tt);
    let moon_distance = norm(moon);
    let (sun_longitude, sun_distance) = sun_geometric_longitude_deg(jd_tt);
    let sun = spherical_to_cartesian(sun_longitude, 0.0);
    let elongation = (dot(moon, sun) / moon_distance).clamp(-1.0, 1.0).acos();
    let phase_angle = (sun_distance * elongation.sin()).atan2(moon_distance - sun_distance * elongation.cos());
    (1.0 + phase_angle.cos()) / 2.0
}

#[derive(Debug, Clone)]
pub struct HeaderTime {
    pub jd_header: f64,
    pub scale: String,
    pub jd_utc: f64,
    pub jd_tt: f64,
    pub jd_tdb: f64,
    pub bjd_header: Option<f64>,
    pub source: String,
    pub notes: Vec<String>,
}

fn trefpos_is_barycentric(value: &str) -> Option<bool> {
    match value.get(..TREFPOS_SIGNIFICANT_CHARS)? {
        "TOP" | "GEO" => Some(false),
        "BAR" => Some(true),
        _ => None,
    }
}

fn timeref_is_barycentric(value: &str) -> Option<bool> {
    match value {
        "LOCAL" | "GEOCENTRIC" => Some(false),
        _ => value.starts_with(TIMEREF_BARYCENTRIC_PREFIX).then_some(true),
    }
}

fn reference_claim(header: &HduHeader, key: &str, is_barycentric: fn(&str) -> Option<bool>) -> Result<Option<(String, bool)>, String> {
    let Some(value) = card_string_upper(header, key) else {
        return Ok(None);
    };
    let barycentric = is_barycentric(&value).ok_or_else(|| {
        format!("{key} {value} is not a supported time reference ({SUPPORTED_TIME_REFERENCES}): the header time is not used, so no light-time correction is added or assumed")
    })?;
    Ok(Some((format!("{key} {value}"), barycentric)))
}

fn barycentric_reference(header: &HduHeader) -> Result<Option<String>, String> {
    let bjd_reference = header.get(BJD_REFERENCE_INTEGER_KEY).is_some().then(|| (BJD_REFERENCE_INTEGER_KEY.to_string(), true));
    let claims: Vec<(String, bool)> = [
        reference_claim(header, TREFPOS_KEY, trefpos_is_barycentric)?,
        reference_claim(header, TIMEREF_KEY, timeref_is_barycentric)?,
        bjd_reference,
    ]
    .into_iter()
    .flatten()
    .collect();
    let barycentric = claims.iter().filter(|(_, barycentric)| *barycentric).count();
    if barycentric > 0 && barycentric < claims.len() {
        let described: Vec<String> = claims
            .iter()
            .map(|(claim, barycentric)| format!("{claim} {}", if *barycentric { "at the barycentre" } else { "at the observer" }))
            .collect();
        return Err(format!(
            "time reference keywords disagree ({}): the header time is not used, so no light-time correction is added or assumed",
            described.join(", ")
        ));
    }
    Ok((barycentric > 0).then(|| claims.into_iter().map(|(claim, _)| claim).collect::<Vec<_>>().join(", ")))
}

fn barycentric_header_note(reference: &str) -> String {
    format!("header time is barycentric ({reference}): reported as BJD_TDB, no light-time term added; JD_UTC derived from it without removing the light-time term (up to 8.3 min)")
}

fn no_time_reason(problems: &[String]) -> String {
    std::iter::once(NO_TIME_REASON.to_string()).chain(problems.iter().cloned()).collect::<Vec<_>>().join("; ")
}

fn barycentric_reference_mid_jd(header: &HduHeader) -> Option<f64> {
    let integer = finite_card(header, BJD_REFERENCE_INTEGER_KEY)?;
    let fraction = finite_card(header, BJD_REFERENCE_FRACTION_KEY).unwrap_or(0.0);
    let start = finite_card(header, TIME_START_KEY)?;
    let stop = finite_card(header, TIME_STOP_KEY)?;
    Some(integer + fraction + (start + stop) / 2.0)
}

fn scale_rule_note(timesys: Option<&str>, scale: &str, jd_header: f64, jd_utc: f64, jd_tt: f64) -> String {
    let (tai_minus_utc, _) = tai_minus_utc_seconds(jd_utc);
    let tdb_minus_tt = tdb_minus_tt_seconds(jd_tt);
    let label = match timesys {
        None => "TIMESYS absent: UTC assumed".to_string(),
        Some(name) if scale == SCALE_UTC => format!("TIMESYS {name}: UTC"),
        Some(name) => format!("TIMESYS {name}: header time is {scale}"),
    };
    let utc_from_tt = format!("UTC = TT - {TT_MINUS_TAI_SECONDS:.3} s - {tai_minus_utc:.3} s");
    match scale {
        SCALE_TT => format!("{label}; {utc_from_tt}; TDB - TT = {tdb_minus_tt:+.4} s"),
        SCALE_TAI => format!("{label}; TT = TAI + {TT_MINUS_TAI_SECONDS:.3} s; UTC = TAI - {tai_minus_utc:.3} s; TDB - TT = {tdb_minus_tt:+.4} s"),
        SCALE_GPS => format!("{label}; TT = GPS + {TAI_MINUS_GPS_SECONDS:.3} s + {TT_MINUS_TAI_SECONDS:.3} s; {utc_from_tt}; TDB - TT = {tdb_minus_tt:+.4} s"),
        SCALE_TCG => {
            let offset = (jd_header - jd_tt) * SECONDS_PER_DAY;
            format!("{label}; TT = TCG - L_G (JD - T0) = TCG - {offset:.4} s (IAU 2000 B1.9); {utc_from_tt}; TDB - TT = {tdb_minus_tt:+.4} s")
        }
        SCALE_TDB => format!("{label}; TT = TDB - ({tdb_minus_tt:+.4} s); {utc_from_tt}"),
        SCALE_TCB => {
            let offset = (jd_header - jd_tdb_from_tcb(jd_header)) * SECONDS_PER_DAY;
            format!("{label}; TDB = TCB - L_B (JD - T0) + TDB0 = TCB - {offset:.4} s (IAU 2006 B3); TT = TDB - ({tdb_minus_tt:+.4} s); {utc_from_tt}")
        }
        _ => format!("{label}; TT = UTC + {tai_minus_utc:.3} s + {TT_MINUS_TAI_SECONDS:.3} s; TDB - TT = {tdb_minus_tt:+.4} s"),
    }
}

pub fn header_time(header: &HduHeader) -> Result<HeaderTime, String> {
    let mut notes = invalid_time_cards(header);
    let mid_exposure = mid_exposure_jd(header).filter(|(jd, _)| jd.is_finite());
    let barycentric_mid = barycentric_reference_mid_jd(header);
    if mid_exposure.is_none() && barycentric_mid.is_none() {
        return Err(no_time_reason(&notes));
    }
    let barycentric = barycentric_reference(header)?;
    let timesys = card_string_upper(header, TIMESYS_KEY);
    let Some((jd, source)) = mid_exposure else {
        let (Some(bjd), Some(reference)) = (barycentric_mid, barycentric.as_deref()) else {
            return Err(no_time_reason(&notes));
        };
        let jd_tt = bjd - tdb_minus_tt_seconds(bjd) / SECONDS_PER_DAY;
        let jd_utc = jd_utc_from_tt(jd_tt);
        notes.push(scale_rule_note(Some(SCALE_TDB), SCALE_TDB, bjd, jd_utc, jd_tt));
        if let Some(name) = timesys.as_deref().filter(|name| *name != SCALE_TDB) {
            notes.push(format!("TIMESYS {name} ignored: a BJDREF time is TDB by definition"));
        }
        notes.push(barycentric_header_note(reference));
        return Ok(HeaderTime {
            jd_header: bjd,
            scale: SCALE_TDB.to_string(),
            jd_utc,
            jd_tt,
            jd_tdb: bjd,
            bjd_header: Some(bjd),
            source: BARYCENTRIC_TIME_SOURCE.to_string(),
            notes,
        });
    };
    let (scale, jd_tt) = match timesys.as_deref() {
        None | Some(SCALE_UTC | "UT" | "UT1" | "GMT") => (SCALE_UTC, jd_tt_from_utc(jd)),
        Some(SCALE_TT | "TDT" | "ET") => (SCALE_TT, jd),
        Some(SCALE_TAI | "IAT") => (SCALE_TAI, jd + TT_MINUS_TAI_SECONDS / SECONDS_PER_DAY),
        Some(SCALE_GPS) => (SCALE_GPS, jd + (TAI_MINUS_GPS_SECONDS + TT_MINUS_TAI_SECONDS) / SECONDS_PER_DAY),
        Some(SCALE_TCG) => (SCALE_TCG, jd_tt_from_tcg(jd)),
        Some(SCALE_TDB) => (SCALE_TDB, jd - tdb_minus_tt_seconds(jd) / SECONDS_PER_DAY),
        Some(SCALE_TCB) => {
            let jd_tdb = jd_tdb_from_tcb(jd);
            (SCALE_TCB, jd_tdb - tdb_minus_tt_seconds(jd_tdb) / SECONDS_PER_DAY)
        }
        Some(other) => {
            notes.push(format!("TIMESYS {other} unknown: treated as UTC"));
            (SCALE_UTC, jd_tt_from_utc(jd))
        }
    };
    if barycentric.is_none() {
        match scale {
            SCALE_TCB => return Err(TCB_AT_OBSERVER.to_string()),
            SCALE_TDB => notes.push(TDB_AT_OBSERVER_NOTE.to_string()),
            _ => {}
        }
    }
    let jd_tdb = match scale {
        SCALE_TDB => jd,
        SCALE_TCB => jd_tdb_from_tcb(jd),
        _ => jd_tdb_from_tt(jd_tt),
    };
    let jd_utc = if scale == SCALE_UTC { jd } else { jd_utc_from_tt(jd_tt) };
    notes.push(scale_rule_note(timesys.as_deref(), scale, jd, jd_utc, jd_tt));
    if tai_minus_utc_seconds(jd_utc).1 {
        notes.push(BEFORE_TABLE_NOTE.to_string());
    }
    if let Some(reference) = barycentric.as_deref() {
        if barycentric_mid.is_none() && !BARYCENTRIC_SCALES.contains(&scale) {
            let named = timesys.as_deref().map_or_else(|| "TIMESYS absent".to_string(), |name| format!("TIMESYS {name}"));
            notes.push(format!(
                "{named} is not a barycentric time scale (FITS 4.0 Table 32 pairs BARYCENTER with TDB or TCB): the header time is read as BJD in {scale} and converted to BJD_TDB"
            ));
        }
        notes.push(barycentric_header_note(reference));
    }
    Ok(HeaderTime {
        jd_header: jd,
        scale: scale.to_string(),
        jd_utc,
        jd_tt,
        jd_tdb,
        bjd_header: barycentric.is_some().then(|| barycentric_mid.unwrap_or(jd_tdb)),
        source,
        notes,
    })
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GeometryOverrides {
    pub target_ra: Option<f64>,
    pub target_dec: Option<f64>,
    pub site_lat: Option<f64>,
    pub site_lon: Option<f64>,
    pub site_height: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedTarget {
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedSite {
    #[serde(flatten)]
    pub site: SiteLocation,
    pub source: String,
}

fn resolve_target_from(
    path: &str,
    header: &HduHeader,
    wcs_pixel: Option<(f64, f64)>,
    wcs_label: &str,
    wcs_suffix: &str,
    overrides: &GeometryOverrides,
) -> Option<ResolvedTarget> {
    if let (Some(ra_deg), Some(dec_deg)) = (overrides.target_ra, overrides.target_dec) {
        return Some(ResolvedTarget { ra_deg, dec_deg, source: SOURCE_USER.to_string() });
    }
    if let Some((x, y)) = wcs_pixel {
        if let Ok(wcs) = load_wcs(path, header) {
            let coord = wcs.pixel_to_world(x, y);
            if coord.ra.is_finite() && coord.dec.is_finite() {
                return Some(ResolvedTarget {
                    ra_deg: coord.ra,
                    dec_deg: coord.dec,
                    source: format!("WCS at {wcs_label} ({x:.1}, {y:.1}){wcs_suffix}"),
                });
            }
        }
    }
    header_target_coordinates(header).map(|(ra_deg, dec_deg, source)| ResolvedTarget { ra_deg, dec_deg, source: source.to_string() })
}

pub fn resolve_target(
    path: &str,
    header: &HduHeader,
    target_pixel: Option<(f64, f64)>,
    overrides: &GeometryOverrides,
) -> Option<ResolvedTarget> {
    resolve_target_from(path, header, target_pixel, IMAGE_CENTRE_LABEL, "", overrides)
}

pub fn resolve_series_target(
    reference_path: &str,
    reference: &HduHeader,
    target_star: Option<(f64, f64, &str)>,
    overrides: &GeometryOverrides,
) -> Option<ResolvedTarget> {
    match target_star {
        Some((x, y, label)) => {
            resolve_target_from(reference_path, reference, Some((x, y)), label, REFERENCE_FRAME_SUFFIX, overrides)
        }
        None => resolve_target_from(reference_path, reference, None, IMAGE_CENTRE_LABEL, "", overrides),
    }
}

pub fn resolve_site(header: &HduHeader, overrides: &GeometryOverrides) -> Option<ResolvedSite> {
    if let (Some(lat_deg), Some(lon_deg)) = (overrides.site_lat, overrides.site_lon) {
        let height_m = overrides.site_height.unwrap_or(0.0);
        return Some(ResolvedSite { site: SiteLocation { lon_deg, lat_deg, height_m }, source: SOURCE_USER.to_string() });
    }
    site_location(header).map(|(site, source)| ResolvedSite { site, source: source.to_string() })
}

pub fn image_centre_pixel(header: &HduHeader) -> Option<(f64, f64)> {
    let compressed = card_string_upper(header, "ZIMAGE").is_some_and(|v| v == "T");
    let (key1, key2) = if compressed { ("ZNAXIS1", "ZNAXIS2") } else { ("NAXIS1", "NAXIS2") };
    let dimension = |key: &str| header.get_i64(key).filter(|n| *n > 0).and_then(|n| usize::try_from(n).ok());
    Some(pixel_center(dimension(key1)?, dimension(key2)?))
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameGeometry {
    pub jd_utc: Option<f64>,
    pub jd_tt: Option<f64>,
    pub jd_tdb: Option<f64>,
    pub bjd_tdb: Option<f64>,
    pub hjd_utc: Option<f64>,
    pub bjd_source: Option<String>,
    pub lst_deg: Option<f64>,
    pub hour_angle_deg: Option<f64>,
    pub altitude_deg: Option<f64>,
    pub azimuth_deg: Option<f64>,
    pub airmass_computed: Option<f64>,
    pub airmass_formula: String,
    pub parallactic_angle_deg: Option<f64>,
    pub sun_altitude_deg: Option<f64>,
    pub moon_altitude_deg: Option<f64>,
    pub moon_illumination: Option<f64>,
    pub moon_separation_deg: Option<f64>,
    pub time_scale_notes: Vec<String>,
}

impl FrameGeometry {
    pub fn without_time(reason: &str) -> Self {
        FrameGeometry {
            jd_utc: None,
            jd_tt: None,
            jd_tdb: None,
            bjd_tdb: None,
            hjd_utc: None,
            bjd_source: None,
            lst_deg: None,
            hour_angle_deg: None,
            altitude_deg: None,
            azimuth_deg: None,
            airmass_computed: None,
            airmass_formula: AIRMASS_FORMULA.to_string(),
            parallactic_angle_deg: None,
            sun_altitude_deg: None,
            moon_altitude_deg: None,
            moon_illumination: None,
            moon_separation_deg: None,
            time_scale_notes: vec![reason.to_string()],
        }
    }
}

struct LightTime {
    bjd_tdb: Option<f64>,
    hjd_utc: Option<f64>,
    bjd_source: Option<String>,
}

fn spacecraft_name(header: &HduHeader) -> Option<String> {
    card_string_upper(header, TELESCOP_KEY).filter(|telescop| is_spacecraft(telescop))
}

enum EarthBoundSite<'a> {
    Known(&'a ResolvedSite),
    Unknown,
    OffEarth,
}

fn earth_bound_site<'a>(
    header: &HduHeader,
    time: &HeaderTime,
    site: Option<&'a ResolvedSite>,
    notes: &mut Vec<String>,
) -> EarthBoundSite<'a> {
    let reason = match spacecraft_name(header) {
        Some(telescop) => Some(format!("TELESCOP {telescop} is a spacecraft")),
        None => time.bjd_header.is_some().then(|| BARYCENTRIC_TIME_SITE_REASON.to_string()),
    };
    match (reason, site) {
        (None, None) => {
            notes.push(NO_SITE_NOTE.to_string());
            EarthBoundSite::Unknown
        }
        (None, Some(resolved)) => EarthBoundSite::Known(resolved),
        (Some(reason), Some(resolved)) => {
            notes.push(format!("site from {} not applied: {reason}; no topocentric block or Moon separation", resolved.source));
            EarthBoundSite::OffEarth
        }
        (Some(reason), None) => {
            notes.push(format!("no topocentric block or Moon separation: {reason}"));
            EarthBoundSite::OffEarth
        }
    }
}

fn light_time(header: &HduHeader, time: &HeaderTime, target: Option<&ResolvedTarget>, notes: &mut Vec<String>) -> LightTime {
    if let Some(bjd) = time.bjd_header {
        return LightTime { bjd_tdb: Some(bjd), hjd_utc: None, bjd_source: Some(BJD_SOURCE_HEADER.to_string()) };
    }
    if let Some(telescop) = spacecraft_name(header) {
        notes.push(format!("TELESCOP {telescop} is a spacecraft: the light-time correction needs its ephemeris"));
        return LightTime { bjd_tdb: None, hjd_utc: None, bjd_source: None };
    }
    let Some(target) = target else {
        notes.push(NO_TARGET_LIGHT_TIME_NOTE.to_string());
        return LightTime { bjd_tdb: None, hjd_utc: None, bjd_source: None };
    };
    let heliocentric = romer_delay_seconds(time.jd_tt, target.ra_deg, target.dec_deg, LightTimeCentre::Heliocentre);
    let barycentric = romer_delay_seconds(time.jd_tt, target.ra_deg, target.dec_deg, LightTimeCentre::Barycentre);
    notes.push(format!(
        "BJD_TDB = JD_TDB {barycentric:+.3} s (Roemer delay to the barycentre); HJD_UTC = JD_UTC {heliocentric:+.3} s; {TDB_MINUS_TT_METHOD}"
    ));
    LightTime {
        bjd_tdb: Some(time.jd_tdb + barycentric / SECONDS_PER_DAY),
        hjd_utc: Some(time.jd_utc + heliocentric / SECONDS_PER_DAY),
        bjd_source: Some(BJD_SOURCE_COMPUTED.to_string()),
    }
}

pub fn frame_geometry(
    header: &HduHeader,
    time: &HeaderTime,
    target: Option<&ResolvedTarget>,
    site: Option<&ResolvedSite>,
) -> FrameGeometry {
    let mut notes = vec![format!("time: {}", time.source)];
    notes.extend(time.notes.iter().cloned());
    let light = light_time(header, time, target, &mut notes);
    let target_of_date = target.map(|t| cartesian_to_spherical(precess_j2000_to_date(spherical_to_cartesian(t.ra_deg, t.dec_deg), time.jd_tt)));
    let (sun_ra, sun_dec, _) = sun_equatorial_of_date(time.jd_tt);
    let moon_geocentric = moon_equatorial_of_date_au(time.jd_tt);
    let mut geometry = FrameGeometry {
        jd_utc: Some(time.jd_utc),
        jd_tt: Some(time.jd_tt),
        jd_tdb: Some(time.jd_tdb),
        bjd_tdb: light.bjd_tdb,
        hjd_utc: light.hjd_utc,
        bjd_source: light.bjd_source,
        lst_deg: None,
        hour_angle_deg: None,
        altitude_deg: None,
        azimuth_deg: None,
        airmass_computed: None,
        airmass_formula: AIRMASS_FORMULA.to_string(),
        parallactic_angle_deg: None,
        sun_altitude_deg: None,
        moon_altitude_deg: None,
        moon_illumination: Some(moon_illuminated_fraction(time.jd_tt)),
        moon_separation_deg: None,
        time_scale_notes: Vec::new(),
    };
    match earth_bound_site(header, time, site, &mut notes) {
        EarthBoundSite::Known(resolved) => {
            let lat = resolved.site.lat_deg;
            let lst = local_sidereal_time_deg(time.jd_utc, resolved.site.lon_deg);
            geometry.lst_deg = Some(lst);
            let moon_topocentric = sub(moon_geocentric, observer_geocentric_au(&resolved.site, lst));
            let (moon_ra, moon_dec) = cartesian_to_spherical(moon_topocentric);
            geometry.moon_altitude_deg = Some(horizontal_coordinates(hour_angle_deg(lst, moon_ra), moon_dec, lat).0);
            geometry.sun_altitude_deg = Some(horizontal_coordinates(hour_angle_deg(lst, sun_ra), sun_dec, lat).0);
            match target_of_date {
                Some((ra, dec)) => {
                    let hour_angle = hour_angle_deg(lst, ra);
                    let (altitude, azimuth) = horizontal_coordinates(hour_angle, dec, lat);
                    geometry.hour_angle_deg = Some(hour_angle);
                    geometry.altitude_deg = Some(altitude);
                    geometry.azimuth_deg = Some(azimuth);
                    geometry.airmass_computed = kasten_young_airmass(altitude);
                    geometry.parallactic_angle_deg = parallactic_angle_deg(hour_angle, dec, lat);
                    geometry.moon_separation_deg = Some(angular_separation(moon_ra, moon_dec, ra, dec));
                }
                None => notes.push(NO_TARGET_HORIZONTAL_NOTE.to_string()),
            }
        }
        EarthBoundSite::Unknown => {
            if let Some((ra, dec)) = target_of_date {
                let (moon_ra, moon_dec) = cartesian_to_spherical(moon_geocentric);
                geometry.moon_separation_deg = Some(angular_separation(moon_ra, moon_dec, ra, dec));
                notes.push(GEOCENTRIC_MOON_SEPARATION_NOTE.to_string());
            }
        }
        EarthBoundSite::OffEarth => {}
    }
    geometry.time_scale_notes = notes;
    geometry
}

pub fn geometry_method_notes(header: &HduHeader, target: Option<&ResolvedTarget>, site: Option<&ResolvedSite>) -> Vec<String> {
    let mut notes = Vec::new();
    match target {
        Some(t) => notes.push(format!("target RA {:.6} Dec {:.6} deg (ICRS/J2000) from {}", t.ra_deg, t.dec_deg, t.source)),
        None => notes.push(NO_TARGET_LIGHT_TIME_NOTE.to_string()),
    }
    match site {
        Some(s) => {
            notes.push(format!(
                "site from {}: lon {:.5} lat {:.5} height {:.0} m (east-positive longitude)",
                s.source, s.site.lon_deg, s.site.lat_deg, s.site.height_m
            ));
            if s.source == SOURCE_USER {
                if let Some((header_site, header_source)) = site_location(header) {
                    notes.push(format!(
                        "site override (lat {:.4}, lon {:.4}) replaces the header site from {}: lat {:.4}, lon {:.4}, height {:.0} m",
                        s.site.lat_deg, s.site.lon_deg, header_source, header_site.lat_deg, header_site.lon_deg, header_site.height_m
                    ));
                }
            } else if longitude_sign_is_ambiguous(&s.source) {
                notes.push(format!(
                    "longitude from {} read as east-positive; a west-positive header flips the hour angle, altitude, azimuth, airmass and parallactic angle: enter the site to correct it",
                    s.source
                ));
            }
        }
        None => notes.push(format!("{}: enter the site to get the topocentric block", no_site_in_header_message())),
    }
    notes.push(TOPOCENTRIC_METHOD_NOTE.to_string());
    notes.push(LIGHT_TIME_METHOD_NOTE.to_string());
    notes.push(MOON_METHOD_NOTE.to_string());
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::frames::{ecliptic_j2000_to_icrs, mat_transpose};
    use crate::core::astrometry::spectral::sun_geometric_longitude_deg;
    use crate::core::astrometry::time::{jd_from_gregorian, SECONDS_PER_DAY};
    use crate::core::imaging::region::test_support::{header_with_cd, make_header, north_up_cd};

    const ONE_TENTH_MS_DAYS: f64 = 1e-4 / 86400.0;

    fn seconds(days: f64) -> f64 {
        days * SECONDS_PER_DAY
    }

    #[test]
    fn leap_second_table_steps_at_the_published_boundaries() {
        assert_eq!(tai_minus_utc_seconds(jd_from_gregorian(1971, 12, 31.0)), (10.0, true));
        assert_eq!(tai_minus_utc_seconds(jd_from_gregorian(1972, 1, 1.0)), (10.0, false));
        assert_eq!(tai_minus_utc_seconds(jd_from_gregorian(1972, 6, 30.0) + 86399.0 / 86400.0).0, 10.0);
        assert_eq!(tai_minus_utc_seconds(jd_from_gregorian(1972, 7, 1.0)).0, 11.0);
        assert_eq!(tai_minus_utc_seconds(jd_from_gregorian(2016, 12, 31.0) + 86399.0 / 86400.0).0, 36.0);
        assert_eq!(tai_minus_utc_seconds(jd_from_gregorian(2017, 1, 1.0)).0, 37.0);
        assert_eq!(tai_minus_utc_seconds(jd_from_gregorian(2026, 9, 25.0)), (37.0, false));
        let jd = jd_from_gregorian(2026, 9, 25.5);
        assert!((seconds(jd_tt_from_utc(jd) - jd) - 69.184).abs() < 1e-4);
    }

    #[test]
    fn utc_to_tt_and_back_round_trips_within_a_tenth_of_a_millisecond() {
        for (year, month, _) in LEAP_SECONDS {
            for offset in [-0.5, 0.5] {
                let u = jd_from_gregorian(year, month, 1.0) + offset;
                let back = jd_utc_from_tt(jd_tt_from_utc(u));
                assert!((back - u).abs() < ONE_TENTH_MS_DAYS, "{year}-{month} {offset}: {} s", seconds(back - u));
            }
        }
        let u = jd_from_gregorian(2017, 1, 1.0) - 30.0 / 86400.0;
        let tt = jd_tt_from_utc(u);
        assert!((seconds(tt - u) - 68.184).abs() < 1e-4, "{}", seconds(tt - u));
        assert!((jd_utc_from_tt(tt) - u).abs() < ONE_TENTH_MS_DAYS, "{} s", seconds(jd_utc_from_tt(tt) - u));
    }

    #[test]
    fn tdb_minus_tt_stays_within_1_7_ms_and_reaches_1_6_ms_over_a_year() {
        let start = jd_from_gregorian(2026, 1, 1.0);
        let mut max = 0.0_f64;
        for day in 0..365 {
            let x = tdb_minus_tt_seconds(start + day as f64);
            assert!(x.abs() <= 0.00172, "day {day}: {x}");
            max = max.max(x.abs());
        }
        assert!(max >= 0.0016, "{max}");
    }

    #[test]
    fn meeus_example_22a_mean_obliquity_on_1987_april_10() {
        assert!((mean_obliquity_deg(2446895.5) - 23.4409464).abs() < 1e-5);
        assert!((mean_obliquity_deg(2451545.0) - 23.4392911).abs() < 1e-7);
    }

    #[test]
    fn meeus_example_21b_theta_persei_precesses_from_j2000_to_2028_november_13() {
        let v = precess_j2000_to_date(spherical_to_cartesian(41.0540625, 49.2277500), 2462088.69);
        let (ra, dec) = cartesian_to_spherical(v);
        assert!((ra - 41.547214).abs() < 0.0005, "{ra}");
        assert!((dec - 49.348483).abs() < 0.0005, "{dec}");
    }

    #[test]
    fn precession_moves_the_j2000_equinox_by_the_annual_rates_after_26_years() {
        let (ra, dec) = cartesian_to_spherical(precess_j2000_to_date([1.0, 0.0, 0.0], jd_from_gregorian(2026, 1, 1.5)));
        assert!((ra - 0.333).abs() < 0.003, "{ra}");
        assert!((dec - 0.145).abs() < 0.003, "{dec}");
    }

    #[test]
    fn meeus_examples_12a_and_12b_local_sidereal_time_at_greenwich() {
        let midnight = jd_from_gregorian(1987, 4, 10.0);
        assert!((local_sidereal_time_deg(midnight, 0.0) - 197.693195).abs() < 1e-4);
        let evening = midnight + (19.0 + 21.0 / 60.0) / 24.0;
        assert!((local_sidereal_time_deg(evening, 0.0) - 128.737873).abs() < 1e-4);
    }

    #[test]
    fn meeus_example_13b_venus_from_the_usno_is_at_altitude_15_1249_and_azimuth_248_0337() {
        let (altitude, azimuth) = horizontal_coordinates(64.352133, -6.719892, 38.921389);
        assert!((altitude - 15.1249).abs() < 0.001, "{altitude}");
        assert!((azimuth - 248.0337).abs() < 0.001, "{azimuth}");
    }

    #[test]
    fn hour_angle_wraps_into_the_half_open_range() {
        assert!((hour_angle_deg(10.0, 350.0) - 20.0).abs() < 1e-9);
        assert!((hour_angle_deg(350.0, 10.0) + 20.0).abs() < 1e-9);
        assert!((hour_angle_deg(0.0, 180.0) - 180.0).abs() < 1e-9);
        assert!(hour_angle_deg(0.0, 0.0).abs() < 1e-9);
        assert!((hour_angle_deg(180.0, 0.0) - 180.0).abs() < 1e-9);
        for lst in 0..36 {
            for ra in 0..36 {
                let h = hour_angle_deg(lst as f64 * 10.0, ra as f64 * 10.0);
                assert!(h > -180.0 && h <= 180.0, "{h}");
            }
        }
    }

    #[test]
    fn kasten_young_airmass_is_one_at_the_zenith_and_about_38_at_the_horizon() {
        assert!((kasten_young_airmass(90.0).unwrap() - 1.0).abs() < 0.001);
        assert!((kasten_young_airmass(0.0).unwrap() - 37.9).abs() < 0.5);
        assert!((kasten_young_airmass(30.0).unwrap() - 1.994).abs() < 0.002);
        assert_eq!(kasten_young_airmass(-1.0), None);
        assert_eq!(kasten_young_airmass(f64::NAN), None);
    }

    #[test]
    fn parallactic_angle_is_zero_at_transit_south_of_the_zenith_and_180_north_of_it() {
        assert!(parallactic_angle_deg(0.0, 20.0, 40.0).unwrap().abs() < 1e-9);
        assert!((parallactic_angle_deg(0.0, 60.0, 40.0).unwrap().abs() - 180.0).abs() < 1e-9);
        let west = parallactic_angle_deg(30.0, 0.0, 40.0).unwrap();
        assert!(west > 0.0, "{west}");
        assert!((parallactic_angle_deg(-30.0, 0.0, 40.0).unwrap() + west).abs() < 1e-9);
        assert_eq!(parallactic_angle_deg(30.0, 0.0, 90.0), None);
    }

    #[test]
    fn romer_delay_toward_the_ecliptic_pole_does_not_depend_on_the_date() {
        let dates = [(1, 1.0), (4, 1.0), (7, 1.0), (10, 1.0)];
        for centre in [LightTimeCentre::Heliocentre, LightTimeCentre::Barycentre] {
            let delays: Vec<f64> = dates
                .iter()
                .map(|&(month, day)| romer_delay_seconds(jd_from_gregorian(2026, month, day), 270.0, 66.5607206, centre))
                .collect();
            let max = delays.iter().cloned().fold(f64::MIN, f64::max);
            let min = delays.iter().cloned().fold(f64::MAX, f64::min);
            assert!(max - min < 0.02, "{delays:?}");
            assert!(delays.iter().all(|d| d.abs() < 0.02), "{delays:?}");
        }
    }

    #[test]
    fn romer_delay_on_the_ecliptic_is_plus_and_minus_499_seconds_at_opposition_and_conjunction() {
        let jd_tt = jd_from_gregorian(2026, 3, 20.5);
        let (lambda, r) = sun_geometric_longitude_deg(jd_tt);
        let (ra_opp, dec_opp) = ecliptic_j2000_to_icrs(lambda + 180.0, 0.0);
        let helio = romer_delay_seconds(jd_tt, ra_opp, dec_opp, LightTimeCentre::Heliocentre);
        let bary = romer_delay_seconds(jd_tt, ra_opp, dec_opp, LightTimeCentre::Barycentre);
        assert!((helio / (499.005 * r) - 1.0).abs() < 0.01, "{helio}");
        assert!((bary - helio).abs() <= 6.0, "{bary} {helio}");
        let (ra_conj, dec_conj) = ecliptic_j2000_to_icrs(lambda, 0.0);
        let helio = romer_delay_seconds(jd_tt, ra_conj, dec_conj, LightTimeCentre::Heliocentre);
        let bary = romer_delay_seconds(jd_tt, ra_conj, dec_conj, LightTimeCentre::Barycentre);
        assert!((helio / (-499.005 * r) - 1.0).abs() < 0.01, "{helio}");
        assert!((bary - helio).abs() <= 6.0, "{bary} {helio}");
    }

    #[test]
    fn astropy_docs_ip_peg_from_greenwich_light_travel_time_example() {
        let jd_tt = jd_tt_from_utc(2456326.45833333);
        let bary = romer_delay_seconds(jd_tt, 350.785625, 18.416472, LightTimeCentre::Barycentre);
        let helio = romer_delay_seconds(jd_tt, 350.785625, 18.416472, LightTimeCentre::Heliocentre);
        assert!((bary + 325.86).abs() < 1.0, "{bary}");
        assert!((helio + 325.36).abs() < 1.0, "{helio}");
    }

    #[test]
    fn meeus_example_25a_sun_position_on_1992_october_13() {
        let (ra, dec, r) = sun_equatorial_of_date(2448908.5);
        assert!((ra - 198.38).abs() < 0.02, "{ra}");
        assert!((dec + 7.785).abs() < 0.02, "{dec}");
        assert!((r - 0.99766).abs() < 0.001, "{r}");
    }

    #[test]
    fn meeus_example_47a_moon_position_on_1992_april_12_within_the_truncated_series() {
        let v = moon_equatorial_of_date_au(2448724.5);
        let (ra, dec) = cartesian_to_spherical(v);
        let distance_km = norm(v) * AU_KM;
        assert!((ra - 134.688).abs() < 0.5, "{ra}");
        assert!((dec - 13.768).abs() < 0.3, "{dec}");
        assert!((distance_km / 368410.0 - 1.0).abs() < 0.01, "{distance_km}");
    }

    #[test]
    fn meeus_example_48a_moon_illuminated_fraction_on_1992_april_12() {
        assert!((moon_illuminated_fraction(2448724.5) - 0.6786).abs() < 0.01);
    }

    #[test]
    fn moon_topocentric_altitude_differs_from_geocentric_by_up_to_the_horizontal_parallax() {
        let site = SiteLocation { lon_deg: 0.0, lat_deg: 0.0, height_m: 0.0 };
        let day = jd_from_gregorian(2026, 9, 25.0);
        let mut max = 0.0_f64;
        for hour in 0..24 {
            let jd_utc = day + hour as f64 / 24.0;
            let jd_tt = jd_tt_from_utc(jd_utc);
            let lst = local_sidereal_time_deg(jd_utc, site.lon_deg);
            let geocentric = moon_equatorial_of_date_au(jd_tt);
            let (ra_geo, dec_geo) = cartesian_to_spherical(geocentric);
            let (alt_geo, _) = horizontal_coordinates(hour_angle_deg(lst, ra_geo), dec_geo, site.lat_deg);
            let topocentric = sub(geocentric, observer_geocentric_au(&site, lst));
            let (ra_topo, dec_topo) = cartesian_to_spherical(topocentric);
            let (alt_topo, _) = horizontal_coordinates(hour_angle_deg(lst, ra_topo), dec_topo, site.lat_deg);
            let diff = (alt_topo - alt_geo).abs();
            assert!(diff <= 1.02, "hour {hour}: {diff}");
            max = max.max(diff);
        }
        assert!((0.85..=1.02).contains(&max), "{max}");
    }

    fn timed_header(timesys: Option<&str>) -> HduHeader {
        let mut pairs = vec![("DATE-OBS", "2026-03-21T10:00:00"), ("EXPTIME", "600")];
        if let Some(scale) = timesys {
            pairs.push(("TIMESYS", scale));
        }
        make_header(&pairs)
    }

    #[test]
    fn header_time_converts_utc_tt_tai_and_tdb_scales_and_keeps_the_header_jd() {
        let reference = header_time(&timed_header(None)).unwrap();
        let expected = jd_from_gregorian(2026, 3, 21.0) + 10.0 / 24.0 + 300.0 / SECONDS_PER_DAY;
        assert!((reference.jd_header - expected).abs() < 1e-9);
        assert_eq!(reference.scale, "UTC");
        assert_eq!(reference.jd_utc, reference.jd_header);
        assert!(reference.notes.iter().any(|n| n.contains("TIMESYS absent")), "{:?}", reference.notes);
        let utc = header_time(&timed_header(Some("UTC"))).unwrap();
        assert_eq!(utc.jd_header, reference.jd_header);
        assert_eq!(utc.jd_utc, reference.jd_utc);
        let tt = header_time(&timed_header(Some("TT"))).unwrap();
        assert_eq!(tt.jd_header, reference.jd_header);
        assert_eq!(tt.jd_tt, tt.jd_header);
        assert!((seconds(tt.jd_header - tt.jd_utc) - 69.184).abs() < 1e-4, "{}", seconds(tt.jd_header - tt.jd_utc));
        let tai = header_time(&timed_header(Some("TAI"))).unwrap();
        assert_eq!(tai.jd_header, reference.jd_header);
        assert!((seconds(tai.jd_tt - tai.jd_header) - 32.184).abs() < 1e-4);
        let tdb = header_time(&timed_header(Some("TDB"))).unwrap();
        assert_eq!(tdb.jd_header, reference.jd_header);
        assert!(seconds(tdb.jd_tt - tdb.jd_header).abs() <= 0.0017);
        assert!((tdb.jd_tdb - tdb.jd_header).abs() < 1e-9);
        let unknown = header_time(&timed_header(Some("XYZ"))).unwrap();
        assert_eq!(unknown.jd_header, reference.jd_header);
        assert_eq!(unknown.scale, "UTC");
        assert_eq!(unknown.jd_utc, reference.jd_utc);
        assert!(unknown.notes.iter().any(|n| n.contains("XYZ") && n.contains("unknown")), "{:?}", unknown.notes);
        assert!(unknown.bjd_header.is_none());
    }

    #[test]
    fn tess_style_barycentric_header_is_reported_not_recorrected() {
        let h = make_header(&[
            ("TIMESYS", "TDB"),
            ("TIMEREF", "SOLARSYSTEM"),
            ("BJDREFI", "2457000"),
            ("BJDREFF", "0.0"),
            ("TSTART", "1325.0"),
            ("TSTOP", "1325.02"),
            ("TELESCOP", "TESS"),
            ("RA_OBJ", "100"),
            ("DEC_OBJ", "-20"),
        ]);
        let time = header_time(&h).unwrap();
        assert!(time.source.contains("BJDREF"), "{}", time.source);
        assert!((time.jd_tdb - 2458325.01).abs() < 1e-9, "{}", time.jd_tdb);
        assert!(time.bjd_header.is_some());
        let target = resolve_target("", &h, None, &GeometryOverrides::default()).unwrap();
        assert!((target.ra_deg - 100.0).abs() < 1e-9 && (target.dec_deg + 20.0).abs() < 1e-9);
        assert_eq!(target.source, "RA_OBJ/DEC_OBJ");
        let g = frame_geometry(&h, &time, Some(&target), None);
        assert!((g.bjd_tdb.unwrap() - 2458325.01).abs() < 1e-9);
        assert_eq!(g.bjd_source.as_deref(), Some("header"));
        assert_eq!(g.hjd_utc, None);
        assert_eq!(g.moon_separation_deg, None, "{g:?}");
        assert!(
            g.time_scale_notes.iter().any(|n| n.contains("no topocentric block or Moon separation") && n.contains("barycentric")),
            "{:?}",
            g.time_scale_notes
        );
        let jd_utc = g.jd_utc.expect("JD_UTC derived from the barycentric time");
        assert!((seconds(2458325.01 - jd_utc) - 69.18).abs() < 0.01, "{}", seconds(2458325.01 - jd_utc));
        assert!(g.time_scale_notes.iter().any(|n| n.contains("barycentric")), "{:?}", g.time_scale_notes);
        let mauna_kea = GeometryOverrides { site_lat: Some(19.82), site_lon: Some(-155.47), ..GeometryOverrides::default() };
        let site = resolve_site(&h, &mauna_kea).unwrap();
        let sited = frame_geometry(&h, &time, Some(&target), Some(&site));
        assert_eq!(sited.bjd_tdb, g.bjd_tdb);
        assert_no_topocentric_block(&sited);
        assert!(
            sited.time_scale_notes.iter().any(|n| n.contains("site from user not applied") && n.contains("light-time term")),
            "{:?}",
            sited.time_scale_notes
        );
    }

    const BJD_TOLERANCE_DAYS: f64 = 1e-9;

    fn equator_target() -> ResolvedTarget {
        ResolvedTarget { ra_deg: 0.0, dec_deg: 0.0, source: "audit".to_string() }
    }

    #[test]
    fn audit_probe_a_tdb_header_at_the_barycentre_keeps_its_bjd() {
        let h = make_header(&[("MJD-AVG", "60000"), ("TIMESYS", "TDB"), ("TREFPOS", "BARYCENTER")]);
        let time = header_time(&h).unwrap();
        let g = frame_geometry(&h, &time, Some(&equator_target()), None);
        assert!(barycentric_reference(&h).unwrap().is_some());
        let bjd = g.bjd_tdb.expect("BJD_TDB from the header");
        assert!((bjd - 2460000.5).abs() <= BJD_TOLERANCE_DAYS, "extra correction {} s", seconds(bjd - 2460000.5));
        assert_eq!(g.bjd_source.as_deref(), Some(BJD_SOURCE_HEADER));
        assert_eq!(g.hjd_utc, None);
        assert!(g.time_scale_notes.iter().any(|n| n.contains("barycentric (TREFPOS BARYCENTER)")), "{:?}", g.time_scale_notes);
        assert!(!g.time_scale_notes.iter().any(|n| n.contains("Roemer")), "{:?}", g.time_scale_notes);
    }

    #[test]
    fn an_explicit_topocentre_or_geocentre_gets_the_same_light_time_as_the_default() {
        let cards = [("DATE-OBS", "2026-03-21T10:00:00"), ("EXPTIME", "600"), ("RA_TARG", "180.0"), ("DEC_TARG", "0.0")];
        let geometry_of = |extra: &[(&str, &str)]| {
            let h = make_header(&[&cards[..], extra].concat());
            let target = resolve_target("", &h, None, &GeometryOverrides::default()).unwrap();
            frame_geometry(&h, &header_time(&h).unwrap(), Some(&target), None)
        };
        let default = geometry_of(&[]);
        let jd_utc = jd_from_gregorian(2026, 3, 21.0) + (10.0 * 3600.0 + 300.0) / SECONDS_PER_DAY;
        let jd_tt = jd_tt_from_utc(jd_utc);
        let roemer = romer_delay_seconds(jd_tt, 180.0, 0.0, LightTimeCentre::Barycentre);
        let expected = jd_utc + (37.0 + TT_MINUS_TAI_SECONDS + tdb_minus_tt_seconds(jd_tt) + roemer) / SECONDS_PER_DAY;
        assert!((default.bjd_tdb.unwrap() - expected).abs() <= BJD_TOLERANCE_DAYS, "{} s", seconds(default.bjd_tdb.unwrap() - expected));
        assert_eq!(default.bjd_source.as_deref(), Some(BJD_SOURCE_COMPUTED));
        for extra in [
            [("TREFPOS", "TOPOCENTER")],
            [("TREFPOS", "GEOCENTER")],
            [("TREFPOS", "TOPOCENT")],
            [("TREFPOS", "GEOCENTR")],
            [("TREFPOS", "TOPOCENTRIC")],
            [("TIMEREF", "LOCAL")],
            [("TIMEREF", "GEOCENTRIC")],
        ] {
            let g = geometry_of(&extra);
            assert_eq!((g.bjd_tdb, g.hjd_utc, g.jd_utc), (default.bjd_tdb, default.hjd_utc, default.jd_utc), "{extra:?}");
            assert_eq!(g.bjd_source.as_deref(), Some(BJD_SOURCE_COMPUTED), "{extra:?}");
        }
    }

    #[test]
    fn a_truncated_barycent_reference_keeps_the_header_bjd_like_the_full_spelling() {
        for spelling in ["BARYCENT", "barycentre"] {
            let h = make_header(&[("MJD-AVG", "60000"), ("TIMESYS", "TDB"), ("TREFPOS", spelling)]);
            let g = frame_geometry(&h, &header_time(&h).unwrap(), Some(&equator_target()), None);
            assert!((g.bjd_tdb.unwrap() - 2460000.5).abs() <= BJD_TOLERANCE_DAYS, "{spelling}");
            assert_eq!(g.bjd_source.as_deref(), Some(BJD_SOURCE_HEADER), "{spelling}");
        }
    }

    #[test]
    fn a_coordinate_time_scale_at_the_observer_is_refused_and_tdb_there_is_named() {
        for extra in [&[][..], &[("TREFPOS", "TOPOCENTER")][..]] {
            let h = make_header(&[&[("MJD-AVG", "60000"), ("TIMESYS", "TCB")][..], extra].concat());
            let err = header_time(&h).unwrap_err();
            assert!(err.starts_with("TIMESYS TCB is a barycentric time scale") && err.ends_with("no light-time correction is added or assumed"), "{err}");
        }
        let tdb = make_header(&[("MJD-AVG", "60000"), ("TIMESYS", "TDB")]);
        let time = header_time(&tdb).unwrap();
        assert!(time.notes.iter().any(|n| n.starts_with("TIMESYS TDB with no barycentric TREFPOS, TIMEREF or BJDREFI")), "{:?}", time.notes);
        let g = frame_geometry(&tdb, &time, Some(&equator_target()), None);
        assert_eq!(g.bjd_source.as_deref(), Some(BJD_SOURCE_COMPUTED));
    }

    #[test]
    fn a_legacy_solarsystem_tdb_header_without_bjdref_keeps_its_header_time_as_bjd() {
        let h = make_header(&[("MJD-AVG", "61120.25"), ("TIMESYS", "TDB"), ("TIMEREF", "SOLARSYSTEM")]);
        let time = header_time(&h).unwrap();
        let g = frame_geometry(&h, &time, Some(&equator_target()), None);
        assert!(barycentric_reference(&h).unwrap().is_some());
        assert!((g.bjd_tdb.unwrap() - 2461120.75).abs() <= BJD_TOLERANCE_DAYS);
        assert_eq!(g.bjd_source.as_deref(), Some(BJD_SOURCE_HEADER));
    }

    #[test]
    fn unsupported_or_contradictory_time_references_are_refused_with_the_keywords_named() {
        let base = [("MJD-AVG", "60000"), ("TIMESYS", "TDB")];
        for (cards, named) in [
            (&[("TREFPOS", "HELIOCENTER")][..], "TREFPOS HELIOCENTER is not a supported time reference"),
            (&[("TREFPOS", "RELOCATABLE")][..], "TREFPOS RELOCATABLE is not a supported time reference"),
            (&[("TREFPOS", "'MARS'")][..], "TREFPOS MARS is not a supported time reference"),
            (&[("TIMEREF", "HELIOCENTRIC")][..], "TIMEREF HELIOCENTRIC is not a supported time reference"),
            (&[("TREFPOS", "TOPOCENTER"), ("TIMEREF", "SOLARSYSTEM")][..], "(TREFPOS TOPOCENTER at the observer, TIMEREF SOLARSYSTEM at the barycentre)"),
            (&[("TREFPOS", "BARYCENTER"), ("TIMEREF", "LOCAL")][..], "(TREFPOS BARYCENTER at the barycentre, TIMEREF LOCAL at the observer)"),
            (&[("TREFPOS", "GEOCENTER"), ("BJDREFI", "2457000")][..], "(TREFPOS GEOCENTER at the observer, BJDREFI at the barycentre)"),
        ] {
            let h = make_header(&[&base[..], cards].concat());
            let err = header_time(&h).unwrap_err();
            assert!(err.contains(named) && err.ends_with("no light-time correction is added or assumed"), "{err}");
            assert!(barycentric_reference(&h).is_err(), "{cards:?}");
        }
        let agreeing = make_header(&[
            ("TIMESYS", "TDB"),
            ("TREFPOS", "barycenter"),
            ("TIMEREF", "SOLARSYSTEM"),
            ("BJDREFI", "2457000"),
            ("BJDREFF", "0.5"),
            ("TSTART", "100.0"),
            ("TSTOP", "100.2"),
        ]);
        let time = header_time(&agreeing).unwrap();
        assert!((time.bjd_header.unwrap() - 2457100.6).abs() <= BJD_TOLERANCE_DAYS, "{:?}", time.bjd_header);
        assert!(time.notes.iter().any(|n| n.contains("barycentric (TREFPOS BARYCENTER, TIMEREF SOLARSYSTEM, BJDREFI)")), "{:?}", time.notes);
    }

    #[test]
    fn a_barycentric_time_in_an_earth_time_scale_is_converted_to_tdb_and_reported() {
        let utc = make_header(&[("MJD-AVG", "60000"), ("TREFPOS", "BARYCENTER")]);
        let time = header_time(&utc).unwrap();
        let g = frame_geometry(&utc, &time, Some(&equator_target()), None);
        let bjd = g.bjd_tdb.unwrap();
        assert!((bjd - jd_tdb_from_tt(jd_tt_from_utc(2460000.5))).abs() <= BJD_TOLERANCE_DAYS);
        assert!((seconds(bjd - 2460000.5) - 69.184).abs() < 0.0018, "{} s", seconds(bjd - 2460000.5));
        assert_eq!(g.bjd_source.as_deref(), Some(BJD_SOURCE_HEADER));
        let reported = "TIMESYS absent is not a barycentric time scale (FITS 4.0 Table 32 pairs BARYCENTER with TDB or TCB)";
        assert!(time.notes.iter().any(|n| n.starts_with(reported) && n.contains("BJD in UTC")), "{:?}", time.notes);
        let tt = make_header(&[("MJD-AVG", "60000"), ("TIMESYS", "TT"), ("TIMEREF", "SOLARSYSTEM")]);
        let time = header_time(&tt).unwrap();
        assert!((time.bjd_header.unwrap() - jd_tdb_from_tt(2460000.5)).abs() <= BJD_TOLERANCE_DAYS);
        assert!(time.notes.iter().any(|n| n.starts_with("TIMESYS TT is not a barycentric time scale")), "{:?}", time.notes);
        let tdb = make_header(&[("MJD-AVG", "60000"), ("TIMESYS", "TDB"), ("TREFPOS", "BARYCENTER")]);
        assert!(!header_time(&tdb).unwrap().notes.iter().any(|n| n.contains("Table 32")));
    }

    #[test]
    fn tcb_tcg_gps_and_the_deprecated_scale_names_follow_their_definitions() {
        let at = |timesys: &str, extra: &[(&str, &str)]| {
            header_time(&make_header(&[&[("MJD-AVG", "51544.5"), ("TIMESYS", timesys)][..], extra].concat())).unwrap()
        };
        let tcb = at("TCB", &[("TREFPOS", "BARYCENTER")]);
        assert!((seconds(2451545.0 - tcb.bjd_header.unwrap()) - 11.25379).abs() < 1e-3, "{}", seconds(2451545.0 - tcb.jd_tdb));
        assert_eq!(tcb.bjd_header, Some(tcb.jd_tdb));
        assert!(tcb.notes.iter().any(|n| n.contains("TDB = TCB - L_B (JD - T0) + TDB0 = TCB - 11.2538 s")), "{:?}", tcb.notes);
        assert!(!tcb.notes.iter().any(|n| n.contains("Table 32")), "{:?}", tcb.notes);
        let tcg = at("TCG", &[]);
        assert!((seconds(2451545.0 - tcg.jd_tt) - 0.50583).abs() < 1e-3, "{}", seconds(2451545.0 - tcg.jd_tt));
        let gps = at("GPS", &[]);
        assert!((seconds(gps.jd_tt - 2451545.0) - 51.184).abs() < 1e-4);
        assert!((seconds(2451545.0 - gps.jd_utc) - 13.0).abs() < 1e-4, "GPS - UTC was 13 s in 2000");
        for (alias, scale) in [("TDT", "TT"), ("ET", "TT"), ("IAT", "TAI"), ("GMT", "UTC")] {
            let (aliased, named) = (at(alias, &[]), at(scale, &[]));
            assert_eq!((aliased.scale.as_str(), aliased.jd_tt, aliased.jd_utc), (scale, named.jd_tt, named.jd_utc), "{alias}");
        }
    }

    #[test]
    fn an_impossible_date_card_is_named_in_the_notes_or_in_the_refusal() {
        let err = header_time(&make_header(&[("DATE-OBS", "2026-02-30T12:00:00"), ("EXPTIME", "60")])).unwrap_err();
        assert_eq!(
            err,
            format!("{NO_TIME_REASON}; DATE-OBS ignored: date '2026-02-30T12:00:00' has day 30, but February 2026 has 28 days")
        );
        let time = header_time(&make_header(&[("DATE-OBS", "2026-04-31"), ("TIME-OBS", "06:00:00"), ("MJD-OBS", "61160.25")])).unwrap();
        assert!(time.source.starts_with("MJD-OBS"), "{}", time.source);
        assert_eq!(time.jd_header, 2461160.75);
        let named = "DATE-OBS/TIME-OBS ignored: date '2026-04-31T06:00:00' has day 31, but April 2026 has 30 days";
        assert!(time.notes.iter().any(|n| n == named), "{:?}", time.notes);
    }

    #[test]
    fn spacecraft_headers_get_no_light_time_correction() {
        let h = make_header(&[("TELESCOP", "JWST"), ("EXPMID", "61120.5"), ("RA_TARG", "10.0"), ("DEC_TARG", "-5.0")]);
        let time = header_time(&h).unwrap();
        let target = resolve_target("", &h, None, &GeometryOverrides::default()).unwrap();
        let g = frame_geometry(&h, &time, Some(&target), None);
        assert!(g.jd_utc.is_some());
        assert_eq!(g.bjd_tdb, None);
        assert_eq!(g.hjd_utc, None);
        assert_eq!(g.bjd_source, None);
        assert_eq!(g.moon_separation_deg, None, "{g:?}");
        assert!(g.moon_illumination.is_some());
        assert!(g.time_scale_notes.iter().any(|n| n.contains("spacecraft")), "{:?}", g.time_scale_notes);
        assert!(
            g.time_scale_notes.iter().any(|n| n.contains("Moon separation") && n.contains("TELESCOP JWST is a spacecraft")),
            "{:?}",
            g.time_scale_notes
        );
        assert!(!g.time_scale_notes.iter().any(|n| n == GEOCENTRIC_MOON_SEPARATION_NOTE), "{:?}", g.time_scale_notes);
    }

    fn assert_no_topocentric_block(g: &FrameGeometry) {
        assert_eq!(g.lst_deg, None, "{g:?}");
        assert_eq!(g.hour_angle_deg, None, "{g:?}");
        assert_eq!(g.altitude_deg, None, "{g:?}");
        assert_eq!(g.azimuth_deg, None, "{g:?}");
        assert_eq!(g.airmass_computed, None, "{g:?}");
        assert_eq!(g.parallactic_angle_deg, None, "{g:?}");
        assert_eq!(g.sun_altitude_deg, None, "{g:?}");
        assert_eq!(g.moon_altitude_deg, None, "{g:?}");
        assert_eq!(g.moon_separation_deg, None, "{g:?}");
    }

    #[test]
    fn spacecraft_headers_keep_a_site_override_and_header_site_cards_out_of_the_topocentric_block() {
        let mauna_kea = GeometryOverrides { site_lat: Some(19.82), site_lon: Some(-155.47), ..GeometryOverrides::default() };
        let jwst = make_header(&[("TELESCOP", "JWST"), ("EXPMID", "61120.5"), ("RA_TARG", "10.0"), ("DEC_TARG", "-5.0")]);
        let time = header_time(&jwst).unwrap();
        let target = resolve_target("", &jwst, None, &GeometryOverrides::default()).unwrap();
        let site = resolve_site(&jwst, &mauna_kea).unwrap();
        assert_eq!(site.source, "user");
        let g = frame_geometry(&jwst, &time, Some(&target), Some(&site));
        assert!(g.jd_utc.is_some());
        assert_no_topocentric_block(&g);
        assert!(g.moon_illumination.is_some());
        assert!(
            g.time_scale_notes.iter().any(|n| n.contains("site from user not applied") && n.contains("JWST is a spacecraft")),
            "{:?}",
            g.time_scale_notes
        );
        let hst = make_header(&[
            ("TELESCOP", "HST"),
            ("EXPMID", "61120.5"),
            ("RA_TARG", "10.0"),
            ("DEC_TARG", "-5.0"),
            ("SITELAT", "19.82"),
            ("SITELONG", "-155.47"),
        ]);
        let time = header_time(&hst).unwrap();
        let site = resolve_site(&hst, &GeometryOverrides::default()).unwrap();
        assert_eq!(site.source, "SITELAT/SITELONG");
        let g = frame_geometry(&hst, &time, Some(&target), Some(&site));
        assert_no_topocentric_block(&g);
        assert!(g.time_scale_notes.iter().any(|n| n.contains("site from SITELAT/SITELONG not applied")), "{:?}", g.time_scale_notes);
        let g = frame_geometry(&jwst, &time, Some(&target), None);
        assert_no_topocentric_block(&g);
        assert!(
            g.time_scale_notes.iter().any(|n| n.starts_with("no topocentric block or Moon separation: TELESCOP JWST")),
            "{:?}",
            g.time_scale_notes
        );
        assert!(!g.time_scale_notes.iter().any(|n| n == NO_SITE_NOTE), "{:?}", g.time_scale_notes);
    }

    #[test]
    fn frame_geometry_without_a_site_has_time_and_light_time_but_no_horizontal_block() {
        let h = make_header(&[("DATE-OBS", "2026-03-21T10:00:00"), ("EXPTIME", "600"), ("RA_TARG", "180.0"), ("DEC_TARG", "0.0")]);
        let time = header_time(&h).unwrap();
        let target = resolve_target("", &h, None, &GeometryOverrides::default()).unwrap();
        let g = frame_geometry(&h, &time, Some(&target), None);
        assert!(g.bjd_tdb.is_some() && g.hjd_utc.is_some());
        assert_eq!(g.bjd_source.as_deref(), Some("computed"));
        assert!(g.lst_deg.is_none() && g.altitude_deg.is_none() && g.airmass_computed.is_none());
        assert!(g.parallactic_angle_deg.is_none() && g.sun_altitude_deg.is_none() && g.moon_altitude_deg.is_none());
        assert!(g.moon_illumination.is_some());
        assert!(g.moon_separation_deg.is_some());
        assert!(g.time_scale_notes.iter().any(|n| n == GEOCENTRIC_MOON_SEPARATION_NOTE), "{:?}", g.time_scale_notes);
        assert_eq!(g.airmass_formula, AIRMASS_FORMULA);
        assert!(g.time_scale_notes.iter().any(|n| n.contains("site")), "{:?}", g.time_scale_notes);
    }

    #[test]
    fn frame_geometry_at_mauna_kea_for_a_transiting_target_has_zero_hour_angle_and_parallactic_angle() {
        let h = make_header(&[("DATE-OBS", "2026-03-21T10:00:00"), ("SITELAT", "19.82"), ("SITELONG", "-155.47")]);
        let time = header_time(&h).unwrap();
        let jd_utc = jd_from_gregorian(2026, 3, 21.0) + 10.0 / 24.0;
        assert!((time.jd_utc - jd_utc).abs() < 1e-9);
        let site = resolve_site(&h, &GeometryOverrides::default()).unwrap();
        let lst = local_sidereal_time_deg(jd_utc, -155.47);
        let of_date = spherical_to_cartesian(lst, 0.0);
        let j2000 = mat_apply(&mat_transpose(&precession_matrix_j2000_to_date(time.jd_tt)), of_date);
        let (ra, dec) = cartesian_to_spherical(j2000);
        let target = ResolvedTarget { ra_deg: ra, dec_deg: dec, source: "test".to_string() };
        let g = frame_geometry(&h, &time, Some(&target), Some(&site));
        assert!(g.hour_angle_deg.unwrap().abs() < 0.001, "{:?}", g.hour_angle_deg);
        assert!((g.altitude_deg.unwrap() - 70.18).abs() < 0.001, "{:?}", g.altitude_deg);
        assert!((g.azimuth_deg.unwrap() - 180.0).abs() < 0.01, "{:?}", g.azimuth_deg);
        assert!(g.parallactic_angle_deg.unwrap().abs() < 0.001, "{:?}", g.parallactic_angle_deg);
        assert!((g.airmass_computed.unwrap() - 1.063).abs() < 0.002, "{:?}", g.airmass_computed);
        assert!((g.lst_deg.unwrap() - lst).abs() < 1e-9);
        assert!(g.sun_altitude_deg.is_some() && g.moon_altitude_deg.is_some() && g.moon_separation_deg.is_some());
    }

    #[test]
    fn resolve_target_prefers_the_wcs_pixel_then_header_keywords_then_nothing_and_the_override_wins() {
        let wcs = header_with_cd(north_up_cd());
        let none = GeometryOverrides::default();
        let t = resolve_target("", &wcs, Some((49.5, 49.5)), &none).unwrap();
        assert!((t.ra_deg - 150.0).abs() < 1e-6 && (t.dec_deg - 2.0).abs() < 1e-6, "{t:?}");
        assert!(t.source.starts_with("WCS at image centre"), "{}", t.source);
        assert_eq!(image_centre_pixel(&wcs), Some((49.5, 49.5)));
        let keywords = make_header(&[("RA_TARG", "12.5"), ("DEC_TARG", "-3.25")]);
        let t = resolve_target("", &keywords, image_centre_pixel(&keywords), &none).unwrap();
        assert_eq!((t.ra_deg, t.dec_deg, t.source.as_str()), (12.5, -3.25, "RA_TARG/DEC_TARG"));
        let user = GeometryOverrides { target_ra: Some(1.0), target_dec: Some(2.0), ..GeometryOverrides::default() };
        let t = resolve_target("", &wcs, Some((49.5, 49.5)), &user).unwrap();
        assert_eq!((t.ra_deg, t.dec_deg, t.source.as_str()), (1.0, 2.0, "user"));
        assert!(resolve_target("", &make_header(&[("OBJECT", "M31")]), None, &none).is_none());
        let compressed = make_header(&[("ZIMAGE", "T"), ("NAXIS1", "8"), ("NAXIS2", "23"), ("ZNAXIS1", "37"), ("ZNAXIS2", "23")]);
        assert_eq!(image_centre_pixel(&compressed), Some((18.0, 11.0)));
        assert_eq!(image_centre_pixel(&make_header(&[("NAXIS1", "0"), ("NAXIS2", "5")])), None);
    }

    #[test]
    fn resolve_site_ranks_the_override_above_obsgeo_and_obsgeo_above_keyword_pairs() {
        let none = GeometryOverrides::default();
        let user = GeometryOverrides { site_lat: Some(1.0), site_lon: Some(2.0), ..GeometryOverrides::default() };
        let pair = make_header(&[("SITELAT", "19.82"), ("SITELONG", "-155.47")]);
        assert_eq!(resolve_site(&pair, &none).unwrap().source, "SITELAT/SITELONG");
        let s = resolve_site(&pair, &user).unwrap();
        assert_eq!((s.site.lat_deg, s.site.lon_deg, s.site.height_m, s.source.as_str()), (1.0, 2.0, 0.0, "user"));
        assert_eq!(resolve_site(&make_header(&[("OBJECT", "M31")]), &user).unwrap().source, "user");
        let obsgeo = make_header(&[("OBSGEO-X", "-5464487.8"), ("OBSGEO-Y", "-2492806.0"), ("OBSGEO-Z", "2151240.2")]);
        let automatic = resolve_site(&obsgeo, &none).unwrap();
        assert_eq!(automatic.source, "OBSGEO-X/Y/Z");
        assert!((automatic.site.lat_deg - 19.826).abs() < 0.02);
        let overridden = resolve_site(&obsgeo, &user).unwrap();
        assert_eq!((overridden.site.lat_deg, overridden.source.as_str()), (1.0, "user"));
        let notes = geometry_method_notes(&obsgeo, None, Some(&overridden));
        assert!(notes.iter().any(|n| n.contains("site override") && n.contains("OBSGEO-X/Y/Z")), "{notes:?}");
        let lbh = make_header(&[("OBSGEO-L", "-155.47"), ("OBSGEO-B", "19.82"), ("OBSGEO-H", "4200")]);
        assert_eq!(resolve_site(&lbh, &none).unwrap().source, "OBSGEO-L/B/H");
        assert_eq!(resolve_site(&lbh, &user).unwrap().source, "user");
        let both = make_header(&[("OBSGEO-L", "-155.47"), ("OBSGEO-B", "19.82"), ("SITELAT", "0"), ("SITELONG", "0")]);
        assert_eq!(resolve_site(&both, &none).unwrap().source, "OBSGEO-L/B/H");
        assert!(resolve_site(&make_header(&[("OBJECT", "M31")]), &none).is_none());
        let ambiguous = geometry_method_notes(&pair, None, resolve_site(&pair, &none).as_ref());
        assert!(ambiguous.iter().any(|n| n.contains("east-positive")), "{ambiguous:?}");
    }
}
