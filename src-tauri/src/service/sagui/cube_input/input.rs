use std::collections::HashSet;
use std::fs::File;
use anyhow::{bail, ensure, Context, Result};
use crate::core::analysis::resolution::psf_noise_equivalent_area_px;
use crate::core::cubeprep::align::{same_grid, CARD_NCORRK, CARD_NCORRSRC, NOISCORR_EXTNAME, NOISCORR_SOURCE_ALIGN, NOISCORR_SOURCE_MATCH};
use crate::core::cubeprep::cube::Cube;
use crate::core::cubeprep::kernel::FWHM_WARNING;
use crate::core::metadata::photcal::card_text;
use crate::core::metadata::units::{classify_unit, UnitClass, UnitInfo};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::{extract_header_by_index, extract_image_planes_by_index, list_extensions, HduInfo};
use crate::infra::fits::writer::is_wcs_card;
use crate::infra::wcs_source::header_wcs;
use crate::service::cubeprep::{filter_registry, read_cube_file};
use crate::types::header::HduHeader;
use crate::types::image_ref::{ImageRef, PlaneSelector};
use super::dq::dq_flag_counts;
use super::memory::memory_estimate;
use crate::service::sagui::types::{Band, InputInfo, NoiseCorrelation};

fn unit_refusal(unit: &UnitInfo) -> Option<String> {
    let bunit = unit.bunit.clone().unwrap_or_default();
    match &unit.class {
        UnitClass::PerPixelPhysical { .. } => None,
        UnitClass::SurfaceBrightness { .. } => Some(format!("BUNIT '{bunit}' is a surface brightness; SAGUI requires a calibrated per-pixel flux density, so convert it to Jy/pixel in Cube Prep")),
        UnitClass::PerPixelCalibrated { cards } => Some(format!("BUNIT '{bunit}' is calibrated counts ({}); SAGUI requires a calibrated per-pixel flux density, so convert it to Jy/pixel in Cube Prep", cards.join(", "))),
        UnitClass::Unknown => Some(format!("BUNIT '{bunit}' is not a recognised per-pixel flux density (accepted: Jy, mJy, uJy, nJy, MJy (exact case), the words milli, micro, nano and mega, N*unit, optional /pixel; kilo prefixes, mixed-case Mjy and all-caps MJY are refused as ambiguous); Cube Prep cannot convert it either")),
    }
}

fn zero_error_warnings(cube: &Cube) -> Vec<String> {
    let Some(err) = &cube.err else { return Vec::new() };
    cube.planes.iter().zip(err).zip(&cube.filters).filter_map(|((plane, sigma), filter)| {
        let zeros = plane.iter().zip(sigma).filter(|(value, sigma)| value.is_finite() && **sigma == 0.0).count();
        (zeros > 0).then(|| format!("{}: {zeros} pixels with finite SCI have ERR = 0; regional uncertainties use them as zero variance, as sagui::extract_region_sed does, so regions containing them understate the error", filter.filter))
    }).collect()
}

fn measured(band: &Band) -> Option<(&Band, f64)> {
    band.psf_fwhm_arcsec.map(|fwhm| (band, fwhm))
}

fn psf_mismatch_warnings(bands: &[Band]) -> Vec<String> {
    let Some((broadest, target)) = bands.iter().filter_map(measured).max_by(|a, b| a.1.total_cmp(&b.1)) else { return Vec::new() };
    bands.iter().filter_map(measured).filter(|(_, fwhm)| (fwhm / target - 1.0).abs() > FWHM_WARNING).map(|(band, fwhm)| {
        format!("{}: PSF_FWHM {fwhm} arcsec differs from the broadest band ({}, {target} arcsec) by more than {:.0} percent, so the bands are not matched to one resolution and regional colours mix different apertures", band.filter, broadest.filter, 100.0 * FWHM_WARNING)
    }).collect()
}

