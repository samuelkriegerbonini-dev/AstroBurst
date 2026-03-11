use anyhow::Result;
use ndarray::{s, Array2, Axis, Zip};
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
                let t = std::f32::consts::PI * i as f32 / (rows as f32 - 1.0).max(1.0);
                0.5 * (1.0 - t.cos())
            })
            .collect();
        let hann_col: Vec<f32> = (0..cols)
            .map(|j| {
                let t = std::f32::consts::PI * j as f32 / (cols as f32 - 1.0).max(1.0);
                0.5 * (1.0 - t.cos())
            })
            .collect();

        Zip::indexed(data).for_each(|(y, x), &v| {
            let w = hann_row[y] * hann_col[x];
            let val = if v.is_finite() { v * w } else { 0.0 };
            padded[[y, x]] = Complex::new(val, 0.0);
        });
    } else {
        Zip::indexed(data).for_each(|(y, x), &v| {
            padded[[y, x]] = Complex::new(if v.is_finite() { v } else { 0.0 }, 0.0);
        });
    }

    let mut planner = FftPlanner::<f32>::new();

    let fft_row = planner.plan_fft_forward(size);
    padded.axis_iter_mut(Axis(0)).into_par_iter().for_each(|mut row| {
        let mut buf: Vec<Complex<f32>> = row.to_vec();
        fft_row.process(&mut buf);
        for (dst, src) in row.iter_mut().zip(buf.iter()) {
            *dst = *src;
        }
    });

    let fft_col = planner.plan_fft_forward(size);
    let mut transposed = padded.t().to_owned();
    transposed.axis_iter_mut(Axis(0)).into_par_iter().for_each(|mut row| {
        let mut buf: Vec<Complex<f32>> = row.to_vec();
        fft_col.process(&mut buf);
        for (dst, src) in row.iter_mut().zip(buf.iter()) {
            *dst = *src;
        }
    });
    padded = transposed.t().to_owned();

    let mut spectrum = Array2::<f32>::zeros((size, size));
    Zip::from(&mut spectrum).and(&padded).par_for_each(|s, c| {
        *s = (1.0 + c.norm()).ln();
    });

    let shifted = fft_shift(&spectrum);

    let display = if size > MAX_DISPLAY_SIZE {
        downsample_area_average(&shifted, MAX_DISPLAY_SIZE, MAX_DISPLAY_SIZE)
    } else {
        shifted
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

            let mut sum = 0.0f32;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += src[[y, x]];
                }
            }
            sum / count
        })
        .collect();

    Array2::from_shape_vec((target_h, target_w), result_data).unwrap()
}

fn fft_shift(data: &Array2<f32>) -> Array2<f32> {
    let (rows, cols) = data.dim();
    let hr = rows / 2;
    let hc = cols / 2;
    let mut shifted = Array2::<f32>::zeros((rows, cols));

    let q2 = data.slice(s![..hr, ..hc]);
    let q1 = data.slice(s![..hr, hc..]);
    let q4 = data.slice(s![hr.., ..hc]);
    let q3 = data.slice(s![hr.., hc..]);

    shifted.slice_mut(s![hr.., hc..]).assign(&q2);
    shifted.slice_mut(s![hr.., ..hc]).assign(&q1);
    shifted.slice_mut(s![..hr, hc..]).assign(&q4);
    shifted.slice_mut(s![..hr, ..hc]).assign(&q3);

    shifted
}
