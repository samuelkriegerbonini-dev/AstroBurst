use anyhow::{bail, Result};

use crate::types::constants::PADDING_THRESHOLD;
use crate::types::image::ImageStats;

pub const MIN_USABLE_MEDIAN: f64 = PADDING_THRESHOLD as f64;
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
}

impl WbAnalysis {
    pub fn has_usable_signal(&self) -> bool {
        self.reference.is_some()
    }
}

pub fn channel_stability(s: &ImageStats) -> Option<f64> {
    if s.valid_count == 0
        || !s.median.is_finite()
        || s.median <= MIN_USABLE_MEDIAN
        || !s.mad.is_finite()
    {
        return None;
    }
    Some(s.mad / s.median)
}

pub fn analyze_wb_reference(sr: &ImageStats, sg: &ImageStats, sb: &ImageStats) -> WbAnalysis {
    let channels = [WbChannel::R, WbChannel::G, WbChannel::B];
    let medians = [sr.median, sg.median, sb.median];
    let stability = [
        channel_stability(sr),
        channel_stability(sg),
        channel_stability(sb),
    ];

    let empty_channels: Vec<WbChannel> = channels
        .iter()
        .enumerate()
        .filter(|(i, _)| stability[*i].is_none())
        .map(|(_, c)| *c)
        .collect();

    const TIE_BREAK_ORDER: [usize; 3] = [0, 2, 1];

    let reference_index = TIE_BREAK_ORDER
        .into_iter()
        .filter(|i| stability[*i].is_some())
        .min_by(|a, b| {
            stability[*a]
                .unwrap()
                .partial_cmp(&stability[*b].unwrap())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

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
    }
}

pub fn select_wb_reference(
    sr: &ImageStats,
    sg: &ImageStats,
    sb: &ImageStats,
) -> Result<(f64, f64, f64)> {
    let analysis = analyze_wb_reference(sr, sg, sb);

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
    fn median_at_padding_threshold_counts_as_empty() {
        let sr = make_stats(MIN_USABLE_MEDIAN, 0.0);
        let sg = make_stats(0.5, 0.01);
        let sb = make_stats(0.3, 0.02);
        let analysis = analyze_wb_reference(&sr, &sg, &sb);

        assert_eq!(analysis.empty_channels, vec![WbChannel::R]);
        assert_eq!(analysis.factors.0, 1.0);
    }

    #[test]
    fn usable_factors_stay_within_validation_bounds() {
        let sr = make_stats(MIN_USABLE_MEDIAN * 2.0, MIN_USABLE_MEDIAN);
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
        let sr = make_stats(MIN_USABLE_MEDIAN * 2.0, MIN_USABLE_MEDIAN);
        let sg = make_stats(0.5, 0.01);
        let sb = make_stats(0.3, 0.02);

        assert!(analyze_wb_reference(&sr, &sg, &sb).factors.0 > MAX_WB_FACTOR);

        let err = select_wb_reference(&sr, &sg, &sb)
            .expect_err("a 2.5e6 gain must not be handed to the pixel loop");
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
