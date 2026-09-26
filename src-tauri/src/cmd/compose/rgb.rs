use std::time::{Duration, Instant, SystemTime};

use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached, load_from_cache_or_disk, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::compose::lrgb::lrgb_combine_normalized;
use crate::core::imaging::resample::resample_image;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig, StfParams, apply_stf_f32};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::scnr::apply_scnr_inplace;
use crate::infra::cache::ImageEntry;
use crate::types::constants::{RES_DIMENSIONS, RES_ELAPSED_MS, RES_PNG_PATH, LRGB_APPLIED, RES_CHANNEL, RES_UPDATED};

const COMPOSITE_PNG_PREFIX: &str = "rgb_composite";
const COMPOSITE_PNG_GRACE: Duration = Duration::from_secs(600);

fn is_stale_composite_png(entry: &std::fs::DirEntry, now: SystemTime) -> bool {
    let name = entry.file_name();
    let name = name.to_string_lossy();
    if !name.starts_with(COMPOSITE_PNG_PREFIX) || !name.ends_with(".png") {
        return false;
    }
    entry
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age > COMPOSITE_PNG_GRACE)
}

pub(super) fn composite_png_path(output_dir: &str) -> String {
    let now = SystemTime::now();
    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            if is_stale_composite_png(&entry, now) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let ts = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}/{}_{}.png", output_dir, COMPOSITE_PNG_PREFIX, ts)
}

pub(crate) fn stf_is_linked(linked: Option<bool>, stfs: [&StfParams; 3]) -> bool {
    linked.unwrap_or_else(|| {
        let [r, g, b] = stfs;
        let same = |a: &StfParams, o: &StfParams| {
            a.shadow == o.shadow && a.midtone == o.midtone && a.highlight == o.highlight
        };
        same(r, g) && same(g, b)
    })
}

