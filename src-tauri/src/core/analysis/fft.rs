use anyhow::Result;
use ndarray::{Array2, Axis, Zip};
use rayon::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};

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

    let mut padded = Array2::<Complex<f32>>::zeros((size, size));

    if apply_window {
        let hann_row: Vec<f32> = (0..rows)
            .map(|i| {
                let t = 2.0 * std::f32::consts::PI * i as f32 / (rows as f32 - 1.0).max(1.0);
                0.5 * (1.0 - t.cos())
            })
            .collect();
        let hann_col: Vec<f32> = (0..cols)
            .map(|j| {
                let t = 2.0 * std::f32::consts::PI * j as f32 / (cols as f32 - 1.0).max(1.0);
                0.5 * (1.0 - t.cos())
            })
            .collect();

        Zip::indexed(data).for_each(|(y, x), &v| {
            let val = if v.is_finite() { v * hann_row[y] * hann_col[x] } else { 0.0 };
            padded[[y, x]] = Complex::new(val, 0.0);
        });
    } else {
        Zip::indexed(data).for_each(|(y, x), &v| {
            padded[[y, x]] = Complex::new(if v.is_finite() { v } else { 0.0 }, 0.0);
        });
    }

    let mut planner = FftPlanner::<f32>::new();
    let zero = Complex::new(0.0f32, 0.0);

    let fft_fwd = planner.plan_fft_forward(size);

    padded.axis_iter_mut(Axis(0)).into_par_iter().for_each_init(
        || vec![zero; size],
        |buf, mut row| {
            buf.iter_mut().zip(row.iter()).for_each(|(b, &r)| *b = r);
            fft_fwd.process(buf);
            row.iter_mut().zip(buf.iter()).for_each(|(d, &s)| *d = s);
        },
    );

    let fft_col = planner.plan_fft_forward(size);
    let mut transposed = padded.t().to_owned();
    transposed.axis_iter_mut(Axis(0)).into_par_iter().for_each_init(
        || vec![zero; size],
        |buf, mut row| {
            buf.iter_mut().zip(row.iter()).for_each(|(b, &r)| *b = r);
            fft_col.process(buf);
            row.iter_mut().zip(buf.iter()).for_each(|(d, &s)| *d = s);
        },
    );
    padded = transposed.t().to_owned();

    let half = size / 2;
    let shifted_log: Vec<f32> = (0..size * size)
        .into_par_iter()
        .map(|idx| {
            let r = idx / size;
            let c = idx % size;
            let sr = (r + half) % size;
            let sc = (c + half) % size;
            (1.0 + padded[[sr, sc]].norm()).ln()
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
