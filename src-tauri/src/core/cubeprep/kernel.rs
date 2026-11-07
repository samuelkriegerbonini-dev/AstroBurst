use std::collections::{HashMap, HashSet};

use ndarray::{s, Array2};
use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::core::cubeprep::cube::set_text;
use crate::core::cubeprep::psf_io::{fit_gaussian, GaussianFit};
use crate::core::imaging::convolve::{convolve_fft, ConvolveError, ConvolveOptions, FftPlanCache, NanTreatment};
use crate::core::imaging::push_history;
use crate::math::fft::FftEngine2D;
use crate::math::window::TaperWindow;
use crate::types::header::HduHeader;

pub const DEFAULT_EPS: f64 = 1e-3;
pub const DEFAULT_REGULARIZATION: f64 = 1e-4;
pub const NORMALIZATION_ATOL: f64 = 1e-8;
pub const AUTO_TOLERANCE: f64 = 0.01;
pub const RESIDUAL_WARNING: f64 = 0.01;
pub const FWHM_WARNING: f64 = 0.02;
pub const NARROWER_TOLERANCE: f64 = 0.02;
pub const AUTO_EPS: [f64; 17] = [1e-1, 3e-2, 1e-2, 3e-3, 1e-3, 3e-4, 1e-4, 3e-5, 1e-5, 3e-6, 1e-6, 3e-7, 1e-7, 3e-8, 1e-8, 3e-9, 1e-9];
const AUTO_WINDOW_STEPS: usize = 20;
const EE_STEP_PX: f64 = 0.5;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum KernelError {
    #[error("the {0} PSF is empty")]
    Empty(&'static str),
    #[error("the {which} PSF is {cols}x{rows}; matching kernels need odd sizes so the PSF has a central pixel")]
    EvenSize { which: &'static str, rows: usize, cols: usize },
    #[error("the {which} PSF has {count} non-finite pixels")]
    NonFinite { which: &'static str, count: usize },
    #[error("the {which} PSF sums to {sum}, so it cannot be normalized")]
    ZeroSum { which: &'static str, sum: f64 },
    #[error("the window recipe needs PSFs of one shape; the source is {source_cols}x{source_rows} and the target {target_cols}x{target_rows}")]
    ShapeMismatch { source_rows: usize, source_cols: usize, target_rows: usize, target_cols: usize },
    #[error("the kernel shape {cols}x{rows} must be odd and hold both PSFs ({needed_cols}x{needed_rows})")]
    KernelShape { rows: usize, cols: usize, needed_rows: usize, needed_cols: usize },
    #[error("eps must be a positive number, got {0}")]
    InvalidEps(f64),
    #[error("the regularization must lie in [0, 1), got {0}")]
    InvalidRegularization(f64),
    #[error("the {window} window needs alpha and beta in [0, 1], got alpha {alpha} and beta {beta}")]
    InvalidWindow { window: &'static str, alpha: f64, beta: f64 },
    #[error("the target PSF (FWHM {target_fwhm_px:.4} px) is narrower than the source PSF (FWHM {source_fwhm_px:.4} px); a matching kernel can only broaden, so match every PSF to the broadest one")]
    TargetNarrower { source_fwhm_px: f64, target_fwhm_px: f64 },
    #[error("the kernel sums to {0}, so it cannot be normalized")]
    NotNormalizable(f64),
    #[error("the {0} window has no parameter to choose automatically")]
    NothingToTune(&'static str),
    #[error(transparent)]
    Convolve(#[from] ConvolveError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpsMode {
    #[default]
    Absolute,
    Relative,
}

impl EpsMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Absolute => "absolute",
            Self::Relative => "relative",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeName {
    Reference,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KernelShape {
    Named(ShapeName),
    Fixed([usize; 2]),
}

impl Default for KernelShape {
    fn default() -> Self {
        Self::Named(ShapeName::Reference)
    }
}

fn default_eps() -> f64 {
    DEFAULT_EPS
}

fn default_regularization() -> f64 {
    DEFAULT_REGULARIZATION
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum KernelMethod {
    Wiener {
        #[serde(default = "default_eps")]
        eps: f64,
        #[serde(default)]
        eps_mode: EpsMode,
        #[serde(default)]
        shape: KernelShape,
    },
    Window {
        #[serde(flatten)]
        window: TaperWindow,
        #[serde(default = "default_regularization")]
        regularization: f64,
    },
}

impl Default for KernelMethod {
    fn default() -> Self {
        Self::Wiener { eps: DEFAULT_EPS, eps_mode: EpsMode::Absolute, shape: KernelShape::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct KernelOptions {
    #[serde(flatten)]
    pub method: KernelMethod,
    #[serde(default)]
    pub auto: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AutoCandidate {
    pub value: f64,
    pub residual_max: f64,
    pub residual_rms: f64,
    pub negative_weight: f64,
    pub sum_squares: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AutoChoice {
    pub parameter: &'static str,
    pub chosen: f64,
    pub candidates: Vec<AutoCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KernelDiagnostics {
    pub rows: usize,
    pub cols: usize,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub negative_fraction: f64,
    pub negative_weight: f64,
    pub sum_squares: f64,
    pub residual_max: f64,
    pub residual_rms: f64,
    pub encircled_energy_difference: f64,
    pub source_fwhm_px: Option<f64>,
    pub target_fwhm_px: Option<f64>,
    pub matched_fwhm_px: Option<f64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MatchingKernel {
    pub data: Array2<f64>,
    pub method: KernelMethod,
    pub eps_absolute: Option<f64>,
    pub auto: Option<AutoChoice>,
    pub diagnostics: KernelDiagnostics,
}

fn checked_psf(which: &'static str, psf: &Array2<f64>) -> Result<f64, KernelError> {
    let (rows, cols) = psf.dim();
    if psf.is_empty() {
        return Err(KernelError::Empty(which));
    }
    if rows % 2 == 0 || cols % 2 == 0 {
        return Err(KernelError::EvenSize { which, rows, cols });
    }
    let count = psf.iter().filter(|v| !v.is_finite()).count();
    if count > 0 {
        return Err(KernelError::NonFinite { which, count });
    }
    let sum = psf.sum();
    if sum.abs() <= NORMALIZATION_ATOL {
        return Err(KernelError::ZeroSum { which, sum });
    }
    Ok(sum)
}

pub(crate) fn validate_window_options(window: TaperWindow, regularization: f64) -> Result<(), KernelError> {
    if !(0.0..1.0).contains(&regularization) {
        return Err(KernelError::InvalidRegularization(regularization));
    }
    check_window(window)
}

fn check_window(window: TaperWindow) -> Result<(), KernelError> {
    let (alpha, beta) = window.alpha_beta();
    let unit = |value: f64| (0.0..=1.0).contains(&value);
    if unit(alpha) && unit(beta) {
        Ok(())
    } else {
        Err(KernelError::InvalidWindow { window: window.name(), alpha, beta })
    }
}

fn corner_index(index: usize, centre: usize, len: usize) -> usize {
    (index + len - centre % len) % len
}

fn spectrum(psf: &Array2<f64>, engine: &FftEngine2D<f64>, centred: bool) -> Vec<Complex<f64>> {
    let (rows, cols) = (engine.fft_rows, engine.fft_cols);
    let (centre_row, centre_col) = if centred { (psf.nrows() / 2, psf.ncols() / 2) } else { (0, 0) };
    let mut buffer = engine.alloc_buffer();
    for ((r, c), &value) in psf.indexed_iter() {
        buffer[corner_index(r, centre_row, rows) * cols + corner_index(c, centre_col, cols)].re = value;
    }
    engine.forward_2d(&mut buffer);
    buffer
}

fn centred_real(buffer: &[Complex<f64>], (rows, cols): (usize, usize)) -> Array2<f64> {
    Array2::from_shape_fn((rows, cols), |(r, c)| buffer[corner_index(r, rows / 2, rows) * cols + corner_index(c, cols / 2, cols)].re)
}

fn unit_sum(kernel: Array2<f64>) -> Result<Array2<f64>, KernelError> {
    let sum = kernel.sum();
    if !sum.is_finite() || sum.abs() <= NORMALIZATION_ATOL {
        return Err(KernelError::NotNormalizable(sum));
    }
    Ok(kernel / sum)
}

pub fn wiener(source: &Array2<f64>, target: &Array2<f64>, shape: (usize, usize), eps: f64) -> Result<Array2<f64>, KernelError> {
    wiener_with(source, target, shape, eps, EpsMode::Absolute).map(|(kernel, _)| kernel)
}

pub fn wiener_with(
    source: &Array2<f64>,
    target: &Array2<f64>,
    shape: (usize, usize),
    eps: f64,
    mode: EpsMode,
) -> Result<(Array2<f64>, f64), KernelError> {
    checked_psf("source", source)?;
    checked_psf("target", target)?;
    let (rows, cols) = shape;
    let (needed_rows, needed_cols) = (source.nrows().max(target.nrows()), source.ncols().max(target.ncols()));
    if rows % 2 == 0 || cols % 2 == 0 || rows < needed_rows || cols < needed_cols {
        return Err(KernelError::KernelShape { rows, cols, needed_rows, needed_cols });
    }
    if !(eps.is_finite() && eps > 0.0) {
        return Err(KernelError::InvalidEps(eps));
    }
    let engine = FftEngine2D::<f64>::new(rows, cols);
    let source_otf = spectrum(source, &engine, true);
    let target_otf = spectrum(target, &engine, true);
    let eps_absolute = match mode {
        EpsMode::Absolute => eps,
        EpsMode::Relative => eps * source_otf.iter().map(|v| v.norm_sqr()).fold(0.0, f64::max),
    };
    let mut ratio: Vec<Complex<f64>> =
        source_otf.iter().zip(&target_otf).map(|(s, t)| t * s.conj() / (s.norm_sqr() + eps_absolute)).collect();
    engine.inverse_2d(&mut ratio);
    Ok((unit_sum(centred_real(&ratio, shape))?, eps_absolute))
}

pub fn ratio_window(source: &Array2<f64>, target: &Array2<f64>, window: TaperWindow, regularization: f64) -> Result<Array2<f64>, KernelError> {
    let source_sum = checked_psf("source", source)?;
    let target_sum = checked_psf("target", target)?;
    if source.dim() != target.dim() {
        let ((source_rows, source_cols), (target_rows, target_cols)) = (source.dim(), target.dim());
        return Err(KernelError::ShapeMismatch { source_rows, source_cols, target_rows, target_cols });
    }
    validate_window_options(window, regularization)?;
    let (rows, cols) = source.dim();
    let engine = FftEngine2D::<f64>::new(rows, cols);
    let source_otf = spectrum(&(source / source_sum), &engine, false);
    let target_otf = spectrum(&(target / target_sum), &engine, false);
    let threshold = regularization * source_otf.iter().map(|v| v.norm()).fold(0.0, f64::max);
    let taper = window.values((rows, cols));
    let mut ratio: Vec<Complex<f64>> = source_otf
        .iter()
        .zip(&target_otf)
        .enumerate()
        .map(|(k, (s, t))| {
            if s.norm() > threshold {
                t / s * taper[[(k / cols + rows / 2) % rows, (k % cols + cols / 2) % cols]]
            } else {
                Complex::new(0.0, 0.0)
            }
        })
        .collect();
    engine.inverse_2d(&mut ratio);
    unit_sum(centred_real(&ratio, (rows, cols)))
}

fn full_convolution(image: &Array2<f64>, kernel: &Array2<f64>) -> Result<Array2<f64>, KernelError> {
    let (kernel_rows, kernel_cols) = kernel.dim();
    let mut padded = Array2::zeros((image.nrows() + kernel_rows - 1, image.ncols() + kernel_cols - 1));
    padded
        .slice_mut(s![kernel_rows / 2..kernel_rows / 2 + image.nrows(), kernel_cols / 2..kernel_cols / 2 + image.ncols()])
        .assign(image);
    let options = ConvolveOptions { nan_treatment: NanTreatment::Fill, normalize_kernel: false, ..ConvolveOptions::default() };
    Ok(convolve_fft(&padded, kernel, &options, &mut FftPlanCache::new())?.image)
}

fn centred_on(array: &Array2<f64>, shape: (usize, usize)) -> Array2<f64> {
    let mut canvas = Array2::zeros(shape);
    let (r0, c0) = ((shape.0 - array.nrows()) / 2, (shape.1 - array.ncols()) / 2);
    canvas.slice_mut(s![r0..r0 + array.nrows(), c0..c0 + array.ncols()]).assign(array);
    canvas
}

struct MatchedPsf {
    matched: Array2<f64>,
    target: Array2<f64>,
    residual_max: f64,
    residual_rms: f64,
}

fn matched_psf(source: &Array2<f64>, target: &Array2<f64>, kernel: &Array2<f64>) -> Result<MatchedPsf, KernelError> {
    let full = full_convolution(source, kernel)?;
    let shape = (full.nrows().max(target.nrows()), full.ncols().max(target.ncols()));
    let (matched, target) = (centred_on(&full, shape), centred_on(target, shape));
    let peak = target.iter().fold(0.0f64, |acc, &v| acc.max(v));
    let (mut worst, mut squares) = (0.0f64, 0.0);
    for (m, t) in matched.iter().zip(target.iter()) {
        let difference = m - t;
        worst = worst.max(difference.abs());
        squares += difference * difference;
    }
    let rms = (squares / matched.len() as f64).sqrt();
    Ok(MatchedPsf { matched, target, residual_max: worst / peak, residual_rms: rms / peak })
}

fn encircled_energy_difference(matched: &Array2<f64>, target: &Array2<f64>, target_dim: (usize, usize), target_fwhm_px: Option<f64>) -> f64 {
    let (rows, cols) = matched.dim();
    let (cy, cx) = ((rows / 2) as f64, (cols / 2) as f64);
    let max_radius = (target_dim.0.min(target_dim.1) / 2) as f64;
    let first = target_fwhm_px.filter(|fwhm| fwhm.is_finite() && *fwhm > 0.0).unwrap_or(1.0).min(max_radius);
    let (matched_total, target_total) = (matched.sum(), target.sum());
    (0..)
        .map(|k| first + EE_STEP_PX * k as f64)
        .take_while(|&radius| radius <= max_radius)
        .map(|radius| {
            let (mut inside_matched, mut inside_target) = (0.0, 0.0);
            for ((y, x), &value) in matched.indexed_iter() {
                if (x as f64 - cx).hypot(y as f64 - cy) <= radius {
                    inside_matched += value;
                    inside_target += target[[y, x]];
                }
            }
            (inside_matched / matched_total - inside_target / target_total).abs()
        })
        .fold(0.0, f64::max)
}

fn fwhm_text(fwhm: Option<f64>) -> String {
    fwhm.map_or_else(|| "unknown".to_string(), |value| format!("{value:.3}"))
}

fn negative_weight_and_sum_squares(kernel: &Array2<f64>) -> (f64, f64) {
    kernel.iter().fold((0.0, 0.0), |(negative, squares), &v| (if v < 0.0 { negative - v } else { negative }, squares + v * v))
}

pub fn diagnose_kernel(source: &Array2<f64>, target: &Array2<f64>, kernel: &Array2<f64>) -> Result<KernelDiagnostics, KernelError> {
    let MatchedPsf { matched, target: target_canvas, residual_max, residual_rms } = matched_psf(source, target, kernel)?;
    let (source_fit, target_fit, matched_fit) = (fit_gaussian(source), fit_gaussian(target), fit_gaussian(&matched));
    let fwhm = |fit: &Option<GaussianFit>| fit.as_ref().map(|fit| fit.fwhm_px);
    let (source_fwhm_px, target_fwhm_px, matched_fwhm_px) = (fwhm(&source_fit), fwhm(&target_fit), fwhm(&matched_fit));
    let encircled_energy_difference = encircled_energy_difference(&matched, &target_canvas, target.dim(), target_fwhm_px);
    let mut warnings = Vec::new();
    if residual_max > RESIDUAL_WARNING {
        warnings.push(format!(
            "the source PSF convolved with the kernel differs from the target by up to {:.2} percent of its peak (RMS {:.3} percent)",
            100.0 * residual_max,
            100.0 * residual_rms
        ));
    }
    if let (Some(matched), Some(target)) = (matched_fwhm_px, target_fwhm_px) {
        if (matched / target - 1.0).abs() > FWHM_WARNING {
            warnings.push(format!("the matched FWHM {matched:.3} px differs from the target FWHM {target:.3} px by {:+.2} percent", 100.0 * (matched / target - 1.0)));
        }
    }
    if encircled_energy_difference > RESIDUAL_WARNING {
        warnings.push(format!("the encircled energy of the matched PSF differs from the target by up to {encircled_energy_difference:.4} beyond one FWHM"));
    }
    if let (Some(source), Some(target)) = (&source_fit, &target_fit) {
        let narrower = |target: f64, source: f64| target < (1.0 - NARROWER_TOLERANCE) * source;
        if narrower(target.sigma_x, source.sigma_x) || narrower(target.sigma_y, source.sigma_y) {
            warnings.push(format!(
                "the target PSF is narrower than the source along one axis (Gaussian sigmas {:.3} x {:.3} px against {:.3} x {:.3} px), so no kernel can match it in every direction",
                target.sigma_x, target.sigma_y, source.sigma_x, source.sigma_y
            ));
        }
    }
    let (rows, cols) = kernel.dim();
    let (negative_weight, sum_squares) = negative_weight_and_sum_squares(kernel);
    Ok(KernelDiagnostics {
        rows,
        cols,
        sum: kernel.sum(),
        min: kernel.iter().copied().fold(f64::INFINITY, f64::min),
        max: kernel.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        negative_fraction: kernel.iter().filter(|&&v| v < 0.0).count() as f64 / kernel.len().max(1) as f64,
        negative_weight,
        sum_squares,
        residual_max,
        residual_rms,
        encircled_energy_difference,
        source_fwhm_px,
        target_fwhm_px,
        matched_fwhm_px,
        warnings,
    })
}

fn construct(source: &Array2<f64>, target: &Array2<f64>, method: KernelMethod) -> Result<(Array2<f64>, Option<f64>), KernelError> {
    match method {
        KernelMethod::Wiener { eps, eps_mode, shape } => {
            let shape = match shape {
                KernelShape::Named(ShapeName::Reference) => target.dim(),
                KernelShape::Named(ShapeName::Max) => (source.nrows().max(target.nrows()), source.ncols().max(target.ncols())),
                KernelShape::Fixed([rows, cols]) => (rows, cols),
            };
            let (kernel, eps_absolute) = wiener_with(source, target, shape, eps, eps_mode)?;
            Ok((kernel, Some(eps_absolute)))
        }
        KernelMethod::Window { window, regularization } => Ok((ratio_window(source, target, window, regularization)?, None)),
    }
}

fn tune(source: &Array2<f64>, target: &Array2<f64>, method: KernelMethod) -> Result<(KernelMethod, AutoChoice), KernelError> {
    let (parameter, candidates): (&'static str, Vec<(f64, KernelMethod)>) = match method {
        KernelMethod::Wiener { shape, .. } => {
            ("eps", AUTO_EPS.iter().map(|&eps| (eps, KernelMethod::Wiener { eps, eps_mode: EpsMode::Relative, shape })).collect())
        }
        KernelMethod::Window { window, regularization } => {
            let ascending: Vec<f64> = (1..=AUTO_WINDOW_STEPS).map(|k| k as f64 / AUTO_WINDOW_STEPS as f64).collect();
            let (parameter, tapers): (&'static str, Vec<(f64, TaperWindow)>) = match window {
                TaperWindow::Hanning => return Err(KernelError::NothingToTune(window.name())),
                TaperWindow::TopHat { .. } => ("beta", ascending.iter().map(|&beta| (beta, TaperWindow::TopHat { beta })).collect()),
                TaperWindow::CosineBell { .. } => ("alpha", ascending.iter().map(|&alpha| (alpha, TaperWindow::CosineBell { alpha })).collect()),
                TaperWindow::SplitCosineBell { beta, .. } => {
                    ("alpha", ascending.iter().map(|&alpha| (alpha, TaperWindow::SplitCosineBell { alpha, beta })).collect())
                }
                TaperWindow::Tukey { .. } => ("alpha", ascending.iter().rev().map(|&alpha| (alpha, TaperWindow::Tukey { alpha })).collect()),
            };
            (parameter, tapers.into_iter().map(|(value, window)| (value, KernelMethod::Window { window, regularization })).collect())
        }
    };
    let scored = candidates
        .iter()
        .map(|&(value, method)| {
            let (kernel, _) = construct(source, target, method)?;
            let MatchedPsf { residual_max, residual_rms, .. } = matched_psf(source, target, &kernel)?;
            let (negative_weight, sum_squares) = negative_weight_and_sum_squares(&kernel);
            Ok(AutoCandidate { value, residual_max, residual_rms, negative_weight, sum_squares })
        })
        .collect::<Result<Vec<_>, KernelError>>()?;
    let best = scored.iter().map(|candidate| candidate.residual_rms).fold(f64::INFINITY, f64::min);
    let index = scored.iter().position(|candidate| candidate.residual_rms <= best * (1.0 + AUTO_TOLERANCE)).unwrap_or(0);
    Ok((candidates[index].1, AutoChoice { parameter, chosen: scored[index].value, candidates: scored }))
}

pub fn build_kernel(source: &Array2<f64>, target: &Array2<f64>, options: &KernelOptions) -> Result<MatchingKernel, KernelError> {
    checked_psf("source", source)?;
    checked_psf("target", target)?;
    if let (Some(source_fit), Some(target_fit)) = (fit_gaussian(source), fit_gaussian(target)) {
        if target_fit.fwhm_px < (1.0 - NARROWER_TOLERANCE) * source_fit.fwhm_px {
            return Err(KernelError::TargetNarrower { source_fwhm_px: source_fit.fwhm_px, target_fwhm_px: target_fit.fwhm_px });
        }
    }
    let (method, auto) = if options.auto {
        let (method, choice) = tune(source, target, options.method)?;
        (method, Some(choice))
    } else {
        (options.method, None)
    };
    let (data, eps_absolute) = construct(source, target, method)?;
    let mut diagnostics = diagnose_kernel(source, target, &data)?;
    if let KernelMethod::Window { window, .. } = method {
        let (alpha, beta) = window.alpha_beta();
        if alpha + beta > 1.0 {
            diagnostics.warnings.push(format!("alpha + beta = {} exceeds 1, so the {} window does not fall to zero along the frequency axes", alpha + beta, window.name()));
        }
    }
    Ok(MatchingKernel { data, method, eps_absolute, auto, diagnostics })
}

pub fn kernel_file_name(filter: &str, reference: &str) -> String {
    format!("kernel_{}_to_{}.fits", filter.trim().to_lowercase(), reference.trim().to_lowercase())
}

pub fn kernel_header(kernel: &MatchingKernel, source_label: &str, target_label: &str, pixel_scale_arcsec: Option<f64>) -> HduHeader {
    let mut header = HduHeader { cards: Vec::new(), index: HashMap::new(), string_keys: Some(HashSet::new()) };
    set_text(&mut header, "SRCPSF", source_label);
    set_text(&mut header, "TGTPSF", target_label);
    let recipe = match kernel.method {
        KernelMethod::Wiener { eps, eps_mode, .. } => {
            set_text(&mut header, "KMETHOD", "wiener");
            header.set_f64("KEPS", eps);
            set_text(&mut header, "KEPSMODE", eps_mode.label());
            let absolute = kernel.eps_absolute.unwrap_or(eps);
            header.set_f64("KEPSABS", absolute);
            format!("Wiener kernel F_t conj(F_s) / (|F_s|^2 + eps) after LIGHTSTACK build_kernel, {} eps {eps} (absolute {absolute:e})", eps_mode.label())
        }
        KernelMethod::Window { window, regularization } => {
            set_text(&mut header, "KMETHOD", "window");
            set_text(&mut header, "KWINDOW", window.name());
            let (alpha, beta) = window.alpha_beta();
            header.set_f64("KALPHA", alpha);
            header.set_f64("KBETA", beta);
            header.set_f64("KREG", regularization);
            format!(
                "ratio kernel T/S where |S| > {regularization} max|S|, after the photutils window recipe, {} window with alpha {alpha} and beta {beta}",
                window.name()
            )
        }
    };
    header.set("KAUTO", if kernel.auto.is_some() { "T" } else { "F" }.to_string());
    if let Some(scale) = pixel_scale_arcsec {
        header.set_f64("PIXSCALE", scale);
    }
    let d = &kernel.diagnostics;
    for (key, fwhm) in [("SRCFWHM", d.source_fwhm_px), ("TGTFWHM", d.target_fwhm_px), ("MATFWHM", d.matched_fwhm_px)] {
        if let Some(fwhm) = fwhm {
            header.set_f64(key, fwhm);
        }
    }
    header.set_f64("KSUMSQ", d.sum_squares);
    push_history(&mut header, &format!("Matching kernel from {source_label} to {target_label}: {recipe}"));
    if let Some(choice) = &kernel.auto {
        push_history(
            &mut header,
            &format!(
                "{} {} chosen automatically: the strongest regularization within {} percent of the smallest matched-PSF RMS residual over {} candidates",
                choice.parameter,
                choice.chosen,
                100.0 * AUTO_TOLERANCE,
                choice.candidates.len()
            ),
        );
    }
    push_history(
        &mut header,
        &format!(
            "Matched-PSF residual max {:.3e} and RMS {:.3e} of the target peak; encircled energy difference {:.4}; FWHM {} px to {} px (target {} px); kernel sum {:.6}, {:.1} percent negative pixels; negative weight W- {:.3}, sum of squared coefficients {:.4} (per-pixel white-noise variance factor)",
            d.residual_max,
            d.residual_rms,
            d.encircled_energy_difference,
            fwhm_text(d.source_fwhm_px),
            fwhm_text(d.matched_fwhm_px),
            fwhm_text(d.target_fwhm_px),
            d.sum,
            100.0 * d.negative_fraction,
            d.negative_weight,
            d.sum_squares
        ),
    );
    header
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::core::cubeprep::psf_io::load_psf;
    use crate::infra::fits::writer::write_fits_f64;
    use crate::math::bspline::scipy_golden::{array2, assert_close, checked, number};
    use serde_json::Value;

    const PARITY: f64 = 1e-9;

    fn gaussian(rows: usize, cols: usize, sigma: f64) -> Array2<f64> {
        let (cy, cx) = ((rows / 2) as f64, (cols / 2) as f64);
        let psf = Array2::from_shape_fn((rows, cols), |(r, c)| {
            let (dy, dx) = (r as f64 - cy, c as f64 - cx);
            (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()
        });
        let total = psf.sum();
        psf / total
    }

    fn fixture(path: &str) -> Array2<f64> {
        load_psf(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightstack").join(path), None).unwrap().data
    }

    fn golden() -> Value {
        checked("kernels", "kernel_goldens.json")
    }

    fn golden_psfs(golden: &Value, source: &str) -> (Array2<f64>, Array2<f64>) {
        (fixture(golden["sources"][source].as_str().unwrap()), fixture(golden["target"].as_str().unwrap()))
    }

    fn d1(name: &str) -> Array2<f64> {
        fixture(&format!("d1/psf/{name}"))
    }

    fn window_of(case: &Value) -> TaperWindow {
        let (alpha, beta) = (number(&case["alpha"]), number(&case["beta"]));
        match case["window"].as_str().unwrap() {
            "top_hat" => TaperWindow::TopHat { beta },
            "cosine_bell" => TaperWindow::CosineBell { alpha },
            "split_cosine_bell" => TaperWindow::SplitCosineBell { alpha, beta },
            "hanning" => TaperWindow::Hanning,
            "tukey" => TaperWindow::Tukey { alpha },
            other => panic!("unexpected window {other}"),
        }
    }

    fn second_moment_sigmas(kernel: &Array2<f64>) -> (f64, f64) {
        let total = kernel.sum();
        let (mut mx, mut my) = (0.0, 0.0);
        for ((r, c), &v) in kernel.indexed_iter() {
            mx += v * c as f64;
            my += v * r as f64;
        }
        let (mx, my) = (mx / total, my / total);
        let (mut vx, mut vy) = (0.0, 0.0);
        for ((r, c), &v) in kernel.indexed_iter() {
            vx += v * (c as f64 - mx).powi(2);
            vy += v * (r as f64 - my).powi(2);
        }
        ((vx / total).sqrt(), (vy / total).sqrt())
    }

    fn wiener_options(eps: f64, eps_mode: EpsMode, shape: KernelShape) -> KernelOptions {
        KernelOptions { method: KernelMethod::Wiener { eps, eps_mode, shape }, auto: false }
    }

    fn window_options(window: TaperWindow) -> KernelOptions {
        KernelOptions { method: KernelMethod::Window { window, regularization: DEFAULT_REGULARIZATION }, auto: false }
    }

    #[test]
    fn gaussian_to_gaussian_kernel_is_gaussian_with_quadrature_sigma() {
        let (source, target) = (gaussian(41, 41, 1.5), gaussian(41, 41, 2.5));
        let recipes = [
            ("wiener", wiener_options(1e-10, EpsMode::Relative, KernelShape::default())),
            ("top hat 1", window_options(TaperWindow::TopHat { beta: 1.0 })),
        ];
        for (name, options) in recipes {
            let kernel = build_kernel(&source, &target, &options).unwrap();
            let fit = fit_gaussian(&kernel.data).unwrap();
            assert!((fit.sigma_x - 2.0).abs() < 0.01 && (fit.sigma_y - 2.0).abs() < 0.01, "{name}: kernel sigma {} x {}", fit.sigma_x, fit.sigma_y);
            assert!((fit.x - 20.0).abs() < 1e-6 && (fit.y - 20.0).abs() < 1e-6, "{name}: kernel centre ({}, {})", fit.x, fit.y);
            assert!((kernel.data.sum() - 1.0).abs() < 1e-12, "{name}: sum {}", kernel.data.sum());
            let d = &kernel.diagnostics;
            assert!(d.residual_max < 1e-3, "{name}: residual {}", d.residual_max);
            assert!((d.matched_fwhm_px.unwrap() / d.target_fwhm_px.unwrap() - 1.0).abs() < 1e-3, "{name}: {d:?}");
        }
        let documented = build_kernel(&gaussian(25, 25, 3.0), &gaussian(25, 25, 5.0), &window_options(TaperWindow::SplitCosineBell { alpha: 0.0, beta: 0.2 })).unwrap();
        let fit = fit_gaussian(&documented.data).unwrap();
        assert!((fit.sigma_x - 4.0).abs() < 0.06 && (fit.sigma_y - 4.0).abs() < 0.06, "photutils' documented case: sigma {} x {}", fit.sigma_x, fit.sigma_y);
    }

    #[test]
    fn wiener_kernel_matches_lightstack_build_kernel() {
        let golden = golden();
        for case in golden["wiener"]["cases"].as_array().unwrap() {
            let name = case["source"].as_str().unwrap();
            let (source, target) = golden_psfs(&golden, name);
            let (source, target) = (source * number(&case["source_factor"]), target * number(&case["target_factor"]));
            let shape = (case["shape"][0].as_u64().unwrap() as usize, case["shape"][1].as_u64().unwrap() as usize);
            let eps = number(&case["eps"]);
            let what = format!("{name} to F444W, shape {shape:?}, eps {eps}");
            let expected = array2(&case["kernel"]);
            assert_close(&wiener(&source, &target, shape, eps).unwrap(), &expected, PARITY, &what);
            let built = build_kernel(&source, &target, &wiener_options(eps, EpsMode::Absolute, KernelShape::Fixed([shape.0, shape.1]))).unwrap();
            assert_close(&built.data, &expected, PARITY, &format!("{what} through build_kernel"));
            assert_eq!(built.eps_absolute, Some(eps));
        }
        let (source, target) = golden_psfs(&golden, "F277W");
        let reference_shape = build_kernel(&source, &target, &wiener_options(0.05, EpsMode::Absolute, KernelShape::default())).unwrap();
        assert_eq!(reference_shape.data.dim(), target.dim(), "shape reference takes the target PSF shape, as LIGHTSTACK's tutorial");
    }

    #[test]
    fn cosine_bell_kernel_matches_photutils() {
        let golden = golden();
        for case in golden["window"]["cases"].as_array().unwrap() {
            let name = case["source"].as_str().unwrap();
            let (source, target) = golden_psfs(&golden, name);
            let window = window_of(case);
            let kernel = ratio_window(&source, &target, window, number(&case["regularization"])).unwrap();
            assert_close(&kernel, &array2(&case["kernel"]), PARITY, &format!("{name} to F444W with {window:?}"));
        }
        let wide = |sigma: f64| gaussian(51, 101, sigma);
        let kernel = ratio_window(&wide(2.0), &wide(3.0), TaperWindow::SplitCosineBell { alpha: 0.3, beta: 0.1 }, DEFAULT_REGULARIZATION).unwrap();
        let (sx, sy) = second_moment_sigmas(&kernel);
        assert!((sx / sy - 1.0).abs() < 0.01, "photutils' documented non-square case: second-moment sigmas {sx} and {sy} differ");
    }

    #[test]
    fn target_narrower_than_source_is_refused() {
        let (broad, narrow) = (d1("d1_psf_JWST_F444W_40mas.fits"), d1("d1_psf_JWST_F150W_40mas.fits"));
        for options in [wiener_options(0.05, EpsMode::Absolute, KernelShape::default()), window_options(TaperWindow::CosineBell { alpha: 0.7 })] {
            match build_kernel(&broad, &narrow, &options) {
                Err(KernelError::TargetNarrower { source_fwhm_px, target_fwhm_px }) => {
                    assert!(target_fwhm_px < source_fwhm_px, "{source_fwhm_px} {target_fwhm_px}");
                }
                other => panic!("{options:?}: expected TargetNarrower, got {other:?}"),
            }
        }
        let same = build_kernel(&broad, &broad, &wiener_options(1e-9, EpsMode::Relative, KernelShape::default())).unwrap();
        assert!(same.diagnostics.residual_max < 1e-3, "a PSF matched to itself is allowed: {:?}", same.diagnostics);
        let source = gaussian(25, 25, 2.0);
        let options = wiener_options(1e-6, EpsMode::Relative, KernelShape::default());
        let comparable = build_kernel(&source, &gaussian(25, 25, 1.99), &options);
        assert!(comparable.is_ok(), "a target within NARROWER_TOLERANCE of the source width is allowed: {comparable:?}");
        let narrower = build_kernel(&source, &gaussian(25, 25, 2.0 * (1.0 - 1.5 * NARROWER_TOLERANCE)), &options);
        assert!(matches!(narrower, Err(KernelError::TargetNarrower { .. })), "{narrower:?}");
    }

    #[test]
    fn relative_eps_makes_the_kernel_independent_of_psf_normalization() {
        let (source, target) = (d1("d1_psf_HST_F814W_40mas.fits"), d1("d1_psf_JWST_F444W_40mas.fits"));
        let (scaled_source, scaled_target) = (&source * 0.97, &target * 0.99);
        let relative = |s: &Array2<f64>, t: &Array2<f64>| wiener_with(s, t, t.dim(), 0.05, EpsMode::Relative).unwrap();
        let (unit, unit_eps) = relative(&source, &target);
        let (scaled, scaled_eps) = relative(&scaled_source, &scaled_target);
        assert_close(&scaled, &unit, 1e-12, "relative eps on PSFs that do not sum to 1");
        assert!((unit_eps - 0.05).abs() < 1e-12, "max |F_s|^2 is 1 for a non-negative unit-sum PSF: {unit_eps}");
        assert!((scaled_eps / (0.05 * 0.97 * 0.97) - 1.0).abs() < 1e-12, "{scaled_eps}");
        let absolute = wiener(&scaled_source, &scaled_target, target.dim(), 0.05).unwrap();
        let worst = absolute.iter().zip(unit.iter()).fold(0.0f64, |acc, (a, b)| acc.max((a - b).abs()));
        assert!(worst > 1e-6, "LIGHTSTACK's absolute eps depends on the PSF sums: {worst}");
    }

    #[test]
    fn automatic_choice_minimizes_the_matched_psf_residual() {
        let (source, target) = (d1("d1_psf_JWST_F356W_40mas.fits"), d1("d1_psf_JWST_F444W_40mas.fits"));
        let options = KernelOptions { auto: true, ..wiener_options(0.05, EpsMode::Absolute, KernelShape::default()) };
        let tuned = build_kernel(&source, &target, &options).unwrap();
        let choice = tuned.auto.as_ref().unwrap();
        assert_eq!((choice.parameter, choice.candidates.len()), ("eps", AUTO_EPS.len()));
        let best = choice.candidates.iter().map(|c| c.residual_rms).fold(f64::INFINITY, f64::min);
        let chosen = choice.candidates.iter().position(|c| c.value == choice.chosen).unwrap();
        assert!(choice.candidates[chosen].residual_rms <= best * (1.0 + AUTO_TOLERANCE));
        assert!(choice.candidates[..chosen].iter().all(|c| c.residual_rms > best * (1.0 + AUTO_TOLERANCE)), "{choice:?}");
        assert_eq!(tuned.method, KernelMethod::Wiener { eps: choice.chosen, eps_mode: EpsMode::Relative, shape: KernelShape::default() });
        let lightstack = build_kernel(&source, &target, &wiener_options(0.05, EpsMode::Absolute, KernelShape::default())).unwrap();
        assert!(tuned.diagnostics.residual_max < RESIDUAL_WARNING, "{:?}", tuned.diagnostics);
        assert!(lightstack.diagnostics.residual_max > 10.0 * tuned.diagnostics.residual_max, "{:?}", lightstack.diagnostics);
        assert!(lightstack.diagnostics.warnings.iter().any(|w| w.contains("percent of its peak")), "{:?}", lightstack.diagnostics.warnings);
        for candidate in &choice.candidates {
            let rebuilt = build_kernel(&source, &target, &wiener_options(candidate.value, EpsMode::Relative, KernelShape::default())).unwrap();
            let d = &rebuilt.diagnostics;
            assert_eq!((candidate.negative_weight, candidate.sum_squares), (d.negative_weight, d.sum_squares), "eps {}", candidate.value);
            assert!(candidate.negative_weight > 0.0 && candidate.sum_squares > 0.0, "{candidate:?}");
        }
        assert!(choice.candidates.windows(2).all(|pair| pair[1].sum_squares >= pair[0].sum_squares), "sum k^2 grows as eps falls: {:?}", choice.candidates);
        let (tuned_d, lightstack_d) = (&tuned.diagnostics, &lightstack.diagnostics);
        assert!((tuned_d.sum_squares - 0.2392).abs() < 1e-3 && (tuned_d.negative_weight - 1.249).abs() < 1e-3, "{tuned_d:?}");
        assert!((lightstack_d.sum_squares - 0.0920).abs() < 1e-3 && (lightstack_d.negative_weight - 0.3808).abs() < 1e-3, "{lightstack_d:?}");
        assert!(tuned_d.warnings.len() <= lightstack_d.warnings.len() && !tuned_d.warnings.iter().any(|w| w.contains("negative weight")), "{:?}", tuned_d.warnings);

        let window = build_kernel(&source, &target, &KernelOptions { auto: true, ..window_options(TaperWindow::Tukey { alpha: 0.5 }) }).unwrap();
        let choice = window.auto.unwrap();
        assert_eq!((choice.parameter, choice.candidates.len()), ("alpha", 20));
        assert!(choice.candidates.windows(2).all(|pair| pair[0].value > pair[1].value), "Tukey goes from the strongest taper (alpha 1) to the weakest");
        let refused = build_kernel(&source, &target, &KernelOptions { auto: true, ..window_options(TaperWindow::Hanning) });
        assert_eq!(refused.unwrap_err(), KernelError::NothingToTune("hanning"));
    }

    #[test]
    fn kernel_diagnostics_report_residual_encircled_energy_fwhm_sum_and_negative_fraction() {
        let (source, target) = (d1("d1_psf_JWST_F150W_40mas.fits"), d1("d1_psf_JWST_F444W_40mas.fits"));
        let good = build_kernel(&source, &target, &KernelOptions { auto: true, ..KernelOptions::default() }).unwrap();
        let d = &good.diagnostics;
        assert_eq!((d.rows, d.cols), (21, 21));
        assert!((d.sum - 1.0).abs() < 1e-12 && d.min < 0.0 && d.max > 0.0, "{d:?}");
        assert!(d.negative_fraction > 0.0 && d.negative_fraction < 1.0, "{d:?}");
        assert!(d.residual_max < RESIDUAL_WARNING && d.residual_rms < d.residual_max, "{d:?}");
        assert!(d.encircled_energy_difference <= 0.01, "{d:?}");
        let (source_fwhm, target_fwhm, matched_fwhm) = (d.source_fwhm_px.unwrap(), d.target_fwhm_px.unwrap(), d.matched_fwhm_px.unwrap());
        assert!(source_fwhm < target_fwhm && (matched_fwhm / target_fwhm - 1.0).abs() < FWHM_WARNING, "{d:?}");
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        let negative = good.data.iter().filter(|&&v| v < 0.0).count() as f64 / good.data.len() as f64;
        assert_eq!(d.negative_fraction, negative);
        assert_eq!(d.sum_squares, good.data.iter().map(|v| v * v).sum::<f64>());
        assert_eq!(d.negative_weight, -good.data.iter().filter(|&&v| v < 0.0).sum::<f64>());
        assert!(d.sum_squares > 0.0 && d.negative_weight > 0.0 && d.sum_squares < 1.0, "{d:?}");

        let poor = build_kernel(&source, &target, &window_options(TaperWindow::CosineBell { alpha: 0.7 })).unwrap();
        let p = &poor.diagnostics;
        assert!(p.residual_max > 0.1 && p.encircled_energy_difference > 0.01, "{p:?}");
        assert!(p.matched_fwhm_px.unwrap() > p.target_fwhm_px.unwrap() * (1.0 + FWHM_WARNING), "{p:?}");
        assert!(p.warnings.iter().any(|w| w.contains("percent of its peak")) && p.warnings.iter().any(|w| w.contains("matched FWHM")), "{:?}", p.warnings);
        let again = diagnose_kernel(&source, &target, &poor.data).unwrap();
        assert_eq!(&again, p);
    }

    #[test]
    fn kernel_header_records_the_recipe_and_the_pixel_scale() {
        let (source, target) = (d1("d1_psf_JWST_F277W_40mas.fits"), d1("d1_psf_JWST_F444W_40mas.fits"));
        let wiener_kernel = build_kernel(&source, &target, &wiener_options(0.05, EpsMode::Absolute, KernelShape::default())).unwrap();
        let header = kernel_header(&wiener_kernel, "JWST_F277W", "JWST_F444W", Some(0.04));
        let text = |key: &str| header.get(key).map(str::to_string);
        assert_eq!((text("SRCPSF"), text("TGTPSF"), text("KMETHOD")), (Some("JWST_F277W".into()), Some("JWST_F444W".into()), Some("wiener".into())));
        assert_eq!((header.get_f64("KEPS"), text("KEPSMODE"), text("KAUTO")), (Some(0.05), Some("absolute".into()), Some("F".into())));
        assert_eq!(header.get_f64("PIXSCALE"), Some(0.04));
        assert!(header.is_string_value("SRCPSF").unwrap() && !header.is_string_value("KEPS").unwrap());
        let history: Vec<&str> = header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.as_str()).collect();
        assert!(history.join(" ").contains("LIGHTSTACK build_kernel"), "{history:?}");
        assert!(history.join(" ").contains("residual"), "{history:?}");
        let d = &wiener_kernel.diagnostics;
        let card = |key: &str, value: Option<f64>| match (header.get_f64(key), value) {
            (Some(card), Some(value)) => (card / value - 1.0).abs() < 1e-13,
            (card, value) => card.is_none() && value.is_none(),
        };
        assert!(card("SRCFWHM", d.source_fwhm_px) && card("TGTFWHM", d.target_fwhm_px) && card("MATFWHM", d.matched_fwhm_px), "{:?}", header.cards);
        assert!(d.source_fwhm_px.is_some() && d.matched_fwhm_px.is_some());
        assert!(card("KSUMSQ", Some(d.sum_squares)), "{:?}", header.get("KSUMSQ"));
        assert!(!header.is_string_value("KSUMSQ").unwrap() && !header.is_string_value("MATFWHM").unwrap());
        let joined = history.join(" ");
        assert!(joined.contains(&format!("negative weight W- {:.3}", d.negative_weight)), "{history:?}");
        assert!(joined.contains(&format!("sum of squared coefficients {:.4} (per-pixel white-noise variance factor)", d.sum_squares)), "{history:?}");
        let scratch = tempfile::tempdir().unwrap();
        let auto_wiener = build_kernel(&source, &target, &KernelOptions { auto: true, ..wiener_options(0.05, EpsMode::Absolute, KernelShape::default()) }).unwrap();
        let auto_window = build_kernel(&source, &target, &KernelOptions { auto: true, ..window_options(TaperWindow::SplitCosineBell { alpha: 0.3, beta: 0.1 }) }).unwrap();
        for (name, kernel) in [("tutorial", &wiener_kernel), ("auto_wiener", &auto_wiener), ("auto_window", &auto_window)] {
            let path = scratch.path().join(format!("{name}.fits"));
            write_fits_f64(path.to_str().unwrap(), &kernel.data, Some(&kernel_header(kernel, "JWST_F277W", "JWST_F444W", Some(0.04)))).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 8640, "{name}: one 2880-byte header block and two data blocks for a 21x21 kernel");
        }

        let window_kernel = build_kernel(&source, &target, &window_options(TaperWindow::SplitCosineBell { alpha: 0.3, beta: 0.1 })).unwrap();
        let header = kernel_header(&window_kernel, "F277W", "F444W", None);
        assert_eq!(header.get("KWINDOW"), Some("split_cosine_bell"));
        assert_eq!((header.get_f64("KALPHA"), header.get_f64("KBETA"), header.get_f64("KREG")), (Some(0.3), Some(0.1), Some(DEFAULT_REGULARIZATION)));
        assert!(header.get("PIXSCALE").is_none() && header.get("KEPS").is_none());
        assert_eq!(kernel_file_name(" F277W", "F444W "), "kernel_f277w_to_f444w.fits");

        let unknown = MatchingKernel {
            diagnostics: KernelDiagnostics { source_fwhm_px: None, target_fwhm_px: None, matched_fwhm_px: None, ..wiener_kernel.diagnostics.clone() },
            ..wiener_kernel.clone()
        };
        let header = kernel_header(&unknown, "JWST_F277W", "JWST_F444W", Some(0.04));
        assert!(header.get("SRCFWHM").is_none() && header.get("TGTFWHM").is_none() && header.get("MATFWHM").is_none());
        assert!((header.get_f64("KSUMSQ").unwrap() / wiener_kernel.diagnostics.sum_squares - 1.0).abs() < 1e-13);
        let history = header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.as_str()).collect::<Vec<_>>().join(" ");
        assert!(history.contains("FWHM unknown px to unknown px (target unknown px)"), "{history}");
    }

    #[test]
    fn shape_max_pads_the_smaller_psf_to_the_larger_odd_shape() {
        let (source, target) = (gaussian(41, 41, 2.0), gaussian(21, 21, 3.0));
        let shape_max = KernelShape::Named(ShapeName::Max);
        let padded = build_kernel(&source, &target, &wiener_options(1e-3, EpsMode::Absolute, shape_max)).unwrap();
        assert_eq!(padded.data.dim(), (41, 41));
        let fixed = build_kernel(&source, &target, &wiener_options(1e-3, EpsMode::Absolute, KernelShape::Fixed([41, 41]))).unwrap();
        assert_eq!(padded.data, fixed.data, "max is the explicit [41, 41] shape bit for bit");
        assert!((padded.data.sum() - 1.0).abs() < 1e-12);
        match build_kernel(&source, &target, &wiener_options(1e-3, EpsMode::Absolute, KernelShape::default())) {
            Err(KernelError::KernelShape { rows: 21, cols: 21, needed_rows: 41, needed_cols: 41 }) => {}
            other => panic!("shape reference keeps refusing a source larger than the reference PSF: {other:?}"),
        }
        let swapped = build_kernel(&gaussian(21, 21, 2.0), &gaussian(41, 41, 3.0), &wiener_options(1e-3, EpsMode::Absolute, shape_max)).unwrap();
        assert_eq!(swapped.data.dim(), (41, 41), "max pads the smaller source too");
        let mixed = build_kernel(&gaussian(41, 21, 2.0), &gaussian(21, 41, 3.0), &wiener_options(1e-3, EpsMode::Absolute, shape_max)).unwrap();
        assert_eq!(mixed.data.dim(), (41, 41), "max takes each axis from the larger PSF");

        let (f277w, f444w) = (d1("d1_psf_JWST_F277W_40mas.fits"), d1("d1_psf_JWST_F444W_40mas.fits"));
        let reference = build_kernel(&f277w, &f444w, &wiener_options(0.05, EpsMode::Absolute, KernelShape::default())).unwrap();
        let max = build_kernel(&f277w, &f444w, &wiener_options(0.05, EpsMode::Absolute, shape_max)).unwrap();
        assert_eq!(max.data, reference.data, "PSFs of one shape give the same kernel bit for bit");
        assert_eq!(max.diagnostics, reference.diagnostics);
    }

    #[test]
    fn kernel_options_parse_the_recipe_step() {
        let parse = |json: Value| serde_json::from_value::<KernelOptions>(json).unwrap();
        let tutorial = parse(serde_json::json!({ "step": "kernels", "reference": "F444W", "method": "wiener", "eps": 0.05, "shape": "reference" }));
        assert_eq!(tutorial, wiener_options(0.05, EpsMode::Absolute, KernelShape::Named(ShapeName::Reference)));
        let fixed = parse(serde_json::json!({ "method": "wiener", "shape": [31, 21], "eps_mode": "relative", "auto": true }));
        assert_eq!(fixed, KernelOptions { auto: true, ..wiener_options(DEFAULT_EPS, EpsMode::Relative, KernelShape::Fixed([31, 21])) });
        let window = parse(serde_json::json!({ "method": "window", "window": "cosine_bell", "alpha": 0.7 }));
        assert_eq!(window, window_options(TaperWindow::CosineBell { alpha: 0.7 }));
        let back = serde_json::to_value(window).unwrap();
        assert_eq!(back, serde_json::json!({ "method": "window", "window": "cosine_bell", "alpha": 0.7, "regularization": DEFAULT_REGULARIZATION, "auto": false }));
        assert_eq!(serde_json::to_value(KernelOptions::default()).unwrap()["shape"], "reference");
        let max = parse(serde_json::json!({ "method": "wiener", "eps": 0.05, "shape": "max" }));
        assert_eq!(max, wiener_options(0.05, EpsMode::Absolute, KernelShape::Named(ShapeName::Max)));
        assert_eq!(serde_json::to_value(max).unwrap()["shape"], "max");
        assert!(serde_json::from_value::<KernelOptions>(serde_json::json!({ "method": "wiener", "shape": "largest" })).is_err());
    }

    #[test]
    fn invalid_psfs_and_parameters_are_refused() {
        let psf = gaussian(21, 21, 2.0);
        let target = gaussian(21, 21, 3.0);
        let options = wiener_options(0.05, EpsMode::Absolute, KernelShape::default());
        let even = gaussian(20, 21, 2.0);
        assert!(matches!(build_kernel(&even, &target, &options), Err(KernelError::EvenSize { which: "source", rows: 20, cols: 21 })));
        let mut holed = psf.clone();
        holed[[3, 4]] = f64::NAN;
        assert!(matches!(build_kernel(&holed, &target, &options), Err(KernelError::NonFinite { which: "source", count: 1 })));
        assert!(matches!(build_kernel(&psf, &Array2::zeros((21, 21)), &options), Err(KernelError::ZeroSum { which: "target", .. })));
        assert!(matches!(wiener(&psf, &target, (19, 21), 0.05), Err(KernelError::KernelShape { .. })));
        assert!(matches!(wiener(&psf, &target, (22, 22), 0.05), Err(KernelError::KernelShape { .. })));
        assert_eq!(wiener(&psf, &target, (21, 21), 0.0).unwrap_err(), KernelError::InvalidEps(0.0));
        assert!(matches!(ratio_window(&psf, &gaussian(23, 23, 3.0), TaperWindow::Hanning, 1e-4), Err(KernelError::ShapeMismatch { .. })));
        assert_eq!(ratio_window(&psf, &target, TaperWindow::Hanning, 1.0).unwrap_err(), KernelError::InvalidRegularization(1.0));
        assert!(matches!(ratio_window(&psf, &target, TaperWindow::CosineBell { alpha: 1.5 }, 1e-4), Err(KernelError::InvalidWindow { window: "cosine_bell", .. })));
        let padded = wiener(&psf, &gaussian(15, 15, 3.0), (25, 25), 0.05).unwrap();
        assert_eq!(padded.dim(), (25, 25), "PSFs smaller than the kernel shape are zero-padded");
    }
}
