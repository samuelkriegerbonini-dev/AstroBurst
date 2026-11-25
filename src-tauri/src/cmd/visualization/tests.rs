use serde_json::json;

use crate::cmd::common::{load_cached, load_cached_full, load_companions};
use crate::cmd::helpers;
use crate::core::imaging::dq_flags::{mask_preview_or, DqTable, FlagSet};
use crate::core::imaging::scale::LimitMode;
use crate::core::imaging::stf::StfParams;
use crate::infra::ipc::encode_mask_with_header;
use crate::infra::render::tiles;
use crate::types::constants::{
    RES_ALGORITHM, RES_BITS_PRESENT, RES_CENTRE, RES_COLORMAPS, RES_DEFAULT_MASK, RES_DQ, RES_DQ_NAMES, RES_DQ_REF,
    RES_DQ_TABLE, RES_ELAPSED_MS, RES_ERR, RES_ERR_STATS, RES_EXCLUSION_MASK, RES_FLAGS, RES_LABEL, RES_NAME,
    RES_NODATA, RES_NOTES, RES_PNG_PATH, RES_RGBA, RES_SIZE, RES_SOURCE, RES_STATS, RES_SYMMETRIC, RES_TABLE,
    RES_UNIT, RES_VALUES, RES_VMAX, RES_VMIN, RES_WAVELENGTH, RES_WAVELENGTH_STATS, RES_WAVELENGTH_UNIT, RES_X,
    RES_X0, RES_Y, RES_Y0,
};
use crate::types::header::HduHeader;

use super::*;
use crate::cmd::common::test_support::{afw_mef, float_dq_mef};
use crate::infra::fits::reader::test_fixtures::{
    sci_err_dq_mef, sci_err_dq_mef_with_dq_cards, sci_err_dq_wave_mef, sci_err_dq_wave_mef_with_wave_cards,
};
use crate::types::constants::RES_VALUE;

#[test]
fn the_default_overlay_is_the_one_of_the_flag_set() {
    let (jwst, hst, raw) =
        (FlagSet::from_table(DqTable::Jwst), FlagSet::from_table(DqTable::Hst), FlagSet::from_table(DqTable::Unknown));
    assert_eq!(resolve_overlay_mask(&jwst, None), 7);
    assert_eq!(resolve_overlay_mask(&hst, None), 4 | 256 | 4096 | 8192);
    assert_eq!(resolve_overlay_mask(&raw, None), 0);
    assert_eq!(resolve_overlay_mask(&raw, Some(5)), 5);
    assert_eq!(resolve_overlay_mask(&jwst, Some(0)), 0);
    assert_eq!(resolve_overlay_mask(&hst, Some(1 << 31)), 1 << 31);

    let dir = tempfile::tempdir().unwrap();
    let k = format!("{}#hdu=1", afw_mef(dir.path(), "calexp.fits", vec![]));
    let afw = load_companions(&k).unwrap().flags.expect("the MASK HDU is the flag layer of the IMAGE HDU");
    assert_eq!(resolve_overlay_mask(&afw, None), 3);
    assert_eq!(resolve_overlay_mask(&afw, Some(5)), 5);
}

#[test]
fn dq_table_for_uses_companion_then_active_header() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vis.fits");
    sci_err_dq_mef_with_dq_cards(&path, 4, 4, vec![-2147483648i32; 16], vec![("TELESCOP", "'JWST'".into())]);
    let p = path.to_str().unwrap().to_string();
    let sci_header = load_cached_full(&format!("{}#hdu=1", p)).unwrap().header().cloned().unwrap();
    assert_eq!(sci_header.get("TELESCOP"), None);
    assert_eq!(DqTable::select(&sci_header), DqTable::Unknown);

    let found = dq_table_for(&format!("{}#hdu=1", p)).unwrap();
    assert_eq!(found.set.table, DqTable::Jwst);
    assert_eq!(found.dq_ref.as_deref(), Some(format!("{}#hdu=3", p).as_str()));
    assert_eq!(found.err_ref.as_deref(), Some(format!("{}#hdu=2", p).as_str()));
    assert_eq!(found.bits_present, None);

    let lonely = dq_table_for(&format!("{}#hdu=4", p)).unwrap();
    assert_eq!(lonely.set.table, DqTable::Unknown);
    assert!(lonely.dq_ref.is_none());
    assert_eq!(lonely.bits_present, None);

    let own = dq_table_for(&format!("{}#hdu=3", p)).unwrap();
    assert_eq!(own.set.table, DqTable::Jwst);
    assert_eq!(own.dq_ref.as_deref(), Some(format!("{}#hdu=3", p).as_str()));
}

