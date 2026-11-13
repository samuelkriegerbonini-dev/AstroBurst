use serde::Serialize;

use crate::cmd::common::blocking_cmd;
use crate::cmd::tso::EMPTY_PATHS_ERROR;
use crate::core::analysis::tso::cache::set_id;
use crate::core::analysis::tso::transit_spectrum::{transit_spectrum, SpectrumFitRequest, TransitSpectrum};
use crate::core::analysis::tso::white_light::{light_curves, CurveRequest};
use crate::core::analysis::tso::{find_series, open_tso, series_label, SeriesKey};

#[derive(Serialize)]
struct SeriesTransitSpectrum {
    set_id: String,
    label: String,
    #[serde(flatten)]
    spectrum: TransitSpectrum,
}

pub(crate) fn tso_transit_spectrum_json(
    paths: &[String],
    series: &SeriesKey,
    request: &CurveRequest,
    fit: &SpectrumFitRequest,
) -> anyhow::Result<serde_json::Value> {
    let set = open_tso(paths)?;
    let found = find_series(&set, series)?;
    let curves = light_curves(&set, found, request)?;
    let keys: Vec<SeriesKey> = set.series.iter().map(|one| one.key.clone()).collect();
    Ok(serde_json::to_value(SeriesTransitSpectrum {
        set_id: set_id(&set),
        label: series_label(series, &keys),
        spectrum: transit_spectrum(&curves, fit)?,
    })?)
}

