mod pattern;
pub(crate) mod extract;

pub use extract::{
    extract_cfa, extract_green, extract_ha, extract_ha_oiii, extraction_pattern, split_cfa,
    CfaExtract, CfaExtracted, CfaPlane,
};
pub use pattern::*;
