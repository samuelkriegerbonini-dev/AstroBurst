use anyhow::Result;

use crate::core::analysis::gain::{CAMERA_GAIN_KEYWORD, GAIN_KEYWORDS};
use crate::core::analysis::photometry::SATURATION_KEYWORDS;
use crate::core::history::stamp::FileStamp;
use crate::core::metadata::photcal::{
    GENERIC_ZERO_POINT_KEYS, ROMAN_CONVERSION_MJY_KEYS, ROMAN_PIXEL_AREA_SR_KEYS,
    SURVEY_ZERO_POINT_KEYS,
};
use crate::core::ops::OutputValues;
use crate::infra::cache::{ImageCache, ImageEntry, PlaneLoad};
use crate::infra::fits::writer::is_layout_card;
use crate::infra::image_source::{
    load_companions_into, load_plane, load_plane_header, LoadedCompanions, LoadedPlane,
};
use crate::types::constants::{
    HEADER_COMBINE_METHOD, HEADER_DISPLAY_REFERRED_CARD, HEADER_DRIZZLE_SCALE, HEADER_NCOMBINE,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

pub const HEADER_ABPROC: &str = "ABPROC";
pub const HEADER_DISPLAY_REFERRED: &str = HEADER_DISPLAY_REFERRED_CARD;

const DEFAULT_ABPROC: &str = "processed";

const DERIVED_STRUCTURAL_CARDS: &[&str] = &[
    "EXTNAME", "EXTVER", "EXTLEVEL", "INHERIT", "DATAMIN", "DATAMAX",
];
const AXIS_INDEXED_PREFIXES: &[&str] = &[
    "NAXIS", "CTYPE", "CRVAL", "CRPIX", "CDELT", "CUNIT", "CROTA",
];
const AXIS_MATRIX_PREFIXES: &[&str] = &["CD", "PC"];
const AXIS_PARAMETER_PREFIXES: &[&str] = &["PV", "PS"];
const CALIBRATION_CARDS: &[&str] = &[
    "BUNIT",
    "PIXAR_SR",
    "PIXAR_A2",
    "ZP",
    "ZPTMAG",
    CAMERA_GAIN_KEYWORD,
    HEADER_NCOMBINE,
    HEADER_COMBINE_METHOD,
    HEADER_DRIZZLE_SCALE,
];
const CALIBRATION_PREFIXES: &[&str] = &["PHOT", "ROMAN_META_PHOTOMETRY"];

pub fn plane_load(r: &ImageRef) -> Result<PlaneLoad> {
    load_plane(r).map(LoadedPlane::into_plane_load)
}

fn load_plane_entry(cache: &ImageCache, key: &str) -> Result<ImageEntry> {
    cache.get_or_load_plane(key, || plane_load(&ImageRef::parse(key)))
}

pub(crate) fn load_cached_with(
    cache: &ImageCache,
    path: &str,
    load: impl FnOnce(&str) -> Result<ImageEntry>,
) -> Result<(ImageEntry, Option<FileStamp>)> {
    let before = cache.revalidate_source(path);
    if let Some(entry) = cache.get(path) {
        return Ok((entry, before));
    }
    let entry = load(path)?;
    cache.forget_if_rewritten(path, before);
    Ok((entry, before))
}

pub fn load_cached(cache: &ImageCache, path: &str) -> Result<ImageEntry> {
    load_cached_with(cache, path, |key| load_plane_entry(cache, key)).map(|(entry, _)| entry)
}

pub fn load_cached_full(cache: &ImageCache, path: &str) -> Result<ImageEntry> {
    let before = cache.revalidate_source(path);
    if let Some(entry) = cache.get(path) {
        if entry.header().is_some() {
            return Ok(entry);
        }
        if let Ok(upgraded) =
            cache.upgrade_header(path, || load_plane_header(&ImageRef::parse(path)))
        {
            cache.forget_if_rewritten(path, before);
            return Ok(upgraded);
        }
    }
    let entry = load_plane_entry(cache, path)?;
    cache.forget_if_rewritten(path, before);
    Ok(entry)
}

pub fn load_companions(cache: &ImageCache, path: &str) -> Result<LoadedCompanions> {
    let (active, before) = load_cached_with(cache, path, |key| load_plane_entry(cache, key))?;
    let companions = load_companions_into(cache, &active, plane_load);
    cache.forget_if_rewritten(path, before);
    Ok(companions)
}

pub fn cached_header(cache: &ImageCache, path: &str) -> Result<HduHeader> {
    cache.revalidate_source(path);
    match cache.get(path).and_then(|entry| entry.header().cloned()) {
        Some(header) => Ok(header),
        None => load_plane_header(&ImageRef::parse(path)),
    }
}

fn indexed_number(digits: &str) -> Option<usize> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn indexed(key: &str, prefix: &str) -> Option<usize> {
    key.strip_prefix(prefix).and_then(indexed_number)
}

fn axis_pair(key: &str, prefix: &str) -> Option<(usize, usize)> {
    let (i, j) = key.strip_prefix(prefix)?.split_once('_')?;
    Some((indexed_number(i)?, indexed_number(j)?))
}

fn describes_a_higher_axis(key: &str) -> bool {
    AXIS_INDEXED_PREFIXES
        .iter()
        .any(|p| indexed(key, p).is_some_and(|n| n >= 3))
        || AXIS_MATRIX_PREFIXES
            .iter()
            .any(|p| axis_pair(key, p).is_some_and(|(i, j)| i >= 3 || j >= 3))
        || AXIS_PARAMETER_PREFIXES
            .iter()
            .any(|p| axis_pair(key, p).is_some_and(|(i, _)| i >= 3))
}

fn is_structural_card(key: &str) -> bool {
    is_layout_card(key) || DERIVED_STRUCTURAL_CARDS.contains(&key) || describes_a_higher_axis(key)
}

fn is_calibration_card(key: &str) -> bool {
    CALIBRATION_CARDS.contains(&key)
        || GAIN_KEYWORDS.contains(&key)
        || CALIBRATION_PREFIXES.iter().any(|p| key.starts_with(p))
        || GENERIC_ZERO_POINT_KEYS.contains(&key)
        || SURVEY_ZERO_POINT_KEYS.contains(&key)
        || SATURATION_KEYWORDS.contains(&key)
        || ROMAN_CONVERSION_MJY_KEYS.contains(&key)
        || ROMAN_PIXEL_AREA_SR_KEYS.contains(&key)
}

fn dropped_from_derived_output(key: &str, values: OutputValues) -> bool {
    is_structural_card(key) || (values != OutputValues::Linear && is_calibration_card(key))
}

pub fn derived_output_header(
    source: Option<&HduHeader>,
    abproc: &str,
    values: OutputValues,
) -> HduHeader {
    let mut header = source.cloned().unwrap_or_else(HduHeader::empty);
    let doomed: Vec<String> = header
        .cards
        .iter()
        .map(|(k, _)| k)
        .chain(header.index.keys())
        .filter(|k| dropped_from_derived_output(k.trim(), values))
        .cloned()
        .collect();
    for key in doomed {
        header.remove(&key);
    }
    if header.get("WCSAXES").is_some() {
        header.set("WCSAXES", "2".to_string());
    }
    let abproc = if abproc.trim().is_empty() {
        DEFAULT_ABPROC
    } else {
        abproc
    };
    header.set(HEADER_ABPROC, abproc.to_string());
    match values {
        OutputValues::DisplayReferred => header.set(HEADER_DISPLAY_REFERRED, "T".to_string()),
        OutputValues::Rescaled => header.remove(HEADER_DISPLAY_REFERRED),
        OutputValues::Linear => {}
    }
    header
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use ndarray::Array2;

    use super::*;
    use crate::core::analysis::gain::{gain_model, PoissonRoute};
    use crate::infra::cache::GLOBAL_IMAGE_CACHE;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;
    use crate::infra::fits::writer::write_fits_mono;

    fn private_cache() -> ImageCache {
        ImageCache::new(8, 64 * 1024 * 1024)
    }

    fn rewrite(path: &str, value: f32) {
        write_fits_mono(path, &Array2::from_elem((4, 4), value), None).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(5))
            .unwrap();
    }

    fn header_with(cards: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for (k, v) in cards {
            header.set(k, v.to_string());
        }
        header
    }

    #[test]
    fn the_loaders_fill_the_cache_they_are_given_and_leave_the_global_cache_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sci_err_dq.fits");
        sci_err_dq_mef(&path, 4, 4, vec![0; 16]);
        let file = path.to_str().unwrap().to_string();
        let sci = format!("{file}#hdu=1");
        let cache = private_cache();

        assert_eq!(load_cached(&cache, &sci).unwrap().arr().dim(), (4, 4));
        assert_eq!(
            load_cached_full(&cache, &sci)
                .unwrap()
                .header()
                .and_then(|h| h.get("EXTNAME")),
            Some("SCI")
        );
        assert_eq!(
            cached_header(&cache, &format!("{file}#hdu=2"))
                .unwrap()
                .get("EXTNAME"),
            Some("ERR")
        );
        let companions = load_companions(&cache, &sci).unwrap();
        assert!(companions.err.is_some(), "the ERR companion was not found");
        assert!(companions.dq.is_some(), "the DQ companion was not found");
        for key in [sci, format!("{file}#hdu=2"), format!("{file}#hdu=3")] {
            assert!(
                cache.contains(&key),
                "{key} is not in the cache the loader was given"
            );
            assert!(
                !GLOBAL_IMAGE_CACHE.contains(&key),
                "{key} leaked into the desktop cache"
            );
        }
    }

    #[test]
    fn each_cache_notices_a_rewrite_even_after_another_cache_reloaded_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.fits").to_str().unwrap().to_string();
        write_fits_mono(&path, &Array2::zeros((4, 4)), None).unwrap();
        let (first, second) = (private_cache(), private_cache());
        assert_eq!(load_cached(&first, &path).unwrap().arr()[[0, 0]], 0.0);
        assert_eq!(load_cached(&second, &path).unwrap().arr()[[0, 0]], 0.0);

        rewrite(&path, 1.0);
        assert_eq!(load_cached(&second, &path).unwrap().arr()[[0, 0]], 1.0);
        assert_eq!(
            load_cached(&first, &path).unwrap().arr()[[0, 0]],
            1.0,
            "the first cache served the pixels from before the rewrite"
        );
    }

    #[test]
    fn derived_output_header_removes_structure_and_keeps_calibration_only_for_linear_values() {
        let source = header_with(&[
            ("XTENSION", "IMAGE"),
            ("EXTNAME", "SCI"),
            ("PCOUNT", "0"),
            ("ZIMAGE", "T"),
            ("ZTILE1", "64"),
            ("ZCMPTYPE", "RICE_1"),
            ("TFIELDS", "1"),
            ("TTYPE1", "COMPRESSED_DATA"),
            ("TFORM1", "1PB"),
            ("TBCOL1", "1"),
            ("THEAP", "0"),
            ("CHECKSUM", "abc"),
            ("DATASUM", "1"),
            ("DATAMIN", "0"),
            ("DATAMAX", "1"),
            ("NAXIS3", "5"),
            ("WCSAXES", "3"),
            ("CTYPE3", "WAVE"),
            ("CD3_3", "0.1"),
            ("PC1_3", "0"),
            ("PV3_1", "0"),
            ("CRVAL1", "10.5"),
            ("CD1_1", "-0.0001"),
            ("PV2_1", "45.0"),
            ("TELESCOP", "JWST"),
            ("BUNIT", "MJy/sr"),
            ("PHOTMJSR", "1.5"),
            ("PIXAR_SR", "2.1E-13"),
            ("ZPT", "25.0"),
            ("ZEROPT", "25.0"),
            ("PHOTZP", "25.0"),
            ("MAGZERO", "30.0"),
            ("SATURATE", "60000"),
        ]);

        let linear = derived_output_header(Some(&source), "deconv", OutputValues::Linear);
        for dropped in [
            "XTENSION", "EXTNAME", "PCOUNT", "ZIMAGE", "ZTILE1", "ZCMPTYPE", "TFIELDS", "TTYPE1",
            "TFORM1", "TBCOL1", "THEAP", "CHECKSUM", "DATASUM", "DATAMIN", "DATAMAX", "NAXIS3",
            "CTYPE3", "CD3_3", "PC1_3", "PV3_1",
        ] {
            assert!(linear.get(dropped).is_none(), "{dropped} survived");
            assert!(
                !linear.cards.iter().any(|(k, _)| k == dropped),
                "{dropped} card survived"
            );
        }
        for kept in [
            "CRVAL1", "CD1_1", "PV2_1", "TELESCOP", "BUNIT", "PHOTMJSR", "PIXAR_SR", "ZPT",
            "SATURATE",
        ] {
            assert!(
                linear.get(kept).is_some(),
                "{kept} dropped from a flux-preserving output"
            );
        }
        assert_eq!(linear.get("WCSAXES"), Some("2"));
        assert_eq!(linear.get(HEADER_ABPROC), Some("deconv"));
        assert!(linear.get(HEADER_DISPLAY_REFERRED).is_none());

        let rescaled = derived_output_header(Some(&source), "pixelmath", OutputValues::Rescaled);
        for dropped in [
            "BUNIT", "PHOTMJSR", "PIXAR_SR", "ZPT", "ZEROPT", "PHOTZP", "MAGZERO", "SATURATE",
        ] {
            assert!(
                rescaled.get(dropped).is_none(),
                "{dropped} survived a rescaling"
            );
        }
        assert_eq!(rescaled.get("CRVAL1"), Some("10.5"));
        assert_eq!(rescaled.get("TELESCOP"), Some("JWST"));

        let stretched =
            derived_output_header(Some(&source), "arcsinh", OutputValues::DisplayReferred);
        assert_eq!(stretched.get(HEADER_DISPLAY_REFERRED), Some("T"));
        assert!(stretched.get("BUNIT").is_none());
        let denoised = derived_output_header(Some(&stretched), "denoised", OutputValues::Linear);
        assert_eq!(
            denoised.get(HEADER_DISPLAY_REFERRED),
            Some("T"),
            "a linear step keeps display-referred values"
        );
        let remapped = derived_output_header(Some(&stretched), "pixelmath", OutputValues::Rescaled);
        assert!(remapped.get(HEADER_DISPLAY_REFERRED).is_none());

        let bare = derived_output_header(None, "  ", OutputValues::Rescaled);
        assert_eq!(bare.get(HEADER_ABPROC), Some("processed"));
    }

    #[test]
    fn a_stack_processed_by_background_keeps_its_combine_scaling() {
        let stack = header_with(&[
            ("NCOMBINE", "16"),
            ("ABCOMB", "mean"),
            ("EGAIN", "0.25"),
            ("ABPROC", "stacked"),
        ]);
        let out = derived_output_header(Some(&stack), "bg_corrected", OutputValues::Linear);
        assert_eq!(out.get(HEADER_ABPROC), Some("bg_corrected"));
        for kept in ["NCOMBINE", "ABCOMB", "EGAIN"] {
            assert!(
                out.get(kept).is_some(),
                "{kept} dropped from a flux-preserving output"
            );
        }
        let model = gain_model(Some(&out), false);
        assert_eq!(model.effective_gain, Some(4.0), "{model:?}");
        assert!(model.combine_scaled);
        assert_eq!(model.poisson_route, PoissonRoute::HeaderGain);
    }

    #[test]
    fn rescaled_and_display_referred_outputs_lose_gain_and_combine_cards() {
        let source = header_with(&[
            ("EGAIN", "0.25"),
            ("GAIN", "120"),
            ("NCOMBINE", "16"),
            ("ABCOMB", "mean"),
            ("ABDRZSCL", "2"),
            ("ATODGAIN", "7"),
            ("CCDGAIN", "2"),
            ("ADCGAIN", "5"),
        ]);
        let cards = [
            "EGAIN", "GAIN", "NCOMBINE", "ABCOMB", "ABDRZSCL", "ATODGAIN", "CCDGAIN", "ADCGAIN",
        ];
        for values in [OutputValues::Rescaled, OutputValues::DisplayReferred] {
            let out = derived_output_header(Some(&source), "pixelmath", values);
            for card in cards {
                assert!(
                    out.get(card).is_none(),
                    "{card} survived a {values:?} output"
                );
                assert!(
                    !out.cards.iter().any(|(k, _)| k == card),
                    "{card} card survived a {values:?} output"
                );
            }
            assert_eq!(
                gain_model(Some(&out), false).effective_gain,
                None,
                "{values:?}"
            );
        }
        let linear = derived_output_header(Some(&source), "deconv", OutputValues::Linear);
        for card in cards {
            assert!(
                linear.get(card).is_some(),
                "{card} dropped from a flux-preserving output"
            );
        }
        assert_eq!(gain_model(Some(&linear), false).effective_gain, Some(1.0));
    }
}
