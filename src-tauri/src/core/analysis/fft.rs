use anyhow::Result;
use ndarray::{s, Array2, Axis, Zip};
use rayon::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};

pub fn compute_power_spectrum(data: &Array2<f32>) -> Result<Array2<f32>> {
    let (rows, cols) = data.dim();
    let size = rows.max(cols).next_power_of_two();

    let mut padded = Array2::<Complex<f32>>::zeros((size, size));
    Zip::indexed(data).for_each(|(y, x), &v| {
        padded[[y, x]] = Complex::new(if v.is_finite() { v } else { 0.0 }, 0.0);
    });

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
        *s = (c.norm_sqr() + 1e-10).log10();
    });

    Ok(fft_shift(&spectrum))
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