#[tokio::test]
async fn dq_flag_table_lists_raw_bits_for_an_unknown_mask_and_names_for_jwst() {
    use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_test_mef, HduData, TestHdu};
    let dir = tempfile::tempdir().unwrap();
    let hsc = dir.path().join("hsc_mask.fits");
    let mut mask = vec![0i32; 16];
    mask[5] = 1;
    write_test_mef(
        &hsc,
        &[("TELESCOP", "'Subaru'".into())],
        &[
            TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
            TestHdu { extname: Some("MASK"), extver: None, cols: 4, rows: 4, data: HduData::I32(mask), extra_cards: vec![] },
        ],
    );
    let k = format!("{}#hdu=2", hsc.to_str().unwrap());
    let unknown = get_dq_flag_table_cmd(k.clone()).await.unwrap();
    assert_eq!(unknown[RES_TABLE], "unknown");
    assert_eq!(unknown[RES_LABEL], "unknown (raw bits)");
    assert_eq!(unknown[RES_FLAGS], json!([]));
    assert_eq!(unknown[RES_BITS_PRESENT], 1);
    assert_eq!(unknown[RES_EXCLUSION_MASK], 0);
    assert_eq!(unknown[RES_DEFAULT_MASK], 0);
    assert_eq!(unknown[RES_DQ_REF], k);

    let jwst = dir.path().join("jwst_bits.fits");
    let raw = |bits: u32| (bits as i64 - 2147483648) as i32;
    let mut dq = vec![raw(0); 16];
    dq[0] = raw(1);
    dq[7] = raw(4);
    dq[15] = raw(1 << 31);
    sci_err_dq_mef_with_dq_cards(&jwst, 4, 4, dq, vec![("TELESCOP", "'JWST'".into())]);
    let p = jwst.to_str().unwrap();
    let named = get_dq_flag_table_cmd(format!("{p}#hdu=1")).await.unwrap();
    assert_eq!(named[RES_TABLE], "jwst");
    assert_eq!(named[RES_FLAGS].as_array().unwrap().len(), 32);
    assert!(named[RES_BITS_PRESENT].is_null(), "{named}");
    assert_eq!(named[RES_EXCLUSION_MASK], 1);
    let lonely = get_dq_flag_table_cmd(format!("{p}#hdu=4")).await.unwrap();
    assert!(lonely[RES_BITS_PRESENT].is_null(), "{lonely}");
    assert!(lonely[RES_DQ_REF].is_null());
}

#[tokio::test]
async fn the_flag_table_of_an_unreadable_dq_has_no_bits_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let p = float_dq_mef(dir.path(), "float_dq.fits");

    let t = get_dq_flag_table_cmd(format!("{p}#hdu=1")).await.unwrap();
    assert!(t[RES_BITS_PRESENT].is_null(), "{t}");
    assert!(t[RES_DQ_REF].is_null(), "{t}");
    assert_eq!(
        t["dq_unavailable"],
        "mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly"
    );
}

#[tokio::test]
async fn the_flag_table_of_an_afw_mask_lists_the_names_of_its_cards() {
    let dir = tempfile::tempdir().unwrap();
    let p = afw_mef(dir.path(), "calexp.fits", vec![]);

    let t = get_dq_flag_table_cmd(format!("{p}#hdu=1")).await.unwrap();
    assert_eq!(t[RES_TABLE], "unknown");
    assert_eq!(t[RES_LABEL], "MP_ cards (afw mask planes)");
    assert_eq!(t[RES_SOURCE], t[RES_LABEL]);
    assert_eq!(t[RES_FLAGS], json!([{"bit": 1, "name": "BAD"}, {"bit": 2, "name": "SAT"}, {"bit": 32, "name": "DETECTED"}]));
    assert_eq!(t[RES_EXCLUSION_MASK], 3);
    assert_eq!(t["exclusion_default"], 3);
    assert_eq!(t["exclusion_edited"], false);
    assert_eq!(t["saturated_mask"], 2);
    assert_eq!(t[RES_DEFAULT_MASK], 3);
    assert!(t[RES_BITS_PRESENT].is_null(), "{t}");
    assert_eq!(t[RES_DQ_REF], format!("{p}#hdu=2"));
    assert_eq!(t["dq_origin"], "EXTNAME MASK, HDU 2");
    assert!(t["dq_unavailable"].is_null(), "{t}");
    assert_eq!(
        t["err_origin"],
        json!({"role": "variance", "source": "EXTNAME VARIANCE, HDU 3: square root", "assumed": false, "n_no_data": 1, "origin": "same_file"})
    );
    assert!(t["err_unavailable"].is_null(), "{t}");
}

#[tokio::test]
async fn the_hover_readout_names_the_flags_and_the_origin_of_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let k = format!("{}#hdu=1", afw_mef(dir.path(), "calexp.fits", vec![]));

    let j = probe_pixel_cmd(k.clone(), 1.0, 1.0, None).await.unwrap();
    assert_eq!(
        j[RES_DQ],
        json!({"bits": 34, "value": 34, "names": ["SAT", "DETECTED"], "table": "unknown", "text": "34: SAT | DETECTED", "source": "MP_ cards (afw mask planes)"})
    );
    assert_eq!(j[RES_ERR], json!({"value": 5.0, "unit": "nJy", "source": "EXTNAME VARIANCE, HDU 3: square root"}));

    let no_data = probe_pixel_cmd(k, 2.0, 1.0, None).await.unwrap();
    assert!(no_data[RES_ERR].is_null(), "{no_data}");
}

#[tokio::test]
async fn the_pixel_table_names_the_layers() {
    let dir = tempfile::tempdir().unwrap();
    let k = format!("{}#hdu=1", afw_mef(dir.path(), "calexp.fits", vec![]));

    let t = pixel_table_cmd(k, 1, 1, Some(3)).await.unwrap();
    assert_eq!(t["err_source"], "EXTNAME VARIANCE, HDU 3: square root");
    assert_eq!(t["dq_source"], "MP_ cards (afw mask planes)");
    assert_eq!(t[RES_DQ_TABLE], "MP_ cards (afw mask planes)");
    assert_eq!(t[RES_DQ_NAMES][1][1], "SAT | DETECTED");
    assert_eq!(t[RES_ERR][1][1], 5.0);
    for key in ["err_unavailable", "dq_unavailable"] {
        assert!(t.as_object().unwrap().contains_key(key), "{key}: {t}");
        assert!(t[key].is_null(), "{key}: {t}");
    }
}

