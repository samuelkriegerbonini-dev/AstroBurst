use std::time::Instant;

use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::core::analysis::transit_fit::{fit_transit, TransitFitRequest};
use crate::types::constants::RES_ELAPSED_MS;

pub(crate) fn transit_fit_json(request: &TransitFitRequest, include_series: bool) -> anyhow::Result<serde_json::Value> {
    let started = Instant::now();
    let mut fit = fit_transit(request)?;
    if !include_series {
        for series in [
            &mut fit.time,
            &mut fit.model_flux,
            &mut fit.baseline_flux,
            &mut fit.residual,
            &mut fit.curve_time,
            &mut fit.curve_flux,
            &mut fit.curve_baseline,
        ] {
            series.clear();
        }
    }
    let mut result = serde_json::to_value(&fit)?;
    result[RES_ELAPSED_MS] = json!(started.elapsed().as_millis() as u64);
    Ok(result)
}

#[tauri::command]
pub async fn light_curve_transit_fit_cmd(
    request: TransitFitRequest,
    include_series: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(transit_fit_json(&request, include_series.unwrap_or(true)))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{json, Value};

    use super::*;
    use crate::core::analysis::transit_fit::{ParameterInput, TransitKind};

    const RESULT_KEYS: [&str; 32] = [
        "baseline",
        "baseline_flux",
        "chi2",
        "converged",
        "correlation",
        "curve_baseline",
        "curve_flux",
        "curve_time",
        "depth",
        "depth_err",
        "derived",
        "dof",
        "elapsed_ms",
        "error_scale",
        "free_names",
        "iterations",
        "kind",
        "model",
        "model_flux",
        "n_in_transit",
        "n_points",
        "notes",
        "parameters",
        "period",
        "reduced_chi2",
        "residual",
        "rms",
        "rms_ppm",
        "time",
        "time_averaging",
        "warnings",
        "weighted",
    ];

    const SERIES_KEYS: [&str; 7] =
        ["time", "model_flux", "baseline_flux", "residual", "curve_time", "curve_flux", "curve_baseline"];

    fn fixture() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transit/fit_series_a.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn numbers(value: &Value) -> Vec<f64> {
        value.as_array().unwrap().iter().map(|member| member.as_f64().unwrap()).collect()
    }

    fn request_of(name: &str) -> TransitFitRequest {
        let fixture = fixture();
        let fit = &fixture["fits"][name];
        let free_names: Vec<&str> = fit["free"].as_array().unwrap().iter().map(|member| member.as_str().unwrap()).collect();
        let input = |parameter: &str| ParameterInput {
            value: fit["start"][parameter].as_f64().unwrap(),
            free: free_names.contains(&parameter),
        };
        TransitFitRequest {
            time: numbers(&fixture["time"]),
            flux: numbers(&fixture["flux"]),
            err: fit["weighted"].as_bool().unwrap().then(|| numbers(&fixture["err"])),
            period: fixture["period"].as_f64().unwrap(),
            t0: input("t0"),
            rp: input("rp"),
            a_rs: input("a_rs"),
            b: input("b"),
            u1: input("u1"),
            u2: input("u2"),
            baseline_order: fit["baseline_order"].as_u64().unwrap() as u8,
            scale_errors: fit["scale_errors"].as_bool().unwrap(),
            ..Default::default()
        }
    }

    fn weighted_request() -> TransitFitRequest {
        let request = request_of("weighted");
        assert_eq!(request.t0, ParameterInput { value: 2460310.502, free: true });
        assert_eq!(request.rp, ParameterInput { value: 0.09, free: true });
        assert_eq!(request.a_rs, ParameterInput { value: 8.0, free: true });
        assert_eq!(request.b, ParameterInput { value: 0.4, free: true });
        assert_eq!(request.u1, ParameterInput { value: 0.4, free: false });
        assert_eq!(request.u2, ParameterInput { value: 0.25, free: false });
        assert_eq!(request.baseline_order, 1);
        assert!(request.err.is_some());
        request
    }

    fn list_len(result: &Value, key: &str) -> usize {
        result[key].as_array().unwrap_or_else(|| panic!("{key} is not a list in {result}")).len()
    }

    #[test]
    fn transit_fit_json_serialises_the_fit_with_the_field_names_the_frontend_reads() {
        let result = transit_fit_json(&weighted_request(), true).unwrap();
        let mut keys: Vec<&str> = result.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, RESULT_KEYS);
        assert_eq!(result["kind"], "transit");
        let parameters = result["parameters"].as_array().unwrap();
        assert_eq!(parameters.len(), 6);
        assert_eq!(parameters[1]["name"], "rp");
        let rp = parameters[1]["value"].as_f64().unwrap();
        assert!((rp - 0.09982034774988176).abs() < 1e-6, "rp {rp}");
        for key in &SERIES_KEYS[..4] {
            assert_eq!(list_len(&result, key), 240, "{key}");
        }
        for key in &SERIES_KEYS[4..] {
            assert_eq!(list_len(&result, key), 400, "{key}");
        }
        assert!(result["derived"]["inclination_deg"].is_f64(), "{}", result["derived"]);
        assert_eq!(result["time_averaging"]["bin_points"], json!([1, 2, 4, 8, 16, 32]));
        assert!(result["elapsed_ms"].is_u64(), "{}", result["elapsed_ms"]);
        assert_eq!(parameters[4]["name"], "u1");
        assert!(parameters[4]["err"].is_null(), "{}", parameters[4]);
        println!("fits.weighted, 240 points, four free shape parameters: elapsed_ms {}", result["elapsed_ms"]);
    }

    #[test]
    fn without_the_series_the_numbers_stay_and_the_lists_are_empty() {
        let full = transit_fit_json(&weighted_request(), true).unwrap();
        let bare = transit_fit_json(&weighted_request(), false).unwrap();
        for key in SERIES_KEYS {
            assert_eq!(bare[key], json!([]), "{key}");
        }
        for key in ["depth", "depth_err", "chi2", "rms_ppm", "parameters", "derived", "time_averaging", "notes"] {
            assert_eq!(bare[key], full[key], "{key}");
        }
        assert!(bare["depth_err"].is_f64(), "{}", bare["depth_err"]);
    }

    #[tokio::test]
    async fn the_command_passes_the_refusal_through() {
        let request = TransitFitRequest { baseline_order: 3, ..weighted_request() };
        let refusal = light_curve_transit_fit_cmd(request, None).await.unwrap_err();
        assert!(refusal.contains("baseline order 3 must be 0, 1 or 2"), "{refusal}");
    }

    #[test]
    fn a_request_as_the_frontend_sends_it_is_read() {
        let mut sent = json!({
            "kind": "eclipse",
            "time": [0.0, 1.0],
            "flux": [1.0, 1.0],
            "err": null,
            "period": 3.0,
            "t0": {"value": 0.5, "free": true},
            "rp": {"value": 0.1, "free": false},
            "a_rs": {"value": 9.0, "free": false},
            "b": {"value": 0.3, "free": false},
            "u1": {"value": 0.0, "free": false},
            "u2": {"value": 0.0, "free": false},
            "flux_ratio": {"value": 0.001, "free": true},
            "baseline_order": 1,
            "scale_errors": false
        });
        let eclipse: TransitFitRequest = serde_json::from_value(sent.clone()).unwrap();
        assert_eq!(eclipse.kind, TransitKind::Eclipse);
        assert_eq!(eclipse.err, None);
        assert_eq!(eclipse.flux_ratio, Some(ParameterInput { value: 0.001, free: true }));
        assert_eq!(eclipse.baseline_order, 1);
        let object = sent.as_object_mut().unwrap();
        object.remove("kind");
        object.remove("flux_ratio");
        let transit: TransitFitRequest = serde_json::from_value(sent).unwrap();
        assert_eq!(transit.kind, TransitKind::Transit);
        assert_eq!(transit.flux_ratio, None);
    }

    #[test]
    fn a_bin_request_with_the_radius_alone_is_fast() {
        let request = request_of("radius_only");
        let with_series = transit_fit_json(&request, true).unwrap();
        let without_series = transit_fit_json(&request, false).unwrap();
        let depth = with_series["depth"].as_f64().unwrap();
        assert!((depth - 0.009970332165374702).abs() < 1e-6, "depth {depth}");
        assert_eq!(without_series["depth"], with_series["depth"]);
        println!(
            "fits.radius_only: elapsed_ms {} with the series, {} without",
            with_series["elapsed_ms"], without_series["elapsed_ms"]
        );
    }
}
