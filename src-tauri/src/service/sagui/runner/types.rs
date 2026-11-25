use std::collections::BTreeMap;
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use crate::core::cubeprep::align::NOISCORR_SOURCE_MATCH;
use crate::core::cubeprep::cube::{Cube, CubePlane};

pub(super) const SED_SCALINGS: [&str; 2] = ["median_centre", "median_mad"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaguiConfig {
    pub input_path: String,
    pub output_dir: String,
    pub rscript_path: String,
    pub library_path: String,
    pub n_regions: usize,
    pub starlet: bool,
    pub starlet_scales: Vec<usize>,
    pub starlet_threshold: f64,
    pub dq_exclude_bits: u32,
    pub psf_target: String,
    pub mode: String,
    pub knn_k: usize,
    #[serde(default = "default_sed_scaling")]
    pub sed_scaling: String,
}

fn default_sed_scaling() -> String { SED_SCALINGS[0].into() }

#[derive(Debug, Clone, Serialize)]
pub struct Band {
    pub filter: String,
    pub pivot_um: f64,
    pub psf_fwhm_arcsec: Option<f64>,
    pub pixscale_arcsec: Option<f64>,
    pub psf_area_px: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaneDqFlags {
    pub filter: String,
    pub table: &'static str,
    pub flagged_finite: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct InputInfo {
    pub width: usize,
    pub height: usize,
    pub bands: Vec<Band>,
    pub unit: String,
    pub has_err: bool,
    pub has_dq: bool,
    pub has_noise_correlation: bool,
    pub dq_flags: Vec<PlaneDqFlags>,
    pub estimated_memory_bytes: u64,
    pub warnings: Vec<String>,
    pub redshift: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionalBand {
    pub filter: String,
    pub pivot_um: f64,
    pub flux: f64,
    pub error: Option<f64>,
    #[serde(default)]
    pub error_correlated: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub region: u32,
    pub n_pixels: usize,
    #[serde(default)]
    pub n_components: usize,
    #[serde(default)]
    pub min_snr: Option<f64>,
    #[serde(default)]
    pub min_snr_lower_bound: Option<f64>,
    pub bands: Vec<RegionalBand>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaguiResult {
    pub run_id: String,
    pub output_dir: String,
    pub labels_path: String,
    pub mask_path: String,
    pub table_path: String,
    pub manifest_path: String,
    pub width: usize,
    pub height: usize,
    pub unit: String,
    #[serde(default, skip_serializing)]
    pub labels: Vec<u32>,
    #[serde(default)]
    pub labels_bin_path: String,
    pub regions: Vec<Region>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NoiseCorrelation {
    pub half: usize,
    pub source: String,
    pub planes: Vec<Array2<f32>>,
}

impl NoiseCorrelation {
    pub(in crate::service::sagui) fn covers(&self, plane: &CubePlane) -> bool {
        self.source == NOISCORR_SOURCE_MATCH || plane.kernel.trim().is_empty()
    }

    pub(in crate::service::sagui) fn withheld(&self, cube: &Cube) -> Vec<String> {
        cube.filters.iter().filter(|plane| !self.covers(plane)).map(|plane| plane.filter.clone()).collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct MapCounts {
    pub pixels: usize,
    pub eligible_pixels: usize,
    pub excluded_by_policy: usize,
    pub support_pixels: usize,
    pub labelled_pixels: usize,
    pub support_unlabelled: usize,
    pub support_on_excluded: usize,
}