#[tokio::test]
async fn deep_zoom_tiles_of_an_rgb_file_come_from_that_file() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("tiles_rgb.fits").to_str().unwrap().to_string();
    let r = ndarray::Array2::from_shape_fn((9, 7), |(y, x)| (y * 7 + x) as f32);
    crate::infra::fits::writer::write_fits_rgb(&src, &r, &r.mapv(|v| v * 2.0), &r.mapv(|v| v * 3.0), None).unwrap();
    let out = dir.path().join("tiles").to_str().unwrap().to_string();

    let pyramid = generate_tiles_rgb(out.clone(), 256, Some(src)).await.unwrap();
    assert_eq!(pyramid["original_width"], 7);
    assert_eq!(pyramid["original_height"], 9);
    assert_eq!(pyramid["base_dir"], out.as_str());
}

#[tokio::test]
async fn tile_sizes_that_would_divide_by_zero_or_explode_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap().to_string();
    for bad in [0u32, 8, 100_000] {
        let err = generate_tiles(dir.path().join("none.fits").to_str().unwrap().to_string(), out.clone(), bad)
            .await
            .unwrap_err();
        assert!(err.contains("tile_size"), "{err}");
        let err = generate_tiles_rgb(out.clone(), bad, None).await.unwrap_err();
        assert!(err.contains("tile_size"), "{err}");
    }
    assert_eq!(tile_params(256).unwrap().tile_size, 256);
}

#[tokio::test]
async fn deep_zoom_tiles_of_a_cleared_composite_say_to_run_blend_again() {
    let _guard = helpers::composite_test_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap().to_string();
    helpers::clear_composite();
    let err = generate_tiles_rgb(out, 256, None).await.expect_err("the composite was cleared");
    assert_eq!(err, "The colour composite is no longer in memory; run Blend again.");
}

#[tokio::test]
async fn an_stf_render_of_a_large_image_matches_the_gpu_view_with_the_same_stf() {
    use crate::cmd::io::test_support::{assert_matches_gpu_view, wide_sky};
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("wide_stf.fits").to_str().unwrap().to_string();
    crate::infra::fits::writer::write_fits_mono(&src, &wide_sky(6), None).unwrap();
    let out = dir.path().to_str().unwrap().to_string();

    let value = apply_stf_render(src.clone(), out, 0.1, 0.3, 0.95).await.unwrap();
    let entry = load_cached(&src).unwrap();
    let stf = StfParams { shadow: 0.1, midtone: 0.3, highlight: 0.95 };
    assert_matches_gpu_view(value[RES_PNG_PATH].as_str().unwrap(), entry.arr(), &stf, entry.stats());
}

#[test]
fn tile_sizes_the_pyramid_builder_refuses_are_rejected_before_any_work() {
    for size in [tiles::MIN_TILE_SIZE, tiles::MAX_TILE_SIZE] {
        assert_eq!(tile_params(size as u32).unwrap().tile_size, size);
    }
    for bad in [16u32, 32, 63, 1025, 2048, 4096] {
        let err = tile_params(bad).unwrap_err().to_string();
        assert!(err.contains("tile_size") && err.contains("1024"), "{bad}: {err}");
    }
}

#[tokio::test]
async fn a_display_referred_image_keeps_its_computed_stretch_under_automatic_limits() {
    let dir = tempfile::tempdir().unwrap();
    let data = ndarray::Array2::from_shape_fn((16, 16), |(y, x)| 0.2 + (y * 16 + x) as f32 / 640.0);
    let mut flagged = HduHeader::empty();
    flagged.set(crate::cmd::common::HEADER_DISPLAY_REFERRED, "T".to_string());
    let stretched = dir.path().join("limits_arcsinh.fits").to_str().unwrap().to_string();
    let linear = dir.path().join("limits_linear.fits").to_str().unwrap().to_string();
    crate::infra::fits::writer::write_fits_mono(&stretched, &data, Some(&flagged)).unwrap();
    crate::infra::fits::writer::write_fits_mono(&linear, &data, None).unwrap();

    for algorithm in ["minmax", "zscale", "percentile"] {
        let limits = compute_scale_limits_cmd(stretched.clone(), algorithm.into(), None, None, None, None, None, None).await.unwrap();
        assert_eq!((limits[RES_VMIN].as_f64(), limits[RES_VMAX].as_f64()), (Some(0.0), Some(1.0)), "{algorithm}");
        let own = compute_scale_limits_cmd(linear.clone(), algorithm.into(), None, None, None, None, None, None).await.unwrap();
        assert!(own[RES_VMIN].as_f64().unwrap() > 0.1, "{algorithm} on linear data: {own}");
    }
    let user = compute_scale_limits_cmd(stretched, "user".into(), Some(0.3), Some(0.4), None, None, None, None).await.unwrap();
    assert_eq!((user[RES_VMIN].as_f64(), user[RES_VMAX].as_f64()), (Some(0.3), Some(0.4)));
}

fn write_ramp_16x16(dir: &std::path::Path, name: &str, header: Option<&HduHeader>) -> String {
    let data = ndarray::Array2::from_shape_fn((16, 16), |(y, x)| 0.2 + (y * 16 + x) as f32 / 640.0);
    let path = dir.join(name).to_str().unwrap().to_string();
    crate::infra::fits::writer::write_fits_mono(&path, &data, header).unwrap();
    path
}

