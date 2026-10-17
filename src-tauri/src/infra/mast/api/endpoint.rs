use serde::de::DeserializeOwned;
use serde::Serialize;

use super::mission::Mission;
use crate::infra::mast::client::{ChunkSink, ImageDownload, ImageOp, MastClient, QueryPairs};
use crate::infra::mast::error::MastResult;
use crate::infra::mast::models::{
    AuthCheck, DatasetIds, DatasetProducts, ImageListResult, SearchParams, SearchResults,
};

pub(crate) fn present<'a, const N: usize>(
    pairs: [(&'a str, Option<&'a str>); N],
) -> Vec<(&'a str, &'a str)> {
    pairs
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect()
}

#[derive(Debug, Clone)]
pub struct MissionEndpoint {
    client: MastClient,
    mission: Mission,
}

impl MissionEndpoint {
    pub fn new(client: MastClient, mission: Mission) -> Self {
        Self { client, mission }
    }

    pub fn mission(&self) -> Mission {
        self.mission
    }

    pub fn client(&self) -> &MastClient {
        &self.client
    }

    pub fn path(&self, operation: &str) -> String {
        self.mission.operation_path(operation)
    }

    pub async fn search<T>(&self, params: &SearchParams) -> MastResult<SearchResults<T>>
    where
        T: DeserializeOwned,
    {
        self.client.post_json(&self.path("search"), params).await
    }

    pub async fn post_operation<B, T>(&self, operation: &str, payload: &B) -> MastResult<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned + Default,
    {
        self.client.post_json(&self.path(operation), payload).await
    }

    pub async fn get_operation<T>(&self, operation: &str, query: &QueryPairs<'_>) -> MastResult<T>
    where
        T: DeserializeOwned + Default,
    {
        self.client.get_json(&self.path(operation), query).await
    }

    pub async fn auth_check(&self) -> MastResult<AuthCheck> {
        self.client.get_json(&self.path("auth_check"), &[]).await
    }

    pub async fn list_products<P>(
        &self,
        dataset_ids: Option<&str>,
    ) -> MastResult<DatasetProducts<P>>
    where
        P: DeserializeOwned,
    {
        let query = present([("dataset_ids", dataset_ids)]);
        self.client
            .get_json(&self.path("list_products"), &query)
            .await
    }

    pub async fn post_list_products<P>(
        &self,
        dataset_ids: &DatasetIds,
    ) -> MastResult<DatasetProducts<P>>
    where
        P: DeserializeOwned,
    {
        self.client
            .post_json_present(&self.path("post_list_products"), dataset_ids)
            .await
    }

    pub async fn get_image_list(
        &self,
        fileset_name: &str,
        category: Option<&str>,
    ) -> MastResult<Vec<ImageListResult>> {
        let query = present([("fileset_name", Some(fileset_name)), ("category", category)]);
        self.client
            .get_json_present(&self.path("get_image_list"), &query)
            .await
    }

    pub async fn get_image_preview(
        &self,
        filename: Option<&str>,
        fileset_name: Option<&str>,
    ) -> MastResult<Vec<u8>> {
        self.get_image(ImageOp::Preview, filename, fileset_name)
            .await
    }

    pub async fn get_image_thumbnail(
        &self,
        filename: Option<&str>,
        fileset_name: Option<&str>,
    ) -> MastResult<Vec<u8>> {
        self.get_image(ImageOp::Thumbnail, filename, fileset_name)
            .await
    }

    pub async fn download_image(
        &self,
        op: ImageOp,
        filename: Option<&str>,
        fileset_name: Option<&str>,
        cap: u64,
        sink: &mut ChunkSink<'_>,
    ) -> MastResult<ImageDownload> {
        let query = present([("filename", filename), ("fileset_name", fileset_name)]);
        self.client
            .download_image(&self.path(op.operation()), &query, cap, sink)
            .await
    }

    pub async fn retrieve_product(&self, product_name: &str) -> MastResult<Vec<u8>> {
        let query = [("product_name", product_name)];
        self.client
            .get_bytes(&self.path("retrieve_product"), &query)
            .await
    }

    async fn get_image(
        &self,
        op: ImageOp,
        filename: Option<&str>,
        fileset_name: Option<&str>,
    ) -> MastResult<Vec<u8>> {
        let mut bytes = Vec::new();
        self.download_image(op, filename, fileset_name, op.cap(), &mut |chunk: &[u8], _total: Option<u64>| {
            bytes.extend_from_slice(chunk);
            Ok(())
        })
        .await?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::mast::client::tests::{serve_once, test_client, JPEG_HEAD};
    use crate::infra::mast::models::DatasetProduct;
    use serde_json::{json, Value};

    #[test]
    fn present_drops_missing_values() {
        assert_eq!(
            present([("a", Some("1")), ("b", None), ("c", Some("3"))]),
            vec![("a", "1"), ("c", "3")]
        );
    }

    #[tokio::test]
    async fn search_posts_params_to_mission_path() {
        let body = json!({"results": [{"x": 1}], "totalResults": 1}).to_string();
        let (base, server) = serve_once("200 OK", "application/json", body.into_bytes()).await;
        let endpoint = MissionEndpoint::new(test_client(&base), Mission::Jwst);
        let params = SearchParams::new().target("M16").page(0, 5);
        let results: SearchResults<Value> = endpoint.search(&params).await.unwrap();
        assert_eq!(results.total_results, 1);
        let request = server.await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/search/jwst/api/v0.1/search");
        assert!(request.headers.contains("content-type: application/json"));
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({"target": ["M16"], "offset": 0, "limit": 5})
        );
    }

    #[tokio::test]
    async fn post_list_products_sends_dataset_ids() {
        let (base, server) = serve_once(
            "200 OK",
            "application/json",
            br#"{"products": []}"#.to_vec(),
        )
        .await;
        let endpoint = MissionEndpoint::new(test_client(&base), Mission::Ullyses);
        let products: DatasetProducts<DatasetProduct> = endpoint
            .post_list_products(&DatasetIds::new(["a", "b"]))
            .await
            .unwrap();
        assert!(products.products.is_empty());
        let request = server.await.unwrap();
        assert_eq!(
            request.target,
            "/search/ullyses/api/v0.1/post_list_products"
        );
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({"dataset_ids": ["a", "b"]})
        );
    }

