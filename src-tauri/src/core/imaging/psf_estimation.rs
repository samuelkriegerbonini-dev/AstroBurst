use ndarray::{Array2, s};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarCandidate {
    pub x: f64,
    pub y: f64,
    pub peak: f64,
    pub flux: f64,
    pub fwhm: f64,
    pub ellipticity: f64,
    pub distance_from_center: f64,
    pub snr: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsfEstimationConfig {
    pub num_stars: usize,
    pub cutout_radius: usize,
    pub saturation_threshold: f64,
    pub min_peak_fraction: f64,
    pub max_ellipticity: f64,
    pub edge_margin: usize,
    pub max_center_distance_fraction: f64,
}

impl Default for PsfEstimationConfig {
    fn default() -> Self {
        Self {
            num_stars: 3,
            cutout_radius: 15,
            saturation_threshold: 0.95,
            min_peak_fraction: 0.10,
            max_ellipticity: 0.3,
            edge_margin: 30,
            max_center_distance_fraction: 0.7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsfResult {
    pub kernel: Vec<Vec<f32>>,
    pub kernel_size: usize,
    pub average_fwhm: f64,
    pub average_ellipticity: f64,
    pub stars_used: Vec<StarCandidate>,
    pub stars_rejected: usize,
    pub spread_pixels: f64,
}

pub fn estimate_psf(
    image: &Array2<f32>,
    config: &PsfEstimationConfig,
) -> Result<PsfResult, String> {
    let (h, w) = image.dim();
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    let max_dist = (cx.powi(2) + cy.powi(2)).sqrt() * config.max_center_distance_fraction;

    let stats = compute_image_stats(image);
    let stars = detect_stars_for_psf(image, &stats, config);

    if stars.is_empty() {
        return Err("No stars detected in image".into());
    }

    let mut candidates: Vec<StarCandidate> = stars
        .into_iter()
        .filter(|s| {
            let norm_peak = s.peak / stats.max_val;
            let in_bounds = s.x >= config.edge_margin as f64
                && s.y >= config.edge_margin as f64
                && s.x < (w - config.edge_margin) as f64
                && s.y < (h - config.edge_margin) as f64;

            let not_saturated = norm_peak < config.saturation_threshold;
            let bright_enough = norm_peak > config.min_peak_fraction;
            let round_enough = s.ellipticity < config.max_ellipticity;
            let close_enough = s.distance_from_center < max_dist;

            in_bounds && not_saturated && bright_enough && round_enough && close_enough
        })
        .collect();

    if candidates.is_empty() {
        return Err("No stars passed quality filters".into());
    }

    candidates.sort_by(|a, b| score_star(b).partial_cmp(&score_star(a)).unwrap());

    let selected: Vec<&StarCandidate> = candidates.iter().take(config.num_stars).collect();

    let cutout_size = config.cutout_radius * 2 + 1;
    let mut psf_sum = Array2::<f64>::zeros((cutout_size, cutout_size));
    let mut count = 0usize;

    for star in &selected {
        if let Some(cutout) = extract_cutout(image, star.x, star.y, config.cutout_radius) {
            let centered = subpixel_center(&cutout);
            let normalized = normalize_cutout(&centered);
            psf_sum += &normalized;
            count += 1;
        }
    }

    if count == 0 {
        return Err("Failed to extract star cutouts".into());
    }

    psf_sum /= count as f64;

    let final_psf = normalize_cutout(&psf_sum);

    let avg_fwhm = selected.iter().map(|s| s.fwhm).sum::<f64>() / selected.len() as f64;
    let avg_ellip = selected.iter().map(|s| s.ellipticity).sum::<f64>() / selected.len() as f64;
    let spread = compute_spread_radius(&final_psf);

    let kernel: Vec<Vec<f32>> = final_psf
        .rows()
        .into_iter()
        .map(|row| row.iter().map(|&v| v as f32).collect())
        .collect();

    Ok(PsfResult {
        kernel,
        kernel_size: cutout_size,
        average_fwhm: avg_fwhm,
        average_ellipticity: avg_ellip,
        stars_used: selected.into_iter().cloned().collect(),
        stars_rejected: candidates.len().saturating_sub(count),
        spread_pixels: spread,
    })
}

pub fn psf_to_kernel(psf: &PsfResult) -> Array2<f32> {
    let size = psf.kernel_size;
    let mut kernel = Array2::<f32>::zeros((size, size));

    for (y, row) in psf.kernel.iter().enumerate() {
        for (x, &val) in row.iter().enumerate() {
            kernel[[y, x]] = val;
        }
    }

    kernel
}

struct LocalStats {
    mean: f64,
    stddev: f64,
    max_val: f64,
    median: f64,
}

fn compute_image_stats(image: &Array2<f32>) -> LocalStats {
    let n = image.len() as f64;
    let sum: f64 = image.iter().map(|&v| v as f64).sum();
    let mean = sum / n;
    let var: f64 = image.iter().map(|&v| ((v as f64) - mean).powi(2)).sum::<f64>() / n;
    let stddev = var.sqrt();
    let max_val = image.iter().cloned().fold(f32::NEG_INFINITY, f32::max) as f64;

    let mut sorted: Vec<f32> = image.iter().cloned().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2] as f64;

    LocalStats { mean, stddev, max_val, median }
}

fn detect_stars_for_psf(
    image: &Array2<f32>,
    stats: &LocalStats,
    config: &PsfEstimationConfig,
) -> Vec<StarCandidate> {
    let (h, w) = image.dim();
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    let threshold = stats.median + 5.0 * stats.stddev;
    let margin = config.edge_margin;
    let search_radius = 5usize;

    let mut stars = Vec::new();
    let mut visited = Array2::<bool>::default((h, w));

    for y in margin..(h - margin) {
        for x in margin..(w - margin) {
            let val = image[[y, x]] as f64;
            if val < threshold || visited[[y, x]] {
                continue;
            }

            let mut is_local_max = true;
            for dy in -(search_radius as i64)..=(search_radius as i64) {
                for dx in -(search_radius as i64)..=(search_radius as i64) {
                    if dy == 0 && dx == 0 {
                        continue;
                    }
                    let ny = (y as i64 + dy) as usize;
                    let nx = (x as i64 + dx) as usize;
                    if ny < h && nx < w && (image[[ny, nx]] as f64) > val {
                        is_local_max = false;
                        break;
                    }
                }
                if !is_local_max {
                    break;
                }
            }

            if !is_local_max {
                continue;
            }

            for dy in -(search_radius as i64)..=(search_radius as i64) {
                for dx in -(search_radius as i64)..=(search_radius as i64) {
                    let ny = (y as i64 + dy) as usize;
                    let nx = (x as i64 + dx) as usize;
                    if ny < h && nx < w {
                        visited[[ny, nx]] = true;
                    }
                }
            }

            let (sub_x, sub_y) = centroid_subpixel(image, x, y, 3);
            let (fwhm_x, fwhm_y) = measure_fwhm(image, sub_x, sub_y);
            let fwhm = (fwhm_x + fwhm_y) / 2.0;
            let ellipticity = if fwhm_x > fwhm_y {
                1.0 - fwhm_y / fwhm_x
            } else {
                1.0 - fwhm_x / fwhm_y
            };

            let flux = aperture_flux(image, sub_x, sub_y, fwhm * 1.5);
            let bg_flux = annulus_background(image, sub_x, sub_y, fwhm * 2.0, fwhm * 3.0);
            let snr = if bg_flux > 0.0 { flux / bg_flux.sqrt() } else { flux };

            let dist = ((sub_x - cx).powi(2) + (sub_y - cy).powi(2)).sqrt();

            if fwhm > 1.5 && fwhm < 20.0 && snr > 10.0 {
                stars.push(StarCandidate {
                    x: sub_x,
                    y: sub_y,
                    peak: val,
                    flux,
                    fwhm,
                    ellipticity,
                    distance_from_center: dist,
                    snr,
                });
            }
        }
    }

    stars
}

fn centroid_subpixel(image: &Array2<f32>, x: usize, y: usize, radius: usize) -> (f64, f64) {
    let (h, w) = image.dim();
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_w = 0.0f64;

    for dy in -(radius as i64)..=(radius as i64) {
        for dx in -(radius as i64)..=(radius as i64) {
            let ny = y as i64 + dy;
            let nx = x as i64 + dx;
            if ny >= 0 && ny < h as i64 && nx >= 0 && nx < w as i64 {
                let val = image[[ny as usize, nx as usize]] as f64;
                sum_x += nx as f64 * val;
                sum_y += ny as f64 * val;
                sum_w += val;
            }
        }
    }

    if sum_w > 0.0 {
        (sum_x / sum_w, sum_y / sum_w)
    } else {
        (x as f64, y as f64)
    }
}

fn measure_fwhm(image: &Array2<f32>, x: f64, y: f64) -> (f64, f64) {
    let (h, w) = image.dim();
    let ix = x.round() as usize;
    let iy = y.round() as usize;

    if ix >= w || iy >= h {
        return (4.0, 4.0);
    }

    let peak = image[[iy, ix]] as f64;
    let half = peak / 2.0;

    let fwhm_x = measure_fwhm_1d(image, ix, iy, true, half);
    let fwhm_y = measure_fwhm_1d(image, ix, iy, false, half);

    (fwhm_x, fwhm_y)
}

fn measure_fwhm_1d(
    image: &Array2<f32>,
    cx: usize,
    cy: usize,
    horizontal: bool,
    half_max: f64,
) -> f64 {
    let (h, w) = image.dim();
    let limit = if horizontal { w } else { h };
    let center = if horizontal { cx } else { cy };

    let get_val = |i: usize| -> f64 {
        if horizontal {
            if i < w { image[[cy, i]] as f64 } else { 0.0 }
        } else {
            if i < h { image[[i, cx]] as f64 } else { 0.0 }
        }
    };

    let mut left = center as f64;
    for i in (0..center).rev() {
        if get_val(i) < half_max {
            let v1 = get_val(i);
            let v2 = get_val(i + 1);
            if (v2 - v1).abs() > 1e-10 {
                left = i as f64 + (half_max - v1) / (v2 - v1);
            } else {
                left = i as f64;
            }
            break;
        }
    }

    let mut right = center as f64;
    for i in (center + 1)..limit {
        if get_val(i) < half_max {
            let v1 = get_val(i - 1);
            let v2 = get_val(i);
            if (v1 - v2).abs() > 1e-10 {
                right = (i - 1) as f64 + (v1 - half_max) / (v1 - v2);
            } else {
                right = i as f64;
            }
            break;
        }
    }

    right - left
}

fn aperture_flux(image: &Array2<f32>, x: f64, y: f64, radius: f64) -> f64 {
    let (h, w) = image.dim();
    let r2 = radius * radius;
    let mut flux = 0.0;

    let y_min = (y - radius).floor().max(0.0) as usize;
    let y_max = ((y + radius).ceil() as usize).min(h - 1);
    let x_min = (x - radius).floor().max(0.0) as usize;
    let x_max = ((x + radius).ceil() as usize).min(w - 1);

    for py in y_min..=y_max {
        for px in x_min..=x_max {
            let dx = px as f64 - x;
            let dy = py as f64 - y;
            if dx * dx + dy * dy <= r2 {
                flux += image[[py, px]] as f64;
            }
        }
    }

    flux
}

fn annulus_background(
    image: &Array2<f32>,
    x: f64,
    y: f64,
    inner_r: f64,
    outer_r: f64,
) -> f64 {
    let (h, w) = image.dim();
    let ir2 = inner_r * inner_r;
    let or2 = outer_r * outer_r;
    let mut vals = Vec::new();

    let y_min = (y - outer_r).floor().max(0.0) as usize;
    let y_max = ((y + outer_r).ceil() as usize).min(h - 1);
    let x_min = (x - outer_r).floor().max(0.0) as usize;
    let x_max = ((x + outer_r).ceil() as usize).min(w - 1);

    for py in y_min..=y_max {
        for px in x_min..=x_max {
            let dx = px as f64 - x;
            let dy = py as f64 - y;
            let d2 = dx * dx + dy * dy;
            if d2 >= ir2 && d2 <= or2 {
                vals.push(image[[py, px]] as f64);
            }
        }
    }

    if vals.is_empty() {
        return 0.0;
    }

    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let lo = vals.len() / 4;
    let hi = 3 * vals.len() / 4;
    let clipped = &vals[lo..hi];
    if clipped.is_empty() {
        return 0.0;
    }
    clipped.iter().sum::<f64>() / clipped.len() as f64
}

fn score_star(star: &StarCandidate) -> f64 {
    let roundness_score = 1.0 - star.ellipticity;
    let snr_score = (star.snr / 100.0).min(1.0);
    let center_score = 1.0 / (1.0 + star.distance_from_center / 500.0);
    let fwhm_consistency = 1.0 / (1.0 + (star.fwhm - 4.0).abs() / 4.0);

    roundness_score * 0.35 + snr_score * 0.30 + center_score * 0.15 + fwhm_consistency * 0.20
}

fn extract_cutout(
    image: &Array2<f32>,
    x: f64,
    y: f64,
    radius: usize,
) -> Option<Array2<f64>> {
    let (h, w) = image.dim();
    let size = radius * 2 + 1;
    let ix = x.round() as i64;
    let iy = y.round() as i64;

    let x_start = ix - radius as i64;
    let y_start = iy - radius as i64;

    if x_start < 0 || y_start < 0 {
        return None;
    }
    if (x_start + size as i64) > w as i64 || (y_start + size as i64) > h as i64 {
        return None;
    }

    let xs = x_start as usize;
    let ys = y_start as usize;

    Some(
        image
            .slice(s![ys..ys + size, xs..xs + size])
            .mapv(|v| v as f64),
    )
}

fn subpixel_center(cutout: &Array2<f64>) -> Array2<f64> {
    let (h, w) = cutout.dim();
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_w = 0.0f64;

    for y in 0..h {
        for x in 0..w {
            let val = cutout[[y, x]];
            sum_x += x as f64 * val;
            sum_y += y as f64 * val;
            sum_w += val;
        }
    }

    if sum_w <= 0.0 {
        return cutout.clone();
    }

    let cx = sum_x / sum_w;
    let cy = sum_y / sum_w;
    let target_cx = (w as f64 - 1.0) / 2.0;
    let target_cy = (h as f64 - 1.0) / 2.0;
    let shift_x = target_cx - cx;
    let shift_y = target_cy - cy;

    bilinear_shift(cutout, shift_x, shift_y)
}

fn bilinear_shift(image: &Array2<f64>, dx: f64, dy: f64) -> Array2<f64> {
    let (h, w) = image.dim();
    let mut result = Array2::<f64>::zeros((h, w));

    for y in 0..h {
        for x in 0..w {
            let sx = x as f64 - dx;
            let sy = y as f64 - dy;

            let x0 = sx.floor() as i64;
            let y0 = sy.floor() as i64;
            let fx = sx - x0 as f64;
            let fy = sy - y0 as f64;

            let sample = |yy: i64, xx: i64| -> f64 {
                if yy >= 0 && yy < h as i64 && xx >= 0 && xx < w as i64 {
                    image[[yy as usize, xx as usize]]
                } else {
                    0.0
                }
            };

            let v = sample(y0, x0) * (1.0 - fx) * (1.0 - fy)
                + sample(y0, x0 + 1) * fx * (1.0 - fy)
                + sample(y0 + 1, x0) * (1.0 - fx) * fy
                + sample(y0 + 1, x0 + 1) * fx * fy;

            result[[y, x]] = v;
        }
    }

    result
}

fn normalize_cutout(cutout: &Array2<f64>) -> Array2<f64> {
    let sum: f64 = cutout.iter().sum();
    if sum > 0.0 {
        cutout / sum
    } else {
        cutout.clone()
    }
}

fn compute_spread_radius(psf: &Array2<f64>) -> f64 {
    let (h, w) = psf.dim();
    let cx = (w as f64 - 1.0) / 2.0;
    let cy = (h as f64 - 1.0) / 2.0;

    let mut sum_r2_w = 0.0f64;
    let mut sum_w = 0.0f64;

    for y in 0..h {
        for x in 0..w {
            let val = psf[[y, x]];
            let r2 = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
            sum_r2_w += r2 * val;
            sum_w += val;
        }
    }

    if sum_w > 0.0 {
        (sum_r2_w / sum_w).sqrt()
    } else {
        0.0
    }
}
