use ndarray::{s, Array2};
use rayon::prelude::*;
use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::core::alignment::wcs_reproject::smallest_odd_support;
use crate::core::cubeprep::align::NoiseSource;
use crate::core::cubeprep::cube::{Cube, CubeNoise, MatchStatus, PlaneNoise, DQ_DO_NOT_USE};
use crate::core::imaging::convolve::{convolve_fft, convolve_fft_with_variance, padded_len, ConvolveError, ConvolveOptions, FftPlanCache, PreparedSpectrum};
use crate::core::imaging::push_history;
use crate::core::stacking::memory::{check_memory, effective_memory, format_bytes, MemoryInfo, MemoryJob};
use crate::math::median::exact_median_f64;
use crate::types::header::HduHeader;

pub const KERNEL_MISSING: &str = "kernel_missing";
pub const KERNEL_SCALE: &str = "kernel_scale";
pub const CONVOLUTION_FAILED: &str = "convolution_failed";
pub const INSUFFICIENT_MEMORY: &str = "insufficient_memory";
pub const CANCELLED: &str = "cancelled";
pub const KERNEL_SCALE_RTOL: f64 = 1e-3;
pub const CARD_SRCFWHM: &str = "SRCFWHM";
pub const CARD_TGTFWHM: &str = "TGTFWHM";
pub const CARD_MATFWHM: &str = "MATFWHM";
pub const CARD_KSUMSQ: &str = "KSUMSQ";
const EDGE_RING_PX: usize = 2;
const EDGE_RING_SIGMAS: f64 = 3.0;
const MAD_TO_SIGMA: f64 = 1.4826;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KernelCards {
    pub source_fwhm_px: f64,
    pub target_fwhm_px: f64,
    pub matched_fwhm_px: f64,
    pub sum_squares: f64,
}

impl KernelCards {
    pub const ABSENT: Self = Self { source_fwhm_px: f64::NAN, target_fwhm_px: f64::NAN, matched_fwhm_px: f64::NAN, sum_squares: f64::NAN };

