use std::time::Instant;

use anyhow::{anyhow, bail};
use serde::Deserialize;
use serde_json::{json, Value};

use ndarray::Array2;

use crate::cmd::analysis::{resolve_dq_mask, DqMask};
use crate::cmd::common::{blocking_cmd, load_cached_full, load_companions};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::region::{
    line_cut, radial_profile, region_data_stats, PhysicalMap, RegionShape, RegionSystem, SigmaClip,
};
use crate::core::imaging::region_file::{parse_reg_with_physical, write_reg_with_physical, Region};
use crate::infra::cache::ImageEntry;
use crate::types::constants::{
    RES_DQ_EXCLUDED, RES_ELAPSED_MS, RES_ERROR, RES_HAS_WCS, RES_ID, RES_MASKED, RES_REGIONS,
    RES_REG_TEXT, RES_STATS, RES_SYSTEM, RES_WARNINGS,
};

const MAX_REGIONS_PER_CALL: usize = 512;
const MAX_REG_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_SIGMA_CLIP_ITERS: usize = 100;

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

fn entry_physical(entry: &ImageEntry) -> PhysicalMap {
    entry.header().map(PhysicalMap::from_header).unwrap_or_else(PhysicalMap::identity)
}

fn sigma_clip_params(sigma: Option<f32>, maxiters: Option<usize>) -> anyhow::Result<SigmaClip> {
    let defaults = SigmaClip::default();
    let sigma = sigma.unwrap_or(defaults.sigma);
    let maxiters = maxiters.unwrap_or(defaults.maxiters);
    if !(sigma.is_finite() && sigma > 0.0) {
        bail!("sigma must be a finite number greater than 0, got {}", sigma);
    }
    if !(1..=MAX_SIGMA_CLIP_ITERS).contains(&maxiters) {
        bail!("maxiters must be between 1 and {}, got {}", MAX_SIGMA_CLIP_ITERS, maxiters);
    }
    Ok(SigmaClip { sigma, maxiters })
}

pub(crate) fn companion_err(path: &str, dims: (usize, usize)) -> Option<ImageEntry> {
    match load_companions(path) {
        Ok(comps) => comps.err.filter(|e| e.arr().dim() == dims),
        Err(e) => {
            log::warn!("ERR companion unavailable for {}: {:#}", path, e);
            None
        }
    }
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
    err: Option<&Array2<f32>>,
    clip: SigmaClip,
) -> Vec<Value> {
    let excluded = mask.map(|m| &m.map);
    regions
        .iter()
        .map(|req| {
            match region_data_stats(entry.arr(), &req.shape, req.background.as_ref(), excluded, err, clip) {
                Ok(stats) => json!({ RES_ID: req.id, RES_STATS: stats, RES_ERROR: Value::Null }),
                Err(e) => json!({ RES_ID: req.id, RES_STATS: Value::Null, RES_ERROR: e.to_string() }),
            }
        })
        .collect()
}

