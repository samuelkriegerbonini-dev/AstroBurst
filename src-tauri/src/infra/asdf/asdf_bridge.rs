use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ndarray::Array2;
use serde_yaml::Value;

use crate::infra::asdf::converter::{
    auto_data_key, is_interleaved_layout, list_arrays, plane_geometry, shape_label,
    AsdfArrayInfo, AsdfImage, AsdfWcs,
};
use crate::core::astrometry::gwcs::{SipFit, SipFitOptions};
use crate::infra::asdf::time as asdf_time;
use crate::infra::asdf::tree::{untag, DType, WcsInfo};
use crate::infra::asdf::AsdfFile;
use crate::infra::fits::reader::{HduInfo, MmapImageResult};
use crate::types::constants::HEADER_BUNIT;
use crate::types::HduHeader;

const SCIENCE_BUNIT_KEYS: [&str; 4] = ["meta.bunit_data", "meta.bunit", "roman.meta.bunit", "header.BUNIT"];
const ERROR_BUNIT_KEYS: [&str; 1] = ["meta.bunit_err"];
const SCIENCE_ARRAY_NAMES: [&str; 4] = ["data", "sci", "science", "image"];
const NON_SCIENCE_ARRAY_NAMES: [&str; 2] = ["dq", "mask"];
const ROMAN_NODE: &str = "roman";
const ROMAN_FLUX_STEP_KEY: &str = "roman.meta.cal_step.flux";
const ROMAN_FLUX_STEP_COMPLETE: &str = "COMPLETE";
const ROMAN_LEVEL_2_IMAGE: &str = "wfi_image";
const ROMAN_MOSAIC: &str = "wfi_mosaic";
const UNIT_DN_PER_SECOND: &str = "DN/s";
const UNIT_MJY_PER_SR: &str = "MJy/sr";
const UNIT_STORED_WITH_ARRAY: &str = "unit stored with the array";

pub const ASDF_SELECTED_PLANE: &str = "ASDFPLAN";
pub const ASDF_PLANE_COUNT: &str = "ASDFNPLN";
pub const ASDF_DATA_KEY_CARD: &str = "ASDFKEY";
pub const ASDF_WCS_NOTE_CARD: &str = "ASDFWCS";
pub const ASDF_UNIT_SOURCE_CARD: &str = "ASDFUNIT";
pub const ASDF_SIP_WARNING_CARD: &str = "SIPWARN";
pub const ASDF_TIME_NOTE_CARD: &str = "ASDFTIME";
pub const ASDF_FRAME_NOTE_CARD: &str = "ASDFFRAM";
pub const ASDF_COMPLEX_NOTE_CARD: &str = "ASDFCPLX";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AliasDecode {
    Raw,
    TimeIso,
    TimeMjd,
    TimeScale,
}

struct FitsAlias {
    card: &'static str,
    sources: &'static [&'static str],
    numeric: bool,
    photometric: bool,
    decode: AliasDecode,
}

const START_TIME_SOURCES: [&str; 2] = ["meta.exposure.start_time", "roman.meta.exposure.start_time"];

const FITS_ALIASES: [FitsAlias; 14] = [
    FitsAlias {
        card: "TELESCOP",
        sources: &["meta.telescope", "roman.meta.telescope", "header.TELESCOP"],
        numeric: false,
        photometric: false,
        decode: AliasDecode::Raw,
    },
    FitsAlias {
        card: "INSTRUME",
        sources: &["meta.instrument.name", "roman.meta.instrument.name", "header.INSTRUME"],
        numeric: false,
        photometric: false,
        decode: AliasDecode::Raw,
    },
    FitsAlias {
        card: "EXPTIME",
        sources: &["meta.exposure.exposure_time", "roman.meta.exposure.exposure_time", "header.EXPTIME"],
        numeric: true,
        photometric: false,
        decode: AliasDecode::Raw,
    },
    FitsAlias {
        card: "FILTER",
        sources: &[
            "meta.instrument.filter",
            "meta.instrument.optical_element",
            "roman.meta.instrument.optical_element",
            "header.FILTER",
        ],
        numeric: false,
        photometric: false,
        decode: AliasDecode::Raw,
    },
    FitsAlias {
        card: "PUPIL",
        sources: &["meta.instrument.pupil", "header.PUPIL"],
        numeric: false,
        photometric: false,
        decode: AliasDecode::Raw,
    },
    FitsAlias {
        card: "DETECTOR",
        sources: &["meta.instrument.detector", "roman.meta.instrument.detector", "header.DETECTOR"],
        numeric: false,
        photometric: false,
        decode: AliasDecode::Raw,
    },
    FitsAlias {
        card: "DATE-OBS",
        sources: &[START_TIME_SOURCES[0], START_TIME_SOURCES[1], "meta.observation.date", "header.DATE-OBS"],
        numeric: false,
        photometric: false,
        decode: AliasDecode::TimeIso,
    },
    FitsAlias {
        card: "DATE-END",
        sources: &["meta.exposure.end_time", "roman.meta.exposure.end_time", "header.DATE-END"],
        numeric: false,
        photometric: false,
        decode: AliasDecode::TimeIso,
    },
    FitsAlias {
        card: "MJD-OBS",
        sources: &[START_TIME_SOURCES[0], START_TIME_SOURCES[1], "header.MJD-OBS"],
        numeric: true,
        photometric: false,
        decode: AliasDecode::TimeMjd,
    },
    FitsAlias {
        card: "EXPMID",
        sources: &["meta.exposure.mid_time", "roman.meta.exposure.mid_time", "header.EXPMID"],
        numeric: true,
        photometric: false,
        decode: AliasDecode::TimeMjd,
    },
    FitsAlias {
        card: "TIMESYS",
        sources: &[START_TIME_SOURCES[0], START_TIME_SOURCES[1], "meta.observation.date"],
        numeric: false,
        photometric: false,
        decode: AliasDecode::TimeScale,
    },
    FitsAlias {
        card: "PHOTMJSR",
        sources: &[
            "meta.photometry.conversion_megajanskys",
            "roman.meta.photometry.conversion_megajanskys",
            "header.PHOTMJSR",
        ],
        numeric: true,
        photometric: true,
        decode: AliasDecode::Raw,
    },
    FitsAlias {
        card: "PIXAR_SR",
        sources: &[
            "roman.meta.photometry.pixel_area",
            "meta.photometry.pixelarea_steradians",
            "roman.meta.photometry.pixelarea_steradians",
            "header.PIXAR_SR",
        ],
        numeric: true,
        photometric: true,
        decode: AliasDecode::Raw,
    },
    FitsAlias {
        card: "PIXAR_A2",
        sources: &[
            "meta.photometry.pixelarea_arcsecsq",
            "roman.meta.photometry.pixelarea_arcsecsq",
            "header.PIXAR_A2",
        ],
        numeric: true,
        photometric: true,
        decode: AliasDecode::Raw,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayRole {
    Science,
    Error,
    Other,
}

fn named(last: &str, names: &[&str]) -> bool {
    names.iter().any(|n| last.eq_ignore_ascii_case(n))
}

fn array_role(key: &str, auto_key: Option<&str>) -> ArrayRole {
    let last = key.rsplit('.').next().unwrap_or(key);
    if last.eq_ignore_ascii_case("err") {
        ArrayRole::Error
    } else if named(last, &NON_SCIENCE_ARRAY_NAMES) || last.to_ascii_lowercase().starts_with("var") {
        ArrayRole::Other
    } else if auto_key == Some(key) || named(last, &SCIENCE_ARRAY_NAMES) {
        ArrayRole::Science
    } else {
        ArrayRole::Other
    }
}

struct ResolvedUnit {
    unit: Option<String>,
    source: Option<String>,
}

fn roman_model(tree: &Value) -> Option<String> {
    let node = tree.get(ROMAN_NODE).filter(|node| untag(node).is_mapping())?;
    Some(WcsInfo::model_name(node)).filter(|name| matches!(node, Value::Tagged(_)) && !name.is_empty())
}

fn roman_schema_unit(tree: &Value, metadata: &HashMap<String, String>) -> ResolvedUnit {
    let (unit, source) = match roman_model(tree).as_deref() {
        Some(ROMAN_LEVEL_2_IMAGE) => {
            let state = metadata.get(ROMAN_FLUX_STEP_KEY).map(|v| v.trim()).filter(|v| !v.is_empty());
            if state.is_some_and(|v| v.eq_ignore_ascii_case(ROMAN_FLUX_STEP_COMPLETE)) {
                (Some(UNIT_MJY_PER_SR), format!("meta.cal_step.flux = {ROMAN_FLUX_STEP_COMPLETE}: flux step applied"))
            } else {
                (
                    Some(UNIT_DN_PER_SECOND),
                    format!("meta.cal_step.flux = {}: flux step not applied", state.unwrap_or("absent")),
                )
            }
        }
        Some(ROMAN_MOSAIC) => (Some(UNIT_MJY_PER_SR), format!("{ROMAN_MOSAIC} data model: mosaics are in {UNIT_MJY_PER_SR}")),
        Some(other) => (None, format!("unknown: data model '{other}' has no unit rule")),
        None => (None, "unknown: no unit on the array and no Roman data model tag".to_string()),
    };
    ResolvedUnit { unit: unit.map(str::to_string), source: Some(source) }
}

fn resolve_unit(tree: &Value, asdf_img: &AsdfImage, role: ArrayRole) -> ResolvedUnit {
    let is_roman = tree.get(ROMAN_NODE).is_some_and(|node| untag(node).is_mapping());
    let named = |text: &str| is_roman.then(|| text.to_string());
    let stored = asdf_img.unit.as_deref().map(str::trim).filter(|v| !v.is_empty());
    if role == ArrayRole::Other {
        return ResolvedUnit { unit: stored.map(str::to_string), source: None };
    }
    if let Some(unit) = stored {
        return ResolvedUnit { unit: Some(unit.to_string()), source: named(UNIT_STORED_WITH_ARRAY) };
    }
    let fallbacks: &[&[&str]] = match role {
        ArrayRole::Error => &[&ERROR_BUNIT_KEYS, &SCIENCE_BUNIT_KEYS],
        _ => &[&SCIENCE_BUNIT_KEYS],
    };
    let from_key = fallbacks.iter().flat_map(|keys| keys.iter()).find_map(|key| {
        asdf_img
            .metadata
            .get(*key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|unit| (*key, unit))
    });
    match from_key {
        Some((key, unit)) => ResolvedUnit { unit: Some(unit.to_string()), source: named(&format!("metadata key {key}")) },
        None if is_roman => roman_schema_unit(tree, &asdf_img.metadata),
        None => ResolvedUnit { unit: None, source: None },
    }
}

fn undecoded_time_note(metadata: &HashMap<String, String>) -> Option<String> {
    let mut formats: Vec<String> = Vec::new();
    for alias in FITS_ALIASES.iter().filter(|alias| alias.decode != AliasDecode::Raw) {
        for source in alias.sources {
            if let Some(format) = asdf_time::undecoded_format(metadata, source) {
                if !formats.contains(&format) {
                    formats.push(format);
                }
            }
        }
    }
    (!formats.is_empty()).then(|| format!("time format {} not decoded: no time cards", formats.join(", ")))
}

fn complex_note(dtype: Option<&DType>) -> Option<String> {
    let name = match dtype? {
        DType::Complex64 => "complex64",
        DType::Complex128 => "complex128",
        _ => return None,
    };
    Some(format!("{name} array: the modulus is shown, the phase is not kept"))
}

fn sip_warning(fit: &SipFit) -> Option<String> {
    if fit.warnings.is_empty() {
        return None;
    }
    let target = SipFitOptions::default();
    let forward_missed = fit.max_err_px > target.max_pix_error;
    let inverse_missed = matches!((fit.inv_err_px, target.max_inv_pix_error), (Some(err), Some(limit)) if err > limit);
    if !(forward_missed || inverse_missed) {
        return Some("SIP fit sampling may be too coarse (double sampling check failed)".to_string());
    }
    let inverse = fit.inv_err_px.map(|err| format!(", inv {err:.2E} px")).unwrap_or_default();
    Some(format!(
        "SIP target {} px not reached: fwd {:.2E} px{inverse}",
        target.max_pix_error, fit.max_err_px
    ))
}

fn gwcs_fit_cards(fit: &SipFit) -> Vec<(String, String)> {
    let mut cards = fit.header_cards();
    if let Some(warning) = sip_warning(fit) {
        cards.push((ASDF_SIP_WARNING_CARD.to_string(), warning));
    }
    cards
}

fn metadata_value<'a>(metadata: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    metadata
        .get(key)
        .or_else(|| metadata.get(&format!("{key}.value")))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
}

fn mjd_card(mjd: f64) -> String {
    let fixed = format!("{mjd:.9}");
    let trimmed = fixed.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{trimmed}0")
    } else {
        trimmed.to_string()
    }
}

