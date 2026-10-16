use super::ids::validate_filename;
use super::normalize::split_optical_elements;
use super::types::{ImageGroup, MastImageEntry, MastImageSet, MastMissionId};
use crate::infra::mast::models::ImageListResult;

const IMAGE_EXTENSIONS: [&str; 3] = [".jpg", ".jpeg", ".png"];
const THUMB_SUFFIX: &str = "_thumb";
const COLOUR_SUFFIX: &str = "_color";
const HAP_SCIENCE: &str = "HapScience";
const JWST_COMBINED_LEVEL: &str = "3";
const SUFFIX_ORDER: [&str; 12] = ["cal", "calints", "i2d", "s2d", "s3d", "crf", "crfints", "rate", "rateints", "uncal", "x1d", "x1dints"];
const SUFFIX_FALLBACK: [&str; 5] = ["i2d", "crf", "rate", "rateints", "uncal"];
const HST_GROUPS: [(&str, &str); 6] = [
    ("combined", "Combined"),
    ("calibrated", "Calibrated"),
    ("intermediate", "Intermediate"),
    ("raw", "Raw"),
    ("hap", "HAP aligned"),
    ("other", "Other"),
];
const HST_PREFERENCE: [&str; 8] = ["drc", "drz", "flc", "flt", "c1m", "mos", "ima", "raw"];
const HST_INSTRUMENTS: [&str; 6] = ["wfc3", "acs", "wfpc2", "nicmos", "stis", "cos"];
const HST_DETECTORS: [&str; 8] = ["uvis", "ir", "wfc", "hrc", "sbc", "pc", "nuv", "fuv"];
const REDIRECT_NEEDS_LIST: [&str; 2] = ["_raw_thumb.jpg", "_ima_thumb.jpg"];

pub fn product_stem(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    let Some(extension) = IMAGE_EXTENSIONS.iter().find(|ext| lower.ends_with(*ext)) else {
        return filename.to_owned();
    };
    let without_extension = &filename[..filename.len() - extension.len()];
    let lower = &lower[..without_extension.len()];
    if lower.ends_with(THUMB_SUFFIX) {
        without_extension[..without_extension.len() - THUMB_SUFFIX.len()].to_owned()
    } else {
        without_extension.to_owned()
    }
}

fn clean(text: Option<&str>) -> Option<String> {
    text.map(str::trim).filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("none") && !t.eq_ignore_ascii_case("null")).map(str::to_owned)
}

fn safe_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && validate_filename(name).is_ok()).then(|| name.to_owned())
}

struct Draft {
    stem: String,
    thumbnail: Option<String>,
    preview: Option<String>,
    level: Option<String>,
    viz_uri: Option<String>,
    science_group: Option<String>,
}

fn collect(thumbs: &[ImageListResult], previews: &[ImageListResult]) -> Vec<Draft> {
    let mut drafts: Vec<Draft> = Vec::new();
    let sources = thumbs.iter().map(|item| (item, true)).chain(previews.iter().map(|item| (item, false)));
    for (item, is_thumb) in sources {
        let Some(filename) = safe_name(&item.filename) else { continue };
        let stem = product_stem(&filename);
        let index = match drafts.iter().position(|draft| draft.stem == stem) {
            Some(index) => index,
            None => {
                drafts.push(Draft {
                    stem,
                    thumbnail: None,
                    preview: None,
                    level: clean(item.product_level.as_deref()),
                    viz_uri: clean(item.viz_uri.as_deref()),
                    science_group: clean(item.science_group.as_deref()),
                });
                drafts.len() - 1
            }
        };
        let draft = &mut drafts[index];
        let slot = if is_thumb { &mut draft.thumbnail } else { &mut draft.preview };
        if slot.is_none() {
            *slot = Some(filename);
        }
    }
    drafts
}

fn archive_name(mission: MastMissionId) -> String {
    match mission {
        MastMissionId::Jwst => "JWST".to_owned(),
        MastMissionId::Hst => "HST".to_owned(),
        other => other.slug().to_ascii_uppercase(),
    }
}

