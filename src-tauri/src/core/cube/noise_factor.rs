use anyhow::{bail, Result};
use serde::Serialize;

use crate::core::cube::moments::ContinuumWindows;
use crate::math::exact_median_f64;
use crate::types::constants::MAD_TO_SIGMA;

pub const MIN_WINDOW_CHANNELS: usize = 5;
pub const EXPECTED_FACTOR_RANGE: (f64, f64) = (0.8, 1.25);
pub const WARNING_STANDARD_ERRORS: f64 = 2.0;
const LINE_PARAMETERS_PER_WINDOW: usize = 2;
const MAD_SIGMA_RELATIVE_SE_COEFFICIENT: f64 = 1.1664;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoiseFactor {
    pub factor: f64,
    pub factor_se: f64,
    pub n_channels: usize,
    pub windows: ContinuumWindows,
    pub warning: Option<String>,
}

fn usable(value: f32, err: f32) -> bool {
    value.is_finite() && err.is_finite() && err > 0.0
}

fn fitted_windows(windows: ContinuumWindows, channels: usize) -> Result<Vec<(usize, usize)>> {
    let (first, second) = windows;
    for (start, end) in [first, second] {
        if start > end || end >= channels {
            bail!(
                "continuum window {}..={} is invalid for a spectrum of {} channels",
                start,
                end,
                channels
            );
        }
        let length = end - start + 1;
        if length < MIN_WINDOW_CHANNELS {
            bail!(
                "continuum window {}..={} has {} channels: the noise factor needs at least {} per window",
                start,
                end,
                length,
                MIN_WINDOW_CHANNELS
            );
        }
    }
    if first == second {
        return Ok(vec![first]);
    }
    if first.0 <= second.1 && second.0 <= first.1 {
        bail!(
            "continuum windows {}..={} and {}..={} overlap: the noise factor needs two separate line-free windows",
            first.0,
            first.1,
            second.0,
            second.1
        );
    }
    Ok(vec![first, second])
}

fn normalised_line_residuals(values: &[f32], err: &[f32], (start, end): (usize, usize)) -> Result<Vec<f64>> {
    let channels: Vec<usize> = (start..=end).filter(|&z| usable(values[z], err[z])).collect();
    if channels.len() < MIN_WINDOW_CHANNELS {
        bail!(
            "continuum window {}..={} has only {} usable channels (finite value and positive error): the noise factor needs at least {} per window",
            start,
            end,
            channels.len(),
            MIN_WINDOW_CHANNELS
        );
    }
    let n = channels.len() as f64;
    let x_mean = channels.iter().map(|&z| z as f64).sum::<f64>() / n;
    let y_mean = channels.iter().map(|&z| values[z] as f64).sum::<f64>() / n;
    let (sxx, sxy) = channels.iter().fold((0.0f64, 0.0f64), |(sxx, sxy), &z| {
        let dx = z as f64 - x_mean;
        (sxx + dx * dx, sxy + dx * (values[z] as f64 - y_mean))
    });
    let slope = sxy / sxx;
    Ok(channels
        .iter()
        .map(|&z| (values[z] as f64 - y_mean - slope * (z as f64 - x_mean)) / err[z] as f64)
        .collect())
}

pub(crate) fn residual_dof_correction(n_residuals: usize, fitted_parameters: usize) -> f64 {
    (n_residuals as f64 / (n_residuals - fitted_parameters) as f64).sqrt()
}

