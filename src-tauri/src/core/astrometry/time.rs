pub const JD_J2000: f64 = 2451545.0;
pub const MJD_EPOCH_JD: f64 = 2400000.5;
pub const DAYS_PER_JULIAN_CENTURY: f64 = 36525.0;
pub const SECONDS_PER_DAY: f64 = 86400.0;
const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
];

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

pub fn gregorian_from_jd(jd: f64) -> (i32, u32, u32, u32, u32, f64) {
    let shifted = jd + 0.5;
    let z = shifted.floor();
    let alpha = ((z - 1867216.25) / 36524.25).floor();
    let b = z + 1.0 + alpha - (alpha / 4.0).floor() + 1524.0;
    let c = ((b - 122.1) / 365.25).floor();
    let d = (365.25 * c).floor();
    let e = ((b - d) / 30.6001).floor();
    let day = b - d - (30.6001 * e).floor();
    let month = if e < 14.0 { e - 1.0 } else { e - 13.0 };
    let year = if month > 2.0 { c - 4716.0 } else { c - 4715.0 };
    let seconds = (shifted - z) * SECONDS_PER_DAY;
    let hour = (seconds / 3600.0).floor();
    let minute = ((seconds - hour * 3600.0) / 60.0).floor();
    (year as i32, month as u32, day as u32, hour as u32, minute as u32, seconds - hour * 3600.0 - minute * 60.0)
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
    validate_calendar_date(year, month, day, source)?;
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
    let year = 1900 + year_two_digits;
    validate_calendar_date(year, month, day, source)?;
    Ok((year, month, day))
}

