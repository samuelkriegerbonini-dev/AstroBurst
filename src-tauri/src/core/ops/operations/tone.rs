use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::imaging::curves::{apply_curve, apply_levels, LevelsParams, SplineLut};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, compute_linked_stf_with_stats, AutoStfConfig, ImageStats, StfParams};

pub const TONE_IDENTITY_ERROR: &str = "Nothing to apply: the levels and the curve are both identity.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneLevelsInput {
    pub black: f64,
    pub gamma: f64,
    pub white: f64,
}

impl From<&ToneLevelsInput> for LevelsParams {
    fn from(input: &ToneLevelsInput) -> Self {
        Self {
            black: input.black,
            gamma: input.gamma,
            white: input.white,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneCurveInput {
    pub points: Vec<[f64; 2]>,
}

pub fn build_spline(input: &ToneCurveInput) -> SplineLut {
    let pts: Vec<(f64, f64)> = input.points.iter().map(|p| (p[0], p[1])).collect();
    SplineLut::from_points(&pts)
}

pub fn is_curve_identity(input: &ToneCurveInput) -> bool {
    let pts: Vec<(f64, f64)> = input.points.iter().map(|p| (p[0], p[1])).collect();
    SplineLut::is_identity(&pts)
}

pub struct ToneOps {
    pub levels: Option<LevelsParams>,
    pub curve: Option<(SplineLut, usize)>,
}

impl ToneOps {
    pub fn parse(levels: Option<&ToneLevelsInput>, curve: Option<&ToneCurveInput>) -> anyhow::Result<Self> {
        let levels = levels.map(LevelsParams::from).filter(|l| !l.is_identity());
        let curve = curve
            .filter(|c| !is_curve_identity(c))
            .map(|c| (build_spline(c), c.points.len()));
        if levels.is_none() && curve.is_none() {
            anyhow::bail!(TONE_IDENTITY_ERROR);
        }
        Ok(Self { levels, curve })
    }

    pub fn apply(&self, display: &Array2<f32>) -> Array2<f32> {
        match (&self.levels, &self.curve) {
            (Some(l), Some((lut, _))) => apply_curve(&apply_levels(display, l), lut),
            (Some(l), None) => apply_levels(display, l),
            (None, Some((lut, _))) => apply_curve(display, lut),
            (None, None) => display.clone(),
        }
    }

    pub fn history(&self) -> String {
        let l = self.levels.clone().unwrap_or_default();
        format!(
            "tone: levels black {:.4} gamma {:.4} white {:.4}; curve {} points",
            l.black,
            l.gamma,
            l.white,
            self.curve.as_ref().map_or(0, |(_, n)| *n)
        )
    }
}

pub fn stf_history(stf: &StfParams) -> String {
    format!(
        "tone: linear input stretched with auto-STF (shadow {:.4} midtone {:.4} highlight {:.4})",
        stf.shadow, stf.midtone, stf.highlight
    )
}

pub fn keep_nonfinite(input: &Array2<f32>, mut toned: Array2<f32>) -> Array2<f32> {
    ndarray::Zip::from(&mut toned).and(input).for_each(|t, &v| {
        if !v.is_finite() {
            *t = f32::NAN;
        }
    });
    toned
}

pub fn linear_input_stf(planes: &[&Array2<f32>]) -> (StfParams, ImageStats) {
    match planes {
        [r, g, b] => {
            let stats = [r, g, b].map(|p| compute_image_stats(p));
            compute_linked_stf_with_stats(&stats[0], &stats[1], &stats[2], &AutoStfConfig::default())
        }
        _ => {
            let stats = compute_image_stats(planes[0]);
            (auto_stf(&stats, &AutoStfConfig::default()), stats)
        }
    }
}

pub struct Toned {
    pub planes: Vec<Array2<f32>>,
    pub stf: Option<(StfParams, ImageStats)>,
}

impl Toned {
    pub fn history_lines(&self, ops: &ToneOps) -> Vec<String> {
        self.stf
            .iter()
            .map(|(stf, _)| stf_history(stf))
            .chain(std::iter::once(ops.history()))
            .collect()
    }
}

pub fn tone_planes(planes: &[&Array2<f32>], display_referred: bool, ops: &ToneOps) -> Toned {
    if display_referred {
        let toned = planes.iter().map(|p| keep_nonfinite(p, ops.apply(p))).collect();
        return Toned { planes: toned, stf: None };
    }
    let (stf, stats) = linear_input_stf(planes);
    let toned = planes
        .iter()
        .map(|p| keep_nonfinite(p, ops.apply(&apply_stf_f32(p, &stf, &stats))))
        .collect();
    Toned { planes: toned, stf: Some((stf, stats)) }
}

pub fn tone_blend_base(planes: &[&Array2<f32>], display_referred: bool) -> Vec<Array2<f32>> {
    if display_referred {
        return planes.iter().map(|p| (*p).clone()).collect();
    }
    let (stf, stats) = linear_input_stf(planes);
    planes.iter().map(|p| keep_nonfinite(p, apply_stf_f32(p, &stf, &stats))).collect()
}
