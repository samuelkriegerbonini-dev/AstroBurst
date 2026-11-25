use serde::{Deserialize, Serialize};

use crate::core::astrometry::wcs::{angular_separation, pixel_edge_corners, WcsTransform};
use crate::core::imaging::geometry::GeometryOp;
use crate::types::header::HduHeader;

use super::error::MaskError;

pub const WCS_MATCH_TOLERANCE: f64 = 1e-9;
pub const WCS_CORNER_TOLERANCE_PX: f64 = 0.5;
const TAN_CTYPES: [&str; 2] = ["RA---TAN", "DEC--TAN"];
const ARCSEC_PER_DEG: f64 = 3600.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WcsSummary {
    pub crval: [f64; 2],
    pub crpix: [f64; 2],
    pub cd: [[f64; 2]; 2],
    pub naxis: [usize; 2],
}

type RenderField = fn(&WcsSummary) -> String;

impl WcsSummary {
    pub fn of(wcs: &WcsTransform, (rows, cols): (usize, usize)) -> Self {
        let (crpix1, crpix2, crval1, crval2, cd, _) = wcs.raw_params();
        Self { crval: [crval1, crval2], crpix: [crpix1, crpix2], cd, naxis: [cols, rows] }
    }

    pub fn mismatch(&self, target: &Self, own: &str, other: &str) -> Option<String> {
        let fields: [(&str, bool, RenderField); 4] = [
            ("NAXIS", self.naxis == target.naxis, |s| format!("{}×{}", s.naxis[0], s.naxis[1])),
            ("CRPIX", close(&self.crpix, &target.crpix, 1.0), |s| format!("({:.6}, {:.6})", s.crpix[0], s.crpix[1])),
            ("CRVAL", close(&self.crval, &target.crval, 1.0), |s| format!("({:.8}, {:.8})", s.crval[0], s.crval[1])),
            ("CD", close(&self.cd.concat(), &target.cd.concat(), 0.0), |s| {
                format!("({:.6e}, {:.6e}, {:.6e}, {:.6e})", s.cd[0][0], s.cd[0][1], s.cd[1][0], s.cd[1][1])
            }),
        ];
        let differing: Vec<&(&str, bool, RenderField)> = fields.iter().filter(|(_, same, _)| !same).collect();
        if differing.is_empty() {
            return None;
        }
        let names: Vec<&str> = differing.iter().map(|(name, _, _)| *name).collect();
        let show = |summary: &Self| {
            differing.iter().map(|(name, _, render)| format!("{name} {}", render(summary))).collect::<Vec<_>>().join(", ")
        };
        Some(format!("WCS summary differs in {}: {} on {own}, {} on {other}", join_with_and(&names), show(self), show(target)))
    }

    fn tan_transform(&self) -> Option<WcsTransform> {
        let mut header = HduHeader::empty();
        let cards = [
            ("NAXIS1", self.naxis[0].to_string()),
            ("NAXIS2", self.naxis[1].to_string()),
            ("CTYPE1", TAN_CTYPES[0].to_string()),
            ("CTYPE2", TAN_CTYPES[1].to_string()),
            ("CRPIX1", self.crpix[0].to_string()),
            ("CRPIX2", self.crpix[1].to_string()),
            ("CRVAL1", self.crval[0].to_string()),
            ("CRVAL2", self.crval[1].to_string()),
            ("CD1_1", self.cd[0][0].to_string()),
            ("CD1_2", self.cd[0][1].to_string()),
            ("CD2_1", self.cd[1][0].to_string()),
            ("CD2_2", self.cd[1][1].to_string()),
        ];
        for (key, value) in cards {
            header.set(key, value);
        }
        WcsTransform::from_header(&header).ok()
    }

    pub fn corner_offset_px(&self, target: &Self) -> Option<f64> {
        let (drawn, image) = (self.tan_transform()?, target.tan_transform()?);
        let arcsec_per_px = image.pixel_scale_arcsec();
        pixel_edge_corners(target.naxis[0], target.naxis[1]).into_iter().try_fold(0.0_f64, |worst, (x, y)| {
            let (on_mask, on_image) = (drawn.pixel_to_world(x, y), image.pixel_to_world(x, y));
            let offset = angular_separation(on_mask.ra, on_mask.dec, on_image.ra, on_image.dec) * ARCSEC_PER_DEG / arcsec_per_px;
            offset.is_finite().then_some(worst.max(offset))
        })
    }
}

fn close(a: &[f64], b: &[f64], floor: f64) -> bool {
    let scale = a.iter().chain(b).fold(floor, |scale, v| scale.max(v.abs()));
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= WCS_MATCH_TOLERANCE * scale)
}

fn join_with_and(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => only.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub stem: String,
    pub geometry: Option<GeometryOp>,
    pub wcs: Option<WcsSummary>,
}

fn geometry_name(geometry: Option<GeometryOp>) -> &'static str {
    geometry.map_or("none", GeometryOp::name)
}

pub fn check_file_grid(mask: &Grid, image: &Grid) -> Result<(), MaskError> {
    let refused = |detail: String| MaskError::FileGrid { mask: mask.stem.clone(), image: image.stem.clone(), detail };
    if mask.geometry != image.geometry {
        return Err(refused(format!(
            "geometry step {} on {}, {} on {}",
            geometry_name(image.geometry),
            image.stem,
            geometry_name(mask.geometry),
            mask.stem
        )));
    }
    match (&mask.wcs, &image.wcs) {
        (Some(drawn), Some(target)) => drawn.mismatch(target, &mask.stem, &image.stem).map_or(Ok(()), |detail| Err(refused(detail))),
        _ => Ok(()),
    }
}

pub fn check_regions_grid(source_wcs: Option<&WcsSummary>, image: &Grid) -> Result<(), MaskError> {
    let Some(drawn) = source_wcs else {
        return Ok(());
    };
    let refused = |detail: String| Err(MaskError::RegionsGrid { image: image.stem.clone(), detail });
    let Some(target) = &image.wcs else {
        return refused(format!("{} has no WCS", image.stem));
    };
    let Some(mismatch) = drawn.mismatch(target, "the mask", &image.stem) else {
        return Ok(());
    };
    if drawn.naxis != target.naxis {
        return refused(mismatch);
    }
    match drawn.corner_offset_px(target) {
        Some(offset) if offset <= WCS_CORNER_TOLERANCE_PX => Ok(()),
        Some(offset) => refused(format!(
            "{mismatch}; the image corners are {offset:.2} px apart, over the {WCS_CORNER_TOLERANCE_PX} px limit"
        )),
        None => refused(mismatch),
    }
}
