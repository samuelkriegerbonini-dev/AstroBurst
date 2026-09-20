use std::borrow::Cow;

use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;

pub use crate::types::stacking::{AlignmentMethod, DrizzleConfig, DrizzleKernel, DrizzleResult};

use crate::core::alignment::affine;
use crate::core::alignment::phase_correlation;
use crate::core::imaging::boundary::clamp_index;
use crate::core::stacking::combine::{reject_and_combine_with, KernelScratch, Sample};
use crate::types::stacking::{RejectionMethod, RejectionParams};

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

    fn finalize_into(&self, img: &mut [f32], wgt: &mut [f32]) {
        for ((&v, &w), out) in self.vsum.iter().zip(self.wsum.iter()).zip(img.iter_mut()) {
            *out = if w > 1e-6 { (v / w) as f32 } else { 0.0 };
        }
        for (&w, out) in self.wsum.iter().zip(wgt.iter_mut()) {
            *out = w.max(0.0) as f32;
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

fn drizzle_band_rows(out_rows: usize, threads: usize) -> usize {
    let max_rows = out_rows.max(1);
    out_rows
        .div_ceil(4 * threads.max(1))
        .clamp(64.min(max_rows), max_rows)
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

    let reference: &Array2<f32> = images_ref[0].as_ref();
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
                        let target = target.as_ref();
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
                        None => frames.push(Cow::Borrowed(images_ref[1 + i].as_ref())),
                    }
                }
            }
            AlignmentMethod::Zncc => {
                let warped: Vec<Array2<f32>> = images_ref[1..]
                    .par_iter()
                    .map(|target| {
                        let target = target.as_ref();
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
            frames.push(Cow::Borrowed(target.as_ref()));
        }
    }

    let band_rows = drizzle_band_rows(out_rows, rayon::current_num_threads());

    let keep_mask = if config.rejection == RejectionMethod::None {
        None
    } else {
        Some(KeepMask::build(&frames, &offsets, in_rows, in_cols, &config.rejection_params()))
    };
    let rejected_pixels = keep_mask.as_ref().map_or(0, |mask| mask.rejected);

    let mut img_data = vec![0.0f32; out_rows * out_cols];
    let mut wgt_data = vec![0.0f32; out_rows * out_cols];

    let chunk = (band_rows * out_cols).max(1);
    img_data
        .par_chunks_mut(chunk)
        .zip(wgt_data.par_chunks_mut(chunk))
        .enumerate()
        .for_each(|(b, (img_chunk, wgt_chunk))| {
            let band_start = b * band_rows;
            let band_end = (band_start + band_rows).min(out_rows);
            let mut accumulator = DrizzleAccumulator::new(band_end - band_start, out_cols);
            for (i, img) in frames.iter().enumerate() {
                let (dx, dy) = offsets[i];
                accumulator.drizzle_frame(
                    img.as_ref(), -dx, -dy, scale, pixfrac, config.kernel,
                    out_rows, band_start, band_end,
                    keep_mask.as_ref().map(|mask| (mask, i)),
                );
            }
            accumulator.finalize_into(img_chunk, wgt_chunk);
        });

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
        for (&v, &w) in result.image.iter().zip(result.weight_map.iter()) {
            if w > 1e-6 {
                assert!((v - 5.0).abs() < 1e-4, "pixel {} != 5.0 (w={})", v, w);
            }
        }
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

        let mask = KeepMask::build(&frames, &offsets, rows, cols, &params);
        assert_eq!(mask.rejected, 1, "shift ignored: {} rejections", mask.rejected);
        assert!(!mask.keeps(1, 5, 6));
        assert!(mask.keeps(1, 5, 5));
        assert!(mask.keeps(0, 5, 5));
        assert!(mask.keeps(1, 0, 0));

        let unshifted = vec![(0.0, 0.0); 5];
        let naive = KeepMask::build(&frames, &unshifted, rows, cols, &params);
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
        for ((oy, ox), &w) in result.weight_map.indexed_iter() {
            assert!(w > 1e-6, "zero weight at ({}, {})", oy, ox);
            let v = result.image[[oy, ox]];
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
            assert!((result.weight_map[[oy, ox]] - 2.0).abs() < 1e-6);
        }
    }
}
