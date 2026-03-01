use std::fs::File;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::domain::calibration::CalibrationConfig;
use crate::utils::mmap::extract_image_mmap;

fn load_fits_image(path: &str) -> Result<Array2<f32>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open {}", path))?;
    let result = extract_image_mmap(&file)
        .with_context(|| format!("Failed to load {}", path))?;
    Ok(result.image)
}

#[derive(Debug, Clone)]
pub struct StackConfig {
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub max_iterations: usize,
    pub align: bool,
}

impl Default for StackConfig {
    fn default() -> Self {
        Self {
            sigma_low: 3.0,
            sigma_high: 3.0,
            max_iterations: 5,
            align: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StackResult {
    pub image: Array2<f32>,
    pub frame_count: usize,
    pub rejected_pixels: u64,
    pub offsets: Vec<(i32, i32)>,
}

fn sigma_clip_combine(
    values: &mut Vec<f32>,
    sigma_low: f32,
    sigma_high: f32,
    max_iter: usize,
) -> (f32, u32) {
    if values.is_empty() {
        return (0.0, 0);
    }
    if values.len() == 1 {
        return (values[0], 0);
    }

    let mut rejected = 0u32;
    let mut active: Vec<f32> = values.clone();

    for _ in 0..max_iter {
        if active.len() < 2 {
            break;
        }

        let n = active.len() as f64;
        let mean = active.iter().map(|v| *v as f64).sum::<f64>() / n;

        let variance = active
            .iter()
            .map(|v| {
                let d = *v as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / (n - 1.0).max(1.0);
        let sigma = variance.sqrt().max(1e-10) as f32;
        let mean_f = mean as f32;

        let before = active.len();
        active.retain(|&v| {
            let dev = v - mean_f;
            dev >= -sigma_low * sigma && dev <= sigma_high * sigma
        });

        let removed = before - active.len();
        rejected += removed as u32;

        if removed == 0 {
            break;
        }
    }

    if active.is_empty() {
        let sum: f64 = values.iter().map(|v| *v as f64).sum();
        return ((sum / values.len() as f64) as f32, rejected);
    }

    let mean = active.iter().map(|v| *v as f64).sum::<f64>() / active.len() as f64;
    (mean as f32, rejected)
}

fn compute_offset(
    reference: &Array2<f32>,
    target: &Array2<f32>,
    search_radius: i32,
) -> (i32, i32) {
    let (rows, cols) = reference.dim();
    if target.dim() != (rows, cols) {
        return (0, 0);
    }

    let cy = rows / 2;
    let cx = cols / 2;
    let region = rows.min(cols).min(256) / 2;
    let y_start = cy.saturating_sub(region);
    let y_end = (cy + region).min(rows);
    let x_start = cx.saturating_sub(region);
    let x_end = (cx + region).min(cols);

    let mut best_score = f64::MIN;
    let mut best_dy = 0i32;
    let mut best_dx = 0i32;

    for dy in -search_radius..=search_radius {
        for dx in -search_radius..=search_radius {
            let mut sum_prod = 0.0f64;
            let mut sum_r2 = 0.0f64;
            let mut sum_t2 = 0.0f64;
            let mut count = 0u32;

            for y in y_start..y_end {
                let ty = y as i32 + dy;
                if ty < 0 || ty >= rows as i32 {
                    continue;
                }
                for x in x_start..x_end {
                    let tx = x as i32 + dx;
                    if tx < 0 || tx >= cols as i32 {
                        continue;
                    }
                    let r = reference[[y, x]] as f64;
                    let t = target[[ty as usize, tx as usize]] as f64;
                    if r.is_finite() && t.is_finite() {
                        sum_prod += r * t;
                        sum_r2 += r * r;
                        sum_t2 += t * t;
                        count += 1;
                    }
                }
            }

            if count > 0 {
                let denom = (sum_r2 * sum_t2).sqrt();
                let score = if denom > 1e-10 {
                    sum_prod / denom
                } else {
                    0.0
                };
                if score > best_score {
                    best_score = score;
                    best_dy = dy;
                    best_dx = dx;
                }
            }
        }
    }

    (best_dy, best_dx)
}

fn shift_image(image: &Array2<f32>, dy: i32, dx: i32) -> Array2<f32> {
    let (rows, cols) = image.dim();
    let mut shifted = Array2::<f32>::from_elem((rows, cols), f32::NAN);

    for y in 0..rows {
        let sy = y as i32 - dy;
        if sy < 0 || sy >= rows as i32 {
            continue;
        }
        for x in 0..cols {
            let sx = x as i32 - dx;
            if sx < 0 || sx >= cols as i32 {
                continue;
            }
            shifted[[y, x]] = image[[sy as usize, sx as usize]];
        }
    }

    shifted
}

pub fn stack_images(
    images: &[Array2<f32>],
    config: &StackConfig,
) -> Result<StackResult> {
    if images.is_empty() {
        bail!("No images to stack");
    }

    let reference = &images[0];
    let (rows, cols) = reference.dim();
    let n = images.len();

    let mut aligned: Vec<Array2<f32>> = Vec::with_capacity(n);
    let mut offsets: Vec<(i32, i32)> = Vec::with_capacity(n);

    aligned.push(reference.clone());
    offsets.push((0, 0));

    let search_radius = 50i32;

    for i in 1..n {
        if images[i].dim() != (rows, cols) {
            bail!(
                "Image {} dimension mismatch: expected ({}, {}), got {:?}",
                i,
                rows,
                cols,
                images[i].dim()
            );
        }

        if config.align {
            let (dy, dx) = compute_offset(reference, &images[i], search_radius);
            offsets.push((dy, dx));
            aligned.push(shift_image(&images[i], dy, dx));
        } else {
            offsets.push((0, 0));
            aligned.push(images[i].clone());
        }
    }

    let npix = rows * cols;
    let sigma_low = config.sigma_low;
    let sigma_high = config.sigma_high;
    let max_iter = config.max_iterations;

    let pixel_results: Vec<(f32, u32)> = (0..npix)
        .into_par_iter()
        .map(|i| {
            let y = i / cols;
            let x = i % cols;
            let mut vals: Vec<f32> = aligned
                .iter()
                .map(|img| img[[y, x]])
                .filter(|v| v.is_finite())
                .collect();

            sigma_clip_combine(&mut vals, sigma_low, sigma_high, max_iter)
        })
        .collect();

    let mut result_data = Vec::with_capacity(npix);
    let mut total_rejected = 0u64;

    for (val, rej) in pixel_results {
        result_data.push(val);
        total_rejected += rej as u64;
    }

    Ok(StackResult {
        image: Array2::from_shape_vec((rows, cols), result_data)
            .context("Failed to reshape stacked image")?,
        frame_count: n,
        rejected_pixels: total_rejected,
        offsets,
    })
}

pub fn stack_from_paths(
    paths: &[String],
    config: &StackConfig,
    calibration: Option<&CalibrationConfig>,
) -> Result<StackResult> {
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

    stack_images(&images, config)
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
    fn test_shift_image() {
        let img = Array2::from_shape_vec(
            (4, 4),
            vec![
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0,
                14.0, 15.0, 16.0,
            ],
        )
            .unwrap();

        let shifted = shift_image(&img, 1, 1);
        assert!((shifted[[1, 1]] - 1.0).abs() < 1e-6);
        assert!((shifted[[2, 2]] - 6.0).abs() < 1e-6);
        assert!(shifted[[0, 0]].is_nan());
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
        };

        let result = stack_images(&images, &config).unwrap();
        assert!((result.image[[2, 2]] - 100.0).abs() < 1.0);
        assert!(result.rejected_pixels > 0);
    }

    #[test]
    fn test_compute_offset_no_shift() {
        let img = Array2::from_shape_vec(
            (64, 64),
            (0..4096)
                .map(|i| (i as f32).sin() * 100.0 + 500.0)
                .collect(),
        )
            .unwrap();

        let (dy, dx) = compute_offset(&img, &img, 10);
        assert_eq!(dy, 0);
        assert_eq!(dx, 0);
    }

    #[test]
    fn test_compute_offset_known_shift() {
        let base = Array2::from_shape_vec(
            (64, 64),
            (0..4096)
                .map(|i| {
                    let y = i / 64;
                    let x = i % 64;
                    ((y as f32 * 0.1).sin() * (x as f32 * 0.1).cos() * 1000.0) + 500.0
                })
                .collect(),
        )
            .unwrap();

        let shifted = shift_image(&base, 3, 5);
        let (dy, dx) = compute_offset(&base, &shifted, 10);
        assert_eq!(dy, 3);
        assert_eq!(dx, 5);
    }
}