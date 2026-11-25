use crate::core::imaging::push_history;
use crate::core::stacking::consistency::{file_label, format_sig4, true_median};
use crate::core::stacking::frame_cards::{exposure_seconds, read_merged_header};
use crate::types::constants::{
    HEADER_COMBINE_METHOD, HEADER_DRIZZLE_SCALE, HEADER_NCOMBINE, HEADER_NORMALIZATION_METHOD,
    HEADER_REJECTION_METHOD, HEADER_TOTEXP,
};
use crate::types::header::HduHeader;
use crate::types::stacking::{CombineMethod, FrameAlignment, RejectionMethod, RejectionParams};

const CARD_EXPTIME: &str = "EXPTIME";
const EXPOSURE_AGREEMENT: f64 = 0.01;

pub const CARD_SIGMA_LOW: &str = "ABSIGLO";
pub const CARD_SIGMA_HIGH: &str = "ABSIGHI";
pub const CARD_REJECTION_ITERATIONS: &str = "ABREJITR";
pub const CARD_WINSOR_CUTOFF: &str = "ABWINSOR";
pub const CARD_PERCENTILE_LOW: &str = "ABPCTLO";
pub const CARD_PERCENTILE_HIGH: &str = "ABPCTHI";
pub const CARD_MINMAX_LOW: &str = "ABMMLO";
pub const CARD_MINMAX_HIGH: &str = "ABMMHI";
pub const CARD_REJECTION_NORMALIZATION: &str = "ABREJNRM";
pub const CARD_DRIZZLE_PIXFRAC: &str = "ABDRZPXF";
pub const CARD_DRIZZLE_KERNEL: &str = "ABDRZKRN";
pub const CARD_SKY_REFERENCE: &str = "ABSKYREF";

const PROVENANCE_CARDS: [&str; 12] = [
    CARD_SIGMA_LOW,
    CARD_SIGMA_HIGH,
    CARD_REJECTION_ITERATIONS,
    CARD_WINSOR_CUTOFF,
    CARD_PERCENTILE_LOW,
    CARD_PERCENTILE_HIGH,
    CARD_MINMAX_LOW,
    CARD_MINMAX_HIGH,
    CARD_REJECTION_NORMALIZATION,
    CARD_DRIZZLE_PIXFRAC,
    CARD_DRIZZLE_KERNEL,
    CARD_SKY_REFERENCE,
];

pub struct DrizzleCards<'a> {
    pub scale: f64,
    pub pixfrac: f64,
    pub kernel: &'a str,
}

pub struct Provenance<'a> {
    pub rejection: &'a RejectionParams,
    pub rejection_normalization: &'a str,
    pub sky_reference: Option<f64>,
    pub notes: &'a [String],
}

pub struct StackCards<'a> {
    pub included_paths: &'a [&'a str],
    pub combine: CombineMethod,
    pub rejection: &'a str,
    pub normalization: Option<&'a str>,
    pub drizzle: Option<DrizzleCards<'a>>,
    pub provenance: Option<Provenance<'a>>,
}

pub fn frame_note(index: usize, path: &str, frame: &FrameAlignment, mut tokens: Vec<String>) -> String {
    if frame.included {
        if let Some((offset, scale)) = frame.rejection_terms {
            tokens.push(format!("rej_offset={}", offset as f32));
            tokens.push(format!("rej_scale={}", scale as f32));
        }
    } else {
        tokens = vec!["left_out".to_string()];
    }
    tokens.push(format!("align={}", frame.method));
    if let Some(fit) = frame.star_fit {
        tokens.push(format!("stars={}/{}", fit.inliers, fit.matched_stars));
        tokens.push(format!("residual_px={:.3}", fit.residual_px));
    }
    match frame.transform.filter(|_| frame.included) {
        Some([a, b, tx, c, d, ty]) if a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 => {
            tokens.push(format!("shift_x={tx}"));
            tokens.push(format!("shift_y={ty}"));
        }
        Some([a, b, tx, c, d, ty]) => {
            tokens.extend([
                format!("a={a}"),
                format!("b={b}"),
                format!("tx={tx}"),
                format!("c={c}"),
                format!("d={d}"),
                format!("ty={ty}"),
            ]);
        }
        None => {}
    }
    format!("AstroBurst frame {}: {} {}", index + 1, file_label(path), tokens.join(" "))
}

