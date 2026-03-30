use std::collections::VecDeque;

use ndarray::Array2;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::math::{sigma_clipped_stats, f64_cmp};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedStar {
    pub x: f64,
    pub y: f64,
    pub flux: f64,
    pub fwhm: f64,
    pub peak: f64,
    pub npix: usize,
    pub snr: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionResult {
    pub stars: Vec<DetectedStar>,
    pub background_median: f64,
    pub background_sigma: f64,
    pub threshold_sigma: f64,
    pub image_width: usize,
    pub image_height: usize,
}

pub fn estimate_background(image: &Array2<f32>, tile_size: usize) -> (f64, f64) {
    let (rows, cols) = image.dim();
    let step = tile_size.max(16);

    let mut tile_coords = Vec::new();
    let mut y = 0;
    while y < rows {
        let mut x = 0;
        while x < cols {
            tile_coords.push((y, x));
            x += step;
        }
        y += step;
    }

    let results: Vec<(f64, f64)> = tile_coords
        .par_iter()
        .filter_map(|&(ty, tx)| {
            let ye = (ty + step).min(rows);
            let xe = (tx + step).min(cols);
            let mut vals: Vec<f32> = Vec::with_capacity((ye - ty) * (xe - tx));
            for r in ty..ye {
                for c in tx..xe {
                    let v = image[[r, c]];
                    if v.is_finite() {
                        vals.push(v);
                    }
                }
            }
            if vals.len() >= 8 {
                let (med, sig) = sigma_clipped_stats(&mut vals, 3.0, 2);
                Some((med, sig))
            } else {
                None
            }
        })
        .collect();

    if results.is_empty() {
        return (0.0, 1.0);
    }

    let mut medians: Vec<f64> = results.iter().map(|r| r.0).collect();
    let mut sigmas: Vec<f64> = results.iter().map(|r| r.1).collect();

    medians.sort_unstable_by(f64_cmp);
    sigmas.sort_unstable_by(f64_cmp);

    let global_median = medians[medians.len() / 2];
    let global_sigma = sigmas[sigmas.len() / 2];

    (global_median, global_sigma.max(1e-10))
}

pub fn detect_stars(image: &Array2<f32>, sigma_threshold: f64) -> DetectionResult {
    let (rows, cols) = image.dim();
    let tile_size = (rows.min(cols) / 8).max(32).min(256);
    let (bg_median, bg_sigma) = estimate_background(image, tile_size);

    let threshold = bg_median + sigma_threshold * bg_sigma;

    let mut visited = Array2::<bool>::default((rows, cols));
    let mut stars = Vec::new();

    for r in 1..rows - 1 {
        for c in 1..cols - 1 {
            let v = image[[r, c]] as f64;
            if v <= threshold || visited[[r, c]] || !v.is_finite() {
                continue;
            }

            let mut queue = VecDeque::new();
            let mut component: Vec<(usize, usize)> = Vec::new();
            queue.push_back((r, c));
            visited[[r, c]] = true;

            while let Some((cr, cc)) = queue.pop_front() {
                component.push((cr, cc));

                for (dr, dc) in &[(-1i32, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)] {
                    let nr = cr as i32 + dr;
                    let nc = cc as i32 + dc;
                    if nr < 0 || nc < 0 || nr >= rows as i32 || nc >= cols as i32 {
                        continue;
                    }
                    let nr = nr as usize;
                    let nc = nc as usize;
                    if visited[[nr, nc]] {
                        continue;
                    }
                    let nv = image[[nr, nc]] as f64;
                    if nv > threshold && nv.is_finite() {
                        visited[[nr, nc]] = true;
                        queue.push_back((nr, nc));
                    }
                }
            }

            let npix = component.len();
            if npix < 3 || npix > 5000 {
                continue;
            }
            let mut sum_flux = 0.0f64;
            let mut sum_x = 0.0f64;
            let mut sum_y = 0.0f64;
            let mut peak_val = 0.0f64;

            for &(pr, pc) in &component {
                let v = (image[[pr, pc]] as f64 - bg_median).max(0.0);
                sum_flux += v;
                sum_x += pc as f64 * v;
                sum_y += pr as f64 * v;
                if v > peak_val {
                    peak_val = v;
                }
            }

            if sum_flux <= 0.0 {
                continue;
            }

            let cx = sum_x / sum_flux;
            let cy = sum_y / sum_flux;

            let mut sum_r2 = 0.0f64;
            for &(pr, pc) in &component {
                let v = (image[[pr, pc]] as f64 - bg_median).max(0.0);
                let dx = pc as f64 - cx;
                let dy = pr as f64 - cy;
                sum_r2 += (dx * dx + dy * dy) * v;
            }
            let sigma_star = (sum_r2 / sum_flux).sqrt();
            let fwhm = sigma_star * 2.355;

            if fwhm < 0.5 || fwhm > 30.0 {
                continue;
            }

            let snr = peak_val / bg_sigma;

            stars.push(DetectedStar {
                x: cx,
                y: cy,
                flux: sum_flux,
                fwhm,
                peak: peak_val,
                npix,
                snr,
            });
        }
    }

    stars.sort_by(|a, b| b.flux.partial_cmp(&a.flux).unwrap_or(std::cmp::Ordering::Equal));

    let dedup_radius = 3.0f64;
    let dedup_r2 = dedup_radius * dedup_radius;
    let cell_size = dedup_radius;
    let mut grid: std::collections::HashMap<(usize, usize), Vec<usize>> =
        std::collections::HashMap::with_capacity(stars.len());
    let mut deduped = Vec::with_capacity(stars.len());

    for (i, star) in stars.iter().enumerate() {
        let gx = (star.x / cell_size) as usize;
        let gy = (star.y / cell_size) as usize;

        let mut too_close = false;
        'outer: for ny in gy.saturating_sub(1)..=gy + 1 {
            for nx in gx.saturating_sub(1)..=gx + 1 {
                if let Some(cell) = grid.get(&(ny, nx)) {
                    for &j in cell {
                        let dx = star.x - stars[j].x;
                        let dy = star.y - stars[j].y;
                        if dx * dx + dy * dy < dedup_r2 {
                            too_close = true;
                            break 'outer;
                        }
                    }
                }
            }
        }

        if !too_close {
            grid.entry((gy, gx)).or_default().push(i);
            deduped.push(star.clone());
        }
    }

    DetectionResult {
        stars: deduped,
        background_median: bg_median,
        background_sigma: bg_sigma,
        threshold_sigma: sigma_threshold,
        image_width: cols,
        image_height: rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_image(rows: usize, cols: usize) -> Array2<f32> {
        let mut img = Array2::from_elem((rows, cols), 100.0f32);
        for r in 0..rows {
            for c in 0..cols {
                img[[r, c]] += ((r * 7 + c * 13) % 17) as f32 * 0.5;
            }
        }
        let stars = [(50, 50, 5000.0), (100, 200, 3000.0), (200, 150, 8000.0)];
        for (sy, sx, peak) in &stars {
            for dy in -5i32..=5 {
                for dx in -5i32..=5 {
                    let r = (*sy as i32 + dy) as usize;
                    let c = (*sx as i32 + dx) as usize;
                    if r < rows && c < cols {
                        let d2 = (dx * dx + dy * dy) as f64;
                        let sigma = 2.0;
                        let val = peak * (-d2 / (2.0 * sigma * sigma)).exp();
                        img[[r, c]] += val as f32;
                    }
                }
            }
        }
        img
    }

    #[test]
    fn test_detect_stars_finds_sources() {
        let img = make_test_image(300, 300);
        let result = detect_stars(&img, 5.0);
        assert!(result.stars.len() >= 3, "Should detect at least 3 stars, got {}", result.stars.len());
        assert!(result.background_sigma > 0.0);
    }

    #[test]
    fn test_detect_stars_brightest_first() {
        let img = make_test_image(300, 300);
        let result = detect_stars(&img, 5.0);
        if result.stars.len() >= 2 {
            assert!(result.stars[0].flux >= result.stars[1].flux);
        }
    }

    #[test]
    fn test_detect_stars_centroid_accuracy() {
        let img = make_test_image(300, 300);
        let result = detect_stars(&img, 5.0);
        let brightest = &result.stars[0];
        assert!((brightest.x - 150.0).abs() < 2.0, "X centroid off: {}", brightest.x);
        assert!((brightest.y - 200.0).abs() < 2.0, "Y centroid off: {}", brightest.y);
    }

    #[test]
    fn test_detect_stars_empty_image() {
        let img = Array2::from_elem((100, 100), 50.0f32);
        let result = detect_stars(&img, 5.0);
        assert!(result.stars.is_empty(), "Flat image should have no detections");
    }

    #[test]
    fn test_background_estimation() {
        let img = Array2::from_elem((200, 200), 100.0f32);
        let (med, sig) = estimate_background(&img, 64);
        assert!((med - 100.0).abs() < 1.0);
        assert!(sig < 1.0, "Flat image should have near-zero sigma");
    }
}
