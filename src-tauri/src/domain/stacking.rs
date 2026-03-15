use anyhow::{bail, Result};
use ndarray::Array2;

use crate::domain::calibration::{CalibrationConfig, load_fits_image};

pub use crate::core::stacking::combine::stack_images;
pub use crate::types::stacking::{StackConfig, StackResult};

pub fn stack_from_paths(
    paths: &[String],
    config: &StackConfig,
    calibration: Option<&CalibrationConfig>,
) -> Result<StackResult> {
    if paths.is_empty() {
        bail!("No image paths provided");
    }

    let images: Vec<Array2<f32>> = paths
        .iter()
        .map(|path| {
            let img = load_fits_image(path)?;
            match calibration {
                Some(cal) => Ok(crate::domain::calibration::calibrate_image(&img, cal)),
                None => Ok(img),
            }
        })
        .collect::<Result<_>>()?;

    stack_images(&images, config)
}
