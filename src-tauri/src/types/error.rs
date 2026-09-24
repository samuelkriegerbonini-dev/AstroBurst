use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Operation cancelled")]
    Cancelled,
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
