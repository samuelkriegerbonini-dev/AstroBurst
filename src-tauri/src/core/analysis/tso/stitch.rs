use std::collections::HashMap;
use std::ops::Range;

use anyhow::{bail, Result};

use super::{ExposureCompleteness, TsoFile, TsoTimeScale};
use crate::infra::fits::gzip::split_gz_suffix;
use crate::infra::fits::x1dints::{TimeOrigin, COL_MJD_AVG, COL_TDB_MID};

pub const TIME_SOURCE_TDB_COLUMN: &str = "TDB-MID column";
pub const TIME_SOURCE_TDB_KEYWORD: &str = "TDB-MID keyword";
pub const TIME_SOURCE_MJD_AVG: &str = "MJD-AVG (UTC, not barycentric)";
pub const TIME_SOURCE_INDEX: &str = "integration number (no usable times)";

const LEVEL_3: u8 = 3;
const PRODUCT_ENDING: &str = "_x1dints.fits";
const SEGMENT_MARK: &str = "-seg";

pub(crate) struct Part {
    pub file_index: usize,
    pub hdu: usize,
    pub rows: Range<usize>,
}

pub(crate) struct Exposures {
    pub ordinal: Vec<usize>,
    pub completeness: Vec<ExposureCompleteness>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

pub(crate) struct TimeAxis {
    pub scale: TsoTimeScale,
    pub source: &'static str,
    pub warning: Option<String>,
}

#[derive(PartialEq)]
enum Identity {
    Started { expstart_bits: u64, nints: Option<i64> },
    Named { name: String, nints: Option<i64> },
    Run { file_index: usize, run: usize },
}

struct Exposure {
    identity: Identity,
    file_index: usize,
    parts: Vec<usize>,
}

pub fn exposure_name(file_name: &str) -> String {
    let (plain, _) = split_gz_suffix(file_name);
    let product = plain
        .len()
        .checked_sub(PRODUCT_ENDING.len())
        .filter(|&cut| plain.is_char_boundary(cut) && plain[cut..].eq_ignore_ascii_case(PRODUCT_ENDING));
    let stem = match product.or_else(|| plain.rfind('.')) {
        Some(cut) => &plain[..cut],
        None => plain,
    };
    let segment = stem
        .match_indices(SEGMENT_MARK)
        .find(|(at, mark)| stem[at + mark.len()..].starts_with(|next: char| next.is_ascii_digit()));
    match segment {
        Some((at, _)) => stem[..at].to_string(),
        None => stem.to_string(),
    }
}

pub(crate) fn missing_ranges(found: &[i64], nints: i64) -> Vec<(i64, i64)> {
    let mut present: Vec<i64> = found.iter().copied().filter(|number| (1..=nints).contains(number)).collect();
    present.sort_unstable();
    let mut missing = Vec::new();
    let mut covered = 0;
    for number in present {
        if number - covered > 1 {
            missing.push((covered + 1, number - 1));
        }
        covered = number;
    }
    if covered < nints {
        missing.push((covered + 1, nints));
    }
    missing
}

pub(crate) fn usable_times(values: &[f64]) -> bool {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return false;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    sorted.windows(2).all(|pair| pair[0] != pair[1])
}

fn column_state(values: &[f64]) -> &'static str {
    if values.iter().all(|value| value.is_nan()) {
        "is missing"
    } else if values.iter().any(|value| !value.is_finite()) {
        "has NaN values"
    } else {
        "repeats values"
    }
}

