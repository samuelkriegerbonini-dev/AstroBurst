pub mod combination;
pub mod errors;
pub mod images;
pub mod settings;

pub use combination::{compose, stacking};
pub use errors::error;
pub use images::{header, image, image_ref};
pub use settings::{config, constants};

pub use header::HduHeader;
pub use image::ImageStats;
pub use image_ref::{ImageRef, PlaneSelector};