fn last_segment(uri: &str) -> Option<&str> {
    uri.rsplit('/').next().map(str::trim).filter(|s| !s.is_empty())
}

pub fn mast_uri(mission: MastMissionId, viz_uri: Option<&str>, product_filename: &str) -> String {
    if let Some(rest) = viz_uri.and_then(|uri| uri.trim().strip_prefix("mast:")) {
        if let (Some((archive, _)), Some(file)) = (rest.split_once('/'), last_segment(rest)) {
            if !archive.is_empty() {
                return format!("mast:{}/product/{file}", archive.to_ascii_uppercase());
            }
        }
    }
    format!("mast:{}/product/{product_filename}", archive_name(mission))
}

struct Parsed {
    label: String,
    group: String,
    suffix: String,
    detector: Option<String>,
    filter: Option<String>,
    colour: bool,
}

fn tail_after_dataset<'a>(stem: &'a str, dataset: &str) -> Option<&'a str> {
    let prefix_len = dataset.len() + 1;
    let matches = stem.len() > prefix_len
        && stem.is_char_boundary(dataset.len())
        && stem[..dataset.len()].eq_ignore_ascii_case(dataset)
        && stem.as_bytes()[dataset.len()] == b'_';
    matches.then(|| &stem[prefix_len..])
}

fn last_token(stem: &str) -> String {
    stem.rsplit('_').next().unwrap_or(stem).to_ascii_lowercase()
}

fn parse_generic(stem: &str, dataset: &str, level: Option<&str>) -> Parsed {
    let suffix = last_token(stem);
    let tokens: Vec<&str> = tail_after_dataset(stem, dataset).map(|tail| tail.split('_').collect()).unwrap_or_default();
    let first = tokens.first().filter(|_| tokens.len() >= 2).map(|token| token.to_ascii_lowercase());
    let (detector, filter) = match (first, level == Some(JWST_COMBINED_LEVEL)) {
        (Some(token), true) => {
            let filters = split_optical_elements(&token.replace('-', ";").to_ascii_uppercase());
            (None, (!filters.is_empty()).then(|| filters.join("-")))
        }
        (Some(token), false) => (Some(token), None),
        (None, _) => (None, None),
    };
    let label = filter.clone().or_else(|| detector.clone()).unwrap_or_else(|| stem.to_owned());
    Parsed { label, group: suffix.clone(), suffix, detector, filter, colour: false }
}

fn hst_filter(tokens: &[String]) -> Option<String> {
    tokens
        .iter()
        .find(|token| {
            let bytes = token.as_bytes();
            bytes.len() >= 5
                && bytes[0] == b'f'
                && bytes[1..4].iter().all(u8::is_ascii_digit)
                && bytes[4..].iter().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
                && bytes[bytes.len() - 1].is_ascii_lowercase()
        })
        .map(|token| token.to_ascii_uppercase())
}

fn hst_detector(tokens: &[String]) -> Option<String> {
    let position = tokens.iter().position(|token| HST_INSTRUMENTS.contains(&token.as_str()))?;
    tokens.get(position + 1).filter(|token| HST_DETECTORS.contains(&token.as_str())).cloned()
}

fn hst_group(suffix: &str, hap: bool) -> &'static str {
    if hap {
        return "hap";
    }
    match suffix {
        "drz" | "drc" | "mos" => "combined",
        "flt" | "flc" | "c0m" | "c1m" => "calibrated",
        "ima" => "intermediate",
        "raw" | "d0m" => "raw",
        _ => "other",
    }
}

fn parse_hst(stem: &str, science_group: Option<&str>) -> Parsed {
    let lower = stem.to_ascii_lowercase();
    let colour = lower.ends_with(COLOUR_SUFFIX);
    let base = if colour { &lower[..lower.len() - COLOUR_SUFFIX.len()] } else { lower.as_str() };
    let suffix = last_token(base);
    let tokens: Vec<String> = lower.split('_').map(str::to_owned).collect();
    let hap = science_group.is_some_and(|group| group.eq_ignore_ascii_case(HAP_SCIENCE));
    Parsed {
        label: stem.to_owned(),
        group: hst_group(&suffix, hap).to_owned(),
        suffix,
        detector: if hap { hst_detector(&tokens) } else { None },
        filter: hst_filter(&tokens),
        colour,
    }
}

