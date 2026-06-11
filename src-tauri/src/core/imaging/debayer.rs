use ndarray::Array2;
use rayon::prelude::*;

use crate::types::header::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum BayerPattern {
    Rggb,
    Bggr,
    Grbg,
    Gbrg,
}

impl BayerPattern {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().trim_matches('\'').trim().to_uppercase().as_str() {
            "RGGB" => Some(Self::Rggb),
            "BGGR" => Some(Self::Bggr),
            "GRBG" => Some(Self::Grbg),
            "GBRG" => Some(Self::Gbrg),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Rggb => "RGGB",
            Self::Bggr => "BGGR",
            Self::Grbg => "GRBG",
            Self::Gbrg => "GBRG",
        }
    }

    #[inline]
    fn cell(&self) -> [[u8; 2]; 2] {
        match self {
            Self::Rggb => [[0, 1], [1, 2]],
            Self::Bggr => [[2, 1], [1, 0]],
            Self::Grbg => [[1, 0], [2, 1]],
            Self::Gbrg => [[1, 2], [0, 1]],
        }
    }

    fn from_cell(cell: [[u8; 2]; 2]) -> Option<Self> {
        [Self::Rggb, Self::Bggr, Self::Grbg, Self::Gbrg]
            .into_iter()
            .find(|p| p.cell() == cell)
    }

    pub fn with_offsets(&self, xoff: i64, yoff: i64) -> Self {
        let base = self.cell();
        let mut cell = [[0u8; 2]; 2];
        for (y, row) in cell.iter_mut().enumerate() {
            for (x, out) in row.iter_mut().enumerate() {
                let sy = (y as i64 + yoff).rem_euclid(2) as usize;
                let sx = (x as i64 + xoff).rem_euclid(2) as usize;
                *out = base[sy][sx];
            }
        }
        Self::from_cell(cell).unwrap_or(*self)
    }

    pub fn detect(header: &HduHeader) -> Option<Self> {
        let raw = header.get("BAYERPAT").or_else(|| header.get("COLORTYP"))?;
        let base = Self::parse(raw)?;
        let xoff = header.get_f64("XBAYROFF").unwrap_or(0.0).round() as i64;
        let yoff = header.get_f64("YBAYROFF").unwrap_or(0.0).round() as i64;
        Some(base.with_offsets(xoff, yoff))
    }
}

pub fn debayer_bilinear(
    img: &Array2<f32>,
    pattern: BayerPattern,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let (h, w) = img.dim();
    let cell = pattern.cell();
    let src = img.as_slice().expect("contiguous");

    let mut r = vec![f32::NAN; h * w];
    let mut g = vec![f32::NAN; h * w];
    let mut b = vec![f32::NAN; h * w];

    r.par_chunks_mut(w)
        .zip(g.par_chunks_mut(w))
        .zip(b.par_chunks_mut(w))
        .enumerate()
        .for_each(|(y, ((row_r, row_g), row_b))| {
            let y0 = y.saturating_sub(1);
            let y1 = (y + 1).min(h - 1);
            for x in 0..w {
                let x0 = x.saturating_sub(1);
                let x1 = (x + 1).min(w - 1);

                let mut sums = [0.0f64; 3];
                let mut counts = [0u32; 3];
                for ny in y0..=y1 {
                    let base = ny * w;
                    for nx in x0..=x1 {
                        let v = src[base + nx];
                        if v.is_finite() {
                            let c = cell[ny & 1][nx & 1] as usize;
                            sums[c] += v as f64;
                            counts[c] += 1;
                        }
                    }
                }

                let own = cell[y & 1][x & 1] as usize;
                let center = src[y * w + x];
                let assign = |c: usize| -> f32 {
                    if c == own && center.is_finite() {
                        center
                    } else if counts[c] > 0 {
                        (sums[c] / counts[c] as f64) as f32
                    } else {
                        f32::NAN
                    }
                };

                row_r[x] = assign(0);
                row_g[x] = assign(1);
                row_b[x] = assign(2);
            }
        });

    (
        Array2::from_shape_vec((h, w), r).unwrap(),
        Array2::from_shape_vec((h, w), g).unwrap(),
        Array2::from_shape_vec((h, w), b).unwrap(),
    )
}

