use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde_json::{json, Value};

use super::{
    collect_detections, core_saturated, flag_fwhm_outliers, header_wcs_cards, input_wcs, measured_detection, merged_cards,
    represent, space_telescope, text_card, tie_catalogue, tie_cone, tie_epoch, tie_observer, tie_sources, unusable_detection,
    unusable_saturation, InputWcs, TieEpochChoice, TieSources, RESAMPLED_KEY, TIE_DETECTION_SIGMA, TIE_MAX_DETECTIONS,
};
use crate::cmd::analysis::{photometry_planes, resolve_dq_mask};
use crate::cmd::common::load_cached_full;
use crate::core::analysis::photometry::{measure_star_prepared, saturation_level, MaskedImage, PhotometryConfig};
use crate::core::analysis::star_detection::detect_stars;
use crate::core::astrometry::catalog::{query_gaia_tie_tsv, ConeQuery};
use crate::core::astrometry::catalog_disk::ENV_CACHE_DIR;
use crate::core::astrometry::gaia_tie::{
    header_update, propagate_linear, tie_to_gaia, unit_vector, GaiaCatalogue, ModelChoice, SkyCorrection, StarResidual,
    TieDecision, TieDetection, TieInput, TieModel, TieOptions, TieResult, BRIGHT_LIMIT_G, RUWE_LIMIT, SOLVED_FIVE_PARAMETER,
    SOLVED_SIX_PARAMETER,
};
use crate::core::astrometry::gwcs::test_support::{mast_dir, real_data_dir};
use crate::core::astrometry::local_solve::{
    healpix_provider, solve_local, CatalogProvider, LocalSolveParams, TierSpec, TWOMASS_TIER, TYCHO2_TIER,
};
use crate::core::astrometry::solve_hints::{Pointing, ScaleRange, SolveHints};
use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
use crate::core::stacking::{never_cancelled, no_stage};
use crate::types::header::HduHeader;

const TAG: &str = "C21VAL";
const MAS_PER_DEG: f64 = 3_600_000.0;
const ARCSEC_PER_DEG: f64 = 3600.0;
const GAIA_EPOCH: f64 = 2016.0;
const GRID_NODES: usize = 16;
const PLATE_SOLVE_DIR_VAR: &str = "ASTROBURST_PLATE_SOLVE_DIR";
const FRAME_FILTER_VAR: &str = "ASTROBURST_C21_FRAMES";
const TIER0_VAR: &str = "ASTROBURST_TIER0_PATH";
const TIER1_VAR: &str = "ASTROBURST_TIER1_PATH";
const FGS_D1: &str = "jw01476003004_02201_00001_guider2_cal.fits";
const FGS_D2: &str = "jw01476003004_02201_00002_guider2_cal.fits";
const FGS_D1_I2D: &str = "jw01476003004_02201_00001_guider2_i2d.fits";
const FGS_D1_CRF: &str = "jw01476003004_02201_00001_guider2_o003_crf.fits";
const J1_REFERENCE_MAS: [f64; 2] = [1029.4, 1438.5];
const J1_REFERENCE_SIGMA_MAS: f64 = 4.5;
const J1_EXTRA_MAS: f64 = 5.0;
const J1_MAX_RMS_MAS: f64 = 50.0;
const J2_MAX_MAS: f64 = 7.0;
const J3_MAX_MAS: f64 = 10.0;
const J3_SIGMAS: f64 = 3.0;
const COMMON_RADIUS_MAS: f64 = 150.0;
const COMMON_MIN_SNR: f64 = 20.0;
const REFIT_LIMIT_PX: f64 = 0.01;
const D1_EPOCH: f64 = 1990.0;
const D1_RMS_MARGIN: f64 = 1.10;
const MEDIAN_TO_SIGMA: f64 = 1.2533;
const D2_TOLERANCE_MAS: f64 = 50.0;
const D2_G_LIMIT: f64 = 19.0;
const D3_SHIFT_ARCSEC: [f64; 2] = [1.7, -0.9];
const D3_ROTATION_DEG: f64 = 0.02;
const D3_SCALE: f64 = 1.0 + 3e-5;
const D3_MAX_MAS: f64 = 10.0;
const D4_SIGMAS: f64 = 3.0;
const D4_SEED: u64 = 0xD4_5EED;
const RETRY_PAUSE: Duration = Duration::from_secs(8);
const D1_FOVS: [&str; 5] = ["fov0.25", "fov0.5", "native", "fov1", "fov2"];
const D1_BANDS: [&str; 6] = ["low1", "low2", "mid1", "mid2", "high1", "high2"];
const D2_ORACLE: [(&str, [f64; 2], [f64; 2]); 7] = [
    ("fov0.25_mid1", [-667.0, 797.0], [140.0, 115.0]),
    ("fov0.25_mid2", [-385.0, 481.0], [94.0, 79.0]),
    ("fov0.25_high1", [-496.0, 721.0], [113.0, 174.0]),
    ("fov0.25_high2", [-291.0, 727.0], [117.0, 101.0]),
    ("fov0.5_mid2", [1507.0, -2178.0], [90.0, 129.0]),
    ("fov0.5_high1", [-303.0, 573.0], [78.0, 94.0]),
    ("native_mid1", [-362.0, 531.0], [132.0, 149.0]),
];
const TIER0_LABEL: &str = TYCHO2_TIER.label;
const TIER1_LABEL: &str = TWOMASS_TIER.label;
const TIER1_MAX_FOV_DEG: f64 = 2.0;
const SOLVE_SCALE_MARGIN: f64 = 0.10;

fn emit(kind: &str, value: Value) {
    eprintln!("{TAG} {kind} {value}");
}

struct Checks {
    failures: Vec<String>,
}

impl Checks {
    fn new() -> Self {
        Self { failures: Vec::new() }
    }

    fn check(&mut self, name: &str, pass: bool, detail: String) {
        emit("check", json!({ "name": name, "pass": pass, "detail": detail }));
        if !pass {
            self.failures.push(format!("{name}: {detail}"));
        }
    }

    fn finish(self) {
        assert!(self.failures.is_empty(), "{} failed checks:\n{}", self.failures.len(), self.failures.join("\n"));
    }
}

struct Frame {
    name: String,
    path: String,
    header: HduHeader,
    size: (usize, usize),
    sources: TieSources,
}

impl Frame {
    fn open(path: &Path) -> Option<Frame> {
        if !path.is_file() {
            eprintln!("skipped: {} is absent", path.display());
            return None;
        }
        let path_text = path.to_string_lossy().into_owned();
        let started = Instant::now();
        let entry = load_cached_full(&path_text).unwrap_or_else(|e| panic!("{path_text}: {e:#}"));
        let header = entry.header().cloned().unwrap_or_else(|| panic!("{path_text}: no header"));
        let (rows, cols) = entry.arr().dim();
        let sources = tie_sources(&path_text, &entry, &header);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        emit(
            "frame",
            json!({
                "file": name,
                "size": [cols, rows],
                "detected": sources.n_detected,
                "kept": sources.detections.len(),
                "usable": sources.detections.iter().filter(|d| !d.saturated).count(),
                "dq_rejected": sources.n_dq_rejected,
                "unmeasured": sources.n_unmeasured,
                "fwhm_outliers": sources.fwhm_outliers.map(|o| json!({ "flagged": o.flagged, "median_px": o.median_px })),
                "seconds": started.elapsed().as_secs_f64(),
            }),
        );
        Some(Frame { name, path: path_text, header, size: (cols, rows), sources })
    }

    fn input(&self, cards: Option<&[(String, String)]>) -> Result<InputWcs, String> {
        input_wcs(&self.path, &self.header, self.size, cards).map_err(|e| format!("{e:#}"))
    }

    fn usable(&self) -> usize {
        self.sources.detections.iter().filter(|d| !d.saturated).count().min(TIE_MAX_DETECTIONS)
    }

    fn epoch(&self, typed: Option<f64>) -> TieEpochChoice {
        tie_epoch(&self.header, typed).unwrap()
    }

    fn options(&self, model: ModelChoice) -> TieOptions {
        TieOptions {
            model,
            max_detections: TIE_MAX_DETECTIONS,
            resampled_pixels: text_card(&self.header, RESAMPLED_KEY).is_some_and(|v| v == "COMPLETE"),
            ..TieOptions::default()
        }
    }
}

fn cache_dir() -> PathBuf {
    std::env::var_os(ENV_CACHE_DIR).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("astroburst-c21-validation"))
}

fn catalogue_for(frame: &Frame, input: &InputWcs) -> Result<(ConeQuery, GaiaCatalogue), String> {
    let query = tie_cone(&input.wcs, frame.size, frame.usable());
    let dir = cache_dir();
    let started = Instant::now();
    let fetch = |q: &ConeQuery| query_gaia_tie_tsv(q);
    let catalogue = tie_catalogue(&query, Some(&dir), &fetch).or_else(|first| {
        eprintln!("{}: retrying the cone after: {first}", frame.name);
        std::thread::sleep(RETRY_PAUSE);
        tie_catalogue(&query, Some(&dir), &fetch)
    })?;
    emit(
        "cone",
        json!({
            "file": frame.name,
            "ra": query.ra,
            "dec": query.dec,
            "radius_deg": query.radius_deg,
            "max_rows": query.max_rows,
            "rows": catalogue.rows.len(),
            "faintest_g": catalogue.rows.iter().filter_map(|r| r.g).fold(f64::NEG_INFINITY, f64::max),
            "source": catalogue.cache_source,
            "seconds": started.elapsed().as_secs_f64(),
        }),
    );
    Ok((query, catalogue))
}

struct Tie {
    result: TieResult,
    cards: Option<Vec<(String, String)>>,
    representation: Option<String>,
    tied: Option<WcsTransform>,
    seconds: f64,
}

impl Tie {
    fn applied(&self) -> bool {
        self.result.decision == TieDecision::Applied && self.tied.is_some()
    }

    fn correction(&self) -> Option<&SkyCorrection> {
        self.result.correction.as_ref().filter(|_| self.applied())
    }
}

