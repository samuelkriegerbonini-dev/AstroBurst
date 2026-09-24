use ndarray::Array2;

use crate::core::astrometry::spectral::spectral_axis;
use crate::types::header::HduHeader;

const SPECTRAL_CTYPES: [&str; 9] = ["WAVE", "FREQ", "VELO", "AWAV", "VRAD", "VOPT", "ZOPT", "BETA", "ENER"];
const SPECTRAL_UNITS: [&str; 19] = [
    "M", "CM", "MM", "UM", "MICRON", "MICRONS", "NM", "ANGSTROM", "ANGSTROMS", "A", "HZ", "KHZ", "MHZ",
    "GHZ", "THZ", "M/S", "KM/S", "EV", "KEV",
];
const INTEGRATION_CARDS: [&str; 2] = ["NINTS", "INTSTART"];
const NORMALIZED_VALID_FLOOR: f32 = 1e-6;
pub(crate) const DISPLAY_ASINH_ALPHA: f32 = 10.0;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpectralClassification {
    pub is_spectral: bool,
    pub reason: String,
    pub axis_type: Option<String>,
    pub axis_unit: Option<String>,
    pub axis_unit_assumed: bool,
    pub channel_count: usize,
}

fn assumed_axis_unit(header: &HduHeader, naxis3: usize) -> Option<String> {
    spectral_axis(header, naxis3)
        .ok()
        .filter(|axis| axis.kind.is_spectral() && !axis.header_unit.is_empty())
        .map(|axis| axis.header_unit.to_uppercase())
}

pub fn classify_spectral_cube(header: &HduHeader, naxis3: usize) -> SpectralClassification {
    let mut classification = classify_spectral_cube_from_cards(header, naxis3);
    if classification.is_spectral && classification.axis_unit.is_none() {
        if let Some(unit) = assumed_axis_unit(header, naxis3) {
            classification.axis_unit = Some(unit);
            classification.axis_unit_assumed = true;
        }
    }
    classification
}

fn card_upper(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_uppercase())
        .filter(|s| !s.is_empty())
}

fn classify_spectral_cube_from_cards(header: &HduHeader, naxis3: usize) -> SpectralClassification {
    let ctype3 = card_upper(header, "CTYPE3");
    let cunit3 = card_upper(header, "CUNIT3");
    let has_cdelt3 = header.get_f64("CDELT3").is_some();
    let has_crval3 = header.get_f64("CRVAL3").is_some();

    let ctype_is_spectral = ctype3
        .as_deref()
        .is_some_and(|ct| SPECTRAL_CTYPES.iter().any(|&s| ct.contains(s)));
    let cunit_is_spectral = cunit3.as_deref().is_some_and(|cu| SPECTRAL_UNITS.contains(&cu));
    let integration_stack = INTEGRATION_CARDS.iter().any(|&key| header.get(key).is_some());

    let (is_spectral, reason) = if ctype_is_spectral {
        (true, format!("CTYPE3 indicates spectral axis: {}", ctype3.as_deref().unwrap_or("")))
    } else if integration_stack {
        (false, format!("NAXIS3={} counts integrations (NINTS/INTSTART present), not spectral channels", naxis3))
    } else if cunit_is_spectral && has_cdelt3 {
        (true, format!("CUNIT3 indicates spectral data: {}", cunit3.as_deref().unwrap_or("")))
    } else if naxis3 <= 4 {
        (false, format!("NAXIS3={} with no spectral keywords: likely RGB/RGBA composition", naxis3))
    } else if has_cdelt3 && has_crval3 {
        (true, format!("NAXIS3={} with CRVAL3/CDELT3 present: likely spectral cube", naxis3))
    } else {
        (false, format!("NAXIS3={} with no spectral metadata: ambiguous, treating as non-spectral", naxis3))
    };

    SpectralClassification {
        is_spectral,
        reason,
        axis_type: ctype3,
        axis_unit: cunit3,
        axis_unit_assumed: false,
        channel_count: naxis3,
    }
}

pub fn build_wavelength_axis(header: &HduHeader) -> Option<Vec<f64>> {
    let naxis3 = header.get_i64("NAXIS3").filter(|n| *n > 0)? as usize;
    spectral_axis(header, naxis3).ok().map(|axis| axis.header_values())
}

