pub const JD_J2000: f64 = 2451545.0;
pub const MJD_EPOCH_JD: f64 = 2400000.5;
pub const DAYS_PER_JULIAN_CENTURY: f64 = 36525.0;
pub const SECONDS_PER_DAY: f64 = 86400.0;

pub fn jd_from_mjd(mjd: f64) -> f64 {
    mjd + MJD_EPOCH_JD
}

pub fn mjd_from_jd(jd: f64) -> f64 {
    jd - MJD_EPOCH_JD
}

pub fn julian_centuries_j2000(jd: f64) -> f64 {
    (jd - JD_J2000) / DAYS_PER_JULIAN_CENTURY
}

pub fn jd_from_gregorian(year: i32, month: u32, day: f64) -> f64 {
    let (y, m) = if month <= 2 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let a = (y as f64 / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y as f64 + 4716.0)).floor() + (30.6001 * (m as f64 + 1.0)).floor() + day + b - 1524.5
}

pub fn gmst_deg(jd_ut: f64) -> f64 {
    let d = jd_ut - JD_J2000;
    let t = d / DAYS_PER_JULIAN_CENTURY;
    let theta = 280.46061837 + 360.98564736629 * d + 0.000387933 * t * t - t * t * t / 38710000.0;
    theta.rem_euclid(360.0)
}

fn clean_card(s: &str) -> &str {
    s.trim().trim_matches('\'').trim()
}

fn parse_component<T: std::str::FromStr>(text: &str, what: &str, source: &str) -> Result<T, String> {
    text.trim()
        .parse::<T>()
        .map_err(|_| format!("cannot parse {} in date/time '{}'", what, source))
}

fn day_fraction_from_time(time: &str, source: &str) -> Result<f64, String> {
    let time = time.trim().trim_end_matches('Z');
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!("time part of '{}' must be hh:mm[:ss.sss]", source));
    }
    let hours: f64 = parse_component(parts[0], "hours", source)?;
    let minutes: f64 = parse_component(parts[1], "minutes", source)?;
    let seconds: f64 = if parts.len() == 3 {
        parse_component(parts[2], "seconds", source)?
    } else {
        0.0
    };
    if !(0.0..24.0).contains(&hours) || !(0.0..60.0).contains(&minutes) || !(0.0..61.0).contains(&seconds) {
        return Err(format!("time part of '{}' is out of range", source));
    }
    Ok((hours * 3600.0 + minutes * 60.0 + seconds) / SECONDS_PER_DAY)
}

fn parse_iso_date(date: &str, source: &str) -> Result<(i32, u32, u32), String> {
    let parts: Vec<&str> = date.trim().split('-').collect();
    if parts.len() != 3 {
        return Err(format!("date part of '{}' must be YYYY-MM-DD", source));
    }
    let year: i32 = parse_component(parts[0], "year", source)?;
    let month: u32 = parse_component(parts[1], "month", source)?;
    let day: u32 = parse_component(parts[2], "day", source)?;
    validate_month_day(month, day, source)?;
    Ok((year, month, day))
}

fn parse_old_fits_date(date: &str, source: &str) -> Result<(i32, u32, u32), String> {
    let parts: Vec<&str> = date.trim().split('/').collect();
    if parts.len() != 3 {
        return Err(format!("date '{}' must be DD/MM/YY", source));
    }
    let day: u32 = parse_component(parts[0], "day", source)?;
    let month: u32 = parse_component(parts[1], "month", source)?;
    let year_two_digits: i32 = parse_component(parts[2], "year", source)?;
    if !(0..=99).contains(&year_two_digits) {
        return Err(format!("two-digit year in '{}' is out of range", source));
    }
    validate_month_day(month, day, source)?;
    Ok((1900 + year_two_digits, month, day))
}

fn validate_month_day(month: u32, day: u32, source: &str) -> Result<(), String> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(format!("date '{}' has an invalid month or day", source));
    }
    Ok(())
}

pub fn parse_fits_datetime(s: &str) -> Result<f64, String> {
    let text = clean_card(s);
    if text.is_empty() {
        return Err("empty date/time string".to_string());
    }
    if text.contains('/') {
        let (y, m, d) = parse_old_fits_date(text, s)?;
        return Ok(jd_from_gregorian(y, m, d as f64));
    }
    let (date_part, time_part) = match text.find(|c: char| c == 'T' || c == ' ') {
        Some(pos) => (&text[..pos], Some(&text[pos + 1..])),
        None => (text, None),
    };
    let (y, m, d) = parse_iso_date(date_part, s)?;
    let fraction = match time_part {
        Some(t) if !t.trim().is_empty() => day_fraction_from_time(t, s)?,
        _ => 0.0,
    };
    Ok(jd_from_gregorian(y, m, d as f64 + fraction))
}

