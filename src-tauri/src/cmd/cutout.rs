use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use ndarray::Array2;
use serde::Serialize;

use crate::cmd::common::{blocking_cmd, load_cached_full, load_companions, output_stem, source_path};
use crate::core::astrometry::wcs::WcsTransform;
use crate::core::imaging::cutout::{
    cut_int_plane, cut_plane, fraction_on_image, padding_plane, rect_from_region, reported_ltv, resolve_cutout_rect,
    shift_header, CutoutRect, CutoutRequest,
};
use crate::core::imaging::dq_flags::DqTable;
use crate::core::imaging::region::{RegionShape, RegionSystem};
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::cache::GLOBAL_IMAGE_CACHE;
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::extract_header_by_index;
use crate::infra::fits::writer::{filter_header, write_mef_images, HduData, ImageHdu};
use crate::types::constants::{EXTNAME_DQ, EXTNAME_ERR};
use crate::types::header::HduHeader;

pub const EXTNAME_SCI: &str = "SCI";
pub const MAX_CUTOUT_PLANE_BYTES: usize = 2 * 1024 * 1024 * 1024;

const PADDED_FLAGS_JWST: &[&str] = &["DO_NOT_USE", "NON_SCIENCE"];
const PADDED_FLAGS_HST: &[&str] = &["REPLACED_FILL", "MASKED"];
const PLANE_ONLY_KEYS: &[&str] = &[
    "BUNIT", "EXTNAME", "EXTVER", "LTV1", "LTV2", "LTM1_1", "LTM2_2", "BZERO", "BSCALE", "BLANK", "DATAMIN", "DATAMAX",
];

#[derive(Debug, Clone, Serialize)]
pub struct CutoutExport {
    pub output_path: String,
    pub rect: CutoutRect,
    pub fraction_on_image: f64,
    pub hdus: Vec<String>,
    pub ltv1: f64,
    pub ltv2: f64,
    pub rotated_box_used_bounds: bool,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

enum IntPixels {
    Unsigned(Array2<u32>),
    Signed(Array2<i32>),
}

pub fn padded_dq_bits(table: DqTable) -> Result<u32> {
    let names = match table {
        DqTable::Hst => PADDED_FLAGS_HST,
        DqTable::Jwst | DqTable::Roman | DqTable::Unknown => PADDED_FLAGS_JWST,
    };
    table.mask_from_names(names).map_err(|e| anyhow!(e))
}

pub fn cutout_file_name(stem: &str, rect: &CutoutRect) -> String {
    format!("{stem}_cutout_{}_{}_{}x{}.fits", rect.x0, rect.y0, rect.width, rect.height)
}

fn comparable_path(path: &str) -> String {
    let resolved = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string());
    resolved.replace('\\', "/").to_ascii_lowercase()
}

pub(crate) fn resolve_target_path(
    path: &str,
    output_path: &str,
    output_dir: Option<&str>,
    rect: &CutoutRect,
) -> Result<String> {
    let file_name = cutout_file_name(&output_stem(path), rect);
    let requested = output_path.trim();
    let target = if requested.is_empty() {
        let dir = output_dir
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .context("no output path or output directory given for the cutout")?;
        std::fs::create_dir_all(dir).with_context(|| format!("Failed to create output directory: {dir}"))?;
        format!("{}/{}", dir.trim_end_matches(['/', '\\']), file_name)
    } else if Path::new(requested).is_dir() {
        format!("{}/{}", requested.trim_end_matches(['/', '\\']), file_name)
    } else {
        requested.to_string()
    };
    let source = source_path(path);
    if comparable_path(&target) == comparable_path(&source) {
        bail!("refusing to overwrite the source file {source}");
    }
    Ok(target)
}

pub(crate) fn provenance_only(merged: &HduHeader) -> HduHeader {
    let mut out = HduHeader::empty();
    let non_wcs = filter_header(merged, false, true).unwrap_or_else(HduHeader::empty);
    for (key, value) in &non_wcs.cards {
        let key = key.trim();
        if !PLANE_ONLY_KEYS.contains(&key) {
            out.set(key, value.clone());
        }
    }
    out
}

