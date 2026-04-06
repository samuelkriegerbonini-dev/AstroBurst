use crate::types::image::ImageStats;

pub fn select_wb_reference(sr: &ImageStats, sg: &ImageStats, sb: &ImageStats) -> (f64, f64, f64) {
    let stability = |s: &ImageStats| -> f64 {
        if s.median > 1e-10 { s.mad / s.median } else { f64::MAX }
    };
    let stab_r = stability(sr);
    let stab_g = stability(sg);
    let stab_b = stability(sb);
    if stab_r <= stab_g && stab_r <= stab_b {
        let m = sr.median.max(1e-10);
        (1.0, m / sg.median.max(1e-10), m / sb.median.max(1e-10))
    } else if stab_b <= stab_g {
        let m = sb.median.max(1e-10);
        (m / sr.median.max(1e-10), m / sg.median.max(1e-10), 1.0)
    } else {
        let m = sg.median.max(1e-10);
        (m / sr.median.max(1e-10), 1.0, m / sb.median.max(1e-10))
    }
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

    #[test]
    fn equal_channels_return_ones() {
        let s = make_stats(0.5, 0.01);
        let (r, g, b) = select_wb_reference(&s, &s, &s);
        assert!((r - 1.0).abs() < 1e-12);
        assert!((g - 1.0).abs() < 1e-12);
        assert!((b - 1.0).abs() < 1e-12);
    }

    #[test]
    fn red_most_stable() {
        let sr = make_stats(0.5, 0.001);
        let sg = make_stats(0.4, 0.02);
        let sb = make_stats(0.3, 0.03);
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb);
        assert!((r - 1.0).abs() < 1e-12);
        assert!((g - sr.median / sg.median).abs() < 1e-12);
        assert!((b - sr.median / sb.median).abs() < 1e-12);
    }

    #[test]
    fn green_most_stable() {
        let sr = make_stats(0.5, 0.05);
        let sg = make_stats(0.4, 0.001);
        let sb = make_stats(0.3, 0.03);
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb);
        assert!((r - sg.median / sr.median).abs() < 1e-12);
        assert!((g - 1.0).abs() < 1e-12);
        assert!((b - sg.median / sb.median).abs() < 1e-12);
    }

    #[test]
    fn blue_most_stable() {
        let sr = make_stats(0.5, 0.05);
        let sg = make_stats(0.4, 0.04);
        let sb = make_stats(0.3, 0.001);
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb);
        assert!((r - sb.median / sr.median).abs() < 1e-12);
        assert!((g - sb.median / sg.median).abs() < 1e-12);
        assert!((b - 1.0).abs() < 1e-12);
    }

    #[test]
    fn near_zero_median_handled() {
        let sr = make_stats(0.0, 0.0);
        let sg = make_stats(0.5, 0.01);
        let sb = make_stats(0.3, 0.02);
        let (r, g, b) = select_wb_reference(&sr, &sg, &sb);
        assert!(r.is_finite());
        assert!(g.is_finite());
        assert!(b.is_finite());
    }
}
