use std::f64::consts::PI;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;

use crate::core::astrometry::catalog::GAIA_REFERENCE_EPOCH_YEAR;
use crate::core::astrometry::gwcs::sip_fit::lu_solve;
use crate::core::astrometry::local_solve::poisson_tail_log10;
use crate::core::astrometry::wcs::{pixel_center, pixel_edge_corners, WcsKind, WcsTransform};
use crate::math::exact_median_f64;
use crate::types::constants::MAD_TO_SIGMA;

pub const GAIA_DR3_ATTRIBUTION_BASE: &str = "Gaia DR3 (Gaia Collaboration, Vallenari et al. 2023, A&A 674, A1), ESA/Gaia/DPAC, CC BY-NC 3.0 IGO";
pub const GAIA_DR3_ATTRIBUTION: &str = "Gaia DR3 (Gaia Collaboration, Vallenari et al. 2023, A&A 674, A1), ESA/Gaia/DPAC, CC BY-NC 3.0 IGO, via VizieR (CDS)";
pub const GAIA_DR3_ATTRIBUTION_SIRIL: &str = "Gaia DR3 (Gaia Collaboration, Vallenari et al. 2023, A&A 674, A1), ESA/Gaia/DPAC, CC BY-NC 3.0 IGO, via the Siril extract (Zenodo 14692304)";
pub const GAIA_ESA_ACKNOWLEDGEMENT: &str = "This work has made use of data from the European Space Agency (ESA) mission Gaia (https://www.cosmos.esa.int/gaia), processed by the Gaia Data Processing and Analysis Consortium (DPAC, https://www.cosmos.esa.int/web/gaia/dpac/consortium). Funding for the DPAC has been provided by national institutions, in particular the institutions participating in the Gaia Multilateral Agreement.";
pub const GAIA_DR3_CATALOGUE: &str = "Gaia DR3 (I/355/gaiadr3) via VizieR, CDS";
pub const GAIA_LICENCE: &str = "CC BY-NC 3.0 IGO";

pub const DEFAULT_SEARCH_RADIUS_ARCSEC: f64 = 6.0;
pub const DEFAULT_CHANCE_FRACTION: f64 = 0.02;
pub const DEFAULT_MAX_DETECTIONS: usize = 500;
pub const DEFAULT_EPOCH_UNCERTAINTY_YR: f64 = 10.0;
pub const BOOTSTRAP_RESAMPLES: usize = 200;
pub const CONVERGENCE_MAS: f64 = 0.1;
pub const MAX_PASSES: usize = 3;
pub const CLIP_SIGMA: f64 = 3.0;
pub const CLIP_PASSES: usize = 3;
pub const RUWE_LIMIT: f64 = 1.4;
pub const SEPSI_STRICT_LIMIT: f64 = 2.0;
pub const IPD_MULTI_PEAK_LIMIT: f64 = 2.0;
pub const BRIGHT_LIMIT_G: f64 = 5.0;
pub const CENTROID_CRB_FACTOR: f64 = 0.6;
pub const SIGMA_FLOOR_SAMPLED_PX: f64 = 0.01;
pub const SIGMA_FLOOR_UNDERSAMPLED_PX: f64 = 0.02;
pub const UNDERSAMPLED_FWHM_PX: f64 = 2.0;
pub const BLEND_FWHM_MULTIPLE: f64 = 2.0;
pub const HISTOGRAM_BIN_MIN_PX: f64 = 2.0;
pub const HISTOGRAM_BIN_MIN_ARCSEC: f64 = 0.2;
pub const PEAK_MIN_COUNT: f64 = 5.0;
pub const PEAK_SIGMA: f64 = 5.0;
pub const MATCH_RADIUS_SIGMAS: f64 = 5.0;
pub const MAGNITUDE_OUTLIER_FLOOR: f64 = 1.0;
pub const MAGNITUDE_OUTLIER_SIGMA: f64 = 4.0;
pub const MAGNITUDE_WINDOW_MARGIN: f64 = 0.5;
pub const LEVER_ARM_FRACTION: f64 = 0.2;
pub const MAX_ROTATION_DEG: f64 = 0.5;
pub const MAX_SCALE_DEVIATION: f64 = 1e-3;
pub const MAX_GENERAL_DEVIATION: f64 = 1e-2;
pub const CV_FOLDS: usize = 5;
pub const GRID_SIZE: usize = 16;
pub const SOLVED_FIVE_PARAMETER: u8 = 31;
pub const SOLVED_SIX_PARAMETER: u8 = 95;

const RAD_TO_ARCSEC: f64 = 180.0 * 3600.0 / PI;
const MAS_PER_ARCSEC: f64 = 1000.0;
const MAS_PER_DEG: f64 = 3_600_000.0;
const EXACT_CARD_TOLERANCE: f64 = 1e-7;
const PIVOT_MATCH_DEG: f64 = 1e-9;
const JACOBIAN_STEP_PX: f64 = 0.5;
const MEDIAN_SIGMA_FLOOR_MAS: f64 = 0.01;
const MEDIAN_TO_SIGMA: f64 = 1.2533;
const BOOTSTRAP_DISAGREEMENT: f64 = 2.0;
const BOOTSTRAP_REFUSAL_RATIO: f64 = 3.0;
const BOOTSTRAP_FAILURE_FRACTION: f64 = 0.01;
const SIGMA_SYS_ITERATIONS: usize = 40;
const REWEIGHT_ROUNDS: usize = 2;
const MIN_MAGNITUDE_FIT_PAIRS: usize = 10;
const MIN_WINDOW_PAIRS: usize = 10;
const MAX_BLEND_WINDOW_PX: f64 = 20.0;
const CRITICAL_95: f64 = 1.96;
const ICRS: &str = "ICRS";
const CARDS_REPLACED_BY_CD: [&str; 8] = ["PC1_1", "PC1_2", "PC2_1", "PC2_2", "CDELT1", "CDELT2", "CROTA1", "CROTA2"];
const CARDS_REPLACED_BY_RADESYS: [&str; 2] = ["EQUINOX", "RADECSYS"];
const PEAK_GRIDS: [[f64; 2]; 4] = [[0.0, 0.0], [0.5, 0.0], [0.0, 0.5], [0.5, 0.5]];
const PEAK_FALSE_ALARM: f64 = 1e-3;
const CHANCE_EXCESS_SIGMA: f64 = 5.0;
const FLAG_CLIPPED: &str = "clipped";
const FLAG_NO_PROPER_MOTION: &str = "no proper motion";
const FLAG_PARALLAX_APPLIED: &str = "parallax applied";
const FLAG_PARALLAX_INFLATED: &str = "parallax inflated";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TieModel {
    Shift,
    Rshift,
    Rscale,
    General,
}

