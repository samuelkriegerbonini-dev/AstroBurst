use crate::types::image_ref::{path_key, ImageRef};

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const PART_SEPARATOR: &[u8] = &[0];
const HEX_ID_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(FNV_OFFSET_BASIS)
    }
}

impl Fnv1a {
    pub fn update(self, bytes: &[u8]) -> Self {
        Self(bytes.iter().fold(self.0, |hash, &b| (hash ^ u64::from(b)).wrapping_mul(FNV_PRIME)))
    }

    pub fn finish(self) -> u64 {
        self.0
    }

    pub fn hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

pub fn is_hex_id(text: &str) -> bool {
    text.len() == HEX_ID_LEN && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn canonical_ref(image_key: &str) -> String {
    let image = ImageRef::parse(image_key);
    let path = path_key(&image.path);
    match image.fragment() {
        Some(fragment) => format!("{path}#{fragment}"),
        None => path,
    }
}

pub fn rebase_ref(image_key: &str, base_path: Option<&str>) -> Option<String> {
    let source = canonical_ref(image_key);
    let source_path = path_key(&ImageRef::parse(image_key).path);
    base_path
        .filter(|b| !b.trim().is_empty())
        .map(canonical_ref)
        .filter(|b| *b != source && *b != source_path)
}

pub fn image_id(image_key: &str, base_path: Option<&str>) -> String {
    let hash = Fnv1a::default().update(canonical_ref(image_key).as_bytes());
    match rebase_ref(image_key, base_path) {
        Some(base) => hash.update(PART_SEPARATOR).update(base.as_bytes()).hex(),
        None => hash.hex(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_the_published_64_bit_vectors() {
        assert_eq!(Fnv1a::default().update(b"").finish(), 0xcbf2_9ce4_8422_2325);
        assert_eq!(Fnv1a::default().update(b"a").finish(), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(Fnv1a::default().update(b"foobar").finish(), 0x8594_4171_f739_67e8);
        assert_eq!(Fnv1a::default().update(b"foo").update(b"bar").hex(), "85944171f73967e8");
    }

    #[cfg(windows)]
    #[test]
    fn image_id_is_stable_across_path_spelling() {
        let backslash = image_id(r"D:\x\A.fits#hdu=1", None);
        let forward = image_id("d:/x/a.fits#hdu=1", None);
        assert_eq!(backslash, forward, "one file spelled two ways got two histories");
        assert_eq!(canonical_ref(r"D:\x\A.fits#hdu=1"), "d:/x/a.fits#hdu=1");
        assert_eq!(image_id(r"D:\x\\A.fits#hdu=1", None), forward, "a doubled separator split one file's history");
        assert_eq!(image_id(r"\\server\share\a.fits", None), image_id("//server/share/a.fits", None));
        assert_eq!(backslash.len(), 16);
        assert!(backslash.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "{backslash}");
    }

    #[test]
    fn image_id_tells_planes_apart_and_keeps_array_keys_case_sensitive() {
        assert_ne!(image_id("/x/a.fits#hdu=1", None), image_id("/x/a.fits#hdu=2", None));
        assert_ne!(image_id("/x/a.fits", None), image_id("/x/a.fits#hdu=1", None));
        assert_ne!(image_id("/x/a.asdf#array=SCI", None), image_id("/x/a.asdf#array=sci", None));
        assert_eq!(image_id("/x/a.fits#hdu=1", None), image_id("/x/a.fits#hdu=1", None));
    }

    #[test]
    fn image_id_spells_paths_the_way_path_key_does() {
        assert_eq!(image_id("/x//a.fits#hdu=1", None), image_id("/x/a.fits#hdu=1", None));
        assert_eq!(canonical_ref("/x//a.fits#hdu=1"), format!("{}#hdu=1", path_key("/x/a.fits")));
    }

    #[test]
    fn image_id_changes_with_the_chain_base() {
        let key = "/data/m42.fits#hdu=0";
        let plain = image_id(key, None);
        let stacked = image_id(key, Some("/app/output/stack_result.fits"));
        let frame = image_id(key, Some("/app/output/cube_frame_12.fits"));
        assert_ne!(plain, stacked, "a rebased chain shared the file's history");
        assert_ne!(stacked, frame);
        assert_eq!(image_id(key, Some("/data/m42.fits")), plain, "a base equal to the source is not a rebase");
        assert_eq!(image_id(key, Some(key)), plain);
        assert_eq!(image_id(key, Some("")), plain);
        assert_eq!(rebase_ref(key, Some("/data/m42.fits")), None);
        assert_eq!(rebase_ref(key, Some("/app/output//stack_result.fits")).as_deref(), Some("/app/output/stack_result.fits"));
    }

    #[test]
    fn hex_ids_are_exactly_16_lowercase_hex_digits() {
        assert!(is_hex_id("0123456789abcdef"));
        for text in ["0123456789ABCDEF", "0123456789abcde", "0123456789abcdef0", "../0123456789abc", ""] {
            assert!(!is_hex_id(text), "{text:?}");
        }
    }
}
