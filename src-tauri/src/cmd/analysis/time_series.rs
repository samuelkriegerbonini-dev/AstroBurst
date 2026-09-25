use std::path::Path;
use std::time::Instant;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use super::{
    annulus_arg, aperture_radius_arg, apply_calibration, check_annulus_clears_aperture, photometry_planes,
    processed_data_warning, resolve_dq_mask,
};
use crate::cmd::common::{blocking_cmd, load_cached, load_cached_full};
use crate::core::alignment::pair::offset_within_limits;
use crate::core::alignment::phase_correlation::{is_low_confidence, phase_correlate};
use crate::core::analysis::photometry::{
    measure_star_prepared, saturation_level, MaskedImage, PhotometryConfig, StarPhotometry,
};
use crate::core::astrometry::spectral::mid_exposure_jd;
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::metadata::photcal::PhotCal;
use crate::infra::cache::ImageEntry;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::EVENT_TIME_SERIES_PROGRESS;
use crate::types::error::AppError;
use crate::types::header::HduHeader;

pub const MAX_TIME_SERIES_FRAMES: usize = 2000;
pub const MAX_TIME_SERIES_TARGETS: usize = 64;
const TRACKED_SEARCH_RADIUS_PX: usize = 3;
const MAX_REFERENCE_OFFSET_PX: f64 = 3.0;
const MAX_TRACKED_OFFSET_PX: f64 = 1.5;
const SKY_ONLY_ERRORS_WARNING: &str = "flux errors leave out source photon noise: no gain was given and the frames carry no ERR plane; enter the gain in e-/ADU to include it";
const ROLE_TARGET: &str = "target";
const ROLE_COMP: &str = "comp";
const ROLE_CHECK: &str = "check";
const ROLE_IGNORE: &str = "ignore";
const ROLE_NAMES: [&str; 4] = [ROLE_TARGET, ROLE_COMP, ROLE_CHECK, ROLE_IGNORE];
const EXPTIME_KEY: &str = "EXPTIME";
const AIRMASS_KEY: &str = "AIRMASS";
const FILTER_KEY: &str = "FILTER";
const TIMESYS_KEY: &str = "TIMESYS";
const UTC_SCALE: &str = "UTC";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TimeSeriesTarget {
    pub x: f64,
    pub y: f64,
    pub label: String,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct FrameOffset {
    pub dx: f64,
    pub dy: f64,
    pub confidence: f64,
    pub registered: bool,
}

#[derive(Debug, Serialize)]
pub struct TimeSeriesFrame {
    pub index: usize,
    pub path: String,
    pub file_name: String,
    pub jd_mid: Option<f64>,
    pub time_source: Option<String>,
    pub exptime: Option<f64>,
    pub filter: Option<String>,
    pub airmass: Option<f64>,
    pub offset: Option<FrameOffset>,
    pub photcal_label: Option<String>,
    pub targets: Vec<Option<StarPhotometry>>,
    pub errors: Vec<Option<String>>,
    pub skipped: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TimeSeriesResult {
    pub reference_path: String,
    pub targets: Vec<TimeSeriesTarget>,
    pub frames: Vec<TimeSeriesFrame>,
    pub n_frames: usize,
    pub n_skipped: usize,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

fn file_name_of(path: &str) -> String {
    let without_fragment = path.split('#').next().unwrap_or(path);
    Path::new(without_fragment)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn header_string(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|v| v.trim().trim_matches('\'').trim().to_string())
        .filter(|v| !v.is_empty())
}

fn finite_card(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|v| v.is_finite())
}

fn load_entry(path: &str) -> anyhow::Result<ImageEntry> {
    load_cached_full(path).or_else(|_| load_cached(path))
}

fn check_cancelled(progress: Option<&ProgressHandle>) -> anyhow::Result<()> {
    if progress.is_some_and(|p| p.is_cancelled()) {
        return Err(AppError::Cancelled.into());
    }
    Ok(())
}

fn skipped_frame(index: usize, path: &str, n_targets: usize, reason: String) -> TimeSeriesFrame {
    TimeSeriesFrame {
        index,
        path: path.to_string(),
        file_name: file_name_of(path),
        jd_mid: None,
        time_source: None,
        exptime: None,
        filter: None,
        airmass: None,
        offset: None,
        photcal_label: None,
        targets: (0..n_targets).map(|_| None).collect(),
        errors: vec![None; n_targets],
        skipped: Some(reason),
    }
}

fn dimension_mismatch(frame: (usize, usize), reference: (usize, usize)) -> String {
    format!(
        "dimensions {} x {} differ from the reference {} x {}",
        frame.1, frame.0, reference.1, reference.0
    )
}

fn filter_mismatch(entry: &ImageEntry, reference: &ImageEntry) -> Option<String> {
    let frame = entry.header().and_then(|h| header_string(h, FILTER_KEY))?;
    let expected = reference.header().and_then(|h| header_string(h, FILTER_KEY))?;
    (frame != expected).then(|| format!("filter {frame} differs from the reference filter {expected}"))
}

fn time_source_with_scale(header: &HduHeader, source: String) -> String {
    match header_string(header, TIMESYS_KEY) {
        None => format!("{source}; UTC assumed (no TIMESYS)"),
        Some(scale) if scale.eq_ignore_ascii_case(UTC_SCALE) => format!("{source}; time scale {scale} (TIMESYS)"),
        Some(scale) => format!("{source}; time scale {scale} (TIMESYS), not converted to UTC"),
    }
}

struct FrameShift {
    offset: Option<FrameOffset>,
    dx: f64,
    dy: f64,
}

fn track_frame(reference: &ImageEntry, entry: &ImageEntry, file_name: &str, warnings: &mut Vec<String>) -> FrameShift {
    let (rows, cols) = entry.arr().dim();
    let r = phase_correlate(reference.arr(), entry.arr());
    let registered = offset_within_limits(r.dy, r.dx, rows, cols) && !is_low_confidence(r.confidence);
    if !registered {
        warnings.push(format!(
            "{file_name}: drift offset ({:.2}, {:.2}) rejected at confidence {:.1}, measured unshifted",
            r.dx, r.dy, r.confidence
        ));
    }
    let (dx, dy) = if registered { (r.dx, r.dy) } else { (0.0, 0.0) };
    FrameShift {
        offset: Some(FrameOffset { dx: r.dx, dy: r.dy, confidence: r.confidence, registered }),
        dx,
        dy,
    }
}

fn reference_anchors(
    targets: &[TimeSeriesTarget],
    reference_frame: &TimeSeriesFrame,
    later_frames_checked: bool,
    warnings: &mut Vec<String>,
) -> Vec<Option<(f64, f64)>> {
    targets
        .iter()
        .zip(&reference_frame.targets)
        .map(|(target, measured)| {
            let Some(phot) = measured else {
                if later_frames_checked && target.role != ROLE_IGNORE {
                    warnings.push(format!(
                        "{}: not measured on the reference frame, so later frames re-find it around the placed point ({:.1}, {:.1}) without an identity check",
                        target.label, target.x, target.y
                    ));
                }
                return None;
            };
            let distance = (phot.x - target.x).hypot(phot.y - target.y);
            if distance > MAX_REFERENCE_OFFSET_PX {
                warnings.push(format!(
                    "{}: the reference centroid ({:.1}, {:.1}) is {distance:.1} px from the placed point ({:.1}, {:.1}); the peak search may have locked onto a brighter neighbour",
                    target.label, phot.x, phot.y, target.x, target.y
                ));
            }
            Some((phot.x, phot.y))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn measure_frame(
    index: usize,
    path: &str,
    entry: &ImageEntry,
    reference: &ImageEntry,
    targets: &[TimeSeriesTarget],
    anchors: &[Option<(f64, f64)>],
    config_base: &PhotometryConfig,
    exclude_dq: bool,
    track_drift: bool,
    warnings: &mut Vec<String>,
) -> TimeSeriesFrame {
    let file_name = file_name_of(path);
    let dims = entry.arr().dim();
    let reference_dims = reference.arr().dim();
    if dims != reference_dims {
        return skipped_frame(index, path, targets.len(), dimension_mismatch(dims, reference_dims));
    }
    if let Some(reason) = filter_mismatch(entry, reference) {
        return skipped_frame(index, path, targets.len(), reason);
    }
    let planes = photometry_planes(path, dims);
    let mask = resolve_dq_mask(path, exclude_dq, dims);
    let header = entry.header();
    if let Some(message) = processed_data_warning(header) {
        if !warnings.contains(&message) {
            warnings.push(message);
        }
    }
    let wcs = header.and_then(|h| WcsTransform::from_header(h).ok());
    let photcal = header.and_then(|h| PhotCal::from_header(h, wcs.as_ref()));
    let config = PhotometryConfig {
        saturation: Some(saturation_level(header, entry.stats().max)),
        ..config_base.clone()
    };
    let tracked_config = PhotometryConfig { search_radius: TRACKED_SEARCH_RADIUS_PX, ..config.clone() };
    let timing = header.and_then(|h| mid_exposure_jd(h).map(|(jd, source)| (jd, time_source_with_scale(h, source))));
    let shift = if track_drift && index > 0 {
        track_frame(reference, entry, &file_name, warnings)
    } else {
        FrameShift { offset: None, dx: 0.0, dy: 0.0 }
    };
    let registered = index > 0 && shift.offset.as_ref().is_some_and(|o| o.registered);
    let source = MaskedImage::new(entry.arr(), mask.as_ref().map(|m| &m.map));

    let mut measured: Vec<Option<StarPhotometry>> = Vec::with_capacity(targets.len());
    let mut errors: Vec<Option<String>> = Vec::with_capacity(targets.len());
    for (i, target) in targets.iter().enumerate() {
        if target.role == ROLE_IGNORE {
            measured.push(None);
            errors.push(None);
            continue;
        }
        let anchor = anchors.get(i).copied().flatten().filter(|_| registered);
        let (expected_x, expected_y, target_config) = match anchor {
            Some((ax, ay)) => (ax + shift.dx, ay + shift.dy, &tracked_config),
            None => (target.x + shift.dx, target.y + shift.dy, &config),
        };
        let outcome = source.as_ref().map_err(String::clone).and_then(|s| {
            measure_star_prepared(
                s,
                planes.err.as_ref().map(|e| e.arr()),
                planes.saturated.as_ref(),
                expected_x,
                expected_y,
                target_config,
            )
        });
        let landed = |phot: &StarPhotometry| (phot.x - expected_x).hypot(phot.y - expected_y);
        match outcome {
            Ok(phot) if anchor.is_some() && landed(&phot) > MAX_TRACKED_OFFSET_PX => {
                measured.push(None);
                errors.push(Some(format!(
                    "centroid ({:.1}, {:.1}) landed {:.1} px from the expected position ({expected_x:.1}, {expected_y:.1}) of the reference star; another source or a cosmic ray is inside the search box",
                    phot.x,
                    phot.y,
                    landed(&phot)
                )));
            }
            Ok(mut phot) => {
                if let Some(cal) = &photcal {
                    apply_calibration(&mut phot, cal);
                }
                phot.growth_curve.clear();
                measured.push(Some(phot));
                errors.push(None);
            }
            Err(e) => {
                measured.push(None);
                errors.push(Some(e));
            }
        }
    }

    TimeSeriesFrame {
        index,
        path: path.to_string(),
        file_name,
        jd_mid: timing.as_ref().map(|(jd, _)| *jd),
        time_source: timing.map(|(_, source)| source),
        exptime: header.and_then(|h| finite_card(h, EXPTIME_KEY)),
        filter: header.and_then(|h| header_string(h, FILTER_KEY)),
        airmass: header.and_then(|h| finite_card(h, AIRMASS_KEY)),
        offset: shift.offset,
        photcal_label: photcal.as_ref().map(|cal| cal.label()),
        targets: measured,
        errors,
        skipped: None,
    }
}

pub(crate) fn measure_time_series(
    paths: &[String],
    targets: &[TimeSeriesTarget],
    config_base: &PhotometryConfig,
    exclude_dq: bool,
    track_drift: bool,
    progress: Option<&ProgressHandle>,
) -> anyhow::Result<TimeSeriesResult> {
    let t0 = Instant::now();
    let Some(reference_path) = paths.first() else {
        bail!("time series needs at least one frame path, got 0");
    };
    let reference = load_entry(reference_path)?;
    let mut warnings: Vec<String> = Vec::new();
    let mut anchors: Vec<Option<(f64, f64)>> = Vec::new();
    let mut frames: Vec<TimeSeriesFrame> = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        check_cancelled(progress)?;
        let loaded = if index == 0 { Ok(reference.clone()) } else { load_entry(path) };
        let frame = match loaded {
            Ok(entry) => measure_frame(
                index,
                path,
                &entry,
                &reference,
                targets,
                &anchors,
                config_base,
                exclude_dq,
                track_drift,
                &mut warnings,
            ),
            Err(e) => skipped_frame(index, path, targets.len(), format!("failed to load: {e:#}")),
        };
        if index == 0 {
            anchors = reference_anchors(targets, &frame, track_drift && paths.len() > 1, &mut warnings);
        }
        let file_name = frame.file_name.clone();
        frames.push(frame);
        if let Some(p) = progress {
            p.tick_with_stage(&file_name);
        }
    }
    let unchecked = frames
        .iter()
        .filter(|f| f.index > 0 && f.skipped.is_none() && !f.offset.as_ref().is_some_and(|o| o.registered))
        .count();
    if unchecked > 0 {
        warnings.push(format!(
            "target identity is not checked on {unchecked} {} where drift is not tracked or its offset was rejected: each star is re-found there by the {} px peak search around its placed point",
            if unchecked == 1 { "frame" } else { "frames" },
            config_base.search_radius
        ));
    }
    let usable_gain = config_base.gain.is_some_and(|g| g.is_finite() && g > 0.0);
    if !usable_gain && frames.iter().flat_map(|f| f.targets.iter().flatten()).any(|t| !t.err_used) {
        warnings.push(SKY_ONLY_ERRORS_WARNING.to_string());
    }
    let n_skipped = frames.iter().filter(|f| f.skipped.is_some()).count();
    Ok(TimeSeriesResult {
        reference_path: reference_path.clone(),
        targets: targets.to_vec(),
        n_frames: frames.len(),
        n_skipped,
        frames,
        warnings,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

pub(crate) fn validate_time_series_args(
    paths: &[String],
    targets: &[TimeSeriesTarget],
    aperture_radius: f64,
    annulus_inner: Option<f64>,
    annulus_outer: Option<f64>,
    gain: Option<f64>,
) -> anyhow::Result<PhotometryConfig> {
    if paths.is_empty() {
        bail!("time series needs at least one frame path, got 0");
    }
    if paths.len() > MAX_TIME_SERIES_FRAMES {
        bail!("time series of {} frames exceeds the limit of {MAX_TIME_SERIES_FRAMES}", paths.len());
    }
    if targets.is_empty() {
        bail!("time series needs at least one target star, got 0");
    }
    if targets.len() > MAX_TIME_SERIES_TARGETS {
        bail!("time series of {} targets exceeds the limit of {MAX_TIME_SERIES_TARGETS}", targets.len());
    }
    for (i, t) in targets.iter().enumerate() {
        if !t.x.is_finite() || !t.y.is_finite() {
            bail!("target {i} ({}) position ({}, {}) must be finite", t.label, t.x, t.y);
        }
        if !ROLE_NAMES.contains(&t.role.as_str()) {
            bail!("target {i} ({}) role '{}' must be one of target, comp, check or ignore", t.label, t.role);
        }
    }
    aperture_radius_arg(Some(aperture_radius))?;
    let sky_annulus = annulus_arg(annulus_inner, annulus_outer)?;
    check_annulus_clears_aperture(Some(aperture_radius), sky_annulus)?;
    if let Some(g) = gain {
        if !g.is_finite() || g <= 0.0 {
            bail!("gain {g} must be a positive finite number of e-/ADU");
        }
    }
    Ok(PhotometryConfig {
        aperture_radius: Some(aperture_radius),
        gain,
        sky_annulus,
        ..PhotometryConfig::default()
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn time_series_photometry_cmd(
    app: tauri::AppHandle,
    paths: Vec<String>,
    targets: Vec<TimeSeriesTarget>,
    aperture_radius: f64,
    annulus_inner: Option<f64>,
    annulus_outer: Option<f64>,
    gain: Option<f64>,
    exclude_dq: Option<bool>,
    track_drift: Option<bool>,
) -> Result<serde_json::Value, String> {
    let config = validate_time_series_args(&paths, &targets, aperture_radius, annulus_inner, annulus_outer, gain)
        .map_err(|e| format!("{e:#}"))?;
    let progress = ProgressHandle::new(&app, EVENT_TIME_SERIES_PROGRESS, paths.len() as u64);
    blocking_cmd!({
        let result = measure_time_series(
            &paths,
            &targets,
            &config,
            exclude_dq.unwrap_or(false),
            track_drift.unwrap_or(true),
            Some(&progress),
        )?;
        progress.emit_complete();
        Ok(serde_json::to_value(result)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::alignment::pair::shift_image_subpixel;
    use crate::core::synth::pipeline::{frame_header, generate_stack, FieldType, PsfType, SynthConfig};
    use crate::core::synth::star_field::{uniform_field, Star};
    use crate::infra::fits::writer::write_fits_mono;
    use ndarray::Array2;

    const FIELD_SIZE: u32 = 128;
    const N_FRAMES: u32 = 8;
    const CADENCE_SECONDS: f64 = 60.0;
    const EDGE_MARGIN: f64 = 20.0;
    const ISOLATION_PX: f64 = 26.0;
    const SYNTH_GAIN: f64 = 1.5;
    const APERTURE_RADIUS: f64 = 5.0;
    const SHIFT_X: f64 = 6.0;
    const SHIFT_Y: f64 = -4.0;
    const OFFSET_TOLERANCE_PX: f64 = 0.5;
    const CENTROID_TOLERANCE_PX: f64 = 0.5;

    fn stack_config(seed: u64) -> SynthConfig {
        let mut config = SynthConfig::default();
        config.field.width = FIELD_SIZE;
        config.field.height = FIELD_SIZE;
        config.field.n_stars = 6;
        config.field.flux_min = 40_000.0;
        config.field.flux_max = 400_000.0;
        config.field.seed = seed;
        config.field_type = FieldType::Uniform;
        config.psf_type = PsfType::Moffat { fwhm: 3.0, beta: 4.0 };
        config.noise.gain = SYNTH_GAIN;
        config.n_frames = N_FRAMES;
        config.cadence_seconds = CADENCE_SECONDS;
        config
    }

    fn brightest_first(stars: &[Star]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..stars.len()).collect();
        order.sort_by(|a, b| stars[*b].flux.total_cmp(&stars[*a].flux));
        order
    }

    fn well_separated(stars: &[Star], chosen: &[usize]) -> bool {
        let limit = FIELD_SIZE as f64 - 1.0 - EDGE_MARGIN;
        chosen.iter().all(|&i| {
            let s = &stars[i];
            let inside = s.x >= EDGE_MARGIN && s.x <= limit && s.y >= EDGE_MARGIN && s.y <= limit;
            let isolated = stars
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .all(|(_, o)| ((o.x - s.x).powi(2) + (o.y - s.y).powi(2)).sqrt() >= ISOLATION_PX);
            inside && isolated
        })
    }

    fn isolated_stack_config() -> SynthConfig {
        (1..=2000u64)
            .map(stack_config)
            .find(|config| {
                let stars = uniform_field(&config.field);
                let order = brightest_first(&stars);
                well_separated(&stars, &order[..3])
            })
            .expect("a seed with three isolated bright stars")
    }

    struct Stack {
        paths: Vec<String>,
        stars: Vec<Star>,
        targets: Vec<TimeSeriesTarget>,
        config: SynthConfig,
    }

    fn write_stack(dir: &std::path::Path, mutate: impl Fn(usize, Array2<f32>) -> Array2<f32>) -> Stack {
        let config = isolated_stack_config();
        let (frames, _, stars) = generate_stack(&config).unwrap();
        let paths: Vec<String> = frames
            .into_iter()
            .enumerate()
            .map(|(i, frame)| {
                let path = dir.join(format!("frame_{i:04}.fits"));
                let header = frame_header(&config.noise, i, config.cadence_seconds);
                write_fits_mono(path.to_str().unwrap(), &mutate(i, frame), Some(&header)).unwrap();
                path.to_str().unwrap().to_string()
            })
            .collect();
        let targets = bright_targets(&stars);
        Stack { paths, stars, targets, config }
    }

    fn bright_targets(stars: &[Star]) -> Vec<TimeSeriesTarget> {
        brightest_first(stars)[..3]
            .iter()
            .enumerate()
            .map(|(k, &i)| TimeSeriesTarget {
                x: stars[i].x,
                y: stars[i].y,
                label: format!("S{k}"),
                role: if k == 0 { ROLE_TARGET.into() } else { ROLE_COMP.into() },
            })
            .collect()
    }

    fn base_config() -> PhotometryConfig {
        PhotometryConfig {
            aperture_radius: Some(APERTURE_RADIUS),
            gain: Some(SYNTH_GAIN),
            ..PhotometryConfig::default()
        }
    }

    fn differential(frame: &TimeSeriesFrame) -> (f64, f64) {
        let t = frame.targets[0].as_ref().unwrap();
        let c1 = frame.targets[1].as_ref().unwrap();
        let c2 = frame.targets[2].as_ref().unwrap();
        let ensemble = c1.net_flux + c2.net_flux;
        let ensemble_err = (c1.flux_err.powi(2) + c2.flux_err.powi(2)).sqrt();
        let mag = -2.5 * (t.net_flux / ensemble).log10();
        let err = 1.0857 * ((t.flux_err / t.net_flux).powi(2) + (ensemble_err / ensemble).powi(2)).sqrt();
        (mag, err)
    }

    #[test]
    fn a_synthetic_stack_gives_a_flat_differential_light_curve_at_the_catalog_ratio() {
        let dir = tempfile::tempdir().unwrap();
        let stack = write_stack(dir.path(), |_, frame| frame);
        let out = measure_time_series(&stack.paths, &stack.targets, &base_config(), false, true, None).unwrap();
        assert_eq!(out.n_frames, N_FRAMES as usize);
        assert_eq!(out.n_skipped, 0);
        assert_eq!(out.reference_path, stack.paths[0]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        for frame in &out.frames {
            assert!(frame.skipped.is_none());
            assert!(frame.targets.iter().all(|t| t.is_some()), "{:?}", frame.errors);
            assert!(frame.targets.iter().flatten().all(|t| t.growth_curve.is_empty()));
            assert_eq!(frame.exptime, Some(stack.config.noise.exposure_time));
            assert!(frame.time_source.as_deref().unwrap().contains("DATE-OBS"));
        }

        let order = brightest_first(&stack.stars);
        let catalog_adu = |i: usize| stack.stars[order[i]].flux / SYNTH_GAIN;
        let expected = -2.5 * (catalog_adu(0) / (catalog_adu(1) + catalog_adu(2))).log10();
        let curve: Vec<(f64, f64)> = out.frames.iter().map(differential).collect();
        let n = curve.len() as f64;
        let mean = curve.iter().map(|(m, _)| m).sum::<f64>() / n;
        let rms = (curve.iter().map(|(m, _)| (m - mean).powi(2)).sum::<f64>() / n).sqrt();
        let mean_err = curve.iter().map(|(_, e)| e).sum::<f64>() / n;
        assert!((mean - expected).abs() < 0.02, "mean {mean} expected {expected}");
        assert!(rms < 3.0 * mean_err, "rms {rms} against propagated error {mean_err}");

        for pair in out.frames.windows(2) {
            let step = pair[1].jd_mid.unwrap() - pair[0].jd_mid.unwrap();
            assert!((step - CADENCE_SECONDS / 86400.0).abs() < 1e-8, "step {step}");
        }
        for frame in &out.frames[1..] {
            let offset = frame.offset.as_ref().expect("tracked offset");
            assert!(offset.registered);
            assert!(offset.dx.abs() < OFFSET_TOLERANCE_PX && offset.dy.abs() < OFFSET_TOLERANCE_PX, "{} {}", offset.dx, offset.dy);
        }
        assert!(out.frames[0].offset.is_none());
    }

    #[test]
    fn drift_tracking_recovers_a_shifted_frame_and_recentres_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let stack = write_stack(dir.path(), |i, frame| if i == 3 { shift_image_subpixel(&frame, -SHIFT_Y, -SHIFT_X) } else { frame });
        let tracked = measure_time_series(&stack.paths, &stack.targets, &base_config(), false, true, None).unwrap();
        let offset = tracked.frames[3].offset.as_ref().expect("offset on the shifted frame");
        assert!(offset.registered, "confidence {}", offset.confidence);
        assert!((offset.dx - SHIFT_X).abs() < OFFSET_TOLERANCE_PX, "dx {}", offset.dx);
        assert!((offset.dy - SHIFT_Y).abs() < OFFSET_TOLERANCE_PX, "dy {}", offset.dy);
        let reference = tracked.frames[0].targets[0].as_ref().unwrap();
        let shifted = tracked.frames[3].targets[0].as_ref().expect("target measured on the shifted frame");
        let moved_x = shifted.x - SHIFT_X;
        let moved_y = shifted.y - SHIFT_Y;
        assert!((moved_x - reference.x).abs() < CENTROID_TOLERANCE_PX, "{moved_x} vs {}", reference.x);
        assert!((moved_y - reference.y).abs() < CENTROID_TOLERANCE_PX, "{moved_y} vs {}", reference.y);
        let (m_ref, e_ref) = differential(&tracked.frames[0]);
        let (m_shift, _) = differential(&tracked.frames[3]);
        assert!((m_ref - m_shift).abs() < 5.0 * e_ref, "{m_ref} vs {m_shift}");

        let untracked = measure_time_series(&stack.paths, &stack.targets, &base_config(), false, false, None).unwrap();
        assert!(untracked.frames.iter().all(|f| f.offset.is_none()));
        let previous = untracked.frames[2].targets[0].as_ref().unwrap();
        match untracked.frames[3].targets[0].as_ref() {
            Some(found) => {
                let jump = ((found.x - previous.x).powi(2) + (found.y - previous.y).powi(2)).sqrt();
                assert!(jump > 2.0, "the peak search followed the star by {jump} px, which the centroid-jump check flags");
            }
            None => assert!(untracked.frames[3].errors[0].is_some()),
        }
    }

    #[test]
    fn a_frame_of_different_dimensions_is_skipped_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let stack = write_stack(dir.path(), |_, frame| frame);
        let small = dir.path().join("small.fits");
        write_fits_mono(small.to_str().unwrap(), &Array2::<f32>::zeros((64, 64)), None).unwrap();
        let mut paths = stack.paths[..2].to_vec();
        paths.push(small.to_str().unwrap().to_string());
        let out = measure_time_series(&paths, &stack.targets, &base_config(), false, true, None).unwrap();
        assert_eq!(out.n_frames, 3);
        assert_eq!(out.n_skipped, 1);
        let skipped = &out.frames[2];
        assert_eq!(skipped.skipped.as_deref(), Some("dimensions 64 x 64 differ from the reference 128 x 128"));
        assert_eq!(skipped.file_name, "small.fits");
        assert!(skipped.targets.iter().all(|t| t.is_none()));
        assert!(skipped.offset.is_none());
        assert!(out.frames[..2].iter().all(|f| f.skipped.is_none()));
        assert_eq!(file_name_of("/data/run/frame.fits#hdu=2"), "frame.fits");
    }

    #[test]
    fn ignored_targets_and_missing_files_do_not_stop_the_series() {
        let dir = tempfile::tempdir().unwrap();
        let stack = write_stack(dir.path(), |_, frame| frame);
        let mut targets = stack.targets.clone();
        targets[1].role = ROLE_IGNORE.into();
        let mut paths = stack.paths[..2].to_vec();
        paths.push(dir.path().join("missing.fits").to_str().unwrap().to_string());
        let out = measure_time_series(&paths, &targets, &base_config(), false, false, None).unwrap();
        assert_eq!(out.n_skipped, 1);
        assert!(out.frames[2].skipped.as_deref().unwrap().starts_with("failed to load"));
        assert!(out.frames[0].targets[1].is_none() && out.frames[0].errors[1].is_none());
        assert!(out.frames[0].targets[0].is_some() && out.frames[0].targets[2].is_some());
    }

    fn target(role: &str) -> TimeSeriesTarget {
        TimeSeriesTarget { x: 10.0, y: 10.0, label: "t".into(), role: role.into() }
    }

    #[test]
    fn too_many_targets_and_no_paths_are_refused_before_loading() {
        let path = vec!["/nowhere/frame.fits".to_string()];
        let err = validate_time_series_args(&[], &[target(ROLE_TARGET)], 5.0, None, None, None).unwrap_err();
        assert!(err.to_string().contains("at least one frame path"), "{err}");
        let many: Vec<TimeSeriesTarget> = (0..65).map(|_| target(ROLE_COMP)).collect();
        let err = validate_time_series_args(&path, &many, 5.0, None, None, None).unwrap_err();
        assert!(err.to_string().contains("65 targets exceeds the limit of 64"), "{err}");
        let err = validate_time_series_args(&path, &[target("guide")], 5.0, None, None, None).unwrap_err();
        assert!(err.to_string().contains("role 'guide'"), "{err}");
        let mut nan = target(ROLE_TARGET);
        nan.x = f64::NAN;
        let err = validate_time_series_args(&path, &[nan], 5.0, None, None, None).unwrap_err();
        assert!(err.to_string().contains("must be finite"), "{err}");
        let err = validate_time_series_args(&path, &[target(ROLE_TARGET)], 1.0, None, None, None).unwrap_err();
        assert!(err.to_string().contains("aperture radius 1"), "{err}");
        let err = validate_time_series_args(&path, &[target(ROLE_TARGET)], 5.0, Some(4.0), Some(12.0), None).unwrap_err();
        assert!(err.to_string().contains("larger than the aperture radius"), "{err}");
        let err = validate_time_series_args(&path, &[target(ROLE_TARGET)], 5.0, Some(8.0), None, None).unwrap_err();
        assert!(err.to_string().contains("both an inner and an outer"), "{err}");
        let err = validate_time_series_args(&path, &[target(ROLE_TARGET)], 5.0, None, None, Some(0.0)).unwrap_err();
        assert!(err.to_string().contains("gain 0"), "{err}");
        let config = validate_time_series_args(&path, &[target(ROLE_CHECK)], 5.0, Some(8.0), Some(12.0), Some(1.5)).unwrap();
        assert_eq!(config.aperture_radius, Some(5.0));
        assert_eq!(config.sky_annulus, Some((8.0, 12.0)));
        assert_eq!(config.gain, Some(1.5));
        let err = validate_time_series_args(&path, &[target(ROLE_TARGET)], 5.0, Some(20.0), Some(3000.0), None).unwrap_err();
        assert!(err.to_string().contains("sky annulus outer radius 3000 must be at most 512 pixels"), "{err}");
        let err = validate_time_series_args(&path, &[target(ROLE_TARGET)], 61.0, None, None, None).unwrap_err();
        assert!(err.to_string().contains("aperture radius 61 must be between 2 and 60 pixels"), "{err}");
    }

    fn gaussian_field(stars: &[(f64, f64, f64)]) -> Array2<f32> {
        let sigma = 3.0 / 2.354_820_045_f64;
        let norm = 2.0 * std::f64::consts::PI * sigma * sigma;
        Array2::from_shape_fn((FIELD_SIZE as usize, FIELD_SIZE as usize), |(y, x)| {
            let flux: f64 = stars
                .iter()
                .map(|&(sx, sy, f)| f / norm * (-((x as f64 - sx).powi(2) + (y as f64 - sy).powi(2)) / (2.0 * sigma * sigma)).exp())
                .sum();
            (100.0 + flux) as f32
        })
    }

    fn write_fields(dir: &Path, prefix: &str, fields: &[Array2<f32>]) -> Vec<String> {
        fields
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let path = dir.join(format!("{prefix}_{i:04}.fits"));
                write_fits_mono(path.to_str().unwrap(), field, None).unwrap();
                path.to_str().unwrap().to_string()
            })
            .collect()
    }

    fn write_headed_stack(dir: &Path, prefix: &str, frames: usize, card: impl Fn(usize, &mut HduHeader)) -> (Vec<String>, Vec<TimeSeriesTarget>) {
        let config = isolated_stack_config();
        let (images, _, stars) = generate_stack(&config).unwrap();
        let paths = images
            .into_iter()
            .take(frames)
            .enumerate()
            .map(|(i, image)| {
                let path = dir.join(format!("{prefix}_{i:04}.fits"));
                let mut header = frame_header(&config.noise, i, config.cadence_seconds);
                card(i, &mut header);
                write_fits_mono(path.to_str().unwrap(), &image, Some(&header)).unwrap();
                path.to_str().unwrap().to_string()
            })
            .collect();
        (paths, bright_targets(&stars))
    }

    fn crowded_targets() -> Vec<TimeSeriesTarget> {
        vec![
            TimeSeriesTarget { x: 64.0, y: 64.0, label: "T".into(), role: ROLE_TARGET.into() },
            TimeSeriesTarget { x: 30.0, y: 30.0, label: "C1".into(), role: ROLE_COMP.into() },
            TimeSeriesTarget { x: 98.0, y: 34.0, label: "C2".into(), role: ROLE_COMP.into() },
        ]
    }

    const CROWDED_COMPS: [(f64, f64, f64); 2] = [(30.0, 30.0, 20_000.0), (98.0, 34.0, 15_000.0)];

    fn crowded_field(stars: &[(f64, f64, f64)]) -> Array2<f32> {
        let all: Vec<(f64, f64, f64)> = stars.iter().chain(CROWDED_COMPS.iter()).copied().collect();
        gaussian_field(&all)
    }

    #[test]
    fn a_target_that_fades_below_a_neighbour_in_the_search_box_is_never_measured_on_the_neighbour() {
        let dir = tempfile::tempdir().unwrap();
        let flux_of = |i: usize| if (2..=4).contains(&i) { 4_000.0 } else { 10_000.0 };
        let fields: Vec<Array2<f32>> = (0..6)
            .map(|i| crowded_field(&[(64.0, 64.0, flux_of(i)), (71.0, 69.0, 8_000.0)]))
            .collect();
        let paths = write_fields(dir.path(), "eclipse", &fields);
        let out = measure_time_series(&paths, &crowded_targets(), &base_config(), false, true, None).unwrap();
        assert!(out.frames[1..].iter().all(|f| f.offset.as_ref().is_some_and(|o| o.registered)));
        let anchor = out.frames[0].targets[0].as_ref().unwrap();
        assert!((anchor.x - 64.0).hypot(anchor.y - 64.0) < 1.0, "anchor ({}, {})", anchor.x, anchor.y);
        let mut rejected = 0;
        for frame in &out.frames {
            match frame.targets[0].as_ref() {
                Some(t) => {
                    assert!((t.x - 64.0).hypot(t.y - 64.0) < 2.0, "frame {} measured at ({}, {})", frame.index, t.x, t.y);
                    let injected = flux_of(frame.index);
                    assert!((t.net_flux / injected - 1.0).abs() < 0.03, "frame {} net {} for {injected}", frame.index, t.net_flux);
                }
                None => {
                    rejected += 1;
                    let error = frame.errors[0].as_deref().unwrap_or("");
                    assert!(error.contains("from the expected position"), "frame {}: {error}", frame.index);
                }
            }
        }
        assert!(rejected > 0);
        assert!(out.frames[0].targets[0].is_some() && out.frames[1].targets[0].is_some() && out.frames[5].targets[0].is_some());
        assert!(out.frames.iter().all(|f| f.targets[1].is_some() && f.targets[2].is_some()));
        assert!(!out.warnings.iter().any(|w| w.contains("identity is not checked")), "{:?}", out.warnings);
    }

    #[test]
    fn a_placed_point_whose_reference_centroid_lands_on_a_brighter_neighbour_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let field = crowded_field(&[(64.0, 64.0, 3_000.0), (70.0, 70.0, 6_000.0)]);
        let paths = write_fields(dir.path(), "crowded", &[field.clone(), field]);
        let out = measure_time_series(&paths, &crowded_targets(), &base_config(), false, true, None).unwrap();
        let reported: Vec<&String> = out.warnings.iter().filter(|w| w.contains("placed point")).collect();
        assert_eq!(reported.len(), 1, "{:?}", out.warnings);
        assert!(reported[0].starts_with("T: the reference centroid (69."), "{}", reported[0]);
        assert!(reported[0].contains("placed point (64.0, 64.0)"), "{}", reported[0]);
        assert!(out.frames[0].targets[0].is_some());
    }

    #[test]
    fn untracked_frames_are_measured_as_before_and_warn_once_that_identity_is_unchecked() {
        let dir = tempfile::tempdir().unwrap();
        let field = crowded_field(&[(64.0, 64.0, 10_000.0)]);
        let paths = write_fields(dir.path(), "untracked", &[field.clone(), field.clone(), field]);
        let out = measure_time_series(&paths, &crowded_targets(), &base_config(), false, false, None).unwrap();
        let unchecked: Vec<&String> = out.warnings.iter().filter(|w| w.contains("identity is not checked")).collect();
        assert_eq!(unchecked.len(), 1, "{:?}", out.warnings);
        assert!(unchecked[0].starts_with("target identity is not checked on 2 frames"), "{}", unchecked[0]);
        assert!(out.frames.iter().all(|f| f.targets.iter().all(|t| t.is_some())));
        let tracked = measure_time_series(&paths, &crowded_targets(), &base_config(), false, true, None).unwrap();
        assert!(tracked.warnings.is_empty(), "{:?}", tracked.warnings);
    }

    #[test]
    fn a_comp_star_drifted_out_of_the_frame_is_reported_instead_of_measured_at_the_edge() {
        let dir = tempfile::tempdir().unwrap();
        let size = 256usize;
        let drift = 40.0;
        let sigma = 1.3;
        let mut stars: Vec<(f64, f64)> = (0..40u32)
            .map(|k| (20.0 + f64::from(k * 37 % 180), 20.0 + f64::from(k * 53 % 216)))
            .collect();
        stars.push((230.0, 128.0));
        let render = |dx: f64| {
            Array2::from_shape_fn((size, size), |(y, x)| {
                let mut v = 100.0f64;
                for (sx, sy) in &stars {
                    let r2 = (x as f64 - sx - dx).powi(2) + (y as f64 - sy).powi(2);
                    v += 20_000.0 * (-r2 / (2.0 * sigma * sigma)).exp();
                }
                v as f32
            })
        };
        let paths = write_fields(dir.path(), "drift", &[render(0.0), render(drift)]);
        let targets = vec![
            TimeSeriesTarget { x: stars[3].0, y: stars[3].1, label: "T".into(), role: ROLE_TARGET.into() },
            TimeSeriesTarget { x: 230.0, y: 128.0, label: "C".into(), role: ROLE_COMP.into() },
        ];
        let out = measure_time_series(&paths, &targets, &base_config(), false, true, None).unwrap();
        let offset = out.frames[1].offset.as_ref().unwrap();
        assert!(offset.registered && (offset.dx - drift).abs() < 0.5, "{} {}", offset.dx, offset.confidence);
        assert!(out.frames[0].targets[1].is_some());
        assert!(out.frames[1].targets[1].is_none(), "{:?}", out.frames[1].targets[1].as_ref().map(|t| (t.x, t.net_flux)));
        let error = out.frames[1].errors[1].as_deref().unwrap();
        assert!(error.contains("lies outside the 256 x 256 image"), "{error}");
        assert!(out.frames[1].targets[0].is_some(), "{:?}", out.frames[1].errors[0]);
    }

    #[test]
    fn a_frame_taken_through_a_different_filter_is_skipped_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let filters = ["V", "V", "B", "v"];
        let (paths, targets) = write_headed_stack(dir.path(), "filtered", 4, |i, header| header.set(FILTER_KEY, filters[i].to_string()));
        let out = measure_time_series(&paths, &targets, &base_config(), false, true, None).unwrap();
        assert_eq!(out.n_frames, 4);
        assert_eq!(out.n_skipped, 2);
        assert!(out.frames[..2].iter().all(|f| f.skipped.is_none() && f.filter.as_deref() == Some("V")));
        assert_eq!(out.frames[2].skipped.as_deref(), Some("filter B differs from the reference filter V"));
        assert_eq!(out.frames[3].skipped.as_deref(), Some("filter v differs from the reference filter V"));
        assert!(out.frames[2..].iter().all(|f| f.targets.iter().all(|t| t.is_none())));

        let (unlabelled, _) = write_headed_stack(dir.path(), "unlabelled", 3, |i, header| {
            if i == 1 {
                header.set(FILTER_KEY, "R".to_string());
            }
        });
        let mixed = measure_time_series(&unlabelled, &targets, &base_config(), false, true, None).unwrap();
        assert_eq!(mixed.n_skipped, 0);
    }

    #[test]
    fn frames_carrying_processing_provenance_raise_the_processed_data_warning_once() {
        let dir = tempfile::tempdir().unwrap();
        let (paths, targets) = write_headed_stack(dir.path(), "stretched", 3, |_, header| {
            header.set("ABPROC", "arcsinh".to_string());
        });
        let out = measure_time_series(&paths, &targets, &base_config(), false, true, None).unwrap();
        let processed: Vec<&str> = out
            .warnings
            .iter()
            .map(String::as_str)
            .filter(|w| w.starts_with("photometry on processed data"))
            .collect();
        assert_eq!(processed, vec!["photometry on processed data (arcsinh)"], "{:?}", out.warnings);
    }

    #[test]
    fn a_series_without_gain_or_err_plane_warns_that_errors_leave_out_source_photon_noise() {
        let dir = tempfile::tempdir().unwrap();
        let stack = write_stack(dir.path(), |_, frame| frame);
        let no_gain = PhotometryConfig { gain: None, ..base_config() };
        let out = measure_time_series(&stack.paths[..2], &stack.targets, &no_gain, false, true, None).unwrap();
        assert!(out.frames[0].targets.iter().flatten().all(|t| !t.err_used));
        assert_eq!(out.warnings, vec![SKY_ONLY_ERRORS_WARNING.to_string()]);
        let with_gain = measure_time_series(&stack.paths[..2], &stack.targets, &base_config(), false, true, None).unwrap();
        assert!(with_gain.warnings.is_empty(), "{:?}", with_gain.warnings);
    }

    #[test]
    fn the_time_source_names_the_header_time_scale_without_changing_the_time() {
        let dir = tempfile::tempdir().unwrap();
        let (tt_paths, targets) = write_headed_stack(dir.path(), "tt", 2, |_, header| header.set(TIMESYS_KEY, "TT".to_string()));
        let (utc_paths, _) = write_headed_stack(dir.path(), "utc", 2, |_, header| header.set(TIMESYS_KEY, "UTC".to_string()));
        let (plain_paths, _) = write_headed_stack(dir.path(), "plain", 2, |_, _| {});
        let run = |paths: &[String]| measure_time_series(paths, &targets, &base_config(), false, true, None).unwrap();
        let (tt, utc, plain) = (run(&tt_paths), run(&utc_paths), run(&plain_paths));
        let source = |out: &TimeSeriesResult| out.frames[0].time_source.clone().unwrap();
        assert!(source(&tt).ends_with("; time scale TT (TIMESYS), not converted to UTC"), "{}", source(&tt));
        assert!(source(&utc).ends_with("; time scale UTC (TIMESYS)"), "{}", source(&utc));
        assert!(source(&plain).ends_with("; UTC assumed (no TIMESYS)"), "{}", source(&plain));
        assert!(source(&plain).starts_with("DATE-OBS"), "{}", source(&plain));
        assert_eq!(tt.frames[0].jd_mid, plain.frames[0].jd_mid);
        assert_eq!(utc.frames[1].jd_mid, plain.frames[1].jd_mid);
    }
}
