use anyhow::{bail, Result};
use ndarray::{Array2, Zip};

use crate::core::alignment::pair::align_pair_with_label;
use crate::core::imaging::resample::resample_image;

pub use crate::types::compose::{AlignMethod, WhiteBalance, ChannelStats, DimensionHarmonize};

pub fn harmonize_dimensions(
    r: Option<&Array2<f32>>,
    g: Option<&Array2<f32>>,
    b: Option<&Array2<f32>>,
    max_ratio: f64,
) -> Result<(
    Option<Array2<f32>>,
    Option<Array2<f32>>,
    Option<Array2<f32>>,
    usize,
    usize,
    Option<DimensionHarmonize>,
)> {
    let dims: Vec<(usize, usize)> = [r, g, b]
        .into_iter()
        .flatten()
        .map(|a| a.dim())
        .collect();

    if dims.is_empty() {
        return Ok((None, None, None, 0, 0, None));
    }

    let min_rows = dims.iter().map(|d| d.0).min().unwrap();
    let min_cols = dims.iter().map(|d| d.1).min().unwrap();
    let max_rows = dims.iter().map(|d| d.0).max().unwrap();
    let max_cols = dims.iter().map(|d| d.1).max().unwrap();

    if max_rows == min_rows && max_cols == min_cols {
        return Ok((None, None, None, max_rows, max_cols, None));
    }

    let ratio_rows = max_rows as f64 / min_rows.max(1) as f64;
    let ratio_cols = max_cols as f64 / min_cols.max(1) as f64;
    let ratio = ratio_rows.max(ratio_cols);

    if ratio > max_ratio {
        let mut msg = format!(
            "Channel dimension ratio {:.1}x exceeds {:.0}x limit.",
            ratio, max_ratio
        );
        if let Some(ra) = r { msg.push_str(&format!(" R={}x{}", ra.dim().1, ra.dim().0)); }
        if let Some(ga) = g { msg.push_str(&format!(" G={}x{}", ga.dim().1, ga.dim().0)); }
        if let Some(ba) = b { msg.push_str(&format!(" B={}x{}", ba.dim().1, ba.dim().0)); }
        msg.push_str(". Check channel assignments.");
        bail!("{}", msg);
    }

    log::info!(
        "harmonize_dimensions: resampling channels to {}x{} (ratio {:.2}x)",
        max_cols, max_rows, ratio
    );

    let info = DimensionHarmonize {
        original_r: r.map(|a| [a.dim().1, a.dim().0]),
        original_g: g.map(|a| [a.dim().1, a.dim().0]),
        original_b: b.map(|a| [a.dim().1, a.dim().0]),
        target: [max_cols, max_rows],
        resampled: true,
    };

    let resample_ch = |channel: Option<&Array2<f32>>| -> Result<Option<Array2<f32>>> {
        match channel {
            Some(a) => {
                let (rows, cols) = a.dim();
                if rows == max_rows && cols == max_cols {
                    Ok(Some(a.clone()))
                } else {
                    Ok(Some(resample_image(a, max_rows, max_cols)?))
                }
            }
            None => Ok(None),
        }
    };

    Ok((
        resample_ch(r)?,
        resample_ch(g)?,
        resample_ch(b)?,
        max_rows,
        max_cols,
        Some(info),
    ))
}

fn channel_or_synth(
    primary: Option<&Array2<f32>>,
    alt1: Option<&Array2<f32>>,
    alt2: Option<&Array2<f32>>,
    rows: usize,
    cols: usize,
) -> Array2<f32> {
    if let Some(ch) = primary { return ch.clone(); }
    match (alt1, alt2) {
        (Some(a), Some(b)) => {
            let mut out = Array2::zeros((rows, cols));
            Zip::from(&mut out).and(a).and(b)
                .par_for_each(|o, &av, &bv| *o = (av + bv) * 0.5);
            out
        }
        (Some(a), None) => a.clone(),
        (None, Some(b)) => b.clone(),
        (None, None) => Array2::zeros((rows, cols)),
    }
}

pub(crate) fn align_channels(
    r: Option<&Array2<f32>>, g: Option<&Array2<f32>>, b: Option<&Array2<f32>>,
    rows: usize, cols: usize, method: AlignMethod,
) -> Result<(Array2<f32>, Array2<f32>, Array2<f32>, (f64, f64), (f64, f64))> {
    let Some(ref_ch) = r.or(g).or(b) else {
        bail!("No channel provided: assign at least one of R, G or B.");
    };

    let align_to_ref = |ch: Option<&Array2<f32>>,
                        label: &str|
     -> Result<(Option<Array2<f32>>, (f64, f64))> {
        match ch {
            Some(img) if std::ptr::eq(img, ref_ch) => Ok((Some(img.clone()), (0.0, 0.0))),
            Some(img) => {
                let res = align_pair_with_label(ref_ch, img, method, rows, cols, label)?;
                Ok((Some(res.aligned), res.offset))
            }
            None => Ok((None, (0.0, 0.0))),
        }
    };

    let (g_aligned, off_g) = align_to_ref(g, "G")?;
    let (b_aligned, off_b) = align_to_ref(b, "B")?;

    let r_img = channel_or_synth(r, g_aligned.as_ref(), b_aligned.as_ref(), rows, cols);
    let g_img = match g_aligned {
        Some(img) => img,
        None => channel_or_synth(None, r, b_aligned.as_ref(), rows, cols),
    };
    let b_img = match b_aligned {
        Some(img) => img,
        None => channel_or_synth(None, r, Some(&g_img), rows, cols),
    };

    Ok((r_img, g_img, b_img, off_g, off_b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligning_without_any_channel_is_an_error_not_a_panic() {
        let (none_r, none_g, none_b, rows, cols, info) = harmonize_dimensions(None, None, None, 8.0).unwrap();
        assert!(none_r.is_none() && none_g.is_none() && none_b.is_none() && info.is_none());
        let err = align_channels(None, None, None, rows, cols, AlignMethod::PhaseCorrelation)
            .expect_err("three missing channels must be refused");
        assert!(err.to_string().contains("No channel provided"), "{err}");
    }

    #[test]
    fn a_single_channel_is_its_own_reference_and_fills_the_others() {
        let only_g = Array2::from_shape_fn((8, 8), |(y, x)| (y * 8 + x) as f32);
        let (r, g, b, off_g, off_b) =
            align_channels(None, Some(&only_g), None, 8, 8, AlignMethod::PhaseCorrelation).unwrap();
        assert_eq!(g, only_g);
        assert_eq!(r, only_g);
        assert_eq!(b, only_g);
        assert_eq!(off_g, (0.0, 0.0));
        assert_eq!(off_b, (0.0, 0.0));
    }
}
