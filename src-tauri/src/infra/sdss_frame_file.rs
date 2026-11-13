use std::fs::File;

use anyhow::{bail, Context, Result};
use ndarray::Array2;

use crate::core::imaging::sdss_frame::{sdss_frame_sigma, sdss_sky_image};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::file_bytes::read_file_bytes;
use crate::infra::fits::reader::{decode_pixels, extract_header_by_index, list_extensions, parse_header_at, HduInfo};
use crate::infra::fits::table::{describe_table, is_bintable_hdu, read_columns, Column, ColumnValues};
use crate::types::image_ref::ImageRef;
use crate::types::HduHeader;

pub const SDSS_SIGMA_PREFIX: &str = "__sdss_sigma";
pub const SDSS_NO_ERROR_REASON: &str = "SDSS frame: the file carries no error plane; it can be rebuilt from the calibration vector (HDU 1) and the sky table (HDU 2) once the gain and the dark variance are typed";
pub const CARD_SDSS_GAIN: &str = "ABGAIN";
pub const CARD_SDSS_DARK_VARIANCE: &str = "ABDARKV";

const SKY_TABLE_HDU: usize = 2;
const SKY_COLUMNS: [&str; 3] = ["ALLSKY", "XINTERP", "YINTERP"];

#[derive(Debug, Clone, PartialEq)]
pub struct SdssFrameParts {
    pub calib: Vec<f32>,
    pub allsky: Array2<f32>,
    pub xinterp: Vec<f32>,
    pub yinterp: Vec<f32>,
}

fn too_few_hdus(found: usize) -> String {
    format!("the file has no calibration vector and no sky table (an SDSS frame has three HDUs or more, this one has {found})")
}

fn is_calibration_vector(image: &HduInfo, vector: &HduInfo) -> bool {
    vector.naxis == 1 && vector.naxis1 == image.naxis1
}

fn is_sky_table(header: &HduHeader) -> bool {
    is_bintable_hdu(header)
        && describe_table(header, SKY_TABLE_HDU).is_ok_and(|table| {
            SKY_COLUMNS.iter().all(|wanted| table.columns.iter().any(|column| column.name == *wanted))
        })
}

pub fn sdss_frame_refusal(exts: &[HduInfo], sky_table: Option<&HduHeader>) -> Option<String> {
    let [image, vector, _, ..] = exts else {
        return Some(too_few_hdus(exts.len()));
    };
    if image.naxis != 2 || !image.has_data {
        return Some("HDU 0 is not a 2D image".to_string());
    }
    if !is_calibration_vector(image, vector) {
        return Some(format!(
            "HDU 1 is not the calibration vector (a 1D array with one value per image column, {} expected)",
            image.naxis1
        ));
    }
    if !sky_table.is_some_and(is_sky_table) {
        return Some(
            "HDU 2 is not the sky table (a binary table with the columns ALLSKY, XINTERP and YINTERP)".to_string(),
        );
    }
    None
}

pub fn sdss_frame_file_refusal(file: &File, exts: &[HduInfo]) -> Option<String> {
    sdss_frame_refusal(exts, extract_header_by_index(file, SKY_TABLE_HDU).ok().as_ref())
}

pub fn looks_like_sdss_frame(file: &File, exts: &[HduInfo]) -> bool {
    let [image, vector, _, ..] = exts else {
        return false;
    };
    image.naxis == 2 && is_calibration_vector(image, vector) && sdss_frame_file_refusal(file, exts).is_none()
}

fn single_precision(column: &Column) -> Vec<f32> {
    match &column.values {
        ColumnValues::F64(values) => values.iter().map(|&value| value as f32).collect(),
        ColumnValues::I64(values) => values.iter().map(|&value| value as f32).collect(),
    }
}

fn sky_grid(allsky: &Column) -> Result<Array2<f32>> {
    let values = single_precision(allsky);
    let shape = match allsky.tdim.as_deref() {
        Some(&[columns, rows]) if columns * rows == values.len() => (rows, columns),
        _ => bail!("the sky table has no TDIM card for ALLSKY"),
    };
    Ok(Array2::from_shape_vec(shape, values)?)
}

