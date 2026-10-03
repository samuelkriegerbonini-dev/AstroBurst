use std::collections::BTreeMap;
use std::fs::File;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, bail, Result};
use ndarray::Array2;
use serde::Serialize;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, output_stem, resolve_output_dir, write_derived_fits, OutputValues,
};
use crate::cmd::cube::{linear_preview, strip_spectral_axis, MapFiles};
use crate::core::astrometry::spectral::VelocityConvention;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::cube::lazy::LazyCube;
use crate::core::cube::linefit::{
    fit_cube_lines, fit_spaxel, plane_names, Components, LineFitConfig, LineFitInputs, LineFitMaps, Weighting,
};
use crate::core::cube::moments::{ContinuumWindows, VELOCITY_UNIT};
use crate::infra::fits::reader::{list_extensions, HduInfo};
use crate::infra::render::grayscale::render_grayscale;
use crate::types::constants::{EXTNAME_DQ, EXTNAME_ERR, HEADER_BUNIT};
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

pub const ABPROC_PREFIX: &str = "linefit-";
pub const FILE_TAG: &str = "linefit";
const CARD_LINEFIT: &str = "LINEFIT";
const CARD_REST: &str = "LFREST";
const CARD_Z0: &str = "LFZ0";
const CARD_Z1: &str = "LFZ1";
const CARD_CONVENTION: &str = "LFCONV";
const CARD_WEIGHT: &str = "LFWEIGHT";
const CARD_ERR_HDU: &str = "LFERRHDU";
const CARD_DQ_HDU: &str = "LFDQHDU";
const CARD_RPOW: &str = "LFRPOW";
const CARD_SIGMA: &str = "LFSIGMA";
const CARD_NCOMP: &str = "LFNCOMP";
const CARD_SNR: &str = "LFSNR";
const CARD_CONT_A: &str = "LFCONTA";
const CARD_CONT_B: &str = "LFCONTB";
const CARD_MASK_A: &str = "LFMASKA";
const CARD_MASK_B: &str = "LFMASKB";
const CARD_MASK_C: &str = "LFMASKC";
const MASK_A_TEXT: &str = "1 fitted,2 const cont,4 dq drop,8 err drop";
const MASK_B_TEXT: &str = "16 unresolved,32 no conv,64 two rej,128 two comp";
const MASK_C_TEXT: &str = "mask 0 & ncomp>=1: converged below S/N threshold; ncomp 0: no fit";
const PLANE_MASK: &str = "mask";
const SIGMA_OBSERVED: &str = "observed";
const SIGMA_CORRECTED: &str = "corrected";
const FLUX_PLANES: [&str; 6] = ["flux", "flux_err", "c1_flux", "c1_flux_err", "c2_flux", "c2_flux_err"];
const VELOCITY_PLANES: [&str; 13] = [
    "velocity",
    "v_err",
    "sigma_obs",
    "sigma_corr",
    "sigma_err",
    "c1_velocity",
    "c1_v_err",
    "c1_sigma_obs",
    "c1_sigma_err",
    "c2_velocity",
    "c2_v_err",
    "c2_sigma_obs",
    "c2_sigma_err",
];

pub(crate) struct CubeSet {
    pub sci: Arc<LazyCube>,
    pub err: Option<Arc<LazyCube>>,
    pub dq: Option<Arc<LazyCube>>,
}

impl CubeSet {
    fn inputs(&self) -> LineFitInputs<'_> {
        LineFitInputs { sci: &self.sci, err: self.err.as_deref(), dq: self.dq.as_deref() }
    }
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn companion_index(extensions: &[HduInfo], name: &str, sci: &LazyCube) -> Option<usize> {
    let g = &sci.geometry;
    let shape = (g.naxis1 as i64, g.naxis2 as i64, g.naxis3 as i64);
    extensions
        .iter()
        .find(|h| {
            h.index != sci.hdu_index
                && h.naxis == 3
                && (h.naxis1, h.naxis2, h.naxis3) == shape
                && h.extname.as_deref().is_some_and(|e| e.trim().trim_matches('\'').trim().eq_ignore_ascii_case(name))
        })
        .map(|h| h.index)
}

pub(crate) fn open_cube_set(path: &str) -> Result<CubeSet> {
    let sci = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    if let Some(name) = card_text(&sci.header, "EXTNAME").map(|s| s.to_uppercase()) {
        if name == EXTNAME_ERR || name == EXTNAME_DQ {
            bail!(
                "{} opens the {} extension (HDU {}): the line fit needs the SCI cube",
                path,
                name,
                sci.hdu_index
            );
        }
    }
    let base = ImageRef::parse(path).path;
    let extensions = File::open(&base).ok().and_then(|file| list_extensions(&file).ok()).unwrap_or_default();
    let open = |name: &str| -> Result<Option<Arc<LazyCube>>> {
        match companion_index(&extensions, name, &sci) {
            Some(index) => Ok(Some(GLOBAL_CUBE_CACHE.get_or_open(&format!("{}#hdu={}", base, index))?)),
            None => Ok(None),
        }
    };
    let err = open(EXTNAME_ERR)?;
    let dq = open(EXTNAME_DQ)?;
    Ok(CubeSet { sci, err, dq })
}

pub(crate) fn plane_unit(plane: &str, maps: &LineFitMaps) -> Option<String> {
    if FLUX_PLANES.contains(&plane) {
        Some(maps.flux_unit.clone())
    } else if VELOCITY_PLANES.contains(&plane) {
        Some(VELOCITY_UNIT.to_string())
    } else {
        None
    }
}

