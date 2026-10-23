use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ndarray::Array2;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Deserialize;

use super::synthetic::{
    at_most, destination, flat_frame, gradient_frame, hot_pixel_frame, noise_frame, wcs_from_cards,
    ARCSEC_PER_DEG,
};
use super::{healpix_provider, solve_local, CatalogProvider, LocalSolveParams};
use crate::core::astrometry::catalog::observation_epoch_year;
use crate::core::astrometry::solve_hints::{Pointing, ScaleRange, SolveHints};
use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
use crate::core::stacking::{never_cancelled, no_stage};
use crate::infra::fits::reader::extract_image_mmap;
use crate::types::header::HduHeader;

const ENV_CORPUS: &str = "ASTROBURST_SOLVE_CORPUS";
const ENV_TIER0: &str = "ASTROBURST_TIER0_PATH";
const ENV_TIER1: &str = "ASTROBURST_TIER1_PATH";
const ENV_TIER1_KIND: &str = "ASTROBURST_TIER1_KIND";
const ENV_TIER1_FLOOR: &str = "ASTROBURST_TIER1_FOV_FLOOR_DEG";
const TIER0_LABEL: &str = "tier0-tycho2";
const TIER0_DEPTH_MAG: f64 = 12.0;
const TIER1_2MASS_LABEL: &str = "tier1-2mass";
const TIER1_2MASS_DEPTH_MAG: f64 = 16.0;
const TIER1_2MASS_FLOOR_DEG: f64 = 0.4;
const TIER1_SIRIL_LABEL: &str = "tier1-gaia-siril";
const TIER1_SIRIL_DEPTH_MAG: f64 = 17.0;
const TIER1_SIRIL_FLOOR_DEG: f64 = 0.25;
const C1_SOLVED_FRACTION: f64 = 0.95;
const C1_TOLERANCE_ARCSEC: f64 = 2.0;
const C1_TOLERANCE_PX: f64 = 1.0;
const C1_MEDIAN_ARCSEC: f64 = 1.0;
const C1_HINT_OFFSET_FIELDS: f64 = 0.5;
const C2_HINT_OFFSET_DEG: f64 = 2.0;
const C2_MAX_RMS_PX: f64 = 0.3;
const C2_MAX_CENTRE_PX: f64 = 1.0;
const C2_MAX_CORNER_PX: f64 = 2.0;
const C2_FRAMES: usize = 5;
const FALSE_POSITIVE_TOLERANCES: f64 = 5.0;
const HINT_SCALE_MARGIN: f64 = 0.10;
const HINT_SCALE_BIAS: f64 = 0.05;
const TIER0_LOW_LATITUDE_FOV_DEG: f64 = 1.0;
const TIER0_LOW_LATITUDE_MAX_GLAT_DEG: f64 = 30.0;
const TIER0_ALL_LATITUDES_FOV_DEG: f64 = 2.0;
const C4_SIZE: (usize, usize) = (3000, 2000);
const C4_POINTINGS: [(f64, f64); 3] = [(266.4, -28.9), (56.75, 24.12), (192.86, 27.13)];
const C4_SCALES_ARCSEC: [f64; 2] = [1.2, 10.0];

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    frames: Vec<FrameEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct FrameEntry {
    file: String,
    #[serde(default)]
    glat_deg: Option<f64>,
    #[serde(default)]
    native: bool,
    #[serde(default)]
    region: Option<[usize; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Zero,
    One,
}

struct TierSetup {
    label: &'static str,
    provider: Box<dyn CatalogProvider>,
    floor_deg: f64,
}

#[derive(Debug, Clone)]
struct Accuracy {
    worst_corner_arcsec: f64,
    worst_corner_px: f64,
    centre_px: f64,
    tolerance_arcsec: f64,
    rms_px: f64,
    matched: usize,
    predicted: usize,
    sip_order: Option<u8>,
    centres_tried: usize,
}

#[derive(Debug, Clone)]
struct Outcome {
    set: &'static str,
    name: String,
    fov_deg: f64,
    glat_deg: Option<f64>,
    solved: Option<Accuracy>,
    has_truth: bool,
    failure: Option<String>,
    elapsed: Duration,
}

impl Outcome {
    fn is_false_positive(&self) -> bool {
        match (&self.solved, self.set) {
            (None, _) => false,
            (Some(_), "c4") => true,
            (Some(accuracy), _) => {
                !self.has_truth
                    || !at_most(
                        accuracy.worst_corner_arcsec,
                        FALSE_POSITIVE_TOLERANCES * accuracy.tolerance_arcsec,
                    )
            }
        }
    }

    fn line(&self) -> String {
        let numbers = match &self.solved {
            Some(a) => format!(
                "solved corner {:.3}\" ({:.3} px) centre {:.3} px tol {:.3}\" rms {:.3} px matched {}/{} sip {:?} centres {}",
                a.worst_corner_arcsec,
                a.worst_corner_px,
                a.centre_px,
                a.tolerance_arcsec,
                a.rms_px,
                a.matched,
                a.predicted,
                a.sip_order,
                a.centres_tried
            ),
            None => format!(
                "no solution: {}",
                self.failure.as_deref().unwrap_or("unknown")
            ),
        };
        format!(
            "{:<3} {:<48} fov {:>6.3} deg |b| {:>5} {:>8.0?}  {numbers}",
            self.set,
            self.name,
            self.fov_deg,
            self.glat_deg
                .map_or("-".to_string(), |b| format!("{:.1}", b.abs())),
            self.elapsed
        )
    }
}

fn corpus_root() -> PathBuf {
    std::env::var_os(ENV_CORPUS)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!(
                "set {ENV_CORPUS} to the folder holding c1/, c2/ and c3/ with their manifest.json"
            )
        })
}

