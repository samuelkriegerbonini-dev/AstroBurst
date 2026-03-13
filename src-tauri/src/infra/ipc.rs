use anyhow::{Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

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

    struct MinMax {
        min: f32,
        max: f32,
    }

    let mm = slice
        .par_chunks(65536)
        .map(|chunk| {
            let mut local = MinMax {
                min: f32::MAX,
                max: f32::MIN,
            };
            for &v in chunk {
                if v.is_finite() {
                    if v < local.min {
                        local.min = v;
                    }
                    if v > local.max {
                        local.max = v;
                    }
                }
            }
            local
        })
        .reduce(
            || MinMax {
                min: f32::MAX,
                max: f32::MIN,
            },
            |a, b| MinMax {
                min: a.min.min(b.min),
                max: a.max.max(b.max),
            },
        );

    let data_min = if mm.min > mm.max { 0.0 } else { mm.min };
    let data_max = if mm.min > mm.max { 1.0 } else { mm.max };

    let npix = slice.len();
    let mut bytes = Vec::with_capacity(npix * 4);

    let has_non_finite = slice.par_iter().any(|v| !v.is_finite());

    if has_non_finite {
        bytes.reserve_exact(npix * 4);
        for &v in slice {
            let clean = if v.is_finite() { v } else { 0.0f32 };
            bytes.extend_from_slice(&clean.to_le_bytes());
        }
    } else {
        let byte_ptr = slice.as_ptr() as *const u8;
        let byte_len = npix * 4;
        bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(byte_ptr, byte_len) });
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

    let mut output = Vec::with_capacity(16 + buf.bytes.len());
    output.extend_from_slice(&buf.width.to_le_bytes());
    output.extend_from_slice(&buf.height.to_le_bytes());
    output.extend_from_slice(&buf.data_min.to_le_bytes());
    output.extend_from_slice(&buf.data_max.to_le_bytes());
    output.extend_from_slice(&buf.bytes);
    Ok(output)
}

pub fn encode_with_header_downsampled(arr: &Array2<f32>, max_dim: usize) -> Result<Vec<u8>> {
    let (rows, cols) = arr.dim();
    if rows <= max_dim && cols <= max_dim {
        return encode_with_header(arr);
    }

    let slice = arr.as_slice().context("Array2 must be contiguous")?;
    let scale = max_dim as f64 / (rows.max(cols) as f64);
    let dst_rows = ((rows as f64) * scale).round().max(1.0) as usize;
    let dst_cols = ((cols as f64) * scale).round().max(1.0) as usize;

    let y_ratio = rows as f64 / dst_rows as f64;
    let x_ratio = cols as f64 / dst_cols as f64;

    struct MinMax {
        min: f32,
        max: f32,
    }

    let npix = dst_rows * dst_cols;
    let mut pixel_bytes = Vec::with_capacity(npix * 4);
    let mut mm = MinMax { min: f32::MAX, max: f32::MIN };

    for dy in 0..dst_rows {
        let sy = ((dy as f64) * y_ratio).min((rows - 1) as f64) as usize;
        let src_row = sy * cols;
        for dx in 0..dst_cols {
            let sx = ((dx as f64) * x_ratio).min((cols - 1) as f64) as usize;
            let v = slice[src_row + sx];
            let clean = if v.is_finite() { v } else { 0.0f32 };
            if clean.is_finite() {
                if clean < mm.min { mm.min = clean; }
                if clean > mm.max { mm.max = clean; }
            }
            pixel_bytes.extend_from_slice(&clean.to_le_bytes());
        }
    }

    let data_min = if mm.min > mm.max { 0.0 } else { mm.min };
    let data_max = if mm.min > mm.max { 1.0 } else { mm.max };

    let mut output = Vec::with_capacity(16 + pixel_bytes.len());
    output.extend_from_slice(&(dst_cols as u32).to_le_bytes());
    output.extend_from_slice(&(dst_rows as u32).to_le_bytes());
    output.extend_from_slice(&data_min.to_le_bytes());
    output.extend_from_slice(&data_max.to_le_bytes());
    output.extend_from_slice(&pixel_bytes);
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

        let first_f32 =
            f32::from_le_bytes([buf.bytes[0], buf.bytes[1], buf.bytes[2], buf.bytes[3]]);
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

        let first =
            f32::from_le_bytes([buf.bytes[0], buf.bytes[1], buf.bytes[2], buf.bytes[3]]);
        assert_eq!(first, 0.0);
    }

    #[test]
    fn test_downsample_no_op_when_small() {
        let arr = Array2::from_shape_fn((100, 100), |(r, c)| (r + c) as f32);
        let data = encode_with_header_downsampled(&arr, 200).unwrap();
        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!(w, 100);
        assert_eq!(h, 100);
    }

    #[test]
    fn test_downsample_large() {
        let arr = Array2::from_shape_fn((4096, 4096), |(r, c)| (r + c) as f32);
        let data = encode_with_header_downsampled(&arr, 2048).unwrap();
        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!(w, 2048);
        assert_eq!(h, 2048);
    }

    #[test]
    fn test_encode_downsampled() {
        let arr = Array2::from_shape_fn((4096, 4096), |(r, c)| (r + c) as f32);
        let data = encode_with_header_downsampled(&arr, 1024).unwrap();
        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!(w, 1024);
        assert_eq!(h, 1024);
        assert_eq!(data.len(), 16 + 1024 * 1024 * 4);
    }
}
