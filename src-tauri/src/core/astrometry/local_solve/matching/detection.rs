use std::borrow::Cow;
use std::collections::HashMap;

use ndarray::{s, Array2, Axis};
use rayon::prelude::*;

use crate::core::alignment::triangles::select_alignment_stars;
use crate::core::analysis::star_detection::{detect_stars, DetectedStar};
use crate::core::astrometry::solve_hints::SolveHints;
use crate::core::imaging::cfa::luminance_bin;
use crate::math::sigma_clipped_stats;

use crate::core::astrometry::local_solve::constants::{
    EDGE_MARGIN_PX, EDGE_MARGIN_FWHM, SATURATION_FRACTION, SATURATION_PLATEAU_PIXELS,
    SIP_RADIAL_BANDS, BACKGROUND_TILE_MIN_PIXELS, BACKGROUND_CLIP_SIGMA, BACKGROUND_CLIP_ROUNDS,
};
use crate::core::astrometry::local_solve::types::{Detection, LocalSolveParams};

#[derive(Clone, Copy)]
struct PixelMap {
    scale: f64,
    offset: f64,
}

impl PixelMap {
    fn identity() -> Self {
        Self {
            scale: 1.0,
            offset: 0.0,
        }
    }

    fn then_binned(self, factor: usize, origin: usize) -> Self {
        let factor = factor as f64;
        Self {
            scale: self.scale * factor,
            offset: self.offset + self.scale * (origin as f64 + (factor - 1.0) / 2.0),
        }
    }

    fn apply(&self, value: f64) -> f64 {
        self.scale * value + self.offset
    }
}

pub(in crate::core::astrometry::local_solve) fn block_mean(image: &Array2<f32>, factor: usize) -> Array2<f32> {
    let (rows, cols) = image.dim();
    let (out_rows, out_cols) = (rows / factor, cols / factor);
    let mut out = Array2::<f32>::zeros((out_rows, out_cols));
    out.axis_iter_mut(Axis(0))
        .into_par_iter()
        .enumerate()
        .for_each(|(oy, mut row)| {
            for (ox, value) in row.iter_mut().enumerate() {
                let block = image.slice(s![
                    oy * factor..(oy + 1) * factor,
                    ox * factor..(ox + 1) * factor
                ]);
                let (sum, count) = block
                    .iter()
                    .filter(|v| v.is_finite())
                    .fold((0.0f64, 0usize), |(sum, count), &v| {
                        (sum + f64::from(v), count + 1)
                    });
                *value = if count > 0 {
                    (sum / count as f64) as f32
                } else {
                    f32::NAN
                };
            }
        });
    out
}

struct BackgroundMap {
    tile: usize,
    medians: Array2<f32>,
    global: f32,
}

impl BackgroundMap {
    fn estimate(image: &Array2<f32>, tile: usize) -> Option<Self> {
        let tile = tile.max(1);
        let (rows, cols) = image.dim();
        let (tile_rows, tile_cols) = (rows.div_ceil(tile), cols.div_ceil(tile));
        if tile_rows == 0 || tile_cols == 0 {
            return None;
        }
        let cells: Vec<f32> = (0..tile_rows * tile_cols)
            .into_par_iter()
            .map(|index| {
                let (ty, tx) = (index / tile_cols, index % tile_cols);
                let block = image.slice(s![
                    ty * tile..((ty + 1) * tile).min(rows),
                    tx * tile..((tx + 1) * tile).min(cols)
                ]);
                let mut values: Vec<f32> = block.iter().copied().filter(|v| v.is_finite()).collect();
                if values.len() < BACKGROUND_TILE_MIN_PIXELS {
                    return f32::NAN;
                }
                sigma_clipped_stats(&mut values, BACKGROUND_CLIP_SIGMA, BACKGROUND_CLIP_ROUNDS).0 as f32
            })
            .collect();
        let medians = Array2::from_shape_vec((tile_rows, tile_cols), cells).ok()?;
        let mut finite: Vec<f32> = medians.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.is_empty() {
            return None;
        }
        let middle = finite.len() / 2;
        finite.select_nth_unstable_by(middle, f32::total_cmp);
        Some(Self {
            tile,
            medians,
            global: finite[middle],
        })
    }