fn tier_setup(tier: Tier) -> TierSetup {
    let (env, label, depth, floor) = match tier {
        Tier::Zero => (
            ENV_TIER0,
            TIER0_LABEL,
            TIER0_DEPTH_MAG,
            TIER0_LOW_LATITUDE_FOV_DEG,
        ),
        Tier::One => {
            let siril = std::env::var(ENV_TIER1_KIND).is_ok_and(|kind| kind == "siril");
            let (label, depth, default_floor) = if siril {
                (
                    TIER1_SIRIL_LABEL,
                    TIER1_SIRIL_DEPTH_MAG,
                    TIER1_SIRIL_FLOOR_DEG,
                )
            } else {
                (
                    TIER1_2MASS_LABEL,
                    TIER1_2MASS_DEPTH_MAG,
                    TIER1_2MASS_FLOOR_DEG,
                )
            };
            let floor = std::env::var(ENV_TIER1_FLOOR)
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(default_floor);
            (ENV_TIER1, label, depth, floor)
        }
    };
    let path = std::env::var_os(env)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {env} to the {label} catalogue file"));
    let provider = healpix_provider(&path, label, depth)
        .unwrap_or_else(|error| panic!("{label} at {}: {error}", path.display()));
    TierSetup {
        label,
        provider,
        floor_deg: floor,
    }
}

fn read_manifest(set_dir: &Path) -> Option<Manifest> {
    let path = set_dir.join("manifest.json");
    let text = std::fs::read_to_string(&path).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display())))
}

struct LoadedFrame {
    image: Array2<f32>,
    header: HduHeader,
    truth: Option<WcsTransform>,
}

