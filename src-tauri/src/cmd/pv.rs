use std::collections::HashSet;
use std::time::Instant;

use serde::Deserialize;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, output_stem, render_named_and_save, resolve_output_dir, write_derived_fits,
    OutputValues,
};
use crate::core::astrometry::spectral::VelocityConvention;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::cube::lazy::LazyCube;
use crate::core::cube::pv::{pv_diagram, PvConfig, PvDiagram, PvSpectralMode};
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_NOTES, RES_OFFSETS, RES_OFFSET_UNIT, RES_PNG_PATH,
    RES_SPECTRAL_UNIT, RES_SPECTRAL_VALUES, RES_SUMMARY,
};
use crate::types::header::HduHeader;

const MAX_PV_CHANNEL: usize = 1 << 24;
const MIN_STEP_PX: f64 = 0.05;
const MAX_STEP_PX: f64 = 1024.0;
const MAX_WIDTH_PX: f64 = 4096.0;
const DEFAULT_MODE: &str = "wavelength_vac";
const DEFAULT_CONVENTION: &str = "optical";
const ABPROC_PV: &str = "pv";
const FITS_ORIGIN_OFFSET: f64 = 1.0;
const TEXT_CARDS_COPIED: [&str; 5] = ["SPECSYS", "OBJECT", "TELESCOP", "INSTRUME", "DATE-OBS"];
const NUMERIC_CARDS_COPIED: [&str; 3] = ["RESTWAV", "RESTFRQ", "VELOSYS"];
const CARD_BUNIT: &str = "BUNIT";
const CARD_WCSAXES: &str = "WCSAXES";
const CARD_CTYPE1: &str = "CTYPE1";
const CARD_CUNIT1: &str = "CUNIT1";
const CARD_CRPIX1: &str = "CRPIX1";
const CARD_CRVAL1: &str = "CRVAL1";
const CARD_CDELT1: &str = "CDELT1";
const CARD_CTYPE2: &str = "CTYPE2";
const CARD_CUNIT2: &str = "CUNIT2";
const CARD_CRPIX2: &str = "CRPIX2";
const CARD_CRVAL2: &str = "CRVAL2";
const CARD_CDELT2: &str = "CDELT2";
const CARD_OFFSET_CTYPE: &str = "OFFSET";
const CARD_PVLINEX0: &str = "PVLINEX0";
const CARD_PVLINEY0: &str = "PVLINEY0";
const CARD_PVLINEX1: &str = "PVLINEX1";
const CARD_PVLINEY1: &str = "PVLINEY1";
const CARD_PVLINEST: &str = "PVLINEST";
const CARD_PVLINEWD: &str = "PVLINEWD";
const CARD_PVLINEZ0: &str = "PVLINEZ0";
const CARD_PVLINEZ1: &str = "PVLINEZ1";
const CARD_PVLINENA: &str = "PVLINENA";