fn group_label(mission: MastMissionId, key: &str) -> String {
    if mission == MastMissionId::Hst {
        if let Some((_, label)) = HST_GROUPS.iter().find(|(k, _)| *k == key) {
            return (*label).to_owned();
        }
    }
    key.to_owned()
}

fn group_rank(mission: MastMissionId, key: &str, first_seen: &[String]) -> usize {
    let known: Vec<&str> = if mission == MastMissionId::Hst {
        HST_GROUPS.iter().map(|(k, _)| *k).collect()
    } else {
        SUFFIX_ORDER.to_vec()
    };
    match known.iter().position(|k| *k == key) {
        Some(index) => index,
        None => known.len() + first_seen.iter().position(|k| k == key).unwrap_or(first_seen.len()),
    }
}

fn first_with<'a>(entries: &'a [MastImageEntry], candidates: &[usize], test: impl Fn(&MastImageEntry) -> bool) -> Option<usize> {
    candidates.iter().copied().find(|&i| test(&entries[i]))
}

fn choose_representative(mission: MastMissionId, entries: &[MastImageEntry]) -> Option<usize> {
    let with_thumbs: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].thumbnail_filename.is_some()).collect();
    let candidates = if with_thumbs.is_empty() { (0..entries.len()).collect() } else { with_thumbs };
    if mission == MastMissionId::Hst {
        for suffix in HST_PREFERENCE {
            if let Some(index) = first_with(entries, &candidates, |e| e.group != "hap" && !e.colour && e.suffix == suffix) {
                return Some(index);
            }
        }
        return candidates.first().copied();
    }
    first_with(entries, &candidates, |e| e.suffix == "cal" && e.detector.as_deref().is_some_and(|d| d.ends_with("long")))
        .or_else(|| first_with(entries, &candidates, |e| e.suffix == "cal"))
        .or_else(|| SUFFIX_FALLBACK.iter().find_map(|suffix| first_with(entries, &candidates, |e| e.suffix == *suffix)))
        .or_else(|| candidates.first().copied())
}

pub fn merge_lists(mission: MastMissionId, dataset: &str, thumbs: &[ImageListResult], previews: &[ImageListResult]) -> MastImageSet {
    let mut entries: Vec<MastImageEntry> = collect(thumbs, previews)
        .into_iter()
        .map(|draft| {
            let parsed = match mission {
                MastMissionId::Hst => parse_hst(&draft.stem, draft.science_group.as_deref()),
                _ => parse_generic(&draft.stem, dataset, draft.level.as_deref()),
            };
            let product_filename = draft
                .viz_uri
                .as_deref()
                .and_then(last_segment)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{}.fits", draft.stem));
            MastImageEntry {
                mast_uri: mast_uri(mission, draft.viz_uri.as_deref(), &product_filename),
                key: draft.stem,
                label: parsed.label,
                group: parsed.group,
                product_filename,
                thumbnail_filename: draft.thumbnail,
                preview_filename: draft.preview,
                level: draft.level,
                suffix: parsed.suffix,
                detector: parsed.detector,
                filter: parsed.filter,
                science_group: draft.science_group,
                colour: parsed.colour,
            }
        })
        .collect();
    let mut first_seen: Vec<String> = Vec::new();
    for entry in &entries {
        if !first_seen.contains(&entry.group) {
            first_seen.push(entry.group.clone());
        }
    }
    entries.sort_by_key(|entry| (group_rank(mission, &entry.group, &first_seen), !entry.colour));
    let mut groups: Vec<ImageGroup> = Vec::new();
    for entry in &entries {
        match groups.iter_mut().find(|group| group.key == entry.group) {
            Some(group) => group.count += 1,
            None => groups.push(ImageGroup { key: entry.group.clone(), label: group_label(mission, &entry.group), count: 1 }),
        }
    }
    let representative = choose_representative(mission, &entries);
    let default_group = representative
        .map(|index| entries[index].group.clone())
        .or_else(|| groups.first().map(|group| group.key.clone()))
        .unwrap_or_default();
    MastImageSet {
        mission,
        dataset_id: dataset.to_owned(),
        entries,
        groups,
        default_group,
        representative,
        elapsed_ms: 0,
        from_cache: false,
    }
}

