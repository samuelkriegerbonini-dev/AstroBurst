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

fn nearest_pixel_centre(v: f64) -> f64 {
    (v + 0.5).floor()
}

fn probe_error(err: ProbeError, x: f64, y: f64) -> AppError {
    match err {
        ProbeError::OutOfBounds { cols, rows, .. } => AppError::BadRequestWithHint {
            code: "pixel_out_of_bounds",
            message: ProbeError::OutOfBounds { x, y, cols, rows }.to_string(),
            hint: Some(format!(
                "pixel centres are integers: x must be in [-0.5, {}) and y in [-0.5, {})",
                cols as f64 - 0.5,
                rows as f64 - 0.5
            )),
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

    let probe = probe_pixel(
        entry.arr(),
        nearest_pixel_centre(params.x),
        nearest_pixel_centre(params.y),
        params.box_size,
    )
    .map_err(|e| probe_error(e, params.x, params.y))?;
    let unit = entry.header().and_then(data_unit);

    let sky = entry
        .header()
        .and_then(|h| WcsTransform::from_header(h).ok())
        .map(|wcs| {
            let c = wcs.pixel_to_world(probe.x as f64, probe.y as f64);
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
        let err = ProbeError::OutOfBounds { x: 100.0, y: 101.0, cols: 8, rows: 6 };
        let (code, message, hint) = parts(probe_error(err, 99.7, 100.6));
        assert_eq!(code, "pixel_out_of_bounds");
        assert!(message.contains("(99.7, 100.6)"), "{message}");
        assert_eq!(
            hint.as_deref(),
            Some("pixel centres are integers: x must be in [-0.5, 7.5) and y in [-0.5, 5.5)")
        );
    }

    #[test]
    fn coordinates_snap_to_the_pixel_whose_centre_is_nearest() {
        for (v, centre) in [(2.7, 3.0), (2.5, 3.0), (2.49, 2.0), (-0.5, 0.0), (-0.51, -1.0), (7.49, 7.0)] {
            assert_eq!(nearest_pixel_centre(v), centre, "{v}");
        }
    }

    #[test]
    fn even_box_maps_to_bad_request() {
        let (code, message, hint) = parts(probe_error(ProbeError::EvenBox(4), 3.0, 3.0));
        assert_eq!(code, "bad_request");
        assert!(message.contains("got 4"), "{message}");
        assert!(hint.unwrap().contains("1, 3, 5"));
    }
}
