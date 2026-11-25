use std::io;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("{0}")]
    ReadOnly(String),
    #[error("{path} cannot be read as an AstroBurst history: {reason}")]
    Unreadable { path: String, reason: String },
    #[error("no node {0} in this history")]
    UnknownNode(String),
    #[error("node {0} is already in this history; node files are never overwritten")]
    NodeExists(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
