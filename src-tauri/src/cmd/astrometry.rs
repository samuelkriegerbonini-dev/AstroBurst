use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, cached_header, derived_output_header, image_ref, load_cached_full, output_stem,
    render_named_and_save, resolve_output_dir, source_path, OutputValues,
};
use crate::core::astrometry::frames::{convert_from_icrs, convert_to_icrs, SkyFrame};
use crate::core::astrometry::grid::{wcs_grid, WcsGrid, DEFAULT_DENSITY, MAX_DENSITY, MIN_DENSITY};
use crate::core::astrometry::wcs::{
    angular_separation, pixel_center, pixel_edge_corners, position_angle_deg, WcsTransform,
};
use crate::infra::config;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::writer::{filter_header, is_wcs_card};
use crate::infra::image_source::{load_plane, load_plane_header};
use crate::types::constants::{
    DEFAULT_API_KEY_SERVICE, HEADER_NAXIS1, HEADER_NAXIS2, RES_A_SKY, RES_B_SKY, RES_CENTER_DEC,
    RES_CENTER_RA, RES_DIMENSIONS, RES_EAST_VEC, RES_ELAPSED_MS, RES_FITS_PATH, RES_FLIPPED,
    RES_FOV_ARCMIN, RES_FOV_H_ARCMIN, RES_FOV_W_ARCMIN, RES_FRAME, RES_NAXIS1, RES_NAXIS2,
    RES_NORTH_VEC, RES_ON_IMAGE, RES_PARITY, RES_PIXEL_LENGTH, RES_PIXEL_SCALE_ARCSEC,
    RES_PIXEL_SCALE_X_ARCSEC, RES_PIXEL_SCALE_Y_ARCSEC, RES_PNG_PATH, RES_POINTS,
    RES_POSITION_ANGLE_DEG, RES_PROJECTION, RES_ROTATION_DEG, RES_SEPARATION_ARCMIN,
    RES_SEPARATION_ARCSEC, RES_SEPARATION_DEG, RES_SIP_PRESENT,
};
use crate::types::config::AppConfig;
use crate::types::header::HduHeader;

const MAX_UPLOAD_DIM: usize = 2048;
const MAX_SKY_POINTS: usize = 20_000;
const MAX_LATITUDE_DEG: f64 = 90.0;
const PARITY_NORMAL: &str = "normal";
const PARITY_FLIPPED: &str = "flipped";
const ABPROC_PLATE_SOLVED: &str = "platesolved";

fn load_header_and_wcs(path: &str) -> anyhow::Result<(crate::types::header::HduHeader, WcsTransform)> {
    let header = load_plane_header(&image_ref(path))?;
    let wcs = WcsTransform::from_header(&header)?;
    Ok((header, wcs))
}

type WcsStamp = (u64, Option<std::time::SystemTime>);

#[derive(Clone)]
struct CachedWcs {
    wcs: std::sync::Arc<WcsTransform>,
    naxis1: usize,
    naxis2: usize,
}

static WCS_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, (WcsStamp, CachedWcs)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

const WCS_CACHE_CAPACITY: usize = 64;

type GridCacheKey = (String, &'static str, u8);

static GRID_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<GridCacheKey, (WcsStamp, std::sync::Arc<WcsGrid>)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

const GRID_CACHE_CAPACITY: usize = 32;

fn file_stamp(path: &str) -> Option<WcsStamp> {
    std::fs::metadata(source_path(path)).ok().map(|m| (m.len(), m.modified().ok()))
}

fn load_wcs_with_dims_cached(path: &str) -> anyhow::Result<CachedWcs> {
    let stamp = file_stamp(path);

    if let Some(st) = &stamp {
        let cache = WCS_CACHE.lock().unwrap();
        if let Some((cached_stamp, cached)) = cache.get(path) {
            if cached_stamp == st {
                return Ok(cached.clone());
            }
        }
    }

    let header = load_plane_header(&image_ref(path))?;
    let (naxis1, naxis2) = image_dims(&header);
    let cached = CachedWcs {
        wcs: std::sync::Arc::new(WcsTransform::from_header(&header)?),
        naxis1,
        naxis2,
    };

    if let Some(st) = stamp {
        let mut cache = WCS_CACHE.lock().unwrap();
        if cache.len() >= WCS_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(path.to_string(), (st, cached.clone()));
    }

    Ok(cached)
}

fn load_wcs_cached(path: &str) -> anyhow::Result<std::sync::Arc<WcsTransform>> {
    load_wcs_with_dims_cached(path).map(|c| c.wcs)
}

fn load_grid_cached(path: &str, frame: SkyFrame, density: u8) -> anyhow::Result<std::sync::Arc<WcsGrid>> {
    let stamp = file_stamp(path);
    let key: GridCacheKey = (path.to_string(), frame.name(), density);

    if let Some(st) = &stamp {
        let cache = GRID_CACHE.lock().unwrap();
        if let Some((cached_stamp, grid)) = cache.get(&key) {
            if cached_stamp == st {
                return Ok(std::sync::Arc::clone(grid));
            }
        }
    }

    let cached = load_wcs_with_dims_cached(path)?;
    let grid = std::sync::Arc::new(
        wcs_grid(&cached.wcs, cached.naxis1, cached.naxis2, frame, density)
            .map_err(|e| anyhow::anyhow!(e))?,
    );

    if let Some(st) = stamp {
        let mut cache = GRID_CACHE.lock().unwrap();
        if cache.len() >= GRID_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(key, (st, std::sync::Arc::clone(&grid)));
    }

    Ok(grid)
}

fn plate_solve_timeout(loaded: anyhow::Result<AppConfig>) -> anyhow::Result<u64> {
    let secs = match loaded {
        Ok(cfg) => cfg.plate_solve_timeout_secs,
        Err(e) => {
            log::warn!("config unavailable ({:#}); using the default plate-solve timeout", e);
            AppConfig::default().plate_solve_timeout_secs
        }
    };
    if secs == 0 {
        anyhow::bail!("the plate-solve timeout is 0 seconds; set it to at least 1 second in Settings");
    }
    Ok(secs)
}

fn resolve_api_key(provided: Option<String>) -> Option<String> {
    if let Some(ref k) = provided {
        if !k.is_empty() {
            return provided;
        }
    }
    config::load_api_key(DEFAULT_API_KEY_SERVICE).ok().flatten()
}

#[tauri::command]
pub async fn plate_solve_cmd(
    path: String,
    api_key: Option<String>,
    scale_lower: Option<f64>,
    scale_upper: Option<f64>,
    scale_units: Option<String>,
    downsample_factor: Option<u32>,
    center_ra: Option<f64>,
    center_dec: Option<f64>,
    radius: Option<f64>,
) -> Result<serde_json::Value, String> {
    let (upload_path, _tmp, _tmp_ds, upload_dims, original_dims, ds_factor, cfg) = tokio::task::spawn_blocking(
        move || -> anyhow::Result<_> {
            let resolved_key = resolve_api_key(api_key);

            let r = image_ref(&path);
            let (resolved_path, tmp) = resolve_single_image(&r.path)?;
            let loaded = load_plane(&r)?;
            let (image, header) = (loaded.arr, loaded.header);

            let (naxis2, naxis1) = image.dim();

            let target_dims: Option<(usize, usize)> = if let Some(f) = downsample_factor.filter(|&f| f > 1) {
                let ds_cols = (naxis1 / f as usize).max(1);
                let ds_rows = (naxis2 / f as usize).max(1);
                Some((ds_rows, ds_cols))
            } else if naxis1 > MAX_UPLOAD_DIM || naxis2 > MAX_UPLOAD_DIM {
                let scale = MAX_UPLOAD_DIM as f64 / naxis1.max(naxis2) as f64;
                Some((
                    ((naxis2 as f64 * scale).round() as usize).max(1),
                    ((naxis1 as f64 * scale).round() as usize).max(1),
                ))
            } else {
                None
            };

            let (upload_fits, tmp_ds, ds_factor, upload_dims) = if let Some((ds_rows, ds_cols)) = target_dims {
                log::info!(
                    "Plate solve: downsampling {}x{} to {}x{} for upload",
                    naxis1, naxis2, ds_cols, ds_rows
                );

                let downsampled = crate::core::alignment::downsample::area_downsample(
                    &image, ds_rows, ds_cols,
                );

                let tmp_file = tempfile::Builder::new()
                    .suffix(".fits")
                    .tempfile()?;
                let tmp_path = tmp_file.path().to_string_lossy().to_string();

                crate::infra::fits::writer::write_fits_mono(
                    &tmp_path,
                    &downsampled,
                    Some(&header),
                )?;

                let fx = naxis1 as f64 / ds_cols as f64;
                let fy = naxis2 as f64 / ds_rows as f64;
                (tmp_path, Some(tmp_file), Some((fx, fy)), (ds_cols, ds_rows))
            } else if r.is_auto() {
                (resolved_path.to_string_lossy().to_string(), None, None, (naxis1, naxis2))
            } else {
                let tmp_file = tempfile::Builder::new()
                    .suffix(".fits")
                    .tempfile()?;
                let tmp_path = tmp_file.path().to_string_lossy().to_string();
                crate::infra::fits::writer::write_fits_mono(&tmp_path, &image, Some(&header))?;
                (tmp_path, Some(tmp_file), None, (naxis1, naxis2))
            };

            let is_pixel_scale = scale_units.as_deref().map_or(true, |u| u.contains("pix"));
            let hint_scale = if is_pixel_scale {
                ds_factor.map_or(1.0, |(fx, fy)| (fx * fy).sqrt())
            } else {
                1.0
            };

            let cfg = crate::infra::astrometry::plate_solve::SolveConfig {
                api_url: config::astrometry_api_url()?,
                api_key: resolved_key.unwrap_or_default(),
                ra_hint: center_ra,
                dec_hint: center_dec,
                radius_hint: radius,
                scale_low: scale_lower.map(|v| v * hint_scale),
                scale_high: scale_upper.map(|v| v * hint_scale),
                scale_units,
                timeout_secs: plate_solve_timeout(config::load_config())?,
            };

            Ok((upload_fits, tmp, tmp_ds, upload_dims, (naxis1, naxis2), ds_factor, cfg))
        },
    )
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e: anyhow::Error| e.to_string())?;

    #[cfg(feature = "astrometry-net")]
    {
        let mut solve_result = crate::infra::astrometry::plate_solve::solve_astrometry_net(
            &upload_path, upload_dims.0, upload_dims.1, &cfg,
        )
            .await
            .map_err(|e| e.to_string())?;

        if let Some((fx, fy)) = ds_factor {
            rescale_solve_to_original(&mut solve_result, fx, fy, original_dims.0, original_dims.1);
            solve_result.wcs_cards =
                rescale_wcs_cards_to_original(&solve_result.wcs_cards, upload_dims, original_dims);
        }

        drop((_tmp, _tmp_ds));
        return serde_json::to_value(&solve_result).map_err(|e| e.to_string());
    }

    #[cfg(not(feature = "astrometry-net"))]
    {
        drop((_tmp, _tmp_ds, upload_path, upload_dims, original_dims, ds_factor, cfg));
        let result = crate::infra::astrometry::plate_solve::solve_offline_placeholder()
            .map_err(|e| e.to_string())?;
        serde_json::to_value(&result).map_err(|e| e.to_string())
    }
}

