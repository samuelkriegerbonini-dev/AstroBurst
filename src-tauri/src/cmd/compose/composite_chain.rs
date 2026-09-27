use std::sync::{Arc, LazyLock, Mutex, MutexGuard, TryLockError};
use std::time::Instant;

use anyhow::{anyhow, bail};
use ndarray::{Array2, Zip};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cmd::common::{blocking_cmd, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers::{self, CompositeSnapshot, CompositeTriplet};
use crate::cmd::pixelmath::{evaluate_per_channel, PixelMathSlot, RES_NON_FINITE_COUNT};
use crate::cmd::processing::{background_config, estimated_kernel, DeconvPsf, COMPOSITE_STRETCH_REQUIRED, RES_DBE_REJECTED_COUNT};
use crate::cmd::psf::{psf_config, psf_result_json};
use crate::core::analysis::deconvolution::richardson_lucy;
use crate::core::imaging::background::{extract_background, BackgroundConfig};
use crate::core::imaging::dbe::{extract_background_dbe, DbeConfig};
use crate::core::imaging::psf_estimation::estimate_psf;
use crate::core::imaging::stats::{combine_channel_stats, compute_image_stats, is_valid_pixel};
use crate::core::imaging::stf::{auto_stf, make_stf_u8_fn, AutoStfConfig};
use crate::core::imaging::wavelet::{wavelet_denoise, WaveletConfig};
use crate::core::pixelmath::OutputOptions;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    BACKGROUND_MODEL_POLYNOMIAL, BACKGROUND_MODEL_SPLINE, CHAIN_INPUT_BASE, CHAIN_STEP_BACKGROUND,
    CHAIN_STEP_DECONV, CHAIN_STEP_DENOISE, CHAIN_STEP_LOCAL_CONTRAST, CHAIN_STEP_MASKED_STRETCH,
    CHAIN_STEP_PIXEL_MATH, CHAIN_STEP_STRETCH, DISPLAYED_LINEAR, DISPLAYED_STRETCHED, DISPLAYED_TONED,
    EVENT_DECONV_PROGRESS, EVENT_WAVELET_PROGRESS, PROGRESS_EVENT, PROGRESS_STEPS, RES_B, RES_BASE_PNG_PATH,
    RES_CHAIN_GENERATION, RES_CHAIN_INPUT, RES_CHAIN_RESTARTED, RES_CONVERGENCE, RES_DIMENSIONS, RES_DISPLAYED,
    RES_ELAPSED_MS, RES_G, RES_ITERATIONS_RUN, RES_LINKED, RES_LIVE_GENERATION, RES_MODEL_PNG_PATH,
    RES_NOISE_ESTIMATE, RES_PNG_PATH, RES_PSF_SOURCE, RES_R, RES_RESTORED, RES_RMS_RESIDUAL, RES_SAMPLE_COUNT,
    RES_SCALES_PROCESSED, RES_STATS, RES_STEPS, RES_STF, RES_WARNINGS,
};
use crate::types::image::{ImageStats, StfParams};
use crate::types::stacking::RLConfig;

