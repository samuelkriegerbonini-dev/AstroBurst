use std::borrow::Cow;
use std::collections::HashMap;
#[cfg(test)]
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ndarray::{s, Array2, Axis};
use rayon::prelude::*;

use crate::core::alignment::affine::{AffineAlignMethod, AffineTransform};
use crate::core::alignment::triangles::{
    build_triangles, match_triangles, ransac_affine, select_alignment_stars, solve_3x3_ls,
    StarPair, TriangleDesc, TriangleMatch, TriangleParams,
};
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::catalog::{propagate_epoch, CatalogRow};
use crate::core::astrometry::gwcs::SipFitOptions;
#[cfg(test)]
use crate::core::astrometry::healpix_cat::{HealpixCatalog, MmapSource};
use crate::core::astrometry::solve_hints::{SolveHints, SCALE_HINT_MARGIN};
use crate::core::imaging::cfa::luminance_bin;
use crate::core::registration::poly::{self, PolyModel, TanParams};
use crate::core::stacking::{CancelCheck, StageCallback};
use crate::math::sigma_clipped_stats;

#[cfg(test)]
mod acceptance;
#[cfg(test)]
mod bench;
#[cfg(test)]
pub(crate) mod synthetic;
#[cfg(test)]
mod tests;

pub const MAX_NEAR_RADIUS_DEG: f64 = 15.0;
pub const TIER0_MEAN_STARS_PER_DEG2: f64 = 62.0;
pub const TIER0_MIN_EXPECTED_STARS: f64 = 50.0;

pub const STAGE_DETECT: &str = "detect";
const STAGE_SEARCH: &str = "search";
const STAGE_FIT: &str = "fit";
const TAN_HYPOTHESIS_PAIRS: usize = 3;
const ARCSEC_PER_DEG: f64 = 3600.0;
const TAN_LON_POLE: f64 = 180.0;
const MIN_TAN_PAIRS: usize = 3;
const CLIP_FLOOR_PX: f64 = 1e-3;
const MAX_CLIP_ROUNDS: usize = 20;
const CRVAL_CONVERGED_DEG: f64 = 1e-13;
const MAX_CRVAL_ITERATIONS: usize = 25;
const POISSON_TERM_CUTOFF: f64 = 1e-18;
const POISSON_MAX_TERMS: usize = 100_000;
const EDGE_MARGIN_PX: f64 = 3.0;
const EDGE_MARGIN_FWHM: f64 = 1.5;
const SATURATION_FRACTION: f64 = 0.98;
const SATURATION_PLATEAU_PIXELS: usize = 4;
const RANSAC_INLIER_FIELD_FACTOR: f64 = 0.1;
const REFINE_PASSES: usize = 2;
const SIP_FOLDS: usize = 5;
const SIP_PASSES: usize = 6;
const SIP_PASS_RADIUS_GROWTH: f64 = 2.0;
const SIP_PASS_MAX_RADIUS_FRACTION: f64 = 0.01;
const RADIAL_RMS_PER_MEDIAN: f64 = 1.201_122_408_786_45;
const MATCH_WINDOWS: usize = 4;
const MATCH_WINDOW_FRACTION: f64 = 2.0 / 3.0;
const SIP_RADIAL_BANDS: usize = 3;
const COVERAGE_GRID: usize = 3;
const COVERAGE_MIN_LOG10: f64 = -4.0;
const BACKGROUND_TILE_MIN_PIXELS: usize = 8;
const BACKGROUND_CLIP_SIGMA: f32 = 3.0;
const BACKGROUND_CLIP_ROUNDS: usize = 2;
const SCALE_WINDOW_TOLERANCE: f64 = 1e-9;
const MAX_TAN_FIELD_DEG: f64 = 120.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankingKey {
    pub colour_slope: f64,
    pub colour_range: (f64, f64),
}

impl RankingKey {
    pub const MAGNITUDE: RankingKey = RankingKey {
        colour_slope: 0.0,
        colour_range: (0.0, 0.0),
    };
    pub const TWO_MASS_J_KS: RankingKey = RankingKey {
        colour_slope: 2.0,
        colour_range: (-0.2, 1.5),
    };

    pub fn of(&self, row: &CatalogRow) -> Option<f64> {
        let mag = row.g.filter(|g| g.is_finite())?;
        let colour = row
            .bp_rp
            .filter(|c| c.is_finite())
            .map_or(0.0, |c| c.clamp(self.colour_range.0, self.colour_range.1));
        Some(mag + self.colour_slope * colour)
    }
}

pub trait CatalogProvider: Send + Sync {
    fn label(&self) -> &'static str;

