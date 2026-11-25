use std::sync::Arc;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::catalog::uncalibrated_reason;
use crate::cmd::common::{extract_image_resolved, try_extract_rgb_resolved};
use crate::cmd::helpers;
use crate::core::imaging::stats::{combine_channel_stats, compute_image_stats};
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig, ImageStats, StfParams};
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::types::constants::{RES_APPLY_STF, RES_DISPLAY_REFERRED, RES_UNCOMPRESSED, RES_WCS_WRITTEN};
use crate::types::header::HduHeader;

use super::rgb_sources::{fits_rgb_uses_composite, png_rgb_uses_composite};

use super::*;
use std::fs::File;
use crate::cmd::common::{load_cached, load_cached_full};
use crate::infra::fits::reader::{extract_image_mmap, parse_header_at};
use crate::infra::fits::reader::test_fixtures::{ramp_f32, sci_err_dq_mef, write_test_mef, HduData, TestHdu};
use crate::types::constants::{
    BLOCK_SIZE, RES_DROPPED, RES_KEPT_RAW, RES_OUTPUT_PATH, RES_OUTPUT_SIZE_BYTES,
    RES_SOURCE_DECOMPRESSED_BYTES, RES_SOURCE_SIZE_BYTES,
};

fn calibrated_source(dir: &tempfile::TempDir) -> String {
    let src = dir.path().join("calibrated_sci.fits");
    let cards = [
        ("BUNIT", "'MJy/sr'"),
        ("PHOTMJSR", "1.5"),
        ("PIXAR_SR", "2.0E-13"),
        ("CTYPE1", "'RA---TAN'"),
        ("CTYPE2", "'DEC--TAN'"),
        ("CRVAL1", "10.0"),
        ("CRVAL2", "20.0"),
        ("CRPIX1", "4.0"),
        ("CRPIX2", "4.0"),
        ("CD1_1", "-1.0E-5"),
        ("CD2_2", "1.0E-5"),
    ];
    write_test_mef(
        &src,
        &[],
        &[TestHdu {
            extname: Some("SCI"),
            extver: Some(1),
            cols: 8,
            rows: 8,
            data: HduData::F32(ramp_f32(8, 8)),
            extra_cards: cards.iter().map(|(k, v)| (*k, (*v).to_string())).collect(),
        }],
    );
    format!("{}#hdu=1", src.to_str().unwrap())
}

fn reopen(path: &str) -> (HduHeader, Array2<f32>) {
    let result = extract_image_mmap(&File::open(path).unwrap()).unwrap();
    (result.header, result.image)
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string())
}

fn history_mentions(header: &HduHeader, needle: &str) -> bool {
    header.cards.iter().any(|(k, v)| k.trim() == "HISTORY" && v.contains(needle))
}

#[tokio::test]
async fn export_fits_with_stf_marks_the_output_display_referred_and_drops_the_calibration_cards() {
    let dir = tempfile::tempdir().unwrap();
    let key = calibrated_source(&dir);
    let out = tmp_path(&dir, "stf_export.fits");
    let value = export_fits(key, out.clone(), Some(true), None, None, None, Some(true), Some(true), None, None, None)
        .await
        .unwrap();
    assert_eq!(value[RES_DISPLAY_REFERRED], json!(true), "{value}");

    let (header, image) = reopen(&out);
    assert_eq!(card_text(&header, "ABDISP").as_deref(), Some("T"));
    assert_eq!(card_text(&header, "ABPROC").as_deref(), Some("stf_export"));
    for dropped in ["BUNIT", "PHOTMJSR", "PIXAR_SR"] {
        assert!(header.get(dropped).is_none(), "{dropped} survived the STF export");
    }
    for kept in ["CRVAL1", "CD1_1", "CTYPE1"] {
        assert!(header.get(kept).is_some(), "{kept} was dropped from the STF export");
    }
    assert!(history_mentions(&header, "display-referred"), "{:?}", header.cards);
    assert!(uncalibrated_reason(&header).is_some());
    assert!(image.iter().all(|v| (0.0..=1.0).contains(v)));
}

#[tokio::test]
async fn export_fits_without_stf_keeps_the_calibration_cards_and_stays_calibrated() {
    let dir = tempfile::tempdir().unwrap();
    let key = calibrated_source(&dir);
    let out = tmp_path(&dir, "linear_export.fits");
    let value = export_fits(key, out.clone(), None, None, None, None, None, None, None, None, None).await.unwrap();
    assert_eq!(value[RES_DISPLAY_REFERRED], json!(false), "{value}");

    let (header, _) = reopen(&out);
    assert_eq!(card_text(&header, "BUNIT").as_deref(), Some("MJy/sr"));
    assert!(header.get("ABDISP").is_none());
    assert_eq!(uncalibrated_reason(&header), None);
}

