use std::fs::File;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, image_ref, source_path};
use crate::core::metadata::header_discovery::{
    detect_filter, palette_channel, suggest_palette_with_type, FilterDetection, PaletteType,
};
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::image_source::{is_dq_name, is_err_name, list_planes, load_plane_header, plane_ref};
use crate::types::constants::{
    RES_BITPIX, RES_CARDS, RES_CATEGORIES, RES_CONFIDENCE, RES_EXTENSIONS,
    RES_EXTNAME, RES_EXTVER, RES_FILE_NAME, RES_FILE_PATH, RES_FILENAME_HINT, RES_FILTER,
    RES_FILTER_DETECTION, RES_FILTER_ID, RES_FILTERS, RES_HAS_DATA,
    RES_HUBBLE_CHANNEL, RES_INDEX, RES_IS_DQ, RES_IS_ERR, RES_KEY, RES_KIND,
    RES_MATCHED_KEYWORD, RES_MATCHED_VALUE,
    RES_NAXIS, RES_NAXIS1, RES_NAXIS2, RES_NAXIS3, RES_PALETTE, RES_PATH, RES_REF,
    RES_TOTAL_CARDS, RES_VALUE,
    CATEGORY_OBSERVATION, CATEGORY_INSTRUMENT, CATEGORY_IMAGE,
    CATEGORY_WCS, CATEGORY_PROCESSING, CATEGORY_OTHER,
    PLANE_KIND_ARRAY, PLANE_KIND_HDU,
};
use crate::types::header::HduHeader;

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

pub(crate) fn extensions_json(path: &str) -> anyhow::Result<serde_json::Value> {
    let source = source_path(path);
    let (extensions, is_asdf) = list_planes(&source)?;
    let ext_json: Vec<serde_json::Value> = extensions
        .iter()
        .map(|ext| {
            let name = ext.extname.as_deref().unwrap_or("");
            json!({
                RES_INDEX: ext.index,
                RES_EXTNAME: ext.extname,
                RES_EXTVER: ext.extver,
                RES_NAXIS: ext.naxis,
                RES_NAXIS1: ext.naxis1,
                RES_NAXIS2: ext.naxis2,
                RES_NAXIS3: ext.naxis3,
                RES_BITPIX: ext.bitpix,
                RES_HAS_DATA: ext.has_data,
                RES_REF: plane_ref(&source, ext, is_asdf).cache_key(),
                RES_KIND: if is_asdf { PLANE_KIND_ARRAY } else { PLANE_KIND_HDU },
                RES_IS_DQ: is_dq_name(name),
                RES_IS_ERR: is_err_name(name),
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
            "RADESYS","EQUINOX","WCSAXES","A_ORDER","B_ORDER"];
        let obs_keys = ["DATE-OBS","MJD-OBS","EXPTIME","EXPOSURE","OBJECT","OBSERVER",
            "TELESCOP","INSTRUME","FILTER","FILTER1","FILTER2","AIRMASS","RA","DEC",
            "EPOCH","GAIN","OFFSET","CCD-TEMP","SET-TEMP"];
        let image_keys = ["NAXIS","NAXIS1","NAXIS2","NAXIS3","BITPIX","BSCALE","BZERO",
            "DATAMIN","DATAMAX","BLANK"];
        let proc_keys = ["SWCREATE","SOFTWARE","HISTORY","COMMENT","PROGRAM","CREATOR",
            "ORIGIN","PIPELINE"];

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
            } else if ku.starts_with("TELESCOP") || ku.starts_with("INSTRUME") || ku.starts_with("CAMERA") || ku.starts_with("CCD") || ku.starts_with("SENSOR") {
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
}
