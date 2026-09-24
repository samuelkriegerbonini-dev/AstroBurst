use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ndarray::Array2;

use crate::infra::asdf::converter::{
    auto_data_key, is_interleaved_layout, list_arrays, plane_geometry, shape_label,
    AsdfArrayInfo, AsdfImage,
};
use crate::infra::asdf::AsdfFile;
use crate::infra::fits::reader::{HduInfo, MmapImageResult};
use crate::types::constants::HEADER_BUNIT;
use crate::types::HduHeader;

const SCIENCE_BUNIT_KEYS: [&str; 4] = ["meta.bunit_data", "meta.bunit", "roman.meta.bunit", "header.BUNIT"];
const ERROR_BUNIT_KEYS: [&str; 1] = ["meta.bunit_err"];
const SCIENCE_ARRAY_NAMES: [&str; 4] = ["data", "sci", "science", "image"];
const NON_SCIENCE_ARRAY_NAMES: [&str; 2] = ["dq", "mask"];

pub const ASDF_SELECTED_PLANE: &str = "ASDFPLAN";
pub const ASDF_PLANE_COUNT: &str = "ASDFNPLN";
pub const ASDF_DATA_KEY_CARD: &str = "ASDFKEY";
pub const ASDF_WCS_NOTE_CARD: &str = "ASDFWCS";

struct FitsAlias {
    card: &'static str,
    sources: &'static [&'static str],
    numeric: bool,
    photometric: bool,
}

const FITS_ALIASES: [FitsAlias; 6] = [
    FitsAlias {
        card: "TELESCOP",
        sources: &["meta.telescope", "roman.meta.telescope", "header.TELESCOP"],
        numeric: false,
        photometric: false,
    },
    FitsAlias {
        card: "INSTRUME",
        sources: &["meta.instrument.name", "roman.meta.instrument.name", "header.INSTRUME"],
        numeric: false,
        photometric: false,
    },
    FitsAlias {
        card: "EXPTIME",
        sources: &["meta.exposure.exposure_time", "roman.meta.exposure.exposure_time", "header.EXPTIME"],
        numeric: true,
        photometric: false,
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
    },
    FitsAlias {
        card: "PIXAR_SR",
        sources: &[
            "meta.photometry.pixelarea_steradians",
            "roman.meta.photometry.pixelarea_steradians",
            "header.PIXAR_SR",
        ],
        numeric: true,
        photometric: true,
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

fn resolve_bunit(asdf_img: &AsdfImage, role: ArrayRole) -> Option<String> {
    let fallbacks: &[&[&str]] = match role {
        ArrayRole::Science => &[&SCIENCE_BUNIT_KEYS],
        ArrayRole::Error => &[&ERROR_BUNIT_KEYS, &SCIENCE_BUNIT_KEYS],
        ArrayRole::Other => &[],
    };
    asdf_img
        .unit
        .iter()
        .map(String::as_str)
        .chain(
            fallbacks
                .iter()
                .flat_map(|keys| keys.iter())
                .filter_map(|k| asdf_img.metadata.get(*k).map(String::as_str)),
        )
        .map(str::trim)
        .find(|v| !v.is_empty())
        .map(str::to_string)
}

fn metadata_value<'a>(metadata: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    metadata
        .get(key)
        .or_else(|| metadata.get(&format!("{key}.value")))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
}

fn alias_value(metadata: &HashMap<String, String>, alias: &FitsAlias) -> Option<String> {
    alias
        .sources
        .iter()
        .filter_map(|k| metadata_value(metadata, k))
        .find(|v| !alias.numeric || v.parse::<f64>().is_ok_and(f64::is_finite))
        .map(str::to_string)
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
        bitpix: a.bitpix,
        has_data: rank >= 2,
        extver: None,
        header_start: 0,
        data_start: 0,
    }
}

