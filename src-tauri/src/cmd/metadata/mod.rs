use std::fs::File;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, image_ref, source_path};
use crate::core::metadata::header_discovery::{detect_filter, suggest_palette, suggest_palette_with_type, PaletteType};
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
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
    if let Some(entry) = GLOBAL_IMAGE_CACHE.get(path) {
        if let Some(header) = entry.header() {
            return Ok(header.clone());
        }
    }
    if let Ok(entry) = GLOBAL_IMAGE_CACHE.upgrade_header(path, || load_plane_header(&image_ref(path))) {
        if let Some(header) = entry.header() {
            return Ok(header.clone());
        }
    }
    load_plane_header(&image_ref(path))
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
pub async fn get_full_header(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
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

        let filter = detect_filter(&header);
        let filter_json = filter.map(|f| json!({
            RES_FILTER: f.filter,
            RES_FILTER_ID: format!("{:?}", f.filter),
            RES_HUBBLE_CHANNEL: f.hubble_channel,
            RES_CONFIDENCE: f.confidence,
            RES_MATCHED_KEYWORD: f.matched_keyword,
            RES_MATCHED_VALUE: f.matched_value,
        }));

        let palette = suggest_palette(&[(path.clone(), header.clone())]);
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
        let palette_type = palette
            .as_deref()
            .map(PaletteType::from_str_loose)
            .unwrap_or_default();

        let mut file_headers: Vec<(String, crate::types::header::HduHeader)> = Vec::new();

        for p in &paths {
            let header = load_plane_header(&image_ref(p))?;
            file_headers.push((p.clone(), header));
        }

        let mut filters = Vec::new();
        for (p, header) in &file_headers {
            if let Some(filter) = detect_filter(header) {
                filters.push(json!({
                    RES_PATH: p,
                    RES_FILTER: filter.filter,
                    RES_HUBBLE_CHANNEL: filter.hubble_channel,
                    RES_CONFIDENCE: filter.confidence,
                    RES_MATCHED_KEYWORD: filter.matched_keyword,
                    RES_MATCHED_VALUE: filter.matched_value,
                }));
            } else {
                filters.push(json!({
                    RES_PATH: p,
                    RES_FILTER: null,
                }));
            }
        }

        let suggestion = suggest_palette_with_type(&file_headers, &palette_type);

        Ok(json!({
            RES_FILTERS: filters,
            RES_PALETTE: serde_json::to_value(&suggestion).unwrap_or(json!(null)),
        }))
    })
}
