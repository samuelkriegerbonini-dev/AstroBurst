use serde_json::json;

use super::JwstSearchResults;

#[test]
fn search_item_maps_camel_case_and_keeps_extra_columns() {
    let results: JwstSearchResults = serde_json::from_value(json!({
        "results": [{
            "ArchiveFileID": 42,
            "fileSetName": "jw02739-o001_t001_nircam_clear-f444w",
            "targname": "NGC 3324",
            "targ_ra": 159.2,
            "targ_dec": -58.6,
            "instrume": "NIRCAM",
            "publicReleaseDate": "2022-07-12",
            "bstrtime": 59000.5
        }],
        "totalResults": 1
    }))
    .unwrap();
    let item = &results.results[0];
    assert_eq!(item.archive_file_id, Some(42));
    assert_eq!(item.instrume.as_deref(), Some("NIRCAM"));
    assert_eq!(item.public_release_date.as_deref(), Some("2022-07-12"));
    assert_eq!(item.extra["bstrtime"], json!(59000.5));
}
