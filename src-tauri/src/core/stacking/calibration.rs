use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::stacking::combine::{reject_and_combine_with, KernelScratch, Sample};
use crate::infra::progress::ProgressHandle;
use crate::math::median::{f32_cmp, median_f32_mut};
use crate::types::constants::STAGE_LOAD_FRAME;
use crate::types::error::AppError;
use crate::types::stacking::{CombineMethod, RejectionMethod, RejectionParams};
pub(crate) use crate::infra::fits::reader::load_fits_image;

const FLAT_MEDIAN_MAX_SAMPLES: usize = 131_072;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MasterConfig {
    pub rejection: RejectionMethod,
    pub combine: CombineMethod,
    pub sigma_low: f32,
    pub sigma_high: f32,
}

impl Default for MasterConfig {
    fn default() -> Self {
        Self {
            rejection: RejectionMethod::WinsorizedSigmaClip,
            combine: CombineMethod::Mean,
            sigma_low: 4.0,
            sigma_high: 3.0,
        }
    }
}

impl MasterConfig {
    pub fn rejection_params(&self) -> RejectionParams {
        RejectionParams {
            rejection: self.rejection,
            combine: self.combine,
            sigma_low: self.sigma_low,
            sigma_high: self.sigma_high,
            ..RejectionParams::default()
        }
    }
}

pub struct CalibrationConfig {
    pub master_bias: Option<Array2<f32>>,
    pub master_dark: Option<Array2<f32>>,
    pub master_flat: Option<Array2<f32>>,
    pub dark_exposure_ratio: f32,
}

pub fn subtract_bias(image: &Array2<f32>, master_bias: &Array2<f32>) -> Array2<f32> {
    image - master_bias
}

pub fn subtract_dark(
    image: &Array2<f32>,
    master_dark: &Array2<f32>,
    exposure_ratio: f32,
) -> Array2<f32> {
    image - &(master_dark * exposure_ratio)
}

pub fn divide_flat(image: &Array2<f32>, master_flat: &Array2<f32>) -> Array2<f32> {
    let (rows, cols) = image.dim();
    let img_slice = image.as_slice().expect("contiguous");
    let flat_slice = master_flat.as_slice().expect("contiguous");

    let result: Vec<f32> = img_slice
        .par_iter()
        .zip(flat_slice.par_iter())
        .map(|(&iv, &fv)| {
            if fv.is_finite() && fv.abs() > 1e-4 {
                iv / fv
            } else {
                iv
            }
        })
        .collect();

    Array2::from_shape_vec((rows, cols), result).unwrap()
}

fn ensure_master_dims(
    name: &str,
    master: Option<&Array2<f32>>,
    rows: usize,
    cols: usize,
) -> Result<()> {
    if let Some(m) = master {
        if m.dim() != (rows, cols) {
            bail!(
                "master {} shape {:?} does not match science frame {:?}",
                name,
                m.dim(),
                (rows, cols)
            );
        }
    }
    Ok(())
}

pub fn calibrate_image(raw: &Array2<f32>, config: &CalibrationConfig) -> Result<Array2<f32>> {
    let (rows, cols) = raw.dim();
    ensure_master_dims("bias", config.master_bias.as_ref(), rows, cols)?;
    ensure_master_dims("dark", config.master_dark.as_ref(), rows, cols)?;
    ensure_master_dims("flat", config.master_flat.as_ref(), rows, cols)?;
    let npix = rows * cols;
    let src = raw.as_slice().expect("contiguous");

    let bias_slice = config.master_bias.as_ref().and_then(|b| b.as_slice());
    let dark_slice = config.master_dark.as_ref().and_then(|d| d.as_slice());
    let flat_slice = config.master_flat.as_ref().and_then(|f| f.as_slice());
    let dark_ratio = config.dark_exposure_ratio;

    let result: Vec<f32> = (0..npix)
        .into_par_iter()
        .map(|i| {
            let mut v = src[i];

            if let Some(bias) = bias_slice {
                v -= bias[i];
            }

            if let Some(dark) = dark_slice {
                v -= dark[i] * dark_ratio;
            }

            if let Some(flat) = flat_slice {
                let fv = flat[i];
                if fv.is_finite() && fv.abs() > 1e-4 {
                    v /= fv;
                }
            }

            v
        })
        .collect();

    Ok(Array2::from_shape_vec((rows, cols), result).unwrap())
}

