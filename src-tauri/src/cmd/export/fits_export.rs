use std::time::Instant;

use ndarray::{Array2, Zip};
use serde_json::{json, Value};

use crate::cmd::catalog::uncalibrated_reason;
use crate::cmd::common::{blocking_cmd, cached_header, extract_image_resolved, image_ref, invalidate_written, OutputValues};
use crate::cmd::science_provenance::{InputIdentity, ProductRecord};
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, ImageStats, StfParams};
use crate::infra::fits::writer::{
    filter_header, write_fits_mono_bitpix, write_fits_mono_rice, write_fits_rgb_bitpix,
    write_fits_rgb_rice,
};
use crate::infra::wcs_source::header_wcs;
use crate::service::common::derived_output_header;
use crate::service::cutout::refuse_source_as_target;
use crate::types::constants::{COPY_WCS, RES_APPLY_STF, RES_BITPIX, RES_COMPRESS, RES_COPY_METADATA, RES_DIMENSIONS, RES_DISPLAY_REFERRED, RES_ELAPSED_MS, RES_FILE_SIZE_BYTES, RES_OUTPUT_PATH, RES_QUANTIZE_LEVEL, RES_WCS_WRITTEN};
use crate::types::header::HduHeader;

use super::png_export::validated_stf_value;
use super::rgb_sources::{channels_export, composite_export, fits_rgb_uses_composite, rgb_file_export};

pub(super) const DEFAULT_QUANTIZE_LEVEL: f64 = 16.0;
const SUPPORTED_BITPIX: [i32; 3] = [16, -32, -64];
const STF_MIDTONE_FLOOR: f64 = 0.0001;
const STF_MIDTONE_CEILING: f64 = 0.9999;
const OP_EXPORT_FITS: &str = "export_fits";
const OP_EXPORT_FITS_RGB: &str = "export_fits_rgb";
const ABPROC_STF_EXPORT: &str = "stf_export";
const STF_EXPORT_VALUES_NOTE: &str = "pixel values are display-referred in [0,1]";

fn stf_export_header(source: Option<&HduHeader>, stf: &StfParams) -> HduHeader {
    let mut header = derived_output_header(source, ABPROC_STF_EXPORT, OutputValues::DisplayReferred);
    header.cards.push((
        "HISTORY".to_string(),
        format!(
            "STF stretch applied (shadow={:.4} midtone={:.4} highlight={:.4})",
            stf.shadow, stf.midtone, stf.highlight
        ),
    ));
    header.cards.push(("HISTORY".to_string(), STF_EXPORT_VALUES_NOTE.to_string()));
    header
}

fn applied_midtone(midtone: f64) -> f64 {
    midtone.clamp(STF_MIDTONE_FLOOR, STF_MIDTONE_CEILING)
}

fn stf_record(record: ProductRecord, stf: &StfParams, stats: &ImageStats) -> ProductRecord {
    record
        .real("ABSTFSH", stf.shadow)
        .real("ABSTFMT", applied_midtone(stf.midtone))
        .real("ABSTFMRQ", stf.midtone)
        .real("ABSTFHI", stf.highlight)
        .real("ABSTFMIN", stats.min)
        .real("ABSTFMAX", stats.max)
}

fn stf_json(stf: Option<&StfParams>) -> Value {
    stf.map_or(Value::Null, |p| {
        json!({ "shadow": p.shadow, "midtone": p.midtone, "midtone_applied": applied_midtone(p.midtone), "highlight": p.highlight })
    })
}

fn checked_bitpix(bitpix: i32) -> anyhow::Result<i32> {
    if !SUPPORTED_BITPIX.contains(&bitpix) {
        anyhow::bail!("BITPIX {bitpix} cannot be written; choose 16, -32 or -64");
    }
    Ok(bitpix)
}

fn keep_blank_pixels(stretched: &mut Array2<f32>, source: &Array2<f32>) {
    Zip::from(stretched).and(source).for_each(|out, &value| {
        if !value.is_finite() {
            *out = f32::NAN;
        }
    });
}

fn explicit_header(header_path: Option<&str>) -> anyhow::Result<Option<HduHeader>> {
    header_path
        .map(|p| {
            cached_header(p).map_err(|e| e.context(format!("Failed to read the header source {}", p)))
        })
        .transpose()
}

fn writes_celestial_wcs(header: Option<&HduHeader>, dims: (usize, usize)) -> bool {
    header.is_some_and(|written| {
        let mut probe = written.clone();
        probe.set("NAXIS1", dims.1.to_string());
        probe.set("NAXIS2", dims.0.to_string());
        header_wcs(&probe).is_ok()
    })
}

fn wants_rice_compression(compress: &Option<String>) -> bool {
    compress.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("rice"))
}