fn display_referred_header() -> HduHeader {
    let mut flagged = HduHeader::empty();
    flagged.set(crate::cmd::common::HEADER_DISPLAY_REFERRED, "T".to_string());
    flagged
}

#[test]
fn symmetric_limits_mirror_the_base_pair_about_the_centre() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_ramp_16x16(dir.path(), "sym_ramp.fits", None);

    let plain = scale_limits_json(&path, LimitMode::MinMax, false, None).unwrap();
    let hi = plain[RES_VMAX].as_f64().unwrap();
    assert_eq!(plain[RES_VMIN].as_f64().unwrap(), 0.2f32 as f64);
    assert_eq!(hi, (0.2f32 + 0.3984375f32) as f64);
    assert_eq!(plain[RES_SYMMETRIC], false);
    assert!(plain[RES_CENTRE].is_null());
    assert_eq!(plain[RES_NOTES], json!([]));

    let sym = scale_limits_json(&path, LimitMode::MinMax, true, None).unwrap();
    assert_eq!(sym[RES_VMIN].as_f64().unwrap(), -hi);
    assert_eq!(sym[RES_VMAX].as_f64().unwrap(), hi);
    assert_eq!(sym[RES_SYMMETRIC], true);
    assert_eq!(sym[RES_CENTRE], 0.0);
    assert_eq!(sym[RES_NOTES], json!([]));
    assert_eq!(sym[RES_ALGORITHM], "minmax");

    let shifted = scale_limits_json(&path, LimitMode::MinMax, true, Some(0.4)).unwrap();
    let lo = 0.2f32 as f64;
    assert!((shifted[RES_VMIN].as_f64().unwrap() - lo).abs() < 1e-12, "{shifted}");
    assert!((shifted[RES_VMAX].as_f64().unwrap() - (0.8 - lo)).abs() < 1e-12, "{shifted}");
    assert_eq!(shifted[RES_CENTRE], 0.4);
    assert_eq!(shifted[RES_SYMMETRIC], true);
}

#[test]
fn user_mode_with_one_bound_symmetrises_the_filled_pair() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_ramp_16x16(dir.path(), "sym_user.fits", None);
    let hi = scale_limits_json(&path, LimitMode::MinMax, false, None).unwrap()[RES_VMAX].as_f64().unwrap();
    let mode = LimitMode::User { vmin: Some(-0.1), vmax: None };
    let sym = scale_limits_json(&path, mode, true, Some(0.0)).unwrap();
    assert_eq!(sym[RES_VMIN].as_f64().unwrap(), -hi);
    assert_eq!(sym[RES_VMAX].as_f64().unwrap(), hi);
    assert_eq!(sym[RES_ALGORITHM], "user");
    assert_eq!(sym[RES_SYMMETRIC], true);
}

#[test]
fn symmetric_limits_are_skipped_on_a_display_referred_image_with_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_ramp_16x16(dir.path(), "sym_abdisp.fits", Some(&display_referred_header()));
    let auto = scale_limits_json(&path, LimitMode::MinMax, true, None).unwrap();
    assert_eq!((auto[RES_VMIN].as_f64(), auto[RES_VMAX].as_f64()), (Some(0.0), Some(1.0)));
    let user = LimitMode::User { vmin: Some(0.3), vmax: Some(0.4) };
    let own = scale_limits_json(&path, user, true, Some(0.0)).unwrap();
    assert_eq!((own[RES_VMIN].as_f64(), own[RES_VMAX].as_f64()), (Some(0.3), Some(0.4)));
    for reply in [auto, own] {
        assert_eq!(reply[RES_SYMMETRIC], false, "{reply}");
        assert!(reply[RES_CENTRE].is_null(), "{reply}");
        let notes = reply[RES_NOTES].as_array().unwrap();
        assert_eq!(notes.len(), 1, "{reply}");
        let note = notes[0].as_str().unwrap();
        assert!(note.contains("display-referred"), "{note}");
        assert!(!note.contains("0..1"), "{note}");
    }
}

#[test]
fn symmetric_limits_on_a_constant_image_fall_back_with_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("flat3.fits").to_str().unwrap().to_string();
    crate::infra::fits::writer::write_fits_mono(&small, &ndarray::Array2::from_elem((8, 8), 3.0f32), None).unwrap();
    let reply = scale_limits_json(&small, LimitMode::MinMax, true, Some(3.0)).unwrap();
    assert_eq!((reply[RES_VMIN].as_f64(), reply[RES_VMAX].as_f64()), (Some(2.0), Some(4.0)));
    assert_eq!(reply[RES_SYMMETRIC], true);
    assert_eq!(reply[RES_CENTRE], 3.0);
    assert!(reply[RES_NOTES][0].as_str().unwrap().contains("half-width"), "{reply}");

    let large = dir.path().join("flat3e7.fits").to_str().unwrap().to_string();
    crate::infra::fits::writer::write_fits_mono(&large, &ndarray::Array2::from_elem((8, 8), 3e7f32), None).unwrap();
    let reply = scale_limits_json(&large, LimitMode::MinMax, true, Some(3e7)).unwrap();
    let (vmin, vmax) = (reply[RES_VMIN].as_f64().unwrap(), reply[RES_VMAX].as_f64().unwrap());
    assert!((vmin as f32) < (vmax as f32), "{reply}");
    assert!(((vmin + vmax) / 2.0 - 3e7).abs() < 1e-6, "{reply}");
    assert!(reply[RES_NOTES][0].as_str().unwrap().contains("half-width"), "{reply}");
}

