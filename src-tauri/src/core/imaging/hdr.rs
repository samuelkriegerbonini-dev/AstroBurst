use anyhow::{bail, Result};
use ndarray::{Array2, Zip};

use crate::core::imaging::luminance::{apply_luminance_ratio, rgb_to_luminance};
use crate::core::imaging::star_mask::{generate_star_mask, StarMaskConfig};
use crate::core::imaging::wavelet::{atrous_decompose, atrous_reconstruct_with_bias};
use crate::infra::progress::ProgressHandle;
use crate::math::median::median_f32_mut;
use crate::types::error::AppError;

pub const HDR_LAYERS_MIN: usize = 2;
pub const HDR_LAYERS_MAX: usize = 8;
pub const HDR_ITERATIONS_MIN: usize = 1;
pub const HDR_ITERATIONS_MAX: usize = 4;
pub const HDR_LINEAR_INPUT_ERROR: &str =
    "HDRMT expects a non-linear (stretched) image; median is below 1% of the maximum";

const LINEAR_MEDIAN_FRACTION: f32 = 0.01;
const OVERDRIVE_LAYERS: usize = 2;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HdrConfig {
    pub layers: usize,
    pub iterations: usize,
    pub overdrive: f32,
    pub inverted: bool,
    pub to_lightness: bool,
    pub deringing: bool,
    pub deringing_amount: f32,
}

impl Default for HdrConfig {
    fn default() -> Self {
        Self {
            layers: 6,
            iterations: 1,
            overdrive: 0.0,
            inverted: false,
            to_lightness: true,
            deringing: false,
            deringing_amount: 0.5,
        }
    }
}

struct HdrParams {
    layers: usize,
    iterations: usize,
    overdrive: f32,
    inverted: bool,
    deringing: bool,
    deringing_amount: f32,
}

impl HdrParams {
    fn from_config(cfg: &HdrConfig) -> Result<Self> {
        if !cfg.overdrive.is_finite() || !cfg.deringing_amount.is_finite() {
            bail!("overdrive and deringing_amount must be finite");
        }
        Ok(Self {
            layers: cfg.layers.clamp(HDR_LAYERS_MIN, HDR_LAYERS_MAX),
            iterations: cfg.iterations.clamp(HDR_ITERATIONS_MIN, HDR_ITERATIONS_MAX),
            overdrive: cfg.overdrive.clamp(0.0, 1.0),
            inverted: cfg.inverted,
            deringing: cfg.deringing,
            deringing_amount: cfg.deringing_amount.clamp(0.0, 1.0),
        })
    }

    fn progress_steps(&self) -> u64 {
        (self.iterations * 2 + usize::from(self.deringing)) as u64
    }
}

fn finite_min_max(image: &Array2<f32>) -> (f32, f32) {
    image
        .iter()
        .filter(|v| v.is_finite())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &v| {
            (mn.min(v), mx.max(v))
        })
}

fn finite_median(image: &Array2<f32>) -> f32 {
    let mut finite: Vec<f32> = image.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return 0.0;
    }
    median_f32_mut(&mut finite)
}

fn rescale_to_unit(image: &mut Array2<f32>) {
    let (min, max) = finite_min_max(image);
    let range = max - min;
    if !range.is_finite() || range <= 0.0 {
        return;
    }
    let inv_range = 1.0 / range;
    image.par_mapv_inplace(|v| (v - min) * inv_range);
}

fn flatten_large_scale(work: &Array2<f32>, params: &HdrParams) -> Array2<f32> {
    let mut decomposition = atrous_decompose(work, params.layers);
    let residual_median = finite_median(&decomposition.residual);
    decomposition
        .residual
        .par_mapv_inplace(|v| v.min(residual_median));
    let bias: Vec<f32> = (0..params.layers)
        .map(|k| {
            if k + OVERDRIVE_LAYERS >= params.layers {
                -params.overdrive
            } else {
                0.0
            }
        })
        .collect();
    atrous_reconstruct_with_bias(&decomposition, &bias)
}

