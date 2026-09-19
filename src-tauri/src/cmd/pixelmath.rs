use std::time::Instant;

use anyhow::anyhow;
use ndarray::Array2;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cmd::common::{
    blocking_cmd, load_cached, load_cached_full, output_stem, render_and_save, resolve_output_dir,
};
use crate::core::imaging::stats::finite_slice_stats;
use crate::core::pixelmath::{compile, evaluate, validate, OutputOptions, PixelMathError};
use crate::infra::cache::ImageEntry;
use crate::infra::fits::writer::{filter_header, write_fits_mono};
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH, RES_STATS,
};
use crate::types::header::HduHeader;
use crate::types::image::ImageStats;

pub const RES_NON_FINITE_COUNT: &str = "non_finite_count";
pub const DEFAULT_PIXELMATH_SUFFIX: &str = "pixelmath";
pub const TARGET_SYMBOL: &str = "$T";

const HEADER_ABPROC: &str = "ABPROC";
const HEADER_PMEXPR: &str = "PMEXPR";
const STRUCTURAL_CARDS: &[&str] = &[
    "XTENSION", "EXTNAME", "EXTVER", "PCOUNT", "GCOUNT", "EXTEND", "NAXIS3", "CHECKSUM", "DATASUM",
];

const KEY_OK: &str = "ok";
const KEY_MESSAGE: &str = "message";
const KEY_POSITION: &str = "position";
const KEY_LENGTH: &str = "length";

#[derive(Debug, Clone, Deserialize)]
pub struct PixelMathSlot {
    pub name: String,
    pub path: String,
}

fn pixelmath_error(err: PixelMathError) -> anyhow::Error {
    anyhow!("pixelmath: {}", err)
}

pub(crate) fn output_suffix(name: Option<&str>) -> String {
    let cleaned: String = name
        .unwrap_or("")
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if cleaned.is_empty() {
        DEFAULT_PIXELMATH_SUFFIX.to_string()
    } else {
        cleaned
    }
}

pub(crate) fn output_header(source: Option<&HduHeader>, expression: &str) -> HduHeader {
    let mut header = source
        .and_then(|h| filter_header(h, true, true))
        .unwrap_or_else(HduHeader::empty);
    for key in STRUCTURAL_CARDS {
        header.remove(key);
    }
    header.set(HEADER_ABPROC, DEFAULT_PIXELMATH_SUFFIX.to_string());
    header.set(HEADER_PMEXPR, expression.to_string());
    header
}

pub(crate) fn finite_output_stats(arr: &Array2<f32>) -> (ImageStats, usize) {
    let mut finite: Vec<f32> = arr.iter().copied().filter(|v| v.is_finite()).collect();
    let non_finite = arr.len() - finite.len();
    (finite_slice_stats(&mut finite), non_finite)
}

pub(crate) fn slot_names_with_target(names: &[String]) -> Vec<String> {
    std::iter::once(TARGET_SYMBOL.to_string())
        .chain(names.iter().cloned())
        .collect()
}

pub(crate) fn run_pixelmath(
    path: &str,
    output_dir: &str,
    expression: &str,
    slots: &[PixelMathSlot],
    opts: OutputOptions,
    name: Option<&str>,
) -> anyhow::Result<Value> {
    let t0 = Instant::now();
    let output_dir = resolve_output_dir(output_dir)?;
    let target = load_cached_full(path)?;
    let mut entries: Vec<ImageEntry> = Vec::with_capacity(slots.len() + 1);
    entries.push(target.clone());
    for slot in slots {
        entries.push(load_cached(&slot.path)?);
    }
    let user_names: Vec<String> = slots.iter().map(|s| s.name.clone()).collect();
    let slot_names = slot_names_with_target(&user_names);
    let program = compile(expression, &slot_names).map_err(pixelmath_error)?;
    let arrays: Vec<&Array2<f32>> = entries.iter().map(|e| e.arr()).collect();
    let result = evaluate(&program, &arrays, &opts).map_err(pixelmath_error)?;

    let suffix = output_suffix(name);
    let rendered = render_and_save(&result, path, &output_dir, &suffix, false)?;
    let fits_path = format!("{}/{}_{}.fits", output_dir, output_stem(path), suffix);
    let header = output_header(target.header(), expression);
    write_fits_mono(&fits_path, &result, Some(&header))?;

    let (stats, non_finite) = finite_output_stats(&result);
    let (rows, cols) = rendered.dims;
    Ok(json!({
        RES_PNG_PATH: rendered.png_path,
        RES_FITS_PATH: fits_path,
        RES_DIMENSIONS: [cols, rows],
        RES_STATS: stats,
        RES_NON_FINITE_COUNT: non_finite,
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
    }))
}

