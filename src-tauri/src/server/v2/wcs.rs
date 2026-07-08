// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::{angular_separation, WcsTransform};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

#[derive(Deserialize)]
pub struct Pix2SkyParams {
    pub points: Vec<[f64; 2]>,
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
}

#[derive(Deserialize)]
pub struct Sky2PixParams {
    pub points: Vec<[f64; 2]>,
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SeparationParams {
    Sky { a: [f64; 2], b: [f64; 2] },
    Pixel {
        a: [f64; 2],
        b: [f64; 2],
        #[serde(default, alias = "ref")]
        image_ref: Option<String>,
    },
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

fn load_wcs(session: &Session, image_ref: &str) -> Result<(WcsTransform, usize, usize)> {
    let entry = session
        .cache
        .get(image_ref)
        .ok_or_else(|| AppError::NotFound(format!("image ref {image_ref} not found in session")))?;
    let (rows, cols) = entry.arr().dim();
    let header = entry.header().ok_or_else(|| AppError::BadRequestWithHint {
        code: "wcs_required",
        message: format!("image {image_ref} has no header, so no WCS is available"),
        hint: Some("open an image whose header carries WCS keywords".into()),
    })?;
    let wcs = WcsTransform::from_header(header).map_err(|e| AppError::BadRequestWithHint {
        code: "wcs_required",
        message: format!("image {image_ref} has no usable WCS: {e}"),
        hint: Some("the header must carry a valid WCS (CTYPE/CRPIX/CRVAL/CD...)".into()),
    })?;
    Ok((wcs, cols, rows))
}

fn on_image(x: f64, y: f64, w: usize, h: usize) -> bool {
    x >= 0.0 && x < w as f64 && y >= 0.0 && y < h as f64
}

fn separation_body(deg: f64) -> Value {
    json!({
        "separation_deg": deg,
        "separation_arcmin": deg * 60.0,
        "separation_arcsec": deg * 3600.0,
    })
}

pub async fn pix2sky(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<Pix2SkyParams>,
) -> Result<Json<Value>> {
    let target = target_ref(&session, params.image_ref).await?;
    let (wcs, w, h) = load_wcs(&session, &target)?;

    let coords: Vec<(f64, f64)> = params.points.iter().map(|p| (p[0], p[1])).collect();
    let sky = wcs.pixel_to_world_batch(&coords);

    let results: Vec<Value> = coords
        .iter()
        .zip(sky.iter())
        .map(|(&(x, y), c)| {
            json!({
                "x": x,
                "y": y,
                "ra": c.ra,
                "dec": c.dec,
                "on_image": on_image(x, y, w, h),
            })
        })
        .collect();

    Ok(Json(json!({
        "ref": target,
        "count": results.len(),
        "results": results,
    })))
}

pub async fn sky2pix(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<Sky2PixParams>,
) -> Result<Json<Value>> {
    let target = target_ref(&session, params.image_ref).await?;
    let (wcs, w, h) = load_wcs(&session, &target)?;

    let coords: Vec<(f64, f64)> = params.points.iter().map(|p| (p[0], p[1])).collect();
    let pixels = wcs.world_to_pixel_batch(&coords);

    let results: Vec<Value> = coords
        .iter()
        .zip(pixels.iter())
        .map(|(&(ra, dec), &(x, y))| {
            json!({
                "ra": ra,
                "dec": dec,
                "x": x,
                "y": y,
                "on_image": on_image(x, y, w, h),
            })
        })
        .collect();

    Ok(Json(json!({
        "ref": target,
        "count": results.len(),
        "results": results,
    })))
}

pub async fn separation(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<SeparationParams>,
) -> Result<Json<Value>> {
    match params {
        SeparationParams::Sky { a, b } => {
            let deg = angular_separation(a[0], a[1], b[0], b[1]);
            Ok(Json(separation_body(deg)))
        }
        SeparationParams::Pixel { a, b, image_ref } => {
            let target = target_ref(&session, image_ref).await?;
            let (wcs, _, _) = load_wcs(&session, &target)?;
            let ca = wcs.pixel_to_world(a[0], a[1]);
            let cb = wcs.pixel_to_world(b[0], b[1]);
            if !(ca.ra.is_finite() && ca.dec.is_finite() && cb.ra.is_finite() && cb.dec.is_finite())
            {
                return Err(AppError::BadRequestWithHint {
                    code: "region_out_of_bounds",
                    message: "one of the pixel points does not project onto the sky".into(),
                    hint: Some("check the pixel coordinates lie within the image".into()),
                });
            }
            let deg = angular_separation(ca.ra, ca.dec, cb.ra, cb.dec);
            let mut body = separation_body(deg);
            body["ref"] = json!(target);
            body["a_sky"] = json!([ca.ra, ca.dec]);
            body["b_sky"] = json!([cb.ra, cb.dec]);
            Ok(Json(body))
        }
    }
}