pub(in crate::service::sagui) fn validate_cube(cube: &Cube) -> Result<InputInfo> {
    ensure!(cube.planes.len() >= 2, "SAGUI requires at least two photometric bands");
    ensure!(card_text(&cube.header, "CTYPE3").is_some_and(|value| value.eq_ignore_ascii_case("FILTER")), "SAGUI requires CTYPE3=FILTER; a spectral cube is not a multiband preparation");
    if let Some(refusal) = unit_refusal(&classify_unit(&cube.header)) { bail!(refusal); }
    cube.wcs().context("The multiband cube needs a valid celestial WCS")?;
    let (height, width) = cube.dims();
    ensure!(width > 0 && height > 0 && cube.planes.iter().all(|p| p.dim() == (height, width)), "Inconsistent science dimensions");
    if let Some(err) = &cube.err { ensure!(err.len() == cube.planes.len() && err.iter().all(|p| p.dim() == (height, width)), "ERR dimensions differ from SCI"); }
    if let Some(dq) = &cube.dq { ensure!(dq.planes.len() == cube.planes.len() && dq.planes.iter().all(|p| p.dim() == (height, width)), "DQ dimensions differ from SCI"); }
    ensure!(cube.filters.len() == cube.planes.len(), "FILTERS order does not cover every science plane");
    let mut seen = HashSet::new();
    let bands = cube.filters.iter().map(|plane| {
        ensure!(!plane.filter.trim().is_empty() && seen.insert(plane.filter.to_lowercase()), "Filters must be nonempty and unique");
        let pivot_um = plane.pivot_um.filter(|v| v.is_finite() && *v > 0.0).context("Every filter requires a positive pivot wavelength")?;
        let psf_fwhm_arcsec = plane.psf_fwhm_arcsec.filter(|v| v.is_finite() && *v > 0.0);
        let pixscale_arcsec = plane.pixscale_arcsec.filter(|v| v.is_finite() && *v > 0.0);
        let psf_area_px = psf_fwhm_arcsec.zip(pixscale_arcsec).map(|(fwhm, scale)| psf_noise_equivalent_area_px(fwhm / scale));
        Ok(Band { filter: plane.filter.clone(), pivot_um, psf_fwhm_arcsec, pixscale_arcsec, psf_area_px })
    }).collect::<Result<Vec<_>>>()?;
    let mut warnings = vec!["External cubes require aligned bands and a verified common PSF; a target name alone does not validate matching".to_string()];
    if cube.err.is_none() { warnings.push("ERR is absent: regional uncertainties remain unknown".into()); }
    if cube.dq.is_none() { warnings.push("DQ is absent: only finite-value support can be checked".into()); }
    warnings.extend(zero_error_warnings(cube));
    warnings.extend(psf_mismatch_warnings(&bands));
    Ok(InputInfo { width, height, bands, unit: card_text(&cube.header, "BUNIT").unwrap_or_default(), has_err: cube.err.is_some(), has_dq: cube.dq.is_some(), has_noise_correlation: false, dq_flags: dq_flag_counts(cube), estimated_memory_bytes: memory_estimate((width * height) as u64, cube.planes.len() as u64, "ward", 40), warnings, redshift: cube.header.get_f64("REDSHIFT").filter(|z| z.is_finite() && *z > 0.0) })
}

pub(in crate::service::sagui) fn validate_companion_header(cube: &Cube, header: &HduHeader, name: &str) -> Result<Vec<String>> {
    if name == "ERR" {
        let sci_unit = classify_unit(&cube.header);
        let err_unit = classify_unit(header);
        ensure!(err_unit.bunit.is_some() && err_unit.class == sci_unit.class, "ERR BUNIT must explicitly match the calibrated SCI unit");
    }
    if let Some(axis) = card_text(header, "CTYPE3") { ensure!(axis.eq_ignore_ascii_case("FILTER"), "{name} band axis differs from SCI"); }
    let mut warnings = Vec::new();
    if header.cards.iter().any(|(key, _)| is_wcs_card(key) && !matches!(key.as_str(), "CTYPE3" | "CRVAL3" | "CRPIX3" | "CDELT3" | "CUNIT3" | "WCSAXES")) {
        let wcs = header_wcs(header).with_context(|| format!("{name} has an incomplete celestial WCS"))?;
        ensure!(same_grid(&cube.wcs()?, cube.dims(), &wcs, cube.dims()), "{name} WCS differs from SCI");
    } else {
        warnings.push(format!("{name} has no celestial WCS: same-index alignment to SCI is assumed from matching FITS dimensions"));
    }
    Ok(warnings)
}

