use serde::Serialize;
use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header};
use crate::core::metadata::filter_wavelengths::{FilterError, FilterRegistry, FilterSource};
use crate::service::cubeprep::filter_registry_report;
use crate::types::constants::RES_ERROR;
use crate::types::header::HduHeader;
use crate::types::image_ref::{source_file, ImageRef};

pub(crate) const FILTER_SOURCE_FILE_NAME: &str = "file name";
const ANGSTROM_PER_MICROMETRE: f64 = 1.0e4;
const WARNING_SEPARATOR: &str = "; ";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IdentifiedFilter {
    pub dataset: String,
    pub filter: String,
    pub pivot_um: f64,
    pub instrument: String,
    pub source: String,
    pub warning: Option<String>,
}

fn skipped_for<'a>(skipped: &'a [String], dataset: &str) -> impl Iterator<Item = &'a String> + 'a {
    let prefix = format!("custom filter dataset {dataset} is not used:");
    skipped.iter().filter(move |text| text.starts_with(&prefix))
}

pub(crate) fn identify_filter_with(
    registry: &FilterRegistry,
    skipped: &[String],
    header: Option<&HduHeader>,
    path: &str,
    dataset: Option<&str>,
) -> Result<IdentifiedFilter, String> {
    let reference = ImageRef::parse(path);
    let file = source_file(&reference.path);
    let dataset = dataset.map(str::trim).filter(|name| !name.is_empty());
    match registry.identify(header, file, dataset) {
        Ok(found) => {
            let warnings: Vec<String> = found.warnings.iter().chain(skipped_for(skipped, &found.entry.dataset)).cloned().collect();
            Ok(IdentifiedFilter {
                dataset: found.entry.dataset,
                filter: found.entry.filter,
                pivot_um: found.entry.pivot_angstrom / ANGSTROM_PER_MICROMETRE,
                instrument: found.entry.instrument,
                source: match found.source {
                    FilterSource::Header { keyword } => format!("header {keyword}"),
                    FilterSource::FileName => FILTER_SOURCE_FILE_NAME.to_string(),
                },
                warning: (!warnings.is_empty()).then(|| warnings.join(WARNING_SEPARATOR)),
            })
        }
        Err(error) => {
            let mut text = error.to_string();
            if let FilterError::UnknownDataset { dataset, .. } = &error {
                for warning in skipped_for(skipped, dataset) {
                    text.push_str(WARNING_SEPARATOR);
                    text.push_str(warning);
                }
            }
            Err(text)
        }
    }
}

pub(crate) fn identify_filter_in(header: &HduHeader, path: &str, dataset: Option<&str>) -> Result<IdentifiedFilter, String> {
    let (registry, skipped) = filter_registry_report();
    identify_filter_with(&registry, &skipped, Some(header), path, dataset)
}

pub(crate) fn identify_filter_for_path(path: &str, dataset: Option<&str>) -> Result<IdentifiedFilter, String> {
    let header = cached_header(path).map_err(|error| format!("{error:#}"))?;
    identify_filter_in(&header, path, dataset)
}

pub(crate) fn identify_filter_json(path: &str, dataset: Option<&str>) -> anyhow::Result<serde_json::Value> {
    match identify_filter_for_path(path, dataset) {
        Ok(found) => Ok(serde_json::to_value(&found)?),
        Err(text) => Ok(json!({ RES_ERROR: text })),
    }
}

