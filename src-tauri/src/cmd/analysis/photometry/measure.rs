use std::time::Instant;
use serde_json::json;
use rayon::prelude::*;
use crate::cmd::catalog::field_geometry;
use crate::cmd::common::{blocking_cmd, load_cached, load_cached_full};
use crate::types::constants::{RES_ELAPSED_MS, RES_PHOTOMETRY, RES_SKY, RES_GAIA, RES_GAIN_USED, RES_MASKED, RES_WARNINGS, RES_ROWS, RES_INDEX, RES_ERROR, RES_N_MEASURED, RES_N_FAILED};
use crate::core::analysis::forced::{ab_from_jy, applied_noise_factor, limit_rule, upper_limit, FORCED_DEFAULT_LIMIT_SIGMA};
use crate::core::analysis::gain::{gain_model, photon_noise_warning};
use crate::core::analysis::photometry::{StarPhotometry, MAX_APERTURE_RADIUS, MIN_APERTURE_RADIUS};
use crate::core::astrometry::catalog::{query_gaia_cached, CatalogHit, CatalogRow, ConeQuery};
use crate::types::image_ref::PlaneSelector;
use crate::cmd::analysis::{RES_PHOTCAL, UserZeroPoint, check_user_zero_point, gaia_match_field, gaia_match_outcome, gaia_rows_for_match, has_err_extension, photometry_context_with, pixel_units};

pub(in crate::cmd::analysis) const RES_LIMIT: &str = "limit";
const MAX_BATCH_POINTS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LimitRequest {
    pub f_corr: Option<f64>,
    pub limit_sigma: f64,
}

impl Default for LimitRequest {
    fn default() -> Self {
        Self { f_corr: None, limit_sigma: FORCED_DEFAULT_LIMIT_SIGMA }
    }
}

pub(in crate::cmd::analysis) fn limit_request(f_corr: Option<f64>, limit_sigma: Option<f64>) -> anyhow::Result<LimitRequest> {
    let is_positive = |value: f64| value.is_finite() && value > 0.0;
    if let Some(factor) = f_corr.filter(|value| !is_positive(*value)) {
        anyhow::bail!("f_corr {factor} must be a positive finite number");
    }
    if let Some(sigma) = limit_sigma.filter(|value| !is_positive(*value)) {
        anyhow::bail!("limit_sigma {sigma} must be a positive finite number");
    }
    Ok(LimitRequest { f_corr, limit_sigma: limit_sigma.unwrap_or(FORCED_DEFAULT_LIMIT_SIGMA) })
}

pub(in crate::cmd::analysis) fn limit_json(phot: &StarPhotometry, request: &LimitRequest) -> serde_json::Value {
    let noise_factor = applied_noise_factor(request.f_corr, phot.err_used);
    let native = upper_limit(phot.net_flux, Some(phot.flux_err), noise_factor, request.limit_sigma);
    let calibrated = phot.flux_jy.zip(phot.flux_err_jy);
    let jy = calibrated.and_then(|(flux, err)| upper_limit(flux, Some(err), noise_factor, request.limit_sigma));
    json!({
        "limit_sigma": request.limit_sigma,
        "f_corr": noise_factor,
        "f_corr_requested": request.f_corr,
        "is_limit": if calibrated.is_some() { jy.is_some() } else { native.is_some() },
        "upper_limit_native": native,
        "upper_limit_jy": jy,
        "upper_limit_ab": jy.and_then(ab_from_jy),
        "rule": limit_rule(request.limit_sigma, request.f_corr, true, phot.err_used),
    })
}
pub(in crate::cmd::analysis) const PAR_BATCH_POINTS: usize = 64;
pub(in crate::cmd::analysis) const MAX_SKY_ANNULUS_RADIUS: f64 = 512.0;

#[cfg(test)]
pub(crate) fn photometry_for_path(
    path: &str,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    sky_annulus: Option<(f64, f64)>,
    gaia_match: bool,
    exclude_dq: bool,
    gain: Option<f64>,
) -> anyhow::Result<serde_json::Value> {
    photometry_for_path_with_catalog(
        path,
        x,
        y,
        aperture_radius,
        sky_annulus,
        gaia_match,
        exclude_dq,
        gain,
        None,
        &mut |q| query_gaia_cached(q),
        LimitRequest::default(),
    )
}

