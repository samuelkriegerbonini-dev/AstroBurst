use std::f64::consts::{LN_2, PI};

use ndarray::{Array2, s};
use serde::{Deserialize, Serialize};

use crate::core::cubeprep::cube::set_text;
use crate::core::cubeprep::psf_io::{normalize_psf, psf_header_base, set_local_psf_wcs};
use crate::core::imaging::push_history;
use crate::core::imaging::region::RegionSystem;
use crate::core::imaging::region_file::{parse_lat, parse_lon, Region};
use crate::core::imaging::stats::is_valid_pixel;
use crate::math::bspline::{shift, zoom, SplineMode};
use crate::math::gauss_newton::solve_linear;
use crate::math::median::{exact_mad_mut, exact_median_f64, exact_median_mut};
use crate::math::sigma_clip::std_clipped_stats;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::header::HduHeader;

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

pub fn propose_psf_stars(image: &Array2<f32>, config: &PsfEstimationConfig) -> Result<Vec<StarCandidate>, String> {
    select_psf_stars(image, config).map(|(selected, _)| selected)
}

fn select_psf_stars(image: &Array2<f32>, config: &PsfEstimationConfig) -> Result<(Vec<StarCandidate>, usize), String> {
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
    candidates.truncate(config.num_stars);
    Ok((candidates, detected_total))
}

