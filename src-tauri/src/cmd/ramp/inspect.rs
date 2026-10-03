use std::fs::File;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::json;
use serde_yaml::Value;

use crate::cmd::common::{blocking_cmd, save_auto_stf_preview_png, write_derived_fits, HEADER_ABPROC};
use crate::cmd::cube::{cube_frame_json, CARD_GROUP, CARD_INTEGRATION};
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::ramp::info::{ramp_info, roman_ramp_info, RampInfo};
use crate::infra::asdf::converter::{
    is_asdf_file, is_interleaved_layout, list_arrays, plane_geometry, AsdfArrayInfo, AsdfImage,
};
use crate::infra::asdf::tree::untag;
use crate::infra::asdf::AsdfFile;
use crate::infra::fits::reader::{extract_header_by_index, list_extensions, read_primary_header, HduInfo};
use crate::infra::fits::table::{list_tables, load_bintable, Column, ColumnInfo, ColumnValues, TableHdu};
use crate::types::constants::{
    HEADER_BUNIT, RES_FITS_PATH, RES_FRAME_INDEX, RES_FRAMES, RES_HEIGHT, RES_OUTPUT_PATH, RES_RAMP, RES_UNIT, RES_WIDTH,
};
use crate::types::header::HduHeader;
use crate::types::image_ref::path_key;
use crate::types::{ImageRef, PlaneSelector};

pub const RAMP_TABLE_TYPE_CODES: [char; 6] = ['B', 'I', 'J', 'K', 'E', 'D'];
pub const DEFAULT_TABLE_ROWS: usize = 200;
pub const ABPROC_RAMP_FRAME: &str = "ramp-frame";
pub const RAMP_UNIT_DN: &str = "DN";
pub const EXTNAME_SCI: &str = "SCI";
pub const EXTNAME_GROUP: &str = "GROUP";
pub const EXTNAME_INT_TIMES: &str = "INT_TIMES";
pub const ROMAN_FRAME_TIME_KEY: &str = "roman.meta.exposure.frame_time";
pub const ROMAN_READ_PATTERN_KEY: &str = "roman.meta.exposure.read_pattern";
pub const ROMAN_NRESULTANTS_KEY: &str = "roman.meta.exposure.nresultants";
pub const ROMAN_MA_TABLE_KEY: &str = "roman.meta.exposure.ma_table_name";
pub const ROMAN_DETECTOR_KEY: &str = "roman.meta.instrument.detector";

const SOURCE_FITS: &str = "fits";
const SOURCE_ASDF: &str = "asdf";
const KEY_SOURCE: &str = "source";
const KEY_ARRAY_KEY: &str = "array_key";
const KEY_X: &str = "x";
const KEY_Y: &str = "y";
const KEY_INTEGRATION: &str = "integration";
const KEY_NGROUPS: &str = "ngroups";
const KEY_NINTS: &str = "nints";
const KEY_VALUES: &str = "values";
const KEY_GROUP_TIMES: &str = "group_times_s";
const KEY_GROUP_TABLE: &str = "group";
const KEY_INT_TIMES_TABLE: &str = "int_times";
const ASDF_DATA_SUFFIX: &str = "data";
const YAML_VALUE_KEY: &str = "value";

#[derive(Debug, Clone, Serialize)]
pub struct TableColumnPreview {
    pub name: String,
    pub unit: Option<String>,
    pub values: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TablePreview {
    pub hdu: usize,
    pub extname: String,
    pub n_rows: usize,
    pub shown_rows: usize,
    pub columns: Vec<TableColumnPreview>,
    pub omitted_columns: Vec<String>,
}

struct AsdfRampEntry {
    path: String,
    key: String,
    mtime: Option<SystemTime>,
    image: Arc<AsdfImage>,
}

static ASDF_RAMP_CACHE: Mutex<Option<AsdfRampEntry>> = Mutex::new(None);

#[cfg(test)]
static ASDF_RAMP_CACHE_EVENTS: Mutex<(usize, usize)> = Mutex::new((0, 0));

#[cfg(test)]
fn note_cache_event(hit: bool) {
    let mut events = ASDF_RAMP_CACHE_EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    if hit {
        events.0 += 1;
    } else {
        events.1 += 1;
    }
}

#[cfg(not(test))]
fn note_cache_event(_hit: bool) {}

fn asdf_ramp_image(path: &str, key: &str) -> Result<Arc<AsdfImage>> {
    let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let mut slot = ASDF_RAMP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = slot.as_ref() {
        if entry.path == path && entry.key == key && entry.mtime == mtime {
            note_cache_event(true);
            return Ok(Arc::clone(&entry.image));
        }
    }
    note_cache_event(false);
    slot.take();
    let asdf = AsdfFile::open(path).map_err(|e| anyhow!("ASDF load failed: {}", e))?;
    let image = Arc::new(AsdfImage::load_array(&asdf, key).map_err(|e| anyhow!("ASDF load failed: {}", e))?);
    *slot = Some(AsdfRampEntry { path: path.to_string(), key: key.to_string(), mtime, image: Arc::clone(&image) });
    Ok(image)
}

pub(crate) fn release_asdf_ramp(path: &str) {
    let source = path_key(&ImageRef::parse(path).path);
    let mut slot = ASDF_RAMP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|entry| path_key(&entry.path) == source) {
        slot.take();
    }
}

fn is_asdf_ref(reference: &ImageRef) -> bool {
    matches!(reference.plane, PlaneSelector::Array(_)) || is_asdf_file(&reference.path)
}

