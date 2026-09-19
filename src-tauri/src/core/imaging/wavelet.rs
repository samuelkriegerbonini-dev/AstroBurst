use anyhow::Result;
use ndarray::Array2;
use rayon::prelude::*;

use crate::infra::progress::ProgressHandle;
use crate::math::median::median_f32_mut;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::error::AppError;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WaveletConfig {
    pub num_scales: usize,
    pub thresholds: Vec<f32>,
    pub linear_denoise: bool,
    #[serde(default)]
    pub layer_bias: Option<Vec<f32>>,
}

impl Default for WaveletConfig {
    fn default() -> Self {
        Self {
            num_scales: 5,
            thresholds: vec![3.0, 2.5, 2.0, 1.5, 1.0],
            linear_denoise: true,
            layer_bias: None,
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

#[derive(Debug, Clone)]
pub struct AtrousLayers {
    pub layers: Vec<Array2<f32>>,
    pub residual: Array2<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct NoiseEstimate {
    pub sigma: f64,
    pub fraction_used: f64,
    pub iterations: usize,
}

static B3_KERNEL_1D: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];

const K_SIGMA_RELATIVE_TOLERANCE: f64 = 1e-3;

pub fn wavelet_denoise(
    image: &Array2<f32>,
    config: &WaveletConfig,
    progress: Option<&ProgressHandle>,
) -> Result<WaveletResult> {
    let start = std::time::Instant::now();
    let num_scales = config.num_scales.clamp(1, 8);

    if let Some(p) = progress {
        p.set_total((num_scales * 2 + 1) as u64);
    }

    let mut decomposition = atrous_decompose_with_progress(image, num_scales, progress)?;

    let noise_sigma = noise_sigma_mad(&decomposition.layers[0]) / noise_scaling_for_layer(0);

    for (scale_idx, layer) in decomposition.layers.iter_mut().enumerate() {
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

        let threshold = threshold_sigma * (noise_sigma * noise_scaling_for_layer(scale_idx)) as f32;

        let slice = layer.as_slice_mut().expect("atrous layers are standard layout");
        if config.linear_denoise {
            soft_threshold_slice(slice, threshold);
        } else {
            hard_threshold_slice(slice, threshold);
        }
    }

    if let Some(p) = progress {
        p.tick_with_stage("reconstructing");
    }

    let bias = config.layer_bias.as_deref().unwrap_or(&[]);
    let denoised = atrous_reconstruct_with_bias(&decomposition, bias);

    if let Some(p) = progress {
        p.emit_complete();
    }

    Ok(WaveletResult {
        denoised,
        scales_processed: num_scales,
        noise_estimate: noise_sigma,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

pub fn atrous_decompose(image: &Array2<f32>, num_scales: usize) -> AtrousLayers {
    atrous_decompose_with_progress(image, num_scales, None)
        .expect("decomposition without a progress handle cannot be cancelled")
}

fn atrous_decompose_with_progress(
    image: &Array2<f32>,
    num_scales: usize,
    progress: Option<&ProgressHandle>,
) -> Result<AtrousLayers> {
    let (rows, cols) = image.dim();
    let npix = rows * cols;

    let mut layers: Vec<Array2<f32>> = Vec::with_capacity(num_scales);
    let mut current: Vec<f32> = match image.as_slice() {
        Some(slice) => slice.to_vec(),
        None => image.iter().copied().collect(),
    };
    let mut h_buf = vec![0.0f32; npix];
    let mut smoothed = vec![0.0f32; npix];

    for scale_idx in 0..num_scales {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
            p.tick_with_stage(&format!("decomposing scale {}/{}", scale_idx + 1, num_scales));
        }

        let step = 1usize << scale_idx;
        atrous_smooth_buffers(&current, rows, cols, step, &mut h_buf, &mut smoothed);

        let detail: Vec<f32> = current
            .par_iter()
            .zip(smoothed.par_iter())
            .map(|(&c, &s)| c - s)
            .collect();
        layers.push(Array2::from_shape_vec((rows, cols), detail).unwrap());

        std::mem::swap(&mut current, &mut smoothed);
    }

    Ok(AtrousLayers {
        layers,
        residual: Array2::from_shape_vec((rows, cols), current).unwrap(),
    })
}

pub fn atrous_reconstruct(layers: &AtrousLayers) -> Array2<f32> {
    atrous_reconstruct_with_bias(layers, &[])
}

pub fn atrous_reconstruct_with_bias(layers: &AtrousLayers, layer_bias: &[f32]) -> Array2<f32> {
    let (rows, cols) = layers.residual.dim();
    let gains: Vec<f32> = (0..layers.layers.len())
        .map(|k| 1.0 + layer_bias.get(k).copied().unwrap_or(0.0))
        .collect();
    let layer_slices: Vec<&[f32]> = layers
        .layers
        .iter()
        .map(|layer| layer.as_slice().expect("atrous layers are standard layout"))
        .collect();

    let mut out: Vec<f32> = match layers.residual.as_slice() {
        Some(slice) => slice.to_vec(),
        None => layers.residual.iter().copied().collect(),
    };

    out.par_iter_mut().enumerate().for_each(|(i, v)| {
        let mut sum = *v;
        for (layer, &gain) in layer_slices.iter().zip(gains.iter()) {
            sum += layer[i] * gain;
        }
        *v = sum;
    });

    Array2::from_shape_vec((rows, cols), out).unwrap()
}

fn atrous_smooth_buffers(
    input: &[f32],
    rows: usize,
    cols: usize,
    step: usize,
    h_buf: &mut [f32],
    out: &mut [f32],
) {
    h_buf.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        let src_row = &input[y * cols..(y + 1) * cols];
        for x in 0..cols {
            let mut sum = 0.0f32;
            let mut wsum = 0.0f32;
            for (ki, &kv) in B3_KERNEL_1D.iter().enumerate() {
                let ox = x as isize + (ki as isize - 2) * step as isize;
                let cx = ox.clamp(0, cols as isize - 1) as usize;
                let v = src_row[cx];
                if v.is_finite() {
                    sum += v * kv;
                    wsum += kv;
                }
            }
            row[x] = if wsum > 0.0 { sum / wsum } else { f32::NAN };
        }
    });

    out.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        for x in 0..cols {
            let mut sum = 0.0f32;
            let mut wsum = 0.0f32;
            for (ki, &kv) in B3_KERNEL_1D.iter().enumerate() {
                let oy = y as isize + (ki as isize - 2) * step as isize;
                let cy = oy.clamp(0, rows as isize - 1) as usize;
                let v = h_buf[cy * cols + x];
                if v.is_finite() {
                    sum += v * kv;
                    wsum += kv;
                }
            }
            row[x] = if wsum > 0.0 { sum / wsum } else { f32::NAN };
        }
    });
}

pub fn noise_sigma_mad(layer0: &Array2<f32>) -> f64 {
    mad_sigma_of_finite(layer0.iter().copied())
}

#[cfg(test)]
fn estimate_noise_sigma(finest_scale: &[f32]) -> f64 {
    mad_sigma_of_finite(finest_scale.iter().copied())
}

fn mad_sigma_of_finite(values: impl Iterator<Item = f32>) -> f64 {
    let mut abs_vals: Vec<f32> = values.filter(|v| v.is_finite()).map(f32::abs).collect();

    if abs_vals.is_empty() {
        return 0.0;
    }

    let median = median_f32_mut(&mut abs_vals);
    (median as f64) * MAD_TO_SIGMA
}

pub fn noise_scaling_for_layer(k: usize) -> f64 {
    const TABLE: [f64; 7] = [0.8908, 0.2007, 0.0856, 0.0413, 0.0205, 0.0103, 0.0051];
    if k < TABLE.len() {
        TABLE[k]
    } else {
        TABLE[6] / (2.0f64.powi(k as i32 - 6))
    }
}

pub fn k_sigma_noise(image: &Array2<f32>, k: f64, max_iter: usize) -> NoiseEstimate {
    let decomposition = atrous_decompose(image, 1);
    let finite: Vec<f64> = decomposition.layers[0]
        .iter()
        .filter(|v| v.is_finite())
        .map(|&v| v as f64)
        .collect();

    if finite.is_empty() {
        return NoiseEstimate {
            sigma: 0.0,
            fraction_used: 0.0,
            iterations: 0,
        };
    }

    let mut sigma_layer = noise_sigma_mad(&decomposition.layers[0]);
    let mut kept = finite.len();
    let mut iterations = 0usize;

    while iterations < max_iter && sigma_layer > 0.0 {
        let clip = k * sigma_layer;
        let (count, sum, sum_sq) = finite
            .iter()
            .filter(|v| v.abs() < clip)
            .fold((0usize, 0.0f64, 0.0f64), |(n, s, q), &v| (n + 1, s + v, q + v * v));

        if count < 2 {
            break;
        }

        iterations += 1;
        kept = count;
        let mean = sum / count as f64;
        let variance = (sum_sq / count as f64 - mean * mean).max(0.0);
        let next_sigma = variance.sqrt();
        let relative_change = (next_sigma - sigma_layer).abs() / sigma_layer;
        sigma_layer = next_sigma;
        if relative_change < K_SIGMA_RELATIVE_TOLERANCE {
            break;
        }
    }

    NoiseEstimate {
        sigma: sigma_layer / noise_scaling_for_layer(0),
        fraction_used: kept as f64 / finite.len() as f64,
        iterations,
    }
}

fn soft_threshold_slice(data: &mut [f32], threshold: f32) {
    data.par_iter_mut().for_each(|v| {
        let abs = v.abs();
        if abs <= threshold {
            *v = 0.0;
        } else {
            *v = v.signum() * (abs - threshold);
        }
    });
}

fn hard_threshold_slice(data: &mut [f32], threshold: f32) {
    data.par_iter_mut().for_each(|v| {
        if v.abs() <= threshold {
            *v = 0.0;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN_SOFT_CHECKSUM: u64 = 0x37b98761d4e60532;
    const GOLDEN_HARD_CHECKSUM: u64 = 0x669d722b5ef0c674;
    const GOLDEN_NOISE_ESTIMATE_BITS: u64 = 0x400276be978202fd;

    fn pseudo_noise(seed: u64) -> f32 {
        let x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((x >> 33) as f32 / u32::MAX as f32 - 0.5) * 2.0
    }

    fn atrous_smooth_alloc(image: &Array2<f32>, scale: usize) -> Array2<f32> {
        let (rows, cols) = image.dim();
        let npix = rows * cols;
        let input = image.as_slice().unwrap().to_vec();
        let mut h_buf = vec![0.0f32; npix];
        let mut out = vec![0.0f32; npix];
        let step = 1usize << scale;
        atrous_smooth_buffers(&input, rows, cols, step, &mut h_buf, &mut out);
        Array2::from_shape_vec((rows, cols), out).unwrap()
    }

    #[test]
    fn test_b3_kernel_sums_to_one() {
        let sum: f32 = B3_KERNEL_1D.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_atrous_smooth_preserves_flat() {
        let image = Array2::from_elem((32, 32), 100.0f32);
        let smoothed = atrous_smooth_alloc(&image, 0);
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
            layer_bias: None,
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
        let mut data = vec![-5.0f32, -1.0, 0.5, 1.0, 3.0, 10.0];
        soft_threshold_slice(&mut data, 2.0);
        assert!((data[0] - (-3.0)).abs() < 1e-6);
        assert!((data[1] - 0.0).abs() < 1e-6);
        assert!((data[2] - 0.0).abs() < 1e-6);
        assert!((data[3] - 0.0).abs() < 1e-6);
        assert!((data[4] - 1.0).abs() < 1e-6);
        assert!((data[5] - 8.0).abs() < 1e-6);
    }

    #[test]
    fn test_noise_reduction() {
        let mut image = Array2::from_elem((64, 64), 100.0f32);
        for y in 0..64 {
            for x in 0..64 {
                image[[y, x]] += pseudo_noise((y * 64 + x) as u64) * 5.0;
            }
        }

        let config = WaveletConfig {
            num_scales: 4,
            thresholds: vec![3.0, 2.0, 1.5, 1.0],
            linear_denoise: true,
            layer_bias: None,
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
        let sigma = estimate_noise_sigma(&noise);
        assert!(sigma > 0.0 && sigma < 2.0, "Sigma estimate: {}", sigma);
    }

    #[test]
    fn test_reconstruction_preserves_negative_and_nan() {
        let mut image = Array2::from_elem((64, 64), -0.5f32);
        image[[0, 0]] = f32::NAN;

        let config = WaveletConfig {
            num_scales: 3,
            thresholds: vec![0.0, 0.0, 0.0],
            linear_denoise: true,
            layer_bias: None,
        };

        let result = wavelet_denoise(&image, &config, None).unwrap();
        assert!(result.denoised[[0, 0]].is_nan());
        for y in 4..60 {
            for x in 4..60 {
                assert!(
                    (result.denoised[[y, x]] + 0.5).abs() < 1e-4,
                    "Negative pixel at ({},{}) was rectified: {}",
                    y, x, result.denoised[[y, x]]
                );
            }
        }
    }

    #[test]
    fn test_nan_region_does_not_dilate() {
        let mut image = Array2::from_elem((64, 64), 100.0f32);
        for y in 0..64 {
            for x in 0..4 {
                image[[y, x]] = f32::NAN;
            }
        }

        let config = WaveletConfig {
            num_scales: 4,
            thresholds: vec![0.0, 0.0, 0.0, 0.0],
            linear_denoise: true,
            layer_bias: None,
        };

        let result = wavelet_denoise(&image, &config, None).unwrap();
        for y in 8..56 {
            for x in 8..56 {
                assert!(
                    (result.denoised[[y, x]] - 100.0).abs() < 0.5,
                    "Real data at ({},{}) was destroyed: {}",
                    y, x, result.denoised[[y, x]]
                );
            }
        }
    }

    fn noisy_image(rows: usize, cols: usize, base: f32, amplitude: f32) -> Array2<f32> {
        let mut image = Array2::from_elem((rows, cols), base);
        for y in 0..rows {
            for x in 0..cols {
                image[[y, x]] += pseudo_noise((y * cols + x) as u64) * amplitude;
            }
        }
        image
    }

    fn gaussian_noise_image(rows: usize, cols: usize, sigma: f64, seed: u64) -> Array2<f32> {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        let mut rng = StdRng::seed_from_u64(seed);
        Array2::from_shape_fn((rows, cols), |_| {
            let u1: f64 = rng.gen::<f64>().max(1e-30);
            let u2: f64 = rng.gen::<f64>();
            (sigma * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
        })
    }

    fn bits_checksum(arr: &Array2<f32>) -> u64 {
        arr.iter().fold(0xcbf29ce484222325u64, |acc, v| {
            (acc ^ v.to_bits() as u64).wrapping_mul(0x100000001b3)
        })
    }

    #[test]
    fn test_denoise_output_golden_checksums() {
        let image = noisy_image(64, 64, 100.0, 5.0);
        let soft = WaveletConfig {
            num_scales: 4,
            thresholds: vec![3.0, 2.0, 1.5, 1.0],
            linear_denoise: true,
            layer_bias: None,
        };
        let hard = WaveletConfig {
            linear_denoise: false,
            ..soft.clone()
        };
        let soft_out = wavelet_denoise(&image, &soft, None).unwrap();
        let hard_out = wavelet_denoise(&image, &hard, None).unwrap();
        assert_eq!(bits_checksum(&soft_out.denoised), GOLDEN_SOFT_CHECKSUM);
        assert_eq!(bits_checksum(&hard_out.denoised), GOLDEN_HARD_CHECKSUM);
        assert_eq!(soft_out.noise_estimate.to_bits(), GOLDEN_NOISE_ESTIMATE_BITS);
    }

    #[test]
    fn test_atrous_decompose_layers_are_successive_smoothing_differences() {
        let image = noisy_image(48, 48, 20.0, 3.0);
        let decomposition = atrous_decompose(&image, 3);
        assert_eq!(decomposition.layers.len(), 3);
        assert_eq!(decomposition.residual.dim(), (48, 48));

        let c1 = atrous_smooth_alloc(&image, 0);
        let c2 = atrous_smooth_alloc(&c1, 1);
        let c3 = atrous_smooth_alloc(&c2, 2);
        for y in 0..48 {
            for x in 0..48 {
                assert_eq!(decomposition.layers[0][[y, x]], image[[y, x]] - c1[[y, x]]);
                assert_eq!(decomposition.layers[1][[y, x]], c1[[y, x]] - c2[[y, x]]);
                assert_eq!(decomposition.layers[2][[y, x]], c2[[y, x]] - c3[[y, x]]);
                assert_eq!(decomposition.residual[[y, x]], c3[[y, x]]);
            }
        }
    }

    #[test]
    fn test_atrous_reconstruct_matches_zero_threshold_denoise() {
        let image = noisy_image(64, 64, 100.0, 5.0);
        let config = WaveletConfig {
            num_scales: 4,
            thresholds: vec![0.0; 4],
            linear_denoise: true,
            layer_bias: None,
        };
        let denoised = wavelet_denoise(&image, &config, None).unwrap().denoised;
        let rebuilt = atrous_reconstruct(&atrous_decompose(&image, 4));
        for y in 0..64 {
            for x in 0..64 {
                assert_eq!(denoised[[y, x]], rebuilt[[y, x]]);
                assert!(
                    (rebuilt[[y, x]] - image[[y, x]]).abs() < 1e-3,
                    "reconstruction drift at ({},{}): {} vs {}",
                    y, x, rebuilt[[y, x]], image[[y, x]]
                );
            }
        }
    }

    #[test]
    fn test_atrous_decompose_propagates_nan_without_dilating() {
        let mut image = Array2::from_elem((32, 32), 10.0f32);
        image[[5, 5]] = f32::NAN;
        let decomposition = atrous_decompose(&image, 2);
        assert!(decomposition.layers[0][[5, 5]].is_nan());
        assert!(decomposition.residual[[5, 5]].is_finite());
        assert!(decomposition.layers[0][[5, 6]].is_finite());
        let rebuilt = atrous_reconstruct(&decomposition);
        assert!(rebuilt[[5, 5]].is_nan());
        assert!((rebuilt[[5, 6]] - 10.0).abs() < 1e-4);
    }

    #[test]
    fn test_noise_scaling_for_layer_follows_b3_table() {
        assert!((noise_scaling_for_layer(0) - 0.8908).abs() < 1e-12);
        assert!((noise_scaling_for_layer(1) - 0.2007).abs() < 1e-12);
        assert!((noise_scaling_for_layer(6) - 0.0051).abs() < 1e-12);
        assert!((noise_scaling_for_layer(7) - 0.00255).abs() < 1e-12);
        assert!((noise_scaling_for_layer(8) - 0.001275).abs() < 1e-12);
    }

    #[test]
    fn test_noise_sigma_mad_matches_slice_estimator() {
        let noise: Vec<f32> = (0..4096).map(|i| pseudo_noise(i as u64)).collect();
        let expected = estimate_noise_sigma(&noise);
        let layer = Array2::from_shape_vec((64, 64), noise).unwrap();
        assert_eq!(noise_sigma_mad(&layer), expected);
        assert!(expected > 0.0);
    }

    #[test]
    fn test_k_sigma_noise_recovers_gaussian_sigma() {
        let image = gaussian_noise_image(256, 256, 2.0, 42);
        let estimate = k_sigma_noise(&image, 3.0, 10);
        assert!(
            (estimate.sigma - 2.0).abs() / 2.0 < 0.05,
            "sigma estimate {} deviates more than 5% from 2.0",
            estimate.sigma
        );
        assert!(estimate.fraction_used > 0.9, "fraction used {}", estimate.fraction_used);
        assert!(estimate.iterations >= 1 && estimate.iterations <= 10);
    }

    #[test]
    fn test_k_sigma_noise_rejects_bright_disk() {
        let mut image = gaussian_noise_image(256, 256, 2.0, 7);
        let radius_sq = 0.05 * 256.0 * 256.0 / std::f64::consts::PI;
        let mut disk_pixels = 0usize;
        for y in 0..256 {
            for x in 0..256 {
                let dy = y as f64 - 128.0;
                let dx = x as f64 - 128.0;
                if dy * dy + dx * dx <= radius_sq {
                    image[[y, x]] += 1000.0;
                    disk_pixels += 1;
                }
            }
        }
        assert!(disk_pixels > 3000 && disk_pixels < 3600, "disk covers {} pixels", disk_pixels);

        let estimate = k_sigma_noise(&image, 3.0, 10);
        assert!(
            (estimate.sigma - 2.0).abs() / 2.0 < 0.10,
            "sigma estimate {} deviates more than 10% from 2.0",
            estimate.sigma
        );
        assert!(estimate.fraction_used > 0.9, "fraction used {}", estimate.fraction_used);
    }

    #[test]
    fn test_k_sigma_noise_handles_all_nan() {
        let image = Array2::from_elem((16, 16), f32::NAN);
        let estimate = k_sigma_noise(&image, 3.0, 5);
        assert_eq!(estimate.sigma, 0.0);
        assert_eq!(estimate.fraction_used, 0.0);
        assert_eq!(estimate.iterations, 0);
    }

    #[test]
    fn test_layer_bias_doubles_finest_detail_of_impulse() {
        let mut image = Array2::from_elem((32, 32), 0.0f32);
        image[[16, 16]] = 1.0;
        let base = WaveletConfig {
            num_scales: 3,
            thresholds: vec![0.0; 3],
            linear_denoise: true,
            layer_bias: None,
        };
        let biased = WaveletConfig {
            layer_bias: Some(vec![1.0]),
            ..base.clone()
        };
        let unbiased_out = wavelet_denoise(&image, &base, None).unwrap().denoised;
        let biased_out = wavelet_denoise(&image, &biased, None).unwrap().denoised;
        let layer0 = &atrous_decompose(&image, 3).layers[0];
        assert!(layer0[[16, 16]] > 0.5);
        for y in 0..32 {
            for x in 0..32 {
                let added = biased_out[[y, x]] - unbiased_out[[y, x]];
                assert!(
                    (added - layer0[[y, x]]).abs() < 1e-6,
                    "bias added {} at ({},{}) but layer0 is {}",
                    added, y, x, layer0[[y, x]]
                );
            }
        }
    }

    #[test]
    fn test_negative_layer_bias_softens_finest_detail() {
        let mut image = Array2::from_elem((32, 32), 0.0f32);
        image[[16, 16]] = 1.0;
        let config = WaveletConfig {
            num_scales: 3,
            thresholds: vec![0.0; 3],
            linear_denoise: true,
            layer_bias: Some(vec![-1.0]),
        };
        let out = wavelet_denoise(&image, &config, None).unwrap().denoised;
        let decomposition = atrous_decompose(&image, 3);
        let expected = out[[16, 16]] + decomposition.layers[0][[16, 16]];
        assert!((expected - 1.0).abs() < 1e-6, "softened center {} plus layer0 should restore the impulse", out[[16, 16]]);
    }

    #[test]
    fn test_zero_layer_bias_is_bit_identical() {
        let mut image = noisy_image(64, 64, 100.0, 5.0);
        image[[3, 3]] = f32::NAN;
        let base = WaveletConfig {
            num_scales: 4,
            thresholds: vec![3.0, 2.0, 1.5, 1.0],
            linear_denoise: true,
            layer_bias: None,
        };
        let zero_bias = WaveletConfig {
            layer_bias: Some(vec![0.0; 4]),
            ..base.clone()
        };
        let a = wavelet_denoise(&image, &base, None).unwrap().denoised;
        let b = wavelet_denoise(&image, &zero_bias, None).unwrap().denoised;
        for y in 0..64 {
            for x in 0..64 {
                let (va, vb) = (a[[y, x]], b[[y, x]]);
                if va.is_nan() || vb.is_nan() {
                    assert!(va.is_nan() && vb.is_nan());
                } else {
                    assert_eq!(va.to_bits(), vb.to_bits(), "bit mismatch at ({},{})", y, x);
                }
            }
        }
    }
}
