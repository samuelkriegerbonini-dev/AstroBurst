use std::time::Instant;

use anyhow::{anyhow, bail, Context};
use ndarray::Array2;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cmd::common::{
    blocking_cmd, derived_output_header, load_cached_full, output_stem, resolve_output_dir,
    save_auto_stf_preview_png, source_path, write_derived_fits, OutputValues,
};
use crate::core::imaging::stats::{finite_slice_stats, is_padding, is_valid_pixel};
use crate::core::pixelmath::{compile, evaluate, validate, OutputOptions, PixelMathError};
use crate::infra::cache::ImageEntry;
use crate::types::constants::{
    RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH, RES_STATS, RES_WARNINGS,
};
use crate::types::header::HduHeader;
use crate::types::image::ImageStats;
use crate::types::image_ref::path_key;

pub const RES_NON_FINITE_COUNT: &str = "non_finite_count";
pub const DEFAULT_PIXELMATH_SUFFIX: &str = "pixelmath";
pub const TARGET_SYMBOL: &str = "$T";

const USER_SUFFIX_PREFIX: &str = "pm_";
const HEADER_PMEXPR: &str = "PMEXPR";
const HEADER_PMSOURCE: &str = "PMSRC";
const LEGACY_HEADER_PMSOURCE: &str = "PMSOURCE";
const MAX_HEADER_VALUE_BYTES: usize = 67;
const MAX_CONTINUATION_CARDS: usize = 9;
const TRUNCATION_MARKER: &str = "...";
const ZERO_FRACTION_WARNING: f64 = 0.05;

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

fn header_safe_path(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii() && !c.is_ascii_control() && c != '\'' && c != '%' {
            encoded.push(c);
            continue;
        }
        let mut buf = [0u8; 4];
        for b in c.encode_utf8(&mut buf).bytes() {
            encoded.push_str(&format!("%{:02X}", b));
        }
    }
    encoded
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

fn remove_long_value(header: &mut HduHeader, base: &str) {
    header.remove(base);
    for i in 1..=MAX_CONTINUATION_CARDS {
        header.remove(&format!("{}{}", base, i));
    }
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
    } else if cleaned == DEFAULT_PIXELMATH_SUFFIX || cleaned.starts_with(USER_SUFFIX_PREFIX) {
        cleaned
    } else {
        format!("{}{}", USER_SUFFIX_PREFIX, cleaned)
    }
}

pub(crate) fn output_header(source: Option<&HduHeader>, expression: &str, target: &str) -> HduHeader {
    let mut header = derived_output_header(source, DEFAULT_PIXELMATH_SUFFIX, OutputValues::Rescaled);
    remove_long_value(&mut header, HEADER_PMEXPR);
    remove_long_value(&mut header, HEADER_PMSOURCE);
    header.remove(LEGACY_HEADER_PMSOURCE);
    set_long_value(&mut header, HEADER_PMEXPR, &header_safe(expression));
    set_long_value(&mut header, HEADER_PMSOURCE, &header_safe_path(target));
    header
}

pub(crate) fn finite_output_stats(arr: &Array2<f32>) -> (Option<ImageStats>, usize) {
    let non_finite = arr.iter().filter(|v| !v.is_finite()).count();
    let mut valid: Vec<f32> = arr.iter().copied().filter(|v| is_valid_pixel(*v)).collect();
    if valid.is_empty() {
        return (None, non_finite);
    }
    (Some(finite_slice_stats(&mut valid)), non_finite)
}

fn preview_display(result: &Array2<f32>, inputs: &[&Array2<f32>]) -> Array2<f32> {
    let mut no_input_data = vec![!inputs.is_empty(); result.len()];
    for input in inputs {
        for (flag, &v) in no_input_data.iter_mut().zip(input.iter()) {
            *flag &= is_padding(v);
        }
    }
    let display: Vec<f32> = result
        .iter()
        .zip(&no_input_data)
        .map(|(&v, &padding)| {
            if padding || !v.is_finite() {
                f32::NAN
            } else if v == 0.0 {
                f32::MIN_POSITIVE
            } else {
                v
            }
        })
        .collect();
    Array2::from_shape_vec(result.dim(), display).unwrap_or_else(|_| result.clone())
}

