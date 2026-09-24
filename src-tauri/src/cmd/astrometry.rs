use serde_json::json;

use crate::cmd::common::{blocking_cmd, image_ref, source_path};
use crate::core::astrometry::frames::{convert_from_icrs, SkyFrame};
use crate::core::astrometry::grid::{wcs_grid, WcsGrid, DEFAULT_DENSITY, MAX_DENSITY, MIN_DENSITY};
use crate::core::astrometry::wcs::{angular_separation, pixel_center, pixel_edge_corners, WcsTransform};
use crate::infra::config;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::image_source::{load_plane, load_plane_header};
use crate::types::constants::{
    DEFAULT_API_KEY_SERVICE, HEADER_NAXIS1,
    HEADER_NAXIS2, RES_CENTER_DEC, RES_CENTER_RA, RES_FOV_ARCMIN,
    RES_FOV_H_ARCMIN, RES_FOV_W_ARCMIN, RES_FRAME, RES_NAXIS1, RES_NAXIS2,
    RES_PIXEL_SCALE_ARCSEC, RES_POINTS,
};
use crate::types::config::AppConfig;

const MAX_UPLOAD_DIM: usize = 2048;

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

        Ok(json!({
            RES_CENTER_RA: center.ra,
            RES_CENTER_DEC: center.dec,
            RES_PIXEL_SCALE_ARCSEC: pixel_scale,
            RES_FOV_W_ARCMIN: fov_w,
            RES_FOV_H_ARCMIN: fov_h,
            RES_FOV_ARCMIN: [fov_w, fov_h],
            RES_NAXIS1: naxis1,
            RES_NAXIS2: naxis2,
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
}
