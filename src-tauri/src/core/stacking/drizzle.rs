use std::borrow::Cow;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::alignment::affine;
use crate::core::alignment::pair;
use crate::core::alignment::phase_correlation;
use crate::core::imaging::boundary::clamp_index;
use crate::core::stacking::combine::{check_percentile_window, reject_and_combine_with, KernelScratch, Sample};
use crate::core::stacking::{never_cancelled, stop_if_cancelled, CancelCheck};
use crate::types::error::AppError;
use crate::types::header::HduHeader;
use crate::types::stacking::{
    AlignmentMethod, DrizzleConfig, DrizzleKernel, DrizzleResult, FrameAlignment, RejectionMethod,
    RejectionParams,
};

struct KeepMask {
    bits: Vec<u8>,
    bytes_per_pixel: usize,
    in_rows: usize,
    in_cols: usize,
    shifts: Vec<(i64, i64)>,
    rejected: u64,
}

impl KeepMask {
    fn build(
        frames: &[Cow<Array2<f32>>],
        offsets: &[(f64, f64)],
        in_rows: usize,
        in_cols: usize,
        params: &RejectionParams,
        cancelled: CancelCheck,
    ) -> Self {
        let frame_count = frames.len();
        let bytes_per_pixel = frame_count.div_ceil(8).max(1);
        let shifts: Vec<(i64, i64)> = offsets
            .iter()
            .map(|&(dx, dy)| (dx.round() as i64, dy.round() as i64))
            .collect();
        let slices: Vec<&[f32]> = frames
            .iter()
            .map(|f| f.as_ref().as_slice().expect("contiguous"))
            .collect();
        let mut bits = vec![u8::MAX; in_rows * in_cols * bytes_per_pixel];
        let row_len = (in_cols * bytes_per_pixel).max(1);
        let rejected: u64 = bits
            .par_chunks_mut(row_len)
            .enumerate()
            .map(|(ry, row)| {
                let mut samples: Vec<Sample> = Vec::with_capacity(frame_count);
                let mut scratch = KernelScratch::default();
                let mut count = 0u64;
                if cancelled() {
                    return count;
                }
                for rx in 0..in_cols {
                    samples.clear();
                    for (i, slice) in slices.iter().enumerate() {
                        let (sx, sy) = shifts[i];
                        let fy = ry as i64 + sy;
                        let fx = rx as i64 + sx;
                        if fy < 0 || fx < 0 || fy >= in_rows as i64 || fx >= in_cols as i64 {
                            continue;
                        }
                        let v = slice[fy as usize * in_cols + fx as usize];
                        if v.is_finite() {
                            samples.push(Sample::plain(v, i as u16));
                        }
                    }
                    if samples.len() < 2 {
                        continue;
                    }
                    let outcome = reject_and_combine_with(&mut samples, None, params, &mut scratch);
                    if outcome.kept == 0 {
                        continue;
                    }
                    for sample in &samples[outcome.kept..] {
                        let frame = sample.frame as usize;
                        row[rx * bytes_per_pixel + frame / 8] &= !(1u8 << (frame % 8));
                        count += 1;
                    }
                }
                count
            })
            .sum();
        Self { bits, bytes_per_pixel, in_rows, in_cols, shifts, rejected }
    }

    #[inline]
    fn keeps(&self, frame: usize, iy: usize, ix: usize) -> bool {
        let (sx, sy) = self.shifts[frame];
        let ry = iy as i64 - sy;
        let rx = ix as i64 - sx;
        if ry < 0 || rx < 0 || ry >= self.in_rows as i64 || rx >= self.in_cols as i64 {
            return true;
        }
        let idx = (ry as usize * self.in_cols + rx as usize) * self.bytes_per_pixel + frame / 8;
        self.bits[idx] & (1u8 << (frame % 8)) != 0
    }
}

fn drizzle_support(scale: f64, pixfrac: f64, kernel: DrizzleKernel) -> (f64, f64, f64) {
    let half = pixfrac * scale * 0.5;
    let sigma = half.max(0.5);
    let support = match kernel {
        DrizzleKernel::Square => half,
        DrizzleKernel::Gaussian => sigma * 3.0,
        DrizzleKernel::Lanczos3 => 3.0 * scale,
    };
    (half, sigma, support)
}

