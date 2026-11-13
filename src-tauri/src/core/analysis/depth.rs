use std::collections::{HashMap, VecDeque};

use ndarray::{s, Array2};
use serde::Serialize;

use crate::core::analysis::aperture::circular_aperture;
use crate::core::analysis::resolution::psf_noise_equivalent_area_px;
use crate::core::cubeprep::psf_io::GAUSSIAN_SIGMA_TO_FWHM;
use crate::core::imaging::contour::gaussian_blur_nan;
use crate::math::median::exact_median_f64;
use crate::math::sigma_clip::std_clipped_stats;
use crate::types::constants::MAD_TO_SIGMA;

pub const SOURCE_SMOOTH_FWHM_PX: f64 = 3.0;
pub const SOURCE_MIN_PIXELS: usize = 5;
pub const SOURCE_GROW_RADIUS_PX: usize = 8;
pub const PLACEMENT_MARGIN_PX: f64 = 1.5;
pub const MASK_FREE: u8 = 0;
pub const MASK_EXCLUDED: u8 = 1;
pub const MASK_KEEP_OUT: u8 = 2;
pub const MAD_SIGMA_REL_ERR_FACTOR: f64 = 1.166;
const MASK_STATISTICS_MAX_VALUES: usize = 2_000_000;
const CLIP_SIGMA: f64 = 3.0;
const CLIP_MAX_ITERATIONS: usize = 10;
const PLACEMENT_TRIES_PER_APERTURE: usize = 2000;
const UNIT_DRAW_SCALE: f64 = 9_007_199_254_740_992.0;
const NEIGHBOUR_OFFSETS: [(isize, isize); 8] = [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)];

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct EmptyAperture {
    pub x: f64,
    pub y: f64,
    pub sum: f64,
    pub formal_err: Option<f64>,
    pub usable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct DepthStatistics {
    pub n_apertures: usize,
    pub n_used: usize,
    pub median_sum: f64,
    pub sigma_empty: f64,
    pub sigma_clipped: f64,
    pub n_after_clip: usize,
    pub sigma_empty_rel_err: f64,
    pub sigma_formal: Option<f64>,
    pub f_corr: Option<f64>,
    pub n_sigma: f64,
    pub limit: f64,
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn unit(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / UNIT_DRAW_SCALE
    }
}

fn median_and_mad_sigma(values: &[f64]) -> (f64, f64) {
    let median = exact_median_f64(values);
    let deviations: Vec<f64> = values.iter().map(|value| (value - median).abs()).collect();
    (median, MAD_TO_SIGMA * exact_median_f64(&deviations))
}

fn sampled_free_values(values: &Array2<f32>, mask: &Array2<u8>, stride: usize) -> Vec<f64> {
    values
        .iter()
        .zip(mask.iter())
        .filter(|(_, &state)| state == MASK_FREE)
        .step_by(stride)
        .map(|(&value, _)| value as f64)
        .collect()
}

fn smoothed_signal_to_noise(image: &Array2<f32>, sigma: Option<&Array2<f32>>, mask: &Array2<u8>, stride: usize) -> Option<Array2<f32>> {
    let signal_to_noise = match sigma {
        Some(sigma) => Array2::from_shape_fn(image.dim(), |at| if mask[at] == MASK_FREE { image[at] / sigma[at] } else { f32::NAN }),
        None => {
            let (median, scatter) = median_and_mad_sigma(&sampled_free_values(image, mask, stride));
            if !(scatter.is_finite() && scatter > 0.0) {
                return None;
            }
            Array2::from_shape_fn(image.dim(), |at| {
                if mask[at] == MASK_FREE {
                    ((image[at] as f64 - median) / scatter) as f32
                } else {
                    f32::NAN
                }
            })
        }
    };
    Some(gaussian_blur_nan(&signal_to_noise, SOURCE_SMOOTH_FWHM_PX / GAUSSIAN_SIGMA_TO_FWHM))
}

fn source_pixels(image: &Array2<f32>, sigma: Option<&Array2<f32>>, mask: &Array2<u8>, detect_nsigma: f64) -> Array2<bool> {
    let (rows, cols) = image.dim();
    let mut sources = Array2::<bool>::default((rows, cols));
    let n_usable = mask.iter().filter(|&&state| state == MASK_FREE).count();
    let stride = n_usable.div_ceil(MASK_STATISTICS_MAX_VALUES).max(1);
    let Some(smoothed) = smoothed_signal_to_noise(image, sigma, mask, stride) else {
        return sources;
    };
    let background = std_clipped_stats(&sampled_free_values(&smoothed, mask, stride), CLIP_SIGMA, CLIP_MAX_ITERATIONS);
    let threshold = background.median + detect_nsigma * background.std;
    let is_above = |y: usize, x: usize| {
        let value = smoothed[[y, x]] as f64;
        value.is_finite() && value > threshold
    };

    let mut visited = Array2::<bool>::default((rows, cols));
    let mut queue = VecDeque::new();
    let mut group: Vec<(usize, usize)> = Vec::new();
    for y in 0..rows {
        for x in 0..cols {
            if visited[[y, x]] || !is_above(y, x) {
                continue;
            }
            visited[[y, x]] = true;
            queue.push_back((y, x));
            group.clear();
            while let Some((cy, cx)) = queue.pop_front() {
                group.push((cy, cx));
                for (dy, dx) in NEIGHBOUR_OFFSETS {
                    let (Some(ny), Some(nx)) = (cy.checked_add_signed(dy), cx.checked_add_signed(dx)) else {
                        continue;
                    };
                    if ny >= rows || nx >= cols || visited[[ny, nx]] || !is_above(ny, nx) {
                        continue;
                    }
                    visited[[ny, nx]] = true;
                    queue.push_back((ny, nx));
                }
            }
            if group.len() >= SOURCE_MIN_PIXELS {
                for &member in &group {
                    sources[member] = true;
                }
            }
        }
    }
    sources
}

fn disc_half_widths(radius: usize) -> Vec<usize> {
    (0..=radius).map(|dy| ((radius * radius - dy * dy) as f64).sqrt() as usize).collect()
}

fn touches_outside(inside: impl Fn(usize, usize) -> bool, rows: usize, cols: usize, y: usize, x: usize) -> bool {
    (x > 0 && !inside(y, x - 1)) || (x + 1 < cols && !inside(y, x + 1)) || (y > 0 && !inside(y - 1, x)) || (y + 1 < rows && !inside(y + 1, x))
}

fn stamp_disc(mask: &mut Array2<u8>, y: usize, x: usize, half_widths: &[usize], paint: impl Fn(u8) -> u8) {
    let (rows, cols) = mask.dim();
    let radius = half_widths.len() - 1;
    for row in y.saturating_sub(radius)..=(y + radius).min(rows - 1) {
        let half = half_widths[row.abs_diff(y)];
        mask.slice_mut(s![row, x.saturating_sub(half)..=(x + half).min(cols - 1)]).mapv_inplace(&paint);
    }
}

pub fn source_exclusion_mask(
    image: &Array2<f32>,
    sigma: Option<&Array2<f32>>,
    bad: Option<&Array2<u8>>,
    radius_px: f64,
    detect_nsigma: f64,
) -> Array2<u8> {
    let (rows, cols) = image.dim();
    let sigma = sigma.filter(|plane| plane.dim() == image.dim());
    let bad = bad.filter(|plane| plane.dim() == image.dim());
    let mut mask = Array2::from_shape_fn((rows, cols), |at| {
        let usable = image[at].is_finite() && bad.is_none_or(|plane| plane[at] == 0) && sigma.is_none_or(|plane| plane[at].is_finite() && plane[at] > 0.0);
        if usable {
            MASK_FREE
        } else {
            MASK_EXCLUDED
        }
    });

    let sources = source_pixels(image, sigma, &mask, detect_nsigma);
    let growth = disc_half_widths(SOURCE_GROW_RADIUS_PX);
    for y in 0..rows {
        for x in 0..cols {
            if !sources[[y, x]] {
                continue;
            }
            mask[[y, x]] = MASK_EXCLUDED;
            if touches_outside(|ny, nx| sources[[ny, nx]], rows, cols, y, x) {
                stamp_disc(&mut mask, y, x, &growth, |_| MASK_EXCLUDED);
            }
        }
    }

    let keep_out = if radius_px.is_finite() && radius_px > 0.0 {
        ((radius_px + PLACEMENT_MARGIN_PX).ceil() as usize).min(rows.max(cols))
    } else {
        0
    };
    let reach = disc_half_widths(keep_out);
    for y in 0..rows {
        for x in 0..cols {
            if mask[[y, x]] == MASK_EXCLUDED {
                if touches_outside(|ny, nx| mask[[ny, nx]] == MASK_EXCLUDED, rows, cols, y, x) {
                    stamp_disc(&mut mask, y, x, &reach, |state| if state == MASK_FREE { MASK_KEEP_OUT } else { state });
                }
            } else if x < keep_out || x + keep_out >= cols || y < keep_out || y + keep_out >= rows {
                mask[[y, x]] = MASK_KEEP_OUT;
            }
        }
    }
    mask
}

pub fn excluded_fraction(mask: &Array2<u8>) -> f64 {
    if mask.is_empty() {
        return 0.0;
    }
    mask.iter().filter(|&&state| state == MASK_EXCLUDED).count() as f64 / mask.len() as f64
}

pub fn draw_aperture_positions(placement: &Array2<u8>, radius_px: f64, n_requested: usize, seed: u64) -> Vec<(f64, f64)> {
    let (rows, cols) = placement.dim();
    let mut kept: Vec<(f64, f64)> = Vec::new();
    if rows == 0 || cols == 0 || !(radius_px.is_finite() && radius_px > 0.0) {
        return kept;
    }
    let separation = 2.0 * radius_px;
    let mut cells: HashMap<(i64, i64), Vec<(f64, f64)>> = HashMap::new();
    let mut generator = SplitMix64 { state: seed };
    for _ in 0..n_requested.saturating_mul(PLACEMENT_TRIES_PER_APERTURE) {
        if kept.len() == n_requested {
            break;
        }
        let column = (generator.unit() * cols as f64) as usize;
        let row = (generator.unit() * rows as f64) as usize;
        if placement[[row, column]] != MASK_FREE {
            continue;
        }
        let x = column as f64 + generator.unit() - 0.5;
        let y = row as f64 + generator.unit() - 0.5;
        let cell_x = (x / separation).floor() as i64;
        let cell_y = (y / separation).floor() as i64;
        let overlaps = (cell_x.saturating_sub(1)..=cell_x.saturating_add(1)).any(|near_x| {
            (cell_y.saturating_sub(1)..=cell_y.saturating_add(1)).any(|near_y| {
                cells
                    .get(&(near_x, near_y))
                    .is_some_and(|members| members.iter().any(|&(kept_x, kept_y)| (kept_x - x).hypot(kept_y - y) <= separation))
            })
        });
        if !overlaps {
            kept.push((x, y));
            cells.entry((cell_x, cell_y)).or_default().push((x, y));
        }
    }
    kept
}

pub fn empty_aperture_sums(
    image: &Array2<f32>,
    sigma: Option<&Array2<f32>>,
    excluded: Option<&Array2<u8>>,
    positions: &[(f64, f64)],
    radius_px: f64,
    subsamples: u8,
) -> Vec<EmptyAperture> {
    let (rows, cols) = image.dim();
    let sigma = sigma.filter(|plane| plane.dim() == image.dim());
    let excluded = excluded.filter(|plane| plane.dim() == image.dim());
    let radius_is_valid = radius_px.is_finite() && radius_px > 0.0;
    positions
        .iter()
        .map(|&(x, y)| {
            let mut usable = radius_is_valid
                && x.is_finite()
                && y.is_finite()
                && x - radius_px >= -0.5
                && x + radius_px <= cols as f64 - 0.5
                && y - radius_px >= -0.5
                && y + radius_px <= rows as f64 - 0.5;
            let mut sum = 0.0f64;
            let mut variance = 0.0f64;
            let mut every_error_is_valid = sigma.is_some();
            for pixel in circular_aperture(rows, cols, x, y, radius_px, subsamples) {
                let at = (pixel.y, pixel.x);
                let weight = pixel.weight as f64;
                if image[at].is_finite() {
                    sum += weight * image[at] as f64;
                } else {
                    usable = false;
                }
                if let Some(plane) = sigma {
                    let error = plane[at] as f64;
                    if error.is_finite() && error >= 0.0 {
                        variance += (weight * error) * (weight * error);
                    } else {
                        every_error_is_valid = false;
                        usable = false;
                    }
                }
                if excluded.is_some_and(|plane| plane[at] == MASK_EXCLUDED) {
                    usable = false;
                }
            }
            EmptyAperture { x, y, sum, formal_err: every_error_is_valid.then(|| variance.sqrt()), usable }
        })
        .collect()
}

pub fn depth_statistics(apertures: &[EmptyAperture], n_sigma: f64) -> Option<DepthStatistics> {
    let usable: Vec<&EmptyAperture> = apertures.iter().filter(|aperture| aperture.usable).collect();
    if usable.len() < 2 {
        return None;
    }
    let sums: Vec<f64> = usable.iter().map(|aperture| aperture.sum).collect();
    let (median_sum, sigma_empty) = median_and_mad_sigma(&sums);
    let clipped = std_clipped_stats(&sums, CLIP_SIGMA, CLIP_MAX_ITERATIONS);
    let formal_errors: Vec<f64> = usable.iter().filter_map(|aperture| aperture.formal_err).collect();
    let sigma_formal = (!formal_errors.is_empty()).then(|| exact_median_f64(&formal_errors));
    Some(DepthStatistics {
        n_apertures: apertures.len(),
        n_used: usable.len(),
        median_sum,
        sigma_empty,
        sigma_clipped: clipped.std,
        n_after_clip: clipped.count,
        sigma_empty_rel_err: MAD_SIGMA_REL_ERR_FACTOR / (usable.len() as f64).sqrt(),
        sigma_formal,
        f_corr: sigma_formal.filter(|&formal| formal > 0.0).map(|formal| sigma_empty / formal),
        n_sigma,
        limit: n_sigma * sigma_empty,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PixelNoise {
    pub median: f64,
    pub p16: f64,
    pub p84: f64,
    pub n: usize,
}

pub fn unmasked_pixel_noise(sigma: &Array2<f32>, mask: Option<&Array2<u8>>) -> Option<PixelNoise> {
    let mask = mask.filter(|plane| plane.dim() == sigma.dim());
    let usable = || {
        sigma
            .indexed_iter()
            .filter(move |&(at, &value)| value.is_finite() && value > 0.0 && mask.is_none_or(|plane| plane[at] != MASK_EXCLUDED))
            .map(|(_, &value)| value as f64)
    };
    let n = usable().count();
    if n == 0 {
        return None;
    }
    let mut sample: Vec<f64> = usable().step_by(n.div_ceil(MASK_STATISTICS_MAX_VALUES)).collect();
    sample.sort_by(f64::total_cmp);
    let last = (sample.len() - 1) as f64;
    Some(PixelNoise {
        median: exact_median_f64(&sample),
        p16: sample[(0.16 * last).floor() as usize],
        p84: sample[(0.84 * last).floor() as usize],
        n,
    })
}

pub fn formal_point_source_limit(sigma_pix: f64, fwhm_px: f64, n_sigma: f64) -> Option<f64> {
    let is_positive = |value: f64| value.is_finite() && value > 0.0;
    (is_positive(sigma_pix) && is_positive(fwhm_px) && is_positive(n_sigma))
        .then(|| n_sigma * sigma_pix * psf_noise_equivalent_area_px(fwhm_px).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::aperture::total_weight;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::image_source::load_plane;
    use crate::types::image_ref::ImageRef;
    use serde_json::Value;
    use std::path::Path;
    use std::time::Instant;

    const SCIENCE_DATA_ENV: &str = "ASTROBURST_SCIENCE_DATA_DIR";
    const L2_IMAGE: &str = "survey/L2/legacysurvey-0532m280-image-r.fits.fz#hdu=1";
    const L2_INVVAR: &str = "survey/L2/legacysurvey-0532m280-invvar-r.fits.fz#hdu=1";
    const L2_RADIUS_PX: f64 = 3.8167938931297707;
    const PHASED_POSITION: (f64, f64) = (32.3, 31.7);
    const PIXEL_IN_THE_APERTURE: (usize, usize) = (31, 32);
    const SOURCE_CENTRE: (f64, f64) = (100.3, 150.6);

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

    fn grid_positions(x0: f64, y0: f64, step: f64, n: usize) -> Vec<(f64, f64)> {
        (0..n).flat_map(|i| (0..n).map(move |j| (x0 + step * i as f64, y0 + step * j as f64))).collect()
    }

    fn sum_squared_weights(x: f64, y: f64, r: f64, subsamples: u8) -> f64 {
        circular_aperture(64, 64, x, y, r, subsamples).iter().map(|pixel| (pixel.weight as f64).powi(2)).sum()
    }

    fn relative_difference(measured: f64, expected: f64) -> f64 {
        ((measured - expected) / expected).abs()
    }

    fn white_noise_apertures(sigma: Option<&Array2<f32>>, subsamples: u8) -> Vec<EmptyAperture> {
        let image = gaussian_noise(1024, 1024, 2.0, 11);
        empty_aperture_sums(&image, sigma, None, &grid_positions(4.3, 3.7, 8.0, 127), 3.0, subsamples)
    }

    fn constant_image_and_error() -> (Array2<f32>, Array2<f32>) {
        (Array2::from_elem((64, 64), 2.0f32), Array2::from_elem((64, 64), 0.5f32))
    }

    fn phased_aperture(image: &Array2<f32>, sigma: Option<&Array2<f32>>, excluded: Option<&Array2<u8>>) -> EmptyAperture {
        empty_aperture_sums(image, sigma, excluded, &[PHASED_POSITION], 3.0, 5)[0]
    }

    fn listed(sum: f64, formal_err: Option<f64>, usable: bool) -> EmptyAperture {
        EmptyAperture { x: 0.0, y: 0.0, sum, formal_err, usable }
    }

    fn known_list() -> Vec<EmptyAperture> {
        (-3..=3).map(|sum| listed(sum as f64, Some(1.0), true)).collect()
    }

    fn assert_known_statistics(stats: &DepthStatistics, n_apertures: usize) {
        let close = |measured: f64, expected: f64| (measured - expected).abs() <= 1e-12 * expected.abs();
        assert_eq!(stats.n_apertures, n_apertures);
        assert_eq!(stats.n_used, 7);
        assert_eq!(stats.median_sum, 0.0);
        assert!(close(stats.sigma_empty, 2.9652), "{stats:?}");
        assert!(close(stats.sigma_clipped, 2.0), "{stats:?}");
        assert_eq!(stats.n_after_clip, 7);
        assert!(close(stats.sigma_formal.unwrap(), 1.0), "{stats:?}");
        assert!(close(stats.f_corr.unwrap(), 2.9652), "{stats:?}");
        assert!(close(stats.limit, 14.826), "{stats:?}");
        assert!(close(stats.sigma_empty_rel_err, 0.4407065755287589), "{stats:?}");
        assert!(close(stats.sigma_empty_rel_err, 1.166 / 7f64.sqrt()), "{stats:?}");
        assert_eq!(stats.n_sigma, 5.0);
    }

    fn noisy_source_field() -> Array2<f32> {
        let noise = gaussian_noise(256, 256, 1.0, 5);
        Array2::from_shape_fn((256, 256), |(y, x)| {
            let d2 = (x as f64 - SOURCE_CENTRE.0).powi(2) + (y as f64 - SOURCE_CENTRE.1).powi(2);
            noise[[y, x]] + (30.0 * (-d2 / (2.0 * 2.0 * 2.0)).exp()) as f32
        })
    }

    fn the_source_core_is_excluded(mask: &Array2<u8>) -> bool {
        mask.indexed_iter()
            .filter(|((y, x), _)| (*x as f64 - SOURCE_CENTRE.0).hypot(*y as f64 - SOURCE_CENTRE.1) <= 8.0)
            .all(|(_, &state)| state == MASK_EXCLUDED)
    }

    fn differing_fraction(a: &Array2<u8>, b: &Array2<u8>) -> f64 {
        a.iter().zip(b.iter()).filter(|(left, right)| left != right).count() as f64 / a.len() as f64
    }

    fn count_of(mask: &Array2<u8>, state: u8) -> usize {
        mask.iter().filter(|&&value| value == state).count()
    }

    #[test]
    fn the_weights_of_a_3_px_aperture_match_the_planning_mirror() {
        let fine = circular_aperture(64, 64, 32.3, 31.7, 3.0, 5);
        assert_eq!(fine.len(), 39);
        assert!((total_weight(&fine) - 28.64).abs() < 1e-5, "{}", total_weight(&fine));
        assert!((sum_squared_weights(32.3, 31.7, 3.0, 5) - 25.5776).abs() < 1e-5);

        let whole = circular_aperture(64, 64, 32.3, 31.7, 3.0, 1);
        assert_eq!(whole.len(), 30);
        assert!(whole.iter().all(|pixel| pixel.weight == 1.0));

        let wide = circular_aperture(64, 64, 32.3, 31.7, 6.0, 5);
        assert_eq!(wide.len(), 133);
        assert!((total_weight(&wide) - 113.12).abs() < 1e-5, "{}", total_weight(&wide));
        assert!((sum_squared_weights(32.3, 31.7, 6.0, 5) - 107.184).abs() < 1e-5);
    }

    #[test]
    fn white_noise_scatter_is_sigma_times_the_root_of_the_summed_squared_weights() {
        let apertures = white_noise_apertures(None, 5);
        assert_eq!(apertures.len(), 16_129);
        assert!(apertures.iter().all(|aperture| aperture.usable));
        let stats = depth_statistics(&apertures, 5.0).unwrap();
        let expected = 2.0 * 25.5776f64.sqrt();
        assert!(relative_difference(stats.sigma_empty, expected) < 0.03, "{stats:?}");
        assert!(relative_difference(stats.sigma_clipped, expected) < 0.03, "{stats:?}");
        assert_eq!(stats.sigma_formal, None);
        assert_eq!(stats.f_corr, None);
        assert!((stats.sigma_empty_rel_err - 1.166 / 127.0).abs() < 1e-12, "{stats:?}");
    }

    #[test]
    fn with_whole_pixel_weights_the_five_sigma_limit_is_five_sigma_root_n_pix() {
        let stats = depth_statistics(&white_noise_apertures(None, 1), 5.0).unwrap();
        assert!(relative_difference(stats.limit, 5.0 * 2.0 * 30f64.sqrt()) < 0.03, "{stats:?}");
    }

    #[test]
    fn a_constant_error_map_on_white_noise_gives_f_corr_one() {
        let sigma = Array2::from_elem((1024, 1024), 2.0f32);
        let apertures = white_noise_apertures(Some(&sigma), 5);
        let expected = 2.0 * 25.5776f64.sqrt();
        assert!(apertures.iter().all(|aperture| relative_difference(aperture.formal_err.unwrap(), expected) < 1e-6));
        let stats = depth_statistics(&apertures, 5.0).unwrap();
        assert!(relative_difference(stats.sigma_formal.unwrap(), expected) < 1e-6, "{stats:?}");
        assert!((stats.f_corr.unwrap() - 1.0).abs() < 0.03, "{stats:?}");
    }

    #[test]
    fn boxcar_correlated_noise_raises_f_corr_to_the_closed_form() {
        let white = gaussian_noise(1026, 1026, 3.0, 5);
        let image = Array2::from_shape_fn((1024, 1024), |(y, x)| {
            (white.slice(s![y..y + 3, x..x + 3]).iter().map(|&value| value as f64).sum::<f64>() / 9.0) as f32
        });
        let sigma = Array2::from_elem((1024, 1024), 1.0f32);
        let apertures = empty_aperture_sums(&image, Some(&sigma), None, &grid_positions(8.3, 7.7, 14.0, 72), 6.0, 5);
        assert_eq!(apertures.len(), 5184);
        assert!(apertures.iter().all(|aperture| aperture.usable));

        let mut stamp = Array2::<f64>::zeros((64, 64));
        for pixel in circular_aperture(64, 64, 32.3, 31.7, 6.0, 5) {
            stamp[[pixel.y, pixel.x]] = pixel.weight as f64;
        }
        let mut boxcar_squares = 0.0f64;
        for y in 1..63 {
            for x in 1..63 {
                boxcar_squares += (stamp.slice(s![y - 1..=y + 1, x - 1..=x + 1]).sum() / 9.0).powi(2);
            }
        }
        let closed_form = (9.0 * boxcar_squares / sum_squared_weights(32.3, 31.7, 6.0, 5)).sqrt();
        assert!((closed_form - 2.8175).abs() < 1e-3, "{closed_form}");

        let stats = depth_statistics(&apertures, 5.0).unwrap();
        assert!(relative_difference(stats.f_corr.unwrap(), closed_form) < 0.05, "{stats:?} against {closed_form}");
    }

    #[test]
    fn a_constant_image_sums_to_value_times_the_aperture_weight() {
        let (image, sigma) = constant_image_and_error();
        let aperture = phased_aperture(&image, Some(&sigma), None);
        assert!((aperture.sum - 57.28).abs() < 1e-5, "{aperture:?}");
        assert!((aperture.formal_err.unwrap() - 2.528715).abs() < 1e-5, "{aperture:?}");
        assert!((aperture.formal_err.unwrap() - 0.5 * 25.5776f64.sqrt()).abs() < 1e-5, "{aperture:?}");
        assert!(aperture.usable);
        assert_eq!((aperture.x, aperture.y), PHASED_POSITION);
    }

    #[test]
    fn an_aperture_that_touches_missing_data_is_not_usable() {
        let (image, sigma) = constant_image_and_error();

        let mut holed = image.clone();
        holed[PIXEL_IN_THE_APERTURE] = f32::NAN;
        assert!(!phased_aperture(&holed, Some(&sigma), None).usable);

        let mut unknown_error = sigma.clone();
        unknown_error[PIXEL_IN_THE_APERTURE] = f32::NAN;
        let aperture = phased_aperture(&image, Some(&unknown_error), None);
        assert!(!aperture.usable);
        assert_eq!(aperture.formal_err, None);

        let mut negative_error = sigma.clone();
        negative_error[PIXEL_IN_THE_APERTURE] = -0.5;
        assert!(!phased_aperture(&image, Some(&negative_error), None).usable);

        let mut excluded = Array2::<u8>::zeros((64, 64));
        excluded[PIXEL_IN_THE_APERTURE] = MASK_EXCLUDED;
        assert!(!phased_aperture(&image, Some(&sigma), Some(&excluded)).usable);

        for position in [(1.0, 32.0), (f64::NAN, 3.0)] {
            assert!(!empty_aperture_sums(&image, Some(&sigma), None, &[position], 3.0, 5)[0].usable, "{position:?}");
        }
        for radius in [0.0, f64::NAN] {
            assert!(!empty_aperture_sums(&image, Some(&sigma), None, &[PHASED_POSITION], radius, 5)[0].usable, "{radius}");
        }
        assert!(empty_aperture_sums(&image, Some(&sigma), None, &[], 3.0, 5).is_empty());
    }

    #[test]
    fn keep_out_pixels_and_zero_errors_do_not_spoil_an_aperture() {
        let (image, sigma) = constant_image_and_error();

        let mut placement = Array2::<u8>::zeros((64, 64));
        placement[PIXEL_IN_THE_APERTURE] = MASK_KEEP_OUT;
        let aperture = phased_aperture(&image, Some(&sigma), Some(&placement));
        assert!(aperture.usable);
        assert!((aperture.sum - 57.28).abs() < 1e-5, "{aperture:?}");

        let mut exact_pixel = sigma.clone();
        exact_pixel[PIXEL_IN_THE_APERTURE] = 0.0;
        let aperture = phased_aperture(&image, Some(&exact_pixel), None);
        assert!(aperture.usable);
        assert!(aperture.formal_err.unwrap() < 2.528715 - 1e-3, "{aperture:?}");
    }

    #[test]
    fn depth_statistics_of_a_known_list() {
        assert_known_statistics(&depth_statistics(&known_list(), 5.0).unwrap(), 7);
    }

    #[test]
    fn depth_statistics_uses_only_usable_apertures_and_needs_two() {
        let mut apertures = known_list();
        apertures.extend([listed(1e9, Some(1.0), false); 3]);
        assert_known_statistics(&depth_statistics(&apertures, 5.0).unwrap(), 10);

        assert_eq!(depth_statistics(&[listed(1.0, Some(1.0), true)], 5.0), None);
        assert_eq!(depth_statistics(&[listed(1.0, Some(1.0), true), listed(2.0, Some(1.0), false)], 5.0), None);
        assert_eq!(depth_statistics(&[], 5.0), None);

        let without_errors: Vec<EmptyAperture> = (-3..=3).map(|sum| listed(sum as f64, None, true)).collect();
        let stats = depth_statistics(&without_errors, 5.0).unwrap();
        assert_eq!(stats.sigma_formal, None);
        assert_eq!(stats.f_corr, None);
        assert!((stats.sigma_empty - 2.9652).abs() < 1e-12, "{stats:?}");
    }

    #[test]
    fn splitmix_positions_are_reproducible() {
        let placement = Array2::<u8>::zeros((100, 100));
        let first = draw_aperture_positions(&placement, 3.0, 20, 1);
        assert_eq!(first.len(), 20);
        assert!((first[0].0 - 56.471002753586796).abs() < 1e-12, "{:?}", first[0]);
        assert!((first[0].1 - 73.94435921705577).abs() < 1e-12, "{:?}", first[0]);
        assert_eq!(draw_aperture_positions(&placement, 3.0, 20, 1), first);
        assert_ne!(draw_aperture_positions(&placement, 3.0, 20, 2)[0], first[0]);
    }

    #[test]
    fn positions_stay_on_free_pixels_and_do_not_overlap() {
        let placement = Array2::from_shape_fn((512, 512), |(y, x)| {
            if x >= 256 {
                MASK_EXCLUDED
            } else if x < 6 || y < 6 || y >= 506 {
                MASK_KEEP_OUT
            } else {
                MASK_FREE
            }
        });
        let positions = draw_aperture_positions(&placement, 4.0, 150, 7);
        assert_eq!(positions.len(), 150);
        for &(x, y) in &positions {
            assert_eq!(placement[[y.round() as usize, x.round() as usize]], MASK_FREE, "({x}, {y})");
        }
        for (i, a) in positions.iter().enumerate() {
            for b in &positions[i + 1..] {
                assert!((a.0 - b.0).hypot(a.1 - b.1) > 8.0, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn a_small_free_area_gives_fewer_positions_and_none_when_nothing_is_free() {
        let island = Array2::from_shape_fn((64, 64), |(y, x)| if (20..44).contains(&y) && (20..44).contains(&x) { MASK_FREE } else { MASK_EXCLUDED });
        let positions = draw_aperture_positions(&island, 4.0, 500, 1);
        assert!((4..=16).contains(&positions.len()), "{}", positions.len());

        assert!(draw_aperture_positions(&Array2::from_elem((64, 64), MASK_EXCLUDED), 4.0, 500, 1).is_empty());
        assert!(draw_aperture_positions(&Array2::<u8>::zeros((0, 0)), 4.0, 500, 1).is_empty());
        assert!(draw_aperture_positions(&Array2::<u8>::zeros((64, 64)), 4.0, 0, 1).is_empty());
        for radius in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(draw_aperture_positions(&Array2::<u8>::zeros((64, 64)), radius, 10, 1).is_empty(), "{radius}");
        }
    }

    #[test]
    fn a_source_and_its_surroundings_are_excluded() {
        let mask = source_exclusion_mask(&noisy_source_field(), None, None, 3.8, 3.0);
        assert!(the_source_core_is_excluded(&mask));
        assert!(excluded_fraction(&mask) < 0.08, "{}", excluded_fraction(&mask));
        assert_eq!(excluded_fraction(&Array2::<u8>::zeros((0, 0))), 0.0);
    }

    #[test]
    fn white_noise_keeps_most_of_the_sky_and_the_threshold_matters() {
        for seed in [1, 2, 3] {
            let image = gaussian_noise(512, 512, 1.0, seed);
            let strict = excluded_fraction(&source_exclusion_mask(&image, None, None, 3.8, 3.0));
            let loose = excluded_fraction(&source_exclusion_mask(&image, None, None, 3.8, 1.5));
            assert!(strict < 0.06, "seed {seed}: {strict}");
            assert!(loose > 0.5, "seed {seed}: {loose}");
        }
    }

    #[test]
    fn bad_and_non_finite_pixels_are_excluded_and_the_border_is_kept_out() {
        let mut image = gaussian_noise(256, 256, 1.0, 9);
        image.slice_mut(s![40..50, 60..70]).fill(f32::NAN);
        let mut bad = Array2::<u8>::zeros((256, 256));
        bad.slice_mut(s![200..210, 20..30]).fill(4);
        let mask = source_exclusion_mask(&image, None, Some(&bad), 3.8, 3.0);
        assert!(mask.slice(s![40..50, 60..70]).iter().all(|&state| state == MASK_EXCLUDED));
        assert!(mask.slice(s![200..210, 20..30]).iter().all(|&state| state == MASK_EXCLUDED));
        assert_ne!(mask[[45, 74]], MASK_FREE);
        assert_ne!(mask[[5, 128]], MASK_FREE);
        assert_ne!(mask[[128, 250]], MASK_FREE);
        assert!(count_of(&mask, MASK_FREE) > 256 * 256 / 2);

        let blank = source_exclusion_mask(&Array2::from_elem((32, 32), f32::NAN), None, None, 3.8, 3.0);
        assert!(blank.iter().all(|&state| state == MASK_EXCLUDED));

        let clean = gaussian_noise(256, 256, 1.0, 9);
        let mut sigma = Array2::from_elem((256, 256), 1.0f32);
        sigma[[100, 100]] = f32::NAN;
        sigma[[120, 100]] = 0.0;
        sigma[[140, 100]] = -1.0;
        let mask = source_exclusion_mask(&clean, Some(&sigma), None, 3.8, 3.0);
        for row in [100, 120, 140] {
            assert_eq!(mask[[row, 100]], MASK_EXCLUDED, "row {row}");
        }
        assert!(count_of(&mask, MASK_FREE) > 256 * 256 / 2);
    }

    #[test]
    fn the_mask_does_not_depend_on_the_flux_unit_or_on_a_pedestal() {
        let field = noisy_source_field();
        let in_cgs = source_exclusion_mask(&field.mapv(|value| value * 1e-17), Some(&Array2::from_elem((256, 256), 1e-17f32)), None, 3.8, 3.0);
        let in_sigmas = source_exclusion_mask(&field, Some(&Array2::from_elem((256, 256), 1.0f32)), None, 3.8, 3.0);
        assert!(differing_fraction(&in_cgs, &in_sigmas) < 0.01, "{}", differing_fraction(&in_cgs, &in_sigmas));
        assert!(the_source_core_is_excluded(&in_cgs) && the_source_core_is_excluded(&in_sigmas));

        let plain = source_exclusion_mask(&field, None, None, 3.8, 3.0);
        let raised = source_exclusion_mask(&field.mapv(|value| value + 1000.0), None, None, 3.8, 3.0);
        assert!(differing_fraction(&plain, &raised) < 0.01, "{}", differing_fraction(&plain, &raised));
        assert!(the_source_core_is_excluded(&plain) && the_source_core_is_excluded(&raised));
    }

    #[test]
    fn a_radius_that_is_not_positive_leaves_no_keep_out_zone() {
        let image = gaussian_noise(256, 256, 1.0, 9);
        for radius in [0.0, f64::NAN] {
            let mask = source_exclusion_mask(&image, None, None, radius, 3.0);
            assert_eq!(count_of(&mask, MASK_KEEP_OUT), 0, "radius {radius}");
            assert!(count_of(&mask, MASK_FREE) > 256 * 256 / 2, "radius {radius}");
        }
        assert!(count_of(&source_exclusion_mask(&image, None, None, 3.8, 3.0), MASK_KEEP_OUT) > 0);
    }

    fn science_plane(relative: &str) -> Array2<f32> {
        let key = env_dir(SCIENCE_DATA_ENV).join(relative).to_str().unwrap().to_string();
        load_plane(&ImageRef::parse(&key)).unwrap().arr
    }

    fn oracle_fixture() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("survey_qa").join("depth_legacy_l2_r_iteration0.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        serde_json::from_str(&text).unwrap()
    }

    fn numbers(value: &Value) -> Vec<f64> {
        value.as_array().unwrap().iter().map(|number| number.as_f64().unwrap()).collect()
    }

    fn magnitude(zero_point: f64, flux: f64) -> f64 {
        zero_point - 2.5 * flux.log10()
    }

    struct MaskedDepth {
        n_positions: usize,
        excluded_fraction: f64,
        stats: DepthStatistics,
    }

    fn depth_with_the_app_mask(image: &Array2<f32>, sigma: &Array2<f32>, bad: &Array2<u8>, radius_px: f64, n_requested: usize) -> MaskedDepth {
        let mask = source_exclusion_mask(image, Some(sigma), Some(bad), radius_px, 3.0);
        let positions = draw_aperture_positions(&mask, radius_px, n_requested, 1);
        let apertures = empty_aperture_sums(image, Some(sigma), Some(&mask), &positions, radius_px, 5);
        MaskedDepth { n_positions: positions.len(), excluded_fraction: excluded_fraction(&mask), stats: depth_statistics(&apertures, 5.0).unwrap() }
    }

    fn print_depth(label: &str, zero_point: f64, depth: &MaskedDepth) -> f64 {
        let stats = &depth.stats;
        let from_mad = magnitude(zero_point, stats.limit);
        println!(
            "{label}: positions {} used {} excluded_fraction {:.6} sigma_empty {:.9e} sigma_clipped {:.9e} n_after_clip {} sigma_formal {:?} f_corr {:?} limit {:.9e} mag(MAD) {:.6} mag(clipped) {:.6}",
            depth.n_positions,
            stats.n_used,
            depth.excluded_fraction,
            stats.sigma_empty,
            stats.sigma_clipped,
            stats.n_after_clip,
            stats.sigma_formal,
            stats.f_corr,
            stats.limit,
            from_mad,
            magnitude(zero_point, stats.n_sigma * stats.sigma_clipped),
        );
        from_mad
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz, survey/L2/legacysurvey-0532m280-invvar-r.fits.fz"]
    fn real_legacy_l2_r_sums_at_the_oracle_positions_match_photutils() {
        let image = science_plane(L2_IMAGE);
        let sigma = science_plane(L2_INVVAR).mapv(|invvar| if invvar > 0.0 { 1.0 / invvar.sqrt() } else { 0.0 });
        let fixture = oracle_fixture();
        let radius = fixture["aperture_radius_px"].as_f64().unwrap();
        assert_eq!(radius, L2_RADIUS_PX);
        let positions: Vec<(f64, f64)> = numbers(&fixture["x"]).into_iter().zip(numbers(&fixture["y"])).collect();
        assert_eq!(positions.len(), 1000);
        let oracle_first_ten = numbers(&fixture["aperture_sum_first_10"]);
        let oracle_total = fixture["sum_of_all_aperture_sums"].as_f64().unwrap();
        let oracle_mad_sigma = fixture["sigma_from_mad"].as_f64().unwrap();
        let oracle_clipped = fixture["std_after_clip"].as_f64().unwrap();
        let oracle_kept = fixture["n_after_clip"].as_u64().unwrap() as usize;
        let oracle_formal = fixture["median_formal_error_w2"].as_f64().unwrap();
        assert_eq!((oracle_total, oracle_mad_sigma, oracle_clipped, oracle_kept, oracle_formal), (2.1386647681648356, 0.011941213567643506, 0.011801076462289086, 982, 0.0070943638406866525));

        for (subsamples, first_ten_tolerance) in [(5u8, 1e-3), (25, 2e-4)] {
            let apertures = empty_aperture_sums(&image, Some(&sigma), None, &positions, radius, subsamples);
            let stats = depth_statistics(&apertures, 5.0).unwrap();
            let total: f64 = apertures.iter().map(|aperture| aperture.sum).sum();
            let worst_of_ten = apertures.iter().zip(&oracle_first_ten).map(|(aperture, oracle)| (aperture.sum - oracle).abs()).fold(0.0, f64::max);
            let n_usable = apertures.iter().filter(|aperture| aperture.usable).count();
            println!(
                "subsamples {subsamples}: total {total:.12} ({:+.4} percent) sigma_empty {:.12} ({:+.4} percent) sigma_clipped {:.12} ({:+.4} percent) n_after_clip {} sigma_formal {:.12} ({:+.4} percent) f_corr {:.6} worst of the first ten {worst_of_ten:.3e} usable {n_usable}",
                100.0 * (total / oracle_total - 1.0),
                stats.sigma_empty,
                100.0 * (stats.sigma_empty / oracle_mad_sigma - 1.0),
                stats.sigma_clipped,
                100.0 * (stats.sigma_clipped / oracle_clipped - 1.0),
                stats.n_after_clip,
                stats.sigma_formal.unwrap(),
                100.0 * (stats.sigma_formal.unwrap() / oracle_formal - 1.0),
                stats.f_corr.unwrap(),
            );
            assert!(relative_difference(total, oracle_total) < 0.002);
            assert!(relative_difference(stats.sigma_empty, oracle_mad_sigma) < 0.02);
            assert!(relative_difference(stats.sigma_clipped, oracle_clipped) < 0.02);
            assert!(stats.n_after_clip.abs_diff(oracle_kept) <= 3);
            assert!(relative_difference(stats.sigma_formal.unwrap(), oracle_formal) < 0.01);
            assert!(relative_difference(stats.f_corr.unwrap(), 1.6832) < 0.03);
            assert!(worst_of_ten < first_ten_tolerance);
            assert_eq!(n_usable, 1000);
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz, survey/L2/legacysurvey-0532m280-invvar-r.fits.fz"]
    fn real_legacy_l2_r_depth_with_the_app_mask_is_a_plausible_depth() {
        let started = Instant::now();
        let image = science_plane(L2_IMAGE);
        let invvar = science_plane(L2_INVVAR);
        let sigma = invvar.mapv(|weight| if weight > 0.0 { 1.0 / weight.sqrt() } else { f32::NAN });
        let bad = invvar.mapv(|weight| u8::from(!(weight.is_finite() && weight > 0.0)));
        let depth = depth_with_the_app_mask(&image, &sigma, &bad, L2_RADIUS_PX, 5000);
        let seconds = started.elapsed().as_secs_f64();
        let mag = print_depth("Legacy L2 r", 22.5, &depth);
        println!("seconds {seconds:.1}");
        assert_eq!(depth.n_positions, 5000);
        assert!((25.40..=25.80).contains(&mag));
        assert!((1.30..=1.90).contains(&depth.stats.f_corr.unwrap()));
        assert!((0.30..=0.60).contains(&depth.excluded_fraction));
        assert!(seconds < 120.0);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn1.fits, survey/D1/des-dr2_A_r_extn2.fits, survey/D1/des-dr2_A_r_extn3.fits"]
    fn real_des_d1_r_depth_is_within_0_1_mag_of_the_oracle() {
        let image = science_plane("survey/D1/des-dr2_A_r_extn1.fits");
        let weight = science_plane("survey/D1/des-dr2_A_r_extn3.fits");
        let flags = science_plane("survey/D1/des-dr2_A_r_extn2.fits");
        let sigma = weight.mapv(|value| if value > 0.0 { 1.0 / value.sqrt() } else { f32::NAN });
        let bad = Array2::from_shape_fn(image.dim(), |at| u8::from(flags[at] != 0.0 || !(weight[at] > 0.0)));
        let depth = depth_with_the_app_mask(&image, &sigma, &bad, 3.8022813688212924, 2000);
        let mag = print_depth("DES D1 r", 30.0, &depth);
        println!("oracle: mag 25.250591282240478 f_corr 1.0541617446782123; difference {:+.4} mag", mag - 25.250591282240478);
        assert!((mag - 25.250591282240478).abs() < 0.1);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-*_cut60_A.fits, survey/E1/EUC_MER_MOSAIC-VIS-RMS_TILE102044185-*_cut60_A.fits"]
    fn real_euclid_e1_vis_depth_is_within_0_1_mag_of_the_oracle() {
        let image = science_plane("survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits");
        let rms = science_plane("survey/E1/EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits");
        let sigma = rms.mapv(|value| if value >= 1e15 { f32::NAN } else { value });
        let bad = rms.mapv(|value| u8::from(value >= 1e15));
        let depth = depth_with_the_app_mask(&image, &sigma, &bad, 10.0, 2000);
        let mag = print_depth("Euclid E1 VIS", 24.6, &depth);
        println!("oracle: mag 25.606656166108493 f_corr 1.0738031457273518; difference {:+.4} mag", mag - 25.606656166108493);
        assert!((mag - 25.606656166108493).abs() < 0.1);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_stack_480px.fits, survey/P1/ps1_A_r_wt_480px.fits"]
    fn real_ps1_p1_r_depth_is_within_0_1_mag_of_the_oracle() {
        let image = science_plane("survey/P1/ps1_A_r_stack_480px.fits");
        let variance = science_plane("survey/P1/ps1_A_r_wt_480px.fits");
        let sigma = variance.mapv(|value| if value > 0.0 { value.sqrt() } else { f32::NAN });
        let bad = Array2::from_shape_fn(image.dim(), |at| u8::from(!image[at].is_finite() || !(variance[at] > 0.0)));
        let zero_point = 25.0 + 2.5 * 600f64.log10();
        assert!((zero_point - 31.94537812595911).abs() < 1e-12);
        let depth = depth_with_the_app_mask(&image, &sigma, &bad, 4.0, 2000);
        let mag = print_depth("PS1 P1 r", zero_point, &depth);
        println!("oracle: mag 23.025188873915404 f_corr 1.5059979215639165; difference {:+.4} mag", mag - 23.025188873915404);
        assert!((mag - 23.025188873915404).abs() < 0.1);
    }

    #[test]
    fn the_formal_point_source_limit_is_n_sigma_times_sigma_times_the_root_of_the_psf_area() {
        let wide = formal_point_source_limit(2.0, 4.0, 5.0).unwrap();
        assert!(relative_difference(wide, 60.2153478231402) <= 1e-12, "{wide}");
        assert!(relative_difference(psf_noise_equivalent_area_px(4.0), 36.25888113461755) <= 1e-12);
        let legacy = formal_point_source_limit(0.0010830039, 1.084 / 0.262, 5.0).unwrap();
        assert!(relative_difference(legacy, 0.03372680671804962) <= 1e-9, "{legacy}");
        assert!((magnitude(22.5, legacy) - 26.1800619).abs() < 1e-7, "{}", magnitude(22.5, legacy));
        for (sigma_pix, fwhm_px, n_sigma) in [(0.0, 4.0, 5.0), (f64::NAN, 4.0, 5.0), (2.0, 0.0, 5.0), (2.0, 4.0, -1.0)] {
            assert_eq!(formal_point_source_limit(sigma_pix, fwhm_px, n_sigma), None, "{sigma_pix} {fwhm_px} {n_sigma}");
        }
    }

    fn two_level_sigma() -> Array2<f32> {
        Array2::from_shape_fn((60, 60), |(y, _)| if y < 40 { 1.0 } else { 2.0 })
    }

    fn first_40_rows_marked(state: u8) -> Array2<u8> {
        Array2::from_shape_fn((60, 60), |(y, _)| if y < 40 { state } else { MASK_FREE })
    }

    #[test]
    fn unmasked_pixel_noise_takes_the_median_and_the_16th_and_84th_percentiles() {
        let sigma = two_level_sigma();
        let both_levels = PixelNoise { median: 1.0, p16: 1.0, p84: 2.0, n: 3600 };
        assert_eq!(unmasked_pixel_noise(&sigma, None), Some(both_levels));
        assert_eq!(
            unmasked_pixel_noise(&sigma, Some(&first_40_rows_marked(MASK_EXCLUDED))),
            Some(PixelNoise { median: 2.0, p16: 2.0, p84: 2.0, n: 1200 })
        );
        assert_eq!(unmasked_pixel_noise(&sigma, Some(&first_40_rows_marked(MASK_KEEP_OUT))), Some(both_levels));
        assert_eq!(unmasked_pixel_noise(&sigma, Some(&Array2::from_elem((10, 10), MASK_EXCLUDED))), Some(both_levels));

        let mut spoiled = sigma.clone();
        spoiled[[0, 0]] = f32::NAN;
        spoiled[[1, 0]] = 0.0;
        spoiled[[50, 0]] = -1.0;
        assert_eq!(unmasked_pixel_noise(&spoiled, None), Some(PixelNoise { median: 1.0, p16: 1.0, p84: 2.0, n: 3597 }));
        assert_eq!(unmasked_pixel_noise(&Array2::from_elem((60, 60), f32::NAN), None), None);
    }
}
