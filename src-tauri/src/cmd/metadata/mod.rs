use std::fs::File;
use std::path::Path;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, image_ref, source_path};
use crate::core::imaging::layer_roles::{classify_layer, LayerGuess, LayerRole};
use crate::core::imaging::pixel_probe::data_unit;
use crate::core::metadata::header_discovery::{
    detect_filter, palette_channel, suggest_palette_with_type, FilterDetection, PaletteType,
};
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::fits::band_planes::{list_band_planes, HduBandPlanes};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::{extract_header_by_index, HduInfo};
use crate::infra::image_source::{is_dq_name, is_err_name, list_planes, load_plane_header, plane_ref};
use crate::types::constants::{
    RES_BITPIX, RES_CARDS, RES_CATEGORIES, RES_CONFIDENCE, RES_EXTENSIONS,
    RES_EXTNAME, RES_EXTVER, RES_FILE_NAME, RES_FILE_PATH, RES_FILENAME_HINT, RES_FILTER,
    RES_FILTER_DETECTION, RES_FILTER_ID, RES_FILTERS, RES_HAS_DATA,
    RES_HUBBLE_CHANNEL, RES_INDEX, RES_IS_DQ, RES_IS_ERR, RES_KEY, RES_KIND,
    RES_MATCHED_KEYWORD, RES_MATCHED_VALUE,
    RES_NAXIS, RES_NAXIS1, RES_NAXIS2, RES_NAXIS3, RES_NAXIS4, RES_PALETTE, RES_PATH, RES_REF,
    RES_TOTAL_CARDS, RES_VALUE,
    CATEGORY_OBSERVATION, CATEGORY_INSTRUMENT, CATEGORY_IMAGE,
    CATEGORY_WCS, CATEGORY_PROCESSING, CATEGORY_OTHER,
    PLANE_KIND_ARRAY, PLANE_KIND_HDU,
};
use crate::types::header::HduHeader;

const RES_ROLE: &str = "role";
const RES_ROLE_BASIS: &str = "role_basis";
const RES_ROLE_ASSUMED: &str = "role_assumed";
const RES_BUNIT: &str = "bunit";
const RES_BANDS: &str = "bands";
const NO_IMAGE_DATA: &str = "no image data";

fn header_for(path: &str) -> anyhow::Result<HduHeader> {
    cached_header(path)
}

fn palette_from(palette: Option<&str>) -> PaletteType {
    palette.map(PaletteType::from_str_loose).unwrap_or_default()
}

fn detection_json(detection: &FilterDetection, palette: &PaletteType) -> serde_json::Value {
    json!({
        RES_FILTER: detection.filter,
        RES_FILTER_ID: format!("{:?}", detection.filter),
        RES_HUBBLE_CHANNEL: palette_channel(palette, detection.filter),
        RES_CONFIDENCE: detection.confidence,
        RES_MATCHED_KEYWORD: detection.matched_keyword,
        RES_MATCHED_VALUE: detection.matched_value,
    })
}

fn narrowband_detections_json(file_headers: &[(String, HduHeader)], palette: &PaletteType) -> Vec<serde_json::Value> {
    file_headers
        .iter()
        .map(|(p, header)| {
            let mut entry = match detect_filter(header) {
                Some(detection) => detection_json(&detection, palette),
                None => json!({ RES_FILTER: null }),
            };
            if let Some(fields) = entry.as_object_mut() {
                fields.insert(RES_PATH.to_string(), json!(p));
            }
            entry
        })
        .collect()
}

fn asdf_layers(extensions: &[HduInfo], file_name: &str) -> Vec<(LayerGuess, Option<String>)> {
    extensions
        .iter()
        .map(|ext| {
            let mut named = HduHeader::empty();
            named.set("EXTNAME", ext.extname.clone().unwrap_or_default());
            (classify_layer(&named, file_name), None)
        })
        .collect()
}

fn listed_band_planes(source: &str) -> Vec<HduBandPlanes> {
    list_band_planes(source).unwrap_or_else(|error| {
        log::warn!("the band planes of {source} could not be listed: {error:#}");
        Vec::new()
    })
}

