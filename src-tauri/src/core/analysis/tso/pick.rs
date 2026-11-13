use std::collections::BTreeMap;
use std::ops::Range;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use super::series::series_label;
use super::{SeriesKey, TsoFile, TsoSeries, TsoSet, TsoTimeScale, DQ_DO_NOT_USE, UNKNOWN_UNIT};
use crate::core::analysis::spectrum1d::{
    tabulated_axis, Precision, Spectrum1d, Spectrum1dSource, WavelengthFrame, WavelengthUnitSource,
};
use crate::core::analysis::x1d::{COL_FLUX_ERROR, COL_WAVELENGTH, EXTNAME_EXTRACT1D};
use crate::core::astrometry::spectral::AxisKind;
use crate::core::imaging::dq_flags::DqTable;
use crate::math::exact_median_f64;

pub const MEDIAN_ERROR_FACTOR: f64 = 1.2533141373155001;

const MEAN_ERROR_FACTOR: f64 = 1.0;
const TIME_NAME_BMJD_TDB: &str = "BMJD (TDB)";
const TIME_NAME_MJD_UTC: &str = "MJD (UTC)";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TsoPick {
    Integration { index: usize },
    Median,
    Mean { first: usize, last: usize },
}

struct Picked {
    wavelength_um: Vec<f64>,
    flux: Vec<f64>,
    err: Vec<f64>,
    dq: Vec<u32>,
    first_row: usize,
    notes: Vec<String>,
}

fn set_keys(set: &TsoSet) -> Vec<SeriesKey> {
    set.series.iter().map(|series| series.key.clone()).collect()
}

pub fn find_series<'a>(set: &'a TsoSet, key: &SeriesKey) -> Result<&'a TsoSeries> {
    if let Some(series) = set.series.iter().find(|series| series.key == *key) {
        return Ok(series);
    }
    let keys = set_keys(set);
    let labels: Vec<String> = keys.iter().map(|found| series_label(found, &keys)).collect();
    let order = key.sporder.map_or_else(String::new, |order| format!(", order {order}"));
    bail!("no series {}{order} in these files; series found: {}", key.detector, labels.join(", "))
}

fn usable_error(error: f64) -> bool {
    error.is_finite() && error > 0.0
}

