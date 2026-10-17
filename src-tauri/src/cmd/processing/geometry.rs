use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, image_ref, invalidate_written, load_companions, load_from_cache_or_disk,
    output_stem, render_and_save_as, resolve_output_dir, source_primary_header, try_extract_rgb_resolved,
    write_derived_fits, write_derived_mef, OutputValues, ResolvedRgbImage, MAX_PREVIEW_DIM,
};
use crate::cmd::helpers;
use crate::cmd::processing::local_contrast::{is_display_referred, output_values_for, source_header};
use crate::cmd::processing::tone::{has_only_cube_planes, is_cube_header};
use crate::core::imaging::cfa::{remove_cfa_cards, CfaPattern};
use crate::core::imaging::dq_flags::TELESCOPE_KEYS;
use crate::core::imaging::geometry::{
    apply_geometry, apply_geometry_generic, geometry_header, has_linear_wcs, GeometryOp,
};
use crate::core::imaging::push_history;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::core::ramp::product::PRIMARY_KEYS_DROPPED;
use crate::infra::cache::ImageEntry;
use crate::infra::fits::writer::{write_fits_rgb, HduData, ImageHdu};
use crate::infra::image_source::PlaneInfo;
use crate::types::constants::{
    EXTNAME_DQ, EXTNAME_ERR, EXTNAME_SCI, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_IS_RGB, RES_OP,
    RES_ORIGINAL_DIMENSIONS, RES_PNG_PATH, RES_WCS_UPDATED,
};
use crate::types::header::HduHeader;
use crate::types::image::IntPlane;
use crate::types::image_ref::PlaneSelector;

pub(crate) const GEOMETRY_CUBE_ERROR: &str = "Geometry works on a 2-D image; select a plane of the cube first.";
const ERR_EXTENSION_CARDS: &[&str] = &["BUNIT"];
const ASDF_SCIENCE_KEY: &str = "data";
const STALE_XTRANS_CARDS_HISTORY: &str = "geometry: X-Trans CFA cards removed (the planes are already debayered)";

#[derive(Default)]
struct CarriedPlanes {
    err: Option<(Array2<f32>, HduHeader)>,
    dq: Option<(IntPlane, HduHeader)>,
    notes: Vec<String>,
}

impl CarriedPlanes {
    fn is_empty(&self) -> bool {
        self.err.is_none() && self.dq.is_none()
    }
}

pub(crate) fn geometry_op_error(op: &str) -> String {
    format!("Unknown geometry operation '{op}' (supported: rot90, rot180, rot270, flip_h, flip_v).")
}

fn parse_op(op: &str) -> anyhow::Result<GeometryOp> {
    GeometryOp::from_name(op).ok_or_else(|| anyhow::anyhow!(geometry_op_error(op)))
}

fn plane_names(err: bool, dq: bool) -> Option<&'static str> {
    match (err, dq) {
        (true, true) => Some("ERR and DQ"),
        (true, false) => Some("ERR"),
        (false, true) => Some("DQ"),
        (false, false) => None,
    }
}

fn copied_cards(source: Option<&HduHeader>, keys: &[&str]) -> HduHeader {
    let mut out = HduHeader::empty();
    let Some(source) = source else { return out };
    for key in keys {
        if let Some(value) = source.get(key) {
            out.set(key, value.to_string());
        }
    }
    out.inherit_string_keys(source);
    out
}

fn is_science_plane(info: Option<&PlaneInfo>) -> bool {
    match info {
        Some(PlaneInfo { kind: PlaneSelector::Array(key), .. }) => {
            key.rsplit('.').next().is_some_and(|last| last.eq_ignore_ascii_case(ASDF_SCIENCE_KEY))
        }
        Some(PlaneInfo { extname: Some(name), .. }) => name.trim().eq_ignore_ascii_case(EXTNAME_SCI),
        _ => true,
    }
}

