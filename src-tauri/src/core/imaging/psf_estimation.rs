use std::f64::consts::{LN_2, PI};

use ndarray::{Array2, s};
use serde::{Deserialize, Serialize};

use crate::core::imaging::stats::is_valid_pixel;
use crate::math::median::{exact_mad_mut, exact_median_mut};
use crate::types::constants::MAD_TO_SIGMA;

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
            num_stars: 30,
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
    let detections = detect_stars_for_psf(image, &stats, config);
    let detected_total = detections.len();

    if detections.is_empty() {
        return Err("No stars detected in image".into());
    }

    let saturation_level = plateau_level(&detections);

    let usable: Vec<StarCandidate> = detections
        .into_iter()
        .filter(|d| {
            let s = &d.star;
            let in_bounds = s.x >= config.edge_margin as f64
                && s.y >= config.edge_margin as f64
                && s.x < (w - config.edge_margin) as f64
                && s.y < (h - config.edge_margin) as f64;

            let round_enough = s.ellipticity < config.max_ellipticity;
            let close_enough = s.distance_from_center < max_dist;
            let saturated = is_saturated(d, saturation_level, stats.median, config.saturation_threshold);

            in_bounds && !saturated && round_enough && close_enough
        })
        .map(|d| d.star)
        .collect();

    let reference = peak_reference(&usable, stats.median);
    let mut candidates: Vec<StarCandidate> = usable
        .into_iter()
        .filter(|s| s.peak - stats.median > config.min_peak_fraction * reference)
        .collect();

    if candidates.is_empty() {
        return Err("No stars passed quality filters".into());
    }

    candidates.sort_by(|a, b| score_star(b).total_cmp(&score_star(a)));

    let selected: Vec<&StarCandidate> = candidates.iter().take(config.num_stars).collect();

    let cutout_size = config.cutout_radius * 2 + 1;
    let mut psf_sum = Array2::<f64>::zeros((cutout_size, cutout_size));
    let mut count = 0usize;

    for star in &selected {
        if let Some(cutout) = extract_cutout(image, star.x, star.y, config.cutout_radius) {
            let bg = cutout_border_median(&cutout);
            let cleaned = cutout.mapv(|v| (v - bg).max(0.0));
            let centered = subpixel_center(&cleaned);
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
        stars_rejected: detected_total.saturating_sub(count),
        spread_pixels: spread,
    })
}

pub fn psf_to_kernel(psf: &PsfResult) -> Array2<f32> {
    let size = psf.kernel_size;
    let mut kernel = Array2::<f32>::zeros((size, size));

    for (y, row) in psf.kernel.iter().enumerate() {
        for (x, &val) in row.iter().enumerate() {
            if y < size && x < size {
                kernel[[y, x]] = val;
            }
        }
    }

    kernel
}

const FWHM_PER_SIGMA: f64 = 2.354_820_045_030_949_3;
const PLATEAU_MIN_PIXELS: usize = 5;
const PLATEAU_WINDOW: usize = 4;
const HALF_MAX_WINDOW: usize = 32;
const PLATEAU_NOISE_SIGMAS: f64 = 3.0;

struct LocalStats {
    _mean: f64,
    stddev: f64,
    median: f64,
    noise: f64,
}

fn compute_image_stats(image: &Array2<f32>) -> LocalStats {
    let slice = image.as_slice().unwrap_or(&[]);

    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut vals: Vec<f32> = Vec::with_capacity(slice.len());

    for &v in slice {
        if !v.is_finite() {
            continue;
        }
        let vf = v as f64;
        sum += vf;
        sum_sq += vf * vf;
        vals.push(v);
    }

    if vals.is_empty() {
        return LocalStats { _mean: 0.0, stddev: 0.0, median: 0.0, noise: 0.0 };
    }

    let n = vals.len() as f64;
    let mean = sum / n;
    let var = (sum_sq / n) - mean * mean;
    let stddev = if var > 0.0 { var.sqrt() } else { 0.0 };

    let mid = vals.len() / 2;
    vals.select_nth_unstable_by(mid, f32::total_cmp);
    let median = vals[mid] as f64;

    vals.retain(|&v| is_valid_pixel(v));
    let center = exact_median_mut(&mut vals) as f32;
    let noise = exact_mad_mut(&mut vals, center) as f64 * MAD_TO_SIGMA;

    LocalStats { _mean: mean, stddev, median, noise }
}

