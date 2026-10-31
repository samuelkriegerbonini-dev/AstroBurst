use ndarray::{Array2, Zip};
use rayon::prelude::*;

use super::MaskError;
use crate::core::imaging::contour::{gaussian_blur_nan, MAX_SMOOTH_SIGMA};

const BOX_PASSES: usize = 3;
const TRANSPOSE_TILE: usize = 64;

pub fn feather(mask: &Array2<f32>, sigma_px: f32) -> Array2<f32> {
    let sigma = f64::from(sigma_px);
    if sigma.is_finite() && sigma > MAX_SMOOTH_SIGMA && !mask.is_empty() {
        box_feather(mask, box_radius(sigma))
    } else {
        gaussian_blur_nan(mask, sigma)
    }
}

pub fn blend(base: &Array2<f32>, processed: &Array2<f32>, mask: &Array2<f32>) -> Result<Array2<f32>, MaskError> {
    if processed.dim() != base.dim() || mask.dim() != base.dim() {
        return Err(MaskError::BlendDims { base: base.dim(), processed: processed.dim(), mask: mask.dim() });
    }
    Ok(Zip::from(base).and(processed).and(mask).par_map_collect(|&b, &p, &m| blend_value(b, p, m)))
}

fn blend_value(base: f32, processed: f32, m: f32) -> f32 {
    if !base.is_finite() {
        processed
    } else if m.is_nan() || m <= 0.0 {
        base
    } else if m >= 1.0 || !processed.is_finite() {
        processed
    } else {
        (f64::from(base) + f64::from(m) * (f64::from(processed) - f64::from(base))) as f32
    }
}

fn box_radius(sigma_px: f64) -> usize {
    let width = (12.0 * sigma_px * sigma_px / BOX_PASSES as f64 + 1.0).sqrt();
    ((width - 1.0) / 2.0).round() as usize
}

fn box_feather(mask: &Array2<f32>, radius: usize) -> Array2<f32> {
    let (rows, cols) = mask.dim();
    let radius = radius.min(rows.max(cols));
    let mut values = mask.as_slice().map_or_else(|| mask.iter().copied().collect(), <[f32]>::to_vec);
    box_lines(&mut values, cols, radius);
    let mut by_column = transposed(&values, rows, cols);
    box_lines(&mut by_column, rows, radius);
    Array2::from_shape_vec((rows, cols), transposed(&by_column, cols, rows))
        .expect("a transposed plane keeps the pixel count of the mask")
}

fn box_lines(values: &mut [f32], len: usize, radius: usize) {
    values.par_chunks_mut(len).for_each_init(
        || vec![0.0f32; len],
        |scratch, line| {
            for _ in 0..BOX_PASSES {
                box_line(line, scratch, radius);
                line.copy_from_slice(scratch);
            }
        },
    );
}

fn box_line(line: &[f32], out: &mut [f32], radius: usize) {
    let mut window = Window::default();
    line.iter().take(radius + 1).for_each(|&v| window.add(v));
    for (j, px) in out.iter_mut().enumerate() {
        *px = if line[j].is_finite() { window.mean() } else { f32::NAN };
        if let Some(&entering) = line.get(j + radius + 1) {
            window.add(entering);
        }
        if let Some(leaving) = j.checked_sub(radius) {
            window.remove(line[leaving]);
        }
    }
}

fn transposed(values: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; values.len()];
    out.par_chunks_mut(TRANSPOSE_TILE * rows).enumerate().for_each(|(tile, block)| {
        let first = tile * TRANSPOSE_TILE;
        let width = block.len() / rows;
        for (i, row) in values.chunks_exact(cols).enumerate() {
            for (k, &v) in row[first..first + width].iter().enumerate() {
                block[k * rows + i] = v;
            }
        }
    });
    out
}

#[derive(Default)]
struct Window {
    sum: f64,
    finite: usize,
    zeros: usize,
    ones: usize,
}

impl Window {
    fn add(&mut self, v: f32) {
        if v.is_finite() {
            self.sum += f64::from(v);
            self.finite += 1;
            self.zeros += usize::from(v == 0.0);
            self.ones += usize::from(v == 1.0);
        }
    }

    fn remove(&mut self, v: f32) {
        if v.is_finite() {
            self.sum -= f64::from(v);
            self.finite -= 1;
            self.zeros -= usize::from(v == 0.0);
            self.ones -= usize::from(v == 1.0);
        }
    }

    fn mean(&self) -> f32 {
        if self.zeros == self.finite {
            0.0
        } else if self.ones == self.finite {
            1.0
        } else {
            (self.sum / self.finite as f64) as f32
        }
    }
}
