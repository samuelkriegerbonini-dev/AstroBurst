use std::collections::HashMap;
use std::fs::File;
use std::io::ErrorKind;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::SystemTime;

use anyhow::Result;

use crate::core::astrometry::gwcs::{
    classify_wcs_node, find_wcs_node, parse_gwcs, wcsinfo_sip_residuals, GwcsOrigin, GwcsSource, WcsNodeClass,
};
use crate::core::astrometry::wcs::WcsTransform;
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::asdf::{AsdfFile, AsdfImage};
use crate::infra::fits::asdf_hdu::{find_asdf_hdu, read_asdf_hdu_bytes};
use crate::infra::fits::dispatcher::resolve_single_image;
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

pub type GwcsLookup = Result<Option<Arc<GwcsSource>>, String>;

type FileStamp = (u64, Option<SystemTime>);

const GWCS_CACHE_CAPACITY: usize = 16;

struct CacheEntry {
    stamp: FileStamp,
    lookup: GwcsLookup,
    last_used: u64,
}

struct GwcsCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    clock: AtomicU64,
}

impl GwcsCache {
    fn new() -> Self {
        Self { entries: Mutex::new(HashMap::new()), clock: AtomicU64::new(0) }
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, CacheEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed)
    }

    fn lookup(&self, path: &str) -> GwcsLookup {
        let image = ImageRef::parse(path);
        if image.is_synthetic() {
            return Ok(None);
        }
        let source = image.path;
        let stamp = file_stamp(&source);
        if let Some(stamp) = &stamp {
            if let Some(entry) = self.entries().get_mut(&source) {
                if entry.stamp == *stamp {
                    entry.last_used = self.tick();
                    return entry.lookup.clone();
                }
            }
        }
        let lookup = lookup_gwcs(&source);
        if let Some(stamp) = stamp {
            let mut entries = self.entries();
            if entries.len() >= GWCS_CACHE_CAPACITY && !entries.contains_key(&source) {
                let oldest = entries.iter().min_by_key(|(_, e)| e.last_used).map(|(k, _)| k.clone());
                if let Some(oldest) = oldest {
                    entries.remove(&oldest);
                }
            }
            entries.insert(source, CacheEntry { stamp, lookup: lookup.clone(), last_used: self.tick() });
        }
        lookup
    }

    fn invalidate(&self, path: &str) {
        self.entries().remove(&ImageRef::parse(path).path);
    }
}

static GWCS_CACHE: LazyLock<GwcsCache> = LazyLock::new(GwcsCache::new);

fn file_stamp(source: &str) -> Option<FileStamp> {
    std::fs::metadata(source).ok().map(|m| (m.len(), m.modified().ok()))
}

pub fn gwcs_for_path(path: &str) -> GwcsLookup {
    GWCS_CACHE.lookup(path)
}

pub fn invalidate_gwcs_cache(path: &str) {
    GWCS_CACHE.invalidate(path);
}

pub fn load_wcs(path: &str, header: &HduHeader) -> Result<WcsTransform> {
    match gwcs_for_path(path) {
        Ok(Some(source)) => match WcsTransform::from_gwcs(source, header) {
            Ok(wcs) => Ok(wcs),
            Err(e) => match WcsTransform::from_header(header) {
                Ok(wcs) => Ok(wcs.with_gwcs_refusal(format!("{e:#}"))),
                Err(_) => Err(e),
            },
        },
        Ok(None) => WcsTransform::from_header(header),
        Err(refusal) => WcsTransform::from_header(header).map(|wcs| wcs.with_gwcs_refusal(refusal)),
    }
}

fn lookup_gwcs(source: &str) -> GwcsLookup {
    let (readable, _tmp) = match resolve_single_image(source) {
        Ok(resolved) => resolved,
        Err(e) => return skipped(source, format!("{e:#}")),
    };
    let file = match File::open(&readable) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return skipped(source, e.to_string()),
    };
    if is_asdf_file(&readable) {
        return match AsdfFile::open(&readable) {
            Ok(asdf) => source_from_tree(&asdf, None),
            Err(e) => skipped(source, e.to_string()),
        };
    }
    let hdu = match find_asdf_hdu(&file) {
        Ok(Some(hdu)) => hdu,
        Ok(None) => return Ok(None),
        Err(e) => return skipped(source, format!("{e:#}")),
    };
    let cell = read_asdf_hdu_bytes(&file, &hdu)
        .map_err(|e| format!("{e:#}"))
        .and_then(|bytes| AsdfFile::from_bytes(bytes).map_err(|e| format!("HDU {} 'ASDF': {e}", hdu.index)));
    match cell {
        Ok(asdf) => source_from_tree(&asdf, Some(hdu.index)),
        Err(reason) => skipped(source, reason),
    }
}

fn skipped(source: &str, reason: String) -> GwcsLookup {
    log::debug!("{source}: gWCS lookup stopped before classification, the header serves the file: {reason}");
    Ok(None)
}

