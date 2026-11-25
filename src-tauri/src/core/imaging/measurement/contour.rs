use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};

use ndarray::Array2;
use rayon::prelude::*;
use serde::Serialize;

use crate::core::analysis::star_detection::estimate_background;
use crate::core::imaging::stats::is_valid_pixel;

pub const MAX_CONTOUR_LEVELS: usize = 32;
pub const MAX_CONTOUR_POINTS: usize = 2_000_000;
pub const MAX_CONTOUR_PIXELS: usize = 16_000_000;
pub const MAX_SMOOTH_SIGMA: f64 = 20.0;
pub const MAX_BIN: usize = 8;
pub const BACKGROUND_TILE: usize = 64;

const KERNEL_RADIUS_SIGMAS: f64 = 3.0;
const TRACE_BAND_ROWS: usize = 64;
const EDGE_HORIZONTAL: u64 = 0;
const EDGE_VERTICAL: u64 = 1 << 63;
const EDGE_ROW_MASK: u64 = 0x7FFF_FFFF;
const EDGE_COL_MASK: u64 = 0xFFFF_FFFF;
const POINTS_HINT: &str = "increase the bin factor or the smoothing";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelMode {
    List,
    Linear,
    Log,
    Sqrt,
    Sigma,
}

impl LevelMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "list" => Ok(Self::List),
            "linear" => Ok(Self::Linear),
            "log" => Ok(Self::Log),
            "sqrt" => Ok(Self::Sqrt),
            "sigma" => Ok(Self::Sigma),
            other => Err(format!(
                "mode must be one of list, linear, log, sqrt or sigma, got '{}'",
                other
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Linear => "linear",
            Self::Log => "log",
            Self::Sqrt => "sqrt",
            Self::Sigma => "sigma",
        }
    }
}

pub struct LevelSpec<'a> {
    pub mode: LevelMode,
    pub list: &'a [f64],
    pub n: usize,
    pub lo: f64,
    pub hi: f64,
    pub sigma_multiples: &'a [f64],
}

pub struct ContourConfig<'a> {
    pub spec: LevelSpec<'a>,
    pub smooth_sigma_px: f64,
    pub bin: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContourLevel {
    pub value: f64,
    pub polylines: Vec<Vec<(f64, f64)>>,
    pub closed: Vec<bool>,
    pub n_points: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContourSet {
    pub levels: Vec<ContourLevel>,
    pub bin: usize,
    pub n_points: usize,
    pub background_median: f64,
    pub background_sigma: f64,
    pub notes: Vec<String>,
}

fn generated_levels(mode: LevelMode, n: usize, lo: f64, hi: f64) -> Result<Vec<f64>, String> {
    if !(1..=MAX_CONTOUR_LEVELS).contains(&n) {
        return Err(format!(
            "n_levels must be between 1 and {}, got {}",
            MAX_CONTOUR_LEVELS, n
        ));
    }
    if !lo.is_finite() || !hi.is_finite() {
        return Err(format!("lo and hi must be finite numbers, got {} and {}", lo, hi));
    }
    if lo >= hi {
        return Err(format!("lo must be less than hi, got {} and {}", lo, hi));
    }
    if mode == LevelMode::Log && lo <= 0.0 {
        return Err(format!("lo must be greater than 0 in log mode, got {}", lo));
    }
    if mode == LevelMode::Sqrt && lo < 0.0 {
        return Err(format!("lo must be at least 0 in sqrt mode, got {}", lo));
    }
    let (forward, inverse): (fn(f64) -> f64, fn(f64) -> f64) = match mode {
        LevelMode::Log => (f64::ln, f64::exp),
        LevelMode::Sqrt => (f64::sqrt, |v| v * v),
        _ => (|v| v, |v| v),
    };
    let a = forward(lo);
    let b = forward(hi);
    let levels = (0..n)
        .map(|i| {
            if n == 1 {
                inverse(0.5 * (a + b))
            } else if i == 0 {
                lo
            } else if i == n - 1 {
                hi
            } else {
                inverse(a + (b - a) * i as f64 / (n - 1) as f64)
            }
        })
        .collect();
    Ok(levels)
}

pub fn contour_levels(spec: &LevelSpec, median: f64, sigma: f64) -> Result<Vec<f64>, String> {
    let raw: Vec<f64> = match spec.mode {
        LevelMode::List => {
            if spec.list.is_empty() {
                return Err("levels must contain at least one value in list mode".to_string());
            }
            spec.list.to_vec()
        }
        LevelMode::Sigma => {
            if spec.sigma_multiples.is_empty() {
                return Err("sigma_multiples must contain at least one value in sigma mode".to_string());
            }
            if !median.is_finite() || !sigma.is_finite() {
                return Err(format!(
                    "the background estimate is not finite (median {}, sigma {})",
                    median, sigma
                ));
            }
            spec.sigma_multiples.iter().map(|k| median + k * sigma).collect()
        }
        LevelMode::Linear | LevelMode::Log | LevelMode::Sqrt => {
            generated_levels(spec.mode, spec.n, spec.lo, spec.hi)?
        }
    };
    if raw.iter().any(|v| !v.is_finite()) {
        return Err(format!("levels must be finite numbers, got {:?}", raw));
    }
    let mut levels = raw;
    levels.sort_by(|a, b| a.total_cmp(b));
    levels.dedup();
    if levels.len() > MAX_CONTOUR_LEVELS {
        return Err(format!(
            "at most {} contour levels are allowed, got {}",
            MAX_CONTOUR_LEVELS,
            levels.len()
        ));
    }
    Ok(levels)
}

fn row_major<T: Copy>(arr: &Array2<T>) -> Cow<'_, [T]> {
    match arr.as_slice() {
        Some(s) => Cow::Borrowed(s),
        None => Cow::Owned(arr.iter().copied().collect()),
    }
}

