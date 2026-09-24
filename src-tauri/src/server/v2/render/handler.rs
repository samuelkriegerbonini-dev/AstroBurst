// astroburst headless server — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use axum::{
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use ndarray::s;
use serde::Deserialize;
use serde_json::json;

use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::imaging::colormap::{apply_colormap_inverted, encode_png_rgb8, Colormap};
use astroburst_lib::core::imaging::scale::{
    normalize_and_stretch, resolve_limits, LimitMode, StretchKind, DEFAULT_ASINH_A, DEFAULT_POWER,
};

use crate::error::{AppError, Result};
use crate::extractors::SessionExtractor;
use crate::session::Session;

use super::super::bin::nan_area_downsample;
use super::super::region::{resolve_region_clamped, RegionSpec, ResolvedRegion};

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
                    let length_display_px = len / png_scale;
                    if png_scale.is_finite() && png_scale > 0.0 && len > 0.0 && length_display_px.is_finite() {
                        ops.push(DrawOp::Scalebar { length_display_px });
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
                let len = length_display_px.round().clamp(1.0, (w as f64).max(1.0)) as i64;
                let margin = 4i64;
                let x0 = margin;
                let x1 = margin.saturating_add(len).min(w as i64 - 1);
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

fn names_of<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> String {
    all.iter().map(|&k| name(k)).collect::<Vec<_>>().join(", ")
}

fn parse_stretch(name: &str) -> Result<StretchKind> {
    StretchKind::from_name(name).map_err(|message| AppError::BadRequestWithHint {
        code: "bad_request",
        message,
        hint: Some(format!(
            "supported stretches: {}",
            names_of(&StretchKind::ALL, StretchKind::name)
        )),
    })
}

fn parse_colormap(name: &str) -> Result<Colormap> {
    Colormap::from_name(name).map_err(|message| AppError::BadRequestWithHint {
        code: "bad_request",
        message,
        hint: Some(format!(
            "supported colormaps: {}",
            names_of(&Colormap::ALL, Colormap::name)
        )),
    })
}

fn parse_limit_mode(scale: &ScaleSpec) -> Result<LimitMode> {
    LimitMode::from_parts(
        scale.algorithm.as_deref().unwrap_or("zscale"),
        scale.vmin,
        scale.vmax,
        scale.percentile,
        scale.zscale_contrast,
    )
    .map_err(|message| AppError::BadRequestWithHint {
        code: "bad_request",
        message,
        hint: Some("supported algorithms: minmax, zscale, percentile, user (alias: manual)".into()),
    })
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
    let mode = parse_limit_mode(&scale)?;
    let stretch_kind = parse_stretch(scale.stretch.as_deref().unwrap_or("linear"))?;
    let cmap = parse_colormap(params.colormap.as_deref().unwrap_or("gray"))?;

    let asinh_a = scale.asinh_a.unwrap_or(DEFAULT_ASINH_A);
    let power = scale.power.unwrap_or(DEFAULT_POWER);
    let invert = params.invert_cmap;

    let pixel_scale = wcs
        .as_ref()
        .map(|w| w.pixel_scale_arcsec())
        .filter(|s| s.is_finite() && *s > 0.0);
    let overlays = build_overlays(&params.overlays, wcs.as_ref(), pixel_scale, factor);

    let resolved_c = resolved.clone();
    let (png, vmin, vmax, below_frac, above_frac) = tokio::task::spawn_blocking(move || {
        let display = if factor > 1 {
            let out_rows = resolved_c.height.div_ceil(factor).max(1);
            let out_cols = resolved_c.width.div_ceil(factor).max(1);
            nan_area_downsample(&region_arr, out_rows, out_cols)
        } else {
            region_arr
        };
        let (disp_rows, disp_cols) = display.dim();

        let (vmin, vmax) = resolve_limits(&display, mode);

        let data = display
            .as_slice()
            .expect("display array is standard-layout after nan_area_downsample/to_owned");
        let (norm, valid, below, above) =
            normalize_and_stretch(data, vmin, vmax, stretch_kind, asinh_a, power);

        let mut rgb = apply_colormap_inverted(&norm, cmap, invert);
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
        "scale_algorithm": mode.name(),
        "stretch": stretch_kind.name(),
        "colormap": cmap.name(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bad_request_parts(err: AppError) -> (&'static str, String, Option<String>) {
        match err {
            AppError::BadRequestWithHint { code, message, hint } => (code, message, hint),
            other => panic!("expected BadRequestWithHint, got {other:?}"),
        }
    }

    #[test]
    fn parse_stretch_accepts_core_names_and_maps_errors_to_bad_request() {
        assert_eq!(parse_stretch(" ASINH ").unwrap(), StretchKind::Asinh);
        for bad in ["histeq", "gamma", ""] {
            let (code, message, hint) = bad_request_parts(parse_stretch(bad).unwrap_err());
            assert_eq!(code, "bad_request", "{bad:?}");
            assert!(message.contains(bad), "{message}");
            let hint = hint.expect("hint present");
            for kind in StretchKind::ALL {
                assert!(hint.contains(kind.name()), "{hint} lacks {}", kind.name());
            }
        }
    }

    #[test]
    fn parse_colormap_accepts_all_nine_and_maps_errors_to_bad_request() {
        for cmap in Colormap::ALL {
            assert_eq!(parse_colormap(cmap.name()).unwrap(), cmap);
        }
        assert_eq!(parse_colormap("grey").unwrap(), Colormap::Gray);
        let (code, message, hint) = bad_request_parts(parse_colormap("bone").unwrap_err());
        assert_eq!(code, "bad_request");
        assert!(message.contains("bone"), "{message}");
        let hint = hint.expect("hint present");
        for cmap in Colormap::ALL {
            assert!(hint.contains(cmap.name()), "{hint} lacks {}", cmap.name());
        }
    }

    #[test]
    fn scalebar_longer_than_the_image_is_clamped_to_the_image_width() {
        let (w, h) = (16usize, 12usize);
        let full = ResolvedRegion { x: 0, y: 0, width: w, height: h, clipped: false };
        for length in [f64::MAX, 1e13, 9.3e18, f64::INFINITY] {
            let mut buf = vec![0u8; w * h * 3];
            draw_overlays(&mut buf, w, h, &full, 1, &[DrawOp::Scalebar { length_display_px: length }]);
            let row = h - 1 - 4;
            for x in 4..w {
                assert_eq!(&buf[(row * w + x) * 3..(row * w + x) * 3 + 3], &SCALEBAR_COLOR, "length {length} x {x}");
            }
            assert_eq!(&buf[(row * w + 3) * 3..(row * w + 3) * 3 + 3], &[0, 0, 0]);
        }
    }

    #[test]
    fn scalebar_overlay_rejects_non_finite_lengths() {
        let overlays = vec![
            serde_json::json!({"type": "scalebar", "length_arcsec": 30.0}),
            serde_json::json!({"type": "scalebar", "length_arcsec": -5.0}),
        ];
        let ops = build_overlays(&overlays, None, Some(0.1), 1);
        assert_eq!(ops.len(), 1);
        assert!(build_overlays(&overlays, None, Some(f64::INFINITY), 1).is_empty());
        assert!(build_overlays(&overlays, None, Some(0.0), 1).is_empty());
    }

    #[test]
    fn parse_limit_mode_defaults_to_zscale_and_accepts_manual_alias() {
        let mode = parse_limit_mode(&ScaleSpec::default()).unwrap();
        assert!(matches!(mode, LimitMode::ZScale { .. }));
        assert_eq!(mode.name(), "zscale");

        let manual = ScaleSpec {
            algorithm: Some("manual".into()),
            vmin: Some(1.0),
            vmax: Some(9.0),
            ..ScaleSpec::default()
        };
        let mode = parse_limit_mode(&manual).unwrap();
        assert_eq!(mode, LimitMode::User { vmin: Some(1.0), vmax: Some(9.0) });
        assert_eq!(mode.name(), "user");

        let bogus = ScaleSpec { algorithm: Some("bogus".into()), ..ScaleSpec::default() };
        let (code, message, hint) = bad_request_parts(parse_limit_mode(&bogus).unwrap_err());
        assert_eq!(code, "bad_request");
        assert!(message.contains("bogus"), "{message}");
        assert!(hint.unwrap().contains("manual"));
    }
}
