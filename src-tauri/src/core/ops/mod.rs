pub mod background;
pub mod domain;
pub mod pixelmath;
pub mod psf;
pub mod stretch;
pub mod tone;

use ndarray::Array2;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};

use crate::core::analysis::deconvolution::richardson_lucy;
use crate::core::imaging::background::{extract_background, DebandAxis};
use crate::core::imaging::dbe::extract_background_dbe;
use crate::core::imaging::hdr::{hdrmt_rgb, hdrmt_with_progress, HdrConfig};
use crate::core::imaging::local_contrast::{lhe_rgb, lhe_with_progress, LheConfig};
use crate::core::imaging::masked_stretch::{masked_stretch, MaskedStretchConfig};
use crate::core::imaging::stats::finite_output_stats;
use crate::core::imaging::stretch::{arcsinh_stretch, arcsinh_stretch_rgb, ghs_stretch, ghs_stretch_rgb, GhsParams};
use crate::core::imaging::wavelet::{check_layer_bias, wavelet_denoise, WaveletConfig};
use crate::core::mask::{blend, MaskError};
use crate::core::ops::background::{
    checked_background_config, checked_deband_settings, deband_axis_token, deband_guard, deband_history, deband_single,
    BackgroundModel, DebandRequest,
};
use crate::core::ops::domain::{is_display_referred, output_values_for};
use crate::core::ops::pixelmath::{evaluate_per_channel, evaluate_target, PixelMathSlot, SlotLoader};
use crate::core::ops::psf::{estimated_kernel, DeconvPsf, PsfSpec};
use crate::core::ops::stretch::{arcsinh_blend_base, arcsinh_factor, clamped_ghs, clamped_masked_stretch, ghs_blend_base};
use crate::core::ops::tone::{tone_blend_base, tone_planes, ToneCurveInput, ToneLevelsInput, ToneOps};
use crate::core::pixelmath::OutputOptions;
use crate::infra::cache::ImageEntry;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    RES_AXIS, RES_CONVERGED, RES_CONVERGENCE, RES_CURVES_APPLIED, RES_DBE_REJECTED_COUNT, RES_DBE_SAMPLES,
    RES_FINAL_BACKGROUND, RES_HIGHLIGHT_PROTECT, RES_INPUT_NORMALIZED, RES_ITERATIONS_RUN, RES_LEVELS_APPLIED,
    RES_LOCAL_INTENSITY, RES_MASK_COVERAGE, RES_MAX, RES_MIN, RES_NOISE_ESTIMATE, RES_NON_FINITE_COUNT,
    RES_PSF_SOURCE, RES_RMS_RESIDUAL, RES_SAMPLE_COUNT, RES_SCALES_PROCESSED, RES_SHADOW_PROTECT, RES_STARS_MASKED,
    RES_STATS, RES_STRETCH_FACTOR, RES_SYMMETRY_POINT, RES_WARNINGS,
};
use crate::types::header::HduHeader;
use crate::types::stacking::RLConfig;

pub const OPSPEC_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputValues {
    Linear,
    Rescaled,
    DisplayReferred,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpVersion;

impl Serialize for OpVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(OPSPEC_VERSION)
    }
}

