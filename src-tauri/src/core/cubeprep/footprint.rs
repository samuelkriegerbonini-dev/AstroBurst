use std::f64::consts::FRAC_PI_2;

use serde::{Deserialize, Serialize};

use crate::core::astrometry::wcs::{pixel_edge_corners, WcsTransform};
use crate::core::imaging::cutout::{fraction_on_image, pixel_bounds_rect, CutoutRect};
use crate::core::imaging::region::{shape_to_sky, PhysicalMap, RegionShape, RegionSystem};
use crate::core::imaging::region_file::parse_reg_with_physical;
use crate::infra::wcs_source::header_wcs;
use crate::math::polygon::{self, Point};
use crate::types::header::HduHeader;

const ARCSEC_PER_DEG: f64 = 3600.0;
const EDGE_SAMPLES: usize = 16;
const LIGHTSTACK_NORTH_OFFSET_ARCSEC: f64 = 1.0;
const SIP_ORDER_CARDS: [&str; 4] = ["A_ORDER", "B_ORDER", "AP_ORDER", "BP_ORDER"];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SkyPoint {
    pub ra: f64,
    pub dec: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SkyBox {
    pub center: SkyPoint,
    pub width_arcsec: f64,
    pub height_arcsec: f64,
    pub angle_deg: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SkyBbox {
    pub ra_min: f64,
    pub ra_max: f64,
    pub dec_min: f64,
    pub dec_max: f64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FootprintError {
    #[error("no image to crop")]
    Empty,
    #[error("the WCS does not map the footprint or the region between the sky and the image plane")]
    OffSky,
    #[error("the footprints span more than a hemisphere around their centre")]
    TooWide,
    #[error("the image WCS has a degenerate pixel scale")]
    DegenerateScale,
    #[error("the images have no common sky footprint")]
    NoCommonFootprint,
    #[error("the footprint crosses RA 0/360 or surrounds a pole, which a lightstack box of RA and Dec limits cannot hold; use the astroburst rounding")]
    RaWrap,
    #[error("the region boxes have no common sky area")]
    NoCommonRegion,
    #[error("the crop lies entirely outside the {width}x{height} image")]
    OutsideImage { width: usize, height: usize },
    #[error("no usable WCS: {0}")]
    Wcs(String),
    #[error("region file: {0}")]
    Region(String),
    #[error("the region file holds no fk5 or icrs box")]
    NoBoxes,
    #[error("region {index} is not a box (the file holds {count} regions)")]
    NotABox { index: usize, count: usize },
    #[error("the {0} crop needs {1}")]
    Missing(&'static str, &'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Tangent {
    origin: SkyPoint,
    sin0: f64,
    cos0: f64,
}

impl Tangent {
    pub fn at(origin: SkyPoint) -> Self {
        let (sin0, cos0) = origin.dec.to_radians().sin_cos();
        Self { origin, sin0, cos0 }
    }

    pub fn project(&self, point: SkyPoint) -> Option<Point> {
        let (sin_dec, cos_dec) = point.dec.to_radians().sin_cos();
        let (sin_delta, cos_delta) = (point.ra - self.origin.ra).to_radians().sin_cos();
        let cos_c = sin_dec * self.sin0 + cos_dec * self.cos0 * cos_delta;
        if !(cos_c > 0.0) {
            return None;
        }
        let scale = ARCSEC_PER_DEG * 180.0 / std::f64::consts::PI / cos_c;
        Some([cos_dec * sin_delta * scale, (self.cos0 * sin_dec - self.sin0 * cos_dec * cos_delta) * scale])
    }

    pub fn deproject(&self, plane: Point) -> SkyPoint {
        let to_radians = |arcsec: f64| (arcsec / ARCSEC_PER_DEG).to_radians();
        let (xi, eta) = (to_radians(plane[0]), to_radians(plane[1]));
        let den = self.cos0 - eta * self.sin0;
        let ra = (self.origin.ra + xi.atan2(den).to_degrees()).rem_euclid(360.0);
        let dec = (self.sin0 + eta * self.cos0).atan2(xi.hypot(den)).to_degrees();
        SkyPoint { ra, dec }
    }
}

pub fn centroid(points: &[SkyPoint]) -> Option<SkyPoint> {
    let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
    for point in points.iter().filter(|p| p.ra.is_finite() && p.dec.is_finite()) {
        let (sin_ra, cos_ra) = point.ra.to_radians().sin_cos();
        let (sin_dec, cos_dec) = point.dec.to_radians().sin_cos();
        x += cos_dec * cos_ra;
        y += cos_dec * sin_ra;
        z += sin_dec;
    }
    let norm = (x * x + y * y + z * z).sqrt();
    (norm > 0.0).then(|| SkyPoint { ra: y.atan2(x).to_degrees().rem_euclid(360.0), dec: z.atan2(x.hypot(y)).to_degrees() })
}

fn plane_of(polygon: &[SkyPoint]) -> Result<(Tangent, Vec<Point>), FootprintError> {
    let tangent = Tangent::at(centroid(polygon).ok_or(FootprintError::OffSky)?);
    let plane = polygon.iter().map(|point| tangent.project(*point)).collect::<Option<Vec<Point>>>().ok_or(FootprintError::TooWide)?;
    Ok((tangent, plane))
}

fn sky(wcs: &WcsTransform, pixels: &[(f64, f64)]) -> Result<Vec<SkyPoint>, FootprintError> {
    let points: Vec<SkyPoint> = wcs.pixel_to_world_batch(pixels).into_iter().map(|c| SkyPoint { ra: c.ra, dec: c.dec }).collect();
    if points.iter().all(|p| p.ra.is_finite() && p.dec.is_finite()) {
        Ok(points)
    } else {
        Err(FootprintError::OffSky)
    }
}

pub fn image_footprint(wcs: &WcsTransform, dims: (usize, usize)) -> Result<Vec<SkyPoint>, FootprintError> {
    if dims.0 == 0 || dims.1 == 0 {
        return Err(FootprintError::Empty);
    }
    let corners = pixel_edge_corners(dims.0, dims.1);
    let pixels: Vec<(f64, f64)> = (0..4)
        .flat_map(|k| {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            (0..EDGE_SAMPLES).map(move |i| {
                let t = i as f64 / EDGE_SAMPLES as f64;
                (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1))
            })
        })
        .collect();
    sky(wcs, &pixels)
}

fn rectangle_polygon(center: SkyPoint, width_arcsec: f64, height_arcsec: f64, angle_deg: f64) -> Vec<SkyPoint> {
    let tangent = Tangent::at(center);
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let (u, v) = ([-cos * width_arcsec / 2.0, sin * width_arcsec / 2.0], [sin * height_arcsec / 2.0, cos * height_arcsec / 2.0]);
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|(a, b)| tangent.deproject([a * u[0] + b * v[0], a * u[1] + b * v[1]]))
        .collect()
}

pub fn square_polygon(center: SkyPoint, size_arcsec: f64) -> Vec<SkyPoint> {
    rectangle_polygon(center, size_arcsec, size_arcsec, 0.0)
}

pub fn box_polygon(sky_box: &SkyBox) -> Vec<SkyPoint> {
    rectangle_polygon(sky_box.center, sky_box.width_arcsec, sky_box.height_arcsec, sky_box.angle_deg)
}

pub fn common_polygon(polygons: &[Vec<SkyPoint>]) -> Result<Vec<SkyPoint>, FootprintError> {
    let everything: Vec<SkyPoint> = polygons.iter().flatten().copied().collect();
    let (tangent, _) = plane_of(&everything)?;
    let mut common: Option<Vec<Point>> = None;
    for polygon in polygons {
        let plane = polygon.iter().map(|point| tangent.project(*point)).collect::<Option<Vec<Point>>>().ok_or(FootprintError::TooWide)?;
        let hull = polygon::convex_hull(&plane);
        common = Some(match common {
            None => hull,
            Some(current) => polygon::clip_convex(&current, &hull),
        });
    }
    let hull = polygon::convex_hull(&common.ok_or(FootprintError::Empty)?);
    if hull.len() < 3 || polygon::area(&hull) <= 0.0 {
        return Err(FootprintError::NoCommonFootprint);
    }
    Ok(hull.iter().map(|point| tangent.deproject(*point)).collect())
}

pub fn polygon_area_arcsec2(polygon: &[SkyPoint]) -> f64 {
    plane_of(polygon).map_or(0.0, |(_, plane)| polygon::area(&plane))
}

pub fn polygon_contains(polygon: &[SkyPoint], point: SkyPoint) -> bool {
    let Ok((tangent, plane)) = plane_of(polygon) else {
        return false;
    };
    tangent.project(point).is_some_and(|projected| polygon::contains(&polygon::convex_hull(&plane), projected))
}

fn densified(polygon: &[SkyPoint]) -> Result<Vec<SkyPoint>, FootprintError> {
    let (tangent, plane) = plane_of(polygon)?;
    Ok((0..plane.len())
        .flat_map(|k| {
            let (a, b) = (plane[k], plane[(k + 1) % plane.len()]);
            (0..EDGE_SAMPLES).map(move |i| {
                let t = i as f64 / EDGE_SAMPLES as f64;
                [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
            })
        })
        .map(|point| tangent.deproject(point))
        .collect())
}

fn pixels_of(wcs: &WcsTransform, points: &[SkyPoint]) -> Result<Vec<(f64, f64)>, FootprintError> {
    let coords: Vec<(f64, f64)> = points.iter().map(|p| (p.ra, p.dec)).collect();
    let pixels = wcs.world_to_pixel_batch(&coords);
    if pixels.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
        Ok(pixels)
    } else {
        Err(FootprintError::OffSky)
    }
}

fn bounds(pixels: &[(f64, f64)]) -> [f64; 4] {
    pixels.iter().fold([f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY], |[x0, x1, y0, y1], &(x, y)| {
        [x0.min(x), x1.max(x), y0.min(y), y1.max(y)]
    })
}

pub fn polygon_rect(polygon: &[SkyPoint], wcs: &WcsTransform) -> Result<CutoutRect, FootprintError> {
    let [x_min, x_max, y_min, y_max] = bounds(&pixels_of(wcs, &densified(polygon)?)?);
    Ok(pixel_bounds_rect(x_min, x_max, y_min, y_max))
}

fn header_wcs_of(header: &HduHeader) -> Result<WcsTransform, FootprintError> {
    header_wcs(header).map_err(|error| FootprintError::Wcs(format!("{error:#}")))
}

pub fn lightstack_wcs(header: &HduHeader) -> Result<WcsTransform, FootprintError> {
    header_wcs_of(header)
}

pub(crate) fn without_sip(header: &HduHeader) -> HduHeader {
    let mut linear = header.clone();
    for key in SIP_ORDER_CARDS {
        linear.remove(key);
    }
    linear
}

pub fn lightstack_linear_wcs(header: &HduHeader) -> Result<WcsTransform, FootprintError> {
    header_wcs_of(&without_sip(header))
}

pub fn lightstack_footprint_bbox(wcs: &WcsTransform, dims: (usize, usize)) -> Result<SkyBbox, FootprintError> {
    if dims.0 == 0 || dims.1 == 0 {
        return Err(FootprintError::Empty);
    }
    let (right, top) = ((dims.0 - 1) as f64, (dims.1 - 1) as f64);
    let corners = sky(wcs, &[(0.0, 0.0), (0.0, top), (right, top), (right, 0.0)])?;
    let pick = |value: fn(&SkyPoint) -> f64, fold: fn(f64, f64) -> f64| corners.iter().map(value).reduce(fold).unwrap_or(f64::NAN);
    let bbox = SkyBbox {
        ra_min: pick(|c| c.ra, f64::min),
        ra_max: pick(|c| c.ra, f64::max),
        dec_min: pick(|c| c.dec, f64::min),
        dec_max: pick(|c| c.dec, f64::max),
    };
    if bbox.ra_max - bbox.ra_min > 180.0 {
        return Err(FootprintError::RaWrap);
    }
    Ok(bbox)
}

pub fn lightstack_common_bbox(bboxes: &[SkyBbox]) -> Result<SkyBbox, FootprintError> {
    let first = bboxes.first().ok_or(FootprintError::Empty)?;
    let common = bboxes.iter().skip(1).fold(*first, |acc, b| SkyBbox {
        ra_min: acc.ra_min.max(b.ra_min),
        ra_max: acc.ra_max.min(b.ra_max),
        dec_min: acc.dec_min.max(b.dec_min),
        dec_max: acc.dec_max.min(b.dec_max),
    });
    if common.ra_min >= common.ra_max || common.dec_min >= common.dec_max {
        return Err(FootprintError::NoCommonFootprint);
    }
    Ok(common)
}

pub fn lightstack_common_region(boxes: &[SkyBox]) -> Result<SkyBox, FootprintError> {
    let limits: Vec<SkyBbox> = boxes
        .iter()
        .map(|b| {
            let (half_width, half_height) = (b.width_arcsec / ARCSEC_PER_DEG / 2.0, b.height_arcsec / ARCSEC_PER_DEG / 2.0);
            SkyBbox {
                ra_min: b.center.ra - half_width,
                ra_max: b.center.ra + half_width,
                dec_min: b.center.dec - half_height,
                dec_max: b.center.dec + half_height,
            }
        })
        .collect();
    if limits.is_empty() {
        return Err(FootprintError::NoBoxes);
    }
    let common = lightstack_common_bbox(&limits).map_err(|_| FootprintError::NoCommonRegion)?;
    Ok(SkyBox {
        center: SkyPoint { ra: (common.ra_min + common.ra_max) / 2.0, dec: (common.dec_min + common.dec_max) / 2.0 },
        width_arcsec: (common.ra_max - common.ra_min) * ARCSEC_PER_DEG,
        height_arcsec: (common.dec_max - common.dec_min) * ARCSEC_PER_DEG,
        angle_deg: 0.0,
    })
}

fn clipped_rect(x_min: i64, x_max: i64, y_min: i64, y_max: i64, dims: (usize, usize)) -> Result<CutoutRect, FootprintError> {
    let (x_min, x_max) = (x_min.max(0), x_max.min(dims.0 as i64));
    let (y_min, y_max) = (y_min.max(0), y_max.min(dims.1 as i64));
    if x_max <= x_min || y_max <= y_min {
        return Err(FootprintError::OutsideImage { width: dims.0, height: dims.1 });
    }
    Ok(CutoutRect { x0: x_min, y0: y_min, width: (x_max - x_min) as usize, height: (y_max - y_min) as usize })
}

pub fn lightstack_square_rect(
    wcs: &WcsTransform,
    dims: (usize, usize),
    center: SkyPoint,
    size_arcsec: f64,
) -> Result<CutoutRect, FootprintError> {
    let [(x_center, y_center)] = pixels_of(wcs, &[center])?[..] else {
        return Err(FootprintError::OffSky);
    };
    let pixscale = wcs.pixel_scale_arcsec();
    if !(pixscale.is_finite() && pixscale > 0.0) {
        return Err(FootprintError::DegenerateScale);
    }
    let half_size_pix = (size_arcsec / pixscale) / 2.0;
    clipped_rect(
        (x_center - half_size_pix).floor() as i64,
        (x_center + half_size_pix).ceil() as i64,
        (y_center - half_size_pix).floor() as i64,
        (y_center + half_size_pix).ceil() as i64,
        dims,
    )
}

pub fn lightstack_bbox_rect(linear_wcs: &WcsTransform, dims: (usize, usize), bbox: &SkyBbox) -> Result<CutoutRect, FootprintError> {
    let corners = [
        SkyPoint { ra: bbox.ra_min, dec: bbox.dec_min },
        SkyPoint { ra: bbox.ra_max, dec: bbox.dec_min },
        SkyPoint { ra: bbox.ra_max, dec: bbox.dec_max },
        SkyPoint { ra: bbox.ra_min, dec: bbox.dec_max },
    ];
    let [x_min, x_max, y_min, y_max] = bounds(&pixels_of(linear_wcs, &corners)?);
    clipped_rect(x_min.floor() as i64, x_max.ceil() as i64, y_min.floor() as i64, y_max.ceil() as i64, dims)
}

fn point_north_of(center: SkyPoint, offset_arcsec: f64) -> SkyPoint {
    let dec = center.dec + offset_arcsec / ARCSEC_PER_DEG;
    if dec > 90.0 {
        SkyPoint { ra: (center.ra + 180.0).rem_euclid(360.0), dec: 180.0 - dec }
    } else {
        SkyPoint { ra: center.ra, dec }
    }
}

pub fn lightstack_region_rect(wcs: &WcsTransform, sky_box: &SkyBox) -> Result<CutoutRect, FootprintError> {
    let north = point_north_of(sky_box.center, LIGHTSTACK_NORTH_OFFSET_ARCSEC);
    let [(x, y), (xn, yn)] = pixels_of(wcs, &[sky_box.center, north])?[..] else {
        return Err(FootprintError::OffSky);
    };
    let pixels_per_arcsec = (xn - x).hypot(yn - y) / LIGHTSTACK_NORTH_OFFSET_ARCSEC;
    if !(pixels_per_arcsec.is_finite() && pixels_per_arcsec > 0.0) {
        return Err(FootprintError::DegenerateScale);
    }
    let north_angle = (yn - y).atan2(xn - x);
    let (sin, cos) = (sky_box.angle_deg.to_radians() + north_angle - FRAC_PI_2).sin_cos();
    let (w2, h2) = (sky_box.width_arcsec * pixels_per_arcsec / 2.0, sky_box.height_arcsec * pixels_per_arcsec / 2.0);
    let dx = (w2 * cos - h2 * sin).abs().max((w2 * cos + h2 * sin).abs());
    let dy = (w2 * sin + h2 * cos).abs().max((w2 * sin - h2 * cos).abs());
    let (ix_min, ix_max) = ((x - dx + 0.5).floor() as i64, (x + dx + 0.5).ceil() as i64);
    let (iy_min, iy_max) = ((y - dy + 0.5).floor() as i64, (y + dy + 0.5).ceil() as i64);
    if ix_max <= ix_min || iy_max <= iy_min {
        return Err(FootprintError::DegenerateScale);
    }
    Ok(CutoutRect { x0: ix_min, y0: iy_min, width: (ix_max - ix_min) as usize, height: (iy_max - iy_min) as usize })
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionBoxes {
    pub boxes: Vec<(usize, SkyBox)>,
    pub count: usize,
    pub warnings: Vec<String>,
}

pub fn sky_boxes_from_reg(text: &str, wcs: &WcsTransform) -> Result<RegionBoxes, FootprintError> {
    let region_error = |error: crate::core::imaging::region::RegionError| FootprintError::Region(error.to_string());
    let parsed = parse_reg_with_physical(text, Some(wcs), &PhysicalMap::identity()).map_err(region_error)?;
    if let Some(system) = parsed.systems.iter().find(|system| !system.is_sky()) {
        return Err(FootprintError::Region(format!(
            "crop boxes must be given in fk5 or icrs coordinates, the file uses {} coordinates",
            system.name()
        )));
    }
    let mut warnings = parsed.warnings.clone();
    let mut boxes = Vec::new();
    for (index, region) in parsed.regions.iter().enumerate() {
        match shape_to_sky(&region.shape, RegionSystem::Icrs, Some(wcs)).map_err(region_error)? {
            RegionShape::Box { x, y, width, height, angle } => boxes.push((
                index,
                SkyBox { center: SkyPoint { ra: x, dec: y }, width_arcsec: width, height_arcsec: height, angle_deg: angle },
            )),
            other => warnings.push(format!("region {index}: {} ignored, only boxes define a crop", other.kind())),
        }
    }
    Ok(RegionBoxes { boxes, count: parsed.regions.len(), warnings })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum CropRegion {
    Radec { ra: f64, dec: f64, size_arcsec: f64 },
    Reference { reference: String },
    Common,
    Region { region_file: String, #[serde(default)] index: usize },
    RegionCommon { region_file: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropRounding {
    #[default]
    Astroburst,
    Lightstack,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CropShape {
    Polygon { vertices: Vec<SkyPoint> },
    LightstackSquare { center: SkyPoint, size_arcsec: f64 },
    LightstackBbox { bbox: SkyBbox },
    LightstackRegion { sky_box: SkyBox },
}

#[derive(Clone, Copy)]
pub struct CropFrame<'a> {
    pub header: &'a HduHeader,
    pub wcs: &'a WcsTransform,
    pub dims: (usize, usize),
}

impl CropShape {
    pub fn rect(&self, frame: &CropFrame) -> Result<CutoutRect, FootprintError> {
        let rect = match self {
            Self::Polygon { vertices } => polygon_rect(vertices, frame.wcs)?,
            Self::LightstackSquare { center, size_arcsec } => {
                lightstack_square_rect(&lightstack_wcs(frame.header)?, frame.dims, *center, *size_arcsec)?
            }
            Self::LightstackBbox { bbox } => lightstack_bbox_rect(&lightstack_linear_wcs(frame.header)?, frame.dims, bbox)?,
            Self::LightstackRegion { sky_box } => lightstack_region_rect(&lightstack_wcs(frame.header)?, sky_box)?,
        };
        if fraction_on_image(&rect, frame.dims.0, frame.dims.1) <= 0.0 {
            return Err(FootprintError::OutsideImage { width: frame.dims.0, height: frame.dims.1 });
        }
        Ok(rect)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCrop {
    pub shape: CropShape,
    pub warnings: Vec<String>,
}

fn region_boxes(inputs: &[CropFrame], region_text: Option<&str>) -> Result<RegionBoxes, FootprintError> {
    let text = region_text.ok_or(FootprintError::Missing("region", "the region file text"))?;
    sky_boxes_from_reg(text, inputs[0].wcs)
}

pub fn resolve_crop_shape(
    region: &CropRegion,
    rounding: CropRounding,
    inputs: &[CropFrame],
    reference: Option<&CropFrame>,
    region_text: Option<&str>,
) -> Result<ResolvedCrop, FootprintError> {
    if inputs.is_empty() {
        return Err(FootprintError::Empty);
    }
    let lightstack = rounding == CropRounding::Lightstack;
    let mut warnings = Vec::new();
    let shape = match region {
        CropRegion::Radec { ra, dec, size_arcsec } => {
            let center = SkyPoint { ra: *ra, dec: *dec };
            if lightstack {
                CropShape::LightstackSquare { center, size_arcsec: *size_arcsec }
            } else {
                CropShape::Polygon { vertices: square_polygon(center, *size_arcsec) }
            }
        }
        CropRegion::Reference { .. } => {
            let frame = reference.ok_or(FootprintError::Missing("reference", "the reference image"))?;
            if lightstack {
                CropShape::LightstackBbox { bbox: lightstack_footprint_bbox(&lightstack_wcs(frame.header)?, frame.dims)? }
            } else {
                CropShape::Polygon { vertices: image_footprint(frame.wcs, frame.dims)? }
            }
        }
        CropRegion::Common => {
            if lightstack {
                let bboxes = inputs
                    .iter()
                    .map(|frame| lightstack_footprint_bbox(&lightstack_wcs(frame.header)?, frame.dims))
                    .collect::<Result<Vec<_>, _>>()?;
                CropShape::LightstackBbox { bbox: lightstack_common_bbox(&bboxes)? }
            } else {
                let footprints =
                    inputs.iter().map(|frame| image_footprint(frame.wcs, frame.dims)).collect::<Result<Vec<_>, _>>()?;
                CropShape::Polygon { vertices: common_polygon(&footprints)? }
            }
        }
        CropRegion::Region { index, .. } => {
            let parsed = region_boxes(inputs, region_text)?;
            warnings.extend(parsed.warnings);
            let (_, sky_box) = *parsed
                .boxes
                .iter()
                .find(|(position, _)| position == index)
                .ok_or(FootprintError::NotABox { index: *index, count: parsed.count })?;
            if lightstack {
                CropShape::LightstackRegion { sky_box }
            } else {
                CropShape::Polygon { vertices: box_polygon(&sky_box) }
            }
        }
        CropRegion::RegionCommon { .. } => {
            let parsed = region_boxes(inputs, region_text)?;
            warnings.extend(parsed.warnings);
            let boxes: Vec<SkyBox> = parsed.boxes.iter().map(|(_, sky_box)| *sky_box).collect();
            if boxes.is_empty() {
                return Err(FootprintError::NoBoxes);
            }
            if lightstack {
                CropShape::LightstackRegion { sky_box: lightstack_common_region(&boxes)? }
            } else {
                let polygons: Vec<Vec<SkyPoint>> = boxes.iter().map(box_polygon).collect();
                let common = common_polygon(&polygons).map_err(|error| match error {
                    FootprintError::NoCommonFootprint => FootprintError::NoCommonRegion,
                    other => other,
                })?;
                CropShape::Polygon { vertices: common }
            }
        }
    };
    Ok(ResolvedCrop { shape, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::cutout::{fraction_on_image, pixel_bounds_rect};

    const ARCSEC: f64 = 1.0 / 3600.0;

    fn frame_header(dims: (usize, usize), crpix: (f64, f64), crval: (f64, f64), scale_arcsec: f64, rotation_deg: f64) -> HduHeader {
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let s = scale_arcsec * ARCSEC;
        let mut header = HduHeader::empty();
        for (key, value) in [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN")] {
            header.set(key, value.to_string());
        }
        header.set("NAXIS1", dims.0.to_string());
        header.set("NAXIS2", dims.1.to_string());
        for (key, value) in [
            ("CRPIX1", crpix.0),
            ("CRPIX2", crpix.1),
            ("CRVAL1", crval.0),
            ("CRVAL2", crval.1),
            ("CD1_1", -s * cos),
            ("CD1_2", -s * sin),
            ("CD2_1", -s * sin),
            ("CD2_2", s * cos),
        ] {
            header.set_f64(key, value);
        }
        header
    }

    fn wcs_of(header: &HduHeader) -> WcsTransform {
        WcsTransform::from_header(header).unwrap()
    }

    fn sky_at(wcs: &WcsTransform, x: f64, y: f64) -> SkyPoint {
        let c = wcs.pixel_to_world(x, y);
        SkyPoint { ra: c.ra, dec: c.dec }
    }

    fn rect(x0: i64, y0: i64, width: usize, height: usize) -> CutoutRect {
        CutoutRect { x0, y0, width, height }
    }

    fn inside_image(wcs: &WcsTransform, dims: (usize, usize), point: SkyPoint, margin: f64) -> Option<bool> {
        let (x, y) = wcs.world_to_pixel(point.ra, point.dec);
        let edges = [x + 0.5, dims.0 as f64 - 0.5 - x, y + 0.5, dims.1 as f64 - 0.5 - y];
        if edges.iter().any(|d| d.abs() < margin) {
            None
        } else {
            Some(edges.iter().all(|d| *d > 0.0))
        }
    }

    #[test]
    fn common_footprint_of_rotated_images_is_their_polygon_intersection() {
        let crval = (150.0, 2.0);
        let straight = frame_header((64, 64), (32.5, 32.5), crval, 0.1, 0.0);
        let turned = frame_header((64, 64), (32.5, 32.5), crval, 0.1, 45.0);
        let (a, b) = (wcs_of(&straight), wcs_of(&turned));
        let footprints = vec![image_footprint(&a, (64, 64)).unwrap(), image_footprint(&b, (64, 64)).unwrap()];
        assert!((polygon_area_arcsec2(&footprints[0]) - 6.4 * 6.4).abs() < 1e-9 * 40.96, "{}", polygon_area_arcsec2(&footprints[0]));

        let common = common_polygon(&footprints).unwrap();
        let octagon = (2.0 * 2f64.sqrt() - 2.0) * 6.4 * 6.4;
        let area = polygon_area_arcsec2(&common);
        assert!((area - octagon).abs() < 1e-9 * octagon, "area {area} against the octagon {octagon}");

        let tangent = Tangent::at(SkyPoint { ra: crval.0, dec: crval.1 });
        let mut checked = 0;
        for i in -14..=14 {
            for j in -14..=14 {
                let point = tangent.deproject([0.37 * i as f64, 0.37 * j as f64]);
                let (Some(in_a), Some(in_b)) = (inside_image(&a, (64, 64), point, 1e-6), inside_image(&b, (64, 64), point, 1e-6)) else {
                    continue;
                };
                assert_eq!(polygon_contains(&common, point), in_a && in_b, "({i}, {j})");
                checked += 1;
            }
        }
        assert!(checked > 800, "{checked}");

        let far = frame_header((64, 64), (32.5, 32.5), (150.0, 2.01), 0.1, 0.0);
        let apart = vec![footprints[0].clone(), image_footprint(&wcs_of(&far), (64, 64)).unwrap()];
        assert_eq!(common_polygon(&apart), Err(FootprintError::NoCommonFootprint));
    }

    #[test]
    fn the_common_footprint_crosses_ra_zero() {
        let west = frame_header((64, 64), (32.5, 32.5), (360.0 - 1.8 * ARCSEC, 0.0), 0.1, 0.0);
        let east = frame_header((64, 64), (32.5, 32.5), (1.8 * ARCSEC, 0.0), 0.1, 0.0);
        let (a, b) = (wcs_of(&west), wcs_of(&east));
        let common = common_polygon(&[image_footprint(&a, (64, 64)).unwrap(), image_footprint(&b, (64, 64)).unwrap()]).unwrap();
        let area = polygon_area_arcsec2(&common);
        assert!((area - 2.8 * 6.4).abs() < 1e-6, "{area}");
        assert!(polygon_contains(&common, SkyPoint { ra: 0.0, dec: 0.0 }));
        assert!(common.iter().all(|p| p.ra < 1e-3 || p.ra > 360.0 - 1e-3), "{common:?}");
        assert_eq!(polygon_rect(&common, &a).unwrap(), rect(0, 0, 28, 64));
        assert_eq!(polygon_rect(&common, &b).unwrap(), rect(36, 0, 28, 64));
    }

    #[test]
    fn lightstack_rounding_refuses_a_footprint_across_ra_zero() {
        let west = frame_header((64, 64), (32.5, 32.5), (360.0 - 1.8 * ARCSEC, 0.0), 0.1, 0.0);
        let east = frame_header((64, 64), (32.5, 32.5), (1.8 * ARCSEC, 0.0), 0.1, 0.0);
        let (a, b) = (wcs_of(&west), wcs_of(&east));
        let frames = [CropFrame { header: &west, wcs: &a, dims: (64, 64) }, CropFrame { header: &east, wcs: &b, dims: (64, 64) }];
        let shape = |region: &CropRegion, rounding: CropRounding, reference: Option<&CropFrame>| {
            resolve_crop_shape(region, rounding, &frames, reference, None).map(|resolved| resolved.shape)
        };
        assert_eq!(shape(&CropRegion::Common, CropRounding::Lightstack, None), Err(FootprintError::RaWrap));
        let reference = CropRegion::Reference { reference: "west.fits".into() };
        assert_eq!(shape(&reference, CropRounding::Lightstack, Some(&frames[0])), Err(FootprintError::RaWrap));
        assert!(FootprintError::RaWrap.to_string().contains("astroburst rounding"));
        assert!(matches!(shape(&CropRegion::Common, CropRounding::Astroburst, None), Ok(CropShape::Polygon { .. })));

        let clear = frame_header((64, 64), (32.5, 32.5), (10.0 * ARCSEC, 0.0), 0.1, 0.0);
        let bbox = lightstack_footprint_bbox(&wcs_of(&clear), (64, 64)).unwrap();
        assert!(bbox.ra_min > 0.0 && bbox.ra_max - bbox.ra_min < 7.0 * ARCSEC, "{bbox:?}");
    }

    #[test]
    fn lightstack_rounding_reproduces_crop_from_radec_bounds() {
        let header = frame_header((100, 80), (50.5, 40.5), (150.0, 2.0), 0.05, 0.0);
        let wcs = wcs_of(&header);
        let center = sky_at(&wcs, 31.3, 40.7);
        assert_eq!(lightstack_square_rect(&wcs, (100, 80), center, 0.5).unwrap(), rect(26, 35, 11, 11));
        let corner = sky_at(&wcs, 2.2, 78.4);
        assert_eq!(lightstack_square_rect(&wcs, (100, 80), corner, 0.5).unwrap(), rect(0, 73, 8, 7));
        let outside = sky_at(&wcs, 130.0, 40.0);
        assert_eq!(
            lightstack_square_rect(&wcs, (100, 80), outside, 0.5),
            Err(FootprintError::OutsideImage { width: 100, height: 80 })
        );

        let mut oblong = header.clone();
        oblong.set_f64("CD1_1", -0.04 * ARCSEC);
        oblong.set_f64("CD2_2", 0.06 * ARCSEC);
        let oblong = wcs_of(&oblong);
        assert_eq!(lightstack_square_rect(&oblong, (100, 80), sky_at(&oblong, 31.3, 40.7), 0.5).unwrap(), rect(26, 35, 11, 11));

        let astroburst = CropShape::Polygon { vertices: square_polygon(center, 0.5) };
        let frame = CropFrame { header: &header, wcs: &wcs, dims: (100, 80) };
        assert_eq!(astroburst.rect(&frame).unwrap(), rect(27, 36, 10, 10));
        let lightstack = CropShape::LightstackSquare { center, size_arcsec: 0.5 };
        assert_eq!(lightstack.rect(&frame).unwrap(), rect(26, 35, 11, 11));
    }

    #[test]
    fn ds9_box_intersection_uses_cos_dec() {
        let ra0 = 210.0;
        let left = SkyBox { center: SkyPoint { ra: ra0, dec: -60.0 }, width_arcsec: 36.0, height_arcsec: 36.0, angle_deg: 0.0 };
        let right = SkyBox { center: SkyPoint { ra: ra0 + 0.0101, dec: -60.0 }, ..left };
        let common = common_polygon(&[box_polygon(&left), box_polygon(&right)]).unwrap();
        let area = polygon_area_arcsec2(&common);
        assert!((area - (36.0 - 0.0101 * 3600.0 * 0.5) * 36.0).abs() < 0.1, "{area}");
        let ras: Vec<f64> = common.iter().map(|p| p.ra).collect();
        let (lo, hi) = (ras.iter().copied().fold(f64::INFINITY, f64::min), ras.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        assert!((lo - (ra0 + 0.0001)).abs() < 1e-5 && (hi - (ra0 + 0.01)).abs() < 1e-5, "{lo} {hi}");
        assert_eq!(lightstack_common_region(&[left, right]), Err(FootprintError::NoCommonRegion));

        let nearer = SkyBox { center: SkyPoint { ra: ra0 + 0.004, dec: -60.0 }, ..left };
        let lightstack = lightstack_common_region(&[left, nearer]).unwrap();
        assert!((lightstack.center.ra - (ra0 + 0.002)).abs() < 1e-12, "{lightstack:?}");
        assert!((lightstack.width_arcsec - 21.6).abs() < 1e-9 && (lightstack.height_arcsec - 36.0).abs() < 1e-9, "{lightstack:?}");
        assert_eq!(lightstack.angle_deg, 0.0);
        let truth = polygon_area_arcsec2(&common_polygon(&[box_polygon(&left), box_polygon(&nearer)]).unwrap());
        assert!((truth - (36.0 - 0.004 * 3600.0 * 0.5) * 36.0).abs() < 0.1, "{truth}");

        let turned = SkyBox { angle_deg: 45.0, center: SkyPoint { ra: ra0, dec: -60.0 }, ..left };
        let octagon = polygon_area_arcsec2(&common_polygon(&[box_polygon(&left), box_polygon(&turned)]).unwrap());
        assert!((octagon - (2.0 * 2f64.sqrt() - 2.0) * 36.0 * 36.0).abs() < 1e-3, "{octagon}");
    }

    #[test]
    fn lightstack_reference_crop_reads_corner_pixel_centres_through_the_linear_wcs() {
        let reference = frame_header((40, 30), (20.5, 15.5), (150.0, 2.0), 0.1, 0.0);
        let reference_wcs = wcs_of(&reference);
        let bbox = lightstack_footprint_bbox(&reference_wcs, (40, 30)).unwrap();
        let centres = [(0.0, 0.0), (0.0, 29.0), (39.0, 29.0), (39.0, 0.0)].map(|(x, y)| sky_at(&reference_wcs, x, y));
        let fold = |values: [f64; 4], pick: fn(f64, f64) -> f64| values.into_iter().reduce(pick).unwrap();
        assert_eq!(bbox.ra_min, fold(centres.map(|c| c.ra), f64::min));
        assert_eq!(bbox.ra_max, fold(centres.map(|c| c.ra), f64::max));
        assert_eq!(bbox.dec_min, fold(centres.map(|c| c.dec), f64::min));
        assert_eq!(bbox.dec_max, fold(centres.map(|c| c.dec), f64::max));
        assert!(bbox.ra_max < sky_at(&reference_wcs, -0.5, -0.5).ra && bbox.dec_min > sky_at(&reference_wcs, -0.5, -0.5).dec);

        let mut target = frame_header((100, 100), (50.5, 50.5), (150.0, 2.0), 0.05, 0.0);
        let linear = wcs_of(&target);
        target.set("CTYPE1", "RA---TAN-SIP".to_string());
        target.set("CTYPE2", "DEC--TAN-SIP".to_string());
        target.set_f64("A_ORDER", 2.0);
        target.set_f64("A_2_0", 1.0e-3);
        target.set_f64("B_ORDER", 2.0);
        target.set_f64("B_0_2", -1.0e-3);
        let full = wcs_of(&target);
        let bounds = |wcs: &WcsTransform| {
            let pixels: Vec<(f64, f64)> = [(bbox.ra_min, bbox.dec_min), (bbox.ra_max, bbox.dec_min), (bbox.ra_max, bbox.dec_max), (bbox.ra_min, bbox.dec_max)]
                .iter()
                .map(|&(ra, dec)| wcs.world_to_pixel(ra, dec))
                .collect();
            let x0 = pixels.iter().map(|p| p.0).fold(f64::INFINITY, f64::min).floor().max(0.0) as i64;
            let x1 = (pixels.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max).ceil() as i64).min(100);
            let y0 = pixels.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).floor().max(0.0) as i64;
            let y1 = (pixels.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max).ceil() as i64).min(100);
            rect(x0, y0, (x1 - x0) as usize, (y1 - y0) as usize)
        };
        let got = lightstack_bbox_rect(&lightstack_linear_wcs(&target).unwrap(), (100, 100), &bbox).unwrap();
        assert_eq!(got, bounds(&linear));
        assert_ne!(got, bounds(&full), "the SIP terms move the corners by more than a pixel");
        let frame = CropFrame { header: &target, wcs: &full, dims: (100, 100) };
        assert_eq!(CropShape::LightstackBbox { bbox }.rect(&frame).unwrap(), got);

        let other = frame_header((40, 30), (21.5, 14.5), (150.0, 2.0), 0.1, 0.0);
        let common = lightstack_common_bbox(&[bbox, lightstack_footprint_bbox(&wcs_of(&other), (40, 30)).unwrap()]).unwrap();
        assert!((common.ra_min - bbox.ra_min - 0.1 * ARCSEC / 2f64.to_radians().cos()).abs() < 1e-9, "{common:?}");
        assert!((common.ra_max - bbox.ra_max).abs() < 1e-12 && (common.dec_max - bbox.dec_max).abs() < 1e-12, "{common:?}");
        assert!((common.dec_min - bbox.dec_min - 0.1 * ARCSEC).abs() < 1e-9, "{common:?}");
        let disjoint = frame_header((40, 30), (20.5, 15.5), (150.0, 2.1), 0.1, 0.0);
        assert_eq!(
            lightstack_common_bbox(&[bbox, lightstack_footprint_bbox(&wcs_of(&disjoint), (40, 30)).unwrap()]),
            Err(FootprintError::NoCommonFootprint)
        );
    }

    #[test]
    fn lightstack_region_bounds_follow_the_regions_bounding_box() {
        let header = frame_header((100, 100), (50.5, 50.5), (150.0, 2.0), 1.0, 0.0);
        let wcs = wcs_of(&header);
        let center = sky_at(&wcs, 40.2, 30.6);
        let plain = SkyBox { center, width_arcsec: 10.0, height_arcsec: 6.0, angle_deg: 0.0 };
        assert_eq!(lightstack_region_rect(&wcs, &plain).unwrap(), rect(35, 28, 11, 7));
        let quarter = SkyBox { angle_deg: 90.0, ..plain };
        assert_eq!(lightstack_region_rect(&wcs, &quarter).unwrap(), rect(37, 26, 7, 11));
        let edge = SkyBox { center: sky_at(&wcs, 1.0, 1.0), width_arcsec: 6.0, height_arcsec: 6.0, angle_deg: 0.0 };
        assert_eq!(lightstack_region_rect(&wcs, &edge).unwrap(), rect(-2, -2, 7, 7));

        let turned_header = frame_header((100, 100), (50.5, 50.5), (150.0, 2.0), 1.0, 30.0);
        let turned = wcs_of(&turned_header);
        let (cx, cy) = turned.world_to_pixel(center.ra, center.dec);
        let (sin, cos) = 30f64.to_radians().sin_cos();
        let (w2, h2) = (5.0, 3.0);
        let dx = (w2 * cos - h2 * sin).abs().max((w2 * cos + h2 * sin).abs());
        let dy = (w2 * sin + h2 * cos).abs().max((w2 * sin - h2 * cos).abs());
        let x0 = (cx - dx + 0.5).floor() as i64;
        let y0 = (cy - dy + 0.5).floor() as i64;
        let expected = rect(x0, y0, ((cx + dx + 0.5).ceil() as i64 - x0) as usize, ((cy + dy + 0.5).ceil() as i64 - y0) as usize);
        assert_eq!(lightstack_region_rect(&turned, &plain).unwrap(), expected);
    }

    const REG_TEXT: &str = "# Region file format: DS9 version 4.1\nglobal color=green\nfk5\nbox(150.0,2.0,10\",6\",30) # text={a}\ncircle(150.001,2.001,2\")\nicrs\nbox(150.002,2.0005,0.002,0.001,0)\n";

    #[test]
    fn a_reg_file_gives_icrs_boxes_and_skips_other_shapes() {
        for rotation in [0.0, 30.0] {
            let header = frame_header((100, 100), (50.5, 50.5), (150.0, 2.0), 0.1, rotation);
            let parsed = sky_boxes_from_reg(REG_TEXT, &wcs_of(&header)).unwrap();
            assert_eq!(parsed.count, 3);
            assert_eq!(parsed.boxes.iter().map(|(index, _)| *index).collect::<Vec<_>>(), vec![0, 2]);
            assert_eq!(parsed.warnings.len(), 1, "{:?}", parsed.warnings);
            assert!(parsed.warnings[0].contains("circle"), "{:?}", parsed.warnings);
            let (_, fk5) = parsed.boxes[0];
            assert!((fk5.center.ra - 150.0).abs() < 0.1 * ARCSEC && (fk5.center.dec - 2.0).abs() < 0.1 * ARCSEC, "{fk5:?}");
            assert!((fk5.width_arcsec - 10.0).abs() < 1e-6 && (fk5.height_arcsec - 6.0).abs() < 1e-6, "{fk5:?}");
            assert!((fk5.angle_deg - 30.0).abs() < 1e-6, "rotation {rotation}: {fk5:?}");
            let (_, icrs) = parsed.boxes[1];
            assert!((icrs.center.ra - 150.002).abs() < 1e-9 && (icrs.center.dec - 2.0005).abs() < 1e-9, "{icrs:?}");
            assert!((icrs.width_arcsec - 7.2).abs() < 1e-6 && (icrs.height_arcsec - 3.6).abs() < 1e-6 && icrs.angle_deg.abs() < 1e-6, "{icrs:?}");
        }
        let header = frame_header((100, 100), (50.5, 50.5), (150.0, 2.0), 0.1, 0.0);
        let image = sky_boxes_from_reg("image\nbox(10,10,4,4,0)\n", &wcs_of(&header)).unwrap_err();
        assert!(matches!(&image, FootprintError::Region(message) if message.contains("image")), "{image}");
    }

    #[test]
    fn astroburst_rounding_covers_the_polygon_with_whole_pixels() {
        let header = frame_header((100, 100), (50.5, 50.5), (150.0, 2.0), 1.0, 0.0);
        let wcs = wcs_of(&header);
        let frame = CropFrame { header: &header, wcs: &wcs, dims: (100, 100) };
        let square = |x: f64, y: f64, size: f64| CropShape::Polygon { vertices: square_polygon(sky_at(&wcs, x, y), size) };
        assert_eq!(square(40.2, 30.6, 10.0).rect(&frame).unwrap(), rect(36, 26, 10, 10));
        let overhang = square(1.3, 1.6, 10.0).rect(&frame).unwrap();
        assert_eq!(overhang, rect(-3, -3, 10, 10));
        assert!(fraction_on_image(&overhang, 100, 100) < 1.0);
        assert_eq!(square(200.0, 200.0, 10.0).rect(&frame), Err(FootprintError::OutsideImage { width: 100, height: 100 }));

        let turned_header = frame_header((100, 100), (50.5, 50.5), (150.0, 2.0), 1.0, 30.0);
        let turned = wcs_of(&turned_header);
        let center = sky_at(&wcs, 40.2, 30.6);
        let (cx, cy) = turned.world_to_pixel(center.ra, center.dec);
        let half = 5.0 * (30f64.to_radians().cos() + 30f64.to_radians().sin());
        let got = polygon_rect(&square_polygon(center, 10.0), &turned).unwrap();
        assert_eq!(got, pixel_bounds_rect(cx - half, cx + half, cy - half, cy + half));
    }

    #[test]
    fn the_crop_shape_follows_the_mode_and_the_rounding() {
        let header = frame_header((64, 64), (32.5, 32.5), (150.0, 2.0), 0.1, 0.0);
        let other_header = frame_header((64, 64), (30.5, 34.5), (150.0, 2.0), 0.1, 10.0);
        let (wcs, other) = (wcs_of(&header), wcs_of(&other_header));
        let frames = [CropFrame { header: &header, wcs: &wcs, dims: (64, 64) }, CropFrame { header: &other_header, wcs: &other, dims: (64, 64) }];
        let radec = CropRegion::Radec { ra: 150.0, dec: 2.0, size_arcsec: 2.0 };
        let shape = |region: &CropRegion, rounding: CropRounding, reference: Option<&CropFrame>, text: Option<&str>| {
            resolve_crop_shape(region, rounding, &frames, reference, text).map(|resolved| resolved.shape)
        };
        assert!(matches!(shape(&radec, CropRounding::Astroburst, None, None), Ok(CropShape::Polygon { vertices }) if vertices.len() == 4));
        assert_eq!(
            shape(&radec, CropRounding::Lightstack, None, None),
            Ok(CropShape::LightstackSquare { center: SkyPoint { ra: 150.0, dec: 2.0 }, size_arcsec: 2.0 })
        );
        let Ok(CropShape::Polygon { vertices }) = shape(&CropRegion::Common, CropRounding::Astroburst, None, None) else {
            panic!("the common footprint is a polygon");
        };
        assert!(polygon_contains(&vertices, SkyPoint { ra: 150.0, dec: 2.0 }));
        assert!(polygon_area_arcsec2(&vertices) < 6.4 * 6.4);
        assert!(matches!(shape(&CropRegion::Common, CropRounding::Lightstack, None, None), Ok(CropShape::LightstackBbox { .. })));

        let reference = CropRegion::Reference { reference: "ref.fits".into() };
        assert_eq!(shape(&reference, CropRounding::Astroburst, None, None), Err(FootprintError::Missing("reference", "the reference image")));
        let Ok(CropShape::Polygon { vertices }) = shape(&reference, CropRounding::Astroburst, Some(&frames[1]), None) else {
            panic!("the reference footprint is a polygon");
        };
        assert!((polygon_area_arcsec2(&vertices) - 6.4 * 6.4).abs() < 1e-6);

        let region = CropRegion::Region { region_file: "boxes.reg".into(), index: 2 };
        assert_eq!(shape(&region, CropRounding::Astroburst, None, None), Err(FootprintError::Missing("region", "the region file text")));
        assert!(matches!(shape(&region, CropRounding::Lightstack, None, Some(REG_TEXT)), Ok(CropShape::LightstackRegion { sky_box }) if (sky_box.width_arcsec - 7.2).abs() < 1e-6));
        let not_a_box = CropRegion::Region { region_file: "boxes.reg".into(), index: 1 };
        assert_eq!(shape(&not_a_box, CropRounding::Astroburst, None, Some(REG_TEXT)), Err(FootprintError::NotABox { index: 1, count: 3 }));
        let common = CropRegion::RegionCommon { region_file: "boxes.reg".into() };
        assert!(matches!(shape(&common, CropRounding::Astroburst, None, Some(REG_TEXT)), Ok(CropShape::Polygon { .. })));
        assert!(matches!(shape(&common, CropRounding::Lightstack, None, Some(REG_TEXT)), Ok(CropShape::LightstackRegion { sky_box }) if sky_box.angle_deg == 0.0));
        assert_eq!(resolve_crop_shape(&radec, CropRounding::Astroburst, &[], None, None), Err(FootprintError::Empty));
    }
}
