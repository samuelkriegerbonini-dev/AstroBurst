use ndarray::Array2;

use crate::core::imaging::star_mask::{generate_star_mask, StarMaskConfig};
use crate::core::imaging::stats::is_valid_pixel;

#[derive(Debug, Clone)]
pub struct StarRemovalConfig {
    pub detection_sigma: f64,
    pub max_eccentricity: f64,
    pub growth_factor: f64,
    pub softness: f64,
    pub bright_ceiling: f64,
}

impl Default for StarRemovalConfig {
    fn default() -> Self {
        Self {
            detection_sigma: 4.0,
            max_eccentricity: 0.9,
            growth_factor: 3.0,
            softness: 6.0,
            bright_ceiling: 0.95,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StarRemovalResult {
    pub starless: Array2<f32>,
    pub stars: Array2<f32>,
    pub stars_removed: usize,
    pub mask_coverage: f64,
}

fn normalize_to_01(image: &Array2<f32>) -> Array2<f32> {
    let mut dmin = f32::INFINITY;
    let mut dmax = f32::NEG_INFINITY;
    for &v in image.iter() {
        if is_valid_pixel(v) {
            if v < dmin {
                dmin = v;
            }
            if v > dmax {
                dmax = v;
            }
        }
    }
    let range = dmax - dmin;
    if !range.is_finite() || range < 1e-10 {
        return Array2::zeros(image.dim());
    }
    let inv = 1.0 / range;
    image.mapv(|v| {
        if !v.is_finite() {
            0.0
        } else {
            ((v - dmin) * inv).clamp(0.0, 1.0)
        }
    })
}

fn downsample_weighted(pv: &Array2<f32>, pw: &Array2<f32>) -> (Array2<f32>, Array2<f32>) {
    let (rows, cols) = pv.dim();
    let nr = rows.div_ceil(2);
    let nc = cols.div_ceil(2);
    let mut ov = Array2::<f32>::zeros((nr, nc));
    let mut ow = Array2::<f32>::zeros((nr, nc));
    for y in 0..rows {
        for x in 0..cols {
            ov[[y / 2, x / 2]] += pv[[y, x]];
            ow[[y / 2, x / 2]] += pw[[y, x]];
        }
    }
    (ov, ow)
}

fn upsample_bilinear(src: &Array2<f32>, rows: usize, cols: usize) -> Array2<f32> {
    let (sr, sc) = src.dim();
    if sr == 0 || sc == 0 {
        return Array2::zeros((rows, cols));
    }
    Array2::from_shape_fn((rows, cols), |(y, x)| {
        let fy = ((y as f32 + 0.5) * sr as f32 / rows as f32 - 0.5).clamp(0.0, sr as f32 - 1.0);
        let fx = ((x as f32 + 0.5) * sc as f32 / cols as f32 - 0.5).clamp(0.0, sc as f32 - 1.0);
        let y0 = fy.floor() as usize;
        let x0 = fx.floor() as usize;
        let y1 = (y0 + 1).min(sr - 1);
        let x1 = (x0 + 1).min(sc - 1);
        let ty = fy - y0 as f32;
        let tx = fx - x0 as f32;
        let a = src[[y0, x0]] * (1.0 - tx) + src[[y0, x1]] * tx;
        let b = src[[y1, x0]] * (1.0 - tx) + src[[y1, x1]] * tx;
        a * (1.0 - ty) + b * ty
    })
}

fn pushpull_fill(image: &Array2<f32>, validity: &Array2<f32>) -> Array2<f32> {
    let (rows, cols) = image.dim();

    let mut pv = Array2::<f32>::zeros((rows, cols));
    let mut pw = Array2::<f32>::zeros((rows, cols));
    ndarray::Zip::from(&mut pv)
        .and(&mut pw)
        .and(image)
        .and(validity)
        .for_each(|v, w, &img, &val| {
            let weight = if img.is_finite() { val } else { 0.0 };
            *v = if img.is_finite() { img * weight } else { 0.0 };
            *w = weight;
        });

    let mut levels: Vec<(Array2<f32>, Array2<f32>)> = vec![(pv, pw)];
    while levels.last().unwrap().0.dim().0 > 2 && levels.last().unwrap().0.dim().1 > 2 {
        let (nv, nw) = downsample_weighted(&levels.last().unwrap().0, &levels.last().unwrap().1);
        levels.push((nv, nw));
    }

    let deepest = levels.len() - 1;
    let mut resolved: Array2<f32> = {
        let (pv, pw) = &levels[deepest];
        let mut fallback = 0.0f32;
        let mut wsum = 0.0f32;
        for (v, w) in pv.iter().zip(pw.iter()) {
            fallback += v;
            wsum += w;
        }
        let global = if wsum > 1e-12 { fallback / wsum } else { 0.0 };
        Array2::from_shape_fn(pv.dim(), |(y, x)| {
            let w = pw[[y, x]];
            if w > 1e-12 {
                pv[[y, x]] / w
            } else {
                global
            }
        })
    };

    for i in (0..deepest).rev() {
        let (pv, pw) = &levels[i];
        let (lr, lc) = pv.dim();
        let parent = upsample_bilinear(&resolved, lr, lc);
        resolved = Array2::from_shape_fn((lr, lc), |(y, x)| {
            let w = (pw[[y, x]]).min(1.0);
            if w > 1e-12 {
                let own = pv[[y, x]] / pw[[y, x]];
                own * w + parent[[y, x]] * (1.0 - w)
            } else {
                parent[[y, x]]
            }
        });
    }

    resolved
}

pub fn remove_stars(
    image: &Array2<f32>,
    config: &StarRemovalConfig,
) -> Result<StarRemovalResult, String> {
    let normalized = normalize_to_01(image);

    let mask_config = StarMaskConfig {
        growth_factor: config.growth_factor,
        softness: config.softness,
        detection_sigma: config.detection_sigma,
        max_eccentricity: config.max_eccentricity,
        luminance_protect: true,
        luminance_ceiling: config.bright_ceiling,
        ..StarMaskConfig::default()
    };

    let mask_result = generate_star_mask(&normalized, &mask_config)?;
    remove_stars_with_mask(image, &mask_result.mask, mask_result.stars_masked, mask_result.coverage_fraction)
}

pub fn remove_stars_with_mask(
    image: &Array2<f32>,
    mask: &Array2<f32>,
    stars_masked: usize,
    coverage: f64,
) -> Result<StarRemovalResult, String> {
    if mask.dim() != image.dim() {
        return Err(format!(
            "Mask dim {:?} does not match image {:?}",
            mask.dim(),
            image.dim()
        ));
    }

    let validity = mask.mapv(|m| (1.0 - m).clamp(0.0, 1.0));
    let filled = pushpull_fill(image, &validity);

    let mut starless = Array2::<f32>::zeros(image.dim());
    let mut stars = Array2::<f32>::zeros(image.dim());
    ndarray::Zip::from(&mut starless)
        .and(&mut stars)
        .and(image)
        .and(&filled)
        .and(mask)
        .par_for_each(|sl, st, &img, &fill, &m| {
            if !img.is_finite() {
                *sl = img;
                *st = 0.0;
                return;
            }
            let blended = img * (1.0 - m) + fill * m;
            *sl = blended;
            *st = (img - blended).max(0.0);
        });

    Ok(StarRemovalResult {
        starless,
        stars,
        stars_removed: stars_masked,
        mask_coverage: coverage,
    })
}

pub struct StarRemovalRgbResult {
    pub r: StarRemovalResult,
    pub g: StarRemovalResult,
    pub b: StarRemovalResult,
    pub stars_removed: usize,
    pub mask_coverage: f64,
}

pub fn remove_stars_rgb_shared(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    config: &StarRemovalConfig,
) -> Result<StarRemovalRgbResult, String> {
    let dim = r.dim();
    if g.dim() != dim || b.dim() != dim {
        return Err(format!(
            "Channel dimension mismatch: R={:?} G={:?} B={:?}",
            dim,
            g.dim(),
            b.dim()
        ));
    }

    let mut luminance = Array2::<f32>::zeros(dim);
    ndarray::Zip::from(&mut luminance)
        .and(r)
        .and(g)
        .and(b)
        .par_for_each(|o, &rv, &gv, &bv| {
            let rn = if rv.is_finite() { rv } else { 0.0 };
            let gn = if gv.is_finite() { gv } else { 0.0 };
            let bn = if bv.is_finite() { bv } else { 0.0 };
            *o = 0.2126 * rn + 0.7152 * gn + 0.0722 * bn;
        });

    let mask_config = StarMaskConfig {
        growth_factor: config.growth_factor,
        softness: config.softness,
        detection_sigma: config.detection_sigma,
        max_eccentricity: config.max_eccentricity,
        luminance_protect: true,
        luminance_ceiling: config.bright_ceiling,
        ..StarMaskConfig::default()
    };

    let mask_result = generate_star_mask(&normalize_to_01(&luminance), &mask_config)?;
    let stars_removed = mask_result.stars_masked;
    let coverage = mask_result.coverage_fraction;

    let (rr, (rg, rb)) = rayon::join(
        || remove_stars_with_mask(r, &mask_result.mask, stars_removed, coverage),
        || {
            rayon::join(
                || remove_stars_with_mask(g, &mask_result.mask, stars_removed, coverage),
                || remove_stars_with_mask(b, &mask_result.mask, stars_removed, coverage),
            )
        },
    );

    Ok(StarRemovalRgbResult {
        r: rr?,
        g: rg?,
        b: rb?,
        stars_removed,
        mask_coverage: coverage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn star_field(rows: usize, cols: usize) -> Array2<f32> {
        let stars = [(30usize, 30usize), (80, 90), (50, 110)];
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            let bg = 0.1 + 0.05 * (y as f32 / rows as f32);
            let mut v = bg;
            for &(sy, sx) in &stars {
                let dy = y as f32 - sy as f32;
                let dx = x as f32 - sx as f32;
                v += 2.0 * (-(dy * dy + dx * dx) / 8.0).exp();
            }
            v
        })
    }

    #[test]
    fn removes_star_peaks() {
        let img = star_field(128, 160);
        let res = remove_stars(&img, &StarRemovalConfig::default()).unwrap();

        assert!(res.stars_removed >= 3, "detected {} stars", res.stars_removed);
        for &(sy, sx) in &[(30usize, 30usize), (80, 90), (50, 110)] {
            let before = img[[sy, sx]];
            let after = res.starless[[sy, sx]];
            assert!(
                after < before * 0.25,
                "star at ({},{}) not removed: {} -> {}",
                sy,
                sx,
                before,
                after
            );
            assert!(
                after < 0.35,
                "residual at ({},{}) too bright: {}",
                sy,
                sx,
                after
            );
        }
    }

    #[test]
    fn background_untouched_outside_mask() {
        let img = star_field(128, 160);
        let res = remove_stars(&img, &StarRemovalConfig::default()).unwrap();
        let corner = res.starless[[5, 5]];
        assert!(
            (corner - img[[5, 5]]).abs() < 1e-5,
            "background changed: {} -> {}",
            img[[5, 5]],
            corner
        );
    }

    #[test]
    fn starless_plus_stars_reconstructs_original() {
        let img = star_field(128, 160);
        let res = remove_stars(&img, &StarRemovalConfig::default()).unwrap();
        for ((sl, st), &orig) in res.starless.iter().zip(res.stars.iter()).zip(img.iter()) {
            let recon = sl + st;
            assert!(
                (recon - orig).abs() < 0.01,
                "reconstruction off: {} vs {}",
                recon,
                orig
            );
        }
    }

    #[test]
    fn nan_pixels_preserved() {
        let mut img = star_field(64, 64);
        img[[0, 0]] = f32::NAN;
        let res = remove_stars(&img, &StarRemovalConfig::default()).unwrap();
        assert!(res.starless[[0, 0]].is_nan());
        assert_eq!(res.stars[[0, 0]], 0.0);
    }

    #[test]
    fn rgb_shared_mask_consistent() {
        let img = star_field(128, 160);
        let res = remove_stars_rgb_shared(&img, &img, &img, &StarRemovalConfig::default()).unwrap();
        assert_eq!(res.r.starless, res.g.starless);
        assert_eq!(res.g.starless, res.b.starless);
        assert!(res.stars_removed >= 3);
    }

    #[test]
    fn pushpull_fills_hole_from_surroundings() {
        let img = Array2::from_elem((64, 64), 0.2f32);
        let mut validity = Array2::from_elem((64, 64), 1.0f32);
        for y in 28..36 {
            for x in 28..36 {
                validity[[y, x]] = 0.0;
            }
        }
        let filled = pushpull_fill(&img, &validity);
        assert!(
            (filled[[32, 32]] - 0.2).abs() < 1e-3,
            "hole not filled with surround: {}",
            filled[[32, 32]]
        );
    }
}
