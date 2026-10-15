use serde_json::Value;

use crate::infra::mast::api::endpoint::{present, MissionEndpoint};
use crate::infra::mast::api::mission::Mission;
use crate::infra::mast::client::MastClient;
use crate::infra::mast::error::MastResult;
use crate::infra::mast::models::{DatasetProducts, ImageListResult, SearchParams};

#[derive(Debug, Clone)]
pub struct MultiMissionApi {
    endpoint: MissionEndpoint,
}

impl MultiMissionApi {
    pub const MISSION: Mission = Mission::MultiMission;

    pub fn new(client: MastClient) -> Self {
        Self {
            endpoint: MissionEndpoint::new(client, Self::MISSION),
        }
    }

    pub fn endpoint(&self) -> &MissionEndpoint {
        &self.endpoint
    }

    pub async fn search(&self, params: &SearchParams) -> MastResult<Value> {
        self.endpoint.post_operation("search", params).await
    }

    pub async fn list_products(&self, dataset_ids: Option<&str>) -> MastResult<DatasetProducts> {
        self.endpoint.list_products(dataset_ids).await
    }

    pub async fn get_image(&self, uri: &str, output_size: Option<u32>) -> MastResult<Vec<u8>> {
        let size = output_size.map(|value| value.to_string());
        let query = present([("uri", Some(uri)), ("output_size", size.as_deref())]);
        self.endpoint
            .client()
            .get_bytes(&self.endpoint.path("get_image"), &query)
            .await
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
        fileset_name: &str,
        filename: Option<&str>,
    ) -> MastResult<Vec<u8>> {
        self.endpoint
            .get_image_preview(filename, Some(fileset_name))
            .await
    }

    pub async fn get_image_thumbnail(
        &self,
        fileset_name: &str,
        filename: Option<&str>,
    ) -> MastResult<Vec<u8>> {
        self.endpoint
            .get_image_thumbnail(filename, Some(fileset_name))
            .await
    }
}
