use std::time::Instant;

use serde_json::{json, Value};

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk};
use crate::core::imaging::stats::compute_image_stats;
use crate::types::constants::{RES_CHANNEL_LEVELS, RES_ELAPSED_MS, RES_MAD, RES_MEDIAN, RES_PATH, RES_SIGMA, RES_VALID_COUNT};

#[tauri::command]
pub async fn measure_channel_levels_cmd(paths: Vec<String>) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        if paths.is_empty() {
            anyhow::bail!("No channel paths provided");
        }
        let levels = paths
            .iter()
            .map(|p| {
                let entry = load_from_cache_or_disk(p)?;
                let stats = compute_image_stats(entry.arr());
                Ok(json!({
                    RES_PATH: p,
                    RES_MEDIAN: stats.median,
                    RES_MAD: stats.mad,
                    RES_SIGMA: stats.sigma,
                    RES_VALID_COUNT: stats.valid_count,
                }))
            })
            .collect::<anyhow::Result<Vec<Value>>>()?;
        Ok(json!({
            RES_CHANNEL_LEVELS: levels,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::writer::write_fits_mono;
    use ndarray::Array2;

    #[tokio::test]
    async fn measure_channel_levels_reports_median_mad_on_valid_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("padded.fits").to_str().unwrap().to_string();
        let padded = Array2::from_shape_fn((8, 8), |(r, c)| {
            if r == 0 || r == 7 {
                f32::NAN
            } else if c == 0 || c == 7 {
                0.0
            } else {
                ((r - 1) * 6 + (c - 1)) as f32 + 1.0
            }
        });
        let interior = padded.slice(ndarray::s![1..7, 1..7]).to_owned();
        let expected = compute_image_stats(&interior);
        write_fits_mono(&path, &padded, None).unwrap();

        let res = measure_channel_levels_cmd(vec![path.clone()]).await.unwrap();

        let levels = res[RES_CHANNEL_LEVELS].as_array().unwrap();
        assert_eq!(levels.len(), 1);
        let level = &levels[0];
        assert_eq!(level[RES_PATH], path);
        assert_eq!(level[RES_VALID_COUNT], 36, "{res}");
        assert_eq!(level[RES_MEDIAN].as_f64(), Some(expected.median), "{res}");
        assert_eq!(level[RES_MAD].as_f64(), Some(expected.mad), "{res}");
        assert_eq!(level[RES_SIGMA].as_f64(), Some(expected.sigma), "{res}");
        assert!(expected.median > 1.0 && expected.mad > 0.0, "the fixture must have signal: {expected:?}");
        assert!(res[RES_ELAPSED_MS].is_u64());
        assert!(measure_channel_levels_cmd(vec![]).await.is_err());
    }
}
