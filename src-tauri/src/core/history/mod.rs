pub mod document;
pub mod storage;
pub mod tree;

#[cfg(test)]
mod test_support;

pub use document::{id, stamp};
pub use storage::kernels;
#[cfg(test)]
pub(crate) use test_support::fixtures;

pub use document::*;
pub use storage::*;
pub use tree::*;
