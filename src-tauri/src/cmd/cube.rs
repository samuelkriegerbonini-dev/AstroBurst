use std::time::Instant;

use ndarray::Array2;
use serde::Serialize;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, output_stem, render_asinh_and_save, resolve_output_dir};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::cube::eager::classify_spectral_cube;
use crate::core::cube::eager::{process_cube, build_wavelength_axis};
use crate::core::cube::lazy::{process_cube_lazy, CollapseMode, LazyCube};
use crate::core::cube::moments::{moment_maps, MomentConfig};
use crate::core::imaging::region::RegionShape;
use crate::core::imaging::stats::percentile;
use crate::core::metadata::photcal::{FluxConvention, PhotCal, JY_PER_MJY};
use crate::infra::fits::writer::write_fits_mono;
use crate::infra::render::grayscale::render_grayscale;
use crate::types::constants::{
    RES_BITPIX, RES_FITS_PATH, RES_FRAME_INDEX, RES_FRAMES, RES_HEIGHT,
    RES_OUTPUT_PATH, RES_SPECTRUM, RES_WIDTH,
    RES_SPECTRAL_CLASSIFICATION, RES_IS_SPECTRAL, RES_SPECTRAL_REASON,
    RES_AXIS_TYPE, RES_AXIS_UNIT, RES_CHANNEL_COUNT, RES_WAVELENGTHS,
};
use crate::types::header::HduHeader;

pub const KEY_SPECTRAL_AXIS: &str = "spectral_axis";
pub const APERTURE_SUBSAMPLES: u8 = 5;

const PREVIEW_LOW_PERCENTILE: f64 = 0.01;
const PREVIEW_HIGH_PERCENTILE: f64 = 0.99;
const AXIS3_KEYS: [&str; 10] = [
    "NAXIS3", "CTYPE3", "CRVAL3", "CDELT3", "CRPIX3", "CUNIT3", "CNAME3", "CRDER3", "CSYER3", "CROTA3",
];

fn is_axis3_matrix_key(key: &str) -> bool {
    let Some(body) = key.strip_prefix("CD").or_else(|| key.strip_prefix("PC")) else {
        return false;
    };
    body.starts_with("3_") || body.ends_with("_3")
}

pub fn strip_spectral_axis(header: &HduHeader) -> HduHeader {
    let mut out = header.clone();
    let keys: Vec<String> = out.cards.iter().map(|(k, _)| k.clone()).collect();
    for key in keys {
        let upper = key.trim().to_uppercase();
        if AXIS3_KEYS.contains(&upper.as_str()) || is_axis3_matrix_key(&upper) {
            out.remove(&key);
        }
    }
    if out.get("NAXIS").is_some() {
        out.set("NAXIS", "2".to_string());
    }
    if out.get("WCSAXES").is_some() || out.get("CTYPE1").is_some() {
        out.set("WCSAXES", "2".to_string());
    }
    out
}

pub fn linear_preview(arr: &Array2<f32>) -> Array2<f32> {
    let mut finite: Vec<f32> = arr.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return Array2::zeros(arr.dim());
    }
    let mut lo = percentile(&mut finite, PREVIEW_LOW_PERCENTILE);
    let mut hi = percentile(&mut finite, PREVIEW_HIGH_PERCENTILE);
    if hi <= lo {
        lo = finite.iter().copied().fold(f32::INFINITY, f32::min);
        hi = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    }
    let range = (hi - lo).max(1e-10);
    arr.mapv(|v| if v.is_finite() { ((v - lo) / range).clamp(0.0, 1.0) } else { 0.0 })
}