fn source_from_tree(asdf: &AsdfFile, hdu_index: Option<usize>) -> GwcsLookup {
    let Some((key, node)) = find_wcs_node(&asdf.tree) else {
        return Ok(None);
    };
    match classify_wcs_node(node) {
        WcsNodeClass::FitsEquivalent | WcsNodeClass::NotImaging2D(_) => Ok(None),
        WcsNodeClass::Imaging2D => {
            let pipeline =
                parse_gwcs(node, &|n| AsdfImage::float_values(asdf, n), &key.0).map_err(|e| format!("{e:#}"))?;
            let (wcsinfo_sip_max_err_px, wcsinfo_sip_inv_err_px) = wcsinfo_sip_residuals(&asdf.tree);
            let origin = match hdu_index {
                Some(hdu_index) => GwcsOrigin::FitsAsdfHdu { hdu_index },
                None => GwcsOrigin::AsdfTree { key: key.0 },
            };
            Ok(Some(Arc::new(GwcsSource {
                pipeline,
                origin,
                wcsinfo_sip_max_err_px,
                wcsinfo_sip_inv_err_px,
            })))
        }
    }
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    pub(crate) fn tan_cards() -> Vec<(&'static str, String)> {
        vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", "2.5".into()),
            ("CRPIX2", "2.5".into()),
            ("CRVAL1", "180.0".into()),
            ("CRVAL2", "45.0".into()),
            ("CDELT1", "-0.001".into()),
            ("CDELT2", "0.001".into()),
        ]
    }

    pub(crate) fn split_yaml(bytes: &[u8]) -> (String, Vec<u8>) {
        let end = bytes.windows(5).position(|w| w == b"\n...\n").expect("YAML document end");
        (String::from_utf8(bytes[..end + 1].to_vec()).unwrap(), bytes[end + 1..].to_vec())
    }

    pub(crate) fn join_yaml(text: &str, tail: &[u8]) -> Vec<u8> {
        let mut out = text.as_bytes().to_vec();
        out.extend_from_slice(tail);
        out
    }

    pub(crate) fn insert_before_top_level_wcs(bytes: &[u8], inserted: &str) -> Vec<u8> {
        let (text, tail) = split_yaml(bytes);
        let at = text.find("\nwcs: ").expect("top-level wcs key") + 1;
        join_yaml(&format!("{}{}{}", &text[..at], inserted, &text[at..]), &tail)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use serde_yaml::Value;

    use super::test_fixtures::{insert_before_top_level_wcs, join_yaml, split_yaml, tan_cards};
    use super::*;
    use crate::core::astrometry::gwcs::test_support::{
        assert_close_slice, documents_fits_dir, fixtures_dir, heavy_test_dir, mast_dir, real_data_dir, real_syntax, skip_if_absent,
    };
    use crate::core::astrometry::gwcs::{parse_gwcs, wcs_node_at, GwcsOrigin, Kind, Node};
    use crate::core::astrometry::wcs::WcsKind;
    use crate::infra::asdf::{AsdfFile, AsdfImage};
    use crate::infra::fits::asdf_hdu::find_asdf_hdu;
    use crate::infra::fits::asdf_hdu::test_fixtures::write_fits_with_asdf_cell;
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, plane_hdu, write_raw_hdus};
    use crate::infra::image_source::load_plane_header;
    use crate::types::image_ref::ImageRef;

    fn s(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn fixture_bytes(name: &str) -> Vec<u8> {
        std::fs::read(fixtures_dir().join(name)).unwrap()
    }

    fn header_of(path: &str) -> HduHeader {
        load_plane_header(&ImageRef::parse(path)).unwrap()
    }

    fn write_plain_tan_fits(path: &Path) {
        let (mut sci, data) = plane_hdu("SCI", 4, 4);
        sci.extend(tan_cards());
        write_raw_hdus(path, &[(empty_primary_cards(), Vec::new()), (sci, data)]);
    }

    fn set_mtime(path: &Path, when: SystemTime) {
        File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
    }

    fn overwrite_with_zeros_keeping_the_stamp(path: &str) {
        let meta = std::fs::metadata(path).unwrap();
        let mtime = meta.modified().unwrap();
        std::fs::write(path, vec![0u8; meta.len() as usize]).unwrap();
        set_mtime(Path::new(path), mtime);
    }

    fn outcome(lookup: &GwcsLookup) -> Result<Option<()>, String> {
        lookup.clone().map(|s| s.map(|_| ()))
    }

    fn indent_following_lines(text: &str, spaces: usize) -> String {
        let pad = " ".repeat(spaces);
        let mut lines = text.split_inclusive('\n');
        let mut out = lines.next().unwrap_or("").to_string();
        for line in lines {
            if line.trim().is_empty() {
                out.push_str(line);
            } else {
                out.push_str(&pad);
                out.push_str(line);
            }
        }
        out
    }

    fn renest_wcs_under_roman(bytes: &[u8]) -> Vec<u8> {
        let (text, tail) = split_yaml(bytes);
        let (pre, wcs) = text.split_once("\nwcs: ").expect("top-level wcs key");
        let nested = indent_following_lines(wcs, 2);
        join_yaml(&format!("{pre}\nroman:\n  wcs_l2: {nested}  wcs_l1: {nested}"), &tail)
    }

    const FRAME2D_LINES: &str = "    frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0>
      axes_names: [x, y]
      axes_order: [0, 1]
      axis_physical_types: ['custom:x', 'custom:y']
      name: detector
      unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]