pub fn debayer_superpixel(
    img: &Array2<f32>,
    pattern: BayerPattern,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let (h, w) = img.dim();
    let oh = h / 2;
    let ow = w / 2;
    let cell = pattern.cell();
    let src = img.as_slice().expect("contiguous");

    let mut r = vec![f32::NAN; oh * ow];
    let mut g = vec![f32::NAN; oh * ow];
    let mut b = vec![f32::NAN; oh * ow];

    r.par_chunks_mut(ow)
        .zip(g.par_chunks_mut(ow))
        .zip(b.par_chunks_mut(ow))
        .enumerate()
        .for_each(|(cy, ((row_r, row_g), row_b))| {
            let sy = cy * 2;
            for cx in 0..ow {
                let sx = cx * 2;
                let mut sums = [0.0f64; 3];
                let mut counts = [0u32; 3];
                for dy in 0..2usize {
                    for dx in 0..2usize {
                        let v = src[(sy + dy) * w + sx + dx];
                        if v.is_finite() {
                            let c = cell[dy][dx] as usize;
                            sums[c] += v as f64;
                            counts[c] += 1;
                        }
                    }
                }
                let pick = |c: usize| -> f32 {
                    if counts[c] > 0 {
                        (sums[c] / counts[c] as f64) as f32
                    } else {
                        f32::NAN
                    }
                };
                row_r[cx] = pick(0);
                row_g[cx] = pick(1);
                row_b[cx] = pick(2);
            }
        });

    (
        Array2::from_shape_vec((oh, ow), r).unwrap(),
        Array2::from_shape_vec((oh, ow), g).unwrap(),
        Array2::from_shape_vec((oh, ow), b).unwrap(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut index = HashMap::new();
        let mut cards = Vec::new();
        for &(k, v) in pairs {
            index.insert(k.to_string(), v.to_string());
            cards.push((k.to_string(), v.to_string()));
        }
        HduHeader { cards, index }
    }

    fn mosaic(h: usize, w: usize, pattern: BayerPattern, rv: f32, gv: f32, bv: f32) -> Array2<f32> {
        let cell = pattern.cell();
        Array2::from_shape_fn((h, w), |(y, x)| match cell[y & 1][x & 1] {
            0 => rv,
            1 => gv,
            _ => bv,
        })
    }

    #[test]
    fn test_parse_variants() {
        assert_eq!(BayerPattern::parse("RGGB"), Some(BayerPattern::Rggb));
        assert_eq!(BayerPattern::parse("'bggr '"), Some(BayerPattern::Bggr));
        assert_eq!(BayerPattern::parse(" GrBg"), Some(BayerPattern::Grbg));
        assert_eq!(BayerPattern::parse("XTRANS"), None);
    }

    #[test]
    fn test_with_offsets() {
        assert_eq!(BayerPattern::Rggb.with_offsets(0, 0), BayerPattern::Rggb);
        assert_eq!(BayerPattern::Rggb.with_offsets(1, 0), BayerPattern::Grbg);
        assert_eq!(BayerPattern::Rggb.with_offsets(0, 1), BayerPattern::Gbrg);
        assert_eq!(BayerPattern::Rggb.with_offsets(1, 1), BayerPattern::Bggr);
        assert_eq!(BayerPattern::Rggb.with_offsets(2, 2), BayerPattern::Rggb);
    }

    #[test]
    fn test_detect_from_header() {
        let h = make_header(&[("BAYERPAT", "'RGGB'")]);
        assert_eq!(BayerPattern::detect(&h), Some(BayerPattern::Rggb));

        let h2 = make_header(&[("BAYERPAT", "RGGB"), ("XBAYROFF", "1"), ("YBAYROFF", "1")]);
        assert_eq!(BayerPattern::detect(&h2), Some(BayerPattern::Bggr));

        let h3 = make_header(&[("OBJECT", "M42")]);
        assert_eq!(BayerPattern::detect(&h3), None);
    }

    #[test]
    fn test_bilinear_reconstructs_constants_all_patterns() {
        for pattern in [
            BayerPattern::Rggb,
            BayerPattern::Bggr,
            BayerPattern::Grbg,
            BayerPattern::Gbrg,
        ] {
            let cfa = mosaic(16, 16, pattern, 0.8, 0.5, 0.2);
            let (r, g, b) = debayer_bilinear(&cfa, pattern);
            for y in 0..16 {
                for x in 0..16 {
                    assert!((r[[y, x]] - 0.8).abs() < 1e-6, "{:?} R at {},{}", pattern, y, x);
                    assert!((g[[y, x]] - 0.5).abs() < 1e-6, "{:?} G at {},{}", pattern, y, x);
                    assert!((b[[y, x]] - 0.2).abs() < 1e-6, "{:?} B at {},{}", pattern, y, x);
                }
            }
        }
    }

    #[test]
    fn test_bilinear_linear_gradient_interior_exact() {
        let pattern = BayerPattern::Rggb;
        let h = 16;
        let w = 16;
        let truth = |x: usize| x as f32 / 15.0;
        let cfa = Array2::from_shape_fn((h, w), |(_, x)| truth(x));
        let (r, g, b) = debayer_bilinear(&cfa, pattern);
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                for ch in [&r, &g, &b] {
                    assert!(
                        (ch[[y, x]] - truth(x)).abs() < 1e-5,
                        "gradient mismatch at {},{}: {} vs {}",
                        y,
                        x,
                        ch[[y, x]],
                        truth(x)
                    );
                }
            }
        }
    }

    #[test]
    fn test_bilinear_nan_center_isolated_to_own_channel() {
        let pattern = BayerPattern::Rggb;
        let mut cfa = mosaic(8, 8, pattern, 0.8, 0.5, 0.2);
        cfa[[2, 2]] = f32::NAN;
        let (r, g, b) = debayer_bilinear(&cfa, pattern);
        assert!(r[[2, 2]].is_nan());
        assert!((g[[2, 2]] - 0.5).abs() < 1e-6);
        assert!((b[[2, 2]] - 0.2).abs() < 1e-6);
        assert!((r[[2, 3]] - 0.8).abs() < 1e-6);
        assert!((r[[3, 3]] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_superpixel_dims_and_values() {
        for pattern in [
            BayerPattern::Rggb,
            BayerPattern::Bggr,
            BayerPattern::Grbg,
            BayerPattern::Gbrg,
        ] {
            let cfa = mosaic(17, 15, pattern, 0.8, 0.5, 0.2);
            let (r, g, b) = debayer_superpixel(&cfa, pattern);
            assert_eq!(r.dim(), (8, 7));
            for &(ch, expect) in &[(&r, 0.8f32), (&g, 0.5), (&b, 0.2)] {
                for &v in ch.iter() {
                    assert!((v - expect).abs() < 1e-6, "{:?}", pattern);
                }
            }
        }
    }
}