    fn cell(&self, ty: isize, tx: isize) -> f32 {
        let (tile_rows, tile_cols) = self.medians.dim();
        let value = self.medians[[
            ty.clamp(0, tile_rows as isize - 1) as usize,
            tx.clamp(0, tile_cols as isize - 1) as usize,
        ]];
        if value.is_finite() {
            value
        } else {
            self.global
        }
    }

    fn at(&self, x: f64, y: f64) -> f32 {
        let fx = (x + 0.5) / self.tile as f64 - 0.5;
        let fy = (y + 0.5) / self.tile as f64 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (wx, wy) = ((fx - x0) as f32, (fy - y0) as f32);
        let (tx, ty) = (x0 as isize, y0 as isize);
        self.cell(ty, tx) * (1.0 - wy) * (1.0 - wx)
            + self.cell(ty, tx + 1) * (1.0 - wy) * wx
            + self.cell(ty + 1, tx) * wy * (1.0 - wx)
            + self.cell(ty + 1, tx + 1) * wy * wx
    }

    fn excess_at(&self, x: f64, y: f64) -> f64 {
        f64::from(self.at(x, y) - self.global)
    }

    fn flattened(&self, image: &Array2<f32>, keep_from: Option<f32>) -> Array2<f32> {
        let mut out = image.clone();
        out.axis_iter_mut(Axis(0))
            .into_par_iter()
            .enumerate()
            .for_each(|(row, mut line)| {
                for (col, value) in line.iter_mut().enumerate() {
                    if keep_from.is_some_and(|plateau| *value >= plateau) {
                        continue;
                    }
                    *value = *value - self.at(col as f64, row as f64) + self.global;
                }
            });
        out
    }
}

pub(in crate::core::astrometry::local_solve) fn saturation_level(image: &Array2<f32>) -> Option<f64> {
    let max = image
        .iter()
        .filter(|v| v.is_finite())
        .fold(f32::NEG_INFINITY, |m, &v| m.max(v));
    if !max.is_finite() {
        return None;
    }
    let floor = max * SATURATION_FRACTION as f32;
    let plateau = image.iter().filter(|&&v| v >= floor).count();
    (plateau >= SATURATION_PLATEAU_PIXELS).then_some(f64::from(max))
}

pub fn detect_for_solve(
    image: &Array2<f32>,
    hints: &SolveHints,
    params: &LocalSolveParams,
) -> Vec<Detection> {
    let mut working: Cow<Array2<f32>> = Cow::Borrowed(image);
    let mut map_x = PixelMap::identity();
    let mut map_y = PixelMap::identity();
    if let Some(cfa) = &hints.cfa {
        let (origin_y, origin_x) = cfa.aligned_origin();
        let cell = cfa.bin_factor();
        working = Cow::Owned(luminance_bin(&working, cfa));
        map_x = map_x.then_binned(cell, origin_x);
        map_y = map_y.then_binned(cell, origin_y);
    }
    let (rows, cols) = working.dim();
    let factor = rows.max(cols).div_ceil(params.max_side_px.max(1));
    if factor > 1 {
        working = Cow::Owned(block_mean(&working, factor));
        map_x = map_x.then_binned(factor, 0);
        map_y = map_y.then_binned(factor, 0);
    }
    let (rows, cols) = working.dim();
    let saturation = saturation_level(&working);
    let background_map = BackgroundMap::estimate(&working, params.background_tile_px);
    if let Some(map) = &background_map {
        let plateau = saturation.map(|level| (SATURATION_FRACTION * level) as f32);
        let flattened = map.flattened(&working, plateau);
        working = Cow::Owned(flattened);
    }
    let detected = detect_stars(&working, params.detect_sigma);
    let background = detected.background_median;
    detected
        .stars
        .into_iter()
        .filter(|star| {
            let margin = EDGE_MARGIN_PX.max(EDGE_MARGIN_FWHM * star.fwhm);
            star.x >= margin
                && star.y >= margin
                && star.x <= cols as f64 - 1.0 - margin
                && star.y <= rows as f64 - 1.0 - margin
        })
        .map(|star| {
            let local_excess = background_map
                .as_ref()
                .map_or(0.0, |map| map.excess_at(star.x, star.y));
            let raw_peak = star.peak + background + local_excess;
            let saturated = saturation.is_some_and(|level| raw_peak >= SATURATION_FRACTION * level);
            Detection {
                star: DetectedStar {
                    x: map_x.apply(star.x),
                    y: map_y.apply(star.y),
                    fwhm: star.fwhm * map_x.scale,
                    ..star
                },
                saturated,
            }
        })
        .collect()
}

