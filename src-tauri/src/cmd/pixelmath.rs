use std::time::Instant;

use anyhow::{anyhow, Context};
use ndarray::Array2;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cmd::common::{
    auto_stretch_preview, blocking_cmd, cleanup_output_dir_keeping, load_validated_full, output_stem,
    resolve_output_dir, save_preview_png, source_path,
};
use crate::core::imaging::stats::finite_slice_stats;
use crate::core::pixelmath::{compile, evaluate, validate, OutputOptions, PixelMathError};
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::infra::fits::writer::{filter_header, write_fits_mono};
use crate::types::constants::{
    RES_CLEANED_BYTES, RES_CLEANED_FILES, RES_CLEANED_PATHS, RES_DIMENSIONS, RES_ELAPSED_MS,
    RES_FITS_PATH, RES_PNG_PATH, RES_STATS, RES_WARNINGS,
};
use crate::types::constants::PADDING_THRESHOLD;
use crate::types::header::HduHeader;
use crate::types::image::ImageStats;

pub const RES_NON_FINITE_COUNT: &str = "non_finite_count";
pub const DEFAULT_PIXELMATH_SUFFIX: &str = "pixelmath";
pub const TARGET_SYMBOL: &str = "$T";

const HEADER_ABPROC: &str = "ABPROC";
const HEADER_PMEXPR: &str = "PMEXPR";
const HEADER_PMSOURCE: &str = "PMSRC";
const MAX_HEADER_VALUE_BYTES: usize = 67;
const MAX_CONTINUATION_CARDS: usize = 9;
const TRUNCATION_MARKER: &str = "...";
const PREVIEW_FLOOR: f32 = 2.0 * PADDING_THRESHOLD;
const STRUCTURAL_CARDS: &[&str] = &[
    "XTENSION", "EXTNAME", "EXTVER", "PCOUNT", "GCOUNT", "EXTEND", "NAXIS3", "CHECKSUM", "DATASUM",
];
const CUBE_AXIS_CARDS: &[&str] = &[
    "CTYPE3", "CRVAL3", "CRPIX3", "CDELT3", "CUNIT3", "CROTA3", "CD3_3", "CD1_3", "CD2_3", "CD3_1",
    "CD3_2", "PC3_3", "PC1_3", "PC2_3", "PC3_1", "PC3_2", "PS3_0", "PS3_1", "PV3_0", "PV3_1",
];
const RESCALED_VALUE_CARDS: &[&str] = &[
    "BUNIT", "DATAMIN", "DATAMAX", "SATURATE", "SATLEVEL", "SATURATION", "MAXLIN", "MAGZPT",
    "PHOTFLAM", "PHOTPLAM", "PHOTZPT", "PHOTMJSR", "PIXAR_SR", "PIXAR_A2", "ZP", "ZPTMAG",
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
    match err.position {
        Some(position) => anyhow!("pixelmath: {} (position {})", err.message, position),
        None => anyhow!("pixelmath: {}", err.message),
    }
}

fn header_safe(value: &str) -> String {
    let mut cleaned = String::with_capacity(value.len());
    let mut pending_space = false;
    for c in value.chars() {
        let printable = if c.is_ascii_graphic() { Some(c) } else { None };
        match printable {
            Some(c) if c != '\'' => {
                if pending_space && !cleaned.is_empty() {
                    cleaned.push(' ');
                }
                pending_space = false;
                cleaned.push(c);
            }
            _ => pending_space = true,
        }
    }
    cleaned
}

