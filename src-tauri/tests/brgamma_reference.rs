use astroburst_lib::core::cube::companions::{open_cube_set, CubeSet};
use astroburst_lib::core::cube::linefit::{fit_cube_lines, fit_spaxel, LineFitConfig, MASK_FITTED};
use astroburst_lib::infra::fits::writer::{write_mef_images, HduData, ImageHdu};
use astroburst_lib::types::header::HduHeader;
use ndarray::Array2;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Fixture {
    kind: String,
    rest_um: f64,
    depth: usize,
    crval3: f64,
    cdelt3: f64,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    x: usize,
    y: usize,
    input_sha256: String,
    sci: Vec<f32>,
    err: Vec<f32>,
    dq: Vec<u32>,
    expected: Option<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    amplitude: f64,
    velocity_kms: f64,
    sigma_kms: f64,
    sigma_corr_kms: f64,
    flux: f64,
    flux_err: f64,
    snr: f64,
    amplitude_err: f64,
    v_err_kms: f64,
    sigma_err_kms: f64,
    continuum_at_rest: f64,
    continuum_slope_per_kms: f64,
    dropped_dq: Vec<usize>,
    detected: bool,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/spectral/brgamma_reference.json")).unwrap()
}

fn open_fixture(fixture: &Fixture) -> (tempfile::TempDir, CubeSet, LineFitConfig) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-brgamma.fits");
    let sci: Vec<_> = (0..fixture.depth).map(|z| Array2::from_shape_fn((2, 3), |(y, x)| fixture.cases[y * 3 + x].sci[z])).collect();
    let err: Vec<_> = (0..fixture.depth).map(|z| Array2::from_shape_fn((2, 3), |(y, x)| fixture.cases[y * 3 + x].err[z])).collect();
    let dq: Vec<_> = (0..fixture.depth).map(|z| Array2::from_shape_fn((2, 3), |(y, x)| fixture.cases[y * 3 + x].dq[z])).collect();
    let mut header = HduHeader::empty();
    for (key, value) in [
        ("CTYPE3", "'WAVE'".to_string()),
        ("CUNIT3", "'um'".to_string()),
        ("CRPIX3", "1".to_string()),
        ("CRVAL3", fixture.crval3.to_string()),
        ("CDELT3", fixture.cdelt3.to_string()),
        ("BUNIT", "'Jy/beam'".to_string()),
        ("SPECSYS", "'TOPOCENT'".to_string()),
    ] {
        header.set(key, value);
    }
    write_mef_images(path.to_str().unwrap(), None, &[
        ImageHdu { data: HduData::F32Cube(&sci), header: Some(&header), extname: "SCI", extver: 1 },
        ImageHdu { data: HduData::F32Cube(&err), header: Some(&header), extname: "ERR", extver: 1 },
        ImageHdu { data: HduData::U32Cube(&dq), header: None, extname: "DQ", extver: 1 },
    ]).unwrap();
    let set = open_cube_set(path.to_str().unwrap()).unwrap();
    let config = serde_json::from_value(serde_json::json!({
        "z0": 35, "z1": 85, "rest_um": fixture.rest_um, "convention": "optical",
        "continuum": [[0, 19], [101, 120]], "snr_threshold": 3,
        "emission_only": false, "use_err": true, "dq_policy": "do_not_use",
        "resolving_power": 2700, "components": "one"
    })).unwrap();
    (dir, set, config)
}

fn close(name: &str, actual: f64, expected: f64, relative: f64, absolute: f64) {
    let tolerance = absolute + relative * expected.abs();
    assert!(actual.is_finite() && (actual - expected).abs() <= tolerance,
        "{name}: actual={actual:.12e}, scipy={expected:.12e}, tolerance={tolerance:.3e}");
}

#[test]
fn brgamma_fixture_is_identified_and_non_square() {
    let fixture = fixture();
    assert_eq!(fixture.kind, "synthetic_brgamma_scipy_reference");
    assert_eq!(fixture.rest_um, 2.16612);
    assert_eq!(fixture.cases.len(), 6);
    for (index, case) in fixture.cases.iter().enumerate() {
        assert_eq!((case.x, case.y), (index % 3, index / 3));
        assert_eq!(case.sci.len(), fixture.depth);
        assert_eq!(case.err.len(), fixture.depth);
        assert_eq!(case.dq.len(), fixture.depth);
        let mut digest = Sha256::new();
        for value in case.sci.iter().chain(&case.err) { digest.update(value.to_le_bytes()); }
        for value in &case.dq { digest.update(value.to_le_bytes()); }
        assert_eq!(format!("{:x}", digest.finalize()), case.input_sha256, "{}", case.name);
    }
}