#[derive(Debug, Clone)]
pub struct GlobalCubeStats {
    pub median: f32,
    pub sigma: f32,
    pub low: f32,
    pub high: f32,
}

pub fn normalize_with_global(data: &Array2<f32>, g: &GlobalCubeStats) -> Array2<f32> {
    let inv_sigma_alpha = DISPLAY_ASINH_ALPHA / g.sigma;
    let lo = (inv_sigma_alpha * (g.low - g.median)).asinh();
    let hi = (inv_sigma_alpha * (g.high - g.median)).asinh();
    let inv = 1.0 / (hi - lo).max(1e-6);

    data.mapv(|v| {
        if !v.is_finite() {
            return 0.0;
        }
        let clamped = v.clamp(g.low, g.high);
        let scaled = inv_sigma_alpha * (clamped - g.median);
        ((scaled.asinh() - lo) * inv).clamp(NORMALIZED_VALID_FLOOR, 1.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::make_header;
    use crate::core::imaging::stats::{is_padding, is_valid_pixel};

    #[test]
    fn normalize_with_global_keeps_pixels_below_median_visible() {
        let g = GlobalCubeStats { median: 100.0, sigma: 5.0, low: 88.0, high: 600.0 };
        let data = Array2::from_shape_vec(
            (2, 3),
            vec![88.0, 99.0, 100.0, 100.01, 600.0, f32::NAN],
        )
        .unwrap();
        let out = normalize_with_global(&data, &g);

        assert_eq!(out[[1, 2]], 0.0);
        assert!(is_padding(out[[1, 2]]));
        for &v in out.iter().take(5) {
            assert!(is_valid_pixel(v) && v > 0.0 && v <= 1.0, "{}", v);
        }
        assert!(out[[0, 0]] < out[[0, 1]]);
        assert!(out[[0, 1]] < out[[0, 2]]);
        assert!(out[[0, 2]] < out[[1, 0]]);
        assert!(out[[1, 0]] < out[[1, 1]]);
        assert!((out[[1, 1]] - 1.0).abs() < 1e-6);
        assert!(out[[0, 2]] > 0.3 && out[[0, 2]] < 0.4, "{}", out[[0, 2]]);
    }

    #[test]
    fn normalize_with_global_degenerate_range_stays_finite() {
        let g = GlobalCubeStats { median: 5.0, sigma: 1e-10, low: 5.0, high: 5.0 };
        let data = Array2::from_shape_vec((1, 2), vec![5.0, 7.0]).unwrap();
        let out = normalize_with_global(&data, &g);
        for &v in out.iter() {
            assert!(v.is_finite() && is_valid_pixel(v) && v > 0.0 && v <= 1.0, "{}", v);
        }
    }

    #[test]
    fn a_negative_valid_minimum_is_never_rendered_as_padding() {
        let g = GlobalCubeStats { median: 0.0, sigma: 1.0, low: -5.0, high: 5.0 };
        let data = Array2::from_shape_vec((1, 3), vec![-50.0, -5.0, f32::NEG_INFINITY]).unwrap();
        let out = normalize_with_global(&data, &g);
        assert!(is_valid_pixel(out[[0, 0]]) && is_valid_pixel(out[[0, 1]]), "{:?}", out);
        assert!(is_padding(out[[0, 2]]));
    }

    #[test]
    fn wavelength_axis_reads_cd3_3_and_pc3_3_and_keeps_header_units() {
        let muse = make_header(&[
            ("NAXIS3", "3"),
            ("CTYPE3", "AWAV"),
            ("CUNIT3", "Angstrom"),
            ("CRVAL3", "4750.0"),
            ("CD3_3", "1.25"),
            ("CRPIX3", "1.0"),
        ]);
        assert_eq!(build_wavelength_axis(&muse).unwrap(), vec![4750.0, 4751.25, 4752.5]);
        let pc = make_header(&[
            ("NAXIS3", "2"),
            ("CTYPE3", "FREQ"),
            ("CUNIT3", "Hz"),
            ("CRVAL3", "2.3e11"),
            ("CDELT3", "1.0e6"),
            ("PC3_3", "2.0"),
            ("CRPIX3", "1.0"),
        ]);
        let axis = build_wavelength_axis(&pc).unwrap();
        assert!((axis[1] - 2.30002e11).abs() < 1.0, "{:?}", axis);
    }

    #[test]
    fn wavelength_axis_keeps_the_legacy_contract_and_drops_non_linear_axes() {
        let legacy = make_header(&[("NAXIS3", "4"), ("CRVAL3", "10.0"), ("CDELT3", "2.0"), ("CRPIX3", "2.0")]);
        assert_eq!(build_wavelength_axis(&legacy).unwrap(), vec![8.0, 10.0, 12.0, 14.0]);
        let log = make_header(&[("NAXIS3", "4"), ("CTYPE3", "WAVE-LOG"), ("CRVAL3", "1.0"), ("CDELT3", "0.1")]);
        assert!(build_wavelength_axis(&log).is_none());
        let no_depth = make_header(&[("CRVAL3", "1.0"), ("CDELT3", "0.1")]);
        assert!(build_wavelength_axis(&no_depth).is_none());
        let no_step = make_header(&[("NAXIS3", "4"), ("CRVAL3", "1.0")]);
        assert!(build_wavelength_axis(&no_step).is_none());
    }

    #[test]
    fn classification_assumes_the_fits_default_unit_when_cunit3_is_missing() {
        let bare = make_header(&[("CTYPE3", "WAVE"), ("CRVAL3", "4.7e-7"), ("CDELT3", "1.25e-10")]);
        let c = classify_spectral_cube(&bare, 3000);
        assert!(c.is_spectral);
        assert_eq!(c.axis_unit.as_deref(), Some("M"));
        assert!(c.axis_unit_assumed);
        let explicit = make_header(&[("CTYPE3", "WAVE"), ("CUNIT3", "um"), ("CRVAL3", "1.0"), ("CDELT3", "0.01")]);
        let c = classify_spectral_cube(&explicit, 3000);
        assert_eq!(c.axis_unit.as_deref(), Some("UM"));
        assert!(!c.axis_unit_assumed);
        let no_axis = make_header(&[("CTYPE3", "WAVE")]);
        let c = classify_spectral_cube(&no_axis, 3000);
        assert!(c.is_spectral);
        assert!(c.axis_unit.is_none());
        assert!(!c.axis_unit_assumed);
    }

    #[test]
    fn unit_letters_inside_other_units_and_channel_counts_do_not_make_a_cube_spectral() {
        for unit in ["min", "ms", "arcsec", "s", "deg"] {
            let header = make_header(&[("CTYPE3", "TIME"), ("CUNIT3", unit), ("CDELT3", "1.0")]);
            let c = classify_spectral_cube(&header, 3);
            assert!(!c.is_spectral, "CUNIT3='{}' read as spectral: {}", unit, c.reason);
        }
        let offsets = make_header(&[("CUNIT3", "arcsec"), ("CDELT3", "0.1")]);
        assert!(!classify_spectral_cube(&offsets, 20).is_spectral);

        let calints = make_header(&[("NINTS", "50"), ("INTSTART", "1"), ("BUNIT", "MJy/sr")]);
        let c = classify_spectral_cube(&calints, 50);
        assert!(!c.is_spectral, "{}", c.reason);
        assert!(c.reason.contains("integrations"), "{}", c.reason);
        let bare = make_header(&[]);
        assert!(!classify_spectral_cube(&bare, 3000).is_spectral);

        let angstrom = make_header(&[("CUNIT3", "Angstrom"), ("CDELT3", "1.25")]);
        assert!(classify_spectral_cube(&angstrom, 3).is_spectral);
        let a = make_header(&[("CUNIT3", "A"), ("CDELT3", "1.25")]);
        assert!(classify_spectral_cube(&a, 3).is_spectral);
        let freq = make_header(&[("CTYPE3", "FREQ-LSR")]);
        assert!(classify_spectral_cube(&freq, 3).is_spectral);
        let ramp = make_header(&[("CRVAL3", "1.0"), ("CDELT3", "1.0")]);
        assert!(classify_spectral_cube(&ramp, 30).is_spectral);
    }
}