fn core_is_finite(image: &Array2<f32>, y: usize, x: usize) -> bool {
    let (h, w) = image.dim();
    (y.saturating_sub(1)..=(y + 1).min(h - 1))
        .all(|ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).all(|nx| image[[ny, nx]].is_finite()))
}

struct Detection {
    star: StarCandidate,
    raw_peak: f64,
    plateau: bool,
}

fn connected_count(
    image: &Array2<f32>,
    y: usize,
    x: usize,
    radius: usize,
    limit: usize,
    accept: impl Fn(f64) -> bool,
) -> usize {
    let (h, w) = image.dim();
    let side = 2 * radius + 1;
    let mut seen = vec![false; side * side];
    let slot = |py: usize, px: usize| -> Option<usize> {
        let ly = (py + radius).checked_sub(y)?;
        let lx = (px + radius).checked_sub(x)?;
        (ly < side && lx < side).then_some(ly * side + lx)
    };
    let mut stack = vec![(y, x)];
    seen[radius * side + radius] = true;
    let mut count = 0usize;
    while let Some((py, px)) = stack.pop() {
        count += 1;
        if count >= limit {
            break;
        }
        for ny in py.saturating_sub(1)..=(py + 1).min(h - 1) {
            for nx in px.saturating_sub(1)..=(px + 1).min(w - 1) {
                let Some(k) = slot(ny, nx) else {
                    continue;
                };
                if seen[k] {
                    continue;
                }
                let v = image[[ny, nx]] as f64;
                if v.is_finite() && accept(v) {
                    seen[k] = true;
                    stack.push((ny, nx));
                }
            }
        }
    }
    count
}

fn has_plateau(image: &Array2<f32>, y: usize, x: usize, fwhm: (f64, f64), sky: f64, noise: f64) -> bool {
    let peak = image[[y, x]] as f64;
    let half_area = |sky: f64| connected_count(image, y, x, HALF_MAX_WINDOW, usize::MAX, |v| v >= 0.5 * (peak + sky)) as f64;
    let half_radius = (half_area(sky) / PI).sqrt();
    let (sky, _) = annulus_stats(image, x as f64, y as f64, 4.0 * half_radius, 6.0 * half_radius);
    let net = peak - sky;
    if !net.is_finite() || net <= 0.0 {
        return false;
    }
    let (fwhm_major, fwhm_minor) = fwhm;
    let elongation = (fwhm_major / fwhm_minor).max(1.0);
    let moment_sigma = fwhm_major / FWHM_PER_SIGMA;
    let sigma_sq = (half_area(sky) * elongation / (2.0 * PI * LN_2)).max(moment_sigma * moment_sigma);
    let tol =0.5 * net * (1.0 - (-0.5 / sigma_sq).exp());
    if !tol.is_finite() || tol <= 0.0 {
        return false;
    }
    let tolerance = if tol >= PLATEAU_NOISE_SIGMAS * noise {
        tol
    } else if tol >= noise {
        0.0
    } else {
        return false;
    };
    connected_count(image, y, x, PLATEAU_WINDOW, PLATEAU_MIN_PIXELS, |v| (v - peak).abs() <= tolerance) >= PLATEAU_MIN_PIXELS
}

fn plateau_level(detections: &[Detection]) -> Option<f64> {
    detections
        .iter()
        .filter(|d| d.plateau)
        .map(|d| d.raw_peak)
        .max_by(f64::total_cmp)
}

fn is_saturated(d: &Detection, level: Option<f64>, median: f64, threshold: f64) -> bool {
    d.plateau || level.is_some_and(|l| l > median && d.raw_peak - median >= threshold * (l - median))
}

fn peak_reference(stars: &[StarCandidate], median: f64) -> f64 {
    let mut peaks: Vec<f64> = stars.iter().map(|s| s.peak - median).filter(|v| v.is_finite()).collect();
    peaks.sort_by(|a, b| b.total_cmp(a));
    peaks.get(peaks.len() / 10).copied().unwrap_or(0.0)
}

