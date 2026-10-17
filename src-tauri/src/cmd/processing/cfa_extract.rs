use std::time::Instant;

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, load_cached_full, output_stem, resolve_output_dir,
    save_output_preview, write_derived_fits,
};
use crate::core::imaging::cfa::{
    extract_cfa, extraction_pattern, scaled_header, CfaExtract, CfaExtracted, CfaPattern, CfaPlane,
};
use crate::core::imaging::push_history;
use crate::core::ops::OutputValues;
use crate::types::constants::{
    ABPROC_CFA_EXTRACT, HEADER_CFA_EXTRACT_CARD, RES_DIMENSIONS, RES_ELAPSED_MS, RES_ERROR,
    RES_EXTRACT_MODE, RES_FAILED, RES_FILTER, RES_FITS_PATH, RES_OFFSET, RES_OIII_SCALE, RES_PATH,
    RES_PATHS, RES_PATTERN, RES_PLANES, RES_PLANE_TAG, RES_PNG_PATH, RES_PROVENANCE, RES_RESULTS,
    RES_SCALE, RES_SUCCEEDED, RES_TRIMMED,
};
use crate::types::header::HduHeader;

const FILTER_CARD: &str = "FILTER";

struct WrittenPlane {
    tag: &'static str,
    filter: Option<&'static str>,
    provenance: &'static str,
    fits_path: String,
}

struct Extraction {
    pattern: CfaPattern,
    planes: Vec<WrittenPlane>,
    dims: (usize, usize),
    trimmed: bool,
    oiii_scale: Option<(f64, f64)>,
    preview_path: Option<String>,
}

fn parse_mode(mode: &str) -> Result<CfaExtract> {
    CfaExtract::parse(mode).ok_or_else(|| {
        anyhow!("Unknown CFA extraction mode '{mode}'. Use green, ha, ha_oiii or split.")
    })
}

fn extraction_tag(path: &str, mode: CfaExtract) -> String {
    format!("{}_{}", output_stem(path), mode.name())
}

fn set_text(header: &mut HduHeader, key: &str, value: &str) {
    header.set(key, value.to_string());
    if let Some(keys) = header.string_keys.as_mut() {
        keys.insert(key.to_string());
    }
}

fn plane_header(
    source: Option<&HduHeader>,
    plane: &CfaPlane,
    mode: CfaExtract,
    pattern: &CfaPattern,
    oiii_scale: Option<(f64, f64)>,
) -> HduHeader {
    let derived = derived_output_header(source, ABPROC_CFA_EXTRACT, OutputValues::Linear);
    let (rows, cols) = plane.data.dim();
    let mut header = scaled_header(&derived, (rows * 2, cols * 2), (rows, cols));
    set_text(&mut header, HEADER_CFA_EXTRACT_CARD, plane.provenance);
    match plane.filter {
        Some(filter) => set_text(&mut header, FILTER_CARD, filter),
        None => header.remove(FILTER_CARD),
    }
    push_history(
        &mut header,
        &format!("cfa extract {} from {}", mode.name(), pattern.name()),
    );
    if let ("OIII", Some((scale, offset))) = (plane.tag, oiii_scale) {
        push_history(
            &mut header,
            &format!("oiii scale k={scale:.6}, c={offset:.6}"),
        );
    }
    header
}

fn write_planes(
    path: &str,
    out_dir: &str,
    mode: CfaExtract,
    pattern: &CfaPattern,
    source: Option<&HduHeader>,
    extracted: &CfaExtracted,
) -> Result<Vec<WrittenPlane>> {
    let tag = extraction_tag(path, mode);
    extracted
        .planes
        .iter()
        .map(|plane| {
            let fits_path = format!("{}/{}_{}.fits", out_dir, tag, plane.tag);
            let header = plane_header(source, plane, mode, pattern, extracted.oiii_scale);
            write_derived_fits(&fits_path, &plane.data, Some(&header))?;
            Ok(WrittenPlane {
                tag: plane.tag,
                filter: plane.filter,
                provenance: plane.provenance,
                fits_path,
            })
        })
        .collect()
}

