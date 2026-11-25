use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::cmd::common::{blocking_cmd, invalidate_written, load_cached_full, load_companions, plane_info_json};
use crate::core::imaging::layer_roles::{classify_layer, LayerGuess, LayerRole};
use crate::infra::asdf::converter::is_asdf_file;
use crate::infra::cache::{ImageEntry, GLOBAL_IMAGE_CACHE};
use crate::infra::image_source::{
    assignment, assignment_key, declared_bitpix, edit_assignment, inexact_bitpix, load_plane, load_plane_header,
    resolve_plane, resolve_plane_info, sdss_recipe_refusal, user_layer_name, DqAssignment, ErrAssignment,
    USER_ASSIGNMENT_BASIS,
};
use crate::infra::sibling_files::{grid_report, sibling_candidates, SiblingCandidate};
use crate::types::header::HduHeader;
use crate::types::image_ref::{path_key, ImageRef, PlaneSelector};

const NO_MASK_PIXELS: &str = "; every pixel is NaN, the file carries no mask";
const FITS_ONLY: &str = "error and mask layers can be assigned between planes of FITS files only";
const ONE_PLANE_LAYER: &str = "a layer is one plane (give one band as <file>#hdu=<N>&plane=<K>; the band of a cutout cube pairs with the same band by itself)";
const AXES_CARD: &str = "NAXIS";
const TILED_AXES_CARD: &str = "ZNAXIS";

#[derive(Clone, Copy)]
enum Side {
    Err,
    Dq,
}

impl Side {
    fn parse(role: &str) -> Result<Side> {
        match role {
            "err" => Ok(Side::Err),
            "dq" => Ok(Side::Dq),
            other => bail!("role must be \"err\" or \"dq\", got \"{other}\""),
        }
    }

    fn layer(self) -> &'static str {
        match self {
            Side::Err => "error layer",
            Side::Dq => "mask layer",
        }
    }

    fn own(self) -> &'static str {
        match self {
            Side::Err => "error layer",
            Side::Dq => "mask",
        }
    }
}

#[derive(serde::Serialize)]
struct ErrInUse {
    source: String,
    role: LayerRole,
    basis: String,
}

#[derive(serde::Serialize)]
struct DqInUse {
    source: String,
    basis: String,
}

#[derive(serde::Serialize)]
struct ErrCandidate {
    source: String,
    role: LayerRole,
    basis: String,
    usable: bool,
    reason: Option<String>,
}

#[derive(serde::Serialize)]
struct DqCandidate {
    source: String,
    basis: String,
    usable: bool,
    reason: Option<String>,
}

#[derive(serde::Serialize)]
struct Candidates {
    err: Vec<ErrCandidate>,
    dq: Vec<DqCandidate>,
}

#[derive(serde::Serialize)]
struct Suggestions {
    role: Option<LayerGuess>,
    err: Option<ErrInUse>,
    dq: Option<DqInUse>,
    applied: bool,
    candidates: Candidates,
    notes: Vec<String>,
}

fn image_file(path: &str) -> Result<ImageRef> {
    let image = ImageRef::parse(path);
    if image.is_synthetic() {
        bail!("layers can be assigned to an image file only");
    }
    Ok(image)
}

fn is_asdf_plane(r: &ImageRef) -> bool {
    matches!(r.plane, PlaneSelector::Array(_)) || is_asdf_file(Path::new(&r.path))
}

fn plane_of(image: &ImageRef, entry: &ImageEntry) -> Result<PlaneSelector> {
    match entry.plane_info() {
        Some(info) => Ok(info.kind.clone()),
        None => Ok(resolve_plane_info(image)?.kind),
    }
}

fn file_name(path: &str) -> &str {
    Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or("")
}

fn unreadable(source: &str, error: anyhow::Error) -> String {
    format!("cannot read {source}: {error:#}")
}

fn plane_count(header: &HduHeader) -> i64 {
    let axis_prefix = if header.get_i64(TILED_AXES_CARD).is_some() { TILED_AXES_CARD } else { AXES_CARD };
    let axes = header.get_i64(axis_prefix).unwrap_or(0);
    (3..=axes).filter_map(|axis| header.get_i64(&format!("{axis_prefix}{axis}"))).product()
}

fn mask_refusal(name: &str, reason: &str) -> String {
    format!("{name} cannot be used as a mask: {reason}")
}

fn is_image_plane(image: &ImageRef, image_plane: &PlaneSelector, layer: &ImageRef) -> Result<bool> {
    if path_key(&image.path) != path_key(&layer.path) {
        return Ok(false);
    }
    let layer_plane = match &layer.plane {
        PlaneSelector::Auto => resolve_plane_info(layer)?.kind,
        plane => plane.clone(),
    };
    Ok(&layer_plane == image_plane)
}

fn error_assignment(layer: &ImageRef, header: &HduHeader, name: &str, kind: Option<LayerRole>) -> Result<ErrAssignment> {
    let (role, basis, assumed) = match kind {
        Some(role) if role.is_error() => (role, USER_ASSIGNMENT_BASIS.to_string(), false),
        Some(role) => bail!("{} is not an error layer kind", role.name()),
        None => {
            let guess = classify_layer(header, file_name(&layer.path));
            if !guess.role.is_error() {
                bail!("cannot tell whether {name} holds sigma, variance or inverse variance: choose the kind");
            }
            (guess.role, format!("{USER_ASSIGNMENT_BASIS} ({})", guess.basis), guess.assumed)
        }
    };
    Ok(ErrAssignment::Layer { source: layer.clone(), role, basis, assumed })
}

fn assign_mask(key: &str, layer: &ImageRef, header: &HduHeader, name: &str) -> Result<()> {
    let bitpix = declared_bitpix(header).unwrap_or(0);
    if let Some(reason) = inexact_bitpix(bitpix) {
        let no_pixels = bitpix < 0 && load_plane(layer).is_ok_and(|plane| plane.arr.iter().all(|v| !v.is_finite()));
        bail!("{}{}", mask_refusal(name, &reason), if no_pixels { NO_MASK_PIXELS } else { "" });
    }
    let previous = assignment(key).and_then(|assigned| assigned.dq);
    edit_assignment(key, |assigned| assigned.dq = Some(DqAssignment::Layer { source: layer.clone() }));
    let refusal = match load_plane(layer) {
        Ok(plane) if plane.int_plane.is_some() => None,
        Ok(plane) => Some(mask_refusal(name, &plane.info.pairing.int_unavailable.unwrap_or_default())),
        Err(error) => Some(unreadable(&layer.cache_key(), error)),
    };
    if let Some(refusal) = refusal {
        edit_assignment(key, |assigned| assigned.dq = previous);
        bail!(refusal);
    }
    Ok(())
}

