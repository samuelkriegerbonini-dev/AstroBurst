use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};

use crate::types::constants::BLOCK_SIZE;

pub const MAX_FITS_AXES: i64 = 999;

const COMMENTARY_KEYS: [&str; 2] = ["COMMENT", "HISTORY"];
const HDU_KIND_KEYS: [&str; 5] = ["SIMPLE", "XTENSION", "EXTEND", "PCOUNT", "GCOUNT"];
const NEVER_INHERITED_KEYS: [&str; 17] = [
    "BITPIX", "NAXIS", "GROUPS", "BSCALE", "BZERO", "BLANK", "BUNIT", "DATAMIN", "DATAMAX", "CHECKSUM", "DATASUM",
    "INHERIT", "WCSAXES", "WCSNAME", "LONPOLE", "LATPOLE", "THEAP",
];
const WCS_AXIS_PREFIXES: [&str; 9] = ["CRPIX", "CRVAL", "CDELT", "CTYPE", "CUNIT", "CROTA", "CRDER", "CSYER", "CNAME"];
const WCS_MATRIX_PREFIXES: [&str; 4] = ["CD", "PC", "PV", "PS"];
const SIP_PREFIXES: [&str; 4] = ["AP_", "BP_", "A_", "B_"];
const SIP_SUFFIXES: [&str; 2] = ["ORDER", "DMAX"];
const INHERIT_CARD: &str = "INHERIT";

pub fn is_commentary_key(key: &str) -> bool {
    COMMENTARY_KEYS.contains(&key.trim())
}

pub fn parse_fits_float(raw: &str) -> Option<f64> {
    let text = raw.trim();
    if let Ok(value) = text.parse::<f64>() {
        return Some(value);
    }
    let (mantissa, exponent) = text.split_once(['D', 'd'])?;
    format!("{mantissa}E{exponent}").parse().ok()
}

