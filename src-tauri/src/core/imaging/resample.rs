use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::types::header::HduHeader;

pub struct ResampleResult {
    pub image: Array2<f32>,
    pub header_updates: Vec<(String, f64)>,
    pub original_dims: [usize; 2],
    pub resampled_dims: [usize; 2],
}

#[inline]
fn catmull_rom(t: f64) -> f64 {
    let a = -0.5;
    let abs_t = t.abs();
    if abs_t <= 1.0 {
        (a + 2.0) * abs_t * abs_t * abs_t - (a + 3.0) * abs_t * abs_t + 1.0
    } else if abs_t <= 2.0 {
        a * abs_t * abs_t * abs_t - 5.0 * a * abs_t * abs_t + 8.0 * a * abs_t - 4.0 * a
    } else {
        0.0
    }
}

#[inline]
fn sample_clamped(image: &Array2<f32>, row: i64, col: i64) -> f32 {
    let (rows, cols) = image.dim();
    let r = row.clamp(0, rows as i64 - 1) as usize;
    let c = col.clamp(0, cols as i64 - 1) as usize;
    image[[r, c]]
}

fn bicubic_sample(image: &Array2<f32>, y: f64, x: f64) -> f32 {
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - ix as f64;
    let fy = y - iy as f64;

    let mut wx = [0.0f64; 4];
    let mut wy = [0.0f64; 4];
    for i in 0..4 {
        wx[i] = catmull_rom(fx - (i as f64 - 1.0));
        wy[i] = catmull_rom(fy - (i as f64 - 1.0));
    }

    let mut val = 0.0f64;
    for j in 0..4 {
        let row = iy + j as i64 - 1;
        let mut row_val = 0.0f64;
        for i in 0..4 {
            let col = ix + i as i64 - 1;
            row_val += sample_clamped(image, row, col) as f64 * wx[i];
        }
        val += row_val * wy[j];
    }

    val as f32
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

    let rows: Vec<Vec<f32>> = (0..target_rows)
        .into_par_iter()
        .map(|ty| {
            let sy = ty as f64 * scale_y + (scale_y - 1.0) * 0.5;
            let mut row = Vec::with_capacity(target_cols);
            for tx in 0..target_cols {
                let sx = tx as f64 * scale_x + (scale_x - 1.0) * 0.5;
                row.push(bicubic_sample(image, sy, sx));
            }
            row
        })
        .collect();

    let flat: Vec<f32> = rows.into_iter().flatten().collect();
    let result = Array2::from_shape_vec((target_rows, target_cols), flat)
        .map_err(|e| anyhow::anyhow!("Reshape failed: {}", e))?;

    Ok(result)
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

    let mut updates = Vec::new();

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