    pub fn read(header: &HduHeader) -> Self {
        let card = |key: &str| header.get_f64(key).unwrap_or(f64::NAN);
        Self { source_fwhm_px: card(CARD_SRCFWHM), target_fwhm_px: card(CARD_TGTFWHM), matched_fwhm_px: card(CARD_MATFWHM), sum_squares: card(CARD_KSUMSQ) }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MatchError {
    #[error("kernel_missing: no matching kernel to {reference} for {}; build the kernels first or set allow_unmatched to copy those planes unmatched", filters.join(", "))]
    KernelMissing { reference: String, filters: Vec<String> },
    #[error("kernel_scale: the kernel {label} for {filter} is sampled at {kernel_arcsec} arcsec per pixel but the cube at {cube_arcsec}; build the kernels from PSFs on the cube grid")]
    KernelScale { filter: String, label: String, kernel_arcsec: f64, cube_arcsec: f64 },
    #[error("convolution_failed: {filter} with {label}: {error}")]
    Convolve { filter: String, label: String, error: ConvolveError },
    #[error("insufficient_memory: {message}")]
    InsufficientMemory { rows: usize, cols: usize, planes: usize, bytes: u64, message: String },
    #[error("cancelled: PSF matching stopped at the cancel request")]
    Cancelled,
}

impl MatchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::KernelMissing { .. } => KERNEL_MISSING,
            Self::KernelScale { .. } => KERNEL_SCALE,
            Self::Convolve { .. } => CONVOLUTION_FAILED,
            Self::InsufficientMemory { .. } => INSUFFICIENT_MEMORY,
            Self::Cancelled => CANCELLED,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DqSpread {
    pub bit: u32,
    pub flagged: usize,
    pub spread: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchedPlane {
    pub filter: String,
    pub status: PlaneStatus,
    pub kernel: Option<String>,
    pub masked_pixels: usize,
    pub boundary_band_px: Option<usize>,
    pub edge_kernel_mass: Option<f64>,
    pub dq_spread: Vec<DqSpread>,
}

#[derive(Debug, Clone)]
pub struct MatchedCube {
    pub cube: Cube,
    pub planes: Vec<MatchedPlane>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MatchProducts {
    pub matched: MatchedCube,
    pub noise: Option<CubeNoise>,
    pub statuses: Vec<MatchStatus>,
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
    dq_spread: Vec<DqSpread>,
}

struct PlaneInput<'a> {
    sci: &'a Array2<f32>,
    err: Option<&'a Array2<f32>>,
    dq: Option<&'a Array2<u32>>,
    no_data_dq: u32,
}

enum Spread {
    Dilation { rows: (usize, usize), cols: (usize, usize) },
    Fft(PreparedSpectrum),
}

impl Spread {
    fn new(kernel: &Array2<f64>, dims: (usize, usize), plans: &mut FftPlanCache<f64>) -> Self {
        let (rows, cols) = kernel.dim();
        let support = kernel.mapv(|weight| if weight.is_finite() && weight != 0.0 { 1.0 } else { 0.0 });
        if support.iter().all(|&inside| inside == 1.0) {
            Spread::Dilation { rows: (rows - 1 - rows / 2, rows / 2), cols: (cols - 1 - cols / 2, cols / 2) }
        } else {
            Spread::Fft(PreparedSpectrum::new(&support, dims, plans))
        }
    }

    fn apply(&mut self, bits: &Array2<u32>, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<Array2<u32>, MatchError> {
        match self {
            Spread::Dilation { rows, cols } => {
                if cancelled() {
                    return Err(MatchError::Cancelled);
                }
                let across = dilate_lines(bits, *cols);
                let down = dilate_lines(&across.t().as_standard_layout().into_owned(), *rows);
                Ok(down.t().as_standard_layout().into_owned())
            }
            Spread::Fft(prepared) => {
                let mut output = Array2::zeros(bits.dim());
                let mut pending = bits.iter().fold(0u32, |all, &value| all | value);
                while pending != 0 {
                    if cancelled() {
                        return Err(MatchError::Cancelled);
                    }
                    let low = pending & pending.wrapping_neg();
                    pending &= !low;
                    let high = pending & pending.wrapping_neg();
                    pending &= !high;
                    let hits = prepared.convolve_complex(|r, c| {
                        let value = bits[[r, c]];
                        Complex::new(f64::from(u8::from(value & low != 0)), f64::from(u8::from(value & high != 0)))
                    });
                    for (value, hit) in output.iter_mut().zip(hits.iter()) {
                        if hit.re > 0.5 {
                            *value |= low;
                        }
                        if hit.im > 0.5 {
                            *value |= high;
                        }
                    }
                }
                Ok(output)
            }
        }
    }
}

fn dilate_lines(plane: &Array2<u32>, (before, after): (usize, usize)) -> Array2<u32> {
    let cols = plane.ncols();
    let width = before + after + 1;
    let padded = cols + width - 1;
    let mut out = Array2::zeros(plane.dim());
    let source = plane.as_slice().expect("standard layout");
    out.as_slice_mut().expect("standard layout").par_chunks_mut(cols).zip(source.par_chunks(cols)).for_each_init(
        || (Vec::with_capacity(padded), vec![0u32; padded]),
        |(prefix, suffix), (line, input)| {
            let at = |t: usize| if t >= before && t - before < cols { input[t - before] } else { 0 };
            prefix.clear();
            let mut head = 0;
            for t in 0..padded {
                head = if t % width == 0 { at(t) } else { head | at(t) };
                prefix.push(head);
            }
            let mut tail = 0;
            for t in (0..padded).rev() {
                tail = if (t + 1) % width == 0 || t + 1 == padded { at(t) } else { tail | at(t) };
                suffix[t] = tail;
            }
            for (i, slot) in line.iter_mut().enumerate() {
                *slot = suffix[i] | prefix[i + width - 1];
            }
        },
    );
    out
}

fn bit_counts(plane: &Array2<u32>) -> [usize; 32] {
    let mut counts = [0; 32];
    for &value in plane {
        let mut bits = value;
        while bits != 0 {
            counts[bits.trailing_zeros() as usize] += 1;
            bits &= bits - 1;
        }
    }
    counts
}

fn boundary_band(kernel: &Array2<f64>) -> usize {
    kernel.nrows().max(kernel.ncols()) / 2
}

fn edge_kernel_mass(kernel: &Array2<f64>) -> f64 {
    let (rows, cols) = kernel.dim();
    let mass = |ys: std::ops::Range<usize>, xs: std::ops::Range<usize>| kernel.slice(s![ys, xs]).iter().filter(|weight| weight.is_finite()).sum::<f64>();
    [mass(rows / 2 + 1..rows, 0..cols), mass(0..rows / 2, 0..cols), mass(0..rows, cols / 2 + 1..cols), mass(0..rows, 0..cols / 2)].into_iter().fold(f64::NEG_INFINITY, f64::max)
}

fn edge_ring_warning(filter: &str, sci: &Array2<f32>, band: usize) -> Option<String> {
    let (rows, cols) = sci.dim();
    let ring: Vec<f64> = sci
        .indexed_iter()
        .filter(|&((y, x), value)| value.is_finite() && y.min(x).min(rows - 1 - y).min(cols - 1 - x) < EDGE_RING_PX)
        .map(|(_, value)| f64::from(*value))
        .collect();
    if ring.is_empty() {
        return None;
    }
    let median = exact_median_f64(&ring);
    let sigma = MAD_TO_SIGMA * exact_median_f64(&ring.iter().map(|value| (value - median).abs()).collect::<Vec<f64>>());
    (median.abs() > EDGE_RING_SIGMAS * sigma).then(|| {
        format!(
            "{filter}: the outer {EDGE_RING_PX}-px ring of the input has median {median:.4e}, more than {EDGE_RING_SIGMAS} robust sigma ({sigma:.4e}) from zero; the zero fill outside the field biases the pixels within {band} px of the edge, so subtract the background before matching"
        )
    })
}

fn matched_correlation(aligned: &Array2<f32>, kernel: &Array2<f64>) -> Array2<f32> {
    let weights = kernel.mapv(|weight| if weight.is_finite() { weight } else { 0.0 });
    let (rows, cols) = weights.dim();
    let overlap = |len: usize, shift: isize| (-shift).max(0) as usize..(len as isize - shift.max(0)) as usize;
    let autocorrelation = Array2::from_shape_fn((2 * rows - 1, 2 * cols - 1), |(i, j)| {
        let (dy, dx) = (i as isize - (rows as isize - 1), j as isize - (cols as isize - 1));
        let mut sum = 0.0;
        for y in overlap(rows, dy) {
            for x in overlap(cols, dx) {
                sum += weights[[y, x]] * weights[[(y as isize + dy) as usize, (x as isize + dx) as usize]];
            }
        }
        sum
    });
    let reach = rows.max(cols) - 1;
    let size = aligned.nrows() + 2 * reach;
    let (top, left) = (reach + 1 - rows, reach + 1 - cols);
    let mut correlation = Array2::<f64>::zeros((size, size));
    for ((qy, qx), &q) in aligned.indexed_iter().filter(|(_, q)| **q != 0.0) {
        for ((ay, ax), &a) in autocorrelation.indexed_iter() {
            correlation[[top + qy + ay, left + qx + ax]] += f64::from(q) * a;
        }
    }
    let centre = correlation[[size / 2, size / 2]];
    smallest_odd_support(&correlation.mapv(|value| (value / centre) as f32))
}

fn planes_in_flight(cube: &Cube, kernels: &[Option<&PlaneKernel>], memory: Option<MemoryInfo>, warnings: &mut Vec<String>) -> Result<usize, MatchError> {
    let matched = kernels.iter().flatten().count();
    let largest = kernels.iter().flatten().map(|kernel| kernel.data.dim()).reduce(|a, b| (a.0.max(b.0), a.1.max(b.1)));
    let (Some(info), Some((kernel_rows, kernel_cols))) = (memory, largest) else {
        return Ok(matched.max(1));
    };
    let planes = cube.planes.len();
    let (rows, cols) = cube.dims();
    let pixels = (rows * cols) as u64;
    let (complex_buffers, real_planes) = if cube.err.is_some() { (8, 4) } else { (4, 3) };
    let per_plane = 16 * (padded_len(rows, kernel_rows) * padded_len(cols, kernel_cols)) as u64 * complex_buffers + 8 * pixels * real_planes;
    let retained = 12 * pixels * matched as u64;
    let assembly = (12 * planes as u64 + 12 * matched as u64 + 1) * pixels;
    let peak = |in_flight: u64| (in_flight * per_plane + retained).max(assembly);
    let job = MemoryJob::PsfMatch { planes, rows, cols };
    check_memory(peak(1), Some(info), &job).map_err(|error| MatchError::InsufficientMemory { rows, cols, planes, bytes: peak(1), message: error.to_string() })?;
    let in_flight = (info.physical_available.saturating_sub(retained) / per_plane).clamp(1, matched as u64);
    if in_flight < matched as u64 {
        warnings.push(format!("PSF matching convolves {in_flight} of the {matched} matched planes at a time to fit {} of free memory", format_bytes(info.physical_available)));
    }
    if let Ok(Some(warning)) = check_memory(peak(in_flight), Some(info), &job) {
        warnings.push(warning);
    }
    Ok(in_flight as usize)
}

fn convolve_plane(
    input: PlaneInput<'_>,
    kernel: &PlaneKernel,
    filter: &str,
    options: &ConvolveOptions,
    plans: &mut FftPlanCache<f64>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PlaneOutput, MatchError> {
    let image = input.sci.mapv(f64::from);
    let failure = |error| MatchError::Convolve { filter: filter.to_string(), label: kernel.label.clone(), error };
    let known = |e: f32| e.is_finite() && e >= 0.0;
    let mut convolved = match input.err {
        Some(err) => convolve_fft_with_variance(&image, &err.mapv(|e| if known(e) { f64::from(e).powi(2) } else { 0.0 }), &kernel.data, options, plans),
        None => convolve_fft(&image, &kernel.data, options, plans),
    }
    .map_err(failure)?;
    if cancelled() {
        return Err(MatchError::Cancelled);
    }
    let mut spread = None;
    if let (Some(err), Some(variance)) = (input.err, convolved.variance.as_mut()) {
        let unknown = Array2::from_shape_fn(image.dim(), |p| u32::from(image[p].is_finite() && !known(err[p])));
        if unknown.iter().any(|&hit| hit != 0) {
            let hits = spread.get_or_insert_with(|| Spread::new(&kernel.data, image.dim(), plans)).apply(&unknown, cancelled)?;
            for (value, &hit) in variance.iter_mut().zip(hits.iter()) {
                if hit != 0 {
                    *value = f64::NAN;
                }
            }
            if cancelled() {
                return Err(MatchError::Cancelled);
            }
        }
    }
    let mut warnings: Vec<String> = convolved.report.warnings.iter().map(|warning| format!("{filter}: {warning}")).collect();
    let mut dq_spread = Vec::new();
    let dq = match input.dq {
        Some(source) => {
            let finite = Array2::from_shape_fn(source.dim(), |p| if image[p].is_finite() { source[p] } else { 0 });
            let mut output = spread.get_or_insert_with(|| Spread::new(&kernel.data, image.dim(), plans)).apply(&finite, cancelled)?;
            let (kernel_rows, kernel_cols) = kernel.data.dim();
            for (bit, (flagged, marked)) in bit_counts(&finite).into_iter().zip(bit_counts(&output)).enumerate().filter(|(_, (flagged, _))| *flagged > 0) {
                if marked > flagged {
                    warnings.push(format!("{filter}: bit {bit} on {flagged} finite pixels marks {marked} after the {kernel_cols}x{kernel_rows} kernel support"));
                }
                dq_spread.push(DqSpread { bit: bit as u32, flagged, spread: marked });
            }
            for (p, &value) in convolved.image.indexed_iter() {
                if !value.is_finite() {
                    output[p] |= input.no_data_dq;
                    if !image[p].is_finite() {
                        output[p] |= source[p];
                    }
                }
            }
            Some(output)
        }
        None => None,
    };
    Ok(PlaneOutput {
        sci: convolved.image.mapv(|v| v as f32),
        err: convolved.variance.map(|variance| variance.mapv(|v| v.sqrt() as f32)),
        dq,
        warnings,
        dq_spread,
    })
}

pub fn match_cube(cube: &Cube, kernels: &[PlaneKernel], options: &MatchOptions, dq_flags: &[u32]) -> Result<MatchedCube, MatchError> {
    match_cube_with(cube, kernels, &[], options, dq_flags, None, effective_memory(), &|| false).map(|products| products.matched)
}

#[allow(clippy::too_many_arguments)]
pub fn match_cube_with(
    cube: &Cube,
    kernels: &[PlaneKernel],
    cards: &[KernelCards],
    options: &MatchOptions,
    dq_flags: &[u32],
    noise: Option<&CubeNoise>,
    memory: Option<MemoryInfo>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<MatchProducts, MatchError> {
    let reference = options.reference.trim();
    let assigned: Vec<(PlaneStatus, Option<(&PlaneKernel, KernelCards)>)> = cube
        .filters
        .iter()
        .map(|plane| {
            if same_filter(&plane.filter, reference) {
                (PlaneStatus::Reference, None)
            } else {
                kernels.iter().position(|kernel| same_filter(&kernel.filter, &plane.filter)).map_or((PlaneStatus::Unmatched, None), |index| {
                    (PlaneStatus::Matched, Some((&kernels[index], cards.get(index).copied().unwrap_or(KernelCards::ABSENT))))
                })
            }
        })
        .collect();
    let missing: Vec<String> =
        cube.filters.iter().zip(&assigned).filter(|(_, (status, _))| *status == PlaneStatus::Unmatched).map(|(plane, _)| plane.filter.clone()).collect();
    if !missing.is_empty() && !options.allow_unmatched {
        return Err(MatchError::KernelMissing { reference: reference.to_string(), filters: missing });
    }
    let mut warnings = Vec::new();
    let chosen: Vec<Option<&PlaneKernel>> = assigned.iter().map(|(_, kernel)| kernel.map(|(kernel, _)| kernel)).collect();
    check_scales(cube, &chosen, &mut warnings)?;
    if !assigned.iter().any(|(status, _)| *status == PlaneStatus::Reference) {
        warnings.push(format!("the reference filter {reference} is not a plane of the cube, so every plane was convolved"));
    }
    let in_flight = planes_in_flight(cube, &chosen, memory, &mut warnings)?;
    let convolve_options = match options.mode {
        MatchMode::Astroburst => ConvolveOptions { preserve_nan: true, ..ConvolveOptions::lightstack() },
        MatchMode::Lightstack => ConvolveOptions::lightstack(),
    };
    let to_convolve: Vec<(usize, &PlaneKernel)> = chosen.iter().enumerate().filter_map(|(z, kernel)| kernel.map(|kernel| (z, kernel))).collect();
    let mut outputs: Vec<Option<PlaneOutput>> = cube.planes.iter().map(|_| None).collect();
    for chunk in to_convolve.chunks(in_flight) {
        let convolved = chunk
            .par_iter()
            .map_init(FftPlanCache::<f64>::new, |plans, &(z, kernel)| {
                if cancelled() {
                    return Err(MatchError::Cancelled);
                }
                let err = cube.err.as_ref().map(|err| &err[z]);
                let dq = cube.dq.as_ref().map(|dq| &dq.planes[z]);
                let input = PlaneInput { sci: &cube.planes[z], err, dq, no_data_dq: dq_flags.get(z).copied().unwrap_or(DQ_DO_NOT_USE) };
                convolve_plane(input, kernel, &cube.filters[z].filter, &convolve_options, plans, cancelled)
            })
            .collect::<Result<Vec<PlaneOutput>, MatchError>>()?;
        for (&(z, _), output) in chunk.iter().zip(convolved) {
            outputs[z] = Some(output);
        }
    }

    let mut matched = cube.clone();
    let mut spreads: Vec<Vec<DqSpread>> = cube.planes.iter().map(|_| Vec::new()).collect();
    for (z, output) in outputs.into_iter().enumerate() {
        let (Some(output), Some(kernel)) = (output, chosen[z]) else { continue };
        matched.planes[z] = output.sci;
        if let (Some(err), Some(propagated)) = (matched.err.as_mut(), output.err) {
            err[z] = propagated;
        }
        if let (Some(dq), Some(propagated)) = (matched.dq.as_mut(), output.dq) {
            dq.planes[z] = propagated;
        }
        warnings.extend(output.warnings);
        warnings.extend(edge_ring_warning(&cube.filters[z].filter, &cube.planes[z], boundary_band(&kernel.data)));
        spreads[z] = output.dq_spread;
    }
    let union = (options.mode == MatchMode::Astroburst)
        .then(|| Array2::from_shape_fn(cube.dims(), |p| cube.planes.iter().any(|plane| !plane[p].is_finite())));
    let target_fwhm_px = assigned.iter().filter_map(|(_, kernel)| kernel.map(|(_, cards)| cards.target_fwhm_px)).find(|fwhm| fwhm.is_finite());
    let mut planes = Vec::with_capacity(cube.planes.len());
    let mut statuses = Vec::with_capacity(cube.planes.len());
    for (z, (status, kernel)) in assigned.iter().enumerate() {
        let mut masked_pixels = 0;
        if let Some(mask) = &union {
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
        if *status == PlaneStatus::Reference && masked_pixels > 0 {
            warnings.push(format!(
                "{filter}: {masked_pixels} pixels finite in the reference are NaN in another plane; the union NaN mask removed them from the reference too, so every plane keeps the same pixels"
            ));
        }
        if *status == PlaneStatus::Unmatched {
            warnings.push(format!("{filter}: no kernel to {reference}, so the plane was left unmatched (allow_unmatched)"));
        }
        let (match_status, fwhm_px) = match kernel {
            _ if *status == PlaneStatus::Reference => (MatchStatus::Reference, target_fwhm_px),
            Some((_, cards)) if cards.sum_squares.is_finite() => (MatchStatus::Matched, Some(cards.matched_fwhm_px)),
            Some(_) => (MatchStatus::External, None),
            None => (MatchStatus::Unmatched, None),
        };
        matched.filters[z].psf_fwhm_arcsec = fwhm_px.zip(matched.filters[z].pixscale_arcsec).map(|(fwhm, scale)| fwhm * scale).filter(|fwhm| fwhm.is_finite());
        if let Some((kernel, _)) = kernel {
            matched.filters[z].kernel = kernel.label.clone();
        }
        planes.push(MatchedPlane {
            filter,
            status: *status,
            kernel: kernel.map(|(kernel, _)| kernel.label.clone()),
            masked_pixels,
            boundary_band_px: kernel.map(|(kernel, _)| boundary_band(&kernel.data)),
            edge_kernel_mass: kernel.map(|(kernel, _)| edge_kernel_mass(&kernel.data)),
            dq_spread: std::mem::take(&mut spreads[z]),
        });
        statuses.push(match_status);
    }
    let noise = noise.and_then(|noise| match noise.for_planes(&cube.filters) {
        Ok(aligned) => Some(CubeNoise {
            planes: aligned
                .planes
                .into_iter()
                .zip(&assigned)
                .map(|(plane, (_, kernel))| match kernel {
                    Some((kernel, cards)) => {
                        PlaneNoise { correlation: matched_correlation(&plane.correlation, &kernel.data), noiscorr_match: 1.0 / cards.sum_squares.sqrt(), ..plane }
                    }
                    None => PlaneNoise { noiscorr_match: 1.0, ..plane },
                })
                .collect(),
            source: NoiseSource::Match,
        }),
        Err(error) => {
            warnings.push(format!("the noise correlation maps were not carried through matching: {error}"));
            None
        }
    });

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
            &format!("PSF matched to {reference} with FFT convolution kernels: NaN pixels interpolated under the kernel and kept NaN, then the union NaN mask of the input planes applied to every plane, the reference included"),
        ),
    }
    if let Some(band) = planes.iter().filter_map(|plane| plane.boundary_band_px).max() {
        push_history(
            header,
            &format!("Outside the field is taken as zero (astropy boundary=fill, fill_value=0); pixels within {band} px of the edge are exact only for a background-subtracted sky"),
        );
    }
    for plane in &planes {
        match (&plane.kernel, plane.status) {
            (Some(label), _) => push_history(header, &format!("{} convolved with {label}", plane.filter)),
            (None, PlaneStatus::Unmatched) => push_history(header, &format!("{} left unmatched: no kernel to {reference}", plane.filter)),
            _ => {}
        }
    }
    Ok(MatchProducts { matched: MatchedCube { cube: matched, planes, warnings }, noise, statuses })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ndarray::s;

    use super::*;
    use crate::core::cubeprep::align::{NoiseSource, CARD_NCORRK, CARD_NCORRSRC};
    use crate::core::cubeprep::cube::test_support::small_cube;
    use crate::core::cubeprep::cube::{DqPlanes, PlaneNoise, DQ_DO_NOT_USE};
    use crate::core::cubeprep::kernel::{build_kernel, kernel_file_name, kernel_header, KernelOptions};
    use crate::core::cubeprep::psf_io::fit_gaussian;
    use crate::core::imaging::convolve::reference::direct_convolve;
    use crate::core::imaging::convolve::{convolve_fft, convolve_fft_with_variance, padded_len, ConvolveOptions, FftPlanCache, NanTreatment};
    use crate::core::stacking::memory::MEMORY_BUDGET_ENV;

    const FILTERS: [(&str, &str); 3] = [("JWST", "F150W"), ("JWST", "F277W"), ("JWST", "F444W")];
    const FIVE_FILTERS: [(&str, &str); 5] = [("JWST", "F090W"), ("JWST", "F150W"), ("JWST", "F200W"), ("JWST", "F277W"), ("JWST", "F444W")];
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

    fn holed_cube() -> Cube {
        let mut cube = small_cube(&FILTERS, DIMS, scene);
        cube.planes[2][[5, 5]] = f32::from_bits(0x7fc0_1234);
        cube.planes[0][[10, 12]] = f32::NAN;
        cube.planes[1][[30, 40]] = f32::NAN;
        cube.err = Some(cube.planes.iter().map(|plane| plane.mapv(|v| 0.05 * v.abs().sqrt() + 0.01)).collect());
        cube.dq = Some(DqPlanes { planes: (0..3).map(|z| Array2::from_shape_fn(DIMS, |(y, x)| u32::from((x + y + z) % 7 == 0) * 4)).collect(), signed: false });
        cube
    }

    fn five_plane_cube() -> (Cube, Vec<PlaneKernel>) {
        let mut cube = small_cube(&FIVE_FILTERS, DIMS, |z, y, x| {
            let sigma = 1.0 + 0.3 * z as f64;
            let star = 50.0 * (-((x as f64 - 20.3).powi(2) + (y as f64 - 17.6).powi(2)) / (2.0 * sigma * sigma)).exp();
            (1.0 + 0.02 * x as f64 + 0.03 * y as f64 + z as f64 + star) as f32
        });
        cube.planes[0][[10, 12]] = f32::NAN;
        cube.planes[3][[30, 40]] = f32::NAN;
        let mut err: Vec<Array2<f32>> = cube.planes.iter().map(|plane| plane.mapv(|v| 0.05 * v.abs().sqrt() + 0.01)).collect();
        err[1][[4, 4]] = -1.0;
        err[2][[20, 33]] = f32::NAN;
        cube.err = Some(err);
        cube.dq = Some(DqPlanes {
            planes: (0..5).map(|z| Array2::from_shape_fn(DIMS, |(y, x)| u32::from((x + y + z) % 7 == 0) * 4 | u32::from((x * y + z) % 11 == 0) * 64)).collect(),
            signed: false,
        });
        let mut holed = gaussian(9, 9, 1.4);
        holed[[0, 4]] = 0.0;
        let kernels = vec![kernel("F090W", lobed()), kernel("F150W", gaussian(7, 7, 1.2)), kernel("F200W", holed), kernel("F277W", gaussian(5, 6, 1.0))];
        (cube, kernels)
    }

    fn fft_spread(bits: &Array2<u32>, kernel: &Array2<f64>) -> Array2<u32> {
        let support = kernel.mapv(|weight| if weight.is_finite() && weight != 0.0 { 1.0 } else { 0.0 });
        let options = ConvolveOptions { normalize_kernel: false, nan_treatment: NanTreatment::Fill, fill_value: 0.0, ..ConvolveOptions::lightstack() };
        let mut output = Array2::zeros(bits.dim());
        let mut flags = bits.iter().fold(0u32, |all, &value| all | value);
        while flags != 0 {
            let bit = 1u32 << flags.trailing_zeros();
            let mask = bits.mapv(|value| if value & bit != 0 { 1.0 } else { 0.0 });
            let spread = convolve_fft(&mask, &support, &options, &mut FftPlanCache::new()).unwrap().image;
            for (value, hits) in output.iter_mut().zip(spread) {
                if hits > 0.5 {
                    *value |= bit;
                }
            }
            flags &= !bit;
        }
        output
    }

    fn autocorrelated(aligned: &Array2<f32>, kernel: &Array2<f64>) -> Array2<f64> {
        let k = aligned.nrows() as isize;
        let half = k / 2 + kernel.nrows().max(kernel.ncols()) as isize - 1;
        let size = (2 * half + 1) as usize;
        let at = |dy: isize, dx: isize| {
            if dy.abs() <= k / 2 && dx.abs() <= k / 2 {
                f64::from(aligned[[(dy + k / 2) as usize, (dx + k / 2) as usize]])
            } else {
                0.0
            }
        };
        let covariance = Array2::from_shape_fn((size, size), |(i, j)| {
            let (dy, dx) = (i as isize - half, j as isize - half);
            let mut sum = 0.0;
            for ((ay, ax), &a) in kernel.indexed_iter() {
                for ((by, bx), &b) in kernel.indexed_iter() {
                    sum += a * b * at(dy - by as isize + ay as isize, dx - bx as isize + ax as isize);
                }
            }
            sum
        });
        let centre = covariance[[half as usize, half as usize]];
        covariance / centre
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
        let cube = holed_cube();
        let kernels = [kernel("F150W", lobed()), kernel("F277W", gaussian(9, 9, 1.4))];
        let before = cube.clone();
        let matched = match_cube(&cube, &kernels, &options(MatchMode::Lightstack), &[512, 512, 512]).unwrap();
        let out = &matched.cube;
        assert!(same_bits(&out.planes[2], &before.planes[2]), "reference SCI");
        assert!(same_bits(&out.err.as_ref().unwrap()[2], &before.err.as_ref().unwrap()[2]), "reference ERR");
        assert_eq!(out.dq.as_ref().unwrap().planes[2], before.dq.as_ref().unwrap().planes[2], "reference DQ");
        assert_eq!(out.filters[2], before.filters[2], "reference metadata");
        let reference = MatchedPlane {
            filter: "F444W".to_string(),
            status: PlaneStatus::Reference,
            kernel: None,
            masked_pixels: 0,
            boundary_band_px: None,
            edge_kernel_mass: None,
            dq_spread: Vec::new(),
        };
        assert_eq!(matched.planes[2], reference);
        assert!(!same_bits(&out.planes[0], &before.planes[0]) && !same_bits(&out.planes[1], &before.planes[1]), "the other planes are convolved");
        assert_eq!(out.filters[0].kernel, "kernel_f150w_to_f444w.fits");

        let matched = match_cube(&cube, &kernels, &options(MatchMode::Astroburst), &[512, 512, 512]).unwrap();
        let out = &matched.cube;
        for z in 0..3 {
            for hole in [(5, 5), (10, 12), (30, 40)] {
                assert!(out.planes[z][hole].is_nan(), "plane {z} {hole:?}: the union NaN mask covers every plane, the reference included");
                assert!(out.err.as_ref().unwrap()[z][hole].is_nan(), "plane {z} {hole:?}: ERR follows the mask");
            }
        }
        assert_eq!(out.planes[2][[5, 5]].to_bits(), 0x7fc0_1234, "a NaN of the reference keeps its bits");
        let masked: Vec<usize> = matched.planes.iter().map(|plane| plane.masked_pixels).collect();
        assert_eq!(masked, [2, 2, 2], "newly masked pixels per plane");
        let dq = &out.dq.as_ref().unwrap().planes;
        for z in 0..3 {
            assert_eq!((dq[z][[30, 40]] & 512, dq[z][[10, 12]] & 512), (512, 512), "plane {z}: the DQ flag marks every newly masked pixel");
        }
        assert_eq!(dq[0][[5, 5]] & 512, 512);
        assert_eq!(dq[2][[5, 5]], before.dq.as_ref().unwrap().planes[2][[5, 5]], "an own NaN of the reference is not newly masked");
        assert_eq!(dq[0][[0, 0]], before.dq.as_ref().unwrap().planes[0][[0, 0]]);
        let notes = history(out);
        assert!(notes.contains("the union NaN mask of the input planes applied to every plane, the reference included"), "{notes}");
        assert!(!notes.contains("which is unchanged"), "{notes}");

        let defaults = match_cube(&cube, &kernels, &options(MatchMode::Astroburst), &[]).unwrap();
        assert_eq!(defaults.cube.dq.as_ref().unwrap().planes[1][[5, 5]] & DQ_DO_NOT_USE, DQ_DO_NOT_USE);
        assert_eq!(defaults.cube.dq.as_ref().unwrap().planes[2][[10, 12]] & DQ_DO_NOT_USE, DQ_DO_NOT_USE);
    }

    #[test]
    fn reference_only_pixels_are_counted_and_warned() {
        let cube = holed_cube();
        let kernels = [kernel("F150W", lobed()), kernel("F277W", gaussian(9, 9, 1.4))];
        let matched = match_cube(&cube, &kernels, &options(MatchMode::Astroburst), &[512, 512, 512]).unwrap();
        assert_eq!((matched.planes[2].status, matched.planes[2].masked_pixels), (PlaneStatus::Reference, 2));
        let (before, after) = (&cube.planes[2], &matched.cube.planes[2]);
        let changed: Vec<(usize, usize)> = before.indexed_iter().zip(after.iter()).filter(|((_, b), a)| b.to_bits() != a.to_bits()).map(|((p, _), _)| p).collect();
        assert_eq!(changed, [(10, 12), (30, 40)], "only the pixels finite in the reference and NaN in another plane change");
        for hole in [(10, 12), (30, 40)] {
            assert!(after[hole].is_nan() && matched.cube.err.as_ref().unwrap()[2][hole].is_nan(), "{hole:?}");
            assert_eq!(matched.cube.dq.as_ref().unwrap().planes[2][hole], cube.dq.as_ref().unwrap().planes[2][hole] | 512, "{hole:?}");
        }
        let notes: Vec<&String> = matched.warnings.iter().filter(|warning| warning.starts_with("F444W: ")).collect();
        assert!(notes.len() == 1 && notes[0].contains("2 pixels") && notes[0].contains("union NaN mask"), "{:?}", matched.warnings);

        let compat = match_cube(&cube, &kernels, &options(MatchMode::Lightstack), &[512, 512, 512]).unwrap();
        assert_eq!(compat.planes[2].masked_pixels, 0);
        assert!(compat.warnings.iter().all(|warning| !warning.starts_with("F444W: ")), "{:?}", compat.warnings);
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

    #[test]
    fn chunked_matching_is_bit_identical_for_every_chunk_size() {
        let (cube, kernels) = five_plane_cube();
        let flags = [512; 5];
        let (rows, cols) = DIMS;
        let pixels = (rows * cols) as u64;
        let per_plane = 16 * (padded_len(rows, 9) * padded_len(cols, 9)) as u64 * 8 + 8 * pixels * 4;
        let retained = 12 * pixels * 4;
        for mode in MODES {
            let single = match_cube_with(&cube, &kernels, &[], &options(mode), &flags, None, None, &|| false).unwrap().matched;
            assert!(single.warnings.iter().all(|warning| !warning.contains("memory")), "{:?}", single.warnings);
            for in_flight in 1..=4u64 {
                let memory = MemoryInfo { physical_available: retained + in_flight * per_plane, commit_available: 1 << 40 };
                let chunked = match_cube_with(&cube, &kernels, &[], &options(mode), &flags, None, Some(memory), &|| false).unwrap().matched;
                for z in 0..5 {
                    assert!(same_bits(&chunked.cube.planes[z], &single.cube.planes[z]), "{mode:?}, {in_flight} in flight: SCI {z}");
                    assert!(same_bits(&chunked.cube.err.as_ref().unwrap()[z], &single.cube.err.as_ref().unwrap()[z]), "{mode:?}, {in_flight} in flight: ERR {z}");
                }
                assert_eq!(chunked.cube.dq, single.cube.dq, "{mode:?}, {in_flight} in flight");
                assert_eq!(chunked.cube.filters, single.cube.filters);
                assert_eq!(chunked.planes, single.planes);
                assert_eq!(history(&chunked.cube), history(&single.cube));
                let (notes, others): (Vec<&String>, Vec<&String>) = chunked.warnings.iter().partition(|warning| warning.contains("memory"));
                assert_eq!(others, single.warnings.iter().collect::<Vec<_>>(), "{mode:?}, {in_flight} in flight");
                let chunking = notes.iter().any(|note| note.contains(&format!("{in_flight} of the 4 matched planes at a time")));
                assert_eq!(chunking, in_flight < 4, "{mode:?}: {notes:?}");
            }
        }
    }

    #[test]
    fn a_budget_below_one_plane_refuses_with_insufficient_memory() {
        let (cube, kernels) = five_plane_cube();
        let (rows, cols) = DIMS;
        let pixels = (rows * cols) as u64;
        let per_plane = 16 * (padded_len(rows, 9) * padded_len(cols, 9)) as u64 * 8 + 8 * pixels * 4;
        let peak = (per_plane + 12 * pixels * 4).max((12 * 5 + 12 * 4 + 1) * pixels);
        let budget = |bytes: u64| Some(MemoryInfo { physical_available: bytes, commit_available: bytes });
        let error = match_cube_with(&cube, &kernels, &[], &options(MatchMode::Astroburst), &[], None, budget(peak - 1), &|| false).unwrap_err();
        assert_eq!(error.code(), INSUFFICIENT_MEMORY);
        let MatchError::InsufficientMemory { rows: error_rows, cols: error_cols, planes, bytes, message } = &error else {
            panic!("{error:?}");
        };
        assert_eq!((*error_rows, *error_cols, *planes, *bytes), (rows, cols, 5, peak));
        let text = error.to_string();
        assert!(text.starts_with("insufficient_memory: PSF matching this cube of 5 planes of 48x40 needs about"), "{text}");
        for needle in ["Crop the cube", "drop filters", MEMORY_BUDGET_ENV] {
            assert!(message.contains(needle), "{needle:?} missing from {message}");
        }
        let fits = match_cube_with(&cube, &kernels, &[], &options(MatchMode::Astroburst), &[], None, budget(peak), &|| false).unwrap();
        assert!(fits.matched.warnings.iter().any(|warning| warning.contains("1 of the 4 matched planes at a time")), "{:?}", fits.matched.warnings);
    }

    #[test]
    fn dq_dilation_matches_the_fft_reference_for_dense_lobed_and_holed_kernels() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut random = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut holed = lobed();
        holed[[0, 0]] = 0.0;
        holed[[3, 5]] = 0.0;
        holed[[6, 1]] = f64::NAN;
        let kernels = [
            ("odd", gaussian(7, 7, 1.1)),
            ("even", gaussian(4, 6, 1.0)),
            ("row", gaussian(1, 5, 1.0)),
            ("column", gaussian(6, 1, 1.0)),
            ("single", Array2::from_elem((1, 1), 1.0)),
            ("wide", gaussian(15, 13, 3.0)),
            ("lobed", lobed()),
            ("holed", holed),
        ];
        for dims in [(23, 31), (5, 4)] {
            let bits = Array2::from_shape_fn(dims, |_| {
                let draw = random();
                if draw % 5 == 0 { (draw >> 32) as u32 } else { 0 }
            });
            assert!(bits.iter().fold(0u32, |all, &value| all | value).count_ones() > 20, "the masks carry many bits");
            let mut plans = FftPlanCache::new();
            for (name, kernel) in &kernels {
                let mut spread = Spread::new(kernel, dims, &mut plans);
                let dense = kernel.iter().all(|weight| weight.is_finite() && *weight != 0.0);
                assert_eq!(matches!(spread, Spread::Dilation { .. }), dense, "{name}: dense kernels take the running-OR dilation");
                assert_eq!(spread.apply(&bits, &|| false).unwrap(), fft_spread(&bits, kernel), "{name} on {dims:?}");
            }
        }
    }

    #[test]
    fn invalid_err_gives_identical_err_with_and_without_the_inner_pass() {
        let mut cube = small_cube(&FILTERS, DIMS, scene);
        cube.planes[0][[6, 7]] = f32::NAN;
        let mut err: Vec<Array2<f32>> = cube.planes.iter().map(|plane| plane.mapv(|v| 0.05 * v.abs().sqrt() + 0.01)).collect();
        for (z, y, x, value) in [(0, 3, 4, f32::NAN), (0, 20, 20, -1.0), (0, 6, 7, -2.0), (1, 0, 0, f32::INFINITY), (1, 39, 47, f32::NEG_INFINITY), (1, 17, 30, -0.5)] {
            err[z][[y, x]] = value;
        }
        cube.err = Some(err.clone());
        let mut holed = gaussian(9, 9, 1.4);
        holed[[4, 0]] = 0.0;
        let kernels = [kernel("F150W", lobed()), kernel("F277W", holed)];
        let known = |e: f32| e.is_finite() && e >= 0.0;
        for mode in MODES {
            let matched = match_cube(&cube, &kernels, &options(mode), &[]).unwrap();
            let convolve_options = match mode {
                MatchMode::Astroburst => ConvolveOptions { preserve_nan: true, ..ConvolveOptions::lightstack() },
                MatchMode::Lightstack => ConvolveOptions::lightstack(),
            };
            for z in 0..2 {
                let image = cube.planes[z].mapv(f64::from);
                let variance = err[z].mapv(|e| if known(e) { f64::from(e).powi(2) } else { f64::NAN });
                let inner = convolve_fft_with_variance(&image, &variance, &kernels[z].data, &convolve_options, &mut FftPlanCache::new()).unwrap();
                let unknown = Array2::from_shape_fn(image.dim(), |p| u32::from(image[p].is_finite() && !known(err[z][p])));
                let mut expected = inner.variance.unwrap();
                for (value, hit) in expected.iter_mut().zip(fft_spread(&unknown, &kernels[z].data)) {
                    if hit != 0 {
                        *value = f64::NAN;
                    }
                }
                let mut expected = expected.mapv(|v| v.sqrt() as f32);
                if mode == MatchMode::Astroburst {
                    for (index, value) in expected.indexed_iter_mut() {
                        if cube.planes.iter().any(|plane| !plane[index].is_finite()) {
                            *value = f32::NAN;
                        }
                    }
                }
                assert!(same_bits(&matched.cube.err.as_ref().unwrap()[z], &expected), "{mode:?}: ERR of plane {z}");
            }
        }
    }

    #[test]
    fn a_cancelled_probe_stops_matching_with_code_cancelled() {
        let (cube, kernels) = five_plane_cube();
        for mode in MODES {
            let calls = AtomicUsize::new(0);
            let complete = match_cube_with(&cube, &kernels, &[], &options(mode), &[], None, None, &|| {
                calls.fetch_add(1, Ordering::SeqCst);
                false
            })
            .unwrap();
            let total = calls.load(Ordering::SeqCst);
            assert!(total >= 4 * 3, "{mode:?}: the plane closures and the steps inside convolve_plane probe: {total}");
            for limit in 0..total {
                let seen = AtomicUsize::new(0);
                let error = match_cube_with(&cube, &kernels, &[], &options(mode), &[], None, None, &|| seen.fetch_add(1, Ordering::SeqCst) >= limit).unwrap_err();
                assert_eq!((error.code(), &error), (CANCELLED, &MatchError::Cancelled), "{mode:?}: cancelled after {limit} of {total} probes");
            }
            assert!(MatchError::Cancelled.to_string().starts_with("cancelled: "));
            let again = match_cube_with(&cube, &kernels, &[], &options(mode), &[], None, None, &|| false).unwrap();
            assert!((0..5).all(|z| same_bits(&again.matched.cube.planes[z], &complete.matched.cube.planes[z])));
        }
    }

    #[test]
    fn flagged_pixel_counts_before_and_after_spreading_are_reported() {
        let mut cube = small_cube(&FILTERS, (12, 12), |_, _, _| 2.0);
        let mut flags = (0..3).map(|_| Array2::zeros((12, 12))).collect::<Vec<_>>();
        flags[0][[6, 6]] = 1 << 4;
        flags[0][[0, 0]] = 1 << 2;
        flags[1][[3, 3]] = 1 << 4;
        flags[2][[8, 8]] = 1 << 4;
        cube.planes[1][[3, 3]] = f32::NAN;
        cube.dq = Some(DqPlanes { planes: flags, signed: false });
        let kernels = [kernel("F150W", gaussian(5, 5, 1.0)), kernel("F277W", gaussian(5, 5, 1.0))];
        for mode in MODES {
            let result = match_cube(&cube, &kernels, &options(mode), &[]).unwrap();
            assert_eq!(result.planes[0].dq_spread, [DqSpread { bit: 2, flagged: 1, spread: 9 }, DqSpread { bit: 4, flagged: 1, spread: 25 }], "{mode:?}");
            assert_eq!(result.planes[1].dq_spread, [], "{mode:?}: a flag on a NaN pixel does not spread");
            assert_eq!(result.planes[2].dq_spread, [], "{mode:?}: the reference is not convolved");
            for expected in ["F150W: bit 4 on 1 finite pixels marks 25 after the 5x5 kernel support", "F150W: bit 2 on 1 finite pixels marks 9 after the 5x5 kernel support"] {
                assert!(result.warnings.iter().any(|warning| warning == expected), "{mode:?}: {expected:?} missing from {:?}", result.warnings);
            }
            assert!(result.warnings.iter().all(|warning| !warning.starts_with("F277W: bit")), "{:?}", result.warnings);
        }
    }

    #[test]
    fn the_zero_fill_at_the_edge_is_reported_per_plane_and_recorded_in_history() {
        let wide = gaussian(9, 9, 1.4);
        let even = gaussian(4, 6, 1.0);
        let kernels = [kernel("F150W", wide.clone()), kernel("F277W", even.clone())];
        let flat = small_cube(&FILTERS, DIMS, |_, _, _| 2.0);
        let centred = small_cube(&FILTERS, DIMS, |z, y, x| if (x + y + z) % 2 == 0 { 1.0 } else { -1.0 });
        let sentence = "Outside the field is taken as zero (astropy boundary=fill, fill_value=0); pixels within 4 px of the edge are exact only for a background-subtracted sky";
        let top = wide.slice(s![5.., ..]).sum();
        let bottom = even.slice(s![..2, ..]).sum();
        assert!((top - 0.5 * (1.0 - wide.row(4).sum())).abs() < 1e-12, "{top}");
        let others = [even.slice(s![3.., ..]).sum(), even.slice(s![.., 4..]).sum(), even.slice(s![.., ..3]).sum()];
        assert!(others.iter().all(|other| *other < bottom), "the two rows below the centre of the 4x6 kernel carry the most mass: {bottom} against {others:?}");
        for mode in MODES {
            let matched = match_cube(&flat, &kernels, &options(mode), &[]).unwrap();
            let bands: Vec<Option<usize>> = matched.planes.iter().map(|plane| plane.boundary_band_px).collect();
            assert_eq!(bands, [Some(4), Some(3), None], "{mode:?}");
            assert!((matched.planes[0].edge_kernel_mass.unwrap() - top).abs() < 1e-14, "{mode:?}: {:?} against {top}", matched.planes[0].edge_kernel_mass);
            assert!((matched.planes[1].edge_kernel_mass.unwrap() - bottom).abs() < 1e-14, "{mode:?}: {:?} against {bottom}", matched.planes[1].edge_kernel_mass);
            assert_eq!(matched.planes[2].edge_kernel_mass, None);
            assert!(history(&matched.cube).contains(sentence), "{mode:?}: {}", history(&matched.cube));
            let ring: Vec<&String> = matched.warnings.iter().filter(|warning| warning.contains("outer 2-px ring")).collect();
            assert!(ring.len() == 2 && ring[0].starts_with("F150W: ") && ring[0].contains("within 4 px") && ring[1].starts_with("F277W: ") && ring[1].contains("within 3 px"), "{mode:?}: {ring:?}");

            let subtracted = match_cube(&centred, &kernels, &options(mode), &[]).unwrap();
            assert!(subtracted.warnings.iter().all(|warning| !warning.contains("outer 2-px ring")), "{mode:?}: {:?}", subtracted.warnings);
            assert!(history(&subtracted.cube).contains(sentence));
        }
        let none = match_cube(&flat, &[], &MatchOptions { allow_unmatched: true, ..options(MatchMode::Astroburst) }, &[]).unwrap();
        assert!(!history(&none.cube).contains("Outside the field"), "nothing was convolved: {}", history(&none.cube));
    }

    #[test]
    fn matched_planes_carry_the_aligned_noise_map_convolved_with_the_kernel_autocorrelation() {
        let cube = small_cube(&FILTERS, DIMS, scene);
        let mut aligned = Array2::<f32>::zeros((3, 3));
        aligned[[1, 1]] = 1.0;
        for (y, x, value) in [(1, 0, 0.25), (1, 2, 0.25), (0, 1, 0.125), (2, 1, 0.125), (0, 0, 0.0625), (2, 2, 0.0625)] {
            aligned[[y, x]] = value;
        }
        let delta = Array2::from_elem((1, 1), 1.0f32);
        let plane = |filter: &str, correlation: &Array2<f32>, noiscorr_align: f64| PlaneNoise { filter: filter.to_string(), correlation: correlation.clone(), noiscorr_align, noiscorr_match: 1.0 };
        let noise = CubeNoise { planes: vec![plane("F150W", &delta, 1.0), plane("F277W", &aligned, 1.5), plane("F444W", &aligned, 1.25)], source: NoiseSource::Align };
        let small = gaussian(5, 5, 1.0);
        let even = lobed().slice(s![1..4, 0..4]).to_owned();
        let kernels = [kernel("F150W", small.clone()), kernel("F277W", even.clone())];
        let cards = [KernelCards { sum_squares: 0.125, ..KernelCards::ABSENT }, KernelCards::ABSENT];
        let products = match_cube_with(&cube, &kernels, &cards, &options(MatchMode::Astroburst), &[], Some(&noise), None, &|| false).unwrap();
        assert_eq!(products.statuses, [MatchStatus::Matched, MatchStatus::External, MatchStatus::Reference]);
        let matched = products.noise.expect("an input map gives an output map");
        assert_eq!(matched.source, NoiseSource::Match);
        for (z, data) in [(0, &small), (1, &even)] {
            let got = &matched.planes[z].correlation;
            let expected = autocorrelated(&noise.planes[z].correlation, data);
            assert_eq!((got.dim(), expected.dim()), ((9, 9), (9, 9)), "plane {z}");
            assert_eq!(got[[4, 4]], 1.0, "plane {z}: Q(0) = 1");
            let worst = got.iter().zip(expected.iter()).map(|(g, e)| (f64::from(*g) - e).abs()).fold(0.0, f64::max);
            assert!(worst < 1e-7, "plane {z}: {worst:e}");
        }
        assert_eq!(matched.planes[0].noiscorr_match, 1.0 / 0.125f64.sqrt());
        assert!(matched.planes[1].noiscorr_match.is_nan(), "a kernel without KSUMSQ is external");
        assert_eq!((matched.planes[0].noiscorr_align, matched.planes[1].noiscorr_align), (1.0, 1.5));
        assert_eq!(matched.planes[2], plane("F444W", &aligned, 1.25), "the reference keeps its alignment map and NOISCORR_MATCH 1");
        assert_eq!(matched.k(), 9);
        let header = matched.header();
        assert_eq!((header.get_i64(CARD_NCORRK), header.get(CARD_NCORRSRC)), (Some(9), Some("match")));

        let allowed = MatchOptions { allow_unmatched: true, ..options(MatchMode::Astroburst) };
        let partial = match_cube_with(&cube, &kernels[..1], &cards[..1], &allowed, &[], Some(&noise), None, &|| false).unwrap();
        assert_eq!(partial.statuses, [MatchStatus::Matched, MatchStatus::Unmatched, MatchStatus::Reference]);
        assert_eq!(partial.noise.unwrap().planes[1], plane("F277W", &aligned, 1.5), "an unmatched plane keeps Q_align");
        assert!(match_cube_with(&cube, &kernels, &cards, &options(MatchMode::Astroburst), &[], None, None, &|| false).unwrap().noise.is_none());
        let short = CubeNoise { planes: noise.planes[..2].to_vec(), source: NoiseSource::Align };
        let missing = match_cube_with(&cube, &kernels, &cards, &options(MatchMode::Astroburst), &[], Some(&short), None, &|| false).unwrap();
        assert!(missing.noise.is_none() && missing.matched.warnings.iter().any(|warning| warning.contains("noise") && warning.contains("F444W")), "{:?}", missing.matched.warnings);
    }

    #[test]
    fn psf_fwhm_and_match_status_follow_the_kernel_cards() {
        let cube = small_cube(&FILTERS, DIMS, scene);
        let auto = KernelOptions { auto: true, ..KernelOptions::default() };
        let built = build_kernel(&gaussian(21, 21, 1.2), &gaussian(21, 21, 2.2), &auto).unwrap();
        let cards = KernelCards::read(&kernel_header(&built, "JWST_F150W", "JWST_F444W", Some(0.04)));
        let d = &built.diagnostics;
        let close = |card: f64, value: f64| (card / value - 1.0).abs() < 1e-13;
        assert!(
            close(cards.source_fwhm_px, d.source_fwhm_px.unwrap())
                && close(cards.target_fwhm_px, d.target_fwhm_px.unwrap())
                && close(cards.matched_fwhm_px, d.matched_fwhm_px.unwrap())
                && close(cards.sum_squares, d.sum_squares),
            "{cards:?} against {d:?}"
        );
        let absent = KernelCards::read(&HduHeader::empty());
        assert!(absent.source_fwhm_px.is_nan() && absent.target_fwhm_px.is_nan() && absent.matched_fwhm_px.is_nan() && absent.sum_squares.is_nan(), "{absent:?}");

        let kernels = [kernel("F150W", built.data.clone()), kernel("F277W", gaussian(9, 9, 1.4))];
        let products = match_cube_with(&cube, &kernels, &[cards, KernelCards::ABSENT], &options(MatchMode::Astroburst), &[], None, None, &|| false).unwrap();
        assert_eq!(products.statuses, [MatchStatus::Matched, MatchStatus::External, MatchStatus::Reference]);
        let filters = &products.matched.cube.filters;
        let scale = |z: usize| filters[z].pixscale_arcsec.unwrap();
        assert_eq!(filters[0].psf_fwhm_arcsec, Some(cards.matched_fwhm_px * scale(0)), "a matched plane takes MATFWHM, never the target");
        assert_eq!(filters[1].psf_fwhm_arcsec, None, "an external kernel has no MATFWHM");
        assert_eq!(filters[2].psf_fwhm_arcsec, Some(cards.target_fwhm_px * scale(2)), "the reference takes TGTFWHM");
        assert!((filters[0].psf_fwhm_arcsec.unwrap() / 0.04 / d.matched_fwhm_px.unwrap() - 1.0).abs() < 1e-6);

        let unfitted = KernelCards { source_fwhm_px: f64::NAN, target_fwhm_px: f64::NAN, matched_fwhm_px: f64::NAN, ..cards };
        let allowed = MatchOptions { allow_unmatched: true, ..options(MatchMode::Lightstack) };
        let products = match_cube_with(&cube, &kernels[..1], &[unfitted], &allowed, &[], None, None, &|| false).unwrap();
        assert_eq!(products.statuses, [MatchStatus::Matched, MatchStatus::Unmatched, MatchStatus::Reference]);
        assert!(products.matched.cube.filters.iter().all(|plane| plane.psf_fwhm_arcsec.is_none()), "a failed fit leaves PSF_FWHM NaN");
        let labels: Vec<&str> = products.statuses.iter().map(|status| status.label()).collect();
        assert_eq!(labels, ["matched", "unmatched", "reference"]);
        let plain = match_cube(&cube, &kernels, &options(MatchMode::Astroburst), &[]).unwrap();
        assert!(plain.cube.filters.iter().all(|plane| plane.psf_fwhm_arcsec.is_none()), "match_cube reads no cards");
    }
}
