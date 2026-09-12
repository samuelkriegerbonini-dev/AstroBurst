// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::pixel_probe::{
    data_unit, probe_companions, probe_json_with_companions, probe_pixel, CompanionProbe,
    ProbeError,
};
use astroburst_lib::infra::cache::ImageEntry;
use astroburst_lib::infra::image_source::{load_companions_into, LoadedCompanions};

use super::images::load_ref;
use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

fn load_session_companions(session: &Session, entry: &ImageEntry) -> LoadedCompanions {
    load_companions_into(&session.cache, entry, load_ref)
}

fn default_box() -> u32 {
    1
}

#[derive(Deserialize)]
pub struct PixelParams {
    pub x: f64,
    pub y: f64,
    #[serde(default = "default_box", rename = "box")]
    pub box_size: u32,
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
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

fn probe_error(err: ProbeError) -> AppError {
    match err {
        ProbeError::OutOfBounds { cols, rows, .. } => AppError::BadRequestWithHint {
            code: "pixel_out_of_bounds",
            message: err.to_string(),
            hint: Some(format!("x must be in [0, {cols}) and y in [0, {rows})")),
        },
        ProbeError::EvenBox(_) => AppError::BadRequestWithHint {
            code: "bad_request",
            message: err.to_string(),
            hint: Some("use 1, 3, 5, ... so the box x box window is centred on the pixel".into()),
        },
    }
}

pub async fn pixel(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<PixelParams>,
) -> Result<Json<Value>> {
    let target = target_ref(&session, params.image_ref).await?;

    let entry = session
        .cache
        .get(&target)
        .ok_or_else(|| AppError::NotFound(format!("image ref {target} not found in session")))?;

    let probe = probe_pixel(entry.arr(), params.x, params.y, params.box_size).map_err(probe_error)?;
    let unit = entry.header().and_then(data_unit);

    let sky = entry
        .header()
        .and_then(|h| WcsTransform::from_header(h).ok())
        .map(|wcs| {
            let c = wcs.pixel_to_world(params.x, params.y);
            json!({ "ra": c.ra, "dec": c.dec })
        })
        .unwrap_or(Value::Null);

    let (companions, err_unit) = if entry.companions().is_some() {
        let sess = session.clone();
        let active = entry.clone();
        let (px, py) = (probe.x, probe.y);
        tokio::task::spawn_blocking(move || {
            let comps = load_session_companions(&sess, &active);
            let dq = comps
                .dq
                .as_ref()
                .and_then(|(e, table)| e.int_plane().map(|p| (p, *table)));
            let err_unit = comps.err.as_ref().and_then(|e| e.header().and_then(data_unit));
            let probe = probe_companions(dq, comps.err.as_ref().map(|e| e.arr()), px, py);
            (probe, err_unit)
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    } else {
        (CompanionProbe { dq: None, err: None }, None)
    };

    let mut body = probe_json_with_companions(&probe, unit.as_deref(), err_unit.as_deref(), &companions);
    body["ref"] = json!(target);
    body["sky"] = sky;
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(e: AppError) -> (&'static str, String, Option<String>) {
        match e {
            AppError::BadRequestWithHint { code, message, hint } => (code, message, hint),
            other => panic!("expected BadRequestWithHint, got {other:?}"),
        }
    }

    #[test]
    fn out_of_bounds_maps_to_pixel_out_of_bounds_with_extent_hint() {
        let err = ProbeError::OutOfBounds { x: 100.0, y: 100.0, cols: 8, rows: 8 };
        let (code, message, hint) = parts(probe_error(err));
        assert_eq!(code, "pixel_out_of_bounds");
        assert!(message.contains("(100, 100)"), "{message}");
        assert_eq!(hint.as_deref(), Some("x must be in [0, 8) and y in [0, 8)"));
    }

    #[test]
    fn even_box_maps_to_bad_request() {
        let (code, message, hint) = parts(probe_error(ProbeError::EvenBox(4)));
        assert_eq!(code, "bad_request");
        assert!(message.contains("got 4"), "{message}");
        assert!(hint.unwrap().contains("1, 3, 5"));
    }
}
