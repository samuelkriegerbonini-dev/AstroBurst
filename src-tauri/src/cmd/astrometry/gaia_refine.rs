use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::cmd::analysis::{photometry_planes, resolve_dq_mask, PHOTOMETRY_SATURATED_FLAG};
use crate::cmd::common::{blocking_cmd, load_cached_full, load_companions};
use crate::core::analysis::photometry::{measure_star_prepared, saturation_level, MaskedImage, PhotometryConfig, StarPhotometry};
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::catalog::{
    observation_epoch_year, query_gaia_tie_cached_in, query_gaia_tie_tsv, ConeQuery, MAX_CONE_RADIUS_DEG, MIN_CONE_RADIUS_DEG,
};
use crate::core::astrometry::catalog_disk;
use crate::core::astrometry::gaia_tie::{
    tie_to_gaia, CatalogueKind, Epoch, EpochSource, GaiaCatalogue, ModelChoice, TieDecision, TieDetection, TieInput,
    TieModel, TieOptions, TieResult, DEFAULT_EPOCH_UNCERTAINTY_YR, DEFAULT_SEARCH_RADIUS_ARCSEC, GAIA_DR3_ATTRIBUTION,
    GAIA_DR3_CATALOGUE, GAIA_ESA_ACKNOWLEDGEMENT,
};
use crate::core::astrometry::gwcs::sip_fit::{fit_tan_sip, SipFitOptions};
use crate::core::astrometry::spectral::{earth_heliocentric_position_au, mid_exposure_jd, sun_barycentric_position_au};
use crate::core::astrometry::time::gregorian_from_jd;
use crate::core::astrometry::wcs::{pixel_center, WcsKind, WcsTransform};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::writer::is_wcs_card;
use crate::infra::wcs_source::load_wcs;
use crate::types::constants::{HEADER_NAXIS1, HEADER_NAXIS2};
use crate::types::header::HduHeader;

pub(super) const ABPROC_GAIA_TIED: &str = "gaiatied";
pub(super) const HISTORY_WIDTH: usize = 70;
const GAIA_TIE_SHORT_ATTRIBUTION: &str = "Gaia DR3, ESA/Gaia/DPAC, via VizieR";
const TIE_DETECTION_SIGMA: f64 = 5.0;
const TIE_MAX_DETECTIONS: usize = 2000;
const TIE_SCAN_LIMIT: usize = 5 * TIE_MAX_DETECTIONS;
const FWHM_OUTLIER_FACTOR: f64 = 2.0;
const TIE_GAIA_MAG_LIMIT: f64 = 21.0;
const TIE_MIN_ROWS: f64 = 200.0;
const TIE_MAX_ROWS: f64 = 5000.0;
const MIN_EPOCH_YEAR: f64 = 1900.0;
const MAX_EPOCH_YEAR: f64 = 2100.0;
const KM_PER_AU: f64 = 149_597_870.7;
const ARCSEC_PER_DEG: f64 = 3600.0;
const ARCMIN_PER_DEG: f64 = 60.0;
const SPACE_TELESCOPES: [&str; 3] = ["JWST", "HST", "ROMAN"];
const JWST_POSITION_KEYS: [&str; 3] = ["JWST_X", "JWST_Y", "JWST_Z"];
const RESAMPLED_KEY: &str = "S_RESAMP";
const KIND_SOLVE: &str = "solve";
const KIND_HEADER: &str = "header";
const KIND_GWCS: &str = "gwcs";
const EXACT_REPRESENTATION: &str = "exact header update (CRVAL and CD)";
const REFIT_TARGET_PX: f64 = 0.001;
const REFIT_LIMIT_PX: f64 = 0.01;
const SECONDS_PER_DAY: f64 = 86_400.0;
const UNIX_EPOCH_JD: f64 = 2_440_587.5;

pub(super) fn history_cards(lines: &[String]) -> Vec<String> {
    let mut cards = Vec::new();
    for line in lines {
        let ascii: String = line.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).collect();
        let mut current = String::new();
        for word in ascii.split_whitespace() {
            if !current.is_empty() && current.len() + 1 + word.len() > HISTORY_WIDTH {
                cards.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            while current.len() > HISTORY_WIDTH {
                let rest = current.split_off(HISTORY_WIDTH);
                cards.push(std::mem::replace(&mut current, rest));
            }
        }
        if !current.is_empty() {
            cards.push(current);
        }
    }
    cards
}

pub(super) struct InputWcs {
    pub wcs: WcsTransform,
    pub cards: Vec<(String, String)>,
    pub kind: &'static str,
}

fn header_wcs_cards(header: &HduHeader) -> Vec<(String, String)> {
    let mut cards = HduHeader::empty();
    for (key, value) in &header.cards {
        let key = key.trim();
        if is_wcs_card(key) {
            cards.set(key, value.clone());
        }
    }
    cards.cards
}

pub(super) fn input_wcs(
    path: &str,
    header: &HduHeader,
    (cols, rows): (usize, usize),
    solve_cards: Option<&[(String, String)]>,
) -> anyhow::Result<InputWcs> {
    let Some(solve_cards) = solve_cards.filter(|c| !c.is_empty()) else {
        let wcs = load_wcs(path, header)
            .map_err(|e| anyhow::anyhow!("no celestial WCS to refine in {path} ({e:#}); plate solve the image first"))?;
        let kind = if wcs.kind() == WcsKind::Gwcs { KIND_GWCS } else { KIND_HEADER };
        return Ok(InputWcs { wcs, cards: header_wcs_cards(header), kind });
    };
    let mut probe = HduHeader::empty();
    for (key, value) in solve_cards {
        let key = key.trim();
        if is_wcs_card(key) {
            probe.set(key, value.clone());
        }
    }
    let cards = probe.cards.clone();
    probe.set(HEADER_NAXIS1, cols.to_string());
    probe.set(HEADER_NAXIS2, rows.to_string());
    let wcs = WcsTransform::from_header(&probe)
        .map_err(|e| anyhow::anyhow!("the solve's WCS is not a usable celestial WCS ({e:#}); nothing was fetched"))?;
    Ok(InputWcs { wcs, cards, kind: KIND_SOLVE })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TieEpochChoice {
    pub year: Option<f64>,
    pub source: EpochSource,
}

impl TieEpochChoice {
    fn epoch(&self) -> Epoch {
        Epoch { year: self.year, source: self.source, assumed_uncertainty_yr: DEFAULT_EPOCH_UNCERTAINTY_YR }
    }

    fn source_name(&self) -> &'static str {
        match self.source {
            EpochSource::Header => "header",
            EpochSource::Typed => "typed",
            EpochSource::None => "none",
        }
    }
}

pub(super) fn tie_epoch(header: &HduHeader, typed: Option<f64>) -> anyhow::Result<TieEpochChoice> {
    if let Some(year) = typed {
        if !year.is_finite() || !(MIN_EPOCH_YEAR..=MAX_EPOCH_YEAR).contains(&year) {
            anyhow::bail!("the typed epoch {year} is not a year between {MIN_EPOCH_YEAR} and {MAX_EPOCH_YEAR}");
        }
        return Ok(TieEpochChoice { year: Some(year), source: EpochSource::Typed });
    }
    Ok(match observation_epoch_year(header) {
        Some(year) => TieEpochChoice { year: Some(year), source: EpochSource::Header },
        None => TieEpochChoice { year: None, source: EpochSource::None },
    })
}

fn text_card(header: &HduHeader, key: &str) -> Option<String> {
    header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_ascii_uppercase()).filter(|v| !v.is_empty())
}

pub(super) fn space_telescope(header: &HduHeader) -> bool {
    text_card(header, "TELESCOP").is_some_and(|t| SPACE_TELESCOPES.iter().any(|s| t.starts_with(s)))
}

pub(super) fn tie_observer(header: &HduHeader, epoch: &TieEpochChoice) -> Option<[f64; 3]> {
    if epoch.source == EpochSource::Typed {
        return None;
    }
    observer_au(header)
}

pub(super) fn observer_au(header: &HduHeader) -> Option<[f64; 3]> {
    if let [Some(x), Some(y), Some(z)] = JWST_POSITION_KEYS.map(|k| header.get_f64(k)) {
        return Some([x / KM_PER_AU, y / KM_PER_AU, z / KM_PER_AU]);
    }
    if space_telescope(header) {
        return None;
    }
    let (jd, _) = mid_exposure_jd(header)?;
    let earth = earth_heliocentric_position_au(jd);
    let sun = sun_barycentric_position_au(jd);
    Some([earth[0] + sun[0], earth[1] + sun[1], earth[2] + sun[2]])
}

pub(super) fn tie_cone(wcs: &WcsTransform, (cols, rows): (usize, usize), detections: usize) -> ConeQuery {
    let (cx, cy) = pixel_center(cols, rows);
    let centre = wcs.pixel_to_world(cx, cy);
    let (fov_w, fov_h) = wcs.field_of_view(cols, rows);
    let half_diagonal_deg = (fov_w * fov_w + fov_h * fov_h).sqrt() / 2.0 / ARCMIN_PER_DEG;
    let radius_deg = (half_diagonal_deg + DEFAULT_SEARCH_RADIUS_ARCSEC / ARCSEC_PER_DEG).clamp(MIN_CONE_RADIUS_DEG, MAX_CONE_RADIUS_DEG);
    let image_area = (fov_w * fov_h).max(f64::MIN_POSITIVE);
    let cone_area = std::f64::consts::PI * (radius_deg * ARCMIN_PER_DEG).powi(2);
    let rows_cap = (2.0 * detections.max(1) as f64 * cone_area / image_area).clamp(TIE_MIN_ROWS, TIE_MAX_ROWS);
    ConeQuery {
        ra: centre.ra,
        dec: centre.dec,
        radius_deg,
        mag_limit: Some(TIE_GAIA_MAG_LIMIT),
        max_rows: rows_cap.round() as usize,
    }
}

