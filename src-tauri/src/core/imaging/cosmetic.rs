use anyhow::{anyhow, bail, Result};
use ndarray::{Array2, Zip};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::math::median::{exact_mad_mut, exact_median_mut, median_f32_mut};
use crate::types::constants::MAD_TO_SIGMA;

pub const FLAG_HOT: u8 = 1;
pub const FLAG_COLD: u8 = 2;
pub const FLAG_LISTED: u8 = 4;

const NEIGHBOUR_OFFSETS: [(isize, isize); 8] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
];
const MIN_NEIGHBOURS_FOR_MEDIAN: usize = 3;
const INNER_RING_STEPS: isize = 1;
const OUTER_RING_STEPS: isize = 2;
const OUTER_RING_CAPACITY: usize = 24;
const MAX_DEFECT_CLUSTER_PIXELS: usize = 3;
const MEAN_ABS_DEV_TO_SIGMA: f64 = 1.2533;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Replacement {
    #[default]
    Median,
    Mean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Defect {
    Point { x: usize, y: usize },
    Column { x: usize, y0: Option<usize>, y1: Option<usize> },
    Row { y: usize, x0: Option<usize>, x1: Option<usize> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CosmeticConfig {
    pub use_master_dark: bool,
    pub dark_hot_sigma: Option<f32>,
    pub dark_cold_sigma: Option<f32>,
    pub auto_hot_sigma: Option<f32>,
    pub auto_cold_sigma: Option<f32>,
    pub defects: Vec<Defect>,
    pub cfa: bool,
    pub amount: f32,
    pub replacement: Replacement,
}

impl Default for CosmeticConfig {
    fn default() -> Self {
        Self {
            use_master_dark: true,
            dark_hot_sigma: Some(3.0),
            dark_cold_sigma: None,
            auto_hot_sigma: None,
            auto_cold_sigma: None,
            defects: Vec::new(),
            cfa: false,
            amount: 1.0,
            replacement: Replacement::Median,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CosmeticResult {
    pub corrected: Array2<f32>,
    pub flagged: usize,
    pub replaced: usize,
    pub hot: usize,
    pub cold: usize,
    pub listed: usize,
}

fn strip_comment(line: &str) -> &str {
    let cut = [line.find('#'), line.find("//")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(line.len());
    &line[..cut]
}

fn parse_coordinate(token: &str) -> Result<usize, String> {
    token
        .parse::<usize>()
        .map_err(|_| format!("'{}' is not a non-negative integer", token))
}

fn ordered_span(start: usize, end: usize) -> Result<(usize, usize), String> {
    if start > end {
        return Err(format!("span start {} is greater than span end {}", start, end));
    }
    Ok((start, end))
}

fn parse_defect_line(line: &str) -> Result<Defect, String> {
    let mut parts = line.split_whitespace();
    let raw_keyword = parts.next().unwrap_or_default();
    let coords = parts.map(parse_coordinate).collect::<Result<Vec<usize>, String>>()?;
    match raw_keyword.to_ascii_lowercase().as_str() {
        "point" => match coords.as_slice() {
            [x, y] => Ok(Defect::Point { x: *x, y: *y }),
            _ => Err("expected 'Point x y'".to_string()),
        },
        "col" | "column" => match coords.as_slice() {
            [x] => Ok(Defect::Column { x: *x, y0: None, y1: None }),
            [x, y0, y1] => {
                let (y0, y1) = ordered_span(*y0, *y1)?;
                Ok(Defect::Column { x: *x, y0: Some(y0), y1: Some(y1) })
            }
            _ => Err("expected 'Col x' or 'Col x y0 y1'".to_string()),
        },
        "row" => match coords.as_slice() {
            [y] => Ok(Defect::Row { y: *y, x0: None, x1: None }),
            [y, x0, x1] => {
                let (x0, x1) = ordered_span(*x0, *x1)?;
                Ok(Defect::Row { y: *y, x0: Some(x0), x1: Some(x1) })
            }
            _ => Err("expected 'Row y' or 'Row y x0 x1'".to_string()),
        },
        _ => Err(format!("unknown keyword '{}'; expected Point, Col or Row", raw_keyword)),
    }
}

pub fn parse_defect_list(text: &str) -> Result<Vec<Defect>, String> {
    let mut defects = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let defect = parse_defect_line(line).map_err(|msg| format!("Line {}: {}", index + 1, msg))?;
        defects.push(defect);
    }
    Ok(defects)
}

fn robust_center_and_scale(values: &mut [f32]) -> Option<(f32, f32)> {
    if values.is_empty() {
        return None;
    }
    let median = exact_median_mut(values) as f32;
    let mad = exact_mad_mut(values, median);
    if mad > 0.0 {
        return Some((median, (mad as f64 * MAD_TO_SIGMA) as f32));
    }
    let mean_abs_dev = values.iter().map(|&d| d as f64).sum::<f64>() / values.len() as f64;
    let sigma = (mean_abs_dev * MEAN_ABS_DEV_TO_SIGMA) as f32;
    (sigma > 0.0).then_some((median, sigma))
}

fn standard_slice(image: &Array2<f32>) -> Vec<f32> {
    image.iter().copied().collect()
}

pub fn defect_map_from_dark(
    master_dark: &Array2<f32>,
    hot_sigma: Option<f32>,
    cold_sigma: Option<f32>,
) -> Array2<u8> {
    let mut map = Array2::<u8>::zeros(master_dark.dim());
    if hot_sigma.is_none() && cold_sigma.is_none() {
        return map;
    }
    let mut finite: Vec<f32> = master_dark.iter().copied().filter(|v| v.is_finite()).collect();
    let Some((median, sigma)) = robust_center_and_scale(&mut finite) else {
        return map;
    };
    let hot_limit = hot_sigma.map(|k| median + k * sigma);
    let cold_limit = cold_sigma.map(|k| median - k * sigma);
    Zip::from(&mut map).and(master_dark).par_for_each(|flag, &v| {
        if !v.is_finite() {
            return;
        }
        if hot_limit.is_some_and(|limit| v > limit) {
            *flag |= FLAG_HOT;
        }
        if cold_limit.is_some_and(|limit| v < limit) {
            *flag |= FLAG_COLD;
        }
    });
    map
}

fn offset_index(rows: usize, cols: usize, y: usize, x: usize, dy: isize, dx: isize) -> Option<usize> {
    let ny = y as isize + dy;
    let nx = x as isize + dx;
    if ny < 0 || nx < 0 || ny >= rows as isize || nx >= cols as isize {
        return None;
    }
    Some(ny as usize * cols + nx as usize)
}

#[allow(clippy::too_many_arguments)]
fn gather_ring(
    src: &[f32],
    flags: Option<&[u8]>,
    rows: usize,
    cols: usize,
    y: usize,
    x: usize,
    stride: usize,
    steps: isize,
    buf: &mut [f32],
) -> usize {
    let mut n = 0;
    for ky in -steps..=steps {
        for kx in -steps..=steps {
            if ky == 0 && kx == 0 {
                continue;
            }
            let Some(idx) = offset_index(rows, cols, y, x, ky * stride as isize, kx * stride as isize) else {
                continue;
            };
            if flags.is_some_and(|f| f[idx] != 0) {
                continue;
            }
            let v = src[idx];
            if v.is_finite() {
                buf[n] = v;
                n += 1;
            }
        }
    }
    n
}

fn local_median_residuals(src: &[f32], rows: usize, cols: usize, stride: usize) -> Vec<f32> {
    let mut residual = vec![f32::NAN; rows * cols];
    residual.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        let mut buf = [0f32; 8];
        for (x, out) in row.iter_mut().enumerate() {
            let v = src[y * cols + x];
            if !v.is_finite() {
                continue;
            }
            let n = gather_ring(src, None, rows, cols, y, x, stride, INNER_RING_STEPS, &mut buf);
            if n < MIN_NEIGHBOURS_FOR_MEDIAN {
                continue;
            }
            *out = v - median_f32_mut(&mut buf[..n]);
        }
    });
    residual
}

fn auto_candidates(
    image: &Array2<f32>,
    hot_sigma: Option<f32>,
    cold_sigma: Option<f32>,
    stride: usize,
) -> Option<Vec<i8>> {
    let (rows, cols) = image.dim();
    let src = standard_slice(image);
    let residual = local_median_residuals(&src, rows, cols, stride);
    let mut finite: Vec<f32> = residual.iter().copied().filter(|v| v.is_finite()).collect();
    let (center, scale) = robust_center_and_scale(&mut finite)?;
    let hot_limit = hot_sigma.map(|k| center + k * scale);
    let cold_limit = cold_sigma.map(|k| center - k * scale);
    Some(
        residual
            .par_iter()
            .map(|&r| {
                if !r.is_finite() {
                    0
                } else if hot_limit.is_some_and(|limit| r > limit) {
                    1
                } else if cold_limit.is_some_and(|limit| r < limit) {
                    -1
                } else {
                    0
                }
            })
            .collect(),
    )
}

fn flag_small_clusters(
    mut candidates: Vec<i8>,
    rows: usize,
    cols: usize,
    stride: usize,
) -> Vec<u8> {
    let mut flags = vec![0u8; candidates.len()];
    let mut component: Vec<usize> = Vec::new();
    let mut frontier: Vec<usize> = Vec::new();
    for start in 0..candidates.len() {
        let sign = candidates[start];
        if sign == 0 {
            continue;
        }
        candidates[start] = 0;
        component.clear();
        frontier.push(start);
        while let Some(idx) = frontier.pop() {
            component.push(idx);
            let (y, x) = (idx / cols, idx % cols);
            for &(dy, dx) in &NEIGHBOUR_OFFSETS {
                let step = (dy * stride as isize, dx * stride as isize);
                let Some(next) = offset_index(rows, cols, y, x, step.0, step.1) else {
                    continue;
                };
                if candidates[next] != sign {
                    continue;
                }
                candidates[next] = 0;
                frontier.push(next);
            }
        }
        if component.len() > MAX_DEFECT_CLUSTER_PIXELS {
            continue;
        }
        let flag = if sign > 0 { FLAG_HOT } else { FLAG_COLD };
        for &idx in &component {
            flags[idx] = flag;
        }
    }
    flags
}

pub fn defect_map_auto(
    image: &Array2<f32>,
    hot_sigma: Option<f32>,
    cold_sigma: Option<f32>,
    cfa: bool,
) -> Array2<u8> {
    let (rows, cols) = image.dim();
    let map = Array2::<u8>::zeros((rows, cols));
    if (hot_sigma.is_none() && cold_sigma.is_none()) || rows == 0 || cols == 0 {
        return map;
    }
    let stride = if cfa { 2 } else { 1 };
    let Some(candidates) = auto_candidates(image, hot_sigma, cold_sigma, stride) else {
        return map;
    };
    let flags = flag_small_clusters(candidates, rows, cols, stride);
    Array2::from_shape_vec((rows, cols), flags).expect("flag buffer matches image shape")
}

fn out_of_range(defect: &Defect, rows: usize, cols: usize) -> anyhow::Error {
    let description = match defect {
        Defect::Point { x, y } => format!("Point x={} y={}", x, y),
        Defect::Column { x, y0, y1 } => format!("Col x={} y0={:?} y1={:?}", x, y0, y1),
        Defect::Row { y, x0, x1 } => format!("Row y={} x0={:?} x1={:?}", y, x0, x1),
    };
    anyhow!(
        "Defect {} lies outside the {}x{} image (0-based coordinates)",
        description,
        cols,
        rows
    )
}

fn resolve_span(start: Option<usize>, end: Option<usize>, len: usize) -> Option<(usize, usize)> {
    let a = start.unwrap_or(0);
    let b = end.unwrap_or(len.checked_sub(1)?);
    (a <= b && b < len).then_some((a, b))
}

pub fn defect_map_from_list(defects: &[Defect], rows: usize, cols: usize) -> Result<Array2<u8>> {
    let mut map = Array2::<u8>::zeros((rows, cols));
    for defect in defects {
        match *defect {
            Defect::Point { x, y } => {
                if x >= cols || y >= rows {
                    return Err(out_of_range(defect, rows, cols));
                }
                map[[y, x]] |= FLAG_LISTED;
            }
            Defect::Column { x, y0, y1 } => {
                let Some((a, b)) = resolve_span(y0, y1, rows).filter(|_| x < cols) else {
                    return Err(out_of_range(defect, rows, cols));
                };
                for y in a..=b {
                    map[[y, x]] |= FLAG_LISTED;
                }
            }
            Defect::Row { y, x0, x1 } => {
                let Some((a, b)) = resolve_span(x0, x1, cols).filter(|_| y < rows) else {
                    return Err(out_of_range(defect, rows, cols));
                };
                for x in a..=b {
                    map[[y, x]] |= FLAG_LISTED;
                }
            }
        }
    }
    Ok(map)
}

pub fn merge_maps(maps: &[&Array2<u8>]) -> Array2<u8> {
    let Some((first, rest)) = maps.split_first() else {
        return Array2::zeros((0, 0));
    };
    let mut merged = (*first).clone();
    for map in rest {
        Zip::from(&mut merged).and(*map).for_each(|out, &v| *out |= v);
    }
    merged
}

#[allow(clippy::too_many_arguments)]
fn neighbour_estimate(
    src: &[f32],
    flags: &[u8],
    rows: usize,
    cols: usize,
    y: usize,
    x: usize,
    stride: usize,
    replacement: Replacement,
) -> Option<f32> {
    let mut buf = [0f32; OUTER_RING_CAPACITY];
    let mut n = gather_ring(src, Some(flags), rows, cols, y, x, stride, INNER_RING_STEPS, &mut buf);
    if n == 0 {
        n = gather_ring(src, Some(flags), rows, cols, y, x, stride, OUTER_RING_STEPS, &mut buf);
    }
    if n == 0 {
        return None;
    }
    let values = &mut buf[..n];
    Some(match replacement {
        Replacement::Median => median_f32_mut(values),
        Replacement::Mean => (values.iter().map(|&v| v as f64).sum::<f64>() / n as f64) as f32,
    })
}

pub fn apply_cosmetic(
    image: &Array2<f32>,
    map: &Array2<u8>,
    replacement: Replacement,
    amount: f32,
    cfa: bool,
) -> (Array2<f32>, usize) {
    let (rows, cols) = image.dim();
    let amount = amount.clamp(0.0, 1.0);
    if map.dim() != image.dim() || rows == 0 || cols == 0 || amount <= 0.0 {
        return (image.clone(), 0);
    }
    let stride = if cfa { 2 } else { 1 };
    let src = standard_slice(image);
    let flags: Vec<u8> = map.iter().copied().collect();
    let mut out = src.clone();
    let replaced = out
        .par_chunks_mut(cols)
        .enumerate()
        .map(|(y, row)| {
            let mut count = 0usize;
            for (x, out) in row.iter_mut().enumerate() {
                let idx = y * cols + x;
                if flags[idx] == 0 {
                    continue;
                }
                let Some(estimate) =
                    neighbour_estimate(&src, &flags, rows, cols, y, x, stride, replacement)
                else {
                    continue;
                };
                let v = src[idx];
                *out = if v.is_finite() { v * (1.0 - amount) + estimate * amount } else { estimate };
                count += 1;
            }
            count
        })
        .sum();
    let corrected = Array2::from_shape_vec((rows, cols), out).expect("output buffer matches image shape");
    (corrected, replaced)
}

fn validate_sigma(value: Option<f32>, name: &str) -> Result<()> {
    if let Some(k) = value {
        if !k.is_finite() || k <= 0.0 {
            bail!("{} must be a positive number, got {}", name, k);
        }
    }
    Ok(())
}

impl CosmeticConfig {
    pub fn validate(&self) -> Result<()> {
        validate_sigma(self.dark_hot_sigma, "Master dark hot sigma")?;
        validate_sigma(self.dark_cold_sigma, "Master dark cold sigma")?;
        validate_sigma(self.auto_hot_sigma, "Auto-detect hot sigma")?;
        validate_sigma(self.auto_cold_sigma, "Auto-detect cold sigma")?;
        if !(0.0..=1.0).contains(&self.amount) {
            bail!("Amount must be between 0 and 1, got {}", self.amount);
        }
        Ok(())
    }
}

fn count_flags(map: &Array2<u8>) -> (usize, usize, usize, usize) {
    map.iter().fold((0, 0, 0, 0), |(flagged, hot, cold, listed), &v| {
        (
            flagged + usize::from(v != 0),
            hot + usize::from(v & FLAG_HOT != 0),
            cold + usize::from(v & FLAG_COLD != 0),
            listed + usize::from(v & FLAG_LISTED != 0),
        )
    })
}

pub fn cosmetic_correct(
    image: &Array2<f32>,
    master_dark: Option<&Array2<f32>>,
    cfg: &CosmeticConfig,
) -> Result<CosmeticResult> {
    let (rows, cols) = image.dim();
    if rows == 0 || cols == 0 {
        bail!("Image is empty");
    }
    cfg.validate()?;

    let mut maps: Vec<Array2<u8>> = Vec::new();
    if cfg.use_master_dark {
        let dark = master_dark
            .ok_or_else(|| anyhow!("Master dark detection is enabled but no master dark was provided"))?;
        if dark.dim() != image.dim() {
            bail!(
                "Master dark is {}x{} but the image is {}x{}",
                dark.ncols(),
                dark.nrows(),
                cols,
                rows
            );
        }
        maps.push(defect_map_from_dark(dark, cfg.dark_hot_sigma, cfg.dark_cold_sigma));
    }
    if cfg.auto_hot_sigma.is_some() || cfg.auto_cold_sigma.is_some() {
        maps.push(defect_map_auto(image, cfg.auto_hot_sigma, cfg.auto_cold_sigma, cfg.cfa));
    }
    if !cfg.defects.is_empty() {
        maps.push(defect_map_from_list(&cfg.defects, rows, cols)?);
    }
    if maps.is_empty() {
        bail!("No detection method is enabled: use a master dark, auto-detection or a defect list");
    }

    let refs: Vec<&Array2<u8>> = maps.iter().collect();
    let merged = merge_maps(&refs);
    let (flagged, hot, cold, listed) = count_flags(&merged);
    let (corrected, replaced) = apply_cosmetic(image, &merged, cfg.replacement, cfg.amount, cfg.cfa);
    Ok(CosmeticResult { corrected, flagged, replaced, hot, cold, listed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(y: usize, x: usize) -> f32 {
        let h = (y as u32).wrapping_mul(73856093) ^ (x as u32).wrapping_mul(19349663);
        ((h & 0xffff) as f32 / 65535.0) * 2.0 - 1.0
    }

    fn noisy_flat(rows: usize, cols: usize, level: f32) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| level + noise(y, x))
    }

    fn ramp(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| (y * cols + x) as f32)
    }

    fn flagged_positions(map: &Array2<u8>) -> Vec<(usize, usize)> {
        map.indexed_iter().filter(|(_, &v)| v != 0).map(|(p, _)| p).collect()
    }

    fn star_pixels(cy: usize, cx: usize, step: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::with_capacity(25);
        for dy in -2i64..=2 {
            for dx in -2i64..=2 {
                let y = (cy as i64 + dy * step as i64) as usize;
                let x = (cx as i64 + dx * step as i64) as usize;
                out.push((y, x));
            }
        }
        out
    }

    fn add_gaussian_star(img: &mut Array2<f32>, cy: usize, cx: usize, amplitude: f32, step: usize) {
        for dy in -2i64..=2 {
            for dx in -2i64..=2 {
                let r2 = (dy * dy + dx * dx) as f32;
                let y = (cy as i64 + dy * step as i64) as usize;
                let x = (cx as i64 + dx * step as i64) as usize;
                img[[y, x]] += amplitude * (-0.5 * r2).exp();
            }
        }
    }

    fn candidate_positions(candidates: &[i8], cols: usize, sign: i8) -> Vec<(usize, usize)> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, &c)| c == sign)
            .map(|(idx, _)| (idx / cols, idx % cols))
            .collect()
    }

    #[test]
    fn parse_defect_list_accepts_all_three_forms() {
        let text = "# hot pixels\nPoint 10 20\ncol 5\nCOL 7 2 9\nRow 3\n\nrow 4 1 6 // trailing note\n// end\n";
        let parsed = parse_defect_list(text).unwrap();
        assert_eq!(
            parsed,
            vec![
                Defect::Point { x: 10, y: 20 },
                Defect::Column { x: 5, y0: None, y1: None },
                Defect::Column { x: 7, y0: Some(2), y1: Some(9) },
                Defect::Row { y: 3, x0: None, x1: None },
                Defect::Row { y: 4, x0: Some(1), x1: Some(6) },
            ]
        );
        assert!(parse_defect_list("").unwrap().is_empty());
        assert!(parse_defect_list("column 3 0 0").is_ok());
    }

    #[test]
    fn parse_defect_list_reports_the_line_number() {
        let err = parse_defect_list("Point 1 2\n\nCol x\n").unwrap_err();
        assert!(err.starts_with("Line 3:"), "{}", err);
        let err = parse_defect_list("Point 1 2\nFoo 1 2").unwrap_err();
        assert!(err.starts_with("Line 2:"), "{}", err);
        assert!(err.contains("Foo"), "{}", err);
        let err = parse_defect_list("Point 1").unwrap_err();
        assert!(err.starts_with("Line 1:"), "{}", err);
        let err = parse_defect_list("Row 1 5 2").unwrap_err();
        assert!(err.starts_with("Line 1:"), "{}", err);
        let err = parse_defect_list("Point -1 0").unwrap_err();
        assert!(err.starts_with("Line 1:"), "{}", err);
    }

    #[test]
    fn defect_map_from_list_uses_zero_based_coordinates() {
        let parsed = parse_defect_list("Point 0 0\nPoint 4 3").unwrap();
        let map = defect_map_from_list(&parsed, 4, 5).unwrap();
        assert_eq!(flagged_positions(&map), vec![(0, 0), (3, 4)]);
        assert_eq!(map[[0, 0]], FLAG_LISTED);

        let map = defect_map_from_list(&[Defect::Column { x: 1, y0: Some(1), y1: Some(2) }], 4, 5).unwrap();
        assert_eq!(flagged_positions(&map), vec![(1, 1), (2, 1)]);

        let map = defect_map_from_list(&[Defect::Row { y: 3, x0: None, x1: None }], 4, 5).unwrap();
        assert_eq!(flagged_positions(&map), vec![(3, 0), (3, 1), (3, 2), (3, 3), (3, 4)]);

        let map = defect_map_from_list(&[Defect::Column { x: 4, y0: None, y1: None }], 4, 5).unwrap();
        assert_eq!(flagged_positions(&map).len(), 4);

        assert!(defect_map_from_list(&[Defect::Point { x: 5, y: 0 }], 4, 5).is_err());
        assert!(defect_map_from_list(&[Defect::Point { x: 0, y: 4 }], 4, 5).is_err());
        assert!(defect_map_from_list(&[Defect::Column { x: 0, y0: Some(0), y1: Some(4) }], 4, 5).is_err());
        assert!(defect_map_from_list(&[Defect::Row { y: 0, x0: Some(5), x1: Some(5) }], 4, 5).is_err());
        assert!(defect_map_from_list(&[Defect::Point { x: 0, y: 0 }], 0, 0).is_err());
    }

    #[test]
    fn from_dark_flags_hot_and_cold_at_five_sigma() {
        let mut dark = noisy_flat(64, 64, 100.0);
        let hot = [(3, 7), (40, 41), (63, 0)];
        let cold = [(10, 10), (50, 20)];
        for &(y, x) in &hot {
            dark[[y, x]] = 5000.0;
        }
        for &(y, x) in &cold {
            dark[[y, x]] = 0.0;
        }
        dark[[20, 20]] = f32::NAN;

        let map = defect_map_from_dark(&dark, Some(5.0), Some(5.0));
        let mut expected: Vec<(usize, usize)> = hot.iter().chain(cold.iter()).copied().collect();
        expected.sort();
        assert_eq!(flagged_positions(&map), expected);
        for &(y, x) in &hot {
            assert_eq!(map[[y, x]], FLAG_HOT);
        }
        for &(y, x) in &cold {
            assert_eq!(map[[y, x]], FLAG_COLD);
        }

        let hot_only = defect_map_from_dark(&dark, Some(5.0), None);
        let mut hot_sorted = hot.to_vec();
        hot_sorted.sort();
        assert_eq!(flagged_positions(&hot_only), hot_sorted);

        let none = defect_map_from_dark(&dark, None, None);
        assert!(flagged_positions(&none).is_empty());
    }

    #[test]
    fn auto_flags_isolated_hot_pixel_and_keeps_star() {
        let mut img = noisy_flat(64, 64, 100.0);
        img[[10, 10]] = 1100.0;
        img[[20, 30]] = 0.0;
        let (cy, cx) = (40usize, 40usize);
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                let r2 = (dy * dy + dx * dx) as f32;
                let y = (cy as i32 + dy) as usize;
                let x = (cx as i32 + dx) as usize;
                img[[y, x]] += 50.0 * (-0.5 * r2).exp();
            }
        }

        let map = defect_map_auto(&img, Some(3.0), Some(3.0), false);
        assert_eq!(flagged_positions(&map), vec![(10, 10), (20, 30)]);
        assert_eq!(map[[10, 10]], FLAG_HOT);
        assert_eq!(map[[20, 30]], FLAG_COLD);

        let hot_only = defect_map_auto(&img, Some(3.0), None, false);
        assert_eq!(flagged_positions(&hot_only), vec![(10, 10)]);

        assert!(flagged_positions(&defect_map_auto(&img, None, None, false)).is_empty());
    }

    #[test]
    fn auto_cfa_stride_ignores_the_other_colours() {
        let mut img = Array2::from_shape_fn((16, 16), |(y, x)| if y % 2 == 0 && x % 2 == 0 { 200.0 } else { 10.0 });
        img[[8, 9]] = 1000.0;

        let cfa = defect_map_auto(&img, Some(3.0), None, true);
        assert_eq!(flagged_positions(&cfa), vec![(8, 9)]);
        assert_eq!(cfa[[8, 9]], FLAG_HOT);

        let mono = defect_map_auto(&img, Some(2.0), None, false);
        assert!(flagged_positions(&mono).len() > 1);
    }

    #[test]
    fn auto_flags_an_adjacent_hot_pair_and_a_lone_hot_pixel_but_not_a_four_pixel_block() {
        let mut img = noisy_flat(64, 64, 100.0);
        img[[10, 10]] = 1100.0;
        img[[10, 11]] = 1100.0;
        img[[30, 40]] = 1100.0;
        for y in 50..52 {
            for x in 50..52 {
                img[[y, x]] = 1100.0;
            }
        }
        let candidates = auto_candidates(&img, Some(3.0), None, 1).unwrap();
        let mut expected_candidates = vec![(10, 10), (10, 11), (30, 40), (50, 50), (50, 51), (51, 50), (51, 51)];
        expected_candidates.sort();
        assert_eq!(candidate_positions(&candidates, 64, 1), expected_candidates);

        let map = defect_map_auto(&img, Some(3.0), None, false);
        assert_eq!(flagged_positions(&map), vec![(10, 10), (10, 11), (30, 40)]);
        assert_eq!(map[[10, 10]], FLAG_HOT);
        assert_eq!(map[[10, 11]], FLAG_HOT);
    }

    #[test]
    fn auto_drops_a_five_by_five_star_whose_pixels_all_exceed_the_threshold() {
        let mut img = noisy_flat(64, 64, 100.0);
        add_gaussian_star(&mut img, 40, 40, 1000.0, 1);
        img[[10, 10]] = 1100.0;

        let candidates = auto_candidates(&img, Some(3.0), Some(3.0), 1).unwrap();
        let mut expected_candidates = star_pixels(40, 40, 1);
        expected_candidates.push((10, 10));
        expected_candidates.sort();
        assert_eq!(candidate_positions(&candidates, 64, 1), expected_candidates);
        assert!(candidate_positions(&candidates, 64, -1).is_empty());

        let map = defect_map_auto(&img, Some(3.0), Some(3.0), false);
        assert_eq!(flagged_positions(&map), vec![(10, 10)]);
    }

    #[test]
    fn auto_cfa_clusters_candidates_on_the_stride_two_lattice() {
        let mut img = Array2::from_shape_fn((64, 64), |(y, x)| if y % 2 == 0 && x % 2 == 0 { 200.0 } else { 10.0 });
        img[[8, 9]] = 1000.0;
        img[[8, 11]] = 1000.0;
        img[[8, 10]] = 1000.0;
        add_gaussian_star(&mut img, 20, 17, 1000.0, 2);

        let candidates = auto_candidates(&img, Some(3.0), None, 2).unwrap();
        let mut expected_candidates = star_pixels(20, 17, 2);
        expected_candidates.extend([(8, 9), (8, 10), (8, 11)]);
        expected_candidates.sort();
        assert_eq!(candidate_positions(&candidates, 64, 1), expected_candidates);

        let cfa = defect_map_auto(&img, Some(3.0), None, true);
        assert_eq!(flagged_positions(&cfa), vec![(8, 9), (8, 10), (8, 11)]);
    }

    #[test]
    fn from_dark_falls_back_to_the_mean_absolute_deviation_when_the_mad_is_zero() {
        let mut dark = Array2::from_shape_fn((64, 64), |(y, x)| if (y * 7 + x) % 3 == 0 { 101.0 } else { 100.0 });
        let hot = [(3, 7), (40, 41), (63, 0)];
        for &(y, x) in &hot {
            dark[[y, x]] = 5000.0;
        }
        let map = defect_map_from_dark(&dark, Some(3.0), Some(3.0));
        let mut expected = hot.to_vec();
        expected.sort();
        assert_eq!(flagged_positions(&map), expected);

        let mut constant = Array2::from_elem((64, 64), 100.0f32);
        for &(y, x) in &hot {
            constant[[y, x]] = 5000.0;
        }
        assert_eq!(flagged_positions(&defect_map_from_dark(&constant, Some(3.0), Some(3.0))), expected);
    }

    #[test]
    fn from_dark_flags_nothing_on_a_perfectly_constant_dark() {
        let dark = Array2::from_elem((32, 32), 100.0f32);
        assert!(flagged_positions(&defect_map_from_dark(&dark, Some(3.0), Some(3.0))).is_empty());
        assert!(flagged_positions(&defect_map_auto(&dark, Some(3.0), Some(3.0), false)).is_empty());
    }

    #[test]
    fn auto_falls_back_to_the_mean_absolute_deviation_when_the_mad_is_zero() {
        let mut img = Array2::from_shape_fn((32, 32), |(y, x)| if y % 3 == 0 && x % 3 == 0 { 101.0 } else { 100.0 });
        img[[16, 16]] = 5000.0;
        let map = defect_map_auto(&img, Some(3.0), Some(3.0), false);
        assert_eq!(flagged_positions(&map), vec![(16, 16)]);
        assert_eq!(map[[16, 16]], FLAG_HOT);
    }

    #[test]
    fn apply_replaces_with_median_of_unflagged_neighbours() {
        let img = ramp(5, 5);
        let mut map = Array2::<u8>::zeros((5, 5));
        map[[2, 2]] = FLAG_HOT;
        map[[1, 1]] = FLAG_LISTED;
        let (out, replaced) = apply_cosmetic(&img, &map, Replacement::Median, 1.0, false);
        assert_eq!(replaced, 2);
        assert_eq!(out[[2, 2]], 13.0);
        assert_eq!(out[[1, 1]], 5.0);
        for (pos, &v) in img.indexed_iter() {
            if pos != (2, 2) && pos != (1, 1) {
                assert_eq!(out[pos], v);
            }
        }
    }

    #[test]
    fn apply_amount_half_blends_and_mean_averages() {
        let mut img = ramp(5, 5);
        img[[2, 2]] = 100.0;
        img[[1, 1]] = 30.0;
        let mut map = Array2::<u8>::zeros((5, 5));
        map[[2, 2]] = FLAG_HOT;

        let (median_full, _) = apply_cosmetic(&img, &map, Replacement::Median, 1.0, false);
        assert_eq!(median_full[[2, 2]], 14.5);

        let (median_half, replaced) = apply_cosmetic(&img, &map, Replacement::Median, 0.5, false);
        assert_eq!(replaced, 1);
        assert!((median_half[[2, 2]] - 57.25).abs() < 1e-5);

        let (mean_full, _) = apply_cosmetic(&img, &map, Replacement::Mean, 1.0, false);
        assert!((mean_full[[2, 2]] - 15.0).abs() < 1e-5);

        let (untouched, replaced) = apply_cosmetic(&img, &map, Replacement::Median, 0.0, false);
        assert_eq!(replaced, 0);
        assert_eq!(untouched[[2, 2]], 100.0);
    }

    #[test]
    fn apply_cfa_stride_keeps_same_colour_neighbours() {
        let mut img = Array2::from_shape_fn((7, 7), |(y, x)| if (y + x) % 2 == 0 { 10.0 } else { 20.0 });
        img[[3, 3]] = 500.0;
        let mut map = Array2::<u8>::zeros((7, 7));
        map[[3, 3]] = FLAG_HOT;

        let (cfa, _) = apply_cosmetic(&img, &map, Replacement::Median, 1.0, true);
        assert_eq!(cfa[[3, 3]], 10.0);

        let (mono, _) = apply_cosmetic(&img, &map, Replacement::Median, 1.0, false);
        assert_eq!(mono[[3, 3]], 15.0);
    }

    #[test]
    fn apply_expands_to_the_outer_ring_and_leaves_isolated_pixels() {
        let mut img = Array2::from_elem((5, 5), 7.0f32);
        let mut map = Array2::<u8>::zeros((5, 5));
        for y in 1..4 {
            for x in 1..4 {
                img[[y, x]] = 100.0;
                map[[y, x]] = FLAG_HOT;
            }
        }
        let (out, replaced) = apply_cosmetic(&img, &map, Replacement::Median, 1.0, false);
        assert_eq!(replaced, 9);
        assert_eq!(out[[2, 2]], 7.0);
        assert_eq!(out[[1, 1]], 7.0);

        let all = Array2::from_elem((3, 3), FLAG_HOT);
        let (same, replaced) = apply_cosmetic(&ramp(3, 3), &all, Replacement::Median, 1.0, false);
        assert_eq!(replaced, 0);
        assert_eq!(same, ramp(3, 3));
    }

    #[test]
    fn apply_keeps_unflagged_nan_and_fills_flagged_nan() {
        let mut img = ramp(5, 5);
        img[[0, 0]] = f32::NAN;
        img[[2, 2]] = f32::NAN;
        img[[2, 3]] = f32::NAN;
        let mut map = Array2::<u8>::zeros((5, 5));
        map[[2, 2]] = FLAG_HOT;
        let (out, replaced) = apply_cosmetic(&img, &map, Replacement::Median, 0.5, false);
        assert_eq!(replaced, 1);
        assert!(out[[0, 0]].is_nan());
        assert!(out[[2, 3]].is_nan());
        assert_eq!(out[[2, 2]], 11.0);
    }

    #[test]
    fn merge_maps_ors_flags() {
        let mut a = Array2::<u8>::zeros((2, 2));
        let mut b = Array2::<u8>::zeros((2, 2));
        a[[0, 0]] = FLAG_HOT;
        b[[0, 0]] = FLAG_LISTED;
        b[[1, 1]] = FLAG_COLD;
        let merged = merge_maps(&[&a, &b]);
        assert_eq!(merged[[0, 0]], FLAG_HOT | FLAG_LISTED);
        assert_eq!(merged[[1, 1]], FLAG_COLD);
        assert_eq!(merged[[0, 1]], 0);
        assert_eq!(merge_maps(&[]).dim(), (0, 0));
    }

    #[test]
    fn cosmetic_correct_counts_every_source() {
        let mut img = Array2::from_elem((8, 8), 100.0f32);
        img[[5, 5]] = 5000.0;
        let mut dark = Array2::from_elem((8, 8), 10.0f32);
        dark[[1, 1]] = 1000.0;
        dark[[6, 2]] = -100.0;
        let cfg = CosmeticConfig {
            use_master_dark: true,
            dark_hot_sigma: Some(3.0),
            dark_cold_sigma: Some(3.0),
            auto_hot_sigma: Some(3.0),
            auto_cold_sigma: None,
            defects: vec![Defect::Point { x: 7, y: 0 }],
            ..Default::default()
        };
        let res = cosmetic_correct(&img, Some(&dark), &cfg).unwrap();
        assert_eq!(res.flagged, 4);
        assert_eq!(res.hot, 2);
        assert_eq!(res.cold, 1);
        assert_eq!(res.listed, 1);
        assert_eq!(res.replaced, 4);
        assert_eq!(res.corrected[[5, 5]], 100.0);
        assert_eq!(res.corrected[[1, 1]], 100.0);
        assert_eq!(res.corrected.dim(), (8, 8));
    }

    #[test]
    fn cosmetic_correct_validates_inputs() {
        let img = Array2::from_elem((4, 4), 1.0f32);
        let dark = Array2::from_elem((4, 5), 1.0f32);
        let cfg = CosmeticConfig::default();
        let err = cosmetic_correct(&img, Some(&dark), &cfg).unwrap_err().to_string();
        assert!(err.contains("Master dark"), "{}", err);

        let err = cosmetic_correct(&img, None, &cfg).unwrap_err().to_string();
        assert!(err.contains("master dark"), "{}", err);

        let nothing = CosmeticConfig { use_master_dark: false, ..Default::default() };
        assert!(cosmetic_correct(&img, None, &nothing).is_err());

        let bad_amount = CosmeticConfig { use_master_dark: false, auto_hot_sigma: Some(3.0), amount: 1.5, ..Default::default() };
        assert!(cosmetic_correct(&img, None, &bad_amount).is_err());

        let bad_sigma = CosmeticConfig { use_master_dark: false, auto_hot_sigma: Some(0.0), ..Default::default() };
        assert!(cosmetic_correct(&img, None, &bad_sigma).is_err());

        let listed = CosmeticConfig {
            use_master_dark: false,
            defects: vec![Defect::Point { x: 9, y: 0 }],
            ..Default::default()
        };
        assert!(cosmetic_correct(&img, None, &listed).is_err());
    }

    #[test]
    fn config_deserializes_from_partial_json() {
        let cfg: CosmeticConfig = serde_json::from_str(
            r#"{"use_master_dark":false,"auto_hot_sigma":4.5,"defects":[{"kind":"point","x":1,"y":2},{"kind":"column","x":3,"y0":null,"y1":null},{"kind":"row","y":4,"x0":0,"x1":9}],"replacement":"mean"}"#,
        )
        .unwrap();
        assert!(!cfg.use_master_dark);
        assert_eq!(cfg.auto_hot_sigma, Some(4.5));
        assert_eq!(cfg.dark_hot_sigma, Some(3.0));
        assert_eq!(cfg.amount, 1.0);
        assert_eq!(cfg.replacement, Replacement::Mean);
        assert_eq!(cfg.defects.len(), 3);
        assert_eq!(cfg.defects[2], Defect::Row { y: 4, x0: Some(0), x1: Some(9) });
    }
}
