use serde_json::json;

use crate::core::metadata::header_discovery::detect_filter;
use crate::types::constants::{
    CATEGORY_INSTRUMENT, CATEGORY_OBSERVATION, CATEGORY_OTHER, CATEGORY_WCS, RES_CATEGORIES, RES_EXTENSIONS,
    RES_EXTNAME, RES_FILTER, RES_HUBBLE_CHANNEL, RES_IS_DQ, RES_IS_ERR, RES_PATH,
};

use super::filter_detection::narrowband_detections_json;
use super::header_cards::header_for;

use std::time::{Duration, SystemTime};

use ndarray::Array2;

use super::*;
use crate::cmd::common::load_cached_full;
use crate::core::imaging::region::test_support::make_header;
use crate::infra::fits::writer::write_fits_mono;

#[test]
fn the_header_explorer_reads_a_rewritten_file_again() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("explored.fits").to_str().unwrap().to_string();
    write_fits_mono(&p, &Array2::zeros((4, 4)), Some(&make_header(&[("EXPTIME", "300")]))).unwrap();
    assert!(load_cached_full(&p).unwrap().header().is_some());
    assert_eq!(header_for(&p).unwrap().get_f64("EXPTIME"), Some(300.0));

    write_fits_mono(&p, &Array2::zeros((4, 4)), Some(&make_header(&[("EXPTIME", "600")]))).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(5)).unwrap();
    drop(file);
    assert_eq!(header_for(&p).unwrap().get_f64("EXPTIME"), Some(600.0), "the cached header of the old file was served");
}

#[test]
fn the_detected_channel_follows_the_requested_palette() {
    let files = vec![
        ("ha.fits".to_string(), make_header(&[("FILTER", "H-alpha")])),
        ("sii.fits".to_string(), make_header(&[("FILTER", "SII")])),
        ("lum.fits".to_string(), make_header(&[("FILTER", "Luminance")])),
    ];
    let hoo = narrowband_detections_json(&files, &palette_from(Some("HOO")));
    assert_eq!(hoo[0][RES_HUBBLE_CHANNEL], "R");
    assert!(hoo[1][RES_HUBBLE_CHANNEL].is_null(), "SII has no channel in HOO: {}", hoo[1]);
    assert!(hoo[2][RES_FILTER].is_null());
    assert_eq!(hoo[2][RES_PATH], "lum.fits");
    assert_eq!(hoo[0][RES_PATH], "ha.fits");

    let sho = narrowband_detections_json(&files, &palette_from(None));
    assert_eq!(sho[0][RES_HUBBLE_CHANNEL], "G");
    assert_eq!(sho[1][RES_HUBBLE_CHANNEL], "R");
    let hos = detection_json(&detect_filter(&files[1].1).unwrap(), &palette_from(Some("hos")));
    assert_eq!(hos[RES_HUBBLE_CHANNEL], "B");
}

#[tokio::test]
async fn jwst_instrument_configuration_keys_group_under_instrument() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("jw01266005001_02103_00001_nrs1_cal.fits").to_str().unwrap().to_string();
    let keys = [
        ("TELESCOP", "JWST"), ("INSTRUME", "NIRSPEC"), ("DETECTOR", "NRS1"), ("FILTER", "F170LP"), ("GRATING", "G235H"),
        ("FOCUSPOS", "3"), ("LAMP", "NONE"), ("OPMODE", "IFU"), ("GWA_XTIL", "0.351098329"), ("GWA_TILT", "36.1412"),
        ("EXP_TYPE", "NRS_IFU"), ("READPATT", "NRSIRS2RAPID"), ("SUBARRAY", "FULL"), ("APERNAME", "NRS_FULL_IFU"),
        ("MODULE", "MULTIPLE"), ("CHANNEL", "LONG"), ("PUPIL", "F470N"), ("PILIN", "F"),
        ("BAND", "SHORT"), ("FPE_SIDE", "A"), ("ICE_SIDE", "A"), ("CCCSTATE", "OPEN"),
        ("FWPOSOFF", "0"), ("FWPOSTOL", "0"), ("PWPOSOFF", "0"), ("PWPOSTOL", "0"),
    ];
    write_fits_mono(&p, &Array2::zeros((4, 4)), Some(&make_header(&keys))).unwrap();
    let out = get_full_header(p, None).await.unwrap();
    let categories = &out[RES_CATEGORIES];
    let observation_keys = ["TELESCOP", "INSTRUME", "FILTER"];
    for (key, _) in keys.iter().filter(|(k, _)| !observation_keys.contains(k)) {
        assert!(categories[CATEGORY_INSTRUMENT].get(*key).is_some(), "{key} is not under Instrument: {}", categories);
        assert!(categories[CATEGORY_OTHER].get(*key).is_none(), "{key} is still under Other");
    }
    assert_eq!(categories[CATEGORY_OBSERVATION]["TELESCOP"], "JWST");
    assert_eq!(categories[CATEGORY_OBSERVATION]["INSTRUME"], "NIRSPEC");
    assert_eq!(categories[CATEGORY_OBSERVATION]["FILTER"], "F170LP");
}

