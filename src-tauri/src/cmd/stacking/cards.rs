use crate::core::stacking::consistency::{format_sig4, true_median};
use crate::core::stacking::frame_cards::{exposure_seconds, read_merged_header};
use crate::types::constants::{
    HEADER_COMBINE_METHOD, HEADER_DRIZZLE_SCALE, HEADER_NCOMBINE, HEADER_NORMALIZATION_METHOD,
    HEADER_REJECTION_METHOD, HEADER_TOTEXP,
};
use crate::types::header::HduHeader;
use crate::types::stacking::CombineMethod;

const CARD_EXPTIME: &str = "EXPTIME";
const CARD_HISTORY: &str = "HISTORY";
const EXPOSURE_AGREEMENT: f64 = 0.01;

pub struct DrizzleCards<'a> {
    pub scale: f64,
    pub pixfrac: f64,
    pub kernel: &'a str,
}

pub struct StackCards<'a> {
    pub included_paths: &'a [&'a str],
    pub combine: CombineMethod,
    pub rejection: &'a str,
    pub normalization: Option<&'a str>,
    pub drizzle: Option<DrizzleCards<'a>>,
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

fn push_history(header: &mut HduHeader, line: String) {
    header.cards.push((CARD_HISTORY.to_string(), line));
}

fn ncombine_cards(header: &mut HduHeader, inputs: &[InputCards]) {
    let stacked: Vec<u64> = inputs.iter().filter_map(|c| c.stacked_frames).collect();
    let all_stacked = !inputs.is_empty() && stacked.len() == inputs.len();
    let ncombine: u64 = if all_stacked { stacked.iter().sum() } else { inputs.len() as u64 };
    header.set(HEADER_NCOMBINE, ncombine.to_string());
    if !stacked.is_empty() && !all_stacked {
        push_history(header, "NCOMBINE counts input files: inputs mix stacks and single frames".to_string());
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
                format!(
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
            format!(
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
    match &cards.drizzle {
        Some(drizzle) => {
            header.set_f64(HEADER_DRIZZLE_SCALE, drizzle.scale);
            push_history(
                header,
                format!(
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
                format!(
                    "AstroBurst stack: {} frames, {}, {}, {}",
                    inputs.len(),
                    cards.rejection,
                    cards.combine.name(),
                    cards.normalization.unwrap_or("none")
                ),
            );
        }
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
}
