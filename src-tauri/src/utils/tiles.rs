use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use image::{GrayImage, Luma};
use ndarray::Array2;

use crate::utils::simd::find_minmax_simd;

#[derive(Debug, Clone)]
pub struct TileParams {
    
    pub tile_size: usize,
}

impl Default for TileParams {
    fn default() -> Self {
        Self { tile_size: 256 }
    }
}


#[derive(Debug, Clone)]
pub struct TileLevel {
    
    pub level: usize,
    
    pub width: usize,
    
    pub height: usize,
    
    pub cols: usize,
    
    pub rows: usize,
    
    pub scale_factor: f64,
}


#[derive(Debug, Clone)]
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

    let mut result = Array2::<f32>::zeros((new_rows, new_cols));

    for ny in 0..new_rows {
        for nx in 0..new_cols {
            let y_start = ny * factor;
            let x_start = nx * factor;
            let y_end = (y_start + factor).min(rows);
            let x_end = (x_start + factor).min(cols);

            let mut sum = 0.0f64;
            let mut count = 0u32;

            for y in y_start..y_end {
                for x in x_start..x_end {
                    let v = data[[y, x]];
                    if v.is_finite() {
                        sum += v as f64;
                        count += 1;
                    }
                }
            }

            result[[ny, nx]] = if count > 0 {
                (sum / count as f64) as f32
            } else {
                0.0
            };
        }
    }

    result
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

    
    
    let mut img = GrayImage::new(tile_size as u32, tile_size as u32);

    for dy in 0..tile_h {
        for dx in 0..tile_w {
            let v = data[[y_start + dy, x_start + dx]];
            let byte = if v.is_finite() {
                ((v - global_min) * inv_range).clamp(0.0, 255.0) as u8
            } else {
                0
            };
            img.put_pixel(dx as u32, dy as u32, Luma([byte]));
        }
    }

    
    if let Some(parent) = Path::new(output_path).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create tile dir {:?}", parent))?;
    }

    img.save(output_path)
        .with_context(|| format!("Failed to save tile {}", output_path))?;
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

pub fn generate_tile_pyramid(
    normalized: &Array2<f32>,
    output_dir: &str,
    params: &TileParams,
) -> Result<TilePyramid> {
    let (orig_rows, orig_cols) = normalized.dim();
    let tile_size = params.tile_size;

    let num_levels = compute_num_levels(orig_cols, orig_rows, tile_size);

    
    let slice = normalized.as_slice().expect("Array2 must be contiguous");
    let (global_min, global_max) = find_minmax_simd(slice);

    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create tile output dir {}", output_dir))?;

    let mut levels = Vec::with_capacity(num_levels);
    
    let max_level = num_levels - 1;

    for level in 0..num_levels {
        
        
        let reduction_power = max_level - level;
        let factor = 1usize << reduction_power; 

        let level_data = if factor > 1 {
            downsample(normalized, factor)
        } else {
            normalized.clone()
        };

        let (level_rows, level_cols) = level_data.dim();
        let tile_cols = (level_cols + tile_size - 1) / tile_size;
        let tile_rows = (level_rows + tile_size - 1) / tile_size;

        let scale_factor = 1.0 / factor as f64;

        let level_dir = format!("{}/{}", output_dir, level);
        fs::create_dir_all(&level_dir)
            .with_context(|| format!("Failed to create level dir {}", level_dir))?;

        for ty in 0..tile_rows {
            for tx in 0..tile_cols {
                let tile_path = format!("{}/{}_{}.png", level_dir, tx, ty);
                render_tile(
                    &level_data,
                    tx,
                    ty,
                    tile_size,
                    global_min,
                    global_max,
                    &tile_path,
                )?;
            }
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

pub fn generate_single_tile(
    normalized: &Array2<f32>,
    output_dir: &str,
    level: usize,
    col: usize,
    row: usize,
    tile_size: usize,
    total_levels: usize,
) -> Result<String> {
    let (orig_rows, orig_cols) = normalized.dim();
    let max_level = total_levels.saturating_sub(1);
    let reduction_power = max_level.saturating_sub(level);
    let factor = 1usize << reduction_power;

    let level_data = if factor > 1 {
        downsample(normalized, factor)
    } else {
        normalized.clone()
    };

    let slice = normalized.as_slice().expect("Array2 must be contiguous");
    let (global_min, global_max) = find_minmax_simd(slice);

    let tile_path = format!("{}/{}/{}_{}.png", output_dir, level, col, row);
    render_tile(
        &level_data,
        col,
        row,
        tile_size,
        global_min,
        global_max,
        &tile_path,
    )?;

    Ok(tile_path)
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