#[test]
fn a_non_finite_or_out_of_range_centre_is_refused_before_loading() {
    let err = scale_limits_json("missing.fits", LimitMode::MinMax, true, Some(f64::NAN)).unwrap_err().to_string();
    assert!(err.contains("centre must be finite"), "{err}");
    let err = scale_limits_json("missing.fits", LimitMode::MinMax, true, Some(1e308)).unwrap_err().to_string();
    assert!(err.contains("centre must be within"), "{err}");

    let dir = tempfile::tempdir().unwrap();
    let path = write_ramp_16x16(dir.path(), "sym_unused_centre.fits", None);
    let reply = scale_limits_json(&path, LimitMode::MinMax, false, Some(1e308)).unwrap();
    assert_eq!(reply[RES_SYMMETRIC], false);
    assert!(reply[RES_CENTRE].is_null());
    assert_eq!(reply[RES_NOTES], json!([]));
}

#[tokio::test]
async fn get_colormap_lut_reports_the_no_data_colour() {
    let bwr = get_colormap_lut_cmd("bwr".into()).await.unwrap();
    assert_eq!(bwr[RES_NODATA], json!([64, 64, 64]));
    assert_eq!(bwr[RES_RGBA].as_array().unwrap().len(), 1024);
    assert_eq!(bwr[RES_COLORMAPS].as_array().unwrap().len(), 13);
    assert_eq!(bwr[RES_NAME], "bwr");
    let gray = get_colormap_lut_cmd("gray".into()).await.unwrap();
    assert_eq!(gray[RES_NODATA], json!([0, 0, 0]));
}


#[tokio::test]
async fn pixel_table_reports_the_grid_the_companions_and_the_stats_with_null_padding_at_the_edge() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.fits");
    let mut dq = vec![-2147483648i32; 16];
    dq[5] = -2147483645;
    sci_err_dq_mef_with_dq_cards(&path, 4, 4, dq, vec![("TELESCOP", "'JWST'".into())]);
    let key = format!("{}#hdu=1", path.to_str().unwrap());

    let t = pixel_table_cmd(key.clone(), 0, 1, Some(3)).await.unwrap();
    assert_eq!(t[RES_X], 0);
    assert_eq!(t[RES_Y], 1);
    assert_eq!(t[RES_SIZE], 3);
    assert_eq!(t[RES_X0], -1);
    assert_eq!(t[RES_Y0], 0);
    assert_eq!(t[RES_VALUES], json!([[null, 0.0, 1.0], [null, 4.0, 5.0], [null, 8.0, 9.0]]));
    assert_eq!(t[RES_ERR], json!([[null, 0.0, 0.5], [null, 2.0, 2.5], [null, 4.0, 4.5]]));
    assert_eq!(t[RES_DQ], json!([[null, 0, 0], [null, 0, 3], [null, 0, 0]]));
    assert_eq!(t[RES_DQ_NAMES], json!([[null, null, null], [null, null, "DO_NOT_USE | SATURATED"], [null, null, null]]));
    assert_eq!(t[RES_DQ_TABLE], "jwst");
    assert_eq!(t[RES_UNIT], "MJy/sr");
    assert_eq!(t[RES_STATS]["min"], 0.0);
    assert_eq!(t[RES_STATS]["max"], 9.0);
    assert_eq!(t[RES_STATS]["mean"], 4.5);
    assert_eq!(t[RES_STATS]["median"], 4.5);
    assert_eq!(t[RES_STATS]["n_finite"], 6);
    assert_eq!(t[RES_STATS]["n_nan"], 0);
    assert_eq!(
        t[RES_ERR_STATS],
        json!({ "min": 0.0, "max": 4.5, "mean": 2.25, "median": 2.25, "n_finite": 6, "n_nan": 0 })
    );
    assert!(t[RES_ELAPSED_MS].is_number());

    let far = pixel_table_cmd(key.clone(), 100, -100, None).await.unwrap();
    assert_eq!(far[RES_SIZE], 7);
    assert_eq!(far[RES_VALUES].as_array().unwrap().len(), 7);
    assert!(far[RES_VALUES].as_array().unwrap().iter().flat_map(|r| r.as_array().unwrap()).all(|c| c.is_null()));
    assert_eq!(far[RES_STATS]["n_finite"], 0);
    assert!(far[RES_STATS]["median"].is_null());

    let plain = dir.path().join("plain.fits").to_str().unwrap().to_string();
    let data = ndarray::Array2::from_shape_fn((5, 5), |(y, x)| (y * 5 + x) as f32);
    crate::infra::fits::writer::write_fits_mono(&plain, &data, None).unwrap();
    let p = pixel_table_cmd(plain, 2, 2, Some(3)).await.unwrap();
    assert_eq!(p[RES_VALUES][1][1], 12.0);
    assert!(p[RES_ERR].is_null());
    assert!(p[RES_ERR_STATS].is_null());
    assert!(p[RES_DQ].is_null());
    assert!(p[RES_DQ_NAMES].is_null());
    assert!(p[RES_DQ_TABLE].is_null());
    assert!(p[RES_UNIT].is_null());
}

