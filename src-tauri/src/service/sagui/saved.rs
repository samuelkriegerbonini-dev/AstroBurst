use super::*;
use std::fmt::Write as FmtWrite;
use std::path::Component;
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct SavedResult {
    pub result: SaguiResult,
    pub config: SaguiConfig,
}

fn table_text(regions: &[Region]) -> Result<String> {
    let mut table = String::from("region,n_pixels,filter,pivot_um,flux,error,unit\n");
    for region in regions {
        for band in &region.bands {
            writeln!(table, "{},{},{},{},{},{},Jy", region.region, region.n_pixels, csv_text(&band.filter), band.pivot_um, band.flux, band.error.map(|value| value.to_string()).unwrap_or_default())?;
        }
    }
    Ok(table)
}

pub fn load(manifest_path: &str) -> Result<SavedResult> {
    let manifest_path = Path::new(manifest_path).canonicalize()?;
    let root = manifest_path.parent().context("The manifest has no result directory")?;
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    ensure!(manifest["schema_version"] == 1 && manifest["status"] == "complete", "Choose a completed SAGUI analysis manifest with schema version 1");
    let mut result: SaguiResult = serde_json::from_value(manifest["result"].clone())?;
    let mut config: SaguiConfig = serde_json::from_value(manifest["config"].clone())?;
    let pixels = result.width.checked_mul(result.height).context("Invalid saved map dimensions")?;
    ensure!(pixels > 0 && result.labels.len() == pixels && result.unit == "Jy" && valid_run_id(&result.run_id), "Invalid saved SAGUI result");
    ensure!(config.n_regions >= 2 && result.regions.len() == config.n_regions && config.n_regions <= pixels, "Saved region count disagrees with the configuration");
    let progress = ProgressHandle::with_sink(Arc::new(|_| {}), 1);
    let mut files = HashSet::new();
    for file in manifest["files"].as_array().context("The manifest has no product hashes")? {
        let name = file["path"].as_str().context("Missing product path")?;
        let mut components = Path::new(name).components();
        ensure!(matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() && files.insert(name.to_owned()), "Invalid or repeated saved product path");
        let path = root.join(name).canonicalize().with_context(|| format!("Missing saved product: {name}"))?;
        ensure!(path.parent() == Some(root), "A saved product links outside its result directory");
        ensure!(sha256(&path, &progress)? == file["sha256"].as_str().unwrap_or_default(), "Saved product changed: {name}");
    }
    for required in ["labels.fits", "support.fits", "regions.csv", "config.json", "labels.bin", "support.bin"] {
        ensure!(files.contains(required), "The manifest does not verify required product {required}");
    }
    let recorded_config: serde_json::Value = serde_json::from_slice(&fs::read(root.join("config.json"))?)?;
    ensure!(recorded_config == manifest["config"], "Saved configuration disagrees with its verified file");
    let labels = read_integers(&root.join("labels.bin"), pixels)?;
    let mask = read_integers(&root.join("support.bin"), pixels)?;
    ensure!(labels == result.labels, "Saved labels disagree with the verified map");
    validate_maps(&labels, &mask, &vec![true; pixels], config.n_regions)?;
    let mut counts = BTreeMap::<u32, usize>::new();
    for &label in labels.iter().filter(|&&label| label > 0) { *counts.entry(label).or_default() += 1; }
    let first = result.regions.first().context("The result contains no regions")?;
    ensure!(first.bands.len() >= 2, "Saved regional photometry needs at least two bands");
    let mut filters = HashSet::new();
    ensure!(first.bands.iter().all(|band| !band.filter.trim().is_empty() && filters.insert(band.filter.to_lowercase())), "Saved filters are empty or repeated");
    for (region, (&label, &count)) in result.regions.iter().zip(&counts) {
        ensure!(region.region == label && region.n_pixels == count && region.bands.len() == first.bands.len(), "Saved region table disagrees with its map");
        for (band, expected) in region.bands.iter().zip(&first.bands) {
            ensure!(band.filter == expected.filter && band.pivot_um == expected.pivot_um && band.pivot_um.is_finite() && band.pivot_um > 0.0 && band.flux.is_finite() && band.error.is_none_or(|value| value.is_finite() && value >= 0.0), "Invalid saved regional photometry");
        }
    }
    ensure!(fs::read_to_string(root.join("regions.csv"))? == table_text(&result.regions)?, "Saved photometry disagrees with its verified CSV");
    let labels_header = extract_header_by_index(&File::open(root.join("labels.fits"))?, 1)?;
    let label_wcs = header_wcs(&labels_header).context("Saved labels have no celestial WCS")?;
    for (name, expected) in [("labels.fits", &labels), ("support.fits", &mask)] {
        let file = File::open(root.join(name))?;
        let header = extract_header_by_index(&file, 1)?;
        ensure!(header.get_i64("NAXIS1") == Some(result.width as i64) && header.get_i64("NAXIS2") == Some(result.height as i64) && header.get_i64("NAXIS") == Some(2), "Saved FITS dimensions disagree with the result");
        ensure!(same_grid(&label_wcs, (result.height, result.width), &header_wcs(&header)?, (result.height, result.width)), "Saved support WCS differs from the labels");
        let image = crate::infra::fits::reader::extract_int_plane_by_index(&file, 1)?;
        ensure!(image.bits.iter().eq(expected.iter()), "Saved FITS map disagrees with its verified binary map: {name}");
    }
    let path = |name: &str| root.join(name).to_string_lossy().into_owned();
    result.output_dir = root.to_string_lossy().into_owned();
    result.labels_path = path("labels.fits");
    result.mask_path = path("support.fits");
    result.table_path = path("regions.csv");
    result.manifest_path = manifest_path.to_string_lossy().into_owned();
    config.output_dir = root.parent().unwrap_or(root).to_string_lossy().into_owned();
    result.warnings.push("Reopened saved products were checked against their manifest. The original input, PSFs and R installation were not revalidated; consult the saved provenance before running another analysis.".into());
    Ok(SavedResult { result, config })
}
