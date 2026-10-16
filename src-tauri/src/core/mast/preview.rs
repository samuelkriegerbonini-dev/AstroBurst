use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};

use super::jpeg::{dimensions, sniff, ImageKind};
use super::types::{MastCmdError, MastErrorCode, MastErrorContext, PreviewSize};

pub const MAX_DECODE_PIXELS: u64 = 400_000_000;
pub const JPEG_QUALITY: u8 = 90;
const DECODE_SLACK_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreviewPlan {
    PassThrough,
    Resize { width: u32, height: u32, factor: f64 },
    TooLarge,
}

fn scaled(side: u32, long: u32, edge: u32) -> u32 {
    ((f64::from(side) * f64::from(edge) / f64::from(long)).round() as u32).max(1)
}

pub fn plan(width: u32, height: u32, size: PreviewSize) -> PreviewPlan {
    let edge = size.long_edge();
    let long = width.max(height);
    if long <= edge {
        return PreviewPlan::PassThrough;
    }
    if u64::from(width) * u64::from(height) > MAX_DECODE_PIXELS {
        return PreviewPlan::TooLarge;
    }
    let (width, height) = if width >= height { (edge, scaled(height, long, edge)) } else { (scaled(width, long, edge), edge) };
    PreviewPlan::Resize { width, height, factor: f64::from(long) / f64::from(edge) }
}

pub fn prepared_name(stem: &str, size: PreviewSize) -> String {
    format!("{stem}.{}.jpg", size.long_edge())
}

fn too_large(bytes: usize) -> MastCmdError {
    MastCmdError::new(MastErrorCode::TooLarge, format!("image exceeds {MAX_DECODE_PIXELS} pixels")).with_context(MastErrorContext {
        bytes: Some(bytes as u64),
        ..Default::default()
    })
}

fn decode_error() -> MastCmdError {
    MastCmdError::new(MastErrorCode::Decode, "image could not be decoded")
}

