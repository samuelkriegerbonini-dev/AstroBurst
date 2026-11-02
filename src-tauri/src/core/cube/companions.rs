use std::fs::File;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::cube::lazy::{CubeGeometry, LazyCube};
use crate::core::metadata::units::{classify_unit, UnitClass};
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DqPolicy {
    #[default]
    Ignore,
    DoNotUse,
}

impl DqPolicy {
    pub fn name(self) -> &'static str {
        match self { Self::Ignore => "ignore", Self::DoNotUse => "do_not_use" }
    }

    pub fn excludes(self, bits: Option<u32>) -> bool {
        self == Self::DoNotUse && bits.is_none_or(|bits| bits & 1 != 0)
    }
}

pub const FLOAT_DQ_NOTE: &str = "Legacy floating-point DQ: nonfinite, negative, fractional and values at or above 2^24 are excluded because exact flag bits cannot be established; integer DQ is decoded without float conversion";

pub fn conservative_float_dq(value: f64) -> Option<u32> {
    (value.is_finite() && value >= 0.0 && value < 16_777_216.0 && value.fract() == 0.0).then_some(value as u32)
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
    fresh: bool,
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
    let path = format!("{}#hdu={}", base, found.index);
    match if fresh { LazyCube::open(&path).map(Arc::new) } else { GLOBAL_CUBE_CACHE.get_or_open(&path) } {
        Ok(cube) => Some(cube),
        Err(reason) => {
            notes.push(format!("{} extension (HDU {}) could not be read: {:#}; {}", name, found.index, reason, unreadable));
            None
        }
    }
}

pub fn open_cube_set(path: &str) -> Result<CubeSet> {
    open_cube_set_using(path, false)
}

pub fn open_cube_set_fresh(path: &str) -> Result<CubeSet> {
    open_cube_set_using(path, true)
}