#[allow(clippy::too_many_arguments)]
fn scatter_frame(
    acc: &mut DrizzleAccumulator,
    frame: &Array2<f32>,
    dx: f64,
    dy: f64,
    scale: f64,
    pixfrac: f64,
    kernel: DrizzleKernel,
    out_rows: usize,
    out_cols: usize,
    band_start: usize,
    band_end: usize,
    keep: Option<(&KeepMask, usize)>,
) {
    let (in_rows, in_cols) = frame.dim();
    let src = frame.as_slice().expect("contiguous");

    let (half, sigma, support) = drizzle_support(scale, pixfrac, kernel);

    let iy_lo_f = ((band_start as f64 - support) / scale) - dy - 2.5;
    let iy_hi_f = ((band_end as f64 + support) / scale) - dy + 1.5;
    let iy_lo = if iy_lo_f < 0.0 { 0 } else { (iy_lo_f.floor() as usize).min(in_rows) };
    let iy_hi = if iy_hi_f < 0.0 { 0 } else { (iy_hi_f.ceil() as usize).min(in_rows) };

    for iy in iy_lo..iy_hi {
        let row_base = iy * in_cols;
        for ix in 0..in_cols {
            let val = src[row_base + ix];
            if !val.is_finite() {
                continue;
            }
            if let Some((mask, frame_index)) = keep {
                if !mask.keeps(frame_index, iy, ix) {
                    continue;
                }
            }

            let cx = (ix as f64 + 0.5 + dx) * scale;
            let cy = (iy as f64 + 0.5 + dy) * scale;

            let ox_min = clamp_index((cx - support).floor() as i64, out_cols);
            let ox_max = clamp_index((cx + support).ceil() as i64, out_cols);
            let oy_min = clamp_index((cy - support).floor() as i64, out_rows).max(band_start);
            let oy_max = clamp_index((cy + support).ceil() as i64, out_rows).min(band_end - 1);
            if oy_min > oy_max {
                continue;
            }

            for oy in oy_min..=oy_max {
                for ox in ox_min..=ox_max {
                    let w = match kernel {
                        DrizzleKernel::Square => {
                            overlap_area(
                                cx - half, cy - half, cx + half, cy + half,
                                ox as f64, oy as f64, ox as f64 + 1.0, oy as f64 + 1.0,
                            )
                        }
                        DrizzleKernel::Gaussian => {
                            let dist2 = (ox as f64 + 0.5 - cx).powi(2)
                                + (oy as f64 + 0.5 - cy).powi(2);
                            (-dist2 / (2.0 * sigma * sigma)).exp()
                        }
                        DrizzleKernel::Lanczos3 => {
                            let ddx = (ox as f64 + 0.5 - cx).abs() / scale;
                            let ddy = (oy as f64 + 0.5 - cy).abs() / scale;
                            lanczos3(ddx) * lanczos3(ddy)
                        }
                    };

                    if w.abs() > 1e-4 {
                        let idx = (oy - band_start) * out_cols + ox;
                        acc.push(idx, val, w);
                    }
                }
            }
        }
    }
}

struct DrizzleAccumulator {
    vsum: Vec<f64>,
    wsum: Vec<f64>,
    out_cols: usize,
}

impl DrizzleAccumulator {
    fn new(out_rows: usize, out_cols: usize) -> Self {
        let n = out_rows * out_cols;
        Self {
            vsum: vec![0.0; n],
            wsum: vec![0.0; n],
            out_cols,
        }
    }

    #[inline]
    fn push(&mut self, idx: usize, val: f32, w: f64) {
        self.vsum[idx] += val as f64 * w;
        self.wsum[idx] += w;
    }

    #[allow(clippy::too_many_arguments)]
    fn drizzle_frame(
        &mut self,
        frame: &Array2<f32>,
        dx: f64,
        dy: f64,
        scale: f64,
        pixfrac: f64,
        kernel: DrizzleKernel,
        full_out_rows: usize,
        band_start: usize,
        band_end: usize,
        keep: Option<(&KeepMask, usize)>,
    ) {
        let out_cols = self.out_cols;
        scatter_frame(
            self, frame, dx, dy, scale, pixfrac, kernel, full_out_rows, out_cols,
            band_start, band_end, keep,
        );
    }

    fn finalize_into(&self, img: &mut [f32]) {
        for ((&v, &w), out) in self.vsum.iter().zip(self.wsum.iter()).zip(img.iter_mut()) {
            *out = if w > 1e-6 { (v / w) as f32 } else { f32::NAN };
        }
    }
}

#[inline]
fn overlap_area(
    ax1: f64, ay1: f64, ax2: f64, ay2: f64,
    bx1: f64, by1: f64, bx2: f64, by2: f64,
) -> f64 {
    let ox = (ax2.min(bx2) - ax1.max(bx1)).max(0.0);
    let oy = (ay2.min(by2) - ay1.max(by1)).max(0.0);
    ox * oy
}

