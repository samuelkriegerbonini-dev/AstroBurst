use std::path::Path;

use crate::types::constants::{REF_FRAGMENT_ARRAY, REF_FRAGMENT_HDU};

const MAX_ARRAY_KEY_LEN: usize = 128;
const MAX_STEM_FRAGMENT_LEN: usize = 64;

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

fn is_array_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'
}

fn is_valid_array_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_ARRAY_KEY_LEN && key.chars().all(is_array_key_char)
}

fn parse_fragment(fragment: &str) -> Option<PlaneSelector> {
    if let Some(n) = fragment.strip_prefix(REF_FRAGMENT_HDU) {
        if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
            return n.parse::<usize>().ok().map(PlaneSelector::Hdu);
        }
        return None;
    }
    if let Some(key) = fragment.strip_prefix(REF_FRAGMENT_ARRAY) {
        if is_valid_array_key(key) {
            return Some(PlaneSelector::Array(key.to_string()));
        }
    }
    None
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
            PlaneSelector::Array(k) => Some(format!("{}{}", REF_FRAGMENT_ARRAY, k)),
        }
    }

    pub fn output_stem(&self) -> String {
        let stem = Path::new(&self.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("output");
        match &self.plane {
            PlaneSelector::Auto => stem.to_string(),
            PlaneSelector::Hdu(n) => format!("{}_hdu{}", stem, n),
            PlaneSelector::Array(k) => format!("{}_{}", stem, sanitize_fragment(k)),
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
        assert_eq!(r.output_stem(), "a");
    }

    #[test]
    fn invalid_fragments_keep_the_whole_string_as_path() {
        for s in ["a.fits#hdu=x", "a.fits#foo=1", "a.fits#hdu=", "a.fits#hdu=-1", "a.fits#hdu= 1", "a.fits#array=", "a.fits#array=a b", "#hdu=1"] {
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
        assert_eq!(ImageRef::parse("C:/d/jw01234_cal.fits").output_stem(), "jw01234_cal");
        assert_eq!(ImageRef::parse("C:/d/jw01234_cal.fits#hdu=3").output_stem(), "jw01234_cal_hdu3");
        assert_eq!(ImageRef::parse("C:/d/r0000.asdf#array=roman.dq").output_stem(), "r0000_roman_dq");
        assert_eq!(ImageRef::parse("").output_stem(), "output");
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
        for s in ["a.fits", "a.fits#hdu=0", "a.fits#hdu=12", "b.asdf#array=dq", "b.asdf#array=roman.err", "C:/x/y#z/a.fits#array=var_poisson"] {
            assert_eq!(ImageRef::parse(s).cache_key(), s, "{s}");
        }
    }

    #[test]
    fn array_key_length_limit() {
        let ok: String = std::iter::repeat('a').take(128).collect();
        assert!(matches!(ImageRef::parse(&format!("a.asdf#array={ok}")).plane, PlaneSelector::Array(_)));
        let too_long: String = std::iter::repeat('a').take(129).collect();
        assert!(ImageRef::parse(&format!("a.asdf#array={too_long}")).is_auto());
    }
}
