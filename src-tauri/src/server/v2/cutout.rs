// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::cutout::{
    box_pixel_rect, cut_plane_within, fraction_on_image, reported_ltv, shift_header, CutoutError, CutoutRect,
    CutoutRequest,
};
use astroburst_lib::core::imaging::region::RegionShape;
use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::infra::cache::PlaneLoad;
use astroburst_lib::types::header::HduHeader;

use super::images::{finite_stats_json, load_replacing, register_and_respond};
use super::region::{pixel_shape, RegionSpec};
use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CutoutParams {
    pub region: RegionSpec,
    #[serde(default, alias = "ref")]
    pub image_ref: Option<String>,
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub preserve_wcs: bool,
    #[serde(default)]
    pub mask_outside: bool,
}

fn default_true() -> bool {
    true
}

fn resolve_cutout_shape(
    region: &RegionSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
) -> Result<Option<RegionShape>> {
    match region {
        RegionSpec::Shape(spec) => Ok(Some(pixel_shape(spec, img_w, img_h, wcs)?)),
        _ => Ok(None),
    }
}

fn extent_hint(img_w: usize, img_h: usize) -> Option<String> {
    Some(format!("image extent is 0..{img_w} x 0..{img_h} px"))
}

fn cutout_request(
    region: &RegionSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
) -> Result<CutoutRequest> {
    Ok(match region {
        RegionSpec::Pixel { x, y, width, height, .. } => {
            CutoutRequest::Pixel { x0: *x, y0: *y, width: *width, height: *height }
        }
        RegionSpec::Shape(spec) => {
            let shape = pixel_shape(spec, img_w, img_h, wcs)?;
            let rect = match box_pixel_rect(&shape) {
                Some(region_rect) => region_rect.rect,
                None => {
                    let b = shape.bounds();
                    CutoutRect {
                        x0: b.x0,
                        y0: b.y0,
                        width: (b.x1 - b.x0 + 1).max(1) as usize,
                        height: (b.y1 - b.y0 + 1).max(1) as usize,
                    }
                }
            };
            CutoutRequest::Pixel { x0: rect.x0, y0: rect.y0, width: rect.width, height: rect.height }
        }
        RegionSpec::Sky { ra, dec, size_arcmin, .. } => {
            let (width_arcmin, height_arcmin) = size_arcmin.wh();
            CutoutRequest::Sky { ra: *ra, dec: *dec, width_arcmin, height_arcmin }
        }
    })
}

fn cutout_error(err: CutoutError, img_w: usize, img_h: usize) -> AppError {
    match err {
        CutoutError::WcsRequired => AppError::BadRequestWithHint {
            code: "wcs_required",
            message: err.to_string(),
            hint: Some("open an image whose header carries WCS keywords, or use a pixel region".into()),
        },
        CutoutError::DegenerateScale => AppError::BadRequestWithHint {
            code: "wcs_required",
            message: err.to_string(),
            hint: None,
        },
        CutoutError::OffImage { .. } | CutoutError::OutsideImage { .. } | CutoutError::EmptyRect => {
            AppError::BadRequestWithHint {
                code: "region_out_of_bounds",
                message: err.to_string(),
                hint: extent_hint(img_w, img_h),
            }
        }
        CutoutError::TooLarge { x0, y0, width, height, max_bytes } => AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: format!(
                "cutout {width}x{height} px at ({x0}, {y0}) exceeds the session memory budget of {max_bytes} bytes"
            ),
            hint: extent_hint(img_w, img_h),
        },
    }
}

fn resolve_cutout_rect(
    region: &RegionSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
    max_bytes: usize,
) -> Result<CutoutRect> {
    let request = cutout_request(region, img_w, img_h, wcs)?;
    astroburst_lib::core::imaging::cutout::resolve_cutout_rect(&request, img_w, img_h, wcs, max_bytes)
        .map_err(|e| cutout_error(e, img_w, img_h))
}

fn attach_cutout_fields(
    body: &mut Value,
    rect: &CutoutRect,
    fraction: f64,
    shape: Option<&RegionShape>,
    ltv: (f64, f64),
) {
    body["fraction_on_image"] = json!(fraction);
    body["region"] = json!({
        "x": rect.x0, "y": rect.y0, "width": rect.width, "height": rect.height,
    });
    if let Some(s) = shape {
        body["region"]["shape"] = json!(s);
    }
    body["ltv1"] = json!(ltv.0);
    body["ltv2"] = json!(ltv.1);
}