fn provenance_cards(header: &mut HduHeader, provenance: &Provenance) {
    let params = provenance.rejection;
    let sigma_cards = |header: &mut HduHeader| {
        header.set(CARD_SIGMA_LOW, params.sigma_low.to_string());
        header.set(CARD_SIGMA_HIGH, params.sigma_high.to_string());
        header.set(CARD_REJECTION_ITERATIONS, params.max_iterations.to_string());
    };
    match params.rejection {
        RejectionMethod::None => {}
        RejectionMethod::SigmaClip | RejectionMethod::LinearFitClip => sigma_cards(header),
        RejectionMethod::WinsorizedSigmaClip => {
            sigma_cards(header);
            header.set(CARD_WINSOR_CUTOFF, params.winsor_cutoff.to_string());
        }
        RejectionMethod::PercentileClip => {
            header.set(CARD_PERCENTILE_LOW, params.percentile_low.to_string());
            header.set(CARD_PERCENTILE_HIGH, params.percentile_high.to_string());
        }
        RejectionMethod::MinMax => {
            header.set(CARD_MINMAX_LOW, params.minmax_low.to_string());
            header.set(CARD_MINMAX_HIGH, params.minmax_high.to_string());
        }
    }
    if params.rejection != RejectionMethod::None {
        header.set(CARD_REJECTION_NORMALIZATION, provenance.rejection_normalization.to_string());
    }
    if let Some(sky) = provenance.sky_reference {
        header.set_f64(CARD_SKY_REFERENCE, sky);
    }
    for note in provenance.notes {
        push_history(header, note);
    }
}

struct InputCards {
    exposure_s: Option<f64>,
    stacked_frames: Option<u64>,
    total_exposure_s: Option<f64>,
}

fn input_cards(path: &str) -> InputCards {
    let header = read_merged_header(path).unwrap_or_else(HduHeader::empty);
    let exposure_s = exposure_seconds(&header);
    let stacked = header.get(HEADER_COMBINE_METHOD).is_some();
    let stacked_frames = header
        .get_i64(HEADER_NCOMBINE)
        .filter(|_| stacked)
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| *n > 0);
    let total_exposure_s = header
        .get_f64(HEADER_TOTEXP)
        .filter(|_| stacked)
        .filter(|t| t.is_finite() && *t > 0.0)
        .or(exposure_s);
    InputCards { exposure_s, stacked_frames, total_exposure_s }
}

fn ncombine_cards(header: &mut HduHeader, inputs: &[InputCards]) {
    let stacked: Vec<u64> = inputs.iter().filter_map(|c| c.stacked_frames).collect();
    let all_stacked = !inputs.is_empty() && stacked.len() == inputs.len();
    let ncombine: u64 = if all_stacked { stacked.iter().sum() } else { inputs.len() as u64 };
    header.set(HEADER_NCOMBINE, ncombine.to_string());
    if !stacked.is_empty() && !all_stacked {
        push_history(header, "NCOMBINE counts input files: inputs mix stacks and single frames");
    }
}

fn exposure_cards(header: &mut HduHeader, inputs: &[InputCards]) {
    let exposures: Vec<f64> = inputs.iter().filter_map(|c| c.exposure_s).collect();
    if let Some(median) = true_median(&exposures) {
        let min = exposures.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = exposures.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        header.set_f64(CARD_EXPTIME, median);
        if median > 0.0 && (max - min) / median > EXPOSURE_AGREEMENT {
            push_history(
                header,
                &format!(
                    "EXPTIME is the median of {} frames ({}..{} s)",
                    exposures.len(),
                    format_sig4(min),
                    format_sig4(max)
                ),
            );
        }
    }

    let totals: Vec<f64> = inputs.iter().filter_map(|c| c.total_exposure_s).collect();
    if totals.is_empty() {
        header.remove(HEADER_TOTEXP);
        return;
    }
    header.set_f64(HEADER_TOTEXP, totals.iter().sum());
    if totals.len() < inputs.len() {
        push_history(
            header,
            &format!(
                "TOTEXP sums {} of {} frames; the others have no exposure card",
                totals.len(),
                inputs.len()
            ),
        );
    }
}