fn alias_value(metadata: &HashMap<String, String>, alias: &FitsAlias) -> Option<String> {
    let decoded_time = || alias.sources.iter().find_map(|k| asdf_time::decode(metadata, k));
    match alias.decode {
        AliasDecode::Raw => alias
            .sources
            .iter()
            .filter_map(|k| metadata_value(metadata, k))
            .find(|v| !alias.numeric || v.parse::<f64>().is_ok_and(f64::is_finite))
            .map(str::to_string),
        AliasDecode::TimeIso => decoded_time().map(|t| t.iso),
        AliasDecode::TimeMjd => decoded_time().map(|t| mjd_card(t.mjd)),
        AliasDecode::TimeScale => decoded_time().map(|t| t.scale),
    }
}

pub fn extract_image_from_asdf(path: &Path) -> Result<MmapImageResult> {
    extract_plane_from_asdf(path, None)
}

fn array_info_to_hdu(i: usize, a: &AsdfArrayInfo) -> HduInfo {
    let rank = a.shape.len();
    let geometry = plane_geometry(&a.shape);
    HduInfo {
        index: i,
        extname: Some(a.key.clone()),
        naxis: rank as i64,
        naxis1: geometry.width as i64,
        naxis2: geometry.height as i64,
        naxis3: if rank >= 3 { geometry.plane_count as i64 } else { 0 },
        naxis4: if rank >= 4 { a.shape[rank - 4] as i64 } else { 0 },
        bitpix: a.bitpix,
        has_data: rank >= 2,
        extver: None,
        header_start: 0,
        data_start: 0,
    }
}

fn is_resultant_ramp_shape(shape: &[usize]) -> bool {
    shape.len() == 3 && shape[0] >= 2 && !is_interleaved_layout(shape)
}

fn multi_plane_refusal(key: &str, shape: &[usize], planes: usize) -> anyhow::Error {
    let mut message = format!(
        "ASDF array '{}' cannot be loaded as a 2D image: {}D array [{}] of {} planes, not a single 2D image",
        key,
        shape.len(),
        shape_label(shape),
        planes
    );
    if is_resultant_ramp_shape(shape) {
        message.push_str(&format!(
            "; a [n, h, w] array is a ramp of {} resultants: open it with #array={} to view resultant 1 and use the Ramp panel to step",
            shape[0], key
        ));
    }
    anyhow::anyhow!(message)
}

pub fn list_asdf_arrays(path: &Path) -> Result<Vec<HduInfo>> {
    let asdf = AsdfFile::open(path).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    Ok(list_arrays(&asdf)
        .iter()
        .enumerate()
        .map(|(i, a)| array_info_to_hdu(i, a))
        .collect())
}

pub fn companion_key(data_key: &str, suffix: &str) -> String {
    match data_key.rfind('.') {
        Some(pos) => format!("{}.{}", &data_key[..pos], suffix),
        None => suffix.to_string(),
    }
}

pub fn extract_plane_from_asdf(path: &Path, key: Option<&str>) -> Result<MmapImageResult> {
    let asdf = AsdfFile::open(path).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    extract_plane_from_open_asdf(&asdf, key)
}

pub fn asdf_plane_header(path: &Path, key: Option<&str>) -> Result<HduHeader> {
    let asdf = AsdfFile::open(path).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    header_from_open_asdf(&asdf, key)
}

pub fn header_from_open_asdf(asdf: &AsdfFile, key: Option<&str>) -> Result<HduHeader> {
    Ok(select_plane(asdf, key, false)?.header)
}

struct SelectedPlane {
    asdf_img: AsdfImage,
    header: HduHeader,
    data_key: String,
    arrays: Vec<AsdfArrayInfo>,
}

fn select_plane(asdf: &AsdfFile, key: Option<&str>, with_pixels: bool) -> Result<SelectedPlane> {
    let arrays = list_arrays(asdf);
    let explicit = key.is_some();
    let auto_key = auto_data_key(asdf);
    if !explicit {
        if let Some(info) = auto_key
            .as_deref()
            .and_then(|k| arrays.iter().find(|a| a.key == k))
        {
            let planes = plane_geometry(&info.shape).plane_count;
            if planes > 1 {
                return Err(multi_plane_refusal(&info.key, &info.shape, planes));
            }
        }
    }

    let asdf_img = match (key, with_pixels) {
        (Some(k), true) => AsdfImage::load_array_plane(asdf, k, 0),
        (Some(k), false) => AsdfImage::describe_array(asdf, k),
        (None, true) => AsdfImage::from_file(asdf),
        (None, false) => AsdfImage::describe_file(asdf),
    }
    .map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;

    if !asdf_img.has_image() {
        anyhow::bail!("ASDF load failed: Missing field: data array");
    }
    let data_key = asdf_img
        .metadata
        .get("ASDF_DATA_KEY")
        .cloned()
        .unwrap_or_default();
    let plane_count = asdf_img.plane_count();
    if !explicit && plane_count != 1 {
        return Err(multi_plane_refusal(&data_key, &asdf_img.shape, plane_count));
    }

    let role = array_role(&data_key, auto_key.as_deref());
    let header = synthesise_header(&asdf.tree, &asdf_img, &data_key, plane_count, role);
    Ok(SelectedPlane { asdf_img, header, data_key, arrays })
}

pub fn extract_plane_from_open_asdf(asdf: &AsdfFile, key: Option<&str>) -> Result<MmapImageResult> {
    let SelectedPlane { asdf_img, header, data_key, arrays } = select_plane(asdf, key, true)?;
    let image = asdf_img
        .into_plane(0)
        .with_context(|| format!("ASDF array '{data_key}' has no readable first plane"))?;

    let extensions: Vec<HduInfo> = arrays
        .iter()
        .enumerate()
        .map(|(i, a)| array_info_to_hdu(i, a))
        .collect();

    Ok(MmapImageResult {
        header,
        image,
        selected_extension: Some(data_key),
        extensions,
    })
}

pub struct AsdfRgbResult {
    pub r: Array2<f32>,
    pub g: Array2<f32>,
    pub b: Array2<f32>,
    pub header: HduHeader,
}

fn is_interleaved_colour(shape: &[usize]) -> bool {
    let geometry = plane_geometry(shape);
    is_interleaved_layout(shape)
        && (3..=4).contains(&geometry.plane_count)
        && geometry.width > 1
        && geometry.height > 1
}

pub fn try_extract_rgb_from_asdf(path: &Path) -> Result<Option<AsdfRgbResult>> {
    let asdf = AsdfFile::open(path).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    let Some(key) = auto_data_key(&asdf) else {
        return Ok(None);
    };
    let arrays = list_arrays(&asdf);
    let Some(info) = arrays.iter().find(|a| a.key == key) else {
        return Ok(None);
    };
    if !is_interleaved_colour(&info.shape) {
        return Ok(None);
    }

    let asdf_img = AsdfImage::load_array(&asdf, &key)
        .map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    let plane_count = asdf_img.plane_count();
    let header = synthesise_header(&asdf.tree, &asdf_img, &key, plane_count, ArrayRole::Science);
    let channel = |i: usize| {
        asdf_img
            .plane(i)
            .with_context(|| format!("ASDF array '{key}' is missing colour plane {}", i + 1))
    };
    Ok(Some(AsdfRgbResult {
        r: channel(0)?,
        g: channel(1)?,
        b: channel(2)?,
        header,
    }))
}