fn tree_node<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    let mut node = tree;
    for part in key.split('.') {
        node = untag(node).as_mapping()?.get(Value::String(part.to_string()))?;
    }
    Some(untag(node))
}

fn scalar_f64(node: &Value) -> Option<f64> {
    match untag(node) {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Mapping(m) => m.get(Value::String(YAML_VALUE_KEY.to_string())).and_then(scalar_f64),
        _ => None,
    }
}

fn tree_f64(tree: &Value, key: &str) -> Option<f64> {
    tree_node(tree, key).and_then(scalar_f64).filter(|v| v.is_finite())
}

fn tree_usize(tree: &Value, key: &str) -> Option<usize> {
    tree_f64(tree, key).filter(|v| v.fract() == 0.0 && *v >= 0.0).map(|v| v as usize)
}

fn tree_str(tree: &Value, key: &str) -> Option<String> {
    match tree_node(tree, key)? {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn tree_read_pattern(tree: &Value, key: &str) -> Option<Vec<Vec<usize>>> {
    let Value::Sequence(resultants) = tree_node(tree, key)? else {
        return None;
    };
    resultants
        .iter()
        .map(|reads| match untag(reads) {
            Value::Sequence(items) => items
                .iter()
                .map(|v| scalar_f64(v).filter(|r| r.fract() == 0.0 && *r >= 0.0).map(|r| r as usize))
                .collect(),
            other => scalar_f64(other).filter(|r| r.fract() == 0.0 && *r >= 0.0).map(|r| vec![r as usize]),
        })
        .collect()
}

struct AsdfRampSource {
    key: String,
    shape: Vec<usize>,
    ramp: Option<RampInfo>,
}

fn is_resultant_stack(info: &AsdfArrayInfo) -> bool {
    info.shape.len() == 3 && info.shape[0] >= 2 && !is_interleaved_layout(&info.shape)
}

fn pick_ramp_array(arrays: &[AsdfArrayInfo], plane: &PlaneSelector) -> Option<AsdfArrayInfo> {
    if let PlaneSelector::Array(key) = plane {
        return arrays.iter().find(|a| &a.key == key).cloned();
    }
    let stacks: Vec<&AsdfArrayInfo> = arrays.iter().filter(|a| is_resultant_stack(a)).collect();
    stacks
        .iter()
        .find(|a| a.key == ASDF_DATA_SUFFIX || a.key.ends_with(&format!(".{}", ASDF_DATA_SUFFIX)))
        .or(stacks.first())
        .map(|a| (*a).clone())
        .or_else(|| arrays.first().cloned())
}

fn asdf_ramp_source(path: &str, plane: &PlaneSelector) -> Result<AsdfRampSource> {
    let asdf = AsdfFile::open(path).map_err(|e| anyhow!("ASDF load failed: {}", e))?;
    let array = pick_ramp_array(&list_arrays(&asdf), plane)
        .with_context(|| format!("{} holds no array that can be read as a ramp", path))?;
    let tree = &asdf.tree;
    let ramp = if is_resultant_stack(&array) {
        let read_pattern = tree_read_pattern(tree, ROMAN_READ_PATTERN_KEY);
        roman_ramp_info(
            &array.shape,
            tree_f64(tree, ROMAN_FRAME_TIME_KEY),
            read_pattern.as_deref(),
            tree_usize(tree, ROMAN_NRESULTANTS_KEY),
            tree_str(tree, ROMAN_MA_TABLE_KEY).as_deref(),
            tree_str(tree, ROMAN_DETECTOR_KEY).as_deref(),
        )
    } else {
        None
    };
    Ok(AsdfRampSource { key: array.key, shape: array.shape, ramp })
}

fn is_named(extname: Option<&str>, wanted: &str) -> bool {
    extname.is_some_and(|n| n.trim().trim_matches('\'').trim().eq_ignore_ascii_case(wanted))
}

fn sci_hdu_index(hdus: &[HduInfo]) -> usize {
    hdus.iter()
        .find(|h| is_named(h.extname.as_deref(), EXTNAME_SCI))
        .or_else(|| hdus.iter().find(|h| h.naxis >= 3))
        .map_or(0, |h| h.index)
}

fn header_axis(header: &HduHeader, key: &str) -> usize {
    header.get_i64(key).and_then(|v| usize::try_from(v).ok()).unwrap_or(0)
}

fn fits_ramp_info_json(path: &str) -> Result<serde_json::Value> {
    let (ramp, frames, width, height) = match GLOBAL_CUBE_CACHE.get_or_open(path) {
        Ok(cube) => {
            let g = &cube.geometry;
            (cube.ramp(), g.depth, g.naxis1, g.naxis2)
        }
        Err(cube_error) => {
            let reference = ImageRef::parse(path);
            let file = File::open(&reference.path).with_context(|| format!("{:#}", cube_error))?;
            let hdus = list_extensions(&file)?;
            let index = match reference.plane {
                PlaneSelector::Hdu(n) => n,
                _ => sci_hdu_index(&hdus),
            };
            let sci = extract_header_by_index(&file, index)?;
            let merged = if index == 0 { sci.clone() } else { read_primary_header(&reference.path)?.merge_with(&sci) };
            let ramp = ramp_info(&merged);
            let frames = ramp
                .as_ref()
                .map_or(header_axis(&sci, "NAXIS3") * header_axis(&sci, "NAXIS4").max(1), |r| r.ngroups * r.nints);
            (ramp, frames, header_axis(&sci, "NAXIS1"), header_axis(&sci, "NAXIS2"))
        }
    };
    Ok(json!({
        KEY_SOURCE: SOURCE_FITS,
        RES_RAMP: ramp,
        RES_FRAMES: frames,
        RES_WIDTH: width,
        RES_HEIGHT: height,
        KEY_ARRAY_KEY: serde_json::Value::Null,
    }))
}

pub(crate) fn ramp_info_json(path: &str) -> Result<serde_json::Value> {
    let reference = ImageRef::parse(path);
    if !is_asdf_ref(&reference) {
        return fits_ramp_info_json(path);
    }
    let source = asdf_ramp_source(&reference.path, &reference.plane)?;
    let geometry = plane_geometry(&source.shape);
    Ok(json!({
        KEY_SOURCE: SOURCE_ASDF,
        RES_RAMP: source.ramp,
        RES_FRAMES: geometry.plane_count,
        RES_WIDTH: geometry.width,
        RES_HEIGHT: geometry.height,
        KEY_ARRAY_KEY: source.key,
    }))
}

fn asdf_pixel_series(image: &AsdfImage, x: usize, y: usize) -> Result<Vec<f32>> {
    let (width, height) = (image.width, image.height);
    if x >= width || y >= height {
        bail!("Pixel ({}, {}) out of bounds", y, x);
    }
    let plane_len = width * height;
    (0..image.plane_count())
        .map(|g| {
            image
                .data
                .get(g * plane_len + y * width + x)
                .copied()
                .with_context(|| format!("resultant {} is missing from the ASDF array", g))
        })
        .collect()
}

pub(crate) fn ramp_pixel_series_json(path: &str, x: usize, y: usize, integration: Option<usize>) -> Result<serde_json::Value> {
    let integration = integration.unwrap_or(0);
    let reference = ImageRef::parse(path);
    let (values, ramp, ngroups, nints) = if is_asdf_ref(&reference) {
        if integration != 0 {
            bail!("Integration {} out of range (nints=1)", integration);
        }
        let source = asdf_ramp_source(&reference.path, &reference.plane)?;
        let image = asdf_ramp_image(&reference.path, &source.key)?;
        let values = asdf_pixel_series(&image, x, y)?;
        (values, source.ramp, image.plane_count(), 1)
    } else {
        let cube = GLOBAL_CUBE_CACHE.get_or_open(path)?;
        let values = cube.extract_group_series(y, x, integration)?;
        let ramp = cube.ramp();
        let (ngroups, nints) = ramp
            .as_ref()
            .map_or((cube.geometry.naxis3, cube.geometry.naxis4), |r| (r.ngroups, r.nints));
        (values, ramp, ngroups, nints)
    };
    Ok(json!({
        KEY_X: x,
        KEY_Y: y,
        KEY_INTEGRATION: integration,
        KEY_NGROUPS: ngroups,
        KEY_NINTS: nints,
        KEY_VALUES: values,
        KEY_GROUP_TIMES: ramp.and_then(|r| r.group_times_s),
        RES_UNIT: RAMP_UNIT_DN,
    }))
}

fn is_previewable_column(info: &ColumnInfo) -> bool {
    info.repeat == 1 && RAMP_TABLE_TYPE_CODES.contains(&info.type_code) && !info.name.is_empty()
}

fn preview_values(column: &Column, info: &ColumnInfo, limit: usize) -> Vec<serde_json::Value> {
    match &column.values {
        ColumnValues::F64(values) => values
            .iter()
            .take(limit)
            .map(|v| if v.is_finite() { json!(v) } else { serde_json::Value::Null })
            .collect(),
        ColumnValues::I64(values) => {
            let null_value = info
                .tnull
                .and_then(|tnull| i64::try_from(tnull as i128 + info.tzero.unwrap_or(0.0) as i128).ok());
            values
                .iter()
                .take(limit)
                .map(|v| if Some(*v) == null_value { serde_json::Value::Null } else { json!(v) })
                .collect()
        }
    }
}

fn table_preview(path: &str, table: &TableHdu, max_rows: usize) -> Result<TablePreview> {
    let unique = |info: &ColumnInfo| table.columns.iter().filter(|c| c.name == info.name).count() == 1;
    let readable: Vec<&ColumnInfo> = table.columns.iter().filter(|c| is_previewable_column(c) && unique(c)).collect();
    let omitted_columns: Vec<String> = table
        .columns
        .iter()
        .filter(|c| !readable.iter().any(|r| r.index == c.index))
        .map(|c| c.name.clone())
        .collect();
    let names: Vec<&str> = readable.iter().map(|c| c.name.as_str()).collect();
    let columns = load_bintable(path, table.index)?.columns(&names)?;
    let shown_rows = table.n_rows.min(max_rows);
    let columns = columns
        .iter()
        .zip(readable.iter())
        .map(|(column, info)| TableColumnPreview {
            name: column.name.clone(),
            unit: column.unit.clone(),
            values: preview_values(column, info, shown_rows),
        })
        .collect();
    Ok(TablePreview {
        hdu: table.index,
        extname: table.extname.clone().unwrap_or_default(),
        n_rows: table.n_rows,
        shown_rows,
        columns,
        omitted_columns,
    })
}

pub(crate) fn ramp_tables_json(path: &str, max_rows: Option<usize>) -> Result<serde_json::Value> {
    let reference = ImageRef::parse(path);
    if is_asdf_ref(&reference) {
        return Ok(json!({ KEY_GROUP_TABLE: serde_json::Value::Null, KEY_INT_TIMES_TABLE: serde_json::Value::Null }));
    }
    let file = File::open(&reference.path).with_context(|| format!("Failed to open {}", reference.path))?;
    let tables = list_tables(&file)?;
    let max_rows = max_rows.unwrap_or(DEFAULT_TABLE_ROWS);
    let preview_of = |wanted: &str| -> Result<Option<TablePreview>> {
        tables
            .iter()
            .find(|(table, _)| is_named(table.extname.as_deref(), wanted))
            .map(|(table, _)| table_preview(&reference.path, table, max_rows))
            .transpose()
    };
    Ok(json!({
        KEY_GROUP_TABLE: preview_of(EXTNAME_GROUP)?,
        KEY_INT_TIMES_TABLE: preview_of(EXTNAME_INT_TIMES)?,
    }))
}

fn asdf_frame_header(frame_index: usize) -> HduHeader {
    let mut header = HduHeader::empty();
    header.set(HEADER_ABPROC, ABPROC_RAMP_FRAME.to_string());
    header.set(CARD_GROUP, frame_index.to_string());
    header.set(CARD_INTEGRATION, "0".to_string());
    header.set(HEADER_BUNIT, RAMP_UNIT_DN.to_string());
    header
}

pub(crate) fn ramp_frame_json(
    path: &str,
    frame_index: usize,
    output_path: &str,
    output_fits: Option<&str>,
) -> Result<serde_json::Value> {
    let reference = ImageRef::parse(path);
    if !is_asdf_ref(&reference) {
        return cube_frame_json(path, frame_index, output_path, output_fits);
    }
    let source = asdf_ramp_source(&reference.path, &reference.plane)?;
    let image = asdf_ramp_image(&reference.path, &source.key)?;
    let plane = image
        .plane(frame_index)
        .with_context(|| format!("Frame index {} out of range (depth={})", frame_index, image.plane_count()))?;
    save_auto_stf_preview_png(&plane, output_path)?;
    if let Some(fits_path) = output_fits {
        write_derived_fits(fits_path, &plane, Some(&asdf_frame_header(frame_index)))?;
    }
    Ok(json!({
        RES_FRAME_INDEX: frame_index,
        RES_OUTPUT_PATH: output_path,
        RES_FITS_PATH: output_fits,
    }))
}

#[tauri::command]
pub async fn ramp_info_cmd(path: String) -> std::result::Result<serde_json::Value, String> {
    blocking_cmd!(ramp_info_json(&path))
}

#[tauri::command]
pub async fn ramp_pixel_series_cmd(
    path: String,
    x: usize,
    y: usize,
    integration: Option<usize>,
) -> std::result::Result<serde_json::Value, String> {
    blocking_cmd!(ramp_pixel_series_json(&path, x, y, integration))
}

#[tauri::command]
pub async fn ramp_tables_cmd(path: String, max_rows: Option<usize>) -> std::result::Result<serde_json::Value, String> {
    blocking_cmd!(ramp_tables_json(&path, max_rows))
}

#[tauri::command]
pub async fn ramp_frame_cmd(
    path: String,
    frame_index: usize,
    output_path: String,
    output_fits: Option<String>,
) -> std::result::Result<serde_json::Value, String> {
    blocking_cmd!(ramp_frame_json(&path, frame_index, &output_path, output_fits.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cube::lazy::test_support::{f32_samples, image_hdu, write_bytes, write_u16_ramp_mef};
    use crate::infra::fits::reader::extract_image_mmap;
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, write_raw_hdus};
    use crate::infra::fits::table::test_support::bintable_cards;
    use crate::types::constants::RES_EXTNAME;

    static ASDF_CACHE_TEST_LOCK: Mutex<()> = Mutex::new(());

    const INTERLEAVED_COLOUR_TREE: &str = "meta:\n  telescope: JWST\nrgb: !core/ndarray-1.0.0\n  data: [[[1, 2, 3], [4, 5, 6]], [[7, 8, 9], [10, 11, 12]], [[13, 14, 15], [16, 17, 18]], [[19, 20, 21], [22, 23, 24]], [[25, 26, 27], [28, 29, 30]]]\n  datatype: float32\n";

    const PLANAR_CUBE_TREE: &str = "cube: !core/ndarray-1.0.0\n  data: [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]\n  datatype: float32\n";

    const ROMAN_RAMP_TREE: &str = "meta:\n  telescope: ROMAN\nroman:\n  meta:\n    exposure:\n      frame_time: 3.04\n      read_pattern: [[1], [2, 3], [4, 5, 6, 7]]\n      nresultants: 3\n      ma_table_name: MA_TABLE_1\n    instrument:\n      detector: WFI01\n  data: !core/ndarray-1.0.0\n    data: [[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]]]\n    datatype: float32\n  dq: !core/ndarray-1.0.0\n    data: [[[0, 1], [0, 0]], [[1, 1], [0, 0]], [[0, 0], [0, 1]]]\n    datatype: uint32\n";

    fn write_asdf(dir: &tempfile::TempDir, name: &str, tree_yaml: &str) -> String {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(tree_yaml.as_bytes());
        bytes.extend_from_slice(b"...\n");
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn ramp_value(i: usize, g: usize, y: usize, x: usize) -> u16 {
        (30000 + i * 1000 + g * 100 + y * 10 + x) as u16
    }

    fn write_uncal(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        let primary = [("NGROUPS", "3"), ("NINTS", "2"), ("TGROUP", "10.0"), ("DATAMODL", "'Level1bModel'"), ("DETECTOR", "'NRS1'")];
        write_u16_ramp_mef(&path, 4, 3, 3, 2, &primary, &[], ramp_value);
        path.to_str().unwrap().to_string()
    }

    fn named_table(
        extname: &str,
        columns: &[(&str, &str, Option<&str>)],
        rows: usize,
        extra: &[(&'static str, String)],
    ) -> Vec<(&'static str, String)> {
        let mut cards = bintable_cards(columns, rows, extra);
        cards.retain(|(k, _)| *k != "EXTNAME");
        cards.push(("EXTNAME", format!("'{extname:<8}'")));
        cards
    }

    fn group_rows(rows: &[(i32, i16, &str, f64)]) -> Vec<u8> {
        let mut data = Vec::new();
        for (integration, group, text, mjd) in rows {
            data.extend_from_slice(&integration.to_be_bytes());
            data.extend_from_slice(&group.to_be_bytes());
            let mut label = text.as_bytes().to_vec();
            label.resize(26, b' ');
            data.extend_from_slice(&label);
            data.extend_from_slice(&mjd.to_be_bytes());
        }
        data
    }

    fn write_owner_like_tables(dir: &tempfile::TempDir, name: &str) -> String {
        let group_columns = [
            ("integration_number", "J", None),
            ("group_number", "I", None),
            ("group_end_time", "26A", None),
            ("bary_end_time", "D", Some("d")),
        ];
        let group_data = group_rows(&[
            (1, 1, "2024-04-01T00:00:14.589", 60401.0001),
            (1, 2, "2024-04-01T00:00:29.178", 60401.0002),
            (1, 3, "2024-04-01T00:00:43.767", 60401.0003),
        ]);
        let int_columns = [("integration_number", "J", None), ("int_start_MJD_UTC", "D", Some("d")), ("int_mid_MJD_UTC", "D", None)];
        let mut int_data = Vec::new();
        for (n, start, mid) in [(1i32, 60401.0f64, 60401.0005f64)] {
            int_data.extend_from_slice(&n.to_be_bytes());
            int_data.extend_from_slice(&start.to_be_bytes());
            int_data.extend_from_slice(&mid.to_be_bytes());
        }
        let path = dir.path().join(name);
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                (named_table(EXTNAME_GROUP, &group_columns, 3, &[]), group_data),
                (named_table(EXTNAME_INT_TIMES, &int_columns, 1, &[]), int_data),
            ],
        );
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn ramp_info_cmd_reports_a_fits_ramp_and_falls_back_when_the_cube_path_refuses_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_uncal(&dir, "uncal.fits");
        let info = ramp_info_json(&path).unwrap();
        assert_eq!(info[KEY_SOURCE], SOURCE_FITS);
        assert_eq!(info[RES_RAMP]["ngroups"], 3);
        assert_eq!(info[RES_RAMP]["nints"], 2);
        assert_eq!(info[RES_RAMP]["detector"], "NRS1");
        assert_eq!(info[RES_FRAMES], 6);
        assert_eq!(info[RES_WIDTH], 4);
        assert_eq!(info[RES_HEIGHT], 3);
        assert!(info[KEY_ARRAY_KEY].is_null());
        let keys: Vec<&String> = info.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 6);
        GLOBAL_CUBE_CACHE.invalidate(&path);

        let refused = dir.path().join("polarised.fits");
        let data = f32_samples(3, 2, 10, |z, _, _| z as f32);
        write_bytes(&refused, &image_hdu("SIMPLE", &[3, 2, 5, 2], -32, &[("TELESCOP", "'JWST'")], data));
        let key = refused.to_str().unwrap();
        assert!(GLOBAL_CUBE_CACHE.get_or_open(key).is_err());
        let info = ramp_info_json(key).unwrap();
        assert_eq!(info[KEY_SOURCE], SOURCE_FITS);
        assert!(info[RES_RAMP].is_null(), "{}", info[RES_RAMP]);
        assert_eq!(info[RES_FRAMES], 10);
        assert_eq!((info[RES_WIDTH].as_u64(), info[RES_HEIGHT].as_u64()), (Some(3), Some(2)));

        assert!(ramp_info_json(dir.path().join("missing.fits").to_str().unwrap()).is_err());
    }

    #[test]
    fn ramp_pixel_series_cmd_returns_one_integration_with_group_times() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_uncal(&dir, "series.fits");
        let series = ramp_pixel_series_json(&path, 2, 1, Some(1)).unwrap();
        assert_eq!(series[KEY_VALUES], json!([31012.0, 31112.0, 31212.0]));
        assert_eq!(series[KEY_GROUP_TIMES], json!([0.0, 10.0, 20.0]));
        assert_eq!(series[KEY_X], 2);
        assert_eq!(series[KEY_Y], 1);
        assert_eq!(series[KEY_INTEGRATION], 1);
        assert_eq!(series[KEY_NGROUPS], 3);
        assert_eq!(series[KEY_NINTS], 2);
        assert_eq!(series[RES_UNIT], RAMP_UNIT_DN);
        let first = ramp_pixel_series_json(&path, 0, 0, None).unwrap();
        assert_eq!(first[KEY_VALUES], json!([30000.0, 30100.0, 30200.0]));
        assert_eq!(first[KEY_INTEGRATION], 0);
        let err = format!("{:#}", ramp_pixel_series_json(&path, 0, 0, Some(2)).unwrap_err());
        assert!(err.contains("out of range"), "{err}");
        assert!(ramp_pixel_series_json(&path, 4, 0, None).is_err());
        GLOBAL_CUBE_CACHE.invalidate(&path);
    }

    #[test]
    fn ramp_tables_cmd_reads_numeric_columns_of_group_and_int_times_and_lists_string_columns_as_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_owner_like_tables(&dir, "tables.fits");
        let tables = ramp_tables_json(&path, None).unwrap();
        let group = &tables[KEY_GROUP_TABLE];
        assert_eq!(group["hdu"], 1);
        assert_eq!(group[RES_EXTNAME], EXTNAME_GROUP);
        assert_eq!(group["n_rows"], 3);
        assert_eq!(group["shown_rows"], 3);
        let names: Vec<&str> = group["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["INTEGRATION_NUMBER", "GROUP_NUMBER", "BARY_END_TIME"]);
        assert_eq!(group["omitted_columns"], json!(["GROUP_END_TIME"]));
        assert_eq!(group["columns"][0]["values"], json!([1, 1, 1]));
        assert_eq!(group["columns"][1]["values"], json!([1, 2, 3]));
        assert_eq!(group["columns"][2]["values"], json!([60401.0001, 60401.0002, 60401.0003]));
        assert_eq!(group["columns"][2]["unit"], "d");
        assert!(group["columns"][0]["unit"].is_null());

        let int_times = &tables[KEY_INT_TIMES_TABLE];
        assert_eq!(int_times["hdu"], 2);
        assert_eq!(int_times[RES_EXTNAME], EXTNAME_INT_TIMES);
        assert_eq!(int_times["n_rows"], 1);
        let names: Vec<&str> = int_times["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["INTEGRATION_NUMBER", "INT_START_MJD_UTC", "INT_MID_MJD_UTC"]);
        assert_eq!(int_times["columns"][1]["values"], json!([60401.0]));
        assert_eq!(int_times["omitted_columns"], json!([]));

        let uncal = write_uncal(&dir, "no_tables.fits");
        let none = ramp_tables_json(&uncal, None).unwrap();
        assert!(none[KEY_GROUP_TABLE].is_null() && none[KEY_INT_TIMES_TABLE].is_null());
        let asdf = write_asdf(&dir, "roman.asdf", ROMAN_RAMP_TREE);
        let none = ramp_tables_json(&asdf, None).unwrap();
        assert!(none[KEY_GROUP_TABLE].is_null() && none[KEY_INT_TIMES_TABLE].is_null());
    }

    #[test]
    fn ramp_tables_cmd_truncates_to_max_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_owner_like_tables(&dir, "truncated.fits");
        let tables = ramp_tables_json(&path, Some(2)).unwrap();
        let group = &tables[KEY_GROUP_TABLE];
        assert_eq!(group["n_rows"], 3);
        assert_eq!(group["shown_rows"], 2);
        for column in group["columns"].as_array().unwrap() {
            assert_eq!(column["values"].as_array().unwrap().len(), 2, "{}", column["name"]);
        }
        assert_eq!(group["columns"][1]["values"], json!([1, 2]));
        let int_times = &tables[KEY_INT_TIMES_TABLE];
        assert_eq!(int_times["n_rows"], 1);
        assert_eq!(int_times["shown_rows"], 1);
        assert_eq!(DEFAULT_TABLE_ROWS, 200);
    }

    #[test]
    fn ramp_tables_cmd_reports_tnull_rows_as_null() {
        let dir = tempfile::tempdir().unwrap();
        let columns = [("integration_number", "J", None), ("group_number", "I", None)];
        let mut data = Vec::new();
        for (integration, group) in [(1i32, 1i16), (-999, 2), (3, -7)] {
            data.extend_from_slice(&integration.to_be_bytes());
            data.extend_from_slice(&group.to_be_bytes());
        }
        let extra = vec![("TNULL1", "-999".to_string()), ("TZERO1", "0".to_string()), ("TNULL2", "-7".to_string())];
        let path = dir.path().join("tnull.fits");
        write_raw_hdus(&path, &[(empty_primary_cards(), Vec::new()), (named_table(EXTNAME_GROUP, &columns, 3, &extra), data)]);
        let tables = ramp_tables_json(path.to_str().unwrap(), None).unwrap();
        let group = &tables[KEY_GROUP_TABLE];
        assert_eq!(group["columns"][0]["values"], json!([1, null, 3]));
        assert_eq!(group["columns"][1]["values"], json!([1, 2, null]));
    }

    #[test]
    fn ramp_info_cmd_reports_an_asdf_resultant_ramp() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, "roman_info.asdf", ROMAN_RAMP_TREE);
        let info = ramp_info_json(&path).unwrap();
        assert_eq!(info[KEY_SOURCE], SOURCE_ASDF);
        assert_eq!(info[KEY_ARRAY_KEY], "roman.data");
        assert_eq!(info[RES_FRAMES], 3);
        assert_eq!(info[RES_WIDTH], 2);
        assert_eq!(info[RES_HEIGHT], 2);
        let ramp = &info[RES_RAMP];
        assert_eq!(ramp["kind"], "roman_resultants");
        assert_eq!(ramp["ngroups"], 3);
        assert_eq!(ramp["nints"], 1);
        assert_eq!(ramp["tframe_s"], 3.04);
        assert!(ramp["tgroup_s"].is_null());
        assert_eq!(ramp["tgroup_source"], "none");
        let times = ramp["group_times_s"].as_array().unwrap();
        for (t, e) in times.iter().zip([3.04, 7.6, 16.72]) {
            assert!((t.as_f64().unwrap() - e).abs() < 1e-9, "{:?}", times);
        }
        assert_eq!(ramp["readpatt"], "MA_TABLE_1");
        assert_eq!(ramp["detector"], "WFI01");
        assert_eq!(ramp["instrument"], "WFI");
        assert_eq!(ramp["frame_width"], 2);
        assert_eq!(ramp["frame_height"], 2);

        let explicit = ramp_info_json(&format!("{}#array=roman.dq", path)).unwrap();
        assert_eq!(explicit[KEY_ARRAY_KEY], "roman.dq");
        assert_eq!(explicit[RES_RAMP]["ngroups"], 3);

        let bare = write_asdf(&dir, "bare.asdf", "roman:\n  data: !core/ndarray-1.0.0\n    data: [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]\n    datatype: float32\n");
        let info = ramp_info_json(&bare).unwrap();
        assert_eq!(info[RES_RAMP]["ngroups"], 2);
        assert!(info[RES_RAMP]["group_times_s"].is_null());
        assert!(info[RES_RAMP]["tframe_s"].is_null());
        assert!(info[RES_RAMP]["readpatt"].is_null());

        let flat = write_asdf(&dir, "flat.asdf", "data: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\n");
        let info = ramp_info_json(&flat).unwrap();
        assert!(info[RES_RAMP].is_null());
        assert_eq!(info[RES_FRAMES], 1);
        assert_eq!(info[KEY_ARRAY_KEY], "data");
    }

    #[test]
    fn an_interleaved_colour_array_is_not_a_roman_ramp() {
        let _serial = ASDF_CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, "rgb.asdf", INTERLEAVED_COLOUR_TREE);
        let explicit = ramp_info_json(&format!("{}#array=rgb", path)).unwrap();
        assert!(explicit[RES_RAMP].is_null(), "{}", explicit[RES_RAMP]);
        assert_eq!(explicit[RES_FRAMES], 3);
        assert_eq!((explicit[RES_WIDTH].as_u64(), explicit[RES_HEIGHT].as_u64()), (Some(2), Some(5)));
        assert_eq!(explicit[KEY_ARRAY_KEY], "rgb");
        let auto = ramp_info_json(&path).unwrap();
        assert!(auto[RES_RAMP].is_null(), "{}", auto[RES_RAMP]);
        assert_eq!(auto[RES_FRAMES], 3);
        assert_eq!(auto[KEY_ARRAY_KEY], "rgb");
        let series = ramp_pixel_series_json(&format!("{}#array=rgb", path), 1, 4, None).unwrap();
        assert_eq!(series[KEY_NGROUPS], 3);
        assert_eq!(series[KEY_VALUES], json!([28.0, 29.0, 30.0]));
        assert!(series[KEY_GROUP_TIMES].is_null());

        let mixed = write_asdf(&dir, "mixed.asdf", &format!("{}{}", INTERLEAVED_COLOUR_TREE, PLANAR_CUBE_TREE));
        let info = ramp_info_json(&mixed).unwrap();
        assert_eq!(info[KEY_ARRAY_KEY], "cube");
        assert_eq!(info[RES_RAMP]["ngroups"], 2);
        assert_eq!(info[RES_FRAMES], 2);
    }

    #[test]
    fn a_failed_reload_empties_the_single_entry_cache_and_release_forgets_the_file() {
        let _serial = ASDF_CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, "release.asdf", ROMAN_RAMP_TREE);
        let image = asdf_ramp_image(&path, "roman.data").unwrap();
        assert_eq!(Arc::strong_count(&image), 2, "the cache holds the second reference");
        assert!(asdf_ramp_image(&path, "roman.missing").is_err());
        assert_eq!(Arc::strong_count(&image), 1, "a miss drops the old entry before loading the new array");
        let image = asdf_ramp_image(&path, "roman.data").unwrap();
        assert_eq!(Arc::strong_count(&image), 2);
        release_asdf_ramp(&format!("{}#array=roman.data", path));
        assert_eq!(Arc::strong_count(&image), 1, "release forgets the entry of that file");
        let other = write_asdf(&dir, "other.asdf", ROMAN_RAMP_TREE);
        let kept = asdf_ramp_image(&other, "roman.data").unwrap();
        release_asdf_ramp(&path);
        assert_eq!(Arc::strong_count(&kept), 2, "releasing another file keeps the entry");
        release_asdf_ramp(&other);
        assert_eq!(Arc::strong_count(&kept), 1);
    }

    #[tokio::test]
    async fn release_cube_cmd_empties_the_asdf_ramp_slot() {
        let _serial = ASDF_CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, "released_by_cmd.asdf", ROMAN_RAMP_TREE);
        let image = asdf_ramp_image(&path, "roman.data").unwrap();
        assert_eq!(Arc::strong_count(&image), 2, "precondition: the slot holds the array");
        crate::cmd::io::release_cube_cmd(format!("{}#array=roman.data", path)).await.unwrap();
        assert_eq!(Arc::strong_count(&image), 1, "release_cube_cmd forgets the cached ASDF ramp");

        let image = asdf_ramp_image(&path, "roman.data").unwrap();
        assert_eq!(Arc::strong_count(&image), 2);
        let forward_slashes = path.replace('\\', "/");
        if cfg!(windows) {
            assert_ne!(forward_slashes, path, "the fixture path must differ in spelling for this case to mean anything");
        }
        crate::cmd::io::release_cube_cmd(forward_slashes).await.unwrap();
        assert_eq!(Arc::strong_count(&image), 1, "a forward-slash spelling of the same file still empties the slot");
    }

    #[test]
    fn ramp_tables_cmd_reads_unsigned_sixty_four_bit_columns_without_overflowing_the_null_value() {
        let dir = tempfile::tempdir().unwrap();
        let columns = [("integration_number", "1K", None), ("group_number", "1K", None)];
        let mut data = Vec::new();
        for (first, second) in [(-1i64, i64::MIN), (-2, -1), (-3, -3)] {
            data.extend_from_slice(&first.to_be_bytes());
            data.extend_from_slice(&second.to_be_bytes());
        }
        let two_63 = "9223372036854775808".to_string();
        let extra = vec![
            ("TZERO1", two_63.clone()),
            ("TNULL1", "7".to_string()),
            ("TZERO2", two_63),
            ("TNULL2", i64::MIN.to_string()),
        ];
        let path = dir.path().join("u64.fits");
        write_raw_hdus(&path, &[(empty_primary_cards(), Vec::new()), (named_table(EXTNAME_GROUP, &columns, 3, &extra), data)]);
        let tables = ramp_tables_json(path.to_str().unwrap(), None).unwrap();
        let group = &tables[KEY_GROUP_TABLE];
        assert_eq!(group["columns"][0]["values"], json!([i64::MAX, i64::MAX - 1, i64::MAX - 2]));
        assert_eq!(group["columns"][1]["values"], json!([null, i64::MAX, i64::MAX - 2]));
    }

    #[test]
    fn the_asdf_ramp_cache_reuses_the_array_for_the_same_path_key_and_mtime() {
        let _serial = ASDF_CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, "cached.asdf", ROMAN_RAMP_TREE);
        let before = *ASDF_RAMP_CACHE_EVENTS.lock().unwrap();
        let first = asdf_ramp_image(&path, "roman.data").unwrap();
        let second = asdf_ramp_image(&path, "roman.data").unwrap();
        assert!(Arc::ptr_eq(&first, &second), "the second call must reuse the loaded array");
        let other = asdf_ramp_image(&path, "roman.dq").unwrap();
        assert!(!Arc::ptr_eq(&first, &other));
        let back = asdf_ramp_image(&path, "roman.data").unwrap();
        assert!(!Arc::ptr_eq(&first, &back), "a single-entry cache reloads after another key displaced it");
        let after = *ASDF_RAMP_CACHE_EVENTS.lock().unwrap();
        let (hits, misses) = (after.0 - before.0, after.1 - before.1);
        println!("asdf ramp cache: hits={} misses={}", hits, misses);
        assert!(hits >= 1 && misses >= 3, "hits={} misses={}", hits, misses);

        let series = ramp_pixel_series_json(&path, 1, 0, None).unwrap();
        assert_eq!(series[KEY_VALUES], json!([2.0, 6.0, 10.0]));
        assert_eq!(series[KEY_NGROUPS], 3);
        assert_eq!(series[KEY_NINTS], 1);
        let times = series[KEY_GROUP_TIMES].as_array().unwrap();
        assert!((times[1].as_f64().unwrap() - 7.6).abs() < 1e-9);
        assert!(ramp_pixel_series_json(&path, 1, 0, Some(1)).is_err());
        assert!(ramp_pixel_series_json(&path, 2, 0, None).is_err());

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, std::fs::read(&path).unwrap()).unwrap();
        let rewritten = asdf_ramp_image(&path, "roman.data").unwrap();
        assert!(!Arc::ptr_eq(&back, &rewritten), "a newer mtime must reload the array");
    }

    #[test]
    fn ramp_frame_cmd_steps_an_asdf_resultant_and_writes_its_fits() {
        let _serial = ASDF_CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = write_asdf(&dir, "frames.asdf", ROMAN_RAMP_TREE);
        let png = dir.path().join("resultant_2.png");
        let fits = dir.path().join("resultant_2.fits");
        let result = ramp_frame_json(&path, 1, png.to_str().unwrap(), Some(fits.to_str().unwrap())).unwrap();
        assert_eq!(result[RES_FRAME_INDEX], 1);
        assert_eq!(result[RES_OUTPUT_PATH], png.to_str().unwrap());
        assert_eq!(result[RES_FITS_PATH], fits.to_str().unwrap());
        assert!(png.exists());
        let reopened = extract_image_mmap(&File::open(&fits).unwrap()).unwrap();
        assert_eq!(reopened.image.as_slice().unwrap(), &[5.0, 6.0, 7.0, 8.0]);
        assert_eq!(reopened.header.get(HEADER_ABPROC), Some(ABPROC_RAMP_FRAME));
        assert_eq!(reopened.header.get_i64(CARD_GROUP), Some(1));
        assert_eq!(reopened.header.get_i64(CARD_INTEGRATION), Some(0));
        assert_eq!(reopened.header.get(HEADER_BUNIT), Some(RAMP_UNIT_DN));
        let err = format!("{:#}", ramp_frame_json(&path, 3, png.to_str().unwrap(), None).unwrap_err());
        assert!(err.contains("out of range"), "{err}");

        let uncal = write_uncal(&dir, "frame_uncal.fits");
        let png = dir.path().join("group.png");
        let fits = dir.path().join("group.fits");
        let result = ramp_frame_json(&uncal, 4, png.to_str().unwrap(), Some(fits.to_str().unwrap())).unwrap();
        assert_eq!(result[RES_FRAME_INDEX], 4);
        let reopened = extract_image_mmap(&File::open(&fits).unwrap()).unwrap();
        assert_eq!(reopened.image[[1, 2]], 31112.0);
        assert_eq!(reopened.header.get_i64(CARD_GROUP), Some(1));
        assert_eq!(reopened.header.get_i64(CARD_INTEGRATION), Some(1));
        GLOBAL_CUBE_CACHE.invalidate(&uncal);
    }
}
