use std::sync::Arc;

use axum::extract::Query;
use axum::{Extension, Json};
use serde_json::{json, Value};

use astroburst_lib::infra::fits::reader::HduInfo;
use astroburst_lib::infra::image_source::{is_dq_name, is_err_name, list_planes, plane_ref};

use super::error::ScienceError;
use super::resolve::{require_file_source, resolve_ref};
use super::state::ScienceState;
use crate::error::AppError;
use crate::extractors::SessionExtractor;
use crate::v2::images::plane_load_error;
use crate::v2::inspect::RefQuery;

const MAX_LISTED_AXES: usize = 4;

pub(crate) async fn planes(
    SessionExtractor(session): SessionExtractor,
    Extension(science): Extension<Arc<ScienceState>>,
    Query(query): Query<RefQuery>,
) -> Result<Json<Value>, ScienceError> {
    let resolved = resolve_ref(&session, query.r#ref).await?;
    let source = require_file_source(&science, &resolved).await?.path;
    let listed = source.clone();
    let (planes, is_asdf) = tokio::task::spawn_blocking(move || list_planes(&listed))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
        .map_err(|e| plane_load_error(e, &source))?;
    Ok(Json(json!({
        "ref": resolved.name,
        "format": if is_asdf { "asdf" } else { "fits" },
        "count": planes.len(),
        "planes": planes.iter().map(|info| plane_json(&source, info, is_asdf)).collect::<Vec<_>>(),
    })))
}

fn plane_json(source: &str, info: &HduInfo, is_asdf: bool) -> Value {
    let name = info.extname.as_deref().unwrap_or("");
    let axes = [info.naxis4, info.naxis3, info.naxis2, info.naxis1];
    let listed = info.naxis.clamp(0, MAX_LISTED_AXES as i64) as usize;
    json!({
        "index": info.index,
        "extname": info.extname,
        "extver": info.extver,
        "naxis": info.naxis,
        "shape": axes[MAX_LISTED_AXES - listed..],
        "bitpix": info.bitpix,
        "has_data": info.has_data,
        "plane_ref": plane_ref(source, info, is_asdf).cache_key(),
        "is_dq": is_dq_name(name),
        "is_err": is_err_name(name),
    })
}