#[tokio::test]
async fn probe_pixel_reports_the_wavelength_companion_with_its_unit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wave_probe.fits");
    let mut wave = vec![2.0f32; 16];
    wave[5] = 1.654;
    wave[15] = f32::NAN;
    sci_err_dq_wave_mef(&path, 4, 4, vec![-2147483648i32; 16], wave);
    let key = format!("{}#hdu=1", path.to_str().unwrap());

    let j = probe_pixel_cmd(key.clone(), 1.0, 1.0, Some(1)).await.unwrap();
    assert!((j[RES_WAVELENGTH][RES_VALUE].as_f64().unwrap() - 1.654).abs() < 1e-6, "{j}");
    assert_eq!(j[RES_WAVELENGTH][RES_UNIT], "um");
    assert_eq!(j[RES_ERR][RES_VALUE], 2.5);
    assert_eq!(j[RES_VALUE], 5.0);

    let nan = probe_pixel_cmd(key, 3.0, 3.0, Some(1)).await.unwrap();
    assert!(nan.as_object().unwrap().contains_key(RES_WAVELENGTH), "{nan}");
    assert!(nan[RES_WAVELENGTH].is_null(), "{nan}");
    assert_eq!(nan[RES_VALUE], 15.0);

    let plain = dir.path().join("plain_probe.fits").to_str().unwrap().to_string();
    let data = ndarray::Array2::from_shape_fn((5, 5), |(y, x)| (y * 5 + x) as f32);
    crate::infra::fits::writer::write_fits_mono(&plain, &data, None).unwrap();
    let p = probe_pixel_cmd(plain, 2.0, 2.0, Some(1)).await.unwrap();
    assert_eq!(p[RES_VALUE], 12.0);
    assert!(p.as_object().unwrap().contains_key(RES_WAVELENGTH), "{p}");
    assert!(p[RES_WAVELENGTH].is_null(), "{p}");
}

#[tokio::test]
async fn pixel_table_includes_the_wavelength_grid_stats_and_unit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wave_table.fits");
    let wave: Vec<f32> = (0..16).map(|i| 1.0 + 0.25 * i as f32).collect();
    sci_err_dq_wave_mef(&path, 4, 4, vec![-2147483648i32; 16], wave);
    let key = format!("{}#hdu=1", path.to_str().unwrap());

    let t = pixel_table_cmd(key, 0, 1, Some(3)).await.unwrap();
    assert_eq!(t[RES_WAVELENGTH], json!([[null, 1.0, 1.25], [null, 2.0, 2.25], [null, 3.0, 3.25]]));
    assert_eq!(
        t[RES_WAVELENGTH_STATS],
        json!({ "min": 1.0, "max": 3.25, "mean": 2.125, "median": 2.125, "n_finite": 6, "n_nan": 0 })
    );
    assert_eq!(t[RES_WAVELENGTH_UNIT], "um");
    assert_eq!(t[RES_VALUES][1][1], 4.0);
    assert_eq!(t[RES_ERR][1][2], 2.5);

    let plain = dir.path().join("plain_table.fits").to_str().unwrap().to_string();
    let data = ndarray::Array2::from_shape_fn((5, 5), |(y, x)| (y * 5 + x) as f32);
    crate::infra::fits::writer::write_fits_mono(&plain, &data, None).unwrap();
    let p = pixel_table_cmd(plain, 2, 2, Some(3)).await.unwrap();
    for key in [RES_WAVELENGTH, RES_WAVELENGTH_STATS, RES_WAVELENGTH_UNIT] {
        assert!(p.as_object().unwrap().contains_key(key), "{key}: {p}");
        assert!(p[key].is_null(), "{key}: {p}");
    }
}

#[tokio::test]
async fn probe_pixel_reads_the_wavelength_unit_from_bunit_and_falls_back_to_um_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let wave: Vec<f32> = (0..16).map(|i| 1.6 + 0.01 * i as f32).collect();
    let bare = dir.path().join("wave_nobunit.fits");
    sci_err_dq_wave_mef_with_wave_cards(&bare, 4, 4, vec![-2147483648i32; 16], wave.clone(), vec![]);
    let j = probe_pixel_cmd(format!("{}#hdu=1", bare.to_str().unwrap()), 2.0, 1.0, Some(1)).await.unwrap();
    assert!((j[RES_WAVELENGTH][RES_VALUE].as_f64().unwrap() - wave[6] as f64).abs() < 1e-6, "{j}");
    assert_eq!(j[RES_WAVELENGTH][RES_UNIT], "um", "{j}");

    let nm = dir.path().join("wave_nm.fits");
    sci_err_dq_wave_mef_with_wave_cards(&nm, 4, 4, vec![-2147483648i32; 16], wave, vec![("BUNIT", "'nm'".into())]);
    let j = probe_pixel_cmd(format!("{}#hdu=1", nm.to_str().unwrap()), 2.0, 1.0, Some(1)).await.unwrap();
    assert_eq!(j[RES_WAVELENGTH][RES_UNIT], "nm", "{j}");
}