fn mjy_per_sr_to_jy_factor(header: &HduHeader) -> Option<f64> {
    let wcs = WcsTransform::from_header(header).ok();
    let cal = PhotCal::from_header(header, wcs.as_ref())?;
    match cal.convention {
        FluxConvention::JwstMjySr { pixar_sr, .. } => Some(pixar_sr * JY_PER_MJY),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RegionSpectrumResponse {
    pub sum: Vec<f32>,
    pub mean: Vec<f32>,
    pub npix: f64,
    pub n_bg: usize,
    pub bg_per_pixel: Option<Vec<f32>>,
    pub wavelengths: Option<Vec<f64>>,
    pub unit: String,
    pub bg_subtracted: bool,
    pub flux_jy: Option<Vec<f64>>,
    pub elapsed_ms: u64,
}

pub(crate) fn region_spectrum_response(
    cube: &LazyCube,
    shape: &RegionShape,
    background: Option<&RegionShape>,
    t0: Instant,
) -> anyhow::Result<RegionSpectrumResponse> {
    let spectrum = cube.extract_spectrum_aperture(shape, background, APERTURE_SUBSAMPLES)?;
    let axis = cube.spectral_axis().ok();
    let flux_jy = mjy_per_sr_to_jy_factor(&cube.header)
        .map(|factor| spectrum.sum.iter().map(|&s| s as f64 * factor).collect());
    Ok(RegionSpectrumResponse {
        sum: spectrum.sum,
        mean: spectrum.mean,
        npix: spectrum.npix,
        n_bg: spectrum.n_bg,
        bg_subtracted: spectrum.bg_per_pixel.is_some(),
        bg_per_pixel: spectrum.bg_per_pixel,
        wavelengths: axis.as_ref().map(|a| a.values.clone()),
        unit: axis.map(|a| a.unit).unwrap_or_default(),
        flux_jy,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct CollapseRangeResponse {
    pub png_path: String,
    pub fits_path: String,
    pub z0: usize,
    pub z1: usize,
    pub mode: &'static str,
    pub dimensions: [usize; 2],
    pub elapsed_ms: u64,
}

pub(crate) fn collapse_range_files(
    cube: &LazyCube,
    path: &str,
    output_dir: &str,
    z0: usize,
    z1: usize,
    mode: CollapseMode,
    t0: Instant,
) -> anyhow::Result<CollapseRangeResponse> {
    let arr = cube.collapse_range(z0, z1, mode)?;
    let name = format!("{}_collapse_{}_{}-{}", output_stem(path), mode.name(), z0, z1);
    let (png_path, _) = render_asinh_and_save(&arr, output_dir, &name, false)?;
    let fits_path = format!("{}/{}.fits", output_dir, name);
    let header = strip_spectral_axis(&cube.header);
    write_fits_mono(&fits_path, &arr, Some(&header))?;
    let (rows, cols) = arr.dim();
    Ok(CollapseRangeResponse {
        png_path,
        fits_path,
        z0,
        z1,
        mode: mode.name(),
        dimensions: [cols, rows],
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct MapFiles {
    pub png_path: String,
    pub fits_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MomentMapsResponse {
    pub m0: MapFiles,
    pub m1: MapFiles,
    pub m2: MapFiles,
    pub m0_unit: String,
    pub velocity_unit: &'static str,
    pub noise_per_channel: Option<f64>,
    pub n_channels: usize,
    pub notes: Vec<String>,
    pub dimensions: [usize; 2],
    pub elapsed_ms: u64,
}

fn save_map(
    arr: &Array2<f32>,
    header_2d: &HduHeader,
    unit: &str,
    output_dir: &str,
    name: &str,
) -> anyhow::Result<MapFiles> {
    let png_path = format!("{}/{}.png", output_dir, name);
    render_grayscale(&linear_preview(arr), &png_path)?;
    let fits_path = format!("{}/{}.fits", output_dir, name);
    let mut header = header_2d.clone();
    header.set("BUNIT", unit.to_string());
    write_fits_mono(&fits_path, arr, Some(&header))?;
    Ok(MapFiles { png_path, fits_path })
}

pub(crate) fn moment_map_files(
    cube: &LazyCube,
    path: &str,
    output_dir: &str,
    cfg: &MomentConfig,
    t0: Instant,
) -> anyhow::Result<MomentMapsResponse> {
    let maps = moment_maps(cube, cfg)?;
    let header = strip_spectral_axis(&cube.header);
    let stem = output_stem(path);
    let range = format!("{}-{}", cfg.z0, cfg.z1);
    let m0 = save_map(&maps.m0, &header, &maps.m0_unit, output_dir, &format!("{}_m0_{}", stem, range))?;
    let m1 = save_map(&maps.m1, &header, maps.velocity_unit, output_dir, &format!("{}_m1_{}", stem, range))?;
    let m2 = save_map(&maps.m2, &header, maps.velocity_unit, output_dir, &format!("{}_m2_{}", stem, range))?;
    let (rows, cols) = maps.m0.dim();
    Ok(MomentMapsResponse {
        m0,
        m1,
        m2,
        m0_unit: maps.m0_unit,
        velocity_unit: maps.velocity_unit,
        noise_per_channel: maps.noise_per_channel,
        n_channels: maps.n_channels,
        notes: maps.notes,
        dimensions: [cols, rows],
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

#[tauri::command]
pub async fn process_cube_cmd(
    path: String,
    output_dir: String,
    frame_step: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let result = process_cube(&path, &output_dir, frame_step)?;
        Ok(serde_json::to_value(&result)?)
    })
}

#[tauri::command]
pub async fn process_cube_lazy_cmd(
    path: String,
    output_dir: String,
    frame_step: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let result = process_cube_lazy(&path, &output_dir, frame_step)?;
        Ok(serde_json::to_value(&result)?)
    })
}

#[tauri::command]
pub async fn get_cube_info(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cube = GLOBAL_CUBE_CACHE.get_or_open(&path)?;
        let geo = &cube.geometry;
        let classification = classify_spectral_cube(&cube.header, geo.naxis3);
        let wavelengths = build_wavelength_axis(&cube.header);
        let spectral_axis = cube.spectral_axis().ok();
        Ok(json!({
            RES_WIDTH: geo.naxis1,
            RES_HEIGHT: geo.naxis2,
            RES_FRAMES: geo.naxis3,
            RES_BITPIX: geo.bitpix,
            RES_SPECTRAL_CLASSIFICATION: {
                RES_IS_SPECTRAL: classification.is_spectral,
                RES_SPECTRAL_REASON: classification.reason,
                RES_AXIS_TYPE: classification.axis_type,
                RES_AXIS_UNIT: classification.axis_unit,
                RES_CHANNEL_COUNT: classification.channel_count,
            },
            RES_WAVELENGTHS: wavelengths,
            KEY_SPECTRAL_AXIS: spectral_axis,
        }))
    })
}

#[tauri::command]
pub async fn get_cube_frame(
    path: String,
    frame_index: usize,
    output_path: String,
    output_fits: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cube = GLOBAL_CUBE_CACHE.get_or_open(&path)?;
        let frame = cube.get_frame(frame_index)?;
        crate::infra::render::grayscale::render_grayscale(&frame, &output_path)?;
        let fits_path = if let Some(fp) = &output_fits {
            crate::infra::fits::writer::write_fits_mono(fp, &frame, None)?;
            Some(fp.clone())
        } else {
            None
        };
        Ok(json!({
            RES_FRAME_INDEX: frame_index,
            RES_OUTPUT_PATH: output_path,
            RES_FITS_PATH: fits_path,
        }))
    })
}

#[tauri::command]
pub async fn get_cube_spectrum(
    path: String,
    x: usize,
    y: usize,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cube = GLOBAL_CUBE_CACHE.get_or_open(&path)?;
        let spectrum = cube.extract_spectrum_at(y, x)?;
        let wavelengths = build_wavelength_axis(&cube.header);
        let classification = classify_spectral_cube(&cube.header, cube.geometry.naxis3);
        Ok(json!({
            RES_SPECTRUM: spectrum,
            RES_WAVELENGTHS: wavelengths,
            RES_IS_SPECTRAL: classification.is_spectral,
        }))
    })
}

#[tauri::command]
pub async fn get_cube_spectrum_region_cmd(
    path: String,
    shape: RegionShape,
    background: Option<RegionShape>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let cube = GLOBAL_CUBE_CACHE.get_or_open(&path)?;
        let response = region_spectrum_response(&cube, &shape, background.as_ref(), t0)?;
        Ok(serde_json::to_value(&response)?)
    })
}

