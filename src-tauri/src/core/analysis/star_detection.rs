use std::collections::VecDeque;

use ndarray::Array2;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::analysis::confidence;
use crate::core::imaging::stats::is_valid_pixel;
use crate::math::{sigma_clipped_stats, f64_cmp};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedStar {
    pub x: f64,
    pub y: f64,
    pub flux: f64,
    pub fwhm: f64,
    pub eccentricity: f64,
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
                    if is_valid_pixel(v) {
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

const DEBLEND_MIN_SEP: f64 = 3.0;
const DEBLEND_MIN_CONTRAST: f64 = 0.1;

fn deblend_component(
    image: &Array2<f32>,
    component: &[(usize, usize)],
    bg_median: f64,
) -> Vec<Vec<(usize, usize)>> {
    if component.len() < 8 {
        return vec![component.to_vec()];
    }
    let (rows, cols) = image.dim();

    let mut maxima: Vec<(usize, usize, f64)> = Vec::new();
    for &(pr, pc) in component {
        let v = image[[pr, pc]] as f64;
        if !v.is_finite() {
            continue;
        }
        let mut is_max = true;
        'neighbors: for dr in -1i32..=1 {
            for dc in -1i32..=1 {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let nr = pr as i32 + dr;
                let nc = pc as i32 + dc;
                if nr < 0 || nc < 0 || nr >= rows as i32 || nc >= cols as i32 {
                    continue;
                }
                if image[[nr as usize, nc as usize]] as f64 > v {
                    is_max = false;
                    break 'neighbors;
                }
            }
        }
        if is_max {
            maxima.push((pr, pc, v - bg_median));
        }
    }

    let mut merged: Vec<(usize, usize, f64)> = Vec::with_capacity(maxima.len());
    let mut used = vec![false; maxima.len()];
    for i in 0..maxima.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut group = vec![i];
        let mut head = 0;
        while head < group.len() {
            let (gr, gc, gv) = maxima[group[head]];
            head += 1;
            for j in 0..maxima.len() {
                if used[j] {
                    continue;
                }
                let (jr, jc, jv) = maxima[j];
                if jv == gv
                    && (jr as i32 - gr as i32).abs() <= 1
                    && (jc as i32 - gc as i32).abs() <= 1
                {
                    used[j] = true;
                    group.push(j);
                }
            }
        }
        let n = group.len() as f64;
        let sr = group.iter().map(|&k| maxima[k].0 as f64).sum::<f64>() / n;
        let sc = group.iter().map(|&k| maxima[k].1 as f64).sum::<f64>() / n;
        merged.push((sr.round() as usize, sc.round() as usize, maxima[i].2));
    }
    let mut maxima = merged;

    if maxima.len() <= 1 {
        return vec![component.to_vec()];
    }

    maxima.sort_by(|a, b| b.2.total_cmp(&a.2));
    let top = maxima[0].2.max(1e-12);
    let mut kept: Vec<(usize, usize, f64)> = Vec::new();
    for m in maxima {
        if m.2 < DEBLEND_MIN_CONTRAST * top {
            break;
        }
        let far = kept.iter().all(|k| {
            let dx = m.1 as f64 - k.1 as f64;
            let dy = m.0 as f64 - k.0 as f64;
            (dx * dx + dy * dy).sqrt() >= DEBLEND_MIN_SEP
        });
        if far {
            kept.push(m);
        }
    }

    if kept.len() <= 1 {
        return vec![component.to_vec()];
    }

    let mut subs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); kept.len()];
    for &(pr, pc) in component {
        let mut best = 0usize;
        let mut best_d = f64::MAX;
        for (ki, k) in kept.iter().enumerate() {
            let dx = pc as f64 - k.1 as f64;
            let dy = pr as f64 - k.0 as f64;
            let d = dx * dx + dy * dy;
            if d < best_d {
                best_d = d;
                best = ki;
            }
        }
        subs[best].push((pr, pc));
    }
    subs.retain(|s| s.len() >= 3);
    if subs.is_empty() {
        return vec![component.to_vec()];
    }
    subs
}

fn truncated_second_moment_fraction(peak: f64, cut: f64) -> f64 {
    if cut <= 0.0 {
        return 1.0;
    }
    let peak_over_cut = peak / cut;
    if !peak_over_cut.is_finite() || peak_over_cut <= 1.0 {
        return 0.0;
    }
    let u = peak_over_cut.ln();
    let tail = (-u).exp();
    (1.0 - tail * (1.0 + u)) / (1.0 - tail)
}