fn check_cancelled(progress: Option<&ProgressHandle>) -> Result<()> {
    if let Some(p) = progress {
        if p.is_cancelled() {
            return Err(AppError::Cancelled.into());
        }
    }
    Ok(())
}

pub fn hdrmt(image: &Array2<f32>, cfg: &HdrConfig) -> Result<Array2<f32>> {
    hdrmt_with_progress(image, cfg, None)
}

pub fn hdrmt_with_progress(
    image: &Array2<f32>,
    cfg: &HdrConfig,
    progress: Option<&ProgressHandle>,
) -> Result<Array2<f32>> {
    let params = HdrParams::from_config(cfg)?;
    let (input_min, input_max) = finite_min_max(image);
    if !input_min.is_finite() || !input_max.is_finite() {
        bail!("HDRMT needs at least one finite pixel");
    }
    let input_median = finite_median(image);
    if input_median < LINEAR_MEDIAN_FRACTION * input_max {
        bail!(HDR_LINEAR_INPUT_ERROR);
    }
    let range = input_max - input_min;
    if range <= 0.0 {
        if let Some(p) = progress {
            p.emit_complete();
        }
        return Ok(image.clone());
    }
    if let Some(p) = progress {
        p.set_total(params.progress_steps());
    }

    let inv_range = 1.0 / range;
    let normalised = image.mapv(|v| (v - input_min) * inv_range);
    let mut work = if params.inverted {
        normalised.mapv(|v| 1.0 - v)
    } else {
        normalised.clone()
    };

    for iteration in 0..params.iterations {
        check_cancelled(progress)?;
        if let Some(p) = progress {
            p.tick_with_stage(&format!(
                "decomposing iteration {}/{}",
                iteration + 1,
                params.iterations
            ));
        }
        work = flatten_large_scale(&work, &params);
        rescale_to_unit(&mut work);
        if let Some(p) = progress {
            p.tick_with_stage(&format!(
                "renormalising iteration {}/{}",
                iteration + 1,
                params.iterations
            ));
        }
    }

    if params.inverted {
        work.par_mapv_inplace(|v| 1.0 - v);
    }

    if params.deringing {
        check_cancelled(progress)?;
        if let Some(p) = progress {
            p.tick_with_stage("deringing star cores");
        }
        let detection_input = normalised.mapv(|v| if v.is_finite() { v } else { 0.0 });
        let star_mask = generate_star_mask(&detection_input, &StarMaskConfig::default())
            .map_err(anyhow::Error::msg)?;
        let amount = params.deringing_amount;
        Zip::from(&mut work)
            .and(&normalised)
            .and(&star_mask.mask)
            .par_for_each(|w, &n, &m| {
                *w += (n - *w) * (m * amount);
            });
    }

    work.par_mapv_inplace(|v| input_min + v * range);
    if let Some(p) = progress {
        p.emit_complete();
    }
    Ok(work)
}