fn chunk_header_value(value: &str) -> Vec<String> {
    let bytes = value.as_bytes();
    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0usize;
    while start < bytes.len() {
        let mut end = (start + MAX_HEADER_VALUE_BYTES).min(bytes.len());
        while end > start + 1 && end < bytes.len() && bytes[end - 1] == b' ' {
            end -= 1;
        }
        chunks.push(value[start..end].to_string());
        start = end;
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    if chunks.len() > MAX_CONTINUATION_CARDS + 1 {
        chunks.truncate(MAX_CONTINUATION_CARDS + 1);
        if let Some(last) = chunks.last_mut() {
            while last.len() + TRUNCATION_MARKER.len() > MAX_HEADER_VALUE_BYTES {
                last.pop();
            }
            last.push_str(TRUNCATION_MARKER);
        }
    }
    chunks
}

fn set_long_value(header: &mut HduHeader, base: &str, value: &str) {
    for (i, chunk) in chunk_header_value(value).into_iter().enumerate() {
        let key = if i == 0 { base.to_string() } else { format!("{}{}", base, i) };
        header.set(&key, chunk);
    }
}

#[cfg(test)]
fn read_long_value(header: &HduHeader, base: &str) -> Option<String> {
    let head = header.get(base)?.to_string();
    let mut full = head;
    for i in 1..=MAX_CONTINUATION_CARDS {
        match header.get(&format!("{}{}", base, i)) {
            Some(chunk) => full.push_str(chunk),
            None => break,
        }
    }
    Some(full)
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

pub(crate) fn output_header(source: Option<&HduHeader>, expression: &str, target: &str) -> HduHeader {
    let mut header = source
        .and_then(|h| filter_header(h, true, true))
        .unwrap_or_else(HduHeader::empty);
    for key in STRUCTURAL_CARDS.iter().chain(CUBE_AXIS_CARDS).chain(RESCALED_VALUE_CARDS) {
        header.remove(key);
    }
    if header.get("WCSAXES").is_some() {
        header.set("WCSAXES", "2".to_string());
    }
    header.set(HEADER_ABPROC, DEFAULT_PIXELMATH_SUFFIX.to_string());

    set_long_value(&mut header, HEADER_PMEXPR, &header_safe(expression));
    set_long_value(&mut header, HEADER_PMSOURCE, &header_safe(target));
    header
}

pub(crate) fn finite_output_stats(arr: &Array2<f32>) -> (Option<ImageStats>, usize) {
    let mut finite: Vec<f32> = arr.iter().copied().filter(|v| v.is_finite()).collect();
    let non_finite = arr.len() - finite.len();
    if finite.is_empty() {
        return (None, non_finite);
    }
    (Some(finite_slice_stats(&mut finite)), non_finite)
}

fn preview_pixels(result: &Array2<f32>) -> Vec<u8> {
    let lo = result
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::INFINITY, f32::min);
    if !lo.is_finite() || lo > PADDING_THRESHOLD {
        return auto_stretch_preview(result);
    }
    let shifted = result.mapv(|v| if v.is_finite() { v - lo + PREVIEW_FLOOR } else { v });
    auto_stretch_preview(&shifted)
}

fn plane_count(entry: &ImageEntry) -> Option<i64> {
    entry
        .header()
        .and_then(|h| h.get_i64("NAXIS3"))
        .filter(|&n| n > 1)
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

    let user_names: Vec<String> = slots.iter().map(|s| s.name.clone()).collect();
    let slot_names = slot_names_with_target(&user_names);
    let program = compile(expression, &slot_names).map_err(pixelmath_error)?;
    let referenced = program.referenced_slots();

    let target = load_validated_full(path)
        .with_context(|| format!("{} = {}", TARGET_SYMBOL, path))?;
    let mut entries: Vec<Option<ImageEntry>> = Vec::with_capacity(slots.len() + 1);
    entries.push(Some(target.clone()));
    for (i, slot) in slots.iter().enumerate() {
        if !referenced[i + 1] {
            entries.push(None);
            continue;
        }
        let entry = load_validated_full(&slot.path)
            .with_context(|| format!("slot {} = {}", slot.name, slot.path))?;
        entries.push(Some(entry));
    }

    let mut warnings: Vec<String> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(entry) = entry else { continue };
        if let Some(planes) = plane_count(entry) {
            warnings.push(format!(
                "{} is plane 1 of {}; the other planes were not read",
                slot_names[i], planes
            ));
        }
        if entry.plane_info().is_some_and(|info| info.is_dq) {
            warnings.push(format!(
                "{} is a data-quality plane read as 32-bit float; flag values above 16777216 lose their low bits, so only zero/non-zero tests are exact",
                slot_names[i]
            ));
        }
    }

    let target_arr = target.arr();
    let arrays: Vec<&Array2<f32>> = entries
        .iter()
        .map(|e| e.as_ref().map(|e| e.arr()).unwrap_or(target_arr))
        .collect();
    let result = evaluate(&program, &arrays, &opts).map_err(pixelmath_error)?;

    let suffix = output_suffix(name);
    let stem = output_stem(path);
    let fits_path = format!("{}/{}_{}.fits", output_dir, stem, suffix);
    let png_path = format!("{}/{}_{}.png", output_dir, stem, suffix);
    let header = output_header(target.header(), expression, &source_path(path));

    write_fits_mono(&fits_path, &result, Some(&header))?;
    GLOBAL_IMAGE_CACHE.invalidate(&fits_path);

    let (rows, cols) = result.dim();
    save_preview_png(preview_pixels(&result), cols, rows, &png_path)?;

    let mut keep_owned: Vec<String> = vec![source_path(path), fits_path.clone(), png_path.clone()];
    keep_owned.extend(slots.iter().map(|s| source_path(&s.path)));
    let keep: Vec<&str> = keep_owned.iter().map(String::as_str).collect();
    let (cleaned_files, cleaned_bytes, cleaned_paths) = cleanup_output_dir_keeping(&output_dir, &keep);

    let (stats, non_finite) = finite_output_stats(&result);
    Ok(json!({
        RES_PNG_PATH: png_path,
        RES_FITS_PATH: fits_path,
        RES_DIMENSIONS: [cols, rows],
        RES_STATS: stats,
        RES_NON_FINITE_COUNT: non_finite,
        RES_WARNINGS: warnings,
        RES_CLEANED_FILES: cleaned_files,
        RES_CLEANED_BYTES: cleaned_bytes,
        RES_CLEANED_PATHS: cleaned_paths,
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
    use crate::cmd::common::load_cached_full;
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
        assert!(header.get("BUNIT").is_none(), "BUNIT no longer describes the pixels");
        assert!(header.get("EXTNAME").is_none());
        assert!(header.get("XTENSION").is_none());
        let full_source = read_long_value(header, HEADER_PMSOURCE).expect("source recorded");
        assert!(
            full_source.ends_with("pm_src.fits"),
            "reassembled source path does not end with pm_src.fits: {full_source}"
        );
        assert_eq!(full_source, source_path(&f.target));
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
        assert_eq!(err, "pixelmath: unexpected token '*' (position 5)");

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
    async fn slots_the_expression_never_references_do_not_constrain_the_run() {
        let f = fixture("pm_unused.fits", 4);
        let other = fixture("pm_unused_other.fits", 5);
        let res = pixelmath_cmd(
            f.target.clone(),
            f.out_dir.clone(),
            "$T * 2".into(),
            vec![slot("A", &other.target), slot("B", "C:/does/not/exist.fits")],
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(res[RES_DIMENSIONS], json!([4, 4]));
        assert_eq!(
            pixelmath_validate_cmd("$T * 2".into(), vec!["A".into(), "B".into()])
                .await
                .unwrap()[KEY_OK],
            true,
            "validation and the run must agree"
        );
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
        let header = output_header(None, "~$T", "C:/data/m42.fits");
        assert_eq!(header.get(HEADER_ABPROC), Some("pixelmath"));
        assert_eq!(header.get(HEADER_PMEXPR), Some("~$T"));
        assert_eq!(header.get(HEADER_PMSOURCE), Some("C:/data/m42.fits"));
        let mut source = HduHeader::empty();
        source.set("EXTNAME", "DQ".into());
        source.set("CRVAL1", "10.5".into());
        let copied = output_header(Some(&source), "$T", "x.fits");
        assert!(copied.get("EXTNAME").is_none());
        assert_eq!(copied.get("CRVAL1"), Some("10.5"));
    }

    #[test]
    fn output_header_drops_cards_that_no_longer_describe_the_pixels() {
        let mut source = HduHeader::empty();
        source.set("BUNIT", "MJy/sr".into());
        source.set("PIXAR_SR", "2.1E-13".into());
        source.set("SATURATE", "60000".into());
        source.set("WCSAXES", "3".into());
        source.set("CTYPE3", "WAVE".into());
        source.set("CRVAL3", "1.5".into());
        source.set("CRVAL1", "83.8".into());
        source.set("EXPTIME", "300".into());
        let out = output_header(Some(&source), "$T * 2", "cube.fits");
        for dropped in ["BUNIT", "PIXAR_SR", "SATURATE", "CTYPE3", "CRVAL3"] {
            assert!(out.get(dropped).is_none(), "{dropped} survived");
        }
        assert_eq!(out.get("WCSAXES"), Some("2"));
        assert_eq!(out.get("CRVAL1"), Some("83.8"));
        assert_eq!(out.get("EXPTIME"), Some("300"));
    }

    #[test]
    fn pmexpr_is_header_safe_and_spills_into_numbered_cards() {
        let header = output_header(None, "$T - med($T)\n + A * 0.5", "x.fits");
        assert_eq!(header.get(HEADER_PMEXPR), Some("$T - med($T) + A * 0.5"));
        assert!(!header.get(HEADER_PMEXPR).unwrap().contains('\n'));

        let long = format!("$T * ({})", vec!["1"; 120].join(" + "));
        let header = output_header(None, &long, "x.fits");
        assert!(header.get(HEADER_PMEXPR).unwrap().len() <= MAX_HEADER_VALUE_BYTES);
        assert_eq!(read_long_value(&header, HEADER_PMEXPR).as_deref(), Some(long.as_str()));
    }

    #[test]
    fn long_provenance_survives_a_real_write_and_read() {
        let long_expr = format!("$T * ({})", vec!["1"; 120].join(" + "));
        let long_source = format!("/{}/deep/pm_src.fits", vec!["some_long_directory"; 4].join("/"));
        assert!(long_expr.len() > MAX_HEADER_VALUE_BYTES && long_source.len() > MAX_HEADER_VALUE_BYTES);

        let header = output_header(None, &long_expr, &long_source);
        assert!(header.get("PMEXPR1").is_some(), "no numbered continuation emitted");
        assert!(
            !header.cards.iter().any(|(k, _)| k == "HISTORY" || k == "COMMENT"),
            "provenance must not depend on HISTORY: reader.rs:372 skips cards without '= '"
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prov.fits").to_str().unwrap().to_string();
        write_fits_mono(&path, &Array2::<f32>::zeros((2, 2)), Some(&header)).unwrap();

        let reread = load_cached_full(&path).unwrap();
        let reread = reread.header().expect("header survived the round trip");
        assert_eq!(read_long_value(reread, HEADER_PMEXPR).as_deref(), Some(long_expr.as_str()));
        assert_eq!(read_long_value(reread, HEADER_PMSOURCE).as_deref(), Some(long_source.as_str()));
    }

    #[test]
    fn an_expression_longer_than_every_card_is_marked_truncated_not_silently_cut() {
        let huge: String = std::iter::repeat("$T + 1 ").take(400).collect();
        let header = output_header(None, &huge, "x.fits");
        let recorded = read_long_value(&header, HEADER_PMEXPR).unwrap();
        assert!(recorded.ends_with(TRUNCATION_MARKER), "{recorded}");
        assert!(header.get(&format!("PMEXPR{}", MAX_CONTINUATION_CARDS)).is_some());
        assert!(header.get(&format!("PMEXPR{}", MAX_CONTINUATION_CARDS + 1)).is_none());
    }

    #[test]
    fn finite_output_stats_ignore_non_finite_and_keep_tiny_values() {
        let arr = Array2::from_shape_vec((2, 2), vec![f32::NAN, 1e-9, f32::INFINITY, 2.0]).unwrap();
        let (stats, non_finite) = finite_output_stats(&arr);
        assert_eq!(non_finite, 2);
        let stats = stats.expect("some finite pixels");
        assert_eq!(stats.valid_count, 2);
        assert_eq!(stats.min, 1e-9f32 as f64);
        assert_eq!(stats.max, 2.0);
    }

    fn synthetic_sky(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(r, c)| {
            let base = 3.5 + ((r * 7 + c * 13) % 11) as f32 * 0.4;
            if r == rows / 2 && c == cols / 2 {
                3356.0
            } else {
                base
            }
        })
    }

    #[test]
    fn preview_of_a_signed_result_is_not_a_black_frame() {
        let sky = synthetic_sky(64, 64);
        let median = 3.5 + 5.0 * 0.4;
        let signed = sky.mapv(|v| v - median);
        assert!(signed.iter().any(|&v| v < 0.0), "fixture must be signed");

        let pixels = preview_pixels(&signed);
        let zeros = pixels.iter().filter(|&&p| p == 0).count();
        let distinct = pixels.iter().copied().collect::<std::collections::HashSet<u8>>().len();

        assert!(
            zeros * 2 < pixels.len(),
            "more than half the preview is black: {zeros}/{}",
            pixels.len()
        );
        assert!(distinct > 8, "preview collapsed to {distinct} levels");
    }

    #[test]
    fn preview_of_an_all_nan_result_is_black_without_panicking() {
        let pixels = preview_pixels(&Array2::from_elem((8, 8), f32::NAN));
        assert!(pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn finite_output_stats_report_nothing_when_every_pixel_is_non_finite() {
        let arr = Array2::from_elem((2, 2), f32::NAN);
        let (stats, non_finite) = finite_output_stats(&arr);
        assert_eq!(non_finite, 4);
        assert!(stats.is_none(), "all-NaN result must not report zeros as measurements");
        assert!(json!({ RES_STATS: stats })[RES_STATS].is_null());
    }
}