#[tauri::command]
pub async fn collapse_cube_range_cmd(
    path: String,
    output_dir: String,
    z0: usize,
    z1: usize,
    mode: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let mode = CollapseMode::parse(&mode)?;
        let out_dir = resolve_output_dir(&output_dir)?;
        let cube = GLOBAL_CUBE_CACHE.get_or_open(&path)?;
        let response = collapse_range_files(&cube, &path, &out_dir, z0, z1, mode, t0)?;
        Ok(serde_json::to_value(&response)?)
    })
}

#[tauri::command]
pub async fn moment_maps_cmd(
    path: String,
    output_dir: String,
    config: MomentConfig,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let out_dir = resolve_output_dir(&output_dir)?;
        let cube = GLOBAL_CUBE_CACHE.get_or_open(&path)?;
        let response = moment_map_files(&cube, &path, &out_dir, &config, t0)?;
        Ok(serde_json::to_value(&response)?)
    })
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;
    use crate::core::astrometry::spectral::VelocityConvention;
    use crate::core::cube::lazy::{region_pixels, test_support::*};
    use crate::core::cube::moments::DEFAULT_SNR_THRESHOLD;
    use crate::infra::fits::reader::extract_image_mmap;

    fn open_cube(dir: &tempfile::TempDir, name: &str, noise: f32) -> (String, LazyCube) {
        let path = dir.path().join(name);
        write_line_cube(&path, noise);
        let text = path.to_str().unwrap().to_string();
        let cube = LazyCube::open(&text).unwrap();
        (text, cube)
    }

    fn disk() -> RegionShape {
        RegionShape::Circle { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r: LINE_DISK_RADIUS }
    }

    #[test]
    fn strip_spectral_axis_removes_every_third_axis_card_and_keeps_bunit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pc.fits");
        write_line_cube_with_cards(&path, &[("PC3_3", "1.0"), ("PC1_3", "0.0"), ("CD2_3", "0.0"), ("CNAME3", "'wave'")]);
        let cube = LazyCube::open(path.to_str().unwrap()).unwrap();
        let header = strip_spectral_axis(&cube.header);
        for key in ["NAXIS3", "CTYPE3", "CRVAL3", "CRPIX3", "CUNIT3", "CD3_3", "PC3_3", "PC1_3", "CD2_3", "CNAME3"] {
            assert!(header.get(key).is_none(), "{} survived", key);
            assert!(!header.cards.iter().any(|(k, _)| k == key), "{} card survived", key);
        }
        assert_eq!(header.get("WCSAXES"), Some("2"));
        assert_eq!(header.get("NAXIS"), Some("2"));
        assert_eq!(header.get("BUNIT"), Some("Jy/beam"));
        assert_eq!(header.get("CTYPE1"), Some("RA---TAN"));
        assert_eq!(header.get("CD1_1"), Some("-1.0E-4"));
        assert_eq!(header.get("RESTWAV"), Some("1.020E-6"));
        assert!(is_axis3_matrix_key("PC3_1") && is_axis3_matrix_key("CD1_3") && !is_axis3_matrix_key("CD1_2"));
        assert!(!is_axis3_matrix_key("PCOUNT"));
    }

    #[test]
    fn collapsed_range_fits_reopens_as_a_2d_image_with_the_wcs_intact() {
        let dir = tempfile::tempdir().unwrap();
        let (path, cube) = open_cube(&dir, "line.fits", 0.0);
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let out = collapse_range_files(&cube, &path, out_dir.to_str().unwrap(), 15, 25, CollapseMode::Sum, Instant::now()).unwrap();
        assert_eq!(out.mode, "sum");
        assert_eq!(out.dimensions, [LINE_CUBE_SIZE, LINE_CUBE_SIZE]);
        assert!(out.fits_path.ends_with("line_collapse_sum_15-25.fits"));
        assert!(std::path::Path::new(&out.png_path).exists());

        let reopened = extract_image_mmap(&File::open(&out.fits_path).unwrap()).unwrap();
        assert_eq!(reopened.image.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
        assert_eq!(reopened.header.get_i64("NAXIS"), Some(2));
        assert!(reopened.header.get("NAXIS3").is_none());
        assert!(reopened.header.get("CTYPE3").is_none());
        assert!(reopened.header.get("CD3_3").is_none());
        assert_eq!(reopened.header.get_i64("WCSAXES"), Some(2));
        assert_eq!(reopened.header.get("BUNIT"), Some("Jy/beam"));
        let n = 11.0f32;
        let line_sum: f32 = (15..=25).map(line_profile).sum();
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        assert!((reopened.image[centre] - (n + line_sum)).abs() < 1e-3);
        assert!((reopened.image[[0, 0]] - n).abs() < 1e-5);

        let original = WcsTransform::from_header(&cube.header).unwrap();
        let written = WcsTransform::from_header(&reopened.header).unwrap();
        let a = original.pixel_to_world(10.0, 10.0);
        let b = written.pixel_to_world(10.0, 10.0);
        assert!((a.ra - b.ra).abs() < 1e-9 && (a.dec - b.dec).abs() < 1e-9, "{:?} vs {:?}", (a.ra, a.dec), (b.ra, b.dec));
    }

    #[test]
    fn region_spectrum_reports_flux_in_jy_only_for_mjy_per_sr_cubes() {
        let dir = tempfile::tempdir().unwrap();
        let (_, cube) = open_cube(&dir, "line.fits", 0.0);
        let response = region_spectrum_response(&cube, &disk(), None, Instant::now()).unwrap();
        assert!(response.flux_jy.is_none());
        assert!(!response.bg_subtracted);
        assert_eq!(response.unit, "um");
        let wavelengths = response.wavelengths.as_ref().unwrap();
        assert_eq!(wavelengths.len(), LINE_CUBE_DEPTH);
        assert!((wavelengths[20] - LINE_REST_UM).abs() < 1e-12);
        assert!((response.npix - disk_pixel_count() as f64).abs() < 1.0);

        let path = dir.path().join("mjy.fits");
        write_line_cube_with_cards(&path, &[("BUNIT", "'MJy/sr'"), ("PIXAR_SR", "1.0E-13")]);
        let mjy = LazyCube::open(path.to_str().unwrap()).unwrap();
        let background = RegionShape::Annulus { x: LINE_DISK_CENTRE, y: LINE_DISK_CENTRE, r_inner: 9.0, r_outer: 13.0 };
        let response = region_spectrum_response(&mjy, &disk(), Some(&background), Instant::now()).unwrap();
        assert!(response.bg_subtracted);
        let flux = response.flux_jy.as_ref().expect("flux in Jy");
        assert_eq!(flux.len(), LINE_CUBE_DEPTH);
        for z in 0..LINE_CUBE_DEPTH {
            let expected = response.sum[z] as f64 * 1.0e-13 * JY_PER_MJY;
            assert!((flux[z] - expected).abs() <= 1e-12 * expected.abs().max(1.0), "z={}", z);
        }
        let line_weight: f64 = region_pixels(&disk(), LINE_CUBE_SIZE, LINE_CUBE_SIZE, APERTURE_SUBSAMPLES)
            .unwrap()
            .iter()
            .filter(|p| inside_disk(p.y, p.x))
            .map(|p| p.weight as f64)
            .sum();
        assert!(line_weight < response.npix);
        assert!((response.sum[20] as f64 - line_weight).abs() < 1e-3, "sum={} weight={}", response.sum[20], line_weight);
    }

    #[test]
    fn moment_map_files_write_three_2d_fits_with_their_units() {
        let dir = tempfile::tempdir().unwrap();
        let (path, cube) = open_cube(&dir, "line.fits", 0.01);
        let out_dir = dir.path().join("maps");
        std::fs::create_dir_all(&out_dir).unwrap();
        let cfg = MomentConfig {
            z0: 8,
            z1: 32,
            rest_um: None,
            convention: VelocityConvention::Optical,
            continuum: Some(((0, 5), (34, 39))),
            snr_threshold: DEFAULT_SNR_THRESHOLD,
            mask_below_threshold: true,
        };
        let out = moment_map_files(&cube, &path, out_dir.to_str().unwrap(), &cfg, Instant::now()).unwrap();
        assert_eq!(out.m0_unit, "Jy/beam km/s");
        assert_eq!(out.velocity_unit, "km/s");
        assert_eq!(out.n_channels, 25);
        assert_eq!(out.dimensions, [LINE_CUBE_SIZE, LINE_CUBE_SIZE]);
        assert!(out.m0.fits_path.ends_with("line_m0_8-32.fits"));
        for (files, unit) in [(&out.m0, "Jy/beam km/s"), (&out.m1, "km/s"), (&out.m2, "km/s")] {
            assert!(std::path::Path::new(&files.png_path).exists());
            let reopened = extract_image_mmap(&File::open(&files.fits_path).unwrap()).unwrap();
            assert_eq!(reopened.header.get_i64("NAXIS"), Some(2));
            assert!(reopened.header.get("CTYPE3").is_none());
            assert_eq!(reopened.header.get("BUNIT"), Some(unit));
            assert_eq!(reopened.image.dim(), (LINE_CUBE_SIZE, LINE_CUBE_SIZE));
            assert!(WcsTransform::from_header(&reopened.header).is_ok());
        }
        let m1 = extract_image_mmap(&File::open(&out.m1.fits_path).unwrap()).unwrap().image;
        let centre = (LINE_DISK_CENTRE as usize, LINE_DISK_CENTRE as usize);
        assert!(m1[centre].is_finite());
        assert!(m1[[0, 0]].is_nan());
    }

    #[test]
    fn linear_preview_scales_percentiles_and_blacks_out_nan() {
        let arr = Array2::from_shape_vec((2, 3), vec![f32::NAN, 0.0, 1.0, 2.0, 3.0, 4.0]).unwrap();
        let out = linear_preview(&arr);
        assert_eq!(out[[0, 0]], 0.0);
        assert_eq!(out[[0, 1]], 0.0);
        assert!((out[[1, 2]] - 1.0).abs() < 1e-6);
        assert!(out[[0, 2]] > 0.0 && out[[0, 2]] < out[[1, 0]] && out[[1, 0]] < out[[1, 1]]);
        let flat = Array2::from_elem((2, 2), 5.0f32);
        assert!(linear_preview(&flat).iter().all(|v| v.is_finite()));
        let empty = Array2::from_elem((2, 2), f32::NAN);
        assert!(linear_preview(&empty).iter().all(|v| *v == 0.0));
    }
}
