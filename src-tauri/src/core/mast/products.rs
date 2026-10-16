use std::collections::BTreeMap;
use std::fmt::Write;

use super::images::mast_uri;
use super::types::{MastMissionId, MastProduct, MastProductClass, MastProductList, OpensAs, ProductBadge, ProductGroup};
use crate::infra::mast::models::DatasetProduct;

pub const DOWNLOAD_FILE_URL: &str = "https://mast.stsci.edu/api/v0.1/Download/file?uri=";

const IMAGE_SUFFIXES: [&str; 10] = ["cal", "calints", "rate", "rateints", "crf", "i2d", "flt", "flc", "drz", "drc"];
const CATALOGUES: [(&str, &str); 4] = [("cat", ".ecsv"), ("segm", ".fits"), ("point-cat", ".ecsv"), ("segment-cat", ".ecsv")];
const PREVIEW_EXTENSIONS: [&str; 3] = [".jpg", ".jpeg", ".png"];
const OPENABLE_EXTENSIONS: [&str; 2] = [".fits", ".asdf"];
const AUXILIARY_CATEGORIES: [&str; 3] = ["AUX", "JITTER/SUPPORT", "OTHER"];
const HAP_CATEGORY_PREFIX: &str = "HAP-";

pub fn normalized_suffix(file_suffix: &str) -> String {
    file_suffix.trim().trim_start_matches('_').to_ascii_lowercase()
}

pub fn product_class(category: &str, product_type: &str, suffix: &str, filename: &str) -> MastProductClass {
    let category = category.trim();
    let filename = filename.trim().to_ascii_lowercase();
    let has_extension = |extensions: &[&str]| extensions.iter().any(|extension| filename.ends_with(extension));
    if category.eq_ignore_ascii_case("REFERENCE") {
        return MastProductClass::Reference;
    }
    if category.eq_ignore_ascii_case("GUIDESTAR") {
        return MastProductClass::Guidestar;
    }
    if product_type.trim().eq_ignore_ascii_case("association") || matches!(suffix, "asn" | "pool") {
        return MastProductClass::Association;
    }
    if has_extension(&PREVIEW_EXTENSIONS) || suffix.ends_with("thumb") || suffix.ends_with("preview") {
        return MastProductClass::Preview;
    }
    if CATALOGUES.iter().any(|(name, extension)| suffix == *name && filename.ends_with(extension)) {
        return MastProductClass::Catalog;
    }
    if has_extension(&OPENABLE_EXTENSIONS) {
        match suffix {
            _ if IMAGE_SUFFIXES.contains(&suffix) => return MastProductClass::Image,
            "s3d" => return MastProductClass::Cube,
            "x1d" | "x1dints" => return MastProductClass::Spectrum,
            "uncal" => return MastProductClass::Ramp,
            _ => {}
        }
    }
    if AUXILIARY_CATEGORIES.iter().any(|name| category.eq_ignore_ascii_case(name)) {
        return MastProductClass::Auxiliary;
    }
    MastProductClass::Other
}

pub fn opens_as(cls: MastProductClass) -> Option<OpensAs> {
    match cls {
        MastProductClass::Image => Some(OpensAs::Image),
        MastProductClass::Cube => Some(OpensAs::Cube),
        MastProductClass::Spectrum => Some(OpensAs::Spectrum),
        MastProductClass::Ramp => Some(OpensAs::Ramp),
        MastProductClass::Catalog => Some(OpensAs::Catalog),
        _ => None,
    }
}

pub fn default_visible(cls: MastProductClass) -> bool {
    opens_as(cls).is_some()
}

pub fn product_mast_uri(mission: MastMissionId, listing_uri: &str, filename: &str) -> String {
    let listing_uri = listing_uri.trim();
    if listing_uri.starts_with("mast:") {
        listing_uri.to_owned()
    } else {
        mast_uri(mission, None, filename)
    }
}