pub fn estimate_psf(
    image: &Array2<f32>,
    config: &PsfEstimationConfig,
) -> Result<PsfResult, String> {
    let (selected, detected_total) = select_psf_stars(image, config)?;

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
        stars_used: selected,
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

pub const QUADRATIC_FIT_BOXSIZE: usize = 5;
const BACKGROUND_CLIP_ITERATIONS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PsfStack {
    Median,
    Mean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RadiusUnit {
    Pixel,
    Arcsec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmpiricalPsfOptions {
    pub cutout_size: usize,
    pub background_border: usize,
    pub background_sigma: f64,
    pub clip_negative: bool,
    pub noise_threshold: Option<f64>,
    pub radial_cleaning: bool,
    pub cleaning_sigma: f64,
    pub cleaning_consecutive_bins: usize,
    pub normalization_radius: f64,
    pub normalization_radius_unit: RadiusUnit,
    pub psf_size: usize,
    pub oversampling: Option<usize>,
    pub stack: PsfStack,
}

impl Default for EmpiricalPsfOptions {
    fn default() -> Self {
        Self {
            cutout_size: 61,
            background_border: 10,
            background_sigma: 4.0,
            clip_negative: true,
            noise_threshold: Some(0.5),
            radial_cleaning: false,
            cleaning_sigma: 1.0,
            cleaning_consecutive_bins: 3,
            normalization_radius: 20.0,
            normalization_radius_unit: RadiusUnit::Pixel,
            psf_size: 51,
            oversampling: None,
            stack: PsfStack::Median,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PsfStar {
    pub name: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CentroidMethod {
    Quadratic,
    Peak,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StarReport {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub used: bool,
    pub rejected: Option<String>,
    pub centroid: Option<CentroidMethod>,
    pub centre_x: Option<f64>,
    pub centre_y: Option<f64>,
    pub background: Option<f64>,
    pub background_std: Option<f64>,
    pub cleaning_radius: Option<f64>,
    pub aperture_flux: Option<f64>,
    pub profile: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmpiricalPsf {
    pub psf: Array2<f64>,
    pub oversampled: Option<Array2<f64>>,
    pub stars: Vec<StarReport>,
    pub normalization_radius_px: f64,
    pub profile: Vec<f64>,
}

impl EmpiricalPsf {
    pub fn stars_used(&self) -> usize {
        self.stars.iter().filter(|star| star.used).count()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PsfHeaderLabels<'a> {
    pub filter: Option<&'a str>,
    pub dataset: Option<&'a str>,
    pub pixel_scale_arcsec: Option<f64>,
    pub stars_used: usize,
}

pub fn build_empirical_psf(
    image: &Array2<f32>,
    stars: &[PsfStar],
    pixel_scale_arcsec: Option<f64>,
    options: &EmpiricalPsfOptions,
) -> Result<EmpiricalPsf, String> {
    validate_recipe(options)?;
    let radius = normalization_radius_px(options, pixel_scale_arcsec)?;
    if stars.is_empty() {
        return Err("no star given; list stars from regions, coordinates or the automatic finder".into());
    }
    let measured: Vec<(StarReport, Option<Array2<f64>>)> = stars.iter().map(|star| measure_star(image, star, radius, options)).collect();
    let cutouts: Vec<&Array2<f64>> = measured.iter().filter_map(|(_, cutout)| cutout.as_ref()).collect();
    if cutouts.is_empty() {
        let reasons: Vec<String> = measured
            .iter()
            .map(|(report, _)| format!("{}: {}", report.name, report.rejected.as_deref().unwrap_or_default()))
            .collect();
        return Err(format!("no star could be used ({})", reasons.join("; ")));
    }
    let stacked = stack_cutouts(&cutouts, options.stack);
    let psf = normalize_psf(&centre_crop(&stacked, options.psf_size)).map_err(|error| error.to_string())?;
    let oversampled = options
        .oversampling
        .map(|factor| normalize_psf(&zoom(&psf, [factor as f64, factor as f64], SplineMode::Nearest)).map_err(|error| error.to_string()))
        .transpose()?;
    Ok(EmpiricalPsf {
        profile: radial_profile(&psf),
        psf,
        oversampled,
        stars: measured.into_iter().map(|(report, _)| report).collect(),
        normalization_radius_px: radius,
    })
}

pub(crate) fn validate_recipe(options: &EmpiricalPsfOptions) -> Result<(), String> {
    for (name, value) in [("cutout_size", options.cutout_size), ("psf_size", options.psf_size)] {
        if value % 2 == 0 {
            return Err(format!("{name} must be a positive odd number of pixels, got {value}"));
        }
    }
    if options.psf_size > options.cutout_size {
        return Err(format!("psf_size ({}) cannot be larger than cutout_size ({})", options.psf_size, options.cutout_size));
    }
    if options.oversampling == Some(0) {
        return Err("oversampling must be a positive integer".into());
    }
    if options.background_border == 0 {
        return Err("background_border must be at least 1 pixel".into());
    }
    if !(options.background_sigma.is_finite() && options.background_sigma > 0.0) {
        return Err(format!("background_sigma must be a positive number, got {}", options.background_sigma));
    }
    if options.radial_cleaning && options.cleaning_consecutive_bins == 0 {
        return Err("cleaning_consecutive_bins must be at least 1".into());
    }
    Ok(())
}

fn normalization_radius_px(options: &EmpiricalPsfOptions, pixel_scale_arcsec: Option<f64>) -> Result<f64, String> {
    let radius = match options.normalization_radius_unit {
        RadiusUnit::Pixel => options.normalization_radius,
        RadiusUnit::Arcsec => {
            let scale = pixel_scale_arcsec
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .ok_or("an aperture radius in arcsec needs the pixel scale; the image has no celestial WCS, so give pixel_scale_arcsec")?;
            options.normalization_radius / scale
        }
    };
    if radius.is_finite() && radius > 0.0 {
        Ok(radius)
    } else {
        Err(format!("normalization_radius must be positive, got {radius} px"))
    }
}

fn measure_star(image: &Array2<f32>, star: &PsfStar, radius: f64, options: &EmpiricalPsfOptions) -> (StarReport, Option<Array2<f64>>) {
    let mut report = StarReport {
        name: star.name.clone(),
        x: star.x,
        y: star.y,
        used: false,
        rejected: None,
        centroid: None,
        centre_x: None,
        centre_y: None,
        background: None,
        background_std: None,
        cleaning_radius: None,
        aperture_flux: None,
        profile: Vec::new(),
    };
    match normalized_cutout(image, star, radius, options, &mut report) {
        Ok(cutout) => {
            report.used = true;
            report.profile = radial_profile(&cutout);
            (report, Some(cutout))
        }
        Err(reason) => {
            report.rejected = Some(reason);
            (report, None)
        }
    }
}

fn normalized_cutout(
    image: &Array2<f32>,
    star: &PsfStar,
    radius: f64,
    options: &EmpiricalPsfOptions,
    report: &mut StarReport,
) -> Result<Array2<f64>, String> {
    let size = options.cutout_size;
    let cutout = partial_cutout(image, star.x, star.y, size).ok_or("the cutout lies outside the image")?;
    let (corrected, background, background_std) =
        subtract_border_background(&cutout, options.background_border, options.background_sigma, options.clip_negative, options.noise_threshold)
            .ok_or("no finite pixel in the background border")?;
    report.background = Some(background);
    report.background_std = Some(background_std);
    let (x, y, method) = measure_centre(&corrected).ok_or("no finite pixel in the cutout")?;
    let (x0, y0) = cutout_origin(star.x, star.y, size);
    report.centroid = Some(method);
    report.centre_x = Some(x0 as f64 + x);
    report.centre_y = Some(y0 as f64 + y);
    let (centred, missing) = recentre(&corrected, x, y);
    let cleaned = if options.radial_cleaning {
        let (cleaned, cleaning_radius) = clean_radially(&centred, background_std, options.cleaning_sigma, options.cleaning_consecutive_bins);
        report.cleaning_radius = cleaning_radius.map(|bin| bin as f64);
        cleaned
    } else {
        centred
    };
    let (rows, cols) = cleaned.dim();
    let inside = |r: usize, c: usize| radius_at(rows, cols, r, c) <= radius;
    if cleaned.indexed_iter().any(|((r, c), v)| v.is_nan() && missing[[r, c]] && inside(r, c)) {
        return Err(format!("missing or off-image pixels fall inside the {radius} px normalization aperture"));
    }
    let flux: f64 = cleaned.indexed_iter().filter(|((r, c), v)| !v.is_nan() && inside(*r, *c)).map(|(_, v)| *v).sum();
    report.aperture_flux = Some(flux);
    if !(flux.is_finite() && flux > 0.0) {
        return Err(format!("the aperture flux {flux} is not positive"));
    }
    Ok(cleaned.mapv(|v| v / flux))
}

fn radius_at(rows: usize, cols: usize, r: usize, c: usize) -> f64 {
    (c as f64 - (cols as f64 - 1.0) / 2.0).hypot(r as f64 - (rows as f64 - 1.0) / 2.0)
}

fn cutout_origin(x: f64, y: f64, size: usize) -> (i64, i64) {
    let half = size as f64 / 2.0;
    ((x - half).ceil() as i64, (y - half).ceil() as i64)
}

fn partial_cutout(image: &Array2<f32>, x: f64, y: f64, size: usize) -> Option<Array2<f64>> {
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let (rows, cols) = image.dim();
    let (x0, y0) = cutout_origin(x, y, size);
    let reach = size as i64;
    if x0 >= cols as i64 || y0 >= rows as i64 || x0 + reach <= 0 || y0 + reach <= 0 {
        return None;
    }
    Some(Array2::from_shape_fn((size, size), |(r, c)| {
        let (row, col) = (y0 + r as i64, x0 + c as i64);
        if row >= 0 && col >= 0 && row < rows as i64 && col < cols as i64 {
            image[[row as usize, col as usize]] as f64
        } else {
            f64::NAN
        }
    }))
}

fn subtract_border_background(
    cutout: &Array2<f64>,
    border: usize,
    sigma: f64,
    clip_negative: bool,
    noise_threshold: Option<f64>,
) -> Option<(Array2<f64>, f64, f64)> {
    let (rows, cols) = cutout.dim();
    let values: Vec<f64> = cutout
        .indexed_iter()
        .filter(|((r, c), _)| *r < border || *c < border || *r + border >= rows || *c + border >= cols)
        .map(|(_, v)| *v)
        .collect();
    let stats = std_clipped_stats(&values, sigma, BACKGROUND_CLIP_ITERATIONS);
    if stats.count == 0 {
        return None;
    }
    let corrected = cutout.mapv(|v| {
        let difference = v - stats.median;
        let difference = if clip_negative && difference < 0.0 { 0.0 } else { difference };
        match noise_threshold {
            Some(threshold) if difference < threshold * stats.std => 0.0,
            _ => difference,
        }
    });
    Some((corrected, stats.median, stats.std))
}

fn finite_argmax(data: &Array2<f64>) -> Option<(usize, usize)> {
    let mut best: Option<((usize, usize), f64)> = None;
    for (at, &value) in data.indexed_iter() {
        if value.is_finite() && best.is_none_or(|(_, top)| value > top) {
            best = Some((at, value));
        }
    }
    best.map(|(at, _)| at)
}

fn fit_window(peak: usize, size: usize, len: usize) -> (usize, usize) {
    let mut start = peak.saturating_sub(size / 2);
    let mut end = (peak + size / 2 + 1).min(len);
    if end - start < size {
        if start == 0 {
            end = size.min(len);
        }
        if end == len {
            start = len.saturating_sub(size);
        }
    }
    (start, end)
}

pub fn centroid_quadratic(data: &Array2<f64>, fit_boxsize: usize) -> Option<(f64, f64)> {
    let (rows, cols) = data.dim();
    let (peak_row, peak_col) = finite_argmax(data)?;
    if peak_row == 0 || peak_col == 0 || peak_row + 1 == rows || peak_col + 1 == cols {
        return Some((peak_col as f64, peak_row as f64));
    }
    let odd = |len: usize| {
        let clamped = fit_boxsize.min(len);
        if clamped % 2 == 0 { clamped.saturating_sub(1) } else { clamped }
    };
    let (row_start, row_end) = fit_window(peak_row, odd(rows), rows);
    let (col_start, col_end) = fit_window(peak_col, odd(cols), cols);
    let mut normal = vec![vec![0.0; 6]; 6];
    let mut rhs = vec![0.0; 6];
    let mut points = 0usize;
    let (mut fitted_rows, mut fitted_cols) = (Vec::new(), Vec::new());
    for r in row_start..row_end {
        for c in col_start..col_end {
            let value = data[[r, c]];
            if !value.is_finite() {
                continue;
            }
            let (x, y) = (c as f64 - peak_col as f64, r as f64 - peak_row as f64);
            let basis = [1.0, x, y, x * y, x * x, y * y];
            for (i, row) in normal.iter_mut().enumerate() {
                rhs[i] += basis[i] * value;
                for (j, cell) in row.iter_mut().enumerate() {
                    *cell += basis[i] * basis[j];
                }
            }
            points += 1;
            if !fitted_rows.contains(&r) {
                fitted_rows.push(r);
            }
            if !fitted_cols.contains(&c) {
                fitted_cols.push(c);
            }
        }
    }
    if points < 6 || fitted_rows.len() < 3 || fitted_cols.len() < 3 {
        return None;
    }
    let coefficients = solve_linear(&mut normal, &mut rhs)?;
    let [_, c10, c01, c11, c20, c02] = coefficients[..] else {
        return None;
    };
    let det = 4.0 * c20 * c02 - c11 * c11;
    if det <= 0.0 || c20 > 0.0 {
        return None;
    }
    let x = (c01 * c11 - 2.0 * c02 * c10) / det + peak_col as f64;
    let y = (c10 * c11 - 2.0 * c20 * c01) / det + peak_row as f64;
    let inside = |v: f64, len: usize| v > 0.0 && v < (len - 1) as f64;
    (inside(x, cols) && inside(y, rows)).then_some((x, y))
}

fn measure_centre(data: &Array2<f64>) -> Option<(f64, f64, CentroidMethod)> {
    if let Some((x, y)) = centroid_quadratic(data, QUADRATIC_FIT_BOXSIZE) {
        return Some((x, y, CentroidMethod::Quadratic));
    }
    finite_argmax(data).map(|(r, c)| (c as f64, r as f64, CentroidMethod::Peak))
}

fn support_has_missing(data: &Array2<f64>, y: f64, x: f64) -> bool {
    let (rows, cols) = data.dim();
    let span = |coordinate: f64, len: usize| {
        let floor = coordinate.floor() as i64;
        ((floor - 1).max(0) as usize)..=((floor + 2).min(len as i64 - 1) as usize)
    };
    span(y, rows).any(|r| span(x, cols).any(|c| !data[[r, c]].is_finite()))
}

fn recentre(data: &Array2<f64>, x: f64, y: f64) -> (Array2<f64>, Array2<bool>) {
    let (rows, cols) = data.dim();
    let offset = [(rows as f64 - 1.0) / 2.0 - y, (cols as f64 - 1.0) / 2.0 - x];
    let mut missing = Array2::from_elem((rows, cols), false);
    if data.iter().all(|v| v.is_finite()) {
        return (shift(data, offset, SplineMode::Constant(f64::NAN)), missing);
    }
    let filled = data.mapv(|v| if v.is_finite() { v } else { 0.0 });
    let mut shifted = shift(&filled, offset, SplineMode::Constant(f64::NAN));
    for ((r, c), value) in shifted.indexed_iter_mut() {
        if !value.is_nan() && support_has_missing(data, r as f64 - offset[0], c as f64 - offset[1]) {
            *value = f64::NAN;
            missing[[r, c]] = true;
        }
    }
    (shifted, missing)
}

fn radial_profile(image: &Array2<f64>) -> Vec<f64> {
    let (rows, cols) = image.dim();
    let (mut sums, mut counts): (Vec<f64>, Vec<usize>) = (Vec::new(), Vec::new());
    for ((r, c), &value) in image.indexed_iter() {
        let bin = radius_at(rows, cols, r, c) as usize;
        if bin >= sums.len() {
            sums.resize(bin + 1, 0.0);
            counts.resize(bin + 1, 0);
        }
        if !value.is_nan() {
            sums[bin] += value;
            counts[bin] += 1;
        }
    }
    sums.iter().zip(&counts).map(|(sum, &count)| if count > 0 { sum / count as f64 } else { f64::NAN }).collect()
}

fn clean_radially(image: &Array2<f64>, background_std: f64, sigma: f64, consecutive: usize) -> (Array2<f64>, Option<usize>) {
    let threshold = sigma * background_std;
    let below: Vec<bool> = radial_profile(image).iter().map(|v| v.is_finite() && *v < threshold).collect();
    let Some(radius) = (0..(below.len() + 1).saturating_sub(consecutive)).find(|&i| below[i..i + consecutive].iter().all(|b| *b)) else {
        return (image.clone(), None);
    };
    let (rows, cols) = image.dim();
    let cleaned = Array2::from_shape_fn((rows, cols), |(r, c)| if radius_at(rows, cols, r, c) > radius as f64 { 0.0 } else { image[[r, c]] });
    (cleaned, Some(radius))
}

fn stack_cutouts(cutouts: &[&Array2<f64>], method: PsfStack) -> Array2<f64> {
    let mut values = Vec::with_capacity(cutouts.len());
    Array2::from_shape_fn(cutouts[0].dim(), |at| {
        values.clear();
        values.extend(cutouts.iter().map(|cutout| cutout[at]).filter(|v| !v.is_nan()));
        if values.is_empty() {
            return f64::NAN;
        }
        match method {
            PsfStack::Median => exact_median_f64(&values),
            PsfStack::Mean => values.iter().sum::<f64>() / values.len() as f64,
        }
    })
}

fn centre_crop(image: &Array2<f64>, size: usize) -> Array2<f64> {
    let (rows, cols) = image.dim();
    let (r0, c0) = ((rows - size) / 2, (cols - size) / 2);
    image.slice(s![r0..r0 + size, c0..c0 + size]).to_owned()
}

pub fn stars_from_regions(regions: &[Region]) -> Vec<PsfStar> {
    regions
        .iter()
        .filter(|region| region.props.include)
        .enumerate()
        .map(|(index, region)| {
            let (x, y) = region.shape.centre();
            let name = region
                .props
                .text
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map_or_else(|| format!("region{}", index + 1), str::to_string);
            PsfStar { name, x, y }
        })
        .collect()
}

pub fn parse_radec_list(text: &str) -> Result<Vec<(String, f64, f64)>, String> {
    let mut stars = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let tokens: Vec<&str> = line.split(|c: char| c == ',' || c.is_whitespace()).filter(|token| !token.is_empty()).collect();
        let at = |message: String| format!("line {}: {message}", index + 1);
        if tokens.len() < 2 {
            return Err(at(format!("expected RA and Dec, got '{line}'")));
        }
        let ra = parse_lon(tokens[0], RegionSystem::Icrs).map_err(|error| at(error.to_string()))?;
        let dec = parse_lat(tokens[1]).map_err(|error| at(error.to_string()))?;
        if !(ra.is_finite() && dec.is_finite() && dec.abs() <= 90.0) {
            return Err(at(format!("RA {ra} and Dec {dec} are not sky coordinates in degrees")));
        }
        let name = if tokens.len() > 2 { tokens[2..].join(" ") } else { format!("star{}", stars.len() + 1) };
        stars.push((name, ra, dec));
    }
    if stars.is_empty() {
        return Err("the list holds no star; give one RA and Dec per line".into());
    }
    Ok(stars)
}

pub fn empirical_psf_header(
    data: &Array2<f64>,
    labels: &PsfHeaderLabels,
    options: &EmpiricalPsfOptions,
    oversampling: Option<usize>,
) -> HduHeader {
    let (rows, cols) = data.dim();
    let scale = labels.pixel_scale_arcsec.map(|scale| scale / oversampling.unwrap_or(1) as f64);
    let mut header = psf_header_base();
    if let Some(scale) = scale {
        set_local_psf_wcs(&mut header, rows, cols, scale);
    }
    set_text(&mut header, "BUNIT", "normalized");
    if let Some(filter) = labels.filter {
        set_text(&mut header, "FILTER", &filter.to_uppercase());
    }
    if let Some(dataset) = labels.dataset {
        set_text(&mut header, "DATASET", &dataset.to_uppercase());
    }
    header.set("NSTARS", labels.stars_used.to_string());
    header.set("CUTSIZE", options.cutout_size.to_string());
    header.set("PSFSIZE", oversampling.map_or(options.psf_size, |_| rows).to_string());
    header.set_f64("APRAD", options.normalization_radius);
    let unit = match options.normalization_radius_unit {
        RadiusUnit::Pixel => "pixel",
        RadiusUnit::Arcsec => "arcsec",
    };
    set_text(&mut header, "APRUNIT", unit);
    if let Some(scale) = scale {
        header.set_f64("PIXSCALE", scale);
    }
    let method = match options.stack {
        PsfStack::Median => "median",
        PsfStack::Mean => "mean",
    };
    set_text(&mut header, "METHOD", method);
    if let Some(factor) = oversampling {
        header.set("OVERSAMP", factor.to_string());
    }
    push_history(&mut header, "Empirical PSF constructed from stellar cutouts.");
    push_history(&mut header, "PSF WCS is local and independent of the science-image WCS.");
    if let Some(factor) = oversampling {
        push_history(&mut header, &format!("PSF oversampled by {factor} with a cubic B-spline zoom (scipy.ndimage.zoom semantics, nearest mode)."));
    }
    header
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
    fn proposed_stars_are_the_stars_the_automatic_estimate_uses() {
        let (image, _) = star_field_with_nan_rings();
        let config = PsfEstimationConfig { num_stars: 12, ..PsfEstimationConfig::default() };
        let proposed = propose_psf_stars(&image, &config).expect("proposals");
        let used = estimate_psf(&image, &config).expect("psf").stars_used;
        assert_eq!(proposed.len(), 12);
        let positions = |stars: &[StarCandidate]| stars.iter().map(|s| (s.x, s.y)).collect::<Vec<_>>();
        assert_eq!(positions(&proposed), positions(&used));
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

#[cfg(test)]
mod empirical_tests {
    use super::*;
    use crate::core::cubeprep::psf_io::normalize_psf;
    use crate::core::imaging::region::RegionShape;
    use crate::core::imaging::region_file::RegionProperties;
    use crate::math::bspline::{shift, zoom, SplineMode};
    use crate::math::sigma_clip::std_clipped_stats;

    const STAMP: usize = 41;
    const STAMP_CENTRE: (f64, f64) = (20.3, 20.6);

    fn gaussian_stamp(side: usize, x: f64, y: f64, flux: f64, sigma: f64) -> Array2<f32> {
        let norm = flux / (2.0 * PI * sigma * sigma);
        Array2::from_shape_fn((side, side), |(r, c)| {
            let d2 = (c as f64 - x).powi(2) + (r as f64 - y).powi(2);
            (norm * (-0.5 * d2 / (sigma * sigma)).exp()) as f32
        })
    }

    fn paste(image: &mut Array2<f32>, stamp: &Array2<f32>, row: usize, col: usize) {
        let (h, w) = stamp.dim();
        let mut target = image.slice_mut(s![row..row + h, col..col + w]);
        target += stamp;
    }

    fn add_gaussian(image: &mut Array2<f32>, x: f64, y: f64, flux: f64, sigma: f64) {
        let norm = flux / (2.0 * PI * sigma * sigma);
        for ((r, c), value) in image.indexed_iter_mut() {
            let d2 = (c as f64 - x).powi(2) + (r as f64 - y).powi(2);
            if d2 < (8.0 * sigma).powi(2) {
                *value += (norm * (-0.5 * d2 / (sigma * sigma)).exp()) as f32;
            }
        }
    }

    const CORNERS: [(usize, usize); 5] = [(20, 20), (20, 110), (20, 200), (120, 20), (120, 110)];

    fn field_of(stamps: &[Array2<f32>]) -> (Array2<f32>, Vec<PsfStar>) {
        let mut image = Array2::<f32>::from_elem((220, 300), 5.0);
        let stars = stamps
            .iter()
            .zip(CORNERS)
            .enumerate()
            .map(|(i, (stamp, (row, col)))| {
                paste(&mut image, stamp, row, col);
                PsfStar { name: format!("s{}", i + 1), x: col as f64 + STAMP_CENTRE.0, y: row as f64 + STAMP_CENTRE.1 }
            })
            .collect();
        (image, stars)
    }

    fn identical_star_field() -> (Array2<f32>, Vec<PsfStar>) {
        let stamp = gaussian_stamp(STAMP, STAMP_CENTRE.0, STAMP_CENTRE.1, 5000.0, 1.8);
        field_of(&vec![stamp; CORNERS.len()])
    }

    fn astropy_gaussian(shape: (usize, usize), amplitude: f64, xc: f64, yc: f64, xstd: f64, ystd: f64, theta: f64) -> Array2<f64> {
        let (cos2, sin2, sin2t) = (theta.cos().powi(2), theta.sin().powi(2), (2.0 * theta).sin());
        let (xs2, ys2) = (xstd * xstd, ystd * ystd);
        let a = 0.5 * (cos2 / xs2 + sin2 / ys2);
        let b = 0.5 * (sin2t / xs2 - sin2t / ys2);
        let c = 0.5 * (sin2 / xs2 + cos2 / ys2);
        Array2::from_shape_fn(shape, |(r, col)| {
            let (dx, dy) = (col as f64 - xc, r as f64 - yc);
            amplitude * (-(a * dx * dx + b * dx * dy + c * dy * dy)).exp()
        })
    }

    fn radius_from_centre(rows: usize, cols: usize, r: usize, c: usize) -> f64 {
        (c as f64 - (cols as f64 - 1.0) / 2.0).hypot(r as f64 - (rows as f64 - 1.0) / 2.0)
    }

    #[test]
    fn median_stack_of_identical_stars_returns_that_star() {
        let (image, stars) = identical_star_field();
        let options = EmpiricalPsfOptions::default();
        let one = build_empirical_psf(&image, &stars[..1], None, &options).expect("one star");
        let all = build_empirical_psf(&image, &stars, None, &options).expect("all stars");
        assert_eq!(all.stars_used(), stars.len(), "{:?}", all.stars);
        assert_eq!(all.psf.dim(), (51, 51));
        assert_eq!(all.psf, one.psf);
        assert!((all.psf.sum() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn border_background_is_sigma_clipped() {
        let size = 21;
        let mut cutout = Array2::<f64>::from_shape_fn((size, size), |(r, c)| 10.0 + 0.1 * (((r * 7 + c * 13) % 11) as f64 - 5.0));
        cutout[[0, 3]] = 900.0;
        cutout[[1, 4]] = 700.0;
        cutout[[20, 20]] = -500.0;
        cutout[[10, 10]] = 60.0;
        let border = 3;
        let values: Vec<f64> = cutout
            .indexed_iter()
            .filter(|((r, c), _)| *r < border || *c < border || *r >= size - border || *c >= size - border)
            .map(|(_, v)| *v)
            .collect();
        let expected = std_clipped_stats(&values, 4.0, 5);
        assert_eq!(expected.count, values.len() - 3);
        let (corrected, background, std) = subtract_border_background(&cutout, border, 4.0, true, Some(0.5)).expect("a background");
        assert_eq!((background, std), (expected.median, expected.std));
        let unclipped_mean = values.iter().sum::<f64>() / values.len() as f64;
        assert!((unclipped_mean - background).abs() > 1.0, "the outliers would bias an unclipped mean: {unclipped_mean}");
        for ((r, c), value) in cutout.indexed_iter() {
            let difference = value - background;
            let want = if difference < 0.0 || difference < 0.5 * std { 0.0 } else { difference };
            assert_eq!(corrected[[r, c]], want, "pixel ({r}, {c})");
        }
        assert_eq!(corrected[[10, 10]], 60.0 - background);
        let (kept, _, _) = subtract_border_background(&cutout, border, 4.0, false, None).unwrap();
        assert_eq!(kept[[20, 20]], -500.0 - background);
    }

    #[test]
    fn aperture_radius_in_arcsec_uses_pixel_scale() {
        let (image, stars) = identical_star_field();
        let in_pixels = EmpiricalPsfOptions { normalization_radius: 16.0, ..EmpiricalPsfOptions::default() };
        let in_arcsec = EmpiricalPsfOptions {
            normalization_radius: 0.5,
            normalization_radius_unit: RadiusUnit::Arcsec,
            ..EmpiricalPsfOptions::default()
        };
        let fluxes = |psf: &EmpiricalPsf| psf.stars.iter().map(|s| s.aperture_flux.expect("a flux")).collect::<Vec<_>>();
        let by_pixels = build_empirical_psf(&image, &stars, None, &in_pixels).unwrap();
        let by_arcsec = build_empirical_psf(&image, &stars, Some(0.03125), &in_arcsec).unwrap();
        assert_eq!(by_arcsec.normalization_radius_px, 16.0);
        assert_eq!(fluxes(&by_arcsec), fluxes(&by_pixels));
        let coarse = build_empirical_psf(&image, &stars, Some(0.0625), &in_arcsec).unwrap();
        assert_eq!(coarse.normalization_radius_px, 8.0);
        assert!(fluxes(&coarse)[0] < fluxes(&by_pixels)[0]);
        let missing = build_empirical_psf(&image, &stars, None, &in_arcsec).unwrap_err();
        assert!(missing.contains("pixel scale"), "{missing}");
    }

    #[test]
    fn quadratic_centroid_matches_photutils_reference() {
        let near = |got: Option<(f64, f64)>, want: (f64, f64), tol: f64| {
            let (x, y) = got.unwrap_or_else(|| panic!("no centroid, want {want:?}"));
            assert!((x - want.0).abs() <= tol && (y - want.1).abs() <= tol, "got ({x}, {y}), want {want:?} within {tol}");
        };
        let mut spikes = Array2::<f64>::zeros((11, 11));
        spikes[[5, 5]] = 100.0;
        spikes[[7, 7]] = 110.0;
        spikes[[9, 9]] = 120.0;
        near(centroid_quadratic(&spikes, 3), (9.0, 9.0), 1e-9);

        let mut edge = Array2::<f64>::zeros((11, 11));
        edge[[1, 1]] = 100.0;
        edge[[9, 9]] = 100.0;
        near(centroid_quadratic(&edge, 5), (0.923077, 0.923077), 1e-6);

        let mut corner = Array2::<f64>::zeros((5, 5));
        corner[[0, 0]] = 100.0;
        assert_eq!(centroid_quadratic(&corner, 5), Some((0.0, 0.0)));

        for (xstd, ystd) in [(3.2, 5.7), (3.2, 4.1), (4.0, 5.7), (4.0, 4.1)] {
            for theta in [30f64.to_radians(), 45f64.to_radians()] {
                let mut data = astropy_gaussian((50, 47), 2.4, 25.7, 26.2, xstd, ystd, theta);
                near(centroid_quadratic(&data, 5), (25.7, 26.2), 0.015);
                data[[15, 11]] = f64::NAN;
                near(centroid_quadratic(&data, 5), (25.7, 26.2), 0.015);
            }
        }

        let wide = astropy_gaussian((51, 51), 2.4, 24.7, 25.2, 5.0, 5.0, 0.0).mapv(|v| if v < 1.0 { f64::NAN } else { v });
        near(centroid_quadratic(&wide, 5), (24.7, 25.2), 0.01);

        let mut holed = astropy_gaussian((11, 11), 100.0, 5.0, 5.0, 2.0, 2.0, 0.0);
        holed[[5, 4]] = f64::NAN;
        near(centroid_quadratic(&holed, 5), (5.0, 5.0), 0.01);

        near(centroid_quadratic(&astropy_gaussian((4, 4), 10.0, 2.0, 2.0, 1.0, 1.0, 0.0), 5), (2.0, 2.0), 0.1);
        near(centroid_quadratic(&astropy_gaussian((5, 5), 10.0, 2.0, 2.0, 1.0, 1.0, 0.0), 7), (2.0, 2.0), 0.1);
        near(centroid_quadratic(&astropy_gaussian((16000, 31), 2.4, 15.7, 15980.2, 2.5, 2.5, 0.0), 5), (15.7, 15980.2), 0.03);

        let mut sparse = Array2::<f64>::zeros((3, 3));
        sparse[[1, 1]] = 1.0;
        sparse.row_mut(0).fill(f64::NAN);
        sparse.row_mut(2).fill(f64::NAN);
        assert_eq!(centroid_quadratic(&sparse, 3), None);
        assert_eq!(centroid_quadratic(&Array2::from_elem((11, 11), f64::NAN), 5), None);
    }

    fn binned_image(size: usize, bins: &[f64]) -> Array2<f64> {
        Array2::from_shape_fn((size, size), |(r, c)| bins[radius_from_centre(size, size, r, c) as usize])
    }

    #[test]
    fn radial_cleaning_radius_matches_lightstack() {
        let bins = [50.0, 40.0, 30.0, 20.0, 10.0, 5.0, 3.0, 1.0, 1.5, 2.5, 1.0, 0.5, 0.2, 0.1, 0.1];
        let mut image = binned_image(21, &bins);
        image[[10, 13]] = f64::NAN;
        image[[0, 0]] = f64::NAN;
        let (cleaned, radius) = clean_radially(&image, 2.0, 1.0, 3);
        assert_eq!(radius, Some(10), "bins 7 and 8 fall below 2.0 but bin 9 rises again, so the first run of three starts at bin 10");
        for ((r, c), value) in image.indexed_iter() {
            let got = cleaned[[r, c]];
            if radius_from_centre(21, 21, r, c) > 10.0 {
                assert_eq!(got, 0.0, "pixel ({r}, {c}) beyond the radius, NaN included");
            } else {
                assert!(got == *value || (got.is_nan() && value.is_nan()), "pixel ({r}, {c}) inside the radius changed");
            }
        }
        assert_eq!(clean_radially(&image, 2.0, 1.0, 2).1, Some(7));
        let (unchanged, none) = clean_radially(&image, 0.01, 1.0, 3);
        assert_eq!(none, None);
        assert!(unchanged.iter().zip(image.iter()).all(|(a, b)| a == b || (a.is_nan() && b.is_nan())));
    }

    #[test]
    fn cutout_window_follows_cutout2d_partial_rounding() {
        let image = Array2::<f32>::from_shape_fn((40, 50), |(r, c)| (r * 100 + c) as f32);
        let at = |x: f64, y: f64| partial_cutout(&image, x, y, 5).expect("an overlapping cutout");
        assert_eq!(at(10.3, 20.0)[[2, 2]], 2010.0);
        assert_eq!(at(10.5, 20.5)[[2, 2]], 2010.0);
        assert_eq!(at(10.51, 20.6)[[2, 2]], 2111.0);
        let corner = at(0.2, 1.0);
        assert!(corner[[0, 0]].is_nan() && corner[[1, 1]].is_nan() && corner[[0, 2]].is_nan());
        assert_eq!((corner[[1, 2]], corner[[2, 2]]), (0.0, 100.0));
        assert!(partial_cutout(&image, -10.0, 5.0, 5).is_none());
        assert!(partial_cutout(&image, 60.0, 5.0, 5).is_none());
        assert!(partial_cutout(&image, f64::NAN, 5.0, 5).is_none());
    }

    #[test]
    fn peak_fallback_when_the_quadratic_fit_fails() {
        let mut saddle = Array2::<f64>::zeros((11, 11));
        for r in 3..8 {
            for c in 3..8 {
                let (x, y) = (c as f64 - 5.0, r as f64 - 5.0);
                saddle[[r, c]] = 50.0 + 3.0 * x * x - 3.0 * y * y;
            }
        }
        saddle[[5, 5]] = 100.0;
        assert_eq!(centroid_quadratic(&saddle, QUADRATIC_FIT_BOXSIZE), None);
        assert_eq!(measure_centre(&saddle), Some((5.0, 5.0, CentroidMethod::Peak)));
        let smooth = astropy_gaussian((21, 21), 10.0, 10.3, 9.6, 2.0, 2.0, 0.0);
        let (x, y, method) = measure_centre(&smooth).expect("a centre");
        assert_eq!(method, CentroidMethod::Quadratic);
        assert!((x - 10.272681150452682).abs() < 1e-9 && (y - 9.635425345767288).abs() < 1e-9, "numpy lstsq gives the same biased fit: ({x}, {y})");
        assert_eq!(measure_centre(&Array2::from_elem((7, 7), f64::NAN)), None);
    }

    #[test]
    fn recentring_without_missing_pixels_is_the_scipy_cubic_shift() {
        let data = astropy_gaussian((15, 15), 10.0, 7.3, 6.8, 1.7, 1.7, 0.0);
        let (shifted, missing) = recentre(&data, 7.3, 6.8);
        let bits = |array: &Array2<f64>| array.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&shifted), bits(&shift(&data, [7.0 - 6.8, 7.0 - 7.3], SplineMode::Constant(f64::NAN))));
        assert!(shifted.iter().any(|v| v.is_nan()) && missing.iter().all(|m| !m));
    }

    #[test]
    fn missing_pixels_stay_local_when_recentring() {
        let clean = astropy_gaussian((31, 31), 10.0, 15.4, 14.7, 1.8, 1.8, 0.0);
        let mut holed = clean.clone();
        holed[[15, 27]] = f64::NAN;
        let (reference, _) = recentre(&clean, 15.4, 14.7);
        let (shifted, missing) = recentre(&holed, 15.4, 14.7);
        let masked: Vec<(usize, usize)> = missing.indexed_iter().filter(|(_, m)| **m).map(|(at, _)| at).collect();
        assert!(!masked.is_empty() && masked.len() <= 16, "{masked:?}");
        assert!(masked.iter().all(|&(r, c)| (r as i64 - 15).abs() <= 2 && (c as i64 - 27).abs() <= 2), "{masked:?}");
        for ((r, c), value) in shifted.indexed_iter() {
            if missing[[r, c]] {
                assert!(value.is_nan());
            } else if reference[[r, c]].is_finite() {
                assert!((value - reference[[r, c]]).abs() < 2e-3, "pixel ({r}, {c}): {value} against {}", reference[[r, c]]);
            }
        }
    }

    #[test]
    fn nan_inside_the_aperture_or_off_image_rejects_only_that_star() {
        let (mut image, stars) = identical_star_field();
        let (hole_x, hole_y) = (stars[0].x.round() as usize + 25, stars[0].y.round() as usize);
        image[[hole_y, hole_x]] = f32::NAN;
        let (core_x, core_y) = (stars[2].x.round() as usize + 5, stars[2].y.round() as usize);
        image[[core_y, core_x]] = f32::NAN;
        let options = EmpiricalPsfOptions::default();
        let result = build_empirical_psf(&image, &stars, None, &options).expect("psf");
        let used: Vec<bool> = result.stars.iter().map(|s| s.used).collect();
        assert_eq!(used, vec![true, true, false, true, true]);
        let reason = result.stars[2].rejected.as_deref().unwrap();
        assert!(reason.contains("aperture"), "{reason}");
        let (clean_image, clean_stars) = identical_star_field();
        let clean = build_empirical_psf(&clean_image, &clean_stars, None, &options).unwrap();
        assert_eq!(result.psf, clean.psf);

        let mut edge = Array2::<f32>::from_elem((120, 150), 5.0);
        add_gaussian(&mut edge, 22.3, 60.6, 5000.0, 1.8);
        add_gaussian(&mut edge, 12.3, 20.6, 5000.0, 1.8);
        add_gaussian(&mut edge, 90.3, 60.6, 5000.0, 1.8);
        let edge_stars = [
            PsfStar { name: "partial".into(), x: 22.3, y: 60.6 },
            PsfStar { name: "clipped".into(), x: 12.3, y: 20.6 },
            PsfStar { name: "inside".into(), x: 90.3, y: 60.6 },
            PsfStar { name: "away".into(), x: -200.0, y: 60.0 },
        ];
        let result = build_empirical_psf(&edge, &edge_stars, None, &options).expect("psf");
        let used: Vec<bool> = result.stars.iter().map(|s| s.used).collect();
        assert_eq!(used, vec![true, false, true, false], "{:?}", result.stars);
        assert!(result.stars[1].rejected.as_deref().unwrap().contains("aperture"));
        assert!(result.stars[3].rejected.as_deref().unwrap().contains("outside the image"));
        assert!(result.psf.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn stars_without_a_usable_cutout_are_rejected_with_reasons() {
        let mut image = Array2::<f32>::from_elem((200, 200), 5.0);
        image.slice_mut(s![100..200, 100..200]).fill(f32::NAN);
        add_gaussian(&mut image, 50.3, 50.6, 5000.0, 1.8);
        let stars = [
            PsfStar { name: "star".into(), x: 50.3, y: 50.6 },
            PsfStar { name: "blank".into(), x: 60.0, y: 150.0 },
            PsfStar { name: "masked".into(), x: 150.0, y: 150.0 },
            PsfStar { name: "gone".into(), x: 500.0, y: 50.0 },
        ];
        let options = EmpiricalPsfOptions::default();
        let result = build_empirical_psf(&image, &stars, None, &options).expect("one usable star");
        let reasons: Vec<Option<&str>> = result.stars.iter().map(|s| s.rejected.as_deref()).collect();
        assert_eq!(reasons[0], None);
        assert!(reasons[1].unwrap().contains("aperture flux"), "{reasons:?}");
        assert!(reasons[2].unwrap().contains("no finite pixel"), "{reasons:?}");
        assert!(reasons[3].unwrap().contains("outside the image"), "{reasons:?}");
        let error = build_empirical_psf(&image, &stars[1..], None, &options).unwrap_err();
        for name in ["blank", "masked", "gone"] {
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn mean_stack_is_kept_as_an_option() {
        let narrow = gaussian_stamp(STAMP, STAMP_CENTRE.0, STAMP_CENTRE.1, 5000.0, 1.8);
        let broad = gaussian_stamp(STAMP, STAMP_CENTRE.0, STAMP_CENTRE.1, 5000.0, 2.4);
        let (image, stars) = field_of(&[narrow.clone(), narrow, broad]);
        let median = EmpiricalPsfOptions::default();
        let mean = EmpiricalPsfOptions { stack: PsfStack::Mean, ..EmpiricalPsfOptions::default() };
        let only_narrow = build_empirical_psf(&image, &stars[..1], None, &median).unwrap().psf;
        let only_broad = build_empirical_psf(&image, &stars[2..], None, &median).unwrap().psf;
        assert_eq!(build_empirical_psf(&image, &stars, None, &median).unwrap().psf, only_narrow);
        let averaged = build_empirical_psf(&image, &stars, None, &mean).unwrap().psf;
        let expected = (&only_narrow * 2.0 + &only_broad) / 3.0;
        let peak = expected.iter().copied().fold(0.0, f64::max);
        let worst = averaged.iter().zip(expected.iter()).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(worst < 1e-5 * peak, "mean stack differs from the average by {worst}");
    }

    #[test]
    fn oversampling_zooms_by_the_integer_factor_and_renormalises() {
        let (image, stars) = identical_star_field();
        let options = EmpiricalPsfOptions { oversampling: Some(2), ..EmpiricalPsfOptions::default() };
        let result = build_empirical_psf(&image, &stars, None, &options).unwrap();
        let oversampled = result.oversampled.as_ref().expect("an oversampled PSF");
        assert_eq!(oversampled.dim(), (102, 102));
        assert_eq!(oversampled, &normalize_psf(&zoom(&result.psf, [2.0, 2.0], SplineMode::Nearest)).unwrap());
        assert!((oversampled.sum() - 1.0).abs() < 1e-12);
        assert!(build_empirical_psf(&image, &stars, None, &EmpiricalPsfOptions::default()).unwrap().oversampled.is_none());
    }

    #[test]
    fn recipe_options_are_validated() {
        let (image, stars) = identical_star_field();
        let check = |options: EmpiricalPsfOptions, needle: &str| {
            let error = build_empirical_psf(&image, &stars, Some(0.04), &options).unwrap_err();
            assert!(error.contains(needle), "{needle}: {error}");
        };
        let base = EmpiricalPsfOptions::default;
        check(EmpiricalPsfOptions { cutout_size: 60, ..base() }, "cutout_size");
        check(EmpiricalPsfOptions { psf_size: 50, ..base() }, "psf_size");
        check(EmpiricalPsfOptions { psf_size: 63, ..base() }, "psf_size");
        check(EmpiricalPsfOptions { oversampling: Some(0), ..base() }, "oversampling");
        check(EmpiricalPsfOptions { normalization_radius: 0.0, ..base() }, "normalization_radius");
        check(EmpiricalPsfOptions { background_border: 0, ..base() }, "background_border");
        check(EmpiricalPsfOptions { background_sigma: -1.0, ..base() }, "background_sigma");
        check(EmpiricalPsfOptions { radial_cleaning: true, cleaning_consecutive_bins: 0, ..base() }, "cleaning_consecutive_bins");
        let none = build_empirical_psf(&image, &[], None, &base()).unwrap_err();
        assert!(none.contains("no star"), "{none}");
    }

    #[test]
    fn stars_come_from_included_regions_named_by_their_text() {
        let region = |shape: RegionShape, text: Option<&str>, include: bool| Region {
            shape,
            props: RegionProperties { text: text.map(String::from), include, ..RegionProperties::default() },
        };
        let regions = [
            region(RegionShape::Box { x: 10.0, y: 20.0, width: 5.0, height: 5.0, angle: 0.0 }, None, true),
            region(RegionShape::Circle { x: 30.5, y: 40.25, r: 3.0 }, Some("bright"), true),
            region(RegionShape::Point { x: 1.0, y: 2.0 }, None, false),
            region(RegionShape::Point { x: 7.0, y: 8.0 }, None, true),
        ];
        assert_eq!(
            stars_from_regions(&regions),
            vec![
                PsfStar { name: "region1".into(), x: 10.0, y: 20.0 },
                PsfStar { name: "bright".into(), x: 30.5, y: 40.25 },
                PsfStar { name: "region3".into(), x: 7.0, y: 8.0 },
            ]
        );
    }

    #[test]
    fn radec_list_accepts_degrees_sexagesimal_and_names() {
        let parsed = parse_radec_list("# ra dec name\n53.1625 -27.7914 alpha\n03:32:39.0, -27:47:29.04\n\n  53.16\t-27.79  gamma ray\n").unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], ("alpha".to_string(), 53.1625, -27.7914));
        assert_eq!(parsed[1].0, "star2");
        assert!((parsed[1].1 - 53.1625).abs() < 1e-9 && (parsed[1].2 + 27.7914).abs() < 1e-9, "{:?}", parsed[1]);
        assert_eq!(parsed[2], ("gamma ray".to_string(), 53.16, -27.79));
        let error = parse_radec_list("53.1\n12.0 abc\n").unwrap_err();
        assert!(error.contains("line 1"), "{error}");
        assert!(parse_radec_list("# nothing\n").unwrap_err().contains("no star"));
    }

    #[test]
    fn empirical_psf_header_carries_the_lightstack_keywords() {
        let psf = Array2::<f64>::from_elem((51, 51), 1.0 / 2601.0);
        let labels = PsfHeaderLabels { filter: Some("f444w"), dataset: Some("jwst"), pixel_scale_arcsec: Some(0.04), stars_used: 7 };
        let header = empirical_psf_header(&psf, &labels, &EmpiricalPsfOptions::default(), None);
        for (key, value) in [
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
            ("CUNIT1", "deg"),
            ("CUNIT2", "deg"),
            ("BUNIT", "normalized"),
            ("FILTER", "F444W"),
            ("DATASET", "JWST"),
            ("APRUNIT", "pixel"),
            ("METHOD", "median"),
        ] {
            assert_eq!(header.get(key), Some(value), "{key}");
            assert_eq!(header.is_string_value(key), Some(true), "{key}");
        }
        let close = |header: &HduHeader, key: &str, want: f64| {
            let got = header.get_f64(key).unwrap_or_else(|| panic!("{key} is missing"));
            assert!((got - want).abs() <= 1e-13 * want.abs(), "{key}: {got} against {want}");
        };
        for (key, value) in [
            ("CRVAL1", 0.0),
            ("CRVAL2", 0.0),
            ("CRPIX1", 26.0),
            ("CRPIX2", 26.0),
            ("CD1_1", -0.04 / 3600.0),
            ("CD1_2", 0.0),
            ("CD2_1", 0.0),
            ("CD2_2", 0.04 / 3600.0),
            ("APRAD", 20.0),
            ("PIXSCALE", 0.04),
        ] {
            close(&header, key, value);
        }
        for (key, value) in [("NSTARS", 7), ("CUTSIZE", 61), ("PSFSIZE", 51)] {
            assert_eq!(header.get_i64(key), Some(value), "{key}");
        }
        assert!(header.get("OVERSAMP").is_none());
        let history: Vec<&str> = header.cards.iter().filter(|(k, _)| k == "HISTORY").map(|(_, v)| v.as_str()).collect();
        let joined = history.join(" ");
        assert!(joined.contains("Empirical PSF constructed from stellar cutouts."), "{history:?}");
        assert!(joined.contains("PSF WCS is local and independent of the science-image WCS."), "{history:?}");

        let oversampled = Array2::<f64>::from_elem((102, 102), 1.0 / 10404.0);
        let mean = EmpiricalPsfOptions {
            stack: PsfStack::Mean,
            normalization_radius: 0.8,
            normalization_radius_unit: RadiusUnit::Arcsec,
            ..EmpiricalPsfOptions::default()
        };
        let header = empirical_psf_header(&oversampled, &labels, &mean, Some(2));
        close(&header, "PIXSCALE", 0.02);
        close(&header, "CRPIX1", 51.5);
        close(&header, "CD2_2", 0.02 / 3600.0);
        assert_eq!(header.get_i64("PSFSIZE"), Some(102));
        assert_eq!(header.get_i64("OVERSAMP"), Some(2));
        assert_eq!(header.get_f64("APRAD"), Some(0.8));
        assert_eq!(header.get("APRUNIT"), Some("arcsec"));
        assert_eq!(header.get("METHOD"), Some("mean"));
        assert!(header.cards.iter().any(|(k, v)| k == "HISTORY" && v.contains("oversampled")));

        let bare = PsfHeaderLabels { filter: None, dataset: None, pixel_scale_arcsec: None, stars_used: 3 };
        let header = empirical_psf_header(&psf, &bare, &EmpiricalPsfOptions::default(), None);
        for key in ["CTYPE1", "CD1_1", "PIXSCALE", "FILTER", "DATASET"] {
            assert!(header.get(key).is_none(), "{key}");
        }
        assert_eq!(header.get("BUNIT"), Some("normalized"));
        assert_eq!(header.get_i64("NSTARS"), Some(3));
    }
}
