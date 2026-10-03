use std::path::Path;

use anyhow::Result;

use crate::core::ramp::info::RampInfo;
use crate::core::ramp::irs2::CARD_UNCAL_FAST_LEN;
use crate::core::ramp::quick_slope::{QuickSlopeParams, QuickSlopeProduct};
use crate::infra::fits::writer::{write_mef_images, HduData, ImageHdu};
use crate::types::header::HduHeader;
use crate::types::image_ref::source_stem;

pub const CARD_ABPROC: &str = "ABPROC";
pub const CARD_VERSION: &str = "ABQSVER";
pub const CARD_INTEGRATION: &str = "ABINTEG";
pub const CARD_NGROUPS: &str = "ABNGROUP";
pub const CARD_TGROUP: &str = "ABTGROUP";
pub const CARD_SAT_DN: &str = "ABSATDN";
pub const CARD_JUMP_K: &str = "ABJUMPK";
pub const CARD_REF_CORRECTION: &str = "ABREFCOR";
pub const CARD_REF_WINDOW: &str = "ABREFWIN";
pub const CARD_FILENAME: &str = "FILENAME";
pub const CARD_HISTORY: &str = "HISTORY";
pub const CARD_BUNIT: &str = "BUNIT";
pub const CARD_NOTE: &str = "ABNOTE";

pub const QSLOPE_SUFFIX: &str = "_qslope";
pub const UNCAL_SUFFIX: &str = "_uncal";
pub const RATE_SUFFIX: &str = "_rate";
pub const ABPROC_QSLOPE: &str = "qslope";
pub const EXT_SCI: &str = "SCI";
pub const EXT_NGOOD: &str = "NGOOD";
pub const EXT_DQ: &str = "DQ";
pub const EXT_NOISE: &str = "NOISE";
pub const HISTORY_MAX_BYTES: usize = 72;
pub const PRIMARY_KEYS_DROPPED: [&str; 6] = ["DATAMODL", "FILENAME", "DATE", "CHECKSUM", "DATASUM", "NEXTEND"];
pub const PRIMARY_PREFIXES_DROPPED: [&str; 4] = ["S_", "R_", "CAL_", "CRDS_"];
pub const REFCOR_AMPLIFIER: &str = "amplifier";
pub const REFCOR_OFF: &str = "off";
pub const REFCOR_NONE: &str = "none";
pub const BUNIT_RATE: &str = "DN/s";
pub const BUNIT_GROUPS: &str = "groups";
pub const DQ_NOTE: &str = "JWST DQ bits: DO_NOT_USE 1, SATURATED 2, JUMP_DET 4";
pub const NOISE_NOTE: &str = "slope error from the robust scale of group differences";

fn strip_suffix_ignore_case(stem: &str, suffix: &str) -> Option<String> {
    let cut = stem.len().checked_sub(suffix.len())?;
    stem.get(cut..)
        .filter(|tail| tail.eq_ignore_ascii_case(suffix))
        .map(|_| stem[..cut].to_string())
}

pub fn qslope_stem(input_path: &str) -> String {
    let stem = source_stem(input_path);
    strip_suffix_ignore_case(&stem, UNCAL_SUFFIX).unwrap_or(stem)
}

fn join_output(output_dir: &str, file_name: &str) -> String {
    format!("{}/{}", output_dir.trim_end_matches(['/', '\\']), file_name)
}

pub fn qslope_output_path(input_path: &str, output_dir: &str, nints: usize, integration: usize) -> String {
    let stem = qslope_stem(input_path);
    let name = if nints > 1 {
        format!("{stem}_int{:03}{QSLOPE_SUFFIX}.fits", integration + 1)
    } else {
        format!("{stem}{QSLOPE_SUFFIX}.fits")
    };
    join_output(output_dir, &name)
}

pub fn sibling_rate_path(uncal_path: &str) -> Option<String> {
    let file_name = format!("{}{RATE_SUFFIX}.fits", qslope_stem(uncal_path));
    let candidate = match Path::new(uncal_path).parent().map(|p| p.to_string_lossy().into_owned()) {
        Some(directory) if !directory.is_empty() => join_output(&directory, &file_name),
        _ => file_name,
    };
    Path::new(&candidate).exists().then_some(candidate)
}