fn wcs_of(cards: &[(String, String)], size: (usize, usize)) -> WcsTransform {
    let mut header = HduHeader::empty();
    for (key, value) in cards {
        header.set(key.trim(), value.clone());
    }
    header.set("NAXIS", "2".to_string());
    header.set("NAXIS1", size.0.to_string());
    header.set("NAXIS2", size.1.to_string());
    WcsTransform::from_header(&header).unwrap_or_else(|e| panic!("tied cards do not form a WCS: {e:#}"))
}

fn run_tie(
    frame: &Frame,
    input: &InputWcs,
    detections: &[TieDetection],
    catalogue: &GaiaCatalogue,
    epoch: TieEpochChoice,
    options: &TieOptions,
) -> Tie {
    let started = Instant::now();
    let tie_input = TieInput {
        wcs: &input.wcs,
        width: frame.size.0,
        height: frame.size.1,
        detections,
        catalogue,
        epoch: epoch.epoch(),
        observer_barycentric_au: tie_observer(&frame.header, &epoch),
        space_telescope: space_telescope(&frame.header),
    };
    let mut result = tie_to_gaia(&tie_input, options);
    let refined = if result.decision == TieDecision::Applied {
        match represent(input, &result, frame.size) {
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
    let tied = refined.as_ref().map(|(cards, _)| wcs_of(cards, frame.size));
    Tie {
        result,
        cards: refined.as_ref().map(|(c, _)| c.clone()),
        representation: refined.map(|(_, r)| r),
        tied,
        seconds: started.elapsed().as_secs_f64(),
    }
}

fn rms_about_zero(stars: &[&StarResidual], pick: fn(&StarResidual) -> [f64; 2]) -> f64 {
    if stars.is_empty() {
        return f64::NAN;
    }
    (stars.iter().map(|s| {
        let r = pick(s);
        r[0] * r[0] + r[1] * r[1]
    })
    .sum::<f64>()
        / stars.len() as f64)
        .sqrt()
}

fn summary(label: &str, frame: &Frame, input: &InputWcs, epoch: &TieEpochChoice, tie: &Tie) -> Value {
    let r = &tie.result.report;
    let used: Vec<&StarResidual> = r.stars.iter().filter(|s| s.used).collect();
    let significant: Vec<Value> = r
        .slopes
        .iter()
        .filter(|s| s.sigma > 0.0 && (s.value / s.sigma).abs() > 3.0)
        .map(|s| json!({ "against": s.against, "axis": s.axis, "value": s.value, "sigma": s.sigma }))
        .collect();
    let mut out = json!({
        "label": label,
        "file": frame.name,
        "input": input.kind,
        "wcs_kind": r.wcs_kind,
        "decision": tie.result.decision.name(),
        "reason": tie.result.reason,
        "model": r.model,
        "notes": r.model_notes,
        "warnings": r.warnings,
        "epoch": r.epoch_year,
        "epoch_source": epoch.source_name(),
        "dt_yr": r.delta_t_yr,
        "epoch_systematic_mas": r.epoch_systematic_mas,
        "parallax": r.parallax,
        "rows": r.catalogue_rows,
        "cuts": r.cuts,
    });
    let matching = json!({
        "histogram": { "bin": r.histogram_bin_arcsec, "peak": r.histogram_peak, "random": r.histogram_random_per_bin, "threshold": r.histogram_threshold, "expected_false_peaks": r.histogram_expected_false_peaks, "offset_arcsec": r.peak_offset_arcsec, "sigma_res_arcsec": r.sigma_res_arcsec },
        "density": r.density_per_arcsec2,
        "r_chance": r.chance_radius_arcsec,
        "r_m": r.match_radius_arcsec,
        "chance_fraction": r.chance_fraction,
        "expected_chance_pairs": r.expected_chance_pairs,
        "n_matched": r.n_matched,
        "n_used": r.n_used,
    });
    let residuals = json!({
        "before": { "median": r.before.median_mas, "robust": r.before.robust_rms_mas, "weighted": r.before.weighted_rms_mas, "rms0": rms_about_zero(&used, |s| s.before_mas) },
        "after": { "median": r.after.median_mas, "robust": r.after.robust_rms_mas, "weighted": r.after.weighted_rms_mas, "rms0": rms_about_zero(&used, |s| s.after_mas) },
        "cv_before": r.cv_rms_before_mas,
        "cv_after": r.cv_rms_after_mas,
        "cv_median_after": r.cv_median_after_mas,
        "cv_median_sigma": r.cv_median_sigma_mas,
    });
    let fit = json!({
        "params": r.params,
        "sigma_formal": r.sigma_formal,
        "sigma_bootstrap": r.sigma_bootstrap,
        "bootstrap_failures": r.bootstrap_failures,
        "errors_disagree": r.errors_disagree,
        "sigma_sys_mas": r.sigma_sys_mas,
        "chi2_nu": r.chi2_nu,
        "passes": r.passes,
        "last_update_mas": r.last_update_mas,
        "uncertainty_centre_mas": r.uncertainty_centre_mas,
        "uncertainty_corner_mas": r.uncertainty_corner_mas,
        "g_range": r.g_range,
        "significant_slopes": significant,
        "representation": tie.representation,
        "seconds": tie.seconds,
    });
    for part in [matching, residuals, fit] {
        if let (Some(target), Value::Object(fields)) = (out.as_object_mut(), part) {
            target.extend(fields);
        }
    }
    out
}

fn grid_pixels(size: (usize, usize)) -> Vec<(f64, f64)> {
    let step = |n: usize, i: usize| (n as f64 - 1.0) * i as f64 / (GRID_NODES - 1) as f64;
    (0..GRID_NODES).flat_map(|j| (0..GRID_NODES).map(move |i| (step(size.0, i), step(size.1, j)))).collect()
}

fn world(wcs: &WcsTransform, x: f64, y: f64) -> Option<(f64, f64)> {
    let c = wcs.pixel_to_world(x, y);
    (c.ra.is_finite() && c.dec.is_finite()).then_some((c.ra, c.dec))
}

fn sep_mas(a: (f64, f64), b: (f64, f64)) -> f64 {
    angular_separation(a.0, a.1, b.0, b.1) * MAS_PER_DEG
}

fn offset_mas(from: (f64, f64), to: (f64, f64)) -> [f64; 2] {
    let mut d_ra = to.0 - from.0;
    if d_ra > 180.0 {
        d_ra -= 360.0;
    }
    if d_ra < -180.0 {
        d_ra += 360.0;
    }
    let dec = 0.5 * (from.1 + to.1);
    [d_ra * dec.to_radians().cos() * MAS_PER_DEG, (to.1 - from.1) * MAS_PER_DEG]
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        0.5 * (sorted[n / 2 - 1] + sorted[n / 2])
    }
}

fn stats(values: &[f64]) -> Value {
    let finite: Vec<f64> = values.iter().cloned().filter(|v| v.is_finite()).collect();
    let max = finite.iter().cloned().fold(f64::NAN, f64::max);
    let rms = (finite.iter().map(|v| v * v).sum::<f64>() / finite.len().max(1) as f64).sqrt();
    json!({ "n": finite.len(), "max": max, "median": median(&finite), "rms": rms })
}

fn max_of(values: &[f64]) -> f64 {
    values.iter().cloned().filter(|v| v.is_finite()).fold(f64::NAN, f64::max)
}

fn grid_separations(size: (usize, usize), a: &dyn Fn(f64, f64) -> Option<(f64, f64)>, b: &dyn Fn(f64, f64) -> Option<(f64, f64)>) -> Vec<f64> {
    grid_pixels(size)
        .into_iter()
        .map(|(x, y)| match (a(x, y), b(x, y)) {
            (Some(p), Some(q)) => sep_mas(p, q),
            _ => f64::NAN,
        })
        .collect()
}

fn sky_points(detections: &[TieDetection], at: &dyn Fn(f64, f64) -> Option<(f64, f64)>, min_snr: f64) -> Vec<(f64, f64)> {
    detections.iter().filter(|d| !d.saturated && d.snr >= min_snr).filter_map(|d| at(d.x, d.y)).collect()
}

fn common_offsets(a: &[(f64, f64)], b: &[(f64, f64)], radius_mas: f64) -> Vec<[f64; 2]> {
    let mut order: Vec<usize> = (0..b.len()).collect();
    order.sort_by(|&i, &j| b[i].1.total_cmp(&b[j].1));
    let radius_deg = radius_mas / MAS_PER_DEG;
    let mut out = Vec::new();
    for &p in a {
        let start = order.partition_point(|&i| b[i].1 < p.1 - radius_deg);
        let mut best: Option<(f64, usize)> = None;
        let mut close = 0;
        for &i in order[start..].iter().take_while(|&&i| b[i].1 <= p.1 + radius_deg) {
            let s = sep_mas(p, b[i]);
            if s <= radius_mas {
                close += 1;
                if best.is_none_or(|(bs, _)| s < bs) {
                    best = Some((s, i));
                }
            }
        }
        if let (Some((_, i)), 1) = (best, close) {
            out.push(offset_mas(p, b[i]));
        }
    }
    out
}

fn offsets_summary(offsets: &[[f64; 2]]) -> Value {
    let e: Vec<f64> = offsets.iter().map(|o| o[0]).collect();
    let n: Vec<f64> = offsets.iter().map(|o| o[1]).collect();
    let norm: Vec<f64> = offsets.iter().map(|o| o[0].hypot(o[1])).collect();
    json!({ "n": offsets.len(), "median_vector": [median(&e), median(&n)], "median_abs": median(&norm), "rms": stats(&norm)["rms"] })
}

fn good_row(row: &crate::core::astrometry::gaia_tie::GaiaRow) -> bool {
    matches!(row.solved, Some(SOLVED_FIVE_PARAMETER) | Some(SOLVED_SIX_PARAMETER))
        && row.ruwe.is_some_and(|r| r < RUWE_LIMIT)
        && row.dup != Some(true)
        && row.g.is_none_or(|g| g >= BRIGHT_LIMIT_G)
}

fn match_audit(label: &str, frame: &Frame, input: &InputWcs, catalogue: &GaiaCatalogue, tie: &Tie) {
    let Some(correction) = tie.correction() else { return };
    let Some(tied) = tie.tied.as_ref() else { return };
    let epoch = tie.result.report.epoch_year;
    let dt = epoch - GAIA_EPOCH;
    let r_m = tie.result.report.match_radius_arcsec * 1000.0;
    let gaia: Vec<(f64, f64)> = catalogue
        .rows
        .iter()
        .filter(|r| good_row(r))
        .map(|r| propagate_linear(r.ra_deg, r.dec_deg, r.pm_ra_masyr.unwrap_or(0.0), r.pm_dec_masyr.unwrap_or(0.0), dt))
        .collect();
    let inside = gaia
        .iter()
        .filter(|&&(ra, dec)| {
            let (x, y) = tied.world_to_pixel(ra, dec);
            x >= 0.0 && y >= 0.0 && x <= frame.size.0 as f64 - 1.0 && y <= frame.size.1 as f64 - 1.0
        })
        .count();
    let at = |x: f64, y: f64| correction.corrected_pixel_to_world(&input.wcs, x, y);
    let dets = sky_points(&frame.sources.detections, &at, 0.0);
    let within = |radius: f64| {
        dets.iter()
            .filter(|&&p| gaia.iter().any(|&g| (g.1 - p.1).abs() * MAS_PER_DEG <= radius && sep_mas(p, g) <= radius))
            .count()
    };
    emit(
        "audit",
        json!({
            "label": label,
            "file": frame.name,
            "usable_detections": dets.len(),
            "good_gaia_rows": gaia.len(),
            "good_gaia_inside_frame": inside,
            "r_m_mas": r_m,
            "detections_with_good_gaia_within_r_m": within(r_m),
            "detections_with_good_gaia_within_2r_m": within(2.0 * r_m),
            "detections_with_good_gaia_within_250mas": within(250.0),
            "core_matched": tie.result.report.n_matched,
            "core_used": tie.result.report.n_used,
            "cuts": tie.result.report.cuts,
        }),
    );
}

fn mast_frame(name: &str) -> Option<Frame> {
    Frame::open(&mast_dir().join(name))
}

fn plate_solve_dir() -> Option<PathBuf> {
    let dir = std::env::var_os(PLATE_SOLVE_DIR_VAR).map(PathBuf::from)?;
    dir.is_dir().then_some(dir)
}

fn selected(name: &str) -> bool {
    std::env::var(FRAME_FILTER_VAR).map_or(true, |list| list.split(',').any(|n| !n.trim().is_empty() && name.contains(n.trim())))
}

fn c1_names() -> Vec<String> {
    D1_FOVS.iter().flat_map(|fov| D1_BANDS.iter().map(move |band| format!("{fov}_{band}"))).filter(|n| selected(n)).collect()
}

fn c1_frame(name: &str) -> Option<Frame> {
    Frame::open(&plate_solve_dir()?.join("c1").join(format!("{name}.fits")))
}

fn shift_axes(tie: &Tie) -> ([f64; 2], [f64; 2]) {
    let r = &tie.result.report;
    let sigma = r.sigma_bootstrap.unwrap_or(r.sigma_formal);
    (r.params.shift_mas, sigma.shift_mas)
}

fn j1_allowance(sigma: f64) -> f64 {
    3.0 * (sigma * sigma + J1_REFERENCE_SIGMA_MAS * J1_REFERENCE_SIGMA_MAS).sqrt() + J1_EXTRA_MAS
}

fn correction_separations(points: &[(f64, f64)], a: &SkyCorrection, b: &SkyCorrection) -> Vec<f64> {
    points
        .iter()
        .map(|&(ra, dec)| match (a.apply(ra, dec), b.apply(ra, dec)) {
            (Some(p), Some(q)) => sep_mas(p, q),
            _ => f64::NAN,
        })
        .collect()
}

fn grid_sky(frame: &Frame, wcs: &WcsTransform) -> Vec<(f64, f64)> {
    grid_pixels(frame.size).into_iter().filter_map(|(x, y)| world(wcs, x, y)).collect()
}

fn refused_by_a_chance_gate(tie: &Tie) -> bool {
    tie.result.decision == TieDecision::Refused
        && tie.result.reason.as_deref().is_some_and(|r| r.starts_with("no significant offset peak") || r.starts_with("chance matches"))
}

fn deep_tie_check(checks: &mut Checks, label: &str, frame: &Frame, input: &InputWcs, reference: &Tie, deep: &Tie) {
    let name = format!("{label} refused by a chance gate, or within 10 mas of the depth {TIE_MAX_DETECTIONS} reference");
    match (deep.correction(), reference.correction()) {
        (Some(c), Some(c_ref)) => {
            let seps = correction_separations(&grid_sky(frame, &input.wcs), c_ref, c);
            emit("grid", json!({ "label": format!("{label} vs reference"), "file": frame.name, "sep_mas": stats(&seps) }));
            checks.check(&name, max_of(&seps) <= J3_MAX_MAS, format!("max {:.1} mas", max_of(&seps)));
        }
        (Some(_), None) => checks.check(&name, false, format!("applied while the reference was not: {:?}", reference.result.reason)),
        (None, _) => checks.check(&name, refused_by_a_chance_gate(deep), format!("{:?} {:?}", deep.result.decision, deep.result.reason)),
    }
}

fn j3_sigma_checks(checks: &mut Checks, label: &str, frame: &Frame, input: &InputWcs, ties: (&Tie, &Tie), corrections: (&SkyCorrection, &SkyCorrection)) -> Vec<f64> {
    let (c1, c2) = corrections;
    let (r1, r2) = (&ties.0.result.report, &ties.1.result.report);
    let seps = correction_separations(&grid_sky(frame, &input.wcs), c1, c2);
    let corner_sigma = r1.uncertainty_corner_mas.hypot(r2.uncertainty_corner_mas);
    let centre_sigma = r1.uncertainty_centre_mas.hypot(r2.uncertainty_centre_mas);
    let (cx, cy) = crate::core::astrometry::wcs::pixel_center(frame.size.0, frame.size.1);
    let centre_sep = world(&input.wcs, cx, cy).map_or(f64::NAN, |p| correction_separations(&[p], c1, c2)[0]);
    let max = max_of(&seps);
    emit("j3", json!({
        "label": label,
        "file": frame.name,
        "sep_mas": stats(&seps),
        "centre_sep_mas": centre_sep,
        "sigma_centre_pair_mas": centre_sigma,
        "sigma_corner_pair_mas": corner_sigma,
        "max_ratio_to_corner_sigma": max / corner_sigma,
        "centre_ratio": centre_sep / centre_sigma,
        "within_10_mas_everywhere": max <= J3_MAX_MAS,
        "models": [r1.model, r2.model],
        "rotation_deg": [r1.params.rotation_deg, r2.params.rotation_deg],
    }));
    checks.check(&format!("{label} grid within 3 sqrt2 sigma at every node (corner sigma)"), max <= J3_SIGMAS * corner_sigma, format!("max {max:.1} vs {:.1} mas", J3_SIGMAS * corner_sigma));
    checks.check(&format!("{label} centre within 3 sqrt2 sigma (centre sigma)"), centre_sep <= J3_SIGMAS * centre_sigma, format!("{centre_sep:.1} vs {:.1} mas", J3_SIGMAS * centre_sigma));
    seps
}

#[test]
#[ignore]
fn live_c21_fgs_j1_j2_j3() {
    let mut checks = Checks::new();
    let Some(d1) = mast_frame(FGS_D1) else { return };
    let Some(d2) = mast_frame(FGS_D2) else { return };
    let in1 = d1.input(None).unwrap();
    let (_, cat1) = catalogue_for(&d1, &in1).unwrap();
    let e1 = d1.epoch(None);
    let t1 = run_tie(&d1, &in1, &d1.sources.detections, &cat1, e1, &d1.options(ModelChoice::Auto));
    emit("tie", summary("J1 dither 1 gwcs", &d1, &in1, &e1, &t1));
    match_audit("J1 dither 1 gwcs", &d1, &in1, &cat1, &t1);

    let in2 = d2.input(None).unwrap();
    let (_, cat2) = catalogue_for(&d2, &in2).unwrap();
    let e2 = d2.epoch(None);
    let t2 = run_tie(&d2, &in2, &d2.sources.detections, &cat2, e2, &d2.options(ModelChoice::Auto));
    emit("tie", summary("J1 dither 2 gwcs", &d2, &in2, &e2, &t2));

    for (label, tie) in [("dither 1", &t1), ("dither 2", &t2)] {
        checks.check(&format!("J1 {label} applied"), tie.applied(), format!("{:?} {:?}", tie.result.decision, tie.result.reason));
        if !tie.applied() {
            continue;
        }
        let robust = tie.result.report.after.robust_rms_mas[2];
        checks.check(&format!("J1 {label} post-fit robust RMS <= 50 mas"), robust <= J1_MAX_RMS_MAS, format!("{robust:.1} mas"));
        let (shift, sigma) = shift_axes(tie);
        for axis in 0..2 {
            let off = (shift[axis] - J1_REFERENCE_MAS[axis]).abs();
            let allowance = j1_allowance(sigma[axis]);
            checks.check(
                &format!("J1 {label} shift axis {axis} vs crude oracle"),
                off <= allowance,
                format!("shift {:.1} +/- {:.1}, oracle {:.1}, off {off:.1}, allowance {allowance:.1}", shift[axis], sigma[axis], J1_REFERENCE_MAS[axis]),
            );
        }
    }

    let header_cards = header_wcs_cards(&d1.header);
    let in1h = d1.input(Some(&header_cards)).unwrap();
    let t1h = run_tie(&d1, &in1h, &d1.sources.detections, &cat1, e1, &d1.options(ModelChoice::Auto));
    emit("tie", summary("J2 dither 1 header SIP", &d1, &in1h, &e1, &t1h));
    let untied = grid_separations(d1.size, &|x, y| world(&in1.wcs, x, y), &|x, y| world(&in1h.wcs, x, y));
    emit("grid", json!({ "label": "J2 untied gwcs vs header SIP", "sep_mas": stats(&untied) }));
    if let (Some(a), Some(b)) = (t1.tied.as_ref(), t1h.tied.as_ref()) {
        let tied = grid_separations(d1.size, &|x, y| world(a, x, y), &|x, y| world(b, x, y));
        emit("grid", json!({ "label": "J2 tied gwcs refit vs tied header SIP", "sep_mas": stats(&tied) }));
        checks.check("J2 tied grids agree within 7 mas", max_of(&tied) <= J2_MAX_MAS, format!("max {:.2} mas", max_of(&tied)));
        if let Some(c) = t1.correction() {
            let refit = grid_separations(d1.size, &|x, y| world(a, x, y), &|x, y| c.corrected_pixel_to_world(&in1.wcs, x, y));
            emit("grid", json!({ "label": "J2 gwcs refit cards vs correction o gwcs", "sep_mas": stats(&refit) }));
        }
    } else {
        checks.check("J2 both ties applied", false, format!("gwcs {:?}, header {:?}", t1.result.reason, t1h.result.reason));
    }

    if let (Some(c1), Some(c2)) = (t1.correction(), t2.correction()) {
        let seps = j3_sigma_checks(&mut checks, "J3 dither 1 vs dither 2", &d1, &in1, (&t1, &t2), (c1, c2));
        emit("grid", json!({ "label": "J3 dither 1 vs dither 2 correction on the dither 1 grid", "sep_mas": stats(&seps) }));
        let tied1 = |x: f64, y: f64| c1.corrected_pixel_to_world(&in1.wcs, x, y);
        let tied2 = |x: f64, y: f64| c2.corrected_pixel_to_world(&in2.wcs, x, y);
        let raw1 = |x: f64, y: f64| world(&in1.wcs, x, y);
        let raw2 = |x: f64, y: f64| world(&in2.wcs, x, y);
        for min_snr in [COMMON_MIN_SNR, 50.0, 100.0] {
            let after = common_offsets(&sky_points(&d1.sources.detections, &tied1, min_snr), &sky_points(&d2.sources.detections, &tied2, min_snr), COMMON_RADIUS_MAS);
            let before = common_offsets(&sky_points(&d1.sources.detections, &raw1, min_snr), &sky_points(&d2.sources.detections, &raw2, min_snr), COMMON_RADIUS_MAS);
            let after_summary = offsets_summary(&after);
            emit("common", json!({ "label": "J3 common stars dither 2 minus dither 1", "min_snr": min_snr, "tied": after_summary, "untied": offsets_summary(&before) }));
            if min_snr == COMMON_MIN_SNR {
                let med = after_summary["median_abs"].as_f64().unwrap_or(f64::NAN);
                emit("j3", json!({
                    "label": "J3 report, common stars and models",
                    "n": after.len(),
                    "median_abs_mas": med,
                    "median_abs_within_10_mas": med <= J3_MAX_MAS,
                    "median_vector_mas": after_summary["median_vector"],
                    "untied_median_abs_mas": offsets_summary(&before)["median_abs"],
                    "grid_max_mas": max_of(&seps),
                    "grid_median_mas": median(&seps),
                    "models": [t1.result.report.model.clone(), t2.result.report.model.clone()],
                }));
            }
        }
        let (s1, _) = shift_axes(&t1);
        let (s2, _) = shift_axes(&t2);
        for axis in 0..2 {
            let apart = (s1[axis] - s2[axis]).abs();
            checks.check(&format!("J3 shift axis {axis} agree within 10 mas"), apart <= J3_MAX_MAS, format!("{apart:.2} mas"));
        }
    }

    if let Some(i2d) = mast_frame(FGS_D1_I2D) {
        let ini = i2d.input(None).unwrap();
        let (_, cati) = catalogue_for(&i2d, &ini).unwrap();
        let ei = i2d.epoch(None);
        let ti = run_tie(&i2d, &ini, &i2d.sources.detections, &cati, ei, &i2d.options(ModelChoice::Auto));
        emit("tie", summary("J1b dither 1 i2d header TAN", &i2d, &ini, &ei, &ti));
        if let (Some(cc), Some(ci)) = (t1.correction(), ti.correction()) {
            let seps = correction_separations(&grid_sky(&d1, &in1.wcs), cc, ci);
            emit("grid", json!({ "label": "J1b cal vs i2d correction on the cal grid", "sep_mas": stats(&seps) }));
            let tied_cal = |x: f64, y: f64| cc.corrected_pixel_to_world(&in1.wcs, x, y);
            let tied_i2d = |x: f64, y: f64| ci.corrected_pixel_to_world(&ini.wcs, x, y);
            let raw_cal = |x: f64, y: f64| world(&in1.wcs, x, y);
            let raw_i2d = |x: f64, y: f64| world(&ini.wcs, x, y);
            let after = common_offsets(&sky_points(&i2d.sources.detections, &tied_i2d, 50.0), &sky_points(&d1.sources.detections, &tied_cal, 50.0), COMMON_RADIUS_MAS);
            let before = common_offsets(&sky_points(&i2d.sources.detections, &raw_i2d, 50.0), &sky_points(&d1.sources.detections, &raw_cal, 50.0), COMMON_RADIUS_MAS);
            emit("common", json!({ "label": "J1b common stars cal minus i2d", "min_snr": 50.0, "tied": offsets_summary(&after), "untied": offsets_summary(&before) }));
        }
    }
    checks.finish();
}

fn tie_file(checks: &mut Checks, label: &str, frame: &Frame, model: ModelChoice) -> Option<Tie> {
    let input = match frame.input(None) {
        Ok(input) => input,
        Err(reason) => {
            emit("tie", json!({ "label": label, "file": frame.name, "decision": "no input WCS", "reason": reason }));
            return None;
        }
    };
    let catalogue = match catalogue_for(frame, &input) {
        Ok((_, catalogue)) => catalogue,
        Err(reason) => {
            emit("tie", json!({ "label": label, "file": frame.name, "decision": "not run", "reason": reason }));
            checks.check(&format!("{label} cone fetched"), false, reason);
            return None;
        }
    };
    let epoch = frame.epoch(None);
    let tie = run_tie(frame, &input, &frame.sources.detections, &catalogue, epoch, &frame.options(model));
    emit("tie", summary(label, frame, &input, &epoch, &tie));
    match_audit(label, frame, &input, &catalogue, &tie);
    Some(tie)
}

fn refused_naming_count(tie: &Tie) -> Option<usize> {
    let reason = tie.result.reason.as_deref()?;
    (tie.result.decision == TieDecision::Refused)
        .then(|| reason.strip_suffix(" used pairs for shift").and_then(|r| r.split(" < ").next()).and_then(|n| n.trim().parse().ok()))
        .flatten()
}

#[test]
#[ignore]
fn live_c21_jwst_refusals_and_products_j5_j6() {
    let mut checks = Checks::new();
    let mast = [
        ("J5 nrca1 cal", "jw01475001001_02101_00001_nrca1_cal.fits"),
        ("J5 nis cal", "jw01475001001_02201_00001_nis_cal.fits"),
        ("J5 nrca1 i2d", "jw01475001001_02101_00001_nrca1_i2d.fits"),
        ("J6 nrca1 o001 crf", "jw01475001001_02101_00001_nrca1_o001_crf.fits"),
        ("J6 nis o001 crf", "jw01475001001_02201_00001_nis_o001_crf.fits"),
        ("J6 guider2 o003 crf", "jw01476003004_02201_00001_guider2_o003_crf.fits"),
    ];
    for (label, name) in mast {
        if !selected(name) {
            continue;
        }
        let Some(frame) = mast_frame(name) else { continue };
        let Some(tie) = tie_file(&mut checks, label, &frame, ModelChoice::Auto) else { continue };
        if label == "J5 nrca1 cal" {
            let count = refused_naming_count(&tie);
            checks.check("J5 nrca1 cal refused naming 0 or 1", count.is_some_and(|n| n <= 1), format!("{:?}", tie.result.reason));
        }
        if label == "J5 nis cal" {
            let count = refused_naming_count(&tie);
            checks.check("J5 NIRISS cal refused naming at most 2", count.is_some_and(|n| n <= 2), format!("{:?}", tie.result.reason));
        }
    }
    let gwcs = [("J5 M16 nrca1 cal gwcs", "jw02739001001_02105_00001_nrca1_cal.fits"), ("J5 M16 mirimage cal gwcs", "jw02739002001_02101_00001_mirimage_cal.fits")];
    for (label, name) in gwcs {
        if !selected(name) {
            continue;
        }
        let Some(frame) = Frame::open(&real_data_dir().join(name)) else { continue };
        let Some(tie) = tie_file(&mut checks, label, &frame, ModelChoice::Auto) else { continue };
        if label == "J5 M16 nrca1 cal gwcs" {
            let model = tie.result.report.model.clone().unwrap_or_default();
            let refit_px = tie
                .representation
                .as_deref()
                .and_then(|r| r.split("max error ").nth(1))
                .and_then(|r| r.trim_end_matches(" px").parse::<f64>().ok());
            let ok = match tie.result.decision {
                TieDecision::Applied => (model == TieModel::Shift.name() || model == TieModel::Rshift.name()) && refit_px.is_some_and(|e| e <= REFIT_LIMIT_PX),
                _ => refused_naming_count(&tie).is_some() || tie.result.decision == TieDecision::NoSignificantCorrection || (tie.result.decision == TieDecision::Refused && tie.result.reason.as_deref().is_some_and(|r| r.starts_with("no significant offset peak"))),
            };
            checks.check("J5 M16 nrca1 shift or rshift with refit <= 0.01 px, or a named refusal", ok, format!("{:?} {model} {:?} {:?}", tie.result.decision, tie.representation, tie.result.reason));
        }
    }
    let Some(dir) = plate_solve_dir() else {
        checks.finish();
        return;
    };
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("c3").join("manifest.json")).unwrap()).unwrap();
    for frame_entry in manifest["frames"].as_array().cloned().unwrap_or_default() {
        if frame_entry["kind"] != "i2d" {
            continue;
        }
        let path = PathBuf::from(frame_entry["file"].as_str().unwrap_or_default());
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !selected(&name) {
            continue;
        }
        let Some(frame) = Frame::open(&path) else { continue };
        tie_file(&mut checks, "C3 i2d mosaic", &frame, ModelChoice::Auto);
    }
    checks.finish();
}

