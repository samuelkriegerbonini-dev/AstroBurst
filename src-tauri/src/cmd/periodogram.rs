use std::time::Instant;

use serde_json::json;

use crate::cmd::common::blocking_cmd;
use crate::core::analysis::periodogram::{decimate_peaks, periodogram, PeriodogramOptions};
use crate::types::constants::RES_ELAPSED_MS;

pub const MAX_RETURNED_SAMPLES: usize = 20_000;

pub(crate) fn periodogram_json(
    time: &[f64],
    value: &[f64],
    err: Option<&[f64]>,
    options: &PeriodogramOptions,
) -> anyhow::Result<serde_json::Value> {
    let started = Instant::now();
    let mut search = periodogram(time, value, err, options)?;
    if search.n_frequencies > MAX_RETURNED_SAMPLES {
        let (frequency, power) = decimate_peaks(&search.frequency, &search.power, MAX_RETURNED_SAMPLES);
        search.frequency = frequency;
        search.power = power;
        search.notes.push(format!(
            "the returned curve keeps the highest power of every {} consecutive frequencies ({} searched)",
            search.n_frequencies.div_ceil(MAX_RETURNED_SAMPLES),
            search.n_frequencies
        ));
    }
    let mut result = serde_json::to_value(&search)?;
    result[RES_ELAPSED_MS] = json!(started.elapsed().as_millis() as u64);
    Ok(result)
}

#[tauri::command]
pub async fn light_curve_periodogram_cmd(
    time: Vec<f64>,
    value: Vec<f64>,
    err: Option<Vec<f64>>,
    min_frequency: Option<f64>,
    max_frequency: Option<f64>,
    samples_per_peak: Option<f64>,
) -> Result<serde_json::Value, String> {
    let options = PeriodogramOptions { min_frequency, max_frequency, samples_per_peak };
    blocking_cmd!(periodogram_json(&time, &value, err.as_deref(), &options))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::periodogram::test_series::series_a;

    const RESULT_KEYS: [&str; 20] = [
        "baseline_days",
        "best_frequency",
        "best_period",
        "best_power",
        "cadence_days",
        "elapsed_ms",
        "false_alarm_probability",
        "fap_method",
        "first_frequency",
        "frequency",
        "frequency_step",
        "last_frequency",
        "n_dropped",
        "n_frequencies",
        "n_points",
        "normalization",
        "notes",
        "power",
        "upper_limit",
        "weighted",
    ];

    #[test]
    fn periodogram_json_serialises_the_search_with_the_field_names_the_frontend_reads() {
        let (time, value, err) = series_a();
        let json = periodogram_json(&time, &value, Some(&err), &PeriodogramOptions::default()).unwrap();
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, RESULT_KEYS);
        assert_eq!(json["frequency"].as_array().unwrap().len(), 500);
        assert_eq!(json["power"].as_array().unwrap().len(), 500);
        assert!((json["best_period"].as_f64().unwrap() - 2.895261402175141).abs() < 1e-9, "{}", json["best_period"]);
        assert!(json["false_alarm_probability"].is_number(), "{}", json["false_alarm_probability"]);
        assert!(json["elapsed_ms"].is_u64(), "{}", json["elapsed_ms"]);
        assert_eq!(json["n_frequencies"], 500);
        assert_eq!(json["notes"].as_array().unwrap().len(), 5);
        assert_eq!(json["upper_limit"], "five_mean_nyquist");
        assert!(json["cadence_days"].is_null(), "{}", json["cadence_days"]);
    }

    #[test]
    fn periodogram_json_shortens_a_long_curve_and_keeps_the_true_peak() {
        let (time, value, err) = series_a();
        let options = PeriodogramOptions { max_frequency: Some(1000.0), ..PeriodogramOptions::default() };
        let json = periodogram_json(&time, &value, Some(&err), &options).unwrap();
        let frequency = json["frequency"].as_array().unwrap();
        let power: Vec<f64> = json["power"].as_array().unwrap().iter().map(|p| p.as_f64().unwrap()).collect();
        assert!(frequency.len() <= MAX_RETURNED_SAMPLES, "{}", frequency.len());
        assert_eq!(frequency.len(), power.len());
        assert_eq!(json["n_frequencies"], 72_381);
        let highest_returned = power.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(json["best_power"].as_f64().unwrap(), highest_returned);
        let last_note = json["notes"].as_array().unwrap().last().unwrap().as_str().unwrap();
        assert!(
            last_note.starts_with("the returned curve keeps the highest power of every 4 consecutive frequencies (72381 searched)"),
            "{last_note}"
        );
    }

    #[tokio::test]
    async fn the_command_passes_the_error_text_through() {
        let refusal = light_curve_periodogram_cmd(vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0], None, None, None, None)
            .await
            .unwrap_err();
        assert!(refusal.contains("at least 4 points"), "{refusal}");
    }
}
