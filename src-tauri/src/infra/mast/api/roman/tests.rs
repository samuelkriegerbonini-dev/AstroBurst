use serde_json::json;

use super::{RomanDatasetProducts, RomanSearchResults};

#[test]
fn roman_product_flattens_base_product_fields() {
    let products: RomanDatasetProducts = serde_json::from_value(json!({
        "products": [{
            "product_key": "r0000101001001001001_0001_wfi01_cal.asdf",
            "access": "PUBLIC",
            "instrument_name": "WFI",
            "filename": "r0000101001001001001_0001_wfi01_cal.asdf",
            "uri": "mast:ROMAN/product/r0000101001001001001_0001_wfi01_cal.asdf",
            "authz_primary_identifier": "r00001",
            "authz_secondary_identifier": "ROMAN",
            "file_suffix": "_cal",
            "category": "L2",
            "size": 2048,
            "type": "science",
            "calibration_software_version": "0.16.0",
            "crds_context_used": "roman_0042.pmap",
            "crds_software_version": "12.0.0",
            "edp_url": "https://example.org/edp"
        }]
    }))
    .unwrap();
    let entry = &products.products[0];
    assert_eq!(entry.product.instrument_name.as_deref(), Some("WFI"));
    assert_eq!(entry.product.size, 2048);
    assert_eq!(entry.crds_context_used, "roman_0042.pmap");
}

#[test]
fn roman_archive_file_id_is_textual() {
    let results: RomanSearchResults = serde_json::from_value(json!({
        "results": [{"ArchiveFileID": "abc-1", "ra": 10.5, "pass": 2}]
    }))
    .unwrap();
    let item = &results.results[0];
    assert_eq!(item.archive_file_id.as_deref(), Some("abc-1"));
    assert_eq!(item.extra["pass"], json!(2));
}
