use std::fs::File;
#[cfg(not(test))]
use std::sync::{LazyLock, Mutex};

use anyhow::Result;

use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::imaging::push_history;
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::cache::{GLOBAL_IMAGE_CACHE, ImageEntry};
use crate::infra::fits::band_planes::band_plane_of_path;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::extract_header_by_index;
use crate::infra::fits::writer::{filter_header, write_mef_with_tables, BinTableHdu, HduData, ImageHdu};
use crate::infra::image_source::LoadedCompanions;
use crate::types::constants::WIZARD_CACHE_PREFIX;
use crate::types::header::{is_commentary_key, HduHeader};
use crate::types::image_ref::{sanitize_fragment, ImageRef, OutputStems};

const HEADER_PROGRAM: &str = "PROGRAM";
const READABLE_WIZARD_STEM_PREFIX: &str = "wizard_";
const PLANE_ONLY_KEYS: &[&str] = &[
    "BUNIT", "EXTNAME", "EXTVER", "LTV1", "LTV2", "LTM1_1", "LTM2_2", "BZERO", "BSCALE", "BLANK", "DATAMIN", "DATAMAX",
    "BSOFTEN", "BOFFSET",
];

pub(crate) fn source_path(path: &str) -> String {
    let path = ImageRef::parse(path).path;
    band_plane_of_path(&path).map_or(path, |band| band.file)
}

#[cfg(not(test))]
static OUTPUT_STEMS: LazyLock<Mutex<OutputStems>> = LazyLock::new(|| Mutex::new(OutputStems::default()));

