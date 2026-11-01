use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Operation cancelled")]
    Cancelled,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct CodedError {
    pub code: &'static str,
    pub message: String,
    pub hint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_reaches_the_frontend_as_a_cancel_message() {
        let e: anyhow::Error = AppError::Cancelled.into();
        assert!(format!("{e:#}").to_lowercase().contains("cancel"));
    }
}
