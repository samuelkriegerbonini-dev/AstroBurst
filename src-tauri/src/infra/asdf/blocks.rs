use std::borrow::Cow;
use std::io::Read;

use super::parser::AsdfError;
use crate::core::stacking::memory::{effective_memory, MemoryInfo};

#[derive(Debug)]
pub struct BlockHeader {
    pub header_size: u16,
    pub flags: u32,
    pub compression: Compression,
    pub allocated_size: u64,
    pub used_size: u64,
    pub data_size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Compression {
    None,
    Zlib,
    Bzip2,
    Lz4,
    Unknown(String),
}

#[derive(Debug)]
pub struct BlockRef {
    pub header: BlockHeader,
    pub data_start: usize,
    pub used_size: usize,
}

impl BlockHeader {
    pub const MAGIC: &'static [u8] = &[0xd3, 0x42, 0x4c, 0x4b];

    pub fn parse(buf: &[u8]) -> Result<(Self, usize), AsdfError> {
        if buf.len() < 6 || &buf[0..4] != Self::MAGIC {
            return Err(AsdfError::InvalidBlockHeader);
        }

        let header_size = u16::from_be_bytes([buf[4], buf[5]]);
        let total_header = 6 + header_size as usize;

        if buf.len() < total_header {
            return Err(AsdfError::InvalidBlockHeader);
        }

        let h = &buf[6..total_header];
        if h.len() < 48 {
            return Err(AsdfError::InvalidBlockHeader);
        }

        let flags = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
        let compression = Self::parse_compression(&h[4..8]);
        let allocated_size = u64::from_be_bytes(h[8..16].try_into().expect("8 bytes"));
        let used_size = u64::from_be_bytes(h[16..24].try_into().expect("8 bytes"));
        let data_size = u64::from_be_bytes(h[24..32].try_into().expect("8 bytes"));

        Ok((
            Self {
                header_size,
                flags,
                compression,
                allocated_size,
                used_size,
                data_size,
            },
            total_header,
        ))
    }

    fn parse_compression(bytes: &[u8]) -> Compression {
        let s: Vec<u8> = bytes.iter().copied().take_while(|&b| b != 0).collect();
        match s.as_slice() {
            [] => Compression::None,
            b"zlib" => Compression::Zlib,
            b"bzp2" => Compression::Bzip2,
            b"lz4" => Compression::Lz4,
            other => Compression::Unknown(String::from_utf8_lossy(other).to_string()),
        }
    }

    pub fn decompress<'a>(&self, raw: &'a [u8]) -> Result<Cow<'a, [u8]>, AsdfError> {
        self.decompress_within(raw, effective_memory())
    }

    pub(crate) fn decompress_within<'a>(
        &self,
        raw: &'a [u8],
        memory: Option<MemoryInfo>,
    ) -> Result<Cow<'a, [u8]>, AsdfError> {
        let name = match &self.compression {
            Compression::None => return Ok(Cow::Borrowed(raw)),
            Compression::Unknown(name) => return Err(AsdfError::UnsupportedCompression(name.clone())),
            Compression::Zlib => "zlib",
            Compression::Bzip2 => "bzip2",
            Compression::Lz4 => "lz4",
        };
        let expected = usize::try_from(self.data_size)
            .ok()
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .ok_or_else(|| AsdfError::ShapeOverflow(format!("{name} block declares data_size {}", self.data_size)))?;
        check_memory(|| format!("Decompressing a {name} block"), self.data_size, memory)?;
        match &self.compression {
            Compression::Zlib => read_declared(flate2::read::ZlibDecoder::new(raw), expected, name).map(Cow::Owned),

            #[cfg(feature = "asdf-full")]
            Compression::Bzip2 => read_declared(bzip2::read::BzDecoder::new(raw), expected, name).map(Cow::Owned),

            #[cfg(feature = "asdf-full")]
            Compression::Lz4 => decompress_lz4_asdf(raw, expected).map(Cow::Owned),

            _ => Err(AsdfError::UnsupportedCompression(format!("{name} (enable 'asdf-full' feature)"))),
        }
    }
}

pub(crate) fn check_memory(
    what: impl FnOnce() -> String,
    needed: u64,
    memory: Option<MemoryInfo>,
) -> Result<(), AsdfError> {
    let available = memory.map_or(isize::MAX as u64, |info| info.commit_available);
    if needed > available {
        return Err(AsdfError::TooLarge { what: what(), needed, available });
    }
    Ok(())
}

fn read_declared(decoder: impl Read, expected: usize, name: &str) -> Result<Vec<u8>, AsdfError> {
    let mut out = Vec::new();
    decoder
        .take(expected as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| AsdfError::DecompressionFailed(e.to_string()))?;
    if out.len() != expected {
        let got = if out.len() > expected {
            format!("more than {expected} bytes")
        } else {
            format!("{} bytes", out.len())
        };
        return Err(AsdfError::DecompressionFailed(format!(
            "{name} output is {got} but the block header declares data_size {expected}"
        )));
    }
    Ok(out)
}