struct C1Case {
    frame: Frame,
    header_input: InputWcs,
    catalogue: GaiaCatalogue,
}

fn c1_case(checks: &mut Checks, name: &str) -> Option<C1Case> {
    let frame = c1_frame(name)?;
    let header_input = frame.input(None).ok()?;
    match catalogue_for(&frame, &header_input) {
        Ok((_, catalogue)) => Some(C1Case { frame, header_input, catalogue }),
        Err(reason) => {
            emit("tie", json!({ "label": "D1", "file": frame.name, "decision": "not run", "reason": reason }));
            checks.check(&format!("D1 {name} cone fetched"), false, reason);
            None
        }
    }
}

fn d1_checks(checks: &mut Checks, label: &str, name: &str, tie: &Tie, typed: bool) {
    let r = &tie.result.report;
    match tie.result.decision {
        TieDecision::Applied => {
            let n = r.n_used.max(1) as f64;
            for axis in 0..2 {
                let limit = 3.0 * MEDIAN_TO_SIGMA * r.after.robust_rms_mas[axis] / n.sqrt();
                let med = r.after.median_mas[axis];
                checks.check(&format!("{label} {name} median after axis {axis}"), med.abs() <= limit, format!("{med:.1} vs {limit:.1} mas (N {})", r.n_used));
            }
            checks.check(&format!("{label} {name} CV after <= before"), r.cv_rms_after_mas <= r.cv_rms_before_mas, format!("{:.1} -> {:.1} mas", r.cv_rms_before_mas, r.cv_rms_after_mas));
            if typed {
                if let Some((_, _, table)) = D2_ORACLE.iter().find(|(n, _, _)| *n == name) {
                    for (axis, sd) in table.iter().enumerate() {
                        let limit = sd * D1_RMS_MARGIN;
                        let got = r.after.robust_rms_mas[axis];
                        checks.check(&format!("{label} {name} robust RMS axis {axis} vs oracle sd"), got <= limit, format!("{got:.1} vs {limit:.1} mas"));
                    }
                }
            }
        }
        TieDecision::NoSignificantCorrection => {
            emit("check", json!({ "name": format!("{label} {name} no significant correction"), "pass": true, "detail": tie.result.reason }));
        }
        TieDecision::Refused => {
            emit("check", json!({ "name": format!("{label} {name} refused"), "pass": true, "detail": tie.result.reason }));
        }
    }
}

