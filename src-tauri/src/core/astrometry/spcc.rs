use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::stats::compute_image_stats;
use crate::types::header::HduHeader;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpccConfig {
    pub min_snr: f64,
    pub max_stars: usize,
    pub saturation_limit: f64,
    pub catalog: SpccCatalog,
    pub white_reference: WhiteReference,
}

impl Default for SpccConfig {
    fn default() -> Self {
        Self {
            min_snr: 20.0,
            max_stars: 200,
            saturation_limit: 0.90,
            catalog: SpccCatalog::BuiltinBpRp,
            white_reference: WhiteReference::AverageSpiral,
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
    Custom(f64, f64, f64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone)]
pub(crate) struct CatalogStar {
    pub(crate) ra: f64,
    pub(crate) dec: f64,
    pub(crate) bp_rp: f64,
    pub(crate) gmag: Option<f64>,
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
    header: &HduHeader,
    config: &SpccConfig,
) -> Result<SpccResult, String> {
    let wcs = WcsTransform::from_header(header)
        .map_err(|e| format!("WCS not available: {}. Run Plate Solve first.", e))?;

    let (h, w) = r_image.dim();

    let luminance = synthesize_luminance(r_image, g_image, b_image);
    let detection = detect_stars(&luminance, 5.0);

    let stats = compute_image_stats(&luminance);
    let sat_limit = (stats.max * config.saturation_limit) as f32;

    let mut good_stars: Vec<&DetectedStar> = detection
        .stars
        .iter()
        .filter(|s| {
            s.snr >= config.min_snr
                && s.peak < sat_limit as f64
                && s.x >= 10.0
                && s.y >= 10.0
                && s.x < w.saturating_sub(10) as f64
                && s.y < h.saturating_sub(10) as f64
        })
        .collect();

    good_stars.sort_by(|a, b| b.snr.partial_cmp(&a.snr).unwrap_or(std::cmp::Ordering::Equal));
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
    let center = wcs.pixel_to_world(w as f64 / 2.0, h as f64 / 2.0);
    let search_radius = (fov_w.max(fov_h) / 60.0) * 0.75;

    let (catalog_stars, is_synthetic) = match config.catalog {
        SpccCatalog::BuiltinBpRp => {
            (generate_synthetic_catalog(&world_coords, &good_stars), true)
        }
        SpccCatalog::GaiaDr3Tap => {
            match query_gaia_vizier(center.ra, center.dec, search_radius, 3) {
                Ok(stars) => (stars, false),
                Err(_) => (generate_synthetic_catalog(&world_coords, &good_stars), true),
            }
        }
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

    let (wr_r, wr_g, wr_b) = white_reference_rgb(&config.white_reference);

    let (r_factor, g_factor, b_factor, avg_ci) = compute_correction_factors(&matched, wr_r, wr_g, wr_b);

    let white_ref_name = match &config.white_reference {
        WhiteReference::AverageSpiral => "Average Spiral Galaxy".into(),
        WhiteReference::G2V => "G2V (Solar)".into(),
        WhiteReference::Photopic => "Photopic (Human Eye)".into(),
        WhiteReference::Custom(r, g, b) => format!("Custom ({:.2},{:.2},{:.2})", r, g, b),
    };

    let catalog_name = match &config.catalog {
        SpccCatalog::BuiltinBpRp => "Built-in Bp-Rp".into(),
        SpccCatalog::GaiaDr3Tap => "Gaia DR3 (VizieR)".into(),
    };

    Ok(SpccResult {
        r_factor,
        g_factor,
        b_factor,
        stars_matched: matched.len(),
        stars_total: good_stars.len(),
        avg_color_index: avg_ci,
        white_ref_name,
        catalog_name,
        is_synthetic_catalog: is_synthetic,
    })
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

fn planck_rgb(teff: f64) -> (f64, f64, f64) {
    let r = planck_intensity(teff, 640.0);
    let g = planck_intensity(teff, 530.0);
    let b = planck_intensity(teff, 460.0);

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

fn white_reference_rgb(wr: &WhiteReference) -> (f64, f64, f64) {
    match wr {
        WhiteReference::G2V => planck_rgb(5778.0),
        WhiteReference::AverageSpiral => {
            let (r, g, b) = planck_rgb(5500.0);
            (r * 0.98, g * 1.0, b * 1.02)
        }
        WhiteReference::Photopic => (1.0, 1.0, 1.0),
        WhiteReference::Custom(r, g, b) => (*r, *g, *b),
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
                gmag: None,
            }
        })
        .collect()
}

fn estimate_bp_rp_from_flux(star: &DetectedStar) -> f64 {
    let norm_flux = (star.flux / star.peak.max(1e-10)).clamp(0.1, 100.0);
    let fwhm_factor = (star.fwhm - 3.0).clamp(-2.0, 5.0) * 0.1;
    (1.0 / norm_flux.sqrt() + fwhm_factor).clamp(-0.3, 4.0)
}

#[cfg(not(feature = "vizier"))]
pub(crate) fn query_gaia_vizier(
    _ra_center: f64,
    _dec_center: f64,
    _radius_deg: f64,
    _min_stars: usize,
) -> Result<Vec<CatalogStar>, String> {
    Err("Gaia DR3 query requires the 'vizier' feature. Using built-in Bp-Rp estimation.".into())
}

#[cfg(feature = "vizier")]
pub(crate) fn query_gaia_vizier(
    ra_center: f64,
    dec_center: f64,
    radius_deg: f64,
    min_stars: usize,
) -> Result<Vec<CatalogStar>, String> {
    let radius = radius_deg.clamp(0.01, 5.0);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("AstroBurst/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("HTTP client init failed: {}", e))?;

    let response = client
        .get("https://vizier.cds.unistra.fr/viz-bin/asu-tsv")
        .query(&[
            ("-source", "I/355/gaiadr3"),
            ("-c", format!("{:.6} {:+.6}", ra_center, dec_center).as_str()),
            ("-c.r", format!("{:.4}", radius).as_str()),
            ("-c.u", "deg"),
            ("-out", "RA_ICRS,DE_ICRS,BP-RP,Gmag"),
            ("-out.max", "500"),
            ("-sort", "Gmag"),
            ("Gmag", "<17"),
        ])
        .send()
        .map_err(|e| format!("VizieR request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("VizieR returned HTTP {}", response.status()));
    }

    let body = response
        .text()
        .map_err(|e| format!("VizieR response read failed: {}", e))?;

    let stars = parse_vizier_tsv(&body);
    if stars.len() < min_stars {
        return Err(format!(
            "VizieR returned only {} usable stars within {:.2} deg",
            stars.len(),
            radius
        ));
    }

    log::info!(
        "Gaia DR3 via VizieR: {} stars within {:.2} deg of ({:.4}, {:+.4})",
        stars.len(),
        radius,
        ra_center,
        dec_center
    );

    Ok(stars)
}

#[cfg_attr(not(feature = "vizier"), allow(dead_code))]
fn parse_vizier_tsv(body: &str) -> Vec<CatalogStar> {
    let mut col_ra: Option<usize> = None;
    let mut col_dec: Option<usize> = None;
    let mut col_color: Option<usize> = None;
    let mut col_gmag: Option<usize> = None;
    let mut in_data = false;
    let mut out = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = trimmed.split('\t').map(|f| f.trim()).collect();

        if !in_data {
            let is_dashes = fields
                .iter()
                .all(|f| !f.is_empty() && f.chars().all(|c| c == '-'));
            if is_dashes {
                if col_ra.is_none() || col_dec.is_none() || col_color.is_none() {
                    return out;
                }
                in_data = true;
            } else if col_ra.is_none() {
                col_ra = fields.iter().position(|f| *f == "RA_ICRS");
                col_dec = fields.iter().position(|f| *f == "DE_ICRS");
                col_color = fields.iter().position(|f| *f == "BP-RP");
                col_gmag = fields.iter().position(|f| *f == "Gmag");
            }
            continue;
        }

        let (Some(ir), Some(id), Some(ic)) = (col_ra, col_dec, col_color) else {
            break;
        };
        if fields.len() <= ir.max(id).max(ic) {
            continue;
        }

        let (Ok(ra), Ok(dec), Ok(bp_rp)) = (
            fields[ir].parse::<f64>(),
            fields[id].parse::<f64>(),
            fields[ic].parse::<f64>(),
        ) else {
            continue;
        };

        if !(ra.is_finite() && dec.is_finite() && bp_rp.is_finite()) {
            continue;
        }
        if !(0.0..360.0).contains(&ra) || !(-90.0..=90.0).contains(&dec) {
            continue;
        }

        let gmag = col_gmag
            .and_then(|ig| fields.get(ig))
            .and_then(|f| f.parse::<f64>().ok())
            .filter(|v| v.is_finite());

        out.push(CatalogStar { ra, dec, bp_rp, gmag });
    }

    out
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
    let inner_annulus = radius * 1.2;
    let outer_annulus = radius * 1.8;
    let inner_r2 = inner_annulus * inner_annulus;
    let outer_r2 = outer_annulus * outer_annulus;
    let mut flux = 0.0f64;
    let mut aperture_count = 0u32;
    let mut bg_sum = 0.0f64;
    let mut bg_count = 0u32;

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
                bg_sum += v;
                bg_count += 1;
            }
        }
    }

    if bg_count > 0 && aperture_count > 0 {
        let bg_per_pixel = bg_sum / bg_count as f64;
        flux -= bg_per_pixel * aperture_count as f64;
    }

    flux.max(0.0)
}

fn compute_correction_factors(
    matched: &[MatchedStar],
    wr_r: f64,
    wr_g: f64,
    wr_b: f64,
) -> (f64, f64, f64, f64) {
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
        let (expected_r, expected_g, expected_b) = planck_rgb(teff);

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

    if sum_weight_r < 1e-10 || sum_weight_g < 1e-10 || sum_weight_b < 1e-10 {
        return (1.0, 1.0, 1.0, 0.0);
    }

    let mut r_factor = sum_ratio_r / sum_weight_r;
    let mut g_factor = sum_ratio_g / sum_weight_g;
    let mut b_factor = sum_ratio_b / sum_weight_b;

    r_factor /= wr_r.max(1e-10);
    g_factor /= wr_g.max(1e-10);
    b_factor /= wr_b.max(1e-10);

    let norm = g_factor;
    if norm > 1e-10 {
        r_factor /= norm;
        g_factor = 1.0;
        b_factor /= norm;
    }

    let avg_ci = if ci_count > 0 { sum_ci / ci_count as f64 } else { 0.0 };

    (r_factor, g_factor, b_factor, avg_ci)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vizier_tsv_basic() {
        let body = "#INFO VizieR result\n#Column list\nRA_ICRS\tDE_ICRS\tBP-RP\tGmag\ndeg\tdeg\tmag\tmag\n---------\t---------\t------\t----\n83.633083\t22.014472\t0.650\t8.5\n83.700000\t22.100000\t\t9.0\n84.000000\t21.900000\t1.234\t10.2\n";
        let stars = parse_vizier_tsv(body);
        assert_eq!(stars.len(), 2);
        assert!((stars[0].ra - 83.633083).abs() < 1e-9);
        assert!((stars[0].dec - 22.014472).abs() < 1e-9);
        assert!((stars[0].bp_rp - 0.650).abs() < 1e-9);
        assert!((stars[1].bp_rp - 1.234).abs() < 1e-9);
        assert_eq!(stars[0].gmag, Some(8.5));
        assert_eq!(stars[1].gmag, Some(10.2));
    }

    #[test]
    fn test_parse_vizier_tsv_missing_columns() {
        let body = "RA_ICRS\tDE_ICRS\tGmag\ndeg\tdeg\tmag\n----\t----\t----\n83.6\t22.0\t8.5\n";
        let stars = parse_vizier_tsv(body);
        assert!(stars.is_empty());
    }

    #[test]
    fn test_parse_vizier_tsv_rejects_out_of_range() {
        let body = "RA_ICRS\tDE_ICRS\tBP-RP\n---\t---\t---\n400.0\t22.0\t0.5\n83.6\t95.0\t0.5\n83.6\t22.0\t0.5\n";
        let stars = parse_vizier_tsv(body);
        assert_eq!(stars.len(), 1);
        assert!((stars[0].ra - 83.6).abs() < 1e-9);
    }

    #[test]
    fn test_parse_vizier_tsv_empty_response() {
        assert!(parse_vizier_tsv("").is_empty());
        assert!(parse_vizier_tsv("#nothing here\n#at all\n").is_empty());
    }
}
