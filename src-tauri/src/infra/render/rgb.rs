use anyhow::{Context, Result};
use image::RgbImage;
use ndarray::Array2;
use rayon::prelude::*;

pub fn render_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    path: &str,
) -> Result<()> {
    let (rows, cols) = r.dim();
    let r_slice = r.as_slice().context("R channel not contiguous")?;
    let g_slice = g.as_slice().context("G channel not contiguous")?;
    let b_slice = b.as_slice().context("B channel not contiguous")?;

    let pixels: Vec<u8> = (0..rows * cols)
        .into_par_iter()
        .flat_map_iter(|i| {
            let rv = (r_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
            let gv = (g_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
            let bv = (b_slice[i].clamp(0.0, 1.0) * 255.0) as u8;
            [rv, gv, bv]
        })
        .collect();

    let img = RgbImage::from_raw(cols as u32, rows as u32, pixels)
        .context("Failed to create RGB image from raw pixels")?;
    img.save(path).context("Failed to save RGB PNG")?;
    Ok(())
}
