use ndarray::{Array2, Array3};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use crate::core::stacking::combine::{reject_and_combine_with, KernelScratch, Sample};
use crate::math::sigma_clip::sigma_clipped_stats;
use crate::types::stacking::{CombineMethod, RejectionMethod, RejectionParams};
use crate::core::imaging::cosmetic::{
    apply_cosmetic, defect_map_auto, defect_map_from_dark, defect_map_from_list, merge_maps, CosmeticConfig,
};
use crate::core::imaging::stats::percentile;
use crate::core::imaging::stretch::{arcsinh_stretch_rgb_with_stats, arcsinh_stretch_with_stats};
use crate::infra::image_source::{is_dq_name, list_planes};
use crate::math::median::{exact_mad_mut, median_f32_mut};
use crate::types::constants::MAD_TO_SIGMA;

const PREVIEW_STRETCH_FACTOR: f32 = 20.0;
pub const DQ_COSMETIC_WARNING: &str = "cosmetic correction applied to data with DQ planes";
const DARK_OPTIMIZE_MAX_SAMPLES: usize = 262_144;
const DARK_OPTIMIZE_SCALE_MAX: f64 = 3.0;
const DARK_OPTIMIZE_TOLERANCE: f64 = 1e-3;
const DARK_OPTIMIZE_MAX_EVALUATIONS: usize = 40;
const DARK_OPTIMIZE_STAR_SIGMA: f32 = 5.0;
const GOLDEN_RATIO_INVERSE: f64 = 0.618_033_988_749_895;

#[derive(Debug, Clone)]
pub struct CalibrationMasters {
    pub dark: Option<Array2<f32>>,
    pub flat: Option<Array2<f32>>,
    pub bias: Option<Array2<f32>>,
}

