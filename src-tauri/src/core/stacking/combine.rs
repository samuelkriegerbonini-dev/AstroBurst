use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

pub use crate::types::stacking::{
    CombineMethod, NormalizationMethod, RejectionMethod, RejectionNormalization, RejectionParams,
    StackConfig, StackResult,
};
use crate::math::median::{exact_mad_mut, f32_cmp, median_f32_mut};
use crate::types::constants::MAD_TO_SIGMA;

use crate::core::stacking::align;

const MEAN_ABS_DEV_TO_SIGMA: f64 = 1.2533141;
const WINSOR_HALF_WIDTH_SIGMAS: f32 = 1.5;
const WINSOR_SIGMA_CORRECTION: f64 = 1.134;
const WINSOR_MAX_STEPS: usize = 10;
const WINSOR_RELATIVE_TOLERANCE: f32 = 1e-4;
const LINEAR_FIT_MIN_SAMPLES: usize = 5;
const PERCENTILE_MIN_SAMPLES: usize = 3;
const FRAME_STATS_MAX_SAMPLES: usize = 131_072;

fn scale_from_deviations(mad: f32, abs_devs: &[f32]) -> f32 {
    let robust = mad as f64 * MAD_TO_SIGMA;
    if robust > 0.0 {
        return robust as f32;
    }
    let mean_abs = abs_devs.iter().map(|d| *d as f64).sum::<f64>() / abs_devs.len().max(1) as f64;
    (mean_abs * MEAN_ABS_DEV_TO_SIGMA) as f32
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub value: f32,
    pub cmp: f32,
    pub frame: u16,
}

impl Sample {
    pub fn plain(value: f32, frame: u16) -> Self {
        Self { value, cmp: value, frame }
    }