pub fn download_url(mast_uri: &str) -> String {
    let mut url = String::from(DOWNLOAD_FILE_URL);
    for byte in mast_uri.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~:/".contains(&byte) {
            url.push(char::from(byte));
        } else {
            let _ = write!(url, "%{byte:02X}");
        }
    }
    url
}

fn owner(dataset_ids: &[String], listed: Option<&str>) -> String {
    match listed.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => dataset_ids.iter().find(|id| id.eq_ignore_ascii_case(name)).cloned().unwrap_or_else(|| name.to_owned()),
        None if dataset_ids.len() == 1 => dataset_ids[0].clone(),
        None => String::new(),
    }
}

fn product(mission: MastMissionId, dataset_ids: &[String], raw: DatasetProduct) -> MastProduct {
    let filename = raw.filename.trim().to_owned();
    let category = raw.category.trim().to_owned();
    let product_type = raw.product_type.trim().to_owned();
    let suffix = normalized_suffix(&raw.file_suffix);
    let cls = product_class(&category, &product_type, &suffix, &filename);
    let mast_uri = product_mast_uri(mission, &raw.uri, &filename);
    let hap = mission == MastMissionId::Hst && category.to_ascii_uppercase().starts_with(HAP_CATEGORY_PREFIX);
    MastProduct {
        dataset_id: owner(dataset_ids, raw.dataset.as_deref()),
        size_bytes: u64::try_from(raw.size).unwrap_or(0),
        access: raw.access.trim().to_owned(),
        download_url: download_url(&mast_uri),
        opens_as: opens_as(cls),
        badges: if hap { vec![ProductBadge::HapAligned] } else { Vec::new() },
        filename,
        category,
        product_type,
        suffix,
        cls,
        mast_uri,
    }
}

