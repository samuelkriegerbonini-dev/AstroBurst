use anyhow::{bail, Result};
use ndarray::{Array2, Zip};

pub fn apply_lrgb(
    l: &Array2<f32>,
    r: &mut Array2<f32>,
    g: &mut Array2<f32>,
    b: &mut Array2<f32>,
    lightness_weight: f32,
    chrominance_weight: f32,
) -> Result<()> {
    let dim = l.dim();

    if r.dim() != dim || g.dim() != dim || b.dim() != dim {
        bail!(
            "L dimensions {:?} do not match RGB (R: {:?}, G: {:?}, B: {:?})",
            dim, r.dim(), g.dim(), b.dim()
        );
    }

    Zip::from(r)
        .and(g)
        .and(b)
        .and(l)
        .par_for_each(|rv, gv, bv, &lum_new| {
            let lum_old = *rv * 0.2126 + *gv * 0.7152 + *bv * 0.0722;

            if lum_old < 1e-10 {
                let blended = lum_new * lightness_weight;
                *rv = blended;
                *gv = blended;
                *bv = blended;
                return;
            }

            let ratio = (lum_new * lightness_weight + lum_old * (1.0 - lightness_weight)) / lum_old;
            let chroma_blend = chrominance_weight;

            *rv = (*rv * ratio * chroma_blend + lum_new * (1.0 - chroma_blend)).clamp(0.0, 1.0);
            *gv = (*gv * ratio * chroma_blend + lum_new * (1.0 - chroma_blend)).clamp(0.0, 1.0);
            *bv = (*bv * ratio * chroma_blend + lum_new * (1.0 - chroma_blend)).clamp(0.0, 1.0);
        });

    Ok(())
}

fn finite_min_max(arrays: &[&Array2<f32>]) -> Option<(f32, f32)> {
    let mut mn = f32::INFINITY;
    let mut mx = f32::NEG_INFINITY;
    for arr in arrays {
        for &v in arr.iter() {
            if v.is_finite() {
                if v < mn {
                    mn = v;
                }
                if v > mx {
                    mx = v;
                }
            }
        }
    }
    if mn < mx {
        Some((mn, mx))
    } else {
        None
    }
}

fn finite_reciprocal(span: f32) -> Option<f32> {
    let inv = 1.0 / span;
    (span.is_finite() && span > 0.0 && inv.is_finite()).then_some(inv)
}

fn normalize_min_max(arr: &Array2<f32>, mn: f32, mx: f32) -> Result<Array2<f32>> {
    let inv = finite_reciprocal(mx - mn).ok_or_else(|| {
        anyhow::anyhow!("Luminance range [{}, {}] is too narrow to normalize", mn, mx)
    })?;
    Ok(arr.mapv(|v| if v.is_finite() { ((v - mn) * inv).clamp(0.0, 1.0) } else { v }))
}

fn scale_by_max(arr: &Array2<f32>, max: f32) -> Result<Array2<f32>> {
    let inv = finite_reciprocal(max)
        .ok_or_else(|| anyhow::anyhow!("RGB peak {} is too small to scale by", max))?;
    Ok(arr.mapv(|v| if v.is_finite() { (v * inv).clamp(0.0, 1.0) } else { v }))
}

