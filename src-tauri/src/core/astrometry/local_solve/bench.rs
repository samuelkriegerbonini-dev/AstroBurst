use std::time::{Duration, Instant};

use ndarray::Array2;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::core::alignment::affine::AffineAlignMethod;
use crate::core::alignment::triangles::{
    match_star_lists, ransac_affine, select_alignment_stars, Parity, TriangleParams,
};
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::healpix_cat::{
    write_catalog, CatHeader, CatType, CatalogStar, HealpixCatalog, Record,
};
use crate::core::synth::noise::{apply_noise, NoiseParams};
use crate::core::synth::psf::{render_stars, GaussianPsf};
use crate::core::synth::star_field::{uniform_field, FieldConfig};

use super::synthetic::{
    at_most, random_detections, render_field, FrameStyle, SyntheticProvider, TruthField,
    TIER1_CUT_STARS_PER_DEG2,
};
use super::{solve_local, solve_stars, FailureReason, LocalSolveParams};
use crate::core::stacking::{never_cancelled, no_stage};

const RUNS: usize = 7;
const DETECT_SIGMA: f64 = 5.0;
const STARS_PER_MEGAPIXEL: f64 = 125.0;
const FWHM_PX: f64 = 3.0;
const FRAMES: [(&str, u32, u32); 2] = [("4 MP", 2448, 1632), ("24 MP", 6000, 4000)];
const MATCH_STARS: usize = 60;
const MATCH_SHARED: usize = 48;
const MATCH_ROTATION_DEG: f64 = 37.0;
const MATCH_SCALE: f64 = 1.03;
const MATCH_SHIFT_PX: (f64, f64) = (120.0, -80.0);
const MIN_INLIERS: usize = 8;
const TIER0_LEVEL: u8 = 6;
const TIER0_STARS_PER_PIXEL: usize = 52;
const TIER0_EPOCH_YEAR: f32 = 2026.0;
const PLEIADES: (f64, f64) = (56.75, 24.12);
const CONE_DIAGONAL_FRACTION: f64 = 0.6;
const FIELD_ASPECT: f64 = 1.5;
const FIELD_LONG_SIDES_DEG: [f64; 4] = [1.0, 3.0, 10.0, 30.0];

#[derive(Debug)]
struct Timing {
    label: String,
    median: Duration,
    min: Duration,
    max: Duration,
    detail: String,
}

#[derive(Debug, Default)]
struct BenchReport {
    detect: Vec<Timing>,
    matching: Vec<Timing>,
    cone: Vec<Timing>,
}

fn timed<T>(mut work: impl FnMut() -> T) -> (Vec<Duration>, T) {
    let mut durations = Vec::with_capacity(RUNS);
    let mut last = None;
    for _ in 0..RUNS {
        let started = Instant::now();
        last = Some(work());
        durations.push(started.elapsed());
    }
    durations.sort();
    (durations, last.expect("RUNS is positive"))
}

fn summarise(label: String, durations: &[Duration], detail: String) -> Timing {
    Timing {
        label,
        median: durations[durations.len() / 2],
        min: durations[0],
        max: durations[durations.len() - 1],
        detail,
    }
}

fn synthetic_frame(width: u32, height: u32, seed: u64) -> Array2<f32> {
    let megapixels = f64::from(width) * f64::from(height) / 1e6;
    let config = FieldConfig {
        width,
        height,
        n_stars: (megapixels * STARS_PER_MEGAPIXEL).round() as usize,
        seed,
        ..FieldConfig::default()
    };
    let clean = render_stars(
        &uniform_field(&config),
        &GaussianPsf::from_fwhm(FWHM_PX),
        width,
        height,
    );
    apply_noise(&clean, &NoiseParams::default(), seed, None)
}

fn bench_detect(report: &mut BenchReport) -> (Vec<DetectedStar>, usize, usize) {
    let mut largest = (Vec::new(), 0, 0);
    for (seed, &(name, width, height)) in (1u64..).zip(FRAMES.iter()) {
        let frame = synthetic_frame(width, height, seed);
        let (durations, result) = timed(|| detect_stars(&frame, DETECT_SIGMA));
        report.detect.push(summarise(
            format!("detect_stars {name} ({width}x{height})"),
            &durations,
            format!("{} stars at {DETECT_SIGMA} sigma", result.stars.len()),
        ));
        largest = (result.stars, height as usize, width as usize);
    }
    largest
}

