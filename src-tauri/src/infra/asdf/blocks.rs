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
    pub checksum: [u8; 16],
}

#[derive(Debug, Clone, PartialEq)]
pub enum Compression {
    None,
    Zlib,
    Bzip2,
    Lz4,
}

#[derive(Debug)]
pub struct BlockData {
    pub index: usize,
    pub data: Vec<u8>,
    pub original_size: usize,
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
        if h.len() < 46 {
            return Err(AsdfError::InvalidBlockHeader);
        }

        let flags = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);

        let compression = Self::parse_compression(&h[4..8])?;

        let allocated_size = u64::from_be_bytes([
            h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15],
        ]);
        let used_size = u64::from_be_bytes([
            h[16], h[17], h[18], h[19], h[20], h[21], h[22], h[23],
        ]);
        let data_size = u64::from_be_bytes([
            h[24], h[25], h[26], h[27], h[28], h[29], h[30], h[31],
        ]);

        let mut checksum = [0u8; 16];
        checksum.copy_from_slice(&h[32..48]);

        Ok((
            Self {
                header_size,
                flags,
                compression,
                allocated_size,
                used_size,
                data_size,
                checksum,
            },
            total_header,
        ))
    }

    fn parse_compression(bytes: &[u8]) -> Result<Compression, AsdfError> {
        let s: Vec<u8> = bytes.iter().copied().take_while(|&b| b != 0).collect();

        if s.is_empty() {
            return Ok(Compression::None);
        }

        match s.as_slice() {
            b"zlib" => Ok(Compression::Zlib),
            b"bzp2" => Ok(Compression::Bzip2),
            b"lz4\x00" | b"lz4" => Ok(Compression::Lz4),
            b"\x00\x00\x00\x00" => Ok(Compression::None),
            other => {
                let name = String::from_utf8_lossy(other).to_string();
                Err(AsdfError::UnsupportedCompression(name))
            }
        }
    }

    pub fn decompress(&self, raw: &[u8]) -> Result<Vec<u8>, AsdfError> {
        match self.compression {
            Compression::None => Ok(raw.to_vec()),

            Compression::Zlib => {
                let mut decoder = flate2::read::ZlibDecoder::new(raw);
                let mut out = Vec::with_capacity(self.data_size as usize);
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| AsdfError::DecompressionFailed(e.to_string()))?;
                Ok(out)
            }

            #[cfg(feature = "asdf-full")]
            Compression::Bzip2 => {
                let mut decoder = bzip2::read::BzDecoder::new(raw);
                let mut out = Vec::with_capacity(self.data_size as usize);
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| AsdfError::DecompressionFailed(e.to_string()))?;
                Ok(out)
            }

            #[cfg(not(feature = "asdf-full"))]
            Compression::Bzip2 => {
                Err(AsdfError::UnsupportedCompression(
                    "bzip2 (enable 'asdf-full' feature)".into(),
                ))
            }

            #[cfg(feature = "asdf-full")]
            Compression::Lz4 => {
                let decompressed = lz4_flex::decompress(raw, self.data_size as usize)
                    .map_err(|e| AsdfError::DecompressionFailed(e.to_string()))?;
                Ok(decompressed)
            }

            #[cfg(not(feature = "asdf-full"))]
            Compression::Lz4 => {
                Err(AsdfError::UnsupportedCompression(
                    "lz4 (enable 'asdf-full' feature)".into(),
                ))
            }
        }
    }
}

impl BlockData {
    pub fn as_f32_be(&self) -> Vec<f32> {
        self.data
            .chunks_exact(4)
            .map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    pub fn as_f64_be(&self) -> Vec<f64> {
        self.data
            .chunks_exact(8)
            .map(|c| {
                f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
            })
            .collect()
    }

    pub fn as_f32_le(&self) -> Vec<f32> {
        self.data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    pub fn as_f64_le(&self) -> Vec<f64> {
        self.data
            .chunks_exact(8)
            .map(|c| {
                f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
            })
            .collect()
    }

    pub fn as_i16_be(&self) -> Vec<i16> {
        self.data
            .chunks_exact(2)
            .map(|c| i16::from_be_bytes([c[0], c[1]]))
            .collect()
    }

    pub fn as_u16_be(&self) -> Vec<u16> {
        self.data
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect()
    }

    pub fn as_i32_be(&self) -> Vec<i32> {
        self.data
            .chunks_exact(4)
            .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }
}
