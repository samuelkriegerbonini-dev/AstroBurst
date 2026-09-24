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

fn area_cells(src: usize, dst: usize) -> Vec<(usize, Vec<f64>)> {
    if src == 0 {
        return vec![(0, Vec::new()); dst];
    }
    (0..dst)
        .map(|d| {
            let lo = d * src;
            let hi = (d + 1) * src;
            let first = lo / dst;
            let last = (hi - 1) / dst;
            let weights = (first..=last)
                .map(|s| {
                    let a = (s * dst).max(lo);
                    let b = ((s + 1) * dst).min(hi);
                    (b - a) as f64
                })
                .collect();
            (first, weights)
        })
        .collect()
}

fn box_reduce(image: &Array2<f32>, out_rows: usize, out_cols: usize) -> Result<Array2<f32>> {
    let (rows, cols) = image.dim();
    let standard = image.as_standard_layout();
    let slice = standard
        .as_slice()
        .ok_or_else(|| anyhow::anyhow!("image buffer is not contiguous"))?;
    let row_cells = area_cells(rows, out_rows);
    let col_cells = area_cells(cols, out_cols);
    let mut buf = vec![0.0f32; out_rows * out_cols];

    buf.par_chunks_mut(out_cols)
        .zip(row_cells.par_iter())
        .for_each(|(row, (y0, wy))| {
            for (px, (x0, wx)) in row.iter_mut().zip(&col_cells) {
                let mut sum = 0.0f64;
                let mut weight = 0.0f64;
                for (dy, &w_y) in wy.iter().enumerate() {
                    let base = (y0 + dy) * cols + x0;
                    for (dx, &w_x) in wx.iter().enumerate() {
                        let v = slice[base + dx];
                        if v.is_finite() {
                            let w = w_y * w_x;
                            sum += v as f64 * w;
                            weight += w;
                        }
                    }
                }
                *px = if weight > 0.0 { (sum / weight) as f32 } else { f32::NAN };
            }
        });

    Array2::from_shape_vec((out_rows, out_cols), buf)
        .map_err(|e| anyhow::anyhow!("Reshape failed: {}", e))
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

    let ky = if scale_y > 2.0 { scale_y.floor() as usize } else { 1 };
    let kx = if scale_x > 2.0 { scale_x.floor() as usize } else { 1 };
    if ky >= 2 || kx >= 2 {
        let reduced = box_reduce(image, (src_rows / ky).max(1), (src_cols / kx).max(1))?;
        return resample_image(&reduced, target_rows, target_cols);
    }
    let half_shift_y = (scale_y - 1.0) * 0.5;
    let half_shift_x = (scale_x - 1.0) * 0.5;

    let standard = image.as_standard_layout();
    let slice = standard
        .as_slice()
        .ok_or_else(|| anyhow::anyhow!("image buffer is not contiguous"))?;
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

    let cd_cards = [("CD1_1", scale_x), ("CD1_2", scale_y), ("CD2_1", scale_x), ("CD2_2", scale_y)];
    let has_cd = cd_cards.iter().any(|(key, _)| header.get_f64(key).is_some());
    if has_cd {
        for (key, factor) in cd_cards {
            if let Some(v) = header.get_f64(key) {
                updates.push((key.to_string(), v * factor));
            }
        }
    } else {
        let pc11 = header.get_f64("PC1_1");
        let pc12 = header.get_f64("PC1_2");
        let pc21 = header.get_f64("PC2_1");
        let pc22 = header.get_f64("PC2_2");
        let has_pc = pc11.is_some() || pc12.is_some() || pc21.is_some() || pc22.is_some();

        if has_pc {
            updates.push(("PC1_1".to_string(), pc11.unwrap_or(1.0) * scale_x));
            updates.push(("PC1_2".to_string(), pc12.unwrap_or(0.0) * scale_y));
            updates.push(("PC2_1".to_string(), pc21.unwrap_or(0.0) * scale_x));
            updates.push(("PC2_2".to_string(), pc22.unwrap_or(1.0) * scale_y));
        } else {
            if let Some(cdelt1) = header.get_f64("CDELT1") {
                updates.push(("CDELT1".to_string(), cdelt1 * scale_x));
            }
            if let Some(cdelt2) = header.get_f64("CDELT2") {
                updates.push(("CDELT2".to_string(), cdelt2 * scale_y));
            }
        }
    }

    updates.extend(physical_updates(header, scale_x, scale_y));
    updates.extend(sip_updates(header, scale_x, scale_y));

    updates.push(("NAXIS1".to_string(), tgt_cols as f64));
    updates.push(("NAXIS2".to_string(), tgt_rows as f64));

    updates
}

