use serde::{Deserialize, Serialize};

pub const SOLVER_LOCAL: &str = "local";
pub const SOLVER_ASTROMETRY_NET: &str = "astrometry.net";
pub const PARITY_NORMAL: &str = "normal";
pub const PARITY_FLIPPED: &str = "flipped";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SolveResult {
    #[serde(rename = "center_ra")]
    pub ra_center: f64,
    #[serde(rename = "center_dec")]
    pub dec_center: f64,
    #[serde(default)]
    pub orientation: Option<f64>,
    #[serde(rename = "pixel_scale_arcsec")]
    pub pixel_scale: f64,
    #[serde(rename = "field_of_view_w_arcmin")]
    pub field_w_arcmin: f64,
    #[serde(rename = "field_of_view_h_arcmin")]
    pub field_h_arcmin: f64,
    #[serde(default)]
    pub annotations: Vec<FieldAnnotation>,
    #[serde(default)]
    pub wcs_cards: Vec<(String, String)>,
    #[serde(default)]
    pub solver: String,
    #[serde(default)]
    pub tier: Option<u8>,
    #[serde(default)]
    pub catalog: Option<String>,
    #[serde(default)]
    pub matched: Option<usize>,
    #[serde(default)]
    pub predicted: Option<usize>,
    #[serde(default)]
    pub rms_px: Option<f64>,
    #[serde(default)]
    pub sip_order: Option<u8>,
    #[serde(default)]
    pub sip_max_err_px: Option<f64>,
    #[serde(default)]
    pub hint_source: Option<String>,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub parity: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldAnnotation {
    #[serde(rename = "type")]
    pub kind: String,
    pub names: Vec<String>,
    pub pixelx: f64,
    pub pixely: f64,
    pub radius: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveConfig {
    pub api_url: String,
    pub api_key: String,
    pub ra_hint: Option<f64>,
    pub dec_hint: Option<f64>,
    pub radius_hint: Option<f64>,
    pub scale_low: Option<f64>,
    pub scale_high: Option<f64>,
    pub scale_units: Option<String>,
    pub timeout_secs: u64,
}

pub fn parity_name(flipped: bool) -> &'static str {
    if flipped {
        PARITY_FLIPPED
    } else {
        PARITY_NORMAL
    }
}

pub fn orientation_east_of_north_deg(cd: [[f64; 2]; 2]) -> Option<f64> {
    let [[cd11, cd12], [cd21, cd22]] = cd;
    let parity = if cd11 * cd22 - cd12 * cd21 >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let t = parity * cd11 + cd22;
    let a = parity * cd21 - cd12;
    let orientation = -a.atan2(t).to_degrees();
    orientation.is_finite().then_some(orientation)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCSEC: f64 = 1.0 / 3600.0;

    fn rotated(rotation_deg: f64, flipped: bool) -> [[f64; 2]; 2] {
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        if flipped {
            [[ARCSEC * cos, ARCSEC * sin], [-ARCSEC * sin, ARCSEC * cos]]
        } else {
            [[-ARCSEC * cos, ARCSEC * sin], [ARCSEC * sin, ARCSEC * cos]]
        }
    }

    #[test]
    fn a_nova_result_without_the_local_fields_still_parses_with_their_defaults() {
        let nova = r#"{"center_ra":83.822,"center_dec":-5.391,"pixel_scale_arcsec":2.0,
            "field_of_view_w_arcmin":34.1,"field_of_view_h_arcmin":22.7,"wcs_cards":[["CRVAL1","83.822"]]}"#;
        let result: SolveResult = serde_json::from_str(nova).unwrap();
        assert_eq!(
            (result.ra_center, result.dec_center, result.pixel_scale),
            (83.822, -5.391, 2.0)
        );
        assert_eq!(result.orientation, None);
        assert_eq!(result.solver, "");
        assert_eq!(
            (
                result.tier,
                result.matched,
                result.predicted,
                result.sip_order,
                result.parity.as_deref()
            ),
            (None, None, None, None, None)
        );
        assert!(result.annotations.is_empty() && result.warnings.is_empty());
        let back = serde_json::to_value(&result).unwrap();
        for key in [
            "solver",
            "tier",
            "catalog",
            "matched",
            "predicted",
            "rms_px",
            "sip_order",
            "sip_max_err_px",
            "hint_source",
            "elapsed_ms",
            "parity",
            "orientation",
            "warnings",
        ] {
            assert!(back.get(key).is_some(), "{key} missing from {back}");
        }
    }

    #[test]
    fn orientation_follows_the_astrometry_net_convention_for_both_parities() {
        let north_up = orientation_east_of_north_deg(rotated(0.0, false)).unwrap();
        assert!(north_up.abs() < 1e-9, "{north_up}");
        let turned = orientation_east_of_north_deg(rotated(33.0, false)).unwrap();
        assert!((turned - 33.0).abs() < 1e-9, "{turned}");
        let mirrored = orientation_east_of_north_deg(rotated(33.0, true)).unwrap();
        assert!((mirrored - 33.0).abs() < 1e-9, "{mirrored}");
        let mirrored_west = orientation_east_of_north_deg(rotated(-120.0, true)).unwrap();
        assert!((mirrored_west + 120.0).abs() < 1e-9, "{mirrored_west}");
        assert_eq!(
            orientation_east_of_north_deg([[f64::NAN, 0.0], [0.0, 1.0]]),
            None
        );
        assert_eq!(
            (parity_name(false), parity_name(true)),
            (PARITY_NORMAL, PARITY_FLIPPED)
        );
    }
}