pub(crate) fn line_fit_header(
    cube: &LazyCube,
    plane: &str,
    unit: Option<&str>,
    cfg: &LineFitConfig,
    maps: &LineFitMaps,
) -> HduHeader {
    let abproc = format!("{}{}", ABPROC_PREFIX, plane);
    let mut header = derived_output_header(Some(&strip_spectral_axis(&cube.header)), &abproc, OutputValues::Rescaled);
    header.set(CARD_LINEFIT, plane.to_string());
    if let Some(rest) = maps.rest_um {
        header.set_f64(CARD_REST, rest);
    }
    header.set(CARD_Z0, cfg.z0.to_string());
    header.set(CARD_Z1, cfg.z1.to_string());
    header.set(CARD_CONVENTION, cfg.convention.name().to_string());
    header.set(CARD_WEIGHT, maps.weighting.name().to_string());
    if let Some(n) = maps.err_hdu {
        header.set(CARD_ERR_HDU, n.to_string());
    }
    if let Some(n) = maps.dq_hdu {
        header.set(CARD_DQ_HDU, n.to_string());
    }
    if let Some(r) = cfg.resolving_power {
        header.set_f64(CARD_RPOW, r);
    }
    let sigma = if cfg.resolving_power.is_some() { SIGMA_CORRECTED } else { SIGMA_OBSERVED };
    header.set(CARD_SIGMA, sigma.to_string());
    header.set(CARD_NCOMP, cfg.components.name().to_string());
    header.set_f64(CARD_SNR, cfg.snr_threshold);
    let windows = maps.continuum_windows;
    header.set(CARD_CONT_A, format!("{}-{}", windows.0 .0, windows.0 .1));
    header.set(CARD_CONT_B, format!("{}-{}", windows.1 .0, windows.1 .1));
    match unit {
        Some(u) => header.set(HEADER_BUNIT, u.to_string()),
        None => header.remove(HEADER_BUNIT),
    }
    if plane == PLANE_MASK {
        header.set(CARD_MASK_A, MASK_A_TEXT.to_string());
        header.set(CARD_MASK_B, MASK_B_TEXT.to_string());
        header.set(CARD_MASK_C, MASK_C_TEXT.to_string());
    }
    header
}

fn save_plane(arr: &Array2<f32>, header: &HduHeader, output_dir: &str, name: &str) -> Result<MapFiles> {
    let png_path = format!("{}/{}.png", output_dir, name);
    render_grayscale(&linear_preview(arr), &png_path)?;
    let fits_path = format!("{}/{}.fits", output_dir, name);
    write_derived_fits(&fits_path, arr, Some(header))?;
    Ok(MapFiles { png_path, fits_path })
}