pub(super) type TieFetch<'a> = &'a dyn Fn(&ConeQuery) -> Result<String, String>;

pub(super) fn tie_catalogue(query: &ConeQuery, dir: Option<&Path>, fetch: TieFetch<'_>) -> Result<GaiaCatalogue, String> {
    let hit = query_gaia_tie_cached_in(dir, query, fetch)?;
    Ok(GaiaCatalogue {
        kind: CatalogueKind::GaiaDr3Vizier,
        quality_columns: hit.rows.iter().any(|r| r.ruwe.is_some() && r.e_ra_mas.is_some()),
        rows: hit.rows.as_ref().clone(),
        cache_source: hit.source.name().to_string(),
    })
}

pub(super) fn unusable_saturation_map(bits: &ndarray::Array2<u32>, saturated: u32, unusable: u32) -> ndarray::Array2<u8> {
    bits.mapv(|b| u8::from(b & saturated != 0 && (unusable == 0 || b & unusable != 0)))
}

pub(super) fn core_saturated(map: &ndarray::Array2<u8>, x: f64, y: f64) -> bool {
    let (rows, cols) = map.dim();
    let (cx, cy) = (x.round(), y.round());
    if !(cx.is_finite() && cy.is_finite()) || cx < -1.0 || cy < -1.0 {
        return false;
    }
    let (cx, cy) = (cx as i64, cy as i64);
    (cy - 1..=cy + 1).any(|r| {
        (cx - 1..=cx + 1).any(|c| {
            r >= 0 && c >= 0 && (r as usize) < rows && (c as usize) < cols && map[[r as usize, c as usize]] != 0
        })
    })
}

fn unusable_saturation(path: &str, dims: (usize, usize)) -> Option<ndarray::Array2<u8>> {
    let (entry, table) = load_companions(path).ok()?.dq?;
    let saturated = table.mask_from_names(&[PHOTOMETRY_SATURATED_FLAG]).ok()?;
    let plane = entry.int_plane().filter(|plane| plane.bits.dim() == dims)?;
    Some(unusable_saturation_map(&plane.bits, saturated, table.exclusion_mask()))
}

pub(super) fn collect_detections(
    measured: impl Iterator<Item = TieDetection>,
    unsaturated_target: usize,
    scan_limit: usize,
) -> Vec<TieDetection> {
    let mut kept = Vec::new();
    let mut unsaturated = 0;
    for detection in measured {
        unsaturated += usize::from(!detection.saturated);
        kept.push(detection);
        if unsaturated >= unsaturated_target || kept.len() >= scan_limit {
            break;
        }
    }
    kept
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FwhmOutliers {
    pub flagged: usize,
    pub median_px: f64,
    pub low_px: f64,
    pub high_px: f64,
}

pub(super) fn flag_fwhm_outliers(mut detections: Vec<TieDetection>) -> (Vec<TieDetection>, Option<FwhmOutliers>) {
    let fwhm: Vec<f64> = detections.iter().filter(|d| !d.saturated && d.fwhm_px.is_finite() && d.fwhm_px > 0.0).map(|d| d.fwhm_px).collect();
    if fwhm.is_empty() {
        return (detections, None);
    }
    let median_px = crate::math::exact_median_f64(&fwhm);
    let (low_px, high_px) = (median_px / FWHM_OUTLIER_FACTOR, median_px * FWHM_OUTLIER_FACTOR);
    let mut flagged = 0;
    for detection in detections.iter_mut().filter(|d| !d.saturated && !(low_px..=high_px).contains(&d.fwhm_px)) {
        detection.saturated = true;
        flagged += 1;
    }
    (detections, Some(FwhmOutliers { flagged, median_px, low_px, high_px }))
}

pub(super) fn unusable_detection(star: &DetectedStar) -> TieDetection {
    TieDetection { x: star.x, y: star.y, fwhm_px: star.fwhm, snr: star.snr, mag_inst: f64::NAN, saturated: true }
}

pub(super) fn measured_detection(
    star: &DetectedStar,
    phot: Result<StarPhotometry, String>,
    saturated_core: impl FnOnce(f64, f64) -> bool,
) -> Option<TieDetection> {
    let phot = phot.ok()?;
    (phot.net_flux.is_finite() && phot.net_flux > 0.0 && phot.x.is_finite() && phot.y.is_finite()).then(|| TieDetection {
        x: phot.x,
        y: phot.y,
        fwhm_px: star.fwhm,
        snr: phot.snr,
        mag_inst: phot.mag_inst,
        saturated: phot.saturated || saturated_core(phot.x, phot.y),
    })
}

#[derive(Default)]
pub(super) struct TieSources {
    pub detections: Vec<TieDetection>,
    pub n_detected: usize,
    pub n_dq_rejected: usize,
    pub n_unmeasured: usize,
    pub dq_mask_error: Option<String>,
    pub fwhm_outliers: Option<FwhmOutliers>,
}

pub(super) fn source_warnings(sources: &TieSources) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(error) = &sources.dq_mask_error {
        warnings.push(format!("the DQ mask could not be applied to the photometry ({error}); the detections were measured without it"));
    }
    if sources.n_dq_rejected > 0 {
        warnings.push(format!(
            "{} detections centred on DQ-flagged pixels kept out of the fit; the tie counts them as saturated and uses them for the blend cut",
            sources.n_dq_rejected
        ));
    }
    if let Some(outliers) = sources.fwhm_outliers.filter(|o| o.flagged > 0) {
        warnings.push(format!(
            "{} detections with a FWHM outside {:.2} to {:.2} px (median {:.2} px) kept out of the fit; the tie counts them as saturated and uses them for the blend cut",
            outliers.flagged, outliers.low_px, outliers.high_px, outliers.median_px
        ));
    }
    if sources.n_unmeasured > 0 {
        warnings.push(format!(
            "{} of {} detections could not be measured (photometry failed or found no positive flux) and were left out",
            sources.n_unmeasured, sources.n_detected
        ));
    }
    warnings
}

pub(super) fn tie_sources(path: &str, entry: &ImageEntry, header: &HduHeader) -> TieSources {
    let image = entry.arr();
    let dims = image.dim();
    let planes = photometry_planes(path, dims);
    let dq = resolve_dq_mask(path, true, dims);
    let saturated = unusable_saturation(path, dims);
    let stars = detect_stars(image, TIE_DETECTION_SIGMA).stars;
    let n_detected = stars.len();
    let (level, level_source) = saturation_level(Some(header), entry.stats().max);
    let config = PhotometryConfig { saturation: Some((level, level_source)), ..PhotometryConfig::default() };
    let excluded_at = |x: f64, y: f64| {
        dq.as_ref().is_some_and(|mask| {
            let (r, c) = (y.round(), x.round());
            r >= 0.0 && c >= 0.0 && mask.map.get((r as usize, c as usize)).is_some_and(|&v| v != 0)
        })
    };
    let masked = MaskedImage::new(image, dq.as_ref().map(|mask| &mask.map));
    let dq_mask_error = masked.as_ref().err().cloned();
    let prepared = masked.or_else(|_| MaskedImage::new(image, None));
    let mut n_dq_rejected = 0;
    let mut n_unmeasured = 0;
    let measured = stars.iter().filter_map(|star| {
        if excluded_at(star.x, star.y) {
            n_dq_rejected += 1;
            return Some(unusable_detection(star));
        }
        let phot = prepared
            .as_ref()
            .map_err(String::clone)
            .and_then(|source| measure_star_prepared(source, planes.err.as_ref().map(|e| e.arr()), None, star.x, star.y, &config));
        let detection = measured_detection(star, phot, |x, y| saturated.as_ref().is_some_and(|map| core_saturated(map, x, y)));
        n_unmeasured += usize::from(detection.is_none());
        detection
    });
    let (detections, fwhm_outliers) = flag_fwhm_outliers(collect_detections(measured, TIE_MAX_DETECTIONS, TIE_SCAN_LIMIT));
    TieSources { detections, n_detected, n_dq_rejected, n_unmeasured, dq_mask_error, fwhm_outliers }
}

pub(super) fn merged_cards(input: &[(String, String)], updates: &[(String, String)], remove: &[String]) -> Vec<(String, String)> {
    let mut header = HduHeader::empty();
    for (key, value) in input {
        let key = key.trim();
        if !remove.iter().any(|replaced| replaced == key) {
            header.set(key, value.clone());
        }
    }
    for (key, value) in updates {
        header.set(key.trim(), value.clone());
    }
    header.cards
}