pub async fn cutout(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
    Json(params): Json<CutoutParams>,
) -> Result<Json<Value>> {
    let target = target_ref(&session, params.image_ref).await?;

    let entry = session
        .cache
        .get(&target)
        .ok_or_else(|| AppError::NotFound(format!("image ref {target} not found in session")))?;
    let (img_h, img_w) = entry.arr().dim();

    let wcs = entry
        .header()
        .and_then(|h| WcsTransform::from_header(h).ok());
    let rect = resolve_cutout_rect(
        &params.region,
        img_w,
        img_h,
        wcs.as_ref(),
        state.config.cache_max_bytes,
    )?;
    let fraction = fraction_on_image(&rect, img_w, img_h);
    let shape = resolve_cutout_shape(&params.region, img_w, img_h, wcs.as_ref())?;
    let mask_shape = if params.mask_outside { shape.clone() } else { None };

    let header = if params.preserve_wcs {
        entry
            .header()
            .map(|h| shift_header(h, &rect))
            .unwrap_or_else(HduHeader::empty)
    } else {
        HduHeader::empty()
    };
    let ltv = reported_ltv(Some(&header), &rect);

    let image_ref = params
        .name
        .clone()
        .unwrap_or_else(|| session.next_free_ref("cutout"));

    let data = entry.data_arc();
    let sess = session.clone();
    let ref_for_load = image_ref.clone();
    let (cutout_entry, response_stats) = tokio::task::spawn_blocking(move || {
        load_replacing(&sess.cache, &ref_for_load, || {
            let out = cut_plane_within(&data, &rect, mask_shape.as_ref());
            let stats = compute_image_stats(&out);
            Ok(PlaneLoad::synthetic(out, stats, header))
        })
        .map(|entry| {
            let stats = finite_stats_json(entry.arr());
            (entry, stats)
        })
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    let mut body = register_and_respond(&session, image_ref.clone(), None, None, &cutout_entry, response_stats);
    attach_cutout_fields(&mut body, &rect, fraction, shape.as_ref(), ltv);
    *session.v2.active_ref.write().await = Some(image_ref);
    Ok(Json(body))
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

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: usize = 2 * 1024 * 1024 * 1024;

    fn code_of(e: &AppError) -> Option<&'static str> {
        match e {
            AppError::BadRequestWithHint { code, .. } => Some(code),
            _ => None,
        }
    }

    fn pixel(x: i64, y: i64, width: usize, height: usize) -> RegionSpec {
        RegionSpec::Pixel { x, y, width, height, clip: None }
    }

    #[test]
    fn cutout_larger_than_the_memory_budget_is_rejected() {
        let err = resolve_cutout_rect(&pixel(0, 0, 100_000, 100_000), 8, 8, None, BUDGET).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));

        let err = resolve_cutout_rect(&pixel(0, 0, 1 << 40, 1 << 40), 8, 8, None, BUDGET).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));

        let err = resolve_cutout_rect(&pixel(0, 0, usize::MAX, 1), 8, 8, None, BUDGET).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));

        let err = resolve_cutout_rect(&pixel(i64::MAX, 0, 4, 4), 8, 8, None, BUDGET).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));

        let err = resolve_cutout_rect(&pixel(0, 0, 4, 4), 8, 8, None, 63).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
    }

    #[test]
    fn cutout_within_budget_keeps_nan_padding_semantics() {
        let r = resolve_cutout_rect(&pixel(6, 6, 4, 4), 8, 8, None, BUDGET).unwrap();
        assert_eq!((r.x0, r.y0, r.width, r.height), (6, 6, 4, 4));
        assert!((fraction_on_image(&r, 8, 8) - 0.25).abs() < 1e-12);

        let r = resolve_cutout_rect(&pixel(-2, -2, 4, 4), 8, 8, None, 64).unwrap();
        assert!((fraction_on_image(&r, 8, 8) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn shape_cutout_rect_comes_from_bounds() {
        let spec: RegionSpec =
            serde_json::from_str(r#"{"type":"shape","shape":"circle","x":4.5,"y":4.5,"r":2}"#).unwrap();
        let r = resolve_cutout_rect(&spec, 10, 10, None, BUDGET).unwrap();
        assert_eq!((r.x0, r.y0, r.width, r.height), (2, 2, 6, 6));
        let shape = resolve_cutout_shape(&spec, 10, 10, None).unwrap().expect("shape");
        assert_eq!(shape.kind(), "circle");
        assert!(resolve_cutout_shape(&pixel(0, 0, 2, 2), 10, 10, None).unwrap().is_none());

        let sky: RegionSpec = serde_json::from_str(
            r#"{"type":"shape","shape":"circle","x":150.0,"y":2.0,"r":3,"system":"fk5"}"#,
        )
        .unwrap();
        let err = resolve_cutout_rect(&sky, 10, 10, None, BUDGET).unwrap_err();
        assert_eq!(code_of(&err), Some("wcs_required"));
    }

    #[test]
    fn box_shape_cutout_rect_matches_the_desktop_pixel_centre_rule() {
        let half_integer: RegionSpec = serde_json::from_str(
            r#"{"type":"shape","shape":"box","x":15.5,"y":10.5,"width":10,"height":6}"#,
        )
        .unwrap();
        let r = resolve_cutout_rect(&half_integer, 40, 40, None, BUDGET).unwrap();
        assert_eq!((r.x0, r.y0, r.width, r.height), (11, 8, 10, 6));
        let desktop = astroburst_lib::core::imaging::cutout::box_pixel_rect(&RegionShape::Box {
            x: 15.5,
            y: 10.5,
            width: 10.0,
            height: 6.0,
            angle: 0.0,
        })
        .unwrap()
        .rect;
        assert_eq!((r.x0, r.y0, r.width, r.height), (desktop.x0, desktop.y0, desktop.width, desktop.height));

        let quarter: RegionSpec = serde_json::from_str(
            r#"{"type":"shape","shape":"box","x":15.5,"y":10.5,"width":10,"height":6,"angle":90}"#,
        )
        .unwrap();
        let r = resolve_cutout_rect(&quarter, 40, 40, None, BUDGET).unwrap();
        assert_eq!((r.width, r.height), (6, 10));

        let integer_centre: RegionSpec =
            serde_json::from_str(r#"{"type":"shape","shape":"box","x":15,"y":10,"width":10,"height":6}"#).unwrap();
        let r = resolve_cutout_rect(&integer_centre, 40, 40, None, BUDGET).unwrap();
        assert_eq!((r.x0, r.y0, r.width, r.height), (10, 7, 11, 7));

        let rotated: RegionSpec = serde_json::from_str(
            r#"{"type":"shape","shape":"box","x":15.5,"y":10.5,"width":10,"height":6,"angle":30}"#,
        )
        .unwrap();
        let r = resolve_cutout_rect(&rotated, 40, 40, None, BUDGET).unwrap();
        let b = RegionShape::Box { x: 15.5, y: 10.5, width: 10.0, height: 6.0, angle: 30.0 }.bounds();
        assert_eq!((r.x0, r.y0, r.width as i64, r.height as i64), (b.x0, b.y0, b.x1 - b.x0 + 1, b.y1 - b.y0 + 1));
    }

    #[test]
    fn sky_request_entirely_outside_the_image_maps_to_region_out_of_bounds() {
        let err = cutout_error(CutoutError::OutsideImage { x0: 3, y0: 1850, width: 6, height: 6 }, 100, 100);
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
        match err {
            AppError::BadRequestWithHint { message, hint, .. } => {
                assert!(message.contains("6x6 px at (3, 1850)"), "{message}");
                assert_eq!(hint.as_deref(), Some("image extent is 0..100 x 0..100 px"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fraction_on_image_does_not_overflow_for_huge_rects() {
        let rect = CutoutRect { x0: 0, y0: 0, width: 1 << 40, height: 1 << 40 };
        let f = fraction_on_image(&rect, 8, 8);
        assert!(f.is_finite());
        assert!(f > 0.0 && f < 1e-20);
    }

    #[test]
    fn empty_and_sky_requests_map_to_the_same_error_codes_as_before() {
        let err = resolve_cutout_rect(&pixel(0, 0, 0, 4), 8, 8, None, BUDGET).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
        let sky: RegionSpec =
            serde_json::from_str(r#"{"type":"sky","ra":150.0,"dec":2.0,"size_arcmin":1.0}"#).unwrap();
        let err = resolve_cutout_rect(&sky, 8, 8, None, BUDGET).unwrap_err();
        assert_eq!(code_of(&err), Some("wcs_required"));
        match err {
            AppError::BadRequestWithHint { hint, .. } => assert!(hint.is_some()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn response_body_reports_the_registered_header_ltv_or_the_rect_origin() {
        let rect = CutoutRect { x0: 6, y0: -3, width: 4, height: 4 };
        let mut body = json!({ "ref": "cutout_0" });
        let plain = reported_ltv(Some(&HduHeader::empty()), &rect);
        attach_cutout_fields(&mut body, &rect, 0.25, None, plain);
        assert_eq!(body["ltv1"], json!(-6.0));
        assert_eq!(body["ltv2"], json!(3.0));
        assert_eq!(body["region"]["x"], 6);
        assert_eq!(body["region"]["height"], 4);
        assert_eq!(body["fraction_on_image"], json!(0.25));
        assert!(body["region"].get("shape").is_none());

        let circle = RegionShape::Circle { x: 4.0, y: 4.0, r: 2.0 };
        attach_cutout_fields(&mut body, &rect, 1.0, Some(&circle), plain);
        assert_eq!(body["region"]["shape"]["shape"], "circle");

        let mut subarray = HduHeader::empty();
        subarray.set_f64("LTV1", -512.0);
        subarray.set_f64("LTV2", 0.0);
        let registered = shift_header(&subarray, &CutoutRect { x0: 10, y0: 4, width: 4, height: 4 });
        let composed = reported_ltv(Some(&registered), &rect);
        attach_cutout_fields(&mut body, &rect, 1.0, None, composed);
        assert_eq!(body["ltv1"], json!(-522.0));
        assert_eq!(body["ltv2"], json!(-4.0));
        assert_eq!(body["ltv1"], json!(registered.get_f64("LTV1").unwrap()));
    }
}
