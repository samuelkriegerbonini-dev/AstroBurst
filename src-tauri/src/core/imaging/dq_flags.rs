use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::imaging::sampling::{cell_range, preview_dims};
use crate::infra::fits::band_planes::PLANE_LABEL_CARD;
use crate::types::header::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DqTable {
    Jwst,
    Roman,
    Hst,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct DqFlag {
    pub bit: u32,
    pub name: &'static str,
}

const fn flag(bit: u32, name: &'static str) -> DqFlag {
    DqFlag { bit, name }
}

pub const JWST_FLAGS: &[DqFlag] = &[
    flag(1, "DO_NOT_USE"),
    flag(2, "SATURATED"),
    flag(4, "JUMP_DET"),
    flag(8, "DROPOUT"),
    flag(16, "OUTLIER"),
    flag(32, "PERSISTENCE"),
    flag(64, "AD_FLOOR"),
    flag(128, "CHARGELOSS"),
    flag(256, "UNRELIABLE_ERROR"),
    flag(512, "NON_SCIENCE"),
    flag(1024, "DEAD"),
    flag(2048, "HOT"),
    flag(4096, "WARM"),
    flag(8192, "LOW_QE"),
    flag(16384, "RC"),
    flag(32768, "TELEGRAPH"),
    flag(65536, "NONLINEAR"),
    flag(131072, "BAD_REF_PIXEL"),
    flag(262144, "NO_FLAT_FIELD"),
    flag(524288, "NO_GAIN_VALUE"),
    flag(1048576, "NO_LIN_CORR"),
    flag(2097152, "NO_SAT_CHECK"),
    flag(4194304, "UNRELIABLE_BIAS"),
    flag(8388608, "UNRELIABLE_DARK"),
    flag(16777216, "UNRELIABLE_SLOPE"),
    flag(33554432, "UNRELIABLE_FLAT"),
    flag(67108864, "OPEN"),
    flag(134217728, "ADJ_OPEN"),
    flag(268435456, "FLUX_ESTIMATED"),
    flag(536870912, "MSA_FAILED_OPEN"),
    flag(1073741824, "OTHER_BAD_PIXEL"),
    flag(2147483648, "REFERENCE_PIXEL"),
];

pub const ROMAN_FLAGS: &[DqFlag] = &[
    flag(1, "DO_NOT_USE"),
    flag(2, "SATURATED"),
    flag(4, "JUMP_DET"),
    flag(8, "DROPOUT"),
    flag(16, "GW_AFFECTED_DATA"),
    flag(32, "PERSISTENCE"),
    flag(64, "AD_FLOOR"),
    flag(128, "OUTLIER"),
    flag(256, "UNRELIABLE_ERROR"),
    flag(512, "NON_SCIENCE"),
    flag(1024, "DEAD"),
    flag(2048, "HOT"),
    flag(4096, "WARM"),
    flag(8192, "LOW_QE"),
    flag(32768, "TELEGRAPH"),
    flag(65536, "NONLINEAR"),
    flag(131072, "BAD_REF_PIXEL"),
    flag(262144, "NO_FLAT_FIELD"),
    flag(524288, "NO_GAIN_VALUE"),
    flag(1048576, "NO_LIN_CORR"),
    flag(2097152, "NO_SAT_CHECK"),
    flag(4194304, "UNRELIABLE_BIAS"),
    flag(8388608, "UNRELIABLE_DARK"),
    flag(16777216, "UNRELIABLE_SLOPE"),
    flag(33554432, "UNRELIABLE_FLAT"),
    flag(268435456, "UNRELIABLE_RESET"),
    flag(1073741824, "OTHER_BAD_PIXEL"),
    flag(2147483648, "REFERENCE_PIXEL"),
];

pub const HST_FLAGS: &[DqFlag] = &[
    flag(1, "REED_SOLOMON"),
    flag(2, "REPLACED_FILL"),
    flag(4, "BAD_DETECTOR"),
    flag(8, "MASKED"),
    flag(16, "HOT"),
    flag(32, "CTE_TAIL"),
    flag(64, "WARM"),
    flag(128, "BAD_BIAS"),
    flag(256, "SATURATED"),
    flag(512, "BAD_FLAT"),
    flag(1024, "CHARGE_TRAP"),
    flag(2048, "ATOD_SATURATED"),
    flag(4096, "COSMIC_RAY"),
    flag(8192, "COSMIC_RAY_SINGLE"),
    flag(16384, "MANUAL"),
];

pub(crate) const TELESCOPE_KEYS: &[&str] = &[
    "TELESCOP",
    "INSTRUME",
    "META_TELESCOPE",
    "META_INSTRUMENT_NAME",
    "ROMAN_META_TELESCOPE",
    "ROMAN_META_INSTRUMENT_NAME",
];

const TELESCOPE_CARDS: &[&str] = &["TELESCOP", "META_TELESCOPE", "ROMAN_META_TELESCOPE"];
const INSTRUMENT_CARDS: &[&str] = &["INSTRUME", "META_INSTRUMENT_NAME", "ROMAN_META_INSTRUMENT_NAME"];
const JWST_TELESCOPE: &str = "JWST";
const ROMAN_TELESCOPES: &[&str] = &["ROMAN", "WFIRST"];
const HST_TELESCOPE: &str = "HST";
const JWST_INSTRUMENTS: &[&str] = &["NIRCAM", "NIRISS", "NIRSPEC", "MIRI", "FGS"];
const ROMAN_INSTRUMENT: &str = "WFI";
const HST_INSTRUMENTS: &[&str] = &["ACS", "WFC3"];
const HST_INSTRUMENTS_WITH_OTHER_FLAGS: &[&str] = &["STIS", "COS", "WFPC2"];

fn card_tokens(value: &str) -> Vec<String> {
    value
        .to_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

fn holds_any(tokens: &[String], markers: &[&str]) -> bool {
    tokens.iter().any(|token| markers.contains(&token.as_str()))
}

impl DqTable {
    pub fn select(header: &HduHeader) -> DqTable {
        let first_present = |cards: &[&str]| cards.iter().find_map(|card| header.get(card)).map(card_tokens);
        if let Some(telescope) = first_present(TELESCOPE_CARDS) {
            if holds_any(&telescope, &[JWST_TELESCOPE]) {
                return DqTable::Jwst;
            }
            if holds_any(&telescope, ROMAN_TELESCOPES) {
                return DqTable::Roman;
            }
            let other_flags = INSTRUMENT_CARDS
                .iter()
                .filter_map(|card| header.get(card))
                .any(|instrument| holds_any(&card_tokens(instrument), HST_INSTRUMENTS_WITH_OTHER_FLAGS));
            return if holds_any(&telescope, &[HST_TELESCOPE]) && !other_flags { DqTable::Hst } else { DqTable::Unknown };
        }
        match first_present(INSTRUMENT_CARDS) {
            Some(instrument) if holds_any(&instrument, JWST_INSTRUMENTS) => DqTable::Jwst,
            Some(instrument) if holds_any(&instrument, &[ROMAN_INSTRUMENT]) => DqTable::Roman,
            Some(instrument) if holds_any(&instrument, HST_INSTRUMENTS) => DqTable::Hst,
            _ => DqTable::Unknown,
        }
    }

    pub fn or_else(self, fallback: impl FnOnce() -> DqTable) -> DqTable {
        match self {
            DqTable::Unknown => fallback(),
            named => named,
        }
    }

    pub fn flags(self) -> &'static [DqFlag] {
        match self {
            DqTable::Hst => HST_FLAGS,
            DqTable::Roman => ROMAN_FLAGS,
            DqTable::Jwst => JWST_FLAGS,
            DqTable::Unknown => &[],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DqTable::Jwst => "jwst",
            DqTable::Roman => "roman",
            DqTable::Hst => "hst",
            DqTable::Unknown => "unknown (raw bits)",
        }
    }

    pub fn id(self) -> u32 {
        match self {
            DqTable::Jwst => 0,
            DqTable::Roman => 1,
            DqTable::Hst => 2,
            DqTable::Unknown => 0xFFFF_FFFF,
        }
    }

    pub fn decode(self, bits: u32) -> Vec<String> {
        let flags = self.flags();
        (0..32)
            .filter(|n| bits & (1u32 << n) != 0)
            .map(|n| {
                let bit = 1u32 << n;
                flags
                    .iter()
                    .find(|f| f.bit == bit)
                    .map(|f| f.name.to_string())
                    .unwrap_or_else(|| format!("BIT{}", n))
            })
            .collect()
    }

    pub fn format(self, bits: u32) -> String {
        if bits == 0 {
            return "0: GOOD".to_string();
        }
        format!("{}: {}", bits, self.decode(bits).join(" | "))
    }

    pub fn mask_from_names(self, names: &[&str]) -> Result<u32, String> {
        let mut mask = 0u32;
        for name in names {
            let wanted = name.trim();
            let named = self.flags().iter().find(|f| f.name.eq_ignore_ascii_case(wanted)).map(|f| f.bit);
            match named.or_else(|| raw_bit(wanted)) {
                Some(bit) => mask |= bit,
                None => return Err(format!("unknown DQ flag name '{}' for table {}", wanted, self.label())),
            }
        }
        Ok(mask)
    }

    pub fn default_overlay_mask(self) -> u32 {
        match self {
            DqTable::Hst => 4 | 256 | 4096 | 8192,
            DqTable::Jwst | DqTable::Roman => 7,
            DqTable::Unknown => 0,
        }
    }

    pub fn exclusion_mask(self) -> u32 {
        match self {
            DqTable::Hst => 0x3FFF,
            DqTable::Jwst | DqTable::Roman => 1,
            DqTable::Unknown => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FlagSet {
    pub table: DqTable,
    pub flags: Vec<(u32, String)>,
    pub exclusion_mask: u32,
    pub exclusion_default: u32,
    pub overlay_mask: u32,
    pub saturated_mask: u32,
    pub source: String,
}

const TABLE_SATURATED_NAME: &str = "SATURATED";
const SOURCE_MARKER_NAMES: &[&str] = &["DETECTED", "DETECTED_NEGATIVE"];

const LONG_KEY_PREFIX: &str = "HIERARCH ";
const AFW_CARD_PREFIX: &str = "MP_";
const AFW_SOURCE: &str = "MP_ cards (afw mask planes)";
const AFW_EXCLUDED_NAMES: &[&str] = &["BAD", "SAT", "CR", "NO_DATA"];
const AFW_OVERLAY_ONLY_NAMES: &[&str] = &["EDGE", "INTRP", "SUSPECT"];
const AFW_SATURATED_NAME: &str = "SAT";

const LEGACY_CARD_PREFIX: &str = "MBIT_";
const LEGACY_CARD_SOURCE: &str = "MBIT_ cards (Legacy maskbits)";
const LEGACY_BUILT_IN_SOURCE: &str = "Legacy DR10 maskbits (built-in table)";
const LEGACY_LAYER_CARDS: &[&str] = &["IMAGETYP", "IMTYPE"];
const LEGACY_LAYER_VALUE: &str = "MASKBITS";
const LEGACY_SUB_BLOB_CARD: &str = "MB_SUBBL";
const LEGACY_SUB_BLOB_BIT: u32 = 1 << 16;
const LEGACY_SUB_BLOB_NAME: &str = "SUB_BLOB";
const LEGACY_SATURATED_PREFIX: &str = "SATUR_";
const LEGACY_ALL_MASKED_PREFIX: &str = "ALLMASK_";
const LEGACY_OVERLAY_ONLY_NAMES: &[&str] = &["BRIGHT", "MEDIUM", "GALAXY", "CLUSTER"];
const LEGACY_BANDS: &[&str] = &["g", "r", "i", "z"];
const LEGACY_DR10_NAMES: [&str; 17] = [
    "NPRIMARY",
    "BRIGHT",
    "SATUR_G",
    "SATUR_R",
    "SATUR_Z",
    "ALLMASK_G",
    "ALLMASK_R",
    "ALLMASK_Z",
    "WISEM1",
    "WISEM2",
    "BAILOUT",
    "MEDIUM",
    "GALAXY",
    "CLUSTER",
    "SATUR_I",
    "ALLMASK_I",
    "SUB_BLOB",
];

const PS1_NAME_CARD_PREFIX: &str = "MSKNAM";
const PS1_VALUE_CARD_PREFIX: &str = "MSKVAL";
const PS1_FIRST_NAME_CARD: &str = "MSKNAM00";
const PS1_SOURCE: &str = "MSKNAM cards (PS1 mask)";
const PS1_DEFAULT_EXCLUSION_NAME: &str = "MASK.VALUE";
const PS1_MARK_CARD_NAME: &str = "MARK.VALUE";
const PS1_MARK_FLAG_NAME: &str = "MARK";
const PS1_SATURATED_NAME: &str = "SAT";

const DES_LAYER_CARD: &str = "DES_EXT";
const DES_MASK_VALUE: &str = "MASK";
const DES_SOURCE: &str = "DES coadd mask (non-zero = no good data)";

fn bit_of_index(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|n| *n < 32).map(|n| 1u32 << n)
}

fn bits_where(flags: &[(u32, String)], wanted: impl Fn(&str) -> bool) -> u32 {
    flags.iter().filter(|(_, name)| wanted(name)).fold(0, |mask, (bit, _)| mask | bit)
}

fn bits_named(flags: &[(u32, String)], names: &[&str]) -> u32 {
    bits_where(flags, |name| names.iter().any(|wanted| name.eq_ignore_ascii_case(wanted)))
}

fn bits_prefixed(flags: &[(u32, String)], prefix: &str) -> u32 {
    bits_where(flags, |name| name.get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix)))
}

fn afw_flags(mask: &HduHeader) -> Vec<(u32, String)> {
    mask.cards
        .iter()
        .filter_map(|(key, value)| {
            let key = key.trim();
            let short_key = key.strip_prefix(LONG_KEY_PREFIX).unwrap_or(key);
            let name = short_key.strip_prefix(AFW_CARD_PREFIX).filter(|name| !name.is_empty())?;
            Some((bit_of_index(value)?, name.to_string()))
        })
        .collect()
}

fn legacy_card_flags(mask: &HduHeader) -> Vec<(u32, String)> {
    let mut flags: Vec<(u32, String)> = mask
        .cards
        .iter()
        .filter_map(|(key, value)| {
            let bit = bit_of_index(key.trim().strip_prefix(LEGACY_CARD_PREFIX)?)?;
            let name = value.trim();
            (!name.is_empty() && mask.is_string_value(key) != Some(false)).then(|| (bit, name.to_string()))
        })
        .collect();
    let sub_blob_declared = mask.get_i64(LEGACY_SUB_BLOB_CARD) == Some(i64::from(LEGACY_SUB_BLOB_BIT));
    if !flags.is_empty() && sub_blob_declared && flags.iter().all(|(bit, _)| *bit != LEGACY_SUB_BLOB_BIT) {
        flags.push((LEGACY_SUB_BLOB_BIT, LEGACY_SUB_BLOB_NAME.to_string()));
    }
    flags
}

fn legacy_built_in_flags() -> Vec<(u32, String)> {
    LEGACY_DR10_NAMES.iter().enumerate().map(|(n, name)| (1u32 << n, name.to_string())).collect()
}

fn is_legacy_maskbits_layer(mask: &HduHeader) -> bool {
    LEGACY_LAYER_CARDS
        .iter()
        .any(|card| mask.get(card).is_some_and(|value| value.trim().eq_ignore_ascii_case(LEGACY_LAYER_VALUE)))
}

fn legacy_band(image: &HduHeader) -> Option<String> {
    let band = image
        .get("FILTERX")
        .or_else(|| image.get("FILTER").map(|filter| filter.split_whitespace().next().unwrap_or("")))
        .or_else(|| image.get("BAND"))
        .or_else(|| image.get(PLANE_LABEL_CARD))?
        .trim()
        .to_lowercase();
    LEGACY_BANDS.contains(&band.as_str()).then_some(band)
}

fn ps1_pairs(mask: &HduHeader) -> Vec<(String, u32)> {
    (0..)
        .map_while(|nn| {
            let name = mask.get(&format!("{PS1_NAME_CARD_PREFIX}{nn:02}"))?;
            let value = mask.get(&format!("{PS1_VALUE_CARD_PREFIX}{nn:02}"))?;
            Some((name.trim().to_string(), value.trim().parse::<u32>().ok()))
        })
        .filter_map(|(name, value)| Some((name, value?)))
        .collect()
}

impl FlagSet {
    fn assembled(
        table: DqTable,
        mut flags: Vec<(u32, String)>,
        exclusion_default: u32,
        overlay_mask: u32,
        saturated_mask: u32,
        source: &str,
    ) -> FlagSet {
        flags.sort_by_key(|(bit, _)| *bit);
        FlagSet {
            table,
            flags,
            exclusion_mask: exclusion_default,
            exclusion_default,
            overlay_mask,
            saturated_mask,
            source: source.to_string(),
        }
    }

    pub fn from_table(table: DqTable) -> FlagSet {
        let flags: Vec<(u32, String)> = table.flags().iter().map(|f| (f.bit, f.name.to_string())).collect();
        let saturated_mask = bits_named(&flags, &[TABLE_SATURATED_NAME]);
        Self::assembled(
            table,
            flags,
            table.exclusion_mask(),
            table.default_overlay_mask(),
            saturated_mask,
            table.label(),
        )
    }

    fn afw(table: DqTable, flags: Vec<(u32, String)>) -> FlagSet {
        let excluded = bits_named(&flags, AFW_EXCLUDED_NAMES);
        let overlay = excluded | bits_named(&flags, AFW_OVERLAY_ONLY_NAMES);
        let saturated = bits_named(&flags, &[AFW_SATURATED_NAME]);
        Self::assembled(table, flags, excluded, overlay, saturated, AFW_SOURCE)
    }

    fn legacy(table: DqTable, flags: Vec<(u32, String)>, image: Option<&HduHeader>, source: &str) -> FlagSet {
        let (saturated, all_masked) = match image.and_then(legacy_band) {
            Some(band) => (
                bits_named(&flags, &[format!("{LEGACY_SATURATED_PREFIX}{band}").as_str()]),
                bits_named(&flags, &[format!("{LEGACY_ALL_MASKED_PREFIX}{band}").as_str()]),
            ),
            None => (bits_prefixed(&flags, LEGACY_SATURATED_PREFIX), bits_prefixed(&flags, LEGACY_ALL_MASKED_PREFIX)),
        };
        let excluded = saturated | all_masked;
        let overlay = excluded | bits_named(&flags, LEGACY_OVERLAY_ONLY_NAMES);
        Self::assembled(table, flags, excluded, overlay, saturated, source)
    }

    fn ps1(table: DqTable, mask: &HduHeader) -> FlagSet {
        let mut flags: Vec<(u32, String)> = Vec::new();
        let mut excluded = 0;
        let mut saturated = 0;
        for (name, value) in ps1_pairs(mask) {
            if name == PS1_DEFAULT_EXCLUSION_NAME {
                excluded = value;
                continue;
            }
            if value.count_ones() != 1 {
                continue;
            }
            if name == PS1_SATURATED_NAME {
                saturated |= value;
            }
            let shown = if name == PS1_MARK_CARD_NAME { PS1_MARK_FLAG_NAME } else { name.as_str() };
            match flags.iter_mut().find(|(bit, _)| *bit == value) {
                Some((_, joined)) => {
                    joined.push('/');
                    joined.push_str(shown);
                }
                None => flags.push((value, shown.to_string())),
            }
        }
        Self::assembled(table, flags, excluded, excluded, saturated, PS1_SOURCE)
    }

    pub fn from_headers(mask: Option<&HduHeader>, image: Option<&HduHeader>, table: DqTable) -> FlagSet {
        let Some(mask) = mask.filter(|_| table == DqTable::Unknown) else {
            return Self::from_table(table);
        };
        let afw = afw_flags(mask);
        if !afw.is_empty() {
            return Self::afw(table, afw);
        }
        let legacy = legacy_card_flags(mask);
        if !legacy.is_empty() {
            return Self::legacy(table, legacy, image, LEGACY_CARD_SOURCE);
        }
        if is_legacy_maskbits_layer(mask) {
            return Self::legacy(table, legacy_built_in_flags(), image, LEGACY_BUILT_IN_SOURCE);
        }
        if mask.get(PS1_FIRST_NAME_CARD).is_some() {
            return Self::ps1(table, mask);
        }
        if mask.get(DES_LAYER_CARD).is_some_and(|value| value.trim().eq_ignore_ascii_case(DES_MASK_VALUE)) {
            return Self::assembled(table, Vec::new(), u32::MAX, u32::MAX, 0, DES_SOURCE);
        }
        Self::from_table(table)
    }

    pub fn with_exclusion(mut self, mask: Option<u32>) -> FlagSet {
        self.exclusion_mask = match mask {
            Some(chosen) => chosen & !bits_named(&self.flags, SOURCE_MARKER_NAMES),
            None => self.exclusion_default,
        };
        self
    }

    pub fn decode(&self, bits: u32) -> Vec<String> {
        (0..32)
            .filter(|n| bits & (1u32 << n) != 0)
            .map(|n| {
                let bit = 1u32 << n;
                self.flags
                    .iter()
                    .find(|(named, _)| *named == bit)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| format!("BIT{}", n))
            })
            .collect()
    }

    pub fn format(&self, bits: u32) -> String {
        if bits == 0 {
            return "0: GOOD".to_string();
        }
        format!("{}: {}", bits, self.decode(bits).join(" | "))
    }

    pub fn mask_from_names(&self, names: &[&str]) -> Result<u32, String> {
        let mut mask = 0u32;
        for name in names {
            let wanted = name.trim();
            let named = self.flags.iter().find(|(_, known)| known.eq_ignore_ascii_case(wanted)).map(|(bit, _)| *bit);
            match named.or_else(|| raw_bit(wanted)) {
                Some(bit) => mask |= bit,
                None => return Err(format!("unknown flag name '{}' for {}", wanted, self.source)),
            }
        }
        Ok(mask)
    }

    pub fn is_named(&self) -> bool {
        !self.flags.is_empty()
    }
}