fn synthesise_header(tree: &Value, asdf_img: &AsdfImage, data_key: &str, plane_count: usize, role: ArrayRole) -> HduHeader {
    let (width, height) = (asdf_img.width, asdf_img.height);
    let rank = asdf_img.shape.len();

    let mut cards = Vec::new();
    let mut index = HashMap::new();

    push_card(&mut cards, &mut index, "NAXIS", rank.max(2).to_string());
    push_card(&mut cards, &mut index, "NAXIS1", width.to_string());
    push_card(&mut cards, &mut index, "NAXIS2", height.to_string());
    for axis in 3..=rank {
        let length = if axis == 3 { plane_count.max(1) } else { 1 };
        push_card(&mut cards, &mut index, &format!("NAXIS{axis}"), length.to_string());
    }
    push_card(&mut cards, &mut index, "BITPIX", "-32".into());
    push_card(&mut cards, &mut index, "EXTNAME", data_key.to_string());
    push_card(&mut cards, &mut index, ASDF_DATA_KEY_CARD, data_key.to_string());
    if plane_count > 1 {
        push_card(&mut cards, &mut index, ASDF_SELECTED_PLANE, "1".into());
        push_card(&mut cards, &mut index, ASDF_PLANE_COUNT, plane_count.to_string());
    }

    match &asdf_img.wcs {
        AsdfWcs::None(Some(note)) => push_card(&mut cards, &mut index, ASDF_WCS_NOTE_CARD, note.clone()),
        AsdfWcs::None(None) => {}
        AsdfWcs::Gwcs { fit, frame_note, .. } => {
            for (k, v) in gwcs_fit_cards(fit) {
                push_card(&mut cards, &mut index, &k, v);
            }
            if let Some(note) = frame_note {
                push_card(&mut cards, &mut index, ASDF_FRAME_NOTE_CARD, note.clone());
            }
        }
        AsdfWcs::Tan(wcs) => {
            let wcs_entries = [
                ("CRPIX1", wcs.crpix[0].to_string()),
                ("CRPIX2", wcs.crpix[1].to_string()),
                ("CRVAL1", wcs.crval[0].to_string()),
                ("CRVAL2", wcs.crval[1].to_string()),
                ("CDELT1", wcs.cdelt[0].to_string()),
                ("CDELT2", wcs.cdelt[1].to_string()),
                ("PC1_1", wcs.pc[0][0].to_string()),
                ("PC1_2", wcs.pc[0][1].to_string()),
                ("PC2_1", wcs.pc[1][0].to_string()),
                ("PC2_2", wcs.pc[1][1].to_string()),
                ("CTYPE1", wcs.ctype[0].clone()),
                ("CTYPE2", wcs.ctype[1].clone()),
                ("CUNIT1", wcs.cunit[0].clone()),
                ("CUNIT2", wcs.cunit[1].clone()),
            ];
            for (k, v) in wcs_entries {
                push_card(&mut cards, &mut index, k, v);
            }
            if let Some(lonpole) = wcs.lonpole {
                push_card(&mut cards, &mut index, "LONPOLE", lonpole.to_string());
            }
            if let Some(note) = &wcs.frame_note {
                push_card(&mut cards, &mut index, ASDF_FRAME_NOTE_CARD, note.clone());
            }
        }
    }

    for alias in &FITS_ALIASES {
        if alias.photometric && role == ArrayRole::Other {
            continue;
        }
        if let Some(value) = alias_value(&asdf_img.metadata, alias) {
            push_card(&mut cards, &mut index, alias.card, value);
        }
    }
    if let Some(note) = undecoded_time_note(&asdf_img.metadata) {
        push_card(&mut cards, &mut index, ASDF_TIME_NOTE_CARD, note);
    }
    if let Some(note) = complex_note(asdf_img.dtype.as_ref()) {
        push_card(&mut cards, &mut index, ASDF_COMPLEX_NOTE_CARD, note);
    }

    let mut extra: Vec<(&String, &String)> = asdf_img
        .metadata
        .iter()
        .filter(|(k, _)| k.as_str() != "ASDF_DATA_KEY")
        .collect();
    extra.sort();
    for (k, v) in extra {
        let fits_key = k
            .replace('.', "_")
            .chars()
            .take(68)
            .collect::<String>()
            .to_uppercase();
        if !index.contains_key(&fits_key) {
            push_card(&mut cards, &mut index, &fits_key, v.clone());
        }
    }

    let resolved = resolve_unit(tree, asdf_img, role);
    if !index.contains_key(HEADER_BUNIT) {
        if let Some(unit) = resolved.unit {
            push_card(&mut cards, &mut index, HEADER_BUNIT, unit);
        }
        if let Some(source) = resolved.source {
            push_card(&mut cards, &mut index, ASDF_UNIT_SOURCE_CARD, source);
        }
    }

    push_card(&mut cards, &mut index, "ASDF_SRC", "true".into());

    HduHeader { cards, index, string_keys: None }
}

