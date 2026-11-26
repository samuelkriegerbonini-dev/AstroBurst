use crate::core::astrometry::wcs::SIP_RESIDUAL_KEYS;

const EXACT_CARDS: [&str; 43] = [
    "CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2", "CDELT1", "CDELT2", "CTYPE1", "CTYPE2", "CUNIT1", "CUNIT2", "CROTA1", "CROTA2",
    "CD1_1", "CD1_2", "CD2_1", "CD2_2", "PC1_1", "PC1_2", "PC2_1", "PC2_2",
    "PC001001", "PC001002", "PC002001", "PC002002", "CD001001", "CD001002", "CD002001", "CD002002",
    "LONPOLE", "LATPOLE", "RADESYS", "RADECSYS", "EQUINOX", "EPOCH", "WCSAXES", "WCSNAME",
    "A_ORDER", "B_ORDER", "AP_ORDER", "BP_ORDER", "A_DMAX", "B_DMAX", "WCSDIM",
];
const LOOKUP_DISTORTION_CARDS: [&str; 20] = [
    "CPDIS1", "CPDIS2", "CQDIS1", "CQDIS2", "DP1", "DP2", "DQ1", "DQ2", "CPERR1", "CPERR2", "CQERR1", "CQERR2",
    "D2IMDIS1", "D2IMDIS2", "D2IM1", "D2IM2", "D2IMERR1", "D2IMERR2", "D2IMEXT", "AXISCORR",
];
const SIP_PREFIXES: [&str; 4] = ["AP_", "BP_", "A_", "B_"];
const PV_PREFIXES: [&str; 2] = ["PV1_", "PV2_"];
const IRAF_PREFIXES: [&str; 3] = ["WAT0_", "WAT1_", "WAT2_"];
const AXIS_INDEXED_PREFIXES: [&str; 6] = ["CTYPE", "CRVAL", "CRPIX", "CDELT", "CUNIT", "CROTA"];
const AXIS_MATRIX_PREFIXES: [&str; 2] = ["PC", "CD"];
const AXIS_PARAMETER_PREFIXES: [&str; 2] = ["PV", "PS"];
const FIRST_AXIS_BEYOND_THE_CELESTIAL_PAIR: usize = 3;

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn is_sip_term(key: &str) -> bool {
    SIP_PREFIXES.iter().any(|prefix| {
        key.strip_prefix(prefix)
            .and_then(|powers| powers.split_once('_'))
            .is_some_and(|(p, q)| p.len() == 1 && q.len() == 1 && is_digits(p) && is_digits(q))
    })
}

fn has_numbered_suffix(key: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| key.strip_prefix(prefix).is_some_and(is_digits))
}

fn axis_number(digits: &str) -> Option<usize> {
    is_digits(digits).then(|| digits.parse().ok()).flatten()
}

fn axis_pair(rest: &str) -> Option<(usize, usize)> {
    let (i, j) = rest.split_once('_')?;
    Some((axis_number(i)?, axis_number(j)?))
}

pub fn axis_beyond_the_celestial_pair(key: &str) -> Option<usize> {
    let beyond = |axis: usize| axis >= FIRST_AXIS_BEYOND_THE_CELESTIAL_PAIR;
    if let Some(axis) = AXIS_INDEXED_PREFIXES.iter().find_map(|prefix| key.strip_prefix(prefix)).map(axis_number) {
        return axis.filter(|&axis| beyond(axis));
    }
    if let Some((i, j)) = AXIS_MATRIX_PREFIXES.iter().find_map(|prefix| key.strip_prefix(prefix)).and_then(axis_pair) {
        return (beyond(i) && beyond(j)).then_some(i.max(j));
    }
    AXIS_PARAMETER_PREFIXES
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .and_then(axis_pair)
        .and_then(|(axis, _)| beyond(axis).then_some(axis))
}

pub fn is_lookup_distortion_card(key: &str) -> bool {
    LOOKUP_DISTORTION_CARDS.contains(&key)
}

pub fn is_celestial_wcs_card(key: &str) -> bool {
    EXACT_CARDS.contains(&key)
        || is_lookup_distortion_card(key)
        || SIP_RESIDUAL_KEYS.contains(&key)
        || is_sip_term(key)
        || has_numbered_suffix(key, &PV_PREFIXES)
        || has_numbered_suffix(key, &IRAF_PREFIXES)
}

#[cfg(test)]
mod tests {
    use super::{axis_beyond_the_celestial_pair, is_celestial_wcs_card};

    #[test]
    fn only_the_cards_of_the_two_celestial_axes_count_as_the_celestial_wcs() {
        for key in [
            "CRVAL1", "CTYPE2", "CD2_1", "PC1_2", "PC001001", "CD002002", "CROTA2", "LONPOLE", "RADESYS", "RADECSYS", "EQUINOX",
            "A_ORDER", "A_0_2", "BP_3_1", "A_DMAX", "PV1_0", "PV2_10", "SIPMXERR", "CPDIS1", "D2IMDIS2", "DP1", "WAT1_001", "WCSAXES",
        ] {
            assert!(is_celestial_wcs_card(key), "{key} belongs to the celestial WCS");
        }
        for key in [
            "CRVAL3", "CTYPE3", "CDELT3", "CUNIT3", "CRPIX3", "CD3_3", "PC3_3", "B_V", "A_V", "AP_RAD", "B_BAND", "CRVAL1A", "PV3_1",
            "PV1_", "A_10_2", "EPOCHS", "WATER", "OBJECT", "DATE-OBS", "BUNIT",
        ] {
            assert!(!is_celestial_wcs_card(key), "{key} is not a card of the celestial WCS");
        }
    }

    #[test]
    fn the_cards_of_a_third_axis_are_told_apart_from_those_that_couple_it_to_the_sky() {
        for (key, axis) in [("CTYPE3", 3), ("CRVAL3", 3), ("CDELT4", 4), ("CUNIT3", 3), ("CRPIX3", 3), ("PC3_3", 3), ("CD4_3", 4), ("PV3_1", 3), ("PS3_0", 3)] {
            assert_eq!(axis_beyond_the_celestial_pair(key), Some(axis), "{key}");
        }
        for key in ["CTYPE1", "CRVAL2", "PC1_3", "CD3_1", "PV1_3", "NAXIS3", "CDELT", "CTYPE3A", "B_V", "CD001001"] {
            assert_eq!(axis_beyond_the_celestial_pair(key), None, "{key}");
        }
    }
}