fn raw_bit(name: &str) -> Option<u32> {
    let digits = name.get(..3).filter(|prefix| prefix.eq_ignore_ascii_case("BIT")).and(name.get(3..))?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|n| *n < 32).map(|n| 1u32 << n)
}

pub fn bits_present(bits: &Array2<u32>) -> u32 {
    match bits.as_slice() {
        Some(src) => src
            .par_chunks(1 << 16)
            .map(|chunk| chunk.iter().fold(0, |acc, &b| acc | b))
            .reduce(|| 0, |a, b| a | b),
        None => bits.iter().fold(0, |acc, &b| acc | b),
    }
}

pub fn exclusion_map(bits: &Array2<u32>, mask: u32) -> Array2<u8> {
    let (rows, cols) = bits.dim();
    let mut out = Array2::<u8>::zeros((rows, cols));
    if let (Some(src), Some(dst)) = (bits.as_slice(), out.as_slice_mut()) {
        dst.par_chunks_mut(cols.max(1))
            .zip(src.par_chunks(cols.max(1)))
            .for_each(|(d, s)| {
                for (o, &b) in d.iter_mut().zip(s) {
                    *o = (b & mask != 0) as u8;
                }
            });
    } else {
        for ((y, x), b) in bits.indexed_iter() {
            out[[y, x]] = (b & mask != 0) as u8;
        }
    }
    out
}

