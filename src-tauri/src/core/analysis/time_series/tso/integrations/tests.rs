use serde_json::json;

use super::test_support::{synth_file, synth_integration, synth_series};
use super::*;

#[test]
fn tso_types_serialise_with_the_names_the_frontend_reads() {
    let key = SeriesKey { detector: "NRS1".into(), sporder: Some(-1) };
    assert_eq!(serde_json::to_value(&key).unwrap(), json!({"detector": "NRS1", "sporder": -1}));
    let without_order = SeriesKey { detector: "NRS1".into(), sporder: None };
    assert_eq!(serde_json::to_value(&without_order).unwrap(), json!({"detector": "NRS1", "sporder": null}));
    assert_eq!(serde_json::to_value(TsoLayout::Rows).unwrap(), json!("rows"));
    assert_eq!(serde_json::to_value(TsoLayout::Extensions).unwrap(), json!("extensions"));
    assert_eq!(serde_json::to_value(TsoTimeScale::BmjdTdb).unwrap(), json!("bmjd_tdb"));
    assert_eq!(serde_json::to_value(TsoTimeScale::MjdUtc).unwrap(), json!("mjd_utc"));
    assert_eq!(serde_json::to_value(TsoTimeScale::Index).unwrap(), json!("index"));
    let parsed: SeriesKey = serde_json::from_value(json!({"detector": "NIS", "sporder": 2})).unwrap();
    assert_eq!(parsed, SeriesKey { detector: "NIS".into(), sporder: Some(2) });

    let file = serde_json::to_value(synth_file("jw_x1dints.fits", "NRS_BRIGHTOBJ", "CLEAR", 3)).unwrap();
    assert_eq!(file["path"], json!("jw_x1dints.fits"));
    assert_eq!(file["file_name"], json!("jw_x1dints.fits"));
    assert_eq!(file["sha256"], json!("0".repeat(64)));
    assert_eq!(file["level"], json!(3));
    assert_eq!(file["datamodel"], json!("TSOMultiSpecModel"));
    assert_eq!(file["layout"], json!("rows"));
    assert_eq!(file["exp_type"], json!("NRS_BRIGHTOBJ"));
    assert_eq!(file["filter"], json!("CLEAR"));
    assert_eq!(file["instrument"], json!(null));
    assert_eq!(file["detector"], json!(null));
    assert_eq!(file["whitelight_log"], json!(null));
    assert_eq!(file["whtlt_path"], json!(null));
    assert_eq!(file["n_extract1d"], json!(1));
    assert_eq!(file["data_bytes"], json!(0));
    let log = WhiteLightLog { min_um: Some(2.47), max_um: None };
    assert_eq!(serde_json::to_value(&log).unwrap(), json!({"min_um": 2.47, "max_um": null}));
}

#[test]
fn a_sample_is_valid_only_with_a_finite_wavelength_a_finite_flux_and_dq_bit_0_clear() {
    assert!(sample_is_valid(1.0, 2.0, 0));
    assert!(!sample_is_valid(f64::NAN, 2.0, 0));
    assert!(!sample_is_valid(1.0, f64::NAN, 0));
    assert!(!sample_is_valid(1.0, f64::INFINITY, 0));
    assert!(!sample_is_valid(1.0, 2.0, 1));
    assert!(!sample_is_valid(1.0, 2.0, 3));
    assert!(sample_is_valid(1.0, 2.0, 2));
    assert!(sample_is_valid(1.0, -5.0, 0));
}

fn two_rows() -> Vec<test_support::SynthIntegration> {
    vec![synth_integration(1, 60000.0, &[1.0, 2.0, 3.0]), synth_integration(2, 60000.5, &[4.0, f64::NAN, 6.0])]
}

#[test]
fn the_synthetic_series_builder_fills_counts_times_and_valid_samples() {
    let series = synth_series("NRS1", Some(0), &[1.0, 1.1, 1.2], two_rows(), TsoTimeScale::BmjdTdb);
    assert_eq!(series.key, SeriesKey { detector: "NRS1".into(), sporder: Some(0) });
    assert_eq!(series.n_integrations, 2);
    assert_eq!(series.n_channels, 3);
    assert_eq!(series.n_valid, vec![3, 2]);
    assert_eq!(series.time(), vec![60000.002, 60000.502]);
    assert_eq!(series.int_num, vec![1, 2]);
    assert!(!series.is_valid(1, 1));
    assert!(series.is_valid(1, 2));
    assert_eq!(series.sample(1, 2), 5);
    assert_eq!(series.grid_um, vec![1.0, 1.1, 1.2]);
    assert!(series.common_grid);
    assert_eq!(series.wavelength_um.len(), 6);
    assert_eq!(series.wavelength_um[3..], [1.0, 1.1, 1.2]);
    assert_eq!(series.flux[..3], [1.0, 2.0, 3.0]);
    assert_eq!(series.flux_error, vec![0.5; 6]);
    assert_eq!(series.dq, vec![0; 6]);
    assert_eq!(series.npixels, vec![7.0; 6]);
    assert_eq!(series.mjd_utc_mid, vec![60000.0, 60000.5]);
    assert_eq!(series.segment, vec![1, 1]);
    assert_eq!(series.file_index, vec![0, 0]);
    assert_eq!(series.flux_unit, "Jy");
    assert_eq!(series.time_source, "TDB-MID column");
    assert!(series.completeness.is_empty() && series.notes.is_empty() && series.warnings.is_empty());

    let by_index = synth_series("NRS1", Some(0), &[1.0, 1.1, 1.2], two_rows(), TsoTimeScale::Index);
    assert_eq!(by_index.time(), vec![1.0, 2.0]);
    assert_eq!(by_index.time_source, "integration number (no usable times)");
    let by_utc = synth_series("NRS1", Some(0), &[1.0, 1.1, 1.2], two_rows(), TsoTimeScale::MjdUtc);
    assert_eq!(by_utc.time(), vec![60000.0, 60000.5]);
    assert_eq!(by_utc.time_source, "MJD-AVG (UTC, not barycentric)");
}

fn cited_script(text: &str) -> Option<String> {
    let start = text.find("scripts/oracle/")?;
    let end = start + text[start..].find(".py")? + ".py".len();
    Some(text[start..end].to_string())
}

#[test]
fn every_time_series_fixture_cites_a_generating_script_kept_in_the_repository() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest.parent().expect("the crate sits inside the repository");
    let mut checked = 0;
    for (folder, expected) in [("transit", 3), ("tso", 4), ("tso/goldens", 36), ("lightcurve", 1)] {
        let mut found = 0;
        for entry in std::fs::read_dir(manifest.join("tests").join("fixtures").join(folder)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let document: serde_json::Value = serde_json::from_str(&text).unwrap();
            let provenance = [&document["produced_by"], &document["produced_by"]["command"], &document["source"]];
            let script = provenance
                .iter()
                .filter_map(|field| field.as_str())
                .find_map(cited_script)
                .unwrap_or_else(|| panic!("{}: neither produced_by nor source cites a script under scripts/oracle", path.display()));
            assert!(repository.join(&script).is_file(), "{} cites {script}, which is not in the repository", path.display());
            found += 1;
        }
        assert_eq!(found, expected, "JSON fixtures under tests/fixtures/{folder}");
        checked += found;
    }
    assert_eq!(checked, 44);
}