fn carried_planes(path: &str, entry: &ImageEntry, op: GeometryOp) -> anyhow::Result<CarriedPlanes> {
    let refs = entry.companions().cloned().unwrap_or_default();
    if !is_science_plane(entry.plane_info()) || (refs.err.is_none() && refs.dq.is_none()) {
        return Ok(CarriedPlanes::default());
    }
    let loaded = load_companions(path)?;
    let err = loaded.err.map(|e| (apply_geometry(e.arr(), op), copied_cards(e.header(), ERR_EXTENSION_CARDS)));
    let dq = loaded.dq.and_then(|(e, _)| {
        let plane = e.int_plane()?;
        let bits = apply_geometry_generic(&plane.bits, op);
        Some((IntPlane { bits, signed: plane.signed }, copied_cards(e.header(), TELESCOPE_KEYS)))
    });
    let carried =
        plane_names(err.is_some(), dq.is_some()).map(|names| format!("geometry: {names} transformed with SCI"));
    let unreadable = plane_names(refs.err.is_some() && err.is_none(), refs.dq.is_some() && dq.is_none())
        .map(|names| format!("geometry: {names} not transformed (the plane could not be read)"));
    Ok(CarriedPlanes { err, dq, notes: carried.into_iter().chain(unreadable).collect() })
}

fn output_header(
    source: Option<&HduHeader>,
    op: GeometryOp,
    dims: (usize, usize),
    values: OutputValues,
    notes: &[String],
) -> anyhow::Result<(HduHeader, bool)> {
    let empty = HduHeader::empty();
    let source = source.unwrap_or(&empty);
    let mut transformed = geometry_header(source, op, dims)?;
    for note in notes {
        push_history(&mut transformed, note);
    }
    Ok((derived_output_header(Some(&transformed), op.name(), values), has_linear_wcs(source)))
}

fn without_source_file_cards(mut header: HduHeader) -> HduHeader {
    for key in PRIMARY_KEYS_DROPPED {
        header.remove(key);
    }
    header
}

fn write_with_companions(
    fits_path: &str,
    primary: &HduHeader,
    sci: &Array2<f32>,
    sci_header: &HduHeader,
    carried: &CarriedPlanes,
) -> anyhow::Result<()> {
    let signed_dq =
        carried.dq.as_ref().filter(|(plane, _)| plane.signed).map(|(plane, _)| plane.bits.mapv(|b| b as i32));
    let mut hdus =
        vec![ImageHdu { data: HduData::F32(sci), header: Some(sci_header), extname: EXTNAME_SCI, extver: 1 }];
    if let Some((arr, header)) = &carried.err {
        hdus.push(ImageHdu { data: HduData::F32(arr), header: Some(header), extname: EXTNAME_ERR, extver: 1 });
    }
    if let Some((plane, header)) = &carried.dq {
        let data = match &signed_dq {
            Some(bits) => HduData::I32(bits),
            None => HduData::U32(&plane.bits),
        };
        hdus.push(ImageHdu { data, header: Some(header), extname: EXTNAME_DQ, extver: 1 });
    }
    write_derived_mef(fits_path, Some(primary), &hdus)
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
    let carried = carried_planes(path, &entry, op)?;
    let transformed = apply_geometry(entry.arr(), op);
    let values = output_values_for(source.as_ref(), OutputValues::Linear);
    let (header, wcs_updated) = output_header(source.as_ref(), op, entry.arr().dim(), values, &carried.notes)?;

    let ro = render_and_save_as(&transformed, path, output_dir, op.name(), false, values)?;
    let fits_path = format!("{}/{}_{}.fits", output_dir, output_stem(path), op.name());
    if carried.is_empty() {
        write_derived_fits(&fits_path, &transformed, Some(&header))?;
    } else {
        let primary = without_source_file_cards(source_primary_header(path, source.as_ref()));
        write_with_companions(&fits_path, &primary, &transformed, &without_source_file_cards(header), &carried)?;
    }
    Ok(response(ro.png_path, fits_path, entry.arr().dim(), transformed.dim(), op, false, wcs_updated, t0))
}

fn without_stale_xtrans_cards(header: &HduHeader) -> HduHeader {
    let mut planes = header.clone();
    if CfaPattern::declared(header).is_some_and(|p| p.is_xtrans()) {
        remove_cfa_cards(&mut planes);
        push_history(&mut planes, STALE_XTRANS_CARDS_HISTORY);
    }
    planes
}

