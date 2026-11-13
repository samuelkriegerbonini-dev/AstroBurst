use std::path::Path;

use crate::core::imaging::layer_roles::LayerRole;
use crate::types::HduHeader;

pub const GRID_CARDS: [&str; 10] =
    ["CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2", "CD1_1", "CD1_2", "CD2_1", "CD2_2", "CDELT1", "CDELT2"];

#[derive(Debug, Clone, PartialEq)]
pub enum SiblingPattern {
    Exact(String),
    Prefix(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SiblingRule {
    pub rule: &'static str,
    pub err: Vec<SiblingPattern>,
    pub err_role: LayerRole,
    pub err_basis: &'static str,
    pub dq: Vec<SiblingPattern>,
    pub dq_basis: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SiblingCandidate {
    pub path: String,
    pub role: LayerRole,
    pub basis: String,
    pub rule: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SiblingCandidates {
    pub err: Vec<SiblingCandidate>,
    pub dq: Vec<SiblingCandidate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridReport {
    pub image: (i64, i64),
    pub layer: (i64, i64),
    pub warnings: Vec<String>,
}

impl GridReport {
    pub fn same_shape(&self) -> bool {
        self.image == self.layer
    }

    pub fn refusal(&self, what: &str, name: &str) -> Option<String> {
        (!self.same_shape()).then(|| {
            format!(
                "{what} {name} is {}x{} and does not match the image {}x{}: not used",
                self.layer.0, self.layer.1, self.image.0, self.image.1
            )
        })
    }
}

const LEGACY_EXTENSIONS: [&str; 3] = [".fits.fz", ".fits", ".fits.gz"];
const LEGACY_PREFIX: &str = "legacysurvey-";
const LEGACY_IMAGE: &str = "-image-";
const EUCLID_SCIENCE_PREFIX: &str = "EUC_MER_BGSUB-MOSAIC-";
const EUCLID_TILE: &str = "_TILE";
const PS1_STACK: &str = ".stk.";
const PS1_SUFFIX: &str = ".unconv.fits";
const SIBLING_SUFFIXES: [&str; 4] = [".fits", ".fits.gz", ".fits.fz", ".fits.bz2"];
const RELATIVE_TOLERANCE: f64 = 1e-9;
const NO_WCS_WARNING: &str =
    "the layer has no celestial WCS cards: same-index alignment is assumed from the equal shape";

fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

fn strip_suffix_ignoring_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let split = text.len().checked_sub(suffix.len())?;
    let tail = text.get(split..)?;
    tail.eq_ignore_ascii_case(suffix).then(|| &text[..split])
}

fn find_ignoring_case(text: &str, needle: &str) -> Option<usize> {
    text.to_ascii_lowercase().find(&needle.to_ascii_lowercase())
}

fn legacy_rule(name: &str) -> Option<SiblingRule> {
    let stem = LEGACY_EXTENSIONS.iter().find_map(|ext| strip_suffix_ignoring_case(name, ext))?;
    let own = &name[stem.len()..];
    let brick_and_band = strip_prefix_ignoring_case(stem, LEGACY_PREFIX)?;
    let at = find_ignoring_case(brick_and_band, LEGACY_IMAGE)?;
    let band = &brick_and_band[at + LEGACY_IMAGE.len()..];
    if at == 0 || band.is_empty() {
        return None;
    }
    let base = &stem[..LEGACY_PREFIX.len() + at];
    let extensions: Vec<&str> = std::iter::once(own)
        .chain(LEGACY_EXTENSIONS.iter().copied().filter(|ext| !ext.eq_ignore_ascii_case(own)))
        .collect();
    let named = |layer: &str| -> Vec<SiblingPattern> {
        extensions.iter().map(|ext| SiblingPattern::Exact(format!("{base}{layer}{ext}"))).collect()
    };
    Some(SiblingRule {
        rule: "Legacy brick",
        err: named(&format!("-invvar-{band}")),
        err_role: LayerRole::InverseVariance,
        err_basis: "file name -invvar-",
        dq: named("-maskbits"),
        dq_basis: "file name -maskbits",
    })
}

fn euclid_rule(name: &str) -> Option<SiblingRule> {
    let rest = strip_prefix_ignoring_case(name, EUCLID_SCIENCE_PREFIX)?;
    let at = find_ignoring_case(rest, EUCLID_TILE)?;
    let instrument = &rest[..at];
    let after = &rest[at + EUCLID_TILE.len()..];
    let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if instrument.is_empty() || digits == 0 || !after[digits..].starts_with('-') {
        return None;
    }
    let tile = &after[..digits];
    Some(SiblingRule {
        rule: "Euclid MER mosaic",
        err: vec![SiblingPattern::Prefix(format!("EUC_MER_MOSAIC-{instrument}-RMS_TILE{tile}-"))],
        err_role: LayerRole::Sigma,
        err_basis: "file name -RMS",
        dq: vec![SiblingPattern::Prefix(format!("EUC_MER_MOSAIC-{instrument}-FLAG_TILE{tile}-"))],
        dq_basis: "file name -FLAG",
    })
}

fn ps1_rule(name: &str) -> Option<SiblingRule> {
    let stem = strip_suffix_ignoring_case(name, PS1_SUFFIX)?;
    find_ignoring_case(name, PS1_STACK)?;
    Some(SiblingRule {
        rule: "PS1 stack",
        err: vec![SiblingPattern::Exact(format!("{stem}.unconv.wt.fits"))],
        err_role: LayerRole::Variance,
        err_basis: "file name .wt",
        dq: vec![SiblingPattern::Exact(format!("{stem}.unconv.mask.fits"))],
        dq_basis: "file name .mask",
    })
}

pub fn sibling_rule(file_name: &str) -> Option<SiblingRule> {
    legacy_rule(file_name).or_else(|| euclid_rule(file_name)).or_else(|| ps1_rule(file_name))
}

fn folder_file_names(folder: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

pub fn sibling_candidates(image_path: &Path) -> SiblingCandidates {
    let Some(rule) = image_path.file_name().and_then(|name| name.to_str()).and_then(sibling_rule) else {
        return SiblingCandidates::default();
    };
    let text = image_path.to_string_lossy();
    let folder_text = text.rfind(['/', '\\']).map_or("", |at| &text[..=at]);
    let folder = image_path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let listed = if rule.err.iter().chain(&rule.dq).any(|pattern| matches!(pattern, SiblingPattern::Prefix(_))) {
        folder_file_names(folder)
    } else {
        Vec::new()
    };
    let matching = |pattern: &SiblingPattern| -> Vec<String> {
        match pattern {
            SiblingPattern::Exact(name) => {
                if Path::new(&format!("{folder_text}{name}")).is_file() {
                    vec![name.clone()]
                } else {
                    Vec::new()
                }
            }
            SiblingPattern::Prefix(start) => listed
                .iter()
                .filter(|name| strip_prefix_ignoring_case(name, start).is_some())
                .filter(|name| SIBLING_SUFFIXES.iter().any(|suffix| strip_suffix_ignoring_case(name, suffix).is_some()))
                .cloned()
                .collect(),
        }
    };
    let found = |patterns: &[SiblingPattern], role: LayerRole, basis: &str| -> Vec<SiblingCandidate> {
        patterns
            .iter()
            .flat_map(|pattern| matching(pattern))
            .map(|name| SiblingCandidate {
                path: format!("{folder_text}{name}"),
                role,
                basis: basis.to_string(),
                rule: rule.rule,
            })
            .collect()
    };
    SiblingCandidates {
        err: found(&rule.err, rule.err_role, rule.err_basis),
        dq: found(&rule.dq, LayerRole::Mask, rule.dq_basis),
    }
}

pub fn plane_shape(header: &HduHeader) -> (i64, i64) {
    let compressed = header.get("ZIMAGE").is_some_and(|value| value.trim().eq_ignore_ascii_case("T"))
        || header.get("ZNAXIS1").is_some();
    let prefix = if compressed { "ZNAXIS" } else { "NAXIS" };
    let axis = |n: u8| header.get_i64(&format!("{prefix}{n}")).unwrap_or(0);
    (axis(1), axis(2))
}

pub fn grid_report(image: &HduHeader, layer: &HduHeader) -> GridReport {
    let mut warnings: Vec<String> = GRID_CARDS
        .iter()
        .filter_map(|card| {
            let (a, b) = (image.get_f64(card)?, layer.get_f64(card)?);
            let scale = 1.0_f64.max(a.abs()).max(b.abs());
            ((a - b).abs() > RELATIVE_TOLERANCE * scale).then(|| format!("{card} differs: image {a} against layer {b}"))
        })
        .collect();
    if image.get("CRVAL1").is_some() && GRID_CARDS.iter().all(|card| layer.get(card).is_none()) {
        warnings.push(NO_WCS_WARNING.to_string());
    }
    GridReport { image: plane_shape(image), layer: plane_shape(layer), warnings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::{header_with_cd, make_header, north_up_cd};

    const LEGACY_IMAGE: &str = "legacysurvey-0532m280-image-r.fits.fz";
    const EUCLID_SCIENCE: &str = "EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits";
    const NO_WCS: &str = "the layer has no celestial WCS cards: same-index alignment is assumed from the equal shape";

    fn exact(names: &[&str]) -> Vec<SiblingPattern> {
        names.iter().map(|name| SiblingPattern::Exact(name.to_string())).collect()
    }

    fn prefix(name: &str) -> Vec<SiblingPattern> {
        vec![SiblingPattern::Prefix(name.to_string())]
    }

    #[test]
    fn legacy_brick_names_give_the_invvar_and_maskbits_files() {
        let rule = sibling_rule(LEGACY_IMAGE).expect("a Legacy brick image has a sibling rule");
        assert_eq!(rule.rule, "Legacy brick");
        assert_eq!(
            rule.err,
            exact(&[
                "legacysurvey-0532m280-invvar-r.fits.fz",
                "legacysurvey-0532m280-invvar-r.fits",
                "legacysurvey-0532m280-invvar-r.fits.gz",
            ])
        );
        assert_eq!(
            rule.dq,
            exact(&[
                "legacysurvey-0532m280-maskbits.fits.fz",
                "legacysurvey-0532m280-maskbits.fits",
                "legacysurvey-0532m280-maskbits.fits.gz",
            ])
        );
        assert_eq!(rule.err_role, LayerRole::InverseVariance);
        assert_eq!((rule.err_basis, rule.dq_basis), ("file name -invvar-", "file name -maskbits"));

        let gzipped = sibling_rule("LegacySurvey-0001p000-IMAGE-g.FITS.GZ").expect("any case");
        assert_eq!(
            gzipped.err,
            exact(&[
                "LegacySurvey-0001p000-invvar-g.FITS.GZ",
                "LegacySurvey-0001p000-invvar-g.fits.fz",
                "LegacySurvey-0001p000-invvar-g.fits",
            ])
        );

        for layer in ["legacysurvey-0532m280-invvar-r.fits.fz", "legacysurvey-0532m280-maskbits.fits.fz"] {
            assert_eq!(sibling_rule(layer), None, "{layer}");
        }
    }

    #[test]
    fn euclid_mosaic_names_give_prefixes_for_the_tile_and_band() {
        let rule = sibling_rule(EUCLID_SCIENCE).expect("a Euclid MER science mosaic has a sibling rule");
        assert_eq!(rule.rule, "Euclid MER mosaic");
        assert_eq!(rule.err, prefix("EUC_MER_MOSAIC-VIS-RMS_TILE102044185-"));
        assert_eq!(rule.dq, prefix("EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-"));
        assert_eq!(rule.err_role, LayerRole::Sigma);
        assert_eq!((rule.err_basis, rule.dq_basis), ("file name -RMS", "file name -FLAG"));

        let nir = sibling_rule("EUC_MER_BGSUB-MOSAIC-NIR-Y_TILE102158277-AAAAAA_x_00.00.fits").expect("NIR-Y");
        assert_eq!(nir.err, prefix("EUC_MER_MOSAIC-NIR-Y-RMS_TILE102158277-"));
        assert_eq!(nir.dq, prefix("EUC_MER_MOSAIC-NIR-Y-FLAG_TILE102158277-"));

        assert_eq!(sibling_rule("EUC_MER_BGSUB-MOSAIC-VIS-3EA2F5_20241021T041200.630008Z_00.00.fits"), None);
        assert_eq!(sibling_rule("EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_x.fits"), None);
    }

    #[test]
    fn ps1_stack_names_give_the_wt_and_mask_files() {
        let rule = sibling_rule("rings.v3.skycell.0647.098.stk.r.unconv.fits").expect("a PS1 stack has a sibling rule");
        assert_eq!(rule.rule, "PS1 stack");
        assert_eq!(rule.err, exact(&["rings.v3.skycell.0647.098.stk.r.unconv.wt.fits"]));
        assert_eq!(rule.err_role, LayerRole::Variance);
        assert_eq!(rule.dq, exact(&["rings.v3.skycell.0647.098.stk.r.unconv.mask.fits"]));
        assert_eq!((rule.err_basis, rule.dq_basis), ("file name .wt", "file name .mask"));

        for other in [
            "ps1_A_r_stack_480px.fits",
            "image.fits",
            "frame-r-001462-4-0165.fits.bz2",
            "",
            "rings.v3.skycell.0647.098.stk.r.unconv.wt.fits",
        ] {
            assert_eq!(sibling_rule(other), None, "{other}");
        }
    }

    #[test]
    fn candidates_are_only_files_that_exist_in_the_same_folder() {
        let dir = tempfile::tempdir().unwrap();
        let rms = "EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00_cut60_A.fits";
        let flag = "EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_20241021T010718.208008Z_00.00_cut60_A.fits.gz";
        for name in [
            EUCLID_SCIENCE,
            rms,
            flag,
            "EUC_MER_MOSAIC-VIS-RMS_TILE102044186-111111_x_00.00.fits",
            "EUC_MER_MOSAIC-NIR-Y-RMS_TILE102044185-222222_x_00.00.fits",
            "EUC_MER_MOSAIC-VIS-RMS_TILE102044185-333333_x_00.00.txt",
            "image.fits",
        ] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        let image = dir.path().join(EUCLID_SCIENCE);
        let folder = dir.path().to_str().unwrap();
        let found = sibling_candidates(&image);
        assert_eq!(
            found.err,
            vec![SiblingCandidate {
                path: dir.path().join(rms).to_str().unwrap().to_string(),
                role: LayerRole::Sigma,
                basis: "file name -RMS".to_string(),
                rule: "Euclid MER mosaic",
            }]
        );
        assert_eq!(
            found.dq,
            vec![SiblingCandidate {
                path: dir.path().join(flag).to_str().unwrap().to_string(),
                role: LayerRole::Mask,
                basis: "file name -FLAG".to_string(),
                rule: "Euclid MER mosaic",
            }]
        );

        let forward = format!("{}/{EUCLID_SCIENCE}", folder.replace('\\', "/"));
        let found = sibling_candidates(Path::new(&forward));
        assert_eq!(found.err[0].path, format!("{}/{rms}", folder.replace('\\', "/")));

        assert_eq!(sibling_candidates(&dir.path().join("image.fits")), SiblingCandidates::default());
        assert_eq!(
            sibling_candidates(&dir.path().join("missing_folder").join(EUCLID_SCIENCE)),
            SiblingCandidates::default()
        );
    }

    fn square_header(side: &str) -> HduHeader {
        let mut header = header_with_cd(north_up_cd());
        header.set("NAXIS1", side.to_string());
        header.set("NAXIS2", side.to_string());
        header
    }

    #[test]
    fn the_grid_report_names_both_shapes_and_every_differing_card() {
        let image = square_header("601");
        let equal = grid_report(&image, &square_header("601"));
        assert!(equal.same_shape());
        assert!(equal.warnings.is_empty(), "{:?}", equal.warnings);
        assert_eq!(equal.refusal("error layer", "x.fits"), None);
        assert_eq!((equal.image, equal.layer), ((601, 601), (601, 601)));

        let mut narrow = square_header("601");
        narrow.set("NAXIS1", "600".to_string());
        let report = grid_report(&image, &narrow);
        assert!(!report.same_shape());
        assert_eq!(
            report.refusal("error layer", "x.fits"),
            Some("error layer x.fits is 600x601 and does not match the image 601x601: not used".to_string())
        );

        let mut shifted_image = square_header("601");
        shifted_image.set("CRPIX1", "4844".to_string());
        let mut shifted_layer = square_header("601");
        shifted_layer.set("CRPIX1", "4845".to_string());
        assert_eq!(
            grid_report(&shifted_image, &shifted_layer).warnings,
            vec!["CRPIX1 differs: image 4844 against layer 4845".to_string()]
        );

        let bare = make_header(&[("NAXIS1", "601"), ("NAXIS2", "601")]);
        let report = grid_report(&image, &bare);
        assert!(report.same_shape());
        assert_eq!(report.warnings, vec![NO_WCS.to_string()]);
        assert!(grid_report(&bare, &bare).warnings.is_empty());

        let mut nearly = square_header("601");
        nearly.set("CRVAL1", format!("{}", 150.0 * (1.0 + 1e-12)));
        assert!(grid_report(&image, &nearly).warnings.is_empty());

        let compressed =
            make_header(&[("ZIMAGE", "T"), ("NAXIS1", "8"), ("NAXIS2", "1296"), ("ZNAXIS1", "3600"), ("ZNAXIS2", "3600")]);
        assert_eq!(plane_shape(&compressed), (3600, 3600));
        assert_eq!(plane_shape(&make_header(&[("NAXIS1", "8")])), (8, 0));
    }
}