#[tokio::test]
async fn pixel_table_reads_the_wavelength_unit_from_bunit_and_falls_back_to_um_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let wave: Vec<f32> = (0..16).map(|i| 1.6 + 0.01 * i as f32).collect();
    let bare = dir.path().join("table_nobunit.fits");
    sci_err_dq_wave_mef_with_wave_cards(&bare, 4, 4, vec![-2147483648i32; 16], wave.clone(), vec![]);
    let t = pixel_table_cmd(format!("{}#hdu=1", bare.to_str().unwrap()), 1, 1, Some(3)).await.unwrap();
    assert_eq!(t[RES_WAVELENGTH_UNIT], "um", "{t}");
    assert!((t[RES_WAVELENGTH][1][1].as_f64().unwrap() - wave[5] as f64).abs() < 1e-6, "{t}");
    assert_eq!(t[RES_WAVELENGTH_STATS]["n_finite"], 9, "{t}");

    let nm = dir.path().join("table_nm.fits");
    sci_err_dq_wave_mef_with_wave_cards(&nm, 4, 4, vec![-2147483648i32; 16], wave, vec![("BUNIT", "'nm'".into())]);
    let t = pixel_table_cmd(format!("{}#hdu=1", nm.to_str().unwrap()), 1, 1, Some(3)).await.unwrap();
    assert_eq!(t[RES_WAVELENGTH_UNIT], "nm", "{t}");
}

#[tokio::test]
async fn pixel_table_err_stats_come_from_the_err_plane_including_its_own_nan_count() {
    use crate::infra::fits::reader::test_fixtures::{
        ramp_f32, write_test_mef, HduData, TestHdu,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("err_nan.fits");
    let mut err = ramp_f32(4, 4).iter().map(|v| v * 0.5).collect::<Vec<f32>>();
    err[5] = f32::NAN;
    write_test_mef(
        &path,
        &[],
        &[
            TestHdu {
                extname: Some("SCI"),
                extver: Some(1),
                cols: 4,
                rows: 4,
                data: HduData::F32(ramp_f32(4, 4)),
                extra_cards: vec![],
            },
            TestHdu {
                extname: Some("ERR"),
                extver: Some(1),
                cols: 4,
                rows: 4,
                data: HduData::F32(err),
                extra_cards: vec![],
            },
        ],
    );
    let key = format!("{}#hdu=1", path.to_str().unwrap());

    let t = pixel_table_cmd(key, 0, 1, Some(3)).await.unwrap();
    assert_eq!(
        t[RES_ERR],
        json!([[null, 0.0, 0.5], [null, 2.0, null], [null, 4.0, 4.5]])
    );
    assert_eq!(
        t[RES_ERR_STATS],
        json!({ "min": 0.0, "max": 4.5, "mean": 2.2, "median": 2.0, "n_finite": 5, "n_nan": 1 })
    );
    assert_eq!(t[RES_STATS]["max"], 9.0);
    assert_eq!(t[RES_STATS]["n_finite"], 6);
    assert_eq!(t[RES_STATS]["n_nan"], 0);
}

#[tokio::test]
async fn pixel_table_refuses_even_and_out_of_range_sizes_before_loading_the_image() {
    let missing = "/nonexistent/pixel_table.fits".to_string();
    for bad in [4usize, 17, 1, 0, 2, 16] {
        let err = pixel_table_cmd(missing.clone(), 0, 0, Some(bad)).await.unwrap_err();
        assert!(err.contains("size must be an odd number between 3 and 15"), "{bad}: {err}");
        assert!(err.contains(&format!("got {bad}")), "{bad}: {err}");
    }
    assert_eq!(pixel_table_size(None).unwrap(), 7);
    assert_eq!(pixel_table_size(Some(15)).unwrap(), 15);
    assert_eq!(pixel_table_size(Some(3)).unwrap(), 3);
}

#[test]
fn dq_name_grid_decodes_set_bits_and_leaves_good_and_off_image_cells_null() {
    let bits = vec![vec![None, Some(0), Some(1)], vec![Some(6), Some(1 << 20), None]];
    let names = dq_name_grid(&bits, &FlagSet::from_table(DqTable::Jwst));
    assert_eq!(names[0], vec![None, None, Some("DO_NOT_USE".to_string())]);
    assert_eq!(names[1][0].as_deref(), Some("SATURATED | JUMP_DET"));
    assert!(names[1][1].is_some());
    assert_eq!(names[1][2], None);
}

#[test]
fn mask_preview_payload_from_mef() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mask.fits");
    let mut dq = vec![-2147483648i32; 16];
    dq[3] = -2147483647;
    sci_err_dq_mef(&path, 4, 4, dq);
    let key = format!("{}#hdu=1", path.to_str().unwrap());
    let loaded = load_companions(&key).unwrap();
    let flags = loaded.flags.clone().unwrap();
    let (entry, table) = loaded.dq.unwrap();
    assert_eq!(table, DqTable::Unknown);
    let plane = entry.int_plane().unwrap();
    let applied = resolve_overlay_mask(&flags, None);
    assert_eq!(applied, 0);
    assert!(mask_preview_or(&plane.bits, applied, 2).0.iter().all(|&c| c == 0));
    let applied = resolve_overlay_mask(&flags, Some(1));
    let (cells, w, h) = mask_preview_or(&plane.bits, applied, 2);
    let bytes = encode_mask_with_header(&cells, w as u32, h as u32, applied, table.id());
    assert_eq!(bytes.len(), 16 + 4);
    assert_eq!(u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]), 1);
    assert_eq!(u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]), 0xFFFF_FFFF);
    assert_eq!(&bytes[16..], &[0, 1, 0, 0]);
}

fn survey_file(set: &str, name: &str) -> String {
    let root = crate::core::astrometry::gwcs::test_support::env_dir("ASTROBURST_SCIENCE_DATA_DIR");
    root.join("survey").join(set).join(name).to_str().unwrap().to_string()
}

