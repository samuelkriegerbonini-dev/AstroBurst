use ndarray::{Array2, Zip};

pub const REC709_R: f32 = 0.2126;
pub const REC709_G: f32 = 0.7152;
pub const REC709_B: f32 = 0.0722;
pub const LUMINANCE_RATIO_FLOOR: f32 = 1e-6;

pub fn rgb_to_luminance(r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>) -> Array2<f32> {
    let mut out = Array2::<f32>::zeros(r.dim());
    Zip::from(&mut out)
        .and(r)
        .and(g)
        .and(b)
        .par_for_each(|o, &rv, &gv, &bv| {
            *o = REC709_R * rv + REC709_G * gv + REC709_B * bv;
        });
    out
}

pub fn apply_luminance_ratio(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    old_l: &Array2<f32>,
    new_l: &Array2<f32>,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let mut ratio = Array2::<f32>::zeros(old_l.dim());
    Zip::from(&mut ratio)
        .and(old_l)
        .and(new_l)
        .par_for_each(|o, &ol, &nl| {
            *o = luminance_ratio(ol, nl);
        });
    (
        scale_channel(r, &ratio),
        scale_channel(g, &ratio),
        scale_channel(b, &ratio),
    )
}

fn luminance_ratio(old_l: f32, new_l: f32) -> f32 {
    if !old_l.is_finite() || !new_l.is_finite() {
        return 1.0;
    }
    new_l / old_l.max(LUMINANCE_RATIO_FLOOR)
}

fn scale_channel(channel: &Array2<f32>, ratio: &Array2<f32>) -> Array2<f32> {
    let mut out = Array2::<f32>::zeros(channel.dim());
    Zip::from(&mut out)
        .and(channel)
        .and(ratio)
        .par_for_each(|o, &c, &k| {
            *o = c * k;
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constant_colour(
        rows: usize,
        cols: usize,
        r: f32,
        g: f32,
        b: f32,
    ) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
        (
            Array2::from_elem((rows, cols), r),
            Array2::from_elem((rows, cols), g),
            Array2::from_elem((rows, cols), b),
        )
    }

    #[test]
    fn luminance_uses_rec709_weights() {
        let (r, g, b) = constant_colour(2, 2, 1.0, 0.0, 0.0);
        assert!((rgb_to_luminance(&r, &g, &b)[[0, 0]] - 0.2126).abs() < 1e-6);
        let (r, g, b) = constant_colour(2, 2, 0.0, 1.0, 0.0);
        assert!((rgb_to_luminance(&r, &g, &b)[[1, 1]] - 0.7152).abs() < 1e-6);
        let (r, g, b) = constant_colour(2, 2, 0.0, 0.0, 1.0);
        assert!((rgb_to_luminance(&r, &g, &b)[[0, 1]] - 0.0722).abs() < 1e-6);
        let (r, g, b) = constant_colour(2, 2, 1.0, 1.0, 1.0);
        assert!((rgb_to_luminance(&r, &g, &b)[[1, 0]] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn luminance_ratio_preserves_hue_of_constant_colour_image() {
        let (r, g, b) = constant_colour(4, 4, 0.8, 0.4, 0.2);
        let old_l = rgb_to_luminance(&r, &g, &b);
        let new_l = old_l.mapv(|v| v * 1.5);
        let (nr, ng, nb) = apply_luminance_ratio(&r, &g, &b, &old_l, &new_l);
        for y in 0..4 {
            for x in 0..4 {
                assert!((nr[[y, x]] - 1.2).abs() < 1e-5);
                assert!((ng[[y, x]] / nr[[y, x]] - 0.5).abs() < 1e-5);
                assert!((nb[[y, x]] / nr[[y, x]] - 0.25).abs() < 1e-5);
            }
        }
        let rebuilt = rgb_to_luminance(&nr, &ng, &nb);
        assert!((rebuilt[[2, 2]] - new_l[[2, 2]]).abs() < 1e-5);
    }

    #[test]
    fn luminance_helpers_are_nan_safe() {
        let (mut r, g, b) = constant_colour(3, 3, 0.5, 0.5, 0.5);
        r[[1, 1]] = f32::NAN;
        let old_l = rgb_to_luminance(&r, &g, &b);
        assert!(old_l[[1, 1]].is_nan());
        assert!(old_l[[0, 0]].is_finite());

        let mut new_l = old_l.mapv(|v| v * 2.0);
        new_l[[2, 2]] = f32::NAN;
        let (nr, ng, nb) = apply_luminance_ratio(&r, &g, &b, &old_l, &new_l);
        assert!(nr[[1, 1]].is_nan());
        assert_eq!(ng[[1, 1]], 0.5);
        assert_eq!(nb[[1, 1]], 0.5);
        assert_eq!(nr[[2, 2]], 0.5);
        assert!((nr[[0, 0]] - 1.0).abs() < 1e-6);
        assert!((ng[[0, 2]] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn luminance_ratio_floor_guards_near_black_pixels() {
        let (r, g, b) = constant_colour(1, 1, 0.0, 0.0, 0.0);
        let old_l = rgb_to_luminance(&r, &g, &b);
        let new_l = Array2::from_elem((1, 1), 0.5f32);
        let (nr, _, _) = apply_luminance_ratio(&r, &g, &b, &old_l, &new_l);
        assert!(nr[[0, 0]].is_finite());
        assert_eq!(nr[[0, 0]], 0.0);
    }
}