    fn cone(&self, ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Result<Vec<CatalogRow>, String>;

    fn ranking_key(&self) -> RankingKey {
        RankingKey::MAGNITUDE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogTier {
    Tier0,
    Tier1,
    Tier2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TierSpec {
    pub tier: CatalogTier,
    pub label: &'static str,
    pub ranking: RankingKey,
}

pub const TYCHO2_TIER: TierSpec = TierSpec {
    tier: CatalogTier::Tier0,
    label: "tier0-tycho2",
    ranking: RankingKey::MAGNITUDE,
};
pub const SIRIL_GAIA_TIER: TierSpec = TierSpec {
    tier: CatalogTier::Tier1,
    label: "tier1-gaia-siril",
    ranking: RankingKey::MAGNITUDE,
};
pub const TWOMASS_TIER: TierSpec = TierSpec {
    tier: CatalogTier::Tier1,
    label: "tier1-2mass",
    ranking: RankingKey::TWO_MASS_J_KS,
};
pub const GAIA_ONLINE_TIER: TierSpec = TierSpec {
    tier: CatalogTier::Tier2,
    label: "tier2-gaia-vizier",
    ranking: RankingKey::MAGNITUDE,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldSize {
    pub width_deg: f64,
    pub height_deg: f64,
}

#[derive(Default)]
pub struct TierChain {
    pub tiers: Vec<(CatalogTier, Box<dyn CatalogProvider>)>,
}

pub fn expected_tier0_stars(field: FieldSize) -> f64 {
    TIER0_MEAN_STARS_PER_DEG2 * field.width_deg * field.height_deg
}

impl TierChain {
    pub fn choose(&self, field: FieldSize) -> Option<(CatalogTier, &dyn CatalogProvider)> {
        let tier0 = self.tier(CatalogTier::Tier0);
        if expected_tier0_stars(field) >= TIER0_MIN_EXPECTED_STARS {
            if let Some(provider) = tier0 {
                return Some((CatalogTier::Tier0, provider));
            }
        }
        self.tiers
            .iter()
            .find(|(tier, _)| *tier != CatalogTier::Tier0)
            .map(|(tier, provider)| (*tier, provider.as_ref()))
            .or_else(|| tier0.map(|provider| (CatalogTier::Tier0, provider)))
    }

    fn tier(&self, wanted: CatalogTier) -> Option<&dyn CatalogProvider> {
        self.tiers
            .iter()
            .find(|(tier, _)| *tier == wanted)
            .map(|(_, provider)| provider.as_ref())
    }
}

#[cfg(test)]
struct HealpixProvider {
    catalog: HealpixCatalog<MmapSource>,
    spec: TierSpec,
}

#[cfg(test)]
impl CatalogProvider for HealpixProvider {
    fn label(&self) -> &'static str {
        self.spec.label
    }

    fn cone(&self, ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Result<Vec<CatalogRow>, String> {
        self.catalog
            .cone(ra_deg, dec_deg, radius_deg, None)
            .map_err(|error| error.to_string())
    }

    fn ranking_key(&self) -> RankingKey {
        self.spec.ranking
    }
}

#[cfg(test)]
pub fn healpix_provider(path: &Path, spec: TierSpec) -> Result<Box<dyn CatalogProvider>, String> {
    let catalog = HealpixCatalog::open_file(path)
        .map_err(|error| format!("{} at {}: {error}", spec.label, path.display()))?;
    Ok(Box::new(HealpixProvider { catalog, spec }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageParity {
    Normal,
    Flipped,
}

#[derive(Debug, Clone)]
pub struct Detection {
    pub star: DetectedStar,
    pub saturated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyPair {
    pub x: f64,
    pub y: f64,
    pub ra: f64,
    pub dec: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TanFit {
    pub tan: TanParams,
    pub rms_px: f64,
    pub inliers: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verification {
    pub matched: usize,
    pub predicted: usize,
    pub detections: usize,
    pub radius_px: f64,
    pub lambda: f64,
    pub false_alarm_log10: f64,
    pub rms_px: f64,
    pub coverage_log10: f64,
    pub coverage_chance_log10: f64,
    pub corners_finite: bool,
    pub scale_in_range: bool,
    pub accepted: bool,
}

impl Verification {
    pub fn covered(&self) -> bool {
        cell_covered(self.coverage_log10, self.coverage_chance_log10)
    }
}

fn cell_covered(coverage_log10: f64, chance_log10: f64) -> bool {
    coverage_log10 >= COVERAGE_MIN_LOG10 || chance_log10 < COVERAGE_MIN_LOG10
}

#[derive(Debug, Clone, PartialEq)]
pub struct DistortionFit {
    pub order: u8,
    pub considered: Vec<(u8, f64, f64)>,
    pub wcs_cards: Vec<(String, String)>,
    pub sip_max_err_px: Option<f64>,
    pub tan: TanParams,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalSolveParams {
    pub near_radius_deg: f64,
    pub near_radius_fields: f64,
    pub max_centres: usize,
    pub max_side_px: usize,
    pub background_tile_px: usize,
    pub detect_sigma: f64,
    pub max_eccentricity: f64,
    pub compact_quantile: f64,
    pub detection_stars: usize,
    pub catalogue_stars: usize,
    pub hinted_match_stars: usize,
    pub spiral_match_stars: usize,
    pub footprint_margin: f64,
    pub footprint_stars_per_detection: usize,
    pub footprint_min_stars: usize,
    pub cone_short_side_fraction: f64,
    pub spiral_spacing_fraction: f64,
    pub spiral_projective_px: f64,
    pub min_matches: usize,
    pub max_false_alarm: f64,
    pub max_rms_px: f64,
    pub tan_rms_px_per_deg: f64,
    pub match_radius_px: f64,
    pub match_radius_fwhm: f64,
    pub clip_sigma: f64,
    pub crval_iterations: usize,
    pub sip_min_inliers: usize,
    pub sip_min_quadrants: usize,
    pub sip_stars_per_band: usize,
    pub sip_order_4_above_deg: f64,
}

impl Default for LocalSolveParams {
    fn default() -> Self {
        Self {
            near_radius_deg: 3.0,
            near_radius_fields: 0.5,
            max_centres: 600,
            max_side_px: 2048,
            background_tile_px: 64,
            detect_sigma: 5.0,
            max_eccentricity: 0.6,
            compact_quantile: 0.75,
            detection_stars: 300,
            catalogue_stars: 120,
            hinted_match_stars: 60,
            spiral_match_stars: 40,
            footprint_margin: 1.2,
            footprint_stars_per_detection: 3,
            footprint_min_stars: 200,
            cone_short_side_fraction: 0.5,
            spiral_spacing_fraction: 0.5,
            spiral_projective_px: TriangleParams::SOLVER.inlier_px / 2.0,
            min_matches: 8,
            max_false_alarm: 1e-9,
            max_rms_px: 1.0,
            tan_rms_px_per_deg: 0.5,
            match_radius_px: 1.5,
            match_radius_fwhm: 1.5,
            clip_sigma: 3.0,
            crval_iterations: 3,
            sip_min_inliers: 40,
            sip_min_quadrants: 3,
            sip_stars_per_band: 300,
            sip_order_4_above_deg: 5.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalSolution {
    pub wcs_cards: Vec<(String, String)>,
    pub tan: TanParams,
    pub centre: (f64, f64),
    pub scale_arcsec: f64,
    pub rotation_deg: f64,
    pub parity: ImageParity,
    pub matched: usize,
    pub predicted: usize,
    pub rms_px: f64,
    pub false_alarm_log10: f64,
    pub sip_order: Option<u8>,
    pub sip_max_err_px: Option<f64>,
    pub detections: usize,
    pub centres_tried: usize,
    pub elapsed: Duration,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    MissingHint,
    TooFewStars,
    NoSolution,
    Cancelled,
    Catalog,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateNumbers {
    pub centre: (f64, f64),
    pub matched: usize,
    pub predicted: usize,
    pub rms_px: f64,
    pub false_alarm_log10: f64,
    pub coverage_log10: f64,
    pub coverage_chance_log10: f64,
}

impl CandidateNumbers {
    pub fn covered(&self) -> bool {
        cell_covered(self.coverage_log10, self.coverage_chance_log10)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolveFailure {
    pub reason: FailureReason,
    pub message: String,
    pub detections: usize,
    pub centres_tried: usize,
    pub best: Option<Box<CandidateNumbers>>,
    pub elapsed: Duration,
}

pub fn solve_local(
    image: &Array2<f32>,
    hints: &SolveHints,
    catalog: &dyn CatalogProvider,
    params: &LocalSolveParams,
    cancel: CancelCheck,
    stage: StageCallback,
) -> Result<LocalSolution, SolveFailure> {
    let started = Instant::now();
    stage(STAGE_DETECT);
    let (height, width) = image.dim();
    let detections = detect_for_solve(image, hints, params);
    solve_detections(
        started,
        &detections,
        (width, height),
        hints,
        catalog,
        params,
        cancel,
        stage,
    )
}

pub fn solve_stars(
    detections: &[Detection],
    size: (usize, usize),
    hints: &SolveHints,
    catalog: &dyn CatalogProvider,
    params: &LocalSolveParams,
    cancel: CancelCheck,
    stage: StageCallback,
) -> Result<LocalSolution, SolveFailure> {
    solve_detections(
        Instant::now(),
        detections,
        size,
        hints,
        catalog,
        params,
        cancel,
        stage,
    )
}

fn ring_step_deg(spacing_deg: f64) -> f64 {
    spacing_deg * 3f64.sqrt() / 2.0
}

fn last_ring(spacing_deg: f64, radius_deg: f64) -> usize {
    if !is_positive(spacing_deg) || !is_positive(radius_deg) {
        return 0;
    }
    (radius_deg / ring_step_deg(spacing_deg)).ceil() as usize
}

fn spiral_ring(hint: (f64, f64), spacing_deg: f64, k: usize) -> Vec<(f64, f64)> {
    let mut ring = Vec::with_capacity(6 * k);
    for corner in 0..6 {
        let (sin0, cos0) = (corner as f64 * 60.0f64).to_radians().sin_cos();
        let (sin1, cos1) = ((corner + 1) as f64 * 60.0f64).to_radians().sin_cos();
        let reach = k as f64 * spacing_deg;
        let (x0, y0) = (reach * cos0, reach * sin0);
        let (x1, y1) = (reach * cos1, reach * sin1);
        for step in 0..k {
            let t = step as f64 / k as f64;
            let (x, y) = (x0 + t * (x1 - x0), y0 + t * (y1 - y0));
            let bearing = x.atan2(y).to_degrees();
            ring.push(destination(hint, bearing, x.hypot(y)));
        }
    }
    ring
}

#[cfg(test)]
fn spiral_centres(hint: (f64, f64), spacing_deg: f64, radius_deg: f64) -> Vec<Vec<(f64, f64)>> {
    let mut rings = vec![vec![hint]];
    for k in 1..=last_ring(spacing_deg, radius_deg) {
        rings.push(spiral_ring(hint, spacing_deg, k));
    }
    rings
}

#[derive(Debug, Clone, PartialEq)]
struct SearchPlan {
    rings: Vec<Vec<(usize, (f64, f64))>>,
    centres: usize,
    radius_reached_deg: f64,
    capped: bool,
}

fn search_plan(
    hint: (f64, f64),
    spacings_deg: &[f64],
    radius_deg: f64,
    max_centres: usize,
) -> SearchPlan {
    let first: Vec<(usize, (f64, f64))> = (0..spacings_deg.len()).map(|h| (h, hint)).collect();
    let mut centres = first.len();
    let mut rings = vec![first];
    let mut reached = vec![0.0f64; spacings_deg.len()];
    let mut capped = false;
    for k in 1.. {
        let mut ring = Vec::new();
        for (h, &spacing) in spacings_deg.iter().enumerate() {
            if k <= last_ring(spacing, radius_deg) {
                ring.extend(spiral_ring(hint, spacing, k).into_iter().map(|c| (h, c)));
            }
        }
        if ring.is_empty() {
            break;
        }
        if centres + ring.len() > max_centres {
            capped = true;
            break;
        }
        centres += ring.len();
        for &(h, _) in &ring {
            reached[h] = radius_deg.min(k as f64 * ring_step_deg(spacings_deg[h]));
        }
        rings.push(ring);
    }
    let radius_reached_deg = if capped {
        reached.iter().copied().fold(f64::INFINITY, f64::min)
    } else {
        radius_deg.max(0.0)
    };
    SearchPlan {
        rings,
        centres,
        radius_reached_deg,
        capped,
    }
}

fn scale_hypotheses(low_arcsec: f64, high_arcsec: f64) -> Vec<f64> {
    let (below, above) = (1.0 - SCALE_HINT_MARGIN, 1.0 + SCALE_HINT_MARGIN);
    let window = above / below;
    let single = high_arcsec / low_arcsec <= window * (1.0 + SCALE_WINDOW_TOLERANCE);
    if !is_positive(low_arcsec) || !is_positive(high_arcsec) || single {
        return vec![(low_arcsec * high_arcsec / (below * above)).sqrt()];
    }
    let first = low_arcsec / below;
    let last = high_arcsec / above;
    let count = ((high_arcsec / low_arcsec).ln() / window.ln())
        .ceil()
        .max(2.0) as usize;
    (0..count)
        .map(|i| first * (last / first).powf(i as f64 / (count - 1) as f64))
        .collect()
}

pub(crate) fn destination(
    (ra, dec): (f64, f64),
    bearing_deg: f64,
    distance_deg: f64,
) -> (f64, f64) {
    let (sin_ra, cos_ra) = ra.to_radians().sin_cos();
    let (sin_dec, cos_dec) = dec.to_radians().sin_cos();
    let (sin_bearing, cos_bearing) = bearing_deg.to_radians().sin_cos();
    let (sin_dist, cos_dist) = distance_deg.to_radians().sin_cos();
    let north = sin_dist * cos_bearing;
    let east = sin_dist * sin_bearing;
    let x = cos_dist * cos_dec * cos_ra - north * sin_dec * cos_ra - east * sin_ra;
    let y = cos_dist * cos_dec * sin_ra - north * sin_dec * sin_ra + east * cos_ra;
    let z = cos_dist * sin_dec + north * cos_dec;
    (
        wrap_longitude_deg(y.atan2(x).to_degrees()),
        z.atan2(x.hypot(y)).to_degrees(),
    )
}

fn wrap_longitude_deg(value: f64) -> f64 {
    let wrapped = value.rem_euclid(360.0);
    if wrapped >= 360.0 {
        0.0
    } else {
        wrapped
    }
}

fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn centre_crpix((width, height): (usize, usize)) -> (f64, f64) {
    ((width as f64 + 1.0) / 2.0, (height as f64 + 1.0) / 2.0)
}

fn tan_plane((ra0, dec0): (f64, f64), ra: f64, dec: f64) -> Option<(f64, f64)> {
    let (sin_dec, cos_dec) = dec.to_radians().sin_cos();
    let (sin_dec0, cos_dec0) = dec0.to_radians().sin_cos();
    let (sin_dra, cos_dra) = (ra - ra0).to_radians().sin_cos();
    let depth = sin_dec * sin_dec0 + cos_dec * cos_dec0 * cos_dra;
    if !is_positive(depth) {
        return None;
    }
    let x = cos_dec * sin_dra / depth;
    let y = (sin_dec * cos_dec0 - cos_dec * sin_dec0 * cos_dra) / depth;
    Some((x.to_degrees(), y.to_degrees()))
}

fn tan_sky((ra0, dec0): (f64, f64), x_deg: f64, y_deg: f64) -> (f64, f64) {
    let (x, y) = (x_deg.to_radians(), y_deg.to_radians());
    let (sin_dec0, cos_dec0) = dec0.to_radians().sin_cos();
    let norm = (1.0 + x * x + y * y).sqrt();
    let dec = ((sin_dec0 + y * cos_dec0) / norm).clamp(-1.0, 1.0).asin();
    let ra = ra0.to_radians() + x.atan2(cos_dec0 - y * sin_dec0);
    (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
}

fn cd_inverse(cd: &[[f64; 2]; 2]) -> Option<[[f64; 2]; 2]> {
    let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
    if !det.is_finite() || det == 0.0 {
        return None;
    }
    Some([
        [cd[1][1] / det, -cd[0][1] / det],
        [-cd[1][0] / det, cd[0][0] / det],
    ])
}

fn pixel_to_sky(tan: &TanParams, x: f64, y: f64) -> (f64, f64) {
    let dx = x + 1.0 - tan.crpix.0;
    let dy = y + 1.0 - tan.crpix.1;
    let plane_x = tan.cd[0][0] * dx + tan.cd[0][1] * dy;
    let plane_y = tan.cd[1][0] * dx + tan.cd[1][1] * dy;
    tan_sky(tan.crval, plane_x, plane_y)
}

fn sky_to_pixel(tan: &TanParams, ra: f64, dec: f64) -> Option<(f64, f64)> {
    let (plane_x, plane_y) = tan_plane(tan.crval, ra, dec)?;
    let inverse = cd_inverse(&tan.cd)?;
    let dx = inverse[0][0] * plane_x + inverse[0][1] * plane_y;
    let dy = inverse[1][0] * plane_x + inverse[1][1] * plane_y;
    Some((dx + tan.crpix.0 - 1.0, dy + tan.crpix.1 - 1.0))
}

fn scale_arcsec(cd: &[[f64; 2]; 2]) -> f64 {
    (cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]).abs().sqrt() * ARCSEC_PER_DEG
}

fn rotation_deg(cd: &[[f64; 2]; 2]) -> f64 {
    (-cd[0][1]).atan2(cd[1][1]).to_degrees()
}

fn parity_of(cd: &[[f64; 2]; 2]) -> ImageParity {
    if cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0] > 0.0 {
        ImageParity::Flipped
    } else {
        ImageParity::Normal
    }
}

fn tan_cards(tan: &TanParams) -> Vec<(String, String)> {
    [
        ("WCSAXES", "2".to_string()),
        ("CTYPE1", "RA---TAN".to_string()),
        ("CTYPE2", "DEC--TAN".to_string()),
        ("CRPIX1", tan.crpix.0.to_string()),
        ("CRPIX2", tan.crpix.1.to_string()),
        ("CRVAL1", tan.crval.0.to_string()),
        ("CRVAL2", tan.crval.1.to_string()),
        ("CUNIT1", "deg".to_string()),
        ("CUNIT2", "deg".to_string()),
        ("CD1_1", tan.cd[0][0].to_string()),
        ("CD1_2", tan.cd[0][1].to_string()),
        ("CD2_1", tan.cd[1][0].to_string()),
        ("CD2_2", tan.cd[1][1].to_string()),
        ("LONPOLE", TAN_LON_POLE.to_string()),
        ("RADESYS", "ICRS".to_string()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect()
}

struct LinearFit {
    m: [[f64; 2]; 2],
    c: (f64, f64),
}

impl LinearFit {
    fn predict(&self, plane: (f64, f64)) -> (f64, f64) {
        (
            self.m[0][0] * plane.0 + self.m[0][1] * plane.1 + self.c.0,
            self.m[1][0] * plane.0 + self.m[1][1] * plane.1 + self.c.1,
        )
    }
}

fn pixel_offset(pair: &SkyPair, crpix: (f64, f64)) -> (f64, f64) {
    (pair.x + 1.0 - crpix.0, pair.y + 1.0 - crpix.1)
}

fn linear_fit(
    pairs: &[SkyPair],
    mask: &[bool],
    crpix: (f64, f64),
    crval: (f64, f64),
) -> Option<LinearFit> {
    let rows: Vec<StarPair> = pairs
        .iter()
        .zip(mask)
        .filter(|(_, &kept)| kept)
        .filter_map(|(pair, _)| {
            let (plane_x, plane_y) = tan_plane(crval, pair.ra, pair.dec)?;
            let (dx, dy) = pixel_offset(pair, crpix);
            Some((plane_x, plane_y, dx, dy))
        })
        .collect();
    if rows.len() < MIN_TAN_PAIRS {
        return None;
    }
    let ((m11, m12), cx) = solve_3x3_ls(&rows, true)?;
    let ((m21, m22), cy) = solve_3x3_ls(&rows, false)?;
    Some(LinearFit {
        m: [[m11, m12], [m21, m22]],
        c: (cx, cy),
    })
}

fn tan_pass(
    pairs: &[SkyPair],
    mask: &[bool],
    crpix: (f64, f64),
    start: (f64, f64),
    iterations: usize,
) -> Option<(TanParams, Vec<f64>)> {
    let mut crval = start;
    let mut fit = linear_fit(pairs, mask, crpix, crval)?;
    for iteration in 0..MAX_CRVAL_ITERATIONS {
        let cd = cd_inverse(&fit.m)?;
        let centre_x = -(cd[0][0] * fit.c.0 + cd[0][1] * fit.c.1);
        let centre_y = -(cd[1][0] * fit.c.0 + cd[1][1] * fit.c.1);
        let shift = centre_x.hypot(centre_y);
        if iteration >= iterations && shift < CRVAL_CONVERGED_DEG {
            break;
        }
        crval = tan_sky(crval, centre_x, centre_y);
        fit = linear_fit(pairs, mask, crpix, crval)?;
    }
    let cd = cd_inverse(&fit.m)?;
    let residuals = pairs
        .iter()
        .map(|pair| match tan_plane(crval, pair.ra, pair.dec) {
            Some(plane) => {
                let (px, py) = fit.predict(plane);
                let (dx, dy) = pixel_offset(pair, crpix);
                (px - dx).hypot(py - dy)
            }
            None => f64::INFINITY,
        })
        .collect();
    Some((TanParams { crval, crpix, cd }, residuals))
}

fn median_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

fn plain_rms(distances: &[f64]) -> f64 {
    if distances.is_empty() {
        0.0
    } else {
        (distances.iter().map(|d| d * d).sum::<f64>() / distances.len() as f64).sqrt()
    }
}

fn clipped_rms(distances: &[f64], clip_sigma: f64) -> f64 {
    if distances.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = distances
        .iter()
        .copied()
        .filter(|d| d.is_finite())
        .collect();
    if sorted.is_empty() {
        return f64::INFINITY;
    }
    sorted.sort_by(f64::total_cmp);
    let mut threshold =
        (clip_sigma * RADIAL_RMS_PER_MEDIAN * median_sorted(&sorted)).max(CLIP_FLOOR_PX);
    let mut rms = plain_rms(&sorted);
    for _ in 0..MAX_CLIP_ROUNDS {
        let kept: Vec<f64> = sorted
            .iter()
            .copied()
            .take_while(|d| *d <= threshold)
            .collect();
        if kept.is_empty() {
            break;
        }
        rms = plain_rms(&kept);
        let next = (clip_sigma * rms).max(CLIP_FLOOR_PX);
        if next >= threshold {
            break;
        }
        threshold = next;
    }
    rms
}

fn masked_rms(residuals: &[f64], mask: &[bool]) -> f64 {
    let (sum, count) = residuals
        .iter()
        .zip(mask)
        .filter(|(_, &kept)| kept)
        .fold((0.0, 0usize), |(sum, count), (r, _)| {
            (sum + r * r, count + 1)
        });
    if count == 0 {
        0.0
    } else {
        (sum / count as f64).sqrt()
    }
}

fn fit_tan(
    pairs: &[SkyPair],
    size: (usize, usize),
    start: (f64, f64),
    params: &LocalSolveParams,
) -> Option<TanFit> {
    let finite = pairs
        .iter()
        .all(|p| [p.x, p.y, p.ra, p.dec].iter().all(|v| v.is_finite()));
    if pairs.len() < MIN_TAN_PAIRS || !finite || !start.0.is_finite() || !start.1.is_finite() {
        return None;
    }
    let crpix = centre_crpix(size);
    let mut mask = vec![true; pairs.len()];
    let mut crval = start;
    for round in 0..MAX_CLIP_ROUNDS {
        let (tan, residuals) = tan_pass(pairs, &mask, crpix, crval, params.crval_iterations)?;
        crval = tan.crval;
        let rms = masked_rms(&residuals, &mask);
        let threshold = (params.clip_sigma * rms).max(CLIP_FLOOR_PX);
        let next: Vec<bool> = residuals.iter().map(|r| *r <= threshold).collect();
        let kept = next.iter().filter(|&&k| k).count();
        if next == mask || kept < MIN_TAN_PAIRS || round + 1 == MAX_CLIP_ROUNDS {
            return Some(TanFit {
                tan,
                rms_px: rms,
                inliers: mask,
            });
        }
        mask = next;
    }
    None
}

fn median_fwhm(detections: &[Detection]) -> f64 {
    let mut values: Vec<f64> = detections
        .iter()
        .map(|d| d.star.fwhm)
        .filter(|f| f.is_finite() && *f > 0.0)
        .collect();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

fn match_radius(detections: &[Detection], params: &LocalSolveParams) -> f64 {
    params
        .match_radius_px
        .max(params.match_radius_fwhm * median_fwhm(detections))
}

fn inside((width, height): (usize, usize), (x, y): (f64, f64)) -> bool {
    x >= -0.5 && y >= -0.5 && x <= width as f64 - 0.5 && y <= height as f64 - 0.5
}

fn nearest_pairs(
    projected: &[(f64, f64)],
    detections: &[Detection],
    radius: f64,
) -> Vec<(usize, usize, f64)> {
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for (i, &(px, py)) in projected.iter().enumerate() {
        for (j, d) in detections.iter().enumerate() {
            let distance = (px - d.star.x).hypot(py - d.star.y);
            if distance <= radius {
                candidates.push((distance, i, j));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut used_projected = vec![false; projected.len()];
    let mut used_detection = vec![false; detections.len()];
    let mut pairs = Vec::new();
    for (distance, i, j) in candidates {
        if used_projected[i] || used_detection[j] {
            continue;
        }
        used_projected[i] = true;
        used_detection[j] = true;
        pairs.push((i, j, distance));
    }
    pairs
}

fn corners_finite(tan: &TanParams, (width, height): (usize, usize)) -> bool {
    let right = width as f64 - 0.5;
    let top = height as f64 - 0.5;
    [(-0.5, -0.5), (right, -0.5), (right, top), (-0.5, top)]
        .iter()
        .all(|&(x, y)| {
            let (ra, dec) = pixel_to_sky(tan, x, y);
            ra.is_finite() && dec.is_finite()
        })
}

#[derive(Debug, Clone, Copy)]
struct StageGate {
    max_rms_px: f64,
    trials: usize,
}

fn false_alarm_log10(matched: usize, lambda: f64, trials: usize) -> f64 {
    let scored = matched.saturating_sub(TAN_HYPOTHESIS_PAIRS);
    (poisson_tail_log10(scored, lambda) + (trials.max(1) as f64).log10()).min(0.0)
}

type SkyToPixel<'a> = &'a dyn Fn(f64, f64) -> Option<(f64, f64)>;

fn verify_pairs(
    tan: &TanParams,
    detections: &[Detection],
    catalogue: &[(f64, f64)],
    size: (usize, usize),
    scale_range_arcsec: Option<(f64, f64)>,
    params: &LocalSolveParams,
    gate: StageGate,
) -> (Verification, Vec<SkyPair>) {
    let predict = |ra: f64, dec: f64| sky_to_pixel(tan, ra, dec);
    verify_with(
        &predict,
        match_radius(detections, params),
        tan,
        detections,
        catalogue,
        size,
        scale_range_arcsec,
        params,
        gate,
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_with(
    predict: SkyToPixel,
    radius: f64,
    tan: &TanParams,
    detections: &[Detection],
    catalogue: &[(f64, f64)],
    size: (usize, usize),
    scale_range_arcsec: Option<(f64, f64)>,
    params: &LocalSolveParams,
    gate: StageGate,
) -> (Verification, Vec<SkyPair>) {
    let (width, height) = size;
    let projected: Vec<((f64, f64), (f64, f64))> = catalogue
        .iter()
        .filter_map(|&(ra, dec)| {
            let pixel = predict(ra, dec)?;
            (pixel.0.is_finite() && pixel.1.is_finite() && inside(size, pixel))
                .then_some((pixel, (ra, dec)))
        })
        .collect();
    let positions: Vec<(f64, f64)> = projected.iter().map(|(pixel, _)| *pixel).collect();
    let matched = nearest_pairs(&positions, detections, radius);
    let distances: Vec<f64> = matched.iter().map(|&(_, _, d)| d).collect();
    let rms = clipped_rms(&distances, params.clip_sigma);
    let (coverage_log10, coverage_chance_log10) =
        coverage_log10(&positions, detections, &matched, size, radius);
    let lambda =
        projected.len() as f64 * detections.len() as f64 * std::f64::consts::PI * radius * radius
            / (width as f64 * height as f64);
    let false_alarm_log10 = false_alarm_log10(matched.len(), lambda, gate.trials);
    let corners = corners_finite(tan, size);
    let scale = scale_arcsec(&tan.cd);
    let scale_in_range = scale_range_arcsec.is_none_or(|(low, high)| scale >= low && scale <= high);
    let accepted = matched.len() >= params.min_matches
        && false_alarm_log10 < params.max_false_alarm.log10()
        && rms <= gate.max_rms_px
        && corners
        && scale_in_range;
    let pairs = matched
        .iter()
        .map(|&(i, j, _)| {
            let (ra, dec) = projected[i].1;
            SkyPair {
                x: detections[j].star.x,
                y: detections[j].star.y,
                ra,
                dec,
            }
        })
        .collect();
    (
        Verification {
            matched: matched.len(),
            predicted: projected.len(),
            detections: detections.len(),
            radius_px: radius,
            lambda,
            false_alarm_log10,
            rms_px: rms,
            coverage_log10,
            coverage_chance_log10,
            corners_finite: corners,
            scale_in_range,
            accepted,
        },
        pairs,
    )
}

fn coverage_cell((width, height): (usize, usize), (x, y): (f64, f64)) -> usize {
    let last = (COVERAGE_GRID - 1) as f64;
    let column = ((x + 0.5) * COVERAGE_GRID as f64 / width as f64)
        .floor()
        .clamp(0.0, last) as usize;
    let row = ((y + 0.5) * COVERAGE_GRID as f64 / height as f64)
        .floor()
        .clamp(0.0, last) as usize;
    row * COVERAGE_GRID + column
}

fn coverage_log10(
    positions: &[(f64, f64)],
    detections: &[Detection],
    matched: &[(usize, usize, f64)],
    size: (usize, usize),
    radius: f64,
) -> (f64, f64) {
    const CELLS: usize = COVERAGE_GRID * COVERAGE_GRID;
    if positions.is_empty() || detections.is_empty() || matched.is_empty() {
        return (0.0, 0.0);
    }
    let mut predicted = [0usize; CELLS];
    let mut detected = [0usize; CELLS];
    let mut found = [0usize; CELLS];
    for &position in positions {
        predicted[coverage_cell(size, position)] += 1;
    }
    for detection in detections {
        detected[coverage_cell(size, (detection.star.x, detection.star.y))] += 1;
    }
    for &(i, _, _) in matched {
        found[coverage_cell(size, positions[i])] += 1;
    }
    let by_predicted = matched.len() as f64 / positions.len() as f64;
    let by_detected = matched.len() as f64 / detections.len() as f64;
    let cell_area = size.0 as f64 * size.1 as f64 / CELLS as f64;
    let chance_per_pair = std::f64::consts::PI * radius * radius / cell_area;
    let mut worst = (0.0, 0.0);
    let mut worst_uncovered: Option<(f64, f64)> = None;
    for cell in 0..CELLS {
        let expected =
            (predicted[cell] as f64 * by_predicted).min(detected[cell] as f64 * by_detected);
        let chance = predicted[cell] as f64 * detected[cell] as f64 * chance_per_pair;
        let deficit = poisson_lower_tail_log10(found[cell], expected);
        let above_chance = poisson_tail_log10(found[cell], chance);
        if deficit < worst.0 {
            worst = (deficit, above_chance);
        }
        if !cell_covered(deficit, above_chance)
            && worst_uncovered.is_none_or(|(d, _)| deficit < d)
        {
            worst_uncovered = Some((deficit, above_chance));
        }
    }
    worst_uncovered.unwrap_or(worst)
}

fn poisson_lower_tail_log10(k: usize, lambda: f64) -> f64 {
    if !is_positive(lambda) {
        return 0.0;
    }
    let ln_lambda = lambda.ln();
    let mut ln_term = -lambda;
    let mut ln_sum = ln_term;
    for i in 1..=k {
        ln_term += ln_lambda - (i as f64).ln();
        let peak = ln_sum.max(ln_term);
        ln_sum = peak + ((ln_sum - peak).exp() + (ln_term - peak).exp()).ln();
    }
    (ln_sum / std::f64::consts::LN_10).min(0.0)
}

#[cfg(test)]
fn verify(
    tan: &TanParams,
    detections: &[Detection],
    catalogue: &[(f64, f64)],
    size: (usize, usize),
    scale_range_arcsec: Option<(f64, f64)>,
    params: &LocalSolveParams,
) -> Verification {
    let gate = StageGate {
        max_rms_px: params.max_rms_px,
        trials: 1,
    };
    verify_pairs(
        tan,
        detections,
        catalogue,
        size,
        scale_range_arcsec,
        params,
        gate,
    )
    .0
}

pub(crate) fn poisson_tail_log10(k: usize, lambda: f64) -> f64 {
    if k == 0 {
        return 0.0;
    }
    if !is_positive(lambda) {
        return f64::NEG_INFINITY;
    }
    let ln_lambda = lambda.ln();
    let ln_factorial: f64 = (1..=k).map(|i| (i as f64).ln()).sum();
    let mut ln_term = -lambda + k as f64 * ln_lambda - ln_factorial;
    let mut ln_sum = ln_term;
    for i in k..k + POISSON_MAX_TERMS {
        ln_term += ln_lambda - ((i + 1) as f64).ln();
        let peak = ln_sum.max(ln_term);
        ln_sum = peak + ((ln_sum - peak).exp() + (ln_term - peak).exp()).ln();
        if (i + 1) as f64 > lambda && ln_term - ln_sum < POISSON_TERM_CUTOFF.ln() {
            break;
        }
    }
    ln_sum / std::f64::consts::LN_10
}

fn quadrants(pairs: &[SkyPair], (width, height): (usize, usize)) -> usize {
    let (xc, yc) = ((width as f64 - 1.0) / 2.0, (height as f64 - 1.0) / 2.0);
    let mut seen = [false; 4];
    for pair in pairs {
        seen[usize::from(pair.x >= xc) + 2 * usize::from(pair.y >= yc)] = true;
    }
    seen.iter().filter(|&&s| s).count()
}

fn plain_tan(tan: &TanParams) -> DistortionFit {
    DistortionFit {
        order: 1,
        considered: Vec::new(),
        wcs_cards: tan_cards(tan),
        sip_max_err_px: None,
        tan: *tan,
    }
}

#[derive(Clone)]
struct SipModel {
    forward: PolyModel,
    inverse: PolyModel,
    kept: Vec<SkyPair>,
}

struct SipSelection {
    considered: Vec<(u8, f64, f64)>,
    model: Option<SipModel>,
}

fn poly_pairs(pairs: &[SkyPair], tan: &TanParams) -> Vec<(f64, f64, f64, f64)> {
    pairs
        .iter()
        .filter_map(|p| sky_to_pixel(tan, p.ra, p.dec).map(|(tx, ty)| (p.x, p.y, tx, ty)))
        .collect()
}

fn max_sip_order(tan: &TanParams, size: (usize, usize), params: &LocalSolveParams) -> u8 {
    let long_side_deg = size.0.max(size.1) as f64 * scale_arcsec(&tan.cd) / ARCSEC_PER_DEG;
    if long_side_deg >= params.sip_order_4_above_deg {
        4
    } else {
        3
    }
}

fn fit_sip_model(
    pairs: &[SkyPair],
    tan: &TanParams,
    size: (usize, usize),
    params: &LocalSolveParams,
) -> Option<SipSelection> {
    if pairs.len() < params.sip_min_inliers || quadrants(pairs, size) < params.sip_min_quadrants {
        return None;
    }
    let max_order = max_sip_order(tan, size, params);
    let mut kept: Vec<SkyPair> = pairs.to_vec();
    for round in 0..MAX_CLIP_ROUNDS {
        let rows = poly_pairs(&kept, tan);
        let selection = poly::fit_select(&rows, 1..=max_order, SIP_FOLDS)?;
        let residuals = selection.model.residuals(&rows);
        let threshold =
            (params.clip_sigma * clipped_rms(&residuals, params.clip_sigma)).max(CLIP_FLOOR_PX);
        let next: Vec<SkyPair> = kept
            .iter()
            .zip(&residuals)
            .filter(|(_, r)| **r <= threshold)
            .map(|(p, _)| *p)
            .collect();
        let stable = next.len() == kept.len()
            || next.len() < params.sip_min_inliers
            || round + 1 == MAX_CLIP_ROUNDS;
        if !stable {
            kept = next;
            continue;
        }
        if quadrants(&kept, size) < params.sip_min_quadrants {
            return None;
        }
        let considered = selection.considered;
        if selection.model.order <= 1 {
            return Some(SipSelection {
                considered,
                model: None,
            });
        }
        let swapped: Vec<(f64, f64, f64, f64)> =
            rows.iter().map(|&(x, y, tx, ty)| (tx, ty, x, y)).collect();
        let inverse = poly::fit(&swapped, selection.model.order);
        return Some(SipSelection {
            considered,
            model: inverse.map(|inverse| SipModel {
                forward: selection.model,
                inverse,
                kept,
            }),
        });
    }
    None
}

fn sip_cards(
    model: &PolyModel,
    considered: Vec<(u8, f64, f64)>,
    tan: &TanParams,
    size: (usize, usize),
) -> Result<DistortionFit, String> {
    let bbox = [[-0.5, size.0 as f64 - 0.5], [-0.5, size.1 as f64 - 0.5]];
    let options = SipFitOptions {
        degree: Some(vec![model.order]),
        crpix: Some([tan.crpix.0, tan.crpix.1]),
        ..SipFitOptions::default()
    };
    let sip = model
        .to_sip_cards(tan, bbox, &options)
        .map_err(|error| error.to_string())?;
    let sip_tan = TanParams {
        crval: (sip.crval[0], sip.crval[1]),
        crpix: (sip.crpix[0], sip.crpix[1]),
        cd: sip.cd,
    };
    Ok(DistortionFit {
        order: sip.a_order,
        considered,
        wcs_cards: sip.header_cards(),
        sip_max_err_px: Some(sip.max_err_px),
        tan: sip_tan,
    })
}

struct Distortion {
    fit: DistortionFit,
    model: Option<PolyModel>,
    warnings: Vec<String>,
}

fn distortion_from(
    selection: Option<SipSelection>,
    tan: &TanParams,
    size: (usize, usize),
) -> Distortion {
    let plain = |considered: Vec<(u8, f64, f64)>| Distortion {
        fit: DistortionFit {
            considered,
            ..plain_tan(tan)
        },
        model: None,
        warnings: Vec::new(),
    };
    let Some(SipSelection { considered, model }) = selection else {
        return plain(Vec::new());
    };
    let Some(model) = model else {
        return plain(considered);
    };
    let populated = quadrants(&model.kept, size);
    let extrapolated = (populated < 4).then(|| {
        format!(
            "SIP fitted on {populated} of 4 image quadrants: the WCS is extrapolated in the quadrant without matched stars"
        )
    });
    match sip_cards(&model.forward, considered, tan, size) {
        Ok(fit) => Distortion {
            fit,
            model: Some(model.forward),
            warnings: extrapolated.into_iter().collect(),
        },
        Err(error) => Distortion {
            warnings: vec![format!("SIP fit skipped: {error}")],
            ..plain(Vec::new())
        },
    }
}

#[cfg(test)]
fn fit_distortion(
    pairs: &[SkyPair],
    tan: &TanParams,
    size: (usize, usize),
    params: &LocalSolveParams,
) -> (DistortionFit, Vec<String>) {
    let distortion = distortion_from(fit_sip_model(pairs, tan, size, params), tan, size);
    (distortion.fit, distortion.warnings)
}

#[derive(Clone, Copy)]
struct PixelMap {
    scale: f64,
    offset: f64,
}

impl PixelMap {
    fn identity() -> Self {
        Self {
            scale: 1.0,
            offset: 0.0,
        }
    }

    fn then_binned(self, factor: usize, origin: usize) -> Self {
        let factor = factor as f64;
        Self {
            scale: self.scale * factor,
            offset: self.offset + self.scale * (origin as f64 + (factor - 1.0) / 2.0),
        }
    }

    fn apply(&self, value: f64) -> f64 {
        self.scale * value + self.offset
    }
}

fn block_mean(image: &Array2<f32>, factor: usize) -> Array2<f32> {
    let (rows, cols) = image.dim();
    let (out_rows, out_cols) = (rows / factor, cols / factor);
    let mut out = Array2::<f32>::zeros((out_rows, out_cols));
    out.axis_iter_mut(Axis(0))
        .into_par_iter()
        .enumerate()
        .for_each(|(oy, mut row)| {
            for (ox, value) in row.iter_mut().enumerate() {
                let block = image.slice(s![
                    oy * factor..(oy + 1) * factor,
                    ox * factor..(ox + 1) * factor
                ]);
                let (sum, count) = block
                    .iter()
                    .filter(|v| v.is_finite())
                    .fold((0.0f64, 0usize), |(sum, count), &v| {
                        (sum + f64::from(v), count + 1)
                    });
                *value = if count > 0 {
                    (sum / count as f64) as f32
                } else {
                    f32::NAN
                };
            }
        });
    out
}

struct BackgroundMap {
    tile: usize,
    medians: Array2<f32>,
    global: f32,
}

impl BackgroundMap {
    fn estimate(image: &Array2<f32>, tile: usize) -> Option<Self> {
        let tile = tile.max(1);
        let (rows, cols) = image.dim();
        let (tile_rows, tile_cols) = (rows.div_ceil(tile), cols.div_ceil(tile));
        if tile_rows == 0 || tile_cols == 0 {
            return None;
        }
        let cells: Vec<f32> = (0..tile_rows * tile_cols)
            .into_par_iter()
            .map(|index| {
                let (ty, tx) = (index / tile_cols, index % tile_cols);
                let block = image.slice(s![
                    ty * tile..((ty + 1) * tile).min(rows),
                    tx * tile..((tx + 1) * tile).min(cols)
                ]);
                let mut values: Vec<f32> = block.iter().copied().filter(|v| v.is_finite()).collect();
                if values.len() < BACKGROUND_TILE_MIN_PIXELS {
                    return f32::NAN;
                }
                sigma_clipped_stats(&mut values, BACKGROUND_CLIP_SIGMA, BACKGROUND_CLIP_ROUNDS).0 as f32
            })
            .collect();
        let medians = Array2::from_shape_vec((tile_rows, tile_cols), cells).ok()?;
        let mut finite: Vec<f32> = medians.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.is_empty() {
            return None;
        }
        let middle = finite.len() / 2;
        finite.select_nth_unstable_by(middle, f32::total_cmp);
        Some(Self {
            tile,
            medians,
            global: finite[middle],
        })
    }

    fn cell(&self, ty: isize, tx: isize) -> f32 {
        let (tile_rows, tile_cols) = self.medians.dim();
        let value = self.medians[[
            ty.clamp(0, tile_rows as isize - 1) as usize,
            tx.clamp(0, tile_cols as isize - 1) as usize,
        ]];
        if value.is_finite() {
            value
        } else {
            self.global
        }
    }

    fn at(&self, x: f64, y: f64) -> f32 {
        let fx = (x + 0.5) / self.tile as f64 - 0.5;
        let fy = (y + 0.5) / self.tile as f64 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (wx, wy) = ((fx - x0) as f32, (fy - y0) as f32);
        let (tx, ty) = (x0 as isize, y0 as isize);
        self.cell(ty, tx) * (1.0 - wy) * (1.0 - wx)
            + self.cell(ty, tx + 1) * (1.0 - wy) * wx
            + self.cell(ty + 1, tx) * wy * (1.0 - wx)
            + self.cell(ty + 1, tx + 1) * wy * wx
    }

    fn excess_at(&self, x: f64, y: f64) -> f64 {
        f64::from(self.at(x, y) - self.global)
    }

    fn flattened(&self, image: &Array2<f32>, keep_from: Option<f32>) -> Array2<f32> {
        let mut out = image.clone();
        out.axis_iter_mut(Axis(0))
            .into_par_iter()
            .enumerate()
            .for_each(|(row, mut line)| {
                for (col, value) in line.iter_mut().enumerate() {
                    if keep_from.is_some_and(|plateau| *value >= plateau) {
                        continue;
                    }
                    *value = *value - self.at(col as f64, row as f64) + self.global;
                }
            });
        out
    }
}

fn saturation_level(image: &Array2<f32>) -> Option<f64> {
    let max = image
        .iter()
        .filter(|v| v.is_finite())
        .fold(f32::NEG_INFINITY, |m, &v| m.max(v));
    if !max.is_finite() {
        return None;
    }
    let floor = max * SATURATION_FRACTION as f32;
    let plateau = image.iter().filter(|&&v| v >= floor).count();
    (plateau >= SATURATION_PLATEAU_PIXELS).then_some(f64::from(max))
}

pub fn detect_for_solve(
    image: &Array2<f32>,
    hints: &SolveHints,
    params: &LocalSolveParams,
) -> Vec<Detection> {
    let mut working: Cow<Array2<f32>> = Cow::Borrowed(image);
    let mut map_x = PixelMap::identity();
    let mut map_y = PixelMap::identity();
    if let Some(cfa) = &hints.cfa {
        let (origin_y, origin_x) = cfa.aligned_origin();
        let cell = cfa.bin_factor();
        working = Cow::Owned(luminance_bin(&working, cfa));
        map_x = map_x.then_binned(cell, origin_x);
        map_y = map_y.then_binned(cell, origin_y);
    }
    let (rows, cols) = working.dim();
    let factor = rows.max(cols).div_ceil(params.max_side_px.max(1));
    if factor > 1 {
        working = Cow::Owned(block_mean(&working, factor));
        map_x = map_x.then_binned(factor, 0);
        map_y = map_y.then_binned(factor, 0);
    }
    let (rows, cols) = working.dim();
    let saturation = saturation_level(&working);
    let background_map = BackgroundMap::estimate(&working, params.background_tile_px);
    if let Some(map) = &background_map {
        let plateau = saturation.map(|level| (SATURATION_FRACTION * level) as f32);
        let flattened = map.flattened(&working, plateau);
        working = Cow::Owned(flattened);
    }
    let detected = detect_stars(&working, params.detect_sigma);
    let background = detected.background_median;
    detected
        .stars
        .into_iter()
        .filter(|star| {
            let margin = EDGE_MARGIN_PX.max(EDGE_MARGIN_FWHM * star.fwhm);
            star.x >= margin
                && star.y >= margin
                && star.x <= cols as f64 - 1.0 - margin
                && star.y <= rows as f64 - 1.0 - margin
        })
        .map(|star| {
            let local_excess = background_map
                .as_ref()
                .map_or(0.0, |map| map.excess_at(star.x, star.y));
            let raw_peak = star.peak + background + local_excess;
            let saturated = saturation.is_some_and(|level| raw_peak >= SATURATION_FRACTION * level);
            Detection {
                star: DetectedStar {
                    x: map_x.apply(star.x),
                    y: map_y.apply(star.y),
                    fwhm: star.fwhm * map_x.scale,
                    ..star
                },
                saturated,
            }
        })
        .collect()
}

fn compactness_limit(detections: &[Detection], params: &LocalSolveParams) -> f64 {
    let mut brightest: Vec<&Detection> = detections
        .iter()
        .filter(|d| d.star.x.is_finite() && d.star.y.is_finite())
        .filter(|d| d.star.eccentricity.is_finite())
        .collect();
    brightest.sort_by(|a, b| b.star.flux.total_cmp(&a.star.flux));
    let mut eccentricities: Vec<f64> = brightest
        .iter()
        .take(params.detection_stars.max(1))
        .map(|d| d.star.eccentricity)
        .collect();
    if eccentricities.is_empty() {
        return params.max_eccentricity;
    }
    eccentricities.sort_by(f64::total_cmp);
    let rank = ((eccentricities.len() - 1) as f64 * params.compact_quantile.clamp(0.0, 1.0))
        .round() as usize;
    params.max_eccentricity.max(eccentricities[rank])
}

fn is_compact(detection: &Detection, limit: f64) -> bool {
    detection.star.eccentricity <= limit
}

fn usable_detections(detections: &[Detection], params: &LocalSolveParams) -> Vec<Detection> {
    let limit = compactness_limit(detections, params);
    let mut usable: Vec<Detection> = detections
        .iter()
        .filter(|d| is_compact(d, limit))
        .filter(|d| d.star.x.is_finite() && d.star.y.is_finite())
        .cloned()
        .collect();
    usable.sort_by(|a, b| b.star.flux.total_cmp(&a.star.flux));
    usable
}

fn dropped_saturated(detections: &[Detection], params: &LocalSolveParams) -> usize {
    let limit = compactness_limit(detections, params);
    detections
        .iter()
        .filter(|d| d.saturated && !is_compact(d, limit))
        .count()
}

fn disc_points(
    usable: &[Detection],
    (width, height): (usize, usize),
    radius_px: f64,
) -> Vec<(f64, f64)> {
    let (xc, yc) = ((width as f64 - 1.0) / 2.0, (height as f64 - 1.0) / 2.0);
    usable
        .iter()
        .map(|d| (d.star.x, d.star.y))
        .filter(|&(x, y)| (x - xc).hypot(y - yc) <= radius_px)
        .collect()
}

fn spread_detections(
    usable: &[Detection],
    (width, height): (usize, usize),
    limit: usize,
) -> Vec<Detection> {
    let stars: Vec<DetectedStar> = usable.iter().map(|d| d.star.clone()).collect();
    let chosen = select_alignment_stars(&stars, limit, height, width);
    let index: HashMap<(u64, u64), usize> = usable
        .iter()
        .enumerate()
        .map(|(i, d)| ((d.star.x.to_bits(), d.star.y.to_bits()), i))
        .collect();
    chosen
        .iter()
        .filter_map(|&(x, y)| index.get(&(x.to_bits(), y.to_bits())))
        .map(|&i| usable[i].clone())
        .collect()
}

fn radial_spread(
    usable: &[Detection],
    (width, height): (usize, usize),
    per_band: usize,
) -> Vec<Detection> {
    let (xc, yc) = ((width as f64 - 1.0) / 2.0, (height as f64 - 1.0) / 2.0);
    let half_diagonal = (width as f64 / 2.0).hypot(height as f64 / 2.0);
    let mut counts = [0usize; SIP_RADIAL_BANDS];
    usable
        .iter()
        .filter(|d| {
            let r2 = ((d.star.x - xc).powi(2) + (d.star.y - yc).powi(2)) / (half_diagonal * half_diagonal);
            let band = ((r2 * SIP_RADIAL_BANDS as f64).floor() as usize).min(SIP_RADIAL_BANDS - 1);
            if counts[band] < per_band {
                counts[band] += 1;
                true
            } else {
                false
            }
        })
        .cloned()
        .collect()
}

struct MatchSide {
    stars: Vec<(f64, f64)>,
    tris: Vec<TriangleDesc>,
    mirrored_stars: Vec<(f64, f64)>,
    mirrored_tris: Vec<TriangleDesc>,
    params: TriangleParams,
}

impl MatchSide {
    fn new(points: &[(f64, f64)], match_stars: usize, inlier_px: f64) -> Self {
        let params = TriangleParams {
            max_stars: match_stars.max(MIN_TAN_PAIRS),
            inlier_px,
            max_residual_px: inlier_px.max(TriangleParams::SOLVER.max_residual_px),
            ..TriangleParams::SOLVER
        };
        let stars = points[..points.len().min(params.max_stars)].to_vec();
        let mirrored_stars: Vec<(f64, f64)> = stars.iter().map(|&(x, y)| (-x, y)).collect();
        Self {
            tris: build_triangles(&stars, &params),
            mirrored_tris: build_triangles(&mirrored_stars, &params),
            stars,
            mirrored_stars,
            params,
        }
    }

    fn match_catalogue(&self, catalogue: &[(f64, f64)]) -> (TriangleMatch, TriangleMatch) {
        let cat_tris = build_triangles(catalogue, &self.params);
        let same = match_triangles(catalogue, &self.stars, &cat_tris, &self.tris, &self.params);
        let mirrored = match_triangles(
            catalogue,
            &self.mirrored_stars,
            &cat_tris,
            &self.mirrored_tris,
            &self.params,
        );
        let unmirrored = TriangleMatch {
            pairs: mirrored
                .pairs
                .into_iter()
                .map(|(rx, ry, tx, ty)| (rx, ry, -tx, ty))
                .collect(),
            votes: mirrored.votes,
            flipped: true,
        };
        (same, unmirrored)
    }
}

fn window_inliers(side: &MatchSide, catalogue: &[(f64, f64)]) -> Option<Vec<StarPair>> {
    let (same, mirrored) = side.match_catalogue(catalogue);
    let [same_inliers, mirrored_inliers] = [&same, &mirrored].map(|m| {
        ransac_affine(&m.pairs, AffineAlignMethod::Affine, &side.params)
            .map(|fit| inlier_pairs(&m.pairs, &fit.transform, side.params.inlier_px))
    });
    match (same_inliers, mirrored_inliers) {
        (Some(a), Some(b)) if b.len() > a.len() => Some(b),
        (Some(a), _) => Some(a),
        (None, b) => b,
    }
}

fn best_window_inliers(side: &MatchSide, catalogue: &[(f64, f64)]) -> Option<Vec<StarPair>> {
    let window = ((side.params.max_stars as f64 * MATCH_WINDOW_FRACTION).round() as usize)
        .max(side.params.min_inliers);
    let step = (window / 2).max(1);
    let mut best: Option<Vec<StarPair>> = None;
    for index in 0..MATCH_WINDOWS {
        let start = index * step;
        if start + side.params.min_inliers > catalogue.len() {
            break;
        }
        let end = (start + window).min(catalogue.len());
        if let Some(inliers) = window_inliers(side, &catalogue[start..end]) {
            if best.as_ref().is_none_or(|b| inliers.len() > b.len()) {
                best = Some(inliers);
            }
        }
    }
    best.filter(|inliers| inliers.len() >= side.params.min_inliers)
}

fn ransac_inlier_px(long_side_px: usize, long_side_deg: f64) -> f64 {
    let long_side_rad = long_side_deg.to_radians();
    TriangleParams::SOLVER
        .inlier_px
        .max(RANSAC_INLIER_FIELD_FACTOR * long_side_px as f64 * long_side_rad * long_side_rad)
}

fn inlier_pairs(
    matched: &[StarPair],
    transform: &AffineTransform,
    inlier_px: f64,
) -> Vec<StarPair> {
    matched
        .iter()
        .copied()
        .filter(|&(rx, ry, tx, ty)| {
            let (px, py) = transform.map(rx, ry);
            (px - tx).hypot(py - ty) < inlier_px
        })
        .collect()
}

struct SearchContext<'a> {
    detections: Vec<Detection>,
    sip_detections: Vec<Detection>,
    saturated: usize,
    size: (usize, usize),
    catalog: &'a dyn CatalogProvider,
    ranking: RankingKey,
    params: &'a LocalSolveParams,
    epoch_year: Option<f64>,
    scale_range: (f64, f64),
    trials: usize,
}

struct Hypothesis {
    scale_deg: f64,
    spacing_deg: f64,
    cone_radius_deg: f64,
    bright_skip: usize,
    stage_rms_px: f64,
    hinted: MatchSide,
    spiral: Option<MatchSide>,
}

impl Hypothesis {
    fn new(
        nominal_arcsec: f64,
        points: &[(f64, f64)],
        saturated: usize,
        size: (usize, usize),
        params: &LocalSolveParams,
    ) -> Self {
        let (width, height) = size;
        let scale_deg = nominal_arcsec / ARCSEC_PER_DEG;
        let field_w = width as f64 * scale_deg;
        let field_h = height as f64 * scale_deg;
        let long_side_deg = field_w.max(field_h);
        let inlier_px = ransac_inlier_px(width.max(height), long_side_deg);
        let cone_radius_deg = params.cone_short_side_fraction * field_w.min(field_h);
        let cone_area = std::f64::consts::PI * cone_radius_deg * cone_radius_deg;
        Self {
            scale_deg,
            spacing_deg: spiral_spacing_deg(scale_deg, field_w.min(field_h), cone_radius_deg, params),
            cone_radius_deg,
            bright_skip: (saturated as f64 * cone_area / (field_w * field_h)).round() as usize,
            stage_rms_px: params
                .max_rms_px
                .max(params.tan_rms_px_per_deg * long_side_deg),
            hinted: MatchSide::new(points, params.hinted_match_stars, inlier_px),
            spiral: None,
        }
    }

    fn side(&self, ring: usize) -> &MatchSide {
        if ring == 0 {
            &self.hinted
        } else {
            self.spiral.as_ref().unwrap_or(&self.hinted)
        }
    }
}

fn spiral_spacing_deg(
    scale_deg: f64,
    short_side_deg: f64,
    cone_radius_deg: f64,
    params: &LocalSolveParams,
) -> f64 {
    let coarse = params.spiral_spacing_fraction * short_side_deg;
    let cone_rad = cone_radius_deg.to_radians();
    let worst_offset_rad = params.spiral_projective_px * scale_deg.to_radians() / (cone_rad * cone_rad);
    let fine = (3f64.sqrt() * worst_offset_rad).to_degrees();
    if is_positive(fine) {
        coarse.min(fine)
    } else {
        coarse
    }
}

struct Solved {
    verification: Verification,
    distortion: DistortionFit,
    rms_px: f64,
    warnings: Vec<String>,
}

struct CentreOutcome {
    candidate: Option<CandidateNumbers>,
    solved: Option<Solved>,
}

fn ranked_sky(
    rows: &[CatalogRow],
    skip: usize,
    limit: usize,
    ranking: RankingKey,
) -> Vec<(f64, f64)> {
    let mut ranked: Vec<(f64, usize)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| ranking.of(row).map(|key| (key, i)))
        .filter(|&(_, i)| rows[i].ra_epoch.is_finite() && rows[i].dec_epoch.is_finite())
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    ranked
        .iter()
        .skip(skip)
        .take(limit)
        .map(|&(_, i)| (rows[i].ra_epoch, rows[i].dec_epoch))
        .collect()
}

fn footprint_limit(detections: usize, params: &LocalSolveParams) -> usize {
    (params.footprint_stars_per_detection * detections).max(params.footprint_min_stars)
}

fn footprint_catalogue(tan: &TanParams, ctx: &SearchContext) -> Result<Vec<(f64, f64)>, String> {
    let (width, height) = ctx.size;
    let half_diagonal_deg =
        0.5 * (width as f64).hypot(height as f64) * scale_arcsec(&tan.cd) / ARCSEC_PER_DEG;
    let radius = ctx.params.footprint_margin * half_diagonal_deg;
    let mut rows = ctx.catalog.cone(tan.crval.0, tan.crval.1, radius)?;
    if let Some(epoch) = ctx.epoch_year {
        propagate_epoch(&mut rows, epoch);
    }
    let deepest = ctx.detections.len().max(ctx.sip_detections.len());
    Ok(ranked_sky(&rows, 0, usize::MAX, ctx.ranking)
        .into_iter()
        .filter(|&(ra, dec)| sky_to_pixel(tan, ra, dec).is_some_and(|p| inside(ctx.size, p)))
        .skip(ctx.saturated)
        .take(footprint_limit(deepest, ctx.params))
        .collect())
}

fn is_significant(verification: &Verification, params: &LocalSolveParams) -> bool {
    verification.matched >= params.min_matches
        && verification.false_alarm_log10 < params.max_false_alarm.log10()
        && verification.corners_finite
        && verification.scale_in_range
}

fn refine_on(
    mut tan: TanParams,
    catalogue: &[(f64, f64)],
    ctx: &SearchContext,
    gate: StageGate,
) -> (TanParams, Verification, Vec<SkyPair>) {
    let (mut verification, mut pairs) = verify_pairs(
        &tan,
        &ctx.detections,
        catalogue,
        ctx.size,
        Some(ctx.scale_range),
        ctx.params,
        gate,
    );
    for _ in 0..REFINE_PASSES {
        if !is_significant(&verification, ctx.params) {
            break;
        }
        let Some(refined) = fit_tan(&pairs, ctx.size, tan.crval, ctx.params) else {
            break;
        };
        let (again, again_pairs) = verify_pairs(
            &refined.tan,
            &ctx.detections,
            catalogue,
            ctx.size,
            Some(ctx.scale_range),
            ctx.params,
            gate,
        );
        let worse = again.rms_px > verification.rms_px && again.matched <= verification.matched;
        if !is_significant(&again, ctx.params) || worse {
            break;
        }
        tan = refined.tan;
        verification = again;
        pairs = again_pairs;
    }
    (tan, verification, pairs)
}

fn model_rms_px(
    model: &PolyModel,
    pairs: &[SkyPair],
    tan: &TanParams,
    fallback: f64,
    params: &LocalSolveParams,
) -> f64 {
    let residuals = model.residuals(&poly_pairs(pairs, tan));
    if residuals.is_empty() {
        fallback
    } else {
        clipped_rms(&residuals, params.clip_sigma)
    }
}

struct Matched {
    verification: Verification,
    pairs: Vec<SkyPair>,
}

fn verify_through(
    model: &SipModel,
    radius: f64,
    tan: &TanParams,
    catalogue: &[(f64, f64)],
    ctx: &SearchContext,
    gate: StageGate,
) -> Matched {
    let predict =
        |ra: f64, dec: f64| sky_to_pixel(tan, ra, dec).map(|(tx, ty)| model.inverse.eval(tx, ty));
    let (verification, pairs) = verify_with(
        &predict,
        radius,
        tan,
        &ctx.sip_detections,
        catalogue,
        ctx.size,
        Some(ctx.scale_range),
        ctx.params,
        gate,
    );
    Matched {
        verification,
        pairs,
    }
}

fn sip_passes(
    tan: &TanParams,
    first: Matched,
    catalogue: &[(f64, f64)],
    ctx: &SearchContext,
    gate: StageGate,
) -> (Matched, Option<SipSelection>) {
    let Some(SipSelection { considered, model }) =
        fit_sip_model(&first.pairs, tan, ctx.size, ctx.params)
    else {
        return (first, None);
    };
    let Some(first_model) = model else {
        return (
            first,
            Some(SipSelection {
                considered,
                model: None,
            }),
        );
    };
    let base_radius = match_radius(&ctx.detections, ctx.params);
    let long_side_px = ctx.size.0.max(ctx.size.1) as f64;
    let max_radius = base_radius.max(SIP_PASS_MAX_RADIUS_FRACTION * long_side_px);
    let mut radius = base_radius;
    let mut current = first_model.clone();
    let mut grew = false;
    for _ in 0..SIP_PASSES {
        radius = (radius * SIP_PASS_RADIUS_GROWTH).min(max_radius);
        let matched = verify_through(&current, radius, tan, catalogue, ctx, gate);
        match fit_sip_model(&matched.pairs, tan, ctx.size, ctx.params).and_then(|s| s.model) {
            Some(model) if model.kept.len() > current.kept.len() => {
                current = model;
                grew = true;
            }
            _ => break,
        }
    }
    let first_selection = SipSelection {
        considered,
        model: Some(first_model),
    };
    if !grew {
        return (first, Some(first_selection));
    }
    let matched = verify_through(&current, base_radius, tan, catalogue, ctx, gate);
    match fit_sip_model(&matched.pairs, tan, ctx.size, ctx.params) {
        Some(selection) if matched.verification.accepted && selection.model.is_some() => {
            (matched, Some(selection))
        }
        _ => (first, Some(first_selection)),
    }
}

fn try_centre(
    centre: (f64, f64),
    hypothesis: &Hypothesis,
    side: &MatchSide,
    ctx: &SearchContext,
) -> Result<CentreOutcome, String> {
    let mut rows = ctx
        .catalog
        .cone(centre.0, centre.1, hypothesis.cone_radius_deg)?;
    if let Some(epoch) = ctx.epoch_year {
        propagate_epoch(&mut rows, epoch);
    }
    let sky = ranked_sky(
        &rows,
        hypothesis.bright_skip,
        ctx.params.catalogue_stars,
        ctx.ranking,
    );
    let plane: Vec<((f64, f64), (f64, f64))> = sky
        .iter()
        .filter_map(|&(ra, dec)| {
            let (x, y) = tan_plane(centre, ra, dec)?;
            Some((
                (-x / hypothesis.scale_deg, y / hypothesis.scale_deg),
                (ra, dec),
            ))
        })
        .collect();
    let none = CentreOutcome {
        candidate: None,
        solved: None,
    };
    if plane.len() < MIN_TAN_PAIRS {
        return Ok(none);
    }
    let catalogue: Vec<(f64, f64)> = plane.iter().map(|(p, _)| *p).collect();
    let by_plane: HashMap<(u64, u64), (f64, f64)> = plane
        .iter()
        .map(|&((x, y), sky)| ((x.to_bits(), y.to_bits()), sky))
        .collect();
    let Some(inliers) = best_window_inliers(side, &catalogue) else {
        return Ok(none);
    };
    let sky_pairs: Vec<SkyPair> = inliers
        .iter()
        .filter_map(|&(rx, ry, tx, ty)| {
            let &(ra, dec) = by_plane.get(&(rx.to_bits(), ry.to_bits()))?;
            Some(SkyPair {
                x: tx,
                y: ty,
                ra,
                dec,
            })
        })
        .collect();
    let Some(fit) = fit_tan(&sky_pairs, ctx.size, centre, ctx.params) else {
        return Ok(none);
    };
    let gate = StageGate {
        max_rms_px: hypothesis.stage_rms_px,
        trials: ctx.trials,
    };
    let (coarse_tan, coarse, _) = refine_on(fit.tan, &sky, ctx, gate);
    let numbers = |tan: &TanParams, v: &Verification, rms_px: f64| CandidateNumbers {
        centre: tan.crval,
        matched: v.matched,
        predicted: v.predicted,
        rms_px,
        false_alarm_log10: v.false_alarm_log10,
        coverage_log10: v.coverage_log10,
        coverage_chance_log10: v.coverage_chance_log10,
    };
    if !coarse.accepted {
        return Ok(CentreOutcome {
            candidate: Some(numbers(&coarse_tan, &coarse, coarse.rms_px)),
            solved: None,
        });
    }
    let footprint = footprint_catalogue(&coarse_tan, ctx)?;
    let tan_footprint =
        &footprint[..footprint.len().min(footprint_limit(ctx.detections.len(), ctx.params))];
    let (tan, verification, pairs) = refine_on(coarse_tan, tan_footprint, ctx, gate);
    if !verification.accepted {
        return Ok(CentreOutcome {
            candidate: Some(numbers(&tan, &verification, verification.rms_px)),
            solved: None,
        });
    }
    let (matched, selection) = sip_passes(
        &tan,
        Matched {
            verification,
            pairs,
        },
        &footprint,
        ctx,
        gate,
    );
    let Matched {
        verification,
        pairs,
    } = matched;
    let Distortion {
        fit: distortion,
        model,
        warnings,
    } = distortion_from(selection, &tan, ctx.size);
    let rms_px = model.map_or(verification.rms_px, |model| {
        model_rms_px(&model, &pairs, &tan, verification.rms_px, ctx.params)
    });
    let candidate = numbers(&tan, &verification, rms_px);
    let covered = verification.covered();
    let solved = (rms_px <= ctx.params.max_rms_px && covered).then_some(Solved {
        verification,
        distortion,
        rms_px,
        warnings,
    });
    Ok(CentreOutcome {
        candidate: Some(candidate),
        solved,
    })
}

fn better_candidate(a: &CandidateNumbers, b: &CandidateNumbers) -> bool {
    (a.matched, -a.false_alarm_log10, -a.rms_px) > (b.matched, -b.false_alarm_log10, -b.rms_px)
}

fn record_best(best: &Mutex<Option<CandidateNumbers>>, candidate: &CandidateNumbers) {
    let mut slot = best.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot
        .as_ref()
        .is_none_or(|current| better_candidate(candidate, current))
    {
        *slot = Some(candidate.clone());
    }
}

fn best_snapshot(best: &Mutex<Option<CandidateNumbers>>) -> Option<CandidateNumbers> {
    best.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn finish_solution(
    solved: Solved,
    detections: usize,
    centres_tried: usize,
    started: Instant,
) -> LocalSolution {
    let distortion = solved.distortion;
    let tan = distortion.tan;
    let sip_order = (distortion.order >= 2).then_some(distortion.order);
    LocalSolution {
        wcs_cards: distortion.wcs_cards,
        tan,
        centre: tan.crval,
        scale_arcsec: scale_arcsec(&tan.cd),
        rotation_deg: rotation_deg(&tan.cd),
        parity: parity_of(&tan.cd),
        matched: solved.verification.matched,
        predicted: solved.verification.predicted,
        rms_px: solved.rms_px,
        false_alarm_log10: solved.verification.false_alarm_log10,
        sip_order,
        sip_max_err_px: distortion.sip_max_err_px,
        detections,
        centres_tried,
        elapsed: started.elapsed(),
        warnings: solved.warnings,
    }
}

#[allow(clippy::too_many_arguments)]
fn solve_detections(
    started: Instant,
    detections: &[Detection],
    size: (usize, usize),
    hints: &SolveHints,
    catalog: &dyn CatalogProvider,
    params: &LocalSolveParams,
    cancel: CancelCheck,
    stage: StageCallback,
) -> Result<LocalSolution, SolveFailure> {
    let failure = |reason: FailureReason,
                   message: String,
                   detections: usize,
                   centres_tried: usize,
                   best: Option<CandidateNumbers>| SolveFailure {
        reason,
        message,
        detections,
        centres_tried,
        best: best.map(Box::new),
        elapsed: started.elapsed(),
    };
    let Some(centre) = hints.centre.as_ref() else {
        return Err(failure(
            FailureReason::MissingHint,
            "no pointing hint: the local solver needs an approximate centre".to_string(),
            detections.len(),
            0,
            None,
        ));
    };
    let Some(scale) = hints.scale.as_ref() else {
        return Err(failure(
            FailureReason::MissingHint,
            "no pixel scale hint: the local solver needs a scale range".to_string(),
            detections.len(),
            0,
            None,
        ));
    };
    let nominal = scale_hypotheses(scale.low_arcsec, scale.high_arcsec);
    let (width, height) = size;
    if !nominal.iter().all(|&s| is_positive(s))
        || !centre.ra.is_finite()
        || !centre.dec.is_finite()
        || width == 0
        || height == 0
    {
        return Err(failure(
            FailureReason::MissingHint,
            format!(
                "unusable hints: centre ({}, {}), scale {}..{} arcsec/px, size {width}x{height}",
                centre.ra, centre.dec, scale.low_arcsec, scale.high_arcsec
            ),
            detections.len(),
            0,
            None,
        ));
    }
    let hinted_long_side_deg = scale.high_arcsec * width.max(height) as f64 / ARCSEC_PER_DEG;
    if hinted_long_side_deg > MAX_TAN_FIELD_DEG {
        return Err(failure(
            FailureReason::MissingHint,
            format!(
                "unusable scale hint: {}..{} arcsec/px from {} gives a {hinted_long_side_deg:.0} deg field, wider than a TAN image can be (at most {MAX_TAN_FIELD_DEG:.0} deg)",
                scale.low_arcsec, scale.high_arcsec, scale.source
            ),
            detections.len(),
            0,
            None,
        ));
    }
    let usable = usable_detections(detections, params);
    let selected = spread_detections(&usable, size, params.detection_stars);
    let n_det = selected.len();
    if n_det < params.min_matches {
        return Err(failure(
            FailureReason::TooFewStars,
            format!(
                "{n_det} usable stars detected, at least {} needed",
                params.min_matches
            ),
            n_det,
            0,
            None,
        ));
    }
    if cancel() {
        return Err(failure(
            FailureReason::Cancelled,
            "cancelled before the search started".to_string(),
            n_det,
            0,
            None,
        ));
    }
    let disc_radius_px = params.cone_short_side_fraction * width.min(height) as f64;
    let points = disc_points(&usable, size, disc_radius_px);
    let saturated = dropped_saturated(detections, params);
    let mut hypotheses: Vec<Hypothesis> = nominal
        .iter()
        .map(|&arcsec| Hypothesis::new(arcsec, &points, saturated, size, params))
        .collect();
    let spacings: Vec<f64> = hypotheses.iter().map(|h| h.spacing_deg).collect();
    let widest_field_deg =
        nominal.iter().copied().fold(0.0f64, f64::max) * width.max(height) as f64 / ARCSEC_PER_DEG;
    let requested_radius = params
        .near_radius_deg
        .max(params.near_radius_fields * widest_field_deg)
        .clamp(0.0, MAX_NEAR_RADIUS_DEG);
    let plan = search_plan(
        (centre.ra, centre.dec),
        &spacings,
        requested_radius,
        params.max_centres.max(1),
    );
    if plan.rings.len() > 1 {
        for hypothesis in &mut hypotheses {
            let inlier_px = hypothesis.hinted.params.inlier_px;
            hypothesis.spiral = Some(MatchSide::new(
                &points,
                params.spiral_match_stars,
                inlier_px,
            ));
        }
    }
    let sip_detections = if params.sip_stars_per_band == 0 {
        selected.clone()
    } else {
        radial_spread(&usable, size, params.sip_stars_per_band)
    };
    let ctx = SearchContext {
        detections: selected,
        sip_detections,
        saturated,
        size,
        catalog,
        ranking: catalog.ranking_key(),
        params,
        epoch_year: hints.epoch_year,
        scale_range: (scale.low_arcsec, scale.high_arcsec),
        trials: 2 * plan.centres,
    };
    let tried = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let cancelled = AtomicBool::new(false);
    let best: Mutex<Option<CandidateNumbers>> = Mutex::new(None);
    let catalog_error: Mutex<Option<String>> = Mutex::new(None);
    let last_ring = plan.rings.len() - 1;
    for (k, ring) in plan.rings.iter().enumerate() {
        stage(&format!("{STAGE_SEARCH} ring {k}/{last_ring}"));
        let mut solved: Vec<(usize, Solved)> = ring
            .par_iter()
            .enumerate()
            .filter_map(|(index, &(h, candidate_centre))| {
                if stop.load(Ordering::SeqCst) {
                    return None;
                }
                if cancel() {
                    stop.store(true, Ordering::SeqCst);
                    cancelled.store(true, Ordering::SeqCst);
                    return None;
                }
                tried.fetch_add(1, Ordering::SeqCst);
                let hypothesis = &hypotheses[h];
                match try_centre(candidate_centre, hypothesis, hypothesis.side(k), &ctx) {
                    Ok(outcome) => {
                        if let Some(candidate) = &outcome.candidate {
                            record_best(&best, candidate);
                        }
                        outcome.solved.map(|s| (index, s))
                    }
                    Err(error) => {
                        let mut slot = catalog_error
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        slot.get_or_insert(error);
                        None
                    }
                }
            })
            .collect();
        let centres_tried = tried.load(Ordering::SeqCst);
        if cancelled.load(Ordering::SeqCst) {
            return Err(failure(
                FailureReason::Cancelled,
                format!("cancelled after {centres_tried} centres"),
                n_det,
                centres_tried,
                best_snapshot(&best),
            ));
        }
        solved.sort_by(|(ia, a), (ib, b)| {
            b.verification
                .matched
                .cmp(&a.verification.matched)
                .then(
                    a.verification
                        .false_alarm_log10
                        .total_cmp(&b.verification.false_alarm_log10),
                )
                .then(a.rms_px.total_cmp(&b.rms_px))
                .then(ia.cmp(ib))
        });
        if let Some((_, winner)) = solved.into_iter().next() {
            stage(STAGE_FIT);
            return Ok(finish_solution(winner, n_det, centres_tried, started));
        }
    }
    let centres_tried = tried.load(Ordering::SeqCst);
    let best = best_snapshot(&best);
    let catalog_error = catalog_error
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match (best.as_ref(), catalog_error) {
        (None, Some(error)) => Err(failure(
            FailureReason::Catalog,
            format!("catalogue read failed: {error}"),
            n_det,
            centres_tried,
            None,
        )),
        (best, _) => {
            let detail = match best {
                Some(b) => format!(
                    "best candidate matched {} of {} predicted stars, rms {:.2} px, log10 P {:.1}, cell coverage log10 P {:.1}, cell matches above chance log10 P {:.1}",
                    b.matched,
                    b.predicted,
                    b.rms_px,
                    b.false_alarm_log10,
                    b.coverage_log10,
                    b.coverage_chance_log10
                ),
                None => format!(
                    "no centre produced {} consistent star matches (scale hint {}..{} arcsec/px from {})",
                    params.min_matches, scale.low_arcsec, scale.high_arcsec, scale.source
                ),
            };
            let searched = if plan.capped {
                format!(
                    "{:.2} deg of the hint ({centres_tried} centres, search capped at {} centres; {requested_radius:.2} deg requested)",
                    plan.radius_reached_deg, params.max_centres
                )
            } else {
                format!("{requested_radius:.2} deg of the hint after {centres_tried} centres")
            };
            Err(failure(
                FailureReason::NoSolution,
                format!("no solution within {searched}: {detail}"),
                n_det,
                centres_tried,
                best.cloned(),
            ))
        }
    }
}
