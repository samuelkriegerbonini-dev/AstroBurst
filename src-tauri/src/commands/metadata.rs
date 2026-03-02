use std::path::Path;

use anyhow::Result;

use crate::domain::header_discovery;
use crate::model::HduHeader;

use super::helpers::*;

#[tauri::command]
pub async fn get_header(path: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (result, _tmp) = extract_image_full(&path)?;

        let keys = [
            "TELESCOP", "INSTRUME", "DETECTOR", "CHANNEL", "FILTER", "TARGNAME", "DATE-OBS",
            "EXPTIME", "EFFEXPTM", "NAXIS", "NAXIS1", "NAXIS2", "NAXIS3", "BITPIX", "BUNIT",
            "RA_TARG", "DEC_TARG", "CRVAL1", "CRVAL2", "RADESYS",
        ];

        let mut map = serde_json::Map::new();
        for key in keys {
            if let Some(val) = result.header.get(key) {
                map.insert(key.to_string(), serde_json::Value::String(val.to_string()));
            }
        }

        map.insert("_is_mef".to_string(), serde_json::Value::Bool(result.is_mef));
        map.insert("_extension_count".to_string(), serde_json::json!(result.extension_count));
        if let Some(ref ext) = result.selected_extension {
            map.insert("_selected_extension".to_string(), serde_json::Value::String(ext.clone()));
        }

        Ok(serde_json::Value::Object(map))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_full_header(path: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (result, _tmp) = extract_image_full(&path)?;
        let header = &result.header;

        let cards: Vec<serde_json::Value> = header
            .cards
            .iter()
            .map(|(k, v)| serde_json::json!({ "key": k, "value": v }))
            .collect();

        let filter_detection = header_discovery::detect_filter(header);

        let file_name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let detection_json = match &filter_detection {
            Some(det) => serde_json::json!({
                "filter": format!("{}", det.filter),
                "filter_id": format!("{:?}", det.filter),
                "hubble_channel": format!("{}", det.hubble_channel),
                "confidence": format!("{:?}", det.confidence),
                "matched_keyword": det.matched_keyword,
                "matched_value": det.matched_value,
            }),
            None => serde_json::Value::Null,
        };

        let filename_detection = filter_detection
            .is_none()
            .then(|| {
                let upper = file_name.to_uppercase();
                if upper.contains("_HA") || upper.contains("-HA") || upper.contains("HALPHA") {
                    Some("Hα → G (Hubble)")
                } else if upper.contains("_OIII")
                    || upper.contains("-OIII")
                    || upper.contains("_O3")
                {
                    Some("[OIII] → B (Hubble)")
                } else if upper.contains("_SII")
                    || upper.contains("-SII")
                    || upper.contains("_S2")
                {
                    Some("[SII] → R (Hubble)")
                } else if upper.contains("_R.") || upper.contains("_RED") {
                    Some("R (Broadband)")
                } else if upper.contains("_G.") || upper.contains("_GREEN") {
                    Some("G (Broadband)")
                } else if upper.contains("_B.") || upper.contains("_BLUE") {
                    Some("B (Broadband)")
                } else if upper.contains("_L.") || upper.contains("_LUM") {
                    Some("Luminance")
                } else {
                    None
                }
            })
            .flatten();

        let categories = categorize_header_cards(header);

        let extensions_json: Vec<serde_json::Value> = result
            .extensions
            .iter()
            .map(|ext| {
                serde_json::json!({
                    "index": ext.index,
                    "extname": ext.extname,
                    "extver": ext.extver,
                    "naxis": ext.naxis,
                    "naxis1": ext.naxis1,
                    "naxis2": ext.naxis2,
                    "bitpix": ext.bitpix,
                    "has_data": ext.has_data,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "file_name": file_name,
            "file_path": path,
            "total_cards": cards.len(),
            "cards": cards,
            "categories": categories,
            "filter_detection": detection_json,
            "filename_hint": filename_detection,
            "is_mef": result.is_mef,
            "extension_count": result.extension_count,
            "selected_extension": result.selected_extension,
            "extensions": extensions_json,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_fits_extensions(path: String) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (extensions, _tmp) = list_fits_extensions(&path)?;

        let ext_json: Vec<serde_json::Value> = extensions
            .iter()
            .map(|ext| {
                serde_json::json!({
                    "index": ext.index,
                    "extname": ext.extname,
                    "extver": ext.extver,
                    "naxis": ext.naxis,
                    "naxis1": ext.naxis1,
                    "naxis2": ext.naxis2,
                    "naxis3": ext.naxis3,
                    "bitpix": ext.bitpix,
                    "has_data": ext.has_data,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "extension_count": extensions.len(),
            "extensions": ext_json,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn get_header_by_hdu(
    path: String,
    hdu_index: usize,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let (result, _tmp) = extract_image_by_hdu(&path, hdu_index)?;

        let cards: Vec<serde_json::Value> = result
            .header
            .cards
            .iter()
            .map(|(k, v)| serde_json::json!({ "key": k, "value": v }))
            .collect();

        let categories = categorize_header_cards(&result.header);

        Ok(serde_json::json!({
            "hdu_index": hdu_index,
            "selected_extension": result.selected_extension,
            "total_cards": cards.len(),
            "cards": cards,
            "categories": categories,
        }))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

#[tauri::command]
pub async fn detect_narrowband_filters(
    paths: Vec<String>,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let mut files: Vec<(String, HduHeader)> = Vec::new();
        for path in &paths {
            let (header, _, _tmp) = extract_image_resolved(path)?;
            files.push((path.clone(), header));
        }
        let palette = header_discovery::suggest_palette(&files);
        Ok(serde_json::json!(palette))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
    .map_err(map_anyhow)
}

fn categorize_header_cards(header: &HduHeader) -> serde_json::Value {
    let mut observation = serde_json::Map::new();
    let mut instrument = serde_json::Map::new();
    let mut image = serde_json::Map::new();
    let mut wcs = serde_json::Map::new();
    let mut processing = serde_json::Map::new();
    let mut other = serde_json::Map::new();

    let obs_keys = [
        "OBJECT", "TARGNAME", "DATE-OBS", "DATE-END", "EXPTIME", "EFFEXPTM", "RA_TARG",
        "DEC_TARG", "RA", "DEC", "AIRMASS", "HA", "OBSERVER", "PROGRAM", "PI_NAME", "EQUINOX",
        "EPOCH",
    ];
    let inst_keys = [
        "TELESCOP", "INSTRUME", "DETECTOR", "CHANNEL", "FILTER", "FILTER1", "FILTER2", "FILTER3",
        "FILTNAM", "APERTURE", "GRATING", "CAMERA", "GAIN", "RDNOISE", "CCD-TEMP", "SET-TEMP",
        "FOCALLEN",
    ];
    let img_keys = [
        "NAXIS", "NAXIS1", "NAXIS2", "NAXIS3", "BITPIX", "BUNIT", "BSCALE", "BZERO", "DATAMIN",
        "DATAMAX", "BLANK",
    ];
    let wcs_keys = [
        "CRVAL1", "CRVAL2", "CRPIX1", "CRPIX2", "CDELT1", "CDELT2", "CD1_1", "CD1_2", "CD2_1",
        "CD2_2", "CTYPE1", "CTYPE2", "RADESYS", "LONPOLE", "LATPOLE",
    ];
    let proc_keys = [
        "HISTORY", "COMMENT", "SOFTWARE", "SWCREATE", "PROCVER", "CALIBVER", "FLATFILE",
        "DARKFILE", "BIASFILE",
    ];

    for (key, value) in &header.cards {
        let k = key.to_uppercase();
        let val = serde_json::Value::String(value.clone());
        if obs_keys.iter().any(|&ok| k == ok) {
            observation.insert(key.clone(), val);
        } else if inst_keys.iter().any(|&ik| k == ik) {
            instrument.insert(key.clone(), val);
        } else if img_keys.iter().any(|&ik| k == ik) {
            image.insert(key.clone(), val);
        } else if wcs_keys.iter().any(|&wk| k == wk) || k.starts_with("CD") || k.starts_with("PC")
        {
            wcs.insert(key.clone(), val);
        } else if proc_keys.iter().any(|&pk| k == pk)
            || k == "SIMPLE"
            || k == "EXTEND"
            || k == "END"
        {
            processing.insert(key.clone(), val);
        } else {
            other.insert(key.clone(), val);
        }
    }

    serde_json::json!({
        "observation": observation,
        "instrument": instrument,
        "image": image,
        "wcs": wcs,
        "processing": processing,
        "other": other,
    })
}
