use std::collections::HashSet;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use super::bins::{check_bins, sum_channels, sum_rows, Interval, IntervalSum};
use super::{SeriesKey, TsoFile, TsoSeries, TsoSet, TsoTimeScale};
use crate::math::exact_median_f64;
use crate::types::constants::MAD_TO_SIGMA;

pub const OPEN_LIMITS_UM: [f64; 2] = [-1.0, 1.0e10];
pub const SOSS_ORDER1_UM: [f64; 2] = [0.93, 2.82];
pub const SOSS_ORDER2_UM: [f64; 2] = [0.64, 0.83];
pub const SOSS_ORDER3_UM: [f64; 2] = [0.64, 0.95];
pub const MIRI_LRS_UM: [f64; 2] = [5.0, 12.0];
pub const NIRCAM_F322W2_UM: [f64; 2] = [2.47, 3.94];
pub const REJECT_NEIGHBOURS: usize = 10;
pub const REJECT_MIN_NEIGHBOURS: usize = 4;
pub const PIXEL_LOSS_RATIO: f64 = 0.9;

const EXP_TYPE_SOSS: &str = "NIS_SOSS";
const EXP_TYPE_MIRI_LRS: &str = "MIR_LRS-SLITLESS";
const EXP_TYPE_NIRCAM_GRISM: &str = "NRC_TSGRISM";
const EXP_TYPE_NIRSPEC_PREFIX: &str = "NRS_";
const FILTER_CLEAR: &str = "CLEAR";
const FILTER_F322W2: &str = "F322W2";
const FLUX_ERR_NOTE: &str =
    "flux_err is the square root of the summed FLUX_ERROR squared: the app's number, not a pipeline product";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CurveMode {
    Pipeline,
    Masked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LimitsOrigin {
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "user")]
    User,
    #[serde(rename = "file log")]
    FileLog,
    #[serde(rename = "built-in default")]
    BuiltInDefault,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CurveRequest {
    pub mode: CurveMode,
    #[serde(default)]
    pub wavelength_min_um: Option<f64>,
    #[serde(default)]
    pub wavelength_max_um: Option<f64>,
    #[serde(default)]
    pub bins: Vec<[f64; 2]>,
    #[serde(default)]
    pub exclude_um: Vec<[f64; 2]>,
    #[serde(default)]
    pub reject_elements_sigma: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Curve {
    pub band_um: [f64; 2],
    pub limits_origin: LimitsOrigin,
    pub flux: Vec<f64>,
    pub flux_err: Vec<f64>,
    pub n_used: Vec<u32>,
    pub npixels_mean: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Dropped {
    pub nan_time: usize,
    pub repeated_time: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RejectedElement {
    pub index: usize,
    pub wavelength_um: f64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LightCurves {
    pub series: SeriesKey,
    pub mode: CurveMode,
    pub time_scale: TsoTimeScale,
    pub time: Vec<f64>,
    pub mjd_utc: Vec<f64>,
    pub int_num: Vec<i64>,
    pub row_index: Vec<usize>,
    pub flux_unit: String,
    pub white: Curve,
    pub bins: Vec<Curve>,
    pub dropped: Dropped,
    pub rejected_elements: Vec<RejectedElement>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitChoice {
    pub band_um: [f64; 2],
    pub origin: LimitsOrigin,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

struct MaskedChannels {
    channels: Vec<usize>,
    rejected: Vec<RejectedElement>,
    notes: Vec<String>,
}

fn limits(band_um: [f64; 2], origin: LimitsOrigin) -> LimitChoice {
    LimitChoice { band_um, origin, notes: Vec::new(), warnings: Vec::new() }
}

fn logged_side(side: Option<f64>) -> String {
    side.map_or_else(|| "open".to_string(), |value| value.to_string())
}

fn open_without_known_limits(exp_type: Option<&str>, filter: Option<&str>) -> LimitChoice {
    let observing_mode = match (exp_type, filter) {
        (Some(exp_type), Some(filter)) => format!("{exp_type} {filter}"),
        (Some(exp_type), None) => exp_type.to_string(),
        (None, _) => "unknown mode".to_string(),
    };
    let note = format!(
        "no white-light limits are known for {observing_mode}: every wavelength is summed; set limits to match the pipeline"
    );
    LimitChoice { notes: vec![note], ..limits(OPEN_LIMITS_UM, LimitsOrigin::Open) }
}

fn soss_clear_limits(sporder: Option<i64>, mode: CurveMode) -> LimitChoice {
    match (mode, sporder) {
        (CurveMode::Pipeline, Some(order)) if order != 1 => {
            let [lo, hi] = SOSS_ORDER1_UM;
            let warning = format!(
                "the pipeline sums order {order} over the order 1 range {lo} to {hi} um, as the archived white light does: this is not a usable light curve of order {order}; the masked mode uses the range of the order"
            );
            LimitChoice { warnings: vec![warning], ..limits(SOSS_ORDER1_UM, LimitsOrigin::BuiltInDefault) }
        }
        (CurveMode::Pipeline, _) | (CurveMode::Masked, Some(1)) => limits(SOSS_ORDER1_UM, LimitsOrigin::BuiltInDefault),
        (CurveMode::Masked, Some(2)) => limits(SOSS_ORDER2_UM, LimitsOrigin::BuiltInDefault),
        (CurveMode::Masked, Some(3)) => limits(SOSS_ORDER3_UM, LimitsOrigin::BuiltInDefault),
        (CurveMode::Masked, _) => open_without_known_limits(Some(EXP_TYPE_SOSS), Some(FILTER_CLEAR)),
    }
}

pub fn default_limits(file: &TsoFile, key: &SeriesKey, mode: CurveMode) -> LimitChoice {
    if let Some(log) = file.whitelight_log.as_ref().filter(|log| log.min_um.is_some() || log.max_um.is_some()) {
        let band_um = [log.min_um.unwrap_or(OPEN_LIMITS_UM[0]), log.max_um.unwrap_or(OPEN_LIMITS_UM[1])];
        let note = format!(
            "white-light limits {} to {} um are the ones the pipeline logged in {}",
            logged_side(log.min_um),
            logged_side(log.max_um),
            file.file_name
        );
        return LimitChoice { notes: vec![note], ..limits(band_um, LimitsOrigin::FileLog) };
    }
    let exp_type = file.exp_type.as_deref();
    let filter = file.filter.as_deref();
    match (exp_type, filter) {
        (Some(EXP_TYPE_SOSS), Some(FILTER_CLEAR)) => soss_clear_limits(key.sporder, mode),
        (Some(EXP_TYPE_MIRI_LRS), _) => limits(MIRI_LRS_UM, LimitsOrigin::BuiltInDefault),
        (Some(EXP_TYPE_NIRCAM_GRISM), Some(FILTER_F322W2)) => limits(NIRCAM_F322W2_UM, LimitsOrigin::BuiltInDefault),
        (Some(nirspec), _) if nirspec.starts_with(EXP_TYPE_NIRSPEC_PREFIX) => limits(OPEN_LIMITS_UM, LimitsOrigin::Open),
        _ => open_without_known_limits(exp_type, filter),
    }
}

fn check_request(series: &TsoSeries, request: &CurveRequest) -> Result<()> {
    let (min, max) = (request.wavelength_min_um, request.wavelength_max_um);
    if min.into_iter().chain(max).any(|limit| !limit.is_finite()) {
        bail!("wavelength limits must be finite numbers");
    }
    if let (Some(min), Some(max)) = (min, max) {
        if min >= max {
            bail!("wavelength limits {min} to {max} um are empty: the lower limit must be below the upper one");
        }
    }
    check_bins(&request.bins)?;
    for (index, [from, to]) in request.exclude_um.iter().enumerate() {
        if !(from.is_finite() && to.is_finite() && from <= to) {
            bail!("exclude range {}: {from} to {to} um is not a range", index + 1);
        }
    }
    if let Some(sigma) = request.reject_elements_sigma {
        if !(sigma.is_finite() && sigma > 0.0) {
            bail!("reject_elements_sigma must be above 0, got {sigma}");
        }
    }
    match request.mode {
        CurveMode::Pipeline if !request.exclude_um.is_empty() || request.reject_elements_sigma.is_some() => bail!(
            "exclude_um and reject_elements_sigma apply to the masked mode only: the pipeline mode reproduces the pipeline sum"
        ),
        CurveMode::Masked if !series.common_grid => bail!(
            "the masked mode needs one wavelength grid for every integration and this series has rows whose wavelengths differ: use the pipeline mode"
        ),
        _ => Ok(()),
    }
}

fn wavelength_range(series: &TsoSeries) -> Option<[f64; 2]> {
    let mut finite = series.wavelength_um.iter().copied().filter(|wavelength| wavelength.is_finite());
    let first = finite.next()?;
    Some(finite.fold([first, first], |[shortest, longest], wavelength| [shortest.min(wavelength), longest.max(wavelength)]))
}

fn requested_limits(request: &CurveRequest) -> Option<LimitChoice> {
    let (min, max) = (request.wavelength_min_um, request.wavelength_max_um);
    (min.is_some() || max.is_some())
        .then(|| limits([min.unwrap_or(OPEN_LIMITS_UM[0]), max.unwrap_or(OPEN_LIMITS_UM[1])], LimitsOrigin::User))
}

fn rows_the_pipeline_keeps(series: &TsoSeries) -> (Vec<usize>, Dropped) {
    let mut kept = Vec::new();
    let mut seen = HashSet::new();
    let mut dropped = Dropped { nan_time: 0, repeated_time: 0 };
    for (row, time) in series.mjd_utc_mid.iter().enumerate() {
        if !time.is_finite() {
            dropped.nan_time += 1;
        } else if seen.insert(time.to_bits()) {
            kept.push(row);
        } else {
            dropped.repeated_time += 1;
        }
    }
    (kept, dropped)
}

fn isolated_elements(series: &TsoSeries, channels: &[usize], sigma: f64) -> Vec<RejectedElement> {
    let time_median: Vec<f64> = channels
        .iter()
        .map(|&channel| {
            let over_time: Vec<f64> = (0..series.n_integrations).map(|row| series.flux[series.sample(row, channel)]).collect();
            exact_median_f64(&over_time)
        })
        .collect();
    let mut rejected = Vec::new();
    for (position, (&channel, &median)) in channels.iter().zip(&time_median).enumerate() {
        let before = &time_median[position.saturating_sub(REJECT_NEIGHBOURS)..position];
        let after = &time_median[position + 1..(position + 1 + REJECT_NEIGHBOURS).min(time_median.len())];
        let neighbours = [before, after].concat();
        if neighbours.len() < REJECT_MIN_NEIGHBOURS {
            continue;
        }
        let centre = exact_median_f64(&neighbours);
        let deviations: Vec<f64> = neighbours.iter().map(|value| (value - centre).abs()).collect();
        let scale = MAD_TO_SIGMA * exact_median_f64(&deviations);
        let robust_sigma = if scale > 0.0 {
            (median - centre).abs() / scale
        } else if median != centre {
            f64::INFINITY
        } else {
            0.0
        };
        if robust_sigma > sigma {
            rejected.push(RejectedElement {
                index: channel,
                wavelength_um: series.grid_um[channel],
                reason: format!(
                    "time-median flux {median:.6e} is {robust_sigma:.3e} robust sigma from the median {centre:.6e} of its {} neighbours",
                    neighbours.len()
                ),
            });
        }
    }
    rejected
}

fn channels_of_the_masked_mode(series: &TsoSeries, request: &CurveRequest) -> MaskedChannels {
    let valid_in_every_row: Vec<usize> = (0..series.n_channels)
        .filter(|&channel| (0..series.n_integrations).all(|row| series.is_valid(row, channel)))
        .collect();
    let mut notes =
        vec![format!("{} of {} channels are valid in every integration", valid_in_every_row.len(), series.n_channels)];
    let excluded = |channel: usize| request.exclude_um.iter().any(|[from, to]| (*from..=*to).contains(&series.grid_um[channel]));
    let mut channels: Vec<usize> = valid_in_every_row.iter().copied().filter(|&channel| !excluded(channel)).collect();
    let removed = valid_in_every_row.len() - channels.len();
    if removed > 0 {
        notes.push(format!("{removed} channel(s) removed by the exclusion ranges"));
    }
    let mut rejected = Vec::new();
    if let Some(sigma) = request.reject_elements_sigma {
        rejected = isolated_elements(series, &channels, sigma);
        channels.retain(|channel| rejected.iter().all(|element| element.index != *channel));
        notes.push(format!("{} element(s) rejected at {sigma} robust sigma", rejected.len()));
    }
    MaskedChannels { channels, rejected, notes }
}

fn pixel_loss_warning(npixels_mean: &[f64], int_num: &[i64]) -> Option<String> {
    let known: Vec<f64> = npixels_mean.iter().copied().filter(|mean| mean.is_finite()).collect();
    if known.is_empty() {
        return None;
    }
    let floor = PIXEL_LOSS_RATIO * exact_median_f64(&known);
    let low: Vec<i64> =
        npixels_mean.iter().zip(int_num).filter(|(mean, _)| mean.is_finite() && **mean < floor).map(|(_, &number)| number).collect();
    let first = low.iter().min()?;
    let last = low.iter().max()?;
    Some(format!(
        "mean NPIXELS of the summed elements falls below {PIXEL_LOSS_RATIO} of its median in {} integration(s) (INT_NUM {first} to {last}): the extraction lost pixels there, so a depth measured over them is biased",
        low.len()
    ))
}

fn empty_curve_note(name: &str, band_um: [f64; 2], range_um: [f64; 2]) -> String {
    format!(
        "{name}: {} to {} um holds no spectral element of this series ({} to {} um)",
        band_um[0], band_um[1], range_um[0], range_um[1]
    )
}

fn curve(sum: IntervalSum, band_um: [f64; 2], limits_origin: LimitsOrigin) -> Curve {
    Curve { band_um, limits_origin, flux: sum.flux, flux_err: sum.flux_err, n_used: sum.n_used, npixels_mean: sum.npixels_mean }
}

pub fn light_curves(set: &TsoSet, series: &TsoSeries, request: &CurveRequest) -> Result<LightCurves> {
    check_request(series, request)?;
    let Some(range_um) = wavelength_range(series) else {
        bail!("this series holds no finite wavelength: there is nothing to sum");
    };
    let LimitChoice { band_um: chosen_um, origin, mut notes, mut warnings } = requested_limits(request)
        .unwrap_or_else(|| default_limits(&set.files[series.file_index[0] as usize], &series.key, request.mode));
    let band_um = [
        if chosen_um[0] == OPEN_LIMITS_UM[0] { range_um[0] } else { chosen_um[0] },
        if chosen_um[1] == OPEN_LIMITS_UM[1] { range_um[1] } else { chosen_um[1] },
    ];

    let (rows, dropped, masked) = match request.mode {
        CurveMode::Pipeline => {
            let (rows, dropped) = rows_the_pipeline_keeps(series);
            (rows, dropped, None)
        }
        CurveMode::Masked => {
            let every_row = (0..series.n_integrations).collect();
            (every_row, Dropped { nan_time: 0, repeated_time: 0 }, Some(channels_of_the_masked_mode(series, request)))
        }
    };
    if dropped.nan_time + dropped.repeated_time > 0 {
        notes.push(format!(
            "pipeline mode dropped {} integration(s) whose MJD-AVG is NaN and {} whose MJD-AVG repeats an earlier one, as the pipeline does",
            dropped.nan_time, dropped.repeated_time
        ));
    }
    if rows.is_empty() {
        warnings.push("the pipeline mode keeps no integration: MJD-AVG is NaN in every row; use the masked mode".to_string());
    }
    let summed = |interval: Interval| match &masked {
        Some(masked) => sum_channels(series, &masked.channels, interval),
        None => sum_rows(series, &rows, interval),
    };
    let white = summed(Interval::white(band_um));
    let bins: Vec<IntervalSum> = request.bins.iter().map(|&pair| summed(Interval::bin(pair))).collect();

    let rejected_elements = match masked {
        Some(masked) => {
            notes.extend(masked.notes);
            masked.rejected
        }
        None => Vec::new(),
    };
    if !rows.is_empty() {
        if !white.any_inside {
            notes.push(empty_curve_note("white light", band_um, range_um));
        }
        for (index, (&pair, bin)) in request.bins.iter().zip(&bins).enumerate() {
            if !bin.any_inside {
                notes.push(empty_curve_note(&format!("bin {}", index + 1), pair, range_um));
            }
        }
    }
    notes.push(FLUX_ERR_NOTE.to_string());

    let int_num: Vec<i64> = rows.iter().map(|&row| series.int_num[row]).collect();
    warnings.extend(pixel_loss_warning(&white.npixels_mean, &int_num));
    let time = series.time();
    Ok(LightCurves {
        series: series.key.clone(),
        mode: request.mode,
        time_scale: series.time_scale,
        time: rows.iter().map(|&row| time[row]).collect(),
        mjd_utc: rows.iter().map(|&row| series.mjd_utc_mid[row]).collect(),
        int_num,
        row_index: rows,
        flux_unit: series.flux_unit.clone(),
        white: curve(white, band_um, origin),
        bins: request.bins.iter().zip(bins).map(|(&pair, bin)| curve(bin, pair, LimitsOrigin::User)).collect(),
        dropped,
        rejected_elements,
        notes,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use serde_json::json;

    use super::*;
    use crate::core::analysis::tso::series::read_set;
    use crate::core::analysis::tso::series::test_support::ramp_set;
    use crate::core::analysis::tso::test_support::synth_file;
    use crate::core::analysis::tso::WhiteLightLog;
    use crate::core::astrometry::gwcs::test_support::env_dir;

    const NO_TEXT: [&str; 0] = [];
    const NO_ROW_DROPPED: Dropped = Dropped { nan_time: 0, repeated_time: 0 };
    const FLUX_ERR_IS_THE_APPS: &str =
        "flux_err is the square root of the summed FLUX_ERROR squared: the app's number, not a pipeline product";
    const MASKED_NEEDS_ONE_GRID: &str = "the masked mode needs one wavelength grid for every integration and this series has rows whose wavelengths differ: use the pipeline mode";
    const MASKED_ONLY: &str =
        "exclude_um and reject_elements_sigma apply to the masked mode only: the pipeline mode reproduces the pipeline sum";
    const SOSS_ORDER_2_WARNING: &str = "the pipeline sums order 2 over the order 1 range 0.93 to 2.82 um, as the archived white light does: this is not a usable light curve of order 2; the masked mode uses the range of the order";
    const SOSS_ORDER_3_WARNING: &str = "the pipeline sums order 3 over the order 1 range 0.93 to 2.82 um, as the archived white light does: this is not a usable light curve of order 3; the masked mode uses the range of the order";

    fn req(mode: CurveMode) -> CurveRequest {
        CurveRequest {
            mode,
            wavelength_min_um: None,
            wavelength_max_um: None,
            bins: Vec::new(),
            exclude_um: Vec::new(),
            reject_elements_sigma: None,
        }
    }

    fn rejecting(sigma: f64) -> CurveRequest {
        CurveRequest { reject_elements_sigma: Some(sigma), ..req(CurveMode::Masked) }
    }

    fn key(detector: &str, sporder: Option<i64>) -> SeriesKey {
        SeriesKey { detector: detector.to_string(), sporder }
    }

    fn at(set: &TsoSet, row: usize, channel: usize) -> usize {
        set.series[0].sample(row, channel)
    }

    fn curves(set: &TsoSet, request: &CurveRequest) -> LightCurves {
        light_curves(set, &set.series[0], request).unwrap()
    }

    fn refusal(set: &TsoSet, request: &CurveRequest) -> String {
        light_curves(set, &set.series[0], request).unwrap_err().to_string()
    }

    fn is_the_empty_curve(curve: &Curve, rows: usize) -> bool {
        curve.flux.len() == rows
            && curve.flux_err.len() == rows
            && curve.npixels_mean.len() == rows
            && curve.flux.iter().chain(&curve.flux_err).chain(&curve.npixels_mean).all(|value| value.is_nan())
            && curve.n_used == vec![0; rows]
    }

    #[test]
    fn pipeline_white_light_is_the_plain_sum_with_nan_skipped_and_no_dq_test() {
        let mut set = ramp_set("NRS1", Some(0), 3, 5);
        let g = set.series[0].grid_um.clone();
        let pipeline = req(CurveMode::Pipeline);

        let found = curves(&set, &pipeline);
        assert_eq!(found.series, key("NRS1", Some(0)));
        assert_eq!(found.mode, CurveMode::Pipeline);
        assert_eq!(found.time_scale, TsoTimeScale::BmjdTdb);
        assert_eq!(found.white.flux, [60.0, 110.0, 160.0]);
        assert_eq!(found.white.flux_err, [1.25f64.sqrt(); 3]);
        assert_eq!(found.white.n_used, [5, 5, 5]);
        assert_eq!(found.white.npixels_mean, [7.0, 7.0, 7.0]);
        assert_eq!(found.white.band_um, [g[0], g[4]]);
        assert_eq!(found.white.limits_origin, LimitsOrigin::Open);
        assert_eq!(found.time, set.series[0].time());
        assert_eq!(found.mjd_utc, [60000.0, 60000.01, 60000.02]);
        assert_eq!(found.int_num, [1, 2, 3]);
        assert_eq!(found.row_index, [0, 1, 2]);
        assert_eq!(found.dropped, NO_ROW_DROPPED);
        assert_eq!(found.flux_unit, "Jy");
        assert!(found.bins.is_empty());
        assert!(found.rejected_elements.is_empty());
        assert_eq!(found.notes.last().map(String::as_str), Some(FLUX_ERR_IS_THE_APPS));
        assert_eq!(found.notes, [FLUX_ERR_IS_THE_APPS]);
        assert_eq!(found.warnings, NO_TEXT);

        let nan_flux = at(&set, 1, 2);
        set.series[0].flux[nan_flux] = f64::NAN;
        let with_a_gap = curves(&set, &pipeline);
        assert_eq!(with_a_gap.white.flux, [60.0, 88.0, 160.0]);
        assert_eq!(with_a_gap.white.n_used, [5, 4, 5]);
        assert_eq!(with_a_gap.white.flux_err, [1.25f64.sqrt(), 1.0, 1.25f64.sqrt()]);
        assert_eq!(with_a_gap.white.npixels_mean, [7.0, 7.0, 7.0]);
        assert_eq!(with_a_gap.row_index, [0, 1, 2]);

        let flagged = at(&set, 2, 4);
        set.series[0].dq[flagged] = 1;
        let with_a_flag = curves(&set, &pipeline);
        assert_eq!(with_a_flag.white.flux, [60.0, 88.0, 160.0]);
        assert_eq!(with_a_flag.white.n_used, [5, 4, 5]);
        assert_eq!(with_a_flag.white.flux_err, [1.25f64.sqrt(), 1.0, 1.25f64.sqrt()]);

        set.series[0].flux_error[nan_flux] = 0.0;
        let nan_error = at(&set, 0, 3);
        set.series[0].flux_error[nan_error] = f64::NAN;
        let zero_error = at(&set, 2, 0);
        set.series[0].flux_error[zero_error] = 0.0;
        let with_unusable_errors = curves(&set, &pipeline);
        assert_eq!(with_unusable_errors.white.flux, [60.0, 88.0, 160.0]);
        assert_eq!(with_unusable_errors.white.n_used, [5, 4, 5]);
        assert_eq!(with_unusable_errors.white.flux_err, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn pipeline_mode_drops_rows_whose_mjd_avg_is_nan_or_repeats_an_earlier_one() {
        let mut set = ramp_set("NRS1", Some(0), 3, 5);
        let pipeline = req(CurveMode::Pipeline);
        set.series[0].mjd_utc_mid = vec![60000.0, f64::NAN, 60000.0];

        let found = curves(&set, &pipeline);
        assert_eq!(found.row_index, [0]);
        assert_eq!(found.int_num, [1]);
        assert_eq!(found.white.flux, [60.0]);
        assert_eq!(found.white.n_used, [5]);
        assert_eq!(found.mjd_utc, [60000.0]);
        assert_eq!(found.time_scale, TsoTimeScale::BmjdTdb);
        assert_eq!(found.time, [set.series[0].bmjd_tdb_mid[0]]);
        assert_eq!(found.dropped, Dropped { nan_time: 1, repeated_time: 1 });
        assert_eq!(
            found.notes,
            [
                "pipeline mode dropped 1 integration(s) whose MJD-AVG is NaN and 1 whose MJD-AVG repeats an earlier one, as the pipeline does",
                FLUX_ERR_IS_THE_APPS
            ]
        );
        assert_eq!(found.warnings, NO_TEXT);

        set.series[0].time_scale = TsoTimeScale::Index;
        let by_number = curves(&set, &pipeline);
        assert_eq!(by_number.time_scale, TsoTimeScale::Index);
        assert_eq!(by_number.time, [1.0]);
        assert_eq!(by_number.row_index, [0]);
        assert_eq!(by_number.dropped, Dropped { nan_time: 1, repeated_time: 1 });

        set.series[0].mjd_utc_mid = vec![60000.0, 60000.5, 60000.0];
        let repeat_of_the_first = curves(&set, &pipeline);
        assert_eq!(repeat_of_the_first.row_index, [0, 1]);
        assert_eq!(repeat_of_the_first.int_num, [1, 2]);
        assert_eq!(repeat_of_the_first.time, [1.0, 2.0]);
        assert_eq!(repeat_of_the_first.mjd_utc, [60000.0, 60000.5]);
        assert_eq!(repeat_of_the_first.white.flux, [60.0, 110.0]);
        assert_eq!(repeat_of_the_first.dropped, Dropped { nan_time: 0, repeated_time: 1 });

        set.series[0].mjd_utc_mid = vec![f64::NAN; 3];
        let none = curves(&set, &pipeline);
        assert!(none.time.is_empty() && none.mjd_utc.is_empty() && none.int_num.is_empty() && none.row_index.is_empty());
        assert!(none.white.flux.is_empty() && none.white.flux_err.is_empty());
        assert!(none.white.n_used.is_empty() && none.white.npixels_mean.is_empty());
        assert_eq!(none.dropped, Dropped { nan_time: 3, repeated_time: 0 });
        assert_eq!(none.warnings, ["the pipeline mode keeps no integration: MJD-AVG is NaN in every row; use the masked mode"]);
        assert_eq!(
            none.notes,
            [
                "pipeline mode dropped 3 integration(s) whose MJD-AVG is NaN and 0 whose MJD-AVG repeats an earlier one, as the pipeline does",
                FLUX_ERR_IS_THE_APPS
            ]
        );

        let masked = curves(&set, &req(CurveMode::Masked));
        assert_eq!(masked.row_index, [0, 1, 2]);
        assert_eq!(masked.int_num, [1, 2, 3]);
        assert_eq!(masked.time, [1.0, 2.0, 3.0]);
        assert_eq!(masked.mjd_utc.len(), 3);
        assert!(masked.mjd_utc.iter().all(|time| time.is_nan()));
        assert_eq!(masked.white.flux, [60.0, 110.0, 160.0]);
        assert_eq!(masked.dropped, NO_ROW_DROPPED);
        assert_eq!(masked.warnings, NO_TEXT);
    }

    #[test]
    fn limits_include_both_ends_and_bins_exclude_the_upper_end() {
        let set = ramp_set("NRS1", Some(0), 3, 5);
        let g = set.series[0].grid_um.clone();
        for mode in [CurveMode::Pipeline, CurveMode::Masked] {
            let both_limits = CurveRequest { wavelength_min_um: Some(g[1]), wavelength_max_um: Some(g[3]), ..req(mode) };
            let found = curves(&set, &both_limits);
            assert_eq!(found.mode, mode);
            assert_eq!(found.white.flux, [36.0, 66.0, 96.0]);
            assert_eq!(found.white.n_used, [3, 3, 3]);
            assert_eq!(found.white.band_um, [g[1], g[3]]);
            assert_eq!(found.white.limits_origin, LimitsOrigin::User);
            assert!(found.bins.is_empty());

            let pairs = vec![[g[1], g[3]], [g[3], 9.0], [5.0, 6.0]];
            let with_bins = curves(&set, &CurveRequest { bins: pairs.clone(), ..both_limits.clone() });
            assert_eq!(with_bins.white.flux, [36.0, 66.0, 96.0]);
            assert_eq!(with_bins.bins.len(), 3);
            assert_eq!(with_bins.bins[0].flux, [23.0, 43.0, 63.0]);
            assert_eq!(with_bins.bins[0].n_used, [2, 2, 2]);
            assert_eq!(with_bins.bins[0].flux_err, [0.5f64.sqrt(); 3]);
            assert_eq!(with_bins.bins[0].npixels_mean, [7.0, 7.0, 7.0]);
            assert_eq!(with_bins.bins[1].flux, [27.0, 47.0, 67.0]);
            assert_eq!(with_bins.bins[1].n_used, [2, 2, 2]);
            assert!(is_the_empty_curve(&with_bins.bins[2], 3), "{:?}", with_bins.bins[2]);
            for (bin, pair) in with_bins.bins.iter().zip(&pairs) {
                assert_eq!(bin.limits_origin, LimitsOrigin::User);
                assert_eq!(bin.band_um, *pair);
            }
            let empty_bin_note = "bin 3: 5 to 6 um holds no spectral element of this series (1 to 1.4 um)";
            assert_eq!(with_bins.notes.iter().filter(|note| note.contains("holds no spectral element")).count(), 1);
            assert_eq!(with_bins.notes[with_bins.notes.len() - 2], empty_bin_note);
            assert_eq!(with_bins.notes.last().map(String::as_str), Some(FLUX_ERR_IS_THE_APPS));

            let lower_limit_only = curves(&set, &CurveRequest { wavelength_min_um: Some(g[2]), ..req(mode) });
            assert_eq!(lower_limit_only.white.flux, [39.0, 69.0, 99.0]);
            assert_eq!(lower_limit_only.white.n_used, [3, 3, 3]);
            assert_eq!(lower_limit_only.white.band_um, [g[2], g[4]]);
            assert_eq!(lower_limit_only.white.limits_origin, LimitsOrigin::User);

            let upper_limit_only = curves(&set, &CurveRequest { wavelength_max_um: Some(g[1]), ..req(mode) });
            assert_eq!(upper_limit_only.white.flux, [21.0, 41.0, 61.0]);
            assert_eq!(upper_limit_only.white.band_um, [g[0], g[1]]);
            assert_eq!(upper_limit_only.white.limits_origin, LimitsOrigin::User);
        }
    }

    #[test]
    fn masked_mode_sums_only_channels_valid_in_every_integration() {
        let mut set = ramp_set("NRS1", Some(0), 3, 5);
        let g = set.series[0].grid_um.clone();
        let nan_flux = at(&set, 1, 2);
        set.series[0].flux[nan_flux] = f64::NAN;
        let flagged = at(&set, 2, 4);
        set.series[0].dq[flagged] = 1;

        let masked = curves(&set, &req(CurveMode::Masked));
        assert_eq!(masked.mode, CurveMode::Masked);
        assert_eq!(masked.white.flux, [34.0, 64.0, 94.0]);
        assert_eq!(masked.white.n_used, [3, 3, 3]);
        assert_eq!(masked.white.flux_err, [0.75f64.sqrt(); 3]);
        assert_eq!(masked.white.npixels_mean, [7.0, 7.0, 7.0]);
        assert_eq!(masked.white.band_um, [g[0], g[4]]);
        assert_eq!(masked.white.limits_origin, LimitsOrigin::Open);
        assert_eq!(masked.row_index, [0, 1, 2]);
        assert_eq!(masked.dropped, NO_ROW_DROPPED);
        assert!(masked.rejected_elements.is_empty());
        assert_eq!(masked.notes, ["3 of 5 channels are valid in every integration", FLUX_ERR_IS_THE_APPS]);
        assert_eq!(masked.warnings, NO_TEXT);

        let pipeline = curves(&set, &req(CurveMode::Pipeline));
        assert_eq!(pipeline.white.flux, [60.0, 88.0, 160.0]);
        assert_eq!(pipeline.white.n_used, [5, 4, 5]);

        let one_excluded = CurveRequest { exclude_um: vec![[g[0], g[0]]], ..req(CurveMode::Masked) };
        let without_the_first = curves(&set, &one_excluded);
        assert_eq!(without_the_first.white.flux, [24.0, 44.0, 64.0]);
        assert_eq!(without_the_first.white.n_used, [2, 2, 2]);
        assert_eq!(
            without_the_first.notes,
            [
                "3 of 5 channels are valid in every integration",
                "1 channel(s) removed by the exclusion ranges",
                FLUX_ERR_IS_THE_APPS
            ]
        );

        let two_ranges = CurveRequest { exclude_um: vec![[g[0], g[1]], [g[2], g[3]]], ..req(CurveMode::Masked) };
        let nothing_left = curves(&set, &two_ranges);
        assert!(is_the_empty_curve(&nothing_left.white, 3), "{:?}", nothing_left.white);
        assert_eq!(
            nothing_left.notes,
            [
                "3 of 5 channels are valid in every integration",
                "3 channel(s) removed by the exclusion ranges",
                "white light: 1 to 1.4 um holds no spectral element of this series (1 to 1.4 um)",
                FLUX_ERR_IS_THE_APPS
            ]
        );

        let moved = at(&set, 1, 4);
        set.series[0].wavelength_um[moved] = 2.0;
        set.series[0].common_grid = false;
        assert_eq!(refusal(&set, &req(CurveMode::Masked)), MASKED_NEEDS_ONE_GRID);
        let each_row_by_its_own_wavelengths = curves(&set, &CurveRequest { wavelength_max_um: Some(g[4]), ..req(CurveMode::Pipeline) });
        assert_eq!(each_row_by_its_own_wavelengths.white.flux, [60.0, 64.0, 160.0]);
        assert_eq!(each_row_by_its_own_wavelengths.white.n_used, [5, 3, 5]);
        assert_eq!(each_row_by_its_own_wavelengths.white.band_um, [g[0], g[4]]);
        let open = curves(&set, &req(CurveMode::Pipeline));
        assert_eq!(open.white.flux, [60.0, 88.0, 160.0]);
        assert_eq!(open.white.band_um, [g[0], 2.0]);
    }

    fn level_set(n_channels: usize, flux_of: impl Fn(usize, usize) -> f64) -> TsoSet {
        let mut set = ramp_set("NRS1", Some(0), 3, n_channels);
        for row in 0..3 {
            for channel in 0..n_channels {
                let sample = at(&set, row, channel);
                set.series[0].flux[sample] = flux_of(row, channel);
            }
        }
        set
    }

    fn rejected_indices(found: &LightCurves) -> Vec<usize> {
        found.rejected_elements.iter().map(|element| element.index).collect()
    }

    #[test]
    fn an_isolated_element_is_rejected_by_the_robust_distance_from_its_neighbours() {
        let ramp = |channel: usize| 100.0 + channel as f64;

        let outlier = level_set(30, |_, channel| if channel == 15 { ramp(channel) - 1000.0 } else { ramp(channel) });
        let g = outlier.series[0].grid_um.clone();
        let found = curves(&outlier, &rejecting(100.0));
        assert_eq!(
            found.rejected_elements,
            [RejectedElement {
                index: 15,
                wavelength_um: g[15],
                reason: "time-median flux -8.850000e2 is 1.226e2 robust sigma from the median 1.150000e2 of its 20 neighbours".to_string(),
            }]
        );
        assert_eq!(found.white.n_used, [29, 29, 29]);
        assert_eq!(found.white.flux[0], 3320.0);
        assert_eq!(found.white.flux, [3320.0, 3320.0, 3320.0]);
        assert_eq!(found.row_index, [0, 1, 2]);
        assert_eq!(
            found.notes,
            ["30 of 30 channels are valid in every integration", "1 element(s) rejected at 100 robust sigma", FLUX_ERR_IS_THE_APPS]
        );
        let kept = curves(&outlier, &req(CurveMode::Masked));
        assert!(kept.rejected_elements.is_empty());
        assert_eq!(kept.white.n_used, [30, 30, 30]);
        assert_eq!(kept.white.flux[0], 2435.0);

        let smooth = level_set(30, |_, channel| ramp(channel));
        let at_the_ends = curves(&smooth, &rejecting(1.4));
        assert_eq!(rejected_indices(&at_the_ends), [0, 29]);
        for element in &at_the_ends.rejected_elements {
            assert!(element.reason.contains("is 1.484e0 robust sigma"), "{}", element.reason);
            assert!(element.reason.ends_with("of its 10 neighbours"), "{}", element.reason);
        }
        assert_eq!(at_the_ends.white.n_used, [28, 28, 28]);
        let none = curves(&smooth, &rejecting(1.5));
        assert!(none.rejected_elements.is_empty());
        assert_eq!(none.white.n_used, [30, 30, 30]);
        assert_eq!(
            none.notes,
            ["30 of 30 channels are valid in every integration", "0 element(s) rejected at 1.5 robust sigma", FLUX_ERR_IS_THE_APPS]
        );

        let five = level_set(5, |_, channel| if channel == 2 { ramp(channel) + 50.0 } else { ramp(channel) });
        let of_five = curves(&five, &rejecting(3.0));
        assert_eq!(rejected_indices(&of_five), [2]);
        assert_eq!(
            of_five.rejected_elements[0].reason,
            "time-median flux 1.520000e2 is 2.248e1 robust sigma from the median 1.020000e2 of its 4 neighbours"
        );
        assert_eq!(of_five.white.flux[0], 408.0);

        let four = level_set(4, |_, channel| if channel == 2 { ramp(channel) + 50.0 } else { ramp(channel) });
        let of_four = curves(&four, &rejecting(3.0));
        assert!(of_four.rejected_elements.is_empty());
        assert_eq!(of_four.white.n_used, [4, 4, 4]);
        assert_eq!(of_four.notes[1], "0 element(s) rejected at 3 robust sigma");

        let flat = level_set(8, |_, channel| if channel == 5 { 9.0 } else { 5.0 });
        let of_flat = curves(&flat, &rejecting(1000.0));
        assert_eq!(rejected_indices(&of_flat), [5]);
        assert_eq!(
            of_flat.rejected_elements[0].reason,
            "time-median flux 9.000000e0 is inf robust sigma from the median 5.000000e0 of its 7 neighbours"
        );
        assert_eq!(of_flat.white.flux, [35.0, 35.0, 35.0]);

        let one_wild_sample = level_set(30, |row, channel| if row == 0 && channel == 15 { ramp(channel) - 1000.0 } else { ramp(channel) });
        let in_one_integration = curves(&one_wild_sample, &rejecting(3.0));
        assert!(in_one_integration.rejected_elements.is_empty());
        assert_eq!(in_one_integration.row_index, [0, 1, 2]);
        assert_eq!(in_one_integration.white.n_used, [30, 30, 30]);

        let mut with_a_gap = level_set(30, |_, channel| if channel == 15 { ramp(channel) - 1000.0 } else { ramp(channel) });
        for channel in 5..15 {
            let sample = at(&with_a_gap, 1, channel);
            with_a_gap.series[0].dq[sample] = 1;
        }
        let across_the_gap = curves(&with_a_gap, &rejecting(100.0));
        assert_eq!(rejected_indices(&across_the_gap), [15]);
        assert!(across_the_gap.rejected_elements[0].reason.ends_with("of its 15 neighbours"), "{}", across_the_gap.rejected_elements[0].reason);
        assert_eq!(across_the_gap.white.n_used, [19, 19, 19]);

        let excluded = CurveRequest { exclude_um: vec![[g[15], g[15]]], ..rejecting(100.0) };
        let already_removed = curves(&outlier, &excluded);
        assert!(already_removed.rejected_elements.is_empty());
        assert_eq!(already_removed.white.n_used, [29, 29, 29]);
        assert_eq!(
            already_removed.notes,
            [
                "30 of 30 channels are valid in every integration",
                "1 channel(s) removed by the exclusion ranges",
                "0 element(s) rejected at 100 robust sigma",
                FLUX_ERR_IS_THE_APPS
            ]
        );
    }

    #[test]
    fn rejection_and_exclusion_are_refused_in_pipeline_mode_and_bad_requests_are_named() {
        let set = ramp_set("NRS1", Some(0), 3, 5);
        let pipeline = req(CurveMode::Pipeline);
        let masked = req(CurveMode::Masked);

        let inverted = CurveRequest { wavelength_min_um: Some(2.0), wavelength_max_um: Some(1.0), ..pipeline.clone() };
        assert_eq!(
            refusal(&set, &inverted),
            "wavelength limits 2 to 1 um are empty: the lower limit must be below the upper one"
        );
        let equal = CurveRequest { wavelength_min_um: Some(1.2), wavelength_max_um: Some(1.2), ..masked.clone() };
        assert_eq!(
            refusal(&set, &equal),
            "wavelength limits 1.2 to 1.2 um are empty: the lower limit must be below the upper one"
        );
        for not_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let lower = CurveRequest { wavelength_min_um: Some(not_finite), ..pipeline.clone() };
            assert_eq!(refusal(&set, &lower), "wavelength limits must be finite numbers");
            let upper = CurveRequest { wavelength_min_um: Some(1.0), wavelength_max_um: Some(not_finite), ..masked.clone() };
            assert_eq!(refusal(&set, &upper), "wavelength limits must be finite numbers");
        }

        let too_many = CurveRequest { bins: vec![[1.0, 1.2]; 513], ..pipeline.clone() };
        assert_eq!(refusal(&set, &too_many), "513 bins requested: at most 512 are computed at once");
        let as_many_as_allowed = CurveRequest { bins: vec![[1.0, 1.2]; 512], ..pipeline.clone() };
        assert_eq!(curves(&set, &as_many_as_allowed).bins.len(), 512);
        let empty_bin = CurveRequest { bins: vec![[1.0, 1.2], [1.3, 1.3]], ..pipeline.clone() };
        assert_eq!(refusal(&set, &empty_bin), "bin 2: 1.3 to 1.3 um is empty or not finite");

        let not_a_range = CurveRequest { exclude_um: vec![[2.0, 1.0]], ..masked.clone() };
        assert_eq!(refusal(&set, &not_a_range), "exclude range 1: 2 to 1 um is not a range");
        let second_not_finite = CurveRequest { exclude_um: vec![[1.0, 1.1], [1.2, f64::NAN]], ..masked.clone() };
        assert_eq!(refusal(&set, &second_not_finite), "exclude range 2: 1.2 to NaN um is not a range");

        assert_eq!(refusal(&set, &rejecting(0.0)), "reject_elements_sigma must be above 0, got 0");
        assert_eq!(refusal(&set, &rejecting(-3.0)), "reject_elements_sigma must be above 0, got -3");
        assert_eq!(refusal(&set, &rejecting(f64::NAN)), "reject_elements_sigma must be above 0, got NaN");
        assert_eq!(refusal(&set, &rejecting(f64::INFINITY)), "reject_elements_sigma must be above 0, got inf");

        let excluding = CurveRequest { exclude_um: vec![[1.0, 1.1]], ..pipeline.clone() };
        assert_eq!(refusal(&set, &excluding), MASKED_ONLY);
        let rejecting_in_pipeline = CurveRequest { reject_elements_sigma: Some(5.0), ..pipeline.clone() };
        assert_eq!(refusal(&set, &rejecting_in_pipeline), MASKED_ONLY);

        let first_refusal_wins = CurveRequest {
            mode: CurveMode::Pipeline,
            wavelength_min_um: Some(2.0),
            wavelength_max_um: Some(1.0),
            bins: vec![[1.3, 1.3]],
            exclude_um: vec![[2.0, 1.0]],
            reject_elements_sigma: Some(0.0),
        };
        assert_eq!(
            refusal(&set, &first_refusal_wins),
            "wavelength limits 2 to 1 um are empty: the lower limit must be below the upper one"
        );
        let then_the_bins = CurveRequest { wavelength_min_um: None, ..first_refusal_wins };
        assert_eq!(refusal(&set, &then_the_bins), "bin 1: 1.3 to 1.3 um is empty or not finite");
        let then_the_exclusion = CurveRequest { bins: Vec::new(), ..then_the_bins };
        assert_eq!(refusal(&set, &then_the_exclusion), "exclude range 1: 2 to 1 um is not a range");
        let then_the_sigma = CurveRequest { exclude_um: vec![[1.0, 2.0]], ..then_the_exclusion };
        assert_eq!(refusal(&set, &then_the_sigma), "reject_elements_sigma must be above 0, got 0");

        let mut without_wavelengths = ramp_set("NRS1", Some(0), 3, 5);
        without_wavelengths.series[0].wavelength_um.fill(f64::NAN);
        without_wavelengths.series[0].grid_um.fill(f64::NAN);
        for request in [&pipeline, &masked] {
            assert_eq!(
                refusal(&without_wavelengths, request),
                "this series holds no finite wavelength: there is nothing to sum"
            );
        }
    }

    fn mode_file(exp_type: Option<&str>, filter: Option<&str>, log: Option<(Option<f64>, Option<f64>)>) -> TsoFile {
        let mut file = synth_file("a_x1dints.fits", "", "", 3);
        file.exp_type = exp_type.map(str::to_string);
        file.filter = filter.map(str::to_string);
        file.whitelight_log = log.map(|(min_um, max_um)| WhiteLightLog { min_um, max_um });
        file
    }

    fn choice(band_um: [f64; 2], origin: LimitsOrigin, notes: &[&str], warnings: &[&str]) -> LimitChoice {
        LimitChoice {
            band_um,
            origin,
            notes: notes.iter().map(|note| note.to_string()).collect(),
            warnings: warnings.iter().map(|warning| warning.to_string()).collect(),
        }
    }

    #[test]
    fn default_limits_follow_the_file_log_then_the_built_in_table() {
        let both_modes = [CurveMode::Pipeline, CurveMode::Masked];
        let not_logged = Some((None, None));

        let grism_logged = mode_file(Some("NRC_TSGRISM"), Some("F322W2"), Some((Some(2.47), Some(3.94))));
        assert_eq!(
            default_limits(&grism_logged, &key("NRCALONG", Some(1)), CurveMode::Pipeline),
            choice(
                [2.47, 3.94],
                LimitsOrigin::FileLog,
                &["white-light limits 2.47 to 3.94 um are the ones the pipeline logged in a_x1dints.fits"],
                &[]
            )
        );
        let grism = mode_file(Some("NRC_TSGRISM"), Some("F322W2"), None);
        for mode in both_modes {
            assert_eq!(
                default_limits(&grism, &key("NRCALONG", Some(1)), mode),
                choice([2.47, 3.94], LimitsOrigin::BuiltInDefault, &[], &[])
            );
        }

        let lrs = mode_file(Some("MIR_LRS-SLITLESS"), Some("P750L"), not_logged);
        assert_eq!(
            default_limits(&lrs, &key("MIRIMAGE", Some(1)), CurveMode::Pipeline),
            choice([5.0, 12.0], LimitsOrigin::BuiltInDefault, &[], &[])
        );
        let lrs_without_filter = mode_file(Some("MIR_LRS-SLITLESS"), None, None);
        assert_eq!(
            default_limits(&lrs_without_filter, &key("MIRIMAGE", None), CurveMode::Masked),
            choice([5.0, 12.0], LimitsOrigin::BuiltInDefault, &[], &[])
        );
        let lrs_lower_logged = mode_file(Some("MIR_LRS-SLITLESS"), Some("P750L"), Some((Some(5.5), None)));
        assert_eq!(
            default_limits(&lrs_lower_logged, &key("MIRIMAGE", Some(1)), CurveMode::Masked),
            choice(
                [5.5, 1.0e10],
                LimitsOrigin::FileLog,
                &["white-light limits 5.5 to open um are the ones the pipeline logged in a_x1dints.fits"],
                &[]
            )
        );
        let lrs_upper_logged = mode_file(Some("MIR_LRS-SLITLESS"), Some("P750L"), Some((None, Some(11.0))));
        assert_eq!(
            default_limits(&lrs_upper_logged, &key("MIRIMAGE", Some(1)), CurveMode::Pipeline),
            choice(
                [-1.0, 11.0],
                LimitsOrigin::FileLog,
                &["white-light limits open to 11 um are the ones the pipeline logged in a_x1dints.fits"],
                &[]
            )
        );

        let nirspec = mode_file(Some("NRS_BRIGHTOBJ"), Some("CLEAR"), not_logged);
        for mode in both_modes {
            assert_eq!(
                default_limits(&nirspec, &key("NRS1", Some(0)), mode),
                choice(OPEN_LIMITS_UM, LimitsOrigin::Open, &[], &[])
            );
        }

        let soss = mode_file(Some("NIS_SOSS"), Some("CLEAR"), not_logged);
        for mode in both_modes {
            assert_eq!(
                default_limits(&soss, &key("NIS", Some(1)), mode),
                choice([0.93, 2.82], LimitsOrigin::BuiltInDefault, &[], &[])
            );
        }
        assert_eq!(
            default_limits(&soss, &key("NIS", Some(2)), CurveMode::Pipeline),
            choice([0.93, 2.82], LimitsOrigin::BuiltInDefault, &[], &[SOSS_ORDER_2_WARNING])
        );
        assert_eq!(
            default_limits(&soss, &key("NIS", Some(3)), CurveMode::Pipeline),
            choice([0.93, 2.82], LimitsOrigin::BuiltInDefault, &[], &[SOSS_ORDER_3_WARNING])
        );
        assert_eq!(
            default_limits(&soss, &key("NIS", None), CurveMode::Pipeline),
            choice([0.93, 2.82], LimitsOrigin::BuiltInDefault, &[], &[])
        );
        assert_eq!(
            default_limits(&soss, &key("NIS", Some(2)), CurveMode::Masked),
            choice([0.64, 0.83], LimitsOrigin::BuiltInDefault, &[], &[])
        );
        assert_eq!(
            default_limits(&soss, &key("NIS", Some(3)), CurveMode::Masked),
            choice([0.64, 0.95], LimitsOrigin::BuiltInDefault, &[], &[])
        );
        let soss_order_without_a_range =
            "no white-light limits are known for NIS_SOSS CLEAR: every wavelength is summed; set limits to match the pipeline";
        for sporder in [Some(4), Some(0), None] {
            assert_eq!(
                default_limits(&soss, &key("NIS", sporder), CurveMode::Masked),
                choice(OPEN_LIMITS_UM, LimitsOrigin::Open, &[soss_order_without_a_range], &[])
            );
        }

        let soss_other_filter = mode_file(Some("NIS_SOSS"), Some("F277W"), None);
        assert_eq!(
            default_limits(&soss_other_filter, &key("NIS", Some(1)), CurveMode::Pipeline),
            choice(
                OPEN_LIMITS_UM,
                LimitsOrigin::Open,
                &["no white-light limits are known for NIS_SOSS F277W: every wavelength is summed; set limits to match the pipeline"],
                &[]
            )
        );
        let grism_other_filter = mode_file(Some("NRC_TSGRISM"), Some("F444W"), not_logged);
        assert_eq!(
            default_limits(&grism_other_filter, &key("NRCALONG", Some(1)), CurveMode::Masked),
            choice(
                OPEN_LIMITS_UM,
                LimitsOrigin::Open,
                &["no white-light limits are known for NRC_TSGRISM F444W: every wavelength is summed; set limits to match the pipeline"],
                &[]
            )
        );
        let grism_without_filter = mode_file(Some("NRC_TSGRISM"), None, None);
        assert_eq!(
            default_limits(&grism_without_filter, &key("NRCALONG", Some(1)), CurveMode::Pipeline),
            choice(
                OPEN_LIMITS_UM,
                LimitsOrigin::Open,
                &["no white-light limits are known for NRC_TSGRISM: every wavelength is summed; set limits to match the pipeline"],
                &[]
            )
        );
        let unknown = mode_file(None, Some("CLEAR"), None);
        assert_eq!(
            default_limits(&unknown, &key("X", None), CurveMode::Pipeline),
            choice(
                OPEN_LIMITS_UM,
                LimitsOrigin::Open,
                &["no white-light limits are known for unknown mode: every wavelength is summed; set limits to match the pipeline"],
                &[]
            )
        );
    }

    #[test]
    fn light_curves_use_the_default_limits_of_the_first_file_of_the_series_and_report_the_range_summed() {
        let mut set = ramp_set("NRS1", Some(0), 3, 5);
        set.files[0].exp_type = Some("MIR_LRS-SLITLESS".to_string());
        let grid: Vec<f64> = (0..5).map(|channel| 4.0 + 2.0 * f64::from(channel)).collect();
        set.series[0].wavelength_um = grid.repeat(3);
        set.series[0].grid_um = grid;
        let pipeline = req(CurveMode::Pipeline);

        let built_in = curves(&set, &pipeline);
        assert_eq!(built_in.white.flux[0], 50.0);
        assert_eq!(built_in.white.flux, [50.0, 90.0, 130.0]);
        assert_eq!(built_in.white.n_used, [4, 4, 4]);
        assert_eq!(built_in.white.band_um, [5.0, 12.0]);
        assert_eq!(built_in.white.limits_origin, LimitsOrigin::BuiltInDefault);
        assert_eq!(built_in.notes, [FLUX_ERR_IS_THE_APPS]);
        let built_in_masked = curves(&set, &req(CurveMode::Masked));
        assert_eq!(built_in_masked.white.flux, [50.0, 90.0, 130.0]);
        assert_eq!(built_in_masked.white.band_um, [5.0, 12.0]);
        assert_eq!(built_in_masked.white.limits_origin, LimitsOrigin::BuiltInDefault);

        set.files[0].whitelight_log = Some(WhiteLightLog { min_um: None, max_um: Some(9.0) });
        let logged = curves(&set, &pipeline);
        assert_eq!(logged.white.flux, [33.0, 63.0, 93.0]);
        assert_eq!(logged.white.n_used, [3, 3, 3]);
        assert_eq!(logged.white.band_um, [4.0, 9.0]);
        assert_eq!(logged.white.limits_origin, LimitsOrigin::FileLog);
        assert_eq!(
            logged.notes,
            ["white-light limits open to 9 um are the ones the pipeline logged in ramp_x1dints.fits", FLUX_ERR_IS_THE_APPS]
        );

        let asked = curves(&set, &CurveRequest { wavelength_max_um: Some(7.0), ..pipeline.clone() });
        assert_eq!(asked.white.flux, [21.0, 41.0, 61.0]);
        assert_eq!(asked.white.n_used, [2, 2, 2]);
        assert_eq!(asked.white.band_um, [4.0, 7.0]);
        assert_eq!(asked.white.limits_origin, LimitsOrigin::User);
        assert_eq!(asked.notes, [FLUX_ERR_IS_THE_APPS]);

        set.files.push(synth_file("grism_x1dints.fits", "NRC_TSGRISM", "F322W2", 2));
        set.series[0].file_index = vec![1, 0, 0];
        let of_the_first_row = curves(&set, &pipeline);
        assert_eq!(of_the_first_row.white.band_um, [2.47, 3.94]);
        assert_eq!(of_the_first_row.white.limits_origin, LimitsOrigin::BuiltInDefault);
        assert!(is_the_empty_curve(&of_the_first_row.white, 3), "{:?}", of_the_first_row.white);
        assert_eq!(
            of_the_first_row.notes,
            ["white light: 2.47 to 3.94 um holds no spectral element of this series (4 to 12 um)", FLUX_ERR_IS_THE_APPS]
        );
        assert_eq!(of_the_first_row.warnings, NO_TEXT);

        let mut soss = ramp_set("NIS", Some(2), 3, 5);
        soss.files[0].exp_type = Some("NIS_SOSS".to_string());
        let summed_as_the_archive = curves(&soss, &pipeline);
        assert_eq!(summed_as_the_archive.white.band_um, [0.93, 2.82]);
        assert_eq!(summed_as_the_archive.white.flux, [60.0, 110.0, 160.0]);
        assert_eq!(summed_as_the_archive.warnings, [SOSS_ORDER_2_WARNING]);
        let asked_of_soss = curves(&soss, &CurveRequest { wavelength_min_um: Some(0.5), ..pipeline.clone() });
        assert_eq!(asked_of_soss.white.limits_origin, LimitsOrigin::User);
        assert_eq!(asked_of_soss.warnings, NO_TEXT);
    }

    fn losing_pixels(npixels_of_the_third_row: f64) -> TsoSet {
        let mut set = ramp_set("NRS1", Some(0), 5, 4);
        for channel in 0..4 {
            let sample = at(&set, 2, channel);
            set.series[0].npixels[sample] = npixels_of_the_third_row;
        }
        set
    }

    #[test]
    fn integrations_that_lose_pixels_are_named_in_a_warning() {
        let half_lost = losing_pixels(3.0);
        let slightly_lower = losing_pixels(6.5);
        for mode in [CurveMode::Pipeline, CurveMode::Masked] {
            let found = curves(&half_lost, &req(mode));
            assert_eq!(found.white.npixels_mean, [7.0, 7.0, 3.0, 7.0, 7.0]);
            assert_eq!(
                found.warnings,
                ["mean NPIXELS of the summed elements falls below 0.9 of its median in 1 integration(s) (INT_NUM 3 to 3): the extraction lost pixels there, so a depth measured over them is biased"]
            );
            assert_eq!(curves(&slightly_lower, &req(mode)).warnings, NO_TEXT);
        }

        let mut two_apart = losing_pixels(3.0);
        for channel in 0..4 {
            let sample = at(&two_apart, 0, channel);
            two_apart.series[0].npixels[sample] = 5.0;
            let unknown = at(&two_apart, 4, channel);
            two_apart.series[0].npixels[unknown] = f64::NAN;
        }
        let found = curves(&two_apart, &req(CurveMode::Pipeline));
        assert!(found.white.npixels_mean[4].is_nan());
        assert_eq!(
            found.warnings,
            ["mean NPIXELS of the summed elements falls below 0.9 of its median in 2 integration(s) (INT_NUM 1 to 3): the extraction lost pixels there, so a depth measured over them is biased"]
        );
    }

    #[test]
    fn light_curves_serialise_with_the_names_and_nulls_the_frontend_reads() {
        let set = ramp_set("NRS1", Some(0), 3, 5);
        let g = set.series[0].grid_um.clone();
        let request = CurveRequest {
            wavelength_min_um: Some(g[1]),
            wavelength_max_um: Some(g[3]),
            bins: vec![[g[1], g[3]], [g[3], 9.0], [5.0, 6.0]],
            ..req(CurveMode::Pipeline)
        };
        let value = serde_json::to_value(curves(&set, &request)).unwrap();
        let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "bins", "dropped", "flux_unit", "int_num", "mjd_utc", "mode", "notes", "rejected_elements", "row_index",
                "series", "time", "time_scale", "warnings", "white"
            ]
        );
        assert_eq!(value["mode"], json!("pipeline"));
        assert_eq!(value["time_scale"], json!("bmjd_tdb"));
        assert_eq!(value["flux_unit"], json!("Jy"));
        assert_eq!(value["int_num"], json!([1, 2, 3]));
        assert_eq!(value["row_index"], json!([0, 1, 2]));
        assert_eq!(value["white"]["limits_origin"], json!("user"));
        assert_eq!(value["white"]["band_um"], json!([g[1], g[3]]));
        assert_eq!(value["white"]["flux"], json!([36.0, 66.0, 96.0]));
        assert_eq!(value["white"]["n_used"], json!([3, 3, 3]));
        assert_eq!(value["white"]["npixels_mean"], json!([7.0, 7.0, 7.0]));
        let curve_keys: Vec<&str> = value["white"].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(curve_keys, ["band_um", "flux", "flux_err", "limits_origin", "n_used", "npixels_mean"]);
        assert_eq!(value["bins"][0]["limits_origin"], json!("user"));
        assert_eq!(value["bins"][2]["flux"], json!([null, null, null]));
        assert_eq!(value["bins"][2]["flux_err"], json!([null, null, null]));
        assert_eq!(value["bins"][2]["npixels_mean"], json!([null, null, null]));
        assert_eq!(value["bins"][2]["n_used"], json!([0, 0, 0]));
        assert_eq!(value["bins"][2]["band_um"], json!([5.0, 6.0]));
        assert_eq!(value["dropped"], json!({"nan_time": 0, "repeated_time": 0}));
        assert_eq!(value["series"], json!({"detector": "NRS1", "sporder": 0}));
        assert_eq!(value["rejected_elements"], json!([]));
        assert_eq!(value["warnings"], json!([]));

        let mut without_times = set.clone();
        without_times.series[0].mjd_utc_mid = vec![f64::NAN; 3];
        let masked = serde_json::to_value(curves(&without_times, &rejecting(1.4))).unwrap();
        assert_eq!(masked["mode"], json!("masked"));
        assert_eq!(masked["mjd_utc"], json!([null, null, null]));
        assert_eq!(masked["white"]["limits_origin"], json!("open"));
        assert_eq!(masked["rejected_elements"][0]["index"], json!(0));
        assert_eq!(masked["rejected_elements"][0]["wavelength_um"], json!(g[0]));
        let element_keys: Vec<&str> = masked["rejected_elements"][0].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(element_keys, ["index", "reason", "wavelength_um"]);

        assert_eq!(serde_json::to_value(LimitsOrigin::Open).unwrap(), json!("open"));
        assert_eq!(serde_json::to_value(LimitsOrigin::User).unwrap(), json!("user"));
        assert_eq!(serde_json::to_value(LimitsOrigin::FileLog).unwrap(), json!("file log"));
        assert_eq!(serde_json::to_value(LimitsOrigin::BuiltInDefault).unwrap(), json!("built-in default"));
        assert_eq!(serde_json::to_value(CurveMode::Masked).unwrap(), json!("masked"));

        let parsed: CurveRequest =
            serde_json::from_value(json!({"mode": "masked", "bins": [[1.0, 2.0]], "reject_elements_sigma": 100.0})).unwrap();
        assert_eq!(parsed, CurveRequest { bins: vec![[1.0, 2.0]], ..rejecting(100.0) });
        assert!(parsed.exclude_um.is_empty());
        assert_eq!((parsed.wavelength_min_um, parsed.wavelength_max_um), (None, None));
        let whole: CurveRequest = serde_json::from_value(json!({
            "mode": "pipeline", "wavelength_min_um": 1.0, "wavelength_max_um": 5.0, "exclude_um": [[2.5, 3.0]]
        }))
        .unwrap();
        assert_eq!(
            whole,
            CurveRequest {
                wavelength_min_um: Some(1.0),
                wavelength_max_um: Some(5.0),
                exclude_um: vec![[2.5, 3.0]],
                ..req(CurveMode::Pipeline)
            }
        );
        assert!(serde_json::from_value::<CurveRequest>(json!({"bins": []})).is_err());
    }

    const A1: &str = "A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits";
    const A3: &str = "A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits";
    const A4: &str = "A4/jw04498-o013_t008_miri_p750l-slitlessprism_x1dints.fits";
    const A5: &str = "A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits";
    const A6: &str = "A6/jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_x1dints.fits";
    const B1: &str = "B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits";
    const C1: &str = "C/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits";
    const E1: &str = "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits";
    const CURVES_ORACLE: &str = "python -I plan/pacotes/TSO-support/tso_checks.py curves <data root>/tso 1000";
    const SETS_ORACLE: &str = "python -I plan/pacotes/TSO-support/tso_checks.py sets <data root>/tso";
    const E1_BINS: [[f64; 2]; 5] = [[0.6, 1.0], [1.0, 2.0], [2.0, 3.0], [3.0, 4.0], [4.0, 5.3]];
    const REAL_SERIES: [(&str, &str, i64); 11] = [
        (A1, "NRCALONG", 1),
        (A3, "NIS", 1),
        (A3, "NIS", 2),
        (A3, "NIS", 3),
        (A4, "MIRIMAGE", 1),
        (A5, "NRS1", -1),
        (A5, "NRS2", -1),
        (A6, "NRS1", 0),
        (B1, "MIRIMAGE", 1),
        (C1, "NRS1", -1),
        (E1, "NRS1", 0),
    ];

    fn real_set(relative: &str) -> TsoSet {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("tso").join(relative);
        read_set(&[path.to_string_lossy().into_owned()]).unwrap()
    }

    fn real_curves(relative: &str, detector: &str, sporder: i64, request: &CurveRequest) -> LightCurves {
        let set = real_set(relative);
        let wanted = key(detector, Some(sporder));
        let series = set
            .series
            .iter()
            .find(|series| series.key == wanted)
            .unwrap_or_else(|| panic!("{relative} holds no series {detector} with SPORDER {sporder}"));
        light_curves(&set, series, request).unwrap()
    }

    fn set_label(relative: &str) -> &str {
        relative.split('/').next().unwrap()
    }

    fn golden_of(relative: &str) -> String {
        let stem = relative.rsplit('/').next().unwrap().trim_end_matches("_x1dints.fits");
        format!("white_light_as_archived of plan/goldens/tso/x1dints_{stem}.json")
    }

    fn relative_difference(found: f64, expected: f64) -> f64 {
        ((found - expected) / expected).abs()
    }

    fn assert_close(found: f64, expected: f64, what: &str) {
        assert!(
            (found - expected).abs() <= 1e-12 * expected.abs(),
            "{what}: found {found:?}, expected {expected:?}, relative difference {:e}",
            relative_difference(found, expected)
        );
    }

    fn assert_all_close(found: &[f64], expected: &[f64], what: &str) {
        assert_eq!(found.len(), expected.len(), "{what}: count");
        for (index, (value, wanted)) in found.iter().zip(expected).enumerate() {
            assert_close(*value, *wanted, &format!("{what} [{index}]"));
        }
    }

    fn total(flux: &[f64]) -> f64 {
        flux.iter().sum()
    }

    fn used_range(curve: &Curve) -> [u32; 2] {
        [*curve.n_used.iter().min().unwrap(), *curve.n_used.iter().max().unwrap()]
    }

    struct Archived {
        relative: &'static str,
        detector: &'static str,
        sporder: i64,
        limits_um: Option<[f64; 2]>,
        origin: LimitsOrigin,
        rows_kept: usize,
        dropped_nan_time: usize,
        n_used: [u32; 2],
        first: &'static [f64],
        last: f64,
        sum: f64,
    }

    fn archived() -> Vec<Archived> {
        let one_row = |relative, detector, sporder, limits_um, origin, dropped_nan_time, n_used: u32, first: &'static [f64]| Archived {
            relative,
            detector,
            sporder,
            limits_um: Some(limits_um),
            origin,
            rows_kept: 1,
            dropped_nan_time,
            n_used: [n_used, n_used],
            first,
            last: first[0],
            sum: first[0],
        };
        vec![
            one_row(A1, "NRCALONG", 1, NIRCAM_F322W2_UM, LimitsOrigin::FileLog, 3, 1514, &[65197945.38277817]),
            one_row(A3, "NIS", 1, SOSS_ORDER1_UM, LimitsOrigin::BuiltInDefault, 4, 1941, &[1503.7119146038247]),
            one_row(A3, "NIS", 2, SOSS_ORDER1_UM, LimitsOrigin::BuiltInDefault, 4, 1012, &[14055.279010227225]),
            one_row(A3, "NIS", 3, SOSS_ORDER1_UM, LimitsOrigin::BuiltInDefault, 4, 75, &[37.554793231367526]),
            Archived {
                relative: A4,
                detector: "MIRIMAGE",
                sporder: 1,
                limits_um: Some(MIRI_LRS_UM),
                origin: LimitsOrigin::FileLog,
                rows_kept: 64,
                dropped_nan_time: 0,
                n_used: [247, 247],
                first: &[105.1244014781455, 106.12335012948235, 106.5334923923868],
                last: 119.50950890995946,
                sum: 7420.285204061691,
            },
            Archived {
                relative: A5,
                detector: "NRS1",
                sporder: -1,
                limits_um: None,
                origin: LimitsOrigin::Open,
                rows_kept: 8,
                dropped_nan_time: 0,
                n_used: [1239, 1239],
                first: &[-5953.629162915719, -6100.921631032233, -5799.353148838366],
                last: -6114.179097844156,
                sum: -48889.218080477076,
            },
            Archived {
                relative: A5,
                detector: "NRS2",
                sporder: -1,
                limits_um: None,
                origin: LimitsOrigin::Open,
                rows_kept: 8,
                dropped_nan_time: 0,
                n_used: [2039, 2039],
                first: &[3.712769405439865, 3.717856876332009, 3.7136729465646683],
                last: 3.7026733212328358,
                sum: 29.66909685181698,
            },
            Archived {
                relative: A6,
                detector: "NRS1",
                sporder: 0,
                limits_um: None,
                origin: LimitsOrigin::Open,
                rows_kept: 65,
                dropped_nan_time: 0,
                n_used: [338, 408],
                first: &[0.016091166200930186, 0.016399022694723386, 0.016655971218064686],
                last: 0.01572451606661172,
                sum: 1.0538909536835777,
            },
            Archived {
                relative: B1,
                detector: "MIRIMAGE",
                sporder: 1,
                limits_um: Some(MIRI_LRS_UM),
                origin: LimitsOrigin::BuiltInDefault,
                rows_kept: 10,
                dropped_nan_time: 0,
                n_used: [243, 243],
                first: &[2.163616608616668, 2.3930142098838867, 2.3525511707014934],
                last: 2.362247136444509,
                sum: 23.347534963816415,
            },
            Archived {
                relative: C1,
                detector: "NRS1",
                sporder: -1,
                limits_um: Some([2.848429652675818, 5.182052338975267]),
                origin: LimitsOrigin::Open,
                rows_kept: 44,
                dropped_nan_time: 0,
                n_used: [1276, 1276],
                first: &[-219.0388902270913, -226.98763483456162, -225.4154675778334],
                last: -232.7479249095655,
                sum: -8701.993488389966,
            },
            Archived {
                relative: E1,
                detector: "NRS1",
                sporder: 0,
                limits_um: Some([0.5530469695008425, 5.370352185327354]),
                origin: LimitsOrigin::Open,
                rows_kept: 240,
                dropped_nan_time: 0,
                n_used: [327, 407],
                first: &[0.16369951048163586, 0.16398261037017267, 0.16391382997867546],
                last: 0.1640142699204498,
                sum: 37.761493394107156,
            },
        ]
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/{A1,A3,A4,A5,A6,B1,C,E1}/*-o0*_x1dints.fits"]
    fn real_pipeline_white_light_reproduces_the_archived_sums_with_the_limits_of_each_mode() {
        for expected in archived() {
            let golden = golden_of(expected.relative);
            let label = format!("{} {} order {}", set_label(expected.relative), expected.detector, expected.sporder);
            let found = real_curves(expected.relative, expected.detector, expected.sporder, &req(CurveMode::Pipeline));
            let white = &found.white;
            let rows = white.flux.len();
            assert_eq!(rows, expected.rows_kept, "{label} rows_kept, {golden}");

            let mut largest = relative_difference(total(&white.flux), expected.sum);
            largest = largest.max(relative_difference(white.flux[rows - 1], expected.last));
            for (value, wanted) in white.flux.iter().zip(expected.first) {
                largest = largest.max(relative_difference(*value, *wanted));
            }
            println!("{label}: largest relative difference of first, last and sum against the archive {largest:e}");

            assert_eq!(found.mode, CurveMode::Pipeline, "{label} mode");
            assert_eq!(white.limits_origin, expected.origin, "{label} limits origin, limits_source of {golden}");
            match expected.limits_um {
                Some(limits_um) => assert_eq!(white.band_um, limits_um, "{label} limits_um, {golden}"),
                None => assert!(white.band_um[0] < white.band_um[1], "{label} band {:?}", white.band_um),
            }
            assert_eq!(
                found.dropped,
                Dropped { nan_time: expected.dropped_nan_time, repeated_time: 0 },
                "{label} rows_dropped_nan_or_repeated_mjd_avg, {golden}"
            );
            assert_eq!(found.row_index, (0..expected.rows_kept).collect::<Vec<usize>>(), "{label} rows kept, {golden}");
            assert_eq!(used_range(white), expected.n_used, "{label} n_used, {CURVES_ORACLE}");
            assert_all_close(&white.flux[..expected.first.len()], expected.first, &format!("{label} first5, {golden}"));
            assert_close(white.flux[rows - 1], expected.last, &format!("{label} last5, {golden}"));
            assert_close(total(&white.flux), expected.sum, &format!("{label} sum_all_fsum, {golden}"));
            assert!(found.bins.is_empty() && found.rejected_elements.is_empty(), "{label} bins and rejected elements");

            if expected.rows_kept == 1 {
                assert_eq!(found.time_scale, TsoTimeScale::Index, "{label} time scale, {golden}");
                assert_eq!(found.time, [1.0], "{label} time, {golden}");
                assert_eq!(found.int_num, [1], "{label} INT_NUM, {golden}");
            } else {
                assert_eq!(found.time_scale, TsoTimeScale::BmjdTdb, "{label} time scale, {golden}");
            }
            let soss_warnings: Vec<&String> = found.warnings.iter().filter(|warning| warning.starts_with("the pipeline sums order")).collect();
            match (expected.relative, expected.sporder) {
                (A3, 2) => assert_eq!(soss_warnings, [SOSS_ORDER_2_WARNING], "{label} warning of the reused range"),
                (A3, 3) => assert_eq!(soss_warnings, [SOSS_ORDER_3_WARNING], "{label} warning of the reused range"),
                _ => assert!(soss_warnings.is_empty(), "{label} warnings {:?}", found.warnings),
            }
            let dropped_note = found.notes.iter().any(|note| note.starts_with("pipeline mode dropped"));
            assert_eq!(dropped_note, expected.dropped_nan_time > 0, "{label} notes {:?}", found.notes);
            let logged_note = found.notes.iter().any(|note| note.contains("are the ones the pipeline logged in"));
            assert_eq!(logged_note, expected.origin == LimitsOrigin::FileLog, "{label} notes {:?}", found.notes);
            assert_eq!(found.notes.last().map(String::as_str), Some(FLUX_ERR_IS_THE_APPS), "{label} last note");
        }

        let a1 = real_curves(A1, "NRCALONG", 1, &req(CurveMode::Pipeline));
        assert_eq!(
            a1.notes,
            [
                "white-light limits 2.47 to 3.94 um are the ones the pipeline logged in jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits",
                "pipeline mode dropped 3 integration(s) whose MJD-AVG is NaN and 0 whose MJD-AVG repeats an earlier one, as the pipeline does",
                FLUX_ERR_IS_THE_APPS
            ],
            "A1 notes"
        );

        let a5 = real_curves(A5, "NRS1", -1, &req(CurveMode::Pipeline));
        assert_close(a5.white.flux_err[0], 184.59832589076666, &format!("A5 NRS1 flux_err[0], {CURVES_ORACLE}"));
        let e1 = real_curves(E1, "NRS1", 0, &req(CurveMode::Pipeline));
        assert_close(e1.white.flux_err[0], 8.396453646622454e-05, &format!("E1 flux_err[0], {CURVES_ORACLE}"));
        assert_close(e1.white.npixels_mean[0], 6.936174931639071, &format!("E1 npixels_mean[0], {CURVES_ORACLE}"));
        let a4 = real_curves(A4, "MIRIMAGE", 1, &req(CurveMode::Pipeline));
        assert_close(a4.white.flux_err[0], 0.03195249273437639, &format!("A4 flux_err[0], {CURVES_ORACLE}"));
        assert_close(a4.white.npixels_mean[0], 11.631578947368421, &format!("A4 npixels_mean[0], {CURVES_ORACLE}"));
    }

    struct Masked {
        relative: &'static str,
        detector: &'static str,
        sporder: i64,
        limits_um: Option<[f64; 2]>,
        origin: LimitsOrigin,
        rows: usize,
        n_used: u32,
        first: [f64; 3],
        sum: f64,
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/{A1,A3,A4,A6,B1,E1}/*-o0*_x1dints.fits"]
    fn real_masked_white_light_keeps_one_set_of_channels_and_the_ranges_of_each_soss_order() {
        let table = [
            Masked {
                relative: A1,
                detector: "NRCALONG",
                sporder: 1,
                limits_um: Some(NIRCAM_F322W2_UM),
                origin: LimitsOrigin::FileLog,
                rows: 4,
                n_used: 1514,
                first: [65197945.38277817, 65301954.81903076, 65258041.191410065],
                sum: 261022668.07726288,
            },
            Masked {
                relative: A3,
                detector: "NIS",
                sporder: 1,
                limits_um: Some(SOSS_ORDER1_UM),
                origin: LimitsOrigin::BuiltInDefault,
                rows: 5,
                n_used: 1941,
                first: [1503.7119146038247, 1503.777442805529, 1503.9937065740291],
                sum: 7519.127987288344,
            },
            Masked {
                relative: A3,
                detector: "NIS",
                sporder: 2,
                limits_um: Some(SOSS_ORDER2_UM),
                origin: LimitsOrigin::BuiltInDefault,
                rows: 5,
                n_used: 427,
                first: [74.12198300093027, 74.04160493748968, 74.09714526848198],
                sum: 370.37380908767545,
            },
            Masked {
                relative: A3,
                detector: "NIS",
                sporder: 3,
                limits_um: Some(SOSS_ORDER3_UM),
                origin: LimitsOrigin::BuiltInDefault,
                rows: 5,
                n_used: 1081,
                first: [458.7474666555534, 454.5781619870285, 458.5917907113789],
                sum: 2293.894629276867,
            },
            Masked {
                relative: A4,
                detector: "MIRIMAGE",
                sporder: 1,
                limits_um: Some(MIRI_LRS_UM),
                origin: LimitsOrigin::FileLog,
                rows: 64,
                n_used: 247,
                first: [105.1244014781455, 106.12335012948235, 106.5334923923868],
                sum: 7420.285204061691,
            },
            Masked {
                relative: A6,
                detector: "NRS1",
                sporder: 0,
                limits_um: None,
                origin: LimitsOrigin::Open,
                rows: 65,
                n_used: 331,
                first: [0.01048560870365226, 0.010673391332145017, 0.010927166146553388],
                sum: 0.6909078090629617,
            },
            Masked {
                relative: B1,
                detector: "MIRIMAGE",
                sporder: 1,
                limits_um: Some(MIRI_LRS_UM),
                origin: LimitsOrigin::BuiltInDefault,
                rows: 10,
                n_used: 243,
                first: [2.163616608616668, 2.3930142098838867, 2.3525511707014934],
                sum: 23.347534963816415,
            },
            Masked {
                relative: E1,
                detector: "NRS1",
                sporder: 0,
                limits_um: Some([0.5530469695008425, 5.370352185327354]),
                origin: LimitsOrigin::Open,
                rows: 240,
                n_used: 316,
                first: [0.0914175844707027, 0.09153811187579863, 0.0914906749071388],
                sum: 21.09879206422516,
            },
        ];
        for expected in table {
            let label = format!("{} {} order {}", set_label(expected.relative), expected.detector, expected.sporder);
            let found = real_curves(expected.relative, expected.detector, expected.sporder, &req(CurveMode::Masked));
            let white = &found.white;
            assert_eq!(found.mode, CurveMode::Masked, "{label} mode");
            assert_eq!(white.limits_origin, expected.origin, "{label} limits origin, {CURVES_ORACLE}");
            match expected.limits_um {
                Some(limits_um) => assert_eq!(white.band_um, limits_um, "{label} masked limits, {CURVES_ORACLE}"),
                None => assert!(white.band_um[0] < white.band_um[1], "{label} band {:?}", white.band_um),
            }
            assert_eq!(found.row_index, (0..expected.rows).collect::<Vec<usize>>(), "{label} every row is kept, {CURVES_ORACLE}");
            assert_eq!(found.dropped, NO_ROW_DROPPED, "{label} dropped");
            assert_eq!(white.n_used, vec![expected.n_used; expected.rows], "{label} n_used, {CURVES_ORACLE}");
            assert_all_close(&white.flux[..3], &expected.first, &format!("{label} masked flux, {CURVES_ORACLE}"));
            assert_close(total(&white.flux), expected.sum, &format!("{label} masked fsum, {CURVES_ORACLE}"));
            assert!(found.rejected_elements.is_empty(), "{label} rejected elements");
            assert!(!found.warnings.iter().any(|warning| warning.starts_with("the pipeline sums order")), "{label} warnings {:?}", found.warnings);
            if expected.rows < 10 {
                let numbers: Vec<f64> = (1..=expected.rows).map(|number| number as f64).collect();
                assert_eq!(found.time_scale, TsoTimeScale::Index, "{label} time scale");
                assert_eq!(found.time, numbers, "{label} time is the integration number");
            }
            let valid_note = match expected.relative {
                E1 => Some("316 of 431 channels are valid in every integration"),
                A6 => Some("331 of 431 channels are valid in every integration"),
                _ => None,
            };
            if let Some(note) = valid_note {
                assert_eq!(found.notes, [note, FLUX_ERR_IS_THE_APPS], "{label} notes, {CURVES_ORACLE}");
            }
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/{A1,A3,A4,A5,A6,B1,C,E1}/*-o0*_x1dints.fits"]
    fn real_element_rejection_removes_the_one_element_that_dominates_two_nirspec_sums() {
        let a5 = real_curves(A5, "NRS1", -1, &rejecting(1000.0));
        assert_eq!(a5.rejected_elements.len(), 1, "A5 NRS1 rejected at 1000, {CURVES_ORACLE}: {:?}", a5.rejected_elements);
        assert_eq!(a5.rejected_elements[0].index, 230, "A5 NRS1 rejected index, {CURVES_ORACLE}");
        assert_eq!(a5.rejected_elements[0].wavelength_um, 3.0178602767154326, "A5 NRS1 rejected wavelength, {CURVES_ORACLE}");
        assert_eq!(
            a5.rejected_elements[0].reason,
            "time-median flux -6.109479e3 is 6.641e7 robust sigma from the median 1.273477e-3 of its 20 neighbours",
            "A5 NRS1 reason, {CURVES_ORACLE}"
        );
        assert_eq!(a5.white.n_used, vec![1238; 8], "A5 NRS1 n_used, {CURVES_ORACLE}");
        assert_all_close(
            &a5.white.flux[..3],
            &[1.9298996495576095, 1.9307528009564383, 1.9309120286351102],
            &format!("A5 NRS1 flux after rejection, {CURVES_ORACLE}"),
        );
        assert_close(total(&a5.white.flux), 15.42232155023074, &format!("A5 NRS1 fsum after rejection, {CURVES_ORACLE}"));
        assert_close(a5.white.flux_err[0], 0.0003412404398588, &format!("A5 NRS1 flux_err[0] after rejection, {CURVES_ORACLE}"));
        assert_eq!(a5.notes[1], "1 element(s) rejected at 1000 robust sigma", "A5 NRS1 notes");

        let c1 = real_curves(C1, "NRS1", -1, &rejecting(1000.0));
        assert_eq!(c1.rejected_elements.len(), 1, "C1 rejected at 1000, {CURVES_ORACLE}: {:?}", c1.rejected_elements);
        assert_eq!(c1.rejected_elements[0].index, 1289, "C1 rejected index, {CURVES_ORACLE}");
        assert_eq!(c1.rejected_elements[0].wavelength_um, 5.16590253862141, "C1 rejected wavelength, {CURVES_ORACLE}");
        assert_eq!(
            c1.rejected_elements[0].reason,
            "time-median flux -2.288517e2 is 1.477e6 robust sigma from the median 3.000157e-4 of its 14 neighbours",
            "C1 reason, {CURVES_ORACLE}"
        );
        assert_eq!(c1.white.n_used, vec![1275; 44], "C1 n_used, {CURVES_ORACLE}");
        assert_close(c1.white.flux[0], 0.3439771601428205, &format!("C1 flux[0] after rejection, {CURVES_ORACLE}"));
        assert_close(total(&c1.white.flux), 15.290835780580851, &format!("C1 fsum after rejection, {CURVES_ORACLE}"));

        for (relative, detector, sporder) in [(A5, "NRS2", -1), (A4, "MIRIMAGE", 1), (A6, "NRS1", 0), (E1, "NRS1", 0)] {
            let found = real_curves(relative, detector, sporder, &rejecting(1000.0));
            assert!(found.rejected_elements.is_empty(), "{relative} {detector} at 1000, {CURVES_ORACLE}: {:?}", found.rejected_elements);
            assert!(
                found.notes.iter().any(|note| note == "0 element(s) rejected at 1000 robust sigma"),
                "{relative} {detector} notes {:?}",
                found.notes
            );
        }

        for (relative, detector, sporder) in REAL_SERIES {
            let found = real_curves(relative, detector, sporder, &rejecting(100.0));
            println!("{} {detector} order {sporder}: {} element(s) rejected at 100 robust sigma", set_label(relative), found.rejected_elements.len());
            let expected: &[usize] = match (relative, detector) {
                (A5, "NRS1") => &[230],
                (C1, _) => &[1289],
                _ => &[],
            };
            assert_eq!(rejected_indices(&found), expected, "{relative} {detector} order {sporder} rejected at 100, tso_checks.py curves <data root>/tso 100");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/{A4,A5,A6,B1,C,E1}/*-o0*_x1dints.fits"]
    fn real_bins_and_the_pixel_loss_warning_on_the_transit_file() {
        let fixtures = "plan/pacotes/TSO-support/fixtures/tso_quicklook_e1_bins_flux.json and tso_quicklook_e1_bins_err.json";
        let lost_in_the_transit = "mean NPIXELS of the summed elements falls below 0.9 of its median in 10 integration(s) (INT_NUM 181 to 190): the extraction lost pixels there, so a depth measured over them is biased";
        let set = real_set(E1);
        let series = &set.series[0];
        let with_bins = |mode| CurveRequest { bins: E1_BINS.to_vec(), ..req(mode) };

        let started = Instant::now();
        let pipeline = light_curves(&set, series, &with_bins(CurveMode::Pipeline)).unwrap();
        println!("E1, pipeline mode, white light and five bins: {:?}", started.elapsed());
        let started = Instant::now();
        let masked = light_curves(&set, series, &with_bins(CurveMode::Masked)).unwrap();
        println!("E1, masked mode, white light and five bins: {:?}", started.elapsed());

        let n_used = [[10, 48], [17, 54], [59, 64], [88, 88], [153, 153]];
        let first = [0.03868635075864136, 0.04492501912588164, 0.02791344398083556, 0.02393669652320723, 0.028238000093070058];
        let at_the_minimum =
            [0.00012701102220275538, 9.029969816949838e-05, 0.0012682855763235515, 0.0006150225726303651, 0.0030259117741344996];
        let sums = [8.936247155521125, 10.327731252074408, 6.44690839365677, 5.482437700111454, 6.568168892743402];
        let first_errors =
            [3.617495732070968e-05, 2.843227550354594e-05, 2.756775301733895e-05, 3.404713975394668e-05, 5.489838635764687e-05];
        assert_eq!(pipeline.bins.len(), 5, "E1 bins");
        for (index, bin) in pipeline.bins.iter().enumerate() {
            let label = format!("E1 pipeline bin {}", index + 1);
            assert_eq!(bin.band_um, E1_BINS[index], "{label} band");
            assert_eq!(bin.limits_origin, LimitsOrigin::User, "{label} origin");
            assert_eq!(bin.flux.len(), 240, "{label} rows, {fixtures}");
            assert_eq!(used_range(bin), n_used[index], "{label} n_used, {CURVES_ORACLE}");
            assert_close(bin.flux[0], first[index], &format!("{label} flux[0], {fixtures}"));
            assert_close(bin.flux[185], at_the_minimum[index], &format!("{label} flux[185], {fixtures}"));
            assert_close(total(&bin.flux), sums[index], &format!("{label} sum, {fixtures}"));
            assert_close(bin.flux_err[0], first_errors[index], &format!("{label} flux_err[0], {fixtures}"));
        }
        assert_eq!(pipeline.int_num[185], 186, "E1 INT_NUM of the minimum, {CURVES_ORACLE}");
        assert_close(pipeline.white.flux[185], 0.00512653064346067, &format!("E1 white flux[185], {CURVES_ORACLE}"));
        assert_eq!(pipeline.white.n_used[185], 327, "E1 white n_used[185], {CURVES_ORACLE}");
        assert_close(pipeline.white.npixels_mean[185], 2.957883175937349, &format!("E1 white npixels_mean[185], {CURVES_ORACLE}"));
        assert_eq!(pipeline.warnings, [lost_in_the_transit], "E1 pipeline warnings, {CURVES_ORACLE}");

        let masked_used = [7, 13, 55, 88, 153];
        let masked_first = [0.0043228321244751485, 0.0115525719469062, 0.023367483783044055, 0.02393669652320723, 0.028238000093070058];
        for (index, bin) in masked.bins.iter().enumerate() {
            let label = format!("E1 masked bin {}", index + 1);
            assert_eq!(bin.n_used, vec![masked_used[index]; 240], "{label} n_used, {SETS_ORACLE}");
            assert_close(bin.flux[0], masked_first[index], &format!("{label} flux[0], {SETS_ORACLE}"));
        }
        assert_eq!(masked.warnings, [lost_in_the_transit], "E1 masked warnings, {CURVES_ORACLE}");

        let a6 = real_curves(A6, "NRS1", 0, &req(CurveMode::Pipeline));
        assert_eq!(
            a6.warnings,
            ["mean NPIXELS of the summed elements falls below 0.9 of its median in 3 integration(s) (INT_NUM 44 to 46): the extraction lost pixels there, so a depth measured over them is biased"],
            "A6 pipeline warnings, {CURVES_ORACLE}"
        );
        for (relative, detector, sporder) in [(A4, "MIRIMAGE", 1), (A5, "NRS1", -1), (A5, "NRS2", -1), (B1, "MIRIMAGE", 1), (C1, "NRS1", -1)] {
            let found = real_curves(relative, detector, sporder, &req(CurveMode::Pipeline));
            assert_eq!(found.warnings, NO_TEXT, "{relative} {detector} pipeline warnings, {CURVES_ORACLE}");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1/*-o030*_x1dints.fits"]
    fn real_user_limits_and_exclusion_ranges_change_the_channels_summed() {
        let limits = |mode| CurveRequest { wavelength_min_um: Some(1.0), wavelength_max_um: Some(5.0), ..req(mode) };

        let pipeline = real_curves(E1, "NRS1", 0, &limits(CurveMode::Pipeline));
        assert_eq!(used_range(&pipeline.white), [277, 319], "E1 pipeline, limits 1.0 to 5.0: n_used, {SETS_ORACLE}");
        assert_close(pipeline.white.flux[0], 0.11841969398180228, &format!("E1 pipeline, limits 1.0 to 5.0: flux[0], {SETS_ORACLE}"));
        assert_close(total(&pipeline.white.flux), 27.283294890668902, &format!("E1 pipeline, limits 1.0 to 5.0: fsum, {SETS_ORACLE}"));
        assert_eq!(pipeline.white.limits_origin, LimitsOrigin::User, "E1 pipeline, limits 1.0 to 5.0: origin");
        assert_eq!(pipeline.white.band_um, [1.0, 5.0], "E1 pipeline, limits 1.0 to 5.0: band");

        let excluding = CurveRequest { exclude_um: vec![[2.5, 3.0]], ..limits(CurveMode::Masked) };
        let masked = real_curves(E1, "NRS1", 0, &excluding);
        let what = "E1 masked, limits 1.0 to 5.0, exclusion 2.5 to 3.0";
        assert_eq!(masked.white.n_used, vec![236; 240], "{what}: n_used, {SETS_ORACLE}");
        assert_close(masked.white.flux[0], 0.06787184644289544, &format!("{what}: flux[0], {SETS_ORACLE}"));
        assert_close(total(&masked.white.flux), 15.655458621179339, &format!("{what}: fsum, {SETS_ORACLE}"));
        assert_eq!(masked.white.limits_origin, LimitsOrigin::User, "{what}: origin");
        assert_eq!(masked.white.band_um, [1.0, 5.0], "{what}: band");
        assert_eq!(masked.notes[0], "316 of 431 channels are valid in every integration", "{what}: notes");
        assert!(masked.notes[1].ends_with("channel(s) removed by the exclusion ranges"), "{what}: notes {:?}", masked.notes);
    }
}