fn decode(bytes: &[u8]) -> Result<DynamicImage, MastCmdError> {
    let format = match sniff(bytes).ok_or_else(decode_error)? {
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Png => ImageFormat::Png,
    };
    let (width, height, channels) = dimensions(bytes).ok_or_else(decode_error)?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_DECODE_PIXELS {
        return Err(too_large(bytes.len()));
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(pixels * u64::from(channels.max(1)) * 2 + DECODE_SLACK_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader.decode().map_err(|_| decode_error())
}

fn encodable(image: DynamicImage) -> DynamicImage {
    match image {
        DynamicImage::ImageLuma8(_) | DynamicImage::ImageRgb8(_) => image,
        DynamicImage::ImageLumaA8(_) | DynamicImage::ImageLuma16(_) | DynamicImage::ImageLumaA16(_) => {
            DynamicImage::ImageLuma8(image.to_luma8())
        }
        other => DynamicImage::ImageRgb8(other.to_rgb8()),
    }
}

pub fn resize_jpeg(bytes: &[u8], plan: PreviewPlan) -> Result<Vec<u8>, MastCmdError> {
    let (width, height) = match plan {
        PreviewPlan::PassThrough => return Ok(bytes.to_vec()),
        PreviewPlan::TooLarge => return Err(too_large(bytes.len())),
        PreviewPlan::Resize { width, height, .. } => (width, height),
    };
    let image = decode(bytes)?;
    let resized = encodable(image.resize_exact(width, height, FilterType::Triangle));
    let mut out = Vec::new();
    resized
        .write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
        .map_err(|_| MastCmdError::new(MastErrorCode::Decode, "preview could not be encoded"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::jpeg::{dimensions, sniff, ImageKind};
    use crate::core::mast::types::MastErrorCode;
    use crate::infra::mast::testing::synthetic_jpeg;

    const HEADER_ONLY_JPEG: [u8; 17] = [0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x03, 0xE8, 0x0B, 0xB8, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xD9];

    fn resize(width: u32, height: u32) -> (u32, u32) {
        match plan(width, height, PreviewSize::Pane) {
            PreviewPlan::Resize { width, height, .. } => (width, height),
            other => panic!("{width}x{height}: {other:?}"),
        }
    }

    #[test]
    fn preview_plan_pass_through_and_resize() {
        assert_eq!(plan(2048, 2048, PreviewSize::Pane), PreviewPlan::PassThrough);
        assert_eq!(plan(2048, 1, PreviewSize::Pane), PreviewPlan::PassThrough);
        assert_eq!(plan(4096, 4096, PreviewSize::Large), PreviewPlan::PassThrough);
        assert_eq!(plan(2049, 100, PreviewSize::Pane), PreviewPlan::Resize { width: 2048, height: 100, factor: 2049.0 / 2048.0 });
        assert_eq!(plan(4096, 4096, PreviewSize::Pane), PreviewPlan::Resize { width: 2048, height: 2048, factor: 2.0 });
        assert_eq!(resize(7065, 4177), (2048, 1211));
        assert_eq!(resize(4253, 4659), (1870, 2048));
        assert_eq!(resize(10_000, 1), (2048, 1));
        assert_eq!(
            plan(14341, 8583, PreviewSize::Large),
            PreviewPlan::Resize { width: 4096, height: 2451, factor: 14341.0 / 4096.0 }
        );
        assert_eq!(plan(4097, 3000, PreviewSize::Large), PreviewPlan::Resize { width: 4096, height: 2999, factor: 4097.0 / 4096.0 });
        assert!(matches!(plan(20_000, 20_000, PreviewSize::Pane), PreviewPlan::Resize { width: 2048, height: 2048, .. }));
        assert_eq!(plan(20_001, 20_000, PreviewSize::Pane), PreviewPlan::TooLarge);
        assert_eq!(plan(20_001, 20_000, PreviewSize::Large), PreviewPlan::TooLarge);
        assert_eq!(prepared_name("jw02739-o001_t001_nircam_clear-f187n_i2d", PreviewSize::Pane), "jw02739-o001_t001_nircam_clear-f187n_i2d.2048.jpg");
        assert_eq!(prepared_name("ick909030_drc", PreviewSize::Large), "ick909030_drc.4096.jpg");
    }

    #[test]
    fn resize_writes_long_edge_2048() {
        let grey = synthetic_jpeg(3000, 1000, false, None);
        let step = plan(3000, 1000, PreviewSize::Pane);
        assert_eq!(step, PreviewPlan::Resize { width: 2048, height: 683, factor: 3000.0 / 2048.0 });
        let out = resize_jpeg(&grey, step).unwrap();
        assert_eq!(sniff(&out), Some(ImageKind::Jpeg));
        assert_eq!(dimensions(&out), Some((2048, 683, 1)));
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2048, 683));
        let rgb = synthetic_jpeg(1000, 3000, true, None);
        let out = resize_jpeg(&rgb, plan(1000, 3000, PreviewSize::Pane)).unwrap();
        assert_eq!(dimensions(&out), Some((683, 2048, 3)));
        let large = resize_jpeg(&rgb, plan(1000, 3000, PreviewSize::Large)).unwrap();
        assert_eq!(large, rgb);
        let too_large = resize_jpeg(&grey, PreviewPlan::TooLarge).unwrap_err();
        assert_eq!(too_large.code, MastErrorCode::TooLarge);
        let broken = resize_jpeg(b"\xFF\xD8\xFF\xE0 not really a jpeg", PreviewPlan::Resize { width: 10, height: 10, factor: 2.0 }).unwrap_err();
        assert_eq!(broken.code, MastErrorCode::Decode);
        let frame_only = HEADER_ONLY_JPEG.to_vec();
        assert_eq!(dimensions(&frame_only), Some((3000, 1000, 1)));
        let undecodable = resize_jpeg(&frame_only, plan(3000, 1000, PreviewSize::Pane)).unwrap_err();
        assert_eq!(undecodable.code, MastErrorCode::Decode);    }
}
