use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use anyhow::{bail, Result};

use super::stitch::{exposures, refuse_mixed_levels, reorder, rows_by_number, rows_by_time, time_axis, Part};
use super::{sample_is_valid, SeriesKey, TsoFile, TsoSeries, TsoSet, TsoTimeScale, GRID_TOLERANCE_UM, UNKNOWN_UNIT};
use crate::core::analysis::x1d::EXTNAME_EXTRACT1D;
use crate::infra::fits::x1dints::{read_x1dints_file, ReadBudget, X1dIntsFile, X1dIntsTable, TSO_SET_BYTE_CAP};
use crate::types::image_ref::path_key;

pub const TSO_MAX_FILES: usize = 256;

const EXP_TYPE_SOSS: &str = "NIS_SOSS";
const NO_UNIT: &str = "no unit";
const UNKNOWN_CAL_VER: &str = "unknown";

struct Member {
    file_index: usize,
    table: X1dIntsTable,
}

struct Group {
    first: Member,
    rest: Vec<Member>,
}

pub fn series_label(key: &SeriesKey, keys: &[SeriesKey]) -> String {
    let another_order = keys.iter().any(|other| other.detector == key.detector && other.sporder != key.sporder);
    match key.sporder {
        Some(order) if order >= 2 || another_order => format!("{}, order {order}", key.detector),
        _ => key.detector.clone(),
    }
}

pub fn read_set(paths: &[String]) -> Result<TsoSet> {
    read_set_with(paths, TSO_SET_BYTE_CAP)
}

pub(crate) fn read_set_with(paths: &[String], cap: u64) -> Result<TsoSet> {
    if paths.is_empty() {
        bail!("no file given: pass at least one x1dints file");
    }
    if paths.iter().any(|path| path.trim().is_empty()) {
        bail!("path must name an x1dints file, got an empty string");
    }
    let mut known = HashSet::new();
    let mut distinct: Vec<(String, &String)> = Vec::new();
    let mut repeats: Vec<String> = Vec::new();
    for path in paths {
        let key = path_key(path);
        if known.insert(key.clone()) {
            distinct.push((key, path));
            continue;
        }
        let name = Path::new(path).file_name().map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
        let note = format!("{name} was given twice: read once");
        if !repeats.contains(&note) {
            repeats.push(note);
        }
    }
    if distinct.len() > TSO_MAX_FILES {
        bail!("{} files given: at most {TSO_MAX_FILES} are opened as one set", distinct.len());
    }
    distinct.sort_by(|(key, _), (other, _)| key.cmp(other));
    let mut budget = ReadBudget { cap, used: 0 };
    let files = distinct.iter().map(|(_, path)| read_x1dints_file(path, &mut budget)).collect::<Result<Vec<_>>>()?;
    let mut set = build_set(files)?;
    set.notes.extend(repeats);
    Ok(set)
}

pub fn build_set(files: Vec<X1dIntsFile>) -> Result<TsoSet> {
    let mut set_files = Vec::with_capacity(files.len());
    let mut notes = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    for (file_index, read) in files.into_iter().enumerate() {
        notes.extend(read.notes.iter().map(|note| format!("{}: {note}", read.file.file_name)));
        for table in read.tables {
            let member = Member { file_index, table };
            match groups.iter_mut().find(|group| group.first.table.key == member.table.key) {
                Some(group) => group.rest.push(member),
                None => groups.push(Group { first: member, rest: Vec::new() }),
            }
        }
        set_files.push(read.file);
    }
    let keys: Vec<SeriesKey> = groups.iter().map(|group| group.first.table.key.clone()).collect();
    let mut series = Vec::with_capacity(groups.len());
    for group in groups {
        let label = series_label(&group.first.table.key, &keys);
        series.push(build_series(&set_files, &label, group)?);
    }
    let warnings = pipeline_versions_warning(&set_files).into_iter().collect();
    Ok(TsoSet { files: set_files, series, notes, warnings })
}

fn pipeline_versions_warning(files: &[TsoFile]) -> Option<String> {
    let versions: BTreeSet<&str> = files.iter().map(|file| file.cal_ver.as_deref().unwrap_or(UNKNOWN_CAL_VER)).collect();
    (versions.len() > 1).then(|| {
        let list: Vec<&str> = versions.into_iter().collect();
        format!("files were calibrated by different pipeline versions ({}): check before stitching them", list.join(", "))
    })
}

fn refuse_another_shape(files: &[TsoFile], label: &str, first: &Member, other: &Member) -> Result<()> {
    let name = &files[other.file_index].file_name;
    let first_name = &files[first.file_index].file_name;
    let hdu = other.table.hdu;
    if other.table.n_channels != first.table.n_channels {
        bail!(
            "{name}: HDU {hdu} ({EXTNAME_EXTRACT1D}) has {} channels, but the series {label} has {} channels in {first_name}: files of different configurations cannot be stitched",
            other.table.n_channels,
            first.table.n_channels
        );
    }
    if other.table.flux_unit != first.table.flux_unit {
        bail!(
            "{name}: HDU {hdu} ({EXTNAME_EXTRACT1D}) gives FLUX in {}, but the series {label} is in {} in {first_name}",
            other.table.flux_unit.as_deref().unwrap_or(NO_UNIT),
            first.table.flux_unit.as_deref().unwrap_or(NO_UNIT)
        );
    }
    Ok(())
}

fn valid_counts(rows: &X1dIntsTable) -> Vec<u32> {
    let width = rows.n_channels;
    (0..rows.int_num.len())
        .map(|row| {
            let valid = (row * width..(row + 1) * width)
                .filter(|&sample| sample_is_valid(rows.wavelength_um[sample], rows.flux[sample], rows.dq[sample]));
            valid.count() as u32
        })
        .collect()
}

fn shared_grid(wavelength_um: &[f64], n_channels: usize) -> (Vec<f64>, bool) {
    let mut grid_um = vec![f64::NAN; n_channels];
    let mut common = true;
    for (sample, &wavelength) in wavelength_um.iter().enumerate() {
        let of_channel = &mut grid_um[sample % n_channels];
        if !wavelength.is_finite() {
            continue;
        }
        if of_channel.is_nan() {
            *of_channel = wavelength;
        } else if (wavelength - *of_channel).abs() > GRID_TOLERANCE_UM {
            common = false;
        }
    }
    (grid_um, common)
}

