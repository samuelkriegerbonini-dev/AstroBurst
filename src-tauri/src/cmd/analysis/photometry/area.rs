use ndarray::Array2;

use crate::core::analysis::photometry::{area_map_mean, MaskedImage};
use crate::core::metadata::photcal::{is_relative_pixel_area, FluxConvention, PhotCal};
use crate::infra::cache::ImageEntry;
use crate::service::area::{load_area, located_area};

const PIXAR_SR_KEY: &str = "PIXAR_SR";

fn calibration_refusal(photcal: Option<&PhotCal>, entry: &ImageEntry) -> Option<String> {
    let Some(cal) = photcal else {
        return Some("the header has no photometric calibration".into());
    };
    if cal.pixar_from_header() {
        return None;
    }
    let has_pixar_card = entry
        .header()
        .and_then(|h| h.get_f64(PIXAR_SR_KEY))
        .is_some_and(|v| v.is_finite() && v > 0.0);
    Some(match &cal.convention {
        FluxConvention::JwstMjySr { .. } if has_pixar_card => {
            "PIXAR_SR was replaced by the WCS pixel area (resampled image)".into()
        }
        FluxConvention::JwstMjySr { .. } => "PIXAR_SR is missing, so the pixel area comes from the WCS".into(),
        _ => format!("{} is not JWST MJy/sr", cal.label()),
    })
}

fn ignored(reason: String) -> String {
    format!("AREA extension ignored: {reason}")
}

pub(crate) struct AreaWeights {
    map: Option<ImageEntry>,
}

impl AreaWeights {
    pub(crate) fn resolve(path: &str, entry: &ImageEntry, photcal: Option<&mut PhotCal>, warnings: &mut Vec<String>) -> Self {
        Self { map: area_map(path, entry, photcal, warnings) }
    }

    pub(crate) fn masked_image<'a>(
        &'a self,
        entry: &'a ImageEntry,
        excluded: Option<&'a Array2<u8>>,
    ) -> Result<MaskedImage<'a>, String> {
        MaskedImage::new(entry.arr(), excluded).map(|source| source.with_area(self.map.as_ref().map(|a| a.arr())))
    }

    #[cfg(test)]
    pub(crate) fn map(&self) -> Option<&Array2<f32>> {
        self.map.as_ref().map(|a| a.arr())
    }
}

fn area_map(path: &str, entry: &ImageEntry, photcal: Option<&mut PhotCal>, warnings: &mut Vec<String>) -> Option<ImageEntry> {
    let hdu = match located_area(path, entry) {
        Ok(Some(hdu)) => hdu,
        Ok(None) => return None,
        Err(reason) => {
            warnings.push(ignored(reason));
            return None;
        }
    };
    if let Some(reason) = calibration_refusal(photcal.as_deref(), entry) {
        warnings.push(ignored(reason));
        return None;
    }
    let area = match load_area(path, &hdu) {
        Ok(entry) => entry,
        Err(reason) => {
            warnings.push(ignored(reason));
            return None;
        }
    };
    let mean = area_map_mean(area.arr());
    if !is_relative_pixel_area(mean) {
        warnings.push(ignored(format!("map mean {mean:.2e} is not a relative pixel area (expected about 1)")));
        return None;
    }
    if let Some(cal) = photcal {
        cal.apply_area_map(hdu.index, mean);
    }
    Some(area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::load_from_cache_or_disk;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::header::HduHeader;

    #[test]
    fn an_area_map_is_refused_for_a_flux_density_per_pixel_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rubin_like.fits");
        let mut header = HduHeader::empty();
        header.set("BUNIT", "nJy".to_string());
        write_fits_mono(path.to_str().unwrap(), &Array2::from_elem((16, 16), 1.0f32), Some(&header)).unwrap();
        let entry = load_from_cache_or_disk(path.to_str().unwrap()).unwrap();
        let cal = PhotCal::from_header(entry.header().unwrap(), None).unwrap();
        assert_eq!(
            calibration_refusal(Some(&cal), &entry),
            Some("flux density per pixel (nJy) is not JWST MJy/sr".to_string())
        );
    }
}
