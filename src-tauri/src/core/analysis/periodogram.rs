use std::f64::consts::PI;

use anyhow::{bail, Result};
use rayon::prelude::*;
use serde::Serialize;

pub const MIN_PERIODOGRAM_POINTS: usize = 4;
pub const MAX_PERIODOGRAM_POINTS: usize = 200_000;
pub const MAX_PERIODOGRAM_FREQUENCIES: usize = 200_000;
pub const MAX_PERIODOGRAM_WORK: u64 = 2_000_000_000;
pub const DEFAULT_SAMPLES_PER_PEAK: f64 = 5.0;
pub const NYQUIST_FACTOR: f64 = 5.0;
pub const REGULAR_CADENCE_TOLERANCE: f64 = 0.1;
pub const REGULAR_CADENCE_SHARE: f64 = 0.5;

const MIN_SAMPLES_PER_PEAK: f64 = 1.0;
const MAX_SAMPLES_PER_PEAK: f64 = 100.0;
const GRID_COUNT_SLACK: f64 = 1e-9;
const DEGENERATE_TERM: f64 = 1e-12;
const NORMALIZATION_STANDARD: &str = "standard";
const FAP_METHOD_BALUEV: &str = "baluev";
const CONSTANT_VALUES: &str = "the values are constant: there is nothing to search";

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PeriodogramOptions {
    pub min_frequency: Option<f64>,
    pub max_frequency: Option<f64>,
    pub samples_per_peak: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrequencyGrid {
    pub first: f64,
    pub step: f64,
    pub count: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpperLimit {
    Given,
    CadenceNyquist,
    FiveMeanNyquist,
}

#[derive(Debug, Clone, Serialize)]
pub struct Periodogram {
    pub frequency: Vec<f64>,
    pub power: Vec<f64>,
    pub best_frequency: f64,
    pub best_period: f64,
    pub best_power: f64,
    pub n_points: usize,
    pub n_dropped: usize,
    pub n_frequencies: usize,
    pub baseline_days: f64,
    pub frequency_step: f64,
    pub first_frequency: f64,
    pub last_frequency: f64,
    pub upper_limit: UpperLimit,
    pub cadence_days: Option<f64>,
    pub normalization: String,
    pub weighted: bool,
    pub false_alarm_probability: Option<f64>,
    pub fap_method: String,
    pub notes: Vec<String>,
}

struct WeightedSeries<'a> {
    elapsed: &'a [f64],
    centred: &'a [f64],
    weight: &'a [f64],
    mean: f64,
    variance: f64,
}

impl WeightedSeries<'_> {
    fn power_at(&self, frequency: f64) -> f64 {
        let angular = 2.0 * PI * frequency;
        let (mut c, mut s, mut yc, mut ys, mut cc, mut ss, mut cs) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for ((t, y), w) in self.elapsed.iter().zip(self.centred).zip(self.weight) {
            let (sine, cosine) = (angular * t).sin_cos();
            let weighted_cosine = w * cosine;
            let weighted_sine = w * sine;
            c += weighted_cosine;
            s += weighted_sine;
            yc += weighted_cosine * y;
            ys += weighted_sine * y;
            cc += weighted_cosine * cosine;
            ss += weighted_sine * sine;
            cs += weighted_cosine * sine;
        }
        let yy = self.variance;
        let yc = yc - self.mean * c;
        let ys = ys - self.mean * s;
        let cc = cc - c * c;
        let ss = ss - s * s;
        let cs = cs - c * s;
        let cosine_alone = yc * yc / (yy * cc);
        let power = if cc < DEGENERATE_TERM && ss < DEGENERATE_TERM {
            0.0
        } else if ss < DEGENERATE_TERM {
            cosine_alone
        } else if cc < DEGENERATE_TERM {
            ys * ys / (yy * ss)
        } else {
            let determinant = cc * ss - cs * cs;
            if determinant < DEGENERATE_TERM * cc * ss {
                cosine_alone
            } else {
                (ss * yc * yc + cc * ys * ys - 2.0 * cs * yc * ys) / (yy * determinant)
            }
        };
        if power > 0.0 {
            power.min(1.0)
        } else {
            0.0
        }
    }
}

fn check_lengths(time: &[f64], value: &[f64], err: Option<&[f64]>) -> Result<()> {
    if time.len() != value.len() {
        bail!("time has {} values and value has {}: they must have the same length", time.len(), value.len());
    }
    if let Some(err) = err {
        if err.len() != time.len() {
            bail!("err has {} values for {} points", err.len(), time.len());
        }
    }
    Ok(())
}

fn check_point_count(points: usize) -> Result<()> {
    if points < MIN_PERIODOGRAM_POINTS {
        bail!("a period search needs at least {MIN_PERIODOGRAM_POINTS} points with a finite time and value, got {points}");
    }
    Ok(())
}

fn check_values_vary(value: &[f64]) -> Result<()> {
    if value.iter().all(|v| *v == value[0]) {
        bail!(CONSTANT_VALUES);
    }
    Ok(())
}

fn is_usable_error(err: f64) -> bool {
    err.is_finite() && err > 0.0
}

fn normalised_weights(points: usize, err: Option<&[f64]>) -> Vec<f64> {
    match err {
        Some(err) => {
            let inverse_variance: Vec<f64> = err.iter().map(|e| 1.0 / (e * e)).collect();
            let total: f64 = inverse_variance.iter().sum();
            inverse_variance.iter().map(|w| w / total).collect()
        }
        None => vec![1.0 / points as f64; points],
    }
}

