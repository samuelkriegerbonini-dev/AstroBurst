use ndarray::{Array2, Zip};

use crate::core::imaging::masked_stretch::MaskedStretchConfig;
use crate::core::imaging::stretch::{finite_min_max, GhsParams};

pub fn arcsinh_factor(factor: f64) -> f32 {
    (factor as f32).clamp(1.0, 500.0)
}

pub fn ghs_params_from_args(
    stretch_factor: f64,
    local_intensity: Option<f64>,
    symmetry_point: Option<f64>,
    shadow_protect: Option<f64>,
    highlight_protect: Option<f64>,
) -> GhsParams {
    let sp = symmetry_point.unwrap_or(0.05).clamp(0.0, 1.0);
    GhsParams {
        d: stretch_factor.clamp(0.0, 50.0),
        b: local_intensity.unwrap_or(0.0).clamp(-10.0, 15.0),
        sp,
        lp: shadow_protect.unwrap_or(0.0).clamp(0.0, sp),
        hp: highlight_protect.unwrap_or(1.0).clamp(sp, 1.0),
    }
}

pub fn clamped_ghs(params: &GhsParams) -> GhsParams {
    ghs_params_from_args(params.d, Some(params.b), Some(params.sp), Some(params.lp), Some(params.hp))
}

#[allow(clippy::too_many_arguments)]
pub fn masked_stretch_config(
    iterations: Option<usize>,
    target_background: Option<f64>,
    mask_growth: Option<f64>,
    mask_softness: Option<f64>,
    protection_amount: Option<f64>,
    luminance_protect: Option<bool>,
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
) -> MaskedStretchConfig {
    MaskedStretchConfig {
        iterations: iterations.unwrap_or(10),
        target_background: target_background.unwrap_or(0.25),
        mask_growth: mask_growth.unwrap_or(2.5),
        mask_softness: mask_softness.unwrap_or(4.0),
        protection_amount: protection_amount.unwrap_or(0.85),
        luminance_protect: luminance_protect.unwrap_or(true),
        detection_sigma: detection_sigma.unwrap_or(8.0).clamp(3.0, 20.0),
        max_eccentricity: max_eccentricity.unwrap_or(0.85).clamp(0.3, 1.0),
        ..MaskedStretchConfig::default()
    }
}

pub fn clamped_masked_stretch(config: &MaskedStretchConfig) -> MaskedStretchConfig {
    MaskedStretchConfig {
        detection_sigma: config.detection_sigma.clamp(3.0, 20.0),
        max_eccentricity: config.max_eccentricity.clamp(0.3, 1.0),
        ..config.clone()
    }
}

fn normalised_with(plane: &Array2<f32>, dmin: f32, dmax: f32) -> Array2<f32> {
    let range = dmax - dmin;
    if range < 1e-10 {
        return Array2::zeros(plane.raw_dim());
    }
    let inv_range = 1.0 / range;
    plane.mapv(|v| if v.is_finite() { ((v - dmin) * inv_range).clamp(0.0, 1.0) } else { 0.0 })
}

fn shared_range(planes: &[&Array2<f32>]) -> (f32, f32) {
    planes
        .iter()
        .map(|p| finite_min_max(p))
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), (mn, mx)| (lo.min(mn), hi.max(mx)))
}

pub fn ghs_blend_base(planes: &[&Array2<f32>]) -> Vec<Array2<f32>> {
    let (dmin, dmax) = shared_range(planes);
    planes.iter().map(|p| normalised_with(p, dmin, dmax)).collect()
}

pub fn arcsinh_blend_base(planes: &[&Array2<f32>]) -> Vec<Array2<f32>> {
    let mut base = ghs_blend_base(planes);
    if let [r, g, b] = planes {
        let mut any_nonfinite = Array2::from_elem(r.raw_dim(), false);
        Zip::from(&mut any_nonfinite)
            .and(*r)
            .and(*g)
            .and(*b)
            .for_each(|flag, &rv, &gv, &bv| *flag = !(rv.is_finite() && gv.is_finite() && bv.is_finite()));
        for plane in &mut base {
            Zip::from(plane).and(&any_nonfinite).for_each(|v, &flag| {
                if flag {
                    *v = 0.0;
                }
            });
        }
    }
    base
}