#[tokio::test]
async fn export_fits_reports_display_referred_from_the_written_header_not_the_stf_flag() {
    let dir = tempfile::tempdir().unwrap();
    let key = calibrated_source(&dir);
    let stretched = tmp_path(&dir, "stretched.fits");
    export_fits(key, stretched.clone(), Some(true), None, None, None, Some(true), Some(true), None, None, None)
        .await
        .unwrap();

    let copied = tmp_path(&dir, "stretched_copy.fits");
    let value = export_fits(
        stretched.clone(),
        copied.clone(),
        Some(false),
        None,
        None,
        None,
        Some(true),
        Some(true),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(value[RES_APPLY_STF], json!(false), "{value}");
    assert_eq!(value[RES_DISPLAY_REFERRED], json!(true), "{value}");
    let (header, _) = reopen(&copied);
    assert_eq!(card_text(&header, "ABDISP").as_deref(), Some("T"));
    assert!(uncalibrated_reason(&header).is_some());

    let bare = tmp_path(&dir, "stretched_bare.fits");
    let value = export_fits(stretched, bare.clone(), Some(false), None, None, None, Some(false), Some(false), None, None, None)
        .await
        .unwrap();
    assert_eq!(value[RES_DISPLAY_REFERRED], json!(false), "{value}");
    let (header, _) = reopen(&bare);
    assert!(header.get("ABDISP").is_none());
}

#[tokio::test]
async fn export_fits_with_stf_keeps_the_marker_through_rice_and_bitpix_16() {
    let dir = tempfile::tempdir().unwrap();
    let key = calibrated_source(&dir);
    let cases = [("stf_rice.fits", None, Some("rice".to_string())), ("stf_i16.fits", Some(16), None)];
    for (name, bitpix, compress) in cases {
        let out = tmp_path(&dir, name);
        let value = export_fits(
            key.clone(),
            out.clone(),
            Some(true),
            None,
            None,
            None,
            Some(true),
            Some(true),
            bitpix,
            compress,
            None,
        )
        .await
        .unwrap();
        assert_eq!(value[RES_DISPLAY_REFERRED], json!(true), "{name}: {value}");
        let entry = load_cached_full(&out).unwrap();
        let header = entry.header().unwrap();
        assert_eq!(card_text(header, "ABDISP").as_deref(), Some("T"), "{name}");
        assert_eq!(card_text(header, "ABPROC").as_deref(), Some("stf_export"), "{name}");
        assert!(header.get("BUNIT").is_none(), "{name}: BUNIT survived the STF export");
        assert!(uncalibrated_reason(header).is_some(), "{name}");
    }
}

#[tokio::test]
async fn export_fits_with_stf_and_no_copied_metadata_still_writes_the_marker() {
    let dir = tempfile::tempdir().unwrap();
    let key = calibrated_source(&dir);
    let out = tmp_path(&dir, "stf_bare.fits");
    export_fits(key, out.clone(), Some(true), None, None, None, Some(false), Some(false), None, None, None)
        .await
        .unwrap();

    let (header, _) = reopen(&out);
    assert_eq!(card_text(&header, "ABDISP").as_deref(), Some("T"));
    assert!(header.get("CRVAL1").is_none());
}

#[tokio::test]
async fn compress_mef_cmd_compresses_sci_drops_dq_and_keeps_err_raw() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("mef_source.fits");
    sci_err_dq_mef(&source, 8, 8, vec![0i32; 64]);
    let output = dir.path().join("mef_output.fits");

    let value = compress_mef_cmd(
        source.to_str().unwrap().to_string(),
        output.to_str().unwrap().to_string(),
        false,
        Some(16.0),
        vec!["DQ".to_string()],
        vec!["ERR".to_string()],
    )
    .await
    .unwrap();

    assert_eq!(value[RES_OUTPUT_PATH], output.to_str().unwrap());
    assert_eq!(value[RES_DROPPED], serde_json::json!(["DQ"]));
    assert_eq!(value[RES_KEPT_RAW], serde_json::json!(["ERR"]));
    assert!(value[RES_SOURCE_SIZE_BYTES].as_u64().unwrap() > 0);
    assert!(value[RES_OUTPUT_SIZE_BYTES].as_u64().unwrap() > 0);

    let bytes = std::fs::read(&output).unwrap();
    let primary = parse_header_at(&bytes, 0).unwrap();
    let mut offset = primary.next_hdu_offset;
    let mut extnames = Vec::new();
    let mut compressed = Vec::new();
    while offset + BLOCK_SIZE <= bytes.len() {
        let hdu = parse_header_at(&bytes, offset).unwrap();
        if let Some(name) = hdu.header.get("EXTNAME") {
            extnames.push(name.trim().trim_matches('\'').trim().to_string());
            compressed.push(hdu.header.get("ZCMPTYPE").is_some());
        }
        offset = hdu.next_hdu_offset;
    }

    assert!(!extnames.iter().any(|n| n == "DQ"), "{extnames:?}");
    let sci = extnames.iter().position(|n| n == "SCI").unwrap();
    assert!(compressed[sci], "SCI must be compressed");
    let err = extnames.iter().position(|n| n == "ERR").unwrap();
    assert!(!compressed[err], "ERR must be copied verbatim");
}

#[tokio::test]
async fn compress_mef_cmd_reads_a_gzip_compressed_source_and_refuses_to_overwrite_it() {
    use crate::infra::fits::gzip::test_support::gzip_copy;
    use crate::infra::fits::reader::extract_image_mmap_by_index;
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("mef_plain.fits");
    sci_err_dq_mef(&plain, 8, 8, vec![0i32; 64]);
    let gz = gzip_copy(&plain, "mef_plain.fits.gz");
    let gz_key = gz.to_str().unwrap().to_string();
    let output = dir.path().join("mef_from_gz.fits");

    let value = compress_mef_cmd(gz_key.clone(), output.to_str().unwrap().to_string(), true, None, Vec::new(), Vec::new())
        .await
        .unwrap();
    assert_eq!(value[RES_OUTPUT_PATH], output.to_str().unwrap());
    let sci = |path: &std::path::Path| extract_image_mmap_by_index(&File::open(path).unwrap(), 1).unwrap().image;
    assert_eq!(sci(&output), sci(&plain));
    let size = |path: &std::path::Path| std::fs::metadata(path).unwrap().len();
    assert_eq!(value[RES_SOURCE_SIZE_BYTES], size(&gz));
    assert_eq!(value[RES_SOURCE_DECOMPRESSED_BYTES], size(&plain));

    let before = std::fs::read(&gz).unwrap();
    let err = compress_mef_cmd(gz_key.clone(), gz_key, true, None, Vec::new(), Vec::new()).await.unwrap_err();
    assert!(err.contains("source"), "{err}");
    assert_eq!(std::fs::read(&gz).unwrap(), before);
}

#[tokio::test]
async fn compress_mef_cmd_reports_the_image_hdus_it_left_uncompressed() {
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, plane_hdu, write_raw_hdus};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("with_spectrum.fits");
    let spectrum_cards: Vec<(&'static str, String)> = vec![
        ("XTENSION", "'IMAGE   '".into()),
        ("BITPIX", "-32".into()),
        ("NAXIS", "1".into()),
        ("NAXIS1", "10".into()),
        ("PCOUNT", "0".into()),
        ("GCOUNT", "1".into()),
        ("EXTNAME", "'SPEC    '".into()),
    ];
    let spectrum: Vec<u8> = (0..10).flat_map(|i| (i as f32).to_be_bytes()).collect();
    write_raw_hdus(&source, &[(empty_primary_cards(), Vec::new()), plane_hdu("SCI", 8, 8), (spectrum_cards, spectrum)]);
    let output = dir.path().join("with_spectrum_compressed.fits");

    let value = compress_mef_cmd(
        source.to_str().unwrap().to_string(),
        output.to_str().unwrap().to_string(),
        true,
        None,
        Vec::new(),
        Vec::new(),
    )
    .await
    .unwrap();
    assert_eq!(value[RES_UNCOMPRESSED], serde_json::json!(["SPEC"]));
}

