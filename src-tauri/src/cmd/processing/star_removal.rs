use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::processing::local_contrast::render_linear_output;
use crate::cmd::helpers;
use crate::core::imaging::star_removal::{remove_stars, remove_stars_rgb_shared, StarRemovalConfig};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_MASK_COVERAGE, RES_PNG_PATH,
    RES_STARS_FITS_PATH, RES_STARS_MASKED, RES_STARS_PNG_PATH,
};

fn config_from_args(
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
    growth_factor: Option<f64>,
    softness: Option<f64>,
    bright_ceiling: Option<f64>,
) -> StarRemovalConfig {
    let defaults = StarRemovalConfig::default();
    StarRemovalConfig {
        detection_sigma: detection_sigma.unwrap_or(defaults.detection_sigma).clamp(2.0, 20.0),
        max_eccentricity: max_eccentricity.unwrap_or(defaults.max_eccentricity).clamp(0.3, 1.0),
        growth_factor: growth_factor.unwrap_or(defaults.growth_factor).clamp(1.0, 10.0),
        softness: softness.unwrap_or(defaults.softness).clamp(0.0, 20.0),
        bright_ceiling: bright_ceiling.unwrap_or(defaults.bright_ceiling).clamp(0.5, 1.0),
    }
}

#[tauri::command]
pub async fn remove_stars_cmd(
    path: String,
    output_dir: String,
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
    growth_factor: Option<f64>,
    softness: Option<f64>,
    bright_ceiling: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let entry = load_from_cache_or_disk(&path)?;
        let image = entry.arr();

        let config = config_from_args(
            detection_sigma,
            max_eccentricity,
            growth_factor,
            softness,
            bright_ceiling,
        );

        let t0 = std::time::Instant::now();
        let result = remove_stars(image, &config).map_err(|e| anyhow::anyhow!(e))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let starless_out = render_linear_output(&result.starless, &path, &entry, &output_dir, "starless")?;
        let stars_out = render_linear_output(&result.stars, &path, &entry, &output_dir, "stars")?;
        let (rows, cols) = starless_out.dims;

        Ok(json!({
            RES_PNG_PATH: starless_out.png_path,
            RES_FITS_PATH: starless_out.fits_path,
            RES_STARS_PNG_PATH: stars_out.png_path,
            RES_STARS_FITS_PATH: stars_out.fits_path,
            RES_STARS_MASKED: result.stars_removed,
            RES_MASK_COVERAGE: result.mask_coverage,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn remove_stars_composite_cmd(
    output_dir: String,
    detection_sigma: Option<f64>,
    max_eccentricity: Option<f64>,
    growth_factor: Option<f64>,
    softness: Option<f64>,
    bright_ceiling: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;

        let (er, eg, eb) = helpers::load_composite_rgb()?;

        let config = config_from_args(
            detection_sigma,
            max_eccentricity,
            growth_factor,
            softness,
            bright_ceiling,
        );

        let t0 = std::time::Instant::now();
        let result = remove_stars_rgb_shared(er.arr(), eg.arr(), eb.arr(), &config)
            .map_err(|e| anyhow::anyhow!(e))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let (rows, cols) = result.r.starless.dim();

        let stats_r = compute_image_stats(&result.r.starless);
        let stats_g = compute_image_stats(&result.g.starless);
        let stats_b = compute_image_stats(&result.b.starless);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let png_path = format!("{}/composite_starless_{}.png", output_dir, ts);
        let stars_png_path = format!("{}/composite_stars_{}.png", output_dir, ts);

        let stf_config = AutoStfConfig::default();
        let (linked_stf, combined_stats) =
            helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_g = make_stf_u8_fn(&linked_stf, &combined_stats);
        let fn_b = make_stf_u8_fn(&linked_stf, &combined_stats);
        helpers::render_rgb_preview_with_stf(
            &result.r.starless,
            &result.g.starless,
            &result.b.starless,
            fn_r,
            fn_g,
            fn_b,
            &png_path,
            MAX_PREVIEW_DIM,
        )?;

        helpers::render_rgb_preview(
            &result.r.stars,
            &result.g.stars,
            &result.b.stars,
            &stars_png_path,
            MAX_PREVIEW_DIM,
        )?;

        helpers::insert_composite_content(
            result.r.starless,
            result.g.starless,
            result.b.starless,
            stats_r,
            stats_g,
            stats_b,
        );

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_STARS_PNG_PATH: stars_png_path,
            RES_STARS_MASKED: result.stars_removed,
            RES_MASK_COVERAGE: result.mask_coverage,
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
    use crate::infra::cache::GLOBAL_IMAGE_CACHE;
    use crate::types::constants::{
        COMPOSITE_KEY_B, COMPOSITE_KEY_G, COMPOSITE_KEY_R, COMPOSITE_ORIG_B, COMPOSITE_ORIG_G, COMPOSITE_ORIG_R,
    };

    fn starry(scale: f32) -> Array2<f32> {
        Array2::from_shape_fn((64, 64), |(y, x)| {
            let mut v = 0.1 + ((y * 31 + x * 17) % 13) as f32 * 0.002;
            for (cy, cx) in [(16.0f32, 16.0f32), (40.0, 22.0), (30.0, 48.0)] {
                let d2 = (y as f32 - cy).powi(2) + (x as f32 - cx).powi(2);
                v += 0.8 * (-d2 / 4.5).exp();
            }
            v * scale
        })
    }

    fn header_with(cards: &[(&str, &str)]) -> crate::types::header::HduHeader {
        let mut header = crate::types::header::HduHeader::empty();
        for (k, v) in cards {
            header.set(k, v.to_string());
        }
        header
    }

    fn unquoted(header: &crate::types::header::HduHeader, key: &str) -> Option<String> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string())
    }

    #[tokio::test]
    async fn starless_and_stars_outputs_keep_the_flux_calibration_of_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("calibrated_i2d.fits").to_str().unwrap().to_string();
        let source = header_with(&[
            ("BUNIT", "'MJy/sr'"),
            ("PHOTMJSR", "1.5"),
            ("PIXAR_SR", "2.1E-13"),
            ("CTYPE1", "'RA---TAN'"),
            ("CRVAL1", "83.8"),
        ]);
        crate::infra::fits::writer::write_fits_mono(&src, &starry(1000.0), Some(&source)).unwrap();

        let value = remove_stars_cmd(src, out, None, None, None, None, None).await.unwrap();
        for key in [RES_FITS_PATH, RES_STARS_FITS_PATH] {
            let header = crate::cmd::common::cached_header(value[key].as_str().unwrap()).unwrap();
            assert_eq!(unquoted(&header, "BUNIT").as_deref(), Some("MJy/sr"), "{key} lost its unit");
            assert_eq!(header.get_f64("PHOTMJSR"), Some(1.5), "{key} lost its flux conversion");
            assert_eq!(header.get_f64("PIXAR_SR"), Some(2.1e-13), "{key} lost its pixel area");
            assert_eq!(header.get_f64("CRVAL1"), Some(83.8), "{key} lost its WCS");
            assert!(!crate::cmd::processing::is_display_referred(Some(&header)));
        }
    }

    #[tokio::test]
    async fn star_removal_on_a_stretched_image_stays_display_referred() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("stretched_arcsinh.fits").to_str().unwrap().to_string();
        let flagged = header_with(&[(crate::cmd::common::HEADER_DISPLAY_REFERRED, "T")]);
        let data = starry(0.9);
        crate::infra::fits::writer::write_fits_mono(&src, &data, Some(&flagged)).unwrap();

        let value = remove_stars_cmd(src, out, None, None, None, None, None).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap();
        let header = crate::cmd::common::cached_header(fits).unwrap();
        assert!(crate::cmd::processing::is_display_referred(Some(&header)));
        let starless = crate::cmd::common::extract_image_resolved(fits).unwrap().arr;
        let as_computed: Vec<u8> = starless.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        let png = image::open(value[RES_PNG_PATH].as_str().unwrap()).unwrap().to_luma8().into_raw();
        assert_eq!(png, as_computed, "the starless preview of a stretched image was stretched again");
    }

    #[tokio::test]
    async fn composite_star_removal_keeps_the_white_balance_base_in_step() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let base = [starry(1.0), starry(0.8), starry(1.2)];
        let factors = [2.0f32, 1.0, 0.5];
        let [sr, sg, sb] = base.each_ref().map(compute_image_stats);
        helpers::insert_composite_and_orig(base[0].clone(), base[1].clone(), base[2].clone(), sr, sg, sb);
        let balanced: [Array2<f32>; 3] = [0, 1, 2].map(|i| base[i].mapv(|v| v * factors[i]));
        helpers::insert_composite_white_balanced(
            balanced.each_ref().map(|a| (Arc::new(a.clone()), compute_image_stats(a))),
            factors,
        );

        let run = remove_stars_composite_cmd(dir.path().to_str().unwrap().to_string(), None, None, None, None, None).await;
        let keys = [COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B].map(|k| GLOBAL_IMAGE_CACHE.get(k));
        let origs = [COMPOSITE_ORIG_R, COMPOSITE_ORIG_G, COMPOSITE_ORIG_B].map(|k| GLOBAL_IMAGE_CACHE.get(k));
        helpers::clear_composite();
        run.unwrap();

        for i in 0..3 {
            let key = keys[i].as_ref().expect("composite channel");
            let orig = origs[i].as_ref().expect("white-balance base");
            assert_ne!(key.arr(), &balanced[i], "precondition: star removal changed channel {i}");
            let consistent = orig
                .arr()
                .iter()
                .zip(key.arr().iter())
                .all(|(o, k)| (o * factors[i] - k).abs() <= 1e-5 * k.abs().max(1.0));
            assert!(consistent, "Color Balance Apply/Reset would bring the stars back in channel {i}");
        }
    }
}