fn times_from_the_earliest(time: &[f64]) -> Vec<f64> {
    let earliest = time.iter().copied().fold(f64::INFINITY, f64::min);
    time.iter().map(|t| t - earliest).collect()
}

fn weighted_mean(weight: &[f64], value: &[f64]) -> f64 {
    weight.iter().zip(value).map(|(w, v)| w * v).sum()
}

fn weighted_variance(weight: &[f64], value: &[f64], mean: f64) -> f64 {
    weight.iter().zip(value).map(|(w, v)| w * v * v).sum::<f64>() - mean * mean
}

fn index_of_highest(power: &[f64]) -> usize {
    (1..power.len()).fold(0, |best, k| if power[k] > power[best] { k } else { best })
}

pub fn gls_power(time: &[f64], value: &[f64], err: Option<&[f64]>, frequency: &[f64]) -> Result<Vec<f64>> {
    check_lengths(time, value, err)?;
    let points = time.len();
    let unusable = (0..points).filter(|&i| !(time[i].is_finite() && value[i].is_finite())).count();
    if unusable > 0 {
        bail!("{unusable} of {points} points have no finite time or value");
    }
    if let Some(err) = err {
        let unusable = err.iter().filter(|e| !is_usable_error(**e)).count();
        if unusable > 0 {
            bail!("{unusable} of {points} points have no positive finite error");
        }
    }
    check_point_count(points)?;
    check_values_vary(value)?;
    let weight = normalised_weights(points, err);
    let elapsed = times_from_the_earliest(time);
    let level = weighted_mean(&weight, value);
    let centred: Vec<f64> = value.iter().map(|v| v - level).collect();
    let mean = weighted_mean(&weight, &centred);
    let variance = weighted_variance(&weight, &centred, mean);
    if !(variance > 0.0) {
        bail!(CONSTANT_VALUES);
    }
    let series = WeightedSeries { elapsed: &elapsed, centred: &centred, weight: &weight, mean, variance };
    Ok(frequency.par_iter().map(|f| series.power_at(*f)).collect())
}

pub fn regular_cadence(time: &[f64]) -> Option<f64> {
    let mut sorted: Vec<f64> = time.iter().copied().filter(|t| t.is_finite()).collect();
    sorted.sort_by(f64::total_cmp);
    let mut spacings: Vec<f64> = sorted
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|spacing| *spacing > 0.0)
        .collect();
    if spacings.is_empty() {
        return None;
    }
    spacings.sort_by(f64::total_cmp);
    let cadence = spacings[spacings.len() / 2];
    let on_cadence = spacings
        .iter()
        .filter(|spacing| {
            let multiple = (*spacing / cadence).round().max(1.0);
            (*spacing - multiple * cadence).abs() <= REGULAR_CADENCE_TOLERANCE * cadence
        })
        .count();
    (on_cadence as f64 >= REGULAR_CADENCE_SHARE * spacings.len() as f64).then_some(cadence)
}

pub fn frequency_grid(n_points: usize, baseline_days: f64, options: &PeriodogramOptions) -> Result<FrequencyGrid> {
    let samples_per_peak = options.samples_per_peak.unwrap_or(DEFAULT_SAMPLES_PER_PEAK);
    if !(MIN_SAMPLES_PER_PEAK..=MAX_SAMPLES_PER_PEAK).contains(&samples_per_peak) {
        bail!("samplesPerPeak {samples_per_peak} must be between {MIN_SAMPLES_PER_PEAK} and {MAX_SAMPLES_PER_PEAK}");
    }
    if let Some(min_frequency) = options.min_frequency {
        if !(min_frequency.is_finite() && min_frequency > 0.0) {
            bail!("minFrequency {min_frequency} must be a positive number of cycles per day");
        }
    }
    let step = 1.0 / (samples_per_peak * baseline_days);
    let first = options.min_frequency.unwrap_or(step);
    let last_wanted = options.max_frequency.unwrap_or(NYQUIST_FACTOR * 0.5 * n_points as f64 / baseline_days);
    if !(last_wanted.is_finite() && last_wanted > first) {
        bail!("maxFrequency {last_wanted} must be above minFrequency {first}");
    }
    let wanted = ((last_wanted - first) / step + GRID_COUNT_SLACK).floor() + 1.0;
    let limit = MAX_PERIODOGRAM_FREQUENCIES.min((MAX_PERIODOGRAM_WORK / n_points.max(1) as u64).max(1) as usize);
    if wanted > limit as f64 {
        let widened = (last_wanted - first) / (limit - 1) as f64;
        let note = format!(
            "frequency grid capped at {limit} samples: step widened from {step:.6e} to {widened:.6e} cycles per day"
        );
        return Ok(FrequencyGrid { first, step: widened, count: limit, notes: vec![note] });
    }
    Ok(FrequencyGrid { first, step, count: wanted as usize, notes: Vec::new() })
}

fn gamma_ratio(order: usize) -> f64 {
    (2..order).fold(1.0 / PI.sqrt(), |ratio, m| (m as f64 - 1.0) / (2.0 * ratio))
}

