use super::MultiMissionApi;
use crate::infra::mast::client::tests::{serve_once, test_client};

#[tokio::test]
async fn get_image_sends_uri_and_size() {
    let (base, server) = serve_once("200 OK", "image/png", vec![0x89, 0x50]).await;
    let api = MultiMissionApi::new(test_client(&base));
    let bytes = api
        .get_image("mast:JWST/product/x_thumb.jpg", Some(120))
        .await
        .unwrap();
    assert_eq!(bytes, vec![0x89, 0x50]);
    let request = server.await.unwrap();
    assert_eq!(
        request.target,
        "/search/multi_mission/api/v0.1/get_image?uri=mast%3AJWST%2Fproduct%2Fx_thumb.jpg&output_size=120"
    );
}