fn exact_zero_fraction(arr: &Array2<f32>) -> f64 {
    let (finite, zeros) = arr.iter().fold((0usize, 0usize), |(finite, zeros), &v| {
        if v.is_finite() {
            (finite + 1, zeros + usize::from(v == 0.0))
        } else {
            (finite, zeros)
        }
    });
    if finite == 0 {
        0.0
    } else {
        zeros as f64 / finite as f64
    }
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

fn ensure_outputs_are_not_inputs(outputs: &[&str], inputs: &[(&str, &str)]) -> anyhow::Result<()> {
    for output in outputs {
        let output_key = path_key(output);
        if let Some((name, path)) = inputs.iter().find(|(_, path)| path_key(&source_path(path)) == output_key) {
            bail!(
                "pixelmath: the output {} is the file of {} ({}); choose another output name so the input is not overwritten",
                output,
                name,
                path
            );
        }
    }
    Ok(())
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
    let reduced = program.reduced_slots();

    let suffix = output_suffix(name);
    let stem = output_stem(path);
    let fits_path = format!("{}/{}_{}.fits", output_dir, stem, suffix);
    let png_path = format!("{}/{}_{}.png", output_dir, stem, suffix);
    let inputs: Vec<(&str, &str)> = std::iter::once((TARGET_SYMBOL, path))
        .chain(slots.iter().map(|s| (s.name.as_str(), s.path.as_str())))
        .collect();
    ensure_outputs_are_not_inputs(&[&fits_path, &png_path], &inputs)?;

    let target = load_cached_full(path)
        .with_context(|| format!("{} = {}", TARGET_SYMBOL, path))?;
    let mut entries: Vec<Option<ImageEntry>> = Vec::with_capacity(slots.len() + 1);
    entries.push(Some(target.clone()));
    for (i, slot) in slots.iter().enumerate() {
        if !referenced[i + 1] {
            entries.push(None);
            continue;
        }
        let entry = load_cached_full(&slot.path)
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
        if reduced.get(i).copied().unwrap_or(false) {
            let zeros = exact_zero_fraction(entry.arr());
            if zeros >= ZERO_FRACTION_WARNING {
                warnings.push(format!(
                    "{} is {:.0}% exact zeros, the value mosaics use as padding; med, mean, sdev, mdev, adev, min and max of {} include those pixels",
                    slot_names[i],
                    zeros * 100.0,
                    slot_names[i]
                ));
            }
        }
    }

    let target_arr = target.arr();
    let arrays: Vec<&Array2<f32>> = entries
        .iter()
        .map(|e| e.as_ref().map(|e| e.arr()).unwrap_or(target_arr))
        .collect();
    let result = evaluate(&program, &arrays, &opts).map_err(pixelmath_error)?;

    let header = output_header(target.header(), expression, &source_path(path));
    write_derived_fits(&fits_path, &result, Some(&header))?;

    let referenced_inputs: Vec<&Array2<f32>> = arrays
        .iter()
        .zip(&referenced)
        .filter(|(_, used)| **used)
        .map(|(arr, _)| *arr)
        .collect();
    let (rows, cols) = result.dim();
    save_auto_stf_preview_png(&preview_display(&result, &referenced_inputs), &png_path)?;

    let (stats, non_finite) = finite_output_stats(&result);
    Ok(json!({
        RES_PNG_PATH: png_path,
        RES_FITS_PATH: fits_path,
        RES_DIMENSIONS: [cols, rows],
        RES_STATS: stats,
        RES_NON_FINITE_COUNT: non_finite,
        RES_WARNINGS: warnings,
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
    use crate::cmd::common::test_support::{assert_png_matches, gpu_view_with_auto_stf, wide_textured_sky};
    use crate::cmd::common::{auto_stretch_preview, HEADER_ABPROC};
    use crate::core::imaging::stats::compute_image_stats;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::{RES_CLEANED_FILES, RES_CLEANED_PATHS, RES_MAX, RES_MIN};

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

    fn percent_decode(value: &str) -> String {
        let bytes = value.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&value[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
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
        assert_eq!(res[RES_STATS][RES_MIN], 2.5);
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
        assert_eq!(percent_decode(&full_source), source_path(&f.target));
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
        assert!(res[RES_FITS_PATH].as_str().unwrap().ends_with("pm_opts_hdu1_pm_clampedv1.fits"));
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
        let written = load_cached_full(res[RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert!(written.arr()[[0, 3]].is_nan());
        assert_eq!(written.arr()[[0, 0]], 0.0);
        assert_eq!(res[RES_STATS][RES_MIN], compute_image_stats(written.arr()).min);
        assert_eq!(res[RES_STATS][RES_MAX], 1.0);
    }

    #[tokio::test]
    async fn a_named_output_never_takes_the_file_name_of_another_step() {
        let f = fixture("pm_named.fits", 4);
        for step_suffix in ["arcsinh", "bg_corrected", "stf", "hdu3", "denoised"] {
            let res = pixelmath_cmd(
                f.target.clone(),
                f.out_dir.clone(),
                "$T * 2".into(),
                vec![],
                None,
                None,
                Some(step_suffix.into()),
            )
            .await
            .unwrap();
            let fits_path = res[RES_FITS_PATH].as_str().unwrap();
            assert!(
                fits_path.ends_with(&format!("pm_named_hdu1_pm_{step_suffix}.fits")),
                "{fits_path} can collide with the {step_suffix} step"
            );
        }
    }

    #[tokio::test]
    async fn pixelmath_refuses_to_overwrite_one_of_its_inputs() {
        let f = fixture("pm_mask.fits", 4);
        let first = pixelmath_cmd(f.target.clone(), f.out_dir.clone(), "$T > 5".into(), vec![], None, None, None)
            .await
            .unwrap();
        let mask = first[RES_FITS_PATH].as_str().unwrap().to_string();
        let before = std::fs::read(&mask).unwrap();

        let err = pixelmath_cmd(f.target.clone(), f.out_dir.clone(), "$T * M".into(), vec![slot("M", &mask)], None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("the file of M"), "{err}");
        assert_eq!(std::fs::read(&mask).unwrap(), before, "the mask slot was overwritten");

        let unused = pixelmath_cmd(
            f.target.clone(),
            f.out_dir.clone(),
            "$T * 3".into(),
            vec![slot("M", &mask.replace('/', "\\"))],
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(unused.contains("the file of M"), "a differently spelled slot path slipped through: {unused}");

        let renamed = pixelmath_cmd(
            f.target.clone(),
            f.out_dir.clone(),
            "$T * M".into(),
            vec![slot("M", &mask)],
            None,
            None,
            Some("masked".into()),
        )
        .await
        .unwrap();
        assert_ne!(renamed[RES_FITS_PATH].as_str().unwrap(), mask);
    }

    #[tokio::test]
    async fn an_output_dir_with_a_trailing_separator_still_protects_the_inputs() {
        let f = fixture("pm_slash.fits", 4);
        let first = pixelmath_cmd(f.target.clone(), f.out_dir.clone(), "$T > 5".into(), vec![], None, None, None)
            .await
            .unwrap();
        let mask = first[RES_FITS_PATH].as_str().unwrap().to_string();
        let before = std::fs::read(&mask).unwrap();

        let err = pixelmath_cmd(
            f.target.clone(),
            format!("{}/", f.out_dir),
            "$T * M".into(),
            vec![slot("M", &mask)],
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("the file of M"), "{err}");
        assert_eq!(std::fs::read(&mask).unwrap(), before, "the mask slot was overwritten through out_dir/");
    }

    #[tokio::test]
    async fn pixelmath_does_not_sweep_the_output_directory() {
        let f = fixture("pm_sweep.fits", 4);
        std::fs::create_dir_all(&f.out_dir).unwrap();
        let older = std::path::Path::new(&f.out_dir).join("other_file_bg_corrected.fits");
        std::fs::write(&older, vec![0u8; 2880]).unwrap();
        let res = pixelmath_cmd(f.target.clone(), f.out_dir.clone(), "$T + 1".into(), vec![], None, None, None)
            .await
            .unwrap();
        assert!(res.get(RES_CLEANED_FILES).is_none(), "{res}");
        assert!(res.get(RES_CLEANED_PATHS).is_none(), "{res}");
        assert!(older.exists());
    }

    #[tokio::test]
    async fn reducing_a_slot_that_is_mostly_zero_padding_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zero_padded.fits").to_str().unwrap().to_string();
        let padded = Array2::from_shape_fn((10, 10), |(r, c)| if r < 6 { 0.0 } else { 100.0 + c as f32 });
        write_fits_mono(&path, &padded, None).unwrap();
        let out_dir = dir.path().join("out").to_str().unwrap().to_string();

        let res = pixelmath_cmd(path.clone(), out_dir.clone(), "$T - med($T)".into(), vec![], None, None, None)
            .await
            .unwrap();
        let warnings: Vec<String> = serde_json::from_value(res[RES_WARNINGS].clone()).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("$T is 60% exact zeros")),
            "no padding warning for med on a zero-padded frame: {warnings:?}"
        );

        let res = pixelmath_cmd(path, out_dir, "$T * 2".into(), vec![], None, None, Some("plain".into()))
            .await
            .unwrap();
        let warnings: Vec<String> = serde_json::from_value(res[RES_WARNINGS].clone()).unwrap();
        assert!(warnings.is_empty(), "no reducer, no warning: {warnings:?}");
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
    fn output_suffix_sanitizes_and_namespaces_user_names() {
        assert_eq!(output_suffix(None), "pixelmath");
        assert_eq!(output_suffix(Some("   ")), "pixelmath");
        assert_eq!(output_suffix(Some("pixelmath")), "pixelmath");
        assert_eq!(output_suffix(Some("ha/oiii ratio")), "pm_haoiiiratio");
        assert_eq!(output_suffix(Some("ratio_v2-final")), "pm_ratio_v2-final");
        assert_eq!(output_suffix(Some("pm_ratio")), "pm_ratio");
        assert_eq!(output_suffix(Some("arcsinh")), "pm_arcsinh");
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
    fn a_rescaled_output_drops_every_zero_point_that_photometry_reads() {
        let mut source = HduHeader::empty();
        for key in crate::core::metadata::photcal::GENERIC_ZERO_POINT_KEYS {
            source.set(key, "30.0".into());
        }
        source.set("PHOTFLAM", "1e-19".into());
        source.set("MAXLIN", "50000".into());
        source.set("CRVAL1", "83.8".into());
        let out = output_header(Some(&source), "$T * 10", "des.fits");
        for key in crate::core::metadata::photcal::GENERIC_ZERO_POINT_KEYS {
            assert!(out.get(key).is_none(), "{key} still calibrates the rescaled pixels");
        }
        assert!(out.get("PHOTFLAM").is_none());
        assert!(out.get("MAXLIN").is_none());
        assert_eq!(out.get("CRVAL1"), Some("83.8"));
    }

    #[test]
    fn rerunning_on_a_pixelmath_output_leaves_no_stale_continuation_cards() {
        let long_expr = format!("$T * ({})", vec!["1"; 70].join(" + "));
        let long_source = format!("/{}/deep/pm_src.fits", vec!["some_long_directory"; 4].join("/"));
        let mut first = output_header(None, &long_expr, &long_source);
        first.set(LEGACY_HEADER_PMSOURCE, "old.fits".into());
        assert!(first.get("PMEXPR2").is_some() && first.get("PMSRC1").is_some());

        let second = output_header(Some(&first), "$T / 2", "short.fits");
        assert_eq!(read_long_value(&second, HEADER_PMEXPR).as_deref(), Some("$T / 2"));
        assert_eq!(read_long_value(&second, HEADER_PMSOURCE).as_deref(), Some("short.fits"));
        for stale in ["PMEXPR1", "PMEXPR2", "PMSRC1", LEGACY_HEADER_PMSOURCE] {
            assert!(second.get(stale).is_none(), "{stale} from the previous run survived");
        }
    }

    #[test]
    fn source_paths_with_apostrophes_or_accents_are_recorded_losslessly() {
        let target = "C:/Users/O\u{27}Brien/Ori\u{e3}o/100%/m42.fits";
        let header = output_header(None, "$T", target);
        let recorded = read_long_value(&header, HEADER_PMSOURCE).unwrap();
        assert_eq!(recorded, "C:/Users/O%27Brien/Ori%C3%A3o/100%25/m42.fits");
        assert!(recorded.is_ascii() && !recorded.contains('\u{27}'));
        assert_eq!(percent_decode(&recorded), target);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accented.fits").to_str().unwrap().to_string();
        write_fits_mono(&path, &Array2::<f32>::zeros((2, 2)), Some(&header)).unwrap();
        let reread = load_cached_full(&path).unwrap();
        let reread = read_long_value(reread.header().unwrap(), HEADER_PMSOURCE).unwrap();
        assert_eq!(percent_decode(&reread), target);
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

    fn median_u8(values: impl Iterator<Item = u8>) -> u8 {
        let mut v: Vec<u8> = values.collect();
        v.sort_unstable();
        v[v.len() / 2]
    }

    #[test]
    fn preview_of_a_signed_result_is_not_a_black_frame() {
        let sky = synthetic_sky(64, 64);
        let median = 3.5 + 5.0 * 0.4;
        let signed = sky.mapv(|v| v - median);
        assert!(signed.iter().any(|&v| v < 0.0), "fixture must be signed");

        let pixels = auto_stretch_preview(&preview_display(&signed, &[&sky]));
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
    fn zero_padding_of_the_input_stays_out_of_the_preview_stretch() {
        let (rows, cols) = (64usize, 64usize);
        let target = Array2::from_shape_fn((rows, cols), |(r, c)| {
            if r < 35 {
                0.0
            } else if r == 50 && c == 32 {
                10000.0
            } else {
                100.0 + ((r * 7 + c * 13) % 11) as f32 * 2.0
            }
        });
        let result = target.mapv(|v| v * 2.0);
        let pixels = auto_stretch_preview(&preview_display(&result, &[&target]));
        let sky = median_u8(pixels.iter().enumerate().filter(|(i, _)| i / cols >= 35).map(|(_, &p)| p));
        let padding = median_u8(pixels.iter().enumerate().filter(|(i, _)| i / cols < 35).map(|(_, &p)| p));
        assert_eq!(padding, 0);
        assert!(sky > 30, "sky rendered near black because padding joined the statistics: {sky}");
    }

    #[test]
    fn a_mask_result_previews_black_and_white() {
        let target = synthetic_sky(16, 16);
        let mask = target.mapv(|v| if v > 5.0 { 1.0 } else { 0.0 });
        assert!(mask.iter().any(|&v| v == 0.0) && mask.iter().any(|&v| v == 1.0));
        let pixels = auto_stretch_preview(&preview_display(&mask, &[&target]));
        for (m, p) in mask.iter().zip(&pixels) {
            assert_eq!(*p, if *m == 1.0 { 255 } else { 0 });
        }
    }

    #[test]
    fn preview_of_an_all_nan_result_is_black_without_panicking() {
        let pixels = auto_stretch_preview(&preview_display(&Array2::from_elem((8, 8), f32::NAN), &[]));
        assert!(pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn result_stats_leave_exact_zero_padding_out_like_the_image_statistics() {
        let arr = Array2::from_shape_vec((2, 4), vec![0.0, -4.0, 2.0, 0.0, 6.0, f32::NAN, -1.0, 3.0]).unwrap();
        let (stats, non_finite) = finite_output_stats(&arr);
        assert_eq!(non_finite, 1, "exact zeros are padding, not non-finite values");
        let stats = stats.expect("valid pixels");
        let image = compute_image_stats(&arr);
        assert_eq!(stats.valid_count, 5);
        assert_eq!(stats.valid_count, image.valid_count);
        assert_eq!(stats.min, -4.0);
        assert_eq!(stats.median, image.median);
        assert_eq!(stats.mean, image.mean);

        let (stats, non_finite) = finite_output_stats(&Array2::zeros((2, 2)));
        assert_eq!(non_finite, 0);
        assert!(stats.is_none(), "an all-padding result must not report measurements");
    }

    #[test]
    fn a_large_result_previews_like_the_gpu_view() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide.fits").to_str().unwrap().to_string();
        let out = dir.path().join("out").to_str().unwrap().to_string();
        let sky = wide_textured_sky();
        write_fits_mono(&src, &sky, None).unwrap();

        let res = run_pixelmath(&src, &out, "$T * 2", &[], OutputOptions::default(), None).unwrap();
        let expected = gpu_view_with_auto_stf(&sky.mapv(|v| v * 2.0));
        assert_png_matches(res[RES_PNG_PATH].as_str().unwrap(), &expected);
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
