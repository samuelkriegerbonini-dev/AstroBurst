use std::fs::File;
use std::path::Path;

use anyhow::{bail, Context, Result};
use ndarray::Array2;

use crate::core::imaging::dq_flags::DqTable;
use crate::core::imaging::stats::compute_image_stats;
use crate::infra::asdf::converter::{auto_data_key, is_asdf_file, AsdfImage};
use crate::infra::asdf::AsdfFile;
use crate::infra::asdf_bridge::{
    companion_key, extract_plane_from_asdf, extract_plane_from_open_asdf, list_asdf_arrays,
};
use crate::infra::cache::{ImageCache, ImageEntry, PlaneLoad};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::infra::fits::reader::{
    auto_hdu_index, extract_header_by_index_merged, extract_header_mmap,
    extract_image_mmap_by_index, extract_int_plane_by_index, list_extensions, HduInfo,
    MmapImageResult,
};
use crate::types::constants::{EXTNAME_DQ, EXTNAME_ERR};
use crate::types::image::IntPlane;
use crate::types::image_ref::{ImageRef, PlaneSelector};
use crate::types::HduHeader;

pub const CALIB_PATTERNS: &[&str] = &[
    "distortion", "filteroffset", "sirskernel", "photom",
    "flat", "dark", "bias", "readnoise", "gain", "linearity",
    "saturation", "superbias", "ipc", "area", "specwcs",
    "regions", "wavelengthrange", "trappars", "mask",
    "drizpars", "throughput", "psfmask",
];

pub fn is_calib_ref_asdf(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.starts_with("jwst_")
        && name.ends_with(".asdf")
        && CALIB_PATTERNS.iter().any(|p| name.contains(p))
}

pub fn bail_if_calib(path: &Path) -> Result<()> {
    if is_calib_ref_asdf(path) {
        bail!(
            "Calibration reference file (no image data): {}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown")
        );
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaneInfo {
    pub kind: PlaneSelector,
    pub extname: Option<String>,
    pub extver: Option<i64>,
    pub is_dq: bool,
    pub is_err: bool,
    pub bitpix: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Companions {
    pub dq: Option<ImageRef>,
    pub err: Option<ImageRef>,
}

pub struct LoadedPlane {
    pub arr: Array2<f32>,
    pub header: HduHeader,
    pub info: PlaneInfo,
    pub int_plane: Option<IntPlane>,
    pub companions: Companions,
    pub extensions: Vec<HduInfo>,
    pub _tmp: Option<tempfile::TempDir>,
}

impl LoadedPlane {
    pub fn into_plane_load(self) -> PlaneLoad {
        let stats = compute_image_stats(&self.arr);
        PlaneLoad {
            arr: self.arr,
            stats,
            header: self.header,
            int_plane: self.int_plane,
            info: Some(self.info),
            companions: Some(self.companions),
        }
    }
}

#[derive(Default)]
pub struct LoadedCompanions {
    pub dq: Option<(ImageEntry, DqTable)>,
    pub err: Option<ImageEntry>,
}

fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

pub fn is_dq_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.eq_ignore_ascii_case(EXTNAME_DQ)
        || trimmed.eq_ignore_ascii_case("MASK")
        || last_segment(trimmed).eq_ignore_ascii_case("dq")
}

pub fn is_err_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.eq_ignore_ascii_case(EXTNAME_ERR) || last_segment(trimmed).eq_ignore_ascii_case("err")
}

fn plane_info_for(kind: PlaneSelector, ext: Option<&HduInfo>) -> PlaneInfo {
    let extname = ext.and_then(|e| e.extname.clone());
    let name = extname.as_deref().unwrap_or("");
    PlaneInfo {
        kind,
        is_dq: is_dq_name(name),
        is_err: is_err_name(name),
        extver: ext.and_then(|e| e.extver),
        bitpix: ext.map(|e| e.bitpix).unwrap_or(0),
        extname,
    }
}

