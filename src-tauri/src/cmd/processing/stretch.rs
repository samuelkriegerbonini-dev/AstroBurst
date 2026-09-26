use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, render_and_save_as, resolve_output_dir, OutputValues, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::imaging::stretch::{arcsinh_stretch, arcsinh_stretch_rgb, ghs_stretch, ghs_stretch_rgb, GhsParams};
use crate::core::imaging::masked_stretch::{masked_stretch, masked_stretch_rgb_shared, MaskedStretchConfig};
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH,
    RES_STRETCH_FACTOR, RES_ITERATIONS_RUN, RES_STARS_MASKED,
    RES_MASK_COVERAGE, RES_FINAL_BACKGROUND, RES_CONVERGED,
    SUFFIX_MASKED_STRETCH,
    RES_LOCAL_INTENSITY, RES_SYMMETRY_POINT, RES_SHADOW_PROTECT, RES_HIGHLIGHT_PROTECT,
    RES_R, RES_G, RES_B, RES_MASK_MODE, CHANNELS,
    MASK_MODE_SHARED, MASK_MODE_PER_CHANNEL,
};

#[tauri::command]
pub async fn apply_arcsinh_stretch_cmd(
    path: String,
    output_dir: String,
    factor: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let clamped_factor = (factor as f32).clamp(1.0, 500.0);

        let t0 = std::time::Instant::now();
        let stretched = arcsinh_stretch(image, clamped_factor);
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_and_save_as(&stretched, &path, &output_dir, "arcsinh", true, OutputValues::DisplayReferred)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_STRETCH_FACTOR: clamped_factor,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

fn ghs_params_from_args(
    stretch_factor: f64,
    local_intensity: Option<f64>,
    symmetry_point: Option<f64>,
    shadow_protect: Option<f64>,
    highlight_protect: Option<f64>,
) -> GhsParams {
    let sp = symmetry_point.unwrap_or(0.05).clamp(0.0, 1.0);
    GhsParams {
        d: stretch_factor.clamp(0.0, 50.0),
        b: local_intensity.unwrap_or(0.0).clamp(-10.0, 15.0),
        sp,
        lp: shadow_protect.unwrap_or(0.0).clamp(0.0, sp),
        hp: highlight_protect.unwrap_or(1.0).clamp(sp, 1.0),
    }
}

#[tauri::command]
pub async fn apply_ghs_stretch_cmd(
    path: String,
    output_dir: String,
    stretch_factor: f64,
    local_intensity: Option<f64>,
    symmetry_point: Option<f64>,
    shadow_protect: Option<f64>,
    highlight_protect: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let params = ghs_params_from_args(
            stretch_factor,
            local_intensity,
            symmetry_point,
            shadow_protect,
            highlight_protect,
        );

        let t0 = std::time::Instant::now();
        let stretched = ghs_stretch(image, &params);
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_and_save_as(&stretched, &path, &output_dir, "ghs", true, OutputValues::DisplayReferred)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_STRETCH_FACTOR: params.d,
            RES_LOCAL_INTENSITY: params.b,
            RES_SYMMETRY_POINT: params.sp,
            RES_SHADOW_PROTECT: params.lp,
            RES_HIGHLIGHT_PROTECT: params.hp,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn ghs_stretch_composite_cmd(
    output_dir: String,
    stretch_factor: f64,
    local_intensity: Option<f64>,
    symmetry_point: Option<f64>,
    shadow_protect: Option<f64>,
    highlight_protect: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()?;

        let params = ghs_params_from_args(
            stretch_factor,
            local_intensity,
            symmetry_point,
            shadow_protect,
            highlight_protect,
        );

        let t0 = std::time::Instant::now();
        let (r, g, b) = ghs_stretch_rgb(er.arr(), eg.arr(), eb.arr(), &params);
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let (rows, cols) = r.dim();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_ghs_{}.png", output_dir, ts);
        helpers::render_rgb_preview(&r, &g, &b, &png_path, MAX_PREVIEW_DIM)?;
        helpers::insert_composite_stretched(r, g, b);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_STRETCH_FACTOR: params.d,
            RES_LOCAL_INTENSITY: params.b,
            RES_SYMMETRY_POINT: params.sp,
            RES_SHADOW_PROTECT: params.lp,
            RES_HIGHLIGHT_PROTECT: params.hp,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn masked_stretch_cmd(
    path: String,
    output_dir: String,
    iterations: Option<usize>,
    target_background: Option<f64>,
    mask_growth: Option<f64>,
    mask_softness: Option<f64>,
    protection_amount: Option<f64>,
    luminance_protect: Option<bool>,
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let config = MaskedStretchConfig {
            iterations: iterations.unwrap_or(10),
            target_background: target_background.unwrap_or(0.25),
            mask_growth: mask_growth.unwrap_or(2.5),
            mask_softness: mask_softness.unwrap_or(4.0),
            protection_amount: protection_amount.unwrap_or(0.85),
            luminance_protect: luminance_protect.unwrap_or(true),
            detection_sigma: detection_sigma.unwrap_or(8.0).clamp(3.0, 20.0),
            max_eccentricity: max_eccentricity.unwrap_or(0.85).clamp(0.3, 1.0),
            ..MaskedStretchConfig::default()
        };

        let t0 = std::time::Instant::now();
        let result = masked_stretch(image, &config).map_err(|e| anyhow::anyhow!(e))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let ro = render_and_save_as(&result.image, &path, &output_dir, SUFFIX_MASKED_STRETCH, true, OutputValues::DisplayReferred)?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: ro.fits_path,
            RES_ITERATIONS_RUN: result.iterations_run,
            RES_FINAL_BACKGROUND: result.final_background,
            RES_STARS_MASKED: result.stars_masked,
            RES_MASK_COVERAGE: result.mask_coverage,
            RES_CONVERGED: result.converged,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn arcsinh_stretch_composite_cmd(
    output_dir: String,
    factor: f64,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()?;
        let clamped_factor = (factor as f32).clamp(1.0, 500.0);

        let t0 = std::time::Instant::now();
        let (r, g, b) = arcsinh_stretch_rgb(er.arr(), eg.arr(), eb.arr(), clamped_factor);
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let (rows, cols) = r.dim();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_arcsinh_{}.png", output_dir, ts);
        helpers::render_rgb_preview(&r, &g, &b, &png_path, MAX_PREVIEW_DIM)?;
        helpers::insert_composite_stretched(r, g, b);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_STRETCH_FACTOR: clamped_factor,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

fn channel_stats_json(r: &crate::core::imaging::masked_stretch::MaskedStretchResult) -> serde_json::Value {
    json!({
        RES_ITERATIONS_RUN: r.iterations_run,
        RES_FINAL_BACKGROUND: r.final_background,
        RES_CONVERGED: r.converged,
    })
}

#[tauri::command]
pub async fn masked_stretch_composite_cmd(
    output_dir: String,
    iterations: Option<usize>,
    target_background: Option<f64>,
    mask_growth: Option<f64>,
    mask_softness: Option<f64>,
    protection_amount: Option<f64>,
    luminance_protect: Option<bool>,
    shared_mask: Option<bool>,
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()?;

        let config = MaskedStretchConfig {
            iterations: iterations.unwrap_or(10),
            target_background: target_background.unwrap_or(0.25),
            mask_growth: mask_growth.unwrap_or(2.5),
            mask_softness: mask_softness.unwrap_or(4.0),
            protection_amount: protection_amount.unwrap_or(0.85),
            luminance_protect: luminance_protect.unwrap_or(true),
            detection_sigma: detection_sigma.unwrap_or(8.0).clamp(3.0, 20.0),
            max_eccentricity: max_eccentricity.unwrap_or(0.85).clamp(0.3, 1.0),
            ..MaskedStretchConfig::default()
        };

        let t0 = std::time::Instant::now();
        let use_shared = shared_mask.unwrap_or(true);

        let (r_img, g_img, b_img, per_channel, stars, coverage, mask_mode) = if use_shared {
            let result = masked_stretch_rgb_shared(er.arr(), eg.arr(), eb.arr(), &config)
                .map_err(|e| anyhow::anyhow!(e))?;
            let pc = json!({
                RES_R: channel_stats_json(&result.r),
                RES_G: channel_stats_json(&result.g),
                RES_B: channel_stats_json(&result.b),
            });
            (
                result.r.image, result.g.image, result.b.image,
                pc, result.shared_stars_masked, result.shared_mask_coverage,
                MASK_MODE_SHARED,
            )
        } else {
            let (res_r, (res_g, res_b)) = rayon::join(
                || masked_stretch(er.arr(), &config),
                || rayon::join(
                    || masked_stretch(eg.arr(), &config),
                    || masked_stretch(eb.arr(), &config),
                ),
            );
            let r = res_r.map_err(|e| anyhow::anyhow!(e))?;
            let g = res_g.map_err(|e| anyhow::anyhow!(e))?;
            let b = res_b.map_err(|e| anyhow::anyhow!(e))?;
            let pc = json!({
                RES_R: channel_stats_json(&r),
                RES_G: channel_stats_json(&g),
                RES_B: channel_stats_json(&b),
            });
            let total_stars = r.stars_masked + g.stars_masked + b.stars_masked;
            let avg_coverage = (r.mask_coverage + g.mask_coverage + b.mask_coverage) / 3.0;
            (
                r.image, g.image, b.image,
                pc, total_stars, avg_coverage,
                MASK_MODE_PER_CHANNEL,
            )
        };

        let elapsed_ms = t0.elapsed().as_millis() as u64;
        let (rows, cols) = r_img.dim();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_masked_{}.png", output_dir, ts);
        helpers::render_rgb_preview(&r_img, &g_img, &b_img, &png_path, MAX_PREVIEW_DIM)?;
        helpers::insert_composite_stretched(r_img, g_img, b_img);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_STARS_MASKED: stars,
            RES_MASK_COVERAGE: coverage,
            CHANNELS: per_channel,
            RES_MASK_MODE: mask_mode,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ndarray::Array2;

    use super::*;
    use crate::cmd::common::{cached_header, extract_image_resolved};
    use crate::cmd::processing::local_contrast::is_display_referred;
    use crate::core::imaging::stats::compute_image_stats;
    use crate::infra::cache::{lock_wizard_entries, GLOBAL_IMAGE_CACHE};
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::wizard_bg_key;
    use crate::types::header::HduHeader;

    fn starry_field() -> Array2<f32> {
        Array2::from_shape_fn((48, 48), |(y, x)| {
            let mut v = 200.0 + ((y * 31 + x * 17) % 13) as f32;
            for (cy, cx) in [(12.0f32, 12.0f32), (30.0, 18.0), (24.0, 38.0)] {
                let d2 = (y as f32 - cy).powi(2) + (x as f32 - cx).powi(2);
                v += 5000.0 * (-d2 / 4.5).exp();
            }
            v
        })
    }

    fn assert_shown_as_computed(result: &serde_json::Value) {
        let fits = result[RES_FITS_PATH].as_str().unwrap();
        let header = cached_header(fits).unwrap();
        assert!(is_display_referred(Some(&header)), "{fits} is not flagged display-referred");
        assert!(header.get("BUNIT").is_none(), "{fits} still claims the source unit");
        assert_eq!(header.get("CRVAL1").map(str::trim), Some("83.8"), "{fits} lost its WCS");
        let values = extract_image_resolved(fits).unwrap().arr;
        let as_computed: Vec<u8> = values.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        let png = image::open(result[RES_PNG_PATH].as_str().unwrap()).unwrap().to_luma8().into_raw();
        assert_eq!(png, as_computed, "the preview of {fits} was auto-stretched again");
    }

    #[tokio::test]
    async fn stretch_outputs_are_previewed_and_flagged_as_the_computed_stretch() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("stretch_src.fits").to_str().unwrap().to_string();
        let mut header = HduHeader::empty();
        for (k, v) in [("BUNIT", "'MJy/sr'"), ("CTYPE1", "'RA---TAN'"), ("CRVAL1", "83.8"), ("CRPIX1", "24.0")] {
            header.set(k, v.to_string());
        }
        write_fits_mono(&src, &starry_field(), Some(&header)).unwrap();

        assert_shown_as_computed(&apply_arcsinh_stretch_cmd(src.clone(), out.clone(), 50.0).await.unwrap());
        assert_shown_as_computed(&apply_ghs_stretch_cmd(src.clone(), out.clone(), 5.0, None, None, None, None).await.unwrap());
        let masked = masked_stretch_cmd(src, out, Some(3), None, None, None, None, None, None, None).await.unwrap();
        assert_shown_as_computed(&masked);
    }

    #[tokio::test]
    async fn a_wizard_channel_held_in_memory_keeps_its_wcs_and_observation_cards_through_the_stretch() {
        let _wizard = lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let key = wizard_bg_key("stretch_test_r");
        let mut header = HduHeader::empty();
        for (k, v) in [
            ("BUNIT", "'MJy/sr'"),
            ("CTYPE1", "'RA---TAN'"),
            ("CRVAL1", "83.8"),
            ("CRPIX1", "24.0"),
            ("DATE-OBS", "'2024-01-02T03:04:05'"),
            ("FILTER", "'F656N'"),
        ] {
            header.set(k, v.to_string());
        }
        let field = starry_field();
        GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(&key, Arc::new(field.clone()), compute_image_stats(&field), Some(header));

        let result = apply_arcsinh_stretch_cmd(key.clone(), out, 50.0).await;
        GLOBAL_IMAGE_CACHE.remove(&key);
        let result = result.unwrap();
        assert_shown_as_computed(&result);
        let written = cached_header(result[RES_FITS_PATH].as_str().unwrap()).unwrap();
        let unquoted = |key: &str| written.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string());
        assert_eq!(unquoted("DATE-OBS").as_deref(), Some("2024-01-02T03:04:05"), "the observation date was dropped");
        assert_eq!(unquoted("FILTER").as_deref(), Some("F656N"), "the filter was dropped");
        assert_eq!(written.get_f64("CRVAL1"), Some(83.8), "the WCS reference value was dropped");
        assert_eq!(unquoted("CTYPE1").as_deref(), Some("RA---TAN"), "the WCS axis type was dropped");
    }
}