#[tokio::test]
async fn fits_exports_refuse_to_overwrite_a_file_they_read() {
    let dir = tempfile::tempdir().unwrap();
    let [r, g, b, header_src] = ["guard_r.fits", "guard_g.fits", "guard_b.fits", "guard_wcs.fits"].map(|n| tmp_path(&dir, n));
    for (i, path) in [&r, &g, &b, &header_src].into_iter().enumerate() {
        crate::infra::fits::writer::write_fits_mono(path, &ramp(i as f32 * 10.0), None).unwrap();
    }
    let before = std::fs::read(&r).unwrap();

    for target in [r.clone(), r.replace('\\', "/")] {
        let err = export_fits(r.clone(), target, None, None, None, None, None, None, None, None, None).await.unwrap_err();
        assert!(err.contains("source"), "{err}");
    }
    let same = || [Some(r.clone()), Some(r.clone()), Some(r.clone())];
    let [sr, sg, sb] = same();
    let err = export_fits_rgb(sr, sg, sb, r.clone(), None, None, None, None, None, None, None).await.unwrap_err();
    assert!(err.contains("source"), "{err}");
    let err = export_fits_rgb(Some(r.clone()), Some(g.clone()), Some(b.clone()), g.clone(), None, None, None, None, None, None, None)
        .await
        .unwrap_err();
    assert!(err.contains("source"), "{err}");
    let err = export_fits_rgb(
        Some(r.clone()), Some(g.clone()), Some(b.clone()), header_src.clone(),
        None, None, None, None, None, None, Some(header_src.clone()),
    )
    .await
    .unwrap_err();
    assert!(err.contains("source"), "{err}");
    assert!(std::fs::read(&r).unwrap() == before, "the source file was modified");

    let fresh = tmp_path(&dir, "guard_out.fits");
    export_fits_rgb(Some(r), Some(g), Some(b), fresh, None, None, None, None, None, None, None).await.unwrap();
}

#[tokio::test]
async fn a_channel_resampled_onto_a_larger_grid_exports_a_header_for_that_grid() {
    let dir = tempfile::tempdir().unwrap();
    let [r, g, b] = ["grid_r.fits", "grid_g.fits", "grid_b.fits"].map(|n| tmp_path(&dir, n));
    let native = header_with(&[
        ("CTYPE1", "'RA---TAN'"),
        ("CTYPE2", "'DEC--TAN'"),
        ("CRPIX1", "4.5"),
        ("CRPIX2", "3.5"),
        ("CDELT1", "-1.0E-4"),
        ("CDELT2", "1.0E-4"),
        ("PIXAR_SR", "4.0E-14"),
    ]);
    crate::infra::fits::writer::write_fits_mono(&r, &ramp(1.0), Some(&native)).unwrap();
    let large = Array2::from_shape_fn((12, 16), |(y, x)| (y * 16 + x) as f32);
    crate::infra::fits::writer::write_fits_mono(&g, &large, None).unwrap();
    crate::infra::fits::writer::write_fits_mono(&b, &large, None).unwrap();

    let out = tmp_path(&dir, "grid_rgb.fits");
    export_fits_rgb(Some(r), Some(g), Some(b), out.clone(), None, None, None, None, None, None, None).await.unwrap();
    let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
    assert_eq!(written.r.dim(), (12, 16));
    let close = |key: &str, expected: f64| {
        let actual = written.header.get_f64(key);
        assert!(actual.is_some_and(|v| (v - expected).abs() <= expected.abs() * 1e-9), "{key}: {actual:?}, expected {expected}");
    };
    close("CDELT1", -5.0e-5);
    close("CDELT2", 5.0e-5);
    close("CRPIX1", 8.5);
    close("CRPIX2", 6.5);
    close("PIXAR_SR", 1.0e-14);
}

#[test]
fn fits_rgb_composite_only_when_at_most_one_real_path() {
    let a = Some("C:/data/r.fits");
    let b = Some("C:/data/g.fits");
    let c = Some("C:/data/b.fits");
    assert!(fits_rgb_uses_composite([None, None, None]));
    assert!(fits_rgb_uses_composite([a, None, None]));
    assert!(fits_rgb_uses_composite([Some("__wizard_ch_r_aligned"), None, None]));
    assert!(fits_rgb_uses_composite([Some("__composite_r"), Some("__composite_g"), Some("__composite_b")]));
    assert!(!fits_rgb_uses_composite([Some("__wizard_ch_r_aligned"), Some("__wizard_ch_g_aligned"), Some("__wizard_ch_b_aligned")]));
    assert!(!fits_rgb_uses_composite([a, Some("__wizard_ch_g_aligned"), None]));
    assert!(!fits_rgb_uses_composite([a, b, None]));
    assert!(!fits_rgb_uses_composite([a, b, c]));
}

#[test]
fn png_single_wizard_channel_is_not_composite() {
    assert!(!png_rgb_uses_composite([Some("__wizard_ch_r_aligned"), None, None]));
    assert!(!png_rgb_uses_composite([None, Some("__wizard_ch_g_cropped"), None]));
    assert!(png_rgb_uses_composite([Some("__composite_r"), Some("__composite_g"), None]));
}

#[test]
fn png_rgb_composite_only_when_no_real_path() {
    let a = Some("C:/data/r.fits");
    let b = Some("C:/data/g.fits");
    let c = Some("C:/data/b.fits");
    assert!(png_rgb_uses_composite([None, None, None]));
    assert!(png_rgb_uses_composite([Some("__composite_r"), None, None]));
    assert!(!png_rgb_uses_composite([a, None, None]));
    assert!(!png_rgb_uses_composite([a, b, None]));
    assert!(!png_rgb_uses_composite([a, b, c]));
}

fn tmp_path(dir: &tempfile::TempDir, name: &str) -> String {
    dir.path().join(name).to_str().unwrap().to_string()
}

fn ramp(base: f32) -> Array2<f32> {
    Array2::from_shape_fn((6, 8), |(y, x)| base + (y * 8 + x) as f32)
}

fn header_with(cards: &[(&str, &str)]) -> HduHeader {
    let mut header = HduHeader::empty();
    for (k, v) in cards {
        header.set(k, v.to_string());
    }
    header
}

fn uniform_stf(shadow: f64, midtone: f64, highlight: f64) -> [[Option<f64>; 3]; 3] {
    [[Some(shadow), Some(midtone), Some(highlight)]; 3]
}

fn apply_each(
    planes: [&Array2<f32>; 3],
    params: [StfParams; 3],
    stats: [&ImageStats; 3],
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    (
        apply_stf_f32(planes[0], &params[0], stats[0]),
        apply_stf_f32(planes[1], &params[1], stats[1]),
        apply_stf_f32(planes[2], &params[2], stats[2]),
    )
}

struct Channels {
    planes: [Array2<f32>; 3],
    stats: [ImageStats; 3],
    combined: ImageStats,
}