fn plane_array<'a>(maps: &'a LineFitMaps, plane: &str) -> Option<&'a Array2<f32>> {
    let component = |index: usize, field: &str| {
        let planes = &maps.components.as_ref()?[index];
        match field {
            "flux" => Some(&planes.flux),
            "velocity" => Some(&planes.velocity),
            "sigma_obs" => Some(&planes.sigma_obs),
            "flux_err" => Some(&planes.flux_err),
            "v_err" => Some(&planes.v_err),
            "sigma_err" => Some(&planes.sigma_err),
            _ => None,
        }
    };
    match plane {
        "flux" => Some(&maps.flux),
        "velocity" => Some(&maps.velocity),
        "sigma_obs" => Some(&maps.sigma_obs),
        "sigma_corr" => maps.sigma_corr.as_ref(),
        "flux_err" => Some(&maps.flux_err),
        "v_err" => Some(&maps.v_err),
        "sigma_err" => Some(&maps.sigma_err),
        "chi2_red" => Some(&maps.chi2_red),
        "snr" => Some(&maps.snr),
        "mask" => Some(&maps.mask),
        "ncomp" => Some(&maps.ncomp),
        other => match other.split_once('_') {
            Some(("c1", field)) => component(0, field),
            Some(("c2", field)) => component(1, field),
            _ => None,
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LineFitUnits {
    pub flux: String,
    pub velocity: &'static str,
    pub sigma: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineFitResponse {
    pub planes: BTreeMap<String, MapFiles>,
    pub plane_order: Vec<String>,
    pub units: LineFitUnits,
    pub sigma_label: String,
    pub weighting: Weighting,
    pub err_hdu: Option<usize>,
    pub dq_hdu: Option<usize>,
    pub z0: usize,
    pub z1: usize,
    pub n_channels: usize,
    pub rest_um: Option<f64>,
    pub convention: VelocityConvention,
    pub components: Components,
    pub continuum_windows: ContinuumWindows,
    pub resolving_power: Option<f64>,
    pub snr_threshold: f64,
    pub n_fit: usize,
    pub n_masked: usize,
    pub n_const_continuum: usize,
    pub n_two_components: usize,
    pub median_chi2_red: Option<f64>,
    pub notes: Vec<String>,
    pub dimensions: [usize; 2],
    pub elapsed_ms: u64,
}

pub(crate) fn line_fit_files(
    set: &CubeSet,
    path: &str,
    output_dir: &str,
    cfg: &LineFitConfig,
    t0: Instant,
) -> Result<LineFitResponse> {
    let maps = fit_cube_lines(&set.inputs(), cfg)?;
    let stem = output_stem(path);
    let range = format!("{}-{}", cfg.z0, cfg.z1);
    let mut planes = BTreeMap::new();
    let mut plane_order = Vec::new();
    for plane in plane_names(cfg, &maps) {
        let arr = plane_array(&maps, plane).ok_or_else(|| anyhow!("line-fit plane '{}' was not produced", plane))?;
        let unit = plane_unit(plane, &maps);
        let header = line_fit_header(&set.sci, plane, unit.as_deref(), cfg, &maps);
        let name = format!("{}_{}_{}_{}", stem, FILE_TAG, plane, range);
        planes.insert(plane.to_string(), save_plane(arr, &header, output_dir, &name)?);
        plane_order.push(plane.to_string());
    }
    let (rows, cols) = maps.flux.dim();
    Ok(LineFitResponse {
        planes,
        plane_order,
        units: LineFitUnits { flux: maps.flux_unit.clone(), velocity: maps.velocity_unit, sigma: VELOCITY_UNIT },
        sigma_label: maps.sigma_label,
        weighting: maps.weighting,
        err_hdu: maps.err_hdu,
        dq_hdu: maps.dq_hdu,
        z0: cfg.z0,
        z1: cfg.z1,
        n_channels: maps.n_channels,
        rest_um: maps.rest_um,
        convention: cfg.convention,
        components: cfg.components,
        continuum_windows: maps.continuum_windows,
        resolving_power: cfg.resolving_power,
        snr_threshold: cfg.snr_threshold,
        n_fit: maps.n_fit,
        n_masked: maps.n_masked,
        n_const_continuum: maps.n_const_continuum,
        n_two_components: maps.n_two_components,
        median_chi2_red: maps.median_chi2_red,
        notes: maps.notes,
        dimensions: [cols, rows],
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

fn line_fit_json(path: &str, output_dir: &str, cfg: &LineFitConfig) -> Result<serde_json::Value> {
    let t0 = Instant::now();
    let out_dir = resolve_output_dir(output_dir)?;
    let set = open_cube_set(path)?;
    let response = line_fit_files(&set, path, &out_dir, cfg, t0)?;
    Ok(serde_json::to_value(&response)?)
}

fn spaxel_json(path: &str, cfg: &LineFitConfig, x: usize, y: usize) -> Result<serde_json::Value> {
    let set = open_cube_set(path)?;
    let spaxel = fit_spaxel(&set.inputs(), cfg, x, y)?;
    Ok(serde_json::to_value(&spaxel)?)
}

#[tauri::command]
pub async fn cube_line_fit_cmd(
    path: String,
    output_dir: String,
    config: LineFitConfig,
) -> std::result::Result<serde_json::Value, String> {
    blocking_cmd!(line_fit_json(&path, &output_dir, &config))
}

#[tauri::command]
pub async fn cube_line_fit_spaxel_cmd(
    path: String,
    config: LineFitConfig,
    x: usize,
    y: usize,
) -> std::result::Result<serde_json::Value, String> {
    blocking_cmd!(spaxel_json(&path, &config, x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::HEADER_ABPROC;
    use crate::cmd::line_measure::{measure_line_json, LineRequest, SpectrumSource};
    use crate::core::analysis::line_measure::LineModel;
    use crate::core::astrometry::wcs::WcsTransform;
    use crate::core::cube::lazy::test_support::*;
    use crate::core::cube::linefit::test_support::*;
    use crate::core::cube::linefit::{MASK_CONST_CONTINUUM, MASK_FITTED, MASK_NOT_CONVERGED, MASK_TWO_COMPONENTS};
    use crate::core::cube::moments::{moment_maps, MomentConfig};
    use crate::infra::fits::reader::extract_image_mmap;

    const NIRSPEC_DIR_VAR: &str = "ASTROBURST_NIRSPEC_DIR";
    const G235H: &str = "jw01266-o005_t001_nirspec_g235h-f170lp_s3d.fits";
    const G395H: &str = "jw01266-o005_t001_nirspec_g395h-f290lp_s3d.fits";
    const PA_ALPHA_UM: f64 = 1.875613;
    const BR_BETA_UM: f64 = 2.625872;
    const BR_ALPHA_UM: f64 = 4.052262;
    const PA_ALPHA_WINDOW: (usize, usize) = (530, 560);
    const PA_ALPHA_CONTINUUM: ContinuumWindows = ((500, 520), (568, 588));
    const BR_BETA_WINDOW: (usize, usize) = (2424, 2453);
    const BR_BETA_CONTINUUM: ContinuumWindows = ((2394, 2414), (2463, 2483));
    const BR_ALPHA_WINDOW: (usize, usize) = (1769, 1786);
    const BR_ALPHA_CONTINUUM: ContinuumWindows = ((1750, 1762), (1789, 1800));
    const G235H_CDELT_UM: f64 = 3.96e-4;

    fn reopen(path: &str) -> crate::infra::fits::reader::MmapImageResult {
        extract_image_mmap(&File::open(path).unwrap()).unwrap()
    }

    fn out_dir(dir: &tempfile::TempDir) -> String {
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        out.to_str().unwrap().to_string()
    }

    fn write_mef(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        write_linefit_mef(
            &path,
            LINE_CUBE_SIZE,
            LINE_CUBE_SIZE,
            LINE_CUBE_DEPTH,
            |z, y, x| line_sci(z, y, x) + deterministic_noise(z, y, x, noise_amplitude(0.02)),
            Some(|_, _, _| 0.02f32),
            Some(|_, _, _| 0i32),
        );
        path.to_str().unwrap().to_string()
    }

    fn bit(mask: f32, flag: u32) -> bool {
        (mask as u32) & flag != 0
    }

    #[test]
    fn err_and_dq_cubes_are_discovered_by_extname_and_named_in_the_response() {
        let dir = tempfile::tempdir().unwrap();
        let mef = write_mef(&dir, "mef.fits");
        let key = format!("{}#hdu=1", mef);
        let out = out_dir(&dir);
        let set = open_cube_set(&key).unwrap();
        assert_eq!(set.sci.hdu_index, 1);
        assert_eq!(set.err.as_ref().map(|c| c.hdu_index), Some(2));
        assert_eq!(set.dq.as_ref().map(|c| c.hdu_index), Some(3));
        let response = line_fit_files(&set, &key, &out, &line_cfg(), Instant::now()).unwrap();
        assert_eq!(response.err_hdu, Some(2));
        assert_eq!(response.dq_hdu, Some(3));
        assert_eq!(response.weighting, Weighting::Err);
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["weighting"], "err");
        assert_eq!(json["components"], "one");
        assert_eq!(json["convention"], "optical");
        assert_eq!(json["continuum_windows"], serde_json::json!([[0, 5], [34, 39]]));
        assert!(response.notes.iter().any(|n| n.starts_with("weights: 1/ERR^2 from HDU 2 (EXTNAME ERR)")), "{:?}", response.notes);
        assert!(response.notes.iter().any(|n| n.starts_with("DQ HDU 3:")), "{:?}", response.notes);

        let no_err = LineFitConfig { use_err: false, ..line_cfg() };
        let set = open_cube_set(&key).unwrap();
        assert!(set.err.is_some());
        let response = line_fit_files(&set, &key, &out, &no_err, Instant::now()).unwrap();
        assert_eq!(response.weighting, Weighting::Continuum);
        assert_eq!(response.err_hdu, None);
        assert_eq!(response.dq_hdu, Some(3));
        assert!(response.notes.iter().any(|n| n.starts_with("weights: 1/scatter^2")), "{:?}", response.notes);

        let no_dq = LineFitConfig { use_dq: false, ..line_cfg() };
        let response = line_fit_files(&set, &key, &out, &no_dq, Instant::now()).unwrap();
        assert_eq!(response.dq_hdu, None);
        assert!(
            response.notes.iter().any(|n| n == "DQ HDU 3 present: channel masking off (use_dq = false)"),
            "{:?}",
            response.notes
        );

        let plain = dir.path().join("plain.fits");
        write_line_cube(&plain, 0.01);
        let plain = plain.to_str().unwrap().to_string();
        let set = open_cube_set(&plain).unwrap();
        assert!(set.err.is_none() && set.dq.is_none());
        let response = line_fit_files(&set, &plain, &out, &line_cfg(), Instant::now()).unwrap();
        assert_eq!((response.err_hdu, response.dq_hdu), (None, None));
        assert_eq!(response.weighting, Weighting::Continuum);
        assert!(response.notes.iter().any(|n| n == "no DQ cube: no channel masking"), "{:?}", response.notes);
        GLOBAL_CUBE_CACHE.invalidate(&key);
        GLOBAL_CUBE_CACHE.invalidate(&plain);
    }

    #[test]
    fn line_fit_files_write_one_2d_fits_per_plane_with_the_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.01);
        let key = path.to_str().unwrap().to_string();
        let out = out_dir(&dir);
        let set = open_cube_set(&key).unwrap();
        let response = line_fit_files(&set, &key, &out, &line_cfg(), Instant::now()).unwrap();
        assert_eq!(
            response.plane_order,
            ["flux", "velocity", "sigma_obs", "flux_err", "v_err", "sigma_err", "chi2_red", "snr", "mask", "ncomp"]
        );
        assert_eq!(response.planes.len(), response.plane_order.len());
        assert_eq!(response.dimensions, [LINE_CUBE_SIZE, LINE_CUBE_SIZE]);
        assert_eq!(response.units.flux, "Jy/beam km/s");
        assert_eq!(response.units.velocity, "km/s");
        assert_eq!(response.sigma_label, crate::core::cube::linefit::SIGMA_OBSERVED_LABEL);
        assert!(response.planes["velocity"].fits_path.ends_with("line_linefit_velocity_8-32.fits"), "{}", response.planes["velocity"].fits_path);
        assert!(response.n_fit > 100, "n_fit={}", response.n_fit);
        for plane in &response.plane_order {
            let files = &response.planes[plane];
            assert!(std::path::Path::new(&files.png_path).exists(), "{}", files.png_path);
            let reopened = reopen(&files.fits_path);
            let header = &reopened.header;
            assert_eq!(reopened.image.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
            assert_eq!(header.get_i64("NAXIS"), Some(2));
            assert!(header.get("CTYPE3").is_none());
            assert_eq!(header.get(HEADER_ABPROC), Some(format!("linefit-{}", plane).as_str()));
            assert_eq!(header.get(CARD_LINEFIT), Some(plane.as_str()));
            assert_eq!(header.get_i64(CARD_Z0), Some(8));
            assert_eq!(header.get_i64(CARD_Z1), Some(32));
            assert_eq!(header.get(CARD_CONVENTION), Some("optical"));
            assert_eq!(header.get(CARD_WEIGHT), Some("continuum"));
            assert_eq!(header.get(CARD_SIGMA), Some("observed"));
            assert_eq!(header.get(CARD_NCOMP), Some("one"));
            assert_eq!(header.get(CARD_CONT_A), Some("0-5"));
            assert_eq!(header.get(CARD_CONT_B), Some("34-39"));
            assert!((header.get_f64(CARD_SNR).unwrap() - 3.0).abs() < 1e-12);
            assert!((header.get_f64(CARD_REST).unwrap() - LINE_REST_UM).abs() < 1e-12);
            assert!(header.get(CARD_ERR_HDU).is_none() && header.get(CARD_RPOW).is_none());
            assert!(WcsTransform::from_header(header).is_ok(), "{} lost its WCS", plane);
            match plane.as_str() {
                "flux" | "flux_err" => assert_eq!(header.get("BUNIT"), Some("Jy/beam km/s")),
                "velocity" | "v_err" | "sigma_obs" | "sigma_err" => assert_eq!(header.get("BUNIT"), Some("km/s")),
                _ => assert!(header.get("BUNIT").is_none(), "{} carries BUNIT", plane),
            }
            if plane == "mask" {
                assert_eq!(header.get(CARD_MASK_A), Some(MASK_A_TEXT));
                assert_eq!(header.get(CARD_MASK_B), Some(MASK_B_TEXT));
                assert_eq!(header.get(CARD_MASK_C), Some(MASK_C_TEXT));
                assert!(MASK_C_TEXT.len() <= 68 && MASK_C_TEXT.contains("ncomp 0") && MASK_C_TEXT.contains("ncomp>=1"));
            } else {
                assert!(header.get(CARD_MASK_A).is_none());
                assert!(header.get(CARD_MASK_C).is_none());
            }
        }
        let velocity = reopen(&response.planes["velocity"].fits_path).image;
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        assert!(velocity[centre].is_finite());
        assert!(velocity[[0, 0]].is_nan());
        let ncomp = reopen(&response.planes["ncomp"].fits_path).image;
        assert_eq!(ncomp[centre], 1.0);

        let automatic = LineFitConfig { continuum: None, ..line_cfg() };
        let response = line_fit_files(&set, &key, &out, &automatic, Instant::now()).unwrap();
        assert_eq!(response.continuum_windows, ((0, 7), (33, 39)));
        let header = reopen(&response.planes["flux"].fits_path).header;
        assert_eq!(header.get(CARD_CONT_A), Some("0-7"));
        assert_eq!(header.get(CARD_CONT_B), Some("33-39"));
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    fn the_plane_ref_the_frontend_sends_is_accepted_and_stems_the_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("refcube.fits");
        write_line_cube(&path, 0.01);
        let key = format!("{}#hdu=0", path.to_str().unwrap());
        let out = out_dir(&dir);
        let json = line_fit_json(&key, &out, &line_cfg()).unwrap();
        let fits = json["planes"]["flux"]["fits_path"].as_str().unwrap();
        assert!(fits.contains("_hdu0_linefit_flux_8-32"), "{}", fits);
        assert_eq!(json["plane_order"][0], "flux");
        assert!(json["elapsed_ms"].is_u64());
        assert_eq!(json["dimensions"], serde_json::json!([32, 32]));
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    fn opening_the_err_plane_as_the_science_cube_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mef = write_mef(&dir, "mef.fits");
        let key = format!("{}#hdu=2", mef);
        let err = open_cube_set(&key).err().expect("the ERR plane must be refused").to_string();
        assert!(err.contains("ERR") && err.contains("SCI"), "{}", err);
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    fn spaxel_inspect_json_has_the_contract_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mef = write_mef(&dir, "mef.fits");
        let key = format!("{}#hdu=1", mef);
        let json = spaxel_json(&key, &line_cfg(), 16, 16).unwrap();
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected = vec![
            "x", "y", "z0", "z1", "continuum_windows", "span", "axis", "axis_unit", "flux", "err", "channels",
            "dropped_dq", "dropped_err", "continuum", "weighting", "single", "chi2", "dof", "chi2_red", "converged",
            "iterations", "components", "ncomp", "delta_bic", "mask", "model", "notes",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(json["x"], 16);
        assert_eq!(json["span"], serde_json::json!([0, 39]));
        assert_eq!(json["axis"].as_array().unwrap().len(), 40);
        assert_eq!(json["flux"].as_array().unwrap().len(), 40);
        assert_eq!(json["err"].as_array().unwrap().len(), 40);
        assert_eq!(json["weighting"], "err");
        assert_eq!(json["ncomp"], 1);
        assert_eq!(json["converged"], true);
        assert_eq!(json["delta_bic"], serde_json::Value::Null);
        assert_eq!(json["components"].as_array().unwrap().len(), 0);
        assert_eq!(json["model"]["components"].as_array().unwrap().len(), 1);
        assert_eq!(json["model"]["channel"].as_array().unwrap().len(), 157);
        assert_eq!(json["model"]["channel"][156], 39.0);
        assert_eq!(json["model"]["channel"][1], 0.25);
        let single = &json["single"];
        for field in ["amplitude", "centre", "sigma", "velocity_kms", "sigma_kms", "flux", "flux_err", "snr"] {
            assert!(single[field].is_number(), "{} missing in {}", field, single);
        }
        assert!(single["sigma_corr_kms"].is_null());
        let err = spaxel_json(&key, &line_cfg(), 99, 0).unwrap_err().to_string();
        assert!(err.contains("outside"), "{}", err);
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    fn a_resolving_power_adds_the_sigma_corr_plane_and_two_components_add_twelve_planes() {
        let dir = tempfile::tempdir().unwrap();
        let mef = write_mef(&dir, "mef.fits");
        let key = format!("{}#hdu=1", mef);
        let out = out_dir(&dir);
        let set = open_cube_set(&key).unwrap();
        let base = ["flux", "velocity", "sigma_obs", "flux_err", "v_err", "sigma_err", "chi2_red", "snr", "mask", "ncomp"];
        let component_planes = [
            "c1_flux", "c1_velocity", "c1_sigma_obs", "c1_flux_err", "c1_v_err", "c1_sigma_err",
            "c2_flux", "c2_velocity", "c2_sigma_obs", "c2_flux_err", "c2_v_err", "c2_sigma_err",
        ];
        let keys = |response: &LineFitResponse| -> Vec<String> { response.planes.keys().cloned().collect() };
        let sorted = |names: &[&str]| -> Vec<String> {
            let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
            v.sort();
            v
        };

        let with_r = LineFitConfig { resolving_power: Some(2700.0), ..line_cfg() };
        let response = line_fit_files(&set, &key, &out, &with_r, Instant::now()).unwrap();
        let mut expected = base.to_vec();
        expected.insert(3, "sigma_corr");
        assert_eq!(response.plane_order, expected);
        assert_eq!(keys(&response), sorted(&expected));
        assert_eq!(response.resolving_power, Some(2700.0));
        let header = reopen(&response.planes["sigma_corr"].fits_path).header;
        assert_eq!(header.get(CARD_SIGMA), Some("corrected"));
        assert_eq!(header.get("BUNIT"), Some("km/s"));
        assert!((header.get_f64(CARD_RPOW).unwrap() - 2700.0).abs() < 1e-9);

        let two = LineFitConfig { components: Components::Two, ..line_cfg() };
        let response = line_fit_files(&set, &key, &out, &two, Instant::now()).unwrap();
        let mut expected = base.to_vec();
        expected.extend(component_planes);
        assert_eq!(response.plane_order, expected);
        assert_eq!(keys(&response), sorted(&expected));
        assert_eq!(response.components, Components::Two);
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["components"], "two");
        assert_eq!(json["plane_order"].as_array().unwrap().len(), 22);
        for plane in component_planes {
            let files = &response.planes[plane];
            assert!(std::path::Path::new(&files.png_path).exists(), "{}", files.png_path);
            let reopened = reopen(&files.fits_path);
            assert_eq!(reopened.image.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
            assert_eq!(reopened.header.get(CARD_NCOMP), Some("two"));
            assert_eq!(reopened.header.get(CARD_LINEFIT), Some(plane));
            let expected_unit = if plane.ends_with("flux") || plane.ends_with("flux_err") { "Jy/beam km/s" } else { "km/s" };
            assert_eq!(reopened.header.get("BUNIT"), Some(expected_unit), "{}", plane);
        }
        assert_eq!(plane_unit("c1_flux_err", &fit_cube_lines(&set.inputs(), &two).unwrap()).as_deref(), Some("Jy/beam km/s"));

        let both = LineFitConfig { resolving_power: Some(2700.0), components: Components::Auto, ..line_cfg() };
        let response = line_fit_files(&set, &key, &out, &both, Instant::now()).unwrap();
        assert_eq!(response.plane_order.len(), 23);
        assert_eq!(response.plane_order[3], "sigma_corr");
        assert_eq!(response.plane_order[10], "ncomp");
        assert_eq!(response.plane_order[11], "c1_flux");
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    fn nirspec_key(name: &str) -> Option<String> {
        match std::env::var(NIRSPEC_DIR_VAR) {
            Ok(dir) => Some(format!("{}/{}#hdu=1", dir.trim_end_matches(['/', '\\']), name)),
            Err(_) => {
                eprintln!("{} not set: skipped", NIRSPEC_DIR_VAR);
                None
            }
        }
    }

    fn nirspec_cfg(window: (usize, usize), continuum: ContinuumWindows, rest_um: f64) -> LineFitConfig {
        LineFitConfig { z0: window.0, z1: window.1, continuum: Some(continuum), rest_um: Some(rest_um), ..line_cfg() }
    }

    fn moment_cfg(cfg: &LineFitConfig) -> MomentConfig {
        MomentConfig {
            z0: cfg.z0,
            z1: cfg.z1,
            rest_um: cfg.rest_um,
            convention: cfg.convention,
            continuum: cfg.continuum,
            snr_threshold: cfg.snr_threshold,
            mask_below_threshold: false,
        }
    }

    fn percentile(values: &mut Vec<f64>, fraction: f64) -> f64 {
        values.sort_by(|a, b| a.total_cmp(b));
        values[((values.len() - 1) as f64 * fraction).round() as usize]
    }

    fn median(values: &mut Vec<f64>) -> f64 {
        percentile(values, 0.5)
    }

    struct MapCounts {
        converged: usize,
        strong: Vec<(usize, usize)>,
    }

    fn count_map(maps: &LineFitMaps, snr_floor: f32) -> MapCounts {
        let (rows, cols) = maps.mask.dim();
        let mut converged = 0usize;
        let mut strong = Vec::new();
        for y in 0..rows {
            for x in 0..cols {
                let m = maps.mask[[y, x]];
                if maps.ncomp[[y, x]] != 0.0 && !bit(m, MASK_NOT_CONVERGED) {
                    converged += 1;
                }
                if bit(m, MASK_FITTED) && maps.snr[[y, x]] >= snr_floor {
                    strong.push((x, y));
                }
            }
        }
        MapCounts { converged, strong }
    }

    #[test]
    #[ignore]
    fn nirspec_pa_alpha_matches_the_shipped_fitter_and_the_first_moment() {
        let Some(key) = nirspec_key(G235H) else { return };
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let parity_cfg = LineFitConfig { use_err: false, use_dq: false, ..nirspec_cfg(PA_ALPHA_WINDOW, PA_ALPHA_CONTINUUM, PA_ALPHA_UM) };
        let set = open_cube_set(&key).unwrap();
        for (x, y) in [(18usize, 32usize), (26, 27), (30, 20)] {
            let spaxel = fit_spaxel(&set.inputs(), &parity_cfg, x, y).unwrap();
            let single = spaxel.single.expect("converged single fit");
            let request = LineRequest {
                z0: PA_ALPHA_WINDOW.0,
                z1: PA_ALPHA_WINDOW.1,
                continuum: Some(PA_ALPHA_CONTINUUM),
                rest_um: Some(PA_ALPHA_UM),
                convention: VelocityConvention::Optical,
                model: LineModel::Gaussian,
                velocity_shift_kms: None,
            };
            let shipped = measure_line_json(&key, &SpectrumSource::Pixel { x, y }, &request).unwrap();
            let centre = shipped["fit"]["centre"].as_f64().unwrap();
            let sigma = shipped["fit"]["sigma"].as_f64().unwrap();
            let d_centre = (single.centre - centre).abs() / G235H_CDELT_UM;
            let d_sigma = (single.sigma - sigma).abs() / sigma;
            println!(
                "parity ({}, {}): centre {:.6} vs {:.6} um (delta {:.2e} ch), sigma {:.6} vs {:.6} (rel {:.2e}), A {:.1}, v {:.1} km/s, sigma_v {:.1} km/s",
                x, y, single.centre, centre, d_centre, single.sigma, sigma, d_sigma, single.amplitude, single.velocity_kms, single.sigma_kms
            );
            assert!(d_centre < 0.01, "centre differs by {} channels at ({}, {})", d_centre, x, y);
            assert!(d_sigma < 1e-3, "sigma differs by {} at ({}, {})", d_sigma, x, y);
        }

        let cfg = nirspec_cfg(PA_ALPHA_WINDOW, PA_ALPHA_CONTINUUM, PA_ALPHA_UM);
        let set = open_cube_set(&key).unwrap();
        let response = line_fit_files(&set, &key, &out, &cfg, Instant::now()).unwrap();
        println!("Pa alpha response: {}", serde_json::to_string(&serde_json::json!({
            "n_fit": response.n_fit, "n_masked": response.n_masked, "n_const_continuum": response.n_const_continuum,
            "median_chi2_red": response.median_chi2_red, "elapsed_ms": response.elapsed_ms, "weighting": response.weighting,
            "err_hdu": response.err_hdu, "dq_hdu": response.dq_hdu, "units": response.units, "notes": response.notes,
        })).unwrap());
        assert_eq!(response.units.flux, "MJy/sr km/s");
        assert_eq!(response.weighting, Weighting::Err);
        assert!(response.notes.iter().any(|n| n.contains("axis frame BARYCENT")), "{:?}", response.notes);
        let maps = fit_cube_lines(&set.inputs(), &cfg).unwrap();
        let counts = count_map(&maps, 10.0);
        println!("Pa alpha: converged {} (floor 1200), bit 1 & snr>=10 {} (floor 700)", counts.converged, counts.strong.len());
        assert!(counts.converged >= 1200, "converged {}", counts.converged);
        assert!(counts.strong.len() >= 700, "strong {}", counts.strong.len());
        let mut chi2: Vec<f64> = counts.strong.iter().map(|&(x, y)| maps.chi2_red[[y, x]] as f64).collect();
        let chi2_median = median(&mut chi2);
        let population: Vec<(usize, usize)> = counts.strong.iter().copied().filter(|&(x, y)| (maps.chi2_red[[y, x]] as f64) < chi2_median).collect();
        let moments = moment_maps(&set.sci, &moment_cfg(&cfg)).unwrap();
        let mut deltas: Vec<f64> = Vec::new();
        let mut signed: Vec<f64> = Vec::new();
        let mut no_m1 = 0usize;
        for &(x, y) in &population {
            let m1 = moments.m1[[y, x]] as f64;
            if !m1.is_finite() {
                no_m1 += 1;
                continue;
            }
            let d = maps.velocity[[y, x]] as f64 - m1;
            deltas.push(d.abs());
            signed.push(d);
        }
        let within = deltas.iter().filter(|d| **d < 63.0).count() as f64 / deltas.len() as f64;
        let median_abs = median(&mut deltas);
        let median_signed = median(&mut signed);
        println!(
            "Pa alpha vs M1: population {} (chi2_red median {:.2}, {} without M1), median |V-M1| {:.1} km/s (bound 30), signed median {:.1}, within 63 km/s {:.3} (bound 0.90), elapsed {} ms (bound 5000)",
            population.len(), chi2_median, no_m1, median_abs, median_signed, within, response.elapsed_ms
        );
        assert!(median_abs < 30.0, "median |V - M1| {}", median_abs);
        assert!(within >= 0.90, "within 63 km/s {}", within);
        assert!(response.elapsed_ms < 5000, "elapsed {} ms", response.elapsed_ms);
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    #[ignore]
    fn nirspec_pa_alpha_and_br_beta_velocities_agree() {
        let Some(key) = nirspec_key(G235H) else { return };
        let pa_cfg = nirspec_cfg(PA_ALPHA_WINDOW, PA_ALPHA_CONTINUUM, PA_ALPHA_UM);
        let br_cfg = nirspec_cfg(BR_BETA_WINDOW, BR_BETA_CONTINUUM, BR_BETA_UM);
        let set = open_cube_set(&key).unwrap();
        let pa = fit_cube_lines(&set.inputs(), &pa_cfg).unwrap();
        let br = fit_cube_lines(&set.inputs(), &br_cfg).unwrap();
        let pa_counts = count_map(&pa, 10.0);
        let br_counts = count_map(&br, 10.0);
        let mut deltas = Vec::new();
        let mut signed = Vec::new();
        for &(x, y) in &pa_counts.strong {
            if !br_counts.strong.contains(&(x, y)) {
                continue;
            }
            let d = pa.velocity[[y, x]] as f64 - br.velocity[[y, x]] as f64;
            deltas.push(d.abs());
            signed.push(d);
        }
        let median_abs = median(&mut deltas);
        let median_signed = median(&mut signed);
        println!(
            "Pa alpha vs Br beta: {} Pa alpha strong, {} Br beta strong, {} pairs, median |dV| {:.1} km/s (bound 20), signed median {:.1} km/s",
            pa_counts.strong.len(), br_counts.strong.len(), deltas.len(), median_abs, median_signed
        );
        assert!(deltas.len() >= 500, "pairs {}", deltas.len());
        assert!(median_abs < 20.0, "median |dV| {}", median_abs);
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }

    #[test]
    #[ignore]
    fn nirspec_br_alpha_counts_gap_fallbacks_and_resolves_the_blue_component() {
        let Some(key) = nirspec_key(G395H) else { return };
        let dir = tempfile::tempdir().unwrap();
        let out = out_dir(&dir);
        let cfg = nirspec_cfg(BR_ALPHA_WINDOW, BR_ALPHA_CONTINUUM, BR_ALPHA_UM);
        let set = open_cube_set(&key).unwrap();
        let response = line_fit_files(&set, &key, &out, &cfg, Instant::now()).unwrap();
        let maps = fit_cube_lines(&set.inputs(), &cfg).unwrap();
        let counts = count_map(&maps, 10.0);
        println!(
            "Br alpha: converged {} (floor 650), n_fit {} (floor 550), bit 1 & snr>=10 {} (floor 250), n_const_continuum {}, median chi2_red {:?}, elapsed {} ms",
            counts.converged, maps.n_fit, counts.strong.len(), maps.n_const_continuum, maps.median_chi2_red, response.elapsed_ms
        );
        assert!(counts.converged >= 650, "converged {}", counts.converged);
        assert!(maps.n_fit >= 550, "n_fit {}", maps.n_fit);
        assert!(counts.strong.len() >= 250, "strong {}", counts.strong.len());
        let moments = moment_maps(&set.sci, &moment_cfg(&cfg)).unwrap();
        let mut v = Vec::new();
        let mut m1 = Vec::new();
        for &(x, y) in &counts.strong {
            let m = moments.m1[[y, x]] as f64;
            if m.is_finite() {
                v.push(maps.velocity[[y, x]] as f64);
                m1.push(m);
            }
        }
        let (v5, v95) = (percentile(&mut v, 0.05), percentile(&mut v, 0.95));
        let (m5, m95) = (percentile(&mut m1, 0.05), percentile(&mut m1, 0.95));
        println!("Br alpha V p5/p95 {:.1}/{:.1} vs M1 {:.1}/{:.1} km/s over {} spaxels (bounds 60/40)", v5, v95, m5, m95, v.len());
        assert!((v5 - m5).abs() < 60.0, "p5 {} vs {}", v5, m5);
        assert!((v95 - m95).abs() < 40.0, "p95 {} vs {}", v95, m95);
        assert!(maps.n_const_continuum > 0);
        let (rows, cols) = maps.mask.dim();
        let mut constant = 0usize;
        let mut constant_fitted = 0usize;
        for y in 0..rows {
            for x in 0..cols {
                let m = maps.mask[[y, x]];
                if !bit(m, MASK_CONST_CONTINUUM) {
                    continue;
                }
                constant += 1;
                if bit(m, MASK_FITTED) {
                    constant_fitted += 1;
                    assert!(maps.velocity[[y, x]].is_finite(), "({}, {}) fitted with a constant continuum but NaN velocity", x, y);
                }
            }
        }
        println!("Br alpha gap: {} constant-continuum spaxels, {} of them fitted (bound 80 %)", constant, constant_fitted);
        assert!(constant_fitted as f64 >= 0.8 * constant as f64, "{} of {}", constant_fitted, constant);

        let auto_cfg = LineFitConfig { components: Components::Auto, ..cfg.clone() };
        let auto_response = line_fit_files(&set, &key, &out, &auto_cfg, Instant::now()).unwrap();
        assert_eq!(auto_response.plane_order.len(), 22);
        assert_eq!(auto_response.components, Components::Auto);
        let auto_maps = fit_cube_lines(&set.inputs(), &auto_cfg).unwrap();
        let planes = auto_maps.components.as_ref().expect("component planes");
        let mut c1_velocities = Vec::new();
        let mut ordered = 0usize;
        let mut split = 0usize;
        for y in 0..rows {
            for x in 0..cols {
                let p = [y, x];
                if auto_maps.ncomp[p] != 2.0 {
                    assert!(planes[0].velocity[p].is_nan(), "c1_velocity({}, {}) set on a single-component spaxel", x, y);
                    continue;
                }
                split += 1;
                assert!(bit(auto_maps.mask[p], MASK_TWO_COMPONENTS), "mask({}, {})={}", x, y, auto_maps.mask[p]);
                assert_eq!(auto_maps.velocity[p].to_bits(), maps.velocity[p].to_bits(), "main velocity changed at ({}, {})", x, y);
                let (v1, v, v2) = (planes[0].velocity[p] as f64, auto_maps.velocity[p] as f64, planes[1].velocity[p] as f64);
                assert!(v1 < v2, "({}, {}) c1 {} c2 {}", x, y, v1, v2);
                c1_velocities.push(v1);
                if v1 < v && v < v2 {
                    ordered += 1;
                }
            }
        }
        let c1_median = median(&mut c1_velocities);
        let ordered_fraction = ordered as f64 / split.max(1) as f64;
        println!(
            "Br alpha auto: n_two_components {} (floor 200), median c1 velocity {:.1} km/s (bounds -320..-180), c1 < V < c2 on {} of {} = {:.3} (bound 0.97), elapsed {} ms (bound 15000)",
            auto_maps.n_two_components, c1_median, ordered, split, ordered_fraction, auto_response.elapsed_ms
        );
        assert_eq!(auto_maps.n_two_components, split);
        assert!(auto_maps.n_two_components >= 200, "n_two_components {}", auto_maps.n_two_components);
        assert!((-320.0..=-180.0).contains(&c1_median), "median c1 velocity {}", c1_median);
        assert!(ordered_fraction >= 0.97, "ordered {}", ordered_fraction);
        assert!(auto_response.elapsed_ms < 15000, "elapsed {} ms", auto_response.elapsed_ms);
        assert!(auto_response.notes.iter().any(|n| n.starts_with(&format!("two components: {} spaxels accepted (ΔBIC", split))), "{:?}", auto_response.notes);
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }
}
