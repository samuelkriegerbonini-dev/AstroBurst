use std::time::Instant;

use anyhow::bail;
use serde_json::{json, Value};

use crate::cmd::analysis::resolve_dq_mask;
use crate::cmd::common::{blocking_cmd, load_cached_full};
use crate::core::imaging::contour::{
    contour_lines, ContourConfig, LevelMode, LevelSpec, MAX_BIN, MAX_CONTOUR_LEVELS,
    MAX_SMOOTH_SIGMA,
};
use crate::types::constants::{RES_ELAPSED_MS, RES_MASKED};

const DEFAULT_SIGMA_MULTIPLES: [f64; 5] = [1.0, 2.0, 3.0, 5.0, 10.0];
const DEFAULT_MODE: &str = "sigma";
const DEFAULT_BIN: usize = 1;
const DEFAULT_SMOOTH_SIGMA: f64 = 0.0;

fn validate_generated(mode: LevelMode, n: usize, lo: f64, hi: f64) -> anyhow::Result<()> {
    if !(1..=MAX_CONTOUR_LEVELS).contains(&n) {
        bail!("n_levels must be between 1 and {}, got {}", MAX_CONTOUR_LEVELS, n);
    }
    if !lo.is_finite() || !hi.is_finite() {
        bail!("lo and hi must be finite numbers, got {} and {}", lo, hi);
    }
    if lo >= hi {
        bail!("lo must be less than hi, got {} and {}", lo, hi);
    }
    if mode == LevelMode::Log && lo <= 0.0 {
        bail!("lo must be greater than 0 in log mode, got {}", lo);
    }
    if mode == LevelMode::Sqrt && lo < 0.0 {
        bail!("lo must be at least 0 in sqrt mode, got {}", lo);
    }
    Ok(())
}

fn validate_values(name: &str, values: &[f64]) -> anyhow::Result<()> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
        bail!("{} must be a non-empty list of finite numbers, got {:?}", name, values);
    }
    if values.len() > MAX_CONTOUR_LEVELS {
        bail!("{} must have at most {} values, got {}", name, MAX_CONTOUR_LEVELS, values.len());
    }
    Ok(())
}

