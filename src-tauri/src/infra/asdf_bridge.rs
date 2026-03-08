use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result};

use crate::infra::asdf::converter::{AsdfImage, is_asdf_file};
use crate::infra::fits::reader::{MmapImageResult, HduInfo};
use crate::types::HduHeader;

pub fn extract_image_from_asdf(path: &Path) -> Result<MmapImageResult> {
    let asdf_img = AsdfImage::load(path)
        .map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;

    let arr = asdf_img.to_array2();

    let mut cards = Vec::new();
    let mut index = HashMap::new();

    index.insert("NAXIS".into(), "2".into());
    index.insert("NAXIS1".into(), asdf_img.width.to_string());
    index.insert("NAXIS2".into(), asdf_img.height.to_string());
    index.insert("BITPIX".into(), "-32".into());
    cards.push(("NAXIS".into(), "2".into()));
    cards.push(("NAXIS1".into(), asdf_img.width.to_string()));
    cards.push(("NAXIS2".into(), asdf_img.height.to_string()));
    cards.push(("BITPIX".into(), "-32".into()));

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
        for (k, v) in &wcs_entries {
            cards.push((k.to_string(), v.clone()));
            index.insert(k.to_string(), v.clone());
        }
    }

    for (k, v) in &asdf_img.metadata {
        let fits_key = k
            .replace('.', "_")
            .chars()
            .take(68)
            .collect::<String>()
            .to_uppercase();
        if !index.contains_key(&fits_key) {
            cards.push((fits_key.clone(), v.clone()));
            index.insert(fits_key, v.clone());
        }
    }

    cards.push(("ASDF_SRC".into(), "true".into()));
    index.insert("ASDF_SRC".into(), "true".into());

    let header = HduHeader { cards, index };

    let info = HduInfo {
        index: 0,
        extname: Some("SCI".into()),
        extver: Some(1),
        naxis: 2,
        naxis1: asdf_img.width as i64,
        naxis2: asdf_img.height as i64,
        naxis3: 0,
        bitpix: -32,
        has_data: true,
        header_start: 0,
        data_start: 0,
    };

    Ok(MmapImageResult {
        header,
        image: arr,
        is_mef: false,
        selected_extension: Some("SCI".into()),
        extension_count: 1,
        extensions: vec![info],
    })
}

pub fn is_asdf_path(path: &Path) -> bool {
    is_asdf_file(path)
}
