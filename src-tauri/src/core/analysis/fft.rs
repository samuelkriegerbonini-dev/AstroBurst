use anyhow::Result;
use ndarray::Array2;
use rayon::prelude::*;

use crate::math::complex;
use crate::math::fft::{self, FftEngine2D};
use crate::math::window;

const MAX_DISPLAY_SIZE: usize = 1024;

pub struct FftResult {
    pub spectrum: Array2<f32>,
    pub display_width: usize,
    pub display_height: usize,
    pub original_size: usize,
    pub windowed: bool,
}

pub fn compute_power_spectrum(data: &Array2<f32>) -> Result<FftResult> {
    compute_power_spectrum_opts(data, true)
}

pub fn compute_power_spectrum_opts(data: &Array2<f32>, apply_window: bool) -> Result<FftResult> {
    let (rows, cols) = data.dim();
    let size = rows.max(cols).next_power_of_two();

    let engine = FftEngine2D::<f32>::new(size, size);

    let mut buf = if apply_window {
        let hann_row = window::hann_symmetric::<f32>(rows);
        let hann_col = window::hann_symmetric::<f32>(cols);
        fft::prepare_windowed_buffer(data, &hann_row, &hann_col, size, size)
    } else {
        fft::prepare_buffer_no_window(data, size, size)
    };

    engine.forward_2d(&mut buf);

    let half = size / 2;
    let shifted_log: Vec<f32> = (0..size * size)
        .into_par_iter()
        .map(|idx| {
            let r = idx / size;
            let c = idx % size;
            let sr = (r + half) % size;
            let sc = (c + half) % size;
            let mag = complex::norm(buf[sr * size + sc]);
            (1.0 + mag).ln()
        })
        .collect();
    let spectrum = Array2::from_shape_vec((size, size), shifted_log).unwrap();

    let display = if size > MAX_DISPLAY_SIZE {
        downsample_area_average(&spectrum, MAX_DISPLAY_SIZE, MAX_DISPLAY_SIZE)
    } else {
        spectrum
    };

    let (dh, dw) = display.dim();

    Ok(FftResult {
        spectrum: display,
        display_width: dw,
        display_height: dh,
        original_size: size,
        windowed: apply_window,
    })
}

fn downsample_area_average(src: &Array2<f32>, target_h: usize, target_w: usize) -> Array2<f32> {
    let (src_h, src_w) = src.dim();
    let scale_y = src_h as f64 / target_h as f64;
    let scale_x = src_w as f64 / target_w as f64;

    let result_data: Vec<f32> = (0..target_h * target_w)
        .into_par_iter()
        .map(|idx| {
            let ty = idx / target_w;
            let tx = idx % target_w;

            let y0 = (ty as f64 * scale_y) as usize;
            let y1 = (((ty + 1) as f64 * scale_y) as usize).min(src_h);
            let x0 = (tx as f64 * scale_x) as usize;
            let x1 = (((tx + 1) as f64 * scale_x) as usize).min(src_w);

            let count = ((y1 - y0) * (x1 - x0)) as f32;
            if count == 0.0 {
                return 0.0;
            }

            let block = src.slice(ndarray::s![y0..y1, x0..x1]);
            block.sum() / count
        })
        .collect();

    Array2::from_shape_vec((target_h, target_w), result_data).unwrap()
}