#[tauri::command]
pub async fn export_fits(
    path: String,
    output_path: String,
    apply_stf_stretch: Option<bool>,
    shadow: Option<f64>,
    midtone: Option<f64>,
    highlight: Option<f64>,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
    bitpix: Option<i32>,
    compress: Option<String>,
    quantize_level: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let do_stf = apply_stf_stretch.unwrap_or(false);
        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);
        let target_bitpix = checked_bitpix(bitpix.unwrap_or(-32))?;
        let use_rice = wants_rice_compression(&compress);
        let qlevel = quantize_level.unwrap_or(DEFAULT_QUANTIZE_LEVEL);
        refuse_source_as_target(&output_path, &path)?;

        let stf = if do_stf {
            Some(StfParams {
                shadow: validated_stf_value("shadow", shadow)?.unwrap_or(0.0),
                midtone: validated_stf_value("midtone", midtone)?.unwrap_or(0.5),
                highlight: validated_stf_value("highlight", highlight)?.unwrap_or(1.0),
            })
        } else {
            None
        };
        let resolved = extract_image_resolved(&path)?;
        let base = filter_header(&resolved.header, do_wcs, do_meta);
        let filtered = match &stf {
            Some(params) => Some(stf_export_header(base.as_ref(), params)),
            None => base,
        };
        let display_referred = filtered.as_ref().is_some_and(|h| uncalibrated_reason(h).is_some());
        let source_ref = &resolved.arr;
        let parameters = json!({
            "bitpix": target_bitpix,
            "compress": if use_rice { "rice" } else { "none" },
            "quantize_level": qlevel,
            "copy_wcs": do_wcs,
            "copy_metadata": do_meta,
            "apply_stf": do_stf,
            "stf": stf_json(stf.as_ref()),
        });
        let mut record = ProductRecord::new(OP_EXPORT_FITS, &[InputIdentity::of("input", &path)], &parameters);

        let stretched;
        let write_ref = match &stf {
            Some(params) => {
                let stats = compute_image_stats(source_ref);
                let mut values = apply_stf_f32(source_ref, params, &stats);
                keep_blank_pixels(&mut values, source_ref);
                stretched = values;
                record = stf_record(record, params, &stats);
                &stretched
            }
            None => source_ref,
        };
        let mut header = filtered.unwrap_or_else(HduHeader::empty);
        record.stamp(&mut header);

        crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&output_path);
        if use_rice {
            write_fits_mono_rice(&output_path, write_ref, Some(&header), target_bitpix, qlevel)?;
        } else {
            write_fits_mono_bitpix(&output_path, write_ref, Some(&header), target_bitpix)?;
        }
        invalidate_written(&output_path);

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BITPIX: target_bitpix,
            RES_APPLY_STF: do_stf,
            RES_DISPLAY_REFERRED: display_referred,
            COPY_WCS: do_wcs,
            RES_COPY_METADATA: do_meta,
            RES_FILE_SIZE_BYTES: file_size,
            RES_COMPRESS: if use_rice { "rice" } else { "none" },
            RES_QUANTIZE_LEVEL: qlevel,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn export_fits_rgb(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_path: String,
    copy_wcs: Option<bool>,
    copy_metadata: Option<bool>,
    bitpix: Option<i32>,
    history: Option<Vec<String>>,
    compress: Option<String>,
    quantize_level: Option<f64>,
    header_path: Option<String>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let do_wcs = copy_wcs.unwrap_or(true);
        let do_meta = copy_metadata.unwrap_or(true);
        let target_bitpix = checked_bitpix(bitpix.unwrap_or(-32))?;
        let use_rice = wants_rice_compression(&compress);
        let qlevel = quantize_level.unwrap_or(DEFAULT_QUANTIZE_LEVEL);
        let paths = [r_path.as_deref(), g_path.as_deref(), b_path.as_deref()];
        let sources = paths.into_iter().chain([header_path.as_deref()]).flatten();
        for source in sources.filter(|p| !image_ref(p).is_synthetic()) {
            refuse_source_as_target(&output_path, source)?;
        }
        let requested_header = explicit_header(header_path.as_deref())?;

        let source = match rgb_file_export(paths)? {
            Some(file) => file,
            None => {
                let composite = if fits_rgb_uses_composite(paths) { composite_export(r_path.as_deref()) } else { None };
                match composite {
                    Some(composite) => composite,
                    None => channels_export(paths)?,
                }
            }
        };
        let (r_arr, g_arr, b_arr) = (source.r, source.g, source.b);
        let header_source = requested_header.or(source.header);

        let mut filtered = header_source
            .as_ref()
            .and_then(|h| filter_header(h, do_wcs, do_meta));

        if let Some(steps) = &history {
            let h = filtered.get_or_insert_with(crate::types::header::HduHeader::empty);
            for step in steps {
                let line: String = step.chars().take(70).collect();
                h.cards.push(("HISTORY".to_string(), line));
            }
        }

        let inputs: Vec<InputIdentity> = ["r", "g", "b"]
            .into_iter()
            .zip(paths)
            .filter_map(|(role, path)| path.map(|p| InputIdentity::of(role, p)))
            .chain(header_path.as_deref().map(|p| InputIdentity::of("header", p)))
            .collect();
        let parameters = json!({
            "bitpix": target_bitpix,
            "compress": if use_rice { "rice" } else { "none" },
            "quantize_level": qlevel,
            "copy_wcs": do_wcs,
            "copy_metadata": do_meta,
            "history": history,
        });
        let wcs_written = writes_celestial_wcs(filtered.as_ref(), r_arr.dim());
        let mut header = filtered.unwrap_or_else(HduHeader::empty);
        ProductRecord::new(OP_EXPORT_FITS_RGB, &inputs, &parameters).stamp(&mut header);

        crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&output_path);
        if use_rice {
            write_fits_rgb_rice(&output_path, &r_arr, &g_arr, &b_arr, Some(&header), target_bitpix, qlevel)?;
        } else {
            write_fits_rgb_bitpix(&output_path, &r_arr, &g_arr, &b_arr, Some(&header), target_bitpix)?;
        }
        invalidate_written(&output_path);

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let (rows, cols) = r_arr.dim();

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BITPIX: target_bitpix,
            COPY_WCS: do_wcs,
            RES_COPY_METADATA: do_meta,
            RES_WCS_WRITTEN: wcs_written,
            RES_FILE_SIZE_BYTES: file_size,
            RES_DIMENSIONS: [cols, rows],
            RES_COMPRESS: if use_rice { "rice" } else { "none" },
            RES_QUANTIZE_LEVEL: qlevel,
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}
