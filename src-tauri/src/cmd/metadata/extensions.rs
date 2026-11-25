use std::fs::File;
use std::path::Path;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, source_path};
use crate::core::imaging::layer_roles::{classify_layer, LayerGuess, LayerRole};
use crate::core::imaging::pixel_probe::data_unit;
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::fits::band_planes::{list_band_planes, HduBandPlanes};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::{extract_header_by_index, HduInfo};
use crate::infra::image_source::{is_dq_name, is_err_name, list_planes, plane_ref};
use crate::types::constants::{
    RES_BITPIX, RES_EXTENSIONS, RES_EXTNAME, RES_EXTVER, RES_HAS_DATA, RES_INDEX, RES_IS_DQ, RES_IS_ERR, RES_KIND,
    RES_NAXIS, RES_NAXIS1, RES_NAXIS2, RES_NAXIS3, RES_NAXIS4, RES_REF, PLANE_KIND_ARRAY, PLANE_KIND_HDU,
};
use crate::types::header::HduHeader;

const RES_ROLE: &str = "role";
const RES_ROLE_BASIS: &str = "role_basis";
const RES_ROLE_ASSUMED: &str = "role_assumed";
const RES_BUNIT: &str = "bunit";
const RES_BANDS: &str = "bands";
const NO_IMAGE_DATA: &str = "no image data";

fn asdf_layers(extensions: &[HduInfo], file_name: &str) -> Vec<(LayerGuess, Option<String>)> {
    extensions
        .iter()
        .map(|ext| {
            let mut named = HduHeader::empty();
            named.set("EXTNAME", ext.extname.clone().unwrap_or_default());
            (classify_layer(&named, file_name), None)
        })
        .collect()
}

fn listed_band_planes(source: &str) -> Vec<HduBandPlanes> {
    list_band_planes(source).unwrap_or_else(|error| {
        log::warn!("the band planes of {source} could not be listed: {error:#}");
        Vec::new()
    })
}

fn fits_layers(source: &str, extensions: &[HduInfo], file_name: &str) -> anyhow::Result<Vec<(LayerGuess, Option<String>)>> {
    let (readable, _tmp) = resolve_single_image(source)?;
    let file = File::open(&readable)?;
    extensions
        .iter()
        .map(|ext| {
            let header = extract_header_by_index(&file, ext.index)?;
            let layer = if ext.has_data {
                classify_layer(&header, file_name)
            } else {
                LayerGuess { role: LayerRole::Other, basis: NO_IMAGE_DATA.to_string(), assumed: false }
            };
            Ok((layer, data_unit(&header)))
        })
        .collect()
}

pub(crate) fn extensions_json(path: &str) -> anyhow::Result<serde_json::Value> {
    let source = source_path(path);
    let (extensions, is_asdf) = list_planes(&source)?;
    let file_name = Path::new(&source).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let (layers, band_planes) = if is_asdf {
        (asdf_layers(&extensions, file_name), Vec::new())
    } else {
        (fits_layers(&source, &extensions, file_name)?, listed_band_planes(&source))
    };
    let ext_json: Vec<serde_json::Value> = extensions
        .iter()
        .zip(&layers)
        .map(|(ext, (layer, bunit))| {
            let name = ext.extname.as_deref().unwrap_or("");
            let bands = band_planes.iter().find(|listed| listed.hdu == ext.index).map_or(&[][..], |listed| &listed.bands[..]);
            json!({
                RES_INDEX: ext.index,
                RES_EXTNAME: ext.extname,
                RES_EXTVER: ext.extver,
                RES_NAXIS: ext.naxis,
                RES_NAXIS1: ext.naxis1,
                RES_NAXIS2: ext.naxis2,
                RES_NAXIS3: ext.naxis3,
                RES_NAXIS4: ext.naxis4,
                RES_BITPIX: ext.bitpix,
                RES_HAS_DATA: ext.has_data,
                RES_REF: plane_ref(&source, ext, is_asdf).cache_key(),
                RES_KIND: if is_asdf { PLANE_KIND_ARRAY } else { PLANE_KIND_HDU },
                RES_IS_DQ: is_dq_name(name),
                RES_IS_ERR: is_err_name(name),
                RES_ROLE: layer.role,
                RES_ROLE_BASIS: layer.basis,
                RES_ROLE_ASSUMED: layer.assumed,
                RES_BUNIT: bunit,
                RES_BANDS: bands,
            })
        })
        .collect();
    Ok(json!({ RES_EXTENSIONS: ext_json }))
}

#[tauri::command]
pub async fn get_fits_extensions(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!(extensions_json(&path))
}

#[tauri::command]
pub async fn get_header_by_hdu(path: String, hdu_index: usize) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let (fits_path, _tmp) = resolve_single_image(&source_path(&path))?;
        if is_asdf_file(&fits_path) {
            anyhow::bail!("ASDF arrays have no HDU index");
        }
        let file = File::open(&fits_path)?;
        let header = crate::infra::fits::reader::extract_header_by_index(&file, hdu_index)?;
        Ok(serde_json::to_value(&header)?)
    })
}