pub(crate) fn time_axis(bmjd_tdb_mid: &[f64], mjd_utc_mid: &[f64], origin: TimeOrigin) -> TimeAxis {
    if usable_times(bmjd_tdb_mid) {
        let source = if origin == TimeOrigin::Keyword { TIME_SOURCE_TDB_KEYWORD } else { TIME_SOURCE_TDB_COLUMN };
        return TimeAxis { scale: TsoTimeScale::BmjdTdb, source, warning: None };
    }
    let tdb_state = column_state(bmjd_tdb_mid);
    if usable_times(mjd_utc_mid) {
        return TimeAxis {
            scale: TsoTimeScale::MjdUtc,
            source: TIME_SOURCE_MJD_AVG,
            warning: Some(format!("{COL_TDB_MID} {tdb_state}: times are {TIME_SOURCE_MJD_AVG}")),
        };
    }
    let utc_state = column_state(mjd_utc_mid);
    TimeAxis {
        scale: TsoTimeScale::Index,
        source: TIME_SOURCE_INDEX,
        warning: Some(format!(
            "no usable integration times: {COL_TDB_MID} {tdb_state} and {COL_MJD_AVG} {utc_state}; the abscissa is the integration number"
        )),
    }
}

pub(crate) fn rows_by_time(times: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..times.len()).collect();
    order.sort_by(|&earlier, &later| times[earlier].total_cmp(&times[later]));
    order
}

pub(crate) fn rows_by_number(exposure: &[usize], segment: &[i64], int_num: &[i64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..int_num.len()).collect();
    order.sort_by_key(|&row| (exposure[row], segment[row], int_num[row]));
    order
}

pub(crate) fn reorder<T: Copy>(values: &mut Vec<T>, order: &[usize], width: usize) {
    let mut moved = Vec::with_capacity(values.len());
    for &row in order {
        moved.extend_from_slice(&values[row * width..(row + 1) * width]);
    }
    *values = moved;
}

pub(crate) fn refuse_mixed_levels(files: &[&TsoFile], label: &str) -> Result<()> {
    let product = files.iter().find(|file| file.level == LEVEL_3);
    let segment = files.iter().find(|file| file.level != LEVEL_3);
    if let (Some(product), Some(segment)) = (product, segment) {
        bail!(
            "{} is a level 3 product and {} is a level 2b product, and both hold the series {label}: the integrations are the same but FLUX is not (level 3 is extracted after outlier detection); open one level at a time",
            product.file_name,
            segment.file_name
        );
    }
    Ok(())
}

fn identities(files: &[TsoFile], parts: &[Part], int_num: &[i64]) -> Vec<Identity> {
    let mut run = 0;
    let mut previous: Option<(usize, Option<i64>)> = None;
    parts
        .iter()
        .map(|part| {
            let file = &files[part.file_index];
            if file.level != LEVEL_3 {
                return match file.expstart {
                    Some(expstart) => Identity::Started { expstart_bits: expstart.to_bits(), nints: file.nints },
                    None => Identity::Named { name: exposure_name(&file.file_name), nints: file.nints },
                };
            }
            let numbers = &int_num[part.rows.clone()];
            let last_of_file = previous.filter(|(file_index, _)| *file_index == part.file_index).map(|(_, last)| last);
            match last_of_file {
                None => run = 1,
                Some(last) => {
                    if numbers.first().zip(last).is_some_and(|(first, last)| *first <= last) {
                        run += 1;
                    }
                }
            }
            previous = Some((part.file_index, numbers.last().copied().or(last_of_file.flatten())));
            Identity::Run { file_index: part.file_index, run }
        })
        .collect()
}

fn shown_name(files: &[TsoFile], all: &[Exposure], exposure: &Exposure) -> String {
    let name = exposure_name(&files[exposure.file_index].file_name);
    let Identity::Run { file_index, run } = exposure.identity else {
        return name;
    };
    let runs = all.iter().filter(|other| matches!(other.identity, Identity::Run { file_index: of, .. } if of == file_index)).count();
    if runs > 1 {
        format!("{name}, exposure {run} of {runs}")
    } else {
        name
    }
}

fn numbers_held_once(files: &[TsoFile], parts: &[Part], int_num: &[i64], exposure: &Exposure, name: &str) -> Result<Vec<i64>> {
    let mut holder: HashMap<i64, &Part> = HashMap::new();
    let mut numbers = Vec::new();
    for &part_index in &exposure.parts {
        let part = &parts[part_index];
        for &number in &int_num[part.rows.clone()] {
            if let Some(first) = holder.insert(number, part) {
                bail!(
                    "{} (HDU {}) and {} (HDU {}) both hold integration {number} of exposure {name}: overlapping INT_NUM ranges cannot be stitched",
                    files[first.file_index].file_name,
                    first.hdu,
                    files[part.file_index].file_name,
                    part.hdu
                );
            }
            numbers.push(number);
        }
    }
    Ok(numbers)
}

