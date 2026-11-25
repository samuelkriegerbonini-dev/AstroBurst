use std::time::Duration;

use crate::core::alignment::triangles::TriangleParams;
use crate::core::analysis::star_detection::DetectedStar;
use crate::core::registration::poly::TanParams;

use super::constants::COVERAGE_MIN_LOG10;

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

pub(in crate::core::astrometry::local_solve) fn cell_covered(coverage_log10: f64, chance_log10: f64) -> bool {
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
    pub sip_max_order: Option<u8>,
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
            sip_max_order: None,
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
