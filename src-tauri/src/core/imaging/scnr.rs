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

    Zip::from(r).and(g).and(b).par_for_each(|rv, gv, bv| {
        let limit = green_limit(*rv, *bv, method);
        let g_corrected = (*gv).min(limit);
        let g_new = *gv + amount * (g_corrected - *gv);

        if preserve {
            let lum_lost = LUM_G * (*gv - g_new);
            if lum_lost > 1e-10 {
                let boost = lum_lost * INV_RB_WEIGHT;
                *rv = (*rv + boost).min(1.0);
                *bv = (*bv + boost).min(1.0);
            }
        }

        *gv = g_new;
    });
}