";

    const CELESTIAL_STEP_LINES: &str = "  - !<tag:stsci.edu:gwcs/step-1.3.0>
    frame: !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>
      axes_names: [lon, lat]
      axes_order: [0, 1]
      axis_physical_types: [pos.eq.ra, pos.eq.dec]
      name: world
      reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0>
        frame_attributes: {}
      unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]
    transform: null
";

    fn wrap_transform_fixture_as_wcs(bytes: &[u8]) -> Vec<u8> {
        let (text, tail) = split_yaml(bytes);
        let (pre, transform) = text.split_once("\ntransform: ").expect("top-level transform key");
        let body = format!(
            "{pre}\nwcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>\n  name: ''\n  steps:\n  - !<tag:stsci.edu:gwcs/step-1.3.0>\n{FRAME2D_LINES}    transform: {}{CELESTIAL_STEP_LINES}",
            indent_following_lines(transform, 4)
        );
        join_yaml(&body, &tail)
    }

    fn asdf_doc(body: &str) -> Vec<u8> {
        real_syntax(body).into_bytes()
    }

    const RATE_BODY: &str = "meta:
  exposure: {type: NRS_IFU}
  instrument: {name: NIRSPEC}
";

    const S3D_BODY: &str = "meta:
  wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>
    name: ''
    pixel_shape: null
    steps:
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/frame-1.2.0>
        axes_names: [x, y, z]
        axes_order: [0, 1, 2]
        axes_type: [SPATIAL, SPATIAL, SPECTRAL]
        axis_physical_types: ['custom:SPATIAL', 'custom:SPATIAL', 'custom:SPECTRAL']
        name: detector
        naxes: 3
        unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]
      transform: !transform/identity-1.2.0
        inputs: [x0, x1, x2]
        n_dims: 3
        outputs: [x0, x1, x2]
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/composite_frame-1.0.0>
        frames:
        - !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>
          axes_names: [RA, DEC]
          axes_order: [0, 1]
          axis_physical_types: [pos.eq.ra, pos.eq.dec]
          name: sky
          reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0>
            frame_attributes: {}
          unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]
        - !<tag:stsci.edu:gwcs/spectral_frame-1.2.0>
          axes_names: [wavelength]
          axes_order: [2]
          axis_physical_types: [em.wl]
          name: spectral
          unit: [!unit/unit-1.0.0 um]
        name: world
      transform: null
";

    const S2D_BODY: &str = "meta:
  wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>
    name: ''
    pixel_shape: null
    steps:
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0>
        axes_names: [x, y]
        axes_order: [0, 1]
        axis_physical_types: ['custom:x', 'custom:y']
        name: detector
        unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]
      transform: !transform/identity-1.2.0
        inputs: [x0, x1]
        n_dims: 2
        outputs: [x0, x1]
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/composite_frame-1.1.0>
        frames:
        - !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>
          axes_names: [lon, lat]
          axes_order: [0, 1]
          axis_physical_types: [pos.eq.ra, pos.eq.dec]
          name: sky
          reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0>
            frame_attributes: {}
          unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]
        - !<tag:stsci.edu:gwcs/spectral_frame-1.2.0>
          axes_names: [wavelength]
          axes_order: [2]
          axis_physical_types: [em.wl]
          name: spectral
          unit: [!unit/unit-1.0.0 um]
        name: world
      transform: null
";

    const FITSWCS_IMAGING_BODY: &str = "meta:
  wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>
    name: ''
    pixel_shape: null
    steps:
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0>
        axes_names: [x, y]
        axes_order: [0, 1]
        axis_physical_types: ['custom:x', 'custom:y']
        name: detector
        unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]
      transform: !<tag:stsci.edu:gwcs/fitswcs_imaging-1.0.0>
        bounding_box: !transform/property/bounding_box-1.1.0
          ignore: []
          intervals:
            x: [-0.5, 3.5]
            y: [-0.5, 3.5]
          order: F
        cdelt: !core/ndarray-1.1.0
          data: [-0.001, 0.001]
          datatype: float64
          shape: [2]
        crpix: !core/ndarray-1.1.0
          data: [1.5, 1.5]
          datatype: float64
          shape: [2]
        crval: !core/ndarray-1.1.0
          data: [180.0, 45.0]
          datatype: float64
          shape: [2]
        inputs: [x, y]
        outputs: [lon, lat]
        pc: !core/ndarray-1.1.0
          data: [[1.0, 0.0], [0.0, 1.0]]
          datatype: float64
          shape: [2, 2]
        projection: !transform/gnomonic-1.3.0
          direction: pix2sky
          inputs: [x, y]
          outputs: [phi, theta]
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>
        axes_names: [lon, lat]
        axes_order: [0, 1]
        axis_physical_types: [pos.eq.ra, pos.eq.dec]
        name: world
        reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0>
          frame_attributes: {}
        unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]
      transform: null