impl<'de> Deserialize<'de> for OpVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = u64::deserialize(deserializer)?;
        if v == u64::from(OPSPEC_VERSION) {
            Ok(OpVersion)
        } else {
            Err(D::Error::custom(format!(
                "unsupported OpSpec version {v}; this build reads version {OPSPEC_VERSION}"
            )))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OpSpec {
    Background {
        v: OpVersion,
        #[serde(flatten)]
        model: BackgroundModel,
    },
    WaveletDenoise {
        v: OpVersion,
        #[serde(flatten)]
        config: WaveletConfig,
    },
    RlDeconv {
        v: OpVersion,
        #[serde(flatten)]
        config: RLConfig,
        psf: PsfSpec,
    },
    Arcsinh {
        v: OpVersion,
        factor: f64,
    },
    Ghs {
        v: OpVersion,
        #[serde(flatten)]
        params: GhsParams,
    },
    MaskedStretch {
        v: OpVersion,
        #[serde(flatten)]
        config: MaskedStretchConfig,
    },
    Lhe {
        v: OpVersion,
        #[serde(flatten)]
        config: LheConfig,
    },
    Hdrmt {
        v: OpVersion,
        #[serde(flatten)]
        config: HdrConfig,
    },
    Tone {
        v: OpVersion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        levels: Option<ToneLevelsInput>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        curve: Option<ToneCurveInput>,
    },
    Pixelmath {
        v: OpVersion,
        expression: String,
        slots: Vec<PixelMathSlot>,
        truncate: bool,
        rescale: bool,
    },
}

impl OpSpec {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Background { .. } => "background",
            Self::WaveletDenoise { .. } => "wavelet_denoise",
            Self::RlDeconv { .. } => "rl_deconv",
            Self::Arcsinh { .. } => "arcsinh",
            Self::Ghs { .. } => "ghs",
            Self::MaskedStretch { .. } => "masked_stretch",
            Self::Lhe { .. } => "lhe",
            Self::Hdrmt { .. } => "hdrmt",
            Self::Tone { .. } => "tone",
            Self::Pixelmath { .. } => "pixelmath",
        }
    }

    fn runs_on_one_plane_only(&self) -> bool {
        matches!(
            self,
            Self::Background { .. } | Self::WaveletDenoise { .. } | Self::RlDeconv { .. } | Self::MaskedStretch { .. }
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("{0} does not take a mask")]
    MaskNotSupported(&'static str),
    #[error(transparent)]
    Mask(#[from] MaskError),
    #[error("{op} cannot run on {planes} planes")]
    PlaneCount { op: &'static str, planes: usize },
    #[error("{op} needs planes of one size; got {dims:?}")]
    PlaneDims { op: &'static str, dims: Vec<(usize, usize)> },
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

#[derive(Clone, Copy, Default)]
pub struct OpCompanions<'a> {
    pub psf_kernel: Option<&'a Array2<f32>>,
    pub load_slot: Option<SlotLoader<'a>>,
}

pub struct OpInput<'a> {
    pub planes: Vec<&'a Array2<f32>>,
    pub header: Option<&'a HduHeader>,
    pub companions: OpCompanions<'a>,
    pub mask: Option<&'a Array2<f32>>,
}

impl<'a> OpInput<'a> {
    pub fn mono(plane: &'a Array2<f32>, header: Option<&'a HduHeader>) -> Self {
        Self { planes: vec![plane], header, companions: OpCompanions::default(), mask: None }
    }

    pub fn rgb(planes: [&'a Array2<f32>; 3], header: Option<&'a HduHeader>) -> Self {
        Self { planes: planes.to_vec(), header, companions: OpCompanions::default(), mask: None }
    }

    pub fn with_companions(self, companions: OpCompanions<'a>) -> Self {
        Self { companions, ..self }
    }

    pub fn with_mask(self, mask: Option<&'a Array2<f32>>) -> Self {
        Self { mask, ..self }
    }

    fn display_referred(&self) -> bool {
        is_display_referred(self.header)
    }
}

#[derive(Debug)]
pub struct OpOutput {
    pub planes: Vec<Array2<f32>>,
    pub model: Option<Array2<f32>>,
    pub history_lines: Vec<String>,
    pub values: OutputValues,
    pub extras: Value,
}

impl OpOutput {
    fn new(planes: Vec<Array2<f32>>, values: OutputValues, extras: Value) -> Self {
        Self { planes, model: None, history_lines: Vec::new(), values, extras }
    }
}

fn check_planes(spec: &OpSpec, input: &OpInput) -> Result<(), OpError> {
    let planes = input.planes.len();
    let allowed = if spec.runs_on_one_plane_only() { planes == 1 } else { planes == 1 || planes == 3 };
    if !allowed {
        return Err(OpError::PlaneCount { op: spec.name(), planes });
    }
    let dims: Vec<(usize, usize)> = input.planes.iter().map(|p| p.dim()).collect();
    if dims.iter().any(|d| *d != dims[0]) {
        return Err(OpError::PlaneDims { op: spec.name(), dims });
    }
    Ok(())
}

pub fn run_op(spec: &OpSpec, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
    check_planes(spec, input)?;
    let Some(mask) = input.mask else {
        return dispatch(spec, input, progress);
    };
    check_mask(spec, input, mask)?;
    if matches!(spec, OpSpec::Pixelmath { .. }) {
        return dispatch(spec, input, progress);
    }
    let base = blend_base(spec, input)?;
    let mut out = dispatch(spec, input, progress)?;
    out.planes = base
        .iter()
        .zip(&out.planes)
        .map(|(base, processed)| blend(base, processed, mask))
        .collect::<Result<_, _>>()?;
    Ok(out)
}

pub fn refuse_mask(spec: &OpSpec) -> Result<(), OpError> {
    if matches!(spec, OpSpec::Background { .. } | OpSpec::MaskedStretch { .. }) {
        return Err(OpError::MaskNotSupported(spec.name()));
    }
    Ok(())
}

fn check_mask(spec: &OpSpec, input: &OpInput, mask: &Array2<f32>) -> Result<(), OpError> {
    refuse_mask(spec)?;
    let image = input.planes[0].dim();
    if mask.dim() != image {
        return Err(MaskError::BlendDims { base: image, processed: image, mask: mask.dim() }.into());
    }
    Ok(())
}

fn dispatch(spec: &OpSpec, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
    match spec {
        OpSpec::Background { model, .. } => run_background(model, input, progress),
        OpSpec::WaveletDenoise { config, .. } => run_wavelet(config, input, progress),
        OpSpec::RlDeconv { config, psf, .. } => run_rl_deconv(config, psf, input, progress),
        OpSpec::Arcsinh { factor, .. } => Ok(run_arcsinh(*factor, input)),
        OpSpec::Ghs { params, .. } => Ok(run_ghs(params, input)),
        OpSpec::MaskedStretch { config, .. } => run_masked_stretch(config, input),
        OpSpec::Lhe { config, .. } => run_lhe(config, input, progress),
        OpSpec::Hdrmt { config, .. } => run_hdrmt(config, input, progress),
        OpSpec::Tone { levels, curve, .. } => run_tone(levels.as_ref(), curve.as_ref(), input),
        OpSpec::Pixelmath { expression, slots, truncate, rescale, .. } => {
            run_pixelmath(expression, slots, OutputOptions { truncate: *truncate, rescale: *rescale }, input)
        }
    }
}

pub fn blend_base(spec: &OpSpec, input: &OpInput) -> Result<Vec<Array2<f32>>, OpError> {
    refuse_mask(spec)?;
    check_planes(spec, input)?;
    let planes = &input.planes;
    Ok(match spec {
        OpSpec::Arcsinh { .. } => arcsinh_blend_base(planes),
        OpSpec::Ghs { .. } => ghs_blend_base(planes),
        OpSpec::Tone { .. } => tone_blend_base(planes, input.display_referred()),
        _ => planes.iter().map(|p| (*p).clone()).collect(),
    })
}

fn per_plane(
    input: &OpInput,
    mono: impl FnOnce(&Array2<f32>) -> anyhow::Result<Array2<f32>>,
    rgb: impl FnOnce(&Array2<f32>, &Array2<f32>, &Array2<f32>) -> anyhow::Result<(Array2<f32>, Array2<f32>, Array2<f32>)>,
) -> anyhow::Result<Vec<Array2<f32>>> {
    match input.planes[..] {
        [r, g, b] => rgb(r, g, b).map(|(r, g, b)| vec![r, g, b]),
        _ => mono(input.planes[0]).map(|plane| vec![plane]),
    }
}

fn run_background(model: &BackgroundModel, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
    let image = input.planes[0];
    let fixed = |axis| DebandRequest::Fixed(axis);
    let (request, settings) = match model {
        BackgroundModel::Polynomial(config) => {
            let bg = extract_background(image, &checked_background_config(config)?, progress)?;
            let extras = json!({ RES_SAMPLE_COUNT: bg.sample_count, RES_RMS_RESIDUAL: bg.rms_residual, RES_AXIS: Value::Null });
            return Ok(OpOutput { model: Some(bg.model), ..OpOutput::new(vec![bg.corrected], OutputValues::Linear, extras) });
        }
        BackgroundModel::Spline(config) => {
            let dbe = extract_background_dbe(image, config, progress)?;
            let extras = json!({
                RES_SAMPLE_COUNT: dbe.sample_count,
                RES_DBE_REJECTED_COUNT: dbe.rejected_count,
                RES_RMS_RESIDUAL: dbe.rms_residual,
                RES_DBE_SAMPLES: serde_json::to_value(&dbe.samples).map_err(anyhow::Error::from)?,
            });
            return Ok(OpOutput { model: Some(dbe.model), ..OpOutput::new(vec![dbe.corrected], OutputValues::Linear, extras) });
        }
        BackgroundModel::DebandRows(s) => (fixed(DebandAxis::Rows), s),
        BackgroundModel::DebandCols(s) => (fixed(DebandAxis::Columns), s),
        BackgroundModel::DebandBoth(s) => (fixed(DebandAxis::Both), s),
        BackgroundModel::DebandAuto(s) => (DebandRequest::Auto, s),
    };
    deband_guard(input.header.into_iter())?;
    let settings = checked_deband_settings(settings)?;
    let out = deband_single(image, request, settings.sigma_clip, settings.iterations);
    if let Some(p) = progress {
        p.emit_complete();
    }
    let history_lines = out.deband.iter().map(|&(axis, detected)| deband_history(axis, detected)).collect();
    let extras = json!({ RES_AXIS: out.deband.map(|(axis, _)| deband_axis_token(axis)) });
    Ok(OpOutput {
        model: Some(out.model),
        history_lines,
        ..OpOutput::new(vec![out.corrected], OutputValues::Linear, extras)
    })
}

fn run_wavelet(config: &WaveletConfig, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
    check_layer_bias(config.layer_bias.as_deref())?;
    let result = wavelet_denoise(input.planes[0], config, progress)?;
    let extras = json!({ RES_SCALES_PROCESSED: result.scales_processed, RES_NOISE_ESTIMATE: result.noise_estimate });
    let values = output_values_for(input.header, OutputValues::Linear);
    Ok(OpOutput::new(vec![result.denoised], values, extras))
}

fn run_rl_deconv(config: &RLConfig, psf: &PsfSpec, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
    let image = input.planes[0];
    let psf = DeconvPsf::from_spec(psf, input.companions.psf_kernel)?;
    let kernel = psf.kernel(|c| estimated_kernel(image, c))?;
    let result = richardson_lucy(image, &kernel, config, progress)?;
    if let Some(p) = progress {
        p.emit_complete();
    }
    let extras = json!({
        RES_ITERATIONS_RUN: result.iterations_run,
        RES_CONVERGENCE: result.convergence,
        RES_PSF_SOURCE: psf.source(),
    });
    let values = output_values_for(input.header, OutputValues::Linear);
    Ok(OpOutput::new(vec![result.image], values, extras))
}

fn run_arcsinh(factor: f64, input: &OpInput) -> OpOutput {
    let factor = arcsinh_factor(factor);
    let planes = match input.planes[..] {
        [r, g, b] => {
            let (r, g, b) = arcsinh_stretch_rgb(r, g, b, factor);
            vec![r, g, b]
        }
        _ => vec![arcsinh_stretch(input.planes[0], factor)],
    };
    OpOutput::new(planes, OutputValues::DisplayReferred, json!({ RES_STRETCH_FACTOR: factor }))
}

fn run_ghs(params: &GhsParams, input: &OpInput) -> OpOutput {
    let params = clamped_ghs(params);
    let planes = match input.planes[..] {
        [r, g, b] => {
            let (r, g, b) = ghs_stretch_rgb(r, g, b, &params);
            vec![r, g, b]
        }
        _ => vec![ghs_stretch(input.planes[0], &params)],
    };
    let extras = json!({
        RES_STRETCH_FACTOR: params.d,
        RES_LOCAL_INTENSITY: params.b,
        RES_SYMMETRY_POINT: params.sp,
        RES_SHADOW_PROTECT: params.lp,
        RES_HIGHLIGHT_PROTECT: params.hp,
    });
    OpOutput::new(planes, OutputValues::DisplayReferred, extras)
}

fn run_masked_stretch(config: &MaskedStretchConfig, input: &OpInput) -> Result<OpOutput, OpError> {
    let result = masked_stretch(input.planes[0], &clamped_masked_stretch(config)).map_err(|e| anyhow::anyhow!(e))?;
    let extras = json!({
        RES_ITERATIONS_RUN: result.iterations_run,
        RES_FINAL_BACKGROUND: result.final_background,
        RES_STARS_MASKED: result.stars_masked,
        RES_MASK_COVERAGE: result.mask_coverage,
        RES_CONVERGED: result.converged,
    });
    Ok(OpOutput::new(vec![result.image], OutputValues::DisplayReferred, extras))
}

fn run_lhe(config: &LheConfig, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
    let planes = per_plane(
        input,
        |p| lhe_with_progress(p, config, progress).map(|r| r.image),
        |r, g, b| lhe_rgb(r, g, b, config),
    )?;
    Ok(OpOutput::new(planes, output_values_for(input.header, OutputValues::Rescaled), json!({})))
}

fn run_hdrmt(config: &HdrConfig, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
    let planes = per_plane(input, |p| hdrmt_with_progress(p, config, progress), |r, g, b| hdrmt_rgb(r, g, b, config))?;
    Ok(OpOutput::new(planes, output_values_for(input.header, OutputValues::Rescaled), json!({})))
}

fn run_pixelmath(expression: &str, slots: &[PixelMathSlot], opts: OutputOptions, input: &OpInput) -> Result<OpOutput, OpError> {
    let unsupplied = |key: &str| -> anyhow::Result<ImageEntry> { anyhow::bail!("no slot loader was supplied to read {key}") };
    let load = input.companions.load_slot.unwrap_or(&unsupplied);
    let (planes, extras) = match input.planes[..] {
        [r, g, b] => {
            let evaluated = evaluate_per_channel(expression, slots, &opts, [r, g, b], load, input.mask)?;
            let extras = json!({
                RES_STATS: evaluated.stats,
                RES_NON_FINITE_COUNT: evaluated.non_finite,
                RES_WARNINGS: evaluated.warnings,
            });
            (Vec::from(evaluated.results), extras)
        }
        _ => {
            let evaluated = evaluate_target(expression, slots, &opts, input.planes[0], load, input.mask)?;
            let (stats, non_finite) = finite_output_stats(&evaluated.result);
            let extras = json!({ RES_STATS: stats, RES_NON_FINITE_COUNT: non_finite, RES_WARNINGS: evaluated.warnings });
            (vec![evaluated.result], extras)
        }
    };
    Ok(OpOutput::new(planes, OutputValues::Rescaled, extras))
}

fn run_tone(levels: Option<&ToneLevelsInput>, curve: Option<&ToneCurveInput>, input: &OpInput) -> Result<OpOutput, OpError> {
    let ops = ToneOps::parse(levels, curve)?;
    let toned = tone_planes(&input.planes, input.display_referred(), &ops);
    let normalized = toned.stf.as_ref().map_or(Value::Null, |(_, s)| json!({ RES_MIN: s.min, RES_MAX: s.max }));
    let extras = json!({
        RES_INPUT_NORMALIZED: normalized,
        RES_LEVELS_APPLIED: ops.levels.is_some(),
        RES_CURVES_APPLIED: ops.curve.is_some(),
    });
    let history_lines = toned.history_lines(&ops);
    Ok(OpOutput { history_lines, ..OpOutput::new(toned.planes, OutputValues::DisplayReferred, extras) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: [(&str, &str); 10] = [
        ("background", include_str!("golden/background.json")),
        ("wavelet_denoise", include_str!("golden/wavelet_denoise.json")),
        ("rl_deconv", include_str!("golden/rl_deconv.json")),
        ("arcsinh", include_str!("golden/arcsinh.json")),
        ("ghs", include_str!("golden/ghs.json")),
        ("masked_stretch", include_str!("golden/masked_stretch.json")),
        ("lhe", include_str!("golden/lhe.json")),
        ("hdrmt", include_str!("golden/hdrmt.json")),
        ("tone", include_str!("golden/tone.json")),
        ("pixelmath", include_str!("golden/pixelmath.json")),
    ];

    fn unix(text: &str) -> String {
        text.replace("\r\n", "\n").trim_end().to_string()
    }

    #[test]
    fn golden_json_round_trips_every_variant() {
        for (name, text) in GOLDEN {
            let golden = unix(text);
            let specs: Vec<OpSpec> = serde_json::from_str(&golden).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!specs.is_empty(), "{name}");
            assert!(specs.iter().all(|s| s.name() == name), "{name}: {specs:?}");
            assert_eq!(serde_json::to_string_pretty(&specs).unwrap(), golden, "{name} did not re-serialise byte-identically");
        }
    }

    #[test]
    fn golden_json_with_version_2_is_refused_by_name() {
        for (name, text) in GOLDEN {
            let bumped = unix(text).replace("\"v\": 1", "\"v\": 2");
            let err = serde_json::from_str::<Vec<OpSpec>>(&bumped).expect_err(name).to_string();
            assert!(err.contains("unsupported OpSpec version 2"), "{name}: {err}");
        }
    }

    #[test]
    fn background_golden_covers_every_model_including_deband_auto() {
        let specs: Vec<OpSpec> = serde_json::from_str(GOLDEN[0].1).unwrap();
        let models: Vec<&str> = specs
            .iter()
            .map(|s| match s {
                OpSpec::Background { model, .. } => model.name(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(models, ["polynomial", "spline", "deband_rows", "deband_cols", "deband_both", "deband_auto"]);
    }

    use std::sync::Arc;

    use ndarray::Zip;

    use crate::core::analysis::deconvolution::{generate_gaussian_psf, provided_psf_kernel, richardson_lucy};
    use crate::core::imaging::background::{deband, detect_band_axis, extract_background, BackgroundConfig, DebandAxis, DebandConfig};
    use crate::core::imaging::dbe::{extract_background_dbe, DbeConfig};
    use crate::core::imaging::hdr::{hdrmt_rgb, hdrmt_with_progress};
    use crate::core::imaging::local_contrast::{lhe_rgb, lhe_with_progress};
    use crate::core::imaging::masked_stretch::masked_stretch;
    use crate::core::imaging::stats::compute_image_stats;
    use crate::core::imaging::stretch::{arcsinh_stretch, arcsinh_stretch_rgb, ghs_stretch, ghs_stretch_rgb};
    use crate::core::imaging::wavelet::wavelet_denoise;
    use crate::core::mask::blend;
    use crate::core::ops::background::DebandSettings;
    use crate::core::pixelmath::{compile, evaluate, evaluate_unfinished, finish_plane, OutputOptions};
    use crate::infra::cache::ImageCache;
    use crate::types::constants::{
        RES_AXIS, RES_CONVERGENCE, RES_ITERATIONS_RUN, RES_NOISE_ESTIMATE, RES_PSF_SOURCE, RES_SAMPLE_COUNT,
        RES_SCALES_PROCESSED, RES_STARS_MASKED, RES_STRETCH_FACTOR, RES_WARNINGS,
    };

    const SIDE: usize = 256;

    struct Lcg(u64);

    impl Lcg {
        fn uniform(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }

        fn gaussian(&mut self) -> f32 {
            let s: f32 = (0..4).map(|_| self.uniform()).sum();
            (s - 2.0) * 3.0f32.sqrt()
        }
    }

    fn seeded_frame(seed: u64, scale: f32) -> Array2<f32> {
        let mut rng = Lcg(seed);
        let mut frame = Array2::from_shape_fn((SIDE, SIDE), |(y, x)| {
            0.2 + 0.0004 * x as f32 + 0.0002 * y as f32 + 0.01 * rng.gaussian()
        });
        let stars: Vec<(f32, f32, f32)> = (0..24)
            .map(|_| (40.0 + 176.0 * rng.uniform(), 40.0 + 176.0 * rng.uniform(), 0.3 + 0.6 * rng.uniform()))
            .collect();
        for ((y, x), v) in frame.indexed_iter_mut() {
            for &(cy, cx, peak) in &stars {
                let d2 = (y as f32 - cy).powi(2) + (x as f32 - cx).powi(2);
                *v += peak * (-d2 / (2.0 * 1.6 * 1.6)).exp();
            }
        }
        frame.mapv_inplace(|v| v * scale);
        frame
    }

    fn banded_frame(seed: u64) -> Array2<f32> {
        let mut frame = seeded_frame(seed, 1000.0);
        for ((y, _), v) in frame.indexed_iter_mut() {
            *v += if y % 7 < 2 { 40.0 } else { 0.0 };
        }
        frame
    }

    fn assert_bitwise(actual: &Array2<f32>, expected: &Array2<f32>, what: &str) {
        assert_eq!(actual.dim(), expected.dim(), "{what}");
        let differing = actual.iter().zip(expected.iter()).filter(|(a, e)| a.to_bits() != e.to_bits()).count();
        assert_eq!(differing, 0, "{what}: {differing} pixels differ");
    }

    fn run_mono(spec: &OpSpec, frame: &Array2<f32>) -> OpOutput {
        run_op(spec, &OpInput::mono(frame, None), None).unwrap_or_else(|e| panic!("{}: {e:#}", spec.name()))
    }

    fn run_rgb(spec: &OpSpec, planes: [&Array2<f32>; 3]) -> OpOutput {
        run_op(spec, &OpInput::rgb(planes, None), None).unwrap_or_else(|e| panic!("{}: {e:#}", spec.name()))
    }

    fn rgb_frames() -> [Array2<f32>; 3] {
        [seeded_frame(21, 900.0), seeded_frame(22, 1400.0), seeded_frame(23, 500.0)]
    }

    #[test]
    fn run_op_arcsinh_matches_arcsinh_stretch_on_one_and_three_planes() {
        let frame = seeded_frame(1, 3000.0);
        let spec = OpSpec::Arcsinh { v: OpVersion, factor: 50.0 };
        let ran = run_mono(&spec, &frame);
        assert_bitwise(&ran.planes[0], &arcsinh_stretch(&frame, 50.0), "arcsinh mono");
        assert_eq!(ran.values, OutputValues::DisplayReferred);

        let [r, g, b] = rgb_frames();
        let ran = run_rgb(&spec, [&r, &g, &b]);
        let (er, eg, eb) = arcsinh_stretch_rgb(&r, &g, &b, 50.0);
        for (plane, expected) in ran.planes.iter().zip([er, eg, eb]) {
            assert_bitwise(plane, &expected, "arcsinh rgb");
        }

        let clamped = run_mono(&OpSpec::Arcsinh { v: OpVersion, factor: 4000.0 }, &frame);
        assert_bitwise(&clamped.planes[0], &arcsinh_stretch(&frame, 500.0), "arcsinh clamp");
        assert_eq!(clamped.extras[RES_STRETCH_FACTOR], json!(500.0));
    }

    #[test]
    fn run_op_ghs_matches_ghs_stretch_on_one_and_three_planes() {
        let frame = seeded_frame(2, 3000.0);
        let params = GhsParams { d: 5.0, b: 2.0, sp: 0.05, lp: 0.0, hp: 1.0 };
        let spec = OpSpec::Ghs { v: OpVersion, params };
        assert_bitwise(&run_mono(&spec, &frame).planes[0], &ghs_stretch(&frame, &params), "ghs mono");

        let [r, g, b] = rgb_frames();
        let ran = run_rgb(&spec, [&r, &g, &b]);
        let (er, eg, eb) = ghs_stretch_rgb(&r, &g, &b, &params);
        for (plane, expected) in ran.planes.iter().zip([er, eg, eb]) {
            assert_bitwise(plane, &expected, "ghs rgb");
        }

        let wild = GhsParams { d: 80.0, b: -40.0, sp: 0.4, lp: 0.9, hp: 0.1 };
        let clamped = run_mono(&OpSpec::Ghs { v: OpVersion, params: wild }, &frame);
        let expected = ghs_stretch(&frame, &stretch::ghs_params_from_args(80.0, Some(-40.0), Some(0.4), Some(0.9), Some(0.1)));
        assert_bitwise(&clamped.planes[0], &expected, "ghs clamp");
    }

    #[test]
    fn run_op_wavelet_denoise_matches_wavelet_denoise() {
        let frame = seeded_frame(3, 1000.0);
        let config = WaveletConfig { layer_bias: Some(vec![1.2, 1.0, 1.0, 0.9, 1.0]), ..WaveletConfig::default() };
        let ran = run_mono(&OpSpec::WaveletDenoise { v: OpVersion, config: config.clone() }, &frame);
        let expected = wavelet_denoise(&frame, &config, None).unwrap();
        assert_bitwise(&ran.planes[0], &expected.denoised, "wavelet");
        assert_eq!(ran.extras[RES_SCALES_PROCESSED], json!(expected.scales_processed));
        assert_eq!(ran.extras[RES_NOISE_ESTIMATE], json!(expected.noise_estimate));
        assert_eq!(ran.values, OutputValues::Linear);

        let bad = WaveletConfig { layer_bias: Some(vec![f32::NAN]), ..WaveletConfig::default() };
        let err = run_op(&OpSpec::WaveletDenoise { v: OpVersion, config: bad }, &OpInput::mono(&frame, None), None)
            .expect_err("a non-finite layer bias must be refused")
            .to_string();
        assert!(err.contains("layer_bias"), "{err}");
    }

    #[test]
    fn run_op_rl_deconv_matches_richardson_lucy_for_gaussian_and_supplied_kernels() {
        let frame = seeded_frame(4, 1000.0);
        let config = RLConfig { iterations: 5, ..RLConfig::default() };
        let gaussian = OpSpec::RlDeconv { v: OpVersion, config: config.clone(), psf: PsfSpec::Gaussian { sigma: 2.0, size: 15 } };
        let ran = run_mono(&gaussian, &frame);
        let expected = richardson_lucy(&frame, &generate_gaussian_psf(15, 2.0), &config, None).unwrap();
        assert_bitwise(&ran.planes[0], &expected.image, "rl gaussian");
        assert_eq!(ran.extras[RES_ITERATIONS_RUN], json!(expected.iterations_run));
        assert_eq!(ran.extras[RES_CONVERGENCE], json!(expected.convergence));
        assert_eq!(ran.extras[RES_PSF_SOURCE], json!("gaussian"));

        let rows = vec![vec![1.0, 2.0, 1.0], vec![2.0, 4.0, 2.0], vec![1.0, 2.0, 1.0]];
        let supplied = Array2::from_shape_fn((3, 3), |(y, x)| rows[y][x]);
        let kernel = OpSpec::RlDeconv { v: OpVersion, config: config.clone(), psf: PsfSpec::Kernel { hash: "abc".into(), rows: 3, cols: 3 } };
        let companions = OpCompanions { psf_kernel: Some(&supplied), ..OpCompanions::default() };
        let ran = run_op(&kernel, &OpInput::mono(&frame, None).with_companions(companions), None).unwrap();
        let expected = richardson_lucy(&frame, &provided_psf_kernel(&rows).unwrap(), &config, None).unwrap();
        assert_bitwise(&ran.planes[0], &expected.image, "rl supplied kernel");
        assert_eq!(ran.extras[RES_PSF_SOURCE], json!("provided"));

        let err = run_op(&kernel, &OpInput::mono(&frame, None), None).expect_err("a kernel op needs its kernel").to_string();
        assert!(err.contains("abc") && err.contains("not supplied"), "{err}");
    }

    #[test]
    fn run_op_lhe_matches_lhe_on_one_and_three_planes() {
        let frame = seeded_frame(5, 1.0);
        let config = LheConfig { kernel_radius: 16, ..LheConfig::default() };
        let spec = OpSpec::Lhe { v: OpVersion, config: config.clone() };
        let ran = run_mono(&spec, &frame);
        assert_bitwise(&ran.planes[0], &lhe_with_progress(&frame, &config, None).unwrap().image, "lhe mono");
        assert_eq!(ran.values, OutputValues::Rescaled);

        let [r, g, b] = rgb_frames().map(|p| p.mapv(|v| v / 1500.0));
        let ran = run_rgb(&spec, [&r, &g, &b]);
        let (er, eg, eb) = lhe_rgb(&r, &g, &b, &config).unwrap();
        for (plane, expected) in ran.planes.iter().zip([er, eg, eb]) {
            assert_bitwise(plane, &expected, "lhe rgb");
        }
    }

    #[test]
    fn run_op_hdrmt_matches_hdrmt_on_one_and_three_planes() {
        let frame = seeded_frame(6, 1.0);
        let config = HdrConfig { layers: 3, ..HdrConfig::default() };
        let spec = OpSpec::Hdrmt { v: OpVersion, config: config.clone() };
        assert_bitwise(&run_mono(&spec, &frame).planes[0], &hdrmt_with_progress(&frame, &config, None).unwrap(), "hdrmt mono");

        let [r, g, b] = rgb_frames().map(|p| p.mapv(|v| v / 1500.0));
        let ran = run_rgb(&spec, [&r, &g, &b]);
        let (er, eg, eb) = hdrmt_rgb(&r, &g, &b, &config).unwrap();
        for (plane, expected) in ran.planes.iter().zip([er, eg, eb]) {
            assert_bitwise(plane, &expected, "hdrmt rgb");
        }
    }

    #[test]
    fn run_op_masked_stretch_matches_masked_stretch_and_refuses_three_planes() {
        let frame = seeded_frame(7, 1000.0);
        let config = MaskedStretchConfig { iterations: 4, detection_sigma: 40.0, ..MaskedStretchConfig::default() };
        let spec = OpSpec::MaskedStretch { v: OpVersion, config: config.clone() };
        let ran = run_mono(&spec, &frame);
        let expected = masked_stretch(&frame, &stretch::clamped_masked_stretch(&config)).unwrap();
        assert_bitwise(&ran.planes[0], &expected.image, "masked stretch");
        assert_eq!(ran.extras[RES_STARS_MASKED], json!(expected.stars_masked));
        assert_eq!(ran.values, OutputValues::DisplayReferred);

        let [r, g, b] = rgb_frames();
        let err = run_op(&spec, &OpInput::rgb([&r, &g, &b], None), None).expect_err("masked stretch is one plane in v1");
        assert!(matches!(err, OpError::PlaneCount { op: "masked_stretch", planes: 3 }), "{err}");
    }

    #[test]
    fn run_op_background_polynomial_matches_extract_background() {
        let frame = seeded_frame(8, 1000.0);
        let config = BackgroundConfig { grid_size: 6, poly_degree: 2, ..BackgroundConfig::default() };
        let spec = OpSpec::Background { v: OpVersion, model: BackgroundModel::Polynomial(config.clone()) };
        let ran = run_mono(&spec, &frame);
        let expected = extract_background(&frame, &config, None).unwrap();
        assert_bitwise(&ran.planes[0], &expected.corrected, "polynomial corrected");
        assert_bitwise(ran.model.as_ref().unwrap(), &expected.model, "polynomial model");
        assert_eq!(ran.extras[RES_SAMPLE_COUNT], json!(expected.sample_count));
        assert_eq!(ran.values, OutputValues::Linear);
    }

    #[test]
    fn run_op_background_spline_matches_extract_background_dbe() {
        let frame = seeded_frame(9, 1000.0);
        let config = DbeConfig { auto_grid: Some(4), ..DbeConfig::default() };
        let spec = OpSpec::Background { v: OpVersion, model: BackgroundModel::Spline(config.clone()) };
        let ran = run_mono(&spec, &frame);
        let expected = extract_background_dbe(&frame, &config, None).unwrap();
        assert_bitwise(&ran.planes[0], &expected.corrected, "dbe corrected");
        assert_bitwise(ran.model.as_ref().unwrap(), &expected.model, "dbe model");
        assert_eq!(ran.extras[RES_SAMPLE_COUNT], json!(expected.sample_count));
    }

    #[test]
    fn run_op_background_deband_fixed_matches_deband() {
        let frame = banded_frame(10);
        let settings = DebandSettings { sigma_clip: 3.0, iterations: 3 };
        let ran = run_mono(&OpSpec::Background { v: OpVersion, model: BackgroundModel::DebandRows(settings) }, &frame);
        let expected = deband(&frame, &DebandConfig { axis: DebandAxis::Rows, sigma_clip: 3.0, iterations: 3 });
        assert_bitwise(&ran.planes[0], &expected, "deband rows");
        assert_eq!(ran.extras[RES_AXIS], json!("rows"));
        assert_eq!(ran.history_lines, ["deband rows"]);
        assert!(ran.model.is_some());

        let mut header = HduHeader::empty();
        header.set("EXP_TYPE", "'NRS_FIXEDSLIT'".to_string());
        let spec = OpSpec::Background { v: OpVersion, model: BackgroundModel::DebandBoth(DebandSettings { sigma_clip: 3.0, iterations: 3 }) };
        let err = run_op(&spec, &OpInput::mono(&frame, Some(&header)), None).expect_err("dispersed data must be refused").to_string();
        assert!(err.contains("dispersed exposure"), "{err}");
    }

    #[test]
    fn run_op_background_deband_auto_matches_detected_axis_then_deband() {
        let frame = banded_frame(11);
        let settings = DebandSettings { sigma_clip: 3.0, iterations: 3 };
        let ran = run_mono(&OpSpec::Background { v: OpVersion, model: BackgroundModel::DebandAuto(settings) }, &frame);
        let base = DebandConfig { axis: DebandAxis::Both, sigma_clip: 3.0, iterations: 3 };
        let axis = detect_band_axis(&frame, &base);
        let expected = deband(&frame, &DebandConfig { axis, ..base });
        assert_bitwise(&ran.planes[0], &expected, "deband auto");
        assert_eq!(ran.history_lines, [background::deband_history(axis, true)]);
        assert!(ran.history_lines[0].ends_with("(auto)"));
    }

    #[test]
    fn run_op_pixelmath_matches_the_evaluator_on_one_and_three_planes() {
        let frame = seeded_frame(12, 1000.0);
        let ha = seeded_frame(13, 400.0);
        let cache = ImageCache::new(4, 1 << 30);
        cache.insert_synthetic("ha", Arc::new(ha.clone()), compute_image_stats(&ha));
        let load = |key: &str| cache.get(key).ok_or_else(|| anyhow::anyhow!("{key} is not loaded"));
        let companions = OpCompanions { load_slot: Some(&load), ..OpCompanions::default() };
        let slot = PixelMathSlot { name: "Ha".into(), path: "ha".into() };
        let spec = OpSpec::Pixelmath {
            v: OpVersion,
            expression: "($T - med($T)) * 2 + Ha".into(),
            slots: vec![slot],
            truncate: false,
            rescale: true,
        };
        let opts = OutputOptions { truncate: false, rescale: true };
        let program = compile("($T - med($T)) * 2 + Ha", &["$T".to_string(), "Ha".to_string()]).unwrap();

        let ran = run_op(&spec, &OpInput::mono(&frame, None).with_companions(companions), None).unwrap();
        assert_bitwise(&ran.planes[0], &evaluate(&program, &[&frame, &ha], &opts).unwrap(), "pixelmath mono");
        assert_eq!(ran.values, OutputValues::Rescaled);
        assert!(ran.extras[RES_WARNINGS].is_array());

        let [r, g, b] = rgb_frames();
        let ran = run_op(&spec, &OpInput::rgb([&r, &g, &b], None).with_companions(companions), None).unwrap();
        for (plane, channel) in ran.planes.iter().zip([&r, &g, &b]) {
            assert_bitwise(plane, &evaluate(&program, &[channel, &ha], &opts).unwrap(), "pixelmath rgb");
        }

        let err = run_op(&spec, &OpInput::mono(&frame, None), None).expect_err("slots need a loader").to_string();
        assert!(err.contains("Ha"), "{err}");
    }

    fn golden_specs() -> Vec<OpSpec> {
        GOLDEN
            .iter()
            .flat_map(|(_, text)| serde_json::from_str::<Vec<OpSpec>>(text).unwrap())
            .collect()
    }

    #[test]
    fn blend_base_is_defined_for_every_mask_capable_variant() {
        let frame = seeded_frame(14, 1000.0);
        let [r, g, b] = rgb_frames();
        let specs = golden_specs();
        assert_eq!(specs.iter().map(OpSpec::name).collect::<std::collections::BTreeSet<_>>().len(), 10);
        for spec in &specs {
            let mono = blend_base(spec, &OpInput::mono(&frame, None));
            match spec {
                OpSpec::Background { .. } | OpSpec::MaskedStretch { .. } => {
                    let err = mono.expect_err(spec.name());
                    assert!(matches!(err, OpError::MaskNotSupported(name) if name == spec.name()), "{err}");
                    assert_eq!(err.to_string(), format!("{} does not take a mask", spec.name()));
                }
                _ => {
                    let base = mono.unwrap_or_else(|e| panic!("{}: {e}", spec.name()));
                    assert_eq!(base.len(), 1, "{}", spec.name());
                    assert_eq!(base[0].dim(), frame.dim(), "{}", spec.name());
                }
            }
            if !spec.runs_on_one_plane_only() {
                let base = blend_base(spec, &OpInput::rgb([&r, &g, &b], None)).unwrap_or_else(|e| panic!("{}: {e}", spec.name()));
                assert_eq!(base.len(), 3, "{}", spec.name());
            }
        }
        for spec in specs.iter().filter(|s| matches!(s, OpSpec::WaveletDenoise { .. } | OpSpec::RlDeconv { .. } | OpSpec::Lhe { .. } | OpSpec::Hdrmt { .. } | OpSpec::Pixelmath { .. })) {
            assert_bitwise(&blend_base(spec, &OpInput::mono(&frame, None)).unwrap()[0], &frame, spec.name());
        }
    }

    fn finite_range(planes: &[&Array2<f32>]) -> (f32, f32) {
        planes
            .iter()
            .flat_map(|p| p.iter().copied())
            .filter(|v| v.is_finite())
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
    }

    #[test]
    fn blend_base_arcsinh_is_the_ops_normalisation() {
        let mut frame = seeded_frame(15, 3000.0);
        frame[[3, 4]] = f32::NAN;
        frame[[9, 9]] = f32::INFINITY;
        let factor = 50.0f32;
        let spec = OpSpec::Arcsinh { v: OpVersion, factor: factor as f64 };
        let base = blend_base(&spec, &OpInput::mono(&frame, None)).unwrap().remove(0);

        let (lo, hi) = finite_range(&[&frame]);
        let inv = 1.0 / (hi - lo);
        let expected = frame.mapv(|v| if v.is_finite() { ((v - lo) * inv).clamp(0.0, 1.0) } else { 0.0 });
        assert_bitwise(&base, &expected, "arcsinh base");
        assert_eq!(base[[3, 4]], 0.0);
        assert_eq!(base[[9, 9]], 0.0);
        assert_eq!(base.iter().copied().fold(f32::MIN, f32::max), 1.0);

        let stretched = run_mono(&spec, &frame).planes.remove(0);
        let inv_denom = 1.0 / factor.asinh();
        let from_base = base.mapv(|n| (n * factor).asinh() * inv_denom);
        assert_bitwise(&stretched, &from_base, "arcsinh is asinh of its own blend base");
    }

    #[test]
    fn blend_base_arcsinh_rgb_uses_the_shared_range() {
        let r = Array2::from_shape_fn((32, 32), |(y, x)| ((y * 32 + x) % 101) as f32);
        let mut g = Array2::from_shape_fn((32, 32), |(y, x)| 50.0 + ((y * 32 + x) % 951) as f32);
        let b = Array2::from_shape_fn((32, 32), |(y, x)| 10.0 + ((y + x) % 11) as f32);
        g[[5, 6]] = f32::NAN;
        let spec = OpSpec::Arcsinh { v: OpVersion, factor: 20.0 };
        let base = blend_base(&spec, &OpInput::rgb([&r, &g, &b], None)).unwrap();

        let (lo, hi) = finite_range(&[&r, &g, &b]);
        assert_eq!((lo, hi), (0.0, 1000.0));
        let inv = 1.0 / (hi - lo);
        for (plane, (source, name)) in base.iter().zip([(&r, "r"), (&g, "g"), (&b, "b")]) {
            let expected = Array2::from_shape_fn(source.dim(), |ix| {
                if [&r, &g, &b].iter().all(|p| p[ix].is_finite()) { ((source[ix] - lo) * inv).clamp(0.0, 1.0) } else { 0.0 }
            });
            assert_bitwise(plane, &expected, name);
            assert_eq!(plane[[5, 6]], 0.0, "{name}: a NaN in one channel zeroes all three");
        }
        assert!(base[0].iter().copied().fold(f32::MIN, f32::max) <= 0.1 + f32::EPSILON, "r was normalised by its own range");
        assert!(base[2].iter().copied().fold(f32::MIN, f32::max) <= 0.02 + f32::EPSILON, "b was normalised by its own range");
    }

    #[test]
    fn blend_base_tone_is_the_auto_stf_on_linear_input_and_the_input_when_display_referred() {
        let mut frame = seeded_frame(16, 3000.0);
        frame[[1, 2]] = f32::NAN;
        let spec = OpSpec::Tone { v: OpVersion, levels: Some(ToneLevelsInput { black: 0.1, gamma: 1.0, white: 0.9 }), curve: None };
        let base = blend_base(&spec, &OpInput::mono(&frame, None)).unwrap().remove(0);
        let (stf, stats) = tone::linear_input_stf(&[&frame]);
        let expected = tone::keep_nonfinite(&frame, crate::core::imaging::stf::apply_stf_f32(&frame, &stf, &stats));
        assert_bitwise(&base, &expected, "tone base on linear input");
        assert!(base[[1, 2]].is_nan());

        let mut header = HduHeader::empty();
        header.set(crate::types::constants::HEADER_DISPLAY_REFERRED_CARD, "T".to_string());
        let display = frame.mapv(|v| v / 3000.0);
        let base = blend_base(&spec, &OpInput::mono(&display, Some(&header))).unwrap().remove(0);
        assert_bitwise(&base, &display, "tone base on display-referred input");
    }

    const LINEAR_PEAK: f32 = 3.2e4;

    fn linear_frame(seed: u64) -> Array2<f32> {
        let frame = seeded_frame(seed, 1.0);
        let peak = frame.iter().copied().fold(f32::MIN, f32::max);
        frame.mapv(|v| v / peak * LINEAR_PEAK)
    }

    fn banded_mask(dims: (usize, usize)) -> Array2<f32> {
        let third = dims.1 / 3;
        Array2::from_shape_fn(dims, |(_, x)| match x / third {
            0 => 0.0,
            1 => 0.5,
            _ => 1.0,
        })
    }

    fn run_masked(spec: &OpSpec, input: OpInput, mask: &Array2<f32>) -> OpOutput {
        run_op(spec, &input.with_mask(Some(mask)), None).unwrap_or_else(|e| panic!("masked {}: {e:#}", spec.name()))
    }

    fn assert_in_unit_range(planes: &[Array2<f32>], what: &str) {
        for (index, plane) in planes.iter().enumerate() {
            let outside = plane.iter().filter(|v| !(0.0..=1.0).contains(*v)).count();
            assert_eq!(outside, 0, "{what} plane {index}: {outside} pixels outside [0, 1]");
        }
    }

    fn assert_banded_blend(masked: &Array2<f32>, base: &Array2<f32>, unmasked: &Array2<f32>, mask: &Array2<f32>, what: &str) {
        Zip::from(masked).and(base).and(unmasked).and(mask).for_each(|&out, &b, &u, &m| {
            if m == 0.0 {
                assert_eq!(out.to_bits(), b.to_bits(), "{what}: m 0 gives the blend base {b}, got {out}");
            } else if m == 1.0 {
                assert_eq!(out.to_bits(), u.to_bits(), "{what}: m 1 gives the unmasked output {u}, got {out}");
            } else {
                assert!(b.min(u) <= out && out <= b.max(u), "{what}: m {m}: {out} is outside [{b}, {u}]");
            }
        });
    }

    #[test]
    fn masked_tone_on_linear_input_stays_in_0_1_and_keeps_abdisp() {
        let frame = linear_frame(31);
        let mask = banded_mask(frame.dim());
        let spec = OpSpec::Tone { v: OpVersion, levels: Some(ToneLevelsInput { black: 0.05, gamma: 1.4, white: 0.8 }), curve: None };
        assert!(frame.iter().any(|&v| v > 1000.0), "precondition: the input is linear");

        let masked = run_masked(&spec, OpInput::mono(&frame, None), &mask);
        let unmasked = run_mono(&spec, &frame);
        let base = blend_base(&spec, &OpInput::mono(&frame, None)).unwrap();
        assert_eq!(masked.values, OutputValues::DisplayReferred, "a masked tone writes a display-referred file (ABDISP = T)");
        assert_in_unit_range(&masked.planes, "masked tone mono");
        assert_banded_blend(&masked.planes[0], &base[0], &unmasked.planes[0], &mask, "tone mono");
        assert_eq!(masked.history_lines, unmasked.history_lines);

        let [r, g, b] = rgb_frames();
        let masked = run_masked(&spec, OpInput::rgb([&r, &g, &b], None), &mask);
        let unmasked = run_rgb(&spec, [&r, &g, &b]);
        let base = blend_base(&spec, &OpInput::rgb([&r, &g, &b], None)).unwrap();
        assert_eq!(masked.values, OutputValues::DisplayReferred);
        assert_in_unit_range(&masked.planes, "masked tone rgb");
        for c in 0..3 {
            assert_banded_blend(&masked.planes[c], &base[c], &unmasked.planes[c], &mask, &format!("tone rgb channel {c}"));
        }
    }

    #[test]
    fn masked_arcsinh_m0_region_equals_the_normalisation_not_the_raw_value() {
        let frame = linear_frame(32);
        let mask = banded_mask(frame.dim());
        let spec = OpSpec::Arcsinh { v: OpVersion, factor: 80.0 };

        let masked = run_masked(&spec, OpInput::mono(&frame, None), &mask);
        let unmasked = run_mono(&spec, &frame);
        let (lo, hi) = finite_range(&[&frame]);
        let inv = 1.0 / (hi - lo);
        let normalised = frame.mapv(|v| ((v - lo) * inv).clamp(0.0, 1.0));
        assert_eq!(masked.values, OutputValues::DisplayReferred);
        assert_in_unit_range(&masked.planes, "masked arcsinh");
        assert_banded_blend(&masked.planes[0], &normalised, &unmasked.planes[0], &mask, "arcsinh");
        let protected_raw = Zip::from(&frame).and(&mask).fold(0usize, |n, &v, &m| n + usize::from(m == 0.0 && v > 1.0));
        assert!(protected_raw > 1000, "precondition: the m = 0 band holds raw linear values above 1 ({protected_raw})");
        assert_eq!(masked.extras, unmasked.extras);
    }

    #[test]
    fn masked_pixelmath_with_rescale_on_linear_input_stays_in_0_1() {
        let frame = linear_frame(33);
        let mask = banded_mask(frame.dim());
        let expression = "$T * 2 - med($T)";
        let spec = OpSpec::Pixelmath { v: OpVersion, expression: expression.into(), slots: vec![], truncate: false, rescale: true };
        let opts = OutputOptions { truncate: false, rescale: true };
        let program = compile(expression, &["$T".to_string()]).unwrap();
        let blended_then_finished = |target: &Array2<f32>| {
            let mut plane = blend(target, &evaluate_unfinished(&program, &[target]).unwrap(), &mask).unwrap();
            finish_plane(&mut plane, &opts);
            plane
        };

        let masked = run_masked(&spec, OpInput::mono(&frame, None), &mask);
        assert_eq!(masked.values, OutputValues::Rescaled);
        assert_in_unit_range(&masked.planes, "masked pixelmath mono");
        assert_bitwise(&masked.planes[0], &blended_then_finished(&frame), "rescale acts on the blended plane");
        let finished_then_blended = blend(&frame, &evaluate(&program, &[&frame], &opts).unwrap(), &mask).unwrap();
        let leaked = finished_then_blended.iter().copied().fold(f32::MIN, f32::max);
        assert!(leaked > 1000.0, "precondition: blending after the rescale would leak linear values ({leaked})");
        let (stats, non_finite) = finite_output_stats(&masked.planes[0]);
        assert_eq!(masked.extras[RES_STATS], json!(stats), "the stats describe the blended output");
        assert_eq!(masked.extras[RES_NON_FINITE_COUNT], json!(non_finite));

        let [r, g, b] = rgb_frames().map(|p| p.mapv(|v| v * 20.0));
        let masked = run_masked(&spec, OpInput::rgb([&r, &g, &b], None), &mask);
        assert_in_unit_range(&masked.planes, "masked pixelmath rgb");
        for (plane, channel) in masked.planes.iter().zip([&r, &g, &b]) {
            assert_bitwise(plane, &blended_then_finished(channel), "pixelmath rgb channel");
        }
    }

    #[test]
    fn background_and_masked_stretch_refuse_a_mask_by_name() {
        let frame = seeded_frame(34, 1000.0);
        let full = Array2::from_elem(frame.dim(), 1.0f32);
        let small = Array2::from_elem((8, 8), 1.0f32);
        let refused: Vec<OpSpec> = golden_specs()
            .into_iter()
            .filter(|s| matches!(s, OpSpec::Background { .. } | OpSpec::MaskedStretch { .. }))
            .collect();
        assert_eq!(refused.len(), 7, "six background models and one masked stretch");
        for spec in &refused {
            for mask in [&full, &small] {
                let err = run_op(spec, &OpInput::mono(&frame, None).with_mask(Some(mask)), None).expect_err(spec.name());
                assert!(matches!(err, OpError::MaskNotSupported(name) if name == spec.name()), "{err}");
                assert_eq!(err.to_string(), format!("{} does not take a mask", spec.name()));
            }
        }

        let lhe = OpSpec::Lhe { v: OpVersion, config: LheConfig { kernel_radius: 16, ..LheConfig::default() } };
        let err = run_op(&lhe, &OpInput::mono(&frame, None).with_mask(Some(&small)), None).expect_err("a mask of another size");
        assert!(matches!(err, OpError::Mask(MaskError::BlendDims { .. })), "{err}");
        assert_eq!(err.to_string(), "a blend needs base, processed and mask of one size; got base 256×256, processed 256×256, mask 8×8");
    }

    const A6_SIDE: usize = 128;

    fn a6_frame(seed: u64, scale: f32, holes: [(usize, usize); 3]) -> Array2<f32> {
        let mut frame = seeded_frame(seed, scale).slice(ndarray::s![..A6_SIDE, ..A6_SIDE]).to_owned();
        for (cell, value) in holes.into_iter().zip([f32::NAN, f32::INFINITY, f32::NEG_INFINITY]) {
            frame[cell] = value;
        }
        frame
    }

    fn a6_planes(scale: f32) -> [Array2<f32>; 3] {
        [
            a6_frame(51, scale, [(7, 9), (60, 3), (101, 77)]),
            a6_frame(52, scale * 1.6, [(20, 40), (61, 3), (90, 90)]),
            a6_frame(53, scale * 0.4, [(7, 10), (5, 120), (127, 0)]),
        ]
    }

    fn ulps_apart(a: f32, b: f32) -> u64 {
        let ordered = |v: f32| {
            let magnitude = i64::from(v.to_bits() & 0x7fff_ffff);
            if v.is_sign_negative() {
                -magnitude
            } else {
                magnitude
            }
        };
        ordered(a).abs_diff(ordered(b))
    }

    fn mask_capable_specs() -> Vec<(OpSpec, f32)> {
        let lhe = LheConfig { kernel_radius: 16, ..LheConfig::default() };
        let rl = RLConfig { iterations: 5, ..RLConfig::default() };
        vec![
            (OpSpec::WaveletDenoise { v: OpVersion, config: WaveletConfig::default() }, 1000.0),
            (OpSpec::RlDeconv { v: OpVersion, config: rl, psf: PsfSpec::Gaussian { sigma: 2.0, size: 15 } }, 1000.0),
            (OpSpec::Arcsinh { v: OpVersion, factor: 50.0 }, 1000.0),
            (OpSpec::Ghs { v: OpVersion, params: GhsParams { d: 5.0, b: 2.0, sp: 0.05, lp: 0.0, hp: 1.0 } }, 1000.0),
            (OpSpec::Lhe { v: OpVersion, config: lhe }, 1.0),
            (OpSpec::Hdrmt { v: OpVersion, config: HdrConfig { layers: 3, ..HdrConfig::default() } }, 1.0),
            (OpSpec::Tone { v: OpVersion, levels: Some(ToneLevelsInput { black: 0.1, gamma: 1.2, white: 0.9 }), curve: None }, 1000.0),
            (OpSpec::Pixelmath { v: OpVersion, expression: "$T * 2 - 0.1".into(), slots: vec![], truncate: false, rescale: false }, 1000.0),
        ]
    }

    fn assert_a6(spec: &OpSpec, planes: &[&Array2<f32>]) {
        let input = || match planes {
            [r, g, b] => OpInput::rgb([*r, *g, *b], None),
            _ => OpInput::mono(planes[0], None),
        };
        let what = format!("{} on {} plane(s)", spec.name(), planes.len());
        let unmasked = run_op(spec, &input(), None).unwrap_or_else(|e| panic!("{what}: {e:#}"));
        let base = blend_base(spec, &input()).unwrap_or_else(|e| panic!("{what}: {e:#}"));
        let dims = planes[0].dim();
        for m in [0.0f32, 1.0, 0.5] {
            let masked = run_masked(spec, input(), &Array2::from_elem(dims, m));
            assert_eq!(masked.planes.len(), planes.len(), "{what}");
            assert_eq!(masked.values, unmasked.values, "{what}: a mask keeps the output domain");
            for (c, ((out, b), u)) in masked.planes.iter().zip(&base).zip(&unmasked.planes).enumerate() {
                Zip::indexed(out).and(b).and(u).and(planes[c]).for_each(|ix, &o, &b, &u, &v| {
                    let at = format!("{what}, channel {c}, pixel {ix:?}, m {m}");
                    if !v.is_finite() || m == 1.0 || (m > 0.0 && !u.is_finite()) {
                        assert_eq!(o.to_bits(), u.to_bits(), "{at}: the unmasked result {u} was not kept, got {o}");
                    } else if m == 0.0 {
                        assert_eq!(o.to_bits(), b.to_bits(), "{at}: the blend base {b} was not kept, got {o}");
                    } else {
                        let midpoint = ((f64::from(b) + f64::from(u)) / 2.0) as f32;
                        assert!(ulps_apart(o, midpoint) <= 1, "{at}: {o:e} is not the midpoint {midpoint:e} of {b:e} and {u:e}");
                    }
                });
            }
        }
    }

    #[test]
    fn blend_exact_for_every_mask_capable_file_op() {
        let specs = mask_capable_specs();
        assert_eq!(specs.len(), 8);
        for (spec, scale) in &specs {
            let planes = a6_planes(*scale);
            assert_a6(spec, &[&planes[0]]);
            if !spec.runs_on_one_plane_only() {
                assert_a6(spec, &[&planes[0], &planes[1], &planes[2]]);
            }
        }
    }

    fn timed_blend(dims: (usize, usize), planes: usize) -> std::time::Duration {
        let base = Array2::from_shape_fn(dims, |(y, x)| ((y * 7 + x * 13) % 4093) as f32);
        let processed = Array2::from_shape_fn(dims, |(y, x)| ((y * 11 + x * 3) % 4091) as f32 * 0.5);
        let mask = Array2::from_shape_fn(dims, |(y, x)| ((y + x) % 257) as f32 / 256.0);
        (0..5)
            .map(|_| {
                let t0 = std::time::Instant::now();
                for _ in 0..planes {
                    std::hint::black_box(blend(&base, &processed, &mask).unwrap());
                }
                t0.elapsed()
            })
            .min()
            .unwrap_or_default()
    }

    #[test]
    #[ignore]
    fn blend_24mp_mono_under_80_ms() {
        let best = timed_blend((4000, 6000), 1);
        println!("blend 6000x4000 mono: best of 5 = {} ms", best.as_millis());
        assert!(best.as_millis() <= 80, "blend of a 24 MP plane took {} ms", best.as_millis());
    }

    #[test]
    #[ignore]
    fn blend_lw_rgb_under_250_ms() {
        let best = timed_blend((4177, 7065), 3);
        println!("blend 7065x4177 x 3 planes: best of 5 = {} ms", best.as_millis());
        assert!(best.as_millis() <= 250, "blend of the LW composite took {} ms", best.as_millis());
    }
}
