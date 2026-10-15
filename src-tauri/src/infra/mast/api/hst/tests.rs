use serde_json::json;

use super::{ClassicSearchParams, HstApi, HstSearchResults};
use crate::infra::mast::client::tests::{serve_once, test_client};

fn sample_params() -> ClassicSearchParams {
    let mut params = ClassicSearchParams {
        target: Some("M101".to_owned()),
        radius: Some(3.0),
        image: Some(vec!["ACS".to_owned(), "WFC3".to_owned()]),
        max_records: Some(10),
        ..ClassicSearchParams::default()
    };
    params.extra.insert("SR".to_owned(), json!(0.05));
    params
}

#[test]
fn classic_params_flatten_into_query_pairs() {
    let pairs = sample_params().to_query_pairs();
    let expected = [
        ("SR", "0.05"),
        ("image[]", "ACS"),
        ("image[]", "WFC3"),
        ("max_records", "10"),
        ("radius", "3.0"),
        ("target", "M101"),
    ];
    let mut sorted = pairs.clone();
    sorted.sort();
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    assert_eq!(sorted, expected);
}

#[test]
fn search_item_keeps_unknown_columns() {
    let results: HstSearchResults = serde_json::from_value(json!({
        "results": [{"sci_data_set_name": "J8D701010", "sci_ra": 210.8, "sci_hapnum": 3}]
    }))
    .unwrap();
    let item = &results.results[0];
    assert_eq!(item.sci_data_set_name.as_deref(), Some("J8D701010"));
    assert_eq!(item.sci_ra, Some(210.8));
    assert_eq!(item.extra["sci_hapnum"], json!(3));
}

#[tokio::test]
async fn get_classic_search_encodes_params_in_query() {
    let (base, server) = serve_once("200 OK", "application/json", b"[]".to_vec()).await;
    let api = HstApi::new(test_client(&base));
    let params = ClassicSearchParams {
        target: Some("M 101".to_owned()),
        image: Some(vec!["ACS".to_owned()]),
        ..ClassicSearchParams::default()
    };
    let value = api.get_classic_search(&params).await.unwrap();
    assert_eq!(value, json!([]));
    let request = server.await.unwrap();
    assert!(request
        .target
        .starts_with("/search/hst/api/v0.1/get_classic_search?"));
    assert!(request.target.contains("target=M+101"));
    assert!(request.target.contains("image%5B%5D=ACS"));
}
