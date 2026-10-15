use serde_json::json;

use super::ClassySearchResults;

#[test]
fn classy_item_maps_mixed_case_columns() {
    let results: ClassySearchResults = serde_json::from_value(json!({
        "results": [{"Target": "J0021+0052", "RA_deg": 5.38, "DEC_deg": 0.87, "log_SFR": 1.1, "O32_log_U": -2.5}]
    }))
    .unwrap();
    let item = &results.results[0];
    assert_eq!(item.target.as_deref(), Some("J0021+0052"));
    assert_eq!(item.ra_deg, Some(5.38));
    assert_eq!(item.log_sfr, Some(1.1));
    assert_eq!(item.extra["O32_log_U"], json!(-2.5));
}
