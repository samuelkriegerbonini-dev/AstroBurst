// quantize — contributed by Jae-Joon Lee <https://github.com/leejjoon>

use std::sync::OnceLock;

pub const NULL_VALUE: i64 = -2147483647;
const N_RESERVED_VALUES: f64 = 10.0;
const N_RANDOM: usize = 10000;

fn rand_table() -> &'static [f32; N_RANDOM] {
    static TABLE: OnceLock<[f32; N_RANDOM]> = OnceLock::new();
    TABLE.get_or_init(|| {
        const A: f64 = 16807.0;
        const M: f64 = 2147483647.0;
        let mut seed = 1.0f64;
        let mut table = [0f32; N_RANDOM];
        for slot in table.iter_mut() {
            let temp = A * seed;
            seed = temp - M * ((temp / M) as i64 as f64);
            *slot = (seed / M) as f32;
        }
        debug_assert_eq!(
            seed as i64, 1043618065,
            "fits_init_randoms self-check failed -- RNG port does not match cfitsio"
        );
        table
    })
}

fn nint(x: f64) -> i64 {
    if x >= 0.0 {
        (x + 0.5) as i64
    } else {
        (x - 0.5) as i64
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileQuant {
    pub scale: f64,
    pub zero: f64,
}

pub enum QuantizeResult {
    Ints { values: Vec<i64>, quant: TileQuant, has_null: bool },
    Overflow,
}

fn estimate_noise(row: &[f32]) -> Option<(f32, f32, f64)> {
    let good: Vec<f32> = row.iter().copied().filter(|v| v.is_finite()).collect();
    if good.len() < 9 {
        return None;
    }
    let minval = good.iter().copied().fold(f32::INFINITY, f32::min);
    let maxval = good.iter().copied().fold(f32::NEG_INFINITY, f32::max);

    let mut differences2 = Vec::with_capacity(good.len());
    let mut differences3 = Vec::with_capacity(good.len());
    let mut differences5 = Vec::with_capacity(good.len());

    for w in good.windows(9) {
        let (v1, v3, v5, v7, v9) =
            (w[0] as f64, w[2] as f64, w[4] as f64, w[6] as f64, w[8] as f64);
        if !(w[4] == w[5] && w[5] == w[6]) {
            differences2.push((v5 - v7).abs());
        }
        if !(w[2] == w[3] && w[3] == w[4] && w[4] == w[5] && w[5] == w[6]) {
            differences3.push((2.0 * v5 - v3 - v7).abs());
            differences5.push((6.0 * v5 - 4.0 * v3 - 4.0 * v7 + v1 + v9).abs());
        }
    }

    fn lower_median(mut v: Vec<f64>) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.total_cmp(b));
        v[(v.len() - 1) / 2]
    }

    let noise2 = 1.0483579 * lower_median(differences2);
    let noise3 = 0.6052697 * lower_median(differences3);
    let noise5 = 0.1772048 * lower_median(differences5);

    let mut stdev = noise3;
    if noise2 != 0.0 && noise2 < stdev {
        stdev = noise2;
    }
    if noise5 != 0.0 && noise5 < stdev {
        stdev = noise5;
    }

    Some((minval, maxval, stdev))
}

pub(crate) struct TileDither {
    table: &'static [f32; N_RANDOM],
    iseed: usize,
    nextrand: usize,
}

impl TileDither {
    pub(crate) fn new(dither_seed: i32, tile_index: usize) -> Self {
        let irow = tile_index as i64 + 1 + dither_seed as i64 - 1;
        let table = rand_table();
        let iseed = ((irow - 1).rem_euclid(N_RANDOM as i64)) as usize;
        let nextrand = (table[iseed] * 500.0) as usize;
        Self { table, iseed, nextrand }
    }

    pub(crate) fn next(&mut self) -> f64 {
        let r = self.table[self.nextrand] as f64;
        self.nextrand += 1;
        if self.nextrand == N_RANDOM {
            self.iseed += 1;
            if self.iseed == N_RANDOM {
                self.iseed = 0;
            }
            self.nextrand = (self.table[self.iseed] * 500.0) as usize;
        }
        r
    }
}