fn is_dropped_primary_key(key: &str) -> bool {
    PRIMARY_KEYS_DROPPED.contains(&key) || PRIMARY_PREFIXES_DROPPED.iter().any(|p| key.starts_with(p))
}

fn ref_correction_label(product: &QuickSlopeProduct) -> &'static str {
    match (&product.layout, product.ref_corrected) {
        (Some(_), true) => REFCOR_AMPLIFIER,
        (Some(_), false) => REFCOR_OFF,
        (None, _) => REFCOR_NONE,
    }
}

pub fn qslope_primary_header(
    uncal_primary: &HduHeader,
    info: &RampInfo,
    params: &QuickSlopeParams,
    product: &QuickSlopeProduct,
    output_name: &str,
) -> HduHeader {
    let mut header = uncal_primary.clone();
    let doomed: Vec<String> = header
        .cards
        .iter()
        .map(|(k, _)| k.trim().to_string())
        .chain(header.index.keys().cloned())
        .filter(|k| is_dropped_primary_key(k))
        .collect();
    for key in doomed {
        header.remove(&key);
    }
    header.set(CARD_FILENAME, output_name.to_string());
    header.set(CARD_ABPROC, ABPROC_QSLOPE.to_string());
    header.set(CARD_VERSION, env!("CARGO_PKG_VERSION").to_string());
    header.set(CARD_INTEGRATION, product.integration.to_string());
    header.set(CARD_NGROUPS, info.ngroups.to_string());
    header.set(CARD_TGROUP, format!("{}", product.tgroup_s));
    header.set(CARD_SAT_DN, format!("{}", params.sat_dn));
    header.set(CARD_JUMP_K, format!("{}", params.jump_k));
    header.set(CARD_REF_CORRECTION, ref_correction_label(product).to_string());
    header.set(CARD_REF_WINDOW, params.ref_window_rows.unwrap_or(0).to_string());
    if let Some(layout) = &product.layout {
        header.set(CARD_UNCAL_FAST_LEN, layout.n_fast.to_string());
    }
    for card in qslope_history_cards(info, params, product) {
        header.cards.push((CARD_HISTORY.to_string(), card));
    }
    header
}

pub fn qslope_history_cards(info: &RampInfo, params: &QuickSlopeParams, product: &QuickSlopeProduct) -> Vec<String> {
    let correction = match (&product.layout, product.ref_corrected, params.ref_window_rows) {
        (Some(_), true, Some(window)) => format!("IRS2 reference offset subtracted per group and column, {window}-row window"),
        (Some(_), true, None) => "IRS2 reference offset subtracted per group and column, whole-band median".to_string(),
        (Some(_), false, _) => "IRS2 rows stripped, no reference-offset correction (ref_correction=off)".to_string(),
        (None, _, _) => "no IRS2 layout: no reference-offset correction, frame size kept".to_string(),
    };
    vec![
        "AstroBurst quick slope: unweighted OLS or median of group differences".to_string(),
        format!("per pixel over {} groups of integration {}", info.ngroups, product.integration),
        format!("divided by TGROUP = {:.6} s", product.tgroup_s),
        correction,
        format!("saturation: fixed threshold {:.0} DN", params.sat_dn),
        format!("jumps: |d - med| > k * max(pixel MAD, floor, band scale), k = {}", params.jump_k),
        "no superbias, linearity, dark, refpix FFT, weighted fit or variances".to_string(),
        "not a calwebb_detector1 rate product".to_string(),
    ]
}

