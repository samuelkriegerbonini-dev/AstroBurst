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

struct ScanResult {
    min: f32,
    max: f32,
    sum: f64,
    sum_sq: f64,
    count: u64,
    has_non_finite: bool,
}

impl ScanResult {
    fn identity() -> Self {
        Self {
            min: f32::MAX,
            max: f32::MIN,
            sum: 0.0,
            sum_sq: 0.0,
            count: 0,
            has_non_finite: false,
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
            sum: self.sum + other.sum,
            sum_sq: self.sum_sq + other.sum_sq,
            count: self.count + other.count,
            has_non_finite: self.has_non_finite || other.has_non_finite,
        }
    }

    fn sigma(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let mean = self.sum / self.count as f64;
        (self.sum_sq / self.count as f64 - mean * mean).max(0.0).sqrt()
    }
}

fn scan_extrema(slice: &[f32]) -> ScanResult {
    slice
        .par_chunks(65536)
        .map(|chunk| {
            let mut local = ScanResult::identity();
            for &v in chunk {
                if v.is_finite() {
                    if v < local.min { local.min = v; }
                    if v > local.max { local.max = v; }
                    local.sum += v as f64;
                    local.sum_sq += (v as f64) * (v as f64);
                    local.count += 1;
                } else {
                    local.has_non_finite = true;
                }
            }
            local
        })
        .reduce(ScanResult::identity, ScanResult::merge)
}

