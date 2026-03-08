use std::fs::{self, File};
use std::path::PathBuf;

use anyhow::{Context, Result};
use ndarray::Array3;
use rayon::prelude::*;

use crate::core::imaging::normalize::asinh_normalize;
use crate::infra::fits::reader::extract_cube_mmap;
use crate::infra::render::render_grayscale;

pub use crate::core::cube::eager::{
    CubeResult,
    collapse_mean, collapse_median, extract_spectrum,
    build_wavelength_axis, compute_global_stats, normalize_with_global,
};

pub fn export_cube_frames_sampled(
    cube: &Array3<f32>,
    output_dir: &str,
    step: usize,
) -> Result<usize> {
    let depth = cube.dim().0;
    let step = step.max(1);
    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create frames dir {}", output_dir))?;

    let global = compute_global_stats(cube);

    let indices: Vec<(usize, usize)> = (0..depth).step_by(step).enumerate().collect();

    indices.par_iter().try_for_each(|&(count, z)| -> Result<()> {
        let slice = cube.index_axis(ndarray::Axis(0), z).to_owned();
        let normalized = normalize_with_global(&slice, &global);
        let path = format!("{}/frame_{:04}.png", output_dir, count);
        render_grayscale(&normalized, &path)
    })?;

    Ok(indices.len())
}

pub fn process_cube(
    input_path: &str,
    output_dir: &str,
    frame_step: usize,
) -> Result<CubeResult> {
    let (actual_fits_path, _tmp_holder) = if input_path.to_lowercase().ends_with(".zip") {
        let resolved = crate::infra::fits::dispatcher::resolve_input(std::path::Path::new(input_path))
            .with_context(|| format!("Failed to resolve ZIP input {}", input_path))?;
        match resolved {
            crate::infra::fits::dispatcher::ResolvedInput::ExtractedFromZip { files, _tmp } => {
                let first = files
                    .into_iter()
                    .next()
                    .context("No .fits in ZIP")?;
                (first, Some(_tmp))
            }
            _ => unreachable!(),
        }
    } else {
        (PathBuf::from(input_path), None)
    };

    let file = File::open(&actual_fits_path)
        .with_context(|| format!("Failed to open FITS {:?}", actual_fits_path))?;
    let result = extract_cube_mmap(&file)
        .context("mmap cube extraction failed")?;

    let cube = result.cube;
    let header = result.header;
    let (depth, rows, cols) = cube.dim();

    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output dir {}", output_dir))?;

    let collapsed = collapse_mean(&cube);
    let collapsed_norm = asinh_normalize(&collapsed);
    let collapsed_path = format!("{}/collapsed_mean.png", output_dir);
    render_grayscale(&collapsed_norm, &collapsed_path)?;

    let collapsed_med = collapse_median(&cube);
    let collapsed_med_norm = asinh_normalize(&collapsed_med);
    let collapsed_med_path = format!("{}/collapsed_median.png", output_dir);
    render_grayscale(&collapsed_med_norm, &collapsed_med_path)?;

    let center_y = rows / 2;
    let center_x = cols / 2;
    let spectrum = extract_spectrum(&cube, center_y, center_x);
    let wavelengths = build_wavelength_axis(&header);

    let frames_dir = format!("{}/frames", output_dir);
    let frame_count = export_cube_frames_sampled(&cube, &frames_dir, frame_step)?;

    Ok(CubeResult {
        dimensions: [cols, rows, depth],
        collapsed_path,
        collapsed_median_path: collapsed_med_path,
        frames_dir,
        frame_count,
        center_spectrum: spectrum,
        wavelengths,
    })
}
