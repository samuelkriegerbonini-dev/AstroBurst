// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::fs::File;

use axum::{extract::Query, Json};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::infra::asdf::converter::is_asdf_file;
use astroburst_lib::infra::fits::dispatcher::resolve_single_image;
use astroburst_lib::infra::fits::reader::list_extensions;
use astroburst_lib::types::header::HduHeader;

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

#[derive(Deserialize)]
pub struct RefQuery {
    #[serde(default, alias = "image")]
    pub r#ref: Option<String>,
}

#[derive(Deserialize)]
pub struct HeaderQuery {
    #[serde(default, alias = "image")]
    pub r#ref: Option<String>,
    pub keys: Option<String>,
}

async fn target_ref(session: &Session, explicit: Option<String>) -> Result<String> {
    match explicit {
        Some(r) => Ok(r),
        None => session
            .v2
            .active_ref
            .read()
            .await
            .clone()
            .ok_or_else(|| {
                AppError::BadRequest("no active image in this session; open a file first".into())
            }),
    }
}

fn cached_header(session: &Session, image_ref: &str) -> Result<HduHeader> {
    let entry = session
        .cache
        .get(image_ref)
        .ok_or_else(|| AppError::NotFound(format!("image ref {image_ref} not found in session")))?;
    entry
        .header()
        .cloned()
        .ok_or_else(|| AppError::BadRequest(format!("image {image_ref} carries no header")))
}

fn dtype_for_bitpix(bitpix: i64) -> &'static str {
    match bitpix {
        8 => "uint8",
        16 => "int16",
        32 => "int32",
        64 => "int64",
        -32 => "float32",
        -64 => "float64",
        _ => "unknown",
    }
}

fn shape_for(naxis: i64, naxis1: i64, naxis2: i64, naxis3: i64) -> Vec<i64> {
    match naxis {
        0 => vec![],
        1 => vec![naxis1],
        2 => vec![naxis2, naxis1],
        _ => vec![naxis3, naxis2, naxis1],
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_ascii_uppercase().chars().collect();
    let t: Vec<char> = text.to_ascii_uppercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark): (Option<usize>, usize) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub async fn structure(
    SessionExtractor(session): SessionExtractor,
    Query(q): Query<RefQuery>,
) -> Result<Json<Value>> {
    let target = target_ref(&session, q.r#ref).await?;
    let source = session
        .v2
        .meta
        .get(&target)
        .and_then(|m| m.source.clone())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "ref {target} has no source file to inspect (derived product?)"
            ))
        })?;

    if is_asdf_file(std::path::Path::new(&source)) {
        return Err(AppError::BadRequest(
            "structure listing is FITS-only; this ref came from an ASDF file".into(),
        ));
    }

    let hdus = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Value>> {
        let (fits_path, _tmp) = resolve_single_image(&source)?;
        let file = File::open(&fits_path)?;
        let exts = list_extensions(&file)?;
        Ok(exts
            .iter()
            .map(|h| {
                json!({
                    "index": h.index,
                    "extname": h.extname,
                    "extver": h.extver,
                    "naxis": h.naxis,
                    "shape": shape_for(h.naxis, h.naxis1, h.naxis2, h.naxis3),
                    "bitpix": h.bitpix,
                    "dtype": dtype_for_bitpix(h.bitpix),
                    "has_data": h.has_data,
                })
            })
            .collect())
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    Ok(Json(json!({
        "ref": target,
        "count": hdus.len(),
        "hdus": hdus,
    })))
}

pub async fn header(
    SessionExtractor(session): SessionExtractor,
    Query(q): Query<HeaderQuery>,
) -> Result<Json<Value>> {
    let target = target_ref(&session, q.r#ref).await?;
    let hdr = cached_header(&session, &target)?;

    let ordered: Vec<(String, String)> = if !hdr.cards.is_empty() {
        hdr.cards.clone()
    } else {
        hdr.index.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    let patterns: Option<Vec<String>> = q.keys.as_ref().map(|s| {
        s.split(',')
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect()
    });

    let mut out = Map::new();
    let mut seen = std::collections::HashSet::new();
    for (k, v) in &ordered {
        let keep = match &patterns {
            None => true,
            Some(ps) => ps.iter().any(|p| glob_match(p, k)),
        };
        if keep && seen.insert(k.clone()) {
            out.insert(k.clone(), json!({ "value": v }));
        }
    }

    Ok(Json(json!({
        "ref": target,
        "count": out.len(),
        "cards": Value::Object(out),
    })))
}

pub async fn wcs(
    SessionExtractor(session): SessionExtractor,
    Query(q): Query<RefQuery>,
) -> Result<Json<Value>> {
    let target = target_ref(&session, q.r#ref).await?;

    let entry = session
        .cache
        .get(&target)
        .ok_or_else(|| AppError::NotFound(format!("image ref {target} not found in session")))?;

    let wcs = match entry.header().and_then(|h| WcsTransform::from_header(h).ok()) {
        Some(w) => w,
        None => {
            return Ok(Json(json!({ "ref": target, "present": false })));
        }
    };

    let (crpix1, crpix2, crval1, crval2, cd, proj) = wcs.raw_params();
    let (cd11, cd12, cd21, cd22) = (cd[0][0], cd[0][1], cd[1][0], cd[1][1]);

    let scale_x = (cd11 * cd11 + cd21 * cd21).sqrt() * 3600.0;
    let scale_y = (cd12 * cd12 + cd22 * cd22).sqrt() * 3600.0;

    let rotation_deg = (-cd12).atan2(cd22).to_degrees();

    let det = cd11 * cd22 - cd12 * cd21;
    let flipped = det > 0.0;

    let (sip_a, sip_b) = wcs.sip_forward_terms();
    let sip_present = sip_a.is_some() || sip_b.is_some();

    Ok(Json(json!({
        "ref": target,
        "present": true,
        "projection": proj,
        "crpix": [crpix1, crpix2],
        "crval": [crval1, crval2],
        "cd": [[cd11, cd12], [cd21, cd22]],
        "pixel_scale_arcsec": (scale_x + scale_y) / 2.0,
        "pixel_scale_x_arcsec": scale_x,
        "pixel_scale_y_arcsec": scale_y,
        "rotation_deg": rotation_deg,
        "flipped": flipped,
        "parity": if flipped { "flipped" } else { "normal" },
        "sip_present": sip_present,
    })))
}