pub(in crate::core::astrometry::local_solve) fn compactness_limit(detections: &[Detection], params: &LocalSolveParams) -> f64 {
    let mut brightest: Vec<&Detection> = detections
        .iter()
        .filter(|d| d.star.x.is_finite() && d.star.y.is_finite())
        .filter(|d| d.star.eccentricity.is_finite())
        .collect();
    brightest.sort_by(|a, b| b.star.flux.total_cmp(&a.star.flux));
    let mut eccentricities: Vec<f64> = brightest
        .iter()
        .take(params.detection_stars.max(1))
        .map(|d| d.star.eccentricity)
        .collect();
    if eccentricities.is_empty() {
        return params.max_eccentricity;
    }
    eccentricities.sort_by(f64::total_cmp);
    let rank = ((eccentricities.len() - 1) as f64 * params.compact_quantile.clamp(0.0, 1.0))
        .round() as usize;
    params.max_eccentricity.max(eccentricities[rank])
}

pub(in crate::core::astrometry::local_solve) fn is_compact(detection: &Detection, limit: f64) -> bool {
    detection.star.eccentricity <= limit
}

pub(in crate::core::astrometry::local_solve) fn usable_detections(detections: &[Detection], params: &LocalSolveParams) -> Vec<Detection> {
    let limit = compactness_limit(detections, params);
    let mut usable: Vec<Detection> = detections
        .iter()
        .filter(|d| is_compact(d, limit))
        .filter(|d| d.star.x.is_finite() && d.star.y.is_finite())
        .cloned()
        .collect();
    usable.sort_by(|a, b| b.star.flux.total_cmp(&a.star.flux));
    usable
}

pub(in crate::core::astrometry::local_solve) fn dropped_saturated(detections: &[Detection], params: &LocalSolveParams) -> usize {
    let limit = compactness_limit(detections, params);
    detections
        .iter()
        .filter(|d| d.saturated && !is_compact(d, limit))
        .count()
}

pub(in crate::core::astrometry::local_solve) fn disc_points(
    usable: &[Detection],
    (width, height): (usize, usize),
    radius_px: f64,
) -> Vec<(f64, f64)> {
    let (xc, yc) = ((width as f64 - 1.0) / 2.0, (height as f64 - 1.0) / 2.0);
    usable
        .iter()
        .map(|d| (d.star.x, d.star.y))
        .filter(|&(x, y)| (x - xc).hypot(y - yc) <= radius_px)
        .collect()
}

pub(in crate::core::astrometry::local_solve) fn spread_detections(
    usable: &[Detection],
    (width, height): (usize, usize),
    limit: usize,
) -> Vec<Detection> {
    let stars: Vec<DetectedStar> = usable.iter().map(|d| d.star.clone()).collect();
    let chosen = select_alignment_stars(&stars, limit, height, width);
    let index: HashMap<(u64, u64), usize> = usable
        .iter()
        .enumerate()
        .map(|(i, d)| ((d.star.x.to_bits(), d.star.y.to_bits()), i))
        .collect();
    chosen
        .iter()
        .filter_map(|&(x, y)| index.get(&(x.to_bits(), y.to_bits())))
        .map(|&i| usable[i].clone())
        .collect()
}

pub(in crate::core::astrometry::local_solve) fn radial_spread(
    usable: &[Detection],
    (width, height): (usize, usize),
    per_band: usize,
) -> Vec<Detection> {
    let (xc, yc) = ((width as f64 - 1.0) / 2.0, (height as f64 - 1.0) / 2.0);
    let half_diagonal = (width as f64 / 2.0).hypot(height as f64 / 2.0);
    let mut counts = [0usize; SIP_RADIAL_BANDS];
    usable
        .iter()
        .filter(|d| {
            let r2 = ((d.star.x - xc).powi(2) + (d.star.y - yc).powi(2)) / (half_diagonal * half_diagonal);
            let band = ((r2 * SIP_RADIAL_BANDS as f64).floor() as usize).min(SIP_RADIAL_BANDS - 1);
            if counts[band] < per_band {
                counts[band] += 1;
                true
            } else {
                false
            }
        })
        .cloned()
        .collect()
}
