use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::analysis::deconvolution::{generate_gaussian_psf, provided_psf_kernel};
use crate::core::imaging::psf_estimation::{estimate_psf, psf_to_kernel, PsfEstimationConfig};
use crate::types::constants::{PSF_SOURCE_ESTIMATED, PSF_SOURCE_GAUSSIAN, PSF_SOURCE_PROVIDED};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PsfSpec {
    Gaussian { sigma: f64, size: usize },
    Empirical { num_stars: usize, cutout_radius: usize },
    Kernel { hash: String, rows: usize, cols: usize },
}

pub fn psf_config(
    num_stars: Option<usize>,
    cutout_radius: Option<usize>,
    saturation_threshold: Option<f64>,
    max_ellipticity: Option<f64>,
) -> PsfEstimationConfig {
    PsfEstimationConfig {
        num_stars: num_stars.unwrap_or(30),
        cutout_radius: cutout_radius.unwrap_or(15),
        saturation_threshold: saturation_threshold.unwrap_or(0.95),
        min_peak_fraction: 0.10,
        max_ellipticity: max_ellipticity.unwrap_or(0.3),
        edge_margin: 30,
        max_center_distance_fraction: 0.7,
    }
}

pub enum DeconvPsf {
    Gaussian { size: usize, sigma: f32 },
    Estimated(PsfEstimationConfig),
    Provided(Array2<f32>),
}

impl DeconvPsf {
    pub fn choose(
        use_empirical_psf: Option<bool>,
        psf_kernel: Option<Vec<Vec<f32>>>,
        psf_size: usize,
        psf_sigma: f64,
        psf_num_stars: Option<usize>,
        psf_cutout_radius: Option<usize>,
    ) -> anyhow::Result<Self> {
        if !use_empirical_psf.unwrap_or(false) {
            return Ok(Self::Gaussian { size: psf_size, sigma: psf_sigma as f32 });
        }
        match psf_kernel {
            Some(rows) => Ok(Self::Provided(provided_psf_kernel(&rows)?)),
            None => Ok(Self::Estimated(psf_config(psf_num_stars, psf_cutout_radius, None, None))),
        }
    }

    pub fn from_spec(spec: &PsfSpec, supplied: Option<&Array2<f32>>) -> anyhow::Result<Self> {
        match spec {
            PsfSpec::Gaussian { sigma, size } => Self::choose(Some(false), None, *size, *sigma, None, None),
            PsfSpec::Empirical { num_stars, cutout_radius } => {
                Self::choose(Some(true), None, 0, 0.0, Some(*num_stars), Some(*cutout_radius))
            }
            PsfSpec::Kernel { hash, rows, cols } => {
                let kernel = supplied.ok_or_else(|| anyhow::anyhow!("the PSF kernel {hash} was not supplied with the input"))?;
                if kernel.dim() != (*rows, *cols) {
                    anyhow::bail!(
                        "the PSF kernel {hash} is {}x{} but the op expects {rows}x{cols}",
                        kernel.nrows(),
                        kernel.ncols()
                    );
                }
                let rows: Vec<Vec<f32>> = kernel.rows().into_iter().map(|r| r.to_vec()).collect();
                Self::choose(Some(true), Some(rows), 0, 0.0, None, None)
            }
        }
    }

    pub fn source(&self) -> &'static str {
        match self {
            Self::Gaussian { .. } => PSF_SOURCE_GAUSSIAN,
            Self::Estimated(_) => PSF_SOURCE_ESTIMATED,
            Self::Provided(_) => PSF_SOURCE_PROVIDED,
        }
    }

    pub fn kernel(
        &self,
        estimate: impl FnOnce(&PsfEstimationConfig) -> anyhow::Result<Array2<f32>>,
    ) -> anyhow::Result<Array2<f32>> {
        match self {
            Self::Gaussian { size, sigma } => Ok(generate_gaussian_psf(*size, *sigma)),
            Self::Estimated(config) => estimate(config),
            Self::Provided(kernel) => Ok(kernel.clone()),
        }
    }
}

pub fn estimated_kernel(image: &Array2<f32>, config: &PsfEstimationConfig) -> anyhow::Result<Array2<f32>> {
    let result = estimate_psf(image, config).map_err(|e| anyhow::anyhow!("PSF estimation failed: {}", e))?;
    Ok(psf_to_kernel(&result))
}
