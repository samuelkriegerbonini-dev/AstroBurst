use std::collections::HashMap;
use std::path::Path;

use crate::types::constants::{REF_FRAGMENT_ARRAY, REF_FRAGMENT_HDU};

const MAX_STEM_FRAGMENT_LEN: usize = 64;
const DEFAULT_STEM: &str = "output";
const COMPRESSION_EXTENSIONS: &[&str] = &["fz", "gz"];
const FITS_EXTENSIONS: &[&str] = &["fits", "fit", "fts"];

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaneSelector {
    Auto,
    Hdu(usize),
    Array(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageRef {
    pub path: String,
    pub plane: PlaneSelector,
}

fn is_array_key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-'
}

fn is_upper_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'A'..=b'F').contains(&b)
}

fn encode_array_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.bytes() {
        if is_array_key_byte(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn decode_array_key(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if is_array_key_byte(b) {
            out.push(b);
            i += 1;
            continue;
        }
        if b != b'%' {
            return None;
        }
        let hex = bytes.get(i + 1..i + 3)?;
        if !hex.iter().all(|&h| is_upper_hex(h)) {
            return None;
        }
        let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
        if is_array_key_byte(value) {
            return None;
        }
        out.push(value);
        i += 3;
    }
    let key = String::from_utf8(out).ok()?;
    (!key.is_empty()).then_some(key)
}

fn parse_fragment(fragment: &str) -> Option<PlaneSelector> {
    if let Some(n) = fragment.strip_prefix(REF_FRAGMENT_HDU) {
        if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
            return n.parse::<usize>().ok().map(PlaneSelector::Hdu);
        }
        return None;
    }
    fragment
        .strip_prefix(REF_FRAGMENT_ARRAY)
        .and_then(decode_array_key)
        .map(PlaneSelector::Array)
}

fn split_extension(name: &str) -> Option<(&str, &str)> {
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    Some((&name[..dot], &name[dot + 1..]))
}