fn from_row_major<T: Clone>(rows: usize, cols: usize, buf: Vec<T>, fallback: &Array2<T>) -> Array2<T> {
    Array2::from_shape_vec((rows, cols), buf).unwrap_or_else(|_| fallback.clone())
}

pub fn block_mean_nan(arr: &Array2<f32>, bin: usize) -> Array2<f32> {
    let (rows, cols) = arr.dim();
    if bin <= 1 || rows == 0 || cols == 0 {
        return arr.clone();
    }
    let out_rows = rows.div_ceil(bin);
    let out_cols = cols.div_ceil(bin);
    let src = row_major(arr);
    let mut buf = vec![f32::NAN; out_rows * out_cols];
    buf.par_chunks_mut(out_cols).enumerate().for_each(|(by, out)| {
        let y0 = by * bin;
        let y1 = (y0 + bin).min(rows);
        for (bx, px) in out.iter_mut().enumerate() {
            let x0 = bx * bin;
            let x1 = (x0 + bin).min(cols);
            let mut sum = 0.0f64;
            let mut count = 0usize;
            for y in y0..y1 {
                for &v in &src[y * cols + x0..y * cols + x1] {
                    if v.is_finite() {
                        sum += v as f64;
                        count += 1;
                    }
                }
            }
            *px = if count > 0 { (sum / count as f64) as f32 } else { f32::NAN };
        }
    });
    from_row_major(out_rows, out_cols, buf, arr)
}

fn blur_line(values: impl Fn(usize) -> f32, centre: usize, len: usize, weights: &[f64], radius: usize) -> f32 {
    let lo = centre.saturating_sub(radius);
    let hi = (centre + radius).min(len - 1);
    let mut sum = 0.0f64;
    let mut wsum = 0.0f64;
    for k in lo..=hi {
        let v = values(k);
        if v.is_finite() {
            let w = weights[k + radius - centre];
            sum += v as f64 * w;
            wsum += w;
        }
    }
    if wsum > 0.0 { (sum / wsum) as f32 } else { f32::NAN }
}

pub fn gaussian_blur_nan(arr: &Array2<f32>, sigma_px: f64) -> Array2<f32> {
    let (rows, cols) = arr.dim();
    if !sigma_px.is_finite() || sigma_px <= 0.0 || rows == 0 || cols == 0 {
        return arr.clone();
    }
    let radius = (KERNEL_RADIUS_SIGMAS * sigma_px.min(MAX_SMOOTH_SIGMA)).ceil() as usize;
    let weights: Vec<f64> = (0..=2 * radius)
        .map(|k| {
            let d = k as f64 - radius as f64;
            (-0.5 * d * d / (sigma_px * sigma_px)).exp()
        })
        .collect();
    let src = row_major(arr);
    let mut horizontal = vec![f32::NAN; rows * cols];
    horizontal.par_chunks_mut(cols).enumerate().for_each(|(i, out)| {
        let row = &src[i * cols..(i + 1) * cols];
        for (j, px) in out.iter_mut().enumerate() {
            if row[j].is_finite() {
                *px = blur_line(|k| row[k], j, cols, &weights, radius);
            }
        }
    });
    let mut vertical = vec![f32::NAN; rows * cols];
    vertical.par_chunks_mut(cols).enumerate().for_each(|(i, out)| {
        for (j, px) in out.iter_mut().enumerate() {
            if horizontal[i * cols + j].is_finite() {
                *px = blur_line(|k| horizontal[k * cols + j], i, rows, &weights, radius);
            }
        }
    });
    from_row_major(rows, cols, vertical, arr)
}

fn edge_key(kind: u64, row: usize, col: usize) -> u64 {
    kind | ((row as u64 & EDGE_ROW_MASK) << 32) | (col as u64 & EDGE_COL_MASK)
}

fn crossing_fraction(level: f64, v0: f64, v1: f64) -> f64 {
    let d = v1 - v0;
    if d == 0.0 || !d.is_finite() {
        return 0.5;
    }
    ((level - v0) / d).clamp(0.0, 1.0)
}

fn edge_point(src: &[f32], cols: usize, level: f64, key: u64) -> (f64, f64) {
    let vertical = key & EDGE_VERTICAL != 0;
    let row = ((key >> 32) & EDGE_ROW_MASK) as usize;
    let col = (key & EDGE_COL_MASK) as usize;
    let at = |r: usize, c: usize| src.get(r * cols + c).copied().unwrap_or(f32::NAN) as f64;
    let v0 = at(row, col);
    if vertical {
        let t = crossing_fraction(level, v0, at(row + 1, col));
        (col as f64, row as f64 + t)
    } else {
        let t = crossing_fraction(level, v0, at(row, col + 1));
        (col as f64 + t, row as f64)
    }
}