fn fits_layers(source: &str, extensions: &[HduInfo], file_name: &str) -> anyhow::Result<Vec<(LayerGuess, Option<String>)>> {
    let (readable, _tmp) = resolve_single_image(source)?;
    let file = File::open(&readable)?;
    extensions
        .iter()
        .map(|ext| {
            let header = extract_header_by_index(&file, ext.index)?;
            let layer = if ext.has_data {
                classify_layer(&header, file_name)
            } else {
                LayerGuess { role: LayerRole::Other, basis: NO_IMAGE_DATA.to_string(), assumed: false }
            };
            Ok((layer, data_unit(&header)))
        })
        .collect()
}

pub(crate) fn extensions_json(path: &str) -> anyhow::Result<serde_json::Value> {
    let source = source_path(path);
    let (extensions, is_asdf) = list_planes(&source)?;
    let file_name = Path::new(&source).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let (layers, band_planes) = if is_asdf {
        (asdf_layers(&extensions, file_name), Vec::new())
    } else {
        (fits_layers(&source, &extensions, file_name)?, listed_band_planes(&source))
    };
    let ext_json: Vec<serde_json::Value> = extensions
        .iter()
        .zip(&layers)
        .map(|(ext, (layer, bunit))| {
            let name = ext.extname.as_deref().unwrap_or("");
            let bands = band_planes.iter().find(|listed| listed.hdu == ext.index).map_or(&[][..], |listed| &listed.bands[..]);
            json!({
                RES_INDEX: ext.index,
                RES_EXTNAME: ext.extname,
                RES_EXTVER: ext.extver,
                RES_NAXIS: ext.naxis,
                RES_NAXIS1: ext.naxis1,
                RES_NAXIS2: ext.naxis2,
                RES_NAXIS3: ext.naxis3,
                RES_NAXIS4: ext.naxis4,
                RES_BITPIX: ext.bitpix,
                RES_HAS_DATA: ext.has_data,
                RES_REF: plane_ref(&source, ext, is_asdf).cache_key(),
                RES_KIND: if is_asdf { PLANE_KIND_ARRAY } else { PLANE_KIND_HDU },
                RES_IS_DQ: is_dq_name(name),
                RES_IS_ERR: is_err_name(name),
                RES_ROLE: layer.role,
                RES_ROLE_BASIS: layer.basis,
                RES_ROLE_ASSUMED: layer.assumed,
                RES_BUNIT: bunit,
                RES_BANDS: bands,
            })
        })
        .collect();
    Ok(json!({ RES_EXTENSIONS: ext_json }))
}

#[tauri::command]
pub async fn get_header(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let header = header_for(&path)?;
        Ok(serde_json::to_value(&header.index)?)
    })
}

