use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::domain::cube::{self, CubeResult};
use crate::utils::dispatcher::resolve_input;

#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub total_files: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub elapsed_ms: u64,
    pub results: Vec<SingleResult>,
}

#[derive(Debug, Clone)]
pub enum SingleResult {
    Ok {
        path: String,
        cube: CubeResult,
        elapsed_ms: u64,
    },
    Err {
        path: String,
        error: String,
    },
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
    let paths = resolved.fits_paths().to_vec();
    let total = paths.len();

    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create pipeline output dir {}", output_dir))?;

    let results: Vec<SingleResult> = paths
        .into_par_iter()
        .map(|fits_path| {
            let file_start = Instant::now();
            let stem = fits_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let sub_dir = PathBuf::from(output_dir).join(&stem);
            let _ = fs::create_dir_all(&sub_dir);
            let sub_dir_str = sub_dir.to_string_lossy().to_string();
            let path_str = fits_path.to_string_lossy().to_string();

            match cube::process_cube(&path_str, &sub_dir_str, frame_step) {
                Ok(cube_result) => SingleResult::Ok {
                    path: path_str,
                    cube: cube_result,
                    elapsed_ms: file_start.elapsed().as_millis() as u64,
                },
                Err(e) => SingleResult::Err {
                    path: path_str,
                    error: format!("{:#}", e),
                },
            }
        })
        .collect();

    let succeeded = results
        .iter()
        .filter(|r| matches!(r, SingleResult::Ok { .. }))
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