#[cfg(feature = "astrometry-net")]
fn rescale_solve_to_original(
    result: &mut crate::infra::astrometry::plate_solve::SolveResult,
    fx: f64,
    fy: f64,
    width: usize,
    height: usize,
) {
    let f_mean = (fx * fy).sqrt();
    result.pixel_scale /= f_mean;
    result.field_w_arcmin = result.pixel_scale * width as f64 / 60.0;
    result.field_h_arcmin = result.pixel_scale * height as f64 / 60.0;

    for ann in &mut result.annotations {
        ann.pixelx = fx * (ann.pixelx - 0.5) + 0.5;
        ann.pixely = fy * (ann.pixely - 0.5) + 0.5;
        if let Some(r) = ann.radius.as_mut() {
            *r *= f_mean;
        }
    }
}

#[cfg(feature = "astrometry-net")]
fn rescale_wcs_cards_to_original(
    cards: &[(String, String)],
    upload_dims: (usize, usize),
    original_dims: (usize, usize),
) -> Vec<(String, String)> {
    let mut header = HduHeader::empty();
    for (key, value) in cards {
        header.set(key.trim(), value.clone());
    }
    let updates = crate::core::imaging::resample::compute_wcs_updates(
        &header,
        (upload_dims.1, upload_dims.0),
        (original_dims.1, original_dims.0),
    );
    for (key, value) in updates {
        if key.starts_with("NAXIS") {
            continue;
        }
        header.set_f64(&key, value);
    }
    header.cards
}

pub(crate) fn write_solved_wcs(
    path: &str,
    wcs_cards: &[(String, String)],
    output_dir: &str,
) -> anyhow::Result<serde_json::Value> {
    let started = std::time::Instant::now();
    let name = output_stem(path);
    let source_header = cached_header(path)?;
    let planes = source_header.get_i64("NAXIS3").unwrap_or(1);
    if source_header.get_i64("NAXIS").unwrap_or(0) > 2 && planes > 1 {
        anyhow::bail!("{name} has {planes} planes; writing a solved WCS supports single-plane images only");
    }

    let entry = load_cached_full(path)?;
    let source = entry.header().cloned().unwrap_or_else(HduHeader::empty);
    let mut merged = filter_header(&source, false, true).unwrap_or_else(HduHeader::empty);
    for (key, value) in wcs_cards {
        let key = key.trim();
        if is_wcs_card(key) {
            merged.set(key, value.clone());
        }
    }
    let header = derived_output_header(Some(&merged), ABPROC_PLATE_SOLVED, OutputValues::Linear);

    let (rows, cols) = entry.arr().dim();
    let mut probe = header.clone();
    probe.set(HEADER_NAXIS1, cols.to_string());
    probe.set(HEADER_NAXIS2, rows.to_string());
    let wcs = WcsTransform::from_header(&probe).map_err(|e| {
        anyhow::anyhow!("The solved WCS is not a usable celestial WCS ({e:#}); nothing was written.")
    })?;

    let output_dir = resolve_output_dir(output_dir)?;
    let (png_path, fits_path) =
        render_named_and_save(entry.arr(), &output_dir, &format!("{name}_wcs"), true, Some(&header))?;
    let fits_path = fits_path.ok_or_else(|| anyhow::anyhow!("the solved WCS FITS was not written"))?;

    let (cx, cy) = pixel_center(cols, rows);
    let centre = wcs.pixel_to_world(cx, cy);
    Ok(json!({
        RES_FITS_PATH: fits_path,
        RES_PNG_PATH: png_path,
        RES_DIMENSIONS: [cols, rows],
        RES_CENTER_RA: centre.ra,
        RES_CENTER_DEC: centre.dec,
        RES_PIXEL_SCALE_ARCSEC: wcs.pixel_scale_arcsec(),
        RES_SIP_PRESENT: wcs.orientation(cols, rows).sip_present,
        RES_ELAPSED_MS: started.elapsed().as_millis() as u64,
    }))
}