pub fn false_alarm_probability_baluev(power: f64, max_frequency: f64, time: &[f64], err: Option<&[f64]>) -> Option<f64> {
    let points = time.len();
    if points < MIN_PERIODOGRAM_POINTS || !(0.0..=1.0).contains(&power) || !(max_frequency.is_finite() && max_frequency > 0.0) {
        return None;
    }
    if err.is_some_and(|err| err.len() != points) {
        return None;
    }
    let weight = normalised_weights(points, err);
    let elapsed = times_from_the_earliest(time);
    let time_variance = weighted_variance(&weight, &elapsed, weighted_mean(&weight, &elapsed));
    if !(time_variance.is_finite() && time_variance > 0.0) {
        return None;
    }
    let freedom_of_the_mean_model = (points - 1) as f64;
    let freedom_of_the_sinusoid_model = (points - 3) as f64;
    let unexplained = 1.0 - power;
    let single_frequency = unexplained.powf(freedom_of_the_sinusoid_model / 2.0);
    let effective_width = max_frequency * (4.0 * PI * time_variance).sqrt();
    let gamma = (2.0 / freedom_of_the_mean_model).sqrt() * gamma_ratio(points - 1);
    let tau = gamma
        * effective_width
        * unexplained.powf((freedom_of_the_sinusoid_model - 1.0) / 2.0)
        * (freedom_of_the_mean_model * power / 2.0).sqrt();
    Some((-(-tau).exp_m1() + single_frequency * (-tau).exp()).clamp(0.0, 1.0))
}

pub fn decimate_peaks(frequency: &[f64], power: &[f64], max_samples: usize) -> (Vec<f64>, Vec<f64>) {
    if max_samples == 0 || frequency.len() <= max_samples {
        return (frequency.to_vec(), power.to_vec());
    }
    let chunk = frequency.len().div_ceil(max_samples);
    frequency
        .chunks(chunk)
        .zip(power.chunks(chunk))
        .map(|(frequencies, powers)| {
            let highest = index_of_highest(powers);
            (frequencies[highest], powers[highest])
        })
        .unzip()
}

pub fn periodogram(time: &[f64], value: &[f64], err: Option<&[f64]>, options: &PeriodogramOptions) -> Result<Periodogram> {
    check_lengths(time, value, err)?;
    let kept: Vec<usize> = (0..time.len()).filter(|&i| time[i].is_finite() && value[i].is_finite()).collect();
    let n_points = kept.len();
    let n_dropped = time.len() - n_points;
    let mut notes = Vec::new();
    if n_dropped > 0 {
        notes.push(format!("{n_dropped} points without a finite time or value were left out"));
    }
    let kept_time: Vec<f64> = kept.iter().map(|&i| time[i]).collect();
    let kept_value: Vec<f64> = kept.iter().map(|&i| value[i]).collect();
    let kept_err: Option<Vec<f64>> = err.map(|err| kept.iter().map(|&i| err[i]).collect());
    let unusable_errors = kept_err.as_ref().map(|err| err.iter().filter(|e| !is_usable_error(**e)).count());
    let weighted = unusable_errors == Some(0);
    if let Some(unusable) = unusable_errors.filter(|count| *count > 0) {
        notes.push(format!(
            "errors not used: {unusable} of {n_points} points have no positive finite error; every point has the same weight"
        ));
    }
    check_point_count(n_points)?;
    if n_points > MAX_PERIODOGRAM_POINTS {
        bail!("{n_points} points exceed the limit of {MAX_PERIODOGRAM_POINTS}");
    }
    let earliest = kept_time.iter().copied().fold(f64::INFINITY, f64::min);
    let latest = kept_time.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let baseline_days = latest - earliest;
    if !(baseline_days > 0.0) {
        bail!("every point has the same time: no period can be searched");
    }
    check_values_vary(&kept_value)?;

    let cadence = regular_cadence(&kept_time);
    let nyquist = cadence.map(|cadence| 0.5 / cadence);
    let cadence_limit =
        nyquist.filter(|limit| options.max_frequency.is_none() && options.min_frequency.is_none_or(|first| first < *limit));
    let upper_limit = match (options.max_frequency, cadence_limit) {
        (Some(_), _) => UpperLimit::Given,
        (None, Some(_)) => UpperLimit::CadenceNyquist,
        (None, None) => UpperLimit::FiveMeanNyquist,
    };
    let grid_options = PeriodogramOptions { max_frequency: options.max_frequency.or(cadence_limit), ..*options };
    let grid = frequency_grid(n_points, baseline_days, &grid_options)?;
    let frequency: Vec<f64> = (0..grid.count).map(|k| grid.first + k as f64 * grid.step).collect();
    let used_err = kept_err.as_deref().filter(|_| weighted);
    let power = gls_power(&kept_time, &kept_value, used_err, &frequency)?;
    let best = index_of_highest(&power);
    let last_frequency = frequency[grid.count - 1];
    let false_alarm_probability = false_alarm_probability_baluev(power[best], last_frequency, &kept_time, used_err);

    notes.extend(grid.notes);
    notes.push("generalised Lomb-Scargle with a floating mean (Zechmeister and Kurster 2009), standard normalisation".to_string());
    notes.push(if weighted { "weights 1 / err^2" } else { "equal weights" }.to_string());
    notes.push(format!(
        "{} frequencies from {:.6e} to {last_frequency:.6e} cycles per day, step {:.6e}",
        grid.count, grid.first, grid.step
    ));
    if let (Some(cadence), Some(nyquist)) = (cadence, nyquist) {
        if upper_limit == UpperLimit::CadenceNyquist {
            notes.push(format!(
                "automatic upper limit {nyquist:.6e} cycles per day: the Nyquist frequency of the regular cadence of {cadence:.6e} d; above it the power repeats aliases of the sampling"
            ));
        } else if last_frequency > nyquist {
            notes.push(format!(
                "above {nyquist:.6e} cycles per day, the Nyquist frequency of the regular cadence of {cadence:.6e} d, the power repeats aliases of the sampling: a peak there can be an alias"
            ));
        }
    }
    notes.push("false-alarm probability: Baluev 2008 upper limit for the highest peak, valid for white Gaussian noise".to_string());
    notes.push("the search assumes a sinusoidal signal: a box-shaped transit shows at its period and at its harmonics".to_string());

    Ok(Periodogram {
        best_frequency: frequency[best],
        best_period: 1.0 / frequency[best],
        best_power: power[best],
        frequency,
        power,
        n_points,
        n_dropped,
        n_frequencies: grid.count,
        baseline_days,
        frequency_step: grid.step,
        first_frequency: grid.first,
        last_frequency,
        upper_limit,
        cadence_days: cadence,
        normalization: NORMALIZATION_STANDARD.to_string(),
        weighted,
        false_alarm_probability,
        fap_method: FAP_METHOD_BALUEV.to_string(),
        notes,
    })
}