fn transformed(
    reference: &[(f64, f64)],
    mirrored: bool,
    width: f64,
    height: f64,
) -> Vec<(f64, f64)> {
    let (sin, cos) = MATCH_ROTATION_DEG.to_radians().sin_cos();
    let mut rng = StdRng::seed_from_u64(if mirrored { 17 } else { 13 });
    reference
        .iter()
        .enumerate()
        .map(|(index, &(x, y))| {
            if index >= MATCH_SHARED {
                return (rng.gen::<f64>() * width, rng.gen::<f64>() * height);
            }
            let x = if mirrored { width - x } else { x };
            (
                MATCH_SCALE * (cos * x - sin * y) + MATCH_SHIFT_PX.0,
                MATCH_SCALE * (sin * x + cos * y) + MATCH_SHIFT_PX.1,
            )
        })
        .collect()
}

fn bench_matching(report: &mut BenchReport, stars: &[DetectedStar], rows: usize, cols: usize) {
    let reference = select_alignment_stars(stars, MATCH_STARS, rows, cols);
    assert_eq!(reference.len(), MATCH_STARS, "detections for the matcher");
    let params = TriangleParams {
        parity: Parity::Either,
        ..TriangleParams::ALIGNMENT
    };
    for mirrored in [false, true] {
        let target = transformed(&reference, mirrored, cols as f64, rows as f64);
        let (match_durations, matched) = timed(|| match_star_lists(&reference, &target, &params));
        let (ransac_durations, fit) =
            timed(|| ransac_affine(&matched.pairs, AffineAlignMethod::Affine, &params));
        let fit = fit.expect("RANSAC fits the matched pairs");
        assert_eq!(matched.flipped, mirrored, "parity of the match");
        assert!(fit.inliers >= MIN_INLIERS, "{} inliers", fit.inliers);
        let parity = if mirrored { "mirrored" } else { "same" };
        report.matching.push(summarise(
            format!("triangles n={MATCH_STARS} both parities, {parity} target"),
            &match_durations,
            format!(
                "{} pairs, {} votes, flipped {}",
                matched.pairs.len(),
                matched.votes,
                matched.flipped
            ),
        ));
        report.matching.push(summarise(
            format!("ransac affine, {parity} target"),
            &ransac_durations,
            format!("{} inliers, rms {:.3} px", fit.inliers, fit.rms_px),
        ));
    }
}

fn tier0_like_catalogue() -> Vec<u8> {
    let mut rng = StdRng::seed_from_u64(61);
    let pixels: Vec<(u32, Vec<Record>)> = (0..cdshealpix::nested::n_hash(TIER0_LEVEL))
        .map(|pixel| {
            let records = (0..TIER0_STARS_PER_PIXEL)
                .map(|_| {
                    let (lon, lat) =
                        cdshealpix::nested::sph_coo(TIER0_LEVEL, pixel, rng.gen(), rng.gen());
                    Record::Star(CatalogStar::at(
                        lon.to_degrees(),
                        lat.to_degrees(),
                        Some(6.0 + 6.0 * rng.gen::<f64>()),
                    ))
                })
                .collect();
            (pixel as u32, records)
        })
        .collect();
    let header = CatHeader::whole("bench", TIER0_LEVEL, CatType::Compact6.code())
        .with_epoch_year(TIER0_EPOCH_YEAR);
    let mut bytes = Vec::new();
    write_catalog(&header, &pixels, &mut bytes).expect("synthetic tier 0");
    bytes
}

fn bench_cone(report: &mut BenchReport) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("tier0-like.abcat");
    std::fs::write(&path, tier0_like_catalogue()).expect("write synthetic tier 0");
    let catalog = HealpixCatalog::open_file(&path).expect("open synthetic tier 0");
    let diagonal_per_long_side = (1.0 + FIELD_ASPECT.powi(-2)).sqrt();
    for long_side in FIELD_LONG_SIDES_DEG {
        let radius = CONE_DIAGONAL_FRACTION * long_side * diagonal_per_long_side;
        let (durations, rows) = timed(|| {
            catalog
                .cone(
                    PLEIADES.0,
                    PLEIADES.1,
                    radius,
                    Some(f64::from(TIER0_EPOCH_YEAR)),
                )
                .expect("cone")
        });
        report.cone.push(summarise(
            format!("tier-0 cone mmap, {long_side} deg field"),
            &durations,
            format!(
                "{} rows, radius {radius:.3} deg, {} records in file",
                rows.len(),
                catalog.total_records()
            ),
        ));
    }
}

fn solve_bench_report() -> BenchReport {
    let mut report = BenchReport::default();
    let (stars, rows, cols) = bench_detect(&mut report);
    bench_matching(&mut report, &stars, rows, cols);
    bench_cone(&mut report);
    report
}

fn usable(timing: &Timing) -> bool {
    timing.min > Duration::ZERO
        && timing.min <= timing.median
        && timing.median <= timing.max
        && timing.median.as_secs_f64().is_finite()
}