pub fn hdrmt_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    cfg: &HdrConfig,
) -> Result<(Array2<f32>, Array2<f32>, Array2<f32>)> {
    if r.dim() != g.dim() || g.dim() != b.dim() {
        bail!(
            "channel dimension mismatch: R={:?} G={:?} B={:?}",
            r.dim(),
            g.dim(),
            b.dim()
        );
    }
    if cfg.to_lightness {
        let luminance = rgb_to_luminance(r, g, b);
        let compressed = hdrmt(&luminance, cfg)?;
        return Ok(apply_luminance_ratio(r, g, b, &luminance, &compressed));
    }
    let (ro, (go, bo)) = rayon::join(
        || hdrmt(r, cfg),
        || rayon::join(|| hdrmt(g, cfg), || hdrmt(b, cfg)),
    );
    Ok((ro?, go?, bo?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 256;
    const BUMP: (usize, usize) = (128, 128);
    const STAR: (usize, usize) = (40, 200);
    const DOT: (usize, usize) = (200, 40);
    const DIP: (usize, usize) = (192, 192);

    fn gaussian(y: usize, x: usize, cy: f32, cx: f32, sigma: f32) -> f32 {
        let dy = y as f32 - cy;
        let dx = x as f32 - cx;
        (-(dy * dy + dx * dx) / (2.0 * sigma * sigma)).exp()
    }

    fn bump_and_star_on_gradient() -> Array2<f32> {
        Array2::from_shape_fn((SIZE, SIZE), |(y, x)| {
            0.1 + 0.1 * x as f32 / (SIZE - 1) as f32
                + 0.85 * gaussian(y, x, BUMP.0 as f32, BUMP.1 as f32, 15.0)
                + 0.85 * gaussian(y, x, STAR.0 as f32, STAR.1 as f32, 1.5)
        })
    }

    fn bump_and_dip() -> Array2<f32> {
        Array2::from_shape_fn((SIZE, SIZE), |(y, x)| {
            0.5 + 0.4 * gaussian(y, x, 64.0, 64.0, 10.0)
                - 0.4 * gaussian(y, x, DIP.0 as f32, DIP.1 as f32, 10.0)
        })
    }

    fn with_dot(image: &Array2<f32>) -> Array2<f32> {
        let mut out = image.clone();
        out[[DOT.0, DOT.1]] += 0.1;
        out
    }

    fn above_median(image: &Array2<f32>, at: (usize, usize)) -> f32 {
        image[[at.0, at.1]] - finite_median(image)
    }

    fn cfg(layers: usize) -> HdrConfig {
        HdrConfig {
            layers,
            ..HdrConfig::default()
        }
    }

    #[test]
    fn bright_bump_is_compressed_while_fine_detail_keeps_its_amplitude() {
        let plain = bump_and_star_on_gradient();
        let dotted = with_dot(&plain);
        let bump_before = above_median(&plain, BUMP);
        assert!(
            bump_before > 0.8,
            "bump should dominate the input: {}",
            bump_before
        );

        let out_plain = hdrmt(&plain, &cfg(3)).unwrap();
        let out_dotted = hdrmt(&dotted, &cfg(3)).unwrap();
        let bump_after = above_median(&out_plain, BUMP);
        assert!(
            bump_after < bump_before * 0.5,
            "large-scale bump should be compressed: before {} after {}",
            bump_before,
            bump_after
        );
        assert!(
            out_plain[[STAR.0, STAR.1]] > 0.8,
            "small-scale star keeps its brightness: {}",
            out_plain[[STAR.0, STAR.1]]
        );

        let dot_after = out_dotted[[DOT.0, DOT.1]] - out_plain[[DOT.0, DOT.1]];
        assert!(
            (dot_after - 0.1).abs() <= 0.02,
            "fine detail amplitude drifted: {}",
            dot_after
        );

        let (min, max) = finite_min_max(&plain);
        for v in out_plain.iter() {
            assert!(
                *v >= min - 1e-5 && *v <= max + 1e-5,
                "value {} outside input range",
                v
            );
        }
        assert_eq!(out_plain.dim(), plain.dim());
    }

    #[test]
    fn more_layers_keep_more_of_the_bump() {
        let image = bump_and_star_on_gradient();
        let few = above_median(&hdrmt(&image, &cfg(3)).unwrap(), BUMP);
        let many = above_median(&hdrmt(&image, &cfg(6)).unwrap(), BUMP);
        assert!(
            few < many,
            "3 layers {} should compress more than 6 layers {}",
            few,
            many
        );
    }

    #[test]
    fn overdrive_and_iterations_compress_more() {
        let image = bump_and_star_on_gradient();
        let base = above_median(&hdrmt(&image, &cfg(4)).unwrap(), BUMP);
        let overdriven = HdrConfig {
            overdrive: 1.0,
            ..cfg(4)
        };
        let iterated = HdrConfig {
            iterations: 3,
            ..cfg(4)
        };
        let od = above_median(&hdrmt(&image, &overdriven).unwrap(), BUMP);
        let it = above_median(&hdrmt(&image, &iterated).unwrap(), BUMP);
        assert!(
            od < base,
            "overdrive {} should compress more than {}",
            od,
            base
        );
        assert!(
            it < base,
            "iterations {} should compress more than {}",
            it,
            base
        );
    }

    #[test]
    fn normal_mode_flattens_bright_structures_and_keeps_dark_ones() {
        let image = bump_and_dip();
        let out = hdrmt(&image, &cfg(3)).unwrap();
        let bump_before = above_median(&image, (64, 64));
        let bump_after = above_median(&out, (64, 64));
        let dip_before = -above_median(&image, DIP);
        let dip_after = -above_median(&out, DIP);
        assert!(
            bump_after < bump_before * 0.5,
            "bump {} -> {}",
            bump_before,
            bump_after
        );
        assert!(
            dip_after > dip_before * 0.7,
            "dip {} -> {}",
            dip_before,
            dip_after
        );
    }

    #[test]
    fn inverted_transform_round_trips_through_the_inverted_image() {
        let image = bump_and_dip();
        let (min, max) = finite_min_max(&image);
        let mirrored = image.mapv(|v| min + max - v);

        let inverted_cfg = HdrConfig {
            inverted: true,
            ..cfg(3)
        };
        let direct = hdrmt(&image, &inverted_cfg).unwrap();
        let via_mirror = hdrmt(&mirrored, &cfg(3)).unwrap().mapv(|v| min + max - v);
        for y in 0..SIZE {
            for x in 0..SIZE {
                assert!(
                    (direct[[y, x]] - via_mirror[[y, x]]).abs() < 1e-5,
                    "round trip mismatch at ({},{}): {} vs {}",
                    y,
                    x,
                    direct[[y, x]],
                    via_mirror[[y, x]]
                );
            }
        }

        let dip_before = -above_median(&image, DIP);
        let dip_inverted = -above_median(&direct, DIP);
        let bump_before = above_median(&image, (64, 64));
        let bump_inverted = above_median(&direct, (64, 64));
        assert!(
            dip_inverted < dip_before * 0.5,
            "inverted mode flattens the dip: {} -> {}",
            dip_before,
            dip_inverted
        );
        assert!(
            bump_inverted > bump_before * 0.7,
            "inverted mode keeps the bump: {} -> {}",
            bump_before,
            bump_inverted
        );
    }

    #[test]
    fn linear_input_is_rejected_with_a_clear_message() {
        let image =
            Array2::from_shape_fn((64, 64), |(y, x)| 0.002 + gaussian(y, x, 32.0, 32.0, 2.0));
        let err = hdrmt(&image, &HdrConfig::default())
            .unwrap_err()
            .to_string();
        assert_eq!(err, HDR_LINEAR_INPUT_ERROR);
    }

    #[test]
    fn lightness_mode_keeps_channel_ratios() {
        let lum = bump_and_star_on_gradient();
        let r = lum.clone();
        let g = lum.mapv(|v| v * 0.6);
        let b = lum.mapv(|v| v * 0.3);
        let (nr, ng, nb) = hdrmt_rgb(&r, &g, &b, &cfg(3)).unwrap();
        let mut changed = 0usize;
        for y in 0..SIZE {
            for x in 0..SIZE {
                assert!(
                    (ng[[y, x]] / nr[[y, x]] - 0.6).abs() < 1e-3,
                    "ratio drift at ({},{})",
                    y,
                    x
                );
                assert!(
                    (nb[[y, x]] / nr[[y, x]] - 0.3).abs() < 1e-3,
                    "ratio drift at ({},{})",
                    y,
                    x
                );
                if (nr[[y, x]] - r[[y, x]]).abs() > 1e-3 {
                    changed += 1;
                }
            }
        }
        assert!(
            changed > 1000,
            "lightness compression should change the channels"
        );
    }

    #[test]
    fn per_channel_mode_processes_each_channel() {
        let lum = bump_and_star_on_gradient();
        let r = lum.clone();
        let g = lum.mapv(|v| v * 0.6);
        let b = lum.mapv(|v| v * 0.3);
        let per_channel = HdrConfig {
            to_lightness: false,
            ..cfg(3)
        };
        let (nr, ng, nb) = hdrmt_rgb(&r, &g, &b, &per_channel).unwrap();
        assert_eq!(nr, hdrmt(&r, &per_channel).unwrap());
        assert_eq!(ng, hdrmt(&g, &per_channel).unwrap());
        assert_eq!(nb, hdrmt(&b, &per_channel).unwrap());
    }

    #[test]
    fn rgb_variant_rejects_mismatched_channels() {
        let r = Array2::from_elem((8, 8), 0.5f32);
        let g = Array2::from_elem((4, 8), 0.5f32);
        assert!(hdrmt_rgb(&r, &g, &r, &HdrConfig::default()).is_err());
    }

    #[test]
    fn nan_pixels_stay_nan_and_neighbours_stay_finite() {
        let mut image = bump_and_star_on_gradient();
        for y in 20..30 {
            for x in 20..30 {
                image[[y, x]] = f32::NAN;
            }
        }
        let out = hdrmt(&image, &cfg(6)).unwrap();
        for y in 0..SIZE {
            for x in 0..SIZE {
                let inside = (20..30).contains(&y) && (20..30).contains(&x);
                assert_eq!(
                    out[[y, x]].is_nan(),
                    inside,
                    "nan mismatch at ({},{})",
                    y,
                    x
                );
            }
        }
    }

    #[test]
    fn deringing_pulls_star_cores_back_toward_the_input() {
        let mut image = Array2::from_shape_fn((SIZE, SIZE), |(y, x)| {
            0.15 + 0.85 * gaussian(y, x, BUMP.0 as f32, BUMP.1 as f32, 15.0)
                + 0.7 * gaussian(y, x, STAR.0 as f32, STAR.1 as f32, 2.0)
        });
        let mut seed = 7u32;
        for v in image.iter_mut() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *v += ((seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5) * 0.004;
        }
        let (min, max) = finite_min_max(&image);
        let normalised = image.mapv(|v| (v - min) / (max - min));
        let mask = generate_star_mask(&normalised, &StarMaskConfig::default()).unwrap();
        assert!(mask.stars_masked >= 1, "synthetic star must be detected");
        assert!(
            mask.mask[[STAR.0, STAR.1]] > 0.5,
            "mask at star {}",
            mask.mask[[STAR.0, STAR.1]]
        );

        let plain = hdrmt(&image, &cfg(3)).unwrap();
        let deringed = hdrmt(
            &image,
            &HdrConfig {
                deringing: true,
                deringing_amount: 1.0,
                ..cfg(3)
            },
        )
        .unwrap();
        let star_in = image[[STAR.0, STAR.1]];
        let plain_err = (plain[[STAR.0, STAR.1]] - star_in).abs();
        let dering_err = (deringed[[STAR.0, STAR.1]] - star_in).abs();
        assert!(
            dering_err < plain_err,
            "deringing should move the star core toward the input: {} vs {}",
            dering_err,
            plain_err
        );
        assert!(
            (plain[[128, 20]] - deringed[[128, 20]]).abs() < 1e-4,
            "far from stars the result is unchanged"
        );
        let half = hdrmt(
            &image,
            &HdrConfig {
                deringing: true,
                deringing_amount: 0.5,
                ..cfg(3)
            },
        )
        .unwrap();
        let expected = 0.5 * plain[[STAR.0, STAR.1]] + 0.5 * star_in;
        assert!(
            (half[[STAR.0, STAR.1]] - expected).abs() < 1e-3,
            "amount blends linearly"
        );
    }

    #[test]
    fn layer_and_iteration_counts_are_clamped_to_the_supported_range() {
        let image = bump_and_star_on_gradient();
        let too_many = HdrConfig {
            layers: 20,
            iterations: 9,
            ..HdrConfig::default()
        };
        let clamped = HdrConfig {
            layers: HDR_LAYERS_MAX,
            iterations: HDR_ITERATIONS_MAX,
            ..HdrConfig::default()
        };
        assert_eq!(
            hdrmt(&image, &too_many).unwrap(),
            hdrmt(&image, &clamped).unwrap()
        );
    }

    #[test]
    fn flat_image_passes_through_untouched() {
        let image = Array2::from_elem((32, 32), 0.6f32);
        let out = hdrmt(&image, &HdrConfig::default()).unwrap();
        assert_eq!(out, image);
    }
}