#[cfg(test)]
pub(crate) mod test_series {
    use std::f64::consts::PI;

    pub(crate) const SERIES_A_POINTS: usize = 40;
    pub(crate) const SERIES_B_POINTS: usize = 24;

    pub(crate) fn series_a() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let time: Vec<f64> = (0..SERIES_A_POINTS).map(|i| 0.37 * i as f64 + 0.11 * (1.3 * i as f64).sin()).collect();
        let value = time
            .iter()
            .map(|t| 1.5 + 0.8 * (2.0 * PI * t / 2.9 + 0.4).sin() + 0.3 * (2.0 * PI * t / 1.1).cos())
            .collect();
        let err = (0..SERIES_A_POINTS).map(|i| 0.1 + 0.0125 * ((7 * i) % 5) as f64).collect();
        (time, value, err)
    }

    pub(crate) fn uniform_deviates(count: usize) -> Vec<f64> {
        let mut state: u64 = 12345;
        (0..count)
            .map(|_| {
                state = (1_103_515_245 * state + 12345) % 2_147_483_648;
                state as f64 / 2_147_483_648.0
            })
            .collect()
    }

    pub(crate) fn series_b() -> (Vec<f64>, Vec<f64>) {
        let u = uniform_deviates(2 * SERIES_B_POINTS);
        let time: Vec<f64> = (0..SERIES_B_POINTS).map(|i| 0.5 * i as f64 + 0.4 * u[i]).collect();
        let value = (0..SERIES_B_POINTS)
            .map(|i| 10.0 + 0.3 * (2.0 * PI * time[i] / 3.3).sin() + (u[i + SERIES_B_POINTS] - 0.5))
            .collect();
        (time, value)
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;
    use std::time::Instant;

    use super::test_series::{series_a, series_b, uniform_deviates};
    use super::*;

    const BASELINE_A: f64 = 14.476307010875704;
    const BASELINE_B: f64 = 11.629745406471192;
    const STEP_A: f64 = 0.013815678256184037;
    const LAST_FREQUENCY_A: f64 = 6.907839128092019;
    const SIX_FREQUENCIES: [f64; 6] = [1.0 / 2.9, 1.0 / 1.1, 0.05, 0.5, 1.0, 1.3];
    const ASTROPY_WEIGHTED_A: [f64; 6] = [
        0.8688332422596348,
        0.10318022493408642,
        0.010182006429847117,
        0.005746430698081302,
        0.002044947787498418,
        0.0021256665116510163,
    ];
    const ASTROPY_UNWEIGHTED_A: [f64; 6] = [
        0.8748676119553445,
        0.13429358835103514,
        0.011095910687627148,
        0.008088855434573205,
        0.002554239385808906,
        0.001548489166391813,
    ];
    const FOUR_FREQUENCIES_B: [f64; 4] = [1.0 / 3.3, 0.1, 0.7, 1.5];
    const ASTROPY_UNWEIGHTED_B: [f64; 4] =
        [0.3974292683198885, 0.1003133005764542, 0.13641195068561582, 0.16502222865826538];
    const METHOD_NOTE: &str = "generalised Lomb-Scargle with a floating mean (Zechmeister and Kurster 2009), standard normalisation";
    const FAP_NOTE: &str = "false-alarm probability: Baluev 2008 upper limit for the highest peak, valid for white Gaussian noise";
    const SINUSOID_NOTE: &str = "the search assumes a sinusoidal signal: a box-shaped transit shows at its period and at its harmonics";

    fn assert_close(got: f64, want: f64, tolerance: f64, what: &str) {
        assert!((got - want).abs() <= tolerance, "{what}: got {got:e}, want {want:e}, off by {:e}", (got - want).abs());
    }

    fn assert_all_close(got: &[f64], want: &[f64], tolerance: f64, what: &str) -> f64 {
        assert_eq!(got.len(), want.len(), "{what}");
        let mut largest = 0.0_f64;
        for (index, (g, w)) in got.iter().zip(want).enumerate() {
            assert_close(*g, *w, tolerance, &format!("{what}[{index}]"));
            largest = largest.max((g - w).abs());
        }
        largest
    }

    fn refusal(result: Result<Periodogram>) -> String {
        format!("{:#}", result.unwrap_err())
    }

    #[test]
    fn the_two_test_series_match_their_checkpoints() {
        let (time, value, err) = series_a();
        assert_eq!(time.len(), 40);
        assert_close(time[1], 0.4759914003958912, 1e-12, "A t[1]");
        assert_close(time[39], 14.476307010875704, 1e-12, "A t[39]");
        assert_close(value[0], 2.1115346738469203, 1e-12, "A y[0]");
        assert_close(value[39], 1.9336194009391554, 1e-12, "A y[39]");
        assert_all_close(&err[0..4], &[0.1, 0.125, 0.15, 0.1125], 1e-12, "A dy");

        let u = uniform_deviates(3);
        assert_all_close(&u, &[0.6551540484651923, 0.30481432331725955, 0.6749606337398291], 1e-12, "u");
        let (time, value) = series_b();
        assert_eq!(time.len(), 24);
        assert_close(time[0], 0.2620616193860769, 1e-12, "B t[0]");
        assert_close(time[23], 11.89180702585727, 1e-12, "B t[23]");
        assert_close(value[0], 9.972576568084936, 1e-12, "B y[0]");
        assert_close(value[23], 9.867329445865252, 1e-12, "B y[23]");
    }

    #[test]
    fn weighted_power_matches_astropy_at_six_frequencies() {
        let (time, value, err) = series_a();
        let power = gls_power(&time, &value, Some(&err), &SIX_FREQUENCIES).unwrap();
        let largest = assert_all_close(&power, &ASTROPY_WEIGHTED_A, 1e-9, "weighted power of series A");
        println!("weighted power, series A: largest difference from astropy {largest:.3e}");
    }

    #[test]
    fn unweighted_power_matches_astropy_at_six_frequencies() {
        let (time, value, _) = series_a();
        let power = gls_power(&time, &value, None, &SIX_FREQUENCIES).unwrap();
        let largest_a = assert_all_close(&power, &ASTROPY_UNWEIGHTED_A, 1e-9, "unweighted power of series A");
        let (time, value) = series_b();
        let power = gls_power(&time, &value, None, &FOUR_FREQUENCIES_B).unwrap();
        let largest_b = assert_all_close(&power, &ASTROPY_UNWEIGHTED_B, 1e-9, "unweighted power of series B");
        println!("unweighted power: largest difference from astropy {largest_a:.3e} (series A), {largest_b:.3e} (series B)");
    }

    #[test]
    fn the_default_grid_has_five_samples_per_peak_up_to_five_times_the_mean_nyquist_frequency() {
        let grid = frequency_grid(40, BASELINE_A, &PeriodogramOptions::default()).unwrap();
        assert_eq!(grid.count, 500);
        assert_close(grid.first, STEP_A, 1e-15, "first");
        assert_close(grid.step, STEP_A, 1e-15, "step");
        assert_close(grid.first + 499.0 * grid.step, LAST_FREQUENCY_A, 1e-12, "last");
        assert!(grid.notes.is_empty(), "{:?}", grid.notes);

        let grid = frequency_grid(24, BASELINE_B, &PeriodogramOptions::default()).unwrap();
        assert_eq!(grid.count, 300);
        assert_close(grid.step, 0.017197281024631295, 1e-15, "step of series B");

        let options = PeriodogramOptions { min_frequency: Some(0.2), max_frequency: Some(0.5), samples_per_peak: None };
        let grid = frequency_grid(40, BASELINE_A, &options).unwrap();
        assert_eq!(grid.count, 22);
        assert_eq!(grid.first, 0.2);
        assert_close(grid.first + 21.0 * grid.step, 0.4901292433798648, 1e-12, "last of the user range");
    }

    #[test]
    fn the_default_search_finds_the_injected_period_of_series_a() {
        let (time, value, err) = series_a();
        let weighted = periodogram(&time, &value, Some(&err), &PeriodogramOptions::default()).unwrap();
        assert_eq!(weighted.n_points, 40);
        assert_eq!(weighted.n_dropped, 0);
        assert_eq!(weighted.n_frequencies, 500);
        assert_eq!(weighted.frequency.len(), 500);
        assert_eq!(weighted.power.len(), 500);
        assert!(weighted.weighted);
        assert_close(weighted.baseline_days, BASELINE_A, 1e-12, "baseline");
        assert_close(weighted.frequency_step, STEP_A, 1e-15, "frequency step");
        assert_close(weighted.best_frequency, 0.3453919564046009, 1e-12, "best frequency");
        assert_eq!(weighted.best_frequency, weighted.frequency[24]);
        assert_close(weighted.best_period, 2.895261402175141, 1e-11, "best period");
        assert_close(weighted.best_power, 0.8675526073476112, 1e-9, "best power");
        assert_eq!(weighted.best_power, weighted.power[24]);
        let fap = weighted.false_alarm_probability.unwrap();
        assert!((fap / 6.64610294915559e-14 - 1.0).abs() < 1e-6, "{fap:e}");
        assert_eq!(weighted.normalization, "standard");
        assert_eq!(weighted.fap_method, "baluev");
        assert_close(weighted.power[0], 0.02649252643625369, 1e-9, "power[0]");
        assert_close(weighted.power[499], 0.009791246122316171, 1e-9, "power[499]");
        assert_eq!(
            weighted.notes,
            vec![
                METHOD_NOTE.to_string(),
                "weights 1 / err^2".to_string(),
                "500 frequencies from 1.381568e-2 to 6.907839e0 cycles per day, step 1.381568e-2".to_string(),
                FAP_NOTE.to_string(),
                SINUSOID_NOTE.to_string(),
            ]
        );
        println!(
            "default search, series A weighted: best power off astropy by {:.3e}, FAP relative difference {:.3e}",
            (weighted.best_power - 0.8675526073476112).abs(),
            (fap / 6.64610294915559e-14 - 1.0).abs()
        );

        let equal = periodogram(&time, &value, None, &PeriodogramOptions::default()).unwrap();
        assert!(!equal.weighted);
        assert_close(equal.best_power, 0.8743691833853808, 1e-9, "unweighted best power");
        let fap = equal.false_alarm_probability.unwrap();
        assert!((fap / 2.573526383531307e-14 - 1.0).abs() < 1e-6, "{fap:e}");
        assert_close(equal.power[499], 0.028822675340794753, 1e-9, "unweighted power[499]");
        assert_eq!(equal.notes[1], "equal weights");
        println!(
            "default search, series A unweighted: best power off astropy by {:.3e}, FAP relative difference {:.3e}",
            (equal.best_power - 0.8743691833853808).abs(),
            (fap / 2.573526383531307e-14 - 1.0).abs()
        );
    }

    const WHITE_LIGHT_CADENCE: f64 = 0.0009353528221254237;
    const WHITE_LIGHT_POINTS: usize = 366;

    fn regular_transit_series() -> (Vec<f64>, Vec<f64>) {
        let noise = uniform_deviates(WHITE_LIGHT_POINTS);
        let time: Vec<f64> = (0..WHITE_LIGHT_POINTS)
            .map(|i| 59782.83 + WHITE_LIGHT_CADENCE * i as f64)
            .collect();
        let middle = time[WHITE_LIGHT_POINTS / 2];
        let value = time
            .iter()
            .zip(&noise)
            .map(|(t, u)| {
                let dip = if (t - middle).abs() < 0.06 { 0.021 } else { 0.0 };
                1.0 - dip + 0.0004 * (u - 0.5)
            })
            .collect();
        (time, value)
    }

    #[test]
    fn the_default_search_of_a_regular_cadence_stops_at_its_nyquist_frequency() {
        let (time, value) = regular_transit_series();
        let nyquist = 0.5 / WHITE_LIGHT_CADENCE;
        let search = periodogram(&time, &value, None, &PeriodogramOptions::default()).unwrap();
        let last = *search.frequency.last().unwrap();
        assert!(last <= nyquist * (1.0 + 1e-9), "last frequency {last} above the Nyquist frequency {nyquist}");
        assert!(last > nyquist - search.frequency_step, "last frequency {last} more than one step below {nyquist}");
        assert!(search.best_frequency <= nyquist, "best frequency {} is an alias of the cadence", search.best_frequency);
        assert_eq!(search.last_frequency, last);
        assert_eq!(search.first_frequency, search.frequency[0]);
        assert_eq!(search.upper_limit, UpperLimit::CadenceNyquist);
        assert_close(search.cadence_days.unwrap(), WHITE_LIGHT_CADENCE, 1e-12, "cadence");
        assert_eq!(
            search.notes[3],
            "automatic upper limit 5.345576e2 cycles per day: the Nyquist frequency of the regular cadence of 9.353528e-4 d; above it the power repeats aliases of the sampling"
        );
        assert_eq!(search.notes.len(), 6);
        println!(
            "regular cadence, automatic grid: {} frequencies up to {last:.4} per day, best period {:.6} d, FAP {:?}",
            search.n_frequencies, search.best_period, search.false_alarm_probability
        );

        let wide = PeriodogramOptions { max_frequency: Some(2680.0), ..PeriodogramOptions::default() };
        let given = periodogram(&time, &value, None, &wide).unwrap();
        assert_eq!(given.upper_limit, UpperLimit::Given);
        assert!(given.last_frequency <= 2680.0 && given.last_frequency > 2680.0 - given.frequency_step, "{}", given.last_frequency);
        assert_eq!(
            given.notes[3],
            "above 5.345576e2 cycles per day, the Nyquist frequency of the regular cadence of 9.353528e-4 d, the power repeats aliases of the sampling: a peak there can be an alias"
        );
        println!(
            "regular cadence, typed limit 2680 per day: best frequency {:.4} per day, best period {:.9} d, FAP {:?}",
            given.best_frequency, given.best_period, given.false_alarm_probability
        );

        let below = PeriodogramOptions { max_frequency: Some(100.0), ..PeriodogramOptions::default() };
        let inside = periodogram(&time, &value, None, &below).unwrap();
        assert_eq!(inside.upper_limit, UpperLimit::Given);
        assert_eq!(inside.notes.len(), 5);

        let beyond = PeriodogramOptions { min_frequency: Some(600.0), ..PeriodogramOptions::default() };
        let above = periodogram(&time, &value, None, &beyond).unwrap();
        assert_eq!(above.upper_limit, UpperLimit::FiveMeanNyquist);
        assert_eq!(above.first_frequency, 600.0);
        assert!(above.notes[3].starts_with("above 5.345576e2 cycles per day"), "{}", above.notes[3]);
    }

    #[test]
    fn a_cadence_is_regular_when_most_steps_are_a_whole_number_of_cadences() {
        let (time_a, _, _) = series_a();
        let (time_b, _) = series_b();
        assert_eq!(regular_cadence(&time_a), None);
        assert_eq!(regular_cadence(&time_b), None);
        let with_gaps: Vec<f64> = (0..40).filter(|i| i % 10 != 7).map(|i| 0.002 * i as f64).collect();
        assert_close(regular_cadence(&with_gaps).unwrap(), 0.002, 1e-15, "cadence with gaps");
        let jitter = uniform_deviates(60);
        let jittered: Vec<f64> = (0..60).rev().map(|i| i as f64 + 0.04 * (jitter[i] - 0.5)).collect();
        assert_close(regular_cadence(&jittered).unwrap(), 1.0, 0.04, "cadence with a jitter of 4 per cent");
        let doubled: Vec<f64> = (0..20).flat_map(|i| [i as f64, i as f64]).collect();
        assert_eq!(regular_cadence(&doubled), Some(1.0));
        assert_eq!(regular_cadence(&[3.0, 3.0, f64::NAN]), None);
        assert_eq!(regular_cadence(&[]), None);
    }

    #[test]
    fn a_noise_peak_gets_a_large_false_alarm_probability() {
        let (time, value) = series_b();
        let search = periodogram(&time, &value, None, &PeriodogramOptions::default()).unwrap();
        assert_eq!(search.n_frequencies, 300);
        assert_close(search.best_frequency, 3.594231734147941, 1e-12, "best frequency");
        assert_eq!(search.best_frequency, search.frequency[208]);
        assert_close(search.best_period, 0.2782235743174926, 1e-12, "best period");
        assert_close(search.best_power, 0.4339482829125417, 1e-9, "best power");
        let fap = search.false_alarm_probability.unwrap();
        assert_close(fap, 0.3746095275854542, 1e-9, "false-alarm probability");
        let at_injected_period = gls_power(&time, &value, None, &[1.0 / 3.3]).unwrap()[0];
        assert_close(at_injected_period, 0.3974292683198885, 1e-9, "power at the injected period");
        assert!(at_injected_period < search.best_power, "the injected period of 3.3 d is not the highest peak");
        println!(
            "default search, series B: best power off astropy by {:.3e}, FAP off by {:.3e}",
            (search.best_power - 0.4339482829125417).abs(),
            (fap - 0.3746095275854542).abs()
        );
    }

    #[test]
    fn the_false_alarm_formula_matches_astropy_and_its_limits() {
        let (time, _, _) = series_a();
        let fap = false_alarm_probability_baluev(0.3, LAST_FREQUENCY_A, &time, None).unwrap();
        assert_close(fap, 0.3329742195307868, 1e-10, "FAP at z = 0.3");
        assert_eq!(false_alarm_probability_baluev(0.0, LAST_FREQUENCY_A, &time, None), Some(1.0));
        assert_eq!(false_alarm_probability_baluev(1.0, LAST_FREQUENCY_A, &time, None), Some(0.0));
        assert_eq!(false_alarm_probability_baluev(1.5, LAST_FREQUENCY_A, &time, None), None);
        assert_eq!(false_alarm_probability_baluev(f64::NAN, LAST_FREQUENCY_A, &time, None), None);
        assert_eq!(false_alarm_probability_baluev(0.3, LAST_FREQUENCY_A, &time[0..3], None), None);
        assert_eq!(false_alarm_probability_baluev(0.3, 0.0, &time, None), None);
    }

    #[test]
    fn a_user_grid_is_searched_as_given() {
        let (time, value, err) = series_a();
        let frequency: Vec<f64> = (0..31).map(|k| 0.2 + 0.01 * k as f64).collect();
        let power = gls_power(&time, &value, Some(&err), &frequency).unwrap();
        assert_eq!(power.len(), 31);
        let highest = (0..31).fold(0, |best, k| if power[k] > power[best] { k } else { best });
        assert_eq!(highest, 14);
        assert_close(power[14], 0.8608813214087616, 1e-9, "power at 0.34 cycles per day");
    }

    #[test]
    fn a_pure_sinusoid_reaches_power_one_at_its_frequency() {
        let (time, _, _) = series_a();
        let value: Vec<f64> = time.iter().map(|t| 3.0 + 2.0 * (2.0 * PI * 0.4 * t + 1.0).sin()).collect();
        let power = gls_power(&time, &value, None, &[0.4, 0.2]).unwrap();
        assert_close(power[0], 1.0, 1e-12, "power at the frequency of the sinusoid");
        assert_close(power[1], 0.011197898022834506, 1e-9, "power at half that frequency");
    }

    #[test]
    fn degenerate_frequencies_of_even_sampling_stay_finite() {
        let time: Vec<f64> = (0..16).map(|k| k as f64).collect();
        let value: Vec<f64> = (0..16).map(|k| if k % 2 == 0 { 4.0 } else { 2.0 }).collect();
        let power = gls_power(&time, &value, None, &[0.25, 0.5, 1.0, 0.1]).unwrap();
        assert!(power.iter().all(|p| p.is_finite() && (0.0..=1.0).contains(p)), "{power:?}");
        assert_close(power[0], 0.0, 1e-9, "at 0.25 both projections vanish");
        assert_close(power[1], 1.0, 1e-9, "at 0.5 the cosine term alone fits the data (astropy returns 1.127: its sine term is rounding noise)");
        assert_close(power[2], 0.0, 1e-9, "at 1.0 both terms are constant (astropy returns NaN)");
        assert_close(power[3], 0.007352941176470595, 1e-9, "at 0.1, a regular frequency, the value is astropy's");
    }

    #[test]
    fn the_power_does_not_depend_on_the_time_offset() {
        let (time, value, err) = series_a();
        let julian: Vec<f64> = time.iter().map(|t| t + 2_460_000.5).collect();
        let power = gls_power(&julian, &value, Some(&err), &SIX_FREQUENCIES).unwrap();
        assert_all_close(&power, &ASTROPY_WEIGHTED_A, 1e-7, "weighted power of series A at JD 2460000.5");
    }

    #[test]
    fn missing_points_are_dropped_and_counted() {
        let (mut time, mut value, err) = series_a();
        value[3] = f64::NAN;
        time[7] = f64::INFINITY;
        let search = periodogram(&time, &value, Some(&err), &PeriodogramOptions::default()).unwrap();
        assert_eq!(search.n_points, 38);
        assert_eq!(search.n_dropped, 2);
        assert!(search.weighted);
        assert_eq!(search.notes[0], "2 points without a finite time or value were left out");
        assert_eq!(search.notes[1], METHOD_NOTE);
        assert!(search.power.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn errors_that_cannot_weight_every_point_are_not_used() {
        let (time, value, mut err) = series_a();
        err[5] = 0.0;
        let search = periodogram(&time, &value, Some(&err), &PeriodogramOptions::default()).unwrap();
        assert!(!search.weighted);
        assert_close(search.best_power, 0.8743691833853808, 1e-9, "best power with equal weights");
        assert_eq!(
            search.notes[0],
            "errors not used: 1 of 40 points have no positive finite error; every point has the same weight"
        );
        assert_eq!(search.notes[2], "equal weights");
    }

    #[test]
    fn inputs_that_cannot_be_searched_are_refused_with_the_reason() {
        let (time, value, err) = series_a();
        let default = PeriodogramOptions::default();
        assert_eq!(
            refusal(periodogram(&time[0..3], &value[0..4], None, &default)),
            "time has 3 values and value has 4: they must have the same length"
        );
        assert_eq!(refusal(periodogram(&time, &value, Some(&err[0..2]), &default)), "err has 2 values for 40 points");
        assert_eq!(
            refusal(periodogram(&time[0..3], &value[0..3], None, &default)),
            "a period search needs at least 4 points with a finite time and value, got 3"
        );
        assert_eq!(
            refusal(periodogram(&[5.0; 4], &value[0..4], None, &default)),
            "every point has the same time: no period can be searched"
        );
        assert_eq!(
            refusal(periodogram(&time, &[2.5; 40], None, &default)),
            "the values are constant: there is nothing to search"
        );
        let coarse = PeriodogramOptions { samples_per_peak: Some(0.5), ..default };
        assert_eq!(refusal(periodogram(&time, &value, None, &coarse)), "samplesPerPeak 0.5 must be between 1 and 100");
        let from_zero = PeriodogramOptions { min_frequency: Some(0.0), ..default };
        assert_eq!(
            refusal(periodogram(&time, &value, None, &from_zero)),
            "minFrequency 0 must be a positive number of cycles per day"
        );
        let reversed = PeriodogramOptions { min_frequency: Some(2.0), max_frequency: Some(1.0), ..default };
        assert_eq!(refusal(periodogram(&time, &value, None, &reversed)), "maxFrequency 1 must be above minFrequency 2");
    }

    #[test]
    fn a_grid_above_the_cap_is_widened_with_a_note() {
        let options = PeriodogramOptions { max_frequency: Some(5000.0), ..PeriodogramOptions::default() };
        let grid = frequency_grid(40, BASELINE_A, &options).unwrap();
        assert_eq!(grid.count, 200_000);
        assert_close(grid.step, 0.02500005592188833, 1e-12, "widened step");
        assert_close(grid.first + 199_999.0 * grid.step, 5000.0, 1e-9, "last frequency");
        assert_eq!(
            grid.notes,
            vec!["frequency grid capped at 200000 samples: step widened from 1.381568e-2 to 2.500006e-2 cycles per day"
                .to_string()]
        );

        let grid = frequency_grid(100_000, 100.0, &PeriodogramOptions::default()).unwrap();
        assert_eq!(grid.count, 20_000, "the work cap is 2e9 / 1e5");
    }

    #[test]
    fn decimate_peaks_keeps_the_highest_sample_of_every_chunk() {
        let frequency: Vec<f64> = (1..=10).map(|k| k as f64).collect();
        let power = [0.0, 5.0, 1.0, 2.0, 9.0, 3.0, 4.0, 4.0, 8.0, 7.0];
        assert_eq!(decimate_peaks(&frequency, &power, 4), (vec![2.0, 5.0, 9.0, 10.0], vec![5.0, 9.0, 8.0, 7.0]));
        assert_eq!(decimate_peaks(&frequency, &power, 10), (frequency.clone(), power.to_vec()));
        assert_eq!(decimate_peaks(&frequency, &power, 0), (frequency.clone(), power.to_vec()));
    }

    #[test]
    #[ignore = "wall time of the default search for 2,000 and 10,000 points; run by hand with --ignored --nocapture"]
    fn the_default_search_of_2000_and_10000_points_recovers_the_period_and_prints_its_wall_time() {
        for points in [2_000_usize, 10_000] {
            let noise = uniform_deviates(2 * points);
            let time: Vec<f64> = (0..points).map(|i| 0.01 * (i as f64 + 0.8 * noise[i])).collect();
            let value: Vec<f64> = (0..points)
                .map(|i| 1.0 + 0.002 * (2.0 * PI * time[i] / 1.7).sin() + 0.004 * (noise[points + i] - 0.5))
                .collect();
            let started = Instant::now();
            let search = periodogram(&time, &value, None, &PeriodogramOptions::default()).unwrap();
            let elapsed = started.elapsed();
            assert!((search.best_period / 1.7 - 1.0).abs() < 0.02, "{}", search.best_period);
            println!(
                "{points} points, {} frequencies: {:.3} s, best period {:.4} d",
                search.n_frequencies,
                elapsed.as_secs_f64(),
                search.best_period
            );
        }
    }
}