pub fn parse_fits_date_and_time(date: &str, time: Option<&str>) -> Result<f64, String> {
    let date_text = clean_card(date);
    let has_time = date_text.contains('T') || date_text.contains(' ');
    match time {
        Some(t) if !has_time && !clean_card(t).is_empty() => {
            let combined = format!("{}T{}", date_text, clean_card(t));
            parse_fits_datetime(&combined)
        }
        _ => parse_fits_datetime(date_text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j2000_epoch_parses_to_its_julian_date() {
        assert_eq!(parse_fits_datetime("2000-01-01T12:00:00").unwrap(), 2451545.0);
        assert_eq!(parse_fits_datetime("'2000-01-01T12:00:00.000'").unwrap(), 2451545.0);
    }

    #[test]
    fn midnight_2026_09_19_matches_the_meeus_algorithm() {
        let expected = {
            let (y, m) = (2026.0_f64, 9.0_f64);
            let a = (y / 100.0).floor();
            let b = 2.0 - a + (a / 4.0).floor();
            (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + 19.0 + b - 1524.5
        };
        assert_eq!(expected, 2461302.5);
        assert_eq!(parse_fits_datetime("2026-09-19T00:00:00").unwrap(), 2461302.5);
        assert_eq!(parse_fits_datetime("2026-09-19").unwrap(), 2461302.5);
        assert!((parse_fits_datetime("2026-09-19T06:00:00").unwrap() - 2461302.75).abs() < 1e-9);
        assert!((parse_fits_datetime("2026-09-19 12:30").unwrap() - (2461302.5 + 12.5 / 24.0)).abs() < 1e-9);
    }

    #[test]
    fn old_dd_mm_yy_convention_means_the_twentieth_century() {
        assert_eq!(parse_fits_datetime("19/09/26").unwrap(), 2424777.5);
        assert_eq!(parse_fits_datetime("01/01/00").unwrap(), jd_from_gregorian(1900, 1, 1.0));
    }

    #[test]
    fn malformed_strings_are_rejected_with_the_input_named() {
        for bad in ["", "2026-13-01", "2026-09-19T25:00:00", "yesterday", "2026/09/19", "2026-09"] {
            let err = parse_fits_datetime(bad).unwrap_err();
            assert!(err.contains(bad) || bad.is_empty(), "{bad}: {err}");
        }
    }

    #[test]
    fn date_obs_and_time_obs_combine_only_when_the_date_has_no_time() {
        let jd = parse_fits_date_and_time("2026-09-19", Some("06:00:00")).unwrap();
        assert!((jd - 2461302.75).abs() < 1e-9);
        let jd = parse_fits_date_and_time("2026-09-19T06:00:00", Some("18:00:00")).unwrap();
        assert!((jd - 2461302.75).abs() < 1e-9);
        assert_eq!(parse_fits_date_and_time("2026-09-19", None).unwrap(), 2461302.5);
    }

    #[test]
    fn mjd_round_trips_and_matches_the_j2000_epoch() {
        assert_eq!(jd_from_mjd(51544.5), 2451545.0);
        assert_eq!(mjd_from_jd(2451545.0), 51544.5);
        for mjd in [0.0, 40000.25, 60000.123456789, 61302.5] {
            assert!((mjd_from_jd(jd_from_mjd(mjd)) - mjd).abs() < 1e-9);
        }
        assert_eq!(julian_centuries_j2000(JD_J2000), 0.0);
        assert!((julian_centuries_j2000(2461302.5) - 9757.5 / 36525.0).abs() < 1e-12);
        assert!((julian_centuries_j2000(2461302.5) - 0.26714579).abs() < 1e-7);
    }

    #[test]
    fn gmst_at_j2000_is_the_textbook_value() {
        assert!((gmst_deg(JD_J2000) - 280.46061837).abs() < 1e-6);
        let one_day_later = gmst_deg(JD_J2000 + 1.0);
        assert!((one_day_later - (280.46061837 + 0.98564736629)).abs() < 1e-5);
        let jd = jd_from_gregorian(1987, 4, 10.0);
        let hours = gmst_deg(jd) / 15.0;
        assert!((hours - 13.1795463).abs() < 1e-5, "{hours}");
    }
}
