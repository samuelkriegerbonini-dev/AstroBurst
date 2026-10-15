use super::models::JwstSearchResults;
use crate::infra::mast::api::endpoint::MissionEndpoint;
use crate::infra::mast::api::mission::Mission;
use crate::infra::mast::client::MastClient;
use crate::infra::mast::error::MastResult;
use crate::infra::mast::models::{DatasetIds, DatasetProducts, ImageListResult, SearchParams};

#[derive(Debug, Clone)]
pub struct JwstApi {
    endpoint: MissionEndpoint,
}

impl JwstApi {
    pub const MISSION: Mission = Mission::Jwst;

    pub fn new(client: MastClient) -> Self {
        Self {
            endpoint: MissionEndpoint::new(client, Self::MISSION),
        }
    }

    pub fn endpoint(&self) -> &MissionEndpoint {
        &self.endpoint
    }

    pub async fn search(&self, params: &SearchParams) -> MastResult<JwstSearchResults> {
        self.endpoint.search(params).await
    }

    pub async fn list_products(&self, dataset_ids: Option<&str>) -> MastResult<DatasetProducts> {
        self.endpoint.list_products(dataset_ids).await
    }

    pub async fn post_list_products(
        &self,
        dataset_ids: &DatasetIds,
    ) -> MastResult<DatasetProducts> {
        self.endpoint.post_list_products(dataset_ids).await
    }

    pub async fn get_image_list(
        &self,
        fileset_name: &str,
        category: Option<&str>,
    ) -> MastResult<Vec<ImageListResult>> {
        self.endpoint.get_image_list(fileset_name, category).await
    }

    pub async fn get_image_preview(
        &self,
        filename: Option<&str>,
        fileset_name: Option<&str>,
    ) -> MastResult<Vec<u8>> {
        self.endpoint
            .get_image_preview(filename, fileset_name)
            .await
    }

    pub async fn get_image_thumbnail(
        &self,
        filename: Option<&str>,
        fileset_name: Option<&str>,
    ) -> MastResult<Vec<u8>> {
        self.endpoint
            .get_image_thumbnail(filename, fileset_name)
            .await
    }

    pub async fn retrieve_product(&self, product_name: &str) -> MastResult<Vec<u8>> {
        self.endpoint.retrieve_product(product_name).await
    }
}
