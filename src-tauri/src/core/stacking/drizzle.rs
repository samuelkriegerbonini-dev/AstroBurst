use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;

pub use crate::types::stacking::{AlignmentMethod, DrizzleConfig, DrizzleKernel, DrizzleResult};

use crate::core::alignment::phase_correlation;
use crate::core::stacking::align;
use crate::types::constants::MAD_TO_SIGMA;

struct DrizzleAccumulator {
    storage: Vec<f32>,
    counts: Vec<u16>,
    weights: Vec<f64>,
    max_per_pixel: usize,
    out_rows: usize,
    out_cols: usize,
}

impl DrizzleAccumulator {
    fn new(out_rows: usize, out_cols: usize, n_frames: usize) -> Self {
        let n = out_rows * out_cols;
        let max_per_pixel = (n_frames * 2).max(4);
        Self {
            storage: vec![0.0f32; n * max_per_pixel],
            counts: vec![0u16; n],
            weights: vec![0.0; n],
            max_per_pixel,
            out_rows,
            out_cols,
        }
    }

    #[inline]
    fn push(&mut self, idx: usize, val: f32, w: f64) {
        let count = self.counts[idx] as usize;
        if count < self.max_per_pixel {
            self.storage[idx * self.max_per_pixel + count] = val;
            self.counts[idx] += 1;
        }
        self.weights[idx] += w;
    }

    fn drizzle_frame(
        &mut self,
        frame: &Array2<f32>,
        dx: f64,
        dy: f64,
        scale: f64,
        pixfrac: f64,
        kernel: DrizzleKernel,
    ) {
        let (in_rows, in_cols) = frame.dim();
        let src = frame.as_slice().expect("contiguous");
        let out_rows = self.out_rows;
        let out_cols = self.out_cols;

        let row_contribs: Vec<Vec<(usize, f32, f64)>> = (0..in_rows)
            .into_par_iter()
            .map(|iy| {
                let mut contribs = Vec::new();
                let row_base = iy * in_cols;
                for ix in 0..in_cols {
                    let val = src[row_base + ix];
                    if !val.is_finite() {
                        continue;
                    }

                    let cx = (ix as f64 + dx) * scale;
                    let cy = (iy as f64 + dy) * scale;

                    let half = pixfrac * scale * 0.5;
                    let ox_min = ((cx - half).floor() as i64).max(0) as usize;
                    let ox_max = ((cx + half).ceil() as i64).min(out_cols as i64 - 1) as usize;
                    let oy_min = ((cy - half).floor() as i64).max(0) as usize;
                    let oy_max = ((cy + half).ceil() as i64).min(out_rows as i64 - 1) as usize;

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
                                    let sigma = half.max(0.5);
                                    (-dist2 / (2.0 * sigma * sigma)).exp()
                                }
                                DrizzleKernel::Lanczos3 => {
                                    let ddx = (ox as f64 + 0.5 - cx).abs();
                                    let ddy = (oy as f64 + 0.5 - cy).abs();
                                    lanczos3(ddx) * lanczos3(ddy)
                                }
                            };

                            if w > 1e-12 {
                                let idx = oy * out_cols + ox;
                                contribs.push((idx, val, w));
                            }
                        }
                    }
                }
                contribs
            })
            .collect();

        for contribs in row_contribs {
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
    ) -> (Array2<f32>, Array2<f32>, u64) {
        let n = self.out_rows * self.out_cols;
        let mpp = self.max_per_pixel;

        let results: Vec<(f32, f32, u64)> = (0..n)
            .into_par_iter()
            .map(|i| {
                let count = self.counts[i] as usize;
                if count == 0 {
                    return (0.0, 0.0, 0);
                }
                if count == 1 {
                    return (self.storage[i * mpp], self.weights[i] as f32, 0);
                }

                let base = i * mpp;
                let mut active: Vec<f32> = self.storage[base..base + count].to_vec();
                let mut rejected = 0u64;

                for _ in 0..sigma_iterations {
                    if active.len() < 3 {
                        break;
                    }

                    let mid = active.len() / 2;
                    active.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let median = active[mid];

                    let mut devs: Vec<f32> = active.iter().map(|v| (v - median).abs()).collect();
                    let dmid = devs.len() / 2;
                    devs.select_nth_unstable_by(dmid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let sigma = (devs[dmid] as f64 * MAD_TO_SIGMA).max(1e-10) as f32;

                    let before = active.len();
                    active.retain(|&v| {
                        let dev = v - median;
                        dev >= -sigma_low * sigma && dev <= sigma_high * sigma
                    });
                    let removed = before - active.len();
                    rejected += removed as u64;
                    if removed == 0 {
                        break;
                    }
                }

                if active.is_empty() {
                    let sum: f64 = self.storage[base..base + count]
                        .iter()
                        .map(|v| *v as f64)
                        .sum();
                    return ((sum / count as f64) as f32, self.weights[i] as f32, rejected);
                }

                let mean = active.iter().map(|v| *v as f64).sum::<f64>() / active.len() as f64;
                (mean as f32, self.weights[i] as f32, rejected)
            })
            .collect();

        let mut img_data = Vec::with_capacity(n);
        let mut wgt_data = Vec::with_capacity(n);
        let mut total_rejected = 0u64;

        for (val, wgt, rej) in results {
            img_data.push(val);
            wgt_data.push(wgt);
            total_rejected += rej;
        }

        let image = Array2::from_shape_vec((self.out_rows, self.out_cols), img_data).unwrap();
        let weights = Array2::from_shape_vec((self.out_rows, self.out_cols), wgt_data).unwrap();
        (image, weights, total_rejected)
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

    if config.align {
        match config.alignment_method {
            AlignmentMethod::PhaseCorrelation => {
                let computed: Vec<(f64, f64)> = images_ref[1..]
                    .par_iter()
                    .map(|target| {
                        let result = phase_correlation::phase_correlate(reference, target);
                        if phase_correlation::is_low_confidence(result.confidence) {
                            let (dy, dx) = align::compute_subpixel_offset(reference, target, 50);
                            (dx, dy)
                        } else {
                            (result.dx, result.dy)
                        }
                    })
                    .collect();
                offsets.extend(computed);
            }
            AlignmentMethod::Zncc => {
                let search_radius = 50i32;
                for i in 1..images_ref.len() {
                    let (dy, dx) = align::compute_subpixel_offset(
                        reference,
                        images_ref[i],
                        search_radius,
                    );
                    offsets.push((dx, dy));
                }
            }
        }
    } else {
        for _ in 1..images_ref.len() {
            offsets.push((0.0, 0.0));
        }
    }

    let mut accumulator = DrizzleAccumulator::new(out_rows, out_cols, images_ref.len());

    for (i, img) in images_ref.iter().enumerate() {
        let (dx, dy) = offsets[i];
        accumulator.drizzle_frame(img, -dx, -dy, scale, pixfrac, config.kernel);
    }

    let (image, weight_map, rejected_pixels) = accumulator.finalize(
        config.sigma_low,
        config.sigma_high,
        config.sigma_iterations,
    );

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
