use anyhow::{bail, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::imaging::sampling::{cell_range, preview_dims};
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

const TELESCOPE_KEYS: &[&str] = &[
    "TELESCOP",
    "INSTRUME",
    "META_TELESCOPE",
    "META_INSTRUMENT_NAME",
    "ROMAN_META_TELESCOPE",
    "ROMAN_META_INSTRUMENT_NAME",
];

const JWST_MARKERS: &[&str] = &["JWST", "NIRCAM", "NIRISS", "NIRSPEC", "MIRI", "FGS"];
const ROMAN_MARKERS: &[&str] = &["ROMAN", "WFI"];
const HST_MARKERS: &[&str] = &["HST", "ACS", "WFC3", "WFPC2", "STIS", "COS"];

impl DqTable {
    pub fn select(header: &HduHeader) -> DqTable {
        for key in TELESCOPE_KEYS {
            let Some(value) = header.get(key) else { continue };
            let upper = value.to_uppercase();
            if JWST_MARKERS.iter().any(|m| upper.contains(m)) {
                return DqTable::Jwst;
            }
            if ROMAN_MARKERS.iter().any(|m| upper.contains(m)) {
                return DqTable::Roman;
            }
            if HST_MARKERS.iter().any(|m| upper.contains(m)) {
                return DqTable::Hst;
            }
        }
        DqTable::Unknown
    }

    pub fn flags(self) -> &'static [DqFlag] {
        match self {
            DqTable::Hst => HST_FLAGS,
            DqTable::Roman => ROMAN_FLAGS,
            DqTable::Jwst | DqTable::Unknown => JWST_FLAGS,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DqTable::Jwst => "jwst",
            DqTable::Roman => "roman",
            DqTable::Hst => "hst",
            DqTable::Unknown => "unknown (JWST-convention bits)",
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
            match self.flags().iter().find(|f| f.name.eq_ignore_ascii_case(wanted)) {
                Some(f) => mask |= f.bit,
                None => return Err(format!("unknown DQ flag name '{}' for table {}", wanted, self.label())),
            }
        }
        Ok(mask)
    }

    pub fn default_overlay_mask(self) -> u32 {
        match self {
            DqTable::Hst => 4 | 256 | 4096 | 8192,
            DqTable::Jwst | DqTable::Roman | DqTable::Unknown => 7,
        }
    }

    pub fn exclusion_mask(self) -> u32 {
        match self {
            DqTable::Hst => 0x3FFF,
            DqTable::Jwst | DqTable::Roman | DqTable::Unknown => 1,
        }
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
    fn roman_uses_its_own_table_and_unknown_uses_jwst_convention() {
        assert_eq!(DqTable::Roman.flags(), ROMAN_FLAGS);
        assert_eq!(DqTable::Unknown.flags(), JWST_FLAGS);
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
        assert!(DqTable::Unknown.label().contains("JWST-convention"));
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
