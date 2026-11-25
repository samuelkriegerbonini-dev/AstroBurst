use crate::core::astrometry::wcs::WcsTransform;

pub(crate) fn axis_scales_arcsec(wcs: &WcsTransform) -> (f64, f64) {
    axis_scales_of_cd(wcs.raw_params().4)
}

pub(crate) fn pixel_area_scale_arcsec(wcs: &WcsTransform) -> Option<f64> {
    area_scale_of_cd(wcs.raw_params().4)
}

const PIXEL_SCALE_ANISOTROPY_TOLERANCE: f64 = 0.01;

pub(crate) fn axis_scale_warning(wcs: &WcsTransform) -> Option<String> {
    let (sx, sy) = axis_scales_arcsec(wcs);
    let s = pixel_area_scale_arcsec(wcs)?;
    ((sx - sy).abs() / sx.max(sy) > PIXEL_SCALE_ANISOTROPY_TOLERANCE).then(|| {
        format!(
            "pixel scales differ per axis ({sx:.5} x {sy:.5} arcsec); radii in arcsec use the square root of the pixel area ({s:.5} arcsec)"
        )
    })
}

fn axis_scales_of_cd(cd: [[f64; 2]; 2]) -> (f64, f64) {
    (cd[0][0].hypot(cd[1][0]) * 3600.0, cd[0][1].hypot(cd[1][1]) * 3600.0)
}

pub(in crate::cmd::analysis) fn area_scale_of_cd(cd: [[f64; 2]; 2]) -> Option<f64> {
    let scale = (cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]).abs().sqrt() * 3600.0;
    (scale.is_finite() && scale > 0.0).then_some(scale)
}
