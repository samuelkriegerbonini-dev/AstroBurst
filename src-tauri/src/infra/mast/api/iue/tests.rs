use serde_json::json;

use super::IueSearchResults;

#[test]
fn iue_item_accepts_textual_values_where_spec_declares_numbers() {
    let results: IueSearchResults = serde_json::from_value(json!({
        "results": [{
            "iue_data_id": "SWP01234",
            "iue_ra2000": 83.82,
            "iue_dec2000": -5.39,
            "iue_pi_name": "SMITH",
            "iue_camera_name": "SWP"
        }]
    }))
    .unwrap();
    let item = &results.results[0];
    assert_eq!(item.iue_data_id.as_deref(), Some("SWP01234"));
    assert_eq!(item.iue_pi_name, Some(json!("SMITH")));
}