pub(crate) fn photometry_for_path_with_catalog(
    path: &str,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    sky_annulus: Option<(f64, f64)>,
    gaia_match: bool,
    exclude_dq: bool,
    gain: Option<f64>,
    zero_point: Option<&UserZeroPoint>,
    cached: &mut dyn FnMut(&ConeQuery) -> Result<CatalogHit, String>,
    limit: LimitRequest,
) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    let ctx = photometry_context_with(path, exclude_dq, zero_point)?;
    let config = ctx.config(aperture_radius, sky_annulus, gain);
    let source = ctx.masked_image().map_err(anyhow::Error::msg)?;
    let phot = ctx.measure(&source, x, y, &config).map_err(|e| anyhow::anyhow!(e))?;

    let sky = ctx.sky_json(&phot);
    let mut warnings = ctx.warnings.clone();
    warnings.extend(photon_noise_warning(&ctx.gain_model, config.gain.is_some(), phot.err_used));
    let gaia = match (&ctx.wcs, gaia_match) {
        (Some(wcs), true) => {
            let coord = wcs.pixel_to_world(phot.x, phot.y);
            if coord.ra.is_finite() && coord.dec.is_finite() {
                let (rows_n, cols) = ctx.entry.arr().dim();
                let geometry = field_geometry(wcs, cols, rows_n);
                let field = gaia_match_field(&geometry);
                match gaia_rows_for_match((coord.ra, coord.dec), &field, cached) {
                    Ok(rows) => {
                        let with_colour: Vec<&CatalogRow> = rows.iter().filter(|r| r.bp_rp.is_some()).collect();
                        let (gaia, warning) = gaia_match_outcome(coord.ra, coord.dec, &with_colour);
                        warnings.extend(warning);
                        gaia
                    }
                    Err(reason) => {
                        warnings.push(format!("Gaia query failed: {reason}"));
                        serde_json::Value::Null
                    }
                }
            } else {
                warnings.push("Gaia match skipped: the position has no sky coordinate".into());
                serde_json::Value::Null
            }
        }
        (None, true) => {
            warnings.push("Gaia match skipped: the file has no celestial WCS".into());
            serde_json::Value::Null
        }
        (_, false) => serde_json::Value::Null,
    };
    let (bunit, summed_unit) = pixel_units(ctx.entry.header());

    Ok(json!({
        RES_PHOTOMETRY: serde_json::to_value(&phot)?,
        RES_LIMIT: limit_json(&phot, &limit),
        RES_SKY: sky,
        RES_GAIA: gaia,
        RES_PHOTCAL: ctx.photcal_json()?,
        "bunit": bunit,
        "summed_unit": summed_unit,
        RES_WARNINGS: warnings,
        RES_GAIN_USED: config.gain,
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        RES_MASKED: ctx.mask.is_some(),
    }))
}

pub(in crate::cmd::analysis) fn annulus_arg(inner: Option<f64>, outer: Option<f64>) -> anyhow::Result<Option<(f64, f64)>> {
    let (i, o) = match (inner, outer) {
        (None, None) => return Ok(None),
        (Some(i), Some(o)) => (i, o),
        _ => anyhow::bail!("sky annulus needs both an inner and an outer radius"),
    };
    if !i.is_finite() || i <= 0.0 {
        anyhow::bail!("sky annulus inner radius {i} must be a positive finite number of pixels");
    }
    if !o.is_finite() || o <= 0.0 {
        anyhow::bail!("sky annulus outer radius {o} must be a positive finite number of pixels");
    }
    if o > MAX_SKY_ANNULUS_RADIUS {
        anyhow::bail!("sky annulus outer radius {o} must be at most {MAX_SKY_ANNULUS_RADIUS} pixels");
    }
    if o <= i {
        anyhow::bail!("sky annulus outer radius {o} must be larger than the inner radius {i}");
    }
    Ok(Some((i, o)))
}

pub(in crate::cmd::analysis) fn aperture_radius_arg(aperture_radius: Option<f64>) -> anyhow::Result<Option<f64>> {
    if let Some(r) = aperture_radius {
        if !(MIN_APERTURE_RADIUS..=MAX_APERTURE_RADIUS).contains(&r) {
            anyhow::bail!("aperture radius {r} must be between {MIN_APERTURE_RADIUS} and {MAX_APERTURE_RADIUS} pixels");
        }
    }
    Ok(aperture_radius)
}