#[tokio::test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/E1 science, RMS and FLAG files of tile 102044185"]
async fn real_euclid_open_response_and_flag_table_name_the_paired_files() {
    let rms = "EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits";
    let flag = "EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits";
    let p = survey_file("E1", "EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits");

    let plane = crate::cmd::common::plane_info_json(&p, &load_cached_full(&p).unwrap()).unwrap();
    println!("euclid open response: err_origin {}, dq_origin {}, notes {}", plane["err_origin"], plane["dq_origin"], plane["pairing_notes"]);
    assert_eq!(plane["err_origin"]["source"], format!("file name -RMS, {rms}: as is"));
    assert_eq!(plane["err_origin"]["origin"], "sibling");
    assert_eq!(
        plane["pairing_notes"],
        json!([
            format!("error layer taken from the sibling file {rms} (name rule Euclid MER mosaic, same 601x601 grid)"),
            format!("mask layer taken from the sibling file {flag} (name rule Euclid MER mosaic, same 601x601 grid)"),
            "flag layer without bit names: nothing is excluded until bits are chosen",
        ])
    );

    let t = get_dq_flag_table_cmd(p.clone()).await.unwrap();
    println!("euclid flag table: label {}, bits_present {}, exclusion {}, err n_no_data {}", t[RES_LABEL], t[RES_BITS_PRESENT], t[RES_EXCLUSION_MASK], t["err_origin"]["n_no_data"]);
    assert_eq!(t[RES_LABEL], "unknown (raw bits)");
    assert_eq!(t[RES_FLAGS], json!([]));
    assert_eq!(t[RES_BITS_PRESENT], 25301342);
    assert_eq!(t[RES_EXCLUSION_MASK], 0);
    assert_eq!(t["err_origin"]["n_no_data"], 22);

    let sentinel = probe_pixel_cmd(p.clone(), 541.0, 70.0, None).await.unwrap();
    let corner = probe_pixel_cmd(p, 0.0, 0.0, None).await.unwrap();
    println!("euclid probe: (541, 70) value {} err {}; (0, 0) err {}", sentinel[RES_VALUE], sentinel[RES_ERR], corner[RES_ERR]);
    assert!(sentinel[RES_ERR].is_null(), "{sentinel}");
    assert_eq!(sentinel[RES_VALUE], 0.0);
    assert_eq!(corner[RES_ERR][RES_VALUE], 0.004187881480902433_f32 as f64);
    assert!(corner[RES_ERR][RES_UNIT].is_null(), "{corner}");
}

#[tokio::test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-image-r.fits.fz with its invvar and maskbits files"]
async fn real_legacy_brick_flag_table_has_the_names_of_the_file() {
    let p = survey_file("L2", "legacysurvey-0532m280-image-r.fits.fz");

    let t = get_dq_flag_table_cmd(p.clone()).await.unwrap();
    println!("legacy flag table: label {}, {} flags, exclusion {}, saturated {}, bits_present {}", t[RES_LABEL], t[RES_FLAGS].as_array().map_or(0, Vec::len), t[RES_EXCLUSION_MASK], t["saturated_mask"], t[RES_BITS_PRESENT]);
    assert_eq!(t[RES_LABEL], "MBIT_ cards (Legacy maskbits)");
    assert_eq!(t[RES_FLAGS].as_array().unwrap().len(), 17);
    assert_eq!(t[RES_EXCLUSION_MASK], 72);
    assert_eq!(t["saturated_mask"], 8);
    assert!(t[RES_BITS_PRESENT].is_null(), "{t}");

    let star = probe_pixel_cmd(p.clone(), 2972.0, 893.0, None).await.unwrap();
    let corner = probe_pixel_cmd(p, 0.0, 0.0, None).await.unwrap();
    println!("legacy probe: (2972, 893) dq {}; (0, 0) err {}", star[RES_DQ], corner[RES_ERR]);
    assert_eq!(star[RES_DQ]["bits"], 51352);
    assert_eq!(star[RES_DQ]["names"], json!(["SATUR_R", "SATUR_Z", "ALLMASK_Z", "MEDIUM", "SATUR_I", "ALLMASK_I"]));
    let expected = 7.175998180759543e-4;
    let sigma = corner[RES_ERR][RES_VALUE].as_f64().unwrap();
    assert!((sigma - expected).abs() <= 1e-6 * expected, "{sigma} against {expected}");
    assert_eq!(corner[RES_ERR][RES_UNIT], "nanomaggy");
}

#[tokio::test]
#[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_mask_480px.fits"]
async fn real_ps1_mask_cutout_shown_on_its_own_says_it_holds_no_mask() {
    let p = survey_file("P1", "ps1_A_r_mask_480px.fits");

    let t = get_dq_flag_table_cmd(p).await.unwrap();
    println!("ps1 mask flag table: label {}, {} flags, dq_ref {}, bits_present {}, dq_unavailable {}", t[RES_LABEL], t[RES_FLAGS].as_array().map_or(0, Vec::len), t[RES_DQ_REF], t[RES_BITS_PRESENT], t["dq_unavailable"]);
    assert!(t[RES_DQ_REF].is_null(), "{t}");
    assert!(t[RES_BITS_PRESENT].is_null(), "{t}");
    assert!(t["dq_unavailable"].as_str().unwrap().contains("floating-point plane (BITPIX -32)"), "{t}");
    assert_eq!(t[RES_LABEL], "MSKNAM cards (PS1 mask)");
    assert_eq!(t[RES_FLAGS].as_array().unwrap().len(), 16);
}