pub(crate) fn import_for_entry(entry: &ImageEntry, reg_text: &str) -> anyhow::Result<Value> {
    let wcs = entry_wcs(entry);
    let parsed = parse_reg_with_physical(reg_text, wcs.as_ref(), &entry_physical(entry))?;
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
    Ok(write_reg_with_physical(regions, system, wcs.as_ref(), sexagesimal, &entry_physical(entry))?)
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
        let clip = sigma_clip_params(sigma, maxiters)?;
        let entry = load_cached_full(&path)?;
        let mask = resolve_dq_mask(&path, exclude_dq.unwrap_or(false), entry.arr().dim());
        let err_entry = companion_err(&path, entry.arr().dim());
        let entries = stats_for_entry(&entry, &regions, mask.as_ref(), err_entry.as_ref().map(|e| e.arr()), clip);
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
        let out = stats_for_entry(&entry, &regions, Some(&mask), None, SigmaClip::default());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0][RES_ID], "all");
        assert!(out[0][RES_ERROR].is_null());
        assert_eq!(out[0][RES_STATS]["n_excluded"], 2);
        assert_eq!(out[0][RES_STATS]["n_padding"], 1);
        assert_eq!(out[0][RES_STATS]["count"], 13);
        assert_eq!(out[1][RES_ID], "empty");
        assert!(out[1][RES_STATS].is_null());
        assert!(out[1][RES_ERROR].as_str().unwrap().contains("no finite pixels"));

        let unmasked = stats_for_entry(&entry, &regions[..1], None, None, SigmaClip::default());
        assert_eq!(unmasked[0][RES_STATS]["n_excluded"], 0);
        assert_eq!(unmasked[0][RES_STATS]["n_padding"], 1);
        assert_eq!(unmasked[0][RES_STATS]["count"], 15);
        assert_eq!(unmasked[0][RES_STATS]["sum"], 120.0);

        let invalid = stats_for_entry(
            &entry,
            &[req("bad", RegionShape::Circle { x: 1.0, y: 1.0, r: -1.0 })],
            None,
            None,
            SigmaClip::default(),
        );
        assert!(invalid[0][RES_ERROR].as_str().unwrap().contains("invalid region"));
    }

    #[test]
    fn stats_for_entry_propagates_the_err_companion_of_a_mef() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("err_regions.fits");
        let mut dq = vec![-2147483648i32; 16];
        dq[1] = -2147483647;
        dq[6] = -2147483645;
        sci_err_dq_mef(&path, 4, 4, dq);
        let key = format!("{}#hdu=1", path.to_str().unwrap());
        let entry = load_cached_full(&key).unwrap();
        let err = companion_err(&key, entry.arr().dim()).expect("ERR companion");
        assert_eq!(err.arr()[[0, 2]], 1.0);

        let whole = RegionShape::Box { x: 1.5, y: 1.5, width: 4.0, height: 4.0, angle: 0.0 };
        let regions = vec![req("all", whole)];
        let out = stats_for_entry(&entry, &regions, None, Some(err.arr()), SigmaClip::default());
        let sum_err = out[0][RES_STATS]["sum_err"].as_f64().unwrap();
        let expected: f64 = (0..16).map(|i| (0.5 * i as f64).powi(2)).sum::<f64>().sqrt();
        assert!((sum_err - expected).abs() < 1e-6, "sum_err={sum_err} expected={expected}");
        assert!(out[0][RES_STATS]["weighted_mean"].as_f64().unwrap().is_finite());

        let mask = resolve_dq_mask(&key, true, entry.arr().dim()).expect("mask");
        let masked = stats_for_entry(&entry, &regions, Some(&mask), Some(err.arr()), SigmaClip::default());
        let expected_masked: f64 = (0..16)
            .filter(|i| *i != 1 && *i != 6)
            .map(|i| (0.5 * i as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        let masked_err = masked[0][RES_STATS]["sum_err"].as_f64().unwrap();
        assert!((masked_err - expected_masked).abs() < 1e-6, "sum_err={masked_err} expected={expected_masked}");

        let without = stats_for_entry(&entry, &regions, None, None, SigmaClip::default());
        assert!(without[0][RES_STATS]["sum_err"].is_null());
        assert!(without[0][RES_STATS]["weighted_mean"].is_null());

        assert!(companion_err(&key, (8, 8)).is_none());
        assert!(companion_err(&format!("{}#hdu=4", path.to_str().unwrap()), (4, 4)).is_none());
    }

    #[test]
    fn region_stats_agree_with_the_statistics_region_mode_on_a_zero_padded_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drz_edge.fits");
        let arr = Array2::from_shape_fn((12, 12), |(y, x)| if x < 5 { 0.0 } else { ((x + 3 * y) % 7) as f32 - 3.5 });
        write_fits_mono(path.to_str().unwrap(), &arr, None).unwrap();
        let entry = load_cached_full(path.to_str().unwrap()).unwrap();
        let shape = RegionShape::Box { x: 5.5, y: 5.5, width: 12.0, height: 12.0, angle: 0.0 };

        let out = stats_for_entry(&entry, &[req("frame", shape.clone())], None, None, SigmaClip::default());
        let region = &out[0][RES_STATS];
        let statistics = crate::core::imaging::statistics::statistics_for_region(entry.arr(), &shape, None).unwrap();
        assert_eq!(region["count"], statistics.count);
        assert_eq!(region["n_padding"], statistics.padding);
        assert_eq!(region["count"], 84);
        assert_eq!(region["median"].as_f64().unwrap(), statistics.median);
        assert_eq!(region["min"].as_f64().unwrap(), statistics.min);
        assert_eq!(region["max"].as_f64().unwrap(), statistics.max);
        assert!((region["mean"].as_f64().unwrap() - statistics.mean).abs() < 1e-12);
        assert_eq!(statistics.min, -3.5);
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

    #[test]
    fn physical_regions_follow_the_ltv_offset_of_a_cutout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cutout.fits");
        let arr = Array2::<f32>::from_elem((64, 64), 1.0);
        let mut header = crate::types::header::HduHeader::empty();
        header.set_f64("LTV1", -10.0);
        header.set_f64("LTV2", -20.0);
        write_fits_mono(path.to_str().unwrap(), &arr, Some(&header)).unwrap();
        let entry = load_cached_full(path.to_str().unwrap()).unwrap();

        let back = import_for_entry(&entry, "physical\ncircle(31,51,2)\n").unwrap();
        let shapes: Vec<Region> = serde_json::from_value(back[RES_REGIONS].clone()).unwrap();
        assert_eq!(shapes[0].shape, RegionShape::Circle { x: 20.0, y: 30.0, r: 2.0 });

        let text = export_for_entry(&entry, &shapes, RegionSystem::Physical, true).unwrap();
        assert_eq!(text.lines().nth(2), Some("physical"));
        assert!(text.lines().nth(3).unwrap().starts_with("circle(31,51,2)"), "{text}");
        let image = export_for_entry(&entry, &shapes, RegionSystem::Image, true).unwrap();
        assert!(image.lines().nth(3).unwrap().starts_with("circle(21,31,2)"), "{image}");
    }

    #[tokio::test]
    async fn region_stats_cmd_rejects_a_non_positive_sigma_and_an_unbounded_maxiters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.fits");
        let data = Array2::<f32>::from_shape_fn((8, 8), |(y, x)| (y * 8 + x) as f32 + 1.0);
        write_fits_mono(path.to_str().unwrap(), &data, None).unwrap();
        let key = path.to_str().unwrap().to_string();
        let regions = || vec![req("all", RegionShape::Box { x: 3.5, y: 3.5, width: 8.0, height: 8.0, angle: 0.0 })];

        for sigma in [-1.0f32, 0.0, f32::NAN, f32::INFINITY] {
            let err = region_stats_cmd(key.clone(), regions(), None, Some(sigma), None).await.unwrap_err();
            assert!(err.contains("sigma must be a finite number greater than 0"), "{err}");
        }
        for maxiters in [0usize, MAX_SIGMA_CLIP_ITERS + 1] {
            let err = region_stats_cmd(key.clone(), regions(), None, None, Some(maxiters)).await.unwrap_err();
            assert!(err.contains("maxiters must be between 1 and 100"), "{err}");
        }
        let ok = region_stats_cmd(key, regions(), None, Some(2.5), Some(MAX_SIGMA_CLIP_ITERS)).await.unwrap();
        assert!(ok[RES_REGIONS][0][RES_ERROR].is_null(), "{ok}");
        assert_eq!(ok[RES_REGIONS][0][RES_STATS]["count"], 64);
    }

    #[tokio::test]
    async fn export_cmd_rejects_unsupported_system() {
        let err = regions_export_cmd("nowhere.fits".into(), vec![], "galactic".into(), None)
            .await
            .unwrap_err();
        assert!(err.contains("unsupported coordinate system 'galactic'"), "{err}");
    }
}