fn missing_asdf_data(e: &anyhow::Error) -> bool {
    e.to_string().contains("Missing field: data array")
}

fn companion_fits_path(source: &Path) -> Option<std::path::PathBuf> {
    let fits = source.with_extension("fits");
    fits.exists().then_some(fits)
}

fn holds_single_plane(info: &HduInfo) -> bool {
    info.has_data && (info.naxis == 2 || (info.naxis == 3 && info.naxis3 == 1))
}

fn pick_companion(exts: &[HduInfo], active: &HduInfo, name: &str) -> Option<usize> {
    let candidates: Vec<&HduInfo> = exts
        .iter()
        .filter(|e| {
            holds_single_plane(e)
                && e.extname.as_deref().is_some_and(|n| n.trim().eq_ignore_ascii_case(name))
        })
        .collect();
    match active.extver {
        Some(v) => candidates.iter().find(|e| e.extver == Some(v)).map(|e| e.index),
        None => candidates.first().map(|e| e.index),
    }
}

fn fits_companions(ref_path: &str, exts: &[HduInfo], idx: usize) -> Result<Companions> {
    let active = exts
        .get(idx)
        .with_context(|| format!("HDU index {} out of range (file has {} HDUs)", idx, exts.len()))?;
    let name = active.extname.as_deref().unwrap_or("");
    let active_ref = ImageRef::hdu(ref_path, idx);
    let dq = if is_dq_name(name) {
        Some(active_ref.clone())
    } else {
        pick_companion(exts, active, EXTNAME_DQ).map(|i| ImageRef::hdu(ref_path, i))
    };
    let err = if is_err_name(name) {
        Some(active_ref)
    } else {
        pick_companion(exts, active, EXTNAME_ERR).map(|i| ImageRef::hdu(ref_path, i))
    };
    Ok(Companions { dq, err })
}

fn asdf_companions(ref_path: &str, asdf: &AsdfFile, data_key: &str) -> Companions {
    let find = |suffix: &str| {
        let key = companion_key(data_key, suffix);
        AsdfImage::array_exists(&asdf.tree, &key).then(|| ImageRef::array(ref_path, &key))
    };
    let active = ImageRef::array(ref_path, data_key);
    let dq = if is_dq_name(data_key) { Some(active.clone()) } else { find("dq") };
    let err = if is_err_name(data_key) { Some(active) } else { find("err") };
    Companions { dq, err }
}

fn fits_plane(file: &File, index: usize, want_int: bool) -> Result<(MmapImageResult, PlaneInfo, Option<IntPlane>)> {
    let result = extract_image_mmap_by_index(file, index)?;
    let info = plane_info_for(PlaneSelector::Hdu(index), result.extensions.get(index));
    let int_plane = if want_int && info.is_dq {
        extract_int_plane_by_index(file, index).ok()
    } else {
        None
    };
    Ok((result, info, int_plane))
}

fn fits_loaded(
    source: &Path,
    index: Option<usize>,
    tmp: Option<tempfile::TempDir>,
    ref_path: Option<&str>,
) -> Result<LoadedPlane> {
    let file = File::open(source).with_context(|| format!("Failed to open {}", source.display()))?;
    let idx = match index {
        Some(i) => i,
        None => auto_hdu_index(&file)?,
    };
    let (result, info, int_plane) = fits_plane(&file, idx, true)?;
    let companions = match ref_path {
        Some(p) => fits_companions(p, &result.extensions, idx)?,
        None => Companions::default(),
    };
    Ok(LoadedPlane {
        arr: result.image,
        header: result.header,
        info,
        int_plane,
        companions,
        extensions: result.extensions,
        _tmp: tmp,
    })
}

