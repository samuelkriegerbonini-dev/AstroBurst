#[derive(Debug, Clone, thiserror::Error)]
pub enum GwcsError {
    #[error("gWCS transform '{tag}' at {path} is not supported by the evaluator")]
    UnsupportedTag { tag: String, path: String },
    #[error("gWCS transform '{tag}' at {path} has no inverse (none stored, none derivable)")]
    NoInverse { tag: String, path: String },
    #[error("gWCS transform at {path}: {reason}")]
    Shape { path: String, reason: String },
    #[error("gWCS frame: {0}")]
    Frame(String),
    #[error("gWCS array at {path}: {reason}")]
    Arrays { path: String, reason: String },
    #[error("gWCS node at {path} lacks '{field}'")]
    Missing { path: String, field: String },
    #[error("gWCS bounding box: {0}")]
    Bbox(String),
    #[error("gWCS: {0}")]
    Parse(String),
}
