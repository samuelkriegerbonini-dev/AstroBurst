use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::imaging::luminance::{apply_luminance_ratio, rgb_to_luminance};
use crate::infra::progress::ProgressHandle;
use crate::types::error::AppError;

pub const LHE_KERNEL_RADIUS_MIN: usize = 16;
pub const LHE_KERNEL_RADIUS_MAX: usize = 512;
pub const LHE_CONTRAST_LIMIT_MIN: f32 = 1.0;
pub const LHE_CONTRAST_LIMIT_MAX: f32 = 64.0;
pub const LHE_HIST_BITS_ALLOWED: [u8; 3] = [8, 10, 12];

const FLAT_RANGE_EPSILON: f32 = 1e-12;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LheConfig {
    pub kernel_radius: usize,
    pub contrast_limit: f32,
    pub amount: f32,
    pub hist_bits: u8,
    pub circular: bool,
}

impl Default for LheConfig {
    fn default() -> Self {
        Self {
            kernel_radius: 64,
            contrast_limit: 2.0,
            amount: 1.0,
            hist_bits: 8,
            circular: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LheResult {
    pub image: Array2<f32>,
    pub input_min: f32,
    pub input_max: f32,
}

struct LheParams {
    radius: usize,
    clip_factor: f32,
    amount: f32,
    bins: usize,
    circular: bool,
}

impl LheParams {
    fn from_config(cfg: &LheConfig) -> Result<Self> {
        if !LHE_HIST_BITS_ALLOWED.contains(&cfg.hist_bits) {
            bail!(
                "hist_bits must be one of 8, 10 or 12 (got {})",
                cfg.hist_bits
            );
        }
        if !cfg.contrast_limit.is_finite() || !cfg.amount.is_finite() {
            bail!("contrast_limit and amount must be finite");
        }
        Ok(Self {
            radius: cfg
                .kernel_radius
                .clamp(LHE_KERNEL_RADIUS_MIN, LHE_KERNEL_RADIUS_MAX),
            clip_factor: cfg
                .contrast_limit
                .clamp(LHE_CONTRAST_LIMIT_MIN, LHE_CONTRAST_LIMIT_MAX),
            amount: cfg.amount.clamp(0.0, 1.0),
            bins: 1usize << cfg.hist_bits,
            circular: cfg.circular,
        })
    }
}

struct AxisWeight {
    lower: usize,
    upper: usize,
    weight: f32,
}

struct TileGrid {
    rows: usize,
    cols: usize,
    centres_y: Vec<f32>,
    centres_x: Vec<f32>,
    weights_x: Vec<AxisWeight>,
    band_bounds: Vec<(usize, usize)>,
}

impl TileGrid {
    fn new(rows: usize, cols: usize, tile: usize) -> Self {
        let centres_y = tile_centres(rows, tile);
        let centres_x = tile_centres(cols, tile);
        let weights_y = axis_weights(rows, &centres_y);
        let weights_x = axis_weights(cols, &centres_x);
        let band_bounds = band_bounds(&weights_y, centres_y.len());
        Self {
            rows,
            cols,
            centres_y,
            centres_x,
            weights_x,
            band_bounds,
        }
    }

    fn tile_rows(&self) -> usize {
        self.centres_y.len()
    }

    fn tile_cols(&self) -> usize {
        self.centres_x.len()
    }
}

fn tile_centres(len: usize, tile: usize) -> Vec<f32> {
    let count = len.div_ceil(tile).max(1);
    (0..count)
        .map(|t| {
            let start = t * tile;
            let end = (start + tile).min(len);
            (start + end - 1) as f32 / 2.0
        })
        .collect()
}

fn axis_weights(len: usize, centres: &[f32]) -> Vec<AxisWeight> {
    let last = centres.len() - 1;
    let mut current = 0usize;
    (0..len)
        .map(|p| {
            let pf = p as f32;
            if pf <= centres[0] {
                return AxisWeight {
                    lower: 0,
                    upper: 0,
                    weight: 0.0,
                };
            }
            if pf >= centres[last] {
                return AxisWeight {
                    lower: last,
                    upper: last,
                    weight: 0.0,
                };
            }
            while pf >= centres[current + 1] {
                current += 1;
            }
            let span = centres[current + 1] - centres[current];
            AxisWeight {
                lower: current,
                upper: current + 1,
                weight: (pf - centres[current]) / span,
            }
        })
        .collect()
}

fn band_bounds(weights: &[AxisWeight], tile_rows: usize) -> Vec<(usize, usize)> {
    let mut bounds = vec![(0usize, 0usize); tile_rows];
    let mut start = 0usize;
    for ty in 0..tile_rows {
        let mut end = start;
        while end < weights.len() && weights[end].lower == ty {
            end += 1;
        }
        bounds[ty] = (start, end);
        start = end;
    }
    bounds
}

struct Normaliser {
    min: f32,
    inv_range: f32,
    top_bin: f32,
}

impl Normaliser {
    fn position(&self, v: f32) -> f32 {
        ((v - self.min) * self.inv_range).clamp(0.0, 1.0) * self.top_bin
    }
}

fn finite_min_max(image: &Array2<f32>) -> (f32, f32) {
    image
        .iter()
        .filter(|v| v.is_finite())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &v| {
            (mn.min(v), mx.max(v))
        })
}

fn contiguous(image: &Array2<f32>) -> std::borrow::Cow<'_, [f32]> {
    match image.as_slice() {
        Some(slice) => std::borrow::Cow::Borrowed(slice),
        None => std::borrow::Cow::Owned(image.iter().copied().collect()),
    }
}

fn tile_lut(
    input: &[f32],
    grid: &TileGrid,
    params: &LheParams,
    norm: &Normaliser,
    ty: usize,
    tx: usize,
    lut: &mut [f32],
) {
    let bins = params.bins;
    let cy = grid.centres_y[ty];
    let cx = grid.centres_x[tx];
    let radius = params.radius as f32;
    let y0 = (cy - radius).floor().max(0.0) as usize;
    let y1 = ((cy + radius).ceil() as usize).min(grid.rows - 1);
    let x0 = (cx - radius).floor().max(0.0) as usize;
    let x1 = ((cx + radius).ceil() as usize).min(grid.cols - 1);
    let radius_sq = radius * radius;

    let mut hist = vec![0u32; bins];
    let mut count = 0u32;
    for y in y0..=y1 {
        let dy = y as f32 - cy;
        let row = &input[y * grid.cols..(y + 1) * grid.cols];
        for (x, &v) in row.iter().enumerate().take(x1 + 1).skip(x0) {
            if params.circular {
                let dx = x as f32 - cx;
                if dy * dy + dx * dx > radius_sq {
                    continue;
                }
            }
            if !v.is_finite() {
                continue;
            }
            let bin = (norm.position(v) + 0.5) as usize;
            hist[bin.min(bins - 1)] += 1;
            count += 1;
        }
    }

    if count == 0 {
        for (i, slot) in lut.iter_mut().enumerate() {
            *slot = i as f32 / norm.top_bin;
        }
        return;
    }

    let clip = (params.clip_factor * count as f32 / bins as f32).max(1.0);
    let mut excess = 0.0f32;
    for (slot, &h) in lut.iter_mut().zip(hist.iter()) {
        let clipped = (h as f32).min(clip);
        excess += h as f32 - clipped;
        *slot = clipped;
    }
    let redistributed = excess / bins as f32;
    let inv_count = 1.0 / count as f32;
    let mut cumulative = 0.0f32;
    for slot in lut.iter_mut() {
        let population = *slot + redistributed;
        cumulative += population;
        *slot = ((cumulative - 0.5 * population) * inv_count).clamp(0.0, 1.0);
    }
}

fn tile_row_luts(
    input: &[f32],
    grid: &TileGrid,
    params: &LheParams,
    norm: &Normaliser,
    ty: usize,
) -> Vec<f32> {
    let mut luts = vec![0.0f32; grid.tile_cols() * params.bins];
    luts.par_chunks_mut(params.bins)
        .enumerate()
        .for_each(|(tx, lut)| tile_lut(input, grid, params, norm, ty, tx, lut));
    luts
}

#[inline]
fn sample_lut(luts: &[f32], bins: usize, tx: usize, b0: usize, b1: usize, t: f32) -> f32 {
    let base = tx * bins;
    let low = luts[base + b0];
    low + (luts[base + b1] - low) * t
}

#[allow(clippy::too_many_arguments)]
fn equalise_band(
    input: &[f32],
    output: &mut [f32],
    grid: &TileGrid,
    params: &LheParams,
    norm: &Normaliser,
    upper: &[f32],
    lower: &[f32],
    ty: usize,
    y_start: usize,
) {
    let bins = params.bins;
    let cols = grid.cols;
    let range = 1.0 / norm.inv_range;
    let keep = 1.0 - params.amount;
    let last_centre = grid.tile_rows() - 1;
    output
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(band_row, out_row)| {
            let y = y_start + band_row;
            let yf = y as f32;
            let wy = if ty < last_centre && yf >= grid.centres_y[ty] {
                (yf - grid.centres_y[ty]) / (grid.centres_y[ty + 1] - grid.centres_y[ty])
            } else {
                0.0
            };
            let in_row = &input[y * cols..(y + 1) * cols];
            for (x, (out, &v)) in out_row.iter_mut().zip(in_row.iter()).enumerate() {
                if !v.is_finite() {
                    *out = v;
                    continue;
                }
                let pos = norm.position(v);
                let b0 = pos as usize;
                let b1 = (b0 + 1).min(bins - 1);
                let t = pos - b0 as f32;
                let wx = &grid.weights_x[x];
                let top = {
                    let a = sample_lut(upper, bins, wx.lower, b0, b1, t);
                    let b = sample_lut(upper, bins, wx.upper, b0, b1, t);
                    a + (b - a) * wx.weight
                };
                let bottom = {
                    let a = sample_lut(lower, bins, wx.lower, b0, b1, t);
                    let b = sample_lut(lower, bins, wx.upper, b0, b1, t);
                    a + (b - a) * wx.weight
                };
                let equalised = norm.min + (top + (bottom - top) * wy) * range;
                let blended = v * keep + equalised * params.amount;
                *out = blended.clamp(norm.min, norm.min + range);
            }
        });
}

