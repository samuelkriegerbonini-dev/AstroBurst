use std::time::Instant;
use tauri::ipc::Response;
use rayon::prelude::*;
use crate::cmd::common::load_cached;
use crate::core::analysis::fft::{compute_power_spectrum, FftResult};

const PAR_THRESHOLD: usize = 1_000_000;

pub(in crate::cmd::analysis) const FFT_WINDOWED_FLAG: u32 = 1;
pub(in crate::cmd::analysis) const FFT_DOWNSAMPLED_FLAG: u32 = 2;
pub(in crate::cmd::analysis) const FFT_BINNED_FLAG: u32 = 4;
pub(in crate::cmd::analysis) const FFT_BIN_COLS_SHIFT: u32 = 8;
pub(in crate::cmd::analysis) const FFT_BIN_ROWS_SHIFT: u32 = 16;
const FFT_BIN_FACTOR_MAX: usize = 255;
pub(in crate::cmd::analysis) const FFT_HEADER_BYTES: usize = 40;

pub(in crate::cmd::analysis) fn fft_header(fft: &FftResult, dc: f32, max_val: f32, elapsed_ms: u32) -> Vec<u8> {
    let (rows, cols) = fft.spectrum.dim();
    let mut buf = Vec::with_capacity(FFT_HEADER_BYTES + rows * cols);
    buf.extend_from_slice(&(cols as u32).to_le_bytes());
    buf.extend_from_slice(&(rows as u32).to_le_bytes());
    buf.extend_from_slice(&dc.to_le_bytes());
    buf.extend_from_slice(&max_val.to_le_bytes());
    buf.extend_from_slice(&elapsed_ms.to_le_bytes());
    buf.extend_from_slice(&(fft.padded_cols as u32).to_le_bytes());
    buf.extend_from_slice(&(fft.padded_rows as u32).to_le_bytes());
    let bin_bits = if fft.bin_rows > 1 || fft.bin_cols > 1 {
        FFT_BINNED_FLAG
            | ((fft.bin_cols.min(FFT_BIN_FACTOR_MAX) as u32) << FFT_BIN_COLS_SHIFT)
            | ((fft.bin_rows.min(FFT_BIN_FACTOR_MAX) as u32) << FFT_BIN_ROWS_SHIFT)
    } else {
        0
    };
    let flags = FFT_WINDOWED_FLAG | if fft.downsampled { FFT_DOWNSAMPLED_FLAG } else { 0 } | bin_bits;
    buf.extend_from_slice(&flags.to_le_bytes());
    buf.extend_from_slice(&(fft.image_cols as u32).to_le_bytes());
    buf.extend_from_slice(&(fft.image_rows as u32).to_le_bytes());
    buf
}

#[tauri::command]
pub async fn compute_fft_spectrum(path: String) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let t0 = Instant::now();
        let fft_result = compute_power_spectrum(load_cached(&path)?.arr())?;
        let spectrum = &fft_result.spectrum;
        let (rows, cols) = spectrum.dim();
        let pixel_count = rows * cols;

        let slice = spectrum.as_slice().expect("FFT spectrum must be contiguous");

        let (min_val, max_val) = if pixel_count > PAR_THRESHOLD {
            (
                slice.par_iter().cloned().reduce(|| f32::INFINITY, f32::min),
                slice.par_iter().cloned().reduce(|| f32::NEG_INFINITY, f32::max),
            )
        } else {
            slice.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &v| (mn.min(v), mx.max(v)))
        };

        let range = (max_val - min_val).max(1e-10);
        let inv_range = 255.0 / range;
        let dc = spectrum[[rows / 2, cols / 2]];
        let elapsed_ms = t0.elapsed().as_millis() as u32;
        let mut buf = fft_header(&fft_result, dc, max_val, elapsed_ms);

        let pixels: Vec<u8> = if pixel_count > PAR_THRESHOLD {
            slice.par_iter().map(|&v| ((v - min_val) * inv_range) as u8).collect()
        } else {
            slice.iter().map(|&v| ((v - min_val) * inv_range) as u8).collect()
        };
        buf.extend(pixels);

        Ok(Response::new(buf))
    })
        .await
        .map_err(|e| format!("{}", e))?
        .map_err(|e| format!("{:#}", e))
}
