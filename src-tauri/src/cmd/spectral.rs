use serde::Serialize;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached_full};
use crate::core::astrometry::spectral::{
    header_target_coordinates, is_non_linear_spectral_ctype, radial_velocity_correction, spectral_axis_on,
    RadialVelocityCorrection, SpectralAxis,
};
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::types::header::HduHeader;
use crate::types::image_ref::{ImageRef, PlaneSelector};

pub const KEY_ERROR: &str = "error";
const COORDINATE_SOURCE_REQUEST: &str = "request";

pub(crate) fn with_spectral_header<T>(
    path: &str,
    read: impl FnOnce(&HduHeader) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let reference = ImageRef::parse(path);
    if matches!(reference.plane, PlaneSelector::Auto) {
        if let Ok(cube) = GLOBAL_CUBE_CACHE.get_or_open(&reference.path) {
            return read(&cube.header);
        }
    }
    let entry = load_cached_full(path)?;
    let header = entry
        .header()
        .ok_or_else(|| anyhow::anyhow!("no header available for {}", path))?;
    read(header)
}

pub fn header_spectral_axis(header: &HduHeader) -> Result<SpectralAxis, String> {
    if let Some(depth) = header.get_i64("NAXIS3").filter(|n| *n > 0) {
        return spectral_axis_on(header, 3, depth as usize);
    }
    for axis in [1usize, 2] {
        let Some(len) = header.get_i64(&format!("NAXIS{axis}")).filter(|n| *n > 0) else {
            continue;
        };
        let non_linear = header
            .get(&format!("CTYPE{axis}"))
            .map_or(false, |ctype| is_non_linear_spectral_ctype(ctype));
        match spectral_axis_on(header, axis, len as usize) {
            Ok(found) if found.kind.is_spectral() => return Ok(found),
            Err(reason) if non_linear => return Err(reason),
            _ => {}
        }
    }
    Err("no spectral axis: NAXIS3 is missing and neither CTYPE1 nor CTYPE2 is a spectral type".to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct CorrectionReport {
    #[serde(flatten)]
    pub correction: RadialVelocityCorrection,
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub coordinate_source: String,
}

fn resolve_target(header: &HduHeader, ra: Option<f64>, dec: Option<f64>) -> Result<(f64, f64, String), String> {
    match (ra, dec) {
        (Some(ra), Some(dec)) => Ok((ra, dec, COORDINATE_SOURCE_REQUEST.to_string())),
        (None, None) => header_target_coordinates(header)
            .map(|(ra, dec, source)| (ra, dec, source.to_string()))
            .ok_or_else(|| {
                "no target coordinates: pass RA and Dec or add CRVAL1/CRVAL2, RA/DEC or OBJCTRA/OBJCTDEC to the header"
                    .to_string()
            }),
        _ => Err("target coordinates need both RA and Dec".to_string()),
    }
}

pub fn correction_report(header: &HduHeader, ra: Option<f64>, dec: Option<f64>) -> Result<CorrectionReport, String> {
    let (ra_deg, dec_deg, coordinate_source) = resolve_target(header, ra, dec)?;
    let correction = radial_velocity_correction(header, ra_deg, dec_deg)?;
    Ok(CorrectionReport { correction, ra_deg, dec_deg, coordinate_source })
}

pub fn correction_json(header: &HduHeader, ra: Option<f64>, dec: Option<f64>) -> serde_json::Value {
    let report = correction_report(header, ra, dec)
        .and_then(|report| serde_json::to_value(&report).map_err(|e| e.to_string()));
    match report {
        Ok(value) => value,
        Err(reason) => json!({ KEY_ERROR: reason }),
    }
}

#[tauri::command]
pub async fn spectral_axis_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let axis = with_spectral_header(&path, |h| header_spectral_axis(h).map_err(anyhow::Error::msg))?;
        Ok(serde_json::to_value(&axis)?)
    })
}

