use std::time::Instant;

use anyhow::bail;
use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::cmd::spectral::with_spectral_header;
use crate::core::astrometry::geometry::{
    frame_geometry, geometry_method_notes, header_time, image_centre_pixel, resolve_site, resolve_target, FrameGeometry,
    GeometryOverrides,
};
use crate::types::constants::{
    RES_AIRMASS_HEADER, RES_ELAPSED_MS, RES_GEOMETRY, RES_NOTES, RES_SITE, RES_TARGET, RES_TIME_SOURCE,
};

const TARGET_RA_MIN: f64 = 0.0;
const TARGET_RA_MAX: f64 = 360.0;
const TARGET_DEC_LIMIT: f64 = 90.0;
const SITE_LAT_LIMIT: f64 = 90.0;
const SITE_LON_MIN: f64 = -180.0;
const SITE_LON_MAX: f64 = 360.0;
const SITE_HEIGHT_MIN_M: f64 = -500.0;
const SITE_HEIGHT_MAX_M: f64 = 10000.0;
const AIRMASS_KEY: &str = "AIRMASS";
const NO_TIME_REASON: &str =
    "no observation time in the header (MJD-AVG, EXPMID, DATE-AVG, DATE-OBS, MJD-OBS or BJDREF with TSTART/TSTOP)";

pub(crate) fn validate_geometry_overrides(overrides: &GeometryOverrides) -> anyhow::Result<()> {
    if let Some(ra) = overrides.target_ra {
        if !(ra.is_finite() && ra >= TARGET_RA_MIN && ra < TARGET_RA_MAX) {
            bail!("target_ra {ra} must be at least {TARGET_RA_MIN} and below {TARGET_RA_MAX} degrees");
        }
    }
    if let Some(dec) = overrides.target_dec {
        if !(dec.is_finite() && dec.abs() <= TARGET_DEC_LIMIT) {
            bail!("target_dec {dec} must be between -{TARGET_DEC_LIMIT} and {TARGET_DEC_LIMIT} degrees");
        }
    }
    if overrides.target_ra.is_some() != overrides.target_dec.is_some() {
        bail!("target override needs both target_ra and target_dec");
    }
    if let Some(lat) = overrides.site_lat {
        if !(lat.is_finite() && lat.abs() <= SITE_LAT_LIMIT) {
            bail!("site_lat {lat} must be between -{SITE_LAT_LIMIT} and {SITE_LAT_LIMIT} degrees");
        }
    }
    if let Some(lon) = overrides.site_lon {
        if !(lon.is_finite() && (SITE_LON_MIN..=SITE_LON_MAX).contains(&lon)) {
            bail!("site_lon {lon} must be between {SITE_LON_MIN} and {SITE_LON_MAX} degrees (east positive)");
        }
    }
    if let Some(height) = overrides.site_height {
        if !(height.is_finite() && (SITE_HEIGHT_MIN_M..=SITE_HEIGHT_MAX_M).contains(&height)) {
            bail!("site_height {height} must be between {SITE_HEIGHT_MIN_M} and {SITE_HEIGHT_MAX_M} m");
        }
    }
    let site_pair_given = overrides.site_lat.is_some() && overrides.site_lon.is_some();
    let any_site_given = overrides.site_lat.is_some() || overrides.site_lon.is_some() || overrides.site_height.is_some();
    if any_site_given && !site_pair_given {
        bail!("site override needs both site_lat and site_lon");
    }
    Ok(())
}