fn push_card(
    cards: &mut Vec<(String, String)>,
    index: &mut HashMap<String, String>,
    key: &str,
    value: String,
) {
    index.insert(key.to_string(), value.clone());
    cards.push((key.to_string(), value));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::gwcs::test_support::{
        fixture_pipeline, real_data_dir, skip_if_absent, truth_dir, truth_from, unwrap_pair,
    };
    use crate::core::astrometry::wcs::{angular_separation, WcsTransform};
    use crate::core::metadata::photcal::PhotCal;
    use crate::infra::asdf::converter::test_fixtures::nircam_gwcs_with_inline_data;
    use crate::infra::asdf::tree::DType;
    use crate::infra::image_source::load_plane;
    use crate::types::image::IntPlane;
    use crate::types::image_ref::ImageRef;

    fn int_plane(path: &Path, key: &str) -> Result<Option<IntPlane>> {
        let asdf = AsdfFile::open(path).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
        AsdfImage::load_array_int(&asdf, key).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))
    }

    fn fits_legal(header: &HduHeader) -> HduHeader {
        let mut legal = HduHeader::empty();
        for (k, v) in &header.cards {
            let keyword = k.len() <= 8
                && k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
            if keyword {
                legal.set(k, v.clone());
            }
        }
        legal
    }

    fn write_asdf(dir: &tempfile::TempDir, tree_yaml: &str) -> std::path::PathBuf {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(tree_yaml.as_bytes());
        bytes.extend_from_slice(b"...\n");
        let path = dir.path().join("fixture.asdf");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    const INLINE_DATA: &str = "data: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\n";

    #[test]
    fn bunit_data_metadata_becomes_bunit_card() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("meta:\n  bunit_data: MJy/sr\n  instrument: {{name: NIRCam}}\n{INLINE_DATA}");
        let result = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap();
        assert_eq!(result.header.get(HEADER_BUNIT), Some("MJy/sr"));
        assert_eq!(result.header.get("META_BUNIT_DATA"), Some("MJy/sr"));
        assert_eq!(result.image.dim(), (2, 2));
    }

    #[test]
    fn quantity_unit_wins_over_metadata_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let tree = "meta:\n  bunit_data: MJy/sr\ndata: !unit/quantity-1.1.0\n  value: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  unit: !unit/unit-1.0.0 DN / s\n";
        let result = extract_image_from_asdf(&write_asdf(&dir, tree)).unwrap();
        assert_eq!(result.header.get(HEADER_BUNIT), Some("DN / s"));
    }

    #[test]
    fn fallback_chain_reaches_header_keys() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("header:\n  BUNIT: ' ADU '\n{INLINE_DATA}");
        let result = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap();
        assert_eq!(result.header.get(HEADER_BUNIT), Some("ADU"));
    }

    const DQ_TREE: &str = "meta:\n  telescope: JWST\ndata: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\ndq: !core/ndarray-1.0.0\n  data: [[0, 1], [3, 0]]\n  datatype: uint32\nerr: !unit/quantity-1.1.0\n  value: !core/ndarray-1.0.0\n    data: [[0.1, 0.2], [0.3, 0.4]]\n    datatype: float32\n  unit: MJy/sr\n";

    #[test]
    fn extract_plane_by_key_sets_extname_and_lists_arrays() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, DQ_TREE);
        let result = extract_plane_from_asdf(&path, Some("dq")).unwrap();
        assert_eq!(result.header.get("EXTNAME"), Some("dq"));
        assert_eq!(result.header.get(ASDF_DATA_KEY_CARD), Some("dq"));
        assert_eq!(result.header.get("META_TELESCOPE"), Some("JWST"));
        assert_eq!(result.header.get("TELESCOP"), Some("JWST"));
        assert_eq!(result.selected_extension.as_deref(), Some("dq"));
        assert_eq!(result.image[[1, 0]], 3.0);
        assert_eq!(result.extensions.len(), 3);
        let names: Vec<&str> = result.extensions.iter().filter_map(|e| e.extname.as_deref()).collect();
        assert_eq!(names, vec!["data", "dq", "err"]);
        assert_eq!(result.extensions[1].bitpix, 32);
        assert!(result.extensions[1].has_data);
        assert_eq!(result.extensions[1].naxis1, 2);

        let err = extract_plane_from_asdf(&path, Some("err")).unwrap();
        assert_eq!(err.header.get(HEADER_BUNIT), Some("MJy/sr"));
        assert_eq!(err.header.get("EXTNAME"), Some("err"));

        let auto = extract_plane_from_asdf(&path, None).unwrap();
        assert_eq!(auto.header.get("EXTNAME"), Some("data"));
        assert_eq!(auto.extensions.len(), 3);
        assert!(extract_plane_from_asdf(&path, Some("nope")).is_err());
    }

    #[test]
    fn list_asdf_arrays_and_int_plane_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, DQ_TREE);
        let arrays = list_asdf_arrays(&path).unwrap();
        assert_eq!(arrays.len(), 3);
        assert_eq!(arrays[2].extname.as_deref(), Some("err"));
        let plane = int_plane(&path, "dq").unwrap().unwrap();
        assert_eq!(plane.bits[[1, 0]], 3);
        assert_eq!(plane.bits[[0, 1]], 1);
        assert!(int_plane(&path, "data").unwrap().is_none());
        assert!(int_plane(&path, "missing").is_err());
    }

    #[test]
    fn companion_key_replaces_last_segment() {
        assert_eq!(companion_key("roman.data", "dq"), "roman.dq");
        assert_eq!(companion_key("data", "err"), "err");
        assert_eq!(companion_key("sci", "dq"), "dq");
        assert_eq!(companion_key("a.b.c", "err"), "a.b.err");
    }

    const CUBE_TREE: &str = "meta:\n  telescope: ROMAN\nroman:\n  data: !core/ndarray-1.0.0\n    data: [[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]]]\n    datatype: float32\n  dq: !core/ndarray-1.0.0\n    data: [[[0, 1], [0, 0]], [[1, 1], [0, 0]], [[0, 0], [0, 1]]]\n    datatype: uint32\n";

    #[test]
    fn auto_refuses_a_cube_but_an_explicit_reference_opens_its_first_plane() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, CUBE_TREE);

        let auto = extract_plane_from_asdf(&path, None).err().expect("a ramp must not load as an image");
        let msg = format!("{auto:#}");
        assert!(msg.contains("'roman.data'"), "{msg}");
        assert!(msg.contains("3D array [3x2x2] of 3 planes"), "{msg}");
        assert!(msg.contains("not a single 2D image"), "{msg}");

        let explicit = extract_plane_from_asdf(&path, Some("roman.data"))
            .expect("an explicit #array= reference must open plane 1, as #hdu= does for a FITS cube");
        assert_eq!(explicit.image.dim(), (2, 2));
        assert_eq!(explicit.image.as_slice().unwrap(), &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(explicit.header.get(ASDF_SELECTED_PLANE), Some("1"));
        assert_eq!(explicit.header.get(ASDF_PLANE_COUNT), Some("3"));
        assert_eq!(explicit.header.get("NAXIS"), Some("3"));
        assert_eq!(explicit.header.get("NAXIS3"), Some("3"));
        assert_eq!(
            explicit.selected_extension.as_deref(),
            Some("roman.data"),
            "the bare key is the ASDF data key that companion resolution consumes"
        );

        let dq = int_plane(&path, "roman.dq").unwrap().unwrap();
        assert_eq!(dq.bits.dim(), (2, 2));
        assert_eq!(
            dq.bits.as_slice().unwrap(),
            &[0, 1, 0, 0],
            "the mask plane must line up with the image plane it accompanies"
        );
    }

    #[test]
    fn the_asdf_multi_plane_refusal_now_points_at_the_ramp_vocabulary() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, CUBE_TREE);
        let msg = format!("{:#}", extract_plane_from_asdf(&path, None).err().unwrap());
        assert!(msg.contains("not a single 2D image"), "{msg}");
        assert!(msg.contains("a [n, h, w] array is a ramp of 3 resultants"), "{msg}");
        assert!(msg.contains("#array=roman.data"), "{msg}");
        assert!(msg.contains("Ramp panel"), "{msg}");
        let listed = list_asdf_arrays(&path).unwrap();
        let cube = listed.iter().find(|e| e.extname.as_deref() == Some("roman.data")).unwrap();
        assert_eq!((cube.naxis3, cube.naxis4), (3, 0));

        let interleaved = write_asdf(&dir, &interleaved_tree());
        let msg = format!("{:#}", extract_plane_from_asdf(&interleaved, None).err().unwrap());
        assert!(msg.contains("not a single 2D image"), "{msg}");
        assert!(!msg.contains("resultants"), "an interleaved colour array is not a ramp: {msg}");

        let four = "stack: !core/ndarray-1.0.0\n  data: [[[[1, 2], [3, 4]], [[5, 6], [7, 8]]], [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]]\n  datatype: float32\n";
        let listed = list_asdf_arrays(&write_asdf(&dir, four)).unwrap();
        assert_eq!((listed[0].naxis, listed[0].naxis4), (4, 2));
    }

    #[test]
    fn a_deep_nested_cube_is_listed_refused_and_reopened_by_its_dotted_key() {
        let dir = tempfile::tempdir().unwrap();
        let values: Vec<String> = (0..60).map(|v| v.to_string()).collect();
        let tree = format!(
            "products:\n  sci:\n    data: !core/ndarray-1.0.0\n      data: [{}]\n      shape: [6, 2, 5]\n      datatype: float32\n",
            values.join(", ")
        );
        let path = write_asdf(&dir, &tree);

        let listed = list_asdf_arrays(&path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].extname.as_deref(), Some("products.sci.data"));
        assert_eq!(listed[0].naxis3, 6);

        let refused = extract_plane_from_asdf(&path, None)
            .err()
            .expect("a nested ramp must not load as an image either");
        let msg = format!("{refused:#}");
        assert!(msg.contains("'products.sci.data'"), "{msg}");
        assert!(msg.contains("3D array [6x2x5] of 6 planes"), "{msg}");
        assert!(msg.contains("not a single 2D image"), "{msg}");

        let reopened = extract_plane_from_asdf(&path, Some("products.sci.data"))
            .expect("the key named in the refusal opens the first plane");
        assert_eq!(reopened.image.dim(), (2, 5));
        assert_eq!(reopened.image[[1, 4]], 9.0);
        assert_eq!(reopened.selected_extension.as_deref(), Some("products.sci.data"));
        assert_eq!(companion_key("products.sci.data", "dq"), "products.sci.dq");
    }

    const JWST_PLANES: &str = "meta:\n  telescope: JWST\n  bunit_data: MJy/sr\n  bunit_err: uJy/arcsec^2\n  photometry:\n    pixelarea_steradians: 2.1e-13\ndata: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\ndq: !core/ndarray-1.0.0\n  data: [[0, 1024], [0, 0]]\n  datatype: uint32\nerr: !core/ndarray-1.0.0\n  data: [[0.1, 0.2], [0.3, 0.4]]\n  datatype: float32\nvar_poisson: !core/ndarray-1.0.0\n  data: [[0.01, 0.04], [0.09, 0.16]]\n  datatype: float32\n";

    #[test]
    fn science_unit_is_not_stamped_on_dq_and_variance_planes() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, JWST_PLANES);
        let header = |key: Option<&str>| extract_plane_from_asdf(&path, key).unwrap().header;

        assert_eq!(header(None).get(HEADER_BUNIT), Some("MJy/sr"));
        assert_eq!(header(Some("data")).get(HEADER_BUNIT), Some("MJy/sr"));
        assert_eq!(header(Some("data")).get("PIXAR_SR"), Some("2.1e-13"));
        assert_eq!(header(Some("err")).get(HEADER_BUNIT), Some("uJy/arcsec^2"));

        let dq = header(Some("dq"));
        assert_eq!(dq.get(HEADER_BUNIT), None, "a DQ bitmask has no physical unit");
        assert_eq!(dq.get("PIXAR_SR"), None);
        assert_eq!(dq.get("TELESCOP"), Some("JWST"));
        assert_eq!(header(Some("var_poisson")).get(HEADER_BUNIT), None, "variance is not in MJy/sr");
    }

    #[test]
    fn a_unit_on_the_array_node_itself_is_kept_for_any_plane() {
        let dir = tempfile::tempdir().unwrap();
        let tree = "meta:\n  bunit_data: MJy/sr\nvar_rnoise: !unit/quantity-1.1.0\n  value: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  unit: MJy2 / sr2\n";
        let result = extract_plane_from_asdf(&write_asdf(&dir, tree), Some("var_rnoise")).unwrap();
        assert_eq!(result.header.get(HEADER_BUNIT), Some("MJy2 / sr2"));
    }

    #[test]
    fn roman_calibration_and_provenance_survive_the_fits_keyword_limit() {
        let dir = tempfile::tempdir().unwrap();
        let tree = "roman: !<asdf://stsci.edu/datamodels/roman/tags/wfi_image-1.0.0>\n  meta:\n    telescope: ROMAN\n    instrument: {name: WFI}\n    exposure: {exposure_time: 107.0}\n    cal_step: {flux: INCOMPLETE}\n    photometry:\n      conversion_megajanskys: !unit/quantity-1.1.0\n        value: 0.3324\n        unit: !unit/unit-1.0.0 MJy.sr**-1\n      pixelarea_steradians: !unit/quantity-1.1.0\n        value: 2.8e-13\n        unit: !unit/unit-1.0.0 sr\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n";
        let loaded = extract_image_from_asdf(&write_asdf(&dir, tree)).unwrap();
        let legal = fits_legal(&loaded.header);

        assert_eq!(legal.get(ASDF_DATA_KEY_CARD), Some("roman.data"));
        assert_eq!(legal.get("TELESCOP"), Some("ROMAN"));
        assert_eq!(legal.get("INSTRUME"), Some("WFI"));
        assert_eq!(legal.get("EXPTIME"), Some("107.0"));
        assert_eq!(legal.get(HEADER_BUNIT), Some("DN/s"));
        assert_eq!(legal.get(ASDF_UNIT_SOURCE_CARD), loaded.header.get(ASDF_UNIT_SOURCE_CARD), "the unit source is a FITS-legal card");

        let full = PhotCal::from_header(&loaded.header, None).expect("ASDF header calibrates");
        let written = PhotCal::from_header(&legal, None).expect("FITS-legal cards still calibrate");
        let one = |cal: &PhotCal| cal.calibrate(1.0, None).map(|f| f.flux_jy).unwrap();
        assert!((one(&full) - 0.3324 * 2.8e-13 * 1e6).abs() < 1e-18);
        assert!((one(&written) - one(&full)).abs() < 1e-18);
    }

    const ROMAN_TIMES: &str = "roman:\n  meta:\n    telescope: ROMAN\n    instrument: {detector: WFI01, name: WFI, optical_element: F129}\n    exposure:\n      start_time: !time/time-1.4.0 2026-10-31T00:00:00.000\n      mid_time: !time/time-1.4.0 2026-10-31T00:01:07.993\n      end_time: !time/time-1.4.0 2026-10-31T00:02:15.986\n      exposure_time: 135.98622\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n";

    const TIME_CARDS: [&str; 5] = ["DATE-OBS", "DATE-END", "MJD-OBS", "EXPMID", "TIMESYS"];

    #[test]
    fn roman_metadata_reaches_filter_detector_and_time_cards() {
        use crate::core::astrometry::spectral::mid_exposure_jd;
        use crate::core::metadata::filter_wavelengths::effective_filter;

        let dir = tempfile::tempdir().unwrap();
        let header = extract_image_from_asdf(&write_asdf(&dir, ROMAN_TIMES)).unwrap().header;
        for (card, expected) in [
            ("FILTER", "F129"),
            ("DETECTOR", "WFI01"),
            ("DATE-OBS", "2026-10-31T00:00:00.000"),
            ("DATE-END", "2026-10-31T00:02:15.986"),
            ("TIMESYS", "UTC"),
            ("INSTRUME", "WFI"),
            ("EXPTIME", "135.98622"),
        ] {
            assert_eq!(header.get(card), Some(expected), "{card}");
        }
        assert_eq!(header.get_f64("MJD-OBS"), Some(61344.0));
        assert!((header.get_f64("EXPMID").unwrap() - 61344.000786956).abs() < 1e-9, "{:?}", header.get("EXPMID"));
        assert_eq!(header.get("PUPIL"), None);
        assert_eq!(effective_filter(&header).as_deref(), Some("F129"));
        let (jd, source) = mid_exposure_jd(&header).expect("a Roman L2 header has a mid-exposure time");
        assert_eq!(source, "mid-exposure from EXPMID");
        assert!((jd - 2461344.500786956).abs() < 1e-8, "{jd}");
        assert_eq!(fits_legal(&header).get("DATE-OBS"), Some("2026-10-31T00:00:00.000"), "the time cards are FITS-legal keywords");

        let tt = ROMAN_TIMES.replace(
            "start_time: !time/time-1.4.0 2026-10-31T00:00:00.000",
            "start_time: !time/time-1.4.0 {value: '2026-10-31T00:00:00.000', scale: tt}",
        );
        let header = extract_image_from_asdf(&write_asdf(&dir, &tt)).unwrap().header;
        assert_eq!(header.get("TIMESYS"), Some("TT"));
        assert_eq!(header.get("DATE-OBS"), Some("2026-10-31T00:00:00.000"));
        assert_eq!(header.get_f64("MJD-OBS"), Some(61344.0));

        let jwst = format!("meta:\n  telescope: JWST\n  instrument: {{name: NIRCAM, filter: F200W, pupil: CLEAR, detector: NRCB1}}\n  observation: {{date: '2022-07-12'}}\n{INLINE_DATA}");
        let header = extract_image_from_asdf(&write_asdf(&dir, &jwst)).unwrap().header;
        for (card, expected) in [("FILTER", "F200W"), ("PUPIL", "CLEAR"), ("DETECTOR", "NRCB1"), ("DATE-OBS", "2022-07-12"), ("TIMESYS", "UTC")] {
            assert_eq!(header.get(card), Some(expected), "{card}");
        }
        for card in ["DATE-END", "MJD-OBS", "EXPMID"] {
            assert_eq!(header.get(card), None, "{card}: a date without a time gives no MJD");
        }
        assert_eq!(effective_filter(&header).as_deref(), Some("F200W"));

        let jwst_mjd = format!("meta:\n  instrument: {{name: NIRSPEC}}\n  exposure: {{start_time: 59772.5, mid_time: 59772.5005, end_time: 59772.501}}\n{INLINE_DATA}");
        let header = extract_image_from_asdf(&write_asdf(&dir, &jwst_mjd)).unwrap().header;
        assert_eq!(header.get("DATE-OBS"), Some("2022-07-12T12:00:00.000"));
        assert_eq!(header.get_f64("EXPMID"), Some(59772.5005));

        let timeless = "roman:\n  meta:\n    instrument: {name: WFI, optical_element: F158}\n    exposure: {exposure_time: 10.0, start_time: !time/time-1.4.0 {value: 'J2026.5', format: jyear_str}}\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n";
        let header = extract_image_from_asdf(&write_asdf(&dir, timeless)).unwrap().header;
        assert_eq!(header.get("FILTER"), Some("F158"));
        for card in TIME_CARDS {
            assert_eq!(header.get(card), None, "{card} is not written without a decodable time");
        }
        let plain = extract_image_from_asdf(&write_asdf(&dir, INLINE_DATA)).unwrap().header;
        for card in TIME_CARDS.iter().chain(&["FILTER", "PUPIL", "DETECTOR"]) {
            assert_eq!(plain.get(card), None, "{card}");
        }
    }

    #[test]
    fn a_gwcs_without_sky_anchor_leaves_no_wcs_cards_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!(
            "roman:\n  meta:\n    wcs:\n      steps:\n        - transform: {{transform_type: Shift, offset: -2043.5}}\n        - transform: {{transform_type: Shift, offset: -2043.5}}\n        - transform: {{transform_type: Scale, factor: 0.00003}}\n        - transform: {{transform_type: Scale, factor: 0.00003}}\n        - frame: {{name: world}}\n{INLINE_DATA}"
        );
        let header = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap().header;
        for card in ["CTYPE1", "CRVAL1", "CRVAL2", "CRPIX1", "CDELT1", "PC1_1"] {
            assert_eq!(header.get(card), None, "{card} must not be fabricated");
        }
        let note = header.get(ASDF_WCS_NOTE_CARD).expect("the reason is kept in the header");
        assert!(note.contains("celestial reference"), "{note}");
        assert!(note.len() <= 67, "{note}");
    }

    #[test]
    fn a_gwcs_anchored_on_the_pole_keeps_its_pole_longitude() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!(
            "wcs:\n  steps:\n    - transform: !transform/compose-1.2.0\n        forward:\n          - !transform/shift-1.2.0 {{offset: -10.0}}\n          - !transform/shift-1.2.0 {{offset: -10.0}}\n          - !transform/scale-1.2.0 {{factor: 0.001}}\n          - !transform/scale-1.2.0 {{factor: 0.001}}\n          - !transform/gnomonic-1.2.0 {{direction: pix2sky}}\n          - !transform/rotate3d-1.3.0 {{phi: 0.0, theta: 90.0, psi: 180.0, direction: native2celestial}}\n    - frame: {{name: world}}\n{INLINE_DATA}"
        );
        let header = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap().header;
        assert_eq!(header.get("CRVAL2"), Some("90"));
        assert_eq!(header.get_f64("LONPOLE"), Some(180.0));
        assert_eq!(header.get("A_ORDER"), None, "a pure TAN chain keeps the exact cards, no fit");
        assert_eq!(header.get("SIPMXERR"), None);

        let wcs = WcsTransform::from_header(&header).unwrap();
        let sky = wcs.pixel_to_world(20.0, 10.0);
        assert!((sky.ra - 90.0).abs() < 1e-9, "gwcs puts +x at RA 90 on the pole, got {}", sky.ra);
        assert!((sky.dec - 89.99).abs() < 1e-6, "{}", sky.dec);
    }

    const TABULAR_WCS: &str = "wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>
  name: ''
  steps:
  - !<tag:stsci.edu:gwcs/step-1.3.0>
    frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0>
      axes_names: [x, y]
      axes_order: [0, 1]
      axis_physical_types: ['custom:x', 'custom:y']
      name: detector
      unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]
    transform: !transform/concatenate-1.2.0
      forward:
      - !transform/tabular-1.4.0
        bounds_error: false
        fill_value: .nan
        inputs: [x]
        lookup_table: [0.0, 1.0, 2.0]
        method: linear
        outputs: [y]
        points: [[0.0, 1.0, 2.0]]
      - !transform/shift-1.2.0 {offset: 1.0}
      inputs: [x0, x1]
      outputs: [y0, y1]
  - !<tag:stsci.edu:gwcs/step-1.3.0>
    frame: !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>
      axes_names: [lon, lat]
      axes_order: [0, 1]
      axis_physical_types: [pos.eq.ra, pos.eq.dec]
      name: world
      reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0> {frame_attributes: {}}
      unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]
    transform: null
