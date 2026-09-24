pub mod calibration;
pub mod combine;
pub mod drizzle;

use crate::types::error::AppError;

pub type CancelCheck<'a> = &'a (dyn Fn() -> bool + Sync);

pub fn never_cancelled() -> bool {
    false
}

pub(crate) fn stop_if_cancelled(cancelled: CancelCheck) -> anyhow::Result<()> {
    if cancelled() {
        return Err(AppError::Cancelled.into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn is_cancellation(err: &anyhow::Error) -> bool {
    matches!(err.downcast_ref::<AppError>(), Some(AppError::Cancelled))
}
