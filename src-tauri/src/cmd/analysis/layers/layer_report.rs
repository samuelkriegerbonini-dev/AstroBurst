use crate::service::area::holds_one_plane;
use crate::cmd::common::{load_cached, load_cached_full, load_companions, source_path};
use crate::core::imaging::dq_flags::exclusion_map;
use crate::core::imaging::layer_roles::{classify_layer, LayerRole};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::band_planes::band_plane_of_path;
use crate::infra::fits::dispatcher::list_source_extensions;
use crate::infra::fits::reader::HduInfo;
use crate::infra::image_source::ErrOrigin;
use crate::types::image_ref::PlaneSelector;
use crate::cmd::analysis::DqMask;

pub(crate) const DQ_NOT_APPLIED_PREFIX: &str = "DQ exclusion requested but not applied: ";

pub(crate) struct PhotometryPlanes {
    pub err: Option<ImageEntry>,
    pub saturated: Option<ndarray::Array2<u8>>,
}

pub(crate) fn photometry_planes(path: &str, dims: (usize, usize)) -> PhotometryPlanes {
    let comps = match load_companions(path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("companion planes unavailable for {}: {:#}", path, e);
            return PhotometryPlanes { err: None, saturated: None };
        }
    };
    let saturation = comps.saturated_mask();
    let err = comps.err.filter(|entry| entry.arr().dim() == dims);
    let saturated = comps.dq.filter(|_| saturation != 0).and_then(|(entry, _)| {
        let plane = entry.int_plane()?;
        (plane.bits.dim() == dims).then(|| exclusion_map(&plane.bits, saturation))
    });
    PhotometryPlanes { err, saturated }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LayerReport {
    pub err_source: Option<String>,
    pub dq_source: Option<String>,
    pub dq_flags: Option<String>,
    pub dq_exclusion_available: bool,
    pub dq_gap: Option<String>,
    pub err_note: Option<String>,
    pub err_gap: Option<String>,
}

fn plane_label(plane: &PlaneSelector) -> String {
    match plane {
        PlaneSelector::Hdu(index) => format!("HDU {index}"),
        PlaneSelector::Array(key) => format!("array {key}"),
        PlaneSelector::Auto => "primary image".to_string(),
    }
}

fn err_origin_text(origin: &ErrOrigin) -> String {
    match origin.n_no_data {
        0 => origin.source.clone(),
        1 => format!("{}, 1 pixel without data", origin.source),
        n => format!("{}, {n} pixels without data", origin.source),
    }
}

pub(crate) fn layer_report(path: &str, entry: &ImageEntry) -> LayerReport {
    let dims = entry.arr().dim();
    let references = entry.companions();
    let dq_reference = references
        .and_then(|c| c.dq.as_ref())
        .map(|r| format!("DQ plane ({})", plane_label(&r.plane)));
    let loaded = match load_companions(path) {
        Ok(loaded) => loaded,
        Err(e) => {
            return LayerReport {
                dq_source: dq_reference,
                dq_gap: Some(format!("the companion planes could not be loaded ({e:#})")),
                ..LayerReport::default()
            };
        }
    };
    let err_source = match &loaded.err_origin {
        Some(origin) => Some(err_origin_text(origin)),
        None => references
            .and_then(|c| c.err.as_ref())
            .filter(|_| loaded.err.as_ref().is_some_and(|plane| plane.arr().dim() == dims))
            .map(|r| format!("ERR plane ({})", plane_label(&r.plane))),
    };
    let err_note = loaded
        .err_origin
        .as_ref()
        .filter(|origin| origin.role != LayerRole::Sigma || origin.assumed || origin.n_no_data > 0)
        .map(|origin| format!("error layer: {}", err_origin_text(origin)));
    let dq_flags = match &loaded.flags {
        Some(flags) => Some(flags.source.clone()),
        None => loaded.dq.as_ref().map(|(_, table)| table.label().to_string()),
    };
    let dq_exclusion_available = loaded.exclusion_mask() != 0
        && loaded
            .dq
            .as_ref()
            .is_some_and(|(companion, _)| companion.int_plane().is_some_and(|plane| plane.bits.dim() == dims));
    let dq_gap = match (&loaded.dq_unavailable, &loaded.flags, &dq_reference) {
        _ if dq_exclusion_available => None,
        (Some(reason), _, _) => Some(reason.clone()),
        (None, Some(flags), _) if !flags.is_named() => Some(format!(
            "the flag layer has no usable bit names ({}), so nothing is excluded until bits are chosen",
            flags.source
        )),
        (None, Some(flags), _) if flags.exclusion_mask == 0 => {
            Some(format!("no flag of this layer is selected for exclusion ({})", flags.source))
        }
        (None, _, None) => Some("this image has no DQ plane".to_string()),
        (None, _, Some(source)) => Some(format!("the {source} could not be read as an integer mask of the image size")),
    };
    LayerReport {
        err_source,
        dq_source: loaded.dq_origin.or(dq_reference),
        dq_flags,
        dq_exclusion_available,
        dq_gap,
        err_note,
        err_gap: loaded.err_unavailable,
    }
}

