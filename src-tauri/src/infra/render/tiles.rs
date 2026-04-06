use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::math::simd::find_minmax_simd;

#[derive(Debug, Clone)]
pub struct TileParams {
    pub tile_size: usize,
}

impl Default for TileParams {
    fn default() -> Self {
        Self { tile_size: 256 }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TileLevel {
    pub level: usize,
    pub width: usize,
    pub height: usize,
    pub cols: usize,
    pub rows: usize,
    pub scale_factor: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TilePyramid {
    pub tile_size: usize,
    pub original_width: usize,
    pub original_height: usize,
    pub levels: Vec<TileLevel>,
    pub base_dir: String,
}

fn downsample(data: &Array2<f32>, factor: usize) -> Array2<f32> {
    if factor <= 1 {
        return data.clone();
    }

    let (rows, cols) = data.dim();
    let new_rows = (rows + factor - 1) / factor;
    let new_cols = (cols + factor - 1) / factor;
    let src = data.as_slice().expect("contiguous");

    let pixels: Vec<f32> = (0..new_rows)
        .into_par_iter()
        .flat_map_iter(move |ny| {
            let y_start = ny * factor;
            let y_end = (y_start + factor).min(rows);
            (0..new_cols).map(move |nx| {
                let x_start = nx * factor;
                let x_end = (x_start + factor).min(cols);
                let mut sum = 0.0f64;
                let mut count = 0u32;
                for y in y_start..y_end {
                    let row_off = y * cols;
                    for x in x_start..x_end {
                        let v = src[row_off + x];
                        if v.is_finite() {
                            sum += v as f64;
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    (sum / count as f64) as f32
                } else {
                    0.0
                }
            })
        })
        .collect();

    Array2::from_shape_vec((new_rows, new_cols), pixels).unwrap()
}

fn render_tile(
    data: &Array2<f32>,
    tile_x: usize,
    tile_y: usize,
    tile_size: usize,
    global_min: f32,
    global_max: f32,
    output_path: &str,
) -> Result<()> {
    let (rows, cols) = data.dim();
    let src = data.as_slice().expect("contiguous");

    let x_start = tile_x * tile_size;
    let y_start = tile_y * tile_size;
    let x_end = (x_start + tile_size).min(cols);
    let y_end = (y_start + tile_size).min(rows);

    let tile_w = x_end - x_start;
    let tile_h = y_end - y_start;

    if tile_w == 0 || tile_h == 0 {
        return Ok(());
    }

    let range = (global_max - global_min).max(1e-10);
    let inv_range = 255.0 / range;

    let mut buf = vec![0u8; tile_size * tile_size];

    for dy in 0..tile_h {
        let src_row = (y_start + dy) * cols;
        let dst_row = dy * tile_size;
        for dx in 0..tile_w {
            let v = src[src_row + x_start + dx];
            buf[dst_row + dx] = if v.is_finite() {
                ((v - global_min) * inv_range).clamp(0.0, 255.0) as u8
            } else {
                0
            };
        }
    }

    if let Some(parent) = Path::new(output_path).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create tile dir {:?}", parent))?;
    }

    let file = fs::File::create(output_path)
        .with_context(|| format!("Failed to create tile {}", output_path))?;
    let bw = std::io::BufWriter::new(file);
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        bw,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );
    use image::ImageEncoder;
    encoder.write_image(&buf, tile_size as u32, tile_size as u32, image::ExtendedColorType::L8)
        .with_context(|| format!("Failed to encode tile {}", output_path))?;
    Ok(())
}

fn compute_num_levels(width: usize, height: usize, tile_size: usize) -> usize {
    let max_dim = width.max(height) as f64;
    let ts = tile_size as f64;

    if max_dim <= ts {
        return 1;
    }

    let levels = (max_dim / ts).log2().ceil() as usize + 1;
    levels.max(1)
}

fn percentile_bounds(slice: &[f32], low_pct: f64, high_pct: f64) -> (f32, f32) {
    let mut valid: Vec<f32> = slice
        .par_iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 1e-7)
        .collect();

    if valid.is_empty() {
        let (gmin, gmax) = find_minmax_simd(slice);
        return (gmin, gmax);
    }

    let n = valid.len();
    let lo_idx = ((n as f64 * low_pct) as usize).min(n - 1);
    let hi_idx = ((n as f64 * high_pct) as usize).min(n - 1);

    let (_, lo_val, _) = valid.select_nth_unstable_by(lo_idx, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    let lo = *lo_val;

    let (_, hi_val, _) = valid.select_nth_unstable_by(hi_idx, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    let hi = *hi_val;

    (lo, hi)
}

pub fn generate_tile_pyramid(
    normalized: &Array2<f32>,
    output_dir: &str,
    params: &TileParams,
) -> Result<TilePyramid> {
    let (orig_rows, orig_cols) = normalized.dim();
    let tile_size = params.tile_size;

    let num_levels = compute_num_levels(orig_cols, orig_rows, tile_size);

    let slice = normalized.as_slice().expect("Array2 must be contiguous");
    let (global_min, global_max) = percentile_bounds(slice, 0.001, 0.999);

    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create tile output dir {}", output_dir))?;

    let mut levels = Vec::with_capacity(num_levels);
    let max_level = num_levels - 1;

    for level in 0..num_levels {
        let reduction_power = max_level - level;
        let factor = 1usize << reduction_power;

        let level_data: std::borrow::Cow<Array2<f32>> = if factor > 1 {
            std::borrow::Cow::Owned(downsample(normalized, factor))
        } else {
            std::borrow::Cow::Borrowed(normalized)
        };

        let (level_rows, level_cols) = level_data.dim();
        let tile_cols = (level_cols + tile_size - 1) / tile_size;
        let tile_rows = (level_rows + tile_size - 1) / tile_size;

        let scale_factor = 1.0 / factor as f64;

        let level_dir = format!("{}/{}", output_dir, level);
        fs::create_dir_all(&level_dir)
            .with_context(|| format!("Failed to create level dir {}", level_dir))?;

        let tile_coords: Vec<(usize, usize)> = (0..tile_rows)
            .flat_map(|ty| (0..tile_cols).map(move |tx| (tx, ty)))
            .collect();

        tile_coords.par_iter().try_for_each(|&(tx, ty)| -> Result<()> {
            let tile_path = format!("{}/{}_{}.png", level_dir, tx, ty);
            render_tile(
                &*level_data,
                tx,
                ty,
                tile_size,
                global_min,
                global_max,
                &tile_path,
            )
        })?;

        levels.push(TileLevel {
            level,
            width: level_cols,
            height: level_rows,
            cols: tile_cols,
            rows: tile_rows,
            scale_factor,
        });
    }

    Ok(TilePyramid {
        tile_size,
        original_width: orig_cols,
        original_height: orig_rows,
        levels,
        base_dir: output_dir.to_string(),
    })
}

fn render_tile_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    tile_x: usize,
    tile_y: usize,
    tile_size: usize,
    output_path: &str,
) -> Result<()> {
    let (rows, cols) = r.dim();
    let r_src = r.as_slice().expect("contiguous");
    let g_src = g.as_slice().expect("contiguous");
    let b_src = b.as_slice().expect("contiguous");

    let x_start = tile_x * tile_size;
    let y_start = tile_y * tile_size;
    let x_end = (x_start + tile_size).min(cols);
    let y_end = (y_start + tile_size).min(rows);

    let tile_w = x_end - x_start;
    let tile_h = y_end - y_start;

    if tile_w == 0 || tile_h == 0 {
        return Ok(());
    }

    let mut buf = vec![0u8; tile_size * tile_size * 3];

    for dy in 0..tile_h {
        let src_row = (y_start + dy) * cols;
        let dst_row = dy * tile_size;
        for dx in 0..tile_w {
            let si = src_row + x_start + dx;
            let di = (dst_row + dx) * 3;
            buf[di] = (r_src[si].clamp(0.0, 1.0) * 255.0) as u8;
            buf[di + 1] = (g_src[si].clamp(0.0, 1.0) * 255.0) as u8;
            buf[di + 2] = (b_src[si].clamp(0.0, 1.0) * 255.0) as u8;
        }
    }

    save_tile_rgb(&buf, tile_size, output_path)
}

fn render_tile_rgb_stf(
    r: &[f32],
    g: &[f32],
    b: &[f32],
    cols: usize,
    tile_x: usize,
    tile_y: usize,
    tile_size: usize,
    rows: usize,
    fn_r: &(dyn Fn(f32) -> u8 + Send + Sync),
    fn_g: &(dyn Fn(f32) -> u8 + Send + Sync),
    fn_b: &(dyn Fn(f32) -> u8 + Send + Sync),
    output_path: &str,
) -> Result<()> {
    let x_start = tile_x * tile_size;
    let y_start = tile_y * tile_size;
    let x_end = (x_start + tile_size).min(cols);
    let y_end = (y_start + tile_size).min(rows);

    let tile_w = x_end - x_start;
    let tile_h = y_end - y_start;

    if tile_w == 0 || tile_h == 0 {
        return Ok(());
    }

    let mut buf = vec![0u8; tile_size * tile_size * 3];

    for dy in 0..tile_h {
        let src_row = (y_start + dy) * cols;
        let dst_row = dy * tile_size;
        for dx in 0..tile_w {
            let si = src_row + x_start + dx;
            let di = (dst_row + dx) * 3;
            buf[di] = fn_r(r[si]);
            buf[di + 1] = fn_g(g[si]);
            buf[di + 2] = fn_b(b[si]);
        }
    }

    save_tile_rgb(&buf, tile_size, output_path)
}

fn save_tile_rgb(buf: &[u8], tile_size: usize, output_path: &str) -> Result<()> {
    if let Some(parent) = Path::new(output_path).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create tile dir {:?}", parent))?;
    }

    let file = fs::File::create(output_path)
        .with_context(|| format!("Failed to create tile {}", output_path))?;
    let bw = std::io::BufWriter::new(file);
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        bw,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );
    use image::ImageEncoder;
    encoder.write_image(buf, tile_size as u32, tile_size as u32, image::ExtendedColorType::Rgb8)
        .with_context(|| format!("Failed to encode tile {}", output_path))?;
    Ok(())
}

pub fn generate_tile_pyramid_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    output_dir: &str,
    params: &TileParams,
) -> Result<TilePyramid> {
    generate_tile_pyramid_rgb_inner(r, g, b, output_dir, params, None, None, None)
}

