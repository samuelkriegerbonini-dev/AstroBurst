// FITS tile-compression support (Rice/GZIP) — contributed by Jae-Joon Lee <https://github.com/leejjoon>
//! RICE_1 tile decoder.
//!
//! Line-for-line port of cfitsio's `fits_rdecomp` / `fits_rdecomp_short` /
//! `fits_rdecomp_byte` (ricecomp.c, R. White / STScI, public domain:
//! https://github.com/HEASARC/cfitsio/blob/develop/ricecomp.c), unified into
//! one function parameterized by BYTEPIX. Verified bit-exact against
//! astropy's own RICE_1 decompression across BYTEPIX 1/2/4, low- and
//! high-entropy blocks, all-zero blocks, and non-block-aligned tile widths.

pub struct RiceParams {
    pub blocksize: u32, // ZVAL (BLOCKSIZE), default 32
    pub bytepix: u32,   // ZVAL (BYTEPIX): 1, 2, or 4
    /// FITS's BITPIX convention makes 8-bit pixels the one native *unsigned*
    /// integer type (0..255); BITPIX 16/32/64 are signed two's complement.
    /// This is a property of ZBITPIX, not of `bytepix` -- cfitsio's C decoder
    /// does all arithmetic in `unsigned int` regardless of pixel width, so
    /// getting this wrong silently wraps every negative/high pixel by 256 in
    /// the byte case (caught during prototyping against real fixtures).
    pub signed: bool,
}

fn nonzero_count(b: u8) -> u32 {
    if b == 0 {
        0
    } else {
        8 - b.leading_zeros()
    }
}

/// Decode `nx` pixels from one Rice-coded tile's compressed byte stream.
/// Returns pixel values as i64 (sign-extended per `params.signed`), i.e. the
/// raw stored integers *before* BSCALE/BZERO or ZSCALE/ZZERO are applied.
pub fn rice_decode(data: &[u8], nx: usize, params: &RiceParams) -> Vec<i64> {
    let (fsbits, fsmax, init_bytes, width_bits): (u32, u32, usize, u32) = match params.bytepix {
        1 => (3, 6, 1, 8),
        2 => (4, 14, 2, 16),
        4 => (5, 25, 4, 32),
        other => panic!("unsupported Rice BYTEPIX {other}"),
    };
    let bbits = 1u32 << fsbits;
    let trunc_mask: u32 = if width_bits >= 32 {
        u32::MAX
    } else {
        (1u32 << width_bits) - 1
    };

    let mut pos = 0usize;
    let mut lastpix: u32 = 0;
    for _ in 0..init_bytes {
        lastpix = (lastpix << 8) | data[pos] as u32;
        pos += 1;
    }

    let mut b: u32 = data[pos] as u32;
    pos += 1;
    let mut nbits: i32 = 8;

    let mut out: Vec<u32> = Vec::with_capacity(nx);
    let mut i = 0usize;
    while i < nx {
        nbits -= fsbits as i32;
        while nbits < 0 {
            b = (b << 8) | data[pos] as u32;
            pos += 1;
            nbits += 8;
        }
        let fs: i32 = (b >> nbits as u32) as i32 - 1;
        b &= (1u32 << nbits as u32) - 1;

        let imax = (i + params.blocksize as usize).min(nx);

        if fs < 0 {
            // low-entropy: all-zero differences within this block
            while i < imax {
                out.push(lastpix);
                i += 1;
            }
        } else if fs as u32 == fsmax {
            // high-entropy: pixels coded verbatim (no Rice split)
            while i < imax {
                let mut k = bbits as i32 - nbits;
                let mut diff: u32 = b << k;
                k -= 8;
                while k >= 0 {
                    b = data[pos] as u32;
                    pos += 1;
                    diff |= b << k;
                    k -= 8;
                }
                if nbits > 0 {
                    b = data[pos] as u32;
                    pos += 1;
                    diff |= b >> (-k);
                    b &= (1u32 << nbits as u32) - 1;
                } else {
                    b = 0;
                }
                diff = if diff & 1 == 0 {
                    diff >> 1
                } else {
                    !(diff >> 1)
                };
                let val = diff.wrapping_add(lastpix) & trunc_mask;
                out.push(val);
                lastpix = val;
                i += 1;
            }
        } else {
            let fs = fs as u32;
            while i < imax {
                while b == 0 {
                    nbits += 8;
                    b = data[pos] as u32;
                    pos += 1;
                }
                let nzero = nbits - nonzero_count(b as u8) as i32;
                nbits -= nzero + 1;
                b ^= 1u32 << nbits as u32;
                nbits -= fs as i32;
                while nbits < 0 {
                    b = (b << 8) | data[pos] as u32;
                    pos += 1;
                    nbits += 8;
                }
                let mut diff = (nzero as u32) << fs | (b >> nbits as u32);
                b &= (1u32 << nbits as u32) - 1;
                diff = if diff & 1 == 0 {
                    diff >> 1
                } else {
                    !(diff >> 1)
                };
                let val = diff.wrapping_add(lastpix) & trunc_mask;
                out.push(val);
                lastpix = val;
                i += 1;
            }
        }
    }

    if params.signed {
        out.into_iter()
            .map(|v| sign_extend(v, width_bits))
            .collect()
    } else {
        out.into_iter().map(|v| v as i64).collect()
    }
}

