use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, source_path};
use crate::core::metadata::header_discovery::{detect_filter, suggest_palette_with_type};
use crate::types::constants::{
    RES_CARDS, RES_CATEGORIES, RES_FILE_NAME, RES_FILE_PATH, RES_FILENAME_HINT, RES_FILTER_DETECTION, RES_KEY,
    RES_TOTAL_CARDS, RES_VALUE, CATEGORY_OBSERVATION, CATEGORY_INSTRUMENT, CATEGORY_IMAGE, CATEGORY_WCS,
    CATEGORY_PROCESSING, CATEGORY_OTHER,
};
use crate::types::header::HduHeader;

use super::filter_detection::{detection_json, palette_from};

pub(super) fn header_for(path: &str) -> anyhow::Result<HduHeader> {
    cached_header(path)
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
