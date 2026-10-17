use std::fs::Metadata;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::astrometry::time::gregorian_from_jd;

pub type FileStamp = (u64, Option<SystemTime>);

const UNIX_EPOCH_JD: f64 = 2_440_587.5;
const SECONDS_PER_DAY: u64 = 86_400;

pub fn file_stamp(meta: &Metadata) -> FileStamp {
    (meta.len(), meta.modified().ok())
}

pub fn utc_timestamp(time: SystemTime) -> String {
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since_epoch.as_secs();
    let (year, month, day, ..) = gregorian_from_jd(UNIX_EPOCH_JD + (seconds / SECONDS_PER_DAY) as f64);
    let of_day = seconds % SECONDS_PER_DAY;
    let clock = format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}", of_day / 3600, of_day / 60 % 60, of_day % 60);
    match since_epoch.subsec_nanos() {
        0 => format!("{clock}Z"),
        nanos => format!("{clock}.{nanos:09}Z"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn utc_timestamp_formats_the_civil_time_and_keeps_nanoseconds() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_760_000_000)), "2025-10-09T08:53:20Z");
        assert_eq!(utc_timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(
            utc_timestamp(UNIX_EPOCH + Duration::new(1_760_000_000, 120_000_700)),
            "2025-10-09T08:53:20.120000700Z",
            "a sub-second mtime change would not change the fingerprint"
        );
    }
}