fn open_cube_set_using(path: &str, fresh: bool) -> Result<CubeSet> {
    let sci = if fresh { Arc::new(LazyCube::open(path)?) } else { GLOBAL_CUBE_CACHE.get_or_open(path)? };
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
    let err = open_companion(&extensions, EXTNAME_ERR, ERR_UNREADABLE, &sci, &base, &mut notes, fresh);
    let dq = open_companion(&extensions, EXTNAME_DQ, DQ_UNREADABLE, &sci, &base, &mut notes, fresh);
    if err.as_ref().is_some_and(|cube| card_text(&cube.header, "BUNIT").is_none()) {
        notes.push(format!("ERR has no BUNIT: when ERR is used, the SCI unit {} is assumed", card_text(&sci.header, "BUNIT").as_deref().unwrap_or("(unspecified)")));
    }
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
        if name == EXTNAME_ERR {
            let science = classify_unit(&sci.header);
            let error = classify_unit(&c.header);
            if let Some(error_unit) = &error.bunit {
                let compact = |unit: &str| unit.chars().filter(|character| !character.is_whitespace()).collect::<String>();
                let same_text = science.bunit.as_deref().is_some_and(|unit| compact(unit) == compact(error_unit));
                let same_physical_scale = matches!(science.class, UnitClass::SurfaceBrightness { .. } | UnitClass::PerPixelPhysical { .. }) && science.class == error.class;
                if !same_text && !same_physical_scale {
                    bail!("ERR BUNIT {error_unit} differs from SCI BUNIT {}; convert or correct the input units before using ERR", science.bunit.as_deref().unwrap_or("(missing)"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::core::cube::lazy::test_support::*;

    fn unit_fixture(path: &std::path::Path, sci_unit: Option<&'static str>, err_unit: Option<&'static str>) -> CubeSet {
        let mut sci_cards = line_cube_cards();
        sci_cards.retain(|(key, _)| *key != "BUNIT");
        sci_cards.push(("EXTNAME", "'SCI'"));
        if let Some(unit) = sci_unit { sci_cards.push(("BUNIT", unit)); }
        let mut err_cards = vec![("EXTNAME", "'ERR'")];
        if let Some(unit) = err_unit { err_cards.push(("BUNIT", unit)); }
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        bytes.extend(image_hdu("XTENSION", &[2, 2, 40], -32, &sci_cards, f32_samples(2, 2, 40, |z, _, _| LINE_CONTINUUM + line_profile(z))));
        bytes.extend(image_hdu("XTENSION", &[2, 2, 40], -32, &err_cards, f32_samples(2, 2, 40, |_, _, _| 0.5)));
        write_bytes(path, &bytes);
        open_cube_set_fresh(path.to_str().unwrap()).unwrap()
    }

    #[test]
    fn p0_err_unit_mismatch_is_rejected_by_spectral_operations() {
        use crate::core::cube::lazy::CollapseMode;
        use crate::core::cube::linefit::{fit_cube_lines, fit_spaxel, test_support::line_cfg};
        use crate::core::cube::moments::{moment_maps_set, MomentConfig};

        let dir = tempfile::tempdir().unwrap();
        let set = unit_fixture(&dir.path().join("mismatch.fits"), Some("'Jy/beam'"), Some("'mJy/beam'"));
        let cfg = line_cfg();
        let collapse = set.sci.collapse_range_with_err(8, 32, CollapseMode::Sum, set.err.as_deref());
        assert!(collapse.is_err(), "ERR in mJy/beam must not be interpreted as Jy/beam by Collapse");
        let moments: MomentConfig = serde_json::from_value(serde_json::json!({ "z0": 8, "z1": 32, "rest_um": LINE_REST_UM, "snr_threshold": 0 })).unwrap();
        assert!(moment_maps_set(&set, &moments).unwrap_err().to_string().contains("ERR BUNIT"));
        assert!(fit_cube_lines(&set.inputs(), &cfg).unwrap_err().to_string().contains("ERR BUNIT"));
        assert!(fit_spaxel(&set.inputs(), &cfg, 0, 0).unwrap_err().to_string().contains("ERR BUNIT"));
    }

    #[test]
    fn p0_err_unit_metadata_policy_preserves_matching_and_safe_physical_aliases() {
        let dir = tempfile::tempdir().unwrap();
        for (index, (sci_unit, err_unit)) in [
            (Some("'Jy/beam'"), Some("'Jy/beam'")),
            (Some("'Jy/beam'"), Some("'Jy / beam'")),
            (Some("'Jy'"), Some("'Jy/pixel'")),
            (Some("'mJy'"), Some("'0.001*Jy/pix'")),
            (Some("'MJy/sr'"), Some("'MJy sr-1'")),
            (Some("'custom'"), Some("'custom'")),
            (Some("'Jy/beam'"), None),
            (None, None),
        ].into_iter().enumerate() {
            let set = unit_fixture(&dir.path().join(format!("same-{index}.fits")), sci_unit, err_unit);
            check_companion("ERR", &set.sci, set.err.as_deref()).unwrap();
        }
    }

    #[test]
    fn p0_err_unit_scale_prefixes_and_unknown_units_are_not_equated() {
        let dir = tempfile::tempdir().unwrap();
        for (index, (sci_unit, err_unit)) in [
            (Some("'Jy'"), Some("'mJy'")),
            (Some("'MJy/sr'"), Some("'mJy/sr'")),
            (Some("'custom-a'"), Some("'custom-b'")),
            (None, Some("'Jy'")),
        ].into_iter().enumerate() {
            let set = unit_fixture(&dir.path().join(format!("different-{index}.fits")), sci_unit, err_unit);
            assert!(check_companion("ERR", &set.sci, set.err.as_deref()).is_err(), "{sci_unit:?} != {err_unit:?}");
            check_companion("DQ", &set.sci, set.err.as_deref()).unwrap();
        }
    }

    #[test]
    fn p0_err_unit_absence_is_recorded_and_disabled_errors_do_not_block_fit() {
        use crate::core::cube::linefit::{fit_cube_lines, fit_spaxel, test_support::line_cfg};

        let dir = tempfile::tempdir().unwrap();
        let missing = unit_fixture(&dir.path().join("missing.fits"), Some("'Jy/beam'"), None);
        assert!(missing.notes.iter().any(|note| note.contains("ERR has no BUNIT") && note.contains("Jy/beam")), "{:?}", missing.notes);
        let mismatch = unit_fixture(&dir.path().join("disabled.fits"), Some("'Jy/beam'"), Some("'mJy/beam'"));
        let cfg = crate::core::cube::linefit::LineFitConfig { use_err: false, ..line_cfg() };
        fit_cube_lines(&mismatch.inputs(), &cfg).unwrap();
        let result = fit_spaxel(&mismatch.inputs(), &cfg, 0, 0).unwrap();
        assert!(result.err.is_none());
    }

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
        assert_eq!(open_cube_set(paired_key).unwrap().notes, ["ERR has no BUNIT: when ERR is used, the SCI unit (unspecified) is assumed"]);
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
