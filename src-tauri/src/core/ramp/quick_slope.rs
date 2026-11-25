use std::f64::consts::{FRAC_PI_2, PI};
use std::ops::Range;

use anyhow::{bail, Context, Result};
use ndarray::{s, Array1, Array2, ArrayViewMut2, Axis, Zip};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::cube::lazy::LazyCube;
use crate::core::ramp::group_dq::RampDq;
use crate::core::ramp::info::RampInfo;
use crate::core::ramp::irs2::{Irs2Layout, Irs2Resolution, Irs2Sample, IRS2_ALREADY_STRIPPED_NOTE};
use crate::core::stacking::{stop_if_cancelled, CancelCheck};
use crate::math::median::median_f32_mut;

pub const DEFAULT_SAT_DN: f32 = 62_258.0;
pub const DEFAULT_JUMP_K: f32 = 5.0;
pub const DEFAULT_SCALE_FLOOR_DN: f32 = 3.0;
pub const DEFAULT_MIN_GROUPS_OLS: usize = 4;
pub const DEFAULT_REF_WINDOW_ROWS: usize = 200;
pub const MIN_REF_WINDOW_ROWS: usize = 40;
pub const MAD_TO_SIGMA_F32: f32 = 1.4826;
pub const MEDIAN_EFFICIENCY_F32: f32 = 1.2533;
pub const BAND_SCALE_ROW_STEP: usize = 3;
pub const BAND_SCALE_COL_STEP: usize = 5;
pub const NO_LAYOUT_BAND_ROWS: usize = 256;
pub const MAX_SAT_DN: f32 = 65_535.0;
pub const MIN_JUMP_K: f32 = 1.0;
pub const MIN_GROUPS_OLS_FLOOR: usize = 2;
pub const JUMP_MODEL_GAIN_E_PER_DN: f32 = 1.0;
pub const MIN_GAIN_E_PER_DN: f32 = 0.001;
pub const MAX_GAIN_E_PER_DN: f32 = 1000.0;
pub const SECOND_DIFFERENCE_VARIANCE_RATIO: f64 = 3.0;

pub const DQ_DO_NOT_USE: u32 = 1;
pub const DQ_SATURATED: u32 = 2;
pub const DQ_JUMP_DET: u32 = 4;

pub const WARN_REF_CORRECTION_OFF: &str = "IRS2 reference rows stripped but no offset correction applied (ref_correction=off): the slope keeps the per-group bias drift (additive offsets up to 0.4 DN/s measured on NIRSpec IRS2 data)";
pub const WARN_NO_LAYOUT: &str = "no IRS2 layout: frame kept at its input size, no reference-offset correction";
pub const ERR_AMPLIFIER_NEEDS_LAYOUT: &str = "ref_correction=amplifier needs NRS_NORM/NRS_REF and the unstripped frame height";
pub const ERR_TGROUP_MISSING: &str = "TGROUP missing: cannot convert DN to DN/s (TFRAME*(NFRAMES+GROUPGAP) also absent)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefCorrection {
    Auto,
    Amplifier,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuickSlopeParams {
    pub sat_dn: f32,
    pub jump_k: f32,
    pub scale_floor_dn: f32,
    pub min_groups_ols: usize,
    pub ref_correction: RefCorrection,
    pub ref_window_rows: Option<usize>,
    pub gain_e_per_dn: Option<f32>,
}

