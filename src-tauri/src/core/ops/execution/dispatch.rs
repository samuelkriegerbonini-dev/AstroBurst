use ndarray::Array2;
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
use crate::core::ops::background::{
    checked_background_config, checked_deband_settings, deband_axis_token, deband_guard, deband_history, deband_single,
    BackgroundModel, DebandRequest,
};
use crate::core::ops::domain::output_values_for;
use crate::core::ops::error::OpError;
use crate::core::ops::op_io::{OpInput, OpOutput};
use crate::core::ops::pixelmath::{evaluate_per_channel, evaluate_target, PixelMathSlot};
use crate::core::ops::psf::{estimated_kernel, DeconvPsf, PsfSpec};
use crate::core::ops::spec::{OpSpec, OutputValues};
use crate::core::ops::stretch::{arcsinh_factor, clamped_ghs, clamped_masked_stretch};
use crate::core::ops::tone::{tone_planes, ToneCurveInput, ToneLevelsInput, ToneOps};
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
use crate::types::stacking::RLConfig;

pub(super) fn dispatch(spec: &OpSpec, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
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
