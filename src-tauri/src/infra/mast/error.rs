use super::models::HttpValidationError;

#[derive(Debug, thiserror::Error)]
pub enum MastError {
    #[error("failed to build MAST HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("failed to encode MAST request body: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("MAST request to {url} failed: {source}")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("MAST returned HTTP {status} for {url}: {body}")]
    Status {
        status: u16,
        url: String,
        body: String,
    },
    #[error("MAST rejected the request to {url}: {summary}")]
    Validation {
        url: String,
        summary: String,
        detail: HttpValidationError,
    },
    #[error("failed to decode MAST response from {url}: {source}")]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

pub type MastResult<T> = Result<T, MastError>;