pub fn classify(mission: MastMissionId, dataset_ids: &[String], listing: Vec<DatasetProduct>) -> MastProductList {
    let mut products: Vec<MastProduct> = listing.into_iter().map(|raw| product(mission, dataset_ids, raw)).collect();
    products.sort_by(|a, b| (a.cls, &a.dataset_id, &a.filename).cmp(&(b.cls, &b.dataset_id, &b.filename)));
    let mut totals: BTreeMap<MastProductClass, (usize, u64)> = BTreeMap::new();
    for item in &products {
        let entry = totals.entry(item.cls).or_default();
        entry.0 += 1;
        entry.1 += item.size_bytes;
    }
    let groups = totals
        .into_iter()
        .map(|(cls, (count, bytes))| ProductGroup { cls, count, bytes, default_visible: default_visible(cls) })
        .collect();
    MastProductList {
        mission,
        dataset_ids: dataset_ids.to_vec(),
        total_count: products.len(),
        total_bytes: products.iter().map(|item| item.size_bytes).sum(),
        products,
        groups,
        elapsed_ms: 0,
        from_cache: false,
        cached_until: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::mast::models::DatasetProducts;
    use std::path::Path;

    const L3: &str = "jw02739-o001_t001_nircam";
    const ASSOCIATION: &str = "ICK909030";

    fn listing(name: &str) -> Vec<DatasetProduct> {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name)).unwrap();
        serde_json::from_str::<DatasetProducts>(&text).unwrap().products
    }

    fn histogram(list: &MastProductList) -> BTreeMap<MastProductClass, usize> {
        let mut counts = BTreeMap::new();
        for product in &list.products {
            *counts.entry(product.cls).or_insert(0) += 1;
        }
        counts
    }

    fn groups(list: &MastProductList) -> Vec<(MastProductClass, usize, u64, bool)> {
        list.groups.iter().map(|g| (g.cls, g.count, g.bytes, g.default_visible)).collect()
    }

    fn find<'a>(list: &'a MastProductList, filename: &str) -> &'a MastProduct {
        list.products.iter().find(|p| p.filename == filename).unwrap_or_else(|| panic!("{filename} missing"))
    }

    fn sorted(list: &MastProductList) -> bool {
        list.products.windows(2).all(|pair| (pair[0].cls, &pair[0].dataset_id, &pair[0].filename) <= (pair[1].cls, &pair[1].dataset_id, &pair[1].filename))
    }

    #[test]
    fn classify_jwst_l3_listing() {
        use MastProductClass::*;
        let list = classify(MastMissionId::Jwst, &[L3.to_owned()], listing("prod_jw.json"));
        assert_eq!(list.mission, MastMissionId::Jwst);
        assert_eq!(list.dataset_ids, vec![L3.to_owned()]);
        assert_eq!(list.total_count, 108);
        assert_eq!(list.total_bytes, 32_869_323_631);
        assert_eq!(
            histogram(&list),
            BTreeMap::from([(Image, 6), (Catalog, 12), (Preview, 12), (Association, 7), (Reference, 38), (Guidestar, 33)])
        );
        assert_eq!(
            groups(&list),
            vec![
                (Image, 6, 19_088_533_440, true),
                (Catalog, 12, 1_982_531_990, true),
                (Preview, 12, 26_775_820, false),
                (Association, 7, 1_103_126, false),
                (Reference, 38, 11_644_739_255, false),
                (Guidestar, 33, 125_640_000, false),
            ]
        );
        assert!(sorted(&list));
        let images: Vec<&MastProduct> = list.products.iter().filter(|p| p.cls == Image).collect();
        let filters: Vec<&str> = images
            .iter()
            .map(|p| p.filename.trim_start_matches("jw02739-o001_t001_nircam_").trim_end_matches("_i2d.fits"))
            .collect();
        assert_eq!(filters, ["clear-f090w", "clear-f187n", "clear-f200w", "clear-f335m", "clear-f444w", "f444w-f470n"]);
        for image in &images {
            assert_eq!(image.opens_as, Some(OpensAs::Image));
            assert_eq!(image.suffix, "i2d");
            assert_eq!(image.dataset_id, L3);
            assert_eq!(image.category, "3");
            assert_eq!(image.product_type, "science");
            assert_eq!(image.access, "PUBLIC");
            assert_eq!(image.mast_uri, format!("mast:JWST/product/{}", image.filename));
            assert_eq!(image.download_url, format!("{DOWNLOAD_FILE_URL}mast:JWST/product/{}", image.filename));
            assert!(image.badges.is_empty());
        }
        let segm = find(&list, "jw02739-o001_t001_nircam_f444w-f470n_segm.fits");
        assert_eq!((segm.cls, segm.opens_as, segm.suffix.as_str()), (Catalog, Some(OpensAs::Catalog), "segm"));
        let catalogue = find(&list, "jw02739-o001_t001_nircam_clear-f090w_cat.ecsv");
        assert_eq!((catalogue.cls, catalogue.opens_as), (Catalog, Some(OpensAs::Catalog)));
        let thumb = find(&list, "jw02739-o001_t001_nircam_clear-f187n_i2d_thumb.jpg");
        assert_eq!((thumb.cls, thumb.opens_as, thumb.suffix.as_str(), thumb.size_bytes), (Preview, None, "thumb", 3507));
        let preview = find(&list, "jw02739-o001_t001_nircam_clear-f444w_i2d.jpg");
        assert_eq!((preview.cls, preview.suffix.as_str()), (Preview, "preview"));
        let guide = find(&list, "jw02739001001_gs-track_2022226085853_cal.fits");
        assert_eq!((guide.cls, guide.opens_as, guide.suffix.as_str()), (Guidestar, None, "cal"));
        let dark = find(&list, "jwst_nircam_dark_0331.fits");
        assert_eq!((dark.cls, dark.size_bytes, dark.category.as_str()), (Reference, 6_291_483_840, "REFERENCE"));
        let pool = find(&list, "jw02739_20260928t225233_pool.csv");
        assert_eq!((pool.cls, pool.suffix.as_str()), (Association, "pool"));
        assert!(list.products.iter().all(|p| p.download_url == format!("{DOWNLOAD_FILE_URL}{}", p.mast_uri)));
        assert_eq!((list.elapsed_ms, list.from_cache, list.cached_until.as_deref()), (0, false, None));
    }

    #[test]
    fn classify_hst_listing() {
        use MastProductClass::*;
        let list = classify(MastMissionId::Hst, &[ASSOCIATION.to_owned()], listing("prod_hst.json"));
        assert_eq!(list.total_count, 94);
        assert_eq!(list.total_bytes, 8_625_648_596);
        assert_eq!(
            histogram(&list),
            BTreeMap::from([(Image, 14), (Catalog, 4), (Preview, 34), (Association, 1), (Auxiliary, 8), (Reference, 18), (Other, 15)])
        );
        assert_eq!(
            groups(&list),
            vec![
                (Image, 14, 7_375_374_720, true),
                (Catalog, 4, 20_785_330, true),
                (Preview, 34, 55_322_502, false),
                (Association, 1, 11_520, false),
                (Auxiliary, 8, 937_822, false),
                (Reference, 18, 1_100_146_095, false),
                (Other, 15, 73_070_607, false),
            ]
        );
        assert!(sorted(&list));
        let drc_jpegs: Vec<&MastProduct> = list.products.iter().filter(|p| p.suffix == "drc" && p.filename.ends_with(".jpg")).collect();
        assert_eq!(drc_jpegs.len(), 7);
        assert!(drc_jpegs.iter().all(|p| p.cls == Preview && p.opens_as.is_none()), "{drc_jpegs:?}");
        let images: Vec<&MastProduct> = list.products.iter().filter(|p| p.cls == Image).collect();
        assert!(images.iter().all(|p| p.filename.ends_with(".fits") && ["drz", "drc", "flc", "flt"].contains(&p.suffix.as_str())));
        assert_eq!(images.iter().filter(|p| p.badges == [ProductBadge::HapAligned]).count(), 8);
        let drc = find(&list, "ick909030_drc.fits");
        assert_eq!((drc.cls, drc.opens_as, drc.suffix.as_str(), drc.category.as_str()), (Image, Some(OpensAs::Image), "drc", "CALIBRATED"));
        assert!(drc.badges.is_empty());
        assert_eq!(drc.mast_uri, "mast:HST/product/ick909030_drc.fits");
        assert_eq!(drc.dataset_id, ASSOCIATION);
        let hlet = find(&list, "ick909c1q_flt_hlet.fits");
        assert_eq!((hlet.cls, hlet.suffix.as_str()), (Other, "flt_hlet"));
        let point = find(&list, "hst_13926_09_wfc3_uvis_total_ick909_point-cat.ecsv");
        assert_eq!((point.cls, point.opens_as, point.badges.as_slice()), (Catalog, Some(OpensAs::Catalog), [ProductBadge::HapAligned].as_slice()));
        assert_eq!(find(&list, "hst_13926_09_wfc3_uvis_total_ick909_segment-cat.ecsv").cls, Catalog);
        assert_eq!(find(&list, "ick909030_asn.fits").cls, Association);
        assert_eq!(find(&list, "ick909030_jif.fits").cls, Auxiliary);
        assert_eq!(find(&list, "ick909030_trl.fits").cls, Auxiliary);
        assert_eq!(find(&list, "ick909030_log.txt").cls, Auxiliary);
        assert_eq!(find(&list, "ick909c1q_raw.fits").cls, Other);
        assert_eq!(find(&list, "ick909030_spt.fits").cls, Other);
        assert_eq!(find(&list, "hst_13926_09_wfc3_uvis_f502n_ick909c1_trl.txt").cls, Other);
        assert_eq!(find(&list, "ick909c1q_raw_thumb.jpg").cls, Preview);
        let colour = find(&list, "hst_13926_09_wfc3_uvis_total_ick909_drc_color.jpg");
        assert_eq!((colour.cls, colour.suffix.as_str(), colour.badges.as_slice()), (Preview, "drc_color", [ProductBadge::HapAligned].as_slice()));
        assert_eq!(list.products.iter().filter(|p| !p.badges.is_empty()).count(), 38);
        assert!(list.products.iter().filter(|p| !p.badges.is_empty()).all(|p| p.category.starts_with("HAP-")));
        assert_eq!(find(&list, "2731450pi_idc.fits").cls, Reference);
        assert_eq!(find(&list, "hst_1338.pmap").cls, Reference);
    }

    #[test]
    fn classify_iue_listing_maps_the_dataset_case_and_keeps_listing_uris() {
        use MastProductClass::*;
        let list = classify(MastMissionId::Iue, &["LWP00501".to_owned()], listing("prod_iue.json"));
        assert_eq!(list.total_count, 6);
        assert!(list.products.iter().all(|p| p.dataset_id == "LWP00501"), "{:?}", list.products);
        let gif = find(&list, "lwp00501.gif");
        assert_eq!((gif.cls, gif.size_bytes, gif.suffix.as_str()), (Preview, 0, "mx_preview"));
        assert_eq!(gif.mast_uri, "mast:IUE/url/browse/previews/iue/mx/lwp/00000/gif/lwp00501.gif");
        let spectrum = find(&list, "lwp00501.mxhi.gz");
        assert_eq!((spectrum.cls, spectrum.opens_as), (Other, None));
        assert_eq!(spectrum.mast_uri, "mast:IUE/url/missions/iue/data/lwp/00000/lwp00501.mxhi.gz");
        assert_eq!(spectrum.download_url, format!("{DOWNLOAD_FILE_URL}mast:IUE/url/missions/iue/data/lwp/00000/lwp00501.mxhi.gz"));
        assert_eq!(histogram(&list), BTreeMap::from([(Preview, 1), (Other, 5)]));
        assert_eq!(list.total_bytes, 717_341 + 495_268 + 344_941 + 646_013 + 683);
        let multi = classify(MastMissionId::Iue, &["SWP01000".to_owned(), "LWP00501".to_owned()], listing("prod_iue.json"));
        assert!(multi.products.iter().all(|p| p.dataset_id == "LWP00501"));
        let unknown = classify(MastMissionId::Iue, &["SWP01000".to_owned(), "SWP01001".to_owned()], listing("prod_iue.json"));
        assert!(unknown.products.iter().all(|p| p.dataset_id == "lwp00501"));
    }

    #[test]
    fn product_rules_cover_the_design_table() {
        use MastProductClass::*;
        assert_eq!(normalized_suffix(" _I2D "), "i2d");
        assert_eq!(normalized_suffix("FLT_HLET"), "flt_hlet");
        assert_eq!(normalized_suffix(""), "");
        let cases = [
            ("2b", "science", "cal", "jw1_nrca1_cal.fits", Image),
            ("2b", "science", "calints", "jw1_calints.fits", Image),
            ("2a", "science", "rate", "jw1_rate.fits", Image),
            ("2a", "science", "rateints", "jw1_rateints.fits", Image),
            ("2b", "science", "crf", "jw1_crf.fits", Image),
            ("3", "science", "i2d", "r1_i2d.asdf", Image),
            ("3", "science", "s3d", "jw1_s3d.fits", Cube),
            ("3", "science", "x1d", "jw1_x1d.fits", Spectrum),
            ("2b", "science", "x1dints", "jw1_x1dints.fits", Spectrum),
            ("1b", "science", "uncal", "jw1_uncal.fits", Ramp),
            ("3", "science", "cat", "jw1_cat.ecsv", Catalog),
            ("3", "science", "segm", "jw1_segm.fits", Catalog),
            ("3", "science", "cat", "jw1_cat.fits", Other),
            ("3", "science", "i2d", "jw1_i2d.png", Preview),
            ("3", "science", "x1d", "jw1_x1d_preview.fits", Spectrum),
            ("3", "science", "x1d_preview", "jw1_x1d.fits", Preview),
            ("3", "science", "i2d", "jw1_i2d.json", Other),
            ("3", "auxiliary", "asn", "jw1_asn.json", Association),
            ("ASSOCIATION", "Association", "", "x.json", Association),
            ("reference", "reference", "cal", "jwst_cal.fits", Reference),
            ("GuideStar", "guidestar", "cal", "gs_cal.fits", Guidestar),
            ("AUX", "science", "trl", "x_trl.fits", Auxiliary),
            ("other", "science", "spt", "x_spt.fits", Auxiliary),
            ("UNCALIBRATED", "science", "raw", "x_raw.fits", Other),
            ("HAP-SVM", "exposure", "wht", "x_wht.fits", Other),
        ];
        for (category, product_type, suffix, filename, expected) in cases {
            assert_eq!(product_class(category, product_type, suffix, filename), expected, "{category} {product_type} {suffix} {filename}");
        }
        for (cls, opens) in [
            (Image, Some(OpensAs::Image)),
            (Cube, Some(OpensAs::Cube)),
            (Spectrum, Some(OpensAs::Spectrum)),
            (Ramp, Some(OpensAs::Ramp)),
            (Catalog, Some(OpensAs::Catalog)),
            (Preview, None),
            (Association, None),
            (Auxiliary, None),
            (Reference, None),
            (Guidestar, None),
            (Other, None),
        ] {
            assert_eq!(opens_as(cls), opens, "{cls:?}");
            assert_eq!(default_visible(cls), opens.is_some(), "{cls:?}");
        }
    }

    #[test]
    fn mast_uri_and_download_url_forms() {
        assert_eq!(product_mast_uri(MastMissionId::Jwst, "jw02739-o001_t001_nircam/x_i2d.fits", "x_i2d.fits"), "mast:JWST/product/x_i2d.fits");
        assert_eq!(product_mast_uri(MastMissionId::Hst, "ICK909030/ick909030_drc.fits", "ick909030_drc.fits"), "mast:HST/product/ick909030_drc.fits");
        assert_eq!(product_mast_uri(MastMissionId::Hst, "", "ick909030_drc.fits"), "mast:HST/product/ick909030_drc.fits");
        assert_eq!(product_mast_uri(MastMissionId::Roman, " mast:ROMAN/product/r0001_cal.asdf ", "r0001_cal.asdf"), "mast:ROMAN/product/r0001_cal.asdf");
        assert_eq!(
            product_mast_uri(MastMissionId::Iue, "mast:IUE/url//pub/iue/data/lwp/00000/lwp00501.lihi.gz", "lwp00501.lihi.gz"),
            "mast:IUE/url//pub/iue/data/lwp/00000/lwp00501.lihi.gz"
        );
        assert_eq!(
            download_url("mast:JWST/product/jw02739-o001_t001_nircam_clear-f187n_i2d.fits"),
            "https://mast.stsci.edu/api/v0.1/Download/file?uri=mast:JWST/product/jw02739-o001_t001_nircam_clear-f187n_i2d.fits"
        );
        assert_eq!(
            download_url("mast:HLSP/classy/j0021+0052/hlsp_classy_hst_cos_j0021+0052_multi_v1_coadded.fits"),
            "https://mast.stsci.edu/api/v0.1/Download/file?uri=mast:HLSP/classy/j0021%2B0052/hlsp_classy_hst_cos_j0021%2B0052_multi_v1_coadded.fits"
        );
        assert_eq!(
            download_url("mast:X/product/a b&c#d%e=f?g~h.fits"),
            "https://mast.stsci.edu/api/v0.1/Download/file?uri=mast:X/product/a%20b%26c%23d%25e%3Df%3Fg~h.fits"
        );
        assert_eq!(download_url("mast:X/product/\u{e9}.fits"), format!("{DOWNLOAD_FILE_URL}mast:X/product/%C3%A9.fits"));
    }
}