";

    fn write_tan_fits_with_cell(dir: &Path, name: &str, cell: &[u8]) -> String {
        let path = dir.join(name);
        write_fits_with_asdf_cell(&path, &tan_cards(), cell);
        s(&path)
    }

    fn non_imaging_files(dir: &Path) -> Vec<String> {
        vec![
            write_tan_fits_with_cell(dir, "rate_shape.fits", &asdf_doc(RATE_BODY)),
            write_tan_fits_with_cell(dir, "s3d_shape.fits", &asdf_doc(S3D_BODY)),
            write_tan_fits_with_cell(dir, "s2d_shape.fits", &asdf_doc(S2D_BODY)),
        ]
    }

    fn tabular_refusal_file(dir: &Path) -> String {
        write_tan_fits_with_cell(dir, "tabular_wcs.fits", &wrap_transform_fixture_as_wcs(&fixture_bytes("tabular1d_with_inverse.asdf")))
    }

    fn same_sky(a: &WcsTransform, b: &WcsTransform, x: f64, y: f64) {
        let (p, q) = (a.pixel_to_world(x, y), b.pixel_to_world(x, y));
        assert_close_slice(&[p.ra, p.dec], &[q.ra, q.dec], 1e-12, &format!("sky at ({x}, {y})"));
    }

    #[test]
    fn gwcs_for_path_reads_a_standalone_asdf_and_caches_by_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wcs.asdf");
        std::fs::write(&path, fixture_bytes("wcs_jwst_miri_cal300.asdf")).unwrap();
        let p = s(&path);
        let cache = GwcsCache::new();

        let first = cache.lookup(&p).unwrap().expect("the MIRI chain is an imaging gWCS");
        assert_eq!(first.origin, GwcsOrigin::AsdfTree { key: "wcs".into() });
        assert_eq!(first.pipeline.n_steps(), 3);
        assert_eq!(first.pipeline.frames_summary(), "detector->v2v3->v2v3vacorr->world");
        let again = cache.lookup(&p).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &again), "a stamp hit returns the cached Arc");

        let before = first.pipeline.forward(0.0, 0.0, false);
        std::fs::write(&path, fixture_bytes("wcs_jwst_fgs_cal300.asdf")).unwrap();
        set_mtime(&path, SystemTime::now() + Duration::from_secs(5));
        let replaced = cache.lookup(&p).unwrap().unwrap();
        assert!(!Arc::ptr_eq(&first, &replaced), "a changed stamp re-reads the file");
        assert_eq!(replaced.pipeline.frames_summary(), first.pipeline.frames_summary());
        let after = replaced.pipeline.forward(0.0, 0.0, false);
        assert!((after[0] - before[0]).abs() > 1e-3 || (after[1] - before[1]).abs() > 1e-3, "{before:?} vs {after:?}");
    }

    #[test]
    fn gwcs_for_path_reads_the_fits_asdf_hdu() {
        let dir = tempfile::tempdir().unwrap();
        let cell = fixture_bytes("wcs_jwst_nircam_cal300.asdf");
        let plain = write_tan_fits_with_cell(dir.path(), "nircam_cal.fits", &cell);
        let cache = GwcsCache::new();

        let src = cache.lookup(&plain).unwrap().expect("the NIRCam chain is an imaging gWCS");
        assert_eq!(src.origin, GwcsOrigin::FitsAsdfHdu { hdu_index: 2 });
        assert_eq!(src.origin.describe(), "ASDF HDU 2");
        assert_eq!(src.pipeline.frames_summary(), "detector->v2v3->v2v3vacorr->world");
        assert_eq!((src.wcsinfo_sip_max_err_px, src.wcsinfo_sip_inv_err_px), (None, None));
        assert_close_slice(&src.pipeline.forward(0.0, 0.0, true), &[274.7253135310639, -13.875903855344635], 1e-10, "nrca1 (0,0)");
        let via_hdu_ref = cache.lookup(&format!("{plain}#hdu=1")).unwrap().unwrap();
        assert!(Arc::ptr_eq(&src, &via_hdu_ref), "the #hdu suffix does not change the source file");

        let with_info = write_tan_fits_with_cell(
            dir.path(),
            "nircam_cal_wcsinfo.fits",
            &insert_before_top_level_wcs(&cell, "meta:\n  wcsinfo: {sipmxerr: 0.0087, sipiverr: 0.0088}\n"),
        );
        let src = gwcs_for_path(&with_info).unwrap().unwrap();
        assert_eq!((src.wcsinfo_sip_max_err_px, src.wcsinfo_sip_inv_err_px), (Some(0.0087), Some(0.0088)));
        assert_close_slice(&src.pipeline.forward(0.0, 0.0, true), &[274.7253135310639, -13.875903855344635], 1e-10, "shifted blocks still resolve");
    }

    #[test]
    fn a_fits_without_an_asdf_hdu_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain_tan.fits");
        write_plain_tan_fits(&path);
        assert!(matches!(gwcs_for_path(&s(&path)), Ok(None)));
    }

    #[test]
    fn synthetic_keys_and_missing_files_have_no_gwcs() {
        let dir = tempfile::tempdir().unwrap();
        let missing_fits = s(&dir.path().join("never_written.fits"));
        let missing_asdf = s(&dir.path().join("never_written.asdf"));
        for key in ["__composite_r", "__wizard_ch_ha_aligned#hdu=1", "__star_mask#array=dq", "", missing_fits.as_str(), missing_asdf.as_str()] {
            let lookup = gwcs_for_path(key);
            assert!(matches!(lookup, Ok(None)), "{key}: {:?}", outcome(&lookup));
        }
    }

    #[test]
    fn an_unsupported_chain_is_an_error_naming_the_tag() {
        let dir = tempfile::tempdir().unwrap();
        let path = tabular_refusal_file(dir.path());
        let refusal = gwcs_for_path(&path).err().expect("an imaging gWCS with a tabular step is refused");
        assert!(refusal.contains("'tabular'"), "{refusal}");
        assert!(refusal.contains("at steps[0].transform"), "{refusal}");
        assert!(refusal.contains("is not supported by the evaluator"), "{refusal}");
    }

    #[test]
    fn load_wcs_falls_back_to_the_header_and_keeps_the_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let refused = tabular_refusal_file(dir.path());
        let header = header_of(&refused);
        let from_loader = load_wcs(&refused, &header).unwrap();
        let from_header = WcsTransform::from_header(&header).unwrap();
        same_sky(&from_loader, &from_header, 1.5, 1.5);
        same_sky(&from_loader, &from_header, 0.0, 3.0);
        assert_eq!(from_loader.kind(), WcsKind::Header);
        let refusal = from_loader.gwcs_refusal().expect("the fallback keeps the refusal text");
        assert!(refusal.contains("'tabular'"), "{refusal}");

        let plain = dir.path().join("plain_tan.fits");
        write_plain_tan_fits(&plain);
        let header = header_of(&s(&plain));
        let wcs = load_wcs(&s(&plain), &header).unwrap();
        same_sky(&wcs, &WcsTransform::from_header(&header).unwrap(), 2.5, 1.0);
        assert_eq!((wcs.kind(), wcs.gwcs_refusal()), (WcsKind::Header, None));
    }

    #[test]
    fn load_wcs_uses_the_gwcs_when_the_file_carries_one() {
        let dir = tempfile::tempdir().unwrap();
        let cell = fixture_bytes("wcs_jwst_nircam_cal300.asdf");
        let path = write_tan_fits_with_cell(dir.path(), "nircam_cal.fits", &cell);
        let header = header_of(&path);
        let wcs = load_wcs(&path, &header).unwrap();
        assert_eq!(wcs.kind(), WcsKind::Gwcs);
        assert_eq!(wcs.gwcs_refusal(), None);
        let info = wcs.gwcs_info().expect("gwcs info");
        assert_eq!((info.n_steps, info.source.as_str()), (3, "ASDF HDU 2"));
        let sky = wcs.pixel_to_world(0.0, 0.0);
        assert_close_slice(&[sky.ra, sky.dec], &[274.7253135310639, -13.875903855344635], 1e-10, "nrca1 (0,0) through load_wcs");
        assert!(Arc::ptr_eq(
            &gwcs_for_path(&path).unwrap().unwrap(),
            &gwcs_for_path(&format!("{path}#hdu=1")).unwrap().unwrap()
        ));
    }

    #[test]
    fn roman_wcs_sidecar_keys_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r_wcs.asdf");
        std::fs::write(&path, renest_wcs_under_roman(&fixture_bytes("wcs_gwcs_examples_tan.asdf"))).unwrap();
        let src = gwcs_for_path(&s(&path)).unwrap().expect("roman.wcs_l2 is an imaging gWCS");
        assert_eq!(src.origin, GwcsOrigin::AsdfTree { key: "roman.wcs_l2".into() });
        assert_eq!(src.pipeline.source_key, "roman.wcs_l2");
        assert_close_slice(&src.pipeline.forward(0.0, 0.0, false), &[29.980362905902044, 44.986109428818445], 1e-10, "wcs_l2 origin");

        let asdf = AsdfFile::open(&path).unwrap();
        let l1 = wcs_node_at(&asdf.tree, "roman.wcs_l1").expect("roman.wcs_l1 is addressable");
        let pipeline = parse_gwcs(l1, &|n| AsdfImage::float_values(&asdf, n), "roman.wcs_l1").unwrap();
        assert_close_slice(&pipeline.forward(0.0, 0.0, false), &[29.980362905902044, 44.986109428818445], 1e-10, "wcs_l1 origin");
        assert!(wcs_node_at(&asdf.tree, "roman.meta.wcs").is_none());
    }

    #[test]
    fn fitswcs_imaging_trees_are_left_to_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_tan_fits_with_cell(dir.path(), "i2d_shape.fits", &asdf_doc(FITSWCS_IMAGING_BODY));
        let lookup = gwcs_for_path(&path);
        assert!(matches!(lookup, Ok(None)), "{:?}", outcome(&lookup));
        let header = header_of(&path);
        let wcs = load_wcs(&path, &header).unwrap();
        same_sky(&wcs, &WcsTransform::from_header(&header).unwrap(), 1.5, 1.5);
        assert_eq!(wcs.orientation(4, 4).projection, "TAN");
        assert_eq!((wcs.kind(), wcs.gwcs_refusal()), (WcsKind::Header, None));
    }

    #[test]
    fn non_imaging_gwcs_is_not_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        for path in non_imaging_files(dir.path()) {
            let lookup = gwcs_for_path(&path);
            assert!(matches!(lookup, Ok(None)), "{path}: {:?}", outcome(&lookup));
            let header = header_of(&path);
            let wcs = load_wcs(&path, &header).unwrap();
            same_sky(&wcs, &WcsTransform::from_header(&header).unwrap(), 1.5, 1.5);
            assert_eq!((wcs.kind(), wcs.gwcs_refusal()), (WcsKind::Header, None), "{path}");
        }
    }

    #[test]
    fn gwcs_lookup_caches_every_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let mut files = non_imaging_files(dir.path());
        files.push(tabular_refusal_file(dir.path()));
        files.push(write_tan_fits_with_cell(dir.path(), "nircam_cal.fits", &fixture_bytes("wcs_jwst_nircam_cal300.asdf")));
        let cache = GwcsCache::new();
        for path in files {
            let first = cache.lookup(&path);
            overwrite_with_zeros_keeping_the_stamp(&path);
            let second = cache.lookup(&path);
            assert_eq!(outcome(&first), outcome(&second), "{path}: the second call served the cache");
            cache.invalidate(&path);
            let third = cache.lookup(&path);
            assert!(matches!(third, Ok(None)), "{path}: zero bytes carry no gWCS and are not a refusal: {:?}", outcome(&third));
            if outcome(&first) != Ok(None) {
                assert_ne!(outcome(&first), outcome(&third), "{path}: invalidation forced a re-read");
            }
        }
    }

    #[test]
    fn invalidating_the_global_cache_forces_a_re_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = s(&dir.path().join("wcs.asdf"));
        std::fs::write(&path, fixture_bytes("wcs_jwst_miri_cal300.asdf")).unwrap();
        assert!(gwcs_for_path(&path).unwrap().is_some());
        overwrite_with_zeros_keeping_the_stamp(&path);
        invalidate_gwcs_cache(&path);
        let after = gwcs_for_path(&path);
        assert!(matches!(after, Ok(None)), "zero bytes carry no gWCS: {:?}", outcome(&after));
    }

    #[test]
    fn a_damaged_asdf_cell_is_not_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let broken_yaml = write_tan_fits_with_cell(
            dir.path(),
            "broken_yaml_cell.fits",
            b"#ASDF 1.0.0\n#ASDF_STANDARD 1.6.0\n%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\nmeta:\n  wcs: [\n",
        );
        let no_magic = write_tan_fits_with_cell(dir.path(), "zero_cell.fits", &[0u8; 64]);
        let empty_asdf = s(&dir.path().join("empty.asdf"));
        std::fs::write(&empty_asdf, b"").unwrap();
        for path in [&broken_yaml, &no_magic, &empty_asdf] {
            let lookup = gwcs_for_path(path);
            assert!(matches!(lookup, Ok(None)), "{path}: {:?}", outcome(&lookup));
        }
        for path in [&broken_yaml, &no_magic] {
            let header = header_of(path);
            let wcs = load_wcs(path, &header).unwrap();
            same_sky(&wcs, &WcsTransform::from_header(&header).unwrap(), 1.5, 1.5);
            assert_eq!((wcs.kind(), wcs.gwcs_refusal()), (WcsKind::Header, None), "{path}");
        }
    }

    const DEGENERATE_MAPPING_BODY: &str = "meta:
  wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>
    name: ''
    pixel_shape: null
    steps:
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0>
        axes_names: [x, y]
        axes_order: [0, 1]
        axis_physical_types: ['custom:x', 'custom:y']
        name: detector
        unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]
      transform: !transform/remap_axes-1.3.0
        inputs: [x0, x1]
        mapping: [0, 0]
        n_inputs: 2
        outputs: [x0, x1]
    - !<tag:stsci.edu:gwcs/step-1.3.0>
      frame: !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>
        axes_names: [lon, lat]
        axes_order: [0, 1]
        axis_physical_types: [pos.eq.ra, pos.eq.dec]
        name: world
        reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0>
          frame_attributes: {}
        unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]
      transform: null
