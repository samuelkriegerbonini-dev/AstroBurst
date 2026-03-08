use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

pub use crate::types::stacking::{StackConfig, StackResult};

use crate::core::stacking::align;

pub fn sigma_clip_combine(
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

pub fn stack_images(
    images: &[Array2<f32>],
    config: &StackConfig,
) -> Result<StackResult> {
    if images.is_empty() {
        bail!("No images to stack");
    }

    let reference = &images[0];
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

    let ref_cropped = crop(reference);

    let mut aligned: Vec<Array2<f32>> = Vec::with_capacity(n);
    let mut offsets: Vec<(i32, i32)> = Vec::with_capacity(n);

    aligned.push(ref_cropped.clone());
    offsets.push((0, 0));

    let search_radius = 50i32;

    for i in 1..n {
        let cropped = crop(&images[i]);

        if config.align {
            let (dy, dx) = align::compute_offset(&ref_cropped, &cropped, search_radius);
            offsets.push((dy, dx));
            aligned.push(align::shift_image(&cropped, dy, dx));
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