pub fn stack_output_cards(header: &mut HduHeader, cards: &StackCards) {
    let inputs: Vec<InputCards> = cards.included_paths.iter().map(|p| input_cards(p)).collect();
    ncombine_cards(header, &inputs);
    exposure_cards(header, &inputs);

    header.set(HEADER_COMBINE_METHOD, cards.combine.name().to_string());
    header.set(HEADER_REJECTION_METHOD, cards.rejection.to_string());
    match cards.normalization {
        Some(name) => header.set(HEADER_NORMALIZATION_METHOD, name.to_string()),
        None => header.remove(HEADER_NORMALIZATION_METHOD),
    }
    for key in PROVENANCE_CARDS {
        header.remove(key);
    }
    match &cards.drizzle {
        Some(drizzle) => {
            header.set_f64(HEADER_DRIZZLE_SCALE, drizzle.scale);
            header.set(CARD_DRIZZLE_PIXFRAC, drizzle.pixfrac.to_string());
            header.set(CARD_DRIZZLE_KERNEL, drizzle.kernel.to_string());
            push_history(
                header,
                &format!(
                    "AstroBurst drizzle: {} frames, scale {}, pixfrac {}, {}",
                    inputs.len(),
                    format_sig4(drizzle.scale),
                    format_sig4(drizzle.pixfrac),
                    drizzle.kernel
                ),
            );
        }
        None => {
            header.remove(HEADER_DRIZZLE_SCALE);
            push_history(
                header,
                &format!(
                    "AstroBurst stack: {} frames, {}, {}, {}",
                    inputs.len(),
                    cards.rejection,
                    cards.combine.name(),
                    cards.normalization.unwrap_or("none")
                ),
            );
        }
    }
    if let Some(provenance) = &cards.provenance {
        provenance_cards(header, provenance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::constants::{HEADER_COMBINE_METHOD, HEADER_NCOMBINE, HEADER_TOTEXP};
    use ndarray::Array2;

    fn write_frame(dir: &tempfile::TempDir, name: &str, cards: &[(&str, &str)]) -> String {
        let path = dir.path().join(name).to_str().unwrap().replace('\\', "/");
        let mut header = HduHeader::empty();
        for (k, v) in cards {
            header.set(k, (*v).to_string());
        }
        let frame = Array2::from_elem((4, 4), 10.0f32);
        crate::infra::fits::writer::write_fits_mono(&path, &frame, Some(&header)).unwrap();
        path
    }

    fn history(header: &HduHeader) -> Vec<&str> {
        header.cards.iter().filter(|(k, _)| k.trim() == "HISTORY").map(|(_, v)| v.as_str()).collect()
    }

    fn stack_cards<'a>(paths: &'a [&'a str]) -> StackCards<'a> {
        StackCards {
            included_paths: paths,
            combine: CombineMethod::Mean,
            rejection: "sigma_clip",
            normalization: Some("additive_scaling"),
            drizzle: None,
            provenance: None,
        }
    }

    #[test]
    fn mixed_exposures_write_the_median_and_a_history_note() {
        let dir = tempfile::tempdir().unwrap();
        let paths = [
            write_frame(&dir, "a.fits", &[("EXPTIME", "120")]),
            write_frame(&dir, "b.fits", &[("EXPTIME", "300")]),
            write_frame(&dir, "c.fits", &[("EXPTIME", "300")]),
        ];
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let mut header = HduHeader::empty();
        stack_output_cards(&mut header, &stack_cards(&refs));
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(3));
        assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(720.0));
        let notes = history(&header);
        assert!(
            notes.iter().any(|h| *h == "EXPTIME is the median of 3 frames (120..300 s)"),
            "{notes:?}"
        );
        assert!(notes.iter().any(|h| *h == "AstroBurst stack: 3 frames, sigma_clip, mean, additive_scaling"), "{notes:?}");
    }

    #[test]
    fn stacked_inputs_sum_their_ncombine_and_totexp() {
        let dir = tempfile::tempdir().unwrap();
        let stacked_a = write_frame(
            &dir,
            "stack_a.fits",
            &[("NCOMBINE", "10"), ("ABCOMB", "mean"), ("TOTEXP", "3000"), ("EXPTIME", "300")],
        );
        let stacked_b = write_frame(
            &dir,
            "stack_b.fits",
            &[("NCOMBINE", "6"), ("ABCOMB", "mean"), ("TOTEXP", "1800"), ("EXPTIME", "300")],
        );
        let wfpc2_a = write_frame(&dir, "wfpc2_a.fits", &[("NCOMBINE", "2"), ("EXPTIME", "1100")]);
        let wfpc2_b = write_frame(&dir, "wfpc2_b.fits", &[("NCOMBINE", "2"), ("EXPTIME", "1100")]);

        let mut header = HduHeader::empty();
        stack_output_cards(&mut header, &stack_cards(&[stacked_a.as_str(), stacked_b.as_str()]));
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(16));
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(4800.0));
        assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
        assert_eq!(header.get(HEADER_COMBINE_METHOD).map(str::trim), Some("mean"));
        assert!(!history(&header).iter().any(|h| h.starts_with("NCOMBINE counts input files")), "{:?}", history(&header));

        let mut header = HduHeader::empty();
        header.set(HEADER_NCOMBINE, "2".to_string());
        stack_output_cards(&mut header, &stack_cards(&[wfpc2_a.as_str(), wfpc2_b.as_str()]));
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(2), "an external NCOMBINE without ABCOMB counts as one frame");
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(2200.0));
        assert_eq!(header.get_f64("EXPTIME"), Some(1100.0));

        let mut header = HduHeader::empty();
        stack_output_cards(&mut header, &stack_cards(&[stacked_a.as_str(), wfpc2_a.as_str()]));
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(2), "a stack mixed with a single frame counts input files");
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(4100.0));
        assert!(
            history(&header).iter().any(|h| *h == "NCOMBINE counts input files: inputs mix stacks and single frames"),
            "{:?}",
            history(&header)
        );
    }

    #[test]
    fn long_stack_history_is_wrapped_not_cut() {
        let dir = tempfile::tempdir().unwrap();
        let paths = [
            write_frame(&dir, "a.fits", &[("EXPTIME", "60")]),
            write_frame(&dir, "b.fits", &[("EXPTIME", "60")]),
            write_frame(&dir, "c.fits", &[("EXPTIME", "60")]),
        ];
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let mut header = HduHeader::empty();
        stack_output_cards(
            &mut header,
            &StackCards {
                included_paths: &refs,
                combine: CombineMethod::Median,
                rejection: "winsorized_sigma_clip",
                normalization: Some("multiplicative_scaling"),
                drizzle: None,
                provenance: None,
            },
        );
        let notes = history(&header);
        assert!(notes.len() >= 2, "{notes:?}");
        assert!(notes.iter().all(|h| h.len() <= 72), "{notes:?}");
        assert_eq!(
            notes.join(" "),
            "AstroBurst stack: 3 frames, winsorized_sigma_clip, median, multiplicative_scaling"
        );
    }

    #[test]
    fn unknown_exposures_are_counted_in_a_history_note_and_never_invent_totexp() {
        let dir = tempfile::tempdir().unwrap();
        let known = write_frame(&dir, "known.fits", &[("EXPTIME", "60")]);
        let unknown = write_frame(&dir, "unknown.fits", &[("OBJECT", "M42")]);

        let mut header = HduHeader::empty();
        stack_output_cards(&mut header, &stack_cards(&[known.as_str(), unknown.as_str()]));
        assert_eq!(header.get_i64(HEADER_NCOMBINE), Some(2));
        assert_eq!(header.get_f64(HEADER_TOTEXP), Some(60.0));
        assert_eq!(header.get_f64("EXPTIME"), Some(60.0));
        assert!(
            history(&header).iter().any(|h| *h == "TOTEXP sums 1 of 2 frames; the others have no exposure card"),
            "{:?}",
            history(&header)
        );

        let mut header = HduHeader::empty();
        header.set(HEADER_TOTEXP, "999".to_string());
        header.set("EXPTIME", "7".to_string());
        stack_output_cards(&mut header, &stack_cards(&[unknown.as_str(), unknown.as_str()]));
        assert_eq!(header.get(HEADER_TOTEXP), None, "a stale TOTEXP must not survive when no exposure is known");
        assert_eq!(header.get_f64("EXPTIME"), Some(7.0));
    }

    #[test]
    fn frame_notes_name_the_placement_and_the_rejection_terms() {
        let shifted = FrameAlignment {
            method: "phase_correlation".into(),
            confidence: Some(0.9),
            transform: Some([1.0, 0.0, 0.5, 0.0, 1.0, -1.25]),
            rejection_terms: Some((-140.5, 0.875)),
            ..FrameAlignment::unaligned()
        };
        assert_eq!(
            frame_note(1, "C:/data/light_002.fits", &shifted, vec!["weight=2".to_string()]),
            "AstroBurst frame 2: light_002.fits weight=2 rej_offset=-140.5 rej_scale=0.875 align=phase_correlation shift_x=0.5 shift_y=-1.25"
        );
        let rotated = FrameAlignment {
            method: "affine".into(),
            transform: Some([0.0, -1.0, 63.0, 1.0, 0.0, 0.5]),
            ..FrameAlignment::unaligned()
        };
        assert_eq!(
            frame_note(0, "a.fits", &rotated, Vec::new()),
            "AstroBurst frame 1: a.fits align=affine a=0 b=-1 tx=63 c=1 d=0 ty=0.5"
        );
        let left_out = FrameAlignment { method: "identity".into(), included: false, ..shifted };
        assert_eq!(
            frame_note(2, "c.fits", &left_out, vec!["weight=3".to_string()]),
            "AstroBurst frame 3: c.fits left_out align=identity"
        );
    }

    #[test]
    fn provenance_cards_follow_the_rejection_method_and_stale_ones_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let frame = write_frame(&dir, "a.fits", &[("EXPTIME", "60")]);
        let paths = [frame.as_str(), frame.as_str(), frame.as_str()];
        let mut header = HduHeader::empty();
        for stale in [CARD_SIGMA_LOW, CARD_SIGMA_HIGH, CARD_REJECTION_ITERATIONS, CARD_DRIZZLE_PIXFRAC, CARD_DRIZZLE_KERNEL] {
            header.set(stale, "9".to_string());
        }
        let rejection = RejectionParams { rejection: RejectionMethod::PercentileClip, ..RejectionParams::default() };
        let notes = vec!["AstroBurst frame 1: a.fits align=reference".to_string()];
        let provenance = Provenance { rejection: &rejection, rejection_normalization: "scale_offset", sky_reference: Some(1234.5), notes: &notes };
        stack_output_cards(
            &mut header,
            &StackCards { rejection: "percentile_clip", provenance: Some(provenance), ..stack_cards(&paths) },
        );
        assert_eq!(header.get(CARD_PERCENTILE_LOW).map(str::trim), Some("0.2"));
        assert_eq!(header.get(CARD_PERCENTILE_HIGH).map(str::trim), Some("0.1"));
        assert_eq!(header.get(CARD_REJECTION_NORMALIZATION).map(str::trim), Some("scale_offset"));
        assert_eq!(header.get_f64(CARD_SKY_REFERENCE), Some(1234.5));
        for absent in [CARD_SIGMA_LOW, CARD_SIGMA_HIGH, CARD_REJECTION_ITERATIONS, CARD_DRIZZLE_PIXFRAC, CARD_DRIZZLE_KERNEL] {
            assert_eq!(header.get(absent), None, "{absent} does not describe a percentile clip stack");
        }
        assert_eq!(history(&header).last().copied(), Some("AstroBurst frame 1: a.fits align=reference"));

        let winsorized = RejectionParams { rejection: RejectionMethod::WinsorizedSigmaClip, ..RejectionParams::default() };
        let provenance = Provenance { rejection: &winsorized, rejection_normalization: "none", sky_reference: None, notes: &[] };
        stack_output_cards(&mut header, &StackCards { provenance: Some(provenance), ..stack_cards(&paths) });
        assert_eq!(header.get(CARD_SIGMA_LOW).map(str::trim), Some("4"));
        assert_eq!(header.get(CARD_SIGMA_HIGH).map(str::trim), Some("3"));
        assert_eq!(header.get(CARD_REJECTION_ITERATIONS).map(str::trim), Some("5"));
        assert_eq!(header.get(CARD_WINSOR_CUTOFF).map(str::trim), Some("5"));
        assert_eq!(header.get(CARD_PERCENTILE_LOW), None);
        assert_eq!(header.get(CARD_SKY_REFERENCE), None);

        stack_output_cards(&mut header, &stack_cards(&paths));
        for key in PROVENANCE_CARDS {
            assert_eq!(header.get(key), None, "{key} survived a stack that records no provenance");
        }
    }
}
