use serde_json::json;

use crate::cmd::common::{blocking_cmd, cached_header, load_from_cache_or_disk};
use crate::cmd::processing::source_header;
use crate::core::imaging::cutout::{shift_header, CutoutRect};
use crate::core::astrometry::spcc::{
    spcc_calibrate_rgb, SpccConfig, SpccCatalog, WhiteReference,
};
use crate::infra::cache::ImageEntry;
use crate::infra::wcs_source::load_wcs;
use crate::types::constants::{
    IS_SYNTHETIC_CATALOG, RES_AVG_COLOR_INDEX, RES_B_FACTOR, RES_CATALOG_NAME, RES_CATALOG_SOURCE, RES_ELAPSED_MS, RES_G_FACTOR,
    RES_R_FACTOR, RES_STARS_MATCHED, RES_STARS_TOTAL, RES_WAVELENGTHS_NM, RES_WAVELENGTH_SOURCE, RES_WHITE_REF,
};
use crate::types::header::HduHeader;

const ADD_WCS_HINT: &str = "Plate solve the R channel (or the first frame it was stacked from) in Analysis > Astrometry and click Write WCS; SPCC then uses that WCS.";

fn naxis_dims(header: &HduHeader) -> Option<(usize, usize)> {
    let cols = usize::try_from(header.get_i64("NAXIS1")?).ok()?;
    let rows = usize::try_from(header.get_i64("NAXIS2")?).ok()?;
    Some((cols, rows))
}

fn wcs_grid_rect(r_header: Option<&HduHeader>, r_shape: (usize, usize), solved: &HduHeader) -> anyhow::Result<CutoutRect> {
    let ltv = |header: Option<&HduHeader>, key: &str| header.and_then(|h| h.get_f64(key)).unwrap_or(0.0);
    let x0 = (ltv(Some(solved), "LTV1") - ltv(r_header, "LTV1")).round() as i64;
    let y0 = (ltv(Some(solved), "LTV2") - ltv(r_header, "LTV2")).round() as i64;
    let (rows, cols) = r_shape;
    let solved_dims = naxis_dims(solved);
    let fits = match solved_dims {
        Some(dims) if x0 == 0 && y0 == 0 => (cols, rows) == dims,
        Some((w, h)) => x0 >= 0 && y0 >= 0 && x0 as usize + cols <= w && y0 as usize + rows <= h,
        None => x0 >= 0 && y0 >= 0,
    };
    if !fits {
        let solved_size = solved_dims.map_or_else(|| "unknown size".to_string(), |(w, h)| format!("{w}x{h}"));
        anyhow::bail!(
            "The R channel ({cols}x{rows}, offset {x0},{y0}) does not fit the plate-solved image ({solved_size}); plate solve the R channel's reference frame again."
        );
    }
    Ok(CutoutRect { x0, y0, width: cols, height: rows })
}

