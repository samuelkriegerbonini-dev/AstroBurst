use std::time::Instant;

use ndarray::Array2;
use serde::Serialize;
use serde_json::{json, Value};

use crate::cmd::analysis::photometry_planes;
use crate::cmd::common::{blocking_cmd, load_cached_full};
use crate::core::analysis::photometry::{measure_star_full, PhotometryConfig};
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::catalog::{
    cross_match, fit_zero_point, observation_epoch_year, propagate_epoch, query_gaia_cached, CatalogRow,
    ConeQuery, CrossMatch, ZeroPointFit, DEFAULT_MAX_ROWS, MAX_CONE_RADIUS_DEG, MIN_CONE_RADIUS_DEG,
};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::metadata::photcal::PhotCal;
use crate::infra::cache::ImageEntry;
use crate::math::exact_median_f64;
use crate::types::constants::{RES_ELAPSED_MS, RES_OUTPUT_PATH, RES_WARNINGS};
use crate::types::header::HduHeader;

pub const DEFAULT_MATCH_RADIUS_ARCSEC: f64 = 2.0;
pub const DEFAULT_DETECTION_SIGMA: f64 = 5.0;
pub const DEFAULT_MAX_STARS: usize = 500;
pub const DEFAULT_BAND: &str = "G";
pub const CSV_KIND_CATALOG: &str = "catalog";
pub const CSV_KIND_SOURCES: &str = "sources";
pub const CSV_KIND_MATCHES: &str = "matches";
pub const RES_N: &str = "n";
const ARCMIN_PER_DEGREE: f64 = 60.0;
const CSV_LINE_END: &str = "\r\n";

pub const CATALOG_CSV_COLUMNS: &[(&str, &str)] = &[
    ("id", "id"),
    ("ra", "ra"),
    ("dec", "dec"),
    ("ra_epoch", "ra_epoch"),
    ("dec_epoch", "dec_epoch"),
    ("pm_ra_masyr", "pm_ra_masyr"),
    ("pm_dec_masyr", "pm_dec_masyr"),
    ("parallax_mas", "parallax_mas"),
    ("g", "g"),
    ("bp", "bp"),
    ("rp", "rp"),
    ("bp_rp", "bp_rp"),
    ("x", "x"),
    ("y", "y"),
    ("on_image", "on_image"),
];

pub const SOURCES_CSV_COLUMNS: &[(&str, &str)] = &[
    ("x", "x"),
    ("y", "y"),
    ("ra", "ra"),
    ("dec", "dec"),
    ("flux", "flux"),
    ("mag_inst", "mag_inst"),
    ("mag_ab", "mag_ab"),
    ("fwhm", "fwhm"),
    ("snr", "snr"),
    ("saturated", "saturated"),
];

pub const MATCHES_CSV_COLUMNS: &[(&str, &str)] = &[
    ("id", "row.id"),
    ("x", "star.x"),
    ("y", "star.y"),
    ("ra", "star.ra"),
    ("dec", "star.dec"),
    ("flux", "star.flux"),
    ("mag_inst", "star.mag_inst"),
    ("mag_ab", "star.mag_ab"),
    ("fwhm", "star.fwhm"),
    ("snr", "star.snr"),
    ("cat_ra", "row.ra"),
    ("cat_dec", "row.dec"),
    ("g", "row.g"),
    ("bp", "row.bp"),
    ("rp", "row.rp"),
    ("bp_rp", "row.bp_rp"),
    ("sep_arcsec", "sep_arcsec"),
    ("d_ra_arcsec", "d_ra_arcsec"),
    ("d_dec_arcsec", "d_dec_arcsec"),
];