pub(super) fn load_entry(path: &Option<String>) -> anyhow::Result<Option<ImageEntry>> {
    match path {
        Some(p) => Ok(Some(load_cached(p)?)),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn lrgb_combine_composite_cmd(
    l_path: String,
    output_dir: String,
    lightness: Option<f64>,
    chrominance: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()?;

        let l_entry = load_from_cache_or_disk(&l_path)?;
        let (rows, cols) = er.arr().dim();
        let l_data = l_entry.arr();
        let l_matched = if l_data.dim() != (rows, cols) {
            resample_image(l_data, rows, cols)?
        } else {
            l_data.to_owned()
        };

        let lightness_w = (lightness.unwrap_or(1.0) as f32).clamp(0.0, 1.0);
        let chrominance_w = (chrominance.unwrap_or(1.0) as f32).clamp(0.0, 1.0);

        let (r, g, b) = lrgb_combine_normalized(
            &l_matched,
            er.arr(),
            eg.arr(),
            eb.arr(),
            lightness_w,
            chrominance_w,
        )?;

        let stats_r = compute_image_stats(&r);
        let stats_g = compute_image_stats(&g);
        let stats_b = compute_image_stats(&b);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_lrgb_{}.png", output_dir, ts);

        let stf_config = AutoStfConfig::default();
        let (linked_stf, combined_stats) =
            helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_g = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_b = make_stf_u8_fn(&linked_stf, &combined_stats);
        helpers::render_rgb_preview_with_stf(&r, &g, &b, fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;

        helpers::insert_composite_content(r, g, b, stats_r, stats_g, stats_b);

        Ok(json!({
            RES_PNG_PATH: png_path,
            LRGB_APPLIED: true,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn restretch_composite_cmd(
    output_dir: String,
    shadow_r: f64, midtone_r: f64, highlight_r: f64,
    shadow_g: f64, midtone_g: f64, highlight_g: f64,
    shadow_b: f64, midtone_b: f64, highlight_b: f64,
    scnr_enabled: Option<bool>,
    scnr_method: Option<String>,
    scnr_amount: Option<f64>,
    cache_result: Option<bool>,
    linked: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        resolve_output_dir(&output_dir)?;

        let (entry_r, entry_g, entry_b) = helpers::load_composite_rgb()?;

        let stf_r = StfParams { shadow: shadow_r, midtone: midtone_r, highlight: highlight_r };
        let stf_g = StfParams { shadow: shadow_g, midtone: midtone_g, highlight: highlight_g };
        let stf_b = StfParams { shadow: shadow_b, midtone: midtone_b, highlight: highlight_b };

        let linked_stats = if stf_is_linked(linked, [&stf_r, &stf_g, &stf_b]) {
            Some(crate::core::imaging::stats::combine_channel_stats(
                entry_r.stats(),
                entry_g.stats(),
                entry_b.stats(),
            ))
        } else {
            None
        };

        let (mut r_stretched, mut g_stretched, mut b_stretched) = match &linked_stats {
            Some(combined) => (
                apply_stf_f32(entry_r.arr(), &stf_r, combined),
                apply_stf_f32(entry_g.arr(), &stf_g, combined),
                apply_stf_f32(entry_b.arr(), &stf_b, combined),
            ),
            None => (
                apply_stf_f32(entry_r.arr(), &stf_r, entry_r.stats()),
                apply_stf_f32(entry_g.arr(), &stf_g, entry_g.stats()),
                apply_stf_f32(entry_b.arr(), &stf_b, entry_b.stats()),
            ),
        };

        if let Some(cfg) = helpers::parse_scnr_config(scnr_enabled, scnr_method.as_deref(), scnr_amount, None) {
            apply_scnr_inplace(&mut r_stretched, &mut g_stretched, &mut b_stretched, &cfg);
        }

        let png_path = composite_png_path(&output_dir);
        helpers::render_rgb_preview(&r_stretched, &g_stretched, &b_stretched, &png_path, MAX_PREVIEW_DIM)?;
        if cache_result.unwrap_or(false) {
            helpers::insert_composite_stretched(r_stretched, g_stretched, b_stretched);
        }

        Ok(json!({ RES_PNG_PATH: png_path, RES_ELAPSED_MS: t0.elapsed().as_millis() as u64 }))
    })
}

#[tauri::command]
pub async fn clear_composite_cache_cmd() -> Result<(), String> {
    helpers::clear_composite();
    Ok(())
}

fn composite_channel_index(channel: &str) -> anyhow::Result<usize> {
    match channel.to_lowercase().as_str() {
        "r" => Ok(0),
        "g" => Ok(1),
        "b" => Ok(2),
        _ => anyhow::bail!("Invalid channel: {}. Must be r, g, or b.", channel),
    }
}

#[tauri::command]
pub async fn update_composite_channel_cmd(
    channel: String,
    path: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let index = composite_channel_index(&channel)?;

        let (er, _, _) = helpers::load_composite_rgb()?;
        let target_dim = er.arr().dim();

        let entry = load_from_cache_or_disk(&path)?;
        let dim = entry.arr().dim();
        if dim != target_dim {
            anyhow::bail!(
                "{} is {}x{} but the composite is {}x{}: the processed file is not aligned and cropped like the composite channels. Re-run Blend (with Align and Crop) to include it.",
                path,
                dim.1,
                dim.0,
                target_dim.1,
                target_dim.0
            );
        }

        helpers::replace_composite_channel(index, entry.data_arc(), entry.stats().clone())?;

        Ok(json!({ RES_CHANNEL: channel, RES_UPDATED: true }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::stats::combine_channel_stats;
    use ndarray::Array2;

    fn stf(shadow: f64, midtone: f64) -> StfParams {
        StfParams { shadow, midtone, highlight: 1.0 }
    }

    #[test]
    fn an_explicit_linked_flag_wins_over_the_value_heuristic() {
        let same = stf(0.1, 0.3);
        let other = stf(0.2, 0.3);
        assert!(!stf_is_linked(Some(false), [&same, &same, &same]));
        assert!(stf_is_linked(Some(true), [&same, &other, &same]));
        assert!(stf_is_linked(None, [&same, &same, &same]));
        assert!(!stf_is_linked(None, [&same, &other, &same]));
    }

    #[tokio::test]
    async fn per_channel_restretch_with_equal_values_normalises_each_channel_on_its_own() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let r = Array2::from_shape_fn((8, 8), |(y, x)| 10.0 + (y * 8 + x) as f32);
        let g = r.mapv(|v| v * 4.0);
        let b = r.mapv(|v| v * 0.25);
        let (sr, sg, sb) = (compute_image_stats(&r), compute_image_stats(&g), compute_image_stats(&b));
        helpers::insert_composite_and_orig(r.clone(), g.clone(), b.clone(), sr.clone(), sg.clone(), sb.clone());
        let p = stf(0.0, 0.5);

        let restretch = |linked: Option<bool>| {
            restretch_composite_cmd(
                out.clone(), 0.0, 0.5, 1.0, 0.0, 0.5, 1.0, 0.0, 0.5, 1.0, None, None, None, Some(true), linked,
            )
        };
        restretch(Some(false)).await.unwrap();
        let (cr, cg, _) = helpers::load_composite_stretched().expect("stretched cache");
        assert_eq!(cr.arr(), &apply_stf_f32(&r, &p, &sr));
        assert_eq!(cg.arr(), &apply_stf_f32(&g, &p, &sg), "the per-channel choice was overridden by equal values");

        restretch(None).await.unwrap();
        let combined = combine_channel_stats(&sr, &sg, &sb);
        let (_, cg, _) = helpers::load_composite_stretched().expect("stretched cache");
        assert_eq!(cg.arr(), &apply_stf_f32(&g, &p, &combined), "without a flag, equal values still mean linked");
    }

    #[tokio::test]
    async fn every_command_on_a_cleared_composite_says_to_run_blend_again() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        helpers::clear_composite();
        let expected = "The colour composite is no longer in memory; run Blend again.";

        let lrgb = lrgb_combine_composite_cmd(out.clone(), out.clone(), None, None)
            .await
            .expect_err("LRGB on a cleared composite must fail");
        assert_eq!(lrgb, expected);

        let restretch = restretch_composite_cmd(
            out.clone(), 0.0, 0.5, 1.0, 0.0, 0.5, 1.0, 0.0, 0.5, 1.0, None, None, None, None, None,
        )
        .await
        .expect_err("restretch on a cleared composite must fail");
        assert_eq!(restretch, expected);

        let update = update_composite_channel_cmd("r".to_string(), out)
            .await
            .expect_err("a channel update on a cleared composite must fail");
        assert_eq!(update, expected);
    }

    #[test]
    fn a_new_composite_png_does_not_delete_one_that_is_still_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap();
        let fresh = dir.path().join("rgb_composite_1.png");
        let stale = dir.path().join("rgb_composite_0.png");
        let other = dir.path().join("keep_me.png");
        for p in [&fresh, &stale, &other] {
            std::fs::write(p, b"png").unwrap();
        }
        let old = SystemTime::now() - COMPOSITE_PNG_GRACE - Duration::from_secs(60);
        std::fs::OpenOptions::new().write(true).open(&stale).unwrap().set_modified(old).unwrap();

        let next = composite_png_path(out);
        assert!(next.starts_with(out) && next.contains("rgb_composite_"));
        assert!(fresh.exists(), "a composite PNG another command just returned was deleted");
        assert!(!stale.exists(), "old composite PNGs are no longer cleaned up");
        assert!(other.exists());
    }
}