#[derive(Debug, Clone)]
pub struct ChannelInput {
    pub lights: Vec<Array2<f32>>,
    pub label: String,
    pub dark_scales: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct BatchStackConfig {
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub max_iterations: usize,
    pub normalize_before_stack: bool,
    pub rejection: RejectionMethod,
    pub combine: CombineMethod,
}

impl Default for BatchStackConfig {
    fn default() -> Self {
        Self {
            sigma_low: 2.5,
            sigma_high: 3.0,
            max_iterations: 5,
            normalize_before_stack: true,
            rejection: RejectionMethod::SigmaClip,
            combine: CombineMethod::Mean,
        }
    }
}

impl BatchStackConfig {
    pub fn rejection_params(&self) -> RejectionParams {
        RejectionParams {
            rejection: self.rejection,
            combine: self.combine,
            sigma_low: self.sigma_low,
            sigma_high: self.sigma_high,
            max_iterations: self.max_iterations,
            ..RejectionParams::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct BatchPipelineConfig {
    pub stack: BatchStackConfig,
    pub align: bool,
    pub cosmetic: Option<CosmeticConfig>,
    pub dark_optimize: bool,
}

impl Default for BatchPipelineConfig {
    fn default() -> Self {
        Self {
            stack: BatchStackConfig::default(),
            align: true,
            cosmetic: None,
            dark_optimize: false,
        }
    }
}

#[derive(Debug)]
pub struct BatchPipelineResult {
    pub master_channels: Vec<(String, Array2<f32>)>,
    pub rgb: Option<Array3<f32>>,
    pub stats: BatchPipelineStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPipelineStats {
    pub darks_combined: usize,
    pub flats_combined: usize,
    pub bias_combined: usize,
    pub channels: Vec<BatchChannelStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchChannelStats {
    pub label: String,
    pub lights_input: usize,
    pub lights_after_rejection: Vec<usize>,
    pub mean: f64,
    pub stddev: f64,
    #[serde(default)]
    pub cosmetic_replaced: Option<u64>,
    #[serde(default)]
    pub dark_scale_min: Option<f32>,
    #[serde(default)]
    pub dark_scale_max: Option<f32>,
    #[serde(default)]
    pub dark_scale_mean: Option<f32>,
}

fn master_slice(master: Option<&Array2<f32>>, npix: usize) -> Option<&[f32]> {
    master
        .map(|m| m.as_slice().expect("contiguous"))
        .filter(|s| s.len() == npix)
}

fn apply_masters(
    light: &Array2<f32>,
    bias: Option<&[f32]>,
    dark: Option<&[f32]>,
    flat: Option<&[f32]>,
    dark_scale: f32,
) -> Array2<f32> {
    let (rows, cols) = light.dim();
    let src = light.as_slice().expect("contiguous");

    let result: Vec<f32> = (0..rows * cols)
        .into_par_iter()
        .map(|i| {
            let mut v = src[i];
            if let Some(b) = bias {
                v -= b[i];
            }
            if let Some(d) = dark {
                v -= d[i] * dark_scale;
            }
            if let Some(f) = flat {
                let fv = f[i];
                if fv.is_finite() && fv.abs() > 1e-4 {
                    v /= fv;
                }
            }
            v
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result).unwrap()
}

pub fn calibrate_light(
    light: &Array2<f32>,
    masters: &CalibrationMasters,
    dark_scale: f32,
) -> Array2<f32> {
    let npix = light.len();
    apply_masters(
        light,
        master_slice(masters.bias.as_ref(), npix),
        master_slice(masters.dark.as_ref(), npix),
        master_slice(masters.flat.as_ref(), npix),
        dark_scale,
    )
}

pub fn calibrate_light_with_cosmetic(
    light: &Array2<f32>,
    masters: &CalibrationMasters,
    dark_scale: f32,
    cosmetic: Option<&ChannelCosmetic>,
) -> (Array2<f32>, usize) {
    let Some(cosmetic) = cosmetic else {
        return (calibrate_light(light, masters, dark_scale), 0);
    };
    let npix = light.len();
    let bias = master_slice(masters.bias.as_ref(), npix);
    let dark = master_slice(masters.dark.as_ref(), npix);
    let flat = master_slice(masters.flat.as_ref(), npix);

    let dark_subtracted = apply_masters(light, bias, dark, None, dark_scale);
    let (corrected, replaced) = cosmetic.correct(&dark_subtracted);
    let calibrated = match flat {
        Some(_) => apply_masters(&corrected, None, None, flat, dark_scale),
        None => corrected,
    };
    (calibrated, replaced)
}

pub fn validate_cosmetic_config(config: &CosmeticConfig) -> Result<(), String> {
    config.validate().map_err(|e| format!("Cosmetic correction: {:#}", e))
}

struct DarkOptimizeSample {
    light_minus_bias: Vec<f32>,
    dark: Vec<f32>,
}

fn dark_optimize_sample(light: &[f32], bias: Option<&[f32]>, dark: &[f32]) -> DarkOptimizeSample {
    let npix = light.len();
    let stride = npix.div_ceil(DARK_OPTIMIZE_MAX_SAMPLES).max(1);
    let capacity = npix / stride + 1;
    let mut light_values = Vec::with_capacity(capacity);
    let mut light_minus_bias = Vec::with_capacity(capacity);
    let mut dark_values = Vec::with_capacity(capacity);
    for i in (0..npix).step_by(stride) {
        let l = light[i];
        let d = dark[i];
        let b = bias.map_or(0.0, |b| b[i]);
        if !(l.is_finite() && d.is_finite() && b.is_finite()) {
            continue;
        }
        light_values.push(l);
        light_minus_bias.push(l - b);
        dark_values.push(d);
    }
    if light_values.is_empty() {
        return DarkOptimizeSample { light_minus_bias, dark: dark_values };
    }
    let mut work = light_values.clone();
    let median = median_f32_mut(&mut work);
    let sigma = exact_mad_mut(&mut work, median) * MAD_TO_SIGMA as f32;
    let ceiling = if sigma > 0.0 { median + DARK_OPTIMIZE_STAR_SIGMA * sigma } else { f32::INFINITY };
    let mut background = DarkOptimizeSample {
        light_minus_bias: Vec::with_capacity(light_values.len()),
        dark: Vec::with_capacity(light_values.len()),
    };
    for ((&l, &lb), &d) in light_values.iter().zip(&light_minus_bias).zip(&dark_values) {
        if l <= ceiling {
            background.light_minus_bias.push(lb);
            background.dark.push(d);
        }
    }
    background
}

fn residual_robust_sigma(light_minus_bias: &[f32], dark: &[f32], scale: f32, work: &mut Vec<f32>) -> f64 {
    work.clear();
    work.extend(light_minus_bias.iter().zip(dark).map(|(&l, &d)| l - scale * d));
    let median = median_f32_mut(work);
    exact_mad_mut(work, median) as f64 * MAD_TO_SIGMA
}

fn golden_section_minimum(
    mut lo: f64,
    mut hi: f64,
    tolerance: f64,
    max_evaluations: usize,
    mut objective: impl FnMut(f64) -> f64,
) -> f64 {
    let mut c = hi - GOLDEN_RATIO_INVERSE * (hi - lo);
    let mut d = lo + GOLDEN_RATIO_INVERSE * (hi - lo);
    let mut fc = objective(c);
    let mut fd = objective(d);
    let mut evaluations = 2;
    while hi - lo > tolerance && evaluations < max_evaluations {
        if fc <= fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - GOLDEN_RATIO_INVERSE * (hi - lo);
            fc = objective(c);
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + GOLDEN_RATIO_INVERSE * (hi - lo);
            fd = objective(d);
        }
        evaluations += 1;
    }
    0.5 * (lo + hi)
}

pub fn optimize_dark_scale(light: &Array2<f32>, bias: Option<&Array2<f32>>, dark: &Array2<f32>) -> f32 {
    let npix = light.len();
    let Some(dark_slice) = master_slice(Some(dark), npix) else {
        return 1.0;
    };
    let light_slice = light.as_slice().expect("contiguous");
    let sample = dark_optimize_sample(light_slice, master_slice(bias, npix), dark_slice);
    if sample.dark.len() < 2 {
        return 1.0;
    }
    let mut work = Vec::with_capacity(sample.dark.len());
    let scale = golden_section_minimum(
        0.0,
        DARK_OPTIMIZE_SCALE_MAX,
        DARK_OPTIMIZE_TOLERANCE,
        DARK_OPTIMIZE_MAX_EVALUATIONS,
        |k| residual_robust_sigma(&sample.light_minus_bias, &sample.dark, k as f32, &mut work),
    );
    scale as f32
}

fn channel_dark_scales(channel: &ChannelInput, masters: &CalibrationMasters, optimize: bool) -> Vec<f32> {
    match masters.dark.as_ref() {
        Some(dark) if optimize => channel
            .lights
            .par_iter()
            .map(|light| optimize_dark_scale(light, masters.bias.as_ref(), dark))
            .collect(),
        _ => (0..channel.lights.len())
            .map(|i| channel.dark_scales.get(i).copied().unwrap_or(1.0))
            .collect(),
    }
}

fn dark_scale_summary(scales: &[f32], has_dark: bool) -> (Option<f32>, Option<f32>, Option<f32>) {
    if !has_dark || scales.is_empty() {
        return (None, None, None);
    }
    let min = scales.iter().copied().fold(f32::INFINITY, f32::min);
    let max = scales.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mean = scales.iter().map(|&s| s as f64).sum::<f64>() / scales.len() as f64;
    (Some(min), Some(max), Some(mean as f32))
}

#[derive(Debug, Clone)]
pub struct CosmeticPlan {
    config: CosmeticConfig,
    dark_map: Option<Array2<u8>>,
}

#[derive(Debug, Clone)]
pub struct ChannelCosmetic {
    config: CosmeticConfig,
    base_map: Option<Array2<u8>>,
}

impl CosmeticPlan {
    pub fn build(config: &CosmeticConfig, master_dark: Option<&Array2<f32>>) -> Result<Self, String> {
        validate_cosmetic_config(config)?;
        let dark_map = match master_dark {
            Some(dark) if config.use_master_dark => {
                Some(defect_map_from_dark(dark, config.dark_hot_sigma, config.dark_cold_sigma))
            }
            _ => None,
        };
        Ok(Self { config: config.clone(), dark_map })
    }

    pub fn for_dims(&self, dims: (usize, usize)) -> Result<ChannelCosmetic, String> {
        let mut maps: Vec<Array2<u8>> = Vec::new();
        if let Some(dark_map) = self.dark_map.as_ref().filter(|m| m.dim() == dims) {
            maps.push(dark_map.clone());
        }
        if !self.config.defects.is_empty() {
            let list_map = defect_map_from_list(&self.config.defects, dims.0, dims.1)
                .map_err(|e| format!("Cosmetic defect list: {:#}", e))?;
            maps.push(list_map);
        }
        let base_map = match maps.len() {
            0 => None,
            1 => maps.pop(),
            _ => Some(merge_maps(&maps.iter().collect::<Vec<_>>())),
        };
        Ok(ChannelCosmetic { config: self.config.clone(), base_map })
    }
}

impl ChannelCosmetic {
    fn auto_enabled(&self) -> bool {
        self.config.auto_hot_sigma.is_some() || self.config.auto_cold_sigma.is_some()
    }

    pub fn correct(&self, image: &Array2<f32>) -> (Array2<f32>, usize) {
        let auto_map = self.auto_enabled().then(|| {
            defect_map_auto(image, self.config.auto_hot_sigma, self.config.auto_cold_sigma, self.config.cfa)
        });
        let merged;
        let map: &Array2<u8> = match (self.base_map.as_ref(), auto_map.as_ref()) {
            (Some(base), Some(auto)) => {
                merged = merge_maps(&[base, auto]);
                &merged
            }
            (Some(base), None) => base,
            (None, Some(auto)) => auto,
            (None, None) => return (image.clone(), 0),
        };
        apply_cosmetic(image, map, self.config.replacement, self.config.amount, self.config.cfa)
    }
}

pub fn carries_dq_plane(path: &str) -> bool {
    match list_planes(path) {
        Ok((planes, _)) => planes
            .iter()
            .any(|plane| plane.extname.as_deref().is_some_and(is_dq_name)),
        Err(_) => false,
    }
}

pub fn lights_with_dq_planes<'a>(paths: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    paths.into_iter().filter(|p| carries_dq_plane(p)).cloned().collect()
}

pub fn run_batch_pipeline(
    channels: Vec<ChannelInput>,
    masters: &CalibrationMasters,
    config: &BatchPipelineConfig,
) -> Result<BatchPipelineResult, String> {
    if channels.is_empty() {
        return Err("No channels provided".into());
    }

    for ch in &channels {
        if ch.lights.is_empty() {
            return Err(format!("Channel '{}' has no lights", ch.label));
        }
        let ref_dim = ch.lights[0].dim();
        for (i, l) in ch.lights.iter().enumerate().skip(1) {
            if l.dim() != ref_dim {
                return Err(format!(
                    "Channel '{}': frame {} has shape {:?} but frame 0 has {:?}. All frames must match.",
                    ch.label, i, l.dim(), ref_dim
                ));
            }
        }
        let master_dims = [
            ("bias", masters.bias.as_ref().map(|m| m.dim())),
            ("dark", masters.dark.as_ref().map(|m| m.dim())),
            ("flat", masters.flat.as_ref().map(|m| m.dim())),
        ];
        for (name, dim) in master_dims {
            if let Some(d) = dim {
                if d != ref_dim {
                    return Err(format!(
                        "Master {} has shape {:?} but channel '{}' lights have {:?}. Calibration masters must match light dimensions.",
                        name, d, ch.label, ref_dim
                    ));
                }
            }
        }
    }

    let mut pipeline_stats = BatchPipelineStats {
        darks_combined: if masters.dark.is_some() { 1 } else { 0 },
        flats_combined: if masters.flat.is_some() { 1 } else { 0 },
        bias_combined: if masters.bias.is_some() { 1 } else { 0 },
        channels: Vec::new(),
    };

    let mut master_channels: Vec<(String, Array2<f32>)> = Vec::new();

    let cosmetic_plan = match &config.cosmetic {
        Some(cfg) => Some(CosmeticPlan::build(cfg, masters.dark.as_ref())?),
        None => None,
    };

    for channel in &channels {
        let channel_cosmetic = match &cosmetic_plan {
            Some(plan) => Some(plan.for_dims(channel.lights[0].dim())?),
            None => None,
        };
        let dark_scales = channel_dark_scales(channel, masters, config.dark_optimize);
        let (dark_scale_min, dark_scale_max, dark_scale_mean) =
            dark_scale_summary(&dark_scales, masters.dark.is_some());
        let (calibrated, replaced_counts): (Vec<Array2<f32>>, Vec<usize>) = channel
            .lights
            .par_iter()
            .enumerate()
            .map(|(i, l)| {
                calibrate_light_with_cosmetic(l, masters, dark_scales[i], channel_cosmetic.as_ref())
            })
            .unzip();
        let cosmetic_replaced = channel_cosmetic
            .as_ref()
            .map(|_| replaced_counts.iter().map(|&n| n as u64).sum());

        let registered = if config.align && calibrated.len() > 1 {
            let (rows, cols) = calibrated[0].dim();
            let reference = calibrated[0].clone();
            let rest: Vec<Array2<f32>> = calibrated[1..]
                .par_iter()
                .map(|target| {
                    let pc = crate::core::alignment::pair::align_pair(
                        &reference,
                        target,
                        crate::types::compose::AlignMethod::PhaseCorrelation,
                        rows,
                        cols,
                    );
                    match pc {
                        Ok(res) if res.method_used == "phase_correlation" => {
                            if res.offset.0.abs() < 0.05 && res.offset.1.abs() < 0.05 {
                                target.clone()
                            } else {
                                res.aligned
                            }
                        }
                        _ => {
                            match crate::core::alignment::pair::align_pair(
                                &reference,
                                target,
                                crate::types::compose::AlignMethod::Affine,
                                rows,
                                cols,
                            ) {
                                Ok(res) => res.aligned,
                                Err(_) => target.clone(),
                            }
                        }
                    }
                })
                .collect();
            let mut frames = Vec::with_capacity(calibrated.len());
            frames.push(reference);
            frames.extend(rest);
            frames
        } else {
            calibrated
        };

        let normalized = if config.stack.normalize_before_stack {
            normalize_frames(&registered)
        } else {
            registered
        };

        let (mut stacked, rejection_counts) =
            reject_and_combine_stack(&normalized, &config.stack);
        stacked.par_mapv_inplace(|v| if v < 0.0 { 0.0 } else { v });

        let mean_val = stacked.iter().map(|&v| v as f64).sum::<f64>() / stacked.len() as f64;
        let var: f64 = stacked
            .iter()
            .map(|&v| ((v as f64) - mean_val).powi(2))
            .sum::<f64>()
            / stacked.len() as f64;

        pipeline_stats.channels.push(BatchChannelStats {
            label: channel.label.clone(),
            lights_input: channel.lights.len(),
            lights_after_rejection: rejection_counts,
            mean: mean_val,
            stddev: var.sqrt(),
            cosmetic_replaced,
            dark_scale_min,
            dark_scale_max,
            dark_scale_mean,
        });

        master_channels.push((channel.label.clone(), stacked));
    }

    let rgb = compose_rgb_from_masters(&master_channels);

    Ok(BatchPipelineResult {
        master_channels,
        rgb,
        stats: pipeline_stats,
    })
}

fn compose_rgb_from_masters(masters: &[(String, Array2<f32>)]) -> Option<Array3<f32>> {
    let find = |label: &str| -> Option<&Array2<f32>> {
        masters.iter().find(|(l, _)| l.eq_ignore_ascii_case(label)).map(|(_, arr)| arr)
    };

    let r = find("R")?;
    let g = find("G")?;
    let b = find("B")?;
    let (h, w) = r.dim();

    if g.dim() != (h, w) || b.dim() != (h, w) {
        let min_h = h.min(g.dim().0).min(b.dim().0);
        let min_w = w.min(g.dim().1).min(b.dim().1);
        let (r_n, g_n, b_n) = stretch_rgb_shared(
            &r.slice(ndarray::s![..min_h, ..min_w]).to_owned(),
            &g.slice(ndarray::s![..min_h, ..min_w]).to_owned(),
            &b.slice(ndarray::s![..min_h, ..min_w]).to_owned(),
        );

        let rs = r_n.as_slice().unwrap();
        let gs = g_n.as_slice().unwrap();
        let bs = b_n.as_slice().unwrap();

        let pixels: Vec<f32> = (0..min_h)
            .into_par_iter()
            .flat_map(|y| {
                let base = y * min_w;
                (0..min_w).flat_map(move |x| {
                    let i = base + x;
                    [rs[i], gs[i], bs[i]]
                }).collect::<Vec<f32>>()
            })
            .collect();

        return Some(Array3::from_shape_vec((min_h, min_w, 3), pixels).unwrap());
    }

    let (r_norm, g_norm, b_norm) = match find("L") {
        Some(lum) if lum.dim() == (h, w) => {
            let (r_n, g_n, b_n) = stretch_rgb_shared(r, g, b);
            let l_n = stretch_mono_robust(lum);
            apply_luminance_rgb(&r_n, &g_n, &b_n, &l_n)
        }
        _ => stretch_rgb_shared(r, g, b),
    };

    let rs = r_norm.as_slice().unwrap();
    let gs = g_norm.as_slice().unwrap();
    let bs = b_norm.as_slice().unwrap();

    let pixels: Vec<f32> = (0..h)
        .into_par_iter()
        .flat_map(|y| {
            let base = y * w;
            (0..w).flat_map(move |x| {
                let i = base + x;
                [rs[i], gs[i], bs[i]]
            }).collect::<Vec<f32>>()
        })
        .collect();

    Some(Array3::from_shape_vec((h, w, 3), pixels).unwrap())
}

fn apply_luminance_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    lum: &Array2<f32>,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let (h, w) = r.dim();
    let npix = h * w;

    let r_s = r.as_slice().unwrap();
    let g_s = g.as_slice().unwrap();
    let b_s = b.as_slice().unwrap();
    let l_s = lum.as_slice().unwrap();

    let mut out_r = vec![0.0f32; npix];
    let mut out_g = vec![0.0f32; npix];
    let mut out_b = vec![0.0f32; npix];

    out_r
        .par_iter_mut()
        .zip(out_g.par_iter_mut())
        .zip(out_b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((or, og), ob))| {
            let rgb_lum = 0.2126 * r_s[i] + 0.7152 * g_s[i] + 0.0722 * b_s[i];
            let scale = if rgb_lum > 1e-10 { l_s[i] / rgb_lum } else { 1.0 };
            *or = (r_s[i] * scale).clamp(0.0, 1.0);
            *og = (g_s[i] * scale).clamp(0.0, 1.0);
            *ob = (b_s[i] * scale).clamp(0.0, 1.0);
        });

    (
        Array2::from_shape_vec((h, w), out_r).unwrap(),
        Array2::from_shape_vec((h, w), out_g).unwrap(),
        Array2::from_shape_vec((h, w), out_b).unwrap(),
    )
}

fn robust_stretch_bounds(channels: &[&Array2<f32>]) -> Option<(f32, f32)> {
    let mut samples: Vec<f32> = Vec::new();
    for ch in channels {
        let stride = (ch.len() / 65536).max(1);
        samples.extend(ch.iter().step_by(stride).copied().filter(|v| v.is_finite()));
    }
    if samples.is_empty() {
        return None;
    }
    let hi_pct = percentile(&mut samples, 0.999);
    let max_val = samples.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let (bg, sigma) = sigma_clipped_stats(&mut samples, 3.0, 3);
    let lo = (bg - 2.0 * sigma) as f32;
    let hi = if hi_pct > lo { hi_pct } else { max_val };
    let range = hi - lo;
    if !range.is_finite() || range < 1e-10 {
        return None;
    }
    Some((lo, hi))
}

fn stretch_rgb_shared(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    match robust_stretch_bounds(&[r, g, b]) {
        Some((lo, hi)) => {
            arcsinh_stretch_rgb_with_stats(r, g, b, Some(lo), Some(hi), PREVIEW_STRETCH_FACTOR, 1.0)
        }
        None => (
            Array2::zeros(r.dim()),
            Array2::zeros(g.dim()),
            Array2::zeros(b.dim()),
        ),
    }
}

fn stretch_mono_robust(ch: &Array2<f32>) -> Array2<f32> {
    match robust_stretch_bounds(&[ch]) {
        Some((lo, hi)) => arcsinh_stretch_with_stats(ch, lo, hi, PREVIEW_STRETCH_FACTOR, 1.0),
        None => Array2::zeros(ch.dim()),
    }
}

fn normalize_frames(frames: &[Array2<f32>]) -> Vec<Array2<f32>> {
    frames.par_iter().map(|frame| {
        let stride = (frame.len() / 65536).max(1);
        let mut samples: Vec<f32> = frame
            .iter()
            .step_by(stride)
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        let (background, _) = sigma_clipped_stats(&mut samples, 3.0, 3);
        if background > 0.0 {
            let inv_bg = 1.0 / background as f32;
            frame.mapv(|v| v * inv_bg)
        } else {
            frame.clone()
        }
    }).collect()
}

fn reject_and_combine_stack(frames: &[Array2<f32>], config: &BatchStackConfig) -> (Array2<f32>, Vec<usize>) {
    let (h, w) = frames[0].dim();
    let n = frames.len();
    let mut result = Array2::<f32>::zeros((h, w));
    let mut rejection_counts = vec![0usize; n];
    let params = config.rejection_params();

    let frame_slices: Vec<&[f32]> = frames.iter()
        .map(|f| f.as_slice().expect("contiguous"))
        .collect();

    let rows: Vec<usize> = (0..h).collect();
    let row_data: Vec<(Vec<f32>, Vec<usize>)> = rows.par_iter().map(|&y| {
        let mut row = vec![0.0f32; w];
        let mut local_rejected = vec![0usize; n];
        let mut samples: Vec<Sample> = Vec::with_capacity(n);
        let mut scratch = KernelScratch::default();

        let base = y * w;

        for x in 0..w {
            samples.clear();
            let idx = base + x;
            for (i, slice) in frame_slices.iter().enumerate() {
                let v = slice[idx];
                if v.is_finite() {
                    samples.push(Sample::plain(v, i as u16));
                }
            }

            let out = reject_and_combine_with(&mut samples, None, &params, &mut scratch);
            for rejected in &samples[out.kept..] {
                local_rejected[rejected.frame as usize] += 1;
            }
            row[x] = out.value;
        }
        (row, local_rejected)
    }).collect();

    for (y, (row, local_rej)) in row_data.into_iter().enumerate() {
        for (x, val) in row.into_iter().enumerate() { result[[y, x]] = val; }
        for (i, count) in local_rej.into_iter().enumerate() { rejection_counts[i] += count; }
    }
    (result, rejection_counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::cosmetic::Defect;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;
    use crate::infra::fits::writer::write_fits_mono;

    fn frame_with_stars(sky: f32, n_bright: usize) -> Array2<f32> {
        let mut a = Array2::from_elem((40, 40), sky);
        for i in 0..n_bright {
            a[[i, i]] = 1000.0;
        }
        a
    }

    #[test]
    fn normalize_uses_robust_background_not_mean() {
        let a = frame_with_stars(10.0, 30);
        let b = frame_with_stars(10.0, 0);
        let out = normalize_frames(&[a, b]);
        let sky_a = out[0][[39, 0]];
        let sky_b = out[1][[39, 0]];
        assert!((sky_a - sky_b).abs() < 0.05, "sky levels diverged: {} vs {}", sky_a, sky_b);
        assert!((sky_a - 1.0).abs() < 0.1, "sky not normalized to ~1: {}", sky_a);
    }

    fn master_with_nebula(sky: f32, nebula: f32, star: f32) -> Array2<f32> {
        let mut a = Array2::from_elem((40, 40), sky);
        for y in 10..14 {
            for x in 10..14 {
                a[[y, x]] = nebula;
            }
        }
        a[[30, 30]] = star;
        a
    }

    #[test]
    fn rgb_preview_uses_shared_robust_stretch() {
        let masters = vec![
            ("R".to_string(), master_with_nebula(1.0, 2.0, 131.0)),
            ("G".to_string(), master_with_nebula(1.0, 2.0, 262.0)),
            ("B".to_string(), master_with_nebula(1.0, 2.0, 50.0)),
        ];
        let rgb = compose_rgb_from_masters(&masters).expect("rgb composed");
        let nr = rgb[[11, 11, 0]];
        let ng = rgb[[11, 11, 1]];
        assert!(nr > 0.2, "nebula R too dark: {}", nr);
        assert!(ng > 0.2, "nebula G too dark: {}", ng);
        assert!(
            (nr - ng).abs() < 0.05 * nr.max(ng),
            "channel balance lost: R={} G={}",
            nr,
            ng
        );
        assert!(rgb[[0, 0, 0]] < nr, "sky not below nebula");
        assert!(rgb[[30, 30, 0]] >= nr, "star not at or above nebula");
    }

    fn frames_1x1(values: &[f32]) -> Vec<Array2<f32>> {
        values.iter().map(|&v| Array2::from_elem((1, 1), v)).collect()
    }

    #[test]
    fn sigma_clip_rejects_lone_outlier_over_tied_background() {
        let frames = frames_1x1(&[1000.0, 1000.0, 1000.0, 1000.0, 60000.0]);
        let config = BatchStackConfig { normalize_before_stack: false, ..Default::default() };
        let (stacked, rejections) = reject_and_combine_stack(&frames, &config);
        assert!((stacked[[0, 0]] - 1000.0).abs() < 1e-3, "outlier leaked: {}", stacked[[0, 0]]);
        assert_eq!(rejections, vec![0, 0, 0, 0, 1]);
    }

    #[test]
    fn sigma_clip_keeps_quantized_noise_around_tied_background() {
        let frames = frames_1x1(&[1000.0, 1000.0, 1000.0, 999.0, 1001.0]);
        let config = BatchStackConfig { normalize_before_stack: false, ..Default::default() };
        let (stacked, rejections) = reject_and_combine_stack(&frames, &config);
        assert!((stacked[[0, 0]] - 1000.0).abs() < 1e-3);
        assert_eq!(rejections, vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn batch_stack_config_defaults_to_sigma_clip_mean() {
        let c = BatchStackConfig::default();
        assert_eq!(c.rejection, RejectionMethod::SigmaClip);
        assert_eq!(c.combine, CombineMethod::Mean);
        assert_eq!((c.sigma_low, c.sigma_high), (2.5, 3.0));
    }

    #[test]
    fn reject_and_combine_stack_honours_rejection_and_combine_choice() {
        let frames = frames_1x1(&[1000.0, 1000.0, 1000.0, 1000.0, 60000.0]);
        let config = BatchStackConfig {
            normalize_before_stack: false,
            rejection: RejectionMethod::WinsorizedSigmaClip,
            combine: CombineMethod::Median,
            ..Default::default()
        };
        let (stacked, rejections) = reject_and_combine_stack(&frames, &config);
        assert!((stacked[[0, 0]] - 1000.0).abs() < 1e-3, "outlier leaked: {}", stacked[[0, 0]]);
        assert_eq!(rejections, vec![0, 0, 0, 0, 1]);

        let frames = frames_1x1(&[1.0, 2.0, 3.0, 4.0, 100.0]);
        let config = BatchStackConfig {
            normalize_before_stack: false,
            rejection: RejectionMethod::None,
            combine: CombineMethod::Median,
            ..Default::default()
        };
        let (stacked, rejections) = reject_and_combine_stack(&frames, &config);
        assert_eq!(stacked[[0, 0]], 3.0);
        assert_eq!(rejections, vec![0; 5]);
    }

    fn pseudo_noise(y: usize, x: usize) -> f32 {
        let h = (y as u32).wrapping_mul(2654435761) ^ (x as u32).wrapping_mul(40503);
        (h % 11) as f32 * 0.3
    }

    fn dark_with_hot_pixel() -> Array2<f32> {
        let mut dark = Array2::from_shape_fn((16, 16), |(y, x)| 10.0 + pseudo_noise(y, x));
        dark[[5, 5]] = 1000.0;
        dark
    }

    fn light_with_hot_pixel() -> Array2<f32> {
        let mut light = Array2::from_shape_fn((16, 16), |(y, x)| 110.0 + pseudo_noise(y, x));
        light[[5, 5]] = 5000.0;
        light
    }

    fn dark_hot_config() -> CosmeticConfig {
        CosmeticConfig { use_master_dark: true, dark_hot_sigma: Some(5.0), ..Default::default() }
    }

    fn dark_only_masters() -> CalibrationMasters {
        CalibrationMasters { dark: Some(dark_with_hot_pixel()), flat: None, bias: None }
    }

    #[test]
    fn cosmetic_step_repairs_hot_pixel_flagged_by_the_master_dark() {
        let masters = dark_only_masters();
        let light = light_with_hot_pixel();
        let plan = CosmeticPlan::build(&dark_hot_config(), masters.dark.as_ref()).unwrap();
        let channel = plan.for_dims(light.dim()).unwrap();

        let untouched = calibrate_light(&light, &masters, 1.0);
        assert!((untouched[[5, 5]] - 4000.0).abs() < 1e-3);

        let (corrected, replaced) = calibrate_light_with_cosmetic(&light, &masters, 1.0, Some(&channel));
        assert_eq!(replaced, 1);
        assert!((corrected[[5, 5]] - 100.0).abs() < 1.0, "hot pixel survived: {}", corrected[[5, 5]]);
        for (pos, &v) in untouched.indexed_iter() {
            if pos != (5, 5) {
                assert_eq!(corrected[pos], v);
            }
        }

        let (plain, replaced) = calibrate_light_with_cosmetic(&light, &masters, 1.0, None);
        assert_eq!(replaced, 0);
        assert_eq!(plain, untouched);
    }

    #[test]
    fn cosmetic_step_runs_after_dark_subtraction_and_before_flat_division() {
        let mut flat = Array2::from_elem((16, 16), 2.0f32);
        flat[[5, 5]] = 4.0;
        let masters = CalibrationMasters { flat: Some(flat), ..dark_only_masters() };
        let light = light_with_hot_pixel();
        let plan = CosmeticPlan::build(&dark_hot_config(), masters.dark.as_ref()).unwrap();
        let channel = plan.for_dims(light.dim()).unwrap();

        let (corrected, replaced) = calibrate_light_with_cosmetic(&light, &masters, 1.0, Some(&channel));
        assert_eq!(replaced, 1);
        assert!((corrected[[5, 5]] - 25.0).abs() < 0.5, "expected neighbour median / flat: {}", corrected[[5, 5]]);
        assert!((corrected[[0, 0]] - 50.0).abs() < 0.5);
    }

    #[test]
    fn run_batch_pipeline_counts_cosmetic_replacements_and_is_off_by_default() {
        let masters = dark_only_masters();
        let channel = ChannelInput {
            lights: (0..3).map(|_| light_with_hot_pixel()).collect(),
            label: "L".into(),
            dark_scales: vec![1.0; 3],
        };
        let stack = BatchStackConfig {
            normalize_before_stack: false,
            rejection: RejectionMethod::None,
            ..Default::default()
        };

        let with_cosmetic = BatchPipelineConfig {
            stack: stack.clone(),
            align: false,
            cosmetic: Some(dark_hot_config()),
            dark_optimize: false,
        };
        let res = run_batch_pipeline(vec![channel.clone()], &masters, &with_cosmetic).unwrap();
        assert_eq!(res.stats.channels[0].cosmetic_replaced, Some(3));
        assert!((res.master_channels[0].1[[5, 5]] - 100.0).abs() < 1.0);

        let default_config = BatchPipelineConfig { stack, align: false, ..Default::default() };
        assert!(default_config.cosmetic.is_none());
        let res = run_batch_pipeline(vec![channel], &masters, &default_config).unwrap();
        assert_eq!(res.stats.channels[0].cosmetic_replaced, None);
        assert!((res.master_channels[0].1[[5, 5]] - 4000.0).abs() < 1e-2);
    }

    #[test]
    fn cosmetic_plan_merges_defect_list_and_auto_map_and_skips_a_missing_dark() {
        let cfg = CosmeticConfig {
            use_master_dark: true,
            auto_hot_sigma: Some(5.0),
            defects: vec![Defect::Point { x: 2, y: 3 }],
            ..Default::default()
        };
        let plan = CosmeticPlan::build(&cfg, None).unwrap();
        let mut light = Array2::from_shape_fn((16, 16), |(y, x)| 100.0 + pseudo_noise(y, x));
        light[[9, 9]] = 5000.0;
        let channel = plan.for_dims(light.dim()).unwrap();
        let masters = CalibrationMasters { dark: None, flat: None, bias: None };

        let (corrected, replaced) = calibrate_light_with_cosmetic(&light, &masters, 1.0, Some(&channel));
        assert_eq!(replaced, 2);
        assert!(corrected[[9, 9]] < 104.0, "auto hot pixel survived: {}", corrected[[9, 9]]);
        assert!((corrected[[3, 2]] - light[[3, 2]]).abs() < 4.0);
        assert_ne!(corrected[[3, 2]], light[[3, 2]]);

        let outside = CosmeticConfig {
            use_master_dark: false,
            defects: vec![Defect::Point { x: 40, y: 0 }],
            ..Default::default()
        };
        let plan = CosmeticPlan::build(&outside, None).unwrap();
        assert!(plan.for_dims((16, 16)).is_err());

        let nothing = CosmeticConfig { use_master_dark: false, ..Default::default() };
        let plan = CosmeticPlan::build(&nothing, None).unwrap();
        let channel = plan.for_dims(light.dim()).unwrap();
        let (same, replaced) = calibrate_light_with_cosmetic(&light, &masters, 1.0, Some(&channel));
        assert_eq!(replaced, 0);
        assert_eq!(same, light);
    }

    #[test]
    fn cosmetic_plan_rejects_invalid_sigma_and_amount() {
        let zero_sigma = CosmeticConfig { dark_hot_sigma: Some(0.0), ..Default::default() };
        assert!(CosmeticPlan::build(&zero_sigma, None).is_err());
        let nan_sigma = CosmeticConfig { auto_cold_sigma: Some(f32::NAN), ..Default::default() };
        assert!(validate_cosmetic_config(&nan_sigma).is_err());
        let bad_amount = CosmeticConfig { amount: 1.5, ..Default::default() };
        assert!(validate_cosmetic_config(&bad_amount).is_err());
        assert!(validate_cosmetic_config(&CosmeticConfig::default()).is_ok());
    }

    #[test]
    fn carries_dq_plane_detects_dq_extensions_only() {
        let dir = tempfile::tempdir().unwrap();
        let mef = dir.path().join("mef.fits");
        sci_err_dq_mef(&mef, 4, 4, vec![0i32; 16]);
        let mef_path = mef.to_str().unwrap().to_string();
        let plain = dir.path().join("plain.fits");
        let plain_path = plain.to_str().unwrap().to_string();
        write_fits_mono(&plain_path, &Array2::from_elem((4, 4), 1.0f32), None).unwrap();

        assert!(carries_dq_plane(&mef_path));
        assert!(carries_dq_plane(&format!("{}#hdu=1", mef_path)));
        assert!(!carries_dq_plane(&plain_path));
        assert!(!carries_dq_plane("C:/definitely/missing/light.fits"));
    }

    fn seeded_uniform(rows: usize, cols: usize, amplitude: f32, seed: u32) -> Array2<f32> {
        let mut state = seed;
        Array2::from_shape_fn((rows, cols), |_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 8) as f32 / (1u32 << 24) as f32 * amplitude
        })
    }

    fn seeded_gaussian(rows: usize, cols: usize, sigma: f32, seed: u64) -> Array2<f32> {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        let mut rng = StdRng::seed_from_u64(seed);
        Array2::from_shape_fn((rows, cols), |_| {
            let u1: f64 = rng.gen::<f64>().max(1e-30);
            let u2: f64 = rng.gen::<f64>();
            ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32 * sigma
        })
    }

    fn robust_sigma(frame: &Array2<f32>) -> f32 {
        let mut values: Vec<f32> = frame.iter().copied().filter(|v| v.is_finite()).collect();
        let median = median_f32_mut(&mut values);
        exact_mad_mut(&mut values, median) * MAD_TO_SIGMA as f32
    }

    fn scaled_dark_scene(noise_seed: u64) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
        let bias = Array2::from_shape_fn((128, 128), |(y, x)| 300.0 + ((x + y) % 3) as f32);
        let dark_pattern = seeded_uniform(128, 128, 300.0, 99);
        let noise = seeded_gaussian(128, 128, 5.0, noise_seed);
        let mut light = &bias + &dark_pattern.mapv(|d| 0.7 * d) + 500.0 + &noise;
        light[[10, 10]] = 60000.0;
        light[[50, 60]] = 40000.0;
        light[[51, 60]] = 45000.0;
        (light, bias, dark_pattern)
    }

    #[test]
    fn dark_optimization_recovers_the_scale_and_lowers_the_calibrated_noise() {
        let (light, bias, dark) = scaled_dark_scene(7);

        let k = optimize_dark_scale(&light, Some(&bias), &dark);
        assert!((k - 0.7).abs() <= 0.05, "recovered dark scale {k}");

        let masters = CalibrationMasters { dark: Some(dark), flat: None, bias: Some(bias) };
        let optimized = robust_sigma(&calibrate_light(&light, &masters, k));
        let unit = robust_sigma(&calibrate_light(&light, &masters, 1.0));
        assert!(optimized < unit, "optimized sigma {optimized} not below unit-scale sigma {unit}");
        assert!(optimized < 7.0, "optimized sigma {optimized} far from the injected 5.0");
    }

    #[test]
    fn golden_section_finds_a_quadratic_minimum_within_tolerance_and_budget() {
        let mut evaluations = 0usize;
        let k = golden_section_minimum(0.0, 3.0, 1e-3, 40, |x| {
            evaluations += 1;
            (x - 1.234).powi(2)
        });
        assert!((k - 1.234).abs() < 1e-3, "minimum {k}");
        assert!(evaluations <= 40, "{evaluations} evaluations");
    }

    #[test]
    fn dark_optimization_sample_drops_stars_and_falls_back_without_a_dark() {
        let light: Vec<f32> = (0..1000).map(|i| if i == 500 { 1e6 } else { 100.0 + (i % 7) as f32 }).collect();
        let dark: Vec<f32> = (0..1000).map(|i| (i % 5) as f32).collect();
        let sample = dark_optimize_sample(&light, None, &dark);
        assert_eq!(sample.dark.len(), 999);
        assert!(sample.light_minus_bias.iter().all(|&v| v < 1e5));

        let mismatched = Array2::from_elem((4, 4), 1.0f32);
        assert_eq!(optimize_dark_scale(&Array2::from_elem((8, 8), 5.0f32), None, &mismatched), 1.0);
    }

    #[test]
    fn run_batch_pipeline_reports_dark_scale_stats_and_only_optimizes_when_asked() {
        let (light_a, bias, dark) = scaled_dark_scene(11);
        let (light_b, _, _) = scaled_dark_scene(12);
        let masters = CalibrationMasters { dark: Some(dark), flat: None, bias: Some(bias) };
        let channel = ChannelInput {
            lights: vec![light_a, light_b],
            label: "L".into(),
            dark_scales: vec![1.0; 2],
        };
        let stack = BatchStackConfig { normalize_before_stack: false, rejection: RejectionMethod::None, ..Default::default() };

        let plain = BatchPipelineConfig { stack: stack.clone(), align: false, ..Default::default() };
        assert!(!plain.dark_optimize);
        let res = run_batch_pipeline(vec![channel.clone()], &masters, &plain).unwrap();
        let stats = &res.stats.channels[0];
        assert_eq!((stats.dark_scale_min, stats.dark_scale_max, stats.dark_scale_mean), (Some(1.0), Some(1.0), Some(1.0)));
        let unit_sigma = robust_sigma(&res.master_channels[0].1);

        let optimized = BatchPipelineConfig { stack: stack.clone(), align: false, dark_optimize: true, ..Default::default() };
        let res = run_batch_pipeline(vec![channel.clone()], &masters, &optimized).unwrap();
        let stats = &res.stats.channels[0];
        let mean = stats.dark_scale_mean.expect("dark scale reported");
        assert!((mean - 0.7).abs() <= 0.05, "mean dark scale {mean}");
        assert!(stats.dark_scale_min.unwrap() <= mean && mean <= stats.dark_scale_max.unwrap());
        let optimized_sigma = robust_sigma(&res.master_channels[0].1);
        assert!(optimized_sigma < unit_sigma, "{optimized_sigma} vs {unit_sigma}");

        let no_dark = CalibrationMasters { dark: None, flat: None, bias: masters.bias.clone() };
        let res = run_batch_pipeline(vec![channel], &no_dark, &optimized).unwrap();
        let stats = &res.stats.channels[0];
        assert_eq!((stats.dark_scale_min, stats.dark_scale_max, stats.dark_scale_mean), (None, None, None));
    }
}
