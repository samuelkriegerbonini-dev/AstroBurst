use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use ndarray::Array2;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::synthetic::{
    at_most, flat_frame, gradient_frame, hot_pixel_frame, noise_frame, wcs_from_cards,
    ARCSEC_PER_DEG,
};
use super::{
    destination, healpix_provider, solve_local, CatalogProvider, LocalSolveParams, RankingKey,
};
use crate::core::alignment::affine::AffineAlignMethod;
use crate::core::alignment::triangles::{ransac_affine, TriangleMatch, TriangleParams};
use crate::core::astrometry::catalog::{observation_epoch_year, propagate_epoch, CatalogRow};
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
const INNER_REGION_FRACTION: f64 = 0.5;
const HINT_SCALE_MARGIN: f64 = 0.10;
const HINT_SCALE_BIAS: f64 = 0.05;
const TIER0_LOW_LATITUDE_FOV_DEG: f64 = 1.0;
const TIER0_LOW_LATITUDE_MAX_GLAT_DEG: f64 = 30.0;
const TIER0_ALL_LATITUDES_FOV_DEG: f64 = 2.0;
const C4_SIZE: (usize, usize) = (3000, 2000);
const C4_POINTINGS: [(f64, f64); 3] = [(266.4, -28.9), (56.75, 24.12), (192.86, 27.13)];
const C4_SCALES_ARCSEC: [f64; 2] = [1.2, 10.0];
const AC4_FOV_DEG: (f64, f64) = (0.9, 1.1);
const AC4_HINT_OFFSET_DEG: f64 = 2.5;
const AC4_BUDGET: Duration = Duration::from_secs(3);
const TRUTH_CRVAL_TOLERANCE_DEG: f64 = 1e-9;
const TRUTH_SCALE_TOLERANCE: f64 = 1e-9;
const TRUTH_FORBIDDEN_KEYS: [&str; 6] = [
    "A_ORDER", "B_ORDER", "PV1_1", "PV2_1", "DATE-OBS", "MJD-OBS",
];
const TRUTH_FORBIDDEN_TEXT: [&str; 4] = ["ASTROBURST", "PLATE SOLV", "SIPMXERR", "ASTROMETRY.NET"];
const HASH_CHUNK_BYTES: usize = 1 << 22;

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    frames: Vec<FrameEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct Hips2FitsRequest {
    ra: f64,
    dec: f64,
    fov: f64,
    width: usize,
    height: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct HeaderRecord {
    #[serde(default)]
    crval: Option<[f64; 2]>,
    #[serde(default)]
    a_order: Option<i64>,
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
    #[serde(default)]
    band: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    bytes: Option<u64>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    request: Option<Hips2FitsRequest>,
    #[serde(default)]
    header: Option<HeaderRecord>,
}

#[derive(Debug, Clone, Copy)]
struct HintSummary {
    offset_deg: f64,
    offset_fields: f64,
    scale_low: f64,
    scale_high: f64,
    truth_scale: f64,
}

fn file_sha256(path: &Path) -> String {
    static HASHES: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let cache = HASHES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(known) = cache.lock().expect("hash cache").get(path) {
        return known.clone();
    }
    let mut file = File::open(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_CHUNK_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    cache
        .lock()
        .expect("hash cache")
        .insert(path.to_path_buf(), digest.clone());
    digest
}

fn header_cd(header: &HduHeader) -> Option<[[f64; 2]; 2]> {
    if let Some(cd11) = header.get_f64("CD1_1") {
        return Some([
            [cd11, header.get_f64("CD1_2").unwrap_or(0.0)],
            [
                header.get_f64("CD2_1").unwrap_or(0.0),
                header.get_f64("CD2_2")?,
            ],
        ]);
    }
    let cdelt1 = header.get_f64("CDELT1")?;
    let cdelt2 = header.get_f64("CDELT2")?;
    let pc = |key: &str, unit: f64| header.get_f64(key).unwrap_or(unit);
    Some([
        [cdelt1 * pc("PC1_1", 1.0), cdelt1 * pc("PC1_2", 0.0)],
        [cdelt2 * pc("PC2_1", 0.0), cdelt2 * pc("PC2_2", 1.0)],
    ])
}

fn truth_problems(set: &str, entry: &FrameEntry, path: &Path, header: &HduHeader) -> Vec<String> {
    let name = &entry.file;
    let mut problems = Vec::new();
    if let Some(bytes) = entry.bytes {
        let on_disk = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if on_disk != bytes {
            problems.push(format!(
                "{set} {name}: size {on_disk} differs from the manifest {bytes}"
            ));
        }
    }
    if let Some(expected) = &entry.sha256 {
        let actual = file_sha256(path);
        if !actual.eq_ignore_ascii_case(expected) {
            problems.push(format!(
                "{set} {name}: SHA-256 {actual} differs from the manifest"
            ));
        }
    }
    if let Some(request) = &entry.request {
        let crval = header.get_f64("CRVAL1").zip(header.get_f64("CRVAL2"));
        match crval {
            Some((ra, dec))
                if (ra - request.ra).abs() <= TRUTH_CRVAL_TOLERANCE_DEG
                    && (dec - request.dec).abs() <= TRUTH_CRVAL_TOLERANCE_DEG => {}
            other => problems.push(format!(
                "{set} {name}: CRVAL {other:?} is not the hips2fits request ({}, {})",
                request.ra, request.dec
            )),
        }
        let naxis = header.get_i64("NAXIS1").zip(header.get_i64("NAXIS2"));
        if naxis != Some((request.width as i64, request.height as i64)) {
            problems.push(format!(
                "{set} {name}: NAXIS {naxis:?} differs from the request"
            ));
        }
        let crpix = header.get_f64("CRPIX1").zip(header.get_f64("CRPIX2"));
        let wanted = (request.width as f64 / 2.0, request.height as f64 / 2.0);
        if crpix != Some(wanted) {
            problems.push(format!(
                "{set} {name}: CRPIX {crpix:?}, the hips2fits rule gives {wanted:?}"
            ));
        }
        match header_cd(header) {
            Some(cd) => {
                let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
                let scale = det.abs().sqrt();
                let rule = (2.0 * (request.fov.to_radians() / 2.0).tan() / request.width as f64)
                    .to_degrees();
                if (scale / rule - 1.0).abs() > TRUTH_SCALE_TOLERANCE {
                    problems.push(format!(
                        "{set} {name}: scale {:.9}\"/px is {:.3e} off the tan rule",
                        scale * ARCSEC_PER_DEG,
                        scale / rule - 1.0
                    ));
                }
                if det >= 0.0 {
                    problems.push(format!(
                        "{set} {name}: det(CD) {det:.3e} is not the sky parity"
                    ));
                }
            }
            None => problems.push(format!("{set} {name}: no CD or CDELT cards")),
        }
        for key in TRUTH_FORBIDDEN_KEYS {
            if header.get(key).is_some() {
                problems.push(format!("{set} {name}: unexpected card {key}"));
            }
        }
    }
    if let Some(record) = &entry.header {
        if let Some([ra, dec]) = record.crval {
            let crval = header.get_f64("CRVAL1").zip(header.get_f64("CRVAL2"));
            match crval {
                Some((got_ra, got_dec))
                    if (got_ra - ra).abs() <= TRUTH_CRVAL_TOLERANCE_DEG
                        && (got_dec - dec).abs() <= TRUTH_CRVAL_TOLERANCE_DEG => {}
                other => problems.push(format!(
                    "{set} {name}: CRVAL {other:?} differs from the manifest record ({ra}, {dec})"
                )),
            }
        }
        if let Some(order) = record.a_order {
            if header.get_i64("A_ORDER") != Some(order) {
                problems.push(format!(
                    "{set} {name}: A_ORDER {:?} differs from the manifest record {order}",
                    header.get_i64("A_ORDER")
                ));
            }
        }
    }
    for (key, value) in &header.cards {
        let text = format!("{key} {value}").to_ascii_uppercase();
        if TRUTH_FORBIDDEN_TEXT
            .iter()
            .any(|needle| text.contains(needle))
        {
            problems.push(format!("{set} {name}: header text {key} = {value}"));
        }
    }
    problems
}

fn check_truth(set: &str, entry: &FrameEntry, path: &Path, header: &HduHeader) {
    let problems = truth_problems(set, entry, path, header);
    assert!(
        problems.is_empty(),
        "the corpus truth is not the untouched source product:\n{}",
        problems.join("\n")
    );
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
    inner_arcsec: f64,
    inner_px: f64,
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
    band: Option<String>,
    note: Option<String>,
    hint: Option<HintSummary>,
    solved: Option<Accuracy>,
    has_truth: bool,
    sip_truth: bool,
    failure: Option<String>,
    elapsed: Duration,
}

fn hint_summary(
    hints: &SolveHints,
    truth_centre: (f64, f64),
    truth_scale: f64,
    fov_deg: f64,
) -> Option<HintSummary> {
    let centre = hints.centre.as_ref()?;
    let scale = hints.scale.as_ref()?;
    let offset_deg = angular_separation(centre.ra, centre.dec, truth_centre.0, truth_centre.1);
    Some(HintSummary {
        offset_deg,
        offset_fields: offset_deg / fov_deg,
        scale_low: scale.low_arcsec,
        scale_high: scale.high_arcsec,
        truth_scale,
    })
}

impl Outcome {
    fn is_false_positive(&self) -> bool {
        match (&self.solved, self.set) {
            (None, _) => false,
            (Some(_), "c4") => true,
            (Some(accuracy), _) => {
                let judged = if self.sip_truth {
                    accuracy.inner_arcsec
                } else {
                    accuracy.worst_corner_arcsec
                };
                !self.has_truth
                    || !at_most(judged, FALSE_POSITIVE_TOLERANCES * accuracy.tolerance_arcsec)
            }
        }
    }

    fn line(&self) -> String {
        let numbers = match &self.solved {
            Some(a) => format!(
                "solved corner {:.3}\" ({:.3} px) centre {:.3} px tol {:.3}\" rms {:.3} px matched {}/{} sip {:?} centres {} inner {:.3} px",
                a.worst_corner_arcsec,
                a.worst_corner_px,
                a.centre_px,
                a.tolerance_arcsec,
                a.rms_px,
                a.matched,
                a.predicted,
                a.sip_order,
                a.centres_tried,
                a.inner_px
            ),
            None => format!(
                "no solution: {}",
                self.failure.as_deref().unwrap_or("unknown")
            ),
        };
        let hint = match &self.hint {
            Some(h) => format!(
                "hint off {:.3} deg ({:.2} fov) scale {:.4}..{:.4}\"/px (truth {:.4})",
                h.offset_deg, h.offset_fields, h.scale_low, h.scale_high, h.truth_scale
            ),
            None => "hint -".to_string(),
        };
        let note = self
            .note
            .as_deref()
            .map_or(String::new(), |note| format!("  note: {note}"));
        format!(
            "{:<3} {:<48} fov {:>6.3} deg |b| {:>5} {:<4} {:>8.0?}  {hint}  {numbers}{note}",
            self.set,
            self.name,
            self.fov_deg,
            self.glat_deg
                .map_or("-".to_string(), |b| format!("{:.1}", b.abs())),
            self.band.as_deref().unwrap_or("-"),
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
    let (env, label, depth, floor, ranking) = match tier {
        Tier::Zero => (
            ENV_TIER0,
            TIER0_LABEL,
            TIER0_DEPTH_MAG,
            TIER0_LOW_LATITUDE_FOV_DEG,
            RankingKey::MAGNITUDE,
        ),
        Tier::One => {
            let siril = std::env::var(ENV_TIER1_KIND).is_ok_and(|kind| kind == "siril");
            let (label, depth, default_floor, ranking) = if siril {
                (
                    TIER1_SIRIL_LABEL,
                    TIER1_SIRIL_DEPTH_MAG,
                    TIER1_SIRIL_FLOOR_DEG,
                    RankingKey::MAGNITUDE,
                )
            } else {
                (
                    TIER1_2MASS_LABEL,
                    TIER1_2MASS_DEPTH_MAG,
                    TIER1_2MASS_FLOOR_DEG,
                    RankingKey::TWO_MASS_J_KS,
                )
            };
            let floor = std::env::var(ENV_TIER1_FLOOR)
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(default_floor);
            (ENV_TIER1, label, depth, floor, ranking)
        }
    };
    let path = std::env::var_os(env)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {env} to the {label} catalogue file"));
    let provider = healpix_provider(&path, label, depth, ranking)
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
    let worse = |w: f64, e: f64| if e.is_nan() || e > w { e } else { w };
    let worst = corners.iter().map(|&(x, y)| error(x, y)).fold(0.0, worse);
    let (xc, yc) = ((region[0] + region[1]) / 2.0, (region[2] + region[3]) / 2.0);
    let (hx, hy) = ((region[1] - region[0]) / 2.0, (region[3] - region[2]) / 2.0);
    let centre = error(xc, yc);
    let inner = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|&(sx, sy): &(f64, f64)| {
            error(
                xc + sx * INNER_REGION_FRACTION * hx,
                yc + sy * INNER_REGION_FRACTION * hy,
            )
        })
        .fold(centre, worse);
    Accuracy {
        worst_corner_arcsec: worst,
        worst_corner_px: worst / scale,
        inner_arcsec: inner,
        inner_px: inner / scale,
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
    band: Option<String>,
    note: Option<String>,
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
        band: entry.band.clone(),
        note: entry.note.clone(),
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
    let hint = loaded.truth.as_ref().and_then(|truth| {
        hint_summary(
            hints,
            truth_centre(truth, region),
            truth.pixel_scale_arcsec(),
            meta.fov_deg,
        )
    });
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
                inner_arcsec: f64::INFINITY,
                inner_px: f64::INFINITY,
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
        band: meta.band,
        note: meta.note,
        hint,
        solved,
        has_truth: loaded.truth.is_some(),
        sip_truth: loaded.header.get_i64("A_ORDER").is_some(),
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
        let path = dir.join(&entry.file);
        let loaded = load_frame(&path);
        check_truth("c1", entry, &path, &loaded.header);
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
        let hints = c1_hints(index, &loaded, truth, &meta);
        out.push(run_frame("c1", meta, &loaded, &hints, setup));
    }
}

fn c1_hints(index: usize, loaded: &LoadedFrame, truth: &WcsTransform, meta: &FrameMeta) -> SolveHints {
    let (height, width) = loaded.image.dim();
    let seed = frame_seed("c1", index);
    let mut rng = StdRng::seed_from_u64(seed);
    let offset = C1_HINT_OFFSET_FIELDS * meta.fov_deg * rng.gen::<f64>();
    let centre = destination(
        truth_centre(truth, meta.region),
        360.0 * rng.gen::<f64>(),
        offset,
    );
    hints_around(
        centre,
        truth.pixel_scale_arcsec(),
        width.max(height),
        observation_epoch_year(&loaded.header),
        seed,
    )
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
        let path = dir.join(&entry.file);
        let loaded = load_frame(&path);
        check_truth("c2", entry, &path, &loaded.header);
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
        let path = dir.join(&entry.file);
        let loaded = load_frame(&path);
        check_truth("c3", entry, &path, &loaded.header);
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
                    band: None,
                    note: None,
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

fn catalogue_window(
    setup: &TierSetup,
    centre: (f64, f64),
    radius_deg: f64,
    skip: usize,
    limit: usize,
) -> String {
    let (rows, _) = setup
        .provider
        .cone(centre.0, centre.1, radius_deg, None)
        .unwrap_or_else(|error| panic!("{}: {error}", setup.label));
    let mut mags: Vec<f64> = rows.iter().filter_map(|row| row.g).collect();
    mags.sort_by(f64::total_cmp);
    let at = |rank: usize| {
        mags.get(rank)
            .map_or("-".to_string(), |mag| format!("{mag:.1}"))
    };
    format!(
        "{}: {} stars in the disc, mag at rank 0 {} / skip {} {} / {} {} / 120 {}",
        setup.label,
        mags.len(),
        at(0),
        skip,
        at(skip),
        skip + limit,
        at(skip + limit),
        at(120)
    )
}

#[test]
#[ignore]
fn corpus_frame_diagnostics_print_detection_and_catalogue_counts() {
    let root = corpus_root();
    let params = LocalSolveParams::default();
    let tiers: Vec<TierSetup> = [(ENV_TIER0, Tier::Zero), (ENV_TIER1, Tier::One)]
        .into_iter()
        .filter(|(env, _)| std::env::var_os(env).is_some())
        .map(|(_, tier)| tier_setup(tier))
        .collect();
    for set in ["c1", "c2"] {
        let dir = root.join(set);
        let Some(manifest) = read_manifest(&dir) else {
            continue;
        };
        for (index, entry) in manifest.frames.iter().enumerate() {
            let loaded = load_frame(&dir.join(&entry.file));
            let Some(truth) = loaded.truth.as_ref() else {
                continue;
            };
            let (height, width) = loaded.image.dim();
            let meta = frame_meta(entry.file.clone(), entry, truth, (width, height));
            let centre = truth_centre(truth, meta.region);
            let scale = truth.pixel_scale_arcsec();
            let hints = hints_around(
                centre,
                scale,
                width.max(height),
                observation_epoch_year(&loaded.header),
                frame_seed(set, index),
            );
            let detections = super::detect_for_solve(&loaded.image, &hints, &params);
            let usable = super::usable_detections(&detections, &params);
            let dropped = super::dropped_saturated(&detections, &params);
            let flagged = detections.iter().filter(|d| d.saturated).count();
            let disc_radius_px = params.cone_short_side_fraction * width.min(height) as f64;
            let disc = super::disc_points(&usable, (width, height), disc_radius_px);
            let fwhm = super::median_fwhm(&usable);
            let field_w = width as f64 * scale / ARCSEC_PER_DEG;
            let field_h = height as f64 * scale / ARCSEC_PER_DEG;
            let cone_radius = params.cone_short_side_fraction * field_w.min(field_h);
            let cone_area = std::f64::consts::PI * cone_radius * cone_radius;
            let skip = (dropped as f64 * cone_area / (field_w * field_h)).round() as usize;
            let windows: Vec<String> = tiers
                .iter()
                .map(|setup| {
                    catalogue_window(setup, centre, cone_radius, skip, params.hinted_match_stars)
                })
                .collect();
            eprintln!(
                "{set} {:<44} fov {:>6.3} |b| {:>5} {:<4} detections {} usable {} flagged {} dropped-saturated {} disc {} (r {:.0} px) fwhm {:.2} px | {}",
                meta.name,
                meta.fov_deg,
                meta.glat_deg
                    .map_or("-".to_string(), |b| format!("{:.1}", b.abs())),
                meta.band.as_deref().unwrap_or("-"),
                detections.len(),
                usable.len(),
                flagged,
                dropped,
                disc.len(),
                disc_radius_px,
                fwhm,
                windows.join(" | ")
            );
        }
    }
}

fn shipped_chain_tier(fov_deg: f64, aspect: f64) -> Tier {
    let expected_tier0 = super::TIER0_MEAN_STARS_PER_DEG2 * fov_deg * fov_deg / aspect;
    let tier0_set = std::env::var_os(ENV_TIER0).is_some();
    let tier1_set = std::env::var_os(ENV_TIER1).is_some();
    if tier0_set && (expected_tier0 >= super::TIER0_MIN_EXPECTED_STARS || !tier1_set) {
        Tier::Zero
    } else {
        assert!(tier1_set, "set {ENV_TIER0} and/or {ENV_TIER1}");
        Tier::One
    }
}

#[test]
#[ignore]
fn corpus_c1_1_deg_fields_solve_within_3_s_with_the_hint_2_5_deg_off() {
    let root = corpus_root();
    let dir = root.join("c1");
    let manifest =
        read_manifest(&dir).unwrap_or_else(|| panic!("{}: no manifest.json", dir.display()));
    let tier = shipped_chain_tier(1.0, 1.5);
    let setup = tier_setup(tier);
    eprintln!(
        "AC-4 runs on {} (the shipped chain's tier for a 1 deg field), hint {AC4_HINT_OFFSET_DEG} deg off",
        setup.label
    );
    let mut problems = Vec::new();
    let mut count = 0usize;
    for (index, entry) in manifest.frames.iter().enumerate() {
        let path = dir.join(&entry.file);
        let loaded = load_frame(&path);
        let truth = loaded
            .truth
            .as_ref()
            .unwrap_or_else(|| panic!("c1 {}: the truth WCS does not parse", entry.file));
        let (height, width) = loaded.image.dim();
        let meta = frame_meta(entry.file.clone(), entry, truth, (width, height));
        if entry.native || !(AC4_FOV_DEG.0..=AC4_FOV_DEG.1).contains(&meta.fov_deg) {
            continue;
        }
        check_truth("c1", entry, &path, &loaded.header);
        count += 1;
        let seed = frame_seed("ac4", index);
        let bearing = 360.0 * StdRng::seed_from_u64(seed).gen::<f64>();
        let centre = destination(
            truth_centre(truth, meta.region),
            bearing,
            AC4_HINT_OFFSET_DEG,
        );
        let hints = hints_around(
            centre,
            truth.pixel_scale_arcsec(),
            width.max(height),
            observation_epoch_year(&loaded.header),
            seed,
        );
        let outcome = run_frame("c1", meta, &loaded, &hints, &setup);
        let within = outcome
            .solved
            .as_ref()
            .is_some_and(|a| at_most(a.worst_corner_arcsec, a.tolerance_arcsec));
        if !within || outcome.elapsed > AC4_BUDGET {
            problems.push(outcome.line());
        }
    }
    assert!(count > 0, "no 1 deg C1 frame in the manifest");
    assert!(
        problems.is_empty(),
        "AC-4 {}: {} of {count} frames failed to solve within {AC4_BUDGET:?} and tolerance:\n{}",
        setup.label,
        problems.len(),
        problems.join("\n")
    );
}

#[test]
#[ignore]
fn corpus_c2_tess_matches_the_header_sip_within_0_3_px_rms_and_2_px_worst_corner() {
    let tier = shipped_chain_tier(12.0, 1.0);
    let run = tier_run(tier);
    eprintln!(
        "AC-5 runs on {} (the shipped chain's tier for a 12 deg field is tier 0{})",
        run.label,
        if tier == Tier::Zero {
            ""
        } else {
            "; tier 0 is not set, so this run is not the shipped configuration"
        }
    );
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

const UNEVEN_MATCH_FRAME: &str = "fov30_low1.fits";

#[test]
#[ignore]
fn corpus_fov30_low1_a_plane_field_with_an_uneven_match_fraction_solves_at_its_harness_hint() {
    let dir = corpus_root().join("c1");
    let manifest =
        read_manifest(&dir).unwrap_or_else(|| panic!("{}: no manifest.json", dir.display()));
    let (index, entry) = manifest
        .frames
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.file == UNEVEN_MATCH_FRAME)
        .unwrap_or_else(|| panic!("{UNEVEN_MATCH_FRAME} is not in the c1 manifest"));
    let path = dir.join(&entry.file);
    let loaded = load_frame(&path);
    check_truth("c1", entry, &path, &loaded.header);
    let truth = loaded
        .truth
        .as_ref()
        .unwrap_or_else(|| panic!("c1 {}: the truth WCS does not parse", entry.file));
    let (height, width) = loaded.image.dim();
    let meta = frame_meta(entry.file.clone(), entry, truth, (width, height));
    let hints = c1_hints(index, &loaded, truth, &meta);
    let mut failures = Vec::new();
    for tier in [Tier::Zero, Tier::One] {
        let setup = tier_setup(tier);
        let outcome = run_frame(
            "c1",
            frame_meta(entry.file.clone(), entry, truth, (width, height)),
            &loaded,
            &hints,
            &setup,
        );
        match &outcome.solved {
            Some(accuracy) if at_most(accuracy.worst_corner_arcsec, accuracy.tolerance_arcsec) => {}
            Some(accuracy) => failures.push(format!(
                "[{}] worst corner {:.3}\" outside the {:.3}\" tolerance",
                setup.label, accuracy.worst_corner_arcsec, accuracy.tolerance_arcsec
            )),
            None => failures.push(format!(
                "[{}] {}",
                setup.label,
                outcome.failure.as_deref().unwrap_or("unknown")
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const ENV_FRAME: &str = "ASTROBURST_SOLVE_FRAME";
const ENV_FRAME_OFFSET: &str = "ASTROBURST_SOLVE_HINT_OFFSET_DEG";
const ENV_FRAME_BEARING: &str = "ASTROBURST_SOLVE_HINT_BEARING_DEG";
const ENV_FRAME_FRACTIONS: &str = "ASTROBURST_SOLVE_DISC_FRACTIONS";
const ENV_FRAME_RESIDUALS: &str = "ASTROBURST_SOLVE_RESIDUALS";
const RESIDUAL_GRID: usize = 3;

type ResidualRow = (f64, f64, f64, f64, f64);

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

fn manifest_entry_for(path: &Path) -> Option<FrameEntry> {
    let dir = path.parent()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    read_manifest(dir)?
        .frames
        .into_iter()
        .find(|entry| entry.file == name)
}

fn percentile_sorted(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn residual_report(
    label: &str,
    wcs: &WcsTransform,
    rows: &[CatalogRow],
    usable: &[super::Detection],
    size: (usize, usize),
    radius: f64,
) {
    let (width, height) = size;
    let mut residuals: Vec<ResidualRow> = Vec::new();
    for row in rows {
        let Some(mag) = row.g else {
            continue;
        };
        let (px, py) = wcs.world_to_pixel(row.ra_epoch, row.dec_epoch);
        if !(px.is_finite() && py.is_finite() && super::inside(size, (px, py))) {
            continue;
        }
        let nearest = usable
            .iter()
            .map(|d| ((d.star.x - px), (d.star.y - py)))
            .filter(|(dx, dy)| dx.hypot(*dy) <= radius)
            .min_by(|a, b| a.0.hypot(a.1).total_cmp(&b.0.hypot(b.1)));
        if let Some((dx, dy)) = nearest {
            residuals.push((px, py, dx, dy, mag));
        }
    }
    let mut sorted: Vec<f64> = residuals.iter().map(|r| r.2.hypot(r.3)).collect();
    sorted.sort_by(f64::total_cmp);
    let rms = super::plain_rms(&sorted);
    let clipped = super::clipped_rms(&sorted, 3.0);
    eprintln!(
        "{label}: {} matched within {radius:.2} px, rms {rms:.3}, clipped {clipped:.3}, p50 {:.3}, p68 {:.3}, p90 {:.3}, p95 {:.3}, max {:.3}",
        residuals.len(),
        percentile_sorted(&sorted, 0.50),
        percentile_sorted(&sorted, 0.68),
        percentile_sorted(&sorted, 0.90),
        percentile_sorted(&sorted, 0.95),
        sorted.last().copied().unwrap_or(f64::NAN)
    );
    let cell_of = |value: f64, extent: usize| {
        ((value / extent as f64 * RESIDUAL_GRID as f64) as usize).min(RESIDUAL_GRID - 1)
    };
    for row in 0..RESIDUAL_GRID {
        let mut line = String::new();
        for col in 0..RESIDUAL_GRID {
            let cell: Vec<&ResidualRow> = residuals
                .iter()
                .filter(|r| cell_of(r.0, width) == col && cell_of(r.1, height) == row)
                .collect();
            let n = cell.len().max(1) as f64;
            let mean_dx = cell.iter().map(|r| r.2).sum::<f64>() / n;
            let mean_dy = cell.iter().map(|r| r.3).sum::<f64>() / n;
            let cell_rms = (cell.iter().map(|r| r.2 * r.2 + r.3 * r.3).sum::<f64>() / n).sqrt();
            line.push_str(&format!(
                " [{:>4} n, mean ({:+.2}, {:+.2}), rms {:.2}]",
                cell.len(),
                mean_dx,
                mean_dy,
                cell_rms
            ));
        }
        eprintln!("{label}: row {row}:{line}");
    }
    let mut by_mag: Vec<&ResidualRow> = residuals.iter().collect();
    by_mag.sort_by(|a, b| a.4.total_cmp(&b.4));
    for chunk in by_mag.chunks((by_mag.len() / 4).max(1)) {
        let n = chunk.len() as f64;
        let chunk_rms = (chunk.iter().map(|r| r.2 * r.2 + r.3 * r.3).sum::<f64>() / n).sqrt();
        eprintln!(
            "{label}: mag {:.1}..{:.1}: {} stars, rms {chunk_rms:.3}",
            chunk.first().map_or(f64::NAN, |r| r.4),
            chunk.last().map_or(f64::NAN, |r| r.4),
            chunk.len()
        );
    }
}

fn grid_error_report(
    label: &str,
    truth: &WcsTransform,
    cards: &[(String, String)],
    size: (usize, usize),
    region: [f64; 4],
) {
    let solved = wcs_from_cards(cards, size.0, size.1);
    let scale = truth.pixel_scale_arcsec();
    let (xc, yc) = ((region[0] + region[1]) / 2.0, (region[2] + region[3]) / 2.0);
    let (hx, hy) = ((region[1] - region[0]) / 2.0, (region[3] - region[2]) / 2.0);
    for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let mut worst: f64 = 0.0;
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let (x, y) = (xc + sx * fraction * hx, yc + sy * fraction * hy);
            let t = truth.pixel_to_world(x, y);
            let s = solved.pixel_to_world(x, y);
            let error = angular_separation(t.ra, t.dec, s.ra, s.dec) * ARCSEC_PER_DEG / scale;
            worst = worst.max(error);
        }
        eprintln!("{label}: worst error at {fraction:.2} of the half extent: {worst:.3} px");
    }
}

fn stage_report(
    setup: &TierSetup,
    fraction: f64,
    detections: &[super::Detection],
    usable: &[super::Detection],
    saturated: usize,
    hint_centre: (f64, f64),
    nominal: f64,
    epoch: Option<f64>,
    truth: &WcsTransform,
    size: (usize, usize),
    params: &LocalSolveParams,
) {
    let (width, height) = size;
    let scale_deg = nominal / ARCSEC_PER_DEG;
    let field_w = width as f64 * scale_deg;
    let field_h = height as f64 * scale_deg;
    let inlier_px = super::ransac_inlier_px(width.max(height), field_w.max(field_h));
    let disc_px = fraction * width.min(height) as f64;
    let points = super::disc_points(usable, size, disc_px);
    let cone_radius_deg = fraction * field_w.min(field_h);
    let cone_area = std::f64::consts::PI * cone_radius_deg * cone_radius_deg;
    let bright_skip = (saturated as f64 * cone_area / (field_w * field_h)).round() as usize;
    let side = super::MatchSide::new(&points, params.hinted_match_stars, inlier_px);
    let (mut rows, _) = setup
        .provider
        .cone(hint_centre.0, hint_centre.1, cone_radius_deg, None)
        .unwrap_or_else(|error| panic!("{}: {error}", setup.label));
    if let Some(epoch) = epoch {
        propagate_epoch(&mut rows, epoch);
    }
    let sky = super::ranked_sky(
        &rows,
        bright_skip,
        params.catalogue_stars,
        setup.provider.ranking_key(),
    );
    let plane: Vec<(f64, f64)> = sky
        .iter()
        .filter_map(|&(ra, dec)| {
            super::tan_plane(hint_centre, ra, dec).map(|(x, y)| (-x / scale_deg, y / scale_deg))
        })
        .collect();
    let (xc, yc) = ((width as f64 - 1.0) / 2.0, (height as f64 - 1.0) / 2.0);
    let first = sky.len().min(side.params.max_stars);
    let truth_px: Vec<(f64, f64)> = sky[..first]
        .iter()
        .map(|&(ra, dec)| truth.world_to_pixel(ra, dec))
        .collect();
    let in_disc = truth_px
        .iter()
        .filter(|&&(x, y)| (x - xc).hypot(y - yc) <= disc_px)
        .count();
    let coincidence = 3.0 * super::median_fwhm(usable).max(1.5);
    let near = |x: f64, y: f64, list: &[(f64, f64)]| {
        list.iter()
            .any(|&(sx, sy)| (sx - x).hypot(sy - y) <= coincidence)
    };
    let common = truth_px
        .iter()
        .filter(|&&(x, y)| near(x, y, &side.stars))
        .count();
    let all_px: Vec<(f64, f64)> = detections.iter().map(|d| (d.star.x, d.star.y)).collect();
    let usable_px: Vec<(f64, f64)> = usable.iter().map(|d| (d.star.x, d.star.y)).collect();
    let any_hit = truth_px
        .iter()
        .filter(|&&(x, y)| near(x, y, &all_px))
        .count();
    let usable_hit = truth_px
        .iter()
        .filter(|&&(x, y)| near(x, y, &usable_px))
        .count();
    let mut ranks: Vec<usize> = truth_px
        .iter()
        .filter_map(|&(x, y)| {
            usable
                .iter()
                .position(|d| (d.star.x - x).hypot(d.star.y - y) <= coincidence)
        })
        .collect();
    ranks.sort_unstable();
    let median_rank = ranks.get(ranks.len() / 2).copied();
    eprintln!(
        "[{}] disc {fraction}: of the first {first} catalogue stars {any_hit} coincide with any detection, {usable_hit} with a compact one (median flux rank {median_rank:?} of {}), {common} with one of the {} triangle stars",
        setup.label,
        usable.len(),
        side.stars.len()
    );
    let deep = super::ranked_sky(&rows, 0, 300, setup.provider.ranking_key());
    let mags: Vec<f64> = {
        let mut sorted: Vec<f64> = rows.iter().filter_map(|row| row.g).collect();
        sorted.sort_by(f64::total_cmp);
        sorted
    };
    for (block, chunk) in deep.chunks(60).enumerate() {
        let px: Vec<(f64, f64)> = chunk
            .iter()
            .map(|&(ra, dec)| truth.world_to_pixel(ra, dec))
            .collect();
        let inside = px
            .iter()
            .filter(|&&(x, y)| (x - xc).hypot(y - yc) <= disc_px)
            .count();
        let any = px.iter().filter(|&&(x, y)| near(x, y, &all_px)).count();
        let compact = px.iter().filter(|&&(x, y)| near(x, y, &usable_px)).count();
        let top = px.iter().filter(|&&(x, y)| near(x, y, &side.stars)).count();
        eprintln!(
            "[{}] disc {fraction}: catalogue ranks {}-{} (mag {:.1}..{:.1}): {inside} inside the disc, {any} with any detection, {compact} with a compact one, {top} among the triangle stars",
            setup.label,
            block * 60,
            block * 60 + chunk.len(),
            mags.get(block * 60).copied().unwrap_or(f64::NAN),
            mags.get(block * 60 + chunk.len() - 1).copied().unwrap_or(f64::NAN)
        );
    }
    let ecc_percentiles = |list: &[f64]| {
        let mut sorted = list.to_vec();
        sorted.sort_by(f64::total_cmp);
        [0.1, 0.25, 0.5, 0.75, 0.9]
            .iter()
            .map(|&p| format!("{:.2}", percentile_sorted(&sorted, p)))
            .collect::<Vec<String>>()
            .join(" ")
    };
    let all_ecc: Vec<f64> = detections.iter().map(|d| d.star.eccentricity).collect();
    let mut by_flux: Vec<&super::Detection> = detections.iter().collect();
    by_flux.sort_by(|a, b| b.star.flux.total_cmp(&a.star.flux));
    let bright_ecc: Vec<f64> = by_flux
        .iter()
        .take(300)
        .map(|d| d.star.eccentricity)
        .collect();
    let star_ecc: Vec<f64> = deep
        .iter()
        .skip(60)
        .filter_map(|&(ra, dec)| {
            let (x, y) = truth.world_to_pixel(ra, dec);
            detections
                .iter()
                .filter(|d| (d.star.x - x).hypot(d.star.y - y) <= coincidence)
                .map(|d| d.star.eccentricity)
                .min_by(f64::total_cmp)
        })
        .collect();
    eprintln!(
        "[{}] eccentricity p10/p25/p50/p75/p90: all {} | brightest 300 {} | catalogue stars of ranks 60-300 {}",
        setup.label,
        ecc_percentiles(&all_ecc),
        ecc_percentiles(&bright_ecc),
        ecc_percentiles(&star_ecc)
    );
    let (same, mirrored) = side.match_catalogue(&plane);
    let loose = TriangleParams {
        min_inliers: 3,
        min_inlier_ratio: 0.0,
        max_residual_px: f64::INFINITY,
        ..side.params
    };
    let count_inliers = |m: &TriangleMatch| {
        ransac_affine(&m.pairs, AffineAlignMethod::Affine, &loose)
            .map_or((0, f64::NAN), |fit| (fit.inliers, fit.residual_px))
    };
    let strict = |m: &TriangleMatch| {
        ransac_affine(&m.pairs, AffineAlignMethod::Affine, &side.params)
            .map(|fit| (fit.inliers, fit.residual_px))
    };
    eprintln!(
        "[{}] disc {fraction}: strict ransac same {:?} mirrored {:?} (max residual {:.2} px)",
        setup.label,
        strict(&same),
        strict(&mirrored),
        side.params.max_residual_px
    );
    let window = ((side.params.max_stars as f64 * super::MATCH_WINDOW_FRACTION).round() as usize)
        .max(side.params.min_inliers);
    let step = (window / 2).max(1);
    for index in 0..super::MATCH_WINDOWS {
        let start = index * step;
        if start + side.params.min_inliers > plane.len() {
            break;
        }
        let end = (start + window).min(plane.len());
        let (ws, wm) = side.match_catalogue(&plane[start..end]);
        eprintln!(
            "[{}] disc {fraction}: window {start}-{end}: loose same {:?} mirrored {:?}; strict same {:?} mirrored {:?}",
            setup.label,
            count_inliers(&ws),
            count_inliers(&wm),
            strict(&ws),
            strict(&wm)
        );
    }
    let windows = super::best_window_inliers(&side, &plane).map(|v| v.len());
    eprintln!(
        "[{}] disc {fraction} ({disc_px:.0} px, cone {cone_radius_deg:.3} deg): image side {} of {} disc stars, catalogue {} in cone, skip {bright_skip}, list {}; of the first {first} catalogue stars {in_disc} inside the disc and {common} coincide with an image star; same {} pairs {} votes {} inliers, mirrored {} pairs {} votes {} inliers (inlier radius {inlier_px:.2} px); windows {windows:?}",
        setup.label,
        side.stars.len(),
        points.len(),
        rows.len(),
        sky.len(),
        same.pairs.len(),
        same.votes,
        count_inliers(&same).0,
        mirrored.pairs.len(),
        mirrored.votes,
        count_inliers(&mirrored).0
    );
}

#[test]
#[ignore]
fn corpus_single_frame_stage_diagnostics() {
    let path = PathBuf::from(
        std::env::var_os(ENV_FRAME).unwrap_or_else(|| panic!("set {ENV_FRAME}")),
    );
    let offset_deg = env_f64(ENV_FRAME_OFFSET, 0.0);
    let bearing_deg = env_f64(ENV_FRAME_BEARING, 0.0);
    let fractions: Vec<f64> = std::env::var(ENV_FRAME_FRACTIONS)
        .unwrap_or_else(|_| "0.5,0.25".to_string())
        .split(',')
        .filter_map(|value| value.trim().parse::<f64>().ok())
        .collect();
    let residuals_wanted = std::env::var_os(ENV_FRAME_RESIDUALS).is_some();
    let params = LocalSolveParams {
        max_side_px: env_f64("ASTROBURST_SOLVE_MAX_SIDE", 2048.0) as usize,
        ..LocalSolveParams::default()
    };
    let loaded = load_frame(&path);
    let truth = loaded.truth.as_ref().expect("the frame has a truth WCS");
    let (height, width) = loaded.image.dim();
    let size = (width, height);
    let file = path.file_name().unwrap().to_string_lossy().to_string();
    let entry = manifest_entry_for(&path).unwrap_or(FrameEntry {
        file,
        glat_deg: None,
        native: false,
        region: None,
        band: None,
        note: None,
        bytes: None,
        sha256: None,
        request: None,
        header: None,
    });
    let meta = frame_meta(entry.file.clone(), &entry, truth, size);
    let region = meta.region;
    let centre = truth_centre(truth, region);
    let hint_centre = destination(centre, bearing_deg, offset_deg);
    let scale = truth.pixel_scale_arcsec();
    let epoch = observation_epoch_year(&loaded.header);
    let hints = hints_around(hint_centre, scale, width.max(height), epoch, 7);
    let scale_hint = hints.scale.as_ref().unwrap();
    let nominal = (scale_hint.low_arcsec * scale_hint.high_arcsec).sqrt();
    let detections = super::detect_for_solve(&loaded.image, &hints, &params);
    let usable = super::usable_detections(&detections, &params);
    let saturated = super::dropped_saturated(&detections, &params);
    eprintln!(
        "{}: {width}x{height} fov {:.3} deg scale {scale:.4} hint off {offset_deg} deg at bearing {bearing_deg}, nominal {nominal:.4}, detections {} usable {} saturated {saturated} flagged {} fwhm {:.2}",
        meta.name,
        meta.fov_deg,
        detections.len(),
        usable.len(),
        detections.iter().filter(|d| d.saturated).count(),
        super::median_fwhm(&usable)
    );
    let plateau_stats = |label: &str, image: &Array2<f32>| {
        let mut values: Vec<f32> = image.iter().copied().filter(|v| v.is_finite()).collect();
        values.sort_by(|a, b| b.total_cmp(a));
        let max = values.first().copied().unwrap_or(f32::NAN);
        let within = values.iter().filter(|&&v| v >= 0.98 * max).count();
        let top: Vec<String> = values.iter().take(12).map(|v| format!("{v:.0}")).collect();
        eprintln!(
            "{label}: max {max:.1}, {within} pixels within 2 % of it, level {:?}, top values {}",
            super::saturation_level(image),
            top.join(" ")
        );
    };
    plateau_stats("unbinned", &loaded.image);
    let factor = width.max(height).div_ceil(params.max_side_px.max(1));
    if factor > 1 {
        plateau_stats(
            &format!("binned x{factor}"),
            &super::block_mean(&loaded.image, factor),
        );
    }
    let limit = super::compactness_limit(&detections, &params);
    eprintln!("compactness limit {limit:.3}");
    let mut peaks: Vec<(f64, f64, bool)> = detections
        .iter()
        .map(|d| (d.star.peak, d.star.eccentricity, super::is_compact(d, limit)))
        .collect();
    peaks.sort_by(|a, b| b.0.total_cmp(&a.0));
    let brightest: Vec<String> = peaks
        .iter()
        .take(16)
        .map(|(peak, ecc, compact)| format!("{peak:.0}/{ecc:.2}{}", if *compact { "" } else { "x" }))
        .collect();
    eprintln!(
        "brightest detection peaks above background (peak/eccentricity, x = dropped): {}",
        brightest.join(" ")
    );
    let tiers: Vec<TierSetup> = [(ENV_TIER0, Tier::Zero), (ENV_TIER1, Tier::One)]
        .into_iter()
        .filter(|(env, _)| std::env::var_os(env).is_some())
        .map(|(_, tier)| tier_setup(tier))
        .collect();
    for setup in &tiers {
        for &fraction in &fractions {
            stage_report(
                setup,
                fraction,
                &detections,
                &usable,
                saturated,
                hint_centre,
                nominal,
                epoch,
                truth,
                size,
                &params,
            );
        }
        let outcome = run_frame(
            "c1",
            frame_meta(entry.file.clone(), &entry, truth, size),
            &loaded,
            &hints,
            setup,
        );
        if let Some(accuracy) = &outcome.solved {
            eprintln!(
                "[{}] solved centre {:.3} px, worst corner {:.3} px",
                setup.label, accuracy.centre_px, accuracy.worst_corner_px
            );
        }
        if !residuals_wanted {
            continue;
        }
        let open = LocalSolveParams {
            max_rms_px: 10.0,
            ..params.clone()
        };
        let result = solve_local(
            &loaded.image,
            &hints,
            setup.provider.as_ref(),
            &open,
            &never_cancelled,
            &no_stage,
        );
        let half_diagonal_deg =
            0.5 * (width as f64).hypot(height as f64) * scale / ARCSEC_PER_DEG;
        let (mut rows, _) = setup
            .provider
            .cone(centre.0, centre.1, 1.2 * half_diagonal_deg, None)
            .unwrap_or_else(|error| panic!("{}: {error}", setup.label));
        if let Some(epoch) = epoch {
            propagate_epoch(&mut rows, epoch);
        }
        let radius = super::match_radius(&usable, &params);
        let label = format!("[{}] truth", setup.label);
        residual_report(&label, truth, &rows, &usable, size, radius);
        match result {
            Ok(solution) => {
                eprintln!(
                    "[{}] open gate: matched {}/{} rms {:.3} sip {:?} log10 P {:.1}",
                    setup.label,
                    solution.matched,
                    solution.predicted,
                    solution.rms_px,
                    solution.sip_order,
                    solution.false_alarm_log10
                );
                let solved = wcs_from_cards(&solution.wcs_cards, width, height);
                let label = format!("[{}] solved", setup.label);
                residual_report(&label, &solved, &rows, &usable, size, radius);
                grid_error_report(&label, truth, &solution.wcs_cards, size, region);
                let selected = super::spread_detections(&usable, size, params.detection_stars);
                let pairs: Vec<super::SkyPair> = rows
                    .iter()
                    .filter_map(|row| {
                        let (px, py) = solved.world_to_pixel(row.ra_epoch, row.dec_epoch);
                        if !(px.is_finite() && py.is_finite() && super::inside(size, (px, py))) {
                            return None;
                        }
                        selected
                            .iter()
                            .map(|d| (d, (d.star.x - px).hypot(d.star.y - py)))
                            .filter(|(_, distance)| *distance <= radius)
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(d, _)| super::SkyPair {
                                x: d.star.x,
                                y: d.star.y,
                                ra: row.ra_epoch,
                                dec: row.dec_epoch,
                            })
                    })
                    .collect();
                match super::fit_sip_model(&pairs, &solution.tan, size, &params) {
                    Some(selection) => eprintln!(
                        "{label}: SIP refit on {} pairs: considered (order, holdout px, bic) {:?}, chosen {:?}",
                        pairs.len(),
                        selection.considered,
                        selection.model.as_ref().map(|m| m.forward.order)
                    ),
                    None => eprintln!("{label}: SIP refit on {} pairs: none", pairs.len()),
                }
            }
            Err(failure) => eprintln!("[{}] open gate: {failure:?}", setup.label),
        }
    }
}

const ENV_MC_SET: &str = "ASTROBURST_SOLVE_MC_SET";
const ENV_MC_OFFSET_DEG: &str = "ASTROBURST_SOLVE_MC_OFFSET_DEG";
const ENV_MC_OFFSET_FIELDS: &str = "ASTROBURST_SOLVE_MC_OFFSET_FIELDS";
const ENV_MC_BEARINGS: &str = "ASTROBURST_SOLVE_MC_BEARINGS";
const ENV_MC_BIASES: &str = "ASTROBURST_SOLVE_MC_BIASES";
const ENV_MC_ONLY: &str = "ASTROBURST_SOLVE_MC_ONLY";
const MC_FIRST_BEARING_DEG: f64 = 17.0;

fn hints_with_bias(
    centre: (f64, f64),
    scale_arcsec: f64,
    long_side_px: usize,
    epoch_year: Option<f64>,
    bias: f64,
) -> SolveHints {
    let nominal = scale_arcsec * (1.0 + bias);
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

#[test]
#[ignore]
fn corpus_hint_monte_carlo_counts_solves_and_false_positives() {
    let root = corpus_root();
    let set = std::env::var(ENV_MC_SET).unwrap_or_else(|_| "c2".to_string());
    let set_name: &'static str = if set == "c1" { "c1" } else { "c2" };
    let bearings = env_f64(ENV_MC_BEARINGS, 4.0).max(1.0) as usize;
    let offset_fields = std::env::var(ENV_MC_OFFSET_FIELDS)
        .ok()
        .and_then(|value| value.parse::<f64>().ok());
    let offset_deg = env_f64(ENV_MC_OFFSET_DEG, 2.0);
    let biases: Vec<f64> = std::env::var(ENV_MC_BIASES)
        .unwrap_or_else(|_| "-0.05,-0.025,0,0.025,0.05".to_string())
        .split(',')
        .filter_map(|value| value.trim().parse::<f64>().ok())
        .collect();
    let only: Option<Vec<String>> = std::env::var(ENV_MC_ONLY)
        .ok()
        .map(|list| list.split(',').map(|name| name.trim().to_string()).collect());
    let tiers: Vec<TierSetup> = [(ENV_TIER0, Tier::Zero), (ENV_TIER1, Tier::One)]
        .into_iter()
        .filter(|(env, _)| std::env::var_os(env).is_some())
        .map(|(_, tier)| tier_setup(tier))
        .collect();
    assert!(!tiers.is_empty(), "set {ENV_TIER0} and/or {ENV_TIER1}");
    let dir = root.join(&set);
    let manifest =
        read_manifest(&dir).unwrap_or_else(|| panic!("{}: no manifest.json", dir.display()));
    let mut totals = vec![(0usize, 0usize, 0usize, 0usize); tiers.len()];
    for entry in &manifest.frames {
        if only
            .as_ref()
            .is_some_and(|names| !names.iter().any(|name| *name == entry.file))
        {
            continue;
        }
        let path = dir.join(&entry.file);
        let loaded = load_frame(&path);
        let truth = loaded
            .truth
            .as_ref()
            .unwrap_or_else(|| panic!("{set} {}: the truth WCS does not parse", entry.file));
        let (height, width) = loaded.image.dim();
        let probe = frame_meta(entry.file.clone(), entry, truth, (width, height));
        let distance = offset_fields.map_or(offset_deg, |fields| fields * probe.fov_deg);
        let centre = truth_centre(truth, probe.region);
        for b in 0..bearings {
            let bearing = 360.0 * b as f64 / bearings as f64 + MC_FIRST_BEARING_DEG;
            let hint_centre = destination(centre, bearing, distance);
            for &bias in &biases {
                let hints = hints_with_bias(
                    hint_centre,
                    truth.pixel_scale_arcsec(),
                    width.max(height),
                    observation_epoch_year(&loaded.header),
                    bias,
                );
                for (t, setup) in tiers.iter().enumerate() {
                    let meta = frame_meta(entry.file.clone(), entry, truth, (width, height));
                    let outcome = run_frame(set_name, meta, &loaded, &hints, setup);
                    totals[t].0 += 1;
                    if let Some(accuracy) = &outcome.solved {
                        totals[t].1 += 1;
                        let target = if set_name == "c2" {
                            C2_MAX_CORNER_PX
                        } else {
                            accuracy.tolerance_arcsec / truth.pixel_scale_arcsec()
                        };
                        if accuracy.worst_corner_px <= target {
                            totals[t].2 += 1;
                        }
                        if outcome.is_false_positive() {
                            totals[t].3 += 1;
                            eprintln!(
                                "MC-FP [{}] {} bearing {bearing:.0} bias {bias:+.3}: {}",
                                setup.label,
                                entry.file,
                                outcome.line()
                            );
                        }
                    }
                }
            }
        }
    }
    for (t, setup) in tiers.iter().enumerate() {
        eprintln!(
            "MC [{}] set {set} offset {}: {} runs, {} solved, {} within the corner target, {} false positives",
            setup.label,
            offset_fields.map_or(format!("{offset_deg} deg"), |fields| format!("{fields} fields")),
            totals[t].0,
            totals[t].1,
            totals[t].2,
            totals[t].3
        );
    }
    let false_positives: usize = totals.iter().map(|t| t.3).sum();
    assert_eq!(false_positives, 0, "wrong solutions were accepted");
}