pub fn apply_exclusion(data: &Array2<f32>, excluded: &Array2<u8>) -> Result<Array2<f32>> {
    if data.dim() != excluded.dim() {
        bail!(
            "DQ mask dimensions {:?} do not match image dimensions {:?}",
            excluded.dim(),
            data.dim()
        );
    }
    let mut out = data.to_owned();
    let (_, cols) = out.dim();
    if let (Some(dst), Some(mask)) = (out.as_slice_mut(), excluded.as_slice()) {
        dst.par_chunks_mut(cols.max(1))
            .zip(mask.par_chunks(cols.max(1)))
            .for_each(|(d, m)| {
                for (v, &e) in d.iter_mut().zip(m) {
                    if e != 0 {
                        *v = f32::NAN;
                    }
                }
            });
    } else {
        for ((y, x), v) in out.indexed_iter_mut() {
            if excluded[[y, x]] != 0 {
                *v = f32::NAN;
            }
        }
    }
    Ok(out)
}

pub fn mask_preview_or(bits: &Array2<u32>, mask: u32, max_dim: usize) -> (Vec<u8>, usize, usize) {
    let (rows, cols) = bits.dim();
    if rows == 0 || cols == 0 {
        return (Vec::new(), cols, rows);
    }
    let (dst_rows, dst_cols) = preview_dims(rows, cols, max_dim);
    let flagged = exclusion_map(bits, mask);
    let src: std::borrow::Cow<[u8]> = match flagged.as_slice() {
        Some(s) => std::borrow::Cow::Borrowed(s),
        None => std::borrow::Cow::Owned(flagged.iter().copied().collect()),
    };
    let mut cells = vec![0u8; dst_rows * dst_cols];
    cells
        .par_chunks_mut(dst_cols)
        .enumerate()
        .for_each(|(dy, out_row)| {
            let (sy0, sy1) = cell_range(dy, rows, dst_rows);
            for (dx, cell) in out_row.iter_mut().enumerate() {
                let (sx0, sx1) = cell_range(dx, cols, dst_cols);
                let hit = (sy0..sy1).any(|yy| src[yy * cols + sx0..yy * cols + sx1].iter().any(|&v| v != 0));
                *cell = hit as u8;
            }
        });
    (cells, dst_cols, dst_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_with(pairs: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in pairs {
            h.set(k, v.to_string());
        }
        h
    }

    #[test]
    fn every_jwst_bit_decodes_to_its_name() {
        assert_eq!(JWST_FLAGS.len(), 32);
        for (n, f) in JWST_FLAGS.iter().enumerate() {
            assert_eq!(f.bit, 1u32 << n, "{}", f.name);
            assert_eq!(DqTable::Jwst.decode(f.bit), vec![f.name.to_string()]);
        }
        assert_eq!(DqTable::Jwst.decode(1 << 31), vec!["REFERENCE_PIXEL".to_string()]);
        assert_eq!(DqTable::Jwst.decode(0), Vec::<String>::new());
    }

    #[test]
    fn format_lists_set_bits_ascending() {
        assert_eq!(DqTable::Jwst.format(3), "3: DO_NOT_USE | SATURATED");
        assert_eq!(DqTable::Jwst.format(0), "0: GOOD");
        assert_eq!(DqTable::Jwst.format(2147483649), "2147483649: DO_NOT_USE | REFERENCE_PIXEL");
        assert_eq!(DqTable::Hst.format(1 << 20), "1048576: BIT20");
        assert_eq!(DqTable::Hst.decode(4 | (1 << 20)), vec!["BAD_DETECTOR".to_string(), "BIT20".to_string()]);
    }

    #[test]
    fn mask_from_names_roundtrip_and_unknown_name() {
        let mask = DqTable::Jwst.mask_from_names(&["DO_NOT_USE", "jump_det", "REFERENCE_PIXEL"]).unwrap();
        assert_eq!(mask, 1 | 4 | (1 << 31));
        assert_eq!(DqTable::Jwst.decode(mask), vec!["DO_NOT_USE", "JUMP_DET", "REFERENCE_PIXEL"]);
        assert!(DqTable::Jwst.mask_from_names(&["COSMIC_RAY"]).is_err());
        assert_eq!(DqTable::Hst.mask_from_names(&["COSMIC_RAY"]).unwrap(), 4096);
        assert_eq!(DqTable::Hst.mask_from_names(&[]).unwrap(), 0);
    }

    #[test]
    fn select_from_header_keys() {
        assert_eq!(DqTable::select(&header_with(&[("TELESCOP", "JWST")])), DqTable::Jwst);
        assert_eq!(DqTable::select(&header_with(&[("INSTRUME", "WFC3")])), DqTable::Hst);
        assert_eq!(DqTable::select(&header_with(&[("META_TELESCOPE", "ROMAN")])), DqTable::Roman);
        assert_eq!(DqTable::select(&header_with(&[("ROMAN_META_INSTRUMENT_NAME", "WFI")])), DqTable::Roman);
        assert_eq!(DqTable::select(&header_with(&[("INSTRUME", "nircam")])), DqTable::Jwst);
        assert_eq!(DqTable::select(&header_with(&[("TELESCOP", "Gemini")])), DqTable::Unknown);
        assert_eq!(DqTable::select(&HduHeader::empty()), DqTable::Unknown);
        assert_eq!(DqTable::select(&header_with(&[("TELESCOP", "HST"), ("INSTRUME", "MIRI")])), DqTable::Hst);
    }

    #[test]
    fn a_marker_inside_another_name_selects_no_table() {
        for cards in [
            &[("TELESCOP", "Baade_Mag_1"), ("INSTRUME", "IMACS")][..],
            &[("TELESCOP", "Herschel Space Observatory"), ("INSTRUME", "PACS")],
            &[("INSTRUME", "COSMOS")],
            &[("TELESCOP", "Keck II"), ("INSTRUME", "NIRSPEC")],
            &[("TELESCOP", "ESO-2P2"), ("INSTRUME", "WFI")],
            &[("TELESCOP", "CTIO 4.0-m telescope"), ("INSTRUME", "DECam")],
        ] {
            assert_eq!(DqTable::select(&header_with(cards)), DqTable::Unknown, "{cards:?}");
        }
    }

    #[test]
    fn stis_cos_and_wfpc2_do_not_take_the_acs_and_wfc3_table() {
        for instrument in ["STIS", "COS", "WFPC2"] {
            assert_eq!(
                DqTable::select(&header_with(&[("TELESCOP", "HST"), ("INSTRUME", instrument)])),
                DqTable::Unknown,
                "{instrument}"
            );
        }
        for cards in [
            &[("TELESCOP", "HST"), ("INSTRUME", "ACS")][..],
            &[("TELESCOP", "HST"), ("INSTRUME", "WFC3")],
            &[("TELESCOP", "HST")],
            &[("INSTRUME", "ACS")],
        ] {
            assert_eq!(DqTable::select(&header_with(cards)), DqTable::Hst, "{cards:?}");
        }
        assert_eq!(DqTable::select(&header_with(&[("INSTRUME", "STIS")])), DqTable::Unknown);
    }

    #[test]
    fn wfirst_and_roman_select_the_roman_table() {
        for telescope in ["WFIRST", "ROMAN", "Nancy Grace Roman Space Telescope"] {
            assert_eq!(DqTable::select(&header_with(&[("TELESCOP", telescope)])), DqTable::Roman, "{telescope}");
        }
    }

    fn named(pairs: &[(u32, &str)]) -> Vec<(u32, String)> {
        pairs.iter().map(|(bit, name)| (*bit, name.to_string())).collect()
    }

    fn afw_mask_header() -> HduHeader {
        header_with(&[
            ("MP_BAD", "0"),
            ("MP_SAT", "1"),
            ("MP_INTRP", "2"),
            ("MP_CR", "3"),
            ("MP_EDGE", "4"),
            ("MP_DETECTED", "5"),
            ("MP_DETECTED_NEGATIVE", "6"),
            ("MP_SUSPECT", "7"),
            ("MP_NO_DATA", "8"),
        ])
    }

    const LEGACY_NAMES: [&str; 17] = [
        "NPRIMARY", "BRIGHT", "SATUR_G", "SATUR_R", "SATUR_Z", "ALLMASK_G", "ALLMASK_R", "ALLMASK_Z", "WISEM1",
        "WISEM2", "BAILOUT", "MEDIUM", "GALAXY", "CLUSTER", "SATUR_I", "ALLMASK_I", "SUB_BLOB",
    ];

    fn legacy_flags() -> Vec<(u32, String)> {
        LEGACY_NAMES.iter().enumerate().map(|(n, name)| (1u32 << n, name.to_string())).collect()
    }

    fn legacy_mask_header() -> HduHeader {
        let mut header = HduHeader::empty();
        for (n, name) in LEGACY_NAMES.iter().take(16).enumerate() {
            header.set(&format!("MBIT_{n}"), name.to_string());
        }
        header.set("MB_SUBBL", "65536".to_string());
        header
    }

    #[test]
    fn built_in_sets_keep_the_masks_of_their_tables() {
        for (table, saturated) in [(DqTable::Jwst, 2), (DqTable::Roman, 2), (DqTable::Hst, 256), (DqTable::Unknown, 0)] {
            let set = FlagSet::from_table(table);
            assert_eq!(set.table, table);
            assert_eq!(set.exclusion_mask, table.exclusion_mask(), "{table:?}");
            assert_eq!(set.exclusion_default, table.exclusion_mask(), "{table:?}");
            assert_eq!(set.overlay_mask, table.default_overlay_mask(), "{table:?}");
            assert_eq!(set.flags.len(), table.flags().len(), "{table:?}");
            assert_eq!(set.source, table.label());
            assert_eq!(set.saturated_mask, saturated, "{table:?}");
            assert_eq!(set.is_named(), table != DqTable::Unknown);
            if table != DqTable::Unknown {
                assert_eq!(set.saturated_mask, table.mask_from_names(&["SATURATED"]).unwrap());
                assert_eq!(FlagSet::from_headers(None, None, table), set);
                assert_eq!(FlagSet::from_headers(Some(&afw_mask_header()), None, table), set);
            }
        }
        assert_eq!(FlagSet::from_table(DqTable::Jwst).format(3), "3: DO_NOT_USE | SATURATED");
    }

    #[test]
    fn afw_mask_planes_take_names_and_bits_from_the_mp_cards() {
        let set = FlagSet::from_headers(Some(&afw_mask_header()), None, DqTable::Unknown);
        assert_eq!(
            set.flags,
            named(&[
                (1, "BAD"),
                (2, "SAT"),
                (4, "INTRP"),
                (8, "CR"),
                (16, "EDGE"),
                (32, "DETECTED"),
                (64, "DETECTED_NEGATIVE"),
                (128, "SUSPECT"),
                (256, "NO_DATA"),
            ])
        );
        assert_eq!(set.table, DqTable::Unknown);
        assert_eq!(set.exclusion_default, 267);
        assert_eq!(set.exclusion_mask, 267);
        assert_eq!(set.overlay_mask, 415);
        assert_eq!(set.saturated_mask, 2);
        assert_eq!(set.source, "MP_ cards (afw mask planes)");
        assert_eq!(set.decode(2 | 32), vec!["SAT".to_string(), "DETECTED".to_string()]);
        assert_eq!(set.format(0), "0: GOOD");
        assert_eq!(set.format(1 << 20), "1048576: BIT20");
        assert_eq!(set.format(1 | 8), "9: BAD | CR");
        assert!(set.is_named());

        let mut as_written = HduHeader::empty();
        for (key, value) in &afw_mask_header().cards {
            let card = if key.len() > 8 { format!("HIERARCH {key}") } else { key.clone() };
            as_written.set(&card, value.clone());
        }
        assert!(as_written.get("HIERARCH MP_DETECTED_NEGATIVE").is_some());
        assert_eq!(
            FlagSet::from_headers(Some(&as_written), None, DqTable::Unknown),
            set,
            "a name longer than a FITS keyword is written as a HIERARCH card"
        );
    }

    #[test]
    fn shuffled_mp_cards_are_read_from_the_file_and_never_from_a_fixed_table() {
        let mask = header_with(&[
            ("MP_SAT", "7"),
            ("MP_BAD", "3"),
            ("MP_DETECTED", "0"),
            ("MP_NO_DATA", "12"),
            ("MP_CR", "1"),
            ("MP_JUNK", "40"),
        ]);
        let set = FlagSet::from_headers(Some(&mask), None, DqTable::Unknown);
        assert_eq!(set.exclusion_default, 4234);
        assert_eq!(set.saturated_mask, 128);
        assert_eq!(
            set.flags,
            named(&[(1, "DETECTED"), (2, "CR"), (8, "BAD"), (128, "SAT"), (4096, "NO_DATA")]),
            "ascending by bit, and bit index 40 names no flag"
        );
        assert_eq!(set.exclusion_default & 1, 0, "DETECTED marks sources and is never excluded");
    }

    #[test]
    fn legacy_maskbits_take_their_names_from_the_mbit_cards_and_exclude_the_band_of_the_image() {
        let mask = legacy_mask_header();
        let set = FlagSet::from_headers(Some(&mask), Some(&header_with(&[("FILTERX", "r")])), DqTable::Unknown);
        assert_eq!(set.flags, legacy_flags());
        assert_eq!(set.flags.last(), Some(&(65536, "SUB_BLOB".to_string())));
        assert_eq!(set.exclusion_default, 72);
        assert_eq!(set.exclusion_mask, 72);
        assert_eq!(set.saturated_mask, 8);
        assert_eq!(set.overlay_mask, 14410);
        assert_eq!(set.source, "MBIT_ cards (Legacy maskbits)");
        assert_eq!(set.decode(51352), vec!["SATUR_R", "SATUR_Z", "ALLMASK_Z", "MEDIUM", "SATUR_I", "ALLMASK_I"]);

        let decam = header_with(&[("FILTER", "g DECam SDSS c0001 4720.0 1520.0")]);
        let g = FlagSet::from_headers(Some(&mask), Some(&decam), DqTable::Unknown);
        assert_eq!(g.exclusion_default, 36);
        assert_eq!(g.saturated_mask, 4);

        for image in [None, Some(HduHeader::empty()), Some(header_with(&[("BAND", "Y")]))] {
            let any_band = FlagSet::from_headers(Some(&mask), image.as_ref(), DqTable::Unknown);
            assert_eq!(any_band.exclusion_default, 49404, "{image:?}");
            assert_eq!(any_band.saturated_mask, 16412, "{image:?}");
            assert_eq!(any_band.overlay_mask, 49404 + 14338, "{image:?}");
        }
        let never_excluded = set.mask_from_names(&["NPRIMARY", "WISEM1", "WISEM2", "BAILOUT"]).unwrap();
        assert_eq!(never_excluded, 1 | 256 | 512 | 1024);
        assert_eq!(FlagSet::from_headers(Some(&mask), None, DqTable::Unknown).exclusion_default & never_excluded, 0);

        let mut with_bit_16 = legacy_mask_header();
        with_bit_16.set("MBIT_16", "RESOLVED".to_string());
        let named_by_card = FlagSet::from_headers(Some(&with_bit_16), None, DqTable::Unknown);
        assert_eq!(named_by_card.flags.len(), 17);
        assert_eq!(named_by_card.flags.last(), Some(&(65536, "RESOLVED".to_string())));
    }

    #[test]
    fn a_legacy_cutout_mask_without_mbit_cards_uses_the_built_in_names() {
        for card in ["IMAGETYP", "IMTYPE"] {
            let set = FlagSet::from_headers(Some(&header_with(&[(card, "MASKBITS")])), None, DqTable::Unknown);
            assert_eq!(set.flags, legacy_flags(), "{card}");
            assert_eq!(set.source, "Legacy DR10 maskbits (built-in table)");
            assert_eq!(set.exclusion_default, 49404);
            assert_eq!(set.decode(1025), vec!["NPRIMARY", "BAILOUT"]);
        }
        let r = FlagSet::from_headers(
            Some(&header_with(&[("IMAGETYP", "maskbits")])),
            Some(&header_with(&[("BAND", "R")])),
            DqTable::Unknown,
        );
        assert_eq!(r.exclusion_default, 72);
        let brick = {
            let mut header = legacy_mask_header();
            header.set("IMTYPE", "maskbits".to_string());
            header
        };
        assert_eq!(FlagSet::from_headers(Some(&brick), None, DqTable::Unknown).source, "MBIT_ cards (Legacy maskbits)");
    }

    #[test]
    fn the_band_label_of_a_cube_plane_selects_the_flags_of_that_band() {
        let mask = header_with(&[("IMAGETYP", "MASKBITS")]);
        for (image, exclusion, saturated) in [
            (header_with(&[("ABPLBAND", "r")]), 72, 8),
            (header_with(&[("ABPLBAND", "i")]), 49152, 16384),
            (header_with(&[("ABPLBAND", "Y")]), 49404, 16412),
            (header_with(&[("FILTERX", "g"), ("ABPLBAND", "r")]), 36, 4),
        ] {
            let set = FlagSet::from_headers(Some(&mask), Some(&image), DqTable::Unknown);
            assert_eq!(set.exclusion_default, exclusion, "{image:?}");
            assert_eq!(set.saturated_mask, saturated, "{image:?}");
        }
    }

    #[test]
    fn ps1_mask_names_come_from_the_msknam_cards_and_mask_value_is_the_default_exclusion() {
        let pairs: [(&str, u32); 18] = [
            ("DETECTOR", 1),
            ("FLAT", 2),
            ("DARK", 4),
            ("BLANK", 8),
            ("CTE", 16),
            ("SAT", 32),
            ("LOW", 64),
            ("SUSPECT", 128),
            ("CR", 256),
            ("SPIKE", 512),
            ("GHOST", 1024),
            ("STREAK", 2048),
            ("STARCORE", 4096),
            ("CONV.BAD", 8192),
            ("CONV.POOR", 16384),
            ("BURNTOOL", 128),
            ("MASK.VALUE", 8575),
            ("MARK.VALUE", 32768),
        ];
        let mut mask = HduHeader::empty();
        for (nn, (name, value)) in pairs.iter().enumerate() {
            mask.set(&format!("MSKNAM{nn:02}"), name.to_string());
            mask.set(&format!("MSKVAL{nn:02}"), value.to_string());
        }
        let set = FlagSet::from_headers(Some(&mask), None, DqTable::Unknown);
        assert_eq!(set.flags.len(), 16);
        assert!(set.flags.contains(&(128, "SUSPECT/BURNTOOL".to_string())), "{:?}", set.flags);
        assert!(set.flags.contains(&(32768, "MARK".to_string())), "{:?}", set.flags);
        assert!(set.flags.iter().all(|(_, name)| name != "MASK.VALUE"));
        assert!(set.flags.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(set.exclusion_default, 8575);
        assert_eq!(set.overlay_mask, 8575);
        assert_eq!(set.saturated_mask, 32);
        assert_eq!(set.source, "MSKNAM cards (PS1 mask)");
    }

    #[test]
    fn a_des_coadd_mask_excludes_every_non_zero_value() {
        let set = FlagSet::from_headers(Some(&header_with(&[("DES_EXT", "MASK")])), None, DqTable::Unknown);
        assert!(set.flags.is_empty());
        assert_eq!(set.exclusion_default, u32::MAX);
        assert_eq!(set.exclusion_mask, u32::MAX);
        assert_eq!(set.overlay_mask, u32::MAX);
        assert_eq!(set.saturated_mask, 0);
        assert_eq!(set.source, "DES coadd mask (non-zero = no good data)");
    }

    #[test]
    fn a_mask_without_names_lists_raw_bits_and_excludes_nothing() {
        for mask in [Some(HduHeader::empty()), Some(header_with(&[("DES_EXT", "IMAGE")])), None] {
            let set = FlagSet::from_headers(mask.as_ref(), None, DqTable::Unknown);
            assert!(set.flags.is_empty());
            assert_eq!(
                (set.exclusion_mask, set.exclusion_default, set.overlay_mask, set.saturated_mask),
                (0, 0, 0, 0)
            );
            assert_eq!(set.source, "unknown (raw bits)");
            assert_eq!(set.decode(5), vec!["BIT0".to_string(), "BIT2".to_string()]);
            assert!(!set.is_named());
            assert_eq!(set, FlagSet::from_table(DqTable::Unknown));
        }
    }

    #[test]
    fn the_user_exclusion_replaces_the_default_but_never_takes_detected() {
        let set = FlagSet::from_headers(Some(&afw_mask_header()), None, DqTable::Unknown);
        let chosen = set.clone().with_exclusion(Some(2 | 32));
        assert_eq!((chosen.exclusion_mask, chosen.exclusion_default), (2, 267));
        let negative = set.clone().with_exclusion(Some(1 | 64));
        assert_eq!(negative.exclusion_mask, 1);
        let nothing = set.clone().with_exclusion(Some(0));
        assert_eq!((nothing.exclusion_mask, nothing.exclusion_default), (0, 267));
        let restored = chosen.with_exclusion(None);
        assert_eq!((restored.exclusion_mask, restored.exclusion_default), (267, 267));
        assert_eq!(restored, set);
    }

    #[test]
    fn flag_names_resolve_to_masks_and_unknown_names_say_which_set() {
        let set = FlagSet::from_headers(Some(&afw_mask_header()), None, DqTable::Unknown);
        assert_eq!(set.mask_from_names(&["sat", "BIT9"]), Ok(2 | 512));
        assert_eq!(set.mask_from_names(&[]), Ok(0));
        assert_eq!(
            set.mask_from_names(&["SATURATED"]),
            Err("unknown flag name 'SATURATED' for MP_ cards (afw mask planes)".to_string())
        );
        assert_eq!(
            FlagSet::from_table(DqTable::Unknown).mask_from_names(&["BAD"]),
            Err("unknown flag name 'BAD' for unknown (raw bits)".to_string())
        );
        let serialised = serde_json::to_value(set.with_exclusion(Some(2))).unwrap();
        assert_eq!(serialised["flags"][1], serde_json::json!([2, "SAT"]));
        assert_eq!(serialised["table"], "unknown");
        assert_eq!(serialised["exclusion_mask"], 2);
        assert_eq!(serialised["exclusion_default"], 267);
        assert_eq!(serialised["saturated_mask"], 2);
    }

    #[test]
    fn roman_uses_its_own_table() {
        assert_eq!(DqTable::Roman.flags(), ROMAN_FLAGS);
        assert_eq!(DqTable::Jwst.flags(), JWST_FLAGS);
        assert_eq!(DqTable::Hst.flags(), HST_FLAGS);
        assert_eq!(DqTable::Roman.format(128), "128: OUTLIER");
        assert_eq!(DqTable::Roman.format(16), "16: GW_AFFECTED_DATA");
        assert_eq!(DqTable::Roman.format(1 << 28), "268435456: UNRELIABLE_RESET");
        assert_eq!(DqTable::Roman.decode(1 << 14), vec!["BIT14".to_string()]);
        assert_eq!(DqTable::Roman.decode(1 << 26), vec!["BIT26".to_string()]);
        assert_eq!(DqTable::Roman.mask_from_names(&["OUTLIER"]).unwrap(), 128);
        assert_eq!(DqTable::Roman.mask_from_names(&["SATURATED"]).unwrap(), 2);
        assert!(DqTable::Roman.mask_from_names(&["CHARGELOSS"]).is_err());
        for f in ROMAN_FLAGS {
            assert_eq!(f.bit.count_ones(), 1, "{}", f.name);
            assert_eq!(DqTable::Roman.decode(f.bit), vec![f.name.to_string()]);
        }
        assert_eq!(DqTable::Roman.label(), "roman");
        assert_eq!(DqTable::Jwst.label(), "jwst");
        assert_eq!(DqTable::Hst.label(), "hst");
        assert_eq!(DqTable::Jwst.id(), 0);
        assert_eq!(DqTable::Roman.id(), 1);
        assert_eq!(DqTable::Hst.id(), 2);
        assert_eq!(DqTable::Unknown.id(), 0xFFFF_FFFF);
        assert_eq!(DqTable::Jwst.default_overlay_mask(), 7);
        assert_eq!(DqTable::Hst.default_overlay_mask(), 4 | 256 | 4096 | 8192);
        assert_eq!(DqTable::Roman.exclusion_mask(), 1);
        assert_eq!(DqTable::Hst.exclusion_mask(), 0x3FFF);
        assert_eq!(serde_json::to_value(DqTable::Roman).unwrap(), serde_json::json!("roman"));
    }

    #[test]
    fn unknown_table_names_raw_bits_and_excludes_nothing() {
        assert!(DqTable::Unknown.flags().is_empty());
        assert_eq!(DqTable::Unknown.decode(5), vec!["BIT0".to_string(), "BIT2".to_string()]);
        assert_eq!(DqTable::Unknown.format(1), "1: BIT0");
        assert_eq!(DqTable::Unknown.format(0), "0: GOOD");
        assert_eq!(DqTable::Unknown.decode(u32::MAX).len(), 32);
        assert_eq!(DqTable::Unknown.exclusion_mask(), 0);
        assert_eq!(DqTable::Unknown.default_overlay_mask(), 0);
        assert_eq!(DqTable::Unknown.label(), "unknown (raw bits)");
        assert_eq!(DqTable::Unknown.mask_from_names(&["BIT3"]), Ok(8));
        assert_eq!(DqTable::Unknown.mask_from_names(&["BIT0", "bit31"]), Ok(1 | (1 << 31)));
        assert_eq!(DqTable::Jwst.mask_from_names(&["BIT9"]), Ok(512));
        assert_eq!(DqTable::Hst.mask_from_names(&["BIT20", "MASKED"]), Ok((1 << 20) | 8));
        assert!(DqTable::Unknown.mask_from_names(&["DO_NOT_USE"]).is_err());
        for bad in ["BIT32", "BIT", "BIT-1", "BIT 3", "BITX", "BIT1a"] {
            assert!(DqTable::Unknown.mask_from_names(&[bad]).is_err(), "{bad}");
        }
        let unnamed = DqTable::Unknown.mask_from_names(&["SATURATED"]).unwrap_err();
        assert!(unnamed.contains("unknown (raw bits)"), "{unnamed}");
        assert_eq!(DqTable::Unknown.id(), 0xFFFF_FFFF);
        let bits = Array2::from_shape_vec((1, 3), vec![0u32, 1, 4]).unwrap();
        assert!(exclusion_map(&bits, DqTable::Unknown.exclusion_mask()).iter().all(|&v| v == 0));
    }

    #[test]
    fn bits_present_ors_every_pixel_in_any_memory_order() {
        let mut bits = Array2::<u32>::zeros((300, 400));
        bits[[0, 0]] = 1;
        bits[[150, 7]] = 4;
        bits[[299, 399]] = 1 << 31;
        assert_eq!(bits_present(&bits), 1 | 4 | (1 << 31));
        let fortran = bits.clone().reversed_axes();
        assert!(fortran.as_slice().is_none());
        assert_eq!(bits_present(&fortran), 1 | 4 | (1 << 31));
        assert_eq!(bits_present(&Array2::<u32>::zeros((3, 3))), 0);
        assert_eq!(bits_present(&Array2::<u32>::zeros((0, 0))), 0);
    }

    #[test]
    fn exclusion_map_and_apply_exclusion() {
        let bits = Array2::from_shape_vec((2, 3), vec![0u32, 1, 2, 3, 4, 8]).unwrap();
        let ex = exclusion_map(&bits, 1);
        assert_eq!(ex.as_slice().unwrap(), &[0, 1, 0, 1, 0, 0]);
        let ex7 = exclusion_map(&bits, 7);
        assert_eq!(ex7.as_slice().unwrap(), &[0, 1, 1, 1, 1, 0]);

        let data = Array2::from_shape_vec((2, 3), vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let masked = apply_exclusion(&data, &ex).unwrap();
        assert!(masked[[0, 1]].is_nan());
        assert!(masked[[1, 0]].is_nan());
        assert_eq!(masked[[0, 0]], 1.0);
        assert_eq!(masked[[1, 2]], 6.0);
        assert_eq!(masked.iter().filter(|v| v.is_nan()).count(), 2);

        let wrong = Array2::<u8>::zeros((3, 2));
        assert!(apply_exclusion(&data, &wrong).is_err());
    }

    #[test]
    fn mask_preview_or_never_averages_away_a_flag() {
        let mut bits = Array2::<u32>::zeros((4, 4));
        bits[[0, 0]] = 1;
        let (cells, w, h) = mask_preview_or(&bits, 1, 2);
        assert_eq!((w, h), (2, 2));
        assert_eq!(cells, vec![1, 0, 0, 0]);

        let mut big = Array2::<u32>::zeros((100, 100));
        big[[99, 99]] = 2;
        let (cells, w, h) = mask_preview_or(&big, 2, 10);
        assert_eq!((w, h), (10, 10));
        assert_eq!(cells.iter().filter(|&&c| c == 1).count(), 1);
        assert_eq!(cells[99], 1);
        let (cells, _, _) = mask_preview_or(&big, 1, 10);
        assert!(cells.iter().all(|&c| c == 0));
    }

    #[test]
    fn mask_preview_or_dims_match_preview_dims() {
        let bits = Array2::<u32>::zeros((30, 50));
        let (cells, w, h) = mask_preview_or(&bits, 1, 100);
        assert_eq!((w, h), (50, 30));
        assert_eq!(cells.len(), 1500);

        let bits = Array2::<u32>::from_elem((3000, 1000), 1u32);
        let (cells, w, h) = mask_preview_or(&bits, 1, 2048);
        assert_eq!((h, w), preview_dims(3000, 1000, 2048));
        assert_eq!(cells.len(), w * h);
        assert!(cells.iter().all(|&c| c == 1));

        let raw = Array2::<f32>::zeros((3000, 1000));
        let data = crate::infra::ipc::encode_with_header_downsampled(&raw, 2048).unwrap();
        let rw = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let rh = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        assert_eq!((rw, rh), (w, h));

        let (cells, w, h) = mask_preview_or(&Array2::<u32>::zeros((0, 0)), 1, 8);
        assert!(cells.is_empty());
        assert_eq!((w, h), (0, 0));
    }
}
