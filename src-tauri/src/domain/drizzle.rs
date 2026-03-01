use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;
use std::fs::File;

use crate::domain::calibration::CalibrationConfig;
use crate::utils::mmap::extract_image_mmap;

#[derive(Debug, Clone)]
pub struct DrizzleConfig {
    pub scale: f64,
    pub pixfrac: f64,
    pub kernel: DrizzleKernel,
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub sigma_iterations: usize,
    pub align: bool,
}

impl Default for DrizzleConfig {
    fn default() -> Self {
        Self {
            scale: 2.0,
            pixfrac: 0.7,
            kernel: DrizzleKernel::Square,
            sigma_low: 3.0,
            sigma_high: 3.0,
            sigma_iterations: 5,
            align: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrizzleKernel {
    Square,
    Gaussian,
    Lanczos3,
}

#[derive(Debug, Clone)]
pub struct DrizzleResult {
    pub image: Array2<f32>,
    pub weight_map: Array2<f32>,
    pub frame_count: usize,
    pub output_scale: f64,
    pub input_dims: (usize, usize),
    pub output_dims: (usize, usize),
    pub offsets: Vec<(f64, f64)>,
    pub rejected_pixels: u64,
}

struct DrizzleAccumulator {
    data: Vec<Vec<f32>>,
    weights: Vec<f64>,
    out_rows: usize,
    out_cols: usize,
}

impl DrizzleAccumulator {
    fn new(out_rows: usize, out_cols: usize) -> Self {
        let n = out_rows * out_cols;
        Self {
            data: vec![Vec::new(); n],
            weights: vec![0.0; n],
            out_rows,
            out_cols,
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
    ) {
        let (in_rows, in_cols) = frame.dim();

        for iy in 0..in_rows {
            for ix in 0..in_cols {
                let val = frame[[iy, ix]];
                if !val.is_finite() {
                    continue;
                }

                let cx = (ix as f64 + dx) * scale;
                let cy = (iy as f64 + dy) * scale;

                let half = pixfrac * scale * 0.5;
                let ox_min = ((cx - half).floor() as i64).max(0) as usize;
                let ox_max = ((cx + half).ceil() as i64).min(self.out_cols as i64 - 1) as usize;
                let oy_min = ((cy - half).floor() as i64).max(0) as usize;
                let oy_max = ((cy + half).ceil() as i64).min(self.out_rows as i64 - 1) as usize;

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
                            let idx = oy * self.out_cols + ox;
                            self.data[idx].push(val);
                            self.weights[idx] += w;
                        }
                    }
                }
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

        let results: Vec<(f32, f32, u64)> = (0..n)
            .into_par_iter()
            .map(|i| {
                let vals = &self.data[i];
                if vals.is_empty() {
                    return (0.0, 0.0, 0);
                }
                if vals.len() == 1 {
                    return (vals[0], self.weights[i] as f32, 0);
                }

                let mut active: Vec<f32> = vals.clone();
                let mut rejected = 0u64;

                for _ in 0..sigma_iterations {
                    if active.len() < 2 {
                        break;
                    }
                    let n_f = active.len() as f64;
                    let mean = active.iter().map(|v| *v as f64).sum::<f64>() / n_f;
                    let var = active.iter().map(|v| {
                        let d = *v as f64 - mean;
                        d * d
                    }).sum::<f64>() / (n_f - 1.0).max(1.0);
                    let sigma = (var.sqrt().max(1e-10)) as f32;
                    let mean_f = mean as f32;

                    let before = active.len();
                    active.retain(|&v| {
                        let dev = v - mean_f;
                        dev >= -sigma_low * sigma && dev <= sigma_high * sigma
                    });
                    let removed = before - active.len();
                    rejected += removed as u64;
                    if removed == 0 {
                        break;
                    }
                }

                if active.is_empty() {
                    let mean = vals.iter().map(|v| *v as f64).sum::<f64>() / vals.len() as f64;
                    return (mean as f32, self.weights[i] as f32, rejected);
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

fn compute_subpixel_offset(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    search_radius: i32,
) -> (f64, f64) {
    let (rows, cols) = reference.dim();
    if target.dim() != (rows, cols) {
        return (0.0, 0.0);
    }

    let cy = rows / 2;
    let cx = cols / 2;
    let region = rows.min(cols).min(256) / 2;
    let y_start = cy.saturating_sub(region);
    let y_end = (cy + region).min(rows);
    let x_start = cx.saturating_sub(region);
    let x_end = (cx + region).min(cols);

    let shifts: Vec<(i32, i32)> = (-search_radius..=search_radius)
        .flat_map(|dy| (-search_radius..=search_radius).map(move |dx| (dy, dx)))
        .collect();

    let scores: Vec<(i32, i32, f64)> = shifts
        .par_iter()
        .map(|&(dy, dx)| {
            let mut r_sum = 0.0f64;
            let mut t_sum = 0.0f64;
            let mut count = 0u32;

            for y in y_start..y_end {
                let ty = y as i32 + dy;
                if ty < 0 || ty >= rows as i32 { continue; }
                for x in x_start..x_end {
                    let tx = x as i32 + dx;
                    if tx < 0 || tx >= cols as i32 { continue; }
                    let rv = reference[[y, x]] as f64;
                    let tv = target[[ty as usize, tx as usize]] as f64;
                    if rv.is_finite() && tv.is_finite() && rv.abs() > 1e-7 && tv.abs() > 1e-7 {
                        r_sum += rv;
                        t_sum += tv;
                        count += 1;
                    }
                }
            }

            if count < 10 {
                return (dy, dx, f64::NEG_INFINITY);
            }

            let r_mean = r_sum / count as f64;
            let t_mean = t_sum / count as f64;
            let mut num = 0.0f64;
            let mut r_var = 0.0f64;
            let mut t_var = 0.0f64;

            for y in y_start..y_end {
                let ty = y as i32 + dy;
                if ty < 0 || ty >= rows as i32 { continue; }
                for x in x_start..x_end {
                    let tx = x as i32 + dx;
                    if tx < 0 || tx >= cols as i32 { continue; }
                    let rv = reference[[y, x]] as f64;
                    let tv = target[[ty as usize, tx as usize]] as f64;
                    if rv.is_finite() && tv.is_finite() && rv.abs() > 1e-7 && tv.abs() > 1e-7 {
                        let rd = rv - r_mean;
                        let td = tv - t_mean;
                        num += rd * td;
                        r_var += rd * rd;
                        t_var += td * td;
                    }
                }
            }

            let score = if r_var > 0.0 && t_var > 0.0 {
                num / (r_var * t_var).sqrt()
            } else {
                f64::NEG_INFINITY
            };
            (dy, dx, score)
        })
        .collect();

    let best = scores.iter().copied().fold(
        (0i32, 0i32, f64::NEG_INFINITY),
        |a, b| if b.2 > a.2 { b } else { a },
    );

    let (by, bx, _) = best;
    let mut top3: Vec<(i32, i32, f64)> = scores.clone();
    top3.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    if top3.len() >= 3 && top3[0].2 > f64::NEG_INFINITY {
        let sub_dy = quadratic_peak(
            &scores, by, bx, true, search_radius,
        ).unwrap_or(by as f64);
        let sub_dx = quadratic_peak(
            &scores, by, bx, false, search_radius,
        ).unwrap_or(bx as f64);
        (sub_dx, sub_dy)
    } else {
        (bx as f64, by as f64)
    }
}

fn quadratic_peak(
    scores: &[(i32, i32, f64)],
    cy: i32,
    cx: i32,
    axis_y: bool,
    search_radius: i32,
) -> Option<f64> {
    let find = |dy: i32, dx: i32| -> Option<f64> {
        scores.iter().find(|s| s.0 == dy && s.1 == dx).map(|s| s.2)
    };

    let c_score = find(cy, cx)?;

    let (prev_score, next_score, center) = if axis_y {
        if cy <= -search_radius || cy >= search_radius { return Some(cy as f64); }
        let p = find(cy - 1, cx)?;
        let n = find(cy + 1, cx)?;
        (p, n, cy as f64)
    } else {
        if cx <= -search_radius || cx >= search_radius { return Some(cx as f64); }
        let p = find(cy, cx - 1)?;
        let n = find(cy, cx + 1)?;
        (p, n, cx as f64)
    };

    if prev_score.is_infinite() || next_score.is_infinite() || c_score.is_infinite() {
        return Some(center);
    }

    let denom = 2.0 * (2.0 * c_score - prev_score - next_score);
    if denom.abs() < 1e-15 {
        return Some(center);
    }

    let offset = (prev_score - next_score) / denom;
    Some(center + offset.clamp(-0.5, 0.5))
}

fn load_fits_image(path: &str) -> Result<Array2<f32>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open {}", path))?;
    let result = extract_image_mmap(&file)
        .with_context(|| format!("Failed to load {}", path))?;
    Ok(result.image)
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

    let reference = &images[0];
    let (in_rows, in_cols) = reference.dim();

    for (i, img) in images.iter().enumerate().skip(1) {
        if img.dim() != (in_rows, in_cols) {
            bail!(
                "Frame {} dimension mismatch: expected ({}, {}), got {:?}",
                i, in_rows, in_cols, img.dim()
            );
        }
    }

    let scale = config.scale.clamp(1.0, 4.0);
    let pixfrac = config.pixfrac.clamp(0.1, 1.0);
    let out_rows = (in_rows as f64 * scale).ceil() as usize;
    let out_cols = (in_cols as f64 * scale).ceil() as usize;

    let mut offsets: Vec<(f64, f64)> = Vec::with_capacity(images.len());
    offsets.push((0.0, 0.0));

    if config.align {
        let search_radius = 50i32;
        for i in 1..images.len() {
            let (dx, dy) = compute_subpixel_offset(reference, &images[i], search_radius);
            offsets.push((dx, dy));
        }
    } else {
        for _ in 1..images.len() {
            offsets.push((0.0, 0.0));
        }
    }

    let mut accumulator = DrizzleAccumulator::new(out_rows, out_cols);

    for (i, img) in images.iter().enumerate() {
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
        frame_count: images.len(),
        output_scale: scale,
        input_dims: (in_rows, in_cols),
        output_dims: (out_rows, out_cols),
        offsets,
        rejected_pixels,
    })
}

pub fn drizzle_from_paths(
    paths: &[String],
    config: &DrizzleConfig,
    calibration: Option<&CalibrationConfig>,
) -> Result<DrizzleResult> {
    if paths.is_empty() {
        bail!("No image paths provided");
    }

    let mut images: Vec<Array2<f32>> = Vec::with_capacity(paths.len());
    for path in paths {
        let mut img = load_fits_image(path)?;
        if let Some(cal) = calibration {
            img = crate::domain::calibration::calibrate_image(&img, cal);
        }
        images.push(img);
    }

    drizzle_stack(&images, config)
}
