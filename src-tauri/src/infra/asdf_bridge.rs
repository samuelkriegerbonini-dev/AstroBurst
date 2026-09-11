use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::infra::asdf::converter::{is_asdf_file, AsdfImage};
use crate::infra::fits::reader::{HduInfo, MmapImageResult};
use crate::types::HduHeader;

pub fn extract_image_from_asdf(path: &Path) -> Result<MmapImageResult> {
    let asdf_img = AsdfImage::load(path)
        .map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;

    let has_image = asdf_img.has_image();
    if !has_image {
        anyhow::bail!("ASDF load failed: Missing field: data array");
    }
    let (width, height) = (asdf_img.width, asdf_img.height);
    let naxis_str = "2";

    let mut cards = Vec::new();
    let mut index = HashMap::new();

    push_card(&mut cards, &mut index, "NAXIS", naxis_str.into());
    push_card(&mut cards, &mut index, "NAXIS1", width.to_string());
    push_card(&mut cards, &mut index, "NAXIS2", height.to_string());
    push_card(&mut cards, &mut index, "BITPIX", "-32".into());

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

    push_card(&mut cards, &mut index, "ASDF_SRC", "true".into());

    let header = HduHeader { cards, index };

    let info = HduInfo {
        index: 0,
        extname: Some("SCI".into()),
        extver: Some(1),
        naxis: if has_image { 2 } else { 0 },
        naxis1: width as i64,
        naxis2: height as i64,
        naxis3: 0,
        bitpix: -32,
        has_data: has_image,
        header_start: 0,
        data_start: 0,
    };

    Ok(MmapImageResult {
        header,
        image: asdf_img.into_array2(),
        is_mef: false,
        selected_extension: Some("SCI".into()),
        extension_count: 1,
        extensions: vec![info],
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
