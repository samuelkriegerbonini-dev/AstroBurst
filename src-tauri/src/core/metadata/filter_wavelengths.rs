use crate::types::header::HduHeader;

pub const FILTER_WAVELENGTHS_NM: &[(&str, u32)] = &[
    ("HA", 656),
    ("HALPHA", 656),
    ("H_ALPHA", 656),
    ("OIII", 501),
    ("O3", 501),
    ("SII", 673),
    ("S2", 673),
    ("NII", 658),
    ("HB", 486),
    ("HBETA", 486),
    ("F656N", 656),
    ("F657N", 657),
    ("F658N", 658),
    ("F673N", 673),
    ("F501N", 501),
    ("F502N", 501),
    ("F503N", 503),
    ("F487N", 487),
    ("F469N", 469),
    ("F631N", 631),
    ("F070W", 700),
    ("F090W", 900),
    ("F115W", 1150),
    ("F140M", 1400),
    ("F150W", 1500),
    ("F150W2", 1500),
    ("F162M", 1620),
    ("F164N", 1640),
    ("F182M", 1820),
    ("F187N", 1870),
    ("F200W", 2000),
    ("F210M", 2100),
    ("F212N", 2120),
    ("F250M", 2500),
    ("F277W", 2770),
    ("F300M", 3000),
    ("F322W2", 3220),
    ("F323N", 3230),
    ("F335M", 3350),
    ("F356W", 3560),
    ("F360M", 3600),
    ("F405N", 4050),
    ("F410M", 4100),
    ("F430M", 4300),
    ("F444W", 4440),
    ("F460M", 4600),
    ("F466N", 4660),
    ("F470N", 4700),
    ("F480M", 4800),
    ("F560W", 5600),
    ("F770W", 7700),
    ("F1000W", 10000),
    ("F1130W", 11300),
    ("F1280W", 12800),
    ("F1500W", 15000),
    ("F1800W", 18000),
    ("F2100W", 21000),
    ("F2550W", 25500),
    ("F158M", 1580),
    ("F380M", 3800),
    ("F336W", 336),
    ("F390W", 390),
    ("F435W", 435),
    ("F438W", 438),
    ("F439W", 439),
    ("F450W", 450),
    ("F475W", 475),
    ("F555W", 555),
    ("F606W", 606),
    ("F625W", 625),
    ("F675W", 675),
    ("F702W", 702),
    ("F775W", 775),
    ("F814W", 814),
];

const FILTER_HEADER_KEYS: [&str; 6] = ["FILTER", "FILTER1", "FILTER2", "FILTNAM1", "FILTNAM2", "PUPIL"];
const PUPIL_FIRST_KEYS: [&str; 6] = ["PUPIL", "FILTER", "FILTER1", "FILTER2", "FILTNAM1", "FILTNAM2"];
const PUPIL_KEY: &str = "PUPIL";
const CLEAR_TOKEN: &str = "CLEAR";

fn table_hit(code: &str) -> Option<(&'static str, u32)> {
    FILTER_WAVELENGTHS_NM.iter().copied().find(|(known, _)| *known == code)
}

pub fn filter_code_and_wavelength_nm(value: &str) -> Option<(&'static str, u32)> {
    let upper = value.to_uppercase();
    if let Some(hit) = table_hit(upper.trim()) {
        return Some(hit);
    }
    upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty() && *token != CLEAR_TOKEN)
        .find_map(table_hit)
}

fn is_wheel_position(value: &str) -> bool {
    (1..=2).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
}

fn header_filter_value(header: &HduHeader, key: &str) -> Option<String> {
    let value = header.get(key)?.trim().trim_matches('\'').trim();
    (!value.is_empty() && !is_wheel_position(value)).then(|| value.to_string())
}

