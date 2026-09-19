use anyhow::{Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use super::background::{apply_correction, solve_linear_system, BackgroundMode};
use crate::infra::progress::ProgressHandle;
use crate::math::median::median_f32_mut;
use crate::math::sigma_clipped_stats;
use crate::types::constants::MAD_TO_SIGMA;
use crate::types::error::AppError;

const MIN_FINITE_FRACTION: f64 = 0.25;
const STAR_SIGMAS: f64 = 5.0;
const BOX_CLIP_KAPPA: f32 = 2.5;
const BOX_CLIP_ITERATIONS: usize = 3;
const RESIDUAL_FLOOR_SIGMAS: f64 = 3.0;
const MIN_COARSE_CELL: usize = 8;
const COARSE_NODES_ALONG_SHORT_AXIS: usize = 256;
const MIN_SAMPLES: usize = 4;
const MIN_AUTO_CELL: usize = 4;
const DUPLICATE_DISTANCE_PX: f64 = 1.0;
const DIVIDE_FLOOR: f32 = 1e-10;
const MIN_SAMPLE_WEIGHT: f64 = 1e-6;
const PROGRESS_STAGES: u64 = 4;

pub const REASON_BOUNDS: &str = "bounds";
pub const REASON_FINITE: &str = "finite";
pub const REASON_STAR: &str = "star";
pub const REASON_DEVIATION: &str = "deviation";

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DbeConfig {
    pub sample_radius: usize,
    pub tolerance: f32,
    pub smoothing: f32,
    pub auto_grid: Option<usize>,
    pub manual_samples: Vec<(f64, f64)>,
    pub reject_stars: bool,
    pub mode: BackgroundMode,
    pub normalize: bool,
    pub max_samples: usize,
}

impl Default for DbeConfig {
    fn default() -> Self {
        Self {
            sample_radius: 5,
            tolerance: 1.0,
            smoothing: 0.25,
            auto_grid: Some(10),
            manual_samples: Vec::new(),
            reject_stars: true,
            mode: BackgroundMode::Subtract,
            normalize: true,
            max_samples: 400,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DbeSample {
    pub x: f64,
    pub y: f64,
    pub value: f64,
    pub weight: f64,
    pub rejected: bool,
    pub reason: Option<&'static str>,
    pub manual: bool,
}

#[derive(Debug, Clone)]
pub struct DbeResult {
    pub model: Array2<f32>,
    pub corrected: Array2<f32>,
    pub samples: Vec<DbeSample>,
    pub sample_count: usize,
    pub rejected_count: usize,
    pub rms_residual: f64,
    pub elapsed_ms: u64,
    pub spline: ThinPlateSpline,
}

#[derive(Debug, Clone)]
pub struct ThinPlateSpline {
    nodes: Vec<(f64, f64)>,
    weights: Vec<f64>,
    affine: [f64; 3],
    scale: f64,
}

impl ThinPlateSpline {
    pub fn fit(
        points: &[(f64, f64)],
        values: &[f64],
        weights: &[f64],
        scale: f64,
        smoothing: f64,
    ) -> Result<Self> {
        let n = points.len();
        if n < MIN_SAMPLES {
            anyhow::bail!("DBE needs at least {} accepted samples, got {}", MIN_SAMPLES, n);
        }
        if values.len() != n || weights.len() != n {
            anyhow::bail!("Spline inputs have mismatched lengths");
        }
        if !(scale > 0.0) {
            anyhow::bail!("Spline coordinate scale must be positive");
        }
        let nodes: Vec<(f64, f64)> = points.iter().map(|&(x, y)| (x / scale, y / scale)).collect();
        let lambda = smoothing.max(0.0) * neighbour_kernel_scale(&nodes);
        let dim = n + 3;
        let mut a = vec![0.0f64; dim * dim];
        let mut b = vec![0.0f64; dim];
        for i in 0..n {
            let (xi, yi) = nodes[i];
            for j in 0..n {
                let (xj, yj) = nodes[j];
                a[i * dim + j] = kernel(((xi - xj).powi(2) + (yi - yj).powi(2)).sqrt());
            }
            a[i * dim + i] += lambda / weights[i].max(MIN_SAMPLE_WEIGHT);
            a[i * dim + n] = 1.0;
            a[i * dim + n + 1] = xi;
            a[i * dim + n + 2] = yi;
            a[n * dim + i] = 1.0;
            a[(n + 1) * dim + i] = xi;
            a[(n + 2) * dim + i] = yi;
            b[i] = values[i];
        }
        solve_linear_system(&mut a, &mut b, dim)
            .context("Failed to solve the spline system: samples may be duplicated or collinear")?;
        if b.iter().any(|v| !v.is_finite()) {
            anyhow::bail!("Spline solution is not finite");
        }
        Ok(Self {
            nodes,
            weights: b[..n].to_vec(),
            affine: [b[n], b[n + 1], b[n + 2]],
            scale,
        })
    }

    pub fn evaluate(&self, x: f64, y: f64) -> f64 {
        let px = x / self.scale;
        let py = y / self.scale;
        let mut acc = self.affine[0] + self.affine[1] * px + self.affine[2] * py;
        for (&(nx, ny), &w) in self.nodes.iter().zip(&self.weights) {
            acc += w * kernel(((px - nx).powi(2) + (py - ny).powi(2)).sqrt());
        }
        acc
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

#[inline]
fn kernel(r: f64) -> f64 {
    if r > 0.0 {
        r * r * r.ln()
    } else {
        0.0
    }
}

fn mean_nearest_neighbour_distance(nodes: &[(f64, f64)]) -> f64 {
    let n = nodes.len();
    if n < 2 {
        return 1.0;
    }
    let mut sum = 0.0f64;
    for i in 0..n {
        let mut nearest = f64::MAX;
        for j in 0..n {
            if i == j {
                continue;
            }
            let (dx, dy) = (nodes[i].0 - nodes[j].0, nodes[i].1 - nodes[j].1);
            nearest = nearest.min((dx * dx + dy * dy).sqrt());
        }
        sum += nearest;
    }
    sum / n as f64
}

fn neighbour_kernel_scale(nodes: &[(f64, f64)]) -> f64 {
    let d = mean_nearest_neighbour_distance(nodes);
    if d > 0.0 && d < 1.0 {
        d * d * d.ln().abs()
    } else {
        1.0
    }
}

struct Candidate {
    x: f64,
    y: f64,
    manual: bool,
}

fn auto_grid_centres(rows: usize, cols: usize, grid: usize) -> Result<Vec<(f64, f64)>> {
    if grid < 2 {
        anyhow::bail!("Automatic grid must have at least 2 samples per axis, got {}", grid);
    }
    let cell_h = rows / grid;
    let cell_w = cols / grid;
    if cell_h < MIN_AUTO_CELL || cell_w < MIN_AUTO_CELL {
        anyhow::bail!("Image too small for an automatic grid of {} samples per axis", grid);
    }
    let margin_h = cell_h / 4;
    let margin_w = cell_w / 4;
    let inner_h = cell_h - 2 * margin_h;
    let inner_w = cell_w - 2 * margin_w;
    let mut centres = Vec::with_capacity(grid * grid);
    for gy in 0..grid {
        for gx in 0..grid {
            let cy = gy * cell_h + margin_h + inner_h / 2;
            let cx = gx * cell_w + margin_w + inner_w / 2;
            centres.push((cx as f64, cy as f64));
        }
    }
    Ok(centres)
}

fn candidate_positions(rows: usize, cols: usize, cfg: &DbeConfig) -> Result<Vec<Candidate>> {
    let mut out: Vec<Candidate> = cfg
        .manual_samples
        .iter()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|&(x, y)| Candidate { x, y, manual: true })
        .collect();
    let manual_count = out.len();

    let mut auto: Vec<(f64, f64)> = match cfg.auto_grid {
        Some(grid) => auto_grid_centres(rows, cols, grid)?,
        None => Vec::new(),
    };
    auto.retain(|&(ax, ay)| {
        !out.iter().any(|m| {
            let d = ((m.x - ax).powi(2) + (m.y - ay).powi(2)).sqrt();
            d <= DUPLICATE_DISTANCE_PX
        })
    });

    let keep = cfg.max_samples.saturating_sub(manual_count).min(auto.len());
    if keep < auto.len() {
        let total = auto.len();
        let thinned: Vec<(f64, f64)> = (0..keep).map(|i| auto[i * total / keep.max(1)]).collect();
        auto = thinned;
    }
    out.extend(auto.into_iter().map(|(x, y)| Candidate { x, y, manual: false }));
    Ok(out)
}

struct BoxStats {
    median: f64,
    sigma: f64,
    max: f64,
    finite_fraction: f64,
}

fn box_stats(image: &Array2<f32>, x: f64, y: f64, radius: usize) -> Option<BoxStats> {
    let (rows, cols) = image.dim();
    let cx = x.round() as i64;
    let cy = y.round() as i64;
    if cx < 0 || cy < 0 || cx >= cols as i64 || cy >= rows as i64 {
        return None;
    }
    let r = radius as i64;
    let y0 = (cy - r).max(0) as usize;
    let y1 = (cy + r).min(rows as i64 - 1) as usize;
    let x0 = (cx - r).max(0) as usize;
    let x1 = (cx + r).min(cols as i64 - 1) as usize;
    let nominal = ((2 * radius + 1) * (2 * radius + 1)) as f64;
    let mut vals: Vec<f32> = Vec::with_capacity(((y1 - y0 + 1) * (x1 - x0 + 1)) as usize);
    let mut max = f64::MIN;
    for yy in y0..=y1 {
        for xx in x0..=x1 {
            let v = image[[yy, xx]];
            if v.is_finite() {
                vals.push(v);
                max = max.max(v as f64);
            }
        }
    }
    let finite_fraction = vals.len() as f64 / nominal;
    if vals.is_empty() {
        return Some(BoxStats { median: f64::NAN, sigma: f64::NAN, max: f64::NAN, finite_fraction });
    }
    let (median, sigma) = sigma_clipped_stats(&mut vals, BOX_CLIP_KAPPA, BOX_CLIP_ITERATIONS);
    Some(BoxStats { median, sigma, max, finite_fraction })
}

fn robust_sigma(values: &[f64]) -> f64 {
    let mut finite: Vec<f32> = values.iter().filter(|v| v.is_finite()).map(|&v| v as f32).collect();
    if finite.is_empty() {
        return 0.0;
    }
    let med = median_f32_mut(&mut finite);
    let mut devs: Vec<f32> = finite.iter().map(|v| (v - med).abs()).collect();
    median_f32_mut(&mut devs) as f64 * MAD_TO_SIGMA
}

fn median_of(values: &[f64]) -> f64 {
    let mut finite: Vec<f32> = values.iter().filter(|v| v.is_finite()).map(|&v| v as f32).collect();
    if finite.is_empty() {
        return 0.0;
    }
    median_f32_mut(&mut finite) as f64
}

fn fit_accepted(samples: &[DbeSample], scale: f64, smoothing: f64) -> Result<ThinPlateSpline> {
    let accepted: Vec<&DbeSample> = samples.iter().filter(|s| !s.rejected).collect();
    let points: Vec<(f64, f64)> = accepted.iter().map(|s| (s.x, s.y)).collect();
    let values: Vec<f64> = accepted.iter().map(|s| s.value).collect();
    let weights: Vec<f64> = accepted.iter().map(|s| s.weight).collect();
    ThinPlateSpline::fit(&points, &values, &weights, scale, smoothing)
}

fn reject_by_deviation(
    samples: &mut [DbeSample],
    spline: ThinPlateSpline,
    scale: f64,
    sky_sigma: f64,
    cfg: &DbeConfig,
) -> Result<ThinPlateSpline> {
    let residuals: Vec<f64> = samples
        .iter()
        .map(|s| if s.rejected { f64::NAN } else { s.value - spline.evaluate(s.x, s.y) })
        .collect();
    let floor = RESIDUAL_FLOOR_SIGMAS * robust_sigma(&residuals);
    let threshold = (cfg.tolerance.max(0.0) as f64 * sky_sigma).max(floor);
    let outliers: Vec<usize> = residuals
        .iter()
        .enumerate()
        .filter(|(_, r)| r.is_finite() && r.abs() > threshold)
        .map(|(i, _)| i)
        .collect();
    if outliers.is_empty() {
        return Ok(spline);
    }
    let remaining = spline.node_count() - outliers.len();
    if remaining < MIN_SAMPLES {
        return Ok(spline);
    }
    for i in outliers {
        samples[i].rejected = true;
        samples[i].reason = Some(REASON_DEVIATION);
        samples[i].weight = 0.0;
    }
    fit_accepted(samples, scale, cfg.smoothing as f64)
}

fn evaluate_model(spline: &ThinPlateSpline, rows: usize, cols: usize) -> Array2<f32> {
    let cell = MIN_COARSE_CELL.max(rows.min(cols) / COARSE_NODES_ALONG_SHORT_AXIS);
    let ny = (rows - 1) / cell + 2;
    let nx = (cols - 1) / cell + 2;
    let nodes: Vec<f64> = (0..ny)
        .into_par_iter()
        .flat_map_iter(|ky| {
            let y = (ky * cell) as f64;
            (0..nx).map(move |kx| spline.evaluate((kx * cell) as f64, y))
        })
        .collect();
    let axis = |len: usize| -> Vec<(usize, usize, f64)> {
        (0..len)
            .map(|p| {
                let g = p as f64 / cell as f64;
                let k0 = g.floor() as usize;
                (k0, k0 + 1, g - k0 as f64)
            })
            .collect()
    };
    let xs = axis(cols);
    let ys = axis(rows);
    let xs = &xs;
    let nodes = &nodes;
    let data: Vec<f32> = (0..rows)
        .into_par_iter()
        .flat_map_iter(|y| {
            let (ky0, ky1, ty) = ys[y];
            let row0 = &nodes[ky0 * nx..(ky0 + 1) * nx];
            let row1 = &nodes[ky1 * nx..(ky1 + 1) * nx];
            xs.iter().map(move |&(kx0, kx1, tx)| {
                let top = row0[kx0] + (row0[kx1] - row0[kx0]) * tx;
                let bottom = row1[kx0] + (row1[kx1] - row1[kx0]) * tx;
                (top + (bottom - top) * ty) as f32
            })
        })
        .collect();
    Array2::from_shape_vec((rows, cols), data).expect("model shape matches image")
}

fn correct(image: &Array2<f32>, model: &Array2<f32>, mode: &BackgroundMode, normalize: bool) -> Array2<f32> {
    if normalize {
        return apply_correction(image, model, mode);
    }
    let data: Vec<f32> = image
        .as_slice()
        .expect("contiguous image")
        .par_iter()
        .zip(model.as_slice().expect("contiguous model").par_iter())
        .map(|(&img, &bg)| match mode {
            BackgroundMode::Subtract => img - bg,
            BackgroundMode::Divide => {
                if bg > DIVIDE_FLOOR {
                    img / bg
                } else {
                    img
                }
            }
        })
        .collect();
    Array2::from_shape_vec(image.dim(), data).expect("corrected shape matches image")
}

fn check_cancelled(progress: Option<&ProgressHandle>) -> Result<()> {
    if let Some(p) = progress {
        if p.is_cancelled() {
            return Err(AppError::Cancelled.into());
        }
    }
    Ok(())
}

pub fn extract_background_dbe(
    image: &Array2<f32>,
    cfg: &DbeConfig,
    progress: Option<&ProgressHandle>,
) -> Result<DbeResult> {
    let start = std::time::Instant::now();
    let (rows, cols) = image.dim();
    if rows == 0 || cols == 0 {
        anyhow::bail!("Cannot extract a background from an empty image");
    }
    image.as_slice().context("Image not contiguous")?;
    if cfg.sample_radius == 0 {
        anyhow::bail!("Sample radius must be at least 1 pixel");
    }

    if let Some(p) = progress {
        p.set_total(PROGRESS_STAGES);
        p.tick_with_stage("evaluating background samples");
    }

    let candidates = candidate_positions(rows, cols, cfg)?;
    if candidates.is_empty() {
        anyhow::bail!("DBE needs an automatic grid or manual samples");
    }
    let evaluated: Vec<Option<BoxStats>> = candidates
        .par_iter()
        .map(|c| box_stats(image, c.x, c.y, cfg.sample_radius))
        .collect();

    let usable_sigmas: Vec<f64> = evaluated
        .iter()
        .flatten()
        .filter(|b| b.finite_fraction >= MIN_FINITE_FRACTION && b.sigma.is_finite())
        .map(|b| b.sigma)
        .collect();
    let sky_sigma = median_of(&usable_sigmas);

    let mut samples: Vec<DbeSample> = candidates
        .iter()
        .zip(evaluated.iter())
        .map(|(c, stats)| {
            let base = DbeSample {
                x: c.x,
                y: c.y,
                value: f64::NAN,
                weight: 0.0,
                rejected: true,
                reason: None,
                manual: c.manual,
            };
            match stats {
                None => DbeSample { reason: Some(REASON_BOUNDS), ..base },
                Some(b) if b.finite_fraction < MIN_FINITE_FRACTION => {
                    DbeSample { value: b.median, reason: Some(REASON_FINITE), ..base }
                }
                Some(b) if cfg.reject_stars && b.max > b.median + STAR_SIGMAS * sky_sigma.max(b.sigma) => {
                    DbeSample { value: b.median, reason: Some(REASON_STAR), ..base }
                }
                Some(b) => DbeSample {
                    value: b.median,
                    weight: b.finite_fraction.min(1.0),
                    rejected: false,
                    ..base
                },
            }
        })
        .collect();

    check_cancelled(progress)?;
    if let Some(p) = progress {
        p.tick_with_stage("fitting thin-plate spline");
    }

    let scale = rows.max(cols) as f64;
    let spline = fit_accepted(&samples, scale, cfg.smoothing as f64)?;
    let spline = reject_by_deviation(&mut samples, spline, scale, sky_sigma, cfg)?;

    check_cancelled(progress)?;
    if let Some(p) = progress {
        p.tick_with_stage("evaluating spline model");
    }

    let model = evaluate_model(&spline, rows, cols);

    if let Some(p) = progress {
        p.tick_with_stage("applying correction");
    }

    let corrected = correct(image, &model, &cfg.mode, cfg.normalize);

    let accepted: Vec<&DbeSample> = samples.iter().filter(|s| !s.rejected).collect();
    let sum_sq: f64 = accepted
        .iter()
        .map(|s| {
            let d = s.value - spline.evaluate(s.x, s.y);
            d * d
        })
        .sum();
    let rms_residual = (sum_sq / accepted.len().max(1) as f64).sqrt();
    let sample_count = accepted.len();
    let rejected_count = samples.len() - sample_count;

    if let Some(p) = progress {
        p.emit_complete();
    }

    Ok(DbeResult {
        model,
        corrected,
        samples,
        sample_count,
        rejected_count,
        rms_residual,
        elapsed_ms: start.elapsed().as_millis() as u64,
        spline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::background::{extract_background, BackgroundConfig};

    struct Lcg(u64);

    impl Lcg {
        fn uniform(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as f64 / (1u64 << 31) as f64
        }

        fn gaussian(&mut self) -> f64 {
            (0..12).map(|_| self.uniform()).sum::<f64>() - 6.0
        }
    }

    fn gradient_bump_truth(rows: usize, cols: usize) -> Array2<f32> {
        let span = 60.0f64;
        let amplitude = 0.3 * span;
        let sigma = 40.0f64;
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            let gx = x as f64 / (cols - 1) as f64;
            let gy = y as f64 / (rows - 1) as f64;
            let gradient = 100.0 + span * (0.7 * gx + 0.3 * gy);
            let dx = x as f64 - 128.0;
            let dy = y as f64 - 128.0;
            let bump = amplitude * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
            (gradient + bump) as f32
        })
    }

    fn rms_fraction_of_range(model: &Array2<f32>, truth: &Array2<f32>) -> f64 {
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        let mut sum_sq = 0.0f64;
        let mut n = 0usize;
        for (m, t) in model.iter().zip(truth.iter()) {
            if !t.is_finite() {
                continue;
            }
            let t = *t as f64;
            lo = lo.min(t);
            hi = hi.max(t);
            let d = *m as f64 - t;
            sum_sq += d * d;
            n += 1;
        }
        (sum_sq / n as f64).sqrt() / (hi - lo)
    }

    #[test]
    fn spline_recovers_gradient_and_bump_where_cubic_polynomial_cannot() {
        let truth = gradient_bump_truth(256, 256);
        let dbe = extract_background_dbe(&truth, &DbeConfig::default(), None).unwrap();
        let spline_error = rms_fraction_of_range(&dbe.model, &truth);
        assert!(
            spline_error < 0.01,
            "spline rms error {} of range should be below 1%",
            spline_error
        );
        assert_eq!(dbe.rejected_count, 0, "{:?}", dbe.samples.iter().filter(|s| s.rejected).collect::<Vec<_>>());

        let poly_cfg = BackgroundConfig {
            grid_size: 8,
            poly_degree: 3,
            sigma_clip: 2.5,
            iterations: 3,
            mode: BackgroundMode::Subtract,
        };
        let poly = extract_background(&truth, &poly_cfg, None).unwrap();
        let poly_error = rms_fraction_of_range(&poly.model, &truth);
        assert!(
            poly_error > 0.03,
            "degree-3 polynomial rms error {} of range should exceed 3%",
            poly_error
        );
    }

    #[test]
    fn zero_smoothing_reproduces_sample_values_at_centres() {
        let rows = 256;
        let cols = 256;
        let image = Array2::from_shape_fn((rows, cols), |(y, x)| {
            let gx = x as f64 / (cols - 1) as f64;
            let gy = y as f64 / (rows - 1) as f64;
            let wave = 0.05 * (std::f64::consts::TAU * gx).sin() * (std::f64::consts::TAU * gy).cos();
            (0.2 + 0.1 * gx + 0.05 * gy + wave) as f32
        });
        let manual: Vec<(f64, f64)> = (0..20)
            .map(|i| {
                let x = 16 + 8 * ((i * 7) % 29);
                let y = 16 + 8 * ((i * 11 + 3) % 29);
                (x as f64, y as f64)
            })
            .collect();
        let cfg = DbeConfig {
            auto_grid: None,
            manual_samples: manual.clone(),
            smoothing: 0.0,
            reject_stars: false,
            ..DbeConfig::default()
        };
        let res = extract_background_dbe(&image, &cfg, None).unwrap();
        assert_eq!(res.sample_count, 20);
        assert_eq!(res.rejected_count, 0);
        for s in &res.samples {
            assert!(!s.rejected);
            assert!(s.manual);
            let at_centre = res.spline.evaluate(s.x, s.y);
            assert!(
                (at_centre - s.value).abs() < 1e-4,
                "spline at ({}, {}) = {} but sample value is {}",
                s.x, s.y, at_centre, s.value
            );
            let model_value = res.model[[s.y as usize, s.x as usize]] as f64;
            assert!(
                (model_value - s.value).abs() < 1e-4,
                "model at ({}, {}) = {} but sample value is {}",
                s.x, s.y, model_value, s.value
            );
        }
        assert!(res.rms_residual < 1e-6, "rms residual {}", res.rms_residual);
    }

    #[test]
    fn star_box_rejected_with_reason_star_and_nan_box_with_reason_finite() {
        let rows = 128;
        let cols = 128;
        let mut rng = Lcg(7);
        let mut image = Array2::from_shape_fn((rows, cols), |_| 0.0f32);
        for y in 0..rows {
            for x in 0..cols {
                let dx = x as f64 - 64.0;
                let dy = y as f64 - 64.0;
                let star = 80.0 * (-(dx * dx + dy * dy) / (2.0 * 1.5 * 1.5)).exp();
                image[[y, x]] = (100.0 + rng.gaussian() + star) as f32;
            }
        }
        for y in 80..110 {
            for x in 20..50 {
                image[[y, x]] = f32::NAN;
            }
        }
        let cfg = DbeConfig {
            auto_grid: Some(6),
            manual_samples: vec![(64.0, 64.0), (35.0, 95.0)],
            ..DbeConfig::default()
        };
        let res = extract_background_dbe(&image, &cfg, None).unwrap();
        let star = res.samples.iter().find(|s| s.manual && s.x == 64.0 && s.y == 64.0).unwrap();
        assert!(star.rejected);
        assert_eq!(star.reason, Some(REASON_STAR));
        assert_eq!(star.weight, 0.0);
        let hole = res.samples.iter().find(|s| s.manual && s.x == 35.0 && s.y == 95.0).unwrap();
        assert!(hole.rejected);
        assert_eq!(hole.reason, Some(REASON_FINITE));
        assert!(res.sample_count >= 20, "accepted {}", res.sample_count);
        assert!(res.rejected_count >= 2);
        assert_eq!(res.sample_count + res.rejected_count, res.samples.len());
        let interior = res.corrected[[10, 100]];
        assert!((interior - 100.0).abs() < 5.0, "corrected interior {}", interior);
    }

    #[test]
    fn manual_only_samples_fit_a_plane() {
        let rows = 96;
        let cols = 96;
        let plane = Array2::from_shape_fn((rows, cols), |(y, x)| 50.0 + 0.1 * x as f32 + 0.2 * y as f32);
        let manual = vec![
            (10.0, 10.0),
            (85.0, 10.0),
            (10.0, 85.0),
            (85.0, 85.0),
            (48.0, 48.0),
            (30.0, 70.0),
            (70.0, 30.0),
            (48.0, 12.0),
        ];
        let cfg = DbeConfig {
            auto_grid: None,
            manual_samples: manual,
            ..DbeConfig::default()
        };
        let res = extract_background_dbe(&plane, &cfg, None).unwrap();
        assert_eq!(res.sample_count, 8);
        assert_eq!(res.rejected_count, 0);
        assert!(res.samples.iter().all(|s| s.manual));
        let err = rms_fraction_of_range(&res.model, &plane);
        assert!(err < 1e-3, "plane rms error {}", err);
        let mut corrected: Vec<f32> = res.corrected.iter().copied().collect();
        let level = median_f32_mut(&mut corrected);
        for v in &corrected {
            assert!((v - level).abs() < 0.05, "corrected plane not flat: {} vs {}", v, level);
        }
    }

    #[test]
    fn thinning_drops_automatic_samples_only() {
        let rows = 256;
        let cols = 256;
        let plane = Array2::from_shape_fn((rows, cols), |(y, x)| 50.0 + 0.1 * x as f32 + 0.2 * y as f32);
        let manual = vec![(33.0, 47.0), (200.0, 61.0), (127.0, 129.0), (40.0, 210.0), (222.0, 222.0)];
        let cfg = DbeConfig {
            auto_grid: Some(10),
            manual_samples: manual.clone(),
            max_samples: 40,
            ..DbeConfig::default()
        };
        let res = extract_background_dbe(&plane, &cfg, None).unwrap();
        assert_eq!(res.samples.len(), 40);
        let manual_kept: Vec<(f64, f64)> = res.samples.iter().filter(|s| s.manual).map(|s| (s.x, s.y)).collect();
        assert_eq!(manual_kept, manual);
        assert_eq!(res.samples.iter().filter(|s| !s.manual).count(), 35);
        assert_eq!(res.rejected_count, 0);

        let unthinned = DbeConfig {
            auto_grid: Some(10),
            manual_samples: manual.clone(),
            ..DbeConfig::default()
        };
        let full = extract_background_dbe(&plane, &unthinned, None).unwrap();
        assert_eq!(full.samples.len(), 105);
    }

    #[test]
    fn nan_border_stays_nan_and_model_is_finite() {
        let rows = 256;
        let cols = 256;
        let border = 8;
        let image = Array2::from_shape_fn((rows, cols), |(y, x)| {
            if y < border || x < border || y >= rows - border || x >= cols - border {
                f32::NAN
            } else {
                100.0 + 0.2 * x as f32 + 0.1 * y as f32
            }
        });
        let res = extract_background_dbe(&image, &DbeConfig::default(), None).unwrap();
        assert!(res.model.iter().all(|v| v.is_finite()));
        assert!(res.sample_count > 50, "accepted {}", res.sample_count);
        for y in 0..rows {
            for x in 0..cols {
                let inside = y >= border && x >= border && y < rows - border && x < cols - border;
                let c = res.corrected[[y, x]];
                if inside {
                    assert!(c.is_finite(), "interior ({}, {}) became {}", x, y, c);
                } else {
                    assert!(c.is_nan(), "border ({}, {}) became {}", x, y, c);
                }
            }
        }
        let interior_error = rms_fraction_of_range(&res.model, &image);
        assert!(interior_error < 0.01, "model rms error {}", interior_error);
    }

    #[test]
    fn normalize_off_and_divide_mode() {
        let flat = Array2::from_elem((96, 96), 100.0f32);
        let subtract_raw = DbeConfig {
            normalize: false,
            ..DbeConfig::default()
        };
        let res = extract_background_dbe(&flat, &subtract_raw, None).unwrap();
        assert!(res.corrected.iter().all(|v| v.abs() < 1e-3), "raw subtract should leave ~0");

        let divide_raw = DbeConfig {
            normalize: false,
            mode: BackgroundMode::Divide,
            ..DbeConfig::default()
        };
        let res = extract_background_dbe(&flat, &divide_raw, None).unwrap();
        assert!(res.corrected.iter().all(|v| (v - 1.0).abs() < 1e-4), "raw divide should leave ~1");

        let res = extract_background_dbe(&flat, &DbeConfig::default(), None).unwrap();
        assert!(res.corrected.iter().all(|v| (v - 100.0).abs() < 1e-2), "normalized subtract keeps the median");
    }

    #[test]
    fn smoothing_scales_with_sample_spacing() {
        let sparse: Vec<(f64, f64)> = (0..5)
            .flat_map(|i| (0..5).map(move |j| (0.1 + 0.2 * i as f64, 0.1 + 0.2 * j as f64)))
            .collect();
        let dense: Vec<(f64, f64)> = (0..10)
            .flat_map(|i| (0..10).map(move |j| (0.05 + 0.1 * i as f64, 0.05 + 0.1 * j as f64)))
            .collect();
        let sparse_scale = neighbour_kernel_scale(&sparse);
        let dense_scale = neighbour_kernel_scale(&dense);
        assert!((sparse_scale - 0.04 * (0.2f64).ln().abs()).abs() < 1e-9, "{}", sparse_scale);
        assert!((dense_scale - 0.01 * (0.1f64).ln().abs()).abs() < 1e-9, "{}", dense_scale);
        assert!(sparse_scale > dense_scale);
        assert_eq!(neighbour_kernel_scale(&[(0.5, 0.5)]), 1.0);
    }

    #[test]
    fn too_few_samples_is_an_error() {
        let flat = Array2::from_elem((64, 64), 100.0f32);
        let cfg = DbeConfig {
            auto_grid: None,
            manual_samples: vec![(10.0, 10.0), (50.0, 50.0)],
            ..DbeConfig::default()
        };
        let err = extract_background_dbe(&flat, &cfg, None).unwrap_err();
        assert!(err.to_string().contains("samples"), "{}", err);

        let none = DbeConfig {
            auto_grid: None,
            ..DbeConfig::default()
        };
        assert!(extract_background_dbe(&flat, &none, None).is_err());

        let tiny = Array2::from_elem((16, 16), 100.0f32);
        let dense = DbeConfig {
            auto_grid: Some(10),
            ..DbeConfig::default()
        };
        assert!(extract_background_dbe(&tiny, &dense, None).is_err());
    }
}