#[tauri::command]
pub async fn identify_filter_cmd(path: String, dataset: Option<String>) -> Result<serde_json::Value, String> {
    if path.trim().is_empty() {
        return Err("identify_filter_cmd: the path is empty".to_string());
    }
    blocking_cmd!(identify_filter_json(&path, dataset.as_deref()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::core::imaging::region::test_support::make_header;
    use crate::core::metadata::photcal::{missing_calibration_reason, PhotCal};
    use crate::infra::fits::reader::test_fixtures::{cube_hdu, empty_primary_cards, write_raw_hdus};
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::config::CustomFilter;

    type Expected<'a> = (&'a str, &'a str, f64, &'a str, &'a str);

    const PIVOT_TOLERANCE_UM: f64 = 1e-12;
    const CONTRACT_KEYS: [&str; 6] = ["dataset", "filter", "instrument", "pivot_um", "source", "warning"];
    const LEGACY_ASSUMPTION: &str = "dataset DECAM is assumed from SURVEY = LegacySurvey (Legacy Surveys DR10 imaging is DECam)";
    const DECAM_R_IMAGE: &str = "C:/d/legacysurvey-0532m280-image-r.fits.fz";
    const DECAM_R: Expected<'static> = ("DECAM", "r", 0.64176, "DECam", "header FILTER");
    const JWST_F200W: Expected<'static> = ("JWST", "F200W", 1.988648, "NIRCam", "header FILTER");
    const PLAIN_REFUSAL: &str = "'plain.fits': the header does not identify a registry dataset, so the dataset must be given";
    const LEGACY_BAND_CARDS: [(&str, &str); 8] = [
        ("SURVEY", "LegacySurvey"),
        ("VERSION", "DR10"),
        ("IMAGETYP", "IMAGE"),
        ("BANDS", "griz"),
        ("BAND0", "g"),
        ("BAND1", "r"),
        ("BAND2", "i"),
        ("BAND3", "z"),
    ];

    fn decam_r_header() -> HduHeader {
        make_header(&[("TELESCOP", "CTIO 4.0-m telescope"), ("INSTRUME", "DECam"), ("FILTER", "r DECam SDSS c0002 6415.0 1480.0")])
    }

    fn jwst_f200w_header() -> HduHeader {
        make_header(&[("TELESCOP", "JWST"), ("INSTRUME", "NIRCAM"), ("FILTER", "F200W")])
    }

    fn legacy_header(extra: &[(&str, &str)]) -> HduHeader {
        make_header(&[&LEGACY_BAND_CARDS[..], extra].concat())
    }

    fn loaded_plane_one() -> HduHeader {
        legacy_header(&[("NAXIS", "2"), ("ABPLHDU", "0"), ("ABPLANE", "1"), ("ABPLBAND", "r")])
    }

    fn legacy_cube() -> HduHeader {
        legacy_header(&[("NAXIS", "3"), ("NAXIS3", "4")])
    }

    fn assert_identified(answer: &IdentifiedFilter, expected: Expected<'_>) {
        let (dataset, filter, pivot_um, instrument, source) = expected;
        assert_eq!(
            (answer.dataset.as_str(), answer.filter.as_str(), answer.instrument.as_str(), answer.source.as_str()),
            (dataset, filter, instrument, source),
            "{answer:?}"
        );
        assert!((answer.pivot_um - pivot_um).abs() < PIVOT_TOLERANCE_UM, "{answer:?}");
    }

    fn sorted_keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap_or_else(|| panic!("{value}")).keys().cloned().collect();
        keys.sort();
        keys
    }

    fn write_image(dir: &Path, name: &str, header: &HduHeader) -> String {
        let path = dir.join(name);
        write_fits_mono(path.to_str().unwrap(), &ndarray::Array2::<f32>::from_elem((16, 16), 1.0), Some(header)).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn write_legacy_cut(dir: &Path) -> String {
        let path = dir.join("cut.fits");
        let mut primary = empty_primary_cards();
        primary.push(("SURVEY", "'LegacySurvey'".into()));
        let cube = cube_hdu(
            "IMAGE",
            4,
            3,
            4,
            &[
                ("BANDS", "'griz    '".into()),
                ("BAND0", "'g       '".into()),
                ("BAND1", "'r       '".into()),
                ("BAND2", "'i       '".into()),
                ("BAND3", "'z       '".into()),
            ],
        );
        write_raw_hdus(&path, &[(primary, Vec::new()), cube]);
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn a_header_filter_is_answered_with_its_pivot_in_micrometres_and_its_source() {
        let registry = FilterRegistry::builtin();
        let none: &[String] = &[];
        let ps1 = make_header(&[("HIERARCH FPA.TELESCOPE", "PS1"), ("HIERARCH FPA.FILTER", "r.00000")]);
        let rows: [(Option<HduHeader>, &str, Option<&str>, Expected<'_>); 4] = [
            (Some(decam_r_header()), DECAM_R_IMAGE, None, DECAM_R),
            (Some(jwst_f200w_header()), "C:/d/jw_nircam.fits#hdu=1", None, JWST_F200W),
            (Some(ps1), "ps1_A_r_stack_480px.fits", None, ("PS1", "r", 0.62012, "GPC1", "header HIERARCH FPA.FILTER")),
            (None, "C:/d/JWST_F444W.fits", Some("jwst"), ("JWST", "F444W", 4.404315, "NIRCam", "file name")),
        ];
        for (header, path, dataset, expected) in rows {
            let answer = identify_filter_with(&registry, none, header.as_ref(), path, dataset).unwrap_or_else(|error| panic!("{path}: {error}"));
            assert_identified(&answer, expected);
            assert_eq!(answer.warning, None, "{path}");
        }
    }

    #[test]
    fn the_real_file_is_named_for_every_kind_of_path() {
        let registry = FilterRegistry::builtin();
        let unnamed = make_header(&[("OBJECT", "x")]);
        for path in ["C:/d/plain.fits", "C:/d/plain.fits#hdu=2", "C:/d/plain.fits#hdu=0&plane=1"] {
            assert_eq!(identify_filter_with(&registry, &[], Some(&unnamed), path, None), Err(PLAIN_REFUSAL.to_string()), "{path}");
        }
        assert_eq!(
            identify_filter_with(&registry, &[], Some(&unnamed), "__composite_r", None),
            Err("'__composite_r': the header does not identify a registry dataset, so the dataset must be given".to_string())
        );
    }

    #[test]
    fn a_loaded_band_plane_is_answered_with_its_band() {
        let registry = FilterRegistry::builtin();
        let answer = identify_filter_with(&registry, &[], Some(&loaded_plane_one()), "C:/d/ls-dr10_A.fits#hdu=0&plane=1", None).unwrap();
        assert_identified(&answer, ("DECAM", "r", 0.64176, "DECam", "header BAND1"));
        assert_eq!(answer.warning.as_deref(), Some(LEGACY_ASSUMPTION));
        assert_eq!(
            identify_filter_with(&registry, &[], Some(&legacy_cube()), "C:/d/ls-dr10_A.fits#hdu=0", None),
            Err("'ls-dr10_A.fits': the file holds the bands g, r, i, z as planes; open one plane to identify its filter".to_string())
        );
    }

    #[test]
    fn a_dataset_named_by_the_caller_is_trimmed_and_an_empty_one_counts_as_absent() {
        let registry = FilterRegistry::builtin();
        let header = decam_r_header();
        let unnamed = identify_filter_with(&registry, &[], Some(&header), DECAM_R_IMAGE, None).unwrap();
        assert_identified(&unnamed, DECAM_R);
        for dataset in ["  decam ", "", "   "] {
            assert_eq!(identify_filter_with(&registry, &[], Some(&header), DECAM_R_IMAGE, Some(dataset)), Ok(unnamed.clone()), "{dataset:?}");
        }
        assert_eq!(
            identify_filter_with(&registry, &[], Some(&header), DECAM_R_IMAGE, Some("PS1")),
            Err("'legacysurvey-0532m280-image-r.fits.fz': the header identifies DECAM but the dataset PS1 was asked for".to_string())
        );
        let unknown = identify_filter_with(&registry, &[], Some(&header), DECAM_R_IMAGE, Some("nosuch")).unwrap_err();
        assert!(unknown.starts_with("unknown dataset 'NOSUCH'; the registry has JWST, HST, "), "{unknown}");
        assert!(!unknown.contains("custom filter dataset"), "{unknown}");
    }

    #[test]
    fn skipped_custom_datasets_are_told_only_where_they_concern_the_answer() {
        let datasets: BTreeMap<String, BTreeMap<String, CustomFilter>> = serde_json::from_value(json!({
            "EUCLID": {"VIS": {"lambda": 7180.0, "instrument": "VIS", "group": "optical"}},
            "BAD": {"X": {"lambda": -1.0, "instrument": "I", "group": "g"}},
            "CFHT": {"u": {"lambda": 3676.1, "instrument": "MegaCam", "group": "optical"}}
        }))
        .unwrap();
        let (registry, skipped) = FilterRegistry::with_custom_datasets(&datasets);
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        let skipped_for = |name: &str| {
            let prefix = format!("custom filter dataset {name} is not used:");
            skipped.iter().find(|text| text.starts_with(&prefix)).cloned().unwrap_or_else(|| panic!("{name}: {skipped:?}"))
        };
        let (euclid, bad) = (skipped_for("EUCLID"), skipped_for("BAD"));

        let vis = identify_filter_with(&registry, &skipped, Some(&make_header(&[("FILTER", "VIS")])), "EUC_MER_BGSUB-MOSAIC-VIS_TILE1.fits", None)
            .unwrap();
        assert_eq!((vis.dataset.as_str(), vis.filter.as_str()), ("EUCLID", "VIS"));
        assert!((vis.pivot_um - 0.71034).abs() < PIVOT_TOLERANCE_UM, "{vis:?}");
        assert_eq!(vis.warning, Some(euclid.clone()));

        let jwst = identify_filter_with(&registry, &skipped, Some(&jwst_f200w_header()), "C:/d/jw_nircam.fits#hdu=1", None).unwrap();
        assert_identified(&jwst, JWST_F200W);
        assert_eq!(jwst.warning, None);

        let cfht = identify_filter_with(&registry, &skipped, Some(&make_header(&[("FILTER", "u")])), "CFHT_u.fits", Some("cfht")).unwrap();
        assert_identified(&cfht, ("CFHT", "u", 0.36761, "MegaCam", "header FILTER"));
        assert_eq!(cfht.warning, None);

        let refused = identify_filter_with(&registry, &skipped, Some(&make_header(&[("FILTER", "X")])), "image.fits", Some("bad")).unwrap_err();
        assert!(refused.starts_with("unknown dataset 'BAD'; the registry has "), "{refused}");
        assert!(refused.ends_with(&format!("; {bad}")), "{refused}");
    }

    #[test]
    fn the_answer_serialises_with_the_six_keys_of_the_contract() {
        let answer = identify_filter_with(&FilterRegistry::builtin(), &[], Some(&decam_r_header()), DECAM_R_IMAGE, None).unwrap();
        let value = serde_json::to_value(&answer).unwrap();
        assert_eq!(sorted_keys(&value), CONTRACT_KEYS);
        assert!(value["warning"].is_null(), "{value}");
        assert_eq!(value["source"], "header FILTER");
        assert!(value["pivot_um"].as_f64().is_some_and(|pivot| (pivot - 0.64176).abs() < PIVOT_TOLERANCE_UM), "{value}");
    }

    #[test]
    fn the_path_function_reads_a_file_and_one_plane_of_a_cube() {
        let dir = tempfile::tempdir().unwrap();
        let jw = write_image(dir.path(), "jw.fits", &jwst_f200w_header());
        let answer = identify_filter_for_path(&jw, None).unwrap();
        assert_eq!((answer.dataset.as_str(), answer.filter.as_str(), answer.source.as_str()), ("JWST", "F200W", "header FILTER"));

        let cut = write_legacy_cut(dir.path());
        for (plane, filter) in [(1, "r"), (3, "z")] {
            let key = format!("{cut}#hdu=1&plane={plane}");
            let answer = identify_filter_for_path(&key, None).unwrap_or_else(|error| panic!("{key}: {error}"));
            assert_eq!(
                (answer.dataset.as_str(), answer.filter.as_str(), answer.source.as_str()),
                ("DECAM", filter, format!("header BAND{plane}").as_str())
            );
            assert!(
                answer.warning.as_deref().is_some_and(|warning| warning.starts_with("dataset DECAM is assumed from SURVEY = LegacySurvey")),
                "{answer:?}"
            );
        }
        assert_eq!(
            identify_filter_for_path(&format!("{cut}#hdu=1"), None),
            Err("'cut.fits': the file holds the bands g, r, i, z as planes; open one plane to identify its filter".to_string())
        );
        let beyond = identify_filter_for_path(&format!("{cut}#hdu=1&plane=9"), None).unwrap_err();
        assert!(beyond.contains("plane 9 is out of range"), "{beyond}");
        assert!(identify_filter_for_path("C:/nowhere/missing.fits", None).is_err());
    }

    #[tokio::test]
    async fn the_command_answers_with_the_filter_or_with_one_error_key() {
        let dir = tempfile::tempdir().unwrap();
        let jw = write_image(dir.path(), "jw.fits", &jwst_f200w_header());
        let answer = identify_filter_cmd(jw.clone(), None).await.unwrap();
        assert_eq!(sorted_keys(&answer), CONTRACT_KEYS);
        assert_eq!(answer["dataset"], "JWST");
        assert_eq!(identify_filter_cmd(jw, Some("  ".into())).await.unwrap(), answer);

        let plain = write_image(dir.path(), "plain.fits", &make_header(&[("OBJECT", "x")]));
        assert_eq!(identify_filter_cmd(plain, None).await.unwrap(), json!({ "error": PLAIN_REFUSAL }));
        assert_eq!(identify_filter_cmd("   ".to_string(), None).await.unwrap_err(), "identify_filter_cmd: the path is empty");
    }

    const SCIENCE_DATA_DIR_VAR: &str = "ASTROBURST_SCIENCE_DATA_DIR";
    const REAL_FACTOR_TOLERANCE: f64 = 5e-7;
    const MICROJANSKY_PER_JANSKY: f64 = 1e6;
    const UJY_PER_NANOMAGGY: f64 = 3.630780547701009;
    const L1: &str = "L1/ls-dr10_A_griz_invvar_maskbits_458px.fits";
    const E1_SCI: &str = "E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits";
    const E1_RMS: &str = "E1/EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits";
    const E1_FLAG: &str = "E1/EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits";
    const L2_MASKBITS: &str = "L2/legacysurvey-0532m280-maskbits.fits.fz";
    const L1_CUBE_REFUSAL: &str =
        "'ls-dr10_A_griz_invvar_maskbits_458px.fits': the file holds the bands g, r, i, z as planes; open one plane to identify its filter";
    const L2_MASKBITS_REFUSAL: &str =
        "'legacysurvey-0532m280-maskbits.fits.fz': the header does not identify a registry dataset, so the dataset must be given";
    const LEGACY_PLANES: [(&str, f64); 4] = [("g", 0.48085), ("r", 0.64176), ("i", 0.78146), ("z", 0.91689)];

    fn survey_key(relative: &str) -> String {
        env_dir(SCIENCE_DATA_DIR_VAR).join("survey").join(relative).to_string_lossy().into_owned()
    }

    fn real_header(relative: &str) -> HduHeader {
        cached_header(&survey_key(relative)).unwrap_or_else(|error| panic!("{relative}: {error:#}"))
    }

    fn real_answer(relative: &str) -> Result<IdentifiedFilter, String> {
        let answer = identify_filter_with(&FilterRegistry::builtin(), &[], Some(&real_header(relative)), &survey_key(relative), None);
        match &answer {
            Ok(found) => println!(
                "{relative}: {} {} pivot {} um, {}, source {}, warning {:?}",
                found.dataset, found.filter, found.pivot_um, found.instrument, found.source, found.warning
            ),
            Err(text) => println!("{relative}: error {text}"),
        }
        answer
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1, L2, E1, D1, S1, P1"]
    fn real_every_survey_header_identifies_its_filter() {
        let band_sources: Vec<String> = (0..LEGACY_PLANES.len()).map(|plane| format!("header BAND{plane}")).collect();
        let mut identified: Vec<(String, Expected<'_>, Option<&str>)> = Vec::new();
        for (band, pivot_um) in [("g", 0.48085), ("r", 0.64176), ("z", 0.91689)] {
            for layer in ["image", "invvar"] {
                identified.push((format!("L2/legacysurvey-0532m280-{layer}-{band}.fits.fz"), ("DECAM", band, pivot_um, "DECam", "header FILTER"), None));
            }
        }
        for (band, pivot_um) in [("g", 0.48085), ("r", 0.64176), ("i", 0.78146)] {
            for extension in 1..=3 {
                identified.push((format!("D1/des-dr2_A_{band}_extn{extension}.fits"), ("DECAM", band, pivot_um, "DECam", "header FILTER"), None));
            }
        }
        for file in [E1_SCI, E1_RMS, E1_FLAG] {
            identified.push((file.to_string(), ("EUCLID", "VIS", 0.71034, "VIS", "header FILTER"), None));
        }
        for (band, pivot_um) in [("g", 0.48491), ("r", 0.62012), ("i", 0.7535)] {
            for layer in ["stack", "wt", "mask"] {
                identified.push((format!("P1/ps1_A_{band}_{layer}_480px.fits"), ("PS1", band, pivot_um, "GPC1", "header HIERARCH FPA.FILTER"), None));
            }
        }
        for (band, pivot_um) in [("g", 0.47025), ("r", 0.61756), ("i", 0.749)] {
            identified.push((format!("S1/frame-{band}-001462-4-0165.fits.bz2"), ("SDSS", band, pivot_um, "SDSS", "header FILTER"), None));
        }
        for hdu in [0, 1] {
            for (plane, (band, pivot_um)) in LEGACY_PLANES.into_iter().enumerate() {
                identified.push((format!("{L1}#hdu={hdu}&plane={plane}"), ("DECAM", band, pivot_um, "DECam", &band_sources[plane]), Some(LEGACY_ASSUMPTION)));
            }
        }

        let mut misses = Vec::new();
        for (relative, expected, warning) in &identified {
            let (dataset, filter, pivot_um, instrument, source) = *expected;
            match real_answer(relative) {
                Ok(found)
                    if (found.dataset.as_str(), found.filter.as_str(), found.instrument.as_str(), found.source.as_str())
                        == (dataset, filter, instrument, source)
                        && (found.pivot_um - pivot_um).abs() < PIVOT_TOLERANCE_UM
                        && found.warning.as_deref() == *warning => {}
                answer => misses.push(format!("{relative}: {answer:?}")),
            }
        }
        for fragment in ["", "#hdu=0", "#hdu=1"] {
            let relative = format!("{L1}{fragment}");
            let answer = real_answer(&relative);
            if answer != Err(L1_CUBE_REFUSAL.to_string()) {
                misses.push(format!("{relative}: {answer:?}"));
            }
        }
        let all_bands_mask = format!("{L1}#hdu=2");
        if let Ok(found) = real_answer(&all_bands_mask) {
            misses.push(format!("{all_bands_mask}: {found:?}"));
        }
        for fragment in ["", "#hdu=2", "#hdu=3"] {
            let relative = format!("{L2_MASKBITS}{fragment}");
            let answer = real_answer(&relative);
            if answer != Err(L2_MASKBITS_REFUSAL.to_string()) {
                misses.push(format!("{relative}: {answer:?}"));
            }
        }
        let beyond = format!("{L1}#hdu=0&plane=4");
        let answer = identify_filter_for_path(&survey_key(&beyond), None);
        println!("{beyond}: {answer:?}");
        if !answer.as_ref().is_err_and(|text| text.contains("plane 4 is out of range")) {
            misses.push(format!("{beyond}: {answer:?}"));
        }
        let through_command = "D1/des-dr2_A_r_extn1.fits";
        let answer = identify_filter_for_path(&survey_key(through_command), None);
        println!("{through_command} through identify_filter_for_path: {answer:?}");
        if !answer.as_ref().is_ok_and(|found| (found.dataset.as_str(), found.filter.as_str()) == ("DECAM", "r")) {
            misses.push(format!("{through_command}: {answer:?}"));
        }
        assert_eq!(identified.len(), 38);
        assert!(misses.is_empty(), "{misses:#?}");
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1, L2, E1, D1, S1, P1"]
    fn real_every_survey_header_has_its_unit_factor_or_none() {
        let mut calibrated: Vec<(String, f64)> = Vec::new();
        for band in ["g", "r", "z"] {
            calibrated.push((format!("L2/legacysurvey-0532m280-image-{band}.fits.fz"), UJY_PER_NANOMAGGY));
        }
        for band in ["g", "r", "i"] {
            calibrated.push((format!("S1/frame-{band}-001462-4-0165.fits.bz2"), UJY_PER_NANOMAGGY));
        }
        calibrated.push((format!("{L1}#hdu=0"), UJY_PER_NANOMAGGY));
        for plane in 0..LEGACY_PLANES.len() {
            calibrated.push((format!("{L1}#hdu=0&plane={plane}"), UJY_PER_NANOMAGGY));
        }
        for band in ["g", "r", "i"] {
            calibrated.push((format!("D1/des-dr2_A_{band}_extn1.fits"), 0.0036307805477010066));
        }
        for file in [E1_SCI, E1_RMS] {
            calibrated.push((file.to_string(), 0.5248074602497712));
        }
        for (band, ujy_per_count) in [("g", 6.031196923091381e-4), ("r", 6.051300912835008e-4), ("i", 2.1232634781877263e-4)] {
            calibrated.push((format!("P1/ps1_A_{band}_stack_480px.fits"), ujy_per_count));
        }

        let mut uncalibrated: Vec<String> = Vec::new();
        for band in ["g", "r", "z"] {
            uncalibrated.push(format!("L2/legacysurvey-0532m280-invvar-{band}.fits.fz"));
        }
        for fragment in ["", "#hdu=2", "#hdu=3"] {
            uncalibrated.push(format!("{L2_MASKBITS}{fragment}"));
        }
        for fragment in ["#hdu=1", "#hdu=2"] {
            uncalibrated.push(format!("{L1}{fragment}"));
        }
        for plane in 0..LEGACY_PLANES.len() {
            uncalibrated.push(format!("{L1}#hdu=1&plane={plane}"));
        }
        for extension in [2, 3] {
            for band in ["g", "r", "i"] {
                uncalibrated.push(format!("D1/des-dr2_A_{band}_extn{extension}.fits"));
            }
        }
        uncalibrated.push(E1_FLAG.to_string());
        for layer in ["wt", "mask"] {
            for band in ["g", "r", "i"] {
                uncalibrated.push(format!("P1/ps1_A_{band}_{layer}_480px.fits"));
            }
        }

        let mut misses = Vec::new();
        for (relative, golden) in &calibrated {
            let header = real_header(relative);
            let Some(cal) = PhotCal::from_header(&header, None) else {
                misses.push(format!("{relative}: no calibration ({})", missing_calibration_reason(Some(&header))));
                continue;
            };
            let measured = cal.jy_per_native_unit.map(|jy| jy * MICROJANSKY_PER_JANSKY);
            println!("{relative}: {measured:?} uJy per unit (golden {golden:e}), origin {}, warnings {:?}", cal.origin, cal.warnings);
            let within = measured.is_some_and(|ujy| ((ujy - golden) / golden).abs() < REAL_FACTOR_TOLERANCE);
            let assumed_when_legacy_cutout = !relative.starts_with(L1) || !cal.warnings.is_empty();
            if !within || cal.origin.is_empty() || !assumed_when_legacy_cutout {
                misses.push(format!("{relative}: {measured:?} uJy per unit, origin {:?}, warnings {:?}", cal.origin, cal.warnings));
            }
        }
        for relative in &uncalibrated {
            let header = real_header(relative);
            let cal = PhotCal::from_header(&header, None);
            println!("{relative}: no factor ({})", missing_calibration_reason(Some(&header)));
            if let Some(cal) = cal {
                misses.push(format!("{relative}: {:?} Jy per unit, origin {}", cal.jy_per_native_unit, cal.origin));
            }
        }
        assert_eq!((calibrated.len(), uncalibrated.len()), (19, 25));
        assert!(misses.is_empty(), "{misses:#?}");
    }
}
