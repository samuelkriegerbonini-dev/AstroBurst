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
struct CatalogStar {
    ra: f64,
    dec: f64,
    bp_rp: f64,
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
                && s.x < (w - 10) as f64
                && s.y < (h - 10) as f64
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
            match query_gaia_vizier(center.ra, center.dec, search_radius) {
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

pub fn apply_spcc_factors(
    r: &mut Array2<f32>,
    g: &mut Array2<f32>,
    b: &mut Array2<f32>,
    result: &SpccResult,
) {
    let rf = result.r_factor as f32;
    let gf = result.g_factor as f32;
    let bf = result.b_factor as f32;

    r.mapv_inplace(|v| (v * rf).clamp(0.0, 1.0));
    g.mapv_inplace(|v| (v * gf).clamp(0.0, 1.0));
    b.mapv_inplace(|v| (v * bf).clamp(0.0, 1.0));
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
            }
        })
        .collect()
}

fn estimate_bp_rp_from_flux(star: &DetectedStar) -> f64 {
    let norm_flux = (star.flux / star.peak.max(1e-10)).clamp(0.1, 100.0);
    let fwhm_factor = (star.fwhm - 3.0).clamp(-2.0, 5.0) * 0.1;
    (1.0 / norm_flux.sqrt() + fwhm_factor).clamp(-0.3, 4.0)
}

fn query_gaia_vizier(_ra_center: f64, _dec_center: f64, _radius_deg: f64) -> Result<Vec<CatalogStar>, String> {
    #[allow(unexpected_cfgs)]
    #[cfg(feature = "vizier")]
    {
        let url = format!(
            "https://vizier.cds.unistra.fr/viz-bin/votable/-A?-source=I/355/gaiadr3&-c={:.6}%20{:+.6}&-c.rd={:.4}&-out=RA_ICRS,DE_ICRS,BP-RP&-out.max=500&BP-RP=!null&-sort=Gmag",
            _ra_center, _dec_center, _radius_deg,
        );
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?;
        let resp = client.get(&url).send()
            .map_err(|e| format!("VizieR request failed: {}", e))?;
        let body = resp.text()
            .map_err(|e| format!("Failed to read VizieR response: {}", e))?;
        return parse_votable_bprp(&body);
    }

    #[allow(unreachable_code)]
    Err("Gaia DR3 TAP requires 'vizier' feature. Using built-in Bp-Rp estimation.".into())
}

#[allow(dead_code)]
fn parse_votable_bprp(xml: &str) -> Result<Vec<CatalogStar>, String> {
    let mut stars = Vec::new();

    let mut ra_col: Option<usize> = None;
    let mut dec_col: Option<usize> = None;
    let mut bprp_col: Option<usize> = None;

    let mut in_tabledata = false;
    let mut in_tr = false;
    let mut col_idx = 0;
    let mut current_ra = 0.0f64;
    let mut current_dec = 0.0f64;
    let mut current_bprp = 0.0f64;
    let mut field_idx = 0;

    for line in xml.lines() {
        let trimmed = line.trim();

        if trimmed.contains("<FIELD") {
            if trimmed.contains("RA_ICRS") || trimmed.contains("ra") {
                ra_col = Some(field_idx);
            } else if trimmed.contains("DE_ICRS") || trimmed.contains("dec") {
                dec_col = Some(field_idx);
            } else if trimmed.contains("BP-RP") || trimmed.contains("bp_rp") {
                bprp_col = Some(field_idx);
            }
            field_idx += 1;
        }

        if trimmed.contains("<TABLEDATA") {
            in_tabledata = true;
            continue;
        }
        if trimmed.contains("</TABLEDATA") {
            in_tabledata = false;
        }

        if !in_tabledata {
            continue;
        }

        if trimmed.contains("<TR") {
            in_tr = true;
            col_idx = 0;
            current_ra = f64::NAN;
            current_dec = f64::NAN;
            current_bprp = f64::NAN;
            continue;
        }

        if trimmed.contains("</TR") {
            if in_tr && current_ra.is_finite() && current_dec.is_finite() && current_bprp.is_finite() {
                stars.push(CatalogStar {
                    ra: current_ra,
                    dec: current_dec,
                    bp_rp: current_bprp,
                });
            }
            in_tr = false;
            continue;
        }

        if trimmed.starts_with("<TD") {
            let val = extract_td_value(trimmed);
            if let Ok(v) = val.parse::<f64>() {
                if ra_col == Some(col_idx) {
                    current_ra = v;
                } else if dec_col == Some(col_idx) {
                    current_dec = v;
                } else if bprp_col == Some(col_idx) {
                    current_bprp = v;
                }
            }
            col_idx += 1;
        }
    }

    if stars.is_empty() {
        return Err("No stars parsed from VizieR response".into());
    }

    Ok(stars)
}

#[allow(dead_code)]
fn extract_td_value(line: &str) -> &str {
    let start = line.find('>').map(|i| i + 1).unwrap_or(0);
    let end = line.rfind("</TD").unwrap_or(line.len());
    &line[start..end]
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
            let dra = (wc.ra - cat.ra) * wc.dec.to_radians().cos();
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
            if d2 <= r2 {
                flux += v;
            } else if d2 >= inner_r2 && d2 <= outer_r2 {
                bg_sum += v;
                bg_count += 1;
            }
        }
    }

    if bg_count > 0 {
        let bg_per_pixel = bg_sum / bg_count as f64;
        let aperture_area = std::f64::consts::PI * r2;
        flux -= bg_per_pixel * aperture_area;
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
    let mut sum_weight = 0.0f64;
    let mut sum_ci = 0.0f64;

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
        }
        if mg > 1e-6 {
            sum_ratio_g += (eg / mg) * weight;
        }
        if mb > 1e-6 {
            sum_ratio_b += (eb / mb) * weight;
        }
        sum_weight += weight;
        sum_ci += star.bp_rp;
    }

    if sum_weight < 1e-10 || matched.is_empty() {
        return (1.0, 1.0, 1.0, 0.0);
    }

    let mut r_factor = sum_ratio_r / sum_weight;
    let mut g_factor = sum_ratio_g / sum_weight;
    let mut b_factor = sum_ratio_b / sum_weight;

    r_factor *= wr_r;
    g_factor *= wr_g;
    b_factor *= wr_b;

    let norm = g_factor;
    if norm > 1e-10 {
        r_factor /= norm;
        g_factor = 1.0;
        b_factor /= norm;
    }

    let avg_ci = sum_ci / matched.len() as f64;

    (r_factor, g_factor, b_factor, avg_ci)
}