#[test]
#[ignore]
fn live_c21_dss2_d1_header_control() {
    let mut checks = Checks::new();
    if plate_solve_dir().is_none() {
        eprintln!("skipped: set {PLATE_SOLVE_DIR_VAR}");
        return;
    }
    for name in c1_names() {
        let Some(case) = c1_case(&mut checks, &name) else { continue };
        for typed in [None, Some(D1_EPOCH)] {
            let epoch = case.frame.epoch(typed);
            let tie = run_tie(&case.frame, &case.header_input, &case.frame.sources.detections, &case.catalogue, epoch, &case.frame.options(ModelChoice::Auto));
            let label = if typed.is_some() { "D1 header 1990" } else { "D1 header undated" };
            emit("tie", summary(label, &case.frame, &case.header_input, &epoch, &tie));
            if name == "fov0.25_mid1" && typed.is_some() {
                match_audit(label, &case.frame, &case.header_input, &case.catalogue, &tie);
            }
            d1_checks(&mut checks, label, &name, &tie, typed.is_some());
        }
    }
    checks.finish();
}

fn tier_provider(var: &str, spec: TierSpec) -> Option<Box<dyn CatalogProvider>> {
    let path = std::env::var_os(var).map(PathBuf::from)?;
    healpix_provider(&path, spec).ok()
}

