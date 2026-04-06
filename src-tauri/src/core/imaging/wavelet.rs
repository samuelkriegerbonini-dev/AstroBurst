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

const TRANSPOSE_BLOCK: usize = 64;
const TRANSPOSE_THRESHOLD_STEP: usize = 16;
const TRANSPOSE_THRESHOLD_ROWS: usize = 256;

pub fn wavelet_denoise(
    image: &Array2<f32>,
    config: &WaveletConfig,
    progress: Option<&ProgressHandle>,
) -> Result<WaveletResult> {
    let start = std::time::Instant::now();
    let num_scales = config.num_scales.clamp(1, 8);
    let (rows, cols) = image.dim();
    let npix = rows * cols;

    if let Some(p) = progress {
        p.set_total((num_scales * 2 + 1) as u64);
    }

    let mut scales: Vec<Vec<f32>> = Vec::with_capacity(num_scales);
    let mut current = image.as_slice().unwrap().to_vec();
    let mut h_buf = vec![0.0f32; npix];
    let mut buf_a = vec![0.0f32; npix];
    let mut t_buf = vec![0.0f32; npix];

    for scale_idx in 0..num_scales {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
            p.tick_with_stage(&format!("decomposing scale {}/{}", scale_idx + 1, num_scales));
        }

        let step = 1usize << scale_idx;
        atrous_smooth_buffers(&current, rows, cols, step, &mut h_buf, &mut buf_a, &mut t_buf);

        let detail: Vec<f32> = current
            .par_iter()
            .zip(buf_a.par_iter())
            .map(|(&c, &s)| c - s)
            .collect();
        scales.push(detail);

        std::mem::swap(&mut current, &mut buf_a);
        buf_a.par_iter_mut().for_each(|v| *v = 0.0);
    }

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

        let threshold = threshold_sigma * (noise_sigma * atrous_noise_scaling(scale_idx)) as f32;

        if config.linear_denoise {
            soft_threshold_slice(scale, threshold);
        } else {
            hard_threshold_slice(scale, threshold);
        }
    }

    if let Some(p) = progress {
        p.tick_with_stage("reconstructing");
    }

    current
        .par_iter_mut()
        .enumerate()
        .for_each(|(i, v)| {
            let mut sum = *v;
            for scale in &scales {
                sum += scale[i];
            }
            *v = if sum.is_finite() && sum >= 0.0 { sum } else { 0.0 };
        });

    if let Some(p) = progress {
        p.emit_complete();
    }

    Ok(WaveletResult {
        denoised: Array2::from_shape_vec((rows, cols), current).unwrap(),
        scales_processed: num_scales,
        noise_estimate: noise_sigma,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

fn atrous_smooth_buffers(
    input: &[f32],
    rows: usize,
    cols: usize,
    step: usize,
    h_buf: &mut [f32],
    out: &mut [f32],
    t_buf: &mut [f32],
) {
    h_buf.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        let src_row = &input[y * cols..(y + 1) * cols];
        for x in 0..cols {
            let mut sum = 0.0f32;
            for (ki, &kv) in B3_KERNEL_1D.iter().enumerate() {
                let ox = x as isize + (ki as isize - 2) * step as isize;
                let cx = ox.clamp(0, cols as isize - 1) as usize;
                sum += src_row[cx] * kv;
            }
            row[x] = sum;
        }
    });

    if step > TRANSPOSE_THRESHOLD_STEP && rows > TRANSPOSE_THRESHOLD_ROWS {
        block_transpose(h_buf, t_buf, rows, cols);

        let t_ref: &[f32] = t_buf;
        out.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
            for x in 0..cols {
                let col_base = x * rows;
                let mut sum = 0.0f32;
                for (ki, &kv) in B3_KERNEL_1D.iter().enumerate() {
                    let oy = y as isize + (ki as isize - 2) * step as isize;
                    let cy = oy.clamp(0, rows as isize - 1) as usize;
                    sum += t_ref[col_base + cy] * kv;
                }
                row[x] = sum;
            }
        });
    } else {
        out.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
            for x in 0..cols {
                let mut sum = 0.0f32;
                for (ki, &kv) in B3_KERNEL_1D.iter().enumerate() {
                    let oy = y as isize + (ki as isize - 2) * step as isize;
                    let cy = oy.clamp(0, rows as isize - 1) as usize;
                    sum += h_buf[cy * cols + x] * kv;
                }
                row[x] = sum;
            }
        });
    }
}

fn block_transpose(src: &[f32], dst: &mut [f32], rows: usize, cols: usize) {
    for by in (0..rows).step_by(TRANSPOSE_BLOCK) {
        let ye = (by + TRANSPOSE_BLOCK).min(rows);
        for bx in (0..cols).step_by(TRANSPOSE_BLOCK) {
            let xe = (bx + TRANSPOSE_BLOCK).min(cols);
            for y in by..ye {
                let src_row = y * cols;
                for x in bx..xe {
                    dst[x * rows + y] = src[src_row + x];
                }
            }
        }
    }
}

fn estimate_noise_sigma(finest_scale: &[f32]) -> f64 {
    let mut abs_vals: Vec<f32> = finest_scale
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| v.abs())
        .collect();

    if abs_vals.is_empty() {
        return 0.0;
    }

    let median = median_f32_mut(&mut abs_vals);
    (median as f64) * MAD_TO_SIGMA
}

fn atrous_noise_scaling(scale: usize) -> f64 {
    const TABLE: [f64; 7] = [0.8908, 0.2007, 0.0856, 0.0413, 0.0205, 0.0103, 0.0051];
    if scale < TABLE.len() {
        TABLE[scale]
    } else {
        TABLE[6] / (2.0f64.powi(scale as i32 - 6))
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
        let mut t_buf = vec![0.0f32; npix];
        let step = 1usize << scale;
        atrous_smooth_buffers(&input, rows, cols, step, &mut h_buf, &mut out, &mut t_buf);
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
    fn test_block_transpose() {
        let rows = 130;
        let cols = 97;
        let src: Vec<f32> = (0..rows * cols).map(|i| i as f32).collect();
        let mut dst = vec![0.0f32; rows * cols];
        block_transpose(&src, &mut dst, rows, cols);
        for y in 0..rows {
            for x in 0..cols {
                assert_eq!(dst[x * rows + y], src[y * cols + x]);
            }
        }
    }
}