pub(in crate::cmd::analysis) fn no_data_exclusion(path: &str, planes: &PhotometryPlanes, mask: Option<&DqMask>) -> Option<ndarray::Array2<u8>> {
    let sigma = planes.err.as_ref()?.arr();
    load_companions(path).ok()?.err_origin.filter(|origin| origin.n_no_data > 0)?;
    let mut excluded = mask.map_or_else(|| ndarray::Array2::zeros(sigma.dim()), |dq| dq.map.clone());
    ndarray::Zip::from(&mut excluded).and(sigma).for_each(|flag, value| {
        if value.is_nan() {
            *flag = 1;
        }
    });
    Some(excluded)
}

pub(in crate::cmd::analysis) fn uncalibrated_layer_reason(path: &str, entry: &ImageEntry) -> Option<String> {
    let layer = entry.plane_info().and_then(|info| info.pairing.layer.clone()).or_else(|| {
        let source = source_path(path);
        let file_name = std::path::Path::new(&source).file_name().and_then(|name| name.to_str()).unwrap_or("");
        entry.header().map(|header| classify_layer(header, file_name))
    })?;
    let kind = match layer.role {
        LayerRole::Mask => "a mask",
        LayerRole::Variance => "a variance",
        LayerRole::InverseVariance => "an inverse-variance",
        LayerRole::Science | LayerRole::Sigma | LayerRole::Other => return None,
    };
    Some(format!("no calibration applied: this plane is {kind} layer ({})", layer.basis))
}

pub(crate) fn dq_not_applied_warning(report: &LayerReport) -> Option<String> {
    report.dq_gap.as_ref().map(|gap| format!("{DQ_NOT_APPLIED_PREFIX}{gap}"))
}

pub(crate) fn has_err_extension(path: &str, dims: (usize, usize)) -> bool {
    let Ok(entry) = load_cached_full(path).or_else(|_| load_cached(path)) else {
        return false;
    };
    let Some(reference) = entry.companions().and_then(|c| c.err.as_ref()) else {
        return false;
    };
    let (rows, cols) = dims;
    let fits_the_image = |h: &HduInfo| h.naxis1 == cols as i64 && h.naxis2 == rows as i64;
    let listed = |file: &str| list_source_extensions(file).unwrap_or_default();
    match (&reference.plane, band_plane_of_path(&reference.path)) {
        (PlaneSelector::Array(_), _) => false,
        (_, Some(band)) => listed(&band.file)
            .iter()
            .any(|h| h.index == band.hdu && h.has_data && h.naxis == 3 && fits_the_image(h) && (band.plane as i64) < h.naxis3),
        (PlaneSelector::Hdu(index), None) => {
            listed(&reference.path).iter().any(|h| h.index == *index && holds_one_plane(h) && fits_the_image(h))
        }
        (PlaneSelector::Auto, None) => {
            listed(&reference.path).iter().find(|h| h.has_data).is_some_and(|h| holds_one_plane(h) && fits_the_image(h))
        }
    }
}