fn extract_one(
    path: &str,
    out_dir: &str,
    mode: CfaExtract,
    requested_pattern: Option<&str>,
    with_preview: bool,
) -> Result<Extraction> {
    let entry = load_cached_full(path)?;
    let pattern = extraction_pattern(requested_pattern, entry.header())?;
    let extracted = extract_cfa(entry.arr(), &pattern, mode)?;
    let planes = write_planes(path, out_dir, mode, &pattern, entry.header(), &extracted)?;
    let first = extracted
        .planes
        .first()
        .ok_or_else(|| anyhow!("CFA extraction produced no plane"))?;
    let preview_path = if with_preview {
        let png_path = format!("{}/{}_extract.png", out_dir, extraction_tag(path, mode));
        save_output_preview(&first.data, OutputValues::Linear, &png_path)?;
        Some(png_path)
    } else {
        None
    };
    Ok(Extraction {
        pattern,
        planes,
        dims: first.data.dim(),
        trimmed: extracted.trimmed,
        oiii_scale: extracted.oiii_scale,
        preview_path,
    })
}

fn extraction_json(path: &str, extraction: &Extraction) -> serde_json::Value {
    let (rows, cols) = extraction.dims;
    let planes: Vec<serde_json::Value> = extraction
        .planes
        .iter()
        .map(|plane| {
            json!({
                RES_PLANE_TAG: plane.tag,
                RES_FILTER: plane.filter,
                RES_PROVENANCE: plane.provenance,
                RES_FITS_PATH: plane.fits_path,
            })
        })
        .collect();
    let paths: Vec<&str> = extraction
        .planes
        .iter()
        .map(|plane| plane.fits_path.as_str())
        .collect();
    json!({
        RES_PATH: path,
        RES_PATTERN: extraction.pattern.name(),
        RES_PATHS: paths,
        RES_PLANES: planes,
        RES_DIMENSIONS: [cols, rows],
        RES_TRIMMED: extraction.trimmed,
        RES_OIII_SCALE: extraction
            .oiii_scale
            .map(|(scale, offset)| json!({ RES_SCALE: scale, RES_OFFSET: offset })),
    })
}

#[tauri::command]
pub async fn extract_cfa_cmd(
    path: String,
    output_dir: String,
    mode: String,
    pattern: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let mode = parse_mode(&mode)?;
        let out_dir = resolve_output_dir(&output_dir)?;
        let extraction = extract_one(&path, &out_dir, mode, pattern.as_deref(), true)?;
        let mut result = extraction_json(&path, &extraction);
        if let Some(object) = result.as_object_mut() {
            object.insert(RES_PNG_PATH.to_string(), json!(extraction.preview_path));
            object.insert(RES_EXTRACT_MODE.to_string(), json!(mode.name()));
            object.insert(
                RES_ELAPSED_MS.to_string(),
                json!(t0.elapsed().as_millis() as u64),
            );
        }
        Ok(result)
    })
}

