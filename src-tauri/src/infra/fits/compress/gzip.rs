// FITS tile-compression support (Rice/GZIP) — contributed by Jae-Joon Lee <https://github.com/leejjoon>
//! GZIP_1 / GZIP_2 tile decompression.
//!
//! Both variants store a tile as an actual gzip stream (RFC 1952, `1f 8b`
//! magic -- verified against real astropy output; it is NOT a bare zlib/
//! deflate stream despite most other FITS compression code being zlib-only).
//! GZIP_2 additionally byte-"planes" the decompressed bytes before gzipping
//! (all pixels' byte 0, then all pixels' byte 1, ...) to improve
//! compressibility of smooth image data; this must be un-planed back to
//! natural [pixel][byte] order after decompression. Either way the final
//! result is exactly the tile's pixel bytes in big-endian FITS order, so
//! callers can feed it straight into the existing `decode_pixels_blank`.

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Read, Write};

/// Decompress a GZIP_1 or GZIP_2 tile into raw big-endian pixel bytes.
pub fn gzip_decode(data: &[u8], bytepix: usize, is_gzip2: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(data)
        .read_to_end(&mut out)
        .context("GZIP tile decompression failed")?;

    if is_gzip2 && bytepix > 1 {
        out = unshuffle_bytes(&out, bytepix);
    }

    Ok(out)
}

/// GZIP_2 stores bytes "planed": all pixels' byte 0 first, then all byte 1,
/// etc. (i.e. a transpose of the natural [pixel][byte] layout). Undo it back
/// to [pixel][byte] order.
fn unshuffle_bytes(planed: &[u8], bytepix: usize) -> Vec<u8> {
    let n = planed.len() / bytepix;
    let mut out = vec![0u8; planed.len()];
    for plane in 0..bytepix {
        let plane_start = plane * n;
        for i in 0..n {
            out[i * bytepix + plane] = planed[plane_start + i];
        }
    }
    out
}

fn shuffle_bytes(natural: &[u8], bytepix: usize) -> Vec<u8> {
    let n = natural.len() / bytepix;
    let mut out = vec![0u8; natural.len()];
    for plane in 0..bytepix {
        let plane_start = plane * n;
        for i in 0..n {
            out[plane_start + i] = natural[i * bytepix + plane];
        }
    }
    out
}

pub(crate) fn gzip2_encode(raw_be: &[u8], bytepix: usize) -> Vec<u8> {
    let planed = if bytepix > 1 {
        shuffle_bytes(raw_be, bytepix)
    } else {
        raw_be.to_vec()
    };
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&planed).expect("gzip encode into a Vec<u8> sink cannot fail");
    enc.finish().expect("gzip finish into a Vec<u8> sink cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn roundtrip_gzip1() {
        let original: Vec<u8> = (0..64u32).flat_map(|v| v.to_be_bytes()).collect();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&original).unwrap();
        let compressed = enc.finish().unwrap();

        let decoded = gzip_decode(&compressed, 4, false).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn shuffle_is_inverse_of_unshuffle() {
        let natural: Vec<u8> = (0..40u32).flat_map(|v| (v.wrapping_mul(2654435761)).to_be_bytes()).collect();
        for bytepix in [1usize, 2, 4, 8] {
            if natural.len() % bytepix != 0 {
                continue;
            }
            let planed = shuffle_bytes(&natural, bytepix);
            assert_eq!(unshuffle_bytes(&planed, bytepix), natural, "bytepix {bytepix}");
        }
    }

    #[test]
    fn gzip2_encode_roundtrips_through_decoder() {
        let raw: Vec<u8> = (0..256u32)
            .flat_map(|v| (1000.0f32 + (v as f32) * 0.25).to_be_bytes())
            .collect();
        let encoded = gzip2_encode(&raw, 4);
        let decoded = gzip_decode(&encoded, 4, true).unwrap();
        assert_eq!(decoded, raw);

        let bytes: Vec<u8> = (0..97u8).collect();
        let enc1 = gzip2_encode(&bytes, 1);
        assert_eq!(gzip_decode(&enc1, 1, true).unwrap(), bytes);
    }

    #[test]
    fn gzip2_encode_is_deterministic() {
        let raw: Vec<u8> = (0..128u32).flat_map(|v| v.to_be_bytes()).collect();
        assert_eq!(gzip2_encode(&raw, 4), gzip2_encode(&raw, 4));
    }

    #[test]
    fn unshuffle_is_inverse_of_plane_shuffle() {
        let pixels: Vec<u8> = (0..32u16).flat_map(|v| v.to_be_bytes()).collect();
        // shuffle: plane-major layout (all byte-0s, then all byte-1s)
        let n = pixels.len() / 2;
        let mut planed = vec![0u8; pixels.len()];
        for i in 0..n {
            planed[i] = pixels[i * 2];
            planed[n + i] = pixels[i * 2 + 1];
        }
        assert_eq!(unshuffle_bytes(&planed, 2), pixels);
    }

    #[test]
    fn matches_astropy_gzip2_i32_tile() {
        // A real GZIP_2-compressed tile of i32 values 0..16, captured via
        // `uv run --with astropy` (CompImageHDU(compression_type="GZIP_2")),
        // not hand-encoded. Confirms both the gzip (not zlib) container and
        // the byte-plane shuffle direction against real astropy output.
        let comp: &[u8] = &[
            31, 139, 8, 0, 143, 68, 74, 106, 2, 255, 99, 96, 32, 17, 48, 50, 49, 179, 176, 178,
            177, 115, 112, 114, 113, 243, 240, 242, 241, 3, 0, 235, 202, 248, 87, 64, 0, 0, 0,
        ];
        let decoded = gzip_decode(comp, 4, true).unwrap();
        let expected: Vec<u8> = (0..16u32).flat_map(|v| v.to_be_bytes()).collect();
        assert_eq!(decoded, expected);
    }
}