pub fn encode_f32_buffer(arr: &Array2<f32>) -> Result<RawPixelBuffer> {
    let (rows, cols) = arr.dim();
    let slice = arr.as_slice().context("Array2 must be contiguous")?;

    let scan = scan_extrema(slice);

    let data_min = if scan.min > scan.max { 0.0 } else { scan.min };
    let data_max = if scan.min > scan.max { 1.0 } else { scan.max };

    let npix = slice.len();
    let byte_len = npix * 4;

    let bytes = if scan.has_non_finite {
        let mut buf = Vec::with_capacity(byte_len);
        for &v in slice {
            let clean = if v.is_finite() { v } else { 0.0f32 };
            buf.extend_from_slice(&clean.to_le_bytes());
        }
        buf
    } else {
        let byte_ptr = slice.as_ptr() as *const u8;
        unsafe { std::slice::from_raw_parts(byte_ptr, byte_len) }.to_vec()
    };

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

struct ChannelPreview {
    data_min: f32,
    data_max: f32,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

fn encode_channel_preview(arr: &Array2<f32>, max_dim: usize) -> Result<ChannelPreview> {
    let (rows, cols) = arr.dim();

    if rows <= max_dim && cols <= max_dim {
        let buf = encode_f32_buffer(arr)?;
        return Ok(ChannelPreview {
            data_min: buf.data_min,
            data_max: buf.data_max,
            width: buf.width,
            height: buf.height,
            pixels: buf.bytes,
        });
    }

    let slice = arr.as_slice().context("Array2 must be contiguous")?;

    let scan = scan_extrema(slice);
    let data_min = if scan.min > scan.max { 0.0 } else { scan.min };
    let data_max = if scan.min > scan.max { 1.0 } else { scan.max };

    let scale = max_dim as f64 / (rows.max(cols) as f64);
    let dst_rows = ((rows as f64) * scale).round().max(1.0) as usize;
    let dst_cols = ((cols as f64) * scale).round().max(1.0) as usize;

    let npix = dst_rows * dst_cols;
    let mut pixel_bytes = vec![0u8; npix * 4];

    const PEAK_SIGNIFICANCE: f32 = 3.0;
    let s = (PEAK_SIGNIFICANCE * scan.sigma() as f32).max(0.0);

    pixel_bytes
        .par_chunks_mut(dst_cols * 4)
        .enumerate()
        .for_each(|(dy, out_row)| {
            let sy0 = dy * rows / dst_rows;
            let sy1 = (((dy + 1) * rows) / dst_rows).max(sy0 + 1).min(rows);
            for dx in 0..dst_cols {
                let sx0 = dx * cols / dst_cols;
                let sx1 = (((dx + 1) * cols) / dst_cols).max(sx0 + 1).min(cols);

                let mut sum = 0.0f64;
                let mut count = 0u32;
                let mut cell_max = f32::MIN;
                for yy in sy0..sy1 {
                    let row = yy * cols;
                    for xx in sx0..sx1 {
                        let v = slice[row + xx];
                        if v.is_finite() {
                            sum += v as f64;
                            count += 1;
                            if v > cell_max { cell_max = v; }
                        }
                    }
                }

                let out = if count == 0 {
                    0.0f32
                } else {
                    let mean = (sum / count as f64) as f32;
                    let d = cell_max - mean;
                    if d > 0.0 { mean + d * (d / (d + s)) } else { mean }
                };
                out_row[dx * 4..dx * 4 + 4].copy_from_slice(&out.to_le_bytes());
            }
        });

    Ok(ChannelPreview {
        data_min,
        data_max,
        width: dst_cols as u32,
        height: dst_rows as u32,
        pixels: pixel_bytes,
    })
}

pub fn encode_with_header_downsampled(arr: &Array2<f32>, max_dim: usize) -> Result<Vec<u8>> {
    let ch = encode_channel_preview(arr, max_dim)?;

    let mut output = Vec::with_capacity(16 + ch.pixels.len());
    output.extend_from_slice(&ch.width.to_le_bytes());
    output.extend_from_slice(&ch.height.to_le_bytes());
    output.extend_from_slice(&ch.data_min.to_le_bytes());
    output.extend_from_slice(&ch.data_max.to_le_bytes());
    output.extend_from_slice(&ch.pixels);
    Ok(output)
}

pub fn encode_rgb_with_header_downsampled(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    max_dim: usize,
) -> Result<Vec<u8>> {
    encode_rgb_with_header_downsampled_flagged(r, g, b, max_dim, 0)
}

pub fn encode_rgb_with_header_downsampled_flagged(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    max_dim: usize,
    source_flag: u8,
) -> Result<Vec<u8>> {
    if r.dim() != g.dim() || r.dim() != b.dim() {
        anyhow::bail!(
            "RGB channels have mismatched dimensions: r={:?} g={:?} b={:?}",
            r.dim(),
            g.dim(),
            b.dim()
        );
    }

    let rc = encode_channel_preview(r, max_dim)?;
    let gc = encode_channel_preview(g, max_dim)?;
    let bc = encode_channel_preview(b, max_dim)?;

    if (rc.width, rc.height) != (gc.width, gc.height)
        || (rc.width, rc.height) != (bc.width, bc.height)
    {
        anyhow::bail!(
            "RGB channel preview dims diverged: r={}x{} g={}x{} b={}x{}",
            rc.width, rc.height, gc.width, gc.height, bc.width, bc.height
        );
    }

    let mut output = Vec::with_capacity(33 + rc.pixels.len() * 3);
    output.extend_from_slice(&rc.width.to_le_bytes());
    output.extend_from_slice(&rc.height.to_le_bytes());
    output.extend_from_slice(&rc.data_min.to_le_bytes());
    output.extend_from_slice(&rc.data_max.to_le_bytes());
    output.extend_from_slice(&gc.data_min.to_le_bytes());
    output.extend_from_slice(&gc.data_max.to_le_bytes());
    output.extend_from_slice(&bc.data_min.to_le_bytes());
    output.extend_from_slice(&bc.data_max.to_le_bytes());
    output.extend_from_slice(&rc.pixels);
    output.extend_from_slice(&gc.pixels);
    output.extend_from_slice(&bc.pixels);
    output.push(source_flag);
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

    #[test]
    fn test_downsampled_extrema_from_full_image() {
        let mut arr = Array2::from_shape_fn((4, 4), |_| 1.0f32);
        arr[[1, 1]] = 999.0;
        arr[[3, 1]] = -7.0;
        let data = encode_with_header_downsampled(&arr, 2).unwrap();

        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!((w, h), (2, 2));

        let data_min = f32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let data_max = f32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        assert_eq!(data_min, -7.0);
        assert_eq!(data_max, 999.0);
    }

    #[test]
    fn test_downsample_hybrid_preserves_peaks() {
        let mut arr = Array2::from_shape_fn((4, 4), |_| 10.0f32);
        arr[[1, 1]] = 1000.0;
        let data = encode_with_header_downsampled(&arr, 2).unwrap();

        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        assert_eq!((w, h), (2, 2));

        let px = |i: usize| {
            let off = 16 + i * 4;
            f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
        };
        let star_cell = px(0);
        let flat_cell = px(3);

        assert!((flat_cell - 10.0).abs() < 1e-3, "flat cell {flat_cell} should be ~10");
        let box_avg = (10.0 + 10.0 + 10.0 + 1000.0) / 4.0;
        assert!(star_cell > box_avg, "star cell {star_cell} should exceed box-average {box_avg}");
        assert!(star_cell < 1000.0, "star cell {star_cell} should be below the raw peak");
        assert!(star_cell > flat_cell, "star cell should be brighter than background");
    }

    fn rgb_px(data: &[u8], base: usize, i: usize) -> f32 {
        let off = base + i * 4;
        f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
    }

    fn rgb_header_f32(data: &[u8], byte: usize) -> f32 {
        f32::from_le_bytes([data[byte], data[byte + 1], data[byte + 2], data[byte + 3]])
    }

    #[test]
    fn test_rgb_header_and_planar_layout_small() {
        let r = Array2::from_shape_fn((4, 4), |(y, x)| (y * 4 + x) as f32);
        let g = Array2::from_shape_fn((4, 4), |(y, x)| 100.0 + (y * 4 + x) as f32);
        let b = Array2::from_shape_fn((4, 4), |(y, x)| -50.0 + (y * 4 + x) as f32);
        let data = encode_rgb_with_header_downsampled(&r, &g, &b, 64).unwrap();

        assert_eq!(data.len(), 32 + 3 * 16 * 4 + 1);
        assert_eq!(data[data.len() - 1], 0);

        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!((w, h), (4, 4));

        assert_eq!(rgb_header_f32(&data, 8), 0.0);
        assert_eq!(rgb_header_f32(&data, 12), 15.0);
        assert_eq!(rgb_header_f32(&data, 16), 100.0);
        assert_eq!(rgb_header_f32(&data, 20), 115.0);
        assert_eq!(rgb_header_f32(&data, 24), -50.0);
        assert_eq!(rgb_header_f32(&data, 28), -35.0);

        let (r_base, g_base, b_base) = (32, 32 + 64, 32 + 128);
        assert_eq!(rgb_px(&data, r_base, 0), 0.0);
        assert_eq!(rgb_px(&data, r_base, 15), 15.0);
        assert_eq!(rgb_px(&data, g_base, 0), 100.0);
        assert_eq!(rgb_px(&data, g_base, 15), 115.0);
        assert_eq!(rgb_px(&data, b_base, 0), -50.0);
        assert_eq!(rgb_px(&data, b_base, 15), -35.0);
    }

    #[test]
    fn test_rgb_downsample_large() {
        let r = Array2::from_shape_fn((1000, 1000), |(y, x)| (y + x) as f32);
        let g = Array2::from_shape_fn((1000, 1000), |(y, x)| (y + x) as f32 + 1.0);
        let b = Array2::from_shape_fn((1000, 1000), |(y, x)| (y + x) as f32 + 2.0);
        let data = encode_rgb_with_header_downsampled(&r, &g, &b, 500).unwrap();

        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!((w, h), (500, 500));
        assert_eq!(data.len(), 32 + 3 * 500 * 500 * 4 + 1);
    }

    #[test]
    fn test_rgb_per_channel_extrema_from_full_image() {
        let mut r = Array2::from_shape_fn((4, 4), |_| 1.0f32);
        r[[1, 1]] = 999.0;
        r[[3, 1]] = -7.0;
        let g = Array2::from_shape_fn((4, 4), |_| 5.0f32);
        let mut b = Array2::from_shape_fn((4, 4), |_| 2.0f32);
        b[[1, 1]] = -3.0;

        let data = encode_rgb_with_header_downsampled(&r, &g, &b, 2).unwrap();

        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!((w, h), (2, 2));

        assert_eq!(rgb_header_f32(&data, 8), -7.0);
        assert_eq!(rgb_header_f32(&data, 12), 999.0);
        assert_eq!(rgb_header_f32(&data, 16), 5.0);
        assert_eq!(rgb_header_f32(&data, 20), 5.0);
        assert_eq!(rgb_header_f32(&data, 24), -3.0);
        assert_eq!(rgb_header_f32(&data, 28), 2.0);
    }

    #[test]
    fn test_rgb_nan_sanitized_per_channel() {
        let mut r = Array2::from_shape_fn((4, 4), |_| 1.0f32);
        r[[0, 0]] = f32::NAN;
        let g = Array2::from_shape_fn((4, 4), |_| 2.0f32);
        let b = Array2::from_shape_fn((4, 4), |_| 3.0f32);

        let data = encode_rgb_with_header_downsampled(&r, &g, &b, 64).unwrap();

        assert_eq!(rgb_header_f32(&data, 8), 1.0);
        assert_eq!(rgb_header_f32(&data, 12), 1.0);
        assert_eq!(rgb_px(&data, 32, 0), 0.0);
    }

    #[test]
    fn test_rgb_mismatched_dims_errors() {
        let r = Array2::from_shape_fn((4, 4), |_| 1.0f32);
        let g = Array2::from_shape_fn((4, 4), |_| 1.0f32);
        let b = Array2::from_shape_fn((3, 3), |_| 1.0f32);
        assert!(encode_rgb_with_header_downsampled(&r, &g, &b, 64).is_err());
    }
}
