use ndarray::{Array2, Zip};

pub use crate::types::image::{ScnrMethod, ScnrConfig};

#[inline(always)]
fn green_limit(r: f32, b: f32, method: ScnrMethod) -> f32 {
    match method {
        ScnrMethod::AverageNeutral => (r + b) * 0.5,
        ScnrMethod::MaximumNeutral => r.max(b),
    }
}

pub fn apply_scnr_inplace(
    r: &Array2<f32>,
    g: &mut Array2<f32>,
    b: &Array2<f32>,
    config: &ScnrConfig,
) {
    assert_eq!(r.dim(), g.dim());
    assert_eq!(g.dim(), b.dim());

    let amount = config.amount.clamp(0.0, 1.0);
    if amount < 1e-7 {
        return;
    }

    let method = config.method;
    let preserve = config.preserve_luminance;

    Zip::from(r).and(g).and(b).par_for_each(|&rv, gv, &bv| {
        let limit = green_limit(rv, bv, method);
        let g_corrected = (*gv).min(limit);

        let g_new = if preserve {
            let lum_before = 0.2126 * rv + 0.7152 * (*gv) + 0.0722 * bv;
            let lum_after = 0.2126 * rv + 0.7152 * g_corrected + 0.0722 * bv;
            let lum_diff = lum_before - lum_after;
            (g_corrected + lum_diff / 0.7152).max(0.0)
        } else {
            g_corrected
        };

        *gv = *gv + amount * (g_new - *gv);
    });
}
