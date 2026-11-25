use crate::core::ops::OutputValues;
use crate::types::constants::HEADER_DISPLAY_REFERRED_CARD;
use crate::types::header::HduHeader;

pub fn is_display_referred(source: Option<&HduHeader>) -> bool {
    source
        .and_then(|h| h.get(HEADER_DISPLAY_REFERRED_CARD))
        .is_some_and(|v| v.trim().trim_matches('\'').trim() == "T")
}

pub fn output_values_for(source: Option<&HduHeader>, fallback: OutputValues) -> OutputValues {
    if is_display_referred(source) { OutputValues::DisplayReferred } else { fallback }
}