fn cell_segments(case: u8, centre_high: bool, i: usize, j: usize, out: &mut Vec<(u64, u64)>) {
    let top = edge_key(EDGE_HORIZONTAL, i, j);
    let right = edge_key(EDGE_VERTICAL, i, j + 1);
    let bottom = edge_key(EDGE_HORIZONTAL, i + 1, j);
    let left = edge_key(EDGE_VERTICAL, i, j);
    match case {
        1 | 14 => out.push((left, top)),
        2 | 13 => out.push((top, right)),
        3 | 12 => out.push((left, right)),
        4 | 11 => out.push((right, bottom)),
        6 | 9 => out.push((top, bottom)),
        7 | 8 => out.push((bottom, left)),
        5 => {
            if centre_high {
                out.push((top, right));
                out.push((bottom, left));
            } else {
                out.push((left, top));
                out.push((right, bottom));
            }
        }
        10 => {
            if centre_high {
                out.push((left, top));
                out.push((right, bottom));
            } else {
                out.push((top, right));
                out.push((bottom, left));
            }
        }
        _ => {}
    }
}

fn segments_in_case(case: u8) -> usize {
    match case {
        0 | 15 => 0,
        5 | 10 => 2,
        _ => 1,
    }
}

fn trace_bands(rows: usize) -> Vec<(usize, usize)> {
    (0..rows.saturating_sub(1))
        .step_by(TRACE_BAND_ROWS)
        .map(|start| (start, (start + TRACE_BAND_ROWS).min(rows - 1)))
        .collect()
}

fn visit_band(
    src: &[f32],
    mask: Option<&[u8]>,
    cols: usize,
    levels: &[f64],
    band: (usize, usize),
    mut visit: impl FnMut(usize, u8, bool, usize, usize),
) {
    for i in band.0..band.1 {
        let top = &src[i * cols..(i + 1) * cols];
        let bottom = &src[(i + 1) * cols..(i + 2) * cols];
        let mask_rows = mask.map(|m| (&m[i * cols..(i + 1) * cols], &m[(i + 1) * cols..(i + 2) * cols]));
        for j in 0..cols - 1 {
            let a = top[j];
            let b = top[j + 1];
            let c = bottom[j + 1];
            let d = bottom[j];
            if !(is_valid_pixel(a) && is_valid_pixel(b) && is_valid_pixel(c) && is_valid_pixel(d)) {
                continue;
            }
            if let Some((mt, mb)) = mask_rows {
                if mt[j] != 0 || mt[j + 1] != 0 || mb[j] != 0 || mb[j + 1] != 0 {
                    continue;
                }
            }
            let lo = a.min(b).min(c).min(d) as f64;
            let hi = a.max(b).max(c).max(d) as f64;
            let start = levels.partition_point(|&l| l <= lo);
            let end = levels.partition_point(|&l| l <= hi);
            if start >= end {
                continue;
            }
            let centre = 0.25 * (a as f64 + b as f64 + c as f64 + d as f64);
            for (li, &level) in levels.iter().enumerate().take(end).skip(start) {
                let case = u8::from(a as f64 >= level)
                    | u8::from(b as f64 >= level) << 1
                    | u8::from(c as f64 >= level) << 2
                    | u8::from(d as f64 >= level) << 3;
                visit(li, case, centre >= level, i, j);
            }
        }
    }
}

fn count_segments(plane: &Array2<f32>, levels: &[f64], excluded: Option<&Array2<u8>>) -> usize {
    let (rows, cols) = plane.dim();
    if rows < 2 || cols < 2 || levels.is_empty() {
        return 0;
    }
    let src = row_major(plane);
    let mask = excluded.filter(|m| m.dim() == plane.dim()).map(row_major);
    trace_bands(rows)
        .par_iter()
        .map(|&band| {
            let mut total = 0usize;
            visit_band(&src, mask.as_deref(), cols, levels, band, |_, case, _, _, _| {
                total += segments_in_case(case);
            });
            total
        })
        .sum()
}

fn collect_segments(
    plane: &Array2<f32>,
    levels: &[f64],
    excluded: Option<&Array2<u8>>,
) -> Vec<Vec<(u64, u64)>> {
    let (rows, cols) = plane.dim();
    let n = levels.len();
    if rows < 2 || cols < 2 || n == 0 {
        return vec![Vec::new(); n];
    }
    let src = row_major(plane);
    let mask = excluded.filter(|m| m.dim() == plane.dim()).map(row_major);
    let per_band: Vec<Vec<Vec<(u64, u64)>>> = trace_bands(rows)
        .par_iter()
        .map(|&band| {
            let mut out: Vec<Vec<(u64, u64)>> = vec![Vec::new(); n];
            visit_band(&src, mask.as_deref(), cols, levels, band, |li, case, centre_high, i, j| {
                if let Some(segs) = out.get_mut(li) {
                    cell_segments(case, centre_high, i, j, segs);
                }
            });
            out
        })
        .collect();
    let mut all: Vec<Vec<(u64, u64)>> = (0..n)
        .map(|li| Vec::with_capacity(per_band.iter().map(|b| b[li].len()).sum()))
        .collect();
    for band in per_band {
        for (li, segs) in band.into_iter().enumerate() {
            all[li].extend(segs);
        }
    }
    all
}

