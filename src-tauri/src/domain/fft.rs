use ndarray::{Array2, ArrayViewMut2, Axis};
use num_complex::Complex;
use rayon::prelude::*;
use rustfft::FftPlanner;

pub struct FftResult {
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub dc_magnitude: f64,
    pub max_magnitude: f64,
}

pub fn compute_power_spectrum(data: &Array2<f32>) -> FftResult {
    let (rows, cols) = data.dim();

    let mut buf: Vec<Complex<f32>> = data
        .as_slice()
        .expect("Array2 must be contiguous")
        .iter()
        .map(|&v| Complex::new(v, 0.0))
        .collect();

    fft_rows(&mut buf, rows, cols);
    fft_cols(&mut buf, rows, cols);
    fft_shift(&mut buf, rows, cols);

    let magnitude: Vec<f32> = buf
        .par_iter()
        .map(|c| c.norm())
        .collect();

    let dc_mag = magnitude[rows / 2 * cols + cols / 2] as f64;
    let max_mag = magnitude
        .par_iter()
        .copied()
        .reduce(|| 0.0f32, f32::max) as f64;

    let log_mag: Vec<f32> = magnitude
        .par_iter()
        .map(|&m| (1.0 + m).ln())
        .collect();

    let log_max = log_mag
        .par_iter()
        .copied()
        .reduce(|| f32::NEG_INFINITY, f32::max);

    let inv = if log_max > 0.0 { 255.0 / log_max } else { 0.0 };

    let pixels: Vec<u8> = log_mag
        .par_iter()
        .map(|&v| (v * inv).clamp(0.0, 255.0) as u8)
        .collect();

    FftResult {
        pixels,
        width: cols,
        height: rows,
        dc_magnitude: dc_mag,
        max_magnitude: max_mag,
    }
}

fn fft_rows(buf: &mut [Complex<f32>], _rows: usize, cols: usize) {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(cols);

    buf.par_chunks_mut(cols).for_each(|row| {
        fft.process(row);
    });
}

fn fft_cols(buf: &mut [Complex<f32>], rows: usize, cols: usize) {
    let mut view = ArrayViewMut2::from_shape((rows, cols), buf)
        .expect("Erro ao mapear dimensões do buffer");

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(rows);

    view.axis_iter_mut(Axis(1)).into_par_iter().for_each(|mut col| {
        let mut col_buf = col.to_vec();

        fft.process(&mut col_buf);

        for (idx, val) in col.iter_mut().enumerate() {
            *val = col_buf[idx];
        }
    });
}

fn fft_shift(buf: &mut [Complex<f32>], rows: usize, cols: usize) {
    let half_r = rows / 2;
    let half_c = cols / 2;

    let mut shifted = vec![Complex::new(0.0f32, 0.0); rows * cols];

    shifted
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(dst_r, dst_row)| {
            let src_r = (dst_r + half_r) % rows;
            for dst_c in 0..cols {
                let src_c = (dst_c + half_c) % cols;
                dst_row[dst_c] = buf[src_r * cols + src_c];
            }
        });

    buf.copy_from_slice(&shifted);
}