pub fn write_qslope(path: &str, primary: &HduHeader, product: &QuickSlopeProduct) -> Result<()> {
    let mut sci = HduHeader::empty();
    sci.set(CARD_BUNIT, BUNIT_RATE.to_string());
    let mut ngood = HduHeader::empty();
    ngood.set(CARD_BUNIT, BUNIT_GROUPS.to_string());
    let mut dq = HduHeader::empty();
    dq.set(CARD_NOTE, DQ_NOTE.to_string());
    let mut noise = HduHeader::empty();
    noise.set(CARD_BUNIT, BUNIT_RATE.to_string());
    noise.set(CARD_NOTE, NOISE_NOTE.to_string());
    let hdus = [
        ImageHdu { data: HduData::F32(&product.sci), header: Some(&sci), extname: EXT_SCI, extver: 1 },
        ImageHdu { data: HduData::I32(&product.ngood), header: Some(&ngood), extname: EXT_NGOOD, extver: 1 },
        ImageHdu { data: HduData::U32(&product.dq), header: Some(&dq), extname: EXT_DQ, extver: 1 },
        ImageHdu { data: HduData::F32(&product.noise), header: Some(&noise), extname: EXT_NOISE, extver: 1 },
    ];
    write_mef_images(path, Some(primary), &hdus)
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use ndarray::Array2;

    use super::*;
    use crate::core::imaging::region::test_support::make_header;
    use crate::core::ramp::info::{Irs2Header, RampKind, TgroupSource};
    use crate::core::ramp::irs2::{Irs2Layout, Irs2Resolution, IRS2_ALREADY_STRIPPED_NOTE};
    use crate::core::ramp::quick_slope::{FlagCounts, RefCorrection, DQ_JUMP_DET, DQ_SATURATED};
    use crate::infra::fits::reader::{extract_header_by_index, extract_image_mmap_by_index, extract_int_plane_by_index, list_extensions, read_primary_header};

    const OWNER_NAME: &str = "jw01266005001_02103_00001_nrs1_uncal.fits";

    fn info(ngroups: usize, tgroup_s: f64, frame_height: usize) -> RampInfo {
        RampInfo {
            kind: RampKind::JwstGroups,
            nints: 1,
            ngroups,
            nframes: 1,
            groupgap: 0,
            tframe_s: Some(tgroup_s),
            tgroup_s: Some(tgroup_s),
            tgroup_source: TgroupSource::Tgroup,
            group_times_s: None,
            readpatt: Some("NRSIRS2RAPID".into()),
            instrument: Some("NIRSPEC".into()),
            detector: Some("NRS2".into()),
            exp_type: Some("NRS_IFU".into()),
            datamodl: Some("Level1bModel".into()),
            irs2: Some(Irs2Header { nrs_norm: 16, nrs_ref: 4, noutputs: 5, fast_axis: Some(-2), slow_axis: Some(-1) }),
            frame_width: 2048,
            frame_height,
        }
    }

    fn sample_product(rows: usize, cols: usize, layout: Option<Irs2Layout>, ref_corrected: bool, integration: usize) -> QuickSlopeProduct {
        QuickSlopeProduct {
            sci: Array2::from_shape_fn((rows, cols), |(y, x)| (y * cols + x) as f32 * 0.5),
            ngood: Array2::from_elem((rows, cols), 10),
            dq: Array2::zeros((rows, cols)),
            noise: Array2::from_elem((rows, cols), 0.07),
            counts: FlagCounts::default(),
            layout,
            ref_corrected,
            band_scales_dn: vec![14.0; 4],
            tgroup_s: 14.589,
            integration,
            warnings: Vec::new(),
        }
    }

    fn uncal_primary() -> HduHeader {
        make_header(&[
            ("TELESCOP", "JWST"),
            ("INSTRUME", "NIRSPEC"),
            ("DETECTOR", "NRS2"),
            ("DATAMODL", "Level1bModel"),
            ("FILENAME", "jw01266005001_02103_00001_nrs2_uncal.fits"),
            ("NEXTEND", "3"),
            ("READPATT", "NRSIRS2RAPID"),
            ("NINTS", "1"),
            ("NGROUPS", "10"),
            ("TFRAME", "14.58889"),
            ("TGROUP", "14.589"),
            ("NRS_NORM", "16"),
            ("NRS_REF", "4"),
            ("NOUTPUTS", "5"),
            ("FASTAXIS", "-2"),
            ("SLOWAXIS", "-1"),
            ("CAL_VER", "1.20.2"),
            ("CRDS_CTX", "jwst_1464.pmap"),
            ("S_RAMP", "COMPLETE"),
            ("S_DQINIT", "COMPLETE"),
            ("R_MASK", "crds://jwst_nirspec_mask_0001.fits"),
            ("DATE", "2026-10-03T00:00:00"),
        ])
    }

    fn history_lengths(info: &RampInfo, params: &QuickSlopeParams, product: &QuickSlopeProduct) -> Vec<(usize, String)> {
        qslope_history_cards(info, params, product).into_iter().map(|c| (c.len(), c)).collect()
    }

    #[test]
    fn the_output_name_strips_the_uncal_suffix_and_names_the_integration_only_above_one_integration() {
        assert_eq!(qslope_stem(&format!("C:/data/{OWNER_NAME}")), "jw01266005001_02103_00001_nrs1");
        assert_eq!(qslope_stem("C:/data/JW_NRS2_UNCAL.FITS"), "JW_NRS2");
        assert_eq!(qslope_stem("other.fits"), "other");
        assert_eq!(qslope_stem("ramp_uncal.fits.gz"), "ramp");
        assert_eq!(qslope_output_path(OWNER_NAME, "C:/out", 1, 0), "C:/out/jw01266005001_02103_00001_nrs1_qslope.fits");
        assert_eq!(qslope_output_path(OWNER_NAME, "C:/out/", 3, 1), "C:/out/jw01266005001_02103_00001_nrs1_int002_qslope.fits");
        assert_eq!(qslope_output_path("other.fits", "out", 1, 0), "out/other_qslope.fits");
        assert_eq!(qslope_output_path("other.fits", "out", 1000, 999), "out/other_int1000_qslope.fits");
    }

    #[test]
    fn the_sibling_rate_is_reported_only_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let uncal = dir.path().join(OWNER_NAME);
        std::fs::write(&uncal, b"").unwrap();
        assert_eq!(sibling_rate_path(uncal.to_str().unwrap()), None);
        let rate = dir.path().join("jw01266005001_02103_00001_nrs1_rate.fits");
        std::fs::write(&rate, b"").unwrap();
        let sibling = sibling_rate_path(uncal.to_str().unwrap()).expect("the rate exists now");
        assert_eq!(Path::new(&sibling), rate.as_path());
        let forward = uncal.to_str().unwrap().replace('\\', "/");
        let forward_sibling = sibling_rate_path(&forward).unwrap();
        assert!(!forward_sibling.contains('\\'), "{forward_sibling}");
        assert!(forward_sibling.ends_with("/jw01266005001_02103_00001_nrs1_rate.fits"));
        let qslope = dir.path().join("jw01266005001_02103_00001_nrs1_qslope.fits");
        std::fs::write(&qslope, b"").unwrap();
        assert_eq!(sibling_rate_path(qslope.to_str().unwrap()), None);
    }

    #[test]
    fn every_history_card_fits_in_seventy_two_bytes_with_worst_case_values() {
        let worst_info = info(65535, 10.73677, 3200);
        let worst_params = QuickSlopeParams { sat_dn: 65535.0, jump_k: 10.0, ref_window_rows: Some(9999), ..Default::default() };
        let layout = Irs2Layout::new(3200, 16, 4, 5, true).unwrap();
        let states = [
            ("windowed", Some(layout.clone()), true, worst_params.clone()),
            ("whole band", Some(layout.clone()), true, QuickSlopeParams { ref_window_rows: None, ..worst_params.clone() }),
            ("off", Some(layout), false, QuickSlopeParams { ref_correction: RefCorrection::Off, ..worst_params.clone() }),
            ("no layout", None, false, worst_params.clone()),
        ];
        for (label, layout, corrected, params) in states {
            let mut product = sample_product(2, 2, layout, corrected, 65535);
            product.tgroup_s = 10.73677;
            let cards = history_lengths(&worst_info, &params, &product);
            assert!(cards.len() >= 8, "{label}: {cards:?}");
            for (len, card) in &cards {
                assert!(*len <= HISTORY_MAX_BYTES, "{label}: {len} bytes: {card}");
                assert!(card.is_ascii());
            }
            let joined = cards.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>().join("\n");
            assert!(joined.contains("quick slope") && joined.contains("65535 groups of integration 65535"));
            assert!(joined.contains("not a calwebb_detector1 rate product"));
            assert!(joined.contains("TGROUP = 10.736770 s"));
            assert!(!joined.contains('%'), "no accuracy figure goes into the file: {joined}");
        }
        let windowed = history_lengths(&worst_info, &worst_params, &sample_product(2, 2, Some(Irs2Layout::new(3200, 16, 4, 5, true).unwrap()), true, 0));
        assert!(windowed.iter().any(|(_, c)| c.contains("9999-row window")));
    }

    #[test]
    fn the_qslope_file_has_four_extensions_with_units_provenance_and_no_pipeline_claims() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jw01266005001_02103_00001_nrs2_qslope.fits");
        let text = path.to_str().unwrap();
        let layout = Irs2Layout::new(3200, 16, 4, 5, true).unwrap();
        let mut product = sample_product(3, 4, Some(layout), true, 0);
        product.dq[[1, 2]] = DQ_JUMP_DET | DQ_SATURATED;
        product.ngood[[1, 2]] = 4;
        let info = info(10, 14.589, 3200);
        let params = QuickSlopeParams::default();
        let primary = qslope_primary_header(&uncal_primary(), &info, &params, &product, "jw01266005001_02103_00001_nrs2_qslope.fits");
        write_qslope(text, &primary, &product).unwrap();

        let file = File::open(text).unwrap();
        let hdus = list_extensions(&file).unwrap();
        let names: Vec<String> = hdus.iter().skip(1).map(|h| h.extname.clone().unwrap_or_default().trim().to_string()).collect();
        assert_eq!(names, vec!["SCI", "NGOOD", "DQ", "NOISE"]);
        assert_eq!(hdus[0].naxis, 0);
        let sci = extract_image_mmap_by_index(&file, 1).unwrap();
        assert_eq!(sci.header.get("BUNIT"), Some("DN/s"));
        assert_eq!(sci.image[[2, 3]], product.sci[[2, 3]]);
        let ngood = extract_header_by_index(&file, 2).unwrap();
        assert_eq!((ngood.get("BUNIT"), ngood.get_i64("BITPIX")), (Some("groups"), Some(32)));
        let dq_header = extract_header_by_index(&file, 3).unwrap();
        assert_eq!((dq_header.get_i64("BITPIX"), dq_header.get_i64("BZERO")), (Some(32), Some(2147483648)));
        assert_eq!(dq_header.get("ABNOTE"), Some(DQ_NOTE));
        let dq = extract_int_plane_by_index(&file, 3).unwrap();
        assert_eq!(dq.bits[[1, 2]], 6);
        assert_eq!(dq.bits[[0, 0]], 0);
        let ngood_plane = extract_int_plane_by_index(&file, 2).unwrap();
        assert_eq!(ngood_plane.bits[[1, 2]] as i32, 4);
        let noise = extract_header_by_index(&file, 4).unwrap();
        assert_eq!((noise.get("BUNIT"), noise.get("ABNOTE")), (Some("DN/s"), Some(NOISE_NOTE)));

        let written = read_primary_header(text).unwrap();
        assert_eq!(written.get("ABPROC"), Some("qslope"));
        assert_eq!(written.get("ABREFCOR"), Some("amplifier"));
        assert_eq!(written.get_i64("ABNFAST"), Some(3200));
        assert_eq!(written.get_i64("ABREFWIN"), Some(200));
        assert_eq!(written.get_i64("ABNGROUP"), Some(10));
        assert_eq!(written.get_i64("ABINTEG"), Some(0));
        assert_eq!(written.get_f64("ABTGROUP"), Some(14.589));
        assert_eq!(written.get_f64("TGROUP"), Some(14.589));
        assert_eq!(written.get("DETECTOR"), Some("NRS2"));
        assert_eq!(written.get_i64("NRS_NORM"), Some(16));
        assert_eq!(written.get_i64("NRS_REF"), Some(4));
        assert_eq!(written.get_i64("FASTAXIS"), Some(-2));
        assert_eq!(written.get("TELESCOP"), Some("JWST"));
        assert_eq!(written.get("READPATT"), Some("NRSIRS2RAPID"));
        assert_eq!(written.get("FILENAME"), Some("jw01266005001_02103_00001_nrs2_qslope.fits"));
        assert_eq!(written.get("ABQSVER"), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(written.get_f64("ABSATDN"), Some(62258.0));
        assert_eq!(written.get_f64("ABJUMPK"), Some(5.0));
        let history: Vec<&str> = written.cards.iter().filter(|(k, _)| k.trim() == "HISTORY").map(|(_, v)| v.as_str()).collect();
        assert!(history.iter().any(|h| h.contains("quick slope")), "{history:?}");
        assert!(history.iter().all(|h| h.len() <= HISTORY_MAX_BYTES));
        assert!(history.len() >= 8, "{history:?}");
        for forbidden in ["CAL_VER", "S_RAMP", "DATAMODL", "S_DQINIT", "R_MASK", "CRDS_CTX", "NEXTEND", "ABDQTAB", "DATE", "BUNIT"] {
            assert_eq!(written.get(forbidden), None, "{forbidden} must not be in the product primary");
        }
    }

    #[test]
    fn the_layout_round_trips_through_the_product_primary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip_qslope.fits");
        let text = path.to_str().unwrap();
        let layout = Irs2Layout::new(3200, 16, 4, 5, true).unwrap();
        let product = sample_product(2, 2, Some(layout.clone()), true, 0);
        let primary = qslope_primary_header(&uncal_primary(), &info(10, 14.589, 3200), &QuickSlopeParams::default(), &product, "roundtrip_qslope.fits");
        write_qslope(text, &primary, &product).unwrap();
        let rebuilt = Irs2Layout::from_product_header(&read_primary_header(text).unwrap()).unwrap();
        assert_eq!(rebuilt, Some(layout));

        let plain = dir.path().join("plain_qslope.fits");
        let plain_text = plain.to_str().unwrap();
        let no_layout = sample_product(2, 2, None, false, 0);
        let mut plain_primary = make_header(&[("TELESCOP", "JWST"), ("DETECTOR", "NRS1"), ("TGROUP", "10.0")]);
        plain_primary.set("NGROUPS", "5".to_string());
        let mut plain_info = info(5, 10.0, 64);
        plain_info.irs2 = None;
        let primary = qslope_primary_header(&plain_primary, &plain_info, &QuickSlopeParams::default(), &no_layout, "plain_qslope.fits");
        write_qslope(plain_text, &primary, &no_layout).unwrap();
        let written = read_primary_header(plain_text).unwrap();
        assert_eq!(written.get("ABREFCOR"), Some("none"));
        assert_eq!(written.get_i64("ABNFAST"), None);
        assert_eq!(Irs2Layout::from_product_header(&written).unwrap(), None);
    }

    #[test]
    fn an_already_stripped_product_keeps_the_irs2_cards_but_no_fast_axis_length_so_no_layout_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stripped_qslope.fits");
        let text = path.to_str().unwrap();
        let stripped_info = info(10, 14.589, 2048);
        assert_eq!(Irs2Layout::from_info(&stripped_info).unwrap(), Irs2Resolution::AlreadyStripped);
        let mut product = sample_product(3, 4, None, false, 0);
        product.warnings.push(IRS2_ALREADY_STRIPPED_NOTE.to_string());
        let primary = qslope_primary_header(&uncal_primary(), &stripped_info, &QuickSlopeParams::default(), &product, "stripped_qslope.fits");
        assert_eq!(primary.get(CARD_UNCAL_FAST_LEN), None);
        write_qslope(text, &primary, &product).unwrap();
        let written = read_primary_header(text).unwrap();
        assert_eq!(written.get("ABREFCOR"), Some("none"));
        assert_eq!(written.get_i64("ABNFAST"), None);
        assert_eq!((written.get_i64("NRS_NORM"), written.get_i64("NRS_REF"), written.get_i64("NOUTPUTS")), (Some(16), Some(4), Some(5)));
        assert_eq!(written.get("DETECTOR"), Some("NRS2"));
        assert_eq!(Irs2Layout::from_product_header(&written).unwrap(), None);
    }
}
