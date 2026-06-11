use ndarray::Array2;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct PhotometryConfig {
    pub search_radius: usize,
    pub aperture_radius: Option<f64>,
    pub image_max: Option<f64>,
}

impl Default for PhotometryConfig {
    fn default() -> Self {
        Self {
            search_radius: 8,
            aperture_radius: None,
            image_max: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StarPhotometry {
    pub x: f64,
    pub y: f64,
    pub peak: f64,
    pub net_flux: f64,
    pub mag_inst: f64,
    pub snr: f64,
    pub fwhm: f64,
    pub aperture_radius: f64,
    pub aperture_pixels: u32,
    pub bg_mean: f64,
    pub bg_sigma: f64,
    pub saturated: bool,
}

fn aperture_sum(image: &Array2<f32>, x: f64, y: f64, radius: f64) -> (f64, u32) {
    let (h, w) = image.dim();
    let r2 = radius * radius;
    let y_min = (y - radius).floor().max(0.0) as usize;
    let y_max = ((y + radius).ceil() as usize).min(h.saturating_sub(1));
    let x_min = (x - radius).floor().max(0.0) as usize;
    let x_max = ((x + radius).ceil() as usize).min(w.saturating_sub(1));

    let mut sum = 0.0f64;
    let mut count = 0u32;
    for py in y_min..=y_max {
        for px in x_min..=x_max {
            let dx = px as f64 - x;
            let dy = py as f64 - y;
            if dx * dx + dy * dy <= r2 {
                let v = image[[py, px]];
                if v.is_finite() {
                    sum += v as f64;
                    count += 1;
                }
            }
        }
    }
    (sum, count)
}

fn annulus_stats(image: &Array2<f32>, x: f64, y: f64, inner_r: f64, outer_r: f64) -> (f64, f64) {
    let (h, w) = image.dim();
    let ir2 = inner_r * inner_r;
    let or2 = outer_r * outer_r;
    let y_min = (y - outer_r).floor().max(0.0) as usize;
    let y_max = ((y + outer_r).ceil() as usize).min(h.saturating_sub(1));
    let x_min = (x - outer_r).floor().max(0.0) as usize;
    let x_max = ((x + outer_r).ceil() as usize).min(w.saturating_sub(1));

    let mut vals: Vec<f64> = Vec::new();
    for py in y_min..=y_max {
        for px in x_min..=x_max {
            let dx = px as f64 - x;
            let dy = py as f64 - y;
            let d2 = dx * dx + dy * dy;
            if d2 >= ir2 && d2 <= or2 {
                let v = image[[py, px]];
                if v.is_finite() {
                    vals.push(v as f64);
                }
            }
        }
    }

    if vals.is_empty() {
        return (0.0, 0.0);
    }

    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
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
    devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sigma = devs[devs.len() / 2] * 1.4826;

    (mean, sigma)
}

fn refine_centroid(image: &Array2<f32>, x0: f64, y0: f64, bg: f64, radius: i64) -> (f64, f64) {
    let (h, w) = image.dim();
    let mut x = x0;
    let mut y = y0;
    for _ in 0..2 {
        let cx = x.round() as i64;
        let cy = y.round() as i64;
        let mut sum_x = 0.0f64;
        let mut sum_y = 0.0f64;
        let mut sum_w = 0.0f64;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let ny = cy + dy;
                let nx = cx + dx;
                if ny < 0 || nx < 0 || ny >= h as i64 || nx >= w as i64 {
                    continue;
                }
                let v = image[[ny as usize, nx as usize]];
                if !v.is_finite() {
                    continue;
                }
                let wgt = (v as f64 - bg).max(0.0);
                sum_x += nx as f64 * wgt;
                sum_y += ny as f64 * wgt;
                sum_w += wgt;
            }
        }
        if sum_w <= 0.0 {
            return (x0, y0);
        }
        x = sum_x / sum_w;
        y = sum_y / sum_w;
    }
    (x, y)
}

fn fwhm_truncated_moments(image: &Array2<f32>, x: f64, y: f64, bg: f64) -> f64 {
    let (h, w) = image.dim();
    let ix = x.round().clamp(0.0, (w - 1) as f64) as usize;
    let iy = y.round().clamp(0.0, (h - 1) as f64) as usize;
    let peak = image[[iy, ix]] as f64;
    let net_peak = peak - bg;
    if !net_peak.is_finite() || net_peak <= 0.0 {
        return 0.0;
    }
    let threshold = bg + 0.5 * net_peak;
    let radius = 12i64;

    let mut m2 = 0.0f64;
    let mut sum_w = 0.0f64;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let ny = iy as i64 + dy;
            let nx = ix as i64 + dx;
            if ny < 0 || nx < 0 || ny >= h as i64 || nx >= w as i64 {
                continue;
            }
            let v = image[[ny as usize, nx as usize]] as f64;
            if !v.is_finite() || v < threshold {
                continue;
            }
            let wgt = v - bg;
            let fx = nx as f64 - x;
            let fy = ny as f64 - y;
            m2 += (fx * fx + fy * fy) * wgt;
            sum_w += wgt;
        }
    }
    if sum_w <= 0.0 {
        return 0.0;
    }

    let half_max_truncation = 1.0 - std::f64::consts::LN_2;
    let sigma2 = (m2 / sum_w / 2.0) / half_max_truncation;
    let fwhm = 2.0 * (2.0_f64.ln() * 2.0).sqrt() * sigma2.max(0.0).sqrt();
    fwhm.clamp(0.5, 40.0)
}

