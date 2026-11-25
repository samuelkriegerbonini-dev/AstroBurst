use crate::infra::fits::reader::test_fixtures::{write_raw_hdus, write_test_mef, HduData, TestHdu};
use crate::infra::fits::writer::write_fits_mono;
use crate::service::area::test_fixtures::gaussian_pixels;
use crate::types::header::HduHeader;
use crate::types::image_ref::BandPlaneKey;

pub const SIZE: usize = 64;
pub const NO_DATA_PIXELS: [(usize, usize); 3] = [(32, 30), (32, 31), (33, 30)];
pub const AFW_BAD_PIXEL: (usize, usize) = (32, 34);
pub const AFW_SATURATED_PIXEL: (usize, usize) = (32, 32);
pub const AFW_DETECTED_BLOCK: std::ops::Range<usize> = 30..35;
pub const UNNAMED_FLAG_PIXEL: (usize, usize) = (32, 33);

pub fn layer(extname: &'static str, data: HduData, extra_cards: Vec<(&'static str, String)>) -> TestHdu {
    TestHdu { extname: Some(extname), extver: None, cols: SIZE, rows: SIZE, data, extra_cards }
}

pub fn star(extname: &'static str) -> TestHdu {
    layer(extname, HduData::F32(gaussian_pixels(SIZE, 1000.0, 2.0, 100.0)), vec![])
}

pub fn flat(extname: &'static str, value: f32) -> TestHdu {
    layer(extname, HduData::F32(vec![value; SIZE * SIZE]), vec![])
}

pub fn flagged(pixels: &[(usize, usize)], value: i32) -> Vec<i32> {
    let mut bits = vec![0i32; SIZE * SIZE];
    for (row, col) in pixels {
        bits[row * SIZE + col] = value;
    }
    bits
}

fn first_plane_key(path: &std::path::Path) -> String {
    format!("{}#hdu=1", path.to_str().unwrap())
}

pub fn afw_star_mef(path: &std::path::Path, error: TestHdu) -> String {
    let mut mask = vec![0i32; SIZE * SIZE];
    for row in AFW_DETECTED_BLOCK {
        for col in AFW_DETECTED_BLOCK {
            mask[row * SIZE + col] = 32;
        }
    }
    mask[AFW_BAD_PIXEL.0 * SIZE + AFW_BAD_PIXEL.1] |= 1;
    mask[AFW_SATURATED_PIXEL.0 * SIZE + AFW_SATURATED_PIXEL.1] |= 2;
    let planes = vec![("MP_BAD", "0".to_string()), ("MP_SAT", "1".to_string()), ("HIERARCH MP_DETECTED", "5".to_string())];
    write_test_mef(path, &[], &[star("IMAGE"), layer("MASK", HduData::I32(mask), planes), error]);
    first_plane_key(path)
}

pub fn inverse_variance_star_mef(path: &std::path::Path) -> String {
    let mut inverse_variance = vec![0.25f32; SIZE * SIZE];
    for (row, col) in NO_DATA_PIXELS {
        inverse_variance[row * SIZE + col] = 0.0;
    }
    write_test_mef(path, &[], &[star("SCI"), layer("IVAR", HduData::F32(inverse_variance), vec![])]);
    first_plane_key(path)
}

pub fn unnamed_flag_star_mef(path: &std::path::Path) -> String {
    write_test_mef(path, &[], &[star("SCI"), layer("FLAG", HduData::I32(flagged(&[UNNAMED_FLAG_PIXEL], 1 << 24)), vec![])]);
    first_plane_key(path)
}

pub const LEGACY_BRICK_IMAGE: &str = "legacysurvey-0001m001-image-r.fits";
pub const LEGACY_BRICK_INVVAR: &str = "legacysurvey-0001m001-invvar-r.fits";
pub const CUTOUT_PLANES: usize = 3;
pub const CUTOUT_STAR_PLANE: usize = 1;

pub fn legacy_brick(dir: &std::path::Path, invvar_size: Option<usize>) -> String {
    let star = ndarray::Array2::from_shape_vec((SIZE, SIZE), gaussian_pixels(SIZE, 1000.0, 2.0, 100.0)).unwrap();
    let mut header = HduHeader::empty();
    header.set("BUNIT", "nanomaggy".to_string());
    let image = dir.join(LEGACY_BRICK_IMAGE).to_str().unwrap().to_string();
    write_fits_mono(&image, &star, Some(&header)).unwrap();
    if let Some(size) = invvar_size {
        let weights = ndarray::Array2::from_elem((size, size), 0.25f32);
        write_fits_mono(dir.join(LEGACY_BRICK_INVVAR).to_str().unwrap(), &weights, None).unwrap();
    }
    image
}

pub fn legacy_band_cutout(path: &std::path::Path, error_size: usize) -> String {
    let image_cards: Vec<(&'static str, String)> = vec![
        ("SIMPLE", "T".into()),
        ("BITPIX", "-32".into()),
        ("NAXIS", "3".into()),
        ("NAXIS1", SIZE.to_string()),
        ("NAXIS2", SIZE.to_string()),
        ("NAXIS3", CUTOUT_PLANES.to_string()),
        ("EXTEND", "T".into()),
        ("IMAGETYP", "'IMAGE   '".into()),
        ("BANDS", "'grz     '".into()),
        ("BAND0", "'g       '".into()),
        ("BAND1", "'r       '".into()),
        ("BAND2", "'z       '".into()),
        ("SURVEY", "'LegacySurvey'".into()),
    ];
    let star = gaussian_pixels(SIZE, 1000.0, 2.0, 100.0);
    let image: Vec<u8> = (0..CUTOUT_PLANES)
        .flat_map(|plane| if plane == CUTOUT_STAR_PLANE { star.clone() } else { vec![100.0f32; SIZE * SIZE] })
        .flat_map(f32::to_be_bytes)
        .collect();
    let invvar_cards: Vec<(&'static str, String)> = vec![
        ("XTENSION", "'IMAGE   '".into()),
        ("BITPIX", "-32".into()),
        ("NAXIS", "3".into()),
        ("NAXIS1", error_size.to_string()),
        ("NAXIS2", error_size.to_string()),
        ("NAXIS3", CUTOUT_PLANES.to_string()),
        ("PCOUNT", "0".into()),
        ("GCOUNT", "1".into()),
        ("IMAGETYP", "'INVVAR  '".into()),
    ];
    let weights: Vec<u8> = vec![0.25f32; error_size * error_size * CUTOUT_PLANES].into_iter().flat_map(f32::to_be_bytes).collect();
    write_raw_hdus(path, &[(image_cards, image), (invvar_cards, weights)]);
    BandPlaneKey::new(path.to_str().unwrap(), 0, CUTOUT_STAR_PLANE).key()
}