fn unbalanced_channels() -> Channels {
    let planes = [ramp(10.0), ramp(10.0).mapv(|v| v * 3.0), ramp(10.0).mapv(|v| v * 9.0 + 400.0)];
    let stats = [
        compute_image_stats(&planes[0]),
        compute_image_stats(&planes[1]),
        compute_image_stats(&planes[2]),
    ];
    let combined = combine_channel_stats(&stats[0], &stats[1], &stats[2]);
    Channels { planes, stats, combined }
}

impl Channels {
    fn planes(&self) -> [&Array2<f32>; 3] {
        [&self.planes[0], &self.planes[1], &self.planes[2]]
    }

    fn stats(&self) -> [&ImageStats; 3] {
        [&self.stats[0], &self.stats[1], &self.stats[2]]
    }

    fn linked(&self) -> [&ImageStats; 3] {
        [&self.combined; 3]
    }
}

fn stf(shadow: f64, midtone: f64, highlight: f64) -> StfParams {
    StfParams { shadow, midtone, highlight }
}

#[test]
fn an_explicit_shadow_and_highlight_are_honoured_with_a_neutral_midtone() {
    let ch = unbalanced_channels();
    let request = PngStf { apply: true, channels: uniform_stf(0.2, 0.5, 0.9), linked: None };
    let expected = apply_each(ch.planes(), [stf(0.2, 0.5, 0.9); 3], ch.linked());
    assert_eq!(png_rgb_planes(ch.planes(), ch.stats(), &request), expected, "the user's clip was replaced by auto STF");

    let ignored = PngStf { apply: false, ..request };
    let auto = auto_stf(&ch.combined, &AutoStfConfig::default());
    assert_eq!(png_rgb_planes(ch.planes(), ch.stats(), &ignored), apply_each(ch.planes(), [auto; 3], ch.linked()));
}

#[test]
fn the_linked_flag_decides_the_normalisation_instead_of_parameter_equality() {
    let ch = unbalanced_channels();
    let equal = uniform_stf(0.1, 0.3, 0.95);
    let unlinked = PngStf { apply: true, channels: equal, linked: Some(false) };
    assert_eq!(
        png_rgb_planes(ch.planes(), ch.stats(), &unlinked),
        apply_each(ch.planes(), [stf(0.1, 0.3, 0.95); 3], ch.stats()),
        "unlinked STF with equal values was normalised with the combined stats"
    );

    let mut different = equal;
    different[2] = [Some(0.0), Some(0.6), Some(1.0)];
    let linked = PngStf { apply: true, channels: different, linked: Some(true) };
    let params = [stf(0.1, 0.3, 0.95), stf(0.1, 0.3, 0.95), stf(0.0, 0.6, 1.0)];
    assert_eq!(png_rgb_planes(ch.planes(), ch.stats(), &linked), apply_each(ch.planes(), params, ch.linked()));

    let inferred = PngStf { linked: None, ..linked };
    assert_eq!(png_rgb_planes(ch.planes(), ch.stats(), &inferred), apply_each(ch.planes(), params, ch.stats()));
    let inferred_equal = PngStf { linked: None, ..unlinked };
    assert_eq!(
        png_rgb_planes(ch.planes(), ch.stats(), &inferred_equal),
        apply_each(ch.planes(), [stf(0.1, 0.3, 0.95); 3], ch.linked())
    );

    let auto_unlinked = PngStf { apply: false, channels: [[None; 3]; 3], linked: Some(false) };
    let config = AutoStfConfig::default();
    let per_channel = [auto_stf(&ch.stats[0], &config), auto_stf(&ch.stats[1], &config), auto_stf(&ch.stats[2], &config)];
    assert_eq!(
        png_rgb_planes(ch.planes(), ch.stats(), &auto_unlinked),
        apply_each(ch.planes(), per_channel, ch.stats())
    );
}

#[tokio::test]
async fn export_rgb_png_rejects_stf_values_outside_the_unit_range() {
    let dir = tempfile::tempdir().unwrap();
    let out = tmp_path(&dir, "bad_stf.png");
    for bad in [2.0, -0.1, f64::NAN] {
        let err = export_rgb_png(
            None, None, None, out.clone(), Some(8), Some(true),
            None, Some(bad), None, None, None, None, None, None, None, None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("midtone_r"), "{err}");
    }
}

#[tokio::test]
async fn the_linear_composite_png_honours_a_neutral_midtone_stf_and_the_linked_flag() {
    let _guard = helpers::composite_test_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let out = tmp_path(&dir, "composite_linear.png");
    let expected_path = tmp_path(&dir, "expected.png");
    let ch = unbalanced_channels();
    helpers::clear_composite();
    helpers::insert_composite_and_orig(
        ch.planes[0].clone(), ch.planes[1].clone(), ch.planes[2].clone(),
        ch.stats[0].clone(), ch.stats[1].clone(), ch.stats[2].clone(),
    );

    let (s, m, h) = (Some(0.1), Some(0.5), Some(0.9));
    let value = export_rgb_png(None, None, None, out.clone(), Some(8), Some(true), s, m, h, s, m, h, s, m, h, Some(false)).await;
    helpers::clear_composite();
    assert_eq!(value.unwrap()[RES_APPLY_STF], json!(true));

    let (er, eg, eb) = apply_each(ch.planes(), [stf(0.1, 0.5, 0.9); 3], ch.stats());
    render_rgb_png((&er, &eg, &eb), 8, &expected_path).unwrap();
    assert!(
        image::open(&out).unwrap().to_rgb8() == image::open(&expected_path).unwrap().to_rgb8(),
        "the composite export replaced the explicit STF or ignored linked=false"
    );
}

#[tokio::test]
async fn export_fits_writes_the_pixels_it_read_from_disk_not_a_stale_cache_entry() {
    let dir = tempfile::tempdir().unwrap();
    let src = tmp_path(&dir, "rerun_hdr.fits");
    let out = tmp_path(&dir, "exported.fits");
    crate::infra::fits::writer::write_fits_mono(&src, &Array2::from_elem((4, 4), 1.0), None).unwrap();
    let first_stamp = std::fs::metadata(&src).unwrap().modified().unwrap();
    assert_eq!(load_cached(&src).unwrap().arr()[[0, 0]], 1.0);

    crate::infra::fits::writer::write_fits_mono(&src, &Array2::from_elem((4, 4), 2.0), None).unwrap();
    std::fs::OpenOptions::new().write(true).open(&src).unwrap().set_modified(first_stamp).unwrap();

    export_fits(src.clone(), out.clone(), None, None, None, None, None, None, None, None, None).await.unwrap();
    assert_eq!(extract_image_resolved(&out).unwrap().arr[[0, 0]], 2.0, "the previous run's pixels were exported");
}

