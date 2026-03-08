use anyhow::{bail, Result};
use ndarray::Array2;

use crate::domain::calibration::{CalibrationConfig, load_fits_image};

pub use crate::core::stacking::drizzle::*;

pub fn drizzle_from_paths(
    paths: &[String],
    config: &DrizzleConfig,
    calibration: Option<&CalibrationConfig>,
) -> Result<DrizzleResult> {
    if paths.is_empty() {
        bail!("No image paths provided");
    }

    let mut images: Vec<Array2<f32>> = Vec::with_capacity(paths.len());
    for path in paths {
        let mut img = load_fits_image(path)?;
        if let Some(cal) = calibration {
            img = crate::domain::calibration::calibrate_image(&img, cal);
        }
        images.push(img);
    }

    drizzle_stack(&images, config)
}
