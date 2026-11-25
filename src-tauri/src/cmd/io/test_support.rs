use ndarray::Array2;

use crate::cmd::common::MAX_PREVIEW_DIM;
use crate::core::imaging::stf::{apply_stf, ImageStats, StfParams};

pub(crate) fn wide_sky(rows: usize) -> Array2<f32> {
    Array2::from_shape_fn((rows, 2 * MAX_PREVIEW_DIM + 8), |(r, c)| {
        let noise = ((r * 7919 + c * 104_729) % 41) as f32 - 20.0;
        let texture = if (r + c) % 2 == 0 { 0.0 } else { 90.0 };
        let star = if r == 2 && c % 997 == 11 { 4000.0 } else { 0.0 };
        1000.0 + 0.01 * c as f32 + noise + texture + star
    })
}

pub(crate) fn assert_matches_gpu_view(png_path: &str, arr: &Array2<f32>, stf: &StfParams, stats: &ImageStats) {
    let encoded = crate::infra::ipc::encode_with_header_downsampled(arr, MAX_PREVIEW_DIM).unwrap();
    let width = u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
    let height = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let values: Vec<f32> = encoded[16..].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let reduced = Array2::from_shape_vec((height as usize, width as usize), values).unwrap();
    let expected = apply_stf(&reduced, stf, stats);
    let png = image::open(png_path).unwrap().to_luma8();
    assert_eq!((png.width(), png.height()), (width, height));
    let off = png.as_raw().iter().zip(&expected).filter(|(a, b)| a.abs_diff(**b) > 1).count();
    assert_eq!(off, 0, "{off} of {} preview pixels of {png_path} differ from the GPU view", expected.len());
}