#[tokio::test]
async fn sip_fit_residual_cards_are_grouped_under_wcs() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("sip_residual.fits").to_str().unwrap().to_string();
    let keys = [("CTYPE1", "RA---TAN-SIP"), ("CTYPE2", "DEC--TAN-SIP"), ("SIPMXERR", "0.0098"), ("SIPIVERR", "0.012")];
    write_fits_mono(&p, &Array2::zeros((4, 4)), Some(&make_header(&keys))).unwrap();
    let out = get_full_header(p, None).await.unwrap();
    let categories = &out[RES_CATEGORIES];
    for key in ["SIPMXERR", "SIPIVERR"] {
        assert!(categories[CATEGORY_WCS].get(key).is_some(), "{key} is not under WCS: {categories}");
        assert!(categories[CATEGORY_OTHER].get(key).is_none(), "{key} is still under Other");
    }
    assert!(categories[CATEGORY_WCS].get("CTYPE2").is_some());
}

fn column(listed: &serde_json::Value, key: &str) -> serde_json::Value {
    listed[RES_EXTENSIONS].as_array().unwrap().iter().map(|row| row[key].clone()).collect()
}

#[test]
fn the_extension_list_gives_the_role_and_unit_of_every_hdu() {
    use crate::cmd::common::test_support::afw_mef;
    use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_test_mef, HduData, TestHdu};
    let dir = tempfile::tempdir().unwrap();
    let wht = TestHdu { extname: Some("WHT"), extver: None, cols: 4, rows: 4, data: HduData::F32(vec![1.0; 16]), extra_cards: vec![] };
    let p = afw_mef(dir.path(), "calexp.fits", vec![wht]);

    let listed = extensions_json(&p).unwrap();
    assert_eq!(column(&listed, "role"), json!(["other", "science", "mask", "variance", "other"]));
    assert_eq!(
        column(&listed, "role_basis"),
        json!(["no image data", "EXTNAME IMAGE", "EXTNAME MASK", "EXTNAME VARIANCE", "EXTNAME WHT"])
    );
    assert_eq!(column(&listed, "role_assumed"), json!([false, false, false, false, false]));
    assert_eq!(column(&listed, "bunit"), json!([null, "nJy", null, "nJy^2", null]));
    assert_eq!(column(&listed, RES_IS_DQ), json!([false, false, true, false, false]));
    assert_eq!(column(&listed, RES_IS_ERR), json!([false, false, false, false, false]));

    let weighted = dir.path().join("weighted.fits");
    write_test_mef(
        &weighted,
        &[],
        &[
            TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
            TestHdu { extname: Some("WGT"), extver: None, cols: 4, rows: 4, data: HduData::F32(vec![4.0; 16]), extra_cards: vec![] },
        ],
    );
    let listed = extensions_json(weighted.to_str().unwrap()).unwrap();
    let wgt = &listed[RES_EXTENSIONS][2];
    assert_eq!(wgt["role"], "inverse_variance", "{wgt}");
    assert_eq!(wgt["role_basis"], "EXTNAME WGT");
    assert_eq!(wgt["role_assumed"], true);
}

#[test]
fn asdf_arrays_get_their_role_from_the_array_name() {
    use crate::infra::image_source::test_support::write_asdf;
    let dir = tempfile::tempdir().unwrap();
    let tree = "data: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\ndq: !core/ndarray-1.0.0\n  data: [[0, 1], [0, 0]]\n  datatype: uint32\nerr: !core/ndarray-1.0.0\n  data: [[0.1, 0.2], [0.3, 0.4]]\n  datatype: float32\n";
    let p = write_asdf(&dir, "layers.asdf", tree);

    let listed = extensions_json(&p).unwrap();
    assert_eq!(column(&listed, RES_EXTNAME), json!(["data", "dq", "err"]));
    assert_eq!(column(&listed, "role"), json!(["science", "mask", "sigma"]));
    assert_eq!(column(&listed, "bunit"), json!([null, null, null]));
    assert_eq!(column(&listed, "bands"), json!([[], [], []]));
}

#[test]
fn the_extension_list_names_the_bands_of_a_cube() {
    use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_test_mef, HduData, TestHdu};
    use crate::infra::image_source::test_support::{write_band_cutout, CUTOUT_INVVAR, CUTOUT_NAME};
    let dir = tempfile::tempdir().unwrap();
    let p = write_band_cutout(dir.path(), CUTOUT_NAME, CUTOUT_INVVAR);
    let bands_of = |hdu: usize| {
        json!([
            {"plane": 0, "label": "g", "key": format!("{p}#hdu={hdu}&plane=0")},
            {"plane": 1, "label": "r", "key": format!("{p}#hdu={hdu}&plane=1")},
        ])
    };

    let listed = extensions_json(&p).unwrap();
    assert_eq!(column(&listed, "bands"), json!([bands_of(0), bands_of(1), []]), "{listed}");
    assert_eq!(column(&listed, "role"), json!(["science", "inverse_variance", "mask"]));
    assert_eq!(column(&listed, "role_basis"), json!(["IMAGETYP IMAGE", "IMAGETYP INVVAR", "IMAGETYP MASKBITS"]));
    assert_eq!(extensions_json(&format!("{p}#hdu=0&plane=1")).unwrap(), listed);

    let two = dir.path().join("two_hdus.fits");
    write_test_mef(
        &two,
        &[],
        &[
            TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
            TestHdu { extname: Some("ERR"), extver: None, cols: 4, rows: 4, data: HduData::F32(vec![0.5; 16]), extra_cards: vec![] },
        ],
    );
    let listed = extensions_json(two.to_str().unwrap()).unwrap();
    assert_eq!(column(&listed, "bands"), json!([[], [], []]), "{listed}");
}