pub(crate) fn observation_geometry_json(path: &str, overrides: &GeometryOverrides) -> anyhow::Result<serde_json::Value> {
    let t0 = Instant::now();
    with_spectral_header(path, |header| {
        let target = resolve_target(header, image_centre_pixel(header), overrides);
        let site = resolve_site(header, overrides);
        let time = header_time(header);
        let geometry = match &time {
            Some(time) => frame_geometry(header, time, target.as_ref(), site.as_ref()),
            None => FrameGeometry::without_time(NO_TIME_REASON),
        };
        let notes = geometry_method_notes(header, target.as_ref(), site.as_ref());
        Ok(json!({
            RES_GEOMETRY: geometry,
            RES_TARGET: target,
            RES_SITE: site,
            RES_TIME_SOURCE: time.as_ref().map(|t| t.source.clone()),
            RES_AIRMASS_HEADER: header.get_f64(AIRMASS_KEY).filter(|v| v.is_finite()),
            RES_NOTES: notes,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn observation_geometry_cmd(
    path: String,
    target_ra: Option<f64>,
    target_dec: Option<f64>,
    site_lat: Option<f64>,
    site_lon: Option<f64>,
    site_height: Option<f64>,
) -> Result<serde_json::Value, String> {
    let overrides = GeometryOverrides { target_ra, target_dec, site_lat, site_lon, site_height };
    validate_geometry_overrides(&overrides).map_err(|e| format!("{e:#}"))?;
    blocking_cmd!(observation_geometry_json(&path, &overrides))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::{make_header, north_up_cd, wcs_cards};
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::header::HduHeader;

    fn sited_header(extra: &[(&str, &str)]) -> HduHeader {
        let cards = wcs_cards(north_up_cd());
        let mut pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        pairs.extend_from_slice(&[("SITELAT", "19.82"), ("SITELONG", "-155.47"), ("DATE-OBS", "2026-03-21T10:00:00"), ("EXPTIME", "600")]);
        pairs.extend_from_slice(extra);
        make_header(&pairs)
    }

    fn write_image(dir: &std::path::Path, name: &str, header: &HduHeader) -> String {
        let path = dir.join(name);
        let data = ndarray::Array2::<f32>::from_elem((100, 100), 1.0);
        write_fits_mono(path.to_str().unwrap(), &data, Some(header)).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn observation_geometry_json_has_the_documented_keys_for_a_sited_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_image(dir.path(), "sited.fits", &sited_header(&[]));
        let j = observation_geometry_json(&path, &GeometryOverrides::default()).unwrap();
        let geometry = &j[RES_GEOMETRY];
        assert!(geometry["bjd_tdb"].as_f64().unwrap().is_finite(), "{j}");
        assert!(geometry["altitude_deg"].as_f64().unwrap().is_finite(), "{j}");
        assert!(geometry["airmass_computed"].as_f64().unwrap().is_finite(), "{j}");
        assert_eq!(geometry["bjd_source"], "computed");
        assert_eq!(geometry["airmass_formula"], "Kasten & Young 1989");
        assert!(j[RES_TARGET]["source"].as_str().unwrap().starts_with("WCS at image centre"), "{j}");
        assert!((j[RES_TARGET]["ra_deg"].as_f64().unwrap() - 150.0).abs() < 1e-6);
        assert_eq!(j[RES_SITE]["source"], "SITELAT/SITELONG");
        assert_eq!(j[RES_SITE]["lat_deg"], 19.82);
        assert_eq!(j[RES_SITE]["lon_deg"], -155.47);
        assert_eq!(j[RES_SITE]["height_m"], 0.0);
        assert!(j[RES_TIME_SOURCE].as_str().unwrap().contains("DATE-OBS"), "{j}");
        assert!(!j[RES_NOTES].as_array().unwrap().is_empty());
        assert!(j[RES_AIRMASS_HEADER].is_null());
        assert!(j[RES_ELAPSED_MS].is_u64());
        let with_airmass = write_image(dir.path(), "airmass.fits", &sited_header(&[("AIRMASS", "1.234")]));
        let j = observation_geometry_json(&with_airmass, &GeometryOverrides::default()).unwrap();
        assert_eq!(j[RES_AIRMASS_HEADER], 1.234);
    }

    #[test]
    fn observation_geometry_json_without_time_returns_nulls_and_a_note_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let header = make_header(&[("NAXIS1", "100"), ("NAXIS2", "100"), ("OBJECT", "M31")]);
        let path = write_image(dir.path(), "untimed.fits", &header);
        let j = observation_geometry_json(&path, &GeometryOverrides::default()).unwrap();
        let geometry = &j[RES_GEOMETRY];
        assert!(geometry["jd_utc"].is_null() && geometry["bjd_tdb"].is_null() && geometry["altitude_deg"].is_null(), "{j}");
        assert!(geometry["moon_illumination"].is_null());
        let notes = geometry["time_scale_notes"].as_array().unwrap();
        assert!(notes.iter().any(|n| n.as_str().unwrap().contains("no observation time")), "{j}");
        assert!(j[RES_TARGET].is_null() && j[RES_SITE].is_null() && j[RES_TIME_SOURCE].is_null(), "{j}");
        let overrides = GeometryOverrides { site_lat: Some(19.82), site_lon: Some(-155.47), site_height: Some(4200.0), ..GeometryOverrides::default() };
        let j = observation_geometry_json(&path, &overrides).unwrap();
        assert_eq!(j[RES_SITE]["source"], "user");
        assert_eq!(j[RES_SITE]["height_m"], 4200.0);
    }

    #[test]
    fn observation_geometry_json_does_not_apply_a_site_override_to_a_spacecraft_header() {
        let dir = tempfile::tempdir().unwrap();
        let header = make_header(&[("TELESCOP", "JWST"), ("EXPMID", "61120.5"), ("RA_TARG", "10.0"), ("DEC_TARG", "-5.0")]);
        let path = write_image(dir.path(), "jwst.fits", &header);
        let overrides = GeometryOverrides { site_lat: Some(19.82), site_lon: Some(-155.47), ..GeometryOverrides::default() };
        let j = observation_geometry_json(&path, &overrides).unwrap();
        let geometry = &j[RES_GEOMETRY];
        assert!(geometry["jd_utc"].as_f64().unwrap().is_finite(), "{j}");
        for key in ["lst_deg", "altitude_deg", "azimuth_deg", "airmass_computed", "parallactic_angle_deg", "sun_altitude_deg", "moon_altitude_deg"] {
            assert!(geometry[key].is_null(), "{key}: {j}");
        }
        assert!(geometry["bjd_tdb"].is_null() && geometry["hjd_utc"].is_null(), "{j}");
        assert_eq!(j[RES_SITE]["source"], "user");
        let notes = geometry["time_scale_notes"].as_array().unwrap();
        assert!(notes.iter().any(|n| n.as_str().unwrap().contains("site from user not applied")), "{j}");
    }

    #[test]
    fn geometry_overrides_are_validated_with_the_named_limits() {
        let base = GeometryOverrides::default();
        let check = |overrides: GeometryOverrides, fragment: &str| {
            let err = validate_geometry_overrides(&overrides).unwrap_err().to_string();
            assert!(err.contains(fragment), "{err}");
        };
        check(GeometryOverrides { target_ra: Some(360.0), target_dec: Some(0.0), ..base }, "target_ra 360 must be at least 0 and below 360");
        check(GeometryOverrides { target_ra: Some(10.0), target_dec: Some(91.0), ..base }, "target_dec 91 must be between -90 and 90");
        check(GeometryOverrides { site_lat: Some(-91.0), site_lon: Some(0.0), ..base }, "site_lat -91 must be between -90 and 90");
        check(GeometryOverrides { site_lat: Some(0.0), site_lon: Some(400.0), ..base }, "site_lon 400 must be between -180 and 360");
        check(GeometryOverrides { site_lat: Some(0.0), site_lon: Some(0.0), site_height: Some(20000.0), ..base }, "site_height 20000 must be between -500 and 10000");
        check(GeometryOverrides { target_ra: Some(10.0), ..base }, "both target_ra and target_dec");
        check(GeometryOverrides { site_height: Some(100.0), ..base }, "both site_lat and site_lon");
        check(GeometryOverrides { site_lat: Some(10.0), ..base }, "both site_lat and site_lon");
        check(GeometryOverrides { target_ra: Some(f64::NAN), target_dec: Some(0.0), ..base }, "target_ra NaN");
        assert!(validate_geometry_overrides(&base).is_ok());
        let full = GeometryOverrides { target_ra: Some(359.9), target_dec: Some(-90.0), site_lat: Some(90.0), site_lon: Some(-180.0), site_height: Some(-500.0) };
        assert!(validate_geometry_overrides(&full).is_ok());
    }

    #[tokio::test]
    async fn observation_geometry_command_refuses_a_half_given_target() {
        let err = observation_geometry_cmd("C:/nowhere/frame.fits".to_string(), Some(10.0), None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("target_dec"), "{err}");
        let err = observation_geometry_cmd("C:/nowhere/frame.fits".to_string(), None, None, Some(1.0), None, None)
            .await
            .unwrap_err();
        assert!(err.contains("site_lon"), "{err}");
    }
}
