use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::core::imaging::normalize::asinh_normalize;
use crate::domain::cube::{self, CubeResult};
use crate::infra::fits::dispatcher::resolve_input;
use crate::infra::fits::reader::{extract_cube_mmap, extract_image_mmap};
use crate::infra::render::render_grayscale;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PipelineResult {
    pub total_files: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub elapsed_ms: u64,
    pub results: Vec<SingleResult>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum SingleResult {
    Cube {
        path: String,
        cube: CubeResult,
        elapsed_ms: u64,
    },
    Image {
        path: String,
        png_path: String,
        fits_path: Option<String>,
        dimensions: [usize; 2],
        elapsed_ms: u64,
    },
    Err {
        path: String,
        error: String,
    },
}

fn try_as_cube(path: &Path) -> bool {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    extract_cube_mmap(&file).is_ok()
}

fn process_single_file(
    fits_path: &Path,
    output_dir: &str,
    frame_step: usize,
) -> Result<SingleResult> {
    let file_start = Instant::now();
    let path_str = fits_path.to_string_lossy().to_string();
    let stem = fits_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let sub_dir = PathBuf::from(output_dir).join(&stem);
    fs::create_dir_all(&sub_dir)?;
    let sub_dir_str = sub_dir.to_string_lossy().to_string();

    if try_as_cube(fits_path) {
        let cube_result = cube::process_cube(&path_str, &sub_dir_str, frame_step)?;
        return Ok(SingleResult::Cube {
            path: path_str,
            cube: cube_result,
            elapsed_ms: file_start.elapsed().as_millis() as u64,
        });
    }

    let file = File::open(fits_path)
        .with_context(|| format!("Failed to open {}", path_str))?;
    let img_result = extract_image_mmap(&file)?;
    let (rows, cols) = img_result.image.dim();

    let normalized = asinh_normalize(&img_result.image);
    let png_path = format!("{}/{}.png", sub_dir_str, stem);
    render_grayscale(&normalized, &png_path)?;

    let fits_out = format!("{}/{}.fits", sub_dir_str, stem);
    crate::infra::fits::writer::write_fits_mono(&fits_out, &img_result.image, None)?;

    Ok(SingleResult::Image {
        path: path_str,
        png_path,
        fits_path: Some(fits_out),
        dimensions: [cols, rows],
        elapsed_ms: file_start.elapsed().as_millis() as u64,
    })
}

pub fn run_pipeline(
    input_path: &str,
    output_dir: &str,
    frame_step: usize,
) -> Result<PipelineResult> {
    let start = Instant::now();
    let input = Path::new(input_path);
    let resolved = resolve_input(input)
        .with_context(|| format!("Failed to resolve input {}", input_path))?;
    let paths = resolved.image_paths().to_vec();
    let total = paths.len();

    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create pipeline output dir {}", output_dir))?;

    let results: Vec<SingleResult> = paths
        .into_par_iter()
        .map(|fits_path| {
            match process_single_file(&fits_path, output_dir, frame_step) {
                Ok(result) => result,
                Err(e) => SingleResult::Err {
                    path: fits_path.to_string_lossy().to_string(),
                    error: format!("{:#}", e),
                },
            }
        })
        .collect();

    let succeeded = results
        .iter()
        .filter(|r| !matches!(r, SingleResult::Err { .. }))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r, SingleResult::Err { .. }))
        .count();

    Ok(PipelineResult {
        total_files: total,
        succeeded,
        failed,
        elapsed_ms: start.elapsed().as_millis() as u64,
        results,
    })
}
