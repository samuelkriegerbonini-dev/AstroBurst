use ndarray::{Array2, Zip};

pub use crate::types::image::{ScnrMethod, ScnrConfig};

#[inline(always)]
fn green_limit(r: f32, b: f32, method: ScnrMethod) -> f32 {
    match method {
        ScnrMethod::AverageNeutral => (r + b) * 0.5,
        ScnrMethod::MaximumNeutral => r.max(b),
    }
}

const LUM_R: f32 = 0.2126;
const LUM_G: f32 = 0.7152;
const LUM_B: f32 = 0.0722;
const INV_RB_WEIGHT: f32 = 1.0 / (LUM_R + LUM_B);

pub fn apply_scnr_inplace(
    r: &mut Array2<f32>,
    g: &mut Array2<f32>,
    b: &mut Array2<f32>,
    config: &ScnrConfig,
) {
    if r.dim() != g.dim() || g.dim() != b.dim() {
        return;
    }

    let amount = config.amount.clamp(0.0, 1.0);
    if amount < 1e-7 {
        return;
    }

    let method = config.method;
    let preserve = config.preserve_luminance;
    let ceiling = if preserve { white_point(r, g, b) } else { 1.0 };

    Zip::from(r).and(g).and(b).par_for_each(|rv, gv, bv| {
        let limit = green_limit(*rv, *bv, method);
        let g_corrected = (*gv).min(limit);
        let g_new = *gv + amount * (g_corrected - *gv);
        let delta_g = *gv - g_new;

        if preserve && delta_g > 1e-10 && *rv <= ceiling && *bv <= ceiling {
            let lum_lost = LUM_G * delta_g;
            let boost = lum_lost * INV_RB_WEIGHT;
            *rv = (*rv + boost).min(ceiling);
            *bv = (*bv + boost).min(ceiling);
        }

        *gv = g_new;
    });
}

fn white_point(r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>) -> f32 {
    Zip::from(r).and(g).and(b).par_fold(
        || 1.0f32,
        |acc, &rv, &gv, &bv| [rv, gv, bv].into_iter().filter(|v| v.is_finite()).fold(acc, f32::max),
        f32::max,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(amount: f32, preserve: bool, method: ScnrMethod) -> ScnrConfig {
        ScnrConfig { method, amount, preserve_luminance: preserve }
    }

    #[test]
    fn removes_dominant_green_in_range() {
        let mut r = Array2::from_elem((2, 2), 0.3f32);
        let mut g = Array2::from_elem((2, 2), 0.9f32);
        let mut b = Array2::from_elem((2, 2), 0.3f32);
        apply_scnr_inplace(&mut r, &mut g, &mut b, &cfg(1.0, false, ScnrMethod::AverageNeutral));
        assert!((g[[0, 0]] - 0.3).abs() < 1e-5);
        assert!((r[[0, 0]] - 0.3).abs() < 1e-5);
        assert!((b[[0, 0]] - 0.3).abs() < 1e-5);
    }

    #[test]
    fn preserve_skips_saturated_stars() {
        let mut r = Array2::from_elem((1, 1), 2.5f32);
        let mut g = Array2::from_elem((1, 1), 1.8f32);
        let mut b = Array2::from_elem((1, 1), 1.2f32);
        apply_scnr_inplace(&mut r, &mut g, &mut b, &cfg(1.0, true, ScnrMethod::MaximumNeutral));
        assert!((r[[0, 0]] - 2.5).abs() < 1e-5, "R should not be modified: {}", r[[0, 0]]);
        assert!((b[[0, 0]] - 1.2).abs() < 1e-5, "B should not be modified: {}", b[[0, 0]]);
    }

    #[test]
    fn preserve_boosts_low_range_pixel() {
        let mut r = Array2::from_elem((1, 1), 0.2f32);
        let mut g = Array2::from_elem((1, 1), 0.6f32);
        let mut b = Array2::from_elem((1, 1), 0.2f32);
        apply_scnr_inplace(&mut r, &mut g, &mut b, &cfg(1.0, true, ScnrMethod::AverageNeutral));
        assert!(r[[0, 0]] > 0.2);
        assert!(b[[0, 0]] > 0.2);
        assert!((g[[0, 0]] - 0.2).abs() < 1e-5);
    }

    #[test]
    fn preserve_restores_luminance_on_linear_native_unit_data() {
        let row = |v: [f32; 4]| Array2::from_shape_vec((1, 4), v.to_vec()).unwrap();
        let (r0, g0, b0) = (
            row([300.0, 0.9, 1.1, 60000.0]),
            row([900.0, 1.5, 1.8, 60000.0]),
            row([300.0, 0.9, 1.1, 60000.0]),
        );
        let (mut r, mut g, mut b) = (r0.clone(), g0.clone(), b0.clone());
        apply_scnr_inplace(&mut r, &mut g, &mut b, &cfg(1.0, true, ScnrMethod::AverageNeutral));
        let lum = |r: f32, g: f32, b: f32| LUM_R * r + LUM_G * g + LUM_B * b;
        for i in 0..3 {
            assert!((g[[0, i]] - r0[[0, i]]).abs() < 1e-4, "green not neutralized at {}: {}", i, g[[0, i]]);
            let before = lum(r0[[0, i]], g0[[0, i]], b0[[0, i]]);
            let after = lum(r[[0, i]], g[[0, i]], b[[0, i]]);
            assert!((after - before).abs() / before < 1e-4, "pixel {} luminance {} -> {}", i, before, after);
        }
        assert_eq!(r[[0, 3]], 60000.0);
    }

    #[test]
    fn preserve_clamps_display_referred_boost_at_one() {
        let mut r = Array2::from_elem((1, 1), 0.9f32);
        let mut g = Array2::from_elem((1, 1), 1.0f32);
        let mut b = Array2::from_elem((1, 1), 0.9f32);
        apply_scnr_inplace(&mut r, &mut g, &mut b, &cfg(1.0, true, ScnrMethod::AverageNeutral));
        assert_eq!(r[[0, 0]], 1.0);
        assert_eq!(b[[0, 0]], 1.0);
        assert!((g[[0, 0]] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn amount_zero_is_noop() {
        let mut r = Array2::from_elem((1, 1), 0.3f32);
        let mut g = Array2::from_elem((1, 1), 0.9f32);
        let mut b = Array2::from_elem((1, 1), 0.3f32);
        apply_scnr_inplace(&mut r, &mut g, &mut b, &cfg(0.0, true, ScnrMethod::AverageNeutral));
        assert!((g[[0, 0]] - 0.9).abs() < 1e-5);
    }
}