pub(super) fn represent(
    input: &InputWcs,
    result: &TieResult,
    (cols, rows): (usize, usize),
) -> Result<(Vec<(String, String)>, String), String> {
    if let Some(update) = &result.header_update {
        return Ok((merged_cards(&input.cards, &update.cards, &update.remove), EXACT_REPRESENTATION.to_string()));
    }
    let correction = result.correction.as_ref().ok_or("the tie returned no correction to apply")?;
    let eval = |x: f64, y: f64| {
        correction.corrected_pixel_to_world(&input.wcs, x, y).map_or([f64::NAN, f64::NAN], |(ra, dec)| [ra, dec])
    };
    let bbox = [[-0.5, cols as f64 - 0.5], [-0.5, rows as f64 - 0.5]];
    let options = SipFitOptions { max_pix_error: REFIT_TARGET_PX, ..SipFitOptions::default() };
    let fit = fit_tan_sip(&eval, bbox, &options).map_err(|e| format!("the TAN-SIP refit of the tied WCS failed: {e}"))?;
    if fit.max_err_px.is_nan() || fit.max_err_px > REFIT_LIMIT_PX {
        return Err(format!(
            "the TAN-SIP refit of the tied WCS reaches {:.4} px at order {}, above the {REFIT_LIMIT_PX} px limit",
            fit.max_err_px, fit.a_order
        ));
    }
    Ok((fit.header_cards(), format!("TAN-SIP refit order {}, max error {:.4} px", fit.a_order, fit.max_err_px)))
}

fn decision_name(decision: TieDecision) -> &'static str {
    match decision {
        TieDecision::Applied => "applied",
        TieDecision::NoSignificantCorrection => "no_significant_correction",
        TieDecision::Refused => "refused",
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn holdout(value: f64) -> Option<f64> {
    finite(value).filter(|v| *v > 0.0)
}

fn signed(value: Option<f64>, digits: usize) -> String {
    value.map_or_else(|| "n/a".to_string(), |v| format!("{v:+.digits$}"))
}

fn plain(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |v| format!("{v:.1}"))
}

pub(super) fn utc_date(unix_s: u64) -> String {
    let (year, month, day, ..) = gregorian_from_jd(UNIX_EPOCH_JD + unix_s as f64 / SECONDS_PER_DAY);
    format!("{year:04}-{month:02}-{day:02}")
}

pub(super) fn refine_json(
    result: &TieResult,
    refined: Option<(Vec<(String, String)>, String)>,
    wcs_kind: &str,
    epoch: &TieEpochChoice,
    n_detected: usize,
    cache_source: &str,
    mut warnings: Vec<String>,
) -> Value {
    let report = &result.report;
    let fitted = report.model.is_some() && report.after.n > 0;
    let fit = |value: f64| finite(value).filter(|_| fitted);
    let rms = |summary: &crate::core::astrometry::gaia_tie::ResidualSummary| {
        finite(summary.robust_rms_mas[2]).filter(|_| summary.n > 0)
    };
    let sigma = report.sigma_bootstrap.unwrap_or(report.sigma_formal);
    let with_scale = matches!(report.model.as_deref(), Some(m) if m == TieModel::Rscale.name() || m == TieModel::General.name());
    let with_rotation = report.model.as_deref().is_some_and(|m| m != TieModel::Shift.name());
    let applied = result.decision == TieDecision::Applied && refined.is_some();
    let (cards, representation) = refined.filter(|_| applied).map_or((Vec::new(), None), |(c, r)| (c, Some(r)));
    warnings.extend(report.model_notes.iter().cloned());
    warnings.extend(report.warnings.iter().cloned());
    let mut body = json!({
        "decision": decision_name(result.decision),
        "reason": result.reason,
        "wcs_cards": cards,
        "history": Vec::<String>::new(),
        "model": report.model,
        "wcs_kind": wcs_kind,
        "representation": representation,
        "n_catalog": report.catalogue_rows,
        "n_detected": n_detected,
        "n_matched": report.n_matched,
        "n_used": report.n_used,
        "rms_before_mas": rms(&report.before),
        "rms_after_mas": rms(&report.after),
        "holdout_rms_before_mas": holdout(report.cv_rms_before_mas),
        "holdout_rms_after_mas": holdout(report.cv_rms_after_mas),
        "shift_east_mas": fit(report.params.shift_mas[0]),
        "shift_north_mas": fit(report.params.shift_mas[1]),
        "shift_err_east_mas": fit(sigma.shift_mas[0]),
        "shift_err_north_mas": fit(sigma.shift_mas[1]),
        "rotation_deg": fit(report.params.rotation_deg).filter(|_| with_rotation),
        "rotation_err_deg": fit(sigma.rotation_deg).filter(|_| with_rotation),
        "scale_ppm": fit(report.params.scale_ppm).filter(|_| with_scale),
        "scale_err_ppm": fit(sigma.scale_ppm).filter(|_| with_scale),
        "sigma_sys_mas": fit(report.sigma_sys_mas),
        "epoch_year": epoch.year,
        "epoch_source": epoch.source_name(),
        "epoch_systematic_mas": report.epoch_systematic_mas,
        "catalog": GAIA_DR3_CATALOGUE,
        "catalog_source": cache_source,
        "attribution": GAIA_TIE_SHORT_ATTRIBUTION,
        "citation": GAIA_DR3_ATTRIBUTION,
        "warnings": warnings,
        "report": report,
    });
    if applied {
        let epoch_text = match epoch.year {
            Some(year) => format!("epoch {year:.3} ({})", epoch.source_name()),
            None => "epoch J2016.0 (no date in the header)".to_string(),
        };
        body["history"] = json!([
            format!(
                "Gaia DR3 tie: {}, {} of {} pairs, hold-out RMS {} to {} mas, robust RMS {} to {} mas",
                report.model.as_deref().unwrap_or("n/a"),
                report.n_used,
                report.n_matched,
                plain(holdout(report.cv_rms_before_mas)),
                plain(holdout(report.cv_rms_after_mas)),
                plain(rms(&report.before)),
                plain(rms(&report.after)),
            ),
            format!(
                "Gaia DR3 tie: shift {} mas E {} mas N{}{}",
                signed(fit(report.params.shift_mas[0]), 1),
                signed(fit(report.params.shift_mas[1]), 1),
                if with_rotation { format!(", rotation {} deg", signed(fit(report.params.rotation_deg), 5)) } else { String::new() },
                if with_scale { format!(", scale {} ppm", signed(fit(report.params.scale_ppm), 1)) } else { String::new() },
            ),
            format!("Gaia DR3 tie: {epoch_text}, input WCS {wcs_kind}, {}", body["representation"].as_str().unwrap_or("")),
            format!("Gaia DR3 tie: date {} UTC", utc_date(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()))),
            GAIA_DR3_ATTRIBUTION,
            GAIA_ESA_ACKNOWLEDGEMENT,
        ]);
    }
    body
}

fn model_choice(model: Option<&str>) -> anyhow::Result<ModelChoice> {
    let name = model.map(str::trim).filter(|m| !m.is_empty()).unwrap_or("auto").to_ascii_lowercase();
    Ok(match name.as_str() {
        "auto" => ModelChoice::Auto,
        "shift" => ModelChoice::Fixed(TieModel::Shift),
        "rshift" => ModelChoice::Fixed(TieModel::Rshift),
        "rscale" => ModelChoice::Fixed(TieModel::Rscale),
        "general" => ModelChoice::Fixed(TieModel::General),
        other => anyhow::bail!("unknown tie model '{other}': use auto, shift, rshift, rscale or general"),
    })
}

