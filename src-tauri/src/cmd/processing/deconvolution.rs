use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir};
use crate::cmd::processing::local_contrast::render_linear_output;
use crate::cmd::psf::psf_config;
use crate::core::analysis::deconvolution::{generate_gaussian_psf, provided_psf_kernel, richardson_lucy};
use crate::core::imaging::psf_estimation::{estimate_psf, psf_to_kernel, PsfEstimationConfig};
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_DECONV_PROGRESS, PSF_SOURCE_ESTIMATED, PSF_SOURCE_GAUSSIAN, PSF_SOURCE_PROVIDED,
    RES_CONVERGENCE, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_ITERATIONS_RUN,
    RES_PNG_PATH, RES_PSF_SOURCE, SUFFIX_DECONV,
};
use crate::types::stacking::RLConfig;

pub(crate) enum DeconvPsf {
    Gaussian { size: usize, sigma: f32 },
    Estimated(PsfEstimationConfig),
    Provided(Array2<f32>),
}

impl DeconvPsf {
    pub(crate) fn choose(
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

    pub(crate) fn source(&self) -> &'static str {
        match self {
            Self::Gaussian { .. } => PSF_SOURCE_GAUSSIAN,
            Self::Estimated(_) => PSF_SOURCE_ESTIMATED,
            Self::Provided(_) => PSF_SOURCE_PROVIDED,
        }
    }

    pub(crate) fn kernel(
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

pub(crate) fn estimated_kernel(image: &Array2<f32>, config: &PsfEstimationConfig) -> anyhow::Result<Array2<f32>> {
    let result = estimate_psf(image, config).map_err(|e| anyhow::anyhow!("PSF estimation failed: {}", e))?;
    Ok(psf_to_kernel(&result))
}

#[tauri::command]
pub async fn deconvolve_rl_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    iterations: usize,
    psf_sigma: f64,
    psf_size: usize,
    regularization: f64,
    deringing: bool,
    dering_threshold: f64,
    use_empirical_psf: Option<bool>,
    psf_num_stars: Option<usize>,
    psf_cutout_radius: Option<usize>,
    psf_kernel: Option<Vec<Vec<f32>>>,
) -> Result<serde_json::Value, String> {
    let progress_clone = ProgressHandle::new(&app, EVENT_DECONV_PROGRESS, iterations as u64).clone();

    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        let psf = DeconvPsf::choose(use_empirical_psf, psf_kernel, psf_size, psf_sigma, psf_num_stars, psf_cutout_radius)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();
        let kernel = psf.kernel(|config| estimated_kernel(image, config))?;

        let rl_config = RLConfig {
            iterations,
            regularization,
            deringing,
            deringing_threshold: dering_threshold as f32,
            ..RLConfig::default()
        };

        let rl_result = richardson_lucy(image, &kernel, &rl_config, Some(&progress_clone))?;

        let ro = render_linear_output(&rl_result.image, &path, &entry, &output_dir, SUFFIX_DECONV)?;
        let (rows, cols) = ro.dims;

        progress_clone.emit_complete();

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_ITERATIONS_RUN: rl_result.iterations_run,
            RES_CONVERGENCE: rl_result.convergence,
            RES_ELAPSED_MS: rl_result.elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
            RES_PSF_SOURCE: psf.source(),
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pyramid_rows() -> Vec<Vec<f32>> {
        vec![vec![1.0, 2.0, 1.0], vec![2.0, 4.0, 2.0], vec![1.0, 2.0, 1.0]]
    }

    fn no_estimate(_: &PsfEstimationConfig) -> anyhow::Result<Array2<f32>> {
        panic!("the PSF must not be estimated when a kernel is provided or a Gaussian is requested")
    }

    #[test]
    fn empirical_psf_with_a_kernel_uses_the_provided_kernel_without_estimating() {
        let psf = DeconvPsf::choose(Some(true), Some(pyramid_rows()), 15, 2.0, Some(10), Some(7)).unwrap();
        assert_eq!(psf.source(), "provided");
        let kernel = psf.kernel(no_estimate).unwrap();
        assert_eq!(kernel, provided_psf_kernel(&pyramid_rows()).unwrap());
        assert!((kernel.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empirical_psf_without_a_kernel_estimates_with_the_requested_star_settings() {
        let psf = DeconvPsf::choose(Some(true), None, 15, 2.0, Some(10), Some(7)).unwrap();
        assert_eq!(psf.source(), "estimated");
        let DeconvPsf::Estimated(config) = &psf else { panic!("expected an estimated PSF") };
        assert_eq!((config.num_stars, config.cutout_radius), (10, 7));
        assert_eq!(config.saturation_threshold, PsfEstimationConfig::default().saturation_threshold);
        let seen = std::cell::Cell::new(false);
        let kernel = psf
            .kernel(|c| {
                seen.set(true);
                Ok(generate_gaussian_psf(c.cutout_radius * 2 + 1, 1.0))
            })
            .unwrap();
        assert!(seen.get(), "the estimator must run when no kernel is provided");
        assert_eq!(kernel.dim(), (15, 15));
    }

    #[test]
    fn a_gaussian_psf_ignores_a_provided_kernel_even_an_invalid_one() {
        for use_empirical in [Some(false), None] {
            let psf = DeconvPsf::choose(use_empirical, Some(vec![vec![f32::NAN; 4]; 4]), 7, 1.5, None, None).unwrap();
            assert_eq!(psf.source(), "gaussian");
            assert_eq!(psf.kernel(no_estimate).unwrap(), generate_gaussian_psf(7, 1.5));
        }
    }

    #[test]
    fn an_invalid_provided_kernel_is_refused_before_anything_runs() {
        let err = DeconvPsf::choose(Some(true), Some(vec![vec![1.0; 4]; 4]), 15, 2.0, None, None)
            .err()
            .expect("an even kernel must be refused")
            .to_string();
        assert!(err.contains("odd size"), "{err}");
        let err = DeconvPsf::choose(Some(true), Some(vec![vec![0.0; 3]; 3]), 15, 2.0, None, None)
            .err()
            .expect("a zero kernel must be refused")
            .to_string();
        assert!(err.contains("sums to 0"), "{err}");
    }
}
