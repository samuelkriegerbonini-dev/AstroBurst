// rice_encode — contributed by Jae-Joon Lee <https://github.com/leejjoon>

use super::rice::{sign_extend, RiceParams};

struct BitWriter {
    out: Vec<u8>,
    buf: u64,
    nbits: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self { out: Vec::new(), buf: 0, nbits: 0 }
    }

    fn push_bits(&mut self, value: u32, width: u32) {
        if width == 0 {
            return;
        }
        let mask = (1u64 << width) - 1;
        self.buf = (self.buf << width) | (value as u64 & mask);
        self.nbits += width;
        while self.nbits >= 8 {
            self.nbits -= 8;
            self.out.push(((self.buf >> self.nbits) & 0xFF) as u8);
        }
    }

    fn push_zero_bits(&mut self, mut n: u64) {
        const CHUNK: u32 = 24;
        while n >= CHUNK as u64 {
            self.push_bits(0, CHUNK);
            n -= CHUNK as u64;
        }
        if n > 0 {
            self.push_bits(0, n as u32);
        }
    }

    fn finish(self) -> Vec<u8> {
        let mut out = self.out;
        if self.nbits > 0 {
            let pad = 8 - self.nbits;
            out.push(((self.buf << pad) & 0xFF) as u8);
        }
        out
    }
}

pub fn rice_encode(pixels: &[i64], params: &RiceParams) -> Vec<u8> {
    let (fsbits, fsmax, init_bytes, width_bits): (u32, u32, usize, u32) = match params.bytepix {
        1 => (3, 6, 1, 8),
        2 => (4, 14, 2, 16),
        4 => (5, 25, 4, 32),
        other => panic!("unsupported Rice BYTEPIX {other}"),
    };
    let trunc_mask: u32 = if width_bits >= 32 {
        u32::MAX
    } else {
        (1u32 << width_bits) - 1
    };

    let nx = pixels.len();
    let mut out = Vec::new();
    if nx == 0 {
        return out;
    }

    let to_u32 = |v: i64| -> u32 { (v as u64 as u32) & trunc_mask };

    let lastpix0 = to_u32(pixels[0]);
    let be = lastpix0.to_be_bytes();
    out.extend_from_slice(&be[4 - init_bytes..]);

    let mut writer = BitWriter::new();
    let mut lastpix = lastpix0;

    let mut i = 0usize;
    while i < nx {
        let imax = (i + params.blocksize as usize).min(nx);

        let mut us: Vec<u32> = Vec::with_capacity(imax - i);
        let mut cur = lastpix;
        for &pixel in &pixels[i..imax] {
            let pu = to_u32(pixel);
            let diff_target = pu.wrapping_sub(cur) & trunc_mask;
            let d = sign_extend(diff_target, width_bits);
            let u: u32 = if d >= 0 {
                (d as u64 * 2) as u32
            } else {
                ((-d) as u64 * 2 - 1) as u32
            };
            us.push(u);
            cur = pu;
        }
        lastpix = cur;

        if us.iter().all(|&u| u == 0) {
            writer.push_bits(0, fsbits);
        } else {
            let mut best_fs = 0u32;
            let mut best_cost = u64::MAX;
            for fs in 0..fsmax {
                let cost: u64 = us
                    .iter()
                    .map(|&u| (u >> fs) as u64 + 1 + fs as u64)
                    .sum();
                if cost < best_cost {
                    best_cost = cost;
                    best_fs = fs;
                }
            }

            let verbatim_cost = us.len() as u64 * width_bits as u64;
            if verbatim_cost < best_cost {
                writer.push_bits(fsmax + 1, fsbits);
                for &u in &us {
                    writer.push_bits(u, width_bits);
                }
            } else {
                writer.push_bits(best_fs + 1, fsbits);
                for &u in &us {
                    let quotient = (u >> best_fs) as u64;
                    let remainder = u & ((1u32 << best_fs) - 1);
                    writer.push_zero_bits(quotient);
                    writer.push_bits(1, 1);
                    writer.push_bits(remainder, best_fs);
                }
            }
        }

        i = imax;
    }

    out.extend(writer.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::super::rice::rice_decode;
    use super::*;

    fn roundtrip(pixels: &[i64], params: &RiceParams) {
        let encoded = rice_encode(pixels, params);
        let decoded = rice_decode(&encoded, pixels.len(), params);
        assert_eq!(decoded, pixels, "round-trip mismatch for {pixels:?}");
    }

    #[test]
    fn roundtrip_i16_small_tile() {
        roundtrip(
            &[5, 5, 5, 5, 100, -100, 0, 7],
            &RiceParams { blocksize: 32, bytepix: 2, signed: true },
        );
    }

    #[test]
    fn roundtrip_byte_tile_unsigned() {
        let pixels: Vec<i64> = vec![
            31, 14, 6, 36, 31, 255, 255, 255, 255, 54, 41, 59, 63, 48, 71, 50, 51, 72, 73, 56, 87,
            64, 92, 63, 97, 100, 103, 101, 96, 109, 95, 118, 112, 111, 116, 121, 103, 110, 117,
            115,
        ];
        roundtrip(&pixels, &RiceParams { blocksize: 32, bytepix: 1, signed: false });
    }

    #[test]
    fn roundtrip_constant_block() {
        roundtrip(&[42; 64], &RiceParams { blocksize: 32, bytepix: 2, signed: true });
    }

    #[test]
    fn roundtrip_high_entropy_block() {
        let pixels: Vec<i64> = (0..40)
            .map(|i| if i % 2 == 0 { 32000 } else { -32000 })
            .collect();
        roundtrip(&pixels, &RiceParams { blocksize: 32, bytepix: 2, signed: true });
    }

    #[test]
    fn roundtrip_int32_wide() {
        let pixels: Vec<i64> = (0..50).map(|i| (i * i - 500) as i64).collect();
        roundtrip(&pixels, &RiceParams { blocksize: 32, bytepix: 4, signed: true });
    }

    #[test]
    fn roundtrip_int32_beyond_f32_mantissa() {
        let base = 1_073_741_824i64;
        let pixels: Vec<i64> = (0..40)
            .map(|i| if i % 2 == 0 { base + i } else { -base - i })
            .collect();
        roundtrip(&pixels, &RiceParams { blocksize: 32, bytepix: 4, signed: true });
    }

    #[test]
    fn roundtrip_non_block_aligned_width() {
        let pixels: Vec<i64> = (0..37).map(|i| (i % 13) - 6).collect();
        roundtrip(&pixels, &RiceParams { blocksize: 32, bytepix: 2, signed: true });
    }

    #[test]
    fn roundtrip_single_pixel() {
        roundtrip(&[123], &RiceParams { blocksize: 32, bytepix: 4, signed: true });
    }

    #[test]
    fn roundtrip_random_stream() {
        let mut state: u32 = 0x9E3779B9;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let pixels: Vec<i64> = (0..500)
            .map(|_| (next() as i32 % 20000) as i64)
            .collect();
        roundtrip(&pixels, &RiceParams { blocksize: 32, bytepix: 4, signed: true });
    }

    #[test]
    fn roundtrip_wide_high_entropy_bytepix4_hits_zero_leftover_bits() {
        let mut state: u32 = 0xDEADBEEF;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let pixels: Vec<i64> = (0..2040).map(|_| next() as i32 as i64).collect();
        roundtrip(&pixels, &RiceParams { blocksize: 32, bytepix: 4, signed: true });
    }
}
