use serde::Serialize;

use crate::core::cube::eager::ctype_is_spectral;
use crate::types::header::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RampKind {
    JwstGroups,
    RomanResultants,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TgroupSource {
    Tgroup,
    TframeProduct,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Irs2Header {
    pub nrs_norm: usize,
    pub nrs_ref: usize,
    pub noutputs: usize,
    pub fast_axis: Option<i64>,
    pub slow_axis: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RampInfo {
    pub kind: RampKind,
    pub nints: usize,
    pub ngroups: usize,
    pub nframes: usize,
    pub groupgap: usize,
    pub tframe_s: Option<f64>,
    pub tgroup_s: Option<f64>,
    pub tgroup_source: TgroupSource,
    pub group_times_s: Option<Vec<f64>>,
    pub readpatt: Option<String>,
    pub instrument: Option<String>,
    pub detector: Option<String>,
    pub exp_type: Option<String>,
    pub datamodl: Option<String>,
    pub irs2: Option<Irs2Header>,
    pub frame_width: usize,
    pub frame_height: usize,
}

pub const RAMP_DATAMODELS: [&str; 2] = ["Level1bModel", "RampModel"];
pub const CUBE_DATAMODELS: [&str; 2] = ["CubeModel", "IFUCubeModel"];
pub const ROMAN_INSTRUMENT: &str = "WFI";
pub const DEFAULT_IRS2_OUTPUTS: usize = 5;

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn card_usize(header: &HduHeader, key: &str) -> Option<usize> {
    header.get_i64(key).and_then(|v| usize::try_from(v).ok())
}

fn positive_seconds(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|t| t.is_finite() && *t > 0.0)
}

fn datamodel_in(datamodl: Option<&str>, models: &[&str]) -> bool {
    datamodl.is_some_and(|d| models.contains(&d))
}

fn is_ramp_header(header: &HduHeader, naxis: i64, naxis3: usize, datamodl: Option<&str>) -> bool {
    let has_group_cards = header.get("NGROUPS").is_some() || header.get("NINTS").is_some();
    if naxis == 4 && has_group_cards {
        return true;
    }
    if datamodel_in(datamodl, &RAMP_DATAMODELS) {
        return true;
    }
    naxis == 3
        && naxis3 > 0
        && card_usize(header, "NGROUPS") == Some(naxis3)
        && card_usize(header, "NINTS").is_none_or(|n| n == 1)
        && !ctype_is_spectral(header)
        && !datamodel_in(datamodl, &CUBE_DATAMODELS)
}

fn irs2_header(header: &HduHeader) -> Option<Irs2Header> {
    let nrs_norm = card_usize(header, "NRS_NORM").filter(|n| *n > 0)?;
    let nrs_ref = card_usize(header, "NRS_REF").filter(|n| *n > 0)?;
    Some(Irs2Header {
        nrs_norm,
        nrs_ref,
        noutputs: card_usize(header, "NOUTPUTS").filter(|n| *n > 0).unwrap_or(DEFAULT_IRS2_OUTPUTS),
        fast_axis: header.get_i64("FASTAXIS"),
        slow_axis: header.get_i64("SLOWAXIS"),
    })
}

fn uniform_group_times(ngroups: usize, tgroup_s: Option<f64>) -> Option<Vec<f64>> {
    tgroup_s.map(|t| (0..ngroups).map(|g| g as f64 * t).collect())
}

pub fn group_time_seconds(header: &HduHeader) -> (Option<f64>, TgroupSource) {
    if let Some(tgroup) = positive_seconds(header, "TGROUP") {
        return (Some(tgroup), TgroupSource::Tgroup);
    }
    if let Some(tframe) = positive_seconds(header, "TFRAME") {
        let nframes = card_usize(header, "NFRAMES").unwrap_or(1);
        let groupgap = card_usize(header, "GROUPGAP").unwrap_or(0);
        return (Some(tframe * (nframes + groupgap) as f64), TgroupSource::TframeProduct);
    }
    (None, TgroupSource::None)
}

pub fn ramp_info(header: &HduHeader) -> Option<RampInfo> {
    let naxis = header.get_i64("NAXIS").unwrap_or(0);
    let naxis3 = card_usize(header, "NAXIS3").unwrap_or(0);
    let naxis4 = card_usize(header, "NAXIS4");
    let datamodl = card_text(header, "DATAMODL");
    if !is_ramp_header(header, naxis, naxis3, datamodl.as_deref()) {
        return None;
    }
    let ngroups = card_usize(header, "NGROUPS").unwrap_or(naxis3);
    let nints = card_usize(header, "NINTS").or(naxis4).unwrap_or(1);
    let declared = ngroups.checked_mul(nints)?;
    let stored = naxis3.checked_mul(naxis4.unwrap_or(1).max(1))?;
    if ngroups == 0 || nints == 0 || declared != stored {
        return None;
    }
    let (tgroup_s, tgroup_source) = group_time_seconds(header);
    Some(RampInfo {
        kind: RampKind::JwstGroups,
        nints,
        ngroups,
        nframes: card_usize(header, "NFRAMES").unwrap_or(1),
        groupgap: card_usize(header, "GROUPGAP").unwrap_or(0),
        tframe_s: header.get_f64("TFRAME"),
        tgroup_s,
        tgroup_source,
        group_times_s: uniform_group_times(ngroups, tgroup_s),
        readpatt: card_text(header, "READPATT"),
        instrument: card_text(header, "INSTRUME"),
        detector: card_text(header, "DETECTOR"),
        exp_type: card_text(header, "EXP_TYPE"),
        datamodl,
        irs2: irs2_header(header),
        frame_width: card_usize(header, "NAXIS1").unwrap_or(0),
        frame_height: card_usize(header, "NAXIS2").unwrap_or(0),
    })
}

fn resultant_times(frame_time_s: Option<f64>, read_pattern: Option<&[Vec<usize>]>, ngroups: usize) -> Option<Vec<f64>> {
    let frame_time = frame_time_s?;
    let pattern = read_pattern?;
    if pattern.len() != ngroups || pattern.iter().any(|reads| reads.is_empty()) {
        return None;
    }
    Some(
        pattern
            .iter()
            .map(|reads| frame_time * reads.iter().sum::<usize>() as f64 / reads.len() as f64)
            .collect(),
    )
}

pub fn roman_ramp_info(
    shape: &[usize],
    frame_time_s: Option<f64>,
    read_pattern: Option<&[Vec<usize>]>,
    nresultants: Option<usize>,
    ma_table: Option<&str>,
    detector: Option<&str>,
) -> Option<RampInfo> {
    if shape.len() != 3 || shape[0] < 2 {
        return None;
    }
    let ngroups = nresultants.filter(|n| *n == shape[0]).unwrap_or(shape[0]);
    Some(RampInfo {
        kind: RampKind::RomanResultants,
        nints: 1,
        ngroups,
        nframes: 1,
        groupgap: 0,
        tframe_s: frame_time_s,
        tgroup_s: None,
        tgroup_source: TgroupSource::None,
        group_times_s: resultant_times(frame_time_s, read_pattern, ngroups),
        readpatt: ma_table.map(str::to_string),
        instrument: Some(ROMAN_INSTRUMENT.to_string()),
        detector: detector.map(str::to_string),
        exp_type: None,
        datamodl: None,
        irs2: None,
        frame_width: shape[2],
        frame_height: shape[1],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::make_header;

    fn owner_cards(detector: &str, fast_axis: Option<&str>) -> Vec<(&'static str, String)> {
        let mut cards: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("TELESCOP", "JWST".into()),
            ("INSTRUME", "NIRSPEC".into()),
            ("DETECTOR", detector.to_string()),
            ("DATAMODL", "Level1bModel".into()),
            ("READPATT", "NRSIRS2RAPID".into()),
            ("EXP_TYPE", "NRS_IFU".into()),
            ("NINTS", "1".into()),
            ("NGROUPS", "10".into()),
            ("NFRAMES", "1".into()),
            ("GROUPGAP", "0".into()),
            ("TFRAME", "14.58889".into()),
            ("TGROUP", "14.589".into()),
            ("NRS_NORM", "16".into()),
            ("NRS_REF", "4".into()),
            ("NOUTPUTS", "5".into()),
            ("EXPOSURE", "1".into()),
            ("XTENSION", "IMAGE".into()),
            ("BITPIX", "16".into()),
            ("NAXIS", "4".into()),
            ("NAXIS1", "2048".into()),
            ("NAXIS2", "3200".into()),
            ("NAXIS3", "10".into()),
            ("NAXIS4", "1".into()),
            ("EXTNAME", "SCI".into()),
        ];
        if let Some(axis) = fast_axis {
            cards.push(("FASTAXIS", axis.to_string()));
            cards.push(("SLOWAXIS", if axis.starts_with('-') { "-1".into() } else { "1".into() }));
        }
        cards
    }

    fn header_of(cards: &[(&'static str, String)]) -> HduHeader {
        let pairs: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (*k, v.as_str())).collect();
        make_header(&pairs)
    }

    #[test]
    fn ramp_info_reads_groups_integrations_group_time_and_irs2_cards_from_the_merged_header() {
        let info = ramp_info(&header_of(&owner_cards("NRS2", Some("-2")))).expect("the owner's uncal is a ramp");
        assert_eq!(info.kind, RampKind::JwstGroups);
        assert_eq!((info.nints, info.ngroups, info.nframes, info.groupgap), (1, 10, 1, 0));
        assert_eq!(info.tframe_s, Some(14.58889));
        assert_eq!(info.tgroup_s, Some(14.589));
        assert_eq!(info.tgroup_source, TgroupSource::Tgroup);
        let times = info.group_times_s.as_ref().expect("group times");
        assert_eq!(times.len(), 10);
        for (g, t) in times.iter().enumerate() {
            assert_eq!(*t, g as f64 * 14.589, "group {}", g);
        }
        assert_eq!(times[0], 0.0);
        assert_eq!(info.readpatt.as_deref(), Some("NRSIRS2RAPID"));
        assert_eq!(info.instrument.as_deref(), Some("NIRSPEC"));
        assert_eq!(info.detector.as_deref(), Some("NRS2"));
        assert_eq!(info.exp_type.as_deref(), Some("NRS_IFU"));
        assert_eq!(info.datamodl.as_deref(), Some("Level1bModel"));
        let irs2 = info.irs2.as_ref().expect("IRS2 cards");
        assert_eq!((irs2.nrs_norm, irs2.nrs_ref, irs2.noutputs), (16, 4, 5));
        assert_eq!(irs2.fast_axis, Some(-2));
        assert_eq!(irs2.slow_axis, Some(-1));
        assert_eq!((info.frame_width, info.frame_height), (2048, 3200));

        let nrs1 = ramp_info(&header_of(&owner_cards("NRS1", Some("2")))).unwrap();
        assert_eq!(nrs1.detector.as_deref(), Some("NRS1"));
        assert_eq!(nrs1.irs2.as_ref().unwrap().fast_axis, Some(2));
    }

    #[test]
    fn irs2_fast_axis_is_none_when_the_card_is_absent() {
        let info = ramp_info(&header_of(&owner_cards("NRS2", None))).unwrap();
        let irs2 = info.irs2.expect("IRS2 cards without FASTAXIS still describe the interleaving");
        assert_eq!(irs2.fast_axis, None);
        assert_eq!(irs2.slow_axis, None);

        let mut cards = owner_cards("NRS1", Some("2"));
        cards.retain(|(k, _)| *k != "NRS_NORM");
        assert!(ramp_info(&header_of(&cards)).unwrap().irs2.is_none(), "NRS_REF alone is not an IRS2 layout");
        let mut cards = owner_cards("NRS1", Some("2"));
        cards.retain(|(k, _)| *k != "NOUTPUTS");
        assert_eq!(ramp_info(&header_of(&cards)).unwrap().irs2.unwrap().noutputs, 5);
    }

    #[test]
    fn group_time_falls_back_to_tframe_times_frames_plus_gap_and_never_reads_exposure() {
        let product = make_header(&[("TFRAME", "10.73677"), ("NFRAMES", "4"), ("GROUPGAP", "1"), ("EXPOSURE", "3")]);
        let (seconds, source) = group_time_seconds(&product);
        assert!((seconds.unwrap() - 53.68385).abs() < 1e-9, "{:?}", seconds);
        assert_eq!(source, TgroupSource::TframeProduct);

        let exposure_only = make_header(&[("EXPOSURE", "3"), ("EXPTIME", "100.0"), ("EFFINTTM", "50.0"), ("XPOSURE", "100.0")]);
        assert_eq!(group_time_seconds(&exposure_only), (None, TgroupSource::None));

        let mut cards = owner_cards("NRS1", Some("2"));
        cards.retain(|(k, _)| *k != "TGROUP" && *k != "TFRAME");
        cards.push(("EXPTIME", "145.89".into()));
        let info = ramp_info(&header_of(&cards)).unwrap();
        assert_eq!(info.tgroup_s, None);
        assert_eq!(info.tgroup_source, TgroupSource::None);
        assert_eq!(info.group_times_s, None);

        let zero = make_header(&[("TGROUP", "0.0"), ("TFRAME", "2.0"), ("NFRAMES", "2")]);
        assert_eq!(group_time_seconds(&zero), (Some(4.0), TgroupSource::TframeProduct));
    }

    #[test]
    fn a_cube_without_ramp_cards_is_not_a_ramp() {
        let muse = make_header(&[
            ("NAXIS", "3"),
            ("NAXIS1", "300"),
            ("NAXIS2", "300"),
            ("NAXIS3", "3681"),
            ("CTYPE3", "AWAV"),
            ("CUNIT3", "Angstrom"),
        ]);
        assert_eq!(ramp_info(&muse), None);
        assert_eq!(ramp_info(&make_header(&[])), None);
        assert_eq!(ramp_info(&make_header(&[("NAXIS", "2"), ("NAXIS1", "10"), ("NAXIS2", "10")])), None);
    }

    #[test]
    fn a_spectral_cube_with_ngroups_equal_to_naxis3_is_still_not_a_ramp() {
        let s3d = make_header(&[
            ("NAXIS", "3"),
            ("NAXIS1", "40"),
            ("NAXIS2", "40"),
            ("NAXIS3", "10"),
            ("NGROUPS", "10"),
            ("NINTS", "1"),
            ("CTYPE3", "WAVE"),
        ]);
        assert_eq!(ramp_info(&s3d), None);
        let ifu = make_header(&[
            ("NAXIS", "3"),
            ("NAXIS1", "40"),
            ("NAXIS2", "40"),
            ("NAXIS3", "10"),
            ("NGROUPS", "10"),
            ("DATAMODL", "IFUCubeModel"),
        ]);
        assert_eq!(ramp_info(&ifu), None);

        let plain = make_header(&[("NAXIS", "3"), ("NAXIS1", "40"), ("NAXIS2", "40"), ("NAXIS3", "10"), ("NGROUPS", "10")]);
        let info = ramp_info(&plain).expect("a 3D stack whose NGROUPS equals NAXIS3 and has no spectral axis is a ramp");
        assert_eq!((info.ngroups, info.nints), (10, 1));
    }

    #[test]
    fn a_rateints_integration_stack_is_not_a_ramp() {
        let rateints = make_header(&[
            ("NAXIS", "3"),
            ("NAXIS1", "2048"),
            ("NAXIS2", "2048"),
            ("NAXIS3", "5"),
            ("NINTS", "5"),
            ("NGROUPS", "10"),
            ("DATAMODL", "CubeModel"),
        ]);
        assert_eq!(ramp_info(&rateints), None);
    }

    #[test]
    fn an_inconsistent_group_count_is_not_a_ramp() {
        let header = make_header(&[
            ("NAXIS", "4"),
            ("NAXIS1", "8"),
            ("NAXIS2", "8"),
            ("NAXIS3", "10"),
            ("NAXIS4", "2"),
            ("NGROUPS", "9"),
            ("NINTS", "2"),
        ]);
        assert_eq!(ramp_info(&header), None);
        let consistent = make_header(&[
            ("NAXIS", "4"),
            ("NAXIS1", "8"),
            ("NAXIS2", "8"),
            ("NAXIS3", "10"),
            ("NAXIS4", "2"),
            ("NGROUPS", "10"),
            ("NINTS", "2"),
        ]);
        let info = ramp_info(&consistent).unwrap();
        assert_eq!((info.ngroups, info.nints), (10, 2));
        let from_axes = make_header(&[("NAXIS", "4"), ("NAXIS1", "8"), ("NAXIS2", "8"), ("NAXIS3", "10"), ("NAXIS4", "3"), ("NGROUPS", "10")]);
        assert_eq!(ramp_info(&from_axes).unwrap().nints, 3);
    }

    #[test]
    fn a_roman_resultant_cube_is_a_ramp_of_resultants() {
        let info = roman_ramp_info(&[8, 4096, 4096], Some(3.04), None, Some(8), Some("MA_TABLE_1"), Some("WFI01"))
            .expect("a [n, h, w] array is a ramp");
        assert_eq!(info.kind, RampKind::RomanResultants);
        assert_eq!((info.ngroups, info.nints, info.nframes, info.groupgap), (8, 1, 1, 0));
        assert_eq!(info.tgroup_s, None);
        assert_eq!(info.tgroup_source, TgroupSource::None);
        assert_eq!(info.tframe_s, Some(3.04));
        assert_eq!(info.group_times_s, None);
        assert_eq!(info.readpatt.as_deref(), Some("MA_TABLE_1"));
        assert_eq!(info.instrument.as_deref(), Some(ROMAN_INSTRUMENT));
        assert_eq!(info.detector.as_deref(), Some("WFI01"));
        assert!(info.irs2.is_none());
        assert_eq!((info.frame_width, info.frame_height), (4096, 4096));

        assert!(roman_ramp_info(&[4096, 4096], Some(3.04), None, None, None, None).is_none());
        assert!(roman_ramp_info(&[1, 4096, 4096], Some(3.04), None, None, None, None).is_none());
    }

    #[test]
    fn roman_resultant_times_average_the_read_pattern() {
        let pattern = vec![vec![1usize], vec![2, 3], vec![4, 5, 6, 7]];
        let info = roman_ramp_info(&[3, 8, 8], Some(3.04), Some(&pattern), Some(3), None, None).unwrap();
        let times = info.group_times_s.expect("resultant times");
        let expected = [3.04, 3.04 * 2.5, 3.04 * 5.5];
        for (t, e) in times.iter().zip(expected.iter()) {
            assert!((t - e).abs() < 1e-12, "{:?} vs {:?}", times, expected);
        }
        let short = vec![vec![1usize], vec![2, 3]];
        assert_eq!(roman_ramp_info(&[3, 8, 8], Some(3.04), Some(&short), None, None, None).unwrap().group_times_s, None);
        assert_eq!(roman_ramp_info(&[3, 8, 8], None, Some(&pattern), None, None, None).unwrap().group_times_s, None);
    }

    #[test]
    fn ramp_info_serialises_with_snake_case_enums_and_null_for_missing_cards() {
        let info = ramp_info(&header_of(&owner_cards("NRS2", None))).unwrap();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["kind"], "jwst_groups");
        assert_eq!(json["tgroup_source"], "tgroup");
        assert_eq!(json["irs2"]["fast_axis"], serde_json::Value::Null);
        assert_eq!(json["irs2"]["nrs_norm"], 16);
        let roman = serde_json::to_value(roman_ramp_info(&[3, 8, 8], None, None, None, None, None).unwrap()).unwrap();
        assert_eq!(roman["kind"], "roman_resultants");
        assert_eq!(roman["tgroup_source"], "none");
    }
}