#[tauri::command]
pub async fn write_solved_wcs_cmd(
    path: String,
    wcs_cards: Vec<(String, String)>,
    output_dir: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(write_solved_wcs(&path, &wcs_cards, &output_dir))
}

fn image_dims(header: &crate::types::header::HduHeader) -> (usize, usize) {
    let compressed = header
        .get("ZIMAGE")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("T"));
    let (key1, key2) = if compressed {
        ("ZNAXIS1", "ZNAXIS2")
    } else {
        (HEADER_NAXIS1, HEADER_NAXIS2)
    };
    let dim = |key: &str| {
        header
            .get_i64(key)
            .filter(|n| *n > 0)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0)
    };
    (dim(key1), dim(key2))
}

#[tauri::command]
pub async fn get_wcs_info(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (header, wcs) = load_header_and_wcs(&path)?;
        let (naxis1, naxis2) = image_dims(&header);
        let pixel_scale = wcs.pixel_scale_arcsec();
        let (fov_w, fov_h) = wcs.field_of_view(naxis1, naxis2);
        let (cx, cy) = pixel_center(naxis1, naxis2);
        let center = wcs.pixel_to_world(cx, cy);
        let orientation = wcs.orientation(naxis1, naxis2);

        Ok(json!({
            RES_CENTER_RA: center.ra,
            RES_CENTER_DEC: center.dec,
            RES_PIXEL_SCALE_ARCSEC: pixel_scale,
            RES_FOV_W_ARCMIN: fov_w,
            RES_FOV_H_ARCMIN: fov_h,
            RES_FOV_ARCMIN: [fov_w, fov_h],
            RES_NAXIS1: naxis1,
            RES_NAXIS2: naxis2,
            RES_ROTATION_DEG: orientation.rotation_deg,
            RES_FLIPPED: orientation.flipped,
            RES_PARITY: if orientation.flipped { PARITY_FLIPPED } else { PARITY_NORMAL },
            RES_PIXEL_SCALE_X_ARCSEC: orientation.pixel_scale_x_arcsec,
            RES_PIXEL_SCALE_Y_ARCSEC: orientation.pixel_scale_y_arcsec,
            RES_PROJECTION: orientation.projection,
            RES_SIP_PRESENT: orientation.sip_present,
            RES_NORTH_VEC: orientation.north_vec,
            RES_EAST_VEC: orientation.east_vec,
        }))
    })
}

#[tauri::command]
pub async fn pixel_to_world_cmd(
    path: String,
    points: Vec<(f64, f64)>,
    frame: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let frame = SkyFrame::from_name(frame.as_deref().unwrap_or("icrs"))
            .map_err(|e| anyhow::anyhow!(e))?;
        let wcs = load_wcs_cached(&path)?;
        let coords = wcs.pixel_to_world_batch(&points);
        let out: Vec<serde_json::Value> = coords
            .into_iter()
            .map(|c| {
                let (lon, lat) = convert_from_icrs(frame, c.ra, c.dec);
                if lon.is_finite() && lat.is_finite() {
                    json!([lon, lat])
                } else {
                    serde_json::Value::Null
                }
            })
            .collect();
        Ok(json!({ RES_POINTS: out, RES_FRAME: frame.name() }))
    })
}

fn pixel_on_image(x: f64, y: f64, naxis1: usize, naxis2: usize) -> bool {
    x >= -0.5 && x < naxis1 as f64 - 0.5 && y >= -0.5 && y < naxis2 as f64 - 0.5
}

fn finite_pair(name: &str, p: (f64, f64)) -> anyhow::Result<()> {
    if !p.0.is_finite() || !p.1.is_finite() {
        anyhow::bail!("point {} must have finite coordinates, got ({}, {})", name, p.0, p.1);
    }
    Ok(())
}

fn latitude_within_poles(name: &str, lat: f64) -> anyhow::Result<()> {
    if lat.is_finite() && lat.abs() > MAX_LATITUDE_DEG {
        anyhow::bail!(
            "{} latitude {} must be within -{} and {} degrees",
            name,
            lat,
            MAX_LATITUDE_DEG,
            MAX_LATITUDE_DEG
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn world_to_pixel_cmd(
    path: String,
    points: Vec<(f64, f64)>,
    frame: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        if points.len() > MAX_SKY_POINTS {
            anyhow::bail!(
                "points holds {} entries, more than the {} allowed per call",
                points.len(),
                MAX_SKY_POINTS
            );
        }
        for (i, &(_, lat)) in points.iter().enumerate() {
            latitude_within_poles(&format!("points[{}]", i), lat)?;
        }
        let frame = SkyFrame::from_name(frame.as_deref().unwrap_or("icrs"))
            .map_err(|e| anyhow::anyhow!(e))?;
        let cached = load_wcs_with_dims_cached(&path)?;
        let icrs: Vec<(f64, f64)> = points
            .iter()
            .map(|&(lon, lat)| {
                if lon.is_finite() && lat.is_finite() {
                    convert_to_icrs(frame, lon, lat)
                } else {
                    (f64::NAN, f64::NAN)
                }
            })
            .collect();
        let pixels = cached.wcs.world_to_pixel_batch(&icrs);
        let mut out = Vec::with_capacity(pixels.len());
        let mut on_image = Vec::with_capacity(pixels.len());
        for &(x, y) in &pixels {
            if x.is_finite() && y.is_finite() {
                out.push(json!([x, y]));
                on_image.push(pixel_on_image(x, y, cached.naxis1, cached.naxis2));
            } else {
                out.push(serde_json::Value::Null);
                on_image.push(false);
            }
        }
        Ok(json!({
            RES_POINTS: out,
            RES_ON_IMAGE: on_image,
            RES_FRAME: frame.name(),
            RES_NAXIS1: cached.naxis1,
            RES_NAXIS2: cached.naxis2,
        }))
    })
}

#[tauri::command]
pub async fn sky_separation_cmd(
    path: Option<String>,
    a: (f64, f64),
    b: (f64, f64),
    pixel: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        finite_pair("a", a)?;
        finite_pair("b", b)?;
        let (a_sky, b_sky, pixel_length, pixel_scale) = if pixel.unwrap_or(false) {
            let path = match path.as_deref().filter(|p| !p.is_empty()) {
                Some(p) => p,
                None => anyhow::bail!("pixel points need a file path, but path is empty"),
            };
            let wcs = load_wcs_cached(path)?;
            let ca = wcs.pixel_to_world(a.0, a.1);
            let cb = wcs.pixel_to_world(b.0, b.1);
            if !(ca.ra.is_finite() && ca.dec.is_finite() && cb.ra.is_finite() && cb.dec.is_finite()) {
                anyhow::bail!("one of the pixel points does not project onto the sky");
            }
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            ((ca.ra, ca.dec), (cb.ra, cb.dec), json!(length), json!(wcs.pixel_scale_arcsec()))
        } else {
            latitude_within_poles("point a", a.1)?;
            latitude_within_poles("point b", b.1)?;
            (a, b, serde_json::Value::Null, serde_json::Value::Null)
        };
        let separation_deg = angular_separation(a_sky.0, a_sky.1, b_sky.0, b_sky.1);
        let position_angle = position_angle_deg(a_sky.0, a_sky.1, b_sky.0, b_sky.1);
        Ok(json!({
            RES_A_SKY: [a_sky.0, a_sky.1],
            RES_B_SKY: [b_sky.0, b_sky.1],
            RES_SEPARATION_DEG: separation_deg,
            RES_SEPARATION_ARCMIN: separation_deg * 60.0,
            RES_SEPARATION_ARCSEC: separation_deg * 3600.0,
            RES_POSITION_ANGLE_DEG: position_angle,
            RES_PIXEL_LENGTH: pixel_length,
            RES_PIXEL_SCALE_ARCSEC: pixel_scale,
        }))
    })
}