pub fn lhe(image: &Array2<f32>, cfg: &LheConfig) -> Result<LheResult> {
    lhe_with_progress(image, cfg, None)
}

pub fn lhe_with_progress(
    image: &Array2<f32>,
    cfg: &LheConfig,
    progress: Option<&ProgressHandle>,
) -> Result<LheResult> {
    let params = LheParams::from_config(cfg)?;
    let (rows, cols) = image.dim();
    let (input_min, input_max) = finite_min_max(image);
    let range = input_max - input_min;
    let degenerate = rows == 0 || cols == 0 || !range.is_finite() || range < FLAT_RANGE_EPSILON;
    if degenerate || params.amount <= 0.0 {
        if let Some(p) = progress {
            p.emit_complete();
        }
        return Ok(LheResult {
            image: image.clone(),
            input_min,
            input_max,
        });
    }

    let input = contiguous(image);
    let grid = TileGrid::new(rows, cols, params.radius);
    let norm = Normaliser {
        min: input_min,
        inv_range: 1.0 / range,
        top_bin: (params.bins - 1) as f32,
    };
    let tile_rows = grid.tile_rows();
    if let Some(p) = progress {
        p.set_total(tile_rows as u64);
    }

    let mut output = vec![0.0f32; rows * cols];
    let mut current = tile_row_luts(&input, &grid, &params, &norm, 0);
    for ty in 0..tile_rows {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
        }
        let next = if ty + 1 < tile_rows {
            Some(tile_row_luts(&input, &grid, &params, &norm, ty + 1))
        } else {
            None
        };
        let (y_start, y_end) = grid.band_bounds[ty];
        if y_end > y_start {
            let lower = next.as_deref().unwrap_or(&current);
            equalise_band(
                &input,
                &mut output[y_start * cols..y_end * cols],
                &grid,
                &params,
                &norm,
                &current,
                lower,
                ty,
                y_start,
            );
        }
        if let Some(p) = progress {
            p.tick_with_stage(&format!("equalising tile row {}/{}", ty + 1, tile_rows));
        }
        if let Some(n) = next {
            current = n;
        }
    }

    if let Some(p) = progress {
        p.emit_complete();
    }

    Ok(LheResult {
        image: Array2::from_shape_vec((rows, cols), output).expect("output matches input shape"),
        input_min,
        input_max,
    })
}