    pub fn normalized(value: f32, cmp: f32, frame: u16) -> Self {
        Self { value, cmp, frame }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelOutcome {
    pub value: f32,
    pub rejected_low: u16,
    pub rejected_high: u16,
    pub kept: usize,
}

#[derive(Default)]
pub struct KernelScratch {
    deviations: Vec<f32>,
    work: Vec<f32>,
}

struct Clipped {
    kept: usize,
    low: usize,
    high: usize,
    last_center: f32,
}

impl Clipped {
    fn untouched(n: usize, last_center: f32) -> Self {
        Self { kept: n, low: 0, high: 0, last_center }
    }
}

pub fn reject_and_combine(
    samples: &mut [Sample],
    weights: Option<&[f64]>,
    cfg: &RejectionParams,
) -> PixelOutcome {
    let mut scratch = KernelScratch::default();
    reject_and_combine_with(samples, weights, cfg, &mut scratch)
}

pub fn reject_and_combine_with(
    samples: &mut [Sample],
    weights: Option<&[f64]>,
    cfg: &RejectionParams,
    scratch: &mut KernelScratch,
) -> PixelOutcome {
    let n = samples.len();
    if n == 0 {
        return PixelOutcome { value: f32::NAN, rejected_low: 0, rejected_high: 0, kept: 0 };
    }
    if n == 1 {
        return PixelOutcome { value: samples[0].value, rejected_low: 0, rejected_high: 0, kept: 1 };
    }

    let clipped = match cfg.rejection {
        RejectionMethod::None => Clipped::untouched(n, f32::NAN),
        RejectionMethod::SigmaClip => sigma_clip(samples, cfg, scratch),
        RejectionMethod::WinsorizedSigmaClip => winsorized_sigma_clip(samples, cfg, scratch),
        RejectionMethod::LinearFitClip => {
            if n >= LINEAR_FIT_MIN_SAMPLES {
                linear_fit_clip(samples, cfg, scratch)
            } else {
                sigma_clip(samples, cfg, scratch)
            }
        }
        RejectionMethod::PercentileClip => percentile_clip(samples, cfg),
        RejectionMethod::MinMax => min_max_clip(samples, cfg),
    };

    let value = if clipped.kept == 0 {
        if clipped.last_center.is_finite() { clipped.last_center } else { 0.0 }
    } else {
        combine_kept(&samples[..clipped.kept], weights, cfg.combine, scratch)
    };

    PixelOutcome {
        value,
        rejected_low: clipped.low as u16,
        rejected_high: clipped.high as u16,
        kept: clipped.kept,
    }
}

fn partition_by_deviation(samples: &mut [Sample], center: f32, lo: f32, hi: f32) -> (usize, usize, usize) {
    let mut write = 0;
    let mut low = 0;
    let mut high = 0;
    for read in 0..samples.len() {
        let dev = samples[read].cmp - center;
        if dev >= lo && dev <= hi {
            samples.swap(write, read);
            write += 1;
        } else if dev < lo {
            low += 1;
        } else {
            high += 1;
        }
    }
    (write, low, high)
}

fn median_by_cmp(samples: &mut [Sample]) -> f32 {
    let mid = samples.len() / 2;
    samples.select_nth_unstable_by(mid, |a, b| f32_cmp(&a.cmp, &b.cmp));
    samples[mid].cmp
}

fn robust_sigma(samples: &[Sample], center: f32, scratch: &mut KernelScratch) -> f32 {
    scratch.deviations.clear();
    scratch.deviations.extend(samples.iter().map(|s| (s.cmp - center).abs()));
    let dmid = scratch.deviations.len() / 2;
    scratch.deviations.select_nth_unstable_by(dmid, |a, b| f32_cmp(a, b));
    let mad = scratch.deviations[dmid];
    scale_from_deviations(mad, &scratch.deviations)
}

fn mean_and_sample_sigma(samples: &[Sample]) -> (f32, f32) {
    let n = samples.len() as f64;
    let mean = samples.iter().map(|s| s.cmp as f64).sum::<f64>() / n;
    let variance = samples
        .iter()
        .map(|s| {
            let d = s.cmp as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0).max(1.0);
    (mean as f32, variance.sqrt() as f32)
}

fn sigma_clip(samples: &mut [Sample], cfg: &RejectionParams, scratch: &mut KernelScratch) -> Clipped {
    let mut len = samples.len();
    let mut low = 0;
    let mut high = 0;
    let mut last_center = f32::NAN;

    for iteration in 0..cfg.max_iterations {
        if len < 2 {
            break;
        }
        let (center, sigma) = if iteration == 0 {
            let med = median_by_cmp(&mut samples[..len]);
            (med, robust_sigma(&samples[..len], med, scratch))
        } else {
            mean_and_sample_sigma(&samples[..len])
        };
        last_center = center;
        if sigma <= 0.0 {
            break;
        }
        let (write, l, h) = partition_by_deviation(
            &mut samples[..len],
            center,
            -cfg.sigma_low * sigma,
            cfg.sigma_high * sigma,
        );
        low += l;
        high += h;
        let removed = len - write;
        len = write;
        if removed == 0 {
            break;
        }
    }

    Clipped { kept: len, low, high, last_center }
}

fn winsorized_sigma(
    samples: &[Sample],
    center: f32,
    initial_sigma: f32,
    cutoff: f32,
    scratch: &mut KernelScratch,
) -> f32 {
    let n = samples.len() as f64;
    let bound = if cutoff.is_finite() && cutoff > 0.0 { cutoff * initial_sigma } else { f32::INFINITY };
    scratch.work.clear();
    scratch.work.extend(samples.iter().map(|s| s.cmp.clamp(center - bound, center + bound)));

    let mut sigma = initial_sigma;
    for _ in 0..WINSOR_MAX_STEPS {
        let half = WINSOR_HALF_WIDTH_SIGMAS * sigma;
        let (lo, hi) = (center - half, center + half);
        let mean = scratch.work.iter().map(|v| v.clamp(lo, hi) as f64).sum::<f64>() / n;
        let variance = scratch
            .work
            .iter()
            .map(|v| {
                let d = v.clamp(lo, hi) as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / (n - 1.0).max(1.0);
        let next = (variance.sqrt() * WINSOR_SIGMA_CORRECTION) as f32;
        if next <= 0.0 {
            return sigma;
        }
        let converged = (next - sigma).abs() <= WINSOR_RELATIVE_TOLERANCE * sigma;
        sigma = next;
        if converged {
            break;
        }
    }
    sigma
}

fn winsorized_sigma_clip(samples: &mut [Sample], cfg: &RejectionParams, scratch: &mut KernelScratch) -> Clipped {
    let mut len = samples.len();
    let mut low = 0;
    let mut high = 0;
    let mut last_center = f32::NAN;

    for _ in 0..cfg.max_iterations {
        if len < 3 {
            break;
        }
        let med = median_by_cmp(&mut samples[..len]);
        last_center = med;
        let robust = robust_sigma(&samples[..len], med, scratch);
        if robust <= 0.0 {
            break;
        }
        let sigma = winsorized_sigma(&samples[..len], med, robust, cfg.winsor_cutoff, scratch);
        if sigma <= 0.0 {
            break;
        }
        let (write, l, h) = partition_by_deviation(
            &mut samples[..len],
            med,
            -cfg.sigma_low * sigma,
            cfg.sigma_high * sigma,
        );
        low += l;
        high += h;
        let removed = len - write;
        len = write;
        if removed == 0 {
            break;
        }
    }

    Clipped { kept: len, low, high, last_center }
}

fn fit_line_inner_ranks(sorted: &[Sample]) -> (f32, f32) {
    let n = sorted.len();
    let trim = n / 4;
    let (start, end) = (trim, n - trim);
    let count = (end - start) as f64;
    let mean_x = (start + end - 1) as f64 / 2.0;
    let mean_y = sorted[start..end].iter().map(|s| s.cmp as f64).sum::<f64>() / count;
    let mut sxy = 0.0f64;
    let mut sxx = 0.0f64;
    for (i, s) in sorted[start..end].iter().enumerate() {
        let dx = (start + i) as f64 - mean_x;
        sxy += dx * (s.cmp as f64 - mean_y);
        sxx += dx * dx;
    }
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    let intercept = mean_y - slope * mean_x;
    (intercept as f32, slope as f32)
}

fn linear_fit_clip(samples: &mut [Sample], cfg: &RejectionParams, scratch: &mut KernelScratch) -> Clipped {
    let mut len = samples.len();
    let mut low = 0;
    let mut high = 0;
    let mut last_center = f32::NAN;

    for _ in 0..cfg.max_iterations {
        if len < LINEAR_FIT_MIN_SAMPLES {
            break;
        }
        samples[..len].sort_unstable_by(|a, b| f32_cmp(&a.cmp, &b.cmp));
        let (intercept, slope) = fit_line_inner_ranks(&samples[..len]);
        last_center = intercept + slope * (len as f32 / 2.0);

        scratch.deviations.clear();
        scratch.deviations.extend(
            samples[..len]
                .iter()
                .enumerate()
                .map(|(i, s)| (s.cmp - (intercept + slope * i as f32)).abs()),
        );
        let dmid = len / 2;
        scratch.deviations.select_nth_unstable_by(dmid, |a, b| f32_cmp(a, b));
        let dispersion = scale_from_deviations(scratch.deviations[dmid], &scratch.deviations);
        if dispersion <= 0.0 {
            break;
        }

        let lo = -cfg.sigma_low * dispersion;
        let hi = cfg.sigma_high * dispersion;
        let mut write = 0;
        for read in 0..len {
            let residual = samples[read].cmp - (intercept + slope * read as f32);
            if residual >= lo && residual <= hi {
                samples.swap(write, read);
                write += 1;
            } else if residual < lo {
                low += 1;
            } else {
                high += 1;
            }
        }
        let removed = len - write;
        len = write;
        if removed == 0 {
            break;
        }
    }

    Clipped { kept: len, low, high, last_center }
}

fn percentile_clip(samples: &mut [Sample], cfg: &RejectionParams) -> Clipped {
    let n = samples.len();
    if n < PERCENTILE_MIN_SAMPLES {
        return Clipped::untouched(n, f32::NAN);
    }
    let med = median_by_cmp(samples);
    let magnitude = med.abs();
    if !(magnitude > 0.0) || !magnitude.is_finite() {
        return Clipped::untouched(n, med);
    }
    let lo = -cfg.percentile_low.max(0.0) * magnitude;
    let hi = cfg.percentile_high.max(0.0) * magnitude;
    let (write, low, high) = partition_by_deviation(samples, med, lo, hi);
    Clipped { kept: write, low, high, last_center: med }
}

fn min_max_clip(samples: &mut [Sample], cfg: &RejectionParams) -> Clipped {
    let n = samples.len();
    let (low, high) = (cfg.minmax_low, cfg.minmax_high);
    if n <= low + high {
        return Clipped::untouched(n, f32::NAN);
    }
    samples.sort_unstable_by(|a, b| f32_cmp(&a.cmp, &b.cmp));
    samples.rotate_left(low);
    Clipped { kept: n - low - high, low, high, last_center: f32::NAN }
}

fn plain_mean(kept: &[Sample]) -> f32 {
    (kept.iter().map(|s| s.value as f64).sum::<f64>() / kept.len() as f64) as f32
}

fn weighted_mean(kept: &[Sample], weights: &[f64]) -> f32 {
    let mut vsum = 0.0f64;
    let mut wsum = 0.0f64;
    for s in kept {
        let w = weights.get(s.frame as usize).copied().unwrap_or(1.0);
        vsum += s.value as f64 * w;
        wsum += w;
    }
    if wsum > 1e-12 {
        (vsum / wsum) as f32
    } else {
        plain_mean(kept)
    }
}

fn combine_kept(
    kept: &[Sample],
    weights: Option<&[f64]>,
    method: CombineMethod,
    scratch: &mut KernelScratch,
) -> f32 {
    match method {
        CombineMethod::Mean => match weights {
            Some(w) => weighted_mean(kept, w),
            None => plain_mean(kept),
        },
        CombineMethod::Median => {
            scratch.work.clear();
            scratch.work.extend(kept.iter().map(|s| s.value));
            median_f32_mut(&mut scratch.work)
        }
        CombineMethod::Min => kept.iter().map(|s| s.value).fold(f32::INFINITY, f32::min),
        CombineMethod::Max => kept.iter().map(|s| s.value).fold(f32::NEG_INFINITY, f32::max),
    }
}

fn sigma_clip_params(sigma_low: f32, sigma_high: f32, max_iter: usize) -> RejectionParams {
    RejectionParams {
        rejection: RejectionMethod::SigmaClip,
        combine: CombineMethod::Mean,
        sigma_low,
        sigma_high,
        max_iterations: max_iter,
        ..RejectionParams::default()
    }
}

pub fn sigma_clip_combine(
    values: &mut Vec<f32>,
    sigma_low: f32,
    sigma_high: f32,
    max_iter: usize,
) -> (f32, u32) {
    let mut samples: Vec<Sample> = values
        .iter()
        .enumerate()
        .map(|(i, &v)| Sample::plain(v, i as u16))
        .collect();
    let out = reject_and_combine(&mut samples, None, &sigma_clip_params(sigma_low, sigma_high, max_iter));
    (out.value, out.rejected_low as u32 + out.rejected_high as u32)
}

pub fn sigma_clip_combine_weighted(
    vals: &mut Vec<(f32, f32)>,
    sigma_low: f32,
    sigma_high: f32,
    max_iter: usize,
) -> (f32, u32) {
    let weights: Vec<f64> = vals.iter().map(|p| p.1 as f64).collect();
    let mut samples: Vec<Sample> = vals
        .iter()
        .enumerate()
        .map(|(i, p)| Sample::plain(p.0, i as u16))
        .collect();
    let out = reject_and_combine(
        &mut samples,
        Some(&weights),
        &sigma_clip_params(sigma_low, sigma_high, max_iter),
    );
    (out.value, out.rejected_low as u32 + out.rejected_high as u32)
}

#[derive(Debug, Clone, Copy)]
struct FrameStats {
    location: f32,
    scale: f32,
}

fn overlap_frame_stats(frames: &[&[f32]], npix: usize) -> Option<Vec<FrameStats>> {
    let stride = (npix / FRAME_STATS_MAX_SAMPLES).max(1);
    let mut per_frame: Vec<Vec<f32>> = vec![Vec::with_capacity(npix / stride + 1); frames.len()];
    for idx in (0..npix).step_by(stride) {
        if frames.iter().all(|f| f[idx].is_finite()) {
            for (f, buf) in frames.iter().zip(per_frame.iter_mut()) {
                buf.push(f[idx]);
            }
        }
    }
    if per_frame.first().map_or(true, |b| b.len() < 2) {
        return None;
    }
    Some(
        per_frame
            .into_par_iter()
            .map(|mut buf| {
                let location = median_f32_mut(&mut buf);
                let mad = exact_mad_mut(&mut buf, location);
                FrameStats { location, scale: (mad as f64 * MAD_TO_SIGMA) as f32 }
            })
            .collect(),
    )
}

fn normalization_terms(method: NormalizationMethod, reference: FrameStats, frame: FrameStats) -> (f64, f64) {
    let (m0, s0) = (reference.location as f64, reference.scale as f64);
    let (mf, sf) = (frame.location as f64, frame.scale as f64);
    let scale_ratio = if s0 > 0.0 && sf > 0.0 && s0.is_finite() && sf.is_finite() { s0 / sf } else { 1.0 };
    let location_ratio = if mf.abs() > 1e-12 && m0.is_finite() && mf.is_finite() { m0 / mf } else { 1.0 };
    let location_shift = if m0.is_finite() && mf.is_finite() { m0 - mf } else { 0.0 };
    match method {
        NormalizationMethod::None => (0.0, 1.0),
        NormalizationMethod::Additive => (location_shift, 1.0),
        NormalizationMethod::Multiplicative => (0.0, location_ratio),
        NormalizationMethod::AdditiveScaling => (m0 - scale_ratio * mf, scale_ratio),
        NormalizationMethod::MultiplicativeScaling => (0.0, scale_ratio),
    }
}

fn rejection_rescaling(
    stats: &[FrameStats],
    applied: &[(f64, f64)],
) -> Vec<Option<(f32, f32)>> {
    let post: Vec<(f64, f64)> = stats
        .iter()
        .zip(applied)
        .map(|(st, &(offset, scale))| (st.location as f64 * scale + offset, st.scale as f64 * scale))
        .collect();
    let (m0, s0) = post[0];
    post.iter()
        .map(|&(m, s)| {
            let relative = if s0 > 0.0 && s > 0.0 { s / s0 } else { 1.0 };
            if m == m0 && relative == 1.0 {
                None
            } else {
                Some((m as f32, relative as f32))
            }
        })
        .collect()
}

struct RowCombiner<'a> {
    slices: &'a [&'a [f32]],
    cols: usize,
    rescaling: Option<&'a [Option<(f32, f32)>]>,
    reference_location: f32,
    weights: Option<&'a [f64]>,
    params: &'a RejectionParams,
}

impl RowCombiner<'_> {
    fn combine_row(
        &self,
        y: usize,
        row_buf: &mut [f32],
        mut low_map: Option<&mut [u16]>,
        mut high_map: Option<&mut [u16]>,
    ) -> u64 {
        let base = y * self.cols;
        let mut rejected: u64 = 0;
        let mut samples: Vec<Sample> = Vec::with_capacity(self.slices.len());
        let mut scratch = KernelScratch::default();
        for x in 0..self.cols {
            samples.clear();
            let idx = base + x;
            for (i, s) in self.slices.iter().enumerate() {
                let v = s[idx];
                if !v.is_finite() {
                    continue;
                }
                let cmp = match self.rescaling.and_then(|r| r[i]) {
                    Some((location, relative_scale)) => (v - location) / relative_scale + self.reference_location,
                    None => v,
                };
                samples.push(Sample::normalized(v, cmp, i as u16));
            }
            let out = reject_and_combine_with(&mut samples, self.weights, self.params, &mut scratch);
            row_buf[x] = out.value;
            rejected += out.rejected_low as u64 + out.rejected_high as u64;
            if let Some(low) = low_map.as_deref_mut() {
                low[x] = out.rejected_low;
            }
            if let Some(high) = high_map.as_deref_mut() {
                high[x] = out.rejected_high;
            }
        }
        rejected
    }
}

pub fn stack_images(
    images: &[Array2<f32>],
    config: &StackConfig,
) -> Result<StackResult> {
    if images.is_empty() {
        bail!("No images to stack");
    }

    let n = images.len();

    let min_rows = images.iter().map(|img| img.dim().0).min().unwrap();
    let min_cols = images.iter().map(|img| img.dim().1).min().unwrap();

    fn crop_to(img: &Array2<f32>, rows: usize, cols: usize) -> Cow<'_, Array2<f32>> {
        let (r, c) = img.dim();
        if r == rows && c == cols {
            Cow::Borrowed(img)
        } else {
            Cow::Owned(img.slice(ndarray::s![..rows, ..cols]).to_owned())
        }
    }

    let ref_cropped = crop_to(&images[0], min_rows, min_cols);

    let mut aligned: Vec<Cow<Array2<f32>>> = Vec::with_capacity(n);
    let mut offsets: Vec<(i32, i32)> = Vec::with_capacity(n);

    aligned.push(Cow::Borrowed(ref_cropped.as_ref()));
    offsets.push((0, 0));

    for i in 1..n {
        let cropped = crop_to(&images[i], min_rows, min_cols);

        if config.align {
            let result = align::align_pair_with_label(
                ref_cropped.as_ref(),
                cropped.as_ref(),
                config.align_method,
                min_rows,
                min_cols,
                &format!("frame_{}", i),
            )?;
            let dy = result.offset.0.round() as i32;
            let dx = result.offset.1.round() as i32;
            offsets.push((dy, dx));
            aligned.push(Cow::Owned(result.aligned));
        } else {
            offsets.push((0, 0));
            aligned.push(cropped);
        }
    }

    let rows = min_rows;
    let cols = min_cols;
    let npix = rows * cols;

    let needs_stats = config.normalization != NormalizationMethod::None
        || config.rejection_normalization == RejectionNormalization::ScaleOffset;
    let stats = if needs_stats {
        let slices: Vec<&[f32]> = aligned
            .iter()
            .map(|img| img.as_slice().expect("contiguous"))
            .collect();
        overlap_frame_stats(&slices, npix)
    } else {
        None
    };

    let mut normalization_applied = vec![(0.0f64, 1.0f64); n];
    if let Some(stats) = &stats {
        if config.normalization != NormalizationMethod::None {
            for f in 1..n {
                let (offset, scale) = normalization_terms(config.normalization, stats[0], stats[f]);
                if offset != 0.0 || scale != 1.0 {
                    let (k, z) = (scale as f32, offset as f32);
                    aligned[f].to_mut().par_mapv_inplace(|v| v * k + z);
                    normalization_applied[f] = (offset, scale);
                }
            }
        }
    }

    let rescaling: Option<Vec<Option<(f32, f32)>>> = match (&stats, config.rejection_normalization) {
        (Some(stats), RejectionNormalization::ScaleOffset) => {
            Some(rejection_rescaling(stats, &normalization_applied))
        }
        _ => None,
    };
    let reference_location = stats.as_ref().map(|s| s[0].location).unwrap_or(0.0);

    let aligned_slices: Vec<&[f32]> = aligned
        .iter()
        .map(|img| img.as_slice().expect("contiguous"))
        .collect();

    let weights: Option<Vec<f64>> = config.weights.as_ref().filter(|w| w.len() == n).cloned();
    let params = config.rejection_params();

    let combiner = RowCombiner {
        slices: &aligned_slices,
        cols,
        rescaling: rescaling.as_deref(),
        reference_location,
        weights: weights.as_deref(),
        params: &params,
    };

    let mut result_data = vec![0.0f32; npix];
    let total_rejected = AtomicU64::new(0);

    let (mut low_map, mut high_map) = if config.rejection_maps {
        (vec![0u16; npix], vec![0u16; npix])
    } else {
        (Vec::new(), Vec::new())
    };

    if config.rejection_maps {
        result_data
            .par_chunks_mut(cols)
            .zip(low_map.par_chunks_mut(cols))
            .zip(high_map.par_chunks_mut(cols))
            .enumerate()
            .for_each(|(y, ((row_buf, low), high))| {
                let rejected = combiner.combine_row(y, row_buf, Some(low), Some(high));
                total_rejected.fetch_add(rejected, Ordering::Relaxed);
            });
    } else {
        result_data
            .par_chunks_mut(cols)
            .enumerate()
            .for_each(|(y, row_buf)| {
                let rejected = combiner.combine_row(y, row_buf, None, None);
                total_rejected.fetch_add(rejected, Ordering::Relaxed);
            });
    }

    let rejected_pixels = total_rejected.load(Ordering::Relaxed);

    let (rejection_low, rejection_high) = if config.rejection_maps {
        (
            Some(Array2::from_shape_vec((rows, cols), low_map).context("Failed to reshape low rejection map")?),
            Some(Array2::from_shape_vec((rows, cols), high_map).context("Failed to reshape high rejection map")?),
        )
    } else {
        (None, None)
    };

    Ok(StackResult {
        image: Array2::from_shape_vec((rows, cols), result_data)
            .context("Failed to reshape stacked image")?,
        frame_count: n,
        rejected_pixels,
        offsets,
        rejection_low,
        rejection_high,
        normalization_applied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sigma_clip_clean_data() {
        let mut vals = vec![10.0, 10.1, 9.9, 10.0, 10.2];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert!((mean - 10.04).abs() < 0.1);
        assert_eq!(rejected, 0);
    }

    #[test]
    fn test_sigma_clip_with_outlier() {
        let mut vals = vec![10.0, 10.1, 9.9, 10.0, 500.0];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert!(mean < 15.0);
        assert!(rejected > 0);
    }

    #[test]
    fn test_sigma_clip_cosmic_ray() {
        let mut vals = vec![100.0, 100.2, 99.8, 100.1, 100.0, 5000.0, 99.9];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 2.0, 2.0, 5);
        assert!((mean - 100.0).abs() < 1.0);
        assert!(rejected >= 1);
    }

    #[test]
    fn test_sigma_clip_zero_mad_keeps_small_deviations() {
        let mut vals = vec![1000.0, 1000.0, 1000.0, 1001.0, 999.0];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert_eq!(rejected, 0);
        assert!((mean - 1000.0).abs() < 1e-4);

        let mut vals = vec![1000.0, 1000.0, 1000.0, 1001.0, 1002.0];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert_eq!(rejected, 0);
        assert!((mean - 1000.6).abs() < 1e-3);
    }

    #[test]
    fn test_sigma_clip_zero_mad_still_rejects_outlier() {
        let mut vals = vec![100.0, 100.0, 100.0, 50000.0, 100.0];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert_eq!(rejected, 1);
        assert!((mean - 100.0).abs() < 1e-4);
    }

    #[test]
    fn test_sigma_clip_all_equal_rejects_nothing() {
        let mut vals = vec![7.0; 6];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert_eq!(rejected, 0);
        assert_eq!(mean, 7.0);
    }

    #[test]
    fn weighted_combine_zero_mad_keeps_small_deviations() {
        let mut vals = vec![
            (1000.0f32, 1.0f32),
            (1000.0f32, 1.0f32),
            (1000.0f32, 1.0f32),
            (1001.0f32, 1.0f32),
            (999.0f32, 1.0f32),
        ];
        let (mean, rejected) = sigma_clip_combine_weighted(&mut vals, 3.0, 3.0, 5);
        assert_eq!(rejected, 0);
        assert!((mean - 1000.0).abs() < 1e-4);
    }

    #[test]
    fn test_sigma_clip_empty() {
        let mut vals: Vec<f32> = vec![];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert!(mean.is_nan());
        assert_eq!(rejected, 0);
    }

    #[test]
    fn test_sigma_clip_single() {
        let mut vals = vec![42.0];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert_eq!(mean, 42.0);
        assert_eq!(rejected, 0);
    }

    #[test]
    fn test_stack_identical() {
        let img = Array2::from_shape_vec(
            (4, 4),
            (0..16).map(|i| i as f32 * 10.0).collect(),
        )
            .unwrap();

        let images = vec![img.clone(), img.clone(), img.clone()];
        let config = StackConfig {
            align: false,
            ..Default::default()
        };

        let result = stack_images(&images, &config).unwrap();
        assert_eq!(result.frame_count, 3);
        assert!((result.image[[0, 0]] - 0.0).abs() < 1e-4);
        assert!((result.image[[1, 1]] - 50.0).abs() < 1e-4);
    }

    #[test]
    fn test_stack_rejects_outlier() {
        let clean = Array2::from_shape_vec((4, 4), vec![100.0; 16]).unwrap();

        let mut noisy = clean.clone();
        noisy[[2, 2]] = 50000.0;

        let images = vec![
            clean.clone(),
            clean.clone(),
            clean.clone(),
            noisy,
            clean.clone(),
        ];

        let config = StackConfig {
            sigma_low: 3.0,
            sigma_high: 3.0,
            max_iterations: 5,
            align: false,
            ..StackConfig::default()
        };

        let result = stack_images(&images, &config).unwrap();
        assert!((result.image[[2, 2]] - 100.0).abs() < 1.0);
        assert!(result.rejected_pixels > 0);
    }

    #[test]
    fn weighted_combine_favors_high_weight() {
        let mut vals = vec![(10.0f32, 1.0f32), (20.0f32, 3.0f32)];
        let (m, _) = sigma_clip_combine_weighted(&mut vals, 3.0, 3.0, 5);
        assert!((m - 17.5).abs() < 1e-4);
    }

    #[test]
    fn weighted_combine_rejects_outlier() {
        let mut vals = vec![
            (100.0f32, 1.0f32),
            (100.2f32, 1.0f32),
            (99.8f32, 1.0f32),
            (100.1f32, 1.0f32),
            (5000.0f32, 1.0f32),
        ];
        let (m, rej) = sigma_clip_combine_weighted(&mut vals, 2.0, 2.0, 5);
        assert!((m - 100.0).abs() < 1.0);
        assert!(rej >= 1);
    }

    #[test]
    fn weighted_stack_pulls_toward_high_weight_frame() {
        let dark = Array2::from_elem((2, 2), 10.0f32);
        let bright = Array2::from_elem((2, 2), 20.0f32);
        let config = StackConfig {
            align: false,
            weights: Some(vec![1.0, 3.0]),
            normalization: NormalizationMethod::None,
            ..Default::default()
        };
        let result = stack_images(&[dark, bright], &config).unwrap();
        assert!((result.image[[0, 0]] - 17.5).abs() < 1e-3);
    }

    fn legacy_sigma_clip(values: &mut Vec<f32>, sigma_low: f32, sigma_high: f32, max_iter: usize) -> (f32, u32) {
        let n_orig = values.len();
        if n_orig == 0 {
            return (f32::NAN, 0);
        }
        if n_orig == 1 {
            return (values[0], 0);
        }
        let mut scratch: Vec<f32> = Vec::with_capacity(n_orig);
        let mut len = n_orig;
        let mut rejected = 0u32;
        let mut last_center: f32 = f32::NAN;
        for iteration in 0..max_iter {
            if len < 2 {
                break;
            }
            let (center, sigma) = if iteration == 0 {
                let mid = len / 2;
                values[..len].select_nth_unstable_by(mid, |a, b| f32_cmp(a, b));
                let med = values[mid];
                scratch.clear();
                scratch.extend(values[..len].iter().map(|v| (v - med).abs()));
                let dmid = scratch.len() / 2;
                scratch.select_nth_unstable_by(dmid, |a, b| f32_cmp(a, b));
                let mad = scratch[dmid];
                (med, scale_from_deviations(mad, &scratch[..len]))
            } else {
                let n = len as f64;
                let mean = values[..len].iter().map(|v| *v as f64).sum::<f64>() / n;
                let variance = values[..len]
                    .iter()
                    .map(|v| {
                        let d = *v as f64 - mean;
                        d * d
                    })
                    .sum::<f64>()
                    / (n - 1.0).max(1.0);
                (mean as f32, variance.sqrt() as f32)
            };
            last_center = center;
            if sigma <= 0.0 {
                break;
            }
            let lo = -sigma_low * sigma;
            let hi = sigma_high * sigma;
            let mut write = 0;
            for read in 0..len {
                let dev = values[read] - center;
                if dev >= lo && dev <= hi {
                    values[write] = values[read];
                    write += 1;
                }
            }
            let removed = len - write;
            rejected += removed as u32;
            len = write;
            if removed == 0 {
                break;
            }
        }
        if len == 0 {
            let fallback = if last_center.is_finite() { last_center } else { 0.0 };
            return (fallback, rejected);
        }
        let mean = values[..len].iter().map(|v| *v as f64).sum::<f64>() / len as f64;
        (mean as f32, rejected)
    }

    struct Lcg(u64);

    impl Lcg {
        fn next_unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
        }
    }

    fn samples_from(values: &[f32]) -> Vec<Sample> {
        values.iter().enumerate().map(|(i, &v)| Sample::plain(v, i as u16)).collect()
    }

    fn params(rejection: RejectionMethod, combine: CombineMethod) -> RejectionParams {
        RejectionParams { rejection, combine, ..RejectionParams::default() }
    }

    #[test]
    fn kernel_sigma_clip_matches_legacy_bit_for_bit() {
        let mut rng = Lcg(42);
        for case in 0..400 {
            let n = 2 + (case % 11);
            let mut values: Vec<f32> = (0..n).map(|_| 1000.0 + (rng.next_unit() - 0.5) * 20.0).collect();
            if case % 3 == 0 {
                values[case % n] = 60000.0;
            }
            if case % 5 == 0 {
                values[(case + 1) % n] = -500.0;
            }
            let (sigma_low, sigma_high) = if case % 2 == 0 { (3.0, 3.0) } else { (2.0, 2.5) };
            let mut legacy_values = values.clone();
            let (legacy_value, legacy_rejected) = legacy_sigma_clip(&mut legacy_values, sigma_low, sigma_high, 5);
            let mut samples = samples_from(&values);
            let cfg = RejectionParams { sigma_low, sigma_high, max_iterations: 5, ..params(RejectionMethod::SigmaClip, CombineMethod::Mean) };
            let out = reject_and_combine(&mut samples, None, &cfg);
            assert_eq!(out.value.to_bits(), legacy_value.to_bits(), "case {case}: {values:?}");
            assert_eq!(out.rejected_low as u32 + out.rejected_high as u32, legacy_rejected, "case {case}");
            assert_eq!(out.kept, n - legacy_rejected as usize);
        }
    }

    #[test]
    fn kernel_leaves_rejected_samples_after_kept_prefix() {
        let mut samples = samples_from(&[100.0, 101.0, 50000.0, 99.0, -50000.0, 100.0, 102.0]);
        let cfg = RejectionParams { sigma_low: 3.0, sigma_high: 3.0, ..params(RejectionMethod::SigmaClip, CombineMethod::Mean) };
        let out = reject_and_combine(&mut samples, None, &cfg);
        assert_eq!(out.kept, 5);
        assert_eq!(out.rejected_high, 1);
        assert_eq!(out.rejected_low, 1);
        let mut rejected_frames: Vec<u16> = samples[out.kept..].iter().map(|s| s.frame).collect();
        rejected_frames.sort_unstable();
        assert_eq!(rejected_frames, vec![2, 4]);
        assert!(samples[..out.kept].iter().all(|s| (99.0..=102.0).contains(&s.value)));
        assert!((out.value - 100.4).abs() < 1e-4);
    }

    #[test]
    fn winsorized_rejects_single_high_outlier() {
        let mut samples = samples_from(&[10.0, 10.2, 9.9, 10.1, 60.0]);
        let out = reject_and_combine(&mut samples, None, &params(RejectionMethod::WinsorizedSigmaClip, CombineMethod::Mean));
        assert!((out.value - 10.05).abs() < 1e-3, "value {}", out.value);
        assert_eq!(out.rejected_high, 1);
        assert_eq!(out.rejected_low, 0);
        assert_eq!(out.kept, 4);
    }

    #[test]
    fn winsorized_keeps_gaussian_like_scatter() {
        let mut samples = samples_from(&[100.0, 101.5, 98.7, 100.4, 99.1, 100.9, 99.6]);
        let out = reject_and_combine(&mut samples, None, &params(RejectionMethod::WinsorizedSigmaClip, CombineMethod::Mean));
        assert_eq!(out.rejected_low + out.rejected_high, 0);
        assert!((out.value - 100.0286).abs() < 1e-3);
    }

    #[test]
    fn percentile_clip_rejects_high_outlier_from_three() {
        let mut samples = samples_from(&[10.0, 10.1, 60.0]);
        let cfg = RejectionParams { percentile_high: 0.2, ..params(RejectionMethod::PercentileClip, CombineMethod::Mean) };
        let out = reject_and_combine(&mut samples, None, &cfg);
        assert_eq!(out.rejected_high, 1);
        assert_eq!(out.rejected_low, 0);
        assert!((out.value - 10.05).abs() < 1e-4);
    }

    #[test]
    fn percentile_clip_is_single_pass_and_also_rejects_low() {
        let mut samples = samples_from(&[1.0, 10.0, 10.2, 9.9, 10.1, 60.0]);
        let cfg = RejectionParams { percentile_low: 0.2, percentile_high: 0.1, ..params(RejectionMethod::PercentileClip, CombineMethod::Mean) };
        let out = reject_and_combine(&mut samples, None, &cfg);
        assert_eq!(out.rejected_low, 1);
        assert_eq!(out.rejected_high, 1);
        assert_eq!(out.kept, 4);
    }

    #[test]
    fn linear_fit_rejects_only_injected_outlier_on_ramp() {
        let mut samples = samples_from(&[100.0, 101.0, 102.0, 103.0, 104.0, 105.0, 106.0, 500.0]);
        let out = reject_and_combine(&mut samples, None, &params(RejectionMethod::LinearFitClip, CombineMethod::Mean));
        assert_eq!(out.rejected_high, 1);
        assert_eq!(out.rejected_low, 0);
        assert_eq!(out.kept, 7);
        assert!((out.value - 103.0).abs() < 1e-4, "value {}", out.value);
    }

    #[test]
    fn linear_fit_keeps_noisy_ramp_without_outlier() {
        let mut samples = samples_from(&[100.3, 100.9, 102.2, 102.8, 104.1, 104.9, 106.2, 106.8, 108.1]);
        let out = reject_and_combine(&mut samples, None, &params(RejectionMethod::LinearFitClip, CombineMethod::Mean));
        assert_eq!(out.rejected_low + out.rejected_high, 0);
    }

    #[test]
    fn linear_fit_falls_back_to_sigma_clip_below_five_samples() {
        let values = [100.0, 100.2, 99.8, 5000.0];
        let mut a = samples_from(&values);
        let mut b = samples_from(&values);
        let fit = reject_and_combine(&mut a, None, &params(RejectionMethod::LinearFitClip, CombineMethod::Mean));
        let clip = reject_and_combine(&mut b, None, &params(RejectionMethod::SigmaClip, CombineMethod::Mean));
        assert_eq!(fit, clip);
        assert_eq!(fit.rejected_high, 1);
    }

    #[test]
    fn min_max_drops_exactly_the_two_extremes() {
        let mut samples = samples_from(&[5.0, 1.0, 9.0, 3.0, 7.0]);
        let cfg = RejectionParams { minmax_low: 1, minmax_high: 1, ..params(RejectionMethod::MinMax, CombineMethod::Mean) };
        let out = reject_and_combine(&mut samples, None, &cfg);
        assert_eq!(out.rejected_low, 1);
        assert_eq!(out.rejected_high, 1);
        assert_eq!(out.kept, 3);
        assert!((out.value - 5.0).abs() < 1e-6);
        let mut kept: Vec<f32> = samples[..3].iter().map(|s| s.value).collect();
        kept.sort_by(f32_cmp);
        assert_eq!(kept, vec![3.0, 5.0, 7.0]);
    }

    #[test]
    fn min_max_rejects_nothing_when_frames_do_not_exceed_low_plus_high() {
        let mut samples = samples_from(&[5.0, 1.0]);
        let cfg = RejectionParams { minmax_low: 1, minmax_high: 1, ..params(RejectionMethod::MinMax, CombineMethod::Mean) };
        let out = reject_and_combine(&mut samples, None, &cfg);
        assert_eq!(out.kept, 2);
        assert!((out.value - 3.0).abs() < 1e-6);
    }

    #[test]
    fn median_combine_ignores_weights() {
        let values = [1.0, 2.0, 3.0, 4.0, 100.0];
        let weights = [10.0, 1.0, 1.0, 1.0, 1.0];
        let mut a = samples_from(&values);
        let mut b = samples_from(&values);
        let cfg = params(RejectionMethod::None, CombineMethod::Median);
        let unweighted = reject_and_combine(&mut a, None, &cfg);
        let weighted = reject_and_combine(&mut b, Some(&weights), &cfg);
        assert_eq!(unweighted.value.to_bits(), weighted.value.to_bits());
        assert_eq!(unweighted.value, 3.0);
        let mut c = samples_from(&values);
        let mean = reject_and_combine(&mut c, Some(&weights), &params(RejectionMethod::None, CombineMethod::Mean));
        assert!((mean.value - (10.0 + 2.0 + 3.0 + 4.0 + 100.0) / 14.0).abs() < 1e-4);
    }

    #[test]
    fn median_combine_averages_the_two_middle_values_for_even_counts() {
        let mut samples = samples_from(&[4.0, 1.0, 3.0, 2.0]);
        let out = reject_and_combine(&mut samples, None, &params(RejectionMethod::None, CombineMethod::Median));
        assert_eq!(out.value, 2.5);
    }

    #[test]
    fn min_and_max_combine_after_rejection() {
        let values = [10.0, 12.0, 11.0, 9.0, 5000.0];
        let mut a = samples_from(&values);
        let mut b = samples_from(&values);
        let min = reject_and_combine(&mut a, None, &RejectionParams { sigma_low: 3.0, sigma_high: 3.0, ..params(RejectionMethod::SigmaClip, CombineMethod::Min) });
        let max = reject_and_combine(&mut b, None, &RejectionParams { sigma_low: 3.0, sigma_high: 3.0, ..params(RejectionMethod::SigmaClip, CombineMethod::Max) });
        assert_eq!(min.value, 9.0);
        assert_eq!(max.value, 12.0);
        assert_eq!(max.rejected_high, 1);
    }

    #[test]
    fn no_samples_yields_nan_and_single_sample_passes_through() {
        let mut empty: Vec<Sample> = Vec::new();
        let out = reject_and_combine(&mut empty, None, &RejectionParams::default());
        assert!(out.value.is_nan());
        assert_eq!(out.kept, 0);
        let mut one = samples_from(&[42.0]);
        let out = reject_and_combine(&mut one, None, &params(RejectionMethod::MinMax, CombineMethod::Median));
        assert_eq!(out.value, 42.0);
        assert_eq!(out.kept, 1);
    }

    #[test]
    fn scale_offset_normalized_samples_reject_on_cmp_and_combine_on_value() {
        let mut samples = vec![
            Sample::normalized(100.0, 100.0, 0),
            Sample::normalized(150.0, 100.0, 1),
            Sample::normalized(100.0, 100.0, 2),
            Sample::normalized(100.0, 100.0, 3),
            Sample::normalized(60000.0, 59950.0, 4),
        ];
        let cfg = RejectionParams { sigma_low: 3.0, sigma_high: 3.0, ..params(RejectionMethod::SigmaClip, CombineMethod::Mean) };
        let out = reject_and_combine(&mut samples, None, &cfg);
        assert_eq!(out.rejected_high, 1);
        assert_eq!(out.kept, 4);
        assert!((out.value - 112.5).abs() < 1e-4);
    }

    fn textured_frame(level: f32) -> Array2<f32> {
        Array2::from_shape_fn((12, 12), |(y, x)| level + ((y * 7 + x * 13) % 11) as f32 * 2.0)
    }

    #[test]
    fn additive_normalization_matches_offset_frame_to_reference() {
        let base = textured_frame(100.0);
        let images = vec![base.clone(), base.clone(), base.clone(), textured_frame(150.0)];
        let config = StackConfig {
            align: false,
            normalization: NormalizationMethod::Additive,
            rejection_normalization: RejectionNormalization::None,
            sigma_low: 3.0,
            sigma_high: 3.0,
            ..StackConfig::default()
        };
        let result = stack_images(&images, &config).unwrap();
        assert_eq!(result.rejected_pixels, 0);
        for (a, b) in result.image.iter().zip(base.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        assert_eq!(result.normalization_applied.len(), 4);
        assert_eq!(result.normalization_applied[0], (0.0, 1.0));
        assert!((result.normalization_applied[3].0 + 50.0).abs() < 1e-6);
        assert_eq!(result.normalization_applied[3].1, 1.0);

        let raw = StackConfig {
            normalization: NormalizationMethod::None,
            ..config.clone()
        };
        let raw_result = stack_images(&images, &raw).unwrap();
        assert!(raw_result.rejected_pixels > 0);
        assert!(raw_result.normalization_applied.iter().all(|&(o, s)| o == 0.0 && s == 1.0));
    }

    #[test]
    fn scale_offset_rejection_normalization_keeps_offset_frame_without_rescaling_output() {
        let base = textured_frame(100.0);
        let images = vec![base.clone(), base.clone(), base.clone(), textured_frame(150.0)];
        let config = StackConfig {
            align: false,
            normalization: NormalizationMethod::None,
            rejection_normalization: RejectionNormalization::ScaleOffset,
            sigma_low: 3.0,
            sigma_high: 3.0,
            ..StackConfig::default()
        };
        let result = stack_images(&images, &config).unwrap();
        assert_eq!(result.rejected_pixels, 0);
        for (a, b) in result.image.iter().zip(base.iter()) {
            assert!((a - (b + 12.5)).abs() < 1e-3, "{a} vs {}", b + 12.5);
        }
    }

    #[test]
    fn multiplicative_normalization_scales_frame_to_reference_median() {
        let base = textured_frame(100.0);
        let doubled = base.mapv(|v| v * 2.0);
        let images = vec![base.clone(), doubled];
        let config = StackConfig {
            align: false,
            normalization: NormalizationMethod::Multiplicative,
            rejection: RejectionMethod::None,
            ..StackConfig::default()
        };
        let result = stack_images(&images, &config).unwrap();
        for (a, b) in result.image.iter().zip(base.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        assert!((result.normalization_applied[1].1 - 0.5).abs() < 1e-6);
        assert_eq!(result.normalization_applied[1].0, 0.0);
    }

    #[test]
    fn additive_scaling_normalization_matches_dispersion_and_location() {
        let base = textured_frame(100.0);
        let stretched = base.mapv(|v| (v - 100.0) * 3.0 + 400.0);
        let images = vec![base.clone(), stretched];
        let config = StackConfig {
            align: false,
            normalization: NormalizationMethod::AdditiveScaling,
            rejection: RejectionMethod::None,
            ..StackConfig::default()
        };
        let result = stack_images(&images, &config).unwrap();
        for (a, b) in result.image.iter().zip(base.iter()) {
            assert!((a - b).abs() < 1e-2, "{a} vs {b}");
        }
        let (offset, scale) = result.normalization_applied[1];
        assert!((scale - 1.0 / 3.0).abs() < 1e-4, "scale {scale}");
        assert!((400.0 * scale + offset - 100.0).abs() < 1e-2, "offset {offset}");
    }

    #[test]
    fn rejection_maps_count_matches_rejected_pixels() {
        let clean = Array2::from_shape_vec((4, 4), vec![100.0; 16]).unwrap();
        let mut hot = clean.clone();
        hot[[2, 2]] = 50000.0;
        hot[[0, 1]] = 50000.0;
        let mut cold = clean.clone();
        cold[[3, 3]] = -50000.0;
        let images = vec![clean.clone(), hot, clean.clone(), cold, clean.clone()];
        let config = StackConfig {
            align: false,
            sigma_low: 3.0,
            sigma_high: 3.0,
            rejection_maps: true,
            ..StackConfig::default()
        };
        let result = stack_images(&images, &config).unwrap();
        let low = result.rejection_low.as_ref().expect("low map");
        let high = result.rejection_high.as_ref().expect("high map");
        assert_eq!(low.dim(), (4, 4));
        let total: u64 = low.iter().map(|&v| v as u64).sum::<u64>() + high.iter().map(|&v| v as u64).sum::<u64>();
        assert_eq!(total, result.rejected_pixels);
        assert_eq!(result.rejected_pixels, 3);
        assert_eq!(high[[2, 2]], 1);
        assert_eq!(high[[0, 1]], 1);
        assert_eq!(low[[3, 3]], 1);
        assert_eq!(low[[2, 2]], 0);

        let without = StackConfig { rejection_maps: false, ..config };
        let plain = stack_images(&images, &without).unwrap();
        assert!(plain.rejection_low.is_none() && plain.rejection_high.is_none());
        assert_eq!(plain.rejected_pixels, 3);
    }

    #[test]
    fn stack_images_normalization_skips_nan_borders_when_measuring_frames() {
        let base = textured_frame(100.0);
        let mut shifted = textured_frame(150.0);
        for x in 0..12 {
            shifted[[0, x]] = f32::NAN;
        }
        let images = vec![base.clone(), shifted];
        let config = StackConfig {
            align: false,
            normalization: NormalizationMethod::Additive,
            rejection: RejectionMethod::None,
            ..StackConfig::default()
        };
        let result = stack_images(&images, &config).unwrap();
        assert!((result.normalization_applied[1].0 + 50.0).abs() < 1e-6);
        assert!((result.image[[0, 0]] - base[[0, 0]]).abs() < 1e-3);
        assert!((result.image[[5, 5]] - base[[5, 5]]).abs() < 1e-3);
    }
}