fn read_noise_correlation(file: &File, hdu: &HduInfo, bands: usize) -> Result<NoiseCorrelation> {
    ensure!(hdu.naxis == 3 && hdu.naxis1 == hdu.naxis2 && hdu.naxis1 % 2 == 1 && hdu.naxis3 == bands as i64, "{NOISCORR_EXTNAME} must be a 3-D [plane, K, K] image with odd K and one map per band; found NAXIS={} {}x{}x{}", hdu.naxis, hdu.naxis1, hdu.naxis2, hdu.naxis3);
    let size = hdu.naxis1 as usize;
    let header = extract_header_by_index(file, hdu.index)?;
    let declared = header.get_i64(CARD_NCORRK);
    ensure!(declared == Some(size as i64), "{NOISCORR_EXTNAME} {CARD_NCORRK}={} disagrees with its {size}x{size} maps", declared.map_or("absent".to_string(), |v| v.to_string()));
    let source = card_text(&header, CARD_NCORRSRC).unwrap_or_default();
    ensure!(source == NOISCORR_SOURCE_ALIGN || source == NOISCORR_SOURCE_MATCH, "{NOISCORR_EXTNAME} {CARD_NCORRSRC}='{source}' is neither '{NOISCORR_SOURCE_ALIGN}' nor '{NOISCORR_SOURCE_MATCH}'");
    let planes = extract_image_planes_by_index(file, hdu.index)?;
    ensure!(planes.len() == bands && planes.iter().all(|plane| plane.dim() == (size, size)), "{NOISCORR_EXTNAME} holds {} maps for {bands} bands", planes.len());
    for (z, plane) in planes.iter().enumerate() {
        let centre = plane[(size / 2, size / 2)];
        ensure!((f64::from(centre) - 1.0).abs() <= f64::from(f32::EPSILON) && plane.iter().all(|v| v.is_finite()), "{NOISCORR_EXTNAME} plane {z} is not a normalised finite autocorrelation: Q(0,0) is {centre}, not 1 to float32 precision, or a value is not finite");
    }
    Ok(NoiseCorrelation { half: size / 2, source, planes })
}

pub(in crate::service::sagui) fn load_input(path: &str) -> Result<(Cube, InputInfo, Option<NoiseCorrelation>)> {
    let reference = ImageRef::parse(path);
    ensure!(matches!(reference.plane, PlaneSelector::Auto), "Select the whole multiband FITS file; SAGUI identifies SCI/ERR/DQ by extension name");
    let cube = read_cube_file(path, &filter_registry())?.cube;
    let mut info = validate_cube(&cube)?;
    let (fits_path, _temporary) = resolve_single_image(&reference.path)?;
    let file = File::open(&fits_path).with_context(|| format!("The input cube {} is not readable", fits_path.display()))?;
    let mut correlation = None;
    let mut correlation_seen = false;
    for hdu in list_extensions(&file)? {
        let name = hdu.extname.as_deref().unwrap_or_default().trim().trim_matches('\'').trim().to_uppercase();
        if name == "ERR" || name == "DQ" {
            ensure!((name == "ERR" && cube.err.is_some()) || (name == "DQ" && cube.dq.is_some()), "{name} was declared but could not be loaded; use a MEF with 3D SCI/ERR/DQ extensions");
            ensure!(hdu.naxis == 3 && hdu.naxis1 == info.width as i64 && hdu.naxis2 == info.height as i64 && hdu.naxis3 == info.bands.len() as i64, "{name} dimensions differ from SCI");
            info.warnings.extend(validate_companion_header(&cube, &extract_header_by_index(&file, hdu.index)?, &name)?);
        } else if name == NOISCORR_EXTNAME && !correlation_seen {
            correlation_seen = true;
            match read_noise_correlation(&file, &hdu, info.bands.len()) {
                Ok(maps) => correlation = Some(maps),
                Err(error) => info.warnings.push(format!("{NOISCORR_EXTNAME} extension is malformed and was ignored ({error:#}); regional errors assume independent pixels and error_correlated stays empty")),
            }
        }
    }
    info.has_noise_correlation = correlation.is_some();
    match &correlation {
        Some(maps) => {
            info.warnings.insert(0, format!("Regional error assumes independent pixels; error_correlated applies the {NOISCORR_EXTNAME} noise autocorrelation ({CARD_NCORRSRC}='{}', {1}x{1} per plane)", maps.source, 2 * maps.half + 1));
            let withheld = maps.withheld(&cube);
            if !withheld.is_empty() {
                info.warnings.push(format!("error_correlated is left empty for {}: {CARD_NCORRSRC}='{NOISCORR_SOURCE_ALIGN}' maps hold only the resampling correlation, and these planes were PSF-matched (FILTERS KERNEL set), so their matching covariance is not represented", withheld.join(", ")));
            }
        }
        None if !correlation_seen => info.warnings.insert(0, format!("{NOISCORR_EXTNAME} extension is absent: regional errors assume independent pixels and error_correlated stays empty; resampling and PSF matching create covariance that is not stored")),
        None => {}
    }
    Ok((cube, info, correlation))
}

pub fn inspect_input(path: &str) -> Result<InputInfo> {
    Ok(load_input(path)?.1)
}
