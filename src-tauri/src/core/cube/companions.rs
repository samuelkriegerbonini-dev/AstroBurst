use std::fs::File;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::cube::lazy::{CubeGeometry, LazyCube};
use crate::infra::fits::reader::{list_extensions, HduInfo};
use crate::types::constants::{EXTNAME_DQ, EXTNAME_ERR};
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

pub struct CubeSet {
    pub sci: Arc<LazyCube>,
    pub err: Option<Arc<LazyCube>>,
    pub dq: Option<Arc<LazyCube>>,
    pub notes: Vec<String>,
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

const ERR_UNREADABLE: &str = "products have no error column";
const DQ_UNREADABLE: &str = "no spaxel or channel is excluded by its flags";

fn is_named(h: &HduInfo, name: &str) -> bool {
    h.extname.as_deref().is_some_and(|e| e.trim().trim_matches('\'').trim().eq_ignore_ascii_case(name))
}

fn matches_sci(h: &HduInfo, sci: &LazyCube) -> bool {
    let g = &sci.geometry;
    h.naxis == 3 && (h.naxis1, h.naxis2, h.naxis3, 1) == (g.naxis1 as i64, g.naxis2 as i64, g.naxis3 as i64, g.naxis4 as i64)
}

fn hdu_shape_text(h: &HduInfo) -> String {
    let axes = [h.naxis1, h.naxis2, h.naxis3, h.naxis4];
    let shown = usize::try_from(h.naxis).unwrap_or(0).min(axes.len());
    if shown == 0 {
        return "empty".to_string();
    }
    axes[..shown].iter().map(|n| n.to_string()).collect::<Vec<_>>().join("x")
}

fn open_companion(
    extensions: &[HduInfo],
    name: &str,
    unreadable: &str,
    sci: &LazyCube,
    base: &str,
    notes: &mut Vec<String>,
) -> Option<Arc<LazyCube>> {
    let named: Vec<&HduInfo> = extensions.iter().filter(|h| h.index != sci.hdu_index && is_named(h, name)).collect();
    let Some(found) = named.iter().find(|h| matches_sci(h, sci)) else {
        notes.extend(named.iter().map(|h| {
            format!(
                "{} extension HDU {} is {} and does not match the SCI cube {}: not used",
                name,
                h.index,
                hdu_shape_text(h),
                shape_text(&sci.geometry)
            )
        }));
        return None;
    };
    match GLOBAL_CUBE_CACHE.get_or_open(&format!("{}#hdu={}", base, found.index)) {
        Ok(cube) => Some(cube),
        Err(reason) => {
            notes.push(format!("{} extension (HDU {}) could not be read: {:#}; {}", name, found.index, reason, unreadable));
            None
        }
    }
}

pub fn open_cube_set(path: &str) -> Result<CubeSet> {
    let sci = GLOBAL_CUBE_CACHE.get_or_open(path)?;
    if let Some(name) = card_text(&sci.header, "EXTNAME").map(|s| s.to_uppercase()) {
        if name == EXTNAME_ERR || name == EXTNAME_DQ {
            bail!(
                "{} opens the {} extension (HDU {}): cube products need the SCI cube",
                path,
                name,
                sci.hdu_index
            );
        }
    }
    let base = ImageRef::parse(path).path;
    let opened = File::open(&sci.source_path)
        .with_context(|| format!("cannot reopen {} to look for its ERR and DQ companions", path))?;
    let extensions = list_extensions(&opened)
        .with_context(|| format!("cannot list the extensions of {} to look for its ERR and DQ companions", path))?;
    let mut notes = Vec::new();
    let err = open_companion(&extensions, EXTNAME_ERR, ERR_UNREADABLE, &sci, &base, &mut notes);
    let dq = open_companion(&extensions, EXTNAME_DQ, DQ_UNREADABLE, &sci, &base, &mut notes);
    Ok(CubeSet { sci, err, dq, notes })
}

fn shape_text(g: &CubeGeometry) -> String {
    if g.naxis4 > 1 {
        format!("{}x{}x{}x{}", g.naxis1, g.naxis2, g.naxis3, g.naxis4)
    } else {
        format!("{}x{}x{}", g.naxis1, g.naxis2, g.naxis3)
    }
}

pub(crate) fn check_companion(name: &str, sci: &LazyCube, cube: Option<&LazyCube>) -> Result<()> {
    if let Some(c) = cube {
        let (a, b) = (&sci.geometry, &c.geometry);
        if (a.naxis1, a.naxis2, a.naxis3, a.naxis4) != (b.naxis1, b.naxis2, b.naxis3, b.naxis4) {
            bail!("{} cube {} does not match the SCI cube {}", name, shape_text(b), shape_text(a));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::core::cube::lazy::test_support::*;

    #[test]
    fn the_companions_of_a_cube_inside_a_zip_are_found_in_the_extracted_file() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner_s3d.fits");
        let flagged = |_: usize, y: usize, x: usize| i32::from((y, x) == (1, 2));
        write_sci_err_dq_mef(&inner, 4, 3, 6, |z, _, _| 10.0 + z as f32, |z, _, _| 0.5 + z as f32, flagged);
        let zip_path = dir.path().join("cube.zip");
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        writer.start_file("inner_s3d.fits", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&std::fs::read(&inner).unwrap()).unwrap();
        writer.finish().unwrap();
        let key = zip_path.to_str().unwrap();
        let set = open_cube_set(key).unwrap();
        assert_eq!(set.sci.hdu_index, 1);
        assert_eq!(set.err.as_ref().map(|c| c.hdu_index), Some(2));
        assert_eq!(set.dq.as_ref().map(|c| c.hdu_index), Some(3));
        assert_eq!(set.err.as_ref().unwrap().extract_spectrum_at(0, 0).unwrap()[4], 4.5);
        assert_eq!(set.dq.as_ref().unwrap().extract_spectrum_at(1, 2).unwrap()[0], 1.0);
        GLOBAL_CUBE_CACHE.invalidate(key);
    }

    #[test]
    fn a_three_dimensional_companion_is_not_paired_with_a_four_dimensional_sci_cube() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jw_ramp_with_err.fits");
        let primary = [("NGROUPS", "3"), ("NINTS", "2"), ("TGROUP", "10.0"), ("DATAMODL", "'Level1bModel'")];
        write_u16_ramp_mef(&path, 4, 3, 3, 2, &primary, &[], |i, g, y, x| (30000 + i * 1000 + g * 100 + y * 10 + x) as u16);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend(image_hdu("XTENSION", &[4, 3, 3], -32, &[("EXTNAME", "'ERR'")], f32_samples(4, 3, 3, |_, _, _| 1.0)));
        write_bytes(&path, &bytes);
        let key = path.to_str().unwrap();
        let set = open_cube_set(key).unwrap();
        assert_eq!((set.sci.hdu_index, set.sci.geometry.depth), (1, 6));
        assert!(set.err.is_none(), "a 4x3x3 ERR must not pair with a 4x3x3x2 SCI");
        assert_eq!(set.notes, vec!["ERR extension HDU 2 is 4x3x3 and does not match the SCI cube 4x3x3x2: not used".to_string()]);
        GLOBAL_CUBE_CACHE.invalidate(key);
    }

    #[test]
    fn named_companions_of_another_shape_are_reported_with_both_shapes_and_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stale_err_s3d.fits");
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        bytes.extend(image_hdu("XTENSION", &[4, 3, 6], -32, &[("EXTNAME", "'SCI'")], f32_samples(4, 3, 6, |z, _, _| z as f32)));
        bytes.extend(image_hdu("XTENSION", &[4, 3], -32, &[("EXTNAME", "'ERR'")], f32_samples(4, 3, 1, |_, _, _| 0.5)));
        bytes.extend(image_hdu("XTENSION", &[4, 3, 5], -32, &[("EXTNAME", "'DQ'")], f32_samples(4, 3, 5, |_, _, _| 0.0)));
        write_bytes(&path, &bytes);
        let key = path.to_str().unwrap();
        let set = open_cube_set(key).unwrap();
        assert!(set.err.is_none() && set.dq.is_none());
        assert_eq!(
            set.notes,
            vec![
                "ERR extension HDU 2 is 4x3 and does not match the SCI cube 4x3x6: not used".to_string(),
                "DQ extension HDU 3 is 4x3x5 and does not match the SCI cube 4x3x6: not used".to_string(),
            ]
        );
        GLOBAL_CUBE_CACHE.invalidate(key);

        let paired = dir.path().join("paired_s3d.fits");
        write_mef_cube(&paired, 4, 3, 6, &[("SCI", 1.0), ("ERR", 0.5), ("DQ", 0.0)]);
        let paired_key = paired.to_str().unwrap();
        assert!(open_cube_set(paired_key).unwrap().notes.is_empty());
        GLOBAL_CUBE_CACHE.invalidate(paired_key);
    }

    #[test]
    fn an_err_extension_cut_short_leaves_the_set_without_err_and_names_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut_s3d.fits");
        write_line_cube_mef(&path, 0.0, |_, _, _| 0.5, |_, _, _| 0);
        let err_data_start = list_extensions(&File::open(&path).unwrap()).unwrap()[2].data_start;
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.truncate(err_data_start + 1000);
        write_bytes(&path, &bytes);
        let key = path.to_str().unwrap();
        let set = open_cube_set(key).unwrap();
        assert_eq!(set.sci.hdu_index, 1);
        assert!(set.err.is_none() && set.dq.is_none());
        assert_eq!(set.notes.len(), 1, "{:?}", set.notes);
        let note = &set.notes[0];
        assert!(note.starts_with("ERR extension (HDU 2) could not be read: "), "{}", note);
        assert!(note.contains("exceeds file size"), "{}", note);
        assert!(note.ends_with("; products have no error column"), "{}", note);
        for hdu in ["", "#hdu=2"] {
            GLOBAL_CUBE_CACHE.invalidate(&format!("{}{}", key, hdu));
        }
    }
}
