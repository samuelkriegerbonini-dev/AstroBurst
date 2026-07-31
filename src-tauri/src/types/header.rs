use std::collections::HashMap;

use super::constants::BLOCK_SIZE;

#[derive(Debug, Clone, serde::Serialize)]
pub struct HduHeader {
    #[allow(dead_code)]
    pub cards: Vec<(String, String)>,
    pub index: HashMap<String, String>,
}

impl HduHeader {
    pub fn empty() -> Self {
        Self {
            cards: Vec::new(),
            index: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.index.get(key).map(|s| s.as_str())
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.index.get(key)?.trim().parse().ok()
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.index.get(key)?.trim().parse().ok()
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
    }

    pub fn remove(&mut self, key: &str) {
        self.cards.retain(|(k, _)| k != key);
        self.index.remove(key);
    }

    pub fn data_byte_count(&self) -> usize {
        let naxis = self.get_i64("NAXIS").unwrap_or(0);
        if naxis == 0 {
            return 0;
        }
        let bitpix = self.get_i64("BITPIX").unwrap_or(0);
        let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
        let mut total: usize = 1;
        for i in 1..=naxis {
            total *= self.get_i64(&format!("NAXIS{}", i)).unwrap_or(1) as usize;
        }
        // General FITS data-unit size: GCOUNT * (PCOUNT + NAXIS1*...*NAXISn) * |BITPIX|/8.
        // PCOUNT/GCOUNT default to 0/1 for plain image HDUs (no-op here), but for a
        // BINTABLE extension PCOUNT is the heap (variable-length column data) byte
        // count appended immediately after the fixed-width table -- required to
        // correctly size compressed-image (ZIMAGE) extensions and any other HDU with
        // variable-length array columns, or HDU scanning desyncs past them.
        let pcount = self.get_i64("PCOUNT").unwrap_or(0).max(0) as usize;
        let gcount = self.get_i64("GCOUNT").unwrap_or(1).max(1) as usize;
        (total + pcount) * bytes_per_pixel * gcount
    }

    pub fn padded_data_bytes(&self) -> usize {
        let raw = self.data_byte_count();
        raw.div_ceil(BLOCK_SIZE) * BLOCK_SIZE
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
            merged_index.insert(k.clone(), v.clone());
            merged_cards.push((k.clone(), v.clone()));
            seen.insert(k.clone());
        }

        for (k, v) in &self.cards {
            let ku = k.to_uppercase();
            if skip_keys.iter().any(|&sk| ku == sk) {
                continue;
            }
            if !seen.contains(k) {
                merged_cards.push((k.clone(), v.clone()));
            }
        }

        HduHeader {
            cards: merged_cards,
            index: merged_index,
        }
    }
}
