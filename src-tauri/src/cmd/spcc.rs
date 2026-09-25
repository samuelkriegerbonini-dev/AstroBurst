use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, load_from_cache_or_disk};
use crate::cmd::processing::source_header;
use crate::core::astrometry::spcc::{
    spcc_calibrate_rgb, SpccConfig, SpccCatalog, WhiteReference,
};
use crate::core::astrometry::wcs::WcsTransform;
use crate::infra::cache::ImageEntry;
use crate::types::constants::{RES_ELAPSED_MS, RES_R_FACTOR, RES_G_FACTOR, RES_B_FACTOR, RES_STARS_MATCHED, RES_STARS_TOTAL, RES_AVG_COLOR_INDEX, RES_WHITE_REF, RES_CATALOG_NAME, IS_SYNTHETIC_CATALOG};
use crate::types::header::HduHeader;

fn spcc_header(r_path: &str, r_entry: &ImageEntry, wcs_path: Option<&str>) -> anyhow::Result<HduHeader> {
    let (label, header) = match wcs_path {
        Some(path) => (format!("WCS source {}", path), cached_header(path).ok()),
        None => (format!("R channel {}", r_path), source_header(r_path, r_entry)),
    };
    let header = header.ok_or_else(|| {
        anyhow::anyhow!("{} carries no FITS header; SPCC needs a plate-solved image whose header holds a celestial WCS.", label)
    })?;
    WcsTransform::from_header(&header).map_err(|e| {
        anyhow::anyhow!("{} has no usable celestial WCS ({:#}); SPCC needs a plate-solved image.", label, e)
    })?;
    Ok(header)
}