fn sign_extend(v: u32, bits: u32) -> i64 {
    if bits >= 32 {
        return v as i32 as i64;
    }
    let shift = 32 - bits;
    (((v << shift) as i32) >> shift) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression fixtures generated via `uv run --with astropy` against real
    // RICE_1-compressed FITS files; the compressed byte streams and expected
    // decoded pixels below are copied verbatim from that ground truth so this
    // test doesn't depend on astropy being available at build/test time.

    #[test]
    fn matches_astropy_i16_single_row() {
        // rice_i16_tiny fixture: [5, 5, 5, 5, 100, -100, 0, 7], BLOCKSIZE=32, BYTEPIX=2.
        // Bytes captured directly from an astropy CompImageHDU(compression_type="RICE_1")
        // output via `uv run --with astropy` -- not hand-encoded.
        let tile: &[u8] = &[0, 5, 120, 16, 32, 64, 63, 1, 60, 72, 156];
        let params = RiceParams {
            blocksize: 32,
            bytepix: 2,
            signed: true,
        };
        let decoded = rice_decode(tile, 8, &params);
        assert_eq!(decoded, vec![5, 5, 5, 5, 100, -100, 0, 7]);
    }

    #[test]
    fn byte_width_stays_unsigned() {
        // A byte tile whose difference stream saturates at 255 must stay in 0..255,
        // not get reinterpreted as a signed -128..127 value (the bug found during
        // prototyping: sign-extension must be gated on BITPIX==8 being unsigned).
        // Bytes + expected values captured from a real astropy RICE_1 uint8 fixture
        // (row containing a deliberate 255-saturation run).
        let tile: &[u8] = &[
            31, 176, 35, 241, 204, 143, 194, 16, 3, 204, 148, 195, 167, 140, 200, 212, 136, 143,
            29, 24, 25, 10, 90, 211, 203, 77, 158, 139, 149, 40, 45, 206, 176,
        ];
        let params = RiceParams {
            blocksize: 32,
            bytepix: 1,
            signed: false,
        };
        let decoded = rice_decode(tile, 40, &params);
        let expected: Vec<i64> = vec![
            31, 14, 6, 36, 31, 255, 255, 255, 255, 54, 41, 59, 63, 48, 71, 50, 51, 72, 73, 56, 87,
            64, 92, 63, 97, 100, 103, 101, 96, 109, 95, 118, 112, 111, 116, 121, 103, 110, 117,
            115,
        ];
        assert_eq!(decoded, expected);
    }
}
