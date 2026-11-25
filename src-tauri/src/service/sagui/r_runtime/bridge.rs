use anyhow::{ensure, Context, Result};
use serde_json::json;

#[derive(Debug, Default)]
pub struct BridgeReport {
    pub clustered_pixels: Option<u64>,
    pub clustering_memory_bytes: Option<u64>,
    pub hclust_limit: Option<u64>,
    pub warnings: Vec<String>,
}

pub(in crate::service::sagui) fn parse_bridge_markers(text: &str) -> BridgeReport {
    let mut report = BridgeReport::default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ASTROBURST_CLUSTER\t") {
            let mut fields = rest.split('\t');
            report.clustered_pixels = fields.next().and_then(|v| v.trim().parse().ok());
            report.clustering_memory_bytes = fields.next().and_then(|v| v.trim().parse().ok());
            report.hclust_limit = fields.next().and_then(|v| v.trim().parse().ok());
        } else if let Some(rest) = line.strip_prefix("ASTROBURST_WARNING\t") {
            report.warnings.push(rest.into());
        }
    }
    report
}

pub(in crate::service::sagui) fn parse_backend(text: &str) -> Result<serde_json::Value> {
    let mut lines = text.lines();
    ensure!(lines.next() == Some("knn_k\trequested_Ncomp\tactual_Ncomp\tdisconnected\tvalid_pixels"), "Malformed SAGUI backend record");
    let fields: Vec<_> = lines.next().context("Missing SAGUI backend record")?.split('\t').collect();
    ensure!(fields.len() == 5, "Malformed SAGUI backend record");
    Ok(json!({ "knn_k_used": fields[0].trim().parse::<u64>()?, "requested_regions": fields[1].trim().parse::<u64>()?, "actual_regions": fields[2].trim().parse::<u64>()?, "disconnected": fields[3].trim() == "TRUE", "valid_pixels": fields[4].trim().parse::<u64>()? }))
}