";

    #[test]
    fn an_unsupported_gwcs_keeps_the_note_and_names_the_tag() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("{TABULAR_WCS}{INLINE_DATA}");
        let header = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap().header;
        for card in ["CTYPE1", "CRVAL1", "CRPIX1", "CD1_1", "PC1_1", "A_ORDER", "SIPMXERR"] {
            assert_eq!(header.get(card), None, "{card} must not be fabricated");
        }
        let note = header.get(ASDF_WCS_NOTE_CARD).expect("the reason is kept in the header");
        assert!(note.contains("'tabular'"), "{note}");
        assert!(note.len() <= 67, "{note}");
    }

    #[test]
    fn a_distorted_gwcs_gets_fitted_sip_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nircam_gwcs.asdf");
        std::fs::write(&path, nircam_gwcs_with_inline_data()).unwrap();
        let header = extract_image_from_asdf(&path).unwrap().header;
        assert_eq!(header.get("CTYPE1"), Some("RA---TAN-SIP"));
        assert_eq!(header.get("CTYPE2"), Some("DEC--TAN-SIP"));
        assert_eq!(header.get("A_ORDER"), Some("5"));
        assert_eq!(header.get("AP_ORDER"), Some("5"));
        assert_eq!(header.get("CRPIX1"), Some("1024.5"));
        assert_eq!(header.get("NAXIS1"), Some("4"));
        let sipmxerr_card = header.get("SIPMXERR").expect("SIPMXERR card");
        assert!(sipmxerr_card.contains("E-"), "{sipmxerr_card}");
        let sipmxerr = header.get_f64("SIPMXERR").unwrap();
        assert!(((sipmxerr - 9.220195712301538e-9) / 9.220195712301538e-9).abs() <= 0.1, "SIPMXERR {sipmxerr_card} vs gwcs 1.0.3 9.2202e-9");
        let sipiverr = header.get_f64("SIPIVERR").unwrap();
        assert!(((sipiverr - 3.593143551386226e-4) / 3.593143551386226e-4).abs() <= 1e-4, "SIPIVERR {:?} vs gwcs 1.0.3 3.5931e-4", header.get("SIPIVERR"));
        assert_eq!(header.get(ASDF_WCS_NOTE_CARD), None);
        assert_eq!(header.get("GWCSKEY"), None);
        let legal = fits_legal(&header);
        for key in ["CTYPE1", "CRPIX1", "CRVAL1", "CD1_1", "A_2_0", "AP_1_0", "SIPMXERR", "SIPIVERR", "LONPOLE", "RADESYS"] {
            assert_eq!(legal.get(key), header.get(key), "{key} survives the FITS keyword filter");
        }

        let wcs = WcsTransform::from_header(&header).unwrap();
        let p = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let truth = p.forward(0.0, 0.0, false);
        let sky = wcs.pixel_to_world(0.0, 0.0);
        let sep_px = angular_separation(sky.ra, sky.dec, truth[0], truth[1]) * 3600.0 / wcs.pixel_scale_arcsec();
        assert!(sep_px < 0.01, "header SIP at (0, 0) is {sep_px} px from the gWCS");
    }

    #[test]
    #[ignore]
    fn gwcs_real_roman_cal_opens_with_a_fitted_tan_sip_header() {
        let path = real_data_dir().join("r9999901001001001001_0001_wfi01_f129_cal.asdf");
        let truth_path = truth_dir().join("roman_wfi01_f129_cal.json");
        if skip_if_absent(&path) || skip_if_absent(&truth_path) {
            return;
        }
        let header = extract_image_from_asdf(&path).unwrap().header;
        assert_eq!(header.get("CTYPE1"), Some("RA---TAN-SIP"));
        assert_eq!(header.get("A_ORDER"), Some("5"));
        assert_eq!(header.get("AP_ORDER"), Some("5"));
        assert_eq!(header.get(ASDF_WCS_NOTE_CARD), None);
        assert_eq!(header.get("GWCSKEY"), None);
        let sipmxerr = header.get_f64("SIPMXERR").expect("SIPMXERR");
        assert!(((sipmxerr - 3.4796e-7) / 3.4796e-7).abs() <= 0.1, "SIPMXERR {sipmxerr} vs gwcs 1.0.3 3.4796e-7");
        let sipiverr = header.get_f64("SIPIVERR").expect("SIPIVERR");
        assert!(((sipiverr - 9.534e-4) / 9.534e-4).abs() <= 1e-3, "SIPIVERR {sipiverr} vs gwcs 1.0.3 9.534e-4");
        assert_eq!(header.get_f64("CRPIX1"), Some(2044.5));
        assert_eq!(header.get_f64("CRPIX2"), Some(2044.5));
        assert!((header.get_f64("CRVAL1").unwrap() - 72.420918974288).abs() < 1e-9, "{:?}", header.get("CRVAL1"));
        assert!((header.get_f64("CRVAL2").unwrap() - -30.664375673956).abs() < 1e-9, "{:?}", header.get("CRVAL2"));
        for (key, expected) in [
            ("CD1_1", -3.0650203286753e-05),
            ("CD1_2", -7.3720009911159e-08),
            ("CD2_1", 1.0857612088582e-07),
            ("CD2_2", 3.0078322504653e-05),
        ] {
            let got = header.get_f64(key).unwrap();
            assert!((got - expected).abs() < 1e-12, "{key}: {got} vs gwcs {expected}");
        }

        let wcs = WcsTransform::from_header(&header).unwrap();
        let truth = truth_from(&truth_path);
        let mut worst_mas = 0.0f64;
        for point in &truth.points {
            let world = unwrap_pair(&point.world_no_bbox);
            if !(world[0].is_finite() && world[1].is_finite()) {
                continue;
            }
            let sky = wcs.pixel_to_world(point.pixel[0], point.pixel[1]);
            worst_mas = worst_mas.max(angular_separation(sky.ra, sky.dec, world[0], world[1]) * 3.6e6);
        }
        println!(
            "roman cal header: A_ORDER {} AP_ORDER {} SIPMXERR {sipmxerr:.4e} SIPIVERR {sipiverr:.4e}; header SIP vs gWCS truth max {worst_mas:.3e} mas over {} points",
            header.get("A_ORDER").unwrap(),
            header.get("AP_ORDER").unwrap(),
            truth.points.len()
        );
        assert!(worst_mas <= 1e-4, "{worst_mas} mas");
    }

    #[test]
    #[ignore]
    fn gwcs_real_roman_float16_err_and_variance_planes_load() {
        let path = real_data_dir().join("r9999901001001001001_0001_wfi01_f129_cal.asdf");
        if skip_if_absent(&path) {
            return;
        }
        let path_text = path.to_string_lossy().into_owned();
        let asdf = AsdfFile::open(&path).unwrap();
        let float16: Vec<String> = list_arrays(&asdf).into_iter().filter(|a| a.dtype == DType::Float16).map(|a| a.key).collect();
        assert_eq!(float16, ["roman.err", "roman.var_poisson", "roman.chisq", "roman.dumo"]);
        let listed = list_asdf_arrays(&path).unwrap();
        for key in &float16 {
            let hdu = listed
                .iter()
                .find(|h| h.extname.as_deref() == Some(key.as_str()))
                .unwrap_or_else(|| panic!("{key} is listed in the Structure panel"));
            assert!(hdu.has_data, "{key}");
            assert_eq!((hdu.bitpix, hdu.naxis1, hdu.naxis2), (-32, 4088, 4088), "{key}");
        }

        let err = extract_plane_from_asdf(&path, Some("roman.err")).unwrap();
        assert_eq!(err.image.dim(), (4088, 4088));
        let finite: Vec<f32> = err.image.iter().copied().filter(|v| v.is_finite()).collect();
        let finite_fraction = finite.len() as f64 / err.image.len() as f64;
        let min = finite.iter().copied().fold(f32::INFINITY, f32::min);
        let max = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        println!("roman.err: finite fraction {finite_fraction:.6}, min {min}, max {max}");
        assert!(finite_fraction > 0.99, "{finite_fraction}");
        assert!(min >= 0.0, "{min}");
        let flux_step = asdf.tree["roman"]["meta"]["cal_step"]["flux"].as_str().unwrap_or("absent").to_string();
        let expected_unit = if flux_step == "COMPLETE" { "MJy/sr" } else { "DN/s" };
        let science = extract_image_from_asdf(&path).unwrap().header;
        println!(
            "roman cal: meta.cal_step.flux {flux_step}; BUNIT {:?} ({:?}); PIXAR_SR {:?}; PHOTMJSR {:?}",
            science.get(HEADER_BUNIT),
            science.get(ASDF_UNIT_SOURCE_CARD),
            science.get("PIXAR_SR"),
            science.get("PHOTMJSR")
        );
        assert_eq!(science.get(HEADER_BUNIT), Some(expected_unit), "the unit follows the flux step record of the file");
        assert_eq!(err.header.get(HEADER_BUNIT), Some(expected_unit), "the error plane is in the science unit");
        assert!(PhotCal::from_header(&science, None).is_some(), "a real Roman level 2 image calibrates");

        let loaded = load_plane(&ImageRef::parse(&path_text)).unwrap();
        assert_eq!(loaded.info.extname.as_deref(), Some("roman.data"));
        assert_eq!(loaded.companions.err, Some(ImageRef::array(&path_text, "roman.err")));
    }

    #[test]
    #[ignore]
    fn gwcs_real_roman_cal_header_has_filter_time_and_detector() {
        use crate::core::astrometry::geometry::header_time;
        use crate::core::astrometry::spectral::mid_exposure_jd;
        use crate::core::metadata::filter_wavelengths::{effective_filter, filter_code_and_wavelength_nm};

        let path = real_data_dir().join("r9999901001001001001_0001_wfi01_f129_cal.asdf");
        if skip_if_absent(&path) {
            return;
        }
        let header = extract_image_from_asdf(&path).unwrap().header;
        let filter = effective_filter(&header);
        assert_eq!(filter.as_deref(), Some("F129"));
        assert_eq!(filter_code_and_wavelength_nm(filter.as_deref().unwrap()), Some(("F129", 1290)));
        assert_eq!(header.get("DETECTOR"), Some("WFI01"));
        assert_eq!(header.get("DATE-OBS"), Some("2026-10-31T00:00:00.000"));
        assert_eq!(header.get("DATE-END"), Some("2026-10-31T00:02:15.986"));
        assert_eq!(header.get_f64("MJD-OBS"), Some(61344.0));
        let (jd, source) = mid_exposure_jd(&header).expect("mid-exposure time");
        assert_eq!(source, "mid-exposure from EXPMID");
        assert!((jd - 2461344.500786956).abs() < 1e-8, "{jd}");
        let time = header_time(&header).expect("header time");
        assert_eq!(time.scale, "UTC");
        println!(
            "roman cal: FILTER {:?} DETECTOR {:?} DATE-OBS {:?} DATE-END {:?} MJD-OBS {:?} EXPMID {:?} TIMESYS {:?}; mid JD {jd} ({source}); scale {}",
            header.get("FILTER"),
            header.get("DETECTOR"),
            header.get("DATE-OBS"),
            header.get("DATE-END"),
            header.get("MJD-OBS"),
            header.get("EXPMID"),
            header.get("TIMESYS"),
            time.scale
        );
    }

    const ROMAN_PHOTOMETRY: &str = "    photometry:\n      conversion_megajanskys: 0.3324\n      conversion_megajanskys_uncertainty: 0.0021\n      pixel_area: 2.8e-13\n";
    const ROMAN_ARRAYS: &str = "  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  err: !core/ndarray-1.0.0\n    data: [[0.1, 0.2], [0.3, 0.4]]\n    datatype: float32\n  dq: !core/ndarray-1.0.0\n    data: [[0, 1], [0, 0]]\n    datatype: uint32\n";
    const ONE_DN_PER_SECOND_JY: f64 = 0.3324 * 2.8e-13 * 1e6;
    const ONE_MJY_PER_SR_JY: f64 = 2.8e-13 * 1e6;

    fn roman_tree(model_tag: &str, cal_step: &str) -> String {
        format!(
            "roman: !<asdf://stsci.edu/datamodels/roman/tags/{model_tag}>\n  meta:\n    telescope: ROMAN\n    instrument: {{name: WFI, detector: WFI01, optical_element: F129}}\n{cal_step}{ROMAN_PHOTOMETRY}{ROMAN_ARRAYS}"
        )
    }

    fn one_unit_in_jy(header: &HduHeader) -> f64 {
        let cal = PhotCal::from_header(header, None).expect("the header calibrates");
        cal.calibrate(1.0, None).map(|f| f.flux_jy).unwrap()
    }

    #[test]
    fn a_roman_level_2_image_before_the_flux_step_is_in_dn_per_second() {
        let dir = tempfile::tempdir().unwrap();
        for state in ["INCOMPLETE", "SKIPPED", "N/A", "FAILED"] {
            let tree = roman_tree("wfi_image-1.2.0", &format!("    cal_step: {{flat_field: COMPLETE, flux: {state}, photom: COMPLETE}}\n"));
            let path = write_asdf(&dir, &tree);
            let header = extract_image_from_asdf(&path).unwrap().header;
            assert_eq!(header.get(HEADER_BUNIT), Some("DN/s"), "flux step {state}");
            let source = header.get(ASDF_UNIT_SOURCE_CARD).unwrap_or_else(|| panic!("flux step {state}: the unit source is named"));
            assert!(source.contains("cal_step.flux") && source.contains(state), "{source}");
            assert_eq!(header.get("PIXAR_SR"), Some("2.8e-13"), "the current schema names the pixel area pixel_area");
            assert!((one_unit_in_jy(&header) - ONE_DN_PER_SECOND_JY).abs() < 1e-18, "flux step {state}");
            assert!((one_unit_in_jy(&fits_legal(&header)) - ONE_DN_PER_SECOND_JY).abs() < 1e-18, "FITS-legal cards, flux step {state}");
            let err = extract_plane_from_asdf(&path, Some("roman.err")).unwrap().header;
            assert_eq!(err.get(HEADER_BUNIT), Some("DN/s"), "the error plane follows the science unit");
            let dq = extract_plane_from_asdf(&path, Some("roman.dq")).unwrap().header;
            assert_eq!((dq.get(HEADER_BUNIT), dq.get(ASDF_UNIT_SOURCE_CARD)), (None, None), "a DQ bitmask has no unit");
        }
        let no_step = roman_tree("wfi_image-1.2.0", "");
        let header = extract_image_from_asdf(&write_asdf(&dir, &no_step)).unwrap().header;
        assert_eq!(header.get(HEADER_BUNIT), Some("DN/s"), "a level 2 image without a flux step record was not flux calibrated");
    }

    #[test]
    fn a_roman_level_2_image_after_the_flux_step_is_in_mjy_per_sr_and_is_not_converted_twice() {
        let dir = tempfile::tempdir().unwrap();
        let tree = roman_tree("wfi_image-2.2.0", "    cal_step: {flat_field: COMPLETE, flux: COMPLETE, photom: COMPLETE}\n");
        let path = write_asdf(&dir, &tree);
        let header = extract_image_from_asdf(&path).unwrap().header;
        assert_eq!(header.get(HEADER_BUNIT), Some("MJy/sr"));
        let source = header.get(ASDF_UNIT_SOURCE_CARD).expect("the unit source is named");
        assert!(source.contains("cal_step.flux") && source.contains("COMPLETE"), "{source}");
        assert!(source.len() <= 67, "{source}");
        let one = one_unit_in_jy(&header);
        assert!((one - ONE_MJY_PER_SR_JY).abs() < 1e-18, "{one}");
        assert!((one - ONE_DN_PER_SECOND_JY).abs() > 1e-8, "conversion_megajanskys must not be applied to MJy/sr data");
        let err = extract_plane_from_asdf(&path, Some("roman.err")).unwrap().header;
        assert_eq!(err.get(HEADER_BUNIT), Some("MJy/sr"));
    }

    #[test]
    fn a_roman_mosaic_is_in_mjy_per_sr() {
        let dir = tempfile::tempdir().unwrap();
        let tree = roman_tree("wfi_mosaic-2.1.0", "    cal_step: {flux: COMPLETE, outlier_detection: COMPLETE, resample: COMPLETE, skymatch: COMPLETE}\n");
        let header = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap().header;
        assert_eq!(header.get(HEADER_BUNIT), Some("MJy/sr"));
        let source = header.get(ASDF_UNIT_SOURCE_CARD).expect("the unit source is named");
        assert!(source.contains("wfi_mosaic"), "{source}");
        assert!((one_unit_in_jy(&header) - ONE_MJY_PER_SR_JY).abs() < 1e-18);
    }

    #[test]
    fn an_older_roman_file_with_units_stored_on_the_arrays_keeps_them() {
        let dir = tempfile::tempdir().unwrap();
        let tree = "roman: !<asdf://stsci.edu/datamodels/roman/tags/wfi_image-1.0.0>\n  meta:\n    telescope: ROMAN\n    cal_step: {flux: COMPLETE}\n    photometry:\n      conversion_megajanskys: !unit/quantity-1.1.0\n        value: 0.3324\n        unit: !unit/unit-1.0.0 MJy.sr**-1\n      pixelarea_steradians: !unit/quantity-1.1.0\n        value: 2.8e-13\n        unit: !unit/unit-1.0.0 sr\n  data: !unit/quantity-1.1.0\n    value: !core/ndarray-1.0.0\n      data: [[1, 2], [3, 4]]\n      datatype: float32\n    unit: !unit/unit-1.0.0 DN / s\n";
        let header = extract_image_from_asdf(&write_asdf(&dir, tree)).unwrap().header;
        assert_eq!(header.get(HEADER_BUNIT), Some("DN / s"), "the unit stored with the array wins over the step record");
        let source = header.get(ASDF_UNIT_SOURCE_CARD).expect("the unit source is named");
        assert!(source.contains("stored with the array"), "{source}");
        assert!((one_unit_in_jy(&header) - ONE_DN_PER_SECOND_JY).abs() < 1e-18);
    }

    #[test]
    fn a_roman_tree_without_a_data_model_tag_gets_no_guessed_unit() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("roman:\n  meta:\n    telescope: ROMAN\n    cal_step: {{flux: COMPLETE}}\n{ROMAN_PHOTOMETRY}{ROMAN_ARRAYS}");
        let header = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap().header;
        assert_eq!(header.get(HEADER_BUNIT), None);
        let source = header.get(ASDF_UNIT_SOURCE_CARD).expect("the missing unit is explained");
        assert!(source.contains("unknown") && source.contains("data model"), "{source}");
        let raw = roman_tree("wfi_science_raw-1.2.0", "");
        let header = extract_plane_from_asdf(&write_asdf(&dir, &raw), Some("roman.data")).unwrap().header;
        assert_eq!(header.get(HEADER_BUNIT), None, "the decision covers level 2 images and mosaics only");
        let plain = extract_image_from_asdf(&write_asdf(&dir, INLINE_DATA)).unwrap().header;
        assert_eq!(plain.get(ASDF_UNIT_SOURCE_CARD), None, "a file that is not a Roman product carries no note");
    }

    #[test]
    fn asdf_header_does_not_touch_the_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let tree = "meta:\n  telescope: JWST\n  bunit_data: MJy/sr\ndata: !core/ndarray-1.0.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [3, 5]\ndq: !core/ndarray-1.0.0\n  source: 0\n  datatype: uint32\n  byteorder: little\n  shape: [3, 5]\n";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(tree.as_bytes());
        bytes.extend_from_slice(b"...\n");
        bytes.extend_from_slice(&[0xd3, 0x42, 0x4c, 0x4b]);
        bytes.extend_from_slice(&48u16.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"blsc");
        for size in [60u64, 60, 60] {
            bytes.extend_from_slice(&size.to_be_bytes());
        }
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(&[0u8; 60]);
        let path = dir.path().join("undecodable.asdf");
        std::fs::write(&path, bytes).unwrap();

        assert!(extract_plane_from_asdf(&path, None).is_err(), "the pixels of this block cannot be decoded");
        let header = asdf_plane_header(&path, None).expect("the header needs no pixel");
        assert_eq!(header.get("NAXIS1"), Some("5"));
        assert_eq!(header.get("NAXIS2"), Some("3"));
        assert_eq!(header.get("EXTNAME"), Some("data"));
        assert_eq!(header.get(HEADER_BUNIT), Some("MJy/sr"));
        assert_eq!(header.get("TELESCOP"), Some("JWST"));
        let dq = asdf_plane_header(&path, Some("dq")).expect("the header of a named array needs no pixel");
        assert_eq!((dq.get("EXTNAME"), dq.get(HEADER_BUNIT)), (Some("dq"), None));
        assert!(asdf_plane_header(&path, Some("nope")).is_err());
    }

    #[test]
    fn the_header_only_path_writes_the_same_cards_as_a_full_load() {
        let dir = tempfile::tempdir().unwrap();
        let nircam = dir.path().join("nircam_gwcs.asdf");
        std::fs::write(&nircam, nircam_gwcs_with_inline_data()).unwrap();
        let cube = write_asdf(&dir, CUBE_TREE);
        for (path, key) in [(nircam.as_path(), None), (cube.as_path(), Some("roman.data")), (cube.as_path(), Some("roman.dq"))] {
            let full = extract_plane_from_asdf(path, key).unwrap().header;
            let light = asdf_plane_header(path, key).unwrap();
            assert_eq!(light.cards, full.cards, "{key:?}");
        }
        let refused = format!("{:#}", asdf_plane_header(&cube, None).err().expect("the auto refusal of a ramp is kept"));
        assert!(refused.contains("not a single 2D image") && refused.contains("#array=roman.data"), "{refused}");
    }

    #[test]
    fn a_rewritten_file_never_gets_the_wcs_of_its_previous_content() {
        let dir = tempfile::tempdir().unwrap();
        let tree = |ra: &str| format!("{}{INLINE_DATA}", PURE_TAN_WCS_AT.replace("RA", ra));
        let path = write_asdf(&dir, &tree("150.0"));
        let first = extract_image_from_asdf(&path).unwrap().header;
        assert_eq!(first.get("CRVAL1"), Some("150"));
        assert_eq!(asdf_plane_header(&path, None).unwrap().cards, first.cards, "a second read of the same file gives the same cards");
        let rewritten = write_asdf(&dir, &tree("151.0"));
        assert_eq!(rewritten, path, "same path, same length, written within the same second");
        assert_eq!(extract_image_from_asdf(&path).unwrap().header.get("CRVAL1"), Some("151"));
        assert_eq!(asdf_plane_header(&path, None).unwrap().get("CRVAL1"), Some("151"));
    }

    #[test]
    fn a_sip_fit_short_of_its_target_leaves_a_note_in_the_header() {
        use crate::core::astrometry::gwcs::{fit_pipeline_tan_sip, SipFitOptions};
        let pipeline = fixture_pipeline("wcs_jwst_nircam_cal300.asdf");
        let short = fit_pipeline_tan_sip(&pipeline, &SipFitOptions { degree: Some(vec![2]), ..SipFitOptions::default() }, None).unwrap();
        assert!(!short.warnings.is_empty(), "degree 2 cannot reach 0.01 px on NIRCam: {:?}", short.max_err_px);
        let cards = gwcs_fit_cards(&short);
        let note = cards
            .iter()
            .find(|(k, _)| k == ASDF_SIP_WARNING_CARD)
            .map(|(_, v)| v.as_str())
            .expect("a fit that misses its accuracy target says so in the header");
        assert!(note.contains("0.01 px") && note.contains("not reached"), "{note}");
        assert!(note.len() <= 67, "{note}");
        let good = fit_pipeline_tan_sip(&pipeline, &SipFitOptions::default(), None).unwrap();
        assert!(good.warnings.is_empty());
        assert!(gwcs_fit_cards(&good).iter().all(|(k, _)| k != ASDF_SIP_WARNING_CARD));
    }

    #[test]
    fn an_undecoded_time_format_leaves_a_note_in_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("meta:\n  exposure:\n    start_time: !time/time-1.4.0 {{value: '2026:304:00:00:00.000', format: yday}}\n    end_time: !time/time-1.4.0 {{value: 'J2026.83', format: jyear_str}}\n{INLINE_DATA}");
        let header = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap().header;
        assert_eq!(header.get("DATE-OBS"), None);
        let note = header.get(ASDF_TIME_NOTE_CARD).expect("the missing time cards are explained");
        assert!(note.contains("yday") && note.contains("jyear_str") && note.contains("not decoded"), "{note}");
        assert!(note.len() <= 67, "{note}");
        let decoded = extract_image_from_asdf(&write_asdf(&dir, ROMAN_TIMES)).unwrap().header;
        assert_eq!(decoded.get(ASDF_TIME_NOTE_CARD), None);
    }

    const PURE_TAN_WCS: &str = "wcs:\n  steps:\n    - transform: !transform/compose-1.2.0\n        forward:\n          - !transform/shift-1.2.0 {offset: -10.0}\n          - !transform/shift-1.2.0 {offset: -10.0}\n          - !transform/scale-1.2.0 {factor: 0.001}\n          - !transform/scale-1.2.0 {factor: 0.001}\n          - !transform/gnomonic-1.2.0 {direction: pix2sky}\n          - !transform/rotate3d-1.3.0 {phi: 150.0, theta: 30.0, psi: 180.0, direction: native2celestial}\n    - frame: {name: world}\n";
    const PURE_TAN_WCS_AT: &str = "wcs:\n  steps:\n    - transform: !transform/compose-1.2.0\n        forward:\n          - !transform/shift-1.2.0 {offset: -10.0}\n          - !transform/shift-1.2.0 {offset: -10.0}\n          - !transform/scale-1.2.0 {factor: 0.001}\n          - !transform/scale-1.2.0 {factor: 0.001}\n          - !transform/gnomonic-1.2.0 {direction: pix2sky}\n          - !transform/rotate3d-1.3.0 {phi: RA, theta: 30.0, psi: 180.0, direction: native2celestial}\n    - frame: {name: world}\n";

    #[test]
    fn an_array_off_the_science_grid_gets_no_wcs_cards() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("{PURE_TAN_WCS}{INLINE_DATA}err: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\namp33: !core/ndarray-1.0.0\n  data: [[1, 2, 3, 4, 5], [6, 7, 8, 9, 10], [1, 2, 3, 4, 5]]\n  datatype: float32\n");
        let path = write_asdf(&dir, &tree);
        for key in [None, Some("err")] {
            let header = extract_plane_from_asdf(&path, key).unwrap().header;
            assert_eq!(header.get("CTYPE1"), Some("RA---TAN"), "{key:?}");
            assert_eq!(header.get("CRVAL1"), Some("150"), "{key:?}");
        }
        let amp = extract_plane_from_asdf(&path, Some("amp33")).unwrap().header;
        for card in ["CTYPE1", "CRVAL1", "CRPIX1", "CDELT1", "PC1_1"] {
            assert_eq!(amp.get(card), None, "{card}: a 3x5 array is not on the 2x2 science grid");
        }
        assert_eq!(amp.get("NAXIS1"), Some("5"));
    }

    #[test]
    fn a_complex_array_says_that_its_modulus_is_shown() {
        let dir = tempfile::tempdir().unwrap();
        let tree = "data: !core/ndarray-1.0.0\n  source: 0\n  datatype: complex64\n  byteorder: little\n  shape: [1, 2]\n";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(tree.as_bytes());
        bytes.extend_from_slice(b"...\n");
        bytes.extend_from_slice(&[0xd3, 0x42, 0x4c, 0x4b]);
        bytes.extend_from_slice(&48u16.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        for size in [16u64, 16, 16] {
            bytes.extend_from_slice(&size.to_be_bytes());
        }
        bytes.extend_from_slice(&[0u8; 16]);
        for v in [3.0f32, 4.0, 0.0, -2.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let path = dir.path().join("complex.asdf");
        std::fs::write(&path, bytes).unwrap();
        let loaded = extract_image_from_asdf(&path).unwrap();
        assert_eq!(loaded.image.as_slice().unwrap(), &[5.0, 2.0]);
        let note = loaded.header.get(ASDF_COMPLEX_NOTE_CARD).expect("the loss of the phase is stated");
        assert!(note.contains("complex64") && note.contains("modulus"), "{note}");
        let real = extract_image_from_asdf(&write_asdf(&dir, INLINE_DATA)).unwrap().header;
        assert_eq!(real.get(ASDF_COMPLEX_NOTE_CARD), None);
    }

    const FK5_J1975_WCS: &str = "wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>\n  name: ''\n  steps:\n  - !<tag:stsci.edu:gwcs/step-1.3.0>\n    frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0> {axes_names: [x, y], axes_order: [0, 1], axis_physical_types: ['custom:x', 'custom:y'], name: detector, unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]}\n    transform: !transform/compose-1.2.0\n      forward:\n      - !transform/compose-1.2.0\n        forward:\n        - !transform/compose-1.2.0\n          forward:\n          - !transform/concatenate-1.2.0\n            forward:\n            - !transform/shift-1.2.0 {offset: -1.5}\n            - !transform/shift-1.2.0 {offset: -1.5}\n          - !transform/concatenate-1.2.0\n            forward:\n            - !transform/scale-1.2.0 {factor: 0.001}\n            - !transform/scale-1.2.0 {factor: 0.001}\n        - !transform/gnomonic-1.2.0 {direction: pix2sky}\n      - !transform/rotate3d-1.3.0 {phi: 150.0, theta: 30.0, psi: 180.0, direction: native2celestial}\n  - !<tag:stsci.edu:gwcs/step-1.3.0>\n    frame: !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>\n      axes_names: [lon, lat]\n      axes_order: [0, 1]\n      axis_physical_types: [pos.eq.ra, pos.eq.dec]\n      name: world\n      reference_frame: !<tag:astropy.org:astropy/coordinates/frames/fk5-1.1.0>\n        frame_attributes:\n          equinox: !time/time-1.1.0 J1975.000\n      unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]\n    transform: null\n";

    #[test]
    fn an_fk5_gwcs_of_another_equinox_is_precessed_to_j2000_in_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("{FK5_J1975_WCS}{}", INLINE_DATA.replace("[[1, 2], [3, 4]]", "[[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12], [13, 14, 15, 16]]"));
        let header = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap().header;
        let wcs = WcsTransform::from_header(&header).unwrap();
        let sky = wcs.pixel_to_world(1.5, 1.5);
        let astropy_fk5_j2000 = (150.36012784985985, 29.87924127015363);
        let off_mas = angular_separation(sky.ra, sky.dec, astropy_fk5_j2000.0, astropy_fk5_j2000.1) * 3.6e6;
        assert!(off_mas < 1e-3, "({}, {}) is {off_mas} mas from astropy 8.0.2 FK5 J1975 -> J2000 (scripts/oracle/fk5_precession_goldens.py)", sky.ra, sky.dec);
        let note = header.get(ASDF_FRAME_NOTE_CARD).expect("the frame handling is stated");
        assert!(note.contains("FK5") && note.contains("J1975") && note.contains("J2000") && note.contains("22 mas"), "{note}");
        assert!(note.len() <= 67, "{note}");
        assert_eq!(header.get(ASDF_WCS_NOTE_CARD), None);
    }

    #[test]
    fn an_err_plane_without_its_own_unit_takes_the_science_unit() {
        let dir = tempfile::tempdir().unwrap();
        let science_only = "meta:\n  bunit_data: MJy/sr\ndata: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\nerr: !core/ndarray-1.0.0\n  data: [[0.1, 0.2], [0.3, 0.4]]\n  datatype: float32\n";
        let err = extract_plane_from_asdf(&write_asdf(&dir, science_only), Some("err")).unwrap();
        assert_eq!(err.header.get(HEADER_BUNIT), Some("MJy/sr"));

        let roman = "roman: !<asdf://stsci.edu/datamodels/roman/tags/wfi_image-1.2.0>\n  meta:\n    cal_step: {flux: INCOMPLETE}\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  err: !core/ndarray-1.0.0\n    data: [[0.1, 0.2], [0.3, 0.4]]\n    datatype: float32\n";
        let err = extract_plane_from_asdf(&write_asdf(&dir, roman), Some("roman.err")).unwrap();
        assert_eq!(err.header.get(HEADER_BUNIT), Some("DN/s"));
    }

    fn interleaved_tree() -> String {
        let values: Vec<String> = (0..30).map(|v| v.to_string()).collect();
        format!(
            "rgb: !core/ndarray-1.0.0\n  data: [{}]\n  shape: [5, 2, 3]\n  datatype: float32\n",
            values.join(", ")
        )
    }

    #[test]
    fn multi_plane_arrays_stay_listed_with_their_real_plane_count() {
        let dir = tempfile::tempdir().unwrap();
        let arrays = list_asdf_arrays(&write_asdf(&dir, CUBE_TREE)).unwrap();
        let cube = arrays
            .iter()
            .find(|e| e.extname.as_deref() == Some("roman.data"))
            .expect("the ramp is still listed");
        assert!(cube.has_data, "a cube still holds pixel data");
        assert_eq!(cube.naxis, 3);
        assert_eq!(cube.naxis1, 2);
        assert_eq!(cube.naxis2, 2);
        assert_eq!(cube.naxis3, 3);

        let path = write_asdf(&dir, &interleaved_tree());
        let listed = list_asdf_arrays(&path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].naxis, 3);
        assert_eq!(listed[0].naxis1, 2);
        assert_eq!(listed[0].naxis2, 5);
        assert_eq!(listed[0].naxis3, 3);

        let refused = extract_plane_from_asdf(&path, None)
            .err()
            .expect("an interleaved 3-channel array is not a single 2D image");
        assert!(format!("{refused:#}").contains("3D array [5x2x3] of 3 planes"), "{refused:#}");
    }

    #[test]
    fn an_interleaved_colour_array_is_recovered_as_an_rgb_composite() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, &interleaved_tree());

        let rgb = try_extract_rgb_from_asdf(&path)
            .unwrap()
            .expect("a [height, width, 3] array is a colour image, not a ramp");
        assert_eq!(rgb.r.dim(), (5, 2));
        assert_eq!(rgb.r.as_slice().unwrap(), &[0.0, 3.0, 6.0, 9.0, 12.0, 15.0, 18.0, 21.0, 24.0, 27.0]);
        assert_eq!(rgb.g.as_slice().unwrap(), &[1.0, 4.0, 7.0, 10.0, 13.0, 16.0, 19.0, 22.0, 25.0, 28.0]);
        assert_eq!(rgb.b.as_slice().unwrap(), &[2.0, 5.0, 8.0, 11.0, 14.0, 17.0, 20.0, 23.0, 26.0, 29.0]);
        assert_eq!(rgb.header.get(ASDF_PLANE_COUNT), Some("3"));

        let explicit = extract_plane_from_asdf(&path, Some("rgb")).expect("explicit reference opens the red plane");
        assert_eq!(explicit.image, rgb.r);
    }

    #[test]
    fn a_planar_ramp_is_never_mistaken_for_a_colour_image() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            try_extract_rgb_from_asdf(&write_asdf(&dir, CUBE_TREE)).unwrap().is_none(),
            "a [3, height, width] Roman ramp is three resultant reads, not three colour channels"
        );
        assert!(
            try_extract_rgb_from_asdf(&write_asdf(&dir, INLINE_DATA)).unwrap().is_none(),
            "a plain 2D array has no colour planes"
        );
    }

    #[test]
    fn degenerate_leading_axis_loads_and_keeps_its_rank_in_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let tree = "data: !core/ndarray-1.0.0\n  data: [[[1, 2, 3], [4, 5, 6]]]\n  datatype: float32\n";
        let result = extract_plane_from_asdf(&write_asdf(&dir, tree), None).unwrap();
        assert_eq!(result.image.dim(), (2, 3));
        assert_eq!(result.image[[1, 2]], 6.0);
        assert_eq!(result.header.get("NAXIS"), Some("3"));
        assert_eq!(result.header.get("NAXIS1"), Some("3"));
        assert_eq!(result.header.get("NAXIS2"), Some("2"));
        assert_eq!(result.header.get("NAXIS3"), Some("1"));
        assert_eq!(result.selected_extension.as_deref(), Some("data"));
    }

    #[test]
    fn plain_2d_array_still_reports_naxis_2() {
        let dir = tempfile::tempdir().unwrap();
        let result = extract_image_from_asdf(&write_asdf(&dir, INLINE_DATA)).unwrap();
        assert_eq!(result.header.get("NAXIS"), Some("2"));
        assert_eq!(result.header.get("NAXIS3"), None);
        assert_eq!(result.image.dim(), (2, 2));
    }

    #[test]
    fn no_unit_anywhere_leaves_header_without_bunit() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("meta:\n  instrument: {{name: WFI}}\n{INLINE_DATA}");
        let result = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap();
        assert_eq!(result.header.get(HEADER_BUNIT), None);
        assert_eq!(result.header.get("ASDF_SRC"), Some("true"));
    }
}