#[tokio::test]
async fn an_rgb_fits_given_for_all_three_channels_exports_its_own_planes() {
    let dir = tempfile::tempdir().unwrap();
    let src = tmp_path(&dir, "osc_rgb.fits");
    let (r, g, b) = (ramp(1.0), ramp(100.0), ramp(1000.0));
    let header = header_with(&[("OBJECT", "'NGC7000'")]);
    crate::infra::fits::writer::write_fits_rgb(&src, &r, &g, &b, Some(&header)).unwrap();

    let fits_out = tmp_path(&dir, "osc_cube.fits");
    export_fits_rgb(
        Some(src.clone()), Some(src.clone()), Some(src.clone()), fits_out.clone(),
        None, None, None, None, None, None, None,
    )
    .await
    .unwrap();
    let written = try_extract_rgb_resolved(&fits_out).unwrap().expect("a 3-plane RGB FITS");
    assert_eq!((&written.r, &written.g, &written.b), (&r, &g, &b));
    assert_eq!(written.header.get("OBJECT").map(|v| v.trim().trim_matches('\'').trim()), Some("NGC7000"));

    let png_out = tmp_path(&dir, "osc_rgb.png");
    export_rgb_png(
        Some(src.clone()), Some(src.clone()), Some(src), png_out.clone(), Some(8), None,
        None, None, None, None, None, None, None, None, None, None,
    )
    .await
    .unwrap();
    let png = image::open(&png_out).unwrap().to_rgb8();
    assert_eq!((png.width(), png.height()), (8, 6));
    assert!(png.pixels().any(|p| p[0] != p[2]), "the RGB file was exported as a grey plane");
}

#[tokio::test]
async fn wizard_channels_held_only_in_memory_export_as_fits_with_the_requested_header() {
    let _wizard = crate::infra::cache::lock_wizard_entries();
    let dir = tempfile::tempdir().unwrap();
    let keys = ["r", "g", "b"].map(|c| crate::types::constants::wizard_bg_key(&format!("export_test_{c}")));
    let planes = [ramp(3.0), ramp(30.0), ramp(300.0)];
    for (key, plane) in keys.iter().zip(&planes) {
        GLOBAL_IMAGE_CACHE.insert_synthetic(key, Arc::new(plane.clone()), compute_image_stats(plane));
    }
    let source = tmp_path(&dir, "bin_first_file.fits");
    let solved = header_with(&[("CTYPE1", "'RA---TAN'"), ("CRVAL1", "83.8"), ("CRPIX1", "4.0")]);
    crate::infra::fits::writer::write_fits_mono(&source, &ramp(0.0), Some(&solved)).unwrap();

    let out = tmp_path(&dir, "wizard_rgb.fits");
    let [kr, kg, kb] = keys.clone().map(Some);
    let result = export_fits_rgb(kr, kg, kb, out.clone(), None, None, None, None, None, None, Some(source)).await;
    let bare_out = tmp_path(&dir, "wizard_rgb_bare.fits");
    let [kr, kg, kb] = keys.clone().map(Some);
    let bare = export_fits_rgb(kr, kg, kb, bare_out, None, None, None, None, None, None, None).await;
    for key in &keys {
        GLOBAL_IMAGE_CACHE.remove(key);
    }
    result.unwrap();
    assert_eq!(bare.unwrap()[RES_WCS_WRITTEN], json!(false), "an export without any header claimed a WCS");

    let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
    assert_eq!([&written.r, &written.g, &written.b], [&planes[0], &planes[1], &planes[2]]);
    assert_eq!(written.header.get("CRVAL1").map(str::trim), Some("83.8"), "the WCS of the bin's source was dropped");

    let [kr, kg, kb] = keys.map(Some);
    let gone = export_fits_rgb(kr, kg, kb, tmp_path(&dir, "gone.fits"), None, None, None, None, None, None, None)
        .await
        .unwrap_err();
    assert!(gone.contains("no longer in memory"), "{gone}");
}

#[tokio::test]
async fn a_background_corrected_wizard_channel_keeps_its_wcs_in_the_composite_export() {
    let _composite = helpers::composite_test_lock().await;
    let _wizard = crate::infra::cache::lock_wizard_entries();
    let dir = tempfile::tempdir().unwrap();
    let key = crate::types::constants::wizard_bg_key("export_wcs_r");
    let solved = header_with(&[
        ("CTYPE1", "RA---TAN"),
        ("CTYPE2", "DEC--TAN"),
        ("CRVAL1", "83.8"),
        ("CRVAL2", "-5.4"),
        ("CRPIX1", "4.5"),
        ("CRPIX2", "3.5"),
        ("CDELT1", "-1.0E-4"),
        ("CDELT2", "1.0E-4"),
    ]);
    let channel = ramp(1.0);
    GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&key, Arc::new(channel.clone()), compute_image_stats(&channel), Some(solved));
    let composite = Array2::from_shape_fn((12, 16), |(y, x)| (y * 16 + x) as f32);
    let stats = compute_image_stats(&composite);
    helpers::clear_composite();
    helpers::insert_composite_and_orig(
        composite.clone(), composite.clone(), composite.clone(),
        stats.clone(), stats.clone(), stats,
    );

    let out = tmp_path(&dir, "composite_bg.fits");
    let result = export_fits_rgb(Some(key.clone()), None, None, out.clone(), None, None, None, None, None, None, None).await;
    helpers::clear_composite();
    GLOBAL_IMAGE_CACHE.remove(&key);
    assert_eq!(result.unwrap()[RES_WCS_WRITTEN], json!(true), "the export did not report the WCS it wrote");

    let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
    assert_eq!(written.r.dim(), (12, 16));
    assert_eq!(written.header.get("CRVAL1").map(str::trim), Some("83.8"), "the WCS of the background-corrected channel was dropped");
    let close = |key: &str, expected: f64| {
        let actual = written.header.get_f64(key);
        assert!(actual.is_some_and(|v| (v - expected).abs() <= expected.abs() * 1e-9), "{key}: {actual:?}, expected {expected}");
    };
    close("CRPIX1", 8.5);
    close("CRPIX2", 6.5);
    close("CDELT1", -5.0e-5);
    close("CDELT2", 5.0e-5);
}