pub fn format_fits_float(value: f64) -> String {
    if !value.is_finite() {
        return format!("{value:E}");
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if mantissa.starts_with('-') { "-" } else { "" };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    if !(-4..16).contains(&exponent) {
        let (head, tail) = digits.split_at(1);
        let point = if tail.is_empty() { "" } else { "." };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{head}{point}{tail}E{exponent_sign}{:02}", exponent.abs());
    }
    if exponent < 0 {
        return format!("{sign}0.{}{digits}", "0".repeat((-exponent - 1) as usize));
    }
    let whole = exponent as usize + 1;
    if digits.len() <= whole {
        format!("{sign}{digits}{}.0", "0".repeat(whole - digits.len()))
    } else {
        format!("{sign}{}.{}", &digits[..whole], &digits[whole..])
    }
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn is_axis_pair(text: &str) -> bool {
    text.split_once('_').is_some_and(|(i, j)| all_digits(i) && all_digits(j))
}

fn is_wcs_key(key: &str) -> bool {
    let primary_version = match key.as_bytes().last() {
        Some(last) if last.is_ascii_uppercase() && key.len() > 1 && key.as_bytes()[key.len() - 2].is_ascii_digit() => &key[..key.len() - 1],
        _ => key,
    };
    WCS_AXIS_PREFIXES.iter().any(|prefix| primary_version.strip_prefix(prefix).is_some_and(all_digits))
        || WCS_MATRIX_PREFIXES.iter().any(|prefix| primary_version.strip_prefix(prefix).is_some_and(is_axis_pair))
        || SIP_PREFIXES
            .iter()
            .any(|prefix| key.strip_prefix(prefix).is_some_and(|rest| is_axis_pair(rest) || SIP_SUFFIXES.contains(&rest)))
}

pub fn is_never_inherited_key(key: &str) -> bool {
    let key = key.trim().to_uppercase();
    HDU_KIND_KEYS.contains(&key.as_str())
        || NEVER_INHERITED_KEYS.contains(&key.as_str())
        || key.strip_prefix("NAXIS").is_some_and(all_digits)
        || is_wcs_key(&key)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HduHeader {
    #[allow(dead_code)]
    pub cards: Vec<(String, String)>,
    pub index: HashMap<String, String>,
    #[serde(skip)]
    pub string_keys: Option<HashSet<String>>,
}

impl HduHeader {
    pub fn empty() -> Self {
        Self {
            cards: Vec::new(),
            index: HashMap::new(),
            string_keys: None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.index.get(key).map(|s| s.as_str())
    }

    pub fn is_string_value(&self, key: &str) -> Option<bool> {
        self.string_keys.as_ref().map(|keys| keys.contains(key))
    }

    pub fn inherit_string_keys(&mut self, source: &HduHeader) {
        let kept = source
            .string_keys
            .as_ref()
            .map(|keys| keys.iter().filter(|k| self.index.contains_key(*k)).cloned().collect());
        self.string_keys = kept;
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.index.get(key)?.trim().parse().ok()
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        parse_fits_float(self.index.get(key)?)
    }

    pub fn set(&mut self, key: &str, value: String) {
        match self.cards.iter().position(|(k, _)| k == key) {
            Some(first) => {
                self.cards[first].1 = value.clone();
                if !is_commentary_key(key) {
                    let mut position = 0;
                    self.cards.retain(|(k, _)| {
                        let kept = position <= first || k != key;
                        position += 1;
                        kept
                    });
                }
            }
            None => self.cards.push((key.to_string(), value.clone())),
        }
        self.index.insert(key.to_string(), value);
    }

    pub fn set_f64(&mut self, key: &str, value: f64) {
        self.set(key, format_fits_float(value));
        if let Some(keys) = self.string_keys.as_mut() {
            keys.remove(key);
        }
    }

    pub fn repeated_keys(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut repeated: Vec<String> = Vec::new();
        for (key, _) in &self.cards {
            if !is_commentary_key(key) && !seen.insert(key.as_str()) && !repeated.contains(key) {
                repeated.push(key.clone());
            }
        }
        repeated
    }

    pub fn holds_data(&self) -> bool {
        self.checked_data_byte_count().map_or(true, |bytes| bytes > 0)
    }

    pub fn declines_inheritance(&self) -> bool {
        self.get(INHERIT_CARD).is_some_and(|value| value.trim().eq_ignore_ascii_case("F"))
    }

    pub fn gives_cards_to(&self, extension: &HduHeader) -> bool {
        !self.holds_data() && !extension.declines_inheritance()
    }

    pub fn remove(&mut self, key: &str) {
        self.cards.retain(|(k, _)| k != key);
        self.index.remove(key);
        if let Some(keys) = self.string_keys.as_mut() {
            keys.remove(key);
        }
    }

    fn non_negative_card(&self, key: &str, default: i64) -> Result<usize> {
        let value = match self.get(key).map(str::trim) {
            None => default,
            Some(raw) => raw
                .parse::<i64>()
                .map_err(|_| anyhow::anyhow!("{key} = '{raw}' is not an integer"))?,
        };
        usize::try_from(value).map_err(|_| anyhow::anyhow!("{key} = {value} is negative"))
    }

    pub fn data_byte_count(&self) -> usize {
        self.checked_data_byte_count().unwrap_or(usize::MAX)
    }

    pub fn checked_data_byte_count(&self) -> Result<usize> {
        let naxis = self.get_i64("NAXIS").unwrap_or(0);
        if !(0..=MAX_FITS_AXES).contains(&naxis) {
            bail!("NAXIS = {naxis} is outside the FITS range 0..={MAX_FITS_AXES}");
        }
        if naxis == 0 {
            return Ok(0);
        }
        let bitpix = self.get_i64("BITPIX").unwrap_or(0);
        let bytes_per_pixel =
            usize::try_from(bitpix.unsigned_abs() / 8).context("BITPIX is out of range")?;
        let mut elements: usize = 1;
        for axis in 1..=naxis {
            let len = self.non_negative_card(&format!("NAXIS{axis}"), 1)?;
            elements = elements
                .checked_mul(len)
                .with_context(|| format!("data unit size overflows at NAXIS{axis}"))?;
        }
        let pcount = self.non_negative_card("PCOUNT", 0).unwrap_or(0);
        let gcount = self.non_negative_card("GCOUNT", 1).unwrap_or(1).max(1);
        elements
            .checked_add(pcount)
            .and_then(|v| v.checked_mul(bytes_per_pixel))
            .and_then(|v| v.checked_mul(gcount))
            .context("data unit size overflows")
    }

    pub fn checked_padded_data_bytes(&self) -> Result<usize> {
        self.checked_data_byte_count()?
            .div_ceil(BLOCK_SIZE)
            .checked_mul(BLOCK_SIZE)
            .context("padded data unit size overflows")
    }

    pub fn merge_with(&self, extension: &HduHeader) -> HduHeader {
        let mut merged_index: HashMap<String, String> = HashMap::new();
        let mut merged_cards: Vec<(String, String)> = Vec::new();
        let mut seen = HashSet::new();

        for (k, v) in &extension.cards {
            let ku = k.to_uppercase();
            if HDU_KIND_KEYS.iter().any(|&sk| ku == sk) {
                continue;
            }
            merged_cards.push((k.clone(), v.clone()));
            if !is_commentary_key(k) {
                merged_index.entry(k.clone()).or_insert_with(|| v.clone());
                seen.insert(k.clone());
            }
        }

        if self.gives_cards_to(extension) {
            for (k, v) in &self.cards {
                if is_commentary_key(k) {
                    merged_cards.push((k.clone(), v.clone()));
                } else if !is_never_inherited_key(k) && !seen.contains(k) {
                    merged_cards.push((k.clone(), v.clone()));
                    merged_index.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            let carded: HashSet<&String> = self.cards.iter().map(|(k, _)| k).collect();
            for (k, v) in &self.index {
                if !carded.contains(k) && !is_never_inherited_key(k) && !seen.contains(k) {
                    merged_index.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }

        let string_keys = match (&self.string_keys, &extension.string_keys) {
            (None, None) => None,
            _ => Some(
                merged_index
                    .keys()
                    .filter(|k| {
                        let owner = if seen.contains(*k) { extension } else { self };
                        owner.is_string_value(k) == Some(true)
                    })
                    .cloned()
                    .collect(),
            ),
        };

        HduHeader {
            cards: merged_cards,
            index: merged_index,
            string_keys,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in cards {
            h.set(k, v.to_string());
        }
        h
    }

    #[test]
    fn get_f64_reads_the_fortran_d_exponent() {
        let h = header(&[
            ("BZERO", "3.2768D+04"),
            ("BSCALE", "3.05185094759D-05"),
            ("CRVAL1", "1.40500000000d+09"),
            ("CDELT1", "-1.5D-04"),
            ("PLAIN", "1.5E3"),
            ("BARE", "1.5D"),
            ("WORD", "DEADBEEF"),
        ]);
        assert_eq!(h.get_f64("BZERO"), Some(32768.0));
        assert_eq!(h.get_f64("BSCALE"), Some(3.05185094759e-5));
        assert_eq!(h.get_f64("CRVAL1"), Some(1.405e9));
        assert_eq!(h.get_f64("CDELT1"), Some(-1.5e-4));
        assert_eq!(h.get_f64("PLAIN"), Some(1500.0));
        assert_eq!(h.get_f64("BARE"), None);
        assert_eq!(h.get_f64("WORD"), None);
        assert_eq!(parse_fits_float(" 2.0D0 "), Some(2.0));
    }

    #[test]
    fn data_byte_count_sizes_images_and_tables() {
        let image = header(&[("NAXIS", "2"), ("BITPIX", "-32"), ("NAXIS1", "10"), ("NAXIS2", "3")]);
        assert_eq!(image.checked_data_byte_count().unwrap(), 120);
        assert_eq!(image.checked_padded_data_bytes().unwrap(), BLOCK_SIZE);
        let table = header(&[
            ("NAXIS", "2"),
            ("BITPIX", "8"),
            ("NAXIS1", "8"),
            ("NAXIS2", "4"),
            ("PCOUNT", "100"),
            ("GCOUNT", "1"),
        ]);
        assert_eq!(table.checked_data_byte_count().unwrap(), 132);
        assert_eq!(header(&[("NAXIS", "0")]).checked_padded_data_bytes().unwrap(), 0);
    }

    #[test]
    fn data_byte_count_rejects_corrupt_axes_instead_of_hanging_or_wrapping() {
        let huge_naxis = header(&[("NAXIS", "999999999999"), ("BITPIX", "16")]);
        assert!(huge_naxis.checked_data_byte_count().is_err());
        assert!(header(&[("NAXIS", "-3")]).checked_data_byte_count().is_err());

        let negative = header(&[("NAXIS", "2"), ("BITPIX", "16"), ("NAXIS1", "-1"), ("NAXIS2", "100")]);
        let err = negative.checked_data_byte_count().unwrap_err();
        assert!(err.to_string().contains("NAXIS1 = -1"), "{err}");

        let wraps = header(&[
            ("NAXIS", "3"),
            ("BITPIX", "16"),
            ("NAXIS1", "1024819115206086201"),
            ("NAXIS2", "3"),
            ("NAXIS3", "3"),
        ]);
        assert!(wraps.checked_data_byte_count().is_err());

        let huge_heap = header(&[
            ("NAXIS", "2"),
            ("BITPIX", "8"),
            ("NAXIS1", "8"),
            ("NAXIS2", "1"),
            ("PCOUNT", "9223372036854775807"),
            ("GCOUNT", "4"),
        ]);
        assert!(huge_heap.checked_data_byte_count().is_err());

        let padding_overflow = header(&[("NAXIS", "1"), ("BITPIX", "16"), ("NAXIS1", "9223372036854775807")]);
        assert!(padding_overflow.checked_data_byte_count().is_ok());
        assert!(padding_overflow.checked_padded_data_bytes().is_err());

        let unparseable = header(&[("NAXIS", "2"), ("BITPIX", "8"), ("NAXIS1", "99999999999999999999"), ("NAXIS2", "2")]);
        assert!(unparseable.checked_data_byte_count().is_err());

        assert_eq!(negative.data_byte_count(), usize::MAX, "an unsizeable data unit never fits a file");
        assert_eq!(huge_naxis.data_byte_count(), usize::MAX);
    }

    #[test]
    fn merge_keeps_commentary_cards_from_both_headers() {
        let mut primary = header(&[("SIMPLE", "T"), ("TELESCOP", "JWST")]);
        primary.cards.push(("HISTORY".into(), "primary step".into()));
        let mut ext = header(&[("XTENSION", "IMAGE"), ("EXTNAME", "SCI")]);
        ext.cards.push(("HISTORY".into(), "extension step".into()));
        ext.cards.push(("COMMENT".into(), "note".into()));

        let merged = primary.merge_with(&ext);
        let history: Vec<&str> = merged
            .cards
            .iter()
            .filter(|(k, _)| k == "HISTORY")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(history, vec!["extension step", "primary step"]);
        assert!(merged.cards.iter().any(|(k, v)| k == "COMMENT" && v == "note"));
        assert_eq!(merged.get("HISTORY"), None);
        assert_eq!(merged.get("TELESCOP"), Some("JWST"));
        assert_eq!(merged.get("XTENSION"), None);
    }

    fn astropy_goldens() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("fitsio")
            .join("astropy_fitsio_goldens.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("golden {} unreadable: {e}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("golden is not JSON: {e}"))
    }

    fn golden_float_cards() -> Vec<(f64, String, Option<String>)> {
        astropy_goldens()["float_cards"]
            .as_array()
            .expect("the golden of scripts/oracle/fitsio_review_goldens.py (astropy 8.0.2) lists float_cards")
            .iter()
            .map(|row| {
                let bits = u64::from_str_radix(row["bits"].as_str().unwrap(), 16).unwrap();
                let astropy_text = row["astropy_text_round_trips"]
                    .as_bool()
                    .unwrap()
                    .then(|| row["astropy_text"].as_str().unwrap().to_string());
                (f64::from_bits(bits), row["python_repr"].as_str().unwrap().to_string(), astropy_text)
            })
            .collect()
    }

    #[test]
    fn set_f64_round_trips_every_bit() {
        let mut h = HduHeader::empty();
        h.set_f64("K", 0.1 + 0.2);
        assert_eq!(h.get_f64("K").map(f64::to_bits), Some((0.1f64 + 0.2).to_bits()), "{:?}", h.get("K"));
        for (value, _, _) in golden_float_cards() {
            h.set_f64("K", value);
            assert_eq!(h.get_f64("K").map(f64::to_bits), Some(value.to_bits()), "{value:e} was written as {:?}", h.get("K"));
        }
    }

    #[test]
    fn set_f64_writes_the_shortest_text_and_agrees_with_astropy_wherever_astropy_keeps_every_bit() {
        let mut h = HduHeader::empty();
        for (value, shortest, astropy_text) in golden_float_cards() {
            h.set_f64("K", value);
            assert_eq!(h.get("K"), Some(shortest.as_str()), "{value:e}");
            if let Some(astropy_text) = astropy_text {
                assert_eq!(h.get("K"), Some(astropy_text.as_str()), "{value:e}");
            }
        }
    }

    #[test]
    fn set_on_a_repeated_keyword_leaves_one_card_with_the_new_value() {
        let mut h = HduHeader::empty();
        h.cards = vec![
            ("BUNIT".into(), "MJy/sr".into()),
            ("EXPTIME".into(), "100.0".into()),
            ("BUNIT".into(), "Jy".into()),
        ];
        h.index.insert("BUNIT".into(), "MJy/sr".into());
        h.index.insert("EXPTIME".into(), "100.0".into());
        h.set("BUNIT", "Jy".into());
        let units: Vec<&str> = h.cards.iter().filter(|(k, _)| k == "BUNIT").map(|(_, v)| v.as_str()).collect();
        assert_eq!(units, vec!["Jy"]);
        assert_eq!(h.cards[0], ("BUNIT".to_string(), "Jy".to_string()), "the card keeps its place");
        assert_eq!(h.get("BUNIT"), Some("Jy"));
    }

    fn primary_with_data() -> HduHeader {
        header(&[
            ("SIMPLE", "T"),
            ("BITPIX", "-32"),
            ("NAXIS", "3"),
            ("NAXIS1", "4"),
            ("NAXIS2", "4"),
            ("NAXIS3", "3"),
            ("EXTEND", "T"),
            ("BUNIT", "MJy/sr"),
            ("CRVAL1", "10.0"),
            ("DATAMAX", "5.0"),
            ("TELESCOP", "JWST"),
        ])
    }

    fn stamp_extension(extra: &[(&str, &str)]) -> HduHeader {
        let mut cards = vec![("XTENSION", "IMAGE"), ("BITPIX", "-32"), ("NAXIS", "2"), ("NAXIS1", "2"), ("NAXIS2", "2"), ("EXTNAME", "STAMP")];
        cards.extend_from_slice(extra);
        header(&cards)
    }

    #[test]
    fn a_primary_that_holds_data_gives_nothing_to_its_extensions() {
        let merged = primary_with_data().merge_with(&stamp_extension(&[]));
        for key in ["BUNIT", "CRVAL1", "DATAMAX", "NAXIS3", "SIMPLE", "EXTEND", "TELESCOP"] {
            assert_eq!(merged.get(key), None, "{key}");
            assert!(!merged.cards.iter().any(|(k, _)| k == key), "{key}");
        }
        assert_eq!(merged.get("EXTNAME"), Some("STAMP"));
        assert_eq!(merged.get_i64("NAXIS"), Some(2));
    }

    fn empty_primary() -> HduHeader {
        let mut primary = header(&[
            ("SIMPLE", "T"),
            ("BITPIX", "8"),
            ("NAXIS", "0"),
            ("EXTEND", "T"),
            ("TELESCOP", "JWST"),
            ("FILTER", "F200W"),
            ("EXPTIME", "10.5"),
            ("DATE-OBS", "2026-01-02"),
            ("BUNIT", "DN"),
            ("BSCALE", "2.0"),
            ("BZERO", "7.0"),
            ("BLANK", "-1"),
            ("DATAMIN", "0.0"),
            ("DATAMAX", "7.0"),
            ("CHECKSUM", "abc"),
            ("DATASUM", "0"),
            ("WCSAXES", "2"),
            ("CRPIX1", "1.0"),
            ("CRVAL1", "33.0"),
            ("CDELT1", "0.1"),
            ("CTYPE1", "RA---TAN"),
            ("CUNIT1", "deg"),
            ("CD1_1", "0.1"),
            ("PC1_1", "1.0"),
            ("PV1_1", "0.0"),
            ("CROTA2", "0.0"),
            ("LONPOLE", "180.0"),
            ("A_ORDER", "2"),
            ("A_0_2", "1e-6"),
            ("CRVAL1A", "34.0"),
        ]);
        primary.cards.push(("HISTORY".into(), "primary step".into()));
        primary
    }

    #[test]
    fn an_empty_primary_gives_observation_cards_but_never_structure_scaling_range_or_wcs() {
        let merged = empty_primary().merge_with(&stamp_extension(&[]));
        for key in ["TELESCOP", "FILTER", "EXPTIME", "DATE-OBS"] {
            assert!(merged.get(key).is_some(), "{key} is observation metadata of the whole file");
        }
        for key in [
            "SIMPLE", "EXTEND", "BUNIT", "BSCALE", "BZERO", "BLANK", "DATAMIN", "DATAMAX", "CHECKSUM", "DATASUM", "WCSAXES",
            "CRPIX1", "CRVAL1", "CDELT1", "CTYPE1", "CUNIT1", "CD1_1", "PC1_1", "PV1_1", "CROTA2", "LONPOLE", "A_ORDER",
            "A_0_2", "CRVAL1A",
        ] {
            assert_eq!(merged.get(key), None, "{key}");
            assert!(!merged.cards.iter().any(|(k, _)| k == key), "{key}");
        }
        assert_eq!(merged.get_i64("BITPIX"), Some(-32));
        assert_eq!(merged.get_i64("NAXIS"), Some(2));
        assert!(merged.cards.iter().any(|(k, v)| k == "HISTORY" && v == "primary step"));
        let indexed: HashSet<&String> = merged.index.keys().collect();
        let carded: HashSet<&String> = merged.cards.iter().map(|(k, _)| k).filter(|k| !is_commentary_key(k)).collect();
        assert_eq!(indexed, carded, "the index and the cards of a merged header name the same keys");
    }

    #[test]
    fn an_extension_with_inherit_f_takes_nothing_from_the_primary() {
        let merged = empty_primary().merge_with(&stamp_extension(&[("INHERIT", "F")]));
        for key in ["TELESCOP", "FILTER", "EXPTIME", "BUNIT", "CRVAL1"] {
            assert_eq!(merged.get(key), None, "{key}");
        }
        assert!(!merged.cards.iter().any(|(k, _)| k == "HISTORY"), "commentary of the primary is not inherited either");
        assert_eq!(merged.get("INHERIT"), Some("F"));
        assert_eq!(empty_primary().merge_with(&stamp_extension(&[("INHERIT", "T")])).get("TELESCOP"), Some("JWST"));
    }

    fn typed(cards: &[(&str, &str)], strings: &[&str]) -> HduHeader {
        let mut h = header(cards);
        h.string_keys = Some(strings.iter().map(|k| k.to_string()).collect());
        h
    }

    #[test]
    fn string_keys_follow_the_card_that_wins_the_merge() {
        let primary = typed(&[("PROGRAM", "01234"), ("VISIT", "001"), ("EXPTIME", "10")], &["PROGRAM", "VISIT", "EXPTIME"]);
        let ext = typed(&[("EXTNAME", "SCI"), ("EXPTIME", "12.5"), ("OBSNUM", "007")], &["EXTNAME"]);

        let merged = primary.merge_with(&ext);
        assert_eq!(merged.is_string_value("PROGRAM"), Some(true));
        assert_eq!(merged.is_string_value("VISIT"), Some(true));
        assert_eq!(merged.is_string_value("EXTNAME"), Some(true));
        assert_eq!(merged.is_string_value("EXPTIME"), Some(false), "the extension's numeric card wins");
        assert_eq!(merged.is_string_value("OBSNUM"), Some(false));

        let untyped = header(&[("PROGRAM", "01234")]).merge_with(&header(&[("EXTNAME", "SCI")]));
        assert_eq!(untyped.is_string_value("PROGRAM"), None, "headers built in code carry no source types");
    }

    #[test]
    fn string_keys_track_removal_and_numeric_updates_but_not_plain_updates() {
        let mut h = typed(&[("PROGRAM", "01234"), ("CRPIX1", "12.5"), ("OBJECT", "M16")], &["PROGRAM", "CRPIX1", "OBJECT"]);
        h.set("PROGRAM", "04321".into());
        assert_eq!(h.is_string_value("PROGRAM"), Some(true));
        h.set_f64("CRPIX1", 3.5);
        assert_eq!(h.is_string_value("CRPIX1"), Some(false));
        h.remove("OBJECT");
        h.set("OBJECT", "7".into());
        assert_eq!(h.is_string_value("OBJECT"), Some(false));

        let mut kept = header(&[("PROGRAM", "04321")]);
        kept.inherit_string_keys(&h);
        assert_eq!(kept.string_keys, Some(["PROGRAM".to_string()].into_iter().collect()));
        kept.inherit_string_keys(&HduHeader::empty());
        assert_eq!(kept.string_keys, None);
    }
}
