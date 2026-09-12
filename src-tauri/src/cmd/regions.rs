use std::time::Instant;

use anyhow::{anyhow, bail};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cmd::analysis::{resolve_dq_mask, DqMask};
use crate::cmd::common::{blocking_cmd, load_cached_full};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::region::{
    line_cut, radial_profile, region_stats, RegionShape, RegionSystem, SigmaClip,
};
use crate::core::imaging::region_file::{parse_reg, write_reg, Region};
use crate::infra::cache::ImageEntry;
use crate::types::constants::{
    RES_DQ_EXCLUDED, RES_ELAPSED_MS, RES_ERROR, RES_HAS_WCS, RES_ID, RES_MASKED, RES_REGIONS,
    RES_REG_TEXT, RES_STATS, RES_SYSTEM, RES_WARNINGS,
};

const MAX_REGIONS_PER_CALL: usize = 512;
const MAX_REG_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
pub struct RegionStatsRequest {
    pub id: String,
    pub shape: RegionShape,
    #[serde(default)]
    pub background: Option<RegionShape>,
}

fn entry_wcs(entry: &ImageEntry) -> Option<WcsTransform> {
    entry.header().and_then(|h| WcsTransform::from_header(h).ok())
}

fn with_timing(mut body: Value, masked: bool, t0: Instant) -> Value {
    if let Some(obj) = body.as_object_mut() {
        obj.insert(RES_MASKED.to_string(), json!(masked));
        obj.insert(RES_ELAPSED_MS.to_string(), json!(t0.elapsed().as_millis() as u64));
    }
    body
}

pub(crate) fn stats_for_entry(
    entry: &ImageEntry,
    regions: &[RegionStatsRequest],
    mask: Option<&DqMask>,
    clip: SigmaClip,
) -> Vec<Value> {
    let excluded = mask.map(|m| &m.map);
    regions
        .iter()
        .map(|req| {
            match region_stats(entry.arr(), &req.shape, req.background.as_ref(), excluded, clip) {
                Ok(stats) => json!({ RES_ID: req.id, RES_STATS: stats, RES_ERROR: Value::Null }),
                Err(e) => json!({ RES_ID: req.id, RES_STATS: Value::Null, RES_ERROR: e.to_string() }),
            }
        })
        .collect()
}

pub(crate) fn import_for_entry(entry: &ImageEntry, reg_text: &str) -> anyhow::Result<Value> {
    let wcs = entry_wcs(entry);
    let parsed = parse_reg(reg_text, wcs.as_ref())?;
    Ok(json!({
        RES_REGIONS: parsed.regions,
        RES_WARNINGS: parsed.warnings,
        RES_HAS_WCS: wcs.is_some(),
    }))
}

pub(crate) fn export_for_entry(
    entry: &ImageEntry,
    regions: &[Region],
    system: RegionSystem,
    sexagesimal: bool,
) -> anyhow::Result<String> {
    let wcs = entry_wcs(entry);
    Ok(write_reg(regions, system, wcs.as_ref(), sexagesimal)?)
}

#[tauri::command]
pub async fn region_stats_cmd(
    path: String,
    regions: Vec<RegionStatsRequest>,
    exclude_dq: Option<bool>,
    sigma: Option<f32>,
    maxiters: Option<usize>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        if regions.len() > MAX_REGIONS_PER_CALL {
            bail!("too many regions in one call ({}); the limit is {}", regions.len(), MAX_REGIONS_PER_CALL);
        }
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let defaults = SigmaClip::default();
        let clip = SigmaClip {
            sigma: sigma.unwrap_or(defaults.sigma),
            maxiters: maxiters.unwrap_or(defaults.maxiters),
        };
        let entries = stats_for_entry(&entry, &regions, mask.as_ref(), clip);
        Ok(json!({
            RES_REGIONS: entries,
            RES_MASKED: mask.is_some(),
            RES_DQ_EXCLUDED: mask.as_ref().map(|m| m.excluded),
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[tauri::command]
pub async fn radial_profile_cmd(
    path: String,
    x: f64,
    y: f64,
    max_radius: f64,
    background: Option<[f64; 2]>,
    exclude_dq: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let profile = radial_profile(
            entry.arr(),
            x,
            y,
            max_radius,
            background.map(|b| (b[0], b[1])),
            mask.as_ref().map(|m| &m.map),
        )?;
        Ok(with_timing(serde_json::to_value(profile)?, mask.is_some(), t0))
    })
}

#[tauri::command]
pub async fn line_cut_cmd(
    path: String,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    exclude_dq: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let cut = line_cut(entry.arr(), x1, y1, x2, y2, mask.as_ref().map(|m| &m.map))?;
        Ok(with_timing(serde_json::to_value(cut)?, mask.is_some(), t0))
    })
}

