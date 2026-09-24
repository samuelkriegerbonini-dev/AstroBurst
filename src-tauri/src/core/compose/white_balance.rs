use anyhow::{bail, Result};

use crate::types::image::ImageStats;

pub const MIN_WB_SKY_SIGMAS: f64 = 1.0;
pub const MIN_WB_FACTOR: f64 = 0.01;
pub const MAX_WB_FACTOR: f64 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WbChannel {
    R,
    G,
    B,
}

impl WbChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            WbChannel::R => "R",
            WbChannel::G => "G",
            WbChannel::B => "B",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WbAnalysis {
    pub factors: (f64, f64, f64),
    pub stability: (Option<f64>, Option<f64>, Option<f64>),
    pub reference: Option<WbChannel>,
    pub empty_channels: Vec<WbChannel>,
    pub sky_not_positive: Vec<WbChannel>,
}

impl WbAnalysis {
    pub fn has_usable_signal(&self) -> bool {
        self.reference.is_some()
    }

    pub fn median_ratio_is_valid(&self) -> Result<()> {
        if self.sky_not_positive.is_empty() {
            return Ok(());
        }
        let names: Vec<&str> = self.sky_not_positive.iter().map(|c| c.as_str()).collect();
        bail!(
            "Auto white balance needs a sky level clearly above zero, but the median of channel {} is within {} sigma of zero (the sky was probably subtracted). Use SPCC or manual factors.",
            names.join(", "),
            MIN_WB_SKY_SIGMAS
        );
    }
}

fn has_data(s: &ImageStats) -> bool {
    s.valid_count > 0 && s.median.is_finite() && s.mad.is_finite() && s.sigma.is_finite()
}

fn sky_is_positive(s: &ImageStats) -> bool {
    s.median > 0.0 && s.median > MIN_WB_SKY_SIGMAS * s.sigma
}

pub fn channel_stability(s: &ImageStats) -> Option<f64> {
    if !has_data(s) || !sky_is_positive(s) {
        return None;
    }
    Some(s.mad / s.median)
}

