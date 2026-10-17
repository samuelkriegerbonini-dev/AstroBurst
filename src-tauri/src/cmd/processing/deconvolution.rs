use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir};
use crate::cmd::processing::local_contrast::render_linear_output;
use crate::core::history::kernels::kernel_hash;
use crate::core::ops::psf::PsfSpec;
use crate::core::ops::{run_op, OpCompanions, OpInput, OpSpec, OpVersion};
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{
    EVENT_DECONV_PROGRESS, RES_CONVERGENCE, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_ITERATIONS_RUN,
    RES_PNG_PATH, RES_PSF_SOURCE, SUFFIX_DECONV,
};
use crate::types::stacking::RLConfig;

pub(crate) use crate::core::ops::psf::{estimated_kernel, DeconvPsf};

fn psf_spec(
    use_empirical_psf: Option<bool>,
    psf_kernel: Option<Vec<Vec<f32>>>,
    psf_size: usize,
    psf_sigma: f64,
    psf_num_stars: Option<usize>,
    psf_cutout_radius: Option<usize>,
) -> anyhow::Result<(PsfSpec, Option<Array2<f32>>)> {
    let chosen = DeconvPsf::choose(use_empirical_psf, psf_kernel.clone(), psf_size, psf_sigma, psf_num_stars, psf_cutout_radius)?;
    Ok(match chosen {
        DeconvPsf::Gaussian { .. } => (PsfSpec::Gaussian { sigma: psf_sigma, size: psf_size }, None),
        DeconvPsf::Estimated(config) => {
            (PsfSpec::Empirical { num_stars: config.num_stars, cutout_radius: config.cutout_radius }, None)
        }
        DeconvPsf::Provided(_) => {
            let rows = psf_kernel.unwrap_or_default();
            let size = rows.len();
            let raw = Array2::from_shape_fn((size, size), |(y, x)| rows[y][x]);
            (PsfSpec::Kernel { hash: kernel_hash(&raw), rows: size, cols: size }, Some(raw))
        }
    })
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
        let (psf, kernel) = psf_spec(use_empirical_psf, psf_kernel, psf_size, psf_sigma, psf_num_stars, psf_cutout_radius)?;

        let entry = load_from_cache_or_disk(&path)?;

        let config = RLConfig {
            iterations,
            regularization,
            deringing,
            deringing_threshold: dering_threshold as f32,
        };

        let spec = OpSpec::RlDeconv { v: OpVersion, config, psf };
        let companions = OpCompanions { psf_kernel: kernel.as_ref(), ..OpCompanions::default() };
        let t0 = std::time::Instant::now();
        let deconvolved = run_op(&spec, &OpInput::mono(entry.arr(), None).with_companions(companions), Some(&progress_clone))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_linear_output(&deconvolved.planes[0], &path, &entry, &output_dir, SUFFIX_DECONV)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_ITERATIONS_RUN: deconvolved.extras[RES_ITERATIONS_RUN],
            RES_CONVERGENCE: deconvolved.extras[RES_CONVERGENCE],
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
            RES_PSF_SOURCE: deconvolved.extras[RES_PSF_SOURCE],
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use crate::core::analysis::deconvolution::{generate_gaussian_psf, provided_psf_kernel};
    use crate::core::imaging::psf_estimation::PsfEstimationConfig;

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
    fn psf_spec_records_the_choice_and_supplies_the_raw_kernel() {
        let (spec, kernel) = psf_spec(Some(false), Some(vec![vec![f32::NAN; 4]; 4]), 7, 1.5, None, None).unwrap();
        assert!(matches!(spec, PsfSpec::Gaussian { size: 7, sigma } if sigma == 1.5), "{spec:?}");
        assert!(kernel.is_none());

        let (spec, kernel) = psf_spec(Some(true), None, 15, 2.0, None, Some(9)).unwrap();
        assert!(matches!(spec, PsfSpec::Empirical { num_stars: 30, cutout_radius: 9 }), "{spec:?}");
        assert!(kernel.is_none());

        let (spec, kernel) = psf_spec(Some(true), Some(pyramid_rows()), 15, 2.0, None, None).unwrap();
        let raw = kernel.expect("a provided kernel is supplied with the op");
        assert_eq!(raw, Array2::from_shape_fn((3, 3), |(y, x)| pyramid_rows()[y][x]), "the kernel was normalised twice");
        let PsfSpec::Kernel { hash, rows: 3, cols: 3 } = spec else { panic!("{spec:?}") };
        assert_eq!(hash, kernel_hash(&raw));

        let err = psf_spec(Some(true), Some(vec![vec![1.0; 4]; 4]), 15, 2.0, None, None).unwrap_err().to_string();
        assert!(err.contains("odd size"), "{err}");
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