impl TieModel {
    pub fn name(self) -> &'static str {
        match self {
            TieModel::Shift => "shift",
            TieModel::Rshift => "rshift",
            TieModel::Rscale => "rscale",
            TieModel::General => "general",
        }
    }

    pub fn min_pairs(self) -> usize {
        match self {
            TieModel::Shift => 10,
            TieModel::Rshift => 15,
            TieModel::Rscale => 20,
            TieModel::General => 30,
        }
    }

    fn parameter_count(self) -> usize {
        match self {
            TieModel::Shift => 2,
            TieModel::Rshift => 3,
            TieModel::Rscale => 4,
            TieModel::General => 6,
        }
    }

    fn min_points(self) -> usize {
        self.parameter_count() / 2 + 1
    }

    fn downgrade(self) -> Option<TieModel> {
        match self {
            TieModel::General => Some(TieModel::Rscale),
            TieModel::Rscale => Some(TieModel::Rshift),
            TieModel::Rshift => Some(TieModel::Shift),
            TieModel::Shift => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelChoice {
    Auto,
    Fixed(TieModel),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CatalogueKind {
    GaiaDr3Vizier,
    GaiaDr3Siril,
    LocalTycho2,
    Local2mass,
    Other(String),
}

impl CatalogueKind {
    fn label(&self) -> String {
        match self {
            CatalogueKind::GaiaDr3Vizier => GAIA_DR3_CATALOGUE.to_string(),
            CatalogueKind::GaiaDr3Siril => "Siril Gaia DR3 extract (Zenodo 14692304)".to_string(),
            CatalogueKind::LocalTycho2 => "local tier 0 (Tycho-2)".to_string(),
            CatalogueKind::Local2mass => "local tier 1 (2MASS)".to_string(),
            CatalogueKind::Other(name) => name.clone(),
        }
    }

    fn attribution(&self) -> &'static str {
        match self {
            CatalogueKind::GaiaDr3Vizier => GAIA_DR3_ATTRIBUTION,
            CatalogueKind::GaiaDr3Siril => GAIA_DR3_ATTRIBUTION_SIRIL,
            _ => GAIA_DR3_ATTRIBUTION_BASE,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GaiaRow {
    pub id: String,
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub pm_ra_masyr: Option<f64>,
    pub pm_dec_masyr: Option<f64>,
    pub parallax_mas: Option<f64>,
    pub e_ra_mas: Option<f64>,
    pub e_dec_mas: Option<f64>,
    pub e_pm_ra_masyr: Option<f64>,
    pub e_pm_dec_masyr: Option<f64>,
    pub e_parallax_mas: Option<f64>,
    pub ra_pm_ra_corr: Option<f64>,
    pub dec_pm_dec_corr: Option<f64>,
    pub ruwe: Option<f64>,
    pub solved: Option<u8>,
    pub dup: Option<bool>,
    pub ipd_frac_multi_peak: Option<f64>,
    pub sepsi: Option<f64>,
    pub g: Option<f64>,
    pub bp_rp: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct GaiaCatalogue {
    pub kind: CatalogueKind,
    pub rows: Vec<GaiaRow>,
    pub quality_columns: bool,
    pub cache_source: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TieDetection {
    pub x: f64,
    pub y: f64,
    pub fwhm_px: f64,
    pub snr: f64,
    pub mag_inst: f64,
    pub saturated: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EpochSource {
    Header,
    Typed,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Epoch {
    pub year: Option<f64>,
    pub source: EpochSource,
    pub assumed_uncertainty_yr: f64,
}

impl Epoch {
    pub fn header(year: f64) -> Self {
        Self { year: Some(year), source: EpochSource::Header, assumed_uncertainty_yr: DEFAULT_EPOCH_UNCERTAINTY_YR }
    }

    pub fn typed(year: f64) -> Self {
        Self { year: Some(year), source: EpochSource::Typed, assumed_uncertainty_yr: DEFAULT_EPOCH_UNCERTAINTY_YR }
    }

    pub fn none() -> Self {
        Self { year: None, source: EpochSource::None, assumed_uncertainty_yr: DEFAULT_EPOCH_UNCERTAINTY_YR }
    }
}

pub struct TieInput<'a> {
    pub wcs: &'a WcsTransform,
    pub width: usize,
    pub height: usize,
    pub detections: &'a [TieDetection],
    pub catalogue: &'a GaiaCatalogue,
    pub epoch: Epoch,
    pub observer_barycentric_au: Option<[f64; 3]>,
    pub space_telescope: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TieOptions {
    pub model: ModelChoice,
    pub search_radius_arcsec: f64,
    pub strict_sepsi: bool,
    pub ipd_cut: bool,
    pub override_source: bool,
    pub max_detections: usize,
    pub bootstrap_resamples: usize,
    pub seed: u64,
    pub gaia_inflation: f64,
    pub sigma_floor_px: Option<f64>,
    pub resampled_pixels: bool,
    pub propagate_proper_motion: bool,
    pub strict_mode: bool,
}

impl Default for TieOptions {
    fn default() -> Self {
        Self {
            model: ModelChoice::Auto,
            search_radius_arcsec: DEFAULT_SEARCH_RADIUS_ARCSEC,
            strict_sepsi: false,
            ipd_cut: false,
            override_source: false,
            max_detections: DEFAULT_MAX_DETECTIONS,
            bootstrap_resamples: BOOTSTRAP_RESAMPLES,
            seed: 0x0C21_5EED,
            gaia_inflation: 1.0,
            sigma_floor_px: None,
            resampled_pixels: false,
            propagate_proper_motion: true,
            strict_mode: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Params {
    pub shift_mas: [f64; 2],
    pub rotation_deg: f64,
    pub scale_ppm: f64,
    pub aspect_ppm: f64,
    pub skew_deg: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct ResidualSummary {
    pub median_mas: [f64; 2],
    pub robust_rms_mas: [f64; 3],
    pub weighted_rms_mas: f64,
    pub n: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StarResidual {
    pub id: String,
    pub g: Option<f64>,
    pub bp_rp: Option<f64>,
    pub x: f64,
    pub y: f64,
    pub before_mas: [f64; 2],
    pub after_mas: [f64; 2],
    pub weight: f64,
    pub used: bool,
    pub flags: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Slope {
    pub against: &'static str,
    pub axis: &'static str,
    pub value: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CutCounts {
    pub rows_in: usize,
    pub invalid_rows: usize,
    pub two_parameter: usize,
    pub ruwe: usize,
    pub sepsi: usize,
    pub dup: usize,
    pub ipd: usize,
    pub bright: usize,
    pub rows_used: usize,
    pub detections_in: usize,
    pub invalid_detections: usize,
    pub saturated: usize,
    pub blended: usize,
    pub beyond_brightest: usize,
    pub detections_used: usize,
    pub ambiguous: usize,
    pub magnitude_outliers: usize,
    pub clipped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TieDecision {
    Applied,
    NoSignificantCorrection,
    Refused,
}

impl TieDecision {
    pub fn name(self) -> &'static str {
        match self {
            TieDecision::Applied => "applied",
            TieDecision::NoSignificantCorrection => "no significant correction",
            TieDecision::Refused => "refused",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeaderUpdate {
    pub crval1: f64,
    pub crval2: f64,
    pub cd: [[f64; 2]; 2],
    pub cards: Vec<(String, String)>,
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TieReport {
    pub wcs_kind: String,
    pub wcs_origin: Option<String>,
    pub banners: Vec<String>,
    pub representation: String,
    pub pivot_deg: [f64; 2],
    pub epoch_year: f64,
    pub epoch_source: EpochSource,
    pub delta_t_yr: f64,
    pub proper_motion: String,
    pub parallax: String,
    pub observer_source: Option<String>,
    pub epoch_systematic_mas: Option<f64>,
    pub catalogue: String,
    pub catalogue_rows: usize,
    pub quality_columns: bool,
    pub cache_source: String,
    pub cone_centre_deg: Option<[f64; 2]>,
    pub cone_radius_deg: Option<f64>,
    pub g_range: Option<[f64; 2]>,
    pub cuts: CutCounts,
    pub search_radius_arcsec: f64,
    pub histogram_bin_arcsec: f64,
    pub histogram_peak: usize,
    pub histogram_random_per_bin: f64,
    pub histogram_threshold: f64,
    pub histogram_expected_false_peaks: Option<f64>,
    pub peak_offset_arcsec: [f64; 2],
    pub sigma_res_arcsec: f64,
    pub density_per_arcsec2: f64,
    pub chance_radius_arcsec: f64,
    pub match_radius_arcsec: f64,
    pub chance_fraction: f64,
    pub n_matched: usize,
    pub expected_chance_pairs: Option<f64>,
    pub n_used: usize,
    pub model: Option<String>,
    pub model_notes: Vec<String>,
    pub params: Params,
    pub rotation_arcsec: f64,
    pub sigma_formal: Params,
    pub sigma_bootstrap: Option<Params>,
    pub bootstrap_failures: usize,
    pub errors_disagree: bool,
    pub sigma_sys_mas: f64,
    pub chi2_nu: f64,
    pub passes: usize,
    pub last_update_mas: f64,
    pub before: ResidualSummary,
    pub after: ResidualSummary,
    pub cv_rms_before_mas: f64,
    pub cv_rms_after_mas: f64,
    pub cv_median_after_mas: [f64; 2],
    pub cv_median_sigma_mas: [f64; 2],
    pub stars: Vec<StarResidual>,
    pub slopes: Vec<Slope>,
    pub uncertainty_centre_mas: f64,
    pub uncertainty_corner_mas: f64,
    pub warnings: Vec<String>,
    pub attribution: String,
    pub acknowledgement: String,
}

impl TieReport {
    fn new(catalogue: &GaiaCatalogue) -> Self {
        Self {
            catalogue: catalogue.kind.label(),
            catalogue_rows: catalogue.rows.len(),
            quality_columns: catalogue.quality_columns,
            cache_source: catalogue.cache_source.clone(),
            attribution: catalogue.kind.attribution().to_string(),
            acknowledgement: GAIA_ESA_ACKNOWLEDGEMENT.to_string(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct TieResult {
    pub decision: TieDecision,
    pub reason: Option<String>,
    pub report: TieReport,
    pub correction: Option<SkyCorrection>,
    pub header_update: Option<HeaderUpdate>,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn scale3(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let n = dot(a, a).sqrt();
    if n > 0.0 {
        scale3(a, 1.0 / n)
    } else {
        a
    }
}

pub fn unit_vector(ra_deg: f64, dec_deg: f64) -> [f64; 3] {
    let (sa, ca) = ra_deg.to_radians().sin_cos();
    let (sd, cd) = dec_deg.to_radians().sin_cos();
    [cd * ca, cd * sa, sd]
}

pub fn ra_dec_deg(v: [f64; 3]) -> (f64, f64) {
    let ra = v[1].atan2(v[0]).to_degrees().rem_euclid(360.0);
    let dec = v[2].clamp(-1.0, 1.0).asin().to_degrees();
    (ra, dec)
}

fn rotate_about(v: [f64; 3], axis: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    let kv = cross(axis, v);
    let kdv = dot(axis, v);
    add3(add3(scale3(v, c), scale3(kv, s)), scale3(axis, kdv * (1.0 - c)))
}

#[derive(Debug, Clone, Copy)]
struct Tangent {
    p: [f64; 3],
    e: [f64; 3],
    n: [f64; 3],
}

impl Tangent {
    fn at(p: [f64; 3]) -> Self {
        let ra = p[1].atan2(p[0]);
        let dec = p[2].clamp(-1.0, 1.0).asin();
        let (sa, ca) = ra.sin_cos();
        let (sd, cd) = dec.sin_cos();
        Self { p, e: [-sa, ca, 0.0], n: [-sd * ca, -sd * sa, cd] }
    }

    fn project(&self, v: [f64; 3]) -> Option<[f64; 2]> {
        let d = dot(v, self.p);
        (d > 1e-12).then(|| [dot(v, self.e) / d, dot(v, self.n) / d])
    }

    fn unproject(&self, xi: f64, eta: f64) -> [f64; 3] {
        normalize(add3(self.p, add3(scale3(self.e, xi), scale3(self.n, eta))))
    }
}

type Mat2 = [[f64; 2]; 2];

const IDENTITY2: Mat2 = [[1.0, 0.0], [0.0, 1.0]];

fn mul2(a: Mat2, b: Mat2) -> Mat2 {
    [
        [a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]],
        [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]],
    ]
}

fn apply2(a: Mat2, v: [f64; 2]) -> [f64; 2] {
    [a[0][0] * v[0] + a[0][1] * v[1], a[1][0] * v[0] + a[1][1] * v[1]]
}

fn transpose2(a: Mat2) -> Mat2 {
    [[a[0][0], a[1][0]], [a[0][1], a[1][1]]]
}

fn det2(a: Mat2) -> f64 {
    a[0][0] * a[1][1] - a[0][1] * a[1][0]
}

fn inv2(a: Mat2) -> Option<Mat2> {
    let d = det2(a);
    (d.is_finite() && d != 0.0).then(|| [[a[1][1] / d, -a[0][1] / d], [-a[1][0] / d, a[0][0] / d]])
}

fn scaled2(a: Mat2, s: f64) -> Mat2 {
    [[a[0][0] * s, a[0][1] * s], [a[1][0] * s, a[1][1] * s]]
}

fn rotation2(theta: f64) -> Mat2 {
    let (s, c) = theta.sin_cos();
    [[c, s], [-s, c]]
}

fn sub2(a: Mat2, b: Mat2) -> Mat2 {
    [[a[0][0] - b[0][0], a[0][1] - b[0][1]], [a[1][0] - b[1][0], a[1][1] - b[1][1]]]
}

fn frobenius(a: Mat2) -> f64 {
    (a[0][0] * a[0][0] + a[0][1] * a[0][1] + a[1][0] * a[1][0] + a[1][1] * a[1][1]).sqrt()
}

fn sub_v(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn add_v(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn norm_v(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}

fn transport(from: &Tangent, to: &Tangent) -> Mat2 {
    let axis = cross(from.p, to.p);
    let s = dot(axis, axis).sqrt();
    if s < 1e-18 {
        return IDENTITY2;
    }
    let k = scale3(axis, 1.0 / s);
    let angle = s.atan2(dot(from.p, to.p));
    let et = rotate_about(from.e, k, angle);
    let nt = rotate_about(from.n, k, angle);
    [[dot(et, to.e), dot(nt, to.e)], [dot(et, to.n), dot(nt, to.n)]]
}

fn matrix_rotation_rad(a: Mat2) -> f64 {
    (a[0][1] - a[1][0]).atan2(a[0][0] + a[1][1])
}

fn matrix_scale(a: Mat2) -> f64 {
    det2(a).abs().sqrt()
}

fn matrix_aspect_ppm(a: Mat2) -> f64 {
    let col1 = a[0][0].hypot(a[1][0]);
    let col2 = a[0][1].hypot(a[1][1]);
    if col2 > 0.0 {
        (col1 / col2 - 1.0) * 1e6
    } else {
        0.0
    }
}

fn matrix_skew_deg(a: Mat2) -> f64 {
    let col1 = a[0][0].hypot(a[1][0]);
    let col2 = a[0][1].hypot(a[1][1]);
    if col1 > 0.0 && col2 > 0.0 {
        let cosine = ((a[0][0] * a[0][1] + a[1][0] * a[1][1]) / (col1 * col2)).clamp(-1.0, 1.0);
        cosine.acos().to_degrees() - 90.0
    } else {
        0.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkyCorrection {
    pivot_in: [f64; 3],
    pivot_out: [f64; 3],
    matrix: Mat2,
}

impl SkyCorrection {
    pub fn identity(pivot: [f64; 3]) -> Self {
        Self { pivot_in: pivot, pivot_out: pivot, matrix: IDENTITY2 }
    }

    pub fn from_parameters(pivot: [f64; 3], shift_arcsec: [f64; 2], rotation_deg: f64, scale: f64) -> Self {
        let a = scaled2(rotation2(rotation_deg.to_radians()), scale);
        Self::identity(pivot).composed_with([shift_arcsec[0] / RAD_TO_ARCSEC, shift_arcsec[1] / RAD_TO_ARCSEC], a)
    }

    fn composed_with(&self, shift_rad: [f64; 2], a: Mat2) -> Self {
        let out = Tangent::at(self.pivot_out);
        let next = Tangent::at(out.unproject(shift_rad[0], shift_rad[1]));
        let b = transport(&out, &next);
        Self { pivot_in: self.pivot_in, pivot_out: next.p, matrix: mul2(b, mul2(a, self.matrix)) }
    }

    pub fn apply(&self, ra_deg: f64, dec_deg: f64) -> Option<(f64, f64)> {
        let plane = Tangent::at(self.pivot_in).project(unit_vector(ra_deg, dec_deg))?;
        let m = apply2(self.matrix, plane);
        Some(ra_dec_deg(Tangent::at(self.pivot_out).unproject(m[0], m[1])))
    }

    pub fn corrected_pixel_to_world(&self, wcs: &WcsTransform, x: f64, y: f64) -> Option<(f64, f64)> {
        let sky = wcs.pixel_to_world(x, y);
        if !(sky.ra.is_finite() && sky.dec.is_finite()) {
            return None;
        }
        self.apply(sky.ra, sky.dec)
    }

    pub fn transported_matrix(&self) -> Mat2 {
        let b = transport(&Tangent::at(self.pivot_in), &Tangent::at(self.pivot_out));
        mul2(transpose2(b), self.matrix)
    }

    pub fn is_identity(&self) -> bool {
        self.pivot_in == self.pivot_out && self.matrix == IDENTITY2
    }
}

pub fn header_update(correction: &SkyCorrection, cd_native: Mat2) -> HeaderUpdate {
    let (crval1, crval2) = ra_dec_deg(correction.pivot_out);
    let cd = mul2(correction.matrix, cd_native);
    let card = |v: f64| format!("{v:.16E}");
    let cards = vec![
        ("CRVAL1".to_string(), card(crval1)),
        ("CRVAL2".to_string(), card(crval2)),
        ("CD1_1".to_string(), card(cd[0][0])),
        ("CD1_2".to_string(), card(cd[0][1])),
        ("CD2_1".to_string(), card(cd[1][0])),
        ("CD2_2".to_string(), card(cd[1][1])),
        ("RADESYS".to_string(), ICRS.to_string()),
    ];
    let remove = CARDS_REPLACED_BY_CD.iter().chain(CARDS_REPLACED_BY_RADESYS.iter()).map(|k| k.to_string()).collect();
    HeaderUpdate { crval1, crval2, cd, cards, remove }
}

pub fn parallax_displacement_mas(ra_deg: f64, dec_deg: f64, parallax_mas: f64, observer_au: [f64; 3]) -> [f64; 2] {
    let (sa, ca) = ra_deg.to_radians().sin_cos();
    let (sd, cd) = dec_deg.to_radians().sin_cos();
    let [bx, by, bz] = observer_au;
    [
        parallax_mas * (bx * sa - by * ca),
        parallax_mas * (bx * ca * sd + by * sa * sd - bz * cd),
    ]
}

pub fn typical_gaia_sigma_mas(g: f64) -> (f64, f64) {
    const KNOTS: [(f64, f64, f64); 4] = [(15.0, 0.025, 0.025), (17.0, 0.05, 0.07), (20.0, 0.4, 0.5), (21.0, 1.0, 1.4)];
    if !g.is_finite() || g <= KNOTS[0].0 {
        return (KNOTS[0].1, KNOTS[0].2);
    }
    for pair in KNOTS.windows(2) {
        let (g0, p0, m0) = pair[0];
        let (g1, p1, m1) = pair[1];
        if g <= g1 {
            let f = (g - g0) / (g1 - g0);
            let interp = |a: f64, b: f64| 10f64.powf(a.log10() + f * (b.log10() - a.log10()));
            return (interp(p0, p1), interp(m0, m1));
        }
    }
    let (_, p, m) = KNOTS[3];
    let f = (g - 21.0).clamp(0.0, 2.0);
    (p * 10f64.powf(0.4 * f), m * 10f64.powf(0.4 * f))
}

pub fn propagate_linear(ra_deg: f64, dec_deg: f64, pm_ra_masyr: f64, pm_dec_masyr: f64, dt_yr: f64) -> (f64, f64) {
    let cos_dec = dec_deg.to_radians().cos();
    let d_ra = if cos_dec.abs() > 1e-12 { pm_ra_masyr * dt_yr / MAS_PER_DEG / cos_dec } else { 0.0 };
    ((ra_deg + d_ra).rem_euclid(360.0), (dec_deg + pm_dec_masyr * dt_yr / MAS_PER_DEG).clamp(-90.0, 90.0))
}

pub fn chance_radius_arcsec(density_per_arcsec2: f64, fraction: f64) -> f64 {
    if density_per_arcsec2 > 0.0 && fraction > 0.0 && fraction < 1.0 {
        (-(1.0 - fraction).ln() / (PI * density_per_arcsec2)).sqrt()
    } else {
        f64::INFINITY
    }
}

fn chance_fraction(radius_arcsec: f64, density_per_arcsec2: f64) -> f64 {
    1.0 - (-PI * radius_arcsec * radius_arcsec * density_per_arcsec2).exp()
}

#[derive(Clone)]
struct GaiaStar {
    index: usize,
    v: [f64; 3],
    plane0: [f64; 2],
    sigma2: f64,
    g: Option<f64>,
    bp_rp: Option<f64>,
    pm: [f64; 2],
    pm_known: bool,
    parallax_known: bool,
}

struct Det {
    x: f64,
    y: f64,
    plane0: [f64; 2],
    sigma2: f64,
    mag_inst: f64,
    fwhm_arcsec: f64,
}

#[derive(Debug, Clone, Copy)]
struct PlaneModel {
    a: Mat2,
    t: [f64; 2],
    c: [f64; 2],
}

impl PlaneModel {
    fn predict(&self, p: [f64; 2]) -> [f64; 2] {
        add_v(add_v(apply2(self.a, sub_v(p, self.c)), self.c), self.t)
    }

    fn residual(&self, p: [f64; 2], q: [f64; 2]) -> [f64; 2] {
        sub_v(q, self.predict(p))
    }

    fn shift_at_origin(&self) -> [f64; 2] {
        add_v(self.t, sub_v(self.c, apply2(self.a, self.c)))
    }

    fn shift_about(&self, c: [f64; 2]) -> [f64; 2] {
        add_v(self.t, apply2(sub2(self.a, IDENTITY2), sub_v(c, self.c)))
    }
}

struct ClippedFit {
    model: PlaneModel,
    inliers: Vec<bool>,
    weights: Vec<f64>,
    sigma_sys2: f64,
    chi2_nu: f64,
    n_used: usize,
}

fn fit_plane(kind: TieModel, p: &[[f64; 2]], q: &[[f64; 2]], inliers: &[bool], w: &[f64]) -> Option<PlaneModel> {
    let mut sw = 0.0;
    let mut cp = [0.0; 2];
    let mut cq = [0.0; 2];
    for i in 0..p.len() {
        if !inliers[i] {
            continue;
        }
        sw += w[i];
        cp = add_v(cp, [w[i] * p[i][0], w[i] * p[i][1]]);
        cq = add_v(cq, [w[i] * q[i][0], w[i] * q[i][1]]);
    }
    if sw <= 0.0 || sw.is_nan() {
        return None;
    }
    cp = [cp[0] / sw, cp[1] / sw];
    cq = [cq[0] / sw, cq[1] / sw];
    let t = sub_v(cq, cp);
    let a = match kind {
        TieModel::Shift => IDENTITY2,
        TieModel::Rshift | TieModel::Rscale => {
            let mut sxx = 0.0;
            let mut sxy = 0.0;
            let mut spp = 0.0;
            for i in 0..p.len() {
                if !inliers[i] {
                    continue;
                }
                let pp = sub_v(p[i], cp);
                let qq = sub_v(q[i], cq);
                sxx += w[i] * (qq[0] * pp[0] + qq[1] * pp[1]);
                sxy += w[i] * (qq[0] * pp[1] - qq[1] * pp[0]);
                spp += w[i] * (pp[0] * pp[0] + pp[1] * pp[1]);
            }
            if spp <= 0.0 || spp.is_nan() {
                return None;
            }
            let theta = sxy.atan2(sxx);
            let s = if kind == TieModel::Rscale { (sxx * theta.cos() + sxy * theta.sin()) / spp } else { 1.0 };
            if !(s.is_finite() && s > 0.0) {
                return None;
            }
            scaled2(rotation2(theta), s)
        }
        TieModel::General => {
            let mut spp = [[0.0; 2]; 2];
            let mut sqp = [[0.0; 2]; 2];
            for i in 0..p.len() {
                if !inliers[i] {
                    continue;
                }
                let pp = sub_v(p[i], cp);
                let qq = sub_v(q[i], cq);
                for r in 0..2 {
                    for c in 0..2 {
                        spp[r][c] += w[i] * pp[r] * pp[c];
                        sqp[r][c] += w[i] * qq[r] * pp[c];
                    }
                }
            }
            mul2(sqp, inv2(spp)?)
        }
    };
    let finite = a.iter().flatten().all(|v| v.is_finite()) && t.iter().all(|v| v.is_finite());
    finite.then_some(PlaneModel { a, t, c: cp })
}

fn solve_sigma_sys2(model: &PlaneModel, p: &[[f64; 2]], q: &[[f64; 2]], sigma2: &[f64], inliers: &[bool], n_par: usize) -> f64 {
    let mut r2 = Vec::new();
    let mut s2 = Vec::new();
    for i in 0..p.len() {
        if inliers[i] {
            let r = model.residual(p[i], q[i]);
            r2.push(r[0] * r[0] + r[1] * r[1]);
            s2.push(sigma2[i]);
        }
    }
    let dof = 2.0 * r2.len() as f64 - n_par as f64;
    if dof <= 0.0 {
        return 0.0;
    }
    let f = |x: f64| r2.iter().zip(&s2).map(|(r, s)| r / (s + x)).sum::<f64>() - dof;
    if f(0.0) <= 0.0 {
        return 0.0;
    }
    let mut hi = r2.iter().cloned().fold(0.0, f64::max).max(1e-20);
    let mut guard = 0;
    while f(hi) > 0.0 && guard < 60 {
        hi *= 2.0;
        guard += 1;
    }
    let mut lo = 0.0;
    for _ in 0..SIGMA_SYS_ITERATIONS {
        let mid = 0.5 * (lo + hi);
        if f(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

fn fit_with_clipping(kind: TieModel, p: &[[f64; 2]], q: &[[f64; 2]], sigma2: &[f64]) -> Option<ClippedFit> {
    let n = p.len();
    if n < kind.min_points() {
        return None;
    }
    let n_par = kind.parameter_count();
    let mut inliers = vec![true; n];
    let mut sigma_sys2 = 0.0;
    let mut weights = vec![0.0; n];
    let mut model = None;
    let noise_floor2 = {
        let mut s: Vec<f64> = sigma2.to_vec();
        s.sort_by(f64::total_cmp);
        2.0 * exact_median_f64(&s)
    };
    for pass in 0..=CLIP_PASSES {
        for _ in 0..REWEIGHT_ROUNDS {
            for i in 0..n {
                weights[i] = 1.0 / (sigma2[i] + sigma_sys2);
            }
            let m = fit_plane(kind, p, q, &inliers, &weights)?;
            sigma_sys2 = solve_sigma_sys2(&m, p, q, sigma2, &inliers, n_par);
        }
        for i in 0..n {
            weights[i] = 1.0 / (sigma2[i] + sigma_sys2);
        }
        let m = fit_plane(kind, p, q, &inliers, &weights)?;
        model = Some(m);
        if pass == CLIP_PASSES {
            break;
        }
        let r2: Vec<f64> = (0..n).map(|i| {
            let r = m.residual(p[i], q[i]);
            r[0] * r[0] + r[1] * r[1]
        }).collect();
        let mut in_r2: Vec<f64> = (0..n).filter(|&i| inliers[i]).map(|i| r2[i]).collect();
        in_r2.sort_by(f64::total_cmp);
        let robust_rmse2 = exact_median_f64(&in_r2) / std::f64::consts::LN_2;
        let threshold2 = CLIP_SIGMA * CLIP_SIGMA * robust_rmse2.max(noise_floor2);
        let next: Vec<bool> = r2.iter().map(|&v| v <= threshold2).collect();
        if next.iter().filter(|&&b| b).count() < kind.min_points() {
            break;
        }
        inliers = next;
    }
    let model = model?;
    let mut chi2 = 0.0;
    let mut n_used = 0usize;
    for i in 0..n {
        if inliers[i] {
            let r = model.residual(p[i], q[i]);
            chi2 += weights[i] * (r[0] * r[0] + r[1] * r[1]);
            n_used += 1;
        }
    }
    let dof = (2 * n_used).saturating_sub(n_par);
    let chi2_nu = if dof > 0 { chi2 / dof as f64 } else { 0.0 };
    Some(ClippedFit { model, inliers, weights, sigma_sys2, chi2_nu, n_used })
}

type Jacobian = [f64; 6];

fn model_jacobian(kind: TieModel, a: Mat2, pp: [f64; 2]) -> (Jacobian, Jacobian) {
    let mut jx = [0.0; 6];
    let mut jy = [0.0; 6];
    jx[0] = 1.0;
    jy[1] = 1.0;
    match kind {
        TieModel::Shift => {}
        TieModel::Rshift | TieModel::Rscale => {
            let s = matrix_scale(a);
            let (st, ct) = matrix_rotation_rad(a).sin_cos();
            jx[2] = s * (-st * pp[0] + ct * pp[1]);
            jy[2] = s * (-ct * pp[0] - st * pp[1]);
            if kind == TieModel::Rscale {
                jx[3] = ct * pp[0] + st * pp[1];
                jy[3] = -st * pp[0] + ct * pp[1];
            }
        }
        TieModel::General => {
            jx[2] = pp[0];
            jx[3] = pp[1];
            jy[4] = pp[0];
            jy[5] = pp[1];
        }
    }
    (jx, jy)
}

fn formal_covariance(kind: TieModel, fit: &ClippedFit, p: &[[f64; 2]]) -> Option<Vec<Vec<f64>>> {
    let n_par = kind.parameter_count();
    let mut normal = vec![vec![0.0; n_par]; n_par];
    for (i, pi) in p.iter().enumerate() {
        if !fit.inliers[i] {
            continue;
        }
        let (jx, jy) = model_jacobian(kind, fit.model.a, sub_v(*pi, fit.model.c));
        let w = fit.weights[i];
        for r in 0..n_par {
            for c in 0..n_par {
                normal[r][c] += w * (jx[r] * jx[c] + jy[r] * jy[c]);
            }
        }
    }
    let scale = fit.chi2_nu.max(1.0);
    let mut cov = vec![vec![0.0; n_par]; n_par];
    for k in 0..n_par {
        let mut m = normal.clone();
        let mut rhs = vec![0.0; n_par];
        rhs[k] = 1.0;
        lu_solve(&mut m, &mut rhs)?;
        for (r, value) in rhs.iter().enumerate() {
            cov[r][k] = scale * value;
        }
    }
    Some(cov)
}

fn quadratic_form(cov: &[Vec<f64>], j: &Jacobian) -> f64 {
    let mut sum = 0.0;
    for (r, row) in cov.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            sum += j[r] * value * j[c];
        }
    }
    sum.max(0.0)
}

fn params_from_matrix(a: Mat2, shift_mas: [f64; 2]) -> Params {
    Params {
        shift_mas,
        rotation_deg: matrix_rotation_rad(a).to_degrees(),
        scale_ppm: (matrix_scale(a) - 1.0) * 1e6,
        aspect_ppm: matrix_aspect_ppm(a),
        skew_deg: matrix_skew_deg(a),
    }
}

fn formal_params(kind: TieModel, cov: &[Vec<f64>], a: Mat2, centre_from_centroid: [f64; 2]) -> Params {
    let (jx, jy) = model_jacobian(kind, a, centre_from_centroid);
    let shift = [quadratic_form(cov, &jx).sqrt() * MAS_PER_ARCSEC, quadratic_form(cov, &jy).sqrt() * MAS_PER_ARCSEC];
    let sigma = |k: usize| cov[k][k].max(0.0).sqrt();
    match kind {
        TieModel::Shift => Params { shift_mas: shift, ..Params::default() },
        TieModel::Rshift => Params { shift_mas: shift, rotation_deg: sigma(2).to_degrees(), ..Params::default() },
        TieModel::Rscale => Params { shift_mas: shift, rotation_deg: sigma(2).to_degrees(), scale_ppm: sigma(3) * 1e6, ..Params::default() },
        TieModel::General => {
            let rot = 0.5 * sigma(3).hypot(sigma(4));
            let sc = 0.5 * sigma(2).hypot(sigma(5));
            Params {
                shift_mas: shift,
                rotation_deg: rot.to_degrees(),
                scale_ppm: sc * 1e6,
                aspect_ppm: sigma(2).hypot(sigma(5)) * 1e6,
                skew_deg: sigma(3).hypot(sigma(4)).to_degrees(),
            }
        }
    }
}

fn general_deviation(a: Mat2) -> f64 {
    frobenius(sub2(mul2(a, rotation2(-matrix_rotation_rad(a))), IDENTITY2))
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = fraction * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

fn half_width_68(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    0.5 * (percentile(values, 0.84) - percentile(values, 0.16))
}

fn robust_sigma(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = exact_median_f64(&sorted);
    let mut dev: Vec<f64> = sorted.iter().map(|v| (v - median).abs()).collect();
    dev.sort_by(f64::total_cmp);
    MAD_TO_SIGMA * exact_median_f64(&dev)
}

fn median_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    exact_median_f64(&sorted)
}

fn summarize(residuals: &[[f64; 2]], weights: &[f64]) -> ResidualSummary {
    let xs: Vec<f64> = residuals.iter().map(|r| r[0] * MAS_PER_ARCSEC).collect();
    let ys: Vec<f64> = residuals.iter().map(|r| r[1] * MAS_PER_ARCSEC).collect();
    let rx = robust_sigma(&xs);
    let ry = robust_sigma(&ys);
    let mut sw = 0.0;
    let mut swr = 0.0;
    for (r, w) in residuals.iter().zip(weights) {
        sw += w;
        swr += w * (r[0] * r[0] + r[1] * r[1]);
    }
    ResidualSummary {
        median_mas: [median_of(&xs), median_of(&ys)],
        robust_rms_mas: [rx, ry, rx.hypot(ry)],
        weighted_rms_mas: if sw > 0.0 { (swr / sw).sqrt() * MAS_PER_ARCSEC } else { 0.0 },
        n: residuals.len(),
    }
}

fn weighted_slope(xs: &[f64], ys: &[f64], w: &[f64]) -> Option<(f64, f64)> {
    let n = xs.len();
    if n < 3 {
        return None;
    }
    let sw: f64 = w.iter().sum();
    if sw <= 0.0 || sw.is_nan() {
        return None;
    }
    let mx = xs.iter().zip(w).map(|(x, w)| x * w).sum::<f64>() / sw;
    let my = ys.iter().zip(w).map(|(y, w)| y * w).sum::<f64>() / sw;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for i in 0..n {
        sxx += w[i] * (xs[i] - mx) * (xs[i] - mx);
        sxy += w[i] * (xs[i] - mx) * (ys[i] - my);
    }
    if sxx <= 0.0 || sxx.is_nan() {
        return None;
    }
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let chi2: f64 = (0..n).map(|i| w[i] * (ys[i] - intercept - slope * xs[i]).powi(2)).sum();
    let chi2_nu = chi2 / (n - 2) as f64;
    Some((slope, (chi2_nu.max(1.0) / sxx).sqrt()))
}

struct PeakSearch {
    peak: usize,
    random_per_bin: f64,
    threshold: f64,
    expected_false_peaks: f64,
    bins: usize,
    offset: [f64; 2],
    sigma_res: f64,
    bin: f64,
}

fn peak_threshold(random_per_bin: f64, bins: usize) -> f64 {
    let gaussian = PEAK_MIN_COUNT.max(random_per_bin + PEAK_SIGMA * random_per_bin.sqrt());
    let limit = PEAK_FALSE_ALARM.log10() - (bins as f64).log10();
    let start = gaussian.ceil().max(1.0) as usize;
    let k = (start..).find(|&k| poisson_tail_log10(k, random_per_bin) <= limit).unwrap_or(start);
    gaussian.max(k as f64)
}

fn expected_false_peaks(peak: usize, random_per_bin: f64, bins: usize) -> f64 {
    10f64.powf((bins as f64).log10() + poisson_tail_log10(peak, random_per_bin))
}

fn chance_needed(expected: f64) -> f64 {
    expected + (CHANCE_EXCESS_SIGMA * expected.sqrt()).max(expected)
}

fn chance_excess(n_dets: usize, r_m: f64, density: f64) -> (f64, f64) {
    let expected = n_dets as f64 * chance_fraction(r_m, density);
    (expected, chance_needed(expected))
}

fn eta_sorted(stars: &[GaiaStar]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..stars.len()).collect();
    order.sort_by(|&a, &b| stars[a].plane0[1].total_cmp(&stars[b].plane0[1]));
    order
}

fn window<'a>(stars: &'a [GaiaStar], order: &'a [usize], eta: f64, radius: f64) -> impl Iterator<Item = usize> + 'a {
    let start = order.partition_point(|&i| stars[i].plane0[1] < eta - radius);
    order[start..].iter().cloned().take_while(move |&i| stars[i].plane0[1] <= eta + radius)
}

fn find_offset_peak(dets: &[Det], stars: &[GaiaStar], order: &[usize], r_s: f64, bin: f64, density: f64) -> PeakSearch {
    let n_bins = ((2.0 * r_s / bin).ceil() as usize).max(1) + 1;
    let grid_cells = n_bins * n_bins;
    let mut counts = vec![0usize; PEAK_GRIDS.len() * grid_cells];
    let mut offsets: Vec<[f64; 2]> = Vec::new();
    let cell = |v: f64, shift: f64| (((v + r_s) / bin + shift).floor() as isize).clamp(0, n_bins as isize - 1) as usize;
    for det in dets {
        for i in window(stars, order, det.plane0[1], r_s) {
            let d = sub_v(stars[i].plane0, det.plane0);
            if d[0].abs() > r_s || norm_v(d) > r_s {
                continue;
            }
            for (g, shift) in PEAK_GRIDS.iter().enumerate() {
                counts[g * grid_cells + cell(d[1], shift[1]) * n_bins + cell(d[0], shift[0])] += 1;
            }
            offsets.push(d);
        }
    }
    let (peak_index, &peak) = counts.iter().enumerate().max_by_key(|(_, &c)| c).unwrap_or((0, &0));
    let random_per_bin = dets.len() as f64 * density * bin * bin;
    let bins = counts.len();
    let threshold = peak_threshold(random_per_bin, bins);
    let expected_false_peaks = expected_false_peaks(peak, random_per_bin, bins);
    let shift = PEAK_GRIDS[peak_index / grid_cells];
    let cell_index = peak_index % grid_cells;
    let centre = [
        -r_s + (((cell_index % n_bins) as f64) + 0.5 - shift[0]) * bin,
        -r_s + (((cell_index / n_bins) as f64) + 0.5 - shift[1]) * bin,
    ];
    let near: Vec<[f64; 2]> = offsets.iter().cloned().filter(|d| norm_v(sub_v(*d, centre)) <= 1.5 * bin).collect();
    let xs: Vec<f64> = near.iter().map(|d| d[0]).collect();
    let ys: Vec<f64> = near.iter().map(|d| d[1]).collect();
    let offset = if near.is_empty() { centre } else { [median_of(&xs), median_of(&ys)] };
    let sigma_res = if near.len() >= 3 { robust_sigma(&xs).max(robust_sigma(&ys)) } else { 0.0 };
    PeakSearch { peak, random_per_bin, threshold, expected_false_peaks, bins, offset, sigma_res, bin }
}

struct Pair {
    det: usize,
    gaia: usize,
}

fn match_one_to_one(dets: &[Det], stars: &[GaiaStar], order: &[usize], offset: [f64; 2], r_m: f64) -> (Vec<Pair>, usize) {
    let mut claims: Vec<(f64, usize, usize)> = Vec::new();
    let mut ambiguous = 0;
    for (d, det) in dets.iter().enumerate() {
        let p = add_v(det.plane0, offset);
        let ambiguity_radius = r_m.max(BLEND_FWHM_MULTIPLE * det.fwhm_arcsec);
        let mut best: Option<(f64, usize)> = None;
        let mut second = f64::INFINITY;
        for i in window(stars, order, p[1], ambiguity_radius) {
            let sep = norm_v(sub_v(stars[i].plane0, p));
            if sep > ambiguity_radius {
                continue;
            }
            match best {
                Some((b, _)) if sep >= b => second = second.min(sep),
                Some((b, _)) => {
                    second = b;
                    best = Some((sep, i));
                }
                None => best = Some((sep, i)),
            }
        }
        let Some((sep, i)) = best else { continue };
        if sep > r_m {
            continue;
        }
        if second <= ambiguity_radius {
            ambiguous += 1;
            continue;
        }
        claims.push((sep, d, i));
    }
    claims.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut det_used = vec![false; dets.len()];
    let mut gaia_used = vec![false; stars.len()];
    let mut pairs = Vec::new();
    for (_, d, i) in claims {
        if det_used[d] || gaia_used[i] {
            continue;
        }
        det_used[d] = true;
        gaia_used[i] = true;
        pairs.push(Pair { det: d, gaia: i });
    }
    (pairs, ambiguous)
}

fn magnitude_consistent(pairs: &[Pair], dets: &[Det], stars: &[GaiaStar]) -> Vec<bool> {
    let usable: Vec<(usize, f64, f64)> = pairs
        .iter()
        .enumerate()
        .filter_map(|(k, pair)| {
            let m = dets[pair.det].mag_inst;
            let g = stars[pair.gaia].g?;
            (m.is_finite() && g.is_finite()).then_some((k, m, g))
        })
        .collect();
    let mut keep = vec![true; pairs.len()];
    if usable.len() < MIN_MAGNITUDE_FIT_PAIRS {
        return keep;
    }
    let mut mask = vec![true; usable.len()];
    let mut line = (0.0, 1.0);
    let mut sigma = 0.0;
    for _ in 0..3 {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for (k, (_, m, g)) in usable.iter().enumerate() {
            if mask[k] {
                xs.push(*m);
                ys.push(*g);
            }
        }
        let w = vec![1.0; xs.len()];
        let Some((slope, _)) = weighted_slope(&xs, &ys, &w) else { return keep };
        let mx = xs.iter().sum::<f64>() / xs.len() as f64;
        let my = ys.iter().sum::<f64>() / ys.len() as f64;
        line = (my - slope * mx, slope);
        let res: Vec<f64> = usable.iter().map(|(_, m, g)| g - line.0 - line.1 * m).collect();
        sigma = robust_sigma(&res);
        mask = res.iter().map(|r| r.abs() <= 3.0 * sigma.max(1e-3)).collect();
    }
    let limit = MAGNITUDE_OUTLIER_FLOOR.max(MAGNITUDE_OUTLIER_SIGMA * sigma);
    for (k, m, g) in usable {
        if (g - line.0 - line.1 * m).abs() > limit {
            keep[k] = false;
        }
    }
    keep
}

fn source_gate(catalogue: &GaiaCatalogue, override_source: bool, space_telescope: bool) -> Result<bool, String> {
    match &catalogue.kind {
        CatalogueKind::GaiaDr3Vizier if catalogue.quality_columns => Ok(true),
        CatalogueKind::GaiaDr3Vizier if override_source => Ok(false),
        CatalogueKind::GaiaDr3Vizier => Err("Gaia DR3 rows without the quality columns (RUWE, errors, Solved): refresh the cone or override".to_string()),
        CatalogueKind::GaiaDr3Siril if override_source && space_telescope => Err("the Siril Gaia extract (Zenodo 14692304) is allowed for amateur fields only: a space-telescope image needs Gaia DR3 via VizieR".to_string()),
        CatalogueKind::GaiaDr3Siril if override_source => Ok(false),
        CatalogueKind::GaiaDr3Siril => Err("the Siril Gaia extract (Zenodo 14692304) has no errors, RUWE or parallax: quality unknown, override to use it on an amateur field".to_string()),
        other => Err(format!("{} is not Gaia DR3", other.label())),
    }
}

fn plane_of(wcs: &WcsTransform, tangent: &Tangent, x: f64, y: f64) -> Option<[f64; 2]> {
    let sky = wcs.pixel_to_world(x, y);
    if !(sky.ra.is_finite() && sky.dec.is_finite()) {
        return None;
    }
    tangent.project(unit_vector(sky.ra, sky.dec))
}

fn exact_cards_possible(wcs: &WcsTransform, tangent: &Tangent, reference: (f64, f64), crval: (f64, f64), cd: Mat2, projection: &str) -> bool {
    if wcs.kind() != WcsKind::Header || !projection.eq_ignore_ascii_case("TAN") {
        return false;
    }
    let (ra, dec) = ra_dec_deg(tangent.p);
    let d_ra = ((ra - crval.0 + 180.0).rem_euclid(360.0) - 180.0) * dec.to_radians().cos();
    if d_ra.abs() > PIVOT_MATCH_DEG || (dec - crval.1).abs() > PIVOT_MATCH_DEG {
        return false;
    }
    let h = JACOBIAN_STEP_PX;
    let (x, y) = reference;
    let (Some(xp), Some(xm), Some(yp), Some(ym)) = (
        plane_of(wcs, tangent, x + h, y),
        plane_of(wcs, tangent, x - h, y),
        plane_of(wcs, tangent, x, y + h),
        plane_of(wcs, tangent, x, y - h),
    ) else {
        return false;
    };
    let j = [
        [(xp[0] - xm[0]) / (2.0 * h), (yp[0] - ym[0]) / (2.0 * h)],
        [(xp[1] - xm[1]) / (2.0 * h), (yp[1] - ym[1]) / (2.0 * h)],
    ];
    let cd_rad = scaled2(cd, PI / 180.0);
    frobenius(sub2(j, cd_rad)) <= EXACT_CARD_TOLERANCE * frobenius(cd_rad)
}

fn centre_shift_mas(wcs: &WcsTransform, correction: &SkyCorrection, centre: (f64, f64)) -> Option<[f64; 2]> {
    let before = wcs.pixel_to_world(centre.0, centre.1);
    let (ra, dec) = correction.apply(before.ra, before.dec)?;
    let d_ra = (ra - before.ra + 180.0).rem_euclid(360.0) - 180.0;
    Some([d_ra * before.dec.to_radians().cos() * MAS_PER_DEG, (dec - before.dec) * MAS_PER_DEG])
}

fn prepare_gaia(catalogue: &GaiaCatalogue, quality_known: bool, dt: f64, observer: Option<[f64; 3]>, options: &TieOptions, tangent0: &Tangent, cuts: &mut CutCounts) -> Vec<GaiaStar> {
    let mut stars = Vec::new();
    cuts.rows_in = catalogue.rows.len();
    for (index, row) in catalogue.rows.iter().enumerate() {
        if !(row.ra_deg.is_finite() && row.dec_deg.is_finite()) {
            cuts.invalid_rows += 1;
            continue;
        }
        if row.g.is_some_and(|g| g < BRIGHT_LIMIT_G) {
            cuts.bright += 1;
            continue;
        }
        if quality_known && !matches!(row.solved, Some(SOLVED_FIVE_PARAMETER) | Some(SOLVED_SIX_PARAMETER)) {
            cuts.two_parameter += 1;
            continue;
        }
        if quality_known && !row.ruwe.is_some_and(|r| r < RUWE_LIMIT) {
            cuts.ruwe += 1;
            continue;
        }
        if row.dup == Some(true) {
            cuts.dup += 1;
            continue;
        }
        if options.strict_sepsi && row.sepsi.is_some_and(|s| s >= SEPSI_STRICT_LIMIT) {
            cuts.sepsi += 1;
            continue;
        }
        if options.ipd_cut && row.ipd_frac_multi_peak.is_some_and(|f| f > IPD_MULTI_PEAK_LIMIT) {
            cuts.ipd += 1;
            continue;
        }
        let pm = match (row.pm_ra_masyr, row.pm_dec_masyr) {
            (Some(a), Some(b)) if a.is_finite() && b.is_finite() => [a, b],
            _ => [0.0, 0.0],
        };
        let (mut ra, mut dec) = if options.propagate_proper_motion {
            propagate_linear(row.ra_deg, row.dec_deg, pm[0], pm[1], dt)
        } else {
            (row.ra_deg, row.dec_deg)
        };
        let (typ_pos, typ_pm) = typical_gaia_sigma_mas(row.g.unwrap_or(20.0));
        let e_pos = |e: Option<f64>| e.filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(typ_pos);
        let e_pm = |e: Option<f64>| e.filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(typ_pm);
        let axis = |s0: f64, sm: f64, rho: Option<f64>| {
            let rho = rho.unwrap_or(0.0);
            s0 * s0 + 2.0 * dt * rho * s0 * sm + dt * dt * sm * sm
        };
        let sig_ra = axis(e_pos(row.e_ra_mas), e_pm(row.e_pm_ra_masyr), row.ra_pm_ra_corr);
        let sig_dec = axis(e_pos(row.e_dec_mas), e_pm(row.e_pm_dec_masyr), row.dec_pm_dec_corr);
        let mut sigma2_mas = 0.5 * (sig_ra + sig_dec) * options.gaia_inflation * options.gaia_inflation;
        let plx = row.parallax_mas.filter(|v| v.is_finite());
        match (observer, plx) {
            (Some(b), Some(plx)) => {
                let d = parallax_displacement_mas(ra, dec, plx, b);
                let cos_dec = dec.to_radians().cos();
                if cos_dec.abs() > 1e-12 {
                    ra = (ra + d[0] / MAS_PER_DEG / cos_dec).rem_euclid(360.0);
                }
                dec = (dec + d[1] / MAS_PER_DEG).clamp(-90.0, 90.0);
            }
            (None, Some(plx)) => sigma2_mas += plx * plx,
            _ => {}
        }
        let v = unit_vector(ra, dec);
        let Some(plane) = tangent0.project(v) else {
            cuts.invalid_rows += 1;
            continue;
        };
        stars.push(GaiaStar {
            index,
            v,
            plane0: [plane[0] * RAD_TO_ARCSEC, plane[1] * RAD_TO_ARCSEC],
            sigma2: sigma2_mas / (MAS_PER_ARCSEC * MAS_PER_ARCSEC),
            g: row.g,
            bp_rp: row.bp_rp,
            pm,
            pm_known: matches!((row.pm_ra_masyr, row.pm_dec_masyr), (Some(a), Some(b)) if a.is_finite() && b.is_finite()),
            parallax_known: plx.is_some(),
        });
    }
    cuts.rows_used = stars.len();
    stars
}

fn prepare_detections(input: &TieInput, options: &TieOptions, tangent0: &Tangent, pixel_scale: f64, cuts: &mut CutCounts) -> Vec<Det> {
    cuts.detections_in = input.detections.len();
    let valid: Vec<usize> = input
        .detections
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            if !(d.x.is_finite() && d.y.is_finite() && d.snr.is_finite() && d.fwhm_px.is_finite()) {
                cuts.invalid_detections += 1;
                return None;
            }
            Some(i)
        })
        .collect();
    let mut by_x = valid.clone();
    by_x.sort_by(|&a, &b| input.detections[a].x.total_cmp(&input.detections[b].x));
    let mut blended = vec![false; input.detections.len()];
    let max_fwhm = by_x.iter().map(|&i| input.detections[i].fwhm_px).fold(0.0, f64::max).min(MAX_BLEND_WINDOW_PX);
    let span = BLEND_FWHM_MULTIPLE * max_fwhm;
    for (k, &i) in by_x.iter().enumerate() {
        let a = &input.detections[i];
        for &j in &by_x[k + 1..] {
            let b = &input.detections[j];
            if b.x - a.x > span {
                break;
            }
            let limit = BLEND_FWHM_MULTIPLE * a.fwhm_px.max(b.fwhm_px);
            if (b.x - a.x).hypot(b.y - a.y) < limit {
                blended[i] = true;
                blended[j] = true;
            }
        }
    }
    let mut kept: Vec<usize> = Vec::new();
    for &i in &valid {
        let d = &input.detections[i];
        if d.saturated {
            cuts.saturated += 1;
            continue;
        }
        if blended[i] {
            cuts.blended += 1;
            continue;
        }
        kept.push(i);
    }
    kept.sort_by(|&a, &b| input.detections[b].snr.total_cmp(&input.detections[a].snr));
    if kept.len() > options.max_detections {
        cuts.beyond_brightest = kept.len() - options.max_detections;
        kept.truncate(options.max_detections);
    }
    let mut dets = Vec::with_capacity(kept.len());
    for i in kept {
        let d = &input.detections[i];
        let Some(plane) = plane_of(input.wcs, tangent0, d.x, d.y) else {
            cuts.invalid_detections += 1;
            continue;
        };
        let floor = options.sigma_floor_px.unwrap_or(if d.fwhm_px < UNDERSAMPLED_FWHM_PX || options.resampled_pixels {
            SIGMA_FLOOR_UNDERSAMPLED_PX
        } else {
            SIGMA_FLOOR_SAMPLED_PX
        });
        let sigma_px = if d.snr > 0.0 { (CENTROID_CRB_FACTOR * d.fwhm_px / d.snr).max(floor) } else { floor };
        let sigma = sigma_px * pixel_scale;
        dets.push(Det {
            x: d.x,
            y: d.y,
            plane0: [plane[0] * RAD_TO_ARCSEC, plane[1] * RAD_TO_ARCSEC],
            sigma2: sigma * sigma,
            mag_inst: d.mag_inst,
            fwhm_arcsec: d.fwhm_px * pixel_scale,
        });
    }
    cuts.detections_used = dets.len();
    dets
}

struct FieldBox {
    corners: [[f64; 2]; 4],
    orientation: f64,
    area: f64,
    perimeter: f64,
}

impl FieldBox {
    fn new(corners: [[f64; 2]; 4]) -> Self {
        let mut twice_area = 0.0;
        let mut perimeter = 0.0;
        for k in 0..4 {
            let a = corners[k];
            let b = corners[(k + 1) % 4];
            twice_area += a[0] * b[1] - b[0] * a[1];
            perimeter += norm_v(sub_v(b, a));
        }
        Self { corners, orientation: if twice_area < 0.0 { -1.0 } else { 1.0 }, area: 0.5 * twice_area.abs(), perimeter }
    }

    fn area(&self, margin: f64) -> f64 {
        self.area + self.perimeter * margin + PI * margin * margin
    }

    fn contains(&self, p: [f64; 2], margin: f64) -> bool {
        (0..4).all(|k| {
            let a = self.corners[k];
            let b = self.corners[(k + 1) % 4];
            let edge = sub_v(b, a);
            let length = norm_v(edge);
            if length <= 0.0 {
                return true;
            }
            let cross = edge[0] * (p[1] - a[1]) - edge[1] * (p[0] - a[0]);
            self.orientation * cross / length >= -margin
        })
    }

    fn half_diagonal(&self) -> f64 {
        let mut worst: f64 = 0.0;
        for i in 0..4 {
            for j in i + 1..4 {
                worst = worst.max(norm_v(sub_v(self.corners[i], self.corners[j])));
            }
        }
        0.5 * worst
    }
}

fn field_box(wcs: &WcsTransform, tangent0: &Tangent, width: usize, height: usize) -> Option<FieldBox> {
    let mut corners = [[0.0; 2]; 4];
    for (k, (x, y)) in pixel_edge_corners(width, height).into_iter().enumerate() {
        let p = plane_of(wcs, tangent0, x, y)?;
        corners[k] = [p[0] * RAD_TO_ARCSEC, p[1] * RAD_TO_ARCSEC];
    }
    Some(FieldBox::new(corners))
}

fn density_in(stars: &[GaiaStar], field: &FieldBox, margin: f64) -> f64 {
    let count = stars.iter().filter(|s| field.contains(s.plane0, margin)).count();
    count as f64 / field.area(margin).max(1e-9)
}

struct MatchGeometry {
    offset: [f64; 2],
    sigma_res: f64,
    pixel_scale: f64,
    margin: f64,
}

struct MatchOutcome {
    pairs: Vec<Pair>,
    ambiguous: usize,
    magnitude_outliers: usize,
    density: f64,
    r_chance: f64,
    r_m: f64,
}

fn run_matching(dets: &[Det], stars: &[GaiaStar], field: &FieldBox, geometry: &MatchGeometry) -> Result<MatchOutcome, String> {
    let MatchGeometry { offset, sigma_res, pixel_scale, margin } = *geometry;
    let order = eta_sorted(stars);
    let density = density_in(stars, field, margin);
    let r_chance = chance_radius_arcsec(density, DEFAULT_CHANCE_FRACTION);
    if r_chance < 2.0 * sigma_res {
        return Err(format!("too crowded: 2 % chance radius {r_chance:.2} arcsec < 2 × {sigma_res:.2} arcsec"));
    }
    let r_m = (MATCH_RADIUS_SIGMAS * sigma_res).max(pixel_scale).min(r_chance);
    let (pairs, ambiguous) = match_one_to_one(dets, stars, &order, offset, r_m);
    let keep = magnitude_consistent(&pairs, dets, stars);
    let magnitude_outliers = keep.iter().filter(|&&k| !k).count();
    let pairs: Vec<Pair> = pairs.into_iter().zip(keep).filter_map(|(p, k)| k.then_some(p)).collect();
    Ok(MatchOutcome { pairs, ambiguous, magnitude_outliers, density, r_chance, r_m })
}

#[derive(Clone, Copy)]
struct PlanePairs<'a> {
    p: &'a [[f64; 2]],
    q: &'a [[f64; 2]],
    sigma2: &'a [f64],
}

struct CvOut {
    rms_before_mas: f64,
    rms_after_mas: f64,
    median_after_mas: [f64; 2],
    median_sigma_mas: [f64; 2],
}

fn cross_validate(kind: TieModel, pairs: PlanePairs, full: &ClippedFit, rng: &mut StdRng) -> Option<CvOut> {
    let PlanePairs { p, q, sigma2 } = pairs;
    let n = p.len();
    if n < CV_FOLDS {
        return None;
    }
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.gen_range(0..=i);
        perm.swap(i, j);
    }
    let fold_of: Vec<usize> = {
        let mut f = vec![0; n];
        for (k, &i) in perm.iter().enumerate() {
            f[i] = k % CV_FOLDS;
        }
        f
    };
    let mut sw = 0.0;
    let mut before = 0.0;
    let mut after = 0.0;
    let mut rx = Vec::new();
    let mut ry = Vec::new();
    for fold in 0..CV_FOLDS {
        let train: Vec<usize> = (0..n).filter(|&i| fold_of[i] != fold).collect();
        let tp: Vec<[f64; 2]> = train.iter().map(|&i| p[i]).collect();
        let tq: Vec<[f64; 2]> = train.iter().map(|&i| q[i]).collect();
        let ts: Vec<f64> = train.iter().map(|&i| sigma2[i]).collect();
        let Some(fit) = fit_with_clipping(kind, &tp, &tq, &ts) else { continue };
        for i in (0..n).filter(|&i| fold_of[i] == fold && full.inliers[i]) {
            let w = 1.0 / (sigma2[i] + full.sigma_sys2);
            let rb = sub_v(q[i], p[i]);
            let ra = fit.model.residual(p[i], q[i]);
            sw += w;
            before += w * (rb[0] * rb[0] + rb[1] * rb[1]);
            after += w * (ra[0] * ra[0] + ra[1] * ra[1]);
            rx.push(ra[0] * MAS_PER_ARCSEC);
            ry.push(ra[1] * MAS_PER_ARCSEC);
        }
    }
    if sw <= 0.0 || sw.is_nan() || rx.is_empty() {
        return None;
    }
    let count = rx.len() as f64;
    let sigma = |v: &[f64]| (MEDIAN_TO_SIGMA * robust_sigma(v) / count.sqrt()).max(MEDIAN_SIGMA_FLOOR_MAS);
    Some(CvOut {
        rms_before_mas: (before / sw).sqrt() * MAS_PER_ARCSEC,
        rms_after_mas: (after / sw).sqrt() * MAS_PER_ARCSEC,
        median_after_mas: [median_of(&rx), median_of(&ry)],
        median_sigma_mas: [sigma(&rx), sigma(&ry)],
    })
}

struct BootstrapOut {
    sigma: Params,
    failures: usize,
    centre_mas: f64,
    corner_mas: f64,
}

struct Uncertainty<'a> {
    grid: &'a [[f64; 2]],
    centre: [f64; 2],
    resamples: usize,
}

fn bootstrap(kind: TieModel, pairs: PlanePairs, full: &ClippedFit, at: &Uncertainty, rng: &mut StdRng) -> Option<BootstrapOut> {
    let PlanePairs { p, q, sigma2 } = pairs;
    let Uncertainty { grid, centre, resamples: b } = *at;
    let n = p.len();
    if n == 0 || b == 0 {
        return None;
    }
    let c = full.model.c;
    let mut shift_x = Vec::with_capacity(b);
    let mut shift_y = Vec::with_capacity(b);
    let mut rot = Vec::with_capacity(b);
    let mut sc = Vec::with_capacity(b);
    let mut asp = Vec::with_capacity(b);
    let mut skew = Vec::with_capacity(b);
    let mut grid_x: Vec<Vec<f64>> = vec![Vec::with_capacity(b); grid.len()];
    let mut grid_y: Vec<Vec<f64>> = vec![Vec::with_capacity(b); grid.len()];
    let mut failures = 0;
    for _ in 0..b {
        let idx: Vec<usize> = (0..n).map(|_| rng.gen_range(0..n)).collect();
        let bp: Vec<[f64; 2]> = idx.iter().map(|&i| p[i]).collect();
        let bq: Vec<[f64; 2]> = idx.iter().map(|&i| q[i]).collect();
        let bs: Vec<f64> = idx.iter().map(|&i| sigma2[i]).collect();
        let Some(fit) = fit_with_clipping(kind, &bp, &bq, &bs) else {
            failures += 1;
            continue;
        };
        let da = sub2(fit.model.a, full.model.a);
        let dt = sub_v(fit.model.shift_about(c), full.model.t);
        let d_centre = add_v(apply2(da, sub_v(centre, c)), dt);
        shift_x.push(d_centre[0] * MAS_PER_ARCSEC);
        shift_y.push(d_centre[1] * MAS_PER_ARCSEC);
        rot.push(matrix_rotation_rad(fit.model.a).to_degrees());
        sc.push((matrix_scale(fit.model.a) - 1.0) * 1e6);
        asp.push(matrix_aspect_ppm(fit.model.a));
        skew.push(matrix_skew_deg(fit.model.a));
        for (k, g) in grid.iter().enumerate() {
            let d = add_v(apply2(da, sub_v(*g, c)), dt);
            grid_x[k].push(d[0] * MAS_PER_ARCSEC);
            grid_y[k].push(d[1] * MAS_PER_ARCSEC);
        }
    }
    if shift_x.len() < 2 {
        return None;
    }
    let centre_mas = half_width_68(&mut shift_x.clone()).hypot(half_width_68(&mut shift_y.clone()));
    let corner_mas = (0..grid.len())
        .map(|k| half_width_68(&mut grid_x[k]).hypot(half_width_68(&mut grid_y[k])))
        .fold(0.0, f64::max);
    Some(BootstrapOut {
        sigma: Params {
            shift_mas: [half_width_68(&mut shift_x), half_width_68(&mut shift_y)],
            rotation_deg: half_width_68(&mut rot),
            scale_ppm: half_width_68(&mut sc),
            aspect_ppm: half_width_68(&mut asp),
            skew_deg: half_width_68(&mut skew),
        },
        failures,
        centre_mas,
        corner_mas,
    })
}

fn refused(report: TieReport, reason: String) -> TieResult {
    TieResult { decision: TieDecision::Refused, reason: Some(reason), report, correction: None, header_update: None }
}

fn choose_model(options: &TieOptions, space_telescope: bool, n_matched: usize, notes: &mut Vec<String>) -> TieModel {
    let mut model = match options.model {
        ModelChoice::Fixed(m) => m,
        ModelChoice::Auto if space_telescope => TieModel::Rshift,
        ModelChoice::Auto if n_matched >= TieModel::General.min_pairs() => TieModel::General,
        ModelChoice::Auto => TieModel::Rshift,
    };
    while n_matched < model.min_pairs() {
        let Some(lower) = model.downgrade() else { break };
        notes.push(format!("{} < {} matched pairs for {}: downgraded to {}", n_matched, model.min_pairs(), model.name(), lower.name()));
        model = lower;
    }
    model
}

struct TieFrame {
    p0: [f64; 3],
    tangent0: Tangent,
    exact: bool,
    cd: Mat2,
    pixel_scale: f64,
    quality_known: bool,
    epoch_source: EpochSource,
    dt: f64,
}

fn frame_of(input: &TieInput, options: &TieOptions, report: &mut TieReport) -> Result<TieFrame, String> {
    let wcs = input.wcs;
    let (crpix1, crpix2, crval1, crval2, cd, projection) = wcs.raw_params();
    let has_sip = {
        let (a, b) = wcs.sip_forward_terms();
        a.is_some() || b.is_some()
    };
    report.wcs_kind = match wcs.kind() {
        WcsKind::Header if has_sip => format!("header {projection}-SIP"),
        WcsKind::Header => format!("header {projection}"),
        WcsKind::Gwcs => "gwcs".to_string(),
    };
    report.search_radius_arcsec = options.search_radius_arcsec;

    let reference = if wcs.kind() == WcsKind::Header { (crpix1 - 1.0, crpix2 - 1.0) } else { pixel_center(input.width, input.height) };
    let reference_sky = wcs.pixel_to_world(reference.0, reference.1);
    if !(reference_sky.ra.is_finite() && reference_sky.dec.is_finite()) {
        return Err("the WCS is not finite at its reference pixel".to_string());
    }
    let p0 = unit_vector(reference_sky.ra, reference_sky.dec);
    let tangent0 = Tangent::at(p0);
    report.pivot_deg = [reference_sky.ra, reference_sky.dec];
    let exact = exact_cards_possible(wcs, &tangent0, reference, (crval1, crval2), cd, projection);
    report.representation = if exact {
        "exact header update (CRVAL, CD and RADESYS; CRPIX and SIP unchanged)".to_string()
    } else {
        "sky correction operator (TAN-SIP refit required to write a header)".to_string()
    };

    let quality_known = source_gate(input.catalogue, options.override_source, input.space_telescope)?;
    if !quality_known {
        report.warnings.push("catalogue quality unknown: Gaia errors modelled from G (EDR3 typical uncertainties)".to_string());
    }

    let (epoch_year, epoch_source) = match (input.epoch.year, input.epoch.source) {
        (Some(y), EpochSource::Typed) if y.is_finite() => (y, EpochSource::Typed),
        (Some(y), _) if y.is_finite() => (y, EpochSource::Header),
        _ => (GAIA_REFERENCE_EPOCH_YEAR, EpochSource::None),
    };
    let dt = epoch_year - GAIA_REFERENCE_EPOCH_YEAR;
    report.epoch_year = epoch_year;
    report.epoch_source = epoch_source;
    report.delta_t_yr = dt;
    report.proper_motion = if options.propagate_proper_motion { "linear from J2016.0".to_string() } else { "not propagated".to_string() };
    report.parallax = if input.observer_barycentric_au.is_some() {
        "applied from the observer's barycentric position".to_string()
    } else {
        "unknown observer: parallax added to the Gaia error".to_string()
    };
    if epoch_source == EpochSource::None {
        report.warnings.push(format!(
            "no observation date: Gaia positions kept at J{GAIA_REFERENCE_EPOCH_YEAR:.1}; the epoch term below assumes ±{:.0} yr",
            input.epoch.assumed_uncertainty_yr
        ));
    }

    let pixel_scale = wcs.pixel_scale_arcsec();
    if !(pixel_scale.is_finite() && pixel_scale > 0.0) {
        return Err("the WCS pixel scale is not finite".to_string());
    }
    Ok(TieFrame { p0, tangent0, exact, cd, pixel_scale, quality_known, epoch_source, dt })
}

struct Matched {
    stars: Vec<GaiaStar>,
    dets: Vec<Det>,
    field: FieldBox,
    pairs: Vec<Pair>,
}

fn match_field(input: &TieInput, options: &TieOptions, frame: &TieFrame, report: &mut TieReport) -> Result<Matched, String> {
    let mut cuts = CutCounts::default();
    let stars = prepare_gaia(input.catalogue, frame.quality_known, frame.dt, input.observer_barycentric_au, options, &frame.tangent0, &mut cuts);
    let dets = prepare_detections(input, options, &frame.tangent0, frame.pixel_scale, &mut cuts);
    report.cuts = cuts;
    let field = field_box(input.wcs, &frame.tangent0, input.width, input.height).ok_or("the image corners do not project through the WCS")?;
    let r_s = options.search_radius_arcsec;
    if stars.len() < TieModel::Shift.min_pairs() || dets.len() < TieModel::Shift.min_pairs() {
        let n = stars.len().min(dets.len());
        return Err(format!("{n} < {} used pairs for shift", TieModel::Shift.min_pairs()));
    }

    let order = eta_sorted(&stars);
    let density = density_in(&stars, &field, r_s);
    let bin = (HISTOGRAM_BIN_MIN_PX * frame.pixel_scale).max(HISTOGRAM_BIN_MIN_ARCSEC);
    let peak = find_offset_peak(&dets, &stars, &order, r_s, bin, density);
    report.histogram_bin_arcsec = peak.bin;
    report.histogram_peak = peak.peak;
    report.histogram_random_per_bin = peak.random_per_bin;
    report.histogram_threshold = peak.threshold;
    report.histogram_expected_false_peaks = Some(peak.expected_false_peaks);
    report.peak_offset_arcsec = peak.offset;
    report.sigma_res_arcsec = peak.sigma_res;
    report.density_per_arcsec2 = density;
    if (peak.peak as f64) < peak.threshold {
        return Err(format!(
            "no significant offset peak within {r_s:.1} arcsec (peak {}, random {:.1} per bin, {} needed to keep the chance of a false peak below {} % over {} bins)",
            peak.peak, peak.random_per_bin, peak.threshold.ceil(), PEAK_FALSE_ALARM * 100.0, peak.bins
        ));
    }

    let geometry = MatchGeometry { offset: peak.offset, sigma_res: peak.sigma_res, pixel_scale: frame.pixel_scale, margin: r_s };
    let mut outcome = run_matching(&dets, &stars, &field, &geometry)?;
    if outcome.pairs.len() >= MIN_WINDOW_PAIRS {
        let mut gs: Vec<f64> = outcome.pairs.iter().filter_map(|p| stars[p.gaia].g).collect();
        if gs.len() >= MIN_WINDOW_PAIRS {
            gs.sort_by(f64::total_cmp);
            let lo = percentile(&gs, 0.05) - MAGNITUDE_WINDOW_MARGIN;
            let hi = percentile(&gs, 0.95) + MAGNITUDE_WINDOW_MARGIN;
            let subset: Vec<usize> = (0..stars.len()).filter(|&i| stars[i].g.is_some_and(|g| g >= lo && g <= hi)).collect();
            if subset.len() >= MIN_WINDOW_PAIRS {
                let windowed: Vec<GaiaStar> = subset.iter().map(|&i| stars[i].clone()).collect();
                if let Ok(second) = run_matching(&dets, &windowed, &field, &geometry) {
                    if second.pairs.len() >= outcome.pairs.len() / 2 {
                        outcome = MatchOutcome {
                            pairs: second.pairs.into_iter().map(|p| Pair { det: p.det, gaia: subset[p.gaia] }).collect(),
                            ..second
                        };
                    }
                }
            }
        }
    }
    report.cuts.ambiguous = outcome.ambiguous;
    report.cuts.magnitude_outliers = outcome.magnitude_outliers;
    report.density_per_arcsec2 = outcome.density;
    report.chance_radius_arcsec = outcome.r_chance;
    report.match_radius_arcsec = outcome.r_m;
    report.chance_fraction = chance_fraction(outcome.r_m, outcome.density);
    report.n_matched = outcome.pairs.len();
    if report.chance_fraction > DEFAULT_CHANCE_FRACTION * 0.999 {
        report.warnings.push(format!("crowded field: match radius capped at the {:.0} % chance-match radius {:.2} arcsec", DEFAULT_CHANCE_FRACTION * 100.0, outcome.r_m));
    }
    Ok(Matched { stars, dets, field, pairs: outcome.pairs })
}

struct PassData<'a> {
    l: &'a [[f64; 2]],
    gaia_v: &'a [[f64; 3]],
    sigma2: &'a [f64],
    p0: [f64; 3],
    half_diagonal: f64,
}

struct PassOutcome {
    model: TieModel,
    correction: SkyCorrection,
    passes: usize,
    last_update_mas: f64,
    fit: ClippedFit,
    p_final: Vec<[f64; 2]>,
    q_final: Vec<[f64; 2]>,
}

type Fitter<'f> = &'f mut dyn FnMut(TieModel, &[[f64; 2]], &[[f64; 2]], &[f64]) -> Option<ClippedFit>;

fn run_passes(mut model: TieModel, data: &PassData, notes: &mut Vec<String>, fitter: Fitter) -> Result<PassOutcome, String> {
    let mut correction = SkyCorrection::identity(data.p0);
    let mut passes = 0;
    let mut last_update_mas = f64::INFINITY;
    let mut last: Option<(ClippedFit, Vec<[f64; 2]>, Vec<[f64; 2]>)> = None;
    while passes < MAX_PASSES {
        passes += 1;
        let tangent = Tangent::at(correction.pivot_out);
        let p: Vec<[f64; 2]> = data.l.iter().map(|v| apply2(correction.matrix, *v)).collect();
        let q: Vec<[f64; 2]> = data
            .gaia_v
            .iter()
            .map(|v| {
                let pr = tangent.project(*v).unwrap_or([f64::NAN; 2]);
                [pr[0] * RAD_TO_ARCSEC, pr[1] * RAD_TO_ARCSEC]
            })
            .collect();
        if q.iter().flatten().any(|v| !v.is_finite()) {
            return Err("a Gaia star does not project onto the tangent plane".to_string());
        }
        let Some(fit) = fitter(model, &p, &q, data.sigma2) else {
            return Err(format!("the {} fit is singular", model.name()));
        };
        if fit.n_used < model.min_pairs() {
            let Some(lower) = model.downgrade() else {
                return Err(format!("{} < {} used pairs for shift", fit.n_used, TieModel::Shift.min_pairs()));
            };
            notes.push(format!("{} < {} used pairs for {}: downgraded to {}", fit.n_used, model.min_pairs(), model.name(), lower.name()));
            model = lower;
            correction = SkyCorrection::identity(data.p0);
            passes = 0;
            last_update_mas = f64::INFINITY;
            last = None;
            continue;
        }
        let t_p = fit.model.shift_at_origin();
        last_update_mas = (norm_v(t_p) + frobenius(sub2(fit.model.a, IDENTITY2)) * data.half_diagonal) * MAS_PER_ARCSEC;
        correction = correction.composed_with([t_p[0] / RAD_TO_ARCSEC, t_p[1] / RAD_TO_ARCSEC], fit.model.a);
        last = Some((fit, p, q));
        if last_update_mas < CONVERGENCE_MAS {
            break;
        }
    }
    let Some((fit, p_final, q_final)) = last else {
        return Err("no fit pass completed".to_string());
    };
    Ok(PassOutcome { model, correction, passes, last_update_mas, fit, p_final, q_final })
}

fn range_check(report: &TieReport, model: TieModel, a_tr: Mat2, centre_shift: [f64; 2], r_s: f64) -> Result<(), String> {
    let shift_arcsec = norm_v(centre_shift) / MAS_PER_ARCSEC;
    if shift_arcsec > r_s {
        return Err(format!("re-solve the image: shift {shift_arcsec:.2} arcsec exceeds the search radius {r_s:.1} arcsec"));
    }
    if report.params.rotation_deg.abs() > MAX_ROTATION_DEG {
        return Err(format!("re-solve the image: rotation {:.3} deg exceeds {MAX_ROTATION_DEG} deg", report.params.rotation_deg));
    }
    if (matrix_scale(a_tr) - 1.0).abs() > MAX_SCALE_DEVIATION {
        return Err(format!("re-solve the image: scale change {:.1} ppm exceeds {:.0} ppm", report.params.scale_ppm, MAX_SCALE_DEVIATION * 1e6));
    }
    if model == TieModel::General && general_deviation(a_tr) > MAX_GENERAL_DEVIATION {
        return Err(format!("re-solve the image: matrix deviation {:.4} beyond the rotation exceeds {MAX_GENERAL_DEVIATION}", general_deviation(a_tr)));
    }
    Ok(())
}

fn star_flags(star: &GaiaStar, used: bool, observer_known: bool) -> Vec<&'static str> {
    let mut flags = Vec::new();
    if !used {
        flags.push(FLAG_CLIPPED);
    }
    if !star.pm_known {
        flags.push(FLAG_NO_PROPER_MOTION);
    }
    if star.parallax_known {
        flags.push(if observer_known { FLAG_PARALLAX_APPLIED } else { FLAG_PARALLAX_INFLATED });
    }
    flags
}

fn residual_slopes(dets: &[Det], stars: &[GaiaStar], pairs: &[Pair], used_idx: &[usize], used_w: &[f64], after_res: &[[f64; 2]]) -> Vec<Slope> {
    let mut slopes = Vec::new();
    let rx: Vec<f64> = used_idx.iter().map(|&k| after_res[k][0] * MAS_PER_ARCSEC).collect();
    let ry: Vec<f64> = used_idx.iter().map(|&k| after_res[k][1] * MAS_PER_ARCSEC).collect();
    let predictors: [(&'static str, Vec<Option<f64>>); 4] = [
        ("x", used_idx.iter().map(|&k| Some(dets[pairs[k].det].x)).collect()),
        ("y", used_idx.iter().map(|&k| Some(dets[pairs[k].det].y)).collect()),
        ("G", used_idx.iter().map(|&k| stars[pairs[k].gaia].g).collect()),
        ("BP-RP", used_idx.iter().map(|&k| stars[pairs[k].gaia].bp_rp).collect()),
    ];
    for (name, values) in predictors {
        let sel: Vec<usize> = (0..values.len()).filter(|&i| values[i].is_some_and(|v| v.is_finite())).collect();
        if sel.len() < 3 {
            continue;
        }
        let xs: Vec<f64> = sel.iter().map(|&i| values[i].unwrap()).collect();
        let w: Vec<f64> = sel.iter().map(|&i| used_w[i]).collect();
        for (axis, res) in [("E", &rx), ("N", &ry)] {
            let ys: Vec<f64> = sel.iter().map(|&i| res[i]).collect();
            if let Some((value, sigma)) = weighted_slope(&xs, &ys, &w) {
                slopes.push(Slope { against: name, axis, value, sigma });
            }
        }
    }
    slopes
}

fn correction_grid(wcs: &WcsTransform, tangent0: &Tangent, correction: &SkyCorrection, width: usize, height: usize) -> Vec<[f64; 2]> {
    let mut g = Vec::with_capacity(GRID_SIZE * GRID_SIZE);
    for j in 0..GRID_SIZE {
        for i in 0..GRID_SIZE {
            let x = -0.5 + (width as f64) * (i as f64 + 0.5) / GRID_SIZE as f64;
            let y = -0.5 + (height as f64) * (j as f64 + 0.5) / GRID_SIZE as f64;
            if let Some(pl) = plane_of(wcs, tangent0, x, y) {
                g.push(apply2(correction.matrix, [pl[0] * RAD_TO_ARCSEC, pl[1] * RAD_TO_ARCSEC]));
            }
        }
    }
    g
}

fn no_significant_correction(input: &TieInput, report: TieReport, centre_shift: [f64; 2], sigma_shift: f64) -> TieResult {
    let bound = norm_v(centre_shift) + CRITICAL_95 * sigma_shift;
    let mut reason = format!("no significant correction: the WCS already agrees with Gaia DR3 within {bound:.1} mas (95 %)");
    if let (EpochSource::None, Some(term)) = (report.epoch_source, report.epoch_systematic_mas) {
        reason.push_str(&format!(", plus an epoch term of {term:.1} mas for the unknown observation date (±{:.0} yr)", input.epoch.assumed_uncertainty_yr));
    }
    TieResult { decision: TieDecision::NoSignificantCorrection, reason: Some(reason), report, correction: None, header_update: None }
}

pub fn tie_to_gaia(input: &TieInput, options: &TieOptions) -> TieResult {
    let mut report = TieReport::new(input.catalogue);
    let wcs = input.wcs;
    let frame = match frame_of(input, options, &mut report) {
        Ok(frame) => frame,
        Err(reason) => return refused(report, reason),
    };
    let Matched { stars, dets, field, pairs } = match match_field(input, options, &frame, &mut report) {
        Ok(matched) => matched,
        Err(reason) => return refused(report, reason),
    };
    let r_s = options.search_radius_arcsec;

    let mut notes = Vec::new();
    let mut model = choose_model(options, input.space_telescope, pairs.len(), &mut notes);
    if pairs.len() < TieModel::Shift.min_pairs() {
        report.model_notes = notes;
        return refused(report, format!("{} < {} used pairs for shift", pairs.len(), TieModel::Shift.min_pairs()));
    }
    let l: Vec<[f64; 2]> = pairs.iter().map(|p| dets[p.det].plane0).collect();
    let q0: Vec<[f64; 2]> = pairs.iter().map(|p| stars[p.gaia].plane0).collect();
    let gaia_v: Vec<[f64; 3]> = pairs.iter().map(|p| stars[p.gaia].v).collect();
    let sigma2: Vec<f64> = pairs.iter().map(|p| dets[p.det].sigma2 + stars[p.gaia].sigma2).collect();
    {
        let n = l.len() as f64;
        let cx = l.iter().map(|v| v[0]).sum::<f64>() / n;
        let cy = l.iter().map(|v| v[1]).sum::<f64>() / n;
        let r_rms = (l.iter().map(|v| (v[0] - cx).powi(2) + (v[1] - cy).powi(2)).sum::<f64>() / n).sqrt();
        if r_rms < LEVER_ARM_FRACTION * field.half_diagonal() && model != TieModel::Shift {
            notes.push(format!("short lever arm: matched r_rms {:.1} arcsec < {:.0} % of the half-diagonal {:.1} arcsec: shift only", r_rms, LEVER_ARM_FRACTION * 100.0, field.half_diagonal()));
            model = TieModel::Shift;
        }
    }

    let data = PassData { l: &l, gaia_v: &gaia_v, sigma2: &sigma2, p0: frame.p0, half_diagonal: field.half_diagonal() };
    let outcome = run_passes(model, &data, &mut notes, &mut |kind, p, q, s| fit_with_clipping(kind, p, q, s));
    report.model_notes = notes;
    let PassOutcome { model, correction, passes, last_update_mas, fit, p_final, q_final } = match outcome {
        Ok(outcome) => outcome,
        Err(reason) => return refused(report, reason),
    };
    report.model = Some(model.name().to_string());
    report.passes = passes;
    report.last_update_mas = last_update_mas;
    report.n_used = fit.n_used;
    report.cuts.clipped = pairs.len() - fit.n_used;
    report.sigma_sys_mas = fit.sigma_sys2.sqrt() * MAS_PER_ARCSEC;
    report.chi2_nu = fit.chi2_nu;

    let inlier_radius = (0..p_final.len())
        .filter(|&k| fit.inliers[k])
        .map(|k| norm_v(fit.model.residual(p_final[k], q_final[k])))
        .fold(0.0, f64::max)
        .min(report.match_radius_arcsec);
    let (expected_chance, needed) = chance_excess(report.cuts.detections_used, inlier_radius, report.density_per_arcsec2);
    report.expected_chance_pairs = Some(expected_chance);
    if (fit.n_used as f64) < needed {
        let reason = format!(
            "chance matches: {} of {} matched pairs used within {inlier_radius:.2} arcsec, {expected_chance:.1} expected by chance from {} detections at {:.3} Gaia stars per square arcsec; at least {} needed",
            fit.n_used, pairs.len(), report.cuts.detections_used, report.density_per_arcsec2, needed.ceil()
        );
        return refused(report, reason);
    }

    let centre_pixel = pixel_center(input.width, input.height);
    let Some(centre_shift) = centre_shift_mas(wcs, &correction, centre_pixel) else {
        return refused(report, "the image centre does not project through the correction".to_string());
    };
    let a_tr = correction.transported_matrix();
    report.params = params_from_matrix(a_tr, centre_shift);
    report.rotation_arcsec = report.params.rotation_deg * 3600.0;
    if let Err(reason) = range_check(&report, model, a_tr, centre_shift, r_s) {
        return refused(report, reason);
    }

    let tangent_final = Tangent::at(correction.pivot_out);
    let after_res: Vec<[f64; 2]> = pairs
        .iter()
        .enumerate()
        .map(|(k, pair)| {
            let pr = tangent_final.project(stars[pair.gaia].v).unwrap_or([f64::NAN; 2]);
            sub_v([pr[0] * RAD_TO_ARCSEC, pr[1] * RAD_TO_ARCSEC], apply2(correction.matrix, l[k]))
        })
        .collect();
    let before_res: Vec<[f64; 2]> = (0..pairs.len()).map(|k| sub_v(q0[k], l[k])).collect();
    let used_idx: Vec<usize> = (0..pairs.len()).filter(|&k| fit.inliers[k]).collect();
    let used_w: Vec<f64> = used_idx.iter().map(|&k| fit.weights[k]).collect();
    report.before = summarize(&used_idx.iter().map(|&k| before_res[k]).collect::<Vec<_>>(), &used_w);
    report.after = summarize(&used_idx.iter().map(|&k| after_res[k]).collect::<Vec<_>>(), &used_w);
    report.g_range = {
        let gs: Vec<f64> = used_idx.iter().filter_map(|&k| stars[pairs[k].gaia].g).filter(|g| g.is_finite()).collect();
        (!gs.is_empty()).then(|| [gs.iter().cloned().fold(f64::INFINITY, f64::min), gs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)])
    };

    let centre_plane = plane_of(wcs, &frame.tangent0, centre_pixel.0, centre_pixel.1)
        .map(|pl| apply2(correction.matrix, [pl[0] * RAD_TO_ARCSEC, pl[1] * RAD_TO_ARCSEC]))
        .unwrap_or([0.0; 2]);
    if let Some(cov) = formal_covariance(model, &fit, &p_final) {
        report.sigma_formal = formal_params(model, &cov, fit.model.a, sub_v(centre_plane, fit.model.c));
    }

    let mut rng = StdRng::seed_from_u64(options.seed);
    let cv = cross_validate(model, PlanePairs { p: &l, q: &q0, sigma2: &sigma2 }, &fit, &mut rng);
    if let Some(cv) = &cv {
        report.cv_rms_before_mas = cv.rms_before_mas;
        report.cv_rms_after_mas = cv.rms_after_mas;
        report.cv_median_after_mas = cv.median_after_mas;
        report.cv_median_sigma_mas = cv.median_sigma_mas;
    }

    let grid = correction_grid(wcs, &frame.tangent0, &correction, input.width, input.height);
    let at = Uncertainty { grid: &grid, centre: centre_plane, resamples: options.bootstrap_resamples };
    let boot = bootstrap(model, PlanePairs { p: &p_final, q: &q_final, sigma2: &sigma2 }, &fit, &at, &mut rng);
    let mut bootstrap_unstable = false;
    if let Some(b) = &boot {
        report.sigma_bootstrap = Some(b.sigma);
        report.bootstrap_failures = b.failures;
        report.uncertainty_centre_mas = b.centre_mas;
        report.uncertainty_corner_mas = b.corner_mas;
        let formal = norm_v(report.sigma_formal.shift_mas);
        let bootstrapped = norm_v(b.sigma.shift_mas);
        if formal > 0.0 && bootstrapped > 0.0 {
            let ratio = (formal / bootstrapped).max(bootstrapped / formal);
            report.errors_disagree = ratio > BOOTSTRAP_DISAGREEMENT;
            if ratio > BOOTSTRAP_REFUSAL_RATIO {
                bootstrap_unstable = true;
                report.warnings.push(format!("bootstrap and formal shift errors differ by a factor {ratio:.1}"));
            }
        }
        if (b.failures as f64) >= BOOTSTRAP_FAILURE_FRACTION * options.bootstrap_resamples as f64 && b.failures > 0 {
            bootstrap_unstable = true;
            report.warnings.push(format!("{} of {} bootstrap resamples failed", b.failures, options.bootstrap_resamples));
        }
    }

    let observer_known = input.observer_barycentric_au.is_some();
    report.stars = pairs
        .iter()
        .enumerate()
        .map(|(k, pair)| {
            let star = &stars[pair.gaia];
            let row = &input.catalogue.rows[star.index];
            StarResidual {
                id: row.id.clone(),
                g: star.g,
                bp_rp: star.bp_rp,
                x: dets[pair.det].x,
                y: dets[pair.det].y,
                before_mas: [before_res[k][0] * MAS_PER_ARCSEC, before_res[k][1] * MAS_PER_ARCSEC],
                after_mas: [after_res[k][0] * MAS_PER_ARCSEC, after_res[k][1] * MAS_PER_ARCSEC],
                weight: fit.weights[k],
                used: fit.inliers[k],
                flags: star_flags(star, fit.inliers[k], observer_known),
            }
        })
        .collect();
    report.slopes = residual_slopes(&dets, &stars, &pairs, &used_idx, &used_w, &after_res);
    if frame.epoch_source == EpochSource::None {
        let sw: f64 = used_w.iter().sum();
        let mean_pm = used_idx.iter().zip(&used_w).fold([0.0; 2], |acc, (&k, &w)| {
            let pm = stars[pairs[k].gaia].pm;
            [acc[0] + w * pm[0], acc[1] + w * pm[1]]
        });
        if sw > 0.0 {
            report.epoch_systematic_mas = Some(norm_v([mean_pm[0] / sw, mean_pm[1] / sw]) * input.epoch.assumed_uncertainty_yr);
        }
    }

    let sigma_shift = norm_v(report.sigma_formal.shift_mas).max(report.sigma_bootstrap.map_or(0.0, |b| norm_v(b.shift_mas)));
    let Some(cv) = cv else {
        return refused(report, "the hold-out check could not run".to_string());
    };
    if cv.rms_after_mas >= cv.rms_before_mas || cv.rms_after_mas.is_nan() {
        return no_significant_correction(input, report, centre_shift, sigma_shift);
    }
    for axis in 0..2 {
        if cv.median_after_mas[axis].abs() > 3.0 * cv.median_sigma_mas[axis] {
            return refused(report, format!(
                "model inadequate or mismatches: hold-out median ({:.2}, {:.2}) mas exceeds 3σ ({:.2}, {:.2}) mas",
                cv.median_after_mas[0], cv.median_after_mas[1], 3.0 * cv.median_sigma_mas[0], 3.0 * cv.median_sigma_mas[1]
            ));
        }
    }
    if bootstrap_unstable && options.strict_mode {
        return refused(report, "unstable bootstrap in strict mode".to_string());
    }

    let header = frame.exact.then(|| header_update(&correction, frame.cd));
    TieResult { decision: TieDecision::Applied, reason: None, report, correction: Some(correction), header_update: header }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::header::HduHeader;

    const ARCSEC_PER_DEG: f64 = 3600.0;
    const FGS_CENTRE: (f64, f64) = (80.657290449338, -69.517748242087);
    const FGS_CD: Mat2 = [[-1.5676882534255E-05, -1.0488772731138E-05], [-1.0475275501278E-05, 1.6294960618825E-05]];
    const FGS_SIZE: (usize, usize) = (2048, 2048);
    const FGS_EPOCH: f64 = 2022.8548;
    const AU_KM: f64 = 149_597_870.7;
    const JWST_BARYCENTRIC_KM: [f64; 3] = [101_064_627.921_434_92, 99_925_594.413_072_47, 43_442_128.872_953_08];
    const INJECTED_SHIFT_ARCSEC: [f64; 2] = [1.7, -0.9];
    const INJECTED_ROTATION_DEG: f64 = 0.02;
    const INJECTED_SCALE: f64 = 1.0 + 3e-5;
    const FWHM_PX: f64 = 2.5;
    const SIP_CARDS: &[(&str, &str)] = &[
        ("A_ORDER", "3"),
        ("B_ORDER", "3"),
        ("A_2_0", "1.0E-7"),
        ("A_1_1", "-2.0E-7"),
        ("A_0_2", "1.5E-7"),
        ("A_3_0", "1.0E-11"),
        ("A_2_1", "-1.0E-11"),
        ("A_1_2", "2.0E-11"),
        ("A_0_3", "-1.5E-11"),
        ("B_2_0", "-1.2E-7"),
        ("B_1_1", "1.0E-7"),
        ("B_0_2", "2.0E-7"),
        ("B_3_0", "-1.0E-11"),
        ("B_2_1", "1.5E-11"),
        ("B_1_2", "-2.0E-11"),
        ("B_0_3", "1.0E-11"),
    ];

    fn jwst_barycentric_au() -> [f64; 3] {
        [JWST_BARYCENTRIC_KM[0] / AU_KM, JWST_BARYCENTRIC_KM[1] / AU_KM, JWST_BARYCENTRIC_KM[2] / AU_KM]
    }

    fn tan_header(centre: (f64, f64), cd: Mat2, size: (usize, usize), sip: bool) -> HduHeader {
        let mut h = HduHeader::empty();
        h.set("NAXIS", "2".into());
        h.set("NAXIS1", size.0.to_string());
        h.set("NAXIS2", size.1.to_string());
        h.set("CTYPE1", if sip { "RA---TAN-SIP".into() } else { "RA---TAN".into() });
        h.set("CTYPE2", if sip { "DEC--TAN-SIP".into() } else { "DEC--TAN".into() });
        h.set("CUNIT1", "deg".into());
        h.set("CUNIT2", "deg".into());
        h.set("CRPIX1", format!("{:.1}", (size.0 as f64 + 1.0) / 2.0));
        h.set("CRPIX2", format!("{:.1}", (size.1 as f64 + 1.0) / 2.0));
        h.set("CRVAL1", format!("{:.16E}", centre.0));
        h.set("CRVAL2", format!("{:.16E}", centre.1));
        h.set("CD1_1", format!("{:.16E}", cd[0][0]));
        h.set("CD1_2", format!("{:.16E}", cd[0][1]));
        h.set("CD2_1", format!("{:.16E}", cd[1][0]));
        h.set("CD2_2", format!("{:.16E}", cd[1][1]));
        h.set("LONPOLE", "180".into());
        h.set("RADESYS", "ICRS".into());
        if sip {
            for (k, v) in SIP_CARDS {
                h.set(k, (*v).into());
            }
        }
        h
    }

    fn fgs_header(sip: bool) -> HduHeader {
        tan_header(FGS_CENTRE, FGS_CD, FGS_SIZE, sip)
    }

    fn polar_header() -> HduHeader {
        let s = 0.6 / ARCSEC_PER_DEG;
        tan_header((120.0, 86.5), [[-s, 0.0], [0.0, s]], (1500, 1500), false)
    }

    fn corner_crpix_header() -> HduHeader {
        let mut h = fgs_header(false);
        h.set("CRPIX1", "1.0".into());
        h.set("CRPIX2", "1.0".into());
        h
    }

    fn fk5_form(header: &HduHeader) -> HduHeader {
        let mut h = header.clone();
        h.remove("RADESYS");
        h.set("EQUINOX", "2000.0".into());
        h
    }

    fn sin_header() -> HduHeader {
        let mut h = fgs_header(false);
        h.set("CTYPE1", "RA---SIN".into());
        h.set("CTYPE2", "DEC--SIN".into());
        h
    }

    fn pc_form(header: &HduHeader) -> HduHeader {
        let mut h = header.clone();
        let cd = [
            [h.get_f64("CD1_1").unwrap(), h.get_f64("CD1_2").unwrap()],
            [h.get_f64("CD2_1").unwrap(), h.get_f64("CD2_2").unwrap()],
        ];
        for key in ["CD1_1", "CD1_2", "CD2_1", "CD2_2"] {
            h.remove(key);
        }
        let cdelt = [-cd[0][0].hypot(cd[0][1]), cd[1][0].hypot(cd[1][1])];
        h.set("CDELT1", format!("{:.16E}", cdelt[0]));
        h.set("CDELT2", format!("{:.16E}", cdelt[1]));
        h.set("PC1_1", format!("{:.16E}", cd[0][0] / cdelt[0]));
        h.set("PC1_2", format!("{:.16E}", cd[0][1] / cdelt[0]));
        h.set("PC2_1", format!("{:.16E}", cd[1][0] / cdelt[1]));
        h.set("PC2_2", format!("{:.16E}", cd[1][1] / cdelt[1]));
        h
    }

    fn pc_first_matrix(header: &HduHeader) -> Mat2 {
        if let Some(pc11) = header.get_f64("PC1_1") {
            let cdelt = [header.get_f64("CDELT1").unwrap_or(1.0), header.get_f64("CDELT2").unwrap_or(1.0)];
            return [
                [cdelt[0] * pc11, cdelt[0] * header.get_f64("PC1_2").unwrap_or(0.0)],
                [cdelt[1] * header.get_f64("PC2_1").unwrap_or(0.0), cdelt[1] * header.get_f64("PC2_2").unwrap_or(1.0)],
            ];
        }
        [
            [header.get_f64("CD1_1").unwrap(), header.get_f64("CD1_2").unwrap_or(0.0)],
            [header.get_f64("CD2_1").unwrap_or(0.0), header.get_f64("CD2_2").unwrap()],
        ]
    }

    fn wcs_of(header: &HduHeader) -> WcsTransform {
        WcsTransform::from_header(header).expect("header forms a WCS")
    }

    fn with_cards(header: &HduHeader, cards: &[(String, String)]) -> HduHeader {
        let mut h = header.clone();
        for (k, v) in cards {
            h.set(k, v.clone());
        }
        h
    }

    fn apply_update(header: &HduHeader, update: &HeaderUpdate) -> HduHeader {
        let mut h = header.clone();
        for key in &update.remove {
            h.remove(key);
        }
        with_cards(&h, &update.cards)
    }

    fn size_of(header: &HduHeader) -> (usize, usize) {
        (header.get_i64("NAXIS1").unwrap() as usize, header.get_i64("NAXIS2").unwrap() as usize)
    }

    fn injected_input(truth: &HduHeader, shift: [f64; 2], rotation_deg: f64, scale: f64) -> HduHeader {
        let (_, _, crval1, crval2, cd, _) = wcs_of(truth).raw_params();
        let inverse = SkyCorrection::from_parameters(unit_vector(crval1, crval2), [-shift[0], -shift[1]], -rotation_deg, 1.0 / scale);
        with_cards(truth, &header_update(&inverse, cd).cards)
    }

    fn gaussian(rng: &mut StdRng) -> f64 {
        let u1 = 1.0 - rng.gen::<f64>();
        let u2 = rng.gen::<f64>();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }

    #[derive(Clone)]
    struct SynthOptions {
        n: usize,
        noise_mas: f64,
        sigma_c_mas: f64,
        seed: u64,
        pm_sigma_masyr: f64,
        pm_mean_masyr: [f64; 2],
        epoch_year: f64,
        two_parameter_fraction: f64,
        outlier_fraction: f64,
        outlier_range_mas: (f64, f64),
        parallax_star: Option<(f64, [f64; 3])>,
        region: Option<[f64; 4]>,
    }

    impl SynthOptions {
        fn noise_free(n: usize) -> Self {
            Self {
                n,
                noise_mas: 0.0,
                sigma_c_mas: 0.0,
                seed: 1,
                pm_sigma_masyr: 0.0,
                pm_mean_masyr: [0.0; 2],
                epoch_year: FGS_EPOCH,
                two_parameter_fraction: 0.0,
                outlier_fraction: 0.0,
                outlier_range_mas: (0.0, 0.0),
                parallax_star: None,
                region: None,
            }
        }

        fn noisy(n: usize, noise_mas: f64, seed: u64) -> Self {
            Self { noise_mas, sigma_c_mas: noise_mas, seed, ..Self::noise_free(n) }
        }
    }

    struct Synthetic {
        detections: Vec<TieDetection>,
        catalogue: GaiaCatalogue,
        pm: Vec<[f64; 2]>,
    }

    fn synthesize(truth: &WcsTransform, size: (usize, usize), opts: &SynthOptions) -> Synthetic {
        let mut rng = StdRng::seed_from_u64(opts.seed);
        let pixel_scale = truth.pixel_scale_arcsec();
        let dt = opts.epoch_year - GAIA_REFERENCE_EPOCH_YEAR;
        let mut detections = Vec::with_capacity(opts.n);
        let mut rows = Vec::with_capacity(opts.n);
        let mut pm_list = Vec::with_capacity(opts.n);
        let margin = 40.0;
        let [x0, x1, y0, y1] = opts.region.unwrap_or([margin, size.0 as f64 - margin, margin, size.1 as f64 - margin]);
        let n_outliers = (opts.outlier_fraction * opts.n as f64).round() as usize;
        let n_two = (opts.two_parameter_fraction * opts.n as f64).round() as usize;
        for i in 0..opts.n {
            let x = x0 + (x1 - x0) * rng.gen::<f64>();
            let y = y0 + (y1 - y0) * rng.gen::<f64>();
            let sky = truth.pixel_to_world(x, y);
            let g = 12.0 + 7.0 * rng.gen::<f64>();
            let pm = if opts.pm_sigma_masyr > 0.0 {
                [opts.pm_mean_masyr[0] + opts.pm_sigma_masyr * gaussian(&mut rng), opts.pm_mean_masyr[1] + opts.pm_sigma_masyr * gaussian(&mut rng)]
            } else {
                [0.0, 0.0]
            };
            let two_parameter = i < n_two;
            let dec_2016 = sky.dec - pm[1] * dt / MAS_PER_DEG;
            let ra_2016 = sky.ra - pm[0] * dt / MAS_PER_DEG / dec_2016.to_radians().cos();
            let (mut ra_row, mut dec_row) = if two_parameter { (sky.ra, sky.dec) } else { (ra_2016, dec_2016) };
            let mut parallax = None;
            if let (0, Some((plx, observer))) = (i, opts.parallax_star) {
                for _ in 0..6 {
                    let d = parallax_displacement_mas(ra_row, dec_row, plx, observer);
                    ra_row = sky.ra - d[0] / MAS_PER_DEG / dec_row.to_radians().cos();
                    dec_row = sky.dec - d[1] / MAS_PER_DEG;
                }
                parallax = Some(plx);
            }
            rows.push(GaiaRow {
                id: format!("G{i}"),
                ra_deg: ra_row,
                dec_deg: dec_row,
                pm_ra_masyr: if two_parameter { None } else { Some(pm[0]) },
                pm_dec_masyr: if two_parameter { None } else { Some(pm[1]) },
                parallax_mas: parallax,
                e_ra_mas: Some(0.02),
                e_dec_mas: Some(0.02),
                e_pm_ra_masyr: Some(0.02),
                e_pm_dec_masyr: Some(0.02),
                e_parallax_mas: Some(0.02),
                ra_pm_ra_corr: Some(0.0),
                dec_pm_dec_corr: Some(0.0),
                ruwe: Some(1.0),
                solved: Some(if two_parameter { 3 } else { SOLVED_FIVE_PARAMETER }),
                dup: Some(false),
                ipd_frac_multi_peak: Some(0.0),
                sepsi: Some(0.5),
                g: Some(g),
                bp_rp: Some(0.8),
            });
            let noise_px = opts.noise_mas / MAS_PER_ARCSEC / pixel_scale;
            let mut dx = noise_px * gaussian(&mut rng);
            let mut dy = noise_px * gaussian(&mut rng);
            if i >= opts.n - n_outliers {
                let (lo, hi) = opts.outlier_range_mas;
                let r = (lo + (hi - lo) * rng.gen::<f64>()) / MAS_PER_ARCSEC / pixel_scale;
                let phi = 2.0 * PI * rng.gen::<f64>();
                dx = r * phi.cos();
                dy = r * phi.sin();
            }
            let sigma_c_px = (opts.sigma_c_mas / MAS_PER_ARCSEC / pixel_scale).max(1e-9);
            let snr = CENTROID_CRB_FACTOR * FWHM_PX / sigma_c_px;
            detections.push(TieDetection { x: x + dx, y: y + dy, fwhm_px: FWHM_PX, snr, mag_inst: g - 25.0 + 0.05 * gaussian(&mut rng), saturated: false });
            pm_list.push(pm);
        }
        Synthetic {
            detections,
            catalogue: GaiaCatalogue { kind: CatalogueKind::GaiaDr3Vizier, rows, quality_columns: true, cache_source: "test".into() },
            pm: pm_list,
        }
    }

    fn options(model: TieModel) -> TieOptions {
        TieOptions { model: ModelChoice::Fixed(model), ..TieOptions::default() }
    }

    fn tie(input_header: &HduHeader, synth: &Synthetic, epoch: Epoch, opts: &TieOptions, observer: Option<[f64; 3]>) -> TieResult {
        tie_with(input_header, synth, epoch, opts, observer, true)
    }

    fn tie_with(input_header: &HduHeader, synth: &Synthetic, epoch: Epoch, opts: &TieOptions, observer: Option<[f64; 3]>, space_telescope: bool) -> TieResult {
        let wcs = wcs_of(input_header);
        let size = size_of(input_header);
        let input = TieInput {
            wcs: &wcs,
            width: size.0,
            height: size.1,
            detections: &synth.detections,
            catalogue: &synth.catalogue,
            epoch,
            observer_barycentric_au: observer,
            space_telescope,
        };
        tie_to_gaia(&input, opts)
    }

    fn mean(v: &[f64]) -> f64 {
        v.iter().sum::<f64>() / v.len() as f64
    }

    fn sd(v: &[f64]) -> f64 {
        let m = mean(v);
        (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0)).sqrt()
    }

    fn sky_difference_mas(a: (f64, f64), b: (f64, f64)) -> [f64; 2] {
        let d_ra = (a.0 - b.0 + 180.0).rem_euclid(360.0) - 180.0;
        [d_ra * b.1.to_radians().cos() * MAS_PER_DEG, (a.1 - b.1) * MAS_PER_DEG]
    }

    fn max_grid_difference_mas(a: &WcsTransform, b: &WcsTransform, size: (usize, usize)) -> f64 {
        let mut worst: f64 = 0.0;
        for j in 0..GRID_SIZE {
            for i in 0..GRID_SIZE {
                let x = -0.5 + size.0 as f64 * (i as f64 + 0.5) / GRID_SIZE as f64;
                let y = -0.5 + size.1 as f64 * (j as f64 + 0.5) / GRID_SIZE as f64;
                let pa = a.pixel_to_world(x, y);
                let pb = b.pixel_to_world(x, y);
                worst = worst.max(norm_v(sky_difference_mas((pa.ra, pa.dec), (pb.ra, pb.dec))));
            }
        }
        worst
    }

    fn truth_centre_shift_mas(truth: &WcsTransform, input: &WcsTransform, size: (usize, usize)) -> [f64; 2] {
        let c = pixel_center(size.0, size.1);
        let t = truth.pixel_to_world(c.0, c.1);
        let i = input.pixel_to_world(c.0, c.1);
        sky_difference_mas((t.ra, t.dec), (i.ra, i.dec))
    }

    fn assert_exact_recovery(truth_header: &HduHeader, model: TieModel) -> TieResult {
        let truth = wcs_of(truth_header);
        let size = size_of(truth_header);
        let input_header = injected_input(truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, INJECTED_SCALE);
        let input = wcs_of(&input_header);
        let synth = synthesize(&truth, size, &SynthOptions::noise_free(300));
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(model), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        let update = result.header_update.as_ref().expect("exact header cards");
        let tied = wcs_of(&with_cards(&input_header, &update.cards));
        let worst = max_grid_difference_mas(&tied, &truth, size);
        eprintln!("S1 {}: grid max {worst:.3e} mas, passes {}, last update {:.3e} mas, matched {}, used {}", result.report.wcs_kind, result.report.passes, result.report.last_update_mas, result.report.n_matched, result.report.n_used);
        assert!(worst < 1e-6, "tied grid differs from the truth by {worst:e} mas");
        let expected_shift = truth_centre_shift_mas(&truth, &input, size);
        let got = result.report.params.shift_mas;
        assert!((got[0] - expected_shift[0]).abs() < 1e-6 && (got[1] - expected_shift[1]).abs() < 1e-6, "centre shift {got:?} vs {expected_shift:?} mas");
        assert!((result.report.params.rotation_deg - INJECTED_ROTATION_DEG).abs() < 1e-10, "rotation {}", result.report.params.rotation_deg);
        assert!((result.report.params.scale_ppm - (INJECTED_SCALE - 1.0) * 1e6).abs() < 1e-6, "scale {} ppm", result.report.params.scale_ppm);
        let correction = result.correction.as_ref().expect("correction operator");
        let (x, y) = (123.4, 1900.2);
        let via_operator = correction.corrected_pixel_to_world(&input, x, y).unwrap();
        let via_cards = tied.pixel_to_world(x, y);
        assert!(norm_v(sky_difference_mas(via_operator, (via_cards.ra, via_cards.dec))) < 1e-6, "operator and cards disagree");
        result
    }

    #[test]
    fn s1_noise_free_fgs_tan_recovers_the_injected_update_exactly() {
        let result = assert_exact_recovery(&fgs_header(false), TieModel::Rscale);
        assert_eq!(result.report.cuts.clipped, 0, "noise-free pairs must not be clipped");
        assert!(result.report.n_matched >= 290 && result.report.n_used == result.report.n_matched, "matched {} used {}", result.report.n_matched, result.report.n_used);
        assert!(result.report.passes >= 2, "passes {}", result.report.passes);
        assert!(result.report.sigma_sys_mas < 1e-6);
    }

    #[test]
    fn s1_noise_free_general_model_matches_the_truth_too() {
        assert_exact_recovery(&fgs_header(false), TieModel::General);
    }

    #[test]
    fn s1_noise_free_order3_sip_header_keeps_sip_and_matches_the_truth() {
        let result = assert_exact_recovery(&fgs_header(true), TieModel::Rscale);
        let update = result.header_update.as_ref().unwrap();
        let keys: Vec<&str> = update.cards.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["CRVAL1", "CRVAL2", "CD1_1", "CD1_2", "CD2_1", "CD2_2", "RADESYS"]);
        assert_eq!(update.remove, ["PC1_1", "PC1_2", "PC2_1", "PC2_2", "CDELT1", "CDELT2", "CROTA1", "CROTA2", "EQUINOX", "RADECSYS"]);
        assert!(update.remove.iter().all(|k| !k.starts_with("A_") && !k.starts_with("B_") && !k.starts_with("CRPIX")));
        assert_eq!(result.report.wcs_kind, "header TAN-SIP");
    }

    #[test]
    fn s1_noise_free_polar_field_at_dec_86_5_is_exact() {
        assert_exact_recovery(&polar_header(), TieModel::Rscale);
    }

    #[test]
    fn transport_carries_the_north_rotation_term_of_the_moving_tangent_point() {
        let from = Tangent::at(unit_vector(FGS_CENTRE.0, FGS_CENTRE.1));
        let to = Tangent::at(from.unproject(1.7 / RAD_TO_ARCSEC, 0.0));
        let b = transport(&from, &to);
        let angle = b[1][0].atan2(b[0][0]).abs();
        let expected = (1.7 / RAD_TO_ARCSEC) * FGS_CENTRE.1.to_radians().tan().abs();
        assert!((angle - expected).abs() < 0.01 * expected, "angle {angle:e} vs {expected:e} rad");
        assert!(angle * 100.0 * MAS_PER_ARCSEC > 1.0, "the term is {:.2} mas at a 100 arcsec corner", angle * 100.0 * MAS_PER_ARCSEC);
    }

    #[test]
    fn s2_half_mas_noise_meets_the_card_tolerances() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, INJECTED_SCALE);
        let input = wcs_of(&input_header);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noisy(300, 0.5, 7));
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rscale), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        let expected = truth_centre_shift_mas(&truth, &input, FGS_SIZE);
        let got = result.report.params.shift_mas;
        eprintln!("S2: shift error {:?} mas, rotation error {:.2e} deg, scale error {:.3} ppm", sub_v(got, expected), result.report.params.rotation_deg - INJECTED_ROTATION_DEG, result.report.params.scale_ppm - 30.0);
        assert!(norm_v(sub_v(got, expected)) < 3.0, "shift error {:?} mas", sub_v(got, expected));
        assert!((result.report.params.rotation_deg - INJECTED_ROTATION_DEG).abs() < 2e-4);
        assert!((result.report.params.scale_ppm - 30.0).abs() < 2.0);
    }

    #[test]
    fn s3_monte_carlo_bias_scatter_and_bootstrap_match_the_prediction() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, 1.0);
        let input = wcs_of(&input_header);
        let expected = truth_centre_shift_mas(&truth, &input, FGS_SIZE);
        let realisations = 200;
        let noise = 15.0;
        let opts = TieOptions { bootstrap_resamples: 50, ..options(TieModel::Rshift) };
        let mut ex = Vec::new();
        let mut ey = Vec::new();
        let mut erot = Vec::new();
        let mut boot_shift = Vec::new();
        let mut boot_rot = Vec::new();
        let mut rms = Vec::new();
        let mut r_rms_arcsec = 0.0;
        let mut median_refusals = 0;
        for seed in 0..realisations {
            let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noisy(300, noise, 1000 + seed));
            let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &opts, None);
            if result.decision == TieDecision::Refused && result.reason.as_deref().is_some_and(|r| r.starts_with("model inadequate")) {
                median_refusals += 1;
                continue;
            }
            assert_eq!(result.decision, TieDecision::Applied, "seed {seed}: {:?}", result.reason);
            let r = &result.report;
            ex.push(r.params.shift_mas[0] - expected[0]);
            ey.push(r.params.shift_mas[1] - expected[1]);
            erot.push(r.params.rotation_deg - INJECTED_ROTATION_DEG);
            let b = r.sigma_bootstrap.unwrap();
            boot_shift.push(0.5 * (b.shift_mas[0] + b.shift_mas[1]));
            boot_rot.push(b.rotation_deg);
            rms.push(0.5 * (r.after.robust_rms_mas[0] + r.after.robust_rms_mas[1]));
            if seed == 0 {
                let centre = pixel_center(FGS_SIZE.0, FGS_SIZE.1);
                let scale = input.pixel_scale_arcsec();
                let n = synth.detections.len() as f64;
                r_rms_arcsec = (synth.detections.iter().map(|d| ((d.x - centre.0).powi(2) + (d.y - centre.1).powi(2)) * scale * scale).sum::<f64>() / n).sqrt();
            }
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let sd = |v: &[f64]| {
            let m = mean(v);
            (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0)).sqrt()
        };
        assert!(median_refusals <= 3, "{median_refusals} of {realisations} realisations tripped the 3 sigma hold-out median rule");
        let sigma_shift = noise / (300f64).sqrt();
        let sigma_rot = (noise / MAS_PER_ARCSEC / ((300f64).sqrt() * r_rms_arcsec)).to_degrees();
        let bias_limit = 3.0 / (realisations as f64).sqrt();
        eprintln!(
            "S3: n {} (median refusals {median_refusals}); bias E {:.3} N {:.3} rot {:.2e}; sd E {:.3} N {:.3} rot {:.2e}; predicted shift {:.3} rot {:.2e}; bootstrap mean shift {:.3} rot {:.2e}; post-fit RMS {:.2} mas",
            ex.len(), mean(&ex), mean(&ey), mean(&erot), sd(&ex), sd(&ey), sd(&erot), sigma_shift, sigma_rot, mean(&boot_shift), mean(&boot_rot), mean(&rms)
        );
        assert!(mean(&ex).abs() < bias_limit * sigma_shift, "east bias {:.3} mas vs sigma {:.3}", mean(&ex), sigma_shift);
        assert!(mean(&ey).abs() < bias_limit * sigma_shift, "north bias {:.3} mas vs sigma {:.3}", mean(&ey), sigma_shift);
        assert!(mean(&erot).abs() < bias_limit * sigma_rot, "rotation bias {:e} deg vs sigma {:e}", mean(&erot), sigma_rot);
        let sd_shift = 0.5 * (sd(&ex) + sd(&ey));
        assert!((sd_shift / sigma_shift - 1.0).abs() < 0.15, "shift sd {sd_shift:.3} vs predicted {sigma_shift:.3} mas");
        assert!((sd(&erot) / sigma_rot - 1.0).abs() < 0.15, "rotation sd {:e} vs predicted {:e} deg", sd(&erot), sigma_rot);
        assert!((mean(&boot_shift) / sd_shift - 1.0).abs() < 0.30, "bootstrap shift sigma {:.3} vs Monte Carlo {sd_shift:.3} mas", mean(&boot_shift));
        assert!((mean(&boot_rot) / sd(&erot) - 1.0).abs() < 0.30, "bootstrap rotation sigma {:e} vs Monte Carlo {:e}", mean(&boot_rot), sd(&erot));
        assert!((mean(&rms) / noise - 1.0).abs() < 0.10, "post-fit robust RMS {:.2} vs injected {noise} mas", mean(&rms));
    }

    #[test]
    fn s4_outliers_inside_the_match_radius_are_rejected_and_counted() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, 1.0);
        let input = wcs_of(&input_header);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions { outlier_fraction: 0.3, outlier_range_mas: (40.0, 65.0), ..SynthOptions::noisy(300, 5.0, 11) });
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rshift), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        let r = &result.report;
        assert!(r.match_radius_arcsec >= 0.065, "match radius {}", r.match_radius_arcsec);
        let injected = r.stars.iter().filter(|s| s.id.trim_start_matches('G').parse::<usize>().unwrap() >= 210).count() as i64;
        eprintln!("S4: matched {}, clipped {}, injected outliers matched {injected}, match radius {:.3} arcsec, sigma_sys {:.2} mas", r.n_matched, r.cuts.clipped, r.match_radius_arcsec, r.sigma_sys_mas);
        assert!(injected >= 80, "outliers matched {injected}");
        assert!((r.cuts.clipped as i64 - injected).abs() <= 5, "clipped {} vs injected {injected}", r.cuts.clipped);
        assert!(r.stars.iter().all(|s| s.used == (s.id.trim_start_matches('G').parse::<usize>().unwrap() < 210)), "clipping did not separate the injected outliers");
        let expected = truth_centre_shift_mas(&truth, &input, FGS_SIZE);
        let sigma_shift = 5.0 / (210f64).sqrt();
        assert!(norm_v(sub_v(r.params.shift_mas, expected)) < 4.0 * sigma_shift * 2f64.sqrt(), "shift error {:?} mas", sub_v(r.params.shift_mas, expected));
        let sigma_rot = (5.0 / MAS_PER_ARCSEC / ((210f64).sqrt() * 57.6)).to_degrees();
        assert!((r.params.rotation_deg - INJECTED_ROTATION_DEG).abs() < 4.0 * sigma_rot, "rotation error {:e}", r.params.rotation_deg - INJECTED_ROTATION_DEG);
    }

    #[test]
    fn s5_sigma_sys_is_recovered_from_underestimated_centroid_errors() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions { noise_mas: (100f64 + 400.0).sqrt(), sigma_c_mas: 10.0, ..SynthOptions::noisy(300, 10.0, 21) });
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Shift), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        eprintln!("S5: sigma_sys {:.2} mas, chi2_nu {:.4}, used {}", result.report.sigma_sys_mas, result.report.chi2_nu, result.report.n_used);
        assert!((result.report.sigma_sys_mas - 20.0).abs() < 3.0, "sigma_sys {:.2} mas", result.report.sigma_sys_mas);
        assert!((result.report.chi2_nu - 1.0).abs() < 1e-2, "chi2_nu {}", result.report.chi2_nu);
    }

    #[test]
    fn s6_epoch_propagation_recovers_the_truth_and_its_absence_biases_by_the_mean_motion() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let input = wcs_of(&input_header);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions { pm_sigma_masyr: 5.0, pm_mean_masyr: [1.819, 0.362], two_parameter_fraction: 0.2, ..SynthOptions::noise_free(300) });
        let expected = truth_centre_shift_mas(&truth, &input, FGS_SIZE);

        let with = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Shift), None);
        assert_eq!(with.decision, TieDecision::Applied, "reason: {:?}", with.reason);
        assert!(norm_v(sub_v(with.report.params.shift_mas, expected)) < 1e-6, "propagated shift error {:?} mas", sub_v(with.report.params.shift_mas, expected));
        assert_eq!(with.report.cuts.two_parameter, 60);
        assert!(with.report.n_matched >= 230 && with.report.n_matched <= 240, "matched {}", with.report.n_matched);
        assert!(with.report.stars.iter().all(|s| s.id.trim_start_matches('G').parse::<usize>().unwrap() >= 60));
        assert_eq!(with.report.proper_motion, "linear from J2016.0");

        let without = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions { propagate_proper_motion: false, ..options(TieModel::Shift) }, None);
        assert_eq!(without.decision, TieDecision::Applied, "reason: {:?}", without.reason);
        let used: Vec<usize> = without.report.stars.iter().filter(|s| s.used).map(|s| s.id.trim_start_matches('G').parse::<usize>().unwrap()).collect();
        let dt = FGS_EPOCH - GAIA_REFERENCE_EPOCH_YEAR;
        let mean_pm = used.iter().fold([0.0; 2], |a, &i| [a[0] + synth.pm[i][0], a[1] + synth.pm[i][1]]);
        let bias = [mean_pm[0] / used.len() as f64 * dt, mean_pm[1] / used.len() as f64 * dt];
        let error = sub_v(without.report.params.shift_mas, expected);
        eprintln!("S6: propagated error {:?} mas; unpropagated error {error:?} mas vs mean motion {bias:?} mas; two-parameter rows cut {}", sub_v(with.report.params.shift_mas, expected), with.report.cuts.two_parameter);
        assert!(norm_v(bias) > 10.0, "the LMC bulk motion over 6.855 yr is about 12.7 mas, got {bias:?} mas");
        assert!(norm_v(add_v(error, bias)) < 0.5, "unpropagated shift error {error:?} should equal minus the mean motion {bias:?} mas");
    }

    #[test]
    fn s7_parallax_displacement_matches_an_independent_geometric_computation() {
        let b = jwst_barycentric_au();
        let plx = 50.0;
        let (ra, dec) = FGS_CENTRE;
        let got = parallax_displacement_mas(ra, dec, plx, b);
        let distance_au = RAD_TO_ARCSEC / (plx / MAS_PER_ARCSEC);
        let u = unit_vector(ra, dec);
        let apparent = normalize(add3(scale3(u, distance_au), scale3(b, -1.0)));
        let (ra2, dec2) = ra_dec_deg(apparent);
        let expected = sky_difference_mas((ra2, dec2), (ra, dec));
        eprintln!("S7: displacement {got:?} mas, geometric {expected:?} mas");
        assert!((got[0] - expected[0]).abs() < 1e-3 && (got[1] - expected[1]).abs() < 1e-3, "formula {got:?} vs geometry {expected:?} mas");
        assert!(norm_v(got) > 20.0, "a 50 mas parallax star moves by {:.1} mas", norm_v(got));

        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions { parallax_star: Some((plx, b)), ..SynthOptions::noise_free(300) });
        let with = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Shift), Some(b));
        assert_eq!(with.decision, TieDecision::Applied, "reason: {:?}", with.reason);
        let star = with.report.stars.iter().find(|s| s.id == "G0").expect("the parallax star is matched");
        assert!(star.used && norm_v(star.after_mas) < 1e-6, "parallax applied: residual {:?} mas", star.after_mas);
        let without = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Shift), None);
        assert_eq!(without.decision, TieDecision::Applied, "reason: {:?}", without.reason);
        let star = without.report.stars.iter().find(|s| s.id == "G0").expect("matched");
        let typical = median_of(&without.report.stars.iter().map(|s| s.weight).collect::<Vec<_>>());
        assert!(star.weight < 1e-3 * typical, "parallax inflates the error: weight {} vs typical {}", star.weight, typical);
    }

    #[test]
    fn s8_too_few_pairs_names_the_count() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(9));
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions::default(), None);
        assert_eq!(result.decision, TieDecision::Refused);
        assert_eq!(result.reason.as_deref(), Some("9 < 10 used pairs for shift"));
    }

    #[test]
    fn s8_pure_noise_refuses_on_the_histogram_peak() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let stars = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(200));
        let detections = synthesize(&truth, FGS_SIZE, &SynthOptions { seed: 99, ..SynthOptions::noise_free(200) });
        let synth = Synthetic { detections: detections.detections, catalogue: stars.catalogue, pm: Vec::new() };
        let result = tie(&truth_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions::default(), None);
        assert_eq!(result.decision, TieDecision::Refused);
        let reason = result.reason.unwrap();
        eprintln!("S8 peak: {reason}");
        assert!(reason.starts_with("no significant offset peak within 6.0 arcsec (peak "), "{reason}");
        assert!(result.report.histogram_peak < 5, "peak {}", result.report.histogram_peak);
    }

    #[test]
    fn s8_crowded_field_refuses_when_the_chance_radius_is_below_two_sigma() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let crowd = synthesize(&truth, FGS_SIZE, &SynthOptions::noisy(10_000, 100.0, 5));
        let synth = Synthetic { detections: crowd.detections[..300].to_vec(), catalogue: crowd.catalogue, pm: Vec::new() };
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions::default(), None);
        assert_eq!(result.decision, TieDecision::Refused, "reason: {:?}", result.reason);
        let reason = result.reason.unwrap();
        eprintln!("S8 crowded: {reason} (density {:.3} per arcsec2)", result.report.density_per_arcsec2);
        assert!(reason.starts_with("too crowded: 2 % chance radius "), "{reason}");
        assert!(result.report.density_per_arcsec2 > 0.4, "density {}", result.report.density_per_arcsec2);
    }

    #[test]
    fn s8_a_2mass_tier_is_refused_by_name() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let mut synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(100));
        synth.catalogue.kind = CatalogueKind::Local2mass;
        let result = tie(&truth_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions::default(), None);
        assert_eq!(result.decision, TieDecision::Refused);
        assert_eq!(result.reason.as_deref(), Some("local tier 1 (2MASS) is not Gaia DR3"));
    }

    #[test]
    fn s8_the_siril_extract_is_refused_by_default_and_quality_unknown_with_an_override() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let mut synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(100));
        synth.catalogue.kind = CatalogueKind::GaiaDr3Siril;
        synth.catalogue.quality_columns = false;
        for row in &mut synth.catalogue.rows {
            row.ruwe = None;
            row.e_ra_mas = None;
            row.e_dec_mas = None;
            row.pm_ra_masyr = None;
            row.pm_dec_masyr = None;
            row.e_pm_ra_masyr = None;
            row.e_pm_dec_masyr = None;
            row.solved = None;
        }
        let refused = tie_with(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Shift), None, false);
        assert_eq!(refused.decision, TieDecision::Refused);
        assert!(refused.reason.as_deref().unwrap().contains("Siril"), "{:?}", refused.reason);
        let allowed = tie_with(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions { override_source: true, ..options(TieModel::Shift) }, None, false);
        assert_eq!(allowed.decision, TieDecision::Applied, "reason: {:?}", allowed.reason);
        assert!(allowed.report.warnings.iter().any(|w| w.contains("quality unknown")), "{:?}", allowed.report.warnings);
        assert!(allowed.report.stars.iter().all(|s| s.flags.contains(&FLAG_NO_PROPER_MOTION)), "rows without proper motion are flagged");
    }

    #[test]
    fn s8_zero_offset_is_no_significant_correction_and_never_worse_than_the_input() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let mut no_correction = 0;
        let mut median_refusals = 0;
        for seed in 0..20 {
            let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noisy(300, 15.0, 500 + seed));
            let result = tie(&truth_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rshift), None);
            let r = &result.report;
            match result.decision {
                TieDecision::Refused if result.reason.as_deref().is_some_and(|m| m.starts_with("model inadequate")) => {
                    median_refusals += 1;
                    assert!(result.header_update.is_none() && result.correction.is_none());
                }
                TieDecision::NoSignificantCorrection => {
                    no_correction += 1;
                    assert!(result.header_update.is_none() && result.correction.is_none());
                    assert!(r.cv_rms_after_mas >= r.cv_rms_before_mas, "seed {seed}: {} < {}", r.cv_rms_after_mas, r.cv_rms_before_mas);
                    assert!(result.reason.as_deref().unwrap().starts_with("no significant correction: the WCS already agrees with Gaia DR3 within "), "{:?}", result.reason);
                }
                TieDecision::Applied => {
                    assert!(r.cv_rms_after_mas < r.cv_rms_before_mas, "seed {seed}: applied without a hold-out gain");
                    let sigma = norm_v(r.sigma_formal.shift_mas).max(r.sigma_bootstrap.map_or(0.0, |b| norm_v(b.shift_mas)));
                    assert!(norm_v(r.params.shift_mas) < 3.0 * sigma + 1.0, "seed {seed}: applied shift {:?} mas is not consistent with zero (sigma {sigma:.2})", r.params.shift_mas);
                }
                TieDecision::Refused => panic!("seed {seed}: refused: {:?}", result.reason),
            }
        }
        eprintln!("S8 zero offset: {no_correction} no significant correction, {median_refusals} median refusals, {} applied within 3 sigma of zero", 20 - no_correction - median_refusals);
        assert!(median_refusals <= 2, "{median_refusals} of 20 tripped the 3 sigma hold-out median rule");
        assert!(no_correction >= 17, "only {no_correction} of 20 zero-offset fields returned no significant correction");
    }

    #[test]
    fn fit_apply_and_fit_again_is_a_no_op() {
        let truth_header = fgs_header(true);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, INJECTED_SCALE);
        let truth = wcs_of(&truth_header);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(300));
        let first = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rscale), None);
        assert_eq!(first.decision, TieDecision::Applied, "reason: {:?}", first.reason);
        let tied_header = with_cards(&input_header, &first.header_update.as_ref().unwrap().cards);
        let second = tie(&tied_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rscale), None);
        assert_ne!(second.decision, TieDecision::Refused, "reason: {:?}", second.reason);
        let p = second.report.params;
        eprintln!("fit again: decision {:?}, params {p:?}, before RMS {:.3e} mas", second.decision, second.report.before.robust_rms_mas[2]);
        assert!(norm_v(p.shift_mas) < 1e-6 && p.rotation_deg.abs() < 1e-10 && p.scale_ppm.abs() < 1e-6, "second tie moved the WCS: {p:?}");
        assert!(second.report.before.robust_rms_mas[2] < 1e-6, "the tied WCS already agrees with Gaia: {:?}", second.report.before);
    }

    #[test]
    fn undated_image_keeps_j2016_and_reports_the_epoch_term() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions { pm_sigma_masyr: 3.0, epoch_year: GAIA_REFERENCE_EPOCH_YEAR, ..SynthOptions::noise_free(200) });
        let result = tie(&input_header, &synth, Epoch::none(), &options(TieModel::Shift), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        let r = &result.report;
        assert_eq!(r.epoch_source, EpochSource::None);
        assert_eq!(r.epoch_year, GAIA_REFERENCE_EPOCH_YEAR);
        assert!(r.warnings.iter().any(|w| w.contains("no observation date")), "{:?}", r.warnings);
        let term = r.epoch_systematic_mas.expect("epoch term");
        let used: Vec<usize> = r.stars.iter().filter(|s| s.used).map(|s| s.id.trim_start_matches('G').parse::<usize>().unwrap()).collect();
        let mean = used.iter().fold([0.0; 2], |a, &i| [a[0] + synth.pm[i][0], a[1] + synth.pm[i][1]]);
        let expected = norm_v([mean[0] / used.len() as f64, mean[1] / used.len() as f64]) * DEFAULT_EPOCH_UNCERTAINTY_YR;
        assert!((term - expected).abs() < 1e-6, "epoch term {term} vs {expected} mas");
    }

    #[test]
    fn attribution_and_catalogue_strings_are_on_every_report() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(50));
        let result = tie(&truth_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Shift), None);
        assert_eq!(result.report.attribution, GAIA_DR3_ATTRIBUTION);
        assert!(result.report.attribution.contains("CC BY-NC 3.0 IGO"));
        assert_eq!(result.report.catalogue, GAIA_DR3_CATALOGUE);
        assert!(result.report.acknowledgement.starts_with("This work has made use of data from the European Space Agency (ESA) mission Gaia"));
    }

    #[test]
    fn typical_gaia_errors_follow_the_edr3_curve() {
        assert!((typical_gaia_sigma_mas(14.0).0 - 0.025).abs() < 1e-9);
        let (p17, m17) = typical_gaia_sigma_mas(17.0);
        assert!((p17 - 0.05).abs() < 1e-9 && (m17 - 0.07).abs() < 1e-9);
        let (p20, m20) = typical_gaia_sigma_mas(20.0);
        assert!((p20 - 0.4).abs() < 1e-9 && (m20 - 0.5).abs() < 1e-9);
        let (p185, _) = typical_gaia_sigma_mas(18.5);
        assert!(p185 > 0.05 && p185 < 0.4);
    }

    #[test]
    fn chance_radius_matches_the_measured_fgs_and_dss2_values() {
        assert!((chance_radius_arcsec(0.098, 0.02) - 0.256).abs() < 0.01);
        assert!((chance_radius_arcsec(6.3e-4, 0.02) - 3.2).abs() < 0.1);
        assert!(chance_radius_arcsec(0.0, 0.02).is_infinite());
    }

    #[test]
    fn s1_pc_cdelt_header_update_leaves_no_pc_form_behind_for_a_pc_first_reader() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = pc_form(&injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, INJECTED_SCALE));
        assert!(input_header.get("CD1_1").is_none() && input_header.get("PC1_1").is_some());
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(300));
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rscale), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        let update = result.header_update.as_ref().expect("a PC+CDELT TAN header is an exact case");
        let tied_header = apply_update(&input_header, update);
        for key in ["PC1_1", "PC1_2", "PC2_1", "PC2_2", "CDELT1", "CDELT2"] {
            assert!(tied_header.get(key).is_none(), "{key} survives next to the CD cards");
        }
        let mut as_read_pc_first = tied_header.clone();
        let m = pc_first_matrix(&tied_header);
        for key in ["PC1_1", "PC1_2", "PC2_1", "PC2_2", "CDELT1", "CDELT2"] {
            as_read_pc_first.remove(key);
        }
        as_read_pc_first.set("CD1_1", format!("{:.16E}", m[0][0]));
        as_read_pc_first.set("CD1_2", format!("{:.16E}", m[0][1]));
        as_read_pc_first.set("CD2_1", format!("{:.16E}", m[1][0]));
        as_read_pc_first.set("CD2_2", format!("{:.16E}", m[1][1]));
        let worst = max_grid_difference_mas(&wcs_of(&as_read_pc_first), &truth, FGS_SIZE);
        eprintln!("S1 PC+CDELT: a PC-first reader of the tied header differs from the truth by {worst:.3e} mas");
        assert!(worst < 1e-6, "a PC-first reader is {worst:e} mas off the truth");
    }

    #[test]
    fn s1_fk5_j2000_header_gets_icrs_cards_and_loses_its_equinox() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = fk5_form(&injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, INJECTED_SCALE));
        assert!(input_header.get("RADESYS").is_none() && input_header.get_f64("EQUINOX") == Some(2000.0));
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(300));
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rscale), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        let update = result.header_update.as_ref().expect("an EQUINOX 2000 TAN header is an exact case");
        assert!(update.cards.iter().any(|(k, v)| k == "RADESYS" && v.trim().trim_matches('\'') == "ICRS"), "cards {:?}", update.cards);
        let tied_header = apply_update(&input_header, update);
        assert_eq!(tied_header.get("RADESYS").map(|v| v.trim().trim_matches('\'')), Some("ICRS"));
        assert!(tied_header.get("EQUINOX").is_none(), "EQUINOX survives next to RADESYS = ICRS");
        let worst = max_grid_difference_mas(&wcs_of(&tied_header), &truth, FGS_SIZE);
        assert!(worst < 1e-6, "tied grid differs from the truth by {worst:e} mas");
    }

    #[test]
    fn s1_corner_crpix_header_is_exact() {
        let result = assert_exact_recovery(&corner_crpix_header(), TieModel::Rscale);
        let pivot = (result.report.pivot_deg[0], result.report.pivot_deg[1]);
        let from_corner_crval = norm_v(sky_difference_mas(pivot, FGS_CENTRE));
        assert!((from_corner_crval - norm_v(INJECTED_SHIFT_ARCSEC) * MAS_PER_ARCSEC).abs() < 1.0, "the pivot sits {from_corner_crval:.1} mas from the corner CRVAL");
        let centre = wcs_of(&corner_crpix_header()).pixel_to_world(1023.5, 1023.5);
        assert!(norm_v(sky_difference_mas(pivot, (centre.ra, centre.dec))) > 60.0 * MAS_PER_ARCSEC, "the pivot is the image centre, not the corner CRPIX");
    }

    #[test]
    fn a_downgrade_on_a_later_pass_restarts_from_the_identity() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, 1.0);
        let input = wcs_of(&input_header);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(15));
        let (_, _, crval1, crval2, _, _) = input.raw_params();
        let p0 = unit_vector(crval1, crval2);
        let tangent0 = Tangent::at(p0);
        let l: Vec<[f64; 2]> = synth.detections.iter().map(|d| {
            let pl = plane_of(&input, &tangent0, d.x, d.y).unwrap();
            [pl[0] * RAD_TO_ARCSEC, pl[1] * RAD_TO_ARCSEC]
        }).collect();
        let gaia_v: Vec<[f64; 3]> = synth.catalogue.rows.iter().map(|r| unit_vector(r.ra_deg, r.dec_deg)).collect();
        let sigma2 = vec![1e-10; 15];
        let data = PassData { l: &l, gaia_v: &gaia_v, sigma2: &sigma2, p0, half_diagonal: 100.0 };
        let mut calls = 0;
        let mut dropping = |kind: TieModel, p: &[[f64; 2]], q: &[[f64; 2]], s: &[f64]| {
            calls += 1;
            let mut fit = fit_with_clipping(kind, p, q, s)?;
            if calls == 2 {
                let first = fit.inliers.iter().position(|&b| b).unwrap();
                fit.inliers[first] = false;
                fit.n_used -= 1;
            }
            Some(fit)
        };
        let mut notes = Vec::new();
        let outcome = run_passes(TieModel::Rshift, &data, &mut notes, &mut dropping).expect("the shift model still fits");
        let params = params_from_matrix(outcome.correction.transported_matrix(), [0.0; 2]);
        eprintln!("late downgrade: model {:?}, passes {}, notes {notes:?}, applied rotation {:.3e} deg", outcome.model, outcome.passes, params.rotation_deg);
        assert_eq!(outcome.model, TieModel::Shift);
        assert!(notes.iter().any(|n| n.contains("14 < 15 used pairs for rshift: downgraded to shift")), "{notes:?}");
        assert!(params.rotation_deg.abs() < 1e-9, "the shift model carries the rshift pass's rotation of {} deg", params.rotation_deg);
        assert!(params.scale_ppm.abs() < 1e-6 && outcome.passes >= 1);
    }

    #[test]
    fn the_report_carries_the_g_range_the_rotation_in_arcsec_and_per_star_flags() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, 1.0);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions { outlier_fraction: 0.3, outlier_range_mas: (40.0, 65.0), parallax_star: Some((50.0, jwst_barycentric_au())), ..SynthOptions::noisy(300, 5.0, 11) });
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rshift), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        let r = &result.report;
        assert!((r.rotation_arcsec - r.params.rotation_deg * 3600.0).abs() < 1e-9);
        let used_g: Vec<f64> = r.stars.iter().filter(|s| s.used).filter_map(|s| s.g).collect();
        let range = r.g_range.expect("G range");
        assert_eq!(range, [used_g.iter().cloned().fold(f64::INFINITY, f64::min), used_g.iter().cloned().fold(f64::NEG_INFINITY, f64::max)]);
        assert!(r.stars.iter().all(|s| s.used != s.flags.contains(&FLAG_CLIPPED)));
        assert!(r.stars.iter().filter(|s| !s.used).count() >= 80);
        let parallax_star = r.stars.iter().find(|s| s.id == "G0").expect("G0 matched");
        assert!(parallax_star.flags.contains(&FLAG_PARALLAX_INFLATED) && !parallax_star.flags.contains(&FLAG_PARALLAX_APPLIED), "{:?}", parallax_star.flags);
        assert!(r.stars.iter().filter(|s| s.id != "G0").all(|s| !s.flags.iter().any(|f| f.starts_with("parallax"))));
        assert!(r.stars.iter().all(|s| !s.flags.contains(&FLAG_NO_PROPER_MOTION)));
        assert!(r.wcs_origin.is_none() && r.banners.is_empty() && r.cone_centre_deg.is_none() && r.observer_source.is_none(), "caller-filled fields start empty");
    }

    #[test]
    fn s1_non_tan_projection_gets_the_operator_only_and_the_operator_is_exact() {
        let truth_header = sin_header();
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, 1.0);
        let input = wcs_of(&input_header);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(300));
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &options(TieModel::Rshift), None);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        assert!(result.header_update.is_none(), "a SIN header must not get exact TAN cards");
        assert!(result.report.representation.starts_with("sky correction operator"), "{}", result.report.representation);
        assert_eq!(result.report.wcs_kind, "header SIN");
        let correction = result.correction.as_ref().expect("operator");
        let mut worst: f64 = 0.0;
        for j in 0..GRID_SIZE {
            for i in 0..GRID_SIZE {
                let x = -0.5 + FGS_SIZE.0 as f64 * (i as f64 + 0.5) / GRID_SIZE as f64;
                let y = -0.5 + FGS_SIZE.1 as f64 * (j as f64 + 0.5) / GRID_SIZE as f64;
                let via_operator = correction.corrected_pixel_to_world(&input, x, y).unwrap();
                let t = truth.pixel_to_world(x, y);
                worst = worst.max(norm_v(sky_difference_mas(via_operator, (t.ra, t.dec))));
            }
        }
        eprintln!("S1 SIN: operator grid max {worst:.3e} mas, passes {}", result.report.passes);
        assert!(worst < 1e-6, "operator differs from the truth by {worst:e} mas");
    }

    #[test]
    fn auto_ground_default_fits_the_general_model_on_a_noisy_field() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, INJECTED_SCALE);
        let input = wcs_of(&input_header);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noisy(300, 15.0, 77));
        let result = tie_with(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions::default(), None, false);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        let r = &result.report;
        assert_eq!(r.model.as_deref(), Some("general"), "notes {:?}", r.model_notes);
        let expected = truth_centre_shift_mas(&truth, &input, FGS_SIZE);
        let sigma_shift = 15.0 / (300f64).sqrt();
        let sigma_rot = (15.0 / MAS_PER_ARCSEC / ((300f64).sqrt() * 57.6)).to_degrees();
        let b = r.sigma_bootstrap.expect("bootstrap");
        eprintln!("auto general: shift error {:?} mas (formal {:?}, bootstrap {:?}), rotation error {:.2e} deg (formal {:.2e}, bootstrap {:.2e}), scale error {:.2} ppm, aspect {:.2} ppm, skew {:.2e} deg", sub_v(r.params.shift_mas, expected), r.sigma_formal.shift_mas, b.shift_mas, r.params.rotation_deg - INJECTED_ROTATION_DEG, r.sigma_formal.rotation_deg, b.rotation_deg, r.params.scale_ppm - 30.0, r.params.aspect_ppm, r.params.skew_deg);
        assert!(norm_v(sub_v(r.params.shift_mas, expected)) < 4.0 * sigma_shift * 2f64.sqrt(), "shift error {:?}", sub_v(r.params.shift_mas, expected));
        assert!((r.params.rotation_deg - INJECTED_ROTATION_DEG).abs() < 4.0 * sigma_rot, "rotation error {:e}", r.params.rotation_deg - INJECTED_ROTATION_DEG);
        assert!((r.params.scale_ppm - 30.0).abs() < 4.0 * 15.0, "scale error {} ppm", r.params.scale_ppm - 30.0);
        assert!(!r.errors_disagree, "formal {:?} vs bootstrap {:?}", r.sigma_formal.shift_mas, b.shift_mas);
        let ratio = norm_v(r.sigma_formal.shift_mas) / norm_v(b.shift_mas);
        assert!(ratio > 0.7 && ratio < 1.4, "formal/bootstrap shift sigma ratio {ratio}");
        let ground_choice = tie_with(&input_header, &synthesize(&truth, FGS_SIZE, &SynthOptions::noisy(25, 15.0, 78)), Epoch::header(FGS_EPOCH), &TieOptions::default(), None, false);
        assert_ne!(ground_choice.report.model.as_deref(), Some("general"), "25 pairs must not pick general: {:?}", ground_choice.report.model_notes);
    }

    #[test]
    fn formal_shift_error_refers_to_the_image_centre_when_the_matched_stars_sit_in_a_corner() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, INJECTED_ROTATION_DEG, 1.0);
        let input = wcs_of(&input_header);
        let expected = truth_centre_shift_mas(&truth, &input, FGS_SIZE);
        let side = 0.4 * FGS_SIZE.0 as f64;
        let region = [40.0, 40.0 + side, 40.0, 40.0 + side];
        let opts = TieOptions { bootstrap_resamples: 50, ..options(TieModel::Rshift) };
        let realisations = 80;
        let mut ex = Vec::new();
        let mut ey = Vec::new();
        let mut formal = Vec::new();
        let mut boot = Vec::new();
        let mut disagree = 0;
        for seed in 0..realisations {
            let synth = synthesize(&truth, FGS_SIZE, &SynthOptions { region: Some(region), ..SynthOptions::noisy(300, 15.0, 3000 + seed) });
            let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &opts, None);
            if result.decision == TieDecision::Refused && result.reason.as_deref().is_some_and(|r| r.starts_with("model inadequate")) {
                continue;
            }
            assert_eq!(result.decision, TieDecision::Applied, "seed {seed}: {:?}", result.reason);
            let r = &result.report;
            assert_eq!(r.model.as_deref(), Some("rshift"), "seed {seed}: {:?}", r.model_notes);
            ex.push(r.params.shift_mas[0] - expected[0]);
            ey.push(r.params.shift_mas[1] - expected[1]);
            formal.push(norm_v(r.sigma_formal.shift_mas));
            boot.push(norm_v(r.sigma_bootstrap.unwrap().shift_mas));
            if r.errors_disagree {
                disagree += 1;
            }
        }
        let sd_norm = sd(&ex).hypot(sd(&ey));
        eprintln!("corner field: n {}, Monte Carlo centre-shift sd {sd_norm:.3} mas, mean formal {:.3} mas, mean bootstrap {:.3} mas, errors_disagree on {disagree}", ex.len(), mean(&formal), mean(&boot));
        assert!(ex.len() >= realisations as usize - 4, "{} of {realisations} realisations applied", ex.len());
        assert!((mean(&formal) / sd_norm - 1.0).abs() < 0.25, "formal centre-shift sigma {:.3} vs Monte Carlo {sd_norm:.3} mas", mean(&formal));
        assert!((mean(&boot) / sd_norm - 1.0).abs() < 0.30, "bootstrap centre-shift sigma {:.3} vs Monte Carlo {sd_norm:.3} mas", mean(&boot));
        assert!(disagree <= realisations as usize / 10, "errors_disagree fired on {disagree} correct fits");
    }

    fn straddling_pairs() -> (Vec<Det>, Vec<GaiaStar>) {
        let mut dets = Vec::new();
        let mut stars = Vec::new();
        for i in 0..10usize {
            let base = [10.0 * i as f64, 10.0 * i as f64];
            let sx = if i % 2 == 0 { -0.03 } else { 0.03 };
            let sy = if (i / 2) % 2 == 0 { -0.03 } else { 0.03 };
            dets.push(Det { x: 0.0, y: 0.0, plane0: base, sigma2: 1e-10, mag_inst: -10.0, fwhm_arcsec: 0.17 });
            stars.push(GaiaStar { index: i, v: [1.0, 0.0, 0.0], plane0: add_v(base, [1.0 + sx, -0.6 + sy]), sigma2: 1e-10, g: Some(15.0), bp_rp: None, pm: [0.0; 2], pm_known: true, parallax_known: false });
        }
        (dets, stars)
    }

    #[test]
    fn offset_peak_survives_pairs_that_straddle_a_bin_corner() {
        let (dets, stars) = straddling_pairs();
        let order = eta_sorted(&stars);
        let peak = find_offset_peak(&dets, &stars, &order, DEFAULT_SEARCH_RADIUS_ARCSEC, HISTOGRAM_BIN_MIN_ARCSEC, 0.0);
        eprintln!("straddle: peak {} of 10 pairs, threshold {}, offset {:?}, sigma_res {:.3}", peak.peak, peak.threshold, peak.offset, peak.sigma_res);
        assert_eq!(peak.threshold, PEAK_MIN_COUNT);
        assert!(peak.peak >= 9, "the peak holds only {} of 10 pairs that straddle a bin corner", peak.peak);
        assert!((peak.offset[0] - 1.0).abs() < 0.031 && (peak.offset[1] + 0.6).abs() < 0.031, "offset {:?}", peak.offset);
    }

    #[test]
    fn s8_the_siril_override_is_for_amateur_fields_only_and_names_its_source() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let mut synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(100));
        synth.catalogue.kind = CatalogueKind::GaiaDr3Siril;
        synth.catalogue.quality_columns = false;
        let opts = TieOptions { override_source: true, ..options(TieModel::Shift) };
        let space = tie_with(&input_header, &synth, Epoch::header(FGS_EPOCH), &opts, None, true);
        assert_eq!(space.decision, TieDecision::Refused, "a space-telescope field accepted the Siril extract: {:?}", space.reason);
        assert!(space.reason.as_deref().unwrap().contains("amateur"), "{:?}", space.reason);
        let ground = tie_with(&input_header, &synth, Epoch::header(FGS_EPOCH), &opts, None, false);
        assert_eq!(ground.decision, TieDecision::Applied, "reason: {:?}", ground.reason);
        assert!(ground.report.attribution.contains("Zenodo 14692304"), "{}", ground.report.attribution);
        assert!(!ground.report.attribution.contains("VizieR"), "{}", ground.report.attribution);
        assert!(ground.report.attribution.contains("CC BY-NC 3.0 IGO"), "{}", ground.report.attribution);
    }

    #[test]
    fn general_range_gate_lets_a_pure_rotation_use_the_rotation_limit() {
        let rotation = rotation2(0.45f64.to_radians());
        assert!(frobenius(sub2(rotation, IDENTITY2)) > MAX_GENERAL_DEVIATION, "the plain Frobenius norm would already pass: {:.4}", frobenius(sub2(rotation, IDENTITY2)));
        assert!(general_deviation(rotation) < 1e-9, "a 0.45 deg rotation reads as a matrix deviation of {:.4}", general_deviation(rotation));
        let skew = [[1.0, 0.02], [0.0, 1.0]];
        assert!(general_deviation(skew) > MAX_GENERAL_DEVIATION, "a 0.02 skew must still be refused: {:.4}", general_deviation(skew));
        assert!((general_deviation(scaled2(IDENTITY2, 1.0 + 3e-5)) - 3e-5 * 2f64.sqrt()).abs() < 1e-9);
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, INJECTED_SHIFT_ARCSEC, 0.45, 1.0);
        let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noise_free(300));
        let result = tie_with(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions::default(), None, false);
        eprintln!("0.45 deg ground general: {:?} {:?}, matched {}, model {:?}, rotation {:.6} deg", result.decision, result.reason, result.report.n_matched, result.report.model, result.report.params.rotation_deg);
        assert_eq!(result.decision, TieDecision::Applied, "reason: {:?}", result.reason);
        assert_eq!(result.report.model.as_deref(), Some("general"), "{:?}", result.report.model_notes);
        assert!((result.report.params.rotation_deg - 0.45).abs() < 1e-6, "rotation {}", result.report.params.rotation_deg);
    }

    #[test]
    fn undated_no_significant_correction_states_the_epoch_term_next_to_the_bound() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let mut found = None;
        for seed in 0..6 {
            let synth = synthesize(&truth, FGS_SIZE, &SynthOptions { pm_sigma_masyr: 3.0, pm_mean_masyr: [1.819, 0.362], epoch_year: GAIA_REFERENCE_EPOCH_YEAR, ..SynthOptions::noisy(300, 15.0, 700 + seed) });
            let result = tie(&truth_header, &synth, Epoch::none(), &options(TieModel::Rshift), None);
            if result.decision == TieDecision::NoSignificantCorrection {
                found = Some(result);
                break;
            }
        }
        let result = found.expect("a zero-offset undated field returns no significant correction");
        let reason = result.reason.as_deref().unwrap();
        let term = result.report.epoch_systematic_mas.expect("epoch term");
        eprintln!("undated no-op: {reason} (epoch term {term:.1} mas)");
        assert!(term > 10.0, "the LMC bulk motion over 10 yr is about 18.5 mas, got {term}");
        assert!(reason.contains("epoch term") && reason.contains(&format!("{term:.1} mas")), "{reason}");
    }

    const J6_INJECTED_SHIFT_ARCSEC: [f64; 2] = [1.04, 1.44];
    const CLUSTER_HALF_WIDTH_ARCSEC: f64 = 0.04;
    const CHANCE_FWHM_PX: f64 = 1.6;
    const CHANCE_SNR: f64 = 40.0;
    const GAIA_MARGIN_PX: f64 = 88.0;
    const ROTATION_SIGMA: f64 = 3.0;
    const POOLED_FGS_ROTATION_DEG: f64 = 0.0127;
    const FEW_PAIRS: usize = 27;
    const FEW_PAIRS_NOISE_MAS: f64 = 25.0;

    fn offset_cluster(n: usize, offset: [f64; 2]) -> (Vec<Det>, Vec<GaiaStar>) {
        let mut dets = Vec::new();
        let mut stars = Vec::new();
        for i in 0..n {
            let base = [10.0 * i as f64, 10.0 * i as f64];
            dets.push(Det { x: 0.0, y: 0.0, plane0: base, sigma2: 1e-10, mag_inst: -10.0, fwhm_arcsec: 0.17 });
            stars.push(GaiaStar { index: i, v: [1.0, 0.0, 0.0], plane0: add_v(base, offset), sigma2: 1e-10, g: Some(15.0), bp_rp: None, pm: [0.0; 2], pm_known: true, parallax_known: false });
        }
        (dets, stars)
    }

    struct ChanceField {
        seed: u64,
        random_detections: usize,
        gaia_density: f64,
        true_pairs: usize,
        true_noise_mas: f64,
        cluster_pairs: usize,
        cluster_offset_arcsec: [f64; 2],
    }

    fn chance_row(id: String, ra: f64, dec: f64, g: f64) -> GaiaRow {
        GaiaRow {
            id,
            ra_deg: ra,
            dec_deg: dec,
            pm_ra_masyr: Some(0.0),
            pm_dec_masyr: Some(0.0),
            e_ra_mas: Some(0.02),
            e_dec_mas: Some(0.02),
            e_pm_ra_masyr: Some(0.02),
            e_pm_dec_masyr: Some(0.02),
            ruwe: Some(1.0),
            solved: Some(SOLVED_FIVE_PARAMETER),
            dup: Some(false),
            sepsi: Some(0.5),
            g: Some(g),
            bp_rp: Some(0.8),
            ..GaiaRow::default()
        }
    }

    fn chance_field(truth: &WcsTransform, input: &WcsTransform, spec: &ChanceField) -> Synthetic {
        let mut rng = StdRng::seed_from_u64(spec.seed);
        let scale = truth.pixel_scale_arcsec();
        let side = FGS_SIZE.0 as f64;
        let uniform = |rng: &mut StdRng, lo: f64, hi: f64| (lo + (hi - lo) * rng.gen::<f64>(), lo + (hi - lo) * rng.gen::<f64>());
        let detection = |x: f64, y: f64, mag_inst: f64| TieDetection { x, y, fwhm_px: CHANCE_FWHM_PX, snr: CHANCE_SNR, mag_inst, saturated: false };
        let mut rows = Vec::new();
        let mut detections = Vec::new();
        let n_rows = (spec.gaia_density * ((side + 2.0 * GAIA_MARGIN_PX) * scale).powi(2)).round() as usize;
        for i in 0..n_rows {
            let (x, y) = uniform(&mut rng, -GAIA_MARGIN_PX, side + GAIA_MARGIN_PX);
            let sky = truth.pixel_to_world(x, y);
            rows.push(chance_row(format!("R{i}"), sky.ra, sky.dec, 18.0 + 2.5 * rng.gen::<f64>()));
        }
        for i in 0..spec.true_pairs {
            let (x, y) = uniform(&mut rng, 40.0, side - 40.0);
            let sky = truth.pixel_to_world(x, y);
            let g = 18.0 + 2.5 * rng.gen::<f64>();
            rows.push(chance_row(format!("T{i}"), sky.ra, sky.dec, g));
            let noise_px = spec.true_noise_mas / MAS_PER_ARCSEC / scale;
            detections.push(detection(x + noise_px * gaussian(&mut rng), y + noise_px * gaussian(&mut rng), g - 25.0 + 0.05 * gaussian(&mut rng)));
        }
        for i in 0..spec.cluster_pairs {
            let (x, y) = uniform(&mut rng, 40.0, side - 40.0);
            let sky = input.pixel_to_world(x, y);
            let dx = spec.cluster_offset_arcsec[0] + CLUSTER_HALF_WIDTH_ARCSEC * (2.0 * rng.gen::<f64>() - 1.0);
            let dy = spec.cluster_offset_arcsec[1] + CLUSTER_HALF_WIDTH_ARCSEC * (2.0 * rng.gen::<f64>() - 1.0);
            let g = 18.0 + 2.5 * rng.gen::<f64>();
            rows.push(chance_row(format!("C{i}"), sky.ra + dx / ARCSEC_PER_DEG / sky.dec.to_radians().cos(), sky.dec + dy / ARCSEC_PER_DEG, g));
            detections.push(detection(x, y, g - 25.0 + 0.05 * gaussian(&mut rng)));
        }
        for _ in 0..spec.random_detections {
            let (x, y) = uniform(&mut rng, 40.0, side - 40.0);
            detections.push(detection(x, y, -7.0 + 2.5 * rng.gen::<f64>()));
        }
        Synthetic { detections, catalogue: GaiaCatalogue { kind: CatalogueKind::GaiaDr3Vizier, rows, quality_columns: true, cache_source: "test".into() }, pm: Vec::new() }
    }

    fn j6_input() -> (HduHeader, WcsTransform, WcsTransform, [f64; 2]) {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, J6_INJECTED_SHIFT_ARCSEC, 0.0, 1.0);
        let input = wcs_of(&input_header);
        let expected = truth_centre_shift_mas(&truth, &input, FGS_SIZE);
        (input_header, truth, input, expected)
    }

    fn three_sigma_t(dof: usize) -> f64 {
        let z = ROTATION_SIGMA;
        let nu = dof as f64;
        let g1 = (z.powi(3) + z) / 4.0;
        let g2 = (5.0 * z.powi(5) + 16.0 * z.powi(3) + 3.0 * z) / 96.0;
        let g3 = (3.0 * z.powi(7) + 19.0 * z.powi(5) + 17.0 * z.powi(3) - 15.0 * z) / 384.0;
        let g4 = (79.0 * z.powi(9) + 776.0 * z.powi(7) + 1482.0 * z.powi(5) - 1920.0 * z.powi(3) - 945.0 * z) / 92160.0;
        z + g1 / nu + g2 / nu.powi(2) + g3 / nu.powi(3) + g4 / nu.powi(4)
    }

    fn describe(result: &TieResult, expected: [f64; 2]) -> String {
        let r = &result.report;
        let (e_pre_fit, needed_pre_fit) = chance_excess(r.cuts.detections_used, r.match_radius_arcsec, r.density_per_arcsec2);
        let gate_b = match r.expected_chance_pairs {
            Some(e) => format!("{e:.1} expected by chance at the inlier radius, {:.1} needed, margin {:+.1}", chance_needed(e), r.n_used as f64 - chance_needed(e)),
            None => "gate B not reached".to_string(),
        };
        format!(
            "{:?} {:?}; peak {} vs threshold {:.2} at b {:.3}; sigma_res {:.1} mas, r_m {:.3} arcsec; matched {} (pre-fit at r_m: {e_pre_fit:.1} expected by chance, {needed_pre_fit:.1} needed), used {} ({gate_b}); shift error {:.1} mas",
            result.decision, result.reason, r.histogram_peak, r.histogram_threshold, r.histogram_random_per_bin, r.sigma_res_arcsec * MAS_PER_ARCSEC, r.match_radius_arcsec, r.n_matched, r.n_used, norm_v(sub_v(r.params.shift_mas, expected))
        )
    }

    #[test]
    fn j6_the_peak_threshold_counts_every_histogram_cell_searched() {
        let (dets, stars) = offset_cluster(12, [-3.31, -0.93]);
        let order = eta_sorted(&stars);
        let crf = find_offset_peak(&dets, &stars, &order, DEFAULT_SEARCH_RADIUS_ARCSEC, HISTOGRAM_BIN_MIN_ARCSEC, 1.6 / (12.0 * 0.04));
        eprintln!("J6 threshold, crf-like: peak {}, random {:.3} per bin, threshold {}", crf.peak, crf.random_per_bin, crf.threshold);
        assert_eq!(crf.peak, 12);
        assert!((crf.random_per_bin - 1.6).abs() < 1e-9, "random per bin {}", crf.random_per_bin);
        assert_eq!(crf.threshold, 13.0, "the maximum of 14884 Poisson(1.6) bins needs 13 to keep the chance of a false peak below 0.1 %");
        assert!((crf.peak as f64) < crf.threshold);
        let (dets, stars) = offset_cluster(26, [1.029, 1.439]);
        let order = eta_sorted(&stars);
        let j1 = find_offset_peak(&dets, &stars, &order, DEFAULT_SEARCH_RADIUS_ARCSEC, HISTOGRAM_BIN_MIN_ARCSEC, 2.626 / (26.0 * 0.04));
        eprintln!("J6 threshold, J1-like: peak {}, random {:.3} per bin, threshold {}", j1.peak, j1.random_per_bin, j1.threshold);
        assert_eq!(j1.peak, 26);
        assert_eq!(j1.threshold, 16.0);
        assert!(j1.peak as f64 >= j1.threshold);
    }

    #[test]
    fn j6_a_chance_cluster_at_a_wrong_offset_is_not_applied() {
        let (input_header, truth, input, expected) = j6_input();
        let spec = ChanceField { seed: 0xC21_0606, random_detections: 783, gaia_density: 0.05, true_pairs: 8, true_noise_mas: 100.0, cluster_pairs: 9, cluster_offset_arcsec: [-3.32, -0.94] };
        let synth = chance_field(&truth, &input, &spec);
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions { max_detections: 5000, ..options(TieModel::Shift) }, None);
        eprintln!("J6 crf analogue: {}", describe(&result, expected));
        let r = &result.report;
        assert!((1.3..2.0).contains(&r.histogram_random_per_bin), "random per bin {}", r.histogram_random_per_bin);
        assert_eq!(result.decision, TieDecision::Refused, "a 9-pair chance cluster was applied {:.0} mas from the truth", norm_v(sub_v(r.params.shift_mas, expected)));
        let reason = result.reason.as_deref().unwrap_or_default();
        assert!(reason.starts_with("no significant offset peak") || reason.starts_with("chance matches"), "{reason}");
    }

    #[test]
    fn j6_a_matched_set_dominated_by_chance_pairs_is_not_applied() {
        let (input_header, truth, input, expected) = j6_input();
        let spec = ChanceField { seed: 0xC21_0603, random_detections: 3000, gaia_density: 0.045, true_pairs: 30, true_noise_mas: 60.0, cluster_pairs: 0, cluster_offset_arcsec: [0.0; 2] };
        let synth = chance_field(&truth, &input, &spec);
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions { max_detections: 5000, ..options(TieModel::Shift) }, None);
        eprintln!("J6 depth 6000 analogue: {}", describe(&result, expected));
        let r = &result.report;
        assert!(r.histogram_peak >= 20, "peak {}", r.histogram_peak);
        assert_eq!(
            result.decision,
            TieDecision::Refused,
            "applied {:.0} mas from the truth with {} used of {} matched pairs against {:?} expected by chance",
            norm_v(sub_v(r.params.shift_mas, expected)), r.n_used, r.n_matched, r.expected_chance_pairs
        );
    }

    #[test]
    fn j6_a_false_peak_in_a_deep_detection_list_is_not_applied() {
        let (input_header, truth, input, expected) = j6_input();
        let spec = ChanceField { seed: 0xC21_0602, random_detections: 4170, gaia_density: 0.052, true_pairs: 8, true_noise_mas: 100.0, cluster_pairs: 22, cluster_offset_arcsec: [0.23, 0.07] };
        let synth = chance_field(&truth, &input, &spec);
        let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &TieOptions { max_detections: 5000, ..options(TieModel::Shift) }, None);
        eprintln!("J6 depth 12000 analogue: {}", describe(&result, expected));
        let r = &result.report;
        assert!((7.0..10.5).contains(&r.histogram_random_per_bin), "random per bin {}", r.histogram_random_per_bin);
        assert_eq!(result.decision, TieDecision::Refused, "a 22-pair chance cluster was applied {:.0} mas from the truth", norm_v(sub_v(r.params.shift_mas, expected)));
    }

    #[test]
    fn j6_a_true_tie_with_29_pairs_among_1200_detections_stays_applied() {
        let (input_header, truth, input, expected) = j6_input();
        let opts = TieOptions { max_detections: 2000, ..TieOptions::default() };
        let tolerance = 4.0 * 2f64.sqrt() * FEW_PAIRS_NOISE_MAS / (29f64).sqrt();
        let mut applied = 0;
        let mut refusals = Vec::new();
        for seed in 0..20u64 {
            let spec = ChanceField { seed: 0xC21_0604 + seed, random_detections: 1165, gaia_density: 0.04, true_pairs: 29, true_noise_mas: FEW_PAIRS_NOISE_MAS, cluster_pairs: 0, cluster_offset_arcsec: [0.0; 2] };
            let synth = chance_field(&truth, &input, &spec);
            let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &opts, None);
            let r = &result.report;
            eprintln!("J1 analogue seed {seed}: {}", describe(&result, expected));
            if result.decision != TieDecision::Applied {
                refusals.push(format!("seed {seed}: {:?}", result.reason));
                continue;
            }
            applied += 1;
            let error = norm_v(sub_v(r.params.shift_mas, expected));
            assert!(error <= tolerance, "seed {seed}: shift error {error:.1} mas exceeds {tolerance:.1} mas");
            assert!(r.expected_chance_pairs.is_some(), "seed {seed}: an applied tie reports no expected chance pairs");
        }
        eprintln!("J1 analogue: {applied} of 20 applied; refusals {refusals:?}");
        assert!(applied >= 19, "{applied} of 20 applied: {refusals:?}");
    }

    #[test]
    fn j6_the_gate_sweep_applies_no_tie_more_than_100_mas_from_the_truth() {
        let (input_header, truth, input, expected) = j6_input();
        let density = 0.025;
        let seeds = 20u64;
        let bin2 = HISTOGRAM_BIN_MIN_ARCSEC * HISTOGRAM_BIN_MIN_ARCSEC;
        let opts = TieOptions { max_detections: 5000, bootstrap_resamples: 20, ..TieOptions::default() };
        eprintln!("J6 sweep at {density} Gaia stars per square arcsec, {FEW_PAIRS_NOISE_MAS} mas noise, {seeds} seeds per cell: true pairs, b, applied, wrong (> 100 mas) among applied, refused by peak, refused by chance matches, mean peak, mean threshold, median and max shift error of the applied ties");
        for true_pairs in [10usize, 15, 20, 30] {
            for b in [0.2f64, 0.5, 1.0, 2.0, 3.0] {
                let total = (b / (density * bin2)).round() as usize;
                let random_detections = total.saturating_sub(true_pairs);
                let (mut wrong, mut by_peak, mut by_chance) = (0, 0, 0);
                let (mut peaks, mut thresholds, mut bs) = (0.0, 0.0, 0.0);
                let mut errors = Vec::new();
                for seed in 0..seeds {
                    let spec = ChanceField { seed: 0xC21_0700 + 1000 * true_pairs as u64 + (b * 100.0) as u64 + seed, random_detections, gaia_density: density, true_pairs, true_noise_mas: FEW_PAIRS_NOISE_MAS, cluster_pairs: 0, cluster_offset_arcsec: [0.0; 2] };
                    let synth = chance_field(&truth, &input, &spec);
                    let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &opts, None);
                    let r = &result.report;
                    peaks += r.histogram_peak as f64;
                    thresholds += r.histogram_threshold;
                    bs += r.histogram_random_per_bin;
                    match result.decision {
                        TieDecision::Applied => {
                            let error = norm_v(sub_v(r.params.shift_mas, expected));
                            errors.push(error);
                            wrong += usize::from(error > 100.0);
                        }
                        _ => {
                            let reason = result.reason.as_deref().unwrap_or_default();
                            by_peak += usize::from(reason.starts_with("no significant offset peak"));
                            by_chance += usize::from(reason.starts_with("chance matches"));
                        }
                    }
                }
                let n = seeds as f64;
                eprintln!(
                    "J6 sweep: true {true_pairs:2}, b {:.2} (target {b}), applied {:2}/{seeds}, wrong {wrong}, by peak {by_peak:2}, by chance {by_chance:2}, mean peak {:.1}, mean threshold {:.1}, shift error median {:.1} max {:.1} mas",
                    bs / n, errors.len(), peaks / n, thresholds / n, median_of(&errors), errors.iter().cloned().fold(0.0, f64::max)
                );
                assert_eq!(wrong, 0, "J6 sweep: {wrong} applied ties more than 100 mas from the truth at {true_pairs} true pairs and b {b}");
            }
        }
    }

    #[test]
    fn j3_the_auto_model_keeps_a_rotation_that_27_pairs_resolve() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let rotation = 0.05;
        let input_header = injected_input(&truth_header, J6_INJECTED_SHIFT_ARCSEC, rotation, 1.0);
        let opts = TieOptions { bootstrap_resamples: 100, ..TieOptions::default() };
        let sigma_rot = 0.0050;
        let mut rshift = 0;
        let mut refusals = Vec::new();
        for seed in 0..20u64 {
            let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noisy(FEW_PAIRS, FEW_PAIRS_NOISE_MAS, 2800 + seed));
            let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &opts, None);
            if result.decision == TieDecision::Refused {
                assert!(result.reason.as_deref().is_some_and(|m| m.starts_with("model inadequate")), "seed {seed}: {:?}", result.reason);
                refusals.push(seed);
                continue;
            }
            assert_eq!(result.decision, TieDecision::Applied, "seed {seed}: {:?}", result.reason);
            let r = &result.report;
            if r.model.as_deref() == Some("rshift") {
                rshift += 1;
                assert!((r.params.rotation_deg - rotation).abs() <= 4.0 * sigma_rot, "seed {seed}: rotation {:.5} deg vs {rotation} injected", r.params.rotation_deg);
            }
        }
        eprintln!("J3 guard: {rshift} of 20 kept rshift at an injected {rotation} deg, refusals {refusals:?}");
        assert!(refusals.len() <= 1, "{refusals:?}");
        assert!(rshift >= 19, "rshift kept on {rshift} of 20");
    }

    #[test]
    #[ignore]
    fn j3_report_only_the_pooled_fgs_rotation_keep_rate_under_candidate_rules() {
        let truth_header = fgs_header(false);
        let truth = wcs_of(&truth_header);
        let input_header = injected_input(&truth_header, J6_INJECTED_SHIFT_ARCSEC, POOLED_FGS_ROTATION_DEG, 1.0);
        let opts = TieOptions { bootstrap_resamples: 100, ..TieOptions::default() };
        let seeds = 40u64;
        let (mut applied, mut kept_wald, mut kept_max_sigma) = (0usize, 0usize, 0usize);
        let mut refusals = Vec::new();
        let (mut thetas, mut formal, mut boot) = (Vec::new(), Vec::new(), Vec::new());
        for seed in 0..seeds {
            let synth = synthesize(&truth, FGS_SIZE, &SynthOptions::noisy(FEW_PAIRS, FEW_PAIRS_NOISE_MAS, 2700 + seed));
            let result = tie(&input_header, &synth, Epoch::header(FGS_EPOCH), &opts, None);
            if result.decision != TieDecision::Applied {
                refusals.push(format!("seed {seed}: {:?}", result.reason));
                continue;
            }
            let r = &result.report;
            assert_eq!(r.model.as_deref(), Some("rshift"), "seed {seed}: {:?}", r.model_notes);
            let theta = r.params.rotation_deg;
            let sigma_formal = r.sigma_formal.rotation_deg;
            let sigma_boot = r.sigma_bootstrap.map_or(0.0, |b| b.rotation_deg);
            applied += 1;
            thetas.push(theta);
            formal.push(sigma_formal);
            boot.push(sigma_boot);
            kept_wald += usize::from(theta.abs() / sigma_formal >= three_sigma_t(2 * r.n_used - 3));
            kept_max_sigma += usize::from(theta.abs() / sigma_formal.max(sigma_boot) >= three_sigma_t(r.n_used - 3));
        }
        eprintln!(
            "J3 report only: injected {POOLED_FGS_ROTATION_DEG} deg (the pooled FGS value) on {FEW_PAIRS} pairs at {FEW_PAIRS_NOISE_MAS} mas; {applied} of {seeds} applied, refused {refusals:?}; fitted rotation mean {:.5} deg, sd {:.5}; mean sigma formal {:.5}, mean sigma bootstrap {:.5}; kept by the Wald t (sigma formal, nu 2N-3) {kept_wald}/{applied}; kept by max(sigma formal, sigma bootstrap) with nu N-3 {kept_max_sigma}/{applied}; today's automatic rshift keeps {applied}/{applied}",
            mean(&thetas), sd(&thetas), mean(&formal), mean(&boot)
        );
        assert!(refusals.iter().all(|r| r.contains("model inadequate")), "{refusals:?}");
    }

    #[test]
    fn j6_the_chance_gate_reproduces_the_logged_crf_and_fgs_numbers() {
        let (expected, needed) = chance_excess(793, 0.35589927486375583, 0.05076980474492112);
        assert!((expected - 15.860).abs() < 0.005, "crf expected {expected}");
        assert!((needed - 35.772).abs() < 0.005, "crf needed {needed}");
        let (expected, needed) = chance_excess(1194, 0.1865186558605832, 0.039951406753973163);
        assert!((expected - 5.202).abs() < 0.005, "J1 expected {expected}");
        assert!((needed - 16.606).abs() < 0.005, "J1 needed {needed}");
    }

    #[test]
    fn j3_three_sigma_t_matches_student_t_quantiles() {
        for (nu, expected) in [(12usize, 3.7643), (17, 3.5075), (24, 3.3448), (50, 3.1571), (51, 3.1539), (100, 3.0768), (1000, 3.0075)] {
            let got = three_sigma_t(nu);
            assert!((got - expected).abs() < 5e-4, "nu {nu}: {got} vs {expected}");
        }
    }
}
