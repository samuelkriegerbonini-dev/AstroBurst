use std::borrow::Cow;
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
    let mut scratch = Vec::with_capacity(values.len());
    sigma_clip_combine_with(values, &mut scratch, sigma_low, sigma_high, max_iter)
}

fn sigma_clip_combine_with(
    values: &mut Vec<f32>,
    scratch: &mut Vec<f32>,
    sigma_low: f32,
    sigma_high: f32,
    max_iter: usize,
) -> (f32, u32) {
    let n_orig = values.len();
    if n_orig == 0 {
        return (f32::NAN, 0);
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

            scratch.clear();
            scratch.extend(values[..len].iter().map(|v| (v - med).abs()));
            let dmid = scratch.len() / 2;
            scratch.select_nth_unstable_by(dmid, |a, b| f32_cmp(a, b));
            let mad = scratch[dmid];
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

pub fn sigma_clip_combine_weighted(
    vals: &mut Vec<(f32, f32)>,
    sigma_low: f32,
    sigma_high: f32,
    max_iter: usize,
) -> (f32, u32) {
    let mut scratch = Vec::with_capacity(vals.len());
    sigma_clip_combine_weighted_with(vals, &mut scratch, sigma_low, sigma_high, max_iter)
}

fn sigma_clip_combine_weighted_with(
    vals: &mut Vec<(f32, f32)>,
    scratch: &mut Vec<f32>,
    sigma_low: f32,
    sigma_high: f32,
    max_iter: usize,
) -> (f32, u32) {
    let n_orig = vals.len();
    if n_orig == 0 {
        return (f32::NAN, 0);
    }
    if n_orig == 1 {
        return (vals[0].0, 0);
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
            vals[..len].select_nth_unstable_by(mid, |a, b| f32_cmp(&a.0, &b.0));
            let med = vals[mid].0;
            scratch.clear();
            scratch.extend(vals[..len].iter().map(|p| (p.0 - med).abs()));
            let dmid = scratch.len() / 2;
            scratch.select_nth_unstable_by(dmid, |a, b| f32_cmp(a, b));
            let mad = scratch[dmid];
            let sig = (mad as f64 * MAD_TO_SIGMA).max(1e-10) as f32;
            (med, sig)
        } else {
            let nf = len as f64;
            let mean = vals[..len].iter().map(|p| p.0 as f64).sum::<f64>() / nf;
            let variance = vals[..len]
                .iter()
                .map(|p| {
                    let d = p.0 as f64 - mean;
                    d * d
                })
                .sum::<f64>()
                / (nf - 1.0).max(1.0);
            (mean as f32, variance.sqrt().max(1e-10) as f32)
        };

        last_center = center;

        let lo = -sigma_low * sigma;
        let hi = sigma_high * sigma;
        let mut write = 0;
        for read in 0..len {
            let dev = vals[read].0 - center;
            if dev >= lo && dev <= hi {
                vals[write] = vals[read];
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

    let mut vsum = 0.0f64;
    let mut wsum = 0.0f64;
    for &(v, w) in &vals[..len] {
        let wf = w as f64;
        vsum += v as f64 * wf;
        wsum += wf;
    }
    let mean = if wsum > 1e-12 {
        (vsum / wsum) as f32
    } else {
        (vals[..len].iter().map(|p| p.0 as f64).sum::<f64>() / len as f64) as f32
    };
    (mean, rejected)
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
                AlignMethod::PhaseCorrelation,
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
    let sigma_low = config.sigma_low;
    let sigma_high = config.sigma_high;
    let max_iter = config.max_iterations;

    let aligned_slices: Vec<&[f32]> = aligned
        .iter()
        .map(|img| img.as_slice().expect("contiguous"))
        .collect();

    let weights_f32: Option<Vec<f32>> = config
        .weights
        .as_ref()
        .filter(|w| w.len() == n)
        .map(|w| w.iter().map(|&x| x as f32).collect());

    let mut result_data = vec![0.0f32; npix];
    let total_rejected = AtomicU64::new(0);

    result_data
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(y, row_buf)| {
            let base = y * cols;
            let mut local_rejected: u64 = 0;
            let mut scratch: Vec<f32> = Vec::with_capacity(aligned_slices.len());
            if let Some(ref weights) = weights_f32 {
                let mut wvals: Vec<(f32, f32)> = Vec::with_capacity(aligned_slices.len());
                for x in 0..cols {
                    wvals.clear();
                    let idx = base + x;
                    for (i, s) in aligned_slices.iter().enumerate() {
                        let v = s[idx];
                        if v.is_finite() {
                            wvals.push((v, weights[i]));
                        }
                    }
                    let (val, rej) = sigma_clip_combine_weighted_with(
                        &mut wvals, &mut scratch, sigma_low, sigma_high, max_iter,
                    );
                    row_buf[x] = val;
                    local_rejected += rej as u64;
                }
            } else {
                let mut vals: Vec<f32> = Vec::with_capacity(aligned_slices.len());
                for x in 0..cols {
                    vals.clear();
                    let idx = base + x;
                    for s in &aligned_slices {
                        let v = s[idx];
                        if v.is_finite() {
                            vals.push(v);
                        }
                    }
                    let (val, rej) = sigma_clip_combine_with(
                        &mut vals, &mut scratch, sigma_low, sigma_high, max_iter,
                    );
                    row_buf[x] = val;
                    local_rejected += rej as u64;
                }
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
            weights: None,
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
            ..Default::default()
        };
        let result = stack_images(&[dark, bright], &config).unwrap();
        assert!((result.image[[0, 0]] - 17.5).abs() < 1e-3);
    }
}