pub(in crate::cmd::analysis) fn check_annulus_clears_aperture(aperture_radius: Option<f64>, sky_annulus: Option<(f64, f64)>) -> anyhow::Result<()> {
    if let (Some(r_ap), Some((r_in, _))) = (aperture_radius.filter(|r| r.is_finite() && *r > 0.0), sky_annulus) {
        if r_in <= r_ap {
            anyhow::bail!("sky annulus inner radius {r_in} must be larger than the aperture radius {r_ap}");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn measure_photometry_cmd(
    path: String,
    x: f64,
    y: f64,
    aperture_radius: Option<f64>,
    annulus_inner: Option<f64>,
    annulus_outer: Option<f64>,
    gaia_match: Option<bool>,
    exclude_dq: Option<bool>,
    gain: Option<f64>,
    zero_point: Option<UserZeroPoint>,
    f_corr: Option<f64>,
    limit_sigma: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let limit = limit_request(f_corr, limit_sigma)?;
        if !x.is_finite() || !y.is_finite() {
            anyhow::bail!("photometry position ({x}, {y}) must be finite");
        }
        let aperture_radius = aperture_radius_arg(aperture_radius)?;
        let sky_annulus = annulus_arg(annulus_inner, annulus_outer)?;
        check_annulus_clears_aperture(aperture_radius, sky_annulus)?;
        check_user_zero_point(zero_point.as_ref())?;
        photometry_for_path_with_catalog(
            &path,
            x,
            y,
            aperture_radius,
            sky_annulus,
            gaia_match.unwrap_or(false),
            exclude_dq.unwrap_or(false),
            gain,
            zero_point.as_ref(),
            &mut |q| query_gaia_cached(q),
            limit,
        )
    })
}

#[tauri::command]
pub async fn measure_photometry_batch_cmd(
    path: String,
    points: Vec<(f64, f64)>,
    aperture_radius: Option<f64>,
    annulus_inner: Option<f64>,
    annulus_outer: Option<f64>,
    gain: Option<f64>,
    exclude_dq: Option<bool>,
    with_growth_curve: Option<bool>,
    zero_point: Option<UserZeroPoint>,
    f_corr: Option<f64>,
    limit_sigma: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let limit = limit_request(f_corr, limit_sigma)?;
        if points.is_empty() {
            anyhow::bail!("batch photometry needs at least one point");
        }
        if points.len() > MAX_BATCH_POINTS {
            anyhow::bail!("batch photometry of {} points exceeds the limit of {MAX_BATCH_POINTS}", points.len());
        }
        if let Some((i, (x, y))) = points.iter().enumerate().find(|(_, (x, y))| !x.is_finite() || !y.is_finite()) {
            anyhow::bail!("batch photometry point {i} ({x}, {y}) must be finite");
        }
        let aperture_radius = aperture_radius_arg(aperture_radius)?;
        let sky_annulus = annulus_arg(annulus_inner, annulus_outer)?;
        check_annulus_clears_aperture(aperture_radius, sky_annulus)?;
        check_user_zero_point(zero_point.as_ref())?;
        let with_growth_curve = with_growth_curve.unwrap_or(false);

        let ctx = photometry_context_with(&path, exclude_dq.unwrap_or(false), zero_point.as_ref())?;
        let config = ctx.config(aperture_radius, sky_annulus, gain);
        let source = ctx.masked_image().map_err(anyhow::Error::msg)?;
        let measure = |&(x, y): &(f64, f64)| ctx.measure(&source, x, y, &config);
        let measured: Vec<Result<StarPhotometry, String>> = if points.len() > PAR_BATCH_POINTS {
            points.par_iter().map(measure).collect()
        } else {
            points.iter().map(measure).collect()
        };

        let mut n_measured = 0usize;
        let mut all_err_used = true;
        let mut rows = Vec::with_capacity(measured.len());
        for (index, outcome) in measured.into_iter().enumerate() {
            match outcome {
                Ok(mut phot) => {
                    n_measured += 1;
                    all_err_used &= phot.err_used;
                    if !with_growth_curve {
                        phot.growth_curve.clear();
                    }
                    let sky = ctx.sky_json(&phot);
                    rows.push(json!({
                        RES_INDEX: index,
                        RES_PHOTOMETRY: serde_json::to_value(&phot)?,
                        RES_LIMIT: limit_json(&phot, &limit),
                        RES_SKY: sky,
                        RES_ERROR: serde_json::Value::Null,
                    }));
                }
                Err(e) => rows.push(json!({
                    RES_INDEX: index,
                    RES_PHOTOMETRY: serde_json::Value::Null,
                    RES_LIMIT: serde_json::Value::Null,
                    RES_SKY: serde_json::Value::Null,
                    RES_ERROR: e,
                })),
            }
        }
        let n_failed = rows.len() - n_measured;
        let mut warnings = ctx.warnings.clone();
        warnings.extend(photon_noise_warning(&ctx.gain_model, config.gain.is_some(), all_err_used));
        let (bunit, summed_unit) = pixel_units(ctx.entry.header());

        Ok(json!({
            RES_ROWS: rows,
            RES_PHOTCAL: ctx.photcal_json()?,
            "bunit": bunit,
            "summed_unit": summed_unit,
            RES_WARNINGS: warnings,
            RES_GAIN_USED: config.gain,
            RES_MASKED: ctx.mask.is_some(),
            RES_N_MEASURED: n_measured,
            RES_N_FAILED: n_failed,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn photometry_gain_model_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let entry = load_cached_full(&path).or_else(|_| load_cached(&path))?;
        let dims = entry.arr().dim();
        let has_err = has_err_extension(&path, dims)
            || entry
                .companions()
                .and_then(|c| c.err.as_ref())
                .is_some_and(|r| matches!(r.plane, PlaneSelector::Array(_)));
        Ok(serde_json::to_value(gain_model(entry.header(), has_err))?)
    })
}