fn source_primary_header(path: &str, merged: Option<&HduHeader>) -> HduHeader {
    let from_disk = resolve_single_image(&source_path(path)).ok().and_then(|(fits, _tmp)| {
        if is_asdf_file(&fits) {
            return None;
        }
        let file = std::fs::File::open(&fits).ok()?;
        extract_header_by_index(&file, 0).ok()
    });
    from_disk.as_ref().or(merged).map(provenance_only).unwrap_or_else(HduHeader::empty)
}

fn synthetic_dq_header(sci_header: Option<&HduHeader>) -> Option<HduHeader> {
    let mut header = sci_header?.clone();
    header.remove("BUNIT");
    Some(header)
}

pub(crate) fn export_cutout(
    path: &str,
    output_path: &str,
    output_dir: Option<&str>,
    region: &RegionShape,
    include_err: bool,
    include_dq: bool,
    sky: bool,
) -> Result<CutoutExport> {
    let started = Instant::now();
    let entry = load_cached_full(path)?;
    let (img_h, img_w) = entry.arr().dim();
    let wcs = entry.header().and_then(|h| WcsTransform::from_header(h).ok());
    let system = if sky { RegionSystem::Icrs } else { RegionSystem::Image };
    let region_rect = rect_from_region(region, system, wcs.as_ref(), (img_w, img_h)).map_err(|e| anyhow!(e))?;
    let budget_request = CutoutRequest::Pixel {
        x0: region_rect.rect.x0,
        y0: region_rect.rect.y0,
        width: region_rect.rect.width,
        height: region_rect.rect.height,
    };
    let rect = resolve_cutout_rect(&budget_request, img_w, img_h, None, MAX_CUTOUT_PLANE_BYTES).map_err(|e| anyhow!(e))?;
    let target = resolve_target_path(path, output_path, output_dir, &rect)?;
    let companions = load_companions(path)?;

    let mut warnings = Vec::new();
    if region_rect.rotated_box_used_bounds {
        warnings.push("rotated box: the cutout covers its axis-aligned pixel bounds".to_string());
    }

    let sci = cut_plane(entry.arr(), &rect);
    let sci_header = entry.header().map(|h| shift_header(h, &rect));

    let err = match (include_err, companions.err.as_ref()) {
        (false, _) => None,
        (true, Some(e)) => Some((cut_plane(e.arr(), &rect), e.header().map(|h| shift_header(h, &rect)))),
        (true, None) => {
            warnings.push("no ERR companion plane found; the ERR extension was skipped".to_string());
            None
        }
    };

    let dq = match (include_dq, companions.dq.as_ref()) {
        (false, _) => None,
        (true, Some((e, table))) => {
            let plane = e.int_plane().context("DQ companion has no integer plane")?;
            let bits = cut_int_plane(&plane.bits, &rect, padded_dq_bits(*table)?);
            let pixels = if plane.signed { IntPixels::Signed(bits.mapv(|b| b as i32)) } else { IntPixels::Unsigned(bits) };
            Some((pixels, e.header().map(|h| shift_header(h, &rect))))
        }
        (true, None) => {
            let table = entry.header().map(DqTable::select).unwrap_or(DqTable::Unknown);
            warnings.push(format!(
                "no DQ companion plane found; wrote a synthetic DQ ({}) that flags only the padded pixels",
                table.label()
            ));
            let bits = padding_plane(&rect, img_w, img_h, padded_dq_bits(table)?);
            Some((IntPixels::Unsigned(bits), synthetic_dq_header(sci_header.as_ref())))
        }
    };

    let primary = source_primary_header(path, entry.header());
    let mut hdus = vec![ImageHdu { data: HduData::F32(&sci), header: sci_header.as_ref(), extname: EXTNAME_SCI, extver: 1 }];
    if let Some((arr, header)) = err.as_ref() {
        hdus.push(ImageHdu { data: HduData::F32(arr), header: header.as_ref(), extname: EXTNAME_ERR, extver: 1 });
    }
    if let Some((pixels, header)) = dq.as_ref() {
        let data = match pixels {
            IntPixels::Unsigned(bits) => HduData::U32(bits),
            IntPixels::Signed(bits) => HduData::I32(bits),
        };
        hdus.push(ImageHdu { data, header: header.as_ref(), extname: EXTNAME_DQ, extver: 1 });
    }
    write_mef_images(&target, Some(&primary), &hdus)?;
    GLOBAL_IMAGE_CACHE.remove_prefix(&target);

    let (ltv1, ltv2) = reported_ltv(sci_header.as_ref(), &rect);
    Ok(CutoutExport {
        hdus: hdus.iter().map(|h| h.extname.to_string()).collect(),
        output_path: target,
        rect,
        fraction_on_image: fraction_on_image(&rect, img_w, img_h),
        ltv1,
        ltv2,
        rotated_box_used_bounds: region_rect.rotated_box_used_bounds,
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[tauri::command]
pub async fn export_cutout_cmd(
    path: String,
    output_path: Option<String>,
    output_dir: Option<String>,
    region: RegionShape,
    include_err: Option<bool>,
    include_dq: Option<bool>,
    sky: Option<bool>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let report = export_cutout(
            &path,
            output_path.as_deref().unwrap_or(""),
            output_dir.as_deref(),
            &region,
            include_err.unwrap_or(true),
            include_dq.unwrap_or(true),
            sky.unwrap_or(false),
        )?;
        Ok(serde_json::to_value(&report)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::reader::test_fixtures::sci_err_dq_mef_with_dq_cards;
    use crate::infra::fits::reader::{
        extract_header_by_index, extract_image_mmap_by_index, extract_int_plane_by_index, list_extensions,
    };
    use crate::infra::fits::writer::write_fits_mono;

    const COLS: usize = 6;
    const ROWS: usize = 5;

    fn unsigned_to_raw(v: u32) -> i32 {
        (v as i64 - 2147483648) as i32
    }

    fn jwst_mef(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        let mut dq: Vec<i32> = (0..COLS * ROWS).map(|i| unsigned_to_raw((i as u32) * 2)).collect();
        dq[2] = unsigned_to_raw(1 << 31);
        sci_err_dq_mef_with_dq_cards(&path, COLS, ROWS, dq, vec![("TELESCOP", "'JWST'".into())]);
        path.to_str().unwrap().to_string()
    }

    fn overhanging_box() -> RegionShape {
        RegionShape::Box { x: 4.5, y: 1.5, width: 6.0, height: 5.0, angle: 0.0 }
    }

    #[test]
    fn export_rejects_a_box_beyond_the_plane_budget_before_allocating() {
        let dir = tempfile::tempdir().unwrap();
        let path = jwst_mef(&dir, "budget.fits");
        let huge = RegionShape::Box { x: 100.0, y: 100.0, width: 100_000.0, height: 100_000.0, angle: 0.0 };
        let err = export_cutout(&path, "", Some(dir.path().to_str().unwrap()), &huge, false, false, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("exceeds the memory budget"), "{err}");
        assert!(!dir.path().join("budget_cutout_-49900_-49900_100000x100000.fits").exists());
    }

    #[test]
    fn padded_dq_bits_follow_the_table() {
        assert_eq!(padded_dq_bits(DqTable::Jwst).unwrap(), 1 | 512);
        assert_eq!(padded_dq_bits(DqTable::Roman).unwrap(), 1 | 512);
        assert_eq!(padded_dq_bits(DqTable::Unknown).unwrap(), 1 | 512);
        assert_eq!(padded_dq_bits(DqTable::Hst).unwrap(), 2 | 8);
    }

    #[test]
    fn cutout_file_name_encodes_the_rect() {
        let r = CutoutRect { x0: -2, y0: 30, width: 40, height: 12 };
        assert_eq!(cutout_file_name("jw01234_cal_hdu1", &r), "jw01234_cal_hdu1_cutout_-2_30_40x12.fits");
    }

    #[test]
    fn target_path_resolution_uses_directories_and_refuses_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = jwst_mef(&dir, "src.fits");
        let key = format!("{}#hdu=1", src);
        let r = CutoutRect { x0: 2, y0: -1, width: 6, height: 6 };
        let dir_str = dir.path().to_str().unwrap().to_string();

        let explicit = resolve_target_path(&key, &format!("{}/named.fits", dir_str), None, &r).unwrap();
        assert!(explicit.ends_with("named.fits"));
        let in_dir = resolve_target_path(&key, &dir_str, None, &r).unwrap();
        assert!(in_dir.ends_with("src_hdu1_cutout_2_-1_6x6.fits"), "{in_dir}");
        let from_output_dir = resolve_target_path(&key, "", Some(&dir_str), &r).unwrap();
        assert_eq!(from_output_dir, in_dir);
        assert!(resolve_target_path(&key, "", None, &r).is_err());
        let refused = resolve_target_path(&key, &src, None, &r).unwrap_err();
        assert!(refused.to_string().contains("source"), "{refused}");
        let refused = resolve_target_path(&key, &src.replace('\\', "/"), None, &r).unwrap_err();
        assert!(refused.to_string().contains("source"), "{refused}");
    }

    #[test]
    fn export_writes_sci_err_dq_with_nan_and_flagged_padding() {
        let dir = tempfile::tempdir().unwrap();
        let src = jwst_mef(&dir, "cal.fits");
        let key = format!("{}#hdu=1", src);
        let out = dir.path().join("cut.fits").to_str().unwrap().to_string();

        let report = export_cutout(&key, &out, None, &overhanging_box(), true, true, false).unwrap();
        assert_eq!(report.output_path, out);
        assert_eq!(report.rect, CutoutRect { x0: 2, y0: -1, width: 6, height: 6 });
        assert!((report.fraction_on_image - 20.0 / 36.0).abs() < 1e-12);
        assert_eq!(report.hdus, vec!["SCI", "ERR", "DQ"]);
        assert_eq!(report.ltv1, -2.0);
        assert_eq!(report.ltv2, 1.0);
        assert!(!report.rotated_box_used_bounds);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let file = std::fs::File::open(&out).unwrap();
        let exts = list_extensions(&file).unwrap();
        assert_eq!(exts.len(), 4);
        assert_eq!(exts[0].naxis, 0);
        assert_eq!(exts[1].extname.as_deref(), Some("SCI"));
        assert_eq!(exts[2].extname.as_deref(), Some("ERR"));
        assert_eq!(exts[3].extname.as_deref(), Some("DQ"));

        let sci = extract_image_mmap_by_index(&file, 1).unwrap().image;
        assert_eq!(sci.dim(), (6, 6));
        assert!(sci[[0, 0]].is_nan());
        assert!(sci[[1, 4]].is_nan());
        assert!(sci[[1, 5]].is_nan());
        assert_eq!(sci[[1, 0]], 2.0);
        assert_eq!(sci[[5, 3]], (4 * COLS + 5) as f32);
        assert_eq!(sci.iter().filter(|v| v.is_finite()).count(), 20);

        let err = extract_image_mmap_by_index(&file, 2).unwrap().image;
        assert!(err[[0, 0]].is_nan());
        assert_eq!(err[[1, 0]], 1.0);
        assert_eq!(err.iter().filter(|v| v.is_finite()).count(), 20);

        let dq = extract_int_plane_by_index(&file, 3).unwrap();
        assert!(!dq.signed);
        assert_eq!(dq.bits.dim(), (6, 6));
        assert_eq!(dq.bits[[0, 0]], 1 | 512);
        assert_eq!(dq.bits[[1, 4]], 1 | 512);
        assert_eq!(dq.bits[[1, 0]], 1 << 31);
        assert_eq!(dq.bits[[2, 1]], ((COLS + 3) as u32) * 2);
        assert_eq!(dq.bits.iter().filter(|&&v| v == (1 | 512)).count(), 16);

        let sci_hdr = extract_header_by_index(&file, 1).unwrap();
        assert_eq!(sci_hdr.get("EXTNAME"), Some("SCI"));
        assert_eq!(sci_hdr.get_i64("EXTVER"), Some(1));
        assert_eq!(sci_hdr.get_f64("LTV1"), Some(-2.0));
        assert_eq!(sci_hdr.get_f64("LTV2"), Some(1.0));
        assert_eq!(sci_hdr.get_f64("LTM1_1"), Some(1.0));
        assert_eq!(sci_hdr.get("BUNIT"), Some("MJy/sr"));
        let dq_hdr = extract_header_by_index(&file, 3).unwrap();
        assert_eq!(dq_hdr.get("EXTNAME"), Some("DQ"));
        assert_eq!(dq_hdr.get_f64("LTV1"), Some(-2.0));
        assert_eq!(dq_hdr.get("TELESCOP"), Some("JWST"));
        let primary = extract_header_by_index(&file, 0).unwrap();
        assert_eq!(primary.get_i64("NAXIS"), Some(0));
        assert!(primary.get("EXTNAME").is_none());
        assert!(primary.get("BUNIT").is_none());
        assert!(primary.get("LTV1").is_none());
    }

    #[test]
    fn export_without_companions_synthesises_dq_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let src = jwst_mef(&dir, "lonely.fits");
        let key = format!("{}#hdu=4", src);
        let out_dir = dir.path().to_str().unwrap().to_string();

        let report = export_cutout(&key, "", Some(&out_dir), &overhanging_box(), true, true, false).unwrap();
        assert!(report.output_path.ends_with("lonely_hdu4_cutout_2_-1_6x6.fits"), "{}", report.output_path);
        assert_eq!(report.hdus, vec!["SCI", "DQ"]);
        assert_eq!(report.warnings.len(), 2);
        assert!(report.warnings.iter().any(|w| w.contains("ERR")));
        assert!(report.warnings.iter().any(|w| w.contains("DQ")));

        let file = std::fs::File::open(&report.output_path).unwrap();
        let exts = list_extensions(&file).unwrap();
        assert_eq!(exts.len(), 3);
        assert_eq!(exts[2].extname.as_deref(), Some("DQ"));
        let dq = extract_int_plane_by_index(&file, 2).unwrap();
        assert_eq!(dq.bits.iter().filter(|&&v| v == (1 | 512)).count(), 16);
        assert_eq!(dq.bits.iter().filter(|&&v| v == 0).count(), 20);

        let sci_only = export_cutout(&key, "", Some(&out_dir), &overhanging_box(), false, false, false).unwrap();
        assert_eq!(sci_only.hdus, vec!["SCI"]);
        assert!(sci_only.warnings.is_empty());
        let file = std::fs::File::open(&sci_only.output_path).unwrap();
        assert_eq!(list_extensions(&file).unwrap().len(), 2);
    }

    #[test]
    fn export_rejects_non_box_regions_and_the_source_as_target() {
        let dir = tempfile::tempdir().unwrap();
        let src = jwst_mef(&dir, "reject.fits");
        let key = format!("{}#hdu=1", src);
        let circle = RegionShape::Circle { x: 2.0, y: 2.0, r: 1.0 };
        let err = export_cutout(&key, "", Some(dir.path().to_str().unwrap()), &circle, false, false, false).unwrap_err();
        assert!(err.to_string().contains("box"), "{err}");
        let err = export_cutout(&key, &src, None, &overhanging_box(), false, false, false).unwrap_err();
        assert!(err.to_string().contains("source"), "{err}");
        let sky = export_cutout(&key, "", Some(dir.path().to_str().unwrap()), &overhanging_box(), false, false, true).unwrap_err();
        assert!(sky.to_string().contains("WCS"), "{sky}");
    }

    #[test]
    fn rotated_box_export_uses_the_bounds_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let src = jwst_mef(&dir, "rotated.fits");
        let key = format!("{}#hdu=1", src);
        let rotated = RegionShape::Box { x: 3.0, y: 2.0, width: 4.0, height: 2.0, angle: 30.0 };
        let report = export_cutout(&key, "", Some(dir.path().to_str().unwrap()), &rotated, false, false, false).unwrap();
        assert!(report.rotated_box_used_bounds);
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("rotated"));
        let b = rotated.bounds();
        assert_eq!(report.rect, CutoutRect { x0: b.x0, y0: b.y0, width: (b.x1 - b.x0 + 1) as usize, height: (b.y1 - b.y0 + 1) as usize });
        assert!(report.output_path.ends_with(&cutout_file_name("rotated_hdu1", &report.rect)));
    }

    #[test]
    fn single_hdu_parent_keeps_its_plane_cards_out_of_the_cutout_primary() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("subarray.fits").to_str().unwrap().to_string();
        let mut header = HduHeader::empty();
        for (k, v) in [
            ("TELESCOP", "JWST"), ("INSTRUME", "NIRCAM"), ("BUNIT", "MJy/sr"), ("LTV1", "-100"), ("LTV2", "-7"),
            ("DATAMIN", "0"), ("CRPIX1", "3"), ("CRPIX2", "3"), ("CRVAL1", "150"), ("CRVAL2", "2"),
        ] {
            header.set(k, v.to_string());
        }
        let pixels = Array2::from_shape_fn((ROWS, COLS), |(y, x)| (y * COLS + x) as f32);
        write_fits_mono(&src, &pixels, Some(&header)).unwrap();

        let out = dir.path().join("cut.fits").to_str().unwrap().to_string();
        let report = export_cutout(&src, &out, None, &overhanging_box(), false, false, false).unwrap();
        assert_eq!(report.rect, CutoutRect { x0: 2, y0: -1, width: 6, height: 6 });
        assert_eq!(report.hdus, vec!["SCI"]);
        assert_eq!((report.ltv1, report.ltv2), (-102.0, -6.0));

        let file = std::fs::File::open(&out).unwrap();
        let primary = extract_header_by_index(&file, 0).unwrap();
        assert_eq!(primary.get_i64("NAXIS"), Some(0));
        assert_eq!(primary.get("TELESCOP"), Some("JWST"));
        assert_eq!(primary.get("INSTRUME"), Some("NIRCAM"));
        for gone in ["LTV1", "LTV2", "BUNIT", "DATAMIN", "CRPIX1", "CRVAL1", "NAXIS1"] {
            assert!(primary.get(gone).is_none(), "{gone}");
        }
        let sci = extract_header_by_index(&file, 1).unwrap();
        assert_eq!(sci.get_f64("LTV1"), Some(-102.0));
        assert_eq!(sci.get_f64("LTV2"), Some(-6.0));
        assert_eq!(sci.get_f64("CRPIX1"), Some(1.0));
        assert_eq!(sci.get_f64("CRPIX2"), Some(4.0));
        assert_eq!(sci.get("BUNIT"), Some("MJy/sr"));
        assert_eq!(sci.get("TELESCOP"), Some("JWST"));
        assert!(extract_image_mmap_by_index(&file, 1).unwrap().image[[0, 0]].is_nan());
    }

    #[test]
    fn provenance_only_drops_wcs_and_plane_cards_but_keeps_observation_metadata() {
        let mut merged = HduHeader::empty();
        for (k, v) in [
            ("TELESCOP", "JWST"), ("INSTRUME", "NIRCAM"), ("DATE-OBS", "2024-01-01"), ("FILTER", "F200W"),
            ("CRPIX1", "10"), ("CD1_1", "1e-5"), ("CTYPE1", "RA---TAN"), ("BUNIT", "MJy/sr"),
            ("EXTNAME", "SCI"), ("EXTVER", "1"), ("LTV1", "-3"), ("BZERO", "0"),
        ] {
            merged.set(k, v.to_string());
        }
        let primary = provenance_only(&merged);
        assert_eq!(primary.get("TELESCOP"), Some("JWST"));
        assert_eq!(primary.get("INSTRUME"), Some("NIRCAM"));
        assert_eq!(primary.get("DATE-OBS"), Some("2024-01-01"));
        assert_eq!(primary.get("FILTER"), Some("F200W"));
        for gone in ["CRPIX1", "CD1_1", "CTYPE1", "BUNIT", "EXTNAME", "EXTVER", "LTV1", "BZERO"] {
            assert!(primary.get(gone).is_none(), "{gone}");
            assert!(!primary.cards.iter().any(|(k, _)| k == gone), "{gone}");
        }
        assert!(provenance_only(&HduHeader::empty()).cards.is_empty());
    }
}