fn measure_component(
    image: &Array2<f32>,
    component: &[(usize, usize)],
    bg_median: f64,
    bg_sigma: f64,
    cut_above_bg: f64,
) -> Option<DetectedStar> {
    let npix = component.len();
    if npix < 3 {
        return None;
    }

    let mut sum_flux = 0.0f64;
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut peak_val = 0.0f64;

    for &(pr, pc) in component {
        let v = (image[[pr, pc]] as f64 - bg_median).max(0.0);
        sum_flux += v;
        sum_x += pc as f64 * v;
        sum_y += pr as f64 * v;
        peak_val = peak_val.max(v);
    }

    if sum_flux <= 0.0 {
        return None;
    }

    let cx = sum_x / sum_flux;
    let cy = sum_y / sum_flux;

    let mut sum_r2 = 0.0f64;
    let mut sum_xx = 0.0f64;
    let mut sum_yy = 0.0f64;
    let mut sum_xy = 0.0f64;
    for &(pr, pc) in component {
        let v = (image[[pr, pc]] as f64 - bg_median).max(0.0);
        let dx = pc as f64 - cx;
        let dy = pr as f64 - cy;
        sum_r2 += (dx * dx + dy * dy) * v;
        sum_xx += dx * dx * v;
        sum_yy += dy * dy * v;
        sum_xy += dx * dy * v;
    }
    let kept_fraction = truncated_second_moment_fraction(peak_val, cut_above_bg);
    if kept_fraction <= 0.0 {
        return None;
    }
    let sigma_star = (sum_r2 / (2.0 * sum_flux * kept_fraction)).sqrt();
    let fwhm = sigma_star * 2.3548200450309493;

    if fwhm < 0.5 || fwhm > 30.0 {
        return None;
    }

    let ixx = sum_xx / sum_flux;
    let iyy = sum_yy / sum_flux;
    let ixy = sum_xy / sum_flux;
    let trace = ixx + iyy;
    let det = (ixx * iyy - ixy * ixy).max(0.0);
    let disc = ((trace * trace / 4.0) - det).max(0.0).sqrt();
    let lambda1 = trace / 2.0 + disc;
    let lambda2 = (trace / 2.0 - disc).max(0.0);
    let eccentricity = if lambda1 > 1e-15 {
        (1.0 - lambda2 / lambda1).sqrt().clamp(0.0, 1.0)
    } else {
        0.0
    };

    let snr = confidence::compute_detection_snr(peak_val, bg_sigma);

    Some(DetectedStar {
        x: cx,
        y: cy,
        flux: sum_flux,
        fwhm,
        eccentricity,
        peak: peak_val,
        npix,
        snr,
    })
}