#[tauri::command]
pub async fn grid_lines_cmd(
    path: String,
    frame: Option<String>,
    density: Option<u8>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let frame = SkyFrame::from_name(frame.as_deref().unwrap_or("icrs"))
            .map_err(|e| anyhow::anyhow!(e))?;
        let density = density.unwrap_or(DEFAULT_DENSITY).clamp(MIN_DENSITY, MAX_DENSITY);
        match load_grid_cached(&path, frame, density) {
            Ok(grid) => Ok(serde_json::to_value(&*grid)?),
            Err(e) => Ok(json!({ "error": format!("{:#}", e) })),
        }
    })
}

struct SkyFootprint {
    center_ra: f64,
    center_dec: f64,
    corners: [(f64, f64); 4],
    fov_w_arcmin: f64,
    fov_h_arcmin: f64,
}

fn sky_footprint(path: &str) -> anyhow::Result<SkyFootprint> {
    let (header, wcs) = load_header_and_wcs(path)?;
    let (n1, n2) = image_dims(&header);
    if n1 == 0 || n2 == 0 {
        anyhow::bail!("Missing NAXIS1/NAXIS2 in header");
    }
    let mut corners = [(0.0f64, 0.0f64); 4];
    for (i, &(x, y)) in pixel_edge_corners(n1, n2).iter().enumerate() {
        let c = wcs.pixel_to_world(x, y);
        if !c.ra.is_finite() || !c.dec.is_finite() {
            anyhow::bail!("Image corner does not project to a valid sky position");
        }
        corners[i] = (c.ra, c.dec);
    }
    let (cx, cy) = pixel_center(n1, n2);
    let center = wcs.pixel_to_world(cx, cy);
    if !center.ra.is_finite() || !center.dec.is_finite() {
        anyhow::bail!("Image center does not project to a valid sky position");
    }
    let (fov_w, fov_h) = wcs.field_of_view(n1, n2);
    Ok(SkyFootprint {
        center_ra: center.ra,
        center_dec: center.dec,
        corners,
        fov_w_arcmin: fov_w,
        fov_h_arcmin: fov_h,
    })
}

