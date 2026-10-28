pub mod calibration;
pub mod cfa_guard;
pub mod combine;
pub mod consistency;
pub mod drizzle;
pub mod frame_cards;
pub mod masters;
pub mod memory;

use crate::types::error::AppError;

pub const STAGE_ALIGN: &str = "align";
pub const STAGE_COMBINE: &str = "combine";
pub const STAGE_CALIBRATE: &str = "calibrate";
pub const STAGE_DRIZZLE: &str = "drizzle";
pub const STAGE_LOW_MEMORY: &str = "low_memory";

pub type CancelCheck<'a> = &'a (dyn Fn() -> bool + Sync);

pub type StageCallback<'a> = &'a (dyn Fn(&str) + Sync);

pub fn never_cancelled() -> bool {
    false
}

pub fn no_stage(_stage: &str) {}

pub(crate) fn stop_if_cancelled(cancelled: CancelCheck) -> anyhow::Result<()> {
    if cancelled() {
        return Err(AppError::Cancelled.into());
    }
    Ok(())
}

#[cfg(any(test, feature = "tauri"))]
pub(crate) fn is_cancellation(err: &anyhow::Error) -> bool {
    matches!(err.downcast_ref::<AppError>(), Some(AppError::Cancelled))
}
