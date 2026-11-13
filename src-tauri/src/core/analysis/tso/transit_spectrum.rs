use std::collections::HashSet;

use anyhow::{bail, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::white_light::{Curve, CurveMode, LightCurves, PIXEL_LOSS_RATIO};
use super::{SeriesKey, TsoTimeScale};
use crate::core::analysis::transit_fit::{fit_transit, ParameterInput, TransitFitRequest};
use crate::core::analysis::transit_model::{self, TransitShape, TRANSIT_MODEL_NAME};
use crate::math::exact_median_f64;

pub const MAX_BASELINE_ORDER: u8 = 2;

const DROPPED_NOTE_PREFIXES: [&str; 2] = ["model: ", "not modelled: "];
const NOT_MODELLED_NOTE: &str = "not modelled: orbital eccentricity, the finite exposure time (the model is evaluated at the mid-integration times), limb darkening from stellar models, systematics beyond the baseline polynomial, correlated noise; no MCMC";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LimbChoice {
    Fixed,
    Free,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpectrumFitRequest {
    pub period_days: f64,
    pub t0: f64,
    pub a_rs: f64,
    pub b: f64,
    pub rp_start: f64,
    pub u1: f64,
    pub u2: f64,
    #[serde(default = "default_limb")]
    pub limb_darkening: LimbChoice,
    #[serde(default = "default_order")]
    pub baseline_order: u8,
    #[serde(default)]
    pub exclude_rows: Vec<usize>,
    #[serde(default)]
    pub scale_errors: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BinFit {
    pub band_um: [f64; 2],
    pub n_points: usize,
    pub n_in_transit: usize,
    pub weighted: bool,
    pub rp: f64,
    pub rp_err: f64,
    pub depth: f64,
    pub depth_err: f64,
    pub u1: f64,
    pub u1_err: f64,
    pub u2: f64,
    pub u2_err: f64,
    pub chi2: f64,
    pub dof: i64,
    pub rms: f64,
    pub grazing: bool,
    pub npixels_ratio: f64,
    pub pixel_loss: bool,
    pub error: Option<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransitSpectrum {
    pub series: SeriesKey,
    pub mode: CurveMode,
    pub time_scale: TsoTimeScale,
    pub fit: SpectrumFitRequest,
    pub model: String,
    pub n_integrations: usize,
    pub n_excluded: usize,
    pub n_in_transit: usize,
    pub bins: Vec<BinFit>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

fn default_limb() -> LimbChoice {
    LimbChoice::Fixed
}

fn default_order() -> u8 {
    1
}

struct Fitted {
    rp: f64,
    rp_err: f64,
    depth: f64,
    depth_err: f64,
    u1: f64,
    u1_err: f64,
    u2: f64,
    u2_err: f64,
    chi2: f64,
    rms: f64,
    notes: Vec<String>,
    warnings: Vec<String>,
}

pub fn sky_separation(time: f64, period_days: f64, t0: f64, a_rs: f64, b: f64) -> (f64, bool) {
    transit_model::sky_separation(time, &TransitShape { t0, period: period_days, rp: 0.0, a_rs, b })
}

pub fn is_grazing(b: f64, rp: f64) -> bool {
    rp.is_finite() && b + rp > 1.0
}

fn is_inside_the_transit(time: f64, fit: &SpectrumFitRequest) -> bool {
    let (separation, near_side) = sky_separation(time, fit.period_days, fit.t0, fit.a_rs, fit.b);
    near_side && separation < 1.0 + fit.rp_start
}

fn excluded_positions(curves: &LightCurves, exclude_rows: &[usize]) -> Vec<bool> {
    let excluded: HashSet<usize> = exclude_rows.iter().copied().collect();
    curves.row_index.iter().map(|row| excluded.contains(row)).collect()
}

pub fn bin_points(curves: &LightCurves, curve: &Curve, exclude_rows: &[usize]) -> Vec<usize> {
    let excluded = excluded_positions(curves, exclude_rows);
    (0..excluded.len()).filter(|&i| !excluded[i] && curve.flux[i].is_finite() && curve.n_used[i] > 0).collect()
}

fn free_parameters(fit: &SpectrumFitRequest) -> usize {
    let limb_darkening = if fit.limb_darkening == LimbChoice::Free { 2 } else { 0 };
    1 + usize::from(fit.baseline_order) + 1 + limb_darkening
}

fn c_transit_fit(time: &[f64], flux: &[f64], err: Option<&[f64]>, fit: &SpectrumFitRequest) -> Result<Fitted> {
    let free_limb = fit.limb_darkening == LimbChoice::Free;
    let request = TransitFitRequest {
        time: time.to_vec(),
        flux: flux.to_vec(),
        err: err.map(|err| err.to_vec()),
        period: fit.period_days,
        t0: ParameterInput { value: fit.t0, free: false },
        rp: ParameterInput { value: fit.rp_start, free: true },
        a_rs: ParameterInput { value: fit.a_rs, free: false },
        b: ParameterInput { value: fit.b, free: false },
        u1: ParameterInput { value: fit.u1, free: free_limb },
        u2: ParameterInput { value: fit.u2, free: free_limb },
        baseline_order: fit.baseline_order,
        scale_errors: fit.scale_errors,
        ..Default::default()
    };
    let found = fit_transit(&request)?;
    let value_and_error = |name: &str| {
        let parameter = found.parameters.iter().find(|parameter| parameter.name == name);
        parameter.map_or((f64::NAN, f64::NAN), |parameter| (parameter.value, parameter.err.unwrap_or(f64::NAN)))
    };
    let (rp, rp_err) = value_and_error("rp");
    let (u1, u1_err) = value_and_error("u1");
    let (u2, u2_err) = value_and_error("u2");
    Ok(Fitted {
        rp,
        rp_err,
        depth: found.depth,
        depth_err: found.depth_err.unwrap_or(f64::NAN),
        u1,
        u1_err,
        u2,
        u2_err,
        chi2: found.chi2,
        rms: found.rms,
        notes: found.notes,
        warnings: found.warnings,
    })
}

fn mean_npixels(curve: &Curve, positions: impl Iterator<Item = usize>) -> f64 {
    let npixels: Vec<f64> = positions.map(|i| curve.npixels_mean[i]).filter(|npixels| npixels.is_finite()).collect();
    npixels.iter().sum::<f64>() / npixels.len() as f64
}

fn npixels_ratio(curve: &Curve, points: &[usize], inside: &[bool]) -> f64 {
    let baseline = mean_npixels(curve, points.iter().copied().filter(|&i| !inside[i]));
    if baseline == 0.0 {
        f64::NAN
    } else {
        mean_npixels(curve, points.iter().copied().filter(|&i| inside[i])) / baseline
    }
}

fn fit_bin(curves: &LightCurves, curve: &Curve, inside: &[bool], fit: &SpectrumFitRequest) -> (BinFit, Vec<String>) {
    let points = bin_points(curves, curve, &fit.exclude_rows);
    let n_points = points.len();
    let n_in_transit = points.iter().filter(|&&i| inside[i]).count();
    let without_error = points.iter().filter(|&&i| !(curve.flux_err[i].is_finite() && curve.flux_err[i] > 0.0)).count();
    let weighted = without_error == 0;
    let free = free_parameters(fit);
    let flux: Vec<f64> = points.iter().map(|&i| curve.flux[i]).collect();
    let median = exact_median_f64(&flux);
    let ratio = npixels_ratio(curve, &points, inside);
    let mut bin = BinFit {
        band_um: curve.band_um,
        n_points,
        n_in_transit,
        weighted,
        rp: f64::NAN,
        rp_err: f64::NAN,
        depth: f64::NAN,
        depth_err: f64::NAN,
        u1: f64::NAN,
        u1_err: f64::NAN,
        u2: f64::NAN,
        u2_err: f64::NAN,
        chi2: f64::NAN,
        dof: n_points as i64 - free as i64,
        rms: f64::NAN,
        grazing: false,
        npixels_ratio: ratio,
        pixel_loss: ratio < PIXEL_LOSS_RATIO,
        error: None,
        notes: Vec::new(),
        warnings: Vec::new(),
    };
    let refusal = if n_points == 0 {
        Some("no integration has a flux in this band".to_string())
    } else if median <= 0.0 {
        Some("the median flux of this bin is not above 0: a transit model cannot be fitted to it".to_string())
    } else if n_points <= free {
        Some(format!("{n_points} integration(s) with a flux: more than {free} are needed for {free} free parameters"))
    } else if n_in_transit == 0 {
        Some("no integration of this bin falls inside the transit".to_string())
    } else {
        None
    };
    if refusal.is_some() {
        bin.error = refusal;
        return (bin, Vec::new());
    }
    let time: Vec<f64> = points.iter().map(|&i| curves.time[i]).collect();
    let normalised: Vec<f64> = flux.iter().map(|flux| flux / median).collect();
    let errors: Option<Vec<f64>> = weighted.then(|| points.iter().map(|&i| curve.flux_err[i] / median).collect());
    let fitted = match c_transit_fit(&time, &normalised, errors.as_deref(), fit) {
        Ok(fitted) if fitted.rp.is_finite() => fitted,
        Ok(_) => {
            bin.error = Some("the fit returned no finite Rp/Rs".to_string());
            return (bin, Vec::new());
        }
        Err(error) => {
            bin.error = Some(format!("{error:#}"));
            return (bin, Vec::new());
        }
    };
    if !weighted {
        bin.notes.push(format!(
            "fitted without weights: {without_error} of {n_points} integrations have no positive flux error, so the errors of this bin are scaled by sqrt(chi2 / dof)"
        ));
    }
    bin.rp = fitted.rp;
    bin.rp_err = fitted.rp_err;
    bin.depth = fitted.depth;
    bin.depth_err = fitted.depth_err;
    bin.u1 = fitted.u1;
    bin.u1_err = fitted.u1_err;
    bin.u2 = fitted.u2;
    bin.u2_err = fitted.u2_err;
    bin.chi2 = fitted.chi2;
    bin.rms = fitted.rms;
    bin.grazing = is_grazing(fit.b, fitted.rp);
    bin.warnings = fitted.warnings;
    let notes_of_the_fit =
        fitted.notes.into_iter().filter(|note| !DROPPED_NOTE_PREFIXES.iter().any(|prefix| note.starts_with(prefix))).collect();
    (bin, notes_of_the_fit)
}

fn check_request(curves: &LightCurves, fit: &SpectrumFitRequest) -> Result<()> {
    if curves.time_scale == TsoTimeScale::Index {
        bail!("a transit fit needs integration times: this series has no usable times and its abscissa is the integration number");
    }
    if curves.bins.is_empty() {
        bail!("add wavelength bins: the fitted spectrum has one point per bin");
    }
    let SpectrumFitRequest { period_days, t0, a_rs, b, rp_start, u1, u2, baseline_order, .. } = *fit;
    if !period_days.is_finite() || period_days <= 0.0 {
        bail!("the period must be a number of days above 0, got {period_days}");
    }
    if !t0.is_finite() {
        bail!("t0 must be a finite time in the scale of the series");
    }
    if !a_rs.is_finite() || a_rs <= 1.0 {
        bail!("a/Rs must be above 1, got {a_rs}");
    }
    if !b.is_finite() || b < 0.0 {
        bail!("the impact parameter b must be 0 or above, got {b}");
    }
    if b >= a_rs {
        bail!("the impact parameter b = {b} must be below a/Rs = {a_rs}");
    }
    if !rp_start.is_finite() || rp_start <= 0.0 {
        bail!("the starting Rp/Rs must be above 0, got {rp_start}");
    }
    if rp_start >= 1.0 + b {
        bail!(
            "the starting Rp/Rs {rp_start} is at or above 1 + b = {}: the planet would cover the whole star at mid-transit, which this model does not fit",
            1.0 + b
        );
    }
    if !u1.is_finite() || !u2.is_finite() {
        bail!("u1 and u2 must be finite numbers");
    }
    if !(0.0..=2.0).contains(&u1) || !(-1.0..=1.0).contains(&u2) {
        bail!("limb darkening out of range: u1 {u1} must be between 0 and 2 and u2 {u2} between -1 and 1");
    }
    if baseline_order > MAX_BASELINE_ORDER {
        bail!("baseline order {baseline_order}: 0, 1 or 2 are fitted");
    }
    Ok(())
}

fn spectrum_notes(time_scale: TsoTimeScale, fit: &SpectrumFitRequest, n_excluded: usize) -> Vec<String> {
    let SpectrumFitRequest { period_days, t0, a_rs, b, u1, u2, baseline_order, .. } = *fit;
    let free_limb = fit.limb_darkening == LimbChoice::Free;
    let fitted = if free_limb { "Rp/Rs, u1, u2" } else { "Rp/Rs" };
    let time_name = if time_scale == TsoTimeScale::MjdUtc { "MJD (UTC)" } else { "BMJD (TDB)" };
    let limb_darkening =
        if free_limb { format!("; u1 and u2 start at {u1} and {u2}") } else { format!(", u1 {u1}, u2 {u2}") };
    let errors =
        if fit.scale_errors { "multiplied in every bin by sqrt(chi2 / dof) of that bin" } else { "from the covariance alone" };
    let mut notes = vec![
        format!("transit model: Mandel and Agol 2002 with quadratic limb darkening on a circular orbit; fitted in every bin: {fitted} and a baseline polynomial of order {baseline_order} in time"),
        format!("fixed in every bin: period {period_days} d, t0 {t0} ({time_name}), a/Rs {a_rs}, b {b}{limb_darkening}"),
        format!("each curve is divided by its median before the fit; the errors are the 1 sigma errors of the least-squares fit, {errors}"),
        NOT_MODELLED_NOTE.to_string(),
    ];
    if n_excluded > 0 {
        notes.push(format!("{n_excluded} integration(s) left out of every bin"));
    }
    notes
}

pub fn spectrum_warnings(time_scale: TsoTimeScale, bins: &[BinFit]) -> Vec<String> {
    let mut warnings = Vec::new();
    if time_scale == TsoTimeScale::MjdUtc {
        warnings.push("the series is on MJD (UTC): t0 and the fitted times are not barycentric".to_string());
    }
    let losing_pixels = bins.iter().filter(|bin| bin.pixel_loss).count();
    if losing_pixels > 0 {
        let with_ratio = bins.iter().filter(|bin| bin.npixels_ratio.is_finite()).count();
        let lowest = bins.iter().filter(|bin| bin.pixel_loss).map(|bin| bin.npixels_ratio).fold(f64::INFINITY, f64::min);
        warnings.push(format!(
            "mean NPIXELS inside the transit is below {PIXEL_LOSS_RATIO} of the baseline in {losing_pixels} of {with_ratio} bins (lowest ratio {lowest:.3}): the extraction lost pixels during the event, so these fitted depths are biased"
        ));
    }
    let fitted = bins.iter().filter(|bin| bin.error.is_none()).count();
    let grazing = bins.iter().filter(|bin| bin.error.is_none() && bin.grazing).count();
    if grazing > 0 {
        warnings.push(format!(
            "grazing transit (b + Rp/Rs above 1) in {grazing} of {fitted} fitted bins: (Rp/Rs)^2 is then not the fraction of light blocked and is not expected to equal the quick-look depth"
        ));
    }
    if fitted < bins.len() {
        warnings.push(format!("{} of {} bins could not be fitted: each says why", bins.len() - fitted, bins.len()));
    }
    warnings
}

pub fn transit_spectrum(curves: &LightCurves, fit: &SpectrumFitRequest) -> Result<TransitSpectrum> {
    check_request(curves, fit)?;
    let excluded = excluded_positions(curves, &fit.exclude_rows);
    let inside: Vec<bool> = curves.time.iter().map(|&time| is_inside_the_transit(time, fit)).collect();
    let n_excluded = excluded.iter().filter(|&&excluded| excluded).count();
    let n_in_transit = (0..inside.len()).filter(|&i| inside[i] && !excluded[i]).count();
    if n_in_transit == 0 {
        bail!(
            "no integration of this series falls inside the transit of this geometry (t0 {}, period {} d, a/Rs {}, b {}, Rp/Rs {}): check t0 and the period",
            fit.t0,
            fit.period_days,
            fit.a_rs,
            fit.b,
            fit.rp_start
        );
    }
    let fits: Vec<(BinFit, Vec<String>)> = curves.bins.par_iter().map(|curve| fit_bin(curves, curve, &inside, fit)).collect();
    let mut notes_of_fitted_bins = fits.iter().filter(|(bin, _)| bin.error.is_none()).map(|(_, notes)| notes);
    let common_notes: Vec<String> = match notes_of_fitted_bins.next() {
        Some(first) => {
            let others: Vec<&Vec<String>> = notes_of_fitted_bins.collect();
            first.iter().filter(|note| others.iter().all(|notes| notes.contains(note))).cloned().collect()
        }
        None => Vec::new(),
    };
    let bins: Vec<BinFit> = fits
        .into_iter()
        .map(|(mut bin, notes)| {
            bin.notes.extend(notes.into_iter().filter(|note| !common_notes.contains(note)));
            bin
        })
        .collect();
    let mut notes = spectrum_notes(curves.time_scale, fit, n_excluded);
    notes.extend(common_notes);
    Ok(TransitSpectrum {
        series: curves.series.clone(),
        mode: curves.mode,
        time_scale: curves.time_scale,
        fit: fit.clone(),
        model: TRANSIT_MODEL_NAME.to_string(),
        n_integrations: curves.time.len(),
        n_excluded,
        n_in_transit,
        warnings: spectrum_warnings(curves.time_scale, &bins),
        bins,
        notes,
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    use serde_json::Value;

    use super::{LimbChoice, SpectrumFitRequest};

    pub const INJECTED_SCALES: [f64; 3] = [3.0, 4.5, 3.0];
    pub const INJECTED_SIGMA: f64 = 0.0003;

    pub fn req() -> SpectrumFitRequest {
        SpectrumFitRequest {
            period_days: 3.0,
            t0: 60000.15,
            a_rs: 8.0,
            b: 0.3,
            rp_start: 0.1,
            u1: 0.3,
            u2: 0.2,
            limb_darkening: LimbChoice::Fixed,
            baseline_order: 1,
            exclude_rows: Vec::new(),
            scale_errors: false,
        }
    }

    pub fn inj() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("tso").join("tso_transit_injected.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("fixture {} unreadable: {error}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("fixture {} is not JSON: {error}", path.display()))
    }

    pub fn numbers(value: &Value) -> Vec<f64> {
        value.as_array().unwrap().iter().map(|number| number.as_f64().unwrap()).collect()
    }

    pub fn injected_flux(fixture: &Value, bin: usize, noisy: bool) -> Vec<f64> {
        let time = numbers(&fixture["time"]);
        let transit = numbers(&fixture["transit"][bin]);
        let noise = numbers(&fixture["noise"][bin]);
        (0..time.len())
            .map(|row| {
                let base = 1.0 + 0.02 * (time[row] - 60000.15);
                INJECTED_SCALES[bin] * (base * transit[row] + if noisy { noise[row] } else { 0.0 })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use serde_json::{json, Value};

    use super::test_support::{inj, injected_flux, numbers, req, INJECTED_SCALES, INJECTED_SIGMA};
    use super::*;
    use crate::core::analysis::tso::white_light::{Dropped, LimitsOrigin};

    const INJECTED_RP: [f64; 3] = [0.095, 0.1, 0.108];
    const NOTE_MODEL_ORDER_1: &str = "transit model: Mandel and Agol 2002 with quadratic limb darkening on a circular orbit; fitted in every bin: Rp/Rs and a baseline polynomial of order 1 in time";
    const NOTE_FIXED: &str = "fixed in every bin: period 3 d, t0 60000.15 (BMJD (TDB)), a/Rs 8, b 0.3, u1 0.3, u2 0.2";
    const NOTE_COVARIANCE_ALONE: &str = "each curve is divided by its median before the fit; the errors are the 1 sigma errors of the least-squares fit, from the covariance alone";
    const NOTE_NOT_MODELLED: &str = "not modelled: orbital eccentricity, the finite exposure time (the model is evaluated at the mid-integration times), limb darkening from stellar models, systematics beyond the baseline polynomial, correlated noise; no MCMC";
    const WARNING_UTC: &str = "the series is on MJD (UTC): t0 and the fitted times are not barycentric";

    fn curve_of(flux: Vec<f64>, err: f64, npixels: f64, band: [f64; 2]) -> Curve {
        let rows = flux.len();
        Curve {
            band_um: band,
            limits_origin: LimitsOrigin::User,
            flux,
            flux_err: vec![err; rows],
            n_used: vec![2; rows],
            npixels_mean: vec![npixels; rows],
        }
    }

    fn curves_of(time: &[f64], bins: Vec<Curve>) -> LightCurves {
        let rows = time.len();
        let white_flux = (0..rows).map(|row| bins.iter().map(|bin| bin.flux[row]).sum()).collect();
        LightCurves {
            series: SeriesKey { detector: "NRS1".into(), sporder: Some(0) },
            mode: CurveMode::Pipeline,
            time_scale: TsoTimeScale::BmjdTdb,
            time: time.to_vec(),
            mjd_utc: time.iter().map(|time| time - 0.002).collect(),
            int_num: (1..=rows as i64).collect(),
            row_index: (0..rows).collect(),
            flux_unit: "Jy".into(),
            white: curve_of(white_flux, INJECTED_SIGMA, 7.0, [1.0, 4.0]),
            bins,
            dropped: Dropped { nan_time: 0, repeated_time: 0 },
            rejected_elements: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn inj_curves(noisy: bool) -> LightCurves {
        let fixture = inj();
        let bins = (0..3)
            .map(|k| {
                let band = [k as f64 + 1.0, k as f64 + 2.0];
                curve_of(injected_flux(&fixture, k, noisy), INJECTED_SCALES[k] * INJECTED_SIGMA, 7.0, band)
            })
            .collect();
        curves_of(&numbers(&fixture["time"]), bins)
    }

    fn scaled() -> SpectrumFitRequest {
        SpectrumFitRequest { scale_errors: true, ..req() }
    }

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs()
    }

    fn largest_relative_difference(found: impl Iterator<Item = f64>, expected: &[f64]) -> f64 {
        found.zip(expected).map(|(found, expected)| ((found - expected) / expected).abs()).fold(0.0, f64::max)
    }

    fn oracle(fixture: &Value, fit: &str, field: &str) -> [f64; 3] {
        [0, 1, 2].map(|k| fixture[fit][k][field].as_f64().unwrap())
    }

    fn refusal(curves: &LightCurves, fit: &SpectrumFitRequest) -> String {
        transit_spectrum(curves, fit).unwrap_err().to_string()
    }

    fn sorted_keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn the_sky_separation_follows_a_circular_orbit_and_knows_the_near_side() {
        let (at_mid_transit, near) = sky_separation(60000.15, 3.0, 60000.15, 8.0, 0.3);
        assert!((at_mid_transit - 0.3).abs() <= 1e-12, "{at_mid_transit}");
        assert!(near);

        let phase = 2.0 * PI * 0.05 / 3.0;
        let on_the_way_out = 8.0 * (phase.sin().powi(2) + (0.3f64 / 8.0).powi(2) * phase.cos().powi(2)).sqrt();
        let (separation, near) = sky_separation(60000.2, 3.0, 60000.15, 8.0, 0.3);
        assert!((separation - on_the_way_out).abs() <= 1e-9, "{separation} against {on_the_way_out}");
        assert!((separation - 0.88785889563).abs() <= 1e-9, "{separation}");
        assert!(near);

        let (a_quarter_later, _) = sky_separation(60000.9, 3.0, 60000.15, 8.0, 0.3);
        assert!((a_quarter_later - 8.0).abs() <= 1e-9, "{a_quarter_later}");

        let (behind_the_star, near) = sky_separation(60001.65, 3.0, 60000.15, 8.0, 0.3);
        assert!((behind_the_star - 0.3).abs() <= 1e-9, "{behind_the_star}");
        assert!(!near);

        assert!(!is_grazing(0.3, 0.1));
        assert!(is_grazing(0.95, 0.1));
        assert!(is_grazing(7.16, 7.7));
        assert!(!is_grazing(0.0, 1.0));
        assert!(!is_grazing(0.3, f64::NAN));
    }

    #[test]
    fn the_request_reads_with_its_defaults_and_serialises_with_the_names_the_frontend_reads() {
        let typed = json!({ "period_days": 3.0, "t0": 60000.15, "a_rs": 8.0, "b": 0.3, "rp_start": 0.1, "u1": 0.3, "u2": 0.2 });
        assert_eq!(serde_json::from_value::<SpectrumFitRequest>(typed.clone()).unwrap(), req());

        let mut complete = typed.clone();
        complete["limb_darkening"] = json!("free");
        complete["baseline_order"] = json!(2);
        complete["exclude_rows"] = json!([4, 2]);
        complete["scale_errors"] = json!(true);
        let read: SpectrumFitRequest = serde_json::from_value(complete.clone()).unwrap();
        assert_eq!(
            read,
            SpectrumFitRequest {
                limb_darkening: LimbChoice::Free,
                baseline_order: 2,
                exclude_rows: vec![4, 2],
                scale_errors: true,
                ..req()
            }
        );

        let mut unknown_choice = complete;
        unknown_choice["limb_darkening"] = json!("auto");
        assert!(serde_json::from_value::<SpectrumFitRequest>(unknown_choice).is_err());
        let mut without_t0 = typed;
        without_t0.as_object_mut().unwrap().remove("t0");
        assert!(serde_json::from_value::<SpectrumFitRequest>(without_t0).is_err());

        let written = serde_json::to_value(req()).unwrap();
        assert_eq!(
            sorted_keys(&written),
            ["a_rs", "b", "baseline_order", "exclude_rows", "limb_darkening", "period_days", "rp_start", "scale_errors", "t0", "u1", "u2"]
        );
        assert_eq!(written["limb_darkening"], "fixed");
        assert_eq!(written["scale_errors"], false);
        assert_eq!(written["baseline_order"], 1);
        assert_eq!(written["exclude_rows"], json!([]));
        assert_eq!(serde_json::to_value(LimbChoice::Free).unwrap(), "free");
    }

    #[test]
    fn the_points_of_a_bin_leave_out_missing_flux_empty_sums_and_excluded_rows() {
        let time = [60000.0, 60000.1, 60000.2, 60000.3, 60000.4];
        let mut curve = curve_of(vec![1.0, f64::NAN, 0.0, 1.0, 1.0], 0.1, 7.0, [1.0, 2.0]);
        curve.n_used = vec![2, 2, 0, 2, 2];
        let mut curves = curves_of(&time, vec![curve]);
        curves.row_index = vec![0, 2, 3, 4, 6];
        assert_eq!(bin_points(&curves, &curves.bins[0], &[]), [0, 3, 4]);
        assert_eq!(bin_points(&curves, &curves.bins[0], &[4, 5]), [0, 4]);
    }

    #[test]
    fn an_injected_spectrum_is_recovered_bin_by_bin() {
        let spectrum = transit_spectrum(&inj_curves(false), &req()).unwrap();
        assert_eq!(spectrum.bins.len(), 3);
        for (k, bin) in spectrum.bins.iter().enumerate() {
            assert_eq!(bin.band_um, [k as f64 + 1.0, k as f64 + 2.0]);
            assert!(close(bin.depth, INJECTED_RP[k] * INJECTED_RP[k], 1e-6), "depth of bin {k}: {}", bin.depth);
            assert!(close(bin.rp, INJECTED_RP[k], 1e-6), "Rp/Rs of bin {k}: {}", bin.rp);
            assert!((bin.depth - bin.rp * bin.rp).abs() <= 1e-9 * bin.depth, "bin {k}: depth {} and rp {}", bin.depth, bin.rp);
            assert!(bin.chi2 < 1e-6, "chi2 of bin {k}: {}", bin.chi2);
            assert_eq!((bin.n_points, bin.n_in_transit, bin.dof), (120, 51, 117), "bin {k}");
            assert!(bin.weighted, "bin {k}");
            assert_eq!(bin.error, None, "bin {k}");
            assert!(!bin.grazing, "bin {k}");
            assert_eq!((bin.u1, bin.u2), (0.3, 0.2), "bin {k}");
            assert!(bin.u1_err.is_nan() && bin.u2_err.is_nan(), "bin {k}: {} and {}", bin.u1_err, bin.u2_err);
            assert!(bin.rp_err.is_finite() && bin.depth_err.is_finite() && bin.rms.is_finite(), "bin {k}");
            assert_eq!(bin.npixels_ratio, 1.0, "bin {k}");
            assert!(!bin.pixel_loss, "bin {k}");
            assert!(!bin.warnings.iter().any(|warning| warning.starts_with("the fit did not converge")), "bin {k}: {:?}", bin.warnings);
            assert!(
                !bin.notes.iter().any(|note| note.starts_with("model: ") || note.starts_with("not modelled: ")),
                "bin {k}: {:?}",
                bin.notes
            );
        }
        println!(
            "noise-free injection: largest relative difference of Rp/Rs {:.1e}, largest chi2 {:.1e}",
            largest_relative_difference(spectrum.bins.iter().map(|bin| bin.rp), &INJECTED_RP),
            spectrum.bins.iter().map(|bin| bin.chi2).fold(0.0, f64::max)
        );
        assert_eq!((spectrum.n_integrations, spectrum.n_excluded, spectrum.n_in_transit), (120, 0, 51));
        assert_eq!(spectrum.model, TRANSIT_MODEL_NAME);
        assert_eq!(spectrum.fit, req());
        assert_eq!(spectrum.series, SeriesKey { detector: "NRS1".into(), sporder: Some(0) });
        assert_eq!(spectrum.mode, CurveMode::Pipeline);
        assert_eq!(spectrum.time_scale, TsoTimeScale::BmjdTdb);
        assert_eq!(spectrum.warnings, Vec::<String>::new());
        assert_eq!(spectrum.notes[..4], [NOTE_MODEL_ORDER_1, NOTE_FIXED, NOTE_COVARIANCE_ALONE, NOTE_NOT_MODELLED]);
        assert_eq!(spectrum.notes.iter().filter(|note| note.starts_with("not modelled: ")).count(), 1, "{:?}", spectrum.notes);
        assert!(!spectrum.notes.iter().any(|note| note.starts_with("model: ")), "{:?}", spectrum.notes);
        assert!(spectrum.notes.contains(&"period 3 d given, not fitted".to_string()), "{:?}", spectrum.notes);
        for bin in &spectrum.bins {
            assert!(!bin.notes.iter().any(|note| spectrum.notes.contains(note)), "{:?}", bin.notes);
        }
    }

    #[test]
    fn noisy_bins_give_the_depths_the_chi_square_and_both_error_rules_of_the_oracle() {
        let fixture = inj();
        let depth = oracle(&fixture, "fit_noisy", "depth");
        let chi2 = oracle(&fixture, "fit_noisy", "chi2");
        let covariance_alone = oracle(&fixture, "fit_noisy", "depth_err_formal");
        let by_the_scatter = oracle(&fixture, "fit_noisy", "depth_err_scaled");
        let rms = oracle(&fixture, "fit_noisy", "rms");
        assert_eq!(depth, [0.009052241150134217, 0.01003046213264659, 0.011645607331166725]);
        assert_eq!(chi2, [117.11726370635283, 114.50550707801735, 101.73796068161136]);
        assert_eq!(covariance_alone, [5.472658070200652e-05, 5.477493060701735e-05, 5.484166518841573e-05]);
        assert_eq!(by_the_scatter, [5.475399879823703e-05, 5.4187871508730384e-05, 5.1139822485623655e-05]);

        let curves = inj_curves(true);
        let unscaled = transit_spectrum(&curves, &req()).unwrap();
        let rescaled = transit_spectrum(&curves, &scaled()).unwrap();
        println!(
            "noisy injection against the oracle, largest relative differences: depth {:.1e}, chi2 {:.1e}, depth error from the covariance alone {:.1e}, depth error scaled by sqrt(chi2 / dof) {:.1e}",
            largest_relative_difference(unscaled.bins.iter().map(|bin| bin.depth), &depth),
            largest_relative_difference(unscaled.bins.iter().map(|bin| bin.chi2), &chi2),
            largest_relative_difference(unscaled.bins.iter().map(|bin| bin.depth_err), &covariance_alone),
            largest_relative_difference(rescaled.bins.iter().map(|bin| bin.depth_err), &by_the_scatter)
        );
        for k in 0..3 {
            let (plain, wide) = (&unscaled.bins[k], &rescaled.bins[k]);
            assert_eq!(plain.error, None, "bin {k}");
            assert!(close(plain.depth, depth[k], 1e-5), "depth of bin {k}: {} against {}", plain.depth, depth[k]);
            assert!(close(plain.chi2, chi2[k], 1e-6), "chi2 of bin {k}: {} against {}", plain.chi2, chi2[k]);
            assert_eq!(plain.dof, 117, "bin {k}");
            assert!(close(plain.rms, rms[k], 1e-6), "rms of bin {k} in units of its median flux: {} against {}", plain.rms, rms[k]);
            assert!(
                close(plain.depth_err, covariance_alone[k], 0.02),
                "depth error of bin {k} from the covariance alone: {} against {}",
                plain.depth_err,
                covariance_alone[k]
            );
            assert_eq!((wide.depth, wide.rp, wide.chi2), (plain.depth, plain.rp, plain.chi2), "bin {k}");
            assert!(
                close(wide.depth_err, by_the_scatter[k], 0.02),
                "depth error of bin {k} scaled by sqrt(chi2 / dof): {} against {}",
                wide.depth_err,
                by_the_scatter[k]
            );
            for bin in [plain, wide] {
                assert!(
                    (bin.depth_err - 2.0 * bin.rp * bin.rp_err).abs() <= 1e-6 * bin.depth_err,
                    "bin {k}: depth error {} and rp error {}",
                    bin.depth_err,
                    bin.rp_err
                );
            }
            let injected = INJECTED_RP[k] * INJECTED_RP[k];
            assert!((plain.depth - injected).abs() <= 3.0 * plain.depth_err, "bin {k}: {} against {injected}", plain.depth);
        }
        assert_eq!(unscaled.notes[2], NOTE_COVARIANCE_ALONE);
        assert_eq!(
            rescaled.notes[2],
            "each curve is divided by its median before the fit; the errors are the 1 sigma errors of the least-squares fit, multiplied in every bin by sqrt(chi2 / dof) of that bin"
        );
        assert_eq!(rescaled.fit, scaled());
    }

    #[test]
    fn the_baseline_order_the_limb_darkening_choice_and_the_rows_left_out_follow_the_request() {
        let fixture = inj();
        let curves = inj_curves(true);

        let constant = oracle(&fixture, "fit_noisy_order0", "depth");
        let constant_chi2 = [4196.78679981284, 4019.052448585432, 4199.466071005137];
        let covariance_alone = oracle(&fixture, "fit_noisy_order0", "depth_err_formal");
        let by_the_scatter = oracle(&fixture, "fit_noisy_order0", "depth_err_scaled");
        assert_eq!(constant, [0.009012434906915097, 0.009991405111919812, 0.011605534806915938]);
        assert_eq!((covariance_alone[0], by_the_scatter[0]), (5.472285725647477e-05, 0.0003263519523901953));
        let unscaled = transit_spectrum(&curves, &SpectrumFitRequest { baseline_order: 0, ..req() }).unwrap();
        let rescaled = transit_spectrum(&curves, &SpectrumFitRequest { baseline_order: 0, ..scaled() }).unwrap();
        println!(
            "constant baseline against the oracle, largest relative differences: depth {:.1e}, chi2 {:.1e}, depth error from the covariance alone {:.1e}, depth error scaled by sqrt(chi2 / dof) {:.1e}",
            largest_relative_difference(unscaled.bins.iter().map(|bin| bin.depth), &constant),
            largest_relative_difference(unscaled.bins.iter().map(|bin| bin.chi2), &constant_chi2),
            largest_relative_difference(unscaled.bins.iter().map(|bin| bin.depth_err), &covariance_alone),
            largest_relative_difference(rescaled.bins.iter().map(|bin| bin.depth_err), &by_the_scatter)
        );
        for k in 0..3 {
            let (plain, wide) = (&unscaled.bins[k], &rescaled.bins[k]);
            assert_eq!((plain.dof, wide.dof), (118, 118), "constant baseline, bin {k}");
            assert!(close(plain.depth, constant[k], 1e-5), "constant baseline, depth of bin {k}: {}", plain.depth);
            assert!(close(plain.chi2, constant_chi2[k], 1e-6), "constant baseline, chi2 of bin {k}: {}", plain.chi2);
            assert!(
                close(plain.depth_err, covariance_alone[k], 0.02),
                "constant baseline, depth error of bin {k} from the covariance alone: {}",
                plain.depth_err
            );
            assert!(
                close(wide.depth_err, by_the_scatter[k], 0.02),
                "constant baseline, depth error of bin {k} scaled by sqrt(chi2 / dof): {}",
                wide.depth_err
            );
        }
        assert!(unscaled.notes[0].ends_with("fitted in every bin: Rp/Rs and a baseline polynomial of order 0 in time"), "{}", unscaled.notes[0]);

        let quadratic = [0.009019492037636736, 0.010092059075010268, 0.011590352346666246];
        let quadratic_chi2 = [116.86543119032805, 113.62004104088959, 101.0303134440226];
        assert_eq!(oracle(&fixture, "fit_noisy_order2", "depth"), quadratic);
        let curved = transit_spectrum(&curves, &SpectrumFitRequest { baseline_order: 2, ..req() }).unwrap();
        println!(
            "quadratic baseline against the oracle, largest relative differences: depth {:.1e}, chi2 {:.1e}",
            largest_relative_difference(curved.bins.iter().map(|bin| bin.depth), &quadratic),
            largest_relative_difference(curved.bins.iter().map(|bin| bin.chi2), &quadratic_chi2)
        );
        for (k, bin) in curved.bins.iter().enumerate() {
            assert_eq!(bin.dof, 116, "quadratic baseline, bin {k}");
            assert!(close(bin.depth, quadratic[k], 1e-5), "quadratic baseline, depth of bin {k}: {}", bin.depth);
            assert!(close(bin.chi2, quadratic_chi2[k], 1e-6), "quadratic baseline, chi2 of bin {k}: {}", bin.chi2);
        }

        let without_rows = [0.009049733559039852, 0.010012680402311017, 0.011642423445337922];
        let without_rows_chi2 = [108.61448736605051, 108.61483972434198, 101.11248687638253];
        assert_eq!(oracle(&fixture, "fit_noisy_excluded", "depth"), without_rows);
        assert_eq!(fixture["excluded_rows"], json!([0, 1, 60, 61, 119]));
        let shortened = transit_spectrum(&curves, &SpectrumFitRequest { exclude_rows: vec![0, 1, 60, 61, 119], ..req() }).unwrap();
        println!(
            "five rows left out against the oracle, largest relative differences: depth {:.1e}, chi2 {:.1e}",
            largest_relative_difference(shortened.bins.iter().map(|bin| bin.depth), &without_rows),
            largest_relative_difference(shortened.bins.iter().map(|bin| bin.chi2), &without_rows_chi2)
        );
        assert_eq!((shortened.n_integrations, shortened.n_excluded, shortened.n_in_transit), (120, 5, 49));
        for (k, bin) in shortened.bins.iter().enumerate() {
            assert_eq!((bin.n_points, bin.n_in_transit, bin.dof), (115, 49, 112), "rows left out, bin {k}");
            assert!(close(bin.depth, without_rows[k], 1e-5), "rows left out, depth of bin {k}: {}", bin.depth);
            assert!(close(bin.chi2, without_rows_chi2[k], 1e-6), "rows left out, chi2 of bin {k}: {}", bin.chi2);
        }
        assert!(shortened.notes.contains(&"5 integration(s) left out of every bin".to_string()), "{:?}", shortened.notes);
        assert_eq!(shortened.notes[4], "5 integration(s) left out of every bin");

        let free_limb = oracle(&fixture, "fit_noisy_free_limb", "depth");
        let free_limb_chi2 = [108.67331620833015, 113.46715516493434, 101.17675230210457];
        let (u1, u2) = (oracle(&fixture, "fit_noisy_free_limb", "u1"), oracle(&fixture, "fit_noisy_free_limb", "u2"));
        assert_eq!(free_limb, [0.008958165250067457, 0.0100640540599928, 0.011635027079376359]);
        assert_eq!((u1[0], u2[0]), (0.377061466065073, 0.17335320124569065));
        let darkened = transit_spectrum(&curves, &SpectrumFitRequest { limb_darkening: LimbChoice::Free, ..req() }).unwrap();
        println!(
            "free limb darkening against the oracle, largest relative differences: depth {:.1e}, chi2 {:.1e}, u1 {:.1e}, u2 {:.1e}",
            largest_relative_difference(darkened.bins.iter().map(|bin| bin.depth), &free_limb),
            largest_relative_difference(darkened.bins.iter().map(|bin| bin.chi2), &free_limb_chi2),
            largest_relative_difference(darkened.bins.iter().map(|bin| bin.u1), &u1),
            largest_relative_difference(darkened.bins.iter().map(|bin| bin.u2), &u2)
        );
        for (k, bin) in darkened.bins.iter().enumerate() {
            assert_eq!(bin.error, None, "free limb darkening, bin {k}");
            assert_eq!(bin.dof, 115, "free limb darkening, bin {k}");
            assert!(close(bin.depth, free_limb[k], 1e-4), "free limb darkening, depth of bin {k}: {}", bin.depth);
            assert!(close(bin.chi2, free_limb_chi2[k], 1e-6), "free limb darkening, chi2 of bin {k}: {}", bin.chi2);
            assert!((bin.u1 - u1[k]).abs() <= 1e-3, "free limb darkening, u1 of bin {k}: {} against {}", bin.u1, u1[k]);
            assert!((bin.u2 - u2[k]).abs() <= 1e-3, "free limb darkening, u2 of bin {k}: {} against {}", bin.u2, u2[k]);
            assert!(bin.u1_err.is_finite() && bin.u2_err.is_finite(), "free limb darkening, bin {k}: {} and {}", bin.u1_err, bin.u2_err);
        }
        assert!(
            darkened.notes[0].ends_with("fitted in every bin: Rp/Rs, u1, u2 and a baseline polynomial of order 1 in time"),
            "{}",
            darkened.notes[0]
        );
        assert!(darkened.notes[1].ends_with("b 0.3; u1 and u2 start at 0.3 and 0.2"), "{}", darkened.notes[1]);
    }

    #[test]
    fn bins_that_cannot_be_fitted_say_why_and_the_others_are_fitted() {
        let mut curves = inj_curves(false);
        let rows = curves.time.len();
        curves.bins[0].flux = vec![f64::NAN; rows];
        curves.bins[0].n_used = vec![0; rows];
        for flux in &mut curves.bins[1].flux {
            *flux *= -1.0;
        }
        let mut three_points = curves.bins[2].clone();
        for flux in &mut three_points.flux[3..] {
            *flux = f64::NAN;
        }
        curves.bins.push(three_points);

        let spectrum = transit_spectrum(&curves, &req()).unwrap();
        assert_eq!(spectrum.bins.len(), 4);
        assert_eq!(spectrum.bins[0].error.as_deref(), Some("no integration has a flux in this band"));
        assert_eq!((spectrum.bins[0].n_points, spectrum.bins[0].dof), (0, -3));
        assert_eq!(
            spectrum.bins[1].error.as_deref(),
            Some("the median flux of this bin is not above 0: a transit model cannot be fitted to it")
        );
        assert_eq!((spectrum.bins[1].n_points, spectrum.bins[1].dof), (120, 117));
        assert_eq!(
            spectrum.bins[3].error.as_deref(),
            Some("3 integration(s) with a flux: more than 3 are needed for 3 free parameters")
        );
        assert_eq!((spectrum.bins[3].n_points, spectrum.bins[3].dof), (3, 0));
        let fitted = &spectrum.bins[2];
        assert_eq!(fitted.error, None);
        assert!(close(fitted.depth, 0.108 * 0.108, 1e-6), "{}", fitted.depth);
        for k in [0, 1, 3] {
            let bin = &spectrum.bins[k];
            assert!(bin.rp.is_nan() && bin.depth.is_nan() && bin.u1.is_nan() && bin.chi2.is_nan(), "bin {k}: {bin:?}");
            assert!(bin.rp_err.is_nan() && bin.depth_err.is_nan() && bin.u2.is_nan() && bin.rms.is_nan(), "bin {k}: {bin:?}");
            assert!(!bin.grazing, "bin {k}");
            assert!(bin.warnings.is_empty() && bin.notes.is_empty(), "bin {k}: {bin:?}");
        }
        assert_eq!(spectrum.warnings.last().map(String::as_str), Some("3 of 4 bins could not be fitted: each says why"));
        assert_eq!(spectrum.n_in_transit, 51);

        let written = serde_json::to_value(&spectrum).unwrap();
        assert!(written["bins"][0]["depth"].is_null(), "{}", written["bins"][0]);
        assert!(written["bins"][0]["error"].is_string(), "{}", written["bins"][0]);
        assert!(written["bins"][2]["error"].is_null(), "{}", written["bins"][2]);
        assert!(written["bins"][2]["depth"].is_number(), "{}", written["bins"][2]);

        let mut one_bin_out_of_transit = inj_curves(false);
        for row in 35..=85 {
            one_bin_out_of_transit.bins[1].flux[row] = f64::NAN;
        }
        let partly = transit_spectrum(&one_bin_out_of_transit, &req()).unwrap();
        assert_eq!(partly.bins[1].error.as_deref(), Some("no integration of this bin falls inside the transit"));
        assert_eq!((partly.bins[1].n_points, partly.bins[1].n_in_transit), (69, 0));
        assert_eq!(partly.bins[0].error, None);

        let larger_than_the_model_allows = SpectrumFitRequest { a_rs: 200.0, b: 150.0, rp_start: 150.5, ..req() };
        let refused_by_the_fit = transit_spectrum(&inj_curves(false), &larger_than_the_model_allows).unwrap();
        for (k, bin) in refused_by_the_fit.bins.iter().enumerate() {
            assert_eq!(
                bin.error.as_deref(),
                Some("rp 150.5 must be above 0 and at most 100 (planet radius over star radius)"),
                "bin {k}"
            );
            assert!(bin.rp.is_nan() && bin.notes.is_empty() && bin.warnings.is_empty(), "bin {k}: {bin:?}");
        }
        assert_eq!(refused_by_the_fit.warnings, ["3 of 3 bins could not be fitted: each says why"]);
        assert_eq!(refused_by_the_fit.notes.len(), 4, "{:?}", refused_by_the_fit.notes);

        let mut unweighted = inj_curves(false);
        unweighted.bins[0].flux_err[10] = 0.0;
        let spectrum = transit_spectrum(&unweighted, &req()).unwrap();
        let bin = &spectrum.bins[0];
        assert_eq!(bin.error, None);
        assert!(!bin.weighted);
        assert_eq!(
            bin.notes[0],
            "fitted without weights: 1 of 120 integrations have no positive flux error, so the errors of this bin are scaled by sqrt(chi2 / dof)"
        );
        assert!(close(bin.depth, 0.095 * 0.095, 1e-6), "{}", bin.depth);
        assert!(spectrum.bins[1].weighted && spectrum.bins[2].weighted);
        assert!(spectrum.bins[1].notes.contains(&"weights 1 / err^2".to_string()), "{:?}", spectrum.bins[1].notes);
        assert!(bin.notes.contains(&"equal weights: the points carry no errors".to_string()), "{:?}", bin.notes);
        assert!(!spectrum.notes.iter().any(|note| note.starts_with("weights") || note.starts_with("equal weights")), "{:?}", spectrum.notes);
    }

    fn hand_built(grazing: bool, error: Option<&str>) -> BinFit {
        BinFit {
            band_um: [1.0, 2.0],
            n_points: 120,
            n_in_transit: 51,
            weighted: true,
            rp: 0.1,
            rp_err: 0.001,
            depth: 0.01,
            depth_err: 0.0002,
            u1: 0.3,
            u1_err: f64::NAN,
            u2: 0.2,
            u2_err: f64::NAN,
            chi2: 117.0,
            dof: 117,
            rms: 0.0003,
            grazing,
            npixels_ratio: 1.0,
            pixel_loss: false,
            error: error.map(str::to_string),
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn pixel_loss_grazing_and_the_utc_scale_reach_the_warnings() {
        let mut curves = inj_curves(false);
        for row in 35..=85 {
            curves.bins[0].npixels_mean[row] = 3.0;
        }
        let spectrum = transit_spectrum(&curves, &req()).unwrap();
        assert!((spectrum.bins[0].npixels_ratio - 3.0 / 7.0).abs() <= 1e-12, "{}", spectrum.bins[0].npixels_ratio);
        assert!(spectrum.bins[0].pixel_loss);
        assert!(!spectrum.bins[1].pixel_loss && !spectrum.bins[2].pixel_loss);
        assert_eq!(
            spectrum.warnings,
            ["mean NPIXELS inside the transit is below 0.9 of the baseline in 1 of 3 bins (lowest ratio 0.429): the extraction lost pixels during the event, so these fitted depths are biased"]
        );

        curves.bins[1].npixels_mean = vec![f64::NAN; curves.time.len()];
        let spectrum = transit_spectrum(&curves, &req()).unwrap();
        assert!(spectrum.bins[1].npixels_ratio.is_nan());
        assert!(!spectrum.bins[1].pixel_loss);
        assert_eq!(spectrum.bins[1].error, None);
        assert_eq!(
            spectrum.warnings,
            ["mean NPIXELS inside the transit is below 0.9 of the baseline in 1 of 2 bins (lowest ratio 0.429): the extraction lost pixels during the event, so these fitted depths are biased"]
        );
        assert!(serde_json::to_value(&spectrum).unwrap()["bins"][1]["npixels_ratio"].is_null());

        for row in (0..35).chain(86..120) {
            curves.bins[2].npixels_mean[row] = 0.0;
        }
        let spectrum = transit_spectrum(&curves, &req()).unwrap();
        assert!(spectrum.bins[2].npixels_ratio.is_nan(), "{}", spectrum.bins[2].npixels_ratio);
        assert!(!spectrum.bins[2].pixel_loss);
        assert!(spectrum.warnings[0].contains("in 1 of 1 bins (lowest ratio 0.429)"), "{:?}", spectrum.warnings);

        let bins = [hand_built(true, None), hand_built(false, None), hand_built(false, Some("x"))];
        assert_eq!(
            spectrum_warnings(TsoTimeScale::MjdUtc, &bins),
            [
                WARNING_UTC,
                "grazing transit (b + Rp/Rs above 1) in 1 of 2 fitted bins: (Rp/Rs)^2 is then not the fraction of light blocked and is not expected to equal the quick-look depth",
                "1 of 3 bins could not be fitted: each says why",
            ]
        );
        assert_eq!(spectrum_warnings(TsoTimeScale::BmjdTdb, &bins[1..2]), Vec::<String>::new());

        let mut on_utc = inj_curves(false);
        on_utc.time_scale = TsoTimeScale::MjdUtc;
        let spectrum = transit_spectrum(&on_utc, &req()).unwrap();
        assert_eq!(spectrum.warnings, [WARNING_UTC]);
        assert_eq!(spectrum.notes[1], "fixed in every bin: period 3 d, t0 60000.15 (MJD (UTC)), a/Rs 8, b 0.3, u1 0.3, u2 0.2");
    }

    #[test]
    fn a_series_without_times_without_bins_or_out_of_transit_is_refused() {
        let curves = inj_curves(false);
        let mut untimed = curves.clone();
        untimed.time_scale = TsoTimeScale::Index;
        assert_eq!(
            refusal(&untimed, &req()),
            "a transit fit needs integration times: this series has no usable times and its abscissa is the integration number"
        );
        let mut unbinned = curves.clone();
        unbinned.bins.clear();
        assert_eq!(refusal(&unbinned, &req()), "add wavelength bins: the fitted spectrum has one point per bin");
        untimed.bins.clear();
        assert!(refusal(&untimed, &req()).starts_with("a transit fit needs integration times"));

        let out_of_transit = |t0: &str| {
            format!("no integration of this series falls inside the transit of this geometry (t0 {t0}, period 3 d, a/Rs 8, b 0.3, Rp/Rs 0.1): check t0 and the period")
        };
        let refused = [
            (SpectrumFitRequest { period_days: 0.0, ..req() }, "the period must be a number of days above 0, got 0".to_string()),
            (SpectrumFitRequest { period_days: f64::NAN, ..req() }, "the period must be a number of days above 0, got NaN".to_string()),
            (SpectrumFitRequest { t0: f64::NAN, ..req() }, "t0 must be a finite time in the scale of the series".to_string()),
            (SpectrumFitRequest { a_rs: -1.0, ..req() }, "a/Rs must be above 1, got -1".to_string()),
            (SpectrumFitRequest { a_rs: 1.0, ..req() }, "a/Rs must be above 1, got 1".to_string()),
            (SpectrumFitRequest { b: -0.1, ..req() }, "the impact parameter b must be 0 or above, got -0.1".to_string()),
            (SpectrumFitRequest { b: 9.0, ..req() }, "the impact parameter b = 9 must be below a/Rs = 8".to_string()),
            (SpectrumFitRequest { b: 8.0, ..req() }, "the impact parameter b = 8 must be below a/Rs = 8".to_string()),
            (SpectrumFitRequest { rp_start: 0.0, ..req() }, "the starting Rp/Rs must be above 0, got 0".to_string()),
            (
                SpectrumFitRequest { rp_start: 1.4, ..req() },
                "the starting Rp/Rs 1.4 is at or above 1 + b = 1.3: the planet would cover the whole star at mid-transit, which this model does not fit"
                    .to_string(),
            ),
            (SpectrumFitRequest { u1: f64::INFINITY, ..req() }, "u1 and u2 must be finite numbers".to_string()),
            (SpectrumFitRequest { u2: f64::NAN, ..req() }, "u1 and u2 must be finite numbers".to_string()),
            (
                SpectrumFitRequest { u1: 2.5, ..req() },
                "limb darkening out of range: u1 2.5 must be between 0 and 2 and u2 0.2 between -1 and 1".to_string(),
            ),
            (
                SpectrumFitRequest { u2: -1.5, ..req() },
                "limb darkening out of range: u1 0.3 must be between 0 and 2 and u2 -1.5 between -1 and 1".to_string(),
            ),
            (SpectrumFitRequest { baseline_order: 3, ..req() }, "baseline order 3: 0, 1 or 2 are fitted".to_string()),
            (SpectrumFitRequest { t0: 60010.0, ..req() }, out_of_transit("60010")),
            (SpectrumFitRequest { t0: 60001.65, ..req() }, out_of_transit("60001.65")),
            (SpectrumFitRequest { exclude_rows: (35..=85).collect(), ..req() }, out_of_transit("60000.15")),
        ];
        for (request, sentence) in refused {
            assert_eq!(refusal(&curves, &request), sentence, "{request:?}");
        }

        let several = SpectrumFitRequest { period_days: 0.0, a_rs: 0.5, baseline_order: 9, ..req() };
        assert_eq!(refusal(&curves, &several), "the period must be a number of days above 0, got 0");
    }
}