";

    #[test]
    fn a_gwcs_the_surrogate_cannot_describe_falls_back_to_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_tan_fits_with_cell(dir.path(), "degenerate_mapping.fits", &asdf_doc(DEGENERATE_MAPPING_BODY));
        let source = gwcs_for_path(&path).unwrap().expect("a 2-axis celestial chain is an imaging gWCS");
        assert_close_slice(&source.pipeline.forward(1.5, 1.5, false), &[1.5, 1.5], 1e-12, "both sky axes copy x");
        let header = header_of(&path);
        let wcs = load_wcs(&path, &header).unwrap_or_else(|e| panic!("load_wcs must fall back to the header: {e:#}"));
        assert_eq!(wcs.kind(), WcsKind::Header);
        same_sky(&wcs, &WcsTransform::from_header(&header).unwrap(), 1.5, 1.5);
        let refusal = wcs.gwcs_refusal().expect("the header fallback keeps the from_gwcs error");
        assert!(refusal.contains("Singular"), "{refusal}");
    }

    #[test]
    fn the_cache_evicts_the_least_recently_used_entry_not_everything() {
        let dir = tempfile::tempdir().unwrap();
        let cache = GwcsCache::new();
        let files: Vec<String> = (0..GWCS_CACHE_CAPACITY)
            .map(|i| {
                let path = dir.path().join(format!("plain_{i}.fits"));
                write_plain_tan_fits(&path);
                s(&path)
            })
            .collect();
        for path in &files {
            assert!(matches!(cache.lookup(path), Ok(None)));
        }
        assert!(matches!(cache.lookup(&files[0]), Ok(None)), "touching the oldest entry makes it recent");
        let extra = dir.path().join("plain_extra.fits");
        write_plain_tan_fits(&extra);
        assert!(matches!(cache.lookup(&s(&extra)), Ok(None)));

        let entries = cache.entries();
        assert_eq!(entries.len(), GWCS_CACHE_CAPACITY, "one eviction, not a flush");
        assert!(entries.contains_key(&files[0]), "the recently touched entry stays");
        assert!(!entries.contains_key(&files[1]), "the least recently used entry is the one evicted");
        for path in &files[2..] {
            assert!(entries.contains_key(path), "{path} stays");
        }
        assert!(entries.contains_key(&s(&extra)));
    }

    #[test]
    fn a_private_cache_instance_survives_the_global_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let path = s(&dir.path().join("wcs.asdf"));
        std::fs::write(&path, fixture_bytes("wcs_jwst_miri_cal300.asdf")).unwrap();
        let cache = GwcsCache::new();
        let first = cache.lookup(&path).unwrap().unwrap();
        for i in 0..GWCS_CACHE_CAPACITY {
            let other = dir.path().join(format!("other_{i}.fits"));
            write_plain_tan_fits(&other);
            assert!(matches!(gwcs_for_path(&s(&other)), Ok(None)));
        }
        let again = cache.lookup(&path).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &again), "filling the global cache to capacity does not evict a private instance");
        assert!(!GWCS_CACHE.entries().contains_key(&path), "a private lookup leaves no entry in the global cache");
    }

    fn mast_files(select: impl Fn(&str) -> bool) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(mast_dir()) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(&select))
            .collect();
        files.sort();
        files
    }

    fn count_stored_inverses(node: &Node) -> usize {
        let own = usize::from(node.stored_inverse.is_some());
        own + match &node.kind {
            Kind::Compose(a, b) | Kind::Concatenate(a, b) => count_stored_inverses(a) + count_stored_inverses(b),
            _ => 0,
        }
    }

    #[test]
    #[ignore]
    fn gwcs_real_nrca1_cal_hdu_layout_and_wcsinfo() {
        let path = real_data_dir().join("jw02739001001_02105_00001_nrca1_cal.fits");
        if skip_if_absent(&path) {
            return;
        }
        let hdu = find_asdf_hdu(&File::open(&path).unwrap()).unwrap().expect("ASDF HDU");
        assert_eq!((hdu.index, hdu.data_start, hdu.byte_len), (8, 117_506_880, 65_276));
        let src = gwcs_for_path(&s(&path)).unwrap().expect("imaging gWCS");
        assert_eq!(src.origin, GwcsOrigin::FitsAsdfHdu { hdu_index: 8 });
        assert_eq!(src.origin.describe(), "ASDF HDU 8");
        assert_eq!(src.pipeline.n_steps(), 3);
        assert_eq!(src.pipeline.frames_summary(), "detector->v2v3->v2v3vacorr->world");
        assert_eq!(src.wcsinfo_sip_max_err_px, Some(0.008736956112743353));
        assert_eq!(src.wcsinfo_sip_inv_err_px, Some(0.008801265065873224));
        assert_close_slice(&src.pipeline.forward(0.0, 0.0, true), &[274.7253135310639, -13.875903855344635], 1e-10, "nrca1 (0,0)");
    }

    #[test]
    #[ignore]
    fn gwcs_real_mirimage_cal_has_ten_stored_inverses() {
        let path = real_data_dir().join("jw02739002001_02101_00001_mirimage_cal.fits");
        if skip_if_absent(&path) {
            return;
        }
        let hdu = find_asdf_hdu(&File::open(&path).unwrap()).unwrap().expect("ASDF HDU");
        assert_eq!((hdu.index, hdu.data_start, hdu.byte_len), (8, 29_652_480, 72_067));
        let src = gwcs_for_path(&s(&path)).unwrap().expect("imaging gWCS");
        let stored: usize = src.pipeline.steps.iter().filter_map(|st| st.transform.as_ref()).map(count_stored_inverses).sum();
        assert_eq!(stored, 10);
        assert_close_slice(&src.pipeline.forward(0.0, 0.0, true), &[274.7271082813119, -13.841811218259549], 1e-10, "mirimage (0,0)");
    }

    #[test]
    #[ignore]
    fn gwcs_real_mast_nrcb3_cal_cell_length() {
        let path = mast_dir().join("jw01475001001_02101_00001_nrcb3_cal.fits");
        if skip_if_absent(&path) {
            return;
        }
        let hdu = find_asdf_hdu(&File::open(&path).unwrap()).unwrap().expect("ASDF HDU");
        assert_eq!(hdu.byte_len, 63_081);
        let src = gwcs_for_path(&s(&path)).unwrap().expect("imaging gWCS");
        assert_eq!(src.pipeline.n_steps(), 3);
    }

    #[test]
    #[ignore]
    fn gwcs_real_roman_cal_and_wcs_sidecar() {
        let cal = real_data_dir().join("r9999901001001001001_0001_wfi01_f129_cal.asdf");
        let sidecar = real_data_dir().join("r9999901001001001001_0001_wfi01_f129_wcs.asdf");
        if skip_if_absent(&cal) || skip_if_absent(&sidecar) {
            return;
        }
        let src = gwcs_for_path(&s(&cal)).unwrap().expect("Roman L2 imaging gWCS");
        assert_eq!(src.origin, GwcsOrigin::AsdfTree { key: "roman.meta.wcs".into() });
        assert_eq!(src.pipeline.n_steps(), 3);
        assert_close_slice(&src.pipeline.forward(0.0, 0.0, true), &[72.49379934026463, -30.725719347990214], 1e-10, "roman cal (0,0)");

        let l2 = gwcs_for_path(&s(&sidecar)).unwrap().expect("roman.wcs_l2");
        assert_eq!(l2.origin, GwcsOrigin::AsdfTree { key: "roman.wcs_l2".into() });
        assert_close_slice(&l2.pipeline.forward(0.0, 0.0, true), &[72.49379934026463, -30.725719347990214], 1e-10, "wcs_l2 (0,0)");

        let asdf = AsdfFile::open(&sidecar).unwrap();
        let node = wcs_node_at(&asdf.tree, "roman.wcs_l1").expect("roman.wcs_l1");
        let l1 = parse_gwcs(node, &|n| AsdfImage::float_values(&asdf, n), "roman.wcs_l1").unwrap();
        assert_close_slice(&l1.forward(0.0, 0.0, true), &[72.49394172327753, -30.725838737618616], 1e-10, "wcs_l1 (0,0)");
    }

    #[test]
    #[ignore]
    fn gwcs_real_classification_of_i2d_s3d_rate_and_crf() {
        let i2d = heavy_test_dir().join("jw02739-o001_t001_nircam_clear-f200w_i2d.fits");
        let s3d = documents_fits_dir().join("jw01266005001_02103_00001_nrs1_s3d.fits");
        let rate = documents_fits_dir().join("jw01266005001_02103_00001_nrs1_rate.fits");
        for (what, path) in [("i2d", &i2d), ("s3d", &s3d), ("rate", &rate)] {
            if skip_if_absent(path) {
                continue;
            }
            let lookup = gwcs_for_path(&s(path));
            assert!(matches!(lookup, Ok(None)), "{what}: {:?}", outcome(&lookup));
        }

        let reference_crf = mast_dir().join("jw01475001001_02101_00001_nrca1_o001_crf.fits");
        let reference_cal = mast_dir().join("jw01475001001_02101_00001_nrca1_cal.fits");
        if !skip_if_absent(&reference_crf) && !skip_if_absent(&reference_cal) {
            let crf = gwcs_for_path(&s(&reference_crf)).unwrap().expect("the tweakreg reference exposure keeps the unchanged 3-step cal chain");
            assert!(matches!(crf.origin, GwcsOrigin::FitsAsdfHdu { .. }), "{:?}", crf.origin);
            assert_eq!(crf.pipeline.n_steps(), 3);
            assert_eq!(crf.pipeline.frames_summary(), "detector->v2v3->v2v3vacorr->world");
            let cal = gwcs_for_path(&s(&reference_cal)).unwrap().expect("the cal sibling");
            assert_eq!(
                crf.pipeline.forward(0.0, 0.0, true),
                cal.pipeline.forward(0.0, 0.0, true),
                "the reference exposure is not corrected: its crf and cal chains agree to the last digit"
            );
        }

        let corrected_nrca1 = mast_dir().join("jw01475001001_02101_00002_nrca1_o001_crf.fits");
        if skip_if_absent(&corrected_nrca1) {
            return;
        }
        let corrected = mast_files(|name| name.contains("_02101_00002_") && name.ends_with("_o001_crf.fits"));
        assert!(corrected.contains(&corrected_nrca1), "{corrected:?}");
        for path in &corrected {
            let file = File::open(path).unwrap();
            let hdu = find_asdf_hdu(&file).unwrap().expect("ASDF HDU");
            let asdf = AsdfFile::from_bytes(read_asdf_hdu_bytes(&file, &hdu).unwrap()).unwrap();
            let (_, node) = find_wcs_node(&asdf.tree).expect("meta.wcs");
            let frames: Vec<&str> = node
                .get("steps")
                .and_then(Value::as_sequence)
                .unwrap()
                .iter()
                .map(|step| step.get("frame").and_then(|f| f.get("name")).and_then(Value::as_str).unwrap_or(""))
                .collect();
            assert_eq!(frames, ["detector", "v2v3", "v2v3vacorr", "v2v3corr", "world"], "{}", path.display());
            let refusal = gwcs_for_path(&s(path))
                .err()
                .expect("the tweakreg-corrected chain carries a divide/constant tangent-plane block the evaluator refuses");
            assert!(refusal.contains("'divide'"), "{}: {refusal}", path.display());
            assert!(refusal.contains("at steps[2].transform"), "{}: {refusal}", path.display());
        }
    }
}
