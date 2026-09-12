use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::infra::asdf::converter::{is_asdf_file, list_arrays, AsdfArrayInfo, AsdfImage};
use crate::infra::asdf::AsdfFile;
use crate::infra::fits::reader::{HduInfo, MmapImageResult};
use crate::types::constants::HEADER_BUNIT;
use crate::types::image::IntPlane;
use crate::types::HduHeader;

const BUNIT_METADATA_KEYS: [&str; 4] = ["meta.bunit_data", "meta.bunit", "roman.meta.bunit", "header.BUNIT"];

fn resolve_bunit(asdf_img: &AsdfImage) -> Option<String> {
    asdf_img
        .unit
        .iter()
        .map(String::as_str)
        .chain(
            BUNIT_METADATA_KEYS
                .iter()
                .filter_map(|k| asdf_img.metadata.get(*k).map(String::as_str)),
        )
        .map(str::trim)
        .find(|v| !v.is_empty())
        .map(str::to_string)
}

pub fn extract_image_from_asdf(path: &Path) -> Result<MmapImageResult> {
    extract_plane_from_asdf(path, None)
}

fn array_info_to_hdu(i: usize, a: &AsdfArrayInfo) -> HduInfo {
    let rank = a.shape.len();
    HduInfo {
        index: i,
        extname: Some(a.key.clone()),
        naxis: rank as i64,
        naxis1: a.shape.get(rank.wrapping_sub(1)).copied().unwrap_or(0) as i64,
        naxis2: a.shape.get(rank.wrapping_sub(2)).copied().unwrap_or(0) as i64,
        naxis3: if rank == 3 { a.shape[0] as i64 } else { 0 },
        bitpix: a.bitpix,
        has_data: rank >= 2,
        extver: None,
        header_start: 0,
        data_start: 0,
    }
}

pub fn list_asdf_arrays(path: &Path) -> Result<Vec<HduInfo>> {
    let asdf = AsdfFile::open(path).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    Ok(list_arrays(&asdf)
        .iter()
        .enumerate()
        .map(|(i, a)| array_info_to_hdu(i, a))
        .collect())
}

pub fn extract_int_plane_from_asdf(path: &Path, key: &str) -> Result<Option<IntPlane>> {
    let asdf = AsdfFile::open(path).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    AsdfImage::load_array_int(&asdf, key).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))
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
    let asdf_img = match key {
        Some(k) => AsdfImage::load_array(asdf, k),
        None => AsdfImage::from_file(asdf),
    }
    .map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;

    let has_image = asdf_img.has_image();
    if !has_image {
        anyhow::bail!("ASDF load failed: Missing field: data array");
    }
    let data_key = asdf_img
        .metadata
        .get("ASDF_DATA_KEY")
        .cloned()
        .unwrap_or_default();
    let (width, height) = (asdf_img.width, asdf_img.height);
    let naxis_str = "2";

    let mut cards = Vec::new();
    let mut index = HashMap::new();

    push_card(&mut cards, &mut index, "NAXIS", naxis_str.into());
    push_card(&mut cards, &mut index, "NAXIS1", width.to_string());
    push_card(&mut cards, &mut index, "NAXIS2", height.to_string());
    push_card(&mut cards, &mut index, "BITPIX", "-32".into());
    push_card(&mut cards, &mut index, "EXTNAME", data_key.clone());
    push_card(&mut cards, &mut index, "ASDF_DATA_KEY", data_key.clone());

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
    }

    let mut extra: Vec<(&String, &String)> = asdf_img.metadata.iter().collect();
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
        if let Some(unit) = resolve_bunit(&asdf_img) {
            push_card(&mut cards, &mut index, HEADER_BUNIT, unit);
        }
    }

    push_card(&mut cards, &mut index, "ASDF_SRC", "true".into());

    let header = HduHeader { cards, index };

    let extensions: Vec<HduInfo> = list_arrays(asdf)
        .iter()
        .enumerate()
        .map(|(i, a)| array_info_to_hdu(i, a))
        .collect();
    let extension_count = extensions.len();

    Ok(MmapImageResult {
        header,
        image: asdf_img.into_array2(),
        is_mef: false,
        selected_extension: Some(data_key),
        extension_count,
        extensions,
    })
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

pub fn is_asdf_path(path: &Path) -> bool {
    is_asdf_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(result.header.get("ASDF_DATA_KEY"), Some("dq"));
        assert_eq!(result.header.get("META_TELESCOPE"), Some("JWST"));
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
        assert_eq!(auto.extension_count, 3);
        assert!(extract_plane_from_asdf(&path, Some("nope")).is_err());
    }

    #[test]
    fn list_asdf_arrays_and_int_plane_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, DQ_TREE);
        let arrays = list_asdf_arrays(&path).unwrap();
        assert_eq!(arrays.len(), 3);
        assert_eq!(arrays[2].extname.as_deref(), Some("err"));
        let plane = extract_int_plane_from_asdf(&path, "dq").unwrap().unwrap();
        assert_eq!(plane.bits[[1, 0]], 3);
        assert_eq!(plane.bits[[0, 1]], 1);
        assert!(extract_int_plane_from_asdf(&path, "data").unwrap().is_none());
        assert!(extract_int_plane_from_asdf(&path, "missing").is_err());
    }

    #[test]
    fn companion_key_replaces_last_segment() {
        assert_eq!(companion_key("roman.data", "dq"), "roman.dq");
        assert_eq!(companion_key("data", "err"), "err");
        assert_eq!(companion_key("sci", "dq"), "dq");
        assert_eq!(companion_key("a.b.c", "err"), "a.b.err");
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