fn asdf_loaded(source: &Path, key: Option<&str>, tmp: Option<tempfile::TempDir>, ref_path: &str) -> Result<LoadedPlane> {
    let asdf = AsdfFile::open(source).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
    let result = extract_plane_from_open_asdf(&asdf, key)?;
    let data_key = result.selected_extension.clone().unwrap_or_default();
    let ext = result.extensions.iter().find(|e| e.extname.as_deref() == Some(data_key.as_str()));
    let info = plane_info_for(PlaneSelector::Array(data_key.clone()), ext);
    let int_plane = if info.is_dq {
        AsdfImage::load_array_int(&asdf, &data_key).ok().flatten()
    } else {
        None
    };
    let companions = asdf_companions(ref_path, &asdf, &data_key);
    Ok(LoadedPlane {
        arr: result.image,
        header: result.header,
        info,
        int_plane,
        companions,
        extensions: result.extensions,
        _tmp: tmp,
    })
}

pub fn load_plane(r: &ImageRef) -> Result<LoadedPlane> {
    let (source, tmp) = resolve_single_image(&r.path)?;
    if is_asdf_file(&source) {
        bail_if_calib(&source)?;
        return match &r.plane {
            PlaneSelector::Hdu(_) => bail!("ASDF files have no HDU index; use #array=<key>"),
            PlaneSelector::Array(k) => asdf_loaded(&source, Some(k), tmp, &r.path),
            PlaneSelector::Auto => match asdf_loaded(&source, None, None, &r.path) {
                Ok(mut loaded) => {
                    loaded._tmp = tmp;
                    Ok(loaded)
                }
                Err(e) if missing_asdf_data(&e) => match companion_fits_path(&source) {
                    Some(fits) => fits_loaded(&fits, None, tmp, None),
                    None => bail!("ASDF has no image data and no companion .fits found"),
                },
                Err(e) => Err(e),
            },
        };
    }
    match &r.plane {
        PlaneSelector::Hdu(n) => fits_loaded(&source, Some(*n), tmp, Some(&r.path)),
        PlaneSelector::Auto => fits_loaded(&source, None, tmp, Some(&r.path)),
        PlaneSelector::Array(_) => bail!("FITS files have no ASDF arrays; use #hdu=<n>"),
    }
}

pub fn load_plane_header(r: &ImageRef) -> Result<HduHeader> {
    let (source, _tmp) = resolve_single_image(&r.path)?;
    if is_asdf_file(&source) {
        bail_if_calib(&source)?;
        return match &r.plane {
            PlaneSelector::Hdu(_) => bail!("ASDF files have no HDU index; use #array=<key>"),
            PlaneSelector::Array(k) => Ok(extract_plane_from_asdf(&source, Some(k))?.header),
            PlaneSelector::Auto => match extract_plane_from_asdf(&source, None) {
                Ok(res) => Ok(res.header),
                Err(e) if missing_asdf_data(&e) => match companion_fits_path(&source) {
                    Some(fits) => {
                        let file = File::open(&fits)?;
                        extract_header_mmap(&file)
                    }
                    None => bail!("ASDF has no image data and no companion .fits found"),
                },
                Err(e) => Err(e),
            },
        };
    }
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    match &r.plane {
        PlaneSelector::Hdu(n) => extract_header_by_index_merged(&file, *n),
        PlaneSelector::Auto => extract_header_mmap(&file),
        PlaneSelector::Array(_) => bail!("FITS files have no ASDF arrays; use #hdu=<n>"),
    }
}

pub fn load_companions_into(
    cache: &ImageCache,
    active: &ImageEntry,
    load: impl Fn(&ImageRef) -> Result<PlaneLoad>,
) -> LoadedCompanions {
    let Some(comps) = active.companions() else {
        return LoadedCompanions::default();
    };
    let dims = active.arr().dim();
    let fetch = |r: &ImageRef| -> Option<ImageEntry> {
        let key = r.cache_key();
        match cache.get_or_load_plane(&key, || load(r)) {
            Ok(e) if e.arr().dim() == dims => Some(e),
            Ok(_) => None,
            Err(e) => {
                log::warn!("companion plane {} failed to load: {:#}", key, e);
                None
            }
        }
    };
    let dq = comps.dq.as_ref().and_then(fetch).filter(|e| e.int_plane().is_some()).map(|e| {
        let table = e.header().map(DqTable::select).unwrap_or(DqTable::Unknown);
        (e, table)
    });
    let err = comps.err.as_ref().and_then(fetch);
    LoadedCompanions { dq, err }
}