#[tokio::test]
async fn an_rgb_export_without_wcs_cards_from_gwcs_backed_channels_reports_no_wcs() {
    use crate::core::astrometry::gwcs::test_support::fixtures_dir;
    use crate::infra::fits::asdf_hdu::test_fixtures::write_fits_with_asdf_cell;
    use crate::infra::wcs_source::test_fixtures::tan_cards;

    let dir = tempfile::tempdir().unwrap();
    let cell = std::fs::read(fixtures_dir().join("wcs_jwst_nircam_cal300.asdf")).unwrap();
    let [r, g, b] = ["r", "g", "b"].map(|c| {
        let path = dir.path().join(format!("nircam_{c}_cal.fits"));
        write_fits_with_asdf_cell(&path, &tan_cards(), &cell);
        path.to_str().unwrap().to_string()
    });
    assert!(crate::infra::wcs_source::gwcs_for_path(&r).unwrap().is_some(), "the channel is gWCS-backed");

    let out = tmp_path(&dir, "rgb_without_wcs.fits");
    let result = export_fits_rgb(
        Some(r.clone()), Some(g.clone()), Some(b.clone()), out.clone(), Some(false), None, None, None, None, None, None,
    )
    .await
    .unwrap();
    assert_eq!(result[RES_WCS_WRITTEN], json!(false), "no WCS card was written: {result}");
    let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
    assert_eq!(written.header.get("CTYPE1"), None);
    assert_eq!(written.header.get("CRVAL1"), None);

    let with_wcs = export_fits_rgb(
        Some(r), Some(g), Some(b), tmp_path(&dir, "rgb_with_wcs.fits"), Some(true), None, None, None, None, None, None,
    )
    .await
    .unwrap();
    assert_eq!(with_wcs[RES_WCS_WRITTEN], json!(true), "{with_wcs}");
}

#[tokio::test]
async fn an_rgb_export_of_tan_pv_channels_reports_the_wcs_it_wrote() {
    use crate::infra::wcs_source::test_fixtures::{tan_cards, PV_DISTORTION_CARDS};

    let dir = tempfile::tempdir().unwrap();
    let mut header = header_with(&PV_DISTORTION_CARDS);
    for (key, value) in tan_cards() {
        header.set(key, value);
    }
    let [r, g, b] = ["r", "g", "b"].map(|c| {
        let path = tmp_path(&dir, &format!("tan_pv_{c}.fits"));
        crate::infra::fits::writer::write_fits_mono(&path, &ramp(1.0), Some(&header)).unwrap();
        path
    });
    let out = tmp_path(&dir, "rgb_tan_pv.fits");

    let result = export_fits_rgb(Some(r), Some(g), Some(b), out.clone(), Some(true), None, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(result[RES_WCS_WRITTEN], json!(true), "the TAN+PV cards were written but reported missing: {result}");
    let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
    assert_eq!(written.header.get_f64("PV1_1"), Some(1.02), "the PV cards travel with the exported WCS");
    crate::infra::wcs_source::load_wcs(&out, &written.header).expect("the exported header loads as a celestial WCS");
}

#[tokio::test]
async fn png_export_of_display_referred_channels_skips_the_auto_stf() {
    let dir = tempfile::tempdir().unwrap();
    let data = Array2::from_shape_fn((16, 16), |(y, x)| {
        if (y * 16 + x) % 37 == 5 { 0.9 } else { 0.1 + ((y * 31 + x * 17) % 13) as f32 * 0.001 }
    });
    let flagged = tmp_path(&dir, "wizard_ha_stretched.fits");
    let linear = tmp_path(&dir, "wizard_ha_linear.fits");
    let display_referred = header_with(&[(crate::cmd::common::HEADER_DISPLAY_REFERRED, "T")]);
    crate::infra::fits::writer::write_fits_mono(&flagged, &data, Some(&display_referred)).unwrap();
    crate::infra::fits::writer::write_fits_mono(&linear, &data, None).unwrap();

    let export = |path: &str, name: &str| {
        let out = tmp_path(&dir, name);
        let p = Some(path.to_string());
        async move {
            let res = export_rgb_png(
                p.clone(), p.clone(), p, out.clone(), Some(8), None,
                None, None, None, None, None, None, None, None, None, Some(true),
            )
            .await
            .unwrap();
            let background = image::open(&out).unwrap().to_rgb8().get_pixel(0, 0).0;
            (res, background)
        }
    };
    let (res, background) = export(&flagged, "flagged.png").await;
    assert_eq!(res[RES_APPLY_STF], json!(false), "{res}");
    for v in background {
        assert!((v as f32 - 0.1 * 255.0).abs() <= 1.0, "the display-referred background 0.1 was re-stretched to {v}");
    }
    let (res, background) = export(&linear, "linear.png").await;
    assert_eq!(res[RES_APPLY_STF], json!(true), "{res}");
    assert!((background[0] as f32 - 0.1 * 255.0).abs() > 5.0, "a linear channel must still get the auto STF");

    let stretched = Some(&display_referred);
    let auto = PngStf { apply: false, channels: [[None; 3]; 3], linked: Some(true) };
    let identity = png_stf_for([stretched, stretched], &auto).expect("identity STF");
    assert_eq!((identity.shadow, identity.midtone, identity.highlight), (0.0, 0.5, 1.0));
    assert!(png_stf_for([stretched, None], &auto).is_none(), "a linear channel needs the auto STF");
    assert!(png_stf_for([], &auto).is_none());
    let explicit = PngStf { apply: true, channels: uniform_stf(0.1, 0.4, 0.9), linked: Some(true) };
    assert!(png_stf_for([stretched; 3], &explicit).is_none(), "an explicit STF wins");
}

#[tokio::test]
async fn png_export_of_a_display_referred_rgb_fits_skips_the_auto_stf() {
    let dir = tempfile::tempdir().unwrap();
    let plane = |level: f32| {
        Array2::from_shape_fn((16, 16), |(y, x)| {
            if (y * 16 + x) % 37 == 5 { 0.9 } else { level + ((y * 31 + x * 17) % 13) as f32 * 0.001 }
        })
    };
    let levels = [0.1f32, 0.2, 0.3];
    let [r, g, b] = levels.map(plane);
    let flagged = tmp_path(&dir, "rgb_stretched.fits");
    let linear = tmp_path(&dir, "rgb_linear.fits");
    let display_referred = header_with(&[(crate::cmd::common::HEADER_DISPLAY_REFERRED, "T")]);
    crate::infra::fits::writer::write_fits_rgb(&flagged, &r, &g, &b, Some(&display_referred)).unwrap();
    crate::infra::fits::writer::write_fits_rgb(&linear, &r, &g, &b, None).unwrap();

    let export = |path: &str, name: &str| {
        let out = tmp_path(&dir, name);
        let p = Some(path.to_string());
        async move {
            let res = export_rgb_png(
                p.clone(), p.clone(), p, out.clone(), Some(8), None,
                None, None, None, None, None, None, None, None, None, Some(true),
            )
            .await
            .unwrap();
            let background = image::open(&out).unwrap().to_rgb8().get_pixel(0, 0).0;
            (res, background)
        }
    };
    let (res, background) = export(&flagged, "flagged_rgb.png").await;
    assert_eq!(res[RES_APPLY_STF], json!(false), "{res}");
    for (v, level) in background.into_iter().zip(levels) {
        assert!((v as f32 - level * 255.0).abs() <= 1.0, "the display-referred background {level} was re-stretched to {v}");
    }
    let (res, background) = export(&linear, "linear_rgb.png").await;
    assert_eq!(res[RES_APPLY_STF], json!(true), "{res}");
    assert!((background[0] as f32 - 0.1 * 255.0).abs() > 5.0, "a linear RGB file must still get the auto STF");
}

#[tokio::test]
async fn export_as_png_of_a_display_referred_rgb_fits_writes_it_as_shown() {
    let dir = tempfile::tempdir().unwrap();
    let plane = |level: f32| {
        Array2::from_shape_fn((16, 16), |(y, x)| {
            if (y * 16 + x) % 37 == 5 { 0.9 } else { level + ((y * 31 + x * 17) % 13) as f32 * 0.001 }
        })
    };
    let levels = [0.1f32, 0.2, 0.3];
    let [r, g, b] = levels.map(plane);
    let flagged = tmp_path(&dir, "curves_rgb.fits");
    let linear = tmp_path(&dir, "linear_rgb.fits");
    let display_referred = header_with(&[(crate::cmd::common::HEADER_DISPLAY_REFERRED, "T")]);
    crate::infra::fits::writer::write_fits_rgb(&flagged, &r, &g, &b, Some(&display_referred)).unwrap();
    crate::infra::fits::writer::write_fits_rgb(&linear, &r, &g, &b, None).unwrap();

    let export = |path: &str, name: &str| {
        let out = tmp_path(&dir, name);
        let path = path.to_string();
        async move {
            let res = export_png(path, out.clone(), Some(8), None, None, None, None).await.unwrap();
            let background = image::open(&out).unwrap().to_rgb8().get_pixel(0, 0).0;
            (res, background)
        }
    };
    let (res, background) = export(&flagged, "curves_rgb.png").await;
    assert_eq!(res[RES_APPLY_STF], json!(false), "{res}");
    for (v, level) in background.into_iter().zip(levels) {
        assert!((v as f32 - level * 255.0).abs() <= 1.0, "the display-referred background {level} was re-stretched to {v}");
    }
    let (res, background) = export(&linear, "linear_rgb.png").await;
    assert_eq!(res[RES_APPLY_STF], json!(true), "{res}");
    assert!((background[0] as f32 - 0.1 * 255.0).abs() > 5.0, "a linear RGB file must still get the linked auto STF");
}

fn png_text_chunks(path: &str) -> std::collections::BTreeMap<String, String> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let mut found = std::collections::BTreeMap::new();
    let mut at = 8;
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let kind = &bytes[at + 4..at + 8];
        let data = &bytes[at + 8..at + 8 + len];
        let mut crc = flate2::Crc::new();
        crc.update(&bytes[at + 4..at + 8 + len]);
        assert_eq!(crc.sum().to_be_bytes(), bytes[at + 8 + len..at + 12 + len], "bad CRC on a {:?} chunk", String::from_utf8_lossy(kind));
        let fields: Vec<&[u8]> = data.splitn(2, |&b| b == 0).collect();
        match kind {
            b"tEXt" => {
                found.insert(String::from_utf8_lossy(fields[0]).into_owned(), String::from_utf8_lossy(fields[1]).into_owned());
            }
            b"iTXt" => {
                let rest = fields[1];
                assert_eq!(&rest[..2], [0, 0], "a compressed iTXt chunk");
                let parts: Vec<&[u8]> = rest[2..].splitn(3, |&b| b == 0).collect();
                found.insert(String::from_utf8_lossy(fields[0]).into_owned(), String::from_utf8(parts[2].to_vec()).unwrap());
            }
            _ => {}
        }
        at += 12 + len;
    }
    found
}