pub(super) fn gaia_refine_for_path(
    path: &str,
    solve_cards: Option<&[(String, String)]>,
    typed_epoch: Option<f64>,
    model: Option<&str>,
    cache_dir: Option<&Path>,
    fetch: TieFetch<'_>,
) -> anyhow::Result<Value> {
    let started = Instant::now();
    let model = model_choice(model)?;
    let entry = load_cached_full(path)?;
    let header = entry.header().cloned().ok_or_else(|| anyhow::anyhow!("no FITS header available for {path}"))?;
    let (rows, cols) = entry.arr().dim();
    let input = input_wcs(path, &header, (cols, rows), solve_cards)?;
    let epoch = tie_epoch(&header, typed_epoch)?;
    let sources = tie_sources(path, &entry, &header);
    let usable = sources.detections.iter().filter(|d| !d.saturated).count().min(TIE_MAX_DETECTIONS);
    let query = tie_cone(&input.wcs, (cols, rows), usable);
    let catalogue = tie_catalogue(&query, cache_dir, fetch).map_err(|e| anyhow::anyhow!(e))?;
    let mut warnings = source_warnings(&sources);
    if catalogue.rows.len() >= query.max_rows {
        warnings.push(format!("the Gaia DR3 cone reached its {}-row cap, so the faintest stars were left out", query.max_rows));
    }
    let options = TieOptions {
        model,
        max_detections: TIE_MAX_DETECTIONS,
        resampled_pixels: text_card(&header, RESAMPLED_KEY).is_some_and(|v| v == "COMPLETE"),
        ..TieOptions::default()
    };
    let tie_input = TieInput {
        wcs: &input.wcs,
        width: cols,
        height: rows,
        detections: &sources.detections,
        catalogue: &catalogue,
        epoch: epoch.epoch(),
        observer_barycentric_au: tie_observer(&header, &epoch),
        space_telescope: space_telescope(&header),
    };
    let mut result = tie_to_gaia(&tie_input, &options);
    let refined = if result.decision == TieDecision::Applied {
        match represent(&input, &result, (cols, rows)) {
            Ok(refined) => Some(refined),
            Err(reason) => {
                result.decision = TieDecision::Refused;
                result.reason = Some(reason);
                None
            }
        }
    } else {
        None
    };
    let mut body = refine_json(&result, refined, input.kind, &epoch, sources.n_detected, &catalogue.cache_source, warnings);
    body["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    Ok(body)
}

#[tauri::command]
pub async fn gaia_refine_wcs_cmd(
    path: String,
    wcs_cards: Option<Vec<(String, String)>>,
    epoch_year: Option<f64>,
    model: Option<String>,
) -> Result<Value, String> {
    blocking_cmd!(gaia_refine_for_path(
        &path,
        wcs_cards.as_deref(),
        epoch_year,
        model.as_deref(),
        catalog_disk::cache_dir(),
        &query_gaia_tie_tsv,
    ))
}

#[cfg(test)]
mod live_validation;

#[cfg(test)]
mod tests {
    use crate::core::imaging::region::test_support::{make_header, north_up_cd, wcs_cards};

    const TIE_HISTORY: [&str; 2] = [
        "Gaia DR3 tie: rshift, 97 of 104 pairs, RMS 14.9 mas, shift +1029.4 mas E +1438.5 mas N, rotation +0.00125 deg",
        "Gaia DR3 (Gaia Collaboration, Vallenari et al. 2023, A&A 674, A1), ESA/Gaia/DPAC, CC BY-NC 3.0 IGO, via VizieR (CDS)",
    ];

    fn sky_cards() -> Vec<(String, String)> {
        wcs_cards(north_up_cd()).into_iter().filter(|(k, _)| !k.starts_with("NAXIS")).collect()
    }

    fn source_fits(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name).to_string_lossy().to_string();
        let cards = sky_cards();
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let arr = ndarray::Array2::<f32>::from_elem((48, 64), 1.0);
        crate::infra::fits::writer::write_fits_mono(&path, &arr, Some(&make_header(&pairs))).unwrap();
        path
    }

    fn written_header(result: &serde_json::Value) -> crate::types::header::HduHeader {
        let fits_path = result["fits_path"].as_str().unwrap().to_string();
        crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&fits_path)).unwrap()
    }

    fn history_of(header: &crate::types::header::HduHeader) -> Vec<String> {
        header.cards.iter().filter(|(k, _)| k.trim() == "HISTORY").map(|(_, v)| v.trim().to_string()).collect()
    }

    #[test]
    fn a_gaia_tied_copy_carries_its_marker_and_the_attribution_in_wrapped_ascii_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = source_fits(&dir, "tied_src.fits");
        let out = dir.path().join("out").to_string_lossy().to_string();
        let history: Vec<String> = TIE_HISTORY.iter().map(|s| s.to_string()).collect();

        let result = super::super::write_wcs_copy(&path, &sky_cards(), &out, Some(&history)).unwrap();

        let header = written_header(&result);
        assert_eq!(header.get("ABPROC").map(str::trim), Some(super::ABPROC_GAIA_TIED));
        assert_eq!(super::ABPROC_GAIA_TIED, "gaiatied");
        let lines = history_of(&header);
        for line in &lines {
            assert!(line.len() <= super::HISTORY_WIDTH, "{} chars: {line}", line.len());
            assert!(line.is_ascii(), "{line}");
        }
        let joined = lines.join(" ");
        for text in TIE_HISTORY {
            assert!(joined.contains(text), "{text} missing from {joined}");
        }
        let crval1 = sky_cards().into_iter().find(|(k, _)| k == "CRVAL1").and_then(|(_, v)| v.trim().parse::<f64>().ok());
        assert!(crval1.is_some());
        assert_eq!(header.get_f64("CRVAL1"), crval1);
    }

    #[test]
    fn a_tied_copy_written_with_radesys_drops_the_deprecated_radecsys_of_its_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fk5_src.fits").to_string_lossy().to_string();
        let mut source = sky_cards();
        source.push(("RADECSYS".to_string(), "'FK5'".to_string()));
        source.push(("EQUINOX".to_string(), "2000.0".to_string()));
        let pairs: Vec<(&str, &str)> = source.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::infra::fits::writer::write_fits_mono(&path, &ndarray::Array2::<f32>::from_elem((48, 64), 1.0), Some(&make_header(&pairs))).unwrap();
        let mut tied = sky_cards();
        tied.push(("RADESYS".to_string(), "'ICRS'".to_string()));
        let out = dir.path().join("out").to_string_lossy().to_string();
        let history: Vec<String> = TIE_HISTORY.iter().map(|s| s.to_string()).collect();

        let header = written_header(&super::super::write_wcs_copy(&path, &tied, &out, Some(&history)).unwrap());

        assert_eq!(header.get("RADESYS").map(|v| v.trim().trim_matches('\'').trim()), Some("ICRS"));
        assert!(header.get("RADECSYS").is_none(), "RADECSYS = {:?} survives next to RADESYS = ICRS", header.get("RADECSYS"));
        assert!(header.get("EQUINOX").is_none(), "EQUINOX = {:?} survives next to RADESYS = ICRS", header.get("EQUINOX"));
    }

    #[test]
    fn a_plain_solve_copy_stays_plate_solved_without_tie_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = source_fits(&dir, "plain_src.fits");
        let out = dir.path().join("out").to_string_lossy().to_string();

        for history in [None, Some(Vec::new())] {
            let result = super::super::write_wcs_copy(&path, &sky_cards(), &out, history.as_deref()).unwrap();
            let header = written_header(&result);
            assert_eq!(header.get("ABPROC").map(str::trim), Some("platesolved"));
            assert!(history_of(&header).iter().all(|line| !line.contains("Gaia DR3")), "{:?}", history_of(&header));
        }
    }

    #[test]
    fn history_lines_wrap_at_word_boundaries_and_drop_non_ascii() {
        let wrapped = super::history_cards(&["Gaia DR3 tie: RMS 14.9 mas, 1\u{3c3} 2.1 mas, rotation +0.00125\u{b0}".to_string()]);
        assert_eq!(wrapped, vec!["Gaia DR3 tie: RMS 14.9 mas, 1 2.1 mas, rotation +0.00125".to_string()]);
        let long = "word ".repeat(40);
        let lines = super::history_cards(&[long.trim().to_string()]);
        assert!(lines.len() >= 3, "{lines:?}");
        assert!(lines.iter().all(|l| l.len() <= super::HISTORY_WIDTH && !l.starts_with(' ') && !l.ends_with(' ')), "{lines:?}");
        assert_eq!(lines.join(" "), long.trim());
    }

    use crate::core::astrometry::catalog::ConeQuery;
    use crate::core::astrometry::gaia_tie::{
        header_update, unit_vector, ResidualSummary, SkyCorrection, TieDecision, TieReport, TieResult,
    };
    use crate::core::astrometry::wcs::WcsTransform;
    use crate::types::header::HduHeader;

    const TIE_COLUMNS_LINE: &str = "Source\tRA_ICRS\tDE_ICRS\te_RA_ICRS\te_DE_ICRS\tpmRA\te_pmRA\tpmDE\te_pmDE\tPlx\te_Plx\tRApmRAcor\tDEpmDEcor\tRUWE\tsepsi\tSolved\tDup\tIPDfmp\tGmag\tBP-RP";

    fn header_of(cards: &[(String, String)], size: (usize, usize)) -> HduHeader {
        let mut header = HduHeader::empty();
        for (k, v) in cards {
            header.set(k, v.clone());
        }
        header.set("NAXIS1", size.0.to_string());
        header.set("NAXIS2", size.1.to_string());
        header
    }

    fn tan_cards(crval: (f64, f64), scale_arcsec: f64, size: (usize, usize)) -> Vec<(String, String)> {
        let s = scale_arcsec / 3600.0;
        vec![
            ("CTYPE1".into(), "RA---TAN".into()),
            ("CTYPE2".into(), "DEC--TAN".into()),
            ("CRPIX1".into(), format!("{:.1}", (size.0 as f64 + 1.0) / 2.0)),
            ("CRPIX2".into(), format!("{:.1}", (size.1 as f64 + 1.0) / 2.0)),
            ("CRVAL1".into(), format!("{:.12}", crval.0)),
            ("CRVAL2".into(), format!("{:.12}", crval.1)),
            ("CD1_1".into(), format!("{:.16E}", -s)),
            ("CD1_2".into(), "0.0".into()),
            ("CD2_1".into(), "0.0".into()),
            ("CD2_2".into(), format!("{:.16E}", s)),
        ]
    }

    fn tsv_of(rows: &[(f64, f64, f64)]) -> String {
        let mut body = format!("#\n{TIE_COLUMNS_LINE}\n");
        for (i, (ra, dec, g)) in rows.iter().enumerate() {
            body.push_str(&format!(
                "{}\t{ra:.11}\t{dec:.11}\t0.0500\t0.0500\t0.000\t0.020\t0.000\t0.020\t0.0000\t0.0300\t0.0000\t0.0000\t1.000\t0.000e+00\t31\t0\t0\t{g:.6}\t0.800000\n",
                9_000_000 + i
            ));
        }
        body
    }

    struct Field {
        path: String,
        truth: WcsTransform,
        sky: Vec<(f64, f64, f64)>,
        size: (usize, usize),
    }

    fn synthetic_field(dir: &tempfile::TempDir, name: &str, injected_arcsec: (f64, f64)) -> Field {
        synthetic_field_with(dir, name, injected_arcsec, &[])
    }

    fn synthetic_field_with(dir: &tempfile::TempDir, name: &str, injected_arcsec: (f64, f64), extra: &[(&str, &str)]) -> Field {
        let size = (400usize, 300usize);
        let centre = (150.0, 2.0);
        let truth_cards = tan_cards(centre, 1.0, size);
        let truth = WcsTransform::from_header(&header_of(&truth_cards, size)).unwrap();
        let cos_dec = centre.1.to_radians().cos();
        let shifted = (centre.0 - injected_arcsec.0 / 3600.0 / cos_dec, centre.1 - injected_arcsec.1 / 3600.0);
        let file_cards = tan_cards(shifted, 1.0, size);
        let mut image = ndarray::Array2::<f32>::zeros((size.1, size.0));
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut noise = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 2001) as f32 / 1000.0 - 1.0
        };
        for v in image.iter_mut() {
            *v = 100.0 + 3.0 * noise();
        }
        let mut sky = Vec::new();
        let sigma = 1.3_f64;
        for i in 0..64usize {
            let x = 18.0 + (i % 8) as f64 * 47.3 + 0.37 * (i as f64 % 3.0);
            let y = 16.0 + (i / 8) as f64 * 35.1 + 0.29 * (i as f64 % 5.0);
            let peak = 3000.0 + 400.0 * (i % 7) as f64;
            for r in (y as usize).saturating_sub(8)..((y as usize) + 9).min(size.1) {
                for c in (x as usize).saturating_sub(8)..((x as usize) + 9).min(size.0) {
                    let d2 = (c as f64 - x).powi(2) + (r as f64 - y).powi(2);
                    image[[r, c]] += (peak * (-d2 / (2.0 * sigma * sigma)).exp()) as f32;
                }
            }
            let p = truth.pixel_to_world(x, y);
            sky.push((p.ra, p.dec, 22.0 - 2.5 * peak.log10()));
        }
        let path = dir.path().join(name).to_string_lossy().to_string();
        let mut file_cards = file_cards;
        file_cards.push(("SATURATE".to_string(), "60000.0".to_string()));
        file_cards.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let pairs: Vec<(&str, &str)> = file_cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::infra::fits::writer::write_fits_mono(&path, &image, Some(&make_header(&pairs))).unwrap();
        Field { path, truth, sky, size }
    }

    fn render_star(image: &mut ndarray::Array2<f32>, (x, y): (f64, f64), peak: f64, sigma: f64) {
        let (rows, cols) = image.dim();
        let reach = (4.0 * sigma).ceil() as usize + 1;
        for r in (y as usize).saturating_sub(reach)..((y as usize) + reach + 1).min(rows) {
            for c in (x as usize).saturating_sub(reach)..((x as usize) + reach + 1).min(cols) {
                let d2 = (c as f64 - x).powi(2) + (r as f64 - y).powi(2);
                image[[r, c]] += (peak * (-d2 / (2.0 * sigma * sigma)).exp()) as f32;
            }
        }
    }

    const STELLAR: [(f64, f64); 5] = [(15.3, 15.2), (45.4, 14.8), (80.2, 16.1), (25.2, 45.3), (60.3, 80.4)];
    const ON_DO_NOT_USE: (f64, f64) = (70.0, 50.0);
    const BROAD: (f64, f64) = (35.0, 78.0);

    fn dq_star_mef(dir: &tempfile::TempDir) -> String {
        use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};
        let size = 96usize;
        let mut image = ndarray::Array2::<f32>::from_shape_fn((size, size), |(r, c)| 100.0 + ((r * 7 + c * 13) % 5) as f32 * 0.2);
        for star in STELLAR {
            render_star(&mut image, star, 4000.0, 1.3);
        }
        render_star(&mut image, ON_DO_NOT_USE, 4000.0, 1.3);
        render_star(&mut image, BROAD, 1500.0, 4.5);
        let mut dq = vec![i32::MIN; size * size];
        dq[ON_DO_NOT_USE.1 as usize * size + ON_DO_NOT_USE.0 as usize] = i32::MIN + 1;
        let path = dir.path().join("dq_stars_cal.fits");
        write_test_mef(
            &path,
            &[("TELESCOP", "'JWST'".into())],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: size, rows: size, data: HduData::F32(image.iter().copied().collect()), extra_cards: vec![("SATURATE", "60000.0".into())] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: size, rows: size, data: HduData::I32(dq), extra_cards: vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into()), ("TELESCOP", "'JWST'".into())] },
            ],
        );
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn detections_on_do_not_use_pixels_and_fwhm_outliers_reach_the_tie_as_unusable_so_its_blend_cut_sees_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dq_star_mef(&dir);
        let entry = crate::cmd::common::load_cached_full(&path).unwrap();
        let header = entry.header().cloned().unwrap();

        let sources = super::tie_sources(&path, &entry, &header);

        let at = |(x, y): (f64, f64)| sources.detections.iter().find(|d| (d.x - x).hypot(d.y - y) < 1.5).copied();
        let listed: Vec<(f64, f64, bool)> = sources.detections.iter().map(|d| (d.x, d.y, d.saturated)).collect();
        assert_eq!(sources.n_dq_rejected, 1, "{listed:?}");
        let on_dq = at(ON_DO_NOT_USE).unwrap_or_else(|| panic!("the detection centred on the DO_NOT_USE pixel is missing: {listed:?}"));
        assert!(on_dq.saturated, "{on_dq:?}");
        let broad = at(BROAD).unwrap_or_else(|| panic!("the broad source is missing: {listed:?}"));
        assert!(broad.saturated, "{broad:?}");
        assert_eq!(sources.fwhm_outliers.map(|o| o.flagged), Some(1));
        assert_eq!((sources.n_unmeasured, sources.dq_mask_error.as_deref()), (0, None));
        for star in STELLAR {
            assert!(at(star).is_some_and(|d| !d.saturated), "{star:?} in {listed:?}");
        }
        assert_eq!(sources.detections.len(), STELLAR.len() + 2, "{listed:?}");
    }

    #[test]
    fn unsaturated_detections_far_from_the_stellar_fwhm_are_flagged_unusable_and_kept() {
        let detection = |fwhm_px: f64, saturated: bool| crate::core::astrometry::gaia_tie::TieDetection {
            x: 1.0,
            y: 1.0,
            fwhm_px,
            snr: 50.0,
            mag_inst: -9.0,
            saturated,
        };
        let mut detections: Vec<_> = (0..10).map(|_| detection(3.5, false)).collect();
        detections.push(detection(0.9, false));
        detections.push(detection(12.0, false));
        detections.push(detection(15.0, true));
        detections.push(detection(f64::NAN, false));
        let (kept, outliers) = super::flag_fwhm_outliers(detections);
        assert_eq!(kept.len(), 14);
        assert_eq!(kept.iter().filter(|d| d.saturated).count(), 4);
        assert!(kept.iter().all(|d| d.saturated || d.fwhm_px == 3.5));
        assert!(kept[10..].iter().all(|d| d.saturated), "{:?}", &kept[10..]);
        let outliers = outliers.unwrap();
        assert_eq!((outliers.flagged, outliers.median_px, outliers.low_px, outliers.high_px), (3, 3.5, 1.75, 7.0));
        let (all, none) = super::flag_fwhm_outliers(vec![detection(4.0, true)]);
        assert_eq!(all.len(), 1);
        assert!(none.is_none());
    }

    #[test]
    fn a_neighbour_saturated_pixel_does_not_flag_a_star_but_its_own_core_does() {
        let mut map = ndarray::Array2::<u8>::zeros((20, 20));
        map[[10, 15]] = 1;
        assert!(!super::core_saturated(&map, 10.0, 10.0), "a pixel 5 px away belongs to a neighbour");
        assert!(super::core_saturated(&map, 14.4, 10.6), "the 3x3 core around (14, 11) holds the flagged pixel");
        assert!(super::core_saturated(&map, 15.0, 10.0));
        assert!(!super::core_saturated(&map, -3.0, 40.0), "outside the image");
    }

    #[test]
    fn saturated_detections_do_not_use_up_the_brightest_slots() {
        let detection = |saturated: bool| crate::core::astrometry::gaia_tie::TieDetection {
            x: 1.0,
            y: 1.0,
            fwhm_px: 2.0,
            snr: 100.0,
            mag_inst: -10.0,
            saturated,
        };
        let consumed = std::cell::Cell::new(0usize);
        let measured = (0..10_000).map(|i| {
            consumed.set(consumed.get() + 1);
            detection(i < 600)
        });
        let kept = super::collect_detections(measured, 500, 5_000);
        assert_eq!(kept.len(), 1_100);
        assert_eq!(kept.iter().filter(|d| d.saturated).count(), 600);
        assert_eq!(consumed.get(), 1_100);
        let capped = super::collect_detections((0..10_000).map(|_| detection(true)), 500, 5_000);
        assert_eq!(capped.len(), 5_000);
    }

    #[test]
    fn only_saturated_pixels_that_are_also_unusable_flag_a_detection_as_saturated() {
        let bits = ndarray::arr2(&[[0u32, 2, 3], [1, 2 | 4, 3 | 4]]);
        let map = super::unusable_saturation_map(&bits, 2, 1);
        assert_eq!(map, ndarray::arr2(&[[0u8, 0, 1], [0, 0, 1]]));
        let without_unusable_flags = super::unusable_saturation_map(&bits, 2, 0);
        assert_eq!(without_unusable_flags, ndarray::arr2(&[[0u8, 1, 1], [0, 1, 1]]));
    }

    #[test]
    fn the_solve_cards_are_refined_instead_of_the_header() {
        let size = (100, 80);
        let header = header_of(&tan_cards((150.0, 2.0), 1.0, size), size);
        let solve = tan_cards((151.0, 3.0), 1.0, size);
        let input = super::input_wcs("C:/unused.fits", &header, size, Some(&solve)).unwrap();
        assert_eq!(input.kind, "solve");
        let (_, _, crval1, crval2, _, _) = input.wcs.raw_params();
        assert_eq!((crval1, crval2), (151.0, 3.0));
        assert!(input.cards.iter().any(|(k, v)| k == "CRVAL1" && v.trim() == "151.000000000000"));
    }

    #[test]
    fn a_frame_without_a_wcs_and_without_a_solve_is_refused_before_anything_goes_online() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nowcs.fits").to_string_lossy().to_string();
        crate::infra::fits::writer::write_fits_mono(&path, &ndarray::Array2::<f32>::zeros((20, 30)), None).unwrap();
        let calls = std::cell::Cell::new(0);
        let fetch = |_: &ConeQuery| {
            calls.set(calls.get() + 1);
            Ok(String::new())
        };
        let err = format!("{:#}", super::gaia_refine_for_path(&path, None, None, None, None, &fetch).unwrap_err());
        assert!(err.starts_with("no celestial WCS to refine"), "{err}");
        assert!(err.contains("plate solve the image first"), "{err}");
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn the_tie_cone_circumscribes_the_frame_plus_the_search_radius_and_caps_rows_by_area() {
        let size = (100, 100);
        let wcs = WcsTransform::from_header(&header_of(&tan_cards((150.0, 2.0), 1.0, size), size)).unwrap();
        let q = super::tie_cone(&wcs, size, 500);
        let expected_radius_arcsec = (2.0_f64).sqrt() * 50.0 + 6.0;
        assert!((q.radius_deg * 3600.0 - expected_radius_arcsec).abs() < 0.05, "{}", q.radius_deg * 3600.0);
        assert!((q.ra - 150.0).abs() < 1e-6 && (q.dec - 2.0).abs() < 1e-6, "{q:?}");
        assert_eq!(q.mag_limit, Some(21.0));
        let expected_rows = 2.0 * 500.0 * std::f64::consts::PI * expected_radius_arcsec.powi(2) / 10_000.0;
        assert!((q.max_rows as f64 - expected_rows).abs() <= 2.0, "{} vs {expected_rows}", q.max_rows);
        assert_eq!(super::tie_cone(&wcs, size, 1).max_rows, 200);
    }

    #[test]
    fn a_typed_epoch_overrides_the_header_and_years_outside_1900_to_2100_are_refused() {
        let mut header = HduHeader::empty();
        header.set("MJD-AVG", "59892.2285".into());
        let from_header = super::tie_epoch(&header, None).unwrap();
        assert_eq!(from_header.source, crate::core::astrometry::gaia_tie::EpochSource::Header);
        assert!((from_header.year.unwrap() - 2022.855).abs() < 1e-3, "{:?}", from_header.year);
        let typed = super::tie_epoch(&header, Some(1990.5)).unwrap();
        assert_eq!((typed.year, typed.source), (Some(1990.5), crate::core::astrometry::gaia_tie::EpochSource::Typed));
        assert_eq!(super::tie_epoch(&HduHeader::empty(), None).unwrap().source, crate::core::astrometry::gaia_tie::EpochSource::None);
        let err = format!("{:#}", super::tie_epoch(&header, Some(1850.0)).unwrap_err());
        assert_eq!(err, "the typed epoch 1850 is not a year between 1900 and 2100");
    }

    #[test]
    fn jwst_position_cards_give_the_observer_and_a_dated_ground_frame_uses_the_earth() {
        let mut jwst = HduHeader::empty();
        jwst.set("TELESCOP", "'JWST'".into());
        jwst.set("JWST_X", "101064627.92143492".into());
        jwst.set("JWST_Y", "99925594.41307247".into());
        jwst.set("JWST_Z", "43442128.87295308".into());
        let b = super::observer_au(&jwst).unwrap();
        assert!((b[0] - 101_064_627.921_434_92 / 149_597_870.7).abs() < 1e-12, "{b:?}");
        assert!(super::space_telescope(&jwst));
        let mut ground = HduHeader::empty();
        ground.set("DATE-OBS", "'2024-03-20T00:00:00'".into());
        let earth = super::observer_au(&ground).unwrap();
        let r = (earth[0] * earth[0] + earth[1] * earth[1] + earth[2] * earth[2]).sqrt();
        assert!((r - 0.996).abs() < 0.02, "{earth:?}");
        assert!(!super::space_telescope(&ground));
        let mut hst = HduHeader::empty();
        hst.set("TELESCOP", "'HST'".into());
        hst.set("DATE-OBS", "'2024-03-20'".into());
        assert!(super::observer_au(&hst).is_none());
        assert!(super::observer_au(&HduHeader::empty()).is_none());
    }

    #[test]
    fn an_exact_update_replaces_crval_and_cd_and_drops_the_pc_matrix_and_the_equinox_it_replaces() {
        let input = vec![
            ("CTYPE1".to_string(), "RA---TAN-SIP".to_string()),
            ("CRPIX1".to_string(), "512.5".to_string()),
            ("CRVAL1".to_string(), "150.0".to_string()),
            ("PC1_1".to_string(), "1.0".to_string()),
            ("CDELT1".to_string(), "-2.7E-4".to_string()),
            ("EQUINOX".to_string(), "2000.0".to_string()),
            ("A_ORDER".to_string(), "2".to_string()),
        ];
        let update = vec![("CRVAL1".to_string(), "150.0001".to_string()), ("CD1_1".to_string(), "-2.7E-4".to_string())];
        let identity = SkyCorrection::from_parameters(unit_vector(150.0, 0.0), [0.0, 0.0], 0.0, 1.0);
        let remove = header_update(&identity, [[-2.7e-4, 0.0], [0.0, 2.7e-4]]).remove;
        let merged = super::merged_cards(&input, &update, &remove);
        let keys: Vec<&str> = merged.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["CTYPE1", "CRPIX1", "CRVAL1", "A_ORDER", "CD1_1"]);
        assert_eq!(merged[2].1, "150.0001");
    }

    #[test]
    fn a_typed_epoch_drops_the_observer_whose_position_belongs_to_the_header_date() {
        let mut ground = HduHeader::empty();
        ground.set("DATE-OBS", "'2024-03-20T00:00:00'".into());
        let header = super::tie_epoch(&ground, None).unwrap();
        let typed = super::tie_epoch(&ground, Some(2015.5)).unwrap();
        assert_eq!(super::tie_observer(&ground, &header), super::observer_au(&ground));
        assert!(super::tie_observer(&ground, &header).is_some());
        assert!(super::tie_observer(&ground, &typed).is_none());
        let mut jwst = HduHeader::empty();
        for (k, v) in [("JWST_X", "1.0E8"), ("JWST_Y", "1.0E8"), ("JWST_Z", "4.0E7"), ("MJD-AVG", "59892.2285")] {
            jwst.set(k, v.into());
        }
        assert!(super::tie_observer(&jwst, &super::tie_epoch(&jwst, None).unwrap()).is_some());
        assert!(super::tie_observer(&jwst, &super::tie_epoch(&jwst, Some(2020.0)).unwrap()).is_none());
    }

    #[test]
    fn the_tie_date_is_the_utc_calendar_day() {
        assert_eq!(super::utc_date(0), "1970-01-01");
        assert_eq!(super::utc_date(1_761_350_399), "2025-10-24");
        assert_eq!(super::utc_date(1_761_350_400), "2025-10-25");
        assert_eq!(super::utc_date(1_709_164_800), "2024-02-29");
    }

    fn detected(x: f64, y: f64) -> crate::core::analysis::star_detection::DetectedStar {
        crate::core::analysis::star_detection::DetectedStar { x, y, flux: 5000.0, fwhm: 3.1, eccentricity: 0.1, peak: 900.0, npix: 30, snr: 80.0 }
    }

    #[test]
    fn a_failed_measurement_is_reported_instead_of_vanishing() {
        let star = detected(12.0, 9.0);
        assert!(super::measured_detection(&star, Err("the aperture holds no usable pixel".to_string()), |_, _| false).is_none());
        let unusable = super::unusable_detection(&star);
        assert_eq!((unusable.x, unusable.y, unusable.fwhm_px, unusable.snr, unusable.saturated), (12.0, 9.0, 3.1, 80.0, true));

        let sources = super::TieSources { n_detected: 400, n_unmeasured: 37, dq_mask_error: Some("mask is 10 x 10, image is 20 x 20".to_string()), ..super::TieSources::default() };
        let warnings = super::source_warnings(&sources);
        assert!(warnings.iter().any(|w| w == "37 of 400 detections could not be measured (photometry failed or found no positive flux) and were left out"), "{warnings:?}");
        assert!(
            warnings.iter().any(|w| w.starts_with("the DQ mask could not be applied to the photometry (mask is 10 x 10, image is 20 x 20)")),
            "{warnings:?}"
        );
        assert!(super::source_warnings(&super::TieSources::default()).is_empty());
    }

    #[test]
    fn an_exact_update_drops_every_card_the_core_replaces_including_the_equinox() {
        let size = (100, 80);
        let mut cards: Vec<(String, String)> = tan_cards((150.0, 2.0), 1.0, size).into_iter().filter(|(k, _)| !k.starts_with("CD")).collect();
        for (k, v) in [("PC1_1", "1.0"), ("PC1_2", "0.0"), ("PC2_1", "0.0"), ("PC2_2", "1.0"), ("CDELT1", "-2.7777777777778E-4"), ("CDELT2", "2.7777777777778E-4"), ("EQUINOX", "2000.0")] {
            cards.push((k.to_string(), v.to_string()));
        }
        let wcs = WcsTransform::from_header(&header_of(&cards, size)).unwrap();
        let input = super::InputWcs { wcs, cards, kind: "header" };
        let correction = SkyCorrection::from_parameters(unit_vector(150.0, 2.0), [1.2, -0.7], 0.0, 1.0);
        let update = header_update(&correction, input.wcs.raw_params().4);
        let result = TieResult {
            decision: TieDecision::Applied,
            reason: None,
            report: TieReport::default(),
            correction: Some(correction),
            header_update: Some(update.clone()),
        };

        let (tied, representation) = super::represent(&input, &result, size).unwrap();

        assert_eq!(representation, "exact header update (CRVAL and CD)");
        for key in &update.remove {
            assert!(tied.iter().all(|(k, _)| k != key), "{key} survives in {tied:?}");
        }
        assert!(tied.iter().any(|(k, v)| k == "RADESYS" && v.contains("ICRS")), "{tied:?}");
        assert!(tied.iter().any(|(k, _)| k == "CRPIX1") && tied.iter().any(|(k, _)| k == "CD2_2"), "{tied:?}");
    }

    #[test]
    fn a_correction_without_exact_cards_is_refit_as_tan_sip_to_a_thousandth_of_a_pixel() {
        let size = (300, 200);
        let cards = tan_cards((80.6573, -69.5177), 0.07, size);
        let wcs = WcsTransform::from_header(&header_of(&cards, size)).unwrap();
        let input = super::InputWcs { wcs, cards, kind: "gwcs" };
        let correction = SkyCorrection::from_parameters(unit_vector(80.6573, -69.5177), [1.03, 1.44], 0.01, 1.0 + 2e-5);
        let result = TieResult {
            decision: TieDecision::Applied,
            reason: None,
            report: TieReport::default(),
            correction: Some(correction.clone()),
            header_update: None,
        };
        let (refit, representation) = super::represent(&input, &result, size).unwrap();
        assert!(representation.starts_with("TAN-SIP refit order"), "{representation}");
        let refit_wcs = WcsTransform::from_header(&header_of(&refit, size)).unwrap();
        for (x, y) in [(0.0, 0.0), (299.0, 0.0), (0.0, 199.0), (299.0, 199.0), (150.0, 100.0)] {
            let (ra, dec) = correction.corrected_pixel_to_world(&input.wcs, x, y).unwrap();
            let (px, py) = refit_wcs.world_to_pixel_batch(&[(ra, dec)])[0];
            assert!((px - x).abs() < 1e-3 && (py - y).abs() < 1e-3, "({x}, {y}) -> ({px}, {py})");
        }
        let exact = TieResult {
            header_update: Some(header_update(&correction, input.wcs.raw_params().4)),
            ..result
        };
        let (cards, representation) = super::represent(&input, &exact, size).unwrap();
        assert_eq!(representation, "exact header update (CRVAL and CD)");
        assert!(cards.iter().any(|(k, _)| k == "CRPIX1"));
    }

    #[test]
    fn the_report_follows_the_panel_contract() {
        let mut report = TieReport {
            n_matched: 104,
            n_used: 97,
            model: Some("rshift".to_string()),
            catalogue_rows: 2378,
            before: ResidualSummary { robust_rms_mas: [1200.0, 1300.0, 1766.2], n: 97, ..ResidualSummary::default() },
            after: ResidualSummary { robust_rms_mas: [10.0, 11.0, 14.9], n: 97, ..ResidualSummary::default() },
            warnings: vec!["2 saturated stars dropped".to_string()],
            attribution: crate::core::astrometry::gaia_tie::GAIA_DR3_ATTRIBUTION.to_string(),
            ..TieReport::default()
        };
        report.cv_rms_before_mas = 1772.0;
        report.cv_rms_after_mas = 34.58;
        report.params.shift_mas = [1029.4, 1438.5];
        report.params.rotation_deg = 0.00125;
        report.sigma_formal.shift_mas = [2.1, 2.3];
        report.sigma_formal.rotation_deg = 0.0004;
        let result = TieResult { decision: TieDecision::Applied, reason: None, report, correction: None, header_update: None };
        let epoch = super::TieEpochChoice { year: Some(2022.8548), source: crate::core::astrometry::gaia_tie::EpochSource::Header };
        let refined = vec![("CRVAL1".to_string(), "80.66".to_string())];
        let json = super::refine_json(&result, Some((refined.clone(), "exact header update (CRVAL and CD)".to_string())), "header", &epoch, 500, "network", Vec::new());
        assert_eq!(json["decision"], "applied");
        assert_eq!(json["wcs_cards"], serde_json::json!(refined));
        assert_eq!(json["n_catalog"], 2378);
        assert_eq!((json["n_matched"].as_u64(), json["n_used"].as_u64()), (Some(104), Some(97)));
        assert_eq!((json["rms_before_mas"].as_f64(), json["rms_after_mas"].as_f64()), (Some(1766.2), Some(14.9)));
        assert_eq!((json["shift_east_mas"].as_f64(), json["shift_north_mas"].as_f64()), (Some(1029.4), Some(1438.5)));
        assert_eq!((json["shift_err_east_mas"].as_f64(), json["rotation_err_deg"].as_f64()), (Some(2.1), Some(0.0004)));
        assert!(json["scale_ppm"].is_null(), "rshift fits no scale");
        assert_eq!(json["epoch_source"], "header");
        assert_eq!(json["catalog"], "Gaia DR3 (I/355/gaiadr3) via VizieR, CDS");
        assert_eq!(json["catalog_source"], "network");
        assert_eq!(json["attribution"], "Gaia DR3, ESA/Gaia/DPAC, via VizieR");
        assert_eq!(json["citation"], crate::core::astrometry::gaia_tie::GAIA_DR3_ATTRIBUTION);
        assert_eq!(json["representation"], "exact header update (CRVAL and CD)");
        assert_eq!(json["warnings"], serde_json::json!(["2 saturated stars dropped"]));
        let history: Vec<String> = serde_json::from_value(json["history"].clone()).unwrap();
        assert!(history.iter().any(|l| l.contains("rshift") && l.contains("97 of 104") && l.contains("hold-out RMS 1772.0 to 34.6 mas")), "{history:?}");
        assert_eq!((json["holdout_rms_before_mas"].as_f64(), json["holdout_rms_after_mas"].as_f64()), (Some(1772.0), Some(34.58)));
        assert!(history.iter().any(|l| l.contains("CC BY-NC 3.0 IGO")), "{history:?}");
        assert!(history.iter().any(|l| l.contains("European Space Agency")), "{history:?}");
        let dated = history.iter().find_map(|l| l.strip_prefix("Gaia DR3 tie: date ")).and_then(|rest| rest.strip_suffix(" UTC"));
        assert!(
            dated.is_some_and(|d| d.len() == 10 && d.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() })),
            "no tie date line in {history:?}"
        );

        let mut shift_only = result.clone();
        shift_only.report.model = Some("shift".to_string());
        shift_only.report.params.rotation_deg = -3.4e-15;
        let json = super::refine_json(&shift_only, Some((refined.clone(), "exact header update (CRVAL and CD)".to_string())), "header", &epoch, 500, "network", Vec::new());
        assert!(json["rotation_deg"].is_null() && json["rotation_err_deg"].is_null(), "a shift fits no rotation: {}", json["rotation_deg"]);
        assert!(json["history"].as_array().unwrap().iter().all(|l| !l.as_str().unwrap().contains("rotation")), "{}", json["history"]);

        let refused = TieResult {
            decision: TieDecision::Refused,
            reason: Some("9 < 15 used pairs for rshift".to_string()),
            report: TieReport { n_matched: 9, ..TieReport::default() },
            correction: None,
            header_update: None,
        };
        let json = super::refine_json(&refused, None, "header", &epoch, 500, "disk", Vec::new());
        assert_eq!(json["decision"], "refused");
        assert_eq!(json["reason"], "9 < 15 used pairs for rshift");
        assert_eq!(json["wcs_cards"], serde_json::json!([]));
        assert_eq!(json["history"], serde_json::json!([]));
        assert!(json["rms_after_mas"].is_null() && json["shift_east_mas"].is_null(), "{json}");
        let no_op = TieResult { decision: TieDecision::NoSignificantCorrection, ..refused };
        assert_eq!(super::refine_json(&no_op, None, "header", &epoch, 500, "disk", Vec::new())["decision"], "no_significant_correction");
    }

    #[test]
    fn the_tie_cone_is_served_from_memory_and_disk_after_the_first_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let q = ConeQuery { ra: 33.3331, dec: -44.4441, radius_deg: 0.02, mag_limit: Some(21.0), max_rows: 777 };
        let calls = std::cell::Cell::new(0);
        let body = tsv_of(&[(33.3331, -44.4441, 15.0), (33.3335, -44.4445, 16.0)]);
        let fetch = |_: &ConeQuery| {
            calls.set(calls.get() + 1);
            Ok(body.clone())
        };
        let first = super::tie_catalogue(&q, Some(dir.path()), &fetch).unwrap();
        assert_eq!((first.cache_source.as_str(), first.rows.len(), first.quality_columns), ("network", 2, true));
        let second = super::tie_catalogue(&q, Some(dir.path()), &fetch).unwrap();
        assert_eq!(second.cache_source, "memory");
        crate::core::astrometry::catalog::forget_tie_cones();
        let third = super::tie_catalogue(&q, Some(dir.path()), &fetch).unwrap();
        assert_eq!((third.cache_source.as_str(), third.rows.len()), ("disk", 2));
        assert_eq!(calls.get(), 1);
        let files: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].starts_with("gaia-dr3_tie_") && files[0].ends_with(".json"), "{files:?}");
    }

    #[test]
    fn a_failed_query_names_vizier_and_caches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let q = ConeQuery { ra: 12.3451, dec: 45.6781, radius_deg: 0.02, mag_limit: Some(21.0), max_rows: 200 };
        let fetch = |_: &ConeQuery| Err("VizieR request failed: error sending request for 127.0.0.1:9".to_string());
        let err = super::tie_catalogue(&q, Some(dir.path()), &fetch).unwrap_err();
        assert_eq!(err, "VizieR request failed: error sending request for 127.0.0.1:9");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_command_sends_the_cone_and_returns_a_refined_wcs_that_puts_the_stars_on_gaia() {
        let dir = tempfile::tempdir().unwrap();
        let field = synthetic_field(&dir, "tie_field.fits", (1.2, -0.7));
        let body = tsv_of(&field.sky);
        let seen = std::cell::RefCell::new(Vec::new());
        let fetch = |q: &ConeQuery| {
            seen.borrow_mut().push(q.clone());
            Ok(body.clone())
        };

        let json = super::gaia_refine_for_path(&field.path, None, None, None, None, &fetch).unwrap();

        let queries = seen.borrow();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].mag_limit, Some(21.0));
        assert!((queries[0].radius_deg * 3600.0 - (250.0 + 6.0)).abs() < 1.0, "{:?}", queries[0]);
        assert_eq!(json["wcs_kind"], "header");
        assert_eq!(json["n_catalog"], 64);
        assert_eq!(json["catalog_source"], "network");
        assert_eq!(json["epoch_source"], "none");
        assert_eq!(json["decision"], "applied", "{}", json["reason"]);
        let cards: Vec<(String, String)> = serde_json::from_value(json["wcs_cards"].clone()).unwrap();
        let refined = WcsTransform::from_header(&header_of(&cards, field.size)).unwrap();
        for (x, y) in [(0.0, 0.0), (399.0, 299.0), (200.0, 150.0)] {
            let a = refined.pixel_to_world(x, y);
            let b = field.truth.pixel_to_world(x, y);
            let d_mas = crate::core::astrometry::wcs::angular_separation(a.ra, a.dec, b.ra, b.dec) * 3_600_000.0;
            assert!(d_mas < 20.0, "({x}, {y}) is {d_mas} mas from the truth");
        }
        assert!((json["shift_east_mas"].as_f64().unwrap() - 1200.0).abs() < 20.0, "{}", json["shift_east_mas"]);
        assert!((json["shift_north_mas"].as_f64().unwrap() + 700.0).abs() < 20.0, "{}", json["shift_north_mas"]);
        assert!(json["history"].as_array().is_some_and(|h| !h.is_empty()));
    }

    #[test]
    fn a_typed_epoch_leaves_the_header_date_out_of_the_parallax() {
        let dir = tempfile::tempdir().unwrap();
        let field = synthetic_field_with(&dir, "dated_field.fits", (-0.9, 0.8), &[("DATE-OBS", "'2024-03-20T00:00:00'")]);
        let body = tsv_of(&field.sky);
        let fetch = |_: &ConeQuery| Ok(body.clone());

        let from_header = super::gaia_refine_for_path(&field.path, None, None, None, None, &fetch).unwrap();
        let typed = super::gaia_refine_for_path(&field.path, None, Some(2015.5), None, None, &fetch).unwrap();

        assert_eq!(from_header["epoch_source"], "header");
        assert_eq!(from_header["report"]["parallax"], "applied from the observer's barycentric position");
        assert_eq!(typed["epoch_source"], "typed");
        assert_eq!(typed["report"]["parallax"], "unknown observer: parallax added to the Gaia error");
    }

    fn mast_file(name: &str) -> Option<String> {
        let path = crate::core::astrometry::gwcs::test_support::mast_dir().join(name);
        if !path.is_file() {
            eprintln!("skipped: {} is absent (set ASTROBURST_MAST_DIR)", path.display());
            return None;
        }
        Some(path.to_string_lossy().into_owned())
    }

    fn live_refine(path: &str) -> serde_json::Value {
        let temp = tempfile::tempdir().unwrap();
        let cache = std::env::var_os(crate::core::astrometry::catalog_disk::ENV_CACHE_DIR)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| temp.path().to_path_buf());
        let fetch = |q: &ConeQuery| crate::core::astrometry::catalog::query_gaia_tie_tsv(q);
        let json = super::gaia_refine_for_path(path, None, None, None, Some(&cache), &fetch).unwrap();
        let mut summary = json.clone();
        let cuts = summary["report"]["cuts"].clone();
        let matching = ["histogram_peak", "histogram_random_per_bin", "match_radius_arcsec", "chance_radius_arcsec", "density_per_arcsec2", "sigma_res_arcsec", "peak_offset_arcsec"]
            .map(|k| format!("{k}={}", summary["report"][k]));
        summary["report"] = serde_json::Value::Null;
        eprintln!("{path}: {summary:#}\ncuts {cuts}\n{}", matching.join(" "));
        json
    }

    const J1_REFERENCE_SHIFT_MAS: [f64; 2] = [1029.4, 1438.5];
    const J1_REFERENCE_SIGMA_MAS: f64 = 4.5;
    const J1_EXTRA_MAS: f64 = 5.0;
    const J1_MAX_RMS_MAS: f64 = 50.0;
    const J3_SHIFT_AGREEMENT_MAS: f64 = 10.0;

    fn j1_allowance_mas(sigma_mas: f64) -> f64 {
        3.0 * (sigma_mas * sigma_mas + J1_REFERENCE_SIGMA_MAS * J1_REFERENCE_SIGMA_MAS).sqrt() + J1_EXTRA_MAS
    }

    #[test]
    #[ignore]
    fn live_fgs_guider2_dithers_tie_to_gaia_dr3() {
        let mut shifts = Vec::new();
        for name in ["jw01476003004_02201_00001_guider2_cal.fits", "jw01476003004_02201_00002_guider2_cal.fits"] {
            let Some(path) = mast_file(name) else { return };
            let json = live_refine(&path);
            assert_eq!(json["wcs_kind"], "gwcs", "{}", json["wcs_kind"]);
            assert_eq!(json["decision"], "applied", "{}", json["reason"]);
            assert!(json["rms_after_mas"].as_f64().unwrap() <= J1_MAX_RMS_MAS, "{}", json["rms_after_mas"]);
            assert!(json["representation"].as_str().unwrap().starts_with("TAN-SIP refit order"), "{}", json["representation"]);
            let shift = [json["shift_east_mas"].as_f64().unwrap(), json["shift_north_mas"].as_f64().unwrap()];
            let sigma = [json["shift_err_east_mas"].as_f64().unwrap(), json["shift_err_north_mas"].as_f64().unwrap()];
            for axis in 0..2 {
                let off = (shift[axis] - J1_REFERENCE_SHIFT_MAS[axis]).abs();
                let allowance = j1_allowance_mas(sigma[axis]);
                assert!(off <= allowance, "{name} axis {axis}: {:.1} mas from the reference, allowance {allowance:.1} mas", off);
            }
            shifts.push(shift);
        }
        for axis in 0..2 {
            let apart = (shifts[0][axis] - shifts[1][axis]).abs();
            assert!(apart <= J3_SHIFT_AGREEMENT_MAS, "the dithers' shifts differ by {apart:.1} mas on axis {axis}: {shifts:?}");
        }
    }

    #[test]
    #[ignore]
    fn live_dss2_header_wcs_is_tied_with_an_exact_card_update() {
        let dir = std::env::var_os("ASTROBURST_PLATE_SOLVE_DIR").map(std::path::PathBuf::from).unwrap_or_default();
        let path = dir.join("c1").join("fov0.25_mid1.fits");
        if !path.is_file() {
            eprintln!("skipped: {} is absent (set ASTROBURST_PLATE_SOLVE_DIR)", path.display());
            return;
        }
        let path = path.to_string_lossy().into_owned();
        let temp = tempfile::tempdir().unwrap();
        let cache = std::env::var_os(crate::core::astrometry::catalog_disk::ENV_CACHE_DIR)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| temp.path().to_path_buf());
        let fetch = |q: &ConeQuery| crate::core::astrometry::catalog::query_gaia_tie_tsv(q);
        let json = super::gaia_refine_for_path(&path, None, Some(1990.0), Some("shift"), Some(&cache), &fetch).unwrap();
        let mut summary = json.clone();
        summary["report"] = serde_json::Value::Null;
        eprintln!("{path}: {summary:#}");
        assert_eq!(json["wcs_kind"], "header");
        assert_eq!(json["epoch_source"], "typed");
        assert_eq!(json["decision"], "applied", "{}", json["reason"]);
        assert_eq!(json["representation"], "exact header update (CRVAL and CD)");
        let (east, north) = (json["shift_east_mas"].as_f64().unwrap(), json["shift_north_mas"].as_f64().unwrap());
        assert!((east - 667.0).abs() < 50.0 && (north + 797.0).abs() < 50.0, "({east}, {north}) vs the science report's (+667, -797)");
    }

    #[test]
    #[ignore]
    fn live_nircam_nrca1_is_refused_naming_its_count() {
        let Some(path) = mast_file("jw01475001001_02101_00001_nrca1_cal.fits") else { return };
        let json = live_refine(&path);
        assert_eq!(json["decision"], "refused", "{json}");
        assert_eq!(json["reason"], "1 < 10 used pairs for shift");
        assert!(json["wcs_cards"].as_array().unwrap().is_empty());
    }
}
