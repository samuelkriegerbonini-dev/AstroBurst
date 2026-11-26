use anyhow::{bail, Result};

use crate::types::header::{parse_fits_float, HduHeader};

const ARCMIN_PER_DEG: f64 = 60.0;
const ARCSEC_PER_DEG: f64 = 3600.0;
const MAS_PER_DEG: f64 = 3_600_000.0;
const ANGLE_TOLERANCE_DEG: f64 = 1e-9;
const DEFAULT_LATPOLE_DEG: f64 = 90.0;
const ZENITHAL_PROJECTIONS: [&str; 10] = ["AZP", "SZP", "TAN", "STG", "SIN", "ARC", "ZPN", "ZEA", "AIR", "NCP"];
const CONIC_PROJECTIONS: [&str; 4] = ["COP", "COD", "COE", "COO"];
pub const ORIGIN_FIDUCIAL_PROJECTIONS: [&str; 9] = ["CYP", "CEA", "CAR", "MER", "SFL", "PAR", "MOL", "AIT", "HPX"];
const LOOKUP_DISTORTION_CARDS: [&str; 6] = ["CPDIS1", "CPDIS2", "CQDIS1", "CQDIS2", "D2IMDIS1", "D2IMDIS2"];

fn card<'a>(header: &'a HduHeader, key: &str) -> Option<&'a str> {
    header
        .cards
        .iter()
        .find(|(name, _)| name.trim().eq_ignore_ascii_case(key))
        .map(|(_, value)| value.trim().trim_matches('\'').trim())
}

fn numeric_card(header: &HduHeader, key: &str) -> Option<f64> {
    card(header, key).and_then(parse_fits_float).filter(|value| value.is_finite())
}

fn degrees_per_unit(axis: usize, unit: &str) -> Result<f64> {
    match unit.to_ascii_lowercase().as_str() {
        "" | "deg" | "degree" | "degrees" => Ok(1.0),
        "arcmin" | "arcmins" | "arcminute" | "arcminutes" => Ok(1.0 / ARCMIN_PER_DEG),
        "arcsec" | "arcsecs" | "arcsecond" | "arcseconds" => Ok(1.0 / ARCSEC_PER_DEG),
        "mas" => Ok(1.0 / MAS_PER_DEG),
        "rad" | "radian" | "radians" => Ok(1.0_f64.to_degrees()),
        _ => bail!("CUNIT{axis} '{unit}' is not a unit this reader converts for a celestial axis (deg, arcmin, arcsec, mas or rad)"),
    }
}

pub fn celestial_unit_factors(header: &HduHeader) -> Result<[f64; 2]> {
    Ok([
        degrees_per_unit(1, card(header, "CUNIT1").unwrap_or(""))?,
        degrees_per_unit(2, card(header, "CUNIT2").unwrap_or(""))?,
    ])
}

fn angles_differ(a_deg: f64, b_deg: f64) -> bool {
    ((a_deg - b_deg + 180.0).rem_euclid(360.0) - 180.0).abs() > ANGLE_TOLERANCE_DEG
}

fn is_pv_card(key: &str) -> bool {
    key.strip_prefix("PV1_").or_else(|| key.strip_prefix("PV2_")).is_some_and(|m| m.parse::<u32>().is_ok())
}

pub fn tan_pole_cards_change_the_projection(header: &HduHeader) -> bool {
    header.cards.iter().any(|(name, value)| {
        let key = name.trim().to_ascii_uppercase();
        match key.as_str() {
            "LATPOLE" => false,
            "LONPOLE" => parse_fits_float(value.trim().trim_matches('\'').trim()).is_none_or(|lonpole| angles_differ(lonpole, 180.0)),
            _ => is_pv_card(&key),
        }
    })
}

pub fn refuse_unsupported_projection_cards(header: &HduHeader, projection: &str, crval2_deg: f64) -> Result<()> {
    let zenithal = ZENITHAL_PROJECTIONS.contains(&projection);
    let conic = CONIC_PROJECTIONS.contains(&projection);
    if projection == "TAN" || !(zenithal || conic || ORIGIN_FIDUCIAL_PROJECTIONS.contains(&projection)) {
        return Ok(());
    }
    let native_latitude = if zenithal {
        90.0
    } else if conic {
        numeric_card(header, "PV2_1").unwrap_or(0.0)
    } else {
        0.0
    };
    for (key, default) in [("PV1_0", 0.0), ("PV1_1", 0.0), ("PV1_2", native_latitude)] {
        let Some(value) = numeric_card(header, key) else { continue };
        if (value - default).abs() > ANGLE_TOLERANCE_DEG {
            bail!(
                "{key} = {value} sets a fiducial point other than the default of the {projection} projection ({default}); headers with an explicit fiducial point (PV1_0, PV1_1, PV1_2) are not supported"
            );
        }
    }
    let default_lonpole = if crval2_deg >= native_latitude { 0.0 } else { 180.0 };
    let lonpole = numeric_card(header, "LONPOLE");
    if !zenithal {
        if let Some(value) = lonpole.filter(|value| angles_differ(*value, default_lonpole)) {
            bail!(
                "LONPOLE = {value} is not the default ({default_lonpole}) for this {projection} header; a non-default LONPOLE on a non-zenithal projection is not supported"
            );
        }
    }
    if let Some(value) = numeric_card(header, "PV1_3").filter(|value| angles_differ(*value, lonpole.unwrap_or(default_lonpole))) {
        bail!("PV1_3 = {value} sets a native longitude of the celestial pole that the LONPOLE card or its default does not; this spelling is not supported on a {projection} projection");
    }
    if !zenithal {
        let latpole = numeric_card(header, "LATPOLE").unwrap_or(DEFAULT_LATPOLE_DEG);
        if let Some(value) = numeric_card(header, "PV1_4").filter(|value| angles_differ(*value, latpole)) {
            bail!("PV1_4 = {value} sets a native latitude of the celestial pole that the LATPOLE card or its default does not; this spelling is not supported on a {projection} projection");
        }
    }
    Ok(())
}

pub fn lookup_distortion_warning(header: &HduHeader) -> Option<String> {
    let present: Vec<&str> = LOOKUP_DISTORTION_CARDS.iter().copied().filter(|key| card(header, key).is_some()).collect();
    (!present.is_empty()).then(|| format!("lookup-table distortion not applied: {}", present.join(", ")))
}
