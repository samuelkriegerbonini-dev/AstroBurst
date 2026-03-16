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
            "Dimensões L {:?} não correspondem ao RGB (R: {:?}, G: {:?}, B: {:?})",
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

pub fn synthesize_luminance(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
) -> Array2<f32> {
    let mut lum = Array2::zeros(r.raw_dim());

    Zip::from(&mut lum)
        .and(r)
        .and(g)
        .and(b)
        .par_for_each(|l_out, &rv, &gv, &bv| {
            *l_out = rv * 0.2126 + gv * 0.7152 + bv * 0.0722;
        });

    lum
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
    fn test_synthesize_luminance() {
        let r = Array2::from_elem((10, 10), 1.0f32);
        let g = Array2::from_elem((10, 10), 1.0f32);
        let b = Array2::from_elem((10, 10), 1.0f32);

        let lum = synthesize_luminance(&r, &g, &b);
        assert!((lum[[5, 5]] - 1.0).abs() < 0.001);
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