#[test]
#[ignore]
fn live_c21_dss2_d1_local_solve_then_tie() {
    let mut checks = Checks::new();
    if plate_solve_dir().is_none() {
        eprintln!("skipped: set {PLATE_SOLVE_DIR_VAR}");
        return;
    }
    let tier0 = tier_provider(TIER0_VAR, TYCHO2_TIER);
    let tier1 = tier_provider(TIER1_VAR, TWOMASS_TIER);
    assert!(tier0.is_some() || tier1.is_some(), "set {TIER0_VAR} and/or {TIER1_VAR}");
    for name in c1_names() {
        let Some(case) = c1_case(&mut checks, &name) else { continue };
        let truth = &case.header_input.wcs;
        let scale = truth.pixel_scale_arcsec();
        let (cx, cy) = crate::core::astrometry::wcs::pixel_center(case.frame.size.0, case.frame.size.1);
        let centre = truth.pixel_to_world(cx, cy);
        let fov = case.frame.size.0.max(case.frame.size.1) as f64 * scale / ARCSEC_PER_DEG;
        let hints = SolveHints {
            centre: Some(Pointing { ra: centre.ra, dec: centre.dec, source: "c21 validation".to_string() }),
            scale: Some(ScaleRange { low_arcsec: scale * (1.0 - SOLVE_SCALE_MARGIN), high_arcsec: scale * (1.0 + SOLVE_SCALE_MARGIN), source: "c21 validation".to_string() }),
            fov_deg: Some(fov),
            ..SolveHints::default()
        };
        let entry = load_cached_full(&case.frame.path).unwrap();
        let order: Vec<(&str, &Option<Box<dyn CatalogProvider>>)> =
            if fov < TIER1_MAX_FOV_DEG { vec![(TIER1_LABEL, &tier1), (TIER0_LABEL, &tier0)] } else { vec![(TIER0_LABEL, &tier0), (TIER1_LABEL, &tier1)] };
        let mut solution = None;
        let mut failures = Vec::new();
        for (label, provider) in order {
            let Some(provider) = provider.as_ref() else { continue };
            match solve_local(entry.arr(), &hints, provider.as_ref(), &LocalSolveParams::default(), &never_cancelled, &no_stage) {
                Ok(found) => {
                    solution = Some((label, found));
                    break;
                }
                Err(failure) => failures.push(format!("{label}: {:?} {}", failure.reason, failure.message)),
            }
        }
        let Some((tier, solved)) = solution else {
            emit("solve", json!({ "file": case.frame.name, "solved": false, "failures": failures }));
            continue;
        };
        let solve_input = match case.frame.input(Some(&solved.wcs_cards)) {
            Ok(input) => input,
            Err(reason) => {
                checks.check(&format!("D1 solve {name} cards form a WCS"), false, reason);
                continue;
            }
        };
        let vs_truth = grid_separations(case.frame.size, &|x, y| world(&solve_input.wcs, x, y), &|x, y| world(truth, x, y));
        emit("solve", json!({ "file": case.frame.name, "solved": true, "tier": tier, "matched": solved.matched, "rms_px": solved.rms_px, "sip_order": solved.sip_order, "vs_header_mas": stats(&vs_truth) }));
        let header_epoch = case.frame.epoch(Some(D1_EPOCH));
        let header_tie = run_tie(&case.frame, &case.header_input, &case.frame.sources.detections, &case.catalogue, header_epoch, &case.frame.options(ModelChoice::Auto));
        for typed in [None, Some(D1_EPOCH)] {
            let epoch = case.frame.epoch(typed);
            let tie = run_tie(&case.frame, &solve_input, &case.frame.sources.detections, &case.catalogue, epoch, &case.frame.options(ModelChoice::Auto));
            let label = if typed.is_some() { "D1 solve 1990" } else { "D1 solve undated" };
            emit("tie", summary(label, &case.frame, &solve_input, &epoch, &tie));
            d1_checks(&mut checks, label, &name, &tie, false);
            if typed.is_some() {
                if let (Some(a), Some(b)) = (tie.tied.as_ref(), header_tie.tied.as_ref()) {
                    let seps = grid_separations(case.frame.size, &|x, y| world(a, x, y), &|x, y| world(b, x, y));
                    emit("grid", json!({ "label": "D1 tied solve vs tied header (1990, auto)", "file": case.frame.name, "sep_mas": stats(&seps), "untied_mas": stats(&vs_truth) }));
                }
            }
        }
    }
    checks.finish();
}

#[test]
#[ignore]
fn live_c21_dss2_d2_independent_oracle() {
    let mut checks = Checks::new();
    if plate_solve_dir().is_none() {
        eprintln!("skipped: set {PLATE_SOLVE_DIR_VAR}");
        return;
    }
    for (name, offset, _) in D2_ORACLE {
        if !selected(name) {
            continue;
        }
        let Some(case) = c1_case(&mut checks, name) else { continue };
        let faint = case.catalogue.rows.iter().filter_map(|r| r.g).fold(f64::NEG_INFINITY, f64::max);
        let mut bright = case.catalogue.clone();
        bright.rows.retain(|r| r.g.is_some_and(|g| g < D2_G_LIMIT));
        let epoch = case.frame.epoch(Some(D1_EPOCH));
        let options = TieOptions { strict_sepsi: true, ..case.frame.options(ModelChoice::Fixed(TieModel::Shift)) };
        let tie = run_tie(&case.frame, &case.header_input, &case.frame.sources.detections, &bright, epoch, &options);
        emit("tie", summary("D2 header shift strict G<19 1990", &case.frame, &case.header_input, &epoch, &tie));
        checks.check(&format!("D2 {name} cone complete to G 19"), faint >= D2_G_LIMIT, format!("faintest G {faint:.2}"));
        if !tie.applied() {
            checks.check(&format!("D2 {name} applied"), false, format!("{:?} {:?}", tie.result.decision, tie.result.reason));
            continue;
        }
        let (shift, sigma) = shift_axes(&tie);
        for axis in 0..2 {
            let expected = -offset[axis];
            let off = shift[axis] - expected;
            checks.check(
                &format!("D2 {name} correction axis {axis} = minus oracle offset within 50 mas"),
                off.abs() <= D2_TOLERANCE_MAS,
                format!("{:.1} +/- {:.1} vs {expected:.1}: {off:+.1} mas", shift[axis], sigma[axis]),
            );
        }
    }
    checks.finish();
}