pub fn detect_stars(image: &Array2<f32>, sigma_threshold: f64) -> DetectionResult {
    let (rows, cols) = image.dim();

    if rows < 3 || cols < 3 {
        return DetectionResult {
            stars: Vec::new(),
            background_median: 0.0,
            background_sigma: 1.0,
            threshold_sigma: sigma_threshold,
            image_width: cols,
            image_height: rows,
        };
    }

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

            for pixels in deblend_component(image, &component, bg_median) {
                if let Some(star) = measure_component(image, &pixels, bg_median, bg_sigma, threshold - bg_median) {
                    stars.push(star);
                }
            }
        }
    }

    stars.sort_by(|a, b| b.flux.total_cmp(&a.flux));

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

    fn add_gaussian(img: &mut Array2<f32>, sy: usize, sx: usize, peak: f64, sigma: f64, clip: f32) {
        let (rows, cols) = img.dim();
        for dy in -8i32..=8 {
            for dx in -8i32..=8 {
                let r = sy as i32 + dy;
                let c = sx as i32 + dx;
                if r < 0 || c < 0 || r >= rows as i32 || c >= cols as i32 {
                    continue;
                }
                let d2 = (dx * dx + dy * dy) as f64;
                let val = peak * (-d2 / (2.0 * sigma * sigma)).exp();
                let cell = &mut img[[r as usize, c as usize]];
                *cell = (*cell + val as f32).min(clip);
            }
        }
    }

    fn component_above(img: &Array2<f32>, threshold: f32) -> Vec<(usize, usize)> {
        let (rows, cols) = img.dim();
        let mut out = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                if img[[r, c]] > threshold {
                    out.push((r, c));
                }
            }
        }
        out
    }

    #[test]
    fn test_deblend_keeps_saturated_plateau_whole() {
        let mut img = Array2::from_elem((41, 41), 100.0f32);
        add_gaussian(&mut img, 20, 20, 20000.0, 3.0, 5000.0);
        let component = component_above(&img, 150.0);
        let plateau = component.iter().filter(|&&(r, c)| img[[r, c]] >= 5000.0).count();
        assert!(plateau > 20, "expected a wide plateau, got {plateau} pixels");
        let subs = deblend_component(&img, &component, 100.0);
        assert_eq!(subs.len(), 1, "saturated star split into {} parts", subs.len());
        assert_eq!(subs[0].len(), component.len());
    }

    #[test]
    fn test_deblend_still_splits_two_distinct_peaks() {
        let mut img = Array2::from_elem((41, 41), 100.0f32);
        add_gaussian(&mut img, 20, 13, 5000.0, 2.0, f32::MAX);
        add_gaussian(&mut img, 20, 27, 5000.0, 2.0, f32::MAX);
        let component = component_above(&img, 150.0);
        let subs = deblend_component(&img, &component, 100.0);
        assert_eq!(subs.len(), 2, "two separated peaks should yield 2 parts, got {}", subs.len());
    }

    #[test]
    fn test_detect_stars_saturated_star_is_single_detection() {
        let mut img = make_test_image(300, 300);
        add_gaussian(&mut img, 150, 60, 20000.0, 3.0, 5000.0);
        let result = detect_stars(&img, 5.0);
        let near: Vec<&DetectedStar> = result
            .stars
            .iter()
            .filter(|s| (s.x - 60.0).abs() < 10.0 && (s.y - 150.0).abs() < 10.0)
            .collect();
        assert_eq!(near.len(), 1, "saturated star produced {} detections", near.len());
        assert!((near[0].x - 60.0).abs() < 1.0, "X centroid off: {}", near[0].x);
        assert!((near[0].y - 150.0).abs() < 1.0, "Y centroid off: {}", near[0].y);
        assert!(near[0].eccentricity < 0.3, "eccentricity too high: {}", near[0].eccentricity);
    }

    fn gaussian_noise(rows: usize, cols: usize, sigma: f64, seed: u64) -> Array2<f32> {
        let mut state = seed;
        let mut uniform = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        Array2::from_shape_fn((rows, cols), |_| {
            let g = (-2.0 * uniform().ln()).sqrt() * (2.0 * std::f64::consts::PI * uniform()).cos();
            (sigma * g) as f32
        })
    }

    fn noise_free_star(size: usize, bg: f32, amp: f64, sigma: f64) -> Array2<f32> {
        let c = (size / 2) as f64;
        Array2::from_shape_fn((size, size), |(y, x)| {
            let d2 = (x as f64 - c).powi(2) + (y as f64 - c).powi(2);
            bg + (amp * (-d2 / (2.0 * sigma * sigma)).exp()) as f32
        })
    }

    #[test]
    fn fwhm_of_a_threshold_truncated_star_does_not_depend_on_its_brightness() {
        let sigma = 2.5;
        let true_fwhm = 2.3548200450309493 * sigma;
        let cut = 50.0;
        for peak_over_cut in [2.0, 3.0, 10.0, 100.0] {
            let img = noise_free_star(41, 100.0, cut * peak_over_cut, sigma);
            let component = component_above(&img, 100.0 + cut as f32);
            let star = measure_component(&img, &component, 100.0, 10.0, cut).expect("star measured");
            assert!(
                (star.fwhm - true_fwhm).abs() / true_fwhm < 0.05,
                "peak {peak_over_cut}x the cut gave FWHM {} for a true {true_fwhm}",
                star.fwhm
            );
        }
    }

    #[test]
    fn sky_subtracted_background_keeps_its_negative_half() {
        let img = gaussian_noise(256, 256, 10.0, 7);
        let (median, sigma) = estimate_background(&img, 64);
        assert!(median.abs() < 1.0, "sky median {median} of a zero-mean sky");
        assert!((sigma - 10.0).abs() < 1.0, "sky sigma {sigma} for a true sigma of 10");

        let mut padded = img.clone();
        padded.slice_mut(ndarray::s![.., ..64]).fill(0.0);
        let (median, sigma) = estimate_background(&padded, 64);
        assert!(median.abs() < 1.0 && (sigma - 10.0).abs() < 1.0, "zero padding biased the sky: {median} {sigma}");
    }

    #[test]
    fn test_background_estimation() {
        let img = Array2::from_elem((200, 200), 100.0f32);
        let (med, sig) = estimate_background(&img, 64);
        assert!((med - 100.0).abs() < 1.0);
        assert!(sig < 1.0, "Flat image should have near-zero sigma");
    }
}
