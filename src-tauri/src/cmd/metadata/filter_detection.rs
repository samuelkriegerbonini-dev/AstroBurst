use serde_json::json;

use crate::cmd::common::{blocking_cmd, image_ref};
use crate::core::metadata::header_discovery::{
    detect_filter, palette_channel, suggest_palette_with_type, FilterDetection, PaletteType,
};
use crate::infra::image_source::load_plane_header;
use crate::types::constants::{
    RES_CONFIDENCE, RES_FILTER, RES_FILTER_ID, RES_FILTERS, RES_HUBBLE_CHANNEL, RES_MATCHED_KEYWORD,
    RES_MATCHED_VALUE, RES_PALETTE, RES_PATH,
};
use crate::types::header::HduHeader;

pub(super) fn palette_from(palette: Option<&str>) -> PaletteType {
    palette.map(PaletteType::from_str_loose).unwrap_or_default()
}

pub(super) fn detection_json(detection: &FilterDetection, palette: &PaletteType) -> serde_json::Value {
    json!({
        RES_FILTER: detection.filter,
        RES_FILTER_ID: format!("{:?}", detection.filter),
        RES_HUBBLE_CHANNEL: palette_channel(palette, detection.filter),
        RES_CONFIDENCE: detection.confidence,
        RES_MATCHED_KEYWORD: detection.matched_keyword,
        RES_MATCHED_VALUE: detection.matched_value,
    })
}

pub(super) fn narrowband_detections_json(file_headers: &[(String, HduHeader)], palette: &PaletteType) -> Vec<serde_json::Value> {
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
