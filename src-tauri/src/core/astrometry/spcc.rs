use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::catalog::{gaia_cone_stars_cached, spcc_cone_query, CatalogHit, ConeQuery};
use crate::core::astrometry::wcs::{pixel_center, CelestialCoord};
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::wcs_source::load_wcs;
use crate::math::sigma_clip::sigma_clipped_stats;
use crate::types::header::HduHeader;

const DEFAULT_WAVELENGTHS_NM: [f64; 3] = [640.0, 530.0, 460.0];
const MIN_WAVELENGTH_NM: f64 = 300.0;
const MAX_WAVELENGTH_NM: f64 = 1200.0;
const SPCC_MIN_GAIA_STARS: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpccConfig {
    pub min_snr: f64,
    pub max_stars: usize,
    pub saturation_limit: f64,
    pub catalog: SpccCatalog,
    pub white_reference: WhiteReference,
    pub wavelengths_nm: [f64; 3],
}

impl Default for SpccConfig {
    fn default() -> Self {
        Self {
            min_snr: 20.0,
            max_stars: 200,
            saturation_limit: 0.90,
            catalog: SpccCatalog::BuiltinBpRp,
            white_reference: WhiteReference::AverageSpiral,
            wavelengths_nm: DEFAULT_WAVELENGTHS_NM,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpccCatalog {
    BuiltinBpRp,
    #[serde(rename = "gaia_dr3")]
    GaiaDr3Tap,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WhiteReference {
    AverageSpiral,
    G2V,
    Photopic,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpccResult {
    pub r_factor: f64,
    pub g_factor: f64,
    pub b_factor: f64,
    pub stars_matched: usize,
    pub stars_total: usize,
    pub avg_color_index: f64,
    pub white_ref_name: String,
    pub catalog_name: String,
    pub is_synthetic_catalog: bool,
    pub wavelengths_nm: [f64; 3],
    pub wavelength_source: String,
    pub catalog_source: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub(crate) struct CatalogStar {
    pub(crate) ra: f64,
    pub(crate) dec: f64,
    pub(crate) bp_rp: f64,
}

#[derive(Debug, Clone)]
struct MatchedStar {
    bp_rp: f64,
    measured_r: f64,
    measured_g: f64,
    measured_b: f64,
}

pub fn spcc_calibrate_rgb(
    r_image: &Array2<f32>,
    g_image: &Array2<f32>,
    b_image: &Array2<f32>,
    wcs_path: &str,
    header: &HduHeader,
    config: &SpccConfig,
) -> Result<SpccResult, String> {
    if g_image.dim() != r_image.dim() || b_image.dim() != r_image.dim() {
        return Err(format!(
            "Channel size mismatch: R {:?}, G {:?}, B {:?}",
            r_image.dim(),
            g_image.dim(),
            b_image.dim()
        ));
    }

    validate_wavelengths(&config.wavelengths_nm)?;
    let wl = &config.wavelengths_nm;

    let wcs = load_wcs(wcs_path, header).map_err(|e| {
        format!(
            "No usable celestial WCS in the header ({}). SPCC needs an image whose FITS header already carries a sky solution.",
            e
        )
    })?;

    let (h, w) = r_image.dim();

    let luminance = synthesize_luminance(r_image, g_image, b_image);
    let detection = detect_stars(&luminance, 5.0);

    let stats = compute_image_stats(&luminance);
    let sat_limit = stats.max * config.saturation_limit;

    let mut good_stars: Vec<&DetectedStar> = detection
        .stars
        .iter()
        .filter(|s| passes_quality_filters(s, detection.background_median, sat_limit, config.min_snr, w, h))
        .collect();

    good_stars.sort_by(|a, b| b.snr.total_cmp(&a.snr));
    good_stars.truncate(config.max_stars);

    if good_stars.len() < 5 {
        return Err(format!(
            "Only {} stars passed quality filters (need 5+). Try lowering min_snr.",
            good_stars.len()
        ));
    }

    let star_coords: Vec<(f64, f64)> = good_stars.iter().map(|s| (s.x, s.y)).collect();
    let world_coords = wcs.pixel_to_world_batch(&star_coords);

    let (fov_w, fov_h) = wcs.field_of_view(w, h);
    let (cx, cy) = pixel_center(w, h);
    let center = wcs.pixel_to_world(cx, cy);
    let search_radius = (fov_w.max(fov_h) / 60.0) * 0.75;

    let (catalog_stars, is_synthetic, label, catalog_source) = match config.catalog {
        SpccCatalog::BuiltinBpRp => (
            generate_synthetic_catalog(&world_coords, &good_stars),
            true,
            catalog_name(&config.catalog, None),
            None,
        ),
        SpccCatalog::GaiaDr3Tap => match gaia_stars_cached(center, search_radius) {
            Ok((stars, source)) => (stars, false, catalog_name(&config.catalog, None), Some(source)),
            Err(reason) => (
                generate_synthetic_catalog(&world_coords, &good_stars),
                true,
                catalog_name(&config.catalog, Some(&reason)),
                None,
            ),
        },
    };

    let matched = cross_match_stars(
        &good_stars,
        &world_coords,
        &catalog_stars,
        r_image,
        g_image,
        b_image,
        wcs.pixel_scale_arcsec(),
    );

    if matched.len() < 3 {
        return Err(format!(
            "Only {} stars cross-matched (need 3+). Check WCS solution quality.",
            matched.len()
        ));
    }

    let (wr_r, wr_g, wr_b) = white_reference_rgb(&config.white_reference, wl);

    let (r_factor, g_factor, b_factor, avg_ci) =
        compute_correction_factors(&matched, wr_r, wr_g, wr_b, wl)?;

    let white_ref_name = match &config.white_reference {
        WhiteReference::AverageSpiral => "Average Spiral Galaxy".into(),
        WhiteReference::G2V => "G2V (Solar)".into(),
        WhiteReference::Photopic => "Photopic (Human Eye)".into(),
    };

    Ok(SpccResult {
        r_factor,
        g_factor,
        b_factor,
        stars_matched: matched.len(),
        stars_total: good_stars.len(),
        avg_color_index: avg_ci,
        white_ref_name,
        catalog_name: label,
        is_synthetic_catalog: is_synthetic,
        wavelengths_nm: config.wavelengths_nm,
        wavelength_source: wavelength_source(wl).to_string(),
        catalog_source,
    })
}

fn validate_wavelengths(wl: &[f64; 3]) -> Result<(), String> {
    let in_range = |v: f64| v.is_finite() && (MIN_WAVELENGTH_NM..=MAX_WAVELENGTH_NM).contains(&v);
    if wl.iter().all(|&v| in_range(v)) {
        return Ok(());
    }
    Err(format!(
        "SPCC wavelengths must be finite and between {} and {} nm (got [{}, {}, {}]).",
        MIN_WAVELENGTH_NM, MAX_WAVELENGTH_NM, wl[0], wl[1], wl[2]
    ))
}

fn wavelength_source(wl: &[f64; 3]) -> &'static str {
    if *wl == DEFAULT_WAVELENGTHS_NM { "default" } else { "filters" }
}

pub(crate) fn gaia_stars(
    center: CelestialCoord,
    radius_deg: f64,
    cached: &mut dyn FnMut(&ConeQuery) -> Result<CatalogHit, String>,
) -> Result<(Vec<CatalogStar>, &'static str), String> {
    let query = spcc_cone_query(center.ra, center.dec, radius_deg);
    let hit = cached(&query)?;
    let stars = hit
        .rows
        .iter()
        .filter_map(|r| {
            r.bp_rp
                .or_else(|| Some(r.bp? - r.rp?))
                .map(|bp_rp| CatalogStar { ra: r.ra, dec: r.dec, bp_rp })
        })
        .collect();
    Ok((stars, hit.source_name()))
}

fn require_min_gaia_stars(
    stars: Vec<CatalogStar>,
    source: &'static str,
    query_radius_deg: f64,
) -> Result<(Vec<CatalogStar>, &'static str), String> {
    if stars.len() < SPCC_MIN_GAIA_STARS {
        return Err(format!("Gaia returned only {} usable stars within {:.2} deg", stars.len(), query_radius_deg));
    }
    Ok((stars, source))
}

fn gaia_stars_cached(center: CelestialCoord, radius_deg: f64) -> Result<(Vec<CatalogStar>, &'static str), String> {
    let (stars, source) = gaia_stars(center, radius_deg, &mut |q| gaia_cone_stars_cached(q.ra, q.dec, q.radius_deg))?;
    require_min_gaia_stars(stars, source, spcc_cone_query(center.ra, center.dec, radius_deg).radius_deg)
}

fn catalog_name(catalog: &SpccCatalog, fallback_reason: Option<&str>) -> String {
    match (catalog, fallback_reason) {
        (SpccCatalog::BuiltinBpRp, _) => "Built-in Bp-Rp".into(),
        (SpccCatalog::GaiaDr3Tap, None) => "Gaia DR3 (VizieR)".into(),
        (SpccCatalog::GaiaDr3Tap, Some(reason)) => {
            format!("Built-in Bp-Rp (fallback, Gaia DR3 via VizieR unavailable: {reason})")
        }
    }
}

const SPCC_EDGE_MARGIN_PX: usize = 10;

fn passes_quality_filters(
    star: &DetectedStar,
    background_median: f64,
    sat_limit: f64,
    min_snr: f64,
    w: usize,
    h: usize,
) -> bool {
    let pedestal = if background_median.is_finite() { background_median } else { 0.0 };
    let margin = SPCC_EDGE_MARGIN_PX as f64;
    star.snr >= min_snr
        && star.peak + pedestal < sat_limit
        && star.x >= margin
        && star.y >= margin
        && star.x < w.saturating_sub(SPCC_EDGE_MARGIN_PX) as f64
        && star.y < h.saturating_sub(SPCC_EDGE_MARGIN_PX) as f64
}

fn synthesize_luminance(r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>) -> Array2<f32> {
    let (h, w) = r.dim();
    let mut lum = Array2::<f32>::zeros((h, w));
    let r_s = r.as_slice().unwrap();
    let g_s = g.as_slice().unwrap();
    let b_s = b.as_slice().unwrap();
    let l_s = lum.as_slice_mut().unwrap();
    for i in 0..r_s.len() {
        l_s[i] = 0.2126 * r_s[i] + 0.7152 * g_s[i] + 0.0722 * b_s[i];
    }
    lum
}

fn bp_rp_to_teff(bp_rp: f64) -> f64 {
    let x = bp_rp.clamp(-0.5, 5.0);
    if x < 0.0 {
        10000.0 + (-x) * 20000.0
    } else if x < 0.5 {
        7500.0 + (0.5 - x) * 5000.0
    } else if x < 1.0 {
        5800.0 + (1.0 - x) * 3400.0
    } else if x < 1.5 {
        4500.0 + (1.5 - x) * 2600.0
    } else if x < 2.5 {
        3500.0 + (2.5 - x) * 1000.0
    } else {
        2800.0 + (5.0 - x) * 280.0
    }
}

fn planck_rgb(teff: f64, wl: &[f64; 3]) -> (f64, f64, f64) {
    let r = planck_intensity(teff, wl[0]);
    let g = planck_intensity(teff, wl[1]);
    let b = planck_intensity(teff, wl[2]);

    let max_val = r.max(g).max(b);
    if max_val < 1e-30 {
        return (1.0, 1.0, 1.0);
    }

    (r / max_val, g / max_val, b / max_val)
}

fn planck_intensity(teff: f64, wavelength_nm: f64) -> f64 {
    let lambda = wavelength_nm * 1e-9;
    let h = 6.626e-34;
    let c = 2.998e8;
    let k = 1.381e-23;

    let exponent = h * c / (lambda * k * teff);
    if exponent > 500.0 {
        return 0.0;
    }

    let numerator = 2.0 * h * c * c / (lambda.powi(5));
    numerator / (exponent.exp() - 1.0)
}

fn white_reference_rgb(wr: &WhiteReference, wl: &[f64; 3]) -> (f64, f64, f64) {
    match wr {
        WhiteReference::G2V => planck_rgb(5778.0, wl),
        WhiteReference::AverageSpiral => {
            let (r, g, b) = planck_rgb(5500.0, wl);
            (r * 0.98, g * 1.0, b * 1.02)
        }
        WhiteReference::Photopic => (1.0, 1.0, 1.0),
    }
}

fn generate_synthetic_catalog(
    world_coords: &[crate::core::astrometry::wcs::CelestialCoord],
    stars: &[&DetectedStar],
) -> Vec<CatalogStar> {
    world_coords
        .iter()
        .zip(stars.iter())
        .map(|(coord, star)| {
            let bp_rp = estimate_bp_rp_from_flux(star);
            CatalogStar {
                ra: coord.ra,
                dec: coord.dec,
                bp_rp,
            }
        })
        .collect()
}

fn estimate_bp_rp_from_flux(star: &DetectedStar) -> f64 {
    let norm_flux = (star.flux / star.peak.max(1e-10)).clamp(0.1, 100.0);
    let fwhm_factor = (star.fwhm - 3.0).clamp(-2.0, 5.0) * 0.1;
    (1.0 / norm_flux.sqrt() + fwhm_factor).clamp(-0.3, 4.0)
}

fn cross_match_stars(
    detected: &[&DetectedStar],
    world_coords: &[crate::core::astrometry::wcs::CelestialCoord],
    catalog: &[CatalogStar],
    r_image: &Array2<f32>,
    g_image: &Array2<f32>,
    b_image: &Array2<f32>,
    pixel_scale: f64,
) -> Vec<MatchedStar> {
    let match_radius = (pixel_scale * 3.0) / 3600.0;
    let match_r2 = match_radius * match_radius;
    let mut matched = Vec::new();

    for (i, star) in detected.iter().enumerate() {
        let wc = &world_coords[i];

        let mut best_dist = f64::MAX;
        let mut best_cat: Option<&CatalogStar> = None;

        for cat in catalog {
            let mut dra = wc.ra - cat.ra;
            if dra > 180.0 { dra -= 360.0; } else if dra < -180.0 { dra += 360.0; }
            let dra = dra * wc.dec.to_radians().cos();
            let ddec = wc.dec - cat.dec;
            let d2 = dra * dra + ddec * ddec;
            if d2 < match_r2 && d2 < best_dist {
                best_dist = d2;
                best_cat = Some(cat);
            }
        }

        if let Some(cat) = best_cat {
            let radius = (star.fwhm * 1.5).max(3.0);
            let r_flux = aperture_flux_f32(r_image, star.x, star.y, radius);
            let g_flux = aperture_flux_f32(g_image, star.x, star.y, radius);
            let b_flux = aperture_flux_f32(b_image, star.x, star.y, radius);

            if r_flux > 0.0 && g_flux > 0.0 && b_flux > 0.0 {
                matched.push(MatchedStar {
                    bp_rp: cat.bp_rp,
                    measured_r: r_flux,
                    measured_g: g_flux,
                    measured_b: b_flux,
                });
            }
        }
    }

    matched
}

fn aperture_flux_f32(image: &Array2<f32>, x: f64, y: f64, radius: f64) -> f64 {
    let (h, w) = image.dim();
    let r2 = radius * radius;
    let inner_annulus = radius * 2.0;
    let outer_annulus = radius * 3.5;
    let inner_r2 = inner_annulus * inner_annulus;
    let outer_r2 = outer_annulus * outer_annulus;
    let mut flux = 0.0f64;
    let mut aperture_count = 0u32;
    let mut bg_vals: Vec<f32> = Vec::new();

    let y_min = (y - outer_annulus).floor().max(0.0) as usize;
    let y_max = ((y + outer_annulus).ceil() as usize).min(h.saturating_sub(1));
    let x_min = (x - outer_annulus).floor().max(0.0) as usize;
    let x_max = ((x + outer_annulus).ceil() as usize).min(w.saturating_sub(1));

    for py in y_min..=y_max {
        for px in x_min..=x_max {
            let dx = px as f64 - x;
            let dy = py as f64 - y;
            let d2 = dx * dx + dy * dy;
            let v = image[[py, px]] as f64;
            if !v.is_finite() {
                continue;
            }
            if d2 <= r2 {
                flux += v;
                aperture_count += 1;
            } else if d2 >= inner_r2 && d2 <= outer_r2 {
                bg_vals.push(v as f32);
            }
        }
    }

    if !bg_vals.is_empty() && aperture_count > 0 {
        let (bg_per_pixel, _) = sigma_clipped_stats(&mut bg_vals, 3.0, 3);
        flux -= bg_per_pixel * aperture_count as f64;
    }

    flux.max(0.0)
}

const MIN_WHITE_REFERENCE_WEIGHT: f64 = 1e-3;

fn validate_white_reference(wr_r: f64, wr_g: f64, wr_b: f64) -> Result<(), String> {
    for (name, weight) in [("R", wr_r), ("G", wr_g), ("B", wr_b)] {
        if !weight.is_finite() || weight <= MIN_WHITE_REFERENCE_WEIGHT {
            return Err(format!(
                "White reference {} weight is {}, which carries no usable signal (must be > {}). Pick a different white reference.",
                name, weight, MIN_WHITE_REFERENCE_WEIGHT
            ));
        }
    }
    Ok(())
}

fn compute_correction_factors(
    matched: &[MatchedStar],
    wr_r: f64,
    wr_g: f64,
    wr_b: f64,
    wl: &[f64; 3],
) -> Result<(f64, f64, f64, f64), String> {
    validate_white_reference(wr_r, wr_g, wr_b)?;

    let mut sum_ratio_r = 0.0f64;
    let mut sum_ratio_g = 0.0f64;
    let mut sum_ratio_b = 0.0f64;
    let mut sum_weight_r = 0.0f64;
    let mut sum_weight_g = 0.0f64;
    let mut sum_weight_b = 0.0f64;
    let mut sum_ci = 0.0f64;
    let mut ci_count = 0usize;

    for star in matched {
        let teff = bp_rp_to_teff(star.bp_rp);
        let (expected_r, expected_g, expected_b) = planck_rgb(teff, wl);

        let total_measured = star.measured_r + star.measured_g + star.measured_b;
        let total_expected = expected_r + expected_g + expected_b;
        if total_measured < 1e-10 || total_expected < 1e-10 {
            continue;
        }

        let weight = total_measured.sqrt();

        let mr = star.measured_r / total_measured;
        let mg = star.measured_g / total_measured;
        let mb = star.measured_b / total_measured;

        let er = expected_r / total_expected;
        let eg = expected_g / total_expected;
        let eb = expected_b / total_expected;

        if mr > 1e-6 {
            sum_ratio_r += (er / mr) * weight;
            sum_weight_r += weight;
        }
        if mg > 1e-6 {
            sum_ratio_g += (eg / mg) * weight;
            sum_weight_g += weight;
        }
        if mb > 1e-6 {
            sum_ratio_b += (eb / mb) * weight;
            sum_weight_b += weight;
        }
        sum_ci += star.bp_rp;
        ci_count += 1;
    }

    let unmeasured: Vec<&str> = [
        ("R", sum_weight_r),
        ("G", sum_weight_g),
        ("B", sum_weight_b),
    ]
    .into_iter()
    .filter(|(_, weight)| !weight.is_finite() || *weight <= 0.0)
    .map(|(name, _)| name)
    .collect();

    if !unmeasured.is_empty() {
        return Err(format!(
            "No matched star carries measurable flux in {}. A channel without signal cannot be color-calibrated; neutral factors would report success on an empty channel.",
            unmeasured.join(", ")
        ));
    }

    let mut r_factor = sum_ratio_r / sum_weight_r / wr_r;
    let mut g_factor = sum_ratio_g / sum_weight_g / wr_g;
    let mut b_factor = sum_ratio_b / sum_weight_b / wr_b;

    if !g_factor.is_finite() || g_factor <= 0.0 {
        return Err(format!(
            "Green correction factor is {}, so the other channels cannot be normalized against it.",
            g_factor
        ));
    }

    r_factor /= g_factor;
    b_factor /= g_factor;
    g_factor = 1.0;

    if !r_factor.is_finite() || !b_factor.is_finite() {
        return Err(format!(
            "Correction factors are not finite after green normalization (R {}, B {}).",
            r_factor, b_factor
        ));
    }

    let avg_ci = if ci_count > 0 { sum_ci / ci_count as f64 } else { 0.0 };

    Ok((r_factor, g_factor, b_factor, avg_ci))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::core::astrometry::catalog::{
        query_gaia_cached_with, CatalogRow, CatalogSource, SPCC_GAIA_MAG_LIMIT, SPCC_GAIA_MAX_ROWS, SPCC_MIN_CONE_RADIUS_DEG,
    };

    #[test]
    fn aperture_background_rejects_neighbor_contamination() {
        let mut img = Array2::from_elem((60, 60), 100.0f32);
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                img[[(30 + dy) as usize, (30 + dx) as usize]] = 500.0;
            }
        }
        let radius = 4.0;
        let flux_clean = aperture_flux_f32(&img, 30.0, 30.0, radius);

        let mut contaminated = img.clone();
        for dy in 0..3usize {
            for dx in 0..3usize {
                contaminated[[30 + dy, 41 + dx]] = 5000.0;
            }
        }
        let flux_contaminated = aperture_flux_f32(&contaminated, 30.0, 30.0, radius);

        let rel = (flux_contaminated - flux_clean).abs() / flux_clean.max(1.0);
        assert!(
            rel < 0.05,
            "neighbor in annulus shifted flux by {:.1}% (clean={}, contaminated={})",
            rel * 100.0,
            flux_clean,
            flux_contaminated
        );
        assert!(flux_clean > 9000.0, "expected ~10000 net star flux, got {}", flux_clean);
    }

    fn matched_sample() -> Vec<MatchedStar> {
        vec![
            MatchedStar { bp_rp: 0.8, measured_r: 1.0, measured_g: 1.2, measured_b: 0.9 },
            MatchedStar { bp_rp: 1.4, measured_r: 2.0, measured_g: 1.8, measured_b: 1.1 },
            MatchedStar { bp_rp: 0.4, measured_r: 0.7, measured_g: 1.0, measured_b: 1.3 },
        ]
    }

    #[test]
    fn compute_correction_factors_rejects_zero_white_reference() {
        let err = compute_correction_factors(&matched_sample(), 1.0, 1.0, 0.0, &DEFAULT_WAVELENGTHS_NM)
            .expect_err("a zero white reference weight must not become a x1e10 gain");
        assert!(err.contains("White reference B"), "unexpected error: {}", err);
    }

    #[test]
    fn compute_correction_factors_rejects_negligible_white_reference() {
        let err = compute_correction_factors(&matched_sample(), 1e-12, 1.0, 1.0, &DEFAULT_WAVELENGTHS_NM)
            .expect_err("a negligible white reference weight must not become a huge gain");
        assert!(err.contains("White reference R"), "unexpected error: {}", err);

        let err = compute_correction_factors(&matched_sample(), 1.0, f64::NAN, 1.0, &DEFAULT_WAVELENGTHS_NM)
            .expect_err("a non-finite white reference weight must be rejected");
        assert!(err.contains("White reference G"), "unexpected error: {}", err);
    }

    #[test]
    fn compute_correction_factors_stay_bounded_for_usable_white_reference() {
        let (wr_r, wr_g, wr_b) = white_reference_rgb(&WhiteReference::AverageSpiral, &DEFAULT_WAVELENGTHS_NM);
        let (r_factor, g_factor, b_factor, _) =
            compute_correction_factors(&matched_sample(), wr_r, wr_g, wr_b, &DEFAULT_WAVELENGTHS_NM).unwrap();

        assert_eq!(g_factor, 1.0);
        assert!(r_factor.is_finite() && r_factor > 1e-3 && r_factor < 1e3, "r_factor {}", r_factor);
        assert!(b_factor.is_finite() && b_factor > 1e-3 && b_factor < 1e3, "b_factor {}", b_factor);
    }

    #[test]
    fn compute_correction_factors_reject_a_channel_without_measured_flux() {
        let (wr_r, wr_g, wr_b) = white_reference_rgb(&WhiteReference::AverageSpiral, &DEFAULT_WAVELENGTHS_NM);
        let matched: Vec<MatchedStar> = matched_sample()
            .into_iter()
            .map(|s| MatchedStar { measured_b: 0.0, ..s })
            .collect();

        let err = compute_correction_factors(&matched, wr_r, wr_g, wr_b, &DEFAULT_WAVELENGTHS_NM)
            .expect_err("an empty B channel must not be reported as a successful calibration");

        assert!(err.contains("measurable flux in B"), "unexpected error: {}", err);
        assert!(err.contains("cannot be color-calibrated"), "unexpected error: {}", err);
        assert!(!err.to_lowercase().contains("spectrophotometr"), "the method is a blackbody approximation: {}", err);
    }

    fn star(peak: f64) -> DetectedStar {
        DetectedStar { x: 50.0, y: 50.0, flux: peak * 10.0, fwhm: 3.0, eccentricity: 0.0, peak, npix: 20, snr: 100.0 }
    }

    #[test]
    fn saturation_filter_compares_the_absolute_peak_with_the_limit() {
        let image_max = 16383.0;
        let sat_limit = 0.9 * image_max;
        let pedestal = 2000.0;
        let clipped = star(image_max - pedestal);
        assert!(
            !passes_quality_filters(&clipped, pedestal, sat_limit, 20.0, 100, 100),
            "a core at the image maximum on a {pedestal} pedestal is saturated"
        );
        assert!(passes_quality_filters(&star(5000.0), pedestal, sat_limit, 20.0, 100, 100));
        assert!(passes_quality_filters(&star(5000.0), f64::NAN, sat_limit, 20.0, 100, 100));
        assert!(!passes_quality_filters(&star(5000.0), pedestal, sat_limit, 200.0, 100, 100));
        let mut edge = star(5000.0);
        edge.x = 95.0;
        assert!(!passes_quality_filters(&edge, pedestal, sat_limit, 20.0, 100, 100));
    }

    #[test]
    fn spcc_calibrate_rgb_rejects_channel_size_mismatch() {
        let mut header = HduHeader::empty();
        for (k, v) in [
            ("NAXIS1", "64"),
            ("NAXIS2", "64"),
            ("CRPIX1", "32"),
            ("CRPIX2", "32"),
            ("CRVAL1", "180.0"),
            ("CRVAL2", "45.0"),
            ("CDELT1", "-0.001"),
            ("CDELT2", "0.001"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ] {
            header.set(k, v.to_string());
        }
        let r = Array2::from_elem((64, 64), 0.1f32);
        let g = Array2::from_elem((32, 32), 0.1f32);
        let b = Array2::from_elem((64, 64), 0.1f32);
        let config = SpccConfig::default();

        let err = spcc_calibrate_rgb(&r, &g, &b, "", &header, &config)
            .expect_err("mismatched G channel must be rejected");
        assert!(err.contains("size mismatch"), "unexpected error: {}", err);
        assert!(err.contains("(32, 32)"), "unexpected error: {}", err);

        let b_small = Array2::from_elem((64, 32), 0.1f32);
        let err = spcc_calibrate_rgb(&r, &r, &b_small, "", &header, &config)
            .expect_err("mismatched B channel must be rejected");
        assert!(err.contains("size mismatch"), "unexpected error: {}", err);
    }

    #[test]
    fn the_catalog_name_says_when_the_synthetic_fallback_replaced_gaia() {
        assert_eq!(catalog_name(&SpccCatalog::BuiltinBpRp, None), "Built-in Bp-Rp");
        assert_eq!(catalog_name(&SpccCatalog::GaiaDr3Tap, None), "Gaia DR3 (VizieR)");
        assert_eq!(
            catalog_name(&SpccCatalog::GaiaDr3Tap, Some("VizieR returned HTTP 503")),
            "Built-in Bp-Rp (fallback, Gaia DR3 via VizieR unavailable: VizieR returned HTTP 503)"
        );
    }

    fn solved_header() -> HduHeader {
        let mut header = HduHeader::empty();
        header.set("NAXIS1", "128".to_string());
        header.set("NAXIS2", "128".to_string());
        header.set("CTYPE1", "RA---TAN".to_string());
        header.set("CTYPE2", "DEC--TAN".to_string());
        header.set_f64("CRVAL1", 83.8);
        header.set_f64("CRVAL2", -5.4);
        header.set_f64("CRPIX1", 64.0);
        header.set_f64("CRPIX2", 64.0);
        header.set_f64("CD1_1", -2.78e-4);
        header.set_f64("CD2_2", 2.78e-4);
        header
    }

    fn star_field(scale: f64) -> Array2<f32> {
        let stars = [
            (20.0, 24.0, 6000.0, 1.6),
            (40.0, 96.0, 500.0, 1.2),
            (64.0, 30.0, 800.0, 1.4),
            (90.0, 60.0, 1200.0, 1.7),
            (28.0, 70.0, 1600.0, 2.0),
            (104.0, 104.0, 2000.0, 2.2),
            (70.0, 84.0, 2400.0, 2.4),
            (96.0, 20.0, 1000.0, 1.3),
            (50.0, 56.0, 700.0, 1.9),
        ];
        Array2::from_shape_fn((128, 128), |(y, x)| {
            let noise = ((y * 31 + x * 17) % 23) as f64 - 11.0;
            let mut v = 100.0 + noise;
            for (cy, cx, amp, sigma) in stars {
                let d2 = (y as f64 - cy).powi(2) + (x as f64 - cx).powi(2);
                v += amp * (-d2 / (2.0 * sigma * sigma)).exp();
            }
            (v * scale) as f32
        })
    }

    fn builtin_config() -> SpccConfig {
        SpccConfig { min_snr: 5.0, catalog: SpccCatalog::BuiltinBpRp, ..SpccConfig::default() }
    }

    fn calibrate_builtin(config: &SpccConfig) -> Result<SpccResult, String> {
        spcc_calibrate_rgb(&star_field(1.0), &star_field(0.8), &star_field(0.6), "", &solved_header(), config)
    }

    #[test]
    fn planck_rgb_uses_the_given_wavelengths() {
        let rg = |teff: f64, wl: &[f64; 3]| {
            let (r, g, _) = planck_rgb(teff, wl);
            r / g
        };
        let hst = [814.0, 555.0, 435.0];
        let hst_ratio = rg(4500.0, &hst) / rg(5500.0, &hst);
        let default_ratio = rg(4500.0, &DEFAULT_WAVELENGTHS_NM) / rg(5500.0, &DEFAULT_WAVELENGTHS_NM);
        let double_ratio = hst_ratio / default_ratio;
        assert!(
            (double_ratio - 1.144).abs() <= 0.005,
            "Planck double ratio {double_ratio} (hst {hst_ratio}, default {default_ratio})"
        );
        assert!((default_ratio - 1.2010).abs() < 0.002, "default (R/G)4500/(R/G)5500 {default_ratio}");
        assert!((hst_ratio - 1.3743).abs() < 0.002, "hst (R/G)4500/(R/G)5500 {hst_ratio}");
    }

    #[test]
    fn white_reference_uses_the_given_wavelengths() {
        let hst = [814.0, 555.0, 435.0];
        let (r, g, b) = planck_rgb(5500.0, &hst);
        let spiral = white_reference_rgb(&WhiteReference::AverageSpiral, &hst);
        assert_eq!(spiral, (r * 0.98, g, b * 1.02));
        assert_ne!(spiral, white_reference_rgb(&WhiteReference::AverageSpiral, &DEFAULT_WAVELENGTHS_NM));

        let g2v = white_reference_rgb(&WhiteReference::G2V, &hst);
        assert_eq!(g2v, planck_rgb(5778.0, &hst));
        assert_ne!(g2v, white_reference_rgb(&WhiteReference::G2V, &DEFAULT_WAVELENGTHS_NM));

        assert_eq!(white_reference_rgb(&WhiteReference::Photopic, &hst), (1.0, 1.0, 1.0));
    }

    #[test]
    fn fewer_than_three_usable_gaia_stars_are_refused() {
        let star = |ra: f64| CatalogStar { ra, dec: 10.0, bp_rp: 0.7 };
        let err = require_min_gaia_stars(vec![star(100.0), star(100.01)], "network", 0.01)
            .expect_err("two stars are below the SPCC minimum");
        assert_eq!(err, "Gaia returned only 2 usable stars within 0.01 deg");

        let (stars, source) = require_min_gaia_stars(vec![star(100.0), star(100.01), star(100.02)], "disk", 0.01)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(stars.len(), 3);
        assert_eq!(source, "disk");
    }

    #[test]
    fn spcc_result_reports_default_wavelengths_when_none_are_given() {
        let result = calibrate_builtin(&builtin_config()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(result.wavelengths_nm, [640.0, 530.0, 460.0]);
        assert_eq!(result.wavelength_source, "default");
        assert_eq!(result.catalog_source, None);
        assert!(result.is_synthetic_catalog);

        let filters = SpccConfig { wavelengths_nm: [814.0, 555.0, 435.0], ..builtin_config() };
        let from_filters = calibrate_builtin(&filters).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(from_filters.wavelengths_nm, [814.0, 555.0, 435.0]);
        assert_eq!(from_filters.wavelength_source, "filters");
        assert_eq!(from_filters.catalog_source, None);
        assert_ne!(from_filters.r_factor, result.r_factor, "the wavelengths must change the Planck expectation");
    }

    fn fake_rows() -> Vec<CatalogRow> {
        let row = |id: &str, ra: f64, dec: f64, bp_rp: Option<f64>, bp: Option<f64>, rp: Option<f64>, g: Option<f64>| CatalogRow {
            id: id.to_string(),
            ra,
            dec,
            ra_epoch: ra,
            dec_epoch: dec,
            pm_ra_masyr: None,
            pm_dec_masyr: None,
            g,
            bp,
            rp,
            bp_rp,
            parallax_mas: None,
        };
        vec![
            row("a", 265.51, 31.26, Some(0.65), None, None, Some(8.5)),
            row("b", 265.49, 31.24, None, Some(11.2), Some(10.1), Some(10.7)),
            row("c", 265.5, 31.25, None, None, None, Some(12.0)),
        ]
    }

    fn assert_same_stars(a: &[CatalogStar], b: &[CatalogStar]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert_eq!((x.ra, x.dec, x.bp_rp), (y.ra, y.dec, y.bp_rp));
        }
    }

    #[test]
    fn gaia_rows_become_catalog_stars_through_the_shared_cache() {
        let center = CelestialCoord { ra: 265.5, dec: 31.25 };
        let mut calls = 0usize;
        let mut seen: Vec<ConeQuery> = Vec::new();
        let mut cached = |q: &ConeQuery| {
            seen.push(q.clone());
            query_gaia_cached_with(q, |_| {
                calls += 1;
                Ok(fake_rows())
            })
        };

        let (first, source) = gaia_stars(center, 0.3, &mut cached).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(source, "network");
        assert_eq!(first.len(), 2, "the row without bp_rp, bp or rp is dropped");
        assert!((first[0].ra - 265.51).abs() < 1e-9 && (first[0].dec - 31.26).abs() < 1e-9);
        assert!((first[0].bp_rp - 0.65).abs() < 1e-9);
        assert!((first[1].bp_rp - 1.1).abs() < 1e-9, "bp - rp when bp_rp is absent: {}", first[1].bp_rp);

        let (second, source) = gaia_stars(center, 0.3, &mut cached).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(source, "memory");
        assert_same_stars(&first, &second);
        assert_eq!(calls, 1, "the second call is served from the shared memory cache");

        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], seen[1]);
        assert_eq!(seen[0].ra, 265.5);
        assert_eq!(seen[0].dec, 31.25);
        assert_eq!(seen[0].radius_deg, 0.3);
        assert_eq!(seen[0].mag_limit, Some(SPCC_GAIA_MAG_LIMIT));
        assert_eq!(seen[0].max_rows, SPCC_GAIA_MAX_ROWS);

        let mut tiny: Option<ConeQuery> = None;
        let (none, source) = gaia_stars(CelestialCoord { ra: 266.0, dec: 32.0 }, 0.001, &mut |q| {
            tiny = Some(q.clone());
            Ok(CatalogHit { rows: Arc::new(Vec::new()), source: CatalogSource::Disk })
        })
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(none.is_empty());
        assert_eq!(source, "disk");
        let tiny = tiny.expect("the seam receives the query");
        assert!(tiny.radius_deg >= SPCC_MIN_CONE_RADIUS_DEG, "radius {}", tiny.radius_deg);
        assert_eq!(tiny.radius_deg, SPCC_MIN_CONE_RADIUS_DEG);

        let failed = gaia_stars(center, 0.3, &mut |_| Err("VizieR request failed: timeout".to_string()))
            .expect_err("a seam error passes through");
        assert_eq!(failed, "VizieR request failed: timeout");
    }

    #[test]
    fn wavelength_validation_rejects_out_of_range() {
        let bad = SpccConfig { wavelengths_nm: [200.0, 530.0, 460.0], ..builtin_config() };
        let err = calibrate_builtin(&bad).expect_err("200 nm is outside the SPCC range");
        assert_eq!(err, "SPCC wavelengths must be finite and between 300 and 1200 nm (got [200, 530, 460]).");

        let nan = validate_wavelengths(&[640.0, f64::NAN, 460.0]).expect_err("NaN is not finite");
        assert!(nan.starts_with("SPCC wavelengths must be finite and between 300 and 1200 nm (got "), "{nan}");
        assert!(validate_wavelengths(&[640.0, 530.0, 1200.5]).is_err());
        assert!(validate_wavelengths(&[640.0, 530.0, f64::INFINITY]).is_err());
        assert!(validate_wavelengths(&[300.0, 530.0, 1200.0]).is_ok());
        assert!(validate_wavelengths(&DEFAULT_WAVELENGTHS_NM).is_ok());
    }
}
