use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, image_ref, invalidate_written, load_from_cache_or_disk, output_stem,
    render_and_save_as, resolve_output_dir, try_extract_rgb_resolved, write_derived_fits, OutputValues,
    ResolvedRgbImage, MAX_PREVIEW_DIM,
};
use crate::cmd::helpers;
use crate::cmd::processing::local_contrast::{is_display_referred, output_values_for, source_header};
use crate::cmd::processing::tone::{has_only_cube_planes, is_cube_header};
use crate::core::imaging::geometry::{apply_geometry, geometry_header, has_linear_wcs, GeometryOp};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::infra::fits::writer::write_fits_rgb;
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_IS_RGB, RES_OP, RES_ORIGINAL_DIMENSIONS, RES_PNG_PATH,
    RES_WCS_UPDATED,
};
use crate::types::header::HduHeader;

pub(crate) const GEOMETRY_CUBE_ERROR: &str = "Geometry works on a 2-D image; select a plane of the cube first.";
const COMPANIONS_HISTORY: &str = "geometry: SCI plane only; ERR/DQ not transformed";

pub(crate) fn geometry_op_error(op: &str) -> String {
    format!("Unknown geometry operation '{op}' (supported: rot90, rot180, rot270, flip_h, flip_v).")
}

fn parse_op(op: &str) -> anyhow::Result<GeometryOp> {
    GeometryOp::from_name(op).ok_or_else(|| anyhow::anyhow!(geometry_op_error(op)))
}

fn output_header(
    source: Option<&HduHeader>,
    op: GeometryOp,
    dims: (usize, usize),
    values: OutputValues,
    companions: bool,
) -> anyhow::Result<(HduHeader, bool)> {
    let empty = HduHeader::empty();
    let source = source.unwrap_or(&empty);
    let mut transformed = geometry_header(source, op, dims)?;
    if companions {
        transformed.cards.push(("HISTORY".to_string(), COMPANIONS_HISTORY.to_string()));
    }
    Ok((derived_output_header(Some(&transformed), op.name(), values), has_linear_wcs(source)))
}

fn response(
    png_path: String,
    fits_path: String,
    original: (usize, usize),
    out: (usize, usize),
    op: GeometryOp,
    is_rgb: bool,
    wcs_updated: bool,
    t0: Instant,
) -> serde_json::Value {
    json!({
        RES_PNG_PATH: png_path,
        RES_FITS_PATH: fits_path,
        RES_DIMENSIONS: [out.1, out.0],
        RES_ORIGINAL_DIMENSIONS: [original.1, original.0],
        RES_OP: op.name(),
        RES_IS_RGB: is_rgb,
        RES_WCS_UPDATED: wcs_updated,
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
    })
}

fn run_geometry_mono(path: &str, output_dir: &str, op: GeometryOp, t0: Instant) -> anyhow::Result<serde_json::Value> {
    let entry = match load_from_cache_or_disk(path) {
        Ok(entry) => entry,
        Err(_) if has_only_cube_planes(path) => anyhow::bail!(GEOMETRY_CUBE_ERROR),
        Err(e) => return Err(e),
    };
    let source = source_header(path, &entry);
    if is_cube_header(source.as_ref()) {
        anyhow::bail!(GEOMETRY_CUBE_ERROR);
    }
    let companions = entry.companions().is_some_and(|c| c.err.is_some() || c.dq.is_some());
    let transformed = apply_geometry(entry.arr(), op);
    let values = output_values_for(source.as_ref(), OutputValues::Linear);
    let (header, wcs_updated) = output_header(source.as_ref(), op, entry.arr().dim(), values, companions)?;

    let ro = render_and_save_as(&transformed, path, output_dir, op.name(), false, values)?;
    let fits_path = format!("{}/{}_{}.fits", output_dir, output_stem(path), op.name());
    write_derived_fits(&fits_path, &transformed, Some(&header))?;
    Ok(response(ro.png_path, fits_path, entry.arr().dim(), transformed.dim(), op, false, wcs_updated, t0))
}

fn run_geometry_rgb(
    rgb: &ResolvedRgbImage,
    path: &str,
    output_dir: &str,
    op: GeometryOp,
    t0: Instant,
) -> anyhow::Result<serde_json::Value> {
    let source = Some(&rgb.header);
    let [r, g, b] = [&rgb.r, &rgb.g, &rgb.b].map(|p| apply_geometry(p, op));
    let values = output_values_for(source, OutputValues::Linear);
    let (header, wcs_updated) = output_header(source, op, rgb.r.dim(), values, false)?;

    let stem = output_stem(path);
    let png_path = format!("{}/{}_{}.png", output_dir, stem, op.name());
    let fits_path = format!("{}/{}_{}.fits", output_dir, stem, op.name());
    if is_display_referred(source) {
        let unit = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        helpers::render_rgb_preview_with_stf(&r, &g, &b, unit, unit, unit, &png_path, MAX_PREVIEW_DIM)?;
    } else {
        let stats = [&r, &g, &b].map(compute_image_stats);
        let (stf, combined) =
            helpers::compute_linked_stf_with_stats(&stats[0], &stats[1], &stats[2], &AutoStfConfig::default());
        let [fr, fg, fb] = [(); 3].map(|_| make_stf_u8_fn(&stf, &combined));
        helpers::render_rgb_preview_with_stf(&r, &g, &b, fr, fg, fb, &png_path, MAX_PREVIEW_DIM)?;
    }

    crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&fits_path);
    write_fits_rgb(&fits_path, &r, &g, &b, Some(&header))?;
    invalidate_written(&fits_path);
    Ok(response(png_path, fits_path, rgb.r.dim(), r.dim(), op, true, wcs_updated, t0))
}