fn wrap180(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

fn footprint_overlap_fraction(a: &SkyFootprint, b: &SkyFootprint) -> f64 {
    let ra0 = a.center_ra;
    let dec0 = a.center_dec;
    let cosd = dec0.to_radians().cos().max(1e-6);

    let bbox = |fp: &SkyFootprint| {
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for &(ra, dec) in &fp.corners {
            let x = wrap180(ra - ra0) * cosd;
            let y = dec - dec0;
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            ymin = ymin.min(y);
            ymax = ymax.max(y);
        }
        (xmin, xmax, ymin, ymax)
    };

    let (ax1, ax2, ay1, ay2) = bbox(a);
    let (bx1, bx2, by1, by2) = bbox(b);

    let ix = (ax2.min(bx2) - ax1.max(bx1)).max(0.0);
    let iy = (ay2.min(by2) - ay1.max(by1)).max(0.0);
    let area_a = (ax2 - ax1) * (ay2 - ay1);
    let area_b = (bx2 - bx1) * (by2 - by1);
    let min_area = area_a.min(area_b);
    if min_area <= 0.0 {
        return 0.0;
    }
    (ix * iy) / min_area
}

#[tauri::command]
pub async fn check_pointing_overlap_cmd(
    paths: Vec<String>,
    threshold: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let threshold = threshold.unwrap_or(0.05).clamp(0.0, 1.0);

        let footprints: Vec<(String, Result<SkyFootprint, String>)> = paths
            .iter()
            .map(|p| (p.clone(), sky_footprint(p).map_err(|e| format!("{:#}", e))))
            .collect();

        let files: Vec<serde_json::Value> = footprints
            .iter()
            .map(|(p, r)| match r {
                Ok(fp) => json!({
                    "path": p,
                    "has_wcs": true,
                    "center_ra": fp.center_ra,
                    "center_dec": fp.center_dec,
                    "fov_w_arcmin": fp.fov_w_arcmin,
                    "fov_h_arcmin": fp.fov_h_arcmin,
                }),
                Err(e) => json!({ "path": p, "has_wcs": false, "error": e }),
            })
            .collect();

        let mut pairs = Vec::new();
        let mut any_disjoint = false;
        for i in 0..footprints.len() {
            for j in (i + 1)..footprints.len() {
                match (&footprints[i].1, &footprints[j].1) {
                    (Ok(fa), Ok(fb)) => {
                        let fraction = footprint_overlap_fraction(fa, fb);
                        if fraction < threshold {
                            any_disjoint = true;
                            let sep = angular_separation(
                                fa.center_ra, fa.center_dec,
                                fb.center_ra, fb.center_dec,
                            ) * 60.0;
                            pairs.push(json!({
                                "a": footprints[i].0,
                                "b": footprints[j].0,
                                "status": "disjoint",
                                "fraction": fraction,
                                "separation_arcmin": sep,
                            }));
                        }
                    }
                    _ => {
                        pairs.push(json!({
                            "a": footprints[i].0,
                            "b": footprints[j].0,
                            "status": "unknown",
                            "fraction": serde_json::Value::Null,
                            "separation_arcmin": serde_json::Value::Null,
                        }));
                    }
                }
            }
        }

        Ok(json!({
            "files": files,
            "pairs": pairs,
            "any_disjoint": any_disjoint,
            "threshold": threshold,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        footprint_overlap_fraction, image_dims, load_grid_cached, load_wcs_with_dims_cached, plate_solve_timeout,
        sky_footprint, wrap180, SkyFootprint,
    };
    use crate::types::config::AppConfig;
    use crate::core::astrometry::frames::SkyFrame;
    use crate::core::astrometry::grid::{DEFAULT_DENSITY, MAX_DENSITY};
    use crate::core::imaging::region::test_support::{make_header, north_up_cd, wcs_cards};
    use crate::infra::astrometry::plate_solve::test_support::nova_wcs_cards;

    fn write_north_up_fits(path: &str, size: usize) {
        let cards = wcs_cards(north_up_cd());
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let header = make_header(&pairs);
        let arr = ndarray::Array2::<f32>::zeros((size, size));
        crate::infra::fits::writer::write_fits_mono(path, &arr, Some(&header)).unwrap();
    }

    fn north_up_wcs_header() -> crate::types::header::HduHeader {
        let cards = wcs_cards(north_up_cd());
        let pairs: Vec<(&str, &str)> = cards
            .iter()
            .filter(|(k, _)| !k.starts_with("NAXIS"))
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        make_header(&pairs)
    }

    #[test]
    fn tile_compressed_images_use_their_decompressed_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rice.fits").to_string_lossy().to_string();
        let arr = ndarray::Array2::<f32>::from_shape_fn((30, 40), |(r, c)| (r * 40 + c) as f32);
        crate::infra::fits::writer::write_fits_mono_rice(&path, &arr, Some(&north_up_wcs_header()), 16, 0.0).unwrap();

        let cached = load_wcs_with_dims_cached(&path).unwrap();
        assert_eq!((cached.naxis1, cached.naxis2), (40, 30));
        let footprint = sky_footprint(&path).unwrap();
        assert!((footprint.fov_w_arcmin - 40.0 / 60.0).abs() < 1e-9, "fov_w {}", footprint.fov_w_arcmin);
        assert!((footprint.fov_h_arcmin - 30.0 / 60.0).abs() < 1e-9, "fov_h {}", footprint.fov_h_arcmin);

        let plain = make_header(&[("NAXIS1", "64"), ("NAXIS2", "32")]);
        assert_eq!(image_dims(&plain), (64, 32));
        let table = make_header(&[("XTENSION", "BINTABLE"), ("ZIMAGE", "T"), ("NAXIS1", "8"), ("NAXIS2", "23"), ("ZNAXIS1", "37"), ("ZNAXIS2", "23")]);
        assert_eq!(image_dims(&table), (37, 23));
        assert_eq!(image_dims(&make_header(&[("NAXIS1", "-4")])), (0, 0));
    }

    #[test]
    fn footprint_centre_and_corners_use_pixel_centres_and_edges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("footprint.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);

        let fp = sky_footprint(&path).unwrap();
        assert!((fp.center_ra - 150.0).abs() < 1e-9 && (fp.center_dec - 2.0).abs() < 1e-9, "({}, {})", fp.center_ra, fp.center_dec);
        let height_arcsec = (fp.corners[2].1 - fp.corners[1].1) * 3600.0;
        assert!((height_arcsec - 100.0).abs() < 1e-4, "footprint spans {height_arcsec} arcsec");
    }

    #[cfg(feature = "astrometry-net")]
    #[test]
    fn rescale_maps_an_upload_space_solution_to_the_original_frame() {
        use crate::infra::astrometry::plate_solve::{FieldAnnotation, SolveResult};
        let mut result = SolveResult {
            ra_center: 10.0,
            dec_center: 20.0,
            orientation: 30.0,
            pixel_scale: 2.0,
            field_w_arcmin: 2.0 * 2048.0 / 60.0,
            field_h_arcmin: 2.0 * 1024.0 / 60.0,
            annotations: vec![FieldAnnotation {
                kind: "ngc".into(),
                names: vec!["NGC 1".into()],
                pixelx: 1024.5,
                pixely: 512.5,
                radius: Some(10.0),
            }],
            wcs_cards: Vec::new(),
        };
        super::rescale_solve_to_original(&mut result, 2.0, 2.0, 4096, 2048);
        assert!((result.pixel_scale - 1.0).abs() < 1e-12);
        assert!((result.field_w_arcmin - 4096.0 / 60.0).abs() < 1e-9);
        assert!((result.field_h_arcmin - 2048.0 / 60.0).abs() < 1e-9);
        let ann = &result.annotations[0];
        assert!((ann.pixelx - 2048.5).abs() < 1e-12 && (ann.pixely - 1024.5).abs() < 1e-12);
        assert_eq!(ann.radius, Some(20.0));
        assert_eq!((result.ra_center, result.dec_center, result.orientation), (10.0, 20.0, 30.0));
    }

    #[cfg(feature = "astrometry-net")]
    #[test]
    fn rescaled_wcs_cards_reproduce_the_upload_solution_at_the_original_corners() {
        use crate::core::astrometry::wcs::{angular_separation, pixel_center, pixel_edge_corners, WcsTransform};

        let wcs_of = |cards: &[(String, String)], (cols, rows): (usize, usize)| {
            let mut h = crate::types::header::HduHeader::empty();
            for (k, v) in cards {
                h.set(k, v.clone());
            }
            h.set("NAXIS1", cols.to_string());
            h.set("NAXIS2", rows.to_string());
            WcsTransform::from_header(&h).unwrap()
        };
        let centred = |(cols, rows): (usize, usize)| -> Vec<(String, String)> {
            nova_wcs_cards()
                .into_iter()
                .map(|(k, v)| match k.as_str() {
                    "CRPIX1" => (k, format!("{}", cols as f64 / 2.0 + 0.5)),
                    "CRPIX2" => (k, format!("{}", rows as f64 / 2.0 + 0.5)),
                    _ => (k, v),
                })
                .collect()
        };
        let cases = [
            ((1024usize, 683usize), (2048usize, 1366usize), nova_wcs_cards()),
            ((2048, 1365), (6000, 4000), centred((2048, 1365))),
        ];
        for (upload, original, cards) in cases {
            let rescaled = super::rescale_wcs_cards_to_original(&cards, upload, original);
            let upload_wcs = wcs_of(&cards, upload);
            let orig_wcs = wcs_of(&rescaled, original);
            let fx = original.0 as f64 / upload.0 as f64;
            let fy = original.1 as f64 / upload.1 as f64;
            let scale = orig_wcs.pixel_scale_arcsec();
            let mut points: Vec<(f64, f64)> = pixel_edge_corners(original.0, original.1).to_vec();
            points.push(pixel_center(original.0, original.1));
            for (x, y) in points {
                let xu = (x + 0.5) / fx - 0.5;
                let yu = (y + 0.5) / fy - 0.5;
                let up = upload_wcs.pixel_to_world(xu, yu);
                let orig = orig_wcs.pixel_to_world(x, y);
                let sep_px = angular_separation(up.ra, up.dec, orig.ra, orig.dec) * 3600.0 / scale;
                assert!(sep_px < 0.05, "{upload:?} -> {original:?} at ({x}, {y}): {sep_px} px apart");
                let (pu, pv) = upload_wcs.world_to_pixel(up.ra, up.dec);
                let (px, py) = orig_wcs.world_to_pixel(up.ra, up.dec);
                let (ex, ey) = (fx * (pu + 0.5) - 0.5, fy * (pv + 0.5) - 0.5);
                assert!(
                    (px - ex).abs() < 0.05 && (py - ey).abs() < 0.05,
                    "{upload:?} -> {original:?} at ({x}, {y}): ({px}, {py}) vs ({ex}, {ey})"
                );
            }
        }
    }

    fn source_with_cards(
        dir: &tempfile::TempDir,
        name: &str,
        extra: &[(String, String)],
    ) -> (String, ndarray::Array2<f32>) {
        let mut pairs = wcs_cards(north_up_cd());
        pairs.extend(extra.iter().cloned());
        pairs.push(("BUNIT".to_string(), "MJy/sr".to_string()));
        pairs.push(("EXPTIME".to_string(), "300".to_string()));
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let arr = ndarray::Array2::from_shape_fn((48, 64), |(y, x)| (y * 64 + x) as f32 * 0.5 + 10.0);
        let path = dir.path().join(name).to_string_lossy().to_string();
        crate::infra::fits::writer::write_fits_mono(&path, &arr, Some(&make_header(&refs))).unwrap();
        (path, arr)
    }

    fn source_with_old_wcs(dir: &tempfile::TempDir, name: &str) -> (String, ndarray::Array2<f32>) {
        source_with_cards(dir, name, &[])
    }

    fn stale_wcs_cards() -> Vec<(String, String)> {
        [
            ("CDELT1", "-2.777777777778E-04"),
            ("CDELT2", "2.777777777778E-04"),
            ("PC1_1", "1.0"),
            ("PC1_2", "0.0"),
            ("PC2_1", "0.0"),
            ("PC2_2", "1.0"),
            ("CROTA2", "12.5"),
            ("A_ORDER", "3"),
            ("A_0_3", "4.0E-11"),
            ("A_3_0", "-7.0E-11"),
            ("B_ORDER", "3"),
            ("B_0_3", "6.0E-11"),
            ("B_3_0", "-2.0E-11"),
            ("PV1_1", "0.0"),
            ("PV1_2", "90.0"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn write_solved_wcs_drops_the_stale_wcs_cards_nova_does_not_send() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = source_with_cards(&dir, "stale_src.fits", &stale_wcs_cards());
        let out = dir.path().join("out").to_string_lossy().to_string();
        let source = crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&path)).unwrap();
        for (key, _) in stale_wcs_cards() {
            assert!(source.get(&key).is_some(), "{key} is missing from the fixture");
        }

        let result = super::write_solved_wcs(&path, &nova_wcs_cards(), &out).unwrap();

        let fits_path = result["fits_path"].as_str().unwrap().to_string();
        let header = crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&fits_path)).unwrap();
        for key in [
            "CDELT1", "CDELT2", "PC1_1", "PC1_2", "PC2_1", "PC2_2", "CROTA2", "A_0_3", "A_3_0", "B_0_3", "B_3_0",
            "PV1_1", "PV1_2",
        ] {
            assert!(header.get(key).is_none(), "stale {key} survived as {:?}", header.get(key));
        }
        assert_eq!(header.get_i64("A_ORDER"), Some(2));
        assert_eq!(header.get_i64("B_ORDER"), Some(2));

        let mut written: Vec<String> = header
            .cards
            .iter()
            .map(|(k, _)| k.trim().to_string())
            .filter(|k| crate::infra::fits::writer::is_wcs_card(k))
            .collect();
        written.sort();
        written.dedup();
        let mut expected: Vec<String> = nova_wcs_cards().into_iter().map(|(k, _)| k).collect();
        expected.sort();
        assert_eq!(written, expected);

        let mut nova_only = crate::types::header::HduHeader::empty();
        for (k, v) in nova_wcs_cards() {
            nova_only.set(&k, v);
        }
        nova_only.set("NAXIS1", "64".to_string());
        nova_only.set("NAXIS2", "48".to_string());
        let expected_wcs = crate::core::astrometry::wcs::WcsTransform::from_header(&nova_only).unwrap();
        let written_wcs = crate::core::astrometry::wcs::WcsTransform::from_header(&header).unwrap();
        for (x, y) in [(0.0, 0.0), (63.0, 0.0), (0.0, 47.0), (63.0, 47.0), (31.5, 23.5)] {
            let ours = written_wcs.pixel_to_world(x, y);
            let theirs = expected_wcs.pixel_to_world(x, y);
            assert!(
                (ours.ra - theirs.ra).abs() < 1e-9 && (ours.dec - theirs.dec).abs() < 1e-9,
                "({x}, {y}): ({}, {}) vs ({}, {})",
                ours.ra,
                ours.dec,
                theirs.ra,
                theirs.dec
            );
        }
        assert_eq!(result["sip_present"], true);
    }

    #[tokio::test]
    async fn write_solved_wcs_replaces_the_old_wcs_and_keeps_pixels_and_units() {
        let dir = tempfile::tempdir().unwrap();
        let (path, arr) = source_with_old_wcs(&dir, "solved_src.fits");
        let out = dir.path().join("out").to_string_lossy().to_string();

        let result = super::write_solved_wcs(&path, &nova_wcs_cards(), &out).unwrap();

        let fits_path = result["fits_path"].as_str().unwrap().to_string();
        assert!(fits_path.ends_with("solved_src_wcs.fits"), "{fits_path}");
        assert!(std::path::Path::new(&fits_path).exists(), "{fits_path} missing");
        assert!(std::path::Path::new(result["png_path"].as_str().unwrap()).exists(), "{}", result["png_path"]);
        assert_eq!(result["dimensions"], serde_json::json!([64, 48]));

        let header = crate::infra::image_source::load_plane_header(&crate::cmd::common::image_ref(&fits_path)).unwrap();
        assert_eq!(header.get("CTYPE1").map(str::trim), Some("RA---TAN-SIP"));
        assert_eq!(header.get("CTYPE2").map(str::trim), Some("DEC--TAN-SIP"));
        assert_eq!(header.get_f64("CRVAL1"), Some(83.822));
        assert_eq!(header.get_f64("CRVAL2"), Some(-5.391));
        assert_eq!(header.get_f64("CD1_1"), Some(-5.55e-4));
        assert_eq!(header.get_f64("A_2_0"), Some(1.2e-7));
        assert_eq!(header.get("BUNIT").map(str::trim), Some("MJy/sr"));
        assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
        assert_eq!(header.get("ABPROC").map(str::trim), Some("platesolved"));
        assert!(header.get("ABDISP").is_none());

        let written = crate::infra::fits::reader::load_fits_image(&fits_path).unwrap();
        assert_eq!(written, arr);

        let info = super::get_wcs_info(fits_path.clone()).await.unwrap();
        for key in ["center_ra", "center_dec", "pixel_scale_arcsec"] {
            let ours = result[key].as_f64().unwrap();
            let theirs = info[key].as_f64().unwrap();
            assert!((ours - theirs).abs() < 1e-9, "{key}: {ours} vs {theirs}");
        }
        let mut expected = crate::types::header::HduHeader::empty();
        for (k, v) in nova_wcs_cards() {
            expected.set(&k, v);
        }
        expected.set("NAXIS1", "64".to_string());
        expected.set("NAXIS2", "48".to_string());
        let centre = crate::core::astrometry::wcs::WcsTransform::from_header(&expected).unwrap().pixel_to_world(31.5, 23.5);
        assert!((result["center_ra"].as_f64().unwrap() - centre.ra).abs() < 1e-9, "{} vs {}", result["center_ra"], centre.ra);
        assert!((result["center_dec"].as_f64().unwrap() - centre.dec).abs() < 1e-9, "{} vs {}", result["center_dec"], centre.dec);
        assert_eq!(result["sip_present"], true);
        assert_eq!(info["sip_present"], true);
        assert!(result["elapsed_ms"].is_u64(), "{}", result["elapsed_ms"]);
    }

    #[test]
    fn write_solved_wcs_refuses_cards_without_a_celestial_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = source_with_old_wcs(&dir, "nocrval.fits");
        let cards: Vec<(String, String)> = nova_wcs_cards().into_iter().filter(|(k, _)| k != "CRVAL1").collect();
        let out = dir.path().join("out");

        let err = format!("{:#}", super::write_solved_wcs(&path, &cards, out.to_str().unwrap()).unwrap_err());

        assert!(err.starts_with("The solved WCS is not a usable celestial WCS"), "{err}");
        assert!(err.contains("CRVAL1"), "{err}");
        assert!(!out.join("nocrval_wcs.fits").exists(), "a FITS was written despite the refusal");
        assert!(!out.exists(), "the output dir was created despite the refusal");
    }

    #[test]
    fn write_solved_wcs_refuses_a_multi_plane_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rgb_src.fits").to_string_lossy().to_string();
        let plane = ndarray::Array2::<f32>::from_elem((16, 24), 1.0);
        crate::infra::fits::writer::write_fits_rgb_bitpix(&path, &plane, &plane, &plane, None, -32).unwrap();
        let out = dir.path().join("out");

        let err = format!("{:#}", super::write_solved_wcs(&path, &nova_wcs_cards(), out.to_str().unwrap()).unwrap_err());

        assert!(err.contains("rgb_src has 3 planes"), "{err}");
        assert!(err.contains("single-plane images only"), "{err}");
        assert!(!out.exists(), "the output dir was created despite the refusal");
    }

    #[test]
    fn grid_cache_is_keyed_by_path_frame_density_and_file_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grid.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);

        let first = load_grid_cached(&path, SkyFrame::Icrs, DEFAULT_DENSITY).unwrap();
        assert_eq!(first.frame, "icrs");
        assert!(!first.lines.is_empty() && !first.labels.is_empty());
        assert!((first.lat_step_deg * 3600.0 - 15.0).abs() < 1e-9, "lat step {}", first.lat_step_deg);

        let again = load_grid_cached(&path, SkyFrame::Icrs, DEFAULT_DENSITY).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &again), "same key must hit the cache");

        let galactic = load_grid_cached(&path, SkyFrame::Galactic, DEFAULT_DENSITY).unwrap();
        assert_eq!(galactic.frame, "galactic");
        assert!(!std::sync::Arc::ptr_eq(&first, &galactic));

        let denser = load_grid_cached(&path, SkyFrame::Icrs, MAX_DENSITY).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first, &denser));
        assert!(denser.lines.len() > first.lines.len(), "{} vs {}", denser.lines.len(), first.lines.len());

        write_north_up_fits(&path, 200);
        let refreshed = load_grid_cached(&path, SkyFrame::Icrs, DEFAULT_DENSITY).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first, &refreshed), "a changed file must invalidate the cache");
        assert!(refreshed.lat_step_deg > first.lat_step_deg, "{} vs {}", refreshed.lat_step_deg, first.lat_step_deg);

        let missing = dir.path().join("missing.fits").to_string_lossy().to_string();
        assert!(load_grid_cached(&missing, SkyFrame::Icrs, DEFAULT_DENSITY).is_err());
    }

    fn footprint(ra: f64, dec: f64, size_deg: f64) -> SkyFootprint {
        let half = size_deg / 2.0;
        let cosd = dec.to_radians().cos().max(1e-6);
        let dra = half / cosd;
        SkyFootprint {
            center_ra: ra,
            center_dec: dec,
            corners: [
                (ra - dra, dec - half),
                (ra + dra, dec - half),
                (ra + dra, dec + half),
                (ra - dra, dec + half),
            ],
            fov_w_arcmin: size_deg * 60.0,
            fov_h_arcmin: size_deg * 60.0,
        }
    }

    #[test]
    fn plate_solve_uses_the_configured_timeout_and_refuses_zero() {
        let configured = AppConfig { plate_solve_timeout_secs: 45, ..AppConfig::default() };
        assert_eq!(plate_solve_timeout(Ok(configured)).unwrap(), 45);
        let fallback = plate_solve_timeout(Err(anyhow::anyhow!("unreadable config.json"))).unwrap();
        assert_eq!(fallback, AppConfig::default().plate_solve_timeout_secs);
        let zero = AppConfig { plate_solve_timeout_secs: 0, ..AppConfig::default() };
        let err = plate_solve_timeout(Ok(zero)).unwrap_err().to_string();
        assert!(err.contains("at least 1 second"), "{err}");
    }

    #[test]
    fn identical_footprints_fully_overlap() {
        let a = footprint(10.0, 5.0, 0.1);
        let b = footprint(10.0, 5.0, 0.1);
        assert!((footprint_overlap_fraction(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn disjoint_pointings_have_zero_overlap() {
        let a = footprint(6.95, 1.65, 0.04);
        let b = footprint(7.40, 1.81, 0.04);
        assert_eq!(footprint_overlap_fraction(&a, &b), 0.0);
    }

    #[test]
    fn ra_wraparound_is_handled() {
        let a = footprint(359.95, 0.0, 0.2);
        let b = footprint(0.05, 0.0, 0.2);
        let f = footprint_overlap_fraction(&a, &b);
        assert!(f > 0.4, "expected overlap across RA=0, got {}", f);
        assert!((wrap180(359.8) + 0.2).abs() < 1e-9);
    }

    #[test]
    fn high_declination_neighbors_overlap_partially() {
        let a = footprint(100.0, 80.0, 0.2);
        let b = footprint(100.0 + 0.1 / 80.0f64.to_radians().cos(), 80.0, 0.2);
        let f = footprint_overlap_fraction(&a, &b);
        assert!(f > 0.3 && f < 0.7, "expected partial overlap, got {}", f);
    }

    fn sky_of(v: &serde_json::Value, key: &str) -> (f64, f64) {
        (v[key][0].as_f64().unwrap(), v[key][1].as_f64().unwrap())
    }

    fn pair_at(v: &serde_json::Value, i: usize) -> (f64, f64) {
        (v[i][0].as_f64().unwrap(), v[i][1].as_f64().unwrap())
    }

    fn angle_diff(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    #[tokio::test]
    async fn world_to_pixel_round_trips_pixel_to_world_and_flags_off_image_points() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);

        let pixels = vec![(12.25, 77.5), (0.0, 0.0), (99.0, 99.0)];
        let sky = super::pixel_to_world_cmd(path.clone(), pixels.clone(), None).await.unwrap();
        let sky_points: Vec<(f64, f64)> = (0..pixels.len()).map(|i| pair_at(&sky["points"], i)).collect();

        let mut points = sky_points.clone();
        points.push((sky_points[0].0 + 10.0, sky_points[0].1));
        points.push((f64::NAN, 2.0));
        let back = super::world_to_pixel_cmd(path, points, Some("icrs".into())).await.unwrap();
        assert_eq!(back["frame"], "icrs");
        assert_eq!(back["naxis1"], 100);
        assert_eq!(back["naxis2"], 100);
        for (i, &(px, py)) in pixels.iter().enumerate() {
            let (x, y) = pair_at(&back["points"], i);
            assert!((x - px).abs() < 1e-6 && (y - py).abs() < 1e-6, "point {i}: ({x}, {y}) vs ({px}, {py})");
            assert_eq!(back["on_image"][i], true);
        }
        assert_eq!(back["on_image"][3], false);
        assert!(back["points"][4].is_null());
        assert_eq!(back["on_image"][4], false);
    }

    #[tokio::test]
    async fn world_to_pixel_rejects_too_many_points_and_unknown_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limits.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 10);
        let too_many = vec![(150.0, 2.0); super::MAX_SKY_POINTS + 1];
        let err = super::world_to_pixel_cmd(path.clone(), too_many, None).await.unwrap_err();
        assert!(err.contains("20000"), "{err}");
        let err = super::world_to_pixel_cmd(path, vec![(150.0, 2.0)], Some("supergalactic".into())).await.unwrap_err();
        assert!(err.contains("supergalactic"), "{err}");
    }

    #[tokio::test]
    async fn galactic_input_lands_where_its_icrs_equivalent_lands() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("galactic.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);
        let (l, b) = crate::core::astrometry::frames::icrs_to_galactic(150.0, 2.0);
        assert!((l - 236.9546).abs() < 0.01 && (b - 41.9051).abs() < 0.01, "galactic ({l}, {b})");

        let gal = super::world_to_pixel_cmd(path.clone(), vec![(l, b)], Some("galactic".into())).await.unwrap();
        let icrs = super::world_to_pixel_cmd(path, vec![(150.0, 2.0)], None).await.unwrap();
        assert_eq!(gal["frame"], "galactic");
        let (gx, gy) = pair_at(&gal["points"], 0);
        let (ix, iy) = pair_at(&icrs["points"], 0);
        assert!((gx - ix).abs() < 1e-6 && (gy - iy).abs() < 1e-6, "({gx}, {gy}) vs ({ix}, {iy})");
        assert!((ix - 49.5).abs() < 1e-6 && (iy - 49.5).abs() < 1e-6, "centre ({ix}, {iy})");
    }

    #[tokio::test]
    async fn pixel_mode_separation_measures_a_vertical_line_with_north_up_position_angles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sep.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);

        let up = super::sky_separation_cmd(Some(path.clone()), (49.5, 40.0), (49.5, 50.0), Some(true)).await.unwrap();
        assert!((up["separation_arcsec"].as_f64().unwrap() - 10.0).abs() < 1e-6, "{}", up["separation_arcsec"]);
        assert!((up["separation_arcmin"].as_f64().unwrap() - 10.0 / 60.0).abs() < 1e-7);
        assert!((up["separation_deg"].as_f64().unwrap() - 10.0 / 3600.0).abs() < 1e-9);
        let pa_up = up["position_angle_deg"].as_f64().unwrap();
        assert!(angle_diff(pa_up, 0.0) < 1e-6, "{pa_up}");
        assert!((up["pixel_length"].as_f64().unwrap() - 10.0).abs() < 1e-12);
        assert!((up["pixel_scale_arcsec"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        let (ra, dec) = sky_of(&up, "a_sky");
        assert!((ra - 150.0).abs() < 1e-6 && (dec - 2.0 + 9.5 / 3600.0).abs() < 1e-9, "a_sky ({ra}, {dec})");

        let down = super::sky_separation_cmd(Some(path.clone()), (49.5, 50.0), (49.5, 40.0), Some(true)).await.unwrap();
        let pa_down = down["position_angle_deg"].as_f64().unwrap();
        assert!(angle_diff(pa_down, 180.0) < 1e-6, "{pa_down}");

        let left = super::sky_separation_cmd(Some(path), (49.5, 49.5), (39.5, 49.5), Some(true)).await.unwrap();
        let pa_left = left["position_angle_deg"].as_f64().unwrap();
        assert!(angle_diff(pa_left, 90.0) < 1e-6, "{pa_left}");
    }

    #[tokio::test]
    async fn sky_mode_separation_needs_no_path_and_pixel_mode_validates_its_inputs() {
        let sky = super::sky_separation_cmd(None, (10.0, 20.0), (11.0, 21.0), None).await.unwrap();
        assert!((sky["position_angle_deg"].as_f64().unwrap() - 42.9531).abs() < 0.01);
        assert!(sky["pixel_length"].is_null() && sky["pixel_scale_arcsec"].is_null());
        assert_eq!(sky["a_sky"], serde_json::json!([10.0, 20.0]));

        let err = super::sky_separation_cmd(None, (0.0, 0.0), (1.0, 1.0), Some(true)).await.unwrap_err();
        assert!(err.contains("pixel points need a file path"), "{err}");
        let err = super::sky_separation_cmd(None, (f64::NAN, 0.0), (1.0, 1.0), None).await.unwrap_err();
        assert!(err.contains("point a"), "{err}");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("offsky.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 10);
        let err = super::sky_separation_cmd(Some(path), (0.0, 0.0), (f64::MAX, f64::MAX), Some(true)).await.unwrap_err();
        assert!(err.contains("does not project onto the sky"), "{err}");
    }

    fn write_fits_centred_at_dec_88(path: &str) {
        let cards: Vec<(String, String)> = wcs_cards(north_up_cd())
            .into_iter()
            .map(|(k, v)| if k == "CRVAL2" { (k, "88.0".to_string()) } else { (k, v) })
            .collect();
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let arr = ndarray::Array2::<f32>::zeros((100, 100));
        crate::infra::fits::writer::write_fits_mono(path, &arr, Some(&make_header(&pairs))).unwrap();
    }

    #[tokio::test]
    async fn sky_latitudes_beyond_the_poles_are_refused_while_pixel_rows_above_ninety_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("polar.fits").to_string_lossy().to_string();
        write_fits_centred_at_dec_88(&path);

        let err = super::world_to_pixel_cmd(path.clone(), vec![(150.0, 88.0), (150.0, 95.0)], None).await.unwrap_err();
        assert!(err.contains("points[1]") && err.contains("95"), "{err}");
        let err = super::world_to_pixel_cmd(path.clone(), vec![(10.0, -90.5)], Some("galactic".into())).await.unwrap_err();
        assert!(err.contains("points[0]") && err.contains("-90.5"), "{err}");
        let pole = super::world_to_pixel_cmd(path.clone(), vec![(150.0, 90.0), (f64::NAN, 2.0), (150.0, f64::INFINITY)], None).await.unwrap();
        assert!(pole["points"][0].is_array(), "{}", pole["points"][0]);
        assert!(pole["points"][1].is_null() && pole["points"][2].is_null(), "{}", pole["points"]);

        let err = super::sky_separation_cmd(None, (10.0, 95.0), (10.0, 20.0), None).await.unwrap_err();
        assert!(err.contains("point a") && err.contains("95"), "{err}");
        let err = super::sky_separation_cmd(None, (10.0, 20.0), (10.0, -91.0), Some(false)).await.unwrap_err();
        assert!(err.contains("point b") && err.contains("-91"), "{err}");
        let at_pole = super::sky_separation_cmd(None, (10.0, 90.0), (10.0, 20.0), None).await.unwrap();
        assert!((at_pole["separation_deg"].as_f64().unwrap() - 70.0).abs() < 1e-9, "{}", at_pole["separation_deg"]);

        let pixel_rows = super::sky_separation_cmd(Some(path), (49.5, 95.0), (49.5, 40.0), Some(true)).await.unwrap();
        assert!((pixel_rows["pixel_length"].as_f64().unwrap() - 55.0).abs() < 1e-12, "{}", pixel_rows["pixel_length"]);
    }

    #[tokio::test]
    async fn wcs_info_reports_the_orientation_of_a_north_up_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("info.fits").to_string_lossy().to_string();
        write_north_up_fits(&path, 100);
        let info = super::get_wcs_info(path).await.unwrap();
        assert_eq!(info["projection"], "TAN");
        assert_eq!(info["parity"], "normal");
        assert_eq!(info["flipped"], false);
        assert_eq!(info["sip_present"], false);
        assert!(info["rotation_deg"].as_f64().unwrap().abs() < 1e-6);
        assert!((info["pixel_scale_x_arcsec"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        let (nx, ny) = sky_of(&info, "north_vec");
        assert!(nx.abs() < 1e-6 && (ny - 1.0).abs() < 1e-6, "north ({nx}, {ny})");
        let (ex, ey) = sky_of(&info, "east_vec");
        assert!((ex + 1.0).abs() < 1e-6 && ey.abs() < 1e-6, "east ({ex}, {ey})");
    }
}