fn refreshed(path: &str, image: &ImageRef, mask: Option<&ImageRef>) -> Result<serde_json::Value> {
    invalidate_written(path);
    if let Some(mask) = mask.filter(|mask| path_key(&mask.path) != path_key(&image.path)) {
        GLOBAL_IMAGE_CACHE.remove(&mask.cache_key());
    }
    let entry = load_cached_full(path)?;
    plane_info_json(path, &entry)
}

pub(crate) fn set_companion_json(path: &str, role: &str, source: &str, layer_role: Option<LayerRole>) -> Result<serde_json::Value> {
    let side = Side::parse(role)?;
    let image = image_file(path)?;
    if source.trim().is_empty() {
        bail!("no layer was given");
    }
    let layer = ImageRef::parse(source);
    if is_asdf_plane(&image) || is_asdf_plane(&layer) {
        bail!(FITS_ONLY);
    }
    let entry = load_cached_full(path)?;
    let plane = plane_of(&image, &entry)?;
    if is_image_plane(&image, &plane, &layer)? {
        bail!("an image cannot be its own {}", side.own());
    }
    let key = assignment_key(&image.path, &plane);
    let name = user_layer_name(&layer);
    let layer_header = load_plane_header(&layer).map_err(|error| anyhow!(unreadable(source, error)))?;
    let image_header = entry.header().context("the image has no header")?;
    if let Some(refusal) = grid_report(image_header, &layer_header).refusal(side.layer(), &name) {
        bail!(refusal);
    }
    let planes = plane_count(&layer_header);
    if planes > 1 {
        bail!("{name} holds {planes} planes: {ONE_PLANE_LAYER}");
    }
    match side {
        Side::Err => {
            let assigned = error_assignment(&layer, &layer_header, &name, layer_role)?;
            edit_assignment(&key, |entry| entry.err = Some(assigned));
            refreshed(path, &image, None)
        }
        Side::Dq => {
            assign_mask(&key, &layer, &layer_header, &name)?;
            refreshed(path, &image, Some(&layer))
        }
    }
}

pub(crate) fn clear_companion_json(path: &str, role: &str, disable: Option<bool>) -> Result<serde_json::Value> {
    let side = Side::parse(role)?;
    let image = image_file(path)?;
    if is_asdf_plane(&image) {
        bail!(FITS_ONLY);
    }
    let entry = load_cached_full(path)?;
    let key = assignment_key(&image.path, &plane_of(&image, &entry)?);
    let switched_off = disable == Some(true);
    edit_assignment(&key, |entry| match side {
        Side::Err => entry.err = switched_off.then_some(ErrAssignment::Off),
        Side::Dq => entry.dq = switched_off.then_some(DqAssignment::Off),
    });
    refreshed(path, &image, None)
}

pub(crate) fn set_flag_exclusion_json(path: &str, mask: Option<u32>) -> Result<serde_json::Value> {
    let image = image_file(path)?;
    if load_companions(path)?.dq.is_none() {
        bail!("this image has no mask layer");
    }
    let entry = load_cached_full(path)?;
    let key = assignment_key(&image.path, &plane_of(&image, &entry)?);
    edit_assignment(&key, |entry| entry.exclusion = mask);
    refreshed(path, &image, None)
}

pub(crate) fn set_sdss_frame_error_json(path: &str, gain: f64, dark_variance: f64) -> Result<serde_json::Value> {
    let image = image_file(path)?;
    if !(gain.is_finite() && gain > 0.0) {
        bail!("gain = {gain} e-/DN must be a positive number");
    }
    if !(dark_variance.is_finite() && dark_variance >= 0.0) {
        bail!("dark variance = {dark_variance} DN^2 must be zero or positive");
    }
    let entry = load_cached_full(path)?;
    let info = match entry.plane_info() {
        Some(info) => info.clone(),
        None => resolve_plane_info(&image)?,
    };
    if info.pairing.err_recipe.is_none() {
        let shown = ImageRef { path: image.path, plane: info.kind };
        let reason = sdss_recipe_refusal(&shown).unwrap_or_else(|error| format!("{error:#}"));
        bail!("this file is not an SDSS frame: {reason}");
    }
    let key = assignment_key(&image.path, &info.kind);
    let previous = assignment(&key).and_then(|assigned| assigned.err);
    edit_assignment(&key, |assigned| assigned.err = Some(ErrAssignment::SdssFrame { gain, dark_variance }));
    invalidate_written(path);
    let rebuilt = load_cached_full(path).and_then(|entry| match load_companions(path)?.err_unavailable {
        Some(reason) => bail!(reason),
        None => plane_info_json(path, &entry),
    });
    if rebuilt.is_err() {
        edit_assignment(&key, |assigned| assigned.err = previous);
        invalidate_written(path);
    }
    rebuilt
}

fn candidate_check(candidate: &SiblingCandidate, image: &HduHeader) -> (bool, Option<String>) {
    let layer = ImageRef::auto(&candidate.path);
    let name = user_layer_name(&layer);
    let header = match load_plane_header(&layer) {
        Ok(header) => header,
        Err(error) => return (false, Some(unreadable(&candidate.path, error))),
    };
    let report = grid_report(image, &header);
    let side = if candidate.role == LayerRole::Mask { Side::Dq } else { Side::Err };
    if let Some(refusal) = report.refusal(side.layer(), &name) {
        return (false, Some(refusal));
    }
    let inexact = match side {
        Side::Dq => declared_bitpix(&header).and_then(inexact_bitpix),
        Side::Err => None,
    };
    if let Some(reason) = inexact {
        return (false, Some(mask_refusal(&name, &reason)));
    }
    (true, (!report.warnings.is_empty()).then(|| report.warnings.join("; ")))
}

