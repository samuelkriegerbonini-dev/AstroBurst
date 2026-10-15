use crate::infra::mast::api::endpoint::MissionEndpoint;
use crate::infra::mast::api::mission::Mission;
use crate::infra::mast::api::roman::{RomanDatasetProducts, RomanSearchResults};
use crate::infra::mast::client::MastClient;
use crate::infra::mast::error::MastResult;
use crate::infra::mast::models::{DatasetIds, SearchParams};

#[derive(Debug, Clone)]
pub struct RomanCgiApi {
    endpoint: MissionEndpoint,
}

impl RomanCgiApi {
    pub const MISSION: Mission = Mission::RomanCgi;

    pub fn new(client: MastClient) -> Self {
        Self {
            endpoint: MissionEndpoint::new(client, Self::MISSION),
        }
    }

    pub fn endpoint(&self) -> &MissionEndpoint {
        &self.endpoint
    }

    pub async fn search(&self, params: &SearchParams) -> MastResult<RomanSearchResults> {
        self.endpoint.search(params).await
    }

    pub async fn auth_check(&self) -> MastResult<RomanSearchResults> {
        self.endpoint.auth_check().await
    }

    pub async fn list_products(
        &self,
        dataset_ids: Option<&str>,
    ) -> MastResult<RomanDatasetProducts> {
        self.endpoint.list_products(dataset_ids).await
    }

    pub async fn post_list_products(
        &self,
        dataset_ids: &DatasetIds,
    ) -> MastResult<RomanDatasetProducts> {
        self.endpoint.post_list_products(dataset_ids).await
    }

    pub async fn retrieve_product(&self, product_name: &str) -> MastResult<Vec<u8>> {
        self.endpoint.retrieve_product(product_name).await
    }
}