const CHANNEL_COUNT: u64 = 3;
const BASE_SUFFIX: &str = "base";
const MODEL_SUFFIX: &str = "bgmodel";

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct DisplayStf {
    pub r: StfParams,
    pub g: StfParams,
    pub b: StfParams,
    pub linked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChainStep {
    Background,
    Denoise,
    Deconv,
    Stretch,
    MaskedStretch,
    LocalContrast,
    PixelMath,
}

const CHAIN_ORDER: [ChainStep; 7] = [
    ChainStep::Background,
    ChainStep::Denoise,
    ChainStep::Deconv,
    ChainStep::Stretch,
    ChainStep::MaskedStretch,
    ChainStep::LocalContrast,
    ChainStep::PixelMath,
];

impl ChainStep {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Background => CHAIN_STEP_BACKGROUND,
            Self::Denoise => CHAIN_STEP_DENOISE,
            Self::Deconv => CHAIN_STEP_DECONV,
            Self::Stretch => CHAIN_STEP_STRETCH,
            Self::MaskedStretch => CHAIN_STEP_MASKED_STRETCH,
            Self::LocalContrast => CHAIN_STEP_LOCAL_CONTRAST,
            Self::PixelMath => CHAIN_STEP_PIXEL_MATH,
        }
    }

    fn parse(name: &str) -> Option<Self> {
        CHAIN_ORDER.into_iter().find(|step| step.name() == name)
    }

    fn stage(self) -> u8 {
        match self {
            Self::Background => 0,
            Self::Denoise => 1,
            Self::Deconv => 2,
            Self::Stretch | Self::MaskedStretch => 3,
            Self::LocalContrast => 4,
            Self::PixelMath => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChainInput {
    Base,
    Step(ChainStep),
}

impl ChainInput {
    fn name(self) -> &'static str {
        match self {
            Self::Base => CHAIN_INPUT_BASE,
            Self::Step(step) => step.name(),
        }
    }

    pub(crate) fn parse(raw: &str, step: ChainStep) -> anyhow::Result<Self> {
        if raw == CHAIN_INPUT_BASE {
            return Ok(Self::Base);
        }
        match ChainStep::parse(raw) {
            Some(input) if step == ChainStep::PixelMath || input.stage() < step.stage() => Ok(Self::Step(input)),
            _ => bail!("Invalid composite chain input {} for step {}", raw, step.name()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayedTier {
    Linear,
    Stretched,
    Toned,
}

impl DisplayedTier {
    fn name(self) -> &'static str {
        match self {
            Self::Linear => DISPLAYED_LINEAR,
            Self::Stretched => DISPLAYED_STRETCHED,
            Self::Toned => DISPLAYED_TONED,
        }
    }
}

pub(crate) fn displayed_tier(state: &CompositeSnapshot) -> DisplayedTier {
    if state.toned.is_some() {
        DisplayedTier::Toned
    } else if state.stretched.is_some() {
        DisplayedTier::Stretched
    } else {
        DisplayedTier::Linear
    }
}

fn tier_planes(state: &CompositeSnapshot, tier: DisplayedTier) -> &CompositeTriplet {
    match tier {
        DisplayedTier::Linear => &state.key,
        DisplayedTier::Stretched => state.stretched.as_ref().unwrap_or(&state.key),
        DisplayedTier::Toned => state.toned.as_ref().unwrap_or(&state.key),
    }
}

fn plane_refs(planes: &CompositeTriplet) -> [&Array2<f32>; 3] {
    [&*planes[0].0, &*planes[1].0, &*planes[2].0]
}

pub(crate) fn key_planes(state: &CompositeSnapshot) -> [&Array2<f32>; 3] {
    plane_refs(&state.key)
}

pub(crate) fn contrast_planes(state: &CompositeSnapshot) -> anyhow::Result<[&Array2<f32>; 3]> {
    state
        .toned
        .as_ref()
        .or(state.stretched.as_ref())
        .map(plane_refs)
        .ok_or_else(|| anyhow!(COMPOSITE_STRETCH_REQUIRED))
}

fn triplet_from([r, g, b]: [Array2<f32>; 3]) -> CompositeTriplet {
    let (stats_r, (stats_g, stats_b)) = rayon::join(
        || compute_image_stats(&r),
        || rayon::join(|| compute_image_stats(&g), || compute_image_stats(&b)),
    );
    [(Arc::new(r), stats_r), (Arc::new(g), stats_g), (Arc::new(b), stats_b)]
}

pub(crate) fn linear_state(input: &CompositeSnapshot, key: [Array2<f32>; 3]) -> CompositeSnapshot {
    helpers::composite_state_from_key(triplet_from(key), input.wb)
}

pub(crate) fn stretched_state(input: &CompositeSnapshot, stretched: [Array2<f32>; 3]) -> CompositeSnapshot {
    CompositeSnapshot {
        orig: input.orig.clone(),
        key: input.key.clone(),
        wb: input.wb,
        stretched: Some(triplet_from(stretched)),
        toned: None,
    }
}

pub(crate) fn toned_state(input: &CompositeSnapshot, toned: [Array2<f32>; 3]) -> CompositeSnapshot {
    CompositeSnapshot { toned: Some(triplet_from(toned)), ..input.clone() }
}

pub(crate) fn key_luminance(key: &CompositeTriplet) -> anyhow::Result<Array2<f32>> {
    let [r, g, b] = plane_refs(key);
    if r.dim() != g.dim() || g.dim() != b.dim() {
        bail!(
            "Composite channels differ in size (R {:?}, G {:?}, B {:?}); re-run Blend",
            r.dim(),
            g.dim(),
            b.dim()
        );
    }
    let mut out = Array2::<f32>::zeros(r.dim());
    Zip::from(&mut out).and(r).and(g).and(b).par_for_each(|o, &rv, &gv, &bv| {
        *o = if is_valid_pixel(rv) && is_valid_pixel(gv) && is_valid_pixel(bv) {
            (rv + gv + bv) / 3.0
        } else {
            f32::NAN
        };
    });
    Ok(out)
}

#[derive(Default)]
struct ChainState {
    generation: Option<u64>,
    base: Option<CompositeSnapshot>,
    steps: [Option<CompositeSnapshot>; 7],
}

impl ChainState {
    fn is_valid(&self, live: u64) -> bool {
        self.base.is_some() && self.generation == Some(live)
    }

    fn snapshot(&self, input: ChainInput) -> Option<&CompositeSnapshot> {
        match input {
            ChainInput::Base => self.base.as_ref(),
            ChainInput::Step(step) => self.steps[step as usize].as_ref(),
        }
    }

    fn step_names(&self) -> Vec<&'static str> {
        CHAIN_ORDER
            .into_iter()
            .filter(|step| self.steps[*step as usize].is_some())
            .map(ChainStep::name)
            .collect()
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn restart(&mut self, base: CompositeSnapshot) {
        self.clear();
        self.base = Some(base);
    }

    fn set_step(&mut self, step: ChainStep, state: CompositeSnapshot, generation: u64) {
        for other in CHAIN_ORDER {
            if other.stage() >= step.stage() {
                self.steps[other as usize] = None;
            }
        }
        self.steps[step as usize] = Some(state);
        self.generation = Some(generation);
    }
}

static CHAIN: LazyLock<Mutex<ChainState>> = LazyLock::new(|| Mutex::new(ChainState::default()));

fn lock_chain() -> MutexGuard<'static, ChainState> {
    CHAIN.lock().unwrap_or_else(|e| e.into_inner())
}

fn try_lock_chain() -> Option<MutexGuard<'static, ChainState>> {
    match CHAIN.try_lock() {
        Ok(chain) => Some(chain),
        Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

pub(crate) fn drop_chain_if_idle() {
    if let Some(mut chain) = try_lock_chain() {
        chain.clear();
    }
}

pub(crate) struct ChainCall {
    pub output_dir: String,
    pub chain_input: String,
    pub display_stf: Option<DisplayStf>,
}

pub(crate) struct StepOutput {
    pub state: CompositeSnapshot,
    pub extras: Value,
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub(crate) fn chain_png_path(output_dir: &str, suffix: &str) -> String {
    format!("{}/composite_chain_{}_{}.png", output_dir, suffix, now_ms())
}

fn channel_stats(planes: &CompositeTriplet, linked: bool) -> [ImageStats; 3] {
    if linked {
        let combined = combine_channel_stats(&planes[0].1, &planes[1].1, &planes[2].1);
        [combined.clone(), combined.clone(), combined]
    } else {
        [planes[0].1.clone(), planes[1].1.clone(), planes[2].1.clone()]
    }
}

fn auto_display_stf(planes: &CompositeTriplet, linked: bool) -> [StfParams; 3] {
    let config = AutoStfConfig::default();
    if linked {
        let (stf, _) = helpers::compute_linked_stf_with_stats(&planes[0].1, &planes[1].1, &planes[2].1, &config);
        [stf; 3]
    } else {
        [
            auto_stf(&planes[0].1, &config),
            auto_stf(&planes[1].1, &config),
            auto_stf(&planes[2].1, &config),
        ]
    }
}

fn render_linear(planes: &CompositeTriplet, stfs: [StfParams; 3], linked: bool, path: &str) -> anyhow::Result<()> {
    let stats = channel_stats(planes, linked);
    let [r, g, b] = plane_refs(planes);
    helpers::render_rgb_preview_with_stf(
        r,
        g,
        b,
        make_stf_u8_fn(&stfs[0], &stats[0]),
        make_stf_u8_fn(&stfs[1], &stats[1]),
        make_stf_u8_fn(&stfs[2], &stats[2]),
        path,
        MAX_PREVIEW_DIM,
    )
}

fn render_direct(planes: &CompositeTriplet, path: &str) -> anyhow::Result<()> {
    let [r, g, b] = plane_refs(planes);
    helpers::render_rgb_preview(r, g, b, path, MAX_PREVIEW_DIM)
}

fn stf_display_json(stfs: [StfParams; 3], linked: bool) -> Value {
    json!({
        RES_R: helpers::stf_json(&stfs[0]),
        RES_G: helpers::stf_json(&stfs[1]),
        RES_B: helpers::stf_json(&stfs[2]),
        RES_LINKED: linked,
    })
}

fn render_base(base: &CompositeSnapshot, display: Option<&DisplayStf>, output_dir: &str) -> anyhow::Result<String> {
    let path = chain_png_path(output_dir, BASE_SUFFIX);
    match displayed_tier(base) {
        DisplayedTier::Linear => {
            let (stfs, linked) = match display {
                Some(d) => ([d.r, d.g, d.b], d.linked),
                None => (auto_display_stf(&base.key, true), true),
            };
            render_linear(&base.key, stfs, linked, &path)?;
        }
        tier => render_direct(tier_planes(base, tier), &path)?,
    }
    Ok(path)
}

fn render_step(
    state: &CompositeSnapshot,
    step: ChainStep,
    linked: bool,
    output_dir: &str,
) -> anyhow::Result<(String, DisplayedTier, Value)> {
    let path = chain_png_path(output_dir, step.name());
    let tier = displayed_tier(state);
    let stf = match tier {
        DisplayedTier::Linear => {
            let stfs = auto_display_stf(&state.key, linked);
            render_linear(&state.key, stfs, linked, &path)?;
            stf_display_json(stfs, linked)
        }
        tier => {
            render_direct(tier_planes(state, tier), &path)?;
            Value::Null
        }
    };
    Ok((path, tier, stf))
}

fn merged(mut common: Value, extras: Value) -> Value {
    if let (Some(target), Value::Object(extra)) = (common.as_object_mut(), extras) {
        target.extend(extra);
    }
    common
}

fn remove_files<'a>(paths: impl IntoIterator<Item = &'a String>) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

struct Committed {
    state: CompositeSnapshot,
    extras: Value,
    png_path: String,
    displayed: DisplayedTier,
    stf: Value,
    generation: u64,
}

fn commit_step(
    step: ChainStep,
    input: &CompositeSnapshot,
    expected: Option<u64>,
    linked: bool,
    output_dir: &str,
    run: impl FnOnce(&CompositeSnapshot, &str) -> anyhow::Result<StepOutput>,
    produced: &mut Vec<String>,
) -> anyhow::Result<Committed> {
    let StepOutput { state, extras } = run(input, output_dir)?;
    produced.extend(extras[RES_MODEL_PNG_PATH].as_str().map(str::to_string));
    let (png_path, displayed, stf) = render_step(&state, step, linked, output_dir)?;
    produced.push(png_path.clone());
    let generation = helpers::commit_composite(&state, expected)?;
    Ok(Committed { state, extras, png_path, displayed, stf, generation })
}

pub(crate) fn run_chain_step(
    step: ChainStep,
    call: &ChainCall,
    run: impl FnOnce(&CompositeSnapshot, &str) -> anyhow::Result<StepOutput>,
) -> anyhow::Result<Value> {
    let t0 = Instant::now();
    let output_dir = resolve_output_dir(&call.output_dir)?;
    let requested = ChainInput::parse(&call.chain_input, step)?;
    let mut chain = lock_chain();
    let live = helpers::composite_generation();
    let held = if chain.is_valid(live) { chain.snapshot(requested).cloned() } else { None };
    let (input, used, expected, captured) = match held {
        Some(state) => (state, requested, chain.generation, None),
        None => {
            let restarted = chain.base.is_some();
            let (base, generation) = helpers::snapshot_composite()?;
            (base.clone(), ChainInput::Base, Some(generation), Some((base, restarted)))
        }
    };
    let base_png = captured
        .as_ref()
        .map(|(base, _)| render_base(base, call.display_stf.as_ref(), &output_dir))
        .transpose()?;
    let restarted = captured.as_ref().is_some_and(|(_, restarted)| *restarted);
    let linked = call.display_stf.map_or(true, |d| d.linked);

    let mut produced: Vec<String> = base_png.iter().cloned().collect();
    let committed = commit_step(step, &input, expected, linked, &output_dir, run, &mut produced);
    let Committed { state, extras, png_path, displayed, stf, generation } = match committed {
        Ok(committed) => committed,
        Err(e) => {
            remove_files(&produced);
            if !chain.is_valid(helpers::composite_generation()) {
                chain.clear();
            }
            return Err(e);
        }
    };
    let (rows, cols) = state.key[0].0.dim();
    if let Some((base, _)) = captured {
        chain.restart(base);
    }
    chain.set_step(step, state, generation);

    let common = json!({
        RES_PNG_PATH: png_path,
        RES_BASE_PNG_PATH: base_png,
        RES_DIMENSIONS: [cols, rows],
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        RES_CHAIN_GENERATION: generation,
        RES_CHAIN_RESTARTED: restarted,
        RES_CHAIN_INPUT: used.name(),
        RES_DISPLAYED: displayed.name(),
        RES_STF: stf,
    });
    Ok(merged(common, extras))
}

type ChannelProgress<'a> = [Option<&'a ProgressHandle>; 3];

fn channel_parts(progress: &ProgressHandle) -> [ProgressHandle; 3] {
    [0, 1, 2].map(|i| progress.part(i, CHANNEL_COUNT))
}

fn per_channel<T>(
    planes: [&Array2<f32>; 3],
    progress: ChannelProgress<'_>,
    mut run: impl FnMut(&Array2<f32>, Option<&ProgressHandle>) -> anyhow::Result<T>,
) -> anyhow::Result<[T; 3]> {
    let mut out = Vec::with_capacity(3);
    for (plane, part) in planes.into_iter().zip(progress) {
        out.push(run(plane, part)?);
    }
    <[T; 3]>::try_from(out).map_err(|_| anyhow!("expected three channels"))
}

enum BackgroundModel {
    Polynomial(BackgroundConfig),
    Spline(DbeConfig),
}

fn background_model(
    model: &str,
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: &str,
    dbe: Option<DbeConfig>,
) -> anyhow::Result<BackgroundModel> {
    match model {
        BACKGROUND_MODEL_POLYNOMIAL => Ok(BackgroundModel::Polynomial(background_config(
            grid_size, poly_degree, sigma_clip, iterations, mode,
        )?)),
        BACKGROUND_MODEL_SPLINE => {
            let config = dbe.ok_or_else(|| anyhow!("The spline background model needs its DBE settings"))?;
            Ok(BackgroundModel::Spline(DbeConfig { manual_samples: Vec::new(), ..config }))
        }
        other => bail!("Unknown background model {}; expected polynomial or spline", other),
    }
}

struct ChannelBackground {
    corrected: Array2<f32>,
    model: Array2<f32>,
    sample_count: usize,
    rms_residual: f64,
    rejected_count: Option<usize>,
}

fn background_channel(
    plane: &Array2<f32>,
    model: &BackgroundModel,
    progress: Option<&ProgressHandle>,
) -> anyhow::Result<ChannelBackground> {
    match model {
        BackgroundModel::Polynomial(config) => {
            let result = extract_background(plane, config, progress)?;
            Ok(ChannelBackground {
                corrected: result.corrected,
                model: result.model,
                sample_count: result.sample_count,
                rms_residual: result.rms_residual,
                rejected_count: None,
            })
        }
        BackgroundModel::Spline(config) => {
            let result = extract_background_dbe(plane, config, progress)?;
            Ok(ChannelBackground {
                corrected: result.corrected,
                model: result.model,
                sample_count: result.sample_count,
                rms_residual: result.rms_residual,
                rejected_count: Some(result.rejected_count),
            })
        }
    }
}

fn background_step(
    input: &CompositeSnapshot,
    model: &BackgroundModel,
    progress: ChannelProgress<'_>,
    output_dir: &str,
) -> anyhow::Result<StepOutput> {
    let [r, g, b] = per_channel(key_planes(input), progress, |plane, part| background_channel(plane, model, part))?;
    let rejected = match (r.rejected_count, g.rejected_count, b.rejected_count) {
        (Some(rr), Some(rg), Some(rb)) => Some([rr, rg, rb]),
        _ => None,
    };
    let model_png = chain_png_path(output_dir, MODEL_SUFFIX);
    let models = triplet_from([r.model, g.model, b.model]);
    render_linear(&models, auto_display_stf(&models, false), false, &model_png)?;
    let extras = json!({
        RES_MODEL_PNG_PATH: model_png,
        RES_SAMPLE_COUNT: [r.sample_count, g.sample_count, b.sample_count],
        RES_RMS_RESIDUAL: [r.rms_residual, g.rms_residual, b.rms_residual],
        RES_DBE_REJECTED_COUNT: rejected,
    });
    Ok(StepOutput { state: linear_state(input, [r.corrected, g.corrected, b.corrected]), extras })
}

#[tauri::command]
pub async fn composite_background_cmd(
    app: tauri::AppHandle,
    output_dir: String,
    chain_input: String,
    display_stf: DisplayStf,
    model: String,
    grid_size: usize,
    poly_degree: usize,
    sigma_clip: f64,
    iterations: usize,
    mode: String,
    dbe: Option<DbeConfig>,
) -> Result<Value, String> {
    let progress = ProgressHandle::new(&app, PROGRESS_EVENT, PROGRESS_STEPS as u64);

    blocking_cmd!({
        let model = background_model(&model, grid_size, poly_degree, sigma_clip, iterations, &mode, dbe)?;
        let parts = channel_parts(&progress);
        let call = ChainCall { output_dir, chain_input, display_stf: Some(display_stf) };
        run_chain_step(ChainStep::Background, &call, |input, dir| {
            background_step(input, &model, parts.each_ref().map(Some), dir)
        })
    })
}

fn denoise_step(
    input: &CompositeSnapshot,
    config: &WaveletConfig,
    progress: ChannelProgress<'_>,
) -> anyhow::Result<StepOutput> {
    let [r, g, b] = per_channel(key_planes(input), progress, |plane, part| wavelet_denoise(plane, config, part))?;
    let extras = json!({
        RES_SCALES_PROCESSED: r.scales_processed,
        RES_NOISE_ESTIMATE: [r.noise_estimate, g.noise_estimate, b.noise_estimate],
    });
    Ok(StepOutput { state: linear_state(input, [r.denoised, g.denoised, b.denoised]), extras })
}

#[tauri::command]
pub async fn composite_wavelet_denoise_cmd(
    app: tauri::AppHandle,
    output_dir: String,
    chain_input: String,
    display_stf: DisplayStf,
    num_scales: usize,
    thresholds: Vec<f64>,
    linear: bool,
    layer_bias: Option<Vec<f32>>,
) -> Result<Value, String> {
    let n_scales = num_scales.clamp(1, 8);
    let progress = ProgressHandle::new(&app, EVENT_WAVELET_PROGRESS, (n_scales * 2 + 1) as u64);

    blocking_cmd!({
        if let Some(bias) = &layer_bias {
            if bias.iter().any(|b| !b.is_finite()) {
                bail!("layer_bias must contain only finite values");
            }
        }
        let config = WaveletConfig {
            num_scales: n_scales,
            thresholds: thresholds.iter().map(|&t| t as f32).collect(),
            linear_denoise: linear,
            layer_bias,
        };
        let parts = channel_parts(&progress);
        let call = ChainCall { output_dir, chain_input, display_stf: Some(display_stf) };
        run_chain_step(ChainStep::Denoise, &call, |input, _| denoise_step(input, &config, parts.each_ref().map(Some)))
    })
}

fn deconv_step(
    input: &CompositeSnapshot,
    psf: &DeconvPsf,
    config: &RLConfig,
    progress: ChannelProgress<'_>,
) -> anyhow::Result<StepOutput> {
    let kernel = psf.kernel(|psf_config| estimated_kernel(&key_luminance(&input.key)?, psf_config))?;
    let [r, g, b] = per_channel(key_planes(input), progress, |plane, part| richardson_lucy(plane, &kernel, config, part))?;
    let extras = json!({
        RES_ITERATIONS_RUN: [r.iterations_run, g.iterations_run, b.iterations_run],
        RES_CONVERGENCE: [r.convergence, g.convergence, b.convergence],
        RES_PSF_SOURCE: psf.source(),
    });
    Ok(StepOutput { state: linear_state(input, [r.image, g.image, b.image]), extras })
}

#[tauri::command]
pub async fn composite_deconvolve_rl_cmd(
    app: tauri::AppHandle,
    output_dir: String,
    chain_input: String,
    display_stf: DisplayStf,
    iterations: usize,
    psf_sigma: f64,
    psf_size: usize,
    regularization: f64,
    deringing: bool,
    dering_threshold: f64,
    use_empirical_psf: Option<bool>,
    psf_num_stars: Option<usize>,
    psf_cutout_radius: Option<usize>,
    psf_kernel: Option<Vec<Vec<f32>>>,
) -> Result<Value, String> {
    let progress = ProgressHandle::new(&app, EVENT_DECONV_PROGRESS, iterations as u64);

    blocking_cmd!({
        let psf = DeconvPsf::choose(use_empirical_psf, psf_kernel, psf_size, psf_sigma, psf_num_stars, psf_cutout_radius)?;
        let config = RLConfig {
            iterations,
            regularization,
            deringing,
            deringing_threshold: dering_threshold as f32,
            ..RLConfig::default()
        };
        let parts = channel_parts(&progress);
        let call = ChainCall { output_dir, chain_input, display_stf: Some(display_stf) };
        let result = run_chain_step(ChainStep::Deconv, &call, |input, _| {
            deconv_step(input, &psf, &config, parts.each_ref().map(Some))
        })?;
        progress.emit_complete();
        Ok(result)
    })
}

fn psf_input_state(chain_input: &str) -> anyhow::Result<CompositeSnapshot> {
    let requested = ChainInput::parse(chain_input, ChainStep::Deconv)?;
    let held = {
        let chain = lock_chain();
        let live = helpers::composite_generation();
        if chain.is_valid(live) { chain.snapshot(requested).cloned() } else { None }
    };
    match held {
        Some(state) => Ok(state),
        None => Ok(helpers::snapshot_composite()?.0),
    }
}

#[tauri::command]
pub async fn composite_estimate_psf_cmd(
    chain_input: String,
    num_stars: Option<usize>,
    cutout_radius: Option<usize>,
    saturation_threshold: Option<f64>,
    max_ellipticity: Option<f64>,
) -> Result<Value, String> {
    blocking_cmd!({
        let state = psf_input_state(&chain_input)?;
        let luminance = key_luminance(&state.key)?;
        let config = psf_config(num_stars, cutout_radius, saturation_threshold, max_ellipticity);
        let result = estimate_psf(&luminance, &config).map_err(|e| anyhow!(e))?;
        Ok(psf_result_json(&result))
    })
}

fn pixelmath_step(
    input: &CompositeSnapshot,
    expression: &str,
    slots: &[PixelMathSlot],
    opts: OutputOptions,
) -> anyhow::Result<StepOutput> {
    let tier = displayed_tier(input);
    let evaluated = evaluate_per_channel(expression, slots, &opts, plane_refs(tier_planes(input, tier)))?;
    let state = match tier {
        DisplayedTier::Linear => linear_state(input, evaluated.results),
        DisplayedTier::Stretched => stretched_state(input, evaluated.results),
        DisplayedTier::Toned => toned_state(input, evaluated.results),
    };
    let [stats_r, stats_g, stats_b] = evaluated.stats;
    let extras = json!({
        RES_STATS: { RES_R: stats_r, RES_G: stats_g, RES_B: stats_b },
        RES_NON_FINITE_COUNT: evaluated.non_finite,
        RES_WARNINGS: evaluated.warnings,
    });
    Ok(StepOutput { state, extras })
}

#[tauri::command]
pub async fn pixelmath_composite_cmd(
    output_dir: String,
    chain_input: String,
    display_stf: DisplayStf,
    expression: String,
    slots: Vec<PixelMathSlot>,
    truncate: Option<bool>,
    rescale: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let opts = OutputOptions {
            truncate: truncate.unwrap_or(false),
            rescale: rescale.unwrap_or(false),
        };
        let call = ChainCall { output_dir, chain_input, display_stf: Some(display_stf) };
        run_chain_step(ChainStep::PixelMath, &call, |input, _| pixelmath_step(input, &expression, &slots, opts))
    })
}

#[tauri::command]
pub async fn composite_chain_reset_cmd() -> Result<Value, String> {
    blocking_cmd!({
        let mut chain = lock_chain();
        let live = helpers::composite_generation();
        let restored = match chain.base.as_ref() {
            Some(base) if chain.generation == Some(live) => {
                helpers::commit_composite(base, Some(live))?;
                true
            }
            _ => false,
        };
        chain.clear();
        Ok(json!({
            RES_RESTORED: restored,
            RES_LIVE_GENERATION: helpers::composite_generation(),
        }))
    })
}

#[tauri::command]
pub async fn composite_chain_state_cmd() -> Result<Value, String> {
    blocking_cmd!({
        let mut chain = lock_chain();
        let live = helpers::composite_generation();
        if !chain.is_valid(live) {
            chain.clear();
        }
        Ok(json!({
            RES_CHAIN_GENERATION: chain.generation,
            RES_LIVE_GENERATION: live,
            RES_STEPS: chain.step_names(),
        }))
    })
}

#[cfg(test)]
pub(crate) fn forget_chain() {
    lock_chain().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Weak;

    use crate::cmd::compose::clear_composite_cache_cmd;
    use crate::cmd::processing::{arcsinh_stretch_composite_cmd, hdrmt_composite_cmd, lhe_composite_cmd, masked_stretch_composite_cmd};
    use crate::core::analysis::deconvolution::{generate_gaussian_psf, provided_psf_kernel};
    use crate::core::imaging::hdr::{hdrmt_rgb, HdrConfig};
    use crate::core::imaging::local_contrast::{lhe_rgb, LheConfig};
    use crate::core::imaging::stretch::arcsinh_stretch_rgb;
    use crate::infra::cache::ImageEntry;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::{RES_KERNEL_SIZE, RES_MAX, RES_STRETCH_FACTOR};

    type Loader = fn() -> Option<(ImageEntry, ImageEntry, ImageEntry)>;

    fn plane(seed: usize) -> Array2<f32> {
        Array2::from_shape_fn((16, 16), |(y, x)| ((y * 16 + x + seed * 5) % 37) as f32 / 40.0 + 0.05)
    }

    fn planes() -> [Array2<f32>; 3] {
        [plane(1), plane(2), plane(3)]
    }

    fn display_stf(linked: bool) -> DisplayStf {
        let p = StfParams { shadow: 0.0, midtone: 0.4, highlight: 1.0 };
        DisplayStf { r: p, g: p, b: p, linked }
    }

    fn install_composite(k: &[Array2<f32>; 3]) {
        forget_chain();
        helpers::insert_composite_and_orig(
            k[0].clone(),
            k[1].clone(),
            k[2].clone(),
            compute_image_stats(&k[0]),
            compute_image_stats(&k[1]),
            compute_image_stats(&k[2]),
        );
    }

    const WB: [f32; 3] = [2.0, 1.0, 0.5];

    fn install_balanced_composite(k: &[Array2<f32>; 3]) -> [Array2<f32>; 3] {
        install_composite(k);
        let balanced = [0, 1, 2].map(|i| k[i].mapv(|v| v * WB[i]));
        helpers::insert_composite_white_balanced(
            [0, 1, 2].map(|i| (Arc::new(balanced[i].clone()), compute_image_stats(&balanced[i]))),
            WB,
        );
        balanced
    }

    fn out_dir(dir: &tempfile::TempDir) -> String {
        dir.path().to_str().unwrap().to_string()
    }

    fn live(load: Loader) -> Option<[Array2<f32>; 3]> {
        load().map(|(r, g, b)| [r.arr().clone(), g.arr().clone(), b.arr().clone()])
    }

    fn live_key() -> [Array2<f32>; 3] {
        let (r, g, b) = helpers::load_composite_rgb().expect("key tier");
        [r.arr().clone(), g.arr().clone(), b.arr().clone()]
    }

    fn live_orig() -> [Array2<f32>; 3] {
        let (r, g, b) = helpers::load_composite_orig_rgb().expect("orig tier");
        [r.arr().clone(), g.arr().clone(), b.arr().clone()]
    }

    fn live_key_r_weak() -> Weak<Array2<f32>> {
        let (r, _, _) = helpers::load_composite_rgb().expect("key tier");
        Arc::downgrade(&r.data_arc())
    }

    fn outside_write(k: &[Array2<f32>; 3]) {
        helpers::insert_composite_and_orig(
            k[0].clone(),
            k[1].clone(),
            k[2].clone(),
            compute_image_stats(&k[0]),
            compute_image_stats(&k[1]),
            compute_image_stats(&k[2]),
        );
    }

    fn doubled_key(input: &CompositeSnapshot) -> anyhow::Result<StepOutput> {
        let key = key_planes(input).map(|p| p.mapv(|v| v * 2.0));
        Ok(StepOutput { state: linear_state(input, key), extras: json!({}) })
    }

    fn file_count(dir: &tempfile::TempDir) -> usize {
        std::fs::read_dir(dir.path()).unwrap().count()
    }

    fn triplet(t: (Array2<f32>, Array2<f32>, Array2<f32>)) -> [Array2<f32>; 3] {
        [t.0, t.1, t.2]
    }

    async fn state() -> Value {
        composite_chain_state_cmd().await.unwrap()
    }

    fn steps(state: &Value) -> Vec<String> {
        serde_json::from_value(state[RES_STEPS].clone()).unwrap()
    }

    fn png_size(path: &str) -> (u32, u32) {
        let img = image::open(path).unwrap();
        (img.width(), img.height())
    }

    fn slot(name: &str, path: &str) -> PixelMathSlot {
        PixelMathSlot { name: name.into(), path: path.into() }
    }

    #[test]
    fn chain_inputs_are_validated_against_the_stage_rules() {
        assert_eq!(ChainInput::parse("base", ChainStep::Stretch).unwrap(), ChainInput::Base);
        assert_eq!(ChainInput::parse("stretch", ChainStep::LocalContrast).unwrap(), ChainInput::Step(ChainStep::Stretch));
        assert_eq!(ChainInput::parse("deconv", ChainStep::MaskedStretch).unwrap(), ChainInput::Step(ChainStep::Deconv));
        assert_eq!(ChainInput::parse("pixelMath", ChainStep::PixelMath).unwrap(), ChainInput::Step(ChainStep::PixelMath));
        assert_eq!(ChainInput::parse("localContrast", ChainStep::PixelMath).unwrap(), ChainInput::Step(ChainStep::LocalContrast));

        let same_stage = ChainInput::parse("stretch", ChainStep::MaskedStretch).unwrap_err().to_string();
        assert_eq!(same_stage, "Invalid composite chain input stretch for step maskedStretch");
        let downstream = ChainInput::parse("localContrast", ChainStep::Stretch).unwrap_err().to_string();
        assert_eq!(downstream, "Invalid composite chain input localContrast for step stretch");
        let unknown = ChainInput::parse("bogus", ChainStep::Denoise).unwrap_err().to_string();
        assert_eq!(unknown, "Invalid composite chain input bogus for step denoise");
    }

    #[test]
    fn the_key_luminance_averages_valid_pixels_and_drops_padding() {
        let mut r = Array2::from_elem((2, 2), 0.3f32);
        let g = Array2::from_elem((2, 2), 0.6f32);
        let mut b = Array2::from_elem((2, 2), 0.9f32);
        r[[0, 0]] = 0.0;
        b[[1, 1]] = f32::NAN;
        let key = [r, g, b].map(|p| {
            let stats = compute_image_stats(&p);
            (Arc::new(p), stats)
        });
        let lum = key_luminance(&key).unwrap();
        assert!(lum[[0, 0]].is_nan(), "a padded channel must not contribute");
        assert!(lum[[1, 1]].is_nan(), "a non-finite channel must not contribute");
        assert!((lum[[0, 1]] - 0.6).abs() < 1e-6);
        assert!((lum[[1, 0]] - 0.6).abs() < 1e-6);
    }

    #[tokio::test]
    async fn the_first_chain_step_captures_and_renders_the_base_and_keeps_the_key_tier() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let k = planes();
        install_composite(&k);

        let value = arcsinh_stretch_composite_cmd(out_dir(&dir), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let stretched = live(helpers::load_composite_stretched);
        let key = live_key();
        let state = state().await;
        helpers::clear_composite();
        let value = value.unwrap();

        let base_png = value[RES_BASE_PNG_PATH].as_str().expect("the first step captures a base");
        assert!(base_png.contains("composite_chain_base_"), "{base_png}");
        assert_eq!(png_size(base_png), (16, 16));
        let png = value[RES_PNG_PATH].as_str().unwrap();
        assert!(png.contains("composite_chain_stretch_"), "{png}");
        assert_eq!(png_size(png), (16, 16));
        assert_eq!(value[RES_CHAIN_RESTARTED], false);
        assert_eq!(value[RES_CHAIN_INPUT], "base");
        assert_eq!(value[RES_DISPLAYED], "stretched");
        assert!(value[RES_STF].is_null(), "a stretched output carries no display STF");
        assert_eq!(value[RES_STRETCH_FACTOR], 50.0);
        assert_eq!(value[RES_DIMENSIONS], json!([16, 16]));
        assert_eq!(value[RES_CHAIN_GENERATION], state[RES_LIVE_GENERATION]);
        assert_eq!(state[RES_CHAIN_GENERATION], state[RES_LIVE_GENERATION]);
        assert_eq!(steps(&state), vec!["stretch"]);

        let expected = triplet(arcsinh_stretch_rgb(&k[0], &k[1], &k[2], 50.0));
        assert_eq!(stretched.expect("the stretch step writes the stretched tier"), expected);
        assert_eq!(key, k, "a stretch step must keep the key tier");
    }

    #[tokio::test]
    async fn a_chain_input_above_the_step_stage_is_refused_by_the_command() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        install_composite(&planes());
        let err = arcsinh_stretch_composite_cmd(out_dir(&dir), 50.0, Some("localContrast".into()), Some(display_stf(true))).await;
        let state = state().await;
        helpers::clear_composite();
        assert_eq!(err.unwrap_err(), "Invalid composite chain input localContrast for step stretch");
        assert!(steps(&state).is_empty(), "a refused input must not touch the chain");
    }

    #[tokio::test]
    async fn re_running_an_upstream_step_drops_the_downstream_snapshots() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        install_composite(&planes());
        let lhe = LheConfig { kernel_radius: 3, ..LheConfig::default() };

        let first = arcsinh_stretch_composite_cmd(out.clone(), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let contrast = lhe_composite_cmd(out.clone(), lhe, Some("stretch".into()), Some(display_stf(true))).await;
        let after_contrast = state().await;
        let toned_after_contrast = live(helpers::load_composite_toned);
        let again = arcsinh_stretch_composite_cmd(out, 20.0, Some("base".into()), Some(display_stf(true))).await;
        let after_again = state().await;
        let toned_after_again = live(helpers::load_composite_toned);
        helpers::clear_composite();

        first.unwrap();
        let contrast = contrast.unwrap();
        assert_eq!(contrast[RES_DISPLAYED], "toned");
        assert_eq!(contrast[RES_CHAIN_INPUT], "stretch");
        assert!(contrast[RES_BASE_PNG_PATH].is_null(), "a valid chain does not capture a new base");
        assert_eq!(steps(&after_contrast), vec!["stretch", "localContrast"]);
        assert!(toned_after_contrast.is_some());

        let again = again.unwrap();
        assert_eq!(again[RES_CHAIN_RESTARTED], false);
        assert_eq!(steps(&after_again), vec!["stretch"]);
        assert!(toned_after_again.is_none(), "re-running the stretch must drop the local contrast output");
    }

    #[tokio::test]
    async fn reset_restores_the_base_state_exactly() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let k = planes();
        let balanced = install_balanced_composite(&k);
        let stretched = planes().map(|p| p.mapv(f32::sqrt));
        let toned = planes().map(|p| p.mapv(|v| v * 0.5));
        helpers::insert_composite_stretched(stretched[0].clone(), stretched[1].clone(), stretched[2].clone());
        helpers::insert_composite_toned(toned[0].clone(), toned[1].clone(), toned[2].clone());
        forget_chain();

        let call = ChainCall { output_dir: out.clone(), chain_input: "base".into(), display_stf: Some(display_stf(true)) };
        let linear = run_chain_step(ChainStep::Denoise, &call, |input, _| doubled_key(input));
        let key_after_linear = live_key();
        let orig_after_linear = live_orig();
        let stretched_after_linear = live(helpers::load_composite_stretched);
        let toned_after_linear = live(helpers::load_composite_toned);
        let reset = composite_chain_reset_cmd().await;
        let restored_orig = live_orig();
        let restored_key = live_key();
        let restored_wb = helpers::composite_wb_factors();
        let restored_stretched = live(helpers::load_composite_stretched);
        let restored_toned = live(helpers::load_composite_toned);
        let state_after_reset = state().await;

        install_composite(&k);
        let stretch = arcsinh_stretch_composite_cmd(out, 50.0, Some("base".into()), Some(display_stf(true))).await;
        let reset_linear = composite_chain_reset_cmd().await;
        let stretched_after_linear_reset = live(helpers::load_composite_stretched);
        let key_after_linear_reset = live_key();
        helpers::clear_composite();

        linear.unwrap();
        assert_eq!(key_after_linear, balanced.each_ref().map(|p| p.mapv(|v| v * 2.0)), "precondition: the linear step rewrote KEY");
        assert_ne!(orig_after_linear, k, "precondition: the linear step rewrote ORIG");
        assert!(stretched_after_linear.is_none() && toned_after_linear.is_none(), "precondition: the linear step cleared the tiers");
        let reset = reset.unwrap();
        assert_eq!(reset[RES_RESTORED], true);
        assert_eq!(reset[RES_LIVE_GENERATION], state_after_reset[RES_LIVE_GENERATION]);
        assert!(state_after_reset[RES_CHAIN_GENERATION].is_null());
        assert!(steps(&state_after_reset).is_empty());
        assert_eq!(restored_orig, k, "ORIG was not restored");
        assert_eq!(restored_key, balanced, "KEY was not restored");
        assert_eq!(restored_wb, WB, "the white balance factors were not restored");
        assert_eq!(restored_stretched, Some(stretched), "STRETCHED was not restored");
        assert_eq!(restored_toned, Some(toned), "TONED was not restored");

        stretch.unwrap();
        assert_eq!(reset_linear.unwrap()[RES_RESTORED], true);
        assert!(stretched_after_linear_reset.is_none(), "a tier absent from the base must be removed by reset");
        assert_eq!(key_after_linear_reset, k);
    }

    #[tokio::test]
    async fn reset_without_a_valid_chain_leaves_the_live_composite_alone() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let k = planes();
        install_composite(&k);
        let stretch = arcsinh_stretch_composite_cmd(out_dir(&dir), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let k2 = [plane(7), plane(8), plane(9)];
        helpers::insert_composite_and_orig(k2[0].clone(), k2[1].clone(), k2[2].clone(), compute_image_stats(&k2[0]), compute_image_stats(&k2[1]), compute_image_stats(&k2[2]));
        let reset = composite_chain_reset_cmd().await;
        let key = live_key();
        let state = state().await;
        helpers::clear_composite();
        stretch.unwrap();
        assert_eq!(reset.unwrap()[RES_RESTORED], false);
        assert_eq!(key, k2, "reset restored a base that predates an outside write");
        assert!(steps(&state).is_empty());
    }

    #[tokio::test]
    async fn an_outside_write_restarts_the_chain_from_the_live_composite() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        install_composite(&planes());
        let stretch = arcsinh_stretch_composite_cmd(out.clone(), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let before_write = state().await;

        let k2 = [plane(7), plane(8), plane(9)];
        outside_write(&k2);
        let live_after_write = helpers::composite_generation();

        let pixelmath = pixelmath_composite_cmd(out, "stretch".into(), display_stf(true), "$T * 2".into(), vec![], None, None).await;
        let key = live_key();
        let after_restart = state().await;
        helpers::clear_composite();

        stretch.unwrap();
        assert_eq!(before_write[RES_CHAIN_GENERATION], before_write[RES_LIVE_GENERATION]);
        assert_ne!(before_write[RES_LIVE_GENERATION].as_u64(), Some(live_after_write), "an outside write must move the live generation");
        let pixelmath = pixelmath.unwrap();
        assert_eq!(pixelmath[RES_CHAIN_RESTARTED], true);
        assert_eq!(pixelmath[RES_CHAIN_INPUT], "base");
        assert!(pixelmath[RES_BASE_PNG_PATH].as_str().is_some_and(|p| std::path::Path::new(p).exists()));
        assert_eq!(pixelmath[RES_DISPLAYED], "linear");
        assert_eq!(key, k2.map(|p| p.mapv(|v| v * 2.0)), "the restarted chain must start from the live composite");
        assert_eq!(steps(&after_restart), vec!["pixelMath"]);
        assert_eq!(after_restart[RES_CHAIN_GENERATION], after_restart[RES_LIVE_GENERATION]);
    }

    #[tokio::test]
    async fn the_commit_refuses_to_overwrite_a_composite_changed_mid_step() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let k = planes();
        install_composite(&k);
        let (snapshot, generation) = helpers::snapshot_composite().unwrap();
        let outside = planes().map(|p| p.mapv(f32::sqrt));
        helpers::insert_composite_stretched(outside[0].clone(), outside[1].clone(), outside[2].clone());
        let refused = helpers::commit_composite(&snapshot, Some(generation));
        let stretched_after_refusal = live(helpers::load_composite_stretched);
        let accepted = helpers::commit_composite(&snapshot, Some(helpers::composite_generation()));
        let stretched_after_commit = live(helpers::load_composite_stretched);

        let call = ChainCall { output_dir: out_dir(&dir), chain_input: "base".into(), display_stf: Some(display_stf(true)) };
        let raced = run_chain_step(ChainStep::Background, &call, |input, dir| {
            let model_png = chain_png_path(dir, MODEL_SUFFIX);
            std::fs::write(&model_png, b"model").unwrap();
            helpers::insert_composite_stretched(outside[0].clone(), outside[1].clone(), outside[2].clone());
            let [r, g, b] = key_planes(input);
            Ok(StepOutput {
                state: linear_state(input, [r.clone(), g.clone(), b.clone()]),
                extras: json!({ RES_MODEL_PNG_PATH: model_png }),
            })
        });
        let stretched_after_race = live(helpers::load_composite_stretched);
        let state = state().await;
        helpers::clear_composite();

        assert_eq!(refused.unwrap_err().to_string(), "The composite changed while this step ran; run it again.");
        assert_eq!(stretched_after_refusal, Some(outside.clone()), "a refused commit must not touch the live composite");
        assert_eq!(accepted.unwrap(), generation + 2, "a commit at the current generation succeeds and bumps it");
        assert!(stretched_after_commit.is_none(), "the committed snapshot had no stretched tier");

        assert_eq!(raced.unwrap_err().to_string(), "The composite changed while this step ran; run it again.");
        assert_eq!(stretched_after_race, Some(outside), "the step output must be discarded");
        assert!(steps(&state).is_empty(), "a refused step must not be recorded");
        assert!(state[RES_CHAIN_GENERATION].is_null(), "a refused first step must not install a base");
        assert_eq!(file_count(&dir), 0, "the discarded step left its PNGs behind");
    }

    #[tokio::test]
    async fn a_refused_commit_drops_the_stale_chain() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        install_composite(&planes());
        let stretch = arcsinh_stretch_composite_cmd(out.clone(), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let held = live_key_r_weak();
        let files_before_race = file_count(&dir);
        let k2 = [plane(7), plane(8), plane(9)];

        let call = ChainCall { output_dir: out, chain_input: "stretch".into(), display_stf: Some(display_stf(true)) };
        let raced = run_chain_step(ChainStep::LocalContrast, &call, |input, _| {
            outside_write(&k2);
            let [r, g, b] = contrast_planes(input)?;
            Ok(StepOutput { state: toned_state(input, [r.clone(), g.clone(), b.clone()]), extras: json!({}) })
        });
        let released = held.upgrade().is_none();
        let files_after_race = file_count(&dir);
        let state = state().await;
        helpers::clear_composite();

        stretch.unwrap();
        assert_eq!(raced.unwrap_err().to_string(), "The composite changed while this step ran; run it again.");
        assert!(released, "a chain refused at commit is stale and must free its snapshots");
        assert!(steps(&state).is_empty());
        assert_eq!(files_after_race, files_before_race, "the refused step left its PNG behind");
    }

    #[tokio::test]
    async fn a_step_that_fails_after_capturing_the_base_removes_the_base_png() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        install_composite(&planes());
        let lhe = LheConfig { kernel_radius: 3, ..LheConfig::default() };
        let failed = lhe_composite_cmd(out_dir(&dir), lhe, Some("base".into()), Some(display_stf(true))).await;
        let leftovers = file_count(&dir);
        let state = state().await;
        helpers::clear_composite();

        assert_eq!(failed.unwrap_err(), COMPOSITE_STRETCH_REQUIRED);
        assert_eq!(leftovers, 0, "the failed step left its base PNG behind");
        assert!(state[RES_CHAIN_GENERATION].is_null(), "a failed first step must not install a base");
    }

    #[tokio::test]
    async fn clearing_the_composite_cache_drops_the_chain_snapshots() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        install_composite(&planes());
        let stretch = arcsinh_stretch_composite_cmd(out_dir(&dir), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let held = live_key_r_weak();
        let held_before_clear = held.upgrade().is_some();
        let cleared = clear_composite_cache_cmd().await;
        let released = held.upgrade().is_none();
        let state = state().await;

        stretch.unwrap();
        cleared.unwrap();
        assert!(held_before_clear, "precondition: the chain shares the live key planes");
        assert!(released, "clearing the composite must free the chain snapshots");
        assert!(state[RES_CHAIN_GENERATION].is_null());
        assert!(steps(&state).is_empty());
    }

    #[tokio::test]
    async fn the_state_query_drops_a_chain_the_live_composite_has_outgrown() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        install_composite(&planes());
        let stretch = arcsinh_stretch_composite_cmd(out.clone(), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let held = live_key_r_weak();
        let k2 = [plane(7), plane(8), plane(9)];
        outside_write(&k2);
        let pinned_by_chain = held.upgrade().is_some();
        let stale = state().await;
        let released = held.upgrade().is_none();
        let next = arcsinh_stretch_composite_cmd(out, 20.0, Some("base".into()), Some(display_stf(true))).await;
        let key = live_key();
        let fresh = state().await;
        helpers::clear_composite();

        stretch.unwrap();
        assert!(pinned_by_chain, "precondition: only the chain still holds the outgrown planes");
        assert!(stale[RES_CHAIN_GENERATION].is_null(), "a stale chain reports no generation");
        assert!(steps(&stale).is_empty());
        assert!(released, "the state query must free a stale chain");
        let next = next.unwrap();
        assert_eq!(next[RES_CHAIN_RESTARTED], false, "a chain already dropped starts fresh instead of restarting");
        assert!(next[RES_BASE_PNG_PATH].as_str().is_some_and(|p| std::path::Path::new(p).exists()));
        assert_eq!(key, k2, "the fresh chain starts from the live composite");
        assert_eq!(steps(&fresh), vec!["stretch"]);
        assert_eq!(fresh[RES_CHAIN_GENERATION], fresh[RES_LIVE_GENERATION]);
    }

    #[tokio::test]
    async fn a_missing_named_snapshot_restarts_a_valid_chain() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let k = planes();
        install_composite(&k);
        let stretch = arcsinh_stretch_composite_cmd(out.clone(), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let masked = masked_stretch_composite_cmd(out, Some(2), None, None, None, None, None, None, None, None, Some("deconv".into()), Some(display_stf(true))).await;
        let key = live_key();
        let state = state().await;
        helpers::clear_composite();

        stretch.unwrap();
        let masked = masked.unwrap();
        assert_eq!(masked[RES_CHAIN_RESTARTED], true, "a valid chain without the named snapshot restarts");
        assert_eq!(masked[RES_CHAIN_INPUT], "base");
        assert!(masked[RES_BASE_PNG_PATH].as_str().is_some_and(|p| std::path::Path::new(p).exists()));
        assert_eq!(masked[RES_DISPLAYED], "stretched");
        assert_eq!(key, k, "the new base is the live composite, whose key tier the stretch left unchanged");
        assert_eq!(steps(&state), vec!["maskedStretch"]);
        assert_eq!(state[RES_CHAIN_GENERATION], state[RES_LIVE_GENERATION]);
    }

    #[tokio::test]
    async fn a_linear_step_writes_the_key_tier_with_orig_divided_by_the_white_balance_and_clears_the_tiers() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let k = planes();
        let balanced = install_balanced_composite(&k);
        let stretched = planes().map(|p| p.mapv(f32::sqrt));
        helpers::insert_composite_stretched(stretched[0].clone(), stretched[1].clone(), stretched[2].clone());
        helpers::insert_composite_toned(stretched[0].clone(), stretched[1].clone(), stretched[2].clone());
        forget_chain();
        let config = WaveletConfig { num_scales: 2, thresholds: vec![1.0], linear_denoise: true, layer_bias: None };

        let linked = ChainCall { output_dir: out_dir(&dir), chain_input: "base".into(), display_stf: Some(display_stf(true)) };
        let value = run_chain_step(ChainStep::Denoise, &linked, |input, _| denoise_step(input, &config, [None; 3]));
        let key = live_key();
        let orig = live_orig();
        let wb = helpers::composite_wb_factors();
        let stretched_after = live(helpers::load_composite_stretched);
        let toned_after = live(helpers::load_composite_toned);
        let state = state().await;

        let unlinked = ChainCall { output_dir: out_dir(&dir), chain_input: "base".into(), display_stf: Some(display_stf(false)) };
        let value_unlinked = run_chain_step(ChainStep::Denoise, &unlinked, |input, _| denoise_step(input, &config, [None; 3]));
        helpers::clear_composite();

        let value = value.unwrap();
        let expected = balanced.each_ref().map(|p| wavelet_denoise(p, &config, None).unwrap().denoised);
        assert_eq!(key, expected, "the linear step must read and write the key tier");
        let expected_orig = [0, 1, 2].map(|i| expected[i].mapv(|v| v * (1.0 / WB[i])));
        assert_eq!(orig, expected_orig, "ORIG must be KEY divided by the white balance");
        assert_eq!(wb, WB, "the white balance factors must survive a linear step");
        assert!(stretched_after.is_none() && toned_after.is_none(), "a linear step must clear the derived tiers");
        assert_eq!(value[RES_DISPLAYED], "linear");
        assert_eq!(value[RES_STF][RES_LINKED], true);
        assert_eq!(value[RES_STF][RES_R], value[RES_STF][RES_G], "a linked STF is shared by the channels");
        assert_eq!(value[RES_SCALES_PROCESSED], 2);
        assert_eq!(value[RES_NOISE_ESTIMATE].as_array().unwrap().len(), 3);
        assert_eq!(steps(&state), vec!["denoise"]);
        assert_eq!(png_size(value[RES_PNG_PATH].as_str().unwrap()), (16, 16));

        let value_unlinked = value_unlinked.unwrap();
        assert_eq!(value_unlinked[RES_STF][RES_LINKED], false);
        assert_eq!(value_unlinked[RES_CHAIN_RESTARTED], false, "the chain stayed valid across its own commits");
    }

    #[tokio::test]
    async fn local_contrast_reads_the_tier_of_its_input_state_not_the_live_one() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let k = planes();
        install_composite(&k);
        let lhe = LheConfig { kernel_radius: 3, ..LheConfig::default() };
        let hdr = HdrConfig { layers: 2, ..HdrConfig::default() };

        let stretch = arcsinh_stretch_composite_cmd(out.clone(), 50.0, Some("base".into()), Some(display_stf(true))).await;
        let s1 = live(helpers::load_composite_stretched);
        let lhe_run = lhe_composite_cmd(out.clone(), lhe.clone(), Some("stretch".into()), Some(display_stf(true))).await;
        let l1 = live(helpers::load_composite_toned);
        let hdr_run = hdrmt_composite_cmd(out.clone(), hdr.clone(), Some("stretch".into()), Some(display_stf(true))).await;
        let after_hdr = live(helpers::load_composite_toned);
        let stretched_after_hdr = live(helpers::load_composite_stretched);
        let state = state().await;
        let on_linear = lhe_composite_cmd(out, lhe.clone(), Some("base".into()), Some(display_stf(true))).await;
        helpers::clear_composite();

        stretch.unwrap();
        lhe_run.unwrap();
        hdr_run.unwrap();
        let s1 = s1.expect("stretched tier");
        let l1 = l1.expect("toned tier after LHE");
        assert_eq!(l1, triplet(lhe_rgb(&s1[0], &s1[1], &s1[2], &lhe).unwrap()));
        let expected = triplet(hdrmt_rgb(&s1[0], &s1[1], &s1[2], &hdr).unwrap());
        let wrong = triplet(hdrmt_rgb(&l1[0], &l1[1], &l1[2], &hdr).unwrap());
        assert_eq!(after_hdr.as_ref(), Some(&expected), "HDRMT with the stretch input must read that state's stretched tier");
        assert_ne!(after_hdr.as_ref(), Some(&wrong), "HDRMT compounded on the live toned tier");
        assert_eq!(stretched_after_hdr, Some(s1));
        assert_eq!(steps(&state), vec!["stretch", "localContrast"]);
        assert_eq!(on_linear.unwrap_err(), COMPOSITE_STRETCH_REQUIRED, "a linear input has no tier for local contrast");
    }

    #[tokio::test]
    async fn pixelmath_per_channel_binds_the_target_to_each_channel_and_refuses_a_slot_of_another_size() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let k = planes();
        install_composite(&k);
        let small = dir.path().join("small.fits").to_str().unwrap().to_string();
        write_fits_mono(&small, &Array2::from_elem((8, 8), 1.0f32), None).unwrap();
        let ones = dir.path().join("ones.fits").to_str().unwrap().to_string();
        write_fits_mono(&ones, &Array2::from_elem((16, 16), 1.0f32), None).unwrap();

        let doubled = pixelmath_composite_cmd(out.clone(), "base".into(), display_stf(false), "$T * 2".into(), vec![], None, None).await;
        let key_doubled = live_key();
        let refused = pixelmath_composite_cmd(out.clone(), "pixelMath".into(), display_stf(false), "$T + A".into(), vec![slot("A", &small)], None, None).await;
        let key_after_refusal = live_key();
        let added = pixelmath_composite_cmd(out, "pixelMath".into(), display_stf(false), "$T + A".into(), vec![slot("A", &ones)], None, None).await;
        let key_added = live_key();
        let state = state().await;
        helpers::clear_composite();

        let doubled = doubled.unwrap();
        assert_eq!(key_doubled, k.each_ref().map(|p| p.mapv(|v| v * 2.0)), "$T must be bound to each channel in turn");
        assert_eq!(doubled[RES_DISPLAYED], "linear");
        assert_eq!(doubled[RES_STF][RES_LINKED], false);
        assert_eq!(doubled[RES_NON_FINITE_COUNT], json!([0, 0, 0]));
        for (channel, plane) in [RES_R, RES_G, RES_B].into_iter().zip(&k) {
            let expected_max = compute_image_stats(plane).max * 2.0;
            assert!((doubled[RES_STATS][channel][RES_MAX].as_f64().unwrap() - expected_max).abs() < 1e-6, "{channel}");
        }
        assert_eq!(doubled[RES_WARNINGS], json!([]));

        let refused = refused.unwrap_err();
        assert!(refused.contains("slot A is 8x8 but the composite is 16x16"), "{refused}");
        assert_eq!(key_after_refusal, key_doubled, "a refused slot must not change the composite");

        added.unwrap();
        assert_eq!(key_added, k.each_ref().map(|p| p.mapv(|v| v * 2.0 + 1.0)), "PixelMath must compound on its own output");
        assert_eq!(steps(&state), vec!["pixelMath"]);
    }

    fn sky(seed: usize) -> Array2<f32> {
        Array2::from_shape_fn((64, 64), |(y, x)| {
            let noise = ((y * 31 + x * 17 + seed * 7) % 23) as f32 - 11.0;
            100.0 + 0.5 * x as f32 + 0.25 * y as f32 + noise + 30.0 * seed as f32
        })
    }

    #[tokio::test]
    async fn the_background_step_corrects_each_channel_and_renders_the_colour_model() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let k = [sky(1), sky(2), sky(3)];
        install_composite(&k);
        let call = ChainCall { output_dir: out_dir(&dir), chain_input: "base".into(), display_stf: Some(display_stf(true)) };

        let polynomial = background_model("polynomial", 8, 2, 3.0, 3, "subtract", None).unwrap();
        let poly = run_chain_step(ChainStep::Background, &call, |input, dir| background_step(input, &polynomial, [None; 3], dir));
        let key_after_poly = live_key();

        let dbe = DbeConfig { auto_grid: Some(4), manual_samples: vec![(1.0, 1.0), (2.0, 2.0)], ..DbeConfig::default() };
        let spline = background_model("spline", 8, 2, 3.0, 3, "subtract", Some(dbe.clone())).unwrap();
        let spline_run = run_chain_step(ChainStep::Background, &call, |input, dir| background_step(input, &spline, [None; 3], dir));
        let key_after_spline = live_key();
        let state = state().await;
        helpers::clear_composite();

        let poly = poly.unwrap();
        let BackgroundModel::Polynomial(config) = &polynomial else { panic!("polynomial model expected") };
        let expected = k.each_ref().map(|p| extract_background(p, config, None).unwrap());
        assert_eq!(key_after_poly, expected.each_ref().map(|r| r.corrected.clone()), "each channel must be corrected with the mono code");
        assert_eq!(poly[RES_SAMPLE_COUNT], json!(expected.each_ref().map(|r| r.sample_count)));
        assert_eq!(poly[RES_RMS_RESIDUAL].as_array().unwrap().len(), 3);
        assert!(poly[RES_DBE_REJECTED_COUNT].is_null(), "the polynomial model rejects nothing");
        let model_png = poly[RES_MODEL_PNG_PATH].as_str().unwrap();
        assert!(model_png.contains("composite_chain_bgmodel_"), "{model_png}");
        assert_eq!(png_size(model_png), (64, 64));
        assert_eq!(poly[RES_DISPLAYED], "linear");
        assert_eq!(poly[RES_STF][RES_LINKED], true);

        let spline_run = spline_run.unwrap();
        let BackgroundModel::Spline(used) = &spline else { panic!("spline model expected") };
        assert!(used.manual_samples.is_empty(), "point samples belong to the file grid and must be ignored");
        let expected = k.each_ref().map(|p| extract_background_dbe(p, used, None).unwrap());
        assert_eq!(key_after_spline, expected.each_ref().map(|r| r.corrected.clone()));
        assert_eq!(spline_run[RES_DBE_REJECTED_COUNT], json!(expected.each_ref().map(|r| r.rejected_count)));
        assert_eq!(spline_run[RES_CHAIN_RESTARTED], false, "re-running the same step keeps the chain");
        assert_eq!(steps(&state), vec!["background"]);

        let missing = background_model("spline", 8, 2, 3.0, 3, "subtract", None).err().expect("spline without DBE settings").to_string();
        assert!(missing.contains("DBE settings"), "{missing}");
        let unknown = background_model("wavelet", 8, 2, 3.0, 3, "subtract", None).err().expect("unknown model").to_string();
        assert!(unknown.contains("Unknown background model wavelet"), "{unknown}");
    }

    fn tilted_sky(offset: f32, seed: usize) -> Array2<f32> {
        Array2::from_shape_fn((64, 64), |(y, x)| {
            let noise = ((y * 31 + x * 17 + seed * 7) % 23) as f32 - 11.0;
            offset + 0.5 * x as f32 + 0.25 * y as f32 + 0.05 * noise
        })
    }

    fn png_channel_spread(path: &str) -> [(u8, u8); 3] {
        let img = image::open(path).unwrap().to_rgb8();
        let mut lo = [u8::MAX; 3];
        let mut hi = [u8::MIN; 3];
        for px in img.pixels() {
            for c in 0..3 {
                lo[c] = lo[c].min(px[c]);
                hi[c] = hi[c].max(px[c]);
            }
        }
        [0, 1, 2].map(|c| (lo[c], hi[c]))
    }

    #[tokio::test]
    async fn the_background_model_png_shows_every_channel_gradient_when_the_channel_levels_differ() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        install_composite(&[tilted_sky(1000.0, 1), tilted_sky(50.0, 2), tilted_sky(5.0, 3)]);
        let call = ChainCall { output_dir: out_dir(&dir), chain_input: "base".into(), display_stf: Some(display_stf(true)) };
        let polynomial = background_model("polynomial", 8, 2, 3.0, 3, "subtract", None).unwrap();
        let value = run_chain_step(ChainStep::Background, &call, |input, dir| background_step(input, &polynomial, [None; 3], dir));
        helpers::clear_composite();

        let value = value.unwrap();
        let spread = png_channel_spread(value[RES_MODEL_PNG_PATH].as_str().unwrap());
        for (channel, (lo, hi)) in [RES_R, RES_G, RES_B].into_iter().zip(spread) {
            assert!(hi - lo >= 64, "{channel} model gradient is not visible in the PNG: min {lo}, max {hi}");
        }
    }

    fn starry(seed: f32) -> Array2<f32> {
        Array2::from_shape_fn((160, 160), |(y, x)| {
            let mut v = 0.02 + ((y * 31 + x * 17) % 13) as f32 * 0.0005 * seed;
            for (cy, cx) in [(80.0f32, 80.0f32), (60.0, 110.0), (110.0, 60.0), (95.0, 100.0)] {
                let d2 = (y as f32 - cy).powi(2) + (x as f32 - cx).powi(2);
                v += 0.6 * seed * (-d2 / 4.5).exp();
            }
            v
        })
    }

    #[tokio::test]
    async fn the_deconvolution_step_uses_a_provided_kernel_and_reports_the_psf_source() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let k = planes();
        install_composite(&k);
        let rows = vec![vec![1.0f32, 2.0, 1.0], vec![2.0, 4.0, 2.0], vec![1.0, 2.0, 1.0]];
        let config = RLConfig { iterations: 3, ..RLConfig::default() };
        let call = ChainCall { output_dir: out_dir(&dir), chain_input: "base".into(), display_stf: Some(display_stf(true)) };

        let provided = DeconvPsf::choose(Some(true), Some(rows.clone()), 15, 2.0, None, None).unwrap();
        let value = run_chain_step(ChainStep::Deconv, &call, |input, _| deconv_step(input, &provided, &config, [None; 3]));
        let key_provided = live_key();
        let gaussian = DeconvPsf::choose(Some(false), Some(rows.clone()), 5, 1.0, None, None).unwrap();
        let value_gaussian = run_chain_step(ChainStep::Deconv, &call, |input, _| deconv_step(input, &gaussian, &config, [None; 3]));
        let key_gaussian = live_key();
        let state = state().await;
        helpers::clear_composite();

        let value = value.unwrap();
        assert_eq!(value[RES_PSF_SOURCE], "provided");
        assert_eq!(value[RES_DISPLAYED], "linear");
        assert_eq!(value[RES_ITERATIONS_RUN].as_array().unwrap().len(), 3);
        let kernel = provided_psf_kernel(&rows).unwrap();
        let expected = k.each_ref().map(|p| richardson_lucy(p, &kernel, &config, None).unwrap().image);
        assert_eq!(key_provided, expected, "each channel must be deconvolved with the provided kernel");

        let value_gaussian = value_gaussian.unwrap();
        assert_eq!(value_gaussian[RES_PSF_SOURCE], "gaussian");
        let expected = k.each_ref().map(|p| richardson_lucy(p, &generate_gaussian_psf(5, 1.0), &config, None).unwrap().image);
        assert_eq!(key_gaussian, expected, "a Gaussian PSF must ignore the provided kernel");
        assert_eq!(steps(&state), vec!["deconv"]);
    }

    #[tokio::test]
    async fn the_composite_psf_is_estimated_on_the_key_luminance_of_the_requested_state() {
        let _guard = helpers::composite_test_lock().await;
        let k = [starry(1.0), starry(0.8), starry(1.2)];
        install_composite(&k);
        let value = composite_estimate_psf_cmd("base".into(), Some(5), Some(6), None, None).await;
        let snapshot = helpers::snapshot_composite().map(|(s, _)| s);
        helpers::clear_composite();

        let value = value.unwrap();
        let luminance = key_luminance(&snapshot.unwrap().key).unwrap();
        let expected = estimate_psf(&luminance, &psf_config(Some(5), Some(6), None, None)).unwrap();
        assert_eq!(value, psf_result_json(&expected));
        assert!(value[RES_KERNEL_SIZE].as_u64().unwrap() > 0);
    }
}
