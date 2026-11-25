use std::sync::Arc;
use std::time::Instant;

use ndarray::Array2;
use serde_json::{json, Value};

use crate::cmd::common::{blocking_cmd, extract_image_resolved, try_extract_rgb_resolved};
use crate::cmd::helpers;
use crate::cmd::science_provenance::{InputIdentity, ProductRecord};
use crate::cmd::processing::is_display_referred;
use crate::core::imaging::stats::{combine_channel_stats, compute_image_stats};
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig, ImageStats, StfParams};
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::render::grayscale::{render_grayscale_hq, render_grayscale_16bit, render_stretched_8bit, render_stretched_16bit};
use crate::infra::render::rgb::{render_rgb, render_rgb_16bit};
use crate::types::constants::{COMPOSITE_KEY_R, COMPOSITE_KEY_G, COMPOSITE_KEY_B, RES_APPLY_STF, RES_BIT_DEPTH, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FILE_SIZE_BYTES, RES_OUTPUT_PATH};
use crate::types::header::HduHeader;

use super::rgb_sources::{load_export_channel, png_rgb_uses_composite, resample_to, rgb_file_export};

pub(super) fn validated_stf_value(name: &str, value: Option<f64>) -> anyhow::Result<Option<f64>> {
    match value {
        Some(v) if !(0.0..=1.0).contains(&v) => {
            anyhow::bail!("{} must be a number between 0 and 1, got {}", name, v)
        }
        other => Ok(other),
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PngStf {
    pub(super) apply: bool,
    pub(super) channels: [[Option<f64>; 3]; 3],
    pub(super) linked: Option<bool>,
}

impl PngStf {
    fn explicit(&self) -> Option<[StfParams; 3]> {
        if !self.apply || self.channels.iter().flatten().all(Option::is_none) {
            return None;
        }
        Some(self.channels.map(|[shadow, midtone, highlight]| StfParams {
            shadow: shadow.unwrap_or(0.0),
            midtone: midtone.unwrap_or(0.5),
            highlight: highlight.unwrap_or(1.0),
        }))
    }
}

fn stf_json(stf: &StfParams) -> Value {
    json!({ "shadow": stf.shadow, "midtone": stf.midtone, "highlight": stf.highlight })
}

fn rgb_png_inputs(paths: [Option<&str>; 3]) -> Vec<InputIdentity> {
    ["r", "g", "b"].into_iter().zip(paths).filter_map(|(role, path)| path.map(|p| InputIdentity::of(role, p))).collect()
}

fn identical_stf(p: &[StfParams; 3]) -> bool {
    p.iter().all(|s| s.shadow == p[0].shadow && s.midtone == p[0].midtone && s.highlight == p[0].highlight)
}

pub(super) fn png_rgb_planes(
    planes: [&Array2<f32>; 3],
    stats: [&ImageStats; 3],
    stf: &PngStf,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let explicit = stf.explicit();
    let linked = stf.linked.unwrap_or_else(|| explicit.as_ref().map_or(true, identical_stf));
    let combined = combine_channel_stats(stats[0], stats[1], stats[2]);
    let config = AutoStfConfig::default();
    let params = match explicit {
        Some(p) => p,
        None if linked => [auto_stf(&combined, &config); 3],
        None => stats.map(|s| auto_stf(s, &config)),
    };
    let norm = if linked { [&combined; 3] } else { stats };
    (
        apply_stf_f32(planes[0], &params[0], norm[0]),
        apply_stf_f32(planes[1], &params[1], norm[1]),
        apply_stf_f32(planes[2], &params[2], norm[2]),
    )
}

const IDENTITY_STF: StfParams = StfParams { shadow: 0.0, midtone: 0.5, highlight: 1.0 };
const OP_EXPORT_PNG: &str = "export_png";
const OP_EXPORT_RGB_PNG: &str = "export_rgb_png";

pub(super) fn png_stf_for<'a>(headers: impl IntoIterator<Item = Option<&'a HduHeader>>, stf: &PngStf) -> Option<StfParams> {
    let mut present = headers.into_iter().peekable();
    let display_referred = present.peek().is_some() && present.all(is_display_referred);
    (display_referred && stf.explicit().is_none()).then_some(IDENTITY_STF)
}

fn planes_as_shown(planes: [&Array2<f32>; 3], params: &StfParams) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let unit = ImageStats { min: 0.0, max: 1.0, ..ImageStats::default() };
    (
        apply_stf_f32(planes[0], params, &unit),
        apply_stf_f32(planes[1], params, &unit),
        apply_stf_f32(planes[2], params, &unit),
    )
}

fn png_planes_for(
    planes: [&Array2<f32>; 3],
    identity: Option<StfParams>,
    stf: &PngStf,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    match identity {
        Some(params) => planes_as_shown(planes, &params),
        None => {
            let stats = planes.map(compute_image_stats);
            png_rgb_planes(planes, [&stats[0], &stats[1], &stats[2]], stf)
        }
    }
}