fn utc_mtime(path: &str) -> String {
    crate::core::history::stamp::utc_timestamp(std::fs::metadata(path).unwrap().modified().unwrap())
}

#[tokio::test]
async fn export_fits_records_the_input_identity_the_parameters_and_the_version() {
    let dir = tempfile::tempdir().unwrap();
    let key = calibrated_source(&dir);
    let file = key.trim_end_matches("#hdu=1").to_string();
    let out = tmp_path(&dir, "identified.fits");
    export_fits(key.clone(), out.clone(), None, None, None, None, None, None, Some(-64), None, None).await.unwrap();

    let (header, _) = reopen(&out);
    assert_eq!(card_text(&header, "ABOP").as_deref(), Some("export_fits"), "{:?}", header.cards);
    assert_eq!(card_text(&header, "ABSRC").as_deref(), Some(key.as_str()));
    assert_eq!(header.get_i64("ABSRCHDU"), Some(1));
    assert_eq!(header.get_i64("ABSRCSZ"), Some(std::fs::metadata(&file).unwrap().len() as i64));
    assert_eq!(card_text(&header, "ABSRCMT"), Some(utc_mtime(&file)));
    assert_eq!(card_text(&header, "ABVER").as_deref(), Some(env!("CARGO_PKG_VERSION")));
    let params: serde_json::Value = serde_json::from_str(&card_text(&header, "ABPARAMS").unwrap()).unwrap();
    assert_eq!(params["bitpix"], json!(-64));
    assert_eq!(params["compress"], json!("none"));
    assert_eq!(params["apply_stf"], json!(false));
    assert_eq!(params["copy_wcs"], json!(true));
}

