use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached_full, resolve_output_dir, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::imaging::debayer::{debayer_bilinear, debayer_superpixel, BayerPattern};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::infra::fits::writer::write_fits_mono;
use crate::types::constants::{RES_DIMENSIONS, RES_ELAPSED_MS, RES_PNG_PATH};
use crate::types::header::HduHeader;

fn output_header(source: Option<&HduHeader>, superpixel: bool) -> Option<HduHeader> {
    let mut h = source?.clone();
    h.remove("BAYERPAT");
    h.remove("XBAYROFF");
    h.remove("YBAYROFF");
    h.remove("COLORTYP");
    if superpixel {
        for key in ["CRPIX1", "CRPIX2"] {
            if let Some(v) = h.get_f64(key) {
                h.set_f64(key, (v - 0.5) / 2.0 + 0.5);
            }
        }
        for key in ["CD1_1", "CD1_2", "CD2_1", "CD2_2", "CDELT1", "CDELT2"] {
            if let Some(v) = h.get_f64(key) {
                h.set_f64(key, v * 2.0);
            }
        }
    }
    Some(h)
}

fn file_stem(path: &str) -> &str {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("frame")
}

fn resolve_pattern(
    requested: Option<&str>,
    header: Option<&HduHeader>,
) -> anyhow::Result<BayerPattern> {
    if let Some(p) = requested {
        if let Some(pat) = BayerPattern::parse(p) {
            return Ok(pat);
        }
        anyhow::bail!("Unknown Bayer pattern '{}'. Use RGGB, BGGR, GRBG or GBRG.", p);
    }
    header
        .and_then(BayerPattern::detect)
        .ok_or_else(|| anyhow::anyhow!(
            "No Bayer pattern found in the FITS header (BAYERPAT). Select one manually."
        ))
}

fn debayer_one(
    path: &str,
    output_dir: &str,
    superpixel: bool,
    pattern: BayerPattern,
) -> anyhow::Result<(String, String, String, (usize, usize))> {
    let entry = load_cached_full(path)?;
    let (r, g, b) = if superpixel {
        debayer_superpixel(entry.arr(), pattern)
    } else {
        debayer_bilinear(entry.arr(), pattern)
    };

    let hdr = output_header(entry.header(), superpixel);
    let stem = file_stem(path);
    let r_path = format!("{}/{}_R.fits", output_dir, stem);
    let g_path = format!("{}/{}_G.fits", output_dir, stem);
    let b_path = format!("{}/{}_B.fits", output_dir, stem);

    write_fits_mono(&r_path, &r, hdr.as_ref())?;
    write_fits_mono(&g_path, &g, hdr.as_ref())?;
    write_fits_mono(&b_path, &b, hdr.as_ref())?;

    Ok((r_path, g_path, b_path, r.dim()))
}

#[tauri::command]
pub async fn debayer_fits_cmd(
    path: String,
    output_dir: String,
    method: Option<String>,
    pattern: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let out_dir = resolve_output_dir(&output_dir)?;

        let entry = load_cached_full(&path)?;
        let pat = resolve_pattern(pattern.as_deref(), entry.header())?;
        let superpixel = matches!(method.as_deref(), Some("superpixel"));

        let (r, g, b) = if superpixel {
            debayer_superpixel(entry.arr(), pat)
        } else {
            debayer_bilinear(entry.arr(), pat)
        };
        let (rows, cols) = r.dim();

        let hdr = output_header(entry.header(), superpixel);
        let stem = file_stem(&path);
        let r_path = format!("{}/{}_R.fits", out_dir, stem);
        let g_path = format!("{}/{}_G.fits", out_dir, stem);
        let b_path = format!("{}/{}_B.fits", out_dir, stem);

        write_fits_mono(&r_path, &r, hdr.as_ref())?;
        write_fits_mono(&g_path, &g, hdr.as_ref())?;
        write_fits_mono(&b_path, &b, hdr.as_ref())?;

        let (stats_r, (stats_g, stats_b)) = rayon::join(
            || compute_image_stats(&r),
            || rayon::join(|| compute_image_stats(&g), || compute_image_stats(&b)),
        );
        let stf_config = AutoStfConfig::default();
        let (linked, combined) =
            helpers::compute_linked_stf_with_stats(&stats_r, &stats_g, &stats_b, &stf_config);
        let fn_r = make_stf_u8_fn(&linked, &combined);
        let fn_g = make_stf_u8_fn(&linked, &combined);
        let fn_b = make_stf_u8_fn(&linked, &combined);
        let png_path = format!("{}/{}_debayer.png", out_dir, stem);
        helpers::render_rgb_preview_with_stf(&r, &g, &b, fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            "pattern": pat.name(),
            "method": if superpixel { "superpixel" } else { "bilinear" },
            "r_path": r_path,
            "g_path": g_path,
            "b_path": b_path,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn debayer_batch_cmd(
    paths: Vec<String>,
    output_dir: String,
    method: Option<String>,
    pattern: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let out_dir = resolve_output_dir(&output_dir)?;
        let superpixel = matches!(method.as_deref(), Some("superpixel"));

        let mut results = Vec::with_capacity(paths.len());
        let mut ok_count = 0usize;

        for path in &paths {
            let item = load_cached_full(path)
                .map_err(anyhow::Error::from)
                .and_then(|entry| {
                    let pat = resolve_pattern(pattern.as_deref(), entry.header())?;
                    drop(entry);
                    let (rp, gp, bp, dims) = debayer_one(path, &out_dir, superpixel, pat)?;
                    Ok((pat, rp, gp, bp, dims))
                });

            match item {
                Ok((pat, rp, gp, bp, (rows, cols))) => {
                    ok_count += 1;
                    results.push(json!({
                        "path": path,
                        "pattern": pat.name(),
                        "r_path": rp,
                        "g_path": gp,
                        "b_path": bp,
                        RES_DIMENSIONS: [cols, rows],
                    }));
                }
                Err(e) => {
                    results.push(json!({
                        "path": path,
                        "error": format!("{:#}", e),
                    }));
                }
            }
        }

        Ok(json!({
            "results": results,
            "succeeded": ok_count,
            "failed": paths.len() - ok_count,
            "method": if superpixel { "superpixel" } else { "bilinear" },
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}
