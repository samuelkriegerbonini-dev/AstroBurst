use anyhow::{bail, Result};

use super::TsoSeries;

pub const MAX_BINS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    pub lo: f64,
    pub hi: f64,
    pub hi_inclusive: bool,
}

impl Interval {
    pub fn white(band_um: [f64; 2]) -> Interval {
        Interval { lo: band_um[0], hi: band_um[1], hi_inclusive: true }
    }

    pub fn bin(pair: [f64; 2]) -> Interval {
        Interval { lo: pair[0], hi: pair[1], hi_inclusive: false }
    }

    pub fn contains(&self, wavelength_um: f64) -> bool {
        wavelength_um >= self.lo && (wavelength_um < self.hi || (self.hi_inclusive && wavelength_um == self.hi))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntervalSum {
    pub flux: Vec<f64>,
    pub flux_err: Vec<f64>,
    pub n_used: Vec<u32>,
    pub npixels_mean: Vec<f64>,
    pub any_inside: bool,
}

#[derive(Default)]
struct RowSum {
    flux: f64,
    error_squared: f64,
    used: u32,
    npixels: f64,
    with_npixels: u32,
}

impl RowSum {
    fn add(&mut self, series: &TsoSeries, sample: usize) {
        self.flux += series.flux[sample];
        self.used += 1;
        let error = series.flux_error[sample];
        if error.is_finite() && error > 0.0 {
            self.error_squared += error * error;
        }
        let npixels = series.npixels[sample];
        if npixels.is_finite() {
            self.npixels += npixels;
            self.with_npixels += 1;
        }
    }

    fn npixels_mean(&self) -> f64 {
        if self.with_npixels == 0 {
            return f64::NAN;
        }
        self.npixels / f64::from(self.with_npixels)
    }
}

fn interval_sum(rows: &[RowSum], any_inside: bool) -> IntervalSum {
    let or_empty = |value: f64| if any_inside { value } else { f64::NAN };
    IntervalSum {
        flux: rows.iter().map(|row| or_empty(row.flux)).collect(),
        flux_err: rows.iter().map(|row| or_empty(row.error_squared.sqrt())).collect(),
        n_used: rows.iter().map(|row| row.used).collect(),
        npixels_mean: rows.iter().map(RowSum::npixels_mean).collect(),
        any_inside,
    }
}

pub fn check_bins(bins: &[[f64; 2]]) -> Result<()> {
    if bins.len() > MAX_BINS {
        bail!("{} bins requested: at most {MAX_BINS} are computed at once", bins.len());
    }
    for (index, [lo, hi]) in bins.iter().enumerate() {
        if !(lo.is_finite() && hi.is_finite() && lo < hi) {
            bail!("bin {}: {lo} to {hi} um is empty or not finite", index + 1);
        }
    }
    Ok(())
}

pub fn sum_rows(series: &TsoSeries, rows: &[usize], interval: Interval) -> IntervalSum {
    let mut any_inside = false;
    let sums: Vec<RowSum> = rows
        .iter()
        .map(|&row| {
            let mut sum = RowSum::default();
            for sample in series.sample(row, 0)..series.sample(row + 1, 0) {
                if !interval.contains(series.wavelength_um[sample]) {
                    continue;
                }
                any_inside = true;
                if !series.flux[sample].is_nan() {
                    sum.add(series, sample);
                }
            }
            sum
        })
        .collect();
    interval_sum(&sums, any_inside)
}

pub fn sum_channels(series: &TsoSeries, channels: &[usize], interval: Interval) -> IntervalSum {
    let inside: Vec<usize> = channels.iter().copied().filter(|&channel| interval.contains(series.grid_um[channel])).collect();
    let sums: Vec<RowSum> = (0..series.n_integrations)
        .map(|row| {
            let mut sum = RowSum::default();
            for &channel in &inside {
                sum.add(series, series.sample(row, channel));
            }
            sum
        })
        .collect();
    interval_sum(&sums, !inside.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::tso::series::test_support::ramp_set;

    #[test]
    fn an_interval_knows_its_ends_and_never_contains_nan() {
        let white = Interval::white([1.0, 2.0]);
        assert_eq!(white, Interval { lo: 1.0, hi: 2.0, hi_inclusive: true });
        assert!(white.contains(1.0) && white.contains(1.5) && white.contains(2.0));
        assert!(!white.contains(2.0000001) && !white.contains(0.9999999));
        assert!(!white.contains(f64::NAN));

        let bin = Interval::bin([1.0, 2.0]);
        assert_eq!(bin, Interval { lo: 1.0, hi: 2.0, hi_inclusive: false });
        assert!(bin.contains(1.0) && bin.contains(1.9999999));
        assert!(!bin.contains(2.0) && !bin.contains(0.9999999));
        assert!(!bin.contains(f64::NAN));

        assert!(check_bins(&[]).is_ok());
        assert!(check_bins(&[[1.0, 2.0], [2.0, 3.0], [0.5, 9.0]]).is_ok());
        assert!(check_bins(&vec![[1.0, 2.0]; MAX_BINS]).is_ok());
        let refusal = |bins: &[[f64; 2]]| check_bins(bins).unwrap_err().to_string();
        assert_eq!(refusal(&vec![[1.0, 2.0]; MAX_BINS + 1]), "513 bins requested: at most 512 are computed at once");
        assert_eq!(refusal(&[[1.0, 1.2], [1.3, 1.3]]), "bin 2: 1.3 to 1.3 um is empty or not finite");
        assert_eq!(refusal(&[[2.0, 1.0]]), "bin 1: 2 to 1 um is empty or not finite");
        assert_eq!(refusal(&[[1.0, 1.2], [1.2, 1.4], [f64::NAN, 2.0]]), "bin 3: NaN to 2 um is empty or not finite");
        assert_eq!(refusal(&[[1.0, f64::INFINITY]]), "bin 1: 1 to inf um is empty or not finite");
    }

    #[test]
    fn row_sums_and_channel_sums_agree_on_a_series_without_gaps() {
        let mut set = ramp_set("NRS1", Some(0), 3, 5);
        let g = set.series[0].grid_um.clone();
        let whole = Interval::white([g[0], g[4]]);
        let beyond = Interval::white([5.0, 6.0]);

        let s = &set.series[0];
        let by_rows = sum_rows(s, &[0, 1, 2], whole);
        let by_channels = sum_channels(s, &[0, 1, 2, 3, 4], whole);
        assert_eq!(by_rows.flux, [60.0, 110.0, 160.0]);
        assert_eq!(by_rows.flux_err, [1.25f64.sqrt(); 3]);
        assert_eq!(by_rows.n_used, [5, 5, 5]);
        assert_eq!(by_rows.npixels_mean, [7.0, 7.0, 7.0]);
        assert!(by_rows.any_inside && by_channels.any_inside);
        assert_eq!(by_rows.flux, by_channels.flux);
        assert_eq!(by_rows.flux_err, by_channels.flux_err);
        assert_eq!(by_rows.n_used, by_channels.n_used);
        assert_eq!(by_rows.npixels_mean, by_channels.npixels_mean);

        let one_row = sum_rows(s, &[1], Interval::bin([g[2], g[4]]));
        assert_eq!(one_row.flux, [45.0]);
        assert_eq!(one_row.n_used, [2]);
        assert_eq!(one_row.flux_err, [0.5f64.sqrt()]);
        let in_the_order_given = sum_rows(s, &[2, 0], whole);
        assert_eq!(in_the_order_given.flux, [160.0, 60.0]);

        let two_channels = sum_channels(s, &[1, 3], whole);
        assert_eq!(two_channels.flux, [24.0, 44.0, 64.0]);
        assert_eq!(two_channels.n_used, [2, 2, 2]);
        let upper_end_left_out = sum_channels(s, &[0, 1, 2, 3, 4], Interval::bin([g[2], g[4]]));
        assert_eq!(upper_end_left_out.flux, [25.0, 45.0, 65.0]);
        assert_eq!(upper_end_left_out.n_used, [2, 2, 2]);

        for outside in [sum_rows(s, &[0, 1, 2], beyond), sum_channels(s, &[0, 1, 2, 3, 4], beyond), sum_channels(s, &[], whole)] {
            assert!(!outside.any_inside);
            assert_eq!(outside.flux.len(), 3);
            assert!(outside.flux.iter().chain(&outside.flux_err).chain(&outside.npixels_mean).all(|value| value.is_nan()));
            assert_eq!(outside.n_used, [0, 0, 0]);
        }
        let no_row = sum_rows(s, &[], whole);
        assert!(!no_row.any_inside);
        assert!(no_row.flux.is_empty() && no_row.flux_err.is_empty() && no_row.n_used.is_empty() && no_row.npixels_mean.is_empty());

        for channel in 0..5 {
            let sample = set.series[0].sample(1, channel);
            set.series[0].flux[sample] = f64::NAN;
        }
        let without_npixels = set.series[0].sample(0, 0);
        set.series[0].npixels[without_npixels] = f64::NAN;
        let with_an_empty_row = sum_rows(&set.series[0], &[0, 1, 2], whole);
        assert!(with_an_empty_row.any_inside);
        assert_eq!(with_an_empty_row.flux, [60.0, 0.0, 160.0]);
        assert_eq!(with_an_empty_row.flux_err, [1.25f64.sqrt(), 0.0, 1.25f64.sqrt()]);
        assert_eq!(with_an_empty_row.n_used, [5, 0, 5]);
        assert_eq!(with_an_empty_row.npixels_mean[0], 7.0);
        assert!(with_an_empty_row.npixels_mean[1].is_nan());
        assert_eq!(with_an_empty_row.npixels_mean[2], 7.0);
    }
}