pub fn effective_filter(header: &HduHeader) -> Option<String> {
    let pupil_resolves = header_filter_value(header, PUPIL_KEY).is_some_and(|v| filter_code_and_wavelength_nm(&v).is_some());
    let keys: &[&str] = if pupil_resolves { &PUPIL_FIRST_KEYS } else { &FILTER_HEADER_KEYS };
    let values: Vec<String> = keys.iter().filter_map(|key| header_filter_value(header, key)).collect();
    values
        .iter()
        .find(|v| filter_code_and_wavelength_nm(v).is_some())
        .or_else(|| values.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (key, value) in cards {
            h.set(key, value.to_string());
        }
        h
    }

    #[test]
    fn effective_filter_prefers_a_pupil_filter_code() {
        let pupil = header(&[("FILTER", "F444W"), ("PUPIL", "F470N")]);
        assert_eq!(effective_filter(&pupil).as_deref(), Some("F470N"));
        let clear_pupil = header(&[("FILTER", "F444W"), ("PUPIL", "CLEAR")]);
        assert_eq!(effective_filter(&clear_pupil).as_deref(), Some("F444W"));
        let niriss = header(&[("FILTER", "CLEAR"), ("PUPIL", "F158M")]);
        assert_eq!(effective_filter(&niriss).as_deref(), Some("F158M"));
        let wfpc2 = header(&[("FILTER1", "23"), ("FILTNAM1", "F502N")]);
        assert_eq!(effective_filter(&wfpc2).as_deref(), Some("F502N"));
        let acs = header(&[("FILTER1", "CLEAR1L"), ("FILTER2", "F814W")]);
        assert_eq!(effective_filter(&acs).as_deref(), Some("F814W"));
        let acs_unknown = header(&[("FILTER1", "CLEAR1L"), ("FILTER2", "F850LP")]);
        assert_eq!(effective_filter(&acs_unknown).as_deref(), Some("CLEAR1L"));
        let nrm = header(&[("PUPIL", "NRM")]);
        assert_eq!(effective_filter(&nrm).as_deref(), Some("NRM"));
        let quoted = header(&[("FILTER", "'F444W   '"), ("PUPIL", "'F470N   '")]);
        assert_eq!(effective_filter(&quoted).as_deref(), Some("F470N"));
        let unresolvable = header(&[("FILTER", "CLEAR"), ("PUPIL", "NRM")]);
        assert_eq!(effective_filter(&unresolvable).as_deref(), Some("CLEAR"));
        assert_eq!(effective_filter(&HduHeader::empty()), None);
    }

    #[test]
    fn tokenised_lookup_matches_the_frontend() {
        assert_eq!(filter_code_and_wavelength_nm("CLEAR/F090W"), Some(("F090W", 900)));
        assert_eq!(filter_code_and_wavelength_nm("F444W;CLEAR"), Some(("F444W", 4440)));
        assert_eq!(filter_code_and_wavelength_nm("CLEAR"), None);
        assert_eq!(filter_code_and_wavelength_nm("CLEAR/CLEAR"), None);
        assert_eq!(filter_code_and_wavelength_nm(" ha "), Some(("HA", 656)));
        assert_eq!(filter_code_and_wavelength_nm("f090w"), Some(("F090W", 900)));
        assert_eq!(filter_code_and_wavelength_nm("F158M"), Some(("F158M", 1580)));
        assert_eq!(filter_code_and_wavelength_nm("F380M"), Some(("F380M", 3800)));
        assert_eq!(filter_code_and_wavelength_nm("F814W"), Some(("F814W", 814)));
        assert_eq!(filter_code_and_wavelength_nm("XYZ123"), None);
        assert_eq!(filter_code_and_wavelength_nm(""), None);
    }

    #[test]
    fn the_table_has_74_unique_codes() {
        assert_eq!(FILTER_WAVELENGTHS_NM.len(), 74);
        let mut codes: Vec<&str> = FILTER_WAVELENGTHS_NM.iter().map(|(code, _)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 74);
        assert!(FILTER_WAVELENGTHS_NM.iter().all(|(code, nm)| *code == code.to_ascii_uppercase() && *nm > 0));
    }
}