pub fn noise_factor(values: &[f32], err: &[f32], windows: ContinuumWindows) -> Result<NoiseFactor> {
    if err.len() != values.len() {
        bail!(
            "{} error values for {} channels: the noise factor needs one error per channel",
            err.len(),
            values.len()
        );
    }
    let fitted = fitted_windows(windows, values.len())?;
    let mut normalised = Vec::new();
    for &window in &fitted {
        normalised.extend(normalised_line_residuals(values, err, window)?);
    }
    let n_channels = normalised.len();
    let fitted_parameters = LINE_PARAMETERS_PER_WINDOW * fitted.len();
    let centre = exact_median_f64(&normalised);
    let deviations: Vec<f64> = normalised.iter().map(|r| (r - centre).abs()).collect();
    let factor =
        exact_median_f64(&deviations) * MAD_TO_SIGMA * residual_dof_correction(n_channels, fitted_parameters);
    let relative_se = MAD_SIGMA_RELATIVE_SE_COEFFICIENT / ((n_channels - fitted_parameters) as f64).sqrt();
    let factor_se = factor * relative_se;
    let (low, high) = EXPECTED_FACTOR_RANGE;
    let spread = (WARNING_STANDARD_ERRORS * relative_se).exp();
    let warning = (factor * spread < low || factor / spread > high).then(|| {
        format!(
            "the empirical noise is {:.2} +- {:.2} x the propagated error over {} line-free channels, outside [{}, {}] by more than {} standard errors: the ERR plane does not describe the scatter of this spectrum",
            factor, factor_se, n_channels, low, high, WARNING_STANDARD_ERRORS
        )
    });
    Ok(NoiseFactor { factor, factor_se, n_channels, windows, warning })
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    use super::*;

    const CHANNELS: usize = 6000;
    const WINDOWS: ContinuumWindows = ((0, 2499), (3500, 5999));
    const LINE_FREE_CHANNELS: usize = 5000;
    const SIGMA: f64 = 0.3;
    const SEED: u64 = 20_261_016;
    const UNIT_TOLERANCE: f64 = 0.05;
    const DOUBLE_TOLERANCE: f64 = 0.1;
    const SHORT_WINDOW_CHANNELS: usize = 6;
    const SHORT_WINDOW_REALISATIONS: usize = 4000;

    fn gaussian(rng: &mut StdRng) -> f64 {
        let u1: f64 = rng.gen::<f64>().max(1e-300);
        let u2: f64 = rng.gen::<f64>();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    fn continuum_and_line(z: usize) -> f64 {
        let d = z as f64 - 3000.0;
        10.0 + 0.002 * z as f64 + 5.0 * (-d * d / (2.0 * 120.0 * 120.0)).exp()
    }

    fn region_spectrum(seed: u64) -> Vec<f32> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..CHANNELS).map(|z| (continuum_and_line(z) + SIGMA * gaussian(&mut rng)) as f32).collect()
    }

    #[test]
    fn white_noise_with_its_true_err_gives_a_factor_of_one_within_5_percent_on_5000_line_free_channels_seed_20261016() {
        let values = region_spectrum(SEED);
        let err = vec![SIGMA as f32; CHANNELS];
        let factor = noise_factor(&values, &err, WINDOWS).unwrap();
        assert_eq!(factor.n_channels, LINE_FREE_CHANNELS);
        assert_eq!(factor.windows, WINDOWS);
        assert!((factor.factor - 1.0).abs() <= UNIT_TOLERANCE, "factor {}", factor.factor);
        assert!(factor.warning.is_none(), "{:?}", factor.warning);
    }

    #[test]
    fn halving_the_err_doubles_the_factor_within_0_1_and_warns_seed_20261016() {
        let values = region_spectrum(SEED);
        let err = vec![(SIGMA / 2.0) as f32; CHANNELS];
        let factor = noise_factor(&values, &err, WINDOWS).unwrap();
        assert!((factor.factor - 2.0).abs() <= DOUBLE_TOLERANCE, "factor {}", factor.factor);
        let expected_se = factor.factor * 1.1664 / ((LINE_FREE_CHANNELS - 4) as f64).sqrt();
        assert!((factor.factor_se - expected_se).abs() <= 1e-12, "se {} expected {}", factor.factor_se, expected_se);
        assert!((0.03..0.036).contains(&factor.factor_se), "se {}", factor.factor_se);
        let warning = factor.warning.expect("a factor of 2 is outside the expected range");
        assert!(warning.contains("outside [0.8, 1.25] by more than 2 standard errors"), "{}", warning);
        assert!(warning.contains(&format!("{:.2} +- {:.2} x", factor.factor, factor.factor_se)), "{}", warning);
    }

    #[test]
    fn the_warning_needs_the_band_exceeded_by_more_than_two_standard_errors() {
        let windows: ContinuumWindows = ((0, 49), (100, 149));
        let length = windows.1 .1 + 1;
        let mut rng = StdRng::seed_from_u64(SEED);
        let values: Vec<f32> = (0..length).map(|z| (continuum_and_line(z) + SIGMA * gaussian(&mut rng)) as f32).collect();
        let true_err = vec![SIGMA as f32; length];
        let reference = noise_factor(&values, &true_err, windows).unwrap();
        let just_outside = EXPECTED_FACTOR_RANGE.1 * 1.05;
        let scaled_err: Vec<f32> = true_err.iter().map(|&e| (e as f64 * reference.factor / just_outside) as f32).collect();
        let scaled = noise_factor(&values, &scaled_err, windows).unwrap();
        assert!((scaled.factor - just_outside).abs() < 1e-5, "factor {}", scaled.factor);
        assert!(scaled.factor_se > 0.1, "se {} over 100 channels", scaled.factor_se);
        assert!(scaled.warning.is_none(), "{:?}", scaled.warning);
        let far_err: Vec<f32> = true_err.iter().map(|&e| (e as f64 * reference.factor / 2.5) as f32).collect();
        let far = noise_factor(&values, &far_err, windows).unwrap();
        assert!(far.warning.is_some(), "factor {} +- {} over 100 channels", far.factor, far.factor_se);
    }

    #[test]
    fn a_window_shorter_than_5_channels_is_refused_by_name() {
        let values = region_spectrum(SEED);
        let err = vec![SIGMA as f32; CHANNELS];
        let first = noise_factor(&values, &err, ((0, 3), WINDOWS.1)).unwrap_err().to_string();
        assert!(first.contains("continuum window 0..=3 has 4 channels"), "{}", first);
        assert!(first.contains("at least 5"), "{}", first);
        let second = noise_factor(&values, &err, (WINDOWS.0, (5997, 5999))).unwrap_err().to_string();
        assert!(second.contains("continuum window 5997..=5999 has 3 channels"), "{}", second);
    }

    #[test]
    fn windows_outside_the_spectrum_overlapping_or_with_mismatched_err_are_refused() {
        let values = region_spectrum(SEED);
        let err = vec![SIGMA as f32; CHANNELS];
        let outside = noise_factor(&values, &err, (WINDOWS.0, (5990, 6000))).unwrap_err().to_string();
        assert!(outside.contains("continuum window 5990..=6000"), "{}", outside);
        assert!(outside.contains("6000 channels"), "{}", outside);
        let overlapping = noise_factor(&values, &err, ((0, 100), (50, 200))).unwrap_err().to_string();
        assert!(overlapping.contains("overlap"), "{}", overlapping);
        let mismatched = noise_factor(&values, &err[..10], WINDOWS).unwrap_err().to_string();
        assert!(mismatched.contains("10 error values for 6000 channels"), "{}", mismatched);
    }

    #[test]
    fn identical_windows_count_once_and_unusable_channels_are_skipped() {
        let mut values = region_spectrum(SEED);
        let mut err = vec![SIGMA as f32; CHANNELS];
        values[10] = f32::NAN;
        err[11] = f32::NAN;
        err[12] = 0.0;
        err[13] = -1.0;
        let factor = noise_factor(&values, &err, (WINDOWS.0, WINDOWS.0)).unwrap();
        assert_eq!(factor.n_channels, 2500 - 4);
        assert!((factor.factor - 1.0).abs() <= 2.0 * UNIT_TOLERANCE, "factor {}", factor.factor);
        let mut blank = values.clone();
        blank[3500..5996].iter_mut().for_each(|v| *v = f32::NAN);
        let refused = noise_factor(&blank, &err, WINDOWS).unwrap_err().to_string();
        assert!(refused.contains("continuum window 3500..=5999 has only 4 usable channels"), "{}", refused);
    }

    #[test]
    fn short_windows_are_corrected_for_the_two_fitted_line_parameters_seed_20261016() {
        let windows: ContinuumWindows = ((0, SHORT_WINDOW_CHANNELS - 1), (100, 100 + SHORT_WINDOW_CHANNELS - 1));
        let length = windows.1 .1 + 1;
        let err = vec![SIGMA as f32; length];
        let mut rng = StdRng::seed_from_u64(SEED);
        let mut factors: Vec<f64> = (0..SHORT_WINDOW_REALISATIONS)
            .map(|_| {
                let values: Vec<f32> =
                    (0..length).map(|z| (continuum_and_line(z) + SIGMA * gaussian(&mut rng)) as f32).collect();
                let factor = noise_factor(&values, &err, windows).unwrap();
                assert_eq!(factor.n_channels, 2 * SHORT_WINDOW_CHANNELS);
                factor.factor
            })
            .collect();
        factors.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = factors[factors.len() / 2];
        assert!((0.9..=1.1).contains(&median), "median factor over 12 line-free channels {}", median);
    }

    const WARNING_RATE_REALISATIONS: usize = 4000;
    const MAX_FALSE_WARNING_RATE: f64 = 0.05;
    const MIN_HALVED_ERR_WARNING_RATE: f64 = 0.75;

    fn warning_rate(channels_per_window: usize, err_scale: f64, seed: u64) -> f64 {
        let second_start = 2 * channels_per_window;
        let windows: ContinuumWindows =
            ((0, channels_per_window - 1), (second_start, second_start + channels_per_window - 1));
        let length = windows.1 .1 + 1;
        let err = vec![(SIGMA * err_scale) as f32; length];
        let mut rng = StdRng::seed_from_u64(seed);
        let warned = (0..WARNING_RATE_REALISATIONS)
            .filter(|_| {
                let values: Vec<f32> =
                    (0..length).map(|z| (continuum_and_line(z) + SIGMA * gaussian(&mut rng)) as f32).collect();
                noise_factor(&values, &err, windows).unwrap().warning.is_some()
            })
            .count();
        warned as f64 / WARNING_RATE_REALISATIONS as f64
    }

    #[test]
    fn a_correct_err_warns_on_at_most_5_percent_of_short_window_spectra_4000_realisations_seed_20261016() {
        for channels_per_window in [MIN_WINDOW_CHANNELS, 50] {
            let rate = warning_rate(channels_per_window, 1.0, SEED);
            assert!(
                rate <= MAX_FALSE_WARNING_RATE,
                "{}+{} line-free channels with the true ERR warn on {:.1} % of spectra",
                channels_per_window,
                channels_per_window,
                100.0 * rate
            );
        }
    }

    #[test]
    fn a_halved_err_still_warns_on_most_50_plus_50_channel_spectra_4000_realisations_seed_20261016() {
        let rate = warning_rate(50, 0.5, SEED);
        assert!(rate >= MIN_HALVED_ERR_WARNING_RATE, "halved ERR over 50+50 channels warns on {:.1} %", 100.0 * rate);
    }

    fn spectrum_with_err_steps(first: f64, second: f64, step_channel: usize, seed: u64) -> (Vec<f32>, Vec<f32>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let err: Vec<f64> = (0..CHANNELS).map(|z| if z < step_channel { first } else { second }).collect();
        let values = (0..CHANNELS).map(|z| (continuum_and_line(z) + err[z] * gaussian(&mut rng)) as f32).collect();
        (values, err.iter().map(|&e| e as f32).collect())
    }

    #[test]
    fn an_err_that_is_correct_but_differs_between_channels_gives_a_factor_of_one_seed_20261016() {
        for (first, second) in [(1.0, 2.0), (1.0, 1.3)] {
            let (values, err) = spectrum_with_err_steps(first, second, 4000, SEED);
            let factor = noise_factor(&values, &err, WINDOWS).unwrap();
            assert_eq!(factor.n_channels, LINE_FREE_CHANNELS);
            assert!(
                (factor.factor - 1.0).abs() <= UNIT_TOLERANCE,
                "ERR {} on 3000 channels and {} on 2000: factor {}",
                first,
                second,
                factor.factor
            );
            assert!(factor.warning.is_none(), "{:?}", factor.warning);
        }
    }
}