#[tauri::command]
pub async fn spcc_calibrate_cmd(
    r_path: String,
    g_path: String,
    b_path: String,
    wcs_path: Option<String>,
    white_reference: Option<String>,
    min_snr: Option<f64>,
    max_stars: Option<usize>,
    catalog: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let r_entry = load_from_cache_or_disk(&r_path)?;
        let g_entry = load_from_cache_or_disk(&g_path)?;
        let b_entry = load_from_cache_or_disk(&b_path)?;

        let header = spcc_header(&r_path, &r_entry, wcs_path.as_deref())?;

        let wr = match white_reference.as_deref() {
            Some("g2v") | Some("G2V") => WhiteReference::G2V,
            Some("photopic") => WhiteReference::Photopic,
            Some("spiral") | Some("average_spiral") | None => WhiteReference::AverageSpiral,
            _ => WhiteReference::AverageSpiral,
        };

        let cat = match catalog.as_deref() {
            Some("builtin") | Some("synthetic") => SpccCatalog::BuiltinBpRp,
            _ => SpccCatalog::GaiaDr3Tap,
        };

        let config = SpccConfig {
            min_snr: min_snr.unwrap_or(20.0),
            max_stars: max_stars.unwrap_or(200),
            catalog: cat,
            white_reference: wr,
            ..SpccConfig::default()
        };

        let t0 = std::time::Instant::now();
        let result = spcc_calibrate_rgb(
            r_entry.arr(),
            g_entry.arr(),
            b_entry.arr(),
            &header,
            &config,
        ).map_err(|e| anyhow::anyhow!(e))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_R_FACTOR: result.r_factor,
            RES_G_FACTOR: result.g_factor,
            RES_B_FACTOR: result.b_factor,
            RES_STARS_MATCHED: result.stars_matched,
            RES_STARS_TOTAL: result.stars_total,
            RES_AVG_COLOR_INDEX: result.avg_color_index,
            RES_WHITE_REF: result.white_ref_name,
            RES_CATALOG_NAME: result.catalog_name,
            IS_SYNTHETIC_CATALOG: result.is_synthetic_catalog,
            RES_ELAPSED_MS: elapsed_ms,
        }))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ndarray::Array2;

    use super::*;
    use crate::core::imaging::stats::compute_image_stats;
    use crate::infra::cache::{lock_wizard_entries, GLOBAL_IMAGE_CACHE};
    use crate::types::constants::wizard_bg_key;
    use crate::types::header::HduHeader;

    fn solved_header() -> HduHeader {
        let mut header = HduHeader::empty();
        header.set("CTYPE1", "RA---TAN".to_string());
        header.set("CTYPE2", "DEC--TAN".to_string());
        header.set_f64("CRVAL1", 83.8);
        header.set_f64("CRVAL2", -5.4);
        header.set_f64("CRPIX1", 64.0);
        header.set_f64("CRPIX2", 64.0);
        header.set_f64("CD1_1", -2.78e-4);
        header.set_f64("CD2_2", 2.78e-4);
        header
    }

    fn star_field(scale: f64) -> Array2<f32> {
        let stars = [
            (20.0, 24.0, 6000.0, 1.6),
            (40.0, 96.0, 500.0, 1.2),
            (64.0, 30.0, 800.0, 1.4),
            (90.0, 60.0, 1200.0, 1.7),
            (28.0, 70.0, 1600.0, 2.0),
            (104.0, 104.0, 2000.0, 2.2),
            (70.0, 84.0, 2400.0, 2.4),
            (96.0, 20.0, 1000.0, 1.3),
            (50.0, 56.0, 700.0, 1.9),
        ];
        Array2::from_shape_fn((128, 128), |(y, x)| {
            let noise = ((y * 31 + x * 17) % 23) as f64 - 11.0;
            let mut v = 100.0 + noise;
            for (cy, cx, amp, sigma) in stars {
                let d2 = (y as f64 - cy).powi(2) + (x as f64 - cx).powi(2);
                v += amp * (-d2 / (2.0 * sigma * sigma)).exp();
            }
            (v * scale) as f32
        })
    }

    fn insert(key: &str, arr: &Array2<f32>, header: Option<HduHeader>) {
        GLOBAL_IMAGE_CACHE.insert_synthetic_with_header(key, Arc::new(arr.clone()), compute_image_stats(arr), header);
    }

    #[tokio::test]
    async fn spcc_runs_on_wizard_channels_with_the_builtin_catalog_without_touching_the_disk() {
        let _wizard = lock_wizard_entries();
        let keys = [wizard_bg_key("spcc_r"), wizard_bg_key("spcc_g"), wizard_bg_key("spcc_b")];
        let bare = wizard_bg_key("spcc_bare_r");
        insert(&keys[0], &star_field(1.0), Some(solved_header()));
        insert(&keys[1], &star_field(0.8), None);
        insert(&keys[2], &star_field(0.6), None);
        insert(&bare, &star_field(1.0), None);

        let own = spcc_calibrate_cmd(
            keys[0].clone(),
            keys[1].clone(),
            keys[2].clone(),
            None,
            None,
            Some(5.0),
            None,
            Some("builtin".to_string()),
        )
        .await;
        let named = spcc_calibrate_cmd(
            bare.clone(),
            keys[1].clone(),
            keys[2].clone(),
            Some(keys[0].clone()),
            None,
            Some(5.0),
            None,
            Some("builtin".to_string()),
        )
        .await;
        for key in keys.iter().chain(std::iter::once(&bare)) {
            GLOBAL_IMAGE_CACHE.remove(key);
        }

        let own = own.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(own[IS_SYNTHETIC_CATALOG], true, "{own}");
        assert!(own[RES_STARS_MATCHED].as_u64().unwrap() >= 3, "{own}");
        let r_factor = own[RES_R_FACTOR].as_f64().unwrap();
        assert!(r_factor.is_finite() && r_factor > 0.0, "{own}");
        assert_eq!(own[RES_G_FACTOR], 1.0);
        let named = named.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(named[RES_R_FACTOR], own[RES_R_FACTOR], "the WCS named by wcs_path must give the same solution");
        assert_eq!(named[RES_STARS_MATCHED], own[RES_STARS_MATCHED]);
    }

    #[tokio::test]
    async fn a_wizard_channel_without_a_celestial_wcs_is_refused_with_a_plate_solving_hint() {
        let _wizard = lock_wizard_entries();
        let bare = wizard_bg_key("spcc_no_header_r");
        let unsolved = wizard_bg_key("spcc_unsolved_r");
        let g = wizard_bg_key("spcc_no_wcs_g");
        let b = wizard_bg_key("spcc_no_wcs_b");
        let mut bunit_only = HduHeader::empty();
        bunit_only.set("BUNIT", "MJy/sr".to_string());
        insert(&bare, &star_field(1.0), None);
        insert(&unsolved, &star_field(1.0), Some(bunit_only));
        insert(&g, &star_field(0.8), None);
        insert(&b, &star_field(0.6), None);

        let mut errors = Vec::new();
        for r in [&bare, &unsolved] {
            errors.push((
                r.clone(),
                spcc_calibrate_cmd(r.clone(), g.clone(), b.clone(), None, None, Some(5.0), None, Some("builtin".to_string()))
                    .await
                    .unwrap_err(),
            ));
        }
        for key in [&bare, &unsolved, &g, &b] {
            GLOBAL_IMAGE_CACHE.remove(key);
        }
        for (key, err) in errors {
            assert!(err.contains(&key), "the channel is not named: {err}");
            assert!(err.contains("plate-solved"), "{err}");
            assert!(!err.contains("Failed to open") && !err.contains("os error"), "a cache key was opened as a file: {err}");
        }
    }
}
