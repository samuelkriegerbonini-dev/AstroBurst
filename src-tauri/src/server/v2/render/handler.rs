// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use ndarray::{s, Array2};
use serde::Deserialize;
use serde_json::json;

use astroburst_lib::core::alignment::downsample::area_downsample;
use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::colormap::{apply_colormap, encode_png_rgb8, Colormap};
use astroburst_lib::core::imaging::stats::percentile;
use astroburst_lib::core::imaging::zscale::{zscale_limits, DEFAULT_CONTRAST};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

use super::super::region::{resolve_region_clamped, RegionSpec, ResolvedRegion};
use super::stretch::{apply_stretch, StretchKind, DEFAULT_ASINH_A, DEFAULT_POWER};

#[derive(Deserialize, Default)]
pub struct RenderParams {
    #[serde(default, alias = "ref", alias = "image")]
    pub image_ref: Option<String>,
    #[serde(default)]
    pub region: Option<RegionSpec>,
    #[serde(default)]
    pub scale: Option<ScaleSpec>,
    #[serde(default)]
    pub colormap: Option<String>,
    #[serde(default)]
    pub invert_cmap: bool,
    #[serde(default)]
    pub max_dim: Option<usize>,
    #[serde(default)]
    pub overlays: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default)]
pub struct ScaleSpec {
    #[serde(default)]
    pub algorithm: Option<String>,
    #[serde(default)]
    pub stretch: Option<String>,
    #[serde(default)]
    pub vmin: Option<f64>,
    #[serde(default)]
    pub vmax: Option<f64>,
    #[serde(default, alias = "percentiles")]
    pub percentile: Option<[f64; 2]>,
    #[serde(default)]
    pub asinh_a: Option<f64>,
    #[serde(default)]
    pub power: Option<f64>,
    #[serde(default)]
    pub zscale_contrast: Option<f64>,
}

async fn target_ref(session: &Session, explicit: Option<String>) -> Result<String> {
    match explicit {
        Some(r) => Ok(r),
        None => session
            .v2
            .active_ref
            .read()
            .await
            .clone()
            .ok_or_else(|| {
                AppError::BadRequest("no active image in this session; open a file first".into())
            }),
    }
}

enum DrawOp {
    Crosshair { px: f64, py: f64 },
    Scalebar { length_display_px: f64 },
}

const CROSSHAIR_COLOR: [u8; 3] = [255, 0, 0];
const SCALEBAR_COLOR: [u8; 3] = [255, 255, 255];

fn build_overlays(
    overlays: &[serde_json::Value],
    wcs: Option<&WcsTransform>,
    pixel_scale_arcsec: Option<f64>,
    factor: usize,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    for ov in overlays {
        let kind = ov.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "crosshair" => {
                let px = ov.get("x").and_then(|v| v.as_f64());
                let py = ov.get("y").and_then(|v| v.as_f64());
                let coord = match (px, py) {
                    (Some(x), Some(y)) => Some((x, y)),
                    _ => match (ov.get("ra").and_then(|v| v.as_f64()), ov.get("dec").and_then(|v| v.as_f64()), wcs) {
                        (Some(ra), Some(dec), Some(w)) => {
                            let (x, y) = w.world_to_pixel(ra, dec);
                            if x.is_finite() && y.is_finite() { Some((x, y)) } else { None }
                        }
                        _ => None,
                    },
                };
                if let Some((px, py)) = coord {
                    ops.push(DrawOp::Crosshair { px, py });
                }
            }
            "scalebar" => {
                let length_arcsec = ov.get("length_arcsec").and_then(|v| v.as_f64());
                if let (Some(len), Some(scale)) = (length_arcsec, pixel_scale_arcsec) {
                    let png_scale = scale * factor as f64;
                    if png_scale > 0.0 && len > 0.0 {
                        ops.push(DrawOp::Scalebar { length_display_px: len / png_scale });
                    }
                }
            }
            _ => {  }
        }
    }
    ops
}

#[inline]
fn set_px(buf: &mut [u8], w: usize, h: usize, x: i64, y: i64, color: [u8; 3]) {
    if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
        return;
    }
    let idx = (y as usize * w + x as usize) * 3;
    buf[idx..idx + 3].copy_from_slice(&color);
}

