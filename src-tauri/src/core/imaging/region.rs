use ndarray::Array2;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::astrometry::frames::{fk5_j2000_to_icrs, icrs_to_fk5_j2000};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::stats::finite_slice_stats;
use crate::math::{exact_mad_mut, exact_median_mut, sigma_clipped_stats};
use crate::types::constants::MAD_TO_SIGMA;

pub const DS9_PIXEL_OFFSET: f64 = 1.0;
pub const PAR_BBOX_PIXELS: usize = 262_144;
pub const MAX_RADIAL_RADIUS: f64 = 4096.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum RegionShape {
    Circle {
        x: f64,
        y: f64,
        r: f64,
    },
    Ellipse {
        x: f64,
        y: f64,
        rx: f64,
        ry: f64,
        #[serde(default)]
        angle: f64,
    },
    Box {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        #[serde(default)]
        angle: f64,
    },
    Annulus {
        x: f64,
        y: f64,
        r_inner: f64,
        r_outer: f64,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    Point {
        x: f64,
        y: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PixelBounds {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RegionError {
    #[error("invalid region: {0}")]
    Invalid(String),
    #[error("{shape} region contains no finite pixels")]
    Empty { shape: &'static str },
    #[error("sky region requires a WCS on the image, but none is present")]
    WcsRequired,
    #[error("region does not project onto the image")]
    OffImage,
    #[error("unsupported coordinate system '{0}'")]
    UnsupportedSystem(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaskedValues {
    pub values: Vec<f32>,
    pub errors: Vec<f32>,
    pub n_inside: u64,
    pub n_nan: u64,
    pub n_excluded: u64,
    pub clipped: bool,
}

fn snap_unit(v: f64) -> f64 {
    if v.abs() < 1e-12 {
        0.0
    } else if (v.abs() - 1.0).abs() < 1e-12 {
        v.signum()
    } else {
        v
    }
}

fn sin_cos_deg(angle_deg: f64) -> (f64, f64) {
    let (s, c) = angle_deg.to_radians().sin_cos();
    (snap_unit(s), snap_unit(c))
}

fn rot_local_to_index(u: f64, v: f64, angle_deg: f64) -> (f64, f64) {
    let (s, c) = sin_cos_deg(angle_deg);
    (u * c - v * s, u * s + v * c)
}

fn rot_index_to_local(dx: f64, dy: f64, angle_deg: f64) -> (f64, f64) {
    let (s, c) = sin_cos_deg(angle_deg);
    (dx * c + dy * s, -dx * s + dy * c)
}

fn extent_bounds(xmin: f64, ymin: f64, xmax: f64, ymax: f64) -> PixelBounds {
    PixelBounds {
        x0: xmin.floor() as i64,
        y0: ymin.floor() as i64,
        x1: xmax.ceil() as i64,
        y1: ymax.ceil() as i64,
    }
}

fn point_extent(points: impl Iterator<Item = (f64, f64)>) -> (f64, f64, f64, f64) {
    let mut xmin = f64::INFINITY;
    let mut ymin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for (x, y) in points {
        xmin = xmin.min(x);
        ymin = ymin.min(y);
        xmax = xmax.max(x);
        ymax = ymax.max(y);
    }
    (xmin, ymin, xmax, ymax)
}

fn require_finite(name: &str, v: f64) -> Result<(), RegionError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(RegionError::Invalid(format!("{name} must be finite")))
    }
}

fn require_positive(name: &str, v: f64) -> Result<(), RegionError> {
    require_finite(name, v)?;
    if v > 0.0 {
        Ok(())
    } else {
        Err(RegionError::Invalid(format!("{name} must be > 0, got {v}")))
    }
}

fn nearest_pixel(x: f64, y: f64) -> (i64, i64) {
    (x.round() as i64, y.round() as i64)
}

struct RowScan {
    values: Vec<f32>,
    errors: Vec<f32>,
    n_inside: u64,
    n_nan: u64,
    n_excluded: u64,
}

impl RowScan {
    fn empty() -> Self {
        Self { values: Vec::new(), errors: Vec::new(), n_inside: 0, n_nan: 0, n_excluded: 0 }
    }

    fn take(&mut self, v: f32, err: Option<f32>, excluded: bool) {
        self.n_inside += 1;
        if excluded {
            self.n_excluded += 1;
        } else if !v.is_finite() {
            self.n_nan += 1;
        } else {
            self.values.push(v);
            if let Some(e) = err {
                self.errors.push(e);
            }
        }
    }

    fn merge(mut self, other: RowScan) -> Self {
        self.values.extend(other.values);
        self.errors.extend(other.errors);
        self.n_inside += other.n_inside;
        self.n_nan += other.n_nan;
        self.n_excluded += other.n_excluded;
        self
    }
}

fn sample_pixels(
    arr: &Array2<f32>,
    err: Option<&Array2<f32>>,
    excluded: Option<&Array2<u8>>,
    pixels: impl Iterator<Item = (i64, i64)>,
) -> RowScan {
    let (rows, cols) = arr.dim();
    let mut scan = RowScan::empty();
    for (px, py) in pixels {
        if px < 0 || py < 0 || px >= cols as i64 || py >= rows as i64 {
            continue;
        }
        let (ux, uy) = (px as usize, py as usize);
        let ex = excluded.is_some_and(|m| m[[uy, ux]] != 0);
        scan.take(arr[[uy, ux]], err.map(|e| e[[uy, ux]]), ex);
    }
    scan
}

impl RegionShape {
    pub fn kind(&self) -> &'static str {
        match self {
            RegionShape::Circle { .. } => "circle",
            RegionShape::Ellipse { .. } => "ellipse",
            RegionShape::Box { .. } => "box",
            RegionShape::Annulus { .. } => "annulus",
            RegionShape::Polygon { .. } => "polygon",
            RegionShape::Line { .. } => "line",
            RegionShape::Point { .. } => "point",
        }
    }

    pub fn validate(&self) -> Result<(), RegionError> {
        match self {
            RegionShape::Circle { x, y, r } => {
                require_finite("x", *x)?;
                require_finite("y", *y)?;
                require_positive("r", *r)
            }
            RegionShape::Ellipse { x, y, rx, ry, angle } => {
                require_finite("x", *x)?;
                require_finite("y", *y)?;
                require_positive("rx", *rx)?;
                require_positive("ry", *ry)?;
                require_finite("angle", *angle)
            }
            RegionShape::Box { x, y, width, height, angle } => {
                require_finite("x", *x)?;
                require_finite("y", *y)?;
                require_positive("width", *width)?;
                require_positive("height", *height)?;
                require_finite("angle", *angle)
            }
            RegionShape::Annulus { x, y, r_inner, r_outer } => {
                require_finite("x", *x)?;
                require_finite("y", *y)?;
                require_finite("r_inner", *r_inner)?;
                require_positive("r_outer", *r_outer)?;
                if *r_inner < 0.0 {
                    return Err(RegionError::Invalid(format!("r_inner must be >= 0, got {r_inner}")));
                }
                if r_inner >= r_outer {
                    return Err(RegionError::Invalid(format!(
                        "r_inner ({r_inner}) must be smaller than r_outer ({r_outer})"
                    )));
                }
                Ok(())
            }
            RegionShape::Polygon { points } => {
                if points.len() < 3 {
                    return Err(RegionError::Invalid(format!(
                        "polygon needs at least 3 points, got {}",
                        points.len()
                    )));
                }
                for p in points {
                    require_finite("polygon vertex", p[0])?;
                    require_finite("polygon vertex", p[1])?;
                }
                Ok(())
            }
            RegionShape::Line { x1, y1, x2, y2 } => {
                require_finite("x1", *x1)?;
                require_finite("y1", *y1)?;
                require_finite("x2", *x2)?;
                require_finite("y2", *y2)
            }
            RegionShape::Point { x, y } => {
                require_finite("x", *x)?;
                require_finite("y", *y)
            }
        }
    }

    pub fn bounds(&self) -> PixelBounds {
        match self {
            RegionShape::Circle { x, y, r } => extent_bounds(x - r, y - r, x + r, y + r),
            RegionShape::Annulus { x, y, r_outer, .. } => {
                extent_bounds(x - r_outer, y - r_outer, x + r_outer, y + r_outer)
            }
            RegionShape::Ellipse { x, y, rx, ry, angle } => {
                let (s, c) = sin_cos_deg(*angle);
                let hx = ((rx * c).powi(2) + (ry * s).powi(2)).sqrt();
                let hy = ((rx * s).powi(2) + (ry * c).powi(2)).sqrt();
                extent_bounds(x - hx, y - hy, x + hx, y + hy)
            }
            RegionShape::Box { x, y, width, height, angle } => {
                let hw = width / 2.0;
                let hh = height / 2.0;
                let corners = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].into_iter().map(|(u, v)| {
                    let (dx, dy) = rot_local_to_index(u, v, *angle);
                    (x + dx, y + dy)
                });
                let (xmin, ymin, xmax, ymax) = point_extent(corners);
                extent_bounds(xmin, ymin, xmax, ymax)
            }
            RegionShape::Polygon { points } => {
                let (xmin, ymin, xmax, ymax) = point_extent(points.iter().map(|p| (p[0], p[1])));
                extent_bounds(xmin, ymin, xmax, ymax)
            }
            RegionShape::Line { x1, y1, x2, y2 } => {
                extent_bounds(x1.min(*x2), y1.min(*y2), x1.max(*x2), y1.max(*y2))
            }
            RegionShape::Point { x, y } => extent_bounds(*x, *y, *x, *y),
        }
    }

    pub fn contains(&self, px: f64, py: f64) -> bool {
        match self {
            RegionShape::Circle { x, y, r } => {
                let dx = px - x;
                let dy = py - y;
                dx * dx + dy * dy <= r * r
            }
            RegionShape::Ellipse { x, y, rx, ry, angle } => {
                let (u, v) = rot_index_to_local(px - x, py - y, *angle);
                (u / rx).powi(2) + (v / ry).powi(2) <= 1.0
            }
            RegionShape::Box { x, y, width, height, angle } => {
                let (u, v) = rot_index_to_local(px - x, py - y, *angle);
                u.abs() <= width / 2.0 && v.abs() <= height / 2.0
            }
            RegionShape::Annulus { x, y, r_inner, r_outer } => {
                let dx = px - x;
                let dy = py - y;
                let d2 = dx * dx + dy * dy;
                r_inner * r_inner < d2 && d2 <= r_outer * r_outer
            }
            RegionShape::Polygon { points } => {
                let n = points.len();
                let mut inside = false;
                let mut j = n.wrapping_sub(1);
                for i in 0..n {
                    let (xi, yi) = (points[i][0], points[i][1]);
                    let (xj, yj) = (points[j][0], points[j][1]);
                    if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
            RegionShape::Line { .. } | RegionShape::Point { .. } => false,
        }
    }

    pub fn area(&self) -> f64 {
        match self {
            RegionShape::Circle { r, .. } => std::f64::consts::PI * r * r,
            RegionShape::Ellipse { rx, ry, .. } => std::f64::consts::PI * rx * ry,
            RegionShape::Box { width, height, .. } => width * height,
            RegionShape::Annulus { r_inner, r_outer, .. } => {
                std::f64::consts::PI * (r_outer * r_outer - r_inner * r_inner)
            }
            RegionShape::Polygon { points } => {
                let n = points.len();
                let mut acc = 0.0;
                for i in 0..n {
                    let j = (i + 1) % n;
                    acc += points[i][0] * points[j][1] - points[j][0] * points[i][1];
                }
                acc.abs() / 2.0
            }
            RegionShape::Line { .. } | RegionShape::Point { .. } => 0.0,
        }
    }

    pub fn centre(&self) -> (f64, f64) {
        match self {
            RegionShape::Circle { x, y, .. }
            | RegionShape::Ellipse { x, y, .. }
            | RegionShape::Box { x, y, .. }
            | RegionShape::Annulus { x, y, .. }
            | RegionShape::Point { x, y } => (*x, *y),
            RegionShape::Polygon { points } => {
                if points.is_empty() {
                    return (0.0, 0.0);
                }
                let n = points.len() as f64;
                let sx: f64 = points.iter().map(|p| p[0]).sum();
                let sy: f64 = points.iter().map(|p| p[1]).sum();
                (sx / n, sy / n)
            }
            RegionShape::Line { x1, y1, x2, y2 } => ((x1 + x2) / 2.0, (y1 + y2) / 2.0),
        }
    }

    pub fn translated(&self, dx: f64, dy: f64) -> RegionShape {
        match self.clone() {
            RegionShape::Circle { x, y, r } => RegionShape::Circle { x: x + dx, y: y + dy, r },
            RegionShape::Ellipse { x, y, rx, ry, angle } => {
                RegionShape::Ellipse { x: x + dx, y: y + dy, rx, ry, angle }
            }
            RegionShape::Box { x, y, width, height, angle } => {
                RegionShape::Box { x: x + dx, y: y + dy, width, height, angle }
            }
            RegionShape::Annulus { x, y, r_inner, r_outer } => {
                RegionShape::Annulus { x: x + dx, y: y + dy, r_inner, r_outer }
            }
            RegionShape::Polygon { points } => RegionShape::Polygon {
                points: points.into_iter().map(|p| [p[0] + dx, p[1] + dy]).collect(),
            },
            RegionShape::Line { x1, y1, x2, y2 } => {
                RegionShape::Line { x1: x1 + dx, y1: y1 + dy, x2: x2 + dx, y2: y2 + dy }
            }
            RegionShape::Point { x, y } => RegionShape::Point { x: x + dx, y: y + dy },
        }
    }

    pub fn to_ds9_image(&self) -> RegionShape {
        self.translated(DS9_PIXEL_OFFSET, DS9_PIXEL_OFFSET)
    }

    pub fn from_ds9_image(&self) -> RegionShape {
        self.translated(-DS9_PIXEL_OFFSET, -DS9_PIXEL_OFFSET)
    }

    fn bounds_exceed(&self, rows: usize, cols: usize) -> bool {
        let b = self.bounds();
        b.x0 < 0 || b.y0 < 0 || b.x1 >= cols as i64 || b.y1 >= rows as i64
    }

    pub fn masked_values(&self, arr: &Array2<f32>, excluded: Option<&Array2<u8>>) -> MaskedValues {
        self.masked_values_with_err(arr, None, excluded)
    }

    pub fn masked_values_with_err(
        &self,
        arr: &Array2<f32>,
        err: Option<&Array2<f32>>,
        excluded: Option<&Array2<u8>>,
    ) -> MaskedValues {
        let (rows, cols) = arr.dim();
        let err = err.filter(|e| e.dim() == arr.dim());
        let clipped = self.bounds_exceed(rows, cols);
        let scan = match self {
            RegionShape::Line { x1, y1, x2, y2 } => {
                let length = (x2 - x1).hypot(y2 - y1);
                let n = length.ceil() as usize + 1;
                let mut last: Option<(i64, i64)> = None;
                let mut pixels = Vec::with_capacity(n);
                for i in 0..n {
                    let t = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
                    let p = nearest_pixel(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
                    if last != Some(p) {
                        pixels.push(p);
                        last = Some(p);
                    }
                }
                sample_pixels(arr, err, excluded, pixels.into_iter())
            }
            RegionShape::Point { x, y } => {
                sample_pixels(arr, err, excluded, std::iter::once(nearest_pixel(*x, *y)))
            }
            _ => self.scan_lattice(arr, err, excluded),
        };
        MaskedValues {
            values: scan.values,
            errors: scan.errors,
            n_inside: scan.n_inside,
            n_nan: scan.n_nan,
            n_excluded: scan.n_excluded,
            clipped,
        }
    }

    fn scan_lattice(
        &self,
        arr: &Array2<f32>,
        err: Option<&Array2<f32>>,
        excluded: Option<&Array2<u8>>,
    ) -> RowScan {
        let (rows, cols) = arr.dim();
        let b = self.bounds();
        let x0 = b.x0.max(0);
        let y0 = b.y0.max(0);
        let x1 = b.x1.min(cols as i64 - 1);
        let y1 = b.y1.min(rows as i64 - 1);
        if x0 > x1 || y0 > y1 {
            return RowScan::empty();
        }
        let scan_row = |py: i64| {
            let mut scan = RowScan::empty();
            let uy = py as usize;
            let fy = py as f64;
            for px in x0..=x1 {
                if !self.contains(px as f64, fy) {
                    continue;
                }
                let ux = px as usize;
                let ex = excluded.is_some_and(|m| m[[uy, ux]] != 0);
                scan.take(arr[[uy, ux]], err.map(|e| e[[uy, ux]]), ex);
            }
            scan
        };
        let bbox_area = ((x1 - x0 + 1) as usize) * ((y1 - y0 + 1) as usize);
        if bbox_area > PAR_BBOX_PIXELS {
            (y0..=y1)
                .into_par_iter()
                .map(scan_row)
                .reduce(RowScan::empty, RowScan::merge)
        } else {
            (y0..=y1).map(scan_row).fold(RowScan::empty(), RowScan::merge)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SigmaClip {
    pub sigma: f32,
    pub maxiters: usize,
}

impl Default for SigmaClip {
    fn default() -> Self {
        Self { sigma: 3.0, maxiters: 5 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BackgroundEstimate {
    pub median: f64,
    pub sigma: f64,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RegionStats {
    pub count: u64,
    pub n_nan: u64,
    pub n_excluded: u64,
    pub area: f64,
    pub bounds: PixelBounds,
    pub clipped: bool,
    pub sum: f64,
    pub mean: f64,
    pub median: f64,
    pub mad: f64,
    pub sigma: f64,
    pub std: f64,
    pub min: f64,
    pub max: f64,
    pub clipped_mean: f64,
    pub clipped_median: f64,
    pub clipped_sigma: f64,
    pub n_rejected: u64,
    pub background: Option<BackgroundEstimate>,
    pub net_sum: Option<f64>,
    pub net_snr: Option<f64>,
    pub sum_err: Option<f64>,
    pub weighted_mean: Option<f64>,
}

fn background_estimate(mut values: Vec<f32>) -> Option<BackgroundEstimate> {
    if values.is_empty() {
        return None;
    }
    let count = values.len() as u64;
    let median = exact_median_mut(&mut values);
    let mad = exact_mad_mut(&mut values, median as f32) as f64;
    Some(BackgroundEstimate { median, sigma: mad * MAD_TO_SIGMA, count })
}

fn sample_std(values: &[f32], mean: f64) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let ss: f64 = values
        .iter()
        .map(|&v| {
            let d = v as f64 - mean;
            d * d
        })
        .sum();
    (ss / (values.len() - 1) as f64).sqrt()
}

pub fn region_stats(
    arr: &Array2<f32>,
    shape: &RegionShape,
    background: Option<&RegionShape>,
    excluded: Option<&Array2<u8>>,
    err: Option<&Array2<f32>>,
    clip: SigmaClip,
) -> Result<RegionStats, RegionError> {
    shape.validate()?;
    if let Some(bg) = background {
        bg.validate()?;
    }
    let mv = shape.masked_values_with_err(arr, err, excluded);
    if mv.values.is_empty() {
        return Err(RegionError::Empty { shape: shape.kind() });
    }
    let bg = match background {
        Some(bg_shape) => {
            let bg_mv = bg_shape.masked_values(arr, excluded);
            Some(
                background_estimate(bg_mv.values)
                    .ok_or_else(|| RegionError::Invalid("background region is empty".into()))?,
            )
        }
        None => None,
    };

    let count = mv.values.len() as u64;
    let sum: f64 = mv.values.iter().map(|&v| v as f64).sum();
    let mut sorted = mv.values.clone();
    let base = finite_slice_stats(&mut sorted);
    let std = sample_std(&mv.values, base.mean);

    let mut kept = mv.values.clone();
    let (clipped_median, clipped_sigma) = sigma_clipped_stats(&mut kept, clip.sigma, clip.maxiters);
    let n_rejected = count - kept.len() as u64;
    let clipped_mean = if kept.is_empty() {
        0.0
    } else {
        kept.iter().map(|&v| v as f64).sum::<f64>() / kept.len() as f64
    };

    let net_sum = bg.as_ref().map(|b| sum - b.median * count as f64);
    let net_snr = match (&bg, net_sum) {
        (Some(b), Some(ns)) if b.sigma > 0.0 => Some(ns / (b.sigma * (count as f64).sqrt())),
        _ => None,
    };
    let (sum_err, weighted_mean) = error_summary(&mv.values, &mv.errors);

    Ok(RegionStats {
        count,
        n_nan: mv.n_nan,
        n_excluded: mv.n_excluded,
        area: shape.area(),
        bounds: shape.bounds(),
        clipped: mv.clipped,
        sum,
        mean: base.mean,
        median: base.median,
        mad: base.mad,
        sigma: base.mad * MAD_TO_SIGMA,
        std,
        min: base.min,
        max: base.max,
        clipped_mean,
        clipped_median,
        clipped_sigma,
        n_rejected,
        background: bg,
        net_sum,
        net_snr,
        sum_err,
        weighted_mean,
    })
}

fn error_summary(values: &[f32], errors: &[f32]) -> (Option<f64>, Option<f64>) {
    if errors.len() != values.len() || errors.is_empty() {
        return (None, None);
    }
    let mut sum_sq = 0.0f64;
    let mut n_err = 0u64;
    let mut weighted_sum = 0.0f64;
    let mut weight_total = 0.0f64;
    for (&v, &e) in values.iter().zip(errors) {
        if !e.is_finite() || e < 0.0 {
            continue;
        }
        let e = e as f64;
        sum_sq += e * e;
        n_err += 1;
        if e > 0.0 {
            let w = 1.0 / (e * e);
            weighted_sum += w * v as f64;
            weight_total += w;
        }
    }
    if n_err == 0 {
        return (None, None);
    }
    let weighted_mean = (weight_total > 0.0).then(|| weighted_sum / weight_total);
    (Some(sum_sq.sqrt()), weighted_mean)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RadialBin {
    pub r: u32,
    pub count: u64,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub std: Option<f64>,
    pub cumulative_sum: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RadialProfile {
    pub x: f64,
    pub y: f64,
    pub max_radius: f64,
    pub background: Option<BackgroundEstimate>,
    pub bins: Vec<RadialBin>,
}

pub fn radial_profile(
    arr: &Array2<f32>,
    x: f64,
    y: f64,
    max_radius: f64,
    background: Option<(f64, f64)>,
    excluded: Option<&Array2<u8>>,
) -> Result<RadialProfile, RegionError> {
    require_finite("x", x)?;
    require_finite("y", y)?;
    if !max_radius.is_finite() || max_radius <= 0.0 || max_radius > MAX_RADIAL_RADIUS {
        return Err(RegionError::Invalid(format!(
            "max_radius must be in (0, {MAX_RADIAL_RADIUS}], got {max_radius}"
        )));
    }
    let bg = match background {
        Some((r_in, r_out)) => {
            let annulus = RegionShape::Annulus { x, y, r_inner: r_in, r_outer: r_out };
            annulus.validate()?;
            let bg_mv = annulus.masked_values(arr, excluded);
            Some(
                background_estimate(bg_mv.values)
                    .ok_or_else(|| RegionError::Invalid("background region is empty".into()))?,
            )
        }
        None => None,
    };
    let offset = bg.as_ref().map_or(0.0, |b| b.median);

    let nbins = max_radius.ceil() as usize;
    let mut per_bin: Vec<Vec<f32>> = vec![Vec::new(); nbins];
    let (rows, cols) = arr.dim();
    let b = RegionShape::Circle { x, y, r: max_radius }.bounds();
    let x0 = b.x0.max(0);
    let y0 = b.y0.max(0);
    let x1 = b.x1.min(cols as i64 - 1);
    let y1 = b.y1.min(rows as i64 - 1);
    for py in y0..=y1 {
        for px in x0..=x1 {
            let d = (px as f64 - x).hypot(py as f64 - y);
            let k = d.floor() as usize;
            if k >= nbins {
                continue;
            }
            let (ux, uy) = (px as usize, py as usize);
            if excluded.is_some_and(|m| m[[uy, ux]] != 0) {
                continue;
            }
            let v = arr[[uy, ux]];
            if v.is_finite() {
                per_bin[k].push(v);
            }
        }
    }

    let mut cumulative = 0.0;
    let bins = per_bin
        .into_iter()
        .enumerate()
        .map(|(k, mut vals)| {
            let count = vals.len() as u64;
            if count == 0 {
                return RadialBin { r: k as u32, count, mean: None, median: None, std: None, cumulative_sum: cumulative };
            }
            let sum: f64 = vals.iter().map(|&v| v as f64 - offset).sum();
            cumulative += sum;
            let mean = sum / count as f64;
            let std = sample_std(&vals, mean + offset);
            let median = exact_median_mut(&mut vals) - offset;
            RadialBin {
                r: k as u32,
                count,
                mean: Some(mean),
                median: Some(median),
                std: Some(std),
                cumulative_sum: cumulative,
            }
        })
        .collect();

    Ok(RadialProfile { x, y, max_radius, background: bg, bins })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineCut {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub length: f64,
    pub n_samples: usize,
    pub distance: Vec<f64>,
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub values: Vec<Option<f64>>,
}

fn bilinear_base(c: f64, n: usize) -> Option<(usize, f64)> {
    if n < 2 || c < 0.0 || c > (n - 1) as f64 {
        return None;
    }
    if c == (n - 1) as f64 {
        return Some((n - 2, 1.0));
    }
    let c0 = c.floor();
    Some((c0 as usize, c - c0))
}

fn bilinear_sample(arr: &Array2<f32>, excluded: Option<&Array2<u8>>, x: f64, y: f64) -> Option<f64> {
    let (rows, cols) = arr.dim();
    let (x0, fx) = bilinear_base(x, cols)?;
    let (y0, fy) = bilinear_base(y, rows)?;
    let mut corners = [0.0f64; 4];
    for (i, (ux, uy)) in [(x0, y0), (x0 + 1, y0), (x0, y0 + 1), (x0 + 1, y0 + 1)].into_iter().enumerate() {
        if excluded.is_some_and(|m| m[[uy, ux]] != 0) {
            return None;
        }
        let v = arr[[uy, ux]];
        if !v.is_finite() {
            return None;
        }
        corners[i] = v as f64;
    }
    let top = corners[0] * (1.0 - fx) + corners[1] * fx;
    let bottom = corners[2] * (1.0 - fx) + corners[3] * fx;
    Some(top * (1.0 - fy) + bottom * fy)
}

pub fn line_cut(
    arr: &Array2<f32>,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    excluded: Option<&Array2<u8>>,
) -> Result<LineCut, RegionError> {
    RegionShape::Line { x1, y1, x2, y2 }.validate()?;
    let length = (x2 - x1).hypot(y2 - y1);
    let n_samples = length.ceil() as usize + 1;
    let mut distance = Vec::with_capacity(n_samples);
    let mut xs = Vec::with_capacity(n_samples);
    let mut ys = Vec::with_capacity(n_samples);
    let mut values = Vec::with_capacity(n_samples);
    for i in 0..n_samples {
        let t = if n_samples > 1 { i as f64 / (n_samples - 1) as f64 } else { 0.0 };
        let sx = x1 + (x2 - x1) * t;
        let sy = y1 + (y2 - y1) * t;
        distance.push(t * length);
        xs.push(sx);
        ys.push(sy);
        values.push(bilinear_sample(arr, excluded, sx, sy));
    }
    Ok(LineCut { x1, y1, x2, y2, length, n_samples, distance, xs, ys, values })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegionSystem {
    #[default]
    Image,
    Fk5,
    Icrs,
}

impl RegionSystem {
    pub fn parse(name: &str) -> Result<Self, RegionError> {
        match name.trim().to_ascii_lowercase().as_str() {
            "image" | "physical" => Ok(RegionSystem::Image),
            "fk5" => Ok(RegionSystem::Fk5),
            "icrs" => Ok(RegionSystem::Icrs),
            other => Err(RegionError::UnsupportedSystem(other.to_string())),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RegionSystem::Image => "image",
            RegionSystem::Fk5 => "fk5",
            RegionSystem::Icrs => "icrs",
        }
    }

    pub fn is_sky(self) -> bool {
        !matches!(self, RegionSystem::Image)
    }
}

pub fn wcs_north_angle_deg(wcs: &WcsTransform) -> f64 {
    let (_, _, _, _, cd, _) = wcs.raw_params();
    cd[0][1].atan2(cd[1][1]).to_degrees()
}

fn sky_scale(wcs: &WcsTransform) -> Result<f64, RegionError> {
    let scale = wcs.pixel_scale_arcsec();
    if scale.is_finite() && scale > 0.0 {
        Ok(scale)
    } else {
        Err(RegionError::Invalid("image WCS has a degenerate pixel scale".into()))
    }
}

fn sky_to_pixel_pos(wcs: &WcsTransform, system: RegionSystem, lon: f64, lat: f64) -> Result<(f64, f64), RegionError> {
    let (ra, dec) = match system {
        RegionSystem::Fk5 => fk5_j2000_to_icrs(lon, lat),
        _ => (lon, lat),
    };
    let (px, py) = wcs.world_to_pixel(ra, dec);
    if px.is_finite() && py.is_finite() {
        Ok((px, py))
    } else {
        Err(RegionError::OffImage)
    }
}

fn pixel_to_sky_pos(wcs: &WcsTransform, system: RegionSystem, px: f64, py: f64) -> Result<(f64, f64), RegionError> {
    let c = wcs.pixel_to_world(px, py);
    if !c.ra.is_finite() || !c.dec.is_finite() {
        return Err(RegionError::OffImage);
    }
    Ok(match system {
        RegionSystem::Fk5 => icrs_to_fk5_j2000(c.ra, c.dec),
        _ => (c.ra, c.dec),
    })
}

fn convert_shape(
    shape: &RegionShape,
    pos: &dyn Fn(f64, f64) -> Result<(f64, f64), RegionError>,
    size: &dyn Fn(f64) -> f64,
    angle: &dyn Fn(f64) -> f64,
) -> Result<RegionShape, RegionError> {
    Ok(match shape {
        RegionShape::Circle { x, y, r } => {
            let (nx, ny) = pos(*x, *y)?;
            RegionShape::Circle { x: nx, y: ny, r: size(*r) }
        }
        RegionShape::Ellipse { x, y, rx, ry, angle: a } => {
            let (nx, ny) = pos(*x, *y)?;
            RegionShape::Ellipse { x: nx, y: ny, rx: size(*rx), ry: size(*ry), angle: angle(*a) }
        }
        RegionShape::Box { x, y, width, height, angle: a } => {
            let (nx, ny) = pos(*x, *y)?;
            RegionShape::Box { x: nx, y: ny, width: size(*width), height: size(*height), angle: angle(*a) }
        }
        RegionShape::Annulus { x, y, r_inner, r_outer } => {
            let (nx, ny) = pos(*x, *y)?;
            RegionShape::Annulus { x: nx, y: ny, r_inner: size(*r_inner), r_outer: size(*r_outer) }
        }
        RegionShape::Polygon { points } => {
            let mut out = Vec::with_capacity(points.len());
            for p in points {
                let (nx, ny) = pos(p[0], p[1])?;
                out.push([nx, ny]);
            }
            RegionShape::Polygon { points: out }
        }
        RegionShape::Line { x1, y1, x2, y2 } => {
            let (nx1, ny1) = pos(*x1, *y1)?;
            let (nx2, ny2) = pos(*x2, *y2)?;
            RegionShape::Line { x1: nx1, y1: ny1, x2: nx2, y2: ny2 }
        }
        RegionShape::Point { x, y } => {
            let (nx, ny) = pos(*x, *y)?;
            RegionShape::Point { x: nx, y: ny }
        }
    })
}

pub fn shape_to_pixel(
    shape: &RegionShape,
    system: RegionSystem,
    wcs: Option<&WcsTransform>,
) -> Result<RegionShape, RegionError> {
    if !system.is_sky() {
        return Ok(shape.clone());
    }
    let wcs = wcs.ok_or(RegionError::WcsRequired)?;
    let scale = sky_scale(wcs)?;
    let north = wcs_north_angle_deg(wcs);
    convert_shape(
        shape,
        &|lon, lat| sky_to_pixel_pos(wcs, system, lon, lat),
        &|arcsec| arcsec / scale,
        &|a| a - north,
    )
}

pub fn shape_to_sky(
    shape: &RegionShape,
    system: RegionSystem,
    wcs: Option<&WcsTransform>,
) -> Result<RegionShape, RegionError> {
    if !system.is_sky() {
        return Ok(shape.clone());
    }
    let wcs = wcs.ok_or(RegionError::WcsRequired)?;
    let scale = sky_scale(wcs)?;
    let north = wcs_north_angle_deg(wcs);
    convert_shape(
        shape,
        &|px, py| pixel_to_sky_pos(wcs, system, px, py),
        &|px| px * scale,
        &|a| a + north,
    )
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashMap;

    use crate::types::header::HduHeader;

    pub fn make_header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut index = HashMap::new();
        let mut cards = Vec::new();
        for &(k, v) in pairs {
            index.insert(k.to_string(), v.to_string());
            cards.push((k.to_string(), v.to_string()));
        }
        HduHeader { cards, index }
    }

    pub fn wcs_cards(cd: [[f64; 2]; 2]) -> Vec<(String, String)> {
        vec![
            ("NAXIS1".to_string(), "100".to_string()),
            ("NAXIS2".to_string(), "100".to_string()),
            ("CRPIX1".to_string(), "50.5".to_string()),
            ("CRPIX2".to_string(), "50.5".to_string()),
            ("CRVAL1".to_string(), "150.0".to_string()),
            ("CRVAL2".to_string(), "2.0".to_string()),
            ("CD1_1".to_string(), format!("{:.12e}", cd[0][0])),
            ("CD1_2".to_string(), format!("{:.12e}", cd[0][1])),
            ("CD2_1".to_string(), format!("{:.12e}", cd[1][0])),
            ("CD2_2".to_string(), format!("{:.12e}", cd[1][1])),
            ("CTYPE1".to_string(), "RA---TAN".to_string()),
            ("CTYPE2".to_string(), "DEC--TAN".to_string()),
        ]
    }

    pub fn north_up_cd() -> [[f64; 2]; 2] {
        let s = 1.0 / 3600.0;
        [[-s, 0.0], [0.0, s]]
    }

    pub fn rotated_cd(deg: f64) -> [[f64; 2]; 2] {
        let s = 1.0 / 3600.0;
        let (sn, cs) = deg.to_radians().sin_cos();
        [[-cs * s, -sn * s], [-sn * s, cs * s]]
    }

    pub fn header_with_cd(cd: [[f64; 2]; 2]) -> HduHeader {
        let cards = wcs_cards(cd);
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        make_header(&pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{header_with_cd, north_up_cd, rotated_cd};
    use super::*;

    fn circle(x: f64, y: f64, r: f64) -> RegionShape {
        RegionShape::Circle { x, y, r }
    }

    fn zeros(rows: usize, cols: usize) -> Array2<f32> {
        Array2::zeros((rows, cols))
    }

    fn brute_count(shape: &RegionShape, rows: usize, cols: usize) -> u64 {
        let mut n = 0;
        for y in 0..rows {
            for x in 0..cols {
                if shape.contains(x as f64, y as f64) {
                    n += 1;
                }
            }
        }
        n
    }

    fn pixel_set(shape: &RegionShape, rows: usize, cols: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for y in 0..rows {
            for x in 0..cols {
                if shape.contains(x as f64, y as f64) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn ds9_circle_counts_at_integer_centre() {
        let arr = zeros(32, 32);
        let mv = circle(10.0, 10.0, 5.0).masked_values(&arr, None);
        assert_eq!(mv.n_inside, 81);
        assert_eq!(mv.values.len(), 81);
        assert!(!mv.clipped);
        let mv = circle(10.0, 10.0, 2.0).masked_values(&arr, None);
        assert_eq!(mv.n_inside, 13);
    }

    #[test]
    fn annulus_count_is_difference_of_circles() {
        let arr = zeros(32, 32);
        let ann = RegionShape::Annulus { x: 10.0, y: 10.0, r_inner: 2.0, r_outer: 5.0 };
        assert_eq!(ann.masked_values(&arr, None).n_inside, 68);
        let outer = circle(10.0, 10.0, 5.0).masked_values(&arr, None).n_inside;
        let inner = circle(10.0, 10.0, 2.0).masked_values(&arr, None).n_inside;
        assert_eq!((outer, inner), (81, 13));
        for r_inner in 1..5 {
            let ri = r_inner as f64;
            let ann = RegionShape::Annulus { x: 10.0, y: 10.0, r_inner: ri, r_outer: 5.0 };
            let inner = circle(10.0, 10.0, ri).masked_values(&arr, None).n_inside;
            let outer = circle(10.0, 10.0, 5.0).masked_values(&arr, None).n_inside;
            assert_eq!(ann.masked_values(&arr, None).n_inside, outer - inner, "r_inner {ri}");
        }
    }

    #[test]
    fn half_pixel_centre_matches_brute_force_and_differs_from_integer_centre() {
        let arr = zeros(32, 32);
        let c = circle(10.5, 10.5, 5.0);
        let mv = c.masked_values(&arr, None);
        assert_ne!(mv.n_inside, 81);
        assert_eq!(mv.n_inside, brute_count(&c, 32, 32));
    }

    #[test]
    fn circle_at_origin_is_clipped() {
        let arr = zeros(10, 10);
        let mv = circle(0.0, 0.0, 2.0).masked_values(&arr, None);
        assert_eq!(mv.n_inside, 6);
        assert!(mv.clipped);
    }

    #[test]
    fn box_counts_and_rotation_by_90_transposes() {
        let arr = zeros(32, 32);
        let b0 = RegionShape::Box { x: 10.0, y: 10.0, width: 4.0, height: 2.0, angle: 0.0 };
        let b90 = RegionShape::Box { x: 10.0, y: 10.0, width: 4.0, height: 2.0, angle: 90.0 };
        assert_eq!(b0.masked_values(&arr, None).n_inside, 15);
        assert_eq!(b90.masked_values(&arr, None).n_inside, 15);
        let s0 = pixel_set(&b0, 32, 32);
        let mut s90: Vec<(usize, usize)> = pixel_set(&b90, 32, 32).into_iter().map(|(x, y)| (y, x)).collect();
        s90.sort();
        let mut s0 = s0;
        s0.sort();
        assert_eq!(s0, s90);
    }

    #[test]
    fn ellipse_matches_circle_and_rotated_axes_swap() {
        let e = RegionShape::Ellipse { x: 10.0, y: 10.0, rx: 5.0, ry: 5.0, angle: 0.0 };
        assert_eq!(pixel_set(&e, 32, 32), pixel_set(&circle(10.0, 10.0, 5.0), 32, 32));
        let a = RegionShape::Ellipse { x: 10.0, y: 10.0, rx: 5.0, ry: 2.0, angle: 90.0 };
        let b = RegionShape::Ellipse { x: 10.0, y: 10.0, rx: 2.0, ry: 5.0, angle: 0.0 };
        assert_eq!(pixel_set(&a, 32, 32), pixel_set(&b, 32, 32));
    }

    #[test]
    fn polygon_square_and_concave_counts() {
        let arr = zeros(16, 16);
        let sq = RegionShape::Polygon { points: vec![[0.5, 0.5], [4.5, 0.5], [4.5, 4.5], [0.5, 4.5]] };
        assert_eq!(sq.masked_values(&arr, None).n_inside, 16);
        let l = RegionShape::Polygon {
            points: vec![[0.5, 0.5], [8.5, 0.5], [8.5, 3.5], [3.5, 3.5], [3.5, 8.5], [0.5, 8.5]],
        };
        let mv = l.masked_values(&arr, None);
        assert_eq!(mv.n_inside, brute_count(&l, 16, 16));
        assert_eq!(mv.n_inside, 8 * 3 + 3 * 5);
    }

    #[test]
    fn line_and_point_sampling() {
        let mut arr = zeros(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                arr[[y, x]] = x as f32;
            }
        }
        let line = RegionShape::Line { x1: 0.0, y1: 3.0, x2: 5.0, y2: 3.0 };
        let mv = line.masked_values(&arr, None);
        assert_eq!(mv.values, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(mv.n_inside, 6);
        arr[[3, 3]] = 42.0;
        let p = RegionShape::Point { x: 3.4, y: 2.6 };
        let mv = p.masked_values(&arr, None);
        assert_eq!(mv.values, vec![42.0]);
        assert_eq!(mv.n_inside, 1);
    }

    #[test]
    fn masked_values_categories_partition_inside() {
        let mut arr = zeros(16, 16);
        arr[[8, 8]] = f32::NAN;
        let mut ex = Array2::<u8>::zeros((16, 16));
        ex[[8, 9]] = 1;
        let mv = circle(8.0, 8.0, 2.0).masked_values(&arr, Some(&ex));
        assert_eq!(mv.n_inside, 13);
        assert_eq!(mv.n_nan, 1);
        assert_eq!(mv.n_excluded, 1);
        assert_eq!(mv.values.len() as u64 + mv.n_nan + mv.n_excluded, mv.n_inside);
    }

    #[test]
    fn large_bbox_uses_parallel_path_with_same_result() {
        let arr = zeros(600, 600);
        let c = circle(300.0, 300.0, 280.0);
        let mv = c.masked_values(&arr, None);
        let area = (c.bounds().x1 - c.bounds().x0 + 1) * (c.bounds().y1 - c.bounds().y0 + 1);
        assert!(area as usize > PAR_BBOX_PIXELS);
        assert_eq!(mv.n_inside, brute_count(&c, 600, 600));
    }

    #[test]
    fn validate_rejects_bad_shapes() {
        assert!(matches!(circle(1.0, 1.0, 0.0).validate(), Err(RegionError::Invalid(_))));
        assert!(matches!(circle(1.0, 1.0, -1.0).validate(), Err(RegionError::Invalid(_))));
        let ann = RegionShape::Annulus { x: 0.0, y: 0.0, r_inner: 5.0, r_outer: 5.0 };
        assert!(matches!(ann.validate(), Err(RegionError::Invalid(_))));
        let poly = RegionShape::Polygon { points: vec![[0.0, 0.0], [1.0, 1.0]] };
        assert!(matches!(poly.validate(), Err(RegionError::Invalid(_))));
        assert!(matches!(circle(f64::NAN, 1.0, 1.0).validate(), Err(RegionError::Invalid(_))));
        let bx = RegionShape::Box { x: 0.0, y: 0.0, width: 1.0, height: 0.0, angle: 0.0 };
        assert!(bx.validate().is_err());
        assert!(circle(1.0, 1.0, 1.0).validate().is_ok());
    }

    #[test]
    fn bounds_of_rotated_box_match_corners_and_areas() {
        let b = RegionShape::Box { x: 10.0, y: 10.0, width: 4.0, height: 4.0, angle: 45.0 };
        let half = 2.0 * std::f64::consts::SQRT_2;
        let pb = b.bounds();
        assert_eq!(pb.x0, (10.0 - half).floor() as i64);
        assert_eq!(pb.x1, (10.0 + half).ceil() as i64);
        assert_eq!(pb.y0, (10.0 - half).floor() as i64);
        assert_eq!(pb.y1, (10.0 + half).ceil() as i64);
        assert_eq!((pb.x0, pb.x1), (7, 13));
        assert!((circle(0.0, 0.0, 5.0).area() - std::f64::consts::PI * 25.0).abs() < 1e-12);
        let ann = RegionShape::Annulus { x: 0.0, y: 0.0, r_inner: 2.0, r_outer: 5.0 };
        assert!((ann.area() - std::f64::consts::PI * 21.0).abs() < 1e-12);
        let sq = RegionShape::Polygon { points: vec![[0.5, 0.5], [4.5, 0.5], [4.5, 4.5], [0.5, 4.5]] };
        assert!((sq.area() - 16.0).abs() < 1e-12);
        assert_eq!(RegionShape::Line { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 }.area(), 0.0);
    }

    #[test]
    fn ds9_offset_round_trip_only_moves_positions() {
        let e = RegionShape::Ellipse { x: 1.0, y: 2.0, rx: 3.0, ry: 4.0, angle: 30.0 };
        let d = e.to_ds9_image();
        assert_eq!(d, RegionShape::Ellipse { x: 2.0, y: 3.0, rx: 3.0, ry: 4.0, angle: 30.0 });
        assert_eq!(d.from_ds9_image(), e);
        let p = RegionShape::Polygon { points: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]] };
        assert_eq!(p.to_ds9_image().from_ds9_image(), p);
        let l = RegionShape::Line { x1: 0.0, y1: 0.0, x2: 3.0, y2: 4.0 };
        assert_eq!(l.to_ds9_image(), RegionShape::Line { x1: 1.0, y1: 1.0, x2: 4.0, y2: 5.0 });
    }

    #[test]
    fn serde_round_trip_and_defaults() {
        let b: RegionShape = serde_json::from_str(r#"{"shape":"box","x":1,"y":2,"width":3,"height":4}"#).unwrap();
        assert_eq!(b, RegionShape::Box { x: 1.0, y: 2.0, width: 3.0, height: 4.0, angle: 0.0 });
        let all = vec![
            circle(10.0, 20.0, 5.0),
            RegionShape::Ellipse { x: 1.0, y: 2.0, rx: 3.0, ry: 4.0, angle: 30.0 },
            RegionShape::Box { x: 1.0, y: 2.0, width: 3.0, height: 4.0, angle: 30.0 },
            RegionShape::Annulus { x: 1.0, y: 2.0, r_inner: 1.0, r_outer: 2.0 },
            RegionShape::Polygon { points: vec![[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]] },
            RegionShape::Line { x1: 0.0, y1: 1.0, x2: 2.0, y2: 3.0 },
            RegionShape::Point { x: 1.0, y: 2.0 },
        ];
        for s in all {
            let j = serde_json::to_string(&s).unwrap();
            assert!(j.contains(&format!("\"shape\":\"{}\"", s.kind())), "{j}");
            let back: RegionShape = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
        let j = serde_json::to_value(circle(10.0, 20.0, 5.0)).unwrap();
        assert_eq!(j, serde_json::json!({"shape":"circle","x":10.0,"y":20.0,"r":5.0}));
        assert!(serde_json::from_str::<RegionShape>(r#"{"shape":"hexagon","x":1,"y":2}"#).is_err());
    }

    fn known_4x4() -> Array2<f32> {
        let mut arr = Array2::from_elem((6, 6), -100.0f32);
        for y in 0..4 {
            for x in 0..4 {
                arr[[y + 1, x + 1]] = (y * 4 + x + 1) as f32;
            }
        }
        arr
    }

    fn whole() -> RegionShape {
        RegionShape::Box { x: 2.5, y: 2.5, width: 4.0, height: 4.0, angle: 0.0 }
    }

    #[test]
    fn region_stats_known_values() {
        let arr = known_4x4();
        let s = region_stats(&arr, &whole(), None, None, None, SigmaClip::default()).unwrap();
        assert_eq!(s.count, 16);
        assert_eq!(s.n_nan, 0);
        assert_eq!(s.sum, 136.0);
        assert_eq!(s.mean, 8.5);
        assert_eq!(s.median, 8.5);
        assert_eq!(s.mad, 4.0);
        assert!((s.sigma - 4.0 * MAD_TO_SIGMA).abs() < 1e-12);
        assert!((s.std - (22.666666666666668f64).sqrt()).abs() < 1e-9);
        assert_eq!(s.min, 1.0);
        assert_eq!(s.max, 16.0);
        assert_eq!(s.area, 16.0);
        assert!(!s.clipped);
        assert_eq!(s.n_rejected, 0);
        assert!(s.background.is_none() && s.net_sum.is_none() && s.net_snr.is_none());
    }

    #[test]
    fn region_stats_counts_nan_and_excluded_separately() {
        let mut arr = known_4x4();
        arr[[1, 1]] = f32::NAN;
        let s = region_stats(&arr, &whole(), None, None, None, SigmaClip::default()).unwrap();
        assert_eq!(s.count, 15);
        assert_eq!(s.n_nan, 1);
        assert_eq!(s.sum, 135.0);
        let mut ex = Array2::<u8>::zeros((6, 6));
        ex[[4, 4]] = 1;
        ex[[3, 3]] = 7;
        let s = region_stats(&arr, &whole(), None, Some(&ex), None, SigmaClip::default()).unwrap();
        assert_eq!(s.count, 13);
        assert_eq!(s.n_excluded, 2);
        assert_eq!(s.max, 15.0);
        assert_eq!(s.sum, 135.0 - 16.0 - 11.0);
    }

    #[test]
    fn region_stats_with_err_plane_reports_sum_err_and_weighted_mean() {
        let arr = known_4x4();
        let err = Array2::from_elem((6, 6), 0.5f32);
        let s = region_stats(&arr, &whole(), None, None, Some(&err), SigmaClip::default()).unwrap();
        assert!((s.sum_err.unwrap() - 0.5 * 4.0).abs() < 1e-9, "sum_err={:?}", s.sum_err);
        assert!((s.weighted_mean.unwrap() - s.mean).abs() < 1e-9, "weighted_mean={:?}", s.weighted_mean);
        assert_eq!(s.count, 16);
        assert_eq!(s.sum, 136.0);

        let mut varied = err.clone();
        varied[[1, 1]] = 2.0;
        let s = region_stats(&arr, &whole(), None, None, Some(&varied), SigmaClip::default()).unwrap();
        assert!((s.sum_err.unwrap() - (15.0f64 * 0.25 + 4.0).sqrt()).abs() < 1e-9);
        let expected_weighted = (4.0 * 135.0 + 0.25) / (15.0 * 4.0 + 0.25);
        assert!((s.weighted_mean.unwrap() - expected_weighted).abs() < 1e-9, "{:?}", s.weighted_mean);

        let mut nan_err = err.clone();
        nan_err[[2, 2]] = f32::NAN;
        let s = region_stats(&arr, &whole(), None, None, Some(&nan_err), SigmaClip::default()).unwrap();
        assert!((s.sum_err.unwrap() - 0.5 * 15f64.sqrt()).abs() < 1e-9);
        assert!((s.weighted_mean.unwrap() - (136.0 - 6.0) / 15.0).abs() < 1e-9);
        assert_eq!(s.count, 16);

        let mut zero_err = err.clone();
        zero_err[[1, 1]] = 0.0;
        let s = region_stats(&arr, &whole(), None, None, Some(&zero_err), SigmaClip::default()).unwrap();
        assert!((s.sum_err.unwrap() - 0.5 * 15f64.sqrt()).abs() < 1e-9);
        assert!((s.weighted_mean.unwrap() - 135.0 / 15.0).abs() < 1e-9);

        let mut nan_arr = known_4x4();
        nan_arr[[1, 1]] = f32::NAN;
        let mut ex = Array2::<u8>::zeros((6, 6));
        ex[[4, 4]] = 1;
        let s = region_stats(&nan_arr, &whole(), None, Some(&ex), Some(&err), SigmaClip::default()).unwrap();
        assert_eq!(s.count, 14);
        assert!((s.sum_err.unwrap() - 0.5 * 14f64.sqrt()).abs() < 1e-9);
        assert!((s.weighted_mean.unwrap() - s.mean).abs() < 1e-9);

        let wrong = Array2::from_elem((3, 3), 0.5f32);
        let s = region_stats(&arr, &whole(), None, None, Some(&wrong), SigmaClip::default()).unwrap();
        assert!(s.sum_err.is_none() && s.weighted_mean.is_none());

        let all_nan_err = Array2::from_elem((6, 6), f32::NAN);
        let s = region_stats(&arr, &whole(), None, None, Some(&all_nan_err), SigmaClip::default()).unwrap();
        assert!(s.sum_err.is_none() && s.weighted_mean.is_none());

        let point = RegionShape::Point { x: 2.0, y: 2.0 };
        let s = region_stats(&arr, &point, None, None, Some(&err), SigmaClip::default()).unwrap();
        assert_eq!(s.sum_err, Some(0.5));
        assert_eq!(s.weighted_mean, Some(s.mean));
    }

    #[test]
    fn region_stats_clips_outlier() {
        let mut arr = Array2::from_elem((8, 8), 10.0f32);
        for y in 0..8 {
            for x in 0..8 {
                arr[[y, x]] = 10.0 + ((x * 7 + y * 3) % 5) as f32 * 0.1;
            }
        }
        let clean_sum = arr.iter().map(|&v| v as f64).sum::<f64>() - arr[[4, 4]] as f64;
        let clean_mean = clean_sum / 63.0;
        arr[[4, 4]] = 1000.0;
        let region = RegionShape::Box { x: 3.5, y: 3.5, width: 8.0, height: 8.0, angle: 0.0 };
        let s = region_stats(&arr, &region, None, None, None, SigmaClip::default()).unwrap();
        assert_eq!(s.n_rejected, 1);
        assert!((s.clipped_mean - clean_mean).abs() < 1e-6, "{} vs {clean_mean}", s.clipped_mean);
        assert!(s.clipped_sigma < 1.0);
    }

    #[test]
    fn region_stats_with_background_annulus() {
        let mut arr = Array2::from_elem((32, 32), 5.0f32);
        for y in 0..32 {
            for x in 0..32 {
                arr[[y, x]] = 5.0 + ((x + y) % 3) as f32;
            }
        }
        let src = circle(16.0, 16.0, 3.0);
        let bg = RegionShape::Annulus { x: 16.0, y: 16.0, r_inner: 6.0, r_outer: 10.0 };
        let s = region_stats(&arr, &src, Some(&bg), None, None, SigmaClip::default()).unwrap();
        let b = s.background.clone().expect("background");
        let bg_mv = bg.masked_values(&arr, None);
        assert_eq!(b.count, bg_mv.values.len() as u64);
        let net = s.sum - b.median * s.count as f64;
        assert!((s.net_sum.unwrap() - net).abs() < 1e-9);
        assert!(b.sigma > 0.0);
        let snr = net / (b.sigma * (s.count as f64).sqrt());
        assert!((s.net_snr.unwrap() - snr).abs() < 1e-9);
    }

    #[test]
    fn region_stats_empty_and_empty_background_error() {
        let arr = known_4x4();
        let off = circle(100.0, 100.0, 2.0);
        assert_eq!(
            region_stats(&arr, &off, None, None, None, SigmaClip::default()),
            Err(RegionError::Empty { shape: "circle" })
        );
        let nan = Array2::from_elem((6, 6), f32::NAN);
        assert!(matches!(
            region_stats(&nan, &whole(), None, None, None, SigmaClip::default()),
            Err(RegionError::Empty { shape: "box" })
        ));
        let bg = RegionShape::Annulus { x: 100.0, y: 100.0, r_inner: 1.0, r_outer: 2.0 };
        assert_eq!(
            region_stats(&arr, &whole(), Some(&bg), None, None, SigmaClip::default()),
            Err(RegionError::Invalid("background region is empty".into()))
        );
        let s = region_stats(&arr, &RegionShape::Point { x: 1.0, y: 1.0 }, None, None, None, SigmaClip::default()).unwrap();
        assert_eq!(s.count, 1);
        assert_eq!(s.std, 0.0);
    }

    #[test]
    fn radial_profile_on_constant_image() {
        let arr = Array2::from_elem((32, 32), 7.0f32);
        let p = radial_profile(&arr, 16.0, 16.0, 5.0, None, None).unwrap();
        assert_eq!(p.bins.len(), 5);
        assert_eq!(p.bins[0].count, 1);
        assert_eq!(p.bins[1].count, 8);
        for b in &p.bins {
            assert_eq!(b.mean, Some(7.0));
            assert_eq!(b.median, Some(7.0));
        }
        assert!((p.bins[4].cumulative_sum - 7.0 * p.bins.iter().map(|b| b.count as f64).sum::<f64>()).abs() < 1e-9);
        let p = radial_profile(&arr, 16.0, 16.0, 5.0, Some((6.0, 9.0)), None).unwrap();
        let bg = p.background.expect("background");
        assert_eq!(bg.median, 7.0);
        for b in &p.bins {
            assert_eq!(b.mean, Some(0.0));
            assert_eq!(b.cumulative_sum, 0.0);
        }
    }

    #[test]
    fn radial_profile_gaussian_is_non_increasing_and_validates() {
        let mut arr = zeros(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let d2 = (x as f64 - 32.0).powi(2) + (y as f64 - 32.0).powi(2);
                arr[[y, x]] = (100.0 * (-d2 / 18.0).exp()) as f32;
            }
        }
        let p = radial_profile(&arr, 32.0, 32.0, 10.0, None, None).unwrap();
        for k in 1..5 {
            assert!(p.bins[k].mean.unwrap() <= p.bins[k - 1].mean.unwrap(), "bin {k}");
        }
        assert!(p.bins[0].std.is_some());
        assert!(radial_profile(&arr, 32.0, 32.0, 0.0, None, None).is_err());
        assert!(radial_profile(&arr, 32.0, 32.0, MAX_RADIAL_RADIUS + 1.0, None, None).is_err());
        assert!(radial_profile(&arr, 32.0, 32.0, f64::NAN, None, None).is_err());
        assert!(radial_profile(&arr, 32.0, 32.0, 3.0, Some((5.0, 4.0)), None).is_err());
        let far = radial_profile(&arr, 200.0, 200.0, 3.0, None, None).unwrap();
        assert!(far.bins.iter().all(|b| b.count == 0 && b.mean.is_none()));
    }

    #[test]
    fn line_cut_horizontal_and_bilinear_plane() {
        let mut arr = zeros(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                arr[[y, x]] = x as f32;
            }
        }
        let lc = line_cut(&arr, 0.0, 3.0, 5.0, 3.0, None).unwrap();
        assert_eq!(lc.n_samples, 6);
        assert_eq!(lc.values, (0..6).map(|i| Some(i as f64)).collect::<Vec<_>>());
        assert_eq!(lc.distance, (0..6).map(|i| i as f64).collect::<Vec<_>>());
        let mut plane = zeros(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                plane[[y, x]] = (2 * x + 3 * y) as f32;
            }
        }
        let lc = line_cut(&plane, 0.5, 0.25, 6.7, 5.9, None).unwrap();
        for (i, v) in lc.values.iter().enumerate() {
            let expected = 2.0 * lc.xs[i] + 3.0 * lc.ys[i];
            assert!((v.unwrap() - expected).abs() < 1e-5, "sample {i}");
        }
        let lc = line_cut(&plane, 7.0, 7.0, 7.0, 7.0, None).unwrap();
        assert_eq!(lc.n_samples, 1);
        assert_eq!(lc.values, vec![Some(35.0)]);
    }

    #[test]
    fn line_cut_off_image_and_excluded_become_none() {
        let arr = Array2::from_elem((8, 8), 1.0f32);
        let lc = line_cut(&arr, 4.0, 4.0, 10.0, 4.0, None).unwrap();
        assert_eq!(lc.n_samples, 7);
        assert_eq!(lc.values[0], Some(1.0));
        assert_eq!(lc.values[3], Some(1.0));
        assert!(lc.values[4..].iter().all(|v| v.is_none()));
        let mut ex = Array2::<u8>::zeros((8, 8));
        ex[[4, 2]] = 1;
        let lc = line_cut(&arr, 0.0, 4.0, 4.0, 4.0, Some(&ex)).unwrap();
        assert_eq!(lc.values, vec![Some(1.0), None, None, Some(1.0), Some(1.0)]);
        let lc = line_cut(&arr, 0.0, 4.0, 4.0, 4.0, None).unwrap();
        assert!(lc.values.iter().all(|v| *v == Some(1.0)));
        assert!(line_cut(&arr, f64::NAN, 0.0, 1.0, 1.0, None).is_err());
    }

    #[test]
    fn system_parse_and_names() {
        assert_eq!(RegionSystem::parse("image").unwrap(), RegionSystem::Image);
        assert_eq!(RegionSystem::parse("PHYSICAL").unwrap(), RegionSystem::Image);
        assert_eq!(RegionSystem::parse("fk5").unwrap(), RegionSystem::Fk5);
        assert_eq!(RegionSystem::parse(" Icrs ").unwrap(), RegionSystem::Icrs);
        assert_eq!(RegionSystem::parse("galactic"), Err(RegionError::UnsupportedSystem("galactic".into())));
        assert!(RegionSystem::Fk5.is_sky() && !RegionSystem::Image.is_sky());
        assert_eq!(RegionSystem::Icrs.name(), "icrs");
        assert_eq!(serde_json::to_string(&RegionSystem::Fk5).unwrap(), "\"fk5\"");
        assert_eq!(RegionSystem::default(), RegionSystem::Image);
    }

    #[test]
    fn north_up_header_maps_crval_to_crpix() {
        let wcs = WcsTransform::from_header(&header_with_cd(north_up_cd())).unwrap();
        assert!(wcs_north_angle_deg(&wcs).abs() < 1e-12);
        let sky = RegionShape::Circle { x: 150.0, y: 2.0, r: 3.5 };
        let px = shape_to_pixel(&sky, RegionSystem::Icrs, Some(&wcs)).unwrap();
        match px {
            RegionShape::Circle { x, y, r } => {
                assert!((x - 49.5).abs() < 1e-6, "x {x}");
                assert!((y - 49.5).abs() < 1e-6, "y {y}");
                assert!((r - 3.5).abs() < 1e-9, "r {r}");
            }
            other => panic!("{other:?}"),
        }
        let sky_box = RegionShape::Box { x: 150.0, y: 2.0, width: 10.0, height: 5.0, angle: 20.0 };
        match shape_to_pixel(&sky_box, RegionSystem::Icrs, Some(&wcs)).unwrap() {
            RegionShape::Box { width, height, angle, .. } => {
                assert!((width - 10.0).abs() < 1e-9 && (height - 5.0).abs() < 1e-9);
                assert!((angle - 20.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(shape_to_pixel(&sky, RegionSystem::Image, None).unwrap(), sky);
    }

    fn assert_shape_close(a: &RegionShape, b: &RegionShape, pos_tol: f64, ang_tol: f64) {
        match (a, b) {
            (
                RegionShape::Box { x, y, width, height, angle },
                RegionShape::Box { x: x2, y: y2, width: w2, height: h2, angle: a2 },
            ) => {
                assert!((x - x2).abs() < pos_tol && (y - y2).abs() < pos_tol, "{a:?} vs {b:?}");
                assert!((width - w2).abs() < pos_tol && (height - h2).abs() < pos_tol, "{a:?} vs {b:?}");
                assert!((angle - a2).abs() < ang_tol, "{a:?} vs {b:?}");
            }
            (
                RegionShape::Ellipse { x, y, rx, ry, angle },
                RegionShape::Ellipse { x: x2, y: y2, rx: rx2, ry: ry2, angle: a2 },
            ) => {
                assert!((x - x2).abs() < pos_tol && (y - y2).abs() < pos_tol, "{a:?} vs {b:?}");
                assert!((rx - rx2).abs() < pos_tol && (ry - ry2).abs() < pos_tol, "{a:?} vs {b:?}");
                assert!((angle - a2).abs() < ang_tol, "{a:?} vs {b:?}");
            }
            (RegionShape::Polygon { points }, RegionShape::Polygon { points: p2 }) => {
                assert_eq!(points.len(), p2.len());
                for (p, q) in points.iter().zip(p2) {
                    assert!((p[0] - q[0]).abs() < pos_tol && (p[1] - q[1]).abs() < pos_tol, "{a:?} vs {b:?}");
                }
            }
            _ => panic!("shape kinds differ: {a:?} vs {b:?}"),
        }
    }

    #[test]
    fn rotated_header_round_trips_through_fk5_and_icrs() {
        let wcs = WcsTransform::from_header(&header_with_cd(rotated_cd(30.0))).unwrap();
        let north = wcs_north_angle_deg(&wcs);
        assert!((north.abs() - 30.0).abs() < 1e-9, "north {north}");
        let shapes = vec![
            RegionShape::Box { x: 40.0, y: 60.0, width: 12.0, height: 6.0, angle: 15.0 },
            RegionShape::Ellipse { x: 55.25, y: 45.75, rx: 8.0, ry: 3.0, angle: 100.0 },
            RegionShape::Polygon { points: vec![[10.0, 10.0], [80.0, 12.0], [40.0, 70.0]] },
        ];
        for s in &shapes {
            for system in [RegionSystem::Fk5, RegionSystem::Icrs] {
                let sky = shape_to_sky(s, system, Some(&wcs)).unwrap();
                let back = shape_to_pixel(&sky, system, Some(&wcs)).unwrap();
                assert_shape_close(s, &back, 1e-6, 1e-9);
            }
        }
        let fk5 = shape_to_sky(&shapes[0], RegionSystem::Fk5, Some(&wcs)).unwrap();
        let icrs = shape_to_sky(&shapes[0], RegionSystem::Icrs, Some(&wcs)).unwrap();
        if let (RegionShape::Box { x: xa, y: ya, .. }, RegionShape::Box { x: xb, y: yb, .. }) = (&fk5, &icrs) {
            let sep = crate::core::astrometry::wcs::angular_separation(*xa, *ya, *xb, *yb) * 3600.0;
            assert!(sep > 0.0 && sep < 0.1, "fk5/icrs separation {sep} arcsec");
        } else {
            panic!("expected boxes");
        }
    }

    #[test]
    fn sky_conversion_errors() {
        let sky = RegionShape::Circle { x: 150.0, y: 2.0, r: 3.5 };
        assert_eq!(shape_to_pixel(&sky, RegionSystem::Fk5, None), Err(RegionError::WcsRequired));
        assert_eq!(shape_to_sky(&sky, RegionSystem::Icrs, None), Err(RegionError::WcsRequired));
        let wcs = WcsTransform::from_header(&header_with_cd(north_up_cd())).unwrap();
        let far = RegionShape::Circle { x: 330.0, y: -2.0, r: 3.5 };
        assert_eq!(shape_to_pixel(&far, RegionSystem::Icrs, Some(&wcs)), Err(RegionError::OffImage));
    }
}
