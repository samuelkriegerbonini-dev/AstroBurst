use super::models::TessSearchResults;
use crate::infra::mast::api::endpoint::MissionEndpoint;
use crate::infra::mast::api::mission::Mission;
use crate::infra::mast::client::MastClient;
use crate::infra::mast::error::MastResult;
use crate::infra::mast::models::SearchParams;

#[derive(Debug, Clone)]
pub struct TessApi {
    endpoint: MissionEndpoint,
}

impl TessApi {
    pub const MISSION: Mission = Mission::Tess;

    pub fn new(client: MastClient) -> Self {
        Self {
            endpoint: MissionEndpoint::new(client, Self::MISSION),
        }
    }

    pub fn endpoint(&self) -> &MissionEndpoint {
        &self.endpoint
    }

    pub async fn search(&self, params: &SearchParams) -> MastResult<TessSearchResults> {
        self.endpoint.search(params).await
    }
}