fn spcc_header(r_path: &str, r_entry: &ImageEntry, wcs_path: Option<&str>) -> anyhow::Result<(String, HduHeader)> {
    let (label, header) = match wcs_path {
        Some(path) => (format!("WCS source {}", path), cached_header(path).ok()),
        None => (format!("R channel {}", r_path), source_header(r_path, r_entry)),
    };
    let header = header.ok_or_else(|| anyhow::anyhow!("{} carries no FITS header. {}", label, ADD_WCS_HINT))?;
    let source_path = wcs_path.unwrap_or(r_path);
    load_wcs(source_path, &header)
        .map_err(|e| anyhow::anyhow!("{} has no usable celestial WCS ({:#}). {}", label, e, ADD_WCS_HINT))?;
    if wcs_path.is_none() {
        return Ok((source_path.to_string(), header));
    }
    let rect = wcs_grid_rect(source_header(r_path, r_entry).as_ref(), r_entry.arr().dim(), &header)?;
    Ok((source_path.to_string(), shift_header(&header, &rect)))
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
    wavelengths_nm: Option<[f64; 3]>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let r_entry = load_from_cache_or_disk(&r_path)?;
        let g_entry = load_from_cache_or_disk(&g_path)?;
        let b_entry = load_from_cache_or_disk(&b_path)?;

        let (wcs_source, header) = spcc_header(&r_path, &r_entry, wcs_path.as_deref())?;

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

        let mut config = SpccConfig {
            min_snr: min_snr.unwrap_or(20.0),
            max_stars: max_stars.unwrap_or(200),
            catalog: cat,
            white_reference: wr,
            ..SpccConfig::default()
        };
        if let Some(wl) = wavelengths_nm {
            config.wavelengths_nm = wl;
        }

        let t0 = std::time::Instant::now();
        let result = spcc_calibrate_rgb(
            r_entry.arr(),
            g_entry.arr(),
            b_entry.arr(),
            &wcs_source,
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
            RES_WAVELENGTHS_NM: result.wavelengths_nm,
            RES_WAVELENGTH_SOURCE: result.wavelength_source,
            RES_CATALOG_SOURCE: result.catalog_source,
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
            None,
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
            None,
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
                spcc_calibrate_cmd(r.clone(), g.clone(), b.clone(), None, None, Some(5.0), None, Some("builtin".to_string()), None)
                    .await
                    .unwrap_err(),
            ));
        }
        for key in [&bare, &unsolved, &g, &b] {
            GLOBAL_IMAGE_CACHE.remove(key);
        }
        for (key, err) in errors {
            assert!(err.contains(&key), "the channel is not named: {err}");
            assert!(err.contains("Plate solve the R channel"), "{err}");
            assert!(!err.contains("Failed to open") && !err.contains("os error"), "a cache key was opened as a file: {err}");
        }
    }

    #[tokio::test]
    async fn spcc_response_reports_wavelengths_and_catalog_source() {
        let _wizard = lock_wizard_entries();
        let keys = [wizard_bg_key("spcc_wl_r"), wizard_bg_key("spcc_wl_g"), wizard_bg_key("spcc_wl_b")];
        insert(&keys[0], &star_field(1.0), Some(solved_header()));
        insert(&keys[1], &star_field(0.8), None);
        insert(&keys[2], &star_field(0.6), None);

        let run = |wl: Option<[f64; 3]>| {
            spcc_calibrate_cmd(
                keys[0].clone(),
                keys[1].clone(),
                keys[2].clone(),
                None,
                None,
                Some(5.0),
                None,
                Some("builtin".to_string()),
                wl,
            )
        };
        let default = run(None).await;
        let filters = run(Some([814.0, 555.0, 435.0])).await;
        let rejected = run(Some([200.0, 530.0, 460.0])).await;
        for key in &keys {
            GLOBAL_IMAGE_CACHE.remove(key);
        }

        let default = default.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(default[RES_WAVELENGTHS_NM], json!([640.0, 530.0, 460.0]), "{default}");
        assert_eq!(default[RES_WAVELENGTH_SOURCE], "default", "{default}");
        assert_eq!(default[RES_CATALOG_SOURCE], serde_json::Value::Null, "{default}");
        assert_eq!(default[IS_SYNTHETIC_CATALOG], true, "{default}");

        let filters = filters.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(filters[RES_WAVELENGTHS_NM], json!([814.0, 555.0, 435.0]), "{filters}");
        assert_eq!(filters[RES_WAVELENGTH_SOURCE], "filters", "{filters}");
        assert_eq!(filters[RES_CATALOG_SOURCE], serde_json::Value::Null, "{filters}");
        assert_ne!(filters[RES_R_FACTOR], default[RES_R_FACTOR], "the wavelengths must change the Planck expectation");

        let err = rejected.expect_err("200 nm is outside the SPCC range");
        assert!(err.contains("SPCC wavelengths must be finite and between 300 and 1200 nm (got [200, 530, 460])."), "{err}");
    }

    const EXPECTED_HINT: &str = "Plate solve the R channel (or the first frame it was stacked from) in Analysis > Astrometry and click Write WCS; SPCC then uses that WCS.";

    #[tokio::test]
    async fn missing_wcs_error_says_how_to_add_one() {
        let _wizard = lock_wizard_entries();
        let unsolved = wizard_bg_key("spcc_hint_unsolved_r");
        let bare = wizard_bg_key("spcc_hint_bare_r");
        let g = wizard_bg_key("spcc_hint_g");
        let b = wizard_bg_key("spcc_hint_b");
        let mut bunit_only = HduHeader::empty();
        bunit_only.set("BUNIT", "ADU".to_string());
        insert(&unsolved, &star_field(1.0), Some(bunit_only));
        insert(&bare, &star_field(1.0), None);
        insert(&g, &star_field(0.8), None);
        insert(&b, &star_field(0.6), None);

        let mut errors = Vec::new();
        for r in [&unsolved, &bare] {
            errors.push(
                spcc_calibrate_cmd(r.clone(), g.clone(), b.clone(), None, None, Some(5.0), None, Some("builtin".to_string()), None)
                    .await
                    .unwrap_err(),
            );
        }
        for key in [&unsolved, &bare, &g, &b] {
            GLOBAL_IMAGE_CACHE.remove(key);
        }
        assert!(errors[0].contains("has no usable celestial WCS ("), "{}", errors[0]);
        for err in errors {
            assert!(err.ends_with(EXPECTED_HINT), "{err}");
            assert!(!err.contains("SPCC needs a plate-solved image"), "{err}");
        }
    }

    fn sized(mut header: HduHeader, cols: usize, rows: usize) -> HduHeader {
        header.set("NAXIS1", cols.to_string());
        header.set("NAXIS2", rows.to_string());
        header
    }

    fn cropped(left: f64, top: f64) -> HduHeader {
        let mut header = HduHeader::empty();
        header.set_f64("LTV1", -left);
        header.set_f64("LTV2", -top);
        header
    }

    #[tokio::test]
    async fn a_cropped_r_channel_shifts_the_solved_wcs() {
        let mut solved = sized(solved_header(), 64, 64);
        solved.set_f64("CRPIX1", 20.0);
        solved.set_f64("CRPIX2", 30.0);

        let rect = wcs_grid_rect(Some(&cropped(3.0, 2.0)), (40, 50), &solved).unwrap();
        assert_eq!(rect, CutoutRect { x0: 3, y0: 2, width: 50, height: 40 });
        let shifted = shift_header(&solved, &rect);
        assert_eq!((shifted.get_f64("CRPIX1"), shifted.get_f64("CRPIX2")), (Some(17.0), Some(28.0)));
        assert_eq!(wcs_grid_rect(None, (64, 64), &solved).unwrap(), CutoutRect { x0: 0, y0: 0, width: 64, height: 64 });

        let too_big = wcs_grid_rect(Some(&cropped(3.0, 2.0)), (64, 64), &solved).unwrap_err().to_string();
        assert_eq!(
            too_big,
            "The R channel (64x64, offset 3,2) does not fit the plate-solved image (64x64); plate solve the R channel's reference frame again."
        );
        let other_grid = wcs_grid_rect(None, (60, 64), &solved).unwrap_err().to_string();
        assert_eq!(
            other_grid,
            "The R channel (64x60, offset 0,0) does not fit the plate-solved image (64x64); plate solve the R channel's reference frame again."
        );

        let _wizard = lock_wizard_entries();
        let r = wizard_bg_key("spcc_cropped_r");
        let wcs = wizard_bg_key("spcc_solved_full");
        insert(&r, &Array2::from_elem((40, 50), 1.0f32), Some(cropped(3.0, 2.0)));
        insert(&wcs, &Array2::from_elem((64, 64), 1.0f32), Some(solved.clone()));
        let header = load_from_cache_or_disk(&r).and_then(|entry| spcc_header(&r, &entry, Some(wcs.as_str())));
        GLOBAL_IMAGE_CACHE.remove(&r);
        GLOBAL_IMAGE_CACHE.remove(&wcs);
        let (source, header) = header.unwrap();
        assert_eq!(source, wcs);
        assert_eq!((header.get_f64("CRPIX1"), header.get_f64("CRPIX2")), (Some(17.0), Some(28.0)));
    }
}