pub fn generate_tile_pyramid_rgb_stf(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    output_dir: &str,
    params: &TileParams,
    fn_r: impl Fn(f32) -> u8 + Send + Sync,
    fn_g: impl Fn(f32) -> u8 + Send + Sync,
    fn_b: impl Fn(f32) -> u8 + Send + Sync,
) -> Result<TilePyramid> {
    generate_tile_pyramid_rgb_inner(r, g, b, output_dir, params, Some(&fn_r), Some(&fn_g), Some(&fn_b))
}

fn generate_tile_pyramid_rgb_inner(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    output_dir: &str,
    params: &TileParams,
    fn_r: Option<&(dyn Fn(f32) -> u8 + Send + Sync)>,
    fn_g: Option<&(dyn Fn(f32) -> u8 + Send + Sync)>,
    fn_b: Option<&(dyn Fn(f32) -> u8 + Send + Sync)>,
) -> Result<TilePyramid> {
    let (orig_rows, orig_cols) = r.dim();
    let tile_size = params.tile_size;
    let num_levels = compute_num_levels(orig_cols, orig_rows, tile_size);

    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create tile output dir {}", output_dir))?;

    let mut levels = Vec::with_capacity(num_levels);
    let max_level = num_levels - 1;

    for level in 0..num_levels {
        let reduction_power = max_level - level;
        let factor = 1usize << reduction_power;

        let (lr, lg, lb): (std::borrow::Cow<Array2<f32>>, std::borrow::Cow<Array2<f32>>, std::borrow::Cow<Array2<f32>>) = if factor > 1 {
            (std::borrow::Cow::Owned(downsample(r, factor)),
             std::borrow::Cow::Owned(downsample(g, factor)),
             std::borrow::Cow::Owned(downsample(b, factor)))
        } else {
            (std::borrow::Cow::Borrowed(r),
             std::borrow::Cow::Borrowed(g),
             std::borrow::Cow::Borrowed(b))
        };

        let (level_rows, level_cols) = lr.dim();
        let tile_cols = (level_cols + tile_size - 1) / tile_size;
        let tile_rows = (level_rows + tile_size - 1) / tile_size;
        let scale_factor = 1.0 / factor as f64;

        let level_dir = format!("{}/{}", output_dir, level);
        fs::create_dir_all(&level_dir)
            .with_context(|| format!("Failed to create level dir {}", level_dir))?;

        let tile_coords: Vec<(usize, usize)> = (0..tile_rows)
            .flat_map(|ty| (0..tile_cols).map(move |tx| (tx, ty)))
            .collect();

        if let (Some(fr), Some(fg), Some(fb)) = (fn_r, fn_g, fn_b) {
            let r_sl = lr.as_slice().expect("contiguous");
            let g_sl = lg.as_slice().expect("contiguous");
            let b_sl = lb.as_slice().expect("contiguous");
            tile_coords.par_iter().try_for_each(|&(tx, ty)| -> Result<()> {
                let tile_path = format!("{}/{}_{}.png", level_dir, tx, ty);
                render_tile_rgb_stf(
                    r_sl, g_sl, b_sl, level_cols,
                    tx, ty, tile_size, level_rows,
                    fr, fg, fb,
                    &tile_path,
                )
            })?;
        } else {
            tile_coords.par_iter().try_for_each(|&(tx, ty)| -> Result<()> {
                let tile_path = format!("{}/{}_{}.png", level_dir, tx, ty);
                render_tile_rgb(&*lr, &*lg, &*lb, tx, ty, tile_size, &tile_path)
            })?;
        }

        levels.push(TileLevel {
            level,
            width: level_cols,
            height: level_rows,
            cols: tile_cols,
            rows: tile_rows,
            scale_factor,
        });
    }

    Ok(TilePyramid {
        tile_size,
        original_width: orig_cols,
        original_height: orig_rows,
        levels,
        base_dir: output_dir.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_num_levels() {
        assert_eq!(compute_num_levels(256, 256, 256), 1);
        assert_eq!(compute_num_levels(512, 512, 256), 2);
        assert_eq!(compute_num_levels(1024, 1024, 256), 3);
        let levels = compute_num_levels(14000, 14000, 256);
        assert!(levels >= 6 && levels <= 8);
    }

    #[test]
    fn test_downsample_identity() {
        let data = Array2::from_shape_vec((4, 4), vec![1.0; 16]).unwrap();
        let result = downsample(&data, 1);
        assert_eq!(result.dim(), (4, 4));
    }

    #[test]
    fn test_downsample_2x() {
        let data = Array2::from_shape_vec(
            (4, 4),
            vec![
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0,
                15.0, 16.0,
            ],
        )
            .unwrap();
        let result = downsample(&data, 2);
        assert_eq!(result.dim(), (2, 2));
        assert!((result[[0, 0]] - 3.5).abs() < 1e-4);
        assert!((result[[1, 1]] - 13.5).abs() < 1e-4);
    }

    #[test]
    fn test_downsample_non_divisible() {
        let data = Array2::<f32>::ones((5, 5));
        let result = downsample(&data, 2);
        assert_eq!(result.dim(), (3, 3));
    }

    #[test]
    fn test_generate_tile_pyramid() {
        let data = Array2::from_shape_vec(
            (512, 512),
            (0..512 * 512).map(|i| (i as f32) / (512.0 * 512.0)).collect(),
        )
            .unwrap();

        let dir = "/tmp/test_tiles_pyramid";
        let _ = fs::remove_dir_all(dir);

        let params = TileParams { tile_size: 256 };
        let pyramid = generate_tile_pyramid(&data, dir, &params).unwrap();

        assert_eq!(pyramid.original_width, 512);
        assert_eq!(pyramid.original_height, 512);
        assert_eq!(pyramid.levels.len(), 2);
        assert_eq!(pyramid.levels[0].cols, 1);
        assert_eq!(pyramid.levels[0].rows, 1);
        assert_eq!(pyramid.levels[1].cols, 2);
        assert_eq!(pyramid.levels[1].rows, 2);

        assert!(Path::new(&format!("{}/0/0_0.png", dir)).exists());
        assert!(Path::new(&format!("{}/1/0_0.png", dir)).exists());
        assert!(Path::new(&format!("{}/1/1_1.png", dir)).exists());

        let _ = fs::remove_dir_all(dir);
    }
}