#[test]
#[ignore]
fn solve_bench_reports_detect_and_match_timings() {
    let report = solve_bench_report();
    for timing in report
        .detect
        .iter()
        .chain(&report.matching)
        .chain(&report.cone)
    {
        eprintln!(
            "{:<44} median {:>10.3?}  min {:>10.3?}  max {:>10.3?}  {}",
            timing.label, timing.median, timing.min, timing.max, timing.detail
        );
    }
    let labels = |timings: &[Timing]| -> Vec<String> {
        timings.iter().map(|timing| timing.label.clone()).collect()
    };
    assert!(
        report.detect.len() == 2,
        "detect_stars timings for 4 MP and 24 MP: {:?}",
        labels(&report.detect)
    );
    assert!(
        report.matching.len() >= 2,
        "triangle matching timings for both parities: {:?}",
        labels(&report.matching)
    );
    assert!(
        !report.cone.is_empty(),
        "tier-0 cone timings: {:?}",
        labels(&report.cone)
    );
    for timing in report
        .detect
        .iter()
        .chain(&report.matching)
        .chain(&report.cone)
    {
        assert!(usable(timing), "{timing:?}");
    }
}

const SOLVE_RUNS: usize = 20;
const SOLVE_FRAME: (usize, usize) = (6000, 4000);
const SOLVE_SCALE_ARCSEC: f64 = 1.0;
const SOLVE_CATALOGUE_STARS_PER_DEG2: f64 = 150.0;
const SOLVE_DEPTH_MAG: f64 = 12.0;
const SOLVE_UNCATALOGUED: usize = 3000;
const SOLVE_FWHM_PX: f64 = 5.0;
const SOLVE_FLUX_AT_DEPTH: f64 = 3000.0;
const SOLVE_MAX_HINT_OFFSET_FIELDS: f64 = 0.5;
const SOLVE_MAX_CORNER_PX: f64 = 1.0;
const SOLVE_P50_BUDGET: Duration = Duration::from_millis(1500);
const SOLVE_P95_BUDGET: Duration = Duration::from_millis(3000);
const SOLVE_EXTRA_BYTES: u64 = 100 * 1024 * 1024;
const MIB: f64 = 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy)]
struct ProcessMemory {
    current: u64,
    peak: u64,
}

#[cfg(windows)]
fn process_memory() -> Option<ProcessMemory> {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut counters = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Counters::default()
    };
    let size = counters.cb;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    (ok != 0).then_some(ProcessMemory {
        current: counters.working_set_size as u64,
        peak: counters.peak_working_set_size as u64,
    })
}

