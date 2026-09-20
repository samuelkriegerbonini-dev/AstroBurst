use std::sync::{Arc, LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::core::astrometry::spectral::mid_exposure_jd;
use crate::core::astrometry::time::JD_J2000;
use crate::core::astrometry::wcs::angular_separation;
use crate::math::exact_median_f64;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::header::HduHeader;

pub const GAIA_REFERENCE_EPOCH_YEAR: f64 = 2016.0;
pub const DEFAULT_MAX_ROWS: usize = 5000;
pub const CATALOG_CACHE_CAPACITY: usize = 16;
pub const ZERO_POINT_CLIP_SIGMA: f64 = 3.0;
pub const ZERO_POINT_CLIP_ITERATIONS: usize = 5;
pub const GAIA_VIZIER_TABLE: &str = "I/355/gaiadr3";
pub const GAIA_TSV_COLUMNS: &str = "Source,RA_ICRS,DE_ICRS,pmRA,pmDE,Plx,Gmag,BPmag,RPmag,BP-RP";
pub const VIZIER_ASU_TSV_URL: &str = "https://vizier.cds.unistra.fr/viz-bin/asu-tsv";
pub const VIZIER_TIMEOUT_SECS: u64 = 30;
pub const MIN_CONE_RADIUS_DEG: f64 = 0.001;
pub const MAX_CONE_RADIUS_DEG: f64 = 5.0;

const MIN_FIT_SAMPLES_WITH_COLOUR: usize = 3;
const DAYS_PER_JULIAN_YEAR: f64 = 365.25;
const MAS_PER_DEGREE: f64 = 3_600_000.0;
const ARCSEC_PER_DEGREE: f64 = 3600.0;
const CACHE_POSITION_QUANTUM_DEG: f64 = 1e-4;
const CACHE_MAG_QUANTUM: f64 = 1e-2;
const SIGMA_FLOOR_MAG: f64 = 1e-6;
const DEGENERATE_COLOUR_SPREAD: f64 = 1e-10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CatalogRow {
    pub id: String,
    pub ra: f64,
    pub dec: f64,
    pub ra_epoch: f64,
    pub dec_epoch: f64,
    pub pm_ra_masyr: Option<f64>,
    pub pm_dec_masyr: Option<f64>,
    pub g: Option<f64>,
    pub bp: Option<f64>,
    pub rp: Option<f64>,
    pub bp_rp: Option<f64>,
    pub parallax_mas: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConeQuery {
    pub ra: f64,
    pub dec: f64,
    pub radius_deg: f64,
    pub mag_limit: Option<f64>,
    pub max_rows: usize,
}

impl ConeQuery {
    pub fn new(ra: f64, dec: f64, radius_deg: f64) -> Self {
        Self {
            ra,
            dec,
            radius_deg,
            mag_limit: None,
            max_rows: DEFAULT_MAX_ROWS,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CrossMatch {
    pub star_index: usize,
    pub row_index: usize,
    pub sep_arcsec: f64,
    pub d_ra_arcsec: f64,
    pub d_dec_arcsec: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ZeroPointFit {
    pub zp: f64,
    pub zp_err: f64,
    pub colour_coeff: Option<f64>,
    pub rms: f64,
    pub n_used: usize,
    pub n_rejected: usize,
    pub n_without_colour: usize,
    pub band: String,
    pub colour_term_used: bool,
}

struct ColumnMap {
    ra: usize,
    dec: usize,
    id: Option<usize>,
    pm_ra: Option<usize>,
    pm_dec: Option<usize>,
    parallax: Option<usize>,
    g: Option<usize>,
    bp: Option<usize>,
    rp: Option<usize>,
    bp_rp: Option<usize>,
}

fn split_fields(line: &str) -> Vec<&str> {
    line.split('\t').map(str::trim).collect()
}

fn is_rule_line(fields: &[&str]) -> bool {
    !fields.is_empty() && fields.iter().all(|f| !f.is_empty() && f.chars().all(|c| c == '-'))
}

fn column_map(fields: &[&str]) -> Result<ColumnMap, String> {
    let find = |name: &str| fields.iter().position(|f| *f == name);
    let ra = find("RA_ICRS").ok_or("VizieR header line has no RA_ICRS column")?;
    let dec = find("DE_ICRS").ok_or("VizieR header line has no DE_ICRS column")?;
    Ok(ColumnMap {
        ra,
        dec,
        id: find("Source"),
        pm_ra: find("pmRA"),
        pm_dec: find("pmDE"),
        parallax: find("Plx"),
        g: find("Gmag"),
        bp: find("BPmag"),
        rp: find("RPmag"),
        bp_rp: find("BP-RP"),
    })
}

fn cell_f64(fields: &[&str], col: Option<usize>) -> Option<f64> {
    col.and_then(|c| fields.get(c))
        .and_then(|f| f.parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

fn fallback_id(ra: f64, dec: f64) -> String {
    format!("J{:.5}{:+.5}", ra, dec)
}

pub fn parse_gaia_tsv(body: &str) -> Result<Vec<CatalogRow>, String> {
    let mut columns: Option<ColumnMap> = None;
    let mut rows = Vec::new();
    for raw in body.lines() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = split_fields(line);
        let Some(map) = &columns else {
            columns = Some(column_map(&fields)?);
            continue;
        };
        if is_rule_line(&fields) {
            continue;
        }
        let (Some(ra), Some(dec)) = (cell_f64(&fields, Some(map.ra)), cell_f64(&fields, Some(map.dec))) else {
            continue;
        };
        if !(0.0..360.0).contains(&ra) || !(-90.0..=90.0).contains(&dec) {
            continue;
        }
        let id = map
            .id
            .and_then(|c| fields.get(c))
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback_id(ra, dec));
        let bp = cell_f64(&fields, map.bp);
        let rp = cell_f64(&fields, map.rp);
        let bp_rp = cell_f64(&fields, map.bp_rp).or_else(|| match (bp, rp) {
            (Some(b), Some(r)) => Some(b - r),
            _ => None,
        });
        rows.push(CatalogRow {
            id,
            ra,
            dec,
            ra_epoch: ra,
            dec_epoch: dec,
            pm_ra_masyr: cell_f64(&fields, map.pm_ra),
            pm_dec_masyr: cell_f64(&fields, map.pm_dec),
            g: cell_f64(&fields, map.g),
            bp,
            rp,
            bp_rp,
            parallax_mas: cell_f64(&fields, map.parallax),
        });
    }
    Ok(rows)
}

pub fn propagate_epoch(rows: &mut [CatalogRow], epoch_year: f64) {
    let dt_years = epoch_year - GAIA_REFERENCE_EPOCH_YEAR;
    for row in rows.iter_mut() {
        row.ra_epoch = row.ra;
        row.dec_epoch = row.dec;
        let (Some(pm_ra), Some(pm_dec)) = (row.pm_ra_masyr, row.pm_dec_masyr) else {
            continue;
        };
        if !pm_ra.is_finite() || !pm_dec.is_finite() || !dt_years.is_finite() {
            continue;
        }
        let cos_dec = row.dec.to_radians().cos();
        let d_ra_deg = if cos_dec.abs() > 1e-12 {
            pm_ra * dt_years / MAS_PER_DEGREE / cos_dec
        } else {
            0.0
        };
        row.ra_epoch = (row.ra + d_ra_deg).rem_euclid(360.0);
        row.dec_epoch = (row.dec + pm_dec * dt_years / MAS_PER_DEGREE).clamp(-90.0, 90.0);
    }
}

pub fn julian_epoch_year(jd: f64) -> f64 {
    2000.0 + (jd - JD_J2000) / DAYS_PER_JULIAN_YEAR
}

pub fn observation_epoch_year(header: &HduHeader) -> Option<f64> {
    mid_exposure_jd(header)
        .map(|(jd, _)| julian_epoch_year(jd))
        .filter(|year| year.is_finite())
}

fn residual_arcsec(star_ra: f64, star_dec: f64, row_ra: f64, row_dec: f64) -> (f64, f64) {
    let mut d_ra = star_ra - row_ra;
    if d_ra > 180.0 {
        d_ra -= 360.0;
    } else if d_ra < -180.0 {
        d_ra += 360.0;
    }
    let cos_dec = ((star_dec + row_dec) / 2.0).to_radians().cos();
    (d_ra * cos_dec * ARCSEC_PER_DEGREE, (star_dec - row_dec) * ARCSEC_PER_DEGREE)
}

pub fn cross_match(stars: &[(f64, f64)], rows: &[CatalogRow], radius_arcsec: f64) -> Vec<CrossMatch> {
    if stars.is_empty() || rows.is_empty() || !(radius_arcsec > 0.0) {
        return Vec::new();
    }
    let radius_deg = radius_arcsec / ARCSEC_PER_DEGREE;
    let mut order: Vec<usize> = (0..rows.len())
        .filter(|&i| rows[i].ra_epoch.is_finite() && rows[i].dec_epoch.is_finite())
        .collect();
    order.sort_by(|&a, &b| rows[a].dec_epoch.total_cmp(&rows[b].dec_epoch));
    let sorted_decs: Vec<f64> = order.iter().map(|&i| rows[i].dec_epoch).collect();

    let mut claims: Vec<CrossMatch> = Vec::new();
    for (star_index, &(ra, dec)) in stars.iter().enumerate() {
        if !ra.is_finite() || !dec.is_finite() {
            continue;
        }
        let start = sorted_decs.partition_point(|&d| d < dec - radius_deg);
        let mut best: Option<CrossMatch> = None;
        for k in start..sorted_decs.len() {
            if sorted_decs[k] > dec + radius_deg {
                break;
            }
            let row_index = order[k];
            let row = &rows[row_index];
            let sep_arcsec = angular_separation(ra, dec, row.ra_epoch, row.dec_epoch) * ARCSEC_PER_DEGREE;
            if sep_arcsec > radius_arcsec || best.as_ref().is_some_and(|b| sep_arcsec >= b.sep_arcsec) {
                continue;
            }
            let (d_ra_arcsec, d_dec_arcsec) = residual_arcsec(ra, dec, row.ra_epoch, row.dec_epoch);
            best = Some(CrossMatch {
                star_index,
                row_index,
                sep_arcsec,
                d_ra_arcsec,
                d_dec_arcsec,
            });
        }
        if let Some(m) = best {
            claims.push(m);
        }
    }

    claims.sort_by(|a, b| a.sep_arcsec.total_cmp(&b.sep_arcsec));
    let mut claimed = vec![false; rows.len()];
    let mut matches: Vec<CrossMatch> = claims
        .into_iter()
        .filter(|m| {
            if claimed[m.row_index] {
                return false;
            }
            claimed[m.row_index] = true;
            true
        })
        .collect();
    matches.sort_by_key(|m| m.star_index);
    matches
}

fn robust_sigma(residuals: &[f64]) -> f64 {
    if residuals.is_empty() {
        return 0.0;
    }
    let centre = exact_median_f64(residuals);
    let deviations: Vec<f64> = residuals.iter().map(|r| (r - centre).abs()).collect();
    let robust = exact_median_f64(&deviations) * MAD_TO_SIGMA;
    if robust > 0.0 {
        return robust;
    }
    if residuals.len() < 2 {
        return 0.0;
    }
    let mean = residuals.iter().sum::<f64>() / residuals.len() as f64;
    let var = residuals.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (residuals.len() - 1) as f64;
    var.sqrt()
}

fn root_mean_square(residuals: &[f64]) -> f64 {
    if residuals.is_empty() {
        return 0.0;
    }
    (residuals.iter().map(|r| r * r).sum::<f64>() / residuals.len() as f64).sqrt()
}

fn linear_fit(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    let sx: f64 = points.iter().map(|(_, x)| x).sum();
    let sy: f64 = points.iter().map(|(y, _)| y).sum();
    let sxx: f64 = points.iter().map(|(_, x)| x * x).sum();
    let sxy: f64 = points.iter().map(|(y, x)| x * y).sum();
    let det = n * sxx - sx * sx;
    if !(det > DEGENERATE_COLOUR_SPREAD * (n * sxx).abs().max(f64::MIN_POSITIVE)) {
        return None;
    }
    let slope = (n * sxy - sx * sy) / det;
    let intercept = (sy - slope * sx) / n;
    Some((intercept, slope))
}

fn fit_with_colour(points: &[(f64, f64)], band: &str) -> Option<ZeroPointFit> {
    let mut kept: Vec<(f64, f64)> = points.to_vec();
    let (mut zp, mut coeff) = linear_fit(&kept)?;
    let mut n_rejected = 0usize;
    for _ in 0..ZERO_POINT_CLIP_ITERATIONS {
        let residuals: Vec<f64> = kept.iter().map(|(d, x)| d - zp - coeff * x).collect();
        let sigma = robust_sigma(&residuals);
        if sigma <= SIGMA_FLOOR_MAG {
            break;
        }
        let limit = ZERO_POINT_CLIP_SIGMA * sigma;
        let filtered: Vec<(f64, f64)> = kept
            .iter()
            .zip(&residuals)
            .filter(|(_, r)| r.abs() <= limit)
            .map(|(p, _)| *p)
            .collect();
        if filtered.len() == kept.len() || filtered.len() < MIN_FIT_SAMPLES_WITH_COLOUR {
            break;
        }
        let Some((next_zp, next_coeff)) = linear_fit(&filtered) else {
            break;
        };
        n_rejected += kept.len() - filtered.len();
        kept = filtered;
        zp = next_zp;
        coeff = next_coeff;
    }
    let residuals: Vec<f64> = kept.iter().map(|(d, x)| d - zp - coeff * x).collect();
    let rms = root_mean_square(&residuals);
    Some(ZeroPointFit {
        zp,
        zp_err: rms / (kept.len() as f64).sqrt(),
        colour_coeff: Some(coeff),
        rms,
        n_used: kept.len(),
        n_rejected,
        n_without_colour: 0,
        band: band.to_string(),
        colour_term_used: true,
    })
}

fn fit_median(diffs: &[f64], band: &str) -> Option<ZeroPointFit> {
    if diffs.is_empty() {
        return None;
    }
    let mut kept: Vec<f64> = diffs.to_vec();
    let mut zp = exact_median_f64(&kept);
    let mut n_rejected = 0usize;
    for _ in 0..ZERO_POINT_CLIP_ITERATIONS {
        let residuals: Vec<f64> = kept.iter().map(|d| d - zp).collect();
        let sigma = robust_sigma(&residuals);
        if sigma <= SIGMA_FLOOR_MAG {
            break;
        }
        let limit = ZERO_POINT_CLIP_SIGMA * sigma;
        let before = kept.len();
        kept.retain(|d| (d - zp).abs() <= limit);
        if kept.len() == before {
            break;
        }
        n_rejected += before - kept.len();
        zp = exact_median_f64(&kept);
    }
    let residuals: Vec<f64> = kept.iter().map(|d| d - zp).collect();
    let rms = root_mean_square(&residuals);
    Some(ZeroPointFit {
        zp,
        zp_err: rms / (kept.len() as f64).sqrt(),
        colour_coeff: None,
        rms,
        n_used: kept.len(),
        n_rejected,
        n_without_colour: 0,
        band: band.to_string(),
        colour_term_used: false,
    })
}

pub fn fit_zero_point(
    mag_inst: &[f64],
    cat_mag: &[f64],
    colour: &[Option<f64>],
    use_colour_term: bool,
    band: &str,
) -> Option<ZeroPointFit> {
    let n = mag_inst.len().min(cat_mag.len());
    let samples: Vec<(f64, Option<f64>)> = (0..n)
        .filter_map(|i| {
            let diff = cat_mag[i] - mag_inst[i];
            diff.is_finite()
                .then(|| (diff, colour.get(i).copied().flatten().filter(|c| c.is_finite())))
        })
        .collect();
    if use_colour_term {
        let with_colour: Vec<(f64, f64)> = samples.iter().filter_map(|(d, c)| c.map(|c| (*d, c))).collect();
        if with_colour.len() >= MIN_FIT_SAMPLES_WITH_COLOUR {
            if let Some(mut fit) = fit_with_colour(&with_colour, band) {
                fit.n_without_colour = samples.len() - with_colour.len();
                return Some(fit);
            }
        }
    }
    let diffs: Vec<f64> = samples.iter().map(|(d, _)| *d).collect();
    fit_median(&diffs, band)
}

pub type CatalogCacheKey = (i64, i64, i64, Option<i64>, usize);

static CATALOG_CACHE: LazyLock<Mutex<Vec<(CatalogCacheKey, Arc<Vec<CatalogRow>>)>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

fn quantize(value: f64, quantum: f64) -> i64 {
    (value / quantum).round() as i64
}

pub fn catalog_cache_key(q: &ConeQuery) -> CatalogCacheKey {
    (
        quantize(q.ra, CACHE_POSITION_QUANTUM_DEG),
        quantize(q.dec, CACHE_POSITION_QUANTUM_DEG),
        quantize(q.radius_deg, CACHE_POSITION_QUANTUM_DEG),
        q.mag_limit.filter(|m| m.is_finite()).map(|m| quantize(m, CACHE_MAG_QUANTUM)),
        q.max_rows,
    )
}

fn cached_rows(key: &CatalogCacheKey) -> Option<Arc<Vec<CatalogRow>>> {
    let mut cache = CATALOG_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let pos = cache.iter().position(|(k, _)| k == key)?;
    let entry = cache.remove(pos);
    let rows = Arc::clone(&entry.1);
    cache.push(entry);
    Some(rows)
}

fn store_rows(key: CatalogCacheKey, rows: Vec<CatalogRow>) -> Arc<Vec<CatalogRow>> {
    let rows = Arc::new(rows);
    let mut cache = CATALOG_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.retain(|(k, _)| *k != key);
    while cache.len() >= CATALOG_CACHE_CAPACITY {
        cache.remove(0);
    }
    cache.push((key, Arc::clone(&rows)));
    rows
}

pub fn query_gaia_cached_with<F>(q: &ConeQuery, fetch: F) -> Result<Arc<Vec<CatalogRow>>, String>
where
    F: FnOnce(&ConeQuery) -> Result<Vec<CatalogRow>, String>,
{
    let key = catalog_cache_key(q);
    if let Some(rows) = cached_rows(&key) {
        return Ok(rows);
    }
    let rows = fetch(q)?;
    Ok(store_rows(key, rows))
}

pub fn query_gaia_cached(q: &ConeQuery) -> Result<Arc<Vec<CatalogRow>>, String> {
    query_gaia_cached_with(q, query_gaia)
}

pub fn vizier_query_params(q: &ConeQuery) -> Vec<(&'static str, String)> {
    let radius = q.radius_deg.clamp(MIN_CONE_RADIUS_DEG, MAX_CONE_RADIUS_DEG);
    let mut params = vec![
        ("-source", GAIA_VIZIER_TABLE.to_string()),
        ("-c", format!("{:.6} {:+.6}", q.ra, q.dec)),
        ("-c.r", format!("{:.4}", radius)),
        ("-c.u", "deg".to_string()),
        ("-out", GAIA_TSV_COLUMNS.to_string()),
        ("-out.max", q.max_rows.max(1).to_string()),
        ("-sort", "Gmag".to_string()),
    ];
    if let Some(limit) = q.mag_limit.filter(|m| m.is_finite()) {
        params.push(("Gmag", format!("<{:.2}", limit)));
    }
    params
}

#[cfg(not(feature = "vizier"))]
pub fn query_gaia(_q: &ConeQuery) -> Result<Vec<CatalogRow>, String> {
    Err("Gaia DR3 query requires the 'vizier' feature".into())
}

#[cfg(feature = "vizier")]
pub fn query_gaia(q: &ConeQuery) -> Result<Vec<CatalogRow>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(VIZIER_TIMEOUT_SECS))
        .user_agent(concat!("AstroBurst/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("HTTP client init failed: {}", e))?;

    let params = vizier_query_params(q);
    let response = client
        .get(VIZIER_ASU_TSV_URL)
        .query(&params)
        .send()
        .map_err(|e| format!("VizieR request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("VizieR returned HTTP {}", response.status()));
    }

    let body = response
        .text()
        .map_err(|e| format!("VizieR response read failed: {}", e))?;
    let rows = parse_gaia_tsv(&body)?;
    log::info!(
        "Gaia DR3 via VizieR: {} rows within {:.3} deg of ({:.4}, {:+.4})",
        rows.len(),
        q.radius_deg,
        q.ra,
        q.dec
    );
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::time::jd_from_mjd;

    fn row(id: &str, ra: f64, dec: f64) -> CatalogRow {
        CatalogRow {
            id: id.to_string(),
            ra,
            dec,
            ra_epoch: ra,
            dec_epoch: dec,
            pm_ra_masyr: None,
            pm_dec_masyr: None,
            g: None,
            bp: None,
            rp: None,
            bp_rp: None,
            parallax_mas: None,
        }
    }

    fn header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in pairs {
            h.set(k, v.to_string());
        }
        h
    }

    const ASU_TSV: &str = "#\n#   VizieR Astronomical Server vizier.cds.unistra.fr\n#INFO votable-version=1.99+ (14-Oct-2013)\n\
Gmag\tRA_ICRS\tDE_ICRS\tSource\tpmRA\tpmDE\tPlx\tBPmag\tRPmag\tBP-RP\n\
mag\tdeg\tdeg\t\tmas/yr\tmas/yr\tmas\tmag\tmag\tmag\n\
------\t------------\t------------\t-------------------\t--------\t--------\t-------\t------\t------\t------\n\
8.512\t83.633083\t22.014472\t3403818172572314624\t1.20\t-3.40\t2.50\t8.90\t7.95\t0.950\n\
12.100\t83.700000\t22.100000\t3403818172572314625\t\t\t\t12.60\t11.40\t\n\
15.000\t84.000000\t21.900000\t\t-5.5\t7.25\t\t\t\t\n";

    #[test]
    fn parse_gaia_tsv_maps_columns_by_name_and_tolerates_blank_cells() {
        let rows = parse_gaia_tsv(ASU_TSV).unwrap();
        assert_eq!(rows.len(), 3);

        let full = &rows[0];
        assert_eq!(full.id, "3403818172572314624");
        assert!((full.ra - 83.633083).abs() < 1e-9);
        assert!((full.dec - 22.014472).abs() < 1e-9);
        assert_eq!(full.ra_epoch, full.ra);
        assert_eq!(full.dec_epoch, full.dec);
        assert_eq!(full.pm_ra_masyr, Some(1.20));
        assert_eq!(full.pm_dec_masyr, Some(-3.40));
        assert_eq!(full.parallax_mas, Some(2.50));
        assert_eq!(full.g, Some(8.512));
        assert_eq!(full.bp, Some(8.90));
        assert_eq!(full.rp, Some(7.95));
        assert_eq!(full.bp_rp, Some(0.950));

        let no_pm = &rows[1];
        assert_eq!(no_pm.pm_ra_masyr, None);
        assert_eq!(no_pm.pm_dec_masyr, None);
        assert_eq!(no_pm.parallax_mas, None);
        assert_eq!(no_pm.g, Some(12.1));
        let derived = no_pm.bp_rp.expect("BP-RP derived from BPmag and RPmag when the column is blank");
        assert!((derived - 1.2).abs() < 1e-9);

        let no_id = &rows[2];
        assert!(!no_id.id.is_empty(), "blank Source must still give a usable id");
        assert_eq!(no_id.pm_ra_masyr, Some(-5.5));
        assert_eq!(no_id.pm_dec_masyr, Some(7.25));
        assert_eq!(no_id.bp_rp, None);
        assert_eq!(no_id.bp, None);
    }

    #[test]
    fn parse_gaia_tsv_requires_position_columns_in_the_header_line() {
        let err = parse_gaia_tsv("RA_ICRS\tGmag\n---\t---\n83.6\t8.5\n").unwrap_err();
        assert!(err.contains("DE_ICRS"), "{err}");
    }

    #[test]
    fn parse_gaia_tsv_returns_no_rows_for_a_body_without_a_table() {
        assert_eq!(parse_gaia_tsv("").unwrap(), Vec::<CatalogRow>::new());
        assert_eq!(parse_gaia_tsv("   \n\r\n\t\n").unwrap(), Vec::<CatalogRow>::new());
        let comments_only = "#\n#   VizieR Astronomical Server vizier.cds.unistra.fr\n#INFO status=OK\n#INFO -out.max=5000\n\n#END#\n";
        assert_eq!(parse_gaia_tsv(comments_only).unwrap(), Vec::<CatalogRow>::new());
    }

    #[test]
    fn parse_gaia_tsv_rejects_data_rows_without_a_header_line() {
        let err = parse_gaia_tsv("#INFO status=OK\n83.633083\t22.014472\t8.512\n").unwrap_err();
        assert!(err.contains("RA_ICRS"), "{err}");
        assert!(parse_gaia_tsv("83.6\t22.0\n84.0\t21.9\n").is_err());
    }

    #[test]
    fn parse_gaia_tsv_skips_out_of_range_and_unparsable_rows() {
        let body = "RA_ICRS\tDE_ICRS\tGmag\n---\t---\t---\n400.0\t22.0\t8.0\n83.6\t95.0\t8.0\nabc\t22.0\t8.0\n83.6\t22.0\tnot-a-mag\n";
        let rows = parse_gaia_tsv(body).unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].ra - 83.6).abs() < 1e-9);
        assert_eq!(rows[0].g, None);
    }

    #[test]
    fn propagate_epoch_moves_a_100_mas_per_year_star_by_one_arcsecond_over_ten_years() {
        let mut along_ra = row("ra", 150.0, 60.0);
        along_ra.pm_ra_masyr = Some(100.0);
        along_ra.pm_dec_masyr = Some(0.0);
        let mut along_dec = row("dec", 150.0, 60.0);
        along_dec.pm_ra_masyr = Some(0.0);
        along_dec.pm_dec_masyr = Some(100.0);
        let fixed = row("fixed", 150.0, 60.0);
        let mut rows = vec![along_ra, along_dec, fixed];

        propagate_epoch(&mut rows, GAIA_REFERENCE_EPOCH_YEAR + 10.0);

        let moved = angular_separation(150.0, 60.0, rows[0].ra_epoch, rows[0].dec_epoch) * 3600.0;
        assert!((moved - 1.0).abs() < 1e-6, "sky motion along RA: {moved} arcsec");
        assert!(rows[0].ra_epoch > 150.0, "positive pmRA increases RA");
        assert!((rows[0].ra_epoch - 150.0) * 3600.0 > 1.9, "RA coordinate change carries the 1/cos(dec) factor");
        assert_eq!(rows[0].ra, 150.0, "catalog-epoch position is kept");

        let d_dec = (rows[1].dec_epoch - 60.0) * 3600.0;
        assert!((d_dec - 1.0).abs() < 1e-9, "dec motion: {d_dec} arcsec");
        assert_eq!(rows[1].ra_epoch, 150.0);

        assert_eq!(rows[2].ra_epoch, 150.0);
        assert_eq!(rows[2].dec_epoch, 60.0);

        propagate_epoch(&mut rows, GAIA_REFERENCE_EPOCH_YEAR);
        assert_eq!(rows[0].ra_epoch, 150.0);
        assert_eq!(rows[1].dec_epoch, 60.0);
    }

    #[test]
    fn observation_epoch_year_reads_date_obs_and_mjd_cards() {
        let march = header(&[("DATE-OBS", "'2026-03-21'")]);
        let year = observation_epoch_year(&march).unwrap();
        assert!((year - 2026.22).abs() < 0.01, "{year}");

        let mjd = header(&[("MJD-AVG", "60000.0"), ("DATE-OBS", "'2000-01-01'")]);
        let expected = julian_epoch_year(jd_from_mjd(60000.0));
        assert_eq!(observation_epoch_year(&mjd).unwrap(), expected);
        assert!((expected - 2023.149).abs() < 0.001);

        assert_eq!(julian_epoch_year(JD_J2000), 2000.0);
        assert!(observation_epoch_year(&header(&[("OBJECT", "'M42'")])).is_none());
    }

    #[test]
    fn cross_match_is_one_to_one_and_honours_the_radius() {
        let arcsec = 1.0 / 3600.0;
        let row_b = row("B", 150.0, 2.0 + 3.6 * arcsec);
        let row_a = row("A", 150.0, 2.0);
        let row_far = row("far", 151.0, 2.0);
        let rows = vec![row_far, row_b, row_a];
        let cos_dec = 2.0f64.to_radians().cos();
        let stars = vec![
            (150.0 + 0.3 * arcsec / cos_dec, 2.0),
            (150.0 + 0.6 * arcsec / cos_dec, 2.0),
            (150.0, 2.0 + 3.6 * arcsec + 0.5 * arcsec),
            (152.0, 2.0),
        ];

        let matches = cross_match(&stars, &rows, 2.0);
        assert_eq!(matches.len(), 2, "{matches:?}");
        let first = &matches[0];
        assert_eq!((first.star_index, first.row_index), (0, 2));
        assert!((first.sep_arcsec - 0.3).abs() < 1e-6, "{first:?}");
        assert!((first.d_ra_arcsec - 0.3).abs() < 1e-6, "{first:?}");
        assert!(first.d_dec_arcsec.abs() < 1e-6, "{first:?}");
        let second = &matches[1];
        assert_eq!((second.star_index, second.row_index), (2, 1));
        assert!((second.sep_arcsec - 0.5).abs() < 1e-6);
        assert!((second.d_dec_arcsec - 0.5).abs() < 1e-6);
        assert!(second.d_ra_arcsec.abs() < 1e-6);
        assert!(matches.iter().all(|m| m.star_index != 1), "the farther claimant of row A is dropped");

        let wide = cross_match(&stars, &rows, 5.0);
        assert_eq!(wide.len(), 2);
        assert!(wide.iter().all(|m| m.star_index != 1), "a loser is not reassigned to a farther row");

        let tight = cross_match(&stars, &rows, 0.4);
        assert_eq!(tight.len(), 1);
        assert_eq!(tight[0].star_index, 0);

        assert!(cross_match(&stars, &[], 2.0).is_empty());
        assert!(cross_match(&[], &rows, 2.0).is_empty());
        assert!(cross_match(&stars, &rows, 0.0).is_empty());
    }

    fn synthetic_photometry(n: usize) -> (Vec<f64>, Vec<f64>, Vec<Option<f64>>) {
        let mut inst = Vec::with_capacity(n);
        let mut cat = Vec::with_capacity(n);
        let mut colour = Vec::with_capacity(n);
        for i in 0..n {
            let c = 0.2 + 0.1 * i as f64;
            let catalog_mag = 12.0 + 0.3 * i as f64;
            let noise = if i % 2 == 0 { 0.004 } else { -0.004 };
            let mut instrumental = catalog_mag - 25.0 - 0.1 * c + noise;
            if i == 7 {
                instrumental += 5.0;
            }
            inst.push(instrumental);
            cat.push(catalog_mag);
            colour.push(Some(c));
        }
        (inst, cat, colour)
    }

    #[test]
    fn fit_zero_point_recovers_zp_and_colour_coefficient_with_the_outlier_rejected() {
        let (inst, cat, colour) = synthetic_photometry(20);
        let fit = fit_zero_point(&inst, &cat, &colour, true, "G").unwrap();
        assert!((fit.zp - 25.0).abs() < 0.02, "{fit:?}");
        let coeff = fit.colour_coeff.unwrap();
        assert!((coeff - 0.1).abs() < 0.02, "{fit:?}");
        assert_eq!(fit.n_used, 19);
        assert_eq!(fit.n_rejected, 1);
        assert_eq!(fit.n_without_colour, 0);
        assert!(fit.colour_term_used);
        assert_eq!(fit.band, "G");
        assert!(fit.rms > 0.0 && fit.rms < 0.01, "{fit:?}");
        assert!((fit.zp_err - fit.rms / 19f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn fit_zero_point_counts_matches_dropped_for_a_missing_colour() {
        let (inst, cat, mut colour) = synthetic_photometry(20);
        for (i, c) in colour.iter_mut().enumerate() {
            if !(4..16).contains(&i) {
                *c = None;
            }
        }
        assert_eq!(colour.iter().filter(|c| c.is_none()).count(), 8);

        let fit = fit_zero_point(&inst, &cat, &colour, true, "G").unwrap();
        assert!(fit.colour_term_used);
        assert_eq!(fit.n_without_colour, 8);
        assert_eq!(fit.n_used, 11, "{fit:?}");
        assert_eq!(fit.n_rejected, 1, "{fit:?}");
        assert_eq!(fit.n_used + fit.n_rejected + fit.n_without_colour, 20);
        assert!((fit.zp - 25.0).abs() < 0.02, "{fit:?}");
        assert!((fit.colour_coeff.unwrap() - 0.1).abs() < 0.02, "{fit:?}");

        let median = fit_zero_point(&inst, &cat, &colour, false, "G").unwrap();
        assert!(!median.colour_term_used);
        assert_eq!(median.n_without_colour, 0, "the median fit uses every match");
        assert_eq!(median.n_used + median.n_rejected, 20);

        let too_few: Vec<Option<f64>> = colour.iter().enumerate().map(|(i, c)| if i < 2 { *c } else { None }).collect();
        let fallback = fit_zero_point(&inst, &cat, &too_few, true, "G").unwrap();
        assert!(!fallback.colour_term_used);
        assert_eq!(fallback.n_without_colour, 0, "the fallback median fit drops nothing");
    }

    #[test]
    fn fit_zero_point_without_colour_term_is_a_sigma_clipped_median() {
        let (inst, cat, colour) = synthetic_photometry(20);
        let fit = fit_zero_point(&inst, &cat, &colour, false, "BP").unwrap();
        assert_eq!(fit.colour_coeff, None);
        assert!(!fit.colour_term_used);
        assert_eq!(fit.band, "BP");
        assert_eq!(fit.n_rejected, 1);
        assert_eq!(fit.n_used, 19);
        assert!((fit.zp - 25.115).abs() < 0.08, "{fit:?}");
        assert!(fit.rms < 0.1, "{fit:?}");

        let no_colour = vec![None; inst.len()];
        let fallback = fit_zero_point(&inst, &cat, &no_colour, true, "G").unwrap();
        assert!(!fallback.colour_term_used, "colour term requested but no colours: falls back");
        assert_eq!(fallback.colour_coeff, None);
        assert_eq!(fallback.n_rejected, 1);

        assert!(fit_zero_point(&[], &[], &[], true, "G").is_none());
        let single = fit_zero_point(&[-13.0], &[12.0], &[None], false, "RP").unwrap();
        assert_eq!(single.zp, 25.0);
        assert_eq!(single.n_used, 1);
        assert_eq!(single.n_rejected, 0);
    }

    #[test]
    fn catalog_cache_key_quantizes_position_and_a_hit_skips_the_fetch() {
        let base = ConeQuery::new(359.123456, -89.5, 0.25);
        let nudged = ConeQuery::new(359.123496, -89.50004, 0.25);
        assert_eq!(catalog_cache_key(&base), catalog_cache_key(&nudged));
        let other_radius = ConeQuery { radius_deg: 0.3, ..base.clone() };
        assert_ne!(catalog_cache_key(&base), catalog_cache_key(&other_radius));
        let with_limit = ConeQuery { mag_limit: Some(18.0), ..base.clone() };
        assert_ne!(catalog_cache_key(&base), catalog_cache_key(&with_limit));
        let same_limit = ConeQuery { mag_limit: Some(18.004), ..base.clone() };
        assert_eq!(catalog_cache_key(&with_limit), catalog_cache_key(&same_limit));
        let more_rows = ConeQuery { max_rows: 10, ..base.clone() };
        assert_ne!(catalog_cache_key(&base), catalog_cache_key(&more_rows));

        let probe = ConeQuery::new(359.987654, -89.987, 0.0123);
        let fetches = std::cell::Cell::new(0usize);
        let fetch = |q: &ConeQuery| {
            fetches.set(fetches.get() + 1);
            Ok(vec![row("fetched", q.ra, q.dec)])
        };
        let first = query_gaia_cached_with(&probe, fetch).unwrap();
        assert_eq!(fetches.get(), 1);
        assert_eq!(first.len(), 1);
        let second = query_gaia_cached_with(&probe, |_| Err("must not be called".to_string())).unwrap();
        assert_eq!(fetches.get(), 1);
        assert!(Arc::ptr_eq(&first, &second));

        let failing = ConeQuery::new(359.5, -89.9, 0.0111);
        let err = query_gaia_cached_with(&failing, |_| Err("offline".to_string())).unwrap_err();
        assert_eq!(err, "offline");
        let retried = query_gaia_cached_with(&failing, |_| Ok(vec![])).unwrap();
        assert!(retried.is_empty(), "a failed fetch is not cached");
    }

    #[test]
    fn vizier_query_params_carry_the_mag_limit_and_row_cap() {
        let q = ConeQuery { mag_limit: Some(17.5), max_rows: 1234, ..ConeQuery::new(83.633, 22.014, 0.5) };
        let params = vizier_query_params(&q);
        let get = |k: &str| params.iter().find(|(key, _)| *key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("-source"), Some(GAIA_VIZIER_TABLE));
        assert_eq!(get("-out"), Some(GAIA_TSV_COLUMNS));
        assert_eq!(get("-out.max"), Some("1234"));
        assert_eq!(get("-c.u"), Some("deg"));
        assert_eq!(get("-c.r"), Some("0.5000"));
        assert_eq!(get("-c"), Some("83.633000 +22.014000"));
        assert_eq!(get("Gmag"), Some("<17.50"));

        let open = ConeQuery::new(10.0, -5.0, 9.0);
        let params = vizier_query_params(&open);
        assert!(params.iter().all(|(k, _)| *k != "Gmag"));
        assert_eq!(params.iter().find(|(k, _)| *k == "-c.r").unwrap().1, "5.0000");
    }
}