fn load_frame(path: &Path) -> LoadedFrame {
    let file = File::open(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let loaded =
        extract_image_mmap(&file).unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
    let truth = WcsTransform::from_header(&loaded.header).ok();
    LoadedFrame {
        image: loaded.image,
        header: loaded.header,
        truth,
    }
}

fn region_box(entry: &FrameEntry, width: usize, height: usize) -> [f64; 4] {
    match entry.region {
        Some([x0, x1, y0, y1]) => [
            x0 as f64 - 0.5,
            x1 as f64 + 0.5,
            y0 as f64 - 0.5,
            y1 as f64 + 0.5,
        ],
        None => [-0.5, width as f64 - 0.5, -0.5, height as f64 - 0.5],
    }
}

fn hints_around(
    centre: (f64, f64),
    scale_arcsec: f64,
    long_side_px: usize,
    epoch_year: Option<f64>,
    seed: u64,
) -> SolveHints {
    let mut rng = StdRng::seed_from_u64(seed);
    let nominal = scale_arcsec * (1.0 + HINT_SCALE_BIAS * (2.0 * rng.gen::<f64>() - 1.0));
    SolveHints {
        centre: Some(Pointing {
            ra: centre.0,
            dec: centre.1,
            source: "corpus".to_string(),
        }),
        scale: Some(ScaleRange {
            low_arcsec: nominal * (1.0 - HINT_SCALE_MARGIN),
            high_arcsec: nominal * (1.0 + HINT_SCALE_MARGIN),
            source: "corpus".to_string(),
        }),
        fov_deg: Some(long_side_px as f64 * nominal / ARCSEC_PER_DEG),
        epoch_year,
        ..SolveHints::default()
    }
}

fn truth_centre(truth: &WcsTransform, region: [f64; 4]) -> (f64, f64) {
    let sky = truth.pixel_to_world((region[0] + region[1]) / 2.0, (region[2] + region[3]) / 2.0);
    (sky.ra, sky.dec)
}

fn measure_accuracy(
    cards: &[(String, String)],
    truth: &WcsTransform,
    size: (usize, usize),
    region: [f64; 4],
    solution: &super::LocalSolution,
) -> Accuracy {
    let solved = wcs_from_cards(cards, size.0, size.1);
    let scale = truth.pixel_scale_arcsec();
    let error = |x: f64, y: f64| {
        let t = truth.pixel_to_world(x, y);
        let s = solved.pixel_to_world(x, y);
        angular_separation(t.ra, t.dec, s.ra, s.dec) * ARCSEC_PER_DEG
    };
    let corners = [
        (region[0], region[2]),
        (region[1], region[2]),
        (region[1], region[3]),
        (region[0], region[3]),
    ];
    let worst = corners
        .iter()
        .map(|&(x, y)| error(x, y))
        .fold(0.0, |w: f64, e| if e.is_nan() || e > w { e } else { w });
    let centre = error((region[0] + region[1]) / 2.0, (region[2] + region[3]) / 2.0);
    Accuracy {
        worst_corner_arcsec: worst,
        worst_corner_px: worst / scale,
        centre_px: centre / scale,
        tolerance_arcsec: C1_TOLERANCE_ARCSEC.max(C1_TOLERANCE_PX * scale),
        rms_px: solution.rms_px,
        matched: solution.matched,
        predicted: solution.predicted,
        sip_order: solution.sip_order,
        centres_tried: solution.centres_tried,
    }
}

struct FrameMeta {
    name: String,
    region: [f64; 4],
    glat_deg: Option<f64>,
    fov_deg: f64,
}

fn frame_meta(
    name: String,
    entry: &FrameEntry,
    truth: &WcsTransform,
    size: (usize, usize),
) -> FrameMeta {
    FrameMeta {
        name,
        region: region_box(entry, size.0, size.1),
        glat_deg: entry.glat_deg,
        fov_deg: size.0.max(size.1) as f64 * truth.pixel_scale_arcsec() / ARCSEC_PER_DEG,
    }
}

fn run_frame(
    set: &'static str,
    meta: FrameMeta,
    loaded: &LoadedFrame,
    hints: &SolveHints,
    setup: &TierSetup,
) -> Outcome {
    let region = meta.region;
    let (height, width) = loaded.image.dim();
    let params = LocalSolveParams::default();
    let started = Instant::now();
    let result = solve_local(
        &loaded.image,
        hints,
        setup.provider.as_ref(),
        &params,
        &never_cancelled,
        &no_stage,
    );
    let elapsed = started.elapsed();
    let (solved, failure) = match result {
        Ok(solution) => {
            let measured = loaded.truth.as_ref().map(|truth| {
                measure_accuracy(
                    &solution.wcs_cards,
                    truth,
                    (width, height),
                    region,
                    &solution,
                )
            });
            let fallback = Accuracy {
                worst_corner_arcsec: f64::INFINITY,
                worst_corner_px: f64::INFINITY,
                centre_px: f64::INFINITY,
                tolerance_arcsec: C1_TOLERANCE_ARCSEC,
                rms_px: solution.rms_px,
                matched: solution.matched,
                predicted: solution.predicted,
                sip_order: solution.sip_order,
                centres_tried: solution.centres_tried,
            };
            (Some(measured.unwrap_or(fallback)), None)
        }
        Err(failure) => (
            None,
            Some(format!("{:?}: {}", failure.reason, failure.message)),
        ),
    };
    let outcome = Outcome {
        set,
        name: meta.name,
        fov_deg: meta.fov_deg,
        glat_deg: meta.glat_deg,
        solved,
        has_truth: loaded.truth.is_some(),
        failure,
        elapsed,
    };
    eprintln!("[{}] {}", setup.label, outcome.line());
    outcome
}

fn frame_seed(set: &str, index: usize) -> u64 {
    set.bytes().fold(index as u64 + 1, |acc, b| {
        acc.wrapping_mul(131).wrapping_add(u64::from(b))
    })
}

fn run_c1(root: &Path, setup: &TierSetup, out: &mut Vec<Outcome>) {
    let dir = root.join("c1");
    let manifest =
        read_manifest(&dir).unwrap_or_else(|| panic!("{}: no manifest.json", dir.display()));
    for (index, entry) in manifest.frames.iter().enumerate() {
        let loaded = load_frame(&dir.join(&entry.file));
        let truth = loaded
            .truth
            .as_ref()
            .unwrap_or_else(|| panic!("c1 {}: the truth WCS does not parse", entry.file));
        let (height, width) = loaded.image.dim();
        let name = if entry.native {
            format!("{} (native)", entry.file)
        } else {
            entry.file.clone()
        };
        let meta = frame_meta(name, entry, truth, (width, height));
        let scale = truth.pixel_scale_arcsec();
        let seed = frame_seed("c1", index);
        let mut rng = StdRng::seed_from_u64(seed);
        let offset = C1_HINT_OFFSET_FIELDS * meta.fov_deg * rng.gen::<f64>();
        let centre = destination(
            truth_centre(truth, meta.region),
            360.0 * rng.gen::<f64>(),
            offset,
        );
        let hints = hints_around(
            centre,
            scale,
            width.max(height),
            observation_epoch_year(&loaded.header),
            seed,
        );
        out.push(run_frame("c1", meta, &loaded, &hints, setup));
    }
}

fn run_c2(root: &Path, setup: &TierSetup, out: &mut Vec<Outcome>) {
    let dir = root.join("c2");
    let Some(manifest) = read_manifest(&dir) else {
        eprintln!(
            "[{}] c2 skipped: no {}",
            setup.label,
            dir.join("manifest.json").display()
        );
        return;
    };
    for (index, entry) in manifest.frames.iter().enumerate() {
        let loaded = load_frame(&dir.join(&entry.file));
        let truth = loaded
            .truth
            .as_ref()
            .unwrap_or_else(|| panic!("c2 {}: the header WCS does not parse", entry.file));
        let (height, width) = loaded.image.dim();
        let meta = frame_meta(entry.file.clone(), entry, truth, (width, height));
        let seed = frame_seed("c2", index);
        let bearing = 360.0 * StdRng::seed_from_u64(seed).gen::<f64>();
        let centre = destination(
            truth_centre(truth, meta.region),
            bearing,
            C2_HINT_OFFSET_DEG,
        );
        let hints = hints_around(
            centre,
            truth.pixel_scale_arcsec(),
            width.max(height),
            observation_epoch_year(&loaded.header),
            seed,
        );
        out.push(run_frame("c2", meta, &loaded, &hints, setup));
    }
}

fn run_c3(root: &Path, setup: &TierSetup, out: &mut Vec<Outcome>) {
    let dir = root.join("c3");
    let Some(manifest) = read_manifest(&dir) else {
        eprintln!(
            "[{}] c3 skipped: no {}",
            setup.label,
            dir.join("manifest.json").display()
        );
        return;
    };
    for (index, entry) in manifest.frames.iter().enumerate() {
        let loaded = load_frame(&dir.join(&entry.file));
        let (height, width) = loaded.image.dim();
        let target = loaded
            .header
            .get_f64("TARG_RA")
            .zip(loaded.header.get_f64("TARG_DEC"));
        let Some(truth) = loaded.truth.as_ref() else {
            eprintln!(
                "[{}] c3 {}: no WCS to take the scale hint from",
                setup.label, entry.file
            );
            continue;
        };
        let meta = frame_meta(entry.file.clone(), entry, truth, (width, height));
        let scale = truth.pixel_scale_arcsec();
        let centre = target.unwrap_or_else(|| truth_centre(truth, meta.region));
        let hints = hints_around(
            centre,
            scale,
            width.max(height),
            observation_epoch_year(&loaded.header),
            frame_seed("c3", index),
        );
        out.push(run_frame("c3", meta, &loaded, &hints, setup));
    }
}

fn run_c4(setup: &TierSetup, out: &mut Vec<Outcome>) {
    let (width, height) = C4_SIZE;
    let frames = [
        ("noise", noise_frame(width, height, 401)),
        ("flat", flat_frame(width, height)),
        ("gradient", gradient_frame(width, height, 402)),
        ("hot pixels", hot_pixel_frame(width, height, 403)),
    ];
    for (name, image) in frames {
        let loaded = LoadedFrame {
            image,
            header: HduHeader::empty(),
            truth: None,
        };
        for (index, &pointing) in C4_POINTINGS.iter().enumerate() {
            for &scale in &C4_SCALES_ARCSEC {
                let hints = hints_around(pointing, scale, width.max(height), None, index as u64);
                let meta = FrameMeta {
                    name: format!("{name} at {pointing:?}, {scale}\"/px"),
                    region: [-0.5, width as f64 - 0.5, -0.5, height as f64 - 0.5],
                    glat_deg: None,
                    fov_deg: width.max(height) as f64 * scale / ARCSEC_PER_DEG,
                };
                out.push(run_frame("c4", meta, &loaded, &hints, setup));
            }
        }
    }
}

fn run_corpus(tier: Tier) -> (TierSetup, Vec<Outcome>) {
    let root = corpus_root();
    let setup = tier_setup(tier);
    let mut out = Vec::new();
    run_c1(&root, &setup, &mut out);
    run_c2(&root, &setup, &mut out);
    run_c3(&root, &setup, &mut out);
    run_c4(&setup, &mut out);
    (setup, out)
}

struct TierRun {
    label: &'static str,
    floor_deg: f64,
    outcomes: Vec<Outcome>,
}

fn tier_run(tier: Tier) -> &'static TierRun {
    static TIER0: OnceLock<TierRun> = OnceLock::new();
    static TIER1: OnceLock<TierRun> = OnceLock::new();
    let cell = match tier {
        Tier::Zero => &TIER0,
        Tier::One => &TIER1,
    };
    cell.get_or_init(|| {
        let (setup, outcomes) = run_corpus(tier);
        TierRun {
            label: setup.label,
            floor_deg: setup.floor_deg,
            outcomes,
        }
    })
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    match values.len() {
        0 => f64::NAN,
        n if n % 2 == 1 => values[n / 2],
        n => (values[n / 2 - 1] + values[n / 2]) / 2.0,
    }
}