#[cfg(target_os = "linux")]
fn process_memory() -> Option<ProcessMemory> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let bytes = |key: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| {
                rest.trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .map(|kib| kib * 1024)
    };
    Some(ProcessMemory {
        current: bytes("VmRSS:")?,
        peak: bytes("VmHWM:")?,
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_memory() -> Option<ProcessMemory> {
    None
}

fn nearest_rank(sorted: &[Duration], fraction: f64) -> Duration {
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[test]
#[ignore]
fn solve_bench_hinted_6000x4000_meets_p50_p95_and_memory_budgets() {
    if cfg!(debug_assertions) {
        panic!("release only, alone in the process: cargo test --release --lib -- --ignored --exact core::astrometry::local_solve::bench::solve_bench_hinted_6000x4000_meets_p50_p95_and_memory_budgets --nocapture");
    }
    let before = process_memory();
    let truth = TruthField::new(
        (279.23, 38.78),
        SOLVE_SCALE_ARCSEC,
        23.0,
        false,
        SOLVE_FRAME.0,
        SOLVE_FRAME.1,
    );
    let provider = SyntheticProvider::uniform(
        "tier0-synthetic",
        truth.centre(),
        4.0,
        SOLVE_CATALOGUE_STARS_PER_DEG2,
        (6.0, SOLVE_DEPTH_MAG),
        91,
    );
    let image = render_field(
        &truth,
        provider.rows(),
        SOLVE_DEPTH_MAG,
        &FrameStyle {
            fwhm_px: SOLVE_FWHM_PX,
            flux_at_depth: SOLVE_FLUX_AT_DEPTH,
            uncatalogued: SOLVE_UNCATALOGUED,
            seed: 92,
            ..FrameStyle::default()
        },
    );
    let image_bytes = (image.len() * std::mem::size_of::<f32>()) as u64;
    let params = LocalSolveParams::default();
    let mut durations = Vec::with_capacity(SOLVE_RUNS);
    let mut failures = Vec::new();
    for run in 0..SOLVE_RUNS {
        let mut rng = StdRng::seed_from_u64(1000 + run as u64);
        let hints = truth.hints(
            SOLVE_MAX_HINT_OFFSET_FIELDS * rng.gen::<f64>(),
            360.0 * rng.gen::<f64>(),
        );
        let started = Instant::now();
        let outcome = solve_local(
            &image,
            &hints,
            &provider,
            &params,
            &never_cancelled,
            &no_stage,
        );
        let elapsed = started.elapsed();
        durations.push(elapsed);
        match outcome {
            Ok(solution) => {
                let corner = truth.worst_corner_px(&solution.wcs_cards);
                eprintln!(
                    "run {run:>2}: {elapsed:>10.3?}  {} centres, {} matched of {} predicted, rms {:.3} px, worst corner {corner:.3} px",
                    solution.centres_tried, solution.matched, solution.predicted, solution.rms_px
                );
                if !at_most(corner, SOLVE_MAX_CORNER_PX) {
                    failures.push(format!("run {run}: worst corner {corner} px"));
                }
            }
            Err(failure) => failures.push(format!("run {run}: {failure:?}")),
        }
    }
    let after = process_memory();
    durations.sort();
    let p50 = nearest_rank(&durations, 0.50);
    let p95 = nearest_rank(&durations, 0.95);
    eprintln!(
        "hinted solve {}x{}: p50 {p50:?}, p95 {p95:?}, min {:?}, max {:?} over {SOLVE_RUNS} runs",
        SOLVE_FRAME.0,
        SOLVE_FRAME.1,
        durations[0],
        durations[SOLVE_RUNS - 1]
    );
    if p50 > SOLVE_P50_BUDGET {
        failures.push(format!("p50 {p50:?} > {SOLVE_P50_BUDGET:?}"));
    }
    if p95 > SOLVE_P95_BUDGET {
        failures.push(format!("p95 {p95:?} > {SOLVE_P95_BUDGET:?}"));
    }
    match (before, after) {
        (Some(before), Some(after)) => {
            let used = after.peak.saturating_sub(before.current);
            let budget = image_bytes + SOLVE_EXTRA_BYTES;
            eprintln!(
                "memory: working set {:.1} MiB before the frame (peak so far {:.1} MiB), peak {:.1} MiB after the runs; image {:.1} MiB; peak minus start {:.1} MiB against image + 100 MiB = {:.1} MiB",
                before.current as f64 / MIB,
                before.peak as f64 / MIB,
                after.peak as f64 / MIB,
                image_bytes as f64 / MIB,
                used as f64 / MIB,
                budget as f64 / MIB
            );
            if used > budget {
                failures.push(format!(
                    "peak memory {:.1} MiB above the start, budget {:.1} MiB (run the bench alone)",
                    used as f64 / MIB,
                    budget as f64 / MIB
                ));
            }
        }
        _ => eprintln!("memory: not measured on this platform"),
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const SMALL_FIELD_LONG_SIDE_DEG: f64 = 0.25;
const SMALL_FIELD_FRAME: (usize, usize) = (1500, 1000);
const SMALL_FIELD_RUNS: usize = 5;
const SMALL_FIELD_BUDGET: Duration = Duration::from_secs(3);

#[test]
#[ignore]
fn solve_bench_a_0_25_degree_field_exhausts_the_capped_default_spiral_within_3_s() {
    if cfg!(debug_assertions) {
        panic!("release only: cargo test --release --lib -- --ignored --exact core::astrometry::local_solve::bench::solve_bench_a_0_25_degree_field_exhausts_the_capped_default_spiral_within_3_s --nocapture");
    }
    let truth = TruthField::with_long_side(
        (12.5, 33.0),
        SMALL_FIELD_LONG_SIDE_DEG,
        0.0,
        false,
        SMALL_FIELD_FRAME.0,
        SMALL_FIELD_FRAME.1,
    );
    let provider = SyntheticProvider::uniform(
        "tier1-synthetic",
        truth.centre(),
        3.5,
        TIER1_CUT_STARS_PER_DEG2,
        (8.0, 16.0),
        61,
    );
    let hints = truth.hints(0.0, 0.0);
    let params = LocalSolveParams::default();
    let mut failures = Vec::new();
    for run in 0..SMALL_FIELD_RUNS {
        let detections = random_detections(40, truth.width, truth.height, 700 + run as u64);
        let started = Instant::now();
        let outcome = solve_stars(
            &detections,
            truth.size(),
            &hints,
            &provider,
            &params,
            &never_cancelled,
            &no_stage,
        );
        let elapsed = started.elapsed();
        match outcome {
            Ok(solution) => failures.push(format!("run {run}: a random list solved: {solution:?}")),
            Err(failure) => {
                eprintln!(
                    "run {run}: {elapsed:?}, {} centres, {:?}: {}",
                    failure.centres_tried, failure.reason, failure.message
                );
                if failure.reason != FailureReason::NoSolution
                    || failure.centres_tried > params.max_centres
                    || !failure.message.contains("capped")
                {
                    failures.push(format!("run {run}: {failure:?}"));
                }
                if elapsed > SMALL_FIELD_BUDGET {
                    failures.push(format!("run {run}: {elapsed:?} > {SMALL_FIELD_BUDGET:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
