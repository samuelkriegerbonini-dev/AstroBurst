use std::borrow::Cow;

use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;

pub use crate::types::stacking::{AlignmentMethod, DrizzleConfig, DrizzleKernel, DrizzleResult};

use crate::core::alignment::affine;
use crate::core::alignment::phase_correlation;
use crate::core::imaging::boundary::clamp_index;
use crate::math::median::median_f32_mut;
use crate::types::constants::MAD_TO_SIGMA;

fn drizzle_support(scale: f64, pixfrac: f64, kernel: DrizzleKernel) -> (f64, f64, f64) {
    let half = pixfrac * scale * 0.5;
    let sigma = half.max(0.5);
    let support = match kernel {
        DrizzleKernel::Square => half,
        DrizzleKernel::Gaussian => sigma * 3.0,
        DrizzleKernel::Lanczos3 => 3.0,
    };
    (half, sigma, support)
}

fn frame_contributions(
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
) -> Vec<Vec<(usize, f32, f64)>> {
    let (in_rows, in_cols) = frame.dim();
    let src = frame.as_slice().expect("contiguous");

    let (half, sigma, support) = drizzle_support(scale, pixfrac, kernel);

    let iy_lo_f = ((band_start as f64 - support) / scale) - dy - 2.5;
    let iy_hi_f = ((band_end as f64 + support) / scale) - dy + 1.5;
    let iy_lo = if iy_lo_f < 0.0 { 0 } else { (iy_lo_f.floor() as usize).min(in_rows) };
    let iy_hi = if iy_hi_f < 0.0 { 0 } else { (iy_hi_f.ceil() as usize).min(in_rows) };

    (iy_lo..iy_hi)
        .into_par_iter()
        .map(|iy| {
            let mut contribs = Vec::new();
            let row_base = iy * in_cols;
            for ix in 0..in_cols {
                let val = src[row_base + ix];
                if !val.is_finite() {
                    continue;
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
                                let ddx = (ox as f64 + 0.5 - cx).abs();
                                let ddy = (oy as f64 + 0.5 - cy).abs();
                                lanczos3(ddx) * lanczos3(ddy)
                            }
                        };

                        if w.abs() > 1e-4 {
                            let idx = (oy - band_start) * out_cols + ox;
                            contribs.push((idx, val, w));
                        }
                    }
                }
            }
            contribs
        })
        .collect()
}

struct DrizzleAccumulator {
    storage: Vec<f32>,
    sample_weights: Vec<f32>,
    slot_starts: Vec<usize>,
    fill: Vec<u32>,
    weights: Vec<f64>,
    out_rows: usize,
    out_cols: usize,
}

impl DrizzleAccumulator {
    fn new(out_rows: usize, out_cols: usize, counts: &[u32]) -> Self {
        let n = out_rows * out_cols;
        let mut slot_starts = Vec::with_capacity(n + 1);
        let mut total = 0usize;
        slot_starts.push(0);
        for &c in counts {
            total += c as usize;
            slot_starts.push(total);
        }
        Self {
            storage: vec![0.0f32; total],
            sample_weights: vec![0.0f32; total],
            slot_starts,
            fill: vec![0u32; n],
            weights: vec![0.0; n],
            out_rows,
            out_cols,
        }
    }

