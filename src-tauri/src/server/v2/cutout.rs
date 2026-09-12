// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::region::RegionShape;
use astroburst_lib::core::imaging::stats::compute_image_stats;
use astroburst_lib::infra::cache::PlaneLoad;
use astroburst_lib::types::header::HduHeader;
use ndarray::Array2;

use super::images::{load_replacing, register_and_respond};
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

#[derive(Debug)]
struct CutoutRect {
    x0: i64,
    y0: i64,
    width: usize,
    height: usize,
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

fn resolve_cutout_rect(
    region: &RegionSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
    max_bytes: usize,
) -> Result<CutoutRect> {
    let (x0, y0, width, height) = match region {
        RegionSpec::Pixel { x, y, width, height, .. } => (*x, *y, *width, *height),
        RegionSpec::Shape(spec) => {
            let b = pixel_shape(spec, img_w, img_h, wcs)?.bounds();
            (b.x0, b.y0, (b.x1 - b.x0 + 1).max(1) as usize, (b.y1 - b.y0 + 1).max(1) as usize)
        }
        RegionSpec::Sky { ra, dec, size_arcmin, .. } => {
            let wcs = wcs.ok_or_else(|| AppError::BadRequestWithHint {
                code: "wcs_required",
                message: "sky region requires a WCS on the image, but none is present".into(),
                hint: Some("open an image whose header carries WCS keywords, or use a pixel region".into()),
            })?;
            let (cx, cy) = wcs.world_to_pixel(*ra, *dec);
            if !cx.is_finite() || !cy.is_finite() {
                return Err(AppError::BadRequestWithHint {
                    code: "region_out_of_bounds",
                    message: format!("sky position ({ra}, {dec}) does not project onto the image plane"),
                    hint: Some(format!("image extent is 0..{img_w} x 0..{img_h} px")),
                });
            }
            let scale = wcs.pixel_scale_arcsec();
            if !(scale.is_finite() && scale > 0.0) {
                return Err(AppError::BadRequestWithHint {
                    code: "wcs_required",
                    message: "image WCS has a degenerate pixel scale".into(),
                    hint: None,
                });
            }
            let (wa, ha) = size_arcmin.wh();
            let wpx = (wa * 60.0 / scale).round().max(1.0) as usize;
            let hpx = (ha * 60.0 / scale).round().max(1.0) as usize;
            let x0 = (cx - wpx as f64 / 2.0).round() as i64;
            let y0 = (cy - hpx as f64 / 2.0).round() as i64;
            (x0, y0, wpx, hpx)
        }
    };

    if width == 0 || height == 0 {
        return Err(AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: "cutout width and height must both be > 0".into(),
            hint: Some(format!("image extent is 0..{img_w} x 0..{img_h} px")),
        });
    }

    let bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()));
    let representable = i64::try_from(width).ok().and_then(|w| x0.checked_add(w)).is_some()
        && i64::try_from(height).ok().and_then(|h| y0.checked_add(h)).is_some();
    if !matches!(bytes, Some(b) if b <= max_bytes) || !representable {
        return Err(AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: format!(
                "cutout {width}x{height} px at ({x0}, {y0}) exceeds the session memory budget of {max_bytes} bytes"
            ),
            hint: Some(format!("image extent is 0..{img_w} x 0..{img_h} px")),
        });
    }

    Ok(CutoutRect { x0, y0, width, height })
}

fn fraction_on_image(rect: &CutoutRect, img_w: usize, img_h: usize) -> f64 {
    let x1 = rect.x0 as f64 + rect.width as f64;
    let y1 = rect.y0 as f64 + rect.height as f64;
    let on_w = (x1.min(img_w as f64) - (rect.x0.max(0) as f64)).max(0.0);
    let on_h = (y1.min(img_h as f64) - (rect.y0.max(0) as f64)).max(0.0);
    (on_w * on_h) / (rect.width as f64 * rect.height as f64)
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
        shifted_header(entry.header(), &rect)
    } else {
        HduHeader::empty()
    };

    let image_ref = params
        .name
        .clone()
        .unwrap_or_else(|| session.v2.next_ref("cutout"));

    let data = entry.data_arc();
    let CutoutRect { x0, y0, width, height } = rect;
    let sess = session.clone();
    let ref_for_load = image_ref.clone();
    let cutout_entry = tokio::task::spawn_blocking(move || {
        load_replacing(&sess.cache, &ref_for_load, || {
            let mut out = Array2::<f32>::from_elem((height, width), f32::NAN);
            for oy in 0..height {
                let sy = y0 + oy as i64;
                if sy < 0 || sy >= img_h as i64 {
                    continue;
                }
                for ox in 0..width {
                    let sx = x0 + ox as i64;
                    if sx < 0 || sx >= img_w as i64 {
                        continue;
                    }
                    if mask_shape.as_ref().is_some_and(|s| !s.contains(sx as f64, sy as f64)) {
                        continue;
                    }
                    out[[oy, ox]] = data[[sy as usize, sx as usize]];
                }
            }
            let stats = compute_image_stats(&out);
            Ok(PlaneLoad::synthetic(out, stats, header))
        })
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))?
    .map_err(AppError::Internal)?;

    let mut body = register_and_respond(&session, image_ref.clone(), None, None, &cutout_entry);
    body["fraction_on_image"] = json!(fraction);
    body["region"] = json!({
        "x": x0, "y": y0, "width": width, "height": height,
    });
    if let Some(s) = &shape {
        body["region"]["shape"] = json!(s);
    }
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

fn shifted_header(parent: Option<&HduHeader>, rect: &CutoutRect) -> HduHeader {
    let mut hdr = match parent {
        Some(h) => h.clone(),
        None => return HduHeader::empty(),
    };
    if let Some(cr1) = hdr.get_f64("CRPIX1") {
        hdr.set_f64("CRPIX1", cr1 - rect.x0 as f64);
    }
    if let Some(cr2) = hdr.get_f64("CRPIX2") {
        hdr.set_f64("CRPIX2", cr2 - rect.y0 as f64);
    }
    hdr.set("NAXIS1", rect.width.to_string());
    hdr.set("NAXIS2", rect.height.to_string());
    hdr
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
    fn fraction_on_image_does_not_overflow_for_huge_rects() {
        let rect = CutoutRect { x0: 0, y0: 0, width: 1 << 40, height: 1 << 40 };
        let f = fraction_on_image(&rect, 8, 8);
        assert!(f.is_finite());
        assert!(f > 0.0 && f < 1e-20);
    }
}