pub fn combine_master_frames(frames: &[Array2<f32>], config: &MasterConfig) -> Result<Array2<f32>> {
    let Some(first) = frames.first() else {
        bail!("No frames to combine");
    };
    let (rows, cols) = first.dim();
    for (i, frame) in frames.iter().enumerate().skip(1) {
        if frame.dim() != (rows, cols) {
            bail!(
                "Dimension mismatch: frame {} is {:?}, expected ({}, {})",
                i, frame.dim(), rows, cols
            );
        }
    }

    let n = frames.len();
    let npix = rows * cols;
    let params = config.rejection_params();

    let slices: Vec<&[f32]> = frames
        .iter()
        .map(|f| f.as_slice().expect("contiguous"))
        .collect();

    let mut result = vec![0.0f32; npix];

    result
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(y, row_buf)| {
            let mut samples: Vec<Sample> = Vec::with_capacity(n);
            let mut scratch = KernelScratch::default();
            let base = y * cols;
            for x in 0..cols {
                samples.clear();
                let idx = base + x;
                for (i, s) in slices.iter().enumerate() {
                    let v = s[idx];
                    if v.is_finite() {
                        samples.push(Sample::plain(v, i as u16));
                    }
                }
                let out = reject_and_combine_with(&mut samples, None, &params, &mut scratch);
                row_buf[x] = if out.kept == 0 { 0.0 } else { out.value };
            }
        });

    Array2::from_shape_vec((rows, cols), result).context("Failed to reshape combined master")
}

fn sampled_positive_median(frame: &Array2<f32>) -> Option<f32> {
    let stride = (frame.len() / FLAT_MEDIAN_MAX_SAMPLES).max(1);
    let mut samples: Vec<f32> = frame
        .iter()
        .step_by(stride)
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if samples.is_empty() {
        return None;
    }
    Some(median_f32_mut(&mut samples))
}

pub fn scale_flats_to_first_median(frames: &mut [Array2<f32>]) {
    let Some(reference) = frames.first().and_then(sampled_positive_median) else {
        return;
    };
    frames.par_iter_mut().skip(1).for_each(|frame| {
        if let Some(median) = sampled_positive_median(frame) {
            if median > 0.0 {
                let gain = reference / median;
                if gain.is_finite() && gain != 1.0 {
                    frame.mapv_inplace(|v| v * gain);
                }
            }
        }
    });
}

fn load_matching_frames(paths: &[String]) -> Result<Vec<Array2<f32>>> {
    let first = load_fits_image(&paths[0])?;
    let (rows, cols) = first.dim();

    let mut frames = Vec::with_capacity(paths.len());
    frames.push(first);

    for path in &paths[1..] {
        let frame = load_fits_image(path)?;
        if frame.dim() != (rows, cols) {
            bail!(
                "Dimension mismatch: expected ({}, {}), got {:?}",
                rows, cols, frame.dim()
            );
        }
        frames.push(frame);
    }
    Ok(frames)
}

pub fn create_master_bias(bias_paths: &[String]) -> Result<Array2<f32>> {
    create_master_bias_with(bias_paths, &MasterConfig::default())
}

pub fn create_master_bias_with(bias_paths: &[String], config: &MasterConfig) -> Result<Array2<f32>> {
    if bias_paths.is_empty() {
        bail!("No bias frames provided");
    }
    let frames = load_matching_frames(bias_paths)?;
    combine_master_frames(&frames, config).context("Failed to combine master bias")
}

pub fn create_master_dark(
    dark_paths: &[String],
    master_bias: Option<&Array2<f32>>,
) -> Result<Array2<f32>> {
    create_master_dark_with(dark_paths, master_bias, &MasterConfig::default())
}