    #[inline]
    fn push(&mut self, idx: usize, val: f32, w: f64) {
        let slot = self.slot_starts[idx] + self.fill[idx] as usize;
        if slot < self.slot_starts[idx + 1] {
            self.storage[slot] = val;
            self.sample_weights[slot] = w as f32;
            self.fill[idx] += 1;
            self.weights[idx] += w;
        }
    }

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
    ) {
        let rows = frame_contributions(
            frame, dx, dy, scale, pixfrac, kernel, full_out_rows, self.out_cols,
            band_start, band_end,
        );
        for contribs in rows {
            for (idx, val, w) in contribs {
                self.push(idx, val, w);
            }
        }
    }

    fn finalize(
        &self,
        sigma_low: f32,
        sigma_high: f32,
        sigma_iterations: usize,
    ) -> (Vec<f32>, Vec<f32>, u64) {
        let n = self.out_rows * self.out_cols;

        let results: Vec<(f32, f32, u64)> = (0..n)
            .into_par_iter()
            .map_init(
                || (Vec::new(), Vec::new(), Vec::new()),
                |(active, vals, devs): &mut (Vec<(f32, f32)>, Vec<f32>, Vec<f32>), i| {
                    let count = self.fill[i] as usize;
                    if count == 0 {
                        return (0.0, 0.0, 0);
                    }
                    let base = self.slot_starts[i];
                    if count == 1 {
                        return (self.storage[base], self.weights[i] as f32, 0);
                    }

                    active.clear();
                    active.extend(
                        (0..count).map(|k| (self.storage[base + k], self.sample_weights[base + k])),
                    );
                    let mut rejected = 0u64;

                    for _ in 0..sigma_iterations {
                        if active.len() < 3 {
                            break;
                        }

                        vals.clear();
                        vals.extend(active.iter().map(|(v, _)| *v));
                        let median = median_f32_mut(vals);
                        devs.clear();
                        devs.extend(vals.iter().map(|v| (v - median).abs()));
                        let mad = median_f32_mut(devs);
                        let sigma = (mad as f64 * MAD_TO_SIGMA).max(1e-10) as f32;

                        let before = active.len();
                        active.retain(|&(v, _)| {
                            let dev = v - median;
                            dev >= -sigma_low * sigma && dev <= sigma_high * sigma
                        });
                        let removed = before - active.len();
                        rejected += removed as u64;
                        if removed == 0 {
                            break;
                        }
                    }

                    let (mut vsum, mut wsum) = (0.0f64, 0.0f64);
                    if active.is_empty() {
                        for k in 0..count {
                            let w = self.sample_weights[base + k] as f64;
                            vsum += self.storage[base + k] as f64 * w;
                            wsum += w;
                        }
                    } else {
                        for &(v, w) in active.iter() {
                            vsum += v as f64 * w as f64;
                            wsum += w as f64;
                        }
                    }

                    let mean = if wsum > 1e-6 { (vsum / wsum) as f32 } else { 0.0 };
                    (mean, wsum.max(0.0) as f32, rejected)
                },
            )
            .collect();

        let mut img_data = Vec::with_capacity(n);
        let mut wgt_data = Vec::with_capacity(n);
        let mut total_rejected = 0u64;

        for (val, wgt, rej) in results {
            img_data.push(val);
            wgt_data.push(wgt);
            total_rejected += rej;
        }

        (img_data, wgt_data, total_rejected)
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

pub fn drizzle_stack(
    images: &[Array2<f32>],
    config: &DrizzleConfig,
) -> Result<DrizzleResult> {
    if images.is_empty() {
        bail!("No images to drizzle");
    }
    if images.len() < 2 {
        bail!("Drizzle requires at least 2 frames for sub-pixel reconstruction");
    }

    let min_rows = images.iter().map(|img| img.dim().0).min().unwrap();
    let min_cols = images.iter().map(|img| img.dim().1).min().unwrap();
    let max_rows = images.iter().map(|img| img.dim().0).max().unwrap();
    let max_cols = images.iter().map(|img| img.dim().1).max().unwrap();

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

    let needs_crop = row_diff > 0 || col_diff > 0;
    let cropped: Vec<Array2<f32>>;
    let images_ref: Vec<&Array2<f32>> = if needs_crop {
        cropped = images.iter().map(|img| {
            let (r, c) = img.dim();
            if r == in_rows && c == in_cols {
                img.clone()
            } else {
                img.slice(ndarray::s![..in_rows, ..in_cols]).to_owned()
            }
        }).collect();
        cropped.iter().collect()
    } else {
        images.iter().collect()
    };

    let scale = config.scale.clamp(1.0, 4.0);
    let pixfrac = config.pixfrac.clamp(0.1, 1.0);
    let out_rows = (in_rows as f64 * scale).ceil() as usize;
    let out_cols = (in_cols as f64 * scale).ceil() as usize;

    let reference = images_ref[0];
    let mut offsets: Vec<(f64, f64)> = Vec::with_capacity(images_ref.len());
    offsets.push((0.0, 0.0));
    let mut frames: Vec<Cow<Array2<f32>>> = Vec::with_capacity(images_ref.len());
    frames.push(Cow::Borrowed(reference));

    if config.align {
        match config.alignment_method {
            AlignmentMethod::PhaseCorrelation => {
                let aligned: Vec<(f64, f64, Option<Array2<f32>>)> = images_ref[1..]
                    .par_iter()
                    .map(|target| {
                        let pc = phase_correlation::phase_correlate(reference, target);
                        if phase_correlation::is_low_confidence(pc.confidence) {
                            let result = affine::align_channel_affine(reference, target);
                            let warped = affine::warp_image(target, &result.transform, in_rows, in_cols);
                            (0.0, 0.0, Some(warped))
                        } else {
                            (pc.dx, pc.dy, None)
                        }
                    })
                    .collect();
                for (i, (dx, dy, warped)) in aligned.into_iter().enumerate() {
                    offsets.push((dx, dy));
                    match warped {
                        Some(w) => frames.push(Cow::Owned(w)),
                        None => frames.push(Cow::Borrowed(images_ref[1 + i])),
                    }
                }
            }
            AlignmentMethod::Zncc => {
                let warped: Vec<Array2<f32>> = images_ref[1..]
                    .par_iter()
                    .map(|target| {
                        let result = affine::align_channel_affine(reference, target);
                        affine::warp_image(target, &result.transform, in_rows, in_cols)
                    })
                    .collect();
                for w in warped {
                    offsets.push((0.0, 0.0));
                    frames.push(Cow::Owned(w));
                }
            }
        }
    } else {
        for target in &images_ref[1..] {
            offsets.push((0.0, 0.0));
            frames.push(Cow::Borrowed(*target));
        }
    }

    let (_, sigma, _) = drizzle_support(scale, pixfrac, config.kernel);
    let cells = |s: f64| ((2.0 * s).ceil() + 1.0).max(1.0);
    let cpp = match config.kernel {
        DrizzleKernel::Square => cells(pixfrac * scale * 0.5).powi(2),
        DrizzleKernel::Gaussian => cells(sigma * 3.0).powi(2),
        DrizzleKernel::Lanczos3 => 49.0,
    };
    const TARGET_BAND_CONTRIBS: f64 = 32_000_000.0;
    let contribs_per_out_row =
        (frames.len() as f64) * (in_cols as f64) * cpp / scale.max(1.0);
    let band_rows = ((TARGET_BAND_CONTRIBS / contribs_per_out_row.max(1.0)).floor() as usize)
        .clamp(16, out_rows.max(16));

    let mut img_data = vec![0.0f32; out_rows * out_cols];
    let mut wgt_data = vec![0.0f32; out_rows * out_cols];
    let mut rejected_pixels = 0u64;

    for band_start in (0..out_rows).step_by(band_rows) {
        let band_end = (band_start + band_rows).min(out_rows);
        let band_n = (band_end - band_start) * out_cols;

        let mut counts = vec![0u32; band_n];
        for (i, img) in frames.iter().enumerate() {
            let (dx, dy) = offsets[i];
            let rows = frame_contributions(
                img.as_ref(), -dx, -dy, scale, pixfrac, config.kernel, out_rows, out_cols,
                band_start, band_end,
            );
            for contribs in rows {
                for (idx, _, _) in contribs {
                    counts[idx] += 1;
                }
            }
        }

        let mut accumulator = DrizzleAccumulator::new(band_end - band_start, out_cols, &counts);
        drop(counts);

        for (i, img) in frames.iter().enumerate() {
            let (dx, dy) = offsets[i];
            accumulator.drizzle_frame(
                img.as_ref(), -dx, -dy, scale, pixfrac, config.kernel,
                out_rows, band_start, band_end,
            );
        }

        let (band_img, band_wgt, band_rej) = accumulator.finalize(
            config.sigma_low,
            config.sigma_high,
            config.sigma_iterations,
        );
        let off = band_start * out_cols;
        img_data[off..off + band_n].copy_from_slice(&band_img);
        wgt_data[off..off + band_n].copy_from_slice(&band_wgt);
        rejected_pixels += band_rej;
    }

    let image = Array2::from_shape_vec((out_rows, out_cols), img_data)
        .expect("drizzle output shape");
    let weight_map = Array2::from_shape_vec((out_rows, out_cols), wgt_data)
        .expect("drizzle weight shape");

    Ok(DrizzleResult {
        image,
        weight_map,
        frame_count: images_ref.len(),
        output_scale: scale,
        input_dims: (in_rows, in_cols),
        output_dims: (out_rows, out_cols),
        offsets,
        rejected_pixels,
    })
}
