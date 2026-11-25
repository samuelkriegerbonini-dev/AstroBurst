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

#[derive(Debug, Clone)]
pub struct StarExtent {
    pub star: DetectedStar,
    pub whole_component_fwhm: f64,
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

const FWHM_PER_SIGMA: f64 = 2.3548200450309493;
const CORE_MIN_PIXELS: usize = 9;

fn core_fwhm(
    image: &Array2<f32>,
    component: &[(usize, usize)],
    bg_median: f64,
    (cx, cy): (f64, f64),
    peak: f64,
    cut_above_bg: f64,
) -> Option<f64> {
    if component.len() <= CORE_MIN_PIXELS {
        return None;
    }
    let mut values: Vec<f64> = component
        .iter()
        .map(|&(pr, pc)| (image[[pr, pc]] as f64 - bg_median).max(0.0))
        .collect();
    values.select_nth_unstable_by(CORE_MIN_PIXELS - 1, |a, b| b.total_cmp(a));
    let level = values[CORE_MIN_PIXELS - 1].min(0.5 * peak);
    if level <= cut_above_bg {
        return None;
    }

    let mut sum = 0.0f64;
    let mut sum_dx = 0.0f64;
    let mut sum_dy = 0.0f64;
    let mut sum_r2 = 0.0f64;
    let mut faintest_kept = peak;
    let mut brightest_left_out = cut_above_bg;
    for &(pr, pc) in component {
        let v = (image[[pr, pc]] as f64 - bg_median).max(0.0);
        if v < level {
            brightest_left_out = brightest_left_out.max(v);
            continue;
        }
        faintest_kept = faintest_kept.min(v);
        let dx = pc as f64 - cx;
        let dy = pr as f64 - cy;
        sum += v;
        sum_dx += dx * v;
        sum_dy += dy * v;
        sum_r2 += (dx * dx + dy * dy) * v;
    }
    let offset2 = (sum_dx * sum_dx + sum_dy * sum_dy) / (sum * sum);
    let central_r2 = (sum_r2 / sum - offset2).max(0.0);
    let truncation_level = (faintest_kept * brightest_left_out).max(0.0).sqrt();
    let kept_fraction = truncated_second_moment_fraction(peak, truncation_level);
    Some((central_r2 / (2.0 * kept_fraction)).sqrt() * FWHM_PER_SIGMA)
}

fn measure_component(
    image: &Array2<f32>,
    component: &[(usize, usize)],
    bg_median: f64,
    bg_sigma: f64,
    cut_above_bg: f64,
) -> Option<StarExtent> {
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
    let component_fwhm = sigma_star * FWHM_PER_SIGMA;

    if component_fwhm < 0.5 || component_fwhm > 30.0 {
        return None;
    }
    let fwhm = core_fwhm(image, component, bg_median, (cx, cy), peak_val, cut_above_bg).unwrap_or(component_fwhm);

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

    let star = DetectedStar {
        x: cx,
        y: cy,
        flux: sum_flux,
        fwhm,
        eccentricity,
        peak: peak_val,
        npix,
        snr,
    };
    Some(StarExtent { star, whole_component_fwhm: component_fwhm })
}

pub fn detect_stars(image: &Array2<f32>, sigma_threshold: f64) -> DetectionResult {
    let (rows, cols) = image.dim();
    let (extents, (background_median, background_sigma)) = star_extents_and_background(image, sigma_threshold);
    DetectionResult {
        stars: extents.into_iter().map(|extent| extent.star).collect(),
        background_median,
        background_sigma,
        threshold_sigma: sigma_threshold,
        image_width: cols,
        image_height: rows,
    }
}

pub fn detect_star_extents(image: &Array2<f32>, sigma_threshold: f64) -> Vec<StarExtent> {
    star_extents_and_background(image, sigma_threshold).0
}

fn star_extents_and_background(image: &Array2<f32>, sigma_threshold: f64) -> (Vec<StarExtent>, (f64, f64)) {
    let (rows, cols) = image.dim();

    if rows < 3 || cols < 3 {
        return (Vec::new(), (0.0, 1.0));
    }

    let tile_size = (rows.min(cols) / 8).max(32).min(256);
    let (bg_median, bg_sigma) = estimate_background(image, tile_size);

    let threshold = bg_median + sigma_threshold * bg_sigma;

    let mut visited = Array2::<bool>::default((rows, cols));
    let mut extents = Vec::new();

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
                if let Some(extent) = measure_component(image, &pixels, bg_median, bg_sigma, threshold - bg_median) {
                    extents.push(extent);
                }
            }
        }
    }

    extents.sort_by(|a, b| b.star.flux.total_cmp(&a.star.flux));

    let dedup_radius = 3.0f64;
    let dedup_r2 = dedup_radius * dedup_radius;
    let cell_size = dedup_radius;
    let mut grid: std::collections::HashMap<(usize, usize), Vec<usize>> =
        std::collections::HashMap::with_capacity(extents.len());
    let mut deduped = Vec::with_capacity(extents.len());

    for (i, extent) in extents.iter().enumerate() {
        let star = &extent.star;
        let gx = (star.x / cell_size) as usize;
        let gy = (star.y / cell_size) as usize;

        let mut too_close = false;
        'outer: for ny in gy.saturating_sub(1)..=gy + 1 {
            for nx in gx.saturating_sub(1)..=gx + 1 {
                if let Some(cell) = grid.get(&(ny, nx)) {
                    for &j in cell {
                        let dx = star.x - extents[j].star.x;
                        let dy = star.y - extents[j].star.y;
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
            deduped.push(extent.clone());
        }
    }

    (deduped, (bg_median, bg_sigma))
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
            let star = measure_component(&img, &component, 100.0, 10.0, cut).expect("star measured").star;
            assert!(
                (star.fwhm - true_fwhm).abs() / true_fwhm < 0.05,
                "peak {peak_over_cut}x the cut gave FWHM {} for a true {true_fwhm}",
                star.fwhm
            );
        }
    }

    fn pixel_integrated_star(size: usize, (x0, y0): (f64, f64), bg: f32, amp: f64, profile: impl Fn(f64) -> f64) -> Array2<f32> {
        const SUBSAMPLES: usize = 9;
        let offset = |i: usize| (i as f64 + 0.5) / SUBSAMPLES as f64 - 0.5;
        Array2::from_shape_fn((size, size), |(y, x)| {
            let mut sum = 0.0;
            for j in 0..SUBSAMPLES {
                for i in 0..SUBSAMPLES {
                    sum += profile((x as f64 + offset(i) - x0).powi(2) + (y as f64 + offset(j) - y0).powi(2));
                }
            }
            bg + (amp * sum / (SUBSAMPLES * SUBSAMPLES) as f64) as f32
        })
    }

    fn moffat(fwhm: f64, beta: f64) -> impl Fn(f64) -> f64 {
        let alpha2 = (fwhm / 2.0).powi(2) / (2f64.powf(1.0 / beta) - 1.0);
        move |d2| (1.0 + d2 / alpha2).powf(-beta)
    }

    fn gaussian(fwhm: f64) -> impl Fn(f64) -> f64 {
        let sigma = fwhm / FWHM_PER_SIGMA;
        move |d2| (-d2 / (2.0 * sigma * sigma)).exp()
    }

    #[test]
    fn fwhm_of_a_moffat_star_does_not_grow_with_its_brightness() {
        let true_fwhm = 4.0;
        let cut = 50.0;
        let measured: Vec<f64> = [2.0, 10.0, 100.0, 1000.0]
            .iter()
            .map(|peak_over_cut| {
                let img = pixel_integrated_star(81, (40.3, 40.6), 100.0, cut * peak_over_cut, moffat(true_fwhm, 2.5));
                let component = component_above(&img, 100.0 + cut as f32);
                measure_component(&img, &component, 100.0, 10.0, cut).expect("star measured").star.fwhm
            })
            .collect();
        assert!(
            measured.iter().all(|fwhm| (fwhm - true_fwhm).abs() / true_fwhm < 0.08),
            "Moffat beta 2.5 star of FWHM {true_fwhm} px measured as {measured:.2?} px at 2, 10, 100 and 1000 times the cut"
        );
    }

    #[test]
    fn detected_moffat_stars_report_one_fwhm_whatever_their_brightness() {
        let true_fwhm = 4.0;
        let mut img = gaussian_noise(240, 480, 10.0, 11);
        img += 1000.0;
        let stars: [(f64, f64, f64); 4] = [(60.3, 120.6, 500.0), (180.7, 120.2, 5000.0), (300.4, 120.5, 50000.0), (420.6, 120.3, 5000.0)];
        for &(x0, y0, amp) in &stars {
            let stamp = pixel_integrated_star(101, (50.0 + x0 - x0.round(), 50.0 + y0 - y0.round()), 0.0, amp, moffat(true_fwhm, 2.5));
            let (row, col) = (y0.round() as usize - 50, x0.round() as usize - 50);
            let mut target = img.slice_mut(ndarray::s![row..row + 101, col..col + 101]);
            target += &stamp;
        }
        let result = detect_stars(&img, 5.0);
        let measured: Vec<f64> = stars
            .iter()
            .map(|&(x0, y0, _)| result.stars.iter().find(|s| (s.x - x0).hypot(s.y - y0) < 1.0).expect("star detected").fwhm)
            .collect();
        assert!(
            measured.iter().all(|fwhm| (fwhm - true_fwhm).abs() / true_fwhm < 0.10),
            "Moffat beta 2.5 stars of FWHM {true_fwhm} px with peaks of 10, 100, 1000 and 100 times the 5 sigma cut measured as {measured:.2?} px"
        );
    }

    #[test]
    fn fwhm_of_a_bright_undersampled_star_does_not_depend_on_its_pixel_phase() {
        let true_fwhm = 1.5;
        let pixel_integrated_fwhm = (true_fwhm * true_fwhm + FWHM_PER_SIGMA.powi(2) / 12.0).sqrt();
        let cut = 50.0;
        for centre in [(20.0, 20.0), (20.5, 20.0), (20.5, 20.5), (20.3, 20.6)] {
            let img = pixel_integrated_star(41, centre, 100.0, cut * 100.0, gaussian(true_fwhm));
            let component = component_above(&img, 100.0 + cut as f32);
            let star = measure_component(&img, &component, 100.0, 10.0, cut).expect("star measured").star;
            assert!(
                (star.fwhm - pixel_integrated_fwhm).abs() / pixel_integrated_fwhm < 0.10,
                "star centred at {centre:?} gave FWHM {} for a pixel-integrated {pixel_integrated_fwhm}",
                star.fwhm
            );
        }
    }

    #[test]
    fn whole_component_fwhm_is_the_brightness_dependent_width_reported_at_c3f19b3() {
        let cut = 50.0;
        let moffat_star = moffat(4.0, 2.5);
        let gaussian_star = gaussian(4.0);
        let profiles: [(&str, &dyn Fn(f64) -> f64, [f64; 3]); 2] = [
            ("Moffat beta 2.5", &moffat_star, [4.7814796957596, 5.86336812212113, 6.7316042072205935]),
            ("Gaussian", &gaussian_star, [4.113771445172538, 4.0602748575409615, 4.055714506820952]),
        ];
        for (name, profile, at_c3f19b3) in profiles {
            let widths: Vec<f64> = [10.0, 100.0, 1000.0]
                .iter()
                .map(|peak_over_cut| {
                    let img = pixel_integrated_star(81, (40.3, 40.6), 100.0, cut * peak_over_cut, profile);
                    let component = component_above(&img, 100.0 + cut as f32);
                    measure_component(&img, &component, 100.0, 10.0, cut).expect("star measured").whole_component_fwhm
                })
                .collect();
            assert!(
                widths.iter().zip(&at_c3f19b3).all(|(width, expected)| (width - expected).abs() < 1e-9),
                "{name} star of FWHM 4 px at 10, 100 and 1000 times the cut: whole-component FWHM {widths:.4?} px, c3f19b3 reported {at_c3f19b3:.4?} px"
            );
        }
    }

    #[test]
    fn star_extents_pair_each_detected_star_with_the_width_of_its_whole_component() {
        let mut img = make_test_image(300, 300);
        add_gaussian(&mut img, 150, 60, 20000.0, 3.0, 5000.0);
        let extents = detect_star_extents(&img, 5.0);
        let stars = detect_stars(&img, 5.0).stars;
        let bits = |s: &DetectedStar| {
            [s.x.to_bits(), s.y.to_bits(), s.flux.to_bits(), s.fwhm.to_bits(), s.eccentricity.to_bits(), s.peak.to_bits(), s.npix as u64, s.snr.to_bits()]
        };
        assert_eq!(extents.iter().map(|extent| bits(&extent.star)).collect::<Vec<_>>(), stars.iter().map(bits).collect::<Vec<_>>());

        let whole_component: Vec<f64> = extents.iter().map(|extent| extent.whole_component_fwhm).collect();
        let reported: Vec<f64> = stars.iter().map(|star| star.fwhm).collect();
        let near = |measured: &[f64], expected: [f64; 4]| measured.len() == 4 && measured.iter().zip(expected).all(|(m, e)| (m - e).abs() < 5e-4);
        assert!(
            near(&whole_component, [8.279, 4.621, 4.632, 4.651]) && near(&reported, [12.349, 4.738, 4.736, 4.739]),
            "clipped star and three Gaussian stars, brightest first: whole-component FWHM {whole_component:.3?} px, reported FWHM {reported:.3?} px"
        );
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