pub fn measure_star(
    image: &Array2<f32>,
    click_x: f64,
    click_y: f64,
    config: &PhotometryConfig,
) -> Result<StarPhotometry, String> {
    let (h, w) = image.dim();
    if h < 16 || w < 16 {
        return Err("Image too small for photometry".into());
    }

    let cx = click_x.round().clamp(0.0, (w - 1) as f64) as usize;
    let cy = click_y.round().clamp(0.0, (h - 1) as f64) as usize;

    let r = config.search_radius.max(1);
    let mut best_x = cx;
    let mut best_y = cy;
    let mut best_v = f32::NEG_INFINITY;
    for ny in cy.saturating_sub(r)..=(cy + r).min(h - 1) {
        for nx in cx.saturating_sub(r)..=(cx + r).min(w - 1) {
            let v = image[[ny, nx]];
            if v.is_finite() && v > best_v {
                best_v = v;
                best_y = ny;
                best_x = nx;
            }
        }
    }
    if !best_v.is_finite() {
        return Err("No finite pixels near the clicked position".into());
    }

    let (bg0, _) = annulus_stats(image, best_x as f64, best_y as f64, 8.0, 14.0);
    let (x, y) = refine_centroid(image, best_x as f64, best_y as f64, bg0, 5);

    let fwhm = fwhm_truncated_moments(image, x, y, bg0);
    let fwhm_eff = if fwhm > 0.0 { fwhm } else { 4.0 };

    let r_ap = config
        .aperture_radius
        .unwrap_or(1.5 * fwhm_eff)
        .clamp(2.0, 60.0);

    let (bg_mean, bg_sigma) = annulus_stats(image, x, y, r_ap * 2.0, r_ap * 3.0);

    let (sum, n_ap) = aperture_sum(image, x, y, r_ap);
    if n_ap == 0 {
        return Err("Aperture contains no finite pixels".into());
    }

    let net_flux = sum - bg_mean * n_ap as f64;
    let noise = bg_sigma * (n_ap as f64).sqrt();
    let snr = if noise > 1e-12 {
        net_flux / noise
    } else {
        net_flux.max(0.0)
    };
    let mag_inst = -2.5 * net_flux.max(1e-12).log10();

    let saturated = config
        .image_max
        .map_or(false, |mx| mx > 0.0 && best_v as f64 >= 0.95 * mx);

    Ok(StarPhotometry {
        x,
        y,
        peak: best_v as f64,
        net_flux,
        mag_inst,
        snr,
        fwhm,
        aperture_radius: r_ap,
        aperture_pixels: n_ap,
        bg_mean,
        bg_sigma,
        saturated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gaussian_scene(
        h: usize,
        w: usize,
        star_x: f64,
        star_y: f64,
        amp: f64,
        sigma: f64,
        bg: f64,
    ) -> Array2<f32> {
        Array2::from_shape_fn((h, w), |(y, x)| {
            let dx = x as f64 - star_x;
            let dy = y as f64 - star_y;
            (bg + amp * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()) as f32
        })
    }

    #[test]
    fn test_measure_recovers_centroid_and_flux() {
        let star_x = 32.4;
        let star_y = 30.7;
        let amp = 1000.0;
        let sigma = 2.0;
        let bg = 100.0;
        let img = gaussian_scene(64, 64, star_x, star_y, amp, sigma, bg);

        let result = measure_star(&img, 31.0, 32.0, &PhotometryConfig::default()).unwrap();

        assert!((result.x - star_x).abs() < 0.1, "x={} vs {}", result.x, star_x);
        assert!((result.y - star_y).abs() < 0.1, "y={} vs {}", result.y, star_y);

        let true_flux = 2.0 * std::f64::consts::PI * amp * sigma * sigma;
        let rel_err = (result.net_flux - true_flux).abs() / true_flux;
        assert!(rel_err < 0.03, "flux={} vs {} ({:.1}%)", result.net_flux, true_flux, rel_err * 100.0);

        let true_fwhm = 2.3548 * sigma;
        assert!(
            (result.fwhm - true_fwhm).abs() / true_fwhm < 0.15,
            "fwhm={} vs {}",
            result.fwhm,
            true_fwhm
        );

        assert!(result.mag_inst.is_finite());
        assert!(!result.saturated);
    }

    #[test]
    fn test_snr_with_noisy_background() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 5000.0, 2.0, 100.0);
        for ((y, x), v) in img.indexed_iter_mut() {
            let n = (((y * 7 + x * 13) % 5) as f32) - 2.0;
            *v += n;
        }
        let result = measure_star(&img, 32.0, 32.0, &PhotometryConfig::default()).unwrap();
        assert!(result.bg_sigma > 0.5, "bg_sigma={}", result.bg_sigma);
        assert!(result.snr > 100.0, "snr={}", result.snr);
    }

    #[test]
    fn test_saturation_flag() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 900.0, 2.0, 100.0);
        let cfg = PhotometryConfig {
            image_max: Some(1000.0),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert!(result.saturated);

        let cfg_high = PhotometryConfig {
            image_max: Some(100000.0),
            ..PhotometryConfig::default()
        };
        let result2 = measure_star(&img, 32.0, 32.0, &cfg_high).unwrap();
        assert!(!result2.saturated);
    }

    #[test]
    fn test_manual_aperture_radius() {
        let img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 50.0);
        let cfg = PhotometryConfig {
            aperture_radius: Some(10.0),
            ..PhotometryConfig::default()
        };
        let result = measure_star(&img, 32.0, 32.0, &cfg).unwrap();
        assert!((result.aperture_radius - 10.0).abs() < 1e-9);
        assert!(result.aperture_pixels > 300);
    }

    #[test]
    fn test_rejects_all_nan_region() {
        let mut img = gaussian_scene(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        for y in 0..20 {
            for x in 0..20 {
                img[[y, x]] = f32::NAN;
            }
        }
        assert!(measure_star(&img, 5.0, 5.0, &PhotometryConfig::default()).is_err());
    }
}