pub(crate) fn suggest_companions_json(path: &str) -> Result<serde_json::Value> {
    let image = ImageRef::parse(path);
    let (companions, info) = resolve_plane(&image)?;
    let header = load_plane_header(&image)?;
    let siblings = sibling_candidates(Path::new(&image.path));
    let pairing = info.pairing;
    let err_in_use = companions.err.as_ref().zip(pairing.err.as_ref()).map(|(source, choice)| ErrInUse {
        source: source.cache_key(),
        role: choice.role,
        basis: choice.basis.clone(),
    });
    let dq_in_use = companions
        .dq
        .as_ref()
        .zip(pairing.dq.as_ref())
        .map(|(source, choice)| DqInUse { source: source.cache_key(), basis: choice.basis.clone() });
    let applied = (err_in_use.is_some() || siblings.err.is_empty()) && (dq_in_use.is_some() || siblings.dq.is_empty());
    let err = err_in_use.or_else(|| {
        siblings.err.first().map(|c| ErrInUse { source: c.path.clone(), role: c.role, basis: c.basis.clone() })
    });
    let dq = dq_in_use.or_else(|| siblings.dq.first().map(|c| DqInUse { source: c.path.clone(), basis: c.basis.clone() }));
    let candidates = Candidates {
        err: siblings
            .err
            .iter()
            .map(|c| {
                let (usable, reason) = candidate_check(c, &header);
                ErrCandidate { source: c.path.clone(), role: c.role, basis: c.basis.clone(), usable, reason }
            })
            .collect(),
        dq: siblings
            .dq
            .iter()
            .map(|c| {
                let (usable, reason) = candidate_check(c, &header);
                DqCandidate { source: c.path.clone(), basis: c.basis.clone(), usable, reason }
            })
            .collect(),
    };
    Ok(serde_json::to_value(Suggestions { role: pairing.layer, err, dq, applied, candidates, notes: pairing.notes })?)
}

#[tauri::command]
pub async fn set_companion_cmd(
    path: String,
    role: String,
    source: String,
    layer_role: Option<LayerRole>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!(set_companion_json(&path, &role, &source, layer_role))
}

#[tauri::command]
pub async fn clear_companion_cmd(path: String, role: String, disable: Option<bool>) -> Result<serde_json::Value, String> {
    blocking_cmd!(clear_companion_json(&path, &role, disable))
}

#[tauri::command]
pub async fn set_flag_exclusion_cmd(path: String, mask: Option<u32>) -> Result<serde_json::Value, String> {
    blocking_cmd!(set_flag_exclusion_json(&path, mask))
}

#[tauri::command]
pub async fn suggest_companions_cmd(path: String) -> Result<serde_json::Value, String> {
    blocking_cmd!(suggest_companions_json(&path))
}