#[tauri::command]
pub async fn get_full_header(path: String, palette: Option<String>) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let palette_type = palette_from(palette.as_deref());
        let header = header_for(&path)?;

        let file_name = std::path::Path::new(&source_path(&path))
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let cards_json: Vec<serde_json::Value> = header.cards.iter().map(|(k, v)| {
            json!({RES_KEY: k, RES_VALUE: v})
        }).collect();

        let wcs_keys = ["CRPIX1","CRPIX2","CRVAL1","CRVAL2","CDELT1","CDELT2",
            "CD1_1","CD1_2","CD2_1","CD2_2","CTYPE1","CTYPE2","LONPOLE","LATPOLE",
            "RADESYS","EQUINOX","WCSAXES","A_ORDER","B_ORDER","SIPMXERR","SIPIVERR"];
        let obs_keys = ["DATE-OBS","MJD-OBS","EXPTIME","EXPOSURE","OBJECT","OBSERVER",
            "TELESCOP","INSTRUME","FILTER","FILTER1","FILTER2","AIRMASS","RA","DEC",
            "EPOCH","GAIN","OFFSET","CCD-TEMP","SET-TEMP"];
        let image_keys = ["NAXIS","NAXIS1","NAXIS2","NAXIS3","BITPIX","BSCALE","BZERO",
            "DATAMIN","DATAMAX","BLANK"];
        let proc_keys = ["SWCREATE","SOFTWARE","HISTORY","COMMENT","PROGRAM","CREATOR",
            "ORIGIN","PIPELINE"];
        let instrument_keys = ["DETECTOR","MODULE","CHANNEL","BAND","GRATING","PUPIL","PILIN",
            "FOCUSPOS","LAMP","OPMODE","FPE_SIDE","ICE_SIDE","CCCSTATE","FWPOSOFF","FWPOSTOL",
            "PWPOSOFF","PWPOSTOL","EXP_TYPE","READPATT","SUBARRAY","APERNAME"];

        let mut categories: std::collections::HashMap<String, std::collections::HashMap<String, String>> = std::collections::HashMap::new();
        for cat in [
            CATEGORY_OBSERVATION, CATEGORY_INSTRUMENT, CATEGORY_IMAGE,
            CATEGORY_WCS, CATEGORY_PROCESSING, CATEGORY_OTHER,
        ] {
            categories.insert(cat.into(), std::collections::HashMap::new());
        }

        for (key, val) in &header.cards {
            let ku = key.to_uppercase();
            if ku == "SIMPLE" || ku == "END" || ku == "EXTEND" { continue; }
            let cat = if wcs_keys.iter().any(|&k| ku == k || ku.starts_with("A_") || ku.starts_with("B_") || ku.starts_with("AP_") || ku.starts_with("BP_")) {
                CATEGORY_WCS
            } else if obs_keys.iter().any(|&k| ku == k) {
                CATEGORY_OBSERVATION
            } else if image_keys.iter().any(|&k| ku == k) {
                CATEGORY_IMAGE
            } else if proc_keys.iter().any(|&k| ku == k || ku.starts_with("HISTORY") || ku.starts_with("COMMENT")) {
                CATEGORY_PROCESSING
            } else if instrument_keys.iter().any(|&k| ku == k) || ku.starts_with("GWA_") || ku.starts_with("TELESCOP") || ku.starts_with("INSTRUME") || ku.starts_with("CAMERA") || ku.starts_with("CCD") || ku.starts_with("SENSOR") {
                CATEGORY_INSTRUMENT
            } else {
                CATEGORY_OTHER
            };
            if let Some(c) = categories.get_mut(cat) {
                c.insert(key.clone(), val.clone());
            }
        }

        let filter_json = detect_filter(&header).map(|f| detection_json(&f, &palette_type));

        let palette = suggest_palette_with_type(&[(path.clone(), header.clone())], &palette_type);
        let filename_hint: Option<String> = if palette.is_complete {
            Some(palette.palette_name.clone())
        } else {
            None
        };

        Ok(json!({
            RES_FILE_NAME: file_name,
            RES_FILE_PATH: path,
            RES_TOTAL_CARDS: header.cards.len(),
            RES_CARDS: cards_json,
            RES_CATEGORIES: categories,
            RES_FILTER_DETECTION: filter_json,
            RES_FILENAME_HINT: filename_hint,
        }))
    })
}

#[tauri::command]
pub async fn get_fits_extensions(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!(extensions_json(&path))
}

#[tauri::command]
pub async fn get_header_by_hdu(path: String, hdu_index: usize) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (fits_path, _tmp) = resolve_single_image(&source_path(&path))?;
        if is_asdf_file(&fits_path) {
            anyhow::bail!("ASDF arrays have no HDU index");
        }
        let file = File::open(&fits_path)?;
        let header = crate::infra::fits::reader::extract_header_by_index(&file, hdu_index)?;
        Ok(serde_json::to_value(&header)?)
    })
}

#[tauri::command]
pub async fn detect_narrowband_filters(paths: Vec<String>, palette: Option<String>) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let palette_type = palette_from(palette.as_deref());

        let mut file_headers: Vec<(String, HduHeader)> = Vec::new();

        for p in &paths {
            let header = load_plane_header(&image_ref(p))?;
            file_headers.push((p.clone(), header));
        }

        let suggestion = suggest_palette_with_type(&file_headers, &palette_type);

        Ok(json!({
            RES_FILTERS: narrowband_detections_json(&file_headers, &palette_type),
            RES_PALETTE: serde_json::to_value(&suggestion).unwrap_or(json!(null)),
        }))
    })
}

#[cfg(test)]
mod tests {
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
}