pub fn prefer_filter(set: &mut MastImageSet, filter: &str) -> bool {
    let group = representative_entry(set).map(|entry| entry.group.clone()).unwrap_or_else(|| set.default_group.clone());
    let found = set.entries.iter().position(|entry| {
        entry.group == group && entry.thumbnail_filename.is_some() && entry.filter.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(filter.trim()))
    });
    match found {
        Some(index) => {
            set.representative = Some(index);
            set.default_group = group;
            true
        }
        None => false,
    }
}

pub fn representative_entry(set: &MastImageSet) -> Option<&MastImageEntry> {
    set.representative.and_then(|index| set.entries.get(index))
}

pub fn needs_list_after_redirect(final_name: &str) -> bool {
    let lower = final_name.to_ascii_lowercase();
    REDIRECT_NEEDS_LIST.iter().any(|suffix| lower.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::types::ImageGroup;

    fn list(name: &str) -> Vec<ImageListResult> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn group(key: &str, label: &str, count: usize) -> ImageGroup {
        ImageGroup { key: key.into(), label: label.into(), count }
    }

    fn jwst_l3() -> MastImageSet {
        merge_lists(
            MastMissionId::Jwst,
            "jw02739-o001_t001_nircam",
            &list("il_jw02739-o001_t001_nircam_THUMBNAIL.json"),
            &list("il_jw02739-o001_t001_nircam_PREVIEW.json"),
        )
    }

    #[test]
    fn product_stem_strips_thumbnail_and_image_suffixes() {
        assert_eq!(product_stem("jw02739-o001_t001_nircam_clear-f090w_i2d_thumb.jpg"), "jw02739-o001_t001_nircam_clear-f090w_i2d");
        assert_eq!(product_stem("jw02739-o001_t001_nircam_clear-f090w_i2d.jpg"), "jw02739-o001_t001_nircam_clear-f090w_i2d");
        assert_eq!(product_stem("x_drc.png"), "x_drc");
        assert_eq!(product_stem("x_drc_thumb.PNG"), "x_drc");
        assert_eq!(product_stem("x_drc.fits"), "x_drc.fits");
    }

    #[test]
    fn merge_jwst_l3_lists() {
        let mut set = jwst_l3();
        assert_eq!(set.mission, MastMissionId::Jwst);
        assert_eq!(set.dataset_id, "jw02739-o001_t001_nircam");
        assert_eq!(set.entries.len(), 6);
        let labels: Vec<&str> = set.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["F090W", "F187N", "F200W", "F335M", "F444W", "F444W-F470N"]);
        assert_eq!(set.groups, vec![group("i2d", "i2d", 6)]);
        assert_eq!(set.default_group, "i2d");
        assert_eq!(set.representative, Some(0));
        assert!(!set.from_cache);
        let first = &set.entries[0];
        assert_eq!(first.key, "jw02739-o001_t001_nircam_clear-f090w_i2d");
        assert_eq!(first.group, "i2d");
        assert_eq!(first.suffix, "i2d");
        assert_eq!(first.level.as_deref(), Some("3"));
        assert_eq!(first.filter.as_deref(), Some("F090W"));
        assert_eq!(first.detector, None);
        assert_eq!(first.thumbnail_filename.as_deref(), Some("jw02739-o001_t001_nircam_clear-f090w_i2d_thumb.jpg"));
        assert_eq!(first.preview_filename.as_deref(), Some("jw02739-o001_t001_nircam_clear-f090w_i2d.jpg"));
        assert_eq!(first.product_filename, "jw02739-o001_t001_nircam_clear-f090w_i2d.fits");
        assert_eq!(first.mast_uri, "mast:JWST/product/jw02739-o001_t001_nircam_clear-f090w_i2d.fits");
        assert!(!first.colour);
        assert_eq!(first.science_group, None);
        assert_eq!(set.entries[5].filter.as_deref(), Some("F444W-F470N"));
        assert!(prefer_filter(&mut set, "f187n"));
        assert_eq!(set.representative, Some(1));
        assert_eq!(representative_entry(&set).unwrap().label, "F187N");
        assert!(!prefer_filter(&mut set, "F999W"));
        assert_eq!(set.representative, Some(1));
    }

    #[test]
    fn prefer_filter_survives_a_stored_representative_out_of_range() {
        let mut set = jwst_l3();
        set.representative = Some(set.entries.len());
        assert!(prefer_filter(&mut set, "F187N"));
        assert_eq!(set.representative, Some(1));
        assert_eq!(set.default_group, "i2d");
        set.representative = Some(usize::MAX);
        set.default_group = "missing".into();
        assert!(!prefer_filter(&mut set, "F187N"));
        assert_eq!(set.representative, Some(usize::MAX));
    }

    #[test]
    fn merge_jwst_exposure_lists() {
        let thumbs = list("il_jw02739001002_02101_00001_THUMBNAIL.json");
        let previews = list("il_jw02739001002_02101_00001_PREVIEW.json");
        let set = merge_lists(MastMissionId::Jwst, "jw02739001002_02101_00001", &thumbs, &previews);
        assert_eq!(set.entries.len(), 60);
        let keys: Vec<&str> = set.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["cal", "i2d", "crf", "rate", "rateints", "uncal"]);
        assert!(set.groups.iter().all(|g| g.count == 10 && g.label == g.key), "{:?}", set.groups);
        assert_eq!(set.default_group, "cal");
        let representative = representative_entry(&set).unwrap();
        assert_eq!(representative.suffix, "cal");
        assert_eq!(representative.detector.as_deref(), Some("nrcalong"));
        assert_eq!(representative.key, "jw02739001002_02101_00001_nrcalong_cal");
        let detectors: Vec<&str> = set.entries.iter().filter(|e| e.group == "cal").filter_map(|e| e.detector.as_deref()).collect();
        assert_eq!(
            detectors,
            ["nrca1", "nrca2", "nrca3", "nrca4", "nrcalong", "nrcb1", "nrcb2", "nrcb3", "nrcb4", "nrcblong"]
        );
        assert!(set.entries[..10].iter().all(|e| e.group == "cal"));
        let crf = set.entries.iter().find(|e| e.key == "jw02739001002_02101_00001_nrca1_o001_crf").unwrap();
        assert_eq!((crf.suffix.as_str(), crf.detector.as_deref(), crf.label.as_str()), ("crf", Some("nrca1"), "nrca1"));
        assert_eq!(crf.level.as_deref(), Some("2c"));
        assert_eq!(crf.filter, None);
        assert!(set.entries.iter().all(|e| e.thumbnail_filename.is_some() && e.preview_filename.is_some()));

        let short_only: Vec<ImageListResult> = thumbs.iter().filter(|e| !e.filename.contains("long")).cloned().collect();
        let fallback = merge_lists(MastMissionId::Jwst, "jw02739001002_02101_00001", &short_only, &[]);
        assert_eq!(representative_entry(&fallback).unwrap().key, "jw02739001002_02101_00001_nrca1_cal");
        let no_cal: Vec<ImageListResult> = thumbs
            .iter()
            .filter(|e| e.filename.contains("_rate") || e.filename.contains("_uncal"))
            .cloned()
            .collect();
        let rate = merge_lists(MastMissionId::Jwst, "jw02739001002_02101_00001", &no_cal, &[]);
        assert_eq!(representative_entry(&rate).unwrap().key, "jw02739001002_02101_00001_nrca1_rate");
        assert_eq!(rate.default_group, "rate");
        assert!(rate.entries.iter().all(|e| e.preview_filename.is_none()));
    }

    #[test]
    fn merge_hst_lists() {
        let set = merge_lists(
            MastMissionId::Hst,
            "IDNM20010",
            &list("hst_il_IDNM20010_THUMBNAIL.json"),
            &list("hst_il_IDNM20010_PREVIEW.json"),
        );
        assert_eq!(set.entries.len(), 20);
        assert_eq!(
            set.groups,
            vec![
                group("combined", "Combined", 1),
                group("calibrated", "Calibrated", 3),
                group("intermediate", "Intermediate", 3),
                group("raw", "Raw", 3),
                group("hap", "HAP aligned", 10),
            ]
        );
        assert_eq!(set.default_group, "combined");
        assert_eq!(set.representative, Some(0));
        assert_eq!(set.entries[0].key, "idnm20010_drz");
        assert_eq!(set.entries[0].level, None);
        let hap: Vec<&MastImageEntry> = set.entries.iter().filter(|e| e.group == "hap").collect();
        assert_eq!(hap[0].key, "hst_15238_20_wfc3_ir_total_idnm20_drz_color");
        assert!(hap[0].colour);
        assert_eq!(hap[0].suffix, "drz");
        assert_eq!(hap[0].science_group.as_deref(), Some("HapScience"));
        assert!(hap[1..].iter().all(|e| !e.colour));
        let f160w = set.entries.iter().find(|e| e.key == "hst_15238_20_wfc3_ir_f160w_idnm20_drz").unwrap();
        assert_eq!(f160w.filter.as_deref(), Some("F160W"));
        assert_eq!(f160w.detector.as_deref(), Some("ir"));
        let flt = set.entries.iter().find(|e| e.key == "idnm20cpq_flt").unwrap();
        assert_eq!((flt.group.as_str(), flt.label.as_str(), flt.filter.as_deref()), ("calibrated", "idnm20cpq_flt", None));
        assert_eq!(flt.mast_uri, "mast:HST/product/idnm20cpq_flt.fits");

        let ick = merge_lists(MastMissionId::Hst, "ICK909030", &list("il_ICK909030_THUMBNAIL.json"), &list("il_ICK909030_PREVIEW.json"));
        assert_eq!(ick.entries.len(), 17);
        let keys: Vec<(&str, usize)> = ick.groups.iter().map(|g| (g.key.as_str(), g.count)).collect();
        assert_eq!(keys, [("combined", 2), ("calibrated", 4), ("raw", 2), ("hap", 9)]);
        assert_eq!(representative_entry(&ick).unwrap().key, "ick909030_drc");
        assert_eq!(ick.default_group, "combined");
        assert_eq!(ick.entries.iter().find(|e| e.group == "hap").unwrap().key, "hst_13926_09_wfc3_uvis_total_ick909_drc_color");
        let raw_only: Vec<ImageListResult> = list("il_ICK909030_THUMBNAIL.json").into_iter().filter(|e| e.filename.contains("_raw")).collect();
        let raw = merge_lists(MastMissionId::Hst, "ICK909030", &raw_only, &[]);
        assert_eq!(representative_entry(&raw).unwrap().key, "ick909c1q_raw");
    }

    #[test]
    fn unsafe_names_are_dropped_and_empty_lists_have_no_representative() {
        let mut thumbs = list("il_jw02739-o001_t001_nircam_THUMBNAIL.json");
        thumbs[0].filename = "../escape_i2d_thumb.jpg".into();
        thumbs[1].filename = String::new();
        let set = merge_lists(MastMissionId::Jwst, "jw02739-o001_t001_nircam", &thumbs, &[]);
        assert_eq!(set.entries.len(), 4);
        assert!(set.entries.iter().all(|e| !e.key.contains("..")));
        let empty = merge_lists(MastMissionId::Jwst, "jw1", &[], &[]);
        assert!(empty.entries.is_empty() && empty.groups.is_empty());
        assert_eq!(empty.representative, None);
        assert_eq!(empty.default_group, "");
        assert!(representative_entry(&empty).is_none());
    }

    #[test]
    fn hst_redirect_names_that_need_the_list() {
        assert!(needs_list_after_redirect("ick909c1q_raw_thumb.jpg"));
        assert!(needs_list_after_redirect("idnm20cpq_ima_thumb.jpg"));
        assert!(needs_list_after_redirect("IDNM20CPQ_IMA_THUMB.JPG"));
        assert!(!needs_list_after_redirect("ick909030_drc_thumb.jpg"));
        assert!(!needs_list_after_redirect("get_image_thumbnail"));
    }
}