#[tauri::command]
pub async fn transform_geometry_cmd(path: String, output_dir: String, op: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let op = parse_op(&op)?;
        let output_dir = resolve_output_dir(&output_dir)?;
        let rgb = if image_ref(&path).is_synthetic() { None } else { try_extract_rgb_resolved(&path)? };
        match rgb {
            Some(rgb) => run_geometry_rgb(&rgb, &path, &output_dir, op, t0),
            None => run_geometry_mono(&path, &output_dir, op, t0),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use ndarray::Array2;

    use crate::cmd::common::{cached_header, extract_image_resolved, HEADER_ABPROC};
    use crate::cmd::processing::tone::write_cube;
    use crate::core::imaging::geometry::pixel_map;
    use crate::infra::fits::writer::write_fits_mono;

    fn ramp(rows: usize, cols: usize, offset: f32) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| offset + (y * cols + x) as f32)
    }

    fn wcs_header() -> HduHeader {
        let mut h = HduHeader::empty();
        h.set("CTYPE1", "RA---TAN".to_string());
        h.set("CTYPE2", "DEC--TAN".to_string());
        h.set_f64("CRVAL1", 150.0);
        h.set_f64("CRVAL2", 2.0);
        h.set_f64("CRPIX1", 2.5);
        h.set_f64("CRPIX2", 1.5);
        h.set_f64("CDELT1", -1e-4);
        h.set_f64("CDELT2", 1e-4);
        h.set_f64("PC1_1", 1.0);
        h.set_f64("PC2_2", 1.0);
        h.set_f64("PA_APER", 92.3);
        h.set_f64("PIXAR_SR", 4e-14);
        h.set_f64("SATURATE", 60000.0);
        h
    }

    fn text<'a>(header: &'a HduHeader, key: &str) -> Option<&'a str> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim())
    }

    fn history(header: &HduHeader) -> Vec<String> {
        header
            .cards
            .iter()
            .filter(|(k, _)| k.trim() == "HISTORY")
            .map(|(_, v)| v.clone())
            .collect()
    }

    #[tokio::test]
    async fn transform_geometry_rotates_a_mono_file_and_updates_its_wcs() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("frame.fits").to_str().unwrap().to_string();
        let input = ramp(4, 6, 10.0);
        write_fits_mono(&src, &input, Some(&wcs_header())).unwrap();

        let value = transform_geometry_cmd(src.clone(), out.clone(), "rot90".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert!(fits.ends_with("frame_rot90.fits"), "{fits}");
        assert!(value[RES_PNG_PATH].as_str().unwrap().ends_with("frame_rot90.png"), "{value}");
        assert!(std::path::Path::new(value[RES_PNG_PATH].as_str().unwrap()).exists());
        assert_eq!(value[RES_DIMENSIONS], json!([4, 6]));
        assert_eq!(value[RES_ORIGINAL_DIMENSIONS], json!([6, 4]));
        assert_eq!(value[RES_OP], "rot90");
        assert_eq!(value[RES_IS_RGB], false);
        assert_eq!(value[RES_WCS_UPDATED], true);

        let written = extract_image_resolved(&fits).unwrap().arr;
        assert_eq!(written, apply_geometry(&input, GeometryOp::Rot90Cw));
        assert_eq!(written.dim(), (6, 4));

        let header = cached_header(&fits).unwrap();
        let (m, t_f) = pixel_map(GeometryOp::Rot90Cw, (4, 6));
        let crpix = [m[0][0] * 2.5 + m[0][1] * 1.5 + t_f[0], m[1][0] * 2.5 + m[1][1] * 1.5 + t_f[1]];
        assert!((header.get_f64("CRPIX1").unwrap() - crpix[0]).abs() < 1e-9, "{:?}", header.get("CRPIX1"));
        assert!((header.get_f64("CRPIX2").unwrap() - crpix[1]).abs() < 1e-9, "{:?}", header.get("CRPIX2"));
        assert!(header.get("PC1_1").is_none());
        assert!(header.get("CDELT1").is_none());
        assert!(header.get_f64("CD1_2").is_some());
        assert!(header.get("PA_APER").is_none());
        assert_eq!(header.get_i64("NAXIS1"), Some(4));
        assert_eq!(header.get_i64("NAXIS2"), Some(6));
        assert_eq!(text(&header, HEADER_ABPROC), Some("rot90"));
        assert!((header.get_f64("PIXAR_SR").unwrap() - 4e-14).abs() < 1e-28);
        assert_eq!(header.get_f64("SATURATE"), Some(60000.0));
        let notes = history(&header);
        assert!(notes.iter().any(|h| h.contains("PC/CDELT converted to CD")), "{notes:?}");
        assert!(notes.iter().any(|h| h.contains("removed stale orientation cards PA_APER")), "{notes:?}");
        assert!(notes.iter().all(|h| !h.contains("ERR/DQ")), "{notes:?}");

        let plain = dir.path().join("plain.fits").to_str().unwrap().to_string();
        write_fits_mono(&plain, &input, None).unwrap();
        let value = transform_geometry_cmd(plain, out, "flip_v".to_string()).await.unwrap();
        assert_eq!(value[RES_WCS_UPDATED], false);
        assert_eq!(value[RES_DIMENSIONS], json!([6, 4]));
        let written = extract_image_resolved(value[RES_FITS_PATH].as_str().unwrap()).unwrap().arr;
        assert_eq!(written, apply_geometry(&input, GeometryOp::FlipV));
    }

    #[tokio::test]
    async fn transform_geometry_on_an_rgb_fits_transforms_three_planes() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("colour.fits").to_str().unwrap().to_string();
        let (r, g, b) = (ramp(5, 7, 1.0), ramp(5, 7, 100.0), ramp(5, 7, 1000.0));
        write_fits_rgb(&src, &r, &g, &b, Some(&wcs_header())).unwrap();

        let value = transform_geometry_cmd(src, out, "flip_h".to_string()).await.unwrap();
        assert_eq!(value[RES_IS_RGB], true);
        assert_eq!(value[RES_OP], "flip_h");
        assert_eq!(value[RES_DIMENSIONS], json!([7, 5]));
        assert_eq!(value[RES_WCS_UPDATED], true);
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert!(fits.ends_with("colour_flip_h.fits"), "{fits}");
        let written = try_extract_rgb_resolved(&fits).unwrap().expect("a 3-plane RGB FITS");
        assert_eq!(written.r, apply_geometry(&r, GeometryOp::FlipH));
        assert_eq!(written.g, apply_geometry(&g, GeometryOp::FlipH));
        assert_eq!(written.b, apply_geometry(&b, GeometryOp::FlipH));
        assert!((written.header.get_f64("CD1_1").unwrap() - 1e-4).abs() < 1e-18, "{:?}", written.header.get("CD1_1"));
        assert!(std::path::Path::new(value[RES_PNG_PATH].as_str().unwrap()).exists());
    }

    #[tokio::test]
    async fn transform_geometry_rewrites_the_bayer_pattern_of_a_cfa_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("mosaic.fits").to_str().unwrap().to_string();
        let mut header = HduHeader::empty();
        header.set("BAYERPAT", "'RGGB'".to_string());
        header.set("XBAYROFF", "0".to_string());
        header.set("YBAYROFF", "0".to_string());
        write_fits_mono(&src, &ramp(4, 6, 0.0), Some(&header)).unwrap();

        let value = transform_geometry_cmd(src, out, "flip_h".to_string()).await.unwrap();
        let written = cached_header(value[RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert_eq!(text(&written, "BAYERPAT"), Some("GRBG"));
        assert!(written.get("XBAYROFF").is_none() && written.get("YBAYROFF").is_none(), "{:?}", written.cards);
        assert!(history(&written).iter().any(|h| h.contains("CFA pattern RGGB -> GRBG")), "{:?}", history(&written));
    }

    #[tokio::test]
    async fn cubes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let cube = dir.path().join("cube.fits");
        write_cube(&cube, 8, 6, 5);
        let refused = transform_geometry_cmd(cube.to_str().unwrap().to_string(), out.clone(), "rot180".to_string()).await;
        assert_eq!(refused.unwrap_err(), GEOMETRY_CUBE_ERROR);
        let plane = transform_geometry_cmd(format!("{}#hdu=0", cube.to_str().unwrap()), out, "rot180".to_string()).await;
        assert_eq!(plane.unwrap_err(), GEOMETRY_CUBE_ERROR);
    }

    #[tokio::test]
    async fn unknown_ops_are_refused_before_loading() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let missing = dir.path().join("absent.fits").to_str().unwrap().to_string();
        let err = transform_geometry_cmd(missing, out, "rotate".to_string()).await.unwrap_err();
        assert_eq!(err, "Unknown geometry operation 'rotate' (supported: rot90, rot180, rot270, flip_h, flip_v).");
    }
}