fn has_extension_in(ext: &str, list: &[&str]) -> bool {
    list.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

pub fn source_stem(path: &str) -> String {
    let name = Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or("");
    let stem = match split_extension(name) {
        Some((stem, ext)) if has_extension_in(ext, COMPRESSION_EXTENSIONS) => match split_extension(stem) {
            Some((inner, inner_ext)) if has_extension_in(inner_ext, FITS_EXTENSIONS) => inner,
            _ => stem,
        },
        Some((stem, _)) => stem,
        None => name,
    };
    if stem.is_empty() {
        DEFAULT_STEM.to_string()
    } else {
        stem.to_string()
    }
}

pub fn path_key(path: &str) -> String {
    let unified = path.replace('\\', "/");
    let prefix = if unified.starts_with("//") { "//" } else { "" };
    let mut key = String::with_capacity(unified.len());
    key.push_str(prefix);
    let mut after_slash = !prefix.is_empty();
    for ch in unified[prefix.len()..].chars() {
        if ch == '/' && after_slash {
            continue;
        }
        after_slash = ch == '/';
        key.push(ch);
    }
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

#[derive(Debug, Default)]
pub struct OutputStems {
    by_source: HashMap<String, String>,
    owners: HashMap<String, String>,
}

impl OutputStems {
    pub fn stem_for(&mut self, source: &str) -> String {
        let source_key = path_key(source);
        if let Some(stem) = self.by_source.get(&source_key) {
            return stem.clone();
        }
        let base = source_stem(source);
        let mut stem = base.clone();
        let mut n = 2usize;
        while self.owners.contains_key(&stem.to_lowercase()) {
            stem = format!("{}_{}", base, n);
            n += 1;
        }
        self.owners.insert(stem.to_lowercase(), source_key.clone());
        self.by_source.insert(source_key, stem.clone());
        stem
    }
}

impl ImageRef {
    pub fn parse(key: &str) -> ImageRef {
        if key.starts_with("__") {
            return ImageRef::auto(key);
        }
        if let Some(pos) = key.rfind('#') {
            let (path, fragment) = (&key[..pos], &key[pos + 1..]);
            if !path.is_empty() {
                if let Some(plane) = parse_fragment(fragment) {
                    return ImageRef { path: path.to_string(), plane };
                }
            }
        }
        ImageRef::auto(key)
    }

    pub fn auto(path: &str) -> ImageRef {
        ImageRef { path: path.to_string(), plane: PlaneSelector::Auto }
    }

    pub fn hdu(path: &str, index: usize) -> ImageRef {
        ImageRef { path: path.to_string(), plane: PlaneSelector::Hdu(index) }
    }

    pub fn array(path: &str, key: &str) -> ImageRef {
        ImageRef { path: path.to_string(), plane: PlaneSelector::Array(key.to_string()) }
    }

    pub fn cache_key(&self) -> String {
        match self.fragment() {
            Some(f) => format!("{}#{}", self.path, f),
            None => self.path.clone(),
        }
    }

    pub fn is_auto(&self) -> bool {
        self.plane == PlaneSelector::Auto
    }

    pub fn is_synthetic(&self) -> bool {
        self.path.starts_with("__")
    }

    pub fn fragment(&self) -> Option<String> {
        match &self.plane {
            PlaneSelector::Auto => None,
            PlaneSelector::Hdu(n) => Some(format!("{}{}", REF_FRAGMENT_HDU, n)),
            PlaneSelector::Array(k) => Some(format!("{}{}", REF_FRAGMENT_ARRAY, encode_array_key(k))),
        }
    }

    pub fn output_stem_in(&self, stems: &mut OutputStems) -> String {
        let base = stems.stem_for(&self.path);
        match &self.plane {
            PlaneSelector::Auto => base,
            PlaneSelector::Hdu(n) => format!("{}_hdu{}", base, n),
            PlaneSelector::Array(k) => format!("{}_{}", base, sanitize_fragment(k)),
        }
    }
}

pub fn sanitize_fragment(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut last_underscore = false;
    for c in key.chars() {
        let mapped = if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' };
        if mapped == '_' {
            if last_underscore {
                continue;
            }
            last_underscore = true;
        } else {
            last_underscore = false;
        }
        out.push(mapped);
    }
    let trimmed = out.trim_matches('_');
    let mut result: String = trimmed.chars().take(MAX_STEM_FRAGMENT_LEN).collect();
    if result.is_empty() {
        result.push_str("plane");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stem(key: &str) -> String {
        ImageRef::parse(key).output_stem_in(&mut OutputStems::default())
    }

    #[test]
    fn parse_auto_round_trips_plain_path() {
        let r = ImageRef::parse("C:/data/a.fits");
        assert_eq!(r, ImageRef::auto("C:/data/a.fits"));
        assert!(r.is_auto());
        assert_eq!(r.cache_key(), "C:/data/a.fits");
        assert_eq!(r.fragment(), None);
    }

    #[test]
    fn parse_hdu_fragment() {
        let r = ImageRef::parse("a.fits#hdu=3");
        assert_eq!(r, ImageRef::hdu("a.fits", 3));
        assert_eq!(r.cache_key(), "a.fits#hdu=3");
        assert_eq!(r.fragment().as_deref(), Some("hdu=3"));
        assert!(!r.is_auto());
    }

    #[test]
    fn parse_array_fragment_with_dotted_key() {
        let r = ImageRef::parse("r0000.asdf#array=roman.dq");
        assert_eq!(r, ImageRef::array("r0000.asdf", "roman.dq"));
        assert_eq!(r.cache_key(), "r0000.asdf#array=roman.dq");
        assert_eq!(r.fragment().as_deref(), Some("array=roman.dq"));
    }

    #[test]
    fn directory_with_hash_stays_whole() {
        let r = ImageRef::parse("C:/data/run#7/a.fits");
        assert_eq!(r, ImageRef::auto("C:/data/run#7/a.fits"));
        assert_eq!(r.cache_key(), "C:/data/run#7/a.fits");
        assert_eq!(stem("C:/data/run#7/a.fits"), "a");
    }

    #[test]
    fn invalid_fragments_keep_the_whole_string_as_path() {
        for s in [
            "a.fits#hdu=x",
            "a.fits#foo=1",
            "a.fits#hdu=",
            "a.fits#hdu=-1",
            "a.fits#hdu= 1",
            "a.fits#array=",
            "a.fits#array=a b",
            "a.fits#array=a%2",
            "a.fits#array=a%2x",
            "a.fits#array=a%2f",
            "a.fits#array=%41",
            "a.fits#array=%FF",
            "#hdu=1",
        ] {
            let r = ImageRef::parse(s);
            assert_eq!(r, ImageRef::auto(s), "{s}");
            assert_eq!(r.cache_key(), s);
        }
    }

    #[test]
    fn synthetic_keys_are_never_split() {
        for s in ["__composite_r", "__wizard_ch_ha_aligned#hdu=1", "__star_mask#array=dq"] {
            let r = ImageRef::parse(s);
            assert_eq!(r.plane, PlaneSelector::Auto, "{s}");
            assert_eq!(r.path, s);
            assert!(r.is_synthetic());
        }
        assert!(!ImageRef::parse("a.fits").is_synthetic());
    }

    #[test]
    fn last_hash_is_the_fragment() {
        let r = ImageRef::parse("C:/run#7/a.fits#hdu=2");
        assert_eq!(r, ImageRef::hdu("C:/run#7/a.fits", 2));
    }

    #[test]
    fn output_stem_for_all_forms() {
        assert_eq!(stem("C:/d/jw01234_cal.fits"), "jw01234_cal");
        assert_eq!(stem("C:/d/jw01234_cal.fits#hdu=3"), "jw01234_cal_hdu3");
        assert_eq!(stem("C:/d/r0000.asdf#array=roman.dq"), "r0000_roman_dq");
        assert_eq!(stem(""), "output");
    }

    #[test]
    fn output_stem_drops_the_whole_compound_fits_extension() {
        assert_eq!(stem("C:/d/jw01234_cal.fits.fz#hdu=1"), "jw01234_cal_hdu1");
        assert_eq!(stem("C:/d/jw01234_cal.FITS.GZ"), "jw01234_cal");
        assert_eq!(stem("C:/d/m31.fit.fz"), "m31");
        assert_eq!(stem("C:/d/notes.tar.gz"), "notes.tar");
        assert_eq!(stem("C:/d/.fits"), ".fits");
    }

    #[test]
    fn same_stem_from_different_sources_gets_distinct_output_stems() {
        let mut stems = OutputStems::default();
        let first = ImageRef::parse("C:/night1/light_001.fits").output_stem_in(&mut stems);
        let second = ImageRef::parse("C:/night2/light_001.fits").output_stem_in(&mut stems);
        let fit = ImageRef::parse("C:/night1/light_001.fit").output_stem_in(&mut stems);
        let zipped = ImageRef::parse("C:/night1/light_001.zip").output_stem_in(&mut stems);
        assert_eq!(first, "light_001");
        assert_eq!(second, "light_001_2");
        assert_eq!(fit, "light_001_3");
        assert_eq!(zipped, "light_001_4");

        assert_eq!(ImageRef::parse("C:/night2/light_001.fits#hdu=1").output_stem_in(&mut stems), "light_001_2_hdu1");
        assert_eq!(ImageRef::parse("C:/night1/light_001.fits").output_stem_in(&mut stems), "light_001");
        assert_eq!(
            ImageRef::parse("C:/other/light_001_2.fits").output_stem_in(&mut stems),
            "light_001_2_2",
            "a source whose own stem is already taken by a renamed source must not share its outputs"
        );
    }

    #[test]
    fn path_spelling_of_the_same_file_keeps_one_stem() {
        let mut stems = OutputStems::default();
        let a = stems.stem_for("C:/data/m42.fits");
        let b = stems.stem_for("C:\\data\\m42.fits");
        let doubled = stems.stem_for("C:/data//m42.fits");
        assert_eq!(a, "m42");
        assert_eq!(b, "m42");
        assert_eq!(doubled, "m42", "a doubled separator split one file into two output stems");
    }

    #[test]
    fn path_key_collapses_repeated_separators_but_keeps_a_unc_prefix() {
        assert_eq!(path_key("C:/out//x.fits"), path_key("C:\\out\\x.fits"));
        assert_eq!(path_key("C:\\out\\\\x.fits"), path_key("C:/out/x.fits"));
        assert!(path_key("\\\\server\\share\\\\x.fits").starts_with("//server/share/x"));
        assert_ne!(path_key("//server/share/x.fits"), path_key("/server/share/x.fits"));
        assert_eq!(path_key("///server/x.fits"), path_key("//server/x.fits"));
    }

    #[test]
    fn sanitize_fragment_cases() {
        assert_eq!(sanitize_fragment("roman.dq"), "roman_dq");
        assert_eq!(sanitize_fragment("var_poisson"), "var_poisson");
        assert_eq!(sanitize_fragment("a..b"), "a_b");
        assert_eq!(sanitize_fragment(""), "plane");
        assert_eq!(sanitize_fragment("..."), "plane");
        assert_eq!(sanitize_fragment("_x_"), "x");
        assert_eq!(sanitize_fragment("a-b"), "a-b");
        let long: String = std::iter::repeat('k').take(100).collect();
        assert_eq!(sanitize_fragment(&long).len(), 64);
    }

    #[test]
    fn cache_key_equals_input_for_canonical_inputs() {
        for s in [
            "a.fits",
            "a.fits#hdu=0",
            "a.fits#hdu=12",
            "b.asdf#array=dq",
            "b.asdf#array=roman.err",
            "C:/x/y#z/a.fits#array=var_poisson",
            "b.asdf#array=sci%20image",
        ] {
            assert_eq!(ImageRef::parse(s).cache_key(), s, "{s}");
        }
    }

    #[test]
    fn every_array_key_a_constructor_accepts_round_trips_through_parse() {
        let long: String = std::iter::repeat('a').take(300).collect();
        for key in ["sci image", "data(2)", "a#b", "ñandú", "100%", "roman.dq", long.as_str()] {
            let r = ImageRef::array("x.asdf", key);
            let key_string = r.cache_key();
            assert_eq!(ImageRef::parse(&key_string), r, "{key_string}");
        }
    }
}