fn is_gregorian_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_gregorian_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn validate_calendar_date(year: i32, month: u32, day: u32, source: &str) -> Result<(), String> {
    if !(1..=12).contains(&month) {
        return Err(format!("date '{source}' has month {month}, outside 1 to 12"));
    }
    let last_day = days_in_month(year, month);
    if !(1..=last_day).contains(&day) {
        let month_name = MONTH_NAMES[month as usize - 1];
        return Err(format!("date '{source}' has day {day}, but {month_name} {year} has {last_day} days"));
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
    match time.map(clean_card).filter(|t| !has_time && !t.is_empty()) {
        Some(t) if date_text.contains('/') => {
            let (y, m, d) = parse_old_fits_date(date_text, date)?;
            Ok(jd_from_gregorian(y, m, d as f64 + day_fraction_from_time(t, t)?))
        }
        Some(t) => parse_fits_datetime(&format!("{date_text}T{t}")),
        None => parse_fits_datetime(date_text),
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
    fn an_old_dd_mm_yy_date_takes_its_time_of_day_from_time_obs() {
        assert_eq!(parse_fits_date_and_time("19/09/96", Some("06:00:00")).unwrap(), jd_from_gregorian(1996, 9, 19.25));
        assert_eq!(parse_fits_date_and_time("'19/09/96'", Some("'18:00:00'")).unwrap(), jd_from_gregorian(1996, 9, 19.75));
        assert_eq!(parse_fits_date_and_time("19/09/96", None).unwrap(), jd_from_gregorian(1996, 9, 19.0));
        let err = parse_fits_date_and_time("30/02/96", Some("06:00:00")).unwrap_err();
        assert!(err.contains("has day 30, but February 1996 has 29 days"), "{err}");
    }

    #[test]
    fn malformed_strings_are_rejected_with_the_input_named() {
        for bad in ["", "2026-13-01", "2026-09-19T25:00:00", "yesterday", "2026/09/19", "2026-09"] {
            let err = parse_fits_datetime(bad).unwrap_err();
            assert!(err.contains(bad) || bad.is_empty(), "{bad}: {err}");
        }
    }

    #[test]
    fn audit_probe_february_30_is_refused_instead_of_becoming_march_2() {
        let err = parse_fits_datetime("2026-02-30T12:00:00").unwrap_err();
        assert!(err.contains("'2026-02-30T12:00:00'") && err.contains("day 30, but February 2026 has 28 days"), "{err}");
        assert_eq!(parse_fits_datetime("2026-03-02T12:00:00"), Ok(2461102.0));
    }

    #[test]
    fn impossible_calendar_dates_are_refused_in_both_syntaxes_with_the_bad_value_named() {
        for (bad, reason) in [
            ("2026-02-30", "day 30, but February 2026 has 28 days"),
            ("2026-04-31T00:00:00", "day 31, but April 2026 has 30 days"),
            ("2025-02-29", "day 29, but February 2025 has 28 days"),
            ("1900-02-29T12:00:00", "day 29, but February 1900 has 28 days"),
            ("2100-02-29", "day 29, but February 2100 has 28 days"),
            ("2026-06-00", "day 0, but June 2026 has 30 days"),
            ("2026-13-01", "month 13, outside 1 to 12"),
            ("30/02/96", "day 30, but February 1996 has 29 days"),
            ("29/02/97", "day 29, but February 1997 has 28 days"),
            ("29/02/00", "day 29, but February 1900 has 28 days"),
            ("31/09/26", "day 31, but September 1926 has 30 days"),
        ] {
            let err = parse_fits_datetime(bad).unwrap_err();
            assert!(err.contains(&format!("'{bad}'")) && err.contains(reason), "{bad}: {err}");
        }
        let err = parse_fits_date_and_time("2026-02-30", Some("06:00:00")).unwrap_err();
        assert!(err.contains("'2026-02-30T06:00:00'") && err.contains("February 2026 has 28 days"), "{err}");
    }

    #[test]
    fn leap_days_follow_the_gregorian_century_rule_in_both_syntaxes() {
        assert_eq!(parse_fits_datetime("2024-02-29").unwrap(), jd_from_gregorian(2024, 2, 29.0));
        assert_eq!(parse_fits_datetime("2000-02-29T12:00:00").unwrap(), 2451604.0);
        assert_eq!(parse_fits_datetime("2400-02-29").unwrap(), jd_from_gregorian(2400, 2, 29.0));
        assert_eq!(parse_fits_datetime("29/02/96").unwrap(), jd_from_gregorian(1996, 2, 29.0));
        assert_eq!(parse_fits_datetime("2024-02-29").unwrap() + 1.0, parse_fits_datetime("2024-03-01").unwrap());
        assert_eq!(parse_fits_datetime("2000-02-29").unwrap() + 1.0, parse_fits_datetime("2000-03-01").unwrap());
        for (last_day, next_day) in [
            ("2026-01-31", "2026-02-01"),
            ("2026-02-28", "2026-03-01"),
            ("2026-04-30", "2026-05-01"),
            ("2026-12-31", "2027-01-01"),
            ("28/02/97", "01/03/97"),
            ("31/12/98", "01/01/99"),
        ] {
            assert_eq!(parse_fits_datetime(last_day).unwrap() + 1.0, parse_fits_datetime(next_day).unwrap(), "{last_day}");
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

    #[test]
    fn gregorian_from_jd_inverts_jd_from_gregorian() {
        let mut dates: Vec<(i32, u32, u32)> = vec![(2000, 2, 29), (2100, 3, 1), (2100, 2, 28), (1900, 3, 1), (1582, 10, 15), (2026, 10, 31), (2026, 12, 31)];
        dates.extend((0..193).map(|k| (1850 + (k * 7) as i32 % 300, 1 + (k * 5) as u32 % 12, 1 + (k * 11) as u32 % 28)));
        assert_eq!(dates.len(), 200);
        for (y, m, d) in dates {
            assert_eq!(gregorian_from_jd(jd_from_gregorian(y, m, d as f64)), (y, m, d, 0, 0, 0.0), "{y}-{m}-{d}");
        }
        assert_eq!(gregorian_from_jd(2461344.75), (2026, 10, 31, 6, 0, 0.0));
        let (y, m, d, h, min, s) = gregorian_from_jd(jd_from_mjd(61344.000786956));
        assert_eq!((y, m, d, h, min), (2026, 10, 31, 0, 1));
        assert!((s - 7.993).abs() < 1e-4, "{s}");
    }
}
