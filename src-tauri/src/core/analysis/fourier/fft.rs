use anyhow::Result;
use ndarray::Array2;
use rayon::prelude::*;

use crate::math::complex;
use crate::math::fft::{self, FftEngine2D};
use crate::math::window;

const MAX_DISPLAY_SIZE: usize = 1024;
pub const MAX_TRANSFORM_SIZE: usize = 4096;

pub struct FftResult {
    pub spectrum: Array2<f32>,
    pub display_width: usize,
    pub display_height: usize,
    pub padded_rows: usize,
    pub padded_cols: usize,
    pub downsampled: bool,
    pub image_rows: usize,
    pub image_cols: usize,
    pub bin_rows: usize,
    pub bin_cols: usize,
}

fn bin_factor(len: usize) -> usize {
    len.div_ceil(MAX_TRANSFORM_SIZE).max(1)
}

fn block_average(data: &Array2<f32>, bin_rows: usize, bin_cols: usize) -> Array2<f32> {
    let (rows, cols) = data.dim();
    Array2::from_shape_fn((rows.div_ceil(bin_rows), cols.div_ceil(bin_cols)), |(y, x)| {
        let block = data.slice(ndarray::s![
            y * bin_rows..((y + 1) * bin_rows).min(rows),
            x * bin_cols..((x + 1) * bin_cols).min(cols)
        ]);
        block.sum() / block.len() as f32
    })
}

pub fn compute_power_spectrum(data: &Array2<f32>) -> Result<FftResult> {
    let (image_rows, image_cols) = data.dim();
    let (bin_rows, bin_cols) = (bin_factor(image_rows), bin_factor(image_cols));
    let binned = (bin_rows > 1 || bin_cols > 1).then(|| block_average(data, bin_rows, bin_cols));
    let data = binned.as_ref().unwrap_or(data);
    let (rows, cols) = data.dim();
    let fft_rows = rows.next_power_of_two();
    let fft_cols = cols.next_power_of_two();

    let engine = FftEngine2D::<f32>::new(fft_rows, fft_cols);

    let hann_row = window::hann_symmetric::<f32>(rows);
    let hann_col = window::hann_symmetric::<f32>(cols);
    let mut buf = fft::prepare_windowed_buffer(data, &hann_row, &hann_col, fft_rows, fft_cols);

    engine.forward_2d(&mut buf);

    let half_r = (fft_rows + 1) / 2;
    let half_c = (fft_cols + 1) / 2;
    let shifted_log: Vec<f32> = (0..fft_rows * fft_cols)
        .into_par_iter()
        .map(|idx| {
            let r = idx / fft_cols;
            let c = idx % fft_cols;
            let sr = (r + half_r) % fft_rows;
            let sc = (c + half_c) % fft_cols;
            let mag = complex::norm(buf[sr * fft_cols + sc]);
            (1.0 + mag).ln()
        })
        .collect();
    let spectrum = Array2::from_shape_vec((fft_rows, fft_cols), shifted_log).unwrap();

    let downsampled = fft_rows > MAX_DISPLAY_SIZE || fft_cols > MAX_DISPLAY_SIZE;
    let display = if downsampled {
        let scale = MAX_DISPLAY_SIZE as f64 / fft_rows.max(fft_cols) as f64;
        let target_h = ((fft_rows as f64 * scale).round() as usize).max(1);
        let target_w = ((fft_cols as f64 * scale).round() as usize).max(1);
        downsample_area_average(&spectrum, target_h, target_w)
    } else {
        spectrum
    };

    let (dh, dw) = display.dim();

    Ok(FftResult {
        spectrum: display,
        display_width: dw,
        display_height: dh,
        padded_rows: fft_rows,
        padded_cols: fft_cols,
        downsampled,
        image_rows,
        image_cols,
        bin_rows,
        bin_cols,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| ((y * 7 + x * 3) % 11) as f32)
    }

    #[test]
    fn a_tall_image_under_the_display_cap_is_padded_per_axis_and_not_downsampled() {
        let result = compute_power_spectrum(&ramp(300, 20)).unwrap();
        assert_eq!((result.padded_rows, result.padded_cols), (512, 32));
        assert_eq!((result.display_height, result.display_width), (512, 32));
        assert_eq!(result.spectrum.dim(), (512, 32));
        assert!(!result.downsampled);
        assert_eq!((result.image_rows, result.image_cols), (300, 20));
        assert_eq!((result.bin_rows, result.bin_cols), (1, 1));
    }

    #[test]
    fn an_image_wider_than_the_transform_cap_is_block_averaged_first() {
        assert_eq!(MAX_TRANSFORM_SIZE, 4096);
        assert_eq!((bin_factor(4096), bin_factor(4097), bin_factor(20000), bin_factor(1), bin_factor(0)), (1, 2, 5, 1, 1));
        let averaged = block_average(&Array2::from_shape_fn((3, 5), |(y, x)| (10 * y + x) as f32), 2, 2);
        assert_eq!(averaged.dim(), (2, 3));
        assert_eq!(averaged.as_slice().unwrap(), &[5.5, 7.5, 9.0, 20.5, 22.5, 24.0]);

        let wave = Array2::from_shape_fn((2, 8192), |(_, x)| (2.0 * std::f32::consts::PI * x as f32 / 512.0).sin());
        let result = compute_power_spectrum(&wave).unwrap();
        assert_eq!((result.bin_rows, result.bin_cols), (1, 2));
        assert_eq!((result.padded_rows, result.padded_cols), (2, 4096));
        assert_eq!((result.image_rows, result.image_cols), (2, 8192));
    }

    #[test]
    fn power_spectrum_padding_is_capped() {
        let wide = compute_power_spectrum(&ramp(1, 20000)).unwrap();
        assert!(wide.padded_cols <= 4096, "a 20000 px wide image is padded to {} columns", wide.padded_cols);
        assert_eq!((wide.image_rows, wide.image_cols), (1, 20000));
        assert!(wide.downsampled);
        assert_eq!(wide.spectrum.dim().1, 1024);

        let tall = compute_power_spectrum(&ramp(9000, 3)).unwrap();
        assert!(tall.padded_rows <= 4096, "a 9000 px tall image is padded to {} rows", tall.padded_rows);
        assert_eq!((tall.image_rows, tall.image_cols), (9000, 3));
    }

    #[test]
    fn an_image_over_the_display_cap_is_downsampled_and_keeps_its_padded_size_per_axis() {
        let result = compute_power_spectrum(&ramp(1025, 20)).unwrap();
        assert_eq!((result.padded_rows, result.padded_cols), (2048, 32));
        assert_eq!((result.display_height, result.display_width), (1024, 16));
        assert_eq!(result.spectrum.dim(), (1024, 16));
        assert!(result.downsampled);
        assert_eq!((result.image_rows, result.image_cols), (1025, 20));
    }
}