pub fn read_sdss_frame_parts(path: &str) -> Result<SdssFrameParts> {
    let (source, _decompressed) = resolve_single_image(&ImageRef::parse(path).path)?;
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    let exts = list_extensions(&file)?;
    let bytes = read_file_bytes(&file)?;
    let sky_table = exts.get(SKY_TABLE_HDU).map(|hdu| parse_header_at(&bytes, hdu.header_start)).transpose()?;
    if let Some(refusal) = sdss_frame_refusal(&exts, sky_table.as_ref().map(|hdu| &hdu.header)) {
        bail!(refusal);
    }
    let (Some(sky_table), [_, vector, ..]) = (sky_table, exts.as_slice()) else {
        bail!(too_few_hdus(exts.len()));
    };
    let vector_header = parse_header_at(&bytes, vector.header_start)?.header;
    let vector_bytes = vector.naxis1 as usize * (vector.bitpix.unsigned_abs() as usize / 8);
    let calib = decode_pixels(
        bytes
            .get(vector.data_start..vector.data_start + vector_bytes)
            .context("the calibration vector is cut short")?,
        vector.bitpix,
        vector_header.get_f64("BSCALE").unwrap_or(1.0),
        vector_header.get_f64("BZERO").unwrap_or(0.0),
    );
    let columns = read_columns(&bytes[sky_table.data_start..], &sky_table.header, &SKY_COLUMNS)?;
    Ok(SdssFrameParts {
        calib,
        allsky: sky_grid(&columns[0])?,
        xinterp: single_precision(&columns[1]),
        yinterp: single_precision(&columns[2]),
    })
}

pub fn sdss_frame_sigma_plane(path: &str, image: &Array2<f32>, gain: f64, dark_variance: f64) -> Result<Array2<f32>> {
    let parts = read_sdss_frame_parts(path)?;
    let sky = sdss_sky_image(&parts.allsky, &parts.xinterp, &parts.yinterp).map_err(anyhow::Error::msg)?;
    sdss_frame_sigma(image, &parts.calib, &sky, gain, dark_variance).map_err(anyhow::Error::msg)
}