fn run_geometry_rgb(
    rgb: &ResolvedRgbImage,
    path: &str,
    output_dir: &str,
    op: GeometryOp,
    t0: Instant,
) -> anyhow::Result<serde_json::Value> {
    let planes_header = without_stale_xtrans_cards(&rgb.header);
    let source = Some(&planes_header);
    let [r, g, b] = [&rgb.r, &rgb.g, &rgb.b].map(|p| apply_geometry(p, op));
    let values = output_values_for(source, OutputValues::Linear);
    let (header, wcs_updated) = output_header(source, op, rgb.r.dim(), values, &[])?;

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

    use crate::cmd::common::{
        cached_header, dq_exclusion, extract_image_resolved, load_cached, load_companions, plane_info_json,
        HEADER_ABPROC,
    };
    use crate::cmd::processing::tone::write_cube;
    use crate::core::imaging::dq_flags::DqTable;
    use crate::core::imaging::geometry::{apply_geometry_generic, pixel_map};
    use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData as TestData, TestHdu};
    use crate::infra::fits::reader::{
        extract_header_by_index, extract_image_mmap_by_index, extract_int_plane_by_index, list_extensions,
    };
    use crate::infra::fits::writer::write_fits_mono;
    use crate::infra::image_source::test_support::{write_asdf, ROMAN_TREE};
    use crate::types::constants::{RES_DQ_REF, RES_DQ_TABLE};

    const ROWS: usize = 4;
    const COLS: usize = 6;
    const DO_NOT_USE_AT: (usize, usize) = (1, 2);
    const SATURATED_AT: (usize, usize) = (3, 0);

    fn ramp(rows: usize, cols: usize, offset: f32) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| offset + (y * cols + x) as f32)
    }

    fn source_dq() -> Array2<u32> {
        let mut dq = Array2::<u32>::zeros((ROWS, COLS));
        dq[DO_NOT_USE_AT] = 1;
        dq[SATURATED_AT] = 2;
        dq
    }

    fn unsigned_dq_hdu_data() -> (TestData, Vec<(&'static str, String)>) {
        let raw = source_dq().iter().map(|&b| (b as i64 - 2_147_483_648) as i32).collect();
        (TestData::I32(raw), vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into())])
    }

    fn jwst_cal_mef(dir: &tempfile::TempDir, (dq, dq_cards): (TestData, Vec<(&'static str, String)>)) -> String {
        let path = dir.path().join("frame.fits");
        let plane = |offset: f32| ramp(ROWS, COLS, offset).iter().copied().collect::<Vec<f32>>();
        let sci_cards: Vec<(&'static str, String)> = vec![
            ("BUNIT", "'MJy/sr'".into()),
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRVAL1", "150.0".into()),
            ("CRVAL2", "2.0".into()),
            ("CRPIX1", "2.5".into()),
            ("CRPIX2", "1.5".into()),
            ("CD1_1", "-1.0E-4".into()),
            ("CD2_2", "1.0E-4".into()),
        ];
        let hdu = |extname, data, extra_cards| TestHdu { extname: Some(extname), extver: Some(1), cols: COLS, rows: ROWS, data, extra_cards };
        write_test_mef(
            &path,
            &[
                ("TELESCOP", "'JWST'".into()),
                ("INSTRUME", "'NIRCAM'".into()),
                ("PROGRAM", "'01234'".into()),
                ("DATAMODL", "'ImageModel'".into()),
                ("FILENAME", "'frame_cal.fits'".into()),
                ("DATE", "'2024-05-01T12:00:00.000'".into()),
                ("NEXTEND", "6".into()),
            ],
            &[
                hdu("SCI", TestData::F32(plane(10.0)), sci_cards),
                hdu("ERR", TestData::F32(plane(0.5)), vec![("BUNIT", "'MJy/sr'".into())]),
                hdu("DQ", dq, dq_cards),
            ],
        );
        path.to_str().unwrap().to_string()
    }

    fn mapped(op: GeometryOp, dims: (usize, usize), (y, x): (usize, usize)) -> (usize, usize) {
        let (m, t) = pixel_map(op, dims);
        let (fx, fy) = (x as f64 + 1.0, y as f64 + 1.0);
        let xm = m[0][0] * fx + m[0][1] * fy + t[0] - 1.0;
        let ym = m[1][0] * fx + m[1][1] * fy + t[1] - 1.0;
        (ym.round() as usize, xm.round() as usize)
    }

    fn extnames(path: &str) -> Vec<Option<String>> {
        let file = std::fs::File::open(path).unwrap();
        list_extensions(&file).unwrap().into_iter().map(|e| e.extname).collect()
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
    async fn an_rgb_fits_with_a_stale_xtrans_card_is_transformed_and_the_cards_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("debayered.fits").to_str().unwrap().to_string();
        let (r, g, b) = (ramp(6, 9, 1.0), ramp(6, 9, 100.0), ramp(6, 9, 1000.0));
        let mut header = wcs_header();
        header.set("BAYERPAT", format!("'{}'", crate::core::imaging::cfa::test_fixtures::XTRANS_LAYOUT));
        header.set("XBAYROFF", "1".to_string());
        write_fits_rgb(&src, &r, &g, &b, Some(&header)).unwrap();

        let value = transform_geometry_cmd(src, out, "rot90".to_string()).await.unwrap();
        assert_eq!(value[RES_IS_RGB], true);
        let written = try_extract_rgb_resolved(value[RES_FITS_PATH].as_str().unwrap()).unwrap().expect("RGB");
        assert_eq!(written.r, apply_geometry(&r, GeometryOp::Rot90Cw));
        for key in ["BAYERPAT", "COLORTYP", "XBAYROFF", "YBAYROFF"] {
            assert!(written.header.get(key).is_none(), "{key} kept");
        }
        assert!(
            history(&written.header).iter().any(|h| h.contains("X-Trans CFA cards removed")),
            "{:?}",
            history(&written.header)
        );

        let mosaic = dir.path().join("mosaic.fits").to_str().unwrap().to_string();
        write_fits_mono(&mosaic, &ramp(6, 9, 0.0), Some(&header)).unwrap();
        let refused = transform_geometry_cmd(mosaic, dir.path().to_str().unwrap().to_string(), "rot90".to_string()).await;
        assert!(refused.is_err_and(|e| e.contains("X-Trans")));
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

    #[tokio::test]
    async fn transform_geometry_carries_err_and_dq_as_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = jwst_cal_mef(&dir, unsigned_dq_hdu_data());
        let op = GeometryOp::Rot90Cw;

        let value = transform_geometry_cmd(src, out, "rot90".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert!(fits.ends_with("frame_rot90.fits"), "{fits}");
        let file = std::fs::File::open(&fits).unwrap();
        let exts = list_extensions(&file).unwrap();
        let layout: Vec<(Option<&str>, Option<i64>)> = exts.iter().map(|e| (e.extname.as_deref(), e.extver)).collect();
        assert_eq!(layout, vec![(None, None), (Some("SCI"), Some(1)), (Some("ERR"), Some(1)), (Some("DQ"), Some(1))]);
        assert_eq!(exts[0].naxis, 0);

        let primary = extract_header_by_index(&file, 0).unwrap();
        assert_eq!(text(&primary, "TELESCOP"), Some("JWST"));
        assert!(primary.get("CRPIX1").is_none() && primary.get("BUNIT").is_none(), "{:?}", primary.cards);
        assert_eq!(text(&primary, "PROGRAM"), Some("01234"));
        for stale in ["DATAMODL", "FILENAME", "DATE", "NEXTEND"] {
            assert!(primary.get(stale).is_none(), "{stale}: {:?}", primary.cards);
        }

        assert_eq!(extract_image_mmap_by_index(&file, 2).unwrap().image, apply_geometry(&ramp(ROWS, COLS, 0.5), op));
        assert_eq!(text(&extract_header_by_index(&file, 2).unwrap(), "BUNIT"), Some("MJy/sr"));

        let dq = extract_int_plane_by_index(&file, 3).unwrap();
        assert!(!dq.signed);
        assert_eq!(dq.bits, apply_geometry_generic(&source_dq(), op));
        let dq_header = extract_header_by_index(&file, 3).unwrap();
        assert_eq!(text(&dq_header, "TELESCOP"), Some("JWST"));
        assert_eq!(text(&dq_header, "INSTRUME"), Some("NIRCAM"));
        assert!(dq_header.get("BUNIT").is_none(), "{:?}", dq_header.cards);

        let companions = load_companions(&fits).unwrap();
        assert_eq!(companions.err.as_ref().map(|e| e.arr().dim()), Some((COLS, ROWS)));
        let (dq_entry, table) = companions.dq.expect("the derived file has a DQ companion");
        assert_eq!(dq_entry.arr().dim(), (COLS, ROWS));
        assert_eq!(table, DqTable::Jwst);
        let excluded = dq_exclusion(&fits).unwrap().expect("a DQ exclusion map");
        let flagged: Vec<(usize, usize)> = excluded.indexed_iter().filter(|(_, v)| **v != 0).map(|(p, _)| p).collect();
        assert_eq!(flagged, vec![mapped(op, (ROWS, COLS), DO_NOT_USE_AT)]);

        let header = cached_header(&fits).unwrap();
        let (m, t_f) = pixel_map(op, (ROWS, COLS));
        assert!((header.get_f64("CRPIX1").unwrap() - (m[0][0] * 2.5 + m[0][1] * 1.5 + t_f[0])).abs() < 1e-9);
        assert!((header.get_f64("CRPIX2").unwrap() - (m[1][0] * 2.5 + m[1][1] * 1.5 + t_f[1])).abs() < 1e-9);
        let notes = history(&header);
        assert!(notes.iter().any(|h| h.contains("geometry: ERR and DQ transformed with SCI")), "{notes:?}");
        assert!(notes.iter().all(|h| !h.contains("not transformed")), "{notes:?}");
        for stale in ["DATAMODL", "FILENAME", "DATE", "NEXTEND"] {
            assert!(header.get(stale).is_none(), "{stale}: {:?}", header.cards);
        }
        assert_eq!(text(&header, "TELESCOP"), Some("JWST"));

        let sci_header = extract_header_by_index(&file, 1).unwrap();
        let stamp = format!("Processed with AstroBurst {}", env!("CARGO_PKG_VERSION"));
        assert_eq!(history(&sci_header).iter().filter(|h| h.trim() == stamp).count(), 1, "{:?}", sci_header.cards);
        assert_eq!(text(&sci_header, "PROGRAM"), Some("01234"));

        let info = plane_info_json(&fits, &load_cached(&fits).unwrap()).unwrap();
        assert!(info[RES_DQ_REF].is_string(), "{info}");
        assert_eq!(info[RES_DQ_TABLE], "jwst", "{info}");
    }

    #[tokio::test]
    async fn transform_geometry_keeps_a_signed_dq_signed() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let mut dq = source_dq().mapv(|b| b as i32);
        dq[[0, 5]] = -1;
        let src = jwst_cal_mef(&dir, (TestData::I32(dq.iter().copied().collect()), vec![]));

        let value = transform_geometry_cmd(src, out, "rot270".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        let written = extract_int_plane_by_index(&std::fs::File::open(&fits).unwrap(), 3).unwrap();
        assert!(written.signed);
        assert_eq!(written.bits.mapv(|b| b as i32), apply_geometry_generic(&dq, GeometryOp::Rot270Cw));
    }

    #[tokio::test]
    async fn transform_geometry_on_a_roman_asdf_carries_dq() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = write_asdf(&dir, "roman.asdf", ROMAN_TREE);
        assert_eq!(load_companions(&src).unwrap().dq.map(|(_, t)| t), Some(DqTable::Roman));

        let value = transform_geometry_cmd(src, out, "flip_h".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert_eq!(extnames(&fits), vec![None, Some("SCI".to_string()), Some("DQ".to_string())]);
        let file = std::fs::File::open(&fits).unwrap();
        assert_eq!(text(&extract_header_by_index(&file, 2).unwrap(), "INSTRUME"), Some("WFI"));

        let companions = load_companions(&fits).unwrap();
        assert!(companions.err.is_none());
        assert_eq!(companions.dq.map(|(_, t)| t), Some(DqTable::Roman));
        assert_eq!(dq_exclusion(&fits).unwrap(), Some(ndarray::array![[1u8, 0], [0, 0]]));
        let header = cached_header(&fits).unwrap();
        let notes = history(&header);
        assert!(notes.iter().any(|h| h.contains("geometry: DQ transformed with SCI")), "{notes:?}");
        let version = env!("CARGO_PKG_VERSION");
        assert!(notes.iter().any(|h| h.trim() == format!("Processed with AstroBurst {version}")), "{notes:?}");
        assert_eq!(text(&header, "PROGRAM"), Some(format!("AstroBurst {version}").as_str()));
    }

    #[tokio::test]
    async fn transform_geometry_without_companions_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("plain.fits").to_str().unwrap().to_string();
        let input = ramp(ROWS, COLS, 10.0);
        write_fits_mono(&src, &input, Some(&wcs_header())).unwrap();

        let value = transform_geometry_cmd(src, out, "rot90".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert_eq!(extnames(&fits), vec![None]);
        let file = std::fs::File::open(&fits).unwrap();
        assert_eq!(list_extensions(&file).unwrap()[0].naxis, 2);
        assert_eq!(extract_image_resolved(&fits).unwrap().arr, apply_geometry(&input, GeometryOp::Rot90Cw));
        let notes = history(&cached_header(&fits).unwrap());
        assert!(notes.iter().all(|h| !h.contains("ERR") && !h.contains("DQ")), "{notes:?}");
        let companions = load_companions(&fits).unwrap();
        assert!(companions.err.is_none() && companions.dq.is_none());
    }

    #[tokio::test]
    async fn transform_geometry_of_a_dq_plane_writes_that_plane_alone() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = jwst_cal_mef(&dir, unsigned_dq_hdu_data());

        let value = transform_geometry_cmd(format!("{src}#hdu=3"), out, "rot180".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert_eq!(extnames(&fits), vec![None]);
        let expected = apply_geometry_generic(&source_dq(), GeometryOp::Rot180).mapv(|b| b as f32);
        assert_eq!(extract_image_resolved(&fits).unwrap().arr, expected);
        let notes = history(&cached_header(&fits).unwrap());
        assert!(notes.iter().all(|h| !h.contains("ERR") && !h.contains("DQ")), "{notes:?}");
    }

    #[tokio::test]
    async fn transform_geometry_of_a_variance_plane_writes_that_plane_alone() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let path = dir.path().join("cal.fits");
        let plane = |offset: f32| TestData::F32(ramp(ROWS, COLS, offset).iter().copied().collect());
        let hdu = |extname, data| TestHdu { extname: Some(extname), extver: Some(1), cols: COLS, rows: ROWS, data, extra_cards: vec![] };
        let (dq, dq_cards) = unsigned_dq_hdu_data();
        write_test_mef(
            &path,
            &[("TELESCOP", "'JWST'".into())],
            &[
                hdu("SCI", plane(10.0)),
                hdu("ERR", plane(0.5)),
                TestHdu { extra_cards: dq_cards, ..hdu("DQ", dq) },
                hdu("VAR_POISSON", plane(1000.0)),
            ],
        );
        let src = path.to_str().unwrap().to_string();

        let value = transform_geometry_cmd(format!("{src}#hdu=4"), out.clone(), "rot180".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert_eq!(extnames(&fits), vec![None]);
        let expected = apply_geometry(&ramp(ROWS, COLS, 1000.0), GeometryOp::Rot180);
        assert_eq!(extract_image_resolved(&fits).unwrap().arr, expected);
        let notes = history(&cached_header(&fits).unwrap());
        assert!(notes.iter().all(|h| !h.contains("transformed with SCI")), "{notes:?}");

        let tree = "roman:\n  meta:\n    instrument: {name: WFI}\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  dq: !core/ndarray-1.0.0\n    data: [[0, 1], [0, 0]]\n    datatype: uint32\n  var_poisson: !core/ndarray-1.0.0\n    data: [[5, 6], [7, 8]]\n    datatype: float32\n";
        let asdf = write_asdf(&dir, "variance.asdf", tree);
        let value = transform_geometry_cmd(format!("{asdf}#array=roman.var_poisson"), out, "flip_h".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert_eq!(extnames(&fits), vec![None]);
        assert_eq!(extract_image_resolved(&fits).unwrap().arr, ndarray::array![[6.0f32, 5.0], [8.0, 7.0]]);
    }

    #[tokio::test]
    async fn transform_geometry_notes_a_companion_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let float_dq = TestData::F32(source_dq().iter().map(|&b| b as f32).collect());
        let src = jwst_cal_mef(&dir, (float_dq, vec![]));

        let value = transform_geometry_cmd(src, out, "flip_v".to_string()).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert_eq!(extnames(&fits), vec![None, Some("SCI".to_string()), Some("ERR".to_string())]);
        let notes = history(&cached_header(&fits).unwrap());
        assert!(notes.iter().any(|h| h.contains("geometry: ERR transformed with SCI")), "{notes:?}");
        assert!(notes.iter().any(|h| h.contains("geometry: DQ not transformed")), "{notes:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn rot180_of_a_nircam_cal_keeps_photometry_errors_and_dq_masking() {
        use crate::cmd::analysis::photometry_for_path;
        use crate::core::astrometry::gwcs::test_support::{real_data_dir, skip_if_absent};

        let cal = real_data_dir().join("jw02739001001_02105_00001_nrca1_cal.fits");
        if skip_if_absent(&cal) {
            return;
        }
        let src = cal.to_string_lossy().into_owned();
        let dir = tempfile::tempdir().unwrap();
        let op = GeometryOp::Rot180;
        let value = transform_geometry_cmd(src.clone(), dir.path().to_string_lossy().into_owned(), op.name().to_string())
            .await
            .unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        assert_eq!(extnames(&fits), vec![None, Some("SCI".to_string()), Some("ERR".to_string()), Some("DQ".to_string())]);

        let sci = load_cached(&src).unwrap();
        let dims = sci.arr().dim();
        let source_excluded = dq_exclusion(&src).unwrap().expect("the cal has a DQ plane");
        assert_eq!(dq_exclusion(&fits).unwrap(), Some(apply_geometry_generic(&source_excluded, op)));
        let margin = 64;
        let masked_near = |(y, x): (usize, usize)| (y - 3..=y + 3).any(|j| (x - 3..=x + 3).any(|i| source_excluded[[j, i]] != 0));
        let (y, x) = (margin..dims.0 - margin)
            .flat_map(|y| (margin..dims.1 - margin).map(move |x| (y, x)))
            .filter(|&p| source_excluded[p] == 0 && sci.arr()[p].is_finite() && masked_near(p))
            .max_by(|&a, &b| sci.arr()[a].total_cmp(&sci.arr()[b]))
            .unwrap();
        let (ym, xm) = mapped(op, dims, (y, x));

        for exclude_dq in [false, true] {
            let measure = |path: &str, px: usize, py: usize| {
                photometry_for_path(path, px as f64, py as f64, Some(5.0), None, false, exclude_dq, None).unwrap()
                    [crate::types::constants::RES_PHOTOMETRY]
                    .clone()
            };
            let before = measure(&src, x, y);
            let after = measure(&fits, xm, ym);
            let number = |v: &serde_json::Value, key: &str| v[key].as_f64().unwrap_or_else(|| panic!("{key}: {v}"));
            let relative = |key: &str| {
                let (a, b) = (number(&before, key), number(&after, key));
                (a - b).abs() / a.abs().max(f64::MIN_POSITIVE)
            };
            assert_eq!(before["err_used"], true, "{before}");
            assert_eq!(after["err_used"], true, "{after}");
            assert!(relative("net_flux") <= 1e-6, "net_flux {} vs {}", before["net_flux"], after["net_flux"]);
            assert!(relative("flux_err") <= 1e-6, "flux_err {} vs {}", before["flux_err"], after["flux_err"]);
            assert_eq!(before["n_masked"], after["n_masked"], "exclude_dq {exclude_dq}");
            assert_eq!(number(&after, "n_masked") > 0.0, exclude_dq, "{after}");
            let centre = (number(&after, "x"), number(&after, "y"));
            let expected = (dims.1 as f64 - 1.0 - number(&before, "x"), dims.0 as f64 - 1.0 - number(&before, "y"));
            assert!((centre.0 - expected.0).abs() < 1e-6 && (centre.1 - expected.1).abs() < 1e-6, "{centre:?} vs {expected:?}");
            eprintln!(
                "exclude_dq {exclude_dq}: ({x}, {y}) -> ({xm}, {ym}) net_flux {} / {} flux_err {} / {} n_masked {}",
                before["net_flux"], after["net_flux"], before["flux_err"], after["flux_err"], after["n_masked"]
            );
        }
    }
}
