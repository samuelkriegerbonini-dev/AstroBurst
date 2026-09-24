use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};

use super::constants::BLOCK_SIZE;

pub const MAX_FITS_AXES: i64 = 999;

const COMMENTARY_KEYS: [&str; 2] = ["COMMENT", "HISTORY"];

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
        if let Some(existing) = self.cards.iter_mut().find(|(k, _)| k == key) {
            existing.1 = value.clone();
        } else {
            self.cards.push((key.to_string(), value.clone()));
        }
        self.index.insert(key.to_string(), value);
    }

    pub fn set_f64(&mut self, key: &str, value: f64) {
        self.set(key, format!("{:.14E}", value));
        if let Some(keys) = self.string_keys.as_mut() {
            keys.remove(key);
        }
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
        let skip_keys: &[&str] = &[
            "SIMPLE", "XTENSION", "EXTEND", "PCOUNT", "GCOUNT",
        ];

        let mut merged_index = self.index.clone();
        let mut merged_cards: Vec<(String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for (k, v) in &extension.cards {
            let ku = k.to_uppercase();
            if skip_keys.iter().any(|&sk| ku == sk) {
                continue;
            }
            merged_cards.push((k.clone(), v.clone()));
            if !is_commentary_key(k) {
                merged_index.insert(k.clone(), v.clone());
                seen.insert(k.clone());
            }
        }

        for (k, v) in &self.cards {
            let ku = k.to_uppercase();
            if skip_keys.iter().any(|&sk| ku == sk) {
                continue;
            }
            if is_commentary_key(k) || !seen.contains(k) {
                merged_cards.push((k.clone(), v.clone()));
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