fn multi_plane_refusal(key: &str, shape: &[usize], planes: usize) -> anyhow::Error {
    anyhow::anyhow!(
        "ASDF array '{}' cannot be loaded as a 2D image: {}D array [{}] of {} planes, not a single 2D image",
        key,
        shape.len(),
        shape_label(shape),
        planes
    )
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

pub fn extract_plane_from_open_asdf(asdf: &AsdfFile, key: Option<&str>) -> Result<MmapImageResult> {
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

    let asdf_img = match key {
        Some(k) => AsdfImage::load_array(asdf, k),
        None => AsdfImage::from_file(asdf),
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
    let header = synthesise_header(&asdf_img, &data_key, plane_count, role);
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
    let header = synthesise_header(&asdf_img, &key, plane_count, ArrayRole::Science);
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

fn synthesise_header(asdf_img: &AsdfImage, data_key: &str, plane_count: usize, role: ArrayRole) -> HduHeader {
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

    if let (None, Some(note)) = (&asdf_img.wcs, &asdf_img.wcs_note) {
        push_card(&mut cards, &mut index, ASDF_WCS_NOTE_CARD, note.clone());
    }

    if let Some(ref wcs) = asdf_img.wcs {
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
    }

    for alias in &FITS_ALIASES {
        if alias.photometric && role == ArrayRole::Other {
            continue;
        }
        if let Some(value) = alias_value(&asdf_img.metadata, alias) {
            push_card(&mut cards, &mut index, alias.card, value);
        }
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

    if !index.contains_key(HEADER_BUNIT) {
        if let Some(unit) = resolve_bunit(asdf_img, role) {
            push_card(&mut cards, &mut index, HEADER_BUNIT, unit);
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
    use crate::core::metadata::photcal::PhotCal;
    use crate::types::image::IntPlane;

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
    fn fallback_chain_reaches_roman_and_header_keys() {
        let dir = tempfile::tempdir().unwrap();
        let tree = format!("roman:\n  meta:\n    bunit: electron\n{INLINE_DATA}");
        let result = extract_image_from_asdf(&write_asdf(&dir, &tree)).unwrap();
        assert_eq!(result.header.get(HEADER_BUNIT), Some("electron"));

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
        let tree = "roman:\n  meta:\n    telescope: ROMAN\n    instrument: {name: WFI}\n    exposure: {exposure_time: 107.0}\n    bunit: DN / s\n    photometry:\n      conversion_megajanskys: !unit/quantity-1.1.0\n        value: 0.3324\n        unit: !unit/unit-1.0.0 MJy.sr**-1\n      pixelarea_steradians: !unit/quantity-1.1.0\n        value: 2.8e-13\n        unit: !unit/unit-1.0.0 sr\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n";
        let loaded = extract_image_from_asdf(&write_asdf(&dir, tree)).unwrap();
        let legal = fits_legal(&loaded.header);

        assert_eq!(legal.get(ASDF_DATA_KEY_CARD), Some("roman.data"));
        assert_eq!(legal.get("TELESCOP"), Some("ROMAN"));
        assert_eq!(legal.get("INSTRUME"), Some("WFI"));
        assert_eq!(legal.get("EXPTIME"), Some("107.0"));
        assert_eq!(legal.get(HEADER_BUNIT), Some("DN / s"));

        let full = PhotCal::from_header(&loaded.header, None).expect("ASDF header calibrates");
        let written = PhotCal::from_header(&legal, None).expect("FITS-legal cards still calibrate");
        let one = |cal: &PhotCal| cal.calibrate(1.0, None).map(|f| f.flux_jy).unwrap();
        assert!((one(&full) - 0.3324 * 2.8e-13 * 1e6).abs() < 1e-18);
        assert!((one(&written) - one(&full)).abs() < 1e-18);
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

        let wcs = crate::core::astrometry::wcs::WcsTransform::from_header(&header).unwrap();
        let sky = wcs.pixel_to_world(20.0, 10.0);
        assert!((sky.ra - 90.0).abs() < 1e-9, "gwcs puts +x at RA 90 on the pole, got {}", sky.ra);
        assert!((sky.dec - 89.99).abs() < 1e-6, "{}", sky.dec);
    }

    #[test]
    fn an_err_plane_without_its_own_unit_takes_the_science_unit() {
        let dir = tempfile::tempdir().unwrap();
        let science_only = "meta:\n  bunit_data: MJy/sr\ndata: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\nerr: !core/ndarray-1.0.0\n  data: [[0.1, 0.2], [0.3, 0.4]]\n  datatype: float32\n";
        let err = extract_plane_from_asdf(&write_asdf(&dir, science_only), Some("err")).unwrap();
        assert_eq!(err.header.get(HEADER_BUNIT), Some("MJy/sr"));

        let roman = "roman:\n  meta:\n    bunit: DN / s\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  err: !core/ndarray-1.0.0\n    data: [[0.1, 0.2], [0.3, 0.4]]\n    datatype: float32\n";
        let err = extract_plane_from_asdf(&write_asdf(&dir, roman), Some("roman.err")).unwrap();
        assert_eq!(err.header.get(HEADER_BUNIT), Some("DN / s"));
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