pub fn quantize_tile(
    pixels: &[f32],
    quantize_level: f64,
    dither_seed: i32,
    tile_index: usize,
) -> QuantizeResult {
    let nx = pixels.len();
    if nx <= 1 || !(quantize_level.is_finite() && quantize_level > 0.0) {
        return QuantizeResult::Overflow;
    }

    let has_nan = pixels.iter().any(|v| !v.is_finite());

    let Some((minval, maxval, stdev)) = estimate_noise(pixels) else {
        return QuantizeResult::Overflow;
    };
    if stdev == 0.0 {
        return QuantizeResult::Overflow;
    }

    let delta = stdev / quantize_level;
    if !(delta.is_finite() && delta > 0.0) {
        return QuantizeResult::Overflow;
    }

    let minval = minval as f64;
    let maxval = maxval as f64;

    if (maxval - minval) / delta > 2.0 * 2147483647.0 - N_RESERVED_VALUES {
        return QuantizeResult::Overflow;
    }

    let ngood = pixels.iter().filter(|v| v.is_finite()).count();
    let zeropt = if ngood != nx {
        minval - delta * (NULL_VALUE as f64 + N_RESERVED_VALUES)
    } else if (maxval - minval) / delta < 2147483647.0 - N_RESERVED_VALUES {
        let iqfactor = (minval / delta + 0.5) as i64;
        iqfactor as f64 * delta
    } else {
        (minval + maxval) / 2.0
    };
    if !zeropt.is_finite() {
        return QuantizeResult::Overflow;
    }

    let mut dither = TileDither::new(dither_seed, tile_index);

    let mut values = Vec::with_capacity(nx);
    for &pixel in pixels {
        let r = dither.next();
        if !pixel.is_finite() {
            values.push(NULL_VALUE);
        } else {
            let y = (pixel as f64 - zeropt) / delta + r - 0.5;
            values.push(nint(y));
        }
    }

    if values.iter().any(|&v| v != NULL_VALUE && (v > i32::MAX as i64 || v < i32::MIN as i64)) {
        return QuantizeResult::Overflow;
    }

    QuantizeResult::Ints {
        values,
        quant: TileQuant { scale: delta, zero: zeropt },
        has_null: has_nan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_table_matches_cfitsio_selfcheck() {
        let table = rand_table();
        assert!(table[0] > 0.0 && table[0] < 1.0);
    }

    #[test]
    fn constant_tile_falls_back_to_lossless_storage() {
        for value in [3.5f32, 0.0, 65535.0] {
            let pixels = vec![value; 40];
            assert!(
                matches!(quantize_tile(&pixels, 16.0, 1, 0), QuantizeResult::Overflow),
                "constant tile of {value} must not be quantized (dithered decode would add noise)"
            );
        }
    }

    #[test]
    fn short_constant_tile_falls_back_to_lossless_storage() {
        let pixels = vec![7.0f32; 4];
        assert!(matches!(quantize_tile(&pixels, 16.0, 1, 0), QuantizeResult::Overflow));
    }

    #[test]
    fn smooth_gradient_quantizes_within_tolerance() {
        let pixels: Vec<f32> = (0..64).map(|i| 100.0 + (i as f32) * 0.01).collect();
        match quantize_tile(&pixels, 16.0, 1, 5) {
            QuantizeResult::Ints { values, quant, .. } => {
                for (orig, &v) in pixels.iter().zip(values.iter()) {
                    let reconstructed = v as f64 * quant.scale + quant.zero;
                    assert!(
                        (reconstructed - *orig as f64).abs() < quant.scale.abs() * 2.0 + 1e-6,
                        "orig={orig} reconstructed={reconstructed} scale={}",
                        quant.scale
                    );
                }
            }
            QuantizeResult::Overflow => panic!("expected smooth tile to quantize"),
        }
    }

    #[test]
    fn nan_pixels_become_null_value_and_flag_has_null() {
        let mut pixels: Vec<f32> = (0..40).map(|i| (i as f32).sin() * 100.0).collect();
        pixels[10] = f32::NAN;
        match quantize_tile(&pixels, 16.0, 1, 2) {
            QuantizeResult::Ints { values, has_null, .. } => {
                assert!(has_null);
                assert_eq!(values[10], NULL_VALUE);
                assert!(values.iter().enumerate().all(|(i, &v)| i == 10 || v != NULL_VALUE));
            }
            QuantizeResult::Overflow => panic!("expected mostly-finite tile to quantize"),
        }
    }

    #[test]
    fn a_level_that_is_not_positive_and_finite_stores_the_tile_losslessly() {
        let pixels: Vec<f32> = (0..64).map(|i| 50.0 + (i as f32 * 0.37).sin() * 5.0).collect();
        for level in [0.0, -0.0, -4.0, f64::NAN, f64::INFINITY, f64::MIN_POSITIVE * 1e-10] {
            assert!(
                matches!(quantize_tile(&pixels, level, 1, 0), QuantizeResult::Overflow),
                "quantize_level {level} must not produce a quantized tile"
            );
        }
        let mut with_null = pixels.clone();
        with_null[3] = f32::NAN;
        assert!(matches!(quantize_tile(&with_null, 0.0, 1, 0), QuantizeResult::Overflow));
    }

    #[test]
    fn too_short_non_constant_row_falls_back() {
        let pixels: Vec<f32> = vec![1.0, 2.0, 1.5, 3.0, 2.5];
        assert!(matches!(quantize_tile(&pixels, 16.0, 1, 0), QuantizeResult::Overflow));
    }

    #[test]
    fn dither_index_advances_deterministically_across_tiles() {
        let pixels: Vec<f32> = (0..64).map(|i| 50.0 + (i as f32 * 0.37).sin() * 5.0).collect();
        let a = quantize_tile(&pixels, 16.0, 1, 0);
        let b = quantize_tile(&pixels, 16.0, 1, 1);
        let (QuantizeResult::Ints { values: va, .. }, QuantizeResult::Ints { values: vb, .. }) =
            (a, b)
        else {
            panic!("expected both to quantize");
        };
        assert_ne!(va, vb, "different tile_index should (almost always) dither differently");
    }
}