#[cfg(not(test))]
fn with_output_stems<T>(f: impl FnOnce(&mut OutputStems) -> T) -> T {
    f(&mut OUTPUT_STEMS.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
thread_local! {
    static TEST_ISOLATED_OUTPUT_STEMS: std::cell::RefCell<OutputStems> = std::cell::RefCell::new(OutputStems::default());
}

#[cfg(test)]
fn with_output_stems<T>(f: impl FnOnce(&mut OutputStems) -> T) -> T {
    TEST_ISOLATED_OUTPUT_STEMS.with(|stems| f(&mut stems.borrow_mut()))
}

fn is_run_token(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn readable_cache_stem(key: &str) -> Option<String> {
    let rest = key.strip_prefix(WIZARD_CACHE_PREFIX)?;
    let bin_stage = rest
        .strip_prefix('t')
        .and_then(|tail| tail.split_once('_'))
        .filter(|(token, tail)| is_run_token(token) && tail.trim_start_matches('_').contains('_'))
        .map_or(rest, |(_, tail)| tail);
    (!bin_stage.is_empty()).then(|| format!("{READABLE_WIZARD_STEM_PREFIX}{}", sanitize_fragment(bin_stage)))
}

pub(crate) fn output_stem(path: &str) -> String {
    let r = ImageRef::parse(path);
    with_output_stems(|stems| match readable_cache_stem(&r.path) {
        Some(base) => stems.stem_for_with_base(&r.path, &base),
        None => r.output_stem_in(stems),
    })
}

pub(crate) fn load_cached(path: &str) -> Result<ImageEntry> {
    crate::service::common::load_cached(&GLOBAL_IMAGE_CACHE, path)
}

pub(crate) fn load_cached_full(path: &str) -> Result<ImageEntry> {
    crate::service::common::load_cached_full(&GLOBAL_IMAGE_CACHE, path)
}

pub(crate) fn load_companions(path: &str) -> Result<LoadedCompanions> {
    crate::service::common::load_companions(&GLOBAL_IMAGE_CACHE, path)
}

pub(crate) fn invalidate_written(path: &str) {
    GLOBAL_IMAGE_CACHE.invalidate_written(path);
}

fn names_program(header: Option<&HduHeader>) -> bool {
    header.is_some_and(|h| h.cards.iter().any(|(k, _)| k.trim() == HEADER_PROGRAM))
}

fn with_software_stamp(header: Option<&HduHeader>, primary_header: Option<&HduHeader>) -> HduHeader {
    let version = env!("CARGO_PKG_VERSION");
    let mut stamped = header.cloned().unwrap_or_else(HduHeader::empty);
    if !names_program(header) && !names_program(primary_header) {
        stamped.set(HEADER_PROGRAM, format!("AstroBurst {version}"));
    }
    push_history(&mut stamped, &format!("Processed with AstroBurst {version}"));
    stamped
}

fn same_data<'a>(data: &HduData<'a>) -> HduData<'a> {
    match data {
        HduData::F32(a) => HduData::F32(a),
        HduData::I32(a) => HduData::I32(a),
        HduData::U32(a) => HduData::U32(a),
        HduData::F32Cube(planes) => HduData::F32Cube(planes),
        HduData::I32Cube(planes) => HduData::I32Cube(planes),
        HduData::U32Cube(planes) => HduData::U32Cube(planes),
    }
}

pub(crate) fn write_derived_mef(path: &str, primary_header: Option<&HduHeader>, hdus: &[ImageHdu]) -> Result<()> {
    write_derived_mef_with_tables(path, primary_header, hdus, &[])
}

pub(crate) fn write_derived_mef_with_tables(
    path: &str,
    primary_header: Option<&HduHeader>,
    hdus: &[ImageHdu],
    tables: &[BinTableHdu],
) -> Result<()> {
    let stamped = hdus.first().map(|first| with_software_stamp(first.header, primary_header));
    let hdus: Vec<ImageHdu> = hdus
        .iter()
        .enumerate()
        .map(|(i, hdu)| ImageHdu {
            data: same_data(&hdu.data),
            header: if i == 0 { stamped.as_ref() } else { hdu.header },
            extname: hdu.extname,
            extver: hdu.extver,
        })
        .collect();
    GLOBAL_CUBE_CACHE.invalidate(path);
    let written = write_mef_with_tables(path, primary_header, &hdus, tables);
    invalidate_written(path);
    written
}

pub(crate) fn provenance_only(merged: &HduHeader) -> HduHeader {
    let mut out = HduHeader::empty();
    let non_wcs = filter_header(merged, false, true).unwrap_or_else(HduHeader::empty);
    for (key, value) in &non_wcs.cards {
        let key = key.trim();
        if is_commentary_key(key) {
            out.cards.push((key.to_string(), value.clone()));
        } else if !PLANE_ONLY_KEYS.contains(&key) {
            out.set(key, value.clone());
        }
    }
    out.inherit_string_keys(&non_wcs);
    out
}

pub(crate) fn source_primary_header(path: &str, merged: Option<&HduHeader>) -> HduHeader {
    let from_disk = resolve_single_image(&source_path(path)).ok().and_then(|(fits, _tmp)| {
        if is_asdf_file(&fits) {
            return None;
        }
        let file = File::open(&fits).ok()?;
        extract_header_by_index(&file, 0).ok()
    });
    from_disk.as_ref().or(merged).map(provenance_only).unwrap_or_else(HduHeader::empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_only_drops_wcs_and_plane_cards_but_keeps_observation_metadata() {
        let mut merged = HduHeader::empty();
        for (k, v) in [
            ("TELESCOP", "JWST"), ("INSTRUME", "NIRCAM"), ("DATE-OBS", "2024-01-01"), ("FILTER", "F200W"),
            ("CRPIX1", "10"), ("CD1_1", "1e-5"), ("CTYPE1", "RA---TAN"), ("BUNIT", "MJy/sr"),
            ("EXTNAME", "SCI"), ("EXTVER", "1"), ("LTV1", "-3"), ("BZERO", "0"),
        ] {
            merged.set(k, v.to_string());
        }
        let primary = provenance_only(&merged);
        assert_eq!(primary.get("TELESCOP"), Some("JWST"));
        assert_eq!(primary.get("INSTRUME"), Some("NIRCAM"));
        assert_eq!(primary.get("DATE-OBS"), Some("2024-01-01"));
        assert_eq!(primary.get("FILTER"), Some("F200W"));
        for gone in ["CRPIX1", "CD1_1", "CTYPE1", "BUNIT", "EXTNAME", "EXTVER", "LTV1", "BZERO"] {
            assert!(primary.get(gone).is_none(), "{gone}");
            assert!(!primary.cards.iter().any(|(k, _)| k == gone), "{gone}");
        }
        assert!(provenance_only(&HduHeader::empty()).cards.is_empty());
    }

    #[test]
    fn provenance_only_drops_the_asinh_cards_of_the_source_plane() {
        let mut merged = HduHeader::empty();
        for (k, v) in [("TELESCOP", "PS1"), ("BSOFTEN", "0.25"), ("BOFFSET", "3.5")] {
            merged.set(k, v.to_string());
        }
        let primary = provenance_only(&merged);
        assert_eq!(primary.get("TELESCOP"), Some("PS1"));
        for gone in ["BSOFTEN", "BOFFSET"] {
            assert!(primary.get(gone).is_none(), "{gone}");
            assert!(!primary.cards.iter().any(|(k, _)| k == gone), "{gone}");
        }
    }

    #[test]
    fn provenance_keeps_every_history_and_comment_line_out_of_the_keyword_index() {
        let mut merged = HduHeader::empty();
        merged.set("TELESCOP", "JWST".to_string());
        for (key, line) in [("HISTORY", "flat-fielded"), ("COMMENT", "reduced by the pipeline"), ("HISTORY", "drizzled")] {
            merged.cards.push((key.to_string(), line.to_string()));
        }
        let primary = provenance_only(&merged);
        let commentary: Vec<(&str, &str)> = primary
            .cards
            .iter()
            .filter(|(k, _)| is_commentary_key(k))
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            commentary,
            vec![("HISTORY", "flat-fielded"), ("COMMENT", "reduced by the pipeline"), ("HISTORY", "drizzled")]
        );
        assert!(primary.index.get("HISTORY").is_none() && primary.index.get("COMMENT").is_none());
        assert_eq!(primary.get("TELESCOP"), Some("JWST"));
    }

    #[test]
    fn the_source_path_of_a_band_plane_is_its_file() {
        assert_eq!(source_path("C:/d/cut.fits#hdu=0&plane=1"), "C:/d/cut.fits");
        assert_eq!(source_path("C:/d/cut.fits#hdu=2"), "C:/d/cut.fits");
        assert_eq!(source_path("C:/d/cut.fits"), "C:/d/cut.fits");
        assert_eq!(source_path("__composite_r"), "__composite_r");

        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("odd.fits#hdu=0&plane=1").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&odd, &ndarray::Array2::zeros((3, 4)), None).unwrap();
        assert_eq!(source_path(&odd), odd, "a file named like a plane key stays a file");
    }
}