#[tauri::command]
pub async fn radial_velocity_correction_cmd(
    path: String,
    ra: Option<f64>,
    dec: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        Ok(match with_spectral_header(&path, |h| Ok(correction_json(h, ra, dec))) {
            Ok(value) => value,
            Err(e) => json!({ KEY_ERROR: format!("{:#}", e) }),
        })
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use ndarray::Array2;

    use super::*;
    use crate::core::astrometry::spectral::{velocity_axis, AxisKind, SPEED_OF_LIGHT_KMS, VelocityConvention};
    use crate::core::imaging::region::test_support::make_header;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::BLOCK_SIZE;
    use crate::types::header::HduHeader;

    fn write_cube(path: &std::path::Path, cards: &[(&str, &str)], cols: usize, rows: usize, depth: usize) {
        let mut header = Vec::new();
        let mut push = |key: &str, value: &str| {
            let mut card = format!("{key:<8}= {value:>20}").into_bytes();
            card.resize(80, b' ');
            header.extend_from_slice(&card);
        };
        push("SIMPLE", "T");
        push("BITPIX", "-32");
        push("NAXIS", "3");
        push("NAXIS1", &cols.to_string());
        push("NAXIS2", &rows.to_string());
        push("NAXIS3", &depth.to_string());
        for (key, value) in cards {
            push(key, value);
        }
        push("END", "");
        while header.len() % BLOCK_SIZE != 0 {
            header.push(b' ');
        }
        let mut data: Vec<u8> = (0..cols * rows * depth).flat_map(|i| (i as f32 + 1.0).to_be_bytes()).collect();
        while data.len() % BLOCK_SIZE != 0 {
            data.push(0);
        }
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(&header).unwrap();
        file.write_all(&data).unwrap();
    }

    fn mauna_kea(extra: &[(&str, &str)]) -> HduHeader {
        let mut pairs = vec![
            ("SITELAT", "19.82"),
            ("SITELONG", "-155.47"),
            ("DATE-OBS", "2026-03-21T10:00:00"),
            ("EXPTIME", "600"),
        ];
        pairs.extend_from_slice(extra);
        make_header(&pairs)
    }

    #[test]
    fn header_spectral_axis_prefers_axis_3_then_a_spectral_axis_1_or_2() {
        let cube = make_header(&[
            ("NAXIS", "3"),
            ("NAXIS3", "3"),
            ("CTYPE3", "WAVE"),
            ("CUNIT3", "um"),
            ("CRVAL3", "1.0"),
            ("CDELT3", "0.5"),
            ("CTYPE1", "RA---TAN"),
            ("CRVAL1", "10"),
            ("CDELT1", "0.1"),
        ]);
        assert_eq!(header_spectral_axis(&cube).unwrap().values, vec![1.0, 1.5, 2.0]);
        let legacy_cube = make_header(&[("NAXIS", "3"), ("NAXIS3", "2"), ("CRVAL3", "10.0"), ("CDELT3", "2.0")]);
        let axis = header_spectral_axis(&legacy_cube).unwrap();
        assert_eq!(axis.kind, AxisKind::Unknown);
        assert_eq!(axis.values, vec![10.0, 12.0]);
        let long_slit = make_header(&[
            ("NAXIS", "2"),
            ("NAXIS1", "4"),
            ("NAXIS2", "10"),
            ("CTYPE1", "AWAV"),
            ("CUNIT1", "Angstrom"),
            ("CRVAL1", "5000"),
            ("CDELT1", "2"),
            ("CTYPE2", "LINEAR"),
        ]);
        let axis = header_spectral_axis(&long_slit).unwrap();
        assert_eq!(axis.kind, AxisKind::Awav);
        assert!((axis.values[3] - 0.5006).abs() < 1e-12, "{:?}", axis.values);
        let spatial_first = make_header(&[
            ("NAXIS", "2"),
            ("NAXIS1", "10"),
            ("NAXIS2", "2"),
            ("CTYPE1", "RA---TAN"),
            ("CRVAL1", "10"),
            ("CDELT1", "0.1"),
            ("CTYPE2", "FREQ"),
            ("CUNIT2", "GHz"),
            ("CRVAL2", "100"),
            ("CDELT2", "1"),
        ]);
        assert_eq!(header_spectral_axis(&spatial_first).unwrap().values, vec![100.0, 101.0]);
        let image = make_header(&[
            ("NAXIS", "2"),
            ("NAXIS1", "10"),
            ("NAXIS2", "10"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
            ("CRVAL1", "10"),
            ("CRVAL2", "20"),
            ("CDELT1", "0.1"),
            ("CDELT2", "0.1"),
        ]);
        assert!(header_spectral_axis(&image).unwrap_err().contains("no spectral axis"));
        let log = make_header(&[("NAXIS", "2"), ("NAXIS1", "4"), ("NAXIS2", "10"), ("CTYPE1", "WAVE-LOG"), ("CRVAL1", "1"), ("CDELT1", "0.1")]);
        assert!(header_spectral_axis(&log).unwrap_err().contains("non-linear"));
    }

    #[test]
    fn correction_json_defaults_coordinates_from_the_header_and_reports_errors_inline() {
        let h = mauna_kea(&[("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CRVAL1", "180.0"), ("CRVAL2", "0.0")]);
        let j = correction_json(&h, None, None);
        assert!(j[KEY_ERROR].is_null(), "{j}");
        assert_eq!(j["ra_deg"], 180.0);
        assert_eq!(j["dec_deg"], 0.0);
        assert_eq!(j["coordinate_source"], "CRVAL1/CRVAL2");
        assert_eq!(j["method"], "Meeus low-precision ephemeris");
        assert_eq!(j["accuracy_kms"], 0.02);
        assert!(j["barycentric_kms"].as_f64().unwrap().abs() < 1.0, "{j}");
        assert!(j["notes"].as_array().unwrap().len() >= 7);
        let explicit = correction_json(&h, Some(270.0), Some(-23.4392911));
        assert_eq!(explicit["coordinate_source"], "request");
        assert!(explicit["barycentric_kms"].as_f64().unwrap() > 29.0, "{explicit}");
        let lsrk = correction_json(&mauna_kea(&[("SPECSYS", "LSRK"), ("TELESCOP", "ALMA")]), Some(270.0), Some(-23.4392911));
        assert_eq!(lsrk["method"], "already in LSRK", "{lsrk}");
        assert_eq!(lsrk["barycentric_kms"], 0.0);
        assert_eq!(lsrk["heliocentric_kms"], 0.0);
        let no_coords = correction_json(&mauna_kea(&[]), None, None);
        assert!(no_coords[KEY_ERROR].as_str().unwrap().contains("coordinates"), "{no_coords}");
        let half = correction_json(&h, Some(1.0), None);
        assert!(half[KEY_ERROR].as_str().unwrap().contains("both"), "{half}");
        let jwst = correction_json(&make_header(&[("TELESCOP", "JWST")]), Some(1.0), Some(2.0));
        assert!(jwst[KEY_ERROR].as_str().unwrap().contains("JWST"), "{jwst}");
        assert!(jwst["barycentric_kms"].is_null());
    }

    #[test]
    fn spectral_axis_of_a_written_cube_and_a_two_dimensional_spectrum() {
        let dir = tempfile::tempdir().unwrap();
        let cube_path = dir.path().join("cube.fits");
        write_cube(
            &cube_path,
            &[("CTYPE3", "'AWAV    '"), ("CUNIT3", "'Angstrom'"), ("CRVAL3", "4750.0"), ("CD3_3", "1.25"), ("CRPIX3", "1.0")],
            2,
            2,
            3,
        );
        let cube_key = cube_path.to_str().unwrap();
        let axis = with_spectral_header(cube_key, |h| header_spectral_axis(h).map_err(anyhow::Error::msg)).unwrap();
        assert_eq!(axis.kind, AxisKind::Awav);
        assert_eq!(axis.values.len(), 3);
        assert!((axis.values[2] - 0.47525).abs() < 1e-12, "{:?}", axis.values);
        assert_eq!(axis.header_unit, "Angstrom");

        let spectrum_path = dir.path().join("spectrum.fits");
        let mut header = HduHeader::empty();
        header.set("CTYPE1", "WAVE".to_string());
        header.set("CUNIT1", "nm".to_string());
        header.set_f64("CRVAL1", 500.0);
        header.set_f64("CDELT1", 0.5);
        header.set_f64("CRPIX1", 1.0);
        let data = Array2::from_shape_fn((4, 8), |(y, x)| (y * 8 + x) as f32 + 1.0);
        write_fits_mono(spectrum_path.to_str().unwrap(), &data, Some(&header)).unwrap();
        let spectrum_key = spectrum_path.to_str().unwrap();
        let axis = with_spectral_header(spectrum_key, |h| header_spectral_axis(h).map_err(anyhow::Error::msg)).unwrap();
        assert_eq!(axis.kind, AxisKind::Wave);
        assert_eq!(axis.values.len(), 8);
        assert!((axis.values[7] - 0.5035).abs() < 1e-12, "{:?}", axis.values);
        let velocity = with_spectral_header(spectrum_key, |h| {
            let axis = header_spectral_axis(h).map_err(anyhow::Error::msg)?;
            velocity_axis(&axis, 0.5, VelocityConvention::Optical).map_err(anyhow::Error::msg)
        })
        .unwrap();
        assert!(velocity.values_kms[0].abs() < 1e-9);
        assert!((velocity.values_kms[7] - SPEED_OF_LIGHT_KMS * 0.0035 / 0.5).abs() < 1e-6, "{:?}", velocity.values_kms);

        let missing = with_spectral_header("C:/definitely/missing/spectrum.fits", |h| header_spectral_axis(h).map_err(anyhow::Error::msg));
        assert!(missing.is_err());
    }
}