fn stitch(plane: &Array2<f32>, level: f64, segments: &[(u64, u64)]) -> ContourLevel {
    let cols = plane.dim().1;
    let src = row_major(plane);
    let mut chains: Vec<VecDeque<u64>> = Vec::new();
    let mut closed: Vec<bool> = Vec::new();
    let mut ends: HashMap<u64, (usize, bool)> = HashMap::with_capacity(segments.len() / 8 + 1);
    for &(k0, k1) in segments {
        match (ends.remove(&k0), ends.remove(&k1)) {
            (None, None) => {
                let idx = chains.len();
                chains.push(VecDeque::from([k0, k1]));
                closed.push(false);
                ends.insert(k0, (idx, false));
                ends.insert(k1, (idx, true));
            }
            (Some((ia, at_back)), None) => {
                if at_back {
                    chains[ia].push_back(k1);
                } else {
                    chains[ia].push_front(k1);
                }
                ends.insert(k1, (ia, at_back));
            }
            (None, Some((ib, at_back))) => {
                if at_back {
                    chains[ib].push_back(k0);
                } else {
                    chains[ib].push_front(k0);
                }
                ends.insert(k0, (ib, at_back));
            }
            (Some((ia, ea)), Some((ib, eb))) => {
                if ia == ib {
                    closed[ia] = true;
                    continue;
                }
                let (big, side_big, small_idx, side_small) =
                    if chains[ia].len() >= chains[ib].len() { (ia, ea, ib, eb) } else { (ib, eb, ia, ea) };
                let small = std::mem::take(&mut chains[small_idx]);
                let far = if side_small { small.front() } else { small.back() }.copied();
                match (side_big, side_small) {
                    (true, false) => chains[big].extend(small),
                    (true, true) => chains[big].extend(small.into_iter().rev()),
                    (false, false) => {
                        for k in small {
                            chains[big].push_front(k);
                        }
                    }
                    (false, true) => {
                        for k in small.into_iter().rev() {
                            chains[big].push_front(k);
                        }
                    }
                }
                if let Some(far) = far {
                    ends.insert(far, (big, side_big));
                }
            }
        }
    }
    let mut polylines = Vec::new();
    let mut closed_flags = Vec::new();
    let mut n_points = 0usize;
    for (chain, is_closed) in chains.into_iter().zip(closed) {
        if chain.len() < 2 {
            continue;
        }
        let points: Vec<(f64, f64)> = chain.iter().map(|&k| edge_point(&src, cols, level, k)).collect();
        n_points += points.len();
        polylines.push(points);
        closed_flags.push(is_closed);
    }
    ContourLevel { value: level, polylines, closed: closed_flags, n_points }
}

pub fn trace_contours(
    plane: &Array2<f32>,
    levels: &[f64],
    excluded: Option<&Array2<u8>>,
) -> Vec<ContourLevel> {
    let segments = collect_segments(plane, levels, excluded);
    levels
        .par_iter()
        .zip(segments.par_iter())
        .map(|(&level, segs)| stitch(plane, level, segs))
        .collect()
}

fn sanitized(arr: &Array2<f32>, excluded: Option<&Array2<u8>>) -> Array2<f32> {
    let (rows, cols) = arr.dim();
    let src = row_major(arr);
    let mask = excluded.map(row_major);
    let mut buf = vec![f32::NAN; rows * cols];
    buf.par_chunks_mut(cols).enumerate().for_each(|(i, out)| {
        let row = &src[i * cols..(i + 1) * cols];
        let mrow = mask.as_deref().map(|m| &m[i * cols..(i + 1) * cols]);
        for (j, px) in out.iter_mut().enumerate() {
            let v = row[j];
            let dropped = !is_valid_pixel(v) || mrow.is_some_and(|m| m[j] != 0);
            *px = if dropped { f32::NAN } else { v };
        }
    });
    from_row_major(rows, cols, buf, arr)
}

fn required_bin(rows: usize, cols: usize) -> usize {
    let ratio = (rows as f64 * cols as f64) / MAX_CONTOUR_PIXELS as f64;
    ratio.sqrt().ceil().max(1.0) as usize
}

fn block_centre(k: usize, bin: usize, n: usize) -> f64 {
    let width = bin.min(n.saturating_sub(k * bin)).max(1);
    k as f64 * bin as f64 + (width as f64 - 1.0) / 2.0
}

fn unbin_axis(v: f64, bin: usize, n: usize) -> f64 {
    let last = n.saturating_sub(1) / bin.max(1);
    let k = (v.floor().max(0.0) as usize).min(last);
    if k == last {
        return block_centre(last, bin, n);
    }
    let c0 = block_centre(k, bin, n);
    c0 + (v - k as f64) * (block_centre(k + 1, bin, n) - c0)
}