#[cfg(feature = "asdf-full")]
fn decompress_lz4_asdf(payload: &[u8], uncompressed_size: usize) -> Result<Vec<u8>, AsdfError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= payload.len() && out.len() < uncompressed_size {
        let chunk_len = u32::from_be_bytes([
            payload[pos],
            payload[pos + 1],
            payload[pos + 2],
            payload[pos + 3],
        ]) as usize;
        pos += 4;
        let end = pos
            .checked_add(chunk_len)
            .ok_or_else(|| AsdfError::DecompressionFailed("lz4 length prefix overflow".into()))?;
        if end > payload.len() {
            return Err(AsdfError::DecompressionFailed(
                "lz4 length prefix exceeds payload".into(),
            ));
        }
        let chunk = &payload[pos..end];
        let claimed = match chunk.get(..4) {
            Some(prefix) => u32::from_le_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]) as usize,
            None => {
                return Err(AsdfError::DecompressionFailed(
                    "lz4 chunk is shorter than its size prefix".into(),
                ))
            }
        };
        let remaining = uncompressed_size - out.len();
        if claimed > remaining {
            return Err(AsdfError::DecompressionFailed(format!(
                "lz4 chunk claims {claimed} bytes but only {remaining} of the declared data_size {uncompressed_size} remain"
            )));
        }
        let decoded = lz4_flex::block::decompress_size_prepended(chunk)
            .map_err(|e| AsdfError::DecompressionFailed(e.to_string()))?;
        out.extend_from_slice(&decoded);
        pos = end;
    }
    if out.len() != uncompressed_size {
        return Err(AsdfError::DecompressionFailed(
            "lz4 output size mismatch".into(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    const BOMB_BYTES: usize = 64 << 20;

    fn header(compression: Compression, used: usize, data_size: u64) -> BlockHeader {
        BlockHeader {
            header_size: 48,
            flags: 0,
            compression,
            allocated_size: used as u64,
            used_size: used as u64,
            data_size,
        }
    }

    fn zlib(bytes: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    fn refusal(result: Result<Cow<'_, [u8]>, AsdfError>) -> String {
        match result {
            Err(AsdfError::DecompressionFailed(reason)) => reason,
            other => panic!("expected a decompression refusal, got {:?}", other.map(|b| b.len())),
        }
    }

    #[test]
    fn zlib_output_beyond_data_size_is_refused() {
        let payload = zlib(&vec![0u8; BOMB_BYTES]);
        let reason = refusal(header(Compression::Zlib, payload.len(), 16).decompress(&payload));
        assert!(reason.contains("data_size") && reason.contains("16"), "{reason}");
    }

    #[test]
    fn zlib_output_short_of_data_size_is_refused() {
        let payload = zlib(&[7u8; 8]);
        let reason = refusal(header(Compression::Zlib, payload.len(), 16).decompress(&payload));
        assert!(reason.contains("8 bytes") && reason.contains("16"), "{reason}");
    }

    #[test]
    fn zlib_output_of_exactly_data_size_is_kept() {
        let payload = zlib(&[7u8; 16]);
        let out = header(Compression::Zlib, payload.len(), 16).decompress(&payload).unwrap();
        assert_eq!(out.as_ref(), &[7u8; 16]);
    }

    #[cfg(feature = "asdf-full")]
    #[test]
    fn bzip2_output_beyond_data_size_is_refused() {
        let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        enc.write_all(&vec![0u8; BOMB_BYTES]).unwrap();
        let payload = enc.finish().unwrap();
        let reason = refusal(header(Compression::Bzip2, payload.len(), 16).decompress(&payload));
        assert!(reason.contains("data_size") && reason.contains("16"), "{reason}");
    }

    #[cfg(feature = "asdf-full")]
    fn lz4_chunk(bytes: &[u8], claimed: u32) -> Vec<u8> {
        let body = lz4_flex::block::compress(bytes);
        let mut out = Vec::new();
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(&claimed.to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[cfg(feature = "asdf-full")]
    #[test]
    fn lz4_chunk_prefix_beyond_the_remaining_size_is_refused() {
        let claimed: u32 = 256 << 20;
        let payload = lz4_chunk(&[7u8; 16], claimed);
        let reason = refusal(header(Compression::Lz4, payload.len(), 16).decompress(&payload));
        assert!(reason.contains("chunk") && reason.contains("268435456") && reason.contains("16"), "{reason}");
    }

    #[cfg(feature = "asdf-full")]
    #[test]
    fn lz4_blocks_of_several_chunks_are_joined_in_order() {
        let first: Vec<u8> = (0..40u8).collect();
        let second: Vec<u8> = (40..100u8).collect();
        let mut payload = lz4_chunk(&first, first.len() as u32);
        payload.extend_from_slice(&lz4_chunk(&second, second.len() as u32));
        let out = header(Compression::Lz4, payload.len(), 100).decompress(&payload).unwrap();
        assert_eq!(out.as_ref(), (0..100u8).collect::<Vec<u8>>().as_slice());
    }

    #[test]
    fn a_declared_size_beyond_the_memory_budget_is_refused_before_decompressing() {
        let payload = zlib(&vec![0u8; 1 << 20]);
        let budget = MemoryInfo { physical_available: 4096, commit_available: 4096 };
        let result = header(Compression::Zlib, payload.len(), 1 << 20).decompress_within(&payload, Some(budget));
        match result {
            Err(AsdfError::TooLarge { needed, available, .. }) => assert_eq!((needed, available), (1 << 20, 4096)),
            other => panic!("expected a memory refusal, got {:?}", other.map(|b| b.len())),
        }
        let message = AsdfError::TooLarge { what: "Decompressing a zlib block".into(), needed: 1 << 20, available: 4096 }.to_string();
        assert!(message.contains("Decompressing a zlib block needs about 1.0 MB") && message.contains("ASTROBURST_MEMORY_BUDGET_MB"), "{message}");
    }
}