fn c1_verdict(label: &str, eligible: &[&Outcome]) -> Vec<String> {
    let mut problems = Vec::new();
    if eligible.is_empty() {
        return vec![format!("{label}: no eligible C1 frame")];
    }
    let solved: Vec<&Accuracy> = eligible.iter().filter_map(|o| o.solved.as_ref()).collect();
    let fraction = solved.len() as f64 / eligible.len() as f64;
    let outside: Vec<String> = eligible
        .iter()
        .filter(|o| {
            o.solved
                .as_ref()
                .is_some_and(|a| !at_most(a.worst_corner_arcsec, a.tolerance_arcsec))
        })
        .map(|o| o.line())
        .collect();
    let mut corners: Vec<f64> = solved.iter().map(|a| a.worst_corner_arcsec).collect();
    let median_corner = median(&mut corners);
    eprintln!(
        "{label}: {} of {} solved ({:.1} %), median worst corner {median_corner:.3}\", {} outside max(2\", 1 px)",
        solved.len(),
        eligible.len(),
        100.0 * fraction,
        outside.len()
    );
    if fraction < C1_SOLVED_FRACTION {
        problems.push(format!(
            "{label}: solved fraction {fraction:.3} < {C1_SOLVED_FRACTION}"
        ));
    }
    if !outside.is_empty() {
        problems.push(format!(
            "{label}: outside tolerance:\n{}",
            outside.join("\n")
        ));
    }
    if !at_most(median_corner, C1_MEDIAN_ARCSEC) {
        problems.push(format!(
            "{label}: median worst corner {median_corner:.3}\" > {C1_MEDIAN_ARCSEC}\""
        ));
    }
    problems
}

