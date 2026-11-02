use ndarray::Array2;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::cubeprep::cube::{Cube, DQ_DO_NOT_USE};
use crate::core::imaging::convolve::{convolve_fft, convolve_fft_with_variance, ConvolveError, ConvolveOptions, FftPlanCache, NanTreatment};
use crate::core::imaging::push_history;

pub const KERNEL_MISSING: &str = "kernel_missing";
pub const KERNEL_SCALE: &str = "kernel_scale";
pub const CONVOLUTION_FAILED: &str = "convolution_failed";
pub const KERNEL_SCALE_RTOL: f64 = 1e-3;
const MATCH_ERROR_NOTE: &str = "ERR uses squared kernel weights assuming independent input pixels; existing covariance from alignment or other processing is not included and output covariance is not stored";
const MATCH_DQ_NOTE: &str = "DQ is the bitwise OR of finite SCI contributors under every nonzero kernel coefficient, including negative lobes; flags do not exclude SCI";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    #[default]
    Astroburst,
    Lightstack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchOptions {
    pub reference: String,
    #[serde(default)]
    pub mode: MatchMode,
    #[serde(default)]
    pub allow_unmatched: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaneKernel {
    pub filter: String,
    pub label: String,
    pub data: Array2<f64>,
    pub pixel_scale_arcsec: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MatchError {
    #[error("kernel_missing: no matching kernel to {reference} for {}; build the kernels first or set allow_unmatched to copy those planes unmatched", filters.join(", "))]
    KernelMissing { reference: String, filters: Vec<String> },
    #[error("kernel_scale: the kernel {label} for {filter} is sampled at {kernel_arcsec} arcsec per pixel but the cube at {cube_arcsec}; build the kernels from PSFs on the cube grid")]
    KernelScale { filter: String, label: String, kernel_arcsec: f64, cube_arcsec: f64 },
    #[error("convolution_failed: {filter} with {label}: {error}")]
    Convolve { filter: String, label: String, error: ConvolveError },
}

impl MatchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::KernelMissing { .. } => KERNEL_MISSING,
            Self::KernelScale { .. } => KERNEL_SCALE,
            Self::Convolve { .. } => CONVOLUTION_FAILED,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaneStatus {
    Reference,
    Matched,
    Unmatched,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchedPlane {
    pub filter: String,
    pub status: PlaneStatus,
    pub kernel: Option<String>,
    pub masked_pixels: usize,
}

#[derive(Debug, Clone)]
pub struct MatchedCube {
    pub cube: Cube,
    pub planes: Vec<MatchedPlane>,
    pub warnings: Vec<String>,
}

pub fn lightstack_history(reference: &str) -> String {
    format!("PSF matched to {reference} using FFT convolution kernels")
}

fn same_filter(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn check_scales(cube: &Cube, kernels: &[Option<&PlaneKernel>], warnings: &mut Vec<String>) -> Result<(), MatchError> {
    let cube_scale = cube.wcs().ok().map(|wcs| wcs.pixel_scale_arcsec()).filter(|scale| scale.is_finite() && *scale > 0.0);
    for (plane, kernel) in cube.filters.iter().zip(kernels) {
        let Some(kernel) = kernel else { continue };
        match (kernel.pixel_scale_arcsec, cube_scale) {
            (Some(kernel_arcsec), Some(cube_arcsec)) if (kernel_arcsec / cube_arcsec - 1.0).abs() > KERNEL_SCALE_RTOL => {
                return Err(MatchError::KernelScale { filter: plane.filter.clone(), label: kernel.label.clone(), kernel_arcsec, cube_arcsec });
            }
            (Some(_), None) => warnings.push(format!("{}: the cube has no celestial WCS, so the pixel scale of {} was not checked", plane.filter, kernel.label)),
            (None, _) => warnings.push(format!(
                "{}: the kernel {} has no pixel scale (no PIXSCALE, PIXELSCL or celestial WCS card), so its sampling was not checked against the cube",
                plane.filter, kernel.label
            )),
            _ => {}
        }
    }
    Ok(())
}

struct PlaneOutput {
    sci: Array2<f32>,
    err: Option<Array2<f32>>,
    dq: Option<Array2<u32>>,
    warnings: Vec<String>,
}

struct PlaneInput<'a> {
    sci: &'a Array2<f32>,
    err: Option<&'a Array2<f32>>,
    dq: Option<&'a Array2<u32>>,
    no_data_dq: u32,
}

fn spread_support(mask: &Array2<f64>, support: &Array2<f64>, plans: &mut FftPlanCache<f64>) -> Result<Array2<f64>, ConvolveError> {
    let options = ConvolveOptions { normalize_kernel: false, nan_treatment: NanTreatment::Fill, fill_value: 0.0, ..ConvolveOptions::lightstack() };
    convolve_fft(mask, support, &options, plans).map(|result| result.image)
}

fn convolve_plane(
    input: PlaneInput<'_>,
    kernel: &PlaneKernel,
    filter: &str,
    options: &ConvolveOptions,
    plans: &mut FftPlanCache<f64>,
) -> Result<PlaneOutput, MatchError> {
    let image = input.sci.mapv(f64::from);
    let failure = |error| MatchError::Convolve { filter: filter.to_string(), label: kernel.label.clone(), error };
    let mut convolved = match input.err {
        Some(err) => convolve_fft_with_variance(&image, &err.mapv(|e| if e.is_finite() && e >= 0.0 { f64::from(e).powi(2) } else { f64::NAN }), &kernel.data, options, plans),
        None => convolve_fft(&image, &kernel.data, options, plans),
    }
    .map_err(failure)?;
    let support = kernel.data.mapv(|weight| if weight.is_finite() && weight != 0.0 { 1.0 } else { 0.0 });
    if let (Some(err), Some(variance)) = (input.err, convolved.variance.as_mut()) {
        let unknown = Array2::from_shape_fn(image.dim(), |p| {
            if image[p].is_finite() && !(err[p].is_finite() && err[p] >= 0.0) { 1.0 } else { 0.0 }
        });
        if unknown.iter().any(|&hit| hit != 0.0) {
            let spread = spread_support(&unknown, &support, plans).map_err(failure)?;
            for (value, hits) in variance.iter_mut().zip(spread) {
                if hits > 0.5 {
                    *value = f64::NAN;
                }
            }
        }
    }
    let dq = input.dq.map(|source| -> Result<Array2<u32>, MatchError> {
        let mut output = Array2::zeros(source.dim());
        let mut flags = source.indexed_iter().filter(|(p, _)| image[*p].is_finite()).fold(0u32, |bits, (_, &value)| bits | value);
        while flags != 0 {
            let bit = 1u32 << flags.trailing_zeros();
            let mask = Array2::from_shape_fn(source.dim(), |p| if image[p].is_finite() && source[p] & bit != 0 { 1.0 } else { 0.0 });
            let spread = spread_support(&mask, &support, plans).map_err(failure)?;
            for (value, hits) in output.iter_mut().zip(spread) {
                if hits > 0.5 {
                    *value |= bit;
                }
            }
            flags &= !bit;
        }
        for (p, &value) in convolved.image.indexed_iter() {
            if !value.is_finite() {
                output[p] |= input.no_data_dq;
                if !image[p].is_finite() {
                    output[p] |= source[p];
                }
            }
        }
        Ok(output)
    }).transpose()?;
    Ok(PlaneOutput {
        sci: convolved.image.mapv(|v| v as f32),
        err: convolved.variance.map(|variance| variance.mapv(|v| v.sqrt() as f32)),
        dq,
        warnings: convolved.report.warnings.iter().map(|warning| format!("{filter}: {warning}")).collect(),
    })
}

pub fn match_cube(cube: &Cube, kernels: &[PlaneKernel], options: &MatchOptions, dq_flags: &[u32]) -> Result<MatchedCube, MatchError> {
    let reference = options.reference.trim();
    let assigned: Vec<(PlaneStatus, Option<&PlaneKernel>)> = cube
        .filters
        .iter()
        .map(|plane| {
            if same_filter(&plane.filter, reference) {
                (PlaneStatus::Reference, None)
            } else {
                kernels
                    .iter()
                    .find(|kernel| same_filter(&kernel.filter, &plane.filter))
                    .map_or((PlaneStatus::Unmatched, None), |kernel| (PlaneStatus::Matched, Some(kernel)))
            }
        })
        .collect();
    let missing: Vec<String> =
        cube.filters.iter().zip(&assigned).filter(|(_, (status, _))| *status == PlaneStatus::Unmatched).map(|(plane, _)| plane.filter.clone()).collect();
    if !missing.is_empty() && !options.allow_unmatched {
        return Err(MatchError::KernelMissing { reference: reference.to_string(), filters: missing });
    }
    let mut warnings = Vec::new();
    let chosen: Vec<Option<&PlaneKernel>> = assigned.iter().map(|(_, kernel)| *kernel).collect();
    check_scales(cube, &chosen, &mut warnings)?;
    if !assigned.iter().any(|(status, _)| *status == PlaneStatus::Reference) {
        warnings.push(format!("the reference filter {reference} is not a plane of the cube, so every plane was convolved"));
    }
    let convolve_options = match options.mode {
        MatchMode::Astroburst => ConvolveOptions { preserve_nan: true, ..ConvolveOptions::lightstack() },
        MatchMode::Lightstack => ConvolveOptions::lightstack(),
    };
    let outputs = cube
        .planes
        .par_iter()
        .enumerate()
        .map_init(FftPlanCache::<f64>::new, |plans, (z, sci)| match assigned[z].1 {
            Some(kernel) => {
                let err = cube.err.as_ref().map(|err| &err[z]);
                let dq = cube.dq.as_ref().map(|dq| &dq.planes[z]);
                let input = PlaneInput { sci, err, dq, no_data_dq: dq_flags.get(z).copied().unwrap_or(DQ_DO_NOT_USE) };
                convolve_plane(input, kernel, &cube.filters[z].filter, &convolve_options, plans).map(Some)
            }
            None => Ok(None),
        })
        .collect::<Result<Vec<Option<PlaneOutput>>, MatchError>>()?;

    let mut matched = cube.clone();
    for (z, output) in outputs.into_iter().enumerate() {
        if let Some(output) = output {
            matched.planes[z] = output.sci;
            if let (Some(err), Some(propagated)) = (matched.err.as_mut(), output.err) {
                err[z] = propagated;
            }
            if let (Some(dq), Some(propagated)) = (matched.dq.as_mut(), output.dq) {
                dq.planes[z] = propagated;
            }
            warnings.extend(output.warnings);
        }
    }
    let union = (options.mode == MatchMode::Astroburst)
        .then(|| Array2::from_shape_fn(cube.dims(), |p| cube.planes.iter().any(|plane| !plane[p].is_finite())));
    let mut planes = Vec::with_capacity(cube.planes.len());
    for (z, (status, kernel)) in assigned.iter().enumerate() {
        let mut masked_pixels = 0;
        if let (Some(mask), false) = (&union, *status == PlaneStatus::Reference) {
            let flag = dq_flags.get(z).copied().unwrap_or(DQ_DO_NOT_USE);
            for ((p, &hit), value) in mask.indexed_iter().zip(matched.planes[z].iter_mut()) {
                if !hit {
                    continue;
                }
                if !value.is_nan() {
                    *value = f32::NAN;
                    masked_pixels += 1;
                    if let Some(dq) = matched.dq.as_mut() {
                        dq.planes[z][p] |= flag;
                    }
                }
                if let Some(err) = matched.err.as_mut() {
                    err[z][p] = f32::NAN;
                }
            }
        }
        let filter = cube.filters[z].filter.clone();
        if *status == PlaneStatus::Unmatched {
            warnings.push(format!("{filter}: no kernel to {reference}, so the plane was left unmatched (allow_unmatched)"));
        }
        if let Some(kernel) = kernel {
            matched.filters[z].kernel = kernel.label.clone();
        }
        planes.push(MatchedPlane { filter, status: *status, kernel: kernel.map(|kernel| kernel.label.clone()), masked_pixels });
    }

    let header = &mut matched.header;
    if assigned.iter().any(|(status, _)| *status == PlaneStatus::Matched) {
        if cube.err.is_some() {
            warnings.push(MATCH_ERROR_NOTE.to_string());
            push_history(header, MATCH_ERROR_NOTE);
        }
        if cube.dq.is_some() {
            warnings.push(MATCH_DQ_NOTE.to_string());
            push_history(header, MATCH_DQ_NOTE);
        }
    }
    match options.mode {
        MatchMode::Lightstack => push_history(header, &lightstack_history(reference)),
        MatchMode::Astroburst => push_history(
            header,
            &format!("PSF matched to {reference} with FFT convolution kernels: NaN pixels interpolated under the kernel and kept NaN, then the union NaN mask of the input planes applied to every plane but the reference, which is unchanged"),
        ),
    }
    for plane in &planes {
        match (&plane.kernel, plane.status) {
            (Some(label), _) => push_history(header, &format!("{} convolved with {label}", plane.filter)),
            (None, PlaneStatus::Unmatched) => push_history(header, &format!("{} left unmatched: no kernel to {reference}", plane.filter)),
            _ => {}
        }
    }
    Ok(MatchedCube { cube: matched, planes, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cubeprep::cube::test_support::small_cube;
    use crate::core::cubeprep::cube::{DqPlanes, DQ_DO_NOT_USE};
    use crate::core::cubeprep::kernel::{build_kernel, kernel_file_name, KernelOptions};
    use crate::core::cubeprep::psf_io::fit_gaussian;
    use crate::core::imaging::convolve::reference::direct_convolve;
    use crate::core::imaging::convolve::{convolve_fft_with_variance, ConvolveOptions, FftPlanCache};

    const FILTERS: [(&str, &str); 3] = [("JWST", "F150W"), ("JWST", "F277W"), ("JWST", "F444W")];
    const DIMS: (usize, usize) = (40, 48);
    const F_CLASS: f64 = 2e-6;
    const MODES: [MatchMode; 2] = [MatchMode::Astroburst, MatchMode::Lightstack];

    fn options(mode: MatchMode) -> MatchOptions {
        MatchOptions { reference: "F444W".to_string(), mode, allow_unmatched: false }
    }

    fn gaussian(rows: usize, cols: usize, sigma: f64) -> Array2<f64> {
        let (cy, cx) = ((rows / 2) as f64, (cols / 2) as f64);
        let psf = Array2::from_shape_fn((rows, cols), |(r, c)| (-((c as f64 - cx).powi(2) + (r as f64 - cy).powi(2)) / (2.0 * sigma * sigma)).exp());
        let total = psf.sum();
        psf / total
    }

    fn lobed() -> Array2<f64> {
        let mut kernel = gaussian(7, 7, 1.1) * 1.3;
        for (r, c) in [(0, 3), (3, 0), (6, 3), (3, 6), (1, 1), (5, 5)] {
            kernel[[r, c]] -= 0.05;
        }
        let total = kernel.sum();
        kernel / total
    }

    fn kernel(filter: &str, data: Array2<f64>) -> PlaneKernel {
        PlaneKernel { filter: filter.to_string(), label: kernel_file_name(filter, "F444W"), data, pixel_scale_arcsec: Some(0.04) }
    }

    fn scene(z: usize, y: usize, x: usize) -> f32 {
        let sigma = [1.0, 1.4, 2.0][z];
        let star = 50.0 * (-((x as f64 - 20.3).powi(2) + (y as f64 - 17.6).powi(2)) / (2.0 * sigma * sigma)).exp();
        (1.0 + 0.02 * x as f64 + 0.03 * y as f64 + z as f64 + star) as f32
    }

    fn same_bits(a: &Array2<f32>, b: &Array2<f32>) -> bool {
        a.dim() == b.dim() && a.iter().zip(b.iter()).all(|(x, y)| x.to_bits() == y.to_bits())
    }

    fn assert_f_class(got: &Array2<f32>, expected: &Array2<f32>, what: &str) {
        assert_eq!(got.dim(), expected.dim(), "{what}: shape");
        let scale = expected.iter().filter(|v| v.is_finite()).fold(0.0f64, |acc, &v| acc.max(f64::from(v).abs()));
        let mut worst = 0.0f64;
        for ((index, g), e) in got.indexed_iter().zip(expected.iter()) {
            assert_eq!(g.is_nan(), e.is_nan(), "{what}: NaN pattern at {index:?}: {g} against {e}");
            if e.is_finite() {
                worst = worst.max((f64::from(*g) - f64::from(*e)).abs());
            }
        }
        assert!(worst <= F_CLASS * scale, "{what}: max difference {worst:e} against {:e}", F_CLASS * scale);
    }

    fn history(cube: &Cube) -> String {
        cube.header.cards.iter().filter(|(key, _)| key == "HISTORY").map(|(_, value)| value.as_str()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn missing_kernel_fails_with_code() {
        let cube = small_cube(&FILTERS, DIMS, scene);
        let kernels = [kernel("F150W", gaussian(7, 7, 1.0))];
        for mode in MODES {
            let error = match_cube(&cube, &kernels, &options(mode), &[]).unwrap_err();
            assert_eq!(error.code(), KERNEL_MISSING);
            assert_eq!(error, MatchError::KernelMissing { reference: "F444W".to_string(), filters: vec!["F277W".to_string()] });
            assert!(error.to_string().starts_with("kernel_missing: ") && error.to_string().contains("F277W"), "{error}");

            let allowed = match_cube(&cube, &kernels, &MatchOptions { allow_unmatched: true, ..options(mode) }, &[]).unwrap();
            let statuses: Vec<PlaneStatus> = allowed.planes.iter().map(|plane| plane.status).collect();
            assert_eq!(statuses, [PlaneStatus::Matched, PlaneStatus::Unmatched, PlaneStatus::Reference], "{mode:?}");
            assert_eq!(allowed.planes[1].kernel, None);
            let unmatched: Vec<&String> = allowed.warnings.iter().filter(|warning| warning.contains("unmatched")).collect();
            assert!(unmatched.len() == 1 && unmatched[0].contains("F277W"), "{mode:?}: {:?}", allowed.warnings);
            assert!(history(&allowed.cube).contains("F277W"), "{mode:?}: the unmatched plane is recorded in HISTORY");

            let none = match_cube(&cube, &[], &MatchOptions { allow_unmatched: true, ..options(mode) }, &[]).unwrap();
            let per_plane: Vec<&String> = none.warnings.iter().filter(|warning| warning.contains("unmatched")).collect();
            assert_eq!(per_plane.len(), 2, "{mode:?}: one warning per unmatched plane: {:?}", none.warnings);
            assert!(per_plane[0].contains("F150W") && per_plane[1].contains("F277W"), "{per_plane:?}");
        }
        let allowed = match_cube(&cube, &kernels, &MatchOptions { allow_unmatched: true, ..options(MatchMode::Lightstack) }, &[]).unwrap();
        assert!(same_bits(&allowed.cube.planes[1], &cube.planes[1]), "LIGHTSTACK copies an unmatched plane unchanged");
    }

    #[test]
    fn reference_plane_is_bitwise_unchanged() {
        let mut cube = small_cube(&FILTERS, DIMS, scene);
        cube.planes[2][[5, 5]] = f32::from_bits(0x7fc0_1234);
        cube.planes[0][[10, 12]] = f32::NAN;
        cube.planes[1][[30, 40]] = f32::NAN;
        cube.err = Some(cube.planes.iter().map(|plane| plane.mapv(|v| 0.05 * v.abs().sqrt() + 0.01)).collect());
        cube.dq = Some(DqPlanes { planes: (0..3).map(|z| Array2::from_shape_fn(DIMS, |(y, x)| u32::from((x + y + z) % 7 == 0) * 4)).collect(), signed: false });
        let kernels = [kernel("F150W", lobed()), kernel("F277W", gaussian(9, 9, 1.4))];
        let before = cube.clone();
        for mode in MODES {
            let matched = match_cube(&cube, &kernels, &options(mode), &[512, 512, 512]).unwrap();
            let out = &matched.cube;
            assert!(same_bits(&out.planes[2], &before.planes[2]), "{mode:?}: reference SCI");
            assert!(same_bits(&out.err.as_ref().unwrap()[2], &before.err.as_ref().unwrap()[2]), "{mode:?}: reference ERR");
            assert_eq!(out.dq.as_ref().unwrap().planes[2], before.dq.as_ref().unwrap().planes[2], "{mode:?}: reference DQ");
            assert_eq!(out.filters[2], before.filters[2], "{mode:?}: reference metadata");
            assert_eq!(matched.planes[2], MatchedPlane { filter: "F444W".to_string(), status: PlaneStatus::Reference, kernel: None, masked_pixels: 0 });
            assert!(!same_bits(&out.planes[0], &before.planes[0]) && !same_bits(&out.planes[1], &before.planes[1]), "{mode:?}: the other planes are convolved");
            assert_eq!(out.filters[0].kernel, "kernel_f150w_to_f444w.fits");
        }

        let matched = match_cube(&cube, &kernels, &options(MatchMode::Astroburst), &[512, 512, 512]).unwrap();
        let out = &matched.cube;
        for z in 0..2 {
            for hole in [(5, 5), (10, 12), (30, 40)] {
                assert!(out.planes[z][hole].is_nan(), "plane {z} {hole:?}: the union NaN mask covers every plane but the reference");
                assert!(out.err.as_ref().unwrap()[z][hole].is_nan(), "plane {z} {hole:?}: ERR follows the mask");
            }
        }
        assert!(out.planes[2][[10, 12]].is_finite() && out.planes[2][[30, 40]].is_finite(), "the reference keeps its own pixels");
        assert_eq!((matched.planes[0].masked_pixels, matched.planes[1].masked_pixels), (2, 2), "newly masked pixels per plane");
        let dq = &out.dq.as_ref().unwrap().planes;
        assert_eq!((dq[0][[5, 5]] & 512, dq[0][[30, 40]] & 512, dq[0][[10, 12]] & 512), (512, 512, 512), "the DQ flag marks every invalid convolved SCI pixel");
        assert_eq!(dq[0][[0, 0]], before.dq.as_ref().unwrap().planes[0][[0, 0]]);
        assert!(history(out).contains("union"), "{}", history(out));

        let defaults = match_cube(&cube, &kernels, &options(MatchMode::Astroburst), &[]).unwrap();
        assert_eq!(defaults.cube.dq.as_ref().unwrap().planes[1][[5, 5]] & DQ_DO_NOT_USE, DQ_DO_NOT_USE);
    }

    #[test]
    fn matched_point_sources_reach_reference_fwhm() {
        let sigmas = [1.2, 1.6, 2.2];
        let stars = [(12.3, 14.6, 40.0), (27.8, 31.2, 25.0)];
        let dims = (44, 48);
        let cube = small_cube(&FILTERS, dims, |z, y, x| {
            stars.iter().fold(0.0, |sum, &(sy, sx, flux)| {
                let variance = sigmas[z] * sigmas[z];
                sum + flux / (2.0 * std::f64::consts::PI * variance) * (-((x as f64 - sx).powi(2) + (y as f64 - sy).powi(2)) / (2.0 * variance)).exp()
            }) as f32
        });
        let auto = KernelOptions { auto: true, ..KernelOptions::default() };
        let kernels: Vec<PlaneKernel> = (0..2)
            .map(|z| kernel(FILTERS[z].1, build_kernel(&gaussian(21, 21, sigmas[z]), &gaussian(21, 21, sigmas[2]), &auto).unwrap().data))
            .collect();
        let matched = match_cube(&cube, &kernels, &options(MatchMode::Astroburst), &[]).unwrap();
        let cutout = |plane: &Array2<f32>, y: f64, x: f64| {
            let (cy, cx) = (y.round() as usize, x.round() as usize);
            plane.slice(ndarray::s![cy - 7..=cy + 7, cx - 7..=cx + 7]).mapv(f64::from)
        };
        let aperture = |plane: &Array2<f32>, y: f64, x: f64, radius: f64| {
            plane.indexed_iter().filter(|((r, c), _)| (*c as f64 - x).hypot(*r as f64 - y) <= radius).map(|(_, v)| f64::from(*v)).sum::<f64>()
        };
        for &(y, x, _) in &stars {
            let reference = fit_gaussian(&cutout(&matched.cube.planes[2], y, x)).unwrap().fwhm_px;
            let radius = 1.5 * reference;
            let reference_flux = aperture(&matched.cube.planes[2], y, x, radius);
            for z in 0..2 {
                let before = fit_gaussian(&cutout(&cube.planes[z], y, x)).unwrap().fwhm_px;
                let after = fit_gaussian(&cutout(&matched.cube.planes[z], y, x)).unwrap().fwhm_px;
                assert!(before < 0.8 * reference, "plane {z}: the source starts narrower ({before} px)");
                assert!((after / reference - 1.0).abs() < 0.02, "plane {z} star at ({x}, {y}): matched FWHM {after} px against the reference {reference} px");
                let flux = aperture(&matched.cube.planes[z], y, x, radius);
                assert!((flux / reference_flux - 1.0).abs() < 0.005, "plane {z} star at ({x}, {y}): aperture flux {flux} against {reference_flux}");
            }
        }
    }

    #[test]
    fn lightstack_compat_reproduces_psf_match_datacube() {
        let mut cube = small_cube(&FILTERS, DIMS, scene);
        cube.planes[0][[10, 12]] = f32::NAN;
        cube.planes[0][[11, 12]] = f32::NAN;
        cube.planes[1][[0, 0]] = f32::NAN;
        cube.planes[2][[20, 20]] = f32::NAN;
        let kernels = [kernel("f150w", lobed()), kernel("F277W", gaussian(9, 9, 1.4))];
        let compat = MatchOptions { reference: "f444w".to_string(), mode: MatchMode::Lightstack, allow_unmatched: false };
        let matched = match_cube(&cube, &kernels, &compat, &[]).unwrap();
        for z in 0..2 {
            let expected = direct_convolve(&cube.planes[z].mapv(f64::from), &kernels[z].data, &ConvolveOptions::lightstack()).mapv(|v| v as f32);
            assert_f_class(&matched.cube.planes[z], &expected, &format!("plane {z} against astropy's convolve_fft(normalize_kernel=False)"));
            assert!(matched.cube.planes[z].iter().all(|v| v.is_finite()), "plane {z}: LIGHTSTACK interpolates every NaN");
        }
        assert!(same_bits(&matched.cube.planes[2], &cube.planes[2]) && matched.cube.planes[2][[20, 20]].is_nan(), "the reference plane is copied");
        assert_eq!(matched.planes.iter().map(|plane| plane.status).collect::<Vec<_>>(), [PlaneStatus::Matched, PlaneStatus::Matched, PlaneStatus::Reference]);
        assert!(history(&matched.cube).contains(&lightstack_history("f444w")), "{}", history(&matched.cube));
        assert_eq!(lightstack_history("F444W"), "PSF matched to F444W using FFT convolution kernels");
        assert_eq!(matched.planes[0].kernel.as_deref(), Some("kernel_f150w_to_f444w.fits"));
        assert!(matched.warnings.iter().all(|warning| !warning.contains("unmatched")), "{:?}", matched.warnings);

        let ours = match_cube(&cube, &kernels, &MatchOptions { mode: MatchMode::Astroburst, ..compat.clone() }, &[]).unwrap();
        assert!(ours.cube.planes[0][[10, 12]].is_nan() && ours.cube.planes[1][[0, 0]].is_nan(), "the AstroBurst mode keeps NaN");
        let preserved = ConvolveOptions { preserve_nan: true, ..ConvolveOptions::lightstack() };
        let union = |plane: &Array2<f32>| {
            let mut masked = plane.clone();
            for (index, value) in masked.indexed_iter_mut() {
                if cube.planes.iter().any(|plane| plane[index].is_nan()) {
                    *value = f32::NAN;
                }
            }
            masked
        };
        for z in 0..2 {
            let expected = direct_convolve(&cube.planes[z].mapv(f64::from), &kernels[z].data, &preserved).mapv(|v| v as f32);
            assert_f_class(&ours.cube.planes[z], &union(&expected), &format!("plane {z} in the AstroBurst mode"));
        }
    }

    #[test]
    fn matched_err_is_propagated_through_the_squared_kernel() {
        let mut cube = small_cube(&FILTERS, DIMS, scene);
        cube.planes[1][[8, 9]] = f32::NAN;
        let err: Vec<Array2<f32>> = cube.planes.iter().map(|plane| plane.mapv(|v| 0.1 * v.abs().sqrt())).collect();
        cube.err = Some(err.clone());
        let kernels = [kernel("F150W", lobed()), kernel("F277W", gaussian(9, 9, 1.4))];
        for mode in MODES {
            let matched = match_cube(&cube, &kernels, &options(mode), &[]).unwrap();
            let convolve_options = match mode {
                MatchMode::Astroburst => ConvolveOptions { preserve_nan: true, ..ConvolveOptions::lightstack() },
                MatchMode::Lightstack => ConvolveOptions::lightstack(),
            };
            for z in 0..2 {
                let variance = err[z].mapv(|e| f64::from(e) * f64::from(e));
                let propagated = convolve_fft_with_variance(&cube.planes[z].mapv(f64::from), &variance, &kernels[z].data, &convolve_options, &mut FftPlanCache::new()).unwrap();
                let mut expected = propagated.variance.unwrap().mapv(|v| v.sqrt() as f32);
                if mode == MatchMode::Astroburst {
                    expected[[8, 9]] = f32::NAN;
                }
                assert_f_class(&matched.cube.err.as_ref().unwrap()[z], &expected, &format!("{mode:?} ERR of plane {z}"));
            }
        }
    }

    #[test]
    fn matched_companions_keep_invalid_err_unknown_on_every_nonzero_kernel_contributor() {
        let mut cube = small_cube(&FILTERS, (12, 12), |_, _, _| 2.0);
        let mut shape = Array2::zeros((3, 3));
        shape[[1, 1]] = 1.25;
        shape[[1, 0]] = -0.25;
        let kernels = [kernel("F150W", shape), kernel("F277W", Array2::from_elem((1, 1), 1.0))];
        for invalid in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut err = (0..3).map(|_| Array2::from_elem((12, 12), 0.5)).collect::<Vec<_>>();
            err[0][[5, 5]] = invalid;
            cube.err = Some(err);
            for mode in MODES {
                let result = match_cube(&cube, &kernels, &options(mode), &[]).unwrap();
                let err = &result.cube.err.as_ref().unwrap()[0];
                assert!(err[[5, 5]].is_nan() && err[[5, 4]].is_nan(), "{mode:?}, {invalid}: {}, {}", err[[5, 5]], err[[5, 4]]);
                assert!(err[[5, 6]].is_finite() && err[[4, 5]].is_finite());
                assert!(result.cube.planes[0][[5, 5]].is_finite());
                assert!(result.warnings.iter().any(|note| note.contains("covariance")));
            }
        }
    }

    #[test]
    fn matched_companions_propagate_dq_by_nonzero_kernel_support_including_negative_lobes() {
        let mut cube = small_cube(&FILTERS, (12, 12), |_, _, _| 2.0);
        let mut flags = (0..3).map(|_| Array2::zeros((12, 12))).collect::<Vec<_>>();
        flags[0][[5, 5]] = 1u32 << 31;
        flags[0][[5, 6]] = 4;
        cube.dq = Some(DqPlanes { planes: flags, signed: false });
        let mut shape = Array2::zeros((3, 3));
        shape[[1, 1]] = 1.25;
        shape[[1, 0]] = -0.25;
        let kernels = [kernel("F150W", shape), kernel("F277W", Array2::from_elem((1, 1), 1.0))];
        for mode in MODES {
            let result = match_cube(&cube, &kernels, &options(mode), &[]).unwrap();
            let flags = &result.cube.dq.as_ref().unwrap().planes[0];
            assert_eq!(flags[[5, 4]], 1u32 << 31, "{mode:?}");
            assert_eq!(flags[[5, 5]], (1u32 << 31) | 4, "{mode:?}");
            assert_eq!(flags[[5, 6]], 4);
            assert_eq!(flags[[5, 7]], 0);
            assert_eq!(flags[[4, 5]], 0);
        }
    }

    #[test]
    fn matched_companions_support_counts_match_direct_flags_for_even_kernels_and_tiny_lobes() {
        let mut cube = small_cube(&FILTERS, (9, 11), |_, _, _| 2.0);
        cube.planes[0][[2, 3]] = f32::NAN;
        cube.err = Some((0..3).map(|_| Array2::from_elem((9, 11), 0.5)).collect());
        cube.err.as_mut().unwrap()[0][[4, 5]] = f32::NAN;
        cube.dq = Some(DqPlanes {
            planes: (0..3).map(|z| Array2::from_shape_fn((9, 11), |(y, x)| (1u32 << ((x + 3 * y + z) % 32)) | u32::from((x + y) % 5 == 0))).collect(),
            signed: false,
        });
        let mut shape = Array2::zeros((2, 4));
        shape[[1, 2]] = 1.0;
        shape[[0, 0]] = 1e-12;
        shape[[1, 3]] = -0.1;
        let kernels = [kernel("F150W", shape.clone()), kernel("F277W", Array2::from_elem((1, 1), 1.0))];
        for mode in MODES {
            let result = match_cube(&cube, &kernels, &options(mode), &[513; 3]).unwrap();
            let actual = &result.cube.dq.as_ref().unwrap().planes[0];
            for ((y, x), &value) in result.cube.planes[0].indexed_iter() {
                if !value.is_finite() {
                    assert_eq!(actual[[y, x]] & 513, 513);
                    continue;
                }
                let mut expected = 0;
                for ((ky, kx), &weight) in shape.indexed_iter() {
                    let (sy, sx) = (y as isize + 1 - ky as isize, x as isize + 2 - kx as isize);
                    if weight != 0.0 && sy >= 0 && sx >= 0 && sy < 9 && sx < 11 && cube.planes[0][[sy as usize, sx as usize]].is_finite() {
                        expected |= cube.dq.as_ref().unwrap().planes[0][[sy as usize, sx as usize]];
                    }
                }
                assert_eq!(actual[[y, x]], expected, "{mode:?} at ({x}, {y})");
            }
            assert!(result.cube.err.as_ref().unwrap()[0][[3, 3]].is_nan(), "even the 1e-12 kernel lobe reaches unknown ERR");
        }
    }

    #[test]
    fn kernel_at_another_pixel_scale_is_refused() {
        let cube = small_cube(&FILTERS, DIMS, scene);
        let mut coarse = kernel("F277W", gaussian(9, 9, 1.4));
        coarse.pixel_scale_arcsec = Some(0.03);
        let kernels = [kernel("F150W", gaussian(7, 7, 1.0)), coarse.clone()];
        for mode in MODES {
            let error = match_cube(&cube, &kernels, &options(mode), &[]).unwrap_err();
            assert_eq!(error.code(), KERNEL_SCALE, "{mode:?}: {error}");
            assert!(error.to_string().contains("kernel_f277w_to_f444w.fits") && error.to_string().contains("0.03"), "{error}");
        }
        let unknown = [kernel("F150W", gaussian(7, 7, 1.0)), PlaneKernel { pixel_scale_arcsec: None, ..coarse.clone() }];
        let unchecked = match_cube(&cube, &unknown, &options(MatchMode::Astroburst), &[]).unwrap();
        assert!(
            unchecked.warnings.iter().any(|w| w.starts_with("F277W: ") && w.contains("kernel_f277w_to_f444w.fits") && w.contains("no pixel scale")),
            "a kernel without a pixel scale (as LIGHTSTACK writes them) is matched with a warning: {:?}",
            unchecked.warnings
        );
        let close = [kernel("F150W", gaussian(7, 7, 1.0)), PlaneKernel { pixel_scale_arcsec: Some(0.04 * (1.0 + 0.5 * KERNEL_SCALE_RTOL)), ..coarse }];
        assert!(match_cube(&cube, &close, &options(MatchMode::Astroburst), &[]).is_ok());
    }

    #[test]
    fn match_options_parse_the_recipe_step() {
        let parsed: MatchOptions = serde_json::from_value(serde_json::json!({ "step": "match", "reference": "F444W" })).unwrap();
        assert_eq!(parsed, options(MatchMode::Astroburst));
        let compat: MatchOptions = serde_json::from_value(serde_json::json!({ "reference": "F444W", "mode": "lightstack", "allow_unmatched": true })).unwrap();
        assert_eq!(compat, MatchOptions { allow_unmatched: true, ..options(MatchMode::Lightstack) });
    }
}
