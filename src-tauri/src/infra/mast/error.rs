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
    #[error("MAST request to {url} did not finish within {timeout_secs} s")]
    Timeout { url: String, timeout_secs: u64 },
    #[error("MAST response from {url} exceeded {limit} bytes ({bytes} bytes)")]
    TooLarge { url: String, bytes: u64, limit: u64 },
    #[error("MAST request to {url} was redirected to a host outside the allowlist")]
    RedirectRefused { url: String },
    #[error("MAST response from {url} is not an image")]
    NotImage { url: String },
    #[error("MAST returned HTTP {status} for {url}: {body}")]
    Status {
        status: u16,
        url: String,
        body: String,
        detail: Option<String>,
        content_type: Option<String>,
        retry_after_secs: Option<u64>,
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
    #[error("failed to store a MAST download: {0}")]
    Io(#[source] std::io::Error),
}

pub type MastResult<T> = Result<T, MastError>;
