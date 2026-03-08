use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("FITS I/O error: {0}")]
    FitsIo(#[from] std::io::Error),

    #[error("Invalid FITS format: {0}")]
    FitsFormat(String),

    #[error("Image processing error: {0}")]
    Processing(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Astrometry API error: {0}")]
    Astrometry(String),

    #[error("Deconvolution error: {0}")]
    Deconvolution(String),

    #[error("Stacking error: {0}")]
    Stacking(String),

    #[error("RGB compose error: {0}")]
    Compose(String),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl From<AppError> for String {
    fn from(e: AppError) -> Self {
        format!("{:#}", e)
    }
}

pub type AppResult<T> = Result<T, AppError>;