#[tauri::command]
pub async fn contour_lines_cmd(
    path: String,
    mode: Option<String>,
    levels: Option<Vec<f64>>,
    n_levels: Option<usize>,
    lo: Option<f64>,
    hi: Option<f64>,
    sigma_multiples: Option<Vec<f64>>,
    smooth_sigma: Option<f64>,
    bin: Option<usize>,
    exclude_dq: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let mode = LevelMode::parse(mode.as_deref().unwrap_or(DEFAULT_MODE)).map_err(anyhow::Error::msg)?;
        let list = levels.unwrap_or_default();
        let n = n_levels.unwrap_or(0);
        let lo = lo.unwrap_or(f64::NAN);
        let hi = hi.unwrap_or(f64::NAN);
        let multiples = sigma_multiples.unwrap_or_else(|| DEFAULT_SIGMA_MULTIPLES.to_vec());
        let smooth = smooth_sigma.unwrap_or(DEFAULT_SMOOTH_SIGMA);
        let bin = bin.unwrap_or(DEFAULT_BIN);
        match mode {
            LevelMode::List => validate_values("levels", &list)?,
            LevelMode::Sigma => validate_values("sigma_multiples", &multiples)?,
            LevelMode::Linear | LevelMode::Log | LevelMode::Sqrt => validate_generated(mode, n, lo, hi)?,
        }
        if !smooth.is_finite() || !(0.0..=MAX_SMOOTH_SIGMA).contains(&smooth) {
            bail!("smooth_sigma must be between 0 and {} pixels, got {}", MAX_SMOOTH_SIGMA, smooth);
        }
        if !(1..=MAX_BIN).contains(&bin) {
            bail!("bin must be between 1 and {}, got {}", MAX_BIN, bin);
        }
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let spec = LevelSpec { mode, list: &list, n, lo, hi, sigma_multiples: &multiples };
        let cfg = ContourConfig { spec, smooth_sigma_px: smooth, bin };
        let set = contour_lines(entry.arr(), &cfg, mask.as_ref().map(|m| &m.map)).map_err(anyhow::Error::msg)?;
        let mut value = serde_json::to_value(&set)?;
        if let Value::Object(map) = &mut value {
            map.insert(RES_MASKED.to_string(), json!(mask.is_some()));
            map.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
        }
        Ok(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::{
        RES_BACKGROUND_MEDIAN, RES_CONTOUR_BIN, RES_CONTOUR_CLOSED, RES_CONTOUR_LEVELS,
        RES_CONTOUR_POLYLINES, RES_N_POINTS, RES_NOTES, RES_VALUE,
    };
    use ndarray::Array2;

    fn gaussian_fits(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("gauss.fits");
        let centre = 63.5f64;
        let data = Array2::from_shape_fn((128, 128), |(y, x)| {
            let dx = x as f64 - centre;
            let dy = y as f64 - centre;
            (100.0 + 1000.0 * (-(dx * dx + dy * dy) / 200.0).exp()) as f32
        });
        write_fits_mono(path.to_str().unwrap(), &data, None).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn the_command_refuses_too_many_levels_and_an_oversized_bin_with_sentences() {
        let dir = tempfile::tempdir().unwrap();
        let key = gaussian_fits(&dir);
        let long: Vec<f64> = (0..33).map(|i| i as f64).collect();
        let err = contour_lines_cmd(key.clone(), Some("list".into()), Some(long), None, None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("levels must have at most 32 values, got 33"), "{err}");
        let err = contour_lines_cmd(key.clone(), Some("linear".into()), None, Some(33), Some(0.0), Some(1.0), None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("n_levels must be between 1 and 32, got 33"), "{err}");
        let err = contour_lines_cmd(key.clone(), Some("list".into()), Some(vec![600.0]), None, None, None, None, None, Some(9), None)
            .await
            .unwrap_err();
        assert!(err.contains("bin must be between 1 and 8, got 9"), "{err}");
        let err = contour_lines_cmd(key.clone(), Some("log".into()), None, Some(3), Some(0.0), Some(1.0), None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("lo must be greater than 0 in log mode"), "{err}");
        let err = contour_lines_cmd(key.clone(), Some("linear".into()), None, Some(3), Some(5.0), Some(1.0), None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("lo must be less than hi"), "{err}");
        let err = contour_lines_cmd(key.clone(), Some("list".into()), Some(vec![600.0]), None, None, None, None, Some(25.0), None, None)
            .await
            .unwrap_err();
        assert!(err.contains("smooth_sigma must be between 0 and 20"), "{err}");
        let err = contour_lines_cmd(key, Some("cubic".into()), None, None, None, None, None, None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("mode must be one of list, linear, log, sqrt or sigma"), "{err}");
    }

    #[tokio::test]
    async fn the_command_returns_the_serialised_contour_set_with_masked_and_timing() {
        let dir = tempfile::tempdir().unwrap();
        let key = gaussian_fits(&dir);
        let out = contour_lines_cmd(key, Some("list".into()), Some(vec![600.0]), None, None, None, None, Some(0.0), Some(1), Some(false))
            .await
            .unwrap();
        assert_eq!(out[RES_CONTOUR_BIN], 1);
        assert_eq!(out[RES_MASKED], false);
        assert!(out[RES_ELAPSED_MS].is_number());
        assert!(out[RES_NOTES].as_array().unwrap().is_empty());
        assert!(out[RES_BACKGROUND_MEDIAN].is_number());
        let level = &out[RES_CONTOUR_LEVELS][0];
        assert_eq!(level[RES_VALUE], 600.0);
        assert_eq!(level[RES_CONTOUR_CLOSED][0], true);
        let line = level[RES_CONTOUR_POLYLINES][0].as_array().unwrap();
        assert_eq!(line.len(), level[RES_N_POINTS].as_u64().unwrap() as usize);
        assert_eq!(out[RES_N_POINTS], level[RES_N_POINTS]);
        assert_eq!(line[0].as_array().unwrap().len(), 2);
    }
}