pub fn lhe_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    cfg: &LheConfig,
) -> Result<(Array2<f32>, Array2<f32>, Array2<f32>)> {
    if r.dim() != g.dim() || g.dim() != b.dim() {
        bail!(
            "channel dimension mismatch: R={:?} G={:?} B={:?}",
            r.dim(),
            g.dim(),
            b.dim()
        );
    }
    let luminance = rgb_to_luminance(r, g, b);
    let equalised = lhe(&luminance, cfg)?;
    Ok(apply_luminance_ratio(r, g, b, &luminance, &equalised.image))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: usize = 128;
    const COLS: usize = 128;

    fn low_contrast_step_image() -> Array2<f32> {
        let mut image = Array2::from_elem((ROWS, COLS), 0.5f32);
        for y in 32..96 {
            for x in 32..96 {
                image[[y, x]] = if x < 64 { 0.45 } else { 0.55 };
            }
        }
        image[[0, 0]] = 0.0;
        image[[ROWS - 1, COLS - 1]] = 1.0;
        image
    }

    fn region_variance(image: &Array2<f32>, y0: usize, y1: usize, x0: usize, x1: usize) -> f64 {
        let mut values = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                values.push(image[[y, x]] as f64);
            }
        }
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
    }

    fn config(kernel_radius: usize, contrast_limit: f32, amount: f32) -> LheConfig {
        LheConfig {
            kernel_radius,
            contrast_limit,
            amount,
            hist_bits: 8,
            circular: true,
        }
    }

    fn assert_same_bits(a: &Array2<f32>, b: &Array2<f32>) {
        assert_eq!(a.dim(), b.dim());
        for (va, vb) in a.iter().zip(b.iter()) {
            assert_eq!(va.to_bits(), vb.to_bits(), "bit mismatch {} vs {}", va, vb);
        }
    }

    #[test]
    fn flat_image_is_unchanged() {
        let mut image = Array2::from_elem((64, 64), 0.37f32);
        image[[3, 3]] = f32::NAN;
        let out = lhe(&image, &LheConfig::default()).unwrap();
        assert_same_bits(&out.image, &image);
        assert_eq!(out.input_min, 0.37);
        assert_eq!(out.input_max, 0.37);
    }

    #[test]
    fn step_inside_a_tile_gains_local_contrast_and_amount_zero_is_identity() {
        let image = low_contrast_step_image();
        let before = region_variance(&image, 56, 72, 56, 72);
        assert!((before - 0.0025).abs() < 1e-6, "input variance {}", before);

        let out = lhe(&image, &config(16, 64.0, 1.0)).unwrap();
        let after = region_variance(&out.image, 56, 72, 56, 72);
        assert!(
            after > before * 4.0,
            "local variance should grow: before {} after {}",
            before,
            after
        );
        assert_eq!(out.input_min, 0.0);
        assert_eq!(out.input_max, 1.0);

        let identity = lhe(&image, &config(16, 64.0, 0.0)).unwrap();
        assert_same_bits(&identity.image, &image);
    }

    #[test]
    fn amount_blends_linearly_between_input_and_equalised() {
        let image = low_contrast_step_image();
        let full = lhe(&image, &config(16, 64.0, 1.0)).unwrap().image;
        let half = lhe(&image, &config(16, 64.0, 0.5)).unwrap().image;
        for y in 40..88 {
            for x in 40..88 {
                let expected = 0.5 * image[[y, x]] + 0.5 * full[[y, x]];
                assert!(
                    (half[[y, x]] - expected).abs() < 1e-5,
                    "blend mismatch at ({},{}): {} vs {}",
                    y,
                    x,
                    half[[y, x]],
                    expected
                );
            }
        }
    }

    #[test]
    fn nan_region_is_untouched_and_neighbours_stay_finite() {
        let mut image = low_contrast_step_image();
        for y in 10..20 {
            for x in 10..20 {
                image[[y, x]] = f32::NAN;
            }
        }
        let out = lhe(&image, &config(16, 8.0, 1.0)).unwrap().image;
        for y in 0..ROWS {
            for x in 0..COLS {
                let inside = (10..20).contains(&y) && (10..20).contains(&x);
                assert_eq!(
                    out[[y, x]].is_nan(),
                    inside,
                    "nan mismatch at ({},{})",
                    y,
                    x
                );
            }
        }
        assert!(out[[20, 20]].is_finite());
        assert!(out[[9, 15]].is_finite());
    }

    #[test]
    fn contrast_limit_effect_is_monotonic() {
        let image = low_contrast_step_image();
        let before = region_variance(&image, 56, 72, 56, 72);
        let weak = lhe(&image, &config(16, 1.0, 1.0)).unwrap().image;
        let medium = lhe(&image, &config(16, 8.0, 1.0)).unwrap().image;
        let strong = lhe(&image, &config(16, 64.0, 1.0)).unwrap().image;
        let v_weak = region_variance(&weak, 56, 72, 56, 72);
        let v_medium = region_variance(&medium, 56, 72, 56, 72);
        let v_strong = region_variance(&strong, 56, 72, 56, 72);
        assert!(
            v_weak <= v_medium && v_medium <= v_strong,
            "{} {} {}",
            v_weak,
            v_medium,
            v_strong
        );
        assert!(
            v_strong > v_weak * 2.0,
            "limit 64 should equalise far more than limit 1"
        );
        assert!(
            (v_weak - before).abs() < before * 0.5,
            "limit 1 should stay close to the input: before {} after {}",
            before,
            v_weak
        );
    }

    #[test]
    fn output_stays_within_the_input_range() {
        let image = low_contrast_step_image();
        for bits in LHE_HIST_BITS_ALLOWED {
            let cfg = LheConfig {
                hist_bits: bits,
                ..config(16, 64.0, 1.0)
            };
            let out = lhe(&image, &cfg).unwrap().image;
            for v in out.iter() {
                assert!(*v >= 0.0 && *v <= 1.0, "value {} outside input range", v);
            }
        }
    }

    #[test]
    fn square_and_circular_kernels_both_equalise() {
        let image = low_contrast_step_image();
        let before = region_variance(&image, 56, 72, 56, 72);
        let square = LheConfig {
            circular: false,
            ..config(16, 64.0, 1.0)
        };
        let out = lhe(&image, &square).unwrap().image;
        assert!(region_variance(&out, 56, 72, 56, 72) > before * 4.0);
    }

    #[test]
    fn kernel_larger_than_the_image_degrades_to_global_equalisation() {
        let image = low_contrast_step_image();
        let out = lhe(&image, &config(512, 64.0, 1.0)).unwrap().image;
        assert_eq!(out.dim(), image.dim());
        assert!(out.iter().all(|v| v.is_finite()));
        let grid = TileGrid::new(ROWS, COLS, 512);
        assert_eq!(grid.tile_rows(), 1);
        assert_eq!(grid.tile_cols(), 1);
    }

    #[test]
    fn tile_geometry_covers_every_row_exactly_once() {
        let grid = TileGrid::new(100, 70, 16);
        assert_eq!(grid.tile_rows(), 7);
        assert_eq!(grid.tile_cols(), 5);
        let covered: usize = grid.band_bounds.iter().map(|(s, e)| e - s).sum();
        assert_eq!(covered, 100);
        assert_eq!(grid.band_bounds[0].0, 0);
        assert_eq!(grid.band_bounds[6].1, 100);
        let w = &grid.weights_x[69];
        assert_eq!((w.lower, w.upper), (4, 4));
        let mid = &grid.weights_x[30];
        assert_eq!((mid.lower, mid.upper), (1, 2));
        assert!(mid.weight > 0.0 && mid.weight < 1.0);
    }

    #[test]
    fn invalid_histogram_depth_is_rejected() {
        let image = low_contrast_step_image();
        let cfg = LheConfig {
            hist_bits: 9,
            ..LheConfig::default()
        };
        let err = lhe(&image, &cfg).unwrap_err().to_string();
        assert!(err.contains("hist_bits"), "{}", err);
    }

    #[test]
    fn rgb_variant_keeps_channel_ratios() {
        let lum = low_contrast_step_image();
        let r = lum.mapv(|v| v * 1.0);
        let g = lum.mapv(|v| v * 0.6);
        let b = lum.mapv(|v| v * 0.3);
        let (nr, ng, nb) = lhe_rgb(&r, &g, &b, &config(16, 64.0, 1.0)).unwrap();
        let mut changed = 0usize;
        for y in 40..88 {
            for x in 40..88 {
                assert!((ng[[y, x]] / nr[[y, x]] - 0.6).abs() < 1e-3);
                assert!((nb[[y, x]] / nr[[y, x]] - 0.3).abs() < 1e-3);
                if (nr[[y, x]] - r[[y, x]]).abs() > 1e-3 {
                    changed += 1;
                }
            }
        }
        assert!(
            changed > 100,
            "luminance equalisation should change the channels"
        );
    }

    #[test]
    fn rgb_variant_rejects_mismatched_channels() {
        let r = Array2::from_elem((8, 8), 0.5f32);
        let g = Array2::from_elem((8, 4), 0.5f32);
        assert!(lhe_rgb(&r, &g, &r, &LheConfig::default()).is_err());
    }
}
