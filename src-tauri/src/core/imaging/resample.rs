use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::imaging::sampling;
use crate::types::header::HduHeader;

pub struct ResampleResult {
    pub image: Array2<f32>,
    pub header_updates: Vec<(String, f64)>,
    pub original_dims: [usize; 2],
    pub resampled_dims: [usize; 2],
}

#[inline]
pub fn catmull_rom(t: f64) -> f64 {
    sampling::catmull_rom(t)
}

#[inline]
pub fn bicubic_sample(slice: &[f32], rows: usize, cols: usize, y: f64, x: f64) -> f32 {
    sampling::bicubic_sample(slice, rows, cols, y, x)
}

pub fn resample_image(
    image: &Array2<f32>,
    target_rows: usize,
    target_cols: usize,
) -> Result<Array2<f32>> {
    let (src_rows, src_cols) = image.dim();

    if target_rows == 0 || target_cols == 0 {
        bail!("Target dimensions must be > 0");
    }

    if target_rows == src_rows && target_cols == src_cols {
        return Ok(image.clone());
    }

    let scale_y = src_rows as f64 / target_rows as f64;
    let scale_x = src_cols as f64 / target_cols as f64;
    let half_shift_y = (scale_y - 1.0) * 0.5;
    let half_shift_x = (scale_x - 1.0) * 0.5;

    let slice = image.as_slice().expect("contiguous");
    let total = target_rows * target_cols;
    let mut buf = vec![0.0f32; total];

    buf.par_chunks_mut(target_cols)
        .enumerate()
        .for_each(|(ty, row)| {
            let sy = ty as f64 * scale_y + half_shift_y;
            for (tx, pixel) in row.iter_mut().enumerate() {
                let sx = tx as f64 * scale_x + half_shift_x;
                *pixel = sampling::bicubic_sample(slice, src_rows, src_cols, sy, sx);
            }
        });

    Array2::from_shape_vec((target_rows, target_cols), buf)
        .map_err(|e| anyhow::anyhow!("Reshape failed: {}", e))
}

pub fn compute_wcs_updates(
    header: &HduHeader,
    original_dims: (usize, usize),
    target_dims: (usize, usize),
) -> Vec<(String, f64)> {
    let (orig_rows, orig_cols) = original_dims;
    let (tgt_rows, tgt_cols) = target_dims;

    let scale_x = orig_cols as f64 / tgt_cols as f64;
    let scale_y = orig_rows as f64 / tgt_rows as f64;

    let mut updates = Vec::with_capacity(8);

    if let Some(crpix1) = header.get_f64("CRPIX1") {
        updates.push(("CRPIX1".to_string(), (crpix1 - 0.5) / scale_x + 0.5));
    }
    if let Some(crpix2) = header.get_f64("CRPIX2") {
        updates.push(("CRPIX2".to_string(), (crpix2 - 0.5) / scale_y + 0.5));
    }

    if let Some(cd1_1) = header.get_f64("CD1_1") {
        updates.push(("CD1_1".to_string(), cd1_1 * scale_x));
        if let Some(cd1_2) = header.get_f64("CD1_2") {
            updates.push(("CD1_2".to_string(), cd1_2 * scale_y));
        }
        if let Some(cd2_1) = header.get_f64("CD2_1") {
            updates.push(("CD2_1".to_string(), cd2_1 * scale_x));
        }
        if let Some(cd2_2) = header.get_f64("CD2_2") {
            updates.push(("CD2_2".to_string(), cd2_2 * scale_y));
        }
    } else {
        if let Some(cdelt1) = header.get_f64("CDELT1") {
            updates.push(("CDELT1".to_string(), cdelt1 * scale_x));
        }
        if let Some(cdelt2) = header.get_f64("CDELT2") {
            updates.push(("CDELT2".to_string(), cdelt2 * scale_y));
        }
    }

    updates.push(("NAXIS1".to_string(), tgt_cols as f64));
    updates.push(("NAXIS2".to_string(), tgt_rows as f64));

    updates
}

pub fn resample_with_wcs(
    image: &Array2<f32>,
    header: &HduHeader,
    target_rows: usize,
    target_cols: usize,
) -> Result<ResampleResult> {
    let (orig_rows, orig_cols) = image.dim();
    let header_updates = compute_wcs_updates(
        header,
        (orig_rows, orig_cols),
        (target_rows, target_cols),
    );

    Ok(ResampleResult {
        image: resample_image(image, target_rows, target_cols)?,
        header_updates,
        original_dims: [orig_cols, orig_rows],
        resampled_dims: [target_cols, target_rows],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catmull_rom_at_zero() {
        assert!((catmull_rom(0.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_at_one() {
        assert!(catmull_rom(1.0).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_at_two() {
        assert!(catmull_rom(2.0).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_symmetry() {
        assert!((catmull_rom(0.5) - catmull_rom(-0.5)).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_partition_of_unity() {
        for i in 0..=10 {
            let t = i as f64 / 10.0;
            let sum = catmull_rom(t + 1.0) + catmull_rom(t) + catmull_rom(t - 1.0) + catmull_rom(t - 2.0);
            assert!((sum - 1.0).abs() < 1e-10, "partition of unity failed at t={}: sum={}", t, sum);
        }
    }

    #[test]
    fn test_resample_identity() {
        let img = Array2::from_shape_fn((100, 100), |(r, c)| (r + c) as f32);
        let result = resample_image(&img, 100, 100).unwrap();
        assert_eq!(result.dim(), (100, 100));
        for r in 0..100 {
            for c in 0..100 {
                assert!((result[[r, c]] - img[[r, c]]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn test_resample_downscale() {
        let img = Array2::from_elem((200, 200), 42.0f32);
        let result = resample_image(&img, 100, 100).unwrap();
        assert_eq!(result.dim(), (100, 100));
        for r in 0..100 {
            for c in 0..100 {
                assert!((result[[r, c]] - 42.0).abs() < 1.0);
            }
        }
    }

    #[test]
    fn test_resample_upscale() {
        let img = Array2::from_elem((50, 50), 10.0f32);
        let result = resample_image(&img, 100, 100).unwrap();
        assert_eq!(result.dim(), (100, 100));
    }
}
