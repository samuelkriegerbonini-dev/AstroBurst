use anyhow::bail;
use serde_json::json;

use tauri::ipc::Response;

use crate::cmd::common::{blocking_cmd, load_cached_full, load_companions, ErrOriginJson};
use crate::core::imaging::dq_flags::{bits_present, mask_preview_or, DqTable, FlagSet};
use crate::infra::ipc::encode_mask_with_header;
use crate::types::constants::{
    DEFAULT_MASK_PREVIEW_DIM, RES_BIT, RES_BITS_PRESENT, RES_DEFAULT_MASK, RES_DQ_REF, RES_ERR_REF,
    RES_EXCLUSION_MASK, RES_FLAGS, RES_LABEL, RES_NAME, RES_SOURCE, RES_TABLE,
};
use crate::types::image_ref::ImageRef;

const RES_SATURATED_MASK: &str = "saturated_mask";
const RES_EXCLUSION_DEFAULT: &str = "exclusion_default";
const RES_EXCLUSION_EDITED: &str = "exclusion_edited";
const RES_DQ_ORIGIN: &str = "dq_origin";
pub(super) const RES_DQ_UNAVAILABLE: &str = "dq_unavailable";
pub(super) const RES_DQ_SOURCE: &str = "dq_source";
const RES_ERR_ORIGIN: &str = "err_origin";
pub(super) const RES_ERR_UNAVAILABLE: &str = "err_unavailable";
pub(super) const RES_ERR_SOURCE: &str = "err_source";

pub(crate) const NO_DQ_PLANE: &str = "No DQ plane available for this image";

pub(crate) fn resolve_overlay_mask(flags: &FlagSet, requested: Option<u32>) -> u32 {
    requested.unwrap_or(flags.overlay_mask)
}

pub(crate) struct DqFlagTable {
    pub(crate) set: FlagSet,
    pub(crate) bits_present: Option<u32>,
    pub(crate) dq_ref: Option<String>,
    pub(crate) err_ref: Option<String>,
    pub(crate) dq_origin: Option<String>,
    pub(crate) dq_unavailable: Option<String>,
    pub(crate) err_origin: Option<ErrOriginJson>,
    pub(crate) err_unavailable: Option<String>,
}

pub(crate) fn dq_table_for(path: &str) -> anyhow::Result<DqFlagTable> {
    let active = load_cached_full(path)?;
    let comps = active.companions().cloned().unwrap_or_default();
    let loaded = load_companions(path)?;
    let set = loaded.flags.clone().unwrap_or_else(|| match active.header() {
        Some(header) => FlagSet::from_headers(Some(header), Some(header), DqTable::select(header)),
        None => FlagSet::from_table(DqTable::Unknown),
    });
    let plane = loaded.dq.as_ref().and_then(|(entry, _)| entry.int_plane());
    let err_choice = active.plane_info().and_then(|info| info.pairing.err.as_ref());
    Ok(DqFlagTable {
        bits_present: plane.filter(|_| !set.is_named()).map(|plane| bits_present(&plane.bits)),
        dq_ref: loaded.dq.as_ref().and(comps.dq.as_ref()).map(ImageRef::cache_key),
        err_ref: comps.err.as_ref().map(ImageRef::cache_key),
        err_origin: loaded.err_origin.as_ref().zip(err_choice).map(|(origin, choice)| ErrOriginJson::loaded(origin, choice)),
        dq_origin: loaded.dq_origin,
        dq_unavailable: loaded.dq_unavailable,
        err_unavailable: loaded.err_unavailable,
        set,
    })
}

#[tauri::command]
pub async fn get_dq_flag_table_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let found = dq_table_for(&path)?;
        let set = &found.set;
        let flags: Vec<serde_json::Value> =
            set.flags.iter().map(|(bit, name)| json!({ RES_BIT: bit, RES_NAME: name })).collect();
        Ok(json!({
            RES_TABLE: set.table,
            RES_LABEL: set.source,
            RES_FLAGS: flags,
            RES_DEFAULT_MASK: set.overlay_mask,
            RES_EXCLUSION_MASK: set.exclusion_mask,
            RES_BITS_PRESENT: found.bits_present,
            RES_DQ_REF: found.dq_ref,
            RES_ERR_REF: found.err_ref,
            RES_SOURCE: set.source,
            RES_SATURATED_MASK: set.saturated_mask,
            RES_EXCLUSION_DEFAULT: set.exclusion_default,
            RES_EXCLUSION_EDITED: set.exclusion_mask != set.exclusion_default,
            RES_DQ_ORIGIN: found.dq_origin,
            RES_DQ_UNAVAILABLE: found.dq_unavailable,
            RES_ERR_ORIGIN: found.err_origin,
            RES_ERR_UNAVAILABLE: found.err_unavailable,
        }))
    })
}

#[tauri::command]
pub async fn get_dq_mask_preview(
    path: String,
    mask: Option<u32>,
    max_dim: Option<u32>,
) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Response> {
        let dim = max_dim.unwrap_or(DEFAULT_MASK_PREVIEW_DIM) as usize;
        let loaded = load_companions(&path)?;
        let (Some((entry, table)), Some(flags)) = (loaded.dq.as_ref(), loaded.flags.as_ref()) else {
            bail!(NO_DQ_PLANE);
        };
        let plane = entry.int_plane().ok_or_else(|| anyhow::anyhow!(NO_DQ_PLANE))?;
        let applied = resolve_overlay_mask(flags, mask);
        let (cells, width, height) = mask_preview_or(&plane.bits, applied, dim);
        Ok(Response::new(encode_mask_with_header(
            &cells,
            width as u32,
            height as u32,
            applied,
            table.id(),
        )))
    })
    .await
    .map_err(|e| format!("{}", e))?
    .map_err(|e| format!("{:#}", e))
}