const RES_RIDGE: &str = "ridge";
const RES_RIDGE_CHANNEL: &str = "ridge_channel";
const RES_PEAK_CHANNEL: &str = "peak_channel";
const RES_PEAK_VALUE: &str = "peak_value";
const RES_PEAK_SPECTRAL: &str = "peak_spectral";
const RES_OFFSET_STEP: &str = "offset_step";
const RES_SPECTRAL_MODE: &str = "spectral_mode";
const RES_SPECTRAL_SHIFT_KMS: &str = "spectral_shift_kms";
const RES_SPECTRAL_REST_UM: &str = "spectral_rest_um";
const RES_SPECTRAL_CONVENTION: &str = "spectral_convention";
const RES_LINE: &str = "line";
const RES_STEP_PX: &str = "step_px";
const RES_WIDTH_PX: &str = "width_px";
const RES_Z0: &str = "z0";
const RES_Z1: &str = "z1";
const RES_N_ACROSS: &str = "n_across";
const RES_XS: &str = "xs";
const RES_YS: &str = "ys";
const RES_X0: &str = "x0";
const RES_Y0: &str = "y0";
const RES_X1: &str = "x1";
const RES_Y1: &str = "y1";

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PvLine {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

pub(crate) struct PvFiles {
    pub png_path: String,
    pub fits_path: String,
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn set_text(header: &mut HduHeader, text_keys: &mut HashSet<String>, key: &str, value: String) {
    header.set(key, value);
    text_keys.insert(key.to_string());
}

pub(crate) fn pv_header(cube: &LazyCube, cfg: &PvConfig, pv: &PvDiagram) -> HduHeader {
    let mut header = derived_output_header(None, ABPROC_PV, OutputValues::Linear);
    let mut text_keys: HashSet<String> = HashSet::new();
    header.set(CARD_WCSAXES, "2".to_string());
    set_text(&mut header, &mut text_keys, CARD_CTYPE1, CARD_OFFSET_CTYPE.to_string());
    set_text(&mut header, &mut text_keys, CARD_CUNIT1, pv.offset_unit.to_string());
    header.set_f64(CARD_CRPIX1, (pv.summary.n_offsets as f64 + 1.0) / 2.0);
    header.set_f64(CARD_CRVAL1, 0.0);
    header.set_f64(CARD_CDELT1, pv.offset_step);
    if let Some(ctype) = &pv.native.ctype {
        set_text(&mut header, &mut text_keys, CARD_CTYPE2, ctype.clone());
    }
    if let Some(cunit) = pv.native.cunit.as_ref().filter(|u| !u.is_empty()) {
        set_text(&mut header, &mut text_keys, CARD_CUNIT2, cunit.clone());
    }
    header.set_f64(CARD_CRPIX2, 1.0);
    header.set_f64(CARD_CRVAL2, pv.native.crval);
    header.set_f64(CARD_CDELT2, pv.native.cdelt);
    if let Some(bunit) = card_text(&cube.header, CARD_BUNIT) {
        set_text(&mut header, &mut text_keys, CARD_BUNIT, bunit);
    }
    for key in TEXT_CARDS_COPIED {
        if let Some(value) = card_text(&cube.header, key) {
            set_text(&mut header, &mut text_keys, key, value);
        }
    }
    for key in NUMERIC_CARDS_COPIED {
        if let Some(value) = cube.header.get_f64(key).filter(|v| v.is_finite()) {
            header.set_f64(key, value);
        }
    }
    header.set_f64(CARD_PVLINEX0, cfg.x0 + FITS_ORIGIN_OFFSET);
    header.set_f64(CARD_PVLINEY0, cfg.y0 + FITS_ORIGIN_OFFSET);
    header.set_f64(CARD_PVLINEX1, cfg.x1 + FITS_ORIGIN_OFFSET);
    header.set_f64(CARD_PVLINEY1, cfg.y1 + FITS_ORIGIN_OFFSET);
    header.set_f64(CARD_PVLINEST, cfg.step_px);
    header.set_f64(CARD_PVLINEWD, cfg.width_px);
    header.set(CARD_PVLINEZ0, cfg.z0.to_string());
    header.set(CARD_PVLINEZ1, cfg.z1.to_string());
    header.set(CARD_PVLINENA, pv.n_across.to_string());
    header.string_keys = Some(text_keys);
    header
}

fn rounded_endpoint(value: f64) -> i64 {
    value.round() as i64
}

pub(crate) fn pv_diagram_files(
    cube: &LazyCube,
    path: &str,
    output_dir: &str,
    cfg: &PvConfig,
) -> anyhow::Result<(PvDiagram, PvFiles)> {
    let pv = pv_diagram(cube, cfg)?;
    let name = format!(
        "{}_pv_{}-{}_{}-{}_{}-{}",
        output_stem(path),
        cfg.z0,
        cfg.z1,
        rounded_endpoint(cfg.x0),
        rounded_endpoint(cfg.y0),
        rounded_endpoint(cfg.x1),
        rounded_endpoint(cfg.y1)
    );
    let (png_path, _) = render_named_and_save(&pv.data, output_dir, &name, false, None)?;
    let fits_path = format!("{}/{}.fits", output_dir, name);
    write_derived_fits(&fits_path, &pv.data, Some(&pv_header(cube, cfg, &pv)))?;
    Ok((pv, PvFiles { png_path, fits_path }))
}

fn nullable(values: &[f64]) -> Vec<Option<f64>> {
    values.iter().map(|v| v.is_finite().then_some(*v)).collect()
}

fn pv_diagram_json(path: &str, output_dir: &str, cfg: &PvConfig, t0: Instant) -> anyhow::Result<serde_json::Value> {
    let out_dir = resolve_output_dir(output_dir)?;
    let cube = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    let (pv, files) = pv_diagram_files(&cube, path, &out_dir, cfg)?;
    let peak_spectral: Vec<Option<f64>> = pv
        .peak_channel
        .iter()
        .map(|peak| {
            peak.and_then(|z| z.checked_sub(cfg.z0))
                .and_then(|k| pv.spectral_values.get(k).copied())
                .filter(|v| v.is_finite())
        })
        .collect();
    Ok(json!({
        RES_FITS_PATH: files.fits_path,
        RES_PNG_PATH: files.png_path,
        RES_DIMENSIONS: [pv.summary.n_offsets, pv.summary.n_channels],
        RES_OFFSETS: pv.offsets,
        RES_OFFSET_UNIT: pv.offset_unit,
        RES_OFFSET_STEP: pv.offset_step,
        RES_SPECTRAL_VALUES: pv.spectral_values,
        RES_SPECTRAL_UNIT: pv.spectral_unit,
        RES_SPECTRAL_MODE: cfg.mode.name(),
        RES_SPECTRAL_SHIFT_KMS: cfg.velocity_shift_kms,
        RES_SPECTRAL_REST_UM: pv.rest_um,
        RES_SPECTRAL_CONVENTION: pv.convention_applies.then(|| cfg.convention.name()),
        RES_RIDGE: nullable(&pv.ridge),
        RES_RIDGE_CHANNEL: nullable(&pv.ridge_channel),
        RES_PEAK_CHANNEL: pv.peak_channel,
        RES_PEAK_VALUE: nullable(&pv.peak_value),
        RES_PEAK_SPECTRAL: peak_spectral,
        RES_XS: pv.xs,
        RES_YS: pv.ys,
        RES_LINE: { RES_X0: cfg.x0, RES_Y0: cfg.y0, RES_X1: cfg.x1, RES_Y1: cfg.y1 },
        RES_STEP_PX: cfg.step_px,
        RES_WIDTH_PX: cfg.width_px,
        RES_Z0: cfg.z0,
        RES_Z1: cfg.z1,
        RES_N_ACROSS: pv.n_across,
        RES_SUMMARY: serde_json::to_value(&pv.summary)?,
        RES_NOTES: pv.notes,
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
    }))
}

#[tauri::command]
pub async fn pv_diagram_cmd(
    path: String,
    output_dir: String,
    line: PvLine,
    step_px: f64,
    width_px: f64,
    z0: usize,
    z1: usize,
    mode: Option<String>,
    rest_um: Option<f64>,
    convention: Option<String>,
    velocity_shift_kms: Option<f64>,
) -> Result<serde_json::Value, String> {
    let t0 = Instant::now();
    if path.trim().is_empty() {
        return Err("path must name a cube file, got an empty string".to_string());
    }
    if ![line.x0, line.y0, line.x1, line.y1].iter().all(|v| v.is_finite()) {
        return Err(format!(
            "line endpoints must be finite, got ({}, {}) -> ({}, {})",
            line.x0, line.y0, line.x1, line.y1
        ));
    }
    if !(step_px.is_finite() && (MIN_STEP_PX..=MAX_STEP_PX).contains(&step_px)) {
        return Err(format!("step_px must be between {} and {} pixels, got {}", MIN_STEP_PX, MAX_STEP_PX, step_px));
    }
    if !(width_px.is_finite() && (0.0..=MAX_WIDTH_PX).contains(&width_px)) {
        return Err(format!("width_px must be between 0 and {} pixels, got {}", MAX_WIDTH_PX, width_px));
    }
    if z0 > z1 || z1 >= MAX_PV_CHANNEL {
        return Err(format!(
            "channel range z0..=z1 must satisfy z0 <= z1 < {}, got {}..={}",
            MAX_PV_CHANNEL, z0, z1
        ));
    }
    if let Some(rest) = rest_um {
        if !(rest.is_finite() && rest > 0.0) {
            return Err(format!("rest_um must be a positive number of micrometres, got {}", rest));
        }
    }
    if let Some(shift) = velocity_shift_kms {
        if !shift.is_finite() {
            return Err(format!("velocity_shift_kms must be a finite number of km/s, got {}", shift));
        }
    }
    let mode = PvSpectralMode::parse(mode.as_deref().unwrap_or(DEFAULT_MODE))?;
    let convention = VelocityConvention::parse(convention.as_deref().unwrap_or(DEFAULT_CONVENTION))?;
    let cfg = PvConfig {
        x0: line.x0,
        y0: line.y0,
        x1: line.x1,
        y1: line.y1,
        z0,
        z1,
        step_px,
        width_px,
        mode,
        rest_um,
        convention,
        velocity_shift_kms: velocity_shift_kms.unwrap_or(0.0),
    };
    blocking_cmd!(pv_diagram_json(&path, &output_dir, &cfg, t0))
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;
    use crate::cmd::common::HEADER_ABPROC;
    use crate::core::astrometry::wcs::WcsTransform;
    use crate::core::cube::lazy::test_support::*;
    use crate::infra::fits::reader::extract_image_mmap;

    const SLIT_H: PvLine = PvLine { x0: 2.0, y0: 16.0, x1: 29.0, y1: 16.0 };
    const SLIT_V: PvLine = PvLine { x0: 16.0, y0: 2.0, x1: 16.0, y1: 29.0 };

    fn config(line: PvLine, z0: usize, z1: usize) -> PvConfig {
        PvConfig {
            x0: line.x0,
            y0: line.y0,
            x1: line.x1,
            y1: line.y1,
            z0,
            z1,
            step_px: 1.0,
            width_px: 1.0,
            mode: PvSpectralMode::Velocity,
            rest_um: None,
            convention: VelocityConvention::Optical,
            velocity_shift_kms: 0.0,
        }
    }

    fn reopen(path: &str) -> crate::infra::fits::reader::MmapImageResult {
        extract_image_mmap(&File::open(path).unwrap()).unwrap()
    }

    fn output_dir(dir: &tempfile::TempDir) -> String {
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        out.to_str().unwrap().to_string()
    }

    #[test]
    fn the_pv_fits_reopens_as_a_mono_image_with_offset_and_native_spectral_axes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.fits");
        write_line_cube(&path, 0.0);
        let key = path.to_str().unwrap().to_string();
        let cube = LazyCube::open(&key).unwrap();
        let out = output_dir(&dir);
        let (pv, files) = pv_diagram_files(&cube, &key, &out, &config(SLIT_H, 5, 35)).unwrap();
        assert_eq!(pv.summary.n_offsets, 28);
        assert!(std::path::Path::new(&files.png_path).exists());

        let reopened = reopen(&files.fits_path);
        let h = &reopened.header;
        assert_eq!(reopened.image.dim(), (31, 28));
        assert_eq!(h.get_i64("NAXIS"), Some(2));
        assert_eq!(h.get_i64("WCSAXES"), Some(2));
        assert_eq!(h.get("CTYPE1"), Some("OFFSET"));
        assert_eq!(h.get("CUNIT1"), Some("arcsec"));
        assert!((h.get_f64("CRPIX1").unwrap() - 14.5).abs() < 1e-12);
        assert_eq!(h.get_f64("CRVAL1"), Some(0.0));
        assert!((h.get_f64("CDELT1").unwrap() - 0.36).abs() < 1e-9);
        assert_eq!(h.get("CTYPE2"), Some("WAVE"));
        assert_eq!(h.get("CUNIT2"), Some("um"));
        assert_eq!(h.get_f64("CRPIX2"), Some(1.0));
        assert!((h.get_f64("CRVAL2").unwrap() - 1.005).abs() < 1e-9);
        assert!((h.get_f64("CDELT2").unwrap() - 0.001).abs() < 1e-12);
        assert_eq!(h.get("BUNIT"), Some("Jy/beam"));
        assert_eq!(h.get(HEADER_ABPROC), Some("pv"));
        assert_eq!(h.get("SPECSYS"), Some("BARYCENT"));
        assert!((h.get_f64("RESTWAV").unwrap() - 1.02e-6).abs() < 1e-18);
        assert_eq!(h.get_f64("PVLINEX0"), Some(3.0));
        assert_eq!(h.get_f64("PVLINEY0"), Some(17.0));
        assert_eq!(h.get_f64("PVLINEX1"), Some(30.0));
        assert_eq!(h.get_f64("PVLINEY1"), Some(17.0));
        assert_eq!(h.get_f64("PVLINEST"), Some(1.0));
        assert_eq!(h.get_f64("PVLINEWD"), Some(1.0));
        assert_eq!(h.get_i64("PVLINEZ0"), Some(5));
        assert_eq!(h.get_i64("PVLINEZ1"), Some(35));
        assert_eq!(h.get_i64("PVLINENA"), Some(1));
        for key in ["NAXIS3", "CTYPE3", "CRVAL3", "CD1_1", "CD3_3", "RADESYS"] {
            assert!(h.get(key).is_none(), "{} survived", key);
        }
        assert!(WcsTransform::from_header(h).is_err());
        assert_eq!(crate::cmd::common::load_cached(&files.fits_path).unwrap().arr().dim(), (31, 28));
    }

    #[test]
    fn the_json_carries_the_ridge_and_summary_but_not_the_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("json.fits");
        write_line_cube(&path, 0.0);
        let key = path.to_str().unwrap().to_string();
        let out = output_dir(&dir);
        let value = pv_diagram_json(&key, &out, &config(SLIT_H, 5, 35), Instant::now()).unwrap();
        for k in [
            RES_FITS_PATH,
            RES_PNG_PATH,
            RES_DIMENSIONS,
            RES_OFFSETS,
            RES_OFFSET_UNIT,
            RES_SPECTRAL_VALUES,
            RES_SPECTRAL_UNIT,
            RES_RIDGE,
            RES_RIDGE_CHANNEL,
            RES_PEAK_CHANNEL,
            RES_SUMMARY,
            RES_NOTES,
            RES_ELAPSED_MS,
        ] {
            assert!(value.get(k).is_some(), "missing {}", k);
        }
        assert!(value.get("data").is_none());
        assert_eq!(value[RES_OFFSETS].as_array().unwrap().len(), 28);
        assert_eq!(value[RES_RIDGE].as_array().unwrap().len(), 28);
        assert_eq!(value[RES_SPECTRAL_VALUES].as_array().unwrap().len(), 31);
        assert_eq!(value[RES_SUMMARY]["n_channels"], 31);
        assert_eq!(value[RES_DIMENSIONS], json!([28, 31]));
        assert_eq!(value[RES_SPECTRAL_CONVENTION], "optical");
        assert_eq!(value[RES_SPECTRAL_MODE], "velocity");
        GLOBAL_CUBE_CACHE.invalidate(&key);

        let vrad = dir.path().join("vrad.fits");
        write_line_cube_with_cards(&vrad, &[("CTYPE3", "'VRAD'"), ("CUNIT3", "'km/s'")]);
        let vrad_key = vrad.to_str().unwrap().to_string();
        let value = pv_diagram_json(&vrad_key, &out, &config(SLIT_H, 5, 35), Instant::now()).unwrap();
        assert!(value[RES_SPECTRAL_CONVENTION].is_null());
        assert_eq!(value[RES_SPECTRAL_UNIT], "km/s");
        GLOBAL_CUBE_CACHE.invalidate(&vrad_key);
    }

    #[tokio::test]
    async fn the_command_rejects_bad_parameters_before_opening_the_cube() {
        let out = "C:/nowhere/pv".to_string();
        let run = |path: &str, line: PvLine, step: f64, width: f64, z0: usize, z1: usize, mode: Option<&str>, rest: Option<f64>, conv: Option<&str>, shift: Option<f64>| {
            pv_diagram_cmd(
                path.to_string(),
                out.clone(),
                line,
                step,
                width,
                z0,
                z1,
                mode.map(str::to_string),
                rest,
                conv.map(str::to_string),
                shift,
            )
        };
        let missing = "C:/nowhere/missing_cube.fits";
        let err = run("", SLIT_H, 1.0, 1.0, 0, 39, None, None, None, None).await.unwrap_err();
        assert!(err.contains("path must name a cube file, got an empty string"), "{}", err);
        let nan = PvLine { x0: f64::NAN, y0: 16.0, x1: 29.0, y1: 16.0 };
        let err = run(missing, nan, 1.0, 1.0, 0, 39, None, None, None, None).await.unwrap_err();
        assert!(err.contains("line endpoints must be finite, got (NaN, 16) -> (29, 16)"), "{}", err);
        let err = run(missing, SLIT_H, 0.01, 1.0, 0, 39, None, None, None, None).await.unwrap_err();
        assert!(err.contains("step_px must be between 0.05 and 1024 pixels, got 0.01"), "{}", err);
        let err = run(missing, SLIT_H, 2000.0, 1.0, 0, 39, None, None, None, None).await.unwrap_err();
        assert!(err.contains("step_px must be between 0.05 and 1024 pixels, got 2000"), "{}", err);
        let err = run(missing, SLIT_H, 1.0, -1.0, 0, 39, None, None, None, None).await.unwrap_err();
        assert!(err.contains("width_px must be between 0 and 4096 pixels, got -1"), "{}", err);
        let err = run(missing, SLIT_H, 1.0, 5000.0, 0, 39, None, None, None, None).await.unwrap_err();
        assert!(err.contains("width_px must be between 0 and 4096 pixels, got 5000"), "{}", err);
        let err = run(missing, SLIT_H, 1.0, 1.0, 10, 5, None, None, None, None).await.unwrap_err();
        assert!(err.contains("channel range z0..=z1 must satisfy z0 <= z1 < 16777216, got 10..=5"), "{}", err);
        let err = run(missing, SLIT_H, 1.0, 1.0, 0, 39, None, Some(0.0), None, None).await.unwrap_err();
        assert!(err.contains("rest_um must be a positive number of micrometres, got 0"), "{}", err);
        let err = run(missing, SLIT_H, 1.0, 1.0, 0, 39, None, None, None, Some(f64::NAN)).await.unwrap_err();
        assert!(err.contains("velocity_shift_kms must be a finite number of km/s, got NaN"), "{}", err);
        let err = run(missing, SLIT_H, 1.0, 1.0, 0, 39, Some("foo"), None, None, None).await.unwrap_err();
        assert!(err.contains("unknown spectral mode 'foo': use wavelength_vac, wavelength_air, frequency or velocity"), "{}", err);
        let err = run(missing, SLIT_H, 1.0, 1.0, 0, 39, None, None, Some("foo"), None).await.unwrap_err();
        assert!(err.contains("unknown velocity convention 'foo'"), "{}", err);
        let err = run(missing, SLIT_H, 1.0, 1.0, 0, 39, None, None, None, None).await.unwrap_err();
        assert!(err.contains("missing_cube.fits"), "{}", err);
        assert!(!err.contains("must"), "{}", err);
    }

    #[test]
    fn the_plane_ref_the_frontend_sends_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pvref.fits");
        write_line_cube(&path, 0.0);
        let key = format!("{}#hdu=0", path.to_str().unwrap());
        let out = output_dir(&dir);
        let first = pv_diagram_json(&key, &out, &config(SLIT_H, 0, 39), Instant::now()).unwrap();
        let fits = first[RES_FITS_PATH].as_str().unwrap();
        assert!(fits.ends_with("_hdu0_pv_0-39_2-16_29-16.fits"), "{}", fits);
        assert!(std::path::Path::new(fits).exists());
        let second = pv_diagram_json(&key, &out, &config(SLIT_V, 0, 39), Instant::now()).unwrap();
        let fits = second[RES_FITS_PATH].as_str().unwrap();
        assert!(fits.ends_with("pvref_hdu0_pv_0-39_16-2_16-29.fits"), "{}", fits);
        GLOBAL_CUBE_CACHE.invalidate(&key);
    }
}
