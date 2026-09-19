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

const PREVIEW_STRETCH_FACTOR: f32 = 20.0;
pub const DQ_COSMETIC_WARNING: &str = "cosmetic correction applied to data with DQ planes";

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
}

impl Default for BatchPipelineConfig {
    fn default() -> Self {
        Self {
            stack: BatchStackConfig::default(),
            align: true,
            cosmetic: None,
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
        let (calibrated, replaced_counts): (Vec<Array2<f32>>, Vec<usize>) = channel
            .lights
            .par_iter()
            .enumerate()
            .map(|(i, l)| {
                let scale = channel.dark_scales.get(i).copied().unwrap_or(1.0);
                calibrate_light_with_cosmetic(l, masters, scale, channel_cosmetic.as_ref())
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

        let with_cosmetic = BatchPipelineConfig { stack: stack.clone(), align: false, cosmetic: Some(dark_hot_config()) };
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
}
