use anyhow::{bail, Result};
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use rayon::prelude::*;

const MIN_COLUMN_WEIGHT: f64 = 1e-6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendWeight {
    pub channel_idx: usize,
    pub r_weight: f64,
    pub g_weight: f64,
    pub b_weight: f64,
}

fn empty_output_columns(weights: &[BlendWeight]) -> Vec<&'static str> {
    let totals = [
        ("R", weights.iter().map(|w| w.r_weight).sum::<f64>()),
        ("G", weights.iter().map(|w| w.g_weight).sum::<f64>()),
        ("B", weights.iter().map(|w| w.b_weight).sum::<f64>()),
    ];
    totals
        .iter()
        .filter(|(_, total)| total.abs() < MIN_COLUMN_WEIGHT)
        .map(|(name, _)| *name)
        .collect()
}

pub fn validate_blend_weights(weights: &[BlendWeight], channel_count: usize) -> Result<()> {
    let usable: Vec<BlendWeight> = weights
        .iter()
        .filter(|w| w.channel_idx < channel_count)
        .cloned()
        .collect();

    if usable.is_empty() {
        bail!(
            "Blend weight matrix is empty: none of the {} weight row(s) reference one of the {} loaded channel(s)",
            weights.len(),
            channel_count
        );
    }

    let empty = empty_output_columns(&usable);
    if !empty.is_empty() {
        let feeders: Vec<String> = usable
            .iter()
            .map(|w| format!("channel {}", w.channel_idx))
            .collect();
        bail!(
            "Blend weight matrix leaves output {} empty: every weight in that column is zero, which would render a black plane. Expected a non-zero weight from one of: {}",
            if empty.len() == 1 {
                format!("channel {}", empty[0])
            } else {
                format!("channels {}", empty.join(" and "))
            },
            feeders.join(", ")
        );
    }

    Ok(())
}