fn build_series(files: &[TsoFile], label: &str, group: Group) -> Result<TsoSeries> {
    let Group { first, rest } = group;
    for other in &rest {
        refuse_another_shape(files, label, &first, other)?;
    }
    let mut from_files: Vec<usize> = std::iter::once(&first).chain(&rest).map(|member| member.file_index).collect();
    let n_tables = from_files.len();
    from_files.dedup();
    let sources: Vec<&TsoFile> = from_files.iter().map(|&file_index| &files[file_index]).collect();
    refuse_mixed_levels(&sources, label)?;
    let outlier_detection = sources.iter().find_map(|file| file.outlier_detection.clone());
    let is_soss = sources[0].exp_type.as_deref() == Some(EXP_TYPE_SOSS);

    let Member { file_index, table: mut rows } = first;
    let mut parts = vec![Part { file_index, hdu: rows.hdu, rows: 0..rows.int_num.len() }];
    let mut file_of_row = vec![file_index as u32; rows.int_num.len()];
    for Member { file_index, mut table } in rest {
        let start = rows.int_num.len();
        rows.int_num.append(&mut table.int_num);
        rows.segment.append(&mut table.segment);
        rows.mjd_utc_mid.append(&mut table.mjd_utc_mid);
        rows.bmjd_tdb_mid.append(&mut table.bmjd_tdb_mid);
        rows.wavelength_um.append(&mut table.wavelength_um);
        rows.flux.append(&mut table.flux);
        rows.flux_error.append(&mut table.flux_error);
        rows.dq.append(&mut table.dq);
        rows.npixels.append(&mut table.npixels);
        parts.push(Part { file_index, hdu: table.hdu, rows: start..rows.int_num.len() });
        file_of_row.resize(rows.int_num.len(), file_index as u32);
    }

    let found = exposures(files, &parts, &rows.int_num)?;
    let axis = time_axis(&rows.bmjd_tdb_mid, &rows.mjd_utc_mid, rows.time_origin);
    let order = match axis.scale {
        TsoTimeScale::BmjdTdb => rows_by_time(&rows.bmjd_tdb_mid),
        TsoTimeScale::MjdUtc => rows_by_time(&rows.mjd_utc_mid),
        TsoTimeScale::Index => rows_by_number(&found.ordinal, &rows.segment, &rows.int_num),
    };
    let n_channels = rows.n_channels;
    if order.windows(2).any(|pair| pair[0] > pair[1]) {
        reorder(&mut rows.int_num, &order, 1);
        reorder(&mut rows.segment, &order, 1);
        reorder(&mut file_of_row, &order, 1);
        reorder(&mut rows.mjd_utc_mid, &order, 1);
        reorder(&mut rows.bmjd_tdb_mid, &order, 1);
        reorder(&mut rows.wavelength_um, &order, n_channels);
        reorder(&mut rows.flux, &order, n_channels);
        reorder(&mut rows.flux_error, &order, n_channels);
        reorder(&mut rows.dq, &order, n_channels);
        reorder(&mut rows.npixels, &order, n_channels);
    }

    let n_integrations = rows.int_num.len();
    let n_valid = valid_counts(&rows);
    let (grid_um, common_grid) = shared_grid(&rows.wavelength_um, n_channels);
    let flux_unit = rows.flux_unit.unwrap_or_else(|| UNKNOWN_UNIT.to_string());

    let mut notes = vec![format!(
        "{n_integrations} integrations from {n_tables} {EXTNAME_EXTRACT1D} table(s) in {} file(s)",
        sources.len()
    )];
    if let Some(value) = &outlier_detection {
        notes.push(format!(
            "extracted after outlier detection (S_OUTLIR = {value}): a deep, short event can lose pixels, check NPIXELS inside the event"
        ));
    }
    notes.extend(found.notes);

    let mut warnings: Vec<String> = axis.warning.into_iter().collect();
    warnings.extend(found.warnings);
    if !common_grid {
        warnings.push(format!(
            "the wavelength grid differs between integrations by more than {GRID_TOLERANCE_UM:e} um: sums use the wavelengths of each row, and the masked mode and the median and mean spectra are not available"
        ));
    }
    if is_soss {
        warnings.push(format!(
            "FLUX is labelled {flux_unit} but NIRISS SOSS numbers are not on that scale: treat the flux as relative"
        ));
    }

    Ok(TsoSeries {
        key: rows.key,
        n_integrations,
        n_channels,
        int_num: rows.int_num,
        segment: rows.segment,
        file_index: file_of_row,
        n_valid,
        mjd_utc_mid: rows.mjd_utc_mid,
        bmjd_tdb_mid: rows.bmjd_tdb_mid,
        time_scale: axis.scale,
        time_source: axis.source.to_string(),
        wavelength_um: rows.wavelength_um,
        flux: rows.flux,
        flux_error: rows.flux_error,
        dq: rows.dq,
        npixels: rows.npixels,
        grid_um,
        flux_unit,
        common_grid,
        outlier_detection,
        completeness: found.completeness,
        notes,
        warnings,
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::build_set;
    use crate::core::analysis::tso::test_support::synth_file;
    use crate::core::analysis::tso::{SeriesKey, TsoSet};
    use crate::infra::fits::x1dints::test_fixtures::{ramp_row, SynthRow};
    use crate::infra::fits::x1dints::{TimeOrigin, X1dIntsFile, X1dIntsTable};

    pub fn table_of(detector: &str, sporder: Option<i64>, hdu: usize, rows: &[SynthRow]) -> X1dIntsTable {
        X1dIntsTable {
            hdu,
            extver: Some(hdu as i64),
            key: SeriesKey { detector: detector.to_string(), sporder },
            n_channels: rows.first().map_or(0, |row| row.wavelength.len()),
            flux_unit: Some("Jy".to_string()),
            time_origin: TimeOrigin::Column,
            int_num: rows.iter().map(|row| i64::from(row.int_num)).collect(),
            segment: rows.iter().map(|row| i64::from(row.segment)).collect(),
            n_alongdisp: rows.iter().map(|row| row.n_alongdisp).collect(),
            mjd_utc_mid: rows.iter().map(|row| row.mjd_avg).collect(),
            bmjd_tdb_mid: rows.iter().map(|row| row.tdb_mid).collect(),
            wavelength_um: rows.iter().flat_map(|row| row.wavelength.iter().copied()).collect(),
            flux: rows.iter().flat_map(|row| row.flux.iter().copied()).collect(),
            flux_error: rows.iter().flat_map(|row| row.flux_error.iter().copied()).collect(),
            dq: rows.iter().flat_map(|row| row.dq.iter().copied()).collect(),
            npixels: rows.iter().flat_map(|row| row.npixels.iter().copied()).collect(),
            rows_dropped_empty: 0,
        }
    }

    pub fn file_of(name: &str, level: u8, nints: Option<i64>, expstart: Option<f64>, tables: Vec<X1dIntsTable>) -> X1dIntsFile {
        let mut file = synth_file(name, "NRS_BRIGHTOBJ", "CLEAR", level);
        file.nints = nints;
        file.expstart = expstart;
        file.n_extract1d = tables.len();
        file.outlier_detection = (level == 3).then(|| "COMPLETE".to_string());
        X1dIntsFile { file, tables, notes: Vec::new() }
    }

    pub fn ramp_set(detector: &str, sporder: Option<i64>, n_rows: usize, n_channels: usize) -> TsoSet {
        let rows: Vec<SynthRow> =
            (0..n_rows).map(|row| ramp_row(row as i32 + 1, 60000.0 + 0.01 * row as f64, n_channels)).collect();
        let table = table_of(detector, sporder, 1, &rows);
        build_set(vec![file_of("ramp_x1dints.fits", 2, Some(n_rows as i64), Some(60000.0), vec![table])])
            .expect("a ramp table is one complete series")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{file_of, ramp_set, table_of};
    use super::*;
    use crate::core::analysis::tso::{ExposureCompleteness, DATAMODEL_ROWS};
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::x1dints::test_fixtures::{ramp_row, rows_hdu, series_primary, write_series_file, SynthRow, SynthTable};
    use crate::infra::fits::x1dints::TimeOrigin;

    const NO_TEXT: [&str; 0] = [];
    const NO_USABLE_TIMES_IN_THE_FILE: &str = "no usable integration times: TDB-MID repeats values and MJD-AVG has NaN values; the abscissa is the integration number";
    const OUTLIER_NOTE: &str = "extracted after outlier detection (S_OUTLIR = COMPLETE): a deep, short event can lose pixels, check NPIXELS inside the event";

    fn key(detector: &str, sporder: Option<i64>) -> SeriesKey {
        SeriesKey { detector: detector.to_string(), sporder }
    }

    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|value| value.to_bits()).collect()
    }

    fn segment_rows(integrations: &[i32], segment: i32) -> Vec<SynthRow> {
        integrations
            .iter()
            .map(|&number| SynthRow { segment, ..ramp_row(number, 60000.0 + 0.1 * f64::from(number), 4) })
            .collect()
    }

    fn segment_file(name: &str, nints: Option<i64>, integrations: &[i32], segment: i32) -> X1dIntsFile {
        file_of(name, 2, nints, Some(60000.0), vec![table_of("NRS1", Some(-1), 1, &segment_rows(integrations, segment))])
    }

    fn only_series(files: Vec<X1dIntsFile>) -> TsoSeries {
        let mut set = build_set(files).unwrap();
        assert_eq!(set.series.len(), 1);
        set.series.remove(0)
    }

    fn record(exposure: &str, nints: i64, found: usize, missing: &[(i64, i64)], complete: bool) -> ExposureCompleteness {
        ExposureCompleteness { exposure: exposure.to_string(), nints: Some(nints), found, missing: missing.to_vec(), complete: Some(complete) }
    }

    #[test]
    fn segments_given_out_of_order_become_one_series_in_time_order() {
        let set = build_set(vec![
            segment_file("e-seg002_x1dints.fits", Some(4), &[3, 4], 2),
            segment_file("e-seg001_x1dints.fits", Some(4), &[1, 2], 1),
        ])
        .unwrap();
        let names: Vec<&str> = set.files.iter().map(|file| file.file_name.as_str()).collect();
        assert_eq!(names, ["e-seg002_x1dints.fits", "e-seg001_x1dints.fits"]);
        assert_eq!(set.notes, NO_TEXT);
        assert_eq!(set.warnings, NO_TEXT);
        assert_eq!(set.series.len(), 1);

        let series = &set.series[0];
        assert_eq!(series.key, key("NRS1", Some(-1)));
        assert_eq!(series.n_integrations, 4);
        assert_eq!(series.n_channels, 4);
        assert_eq!(series.int_num, [1, 2, 3, 4]);
        assert_eq!(series.segment, [1, 1, 2, 2]);
        assert_eq!(series.file_index, [1, 1, 0, 0]);
        assert_eq!(series.time_scale, TsoTimeScale::BmjdTdb);
        assert_eq!(series.time_source, "TDB-MID column");
        let utc: Vec<f64> = (1..=4).map(|number| 60000.0 + 0.1 * f64::from(number)).collect();
        let tdb: Vec<f64> = utc.iter().map(|time| time + 0.002).collect();
        assert_eq!(series.time(), tdb);
        assert_eq!(series.bmjd_tdb_mid, tdb);
        assert_eq!(series.mjd_utc_mid, utc);
        let flux: Vec<f64> = (1..=4).flat_map(|number| (0..4).map(move |channel| f64::from(10 * number + channel))).collect();
        assert_eq!(series.flux, flux);
        let grid: Vec<f64> = (0..4).map(|channel| 1.0 + 0.1 * f64::from(channel)).collect();
        assert_eq!(series.wavelength_um, grid.repeat(4));
        assert_eq!(series.flux_error, [0.5; 16]);
        assert_eq!(series.dq, [0; 16]);
        assert_eq!(series.npixels, [7.0; 16]);
        assert_eq!(series.n_valid, [4, 4, 4, 4]);
        assert_eq!(series.grid_um, grid);
        assert!(series.common_grid);
        assert_eq!(series.flux_unit, "Jy");
        assert_eq!(series.outlier_detection, None);
        assert_eq!(series.completeness, [record("e", 4, 4, &[], true)]);
        assert_eq!(series.warnings, NO_TEXT);
        assert_eq!(series.notes, ["4 integrations from 2 EXTRACT1D table(s) in 2 file(s)"]);
    }

    #[test]
    fn detectors_and_orders_are_never_merged_and_labels_print_an_order_only_when_it_tells_series_apart() {
        let product = file_of(
            "l3_x1dints.fits",
            3,
            Some(4),
            Some(60000.0),
            vec![
                table_of("NRS1", Some(-1), 1, &segment_rows(&[1, 2], 1)),
                table_of("NRS2", Some(-1), 2, &segment_rows(&[1, 2], 1)),
                table_of("NRS1", Some(-1), 3, &segment_rows(&[3, 4], 2)),
            ],
        );
        let set = build_set(vec![product]).unwrap();
        let keys: Vec<SeriesKey> = set.series.iter().map(|series| series.key.clone()).collect();
        assert_eq!(keys, [key("NRS1", Some(-1)), key("NRS2", Some(-1))]);
        assert_eq!(set.series[0].n_integrations, 4);
        assert_eq!(set.series[0].int_num, [1, 2, 3, 4]);
        assert_eq!(set.series[0].segment, [1, 1, 2, 2]);
        assert_eq!(set.series[0].notes[0], "4 integrations from 2 EXTRACT1D table(s) in 1 file(s)");
        assert_eq!(set.series[1].n_integrations, 2);
        assert_eq!(set.series[1].int_num, [1, 2]);
        assert_eq!(set.series[1].notes[0], "2 integrations from 1 EXTRACT1D table(s) in 1 file(s)");
        let labels: Vec<String> = keys.iter().map(|one| series_label(one, &keys)).collect();
        assert_eq!(labels, ["NRS1", "NRS2"]);

        let soss = [key("NIS", Some(1)), key("NIS", Some(2)), key("NIS", Some(3))];
        let labels: Vec<String> = soss.iter().map(|one| series_label(one, &soss)).collect();
        assert_eq!(labels, ["NIS, order 1", "NIS, order 2", "NIS, order 3"]);

        let alone = |one: SeriesKey| series_label(&one, std::slice::from_ref(&one));
        assert_eq!(alone(key("NRCALONG", Some(1))), "NRCALONG");
        assert_eq!(alone(key("NRS1", Some(0))), "NRS1");
        assert_eq!(alone(key("MIRIMAGE", None)), "MIRIMAGE");
        assert_eq!(alone(key("NIS", Some(2))), "NIS, order 2");

        let prism_and_grating = [key("NRS1", Some(0)), key("NRS1", Some(-1))];
        assert_eq!(series_label(&prism_and_grating[0], &prism_and_grating), "NRS1, order 0");
        assert_eq!(series_label(&prism_and_grating[1], &prism_and_grating), "NRS1, order -1");

        let with_and_without_order = [key("NRS1", Some(1)), key("NRS1", None), key("NRS2", Some(1))];
        assert_eq!(series_label(&with_and_without_order[0], &with_and_without_order), "NRS1, order 1");
        assert_eq!(series_label(&with_and_without_order[1], &with_and_without_order), "NRS1");
        assert_eq!(series_label(&with_and_without_order[2], &with_and_without_order), "NRS2");
    }

    fn timed_series(tdb_mid: [f64; 3], mjd_avg: [f64; 3], origin: TimeOrigin) -> TsoSeries {
        let rows: Vec<SynthRow> = (0..3)
            .map(|row| SynthRow { mjd_avg: mjd_avg[row], tdb_mid: tdb_mid[row], ..ramp_row(row as i32 + 1, 60000.0, 4) })
            .collect();
        let mut table = table_of("NRS1", Some(-1), 1, &rows);
        table.time_origin = origin;
        only_series(vec![file_of("t_x1dints.fits", 2, Some(3), Some(60000.0), vec![table])])
    }

    #[test]
    fn one_time_column_serves_the_series_and_there_is_no_row_by_row_fallback() {
        let nan = f64::NAN;

        let repeated = timed_series([60000.5; 3], [60000.4, nan, nan], TimeOrigin::Column);
        assert_eq!(repeated.time_scale, TsoTimeScale::Index);
        assert_eq!(repeated.time(), [1.0, 2.0, 3.0]);
        assert_eq!(repeated.time_source, "integration number (no usable times)");
        assert_eq!(repeated.warnings, [NO_USABLE_TIMES_IN_THE_FILE]);
        assert_eq!(repeated.n_integrations, 3);
        assert_eq!(repeated.bmjd_tdb_mid, [60000.5; 3]);
        assert_eq!(bits(&repeated.mjd_utc_mid), bits(&[60000.4, nan, nan]));

        let utc_only = timed_series([60000.1, nan, 60000.3], [60000.0, 60000.1, 60000.2], TimeOrigin::Column);
        assert_eq!(utc_only.time_scale, TsoTimeScale::MjdUtc);
        assert_eq!(utc_only.time(), [60000.0, 60000.1, 60000.2]);
        assert_eq!(utc_only.time_source, "MJD-AVG (UTC, not barycentric)");
        assert_eq!(utc_only.warnings, ["TDB-MID has NaN values: times are MJD-AVG (UTC, not barycentric)"]);

        let utc_descending = timed_series([nan; 3], [60000.2, 60000.1, 60000.0], TimeOrigin::Column);
        assert_eq!(utc_descending.time_scale, TsoTimeScale::MjdUtc);
        assert_eq!(utc_descending.time(), [60000.0, 60000.1, 60000.2]);
        assert_eq!(utc_descending.int_num, [3, 2, 1]);
        assert_eq!(utc_descending.flux[..4], [30.0, 31.0, 32.0, 33.0]);
        assert_eq!(utc_descending.warnings, ["TDB-MID is missing: times are MJD-AVG (UTC, not barycentric)"]);

        let never_mixed = timed_series([60000.1, nan, 60000.3], [nan, 60000.1, nan], TimeOrigin::Column);
        assert_eq!(never_mixed.time_scale, TsoTimeScale::Index);
        assert_eq!(never_mixed.time(), [1.0, 2.0, 3.0]);
        assert_eq!(
            never_mixed.warnings,
            ["no usable integration times: TDB-MID has NaN values and MJD-AVG has NaN values; the abscissa is the integration number"]
        );

        let absent = timed_series([nan; 3], [nan; 3], TimeOrigin::Absent);
        assert_eq!(absent.time_scale, TsoTimeScale::Index);
        assert_eq!(absent.time_source, "integration number (no usable times)");
        assert_eq!(
            absent.warnings,
            ["no usable integration times: TDB-MID is missing and MJD-AVG is missing; the abscissa is the integration number"]
        );

        let from_cards = timed_series([60000.1, 60000.2, 60000.3], [60000.4, nan, nan], TimeOrigin::Keyword);
        assert_eq!(from_cards.time_scale, TsoTimeScale::BmjdTdb);
        assert_eq!(from_cards.time_source, "TDB-MID keyword");
        assert_eq!(from_cards.time(), [60000.1, 60000.2, 60000.3]);
        assert_eq!(from_cards.warnings, NO_TEXT);
    }

    #[test]
    fn rows_without_usable_times_are_ordered_by_exposure_segment_and_integration() {
        let untimed = |name: &str, integrations: &[i32], segment: i32| {
            let rows: Vec<SynthRow> = segment_rows(integrations, segment)
                .into_iter()
                .map(|row| SynthRow { mjd_avg: f64::NAN, tdb_mid: f64::NAN, ..row })
                .collect();
            file_of(name, 2, Some(4), Some(60000.0), vec![table_of("NRS1", Some(-1), 1, &rows)])
        };
        let series = only_series(vec![untimed("e-seg002_x1dints.fits", &[3, 4], 2), untimed("e-seg001_x1dints.fits", &[1, 2], 1)]);
        assert_eq!(series.time_scale, TsoTimeScale::Index);
        assert_eq!(series.int_num, [1, 2, 3, 4]);
        assert_eq!(series.segment, [1, 1, 2, 2]);
        assert_eq!(series.file_index, [1, 1, 0, 0]);
        assert_eq!(series.time(), [1.0, 2.0, 3.0, 4.0]);
        let flux: Vec<f64> = (1..=4).flat_map(|number| (0..4).map(move |channel| f64::from(10 * number + channel))).collect();
        assert_eq!(series.flux, flux);
        assert_eq!(series.completeness, [record("e", 4, 4, &[], true)]);
    }

    #[test]
    fn tables_of_one_series_must_share_channels_and_unit() {
        let one_row_file = |name: &str, expstart: f64, channels: usize, unit: Option<&str>| {
            let mut table = table_of("NRS1", Some(-1), 1, &[ramp_row(1, expstart, channels)]);
            table.flux_unit = unit.map(str::to_string);
            file_of(name, 2, Some(1), Some(expstart), vec![table])
        };
        let refusal = |first: X1dIntsFile, second: X1dIntsFile| build_set(vec![first, second]).unwrap_err().to_string();

        assert_eq!(
            refusal(one_row_file("a_x1dints.fits", 60000.0, 4, Some("Jy")), one_row_file("b_x1dints.fits", 60001.0, 5, Some("Jy"))),
            "b_x1dints.fits: HDU 1 (EXTRACT1D) has 5 channels, but the series NRS1 has 4 channels in a_x1dints.fits: files of different configurations cannot be stitched"
        );
        assert_eq!(
            refusal(one_row_file("a_x1dints.fits", 60000.0, 4, Some("Jy")), one_row_file("b_x1dints.fits", 60001.0, 4, Some("MJy"))),
            "b_x1dints.fits: HDU 1 (EXTRACT1D) gives FLUX in MJy, but the series NRS1 is in Jy in a_x1dints.fits"
        );
        assert_eq!(
            refusal(one_row_file("a_x1dints.fits", 60000.0, 4, Some("Jy")), one_row_file("b_x1dints.fits", 60001.0, 4, None)),
            "b_x1dints.fits: HDU 1 (EXTRACT1D) gives FLUX in no unit, but the series NRS1 is in Jy in a_x1dints.fits"
        );
        assert_eq!(
            refusal(one_row_file("a_x1dints.fits", 60000.0, 4, None), one_row_file("b_x1dints.fits", 60001.0, 4, Some("Jy"))),
            "b_x1dints.fits: HDU 1 (EXTRACT1D) gives FLUX in Jy, but the series NRS1 is in no unit in a_x1dints.fits"
        );

        let unlabelled =
            only_series(vec![one_row_file("a_x1dints.fits", 60000.0, 4, None), one_row_file("b_x1dints.fits", 60001.0, 4, None)]);
        assert_eq!(unlabelled.flux_unit, "unknown");
        assert_eq!(unlabelled.n_integrations, 2);
    }

    fn series_with_padding(change: impl Fn(&mut Vec<SynthRow>)) -> TsoSeries {
        let mut rows: Vec<SynthRow> = (1..=3).map(|number| ramp_row(number, 60000.0 + f64::from(number), 4)).collect();
        rows[0].wavelength[2] = f64::NAN;
        rows[0].flux[2] = f64::NAN;
        rows[0].flux_error[2] = f64::NAN;
        rows[0].dq[2] = 1;
        rows[0].npixels[2] = 0.0;
        change(&mut rows);
        only_series(vec![file_of("g_x1dints.fits", 2, Some(3), Some(60000.0), vec![table_of("NRS1", Some(-1), 1, &rows)])])
    }

    #[test]
    fn the_grid_is_the_first_finite_wavelength_of_each_channel_and_small_differences_are_tolerated() {
        let grid: Vec<f64> = (0..4).map(|channel| 1.0 + 0.1 * f64::from(channel)).collect();

        let padded = series_with_padding(|_| {});
        assert_eq!(padded.grid_um, grid);
        assert_eq!(padded.n_valid, [3, 4, 4]);
        assert!(padded.common_grid);
        assert_eq!(padded.warnings, NO_TEXT);

        let within_tolerance = series_with_padding(|rows| rows[2].wavelength[1] = 1.1 + 5e-10);
        assert_eq!(within_tolerance.grid_um, grid);
        assert!(within_tolerance.common_grid);
        assert_eq!(within_tolerance.warnings, NO_TEXT);

        let beyond_tolerance = series_with_padding(|rows| rows[2].wavelength[1] = 1.1 + 2e-9);
        assert_eq!(beyond_tolerance.grid_um, grid);
        assert!(!beyond_tolerance.common_grid);
        assert_eq!(
            beyond_tolerance.warnings,
            ["the wavelength grid differs between integrations by more than 1e-9 um: sums use the wavelengths of each row, and the masked mode and the median and mean spectra are not available"]
        );

        let flagged = series_with_padding(|rows| {
            rows[1].dq[0] = 1;
            rows[1].dq[3] = 2;
            rows[2].flux[0] = f64::NAN;
        });
        assert_eq!(flagged.n_valid, [3, 3, 3]);
        assert!(flagged.common_grid);

        let never_finite = series_with_padding(|rows| rows.iter_mut().for_each(|row| row.wavelength[3] = f64::NAN));
        assert_eq!(bits(&never_finite.grid_um), bits(&[grid[0], grid[1], grid[2], f64::NAN]));
        assert_eq!(never_finite.n_valid, [2, 3, 3]);
        assert!(never_finite.common_grid);
    }

    #[test]
    fn notes_of_the_reader_and_caveats_of_the_mode_reach_the_set() {
        let mut with_a_reader_note = segment_file("e-seg001_x1dints.fits", Some(2), &[1, 2], 1);
        with_a_reader_note.notes.push("HDU 1 (EXTRACT1D): 1 row(s) with N_ALONGDISP = 0 dropped".to_string());
        let set = build_set(vec![with_a_reader_note]).unwrap();
        assert_eq!(set.notes, ["e-seg001_x1dints.fits: HDU 1 (EXTRACT1D): 1 row(s) with N_ALONGDISP = 0 dropped"]);
        assert_eq!(set.warnings, NO_TEXT);

        let calibrated_by = |name: &str, detector: &str, version: Option<&str>| {
            let mut file = file_of(name, 2, Some(2), Some(60000.0), vec![table_of(detector, Some(-1), 1, &segment_rows(&[1, 2], 1))]);
            file.file.cal_ver = version.map(str::to_string);
            file
        };
        let two_versions = build_set(vec![
            calibrated_by("new_nrs1_x1dints.fits", "NRS1", Some("3.0.0")),
            calibrated_by("old_nrs2_x1dints.fits", "NRS2", Some("1.15.1")),
        ])
        .unwrap();
        assert_eq!(two_versions.series.len(), 2);
        assert_eq!(
            two_versions.warnings,
            ["files were calibrated by different pipeline versions (1.15.1, 3.0.0): check before stitching them"]
        );
        let one_without_the_card = build_set(vec![
            calibrated_by("new_nrs1_x1dints.fits", "NRS1", Some("3.0.0")),
            calibrated_by("bare_nrs2_x1dints.fits", "NRS2", None),
        ])
        .unwrap();
        assert_eq!(
            one_without_the_card.warnings,
            ["files were calibrated by different pipeline versions (3.0.0, unknown): check before stitching them"]
        );
        let one_version = build_set(vec![
            calibrated_by("new_nrs1_x1dints.fits", "NRS1", Some("3.0.0")),
            calibrated_by("new_nrs2_x1dints.fits", "NRS2", Some("3.0.0")),
        ])
        .unwrap();
        assert_eq!(one_version.warnings, NO_TEXT);

        let mut soss_table = table_of("NIS", Some(1), 1, &segment_rows(&[1, 2], 1));
        soss_table.flux_unit = Some("MJy".to_string());
        let mut soss_file = file_of("soss_x1dints.fits", 2, Some(2), Some(60000.0), vec![soss_table]);
        soss_file.file.exp_type = Some("NIS_SOSS".to_string());
        let soss = only_series(vec![soss_file]);
        assert_eq!(soss.flux_unit, "MJy");
        assert_eq!(soss.warnings, ["FLUX is labelled MJy but NIRISS SOSS numbers are not on that scale: treat the flux as relative"]);

        let product = file_of("l3_x1dints.fits", 3, Some(2), Some(60000.0), vec![table_of("NRS1", Some(-1), 1, &segment_rows(&[1, 2], 1))]);
        let extracted_late = only_series(vec![product]);
        assert_eq!(extracted_late.outlier_detection.as_deref(), Some("COMPLETE"));
        assert_eq!(extracted_late.notes, ["2 integrations from 1 EXTRACT1D table(s) in 1 file(s)", OUTLIER_NOTE]);
    }

    #[test]
    fn read_set_sorts_removes_repeats_shares_one_budget_and_refuses_bad_lists() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, integrations: &[i32], segment: i32| {
            let primary = series_primary(DATAMODEL_ROWS, &[("EXPSTART", "60000.0".to_string()), ("NINTS", "4".to_string())]);
            let table = SynthTable {
                extver: 1,
                detector: Some("NRS1"),
                sporder: Some(-1),
                flux_unit: Some("Jy"),
                rows: segment_rows(integrations, segment),
            };
            write_series_file(dir.path(), name, primary, vec![rows_hdu(&table)]).to_str().unwrap().to_string()
        };
        let b = write("b-seg002_x1dints.fits", &[3, 4], 2);
        let a = write("a-seg001_x1dints.fits", &[1, 2], 1);

        let given_twice = read_set(&[b.clone(), a.clone(), b.replace('\\', "/")]).unwrap();
        let names: Vec<&str> = given_twice.files.iter().map(|file| file.file_name.as_str()).collect();
        assert_eq!(names, ["a-seg001_x1dints.fits", "b-seg002_x1dints.fits"]);
        assert_eq!(given_twice.notes, ["b-seg002_x1dints.fits was given twice: read once"]);
        assert_eq!(given_twice.series.len(), 1);
        let stitched = &given_twice.series[0];
        assert_eq!(stitched.key, key("NRS1", Some(-1)));
        assert_eq!(stitched.int_num, [1, 2, 3, 4]);
        assert_eq!(stitched.segment, [1, 1, 2, 2]);
        assert_eq!(stitched.file_index, [0, 0, 1, 1]);
        assert_eq!(stitched.completeness, [record("a", 4, 4, &[], true)]);

        let in_order = read_set(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(in_order.notes, NO_TEXT);
        assert_eq!(in_order.series[0].int_num, stitched.int_num);
        assert_eq!(in_order.series[0].flux, stitched.flux);
        assert_eq!(in_order.series[0].file_index, stitched.file_index);

        let over_budget = read_set_with(&[a.clone(), b.clone()], 500).unwrap_err().to_string();
        assert!(over_budget.contains("more than the 500 bytes this reader loads at once"), "{over_budget}");
        assert!(over_budget.starts_with("b-seg002_x1dints.fits: "), "{over_budget}");
        assert!(read_set_with(&[a.clone(), b.clone()], 688).is_ok());

        assert_eq!(read_set(&[]).unwrap_err().to_string(), "no file given: pass at least one x1dints file");
        assert_eq!(read_set(&["  ".to_string()]).unwrap_err().to_string(), "path must name an x1dints file, got an empty string");
        assert_eq!(
            read_set(&[a.clone(), String::new()]).unwrap_err().to_string(),
            "path must name an x1dints file, got an empty string"
        );
        let absent = |count: usize| -> Vec<String> {
            (0..count).map(|index| dir.path().join(format!("absent-{index}_x1dints.fits")).to_str().unwrap().to_string()).collect()
        };
        assert_eq!(read_set(&absent(257)).unwrap_err().to_string(), "257 files given: at most 256 are opened as one set");
        let mut repeats_within_the_limit = absent(256);
        repeats_within_the_limit.extend(absent(3));
        let not_the_count = read_set(&repeats_within_the_limit).unwrap_err().to_string();
        assert!(!not_the_count.contains("files given"), "{not_the_count}");
    }

    #[test]
    fn the_builders_give_a_ready_series() {
        let set = ramp_set("NRS1", Some(0), 5, 6);
        assert_eq!(set.files.len(), 1);
        assert_eq!(set.series.len(), 1);
        let series = &set.series[0];
        assert_eq!(series.key, key("NRS1", Some(0)));
        assert_eq!(series.n_integrations, 5);
        assert_eq!(series.n_channels, 6);
        assert_eq!(series.time_scale, TsoTimeScale::BmjdTdb);
        assert_eq!(series.int_num, [1, 2, 3, 4, 5]);
        assert_eq!(series.completeness[0].complete, Some(true));
        assert_eq!(series.flux[series.sample(2, 3)], 33.0);
        assert_eq!(series.warnings, NO_TEXT);
    }

    const C_LEVEL_3: &str = "C/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits";
    const C_STITCH_GOLDEN: &str = "plan/goldens/tso/stitch_jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048.json";
    const A5_STITCH_GOLDEN: &str = "plan/goldens/tso/stitch_jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048.json";

    fn real(relative: &str) -> String {
        env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("tso").join(relative).to_string_lossy().into_owned()
    }

    fn c_segment(number: usize) -> String {
        real(&format!("C/jw02327001001_03101_00001-seg{number:03}_nrs1_x1dints.fits"))
    }

    fn differing_samples(left: &[f64], right: &[f64]) -> usize {
        assert_eq!(left.len(), right.len());
        left.iter().zip(right).filter(|(a, b)| !(a.is_nan() && b.is_nan()) && a.to_bits() != b.to_bits()).count()
    }

    fn assert_same_but_flux(level_3: &TsoSeries, level_2: &TsoSeries, golden: &str) {
        assert_eq!(level_3.key, level_2.key, "series key, {golden}");
        assert_eq!(level_3.int_num, level_2.int_num, "INT_NUM, {golden}");
        assert_eq!(level_3.segment, level_2.segment, "SEGMENT, {golden}");
        assert_eq!(bits(&level_3.bmjd_tdb_mid), bits(&level_2.bmjd_tdb_mid), "TDB-MID, {golden}");
        assert_eq!(bits(&level_3.mjd_utc_mid), bits(&level_2.mjd_utc_mid), "MJD-AVG, {golden}");
        assert_eq!(bits(&level_3.time()), bits(&level_2.time()), "time axis, {golden}");
        assert_eq!(bits(&level_3.wavelength_um), bits(&level_2.wavelength_um), "WAVELENGTH, {golden}");
        assert_eq!(level_3.dq, level_2.dq, "DQ, {golden}");
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/C/*_x1dints.fits"]
    fn real_six_segments_stitch_to_the_level_3_series_in_identity_time_wavelength_and_dq() {
        let golden = C_STITCH_GOLDEN;
        let level_3 = read_set(&[real(C_LEVEL_3)]).unwrap();
        let level_2 = read_set(&(1..=6).rev().map(c_segment).collect::<Vec<_>>()).unwrap();
        for set in [&level_3, &level_2] {
            assert_eq!(set.series.len(), 1, "series count, {golden}");
            let series = &set.series[0];
            assert_eq!(series.key, key("NRS1", Some(-1)), "series key, {golden}");
            assert_eq!((series.n_integrations, series.n_channels), (44, 1299), "rows x channels, {golden}");
            assert_eq!(series.time_scale, TsoTimeScale::BmjdTdb, "time scale, {golden}");
            assert_eq!(series.time_source, "TDB-MID column", "time source, {golden}");
            let time = series.time();
            assert_eq!(time[0].to_bits(), 60280.05657544178_f64.to_bits(), "first TDB-MID, {golden}");
            assert_eq!(time[43].to_bits(), 60280.50418870245_f64.to_bits(), "last TDB-MID, {golden}");
            assert!(series.common_grid, "common grid, {golden}");
        }
        let (product, stitched) = (&level_3.series[0], &level_2.series[0]);
        assert_eq!(product.int_num, (1..=44).collect::<Vec<i64>>(), "INT_NUM 1 to 44, {golden}");
        let segments: Vec<i64> = (0..44).map(|row| row / 8 + 1).collect();
        assert_eq!(product.segment, segments, "SEGMENT 1 to 6, eight rows each and four in the last, {golden}");
        assert_eq!(product.wavelength_um.len(), 57156, "samples, {golden}");
        assert_same_but_flux(product, stitched, golden);
        assert_eq!(differing_samples(&product.flux, &stitched.flux), 17643, "FLUX n_different, {golden}");

        assert_eq!(
            product.completeness,
            [record("jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048", 44, 44, &[], true)],
            "level 3 completeness, {golden}"
        );
        assert_eq!(stitched.completeness, [record("jw02327001001_03101_00001", 44, 44, &[], true)], "level 2b completeness, {golden}");
        let names: Vec<String> = level_2.files.iter().map(|file| file.file_name.clone()).collect();
        let expected: Vec<String> = (1..=6).map(|number| format!("jw02327001001_03101_00001-seg{number:03}_nrs1_x1dints.fits")).collect();
        assert_eq!(names, expected, "files in set order, {golden}");
        assert_eq!((stitched.file_index[0], stitched.file_index[43]), (0, 5), "file of the first and last row, {golden}");
        assert_eq!(product.outlier_detection.as_deref(), Some("COMPLETE"), "S_OUTLIR, {golden}");
        assert_eq!(stitched.outlier_detection, None, "S_OUTLIR of level 2b, {golden}");
        assert_eq!(stitched.warnings, NO_TEXT, "level 2b warnings, {golden}");
        assert_eq!(level_2.warnings, NO_TEXT, "set warnings, {golden}");
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/C/*_x1dints.fits"]
    fn real_level_3_and_its_segments_are_refused_together_and_a_missing_segment_is_reported() {
        let golden = C_STITCH_GOLDEN;
        let mixed = read_set(&[real(C_LEVEL_3), c_segment(1)]).unwrap_err().to_string();
        assert!(
            mixed.contains("jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits is a level 3 product and jw02327001001_03101_00001-seg001_nrs1_x1dints.fits is a level 2b product"),
            "level mix of {golden}: {mixed}"
        );

        let without_the_third = read_set(&[1, 2, 4, 5, 6].map(c_segment)).unwrap();
        assert_eq!(without_the_third.series.len(), 1, "series count, {golden}");
        let series = &without_the_third.series[0];
        assert_eq!(series.n_integrations, 36, "rows without seg003, {golden}");
        assert_eq!(
            series.completeness,
            [record("jw02327001001_03101_00001", 44, 36, &[(17, 24)], false)],
            "completeness without seg003, {golden}"
        );
        assert_eq!(
            series.warnings,
            ["exposure jw02327001001_03101_00001: 36 of 44 integrations found, missing 17 to 24"],
            "warnings without seg003, {golden}"
        );
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/A5/*_x1dints.fits"]
    fn real_two_detector_products_keep_one_series_per_detector() {
        let golden = A5_STITCH_GOLDEN;
        let level_3 = read_set(&[real("A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits")]).unwrap();
        let level_2 = read_set(&[
            real("A5/jw03375001001_03101_00001-seg001_nrs2_x1dints.fits"),
            real("A5/jw03375001001_03101_00001-seg001_nrs1_x1dints.fits"),
        ])
        .unwrap();
        let expected = [
            ("NRS1", 1271, 60496.522504382556_f64, 60496.55911503738_f64, 103, 10168),
            ("NRS2", 2048, 60496.52240268599_f64, 60496.55901266559_f64, 343, 16384),
        ];
        for set in [&level_3, &level_2] {
            assert_eq!(set.series.len(), 2, "series count, {golden}");
            let keys: Vec<SeriesKey> = set.series.iter().map(|series| series.key.clone()).collect();
            assert_eq!(keys, [key("NRS1", Some(-1)), key("NRS2", Some(-1))], "series keys, {golden}");
            for (series, (detector, channels, first, last, _, _)) in set.series.iter().zip(expected) {
                assert_eq!(series_label(&series.key, &keys), detector, "label, {golden}");
                assert_eq!((series.n_integrations, series.n_channels), (8, channels), "{detector} rows x channels, {golden}");
                assert_eq!(series.time_scale, TsoTimeScale::BmjdTdb, "{detector} time scale, {golden}");
                let time = series.time();
                assert_eq!(time[0].to_bits(), first.to_bits(), "{detector} first TDB-MID, {golden}");
                assert_eq!(time[7].to_bits(), last.to_bits(), "{detector} last TDB-MID, {golden}");
                assert_eq!(series.int_num, (1..=8).collect::<Vec<i64>>(), "{detector} INT_NUM, {golden}");
            }
        }
        for ((product, stitched), (detector, _, _, _, different, samples)) in level_3.series.iter().zip(&level_2.series).zip(expected) {
            assert_same_but_flux(product, stitched, golden);
            assert_eq!(product.flux.len(), samples, "{detector} samples, {golden}");
            assert_eq!(differing_samples(&product.flux, &stitched.flux), different, "{detector} FLUX n_different, {golden}");
            assert_eq!(
                stitched.completeness,
                [record("jw03375001001_03101_00001", 8, 8, &[], true)],
                "{detector} level 2b completeness, {golden}"
            );
            assert_eq!(
                product.completeness,
                [record("jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048", 8, 8, &[], true)],
                "{detector} level 3 completeness, {golden}"
            );
        }
        assert_eq!((level_2.series[0].file_index[0], level_2.series[1].file_index[0]), (0, 1), "files of the detectors, {golden}");
    }

    struct RealSeries {
        label: &'static str,
        rows: usize,
        channels: usize,
        valid_samples: u32,
    }

    struct RealProduct {
        relative: &'static str,
        series: Vec<RealSeries>,
        time_scale: TsoTimeScale,
        time_source: &'static str,
        first_time: f64,
        last_time: f64,
        flux_unit: &'static str,
        nints: i64,
    }

    fn real_series(label: &'static str, rows: usize, channels: usize, valid_samples: u32) -> RealSeries {
        RealSeries { label, rows, channels, valid_samples }
    }

    fn real_products() -> Vec<RealProduct> {
        let by_number = "integration number (no usable times)";
        vec![
            RealProduct {
                relative: "A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits",
                series: vec![real_series("NRCALONG", 4, 2040, 8160)],
                time_scale: TsoTimeScale::Index,
                time_source: by_number,
                first_time: 1.0,
                last_time: 4.0,
                flux_unit: "DN/s",
                nints: 4,
            },
            RealProduct {
                relative: "A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits",
                series: vec![
                    real_series("NIS, order 1", 5, 2048, 10190),
                    real_series("NIS, order 2", 5, 1783, 8890),
                    real_series("NIS, order 3", 5, 1134, 5645),
                ],
                time_scale: TsoTimeScale::Index,
                time_source: by_number,
                first_time: 1.0,
                last_time: 5.0,
                flux_unit: "MJy",
                nints: 5,
            },
            RealProduct {
                relative: "A4/jw04498-o013_t008_miri_p750l-slitlessprism_x1dints.fits",
                series: vec![real_series("MIRIMAGE", 64, 388, 24511)],
                time_scale: TsoTimeScale::BmjdTdb,
                time_source: "TDB-MID column",
                first_time: 60463.278435296284,
                last_time: 60463.2791309261,
                flux_unit: "Jy",
                nints: 64,
            },
            RealProduct {
                relative: "A6/jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_x1dints.fits",
                series: vec![real_series("NRS1", 65, 431, 26425)],
                time_scale: TsoTimeScale::BmjdTdb,
                time_source: "TDB-MID column",
                first_time: 60796.95842671788,
                last_time: 60797.05911576449,
                flux_unit: "Jy",
                nints: 65,
            },
            RealProduct {
                relative: "B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits",
                series: vec![real_series("MIRIMAGE", 10, 388, 3770)],
                time_scale: TsoTimeScale::BmjdTdb,
                time_source: "TDB-MID keyword",
                first_time: 60126.80095917577,
                last_time: 60126.80143914741,
                flux_unit: "Jy",
                nints: 10,
            },
            RealProduct {
                relative: "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits",
                series: vec![real_series("NRS1", 240, 431, 97522)],
                time_scale: TsoTimeScale::BmjdTdb,
                time_source: "TDB-MID column",
                first_time: 60061.4421733457,
                last_time: 60061.52706665472,
                flux_unit: "Jy",
                nints: 240,
            },
        ]
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/{A1,A3,A4,A6,B1,E1}/*-o0*_x1dints.fits"]
    fn real_series_have_the_time_scale_unit_and_valid_counts_of_the_files() {
        for product in real_products() {
            let file_name = product.relative.rsplit('/').next().unwrap();
            let stem = file_name.trim_end_matches("_x1dints.fits");
            let golden = format!("plan/goldens/tso/x1dints_{stem}.json");
            let set = read_set(&[real(product.relative)]).unwrap();
            let keys: Vec<SeriesKey> = set.series.iter().map(|series| series.key.clone()).collect();
            let labels: Vec<String> = keys.iter().map(|one| series_label(one, &keys)).collect();
            let expected_labels: Vec<&str> = product.series.iter().map(|series| series.label).collect();
            assert_eq!(labels, expected_labels, "series labels, {golden}");
            let level_3 = set.files[0].level == 3;
            let soss = product.flux_unit == "MJy";
            for (series, expected) in set.series.iter().zip(&product.series) {
                let label = expected.label;
                assert_eq!((series.n_integrations, series.n_channels), (expected.rows, expected.channels), "{label} rows x channels, {golden}");
                assert_eq!(series.time_scale, product.time_scale, "{label} time scale, {golden}");
                assert_eq!(series.time_source, product.time_source, "{label} time source, {golden}");
                let time = series.time();
                assert_eq!(time[0].to_bits(), product.first_time.to_bits(), "{label} first time, series[].time of {golden}");
                assert_eq!(time[expected.rows - 1].to_bits(), product.last_time.to_bits(), "{label} last time, series[].time of {golden}");
                assert_eq!(series.flux_unit, product.flux_unit, "{label} flux unit, series[].units of {golden}");
                assert_eq!(
                    series.n_valid.iter().sum::<u32>(),
                    expected.valid_samples,
                    "{label} sum of n_valid, masking_counts.valid_elements - masking_counts.flux_nan of {golden}"
                );
                assert!(series.common_grid, "{label} common grid, {golden}");
                assert_eq!(
                    series.warnings.iter().any(|warning| warning == NO_USABLE_TIMES_IN_THE_FILE),
                    product.time_scale == TsoTimeScale::Index,
                    "{label} time warning, {golden}: {:?}",
                    series.warnings
                );
                let soss_warning = "FLUX is labelled MJy but NIRISS SOSS numbers are not on that scale: treat the flux as relative";
                assert_eq!(
                    series.warnings.iter().any(|warning| warning == soss_warning),
                    soss,
                    "{label} SOSS warning, {golden}: {:?}",
                    series.warnings
                );
                assert_eq!(series.warnings.len(), usize::from(product.time_scale == TsoTimeScale::Index) + usize::from(soss), "{label} warnings, {golden}: {:?}", series.warnings);
                assert_eq!(series.notes.iter().any(|note| note == OUTLIER_NOTE), level_3, "{label} outlier note, {golden}: {:?}", series.notes);
                assert_eq!(series.outlier_detection.as_deref(), level_3.then_some("COMPLETE"), "{label} S_OUTLIR, {golden}");
                assert_eq!(
                    series.completeness,
                    [record(stem, product.nints, expected.rows, &[], true)],
                    "{label} completeness against NINTS, {golden}"
                );
            }
        }
    }
}