#[tauri::command]
pub async fn regions_import_cmd(path: String, reg_text: String) -> Result<Value, String> {
    blocking_cmd!({
        if reg_text.len() > MAX_REG_TEXT_BYTES {
            bail!("region file is too large ({} bytes); the limit is {} bytes", reg_text.len(), MAX_REG_TEXT_BYTES);
        }
        let entry = load_cached_full(&path)?;
        import_for_entry(&entry, &reg_text)
    })
}

#[tauri::command]
pub async fn regions_export_cmd(
    path: String,
    regions: Vec<Region>,
    system: String,
    sexagesimal: Option<bool>,
) -> Result<Value, String> {
    blocking_cmd!({
        let system = RegionSystem::parse(&system).map_err(|e| anyhow!(e))?;
        let entry = load_cached_full(&path)?;
        let text = export_for_entry(&entry, &regions, system, sexagesimal.unwrap_or(true))?;
        Ok(json!({ RES_REG_TEXT: text, RES_SYSTEM: system.name() }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::{header_with_cd, north_up_cd};
    use crate::core::imaging::region_file::RegionProperties;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef;
    use crate::infra::fits::writer::write_fits_mono;
    use ndarray::Array2;

    fn req(id: &str, shape: RegionShape) -> RegionStatsRequest {
        RegionStatsRequest { id: id.into(), shape, background: None }
    }

    #[test]
    fn stats_for_entry_reports_dq_exclusion_and_isolates_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("regions.fits");
        let mut dq = vec![-2147483648i32; 16];
        dq[1] = -2147483647;
        dq[6] = -2147483645;
        dq[9] = -2147483646;
        sci_err_dq_mef(&path, 4, 4, dq);
        let key = format!("{}#hdu=1", path.to_str().unwrap());
        let entry = load_cached_full(&key).unwrap();
        let mask = resolve_dq_mask(&key, true, entry.arr().dim()).expect("mask");
        assert_eq!(mask.excluded, 2);

        let whole = RegionShape::Box { x: 1.5, y: 1.5, width: 4.0, height: 4.0, angle: 0.0 };
        let regions = vec![
            req("all", whole.clone()),
            req("empty", RegionShape::Circle { x: 100.0, y: 100.0, r: 2.0 }),
        ];
        let out = stats_for_entry(&entry, &regions, Some(&mask), SigmaClip::default());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0][RES_ID], "all");
        assert!(out[0][RES_ERROR].is_null());
        assert_eq!(out[0][RES_STATS]["n_excluded"], 2);
        assert_eq!(out[0][RES_STATS]["count"], 14);
        assert_eq!(out[1][RES_ID], "empty");
        assert!(out[1][RES_STATS].is_null());
        assert!(out[1][RES_ERROR].as_str().unwrap().contains("no finite pixels"));

        let unmasked = stats_for_entry(&entry, &regions[..1], None, SigmaClip::default());
        assert_eq!(unmasked[0][RES_STATS]["n_excluded"], 0);
        assert_eq!(unmasked[0][RES_STATS]["count"], 16);
        assert_eq!(unmasked[0][RES_STATS]["sum"], 120.0);

        let invalid = stats_for_entry(
            &entry,
            &[req("bad", RegionShape::Circle { x: 1.0, y: 1.0, r: -1.0 })],
            None,
            SigmaClip::default(),
        );
        assert!(invalid[0][RES_ERROR].as_str().unwrap().contains("invalid region"));
    }

    #[test]
    fn export_then_import_round_trips_in_image_and_fk5() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wcs_regions.fits");
        let arr = Array2::<f32>::from_elem((100, 100), 3.0);
        let header = header_with_cd(north_up_cd());
        write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        let key = path.to_str().unwrap().to_string();
        let entry = load_cached_full(&key).unwrap();
        assert!(entry_wcs(&entry).is_some(), "header should carry the WCS cards");

        let regions = vec![
            Region {
                shape: RegionShape::Circle { x: 49.5, y: 49.5, r: 3.5 },
                props: RegionProperties { color: Some("red".into()), text: Some("star A".into()), ..Default::default() },
            },
            Region {
                shape: RegionShape::Box { x: 20.0, y: 30.0, width: 10.0, height: 4.0, angle: 25.0 },
                props: RegionProperties { include: false, ..Default::default() },
            },
        ];

        let text = export_for_entry(&entry, &regions, RegionSystem::Image, true).unwrap();
        assert!(text.lines().nth(3).unwrap().starts_with("circle(50.5,50.5,3.5)"));
        let back = import_for_entry(&entry, &text).unwrap();
        assert_eq!(back[RES_HAS_WCS], true);
        assert!(back[RES_WARNINGS].as_array().unwrap().is_empty());
        let shapes: Vec<Region> = serde_json::from_value(back[RES_REGIONS].clone()).unwrap();
        assert_eq!(shapes[0].shape, regions[0].shape);
        assert_eq!(shapes[0].props.color.as_deref(), Some("red"));
        assert_eq!(shapes[0].props.text.as_deref(), Some("star A"));
        assert!(!shapes[1].props.include);
        assert_eq!(shapes[1].shape, regions[1].shape);

        let text = export_for_entry(&entry, &regions, RegionSystem::Fk5, true).unwrap();
        assert_eq!(text.lines().nth(2), Some("fk5"));
        let back = import_for_entry(&entry, &text).unwrap();
        let shapes: Vec<Region> = serde_json::from_value(back[RES_REGIONS].clone()).unwrap();
        let (x, y) = shapes[0].shape.centre();
        assert!((x - 49.5).abs() < 1e-2 && (y - 49.5).abs() < 1e-2, "({x},{y})");
        match &shapes[0].shape {
            RegionShape::Circle { r, .. } => assert!((r - 3.5).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
        match &shapes[1].shape {
            RegionShape::Box { x, y, width, height, angle } => {
                assert!((x - 20.0).abs() < 1e-2 && (y - 30.0).abs() < 1e-2);
                assert!((width - 10.0).abs() < 1e-6 && (height - 4.0).abs() < 1e-6);
                assert!((angle - 25.0).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn import_without_wcs_rejects_sky_regions_and_reports_has_wcs_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.fits");
        let arr = Array2::<f32>::from_elem((8, 8), 1.0);
        write_fits_mono(path.to_str().unwrap(), &arr, None).unwrap();
        let entry = load_cached_full(path.to_str().unwrap()).unwrap();
        let back = import_for_entry(&entry, "image\ncircle(4,4,2)\n").unwrap();
        assert_eq!(back[RES_HAS_WCS], false);
        assert_eq!(back[RES_REGIONS].as_array().unwrap().len(), 1);
        let err = import_for_entry(&entry, "fk5\ncircle(150,2,3\")\n").unwrap_err();
        assert!(err.to_string().contains("requires a WCS"));
        assert!(export_for_entry(&entry, &[], RegionSystem::Icrs, true).is_err());
    }

    #[tokio::test]
    async fn export_cmd_rejects_unsupported_system() {
        let err = regions_export_cmd("nowhere.fits".into(), vec![], "galactic".into(), None)
            .await
            .unwrap_err();
        assert!(err.contains("unsupported coordinate system 'galactic'"), "{err}");
    }
}