#[tauri::command]
pub async fn tso_transit_spectrum_cmd(
    paths: Vec<String>,
    series: SeriesKey,
    request: CurveRequest,
    fit: SpectrumFitRequest,
) -> Result<serde_json::Value, String> {
    if paths.is_empty() || paths.iter().any(|path| path.trim().is_empty()) {
        return Err(EMPTY_PATHS_ERROR.to_string());
    }
    blocking_cmd!(tso_transit_spectrum_json(&paths, &series, &request, &fit))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Instant;

    use serde_json::{json, Value};

    use super::*;
    use crate::cmd::tso::{tso_open_cmd, EMPTY_PATHS_ERROR};
    use crate::core::analysis::transit_model::TRANSIT_MODEL_NAME;
    use crate::core::analysis::tso::transit_spectrum::test_support::{
        inj, injected_flux, numbers, req, INJECTED_SCALES, INJECTED_SIGMA,
    };
    use crate::core::analysis::tso::transit_spectrum::LimbChoice;
    use crate::core::analysis::tso::white_light::CurveMode;
    use crate::core::analysis::tso::DATAMODEL_ROWS;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::x1dints::test_fixtures::{rows_hdu, series_primary, write_series_file, SynthRow, SynthTable};

    const SPECTRUM_KEYS: [&str; 13] = [
        "bins",
        "fit",
        "label",
        "mode",
        "model",
        "n_excluded",
        "n_in_transit",
        "n_integrations",
        "notes",
        "series",
        "set_id",
        "time_scale",
        "warnings",
    ];
    const FIT_KEYS: [&str; 11] = [
        "a_rs",
        "b",
        "baseline_order",
        "exclude_rows",
        "limb_darkening",
        "period_days",
        "rp_start",
        "scale_errors",
        "t0",
        "u1",
        "u2",
    ];
    const BIN_KEYS: [&str; 21] = [
        "band_um",
        "chi2",
        "depth",
        "depth_err",
        "dof",
        "error",
        "grazing",
        "n_in_transit",
        "n_points",
        "notes",
        "npixels_ratio",
        "pixel_loss",
        "rms",
        "rp",
        "rp_err",
        "u1",
        "u1_err",
        "u2",
        "u2_err",
        "warnings",
        "weighted",
    ];

    fn sorted_keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    fn texts(value: &Value) -> Vec<&str> {
        value.as_array().unwrap().iter().map(|text| text.as_str().unwrap()).collect()
    }

    fn injected_file(dir: &Path) -> String {
        let fixture = inj();
        let time = numbers(&fixture["time"]);
        let flux: Vec<Vec<f64>> = (0..3).map(|bin| injected_flux(&fixture, bin, true)).collect();
        let rows = (0..time.len())
            .map(|row| SynthRow {
                int_num: row as i32 + 1,
                n_alongdisp: 3,
                segment: 1,
                mjd_avg: time[row] - 0.002,
                tdb_mid: time[row],
                wavelength: vec![1.0, 2.0, 3.0],
                flux: flux.iter().map(|bin| bin[row]).collect(),
                flux_error: INJECTED_SCALES.iter().map(|scale| scale * INJECTED_SIGMA).collect(),
                dq: vec![0; 3],
                npixels: vec![7.0; 3],
            })
            .collect();
        let table = SynthTable { extver: 1, detector: Some("NRS1"), sporder: Some(0), flux_unit: Some("Jy"), rows };
        let cards = [
            ("NINTS", "120".to_string()),
            ("EXPSTART", "60000.0".to_string()),
            ("EXP_TYPE", "'NRS_BRIGHTOBJ'".to_string()),
            ("DETECTOR", "'NRS1'".to_string()),
        ];
        let primary = series_primary(DATAMODEL_ROWS, &cards);
        write_series_file(dir, "inj_x1dints.fits", primary, vec![rows_hdu(&table)]).to_str().unwrap().to_string()
    }

    fn nrs1() -> SeriesKey {
        SeriesKey { detector: "NRS1".into(), sporder: Some(0) }
    }

    fn pipeline(bins: Vec<[f64; 2]>) -> CurveRequest {
        CurveRequest {
            mode: CurveMode::Pipeline,
            wavelength_min_um: None,
            wavelength_max_um: None,
            bins,
            exclude_um: Vec::new(),
            reject_elements_sigma: None,
        }
    }

    fn one_channel_per_bin() -> CurveRequest {
        pipeline(vec![[0.5, 1.5], [1.5, 2.5], [2.5, 3.5]])
    }

    #[tokio::test]
    async fn tso_transit_spectrum_cmd_returns_one_row_per_bin_with_the_request_echoed() {
        let dir = tempfile::tempdir().unwrap();
        let path = injected_file(dir.path());
        let spectrum = tso_transit_spectrum_cmd(vec![path.clone()], nrs1(), one_channel_per_bin(), req()).await.unwrap();
        assert_eq!(sorted_keys(&spectrum), SPECTRUM_KEYS);
        assert_eq!(spectrum["label"], "NRS1");
        let opened = tso_open_cmd(vec![path]).await.unwrap();
        assert_eq!(spectrum["set_id"], opened["set_id"]);
        assert_eq!(spectrum["series"], json!({"detector": "NRS1", "sporder": 0}));
        assert_eq!(spectrum["mode"], "pipeline");
        assert_eq!(spectrum["time_scale"], "bmjd_tdb");
        assert_eq!(spectrum["model"], TRANSIT_MODEL_NAME);
        assert_eq!(sorted_keys(&spectrum["fit"]), FIT_KEYS);
        assert_eq!(spectrum["fit"], serde_json::to_value(req()).unwrap());

        let fixture = inj();
        let bins = spectrum["bins"].as_array().unwrap();
        assert_eq!(bins.len(), 3);
        for (k, bin) in bins.iter().enumerate() {
            assert_eq!(sorted_keys(bin), BIN_KEYS, "bin {k}");
            let expected = fixture["fit_noisy"][k]["depth"].as_f64().unwrap();
            let depth = bin["depth"].as_f64().unwrap();
            assert!(((depth - expected) / expected).abs() <= 1e-5, "depth of bin {k}: {depth} against {expected}");
            assert_eq!(bin["n_points"], 120, "bin {k}");
            assert_eq!(bin["dof"], 117, "bin {k}");
        }
        assert_eq!(bins[0]["band_um"], json!([0.5, 1.5]));
        assert_eq!(bins[2]["band_um"], json!([2.5, 3.5]));
        assert!(bins[0]["u1_err"].is_null(), "{}", bins[0]);
        assert!(bins[0]["error"].is_null(), "{}", bins[0]);
        assert!(bins[0]["warnings"].is_array(), "{}", bins[0]);
        assert!(bins[0]["notes"].is_array(), "{}", bins[0]);
        assert_eq!(bins[0]["u1"], 0.3);
        assert_eq!(bins[0]["weighted"], true);
        assert_eq!(bins[0]["npixels_ratio"], 1.0);
        assert_eq!(spectrum["n_integrations"], 120);
        assert_eq!(spectrum["n_excluded"], 0);
        assert_eq!(spectrum["n_in_transit"], 51);
        assert_eq!(spectrum["warnings"], json!([]));
        assert_eq!(
            spectrum["notes"][1],
            "fixed in every bin: period 3 d, t0 60000.15 (BMJD (TDB)), a/Rs 8, b 0.3, u1 0.3, u2 0.2"
        );
    }

    #[tokio::test]
    async fn the_command_refuses_empty_lists_and_passes_refusals_through() {
        let nothing = tso_transit_spectrum_cmd(vec![], nrs1(), one_channel_per_bin(), req()).await.unwrap_err();
        assert_eq!(nothing, EMPTY_PATHS_ERROR);
        let blank = tso_transit_spectrum_cmd(vec![" ".into()], nrs1(), one_channel_per_bin(), req()).await.unwrap_err();
        assert_eq!(blank, EMPTY_PATHS_ERROR);

        let dir = tempfile::tempdir().unwrap();
        let path = injected_file(dir.path());
        let another = SeriesKey { detector: "NRS2".into(), sporder: Some(0) };
        let absent = tso_transit_spectrum_cmd(vec![path.clone()], another, one_channel_per_bin(), req()).await.unwrap_err();
        assert!(absent.contains("no series NRS2"), "{absent}");

        let unbinned = tso_transit_spectrum_cmd(vec![path.clone()], nrs1(), pipeline(Vec::new()), req()).await.unwrap_err();
        assert!(unbinned.contains("add wavelength bins: the fitted spectrum has one point per bin"), "{unbinned}");

        let rejecting = CurveRequest { reject_elements_sigma: Some(5.0), ..one_channel_per_bin() };
        let refused = tso_transit_spectrum_cmd(vec![path.clone()], nrs1(), rejecting, req()).await.unwrap_err();
        assert!(refused.contains("apply to the masked mode only"), "{refused}");

        let backwards = SpectrumFitRequest { period_days: -1.0, ..req() };
        let unfitted = tso_transit_spectrum_cmd(vec![path], nrs1(), one_channel_per_bin(), backwards).await.unwrap_err();
        assert!(unfitted.contains("the period must be a number of days above 0, got -1"), "{unfitted}");
    }

    const E1: &str = "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits";
    const E1_ORACLE: &str = "plan/pacotes/TSO-support/tso_transit_oracle.py e1";
    const PIN_NOT_PLANET: &str = "a pin of the code on this file, not a measurement of the planet: the level 3 extraction lost pixels during the event";
    const E1_PIXEL_LOSS: &str = "mean NPIXELS inside the transit is below 0.9 of the baseline in 5 of 5 bins (lowest ratio 0.637): the extraction lost pixels during the event, so these fitted depths are biased";
    const E1_GRAZING: &str = "grazing transit (b + Rp/Rs above 1) in 5 of 5 fitted bins: (Rp/Rs)^2 is then not the fraction of light blocked and is not expected to equal the quick-look depth";

    struct PinnedBin {
        band_um: [f64; 2],
        rp: f64,
        depth: f64,
        depth_err_covariance_alone: f64,
        depth_err_scaled: f64,
        chi2: f64,
        npixels_ratio: f64,
    }

    const E1_BINS: [PinnedBin; 5] = [
        PinnedBin {
            band_um: [0.6, 1.0],
            rp: 7.699960796108718,
            depth: 59.28939626161121,
            depth_err_covariance_alone: 0.007033648474061715,
            depth_err_scaled: 0.3126939701183128,
            chi2: 468409.872724232,
            npixels_ratio: 0.6396400343186672,
        },
        PinnedBin {
            band_um: [1.0, 2.0],
            rp: 7.9615132223691125,
            depth: 63.38569278995821,
            depth_err_covariance_alone: 0.003182233700074425,
            depth_err_scaled: 0.2889656745561202,
            chi2: 1954234.6936146035,
            npixels_ratio: 0.637321663442749,
        },
        PinnedBin {
            band_um: [2.0, 3.0],
            rp: 7.567332917887036,
            depth: 57.26452749013672,
            depth_err_covariance_alone: 0.008504208394856369,
            depth_err_scaled: 0.20831875715814466,
            chi2: 142212.34893455153,
            npixels_ratio: 0.6659101784796412,
        },
        PinnedBin {
            band_um: [3.0, 4.0],
            rp: 8.021313707756546,
            depth: 64.34147359824307,
            depth_err_covariance_alone: 0.00478733012836223,
            depth_err_scaled: 0.2145457514797924,
            chi2: 475993.94688573846,
            npixels_ratio: 0.691274543080914,
        },
        PinnedBin {
            band_um: [4.0, 5.3],
            rp: 7.723964099987176,
            depth: 59.65962141789071,
            depth_err_covariance_alone: 0.008471999509501178,
            depth_err_scaled: 0.19240731860631421,
            chi2: 122241.78324212437,
            npixels_ratio: 0.8240607177194025,
        },
    ];

    fn assert_pinned(bin: &Value, field: &str, expected: f64, tolerance: f64, what: &str) {
        let measured = bin[field].as_f64().unwrap_or(f64::NAN);
        let relative = ((measured - expected) / expected).abs();
        println!("{what}, {field}: measured {measured:?}, expected {expected:?}, relative difference {relative:.1e}");
        assert!(
            relative <= tolerance,
            "{what}, {field}: measured {measured:?}, expected {expected:?}, relative difference {relative:e} ({PIN_NOT_PLANET}; {E1_ORACLE})"
        );
    }

    #[tokio::test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: tso/E1"]
    async fn real_fitted_spectrum_of_the_transit_file_matches_the_oracle() {
        let oracle = E1_ORACLE;
        let file = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("tso").join(E1);
        assert!(file.is_file(), "{} is missing", file.display());
        let path = file.to_str().unwrap().to_string();
        let curve_request = pipeline(E1_BINS.iter().map(|pinned| pinned.band_um).collect());
        let typed = SpectrumFitRequest {
            period_days: 1.4079405,
            t0: 60061.507668,
            a_rs: 336.0,
            b: 7.16,
            rp_start: 7.73,
            u1: 0.2,
            u2: 0.25,
            limb_darkening: LimbChoice::Fixed,
            baseline_order: 1,
            exclude_rows: Vec::new(),
            scale_errors: false,
        };
        for scale_errors in [false, true] {
            let fit = SpectrumFitRequest { scale_errors, ..typed.clone() };
            let started = Instant::now();
            let spectrum = tso_transit_spectrum_cmd(vec![path.clone()], nrs1(), curve_request.clone(), fit).await.unwrap();
            let run = if scale_errors { "errors scaled by sqrt(chi2 / dof)" } else { "errors from the covariance alone" };
            println!("tso_transit_spectrum_cmd on E1, five bins, {run}: {:.1?}", started.elapsed());

            assert_eq!(spectrum["set_id"], "4b385da81d5c70b8", "{run}, {oracle}");
            assert_eq!(spectrum["n_integrations"], 240, "{run}, {oracle}");
            assert_eq!(spectrum["n_excluded"], 0, "{run}, {oracle}");
            assert_eq!(spectrum["n_in_transit"], 18, "INT_NUM 177 to 194, {run}, {oracle}");
            let bins = spectrum["bins"].as_array().unwrap();
            assert_eq!(bins.len(), 5, "{run}, {oracle}");
            for (bin, pinned) in bins.iter().zip(&E1_BINS) {
                let what = format!("bin {} to {} um, {run}", pinned.band_um[0], pinned.band_um[1]);
                println!("{what}: warnings {}, notes {}", bin["warnings"], bin["notes"]);
                assert_eq!(bin["band_um"], json!(pinned.band_um), "{what}, {oracle}");
                assert_eq!(bin["n_points"], 240, "{what}, {oracle}");
                assert_eq!(bin["n_in_transit"], 18, "{what}, {oracle}");
                assert_eq!(bin["dof"], 237, "{what}, {oracle}");
                assert_eq!(bin["weighted"], true, "{what}, {oracle}");
                assert!(bin["error"].is_null(), "{what}, {oracle}: {}", bin["error"]);
                assert_eq!(bin["grazing"], true, "{what} ({PIN_NOT_PLANET}), {oracle}");
                assert_eq!(bin["pixel_loss"], true, "{what} ({PIN_NOT_PLANET}), {oracle}");
                assert_eq!(bin["u1"], 0.2, "{what}, {oracle}");
                assert_eq!(bin["u2"], 0.25, "{what}, {oracle}");
                assert_pinned(bin, "depth", pinned.depth, 1e-4, &what);
                assert_pinned(bin, "rp", pinned.rp, 5e-5, &what);
                assert_pinned(bin, "chi2", pinned.chi2, 1e-5, &what);
                assert_pinned(bin, "npixels_ratio", pinned.npixels_ratio, 1e-9, &what);
                let depth_err = if scale_errors { pinned.depth_err_scaled } else { pinned.depth_err_covariance_alone };
                assert_pinned(bin, "depth_err", depth_err, 0.02, &what);

                let warnings = texts(&bin["warnings"]);
                let reduced_chi2 = bin["chi2"].as_f64().unwrap() / bin["dof"].as_f64().unwrap();
                let unscaled = format!(
                    "reduced chi2 = {reduced_chi2:.1}: the model or the errors do not describe the scatter, so the errors of the fit are too small unless they are scaled"
                );
                if scale_errors {
                    assert!(!warnings.iter().any(|warning| warning.starts_with("reduced chi2 = ")), "{what}: {warnings:?}");
                } else {
                    assert!(warnings.contains(&unscaled.as_str()), "{what}: {warnings:?}");
                }
            }
            println!("{run}: notes of the spectrum {}", spectrum["notes"]);
            assert_eq!(texts(&spectrum["warnings"]), [E1_PIXEL_LOSS, E1_GRAZING], "{run} ({PIN_NOT_PLANET}), {oracle}");
            let notes = texts(&spectrum["notes"]);
            assert_eq!(
                notes[1],
                "fixed in every bin: period 1.4079405 d, t0 60061.507668 (BMJD (TDB)), a/Rs 336, b 7.16, u1 0.2, u2 0.25",
                "{run}, {oracle}"
            );
            let error_rule =
                if scale_errors { "multiplied in every bin by sqrt(chi2 / dof) of that bin" } else { "from the covariance alone" };
            assert!(notes[2].ends_with(error_rule), "{run}, {oracle}: {}", notes[2]);
        }
    }
}
