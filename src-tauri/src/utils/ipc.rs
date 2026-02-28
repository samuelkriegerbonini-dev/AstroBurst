use anyhow::{Context, Result};
use ndarray::Array2;

pub struct RawPixelBuffer {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub data_min: f32,
    pub data_max: f32,
}

pub fn encode_f32_buffer(arr: &Array2<f32>) -> Result<RawPixelBuffer> {
    let (rows, cols) = arr.dim();
    let slice = arr.as_slice().context("Array2 must be contiguous")?;

    let mut data_min = f32::MAX;
    let mut data_max = f32::MIN;

    let byte_len = slice.len() * 4;
    let mut bytes = Vec::with_capacity(byte_len);

    for &v in slice {
        let safe = if v.is_finite() { v } else { 0.0 };
        if safe.is_finite() && safe > 1e-7 {
            if safe < data_min { data_min = safe; }
            if safe > data_max { data_max = safe; }
        }
        bytes.extend_from_slice(&safe.to_le_bytes());
    }

    if data_min > data_max {
        data_min = 0.0;
        data_max = 1.0;
    }

    Ok(RawPixelBuffer {
        bytes,
        width: cols as u32,
        height: rows as u32,
        data_min,
        data_max,
    })
}

pub fn build_header(buf: &RawPixelBuffer) -> Vec<u8> {
    let mut header = Vec::with_capacity(16);
    header.extend_from_slice(&buf.width.to_le_bytes());
    header.extend_from_slice(&buf.height.to_le_bytes());
    header.extend_from_slice(&buf.data_min.to_le_bytes());
    header.extend_from_slice(&buf.data_max.to_le_bytes());
    header
}

pub fn encode_with_header(arr: &Array2<f32>) -> Result<Vec<u8>> {
    let buf = encode_f32_buffer(arr)?;
    let header = build_header(&buf);

    let mut output = Vec::with_capacity(header.len() + buf.bytes.len());
    output.extend_from_slice(&header);
    output.extend_from_slice(&buf.bytes);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_roundtrip() {
        let arr = Array2::from_shape_fn((64, 64), |(r, c)| (r * 64 + c) as f32);
        let buf = encode_f32_buffer(&arr).unwrap();

        assert_eq!(buf.width, 64);
        assert_eq!(buf.height, 64);
        assert_eq!(buf.bytes.len(), 64 * 64 * 4);

        let first_f32 = f32::from_le_bytes([buf.bytes[0], buf.bytes[1], buf.bytes[2], buf.bytes[3]]);
        assert!((first_f32 - 0.0).abs() < 1e-6);

        let last_offset = (64 * 64 - 1) * 4;
        let last_f32 = f32::from_le_bytes([
            buf.bytes[last_offset],
            buf.bytes[last_offset + 1],
            buf.bytes[last_offset + 2],
            buf.bytes[last_offset + 3],
        ]);
        assert!((last_f32 - 4095.0).abs() < 1e-6);
    }

    #[test]
    fn test_header_layout() {
        let arr = Array2::from_shape_fn((100, 200), |(r, c)| (r + c) as f32 + 1.0);
        let buf = encode_f32_buffer(&arr).unwrap();
        let header = build_header(&buf);

        assert_eq!(header.len(), 16);
        let w = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let h = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        assert_eq!(w, 200);
        assert_eq!(h, 100);
    }

    #[test]
    fn test_encode_with_header() {
        let arr = Array2::from_shape_fn((10, 10), |(r, c)| (r * 10 + c) as f32 + 1.0);
        let data = encode_with_header(&arr).unwrap();
        assert_eq!(data.len(), 16 + 10 * 10 * 4);
    }

    #[test]
    fn test_nan_handling() {
        let mut raw = vec![1.0f32; 16];
        raw[0] = f32::NAN;
        raw[1] = f32::INFINITY;
        let arr = Array2::from_shape_vec((4, 4), raw).unwrap();
        let buf = encode_f32_buffer(&arr).unwrap();

        let first = f32::from_le_bytes([buf.bytes[0], buf.bytes[1], buf.bytes[2], buf.bytes[3]]);
        assert_eq!(first, 0.0);
    }
}