pub(crate) fn validation_json(expression: &str, slot_names: &[String]) -> Value {
    match validate(expression, &slot_names_with_target(slot_names)) {
        Ok(()) => json!({
            KEY_OK: true,
            KEY_MESSAGE: Value::Null,
            KEY_POSITION: Value::Null,
            KEY_LENGTH: Value::Null,
        }),
        Err(err) => json!({
            KEY_OK: false,
            KEY_MESSAGE: err.message,
            KEY_POSITION: err.position,
            KEY_LENGTH: err.length,
        }),
    }
}

#[tauri::command]
pub async fn pixelmath_cmd(
    path: String,
    output_dir: String,
    expression: String,
    slots: Vec<PixelMathSlot>,
    truncate: Option<bool>,
    rescale: Option<bool>,
    name: Option<String>,
) -> Result<Value, String> {
    blocking_cmd!({
        let opts = OutputOptions {
            truncate: truncate.unwrap_or(false),
            rescale: rescale.unwrap_or(false),
        };
        run_pixelmath(&path, &output_dir, &expression, &slots, opts, name.as_deref())
    })
}

#[tauri::command]
pub async fn pixelmath_validate_cmd(
    expression: String,
    slot_names: Vec<String>,
) -> Result<Value, String> {
    Ok(validation_json(&expression, &slot_names))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;
    use crate::types::constants::{RES_MAX, RES_MIN};

    struct Fixture {
        _dir: tempfile::TempDir,
        target: String,
        err_plane: String,
        out_dir: String,
    }

    fn fixture(name: &str, size: usize) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mef = dir.path().join(name);
        sci_err_dq_mef(&mef, size, size, vec![0; size * size]);
        let base = mef.to_str().unwrap().to_string();
        let out_dir = dir.path().join("out").to_str().unwrap().to_string();
        Fixture {
            _dir: dir,
            target: format!("{}#hdu=1", base),
            err_plane: format!("{}#hdu=2", base),
            out_dir,
        }
    }

    fn slot(name: &str, path: &str) -> PixelMathSlot {
        PixelMathSlot { name: name.into(), path: path.into() }
    }

    #[tokio::test]
    async fn pixelmath_cmd_writes_fits_with_copied_header_and_finite_stats() {
        let f = fixture("pm_src.fits", 4);
        let res = pixelmath_cmd(
            f.target.clone(),
            f.out_dir.clone(),
            "$T * 2 + A".into(),
            vec![slot("A", &f.err_plane)],
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(res[RES_DIMENSIONS], json!([4, 4]));
        assert_eq!(res[RES_NON_FINITE_COUNT], 0);
        assert_eq!(res[RES_STATS][RES_MIN], 0.0);
        assert_eq!(res[RES_STATS][RES_MAX], 37.5);
        assert!(std::path::Path::new(res[RES_PNG_PATH].as_str().unwrap()).exists());
        let fits_path = res[RES_FITS_PATH].as_str().unwrap().to_string();
        assert!(fits_path.ends_with("pm_src_hdu1_pixelmath.fits"), "{fits_path}");

        let written = load_cached_full(&fits_path).unwrap();
        assert_eq!(written.arr().dim(), (4, 4));
        assert_eq!(written.arr()[[1, 1]], 12.5);
        let header = written.header().expect("header");
        assert_eq!(header.get(HEADER_ABPROC), Some("pixelmath"));
        assert_eq!(header.get(HEADER_PMEXPR), Some("$T * 2 + A"));
        assert_eq!(header.get("BUNIT"), Some("MJy/sr"));
        assert!(header.get("EXTNAME").is_none());
        assert!(header.get("XTENSION").is_none());
    }

    #[tokio::test]
    async fn pixelmath_cmd_honours_output_name_truncate_and_rescale() {
        let f = fixture("pm_opts.fits", 4);
        let res = pixelmath_cmd(
            f.target.clone(),
            f.out_dir.clone(),
            "$T / 8".into(),
            vec![],
            Some(true),
            None,
            Some("clamped v1".into()),
        )
        .await
        .unwrap();
        assert!(res[RES_FITS_PATH].as_str().unwrap().ends_with("pm_opts_hdu1_clampedv1.fits"));
        assert_eq!(res[RES_STATS][RES_MAX], 1.0);

        let res = pixelmath_cmd(
            f.target.clone(),
            f.out_dir.clone(),
            "iif($T == 3, 1/0, $T * 100)".into(),
            vec![],
            None,
            Some(true),
            None,
        )
        .await
        .unwrap();
        assert_eq!(res[RES_NON_FINITE_COUNT], 1);
        assert_eq!(res[RES_STATS][RES_MIN], 0.0);
        assert_eq!(res[RES_STATS][RES_MAX], 1.0);
        let written = load_cached_full(res[RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert!(written.arr()[[0, 3]].is_nan());
    }

    #[tokio::test]
    async fn pixelmath_cmd_reports_language_and_dimension_errors() {
        let f = fixture("pm_err.fits", 4);
        let err = pixelmath_cmd(f.target.clone(), f.out_dir.clone(), "$T + * 2".into(), vec![], None, None, None)
            .await
            .unwrap_err();
        assert_eq!(err, "pixelmath: unexpected token '*' at 5");

        let err = pixelmath_cmd(f.target.clone(), f.out_dir.clone(), "$T + B".into(), vec![], None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("unknown symbol 'B'; available: $T"), "{err}");

        let other = fixture("pm_other.fits", 5);
        let err = pixelmath_cmd(
            f.target.clone(),
            f.out_dir.clone(),
            "$T + big".into(),
            vec![slot("big", &other.target)],
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("dimension mismatch"), "{err}");
        assert!(err.contains("$T is 4x4"), "{err}");
        assert!(err.contains("big is 5x5"), "{err}");
        assert!(!std::path::Path::new(&f.out_dir).join("pm_err_hdu1_pixelmath.fits").exists());
    }

    #[tokio::test]
    async fn validate_cmd_reports_ok_and_positioned_errors() {
        let ok = pixelmath_validate_cmd("$T - med($T) + A".into(), vec!["A".into()]).await.unwrap();
        assert_eq!(ok[KEY_OK], true);
        assert!(ok[KEY_MESSAGE].is_null());

        let bad = pixelmath_validate_cmd("1 + * 2".into(), vec![]).await.unwrap();
        assert_eq!(bad[KEY_OK], false);
        assert_eq!(bad[KEY_POSITION], 4);
        assert_eq!(bad[KEY_LENGTH], 1);
        assert!(bad[KEY_MESSAGE].as_str().unwrap().contains("unexpected token '*'"));

        let unknown = pixelmath_validate_cmd("$T + Q".into(), vec!["A".into()]).await.unwrap();
        assert_eq!(unknown[KEY_OK], false);
        assert!(unknown[KEY_MESSAGE].as_str().unwrap().contains("available: $T, A"));
    }

    #[test]
    fn output_suffix_sanitizes_names() {
        assert_eq!(output_suffix(None), "pixelmath");
        assert_eq!(output_suffix(Some("   ")), "pixelmath");
        assert_eq!(output_suffix(Some("ha/oiii ratio")), "haoiiiratio");
        assert_eq!(output_suffix(Some("ratio_v2-final")), "ratio_v2-final");
    }

    #[test]
    fn output_header_without_source_still_carries_provenance() {
        let header = output_header(None, "~$T");
        assert_eq!(header.get(HEADER_ABPROC), Some("pixelmath"));
        assert_eq!(header.get(HEADER_PMEXPR), Some("~$T"));
        let mut source = HduHeader::empty();
        source.set("EXTNAME", "DQ".into());
        source.set("CRVAL1", "10.5".into());
        let copied = output_header(Some(&source), "$T");
        assert!(copied.get("EXTNAME").is_none());
        assert_eq!(copied.get("CRVAL1"), Some("10.5"));
    }

    #[test]
    fn finite_output_stats_ignore_non_finite_and_keep_tiny_values() {
        let arr = Array2::from_shape_vec((2, 2), vec![f32::NAN, 1e-9, f32::INFINITY, 2.0]).unwrap();
        let (stats, non_finite) = finite_output_stats(&arr);
        assert_eq!(non_finite, 2);
        assert_eq!(stats.valid_count, 2);
        assert_eq!(stats.min, 1e-9f32 as f64);
        assert_eq!(stats.max, 2.0);
    }
}
