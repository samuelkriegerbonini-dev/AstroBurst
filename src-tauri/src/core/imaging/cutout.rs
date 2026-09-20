use ndarray::{s, Array2};
use serde::Serialize;

use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::region::{shape_to_pixel, RegionShape, RegionSystem};
use crate::types::header::HduHeader;

const AXIS_ANGLE_TOLERANCE_DEG: f64 = 1e-9;
const PIXEL_EDGE_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CutoutRect {
    pub x0: i64,
    pub y0: i64,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CutoutRequest {
    Pixel { x0: i64, y0: i64, width: usize, height: usize },
    Sky { ra: f64, dec: f64, width_arcmin: f64, height_arcmin: f64 },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CutoutError {
    #[error("sky region requires a WCS on the image, but none is present")]
    WcsRequired,
    #[error("sky position ({ra}, {dec}) does not project onto the image plane")]
    OffImage { ra: f64, dec: f64 },
    #[error("image WCS has a degenerate pixel scale")]
    DegenerateScale,
    #[error("sky cutout resolves to {width}x{height} px at ({x0}, {y0}), entirely outside the image")]
    OutsideImage { x0: i64, y0: i64, width: usize, height: usize },
    #[error("cutout width and height must both be > 0")]
    EmptyRect,
    #[error("cutout {width}x{height} px at ({x0}, {y0}) exceeds the memory budget of {max_bytes} bytes")]
    TooLarge { x0: i64, y0: i64, width: usize, height: usize, max_bytes: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RegionRect {
    pub rect: CutoutRect,
    pub rotated_box_used_bounds: bool,
}

struct Overlap {
    out_x: std::ops::Range<usize>,
    out_y: std::ops::Range<usize>,
    src_x0: usize,
    src_y0: usize,
}

fn overlap(rect: &CutoutRect, img_w: usize, img_h: usize) -> Option<Overlap> {
    let x_end = rect.x0.saturating_add(i64::try_from(rect.width).unwrap_or(i64::MAX)).min(img_w as i64);
    let y_end = rect.y0.saturating_add(i64::try_from(rect.height).unwrap_or(i64::MAX)).min(img_h as i64);
    let x_start = rect.x0.max(0);
    let y_start = rect.y0.max(0);
    if x_end <= x_start || y_end <= y_start {
        return None;
    }
    Some(Overlap {
        out_x: (x_start - rect.x0) as usize..(x_end - rect.x0) as usize,
        out_y: (y_start - rect.y0) as usize..(y_end - rect.y0) as usize,
        src_x0: x_start as usize,
        src_y0: y_start as usize,
    })
}

fn cut_with<T: Copy>(src: &Array2<T>, rect: &CutoutRect, fill: T, keep: Option<&RegionShape>) -> Array2<T> {
    let (img_h, img_w) = src.dim();
    let mut out = Array2::from_elem((rect.height, rect.width), fill);
    let Some(o) = overlap(rect, img_w, img_h) else {
        return out;
    };
    let cols = o.out_x.len();
    for (dy, oy) in o.out_y.enumerate() {
        let sy = o.src_y0 + dy;
        let src_row = src.slice(s![sy, o.src_x0..o.src_x0 + cols]);
        let mut out_row = out.slice_mut(s![oy, o.out_x.clone()]);
        match keep {
            None => out_row.assign(&src_row),
            Some(shape) => {
                for (dx, (dst, &value)) in out_row.iter_mut().zip(src_row.iter()).enumerate() {
                    if shape.contains((o.src_x0 + dx) as f64, sy as f64) {
                        *dst = value;
                    }
                }
            }
        }
    }
    out
}

pub fn resolve_cutout_rect(
    request: &CutoutRequest,
    img_w: usize,
    img_h: usize,
    wcs: Option<&WcsTransform>,
    max_bytes: usize,
) -> Result<CutoutRect, CutoutError> {
    let (x0, y0, width, height) = match *request {
        CutoutRequest::Pixel { x0, y0, width, height } => (x0, y0, width, height),
        CutoutRequest::Sky { ra, dec, width_arcmin, height_arcmin } => {
            let wcs = wcs.ok_or(CutoutError::WcsRequired)?;
            let (cx, cy) = wcs.world_to_pixel(ra, dec);
            if !cx.is_finite() || !cy.is_finite() {
                return Err(CutoutError::OffImage { ra, dec });
            }
            let scale = wcs.pixel_scale_arcsec();
            if !(scale.is_finite() && scale > 0.0) {
                return Err(CutoutError::DegenerateScale);
            }
            let wpx = (width_arcmin * 60.0 / scale).round().max(1.0) as usize;
            let hpx = (height_arcmin * 60.0 / scale).round().max(1.0) as usize;
            let x0 = (cx - wpx as f64 / 2.0).round() as i64;
            let y0 = (cy - hpx as f64 / 2.0).round() as i64;
            (x0, y0, wpx, hpx)
        }
    };

    if width == 0 || height == 0 {
        return Err(CutoutError::EmptyRect);
    }
    let bytes = width.checked_mul(height).and_then(|n| n.checked_mul(std::mem::size_of::<f32>()));
    let representable = i64::try_from(width).ok().and_then(|w| x0.checked_add(w)).is_some()
        && i64::try_from(height).ok().and_then(|h| y0.checked_add(h)).is_some();
    if !matches!(bytes, Some(b) if b <= max_bytes) || !representable {
        return Err(CutoutError::TooLarge { x0, y0, width, height, max_bytes });
    }
    let rect = CutoutRect { x0, y0, width, height };
    if matches!(request, CutoutRequest::Sky { .. }) && fraction_on_image(&rect, img_w, img_h) <= 0.0 {
        return Err(CutoutError::OutsideImage { x0, y0, width, height });
    }
    Ok(rect)
}

pub fn fraction_on_image(rect: &CutoutRect, img_w: usize, img_h: usize) -> f64 {
    let x1 = rect.x0 as f64 + rect.width as f64;
    let y1 = rect.y0 as f64 + rect.height as f64;
    let on_w = (x1.min(img_w as f64) - (rect.x0.max(0) as f64)).max(0.0);
    let on_h = (y1.min(img_h as f64) - (rect.y0.max(0) as f64)).max(0.0);
    (on_w * on_h) / (rect.width as f64 * rect.height as f64)
}

pub fn cut_plane(arr: &Array2<f32>, rect: &CutoutRect) -> Array2<f32> {
    cut_with(arr, rect, f32::NAN, None)
}

pub fn cut_plane_within(arr: &Array2<f32>, rect: &CutoutRect, keep: Option<&RegionShape>) -> Array2<f32> {
    cut_with(arr, rect, f32::NAN, keep)
}

pub fn cut_int_plane(bits: &Array2<u32>, rect: &CutoutRect, pad_value: u32) -> Array2<u32> {
    cut_with(bits, rect, pad_value, None)
}

pub fn padding_plane(rect: &CutoutRect, img_w: usize, img_h: usize, pad_value: u32) -> Array2<u32> {
    let mut out = Array2::from_elem((rect.height, rect.width), pad_value);
    if let Some(o) = overlap(rect, img_w, img_h) {
        out.slice_mut(s![o.out_y, o.out_x]).fill(0);
    }
    out
}

pub fn reported_ltv(shifted: Option<&HduHeader>, rect: &CutoutRect) -> (f64, f64) {
    (
        shifted.and_then(|h| h.get_f64("LTV1")).unwrap_or(-(rect.x0 as f64)),
        shifted.and_then(|h| h.get_f64("LTV2")).unwrap_or(-(rect.y0 as f64)),
    )
}

pub fn shift_header(parent: &HduHeader, rect: &CutoutRect) -> HduHeader {
    let mut header = parent.clone();
    let (dx, dy) = (rect.x0 as f64, rect.y0 as f64);
    if let Some(crpix1) = header.get_f64("CRPIX1") {
        header.set_f64("CRPIX1", crpix1 - dx);
    }
    if let Some(crpix2) = header.get_f64("CRPIX2") {
        header.set_f64("CRPIX2", crpix2 - dy);
    }
    header.set_f64("LTV1", header.get_f64("LTV1").unwrap_or(0.0) - dx);
    header.set_f64("LTV2", header.get_f64("LTV2").unwrap_or(0.0) - dy);
    header.set_f64("LTM1_1", header.get_f64("LTM1_1").unwrap_or(1.0));
    header.set_f64("LTM2_2", header.get_f64("LTM2_2").unwrap_or(1.0));
    header.set("NAXIS1", rect.width.to_string());
    header.set("NAXIS2", rect.height.to_string());
    header
}

fn pixel_extent_rect(cx: f64, cy: f64, width: f64, height: f64) -> CutoutRect {
    let x0 = (cx - width / 2.0 - PIXEL_EDGE_TOLERANCE).ceil() as i64;
    let x1 = (cx + width / 2.0 + PIXEL_EDGE_TOLERANCE).floor() as i64;
    let y0 = (cy - height / 2.0 - PIXEL_EDGE_TOLERANCE).ceil() as i64;
    let y1 = (cy + height / 2.0 + PIXEL_EDGE_TOLERANCE).floor() as i64;
    CutoutRect { x0, y0, width: (x1 - x0 + 1).max(1) as usize, height: (y1 - y0 + 1).max(1) as usize }
}

pub fn rect_from_region(
    shape: &RegionShape,
    system: RegionSystem,
    wcs: Option<&WcsTransform>,
    dims: (usize, usize),
) -> Result<RegionRect, String> {
    let RegionShape::Box { .. } = shape else {
        return Err(format!("cutout region must be a box, got {}", shape.kind()));
    };
    shape.validate().map_err(|e| e.to_string())?;
    let pixel = shape_to_pixel(shape, system, wcs).map_err(|e| e.to_string())?;
    let region_rect =
        box_pixel_rect(&pixel).ok_or_else(|| format!("cutout region must be a box, got {}", pixel.kind()))?;

    let (img_w, img_h) = dims;
    if fraction_on_image(&region_rect.rect, img_w, img_h) <= 0.0 {
        return Err(format!("cutout region lies entirely outside the {img_w}x{img_h} image"));
    }
    Ok(region_rect)
}

pub fn box_pixel_rect(pixel_box: &RegionShape) -> Option<RegionRect> {
    let RegionShape::Box { x, y, width, height, angle } = *pixel_box else {
        return None;
    };
    let turn = angle.rem_euclid(180.0);
    let axis_aligned = turn < AXIS_ANGLE_TOLERANCE_DEG || (180.0 - turn) < AXIS_ANGLE_TOLERANCE_DEG;
    let quarter_turn = (turn - 90.0).abs() < AXIS_ANGLE_TOLERANCE_DEG;
    let (rect, rotated_box_used_bounds) = if axis_aligned {
        (pixel_extent_rect(x, y, width, height), false)
    } else if quarter_turn {
        (pixel_extent_rect(x, y, height, width), false)
    } else {
        let b = pixel_box.bounds();
        let rect = CutoutRect {
            x0: b.x0,
            y0: b.y0,
            width: (b.x1 - b.x0 + 1).max(1) as usize,
            height: (b.y1 - b.y0 + 1).max(1) as usize,
        };
        (rect, true)
    };
    Some(RegionRect { rect, rotated_box_used_bounds })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::{header_with_cd, make_header, north_up_cd};

    const BUDGET: usize = 1 << 30;

    fn rect(x0: i64, y0: i64, width: usize, height: usize) -> CutoutRect {
        CutoutRect { x0, y0, width, height }
    }

    fn ramp(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| (y * cols + x) as f32)
    }

    fn pixel(x0: i64, y0: i64, width: usize, height: usize) -> CutoutRequest {
        CutoutRequest::Pixel { x0, y0, width, height }
    }

    #[test]
    fn shift_header_moves_crpix_and_records_ltv_ltm() {
        let parent = make_header(&[
            ("NAXIS1", "100"),
            ("NAXIS2", "100"),
            ("CRPIX1", "50"),
            ("CRPIX2", "50"),
            ("OBJECT", "M16"),
        ]);
        let shifted = shift_header(&parent, &rect(10, 20, 30, 40));
        assert_eq!(shifted.get_f64("CRPIX1"), Some(40.0));
        assert_eq!(shifted.get_f64("CRPIX2"), Some(30.0));
        assert_eq!(shifted.get_f64("LTV1"), Some(-10.0));
        assert_eq!(shifted.get_f64("LTV2"), Some(-20.0));
        assert_eq!(shifted.get_f64("LTM1_1"), Some(1.0));
        assert_eq!(shifted.get_f64("LTM2_2"), Some(1.0));
        assert_eq!(shifted.get_i64("NAXIS1"), Some(30));
        assert_eq!(shifted.get_i64("NAXIS2"), Some(40));
        assert_eq!(shifted.get("OBJECT"), Some("M16"));
        assert_eq!(parent.get_f64("CRPIX1"), Some(50.0));
    }

    #[test]
    fn shift_header_composes_with_an_existing_ltv() {
        let parent = make_header(&[("CRPIX1", "50"), ("CRPIX2", "50"), ("LTV1", "-5"), ("LTV2", "3")]);
        let shifted = shift_header(&parent, &rect(10, 20, 4, 4));
        assert_eq!(shifted.get_f64("LTV1"), Some(-15.0));
        assert_eq!(shifted.get_f64("LTV2"), Some(-17.0));
    }

    #[test]
    fn shifted_header_wcs_agrees_with_the_parent_at_the_cutout_centre() {
        let parent = header_with_cd(north_up_cd());
        let r = rect(10, 20, 30, 40);
        let shifted = shift_header(&parent, &r);
        let parent_wcs = WcsTransform::from_header(&parent).unwrap();
        let cut_wcs = WcsTransform::from_header(&shifted).unwrap();
        let (cx, cy) = (14.5, 19.5);
        let a = cut_wcs.pixel_to_world(cx, cy);
        let b = parent_wcs.pixel_to_world(cx + 10.0, cy + 20.0);
        assert!((a.ra - b.ra).abs() < 1e-9, "{} vs {}", a.ra, b.ra);
        assert!((a.dec - b.dec).abs() < 1e-9, "{} vs {}", a.dec, b.dec);
        let corner = cut_wcs.pixel_to_world(0.0, 0.0);
        let parent_corner = parent_wcs.pixel_to_world(10.0, 20.0);
        assert!((corner.ra - parent_corner.ra).abs() < 1e-9);
        assert!((corner.dec - parent_corner.dec).abs() < 1e-9);
    }

    #[test]
    fn cut_plane_copies_inside_and_pads_nan_outside_the_parent() {
        let arr = ramp(5, 6);
        let inside = cut_plane(&arr, &rect(1, 2, 3, 2));
        assert_eq!(inside.dim(), (2, 3));
        assert_eq!(inside[[0, 0]], 13.0);
        assert_eq!(inside[[1, 2]], 21.0);
        assert!(inside.iter().all(|v| v.is_finite()));

        let r = rect(4, 3, 4, 4);
        let over = cut_plane(&arr, &r);
        assert_eq!(over.dim(), (4, 4));
        assert_eq!(over[[0, 0]], 22.0);
        assert_eq!(over[[1, 1]], 29.0);
        assert!(over[[0, 2]].is_nan());
        assert!(over[[2, 0]].is_nan());
        assert_eq!(over.iter().filter(|v| v.is_finite()).count(), 4);
        assert!((fraction_on_image(&r, 6, 5) - 0.25).abs() < 1e-12);
        assert!(fraction_on_image(&r, 6, 5) < 1.0);
        assert!((fraction_on_image(&rect(1, 2, 3, 2), 6, 5) - 1.0).abs() < 1e-12);

        let negative = cut_plane(&arr, &rect(-2, -1, 3, 2));
        assert!(negative[[0, 0]].is_nan());
        assert!(negative[[0, 1]].is_nan());
        assert!(negative[[1, 1]].is_nan());
        assert_eq!(negative[[1, 2]], 0.0);

        let mut with_nan = ramp(3, 3);
        with_nan[[1, 1]] = f32::NAN;
        let kept = cut_plane(&with_nan, &rect(0, 0, 3, 3));
        assert!(kept[[1, 1]].is_nan());
        assert_eq!(kept[[2, 2]], 8.0);
    }

    #[test]
    fn cut_plane_within_masks_pixels_outside_the_shape() {
        let arr = ramp(9, 9);
        let circle = RegionShape::Circle { x: 4.0, y: 4.0, r: 2.0 };
        let out = cut_plane_within(&arr, &rect(2, 2, 5, 5), Some(&circle));
        assert_eq!(out.dim(), (5, 5));
        assert_eq!(out.iter().filter(|v| v.is_finite()).count(), 13);
        assert_eq!(out[[2, 2]], 40.0);
        assert!(out[[0, 0]].is_nan());
        let plain = cut_plane_within(&arr, &rect(2, 2, 5, 5), None);
        assert_eq!(plain.iter().filter(|v| v.is_finite()).count(), 25);
    }

    #[test]
    fn cut_int_plane_applies_the_pad_value_outside_the_parent() {
        let bits = Array2::from_shape_fn((4, 4), |(y, x)| (y * 4 + x) as u32 + 100);
        let out = cut_int_plane(&bits, &rect(2, 2, 4, 3), 513);
        assert_eq!(out.dim(), (3, 4));
        assert_eq!(out[[0, 0]], 110);
        assert_eq!(out[[1, 1]], 115);
        assert_eq!(out[[0, 2]], 513);
        assert_eq!(out[[2, 0]], 513);
        assert_eq!(out.iter().filter(|&&v| v == 513).count(), 8);
        let big = Array2::from_elem((2, 2), 1u32 << 31);
        let out = cut_int_plane(&big, &rect(-1, 0, 3, 2), 0);
        assert_eq!(out[[0, 0]], 0);
        assert_eq!(out[[0, 1]], 1 << 31);
    }

    #[test]
    fn resolve_pixel_requests_and_budget() {
        let r = resolve_cutout_rect(&pixel(6, 6, 4, 4), 8, 8, None, BUDGET).unwrap();
        assert_eq!(r, rect(6, 6, 4, 4));
        assert!((fraction_on_image(&r, 8, 8) - 0.25).abs() < 1e-12);
        assert_eq!(resolve_cutout_rect(&pixel(0, 0, 0, 4), 8, 8, None, BUDGET), Err(CutoutError::EmptyRect));
        assert!(matches!(
            resolve_cutout_rect(&pixel(0, 0, 100_000, 100_000), 8, 8, None, BUDGET),
            Err(CutoutError::TooLarge { .. })
        ));
        assert!(matches!(
            resolve_cutout_rect(&pixel(0, 0, usize::MAX, 1), 8, 8, None, BUDGET),
            Err(CutoutError::TooLarge { .. })
        ));
        assert!(matches!(
            resolve_cutout_rect(&pixel(i64::MAX, 0, 4, 4), 8, 8, None, BUDGET),
            Err(CutoutError::TooLarge { .. })
        ));
        assert!(matches!(resolve_cutout_rect(&pixel(0, 0, 4, 4), 8, 8, None, 63), Err(CutoutError::TooLarge { .. })));
        let huge = fraction_on_image(&rect(0, 0, 1 << 40, 1 << 40), 8, 8);
        assert!(huge.is_finite() && huge > 0.0 && huge < 1e-20);
    }

    #[test]
    fn resolve_sky_requests_through_the_wcs() {
        let header = header_with_cd(north_up_cd());
        let wcs = WcsTransform::from_header(&header).unwrap();
        let sky = CutoutRequest::Sky { ra: 150.0, dec: 2.0, width_arcmin: 0.1, height_arcmin: 0.05 };
        let r = resolve_cutout_rect(&sky, 100, 100, Some(&wcs), BUDGET).unwrap();
        assert_eq!((r.width, r.height), (6, 3));
        assert_eq!((r.x0, r.y0), (47, 48));
        assert_eq!(resolve_cutout_rect(&sky, 100, 100, None, BUDGET), Err(CutoutError::WcsRequired));
        let off = CutoutRequest::Sky { ra: 330.0, dec: -2.0, width_arcmin: 1.0, height_arcmin: 1.0 };
        assert!(matches!(resolve_cutout_rect(&off, 100, 100, Some(&wcs), BUDGET), Err(CutoutError::OffImage { .. })));
    }

    #[test]
    fn rect_from_region_uses_the_box_extent_in_pixels() {
        let shape = RegionShape::Box { x: 15.5, y: 10.5, width: 10.0, height: 6.0, angle: 0.0 };
        let out = rect_from_region(&shape, RegionSystem::Image, None, (40, 40)).unwrap();
        assert_eq!(out.rect, rect(11, 8, 10, 6));
        assert!(!out.rotated_box_used_bounds);

        let integer_centre = RegionShape::Box { x: 15.0, y: 10.0, width: 10.0, height: 6.0, angle: 0.0 };
        let out = rect_from_region(&integer_centre, RegionSystem::Image, None, (40, 40)).unwrap();
        assert_eq!(out.rect, rect(10, 7, 11, 7));

        let quarter = RegionShape::Box { x: 15.5, y: 10.5, width: 10.0, height: 6.0, angle: 90.0 };
        let out = rect_from_region(&quarter, RegionSystem::Image, None, (40, 40)).unwrap();
        assert_eq!(out.rect, rect(13, 6, 6, 10));
        assert!(!out.rotated_box_used_bounds);

        let rotated = RegionShape::Box { x: 15.5, y: 10.5, width: 10.0, height: 6.0, angle: 30.0 };
        let out = rect_from_region(&rotated, RegionSystem::Image, None, (40, 40)).unwrap();
        assert!(out.rotated_box_used_bounds);
        let b = rotated.bounds();
        assert_eq!(out.rect.x0, b.x0);
        assert_eq!(out.rect.width as i64, b.x1 - b.x0 + 1);

        let overhang = RegionShape::Box { x: 1.0, y: 1.0, width: 6.0, height: 6.0, angle: 0.0 };
        let out = rect_from_region(&overhang, RegionSystem::Image, None, (40, 40)).unwrap();
        assert_eq!(out.rect, rect(-2, -2, 7, 7));
        assert!(fraction_on_image(&out.rect, 40, 40) < 1.0);

        let outside = RegionShape::Box { x: 100.0, y: 100.0, width: 4.0, height: 4.0, angle: 0.0 };
        assert!(rect_from_region(&outside, RegionSystem::Image, None, (40, 40)).is_err());
        let circle = RegionShape::Circle { x: 5.0, y: 5.0, r: 2.0 };
        assert!(rect_from_region(&circle, RegionSystem::Image, None, (40, 40)).unwrap_err().contains("box"));
        let invalid = RegionShape::Box { x: 5.0, y: 5.0, width: -1.0, height: 4.0, angle: 0.0 };
        assert!(rect_from_region(&invalid, RegionSystem::Image, None, (40, 40)).is_err());
    }

    #[test]
    fn rect_from_region_converts_a_sky_box_through_the_wcs() {
        let header = header_with_cd(north_up_cd());
        let wcs = WcsTransform::from_header(&header).unwrap();
        let sky = RegionShape::Box { x: 150.0, y: 2.0, width: 6.0, height: 4.0, angle: 0.0 };
        let out = rect_from_region(&sky, RegionSystem::Icrs, Some(&wcs), (100, 100)).unwrap();
        assert_eq!((out.rect.width, out.rect.height), (6, 4));
        assert_eq!((out.rect.x0, out.rect.y0), (47, 48));
        assert!(!out.rotated_box_used_bounds);
        assert!(rect_from_region(&sky, RegionSystem::Icrs, None, (100, 100)).unwrap_err().contains("WCS"));
    }

    #[test]
    fn sky_request_projecting_entirely_off_the_image_is_rejected() {
        let header = header_with_cd(north_up_cd());
        let wcs = WcsTransform::from_header(&header).unwrap();
        let far = CutoutRequest::Sky { ra: 150.0, dec: 2.5, width_arcmin: 0.1, height_arcmin: 0.1 };
        let err = resolve_cutout_rect(&far, 100, 100, Some(&wcs), BUDGET).unwrap_err();
        let CutoutError::OutsideImage { x0, y0, width, height } = err else {
            panic!("{err:?}");
        };
        assert_eq!((width, height), (6, 6));
        assert!(y0 > 100, "{y0}");
        assert!((0..100).contains(&x0), "{x0}");
        assert_eq!(err.to_string(), format!("sky cutout resolves to 6x6 px at ({x0}, {y0}), entirely outside the image"));
        let edge = CutoutRequest::Sky { ra: 150.0, dec: 2.0 + 49.0 / 3600.0, width_arcmin: 0.1, height_arcmin: 0.1 };
        let r = resolve_cutout_rect(&edge, 100, 100, Some(&wcs), BUDGET).unwrap();
        assert!(fraction_on_image(&r, 100, 100) < 1.0);
        assert!(fraction_on_image(&r, 100, 100) > 0.0);
    }

    #[test]
    fn padding_plane_flags_only_pixels_outside_the_parent() {
        let out = padding_plane(&rect(2, 2, 4, 3), 4, 4, 513);
        assert_eq!(out.dim(), (3, 4));
        assert_eq!(out, cut_int_plane(&Array2::<u32>::zeros((4, 4)), &rect(2, 2, 4, 3), 513));
        assert_eq!(out.iter().filter(|&&v| v == 513).count(), 8);
        assert_eq!(out[[0, 0]], 0);
        assert!(padding_plane(&rect(0, 0, 2, 2), 4, 4, 1).iter().all(|&v| v == 0));
        assert!(padding_plane(&rect(10, 10, 2, 2), 4, 4, 1).iter().all(|&v| v == 1));
    }

    #[test]
    fn shift_header_subtracts_the_origin_from_ltv_regardless_of_ltm() {
        let parent = make_header(&[("LTV1", "0"), ("LTM1_1", "0.5"), ("LTM2_2", "0.5")]);
        let shifted = shift_header(&parent, &rect(10, 20, 4, 4));
        assert_eq!(shifted.get_f64("LTV1"), Some(-10.0));
        assert_eq!(shifted.get_f64("LTV2"), Some(-20.0));
        assert_eq!(shifted.get_f64("LTM1_1"), Some(0.5));
        assert_eq!(shifted.get_f64("LTM2_2"), Some(0.5));
        assert!(shifted.get_f64("CRPIX1").is_none());

        let binned = make_header(&[("LTV1", "0.25"), ("LTV2", "0.25"), ("LTM1_1", "0.5"), ("LTM2_2", "0.5")]);
        let shifted = shift_header(&binned, &rect(10, 4, 4, 4));
        assert_eq!(shifted.get_f64("LTV1"), Some(-9.75));
        assert_eq!(shifted.get_f64("LTV2"), Some(-3.75));
        let physical = 40.0;
        let parent_logical = 0.5 * physical + 0.25;
        let cutout_logical = 0.5 * physical + shifted.get_f64("LTV1").unwrap();
        assert_eq!(cutout_logical, parent_logical - 10.0);
    }

    #[test]
    fn reported_ltv_reads_the_composed_header_value_or_falls_back_to_the_origin() {
        let r = rect(10, 4, 4, 4);
        assert_eq!(reported_ltv(None, &r), (-10.0, -4.0));
        assert_eq!(reported_ltv(Some(&HduHeader::empty()), &r), (-10.0, -4.0));
        let subarray = make_header(&[("LTV1", "-512"), ("LTV2", "-8")]);
        let shifted = shift_header(&subarray, &r);
        assert_eq!(reported_ltv(Some(&shifted), &r), (-522.0, -12.0));
        assert_eq!(reported_ltv(Some(&shifted), &r), (shifted.get_f64("LTV1").unwrap(), shifted.get_f64("LTV2").unwrap()));
    }

    #[test]
    fn box_pixel_rect_is_the_shared_rule_for_box_shapes() {
        let half_integer = RegionShape::Box { x: 15.5, y: 10.5, width: 10.0, height: 6.0, angle: 0.0 };
        let out = box_pixel_rect(&half_integer).unwrap();
        assert_eq!(out.rect, rect(11, 8, 10, 6));
        assert!(!out.rotated_box_used_bounds);
        assert!(box_pixel_rect(&RegionShape::Circle { x: 5.0, y: 5.0, r: 2.0 }).is_none());
        let rotated = RegionShape::Box { x: 15.5, y: 10.5, width: 10.0, height: 6.0, angle: 45.0 };
        assert!(box_pixel_rect(&rotated).unwrap().rotated_box_used_bounds);
    }
}
