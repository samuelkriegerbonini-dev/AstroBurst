use anyhow::{anyhow, bail, Result};
use fitsio_pure::hdu::{Hdu, HduInfo};
use fitsio_pure::header::Card;
use fitsio_pure::image::{serialize_image, ImageData};
use fitsio_pure::tiled::read_tiled_image;
use fitsio_pure::value::Value;

use crate::types::header::parse_fits_float;
use crate::types::image::IntPlane;
use crate::types::HduHeader;

use super::reader::{decode_pixels_int, int_pixels_to_plane};

const QUANTISATION_NAMES: [&str; 2] = ["ZSCALE", "ZZERO"];

fn names_quantisation(name: &str) -> bool {
    QUANTISATION_NAMES.iter().any(|known| name.trim().eq_ignore_ascii_case(known))
}

fn is_column_name_key(key: &str) -> bool {
    key.strip_prefix("TTYPE")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn quantisation(header: &HduHeader) -> Option<String> {
    header.cards.iter().find_map(|(key, value)| {
        (names_quantisation(key) || (is_column_name_key(key) && names_quantisation(value)))
            .then(|| format!("{key} = {}", value.trim()))
    })
}

fn hcompress_scale(header: &HduHeader) -> Option<f64> {
    header.cards.iter().find_map(|(key, name)| {
        let n = key.strip_prefix("ZNAME")?;
        if !name.trim().eq_ignore_ascii_case("SCALE") {
            return None;
        }
        header.get_f64(&format!("ZVAL{n}"))
    })
}

pub(super) fn refuse_unless_lossless_integers(header: &HduHeader, hdu_index: usize) -> Result<()> {
    let zbitpix = header.get("ZBITPIX").map(str::trim);
    match zbitpix.and_then(|raw| raw.parse::<i64>().ok()) {
        Some(8 | 16 | 32) => {}
        Some(float @ (-32 | -64)) => bail!(
            "HDU {hdu_index} is a tile-compressed floating-point image (ZBITPIX = {float}); it has no lossless integer form"
        ),
        Some(64) => bail!(
            "HDU {hdu_index} is a tile-compressed 64-bit integer image (ZBITPIX = 64); integer planes are read exactly only for 8, 16 and 32 bits"
        ),
        _ => bail!(
            "HDU {hdu_index} has ZBITPIX = {}, which is not an 8, 16 or 32 bit integer image",
            zbitpix.unwrap_or("missing")
        ),
    }
    if let Some(what) = quantisation(header) {
        bail!("HDU {hdu_index} is an integer image with quantisation ({what}); its tiles are not lossless");
    }
    if header.get("ZCMPTYPE").is_some_and(|codec| codec.trim() == "HCOMPRESS_1") {
        if let Some(scale) = hcompress_scale(header).filter(|scale| *scale != 0.0) {
            bail!(
                "HDU {hdu_index} is HCOMPRESS_1 with SCALE = {scale}: that compression is lossy, so its integers are not the stored values"
            );
        }
    }
    Ok(())
}

fn decoder_cards(header: &HduHeader) -> Vec<Card> {
    header
        .cards
        .iter()
        .filter_map(|(key, raw)| {
            let text = raw.trim();
            let value = if header.is_string_value(key) == Some(true) {
                Value::String(raw.clone())
            } else if let Ok(integer) = text.parse::<i64>() {
                Value::Integer(integer)
            } else if text == "T" || text == "F" {
                Value::Logical(text == "T")
            } else if let Some(float) = parse_fits_float(text) {
                Value::Float(float)
            } else {
                Value::String(raw.clone())
            };
            Card::new(key, value).ok()
        })
        .collect()
}

pub(super) fn decode_int_planes(
    mmap: &[u8],
    header: &HduHeader,
    data_start: usize,
    hdu_index: usize,
    bscale: f64,
    bzero: f64,
) -> Result<Vec<IntPlane>> {
    let failed = |error: fitsio_pure::Error| {
        anyhow!("HDU {hdu_index}: decoding the tile-compressed integer image failed: {error}")
    };
    let hdu = Hdu::from_cards(decoder_cards(header), data_start, data_start).map_err(failed)?;
    let HduInfo::CompressedImage { znaxes, .. } = &hdu.info else {
        bail!("HDU {hdu_index}: decoding the tile-compressed integer image failed: the header does not describe a tile-compressed image");
    };
    let axis = |n: usize| znaxes.get(n).copied().unwrap_or(1);
    let (cols, rows) = (axis(0), axis(1));
    let planes: usize = znaxes.iter().skip(2).product();
    let data = read_tiled_image(mmap, &hdu).map_err(failed)?;
    let (zbitpix, values): (i64, usize) = match &data {
        ImageData::U8(pixels) => (8, pixels.len()),
        ImageData::I16(pixels) => (16, pixels.len()),
        ImageData::I32(pixels) => (32, pixels.len()),
        _ => bail!("HDU {hdu_index}: decoding the tile-compressed integer image failed: the decoder returned pixels that are not 8, 16 or 32 bit integers"),
    };
    if values != planes * rows * cols {
        bail!("HDU {hdu_index}: the decoder returned {values} values for {planes} planes of {cols} x {rows}");
    }
    let bytes = serialize_image(&data);
    let plane_bytes = rows * cols * zbitpix.unsigned_abs() as usize / 8;
    (0..planes)
        .map(|z| {
            let plane = &bytes[z * plane_bytes..][..plane_bytes];
            int_pixels_to_plane(decode_pixels_int(plane, zbitpix, bscale, bzero)?, rows, cols)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs::File;
    use std::path::{Path, PathBuf};

    use sha2::{Digest, Sha256};

    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::core::imaging::dq_flags::DqTable;
    use crate::infra::cache::ImageCache;
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, sci_err_dq_mef, write_raw_hdus};
    use crate::infra::fits::reader::{extract_int_plane_by_index, extract_int_planes_by_index, load_fits_image};
    use crate::infra::image_source::load_plane;
    use crate::service::common::load_companions;
    use crate::types::image::IntPlane;
    use crate::types::ImageRef;

    const SYNTHETIC_SHAPE: (usize, usize) = (23, 37);

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("fits_compressed")
            .join(name)
    }

    fn fixture_plane(name: &str) -> IntPlane {
        extract_int_plane_by_index(&File::open(fixture(name)).unwrap(), 1).unwrap()
    }

    fn values(plane: &IntPlane) -> impl Iterator<Item = i64> + '_ {
        let (rows, cols) = plane.bits.dim();
        (0..rows).flat_map(move |y| (0..cols).map(move |x| plane.value_at(y, x)))
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn sha256_of_little_endian_words(plane: &IntPlane) -> String {
        sha256_hex(&plane.bits.iter().flat_map(|bits| bits.to_le_bytes()).collect::<Vec<u8>>())
    }

    fn sha256_of_single_bytes(plane: &IntPlane) -> String {
        sha256_hex(&plane.bits.iter().map(|&bits| bits as u8).collect::<Vec<u8>>())
    }

    fn assert_plane(name: &str, plane: &IntPlane, signed: bool, sum: i64, at: &[(usize, usize)], samples: &[i64]) {
        assert_eq!(plane.signed, signed, "{name}: signed");
        assert_eq!(plane.bits.dim(), SYNTHETIC_SHAPE, "{name}: shape (rows, columns)");
        assert_eq!(values(plane).sum::<i64>(), sum, "{name}: sum of the values");
        let read: Vec<i64> = at.iter().map(|&(y, x)| plane.value_at(y, x)).collect();
        assert_eq!(read, samples, "{name}: values at {at:?}");
    }

    #[test]
    fn a_tile_compressed_dq_companion_keeps_its_integer_plane() {
        let path = fixture("mef_sci_rice_dq.fits");
        let text = path.to_str().unwrap();

        let plane = extract_int_plane_by_index(&File::open(&path).unwrap(), 2).unwrap();
        assert!(!plane.signed);
        assert_eq!(plane.bits.dim(), (23, 37));
        assert_eq!(plane.bits[[0, 0]], 2147483649);
        assert_eq!(plane.bits[[1, 2]], 1);
        assert_eq!(plane.bits[[3, 4]], 4);
        assert_eq!(plane.bits[[11, 18]], 1090519045);
        assert_eq!(plane.bits[[22, 36]], 4294967295);
        assert_eq!(plane.bits.iter().filter(|&&bits| bits != 0).count(), 5);
        assert_eq!(plane.bits.iter().map(|&bits| u64::from(bits)).sum::<u64>(), 7_532_969_994);

        assert!(load_plane(&ImageRef::hdu(text, 2)).unwrap().int_plane.is_some());

        let companions = load_companions(&ImageCache::new(8, 1 << 30), text).unwrap();
        let Some((entry, DqTable::Jwst)) = companions.dq else {
            panic!("the tile-compressed DQ companion was dropped or lost its JWST table");
        };
        assert_eq!(entry.int_plane().unwrap().bits[[0, 0]], 2147483649);
    }

    #[test]
    fn rice_gzip_and_nocompress_tiles_decode_to_the_integers_astropy_reads() {
        const CORNERS_AND_INTERIOR: [(usize, usize); 6] = [(0, 0), (0, 36), (22, 0), (22, 36), (1, 2), (11, 18)];
        const THIRTY_TWO_BIT_SHA256: &str = "98bd3d8d737a734ed620b5c8b2bb4be4c448b914649413163a453db1f20793f1";
        for name in ["rice_i16_default_tile.fits", "rice_i16_2d_tiles.fits", "gzip1_i16.fits", "gzip2_i16_2d_tiles.fits"] {
            assert_plane(name, &fixture_plane(name), true, 261_136, &CORNERS_AND_INTERIOR, &[13, 114, 152, 266, 27, 119]);
        }
        for name in ["rice_i32_default_tile.fits", "gzip2_i32.fits"] {
            let plane = fixture_plane(name);
            assert_plane(name, &plane, true, 111_456_500, &CORNERS_AND_INTERIOR, &[6500, 111000, 153000, 264000, 20000, 125000]);
            assert_eq!(sha256_of_little_endian_words(&plane), THIRTY_TWO_BIT_SHA256, "{name}: SHA-256 of the little-endian i32 values");
        }
        let name = "rice_u8_default_tile.fits";
        assert_plane(name, &fixture_plane(name), false, 90_752, &CORNERS_AND_INTERIOR, &[13, 114, 152, 66, 27, 119]);

        const WRITTEN_SAMPLES: [(usize, usize); 6] = [(0, 0), (0, 1), (1, 2), (3, 4), (11, 18), (22, 36)];
        let name = "nocompress_i16.fits";
        assert_plane(name, &fixture_plane(name), true, -1_398_075, &WRITTEN_SAMPLES, &[-10000, -9969, -8791, -6435, 3175, -3650]);
        let name = "rice_u16_bzero.fits";
        assert_plane(name, &fixture_plane(name), false, 27_947_201, &WRITTEN_SAMPLES, &[65535, 32768, 3003, 8855, 32725, 65450]);
    }

    #[test]
    fn hcompress_and_plio_tiles_decode_exactly_above_two_to_the_24() {
        const WRITTEN_SAMPLES: [(usize, usize); 6] = [(0, 0), (0, 1), (1, 2), (3, 4), (11, 18), (22, 36)];

        let name = "hcompress_i32_flags.fits";
        let plane = fixture_plane(name);
        assert_plane(name, &plane, true, 1_117_607_129, &WRITTEN_SAMPLES, &[25296896, 2481, 16777217, 2691, 1073741825, 53246]);
        assert_eq!(
            sha256_of_little_endian_words(&plane),
            "6dd0e1d16d161a1e433ee03615b2eff82f69b8bbc55fd7d9dbc5fb973a4cdd49",
            "{name}: SHA-256 of the little-endian i32 values"
        );

        let name = "hcompress_u8.fits";
        let plane = fixture_plane(name);
        assert_plane(name, &plane, false, 105_308, &WRITTEN_SAMPLES, &[0, 7, 22, 52, 214, 177]);
        assert_eq!(
            sha256_of_single_bytes(&plane),
            "689d69ba9b282a826661dae2fbe734e2293cbc792168408a616b88dead2c64e8",
            "{name}: SHA-256 of the u8 values"
        );

        let name = "plio_i32.fits";
        let plane = fixture_plane(name);
        assert_plane(name, &plane, true, 18_873_535, &WRITTEN_SAMPLES, &[0, 97, 3783, 16777215, 1225, 2450]);
        assert_eq!(
            sha256_of_little_endian_words(&plane),
            "626f6676af3a13b88e6a58f54aa7d41ede249e06faac0d8ecf824b93389af29b",
            "{name}: SHA-256 of the little-endian i32 values"
        );

        let through_f32 = load_fits_image(fixture("hcompress_i32_flags.fits").to_str().unwrap()).unwrap();
        assert_eq!(through_f32[[1, 2]], 16777216.0, "an f32 plane cannot hold 16777217");
    }

    #[test]
    fn the_plural_reader_returns_one_plane_for_a_compressed_image() {
        let file = File::open(fixture("rice_i32_default_tile.fits")).unwrap();
        let planes = extract_int_planes_by_index(&file, 1).unwrap();
        let single = extract_int_plane_by_index(&file, 1).unwrap();
        assert_eq!(planes.len(), 1);
        assert_eq!(planes[0].signed, single.signed);
        assert_eq!(planes[0].bits, single.bits);
    }

    fn tile_table_cards(zbitpix: i64, znaxes: &[usize]) -> Vec<(&'static str, String)> {
        const ZAXIS_KEYS: [&str; 3] = ["ZNAXIS1", "ZNAXIS2", "ZNAXIS3"];
        let mut cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "8".into()),
            ("NAXIS2", "3".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("TTYPE1", "'COMPRESSED_DATA'".into()),
            ("TFORM1", "'1PB     '".into()),
            ("ZIMAGE", "T".into()),
            ("ZCMPTYPE", "'RICE_1  '".into()),
            ("ZBITPIX", zbitpix.to_string()),
            ("ZNAXIS", znaxes.len().to_string()),
        ];
        cards.extend(ZAXIS_KEYS.iter().zip(znaxes).map(|(key, length)| (*key, length.to_string())));
        cards.push(("EXTNAME", "'SCI     '".into()));
        cards
    }

    fn refusal_of_cards(dir: &Path, name: &str, cards: Vec<(&'static str, String)>) -> String {
        let path = dir.join(name);
        write_raw_hdus(&path, &[(empty_primary_cards(), Vec::new()), (cards, vec![0u8; 24])]);
        refusal(&path, 1)
    }

    fn refusal(path: &Path, hdu: usize) -> String {
        format!("{:#}", extract_int_plane_by_index(&File::open(path).unwrap(), hdu).unwrap_err())
    }

    #[test]
    fn planes_without_a_lossless_integer_form_are_refused_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();

        let float = refusal(&fixture("rice_f32_quantized.fits"), 1);
        assert!(float.contains("floating-point") && float.contains("ZBITPIX = -32"), "{float}");

        let wide = refusal_of_cards(dir.path(), "i64.fits", tile_table_cards(64, &[4, 3]));
        assert!(wide.contains("64-bit"), "{wide}");

        let odd = refusal_of_cards(dir.path(), "i24.fits", tile_table_cards(24, &[4, 3]));
        assert_eq!(odd, "HDU 1 has ZBITPIX = 24, which is not an 8, 16 or 32 bit integer image");

        let mut untyped_cards = tile_table_cards(16, &[4, 3]);
        untyped_cards.retain(|(key, _)| *key != "ZBITPIX");
        let untyped = refusal_of_cards(dir.path(), "untyped.fits", untyped_cards);
        assert_eq!(untyped, "HDU 1 has ZBITPIX = missing, which is not an 8, 16 or 32 bit integer image");

        let mut quantised_cards = tile_table_cards(16, &[4, 3]);
        quantised_cards.iter_mut().find(|(key, _)| *key == "TFIELDS").unwrap().1 = "2".into();
        quantised_cards.extend([("TTYPE2", "'ZSCALE  '".to_string()), ("TFORM2", "'1D      '".to_string())]);
        let quantised = refusal_of_cards(dir.path(), "quantised.fits", quantised_cards);
        assert!(quantised.contains("quantisation") && quantised.contains("TTYPE2"), "{quantised}");

        let mut scaled_cards = tile_table_cards(16, &[4, 3]);
        scaled_cards.push(("ZZERO", "0.5".into()));
        let scaled = refusal_of_cards(dir.path(), "scaled.fits", scaled_cards);
        assert_eq!(scaled, "HDU 1 is an integer image with quantisation (ZZERO = 0.5); its tiles are not lossless");

        let empty_tiles = refusal_of_cards(dir.path(), "empty_tiles.fits", tile_table_cards(16, &[4, 3]));
        assert!(
            empty_tiles.starts_with("HDU 1: decoding the tile-compressed integer image failed: "),
            "{empty_tiles}"
        );

        let lossy = refusal(&fixture("hcompress_i32_scale2.fits"), 1);
        assert!(lossy.contains("SCALE = 2") && lossy.contains("lossy"), "{lossy}");

        let cube = refusal_of_cards(dir.path(), "cube.fits", tile_table_cards(32, &[4, 3, 5]));
        assert!(cube.contains("3D cube of 5 planes"), "{cube}");

        let out_of_range = refusal(&fixture("rice_i32_default_tile.fits"), 9);
        assert!(out_of_range.contains("HDU index 9 out of range (file has 2 HDUs)"), "{out_of_range}");
    }

    #[test]
    fn an_uncompressed_integer_plane_reads_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dq.fits");
        let mut dq = vec![0i32; 16];
        dq[5] = 1;
        dq[9] = ((2147483648u32 | 1) as i64 - 2147483648) as i32;
        sci_err_dq_mef(&path, 4, 4, dq);
        let file = File::open(&path).unwrap();
        let plane = extract_int_plane_by_index(&file, 3).unwrap();
        assert!(!plane.signed);
        assert_eq!(plane.bits.dim(), (4, 4));
        assert_eq!(plane.bits[[1, 1]], 2147483649);
        assert_eq!(plane.bits[[2, 1]], 2147483649);
        assert_eq!(plane.value_at(2, 1), 2147483649);
        assert_eq!(plane.bits.iter().filter(|&&bits| bits != 2147483648).count(), 2);
        assert!(extract_int_plane_by_index(&file, 1).is_err());
        assert!(extract_int_plane_by_index(&file, 0).is_err());
    }

    fn real(relative: &[&str]) -> PathBuf {
        let path = relative.iter().fold(env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey"), |dir, part| dir.join(part));
        assert!(path.is_file(), "{} is missing", path.display());
        path
    }

    fn real_plane(path: &Path, hdu: usize) -> IntPlane {
        extract_int_plane_by_index(&File::open(path).unwrap(), hdu).unwrap()
    }

    fn non_zero(plane: &IntPlane) -> usize {
        plane.bits.iter().filter(|&&bits| bits != 0).count()
    }

    fn pixels_with_bit(plane: &IntPlane, bit: u32) -> usize {
        plane.bits.iter().filter(|&&bits| bits & (1 << bit) != 0).count()
    }

    fn assert_bit_counts(what: &str, plane: &IntPlane, expected: &[(u32, usize)]) {
        let measured: Vec<(u32, usize)> = expected.iter().map(|&(bit, _)| (bit, pixels_with_bit(plane, bit))).collect();
        println!("{what}: pixels with each bit set {measured:?}");
        assert_eq!(measured, expected, "{what}: pixels with each bit set");
    }

    fn disagreements(mask: &IntPlane, bit: u32, flags: &IntPlane) -> usize {
        mask.bits
            .iter()
            .zip(flags.bits.iter())
            .filter(|(&mask_bits, &flag_bits)| (mask_bits & (1 << bit) != 0) != (flag_bits != 0))
            .count()
    }

    const LEGACY_MASKBITS: &str = "legacysurvey-0532m280-maskbits.fits.fz";
    const LEGACY_SHAPE: (usize, usize) = (3600, 3600);
    const LEGACY_MASK_GOLDEN: &str = "plan/goldens/survey/layers_legacy_l2.json";
    const LEGACY_CATALOGUE_GOLDEN: &str = "plan/goldens/survey/phot_legacy_l2_brick.json (x, y, cat_maskbits)";
    const LEGACY_CATALOGUE_MASKBITS: [(usize, usize, i64); 69] = [
        (140, 3445, 1024), (295, 3461, 1024), (714, 3442, 0), (1984, 3474, 0), (3514, 3505, 0), (155, 3409, 1024), (389, 3240, 0), (368, 2898, 0),
        (625, 3334, 0), (1073, 3399, 0), (987, 3329, 0), (859, 3081, 0), (884, 3082, 0), (893, 2972, 51352), (391, 2527, 18584), (1238, 3378, 0),
        (1284, 3288, 0), (1103, 3021, 51420), (1342, 3367, 0), (1507, 3224, 0), (1456, 3169, 16), (995, 2939, 0), (1149, 2927, 16), (1376, 2958, 0),
        (1497, 3041, 0), (1778, 3230, 0), (1860, 3277, 0), (1748, 3127, 0), (1376, 1505, 0), (2050, 3147, 0), (2301, 3137, 0), (2312, 2948, 51228),
        (2569, 3224, 1040), (2514, 3173, 1024), (2817, 3246, 4096), (2825, 3226, 4096), (2616, 2849, 0), (2637, 2644, 0), (2096, 1740, 16), (3027, 3361, 0),
        (3085, 3366, 0), (2916, 3028, 0), (2818, 2736, 0), (2864, 2638, 16400), (3022, 2743, 0), (2868, 2387, 0), (2966, 2508, 16408), (3093, 2467, 0),
        (3098, 2420, 0), (3012, 2289, 0), (2944, 2055, 0), (2994, 2085, 0), (3202, 2090, 16408), (3056, 1829, 0), (3179, 1758, 0), (1527, 1376, 0),
        (1488, 1358, 0), (1754, 1277, 16400), (2041, 1403, 16400), (2072, 873, 0), (1724, 254, 0), (2725, 1564, 256), (2696, 1359, 0), (2747, 1347, 0),
        (2677, 1054, 0), (2192, 784, 0), (1992, 133, 0), (2669, 464, 0), (2712, 195, 0),
    ];

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-maskbits.fits.fz"]
    fn real_legacy_maskbits_hcompress_planes_decode_to_the_oracle_integers() {
        let golden = LEGACY_MASK_GOLDEN;
        let path = real(&["L2", LEGACY_MASKBITS]);

        let maskbits = real_plane(&path, 1);
        let sum: i64 = values(&maskbits).sum();
        let maximum = values(&maskbits).max().unwrap();
        let distinct = maskbits.bits.iter().collect::<HashSet<_>>().len();
        let sha256 = sha256_of_little_endian_words(&maskbits);
        println!(
            "MASKBITS (HDU 1): shape {:?}, signed {}, {} non-zero pixels, sum {sum}, maximum {maximum}, {distinct} distinct values, SHA-256 {sha256}",
            maskbits.bits.dim(),
            maskbits.signed,
            non_zero(&maskbits)
        );
        assert_eq!(maskbits.bits.dim(), LEGACY_SHAPE, "{golden}");
        assert!(maskbits.signed, "{golden}");
        assert_eq!(non_zero(&maskbits), 7_274_048, "non-zero pixels, {golden}");
        assert_eq!(sum, 10_275_663_306, "sum of the mask values, {golden}");
        assert_eq!(maximum, 53246, "{golden}");
        assert_eq!(distinct, 222, "distinct values, {golden}");
        let samples: Vec<i64> = [(0, 0), (1799, 1799), (1800, 1800), (3599, 3599), (100, 2500), (2500, 100), (90, 90), (86, 86)]
            .iter()
            .map(|&(y, x)| maskbits.value_at(y, x))
            .collect();
        assert_eq!(samples, [1, 0, 0, 1, 1024, 1024, 1024, 1025], "sample pixels, {golden}");
        assert_bit_counts(
            "MASKBITS (HDU 1)",
            &maskbits,
            &[
                (0, 1_183_084), (1, 167_687), (2, 28_628), (3, 74_604), (4, 75_722), (5, 1_638), (6, 6_475), (7, 6_193), (8, 643_503),
                (9, 370_208), (10, 6_334_143), (11, 774_431), (12, 5_957), (13, 0), (14, 91_193), (15, 9_948), (16, 0),
            ],
        );
        assert_eq!(sha256, "d54c4a620cb8896819062c138b903bc6eade8d448968bdad11cc53a9ec9c8bcd", "little-endian i32 values, {golden}");

        let wise_m1 = real_plane(&path, 2);
        let sha256 = sha256_of_single_bytes(&wise_m1);
        let maximum = values(&wise_m1).max().unwrap();
        println!("WISEM1 (HDU 2): signed {}, {} non-zero pixels, maximum {maximum}, SHA-256 {sha256}", wise_m1.signed, non_zero(&wise_m1));
        assert_eq!(wise_m1.bits.dim(), LEGACY_SHAPE, "{golden}");
        assert!(!wise_m1.signed, "{golden}");
        assert_eq!(non_zero(&wise_m1), 643_503, "non-zero pixels, {golden}");
        assert_eq!(maximum, 227, "{golden}");
        assert_bit_counts(
            "WISEM1 (HDU 2)",
            &wise_m1,
            &[(0, 156_804), (1, 163_681), (3, 14_646), (4, 11_178), (5, 455_582), (6, 2_302), (7, 214_590)],
        );
        assert_eq!(sha256, "99f8d166c4f208e9c28670a9e17e71598615e6cb084b46d04604f90d56ba06ee", "u8 values, {golden}");

        let wise_m2 = real_plane(&path, 3);
        let sha256 = sha256_of_single_bytes(&wise_m2);
        println!("WISEM2 (HDU 3): signed {}, {} non-zero pixels, SHA-256 {sha256}", wise_m2.signed, non_zero(&wise_m2));
        assert_eq!(wise_m2.bits.dim(), LEGACY_SHAPE, "{golden}");
        assert!(!wise_m2.signed, "{golden}");
        assert_eq!(non_zero(&wise_m2), 370_208, "non-zero pixels, {golden}");
        assert_bit_counts(
            "WISEM2 (HDU 3)",
            &wise_m2,
            &[(0, 143_708), (1, 118_714), (2, 10_695), (5, 247_319), (6, 1_864), (7, 182_311)],
        );
        assert_eq!(sha256, "a2f15a40628684e8f20591d261a5c7cdb3191671000e38fc538efe2b9c18ddab", "u8 values, {golden}");

        let (against_m1, against_m2) = (disagreements(&maskbits, 8, &wise_m1), disagreements(&maskbits, 9, &wise_m2));
        println!("MASKBITS bit 8 against WISEM1: {against_m1} disagreements; bit 9 against WISEM2: {against_m2} disagreements");
        assert_eq!((against_m1, against_m2), (0, 0), "{golden}");

        let matching = LEGACY_CATALOGUE_MASKBITS
            .iter()
            .filter(|&&(y, x, catalogue)| maskbits.value_at(y, x) == catalogue)
            .count();
        println!("mask equals the catalogue maskbits at {matching} of {} source positions", LEGACY_CATALOGUE_MASKBITS.len());
        assert_eq!(matching, 69, "{LEGACY_CATALOGUE_GOLDEN}");
    }

    struct BrickPlane {
        file: &'static str,
        exact_zeros: usize,
        sum: f64,
        sha256: &'static str,
        samples: &'static [((usize, usize), f32)],
    }

    const BRICK_GOLDEN: &str = "plan/goldens/survey/file_facts.json";
    const BRICK_SUM_RELATIVE_TOLERANCE: f64 = 1e-9;
    const BRICK_PLANES: [BrickPlane; 6] = [
        BrickPlane {
            file: "legacysurvey-0532m280-image-g.fits.fz",
            exact_zeros: 262,
            sum: 69563.50413642128,
            sha256: "adeb9c4c165b5c7be2a836b556d8d791e695a591d1ee2a9d04d74a234fc6d218",
            samples: &[],
        },
        BrickPlane {
            file: "legacysurvey-0532m280-image-r.fits.fz",
            exact_zeros: 2_070,
            sum: 160138.02966698975,
            sha256: "1af6650a8a2c4addac39bc80c27f07cc6ce3ec11bd2ba870a44685a6ca33b9f5",
            samples: &[
                ((0, 0), 0.00016394021804444492),
                ((1799, 1799), -0.0010556620545685291),
                ((3599, 3599), 0.00036353792529553175),
                ((100, 2500), 0.0011159803252667189),
            ],
        },
        BrickPlane {
            file: "legacysurvey-0532m280-image-z.fits.fz",
            exact_zeros: 1_683,
            sum: 185504.19217351265,
            sha256: "38ba27d5a63b23746252e8392c67a60fff54752e72d208029879948e5c88d8b9",
            samples: &[],
        },
        BrickPlane {
            file: "legacysurvey-0532m280-invvar-g.fits.fz",
            exact_zeros: 4_724,
            sum: 25746704118217.51,
            sha256: "339dc001350f3df0cae38081ad8aea523d55dfa86554db040e8d4072d6a6e563",
            samples: &[],
        },
        BrickPlane {
            file: "legacysurvey-0532m280-invvar-r.fits.fz",
            exact_zeros: 14_346,
            sum: 25137446820843.4,
            sha256: "cbbf56a9cc04fb28979d1b009dd3d689a54aff9484b1e3adfd23717a04e152d4",
            samples: &[((0, 0), 1941938.0), ((1799, 1799), 3069190.5), ((3599, 3599), 859190.8125), ((2500, 100), 3158603.75)],
        },
        BrickPlane {
            file: "legacysurvey-0532m280-invvar-z.fits.fz",
            exact_zeros: 14_128,
            sum: 2278855624435.879,
            sha256: "5ce1d2ba07efb9738064229e5a5b475b3e22338b8855a69997f5d6e01ebda551",
            samples: &[],
        },
    ];

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L2/legacysurvey-0532m280-{image,invvar}-{g,r,z}.fits.fz"]
    fn real_legacy_brick_float_tiles_decode_to_the_oracle_pixels() {
        for expected in &BRICK_PLANES {
            let file = expected.file;
            let image = load_fits_image(real(&["L2", file]).to_str().unwrap()).unwrap();
            let not_a_number = image.iter().filter(|pixel| pixel.is_nan()).count();
            let exact_zeros = image.iter().filter(|&&pixel| pixel == 0.0).count();
            let sum: f64 = image.iter().map(|&pixel| f64::from(pixel)).sum();
            let relative = ((sum - expected.sum) / expected.sum).abs();
            let sha256 = sha256_hex(&image.iter().flat_map(|pixel| pixel.to_le_bytes()).collect::<Vec<u8>>());
            println!(
                "{file}: shape {:?}, {not_a_number} NaN, {exact_zeros} exact zeros, sum {sum:?} (oracle {:?}, relative difference {relative:.1e}), SHA-256 {sha256}",
                image.dim(),
                expected.sum
            );
            assert_eq!(image.dim(), LEGACY_SHAPE, "{file}, {BRICK_GOLDEN}");
            assert_eq!(not_a_number, 0, "{file}: NaN pixels, {BRICK_GOLDEN}");
            for &((y, x), pixel) in expected.samples {
                assert_eq!(image[[y, x]], pixel, "{file}: pixel [{y}, {x}], {BRICK_GOLDEN}");
            }
            assert_eq!(exact_zeros, expected.exact_zeros, "{file}: exact zeros, {BRICK_GOLDEN}");
            assert!(relative <= BRICK_SUM_RELATIVE_TOLERANCE, "{file}: sum {sum:?} against {:?}, {BRICK_GOLDEN}", expected.sum);
            assert_eq!(sha256, expected.sha256, "{file}: little-endian f32 pixels, {BRICK_GOLDEN}");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn2.fits, survey/E1/EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits"]
    fn real_plain_survey_masks_read_as_exact_integer_planes() {
        let golden = BRICK_GOLDEN;

        let des = real_plane(&real(&["D1", "des-dr2_A_r_extn2.fits"]), 0);
        let sha256 = sha256_of_little_endian_words(&des);
        println!("DES MSK (HDU 0): shape {:?}, signed {}, {} non-zero pixels, SHA-256 {sha256}", des.bits.dim(), des.signed, non_zero(&des));
        assert_eq!(des.bits.dim(), (458, 458), "{golden}");
        assert!(des.signed, "{golden}");
        assert_eq!(non_zero(&des), 0, "every DES mask value is 0, {golden}");
        assert_eq!(sha256, "52f97647edec082f0f09d364e37a59cbb9814c1b58552cb403cd77b01cd2d5b3", "little-endian i32 values, {golden}");

        let euclid = real_plane(
            &real(&["E1", "EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits"]),
            0,
        );
        let sha256 = sha256_of_little_endian_words(&euclid);
        let maximum = values(&euclid).max().unwrap();
        let zeros = euclid.bits.len() - non_zero(&euclid);
        println!("Euclid FLAG (HDU 0): shape {:?}, signed {}, maximum {maximum}, {zeros} zeros, SHA-256 {sha256}", euclid.bits.dim(), euclid.signed);
        assert_eq!(euclid.bits.dim(), (601, 601), "{golden}");
        assert_eq!(maximum, 25_296_896, "{golden}");
        assert_eq!(zeros, 280_186, "{golden}");
        assert_eq!(sha256, "8e19596a876efee0bbff43e0283902fd7392d34675a30053d6b1187a47d27f0f", "little-endian i32 values, {golden}");
    }
}