fn injected_cards(cards: &[(String, String)], size: (usize, usize), shift: [f64; 2], rotation_deg: f64, scale: f64) -> Vec<(String, String)> {
    let wcs = wcs_of(cards, size);
    let (_, _, crval1, crval2, cd, _) = wcs.raw_params();
    let inverse = SkyCorrection::from_parameters(unit_vector(crval1, crval2), [-shift[0], -shift[1]], -rotation_deg, 1.0 / scale);
    let update = header_update(&inverse, cd);
    merged_cards(cards, &update.cards, &update.remove)
}

fn shared_fraction(a: &Tie, b: &Tie) -> f64 {
    let ids = |t: &Tie| -> std::collections::HashSet<String> { t.result.report.stars.iter().filter(|s| s.used).map(|s| s.id.clone()).collect() };
    let (sa, sb) = (ids(a), ids(b));
    let shared = sa.intersection(&sb).count();
    shared as f64 / sa.len().max(sb.len()).max(1) as f64
}

struct Injection {
    model: ModelChoice,
    rotation_deg: f64,
    scale: f64,
    input_already_tied: bool,
}

fn injection_case(checks: &mut Checks, label: &str, frame: &Frame, first_input: &InputWcs, catalogue: &GaiaCatalogue, epoch: TieEpochChoice, injection: Injection) {
    let first = run_tie(frame, first_input, &frame.sources.detections, catalogue, epoch, &frame.options(injection.model));
    emit("tie", summary(&format!("{label} first tie"), frame, first_input, &epoch, &first));
    let (cards, first_wcs) = match (first.cards.clone(), first.tied.as_ref()) {
        (Some(cards), Some(wcs)) => (cards, wcs),
        _ if injection.input_already_tied => (first_input.cards.clone(), &first_input.wcs),
        _ => {
            checks.check(&format!("{label} first tie applied"), false, format!("{:?}", first.result.reason));
            return;
        }
    };
    let injected = injected_cards(&cards, frame.size, D3_SHIFT_ARCSEC, injection.rotation_deg, injection.scale);
    let injected_input = frame.input(Some(&injected)).unwrap();
    let displaced = grid_separations(frame.size, &|x, y| world(first_wcs, x, y), &|x, y| world(&injected_input.wcs, x, y));
    let second = run_tie(frame, &injected_input, &frame.sources.detections, catalogue, epoch, &frame.options(injection.model));
    emit("tie", summary(&format!("{label} re-tie after injection"), frame, &injected_input, &epoch, &second));
    let Some(second_wcs) = second.tied.as_ref() else {
        checks.check(&format!("{label} re-tie applied"), false, format!("{:?}", second.result.reason));
        return;
    };
    let seps = grid_separations(frame.size, &|x, y| world(first_wcs, x, y), &|x, y| world(second_wcs, x, y));
    let shared = shared_fraction(&first, &second);
    emit("grid", json!({ "label": format!("{label} re-tied vs first tied"), "file": frame.name, "injected_displacement_mas": stats(&displaced), "sep_mas": stats(&seps), "shared_fraction": shared, "recovered": second.result.report.params }));
    checks.check(&format!("{label} re-tied grid within 10 mas of the first"), max_of(&seps) <= D3_MAX_MAS, format!("max {:.3} mas, shared {:.3}", max_of(&seps), shared));
}

#[test]
#[ignore]
fn live_c21_d3_injection_on_real_pairs() {
    let mut checks = Checks::new();
    if selected("fov0.25_mid1") {
        if let Some(case) = c1_case(&mut checks, "fov0.25_mid1") {
            let epoch = case.frame.epoch(Some(D1_EPOCH));
            injection_case(&mut checks, "D3 DSS2 fov0.25_mid1 general", &case.frame, &case.header_input, &case.catalogue, epoch, Injection { model: ModelChoice::Auto, rotation_deg: D3_ROTATION_DEG, scale: D3_SCALE, input_already_tied: false });
            injection_case(&mut checks, "D3 DSS2 fov0.25_mid1 shift", &case.frame, &case.header_input, &case.catalogue, epoch, Injection { model: ModelChoice::Fixed(TieModel::Shift), rotation_deg: 0.0, scale: 1.0, input_already_tied: false });
        }
    }
    if selected(FGS_D1) {
        if let Some(d1) = mast_frame(FGS_D1) {
            let gwcs = d1.input(None).unwrap();
            if let Ok((_, catalogue)) = catalogue_for(&d1, &gwcs) {
                let epoch = d1.epoch(None);
                let first = run_tie(&d1, &gwcs, &d1.sources.detections, &catalogue, epoch, &d1.options(ModelChoice::Auto));
                if let Some(cards) = first.cards.clone() {
                    let refit_input = d1.input(Some(&cards)).unwrap();
                    injection_case(&mut checks, "D3 FGS refit cards rshift", &d1, &refit_input, &catalogue, epoch, Injection { model: ModelChoice::Auto, rotation_deg: D3_ROTATION_DEG, scale: 1.0, input_already_tied: true });
                }
            }
        }
    }
    checks.finish();
}

fn split_catalogue(catalogue: &GaiaCatalogue, seed: u64) -> (GaiaCatalogue, GaiaCatalogue) {
    let mut order: Vec<usize> = (0..catalogue.rows.len()).collect();
    order.shuffle(&mut StdRng::seed_from_u64(seed));
    let first: std::collections::HashSet<usize> = order.iter().take(order.len() / 2).cloned().collect();
    let mut a = catalogue.clone();
    let mut b = catalogue.clone();
    a.rows = catalogue.rows.iter().enumerate().filter(|(i, _)| first.contains(i)).map(|(_, r)| r.clone()).collect();
    b.rows = catalogue.rows.iter().enumerate().filter(|(i, _)| !first.contains(i)).map(|(_, r)| r.clone()).collect();
    (a, b)
}

fn split_halves(detections: &[TieDetection], seed: u64) -> (Vec<TieDetection>, Vec<TieDetection>) {
    let mut usable: Vec<usize> = (0..detections.len()).filter(|&i| !detections[i].saturated).collect();
    usable.shuffle(&mut StdRng::seed_from_u64(seed));
    let half: std::collections::HashSet<usize> = usable.iter().take(usable.len() / 2).cloned().collect();
    let mut a = Vec::new();
    let mut b = Vec::new();
    for (i, d) in detections.iter().enumerate() {
        if d.saturated {
            a.push(*d);
            b.push(*d);
        } else if half.contains(&i) {
            a.push(*d);
        } else {
            b.push(*d);
        }
    }
    (a, b)
}

#[test]
#[ignore]
fn live_c21_d4_split_halves() {
    let mut checks = Checks::new();
    if plate_solve_dir().is_none() {
        eprintln!("skipped: set {PLATE_SOLVE_DIR_VAR}");
        return;
    }
    for name in ["fov0.5_mid2", "native_mid1", "fov0.25_mid1"] {
        if !selected(name) {
            continue;
        }
        let Some(case) = c1_case(&mut checks, name) else { continue };
        let epoch = case.frame.epoch(Some(D1_EPOCH));
        let options = case.frame.options(ModelChoice::Auto);
        let (a, b) = split_halves(&case.frame.sources.detections, D4_SEED);
        let ta = run_tie(&case.frame, &case.header_input, &a, &case.catalogue, epoch, &options);
        let tb = run_tie(&case.frame, &case.header_input, &b, &case.catalogue, epoch, &options);
        emit("tie", summary("D4 detection half A", &case.frame, &case.header_input, &epoch, &ta));
        emit("tie", summary("D4 detection half B", &case.frame, &case.header_input, &epoch, &tb));
        emit("d4", json!({ "label": "D4 detection halves share Gaia stars", "file": case.frame.name, "shared_fraction": shared_fraction(&ta, &tb) }));
        let (gaia_a, gaia_b) = split_catalogue(&case.catalogue, D4_SEED);
        let ta = run_tie(&case.frame, &case.header_input, &case.frame.sources.detections, &gaia_a, epoch, &options);
        let tb = run_tie(&case.frame, &case.header_input, &case.frame.sources.detections, &gaia_b, epoch, &options);
        emit("tie", summary("D4 half A", &case.frame, &case.header_input, &epoch, &ta));
        emit("tie", summary("D4 half B", &case.frame, &case.header_input, &epoch, &tb));
        let (Some(wa), Some(wb)) = (ta.tied.as_ref(), tb.tied.as_ref()) else {
            checks.check(&format!("D4 {name} both halves applied"), false, format!("{:?} / {:?}", ta.result.reason, tb.result.reason));
            continue;
        };
        let pixels = grid_pixels(case.frame.size);
        let seps: Vec<f64> = pixels.iter().map(|&(x, y)| match (world(wa, x, y), world(wb, x, y)) {
            (Some(p), Some(q)) => sep_mas(p, q),
            _ => f64::NAN,
        }).collect();
        let (ra, rb) = (&ta.result.report, &tb.result.report);
        let corner_sigma = ra.uncertainty_corner_mas.hypot(rb.uncertainty_corner_mas);
        let centre_sigma = ra.uncertainty_centre_mas.hypot(rb.uncertainty_centre_mas);
        let (cx, cy) = crate::core::astrometry::wcs::pixel_center(case.frame.size.0, case.frame.size.1);
        let centre_sep = match (world(wa, cx, cy), world(wb, cx, cy)) {
            (Some(p), Some(q)) => sep_mas(p, q),
            _ => f64::NAN,
        };
        let shared = shared_fraction(&ta, &tb);
        emit("grid", json!({
            "label": "D4 half A vs half B",
            "file": case.frame.name,
            "sep_mas": stats(&seps),
            "centre_sep_mas": centre_sep,
            "sigma_centre_pair_mas": centre_sigma,
            "sigma_corner_pair_mas": corner_sigma,
            "max_ratio_to_corner_sigma": max_of(&seps) / corner_sigma,
            "centre_ratio": centre_sep / centre_sigma,
            "shared_fraction": shared,
        }));
        checks.check(&format!("D4 {name} grid within 3 sqrt2 sigma (corner sigma)"), max_of(&seps) <= D4_SIGMAS * corner_sigma, format!("max {:.1} vs {:.1} mas", max_of(&seps), D4_SIGMAS * corner_sigma));
        checks.check(&format!("D4 {name} centre within 3 sqrt2 sigma (centre sigma)"), centre_sep <= D4_SIGMAS * centre_sigma, format!("{centre_sep:.1} vs {:.1} mas", D4_SIGMAS * centre_sigma));
        checks.check(&format!("D4 {name} Gaia halves share no stars"), shared == 0.0, format!("{shared:.3}"));
    }
    checks.finish();
}