#[derive(Debug, Clone, Serialize)]
pub struct PlacedRow {
    #[serde(flatten)]
    pub row: CatalogRow,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub on_image: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConeSearchResult {
    pub rows: Vec<PlacedRow>,
    pub epoch_year: Option<f64>,
    pub n_total: usize,
    pub n_on_image: usize,
    pub radius_arcmin: f64,
    pub center_ra: f64,
    pub center_dec: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeasuredSource {
    pub x: f64,
    pub y: f64,
    pub ra: f64,
    pub dec: f64,
    pub flux: f64,
    pub fwhm: f64,
    pub snr: f64,
    pub mag_inst: f64,
    pub mag_ab: Option<f64>,
    pub saturated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchedRow {
    pub id: String,
    pub ra: f64,
    pub dec: f64,
    pub g: Option<f64>,
    pub bp: Option<f64>,
    pub rp: Option<f64>,
    pub bp_rp: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchEntry {
    pub star: MeasuredSource,
    pub row: MatchedRow,
    pub sep_arcsec: f64,
    pub d_ra_arcsec: f64,
    pub d_dec_arcsec: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AstrometrySummary {
    pub median_d_ra_arcsec: f64,
    pub median_d_dec_arcsec: f64,
    pub rms_arcsec: f64,
    pub n: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrossMatchResult {
    pub matches: Vec<MatchEntry>,
    pub sources: Vec<MeasuredSource>,
    pub astrometry: Option<AstrometrySummary>,
    pub zero_point: Option<ZeroPointFit>,
    pub photcal_present: bool,
    pub epoch_year: Option<f64>,
    pub n_detected: usize,
    pub n_catalog: usize,
    pub band: String,
    pub match_radius_arcsec: f64,
}

pub(crate) struct FieldGeometry {
    pub center_ra: f64,
    pub center_dec: f64,
    pub diagonal_arcmin: f64,
}

pub(crate) fn field_geometry(wcs: &WcsTransform, cols: usize, rows: usize) -> FieldGeometry {
    let center = wcs.pixel_to_world(cols as f64 / 2.0 - 0.5, rows as f64 / 2.0 - 0.5);
    let (fov_w, fov_h) = wcs.field_of_view(cols, rows);
    FieldGeometry {
        center_ra: center.ra,
        center_dec: center.dec,
        diagonal_arcmin: (fov_w * fov_w + fov_h * fov_h).sqrt(),
    }
}

pub(crate) fn cone_query_for_field(
    geometry: &FieldGeometry,
    radius_arcmin: Option<f64>,
    mag_limit: Option<f64>,
    max_rows: Option<usize>,
) -> ConeQuery {
    let radius_arcmin = radius_arcmin
        .filter(|r| r.is_finite() && *r > 0.0)
        .unwrap_or(geometry.diagonal_arcmin / 2.0);
    ConeQuery {
        ra: geometry.center_ra,
        dec: geometry.center_dec,
        radius_deg: (radius_arcmin / ARCMIN_PER_DEGREE).clamp(MIN_CONE_RADIUS_DEG, MAX_CONE_RADIUS_DEG),
        mag_limit: mag_limit.filter(|m| m.is_finite()),
        max_rows: max_rows.filter(|n| *n > 0).unwrap_or(DEFAULT_MAX_ROWS),
    }
}

fn wcs_for<'a>(entry: &'a ImageEntry, path: &str) -> anyhow::Result<(&'a HduHeader, WcsTransform)> {
    let header = entry
        .header()
        .ok_or_else(|| anyhow::anyhow!("No FITS header available for {}", path))?;
    let wcs = WcsTransform::from_header(header)
        .map_err(|e| anyhow::anyhow!("WCS not available: {:#}. Run Plate Solve first.", e))?;
    Ok((header, wcs))
}

pub(crate) fn place_rows(wcs: &WcsTransform, rows: Vec<CatalogRow>, cols: usize, image_rows: usize) -> Vec<PlacedRow> {
    let sky: Vec<(f64, f64)> = rows.iter().map(|r| (r.ra_epoch, r.dec_epoch)).collect();
    let pixels = wcs.world_to_pixel_batch(&sky);
    rows.into_iter()
        .zip(pixels)
        .map(|(row, (x, y))| {
            let finite = x.is_finite() && y.is_finite();
            let on_image = finite && x >= -0.5 && y >= -0.5 && x < cols as f64 - 0.5 && y < image_rows as f64 - 0.5;
            PlacedRow {
                row,
                x: finite.then_some(x),
                y: finite.then_some(y),
                on_image,
            }
        })
        .collect()
}

fn with_envelope(body: Value, elapsed_ms: u64, warnings: Vec<String>) -> Value {
    let mut value = body;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(RES_ELAPSED_MS.to_string(), json!(elapsed_ms));
        obj.insert(RES_WARNINGS.to_string(), json!(warnings));
    }
    value
}

fn epoch_propagated_rows(header: &HduHeader, catalog: &[CatalogRow], warnings: &mut Vec<String>) -> (Vec<CatalogRow>, Option<f64>) {
    let mut rows = catalog.to_vec();
    let epoch_year = observation_epoch_year(header);
    match epoch_year {
        Some(year) => propagate_epoch(&mut rows, year),
        None => warnings.push(
            "no observation date in the header (DATE-OBS/MJD-OBS); catalog positions are left at J2016.0".to_string(),
        ),
    }
    (rows, epoch_year)
}

pub(crate) fn cone_search_for_path(
    path: &str,
    radius_arcmin: Option<f64>,
    mag_limit: Option<f64>,
    max_rows: Option<usize>,
) -> anyhow::Result<Value> {
    let t0 = Instant::now();
    let entry = load_cached_full(path)?;
    let (header, wcs) = wcs_for(&entry, path)?;
    let (image_rows, cols) = entry.arr().dim();
    let geometry = field_geometry(&wcs, cols, image_rows);
    let query = cone_query_for_field(&geometry, radius_arcmin, mag_limit, max_rows);
    let mut warnings: Vec<String> = Vec::new();
    let catalog = query_gaia_cached(&query).map_err(|e| anyhow::anyhow!(e))?;
    if catalog.len() >= query.max_rows {
        warnings.push(format!(
            "VizieR row cap of {} reached; raise the row limit or lower the magnitude limit",
            query.max_rows
        ));
    }
    let (rows, epoch_year) = epoch_propagated_rows(header, &catalog, &mut warnings);
    let placed = place_rows(&wcs, rows, cols, image_rows);
    let n_on_image = placed.iter().filter(|r| r.on_image).count();
    let body = serde_json::to_value(ConeSearchResult {
        n_total: placed.len(),
        n_on_image,
        rows: placed,
        epoch_year,
        radius_arcmin: query.radius_deg * ARCMIN_PER_DEGREE,
        center_ra: query.ra,
        center_dec: query.dec,
    })?;
    Ok(with_envelope(body, t0.elapsed().as_millis() as u64, warnings))
}

fn normalize_band(band: Option<String>) -> anyhow::Result<String> {
    let name = band.as_deref().map(str::trim).filter(|b| !b.is_empty()).unwrap_or(DEFAULT_BAND).to_ascii_uppercase();
    match name.as_str() {
        "G" | "BP" | "RP" => Ok(name),
        other => anyhow::bail!("unknown Gaia band '{}': use G, BP or RP", other),
    }
}

fn band_magnitude(row: &CatalogRow, band: &str) -> Option<f64> {
    match band {
        "BP" => row.bp,
        "RP" => row.rp,
        _ => row.g,
    }
}

fn measure_sources(
    image: &Array2<f32>,
    err: Option<&Array2<f32>>,
    saturated: Option<&Array2<u8>>,
    stars: &[DetectedStar],
    config: &PhotometryConfig,
    wcs: &WcsTransform,
    photcal: Option<&PhotCal>,
) -> Vec<MeasuredSource> {
    let mut sources: Vec<MeasuredSource> = Vec::with_capacity(stars.len());
    for star in stars {
        let Ok(phot) = measure_star_full(image, err, None, saturated, star.x, star.y, config) else {
            continue;
        };
        if !phot.net_flux.is_finite() || phot.net_flux <= 0.0 {
            continue;
        }
        let sky = wcs.pixel_to_world(phot.x, phot.y);
        let mag_ab = photcal
            .and_then(|cal| cal.calibrate(phot.net_flux, Some(phot.flux_err)))
            .and_then(|c| c.mag_ab);
        sources.push(MeasuredSource {
            x: phot.x,
            y: phot.y,
            ra: sky.ra,
            dec: sky.dec,
            flux: phot.net_flux,
            fwhm: phot.fwhm,
            snr: phot.snr,
            mag_inst: phot.mag_inst,
            mag_ab,
            saturated: phot.saturated,
        });
    }
    sources
}

fn summarize_astrometry(matches: &[CrossMatch]) -> Option<AstrometrySummary> {
    if matches.is_empty() {
        return None;
    }
    let d_ra: Vec<f64> = matches.iter().map(|m| m.d_ra_arcsec).collect();
    let d_dec: Vec<f64> = matches.iter().map(|m| m.d_dec_arcsec).collect();
    let rms = (matches.iter().map(|m| m.sep_arcsec * m.sep_arcsec).sum::<f64>() / matches.len() as f64).sqrt();
    Some(AstrometrySummary {
        median_d_ra_arcsec: exact_median_f64(&d_ra),
        median_d_dec_arcsec: exact_median_f64(&d_dec),
        rms_arcsec: rms,
        n: matches.len(),
    })
}

pub(crate) fn crossmatch_for_path(
    path: &str,
    sigma: Option<f64>,
    max_stars: Option<usize>,
    radius_arcsec: Option<f64>,
    band: Option<String>,
    colour_term: Option<bool>,
    aperture_radius: Option<f64>,
) -> anyhow::Result<Value> {
    let t0 = Instant::now();
    let band = normalize_band(band)?;
    let entry = load_cached_full(path)?;
    let (header, wcs) = wcs_for(&entry, path)?;
    let dims = entry.arr().dim();
    let (image_rows, cols) = dims;
    let mut warnings: Vec<String> = Vec::new();
    let planes = photometry_planes(path, dims, &mut warnings);
    let photcal = PhotCal::from_header(header, Some(&wcs));

    let detection_sigma = sigma.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(DEFAULT_DETECTION_SIGMA);
    let mut stars = detect_stars(entry.arr(), detection_sigma).stars;
    let n_detected = stars.len();
    stars.truncate(max_stars.filter(|n| *n > 0).unwrap_or(DEFAULT_MAX_STARS));

    let config = PhotometryConfig {
        aperture_radius: aperture_radius.filter(|r| r.is_finite() && *r > 0.0),
        image_max: Some(entry.stats().max),
        ..PhotometryConfig::default()
    };
    let sources = measure_sources(
        entry.arr(),
        planes.err.as_ref().map(|e| e.arr()),
        planes.saturated.as_ref(),
        &stars,
        &config,
        &wcs,
        photcal.as_ref(),
    );

    let geometry = field_geometry(&wcs, cols, image_rows);
    let query = cone_query_for_field(&geometry, None, None, None);
    let catalog = query_gaia_cached(&query).map_err(|e| anyhow::anyhow!(e))?;
    let (rows, epoch_year) = epoch_propagated_rows(header, &catalog, &mut warnings);

    let match_radius = radius_arcsec
        .filter(|r| r.is_finite() && *r > 0.0)
        .unwrap_or(DEFAULT_MATCH_RADIUS_ARCSEC);
    let sky: Vec<(f64, f64)> = sources.iter().map(|s| (s.ra, s.dec)).collect();
    let pairs = cross_match(&sky, &rows, match_radius);
    if pairs.is_empty() {
        warnings.push(format!(
            "no catalog match within {:.1} arcsec for {} measured stars; check the WCS solution and the observation epoch",
            match_radius,
            sources.len()
        ));
    }

    let usable: Vec<&CrossMatch> = pairs.iter().filter(|m| !sources[m.star_index].saturated).collect();
    let mag_inst: Vec<f64> = usable.iter().map(|m| sources[m.star_index].mag_inst).collect();
    let cat_mag: Vec<f64> = usable
        .iter()
        .map(|m| band_magnitude(&rows[m.row_index], &band).unwrap_or(f64::NAN))
        .collect();
    let colour: Vec<Option<f64>> = usable.iter().map(|m| rows[m.row_index].bp_rp).collect();
    let zero_point = fit_zero_point(&mag_inst, &cat_mag, &colour, colour_term.unwrap_or(true), &band);
    if pairs.len() > usable.len() {
        warnings.push(format!(
            "{} saturated star(s) excluded from the zero point",
            pairs.len() - usable.len()
        ));
    }
    if let Some(cal) = &photcal {
        warnings.push(format!(
            "image is already flux-calibrated ({}); the Gaia zero point is informational",
            cal.label()
        ));
    }

    let astrometry = summarize_astrometry(&pairs);
    let matches: Vec<MatchEntry> = pairs
        .iter()
        .map(|m| {
            let row = &rows[m.row_index];
            MatchEntry {
                star: sources[m.star_index].clone(),
                row: MatchedRow {
                    id: row.id.clone(),
                    ra: row.ra_epoch,
                    dec: row.dec_epoch,
                    g: row.g,
                    bp: row.bp,
                    rp: row.rp,
                    bp_rp: row.bp_rp,
                },
                sep_arcsec: m.sep_arcsec,
                d_ra_arcsec: m.d_ra_arcsec,
                d_dec_arcsec: m.d_dec_arcsec,
            }
        })
        .collect();

    let body = serde_json::to_value(CrossMatchResult {
        matches,
        sources,
        astrometry,
        zero_point,
        photcal_present: photcal.is_some(),
        epoch_year,
        n_detected,
        n_catalog: rows.len(),
        band,
        match_radius_arcsec: match_radius,
    })?;
    Ok(with_envelope(body, t0.elapsed().as_millis() as u64, warnings))
}

fn csv_columns(kind: &str) -> anyhow::Result<&'static [(&'static str, &'static str)]> {
    match kind {
        CSV_KIND_CATALOG => Ok(CATALOG_CSV_COLUMNS),
        CSV_KIND_SOURCES => Ok(SOURCES_CSV_COLUMNS),
        CSV_KIND_MATCHES => Ok(MATCHES_CSV_COLUMNS),
        other => anyhow::bail!("unknown CSV export kind '{}': use catalog, sources or matches", other),
    }
}

fn lookup<'a>(item: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(item, |current, key| current.get(key))
}

pub(crate) fn  csv_escape(text: &str) -> String {
    if text.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn csv_cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => csv_escape(s),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => csv_escape(&other.to_string()),
    }
}

pub(crate) fn build_csv(kind: &str, items: &[Value]) -> anyhow::Result<String> {
    let columns = csv_columns(kind)?;
    let mut out = String::new();
    let header: Vec<String> = columns.iter().map(|(name, _)| csv_escape(name)).collect();
    out.push_str(&header.join(","));
    out.push_str(CSV_LINE_END);
    for item in items {
        let cells: Vec<String> = columns.iter().map(|(_, path)| csv_cell(lookup(item, path))).collect();
        out.push_str(&cells.join(","));
        out.push_str(CSV_LINE_END);
    }
    Ok(out)
}

pub(crate) fn write_csv(output_path: &str, kind: &str, items: &[Value]) -> anyhow::Result<usize> {
    let csv = build_csv(kind, items)?;
    std::fs::write(output_path, csv).map_err(|e| anyhow::anyhow!("cannot write {}: {}", output_path, e))?;
    Ok(items.len())
}

#[tauri::command]
pub async fn catalog_cone_search_cmd(
    path: String,
    radius_arcmin: Option<f64>,
    mag_limit: Option<f64>,
    max_rows: Option<usize>,
) -> Result<Value, String> {
    blocking_cmd!(cone_search_for_path(&path, radius_arcmin, mag_limit, max_rows))
}

#[tauri::command]
pub async fn catalog_crossmatch_cmd(
    path: String,
    sigma: Option<f64>,
    max_stars: Option<usize>,
    radius_arcsec: Option<f64>,
    band: Option<String>,
    colour_term: Option<bool>,
    aperture_radius: Option<f64>,
) -> Result<Value, String> {
    blocking_cmd!({
        crossmatch_for_path(&path, sigma, max_stars, radius_arcsec, band, colour_term, aperture_radius)
    })
}

#[tauri::command]
pub async fn catalog_export_csv_cmd(
    path: String,
    output_path: String,
    kind: String,
    rows: Option<Vec<Value>>,
    matches: Option<Vec<Value>>,
) -> Result<Value, String> {
    blocking_cmd!({
        let items = if kind == CSV_KIND_MATCHES { matches } else { rows }.unwrap_or_default();
        let n = write_csv(&output_path, &kind, &items)?;
        log::info!("catalog CSV ({}) for {} -> {} ({} rows)", kind, path, output_path, n);
        Ok(json!({ RES_OUTPUT_PATH: output_path, RES_N: n }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::catalog::{catalog_cache_key, query_gaia_cached_with};
    use crate::core::astrometry::time::jd_from_gregorian;
    use crate::core::astrometry::catalog::julian_epoch_year;
    use crate::infra::fits::writer::write_fits_mono;

    const IMAGE_SIZE: usize = 128;
    const STAR_SIGMA_PX: f64 = 2.0;
    const STAR_POSITIONS: [(f64, f64); 5] = [(30.0, 30.0), (90.0, 40.0), (60.0, 95.0), (100.0, 100.0), (25.0, 80.0)];
    const STAR_AMPLITUDES: [f64; 5] = [4000.0, 2500.0, 1500.0, 900.0, 600.0];
    const TEST_ZERO_POINT: f64 = 25.0;

    fn wcs_header(extra: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        let scale = 1.0 / 3600.0;
        for (k, v) in [
            ("NAXIS1", IMAGE_SIZE.to_string()),
            ("NAXIS2", IMAGE_SIZE.to_string()),
            ("CRPIX1", "64.5".to_string()),
            ("CRPIX2", "64.5".to_string()),
            ("CRVAL1", "150.0".to_string()),
            ("CRVAL2", "2.0".to_string()),
            ("CD1_1", format!("{:.12e}", -scale)),
            ("CD1_2", "0.0".to_string()),
            ("CD2_1", "0.0".to_string()),
            ("CD2_2", format!("{:.12e}", scale)),
            ("CTYPE1", "RA---TAN".to_string()),
            ("CTYPE2", "DEC--TAN".to_string()),
        ] {
            header.set(k, v);
        }
        for (k, v) in extra {
            header.set(k, v.to_string());
        }
        header
    }

    fn star_field() -> Array2<f32> {
        let mut arr = Array2::<f32>::from_elem((IMAGE_SIZE, IMAGE_SIZE), 100.0);
        for r in 0..IMAGE_SIZE {
            for c in 0..IMAGE_SIZE {
                let mut v = 100.0f64;
                for ((sx, sy), amp) in STAR_POSITIONS.iter().zip(STAR_AMPLITUDES) {
                    let dx = c as f64 - sx;
                    let dy = r as f64 - sy;
                    v += amp * (-(dx * dx + dy * dy) / (2.0 * STAR_SIGMA_PX * STAR_SIGMA_PX)).exp();
                }
                arr[[r, c]] = v as f32;
            }
        }
        arr
    }

    fn expected_total_flux(amp: f64) -> f64 {
        2.0 * std::f64::consts::PI * amp * STAR_SIGMA_PX * STAR_SIGMA_PX
    }

    fn write_star_field(dir: &tempfile::TempDir, name: &str, extra: &[(&str, &str)]) -> String {
        let path = dir.path().join(name);
        write_fits_mono(path.to_str().unwrap(), &star_field(), Some(&wcs_header(extra))).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn catalog_rows_for_field(wcs: &WcsTransform) -> Vec<CatalogRow> {
        STAR_POSITIONS
            .iter()
            .zip(STAR_AMPLITUDES)
            .enumerate()
            .map(|(i, ((x, y), amp))| {
                let sky = wcs.pixel_to_world(*x, *y);
                CatalogRow {
                    id: format!("star{}", i),
                    ra: sky.ra,
                    dec: sky.dec,
                    ra_epoch: sky.ra,
                    dec_epoch: sky.dec,
                    pm_ra_masyr: None,
                    pm_dec_masyr: None,
                    g: Some(TEST_ZERO_POINT - 2.5 * expected_total_flux(amp).log10()),
                    bp: None,
                    rp: None,
                    bp_rp: Some(0.8),
                    parallax_mas: None,
                }
            })
            .collect()
    }

    fn prime_cache(query: &ConeQuery, rows: Vec<CatalogRow>) {
        query_gaia_cached_with(query, |_| Ok(rows)).unwrap();
    }

    #[test]
    fn csv_export_escapes_fields_and_writes_empty_cells_for_missing_values() {
        let items = vec![
            json!({"x": 1.5, "y": 2.0, "ra": 150.0, "dec": 2.0, "flux": 1234.5, "mag_inst": -7.73, "mag_ab": 17.27, "fwhm": 4.7, "snr": 120.0, "saturated": false}),
            json!({"x": 3.0, "y": 4.0, "ra": 150.1, "dec": 2.1, "flux": 10.0, "mag_inst": -2.5, "mag_ab": null, "fwhm": 4.1, "snr": 5.0, "saturated": true}),
        ];
        let csv = build_csv(CSV_KIND_SOURCES, &items).unwrap();
        let lines: Vec<&str> = csv.split(CSV_LINE_END).collect();
        assert_eq!(lines[0], "x,y,ra,dec,flux,mag_inst,mag_ab,fwhm,snr,saturated");
        assert_eq!(lines[1], "1.5,2.0,150.0,2.0,1234.5,-7.73,17.27,4.7,120.0,false");
        assert_eq!(lines[2], "3.0,4.0,150.1,2.1,10.0,-2.5,,4.1,5.0,true");
        assert_eq!(lines[3], "");
        assert_eq!(lines.len(), 4, "file ends with a single line terminator");

        let tricky = vec![json!({
            "id": "say \"hi\", friend",
            "ra": 1.0, "dec": 2.0, "ra_epoch": 1.0, "dec_epoch": 2.0,
            "pm_ra_masyr": null, "pm_dec_masyr": null, "parallax_mas": null,
            "g": 9.5, "bp": null, "rp": null, "bp_rp": null, "x": 10.0, "y": 20.0, "on_image": true
        }), json!({
            "id": "multi\nline", "ra": 3.0, "dec": 4.0, "ra_epoch": 3.0, "dec_epoch": 4.0,
            "g": null, "x": null, "y": null, "on_image": false
        })];
        let csv = build_csv(CSV_KIND_CATALOG, &tricky).unwrap();
        let lines: Vec<&str> = csv.split(CSV_LINE_END).collect();
        assert_eq!(lines[0], "id,ra,dec,ra_epoch,dec_epoch,pm_ra_masyr,pm_dec_masyr,parallax_mas,g,bp,rp,bp_rp,x,y,on_image");
        assert_eq!(lines[1], "\"say \"\"hi\"\", friend\",1.0,2.0,1.0,2.0,,,,9.5,,,,10.0,20.0,true");
        assert_eq!(lines[2], "\"multi\nline\",3.0,4.0,3.0,4.0,,,,,,,,,,false");

        let matched = vec![json!({
            "star": {"x": 30.0, "y": 30.0, "ra": 150.0, "dec": 2.0, "flux": 100.0, "mag_inst": -5.0, "mag_ab": null, "fwhm": 4.0, "snr": 50.0},
            "row": {"id": "gaia1", "ra": 150.0001, "dec": 2.0001, "g": 20.0, "bp": null, "rp": null, "bp_rp": 0.8},
            "sep_arcsec": 0.3, "d_ra_arcsec": 0.2, "d_dec_arcsec": -0.1
        })];
        let csv = build_csv(CSV_KIND_MATCHES, &matched).unwrap();
        let lines: Vec<&str> = csv.split(CSV_LINE_END).collect();
        assert_eq!(lines[0], "id,x,y,ra,dec,flux,mag_inst,mag_ab,fwhm,snr,cat_ra,cat_dec,g,bp,rp,bp_rp,sep_arcsec,d_ra_arcsec,d_dec_arcsec");
        assert_eq!(lines[1], "gaia1,30.0,30.0,150.0,2.0,100.0,-5.0,,4.0,50.0,150.0001,2.0001,20.0,,,0.8,0.3,0.2,-0.1");

        let err = build_csv("bogus", &[]).unwrap_err().to_string();
        assert!(err.contains("bogus"), "{err}");

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("sources.csv");
        let n = write_csv(out.to_str().unwrap(), CSV_KIND_SOURCES, &items).unwrap();
        assert_eq!(n, 2);
        let written = std::fs::read_to_string(&out).unwrap();
        assert_eq!(written, build_csv(CSV_KIND_SOURCES, &items).unwrap());
    }

    #[test]
    fn cone_search_places_epoch_propagated_rows_on_the_image_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_star_field(&dir, "cone.fits", &[("DATE-OBS", "2026-01-01T00:00:00")]);
        let header = wcs_header(&[]);
        let wcs = WcsTransform::from_header(&header).unwrap();
        let geometry = field_geometry(&wcs, IMAGE_SIZE, IMAGE_SIZE);
        let query = cone_query_for_field(&geometry, Some(1.5), Some(18.0), Some(50));
        assert!((query.radius_deg - 1.5 / 60.0).abs() < 1e-12);
        assert_eq!(query.mag_limit, Some(18.0));
        assert_eq!(query.max_rows, 50);

        let mut rows = catalog_rows_for_field(&wcs);
        rows[0].pm_ra_masyr = Some(0.0);
        rows[0].pm_dec_masyr = Some(-1000.0);
        let outside = wcs.pixel_to_world(400.0, 400.0);
        rows.push(CatalogRow {
            id: "outside".into(),
            ra: outside.ra,
            dec: outside.dec,
            ra_epoch: outside.ra,
            dec_epoch: outside.dec,
            pm_ra_masyr: None,
            pm_dec_masyr: None,
            g: Some(15.0),
            bp: None,
            rp: None,
            bp_rp: None,
            parallax_mas: None,
        });
        prime_cache(&query, rows);

        let out = cone_search_for_path(&path, Some(1.5), Some(18.0), Some(50)).unwrap();
        assert_eq!(out["n_total"], 6);
        assert_eq!(out["n_on_image"], 5);
        let epoch = out["epoch_year"].as_f64().unwrap();
        assert!((epoch - julian_epoch_year(jd_from_gregorian(2026, 1, 1.0))).abs() < 1e-9);
        assert!((out["radius_arcmin"].as_f64().unwrap() - 1.5).abs() < 1e-9);
        assert!(out[RES_ELAPSED_MS].is_number());
        assert!(out[RES_WARNINGS].as_array().unwrap().is_empty(), "{:?}", out[RES_WARNINGS]);

        let placed = out["rows"].as_array().unwrap();
        assert_eq!(placed.len(), 6);
        let moved = &placed[0];
        assert_eq!(moved["id"], "star0");
        let dy = moved["y"].as_f64().unwrap() - STAR_POSITIONS[0].1;
        assert!((dy - (-10.0)).abs() < 0.05, "10 years of -1000 mas/yr moves the star 10 px south: dy={dy}");
        assert!((moved["x"].as_f64().unwrap() - STAR_POSITIONS[0].0).abs() < 0.05);
        assert_eq!(moved["on_image"], true);
        for (i, (x, y)) in STAR_POSITIONS.iter().enumerate().skip(1) {
            assert!((placed[i]["x"].as_f64().unwrap() - x).abs() < 1e-6);
            assert!((placed[i]["y"].as_f64().unwrap() - y).abs() < 1e-6);
            assert_eq!(placed[i]["on_image"], true);
        }
        assert_eq!(placed[5]["id"], "outside");
        assert_eq!(placed[5]["on_image"], false);

        let no_wcs = dir.path().join("plain.fits");
        write_fits_mono(no_wcs.to_str().unwrap(), &star_field(), None).unwrap();
        let err = cone_search_for_path(no_wcs.to_str().unwrap(), None, None, None).unwrap_err().to_string();
        assert!(err.contains("WCS"), "{err}");
    }

    #[test]
    fn crossmatch_measures_detected_stars_and_recovers_the_zero_point() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_star_field(&dir, "match.fits", &[("DATE-OBS", "2016-01-01T00:00:00")]);
        let header = wcs_header(&[]);
        let wcs = WcsTransform::from_header(&header).unwrap();
        let geometry = field_geometry(&wcs, IMAGE_SIZE, IMAGE_SIZE);
        let query = cone_query_for_field(&geometry, None, None, None);
        assert!((geometry.diagonal_arcmin - (IMAGE_SIZE as f64) * 2f64.sqrt() / 60.0).abs() < 1e-9);
        assert_eq!(catalog_cache_key(&query), catalog_cache_key(&cone_query_for_field(&geometry, None, None, None)));
        prime_cache(&query, catalog_rows_for_field(&wcs));

        let out = crossmatch_for_path(&path, Some(5.0), Some(50), Some(2.0), None, Some(false), None).unwrap();
        assert_eq!(out["n_catalog"], 5);
        assert_eq!(out["band"], "G");
        assert_eq!(out["photcal_present"], false);
        assert!((out["match_radius_arcsec"].as_f64().unwrap() - 2.0).abs() < 1e-12);
        let sources = out["sources"].as_array().unwrap();
        assert!(sources.len() >= 5, "{} sources", sources.len());
        let matches = out["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 5, "{matches:?}");
        for m in matches {
            assert!(m["sep_arcsec"].as_f64().unwrap() < 0.3, "{m:?}");
            assert!(m["star"]["mag_ab"].is_null());
            assert!(m["star"]["mag_inst"].as_f64().unwrap() < 0.0);
            assert!(m["row"]["id"].as_str().unwrap().starts_with("star"));
        }
        let astrometry = &out["astrometry"];
        assert_eq!(astrometry["n"], 5);
        assert!(astrometry["rms_arcsec"].as_f64().unwrap() < 0.3);
        assert!(astrometry["median_d_ra_arcsec"].as_f64().unwrap().abs() < 0.3);
        let brightest = sources
            .iter()
            .max_by(|a, b| a["flux"].as_f64().unwrap().total_cmp(&b["flux"].as_f64().unwrap()))
            .unwrap();
        assert_eq!(brightest["saturated"], true, "the star at the image maximum is flagged saturated: {brightest:?}");
        let zp = &out["zero_point"];
        assert_eq!(zp["band"], "G");
        assert_eq!(zp["colour_term_used"], false);
        assert_eq!(zp["n_used"], 4, "the saturated star stays matched but leaves the zero-point fit");
        let value = zp["zp"].as_f64().unwrap();
        assert!((value - TEST_ZERO_POINT).abs() < 0.1, "zp={value}");
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(warnings.iter().any(|w| w.as_str().unwrap() == "1 saturated star(s) excluded from the zero point"), "{warnings:?}");
        assert!(!warnings.iter().any(|w| w.as_str().unwrap().contains("flux-calibrated")), "{warnings:?}");

        let with_colour = crossmatch_for_path(&path, None, None, None, Some("G".into()), Some(true), None).unwrap();
        let zp = &with_colour["zero_point"];
        assert_eq!(zp["colour_term_used"], false, "identical colours cannot support a colour term");
        assert!((zp["zp"].as_f64().unwrap() - TEST_ZERO_POINT).abs() < 0.1);

        let bad_band = crossmatch_for_path(&path, None, None, None, Some("V".into()), None, None).unwrap_err().to_string();
        assert!(bad_band.contains("band"), "{bad_band}");

        let rp_band = crossmatch_for_path(&path, None, None, None, Some("rp".into()), None, None).unwrap();
        assert_eq!(rp_band["band"], "RP");
        assert!(rp_band["zero_point"].is_null(), "no RP magnitudes in the catalog rows");
        assert_eq!(rp_band["matches"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn crossmatch_warns_when_the_image_is_already_flux_calibrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_star_field(
            &dir,
            "calibrated.fits",
            &[("MAGZPT", "25.0"), ("BUNIT", "ADU"), ("DATE-OBS", "2016-01-01")],
        );
        let header = wcs_header(&[]);
        let wcs = WcsTransform::from_header(&header).unwrap();
        let geometry = field_geometry(&wcs, IMAGE_SIZE, IMAGE_SIZE);
        prime_cache(&cone_query_for_field(&geometry, None, None, None), catalog_rows_for_field(&wcs));

        let out = crossmatch_for_path(&path, None, None, None, None, None, None).unwrap();
        assert_eq!(out["photcal_present"], true);
        let warnings = out[RES_WARNINGS].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| {
                let text = w.as_str().unwrap();
                text.starts_with("image is already flux-calibrated (zero point MAGZPT")
                    && text.ends_with("the Gaia zero point is informational")
            }),
            "{warnings:?}"
        );
        let matches = out["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 5);
        for m in matches {
            let mag_ab = m["star"]["mag_ab"].as_f64().unwrap();
            let mag_inst = m["star"]["mag_inst"].as_f64().unwrap();
            assert!((mag_ab - (mag_inst + 25.0)).abs() < 1e-6, "{m:?}");
        }
    }
}