#[inline]
fn lanczos3(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        return 1.0;
    }
    if x.abs() >= 3.0 {
        return 0.0;
    }
    let pi_x = std::f64::consts::PI * x;
    let pi_x_3 = pi_x / 3.0;
    (pi_x.sin() / pi_x) * (pi_x_3.sin() / pi_x_3)
}

const SIP_MAX_ORDER_SUM: i32 = 128;

fn to_drizzle_pixel(p: f64, scale: f64) -> f64 {
    scale * (p - 0.5) + 0.5
}

fn sip_power_sum(key: &str) -> Option<i32> {
    let rest = ["AP_", "BP_", "A_", "B_"].iter().find_map(|prefix| key.strip_prefix(prefix))?;
    let (p, q) = rest.split_once('_')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(p) || !digits(q) {
        return None;
    }
    let sum = p.parse::<i32>().ok()?.checked_add(q.parse::<i32>().ok()?)?;
    (sum <= SIP_MAX_ORDER_SUM).then_some(sum)
}

pub fn drizzle_wcs_updates(header: &HduHeader, scale: f64) -> Vec<(String, f64)> {
    let mut updates = Vec::new();
    for key in ["CRPIX1", "CRPIX2"] {
        if let Some(v) = header.get_f64(key) {
            updates.push((key.to_string(), to_drizzle_pixel(v, scale)));
        }
    }
    let cd_keys = ["CD1_1", "CD1_2", "CD2_1", "CD2_2"];
    let linear_keys = ["CDELT1", "CDELT2", "PC1_1", "PC1_2", "PC2_1", "PC2_2", "CROTA2"];
    if cd_keys.iter().any(|k| header.get_f64(k).is_some()) {
        for key in cd_keys {
            if let Some(v) = header.get_f64(key) {
                updates.push((key.to_string(), v / scale));
            }
        }
    } else if linear_keys.iter().any(|k| header.get_f64(k).is_some()) {
        for key in ["CDELT1", "CDELT2"] {
            updates.push((key.to_string(), header.get_f64(key).unwrap_or(1.0) / scale));
        }
    }
    let physical_keys = ["LTV1", "LTV2", "LTM1_1", "LTM1_2", "LTM2_1", "LTM2_2"];
    if physical_keys.iter().any(|k| header.get_f64(k).is_some()) {
        for key in ["LTV1", "LTV2"] {
            updates.push((key.to_string(), to_drizzle_pixel(header.get_f64(key).unwrap_or(0.0), scale)));
        }
        for key in ["LTM1_1", "LTM2_2"] {
            updates.push((key.to_string(), header.get_f64(key).unwrap_or(1.0) * scale));
        }
        for key in ["LTM1_2", "LTM2_1"] {
            if let Some(v) = header.get_f64(key) {
                updates.push((key.to_string(), v * scale));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for (raw_key, _) in &header.cards {
        let key = raw_key.trim();
        if !seen.insert(key) {
            continue;
        }
        let factor = match key {
            "A_DMAX" | "B_DMAX" => Some(scale),
            _ => sip_power_sum(key).map(|sum| scale.powi(1 - sum)),
        };
        if let (Some(factor), Some(v)) = (factor, header.get_f64(key)) {
            updates.push((key.to_string(), v * factor));
        }
    }
    updates
}

fn drizzle_band_rows(out_rows: usize, threads: usize) -> usize {
    let max_rows = out_rows.max(1);
    out_rows
        .div_ceil(4 * threads.max(1))
        .clamp(64.min(max_rows), max_rows)
}

enum Registration {
    Shift(f64, f64),
    Warped(Array2<f32>),
    Failed,
}

fn register_frame(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    method: AlignmentMethod,
    rows: usize,
    cols: usize,
) -> (Registration, FrameAlignment) {
    if method == AlignmentMethod::PhaseCorrelation {
        let pc = phase_correlation::phase_correlate(reference, target);
        if !phase_correlation::is_low_confidence(pc.confidence) && pair::offset_within_limits(pc.dy, pc.dx, rows, cols) {
            let record = FrameAlignment {
                method: "phase_correlation".into(),
                confidence: Some(pc.confidence),
                included: true,
            };
            return (Registration::Shift(pc.dx, pc.dy), record);
        }
    }
    let result = affine::align_channel_affine(reference, target);
    let registered = result.method != affine::AffineAlignMethod::Identity;
    let record = FrameAlignment {
        method: result.method.to_string(),
        confidence: Some(result.confidence),
        included: registered,
    };
    if !registered {
        return (Registration::Failed, record);
    }
    (Registration::Warped(affine::warp_image(target, &result.transform, rows, cols)), record)
}

pub fn drizzle_stack(
    images: &[Array2<f32>],
    config: &DrizzleConfig,
) -> Result<DrizzleResult> {
    drizzle_stack_cancellable(images, config, &never_cancelled)
}

pub fn drizzle_stack_cancellable(
    images: &[Array2<f32>],
    config: &DrizzleConfig,
    cancelled: CancelCheck,
) -> Result<DrizzleResult> {
    if images.is_empty() {
        bail!("No images to drizzle");
    }
    if images.len() < 2 {
        bail!("Drizzle requires at least 2 frames for sub-pixel reconstruction");
    }

    let min_rows = images.iter().map(|img| img.dim().0).min().unwrap_or(0);
    let min_cols = images.iter().map(|img| img.dim().1).min().unwrap_or(0);
    let max_rows = images.iter().map(|img| img.dim().0).max().unwrap_or(0);
    let max_cols = images.iter().map(|img| img.dim().1).max().unwrap_or(0);

    let row_diff = max_rows - min_rows;
    let col_diff = max_cols - min_cols;
    let tolerance = (min_rows.max(min_cols) as f64 * 0.05) as usize;

    if row_diff > tolerance || col_diff > tolerance {
        bail!(
            "Frame dimensions vary too much (rows: {}px, cols: {}px, tolerance: {}px)",
            row_diff, col_diff, tolerance
        );
    }

    let in_rows = min_rows;
    let in_cols = min_cols;

    let images_ref: Vec<Cow<Array2<f32>>> = images
        .iter()
        .map(|img| {
            let (r, c) = img.dim();
            if r == in_rows && c == in_cols {
                Cow::Borrowed(img)
            } else {
                Cow::Owned(img.slice(ndarray::s![..in_rows, ..in_cols]).to_owned())
            }
        })
        .collect();

    let scale = config.scale.clamp(1.0, 4.0);
    let pixfrac = config.pixfrac.clamp(0.1, 1.0);
    let out_rows = (in_rows as f64 * scale).ceil() as usize;
    let out_cols = (in_cols as f64 * scale).ceil() as usize;
    let params = config.rejection_params();

    let reference: &Array2<f32> = images_ref[0].as_ref();

    let total = images_ref.len();
    let mut offsets: Vec<(f64, f64)> = Vec::with_capacity(total);
    offsets.push((0.0, 0.0));
    let mut frames: Vec<Cow<Array2<f32>>> = Vec::with_capacity(total);
    frames.push(Cow::Borrowed(reference));
    let mut alignment: Vec<FrameAlignment> = Vec::with_capacity(total);
    alignment.push(FrameAlignment::reference());
    let mut warnings: Vec<String> = Vec::new();

    if config.align {
        let registrations: Option<Vec<(Registration, FrameAlignment)>> = images_ref[1..]
            .par_iter()
            .map(|target| {
                (!cancelled()).then(|| register_frame(reference, target.as_ref(), config.alignment_method, in_rows, in_cols))
            })
            .collect();
        let Some(registrations) = registrations else {
            return Err(AppError::Cancelled.into());
        };
        for (i, (registration, record)) in registrations.into_iter().enumerate() {
            match registration {
                Registration::Shift(dx, dy) => {
                    offsets.push((dx, dy));
                    frames.push(Cow::Borrowed(images_ref[1 + i].as_ref()));
                }
                Registration::Warped(warped) => {
                    offsets.push((0.0, 0.0));
                    frames.push(Cow::Owned(warped));
                }
                Registration::Failed => {
                    let message = format!(
                        "Frame {} of {} was left out of the drizzle: it could not be aligned to frame 1 ({}, confidence {:.2})",
                        i + 2,
                        total,
                        record.method,
                        record.confidence.unwrap_or(0.0)
                    );
                    log::warn!("{}", message);
                    warnings.push(message);
                }
            }
            alignment.push(record);
        }
    } else {
        for target in &images_ref[1..] {
            offsets.push((0.0, 0.0));
            frames.push(Cow::Borrowed(target.as_ref()));
            alignment.push(FrameAlignment::unaligned());
        }
    }

    if frames.len() < 2 {
        bail!(
            "Drizzle needs at least 2 aligned frames, but no frame could be aligned to the reference frame (frame 1); if the frames are already registered, drizzle them with alignment off. {}",
            warnings.join(" ")
        );
    }
    check_percentile_window(reference.view(), frames.len(), &params)?;

    let band_rows = drizzle_band_rows(out_rows, rayon::current_num_threads());

    let keep_mask = if config.rejection == RejectionMethod::None {
        None
    } else {
        Some(KeepMask::build(&frames, &offsets, in_rows, in_cols, &params, cancelled))
    };
    stop_if_cancelled(cancelled)?;
    let rejected_pixels = keep_mask.as_ref().map_or(0, |mask| mask.rejected);

    let mut img_data = vec![0.0f32; out_rows * out_cols];

    let chunk = (band_rows * out_cols).max(1);
    img_data
        .par_chunks_mut(chunk)
        .enumerate()
        .for_each(|(b, img_chunk)| {
            let band_start = b * band_rows;
            let band_end = (band_start + band_rows).min(out_rows);
            let mut accumulator = DrizzleAccumulator::new(band_end - band_start, out_cols);
            for (i, img) in frames.iter().enumerate() {
                if cancelled() {
                    return;
                }
                let (dx, dy) = offsets[i];
                accumulator.drizzle_frame(
                    img.as_ref(), -dx, -dy, scale, pixfrac, config.kernel,
                    out_rows, band_start, band_end,
                    keep_mask.as_ref().map(|mask| (mask, i)),
                );
            }
            accumulator.finalize_into(img_chunk);
        });
    stop_if_cancelled(cancelled)?;

    let image = Array2::from_shape_vec((out_rows, out_cols), img_data)
        .context("Failed to reshape drizzle output")?;

    Ok(DrizzleResult {
        image,
        frame_count: frames.len(),
        output_scale: scale,
        input_dims: (in_rows, in_cols),
        output_dims: (out_rows, out_cols),
        offsets,
        rejected_pixels,
        alignment,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(scale: f64) -> DrizzleConfig {
        DrizzleConfig {
            scale,
            pixfrac: 0.7,
            kernel: DrizzleKernel::Square,
            align: false,
            ..DrizzleConfig::default()
        }
    }

    #[test]
    fn uniform_frames_yield_weighted_mean_identity() {
        let frames = vec![
            Array2::from_elem((32, 32), 5.0f32),
            Array2::from_elem((32, 32), 5.0f32),
            Array2::from_elem((32, 32), 5.0f32),
        ];
        let result = drizzle_stack(&frames, &config(2.0)).unwrap();
        assert_eq!(result.rejected_pixels, 0);
        let covered: Vec<f32> = result.image.iter().copied().filter(|v| v.is_finite()).collect();
        assert!(covered.len() > result.image.len() / 2);
        for v in covered {
            assert!((v - 5.0).abs() < 1e-4, "pixel {} != 5.0", v);
        }
    }

    #[test]
    fn uncovered_output_pixels_are_nan_not_zero() {
        let frames = vec![
            Array2::from_elem((16, 16), 5.0f32),
            Array2::from_elem((16, 16), 5.0f32),
        ];
        let mut cfg = config(4.0);
        cfg.pixfrac = 0.1;
        let result = drizzle_stack(&frames, &cfg).unwrap();
        let holes = result.image.iter().filter(|v| v.is_nan()).count();
        assert!(holes > 0);
        assert!(result.image.iter().all(|&v| v.is_nan() || (v - 5.0).abs() < 1e-4));
        assert!(!result.image.iter().any(|&v| v == 0.0));
    }

    fn blob_frame(cx: f32) -> Array2<f32> {
        Array2::from_shape_fn((300, 300), |(y, x)| {
            let dy = y as f32 - 150.0;
            let dx = x as f32 - cx;
            100.0 + 1000.0 * (-(dy * dy + dx * dx) / 18.0).exp()
        })
    }

    #[test]
    fn frame_with_an_out_of_range_shift_is_left_out_and_reported() {
        let reference = blob_frame(50.0);
        let frames = vec![reference.clone(), reference.clone(), blob_frame(250.0)];
        let mut cfg = config(1.0);
        cfg.align = true;
        cfg.pixfrac = 1.0;
        cfg.rejection = RejectionMethod::None;
        let result = drizzle_stack(&frames, &cfg).unwrap();
        assert_eq!(result.frame_count, 2);
        assert_eq!(result.alignment.len(), 3);
        assert!(result.alignment[1].included, "{:?}", result.alignment[1]);
        assert!(!result.alignment[2].included, "{:?}", result.alignment[2]);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("Frame 3 of 3"), "{}", result.warnings[0]);
        assert!((result.image[[150, 250]] - 100.0).abs() < 1e-2, "ghost {}", result.image[[150, 250]]);

        let lonely = vec![reference, blob_frame(250.0)];
        let err = drizzle_stack(&lonely, &cfg).unwrap_err().to_string();
        assert!(err.contains("at least 2 aligned frames"), "{err}");
    }

    #[test]
    fn drizzle_percentile_clip_refuses_a_background_near_zero() {
        let frames: Vec<Array2<f32>> = (0..3)
            .map(|k| Array2::from_shape_fn((24, 24), |(y, x)| ((y * 7 + x * 13 + k * 5) % 11) as f32 - 5.0))
            .collect();
        let mut cfg = config(1.0);
        cfg.rejection = RejectionMethod::PercentileClip;
        let err = drizzle_stack(&frames, &cfg).unwrap_err().to_string();
        assert!(err.contains("Percentile clipping"), "{err}");
    }

    fn noisy_frames(scene: impl Fn(usize, usize) -> f32, count: usize, sigma: f32) -> Vec<Array2<f32>> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let spread = sigma * 12f32.sqrt();
        (0..count)
            .map(|_| {
                Array2::from_shape_fn((32, 32), |(y, x)| {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let unit = ((state >> 40) as f32) / ((1u64 << 24) as f32);
                    scene(y, x) + (unit - 0.5) * spread
                })
            })
            .collect()
    }

    #[test]
    fn drizzle_percentile_clip_accepts_a_sky_gradient_and_small_stacks() {
        let mut cfg = config(1.0);
        cfg.rejection = RejectionMethod::PercentileClip;
        let gradient = noisy_frames(|_, x| 400.0 + 600.0 * x as f32 / 31.0, 3, 5.0);
        let result = drizzle_stack(&gradient, &cfg).unwrap();
        assert_eq!(result.frame_count, 3);

        let near_zero = noisy_frames(|_, _| 0.0, 2, 1.0);
        let result = drizzle_stack(&near_zero, &cfg).unwrap();
        assert_eq!(result.rejected_pixels, 0);
    }

    #[test]
    fn two_constant_frames_average_in_overlap() {
        let frames = vec![
            Array2::from_elem((24, 24), 2.0f32),
            Array2::from_elem((24, 24), 6.0f32),
        ];
        let result = drizzle_stack(&frames, &config(1.0)).unwrap();
        assert_eq!(result.rejected_pixels, 0);
        let center = result.image[[12, 12]];
        assert!((center - 4.0).abs() < 1e-4, "center {} != 4.0", center);
    }

    #[test]
    fn drizzle_rejects_a_cosmic_ray_present_in_one_of_five_frames() {
        let mut frames: Vec<Array2<f32>> =
            (0..5).map(|_| Array2::from_elem((16, 16), 1.0f32)).collect();
        frames[2][[8, 8]] = 10000.0;

        let result = drizzle_stack(&frames, &config(2.0)).unwrap();
        assert!(result.rejected_pixels >= 1, "rejected {}", result.rejected_pixels);
        for oy in 16..18 {
            for ox in 16..18 {
                let v = result.image[[oy, ox]];
                assert!((v - 1.0).abs() < 1e-3, "pixel ({}, {}) = {} != 1.0", oy, ox, v);
            }
        }

        let mut off = config(2.0);
        off.rejection = RejectionMethod::None;
        let leaked = drizzle_stack(&frames, &off).unwrap();
        assert_eq!(leaked.rejected_pixels, 0);
        assert!(leaked.image[[17, 17]] > 100.0, "ray missing without rejection: {}", leaked.image[[17, 17]]);
    }

    #[test]
    fn keep_mask_compares_frames_at_their_integer_shifted_positions() {
        let rows = 16;
        let cols = 16;
        let reference = Array2::from_shape_fn((rows, cols), |(_, x)| 100.0 * x as f32);
        let mut shifted = Array2::from_shape_fn((rows, cols), |(_, x)| 100.0 * x.saturating_sub(1) as f32);
        shifted[[5, 6]] = 50000.0;
        let frames: Vec<Cow<Array2<f32>>> = vec![
            Cow::Borrowed(&reference),
            Cow::Owned(shifted),
            Cow::Borrowed(&reference),
            Cow::Borrowed(&reference),
            Cow::Borrowed(&reference),
        ];
        let offsets = vec![(0.0, 0.0), (1.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)];
        let params = config(1.0).rejection_params();

        let mask = KeepMask::build(&frames, &offsets, rows, cols, &params, &never_cancelled);
        assert_eq!(mask.rejected, 1, "shift ignored: {} rejections", mask.rejected);
        assert!(!mask.keeps(1, 5, 6));
        assert!(mask.keeps(1, 5, 5));
        assert!(mask.keeps(0, 5, 5));
        assert!(mask.keeps(1, 0, 0));

        let unshifted = vec![(0.0, 0.0); 5];
        let naive = KeepMask::build(&frames, &unshifted, rows, cols, &params, &never_cancelled);
        assert!(naive.rejected > 100, "expected the slope to be rejected without the shift: {}", naive.rejected);
    }

    #[test]
    fn lanczos3_at_integer_scale_three_leaves_no_holes() {
        let frames = vec![
            Array2::from_elem((24, 24), 5.0f32),
            Array2::from_elem((24, 24), 5.0f32),
        ];
        let mut cfg = config(3.0);
        cfg.kernel = DrizzleKernel::Lanczos3;
        let result = drizzle_stack(&frames, &cfg).unwrap();
        assert_eq!(result.output_dims, (72, 72));
        for ((oy, ox), &v) in result.image.indexed_iter() {
            assert!((v - 5.0).abs() < 1e-3, "pixel ({}, {}) = {} != 5.0", oy, ox, v);
        }
    }

    #[test]
    fn mixed_frame_dims_are_cropped_to_smallest() {
        let frames = vec![
            Array2::from_elem((24, 24), 2.0f32),
            Array2::from_elem((25, 24), 6.0f32),
            Array2::from_elem((24, 25), 6.0f32),
        ];
        let mut cfg = config(1.0);
        cfg.pixfrac = 1.0;
        let result = drizzle_stack(&frames, &cfg).unwrap();
        assert_eq!(result.input_dims, (24, 24));
        assert_eq!(result.frame_count, 3);
        for &v in result.image.iter() {
            assert!((v - 14.0 / 3.0).abs() < 1e-4, "pixel {} != 14/3", v);
        }
    }

    fn distorted_header() -> HduHeader {
        let mut header = HduHeader::empty();
        for (key, value) in [
            ("CTYPE1", "RA---TAN-SIP"),
            ("CRPIX1", "7.3"),
            ("CRPIX2", "5.6"),
            ("CD1_1", "-2.0E-4"),
            ("CD1_2", "3.0E-5"),
            ("CD2_1", "4.0E-5"),
            ("CD2_2", "2.0E-4"),
            ("A_ORDER", "2"),
            ("A_2_0", "1.0E-4"),
            ("A_1_1", "-2.0E-5"),
            ("B_ORDER", "2"),
            ("B_0_2", "3.0E-5"),
            ("AP_ORDER", "2"),
            ("AP_1_0", "1.0E-6"),
            ("AP_2_0", "-1.0E-4"),
            ("A_DMAX", "0.5"),
            ("LTV1", "-10.0"),
            ("LTV2", "-20.0"),
        ] {
            header.set(key, value.to_string());
        }
        header
    }

    fn apply(header: &HduHeader, updates: Vec<(String, f64)>) -> HduHeader {
        let mut out = header.clone();
        for (key, value) in updates {
            out.set_f64(&key, value);
        }
        out
    }

    fn card(header: &HduHeader, key: &str) -> f64 {
        header.get_f64(key).unwrap_or(0.0)
    }

    fn intermediate(header: &HduHeader, px: f64, py: f64) -> (f64, f64) {
        let (u, v) = (px - card(header, "CRPIX1"), py - card(header, "CRPIX2"));
        let x = u + card(header, "A_2_0") * u * u + card(header, "A_1_1") * u * v;
        let y = v + card(header, "B_0_2") * v * v;
        (
            card(header, "CD1_1") * x + card(header, "CD1_2") * y,
            card(header, "CD2_1") * x + card(header, "CD2_2") * y,
        )
    }

    fn inverse_distortion(header: &HduHeader, u: f64) -> f64 {
        card(header, "AP_1_0") * u + card(header, "AP_2_0") * u * u
    }

    fn physical(header: &HduHeader, px: f64, py: f64) -> (f64, f64) {
        let ltm = |key: &str| header.get_f64(key).unwrap_or(1.0);
        ((px - card(header, "LTV1")) / ltm("LTM1_1"), (py - card(header, "LTV2")) / ltm("LTM2_2"))
    }

    #[test]
    fn drizzle_wcs_follows_the_pixel_the_drizzle_writes() {
        let (iy, ix) = (4usize, 6usize);
        let mut frame = Array2::from_elem((10, 12), 0.0f32);
        frame[[iy, ix]] = 900.0;
        let mut cfg = config(3.0);
        cfg.pixfrac = 1.0;
        cfg.rejection = RejectionMethod::None;
        let result = drizzle_stack(&[frame.clone(), frame], &cfg).unwrap();
        let (oy, ox) = (3 * iy + 1, 3 * ix + 1);
        for dy in 0..3 {
            for dx in 0..3 {
                assert_eq!(result.image[[oy - 1 + dy, ox - 1 + dx]], 900.0);
            }
        }
        assert_eq!(result.image[[oy - 2, ox]], 0.0);

        let source = distorted_header();
        let drizzled = apply(&source, drizzle_wcs_updates(&source, result.output_scale));
        let (in_px, in_py) = ((ix + 1) as f64, (iy + 1) as f64);
        let (out_px, out_py) = ((ox + 1) as f64, (oy + 1) as f64);
        let (a, b) = (intermediate(&source, in_px, in_py), intermediate(&drizzled, out_px, out_py));
        assert!((a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12, "{a:?} vs {b:?}");
        let (pa, pb) = (physical(&source, in_px, in_py), physical(&drizzled, out_px, out_py));
        assert!((pa.0 - pb.0).abs() < 1e-9 && (pa.1 - pb.1).abs() < 1e-9, "{pa:?} vs {pb:?}");

        let u_in = in_px - card(&source, "CRPIX1");
        let u_out = out_px - card(&drizzled, "CRPIX1");
        let expected = 3.0 * inverse_distortion(&source, u_in);
        assert!((inverse_distortion(&drizzled, u_out) - expected).abs() < 1e-9);
        assert_eq!(card(&drizzled, "A_DMAX"), 1.5);
        assert_eq!(drizzled.get("A_ORDER"), Some("2"));
        assert_eq!(drizzled.get("CTYPE1"), Some("RA---TAN-SIP"));
    }

    #[test]
    fn drizzle_wcs_scales_cdelt_when_there_is_no_cd_matrix() {
        let mut header = HduHeader::empty();
        for (key, value) in [("CRPIX1", "10.5"), ("CRPIX2", "20.5"), ("CDELT1", "-0.001"), ("PC1_2", "0.1")] {
            header.set(key, value.to_string());
        }
        let updates: std::collections::HashMap<String, f64> = drizzle_wcs_updates(&header, 2.0).into_iter().collect();
        assert_eq!(updates["CRPIX1"], 20.5);
        assert_eq!(updates["CRPIX2"], 40.5);
        assert_eq!(updates["CDELT1"], -0.0005);
        assert_eq!(updates["CDELT2"], 0.5);
        assert!(!updates.contains_key("PC1_2"));
        assert!(drizzle_wcs_updates(&HduHeader::empty(), 2.0).is_empty());
    }

    #[test]
    fn a_cancelled_drizzle_stops_with_a_cancellation_error() {
        let frames = vec![blob_frame(50.0), blob_frame(50.0), blob_frame(51.0)];
        let mut cfg = config(1.0);
        cfg.align = true;
        let err = drizzle_stack_cancellable(&frames, &cfg, &|| true).unwrap_err();
        assert!(crate::core::stacking::is_cancellation(&err), "{err:#}");

        let checks = std::sync::atomic::AtomicUsize::new(0);
        let late = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 2;
        let err = drizzle_stack_cancellable(&frames, &cfg, &late).unwrap_err();
        assert!(crate::core::stacking::is_cancellation(&err), "a cancel raised after registration was ignored");
    }

    #[test]
    fn band_rows_follow_thread_count_within_bounds() {
        assert_eq!(drizzle_band_rows(0, 8), 1);
        assert_eq!(drizzle_band_rows(48, 8), 48);
        assert_eq!(drizzle_band_rows(200, 1), 64);
        assert_eq!(drizzle_band_rows(200, 16), 64);
        assert_eq!(drizzle_band_rows(8000, 8), 250);
        assert_eq!(drizzle_band_rows(8000, 1), 2000);
    }

    #[test]
    fn multi_band_output_matches_input_gradient_exactly() {
        let rows = 100;
        let cols = 30;
        let gradient = Array2::from_shape_fn((rows, cols), |(y, x)| (y * cols + x) as f32);
        let frames = vec![gradient.clone(), gradient.clone()];
        let mut cfg = config(2.0);
        cfg.pixfrac = 1.0;
        let result = drizzle_stack(&frames, &cfg).unwrap();
        assert_eq!(result.output_dims, (200, 60));
        assert!(drizzle_band_rows(200, rayon::current_num_threads()) < 200);
        for ((oy, ox), &v) in result.image.indexed_iter() {
            let expected = gradient[[oy / 2, ox / 2]];
            assert!((v - expected).abs() < 1e-3, "pixel ({}, {}) = {} != {}", oy, ox, v, expected);
        }
    }
}