fn ranges_text(ranges: &[(i64, i64)]) -> String {
    let written: Vec<String> = ranges
        .iter()
        .map(|&(first, last)| if first == last { first.to_string() } else { format!("{first} to {last}") })
        .collect();
    written.join(", ")
}

pub(crate) fn exposures(files: &[TsoFile], parts: &[Part], int_num: &[i64]) -> Result<Exposures> {
    let mut all: Vec<Exposure> = Vec::new();
    let mut ordinal = vec![0; int_num.len()];
    for (part_index, (part, identity)) in parts.iter().zip(identities(files, parts, int_num)).enumerate() {
        let at = match all.iter().position(|exposure| exposure.identity == identity) {
            Some(at) => at,
            None => {
                all.push(Exposure { identity, file_index: part.file_index, parts: Vec::new() });
                all.len() - 1
            }
        };
        all[at].parts.push(part_index);
        ordinal[part.rows.clone()].fill(at);
    }

    let mut found = Exposures { ordinal, completeness: Vec::new(), notes: Vec::new(), warnings: Vec::new() };
    for exposure in &all {
        let name = shown_name(files, &all, exposure);
        let numbers = numbers_held_once(files, parts, int_num, exposure, &name)?;
        let nints = files[exposure.file_index].nints;
        let missing = nints.map(|nints| missing_ranges(&numbers, nints)).unwrap_or_default();
        match nints {
            None => found.notes.push(format!("exposure {name}: NINTS card missing, completeness not checked")),
            Some(nints) => {
                if !missing.is_empty() {
                    found.warnings.push(format!(
                        "exposure {name}: {} of {nints} integrations found, missing {}",
                        numbers.len(),
                        ranges_text(&missing)
                    ));
                }
                if let Some(highest) = numbers.iter().copied().max().filter(|highest| *highest > nints) {
                    found.notes.push(format!("exposure {name}: integration numbers reach {highest}, above NINTS = {nints}"));
                }
            }
        }
        let complete = nints.map(|nints| missing.is_empty() && numbers.len() as i64 == nints);
        found.completeness.push(ExposureCompleteness { exposure: name, nints, found: numbers.len(), missing, complete });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::tso::series::build_set;
    use crate::core::analysis::tso::series::test_support::{file_of, table_of};
    use crate::core::analysis::tso::TsoSeries;
    use crate::infra::fits::x1dints::test_fixtures::{ramp_row, SynthRow};
    use crate::infra::fits::x1dints::{X1dIntsFile, X1dIntsTable};

    fn rows_from(start: f64, integrations: &[i32], segment: i32) -> Vec<SynthRow> {
        integrations
            .iter()
            .map(|&number| SynthRow { segment, ..ramp_row(number, start + 0.1 * f64::from(number), 4) })
            .collect()
    }

    fn nrs1_table(hdu: usize, rows: &[SynthRow]) -> X1dIntsTable {
        table_of("NRS1", Some(-1), hdu, rows)
    }

    fn exposure_file(name: &str, nints: Option<i64>, expstart: Option<f64>, integrations: &[i32], segment: i32) -> X1dIntsFile {
        let rows = rows_from(expstart.unwrap_or(60000.0), integrations, segment);
        file_of(name, 2, nints, expstart, vec![nrs1_table(1, &rows)])
    }

    fn segment_file(name: &str, nints: Option<i64>, integrations: &[i32], segment: i32) -> X1dIntsFile {
        exposure_file(name, nints, Some(60000.0), integrations, segment)
    }

    fn level_3_file(tables: &[&[i32]]) -> X1dIntsFile {
        let mut position = 0_i32;
        let tables = tables
            .iter()
            .enumerate()
            .map(|(index, integrations)| {
                let rows: Vec<SynthRow> = integrations
                    .iter()
                    .map(|&number| {
                        position += 1;
                        ramp_row(number, 60000.0 + 0.1 * f64::from(position), 4)
                    })
                    .collect();
                nrs1_table(index + 1, &rows)
            })
            .collect();
        file_of("l3_x1dints.fits", 3, Some(4), Some(60000.0), tables)
    }

    fn only_series(files: Vec<X1dIntsFile>) -> TsoSeries {
        let mut set = build_set(files).unwrap();
        assert_eq!(set.series.len(), 1);
        set.series.remove(0)
    }

    fn record(exposure: &str, nints: Option<i64>, found: usize, missing: &[(i64, i64)], complete: Option<bool>) -> ExposureCompleteness {
        ExposureCompleteness { exposure: exposure.to_string(), nints, found, missing: missing.to_vec(), complete }
    }

    const NO_TEXT: [&str; 0] = [];
    const NONE_MISSING: [(i64, i64); 0] = [];

    #[test]
    fn completeness_is_judged_per_exposure_against_nints() {
        assert_eq!(missing_ranges(&[1, 2, 3, 7, 8], 8), [(4, 6)]);
        assert_eq!(missing_ranges(&[1, 2, 3, 4, 5, 6], 8), [(7, 8)]);
        assert_eq!(missing_ranges(&[1, 3, 4, 7], 8), [(2, 2), (5, 6), (8, 8)]);
        assert_eq!(missing_ranges(&[], 3), [(1, 3)]);
        assert_eq!(missing_ranges(&[1, 2], 2), NONE_MISSING);
        assert_eq!(missing_ranges(&[2, 1], 2), NONE_MISSING);
        assert_eq!(missing_ranges(&[0, 1, 2, 9], 3), [(3, 3)]);
        assert_eq!(missing_ranges(&[5], 0), NONE_MISSING);
        assert_eq!(missing_ranges(&[1, i64::MAX], i64::MAX), [(2, i64::MAX - 1)]);

        let with_a_gap = only_series(vec![
            segment_file("e-seg001_x1dints.fits", Some(8), &[1, 2, 3], 1),
            segment_file("e-seg003_x1dints.fits", Some(8), &[7, 8], 3),
        ]);
        assert_eq!(with_a_gap.completeness, [record("e", Some(8), 5, &[(4, 6)], Some(false))]);
        assert_eq!(with_a_gap.warnings, ["exposure e: 5 of 8 integrations found, missing 4 to 6"]);
        assert_eq!(with_a_gap.notes, ["5 integrations from 2 EXTRACT1D table(s) in 2 file(s)"]);

        let with_three_gaps = only_series(vec![segment_file("e-seg001_x1dints.fits", Some(8), &[1, 3, 4, 7], 1)]);
        assert_eq!(with_three_gaps.completeness, [record("e", Some(8), 4, &[(2, 2), (5, 6), (8, 8)], Some(false))]);
        assert_eq!(with_three_gaps.warnings, ["exposure e: 4 of 8 integrations found, missing 2, 5 to 6, 8"]);

        let without_nints = only_series(vec![
            segment_file("e-seg001_x1dints.fits", None, &[1, 2, 3], 1),
            segment_file("e-seg003_x1dints.fits", None, &[7, 8], 3),
        ]);
        assert_eq!(without_nints.completeness, [record("e", None, 5, &[], None)]);
        assert_eq!(
            without_nints.notes,
            [
                "5 integrations from 2 EXTRACT1D table(s) in 2 file(s)",
                "exposure e: NINTS card missing, completeness not checked"
            ]
        );
        assert_eq!(without_nints.warnings, NO_TEXT);

        let beyond_nints = only_series(vec![segment_file("e-seg001_x1dints.fits", Some(2), &[1, 2, 3], 1)]);
        assert_eq!(beyond_nints.completeness, [record("e", Some(2), 3, &[], Some(false))]);
        assert_eq!(
            beyond_nints.notes,
            [
                "3 integrations from 1 EXTRACT1D table(s) in 1 file(s)",
                "exposure e: integration numbers reach 3, above NINTS = 2"
            ]
        );
        assert_eq!(beyond_nints.warnings, NO_TEXT);

        let complete = only_series(vec![segment_file("e-seg001_x1dints.fits", Some(3), &[1, 2, 3], 1)]);
        assert_eq!(complete.completeness, [record("e", Some(3), 3, &[], Some(true))]);
        assert_eq!(complete.notes, ["3 integrations from 1 EXTRACT1D table(s) in 1 file(s)"]);
        assert_eq!(complete.warnings, NO_TEXT);
    }

    #[test]
    fn overlapping_integrations_of_one_exposure_are_refused_naming_both_files() {
        let error = build_set(vec![
            segment_file("e-seg001_x1dints.fits", Some(4), &[1, 2, 3], 1),
            segment_file("e-seg002_x1dints.fits", Some(4), &[3, 4], 2),
        ])
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "e-seg001_x1dints.fits (HDU 1) and e-seg002_x1dints.fits (HDU 1) both hold integration 3 of exposure e: overlapping INT_NUM ranges cannot be stitched"
        );

        let repeated_in_one_table = build_set(vec![segment_file("e-seg001_x1dints.fits", Some(4), &[1, 2, 2], 1)]).unwrap_err();
        assert_eq!(
            repeated_in_one_table.to_string(),
            "e-seg001_x1dints.fits (HDU 1) and e-seg001_x1dints.fits (HDU 1) both hold integration 2 of exposure e: overlapping INT_NUM ranges cannot be stitched"
        );
    }

    #[test]
    fn several_exposures_share_a_series_and_integration_numbers_may_repeat() {
        let two_exposures = only_series(vec![
            exposure_file("a-seg001_x1dints.fits", Some(2), Some(60000.0), &[1, 2], 1),
            exposure_file("b-seg001_x1dints.fits", Some(2), Some(60001.0), &[1, 2], 1),
        ]);
        assert_eq!(two_exposures.int_num, [1, 2, 1, 2]);
        assert_eq!(two_exposures.file_index, [0, 0, 1, 1]);
        let mid_time = |start: f64, number: i32| start + 0.1 * f64::from(number) + 0.002;
        assert_eq!(
            two_exposures.time(),
            [mid_time(60000.0, 1), mid_time(60000.0, 2), mid_time(60001.0, 1), mid_time(60001.0, 2)]
        );
        assert_eq!(two_exposures.completeness, [record("a", Some(2), 2, &[], Some(true)), record("b", Some(2), 2, &[], Some(true))]);
        assert_eq!(two_exposures.warnings, NO_TEXT);

        let same_start_other_nints = only_series(vec![
            exposure_file("a-seg001_x1dints.fits", Some(2), Some(60000.0), &[1, 2], 1),
            exposure_file("b-seg001_x1dints.fits", Some(3), Some(60000.0), &[3], 1),
        ]);
        assert_eq!(
            same_start_other_nints.completeness,
            [record("a", Some(2), 2, &[], Some(true)), record("b", Some(3), 1, &[(1, 2)], Some(false))]
        );

        let named_alike = only_series(vec![
            exposure_file("e-seg001_x1dints.fits", Some(4), None, &[1, 2], 1),
            exposure_file("e-seg002_x1dints.fits", Some(4), None, &[3, 4], 2),
        ]);
        assert_eq!(named_alike.completeness, [record("e", Some(4), 4, &[], Some(true))]);
        let named_apart = only_series(vec![
            exposure_file("a-seg001_x1dints.fits", Some(2), None, &[1, 2], 1),
            exposure_file("b-seg001_x1dints.fits", Some(2), None, &[3, 4], 1),
        ]);
        assert_eq!(
            named_apart.completeness,
            [record("a", Some(2), 2, &[], Some(true)), record("b", Some(2), 2, &[(1, 2)], Some(false))]
        );
        assert_eq!(named_apart.warnings, ["exposure b: 2 of 2 integrations found, missing 1 to 2"]);
        assert_eq!(
            named_apart.notes,
            [
                "4 integrations from 2 EXTRACT1D table(s) in 2 file(s)",
                "exposure b: integration numbers reach 4, above NINTS = 2"
            ]
        );

        let two_runs = only_series(vec![level_3_file(&[&[1, 2], &[3, 4], &[1, 2], &[3, 4]])]);
        assert_eq!(two_runs.n_integrations, 8);
        assert_eq!(two_runs.int_num, [1, 2, 3, 4, 1, 2, 3, 4]);
        assert_eq!(
            two_runs.completeness,
            [
                record("l3, exposure 1 of 2", Some(4), 4, &[], Some(true)),
                record("l3, exposure 2 of 2", Some(4), 4, &[], Some(true))
            ]
        );
        assert_eq!(two_runs.warnings, NO_TEXT);

        let one_run = only_series(vec![level_3_file(&[&[1, 2], &[3, 4]])]);
        assert_eq!(one_run.completeness, [record("l3", Some(4), 4, &[], Some(true))]);

        let a_run_restarts = only_series(vec![level_3_file(&[&[1, 2], &[2, 3]])]);
        assert_eq!(
            a_run_restarts.completeness,
            [
                record("l3, exposure 1 of 2", Some(4), 2, &[(3, 4)], Some(false)),
                record("l3, exposure 2 of 2", Some(4), 2, &[(1, 1), (4, 4)], Some(false))
            ]
        );
        assert_eq!(
            a_run_restarts.warnings,
            [
                "exposure l3, exposure 1 of 2: 2 of 4 integrations found, missing 3 to 4",
                "exposure l3, exposure 2 of 2: 2 of 4 integrations found, missing 1, 4"
            ]
        );
    }

    #[test]
    fn a_level_3_file_and_a_level_2b_file_of_the_same_series_are_refused_naming_both() {
        let product = |detector: &str| {
            file_of("jw-o001_x1dints.fits", 3, Some(2), Some(60000.0), vec![table_of(detector, Some(-1), 1, &rows_from(60000.0, &[1, 2], 1))])
        };
        let segment = || exposure_file("jw001-seg001_nrs1_x1dints.fits", Some(2), Some(60000.0), &[1, 2], 1);
        let refusal = "jw-o001_x1dints.fits is a level 3 product and jw001-seg001_nrs1_x1dints.fits is a level 2b product, and both hold the series NRS1: the integrations are the same but FLUX is not (level 3 is extracted after outlier detection); open one level at a time";
        assert_eq!(build_set(vec![product("NRS1"), segment()]).unwrap_err().to_string(), refusal);
        assert_eq!(build_set(vec![segment(), product("NRS1")]).unwrap_err().to_string(), refusal);

        let apart = build_set(vec![product("NRS2"), segment()]).unwrap();
        let detectors: Vec<&str> = apart.series.iter().map(|series| series.key.detector.as_str()).collect();
        assert_eq!(detectors, ["NRS2", "NRS1"]);
    }

    #[test]
    fn exposure_names_drop_the_product_suffix_and_the_segment() {
        assert_eq!(exposure_name("jw02327001001_03101_00001-seg003_nrs1_x1dints.fits"), "jw02327001001_03101_00001");
        assert_eq!(
            exposure_name("jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits"),
            "jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048"
        );
        assert_eq!(exposure_name("x_x1dints.fits.gz"), "x");
        assert_eq!(exposure_name("plain.fits"), "plain");
        assert_eq!(exposure_name("a-seg12_nrs2_X1DINTS.FITS"), "a");
        assert_eq!(exposure_name("noext"), "noext");
        assert_eq!(exposure_name("a-segment-seg2_x1dints.fits"), "a-segment");
        assert_eq!(exposure_name("a-seg_x1dints.fits"), "a-seg");
    }
}