pub fn create_master_dark_with(
    dark_paths: &[String],
    master_bias: Option<&Array2<f32>>,
    config: &MasterConfig,
) -> Result<Array2<f32>> {
    if dark_paths.is_empty() {
        bail!("No dark frames provided");
    }

    let mut frames = load_matching_frames(dark_paths)?;
    if let Some(bias) = master_bias {
        for frame in frames.iter_mut() {
            *frame = subtract_bias(frame, bias);
        }
    }
    combine_master_frames(&frames, config).context("Failed to combine master dark")
}

fn median_exposure_seconds(paths: &[String]) -> Option<f64> {
    let mut vals: Vec<f64> = paths
        .iter()
        .filter_map(|p| {
            let header = crate::infra::fits::reader::read_primary_header(p).ok()?;
            header
                .get_f64("EXPTIME")
                .or_else(|| header.get_f64("EXPOSURE"))
                .filter(|v| v.is_finite() && *v > 0.0)
        })
        .collect();
    if vals.is_empty() {
        return None;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(vals[vals.len() / 2])
}

fn flat_dark_scale(flat_exposure: Option<f64>, dark_exposure: Option<f64>) -> f32 {
    match (flat_exposure, dark_exposure) {
        (Some(flat), Some(dark)) if dark > 0.0 && flat.is_finite() && flat >= 0.0 => {
            ((flat / dark) as f32).clamp(0.0, 20.0)
        }
        _ => 1.0,
    }
}

pub fn create_master_flat(
    flat_paths: &[String],
    master_bias: Option<&Array2<f32>>,
    master_dark: Option<&Array2<f32>>,
    dark_exposure_seconds: Option<f64>,
) -> Result<Array2<f32>> {
    create_master_flat_with(flat_paths, master_bias, master_dark, dark_exposure_seconds, &MasterConfig::default())
}

pub fn create_master_flat_with(
    flat_paths: &[String],
    master_bias: Option<&Array2<f32>>,
    master_dark: Option<&Array2<f32>>,
    dark_exposure_seconds: Option<f64>,
    config: &MasterConfig,
) -> Result<Array2<f32>> {
    if flat_paths.is_empty() {
        bail!("No flat frames provided");
    }

    let dark_scale = if master_dark.is_some() && master_bias.is_some() {
        flat_dark_scale(median_exposure_seconds(flat_paths), dark_exposure_seconds)
    } else {
        1.0
    };

    let mut frames = load_matching_frames(flat_paths)?;
    for frame in frames.iter_mut() {
        if let Some(bias) = master_bias {
            *frame = subtract_bias(frame, bias);
        }
        if let Some(dark) = master_dark {
            *frame = subtract_dark(frame, dark, dark_scale);
        }
    }

    scale_flats_to_first_median(&mut frames);

    let mut result = combine_master_frames(&frames, config).context("Failed to combine master flat")?;

    let mut positives: Vec<f32> = result
        .iter()
        .filter(|v| v.is_finite() && **v > 0.0)
        .copied()
        .collect();

    if !positives.is_empty() {
        let mid = positives.len() / 2;
        positives.select_nth_unstable_by(mid, |a, b| f32_cmp(a, b));
        let median = positives[mid] as f64;
        let inv_median = if median.abs() > 1e-10 { 1.0 / median as f32 } else { 1.0 };

        result.par_mapv_inplace(|v| {
            if v.is_finite() && v > 0.0 {
                v * inv_median
            } else {
                1.0
            }
        });
    }

    Ok(result)
}

pub fn calibrate_from_paths(
    science_path: &str,
    bias_paths: Option<&[String]>,
    dark_paths: Option<&[String]>,
    flat_paths: Option<&[String]>,
    dark_exposure_ratio: f32,
) -> Result<Array2<f32>> {
    let science = load_fits_image(science_path)?;

    let master_bias = match bias_paths {
        Some(paths) if !paths.is_empty() => Some(create_master_bias(paths)?),
        _ => None,
    };

    let master_dark = match dark_paths {
        Some(paths) if !paths.is_empty() => {
            Some(create_master_dark(paths, master_bias.as_ref())?)
        }
        _ => None,
    };

    let master_flat = match flat_paths {
        Some(paths) if !paths.is_empty() => {
            let dark_exposure = dark_paths.and_then(median_exposure_seconds);
            Some(create_master_flat(
                paths,
                master_bias.as_ref(),
                master_dark.as_ref(),
                dark_exposure,
            )?)
        }
        _ => None,
    };

    let config = CalibrationConfig {
        master_bias,
        master_dark,
        master_flat,
        dark_exposure_ratio,
    };

    calibrate_image(&science, &config)
}

fn load_frames_with_progress(
    paths: &[String],
    calibration: Option<&CalibrationConfig>,
    progress: Option<&ProgressHandle>,
) -> Result<Vec<Array2<f32>>> {
    let mut images: Vec<Array2<f32>> = Vec::with_capacity(paths.len());
    for path in paths {
        if let Some(p) = progress {
            if p.is_cancelled() {
                return Err(AppError::Cancelled.into());
            }
        }
        let mut img = load_fits_image(path)?;
        if let Some(cal) = calibration {
            img = calibrate_image(&img, cal)?;
        }
        images.push(img);
        if let Some(p) = progress {
            p.tick_with_stage(STAGE_LOAD_FRAME);
        }
    }
    Ok(images)
}

pub fn stack_from_paths(
    paths: &[String],
    config: &crate::types::stacking::StackConfig,
    calibration: Option<&CalibrationConfig>,
    progress: Option<&ProgressHandle>,
) -> Result<crate::types::stacking::StackResult> {
    if paths.is_empty() {
        bail!("No image paths provided");
    }

    let images = load_frames_with_progress(paths, calibration, progress)?;

    if let Some(p) = progress {
        if p.is_cancelled() {
            return Err(AppError::Cancelled.into());
        }
    }

    let result = crate::core::stacking::combine::stack_images(&images, config)?;
    Ok(result)
}

pub fn drizzle_from_paths(
    paths: &[String],
    config: &crate::types::stacking::DrizzleConfig,
    calibration: Option<&CalibrationConfig>,
    progress: Option<&ProgressHandle>,
) -> Result<crate::types::stacking::DrizzleResult> {
    if paths.is_empty() {
        bail!("No image paths provided");
    }

    let images = load_frames_with_progress(paths, calibration, progress)?;

    if let Some(p) = progress {
        if p.is_cancelled() {
            return Err(AppError::Cancelled.into());
        }
    }

    let result = crate::core::stacking::drizzle::drizzle_stack(&images, config)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_dark_scale_scales_by_exposure_ratio() {
        let s = flat_dark_scale(Some(5.0), Some(300.0));
        assert!((s - (5.0 / 300.0)).abs() < 1e-6, "got {}", s);
    }

    #[test]
    fn flat_dark_scale_identity_on_equal_exposure() {
        assert!((flat_dark_scale(Some(300.0), Some(300.0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn flat_dark_scale_falls_back_to_one_when_unknown() {
        assert_eq!(flat_dark_scale(None, Some(300.0)), 1.0);
        assert_eq!(flat_dark_scale(Some(5.0), None), 1.0);
        assert_eq!(flat_dark_scale(Some(5.0), Some(0.0)), 1.0);
    }

    #[test]
    fn test_subtract_bias() {
        let image =
            Array2::from_shape_vec((2, 2), vec![110.0, 120.0, 130.0, 140.0]).unwrap();
        let bias = Array2::from_shape_vec((2, 2), vec![10.0, 10.0, 10.0, 10.0]).unwrap();
        let result = subtract_bias(&image, &bias);
        assert!((result[[0, 0]] - 100.0).abs() < 1e-6);
        assert!((result[[1, 1]] - 130.0).abs() < 1e-6);
    }

    #[test]
    fn test_subtract_dark_with_ratio() {
        let image =
            Array2::from_shape_vec((2, 2), vec![200.0, 200.0, 200.0, 200.0]).unwrap();
        let dark = Array2::from_shape_vec((2, 2), vec![20.0, 20.0, 20.0, 20.0]).unwrap();
        let result = subtract_dark(&image, &dark, 2.0);
        assert!((result[[0, 0]] - 160.0).abs() < 1e-6);
    }

    #[test]
    fn test_divide_flat() {
        let image =
            Array2::from_shape_vec((2, 2), vec![100.0, 200.0, 300.0, 400.0]).unwrap();
        let flat = Array2::from_shape_vec((2, 2), vec![0.5, 1.0, 1.5, 2.0]).unwrap();
        let result = divide_flat(&image, &flat);
        assert!((result[[0, 0]] - 200.0).abs() < 1e-4);
        assert!((result[[0, 1]] - 200.0).abs() < 1e-4);
        assert!((result[[1, 0]] - 200.0).abs() < 1e-4);
        assert!((result[[1, 1]] - 200.0).abs() < 1e-4);
    }

    #[test]
    fn test_divide_flat_zero_safe() {
        let image =
            Array2::from_shape_vec((2, 2), vec![100.0, 200.0, 300.0, 400.0]).unwrap();
        let flat =
            Array2::from_shape_vec((2, 2), vec![0.0, 1.0, f32::NAN, 2.0]).unwrap();
        let result = divide_flat(&image, &flat);
        assert!((result[[0, 0]] - 100.0).abs() < 1e-4);
        assert!(result[[0, 1]].is_finite());
        assert!((result[[1, 0]] - 300.0).abs() < 1e-4);
    }

    #[test]
    fn test_full_calibration_pipeline() {
        let raw = Array2::from_shape_vec(
            (3, 3),
            vec![
                110.0, 120.0, 130.0, 140.0, 150.0, 160.0, 170.0, 180.0, 190.0,
            ],
        )
        .unwrap();
        let bias = Array2::from_shape_vec((3, 3), vec![10.0; 9]).unwrap();
        let dark = Array2::from_shape_vec((3, 3), vec![5.0; 9]).unwrap();
        let flat = Array2::from_shape_vec((3, 3), vec![1.0; 9]).unwrap();

        let config = CalibrationConfig {
            master_bias: Some(bias),
            master_dark: Some(dark),
            master_flat: Some(flat),
            dark_exposure_ratio: 1.0,
        };

        let result = calibrate_image(&raw, &config).unwrap();
        assert!((result[[0, 0]] - 95.0).abs() < 1e-4);
        assert!((result[[2, 2]] - 175.0).abs() < 1e-4);
    }

    #[test]
    fn test_calibrate_rejects_mismatched_master() {
        let raw = Array2::from_elem((4, 4), 100.0f32);
        let config = CalibrationConfig {
            master_bias: Some(Array2::from_elem((3, 3), 10.0f32)),
            master_dark: None,
            master_flat: None,
            dark_exposure_ratio: 1.0,
        };
        assert!(calibrate_image(&raw, &config).is_err());
    }

    struct Lcg(u64);

    impl Lcg {
        fn unit(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }

        fn gaussian(&mut self) -> f64 {
            let u1 = self.unit().max(1e-12);
            let u2 = self.unit();
            (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
        }
    }

    fn gaussian_frames(n: usize, rows: usize, cols: usize, mean: f32, sigma: f32, seed: u64) -> Vec<Array2<f32>> {
        let mut rng = Lcg(seed);
        (0..n)
            .map(|_| Array2::from_shape_fn((rows, cols), |_| mean + sigma * rng.gaussian() as f32))
            .collect()
    }

    fn sample_sigma(arr: &Array2<f32>) -> f64 {
        let n = arr.len() as f64;
        let mean = arr.iter().map(|&v| v as f64).sum::<f64>() / n;
        (arr.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    }

    #[test]
    fn master_config_default_is_winsorized_mean_with_pixinsight_sigmas() {
        let c = MasterConfig::default();
        assert_eq!(c.rejection, RejectionMethod::WinsorizedSigmaClip);
        assert_eq!(c.combine, CombineMethod::Mean);
        assert_eq!((c.sigma_low, c.sigma_high), (4.0, 3.0));
    }

    #[test]
    fn master_combine_removes_cosmic_ray_and_keeps_level() {
        let mut frames: Vec<Array2<f32>> = (0..7).map(|_| Array2::from_elem((4, 4), 100.0)).collect();
        frames[3][[1, 2]] = 60000.0;
        let master = combine_master_frames(&frames, &MasterConfig::default()).unwrap();
        assert_eq!(master.dim(), (4, 4));
        assert!((master[[1, 2]] - 100.0).abs() < 1e-3, "cosmic ray leaked: {}", master[[1, 2]]);
        assert!((master[[0, 0]] - 100.0).abs() < 1e-3);

        let plain_mean = MasterConfig { rejection: RejectionMethod::None, combine: CombineMethod::Mean, ..MasterConfig::default() };
        let leaked = combine_master_frames(&frames, &plain_mean).unwrap();
        assert!(leaked[[1, 2]] > 8000.0);
    }

    #[test]
    fn master_combine_reduces_noise_like_a_mean_not_a_median() {
        let frames = gaussian_frames(7, 64, 64, 1000.0, 1.0, 20260919);
        let mean_master = combine_master_frames(&frames, &MasterConfig::default()).unwrap();
        let median_only = MasterConfig { rejection: RejectionMethod::None, combine: CombineMethod::Median, ..MasterConfig::default() };
        let median_master = combine_master_frames(&frames, &median_only).unwrap();
        let expected = 1.0 / 7f64.sqrt();
        let s_mean = sample_sigma(&mean_master);
        let s_median = sample_sigma(&median_master);
        assert!((s_mean - expected).abs() < 0.15 * expected, "mean master sigma {s_mean} vs expected {expected}");
        assert!(s_median > s_mean * 1.1, "median master {s_median} should be noisier than mean master {s_mean}");
    }

    #[test]
    fn master_combine_rejects_shape_mismatch_and_empty_input() {
        let frames = vec![Array2::from_elem((2, 2), 1.0f32), Array2::from_elem((2, 3), 1.0f32)];
        assert!(combine_master_frames(&frames, &MasterConfig::default()).is_err());
        assert!(combine_master_frames(&[], &MasterConfig::default()).is_err());
    }

    #[test]
    fn flat_frames_are_scaled_to_first_median_before_combining() {
        let a = Array2::from_shape_vec((1, 4), vec![10.0, 20.0, 30.0, 40.0]).unwrap();
        let mut frames = vec![a.clone(), a.mapv(|v| v * 2.0), a.mapv(|v| v * 0.25)];
        scale_flats_to_first_median(&mut frames);
        assert_eq!(frames[0], a);
        for frame in &frames[1..] {
            for (x, y) in frame.iter().zip(a.iter()) {
                assert!((x - y).abs() < 1e-4, "{x} vs {y}");
            }
        }
    }

    #[test]
    fn master_bias_from_paths_uses_rejection_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..7 {
            let mut frame = Array2::from_elem((4, 4), 100.0f32);
            if i == 4 {
                frame[[2, 1]] = 60000.0;
            }
            let path = dir.path().join(format!("bias{i}.fits"));
            crate::infra::fits::writer::write_fits_mono(path.to_str().unwrap(), &frame, None).unwrap();
            paths.push(path.to_str().unwrap().to_string());
        }
        let master = create_master_bias(&paths).unwrap();
        assert!((master[[2, 1]] - 100.0).abs() < 1e-3);
        let plain_mean = MasterConfig { rejection: RejectionMethod::None, combine: CombineMethod::Mean, ..MasterConfig::default() };
        let leaked = create_master_bias_with(&paths, &plain_mean).unwrap();
        assert!(leaked[[2, 1]] > 8000.0);
    }
}