#[test]
fn brgamma_spaxels_match_independent_scipy_fit_and_continuum_uncertainty() {
    let fixture = fixture();
    let (_dir, set, config) = open_fixture(&fixture);
    for case in &fixture.cases {
        let fit = fit_spaxel(&set.inputs(), &config, case.x, case.y).unwrap();
        let Some(reference) = &case.expected else {
            assert!(!fit.converged);
            assert!(fit.single.is_none());
            assert!(fit.channels.is_empty());
            continue;
        };
        assert!(fit.converged, "{}: {:?}", case.name, fit.notes);
        assert_eq!(fit.mask & MASK_FITTED != 0, reference.detected, "{}", case.name);
        assert_eq!(fit.dropped_dq, reference.dropped_dq, "{}", case.name);
        let c = fit.single.unwrap();
        close(&format!("{} amplitude", case.name), c.amplitude, reference.amplitude, 5e-4, 1e-6);
        close(&format!("{} velocity", case.name), c.velocity_kms, reference.velocity_kms, 0.0, 0.02);
        close(&format!("{} sigma", case.name), c.sigma_kms, reference.sigma_kms, 0.0, 0.02);
        close("instrumental correction", c.sigma_corr_kms.unwrap(), reference.sigma_corr_kms, 5e-4, 0.02);
        close("integrated flux", c.flux, reference.flux, 5e-4, 1e-5);
        for (name, actual, expected) in [
            ("flux uncertainty", c.flux_err, reference.flux_err),
            ("amplitude uncertainty", c.amplitude_err, reference.amplitude_err),
            ("velocity uncertainty", c.v_err_kms, reference.v_err_kms),
            ("sigma uncertainty", c.sigma_err_kms, reference.sigma_err_kms),
            ("integrated S/N", c.snr, reference.snr),
        ] { close(name, actual, expected, 0.01, 1e-5); }
        let continuum = fit.continuum.unwrap();
        close("continuum at rest", continuum.intercept, reference.continuum_at_rest, 1e-7, 1e-8);
        close("continuum slope", continuum.slope * fixture.cdelt3 / 20.0, reference.continuum_slope_per_kms, 1e-7, 1e-10);
        eprintln!("{}: flux={:.8}, velocity={:.8} km/s, sigma={:.8} km/s, flux_err={:.8}, S/N={:.5}, detected={}",
            case.name, c.flux, c.velocity_kms, c.sigma_kms, c.flux_err, c.snr, reference.detected);
    }
}

#[test]
fn brgamma_maps_preserve_detection_states_units_and_spaxel_values() {
    let fixture = fixture();
    let (_dir, set, config) = open_fixture(&fixture);
    let maps = fit_cube_lines(&set.inputs(), &config).unwrap();
    assert_eq!(maps.flux_unit, "Jy/beam km/s");
    assert_eq!(maps.n_fit, 4);
    assert_eq!(maps.n_masked, 1);
    for case in &fixture.cases {
        let position = (case.y, case.x);
        let fit = fit_spaxel(&set.inputs(), &config, case.x, case.y).unwrap();
        if case.expected.as_ref().is_some_and(|reference| reference.detected) {
            let c = fit.single.unwrap();
            assert_eq!(maps.flux[position].to_bits(), (c.flux as f32).to_bits());
            assert_eq!(maps.velocity[position].to_bits(), (c.velocity_kms as f32).to_bits());
            assert_eq!(maps.flux_err[position].to_bits(), (c.flux_err as f32).to_bits());
        } else {
            assert!(maps.flux[position].is_nan());
            assert!(maps.velocity[position].is_nan());
        }
    }
    assert!(maps.snr[(1, 1)].is_finite());
    assert!(maps.snr[(1, 1)].abs() < 3.0);
    assert!(maps.snr[(1, 2)].is_nan());
}
