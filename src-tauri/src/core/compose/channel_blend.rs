use ndarray::Array2;
use serde::{Deserialize, Serialize};
use rayon::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendWeight {
    pub channel_idx: usize,
    pub r_weight: f64,
    pub g_weight: f64,
    pub b_weight: f64,
}

pub fn blend_channels(
    channels: &[&Array2<f32>],
    weights: &[BlendWeight],
    rows: usize,
    cols: usize,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let npix = rows * cols;

    let valid_weights: Vec<(usize, f32, f32, f32)> = weights
        .iter()
        .filter(|w| w.channel_idx < channels.len())
        .map(|w| (w.channel_idx, w.r_weight as f32, w.g_weight as f32, w.b_weight as f32))
        .collect();

    let slices: Vec<&[f32]> = channels
        .iter()
        .map(|ch| ch.as_slice().unwrap_or(&[]))
        .collect();

    let mut r_out = vec![0.0f32; npix];
    let mut g_out = vec![0.0f32; npix];
    let mut b_out = vec![0.0f32; npix];

    r_out
        .par_chunks_mut(cols)
        .zip(g_out.par_chunks_mut(cols))
        .zip(b_out.par_chunks_mut(cols))
        .enumerate()
        .for_each(|(row_idx, ((r_row, g_row), b_row))| {
            let base = row_idx * cols;
            for x in 0..cols {
                let i = base + x;
                let mut rv = 0.0f32;
                let mut gv = 0.0f32;
                let mut bv = 0.0f32;

                for &(ch_idx, rw, gw, bw) in &valid_weights {
                    let src = slices[ch_idx];
                    if i < src.len() {
                        let v = src[i];
                        rv += v * rw;
                        gv += v * gw;
                        bv += v * bw;
                    }
                }

                r_row[x] = rv;
                g_row[x] = gv;
                b_row[x] = bv;
            }
        });

    let r = Array2::from_shape_vec((rows, cols), r_out).unwrap();
    let g = Array2::from_shape_vec((rows, cols), g_out).unwrap();
    let b = Array2::from_shape_vec((rows, cols), b_out).unwrap();

    (r, g, b)
}