fn draw_overlays(
    buf: &mut [u8],
    w: usize,
    h: usize,
    resolved: &ResolvedRegion,
    factor: usize,
    ops: &[DrawOp],
) {
    let f = factor as f64;
    for op in ops {
        match *op {
            DrawOp::Crosshair { px, py } => {
                let dx = ((px - resolved.x as f64) / f).round() as i64;
                let dy = ((py - resolved.y as f64) / f).round() as i64;
                for y in 0..h as i64 {
                    set_px(buf, w, h, dx, y, CROSSHAIR_COLOR);
                }
                for x in 0..w as i64 {
                    set_px(buf, w, h, x, dy, CROSSHAIR_COLOR);
                }
            }
            DrawOp::Scalebar { length_display_px } => {
                let len = (length_display_px.round() as i64).max(1);
                let margin = 4i64;
                let x0 = margin;
                let x1 = margin + len;
                let y_base = h as i64 - 1 - margin;
                for t in 0..3i64 {
                    for x in x0..=x1 {
                        set_px(buf, w, h, x, y_base - t, SCALEBAR_COLOR);
                    }
                }
            }
        }
    }
}

fn resolve_scale(
    display: &Array2<f32>,
    algorithm: &str,
    scale: &ScaleSpec,
) -> Result<(f64, f64)> {
    match algorithm {
        "zscale" => {
            let contrast = scale.zscale_contrast.unwrap_or(DEFAULT_CONTRAST);
            let (mut vmin, mut vmax) = zscale_limits(display, contrast);
            if !vmin.is_finite() || !vmax.is_finite() {
                vmin = 0.0;
                vmax = 1.0;
            }
            Ok((vmin, vmax))
        }
        "minmax" => Ok(finite_min_max(display)),
        "manual" => {
            let (dmin, dmax) = finite_min_max(display);
            Ok((scale.vmin.unwrap_or(dmin), scale.vmax.unwrap_or(dmax)))
        }
        "percentile" => {
            let [lo, hi] = scale.percentile.unwrap_or([1.0, 99.5]);
            let mut valid: Vec<f32> = display
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .collect();
            if valid.is_empty() {
                return Ok((0.0, 1.0));
            }
            let vmin = percentile(&mut valid, (lo / 100.0).clamp(0.0, 1.0)) as f64;
            let vmax = percentile(&mut valid, (hi / 100.0).clamp(0.0, 1.0)) as f64;
            Ok((vmin, vmax))
        }
        other => Err(AppError::BadRequestWithHint {
            code: "bad_request",
            message: format!("unknown scale algorithm '{other}'"),
            hint: Some("supported algorithms: zscale, minmax, percentile, manual".into()),
        }),
    }
}

fn finite_min_max(a: &Array2<f32>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &v in a.iter() {
        if v.is_finite() {
            let v = v as f64;
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
        }
    }
    if min.is_finite() && max.is_finite() {
        (min, max)
    } else {
        (0.0, 1.0)
    }
}

fn stretch_name(kind: StretchKind) -> &'static str {
    match kind {
        StretchKind::Linear => "linear",
        StretchKind::Log => "log",
        StretchKind::Sqrt => "sqrt",
        StretchKind::Asinh => "asinh",
        StretchKind::Power => "power",
    }
}

fn colormap_name(cmap: Colormap) -> &'static str {
    match cmap {
        Colormap::Gray => "gray",
        Colormap::Viridis => "viridis",
    }
}

