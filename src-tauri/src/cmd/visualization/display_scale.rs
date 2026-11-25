use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, load_cached};
use crate::cmd::processing::is_display_referred;
use crate::core::imaging::colormap::Colormap;
use crate::core::imaging::scale::{
    resolve_limits, symmetric_about, validate_symmetric_centre, LimitMode, DEFAULT_SYMMETRIC_CENTRE,
    SYMMETRIC_FALLBACK_NOTE,
};
use crate::types::constants::{
    RES_ALGORITHM, RES_CENTRE, RES_COLORMAPS, RES_NAME, RES_NODATA, RES_NOTES, RES_RGBA, RES_SYMMETRIC,
    RES_VMAX, RES_VMIN,
};
use crate::types::header::HduHeader;

const SYMMETRIC_SKIPPED_DISPLAY_REFERRED_NOTE: &str =
    "symmetric limits skipped: the image is display-referred (ABDISP=T)";

fn display_limits(data: &ndarray::Array2<f32>, header: Option<&HduHeader>, mode: LimitMode) -> (f64, f64) {
    match mode {
        LimitMode::User { .. } => resolve_limits(data, mode),
        _ if is_display_referred(header) => (0.0, 1.0),
        _ => resolve_limits(data, mode),
    }
}

pub(crate) fn scale_limits_json(
    path: &str,
    mode: LimitMode,
    symmetric: bool,
    centre: Option<f64>,
) -> anyhow::Result<serde_json::Value> {
    let centre = centre.unwrap_or(DEFAULT_SYMMETRIC_CENTRE);
    if symmetric {
        validate_symmetric_centre(centre).map_err(|e| anyhow::anyhow!(e))?;
    }
    let cached = load_cached(path)?;
    let header = cached_header(path).ok();
    let (mut lo, mut hi) = display_limits(cached.arr(), header.as_ref(), mode);
    let mut notes: Vec<&str> = Vec::new();
    let mut applied = false;
    if symmetric && is_display_referred(header.as_ref()) {
        notes.push(SYMMETRIC_SKIPPED_DISPLAY_REFERRED_NOTE);
    } else if symmetric {
        let s = symmetric_about(centre, lo, hi);
        (lo, hi) = (s.vmin, s.vmax);
        applied = true;
        if s.fallback {
            notes.push(SYMMETRIC_FALLBACK_NOTE);
        }
    }
    Ok(json!({
        RES_VMIN: lo,
        RES_VMAX: hi,
        RES_ALGORITHM: mode.name(),
        RES_SYMMETRIC: applied,
        RES_CENTRE: applied.then_some(centre),
        RES_NOTES: notes,
    }))
}

#[tauri::command]
pub async fn compute_scale_limits_cmd(
    path: String,
    algorithm: String,
    vmin: Option<f64>,
    vmax: Option<f64>,
    percentile: Option<[f64; 2]>,
    zscale_contrast: Option<f64>,
    symmetric: Option<bool>,
    centre: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let mode = LimitMode::from_parts(&algorithm, vmin, vmax, percentile, zscale_contrast)
            .map_err(|e| anyhow::anyhow!(e))?;
        scale_limits_json(&path, mode, symmetric.unwrap_or(false), centre)
    })
}

#[tauri::command]
pub async fn get_colormap_lut_cmd(name: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let cmap = Colormap::from_name(&name).map_err(|e| anyhow::anyhow!(e))?;
        let names: Vec<&str> = Colormap::ALL.iter().map(|c| c.name()).collect();
        Ok(json!({
            RES_NAME: cmap.name(),
            RES_RGBA: cmap.lut_rgba().to_vec(),
            RES_NODATA: cmap.nodata_rgb().to_vec(),
            RES_COLORMAPS: names,
        }))
    })
}