fn physical_updates(header: &HduHeader, scale_x: f64, scale_y: f64) -> Vec<(String, f64)> {
    let keys = ["LTV1", "LTV2", "LTM1_1", "LTM1_2", "LTM2_1", "LTM2_2"];
    if !keys.iter().any(|k| header.get_f64(k).is_some()) {
        return Vec::new();
    }
    let ltv1 = header.get_f64("LTV1").unwrap_or(0.0);
    let ltv2 = header.get_f64("LTV2").unwrap_or(0.0);
    let mut out = vec![
        ("LTV1".to_string(), (ltv1 - 0.5) / scale_x + 0.5),
        ("LTV2".to_string(), (ltv2 - 0.5) / scale_y + 0.5),
        ("LTM1_1".to_string(), header.get_f64("LTM1_1").unwrap_or(1.0) / scale_x),
        ("LTM2_2".to_string(), header.get_f64("LTM2_2").unwrap_or(1.0) / scale_y),
    ];
    if let Some(v) = header.get_f64("LTM1_2") {
        out.push(("LTM1_2".to_string(), v / scale_x));
    }
    if let Some(v) = header.get_f64("LTM2_1") {
        out.push(("LTM2_1".to_string(), v / scale_y));
    }
    out
}

fn sip_coefficient_factor(key: &str, scale_x: f64, scale_y: f64) -> Option<f64> {
    let (prefix, rest) = ["AP_", "BP_", "A_", "B_"]
        .iter()
        .find_map(|p| key.strip_prefix(p).map(|rest| (*p, rest)))?;
    let mut parts = rest.split('_');
    let (p, q) = match (parts.next(), parts.next(), parts.next()) {
        (Some(p), Some(q), None) => (p, q),
        _ => return None,
    };
    if p.is_empty() || q.is_empty() || !p.chars().chain(q.chars()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let p: i32 = p.parse().ok().filter(|v| *v <= 64)?;
    let q: i32 = q.parse().ok().filter(|v| *v <= 64)?;
    if prefix.starts_with('A') {
        Some(scale_x.powi(p - 1) * scale_y.powi(q))
    } else {
        Some(scale_x.powi(p) * scale_y.powi(q - 1))
    }
}

fn sip_updates(header: &HduHeader, scale_x: f64, scale_y: f64) -> Vec<(String, f64)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (raw_key, _) in &header.cards {
        let key = raw_key.trim();
        if !seen.insert(key.to_string()) {
            continue;
        }
        let factor = match key {
            "A_DMAX" => Some(1.0 / scale_x),
            "B_DMAX" => Some(1.0 / scale_y),
            _ => sip_coefficient_factor(key, scale_x, scale_y),
        };
        if let (Some(factor), Some(value)) = (factor, header.get_f64(key)) {
            out.push((key.to_string(), value * factor));
        }
    }
    out
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

    fn header_with(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in cards {
            h.set(k, v.to_string());
        }
        h
    }

    fn update(updates: &[(String, f64)], key: &str) -> Option<f64> {
        updates.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }

    #[test]
    fn box_reduction_keeps_the_trailing_rows_and_columns() {
        let img = Array2::from_shape_fn((7, 7), |(r, c)| (r + 10 * c) as f32);
        let out = resample_image(&img, 2, 2).unwrap();
        let first = (0.0 + 1.0 + 2.0 + 0.5 * 3.0) / 3.5;
        let second = (0.5 * 3.0 + 4.0 + 5.0 + 6.0) / 3.5;
        for (r, expected_row) in [first, second].into_iter().enumerate() {
            for (c, expected_col) in [first, second].into_iter().enumerate() {
                let expected = expected_row + 10.0 * expected_col;
                assert!((out[[r, c]] as f64 - expected).abs() < 1e-4, "({r},{c}) {} vs {expected}", out[[r, c]]);
            }
        }
    }

    #[test]
    fn downsampled_rows_stay_on_the_grid_the_wcs_update_assumes() {
        let (rows, target) = (1001usize, 300usize);
        let img = Array2::from_shape_fn((rows, 4), |(r, _)| r as f32);
        let out = resample_image(&img, target, 4).unwrap();
        let scale = rows as f64 / target as f64;
        for t in [10usize, 150, 280, 296] {
            let expected = (t as f64 + 0.5) * scale - 0.5;
            let got = out[[t, 2]] as f64;
            assert!((got - expected).abs() < 0.15, "row {t}: {got} vs {expected}");
        }
    }

    #[test]
    fn cd_matrix_without_cd1_1_is_scaled_card_by_card() {
        let h = header_with(&[
            ("CRPIX1", "100.5"),
            ("CRPIX2", "100.5"),
            ("CD1_2", "1.0E-4"),
            ("CD2_1", "-1.0E-4"),
            ("CDELT1", "3.0E-4"),
        ]);
        let u = compute_wcs_updates(&h, (200, 200), (100, 50));
        assert!((update(&u, "CD1_2").unwrap() - 2.0e-4).abs() < 1e-15);
        assert!((update(&u, "CD2_1").unwrap() + 4.0e-4).abs() < 1e-15);
        assert!(update(&u, "CD1_1").is_none() && update(&u, "CD2_2").is_none());
        assert!(update(&u, "CDELT1").is_none() && update(&u, "PC1_1").is_none());
    }

    #[test]
    fn sip_coefficients_are_rescaled_with_the_pixel_grid() {
        let h = header_with(&[
            ("CTYPE1", "RA---TAN-SIP"),
            ("CTYPE2", "DEC--TAN-SIP"),
            ("A_ORDER", "2"),
            ("B_ORDER", "2"),
            ("A_2_0", "1.0E-5"),
            ("A_1_1", "-2.0E-6"),
            ("A_0_2", "3.0E-6"),
            ("B_2_0", "4.0E-6"),
            ("B_1_1", "5.0E-6"),
            ("B_0_2", "-6.0E-6"),
            ("AP_1_0", "1.0E-3"),
            ("AP_2_0", "-1.0E-5"),
            ("BP_0_1", "2.0E-3"),
            ("BP_0_2", "7.0E-6"),
            ("A_DMAX", "3.0"),
            ("B_DMAX", "8.0"),
        ]);
        let (sx, sy) = (2.0, 4.0);
        let u = compute_wcs_updates(&h, (400, 400), (100, 200));
        assert!(update(&u, "A_ORDER").is_none());
        let coef = |key: &str| update(&u, key).unwrap_or_else(|| panic!("{key} missing from {u:?}"));
        let f = |x: f64, y: f64| 1.0e-5 * x * x - 2.0e-6 * x * y + 3.0e-6 * y * y;
        let g = |x: f64, y: f64| 4.0e-6 * x * x + 5.0e-6 * x * y - 6.0e-6 * y * y;
        let f_new = |x: f64, y: f64| coef("A_2_0") * x * x + coef("A_1_1") * x * y + coef("A_0_2") * y * y;
        let g_new = |x: f64, y: f64| coef("B_2_0") * x * x + coef("B_1_1") * x * y + coef("B_0_2") * y * y;
        for (x, y) in [(150.0, -80.0), (-190.0, 170.0), (30.0, 5.0)] {
            assert!((sx * f_new(x / sx, y / sy) - f(x, y)).abs() < 1e-12, "A at ({x},{y})");
            assert!((sy * g_new(x / sx, y / sy) - g(x, y)).abs() < 1e-12, "B at ({x},{y})");
        }
        assert!((coef("AP_1_0") - 1.0e-3).abs() < 1e-15);
        assert!((coef("AP_2_0") + 1.0e-5 * sx).abs() < 1e-15);
        assert!((coef("BP_0_1") - 2.0e-3).abs() < 1e-15);
        assert!((coef("BP_0_2") - 7.0e-6 * sy).abs() < 1e-15);
        assert!((coef("A_DMAX") - 1.5).abs() < 1e-12);
        assert!((coef("B_DMAX") - 2.0).abs() < 1e-12);
    }

    #[test]
    fn physical_coordinates_follow_the_resampled_grid() {
        let h = header_with(&[("LTV1", "-100.0"), ("LTV2", "-50.0"), ("LTM1_1", "1.0"), ("LTM2_2", "1.0")]);
        let (sx, sy) = (2.0, 4.0);
        let u = compute_wcs_updates(&h, (400, 400), (100, 200));
        let ltm11 = update(&u, "LTM1_1").unwrap();
        let ltm22 = update(&u, "LTM2_2").unwrap();
        let ltv1 = update(&u, "LTV1").unwrap();
        let ltv2 = update(&u, "LTV2").unwrap();
        for (px, py) in [(101.0, 51.0), (180.5, 90.25), (400.0, 60.0)] {
            let (lx, ly) = (px - 100.0, py - 50.0);
            let (nx, ny) = ((lx - 0.5) / sx + 0.5, (ly - 0.5) / sy + 0.5);
            assert!((ltm11 * px + ltv1 - nx).abs() < 1e-12, "x for {px}");
            assert!((ltm22 * py + ltv2 - ny).abs() < 1e-12, "y for {py}");
        }
        let none = compute_wcs_updates(&header_with(&[("CRPIX1", "1")]), (400, 400), (100, 200));
        assert!(update(&none, "LTV1").is_none() && update(&none, "LTM1_1").is_none());
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