pub async fn render(
    SessionExtractor(session): SessionExtractor,
    Json(params): Json<RenderParams>,
) -> Result<Response> {
    let target = target_ref(&session, params.image_ref).await?;
    let entry = session
        .cache
        .get(&target)
        .ok_or_else(|| AppError::NotFound(format!("image ref {target} not found in session")))?;
    let arr = entry.arr();
    let (rows, cols) = arr.dim();
    let wcs = entry.header().and_then(|h| WcsTransform::from_header(h).ok());

    let resolved = match &params.region {
        Some(spec) => resolve_region_clamped(spec, cols, rows, wcs.as_ref())?,
        None => ResolvedRegion { x: 0, y: 0, width: cols, height: rows, clipped: false },
    };

    let region_arr = arr
        .slice(s![resolved.y..resolved.y + resolved.height, resolved.x..resolved.x + resolved.width])
        .to_owned();

    let long_side = resolved.width.max(resolved.height);
    let factor = match params.max_dim {
        Some(md) if md >= 1 && long_side > md => {
            (((long_side as f64) / md as f64).ceil() as usize).max(1)
        }
        _ => 1,
    };

    let scale = params.scale.unwrap_or_default();
    let algorithm = scale
        .algorithm
        .as_deref()
        .unwrap_or("zscale")
        .trim()
        .to_ascii_lowercase();
    let stretch_kind = StretchKind::from_name(scale.stretch.as_deref().unwrap_or("linear"))?;
    let cmap = Colormap::from_name(
        &params.colormap.as_deref().unwrap_or("gray").trim().to_ascii_lowercase(),
    )
    .map_err(|m| AppError::BadRequestWithHint {
        code: "bad_request",
        message: m,
        hint: Some("supported colormaps: gray, viridis".into()),
    })?;

    let asinh_a = scale.asinh_a.unwrap_or(DEFAULT_ASINH_A);
    let power = scale.power.unwrap_or(DEFAULT_POWER);
    let invert = params.invert_cmap;

    let pixel_scale = wcs
        .as_ref()
        .map(|w| w.pixel_scale_arcsec())
        .filter(|s| s.is_finite() && *s > 0.0);
    let overlays = build_overlays(&params.overlays, wcs.as_ref(), pixel_scale, factor);

    let resolved_c = resolved.clone();
    let alg_c = algorithm.clone();
    let (png, vmin, vmax, below_frac, above_frac) = tokio::task::spawn_blocking(move || {
        let display = if factor > 1 {
            let out_rows = resolved_c.height.div_ceil(factor).max(1);
            let out_cols = resolved_c.width.div_ceil(factor).max(1);
            area_downsample(&region_arr, out_rows, out_cols)
        } else {
            region_arr
        };
        let (disp_rows, disp_cols) = display.dim();

        let (vmin, vmax) = resolve_scale(&display, &alg_c, &scale)?;
        let range = vmax - vmin;

        let data = display
            .as_slice()
            .expect("display array is standard-layout after area_downsample/to_owned");
        let mut norm = Vec::with_capacity(data.len());
        let mut valid: u64 = 0;
        let mut below: u64 = 0;
        let mut above: u64 = 0;
        for &v in data {
            if v.is_finite() {
                valid += 1;
                let vd = v as f64;
                if vd <= vmin {
                    below += 1;
                }
                if vd >= vmax {
                    above += 1;
                }
                let n = if range > 0.0 {
                    (((vd - vmin) / range) as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let mut s = apply_stretch(n, stretch_kind, asinh_a, power);
                if invert {
                    s = 1.0 - s;
                }
                norm.push(s);
            } else {
                norm.push(f32::NAN);
            }
        }

        let mut rgb = apply_colormap(&norm, cmap);
        draw_overlays(&mut rgb, disp_cols, disp_rows, &resolved_c, factor, &overlays);
        let png = encode_png_rgb8(&rgb, disp_cols, disp_rows).map_err(AppError::Internal)?;

        let (below_frac, above_frac) = if valid > 0 {
            (below as f64 / valid as f64, above as f64 / valid as f64)
        } else {
            (0.0, 0.0)
        };
        Ok::<_, AppError>((png, vmin, vmax, below_frac, above_frac))
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("task panic: {e}")))??;

    let png_scale = pixel_scale.map(|s| s * factor as f64);

    let resolved_json = json!({
        "ref": target,
        "region": { "x": resolved.x, "y": resolved.y, "w": resolved.width, "h": resolved.height },
        "region_clipped": resolved.clipped,
        "vmin": vmin,
        "vmax": vmax,
        "scale_algorithm": algorithm,
        "stretch": stretch_name(stretch_kind),
        "colormap": colormap_name(cmap),
        "binning_applied": factor,
        "png_scale_arcsec_per_px": png_scale,
        "clipped_fraction": { "below_vmin": below_frac, "above_vmax": above_frac },
    });
    let resolved_hdr = serde_json::to_string(&resolved_json).unwrap_or_default();

    Ok((
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            ("x-render-resolved".parse().unwrap(), resolved_hdr),
        ],
        png,
    )
        .into_response())
}