pub fn lrgb_combine_normalized(
    l: &Array2<f32>,
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    lightness_weight: f32,
    chrominance_weight: f32,
) -> Result<(Array2<f32>, Array2<f32>, Array2<f32>)> {
    let (l_min, l_max) = finite_min_max(&[l])
        .ok_or_else(|| anyhow::anyhow!("Luminance channel has no dynamic range"))?;
    let mut rgb_max = f32::NEG_INFINITY;
    for arr in [r, g, b] {
        for &v in arr.iter() {
            if v.is_finite() && v > rgb_max {
                rgb_max = v;
            }
        }
    }
    if !rgb_max.is_finite() || rgb_max <= 0.0 {
        bail!("RGB composite has no positive signal");
    }

    let l_norm = normalize_min_max(l, l_min, l_max)?;
    let mut rn = scale_by_max(r, rgb_max)?;
    let mut gn = scale_by_max(g, rgb_max)?;
    let mut bn = scale_by_max(b, rgb_max)?;

    apply_lrgb(&l_norm, &mut rn, &mut gn, &mut bn, lightness_weight, chrominance_weight)?;

    Ok((rn, gn, bn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lrgb_preserves_gray() {
        let l = Array2::from_elem((10, 10), 0.5f32);
        let mut r = Array2::from_elem((10, 10), 0.5f32);
        let mut g = Array2::from_elem((10, 10), 0.5f32);
        let mut b = Array2::from_elem((10, 10), 0.5f32);

        apply_lrgb(&l, &mut r, &mut g, &mut b, 1.0, 1.0).unwrap();

        for y in 0..10 {
            for x in 0..10 {
                assert!((r[[y, x]] - 0.5).abs() < 0.01);
                assert!((g[[y, x]] - 0.5).abs() < 0.01);
                assert!((b[[y, x]] - 0.5).abs() < 0.01);
            }
        }
    }

    #[test]
    fn test_lrgb_boosts_luminance() {
        let l = Array2::from_elem((10, 10), 0.8f32);
        let mut r = Array2::from_elem((10, 10), 0.3f32);
        let mut g = Array2::from_elem((10, 10), 0.1f32);
        let mut b = Array2::from_elem((10, 10), 0.05f32);

        apply_lrgb(&l, &mut r, &mut g, &mut b, 1.0, 1.0).unwrap();

        assert!(r[[5, 5]] > 0.3);
        assert!(g[[5, 5]] > 0.1);
    }

    #[test]
    fn test_lrgb_dimension_mismatch() {
        let l = Array2::from_elem((10, 10), 0.5f32);
        let mut r = Array2::from_elem((10, 20), 0.5f32);
        let mut g = Array2::from_elem((10, 20), 0.5f32);
        let mut b = Array2::from_elem((10, 20), 0.5f32);

        assert!(apply_lrgb(&l, &mut r, &mut g, &mut b, 1.0, 1.0).is_err());
    }

    #[test]
    fn lrgb_combine_normalized_preserves_color_ratio() {
        let l = Array2::from_shape_fn((16, 16), |(y, _)| 100.0 + y as f32 * 20.0);
        let r = Array2::from_elem((16, 16), 0.4f32);
        let g = Array2::from_elem((16, 16), 0.2f32);
        let b = Array2::from_elem((16, 16), 0.1f32);

        let (rn, gn, bn) = lrgb_combine_normalized(&l, &r, &g, &b, 1.0, 1.0).unwrap();

        let (y, x) = (8, 8);
        let rv = rn[[y, x]];
        let gv = gn[[y, x]];
        let bv = bn[[y, x]];
        assert!(rv > 0.0 && gv > 0.0 && bv > 0.0);
        assert!((rv / gv - 2.0).abs() < 0.05, "R/G ratio broken: {}", rv / gv);
        assert!((gv / bv - 2.0).abs() < 0.05, "G/B ratio broken: {}", gv / bv);
    }

    #[test]
    fn lrgb_combine_normalized_follows_luminance_structure() {
        let l = Array2::from_shape_fn((16, 16), |(y, _)| y as f32);
        let r = Array2::from_elem((16, 16), 0.5f32);
        let g = Array2::from_elem((16, 16), 0.5f32);
        let b = Array2::from_elem((16, 16), 0.5f32);

        let (rn, _gn, _bn) = lrgb_combine_normalized(&l, &r, &g, &b, 1.0, 1.0).unwrap();
        assert!(
            rn[[15, 8]] > rn[[0, 8]] + 0.5,
            "luminance gradient not transferred: {} vs {}",
            rn[[15, 8]],
            rn[[0, 8]]
        );
    }

    #[test]
    fn normalize_min_max_rejects_unrepresentable_span() {
        let smallest_subnormal = f32::from_bits(1);
        let arr = Array2::from_elem((4, 4), 0.0f32);

        let err = normalize_min_max(&arr, 0.0, smallest_subnormal)
            .expect_err("a span with an infinite reciprocal must be rejected");
        assert!(err.to_string().contains("too narrow"), "unexpected error: {}", err);
    }

    #[test]
    fn normalize_min_max_still_maps_a_real_range() {
        let arr = Array2::from_shape_fn((2, 2), |(y, x)| (y * 2 + x) as f32);
        let out = normalize_min_max(&arr, 0.0, 3.0).unwrap();
        assert!((out[[0, 0]] - 0.0).abs() < 1e-6);
        assert!((out[[1, 1]] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn scale_by_max_rejects_unrepresentable_peak() {
        let arr = Array2::from_elem((4, 4), 0.0f32);
        let err = scale_by_max(&arr, f32::from_bits(1))
            .expect_err("a peak with an infinite reciprocal must be rejected");
        assert!(err.to_string().contains("too small"), "unexpected error: {}", err);
    }

    #[test]
    fn lrgb_combine_normalized_rejects_degenerate_luminance_span() {
        let mut l = Array2::from_elem((8, 8), 0.0f32);
        l[[3, 3]] = f32::from_bits(1);
        let r = Array2::from_elem((8, 8), 0.4f32);
        let g = Array2::from_elem((8, 8), 0.2f32);
        let b = Array2::from_elem((8, 8), 0.1f32);

        let err = lrgb_combine_normalized(&l, &r, &g, &b, 1.0, 1.0)
            .expect_err("a luminance plane with no representable span must not produce NaN pixels");
        assert!(err.to_string().contains("too narrow"), "unexpected error: {}", err);
    }

    #[test]
    fn test_output_clamped() {
        let l = Array2::from_elem((10, 10), 1.0f32);
        let mut r = Array2::from_elem((10, 10), 0.9f32);
        let mut g = Array2::from_elem((10, 10), 0.1f32);
        let mut b = Array2::from_elem((10, 10), 0.1f32);

        apply_lrgb(&l, &mut r, &mut g, &mut b, 1.0, 1.0).unwrap();

        for y in 0..10 {
            for x in 0..10 {
                assert!(r[[y, x]] >= 0.0 && r[[y, x]] <= 1.0);
                assert!(g[[y, x]] >= 0.0 && g[[y, x]] <= 1.0);
                assert!(b[[y, x]] >= 0.0 && b[[y, x]] <= 1.0);
            }
        }
    }
}
