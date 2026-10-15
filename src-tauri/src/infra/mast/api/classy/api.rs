use super::models::ClassySearchResults;
use crate::infra::mast::api::endpoint::MissionEndpoint;
use crate::infra::mast::api::mission::Mission;
use crate::infra::mast::client::MastClient;
use crate::infra::mast::error::MastResult;
use crate::infra::mast::models::{DatasetIds, DatasetProducts, SearchParams};

#[derive(Debug, Clone)]
pub struct ClassyApi {
    endpoint: MissionEndpoint,
}

impl ClassyApi {
    pub const MISSION: Mission = Mission::Classy;

    pub fn new(client: MastClient) -> Self {
        Self {
            endpoint: MissionEndpoint::new(client, Self::MISSION),
        }
    }

    pub fn endpoint(&self) -> &MissionEndpoint {
        &self.endpoint
    }

    pub async fn search(&self, params: &SearchParams) -> MastResult<ClassySearchResults> {
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
}
