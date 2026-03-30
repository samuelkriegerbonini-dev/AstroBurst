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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendPreset {
    pub name: String,
    pub weights: Vec<BlendWeight>,
}

pub fn preset_sho() -> Vec<BlendWeight> {
    vec![
        BlendWeight { channel_idx: 0, r_weight: 1.0, g_weight: 0.0, b_weight: 0.0 },
        BlendWeight { channel_idx: 1, r_weight: 0.0, g_weight: 1.0, b_weight: 0.0 },
        BlendWeight { channel_idx: 2, r_weight: 0.0, g_weight: 0.0, b_weight: 1.0 },
    ]
}

pub fn preset_hubble_legacy() -> Vec<BlendWeight> {
    vec![
        BlendWeight { channel_idx: 0, r_weight: 0.7, g_weight: 0.3, b_weight: 0.0 },
        BlendWeight { channel_idx: 1, r_weight: 0.3, g_weight: 0.8, b_weight: 0.2 },
        BlendWeight { channel_idx: 2, r_weight: 0.0, g_weight: 0.15, b_weight: 0.85 },
    ]
}

pub fn preset_hoo() -> Vec<BlendWeight> {
    vec![
        BlendWeight { channel_idx: 0, r_weight: 1.0, g_weight: 0.0, b_weight: 0.0 },
        BlendWeight { channel_idx: 1, r_weight: 0.0, g_weight: 0.5, b_weight: 0.5 },
    ]
}

pub fn preset_foraxx() -> Vec<BlendWeight> {
    vec![
        BlendWeight { channel_idx: 0, r_weight: 0.8, g_weight: 0.2, b_weight: 0.0 },
        BlendWeight { channel_idx: 1, r_weight: 0.2, g_weight: 0.7, b_weight: 0.1 },
        BlendWeight { channel_idx: 2, r_weight: 0.0, g_weight: 0.1, b_weight: 0.9 },
    ]
}

pub fn preset_dynamic_hoo() -> Vec<BlendWeight> {
    vec![
        BlendWeight { channel_idx: 0, r_weight: 0.9, g_weight: 0.4, b_weight: 0.0 },
        BlendWeight { channel_idx: 1, r_weight: 0.1, g_weight: 0.6, b_weight: 1.0 },
    ]
}

pub fn preset_rgb() -> Vec<BlendWeight> {
    vec![
        BlendWeight { channel_idx: 0, r_weight: 1.0, g_weight: 0.0, b_weight: 0.0 },
        BlendWeight { channel_idx: 1, r_weight: 0.0, g_weight: 1.0, b_weight: 0.0 },
        BlendWeight { channel_idx: 2, r_weight: 0.0, g_weight: 0.0, b_weight: 1.0 },
    ]
}

pub fn get_preset(name: &str) -> Vec<BlendWeight> {
    match name.to_lowercase().as_str() {
        "sho" | "hubble" => preset_sho(),
        "hubble_legacy" | "hubblelegacy" => preset_hubble_legacy(),
        "hoo" => preset_hoo(),
        "foraxx" => preset_foraxx(),
        "dynamic_hoo" | "dynamichoo" => preset_dynamic_hoo(),
        "rgb" => preset_rgb(),
        _ => preset_rgb(),
    }
}

pub fn blend_channels(
    channels: &[&Array2<f32>],
    weights: &[BlendWeight],
    rows: usize,
    cols: usize,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let npix = rows * cols;
    let mut r_out = vec![0.0f32; npix];
    let mut g_out = vec![0.0f32; npix];
    let mut b_out = vec![0.0f32; npix];

    for w in weights {
        if w.channel_idx >= channels.len() {
            continue;
        }
        let ch = channels[w.channel_idx];
        let src = ch.as_slice().unwrap_or(&[]);
        let rw = w.r_weight as f32;
        let gw = w.g_weight as f32;
        let bw = w.b_weight as f32;
        let len = npix.min(src.len());

        r_out[..len].par_iter_mut().zip(g_out[..len].par_iter_mut()).zip(b_out[..len].par_iter_mut()).zip(src[..len].par_iter())
            .for_each(|(((r, g), b), &v)| {
                if rw != 0.0 { *r += v * rw; }
                if gw != 0.0 { *g += v * gw; }
                if bw != 0.0 { *b += v * bw; }
            });
    }

    let r = Array2::from_shape_vec((rows, cols), r_out).unwrap();
    let g = Array2::from_shape_vec((rows, cols), g_out).unwrap();
    let b = Array2::from_shape_vec((rows, cols), b_out).unwrap();

    (r, g, b)
}

pub fn list_presets() -> Vec<BlendPreset> {
    vec![
        BlendPreset { name: "RGB".into(), weights: preset_rgb() },
        BlendPreset { name: "SHO (Hubble)".into(), weights: preset_sho() },
        BlendPreset { name: "Hubble Legacy".into(), weights: preset_hubble_legacy() },
        BlendPreset { name: "HOO".into(), weights: preset_hoo() },
        BlendPreset { name: "Dynamic HOO".into(), weights: preset_dynamic_hoo() },
        BlendPreset { name: "Foraxx".into(), weights: preset_foraxx() },
    ]
}