    #[tokio::test]
    async fn image_requests_only_send_present_query_values() {
        let (base, server) = serve_once("200 OK", "application/octet-stream", JPEG_HEAD.to_vec()).await;
        let endpoint = MissionEndpoint::new(test_client(&base), Mission::Roman);
        let bytes = endpoint
            .get_image_thumbnail(None, Some("r0001"))
            .await
            .unwrap();
        assert_eq!(bytes, JPEG_HEAD.to_vec());
        let request = server.await.unwrap();
        assert_eq!(
            request.target,
            "/search/roman/api/v0.1/get_image_thumbnail?fileset_name=r0001"
        );
    }

    #[tokio::test]
    async fn auth_check_decodes_the_authorized_flag() {
        let (base, server) = serve_once("200 OK", "application/json", br#"{"authorized":false}"#.to_vec()).await;
        let endpoint = MissionEndpoint::new(test_client(&base), Mission::Roman);
        let check = endpoint.auth_check().await.unwrap();
        assert!(!check.authorized);
        assert_eq!(server.await.unwrap().target, "/search/roman/api/v0.1/auth_check");
    }

    #[tokio::test]
    async fn no_content_search_yields_empty_results() {
        let (base, server) = serve_once("204 No Content", "application/json", Vec::new()).await;
        let endpoint = MissionEndpoint::new(test_client(&base), Mission::Tess);
        let results: SearchResults<Value> = endpoint.search(&SearchParams::new()).await.unwrap();
        server.await.unwrap();
        assert!(results.results.is_empty());
        assert_eq!(results.total_results, -1);
    }
}
