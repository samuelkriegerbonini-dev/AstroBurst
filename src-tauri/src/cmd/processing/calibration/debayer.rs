use std::time::Instant;

use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_cached_full, output_stem, resolve_output_dir, write_derived_fits, MAX_PREVIEW_DIM};
use crate::cmd::helpers;
use crate::core::imaging::cfa::{remove_cfa_cards, scaled_header};
use crate::core::imaging::debayer::{debayer_bilinear, debayer_superpixel, BayerPattern};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{make_stf_u8_fn, AutoStfConfig};
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_PNG_PATH,
    RES_PATTERN, RES_METHOD, RES_R_PATH, RES_G_PATH, RES_B_PATH,
    RES_PATH, RES_ERROR, RES_RESULTS, RES_SUCCEEDED, RES_FAILED,
    METHOD_SUPERPIXEL, METHOD_BILINEAR,
};
use crate::types::header::HduHeader;

fn output_header(source: Option<&HduHeader>, superpixel_dims: Option<(usize, usize)>) -> Option<HduHeader> {
    let source = source?;
    Some(match superpixel_dims {
        Some((rows, cols)) => scaled_header(source, (rows * 2, cols * 2), (rows, cols)),
        None => {
            let mut header = source.clone();
            remove_cfa_cards(&mut header);
            header
        }
    })
}

fn method_name(superpixel: bool) -> &'static str {
    if superpixel { METHOD_SUPERPIXEL } else { METHOD_BILINEAR }
}

fn debayer_tag(path: &str, superpixel: bool, pattern: BayerPattern) -> String {
    format!("{}_{}_{}", output_stem(path), method_name(superpixel), pattern.name())
}

