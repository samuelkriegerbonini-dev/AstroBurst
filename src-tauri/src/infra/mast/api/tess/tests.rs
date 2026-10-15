use serde_json::json;

use super::TessSearchResults;

#[test]
fn tess_item_maps_camel_case_columns() {
    let results: TessSearchResults = serde_json::from_value(json!({
        "results": [{
            "fileSetName": "tess2018206045859-s0001-0000000025155310-0120-s",
            "fileSize": 1990080,
            "sector": 1,
            "ticid": 25155310,
            "tessmag": 9.9,
            "pmra": 1.2
        }]
    }))
    .unwrap();
    let item = &results.results[0];
    assert_eq!(item.file_size, Some(1990080));
    assert_eq!(item.ticid, Some(25155310));
    assert_eq!(item.extra["pmra"], json!(1.2));
}
