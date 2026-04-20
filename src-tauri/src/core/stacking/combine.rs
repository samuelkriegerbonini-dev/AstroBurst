use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

pub use crate::types::stacking::{StackConfig, StackResult};
use crate::math::median::f32_cmp;
use crate::types::compose::AlignMethod;
use crate::types::constants::MAD_TO_SIGMA;

use crate::core::stacking::align;

pub fn sigma_clip_combine(
    values: &mut Vec<f32>,
    sigma_low: f32,
    sigma_high: f32,
    max_iter: usize,
) -> (f32, u32) {
    let n_orig = values.len();
    if n_orig == 0 {
        return (0.0, 0);
    }
    if n_orig == 1 {
        return (values[0], 0);
    }

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

            let mut devs: Vec<f32> =
                values[..len].iter().map(|v| (v - med).abs()).collect();
            let dmid = devs.len() / 2;
            devs.select_nth_unstable_by(dmid, |a, b| f32_cmp(a, b));
            let mad = devs[dmid];
            let sig = (mad as f64 * MAD_TO_SIGMA).max(1e-10) as f32;
            (med, sig)
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
            (mean as f32, variance.sqrt().max(1e-10) as f32)
        };

        last_center = center;

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

    let crop = |img: &Array2<f32>| -> Array2<f32> {
        let (r, c) = img.dim();
        if r == min_rows && c == min_cols {
            return img.clone();
        }
        img.slice(ndarray::s![..min_rows, ..min_cols]).to_owned()
    };

    let ref_cropped = crop(&images[0]);

    let mut aligned: Vec<Array2<f32>> = Vec::with_capacity(n);
    let mut offsets: Vec<(i32, i32)> = Vec::with_capacity(n);

    aligned.push(ref_cropped.clone());
    offsets.push((0, 0));

    for i in 1..n {
        let cropped = crop(&images[i]);

        if config.align {
            let result = align::align_pair_with_label(
                &ref_cropped,
                &cropped,
                AlignMethod::PhaseCorrelation,
                min_rows,
                min_cols,
                &format!("frame_{}", i),
            )?;
            let dy = result.offset.0.round() as i32;
            let dx = result.offset.1.round() as i32;
            offsets.push((dy, dx));
            aligned.push(result.aligned);
        } else {
            offsets.push((0, 0));
            aligned.push(cropped);
        }
    }

    let rows = min_rows;
    let cols = min_cols;
    let npix = rows * cols;
    let sigma_low = config.sigma_low;
    let sigma_high = config.sigma_high;
    let max_iter = config.max_iterations;

    let aligned_slices: Vec<&[f32]> = aligned
        .iter()
        .map(|img| img.as_slice().expect("contiguous"))
        .collect();

    let mut result_data = vec![0.0f32; npix];
    let total_rejected = AtomicU64::new(0);

    result_data
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(y, row_buf)| {
            let mut vals: Vec<f32> = Vec::with_capacity(aligned_slices.len());
            let base = y * cols;
            let mut local_rejected: u64 = 0;
            for x in 0..cols {
                vals.clear();
                let idx = base + x;
                for s in &aligned_slices {
                    let v = s[idx];
                    if v.is_finite() {
                        vals.push(v);
                    }
                }
                let (val, rej) =
                    sigma_clip_combine(&mut vals, sigma_low, sigma_high, max_iter);
                row_buf[x] = val;
                local_rejected += rej as u64;
            }
            total_rejected.fetch_add(local_rejected, Ordering::Relaxed);
        });

    let rejected_pixels = total_rejected.load(Ordering::Relaxed);

    Ok(StackResult {
        image: Array2::from_shape_vec((rows, cols), result_data)
            .context("Failed to reshape stacked image")?,
        frame_count: n,
        rejected_pixels,
        offsets,
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
    fn test_sigma_clip_empty() {
        let mut vals: Vec<f32> = vec![];
        let (mean, rejected) = sigma_clip_combine(&mut vals, 3.0, 3.0, 5);
        assert_eq!(mean, 0.0);
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
        };

        let result = stack_images(&images, &config).unwrap();
        assert!((result.image[[2, 2]] - 100.0).abs() < 1.0);
        assert!(result.rejected_pixels > 0);
    }
}