fn channel_paths(output_dir: &str, tag: &str) -> [String; 3] {
    ["R", "G", "B"].map(|c| format!("{}/{}_{}.fits", output_dir, tag, c))
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

    let hdr = output_header(entry.header(), superpixel.then(|| r.dim()));
    let [r_path, g_path, b_path] = channel_paths(output_dir, &debayer_tag(path, superpixel, pattern));

    write_derived_fits(&r_path, &r, hdr.as_ref())?;
    write_derived_fits(&g_path, &g, hdr.as_ref())?;
    write_derived_fits(&b_path, &b, hdr.as_ref())?;

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
        let superpixel = matches!(method.as_deref(), Some(METHOD_SUPERPIXEL));

        let (r, g, b) = if superpixel {
            debayer_superpixel(entry.arr(), pat)
        } else {
            debayer_bilinear(entry.arr(), pat)
        };
        let (rows, cols) = r.dim();

        let hdr = output_header(entry.header(), superpixel.then_some((rows, cols)));
        let tag = debayer_tag(&path, superpixel, pat);
        let [r_path, g_path, b_path] = channel_paths(&out_dir, &tag);

        write_derived_fits(&r_path, &r, hdr.as_ref())?;
        write_derived_fits(&g_path, &g, hdr.as_ref())?;
        write_derived_fits(&b_path, &b, hdr.as_ref())?;

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
        let png_path = format!("{}/{}_debayer.png", out_dir, tag);
        helpers::render_rgb_preview_with_stf(&r, &g, &b, fn_r, fn_g, fn_b, &png_path, MAX_PREVIEW_DIM)?;

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_PATTERN: pat.name(),
            RES_METHOD: method_name(superpixel),
            RES_R_PATH: r_path,
            RES_G_PATH: g_path,
            RES_B_PATH: b_path,
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
        let superpixel = matches!(method.as_deref(), Some(METHOD_SUPERPIXEL));

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
                        RES_PATH: path,
                        RES_PATTERN: pat.name(),
                        RES_R_PATH: rp,
                        RES_G_PATH: gp,
                        RES_B_PATH: bp,
                        RES_DIMENSIONS: [cols, rows],
                    }));
                }
                Err(e) => {
                    results.push(json!({
                        RES_PATH: path,
                        RES_ERROR: format!("{:#}", e),
                    }));
                }
            }
        }

        Ok(json!({
            RES_RESULTS: results,
            RES_SUCCEEDED: ok_count,
            RES_FAILED: paths.len() - ok_count,
            RES_METHOD: method_name(superpixel),
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use ndarray::Array2;

    use super::*;
    use crate::cmd::common::extract_image_resolved;
    use crate::infra::fits::writer::write_fits_mono;

    #[tokio::test]
    async fn superpixel_output_scales_wcs_sip_ltv_and_binning() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("osc.fits").to_str().unwrap().to_string();
        let mut header = HduHeader::empty();
        for (key, value) in [
            ("BAYERPAT", "'RGGB'"),
            ("CTYPE1", "'RA---TAN-SIP'"),
            ("CTYPE2", "'DEC--TAN-SIP'"),
            ("CRVAL1", "83.8"),
            ("CRVAL2", "-5.4"),
            ("CRPIX1", "8.5"),
            ("CRPIX2", "8.5"),
            ("CD1_1", "1.0E-4"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "1.0E-4"),
            ("A_ORDER", "2"),
            ("A_2_0", "1.0E-7"),
            ("B_ORDER", "2"),
            ("B_0_2", "1.0E-7"),
            ("LTV1", "10"),
        ] {
            header.set(key, value.to_string());
        }
        write_fits_mono(&src, &Array2::from_elem((17, 16), 100.0f32), Some(&header)).unwrap();

        let superpixel = debayer_fits_cmd(src.clone(), out.clone(), Some(METHOD_SUPERPIXEL.into()), None).await.unwrap();
        let written = extract_image_resolved(superpixel[RES_R_PATH].as_str().unwrap()).unwrap();
        assert_eq!(written.arr.dim(), (8, 8));
        let h = &written.header;
        assert!(h.get("BAYERPAT").is_none());
        assert_eq!((h.get_f64("CRPIX1"), h.get_f64("CD1_1")), (Some(4.5), Some(2.0e-4)));
        assert_eq!((h.get_f64("A_2_0"), h.get_f64("B_0_2")), (Some(2.0e-7), Some(2.0e-7)));
        assert_eq!(h.get_f64("LTV1"), Some(5.25));
        assert_eq!((h.get_i64("XBINNING"), h.get_i64("YBINNING")), (Some(2), Some(2)));

        let bilinear = debayer_fits_cmd(src, out, None, None).await.unwrap();
        let written = extract_image_resolved(bilinear[RES_R_PATH].as_str().unwrap()).unwrap();
        let h = &written.header;
        assert!(h.get("BAYERPAT").is_none());
        assert_eq!((h.get_f64("CRPIX1"), h.get_f64("CD1_1"), h.get_f64("LTV1")), (Some(8.5), Some(1.0e-4), Some(10.0)));
        assert_eq!(h.get("XBINNING"), None);
    }

    #[tokio::test]
    async fn runs_with_another_method_or_pattern_write_their_own_files() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("M42.fits").to_str().unwrap().to_string();
        write_fits_mono(&src, &Array2::from_shape_fn((8, 8), |(y, x)| (y * 8 + x) as f32 + 10.0), None).unwrap();

        let bilinear = debayer_fits_cmd(src.clone(), out.clone(), None, Some("RGGB".into())).await.unwrap();
        let superpixel = debayer_fits_cmd(src.clone(), out.clone(), Some(METHOD_SUPERPIXEL.into()), Some("RGGB".into())).await.unwrap();
        let other_pattern = debayer_fits_cmd(src.clone(), out.clone(), None, Some("GBRG".into())).await.unwrap();

        for key in [RES_R_PATH, RES_G_PATH, RES_B_PATH, RES_PNG_PATH] {
            assert_ne!(bilinear[key], superpixel[key], "{key}: the super-pixel run replaced the bilinear files");
            assert_ne!(bilinear[key], other_pattern[key], "{key}: a new pattern replaced the previous files");
        }
        let first = bilinear[RES_R_PATH].as_str().unwrap();
        assert!(first.ends_with("M42_bilinear_RGGB_R.fits"), "{first}");
        assert_eq!(extract_image_resolved(first).unwrap().arr.dim(), (8, 8), "the bilinear R plane was overwritten");
        assert_eq!(extract_image_resolved(superpixel[RES_R_PATH].as_str().unwrap()).unwrap().arr.dim(), (4, 4));

        let batch = debayer_batch_cmd(vec![src], out, Some(METHOD_SUPERPIXEL.into()), Some("RGGB".into())).await.unwrap();
        let item = &batch[RES_RESULTS][0];
        for key in [RES_R_PATH, RES_G_PATH, RES_B_PATH] {
            assert_eq!(item[key], superpixel[key], "{key}: batch and single runs name the same output differently");
        }
    }
}
