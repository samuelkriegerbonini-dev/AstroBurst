// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::frames::{convert_from_icrs, SkyFrame};
use astroburst_lib::core::astrometry::grid::{wcs_grid, DEFAULT_DENSITY, MAX_DENSITY, MIN_DENSITY};
use astroburst_lib::core::astrometry::wcs::{angular_separation, WcsTransform};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

#[derive(Deserialize)]
pub struct Pix2SkyParams {
    pub points: Vec<[f64; 2]>,
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
    #[serde(default)]
    pub frame: Option<String>,
}

fn parse_frame(name: Option<&str>) -> Result<SkyFrame> {
    SkyFrame::from_name(name.unwrap_or("icrs")).map_err(|message| AppError::BadRequestWithHint {
        code: "bad_request",
        message,
        hint: Some(format!(
            "supported frames: {}",
            SkyFrame::ALL.iter().map(|f| f.name()).collect::<Vec<_>>().join(", ")
        )),
    })
}

#[derive(Deserialize)]
pub struct Sky2PixParams {
    pub points: Vec<[f64; 2]>,
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
}

#[derive(Deserialize)]
pub struct GridParams {
    #[serde(default, alias = "ref", alias = "image_ref")]
    pub image: Option<String>,
    #[serde(default)]
    pub frame: Option<String>,
    #[serde(default)]
    pub density: Option<u8>,
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
    x >= -0.5 && x < w as f64 - 0.5 && y >= -0.5 && y < h as f64 - 0.5
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
    let frame = parse_frame(params.frame.as_deref())?;
    let target = target_ref(&session, params.image_ref).await?;
    let (wcs, w, h) = load_wcs(&session, &target)?;

    let coords: Vec<(f64, f64)> = params.points.iter().map(|p| (p[0], p[1])).collect();
    let sky = wcs.pixel_to_world_batch(&coords);

    let results: Vec<Value> = coords
        .iter()
        .zip(sky.iter())
        .map(|(&(x, y), c)| {
            let (lon, lat) = convert_from_icrs(frame, c.ra, c.dec);
            json!({
                "x": x,
                "y": y,
                "ra": lon,
                "dec": lat,
                "on_image": on_image(x, y, w, h),
            })
        })
        .collect();

    Ok(Json(json!({
        "ref": target,
        "frame": frame.name(),
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

pub async fn grid(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<GridParams>,
) -> Result<Json<Value>> {
    let frame = parse_frame(params.frame.as_deref())?;
    let density = params.density.unwrap_or(DEFAULT_DENSITY);
    if !(MIN_DENSITY..=MAX_DENSITY).contains(&density) {
        return Err(AppError::BadRequestWithHint {
            code: "bad_request",
            message: format!("grid density {density} is out of range"),
            hint: Some(format!("density must be between {MIN_DENSITY} and {MAX_DENSITY}")),
        });
    }
    let target = target_ref(&session, params.image).await?;
    let (wcs, w, h) = load_wcs(&session, &target)?;
    let grid = wcs_grid(&wcs, w, h, frame, density).map_err(AppError::BadRequest)?;
    let mut body = serde_json::to_value(&grid).map_err(|e| AppError::Internal(e.into()))?;
    body["ref"] = json!(target);
    Ok(Json(body))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_image_uses_the_pixel_edges_around_integer_centres() {
        assert!(on_image(-0.5, 0.0, 8, 8));
        assert!(on_image(-0.3, 7.49, 8, 8));
        assert!(!on_image(-0.51, 0.0, 8, 8));
        assert!(!on_image(7.5, 0.0, 8, 8));
        assert!(on_image(7.3, 0.0, 8, 8));
        assert!(!on_image(0.0, 7.7, 8, 8));
        assert!(!on_image(f64::NAN, 0.0, 8, 8));
    }

    #[test]
    fn parse_frame_defaults_to_icrs_and_accepts_the_four_names() {
        assert_eq!(parse_frame(None).unwrap(), SkyFrame::Icrs);
        assert_eq!(parse_frame(Some(" Galactic ")).unwrap(), SkyFrame::Galactic);
        assert_eq!(parse_frame(Some("fk5")).unwrap(), SkyFrame::Fk5J2000);
        assert_eq!(parse_frame(Some("ecliptic")).unwrap(), SkyFrame::EclipticJ2000);
    }

    #[test]
    fn parse_frame_rejects_unknown_with_bad_request_and_hint() {
        match parse_frame(Some("supergalactic")).unwrap_err() {
            AppError::BadRequestWithHint { code, message, hint } => {
                assert_eq!(code, "bad_request");
                assert!(message.contains("supergalactic"), "{message}");
                let hint = hint.unwrap();
                for f in SkyFrame::ALL {
                    assert!(hint.contains(f.name()), "{hint} lacks {}", f.name());
                }
            }
            other => panic!("expected BadRequestWithHint, got {other:?}"),
        }
    }
}