fn detect_stars_for_psf(
    image: &Array2<f32>,
    stats: &LocalStats,
    config: &PsfEstimationConfig,
) -> Vec<Detection> {
    let (h, w) = image.dim();
    let margin = config.edge_margin;
    if h <= margin * 2 || w <= margin * 2 {
        return Vec::new();
    }
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    let threshold = stats.median + 5.0 * stats.stddev;
    let search_radius = 5i64;

    let mut stars = Vec::new();
    let mut visited = Array2::<bool>::default((h, w));

    for y in margin..(h - margin) {
        for x in margin..(w - margin) {
            let val = image[[y, x]] as f64;
            if !val.is_finite() || val < threshold || visited[[y, x]] {
                continue;
            }

            let mut is_local_max = true;
            for dy in -search_radius..=search_radius {
                for dx in -search_radius..=search_radius {
                    if dy == 0 && dx == 0 {
                        continue;
                    }
                    let ny = y as i64 + dy;
                    let nx = x as i64 + dx;
                    if ny < 0 || nx < 0 || ny >= h as i64 || nx >= w as i64 {
                        continue;
                    }
                    if (image[[ny as usize, nx as usize]] as f64) > val {
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

            for dy in -search_radius..=search_radius {
                for dx in -search_radius..=search_radius {
                    let ny = y as i64 + dy;
                    let nx = x as i64 + dx;
                    if ny >= 0 && nx >= 0 && ny < h as i64 && nx < w as i64 {
                        visited[[ny as usize, nx as usize]] = true;
                    }
                }
            }

            if !core_is_finite(image, y, x) {
                continue;
            }

            let (sub_x, sub_y) = centroid_subpixel(image, x, y, 3);
            let sub_peak = subpixel_peak(image, x, y);
            let Some((fwhm_major, fwhm_minor)) = measure_fwhm(image, sub_x, sub_y) else {
                continue;
            };
            let fwhm = (fwhm_major + fwhm_minor) / 2.0;
            let ellipticity = if fwhm_major.max(fwhm_minor) > 1e-10 {
                1.0 - fwhm_minor.min(fwhm_major) / fwhm_major.max(fwhm_minor)
            } else {
                0.0
            };

            let (flux_sum, n_aperture) = aperture_flux(image, sub_x, sub_y, fwhm * 1.5);
            let (bg_mean, bg_sigma) = annulus_stats(image, sub_x, sub_y, fwhm * 2.0, fwhm * 3.0);
            let flux = flux_sum - bg_mean * n_aperture as f64;
            let noise = bg_sigma * (n_aperture as f64).sqrt();
            let snr = if noise > 1e-12 { flux / noise } else { flux.max(0.0) };

            let dist = ((sub_x - cx).powi(2) + (sub_y - cy).powi(2)).sqrt();

            if fwhm > 1.5 && fwhm < 20.0 && snr > 10.0 {
                stars.push(Detection {
                    star: StarCandidate {
                        x: sub_x,
                        y: sub_y,
                        peak: sub_peak,
                        flux,
                        fwhm,
                        ellipticity,
                        distance_from_center: dist,
                        snr,
                    },
                    raw_peak: val,
                    plateau: has_plateau(image, y, x, (fwhm_major, fwhm_minor), bg_mean, stats.noise),
                });
            }
        }
    }

    stars
}

fn centroid_subpixel(image: &Array2<f32>, x: usize, y: usize, radius: usize) -> (f64, f64) {
    let (h, w) = image.dim();
    let bg = estimate_local_bg(image, x, y, 10);
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_w = 0.0f64;

    let r = radius as i64;
    for dy in -r..=r {
        for dx in -r..=r {
            let ny = y as i64 + dy;
            let nx = x as i64 + dx;
            if ny >= 0 && ny < h as i64 && nx >= 0 && nx < w as i64 {
                let val = (image[[ny as usize, nx as usize]] as f64 - bg).max(0.0);
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

fn measure_fwhm(image: &Array2<f32>, x: f64, y: f64) -> Option<(f64, f64)> {
    let (h, w) = image.dim();
    let ix = x.round() as usize;
    let iy = y.round() as usize;

    if ix >= w || iy >= h {
        return Some((4.0, 4.0));
    }

    let peak = subpixel_peak(image, ix, iy);
    if !peak.is_finite() {
        return None;
    }
    let bg = estimate_local_bg(image, ix, iy, 10);
    let net_peak = peak - bg;
    if net_peak <= 0.0 {
        return Some((4.0, 4.0));
    }

    let threshold = bg + net_peak * 0.5;
    let radius = 12i64;

    let mut m_xx = 0.0f64;
    let mut m_yy = 0.0f64;
    let mut m_xy = 0.0f64;
    let mut sum_w = 0.0f64;

    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let py = iy as i64 + dy;
            let px = ix as i64 + dx;
            if py < 0 || py >= h as i64 || px < 0 || px >= w as i64 {
                continue;
            }
            let val = image[[py as usize, px as usize]] as f64;
            if !val.is_finite() || val < threshold {
                continue;
            }
            let weight = val - bg;
            let fx = px as f64 - x;
            let fy = py as f64 - y;
            m_xx += fx * fx * weight;
            m_yy += fy * fy * weight;
            m_xy += fx * fy * weight;
            sum_w += weight;
        }
    }

    if sum_w <= 0.0 {
        return Some((4.0, 4.0));
    }

    let half_max_truncation = 1.0 - std::f64::consts::LN_2;
    let sigma_xx = m_xx / sum_w / half_max_truncation;
    let sigma_yy = m_yy / sum_w / half_max_truncation;
    let sigma_xy = m_xy / sum_w / half_max_truncation;

    let trace = sigma_xx + sigma_yy;
    let det = sigma_xx * sigma_yy - sigma_xy * sigma_xy;
    let disc = (trace * trace - 4.0 * det).max(0.0).sqrt();
    let lambda1 = ((trace + disc) / 2.0).max(0.0);
    let lambda2 = ((trace - disc) / 2.0).max(0.0);

    let fwhm_major = FWHM_PER_SIGMA * lambda1.sqrt();
    let fwhm_minor = FWHM_PER_SIGMA * lambda2.sqrt();

    let fwhm_major = fwhm_major.clamp(1.0, 30.0);
    let fwhm_minor = fwhm_minor.clamp(1.0, 30.0);

    Some((fwhm_major, fwhm_minor))
}

fn subpixel_peak(image: &Array2<f32>, ix: usize, iy: usize) -> f64 {
    let (h, w) = image.dim();
    if ix < 1 || iy < 1 || ix + 1 >= w || iy + 1 >= h {
        return image[[iy, ix]] as f64;
    }

    let v = |dy: i64, dx: i64| -> f64 {
        image[[(iy as i64 + dy) as usize, (ix as i64 + dx) as usize]] as f64
    };

    let c = v(0, 0);
    if !core_is_finite(image, iy, ix) {
        return c;
    }
    let dx_val = (v(0, 1) - v(0, -1)) * 0.5;
    let dy_val = (v(1, 0) - v(-1, 0)) * 0.5;
    let dxx = v(0, 1) + v(0, -1) - 2.0 * c;
    let dyy = v(1, 0) + v(-1, 0) - 2.0 * c;
    let dxy = (v(1, 1) + v(-1, -1) - v(1, -1) - v(-1, 1)) * 0.25;

    let det = dxx * dyy - dxy * dxy;
    if det.abs() < 1e-12 || det < 0.0 {
        return c;
    }

    let sx = -(dyy * dx_val - dxy * dy_val) / det;
    let sy = -(dxx * dy_val - dxy * dx_val) / det;

    if sx.abs() > 1.0 || sy.abs() > 1.0 {
        return c;
    }

    c + 0.5 * (dx_val * sx + dy_val * sy)
}

fn estimate_local_bg(image: &Array2<f32>, ix: usize, iy: usize, radius: usize) -> f64 {
    let (h, w) = image.dim();
    let inner_r2 = (radius as f64 * 0.6) * (radius as f64 * 0.6);
    let outer_r2 = (radius as f64) * (radius as f64);
    let mut vals = Vec::new();

    let r = radius as i64;
    for dy in -r..=r {
        for dx in -r..=r {
            let py = iy as i64 + dy;
            let px = ix as i64 + dx;
            if py < 0 || py >= h as i64 || px < 0 || px >= w as i64 {
                continue;
            }
            let d2 = (dx * dx + dy * dy) as f64;
            let v = image[[py as usize, px as usize]] as f64;
            if d2 >= inner_r2 && d2 <= outer_r2 && v.is_finite() {
                vals.push(v);
            }
        }
    }

    if vals.is_empty() {
        return 0.0;
    }

    vals.sort_by(f64::total_cmp);
    let lo = vals.len() / 4;
    let hi = (3 * vals.len() / 4).max(lo + 1).min(vals.len());
    let clipped = &vals[lo..hi];
    if clipped.is_empty() {
        return 0.0;
    }
    clipped.iter().sum::<f64>() / clipped.len() as f64
}

fn aperture_flux(image: &Array2<f32>, x: f64, y: f64, radius: f64) -> (f64, u32) {
    let (h, w) = image.dim();
    let r2 = radius * radius;
    let mut flux = 0.0;
    let mut count = 0u32;

    let y_min = (y - radius).floor().max(0.0) as usize;
    let y_max = ((y + radius).ceil() as usize).min(h.saturating_sub(1));
    let x_min = (x - radius).floor().max(0.0) as usize;
    let x_max = ((x + radius).ceil() as usize).min(w.saturating_sub(1));

    for py in y_min..=y_max {
        for px in x_min..=x_max {
            let dx = px as f64 - x;
            let dy = py as f64 - y;
            if dx * dx + dy * dy <= r2 {
                let v = image[[py, px]] as f64;
                if v.is_finite() {
                    flux += v;
                    count += 1;
                }
            }
        }
    }

    (flux, count)
}

fn annulus_stats(
    image: &Array2<f32>,
    x: f64,
    y: f64,
    inner_r: f64,
    outer_r: f64,
) -> (f64, f64) {
    let (h, w) = image.dim();
    let ir2 = inner_r * inner_r;
    let or2 = outer_r * outer_r;
    let mut vals = Vec::new();

    let y_min = (y - outer_r).floor().max(0.0) as usize;
    let y_max = ((y + outer_r).ceil() as usize).min(h.saturating_sub(1));
    let x_min = (x - outer_r).floor().max(0.0) as usize;
    let x_max = ((x + outer_r).ceil() as usize).min(w.saturating_sub(1));

    for py in y_min..=y_max {
        for px in x_min..=x_max {
            let dx = px as f64 - x;
            let dy = py as f64 - y;
            let d2 = dx * dx + dy * dy;
            if d2 >= ir2 && d2 <= or2 {
                let v = image[[py, px]] as f64;
                if v.is_finite() {
                    vals.push(v);
                }
            }
        }
    }

    if vals.is_empty() {
        return (0.0, 0.0);
    }

    vals.sort_by(f64::total_cmp);
    let median = vals[vals.len() / 2];

    let lo = vals.len() / 4;
    let hi = (3 * vals.len() / 4).max(lo + 1).min(vals.len());
    let clipped = &vals[lo..hi];
    let mean = if clipped.is_empty() {
        median
    } else {
        clipped.iter().sum::<f64>() / clipped.len() as f64
    };

    let mut devs: Vec<f64> = vals.iter().map(|v| (v - median).abs()).collect();
    devs.sort_by(f64::total_cmp);
    let sigma = devs[devs.len() / 2] * 1.4826;

    (mean, sigma)
}

fn cutout_border_median(cutout: &Array2<f64>) -> f64 {
    let (h, w) = cutout.dim();
    if h < 2 || w < 2 {
        return 0.0;
    }
    let mut vals = Vec::with_capacity(2 * (h + w));
    for x in 0..w {
        vals.push(cutout[[0, x]]);
        vals.push(cutout[[h - 1, x]]);
    }
    for y in 1..h - 1 {
        vals.push(cutout[[y, 0]]);
        vals.push(cutout[[y, w - 1]]);
    }
    vals.retain(|v| v.is_finite());
    if vals.is_empty() {
        return 0.0;
    }
    let mid = vals.len() / 2;
    vals.select_nth_unstable_by(mid, f64::total_cmp);
    vals[mid]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn add_star(image: &mut Array2<f32>, cx: usize, cy: usize, peak: f32, sigma: f32) {
        add_star_at(image, cx as f64, cy as f64, peak, sigma);
    }

    fn add_star_at(image: &mut Array2<f32>, cx: f64, cy: f64, peak: f32, sigma: f32) {
        let (h, w) = image.dim();
        let r = (sigma as f64 * 6.0).ceil() as i64;
        let (x0, y0) = (cx.round() as i64, cy.round() as i64);
        for y in (y0 - r)..=(y0 + r) {
            for x in (x0 - r)..=(x0 + r) {
                if y < 0 || x < 0 || y >= h as i64 || x >= w as i64 {
                    continue;
                }
                let d2 = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)) as f32;
                image[[y as usize, x as usize]] += peak * (-d2 / (2.0 * sigma * sigma)).exp();
            }
        }
    }

    const BRIGHT_STARS: [(usize, usize); 3] = [(60, 60), (196, 60), (60, 196)];

    fn star_field_with_nan_rings() -> (Array2<f32>, Vec<(usize, usize)>) {
        let (h, w) = (256usize, 256usize);
        let mut state: u32 = 12345;
        let mut image = Array2::<f32>::from_shape_fn((h, w), |_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            100.0 + ((state >> 16) & 0x7fff) as f32 / 32768.0 - 0.5
        });
        for (cx, cy) in BRIGHT_STARS {
            add_star(&mut image, cx, cy, 12000.0, 4.0);
        }
        let mut ringed = Vec::new();
        for (j, sy) in [70usize, 100, 130, 160, 190].into_iter().enumerate() {
            for (i, sx) in [70usize, 100, 130, 160, 190].into_iter().enumerate() {
                let peak = 4000.0 + 180.0 * (i * 5 + j) as f32;
                add_star(&mut image, sx, sy, peak, 1.8);
                if (i + j) % 2 == 0 {
                    ringed.push((sx, sy));
                }
            }
        }
        let ring_offsets: [(i64, i64); 6] = [(8, 0), (0, -7), (-6, 6), (7, 5), (-9, -2), (3, 9)];
        for &(sx, sy) in &ringed {
            for (dx, dy) in ring_offsets {
                image[[(sy as i64 + dy) as usize, (sx as i64 + dx) as usize]] = f32::NAN;
            }
        }
        (image, ringed)
    }

    #[test]
    fn estimate_psf_survives_nan_pixels_in_background_ring() {
        let (image, ringed) = star_field_with_nan_rings();
        let result = estimate_psf(&image, &PsfEstimationConfig::default());
        let psf = result.expect("estimate_psf must succeed on a star field with NaN background pixels");
        assert_eq!(psf.kernel_size, 31);
        assert!(!psf.stars_used.is_empty());
        let unblended = |&&(x, y): &&(usize, usize)| {
            BRIGHT_STARS.iter().all(|&(bx, by)| (x as f64 - bx as f64).hypot(y as f64 - by as f64) > 20.0)
        };
        let isolated: Vec<&(usize, usize)> = ringed.iter().filter(unblended).collect();
        assert_eq!(isolated.len(), ringed.len() - 3);
        for &&(x, y) in &isolated {
            assert!(has_star_near(&psf, x as f64, y as f64, 0.5), "NaN-ringed star at ({x},{y}) dropped: {:?}", psf.stars_used);
        }
        let total: f64 = psf.kernel.iter().flatten().map(|&v| v as f64).sum();
        assert!(psf.kernel.iter().flatten().all(|v| v.is_finite()));
        assert!((total - 1.0).abs() < 1e-3, "kernel sum {}", total);
        assert!(psf.average_fwhm > 3.0 && psf.average_fwhm < 6.0, "fwhm {}", psf.average_fwhm);
    }

    fn noisy_background(h: usize, w: usize, level: f32, half_range: f32, seed: u32) -> Array2<f32> {
        let mut state = seed;
        Array2::<f32>::from_shape_fn((h, w), |_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let u = ((state >> 16) & 0x7fff) as f32 / 32768.0;
            level + (2.0 * u - 1.0) * half_range
        })
    }

    fn has_star_near(psf: &PsfResult, x: f64, y: f64, tol: f64) -> bool {
        psf.stars_used.iter().any(|s| (s.x - x).abs() < tol && (s.y - y).abs() < tol)
    }

    #[test]
    fn nan_inside_the_fwhm_window_does_not_drop_the_star() {
        let mut image = noisy_background(256, 256, 100.0, 0.5, 777);
        for (bx, by) in [(40usize, 40usize), (212, 40), (40, 212), (212, 212)] {
            for y in by..by + 4 {
                for x in bx..bx + 4 {
                    image[[y, x]] = 10000.0;
                }
            }
        }
        let stars = [(80usize, 128usize), (128, 80), (128, 128), (176, 128), (128, 176), (176, 176)];
        for (x, y) in stars {
            add_star(&mut image, x, y, 3000.0, 1.5);
        }
        image[[128, 139]] = f32::NAN;
        image[[128, 88]] = f32::NAN;

        let psf = estimate_psf(&image, &PsfEstimationConfig::default()).expect("psf");
        assert_eq!(psf.stars_used.len(), stars.len(), "{:?}", psf.stars_used);
        for (x, y) in stars {
            assert!(has_star_near(&psf, x as f64, y as f64, 0.5), "star at ({x},{y}) dropped");
        }
    }

    #[test]
    fn star_with_a_nan_core_is_not_used() {
        let mut image = noisy_background(256, 256, 100.0, 0.5, 31);
        let stars = [(96usize, 128usize), (160, 128), (128, 96), (128, 160)];
        for (x, y) in stars {
            add_star(&mut image, x, y, 3000.0, 1.5);
        }
        add_star(&mut image, 170, 170, 3000.0, 1.5);
        for y in 170..173 {
            for x in 170..173 {
                image[[y, x]] = f32::NAN;
            }
        }
        let psf = estimate_psf(&image, &PsfEstimationConfig::default()).expect("psf");
        assert!(!has_star_near(&psf, 170.0, 170.0, 4.0), "{:?}", psf.stars_used);
        for (x, y) in stars {
            assert!(has_star_near(&psf, x as f64, y as f64, 0.5), "star at ({x},{y}) dropped");
        }
        assert!(psf.kernel.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn brightest_unsaturated_star_is_used() {
        let mut image = noisy_background(512, 512, 100.0, 8.66, 4242);
        add_star(&mut image, 256, 256, 9900.0, 1.7);
        for (x, y, a) in [(180usize, 256usize, 1000.0f32), (332, 256, 600.0), (256, 180, 400.0), (256, 332, 300.0)] {
            add_star(&mut image, x, y, a, 1.7);
        }
        let psf = estimate_psf(&image, &PsfEstimationConfig::default()).expect("psf");
        assert!(has_star_near(&psf, 256.0, 256.0, 1.0), "bright star rejected: {:?}", psf.stars_used);
    }

    #[test]
    fn brightest_unsaturated_star_is_used_off_centre_and_when_broad() {
        let mut rejected = Vec::new();
        for sigma in [1.7f32, 2.5, 3.5, 7.0] {
            for oy in [0.0f64, 0.2, 0.5] {
                for ox in [0.0f64, 0.3, 0.4, 0.5] {
                    let mut image = noisy_background(512, 512, 100.0, 8.66, 4242);
                    let (cx, cy) = (256.0 + ox, 256.0 + oy);
                    add_star_at(&mut image, cx, cy, 9900.0, sigma);
                    for (x, y, a) in [(180.0, 256.0, 1000.0f32), (332.0, 256.0, 600.0), (256.0, 180.0, 400.0), (256.0, 332.0, 300.0)] {
                        add_star_at(&mut image, x, y, a, sigma);
                    }
                    let used = estimate_psf(&image, &PsfEstimationConfig::default())
                        .is_ok_and(|psf| has_star_near(&psf, cx, cy, 1.0));
                    if !used {
                        rejected.push((sigma, cx, cy));
                    }
                }
            }
        }
        assert!(rejected.is_empty(), "bright unsaturated star rejected at (sigma, x, y) = {rejected:?}");
    }

    fn grid_positions(n: usize, start: usize, step: usize) -> Vec<(usize, usize)> {
        (0..n * n).map(|i| (start + step * (i % n), start + step * (i / n))).collect()
    }

    #[test]
    fn a_hot_pixel_pair_does_not_raise_the_brightness_reference() {
        let mut image = noisy_background(512, 512, 100.0, 8.66, 2024);
        let stars = grid_positions(5, 136, 60);
        for (i, &(x, y)) in stars.iter().enumerate() {
            add_star(&mut image, x, y, 2000.0 + 150.0 * i as f32, 1.7);
        }
        image[[300, 95]] = 65535.0;
        image[[300, 96]] = 65535.0;

        let psf = estimate_psf(&image, &PsfEstimationConfig::default()).expect("a hot pixel pair must not reject every star");
        assert!(psf.stars_used.len() >= 20, "only {} stars used", psf.stars_used.len());
        assert!(!has_star_near(&psf, 95.5, 300.0, 3.0), "hot pair used: {:?}", psf.stars_used);
    }

    #[test]
    fn clipped_stars_are_rejected_next_to_a_hot_pixel_pair() {
        let mut image = noisy_background(512, 512, 100.0, 8.66, 77);
        let clipped = [(166usize, 166usize), (346, 166), (166, 346), (346, 346), (256, 166)];
        let unsaturated = [(256usize, 256usize), (196, 256), (316, 256), (256, 316), (196, 316), (316, 316)];
        for &(x, y) in &clipped {
            add_star(&mut image, x, y, 40000.0, 1.7);
        }
        for (i, &(x, y)) in unsaturated.iter().enumerate() {
            add_star(&mut image, x, y, 4000.0 + 500.0 * i as f32, 1.7);
        }
        image.mapv_inplace(|v| v.min(20000.0));
        image[[300, 95]] = 65535.0;
        image[[300, 96]] = 65535.0;

        let psf = estimate_psf(&image, &PsfEstimationConfig::default()).expect("psf");
        for &(x, y) in &clipped {
            assert!(!has_star_near(&psf, x as f64, y as f64, 3.0), "clipped star at ({x},{y}) used: {:?}", psf.stars_used);
        }
        for &(x, y) in &unsaturated {
            assert!(has_star_near(&psf, x as f64, y as f64, 1.0), "unsaturated star at ({x},{y}) dropped: {:?}", psf.stars_used);
        }
        assert_eq!(psf.stars_used.len(), unsaturated.len(), "{:?}", psf.stars_used);
    }

    fn assert_clipped_rejected_and_unsaturated_used(
        image: &Array2<f32>,
        clipped: &[(usize, usize, f32)],
        unsaturated: &[(usize, usize)],
    ) {
        let psf = estimate_psf(image, &PsfEstimationConfig::default()).expect("psf");
        for &(x, y, sigma) in clipped {
            assert!(!has_star_near(&psf, x as f64, y as f64, 8.0), "clipped star (sigma {sigma}) at ({x},{y}) used: {:?}", psf.stars_used);
        }
        for &(x, y) in unsaturated {
            assert!(has_star_near(&psf, x as f64, y as f64, 1.0), "unsaturated star at ({x},{y}) dropped: {:?}", psf.stars_used);
        }
    }

    #[test]
    fn broad_and_flat_fielded_clipped_cores_are_still_rejected() {
        let mut image = noisy_background(512, 512, 100.0, 8.66, 5150);
        let clipped = [(166usize, 166usize, 1.7f32), (346, 166, 3.5), (166, 346, 2.5), (346, 346, 5.0)];
        let unsaturated = [(256usize, 256usize), (196, 256), (316, 256), (256, 316)];
        for &(x, y, sigma) in &clipped {
            add_star(&mut image, x, y, 200000.0, sigma);
        }
        for (i, &(x, y)) in unsaturated.iter().enumerate() {
            add_star(&mut image, x, y, 11000.0 + 1000.0 * i as f32, 1.7);
        }
        let flat = noisy_background(512, 512, 1.0, 0.003, 616);
        image.zip_mut_with(&flat, |v, f| *v *= f);
        image.mapv_inplace(|v| v.min(20000.0));
        image.zip_mut_with(&flat, |v, f| *v /= f);

        assert_clipped_rejected_and_unsaturated_used(&image, &clipped, &unsaturated);
    }

    #[test]
    fn a_lone_broad_clipped_star_is_rejected_by_its_own_plateau() {
        let mut image = noisy_background(512, 512, 100.0, 8.66, 3131);
        let clipped = [(346usize, 346usize, 5.0f32)];
        let unsaturated = [(256usize, 256usize), (196, 256), (316, 256), (256, 316)];
        add_star(&mut image, 346, 346, 200000.0, 5.0);
        for (i, &(x, y)) in unsaturated.iter().enumerate() {
            add_star(&mut image, x, y, 11000.0 + 1000.0 * i as f32, 1.7);
        }
        image.mapv_inplace(|v| v.min(20000.0));

        assert_clipped_rejected_and_unsaturated_used(&image, &clipped, &unsaturated);
    }

    #[test]
    fn clipped_cores_of_normalized_data_are_rejected() {
        let mut image = noisy_background(512, 512, 0.1, 0.004, 8080);
        let clipped = [(166usize, 166usize, 1.7f32), (346, 166, 3.5), (166, 346, 2.5)];
        let unsaturated = [(256usize, 256usize), (196, 256), (316, 256), (256, 316)];
        for &(x, y, sigma) in &clipped {
            add_star(&mut image, x, y, 9.0, sigma);
        }
        for (i, &(x, y)) in unsaturated.iter().enumerate() {
            add_star(&mut image, x, y, 0.45 + 0.1 * i as f32, 1.7);
        }
        image.mapv_inplace(|v| v.min(1.0));

        assert_clipped_rejected_and_unsaturated_used(&image, &clipped, &unsaturated);
    }

    #[test]
    fn flat_topped_star_is_rejected_as_saturated() {
        let mut image = noisy_background(256, 256, 100.0, 0.5, 99);
        add_star(&mut image, 100, 128, 30000.0, 1.7);
        image.mapv_inplace(|v| v.min(20000.0));
        add_star(&mut image, 156, 128, 9900.0, 1.7);
        let psf = estimate_psf(&image, &PsfEstimationConfig::default()).expect("psf");
        assert!(has_star_near(&psf, 156.0, 128.0, 1.0), "unsaturated star rejected: {:?}", psf.stars_used);
        assert!(!has_star_near(&psf, 100.0, 128.0, 3.0), "clipped star used: {:?}", psf.stars_used);
    }
}
