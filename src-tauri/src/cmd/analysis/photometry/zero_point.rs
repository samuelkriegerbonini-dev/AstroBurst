use crate::core::metadata::photcal::{MagSystem, PhotCal};
use crate::types::header::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserMagSystem {
    Ab,
    Vega,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct UserZeroPoint {
    pub zp: f64,
    pub system: UserMagSystem,
    pub per_second: bool,
}

pub(crate) use crate::core::metadata::photcal::USER_ZERO_POINT_WARNING;

pub(in crate::cmd::analysis) fn check_user_zero_point(zero_point: Option<&UserZeroPoint>) -> anyhow::Result<()> {
    if let Some(UserZeroPoint { zp, .. }) = zero_point {
        if !zp.is_finite() {
            anyhow::bail!("user zero point {zp} must be a finite magnitude");
        }
    }
    Ok(())
}

pub(crate) fn user_photcal(header: Option<&HduHeader>, zero_point: &UserZeroPoint) -> anyhow::Result<(PhotCal, Vec<String>)> {
    let bunit = header.and_then(|h| crate::core::metadata::photcal::card_text(h, "BUNIT"));
    let rate = bunit.as_deref().is_some_and(|unit| {
        crate::core::metadata::photcal::is_rate_unit(&crate::core::metadata::photcal::normalized_unit(unit))
    });
    let exptime = header.and_then(|h| h.get_f64("EXPTIME")).filter(|t| t.is_finite() && *t > 0.0);
    if zero_point.per_second && !rate && exptime.is_none() {
        anyhow::bail!(
            "a zero point per second needs EXPTIME to turn the pixel values into a rate, and this header has none (BUNIT '{}')",
            bunit.as_deref().unwrap_or("missing")
        );
    }
    let system = match zero_point.system {
        UserMagSystem::Ab => MagSystem::Ab,
        UserMagSystem::Vega => MagSystem::Vega,
    };
    Ok((PhotCal::user_zero_point_for(header, zero_point.zp, system, zero_point.per_second), Vec::new()))
}