#[tokio::test]
async fn export_fits_records_the_stf_it_applied_at_full_precision() {
    let dir = tempfile::tempdir().unwrap();
    let key = calibrated_source(&dir);
    let midtone = 0.75 * 0.00012 / (0.25 + 0.5 * 0.00012);
    let out = tmp_path(&dir, "faint_stf.fits");
    export_fits(key.clone(), out.clone(), Some(true), Some(0.0), Some(midtone), Some(1.0), None, None, None, None, None).await.unwrap();
    let (header, _) = reopen(&out);
    let applied = header.get_f64("ABSTFMT").expect("no ABSTFMT card");
    assert!((applied - midtone).abs() <= 1e-12 * midtone, "ABSTFMT {applied} recorded for midtone {midtone}");
    assert_eq!(header.get_f64("ABSTFSH"), Some(0.0));
    assert_eq!(header.get_f64("ABSTFHI"), Some(1.0));
    let ramp: Vec<f32> = ramp_f32(8, 8).into_iter().filter(|v| v.is_finite() && *v != 0.0).collect();
    let lo = ramp.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = ramp.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(header.get_f64("ABSTFMIN"), Some(f64::from(lo)));
    assert_eq!(header.get_f64("ABSTFMAX"), Some(f64::from(hi)));

    let clamped = tmp_path(&dir, "clamped_stf.fits");
    export_fits(key.clone(), clamped.clone(), Some(true), None, Some(0.00005), None, None, None, None, None, None).await.unwrap();
    let (header, _) = reopen(&clamped);
    assert_eq!(header.get_f64("ABSTFMT"), Some(0.0001), "the applied midtone is the STF floor, not the request");
    assert_eq!(header.get_f64("ABSTFMRQ"), Some(0.00005));

    let err = export_fits(key, tmp_path(&dir, "bad_stf.fits"), Some(true), None, Some(1.5), None, None, None, None, None, None).await.unwrap_err();
    assert!(err.contains("midtone"), "{err}");
}

#[tokio::test]
async fn export_fits_with_stf_keeps_non_finite_pixels_blank() {
    let dir = tempfile::tempdir().unwrap();
    let src = tmp_path(&dir, "with_nan.fits");
    let mut data = ramp(10.0);
    data[[2, 3]] = f32::NAN;
    crate::infra::fits::writer::write_fits_mono(&src, &data, None).unwrap();
    let out = tmp_path(&dir, "with_nan_stf.fits");
    export_fits(src, out.clone(), Some(true), None, None, None, None, None, None, None, None).await.unwrap();
    let (_, image) = reopen(&out);
    assert!(image[[2, 3]].is_nan(), "a NaN pixel was exported as {}", image[[2, 3]]);
    assert!(image[[2, 4]].is_finite());
}

#[tokio::test]
async fn fits_exports_refuse_a_bitpix_they_cannot_write() {
    let dir = tempfile::tempdir().unwrap();
    let src = tmp_path(&dir, "plain.fits");
    crate::infra::fits::writer::write_fits_mono(&src, &ramp(1.0), None).unwrap();
    for bitpix in [8, 32, 64, -16] {
        let out = tmp_path(&dir, &format!("bitpix_{bitpix}.fits"));
        let err = export_fits(src.clone(), out.clone(), None, None, None, None, None, None, Some(bitpix), None, None).await.unwrap_err();
        assert!(err.contains("BITPIX"), "{bitpix}: {err}");
        assert!(!std::path::Path::new(&out).exists(), "{bitpix}: a file was written");
        let rgb_out = tmp_path(&dir, &format!("rgb_bitpix_{bitpix}.fits"));
        let err = export_fits_rgb(Some(src.clone()), Some(src.clone()), Some(src.clone()), rgb_out, None, None, Some(bitpix), None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("BITPIX"), "{bitpix}: {err}");
    }
}

#[tokio::test]
async fn export_fits_rgb_records_the_source_of_every_channel() {
    let dir = tempfile::tempdir().unwrap();
    let [r, g, b] = ["r", "g", "b"].map(|c| {
        let path = tmp_path(&dir, &format!("chan_{c}.fits"));
        crate::infra::fits::writer::write_fits_mono(&path, &ramp(1.0), None).unwrap();
        path
    });
    let out = tmp_path(&dir, "channels.fits");
    export_fits_rgb(Some(r.clone()), Some(g.clone()), Some(b.clone()), out.clone(), None, None, None, Some(vec!["a".repeat(120)]), None, None, None)
        .await
        .unwrap();
    let written = try_extract_rgb_resolved(&out).unwrap().expect("a 3-plane RGB FITS");
    assert_eq!(card_text(&written.header, "ABOP").as_deref(), Some("export_fits_rgb"));
    let inputs: serde_json::Value = serde_json::from_str(&card_text(&written.header, "ABINPUTS").expect("no ABINPUTS card")).unwrap();
    for (index, (role, path)) in [("r", &r), ("g", &g), ("b", &b)].into_iter().enumerate() {
        assert_eq!(inputs[index]["role"], json!(role));
        assert_eq!(inputs[index]["ref"], json!(path));
        assert_eq!(inputs[index]["size_bytes"], json!(std::fs::metadata(path).unwrap().len()));
    }
    let params: serde_json::Value = serde_json::from_str(&card_text(&written.header, "ABPARAMS").unwrap()).unwrap();
    assert_eq!(params["history"], json!(["a".repeat(120)]), "the requested history was cut");
}

#[tokio::test]
async fn png_exports_carry_the_input_identity_as_text_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let key = calibrated_source(&dir);
    let file = key.trim_end_matches("#hdu=1").to_string();
    let out = tmp_path(&dir, "identified.png");
    export_png(key.clone(), out.clone(), Some(16), Some(true), Some(0.0), Some(0.25), Some(1.0)).await.unwrap();
    let text = png_text_chunks(&out);
    assert_eq!(text.get("ABOP").map(String::as_str), Some("export_png"), "{text:?}");
    assert_eq!(text.get("ABSRC"), Some(&key));
    assert_eq!(text.get("ABSRCHDU").map(String::as_str), Some("1"));
    assert_eq!(text.get("ABSRCSZ"), Some(&std::fs::metadata(&file).unwrap().len().to_string()));
    assert_eq!(text.get("ABSRCMT"), Some(&utc_mtime(&file)));
    assert_eq!(text.get("ABVER").map(String::as_str), Some(env!("CARGO_PKG_VERSION")));
    let params: serde_json::Value = serde_json::from_str(text.get("ABPARAMS").unwrap()).unwrap();
    assert_eq!(params["bit_depth"], json!(16));
    assert_eq!(params["stf"]["midtone"], json!(0.25));
    image::open(&out).expect("the PNG with text chunks still decodes");

    let rgb_out = tmp_path(&dir, "identified_rgb.png");
    let p = Some(file.clone());
    export_rgb_png(p.clone(), p.clone(), p, rgb_out.clone(), Some(8), None, None, None, None, None, None, None, None, None, None, None)
        .await
        .unwrap();
    let text = png_text_chunks(&rgb_out);
    assert_eq!(text.get("ABOP").map(String::as_str), Some("export_rgb_png"), "{text:?}");
    let inputs: serde_json::Value = serde_json::from_str(text.get("ABINPUTS").unwrap()).unwrap();
    assert_eq!(inputs.as_array().map(Vec::len), Some(3));
    assert_eq!(inputs[1]["ref"], json!(file));
    image::open(&rgb_out).expect("the RGB PNG with text chunks still decodes");
}