impl Default for QuickSlopeParams {
    fn default() -> Self {
        Self {
            sat_dn: DEFAULT_SAT_DN,
            jump_k: DEFAULT_JUMP_K,
            scale_floor_dn: DEFAULT_SCALE_FLOOR_DN,
            min_groups_ols: DEFAULT_MIN_GROUPS_OLS,
            ref_correction: RefCorrection::Auto,
            ref_window_rows: Some(DEFAULT_REF_WINDOW_ROWS),
            gain_e_per_dn: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoiseModel {
    ReadNoiseOnly,
    ReadAndPhoton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DqSource {
    Threshold,
    Groupdq,
}

impl QuickSlopeParams {
    pub fn noise_model(&self) -> NoiseModel {
        match self.gain_e_per_dn {
            Some(_) => NoiseModel::ReadAndPhoton,
            None => NoiseModel::ReadNoiseOnly,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !self.sat_dn.is_finite() || self.sat_dn <= 0.0 || self.sat_dn > MAX_SAT_DN {
            bail!("sat_dn must be a finite threshold in (0, {MAX_SAT_DN}] DN, got {}", self.sat_dn);
        }
        if self.jump_k.is_nan() || self.jump_k < MIN_JUMP_K {
            bail!("jump_k must be at least {MIN_JUMP_K}, got {}", self.jump_k);
        }
        if self.scale_floor_dn.is_nan() || self.scale_floor_dn < 0.0 {
            bail!("scale_floor_dn must be zero or positive, got {}", self.scale_floor_dn);
        }
        if self.min_groups_ols < MIN_GROUPS_OLS_FLOOR {
            bail!("min_groups_ols must be at least {MIN_GROUPS_OLS_FLOOR}, got {}", self.min_groups_ols);
        }
        if let Some(window) = self.ref_window_rows {
            if window < MIN_REF_WINDOW_ROWS {
                bail!("ref_window_rows must be at least {MIN_REF_WINDOW_ROWS} rows (or null for the whole band), got {window}");
            }
        }
        if let Some(gain) = self.gain_e_per_dn {
            if !gain.is_finite() || !(MIN_GAIN_E_PER_DN..=MAX_GAIN_E_PER_DN).contains(&gain) {
                bail!("gain_e_per_dn must be between {MIN_GAIN_E_PER_DN} and {MAX_GAIN_E_PER_DN} electrons per DN (or null when the gain is not known), got {gain}");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PixelFit {
    pub slope: f32,
    pub ngood: u32,
    pub dq: u32,
    pub noise: f32,
    pub n_usable: u32,
    pub first_saturated: Option<u32>,
}

struct UsableSpan {
    start: usize,
    end: usize,
    saturated_at: Option<usize>,
}

fn usable_span(raw: &[f32], corrected: &[f32], sat_dn: f32, group_dq: Option<&[u32]>) -> UsableSpan {
    let len = raw.len().min(corrected.len());
    let Some(flags) = group_dq else {
        for g in 0..len {
            if raw[g] >= sat_dn {
                return UsableSpan { start: 0, end: g, saturated_at: Some(g) };
            }
            if !corrected[g].is_finite() {
                return UsableSpan { start: 0, end: g, saturated_at: None };
            }
        }
        return UsableSpan { start: 0, end: len, saturated_at: None };
    };
    let len = len.min(flags.len());
    let mut start = 0;
    while start < len && flags[start] & DQ_DO_NOT_USE != 0 && flags[start] & DQ_SATURATED == 0 {
        start += 1;
    }
    for g in start..len {
        if flags[g] & DQ_SATURATED != 0 {
            return UsableSpan { start, end: g, saturated_at: Some(g) };
        }
        if flags[g] & DQ_DO_NOT_USE != 0 || !corrected[g].is_finite() {
            return UsableSpan { start, end: g, saturated_at: None };
        }
    }
    UsableSpan { start, end: len, saturated_at: None }
}

fn ols_slope(values: &[f32], tgroup_s: f64) -> f64 {
    let n = values.len() as f64;
    let t_mean = tgroup_s * (n - 1.0) / 2.0;
    let v_mean = values.iter().map(|v| *v as f64).sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (g, v) in values.iter().enumerate() {
        let dt = g as f64 * tgroup_s - t_mean;
        num += dt * (*v as f64 - v_mean);
        den += dt * dt;
    }
    num / den
}

fn robust_difference_scale(d: &[f32], scratch: &mut [f32], med: f32, params: &QuickSlopeParams, band_scale_dn: f32) -> f32 {
    for (s, v) in scratch.iter_mut().zip(d.iter()) {
        *s = (v - med).abs();
    }
    let mad_sigma = MAD_TO_SIGMA_F32 * median_f32_mut(scratch);
    let gain = params.gain_e_per_dn.unwrap_or(JUMP_MODEL_GAIN_E_PER_DN);
    let model_sigma = (band_scale_dn * band_scale_dn + med.max(0.0) / gain).sqrt();
    mad_sigma.max(params.scale_floor_dn).max(model_sigma)
}

fn photon_variance_of_a_step(step_dn: f64, params: &QuickSlopeParams) -> f64 {
    params.gain_e_per_dn.map_or(0.0, |gain| step_dn.max(0.0) / gain as f64)
}

fn ols_step_variance(groups: usize, read_variance: f64, photon_variance: f64) -> f64 {
    let n = groups as f64;
    let span = n * (n * n - 1.0);
    12.0 * read_variance / span + 6.0 * (n * n + 1.0) * photon_variance / (5.0 * span)
}

fn independent_median_factor(kept: usize) -> f64 {
    let k = kept as f64;
    let offset = if kept % 2 == 1 { FRAC_PI_2 - 1.0 } else { PI - 2.0 };
    FRAC_PI_2 * k / (k + offset)
}

fn median_step_variance(kept: usize, adjacent_pairs: usize, read_variance: f64, photon_variance: f64) -> f64 {
    let single = 2.0 * read_variance + photon_variance;
    if kept == 0 {
        return f64::NAN;
    }
    if single <= 0.0 {
        return 0.0;
    }
    let correlation = -read_variance / single;
    let k = kept as f64;
    match kept {
        1 => single,
        2 => single * (1.0 + if adjacent_pairs == 1 { correlation } else { 0.0 }) / 2.0,
        _ => single / k * independent_median_factor(kept) * (1.0 + 4.0 * adjacent_pairs as f64 / (PI * k) * correlation.asin()),
    }
}

pub fn fit_pixel(
    raw: &[f32],
    corrected: &[f32],
    tgroup_s: f64,
    band_scale_dn: f32,
    params: &QuickSlopeParams,
    diffs: &mut Vec<f32>,
    flagged: &mut Vec<usize>,
) -> PixelFit {
    fit_pixel_with_group_dq(raw, corrected, None, tgroup_s, band_scale_dn, params, diffs, flagged)
}

pub fn fit_pixel_with_group_dq(
    raw: &[f32],
    corrected: &[f32],
    group_dq: Option<&[u32]>,
    tgroup_s: f64,
    band_scale_dn: f32,
    params: &QuickSlopeParams,
    diffs: &mut Vec<f32>,
    flagged: &mut Vec<usize>,
) -> PixelFit {
    let span = usable_span(raw, corrected, params.sat_dn, group_dq);
    let n = span.end - span.start;
    let values = &corrected[span.start..span.end];
    let mut dq = if span.saturated_at.is_some() { DQ_SATURATED } else { 0 };
    let n_usable = n as u32;
    let first_saturated = span.saturated_at.map(|g| g as u32);
    flagged.clear();
    if n < 2 {
        dq |= DQ_DO_NOT_USE;
        return PixelFit { slope: f32::NAN, ngood: n_usable, dq, noise: f32::NAN, n_usable, first_saturated };
    }
    let m = n - 1;
    if let Some(flags) = group_dq {
        flagged.extend((0..m).filter(|i| flags[span.start + i + 1] & DQ_JUMP_DET != 0).map(|i| span.start + i));
    }
    if n == 2 && flagged.is_empty() {
        let slope = ((values[1] - values[0]) as f64 / tgroup_s) as f32;
        return PixelFit { slope, ngood: 2, dq, noise: f32::NAN, n_usable, first_saturated };
    }
    diffs.clear();
    diffs.extend((0..m).map(|i| values[i + 1] - values[i]));
    diffs.extend_from_within(0..m);
    let (d, scratch) = diffs.split_at_mut(m);
    let med = median_f32_mut(scratch);
    let s = robust_difference_scale(d, scratch, med, params, band_scale_dn);
    if n >= 4 {
        let threshold = params.jump_k * s;
        for (i, v) in d.iter().enumerate() {
            let index = span.start + i;
            if (*v - med).abs() > threshold && !flagged.contains(&index) {
                flagged.push(index);
            }
        }
        flagged.sort_unstable();
    }
    let read_variance = (band_scale_dn as f64).powi(2) / 2.0;
    if !flagged.is_empty() {
        let (mut kept, mut adjacent_pairs, mut previous_kept) = (0, 0, false);
        for (i, v) in d.iter().enumerate() {
            let is_kept = !flagged.contains(&(span.start + i));
            if is_kept {
                scratch[kept] = *v;
                kept += 1;
                adjacent_pairs += previous_kept as usize;
            }
            previous_kept = is_kept;
        }
        let ngood = (n - flagged.len()) as u32;
        if kept == 0 {
            return PixelFit { slope: f32::NAN, ngood, dq: dq | DQ_JUMP_DET | DQ_DO_NOT_USE, noise: f32::NAN, n_usable, first_saturated };
        }
        let kept_median = median_f32_mut(&mut scratch[..kept]) as f64;
        let photon_variance = photon_variance_of_a_step(kept_median, params);
        let noise = median_step_variance(kept, adjacent_pairs, read_variance, photon_variance).sqrt() / tgroup_s;
        return PixelFit {
            slope: (kept_median / tgroup_s) as f32,
            ngood,
            dq: dq | DQ_JUMP_DET,
            noise: noise as f32,
            n_usable,
            first_saturated,
        };
    }
    if n >= params.min_groups_ols {
        let slope = ols_slope(values, tgroup_s);
        let photon_variance = photon_variance_of_a_step(slope * tgroup_s, params);
        let noise = ols_step_variance(n, read_variance, photon_variance).sqrt() / tgroup_s;
        return PixelFit { slope: slope as f32, ngood: n_usable, dq, noise: noise as f32, n_usable, first_saturated };
    }
    let photon_variance = photon_variance_of_a_step(med as f64, params);
    let noise = median_step_variance(m, m - 1, read_variance, photon_variance).sqrt() / tgroup_s;
    PixelFit { slope: (med as f64 / tgroup_s) as f32, ngood: n_usable, dq, noise: noise as f32, n_usable, first_saturated }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceOffsets {
    pub block_rows: usize,
    pub ngroups: usize,
    pub width: usize,
    pub values: Vec<f32>,
}

impl ReferenceOffsets {
    pub fn zero(ngroups: usize, width: usize) -> Self {
        Self { block_rows: 0, ngroups, width, values: vec![0.0; ngroups * width] }
    }

    pub fn blocks(&self) -> usize {
        self.values.len() / (self.ngroups * self.width).max(1)
    }

    pub fn row(&self, band_row: usize, g: usize) -> &[f32] {
        let block = if self.block_rows == 0 { 0 } else { band_row / self.block_rows };
        let start = (block * self.ngroups + g) * self.width;
        &self.values[start..start + self.width]
    }

    pub fn get(&self, band_row: usize, g: usize, x: usize) -> f32 {
        self.row(band_row, g)[x]
    }
}

pub fn reference_sample_sets(band_rows: Range<usize>, layout: &Irs2Layout, window_rows: Option<usize>) -> Result<Vec<Vec<usize>>> {
    let block_rows = layout.block_rows();
    let band_len = band_rows.len();
    let reference_rows: Vec<usize> = (0..band_len).filter(|r| layout.sample_kind(band_rows.start + r) == Irs2Sample::Reference).collect();
    (0..band_len.div_ceil(block_rows))
        .map(|b| {
            let set: Vec<usize> = match window_rows {
                None => reference_rows.clone(),
                Some(w) => {
                    let centre = (b * block_rows + block_rows / 2) as i64;
                    let half = (w / 2) as i64;
                    reference_rows.iter().copied().filter(|r| (*r as i64) >= centre - half && (*r as i64) < centre + half).collect()
                }
            };
            if set.is_empty() {
                bail!(
                    "no IRS2 reference rows inside the {} around block {b} of the band at DMS rows {}..{}",
                    window_rows.map_or("whole band".to_string(), |w| format!("{w}-row window")),
                    band_rows.start,
                    band_rows.end
                );
            }
            Ok(set)
        })
        .collect()
}

fn check_band_shape(band: &[Vec<f32>], width: usize, band_rows: &Range<usize>) -> Result<()> {
    let expected = band_rows.len() * width;
    if let Some(g) = band.iter().position(|plane| plane.len() != expected) {
        bail!("group {g} of the band holds {} samples, expected {} rows x {width} columns", band[g].len(), band_rows.len());
    }
    Ok(())
}

pub fn reference_offsets(
    band: &[Vec<f32>],
    width: usize,
    band_rows: Range<usize>,
    layout: &Irs2Layout,
    window_rows: Option<usize>,
) -> Result<ReferenceOffsets> {
    check_band_shape(band, width, &band_rows)?;
    let sets = reference_sample_sets(band_rows, layout, window_rows)?;
    let ngroups = band.len();
    let mut values = vec![0f32; sets.len() * ngroups * width];
    values.par_chunks_mut(ngroups * width).zip(sets.par_iter()).for_each(|(chunk, set)| {
        let mut samples = vec![0f32; set.len()];
        for (g, plane) in band.iter().enumerate() {
            for x in 0..width {
                for (k, r) in set.iter().enumerate() {
                    samples[k] = plane[r * width + x];
                }
                chunk[g * width + x] = median_f32_mut(&mut samples);
            }
        }
    });
    Ok(ReferenceOffsets { block_rows: layout.block_rows(), ngroups, width, values })
}

pub fn band_scale(
    band: &[Vec<f32>],
    width: usize,
    band_rows: Range<usize>,
    science_rows: &[usize],
    offsets: &ReferenceOffsets,
    params: &QuickSlopeParams,
) -> f32 {
    band_scale_with_group_dq(band, None, width, band_rows, science_rows, offsets, params)
}

pub fn band_scale_with_group_dq(
    band: &[Vec<f32>],
    group_dq: Option<&[Vec<f32>]>,
    width: usize,
    band_rows: Range<usize>,
    science_rows: &[usize],
    offsets: &ReferenceOffsets,
    params: &QuickSlopeParams,
) -> f32 {
    if check_band_shape(band, width, &band_rows).is_err() || group_dq.is_some_and(|dq| check_band_shape(dq, width, &band_rows).is_err()) {
        return 0.0;
    }
    let ngroups = band.len();
    let mut raw = vec![0f32; ngroups];
    let mut corrected = vec![0f32; ngroups];
    let mut flags = vec![0u32; ngroups];
    let mut steps: Vec<f32> = Vec::with_capacity(ngroups);
    let mut step_medians: Vec<f32> = Vec::new();
    let mut pool: Vec<f32> = Vec::new();
    for r in science_rows.iter().copied().step_by(BAND_SCALE_ROW_STEP) {
        let offset_rows: Vec<&[f32]> = (0..ngroups).map(|g| offsets.row(r, g)).collect();
        for x in (0..width).step_by(BAND_SCALE_COL_STEP) {
            for g in 0..ngroups {
                raw[g] = band[g][r * width + x];
                corrected[g] = raw[g] - offset_rows[g][x];
                if let Some(dq) = group_dq {
                    flags[g] = dq[g][r * width + x] as u32;
                }
            }
            let span = usable_span(&raw, &corrected, params.sat_dn, group_dq.map(|_| flags.as_slice()));
            if span.end - span.start < 3 {
                continue;
            }
            let values = &corrected[span.start..span.end];
            pool.extend(values.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).abs()));
            steps.clear();
            steps.extend(values.windows(2).map(|w| w[1] - w[0]));
            step_medians.push(median_f32_mut(&mut steps));
        }
    }
    if pool.is_empty() {
        return 0.0;
    }
    let second_difference_sigma = (MAD_TO_SIGMA_F32 * median_f32_mut(&mut pool)) as f64;
    let mut variance = second_difference_sigma * second_difference_sigma;
    if let Some(gain) = params.gain_e_per_dn {
        let typical_step = median_f32_mut(&mut step_medians).max(0.0) as f64;
        variance = (variance - 2.0 * typical_step / gain as f64).max(0.0);
    }
    (variance / SECOND_DIFFERENCE_VARIANCE_RATIO).sqrt() as f32
}

pub struct PreparedBand {
    pub groups: Vec<Vec<f32>>,
    pub group_dq: Option<Vec<Vec<f32>>>,
    pub science_rows: Vec<usize>,
    pub offsets: ReferenceOffsets,
    pub scale: f32,
}

impl PreparedBand {
    pub fn group_dq_at(&self, band_row: usize, x: usize, width: usize) -> Option<Vec<u32>> {
        self.group_dq.as_ref().map(|planes| planes.iter().map(|plane| plane[band_row * width + x] as u32).collect())
    }
}

pub fn prepare_band(
    cube: &LazyCube,
    ramp_dq: Option<&RampDq>,
    info: &RampInfo,
    plan: &SlopePlan,
    params: &QuickSlopeParams,
    integration: usize,
    band: &Range<usize>,
) -> Result<PreparedBand> {
    let width = cube.geometry.naxis1;
    let z0 = integration * info.ngroups;
    let groups = cube.decode_row_band_batch(z0, z0 + info.ngroups, band.start, band.len())?;
    let group_dq = ramp_dq.map(|dq| dq.decode_groups(z0, z0 + info.ngroups, band.start, band.len())).transpose()?;
    let science_rows = science_rows_of_band(plan.layout.as_ref(), band);
    let offsets = match (&plan.layout, plan.apply_correction) {
        (Some(layout), true) => reference_offsets(&groups, width, band.clone(), layout, params.ref_window_rows)?,
        _ => ReferenceOffsets::zero(info.ngroups, width),
    };
    let scale = band_scale_with_group_dq(&groups, group_dq.as_deref(), width, band.clone(), &science_rows, &offsets, params);
    Ok(PreparedBand { groups, group_dq, science_rows, offsets, scale })
}

#[derive(Debug, Clone)]
pub struct SlopePlan {
    pub layout: Option<Irs2Layout>,
    pub apply_correction: bool,
    pub warnings: Vec<String>,
}

pub fn plan_slope(info: &RampInfo, params: &QuickSlopeParams) -> Result<SlopePlan> {
    let mut warnings = Vec::new();
    let resolution = Irs2Layout::from_info(info)?;
    let already_stripped = resolution == Irs2Resolution::AlreadyStripped;
    let layout = match resolution {
        Irs2Resolution::Layout(layout) => Some(layout),
        Irs2Resolution::NoIrs2Cards => None,
        Irs2Resolution::AlreadyStripped => {
            warnings.push(IRS2_ALREADY_STRIPPED_NOTE.to_string());
            None
        }
    };
    if params.ref_correction == RefCorrection::Amplifier && layout.is_none() {
        bail!("{ERR_AMPLIFIER_NEEDS_LAYOUT}");
    }
    let apply_correction = layout.is_some() && params.ref_correction != RefCorrection::Off;
    match &layout {
        Some(_) if !apply_correction => warnings.push(WARN_REF_CORRECTION_OFF.to_string()),
        Some(_) => {}
        None if already_stripped => {}
        None => warnings.push(WARN_NO_LAYOUT.to_string()),
    }
    Ok(SlopePlan { layout, apply_correction, warnings })
}

pub fn slope_bands(layout: Option<&Irs2Layout>, height: usize) -> Vec<Range<usize>> {
    match layout {
        Some(layout) => (0..layout.science_outputs()).map(|amp| layout.amplifier_rows(amp)).collect(),
        None => (0..height)
            .step_by(NO_LAYOUT_BAND_ROWS)
            .map(|start| start..(start + NO_LAYOUT_BAND_ROWS).min(height))
            .collect(),
    }
}

pub fn science_rows_of_band(layout: Option<&Irs2Layout>, band: &Range<usize>) -> Vec<usize> {
    match layout {
        Some(layout) => (0..band.len()).filter(|r| layout.sample_kind(band.start + r) == Irs2Sample::Science).collect(),
        None => (0..band.len()).collect(),
    }
}

fn count_flags(dq: &Array2<u32>) -> FlagCounts {
    let mut counts = FlagCounts::default();
    for bits in dq.iter() {
        counts.saturated += (bits & DQ_SATURATED != 0) as usize;
        counts.jump_det += (bits & DQ_JUMP_DET != 0) as usize;
        counts.do_not_use += (bits & DQ_DO_NOT_USE != 0) as usize;
    }
    counts
}

struct BandOutputs<'a> {
    sci: ArrayViewMut2<'a, f32>,
    ngood: ArrayViewMut2<'a, i32>,
    dq: ArrayViewMut2<'a, u32>,
    noise: ArrayViewMut2<'a, f32>,
}

fn fit_band(
    prepared: &PreparedBand,
    width: usize,
    band_start: usize,
    ramp_dq: Option<&RampDq>,
    tgroup_s: f64,
    params: &QuickSlopeParams,
    mut out: BandOutputs,
) {
    let rows = Array1::from_vec(prepared.science_rows.clone());
    let ngroups = prepared.groups.len();
    Zip::from(&rows)
        .and(out.sci.axis_iter_mut(Axis(0)))
        .and(out.ngood.axis_iter_mut(Axis(0)))
        .and(out.dq.axis_iter_mut(Axis(0)))
        .and(out.noise.axis_iter_mut(Axis(0)))
        .par_for_each(|&r, mut sci_row, mut ngood_row, mut dq_row, mut noise_row| {
            let offset_rows: Vec<&[f32]> = (0..ngroups).map(|g| prepared.offsets.row(r, g)).collect();
            let mut raw = vec![0f32; ngroups];
            let mut corrected = vec![0f32; ngroups];
            let mut flags = vec![0u32; ngroups];
            let mut diffs = Vec::with_capacity(2 * ngroups);
            let mut flagged = Vec::with_capacity(ngroups);
            let row_start = r * width;
            for x in 0..width {
                for g in 0..ngroups {
                    let value = prepared.groups[g][row_start + x];
                    raw[g] = value;
                    corrected[g] = value - offset_rows[g][x];
                    if let Some(dq) = &prepared.group_dq {
                        flags[g] = dq[g][row_start + x] as u32;
                    }
                }
                let group_dq = prepared.group_dq.as_ref().map(|_| flags.as_slice());
                let fit = fit_pixel_with_group_dq(&raw, &corrected, group_dq, tgroup_s, prepared.scale, params, &mut diffs, &mut flagged);
                sci_row[x] = fit.slope;
                ngood_row[x] = fit.ngood as i32;
                dq_row[x] = fit.dq | ramp_dq.map_or(0, |dq| dq.pixel_bits(band_start + r, x));
                noise_row[x] = fit.noise;
            }
        });
}

#[derive(Debug, Clone)]
pub struct QuickSlopeProduct {
    pub sci: Array2<f32>,
    pub ngood: Array2<i32>,
    pub dq: Array2<u32>,
    pub noise: Array2<f32>,
    pub counts: FlagCounts,
    pub layout: Option<Irs2Layout>,
    pub ref_corrected: bool,
    pub band_scales_dn: Vec<f32>,
    pub tgroup_s: f64,
    pub integration: usize,
    pub warnings: Vec<String>,
    pub noise_model: NoiseModel,
    pub dq_source: DqSource,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FlagCounts {
    pub saturated: usize,
    pub jump_det: usize,
    pub do_not_use: usize,
}

pub fn quick_slope(
    cube: &LazyCube,
    info: &RampInfo,
    integration: usize,
    params: &QuickSlopeParams,
    on_band_done: &(dyn Fn(usize, usize) + Sync),
    cancelled: CancelCheck,
) -> Result<QuickSlopeProduct> {
    let tgroup_s = info.tgroup_s.context(ERR_TGROUP_MISSING)?;
    if integration >= info.nints {
        bail!("integration {integration} out of range (nints={})", info.nints);
    }
    params.validate()?;
    let plan = plan_slope(info, params)?;
    let width = cube.geometry.naxis1;
    let height = cube.geometry.naxis2;
    if let Some(layout) = &plan.layout {
        if layout.n_fast != height {
            bail!("IRS2 layout spans {} rows but the cube frame has {height}", layout.n_fast);
        }
    }
    let ramp_dq = RampDq::open(cube, info)?;
    let mut warnings = plan.warnings.clone();
    if let Some(dq) = &ramp_dq {
        warnings.push(dq.note().to_string());
    }
    let out_rows = plan.layout.as_ref().map_or(height, Irs2Layout::science_rows);
    let mut sci = Array2::from_elem((out_rows, width), f32::NAN);
    let mut noise = Array2::from_elem((out_rows, width), f32::NAN);
    let mut ngood = Array2::<i32>::zeros((out_rows, width));
    let mut dq = Array2::<u32>::zeros((out_rows, width));
    let bands = slope_bands(plan.layout.as_ref(), height);
    let mut band_scales_dn = Vec::with_capacity(bands.len());
    for (band_index, band) in bands.iter().enumerate() {
        stop_if_cancelled(cancelled)?;
        let prepared = prepare_band(cube, ramp_dq.as_ref(), info, &plan, params, integration, band)?;
        band_scales_dn.push(prepared.scale);
        let out_start = match (&plan.layout, prepared.science_rows.first()) {
            (Some(layout), Some(first)) => layout.science_row_of(band.start + first).unwrap_or(0),
            _ => band.start,
        };
        let out_end = out_start + prepared.science_rows.len();
        fit_band(
            &prepared,
            width,
            band.start,
            ramp_dq.as_ref(),
            tgroup_s,
            params,
            BandOutputs {
                sci: sci.slice_mut(s![out_start..out_end, ..]),
                ngood: ngood.slice_mut(s![out_start..out_end, ..]),
                dq: dq.slice_mut(s![out_start..out_end, ..]),
                noise: noise.slice_mut(s![out_start..out_end, ..]),
            },
        );
        on_band_done(band_index, bands.len());
    }
    let counts = count_flags(&dq);
    Ok(QuickSlopeProduct {
        sci,
        ngood,
        dq,
        noise,
        counts,
        layout: plan.layout,
        ref_corrected: plan.apply_correction,
        band_scales_dn,
        tgroup_s,
        integration,
        warnings,
        noise_model: params.noise_model(),
        dq_source: if ramp_dq.is_some() { DqSource::Groupdq } else { DqSource::Threshold },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cube::lazy::LazyCube;
    use crate::core::imaging::dq_flags::JWST_FLAGS;
    use crate::core::ramp::irs2::{Irs2Resolution, Irs2Sample, IRS2_ALREADY_STRIPPED_NOTE};
    use crate::core::ramp::test_support::{
        deterministic_noise, drift_slope_dn_per_s, irs2_synthetic_value, irs2_truth_slope, owner_like_irs2, write_synthetic_uncal,
        NoiseRamps, SyntheticUncal, SYNTHETIC_DRIFT_DN, SYNTHETIC_PEDESTAL_DN, SYNTHETIC_TGROUP_S,
    };
    use crate::core::stacking::is_cancellation;

    const TGROUP: f64 = 10.0;

    fn fit_raw(raw: &[f32], band_scale_dn: f32) -> (PixelFit, Vec<usize>) {
        fit_with(raw, raw, band_scale_dn, &QuickSlopeParams::default())
    }

    fn fit_with(raw: &[f32], corrected: &[f32], band_scale_dn: f32, params: &QuickSlopeParams) -> (PixelFit, Vec<usize>) {
        let mut diffs = Vec::new();
        let mut flagged = Vec::new();
        let fit = fit_pixel(raw, corrected, TGROUP, band_scale_dn, params, &mut diffs, &mut flagged);
        (fit, flagged)
    }

    fn linear(n: usize, step: f32) -> Vec<f32> {
        (0..n).map(|g| 1000.0 + step * g as f32).collect()
    }

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    fn noise_band(rows: usize, width: usize, ngroups: usize, sigma: f32, step: f32, seed: u64) -> Vec<Vec<f32>> {
        (0..ngroups)
            .map(|g| {
                (0..rows * width)
                    .map(|p| 1000.0 + step * g as f32 + sigma * deterministic_noise(seed * 64 + g as u64, p))
                    .collect()
            })
            .collect()
    }

    fn fit_every_pixel(band: &[Vec<f32>], width: usize, rows: usize, band_scale_dn: f32) -> Vec<PixelFit> {
        let params = QuickSlopeParams::default();
        let ngroups = band.len();
        let mut raw = vec![0.0; ngroups];
        let mut diffs = Vec::new();
        let mut flagged = Vec::new();
        let mut fits = Vec::with_capacity(rows * width);
        for p in 0..rows * width {
            for g in 0..ngroups {
                raw[g] = band[g][p];
            }
            fits.push(fit_pixel(&raw, &raw, TGROUP, band_scale_dn, &params, &mut diffs, &mut flagged));
        }
        fits
    }

    fn jump_fraction(fits: &[PixelFit]) -> f64 {
        fits.iter().filter(|f| f.dq & DQ_JUMP_DET != 0).count() as f64 / fits.len() as f64
    }

    fn owner_band_layout() -> Irs2Layout {
        Irs2Layout::new(3200, 16, 4, 5, false).unwrap()
    }

    fn synthetic_band(layout: &Irs2Layout, amp: usize, width: usize, ngroups: usize, value: impl Fn(Irs2Sample, usize, usize, usize) -> f32) -> (Vec<Vec<f32>>, Range<usize>) {
        let rows = layout.amplifier_rows(amp);
        let band = (0..ngroups)
            .map(|g| {
                let mut plane = vec![0.0; rows.len() * width];
                for (r, y) in rows.clone().enumerate() {
                    for x in 0..width {
                        plane[r * width + x] = value(layout.sample_kind(y), g, y, x);
                    }
                }
                plane
            })
            .collect();
        (band, rows)
    }

    fn open_synthetic(dir: &tempfile::TempDir, name: &str, spec: &SyntheticUncal, value: impl Fn(usize, usize, usize, usize) -> f32) -> (LazyCube, RampInfo) {
        let path = write_synthetic_uncal(&dir.path().join(name), spec, value);
        let cube = LazyCube::open(&path).unwrap();
        let info = cube.ramp().expect("synthetic uncal is a ramp");
        (cube, info)
    }

    fn run(cube: &LazyCube, info: &RampInfo, params: &QuickSlopeParams) -> Result<QuickSlopeProduct> {
        quick_slope(cube, info, 0, params, &|_, _| {}, &|| false)
    }

    fn irs2_spec(cols: usize, reversed: bool) -> SyntheticUncal {
        let mut spec = owner_like_irs2(cols, reversed);
        spec.tgroup_s = SYNTHETIC_TGROUP_S;
        spec
    }

    #[test]
    fn the_quick_slope_dq_bits_are_the_jwst_table_bits() {
        let bit = |name: &str| JWST_FLAGS.iter().find(|f| f.name == name).unwrap().bit;
        assert_eq!(DQ_DO_NOT_USE, bit("DO_NOT_USE"));
        assert_eq!(DQ_SATURATED, bit("SATURATED"));
        assert_eq!(DQ_JUMP_DET, bit("JUMP_DET"));
    }

    #[test]
    fn params_validation_rejects_out_of_range_values() {
        assert!(QuickSlopeParams::default().validate().is_ok());
        let cases: [(&str, QuickSlopeParams); 7] = [
            ("sat_dn zero", QuickSlopeParams { sat_dn: 0.0, ..Default::default() }),
            ("sat_dn above u16", QuickSlopeParams { sat_dn: 70_000.0, ..Default::default() }),
            ("sat_dn nan", QuickSlopeParams { sat_dn: f32::NAN, ..Default::default() }),
            ("jump_k below one", QuickSlopeParams { jump_k: 0.5, ..Default::default() }),
            ("negative floor", QuickSlopeParams { scale_floor_dn: -1.0, ..Default::default() }),
            ("ols with one group", QuickSlopeParams { min_groups_ols: 1, ..Default::default() }),
            ("window too small", QuickSlopeParams { ref_window_rows: Some(30), ..Default::default() }),
        ];
        for (label, params) in cases {
            assert!(params.validate().is_err(), "{label}");
        }
        assert!(QuickSlopeParams { ref_window_rows: None, ..Default::default() }.validate().is_ok());
        assert!(QuickSlopeParams { sat_dn: 65_535.0, ..Default::default() }.validate().is_ok());
    }

    #[test]
    fn a_linear_ramp_fits_its_slope_exactly_with_no_flags() {
        let (fit, flagged) = fit_raw(&linear(10, 50.0), 3.0);
        assert!(close(fit.slope, 5.0, 1e-6), "{}", fit.slope);
        assert_eq!((fit.ngood, fit.dq, fit.n_usable, fit.first_saturated), (10, 0, 10, None));
        let expected = 3.0 / 2f32.sqrt() * (12.0f32 / 990.0).sqrt() / 10.0;
        assert!(close(fit.noise, expected, 1e-5), "noise {} expected {expected}", fit.noise);
        assert!(close(fit.noise, 0.023355, 1e-5));
        assert!(flagged.is_empty());
        let (noiseless, _) = fit_raw(&linear(10, 50.0), 0.0);
        assert_eq!(noiseless.noise, 0.0, "a band with no read noise and no gain has no read noise term");
    }

    #[test]
    fn saturation_at_group_six_truncates_the_fit_and_sets_only_the_saturated_bit() {
        let mut raw = linear(10, 50.0);
        for v in raw.iter_mut().skip(6) {
            *v = 65_000.0;
        }
        let (fit, _) = fit_raw(&raw, 3.0);
        assert_eq!((fit.n_usable, fit.first_saturated, fit.dq, fit.ngood), (6, Some(6), DQ_SATURATED, 6));
        assert!(close(fit.slope, 5.0, 1e-6));
        let six = 3.0 / 2f32.sqrt() * (12.0f32 / (6.0 * 35.0)).sqrt() / 10.0;
        assert!(close(fit.noise, six, 1e-5));
    }

    #[test]
    fn a_group_exactly_at_the_saturation_threshold_is_saturated_and_one_dn_below_is_not() {
        let at_threshold: Vec<f32> = (0..10).map(|g| DEFAULT_SAT_DN - 50.0 * (9 - g) as f32).collect();
        let (fit, _) = fit_raw(&at_threshold, 0.0);
        assert_eq!((fit.n_usable, fit.first_saturated, fit.dq, fit.ngood), (9, Some(9), DQ_SATURATED, 9));
        assert!(close(fit.slope, 5.0, 1e-6), "{}", fit.slope);
        let below: Vec<f32> = at_threshold.iter().map(|v| v - 1.0).collect();
        let (fit, _) = fit_raw(&below, 0.0);
        assert_eq!((fit.n_usable, fit.first_saturated, fit.dq, fit.ngood), (10, None, 0, 10));
        assert!(close(fit.slope, 5.0, 1e-6), "{}", fit.slope);
    }

    #[test]
    fn a_difference_exactly_at_the_jump_threshold_is_not_flagged_and_one_above_is() {
        let params = QuickSlopeParams { scale_floor_dn: 3.0, jump_k: 5.0, ..Default::default() };
        let mut ramp: Vec<f32> = vec![100.0; 10];
        ramp[9] = ramp[8] + 15.0;
        let (fit, flagged) = fit_with(&ramp, &ramp, 0.0, &params);
        assert!(flagged.is_empty(), "{flagged:?}");
        assert_eq!((fit.dq, fit.ngood, fit.n_usable), (0, 10, 10));
        ramp[9] = ramp[8] + 15.5;
        let (fit, flagged) = fit_with(&ramp, &ramp, 0.0, &params);
        assert_eq!(flagged, vec![8]);
        assert_eq!((fit.dq, fit.ngood, fit.n_usable), (DQ_JUMP_DET, 9, 10));
        assert!(close(fit.slope, 0.0, 1e-6), "{}", fit.slope);

        let mut lit: Vec<f32> = (0..10).map(|g| 10.0 * g as f32).collect();
        lit[9] = lit[8] + 25.5;
        let (fit, flagged) = fit_with(&lit, &lit, 0.0, &params);
        assert!(flagged.is_empty(), "10 DN per group at 1 e-/DN has a photon sigma of sqrt(10) DN, above the 3 DN floor: {flagged:?}");
        assert_eq!(fit.dq, 0);
        lit[9] = lit[8] + 26.0;
        let (fit, flagged) = fit_with(&lit, &lit, 0.0, &params);
        assert_eq!(flagged, vec![8], "16 DN from the median is above 5 x sqrt(10) = 15.81 DN");
        assert!(close(fit.slope, 1.0, 1e-6), "{}", fit.slope);
        let with_gain = QuickSlopeParams { gain_e_per_dn: Some(4.0), ..params.clone() };
        lit[9] = lit[8] + 25.5;
        let (_, flagged) = fit_with(&lit, &lit, 0.0, &with_gain);
        assert_eq!(flagged, vec![8], "at 4 e-/DN the photon sigma is sqrt(10 / 4) DN and the 3 DN floor decides again");
    }

    #[test]
    fn saturation_at_group_zero_is_do_not_use_with_nan_outputs() {
        let (fit, _) = fit_raw(&[65_000.0; 10], 0.0);
        assert_eq!((fit.dq, fit.ngood, fit.n_usable, fit.first_saturated), (DQ_DO_NOT_USE | DQ_SATURATED, 0, 0, Some(0)));
        assert!(fit.slope.is_nan() && fit.noise.is_nan());
    }

    #[test]
    fn one_usable_group_is_do_not_use() {
        let mut raw = linear(10, 50.0);
        for v in raw.iter_mut().skip(1) {
            *v = 65_000.0;
        }
        let (fit, _) = fit_raw(&raw, 0.0);
        assert_eq!((fit.dq, fit.ngood, fit.n_usable, fit.first_saturated), (DQ_DO_NOT_USE | DQ_SATURATED, 1, 1, Some(1)));
        assert!(fit.slope.is_nan() && fit.noise.is_nan());
    }

    #[test]
    fn two_groups_use_the_single_difference_and_no_noise() {
        let (fit, _) = fit_raw(&[1000.0, 1070.0], 0.0);
        assert_eq!((fit.dq, fit.ngood, fit.n_usable, fit.first_saturated), (0, 2, 2, None));
        assert!(close(fit.slope, 7.0, 1e-6));
        assert!(fit.noise.is_nan());
    }

    #[test]
    fn a_single_jump_is_flagged_and_the_slope_comes_from_the_unflagged_differences() {
        let mut raw = linear(10, 50.0);
        for v in raw.iter_mut().skip(5) {
            *v += 500.0;
        }
        let (fit, flagged) = fit_raw(&raw, 3.0);
        assert_eq!((fit.dq, fit.ngood, fit.n_usable), (DQ_JUMP_DET, 9, 10));
        assert!(close(fit.slope, 5.0, 1e-6), "{}", fit.slope);
        assert_eq!(flagged, vec![4]);
        let single_difference_variance = 9.0f64;
        let even_median_factor = FRAC_PI_2 * 8.0 / (8.0 + PI - 2.0);
        let adjacent_pairs_of_eight_kept = 6.0;
        let correlation_term = 1.0 + 4.0 * adjacent_pairs_of_eight_kept / (PI * 8.0) * (-0.5f64).asin();
        let expected = ((single_difference_variance / 8.0 * even_median_factor * correlation_term).sqrt() / 10.0) as f32;
        assert!(close(expected, 0.08793, 1e-4), "{expected}");
        assert!(close(fit.noise, expected, 1e-5), "noise {} expected {expected}", fit.noise);
    }

    #[test]
    fn three_groups_skip_the_jump_test_and_use_the_median_difference() {
        let (fit, flagged) = fit_raw(&[1000.0, 1050.0, 1300.0], 4.0);
        assert_eq!((fit.dq, fit.ngood, fit.n_usable), (0, 3, 3));
        assert!(flagged.is_empty());
        assert!(close(fit.slope, 150.0 / 10.0, 1e-5), "{}", fit.slope);
        let read_sigma = 4.0 / 2f32.sqrt();
        let half_span_slope_error = read_sigma * 2f32.sqrt() / 2.0 / 10.0;
        assert!(close(fit.noise, half_span_slope_error, 1e-5), "the median of two differences is (last - first) / 2: {}", fit.noise);
        assert!(close(fit.noise, 0.2, 1e-5));
    }

    #[test]
    fn the_noise_floor_keeps_quantisation_steps_from_being_flagged() {
        let mut raw = vec![1000.0; 10];
        for v in raw.iter_mut().skip(4) {
            *v += 1.0;
        }
        let (fit, flagged) = fit_raw(&raw, 0.0);
        assert_eq!(fit.dq, 0);
        assert!(flagged.is_empty());
        let ols_of_unit_step = 120.0 / 8250.0;
        assert!(close(fit.slope, ols_of_unit_step, 1e-6), "{}", fit.slope);
    }

    #[test]
    fn the_band_scale_floors_the_pixel_scale() {
        let mut sixty = vec![1000.0; 10];
        for v in sixty.iter_mut().skip(5) {
            *v += 60.0;
        }
        let (fit, _) = fit_raw(&sixty, 15.0);
        assert_eq!(fit.dq, 0, "a 60 DN step is 4 band scales: not a jump");
        let (fit_without_band, _) = fit_raw(&sixty, 0.0);
        assert_eq!(fit_without_band.dq, DQ_JUMP_DET, "without the band floor the 3 DN floor flags it");
        let mut hundred = vec![1000.0; 10];
        for v in hundred.iter_mut().skip(5) {
            *v += 100.0;
        }
        let (fit, flagged) = fit_raw(&hundred, 15.0);
        assert_eq!((fit.dq, flagged.as_slice()), (DQ_JUMP_DET, &[4][..]));
    }

    #[test]
    fn white_noise_ramps_are_rarely_flagged_with_the_band_floor() {
        let (rows, width) = (100, 100);
        let band = noise_band(rows, width, 10, 8.0, 50.0, 1);
        let all_rows: Vec<usize> = (0..rows).collect();
        let scale = band_scale(&band, width, 0..rows, &all_rows, &ReferenceOffsets::zero(10, width), &QuickSlopeParams::default());
        let with_floor = jump_fraction(&fit_every_pixel(&band, width, rows, scale));
        let without_floor = jump_fraction(&fit_every_pixel(&band, width, rows, 0.0));
        println!("white noise sigma 8: band_scale_dn={scale:.3} false JUMP_DET with band floor={:.3}% pixel MAD only={:.3}%", with_floor * 100.0, without_floor * 100.0);
        assert!(scale > 8.0 && scale < 16.0, "band scale {scale} should sit near sigma*sqrt(2) = 11.3 DN");
        assert!(with_floor < 0.005, "false flag rate {with_floor}");
    }

    #[test]
    fn a_hundred_dn_jump_at_sigma_eight_is_flagged() {
        let (rows, width) = (100, 100);
        let mut band = noise_band(rows, width, 10, 8.0, 50.0, 2);
        for plane in band.iter_mut().skip(5) {
            for v in plane.iter_mut().take(1000) {
                *v += 100.0;
            }
        }
        let all_rows: Vec<usize> = (0..rows).collect();
        let scale = band_scale(&band, width, 0..rows, &all_rows, &ReferenceOffsets::zero(10, width), &QuickSlopeParams::default());
        let fits = fit_every_pixel(&band, width, rows, scale);
        let detected = jump_fraction(&fits[..1000]);
        let on_clean = jump_fraction(&fits[1000..]);
        println!("planted +100 DN jump at sigma 8: band_scale_dn={scale:.3} detected={:.1}% clean flagged={:.3}%", detected * 100.0, on_clean * 100.0);
        assert!(detected >= 0.8, "detected {detected}");
        let flagged_index_ok = fits[..1000].iter().filter(|f| f.dq & DQ_JUMP_DET != 0).all(|f| f.ngood <= 9);
        assert!(flagged_index_ok);
    }

    #[test]
    fn the_noise_plane_tracks_the_empirical_slope_scatter() {
        let (rows, width) = (100, 100);
        let band = noise_band(rows, width, 10, 8.0, 0.0, 3);
        let all_rows: Vec<usize> = (0..rows).collect();
        let scale = band_scale(&band, width, 0..rows, &all_rows, &ReferenceOffsets::zero(10, width), &QuickSlopeParams::default());
        let fits = fit_every_pixel(&band, width, rows, scale);
        let clean: Vec<&PixelFit> = fits.iter().filter(|f| f.dq == 0).collect();
        let n = clean.len() as f64;
        let mean = clean.iter().map(|f| f.slope as f64).sum::<f64>() / n;
        let std = (clean.iter().map(|f| (f.slope as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
        let mut noises: Vec<f32> = clean.iter().map(|f| f.noise).collect();
        let median_noise = crate::math::median::median_f32_mut(&mut noises) as f64;
        let closed_form = 8.0 * (12.0f64 / 990.0).sqrt() / TGROUP;
        println!("white noise sigma 8: std(slope)={std:.5} median(noise)={median_noise:.5} closed form={closed_form:.5} band_scale_dn={scale:.3}");
        assert!((median_noise - std).abs() / std < 0.15, "median noise {median_noise} vs scatter {std}");
    }

    #[test]
    fn a_non_finite_group_truncates_the_ramp_like_saturation_without_the_saturated_bit() {
        let raw = linear(10, 50.0);
        let mut corrected = raw.clone();
        corrected[6] = f32::NAN;
        let (fit, _) = fit_with(&raw, &corrected, 0.0, &QuickSlopeParams::default());
        assert_eq!((fit.n_usable, fit.first_saturated, fit.dq, fit.ngood), (6, None, 0, 6));
        assert!(close(fit.slope, 5.0, 1e-6));
    }

    #[test]
    fn jump_detection_is_only_tried_with_at_least_three_differences() {
        let (three, flagged) = fit_raw(&[1000.0, 1000.0, 2000.0], 0.0);
        assert_eq!(three.dq, 0);
        assert!(flagged.is_empty());
        let (four, flagged) = fit_raw(&[1000.0, 1000.0, 1000.0, 2000.0], 0.0);
        assert_eq!((four.dq, flagged.as_slice()), (DQ_JUMP_DET, &[2][..]));
        assert_eq!(four.ngood, 3);
    }

    #[test]
    fn reference_offsets_are_the_per_block_per_column_medians_of_the_windowed_reference_rows() {
        let layout = owner_band_layout();
        let width = 6;
        let (band, rows) = synthetic_band(&layout, 0, width, 10, |kind, g, _y, x| match kind {
            Irs2Sample::Reference => (g * 10 + x) as f32,
            _ => 5000.0 + x as f32,
        });
        for window in [Some(200), None] {
            let offsets = reference_offsets(&band, width, rows.clone(), &layout, window).unwrap();
            assert_eq!((offsets.block_rows, offsets.ngroups, offsets.width, offsets.blocks()), (20, 10, width, 32));
            for r in 0..rows.len() {
                for g in 0..10 {
                    for x in 0..width {
                        assert_eq!(offsets.get(r, g, x), (g * 10 + x) as f32, "window {window:?} row {r} group {g} column {x}");
                    }
                }
            }
        }
    }

    #[test]
    fn one_outlier_reference_row_inside_the_window_does_not_move_the_offset() {
        let layout = owner_band_layout();
        let width = 2;
        let outlier_row = 848;
        assert_eq!(layout.sample_kind(outlier_row), Irs2Sample::Reference);
        let (band, rows) = synthetic_band(&layout, 0, width, 4, |kind, _g, y, _x| match kind {
            Irs2Sample::Reference if y == outlier_row => 10_100.0,
            Irs2Sample::Reference => 100.0,
            _ => 5000.0,
        });
        let outlier_band_row = outlier_row - rows.start;
        let sets = reference_sample_sets(rows.clone(), &layout, Some(200)).unwrap();
        let blocks_seeing_it: Vec<usize> = (0..sets.len()).filter(|b| sets[*b].contains(&outlier_band_row)).collect();
        assert_eq!(blocks_seeing_it, (5..=14).collect::<Vec<_>>());
        for window in [Some(200), None] {
            let offsets = reference_offsets(&band, width, rows.clone(), &layout, window).unwrap();
            for b in 0..32 {
                for g in 0..4 {
                    for x in 0..width {
                        assert_eq!(offsets.get(b * 20, g, x), 100.0, "window {window:?} block {b} group {g} column {x}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_windowed_offset_tracks_a_reference_ramp_along_y_and_the_whole_band_median_does_not() {
        let layout = owner_band_layout();
        let width = 3;
        let (band, rows) = synthetic_band(&layout, 1, width, 4, |kind, _g, y, _x| match kind {
            Irs2Sample::Reference => 0.1 * y as f32,
            _ => 0.0,
        });
        let windowed = reference_offsets(&band, width, rows.clone(), &layout, Some(200)).unwrap();
        let whole = reference_offsets(&band, width, rows.clone(), &layout, None).unwrap();
        let whole_value = whole.get(0, 0, 0);
        let mut previous = f32::NEG_INFINITY;
        for b in 0..32 {
            let band_row = b * 20;
            let centre = (rows.start + band_row + 10) as f32 * 0.1;
            let value = windowed.get(band_row, 0, 0);
            let interior = (5..=26).contains(&b);
            let tolerance = if interior { 2.0 } else { 6.0 };
            assert!((value - centre).abs() <= tolerance, "block {b}: offset {value} centre {centre}");
            assert!(value >= previous, "offsets follow the reference ramp monotonically");
            previous = value;
            assert_eq!(whole.get(band_row, 0, 0), whole_value);
            assert_eq!(whole.get(band_row, 3, 2), whole_value);
        }
        assert!((whole_value - (rows.start as f32 + 320.0) * 0.1).abs() <= 2.0);
    }

    #[test]
    fn each_band_has_thirty_two_blocks_and_edge_blocks_still_see_at_least_twenty_reference_samples() {
        let layout = owner_band_layout();
        for amp in 0..4 {
            let sets = reference_sample_sets(layout.amplifier_rows(amp), &layout, Some(200)).unwrap();
            assert_eq!(sets.len(), 32);
            let sizes: Vec<usize> = sets.iter().map(Vec::len).collect();
            assert!(sizes.iter().all(|n| (20..=40).contains(n)), "{sizes:?}");
            assert!(sizes[8..24].iter().all(|n| (36..=40).contains(n)), "{sizes:?}");
            assert_eq!(sizes[0], 22);
            let whole = reference_sample_sets(layout.amplifier_rows(amp), &layout, None).unwrap();
            assert!(whole.iter().all(|s| s.len() == 128));
        }
    }

    #[test]
    fn an_empty_reference_window_is_an_error_not_zero() {
        let layout = owner_band_layout();
        let science_only = 640..648;
        let band: Vec<Vec<f32>> = (0..4).map(|_| vec![1.0; science_only.len() * 2]).collect();
        let err = reference_offsets(&band, 2, science_only.clone(), &layout, Some(40)).unwrap_err();
        assert!(err.to_string().contains("no IRS2 reference rows"), "{err}");
        assert!(reference_offsets(&band, 2, science_only, &layout, None).is_err());
    }

    #[test]
    fn a_per_group_offset_shared_by_reference_and_science_rows_cancels_in_the_corrected_slope() {
        let layout = owner_band_layout();
        let width = 4;
        let (band, rows) = synthetic_band(&layout, 2, width, 10, |kind, g, _y, x| {
            let drift = SYNTHETIC_DRIFT_DN[g];
            match kind {
                Irs2Sample::Reference => SYNTHETIC_PEDESTAL_DN + drift + x as f32,
                _ => SYNTHETIC_PEDESTAL_DN + drift + x as f32 + 50.0 * g as f32,
            }
        });
        let offsets = reference_offsets(&band, width, rows.clone(), &layout, Some(200)).unwrap();
        let r = (0..rows.len()).find(|r| layout.sample_kind(rows.start + r) == Irs2Sample::Science).unwrap();
        let raw: Vec<f32> = (0..10).map(|g| band[g][r * width + 1]).collect();
        let corrected: Vec<f32> = (0..10).map(|g| raw[g] - offsets.get(r, g, 1)).collect();
        let (corrected_fit, _) = fit_with(&raw, &corrected, 0.0, &QuickSlopeParams::default());
        let (naive_fit, _) = fit_with(&raw, &raw, 0.0, &QuickSlopeParams::default());
        assert!(close(corrected_fit.slope, 5.0, 1e-5), "{}", corrected_fit.slope);
        let drifted = 5.0 + drift_slope_dn_per_s(TGROUP) as f32;
        assert!(close(naive_fit.slope, drifted, 1e-5), "{} vs {drifted}", naive_fit.slope);
        assert!((naive_fit.slope - 5.0).abs() > 1e-3);
    }

    #[test]
    fn quick_slope_strips_the_irs2_layout_and_scatters_rows_through_science_row_of() {
        let dir = tempfile::tempdir().unwrap();
        let layout = owner_band_layout();
        let (cube, info) = open_synthetic(&dir, "nrs1_uncal.fits", &irs2_spec(8, false), irs2_synthetic_value(&layout, SYNTHETIC_TGROUP_S));
        let bands_done = std::sync::Mutex::new(Vec::new());
        let product = quick_slope(&cube, &info, 0, &QuickSlopeParams::default(), &|k, n| bands_done.lock().unwrap().push((k, n)), &|| false).unwrap();
        assert_eq!(product.sci.dim(), (2048, 8));
        assert_eq!(product.dq.dim(), (2048, 8));
        for r in 0..2048 {
            for x in 0..8 {
                let truth = irs2_truth_slope(r, x);
                assert!(close(product.sci[[r, x]], truth, 1e-3), "row {r} col {x}: {} vs {truth}", product.sci[[r, x]]);
                assert_eq!(product.ngood[[r, x]], 10);
            }
        }
        assert_eq!(product.counts, FlagCounts::default());
        assert!(product.ref_corrected);
        assert_eq!(product.layout.as_ref(), Some(&layout));
        assert_eq!(product.band_scales_dn.len(), 4);
        assert!(product.band_scales_dn.iter().all(|s| *s < 1.0), "{:?}", product.band_scales_dn);
        assert_eq!((product.tgroup_s, product.integration), (SYNTHETIC_TGROUP_S, 0));
        assert!(product.warnings.is_empty(), "{:?}", product.warnings);
        assert!(product.noise.iter().all(|n| n.is_finite()));
        assert_eq!(*bands_done.lock().unwrap(), vec![(0, 4), (1, 4), (2, 4), (3, 4)]);
    }

    #[test]
    fn quick_slope_on_an_nrs2_layout_strips_in_reversed_order() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Irs2Layout::new(3200, 16, 4, 5, true).unwrap();
        let (cube, info) = open_synthetic(&dir, "nrs2_uncal.fits", &irs2_spec(8, true), |i, g, y, x| {
            let per_row = 0.1 * x as f32 + 0.1 * (y % 7) as f32;
            match layout.sample_kind(y) {
                Irs2Sample::Science => SYNTHETIC_PEDESTAL_DN + SYNTHETIC_DRIFT_DN[g] + per_row * (SYNTHETIC_TGROUP_S * g as f64) as f32,
                _ => irs2_synthetic_value(&layout, SYNTHETIC_TGROUP_S)(i, g, y, x),
            }
        });
        let product = run(&cube, &info, &QuickSlopeParams::default()).unwrap();
        assert_eq!(product.sci.dim(), (2048, 8));
        for x in 0..8 {
            let dms_row_zero = 0.1 * x as f32;
            assert!(close(product.sci[[0, x]], dms_row_zero, 1e-3), "{}", product.sci[[0, x]]);
            let last_science = 0.1 * x as f32 + 0.1 * (2559 % 7) as f32;
            assert!(close(product.sci[[2047, x]], last_science, 1e-3), "{}", product.sci[[2047, x]]);
        }
        assert_eq!(product.layout.as_ref().map(|l| l.reversed), Some(true));
        assert_eq!(product.counts, FlagCounts::default());
    }

    #[test]
    fn ref_correction_off_still_strips_the_irs2_layout() {
        let dir = tempfile::tempdir().unwrap();
        let layout = owner_band_layout();
        let (cube, info) = open_synthetic(&dir, "nrs1_uncal.fits", &irs2_spec(8, false), irs2_synthetic_value(&layout, SYNTHETIC_TGROUP_S));
        let params = QuickSlopeParams { ref_correction: RefCorrection::Off, ..Default::default() };
        let product = run(&cube, &info, &params).unwrap();
        assert_eq!(product.sci.dim(), (2048, 8));
        assert!(!product.ref_corrected);
        assert!(product.layout.is_some());
        assert!(product.warnings.iter().any(|w| w.contains("ref_correction=off")), "{:?}", product.warnings);
        let drifted = drift_slope_dn_per_s(SYNTHETIC_TGROUP_S) as f32;
        assert!(drifted.abs() > 1e-3);
        for r in [0usize, 700, 2047] {
            for x in 0..8 {
                let expected = irs2_truth_slope(r, x) + drifted;
                assert!(close(product.sci[[r, x]], expected, 1e-3), "row {r} col {x}: {} vs {expected}", product.sci[[r, x]]);
            }
        }
    }

    #[test]
    fn quick_slope_without_irs2_cards_keeps_the_frame_size_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let spec = SyntheticUncal { cols: 8, rows: 64, ngroups: 10, nints: 1, irs2: None, tgroup_s: TGROUP, detector: "NRS1", write_fastaxis: false };
        let (cube, info) = open_synthetic(&dir, "plain_uncal.fits", &spec, |_, g, y, x| 1000.0 + (x as f32 + 0.5 * y as f32) * 10.0 * g as f32);
        let product = run(&cube, &info, &QuickSlopeParams::default()).unwrap();
        assert_eq!(product.sci.dim(), (64, 8));
        assert!(product.layout.is_none() && !product.ref_corrected);
        assert_eq!(product.warnings, vec![WARN_NO_LAYOUT.to_string()]);
        assert_eq!(product.band_scales_dn.len(), 1);
        for y in 0..64 {
            for x in 0..8 {
                assert!(close(product.sci[[y, x]], x as f32 + 0.5 * y as f32, 1e-3));
            }
        }
        let amplifier = QuickSlopeParams { ref_correction: RefCorrection::Amplifier, ..Default::default() };
        assert!(run(&cube, &info, &amplifier).unwrap_err().to_string().contains("ref_correction=amplifier"));
    }

    #[test]
    fn quick_slope_on_an_already_stripped_frame_keeps_the_size_and_notes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = irs2_spec(4, false);
        spec.rows = 2048;
        let (cube, info) = open_synthetic(&dir, "stripped_uncal.fits", &spec, |_, g, y, x| 1000.0 + (x as f32 + (y % 5) as f32) * 10.0 * g as f32);
        assert_eq!(Irs2Layout::from_info(&info).unwrap(), Irs2Resolution::AlreadyStripped);
        let product = run(&cube, &info, &QuickSlopeParams::default()).unwrap();
        assert_eq!(product.sci.dim(), (2048, 4));
        assert!(product.layout.is_none() && !product.ref_corrected);
        assert_eq!(product.warnings, vec![IRS2_ALREADY_STRIPPED_NOTE.to_string()]);
        assert_eq!(product.band_scales_dn.len(), 8);
        let amplifier = QuickSlopeParams { ref_correction: RefCorrection::Amplifier, ..Default::default() };
        assert!(run(&cube, &info, &amplifier).is_err());
    }

    #[test]
    fn quick_slope_refuses_a_ramp_without_group_time() {
        let dir = tempfile::tempdir().unwrap();
        let spec = SyntheticUncal { cols: 4, rows: 8, ngroups: 5, nints: 1, irs2: None, tgroup_s: TGROUP, detector: "NRS1", write_fastaxis: false };
        let (cube, info) = open_synthetic(&dir, "plain_uncal.fits", &spec, |_, g, _, _| 1000.0 + 10.0 * g as f32);
        let mut without_time = info.clone();
        without_time.tgroup_s = None;
        let err = run(&cube, &without_time, &QuickSlopeParams::default()).unwrap_err();
        assert!(err.to_string().contains("TGROUP"), "{err}");
    }

    #[test]
    fn quick_slope_refuses_an_integration_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let spec = SyntheticUncal { cols: 4, rows: 8, ngroups: 5, nints: 2, irs2: None, tgroup_s: TGROUP, detector: "NRS1", write_fastaxis: false };
        let (cube, info) = open_synthetic(&dir, "two_ints_uncal.fits", &spec, |i, g, _, _| 1000.0 + (10.0 + 5.0 * i as f32) * g as f32);
        let err = quick_slope(&cube, &info, 2, &QuickSlopeParams::default(), &|_, _| {}, &|| false).unwrap_err();
        assert!(err.to_string().contains("integration"), "{err}");
        let second = quick_slope(&cube, &info, 1, &QuickSlopeParams::default(), &|_, _| {}, &|| false).unwrap();
        assert!(close(second.sci[[3, 2]], 1.5, 1e-4), "{}", second.sci[[3, 2]]);
        assert_eq!(second.integration, 1);
    }

    #[test]
    fn quick_slope_stops_when_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let spec = SyntheticUncal { cols: 4, rows: 8, ngroups: 5, nints: 1, irs2: None, tgroup_s: TGROUP, detector: "NRS1", write_fastaxis: false };
        let (cube, info) = open_synthetic(&dir, "plain_uncal.fits", &spec, |_, g, _, _| 1000.0 + 10.0 * g as f32);
        let err = quick_slope(&cube, &info, 0, &QuickSlopeParams::default(), &|_, _| {}, &|| true).unwrap_err();
        assert!(is_cancellation(&err), "{err}");
        assert!(format!("{err:#}").to_lowercase().contains("cancelled"));
    }

    #[test]
    fn quick_slope_marks_saturated_and_jump_pixels_in_the_counts() {
        let dir = tempfile::tempdir().unwrap();
        let spec = SyntheticUncal { cols: 8, rows: 16, ngroups: 10, nints: 1, irs2: None, tgroup_s: TGROUP, detector: "NRS1", write_fastaxis: false };
        let (cube, info) = open_synthetic(&dir, "flags_uncal.fits", &spec, |_, g, y, x| {
            let base = 1000.0 + 50.0 * g as f32;
            match (y, x) {
                (3, 4) if g >= 6 => 65_000.0,
                (9, 1) if g >= 5 => base + 500.0,
                (12, 7) => 65_000.0,
                _ => base,
            }
        });
        let product = run(&cube, &info, &QuickSlopeParams::default()).unwrap();
        assert_eq!(product.counts, FlagCounts { saturated: 2, jump_det: 1, do_not_use: 1 });
        assert_eq!(product.dq[[3, 4]], DQ_SATURATED);
        assert_eq!(product.ngood[[3, 4]], 6);
        assert_eq!(product.dq[[9, 1]], DQ_JUMP_DET);
        assert_eq!(product.ngood[[9, 1]], 9);
        assert_eq!(product.dq[[12, 7]], DQ_SATURATED | DQ_DO_NOT_USE);
        assert!(product.sci[[12, 7]].is_nan());
        assert!(close(product.sci[[9, 1]], 5.0, 1e-5));
    }

    const NOISE_ORACLE: &str = "scripts/oracle/quick_slope_noise_oracle.py, numpy 2.5.4, 400000 ramps per case";
    const ORACLE_TGROUP_S: f64 = 14.589;
    const ORACLE_READ_DN: f64 = 8.0;
    const ORACLE_DIFFERENCE_SIGMA_DN: f64 = 11.313708498984761;
    const ORACLE_OLS_DARK_STD: f64 = 0.06031;
    const ORACLE_OLS_READ_ONLY_STD: f64 = 0.06037;
    const ORACLE_OLS_FIFTY_STD: f64 = 0.65054;
    const ORACLE_OLS_MIRI_LIKE_STD: f64 = 0.74594;
    const ORACLE_MEDIAN_DARK_HIT_STD: f64 = 0.21822;
    const ORACLE_MEDIAN_FIFTY_HIT_STD: f64 = 0.79012;
    const MC_ROWS: usize = 200;
    const MC_WIDTH: usize = 100;
    const MC_BAND_ROWS: usize = 300;
    const MC_BAND_WIDTH: usize = 200;

    fn oracle_ramps(rate_dn_s: f64, seed: u64) -> NoiseRamps {
        NoiseRamps { ngroups: 10, tgroup_s: ORACLE_TGROUP_S, rate_dn_s, read_dn: ORACLE_READ_DN, gain_e_dn: 1.0, hit_dn: 0.0, hit_from_group: 0, seed }
    }

    fn dark_band_scale(ramps: &NoiseRamps, params: &QuickSlopeParams) -> f32 {
        let dark = NoiseRamps { rate_dn_s: 0.0, hit_dn: 0.0, seed: ramps.seed + 1, ..*ramps };
        let band = dark.band(MC_BAND_ROWS, MC_BAND_WIDTH);
        let rows: Vec<usize> = (0..MC_BAND_ROWS).collect();
        band_scale(&band, MC_BAND_WIDTH, 0..MC_BAND_ROWS, &rows, &ReferenceOffsets::zero(ramps.ngroups, MC_BAND_WIDTH), params)
    }

    fn fit_noise_ramps(ramps: &NoiseRamps, band_scale_dn: f32, params: &QuickSlopeParams) -> Vec<PixelFit> {
        let mut diffs = Vec::new();
        let mut flagged = Vec::new();
        (0..MC_ROWS * MC_WIDTH)
            .map(|p| {
                let raw = ramps.pixel(p);
                fit_pixel(&raw, &raw, ramps.tgroup_s, band_scale_dn, params, &mut diffs, &mut flagged)
            })
            .collect()
    }

    fn scatter_and_median_noise(fits: &[&PixelFit]) -> (f64, f64, f64) {
        let n = fits.len() as f64;
        let mean = fits.iter().map(|f| f.slope as f64).sum::<f64>() / n;
        let std = (fits.iter().map(|f| (f.slope as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
        let mut noises: Vec<f32> = fits.iter().map(|f| f.noise).collect();
        (mean, std, crate::math::median::median_f32_mut(&mut noises) as f64)
    }

    fn within(value: f64, reference: f64, fraction: f64) -> bool {
        (value - reference).abs() <= fraction * reference
    }

    #[test]
    fn with_a_gain_the_noise_plane_tracks_the_slope_scatter_of_an_illuminated_pixel() {
        let params = QuickSlopeParams { gain_e_per_dn: Some(1.0), ..Default::default() };
        let ramps = oracle_ramps(50.0, 11);
        let fits = fit_noise_ramps(&ramps, dark_band_scale(&ramps, &params), &params);
        let clean: Vec<&PixelFit> = fits.iter().filter(|f| f.dq == 0).collect();
        let (mean, std, noise) = scatter_and_median_noise(&clean);
        println!("50 DN/s gain 1: mean {mean:.4} std {std:.4} median NOISE {noise:.4} oracle {ORACLE_OLS_FIFTY_STD} ({NOISE_ORACLE})");
        assert!(within(mean, 50.0, 0.001), "the unweighted fit is unbiased: {mean}");
        assert!(within(std, ORACLE_OLS_FIFTY_STD, 0.03), "simulated scatter {std} against {ORACLE_OLS_FIFTY_STD} from {NOISE_ORACLE}");
        assert!(within(noise, ORACLE_OLS_FIFTY_STD, 0.10), "median NOISE {noise} against the slope scatter {ORACLE_OLS_FIFTY_STD} from {NOISE_ORACLE}");

        let miri_params = QuickSlopeParams { gain_e_per_dn: Some(3.9), ..Default::default() };
        let miri = NoiseRamps { ngroups: 40, tgroup_s: 2.775, rate_dn_s: 200.0, read_dn: 5.0, gain_e_dn: 3.9, hit_dn: 0.0, hit_from_group: 0, seed: 13 };
        let fits = fit_noise_ramps(&miri, dark_band_scale(&miri, &miri_params), &miri_params);
        let clean: Vec<&PixelFit> = fits.iter().filter(|f| f.dq == 0).collect();
        let (_, std, noise) = scatter_and_median_noise(&clean);
        println!("200 DN/s, 40 groups, gain 3.9: std {std:.4} median NOISE {noise:.4} oracle {ORACLE_OLS_MIRI_LIKE_STD}");
        assert!(within(std, ORACLE_OLS_MIRI_LIKE_STD, 0.03), "simulated scatter {std} against {ORACLE_OLS_MIRI_LIKE_STD} from {NOISE_ORACLE}");
        assert!(within(noise, ORACLE_OLS_MIRI_LIKE_STD, 0.10), "median NOISE {noise} against {ORACLE_OLS_MIRI_LIKE_STD} from {NOISE_ORACLE}");
    }

    #[test]
    fn without_a_gain_the_noise_plane_is_the_read_noise_part_whatever_the_flux() {
        let params = QuickSlopeParams::default();
        assert_eq!(params.gain_e_per_dn, None);
        for (rate, seed) in [(0.0, 21u64), (50.0, 23)] {
            let ramps = oracle_ramps(rate, seed);
            let fits = fit_noise_ramps(&ramps, dark_band_scale(&ramps, &params), &params);
            let clean: Vec<&PixelFit> = fits.iter().filter(|f| f.dq == 0).collect();
            let (_, std, noise) = scatter_and_median_noise(&clean);
            println!("{rate} DN/s without gain: std {std:.4} median NOISE {noise:.4} read part {ORACLE_OLS_READ_ONLY_STD}");
            assert!(within(noise, ORACLE_OLS_READ_ONLY_STD, 0.05), "{rate} DN/s: median NOISE {noise} against the read noise part {ORACLE_OLS_READ_ONLY_STD} from {NOISE_ORACLE}");
        }
    }

    #[test]
    fn the_band_scale_is_the_read_noise_sigma_of_a_group_difference() {
        let params = QuickSlopeParams::default();
        let ramps = oracle_ramps(0.0, 31);
        let scale = dark_band_scale(&ramps, &params) as f64;
        println!("read noise {ORACLE_READ_DN} DN: band scale {scale:.3} DN against sqrt(2) x read = {ORACLE_DIFFERENCE_SIGMA_DN:.3}");
        assert!(within(scale, ORACLE_DIFFERENCE_SIGMA_DN, 0.04), "band scale {scale} against {ORACLE_DIFFERENCE_SIGMA_DN}");
        let fits = fit_noise_ramps(&ramps, scale as f32, &params);
        let clean: Vec<&PixelFit> = fits.iter().filter(|f| f.dq == 0).collect();
        let (_, std, noise) = scatter_and_median_noise(&clean);
        assert!(within(std, ORACLE_OLS_DARK_STD, 0.03), "simulated scatter {std} against {ORACLE_OLS_DARK_STD} from {NOISE_ORACLE}");
        assert!(within(noise, ORACLE_OLS_DARK_STD, 0.05), "median NOISE {noise} against {ORACLE_OLS_DARK_STD} from {NOISE_ORACLE}");

        let gain_params = QuickSlopeParams { gain_e_per_dn: Some(1.0), ..Default::default() };
        let lit = oracle_ramps(50.0, 33).band(MC_BAND_ROWS, MC_BAND_WIDTH);
        let rows: Vec<usize> = (0..MC_BAND_ROWS).collect();
        let lit_scale = band_scale(&lit, MC_BAND_WIDTH, 0..MC_BAND_ROWS, &rows, &ReferenceOffsets::zero(10, MC_BAND_WIDTH), &gain_params) as f64;
        println!("uniformly lit band at 50 DN/s with gain 1: band scale {lit_scale:.3} DN");
        assert!(within(lit_scale, ORACLE_DIFFERENCE_SIGMA_DN, 0.10), "with a gain the photon noise of the band is taken out of its read noise scale: {lit_scale}");
    }

    #[test]
    fn the_median_path_noise_tracks_the_slope_scatter_after_a_hit() {
        let cases = [(0.0, None, ORACLE_MEDIAN_DARK_HIT_STD, 41u64), (50.0, Some(1.0f32), ORACLE_MEDIAN_FIFTY_HIT_STD, 43)];
        for (rate, gain, oracle_std, seed) in cases {
            let params = QuickSlopeParams { gain_e_per_dn: gain, ..Default::default() };
            let ramps = NoiseRamps { hit_dn: 400.0, hit_from_group: 5, ..oracle_ramps(rate, seed) };
            let fits = fit_noise_ramps(&ramps, dark_band_scale(&ramps, &params), &params);
            let hit: Vec<&PixelFit> = fits.iter().filter(|f| f.dq == DQ_JUMP_DET && f.ngood == 9).collect();
            assert!(hit.len() as f64 > 0.95 * fits.len() as f64, "{rate} DN/s: {} of {} ramps flagged exactly the hit", hit.len(), fits.len());
            let (mean, std, noise) = scatter_and_median_noise(&hit);
            println!("{rate} DN/s, 400 DN hit at group 5, gain {gain:?}: mean {mean:.4} std {std:.4} median NOISE {noise:.4} oracle {oracle_std}");
            assert!(within(std, oracle_std, 0.05), "{rate} DN/s: simulated scatter {std} against {oracle_std} from {NOISE_ORACLE}");
            assert!(within(noise, oracle_std, 0.10), "{rate} DN/s: median NOISE {noise} against {oracle_std} from {NOISE_ORACLE}");
        }
    }

    #[test]
    fn bright_clean_ramps_are_rarely_flagged() {
        let params = QuickSlopeParams::default();
        for (rate, seed) in [(50.0, 51u64), (300.0, 53)] {
            let ramps = oracle_ramps(rate, seed);
            let fits = fit_noise_ramps(&ramps, dark_band_scale(&ramps, &params), &params);
            let flagged = jump_fraction(&fits);
            println!("{rate} DN/s clean ramps: false JUMP_DET {:.3} %", flagged * 100.0);
            assert!(flagged < 0.005, "{rate} DN/s: false JUMP_DET fraction {flagged}");
        }
    }

    #[test]
    fn a_hit_of_twelve_sigma_on_a_lit_pixel_is_still_flagged() {
        let params = QuickSlopeParams::default();
        let sigma = (2.0 * ORACLE_READ_DN * ORACLE_READ_DN + 5.0 * ORACLE_TGROUP_S).sqrt();
        let ramps = NoiseRamps { hit_dn: (12.0 * sigma) as f32, hit_from_group: 5, ..oracle_ramps(5.0, 61) };
        let fits = fit_noise_ramps(&ramps, dark_band_scale(&ramps, &params), &params);
        let detected = jump_fraction(&fits);
        assert!(detected > 0.95, "a 12 sigma hit at 5 DN/s is flagged in {detected} of the ramps");
    }

    #[test]
    fn group_counts_above_the_sixteen_bit_range_are_not_wrapped() {
        let groups = 70_000usize;
        let raw: Vec<f32> = (0..groups).map(|g| 1000.0 + 0.5 * g as f32).collect();
        let (fit, _) = fit_raw(&raw, 0.0);
        assert_eq!((fit.ngood as usize, fit.n_usable as usize, fit.dq), (groups, groups, 0));
        let mut saturating = raw.clone();
        for v in saturating.iter_mut().skip(66_000) {
            *v = 65_000.0;
        }
        let (fit, _) = fit_raw(&saturating, 0.0);
        assert_eq!(fit.first_saturated.map(|g| g as usize), Some(66_000));
        assert_eq!(fit.ngood as usize, 66_000);
    }
}
