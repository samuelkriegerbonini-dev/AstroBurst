use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};

use crate::model::HduHeader;
use crate::utils::dispatcher;
use crate::utils::mmap::{self, extract_image_mmap, MmapImageResult};

pub fn resolve_fits(path: &str) -> Result<(std::path::PathBuf, Option<tempfile::TempDir>)> {
    dispatcher::resolve_single_fits(path)
}

pub fn extract_image_resolved(
    path: &str,
) -> Result<(HduHeader, ndarray::Array2<f32>, Option<tempfile::TempDir>)> {
    let (fits_path, tmp) = resolve_fits(path)?;
    let fits_str = fits_path.to_string_lossy().to_string();
    let file =
        File::open(&fits_path).with_context(|| format!("Failed to open {}", fits_str))?;
    let result = extract_image_mmap(&file)?;
    Ok((result.header, result.image, tmp))
}

pub fn extract_image_full(
    path: &str,
) -> Result<(MmapImageResult, Option<tempfile::TempDir>)> {
    let (fits_path, tmp) = resolve_fits(path)?;
    let fits_str = fits_path.to_string_lossy().to_string();
    let file =
        File::open(&fits_path).with_context(|| format!("Failed to open {}", fits_str))?;
    let result = extract_image_mmap(&file)?;
    Ok((result, tmp))
}

pub fn extract_image_by_hdu(
    path: &str,
    hdu_index: usize,
) -> Result<(MmapImageResult, Option<tempfile::TempDir>)> {
    let (fits_path, tmp) = resolve_fits(path)?;
    let fits_str = fits_path.to_string_lossy().to_string();
    let file =
        File::open(&fits_path).with_context(|| format!("Failed to open {}", fits_str))?;
    let result = mmap::extract_image_mmap_by_index(&file, hdu_index)?;
    Ok((result, tmp))
}

pub fn list_fits_extensions(
    path: &str,
) -> Result<(Vec<mmap::HduInfo>, Option<tempfile::TempDir>)> {
    let (fits_path, tmp) = resolve_fits(path)?;
    let fits_str = fits_path.to_string_lossy().to_string();
    let file =
        File::open(&fits_path).with_context(|| format!("Failed to open {}", fits_str))?;
    let extensions = mmap::list_extensions(&file)?;
    Ok((extensions, tmp))
}

pub fn resolve_output_dir(output_dir: &str) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output dir {}", output_dir))?;
    let resolved = std::fs::canonicalize(Path::new(output_dir))
        .unwrap_or_else(|_| Path::new(output_dir).to_path_buf());
    Ok(resolved)
}

pub fn map_anyhow(e: anyhow::Error) -> String {
    format!("{:#}", e)
}
