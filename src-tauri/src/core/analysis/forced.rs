use ndarray::Array2;
use serde::Serialize;

use crate::core::analysis::aperture::{circular_aperture, total_weight, AperturePixel};
use crate::core::analysis::photometry::{area_factor, GrowthPoint, MaskedImage, MAX_APERTURE_RADIUS, MIN_APERTURE_RADIUS};
use crate::core::metadata::photcal::AB_MAG_ZERO_POINT;
use crate::math::median::{exact_median_f64, f64_cmp};
use crate::types::constants::MAD_TO_SIGMA;

pub const FORCED_SUBSAMPLES: u8 = 5;
pub const FORCED_DEFAULT_LIMIT_SIGMA: f64 = 3.0;
pub const FORCED_DEFAULT_REFERENCE_RADIUS_ARCSEC: f64 = 3.5;
pub const FORCED_MIN_CORRECTION_STARS: usize = 3;
pub const FORCED_STAR_MIN_SNR: f64 = 50.0;
pub const FORCED_AXIS_SCALE_TOLERANCE: f64 = 0.01;
pub const FORCED_RADIUS_MATCH_TOLERANCE_PX: f64 = 1e-6;
pub const FORCED_LOCUS_TOLERANCE: f64 = 0.5;
pub const FORCED_LOCUS_MIN_WIDTH: f64 = 0.01;
const ARCSEC_PER_DEGREE: f64 = 3600.0;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PixelScale {
    pub area_scale_arcsec: f64,
    pub x_arcsec: f64,
    pub y_arcsec: f64,
}

pub fn pixel_scale_from_cd(cd: [[f64; 2]; 2]) -> Option<PixelScale> {
    let determinant = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
    if !determinant.is_finite() || determinant == 0.0 {
        return None;
    }
    Some(PixelScale {
        area_scale_arcsec: ARCSEC_PER_DEGREE * determinant.abs().sqrt(),
        x_arcsec: ARCSEC_PER_DEGREE * cd[0][0].hypot(cd[1][0]),
        y_arcsec: ARCSEC_PER_DEGREE * cd[0][1].hypot(cd[1][1]),
    })
}

