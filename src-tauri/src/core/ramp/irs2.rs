use anyhow::{bail, Result};

use crate::core::ramp::info::RampInfo;
use crate::types::header::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Irs2Sample {
    RefOutput,
    Reference,
    Science,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Irs2Layout {
    pub n_fast: usize,
    pub nrs_norm: usize,
    pub nrs_ref: usize,
    pub noutputs: usize,
    pub reversed: bool,
    kinds: Vec<Irs2Sample>,
    science_before: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Irs2Resolution {
    Layout(Irs2Layout),
    NoIrs2Cards,
    AlreadyStripped,
}

pub const IRS2_ALREADY_STRIPPED_NOTE: &str =
    "IRS2 cards present but the frame holds only the science rows: already reference-corrected and stripped";
pub const IRS2_SUPPORTED_OUTPUTS: usize = 5;
pub const CARD_UNCAL_FAST_LEN: &str = "ABNFAST";
pub const DETECTOR_REVERSED: &str = "NRS2";
pub const IRS2_FAST_AXIS_MAGNITUDE: i64 = 2;

pub fn make_irs2_mask(n_fast: usize, nrs_norm: usize, nrs_ref: usize, noutputs: usize) -> Result<Vec<bool>> {
    if noutputs < 2 {
        bail!("IRS2 needs a reference output and at least one science output, got NOUTPUTS = {noutputs}");
    }
    if nrs_norm == 0 || nrs_ref == 0 {
        bail!("IRS2 needs NRS_NORM > 0 and NRS_REF > 0, got NRS_NORM = {nrs_norm}, NRS_REF = {nrs_ref}");
    }
    if nrs_norm % 2 != 0 {
        bail!("NRS_NORM = {nrs_norm} is odd: jwst's symmetric IRS2 mask splits the normal samples into two equal halves");
    }
    if n_fast % noutputs != 0 {
        bail!("fast axis of {n_fast} samples does not split into {noutputs} equal outputs");
    }
    let output_len = n_fast / noutputs;
    let period = nrs_norm + nrs_ref;
    if output_len % period != 0 {
        bail!("output of {output_len} samples is not a whole number of {period}-sample IRS2 blocks (NRS_NORM {nrs_norm} + NRS_REF {nrs_ref})");
    }
    let half = nrs_norm / 2;
    Ok((0..n_fast)
        .map(|y| {
            y >= output_len && {
                let p = (y - output_len) % period;
                p < half || p >= half + nrs_ref
            }
        })
        .collect())
}

fn card_usize(header: &HduHeader, key: &str) -> Option<usize> {
    header.get_i64(key).and_then(|v| usize::try_from(v).ok())
}

fn card_text(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn check_fast_axis(fast_axis: Option<i64>, reversed: bool, detector: &str) -> Result<()> {
    let Some(fast_axis) = fast_axis else { return Ok(()) };
    if fast_axis.abs() != IRS2_FAST_AXIS_MAGNITUDE {
        bail!("IRS2 interleaving along NAXIS1 is not supported: the row-band design needs FASTAXIS = +/-2, got FASTAXIS = {fast_axis}");
    }
    if (fast_axis < 0) != reversed {
        bail!("DETECTOR and FASTAXIS disagree about the readout direction (DETECTOR {detector}, FASTAXIS {fast_axis})");
    }
    Ok(())
}

fn stripped_science_rows(output_len: usize, nrs_norm: usize, nrs_ref: usize, noutputs: usize) -> usize {
    (noutputs - 1) * output_len / (nrs_norm + nrs_ref) * nrs_norm
}

fn is_stripped_height(frame_height: usize, nrs_norm: usize, nrs_ref: usize, noutputs: usize) -> bool {
    let period = nrs_norm + nrs_ref;
    let science_per_output_period = (noutputs - 1) * nrs_norm;
    let scaled = frame_height * period;
    scaled % science_per_output_period == 0 && (scaled / science_per_output_period) % period == 0
}

fn candidate_heights(frame_height: usize, nrs_norm: usize, nrs_ref: usize, noutputs: usize) -> (usize, usize) {
    let period = nrs_norm + nrs_ref;
    let round_to_blocks = |len: f64| ((len / period as f64).round().max(1.0) as usize) * period;
    let from_unstripped = round_to_blocks(frame_height as f64 / noutputs as f64);
    let from_stripped = round_to_blocks(frame_height as f64 / ((noutputs - 1) * nrs_norm) as f64 * period as f64);
    let heights = |output_len: usize| (noutputs * output_len, stripped_science_rows(output_len, nrs_norm, nrs_ref, noutputs));
    let (unstripped_u, stripped_u) = heights(from_unstripped);
    let (unstripped_s, stripped_s) = heights(from_stripped);
    if unstripped_u.abs_diff(frame_height) <= stripped_s.abs_diff(frame_height) {
        (unstripped_u, stripped_u)
    } else {
        (unstripped_s, stripped_s)
    }
}

impl Irs2Layout {
    pub fn new(n_fast: usize, nrs_norm: usize, nrs_ref: usize, noutputs: usize, reversed: bool) -> Result<Self> {
        let mask = make_irs2_mask(n_fast, nrs_norm, nrs_ref, noutputs)?;
        if noutputs != IRS2_SUPPORTED_OUTPUTS {
            bail!("NOUTPUTS = {noutputs}: jwst's make_irs2_mask assumes one reference output and four science outputs");
        }
        let output_len = n_fast / noutputs;
        let kinds: Vec<Irs2Sample> = (0..n_fast)
            .map(|y| {
                let d = if reversed { n_fast - 1 - y } else { y };
                if d < output_len {
                    Irs2Sample::RefOutput
                } else if mask[d] {
                    Irs2Sample::Science
                } else {
                    Irs2Sample::Reference
                }
            })
            .collect();
        let mut science_before = Vec::with_capacity(n_fast + 1);
        let mut count = 0;
        for kind in &kinds {
            science_before.push(count);
            if *kind == Irs2Sample::Science {
                count += 1;
            }
        }
        science_before.push(count);
        Ok(Self { n_fast, nrs_norm, nrs_ref, noutputs, reversed, kinds, science_before })
    }

    pub fn from_info(info: &RampInfo) -> Result<Irs2Resolution> {
        let Some(irs2) = &info.irs2 else {
            return Ok(Irs2Resolution::NoIrs2Cards);
        };
        if irs2.noutputs != IRS2_SUPPORTED_OUTPUTS {
            bail!("NOUTPUTS = {}: jwst's make_irs2_mask assumes one reference output and four science outputs", irs2.noutputs);
        }
        let Some(detector) = info.detector.as_deref().map(|d| d.trim().to_ascii_uppercase()).filter(|d| !d.is_empty()) else {
            bail!("IRS2 orientation needs DETECTOR (NRS1 keeps the detector frame, NRS2 reverses the fast axis)");
        };
        let reversed = detector == DETECTOR_REVERSED;
        check_fast_axis(irs2.fast_axis, reversed, &detector)?;
        match Self::new(info.frame_height, irs2.nrs_norm, irs2.nrs_ref, irs2.noutputs, reversed) {
            Ok(layout) => Ok(Irs2Resolution::Layout(layout)),
            Err(layout_error) => {
                if is_stripped_height(info.frame_height, irs2.nrs_norm, irs2.nrs_ref, irs2.noutputs) {
                    return Ok(Irs2Resolution::AlreadyStripped);
                }
                let (unstripped, stripped) = candidate_heights(info.frame_height, irs2.nrs_norm, irs2.nrs_ref, irs2.noutputs);
                bail!(
                    "frame height {} matches neither the unstripped IRS2 frame ({unstripped} rows) nor the stripped science frame ({stripped} rows) for NRS_NORM {} / NRS_REF {} / NOUTPUTS {}: {layout_error:#}",
                    info.frame_height,
                    irs2.nrs_norm,
                    irs2.nrs_ref,
                    irs2.noutputs
                );
            }
        }
    }

    pub fn from_product_header(primary: &HduHeader) -> Result<Option<Self>> {
        let (Some(n_fast), Some(nrs_norm)) = (card_usize(primary, CARD_UNCAL_FAST_LEN), card_usize(primary, "NRS_NORM")) else {
            return Ok(None);
        };
        let Some(nrs_ref) = card_usize(primary, "NRS_REF") else {
            bail!("product header carries {CARD_UNCAL_FAST_LEN} and NRS_NORM but no NRS_REF: the IRS2 layout cannot be rebuilt");
        };
        let noutputs = card_usize(primary, "NOUTPUTS").unwrap_or(IRS2_SUPPORTED_OUTPUTS);
        let Some(detector) = card_text(primary, "DETECTOR").map(|d| d.to_ascii_uppercase()) else {
            bail!("product header carries {CARD_UNCAL_FAST_LEN} but no DETECTOR: the IRS2 orientation cannot be rebuilt");
        };
        let reversed = detector == DETECTOR_REVERSED;
        check_fast_axis(primary.get_i64("FASTAXIS"), reversed, &detector)?;
        Self::new(n_fast, nrs_norm, nrs_ref, noutputs, reversed).map(Some)
    }

    pub fn block_rows(&self) -> usize {
        self.nrs_norm + self.nrs_ref
    }

    pub fn output_len(&self) -> usize {
        self.n_fast / self.noutputs
    }

    pub fn science_outputs(&self) -> usize {
        self.noutputs - 1
    }

    pub fn science_rows(&self) -> usize {
        self.science_before[self.n_fast]
    }

    fn detector_index(&self, y: usize) -> usize {
        if self.reversed {
            self.n_fast - 1 - y
        } else {
            y
        }
    }

    pub fn sample_kind(&self, y: usize) -> Irs2Sample {
        self.kinds[y]
    }

    pub fn science_row_of(&self, y: usize) -> Option<usize> {
        (self.kinds[y] == Irs2Sample::Science).then(|| self.science_before[y])
    }

    pub fn amplifier_of(&self, y: usize) -> Option<usize> {
        (self.kinds[y] != Irs2Sample::RefOutput).then(|| self.detector_index(y) / self.output_len() - 1)
    }

    pub fn amplifier_rows(&self, amp: usize) -> std::ops::Range<usize> {
        let output_len = self.output_len();
        if self.reversed {
            self.n_fast - output_len * (amp + 2)..self.n_fast - output_len * (amp + 1)
        } else {
            output_len * (amp + 1)..output_len * (amp + 2)
        }
    }

    pub fn science_rows_of_amplifier(&self, amp: usize) -> (usize, usize) {
        let rows = self.amplifier_rows(amp);
        self.uncal_rows_to_science_rows(rows.start, rows.end)
            .expect("every science output holds science rows")
    }

    pub fn science_mask(&self) -> Vec<bool> {
        self.kinds.iter().map(|k| *k == Irs2Sample::Science).collect()
    }

    pub fn uncal_rows_to_science_rows(&self, y0: usize, y1: usize) -> Option<(usize, usize)> {
        let y0 = y0.min(self.n_fast);
        let y1 = y1.min(self.n_fast);
        if y1 <= y0 {
            return None;
        }
        let (s0, s1) = (self.science_before[y0], self.science_before[y1]);
        (s1 > s0).then_some((s0, s1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::region::test_support::make_header;
    use crate::core::ramp::info::{Irs2Header, RampInfo, RampKind, TgroupSource};

    const OWNER_FAST: usize = 3200;
    const OUTPUT: usize = 640;
    const BLOCK: usize = 20;

    fn owner_mask_value(y: usize) -> bool {
        y >= OUTPUT && !(8..12).contains(&((y - OUTPUT) % BLOCK))
    }

    fn owner_layout(reversed: bool) -> Irs2Layout {
        Irs2Layout::new(OWNER_FAST, 16, 4, 5, reversed).unwrap()
    }

    fn info(detector: Option<&str>, fast_axis: Option<i64>, frame_height: usize, noutputs: usize, irs2: bool) -> RampInfo {
        RampInfo {
            kind: RampKind::JwstGroups,
            nints: 1,
            ngroups: 10,
            nframes: 1,
            groupgap: 0,
            tframe_s: Some(14.58889),
            tgroup_s: Some(14.589),
            tgroup_source: TgroupSource::Tgroup,
            group_times_s: None,
            readpatt: Some("NRSIRS2RAPID".into()),
            instrument: Some("NIRSPEC".into()),
            detector: detector.map(str::to_string),
            exp_type: None,
            datamodl: Some("Level1bModel".into()),
            irs2: irs2.then_some(Irs2Header { nrs_norm: 16, nrs_ref: 4, noutputs, fast_axis, slow_axis: fast_axis.map(|f| f.signum()) }),
            frame_width: 2048,
            frame_height,
        }
    }

    #[test]
    fn the_mask_for_sixteen_by_four_has_the_reference_output_first_and_reference_samples_at_offsets_eight_to_eleven() {
        let mask = make_irs2_mask(OWNER_FAST, 16, 4, 5).unwrap();
        assert_eq!(mask.len(), OWNER_FAST);
        assert!(mask[..OUTPUT].iter().all(|m| !m));
        for (y, m) in mask.iter().enumerate().skip(OUTPUT) {
            assert_eq!(*m, owner_mask_value(y), "row {y}");
        }
        assert_eq!(mask.iter().filter(|m| **m).count(), 2048);
    }

    #[test]
    fn nrs2_reverses_the_mask_so_the_reference_output_is_the_last_output() {
        let layout = owner_layout(true);
        assert!((2560..OWNER_FAST).all(|y| layout.sample_kind(y) == Irs2Sample::RefOutput));
        assert!((0..2560).all(|y| layout.sample_kind(y) != Irs2Sample::RefOutput));
        for y in 0..2560 {
            let detector_index = OWNER_FAST - 1 - y;
            let expected = if owner_mask_value(detector_index) { Irs2Sample::Science } else { Irs2Sample::Reference };
            assert_eq!(layout.sample_kind(y), expected, "DMS row {y}");
        }
        let mask = layout.science_mask();
        assert_eq!(mask.iter().filter(|m| **m).count(), 2048);
        assert!(mask[0] && !mask[3199]);
    }

    #[test]
    fn science_row_of_is_dense_and_zero_based_in_increasing_dms_order() {
        let nrs1 = owner_layout(false);
        assert_eq!(nrs1.science_rows(), 2048);
        assert_eq!(nrs1.science_row_of(639), None);
        assert_eq!(nrs1.science_row_of(640), Some(0));
        assert_eq!(nrs1.science_row_of(647), Some(7));
        assert!((648..652).all(|y| nrs1.science_row_of(y).is_none()));
        assert_eq!(nrs1.science_row_of(652), Some(8));
        assert_eq!(nrs1.science_row_of(3199), Some(2047));
        let mut expected = 0;
        for y in 0..OWNER_FAST {
            if let Some(s) = nrs1.science_row_of(y) {
                assert_eq!(s, expected);
                expected += 1;
            }
        }
        let nrs2 = owner_layout(true);
        assert_eq!(nrs2.science_row_of(0), Some(0));
        assert!((2560..OWNER_FAST).all(|y| nrs2.science_row_of(y).is_none()));
        assert_eq!(nrs2.science_row_of(2559), Some(2047));
    }

    #[test]
    fn each_amplifier_band_holds_512_science_and_128_reference_rows_and_bands_are_disjoint() {
        for reversed in [false, true] {
            let layout = owner_layout(reversed);
            let mut covered = vec![false; OWNER_FAST];
            for amp in 0..layout.science_outputs() {
                let rows = layout.amplifier_rows(amp);
                assert_eq!(rows.len(), OUTPUT);
                let mut science = 0;
                let mut reference = 0;
                for y in rows {
                    assert!(!covered[y], "row {y} belongs to two amplifiers");
                    covered[y] = true;
                    assert_eq!(layout.amplifier_of(y), Some(amp));
                    match layout.sample_kind(y) {
                        Irs2Sample::Science => science += 1,
                        Irs2Sample::Reference => reference += 1,
                        Irs2Sample::RefOutput => panic!("amplifier {amp} includes reference-output row {y}"),
                    }
                }
                assert_eq!((science, reference), (512, 128), "amplifier {amp} reversed={reversed}");
            }
            let uncovered: Vec<usize> = (0..OWNER_FAST).filter(|y| !covered[*y]).collect();
            assert_eq!(uncovered.len(), OUTPUT);
            assert!(uncovered.iter().all(|y| layout.sample_kind(*y) == Irs2Sample::RefOutput && layout.amplifier_of(*y).is_none()));
        }
    }

    #[test]
    fn science_rows_of_amplifier_are_the_pinned_stripped_ranges() {
        let nrs1 = owner_layout(false);
        let nrs2 = owner_layout(true);
        for k in 0..4 {
            assert_eq!(nrs1.science_rows_of_amplifier(k), (512 * k, 512 * k + 512));
            assert_eq!(nrs2.science_rows_of_amplifier(k), (1536 - 512 * k, 2048 - 512 * k));
        }
    }

    #[test]
    fn uncal_row_windows_map_to_stripped_row_windows() {
        let nrs1 = owner_layout(false);
        let nrs2 = owner_layout(true);
        assert_eq!(nrs1.uncal_rows_to_science_rows(1300, 1500), Some((528, 688)));
        assert_eq!(nrs2.uncal_rows_to_science_rows(700, 900), Some((560, 720)));
        assert_eq!(nrs2.uncal_rows_to_science_rows(1300, 1500), Some((1040, 1200)));
        assert_eq!(nrs1.uncal_rows_to_science_rows(0, 640), None);
        assert_eq!(nrs1.uncal_rows_to_science_rows(648, 652), None);
        assert_eq!(nrs1.uncal_rows_to_science_rows(0, OWNER_FAST), Some((0, 2048)));
        assert_eq!(nrs1.uncal_rows_to_science_rows(3000, 9999), Some((1888, 2048)));
        assert_eq!(nrs1.uncal_rows_to_science_rows(900, 700), None);
    }

    #[test]
    fn a_pattern_that_does_not_tile_the_fast_axis_is_refused() {
        assert!(make_irs2_mask(3210, 16, 4, 5).is_err());
        assert!(make_irs2_mask(3201, 16, 4, 5).is_err());
        assert!(make_irs2_mask(OWNER_FAST, 15, 4, 5).is_err());
        assert!(make_irs2_mask(OWNER_FAST, 16, 0, 5).is_err());
        assert!(make_irs2_mask(OWNER_FAST, 0, 4, 5).is_err());
        assert!(make_irs2_mask(OWNER_FAST, 16, 4, 1).is_err());
        assert!(make_irs2_mask(3000, 16, 4, 5).is_ok());
        assert!(Irs2Layout::new(3210, 16, 4, 5, false).is_err());
    }

    #[test]
    fn from_info_reverses_by_detector_not_by_fastaxis() {
        let nrs2 = Irs2Layout::from_info(&info(Some("NRS2"), None, OWNER_FAST, 5, true)).unwrap();
        assert_eq!(nrs2, Irs2Resolution::Layout(owner_layout(true)));
        let nrs1 = Irs2Layout::from_info(&info(Some("NRS1"), None, OWNER_FAST, 5, true)).unwrap();
        assert_eq!(nrs1, Irs2Resolution::Layout(owner_layout(false)));
        let lower = Irs2Layout::from_info(&info(Some(" nrs2 "), Some(-2), OWNER_FAST, 5, true)).unwrap();
        assert_eq!(lower, Irs2Resolution::Layout(owner_layout(true)));
        let missing = Irs2Layout::from_info(&info(None, Some(2), OWNER_FAST, 5, true));
        assert!(missing.unwrap_err().to_string().contains("DETECTOR"));
    }

    #[test]
    fn from_info_errors_when_fastaxis_disagrees_with_detector_and_when_the_fast_axis_is_not_two() {
        let disagree = Irs2Layout::from_info(&info(Some("NRS2"), Some(2), OWNER_FAST, 5, true)).unwrap_err();
        assert!(disagree.to_string().contains("disagree"), "{disagree}");
        let disagree_nrs1 = Irs2Layout::from_info(&info(Some("NRS1"), Some(-2), OWNER_FAST, 5, true));
        assert!(disagree_nrs1.is_err());
        let along_x = Irs2Layout::from_info(&info(Some("NRS1"), Some(1), OWNER_FAST, 5, true)).unwrap_err();
        assert!(along_x.to_string().contains("FASTAXIS"), "{along_x}");
    }

    #[test]
    fn from_info_refuses_more_or_fewer_than_five_outputs() {
        let err = Irs2Layout::from_info(&info(Some("NRS1"), Some(2), OWNER_FAST, 4, true)).unwrap_err();
        assert!(err.to_string().contains("four science outputs"), "{err}");
        assert!(Irs2Layout::new(OWNER_FAST * 6 / 5, 16, 4, 6, false).is_err());
    }

    #[test]
    fn from_info_returns_none_without_irs2_cards_and_already_stripped_for_a_frame_of_science_rows_only() {
        assert_eq!(Irs2Layout::from_info(&info(Some("NRS1"), None, OWNER_FAST, 5, false)).unwrap(), Irs2Resolution::NoIrs2Cards);
        assert_eq!(Irs2Layout::from_info(&info(Some("NRS2"), Some(-2), 2048, 5, true)).unwrap(), Irs2Resolution::AlreadyStripped);
        let err = Irs2Layout::from_info(&info(Some("NRS1"), Some(2), 3210, 5, true)).unwrap_err().to_string();
        assert!(err.contains("3210") && err.contains("3200") && err.contains("2048"), "{err}");
    }

    #[test]
    fn from_product_header_rebuilds_the_layout_from_abnfast_and_the_irs2_cards() {
        let primary = make_header(&[
            ("ABNFAST", "3200"),
            ("NRS_NORM", "16"),
            ("NRS_REF", "4"),
            ("NOUTPUTS", "5"),
            ("DETECTOR", "NRS2"),
            ("FASTAXIS", "-2"),
        ]);
        assert_eq!(Irs2Layout::from_product_header(&primary).unwrap(), Some(owner_layout(true)));
        let without_fast_len = make_header(&[("NRS_NORM", "16"), ("NRS_REF", "4"), ("DETECTOR", "NRS2")]);
        assert_eq!(Irs2Layout::from_product_header(&without_fast_len).unwrap(), None);
        let without_irs2 = make_header(&[("ABNFAST", "3200"), ("DETECTOR", "NRS1")]);
        assert_eq!(Irs2Layout::from_product_header(&without_irs2).unwrap(), None);
        let defaults = make_header(&[("ABNFAST", "3200"), ("NRS_NORM", "16"), ("NRS_REF", "4"), ("DETECTOR", "NRS1")]);
        assert_eq!(Irs2Layout::from_product_header(&defaults).unwrap(), Some(owner_layout(false)));
        let contradiction = make_header(&[("ABNFAST", "3200"), ("NRS_NORM", "16"), ("NRS_REF", "4"), ("DETECTOR", "NRS1"), ("FASTAXIS", "-2")]);
        assert!(Irs2Layout::from_product_header(&contradiction).is_err());
    }
}
