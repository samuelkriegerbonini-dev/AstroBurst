// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use ndarray::{s, Array2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::region::{
    shape_to_pixel, PixelBounds, RegionError, RegionShape, RegionSystem,
};

use crate::error::AppError;

#[derive(Debug, Clone, Deserialize)]
pub struct ShapeSpec {
    #[serde(flatten)]
    pub shape: RegionShape,
    #[serde(default)]
    pub system: RegionSystem,
    #[serde(default)]
    pub clip: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SkySize {
    Square(f64),
    Rect([f64; 2]),
}

impl SkySize {
    pub(crate) fn wh(&self) -> (f64, f64) {
        match self {
            SkySize::Square(s) => (*s, *s),
            SkySize::Rect([w, h]) => (*w, *h),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RegionSpec {
    Pixel {
        x: i64,
        y: i64,
        width: usize,
        height: usize,
        clip: Option<bool>,
    },
    Sky {
        ra: f64,
        dec: f64,
        size_arcmin: SkySize,
        clip: Option<bool>,
    },
    Shape(ShapeSpec),
}

fn extent_hint(img_w: usize, img_h: usize) -> String {
    format!("image extent is 0..{img_w} x 0..{img_h} px")
}

fn wcs_required() -> AppError {
    AppError::BadRequestWithHint {
        code: "wcs_required",
        message: "sky region requires a WCS on the image, but none is present".into(),
        hint: Some("open an image whose header carries WCS keywords, or use a pixel region".into()),
    }
}

fn region_error(e: RegionError, img_w: usize, img_h: usize) -> AppError {
    match e {
        RegionError::WcsRequired => wcs_required(),
        RegionError::OffImage => AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: e.to_string(),
            hint: Some(extent_hint(img_w, img_h)),
        },
        other => AppError::BadRequest(other.to_string()),
    }
}

pub(crate) fn pixel_shape(
    spec: &ShapeSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
) -> Result<RegionShape, AppError> {
    spec.shape
        .validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let shape = shape_to_pixel(&spec.shape, spec.system, wcs).map_err(|e| region_error(e, img_w, img_h))?;
    if spec.system.is_sky() {
        shape.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    }
    Ok(shape)
}

fn bounds_rect(b: PixelBounds) -> (i64, i64, usize, usize) {
    (b.x0, b.y0, (b.x1 - b.x0 + 1).max(1) as usize, (b.y1 - b.y0 + 1).max(1) as usize)
}

pub fn resolve_shape(
    spec: &ShapeSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
) -> Result<(RegionShape, PixelBounds, bool), AppError> {
    let shape = pixel_shape(spec, img_w, img_h, wcs)?;
    let b = shape.bounds();
    let overlaps = b.x1 >= 0 && b.y1 >= 0 && b.x0 < img_w as i64 && b.y0 < img_h as i64;
    if !overlaps {
        return Err(AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: "region does not overlap the image at all".into(),
            hint: Some(extent_hint(img_w, img_h)),
        });
    }
    let clipped = b.x0 < 0 || b.y0 < 0 || b.x1 >= img_w as i64 || b.y1 >= img_h as i64;
    if clipped && spec.clip != Some(true) {
        return Err(AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: format!(
                "{} region bounds [x={}..{}, y={}..{}] do not fit the image",
                shape.kind(),
                b.x0,
                b.x1,
                b.y0,
                b.y1
            ),
            hint: Some(format!("{}; pass clip=true to clamp", extent_hint(img_w, img_h))),
        });
    }
    Ok((shape, b, clipped))
}

pub struct RegionValues {
    pub finite: Vec<f32>,
    pub n_nan: u64,
    pub region: serde_json::Value,
    pub shape: Option<RegionShape>,
}

pub fn region_values(
    arr: &Array2<f32>,
    spec: Option<&RegionSpec>,
    wcs: Option<&WcsTransform>,
) -> Result<RegionValues, AppError> {
    let (rows, cols) = arr.dim();
    if let Some(RegionSpec::Shape(shape_spec)) = spec {
        let (shape, bounds, clipped) = resolve_shape(shape_spec, cols, rows, wcs)?;
        let mv = shape.masked_values(arr, None);
        let mut region = serde_json::to_value(&shape)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialising region shape: {e}")))?;
        if let Some(obj) = region.as_object_mut() {
            obj.insert("bounds".into(), json!(bounds));
            obj.insert("clipped".into(), json!(clipped));
        }
        return Ok(RegionValues { finite: mv.values, n_nan: mv.n_nan, region, shape: Some(shape) });
    }

    let (region_arr, resolved): (Array2<f32>, ResolvedRegion) = match spec {
        Some(spec) => {
            let r = resolve_region(spec, cols, rows, wcs)?;
            let sub = arr
                .slice(s![r.y..r.y + r.height, r.x..r.x + r.width])
                .to_owned();
            (sub, r)
        }
        None => (
            arr.to_owned(),
            ResolvedRegion { x: 0, y: 0, width: cols, height: rows, clipped: false },
        ),
    };
    let slice = region_arr
        .as_slice()
        .expect("region_arr is standard-layout after to_owned()");
    let n_nan = slice.iter().filter(|v| v.is_nan()).count() as u64;
    let finite: Vec<f32> = slice.iter().copied().filter(|v| v.is_finite()).collect();
    Ok(RegionValues { finite, n_nan, region: json!(resolved), shape: None })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedRegion {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub clipped: bool,
}

pub(crate) fn spec_to_rect(
    spec: &RegionSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
) -> Result<(i64, i64, usize, usize), AppError> {
    let (x0, y0, w, h) = match spec {
        RegionSpec::Pixel { x, y, width, height, .. } => (*x, *y, *width, *height),
        RegionSpec::Shape(shape_spec) => bounds_rect(pixel_shape(shape_spec, img_w, img_h, wcs)?.bounds()),
        RegionSpec::Sky { ra, dec, size_arcmin, .. } => {
            let wcs = wcs.ok_or_else(wcs_required)?;
            let (cx, cy) = wcs.world_to_pixel(*ra, *dec);
            if !cx.is_finite() || !cy.is_finite() {
                return Err(AppError::BadRequestWithHint {
                    code: "region_out_of_bounds",
                    message: format!("sky position ({ra}, {dec}) does not project onto the image"),
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

    if w == 0 || h == 0 {
        return Err(AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: "region width and height must both be > 0".into(),
            hint: Some(format!("image extent is 0..{img_w} x 0..{img_h} px")),
        });
    }

    let representable = i64::try_from(w).ok().and_then(|v| x0.checked_add(v)).is_some()
        && i64::try_from(h).ok().and_then(|v| y0.checked_add(v)).is_some();
    if !representable {
        return Err(AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: format!("region [x={x0}, y={y0}, w={w}, h={h}] does not fit the image"),
            hint: Some(format!("image extent is 0..{img_w} x 0..{img_h} px")),
        });
    }

    Ok((x0, y0, w, h))
}

pub fn resolve_region(
    spec: &RegionSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
) -> Result<ResolvedRegion, AppError> {
    let clip = match spec {
        RegionSpec::Pixel { clip, .. } | RegionSpec::Sky { clip, .. } => clip.unwrap_or(false),
        RegionSpec::Shape(shape_spec) => shape_spec.clip.unwrap_or(false),
    };
    let (x0, y0, w, h) = spec_to_rect(spec, img_w, img_h, wcs)?;

    let x1 = x0 + w as i64;
    let y1 = y0 + h as i64;

    let fully_inside = x0 >= 0 && y0 >= 0 && x1 <= img_w as i64 && y1 <= img_h as i64;

    if fully_inside {
        return Ok(ResolvedRegion {
            x: x0 as usize,
            y: y0 as usize,
            width: w,
            height: h,
            clipped: false,
        });
    }

    if !clip {
        return Err(AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: format!(
                "region [x={x0}, y={y0}, w={w}, h={h}] does not fit the image"
            ),
            hint: Some(format!("image extent is 0..{img_w} x 0..{img_h} px; pass clip=true to clamp")),
        });
    }

    let cx0 = x0.clamp(0, img_w as i64);
    let cy0 = y0.clamp(0, img_h as i64);
    let cx1 = x1.clamp(0, img_w as i64);
    let cy1 = y1.clamp(0, img_h as i64);
    let cw = (cx1 - cx0).max(0) as usize;
    let ch = (cy1 - cy0).max(0) as usize;

    if cw == 0 || ch == 0 {
        return Err(AppError::BadRequestWithHint {
            code: "region_out_of_bounds",
            message: "region does not overlap the image at all".into(),
            hint: Some(format!("image extent is 0..{img_w} x 0..{img_h} px")),
        });
    }

    Ok(ResolvedRegion {
        x: cx0 as usize,
        y: cy0 as usize,
        width: cw,
        height: ch,
        clipped: true,
    })
}

pub fn resolve_region_clamped(
    spec: &RegionSpec,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
) -> Result<ResolvedRegion, AppError> {
    let (x0, y0, w, h) = spec_to_rect(spec, img_w, img_h, wcs)?;

    let x1 = x0 + w as i64;
    let y1 = y0 + h as i64;

    let cx0 = x0.clamp(0, img_w as i64);
    let cy0 = y0.clamp(0, img_h as i64);
    let cx1 = x1.clamp(0, img_w as i64);
    let cy1 = y1.clamp(0, img_h as i64);
    let mut cw = (cx1 - cx0).max(0) as usize;
    let mut ch = (cy1 - cy0).max(0) as usize;
    let mut fx0 = cx0;
    let mut fy0 = cy0;

    if cw == 0 {
        fx0 = cx0.min(img_w as i64 - 1).max(0);
        cw = 1;
    }
    if ch == 0 {
        fy0 = cy0.min(img_h as i64 - 1).max(0);
        ch = 1;
    }

    let clipped = fx0 != x0 || fy0 != y0 || cw != w || ch != h;

    Ok(ResolvedRegion {
        x: fx0 as usize,
        y: fy0 as usize,
        width: cw,
        height: ch,
        clipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(e: &AppError) -> Option<&'static str> {
        match e {
            AppError::BadRequestWithHint { code, .. } => Some(code),
            _ => None,
        }
    }

    fn hint_of(e: &AppError) -> Option<String> {
        match e {
            AppError::BadRequestWithHint { hint, .. } => hint.clone(),
            _ => None,
        }
    }

    #[test]
    fn pixel_region_fully_inside_resolves_unchanged() {
        let spec = RegionSpec::Pixel { x: 10, y: 20, width: 30, height: 40, clip: None };
        let r = resolve_region(&spec, 100, 100, None).unwrap();
        assert_eq!(
            r,
            ResolvedRegion { x: 10, y: 20, width: 30, height: 40, clipped: false }
        );
    }

    #[test]
    fn oversized_pixel_region_errors_out_of_bounds_by_default() {
        let spec = RegionSpec::Pixel { x: 50, y: 0, width: 80, height: 10, clip: None };
        let err = resolve_region(&spec, 100, 100, None).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
        let hint = hint_of(&err).expect("hint present");
        assert!(hint.contains("100"), "hint should name the extent: {hint}");
    }

    #[test]
    fn oversized_pixel_region_clamps_when_clip_true() {
        let spec = RegionSpec::Pixel { x: 50, y: 0, width: 80, height: 10, clip: Some(true) };
        let r = resolve_region(&spec, 100, 100, None).unwrap();
        assert_eq!(
            r,
            ResolvedRegion { x: 50, y: 0, width: 50, height: 10, clipped: true }
        );
    }

    #[test]
    fn negative_origin_clamps_when_clip_true() {
        let spec = RegionSpec::Pixel { x: -5, y: -10, width: 20, height: 20, clip: Some(true) };
        let r = resolve_region(&spec, 100, 100, None).unwrap();
        assert_eq!(
            r,
            ResolvedRegion { x: 0, y: 0, width: 15, height: 10, clipped: true }
        );
    }

    #[test]
    fn sky_region_without_wcs_errors_wcs_required() {
        let spec = RegionSpec::Sky {
            ra: 10.0,
            dec: 20.0,
            size_arcmin: SkySize::Square(1.0),
            clip: None,
        };
        let err = resolve_region(&spec, 100, 100, None).unwrap_err();
        assert_eq!(code_of(&err), Some("wcs_required"));
    }

    #[test]
    fn zero_size_region_is_rejected() {
        let spec = RegionSpec::Pixel { x: 0, y: 0, width: 0, height: 10, clip: Some(true) };
        let err = resolve_region(&spec, 100, 100, None).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
    }

    #[test]
    fn huge_width_is_rejected_instead_of_wrapping_past_the_bounds_check() {
        for width in [1usize << 63, usize::MAX] {
            let spec = RegionSpec::Pixel { x: 0, y: 0, width, height: 1, clip: None };
            let err = resolve_region(&spec, 100, 100, None).unwrap_err();
            assert_eq!(code_of(&err), Some("region_out_of_bounds"), "width {width}");
            let spec = RegionSpec::Pixel { x: 0, y: 0, width, height: 1, clip: Some(true) };
            let err = resolve_region(&spec, 100, 100, None).unwrap_err();
            assert_eq!(code_of(&err), Some("region_out_of_bounds"), "clip width {width}");
        }
        let spec = RegionSpec::Pixel { x: 0, y: 0, width: 1, height: 1 << 63, clip: None };
        let err = resolve_region(&spec, 100, 100, None).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
    }

    #[test]
    fn origin_near_i64_max_is_rejected_in_both_resolvers() {
        let spec = RegionSpec::Pixel { x: i64::MAX, y: 0, width: 1, height: 1, clip: None };
        let err = resolve_region(&spec, 100, 100, None).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
        let err = resolve_region_clamped(&spec, 100, 100, None).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));

        let spec = RegionSpec::Pixel { x: 0, y: i64::MAX, width: 1, height: 1, clip: Some(true) };
        let err = resolve_region(&spec, 100, 100, None).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
    }

    #[test]
    fn clamped_resolver_never_errors_on_partial_overrun() {
        let spec = RegionSpec::Pixel { x: 6, y: 6, width: 10, height: 10, clip: None };
        let r = resolve_region_clamped(&spec, 8, 8, None).unwrap();
        assert_eq!(r, ResolvedRegion { x: 6, y: 6, width: 2, height: 2, clipped: true });
    }

    #[test]
    fn clamped_resolver_fully_inside_is_not_clipped() {
        let spec = RegionSpec::Pixel { x: 1, y: 1, width: 3, height: 3, clip: None };
        let r = resolve_region_clamped(&spec, 8, 8, None).unwrap();
        assert_eq!(r, ResolvedRegion { x: 1, y: 1, width: 3, height: 3, clipped: false });
    }

    #[test]
    fn clamped_resolver_fully_outside_degrades_to_edge_pixel() {
        let spec = RegionSpec::Pixel { x: 100, y: 100, width: 10, height: 10, clip: None };
        let r = resolve_region_clamped(&spec, 8, 8, None).unwrap();
        assert_eq!(r, ResolvedRegion { x: 7, y: 7, width: 1, height: 1, clipped: true });
    }

    #[test]
    fn shape_spec_deserialises_and_unknown_shapes_are_rejected() {
        let spec: RegionSpec =
            serde_json::from_str(r#"{"type":"shape","shape":"circle","x":4.5,"y":4.5,"r":2.0,"system":"image"}"#).unwrap();
        match spec {
            RegionSpec::Shape(s) => {
                assert_eq!(s.shape, RegionShape::Circle { x: 4.5, y: 4.5, r: 2.0 });
                assert_eq!(s.system, RegionSystem::Image);
                assert_eq!(s.clip, None);
            }
            other => panic!("{other:?}"),
        }
        let spec: RegionSpec = serde_json::from_str(
            r#"{"type":"shape","shape":"box","x":150.1,"y":2.2,"width":30,"height":15,"angle":0,"system":"fk5","clip":true}"#,
        )
        .unwrap();
        match spec {
            RegionSpec::Shape(s) => {
                assert_eq!(s.system, RegionSystem::Fk5);
                assert_eq!(s.clip, Some(true));
                assert_eq!(s.shape.kind(), "box");
            }
            other => panic!("{other:?}"),
        }
        assert!(serde_json::from_str::<RegionSpec>(r#"{"type":"shape","shape":"hexagon"}"#).is_err());
        let spec: RegionSpec =
            serde_json::from_str(r#"{"type":"pixel","x":1,"y":2,"width":3,"height":4}"#).unwrap();
        assert!(matches!(spec, RegionSpec::Pixel { x: 1, y: 2, width: 3, height: 4, clip: None }));
    }

    fn shape_spec(shape: RegionShape, clip: Option<bool>) -> ShapeSpec {
        ShapeSpec { shape, system: RegionSystem::Image, clip }
    }

    #[test]
    fn resolve_shape_bounds_clip_and_errors() {
        let inside = shape_spec(RegionShape::Circle { x: 4.5, y: 4.5, r: 2.0 }, None);
        let (shape, b, clipped) = resolve_shape(&inside, 10, 10, None).unwrap();
        assert_eq!(shape.kind(), "circle");
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (2, 2, 7, 7));
        assert!(!clipped);

        let edge = shape_spec(RegionShape::Circle { x: 8.0, y: 4.0, r: 3.0 }, None);
        let err = resolve_shape(&edge, 10, 10, None).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));
        assert!(hint_of(&err).unwrap().contains("clip=true"));
        let edge = shape_spec(RegionShape::Circle { x: 8.0, y: 4.0, r: 3.0 }, Some(true));
        let (_, _, clipped) = resolve_shape(&edge, 10, 10, None).unwrap();
        assert!(clipped);

        let far = shape_spec(RegionShape::Circle { x: 80.0, y: 80.0, r: 3.0 }, Some(true));
        let err = resolve_shape(&far, 10, 10, None).unwrap_err();
        assert_eq!(code_of(&err), Some("region_out_of_bounds"));

        let bad = shape_spec(RegionShape::Circle { x: 4.0, y: 4.0, r: 0.0 }, None);
        assert!(matches!(resolve_shape(&bad, 10, 10, None).unwrap_err(), AppError::BadRequest(_)));

        let sky = ShapeSpec {
            shape: RegionShape::Circle { x: 150.0, y: 2.0, r: 3.0 },
            system: RegionSystem::Fk5,
            clip: None,
        };
        let err = resolve_shape(&sky, 10, 10, None).unwrap_err();
        assert_eq!(code_of(&err), Some("wcs_required"));

        let rect = resolve_region(&RegionSpec::Shape(inside), 10, 10, None).unwrap();
        assert_eq!(rect, ResolvedRegion { x: 2, y: 2, width: 6, height: 6, clipped: false });
    }

    #[test]
    fn region_values_shape_and_rect_paths() {
        let mut arr = Array2::from_elem((10, 10), 1.0f32);
        arr[[4, 4]] = f32::NAN;
        arr[[0, 0]] = f32::INFINITY;
        let spec = RegionSpec::Shape(shape_spec(RegionShape::Circle { x: 4.5, y: 4.5, r: 2.0 }, None));
        let rv = region_values(&arr, Some(&spec), None).unwrap();
        let expected = RegionShape::Circle { x: 4.5, y: 4.5, r: 2.0 }.masked_values(&arr, None);
        assert_eq!(rv.finite.len(), expected.values.len());
        assert_eq!(rv.n_nan, 1);
        assert_eq!(rv.region["shape"], "circle");
        assert_eq!(rv.region["bounds"]["x0"], 2);
        assert_eq!(rv.region["clipped"], false);
        assert!(rv.shape.is_some());

        let rv = region_values(&arr, None, None).unwrap();
        assert_eq!(rv.finite.len(), 98);
        assert_eq!(rv.n_nan, 1);
        assert_eq!(rv.region["width"], 10);
        assert!(rv.shape.is_none());

        let px = RegionSpec::Pixel { x: 0, y: 0, width: 2, height: 2, clip: None };
        let rv = region_values(&arr, Some(&px), None).unwrap();
        assert_eq!(rv.finite.len(), 3);
        assert_eq!(rv.n_nan, 0);
        assert_eq!(rv.region["clipped"], false);
    }
}
