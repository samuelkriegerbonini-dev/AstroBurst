use anyhow::Result;
use ndarray::Array2;
use rayon::prelude::*;

use crate::infra::progress::ProgressHandle;
use crate::math::median::{median_f32_mut};
use crate::types::error::AppError;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WaveletConfig {
    pub num_scales: usize,
    pub thresholds: Vec<f32>,
    pub linear_denoise: bool,
}

impl Default for WaveletConfig {
    fn default() -> Self {
        Self {
            num_scales: 5,
            thresholds: vec![3.0, 2.5, 2.0, 1.5, 1.0],
            linear_denoise: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WaveletResult {
    pub denoised: Array2<f32>,
    pub scales_processed: usize,
    pub noise_estimate: f64,
    pub elapsed_ms: u64,
}

static B3_KERNEL_1D: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];

pub fn wavelet_denoise(
    image: &Array2<f32>,
    config: &WaveletConfig,
    progress: Option<&ProgressHandle>,
) -> Result<WaveletResult> {
    let start = std::time::Instant::now();
    let num_scales = config.num_scales.min(8).max(1);

    if let Some(p) = progress {
        p.set_total((num_scales * 2 + 1) as u64);
    }

    let mut scales: Vec<Array2<f32>> = Vec::with_capacity(num_scales);
    let mut current = image.clone();

    for scale_idx in 0..num_scales {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
            p.tick_with_stage(&format!("decomposing scale {}/{}", scale_idx + 1, num_scales));
        }

        let smoothed = atrous_smooth(&current, scale_idx);
        let detail = &current - &smoothed;
        scales.push(detail);
        current = smoothed;
    }

    let residual = current;

    let noise_sigma = estimate_noise_sigma(&scales[0]);

    for (scale_idx, scale) in scales.iter_mut().enumerate() {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
            p.tick_with_stage(&format!("thresholding scale {}/{}", scale_idx + 1, num_scales));
        }

        let threshold_sigma = if scale_idx < config.thresholds.len() {
            config.thresholds[scale_idx]
        } else {
            *config.thresholds.last().unwrap_or(&1.0)
        };

        let scale_noise = noise_sigma * atrous_noise_scaling(scale_idx);
        let threshold = threshold_sigma * scale_noise as f32;

        if config.linear_denoise {
            soft_threshold(scale, threshold);
        } else {
            hard_threshold(scale, threshold);
        }
    }

    if let Some(p) = progress {
        p.tick_with_stage("reconstructing");
    }

    let mut result = residual;
    for scale in scales.iter().rev() {
        result += scale;
    }

    result.mapv_inplace(|v| {
        if !v.is_finite() || v < 0.0 {
            0.0
        } else {
            v
        }
    });

    if let Some(p) = progress {
        p.emit_complete();
    }

    Ok(WaveletResult {
        denoised: result,
        scales_processed: num_scales,
        noise_estimate: noise_sigma,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn atrous_smooth(image: &Array2<f32>, scale: usize) -> Array2<f32> {
    let (rows, cols) = image.dim();
    let step = 1usize << scale;

    let mut temp = vec![0.0f32; rows * cols];
    temp.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        for x in 0..cols {
            let mut sum = 0.0f32;
            for (ki, &kv) in B3_KERNEL_1D.iter().enumerate() {
                let ox = x as isize + (ki as isize - 2) * step as isize;
                let cx = ox.clamp(0, cols as isize - 1) as usize;
                sum += image[[y, cx]] * kv;
            }
            row[x] = sum;
        }
    });

    let temp_arr = Array2::from_shape_vec((rows, cols), temp).unwrap();

    if step > 16 && rows > 256 {
        atrous_vertical_transposed(&temp_arr, rows, cols, step)
    } else {
        atrous_vertical_direct(&temp_arr, rows, cols, step)
    }
}

fn atrous_vertical_direct(temp: &Array2<f32>, rows: usize, cols: usize, step: usize) -> Array2<f32> {
    let mut output = vec![0.0f32; rows * cols];
    output.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        for x in 0..cols {
            let mut sum = 0.0f32;
            for (ki, &kv) in B3_KERNEL_1D.iter().enumerate() {
                let oy = y as isize + (ki as isize - 2) * step as isize;
                let cy = oy.clamp(0, rows as isize - 1) as usize;
                sum += temp[[cy, x]] * kv;
            }
            row[x] = sum;
        }
    });
    Array2::from_shape_vec((rows, cols), output).unwrap()
}

fn atrous_vertical_transposed(temp: &Array2<f32>, rows: usize, cols: usize, step: usize) -> Array2<f32> {
    let mut transposed = vec![0.0f32; rows * cols];
    for y in 0..rows {
        for x in 0..cols {
            transposed[x * rows + y] = temp[[y, x]];
        }
    }

    let mut out_t = vec![0.0f32; rows * cols];
    out_t.par_chunks_mut(rows).enumerate().for_each(|(x, col)| {
        for y in 0..rows {
            let mut sum = 0.0f32;
            for (ki, &kv) in B3_KERNEL_1D.iter().enumerate() {
                let oy = y as isize + (ki as isize - 2) * step as isize;
                let cy = oy.clamp(0, rows as isize - 1) as usize;
                sum += transposed[x * rows + cy] * kv;
            }
            col[y] = sum;
        }
    });

    let mut output = vec![0.0f32; rows * cols];
    for x in 0..cols {
        for y in 0..rows {
            output[y * cols + x] = out_t[x * rows + y];
        }
    }
    Array2::from_shape_vec((rows, cols), output).unwrap()
}

