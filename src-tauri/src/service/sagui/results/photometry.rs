use std::collections::BTreeMap;
use anyhow::{bail, ensure, Context, Result};
use ndarray::Array2;
use crate::core::cubeprep::cube::Cube;
use crate::core::metadata::units::{classify_unit, UnitClass};
use crate::infra::progress::ProgressHandle;
use super::maps::component_sizes;
use crate::service::sagui::types::{NoiseCorrelation, Region, RegionalBand};

const CANCEL_CHECK_PIXELS: usize = 1024;
const CSV_HEADER: &str = "region,n_pixels,filter,pivot_um,flux,error,error_correlated_jy,unit";
pub(in crate::service::sagui) const REGION_PHOTOMETRY_NOTE: &str = "sum of original calibrated SCI converted to Jy; error = sqrt(sum(ERR^2)) converted to Jy, ERR = 0 is used as zero variance; unknown if any used ERR is NaN, infinite, negative or absent; error_correlated = sqrt(sum over pixel pairs p,q of ERR_p ERR_q NOISCORR_z(p-q)) converted to Jy when the input carries NOISCORR, empty otherwise and on PSF-matched planes whose NOISCORR covers alignment only (NCORRSRC='align')";

struct Footprint<'a> {
    labels: &'a [u32],
    width: usize,
    region: u32,
    pixels: &'a [usize],
}

fn correlated_variance(footprint: &Footprint, err: &Array2<f32>, map: &Array2<f32>, progress: &ProgressHandle) -> Result<f64> {
    let Footprint { labels, width, region, pixels } = *footprint;
    let height = labels.len() / width;
    let centre = (map.dim().0 / 2) as isize;
    let (reach_y, reach_x) = (centre.min(height as isize - 1), centre.min(width as isize - 1));
    let mut total = 0.0;
    for (n, &p) in pixels.iter().enumerate() {
        if n % CANCEL_CHECK_PIXELS == 0 { ensure!(!progress.is_cancelled(), "SAGUI cancelled; results were not published"); }
        let (y, x) = ((p / width) as isize, (p % width) as isize);
        let sigma_p = f64::from(err[(p / width, p % width)]);
        for dy in -reach_y..=reach_y {
            let qy = y + dy;
            if qy < 0 || qy >= height as isize { continue; }
            for dx in -reach_x..=reach_x {
                let qx = x + dx;
                if qx < 0 || qx >= width as isize || labels[qy as usize * width + qx as usize] != region { continue; }
                let weight = f64::from(map[((centre + dy) as usize, (centre + dx) as usize)]);
                if weight != 0.0 { total += sigma_p * f64::from(err[(qy as usize, qx as usize)]) * weight; }
            }
        }
    }
    Ok(total)
}

fn min_snr(bands: &[RegionalBand]) -> Option<f64> {
    let lowest = bands.iter().map(|band| Some(band.flux / band.error?)).collect::<Option<Vec<f64>>>()?.into_iter().fold(f64::INFINITY, f64::min);
    lowest.is_finite().then_some(lowest)
}

pub(in crate::service::sagui) fn diagnostics(labels: &[u32], width: usize, region: &Region) -> (usize, Option<f64>, Option<f64>) {
    let snr = min_snr(&region.bands);
    (component_sizes(labels, width, region.region).len(), snr, snr.map(|value| value / (region.n_pixels as f64).sqrt()))
}

pub(in crate::service::sagui) fn photometry(cube: &Cube, labels: &[u32], correlation: Option<&NoiseCorrelation>, progress: &ProgressHandle) -> Result<Vec<Region>> {
    let (_, width) = cube.dims();
    let UnitClass::PerPixelPhysical { jy_per_unit } = classify_unit(&cube.header).class else {
        bail!("Regional photometry requires calibrated per-pixel flux units");
    };
    let mut regions: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, &label) in labels.iter().enumerate().filter(|(_, label)| **label > 0) { regions.entry(label).or_default().push(i); }
    let count = regions.len();
    regions.into_iter().map(|(region, indices)| {
        let footprint = Footprint { labels, width, region, pixels: &indices };
        let bands = cube.planes.iter().zip(&cube.filters).enumerate().map(|(b, (plane, filter))| {
            let flux = indices.iter().map(|&i| f64::from(plane[(i / width, i % width)])).sum::<f64>() * jy_per_unit;
            ensure!(flux.is_finite(), "Nonfinite regional flux");
            let err = cube.err.as_ref().map(|err| &err[b]);
            let sigma = err.and_then(|err| indices.iter().map(|&i| {
                let sigma = f64::from(err[(i / width, i % width)]);
                (sigma.is_finite() && sigma >= 0.0).then_some(sigma)
            }).collect::<Option<Vec<f64>>>());
            let error = sigma.as_ref().map(|sigma| sigma.iter().map(|s| s * s).sum::<f64>().sqrt() * jy_per_unit).filter(|v| v.is_finite());
            let map = correlation.filter(|maps| maps.covers(filter)).map(|maps| &maps.planes[b]);
            let error_correlated = match (&sigma, err, map) {
                (Some(_), Some(err), Some(map)) => {
                    progress.emit_stage(&format!("Measuring correlated regional error: region {region} of {count}, {}", filter.filter));
                    Some(correlated_variance(&footprint, err, map, progress)?.sqrt() * jy_per_unit).filter(|v| v.is_finite())
                }
                _ => None,
            };
            Ok(RegionalBand { filter: filter.filter.clone(), pivot_um: filter.pivot_um.context("Missing wavelength")?, flux, error, error_correlated })
        }).collect::<Result<Vec<_>>>()?;
        let mut entry = Region { region, n_pixels: indices.len(), n_components: 0, min_snr: None, min_snr_lower_bound: None, bands };
        (entry.n_components, entry.min_snr, entry.min_snr_lower_bound) = diagnostics(labels, width, &entry);
        Ok(entry)
    }).collect()
}

pub(crate) fn csv_text(text: &str) -> String { format!("\"{}\"", text.replace('"', "\"\"")) }

pub(crate) fn csv_number(value: Option<f64>) -> String { value.map(|v| v.to_string()).unwrap_or_default() }

pub(crate) fn regions_csv(regions: &[Region]) -> String {
    let mut table = format!("{CSV_HEADER}\n");
    for region in regions {
        for band in &region.bands {
            table.push_str(&format!("{},{},{},{},{},{},{},Jy\n", region.region, region.n_pixels, csv_text(&band.filter), band.pivot_um, band.flux, csv_number(band.error), csv_number(band.error_correlated)));
        }
    }
    table
}