fn mean_of(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn refuse_beyond_the_series(index: usize, label: &str, n_integrations: usize) -> Result<()> {
    if index >= n_integrations {
        bail!(
            "integration index {index} is out of range: the series {label} has {n_integrations} integrations (0 to {})",
            n_integrations - 1
        );
    }
    Ok(())
}

fn refuse_differing_grids(series: &TsoSeries, label: &str) -> Result<()> {
    if !series.common_grid {
        bail!("the median and mean spectra need one wavelength grid for every integration and the series {label} has rows whose wavelengths differ: pick one integration");
    }
    Ok(())
}

fn one_integration(series: &TsoSeries, label: &str, index: usize) -> Picked {
    let samples = series.sample(index, 0)..series.sample(index + 1, 0);
    let moment = match series.time_scale {
        TsoTimeScale::BmjdTdb => format!(", {TIME_NAME_BMJD_TDB} {}", series.bmjd_tdb_mid[index]),
        TsoTimeScale::MjdUtc => format!(", {TIME_NAME_MJD_UTC} {}", series.mjd_utc_mid[index]),
        TsoTimeScale::Index => String::new(),
    };
    let err = series.flux_error[samples.clone()].iter().map(|&error| if usable_error(error) { error } else { f64::NAN });
    Picked {
        wavelength_um: series.wavelength_um[samples.clone()].to_vec(),
        flux: series.flux[samples.clone()].to_vec(),
        err: err.collect(),
        dq: series.dq[samples].to_vec(),
        first_row: index,
        notes: vec![format!(
            "time series {label}: integration {} of {} (INT_NUM {}{moment})",
            index + 1,
            series.n_integrations,
            series.int_num[index]
        )],
    }
}

fn reduced(series: &TsoSeries, rows: Range<usize>, centre: fn(&[f64]) -> f64, error_factor: f64, described: String) -> Picked {
    let n_channels = series.n_channels;
    let mut flux = Vec::with_capacity(n_channels);
    let mut err = Vec::with_capacity(n_channels);
    let mut dq = Vec::with_capacity(n_channels);
    let mut valid = Vec::with_capacity(rows.len());
    for channel in 0..n_channels {
        valid.clear();
        let mut squared_errors = 0.0;
        let mut has_error = false;
        for row in rows.clone().filter(|&row| series.is_valid(row, channel)) {
            let sample = series.sample(row, channel);
            valid.push(series.flux[sample]);
            let error = series.flux_error[sample];
            if usable_error(error) {
                squared_errors += error * error;
                has_error = true;
            }
        }
        if valid.is_empty() {
            flux.push(f64::NAN);
            err.push(f64::NAN);
            dq.push(DQ_DO_NOT_USE);
            continue;
        }
        flux.push(centre(&valid));
        err.push(if has_error { error_factor * squared_errors.sqrt() / valid.len() as f64 } else { f64::NAN });
        dq.push(0);
    }
    let mut notes = vec![
        described,
        format!("per channel over the valid samples; error of the mean = sqrt(sum of {COL_FLUX_ERROR}^2) / n, error of the median = {MEDIAN_ERROR_FACTOR:.4} x that (sqrt(pi / 2))"),
    ];
    let empty = dq.iter().filter(|&&flags| flags == DQ_DO_NOT_USE).count();
    if empty > 0 {
        notes.push(format!("{empty} of {n_channels} channels have no valid sample in these integrations and are NaN"));
    }
    Picked { wavelength_um: series.grid_um.clone(), flux, err, dq, first_row: rows.start, notes }
}

fn identity_cards(file: &TsoFile, key: &SeriesKey) -> BTreeMap<String, String> {
    [
        ("TARGNAME", file.target.clone()),
        ("INSTRUME", file.instrument.clone()),
        ("EXP_TYPE", file.exp_type.clone()),
        ("FILTER", file.filter.clone()),
        ("GRATING", file.grating.clone()),
        ("PUPIL", file.pupil.clone()),
        ("DETECTOR", Some(key.detector.clone())),
    ]
    .into_iter()
    .filter_map(|(card, value)| value.map(|value| (card.to_string(), value)))
    .collect()
}

pub fn series_spectrum(set: &TsoSet, key: &SeriesKey, pick: &TsoPick) -> Result<Spectrum1d> {
    let series = find_series(set, key)?;
    let label = series_label(key, &set_keys(set));
    let n_integrations = series.n_integrations;
    if n_integrations == 0 {
        bail!("the series {label} has no integrations");
    }
    let picked = match *pick {
        TsoPick::Integration { index } => {
            refuse_beyond_the_series(index, &label, n_integrations)?;
            one_integration(series, &label, index)
        }
        TsoPick::Median => {
            refuse_differing_grids(series, &label)?;
            let described = format!("time series {label}: median of {n_integrations} integrations");
            reduced(series, 0..n_integrations, exact_median_f64, MEDIAN_ERROR_FACTOR, described)
        }
        TsoPick::Mean { first, last } => {
            refuse_differing_grids(series, &label)?;
            if first > last {
                bail!("mean of integrations {first} to {last}: the first index must not be above the last");
            }
            refuse_beyond_the_series(last, &label, n_integrations)?;
            let described =
                format!("time series {label}: mean of integrations {} to {} of {n_integrations}", first + 1, last + 1);
            reduced(series, first..last + 1, mean_of, MEAN_ERROR_FACTOR, described)
        }
    };
    let mut notes = vec![format!("{EXTNAME_EXTRACT1D} table of a space-mission pipeline: vacuum wavelengths")];
    notes.extend(picked.notes);
    let has_error = picked.err.iter().any(|error| !error.is_nan());
    Ok(Spectrum1d {
        source: Spectrum1dSource::X1dInts {
            paths: set.files.iter().map(|file| file.path.clone()).collect(),
            detector: key.detector.clone(),
            sporder: key.sporder,
            pick: pick.clone(),
        },
        axis: tabulated_axis(picked.wavelength_um, AxisKind::Wave, COL_WAVELENGTH),
        wavelength_frame: WavelengthFrame::Vacuum,
        precision: Precision::F64,
        flux: picked.flux,
        err: has_error.then_some(picked.err),
        dq: Some(picked.dq),
        dq_table: DqTable::Jwst,
        flux_unit: (series.flux_unit != UNKNOWN_UNIT).then(|| series.flux_unit.clone()),
        n_rows: series.n_channels,
        cards: identity_cards(&set.files[series.file_index[picked.first_row] as usize], key),
        notes,
        wavelength_unit_source: WavelengthUnitSource::File,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::core::analysis::tso::series::build_set;
    use crate::core::analysis::tso::series::test_support::{file_of, ramp_set, table_of};
    use crate::core::analysis::tso::test_support::synth_file;
    use crate::infra::fits::x1dints::test_fixtures::ramp_row;

    const PIPELINE_NOTE: &str = "EXTRACT1D table of a space-mission pipeline: vacuum wavelengths";
    const METHOD_NOTE: &str = "per channel over the valid samples; error of the mean = sqrt(sum of FLUX_ERROR^2) / n, error of the median = 1.2533 x that (sqrt(pi / 2))";
    const DIFFERING_GRIDS: &str = "the median and mean spectra need one wavelength grid for every integration and the series NRS1 has rows whose wavelengths differ: pick one integration";

    fn key() -> SeriesKey {
        SeriesKey { detector: "NRS1".into(), sporder: Some(0) }
    }

    fn ramp() -> TsoSet {
        ramp_set("NRS1", Some(0), 4, 3)
    }

    fn ramp_with(change: impl FnOnce(&mut TsoSeries)) -> TsoSet {
        let mut set = ramp();
        change(&mut set.series[0]);
        set
    }

    fn spectrum(set: &TsoSet, pick: TsoPick) -> Spectrum1d {
        series_spectrum(set, &key(), &pick).unwrap()
    }

    fn refusal(set: &TsoSet, key: &SeriesKey, pick: TsoPick) -> String {
        series_spectrum(set, key, &pick).unwrap_err().to_string()
    }

    fn errors(spectrum: &Spectrum1d) -> &[f64] {
        spectrum.err.as_deref().expect("the pick has errors")
    }

    fn flags(spectrum: &Spectrum1d) -> &[u32] {
        spectrum.dq.as_deref().expect("a series pick always has DQ")
    }

    fn source_of(paths: &[&str], pick: TsoPick) -> Spectrum1dSource {
        Spectrum1dSource::X1dInts {
            paths: paths.iter().map(|path| path.to_string()).collect(),
            detector: "NRS1".into(),
            sporder: Some(0),
            pick,
        }
    }

    #[test]
    fn one_integration_is_returned_as_stored_with_its_position_in_the_series() {
        let set = ramp();
        let third = spectrum(&set, TsoPick::Integration { index: 2 });
        assert_eq!(third.flux, [30.0, 31.0, 32.0]);
        assert_eq!(errors(&third), [0.5, 0.5, 0.5]);
        assert_eq!(flags(&third), [0, 0, 0]);
        assert_eq!(third.axis.values, set.series[0].grid_um);
        assert!(third.axis.tabulated);
        assert_eq!(third.axis.unit, "um");
        assert_eq!(third.n_rows, 3);
        assert_eq!(third.flux_unit.as_deref(), Some("Jy"));
        assert_eq!(third.wavelength_frame, WavelengthFrame::Vacuum);
        assert_eq!(third.precision, Precision::F64);
        assert_eq!(third.dq_table, DqTable::Jwst);
        assert_eq!(third.wavelength_unit_source, WavelengthUnitSource::File);
        assert_eq!(third.source, source_of(&["ramp_x1dints.fits"], TsoPick::Integration { index: 2 }));
        assert_eq!(third.notes.len(), 2, "{:?}", third.notes);
        assert_eq!(third.notes[0], PIPELINE_NOTE);
        assert!(third.notes[1].starts_with("time series NRS1: integration 3 of 4 (INT_NUM 3, BMJD (TDB) "), "{}", third.notes[1]);
        assert!(third.notes[1].ends_with(')'), "{}", third.notes[1]);
        let tdb_mid = 60000.0 + 0.01 * 2.0 + 0.002;
        assert_eq!(third.notes[1], format!("time series NRS1: integration 3 of 4 (INT_NUM 3, BMJD (TDB) {tdb_mid})"));
        assert_eq!(third.cards["DETECTOR"], "NRS1");
        assert_eq!(third.cards["EXP_TYPE"], "NRS_BRIGHTOBJ");
        assert_eq!(third.cards["FILTER"], "CLEAR");
        assert!(!third.cards.contains_key("TARGNAME"));
        assert_eq!(third.cards.len(), 3, "{:?}", third.cards);

        let flagged = ramp_with(|series| {
            let sample = series.sample(2, 1);
            series.flux[sample] = f64::NAN;
            series.flux_error[sample] = 0.0;
            series.dq[sample] = 1;
            let padding = series.sample(2, 2);
            series.wavelength_um[padding] = f64::NAN;
        });
        let third = spectrum(&flagged, TsoPick::Integration { index: 2 });
        assert_eq!(third.flux[0], 30.0);
        assert!(third.flux[1].is_nan());
        assert_eq!(errors(&third)[0], 0.5);
        assert!(errors(&third)[1].is_nan());
        assert_eq!(flags(&third), [0, 1, 0]);
        assert_eq!(third.axis.values[..2], [1.0, 1.1]);
        assert!(third.axis.values[2].is_nan(), "an integration keeps its own wavelengths, padding included");

        let by_number = ramp_with(|series| series.time_scale = TsoTimeScale::Index);
        assert_eq!(
            spectrum(&by_number, TsoPick::Integration { index: 2 }).notes[1],
            "time series NRS1: integration 3 of 4 (INT_NUM 3)"
        );
        let by_utc = ramp_with(|series| series.time_scale = TsoTimeScale::MjdUtc);
        let mjd_avg = 60000.0 + 0.01 * 2.0;
        assert_eq!(
            spectrum(&by_utc, TsoPick::Integration { index: 2 }).notes[1],
            format!("time series NRS1: integration 3 of 4 (INT_NUM 3, MJD (UTC) {mjd_avg})")
        );

        let unlabelled = ramp_with(|series| series.flux_unit = UNKNOWN_UNIT.to_string());
        assert_eq!(spectrum(&unlabelled, TsoPick::Integration { index: 0 }).flux_unit, None);
    }

    #[test]
    fn the_cards_come_from_the_file_of_the_first_row_used_and_the_source_lists_every_file() {
        let mut two_files = ramp();
        let mut later = synth_file("later_x1dints.fits", "NRS_BRIGHTOBJ", "F290LP", 2);
        later.target = Some("WASP-39".into());
        later.instrument = Some("NIRSPEC".into());
        later.grating = Some("G395H".into());
        later.pupil = Some("CLEARP".into());
        two_files.files.push(later);
        two_files.series[0].file_index[2] = 1;
        two_files.series[0].file_index[3] = 1;

        let from_the_second = spectrum(&two_files, TsoPick::Integration { index: 2 });
        let cards: Vec<(&str, &str)> = from_the_second.cards.iter().map(|(card, value)| (card.as_str(), value.as_str())).collect();
        assert_eq!(
            cards,
            [
                ("DETECTOR", "NRS1"),
                ("EXP_TYPE", "NRS_BRIGHTOBJ"),
                ("FILTER", "F290LP"),
                ("GRATING", "G395H"),
                ("INSTRUME", "NIRSPEC"),
                ("PUPIL", "CLEARP"),
                ("TARGNAME", "WASP-39"),
            ]
        );
        let both = ["ramp_x1dints.fits", "later_x1dints.fits"];
        assert_eq!(from_the_second.source, source_of(&both, TsoPick::Integration { index: 2 }));
        assert_eq!(spectrum(&two_files, TsoPick::Median).cards["FILTER"], "CLEAR");
        assert_eq!(spectrum(&two_files, TsoPick::Mean { first: 1, last: 3 }).cards["FILTER"], "CLEAR");
        let late_mean = spectrum(&two_files, TsoPick::Mean { first: 2, last: 3 });
        assert_eq!(late_mean.cards["FILTER"], "F290LP");
        assert_eq!(late_mean.source, source_of(&both, TsoPick::Mean { first: 2, last: 3 }));
    }

    #[test]
    fn median_and_mean_reduce_the_valid_samples_of_each_channel_and_propagate_the_error() {
        let set = ramp();
        let median = spectrum(&set, TsoPick::Median);
        assert_eq!(median.flux, [25.0, 26.0, 27.0]);
        let median_error = MEDIAN_ERROR_FACTOR * (4.0f64 * 0.25).sqrt() / 4.0;
        assert_eq!(median_error, 0.31332853432887503);
        assert_eq!(errors(&median), [median_error; 3]);
        assert_eq!(flags(&median), [0, 0, 0]);
        assert_eq!(median.axis.values, set.series[0].grid_um);
        assert_eq!(median.n_rows, 3);
        assert_eq!(median.notes, [PIPELINE_NOTE, "time series NRS1: median of 4 integrations", METHOD_NOTE]);
        assert_eq!(median.source, source_of(&["ramp_x1dints.fits"], TsoPick::Median));

        let mean = spectrum(&set, TsoPick::Mean { first: 1, last: 2 });
        assert_eq!(mean.flux, [25.0, 26.0, 27.0]);
        let mean_error = 0.5f64.sqrt() / 2.0;
        assert_eq!(mean_error, 0.3535533905932738);
        assert_eq!(errors(&mean), [mean_error; 3]);
        assert_eq!(flags(&mean), [0, 0, 0]);
        assert_eq!(mean.notes, [PIPELINE_NOTE, "time series NRS1: mean of integrations 2 to 3 of 4", METHOD_NOTE]);

        let last = spectrum(&set, TsoPick::Mean { first: 3, last: 3 });
        assert_eq!(last.flux, spectrum(&set, TsoPick::Integration { index: 3 }).flux);
        assert_eq!(last.flux, [40.0, 41.0, 42.0]);
        assert_eq!(errors(&last), [0.5; 3]);

        let whole = spectrum(&set, TsoPick::Mean { first: 0, last: 3 });
        assert_eq!(whole.flux, [25.0, 26.0, 27.0]);
        assert_eq!(errors(&whole), [0.25; 3]);
        assert_eq!(whole.notes[1], "time series NRS1: mean of integrations 1 to 4 of 4");
    }

    #[test]
    fn invalid_samples_are_left_out_per_channel_and_empty_channels_are_nan_with_dq_1() {
        let with_a_gap = ramp_with(|series| {
            let sample = series.sample(1, 0);
            series.flux[sample] = f64::NAN;
        });
        let mean = spectrum(&with_a_gap, TsoPick::Mean { first: 1, last: 2 });
        assert_eq!(mean.flux, [30.0, 26.0, 27.0]);
        assert_eq!(errors(&mean)[0], 0.5);
        assert_eq!(errors(&mean)[1], 0.5f64.sqrt() / 2.0);
        let median = spectrum(&with_a_gap, TsoPick::Median);
        assert_eq!(median.flux, [30.0, 26.0, 27.0]);
        assert_eq!(errors(&median)[0], MEDIAN_ERROR_FACTOR * 0.75f64.sqrt() / 3.0);
        assert_eq!(flags(&median), [0, 0, 0]);
        assert_eq!(median.notes.len(), 3, "{:?}", median.notes);

        let with_an_unusable_error = ramp_with(|series| {
            let zero = series.sample(0, 1);
            series.flux_error[zero] = 0.0;
            let infinite = series.sample(1, 2);
            series.flux_error[infinite] = f64::INFINITY;
        });
        let mean = spectrum(&with_an_unusable_error, TsoPick::Mean { first: 0, last: 1 });
        assert_eq!(mean.flux, [15.0, 16.0, 17.0]);
        assert_eq!(errors(&mean), [0.5f64.sqrt() / 2.0, 0.25, 0.25]);

        let flagged_channel = ramp_with(|series| {
            for row in 0..4 {
                let sample = series.sample(row, 2);
                series.dq[sample] = 1;
            }
        });
        let median = spectrum(&flagged_channel, TsoPick::Median);
        assert_eq!(median.flux[..2], [25.0, 26.0]);
        assert!(median.flux[2].is_nan());
        assert!(errors(&median)[2].is_nan());
        assert_eq!(flags(&median), [0, 0, 1]);
        assert_eq!(
            median.notes,
            [
                PIPELINE_NOTE,
                "time series NRS1: median of 4 integrations",
                METHOD_NOTE,
                "1 of 3 channels have no valid sample in these integrations and are NaN",
            ]
        );
        let mean = spectrum(&flagged_channel, TsoPick::Mean { first: 0, last: 1 });
        assert!(mean.flux[2].is_nan());
        assert_eq!(flags(&mean), [0, 0, 1]);
        assert_eq!(mean.notes[3], "1 of 3 channels have no valid sample in these integrations and are NaN");
        assert_eq!(spectrum(&flagged_channel, TsoPick::Integration { index: 0 }).notes.len(), 2);

        let without_errors = ramp_with(|series| series.flux_error.fill(f64::NAN));
        assert_eq!(spectrum(&without_errors, TsoPick::Median).err, None);
        assert_eq!(spectrum(&without_errors, TsoPick::Mean { first: 0, last: 3 }).err, None);
        assert_eq!(spectrum(&without_errors, TsoPick::Integration { index: 1 }).err, None);
        assert_eq!(flags(&spectrum(&without_errors, TsoPick::Median)), [0, 0, 0]);
    }

    #[test]
    fn picks_outside_the_series_and_series_that_are_not_there_are_refused_by_name() {
        let set = ramp();
        assert_eq!(
            refusal(&set, &key(), TsoPick::Integration { index: 4 }),
            "integration index 4 is out of range: the series NRS1 has 4 integrations (0 to 3)"
        );
        assert_eq!(
            refusal(&set, &key(), TsoPick::Mean { first: 2, last: 1 }),
            "mean of integrations 2 to 1: the first index must not be above the last"
        );
        assert_eq!(
            refusal(&set, &key(), TsoPick::Mean { first: 0, last: 9 }),
            "integration index 9 is out of range: the series NRS1 has 4 integrations (0 to 3)"
        );
        let another_detector = SeriesKey { detector: "NRS9".into(), sporder: Some(-1) };
        assert_eq!(
            refusal(&set, &another_detector, TsoPick::Median),
            "no series NRS9, order -1 in these files; series found: NRS1"
        );
        let without_order = SeriesKey { detector: "NRS1".into(), sporder: None };
        assert_eq!(refusal(&set, &without_order, TsoPick::Median), "no series NRS1 in these files; series found: NRS1");
        assert_eq!(
            find_series(&set, &without_order).unwrap_err().to_string(),
            "no series NRS1 in these files; series found: NRS1"
        );
        assert_eq!(find_series(&set, &key()).unwrap().n_integrations, 4);

        let differing = ramp_with(|series| series.common_grid = false);
        assert_eq!(refusal(&differing, &key(), TsoPick::Median), DIFFERING_GRIDS);
        assert_eq!(refusal(&differing, &key(), TsoPick::Mean { first: 2, last: 1 }), DIFFERING_GRIDS);
        assert_eq!(spectrum(&differing, TsoPick::Integration { index: 0 }).flux, [10.0, 11.0, 12.0]);

        let without_rows = ramp_with(|series| {
            series.n_integrations = 0;
            series.int_num.clear();
            series.segment.clear();
            series.file_index.clear();
            series.n_valid.clear();
            series.mjd_utc_mid.clear();
            series.bmjd_tdb_mid.clear();
            series.wavelength_um.clear();
            series.flux.clear();
            series.flux_error.clear();
            series.dq.clear();
            series.npixels.clear();
        });
        for pick in [TsoPick::Integration { index: 0 }, TsoPick::Median, TsoPick::Mean { first: 0, last: 0 }] {
            assert_eq!(refusal(&without_rows, &key(), pick), "the series NRS1 has no integrations");
        }
    }

    #[test]
    fn the_orders_of_one_detector_are_told_apart_in_the_notes_and_in_the_refusal() {
        let rows = [ramp_row(1, 60000.0, 3), ramp_row(2, 60000.5, 3)];
        let orders = vec![table_of("NIS", Some(1), 1, &rows), table_of("NIS", Some(2), 2, &rows)];
        let set = build_set(vec![file_of("soss_x1dints.fits", 3, Some(2), Some(60000.0), orders)]).unwrap();
        let second = SeriesKey { detector: "NIS".into(), sporder: Some(2) };
        let median = series_spectrum(&set, &second, &TsoPick::Median).unwrap();
        assert_eq!(median.notes[1], "time series NIS, order 2: median of 2 integrations");
        assert_eq!(median.flux, [15.0, 16.0, 17.0]);
        assert_eq!(
            median.source,
            Spectrum1dSource::X1dInts {
                paths: vec!["soss_x1dints.fits".into()],
                detector: "NIS".into(),
                sporder: Some(2),
                pick: TsoPick::Median,
            }
        );
        let fourth = SeriesKey { detector: "NIS".into(), sporder: Some(4) };
        assert_eq!(
            series_spectrum(&set, &fourth, &TsoPick::Median).unwrap_err().to_string(),
            "no series NIS, order 4 in these files; series found: NIS, order 1, NIS, order 2"
        );
        assert_eq!(
            series_spectrum(&set, &second, &TsoPick::Integration { index: 2 }).unwrap_err().to_string(),
            "integration index 2 is out of range: the series NIS, order 2 has 2 integrations (0 to 1)"
        );
    }

    #[test]
    fn a_pick_serialises_with_the_kind_tag_and_reads_back() {
        let cases = [
            (TsoPick::Integration { index: 3 }, json!({"kind": "integration", "index": 3})),
            (TsoPick::Median, json!({"kind": "median"})),
            (TsoPick::Mean { first: 0, last: 7 }, json!({"kind": "mean", "first": 0, "last": 7})),
        ];
        for (pick, wire) in cases {
            assert_eq!(serde_json::to_value(&pick).unwrap(), wire);
            assert_eq!(serde_json::from_value::<TsoPick>(wire).unwrap(), pick);
        }
        assert!(serde_json::from_value::<TsoPick>(json!({"kind": "mode"})).is_err());
        assert!(serde_json::from_value::<TsoPick>(json!({"kind": "integration"})).is_err());
        assert!(serde_json::from_value::<TsoPick>(json!({"kind": "mean", "first": -1, "last": 2})).is_err());
    }
}