fn sources_to_depth(frame: &Frame, unsaturated_target: usize) -> TieSources {
    let entry = load_cached_full(&frame.path).unwrap();
    let image = entry.arr();
    let dims = image.dim();
    let planes = photometry_planes(&frame.path, dims);
    let dq = resolve_dq_mask(&frame.path, true, dims);
    let saturated = unusable_saturation(&frame.path, dims);
    let stars = detect_stars(image, TIE_DETECTION_SIGMA).stars;
    let n_detected = stars.len();
    let (level, level_source) = saturation_level(Some(&frame.header), entry.stats().max);
    let config = PhotometryConfig { saturation: Some((level, level_source)), ..PhotometryConfig::default() };
    let excluded_at = |x: f64, y: f64| {
        dq.as_ref().is_some_and(|mask| {
            let (r, c) = (y.round(), x.round());
            r >= 0.0 && c >= 0.0 && mask.map.get((r as usize, c as usize)).is_some_and(|&v| v != 0)
        })
    };
    let prepared = MaskedImage::new(image, dq.as_ref().map(|mask| &mask.map)).or_else(|_| MaskedImage::new(image, None));
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
    let (detections, fwhm_outliers) = flag_fwhm_outliers(collect_detections(measured, unsaturated_target, 5 * unsaturated_target));
    TieSources { detections, n_detected, n_dq_rejected, n_unmeasured, dq_mask_error: None, fwhm_outliers }
}

#[test]
#[ignore]
fn live_c21_fgs_depth_and_model_experiment() {
    let mut checks = Checks::new();
    let Some(d1) = mast_frame(FGS_D1) else { return };
    let Some(d2) = mast_frame(FGS_D2) else { return };
    let in1 = d1.input(None).unwrap();
    let in2 = d2.input(None).unwrap();
    let (_, cat1) = catalogue_for(&d1, &in1).unwrap();
    let (_, cat2) = catalogue_for(&d2, &in2).unwrap();
    let (e1, e2) = (d1.epoch(None), d2.epoch(None));
    let models = [ModelChoice::Auto, ModelChoice::Fixed(TieModel::Shift)];
    let mut references: Vec<(Tie, Tie)> = Vec::new();
    for target in [TIE_MAX_DETECTIONS, 3 * TIE_MAX_DETECTIONS, 6 * TIE_MAX_DETECTIONS] {
        let s1 = if target == TIE_MAX_DETECTIONS { None } else { Some(sources_to_depth(&d1, target)) };
        let s2 = if target == TIE_MAX_DETECTIONS { None } else { Some(sources_to_depth(&d2, target)) };
        let det1 = s1.as_ref().map_or(&d1.sources.detections, |s| &s.detections);
        let det2 = s2.as_ref().map_or(&d2.sources.detections, |s| &s.detections);
        emit("depth", json!({ "target": target, "kept": [det1.len(), det2.len()], "usable": [det1.iter().filter(|d| !d.saturated).count(), det2.iter().filter(|d| !d.saturated).count()] }));
        for (m, model) in models.into_iter().enumerate() {
            let options1 = TieOptions { max_detections: target, ..d1.options(model) };
            let options2 = TieOptions { max_detections: target, ..d2.options(model) };
            let t1 = run_tie(&d1, &in1, det1, &cat1, e1, &options1);
            let t2 = run_tie(&d2, &in2, det2, &cat2, e2, &options2);
            let label = format!("EXP depth {target} model {model:?}");
            emit("tie", summary(&format!("{label} dither 1"), &d1, &in1, &e1, &t1));
            emit("tie", summary(&format!("{label} dither 2"), &d2, &in2, &e2, &t2));
            if target > TIE_MAX_DETECTIONS {
                let (r1, r2) = &references[m];
                deep_tie_check(&mut checks, &format!("{label} dither 1"), &d1, &in1, r1, &t1);
                deep_tie_check(&mut checks, &format!("{label} dither 2"), &d2, &in2, r2, &t2);
                continue;
            }
            if let (Some(c1), Some(c2)) = (t1.correction(), t2.correction()) {
                let seps = j3_sigma_checks(&mut checks, &format!("{label} J3"), &d1, &in1, (&t1, &t2), (c1, c2));
                let tied1 = |x: f64, y: f64| c1.corrected_pixel_to_world(&in1.wcs, x, y);
                let tied2 = |x: f64, y: f64| c2.corrected_pixel_to_world(&in2.wcs, x, y);
                let common = common_offsets(
                    &sky_points(&d1.sources.detections, &tied1, COMMON_MIN_SNR),
                    &sky_points(&d2.sources.detections, &tied2, COMMON_MIN_SNR),
                    COMMON_RADIUS_MAS,
                );
                emit("grid", json!({ "label": format!("{label} J3 correction dither 1 vs dither 2"), "sep_mas": stats(&seps), "common": offsets_summary(&common), "within_10_mas_everywhere": max_of(&seps) <= J3_MAX_MAS }));
            } else {
                checks.check(&format!("{label} both applied"), false, format!("{:?} / {:?}", t1.result.reason, t2.result.reason));
            }
            references.push((t1, t2));
        }
    }
    if let Some(m16) = Frame::open(&real_data_dir().join("jw02739001001_02105_00001_nrca1_cal.fits")) {
        let gwcs = m16.input(None).unwrap();
        let header = m16.input(Some(&header_wcs_cards(&m16.header))).unwrap();
        let untied = grid_separations(m16.size, &|x, y| world(&gwcs.wcs, x, y), &|x, y| world(&header.wcs, x, y));
        emit("grid", json!({ "label": "J2 M16 nrca1 untied gwcs vs header SIP", "sep_mas": stats(&untied) }));
        if let Ok((_, catalogue)) = catalogue_for(&m16, &gwcs) {
            let epoch = m16.epoch(None);
            for model in [ModelChoice::Auto, ModelChoice::Fixed(TieModel::Shift)] {
                let tg = run_tie(&m16, &gwcs, &m16.sources.detections, &catalogue, epoch, &m16.options(model));
                let th = run_tie(&m16, &header, &m16.sources.detections, &catalogue, epoch, &m16.options(model));
                emit("tie", summary(&format!("J2 M16 nrca1 gwcs {model:?}"), &m16, &gwcs, &epoch, &tg));
                emit("tie", summary(&format!("J2 M16 nrca1 header SIP {model:?}"), &m16, &header, &epoch, &th));
                match_audit(&format!("J2 M16 nrca1 gwcs {model:?}"), &m16, &gwcs, &catalogue, &tg);
                if let (Some(a), Some(b)) = (tg.tied.as_ref(), th.tied.as_ref()) {
                    let tied = grid_separations(m16.size, &|x, y| world(a, x, y), &|x, y| world(b, x, y));
                    emit("grid", json!({ "label": format!("J2 M16 nrca1 tied gwcs refit vs tied header SIP {model:?}"), "sep_mas": stats(&tied) }));
                    checks.check(&format!("J2 M16 nrca1 {model:?} tied grids within 7 mas"), max_of(&tied) <= J2_MAX_MAS, format!("max {:.2} mas", max_of(&tied)));
                }
            }
        }
    }
    checks.finish();
}

fn shifted(p: (f64, f64), offset_arcsec: [f64; 2]) -> (f64, f64) {
    (p.0 + offset_arcsec[0] / ARCSEC_PER_DEG / p.1.to_radians().cos(), p.1 + offset_arcsec[1] / ARCSEC_PER_DEG)
}

fn good_gaia_at(catalogue: &GaiaCatalogue, epoch_year: f64) -> Vec<(f64, f64)> {
    let dt = epoch_year - GAIA_EPOCH;
    catalogue
        .rows
        .iter()
        .filter(|r| good_row(r))
        .map(|r| propagate_linear(r.ra_deg, r.dec_deg, r.pm_ra_masyr.unwrap_or(0.0), r.pm_dec_masyr.unwrap_or(0.0), dt))
        .collect()
}

fn pairs_at_offset(points: &[(f64, f64)], gaia: &[(f64, f64)], offset_arcsec: [f64; 2], radius_mas: f64) -> usize {
    points
        .iter()
        .filter(|&&p| {
            let q = shifted(p, offset_arcsec);
            gaia.iter().any(|&g| (g.1 - q.1).abs() * MAS_PER_DEG <= radius_mas && sep_mas(q, g) <= radius_mas)
        })
        .count()
}