pub fn contour_lines(
    arr: &Array2<f32>,
    cfg: &ContourConfig,
    excluded: Option<&Array2<u8>>,
) -> Result<ContourSet, String> {
    let (rows, cols) = arr.dim();
    if rows < 2 || cols < 2 {
        return Err(format!("image must be at least 2 x 2 pixels, got {} x {}", cols, rows));
    }
    if !(1..=MAX_BIN).contains(&cfg.bin) {
        return Err(format!("bin must be between 1 and {}, got {}", MAX_BIN, cfg.bin));
    }
    if !cfg.smooth_sigma_px.is_finite() || !(0.0..=MAX_SMOOTH_SIGMA).contains(&cfg.smooth_sigma_px) {
        return Err(format!(
            "smooth_sigma must be between 0 and {} pixels, got {}",
            MAX_SMOOTH_SIGMA, cfg.smooth_sigma_px
        ));
    }
    let mut notes = Vec::new();
    let excluded = excluded.filter(|m| m.dim() == arr.dim());
    let needed = required_bin(rows, cols);
    let bin = if needed > cfg.bin {
        if needed > MAX_BIN {
            return Err(format!(
                "an image of {} x {} pixels needs bin {} to stay under {} traced pixels, above the maximum of {}",
                cols, rows, needed, MAX_CONTOUR_PIXELS, MAX_BIN
            ));
        }
        notes.push(format!(
            "bin raised from {} to {} to keep the traced plane under {} pixels",
            cfg.bin, needed, MAX_CONTOUR_PIXELS
        ));
        needed
    } else {
        cfg.bin
    };
    let clean = sanitized(arr, excluded);
    let binned = block_mean_nan(&clean, bin);
    let smoothed = gaussian_blur_nan(&binned, cfg.smooth_sigma_px / bin as f64);
    let (median, sigma) = estimate_background(&smoothed, BACKGROUND_TILE);
    let levels = contour_levels(&cfg.spec, median, sigma)?;
    let estimated_points = count_segments(&smoothed, &levels, None);
    if estimated_points > MAX_CONTOUR_POINTS {
        return Err(format!(
            "the contours would have about {} points, above the limit of {}; {}",
            estimated_points, MAX_CONTOUR_POINTS, POINTS_HINT
        ));
    }
    let segments = collect_segments(&smoothed, &levels, None);
    let traced: Vec<ContourLevel> = levels
        .par_iter()
        .zip(segments.par_iter())
        .map(|(&level, segs)| {
            let mut traced = stitch(&smoothed, level, segs);
            if bin > 1 {
                for polyline in &mut traced.polylines {
                    for p in polyline.iter_mut() {
                        *p = (unbin_axis(p.0, bin, cols), unbin_axis(p.1, bin, rows));
                    }
                }
            }
            traced
        })
        .collect();
    let n_points: usize = traced.iter().map(|l| l.n_points).sum();
    if n_points > MAX_CONTOUR_POINTS {
        return Err(format!(
            "the contours have {} points, above the limit of {}; {}",
            n_points, MAX_CONTOUR_POINTS, POINTS_HINT
        ));
    }
    Ok(ContourSet {
        levels: traced,
        bin,
        n_points,
        background_median: median,
        background_sigma: sigma,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAUSS_AMPLITUDE: f64 = 1000.0;
    const GAUSS_SIGMA: f64 = 10.0;
    const GAUSS_BACKGROUND: f64 = 100.0;
    const GAUSS_SIZE: usize = 128;

    fn gaussian_plane() -> Array2<f32> {
        let centre = (GAUSS_SIZE as f64 - 1.0) / 2.0;
        Array2::from_shape_fn((GAUSS_SIZE, GAUSS_SIZE), |(y, x)| {
            let dx = x as f64 - centre;
            let dy = y as f64 - centre;
            let r2 = dx * dx + dy * dy;
            (GAUSS_BACKGROUND + GAUSS_AMPLITUDE * (-r2 / (2.0 * GAUSS_SIGMA * GAUSS_SIGMA)).exp()) as f32
        })
    }

    fn half_max_level() -> f64 {
        GAUSS_BACKGROUND + GAUSS_AMPLITUDE / 2.0
    }

    fn expected_half_max_radius() -> f64 {
        GAUSS_SIGMA * (2.0 * std::f64::consts::LN_2).sqrt()
    }

    fn radii(points: &[(f64, f64)], centre: f64) -> Vec<f64> {
        points.iter().map(|&(x, y)| ((x - centre).powi(2) + (y - centre).powi(2)).sqrt()).collect()
    }

    fn list_spec(list: &[f64]) -> LevelSpec<'_> {
        LevelSpec { mode: LevelMode::List, list, n: 0, lo: f64::NAN, hi: f64::NAN, sigma_multiples: &[] }
    }

    #[test]
    fn half_maximum_contour_of_a_gaussian_is_one_closed_ring_at_the_fwhm_radius() {
        let plane = gaussian_plane();
        let centre = (GAUSS_SIZE as f64 - 1.0) / 2.0;
        let levels = trace_contours(&plane, &[half_max_level()], None);
        assert_eq!(levels.len(), 1);
        let level = &levels[0];
        assert_eq!(level.polylines.len(), 1, "polylines: {}", level.polylines.len());
        assert!(level.closed[0]);
        let r = radii(&level.polylines[0], centre);
        let mean = r.iter().sum::<f64>() / r.len() as f64;
        let expected = expected_half_max_radius();
        assert!((mean - expected).abs() < 0.05, "mean radius {} expected {}", mean, expected);
        let max_dev = r.iter().map(|v| (v - expected).abs()).fold(0.0, f64::max);
        assert!(max_dev < 0.2, "max deviation {}", max_dev);
        assert_eq!(level.n_points, level.polylines[0].len());
    }

    #[test]
    fn a_ramp_plane_contoured_between_two_columns_gives_one_open_vertical_polyline() {
        let plane = Array2::from_shape_fn((32, 32), |(_, x)| x as f32);
        let levels = trace_contours(&plane, &[10.5], None);
        let level = &levels[0];
        assert_eq!(level.polylines.len(), 1);
        assert!(!level.closed[0]);
        let line = &level.polylines[0];
        assert!(line.iter().all(|&(x, _)| (x - 10.5).abs() < 1e-6), "{:?}", line);
        let ys: Vec<f64> = line.iter().map(|p| p.1).collect();
        let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(min_y, 0.0);
        assert_eq!(max_y, 31.0);
        assert_eq!(line.len(), 32);
    }

    #[test]
    fn a_nan_hole_on_the_ring_opens_the_contour_and_no_vertex_lies_inside_the_hole() {
        let mut plane = gaussian_plane();
        let centre = (GAUSS_SIZE as f64 - 1.0) / 2.0;
        let hole_x = (centre + expected_half_max_radius()).round() as usize;
        let hole_y = centre.round() as usize;
        for y in hole_y - 4..=hole_y + 4 {
            for x in hole_x - 4..=hole_x + 4 {
                plane[[y, x]] = f32::NAN;
            }
        }
        let levels = trace_contours(&plane, &[half_max_level()], None);
        let level = &levels[0];
        assert!(!level.polylines.is_empty());
        assert!(level.closed.iter().all(|&c| !c), "expected only open polylines");
        for line in &level.polylines {
            for &(x, y) in line {
                let inside = x > (hole_x as f64 - 4.0) - 1e-9
                    && x < (hole_x as f64 + 4.0) + 1e-9
                    && y > (hole_y as f64 - 4.0) - 1e-9
                    && y < (hole_y as f64 + 4.0) + 1e-9;
                assert!(!inside, "vertex ({}, {}) lies inside the hole", x, y);
            }
        }
    }

    #[test]
    fn a_constant_plane_yields_no_polylines() {
        let plane = Array2::from_elem((16, 16), 5.0f32);
        let levels = trace_contours(&plane, &[1.0, 5.0, 9.0], None);
        assert_eq!(levels.len(), 3);
        assert!(levels.iter().all(|l| l.polylines.is_empty() && l.n_points == 0));
    }

    #[test]
    fn binning_by_four_maps_the_contour_back_to_full_resolution_coordinates() {
        let plane = gaussian_plane();
        let centre = (GAUSS_SIZE as f64 - 1.0) / 2.0;
        let level = [half_max_level()];
        let unbinned = contour_lines(
            &plane,
            &ContourConfig { spec: list_spec(&level), smooth_sigma_px: 0.0, bin: 1 },
            None,
        )
        .unwrap();
        let binned = contour_lines(
            &plane,
            &ContourConfig { spec: list_spec(&level), smooth_sigma_px: 0.0, bin: 4 },
            None,
        )
        .unwrap();
        assert_eq!(binned.bin, 4);
        assert_eq!(binned.levels[0].polylines.len(), 1);
        let line = &binned.levels[0].polylines[0];
        let cx = line.iter().map(|p| p.0).sum::<f64>() / line.len() as f64;
        let cy = line.iter().map(|p| p.1).sum::<f64>() / line.len() as f64;
        assert!((cx - centre).abs() < 0.5, "centroid x {} vs {}", cx, centre);
        assert!((cy - centre).abs() < 0.5, "centroid y {} vs {}", cy, centre);
        let mean_binned = radii(line, centre).iter().sum::<f64>() / line.len() as f64;
        let full = &unbinned.levels[0].polylines[0];
        let mean_full = radii(full, centre).iter().sum::<f64>() / full.len() as f64;
        assert!((mean_binned - mean_full).abs() < 1.0, "radius {} vs {}", mean_binned, mean_full);
    }

    #[test]
    fn gaussian_blur_preserves_the_finite_sum_and_keeps_nan_pixels_nan() {
        let mut plane = gaussian_plane();
        plane[[10, 10]] = f32::NAN;
        plane[[70, 71]] = f32::NAN;
        let before: f64 = plane.iter().filter(|v| v.is_finite()).map(|&v| v as f64).sum();
        let blurred = gaussian_blur_nan(&plane, 2.0);
        let after: f64 = blurred.iter().filter(|v| v.is_finite()).map(|&v| v as f64).sum();
        assert!(((after - before) / before).abs() < 1e-4, "sum {} vs {}", after, before);
        assert!(blurred[[10, 10]].is_nan());
        assert!(blurred[[70, 71]].is_nan());
        assert_eq!(blurred.iter().filter(|v| v.is_nan()).count(), 2);
        let untouched = gaussian_blur_nan(&plane, 0.0);
        assert_eq!(untouched[[5, 5]], plane[[5, 5]]);
    }

    #[test]
    fn block_mean_ignores_the_nan_pixel_of_a_two_by_two_block() {
        let plane = Array2::from_shape_vec((2, 2), vec![1.0f32, 2.0, f32::NAN, 6.0]).unwrap();
        let binned = block_mean_nan(&plane, 2);
        assert_eq!(binned.dim(), (1, 1));
        assert!((binned[[0, 0]] - 3.0).abs() < 1e-6);
        let all_nan = Array2::from_elem((2, 2), f32::NAN);
        assert!(block_mean_nan(&all_nan, 2)[[0, 0]].is_nan());
        let odd = Array2::from_shape_fn((5, 3), |(y, x)| (y * 3 + x) as f32);
        assert_eq!(block_mean_nan(&odd, 2).dim(), (3, 2));
    }

    #[test]
    fn contour_levels_follow_each_mode_and_refuse_bad_input() {
        let sigma_spec = LevelSpec {
            mode: LevelMode::Sigma,
            list: &[],
            n: 0,
            lo: f64::NAN,
            hi: f64::NAN,
            sigma_multiples: &[1.0, 2.0, 3.0],
        };
        assert_eq!(contour_levels(&sigma_spec, 100.0, 5.0).unwrap(), vec![105.0, 110.0, 115.0]);

        let log_spec = LevelSpec { mode: LevelMode::Log, list: &[], n: 3, lo: 0.0, hi: 10.0, sigma_multiples: &[] };
        let err = contour_levels(&log_spec, 0.0, 1.0).unwrap_err();
        assert!(err.contains("lo must be greater than 0 in log mode"), "{err}");

        let linear = LevelSpec { mode: LevelMode::Linear, list: &[], n: 3, lo: 0.0, hi: 10.0, sigma_multiples: &[] };
        assert_eq!(contour_levels(&linear, 0.0, 1.0).unwrap(), vec![0.0, 5.0, 10.0]);

        let too_many = LevelSpec { mode: LevelMode::Linear, list: &[], n: 33, lo: 0.0, hi: 10.0, sigma_multiples: &[] };
        let err = contour_levels(&too_many, 0.0, 1.0).unwrap_err();
        assert!(err.contains("n_levels must be between 1 and 32"), "{err}");

        let long_list: Vec<f64> = (0..33).map(|i| i as f64).collect();
        let err = contour_levels(&list_spec(&long_list), 0.0, 1.0).unwrap_err();
        assert!(err.contains("at most 32 contour levels"), "{err}");

        let dup = [3.0, 1.0, 3.0, 2.0];
        assert_eq!(contour_levels(&list_spec(&dup), 0.0, 1.0).unwrap(), vec![1.0, 2.0, 3.0]);

        let nan_list = [1.0, f64::NAN];
        assert!(contour_levels(&list_spec(&nan_list), 0.0, 1.0).is_err());

        let sqrt = LevelSpec { mode: LevelMode::Sqrt, list: &[], n: 3, lo: 0.0, hi: 16.0, sigma_multiples: &[] };
        let levels = contour_levels(&sqrt, 0.0, 1.0).unwrap();
        assert!((levels[1] - 4.0).abs() < 1e-9, "{:?}", levels);
        let log = LevelSpec { mode: LevelMode::Log, list: &[], n: 3, lo: 1.0, hi: 100.0, sigma_multiples: &[] };
        let levels = contour_levels(&log, 0.0, 1.0).unwrap();
        assert!((levels[1] - 10.0).abs() < 1e-9, "{:?}", levels);
        assert_eq!(LevelMode::parse("SIGMA").unwrap(), LevelMode::Sigma);
        assert!(LevelMode::parse("cubic").is_err());
    }

    #[test]
    fn a_dq_flagged_pixel_inside_a_binned_block_behaves_like_a_nan_pixel_and_keeps_the_ring_closed() {
        let plane = gaussian_plane();
        let centre = (GAUSS_SIZE as f64 - 1.0) / 2.0;
        let hole_x = (centre + expected_half_max_radius()).round() as usize;
        let hole_y = centre.round() as usize;
        let mut mask = Array2::<u8>::zeros(plane.dim());
        mask[[hole_y, hole_x]] = 1;
        let mut nan_plane = plane.clone();
        nan_plane[[hole_y, hole_x]] = f32::NAN;
        let level = [half_max_level()];
        let cfg = ContourConfig { spec: list_spec(&level), smooth_sigma_px: 0.0, bin: 2 };
        let flagged = contour_lines(&plane, &cfg, Some(&mask)).unwrap();
        let as_nan = contour_lines(&nan_plane, &cfg, None).unwrap();
        assert_eq!(flagged.levels[0].polylines.len(), 1);
        assert!(flagged.levels[0].closed[0]);
        assert_eq!(flagged.levels[0].polylines, as_nan.levels[0].polylines);
    }

    #[test]
    fn edge_cases_do_not_panic_and_bad_bin_or_sigma_are_refused() {
        let plane = gaussian_plane();
        let level = [half_max_level()];
        let tiny = Array2::from_elem((1, 1), 1.0f32);
        assert!(contour_lines(&tiny, &ContourConfig { spec: list_spec(&level), smooth_sigma_px: 0.0, bin: 1 }, None).is_err());
        assert!(trace_contours(&tiny, &level, None)[0].polylines.is_empty());
        let empty = Array2::<f32>::zeros((0, 0));
        assert!(trace_contours(&empty, &level, None)[0].polylines.is_empty());
        assert_eq!(block_mean_nan(&empty, 4).dim(), (0, 0));
        assert_eq!(gaussian_blur_nan(&empty, 1.0).dim(), (0, 0));
        let bad_bin = contour_lines(&plane, &ContourConfig { spec: list_spec(&level), smooth_sigma_px: 0.0, bin: 9 }, None);
        assert!(bad_bin.unwrap_err().contains("bin must be between 1 and 8"));
        let bad_sigma = contour_lines(&plane, &ContourConfig { spec: list_spec(&level), smooth_sigma_px: 21.0, bin: 1 }, None);
        assert!(bad_sigma.unwrap_err().contains("smooth_sigma must be between 0 and 20"));
    }

    #[test]
    fn sigma_mode_levels_come_from_the_background_of_the_smoothed_plane() {
        let plane = gaussian_plane();
        let spec = LevelSpec {
            mode: LevelMode::Sigma,
            list: &[],
            n: 0,
            lo: f64::NAN,
            hi: f64::NAN,
            sigma_multiples: &[3.0],
        };
        let set = contour_lines(&plane, &ContourConfig { spec, smooth_sigma_px: 1.0, bin: 1 }, None).unwrap();
        assert!((set.background_median - GAUSS_BACKGROUND).abs() < 1.0, "{}", set.background_median);
        assert_eq!(set.levels.len(), 1);
        assert!((set.levels[0].value - (set.background_median + 3.0 * set.background_sigma)).abs() < 1e-9);
        assert!(set.notes.is_empty());
    }

    #[test]
    fn smoothing_sigma_is_in_image_pixels_whatever_the_bin_factor() {
        let plane = gaussian_plane();
        let centre = (GAUSS_SIZE as f64 - 1.0) / 2.0;
        let smooth = 6.0;
        let effective_var = GAUSS_SIGMA * GAUSS_SIGMA + smooth * smooth;
        let level = [GAUSS_BACKGROUND + 0.5 * GAUSS_AMPLITUDE * GAUSS_SIGMA * GAUSS_SIGMA / effective_var];
        let expected = (2.0 * std::f64::consts::LN_2 * effective_var).sqrt();
        for bin in [1usize, 2, 4] {
            let set = contour_lines(&plane, &ContourConfig { spec: list_spec(&level), smooth_sigma_px: smooth, bin }, None)
                .unwrap();
            assert_eq!(set.levels[0].polylines.len(), 1, "bin {}", bin);
            let line = &set.levels[0].polylines[0];
            let mean = radii(line, centre).iter().sum::<f64>() / line.len() as f64;
            assert!((mean - expected).abs() < 0.15, "bin {} radius {} expected {}", bin, mean, expected);
        }
    }

    fn checkerboard_plane(size: usize) -> Array2<f32> {
        Array2::from_shape_fn((size, size), |(y, x)| if (x + y) % 2 == 0 { 1.0 } else { 2.0 })
    }

    fn dense_levels() -> Vec<f64> {
        (0..32).map(|i| 1.01 + 0.98 * i as f64 / 31.0).collect()
    }

    #[test]
    fn the_segment_count_matches_the_segments_that_are_collected() {
        let mut plane = gaussian_plane();
        plane[[64, 70]] = f32::NAN;
        let mut mask = Array2::<u8>::zeros(plane.dim());
        mask[[20, 20]] = 1;
        let levels = [150.0, 600.0, 1000.0];
        for m in [None, Some(&mask)] {
            let collected: usize = collect_segments(&plane, &levels, m).iter().map(Vec::len).sum();
            assert!(collected > 0);
            assert_eq!(count_segments(&plane, &levels, m), collected);
        }
        let board = checkerboard_plane(64);
        let dense = dense_levels();
        let collected: usize = collect_segments(&board, &dense, None).iter().map(Vec::len).sum();
        assert_eq!(collected, 63 * 63 * 64);
        assert_eq!(count_segments(&board, &dense, None), collected);
    }

    #[test]
    fn a_request_over_the_point_cap_is_refused_from_the_segment_count_before_collection() {
        let plane = checkerboard_plane(256);
        let levels = dense_levels();
        assert_eq!(count_segments(&plane, &levels, None), 255 * 255 * 64);
        let err = contour_lines(&plane, &ContourConfig { spec: list_spec(&levels), smooth_sigma_px: 0.0, bin: 1 }, None)
            .unwrap_err();
        assert!(err.contains("about 4161600 points, above the limit of 2000000"), "{err}");
    }

    #[test]
    fn a_ramp_contour_in_the_trailing_partial_block_lands_on_its_true_column_inside_the_image() {
        let plane = Array2::from_shape_fn((17, 129), |(_, x)| 1000.0 + x as f32);
        let level = [1126.0];
        let set = contour_lines(&plane, &ContourConfig { spec: list_spec(&level), smooth_sigma_px: 0.0, bin: 8 }, None)
            .unwrap();
        assert_eq!(set.bin, 8);
        assert_eq!(set.levels[0].polylines.len(), 1);
        let line = &set.levels[0].polylines[0];
        assert!(line.iter().all(|&(x, _)| (x - 126.0).abs() < 1e-6), "{:?}", line);
        let max_y = line.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(max_y, 16.0);
    }

    #[test]
    fn unbinning_keeps_full_block_centres_and_never_leaves_the_image() {
        for bin in 2..=MAX_BIN {
            for n in 2..200usize {
                let last = (n - 1) / bin;
                for step in 0..=(last * 8) {
                    let v = step as f64 / 8.0;
                    let x = unbin_axis(v, bin, n);
                    assert!((0.0..=(n - 1) as f64).contains(&x), "bin {} n {} v {} gives {}", bin, n, v, x);
                    if (v.floor() as usize + 2) * bin <= n {
                        let full = v * bin as f64 + (bin as f64 - 1.0) / 2.0;
                        assert!((x - full).abs() < 1e-9, "bin {} n {} v {} gives {} not {}", bin, n, v, x, full);
                    }
                }
                assert_eq!(unbin_axis(last as f64, bin, n), block_centre(last, bin, n));
            }
        }
    }
}