#[tauri::command]
pub async fn extract_cfa_batch_cmd(
    paths: Vec<String>,
    output_dir: String,
    mode: String,
    pattern: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let mode = parse_mode(&mode)?;
        let out_dir = resolve_output_dir(&output_dir)?;
        let mut results = Vec::with_capacity(paths.len());
        let mut ok_count = 0usize;
        for path in &paths {
            match extract_one(path, &out_dir, mode, pattern.as_deref(), false) {
                Ok(extraction) => {
                    ok_count += 1;
                    results.push(extraction_json(path, &extraction));
                }
                Err(error) => results.push(json!({
                    RES_PATH: path,
                    RES_ERROR: format!("{:#}", error),
                })),
            }
        }
        Ok(json!({
            RES_RESULTS: results,
            RES_SUCCEEDED: ok_count,
            RES_FAILED: paths.len() - ok_count,
            RES_EXTRACT_MODE: mode.name(),
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use ndarray::Array2;

    use super::*;
    use crate::cmd::common::extract_image_resolved;
    use crate::core::imaging::debayer::BayerPattern;
    use crate::core::imaging::resample::compute_wcs_updates;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::header::HduHeader;

    const XTRANS_LAYOUT: &str = "GBGGRGRGRBGBGBGGRGGRGGBGBGBRGRGRGGBG";

    fn osc_header(pattern: &str) -> HduHeader {
        let mut header = HduHeader::empty();
        for (key, value) in [
            ("BAYERPAT", format!("'{pattern}'")),
            ("CTYPE1", "'RA---TAN-SIP'".into()),
            ("CTYPE2", "'DEC--TAN-SIP'".into()),
            ("CRVAL1", "83.8".into()),
            ("CRVAL2", "-5.4".into()),
            ("CRPIX1", "8.5".into()),
            ("CRPIX2", "8.5".into()),
            ("CD1_1", "1.0E-4".into()),
            ("CD1_2", "0.0".into()),
            ("CD2_1", "0.0".into()),
            ("CD2_2", "1.0E-4".into()),
            ("A_ORDER", "2".into()),
            ("A_2_0", "1.0E-7".into()),
            ("B_ORDER", "2".into()),
            ("B_0_2", "1.0E-7".into()),
            ("LTV1", "10".into()),
            ("FILTER", "'L'".into()),
        ] {
            header.set(key, value);
        }
        header
    }

    fn rggb_mosaic(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            let level = (7 * (y / 2) + x / 2) as f32;
            match BayerPattern::Rggb.colour_at(y, x) {
                0 => 300.0 + level,
                1 => 1000.0 + 5.0 * level,
                _ => 400.0 + 2.0 * level,
            }
        })
    }

    fn write_source(
        dir: &tempfile::TempDir,
        name: &str,
        data: &Array2<f32>,
        header: Option<&HduHeader>,
    ) -> String {
        let path = dir.path().join(name).to_str().unwrap().to_string();
        write_fits_mono(&path, data, header).unwrap();
        path
    }

    fn text(header: &HduHeader, key: &str) -> Option<String> {
        header
            .get(key)
            .map(|v| v.trim().trim_matches('\'').trim().to_string())
    }

    fn histories(header: &HduHeader) -> Vec<String> {
        header
            .cards
            .iter()
            .filter(|(k, _)| k.trim() == "HISTORY")
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn plane_paths(result: &serde_json::Value) -> Vec<String> {
        result[RES_PATHS]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn ha_oiii_writes_two_scaled_planes_with_the_provenance_cards() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let header = osc_header("RGGB");
        let src = write_source(&dir, "M42.fits", &rggb_mosaic(16, 16), Some(&header));

        let result = extract_cfa_cmd(src, out, "ha_oiii".into(), None)
            .await
            .unwrap();
        let paths = plane_paths(&result);
        assert_eq!(paths.len(), 2, "{result}");
        assert!(paths[0].ends_with("M42_ha_oiii_Ha.fits"), "{}", paths[0]);
        assert!(paths[1].ends_with("M42_ha_oiii_OIII.fits"), "{}", paths[1]);
        assert_eq!(result[RES_PATTERN], "RGGB");
        assert_eq!(result[RES_EXTRACT_MODE], "ha_oiii");
        assert_eq!(result[RES_DIMENSIONS], serde_json::json!([8, 8]));
        assert!(
            result[RES_OIII_SCALE][RES_SCALE]
                .as_f64()
                .is_some_and(f64::is_finite),
            "{result}"
        );
        assert!(
            result[RES_PNG_PATH]
                .as_str()
                .is_some_and(|p| std::path::Path::new(p).is_file()),
            "{result}"
        );

        let expected: std::collections::HashMap<String, f64> =
            compute_wcs_updates(&header, (16, 16), (8, 8))
                .into_iter()
                .collect();
        for (path, filter, provenance) in [(&paths[0], "Ha", "ha"), (&paths[1], "OIII", "oiii")] {
            let written = extract_image_resolved(path).unwrap();
            assert_eq!(written.arr.dim(), (8, 8));
            let h = &written.header;
            assert_eq!(h.get_f64("CRPIX1"), Some(4.5));
            assert_eq!(h.get_f64("CD1_1"), Some(2.0e-4));
            assert_eq!(h.get_f64("A_2_0"), expected.get("A_2_0").copied());
            assert_eq!(h.get_f64("LTV1"), expected.get("LTV1").copied());
            assert_eq!(
                (h.get_i64("XBINNING"), h.get_i64("YBINNING")),
                (Some(2), Some(2))
            );
            assert_eq!(text(h, "FILTER").as_deref(), Some(filter));
            assert_eq!(text(h, "ABPROC").as_deref(), Some("cfa_extract"));
            assert_eq!(text(h, "ABCFAEXT").as_deref(), Some(provenance));
            assert!(h.get("BAYERPAT").is_none() && h.get("COLORTYP").is_none());
            assert!(
                histories(h)
                    .iter()
                    .any(|line| line.contains("cfa extract ha_oiii from RGGB")),
                "{:?}",
                histories(h)
            );
        }
        let oiii = extract_image_resolved(&paths[1]).unwrap();
        assert!(
            histories(&oiii.header)
                .iter()
                .any(|line| line.contains("oiii scale k=") && line.contains("c=")),
            "{:?}",
            histories(&oiii.header)
        );
        let ha = extract_image_resolved(&paths[0]).unwrap();
        assert!(!histories(&ha.header)
            .iter()
            .any(|line| line.contains("oiii scale")));
        assert_eq!(ha.arr[[1, 2]], 300.0 + (7 + 2) as f32);
    }

    #[tokio::test]
    async fn green_mode_writes_the_card_that_c01_reads() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = write_source(
            &dir,
            "V1405.fits",
            &rggb_mosaic(10, 12),
            Some(&osc_header("RGGB")),
        );

        let result = extract_cfa_cmd(src, out, "green".into(), None)
            .await
            .unwrap();
        let paths = plane_paths(&result);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("V1405_green_G.fits"), "{}", paths[0]);
        let written = extract_image_resolved(&paths[0]).unwrap();
        assert_eq!(written.arr.dim(), (5, 6));
        assert_eq!(text(&written.header, "ABCFAEXT").as_deref(), Some("green"));
        assert_eq!(text(&written.header, "FILTER").as_deref(), Some("G"));
        assert_eq!(written.arr[[0, 0]], 1000.0);
    }

    #[tokio::test]
    async fn split_mode_writes_four_planes_without_a_filter_card() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = write_source(
            &dir,
            "osc.fits",
            &rggb_mosaic(9, 8),
            Some(&osc_header("RGGB")),
        );

        let result = extract_cfa_cmd(src, out, "split".into(), None)
            .await
            .unwrap();
        let paths = plane_paths(&result);
        assert_eq!(paths.len(), 4);
        assert_eq!(result[RES_TRIMMED], true);
        for (index, path) in paths.iter().enumerate() {
            assert!(
                path.ends_with(&format!("osc_split_CFA{index}.fits")),
                "{path}"
            );
            let written = extract_image_resolved(path).unwrap();
            assert_eq!(written.arr.dim(), (4, 4));
            assert_eq!(
                text(&written.header, "ABCFAEXT"),
                Some(format!("cfa{index}"))
            );
            assert!(written.header.get("FILTER").is_none(), "{path}");
        }
    }

    #[tokio::test]
    async fn a_requested_pattern_overrides_the_header_and_a_mono_frame_is_refused_naming_bayerpat()
    {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let mono = write_source(&dir, "mono.fits", &rggb_mosaic(8, 8), None);

        let refused = extract_cfa_cmd(mono.clone(), out.clone(), "ha".into(), None)
            .await
            .unwrap_err();
        assert!(refused.contains("BAYERPAT"), "{refused}");

        let forced = extract_cfa_cmd(mono, out.clone(), "ha".into(), Some("BGGR".into()))
            .await
            .unwrap();
        assert_eq!(forced[RES_PATTERN], "BGGR");
        let written = extract_image_resolved(plane_paths(&forced)[0].as_str()).unwrap();
        assert_eq!(written.arr[[0, 0]], 400.0);

        let unknown = extract_cfa_cmd(
            write_source(&dir, "a.fits", &rggb_mosaic(8, 8), None),
            out.clone(),
            "warp".into(),
            None,
        )
        .await
        .unwrap_err();
        assert!(unknown.contains("warp"), "{unknown}");
    }

    #[tokio::test]
    async fn an_xtrans_frame_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = write_source(
            &dir,
            "raf.fits",
            &Array2::from_elem((12, 12), 5.0f32),
            Some(&osc_header(XTRANS_LAYOUT)),
        );
        let refused = extract_cfa_cmd(src, out, "green".into(), None)
            .await
            .unwrap_err();
        assert!(refused.contains("X-Trans"), "{refused}");
    }

    #[tokio::test]
    async fn the_batch_names_the_same_files_and_reports_each_failure() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let osc = write_source(
            &dir,
            "M42.fits",
            &rggb_mosaic(16, 16),
            Some(&osc_header("RGGB")),
        );
        let mono = write_source(&dir, "mono.fits", &rggb_mosaic(8, 8), None);

        let single = extract_cfa_cmd(osc.clone(), out.clone(), "ha_oiii".into(), None)
            .await
            .unwrap();
        let batch = extract_cfa_batch_cmd(vec![osc, mono], out, "ha_oiii".into(), None)
            .await
            .unwrap();
        assert_eq!(
            (batch[RES_SUCCEEDED].as_u64(), batch[RES_FAILED].as_u64()),
            (Some(1), Some(1))
        );
        assert_eq!(batch[RES_EXTRACT_MODE], "ha_oiii");
        let results = batch[RES_RESULTS].as_array().unwrap();
        assert_eq!(plane_paths(&results[0]), plane_paths(&single));
        assert!(
            results[1][RES_ERROR]
                .as_str()
                .is_some_and(|e| e.contains("BAYERPAT")),
            "{batch}"
        );
    }
}