pub(super) fn render_rgb_png(
    planes: (&Array2<f32>, &Array2<f32>, &Array2<f32>),
    depth: u8,
    output_path: &str,
) -> anyhow::Result<()> {
    if depth == 16 {
        render_rgb_16bit(planes.0, planes.1, planes.2, output_path)
    } else {
        render_rgb(planes.0, planes.1, planes.2, output_path)
    }
}

#[tauri::command]
pub async fn export_png(
    path: String,
    output_path: String,
    bit_depth: Option<u8>,
    apply_stf_stretch: Option<bool>,
    shadow: Option<f64>,
    midtone: Option<f64>,
    highlight: Option<f64>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let depth = bit_depth.unwrap_or(16);
        let do_stf = apply_stf_stretch.unwrap_or(false);
        let checked = |name: &str, value: Option<f64>| if do_stf { validated_stf_value(name, value) } else { Ok(value) };
        let requested = StfParams {
            shadow: checked("shadow", shadow)?.unwrap_or(0.0),
            midtone: checked("midtone", midtone)?.unwrap_or(0.5),
            highlight: checked("highlight", highlight)?.unwrap_or(1.0),
        };
        let inputs = [InputIdentity::of("input", &path)];

        if let Some(rgb) = try_extract_rgb_resolved(&path)? {
            let shown_as_is = !do_stf && is_display_referred(Some(&rgb.header));
            let (r_out, g_out, b_out, applied) = if shown_as_is {
                let (r, g, b) = planes_as_shown([&rgb.r, &rgb.g, &rgb.b], &IDENTITY_STF);
                (r, g, b, IDENTITY_STF)
            } else {
                let sr = compute_image_stats(&rgb.r);
                let sg = compute_image_stats(&rgb.g);
                let sb = compute_image_stats(&rgb.b);
                let combined = crate::core::imaging::stats::combine_channel_stats(&sr, &sg, &sb);
                let stf = if do_stf {
                    requested
                } else {
                    helpers::compute_linked_stf_with_stats(&sr, &sg, &sb, &AutoStfConfig::default()).0
                };
                (
                    apply_stf_f32(&rgb.r, &stf, &combined),
                    apply_stf_f32(&rgb.g, &stf, &combined),
                    apply_stf_f32(&rgb.b, &stf, &combined),
                    stf,
                )
            };

            if depth == 16 {
                render_rgb_16bit(&r_out, &g_out, &b_out, &output_path)?;
            } else {
                render_rgb(&r_out, &g_out, &b_out, &output_path)?;
            }
            let parameters = json!({ "bit_depth": depth, "apply_stf": !shown_as_is, "requested_stf": do_stf, "stf": stf_json(&applied) });
            ProductRecord::new(OP_EXPORT_PNG, &inputs, &parameters).write_png_text(&output_path)?;

            let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
            let (rows, cols) = rgb.r.dim();

            return Ok(json!({
                RES_OUTPUT_PATH: output_path,
                RES_BIT_DEPTH: depth,
                RES_APPLY_STF: !shown_as_is,
                RES_FILE_SIZE_BYTES: file_size,
                RES_DIMENSIONS: [cols, rows],
                RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
            }));
        }

        let resolved = extract_image_resolved(&path)?;

        if do_stf {
            let stats = compute_image_stats(&resolved.arr);
            let stretched = apply_stf_f32(&resolved.arr, &requested, &stats);
            if depth == 16 {
                render_stretched_16bit(&stretched, &output_path)?;
            } else {
                render_stretched_8bit(&stretched, &output_path)?;
            }
        } else if depth == 16 {
            render_grayscale_16bit(&resolved.arr, &output_path)?;
        } else {
            render_grayscale_hq(&resolved.arr, &output_path)?;
        }
        let stf = if do_stf { stf_json(&requested) } else { Value::Null };
        let parameters = json!({ "bit_depth": depth, "apply_stf": do_stf, "requested_stf": do_stf, "stf": stf });
        ProductRecord::new(OP_EXPORT_PNG, &inputs, &parameters).write_png_text(&output_path)?;

        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let (rows, cols) = resolved.arr.dim();

        Ok(json!({
            RES_OUTPUT_PATH: output_path,
            RES_BIT_DEPTH: depth,
            RES_APPLY_STF: do_stf,
            RES_FILE_SIZE_BYTES: file_size,
            RES_DIMENSIONS: [cols, rows],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

fn rgb_png_response(
    output_path: &str,
    depth: u8,
    applied_stf: bool,
    dims: (usize, usize),
    t0: Instant,
) -> serde_json::Value {
    let file_size = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    json!({
        RES_OUTPUT_PATH: output_path,
        RES_BIT_DEPTH: depth,
        RES_APPLY_STF: applied_stf,
        RES_FILE_SIZE_BYTES: file_size,
        RES_DIMENSIONS: [dims.1, dims.0],
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
    })
}

#[tauri::command]
pub async fn export_rgb_png(
    r_path: Option<String>,
    g_path: Option<String>,
    b_path: Option<String>,
    output_path: String,
    bit_depth: Option<u8>,
    apply_stf_stretch: Option<bool>,
    shadow_r: Option<f64>,
    midtone_r: Option<f64>,
    highlight_r: Option<f64>,
    shadow_g: Option<f64>,
    midtone_g: Option<f64>,
    highlight_g: Option<f64>,
    shadow_b: Option<f64>,
    midtone_b: Option<f64>,
    highlight_b: Option<f64>,
    linked: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let depth = bit_depth.unwrap_or(16);
        let do_stf = apply_stf_stretch.unwrap_or(false);
        let stf = PngStf {
            apply: do_stf,
            channels: [
                [
                    validated_stf_value("shadow_r", shadow_r)?,
                    validated_stf_value("midtone_r", midtone_r)?,
                    validated_stf_value("highlight_r", highlight_r)?,
                ],
                [
                    validated_stf_value("shadow_g", shadow_g)?,
                    validated_stf_value("midtone_g", midtone_g)?,
                    validated_stf_value("highlight_g", highlight_g)?,
                ],
                [
                    validated_stf_value("shadow_b", shadow_b)?,
                    validated_stf_value("midtone_b", midtone_b)?,
                    validated_stf_value("highlight_b", highlight_b)?,
                ],
            ],
            linked,
        };
        let paths = [r_path.as_deref(), g_path.as_deref(), b_path.as_deref()];
        let parameters = json!({ "bit_depth": depth, "apply_stf": do_stf, "linked": linked, "channels": stf.channels });
        let record = ProductRecord::new(OP_EXPORT_RGB_PNG, &rgb_png_inputs(paths), &parameters);

        if let Some(file) = rgb_file_export(paths)? {
            let identity = png_stf_for([file.header.as_ref()], &stf);
            let (r_out, g_out, b_out) = png_planes_for([&file.r, &file.g, &file.b], identity, &stf);
            render_rgb_png((&r_out, &g_out, &b_out), depth, &output_path)?;
            record.write_png_text(&output_path)?;
            return Ok(rgb_png_response(&output_path, depth, identity.is_none(), file.r.dim(), t0));
        }

        if png_rgb_uses_composite(paths) {
            if let Some((tr, tg, tb)) = helpers::load_composite_toned().or_else(helpers::load_composite_stretched) {
                render_rgb_png((tr.arr(), tg.arr(), tb.arr()), depth, &output_path)?;
                record.write_png_text(&output_path)?;
                return Ok(rgb_png_response(&output_path, depth, false, tr.arr().dim(), t0));
            }

            let cache_r = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_R);
            let cache_g = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_G);
            let cache_b = GLOBAL_IMAGE_CACHE.get(COMPOSITE_KEY_B);

            if let (Some(cr), Some(cg), Some(cb)) = (&cache_r, &cache_g, &cache_b) {
                let (r_out, g_out, b_out) =
                    png_rgb_planes([cr.arr(), cg.arr(), cb.arr()], [cr.stats(), cg.stats(), cb.stats()], &stf);
                render_rgb_png((&r_out, &g_out, &b_out), depth, &output_path)?;
                record.write_png_text(&output_path)?;
                return Ok(rgb_png_response(&output_path, depth, do_stf, cr.arr().dim(), t0));
            }
        }

        let [r_entry, g_entry, b_entry] = paths.map(|p| p.map(load_export_channel).transpose());
        let (r_entry, g_entry, b_entry) = (r_entry?, g_entry?, b_entry?);

        let any_entry = r_entry.as_ref().or(g_entry.as_ref()).or(b_entry.as_ref())
            .ok_or_else(|| anyhow::anyhow!("At least one channel path required"))?;
        let identity = png_stf_for([&r_entry, &g_entry, &b_entry].into_iter().flatten().map(|e| e.header.as_ref()), &stf);
        let rows = [&r_entry, &g_entry, &b_entry].iter().flat_map(|e| e.as_ref()).map(|e| e.arr.dim().0).max().unwrap_or(0);
        let cols = [&r_entry, &g_entry, &b_entry].iter().flat_map(|e| e.as_ref()).map(|e| e.arr.dim().1).max().unwrap_or(0);
        let zeros = Arc::new(Array2::<f32>::zeros(any_entry.arr.dim()));

        let [ra, ga, ba] = [r_entry, g_entry, b_entry]
            .map(|e| resample_to(e.map_or_else(|| Arc::clone(&zeros), |e| e.arr), (rows, cols)));
        let (ra, ga, ba) = (ra?, ga?, ba?);

        let (r_out, g_out, b_out) = png_planes_for([&ra, &ga, &ba], identity, &stf);
        render_rgb_png((&r_out, &g_out, &b_out), depth, &output_path)?;
        record.write_png_text(&output_path)?;
        Ok(rgb_png_response(&output_path, depth, identity.is_none(), (rows, cols), t0))
    })
}