pub fn analyze_wb_reference(sr: &ImageStats, sg: &ImageStats, sb: &ImageStats) -> WbAnalysis {
    let channels = [WbChannel::R, WbChannel::G, WbChannel::B];
    let all = [sr, sg, sb];
    let medians = [sr.median, sg.median, sb.median];
    let stability = [
        channel_stability(sr),
        channel_stability(sg),
        channel_stability(sb),
    ];

    let empty_channels: Vec<WbChannel> = channels
        .iter()
        .zip(all)
        .filter(|(_, s)| !has_data(s))
        .map(|(c, _)| *c)
        .collect();

    let sky_not_positive: Vec<WbChannel> = channels
        .iter()
        .zip(all)
        .filter(|(_, s)| has_data(s) && !sky_is_positive(s))
        .map(|(c, _)| *c)
        .collect();

    const TIE_BREAK_ORDER: [usize; 3] = [0, 2, 1];

    let reference_index = TIE_BREAK_ORDER
        .into_iter()
        .filter_map(|i| stability[i].map(|st| (i, st)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i);

    let factors = match reference_index {
        Some(ref_idx) => {
            let ref_median = medians[ref_idx];
            let factor = |i: usize| {
                if stability[i].is_none() {
                    1.0
                } else {
                    ref_median / medians[i]
                }
            };
            (factor(0), factor(1), factor(2))
        }
        None => (1.0, 1.0, 1.0),
    };

    WbAnalysis {
        factors,
        stability: (stability[0], stability[1], stability[2]),
        reference: reference_index.map(|i| channels[i]),
        empty_channels,
        sky_not_positive,
    }
}

pub fn select_wb_reference(
    sr: &ImageStats,
    sg: &ImageStats,
    sb: &ImageStats,
) -> Result<(f64, f64, f64)> {
    let analysis = analyze_wb_reference(sr, sg, sb);
    analysis.median_ratio_is_valid()?;

    if !analysis.has_usable_signal() {
        bail!("Auto white balance found no usable signal: R, G and B are all empty. Check the stacked channels.");
    }

    let (r, g, b) = analysis.factors;

    Ok((
        validate_wb_factor(WbChannel::R, r)?,
        validate_wb_factor(WbChannel::G, g)?,
        validate_wb_factor(WbChannel::B, b)?,
    ))
}

pub fn validate_wb_factor(channel: WbChannel, factor: f64) -> Result<f64> {
    if !factor.is_finite() || factor < MIN_WB_FACTOR || factor > MAX_WB_FACTOR {
        bail!(
            "White balance factor for channel {} is {:e}, outside the usable range [{}, {}]. A channel without signal cannot be recovered by scaling.",
            channel.as_str(),
            factor,
            MIN_WB_FACTOR,
            MAX_WB_FACTOR
        );
    }
    Ok(factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stats(median: f64, mad: f64) -> ImageStats {
        ImageStats {
            min: 0.0,
            max: 1.0,
            median,
            mad,
            sigma: mad * 1.4826,
            mean: median,
            valid_count: 1000,
        }
    }

    fn empty_stats() -> ImageStats {
        ImageStats::default()
    }

    fn noisy_channel(pedestal: f32, sigma: f32, seed: u64) -> ImageStats {
        let mut state = seed;
        let mut uniform = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        let data = ndarray::Array2::from_shape_fn((128, 128), |(y, x)| {
            let gauss = (-2.0 * uniform().ln()).sqrt() * (2.0 * std::f64::consts::PI * uniform()).cos();
            let star = if y % 32 == 16 && x % 32 == 16 { 500.0 } else { 0.0 };
            pedestal + sigma * gauss as f32 + star
        });
        crate::core::imaging::stats::compute_image_stats(&data)
    }

    #[test]
    fn sky_subtracted_channels_refuse_auto_white_balance_instead_of_returning_noise_ratios() {
        let (sr, sg, sb) = (noisy_channel(0.0, 3.0, 1), noisy_channel(0.0, 1.0, 2), noisy_channel(0.0, 2.0, 3));
        let err = select_wb_reference(&sr, &sg, &sb).expect_err("noise ratios were returned as colour factors");
        assert!(err.to_string().contains("sky level clearly above zero"), "unexpected error: {}", err);

        let (sr, sg, sb) = (noisy_channel(100.0, 3.0, 1), noisy_channel(100.0, 1.0, 2), noisy_channel(100.0, 2.0, 3));
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb).unwrap();
        for f in [r, g, b] {
            assert!((f - 1.0).abs() < 0.01, "neutral channels with a sky pedestal gave {r} {g} {b}");
        }
    }

    #[test]
    fn equal_channels_return_ones() {
        let s = make_stats(0.5, 0.01);
        let (r, g, b) = select_wb_reference(&s, &s, &s).unwrap();
        assert!((r - 1.0).abs() < 1e-12);
        assert!((g - 1.0).abs() < 1e-12);
        assert!((b - 1.0).abs() < 1e-12);
    }

    #[test]
    fn red_most_stable() {
        let sr = make_stats(0.5, 0.001);
        let sg = make_stats(0.4, 0.02);
        let sb = make_stats(0.3, 0.03);
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb).unwrap();
        assert!((r - 1.0).abs() < 1e-12);
        assert!((g - sr.median / sg.median).abs() < 1e-12);
        assert!((b - sr.median / sb.median).abs() < 1e-12);
    }

    #[test]
    fn green_most_stable() {
        let sr = make_stats(0.5, 0.05);
        let sg = make_stats(0.4, 0.001);
        let sb = make_stats(0.3, 0.03);
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb).unwrap();
        assert!((r - sg.median / sr.median).abs() < 1e-12);
        assert!((g - 1.0).abs() < 1e-12);
        assert!((b - sg.median / sb.median).abs() < 1e-12);
    }

    #[test]
    fn blue_most_stable() {
        let sr = make_stats(0.5, 0.05);
        let sg = make_stats(0.4, 0.04);
        let sb = make_stats(0.3, 0.001);
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb).unwrap();
        assert!((r - sb.median / sr.median).abs() < 1e-12);
        assert!((g - sb.median / sg.median).abs() < 1e-12);
        assert!((b - 1.0).abs() < 1e-12);
    }

    #[test]
    fn empty_channel_yields_neutral_factor_and_is_reported() {
        let sr = empty_stats();
        let sg = make_stats(0.5, 0.01);
        let sb = make_stats(0.3, 0.02);
        let analysis = analyze_wb_reference(&sr, &sg, &sb);

        assert_eq!(analysis.factors.0, 1.0);
        assert_eq!(analysis.empty_channels, vec![WbChannel::R]);
        assert_eq!(analysis.reference, Some(WbChannel::G));
        assert!(analysis.stability.0.is_none());
        assert!((analysis.factors.2 - sg.median / sb.median).abs() < 1e-12);
    }

    #[test]
    fn two_empty_channels_do_not_produce_identical_large_factors() {
        let sr = empty_stats();
        let sg = make_stats(6.367_100_715_637_207e0, 0.01);
        let sb = empty_stats();
        let analysis = analyze_wb_reference(&sr, &sg, &sb);

        assert_eq!(analysis.reference, Some(WbChannel::G));
        assert_eq!(analysis.factors, (1.0, 1.0, 1.0));
        assert_eq!(
            analysis.empty_channels,
            vec![WbChannel::R, WbChannel::B]
        );
    }

    #[test]
    fn all_channels_empty_report_no_usable_signal() {
        let analysis = analyze_wb_reference(&empty_stats(), &empty_stats(), &empty_stats());

        assert!(!analysis.has_usable_signal());
        assert_eq!(analysis.reference, None);
        assert_eq!(
            analysis.empty_channels,
            vec![WbChannel::R, WbChannel::G, WbChannel::B]
        );
    }

    #[test]
    fn empty_channel_never_becomes_reference() {
        let sr = ImageStats {
            valid_count: 0,
            ..make_stats(0.5, 0.0)
        };
        let sg = make_stats(0.4, 0.02);
        let sb = make_stats(0.3, 0.03);
        let analysis = analyze_wb_reference(&sr, &sg, &sb);

        assert_eq!(analysis.reference, Some(WbChannel::G));
        assert_eq!(analysis.factors.0, 1.0);
    }

    #[test]
    fn a_median_within_the_noise_of_zero_refuses_the_median_ratio() {
        let sr = make_stats(0.001, 0.01);
        let sg = make_stats(0.5, 0.01);
        let sb = make_stats(-0.002, 0.02);
        let analysis = analyze_wb_reference(&sr, &sg, &sb);

        assert_eq!(analysis.sky_not_positive, vec![WbChannel::R, WbChannel::B]);
        assert!(analysis.empty_channels.is_empty());
        assert!(analysis.stability.0.is_none());
        let err = select_wb_reference(&sr, &sg, &sb).expect_err("a noise-level median must not be divided");
        assert!(err.to_string().contains("channel R, B"), "unexpected error: {}", err);
        assert!(err.to_string().contains("SPCC"), "unexpected error: {}", err);
    }

    #[test]
    fn a_tiny_flux_scale_is_balanced_like_any_other_scale() {
        let scale = 1e-19;
        let sr = make_stats(0.5 * scale, 0.01 * scale);
        let sg = make_stats(0.25 * scale, 0.02 * scale);
        let sb = make_stats(0.125 * scale, 0.03 * scale);
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb).unwrap();
        assert!((r - 1.0).abs() < 1e-9);
        assert!((g - 2.0).abs() < 1e-9);
        assert!((b - 4.0).abs() < 1e-9);
    }

    #[test]
    fn usable_factors_stay_within_validation_bounds() {
        let sr = make_stats(0.001, 0.0001);
        let sg = make_stats(0.5, 0.01);
        let sb = make_stats(0.3, 0.02);
        let analysis = analyze_wb_reference(&sr, &sg, &sb);

        assert!(analysis.factors.0 > MAX_WB_FACTOR);
        assert!(validate_wb_factor(WbChannel::R, analysis.factors.0).is_err());
    }

    #[test]
    fn validate_wb_factor_rejects_degenerate_gains() {
        assert!(validate_wb_factor(WbChannel::R, 63_671_007_156.372_07).is_err());
        assert!(validate_wb_factor(WbChannel::B, f64::NAN).is_err());
        assert!(validate_wb_factor(WbChannel::G, 0.0).is_err());
        assert!(validate_wb_factor(WbChannel::G, f64::INFINITY).is_err());
        assert_eq!(validate_wb_factor(WbChannel::G, 1.0).unwrap(), 1.0);
        assert_eq!(
            validate_wb_factor(WbChannel::R, MAX_WB_FACTOR).unwrap(),
            MAX_WB_FACTOR
        );
    }

    #[test]
    fn select_wb_reference_rejects_all_empty_composite() {
        let err = select_wb_reference(&empty_stats(), &empty_stats(), &empty_stats())
            .expect_err("an entirely empty composite must not report neutral factors as success");

        assert!(
            err.to_string().contains("no usable signal"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn select_wb_reference_rejects_runaway_gain_before_it_reaches_pixels() {
        let sr = make_stats(0.001, 0.0001);
        let sg = make_stats(0.5, 0.01);
        let sb = make_stats(0.3, 0.02);

        assert!(analyze_wb_reference(&sr, &sg, &sb).factors.0 > MAX_WB_FACTOR);

        let err = select_wb_reference(&sr, &sg, &sb)
            .expect_err("a 500x gain must not be handed to the pixel loop");
        assert!(err.to_string().contains("channel R"), "unexpected error: {}", err);
    }

    #[test]
    fn select_wb_reference_passes_empty_channels_through_as_neutral() {
        let sg = make_stats(0.5, 0.01);
        let sb = make_stats(0.25, 0.02);
        let (r, g, b) = select_wb_reference(&empty_stats(), &sg, &sb).unwrap();

        assert_eq!(r, 1.0);
        assert_eq!(g, 1.0);
        assert!((b - 2.0).abs() < 1e-12);
    }

    #[test]
    fn reference_tie_between_blue_and_green_picks_blue() {
        let sr = make_stats(0.5, 0.05);
        let sg = make_stats(0.4, 0.004);
        let sb = make_stats(0.4, 0.004);
        let analysis = analyze_wb_reference(&sr, &sg, &sb);

        assert_eq!(analysis.stability.1, analysis.stability.2);
        assert_eq!(analysis.reference, Some(WbChannel::B));
    }
}