impl PixelScale {
    pub fn anisotropy(&self) -> f64 {
        (self.x_arcsec - self.y_arcsec).abs() / self.x_arcsec.max(self.y_arcsec)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApertureCoverage {
    pub on_image_weight: f64,
    pub full_weight: f64,
}

fn circle_fits_axis(len: usize, centre: f64, radius_px: f64) -> bool {
    centre - radius_px >= -0.5 && centre + radius_px <= len as f64 - 0.5
}

pub fn aperture_coverage(rows: usize, cols: usize, x: f64, y: f64, radius_px: f64, subsamples: u8) -> ApertureCoverage {
    let on_image_weight = total_weight(&circular_aperture(rows, cols, x, y, radius_px, subsamples));
    let inside = circle_fits_axis(cols, x, radius_px) && circle_fits_axis(rows, y, radius_px);
    let finite = x.is_finite() && y.is_finite() && radius_px.is_finite();
    if inside || !finite {
        return ApertureCoverage { on_image_weight, full_weight: on_image_weight };
    }
    let reach = radius_px.ceil();
    let side = (2.0 * reach + 5.0) as usize;
    let margin = reach + 2.0;
    let canvas = circular_aperture(side, side, x - x.floor() + margin, y - y.floor() + margin, radius_px, subsamples);
    ApertureCoverage { on_image_weight, full_weight: total_weight(&canvas) }
}

pub fn masked_fraction(used_weight: f64, coverage: &ApertureCoverage) -> f64 {
    if used_weight >= coverage.full_weight {
        return 0.0;
    }
    (1.0 - used_weight / coverage.full_weight).clamp(0.0, 1.0)
}

fn area_weight(area: Option<&Array2<f32>>, pixel: &AperturePixel) -> f64 {
    pixel.weight as f64 * area.map_or(1.0, |map| area_factor(map[[pixel.y, pixel.x]]))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlainAperture {
    pub x: f64,
    pub y: f64,
    pub radius_px: f64,
    pub flux: f64,
    pub flux_err: Option<f64>,
    pub used_area: f64,
    pub n_used: u32,
    pub n_masked: u32,
    pub n_nonfinite: u32,
    pub masked_fraction: f64,
    pub peak: f64,
    pub area_applied: bool,
}

pub fn measure_plain_aperture(
    source: &MaskedImage,
    err: Option<&Array2<f32>>,
    x: f64,
    y: f64,
    radius_px: f64,
    subsamples: u8,
) -> Result<PlainAperture, String> {
    let image = source.image();
    let (h, w) = image.dim();
    let on_image = x >= -0.5 && x < w as f64 - 0.5 && y >= -0.5 && y < h as f64 - 0.5;
    if !on_image {
        return Err(format!("position ({x}, {y}) lies outside the {w} x {h} image"));
    }
    if !(MIN_APERTURE_RADIUS..=MAX_APERTURE_RADIUS).contains(&radius_px) {
        return Err(format!(
            "aperture radius {radius_px} px is outside the {MIN_APERTURE_RADIUS} to {MAX_APERTURE_RADIUS} px limits"
        ));
    }
    let excluded = source.excluded();
    let area = source.area();
    let err = err.filter(|e| e.dim() == image.dim());

    let mut flux = 0.0;
    let mut variance = 0.0;
    let mut used_area = 0.0;
    let mut peak = f64::NEG_INFINITY;
    let mut n_used = 0u32;
    let mut n_masked = 0u32;
    let mut n_nonfinite = 0u32;
    for pixel in circular_aperture(h, w, x, y, radius_px, subsamples) {
        let at = (pixel.y, pixel.x);
        if excluded.is_some_and(|mask| mask[at] != 0) {
            n_masked += 1;
            continue;
        }
        let value = image[at] as f64;
        let sigma = err.map(|e| e[at] as f64);
        if !value.is_finite() || sigma.is_some_and(|s| !(s.is_finite() && s > 0.0)) {
            n_nonfinite += 1;
            continue;
        }
        let weight = area_weight(area, &pixel);
        flux += weight * value;
        if let Some(sigma) = sigma {
            variance += (weight * sigma).powi(2);
        }
        used_area += pixel.weight as f64;
        peak = peak.max(value);
        n_used += 1;
    }

    let measured = n_used > 0;
    let coverage = aperture_coverage(h, w, x, y, radius_px, subsamples);
    Ok(PlainAperture {
        x,
        y,
        radius_px,
        flux: if measured { flux } else { f64::NAN },
        flux_err: (measured && err.is_some()).then(|| variance.sqrt()),
        used_area,
        n_used,
        n_masked,
        n_nonfinite,
        masked_fraction: masked_fraction(used_area, &coverage),
        peak: if measured { peak } else { f64::NAN },
        area_applied: area.is_some(),
    })
}

pub fn flags_under_aperture(bits: &Array2<u32>, x: f64, y: f64, radius_px: f64, subsamples: u8) -> u32 {
    let (rows, cols) = bits.dim();
    circular_aperture(rows, cols, x, y, radius_px, subsamples)
        .iter()
        .fold(0, |flags, pixel| flags | bits[[pixel.y, pixel.x]])
}

pub fn marked_pixels_under_aperture(map: &Array2<u8>, x: f64, y: f64, radius_px: f64, subsamples: u8) -> u32 {
    let (rows, cols) = map.dim();
    circular_aperture(rows, cols, x, y, radius_px, subsamples)
        .iter()
        .filter(|pixel| map[[pixel.y, pixel.x]] != 0)
        .count() as u32
}

pub fn growth_at_radii(
    source: &MaskedImage,
    x: f64,
    y: f64,
    radii_px: &[f64],
    sky_per_pixel: f64,
    subsamples: u8,
) -> Vec<GrowthPoint> {
    let image = source.image();
    let (h, w) = image.dim();
    let excluded = source.excluded();
    let area = source.area();
    radii_px
        .iter()
        .map(|&r| {
            if !(r.is_finite() && r > 0.0) {
                return GrowthPoint { r, flux: f64::NAN };
            }
            let mut sum = 0.0;
            let mut weight_sum = 0.0;
            for pixel in circular_aperture(h, w, x, y, r, subsamples) {
                let at = (pixel.y, pixel.x);
                let value = image[at] as f64;
                if excluded.is_some_and(|mask| mask[at] != 0) || !value.is_finite() {
                    continue;
                }
                let weight = area_weight(area, &pixel);
                sum += weight * value;
                weight_sum += weight;
            }
            GrowthPoint { r, flux: sum - sky_per_pixel * weight_sum }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApertureCorrection {
    pub radius_px: f64,
    pub reference_radius_px: f64,
    pub factor: f64,
    pub scatter: f64,
    pub n_stars: usize,
    pub n_candidates: usize,
}

fn flux_at_radius(curve: &[GrowthPoint], radius_px: f64) -> Option<f64> {
    if let Some(point) = curve.iter().find(|p| (p.r - radius_px).abs() <= FORCED_RADIUS_MATCH_TOLERANCE_PX) {
        return Some(point.flux);
    }
    let below = curve.iter().filter(|p| p.r < radius_px).max_by(|a, b| f64_cmp(&a.r, &b.r))?;
    let above = curve.iter().filter(|p| p.r > radius_px).min_by(|a, b| f64_cmp(&a.r, &b.r))?;
    let position = (radius_px - below.r) / (above.r - below.r);
    Some(below.flux + position * (above.flux - below.flux))
}

pub fn growth_ratio(curve: &[GrowthPoint], radius_px: f64, reference_radius_px: f64) -> Option<f64> {
    let flux = flux_at_radius(curve, radius_px)?;
    let reference_flux = flux_at_radius(curve, reference_radius_px)?;
    let ratio = reference_flux / flux;
    (flux.is_finite() && reference_flux.is_finite() && flux > 0.0 && ratio.is_finite() && ratio > 0.0).then_some(ratio)
}

pub fn point_source_locus(ratios: &[f64]) -> Vec<usize> {
    let mut finite: Vec<f64> = ratios.iter().copied().filter(|ratio| ratio.is_finite()).collect();
    if finite.len() < FORCED_MIN_CORRECTION_STARS {
        return Vec::new();
    }
    finite.sort_by(f64_cmp);
    let base = finite[1];
    let limit = base + (FORCED_LOCUS_TOLERANCE * (base - 1.0)).max(FORCED_LOCUS_MIN_WIDTH);
    (0..ratios.len()).filter(|&i| ratios[i].is_finite() && ratios[i] <= limit).collect()
}

pub fn aperture_correction_from_growth(
    curves: &[Vec<GrowthPoint>],
    radius_px: f64,
    reference_radius_px: f64,
) -> Option<ApertureCorrection> {
    let ratios: Vec<f64> =
        curves.iter().filter_map(|curve| growth_ratio(curve, radius_px, reference_radius_px)).collect();
    let members: Vec<f64> = point_source_locus(&ratios).into_iter().map(|i| ratios[i]).collect();
    if members.len() < FORCED_MIN_CORRECTION_STARS {
        return None;
    }
    let factor = exact_median_f64(&members);
    let deviations: Vec<f64> = members.iter().map(|ratio| (ratio - factor).abs()).collect();
    Some(ApertureCorrection {
        radius_px,
        reference_radius_px,
        factor,
        scatter: MAD_TO_SIGMA * exact_median_f64(&deviations),
        n_stars: members.len(),
        n_candidates: ratios.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CorrectionStar {
    pub x: f64,
    pub y: f64,
    pub snr: f64,
    pub saturated: bool,
    pub masked_fraction: f64,
}

pub fn select_correction_stars(stars: &[CorrectionStar], reference_radius_px: f64, min_snr: f64) -> Vec<usize> {
    (0..stars.len())
        .filter(|&i| {
            let star = &stars[i];
            let isolated = stars
                .iter()
                .enumerate()
                .all(|(j, other)| j == i || (other.x - star.x).hypot(other.y - star.y) >= reference_radius_px);
            star.snr >= min_snr && !star.saturated && star.masked_fraction == 0.0 && isolated
        })
        .collect()
}

pub fn upper_limit(flux: f64, flux_err: Option<f64>, f_corr: Option<f64>, limit_sigma: f64) -> Option<f64> {
    let effective_err = flux_err? * f_corr.unwrap_or(1.0);
    let limit = limit_sigma * effective_err;
    (effective_err.is_finite() && effective_err > 0.0 && flux < limit).then_some(limit)
}

pub fn ab_from_jy(flux_jy: f64) -> Option<f64> {
    (flux_jy.is_finite() && flux_jy > 0.0).then(|| -2.5 * flux_jy.log10() + AB_MAG_ZERO_POINT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::aperture::weighted_sum;

    const GAUSSIAN_TOTAL_FLUX: f64 = 25132.741228718343;
    const STAR_X: f64 = 32.4;
    const STAR_Y: f64 = 30.7;

    fn gaussian(h: usize, w: usize, x0: f64, y0: f64, amp: f64, sigma: f64, bg: f64) -> Array2<f32> {
        Array2::from_shape_fn((h, w), |(y, x)| {
            let dx = x as f64 - x0;
            let dy = y as f64 - y0;
            (bg + amp * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()) as f32
        })
    }

    fn off_centre_star(bg: f64) -> Array2<f32> {
        gaussian(64, 64, STAR_X, STAR_Y, 1000.0, 2.0, bg)
    }

    fn relative_gap(actual: f64, expected: f64) -> f64 {
        ((actual - expected) / expected).abs()
    }

    fn plain(image: &Array2<f32>, err: Option<&Array2<f32>>, x: f64, y: f64, radius_px: f64) -> PlainAperture {
        let source = MaskedImage::new(image, None).unwrap();
        measure_plain_aperture(&source, err, x, y, radius_px, FORCED_SUBSAMPLES).unwrap()
    }

    fn quadrature_of_weights(pixels: &[AperturePixel]) -> f64 {
        pixels.iter().map(|p| (p.weight as f64).powi(2)).sum::<f64>().sqrt()
    }

    fn two_point_curve(inner_flux: f64, outer_flux: f64) -> Vec<GrowthPoint> {
        vec![GrowthPoint { r: 3.0, flux: inner_flux }, GrowthPoint { r: 8.0, flux: outer_flux }]
    }

    fn clean_star(x: f64, y: f64, snr: f64) -> CorrectionStar {
        CorrectionStar { x, y, snr, saturated: false, masked_fraction: 0.0 }
    }

    #[test]
    fn a_plain_aperture_without_err_mask_or_area_equals_the_validated_aperture_sum_bit_for_bit() {
        let img = off_centre_star(100.0);
        let measured = plain(&img, None, STAR_X, STAR_Y, 6.0);
        let reference = weighted_sum(&img, &circular_aperture(64, 64, STAR_X, STAR_Y, 6.0, 5), None);
        assert_eq!(measured.flux.to_bits(), reference.sum.to_bits());
        assert_eq!(measured.used_area, reference.weight);
        assert_eq!(measured.flux_err, None);
        assert_eq!(measured.masked_fraction, 0.0);
        assert_eq!((measured.n_masked, measured.n_nonfinite), (0, 0));
        assert_eq!(measured.n_used, reference.n_finite);
        assert!(!measured.area_applied);
    }

    #[test]
    fn a_sky_free_sum_keeps_the_pedestal_and_matches_the_enclosed_gaussian_flux() {
        let bare = plain(&off_centre_star(0.0), None, STAR_X, STAR_Y, 6.0);
        let enclosed = GAUSSIAN_TOTAL_FLUX * (1.0 - (-4.5f64).exp());
        assert!((enclosed - 24853.54).abs() < 0.01, "enclosed={enclosed}");
        assert!(relative_gap(bare.flux, enclosed) < 0.01, "flux={}", bare.flux);

        let on_pedestal = plain(&off_centre_star(100.0), None, STAR_X, STAR_Y, 6.0);
        let pedestal = on_pedestal.flux - bare.flux;
        assert!(relative_gap(pedestal, 100.0 * on_pedestal.used_area) < 1e-6, "pedestal={pedestal}");
    }

    #[test]
    fn the_error_is_the_error_layer_in_quadrature_over_the_weights() {
        let img = Array2::from_elem((64, 64), 2.0f32);
        let err = Array2::from_elem(img.dim(), 0.5f32);
        let measured = plain(&img, Some(&err), 32.3, 31.6, 3.0);
        let expected_err = 0.5 * quadrature_of_weights(&circular_aperture(64, 64, 32.3, 31.6, 3.0, 5));
        assert!(relative_gap(measured.flux_err.unwrap(), expected_err) < 1e-12, "flux_err={:?}", measured.flux_err);
        assert!(relative_gap(measured.flux, 2.0 * measured.used_area) < 1e-12, "flux={}", measured.flux);
        assert!(relative_gap(measured.used_area, 9.0 * std::f64::consts::PI) < 0.02, "used_area={}", measured.used_area);
        assert_eq!(measured.peak, 2.0);
    }

    #[test]
    fn pixels_without_data_in_the_error_layer_leave_the_sum_and_the_error() {
        let img = off_centre_star(0.0);
        let without_data = [(31usize, 32usize), (30, 34), (28, 31)];
        let mut err = Array2::from_elem(img.dim(), 0.5f32);
        err[without_data[0]] = f32::NAN;
        err[without_data[1]] = 0.0;
        err[without_data[2]] = -1.0;
        let measured = plain(&img, Some(&err), STAR_X, STAR_Y, 6.0);

        let pixels = circular_aperture(64, 64, STAR_X, STAR_Y, 6.0, 5);
        let (dropped, kept): (Vec<AperturePixel>, Vec<AperturePixel>) =
            pixels.iter().copied().partition(|p| without_data.contains(&(p.y, p.x)));
        assert_eq!(dropped.len(), 3);
        let kept_flux: f64 = kept.iter().map(|p| p.weight as f64 * img[[p.y, p.x]] as f64).sum();

        assert_eq!(measured.n_nonfinite, 3);
        assert_eq!(measured.n_used as usize, kept.len());
        assert!(relative_gap(measured.flux, kept_flux) < 1e-12, "flux={} kept={kept_flux}", measured.flux);
        assert!(relative_gap(measured.flux_err.unwrap(), 0.5 * quadrature_of_weights(&kept)) < 1e-12);
        let dropped_share = total_weight(&dropped) / total_weight(&pixels);
        assert!((measured.masked_fraction - dropped_share).abs() < 1e-12, "masked_fraction={}", measured.masked_fraction);
    }

    #[test]
    fn excluded_and_non_finite_pixels_are_counted_apart_and_a_masked_nan_counts_as_masked() {
        let mut img = off_centre_star(0.0);
        let mut mask = Array2::<u8>::zeros(img.dim());
        mask[[30, 33]] = 1;
        mask[[31, 30]] = 1;
        img[[30, 33]] = f32::NAN;
        img[[33, 32]] = f32::NAN;
        let source = MaskedImage::new(&img, Some(&mask)).unwrap();
        let measured = measure_plain_aperture(&source, None, STAR_X, STAR_Y, 6.0, FORCED_SUBSAMPLES).unwrap();
        assert_eq!(measured.n_masked, 2);
        assert_eq!(measured.n_nonfinite, 1);
        assert_eq!(measured.n_used as usize, circular_aperture(64, 64, STAR_X, STAR_Y, 6.0, 5).len() - 3);
        assert!(measured.flux.is_finite());
    }

    #[test]
    fn an_aperture_cut_by_the_image_edge_reports_the_missing_area() {
        let img = Array2::from_elem((64, 64), 1.0f32);
        let measured = plain(&img, None, 1.0, 30.0, 4.0);
        let segment = (16.0 * 0.375f64.acos() - 1.5 * 13.75f64.sqrt()) / (16.0 * std::f64::consts::PI);
        assert!((segment - 0.26699).abs() < 1e-5, "segment={segment}");
        assert!((measured.masked_fraction - segment).abs() < 0.005, "masked_fraction={}", measured.masked_fraction);

        let coverage = aperture_coverage(64, 64, 1.0, 30.0, 4.0, 5);
        assert!(coverage.on_image_weight < coverage.full_weight, "{coverage:?}");
        assert_eq!(coverage.on_image_weight.to_bits(), measured.used_area.to_bits());
        assert!((coverage.on_image_weight - 36.76).abs() < 0.005, "{coverage:?}");
        assert!((coverage.full_weight - 50.20).abs() < 0.005, "{coverage:?}");
    }

    #[test]
    fn a_position_off_the_image_and_a_radius_outside_the_limits_are_refused() {
        let img = off_centre_star(0.0);
        let source = MaskedImage::new(&img, None).unwrap();
        let refusal = |x: f64, y: f64, radius_px: f64| {
            measure_plain_aperture(&source, None, x, y, radius_px, FORCED_SUBSAMPLES).unwrap_err()
        };
        assert_eq!(refusal(-2.0, 10.0, 6.0), "position (-2, 10) lies outside the 64 x 64 image");
        assert_eq!(refusal(32.0, 32.0, 1.99), "aperture radius 1.99 px is outside the 2 to 60 px limits");
        assert_eq!(refusal(32.0, 32.0, 60.01), "aperture radius 60.01 px is outside the 2 to 60 px limits");
        assert_eq!(refusal(32.0, 32.0, f64::NAN), "aperture radius NaN px is outside the 2 to 60 px limits");
        assert!(measure_plain_aperture(&source, None, 32.0, 32.0, 2.0, FORCED_SUBSAMPLES).is_ok());
        assert!(measure_plain_aperture(&source, None, 32.0, 32.0, 60.0, FORCED_SUBSAMPLES).is_ok());
    }

    #[test]
    fn an_aperture_with_no_usable_pixel_is_returned_with_a_masked_fraction_of_one() {
        let img = off_centre_star(0.0);
        let mask = Array2::from_elem(img.dim(), 1u8);
        let err = Array2::from_elem(img.dim(), 0.5f32);
        let source = MaskedImage::new(&img, Some(&mask)).unwrap();
        let measured = measure_plain_aperture(&source, Some(&err), STAR_X, STAR_Y, 6.0, FORCED_SUBSAMPLES).unwrap();
        assert_eq!(measured.n_used, 0);
        assert!(measured.flux.is_nan());
        assert!(measured.peak.is_nan());
        assert!(measured.flux_err.is_none());
        assert_eq!(measured.masked_fraction, 1.0);
    }

    #[test]
    fn an_area_map_weights_the_flux_and_the_error() {
        let img = Array2::from_elem((64, 64), 1.0f32);
        let area = Array2::from_elem(img.dim(), 1.02f32);
        let err = Array2::from_elem(img.dim(), 0.5f32);
        let source = MaskedImage::new(&img, None).unwrap().with_area(Some(&area));
        let measured = measure_plain_aperture(&source, Some(&err), 32.3, 31.6, 3.0, FORCED_SUBSAMPLES).unwrap();
        let factor = 1.02f32 as f64;
        let quadrature = quadrature_of_weights(&circular_aperture(64, 64, 32.3, 31.6, 3.0, 5));
        assert!(relative_gap(measured.flux, factor * measured.used_area) < 1e-12, "flux={}", measured.flux);
        assert!(relative_gap(measured.flux_err.unwrap(), factor * 0.5 * quadrature) < 1e-12);
        assert!(measured.area_applied);
    }

    #[test]
    fn coverage_inside_the_image_has_equal_weights() {
        let coverage = aperture_coverage(64, 64, STAR_X, STAR_Y, 6.0, 5);
        assert_eq!(coverage.on_image_weight.to_bits(), coverage.full_weight.to_bits());
        assert!(coverage.full_weight > 0.0);
        assert_eq!(masked_fraction(coverage.on_image_weight, &coverage), 0.0);
        assert_eq!(masked_fraction(0.25 * coverage.full_weight, &coverage), 0.75);
    }

    #[test]
    fn flags_are_the_or_of_the_bits_under_the_aperture() {
        let mut bits = Array2::<u32>::zeros((64, 64));
        bits[[20, 20]] = 4;
        bits[[22, 21]] = 16;
        bits[[18, 19]] = 1 << 24;
        bits[[20, 30]] = 2;
        assert_eq!(flags_under_aperture(&bits, 20.0, 20.0, 4.0, 5), 4 | 16 | (1 << 24));

        let mut map = Array2::<u8>::zeros((64, 64));
        map[[20, 20]] = 1;
        map[[21, 19]] = 7;
        map[[40, 40]] = 1;
        assert_eq!(marked_pixels_under_aperture(&map, 20.0, 20.0, 4.0, 5), 2);
    }

    #[test]
    fn growth_at_given_radii_follows_the_gaussian_enclosed_flux_and_removes_a_sky_level() {
        let radii = [2.0f64, 4.0, 6.0, 8.0];
        let enclosed = [9888.96f64, 21731.39, 24853.54, 25124.31];
        let tolerances = [0.04, 0.01, 0.01, 0.01];
        for (r, expected) in radii.into_iter().zip(enclosed) {
            let closed_form = GAUSSIAN_TOTAL_FLUX * (1.0 - f64::exp(-r * r / 8.0));
            assert!((closed_form - expected).abs() < 0.01, "r={r} closed_form={closed_form}");
        }

        let bare_img = gaussian(64, 64, 32.0, 32.0, 1000.0, 2.0, 0.0);
        let bare = growth_at_radii(&MaskedImage::new(&bare_img, None).unwrap(), 32.0, 32.0, &radii, 0.0, 5);
        let pedestal_img = gaussian(64, 64, 32.0, 32.0, 1000.0, 2.0, 100.0);
        let sky_removed = growth_at_radii(&MaskedImage::new(&pedestal_img, None).unwrap(), 32.0, 32.0, &radii, 100.0, 5);

        assert_eq!(bare.iter().map(|p| p.r).collect::<Vec<_>>(), radii);
        assert_eq!(sky_removed.len(), 4);
        for i in 0..4 {
            assert!(relative_gap(bare[i].flux, enclosed[i]) < tolerances[i], "r={} flux={}", radii[i], bare[i].flux);
            assert!(relative_gap(sky_removed[i].flux, enclosed[i]) < tolerances[i], "r={} flux={}", radii[i], sky_removed[i].flux);
            assert!(relative_gap(sky_removed[i].flux, bare[i].flux) < 1e-6, "r={} flux={}", radii[i], sky_removed[i].flux);
        }
        assert!(bare.windows(2).all(|pair| pair[1].flux >= pair[0].flux));
        assert!(relative_gap(bare[0].flux, 9569.10) < 1e-5, "flux={}", bare[0].flux);

        let unusable = growth_at_radii(&MaskedImage::new(&bare_img, None).unwrap(), 32.0, 32.0, &[0.0, f64::NAN, 4.0], 0.0, 5);
        assert!(unusable[0].flux.is_nan() && unusable[1].flux.is_nan());
        assert_eq!(unusable[2].flux.to_bits(), bare[1].flux.to_bits());
    }

    #[test]
    fn the_band_correction_is_the_median_ratio_of_the_point_source_locus() {
        let curves: Vec<Vec<GrowthPoint>> =
            [110.0, 112.0, 108.0, 111.0, 150.0].iter().map(|&outer| two_point_curve(100.0, outer)).collect();
        let correction = aperture_correction_from_growth(&curves, 3.0, 8.0).unwrap();
        assert!((correction.factor - 1.105).abs() < 1e-12, "factor={}", correction.factor);
        assert!((correction.scatter - 0.014826).abs() < 1e-12, "scatter={}", correction.scatter);
        assert_eq!((correction.n_stars, correction.n_candidates), (4, 5));
        assert_eq!((correction.radius_px, correction.reference_radius_px), (3.0, 8.0));

        assert_eq!(aperture_correction_from_growth(&curves[..2], 3.0, 8.0), None);
        let with_one_galaxy = [two_point_curve(100.0, 105.0), two_point_curve(100.0, 106.0), two_point_curve(100.0, 200.0)];
        assert_eq!(aperture_correction_from_growth(&with_one_galaxy, 3.0, 8.0), None);

        let three_point_curve = [
            GrowthPoint { r: 2.0, flux: 100.0 },
            GrowthPoint { r: 4.0, flux: 200.0 },
            GrowthPoint { r: 8.0, flux: 300.0 },
        ];
        assert_eq!(growth_ratio(&three_point_curve, 3.0, 8.0), Some(2.0));
        assert_eq!(growth_ratio(&two_point_curve(0.0, 110.0), 3.0, 8.0), None);
        assert_eq!(growth_ratio(&two_point_curve(-5.0, 110.0), 3.0, 8.0), None);
        assert_eq!(growth_ratio(&three_point_curve, 3.0, 9.0), None);
        assert_eq!(growth_ratio(&two_point_curve(100.0, f64::NAN), 3.0, 8.0), None);

        let mut with_unusable_curves = curves[..3].to_vec();
        with_unusable_curves.push(two_point_curve(0.0, 110.0));
        with_unusable_curves.push(vec![GrowthPoint { r: 3.0, flux: 100.0 }, GrowthPoint { r: 6.0, flux: 108.0 }]);
        let from_three = aperture_correction_from_growth(&with_unusable_curves, 3.0, 8.0).unwrap();
        assert_eq!((from_three.n_stars, from_three.n_candidates), (3, 3));

        let below_one: Vec<Vec<GrowthPoint>> = [97.0, 97.5, 98.0].iter().map(|&outer| two_point_curve(100.0, outer)).collect();
        let shrinking = aperture_correction_from_growth(&below_one, 3.0, 8.0).unwrap();
        assert!((shrinking.factor - 0.975).abs() < 1e-12, "factor={}", shrinking.factor);
        assert_eq!(shrinking.n_stars, 3);
    }

    #[test]
    fn the_point_source_locus_keeps_the_most_concentrated_sources() {
        assert_eq!(point_source_locus(&[1.30, 1.041, 1.036, 1.25, 1.044, 1.037, 1.15, f64::NAN]), [1, 2, 4, 5]);
        assert_eq!(point_source_locus(&[1.020, 1.040, 1.041, 1.043]), [0, 1, 2, 3]);
        assert!(point_source_locus(&[1.2, 1.3]).is_empty());
        assert!(point_source_locus(&[]).is_empty());
        assert_eq!(point_source_locus(&[1.004, 1.005, 1.006, 1.014, 1.03]), [0, 1, 2, 3]);
    }

    #[test]
    fn correction_stars_are_bright_clean_unsaturated_and_isolated() {
        let stars = [
            clean_star(20.0, 20.0, 100.0),
            clean_star(60.0, 20.0, 40.0),
            CorrectionStar { saturated: true, ..clean_star(100.0, 20.0, 100.0) },
            CorrectionStar { masked_fraction: 0.01, ..clean_star(20.0, 60.0, 100.0) },
            clean_star(60.0, 60.0, 100.0),
            clean_star(65.0, 60.0, 5.0),
        ];
        assert_eq!(select_correction_stars(&stars, 10.0, FORCED_STAR_MIN_SNR), [0]);
    }

    #[test]
    fn a_flux_below_k_times_the_effective_error_becomes_a_limit() {
        let k = FORCED_DEFAULT_LIMIT_SIGMA;
        assert_eq!(upper_limit(1.0, Some(0.5), None, k), Some(1.5));
        assert_eq!(upper_limit(2.0, Some(0.5), None, k), None);
        assert_eq!(upper_limit(1.5, Some(0.5), None, k), None);
        let corrected = upper_limit(-0.3, Some(0.2), Some(1.5), k).unwrap();
        assert!((corrected - 0.9).abs() < 1e-15, "limit={corrected}");
        assert_eq!(upper_limit(1.0, None, None, k), None);
        assert_eq!(upper_limit(1.0, Some(0.0), None, k), None);
        assert_eq!(upper_limit(f64::NAN, Some(0.5), None, k), None);

        assert!((ab_from_jy(3.630780547701009e-6).unwrap() - 22.5).abs() < 1e-9);
        assert_eq!(ab_from_jy(0.0), None);
        assert_eq!(ab_from_jy(-1.0e-6), None);
    }

    #[test]
    fn the_pixel_area_scale_comes_from_the_cd_determinant() {
        let legacy = pixel_scale_from_cd([[-7.27777777777778e-5, 0.0], [0.0, 7.27777777777778e-5]]).unwrap();
        assert!((legacy.area_scale_arcsec - 0.262).abs() < 1e-12, "{legacy:?}");

        let sdss = pixel_scale_from_cd([[-2.70219937496e-6, 1.09992169645e-4], [1.09959802344e-4, 2.76135555586e-6]]).unwrap();
        assert!((sdss.area_scale_arcsec - 0.39603565421543135).abs() < 1e-9, "{sdss:?}");
        assert!(sdss.anisotropy() < 0.001, "anisotropy={}", sdss.anisotropy());

        let oblong = pixel_scale_from_cd([[1.0 / 3600.0, 0.0], [0.0, 1.05 / 3600.0]]).unwrap();
        assert!((oblong.area_scale_arcsec - 1.0246951).abs() < 1e-7, "{oblong:?}");
        assert!((oblong.x_arcsec - 1.0).abs() < 1e-12 && (oblong.y_arcsec - 1.05).abs() < 1e-12, "{oblong:?}");
        assert!((oblong.anisotropy() - 0.047619).abs() < 1e-6, "anisotropy={}", oblong.anisotropy());
        assert!(oblong.anisotropy() > FORCED_AXIS_SCALE_TOLERANCE);

        assert_eq!(pixel_scale_from_cd([[1.0, 2.0], [2.0, 4.0]]), None);
        assert_eq!(pixel_scale_from_cd([[f64::NAN, 0.0], [0.0, 1.0]]), None);
    }
}