pub fn resolve_plane_info(r: &ImageRef) -> Result<PlaneInfo> {
    let (source, _tmp) = resolve_single_image(&r.path)?;
    if is_asdf_file(&source) {
        let key = match &r.plane {
            PlaneSelector::Hdu(_) => bail!("ASDF files have no HDU index; use #array=<key>"),
            PlaneSelector::Array(k) => k.clone(),
            PlaneSelector::Auto => {
                let asdf = AsdfFile::open(&source).map_err(|e| anyhow::anyhow!("ASDF load failed: {}", e))?;
                auto_data_key(&asdf).context("ASDF load failed: Missing field: data array")?
            }
        };
        let arrays = list_asdf_arrays(&source)?;
        let ext = arrays.iter().find(|e| e.extname.as_deref() == Some(key.as_str()));
        let mut info = plane_info_for(PlaneSelector::Array(key.clone()), ext);
        if info.extname.is_none() {
            info.extname = Some(key.clone());
            info.is_dq = is_dq_name(&key);
            info.is_err = is_err_name(&key);
        }
        return Ok(info);
    }
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    let idx = match &r.plane {
        PlaneSelector::Hdu(n) => *n,
        PlaneSelector::Auto => auto_hdu_index(&file)?,
        PlaneSelector::Array(_) => bail!("FITS files have no ASDF arrays; use #hdu=<n>"),
    };
    let exts = list_extensions(&file)?;
    let ext = exts
        .get(idx)
        .with_context(|| format!("HDU index {} out of range (file has {} HDUs)", idx, exts.len()))?;
    Ok(plane_info_for(PlaneSelector::Hdu(idx), Some(ext)))
}

pub fn list_planes(path: &str) -> Result<(Vec<HduInfo>, bool)> {
    let r = ImageRef::parse(path);
    let (source, _tmp) = resolve_single_image(&r.path)?;
    if is_asdf_file(&source) {
        return Ok((list_asdf_arrays(&source)?, true));
    }
    let file = File::open(&source).with_context(|| format!("Failed to open {}", source.display()))?;
    Ok((list_extensions(&file)?, false))
}