pub fn blend_channels(
    channels: &[&Array2<f32>],
    weights: &[BlendWeight],
    rows: usize,
    cols: usize,
) -> Result<(Array2<f32>, Array2<f32>, Array2<f32>)> {
    validate_blend_weights(weights, channels.len())?;

    let npix = rows * cols;

    let valid_weights: Vec<(usize, f32, f32, f32)> = weights
        .iter()
        .filter(|w| w.channel_idx < channels.len())
        .filter(|w| w.r_weight != 0.0 || w.g_weight != 0.0 || w.b_weight != 0.0)
        .map(|w| (w.channel_idx, w.r_weight as f32, w.g_weight as f32, w.b_weight as f32))
        .collect();

    let slices: Vec<&[f32]> = channels
        .iter()
        .map(|ch| ch.as_slice().unwrap_or(&[]))
        .collect();

    let mut r_out = vec![0.0f32; npix];
    let mut g_out = vec![0.0f32; npix];
    let mut b_out = vec![0.0f32; npix];

    r_out
        .par_chunks_mut(cols)
        .zip(g_out.par_chunks_mut(cols))
        .zip(b_out.par_chunks_mut(cols))
        .enumerate()
        .for_each(|(row_idx, ((r_row, g_row), b_row))| {
            let base = row_idx * cols;
            for x in 0..cols {
                let i = base + x;
                let mut rv = 0.0f32;
                let mut gv = 0.0f32;
                let mut bv = 0.0f32;

                for &(ch_idx, rw, gw, bw) in &valid_weights {
                    let src = slices[ch_idx];
                    if i >= src.len() {
                        continue;
                    }
                    let v = src[i];
                    if !v.is_finite() {
                        continue;
                    }
                    rv += v * rw;
                    gv += v * gw;
                    bv += v * bw;
                }

                r_row[x] = rv;
                g_row[x] = gv;
                b_row[x] = bv;
            }
        });

    let r = Array2::from_shape_vec((rows, cols), r_out)?;
    let g = Array2::from_shape_vec((rows, cols), g_out)?;
    let b = Array2::from_shape_vec((rows, cols), b_out)?;

    Ok((r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::arr2;

    fn weight(channel_idx: usize, r_weight: f64, g_weight: f64, b_weight: f64) -> BlendWeight {
        BlendWeight { channel_idx, r_weight, g_weight, b_weight }
    }

    #[test]
    fn rejects_matrix_with_an_empty_color_column() {
        let ha = arr2(&[[1.0f32, 2.0], [3.0, 4.0]]);
        let oiii = arr2(&[[5.0f32, 6.0], [7.0, 8.0]]);
        let channels = [&ha, &oiii];
        let weights = vec![weight(0, 0.0, 1.0, 0.0), weight(1, 0.0, 0.5, 0.0)];

        let err = blend_channels(&channels, &weights, 2, 2).unwrap_err().to_string();

        assert!(err.contains("R and B"), "error must name the empty columns: {}", err);
        assert!(err.contains("channel 0") && err.contains("channel 1"), "error must name the feeders: {}", err);
    }

    #[test]
    fn rejects_matrix_with_a_single_empty_column() {
        let a = arr2(&[[1.0f32, 2.0]]);
        let b = arr2(&[[3.0f32, 4.0]]);
        let channels = [&a, &b];
        let weights = vec![weight(0, 1.0, 0.0, 0.0), weight(1, 0.0, 0.0, 1.0)];

        let err = blend_channels(&channels, &weights, 1, 2).unwrap_err().to_string();

        assert!(err.contains("channel G"), "error must name the empty column: {}", err);
    }

    #[test]
    fn rejects_matrix_whose_rows_point_past_the_loaded_channels() {
        let a = arr2(&[[1.0f32, 2.0]]);
        let channels = [&a];
        let weights = vec![weight(7, 1.0, 1.0, 1.0)];

        let err = blend_channels(&channels, &weights, 1, 2).unwrap_err().to_string();

        assert!(err.contains("empty"), "error must report an unusable matrix: {}", err);
    }

    #[test]
    fn accepts_matrix_covering_every_column_and_sums_contributions() {
        let a = arr2(&[[1.0f32, 2.0]]);
        let b = arr2(&[[10.0f32, 20.0]]);
        let channels = [&a, &b];
        let weights = vec![weight(0, 1.0, 0.5, 0.0), weight(1, 0.0, 0.5, 1.0)];

        let (r, g, blue) = blend_channels(&channels, &weights, 1, 2).unwrap();

        assert_eq!(r[[0, 0]], 1.0);
        assert_eq!(g[[0, 0]], 0.5 * 1.0 + 0.5 * 10.0);
        assert_eq!(blue[[0, 0]], 10.0);
        assert_eq!(r[[0, 1]], 2.0);
        assert_eq!(g[[0, 1]], 0.5 * 2.0 + 0.5 * 20.0);
        assert_eq!(blue[[0, 1]], 20.0);
    }

    #[test]
    fn non_finite_feeder_does_not_inflate_the_surviving_channels() {
        let bright = arr2(&[[10.0f32, 10.0]]);
        let faint = arr2(&[[f32::NAN, 2.0]]);
        let channels = [&bright, &faint];
        let weights = vec![weight(0, 1.0, 1.0, 1.0), weight(1, 0.5, 0.5, 0.5)];

        let (r, g, b) = blend_channels(&channels, &weights, 1, 2).unwrap();

        assert_eq!(r[[0, 0]], 10.0);
        assert_eq!(g[[0, 0]], 10.0);
        assert_eq!(b[[0, 0]], 10.0);
        assert_eq!(r[[0, 1]], 11.0);
    }

    #[test]
    fn pixels_with_no_finite_feeder_stay_zero_for_the_padding_rule() {
        let a = arr2(&[[f32::NAN, 4.0]]);
        let b = arr2(&[[f32::INFINITY, 6.0]]);
        let channels = [&a, &b];
        let weights = vec![weight(0, 1.0, 1.0, 0.0), weight(1, 0.0, 1.0, 1.0)];

        let (r, g, blue) = blend_channels(&channels, &weights, 1, 2).unwrap();

        assert_eq!(r[[0, 0]], 0.0);
        assert_eq!(g[[0, 0]], 0.0);
        assert_eq!(blue[[0, 0]], 0.0);
        assert_eq!(r[[0, 1]], 4.0);
    }
}
