use ndarray::{Array2, Array3};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone)]
pub struct CalibrationMasters {
    pub dark: Option<Array2<f32>>,
    pub flat: Option<Array2<f32>>,
    pub bias: Option<Array2<f32>>,
}

#[derive(Debug, Clone)]
pub struct ChannelInput {
    pub lights: Vec<Array2<f32>>,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct BatchStackConfig {
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub max_iterations: usize,
    pub normalize_before_stack: bool,
}

impl Default for BatchStackConfig {
    fn default() -> Self {
        Self {
            sigma_low: 2.5,
            sigma_high: 3.0,
            max_iterations: 5,
            normalize_before_stack: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BatchPipelineConfig {
    pub stack: BatchStackConfig,
}

impl Default for BatchPipelineConfig {
    fn default() -> Self {
        Self {
            stack: BatchStackConfig::default(),
        }
    }
}

#[derive(Debug)]
pub struct BatchPipelineResult {
    pub master_channels: Vec<(String, Array2<f32>)>,
    pub rgb: Option<Array3<f32>>,
    pub stats: BatchPipelineStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPipelineStats {
    pub darks_combined: usize,
    pub flats_combined: usize,
    pub bias_combined: usize,
    pub channels: Vec<BatchChannelStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchChannelStats {
    pub label: String,
    pub lights_input: usize,
    pub lights_after_rejection: Vec<usize>,
    pub mean: f64,
    pub stddev: f64,
}

pub fn calibrate_light(
    light: &Array2<f32>,
    masters: &CalibrationMasters,
) -> Array2<f32> {
    let (rows, cols) = light.dim();
    let npix = rows * cols;
    let src = light.as_slice().expect("contiguous");

    let bias_slice = masters.bias.as_ref().map(|b| b.as_slice().expect("contiguous"));
    let dark_slice = masters.dark.as_ref().map(|d| d.as_slice().expect("contiguous"));
    let flat_slice = masters.flat.as_ref().map(|f| f.as_slice().expect("contiguous"));

    let bias_ok = bias_slice.map_or(true, |s| s.len() == npix);
    let dark_ok = dark_slice.map_or(true, |s| s.len() == npix);
    let flat_ok = flat_slice.map_or(true, |s| s.len() == npix);

    let result: Vec<f32> = (0..npix)
        .into_par_iter()
        .map(|i| {
            let mut v = src[i];
            if bias_ok {
                if let Some(b) = bias_slice {
                    v -= b[i];
                }
            }
            if dark_ok {
                if let Some(d) = dark_slice {
                    v -= d[i];
                }
            }
            if flat_ok {
                if let Some(f) = flat_slice {
                    let fv = f[i];
                    if fv.is_finite() && fv.abs() > 1e-4 {
                        v /= fv;
                    }
                }
            }
            if v < 0.0 { 0.0 } else { v }
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result).unwrap()
}

pub fn run_batch_pipeline(
    channels: Vec<ChannelInput>,
    masters: &CalibrationMasters,
    config: &BatchPipelineConfig,
) -> Result<BatchPipelineResult, String> {
    if channels.is_empty() {
        return Err("No channels provided".into());
    }

    for ch in &channels {
        if ch.lights.is_empty() {
            return Err(format!("Channel '{}' has no lights", ch.label));
        }
        let ref_dim = ch.lights[0].dim();
        for (i, l) in ch.lights.iter().enumerate().skip(1) {
            if l.dim() != ref_dim {
                return Err(format!(
                    "Channel '{}': frame {} has shape {:?} but frame 0 has {:?}. All frames must match.",
                    ch.label, i, l.dim(), ref_dim
                ));
            }
        }
    }

    let mut pipeline_stats = BatchPipelineStats {
        darks_combined: if masters.dark.is_some() { 1 } else { 0 },
        flats_combined: if masters.flat.is_some() { 1 } else { 0 },
        bias_combined: if masters.bias.is_some() { 1 } else { 0 },
        channels: Vec::new(),
    };

    let mut master_channels: Vec<(String, Array2<f32>)> = Vec::new();

    for channel in &channels {
        let calibrated: Vec<Array2<f32>> = channel
            .lights
            .par_iter()
            .map(|l| calibrate_light(l, masters))
            .collect();

        let normalized = if config.stack.normalize_before_stack {
            normalize_frames(&calibrated)
        } else {
            calibrated
        };

        let (stacked, rejection_counts) =
            sigma_clipped_mean_stack(&normalized, &config.stack);

        let mean_val = stacked.iter().map(|&v| v as f64).sum::<f64>() / stacked.len() as f64;
        let var: f64 = stacked
            .iter()
            .map(|&v| ((v as f64) - mean_val).powi(2))
            .sum::<f64>()
            / stacked.len() as f64;

        pipeline_stats.channels.push(BatchChannelStats {
            label: channel.label.clone(),
            lights_input: channel.lights.len(),
            lights_after_rejection: rejection_counts,
            mean: mean_val,
            stddev: var.sqrt(),
        });

        master_channels.push((channel.label.clone(), stacked));
    }

    let rgb = compose_rgb_from_masters(&master_channels);

    Ok(BatchPipelineResult {
        master_channels,
        rgb,
        stats: pipeline_stats,
    })
}

fn compose_rgb_from_masters(masters: &[(String, Array2<f32>)]) -> Option<Array3<f32>> {
    let find = |label: &str| -> Option<&Array2<f32>> {
        masters.iter().find(|(l, _)| l.eq_ignore_ascii_case(label)).map(|(_, arr)| arr)
    };

    let r = find("R")?;
    let g = find("G")?;
    let b = find("B")?;
    let (h, w) = r.dim();

    if g.dim() != (h, w) || b.dim() != (h, w) {
        let min_h = h.min(g.dim().0).min(b.dim().0);
        let min_w = w.min(g.dim().1).min(b.dim().1);
        let r_n = normalize_channel(&r.slice(ndarray::s![..min_h, ..min_w]).to_owned());
        let g_n = normalize_channel(&g.slice(ndarray::s![..min_h, ..min_w]).to_owned());
        let b_n = normalize_channel(&b.slice(ndarray::s![..min_h, ..min_w]).to_owned());

        let rs = r_n.as_slice().unwrap();
        let gs = g_n.as_slice().unwrap();
        let bs = b_n.as_slice().unwrap();

        let pixels: Vec<f32> = (0..min_h)
            .into_par_iter()
            .flat_map(|y| {
                let base = y * min_w;
                (0..min_w).flat_map(move |x| {
                    let i = base + x;
                    [rs[i], gs[i], bs[i]]
                }).collect::<Vec<f32>>()
            })
            .collect();

        return Some(Array3::from_shape_vec((min_h, min_w, 3), pixels).unwrap());
    }

    let (r_norm, g_norm, b_norm) = match find("L") {
        Some(lum) if lum.dim() == (h, w) => {
            let r_n = normalize_channel(r);
            let g_n = normalize_channel(g);
            let b_n = normalize_channel(b);
            let l_n = normalize_channel(lum);
            (
                apply_luminance(&r_n, &g_n, &b_n, &l_n, 0),
                apply_luminance(&r_n, &g_n, &b_n, &l_n, 1),
                apply_luminance(&r_n, &g_n, &b_n, &l_n, 2),
            )
        }
        _ => (normalize_channel(r), normalize_channel(g), normalize_channel(b)),
    };

    let rs = r_norm.as_slice().unwrap();
    let gs = g_norm.as_slice().unwrap();
    let bs = b_norm.as_slice().unwrap();

    let pixels: Vec<f32> = (0..h)
        .into_par_iter()
        .flat_map(|y| {
            let base = y * w;
            (0..w).flat_map(move |x| {
                let i = base + x;
                [rs[i], gs[i], bs[i]]
            }).collect::<Vec<f32>>()
        })
        .collect();

    Some(Array3::from_shape_vec((h, w, 3), pixels).unwrap())
}

fn apply_luminance(r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>, lum: &Array2<f32>, ch: usize) -> Array2<f32> {
    let (h, w) = r.dim();
    let npix = h * w;

    let r_s = r.as_slice().unwrap();
    let g_s = g.as_slice().unwrap();
    let b_s = b.as_slice().unwrap();
    let l_s = lum.as_slice().unwrap();

    let result: Vec<f32> = (0..npix)
        .into_par_iter()
        .map(|i| {
            let rgb_lum = 0.2126 * r_s[i] + 0.7152 * g_s[i] + 0.0722 * b_s[i];
            let scale = if rgb_lum > 1e-10 { l_s[i] / rgb_lum } else { 1.0 };
            let val = match ch { 0 => r_s[i], 1 => g_s[i], _ => b_s[i] };
            (val * scale).clamp(0.0, 1.0)
        })
        .collect();

    Array2::from_shape_vec((h, w), result).unwrap()
}

fn normalize_channel(ch: &Array2<f32>) -> Array2<f32> {
    let slice = ch.as_slice().unwrap();
    let mut min_val = f32::INFINITY;
    let mut max_val = f32::NEG_INFINITY;

    for &v in slice {
        if v < min_val { min_val = v; }
        if v > max_val { max_val = v; }
    }

    let range = max_val - min_val;
    if range < 1e-10 {
        return Array2::zeros(ch.dim());
    }

    let inv_range = 1.0 / range;
    ch.mapv(|v| ((v - min_val) * inv_range).clamp(0.0, 1.0))
}

fn normalize_frames(frames: &[Array2<f32>]) -> Vec<Array2<f32>> {
    frames.par_iter().map(|frame| {
        let mean = frame.iter().map(|&v| v as f64).sum::<f64>() / frame.len() as f64;
        if mean > 0.0 {
            let inv_mean = 1.0 / mean as f32;
            frame.mapv(|v| v * inv_mean)
        } else {
            frame.clone()
        }
    }).collect()
}

fn sigma_clipped_mean_stack(frames: &[Array2<f32>], config: &BatchStackConfig) -> (Array2<f32>, Vec<usize>) {
    let (h, w) = frames[0].dim();
    let n = frames.len();
    let mut result = Array2::<f32>::zeros((h, w));
    let mut rejection_counts = vec![0usize; n];

    let frame_slices: Vec<&[f32]> = frames.iter()
        .map(|f| f.as_slice().expect("contiguous"))
        .collect();

    let rows: Vec<usize> = (0..h).collect();
    let row_data: Vec<(Vec<f32>, Vec<usize>)> = rows.par_iter().map(|&y| {
        let mut row = vec![0.0f32; w];
        let mut local_rejected = vec![0usize; n];
        let mut vals: Vec<(f32, usize)> = Vec::with_capacity(n);

        let base = y * w;

        for x in 0..w {
            vals.clear();
            let idx = base + x;
            for (i, slice) in frame_slices.iter().enumerate() {
                vals.push((slice[idx], i));
            }

            for _ in 0..config.max_iterations {
                if vals.len() < 3 { break; }
                let mean: f32 = vals.iter().map(|(v, _)| v).sum::<f32>() / vals.len() as f32;
                let var: f32 = vals.iter().map(|(v, _)| (v - mean).powi(2)).sum::<f32>() / vals.len() as f32;
                let sigma = var.sqrt();
                if sigma < 1e-10 { break; }
                let before = vals.len();
                vals.retain(|(v, idx)| {
                    let z = (v - mean) / sigma;
                    let keep = z > -config.sigma_low && z < config.sigma_high;
                    if !keep { local_rejected[*idx] += 1; }
                    keep
                });
                if vals.len() == before { break; }
            }

            row[x] = if vals.is_empty() { 0.0 } else { vals.iter().map(|(v, _)| v).sum::<f32>() / vals.len() as f32 };
        }
        (row, local_rejected)
    }).collect();

    for (y, (row, local_rej)) in row_data.into_iter().enumerate() {
        for (x, val) in row.into_iter().enumerate() { result[[y, x]] = val; }
        for (i, count) in local_rej.into_iter().enumerate() { rejection_counts[i] += count; }
    }
    (result, rejection_counts)
}
