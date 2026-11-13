pub(crate) const BZIP2_MAGIC: [u8; 3] = *b"BZh";
pub(crate) const BZIP2_HEAD_BYTES: usize = 10;
const BZIP2_SUFFIX: &str = ".bz2";
const BLOCK_MARKER: [u8; 6] = [0x31, 0x41, 0x59, 0x26, 0x53, 0x59];
const END_OF_STREAM_MARKER: [u8; 6] = [0x17, 0x72, 0x45, 0x38, 0x50, 0x90];

pub const BZIP2_BYTES_REFUSAL: &str =
    "this tool does not read bzip2-compressed FITS yet; decompress the file (for example with bunzip2) and open the plain FITS file";

pub fn starts_with_bzip2(head: &[u8]) -> bool {
    head.len() >= BZIP2_HEAD_BYTES
        && head.starts_with(&BZIP2_MAGIC)
        && (b'1'..=b'9').contains(&head[BZIP2_MAGIC.len()])
        && [BLOCK_MARKER, END_OF_STREAM_MARKER]
            .iter()
            .any(|marker| head[BZIP2_MAGIC.len() + 1..BZIP2_HEAD_BYTES] == *marker)
}

pub(crate) fn split_bz2_suffix(name: &str) -> (&str, Option<&str>) {
    let stem_len = name.len().checked_sub(BZIP2_SUFFIX.len()).filter(|&len| {
        len > 0 && name.is_char_boundary(len) && name[len..].eq_ignore_ascii_case(BZIP2_SUFFIX)
    });
    match stem_len {
        Some(len) => (&name[..len], Some(&name[len..])),
        None => (name, None),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    pub fn bzip2_bytes(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    pub fn bzip2_copy(plain: &Path, name: &str) -> PathBuf {
        let target = plain.with_file_name(name);
        std::fs::write(&target, bzip2_bytes(&std::fs::read(plain).unwrap())).unwrap();
        target
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sha2::{Digest, Sha256};

    use super::test_support::bzip2_bytes;
    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::gzip::resolve_gzip;
    use crate::infra::image_source::{list_planes, load_plane};
    use crate::types::ImageRef;

    const FRAME_BYTES: usize = 12_447_360;

    fn real_frame(band: &str) -> PathBuf {
        env_dir("ASTROBURST_SCIENCE_DATA_DIR")
            .join("survey")
            .join("S1")
            .join(format!("frame-{band}-001462-4-0165.fits.bz2"))
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn bzip2_magic_needs_the_stream_header_and_a_block_or_end_marker() {
        let empty = bzip2_bytes(b"");
        assert_eq!(empty.len(), 14);
        assert_eq!(empty[4..10], [0x17, 0x72, 0x45, 0x38, 0x50, 0x90]);
        assert!(starts_with_bzip2(&empty));
        assert!(starts_with_bzip2(&bzip2_bytes(b"x")));
        assert!(starts_with_bzip2(b"BZh91AY&SY\x00\x00"));
        for head in [
            b"BZh9".as_slice(),
            b"BZh01AY&SY",
            b"BZh9garbage",
            &[0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0],
            b"SIMPLE  =                    T",
            b"",
        ] {
            assert!(!starts_with_bzip2(head), "{head:?}");
        }
    }

    #[test]
    fn the_bz2_suffix_is_split_without_regard_to_case() {
        assert_eq!(split_bz2_suffix("frame.fits.bz2"), ("frame.fits", Some(".bz2")));
        assert_eq!(split_bz2_suffix("LUM.FIT.BZ2"), ("LUM.FIT", Some(".BZ2")));
        assert_eq!(split_bz2_suffix("plain.fits"), ("plain.fits", None));
        assert_eq!(split_bz2_suffix(".bz2"), (".bz2", None));
        assert_eq!(split_bz2_suffix("x.bz"), ("x.bz", None));
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/S1/frame-<g,r,i>-001462-4-0165.fits.bz2"]
    fn real_sdss_frames_decompress_to_the_bytes_of_the_oracle() {
        for (band, compressed_bytes, decompressed_sha256) in [
            ("g", 3_342_526, "fcc24b2394b322f0718573521b021a475c52571f9bc3fbaaa162b6989c445375"),
            ("r", 3_545_788, "12306cf5f0cf08d17e5433320176fc485f4b5d777eaa73bd8a351a952d1f04b4"),
            ("i", 4_022_981, "563f9c623aaf25ae3ea36102cc09b789c8c7d88def9d5120da976df3af6a4141"),
        ] {
            let path = real_frame(band);
            let on_disk = std::fs::metadata(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
                .len();
            let (copy, guard) = resolve_gzip(&path).unwrap();
            let bytes = std::fs::read(&copy).unwrap();
            let measured = sha256_hex(&bytes);
            println!(
                "{band}: {on_disk} bytes compressed, {} bytes decompressed as {}, SHA-256 {measured}",
                bytes.len(),
                copy.file_name().unwrap().to_string_lossy()
            );
            assert_eq!(on_disk, compressed_bytes, "{band}");
            assert!(guard.is_some(), "{band}");
            assert_eq!(
                copy.file_name().unwrap().to_str().unwrap(),
                format!("frame-{band}-001462-4-0165.fits")
            );
            assert_eq!(bytes.len(), FRAME_BYTES, "{band}");
            assert_eq!(
                measured, decompressed_sha256,
                "plan/goldens/survey/layers_sdss_s1.json facts.{band}.decompressed_sha256"
            );
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/S1/frame-<g,r,i>-001462-4-0165.fits.bz2"]
    fn real_sdss_frames_open_through_the_image_loader() {
        for (band, pixel_sha256, exact_zeros) in [
            ("g", "448097a292561dab7fbf3b3c0574e91c7fc404908ca6fcabbf2a25751bb21f5b", 0),
            ("r", "8232fcc1ca13e46f94547e73c026b017ea27e52ca1cf836aaff8ec6e206c21b9", 3),
            ("i", "0d9e6f870a60fbf2edbb800521f1f3e9d240ec6e16d61a1259ebbf4339b2d23b", 6),
        ] {
            let key = real_frame(band).to_string_lossy().into_owned();
            let (planes, _) = list_planes(&key).unwrap();
            let loaded = load_plane(&ImageRef::parse(&key)).unwrap();
            let pixels: Vec<u8> = loaded.arr.iter().flat_map(|value| value.to_le_bytes()).collect();
            let measured = sha256_hex(&pixels);
            let zeros = loaded.arr.iter().filter(|&&value| value == 0.0).count();
            let minimum = loaded.arr.iter().copied().fold(f32::INFINITY, f32::min);
            let maximum = loaded.arr.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            println!(
                "{band}: {} HDUs, shape {:?}, BUNIT {:?}, pixel SHA-256 {measured}, {zeros} exact zeros, minimum {minimum} nanomaggy, maximum {maximum} nanomaggy",
                planes.len(),
                loaded.arr.dim(),
                loaded.header.get("BUNIT")
            );
            assert_eq!(planes.len(), 4, "{band}");
            assert_eq!(loaded.arr.dim(), (1489, 2048), "{band}");
            assert_eq!(loaded.header.get("BUNIT"), Some("nanomaggy"), "{band}");
            assert_eq!(
                measured, pixel_sha256,
                "plan/goldens/survey/file_facts.json sha256_of_little_endian_pixels of HDU 0, band {band}"
            );
            assert_eq!(zeros, exact_zeros, "{band}");
            if band == "r" {
                assert_eq!((minimum, maximum), (-0.156_982_421_875, 205.5));
            }
        }
    }
}