pub fn plane_ref(path: &str, info: &HduInfo, is_asdf: bool) -> ImageRef {
    if is_asdf {
        ImageRef::array(path, info.extname.as_deref().unwrap_or(""))
    } else {
        ImageRef::hdu(path, info.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::fits::reader::test_fixtures::{
        cube_hdu, empty_primary_cards, ramp_f32, sci_err_dq_mef, write_raw_hdus, write_test_mef,
        HduData, TestHdu,
    };

    fn mef_path(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("jw_cal.fits");
        let mut dq = vec![0i32; 16];
        dq[0] = 1 - 2147483647 - 1;
        dq[3] = 3 - 2147483647 - 1;
        sci_err_dq_mef(&path, 4, 4, dq);
        path.to_str().unwrap().to_string()
    }

    fn write_asdf(dir: &tempfile::TempDir, name: &str, tree_yaml: &str) -> String {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#ASDF 1.0.0\n#ASDF_STANDARD 1.5.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n");
        bytes.extend_from_slice(tree_yaml.as_bytes());
        bytes.extend_from_slice(b"...\n");
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn companions_of(r: &ImageRef) -> Companions {
        load_plane(r).unwrap().companions
    }

    fn plane_load(r: &ImageRef) -> Result<PlaneLoad> {
        load_plane(r).map(LoadedPlane::into_plane_load)
    }

    const ASDF_TREE: &str = "data: !core/ndarray-1.0.0\n  data: [[1, 2], [3, 4]]\n  datatype: float32\ndq: !core/ndarray-1.0.0\n  data: [[0, 1], [0, 0]]\n  datatype: uint32\nerr: !core/ndarray-1.0.0\n  data: [[0.1, 0.2], [0.3, 0.4]]\n  datatype: float32\n";
    const ROMAN_TREE: &str = "roman:\n  meta:\n    instrument: {name: WFI}\n  data: !core/ndarray-1.0.0\n    data: [[1, 2], [3, 4]]\n    datatype: float32\n  dq: !core/ndarray-1.0.0\n    data: [[0, 1], [0, 0]]\n    datatype: uint32\n";

    #[test]
    fn load_plane_resolves_fits_companions_by_extver() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let c = companions_of(&ImageRef::hdu(&p, 1));
        assert_eq!(c.dq, Some(ImageRef::hdu(&p, 3)));
        assert_eq!(c.err, Some(ImageRef::hdu(&p, 2)));

        let c = companions_of(&ImageRef::hdu(&p, 4));
        assert_eq!(c, Companions::default());

        let c = companions_of(&ImageRef::hdu(&p, 3));
        assert_eq!(c.dq, Some(ImageRef::hdu(&p, 3)));
        assert_eq!(c.err, Some(ImageRef::hdu(&p, 2)));

        let c = companions_of(&ImageRef::auto(&p));
        assert_eq!(c.dq, Some(ImageRef::hdu(&p, 3)));
    }

    fn sci_2d_hdu(cols: usize, rows: usize) -> (Vec<(&'static str, String)>, Vec<u8>) {
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", cols.to_string()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", "'SCI     '".into()),
        ];
        let data: Vec<u8> = (0..cols * rows).flat_map(|i| (i as f32).to_be_bytes()).collect();
        (cards, data)
    }

    #[test]
    fn a_cube_companion_is_skipped_instead_of_being_read_as_its_first_plane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cube_companions.fits");
        write_raw_hdus(
            &path,
            &[
                (empty_primary_cards(), Vec::new()),
                sci_2d_hdu(4, 3),
                cube_hdu("ERR", 4, 3, 5, &[]),
                cube_hdu("DQ", 4, 3, 5, &[]),
            ],
        );
        let p = path.to_str().unwrap().to_string();

        let c = companions_of(&ImageRef::hdu(&p, 1));
        assert_eq!(
            c,
            Companions::default(),
            "a 3D ERR or DQ extension holds one plane per group read, none of which is the companion of plane 1"
        );

        let single_plane = dir.path().join("single_plane_companions.fits");
        write_raw_hdus(
            &single_plane,
            &[
                (empty_primary_cards(), Vec::new()),
                sci_2d_hdu(4, 3),
                cube_hdu("ERR", 4, 3, 1, &[]),
            ],
        );
        let sp = single_plane.to_str().unwrap().to_string();
        assert_eq!(
            companions_of(&ImageRef::hdu(&sp, 1)).err,
            Some(ImageRef::hdu(&sp, 2)),
            "a rank-3 extension of a single plane is still a usable companion"
        );
    }

    #[test]
    fn load_plane_decodes_int_plane_only_for_dq() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let dq = load_plane(&ImageRef::hdu(&p, 3)).unwrap();
        assert!(dq.info.is_dq);
        assert!(!dq.info.is_err);
        assert_eq!(dq.info.kind, PlaneSelector::Hdu(3));
        assert_eq!(dq.info.extname.as_deref(), Some("DQ"));
        assert_eq!(dq.info.extver, Some(1));
        assert_eq!(dq.info.bitpix, 32);
        let plane = dq.int_plane.as_ref().unwrap();
        assert_eq!(plane.bits[[0, 0]], 1);
        assert_eq!(plane.bits[[0, 3]], 3);
        assert_eq!(dq.arr[[0, 3]], 3.0);
        assert_eq!(dq.extensions.len(), 5);

        let sci = load_plane(&ImageRef::hdu(&p, 1)).unwrap();
        assert!(sci.int_plane.is_none());
        assert!(!sci.info.is_dq);
        assert_eq!(sci.header.get("BUNIT"), Some("MJy/sr"));

        let auto = load_plane(&ImageRef::auto(&p)).unwrap();
        assert_eq!(auto.info.kind, PlaneSelector::Hdu(1));
        assert_eq!(auto.info.extname.as_deref(), Some("SCI"));

        let err = load_plane(&ImageRef::hdu(&p, 2)).unwrap();
        assert!(err.info.is_err);
        assert!(load_plane(&ImageRef::array(&p, "dq")).is_err());
        assert!(load_plane(&ImageRef::hdu(&p, 7)).is_err());
    }

    #[test]
    fn into_plane_load_carries_info_and_companions() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let load = load_plane(&ImageRef::hdu(&p, 3)).unwrap().into_plane_load();
        assert_eq!(load.stats.min, 1.0);
        assert_eq!(load.stats.valid_count, 16);
        assert!(load.int_plane.is_some());
        assert_eq!(load.info.as_ref().unwrap().kind, PlaneSelector::Hdu(3));
        assert_eq!(load.companions.as_ref().unwrap().dq, Some(ImageRef::hdu(&p, 3)));
        assert_eq!(load.header.get("EXTNAME"), Some("DQ"));
    }

    #[test]
    fn load_companions_into_uses_the_entry_companions() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let cache = ImageCache::new(8, usize::MAX);
        let key = format!("{}#hdu=1", p);
        let active = cache.get_or_load_plane(&key, || plane_load(&ImageRef::hdu(&p, 1))).unwrap();

        let comps = load_companions_into(&cache, &active, plane_load);
        let (dq, table) = comps.dq.expect("dq companion");
        assert_eq!(table, DqTable::Unknown);
        assert_eq!(dq.int_plane().unwrap().bits[[0, 3]], 3);
        assert_eq!(comps.err.expect("err companion").arr()[[0, 1]], 0.5);
        assert!(cache.contains(&format!("{}#hdu=3", p)));
        assert!(cache.contains(&format!("{}#hdu=2", p)));

        let again = load_companions_into(&cache, &active, |_| panic!("companions must come from the cache"));
        assert!(again.dq.is_some());
        assert!(again.err.is_some());

        let lonely = cache
            .get_or_load_plane(&format!("{}#hdu=4", p), || plane_load(&ImageRef::hdu(&p, 4)))
            .unwrap();
        let none = load_companions_into(&cache, &lonely, plane_load);
        assert!(none.dq.is_none());
        assert!(none.err.is_none());

        let synthetic = cache
            .get_or_load("__composite_r", || Ok((Array2::<f32>::zeros((4, 4)), compute_image_stats(&Array2::zeros((4, 4))))))
            .unwrap();
        let skipped = load_companions_into(&cache, &synthetic, |_| panic!("synthetic entries have no companions"));
        assert!(skipped.dq.is_none());
    }

    #[test]
    fn load_companions_into_drops_dq_with_mismatched_dims() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mismatch.fits");
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: 2, rows: 2, data: HduData::I32(vec![1; 4]), extra_cards: vec![] },
            ],
        );
        let p = path.to_str().unwrap().to_string();
        let cache = ImageCache::new(8, usize::MAX);
        let active = cache.get_or_load_plane(&p, || plane_load(&ImageRef::auto(&p))).unwrap();
        assert_eq!(active.companions().unwrap().dq, Some(ImageRef::hdu(&p, 2)));
        let comps = load_companions_into(&cache, &active, plane_load);
        assert!(comps.dq.is_none());
        assert!(comps.err.is_none());
    }

    #[test]
    fn load_plane_header_uses_requested_hdu() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let h = load_plane_header(&ImageRef::hdu(&p, 3)).unwrap();
        assert_eq!(h.get("EXTNAME"), Some("DQ"));
        assert_eq!(h.get("EXTEND"), Some("T"));
        let h = load_plane_header(&ImageRef::auto(&p)).unwrap();
        assert_eq!(h.get("EXTNAME"), Some("SCI"));
        assert!(load_plane_header(&ImageRef::array(&p, "x")).is_err());
    }

    #[test]
    fn asdf_companions_and_planes() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "a.asdf", ASDF_TREE);
        let c = companions_of(&ImageRef::auto(&p));
        assert_eq!(c.dq, Some(ImageRef::array(&p, "dq")));
        assert_eq!(c.err, Some(ImageRef::array(&p, "err")));
        let c = companions_of(&ImageRef::array(&p, "dq"));
        assert_eq!(c.dq, Some(ImageRef::array(&p, "dq")));
        assert_eq!(c.err, Some(ImageRef::array(&p, "err")));

        let loaded = load_plane(&ImageRef::array(&p, "dq")).unwrap();
        assert!(loaded.info.is_dq);
        assert_eq!(loaded.info.kind, PlaneSelector::Array("dq".into()));
        assert_eq!(loaded.info.bitpix, 32);
        assert_eq!(loaded.int_plane.as_ref().unwrap().bits[[0, 1]], 1);
        assert_eq!(loaded.extensions.len(), 3);

        let auto = load_plane(&ImageRef::auto(&p)).unwrap();
        assert_eq!(auto.info.kind, PlaneSelector::Array("data".into()));
        assert!(auto.int_plane.is_none());
        assert_eq!(load_plane_header(&ImageRef::array(&p, "err")).unwrap().get("EXTNAME"), Some("err"));
        assert!(load_plane(&ImageRef::hdu(&p, 1)).is_err());

        let r = write_asdf(&dir, "r.asdf", ROMAN_TREE);
        let c = companions_of(&ImageRef::auto(&r));
        assert_eq!(c.dq, Some(ImageRef::array(&r, "roman.dq")));
        assert_eq!(c.err, None);
        let c = companions_of(&ImageRef::array(&r, "roman.data"));
        assert_eq!(c.dq, Some(ImageRef::array(&r, "roman.dq")));
    }

    #[test]
    fn list_planes_and_plane_ref_for_both_formats() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let (exts, is_asdf) = list_planes(&format!("{}#hdu=2", p)).unwrap();
        assert!(!is_asdf);
        assert_eq!(exts.len(), 5);
        assert_eq!(plane_ref(&p, &exts[3], false), ImageRef::hdu(&p, 3));

        let a = write_asdf(&dir, "b.asdf", ASDF_TREE);
        let (exts, is_asdf) = list_planes(&a).unwrap();
        assert!(is_asdf);
        assert_eq!(exts.len(), 3);
        assert_eq!(plane_ref(&a, &exts[1], true), ImageRef::array(&a, "dq"));
    }

    #[test]
    fn resolve_plane_info_is_header_only_and_matches_load_plane() {
        let dir = tempfile::tempdir().unwrap();
        let p = mef_path(&dir);
        let auto = resolve_plane_info(&ImageRef::auto(&p)).unwrap();
        assert_eq!(auto.kind, PlaneSelector::Hdu(1));
        assert_eq!(auto.extname.as_deref(), Some("SCI"));
        assert_eq!(auto.extver, Some(1));
        let dq = resolve_plane_info(&ImageRef::hdu(&p, 3)).unwrap();
        assert!(dq.is_dq);
        assert_eq!(dq.bitpix, 32);
        assert!(resolve_plane_info(&ImageRef::hdu(&p, 8)).is_err());

        let a = write_asdf(&dir, "c.asdf", ROMAN_TREE);
        let info = resolve_plane_info(&ImageRef::auto(&a)).unwrap();
        assert_eq!(info.kind, PlaneSelector::Array("roman.data".into()));
        assert_eq!(info.extname.as_deref(), Some("roman.data"));
        let info = resolve_plane_info(&ImageRef::array(&a, "roman.dq")).unwrap();
        assert!(info.is_dq);
        assert_eq!(info.bitpix, 32);
        assert!(resolve_plane_info(&ImageRef::hdu(&a, 0)).is_err());
    }

    #[test]
    fn dq_and_err_name_rules() {
        assert!(is_dq_name("DQ"));
        assert!(is_dq_name("dq"));
        assert!(is_dq_name("MASK"));
        assert!(is_dq_name("roman.dq"));
        assert!(is_dq_name(" DQ "));
        assert!(!is_dq_name("SCI"));
        assert!(!is_dq_name("dqx"));
        assert!(is_err_name("ERR"));
        assert!(is_err_name("err"));
        assert!(is_err_name("roman.err"));
        assert!(!is_err_name("ERROR"));
        assert!(!is_err_name(""));
    }

    #[test]
    fn calibration_reference_asdf_bails() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "jwst_nircam_flat_0001.asdf", ASDF_TREE);
        let err = load_plane(&ImageRef::auto(&p)).err().expect("calibration reference must be rejected");
        assert!(err.to_string().contains("Calibration reference file"), "{err}");
        assert!(load_plane_header(&ImageRef::auto(&p)).is_err());
        assert!(is_calib_ref_asdf(Path::new("jwst_miri_photom_0042.asdf")));
        assert!(!is_calib_ref_asdf(Path::new("jw01234_cal.asdf")));
    }

    const RAMP_TREE: &str = "roman:\n  data: !core/ndarray-1.0.0\n    data: [[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]]]\n    datatype: float32\n";

    #[test]
    fn a_multi_plane_asdf_refusal_is_never_read_as_missing_data() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "jw_cal.asdf", RAMP_TREE);
        mef_path(&dir);

        let refusal = extract_plane_from_asdf(Path::new(&p), None)
            .err()
            .expect("a ramp must not load as a 2D image");
        assert!(
            !missing_asdf_data(&refusal),
            "the multi-plane refusal must stay clear of the missing-data phrase that triggers the companion fallback: {refusal:#}"
        );

        let err = load_plane(&ImageRef::auto(&p))
            .err()
            .expect("the refusal must surface instead of opening the companion .fits");
        assert!(format!("{err:#}").contains("not a single 2D image"), "{err:#}");
        let header_err = load_plane_header(&ImageRef::auto(&p))
            .err()
            .expect("the header path must refuse the ramp too");
        assert!(format!("{header_err:#}").contains("not a single 2D image"), "{header_err:#}");
    }

    #[test]
    fn an_explicit_array_ref_recovers_the_first_plane_of_a_multi_plane_ramp() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "jw_ramp.asdf", RAMP_TREE);

        let loaded = load_plane(&ImageRef::array(&p, "roman.data"))
            .expect("an explicit array ref is the recovery the queue retries with");
        assert_eq!(loaded.arr.dim(), (2, 2));
        assert_eq!(loaded.arr[[0, 0]], 1.0);
        assert_eq!(loaded.arr[[1, 1]], 4.0);
        assert_eq!(loaded.info.kind, PlaneSelector::Array("roman.data".into()));

        load_plane_header(&ImageRef::array(&p, "roman.data"))
            .expect("the header path must accept the same explicit ref");
    }

    #[test]
    fn asdf_without_arrays_falls_back_to_companion_fits() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_asdf(&dir, "jw_cal.asdf", "meta:\n  telescope: JWST\n");
        mef_path(&dir);
        let loaded = load_plane(&ImageRef::auto(&p)).unwrap();
        assert_eq!(loaded.info.kind, PlaneSelector::Hdu(1));
        assert_eq!(loaded.companions, Companions::default());
        assert_eq!(load_plane_header(&ImageRef::auto(&p)).unwrap().get("EXTNAME"), Some("SCI"));
        let lonely = write_asdf(&dir, "lonely.asdf", "meta:\n  telescope: JWST\n");
        assert!(load_plane(&ImageRef::auto(&lonely)).is_err());
    }
}