fn estimate_noise_sigma(finest_scale: &Array2<f32>) -> f64 {
    let mut abs_vals: Vec<f32> = finest_scale
        .as_slice()
        .unwrap()
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| v.abs())
        .collect();

    if abs_vals.is_empty() {
        return 0.0;
    }

    let median = median_f32_mut(&mut abs_vals);
    (median as f64) / 0.6745
}

fn atrous_noise_scaling(scale: usize) -> f64 {
    match scale {
        0 => 0.8908,
        1 => 0.2007,
        2 => 0.0856,
        3 => 0.0413,
        4 => 0.0205,
        5 => 0.0103,
        6 => 0.0051,
        _ => 0.0051 / (2.0f64.powi(scale as i32 - 6)),
    }
}

fn soft_threshold(scale: &mut Array2<f32>, threshold: f32) {
    scale
        .as_slice_mut()
        .unwrap()
        .par_iter_mut()
        .for_each(|v| {
            if v.abs() <= threshold {
                *v = 0.0;
            } else if *v > 0.0 {
                *v -= threshold;
            } else {
                *v += threshold;
            }
        });
}

fn hard_threshold(scale: &mut Array2<f32>, threshold: f32) {
    scale
        .as_slice_mut()
        .unwrap()
        .par_iter_mut()
        .for_each(|v| {
            if v.abs() <= threshold {
                *v = 0.0;
            }
        });
}

fn pseudo_noise(seed: u64) -> f32 {
    let x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((x >> 33) as f32 / u32::MAX as f32 - 0.5) * 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_b3_kernel_sums_to_one() {
        let sum: f32 = B3_KERNEL_1D.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_atrous_smooth_preserves_flat() {
        let image = Array2::from_elem((32, 32), 100.0f32);
        let smoothed = atrous_smooth(&image, 0);
        for y in 2..30 {
            for x in 2..30 {
                assert!(
                    (smoothed[[y, x]] - 100.0).abs() < 0.01,
                    "Smoothed flat image should remain flat"
                );
            }
        }
    }

    #[test]
    fn test_wavelet_roundtrip_flat() {
        let image = Array2::from_elem((64, 64), 50.0f32);
        let config = WaveletConfig {
            num_scales: 3,
            thresholds: vec![0.0, 0.0, 0.0],
            linear_denoise: true,
        };

        let result = wavelet_denoise(&image, &config, None).unwrap();
        for y in 4..60 {
            for x in 4..60 {
                assert!(
                    (result.denoised[[y, x]] - 50.0).abs() < 0.1,
                    "Roundtrip should preserve flat image"
                );
            }
        }
    }

    #[test]
    fn test_soft_threshold() {
        let mut arr = Array2::from_shape_vec((2, 3), vec![-5.0, -1.0, 0.5, 1.0, 3.0, 10.0]).unwrap();
        soft_threshold(&mut arr, 2.0);
        assert!((arr[[0, 0]] - (-3.0)).abs() < 1e-6);
        assert!((arr[[0, 1]] - 0.0).abs() < 1e-6);
        assert!((arr[[0, 2]] - 0.0).abs() < 1e-6);
        assert!((arr[[1, 0]] - 0.0).abs() < 1e-6);
        assert!((arr[[1, 1]] - 1.0).abs() < 1e-6);
        assert!((arr[[1, 2]] - 8.0).abs() < 1e-6);
    }

    #[test]
    fn test_noise_reduction() {
        let mut image = Array2::from_elem((64, 64), 100.0f32);
        for y in 0..64 {
            for x in 0..64 {
                let noise = pseudo_noise((y * 64 + x) as u64) * 5.0;
                image[[y, x]] += noise;
            }
        }

        let config = WaveletConfig {
            num_scales: 4,
            thresholds: vec![3.0, 2.0, 1.5, 1.0],
            linear_denoise: true,
        };

        let result = wavelet_denoise(&image, &config, None).unwrap();

        let mut orig_var = 0.0f64;
        let mut denoised_var = 0.0f64;
        let n = 56 * 56;
        for y in 4..60 {
            for x in 4..60 {
                orig_var += (image[[y, x]] as f64 - 100.0).powi(2);
                denoised_var += (result.denoised[[y, x]] as f64 - 100.0).powi(2);
            }
        }
        orig_var /= n as f64;
        denoised_var /= n as f64;

        assert!(
            denoised_var < orig_var,
            "Denoised variance ({}) should be less than original ({})",
            denoised_var, orig_var
        );
    }

    #[test]
    fn test_estimate_noise_sigma() {
        let noise: Vec<f32> = (0..10000)
            .map(|i| pseudo_noise(i as u64))
            .collect();
        let arr = Array2::from_shape_vec((100, 100), noise).unwrap();
        let sigma = estimate_noise_sigma(&arr);
        assert!(sigma > 0.0 && sigma < 2.0, "Sigma estimate: {}", sigma);
    }
}