#[tauri::command]
pub async fn set_sdss_frame_error_cmd(path: String, gain: f64, dark_variance: f64) -> Result<serde_json::Value, String> {
    blocking_cmd!(set_sdss_frame_error_json(&path, gain, dark_variance))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ndarray::Array2;
    use serde_json::{json, Value};

    use super::*;
    use crate::cmd::common::load_companions;
    use crate::cmd::common::test_support::afw_mef;
    use crate::cmd::visualization::get_dq_flag_table_cmd;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::core::imaging::region::test_support::{north_up_cd, wcs_cards};
    use crate::infra::cache::GLOBAL_IMAGE_CACHE;
    use crate::infra::fits::reader::test_fixtures::{ramp_f32, write_raw_hdus, write_test_mef, HduData, TestHdu};
    use crate::infra::fits::writer::write_fits_mono;
    use crate::infra::image_source::{assignment, assignment_key, CompanionAssignment};
    use crate::infra::sdss_frame_file::test_support::{
        assert_sigma_plane, frame_image, write_frame, FRAME_XINTERP, NARROW_XINTERP, SIGMA_GAIN_4_DARK_1,
    };
    use crate::infra::sdss_frame_file::SDSS_NO_ERROR_REASON;
    use crate::infra::sibling_files::GRID_CARDS;
    use crate::types::image_ref::PlaneSelector;

    const UNNAMED_FLAGS_NOTE: &str = "flag layer without bit names: nothing is excluded until bits are chosen";
    const ERROR_SWITCHED_OFF: &str = "error layer switched off by the user";
    const LEGACY_IMAGE: &str = "legacysurvey-0001p000-image-r.fits";
    const LEGACY_INVVAR: &str = "legacysurvey-0001p000-invvar-r.fits";
    const LEGACY_MASKBITS: &str = "legacysurvey-0001p000-maskbits.fits";
    const EUCLID_SCIENCE: &str = "EUC_MER_BGSUB-MOSAIC-VIS_TILE1-AAA_t1_00.00.fits";
    const EUCLID_RMS: &str = "EUC_MER_MOSAIC-VIS-RMS_TILE1-BBB_t2_00.00.fits";
    const EUCLID_FLAG: &str = "EUC_MER_MOSAIC-VIS-FLAG_TILE1-CCC_t3_00.00.fits";

    fn in_folder(dir: &Path, name: &str) -> String {
        dir.join(name).to_str().unwrap().to_string()
    }

    fn sky_cards() -> Vec<(&'static str, String)> {
        let cards = wcs_cards(north_up_cd());
        GRID_CARDS
            .iter()
            .filter_map(|card| cards.iter().find(|(key, _)| key == card).map(|(_, value)| (*card, value.clone())))
            .collect()
    }

    fn plane_cards(side: usize, bitpix: i64, extra: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        let mut cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", bitpix.to_string()),
            ("NAXIS", "2".into()),
            ("NAXIS1", side.to_string()),
            ("NAXIS2", side.to_string()),
        ];
        cards.extend(sky_cards());
        for (key, value) in extra {
            match cards.iter_mut().find(|(known, _)| known == key) {
                Some(card) => card.1 = value.to_string(),
                None => cards.push((key, value.to_string())),
            }
        }
        cards
    }

    fn write_f32(dir: &Path, name: &str, side: usize, values: &[f32], extra: &[(&'static str, &str)]) -> String {
        let path = dir.join(name);
        write_raw_hdus(&path, &[(plane_cards(side, -32, extra), values.iter().flat_map(|v| v.to_be_bytes()).collect())]);
        path.to_str().unwrap().to_string()
    }

    fn write_i32(dir: &Path, name: &str, side: usize, values: &[i32], extra: &[(&'static str, &str)]) -> String {
        let path = dir.join(name);
        write_raw_hdus(&path, &[(plane_cards(side, 32, extra), values.iter().flat_map(|v| v.to_be_bytes()).collect())]);
        path.to_str().unwrap().to_string()
    }

    struct DesSet {
        sci: String,
        wgt: String,
        msk: String,
    }

    fn des_set(dir: &Path) -> DesSet {
        let mut weights = vec![4.0_f32; 16];
        weights[9] = 0.0;
        let mut mask = vec![0i32; 16];
        mask[3] = 1;
        DesSet {
            sci: write_f32(dir, "sci.fits", 4, &ramp_f32(4, 4), &[("EXTNAME", "'SCI     '"), ("DES_EXT", "'IMAGE   '"), ("BUNIT", "'COUNTS/S'")]),
            wgt: write_f32(dir, "wgt.fits", 4, &weights, &[("EXTNAME", "'WGT     '"), ("DES_EXT", "'WEIGHT  '")]),
            msk: write_i32(dir, "msk.fits", 4, &mask, &[("EXTNAME", "'MSK     '"), ("DES_EXT", "'MASK    '")]),
        }
    }

    fn error_of(result: Result<Value>) -> String {
        format!("{:#}", result.expect_err("the call must be refused"))
    }

    fn notes_of(plane: &Value) -> Vec<String> {
        plane["pairing_notes"].as_array().unwrap().iter().map(|note| note.as_str().unwrap().to_string()).collect()
    }

    fn primary_key(path: &str) -> String {
        assignment_key(path, &PlaneSelector::Hdu(0))
    }

    #[test]
    fn a_weight_file_and_a_mask_file_are_assigned_by_hand() {
        let dir = tempfile::tempdir().unwrap();
        let des = des_set(dir.path());
        let plane = set_companion_json(&des.sci, "err", &des.wgt, None).unwrap();
        assert_eq!(plane["err_ref"], json!(des.wgt), "{plane}");
        assert_eq!(
            plane["err_origin"],
            json!({
                "role": "inverse_variance",
                "source": "user assignment (EXTNAME WGT), wgt.fits: 1 / square root, assumed inverse variance",
                "assumed": true,
                "n_no_data": null,
                "origin": "user"
            })
        );
        let loaded = load_companions(&des.sci).unwrap();
        let err = loaded.err.as_ref().expect("the weight file is the error layer");
        assert_eq!(err.arr()[[0, 0]], 0.5);
        assert_eq!(err.arr().iter().filter(|v| v.is_nan()).count(), 1);
        assert_eq!(err.header().and_then(|h| h.get("BUNIT")), Some("COUNTS/S"));
        assert_eq!(loaded.err_origin.as_ref().map(|origin| origin.n_no_data), Some(1));

        let plane = set_companion_json(&des.sci, "dq", &des.msk, None).unwrap();
        assert_eq!(plane["dq_ref"], json!(des.msk), "{plane}");
        assert_eq!(plane["dq_origin"], "user assignment, msk.fits");
        assert_eq!(plane["dq_origin_kind"], "user");
        assert_eq!(plane["err_ref"], json!(des.wgt));
        let loaded = load_companions(&des.sci).unwrap();
        assert_eq!(loaded.exclusion_mask(), u32::MAX);
        assert_eq!(loaded.dq.as_ref().and_then(|(dq, _)| dq.int_plane()).map(|plane| plane.value_at(0, 3)), Some(1));
    }

    #[test]
    fn the_kind_of_an_unnamed_layer_must_be_given() {
        let dir = tempfile::tempdir().unwrap();
        let des = des_set(dir.path());
        let noise = in_folder(dir.path(), "noise.fits");
        write_fits_mono(&noise, &Array2::from_elem((4, 4), 4.0), None).unwrap();
        assert_eq!(
            error_of(set_companion_json(&des.sci, "err", &noise, None)),
            "cannot tell whether noise.fits holds sigma, variance or inverse variance: choose the kind"
        );
        let plane = set_companion_json(&des.sci, "err", &noise, Some(LayerRole::Variance)).unwrap();
        assert_eq!(plane["err_origin"]["source"], "user assignment, noise.fits: square root", "{plane}");
        assert_eq!(load_companions(&des.sci).unwrap().err.expect("the noise file is the error layer").arr()[[0, 0]], 2.0);
        assert_eq!(error_of(set_companion_json(&des.sci, "err", &noise, Some(LayerRole::Mask))), "mask is not an error layer kind");
    }

    #[test]
    fn a_layer_of_another_size_is_refused_with_both_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let des = des_set(dir.path());
        let small = write_f32(dir.path(), "wgt_small.fits", 2, &[4.0; 4], &[("EXTNAME", "'WGT     '")]);
        assert_eq!(
            error_of(set_companion_json(&des.sci, "err", &small, None)),
            "error layer wgt_small.fits is 2x2 and does not match the image 4x4: not used"
        );
        assert_eq!(assignment(&primary_key(&des.sci)), None);
    }

    #[test]
    fn a_float_mask_is_refused_and_an_all_nan_one_is_called_what_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let des = des_set(dir.path());
        let zeros = write_f32(dir.path(), "maskf.fits", 4, &[0.0; 16], &[]);
        assert_eq!(
            error_of(set_companion_json(&des.sci, "dq", &zeros, None)),
            "maskf.fits cannot be used as a mask: floating-point plane (BITPIX -32): flag bits cannot be read exactly"
        );
        let empty = write_f32(dir.path(), "masknan.fits", 4, &[f32::NAN; 16], &[]);
        assert_eq!(
            error_of(set_companion_json(&des.sci, "dq", &empty, None)),
            "masknan.fits cannot be used as a mask: floating-point plane (BITPIX -32): flag bits cannot be read exactly; every pixel is NaN, the file carries no mask"
        );
        assert_eq!(assignment(&primary_key(&des.sci)), None);
    }

    #[test]
    fn a_layer_on_a_shifted_grid_is_applied_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let des = des_set(dir.path());
        let shifted = write_f32(dir.path(), "wgt_shift.fits", 4, &[4.0; 16], &[("EXTNAME", "'WGT     '"), ("CRPIX1", "51.5")]);
        let plane = set_companion_json(&des.sci, "err", &shifted, None).unwrap();
        assert_eq!(plane["err_ref"], json!(shifted), "{plane}");
        assert!(
            notes_of(&plane).contains(&"CRPIX1 differs: image 50.5 against layer 51.5 (user assignment wgt_shift.fits)".to_string()),
            "{plane}"
        );
    }

    #[test]
    fn an_hdu_of_the_same_file_can_be_assigned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noise_mef.fits");
        write_test_mef(
            &path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("NOISE2"), extver: None, cols: 4, rows: 4, data: HduData::F32(vec![0.5; 16]), extra_cards: vec![] },
            ],
        );
        let p = path.to_str().unwrap().to_string();
        let image = format!("{p}#hdu=1");
        let noise = format!("{p}#hdu=2");
        let plane = set_companion_json(&image, "err", &noise, Some(LayerRole::Sigma)).unwrap();
        assert_eq!(plane["err_ref"], json!(noise), "{plane}");
        assert_eq!(plane["err_origin"]["source"], "user assignment, HDU 2: as is");
        assert_eq!(plane["err_origin"]["origin"], "user");
        assert_eq!(load_companions(&image).unwrap().err.expect("HDU 2 is the error layer").arr()[[0, 0]], 0.5);
        assert_eq!(
            error_of(set_companion_json(&image, "err", &image, Some(LayerRole::Sigma))),
            "an image cannot be its own error layer"
        );
        assert_eq!(error_of(set_companion_json(&image, "dq", &p, None)), "an image cannot be its own mask");
    }

    #[test]
    fn clearing_goes_back_to_the_automatic_choice_and_disable_switches_the_layer_off() {
        let dir = tempfile::tempdir().unwrap();
        let image = write_f32(dir.path(), LEGACY_IMAGE, 4, &ramp_f32(4, 4), &[("BUNIT", "'nanomaggy'")]);
        let invvar = write_f32(dir.path(), LEGACY_INVVAR, 4, &[4.0; 16], &[]);
        write_i32(dir.path(), LEGACY_MASKBITS, 4, &[0; 16], &[("MBIT_3", "'SATUR_R '")]);
        let other = write_f32(dir.path(), "other.fits", 4, &[9.0; 16], &[]);
        assert_eq!(set_companion_json(&image, "err", &other, Some(LayerRole::Variance)).unwrap()["err_ref"], json!(other));

        let plane = clear_companion_json(&image, "err", None).unwrap();
        assert_eq!(plane["err_ref"], json!(invvar), "{plane}");
        assert_eq!(plane["err_origin"]["origin"], "sibling");

        let plane = clear_companion_json(&image, "err", Some(true)).unwrap();
        assert_eq!(plane["err_ref"], Value::Null, "{plane}");
        assert_eq!(plane["err_origin"], Value::Null);
        assert_eq!(plane["err_unavailable"], ERROR_SWITCHED_OFF);
        assert!(load_companions(&image).unwrap().err.is_none());

        let plane = clear_companion_json(&image, "err", None).unwrap();
        assert_eq!(plane["err_ref"], json!(invvar), "{plane}");
        assert_eq!(plane["err_unavailable"], Value::Null);
        assert_eq!(assignment(&primary_key(&image)), None);
    }

    #[test]
    fn bad_arguments_are_refused_before_anything_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let absent = in_folder(dir.path(), "absent.fits");
        let des = des_set(dir.path());
        let wrong_role = r#"role must be "err" or "dq", got "wavelength""#;
        assert_eq!(error_of(set_companion_json(&absent, "wavelength", &des.wgt, None)), wrong_role);
        assert_eq!(error_of(clear_companion_json(&absent, "wavelength", None)), wrong_role);
        assert_eq!(error_of(set_companion_json("__composite_r", "err", &des.wgt, None)), "layers can be assigned to an image file only");
        assert_eq!(error_of(clear_companion_json("__composite_r", "dq", None)), "layers can be assigned to an image file only");
        assert_eq!(error_of(set_companion_json(&absent, "err", "", None)), "no layer was given");
        let missing = error_of(set_companion_json(&des.sci, "err", &in_folder(dir.path(), "missing.fits"), None));
        assert!(missing.starts_with("cannot read"), "{missing}");
        assert_eq!(assignment(&primary_key(&des.sci)), None);
    }

    #[test]
    fn an_assignment_survives_a_reload_of_the_image() {
        let dir = tempfile::tempdir().unwrap();
        let des = des_set(dir.path());
        set_companion_json(&des.sci, "err", &des.wgt, None).unwrap();
        GLOBAL_IMAGE_CACHE.invalidate_written(&des.sci);
        let loaded = load_companions(&des.sci).unwrap();
        assert_eq!(loaded.err.expect("the weight is still the error layer").arr()[[0, 0]], 0.5);
        assert_eq!(
            loaded.err_origin.map(|origin| origin.source).as_deref(),
            Some("user assignment (EXTNAME WGT), wgt.fits: 1 / square root, assumed inverse variance")
        );
    }

    #[tokio::test]
    async fn the_flag_exclusion_command_edits_the_set_and_restores_its_default() {
        let dir = tempfile::tempdir().unwrap();
        let afw = afw_mef(dir.path(), "afw.fits", vec![]);
        set_flag_exclusion_json(&afw, Some(2 | 32)).unwrap();
        let table = get_dq_flag_table_cmd(afw.clone()).await.unwrap();
        assert_eq!(
            (&table["exclusion_mask"], &table["exclusion_default"], &table["exclusion_edited"]),
            (&json!(2), &json!(3), &json!(true)),
            "{table}"
        );
        set_flag_exclusion_json(&afw, None).unwrap();
        let table = get_dq_flag_table_cmd(afw.clone()).await.unwrap();
        assert_eq!((&table["exclusion_mask"], &table["exclusion_edited"]), (&json!(3), &json!(false)), "{table}");

        let des = des_set(dir.path());
        assert_eq!(error_of(set_flag_exclusion_json(&des.sci, Some(1))), "this image has no mask layer");

        let unnamed = dir.path().join("unnamed_mask.fits");
        write_test_mef(
            &unnamed,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: None, cols: 4, rows: 4, data: HduData::F32(ramp_f32(4, 4)), extra_cards: vec![] },
                TestHdu { extname: Some("MASK"), extver: None, cols: 4, rows: 4, data: HduData::I32(vec![5; 16]), extra_cards: vec![] },
            ],
        );
        let k = unnamed.to_str().unwrap().to_string();
        let plane = set_flag_exclusion_json(&k, Some(1)).unwrap();
        assert!(!notes_of(&plane).contains(&UNNAMED_FLAGS_NOTE.to_string()), "{plane}");
        assert_eq!(load_companions(&k).unwrap().exclusion_mask(), 1);
        let plane = set_flag_exclusion_json(&k, None).unwrap();
        assert!(notes_of(&plane).contains(&UNNAMED_FLAGS_NOTE.to_string()), "{plane}");
        assert_eq!(load_companions(&k).unwrap().exclusion_mask(), 0);
    }

    #[test]
    fn suggestions_list_what_is_in_use_and_what_could_be() {
        let euclid = tempfile::tempdir().unwrap();
        let image = write_f32(euclid.path(), EUCLID_SCIENCE, 4, &ramp_f32(4, 4), &[]);
        let rms = write_f32(euclid.path(), EUCLID_RMS, 4, &[0.25; 16], &[("CRPIX1", "51.5")]);
        let flag = write_i32(euclid.path(), EUCLID_FLAG, 4, &[0; 16], &[]);
        let suggested = suggest_companions_json(&image).unwrap();
        assert_eq!(suggested["role"]["role"], "science", "{suggested}");
        assert_eq!(suggested["err"], json!({ "source": rms, "role": "sigma", "basis": "file name -RMS" }), "{suggested}");
        assert_eq!(suggested["dq"], json!({ "source": flag, "basis": "file name -FLAG" }));
        assert_eq!(suggested["applied"], false);
        assert_eq!(
            suggested["candidates"]["err"],
            json!([{ "source": rms, "role": "sigma", "basis": "file name -RMS", "usable": true, "reason": "CRPIX1 differs: image 50.5 against layer 51.5" }])
        );
        assert_eq!(suggested["candidates"]["dq"], json!([{ "source": flag, "basis": "file name -FLAG", "usable": true, "reason": null }]));
        assert_eq!(suggested["notes"].as_array().map(Vec::len), Some(3), "{suggested}");

        let legacy = tempfile::tempdir().unwrap();
        let image = write_f32(legacy.path(), LEGACY_IMAGE, 4, &ramp_f32(4, 4), &[]);
        let invvar = write_f32(legacy.path(), LEGACY_INVVAR, 4, &[4.0; 16], &[]);
        let maskbits = write_i32(legacy.path(), LEGACY_MASKBITS, 4, &[0; 16], &[("MBIT_3", "'SATUR_R '")]);
        let suggested = suggest_companions_json(&image).unwrap();
        assert_eq!(suggested["applied"], true, "{suggested}");
        assert_eq!(suggested["err"]["source"], json!(invvar));
        assert_eq!(suggested["err"]["role"], "inverse_variance");
        assert_eq!(suggested["dq"]["source"], json!(maskbits));

        let alone = tempfile::tempdir().unwrap();
        let image = write_f32(alone.path(), "image.fits", 4, &ramp_f32(4, 4), &[]);
        let suggested = suggest_companions_json(&image).unwrap();
        assert_eq!((&suggested["err"], &suggested["dq"], &suggested["applied"]), (&Value::Null, &Value::Null, &json!(true)));
        assert_eq!(suggested["candidates"], json!({ "err": [], "dq": [] }));
        assert_eq!(suggested["notes"], json!([]));
    }

    const RECIPE_SOURCE_GAIN_4_DARK_1: &str = "SDSS frame recipe (gain 4 e-/DN, dark variance 1 DN^2, typed by the user): rebuilt from the calibration vector and the sky table";

    fn untyped_recipe() -> Value {
        json!({ "kind": "sdss_frame", "gain": null, "dark_variance": null })
    }

    #[test]
    fn the_sdss_command_rebuilds_the_error_and_returns_the_plane() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        let plane = set_sdss_frame_error_json(&frame, 4.0, 1.0).unwrap();
        assert_eq!(plane["err_ref"], json!(format!("{frame}#array=__sdss_sigma.g4.d1")), "{plane}");
        assert_eq!(
            plane["err_origin"],
            json!({ "role": "sigma", "source": RECIPE_SOURCE_GAIN_4_DARK_1, "assumed": false, "n_no_data": 1, "origin": "recipe" })
        );
        assert_eq!(plane["err_recipe"], json!({ "kind": "sdss_frame", "gain": 4.0, "dark_variance": 1.0 }));
        assert_eq!(plane["err_unavailable"], Value::Null);
        let loaded = load_companions(&frame).unwrap();
        assert_sigma_plane(loaded.err.as_ref().expect("the rebuilt sigma is the error layer").arr(), &SIGMA_GAIN_4_DARK_1);
        assert_eq!(loaded.err_origin.map(|origin| origin.source).as_deref(), Some(RECIPE_SOURCE_GAIN_4_DARK_1));

        let plane = clear_companion_json(&frame, "err", None).unwrap();
        assert_eq!(plane["err_ref"], Value::Null, "{plane}");
        assert_eq!(plane["err_origin"], Value::Null);
        assert_eq!(plane["err_recipe"], untyped_recipe());
        assert_eq!(plane["err_unavailable"], SDSS_NO_ERROR_REASON);
        assert_eq!(assignment(&primary_key(&frame)), None);
    }

    #[test]
    fn bad_noise_values_and_other_files_are_refused_before_anything_is_stored() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(dir.path(), "frame.fits", &FRAME_XINTERP);
        for (gain, dark_variance, refusal) in [
            (0.0, 1.0, "gain = 0 e-/DN must be a positive number"),
            (f64::NAN, 1.0, "gain = NaN e-/DN must be a positive number"),
            (4.0, -1.0, "dark variance = -1 DN^2 must be zero or positive"),
        ] {
            assert_eq!(error_of(set_sdss_frame_error_json(&frame, gain, dark_variance)), refusal);
            assert_eq!(assignment(&primary_key(&frame)), None, "{refusal}");
        }
        let mono = in_folder(dir.path(), "mono.fits");
        write_fits_mono(&mono, &frame_image(), None).unwrap();
        assert_eq!(
            error_of(set_sdss_frame_error_json(&mono, 4.0, 1.0)),
            "this file is not an SDSS frame: the file has no calibration vector and no sky table (an SDSS frame has three HDUs or more, this one has 1)"
        );
        assert_eq!(assignment(&primary_key(&mono)), None);
        assert_eq!(
            error_of(set_sdss_frame_error_json("__composite_r", 4.0, 1.0)),
            "layers can be assigned to an image file only"
        );
        assert_eq!(assignment(&primary_key("__composite_r")), None);
    }

    #[test]
    fn a_frame_whose_sky_table_does_not_fit_is_refused_and_nothing_stays_assigned() {
        let dir = tempfile::tempdir().unwrap();
        let narrow = write_frame(dir.path(), "narrow.fits", &NARROW_XINTERP);
        let key = primary_key(&narrow);
        for previous in [None, Some(ErrAssignment::Off)] {
            edit_assignment(&key, |assigned| assigned.err = previous.clone());
            let refused = error_of(set_sdss_frame_error_json(&narrow, 4.0, 1.0));
            assert!(refused.starts_with("the SDSS frame error could not be rebuilt:"), "{refused}");
            assert!(refused.contains("the sky image is 3 x 3 but the frame is 4 x 3"), "{refused}");
            assert_eq!(assignment(&key).unwrap_or_default(), CompanionAssignment { err: previous, ..CompanionAssignment::default() });
        }
        let plane = plane_info_json(&narrow, &load_cached_full(&narrow).unwrap()).unwrap();
        assert_eq!(plane["err_unavailable"], ERROR_SWITCHED_OFF, "{plane}");
        assert_eq!(plane["err_recipe"], untyped_recipe());
    }

    fn survey_copy(dir: &Path, set: &str, name: &str) -> String {
        let target = dir.join(name);
        std::fs::copy(env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join(set).join(name), &target).unwrap();
        target.to_str().unwrap().to_string()
    }

    fn within_1e6(measured: f64, expected: f64) -> bool {
        ((measured - expected) / expected).abs() <= 1e-6
    }

    fn finite_median(values: &Array2<f32>) -> f64 {
        let mut sorted: Vec<f64> = values.iter().filter(|v| v.is_finite()).map(|v| f64::from(*v)).collect();
        sorted.sort_by(f64::total_cmp);
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/D1/des-dr2_A_r_extn1.fits, _extn2.fits, _extn3.fits"]
    fn real_des_layers_are_assigned_by_hand_from_their_extname() {
        let dir = tempfile::tempdir().unwrap();
        let image = survey_copy(dir.path(), "D1", "des-dr2_A_r_extn1.fits");
        let mask = survey_copy(dir.path(), "D1", "des-dr2_A_r_extn2.fits");
        let weight = survey_copy(dir.path(), "D1", "des-dr2_A_r_extn3.fits");
        let plane = set_companion_json(&image, "err", &weight, None).unwrap();
        println!("des error assignment: err_origin {}, notes {}", plane["err_origin"], plane["pairing_notes"]);
        assert_eq!(plane["err_origin"]["role"], "inverse_variance");
        assert_eq!(plane["err_origin"]["assumed"], true);
        assert_eq!(
            plane["err_origin"]["source"],
            "user assignment (EXTNAME WGT), des-dr2_A_r_extn3.fits: 1 / square root, assumed inverse variance"
        );
        let plane = set_companion_json(&image, "dq", &mask, None).unwrap();
        println!("des mask assignment: dq_origin {}, dq_origin_kind {}, notes {}", plane["dq_origin"], plane["dq_origin_kind"], plane["pairing_notes"]);

        let loaded = load_companions(&image).unwrap();
        let err = loaded.err.as_ref().expect("the weight cutout is the error layer");
        let origin = loaded.err_origin.as_ref().expect("origin of the error layer");
        let unit = err.header().and_then(|h| h.get("BUNIT").map(str::to_string));
        println!(
            "des error: [0, 0] = {}, [229, 229] = {}, n_no_data {}, BUNIT {unit:?}",
            f64::from(err.arr()[[0, 0]]),
            f64::from(err.arr()[[229, 229]]),
            origin.n_no_data
        );
        assert!(within_1e6(f64::from(err.arr()[[0, 0]]), 2.452611726935556));
        assert!(within_1e6(f64::from(err.arr()[[229, 229]]), 2.1765951301075823));
        assert_eq!(origin.n_no_data, 0);
        assert_eq!(unit.as_deref(), Some("COUNTS/S"));
        let (dq, _) = loaded.dq.as_ref().expect("the mask cutout is the mask layer");
        let bits = dq.int_plane().expect("the mask is read as integers").bits.iter().fold(0u32, |seen, bits| seen | bits);
        let flags = loaded.flags.as_ref().expect("flag set of the mask");
        println!("des mask: bits present {bits}, flag set {:?}, exclusion {}", flags.source, loaded.exclusion_mask());
        assert_eq!(bits, 0);
        assert_eq!(flags.source, "DES coadd mask (non-zero = no good data)");
        assert_eq!(loaded.exclusion_mask(), u32::MAX);
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_r_stack_480px.fits, _wt_480px.fits, _mask_480px.fits"]
    fn real_ps1_weight_is_assigned_as_a_variance_and_the_mask_cutout_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let image = survey_copy(dir.path(), "P1", "ps1_A_r_stack_480px.fits");
        let weight = survey_copy(dir.path(), "P1", "ps1_A_r_wt_480px.fits");
        let mask = survey_copy(dir.path(), "P1", "ps1_A_r_mask_480px.fits");
        let plane = set_companion_json(&image, "err", &weight, None).unwrap();
        println!("ps1 error assignment: err_origin {}, notes {}", plane["err_origin"], plane["pairing_notes"]);
        assert_eq!(plane["err_origin"]["role"], "variance");
        assert_eq!(plane["err_origin"]["assumed"], true);
        assert_eq!(
            plane["err_origin"]["source"],
            "user assignment (PS_COVAR card (PS1 variance layer)), ps1_A_r_wt_480px.fits: square root, assumed variance"
        );
        let loaded = load_companions(&image).unwrap();
        let err = loaded.err.as_ref().expect("the weight cutout is the error layer");
        let median = finite_median(err.arr());
        println!(
            "ps1 error: [0, 0] = {}, [240, 240] = {}, median {median}",
            f64::from(err.arr()[[0, 0]]),
            f64::from(err.arr()[[240, 240]])
        );
        assert!(within_1e6(f64::from(err.arr()[[0, 0]]), 65.84874803052446));
        assert!(within_1e6(f64::from(err.arr()[[240, 240]]), 81.00975900894595));
        assert!(within_1e6(median, 72.20157127466999), "median {median}");

        let refused = error_of(set_companion_json(&image, "dq", &mask, None));
        println!("ps1 mask assignment: {refused}");
        assert_eq!(
            refused,
            "ps1_A_r_mask_480px.fits cannot be used as a mask: floating-point plane (BITPIX -32): flag bits cannot be read exactly; every pixel is NaN, the file carries no mask"
        );
    }

    const SDSS_PIXELS: [[usize; 2]; 5] = [[0, 0], [744, 1024], [1488, 2047], [100, 2000], [7, 3]];
    const SDSS_SIGMA_GAIN_4_76_DARK_1_3225: [f64; 5] =
        [0.03336109225098343, 0.03207692514935458, 0.03345319614361214, 0.03255361640767915, 0.03288596792515526];
    const SDSS_SIGMA_GAIN_5_DARK_2: [f64; 5] =
        [0.0328761318226313, 0.03163039449334741, 0.032959991049456495, 0.03209103313694354, 0.03241681960108913];
    const SDSS_SIGMA_MEDIAN: f64 = 0.03272367192425486;
    const SDSS_SIGMA_MINIMUM: f64 = 0.029885838903500255;
    const SDSS_SIGMA_MAXIMUM: f64 = 0.47967954053300454;
    const SDSS_PIXEL_TOLERANCE: f64 = 5e-6;
    const SDSS_STATISTIC_TOLERANCE: f64 = 1e-5;

    fn within(measured: f64, expected: f64, relative: f64) -> bool {
        ((measured - expected) / expected).abs() <= relative
    }

    fn assert_sdss_pixels(sigma: &Array2<f32>, expected: [f64; 5], noise: &str) {
        for (pixel, expected) in SDSS_PIXELS.into_iter().zip(expected) {
            let measured = f64::from(sigma[pixel]);
            println!("sdss r sigma ({noise}) {pixel:?} = {measured} nanomaggy (oracle {expected})");
            assert!(within(measured, expected, SDSS_PIXEL_TOLERANCE), "{noise} {pixel:?}: {measured} against {expected} nanomaggy");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/S1/frame-r-001462-4-0165.fits.bz2"]
    fn real_sdss_r_frame_error_is_rebuilt_from_typed_noise() {
        for band in ["g", "r", "i"] {
            let shared = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join("S1").join(format!("frame-{band}-001462-4-0165.fits.bz2"));
            let shared = shared.to_str().unwrap();
            let plane = plane_info_json(shared, &load_cached_full(shared).unwrap()).unwrap();
            println!("sdss {band} frame: err_recipe {}, err_unavailable {}", plane["err_recipe"], plane["err_unavailable"]);
            assert_eq!(plane["err_recipe"], untyped_recipe(), "{band}");
        }

        let dir = tempfile::tempdir().unwrap();
        let frame = survey_copy(dir.path(), "S1", "frame-r-001462-4-0165.fits.bz2");
        let plane = plane_info_json(&frame, &load_cached_full(&frame).unwrap()).unwrap();
        println!("sdss r plane before the noise values: err_ref {}, err_recipe {}, err_unavailable {}", plane["err_ref"], plane["err_recipe"], plane["err_unavailable"]);
        assert_eq!(plane["err_ref"], Value::Null);
        assert_eq!(plane["err_recipe"], untyped_recipe());
        assert_eq!(plane["err_unavailable"], SDSS_NO_ERROR_REASON);

        let plane = set_sdss_frame_error_json(&frame, 4.76, 1.3225).unwrap();
        println!("sdss r plane after the noise values: err_origin {}, err_recipe {}, err_unavailable {}", plane["err_origin"], plane["err_recipe"], plane["err_unavailable"]);
        let loaded = load_companions(&frame).unwrap();
        let err = loaded.err.as_ref().expect("the rebuilt sigma is the error layer");
        let origin = loaded.err_origin.as_ref().expect("origin of the rebuilt sigma");
        assert_eq!(err.arr().dim(), (1489, 2048));
        assert_sdss_pixels(err.arr(), SDSS_SIGMA_GAIN_4_76_DARK_1_3225, "gain 4.76, dark variance 1.3225");
        let median = finite_median(err.arr());
        let minimum = f64::from(err.arr().iter().copied().fold(f32::INFINITY, f32::min));
        let maximum = f64::from(err.arr().iter().copied().fold(f32::NEG_INFINITY, f32::max));
        let unit = err.header().and_then(|h| h.get("BUNIT").map(str::to_string));
        println!("sdss r sigma: median {median}, minimum {minimum}, maximum {maximum} nanomaggy, n_no_data {}, BUNIT {unit:?}", origin.n_no_data);
        println!("sdss r sigma source: {}", origin.source);
        assert!(within(median, SDSS_SIGMA_MEDIAN, SDSS_STATISTIC_TOLERANCE), "median {median}");
        assert!(within(minimum, SDSS_SIGMA_MINIMUM, SDSS_STATISTIC_TOLERANCE), "minimum {minimum}");
        assert!(within(maximum, SDSS_SIGMA_MAXIMUM, SDSS_STATISTIC_TOLERANCE), "maximum {maximum}");
        assert_eq!(origin.n_no_data, 0);
        assert_eq!(unit.as_deref(), Some("nanomaggy"));
        assert_eq!(
            origin.source,
            "SDSS frame recipe (gain 4.76 e-/DN, dark variance 1.3225 DN^2, typed by the user): rebuilt from the calibration vector and the sky table"
        );

        set_sdss_frame_error_json(&frame, 5.0, 2.0).unwrap();
        let loaded = load_companions(&frame).unwrap();
        assert_sdss_pixels(loaded.err.as_ref().expect("the second pair of noise values").arr(), SDSS_SIGMA_GAIN_5_DARK_2, "gain 5, dark variance 2");

        let plane = clear_companion_json(&frame, "err", None).unwrap();
        assert_eq!(plane["err_ref"], Value::Null);
        assert_eq!(assignment(&primary_key(&frame)), None);
    }

    #[test]
    fn a_cube_is_refused_as_a_layer_and_one_of_its_bands_is_accepted() {
        use crate::infra::image_source::test_support::{write_band_cutout, CUTOUT_INVVAR, CUTOUT_NAME};
        let dir = tempfile::tempdir().unwrap();
        let cutout = write_band_cutout(dir.path(), CUTOUT_NAME, CUTOUT_INVVAR);
        let sci = in_folder(dir.path(), "sci.fits");
        write_fits_mono(&sci, &Array2::from_shape_fn((3, 4), |(y, x)| (y * 4 + x) as f32), None).unwrap();

        assert_eq!(
            error_of(set_companion_json(&sci, "err", &format!("{cutout}#hdu=1"), Some(LayerRole::InverseVariance))),
            format!(
                "{CUTOUT_NAME} HDU 1 holds 2 planes: a layer is one plane (give one band as <file>#hdu=<N>&plane=<K>; the band of a cutout cube pairs with the same band by itself)"
            )
        );
        assert_eq!(assignment(&primary_key(&sci)), None);

        let plane =
            set_companion_json(&sci, "err", &format!("{cutout}#hdu=1&plane=0"), Some(LayerRole::InverseVariance)).unwrap();
        assert_eq!(plane["err_ref"], json!(format!("{cutout}#hdu=1&plane=0")), "{plane}");
        let loaded = load_companions(&sci).unwrap();
        let err = loaded.err.as_ref().expect("the g plane of the INVVAR cube is the error layer");
        assert!(err.arr().iter().all(|sigma| *sigma == 0.5), "{:?}", err.arr());
        assert_eq!(
            loaded.err_origin.map(|origin| origin.source),
            Some(format!("user assignment, {CUTOUT_NAME} HDU 1 plane 0: 1 / square root"))
        );
        clear_companion_json(&sci, "err", None).unwrap();
        assert_eq!(assignment(&primary_key(&sci)), None);

        let r_band = format!("{cutout}#hdu=0&plane=1");
        let plane =
            set_companion_json(&r_band, "err", &format!("{cutout}#hdu=1&plane=0"), Some(LayerRole::InverseVariance)).unwrap();
        assert_eq!(plane["err_origin"]["source"], "user assignment, HDU 1 plane 0: 1 / square root", "{plane}");
        assert!(assignment(&assignment_key(&r_band, &PlaneSelector::Hdu(0))).is_some());
        clear_companion_json(&r_band, "err", None).unwrap();
        assert_eq!(assignment(&assignment_key(&r_band, &PlaneSelector::Hdu(0))), None);
    }
}