#[test]
#[ignore]
fn corpus_c1_tier1_hinted_solves_95_percent_within_max_2_arcsec_1_px_median_1_arcsec() {
    let run = tier_run(Tier::One);
    let eligible: Vec<&Outcome> = run
        .outcomes
        .iter()
        .filter(|o| o.set == "c1" && o.fov_deg >= run.floor_deg)
        .collect();
    let problems = c1_verdict(&format!("AC-1 {}", run.label), &eligible);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
#[ignore]
fn corpus_c1_tier0_only_solves_95_percent_at_1_deg_low_latitude_and_2_deg_everywhere() {
    let run = tier_run(Tier::Zero);
    let c1: Vec<&Outcome> = run.outcomes.iter().filter(|o| o.set == "c1").collect();
    let fov_at_least = |o: &Outcome, fov: f64| o.fov_deg >= fov;
    let low_latitude: Vec<&Outcome> = c1
        .iter()
        .copied()
        .filter(|o| {
            fov_at_least(o, TIER0_LOW_LATITUDE_FOV_DEG)
                && o.glat_deg
                    .is_some_and(|b| b.abs() <= TIER0_LOW_LATITUDE_MAX_GLAT_DEG)
        })
        .collect();
    let everywhere: Vec<&Outcome> = c1
        .iter()
        .copied()
        .filter(|o| fov_at_least(o, TIER0_ALL_LATITUDES_FOV_DEG))
        .collect();
    let mut problems = c1_verdict("AC-2 tier 0, >= 1 deg at |b| <= 30", &low_latitude);
    problems.extend(c1_verdict("AC-2 tier 0, >= 2 deg", &everywhere));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
#[ignore]
fn corpus_c2_tess_matches_the_header_sip_within_0_3_px_rms_and_2_px_worst_corner() {
    let run = if std::env::var_os(ENV_TIER0).is_some() {
        tier_run(Tier::Zero)
    } else {
        tier_run(Tier::One)
    };
    let c2: Vec<&Outcome> = run.outcomes.iter().filter(|o| o.set == "c2").collect();
    let mut problems = Vec::new();
    if c2.len() != C2_FRAMES {
        problems.push(format!("{} C2 frames, expected {C2_FRAMES}", c2.len()));
    }
    for outcome in &c2 {
        let ok = outcome.solved.as_ref().is_some_and(|a| {
            a.rms_px <= C2_MAX_RMS_PX
                && a.centre_px <= C2_MAX_CENTRE_PX
                && a.worst_corner_px <= C2_MAX_CORNER_PX
                && a.sip_order.is_some()
        });
        if !ok {
            problems.push(outcome.line());
        }
    }
    assert!(
        problems.is_empty(),
        "AC-5 {}:\n{}",
        run.label,
        problems.join("\n")
    );
}

#[test]
#[ignore]
fn corpus_c1_to_c4_have_zero_false_positives() {
    let tiers: Vec<Tier> = [(ENV_TIER0, Tier::Zero), (ENV_TIER1, Tier::One)]
        .into_iter()
        .filter(|(env, _)| std::env::var_os(env).is_some())
        .map(|(_, tier)| tier)
        .collect();
    assert!(!tiers.is_empty(), "set {ENV_TIER0} and/or {ENV_TIER1}");
    let mut problems = Vec::new();
    for tier in tiers {
        let run = tier_run(tier);
        let false_positives: Vec<String> = run
            .outcomes
            .iter()
            .filter(|o| o.is_false_positive())
            .map(|o| o.line())
            .collect();
        eprintln!(
            "AC-3 {}: {} runs, {} false positives",
            run.label,
            run.outcomes.len(),
            false_positives.len()
        );
        if !false_positives.is_empty() {
            problems.push(format!("{}:\n{}", run.label, false_positives.join("\n")));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
