use crate::core::mask::MaskError;

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("{0} does not take a mask")]
    MaskNotSupported(&'static str),
    #[error(transparent)]
    Mask(#[from] MaskError),
    #[error("{op} cannot run on {planes} planes")]
    PlaneCount { op: &'static str, planes: usize },
    #[error("{op} needs planes of one size; got {dims:?}")]
    PlaneDims { op: &'static str, dims: Vec<(usize, usize)> },
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}
