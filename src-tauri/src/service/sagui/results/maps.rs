use std::collections::HashSet;
use std::fs;
use std::path::Path;
use anyhow::{ensure, Context, Result};
use crate::core::cubeprep::cube::Cube;
use crate::service::sagui::types::{MapCounts, SaguiConfig};

pub(in crate::service::sagui) fn read_integers(path: &Path, pixels: usize) -> Result<Vec<u32>> {
    let length = fs::metadata(path).with_context(|| format!("Cannot read the SAGUI map {}", path.display()))?.len();
    ensure!(length == (pixels as u64).saturating_mul(4), "SAGUI returned an incorrectly sized map: {}", path.display());
    Ok(fs::read(path).with_context(|| format!("Cannot read the SAGUI map {}", path.display()))?.chunks_exact(4).map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap())).collect())
}

pub(in crate::service::sagui) fn validate_maps(labels: &[u32], mask: &[u32], eligible: &[bool], n_regions: usize) -> Result<()> {
    ensure!(labels.len() == eligible.len() && mask.len() == eligible.len(), "Map dimensions disagree");
    let mut seen = HashSet::new();
    for ((&label, &mask), &valid) in labels.iter().zip(mask).zip(eligible) {
        ensure!(label <= n_regions as u32 && mask <= 1, "Invalid SAGUI labels or support values");
        ensure!(mask == 0 || valid, "SAGUI marked an excluded pixel as support");
        ensure!(label == 0 || (valid && mask == 1), "SAGUI assigned an excluded pixel to a region");
        if label > 0 { seen.insert(label); }
    }
    ensure!(seen.len() == n_regions, "SAGUI returned a different number of regions than requested");
    Ok(())
}

pub(in crate::service::sagui) fn map_counts(labels: &[u32], mask: &[u32], eligible: &[bool]) -> MapCounts {
    let mut counts = MapCounts { pixels: labels.len(), ..MapCounts::default() };
    for ((&label, &mask), &valid) in labels.iter().zip(mask).zip(eligible) {
        counts.eligible_pixels += usize::from(valid);
        counts.support_pixels += usize::from(mask == 1);
        counts.labelled_pixels += usize::from(label > 0);
        counts.support_unlabelled += usize::from(mask == 1 && label == 0);
        counts.support_on_excluded += usize::from(mask == 1 && !valid);
    }
    counts.excluded_by_policy = counts.pixels - counts.eligible_pixels;
    counts
}

pub(in crate::service::sagui) fn map_warnings(counts: &MapCounts, upstream_support_on_excluded: usize) -> Vec<String> {
    let mut warnings = Vec::new();
    if counts.support_unlabelled > 0 {
        warnings.push(format!("{} support pixels were left unlabelled by SAGUI's validity filter (non-positive sum, zero MAD or a non-finite spectrum)", counts.support_unlabelled));
    }
    if upstream_support_on_excluded > 0 {
        warnings.push(format!("{upstream_support_on_excluded} excluded pixels were marked as support by SAGUI's upstream mask; the published support removes them and support-upstream.bin keeps the raw mask"));
    }
    warnings
}

pub(in crate::service::sagui) fn check_connected(labels: &[u32], config: &SaguiConfig) -> Result<()> {
    let clusters = labels.iter().copied().max().unwrap_or(0) as usize;
    ensure!(config.mode != "sparse" || clusters <= config.n_regions, "Sparse Ward neighbour graph is disconnected: {clusters} clusters for {} requested with knn_k {}; increase the nearest-neighbour count", config.n_regions, config.knn_k);
    Ok(())
}

pub(in crate::service::sagui) fn support_per_region_warning(cube: &Cube, psf_target: &str, support_pixels: usize, n_regions: usize) -> Option<String> {
    let per_region = support_pixels as f64 / n_regions as f64;
    let positive = |value: Option<f64>| value.filter(|v| v.is_finite() && *v > 0.0);
    let target = psf_target.trim();
    let resolved = cube.filters.iter().find(|plane| plane.filter.eq_ignore_ascii_case(target)).and_then(|plane| Some((&plane.filter, positive(plane.psf_fwhm_arcsec)?, positive(plane.pixscale_arcsec)?)));
    let Some((filter, fwhm, scale)) = resolved else {
        return Some(format!("The PSF-size check could not run: FILTERS gives no finite PSF_FWHM and PIXSCALE for the PSF target {target}; support of {support_pixels} pixels over {n_regions} regions gives {per_region:.1} pixels per region, and regions smaller than the PSF noise-equivalent area (pi FWHM^2 / (2 ln 2) in pixels) are not resolved"));
    };
    let fwhm_pixels = fwhm / scale;
    let noise_equivalent_area = std::f64::consts::PI * fwhm_pixels * fwhm_pixels / (2.0 * std::f64::consts::LN_2);
    (per_region < noise_equivalent_area).then(|| format!("Support of {support_pixels} pixels over {n_regions} regions gives {per_region:.1} pixels per region, below the PSF noise-equivalent area of {noise_equivalent_area:.1} pixels ({filter} FWHM {fwhm:.4} arcsec at {scale:.4} arcsec per pixel); regions smaller than the PSF are not resolved, so request fewer regions"))
}

pub(in crate::service::sagui) fn component_sizes(labels: &[u32], width: usize, region: u32) -> Vec<usize> {
    let height = labels.len() / width;
    let mut visited = vec![false; labels.len()];
    let mut sizes = Vec::new();
    for start in (0..labels.len()).filter(|&i| labels[i] == region) {
        if visited[start] { continue; }
        visited[start] = true;
        let mut stack = vec![start];
        let mut size = 0;
        while let Some(index) = stack.pop() {
            size += 1;
            let (y, x) = ((index / width) as isize, (index % width) as isize);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (ny, nx) = (y + dy, x + dx);
                    if ny < 0 || nx < 0 || ny >= height as isize || nx >= width as isize { continue; }
                    let neighbour = ny as usize * width + nx as usize;
                    if labels[neighbour] == region && !visited[neighbour] {
                        visited[neighbour] = true;
                        stack.push(neighbour);
                    }
                }
            }
        }
        sizes.push(size);
    }
    sizes
}

pub(in crate::service::sagui) fn fragment_warning(labels: &[u32], width: usize) -> Option<String> {
    let regions: Vec<u32> = labels.iter().copied().filter(|&label| label > 0).collect::<std::collections::BTreeSet<_>>().into_iter()
        .filter(|&region| component_sizes(labels, width, region).contains(&1)).collect();
    let ids = regions.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
    match regions.len() {
        0 => None,
        1 => Some(format!("Region {ids} contains 1-pixel fragments (8-connected); SAGUI groups pixels by spectrum, not by position")),
        _ => Some(format!("Regions {ids} contain 1-pixel fragments (8-connected); SAGUI groups pixels by spectrum, not by position")),
    }
}