#[test]
#[ignore]
fn live_c21_crf_false_peak_diagnostic() {
    let mut checks = Checks::new();
    let Some(cal) = mast_frame(FGS_D1) else { return };
    let Some(crf) = mast_frame("jw01476003004_02201_00001_guider2_o003_crf.fits") else { return };
    let in_cal = cal.input(None).unwrap();
    let in_crf = crf.input(None).unwrap();
    let (_, catalogue) = catalogue_for(&cal, &in_cal).unwrap();
    let epoch = cal.epoch(None);
    let t_cal = run_tie(&cal, &in_cal, &cal.sources.detections, &catalogue, epoch, &cal.options(ModelChoice::Auto));
    let t_crf = run_tie(&crf, &in_crf, &crf.sources.detections, &catalogue, epoch, &crf.options(ModelChoice::Auto));
    emit("tie", summary("CRF cal reference", &cal, &in_cal, &epoch, &t_cal));
    emit("tie", summary("CRF crf", &crf, &in_crf, &epoch, &t_crf));
    let untied = grid_separations(cal.size, &|x, y| world(&in_cal.wcs, x, y), &|x, y| world(&in_crf.wcs, x, y));
    emit("grid", json!({ "label": "CRF untied cal vs crf WCS", "sep_mas": stats(&untied) }));
    let gaia = good_gaia_at(&catalogue, t_cal.result.report.epoch_year);
    let true_offset = t_cal.result.report.peak_offset_arcsec;
    let false_offset = t_crf.result.report.peak_offset_arcsec;
    for (label, frame, input) in [("cal", &cal, &in_cal), ("crf", &crf, &in_crf)] {
        let at = |x: f64, y: f64| world(&input.wcs, x, y);
        let points = sky_points(&frame.sources.detections, &at, 0.0);
        let mut control = Vec::new();
        for k in 0..8 {
            let angle = std::f64::consts::PI * k as f64 / 4.0;
            control.push(pairs_at_offset(&points, &gaia, [true_offset[0] + 3.0 * angle.cos(), true_offset[1] + 3.0 * angle.sin()], 100.0));
        }
        emit("crf", json!({
            "frame": label,
            "usable": points.len(),
            "pairs_within_100mas_at_true_offset": pairs_at_offset(&points, &gaia, true_offset, 100.0),
            "pairs_within_100mas_at_false_offset": pairs_at_offset(&points, &gaia, false_offset, 100.0),
            "pairs_within_100mas_at_8_control_offsets_3arcsec_away": control,
            "true_offset_arcsec": true_offset,
            "false_offset_arcsec": false_offset,
        }));
    }
    let cal_points = sky_points(&cal.sources.detections, &|x, y| world(&in_cal.wcs, x, y), 0.0);
    let crf_points = sky_points(&crf.sources.detections, &|x, y| world(&in_crf.wcs, x, y), 0.0);
    let shared = common_offsets(&crf_points, &cal_points, 100.0);
    emit("crf", json!({ "label": "usable detections shared by crf and cal within 100 mas", "n": shared.len(), "crf_usable": crf_points.len(), "cal_usable": cal_points.len() }));
    let r = &t_crf.result.report;
    emit("crf", json!({ "label": "expected chance pairs in the crf tie", "detections_used": r.cuts.detections_used, "expected_chance_pairs": r.expected_chance_pairs, "n_matched": r.n_matched, "n_used": r.n_used }));
    match (t_cal.correction(), t_crf.correction()) {
        (Some(a), Some(b)) => {
            let seps = correction_separations(&grid_sky(&cal, &in_cal.wcs), a, b);
            emit("grid", json!({ "label": "CRF correction cal vs crf", "sep_mas": stats(&seps) }));
            checks.check("CRF crf tie agrees with the cal tie within 10 mas", max_of(&seps) <= J3_MAX_MAS, format!("max {:.1} mas", max_of(&seps)));
        }
        (Some(_), None) => checks.check("CRF crf tie refused by a chance gate while the cal tie applied", refused_by_a_chance_gate(&t_crf), format!("{:?} {:?}", t_crf.result.decision, t_crf.result.reason)),
        _ => checks.check("CRF cal applied", false, format!("{:?} / {:?}", t_cal.result.reason, t_crf.result.reason)),
    }
    checks.finish();
}

#[test]
#[ignore]
fn live_c21_j6_crf_tie_is_refused_or_agrees_with_the_cal() {
    let mut checks = Checks::new();
    let Some(cal) = mast_frame(FGS_D1) else { return };
    let Some(crf) = mast_frame(FGS_D1_CRF) else { return };
    let in_cal = cal.input(None).unwrap();
    let in_crf = crf.input(None).unwrap();
    let (_, catalogue) = catalogue_for(&cal, &in_cal).unwrap();
    let epoch = cal.epoch(None);
    let t_cal = run_tie(&cal, &in_cal, &cal.sources.detections, &catalogue, epoch, &cal.options(ModelChoice::Auto));
    let t_crf = run_tie(&crf, &in_crf, &crf.sources.detections, &catalogue, epoch, &crf.options(ModelChoice::Auto));
    emit("tie", summary("J6 cal reference", &cal, &in_cal, &epoch, &t_cal));
    emit("tie", summary("J6 crf", &crf, &in_crf, &epoch, &t_crf));
    checks.check("J6 cal tie applied", t_cal.applied(), format!("{:?} {:?}", t_cal.result.decision, t_cal.result.reason));
    let name = "J6 crf refused by a chance gate, or applied within 10 mas of the cal on the cal grid";
    match (t_cal.correction(), t_crf.correction()) {
        (Some(c_cal), Some(c_crf)) => {
            let seps = correction_separations(&grid_sky(&cal, &in_cal.wcs), c_cal, c_crf);
            emit("grid", json!({ "label": "J6 correction cal vs crf", "sep_mas": stats(&seps) }));
            checks.check(name, max_of(&seps) <= J3_MAX_MAS, format!("max {:.1} mas", max_of(&seps)));
        }
        _ => checks.check(name, refused_by_a_chance_gate(&t_crf), format!("crf {:?} {:?}", t_crf.result.decision, t_crf.result.reason)),
    }
    checks.finish();
}

#[test]
#[ignore]
fn live_c21_j6_deep_detection_lists_are_refused_or_agree_with_the_default_depth() {
    let mut checks = Checks::new();
    for name in [FGS_D1, FGS_D2] {
        let Some(frame) = mast_frame(name) else { continue };
        let input = frame.input(None).unwrap();
        let (_, catalogue) = catalogue_for(&frame, &input).unwrap();
        let epoch = frame.epoch(None);
        let reference = run_tie(&frame, &input, &frame.sources.detections, &catalogue, epoch, &frame.options(ModelChoice::Auto));
        emit("tie", summary(&format!("J6 depth {TIE_MAX_DETECTIONS} reference"), &frame, &input, &epoch, &reference));
        checks.check(&format!("J6 {name} reference applied"), reference.applied(), format!("{:?} {:?}", reference.result.decision, reference.result.reason));
        for target in [3 * TIE_MAX_DETECTIONS, 6 * TIE_MAX_DETECTIONS] {
            let sources = sources_to_depth(&frame, target);
            let options = TieOptions { max_detections: target, ..frame.options(ModelChoice::Auto) };
            let tie = run_tie(&frame, &input, &sources.detections, &catalogue, epoch, &options);
            emit("tie", summary(&format!("J6 depth {target}"), &frame, &input, &epoch, &tie));
            deep_tie_check(&mut checks, &format!("J6 {name} depth {target}"), &frame, &input, &reference, &tie);
        }
    }
    checks.finish();
}

fn tied_product(path: &Path) -> Option<(Frame, InputWcs, Tie)> {
    let frame = Frame::open(path)?;
    let input = frame.input(None).ok()?;
    let (_, catalogue) = catalogue_for(&frame, &input).ok()?;
    let epoch = frame.epoch(None);
    let tie = run_tie(&frame, &input, &frame.sources.detections, &catalogue, epoch, &frame.options(ModelChoice::Auto));
    emit("tie", summary("XP product", &frame, &input, &epoch, &tie));
    Some((frame, input, tie))
}

#[test]
#[ignore]
fn live_c21_cross_product_consistency() {
    let mut checks = Checks::new();
    let Some(dir) = plate_solve_dir() else { return };
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("c3").join("manifest.json")).unwrap()).unwrap();
    let path_of = |needle: &str| -> Option<PathBuf> {
        manifest["frames"].as_array()?.iter().filter_map(|f| f["file"].as_str()).find(|f| f.contains(needle)).map(PathBuf::from)
    };
    let pairs = [
        ("jw06565-o002_t001_nircam_clear-f444w_i2d", "jw06565-o003_t001_nircam_clear-f200w_i2d"),
        ("jw06565-o002_t001_nircam_clear-f444w_i2d", "jw06565-o001_t001_miri_f770w_i2d"),
        ("jw02739-o001_t001_nircam_clear-f444w_i2d", "jw02739002001_02101_00001_mirimage_cal"),
    ];
    let mut cache: Vec<(String, Frame, InputWcs, Tie)> = Vec::new();
    for (a, b) in pairs {
        if !selected(a) && !selected(b) {
            continue;
        }
        for name in [a, b] {
            if cache.iter().any(|(n, ..)| n == name) {
                continue;
            }
            let Some(path) = path_of(name) else { continue };
            if let Some((frame, input, tie)) = tied_product(&path) {
                cache.push((name.to_string(), frame, input, tie));
            }
        }
        let find = |name: &str| cache.iter().find(|(n, ..)| n == name);
        let (Some((_, fa, ia, ta)), Some((_, fb, ib, tb))) = (find(a), find(b)) else {
            checks.check(&format!("XP {a} vs {b} both loaded"), false, String::new());
            continue;
        };
        let (Some(ca), Some(cb)) = (ta.correction(), tb.correction()) else {
            checks.check(&format!("XP {a} vs {b} both applied"), false, format!("{:?} / {:?}", ta.result.reason, tb.result.reason));
            continue;
        };
        for min_snr in [30.0, 100.0] {
            let raw_a = sky_points(&fa.sources.detections, &|x, y| world(&ia.wcs, x, y), min_snr);
            let raw_b = sky_points(&fb.sources.detections, &|x, y| world(&ib.wcs, x, y), min_snr);
            let tied_a = sky_points(&fa.sources.detections, &|x, y| ca.corrected_pixel_to_world(&ia.wcs, x, y), min_snr);
            let tied_b = sky_points(&fb.sources.detections, &|x, y| cb.corrected_pixel_to_world(&ib.wcs, x, y), min_snr);
            let before = common_offsets(&raw_a, &raw_b, COMMON_RADIUS_MAS);
            let after = common_offsets(&tied_a, &tied_b, COMMON_RADIUS_MAS);
            let after_summary = offsets_summary(&after);
            emit("common", json!({ "label": format!("XP {b} minus {a}"), "min_snr": min_snr, "untied": offsets_summary(&before), "tied": after_summary }));
            if min_snr == 30.0 {
                let v = &after_summary["median_vector"];
                let norm = v[0].as_f64().unwrap_or(f64::NAN).hypot(v[1].as_f64().unwrap_or(f64::NAN));
                checks.check(&format!("XP {b} vs {a} tied median offset <= 10 mas"), norm <= J3_MAX_MAS, format!("{norm:.2} mas over {} sources", after.len()));
            }
        }
    }
    checks.finish();
}
