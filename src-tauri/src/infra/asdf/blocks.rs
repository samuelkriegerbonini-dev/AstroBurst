use std::borrow::Cow;
use std::io::Read;

use super::parser::AsdfError;

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
        let expected = self.data_size as usize;
        match &self.compression {
            Compression::None => Ok(Cow::Borrowed(raw)),

            Compression::Zlib => {
                let mut decoder = flate2::read::ZlibDecoder::new(raw);
                let mut out = Vec::new();
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| AsdfError::DecompressionFailed(e.to_string()))?;
                Ok(Cow::Owned(out))
            }

            #[cfg(feature = "asdf-full")]
            Compression::Bzip2 => {
                let mut decoder = bzip2::read::BzDecoder::new(raw);
                let mut out = Vec::new();
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| AsdfError::DecompressionFailed(e.to_string()))?;
                Ok(Cow::Owned(out))
            }

            #[cfg(not(feature = "asdf-full"))]
            Compression::Bzip2 => Err(AsdfError::UnsupportedCompression(
                "bzip2 (enable 'asdf-full' feature)".into(),
            )),

            #[cfg(feature = "asdf-full")]
            Compression::Lz4 => decompress_lz4_asdf(raw, expected).map(Cow::Owned),

            #[cfg(not(feature = "asdf-full"))]
            Compression::Lz4 => Err(AsdfError::UnsupportedCompression(
                "lz4 (enable 'asdf-full' feature)".into(),
            )),

            Compression::Unknown(name) => Err(AsdfError::UnsupportedCompression(name.clone())),
        }
    }
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
        let chunk = lz4_flex::block::decompress_size_prepended(&payload[pos..end])
            .map_err(|e| AsdfError::DecompressionFailed(e.to_string()))?;
        out.extend_from_slice(&chunk);
        pos = end;
    }
    if out.len() != uncompressed_size {
        return Err(AsdfError::DecompressionFailed(
            "lz4 output size mismatch".into(),
        ));
    }
    Ok(out)
}
