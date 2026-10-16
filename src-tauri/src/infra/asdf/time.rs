use std::collections::HashMap;
use std::ops::RangeInclusive;

use crate::core::astrometry::time::{gregorian_from_jd, jd_from_mjd, mjd_from_jd, parse_fits_datetime, SECONDS_PER_DAY};

const DEFAULT_SCALE: &str = "UTC";
const UNIX_EPOCH_MJD: f64 = 40587.0;
const BARE_MJD_RANGE: RangeInclusive<f64> = 40000.0..=90000.0;
const MS_PER_DAY: f64 = 86_400_000.0;

#[derive(Debug, Clone, PartialEq)]
pub struct AsdfTime {
    pub iso: String,
    pub mjd: f64,
    pub scale: String,
}

pub fn decode(metadata: &HashMap<String, String>, key: &str) -> Option<AsdfTime> {
    let field = |suffix: &str| non_empty(metadata.get(&format!("{key}.{suffix}")));
    let (value, format) = match non_empty(metadata.get(key)) {
        Some(scalar) => (scalar, None),
        None => (field("value")?, field("format").map(str::to_ascii_lowercase)),
    };
    let (mjd, iso) = match format.as_deref() {
        None | Some("iso") | Some("isot") => implicit(key, value)?,
        Some("mjd") => from_mjd(value.parse().ok()?),
        Some("jd") => from_mjd(mjd_from_jd(value.parse().ok()?)),
        Some("unix") => from_mjd(UNIX_EPOCH_MJD + value.parse::<f64>().ok()? / SECONDS_PER_DAY),
        Some(other) => {
            log::debug!("{key}: ASDF time format '{other}' is not decoded, no FITS time card is written");
            return None;
        }
    };
    let scale = field("scale").map_or_else(|| DEFAULT_SCALE.to_string(), str::to_ascii_uppercase);
    mjd.is_finite().then_some(AsdfTime { iso, mjd, scale })
}

pub fn iso_from_mjd(mjd: f64) -> String {
    let mut day = mjd.floor();
    let mut ms = ((mjd - day) * MS_PER_DAY).round();
    if ms >= MS_PER_DAY {
        day += 1.0;
        ms -= MS_PER_DAY;
    }
    let (year, month, date, ..) = gregorian_from_jd(jd_from_mjd(day));
    let ms = ms as u64;
    format!(
        "{year:04}-{month:02}-{date:02}T{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

fn non_empty(value: Option<&String>) -> Option<&str> {
    value.map(|v| v.trim()).filter(|v| !v.is_empty())
}

fn from_mjd(mjd: f64) -> (f64, String) {
    (mjd, iso_from_mjd(mjd))
}

fn implicit(key: &str, value: &str) -> Option<(f64, String)> {
    if let Ok(mjd) = value.parse::<f64>() {
        return BARE_MJD_RANGE.contains(&mjd).then(|| from_mjd(mjd));
    }
    if !starts_with_iso_date(value) {
        log::debug!("{key}: ASDF time '{value}' is not an ISO date (byear, jyear and yday are not decoded)");
        return None;
    }
    let jd = parse_fits_datetime(value).ok()?;
    Some((mjd_from_jd(jd), value.trim_end_matches('Z').replacen(' ', "T", 1)))
}

fn starts_with_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 10 && bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-' && bytes[7] == b'-'
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn metadata(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    const START: &str = "roman.meta.exposure.start_time";

    #[test]
    fn the_implicit_iso_scalar_is_utc() {
        let t = decode(&metadata(&[(START, "2026-10-31T00:00:00.000")]), START).unwrap();
        assert_eq!(t.mjd, 61344.0);
        assert_eq!(t.iso, "2026-10-31T00:00:00.000");
        assert_eq!(t.scale, "UTC");
    }

    #[test]
    fn the_mapping_form_keeps_its_scale() {
        let tree = metadata(&[(&format!("{START}.value"), "2026-10-31T00:01:07.993"), (&format!("{START}.scale"), "tt")]);
        let t = decode(&tree, START).unwrap();
        assert!((t.mjd - 61344.000786956).abs() < 1e-9, "{}", t.mjd);
        assert_eq!(t.iso, "2026-10-31T00:01:07.993");
        assert_eq!(t.scale, "TT");
        let datetime = metadata(&[(&format!("{START}.value"), "2026-07-09T21:10:10.493"), (&format!("{START}.base_format"), "datetime")]);
        let t = decode(&datetime, START).unwrap();
        assert_eq!((t.iso.as_str(), t.scale.as_str()), ("2026-07-09T21:10:10.493", "UTC"));
    }

    #[test]
    fn mjd_jd_and_unix_formats_rebuild_the_iso_instant() {
        let mjd = decode(&metadata(&[(&format!("{START}.value"), "61344.5"), (&format!("{START}.format"), "mjd")]), START).unwrap();
        assert_eq!((mjd.iso.as_str(), mjd.mjd, mjd.scale.as_str()), ("2026-10-31T12:00:00.000", 61344.5, "UTC"));
        let jd = decode(&metadata(&[(&format!("{START}.value"), "2461345.0"), (&format!("{START}.format"), "jd")]), START).unwrap();
        assert_eq!((jd.iso.as_str(), jd.mjd), ("2026-10-31T12:00:00.000", 61344.5));
        let unix_seconds = (61344.5 - 40587.0) * 86400.0;
        let unix = decode(&metadata(&[(&format!("{START}.value"), &unix_seconds.to_string()), (&format!("{START}.format"), "unix")]), START).unwrap();
        assert_eq!((unix.iso.as_str(), unix.mjd), ("2026-10-31T12:00:00.000", 61344.5));
    }

    #[test]
    fn a_bare_mjd_float_is_read_as_mjd() {
        let t = decode(&metadata(&[("meta.exposure.start_time", "59000.25")]), "meta.exposure.start_time").unwrap();
        assert_eq!((t.iso.as_str(), t.mjd, t.scale.as_str()), ("2020-05-31T06:00:00.000", 59000.25, "UTC"));
        assert!(decode(&metadata(&[("meta.exposure.start_time", "123.5")]), "meta.exposure.start_time").is_none());
    }

    #[test]
    fn unsupported_formats_and_missing_keys_decode_to_none() {
        let jyear = metadata(&[(&format!("{START}.value"), "J2000.0"), (&format!("{START}.format"), "jyear_str")]);
        assert!(decode(&jyear, START).is_none());
        for implicit in ["J2000.0", "B1950.0", "2026:304:00:00:00.000", "", "not a time"] {
            assert!(decode(&metadata(&[(START, implicit)]), START).is_none(), "{implicit:?}");
        }
        assert!(decode(&metadata(&[]), START).is_none());
        assert!(decode(&metadata(&[("roman.meta.exposure.end_time", "2026-10-31T00:02:15.986")]), START).is_none());
    }

    #[test]
    fn a_space_separated_iso_value_is_written_with_a_t() {
        let t = decode(&metadata(&[(START, "2026-10-31 00:02:15.986")]), START).unwrap();
        assert_eq!(t.iso, "2026-10-31T00:02:15.986");
        assert!((t.mjd - (61344.0 + 135.986 / 86400.0)).abs() < 1e-9);
    }

    #[test]
    fn mjd_rounding_carries_into_the_next_day() {
        assert_eq!(iso_from_mjd(61344.0 - 1e-10), "2026-10-31T00:00:00.000");
        assert_eq!(iso_from_mjd(61344.999_999_999_9), "2026-11-01T00:00:00.000");
        assert_eq!(iso_from_mjd(61344.000786956), "2026-10-31T00:01:07.993");
    }
}