pub fn sdss_sigma_ref(path: &str, gain: f64, dark_variance: f64) -> ImageRef {
    ImageRef::array(path, &format!("{SDSS_SIGMA_PREFIX}.g{gain}.d{dark_variance}"))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    use ndarray::Array2;

    use crate::infra::fits::reader::test_fixtures::write_raw_hdus;
    use crate::infra::fits::table::test_support::bintable_cards;

    pub(crate) type RawHdu = (Vec<(&'static str, String)>, Vec<u8>);

    pub(crate) const FRAME_COLS: usize = 4;
    pub(crate) const FRAME_ROWS: usize = 3;
    pub(crate) const FRAME_CALIB: [f32; 4] = [0.005, 0.004, 0.005, 0.01];
    pub(crate) const FRAME_XINTERP: [f32; 4] = [0.0, 0.5, 1.25, 2.0];
    pub(crate) const NARROW_XINTERP: [f32; 3] = [0.0, 0.5, 1.25];
    pub(crate) const FRAME_YINTERP: [f32; 3] = [0.0, 0.25, 1.0];
    pub(crate) const FRAME_ALLSKY: [f32; 6] = [0.0, 1.0, 2.0, 10.0, 11.0, 12.0];
    pub(crate) const SIGMA_GAIN_4_DARK_1: [f64; 12] = [
        0.005,
        0.004242640687119285,
        f64::NAN,
        0.01224744871391589,
        0.006373774391990981,
        0.006928203230275509,
        0.006959705453537527,
        0.014577379737113252,
        0.009354143466934854,
        0.007615773105863909,
        0.009762812094883317,
        0.01,
    ];
    pub(crate) const SIGMA_GAIN_2_DARK_0: [f64; 12] = [
        0.0,
        0.002,
        f64::NAN,
        0.01,
        0.005590169943749474,
        0.008,
        0.006846531968814577,
        0.015,
        0.011180339887498949,
        0.00916515138991168,
        0.011858541225631423,
        0.0,
    ];
    const SIGMA_RELATIVE_TOLERANCE: f64 = 1e-6;

    fn big_endian(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|value| value.to_be_bytes()).collect()
    }

    pub(crate) fn frame_image() -> Array2<f32> {
        let mut image = Array2::<f32>::zeros((FRAME_ROWS, FRAME_COLS));
        image[[0, 2]] = f32::NAN;
        image[[1, 1]] = 0.02;
        image[[2, 3]] = -1.0;
        image
    }

    pub(crate) fn image_hdu() -> RawHdu {
        let cards = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", FRAME_COLS.to_string()),
            ("NAXIS2", FRAME_ROWS.to_string()),
            ("EXTEND", "T".into()),
            ("BUNIT", "'nanomaggy'".into()),
        ];
        (cards, big_endian(frame_image().as_slice().unwrap()))
    }

    pub(crate) fn calib_hdu(values: &[f32]) -> RawHdu {
        let cards = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "1".into()),
            ("NAXIS1", values.len().to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
        ];
        (cards, big_endian(values))
    }

    pub(crate) fn sky_hdu(columns: &[(&str, &[f32])]) -> RawHdu {
        let forms: Vec<String> = columns.iter().map(|(_, values)| format!("{}E", values.len())).collect();
        let described: Vec<(&str, &str, Option<&str>)> =
            columns.iter().zip(&forms).map(|((name, _), form)| (*name, form.as_str(), None)).collect();
        let cards = bintable_cards(&described, 1, &[("EXTNAME", "'SKY     '".into()), ("TDIM1", "'(3,2)'".into())]);
        (cards, columns.iter().flat_map(|(_, values)| big_endian(values)).collect())
    }

    pub(crate) fn write_hdus(dir: &Path, name: &str, hdus: &[RawHdu]) -> String {
        let path = dir.join(name);
        write_raw_hdus(&path, hdus);
        path.to_str().unwrap().to_string()
    }

    pub(crate) fn write_frame(dir: &Path, name: &str, xinterp: &[f32]) -> String {
        let sky = sky_hdu(&[("ALLSKY", &FRAME_ALLSKY), ("XINTERP", xinterp), ("YINTERP", &FRAME_YINTERP)]);
        write_hdus(dir, name, &[image_hdu(), calib_hdu(&FRAME_CALIB), sky])
    }

    pub(crate) fn assert_sigma_plane(found: &Array2<f32>, expected: &[f64; 12]) {
        assert_eq!(found.dim(), (FRAME_ROWS, FRAME_COLS));
        for ((pixel, value), wanted) in found.indexed_iter().zip(expected) {
            let value = f64::from(*value);
            let agrees = if wanted.is_nan() {
                value.is_nan()
            } else if *wanted == 0.0 {
                value == 0.0
            } else {
                ((value - wanted) / wanted).abs() <= SIGMA_RELATIVE_TOLERANCE
            };
            assert!(agrees, "pixel {pixel:?}: {value} against {wanted} nanomaggy");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{
        assert_sigma_plane, calib_hdu, frame_image, image_hdu, sky_hdu, write_frame, write_hdus, FRAME_ALLSKY,
        FRAME_CALIB, FRAME_XINTERP, FRAME_YINTERP, NARROW_XINTERP, SIGMA_GAIN_4_DARK_1,
    };
    use super::*;
    use crate::infra::fits::reader::{extract_header_by_index, list_extensions};
    use crate::infra::fits::writer::write_fits_mono;

    const TOO_FEW_HDUS: &str =
        "the file has no calibration vector and no sky table (an SDSS frame has three HDUs or more, this one has 2)";
    const NOT_THE_CALIBRATION_VECTOR: &str =
        "HDU 1 is not the calibration vector (a 1D array with one value per image column, 4 expected)";
    const NOT_THE_SKY_TABLE: &str =
        "HDU 2 is not the sky table (a binary table with the columns ALLSKY, XINTERP and YINTERP)";
    const NOT_A_2D_IMAGE: &str = "HDU 0 is not a 2D image";

    fn refusal_of(path: &str) -> Option<String> {
        let file = File::open(path).unwrap();
        let exts = list_extensions(&file).unwrap();
        let sky_table = extract_header_by_index(&file, 2).ok();
        assert_eq!(looks_like_sdss_frame(&file, &exts), sdss_frame_refusal(&exts, sky_table.as_ref()).is_none());
        sdss_frame_refusal(&exts, sky_table.as_ref())
    }

    fn default_sky() -> super::test_support::RawHdu {
        sky_hdu(&[("ALLSKY", &FRAME_ALLSKY), ("XINTERP", &FRAME_XINTERP), ("YINTERP", &FRAME_YINTERP)])
    }

    #[test]
    fn a_frame_is_told_by_its_three_hdus() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        let file = File::open(&frame).unwrap();
        let exts = list_extensions(&file).unwrap();
        assert_eq!(sdss_frame_refusal(&exts, Some(&extract_header_by_index(&file, 2).unwrap())), None);
        assert!(looks_like_sdss_frame(&file, &exts));
        assert_eq!(sdss_frame_refusal(&exts, None).as_deref(), Some(NOT_THE_SKY_TABLE));

        let two = write_hdus(dir.path(), "two.fits", &[image_hdu(), calib_hdu(&FRAME_CALIB)]);
        assert_eq!(refusal_of(&two).as_deref(), Some(TOO_FEW_HDUS));

        let short = write_hdus(dir.path(), "short.fits", &[image_hdu(), calib_hdu(&FRAME_CALIB[..3]), default_sky()]);
        assert_eq!(refusal_of(&short).as_deref(), Some(NOT_THE_CALIBRATION_VECTOR));

        let two_columns = sky_hdu(&[("ALLSKY", &FRAME_ALLSKY), ("XINTERP", &FRAME_XINTERP)]);
        let partial = write_hdus(dir.path(), "partial.fits", &[image_hdu(), calib_hdu(&FRAME_CALIB), two_columns]);
        assert_eq!(refusal_of(&partial).as_deref(), Some(NOT_THE_SKY_TABLE));

        let (mut cube_cards, pixels) = image_hdu();
        for (key, value) in cube_cards.iter_mut() {
            if *key == "NAXIS" {
                *value = "3".to_string();
            }
        }
        cube_cards.insert(5, ("NAXIS3", "2".into()));
        let cube = (cube_cards, [pixels.clone(), pixels].concat());
        let cube = write_hdus(dir.path(), "cube.fits", &[cube, calib_hdu(&FRAME_CALIB), default_sky()]);
        assert_eq!(refusal_of(&cube).as_deref(), Some(NOT_A_2D_IMAGE));

        let mono = dir.path().join("mono.fits").to_str().unwrap().to_string();
        write_fits_mono(&mono, &frame_image(), None).unwrap();
        let file = File::open(&mono).unwrap();
        assert!(!looks_like_sdss_frame(&file, &list_extensions(&file).unwrap()));
    }

    #[test]
    fn the_parts_are_read_from_hdu_1_and_hdu_2() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        let parts = read_sdss_frame_parts(&frame).unwrap();
        assert_eq!(parts.calib, FRAME_CALIB.to_vec());
        assert_eq!(parts.allsky, Array2::from_shape_vec((2, 3), FRAME_ALLSKY.to_vec()).unwrap());
        assert_eq!(parts.xinterp, FRAME_XINTERP.to_vec());
        assert_eq!(parts.yinterp, FRAME_YINTERP.to_vec());

        let two = write_hdus(dir.path(), "two.fits", &[image_hdu(), calib_hdu(&FRAME_CALIB)]);
        assert_eq!(format!("{:#}", read_sdss_frame_parts(&two).unwrap_err()), TOO_FEW_HDUS);
    }

    #[test]
    fn the_sigma_plane_follows_the_recipe_of_the_frame() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        let sigma = sdss_frame_sigma_plane(&frame, &frame_image(), 4.0, 1.0).unwrap();
        assert_sigma_plane(&sigma, &SIGMA_GAIN_4_DARK_1);
        assert!(sigma[[0, 2]].is_nan());
        assert_eq!(sigma[[2, 3]], 0.01);
    }

    #[test]
    fn a_sky_table_that_does_not_fit_the_image_is_a_message() {
        let dir = tempfile::tempdir().unwrap();
        let narrow = write_frame(dir.path(), "narrow.fits", &NARROW_XINTERP);
        let refused = format!("{:#}", sdss_frame_sigma_plane(&narrow, &frame_image(), 4.0, 1.0).unwrap_err());
        assert!(refused.contains("the sky image is 3 x 3 but the frame is 4 x 3"), "{refused}");

        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        let refused = format!("{:#}", sdss_frame_sigma_plane(&frame, &frame_image(), 0.0, 1.0).unwrap_err());
        assert!(refused.contains("gain = 0 e-/DN must be a positive number"), "{refused}");
    }

    #[test]
    fn the_sigma_reference_names_the_noise_values() {
        let key = sdss_sigma_ref("C:/d/frame.fits", 4.76, 1.3225).cache_key();
        assert_eq!(key, "C:/d/frame.fits#array=__sdss_sigma.g4.76.d1.3225");
        assert_eq!(ImageRef::parse(&key).path, "C:/d/frame.fits");
        assert!(sdss_sigma_ref("C:/d/frame.fits", 5.0, 0.0).cache_key().ends_with("__sdss_sigma.g5.d0"));
    }
}
