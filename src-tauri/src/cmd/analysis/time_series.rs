use std::path::Path;
use std::time::Instant;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use super::{annulus_arg, apply_calibration, check_annulus_clears_aperture, photometry_planes, resolve_dq_mask};
use crate::cmd::common::{blocking_cmd, load_cached, load_cached_full};
use crate::core::alignment::pair::offset_within_limits;
use crate::core::alignment::phase_correlation::{is_low_confidence, phase_correlate};
use crate::core::analysis::photometry::{measure_star_full, saturation_level, PhotometryConfig, StarPhotometry};
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
const MIN_APERTURE_RADIUS: f64 = 2.0;
const MAX_APERTURE_RADIUS: f64 = 60.0;
const ROLE_TARGET: &str = "target";
const ROLE_COMP: &str = "comp";
const ROLE_CHECK: &str = "check";
const ROLE_IGNORE: &str = "ignore";
const ROLE_NAMES: [&str; 4] = [ROLE_TARGET, ROLE_COMP, ROLE_CHECK, ROLE_IGNORE];
const EXPTIME_KEY: &str = "EXPTIME";
const AIRMASS_KEY: &str = "AIRMASS";
const FILTER_KEY: &str = "FILTER";

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

#[allow(clippy::too_many_arguments)]
fn measure_frame(
    index: usize,
    path: &str,
    entry: &ImageEntry,
    reference: &ImageEntry,
    targets: &[TimeSeriesTarget],
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
    let planes = photometry_planes(path, dims);
    let mask = resolve_dq_mask(path, exclude_dq, dims);
    let header = entry.header();
    let wcs = header.and_then(|h| WcsTransform::from_header(h).ok());
    let photcal = header.and_then(|h| PhotCal::from_header(h, wcs.as_ref()));
    let config = PhotometryConfig {
        saturation: Some(saturation_level(header, entry.stats().max)),
        ..config_base.clone()
    };
    let timing = header.and_then(mid_exposure_jd);
    let shift = if track_drift && index > 0 {
        track_frame(reference, entry, &file_name, warnings)
    } else {
        FrameShift { offset: None, dx: 0.0, dy: 0.0 }
    };

    let mut measured: Vec<Option<StarPhotometry>> = Vec::with_capacity(targets.len());
    let mut errors: Vec<Option<String>> = Vec::with_capacity(targets.len());
    for target in targets {
        if target.role == ROLE_IGNORE {
            measured.push(None);
            errors.push(None);
            continue;
        }
        let outcome = measure_star_full(
            entry.arr(),
            planes.err.as_ref().map(|e| e.arr()),
            mask.as_ref().map(|m| &m.map),
            planes.saturated.as_ref(),
            target.x + shift.dx,
            target.y + shift.dy,
            &config,
        );
        match outcome {
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
                config_base,
                exclude_dq,
                track_drift,
                &mut warnings,
            ),
            Err(e) => skipped_frame(index, path, targets.len(), format!("failed to load: {e:#}")),
        };
        let file_name = frame.file_name.clone();
        frames.push(frame);
        if let Some(p) = progress {
            p.tick_with_stage(&file_name);
        }
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
    if !aperture_radius.is_finite() || !(MIN_APERTURE_RADIUS..=MAX_APERTURE_RADIUS).contains(&aperture_radius) {
        bail!("aperture radius {aperture_radius} must be between {MIN_APERTURE_RADIUS} and {MAX_APERTURE_RADIUS} pixels");
    }
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
        let order = brightest_first(&stars);
        let targets = order[..3]
            .iter()
            .enumerate()
            .map(|(k, &i)| TimeSeriesTarget {
                x: stars[i].x,
                y: stars[i].y,
                label: format!("S{k}"),
                role: if k == 0 { ROLE_TARGET.into() } else { ROLE_COMP.into() },
            })
            .collect();
        Stack { paths, stars, targets, config }
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
    }
}
