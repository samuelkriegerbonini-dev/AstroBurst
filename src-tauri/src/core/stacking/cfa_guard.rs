use anyhow::{bail, Result};

use crate::core::imaging::debayer::BayerPattern;
use crate::infra::image_source::load_plane_header;
use crate::types::header::HduHeader;
use crate::types::image_ref::ImageRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfaStep {
    Align,
    Drizzle,
}

pub fn cfa_pattern(header: &HduHeader) -> Option<BayerPattern> {
    header
        .get("BAYERPAT")
        .or_else(|| header.get("COLORTYP"))
        .and_then(BayerPattern::parse)
}

pub fn cfa_refusal(patterns: &[Option<BayerPattern>], step: CfaStep) -> Option<String> {
    let first = patterns.iter().flatten().next()?;
    let n = patterns.iter().flatten().count();
    let m = patterns.len();
    let pattern = first.name();
    Some(match step {
        CfaStep::Align => format!(
            "{n} of {m} lights are one-shot-colour frames (BAYERPAT={pattern}); aligning them before debayering mixes the colour sites. Calibrate the lights, debayer them (Processing > Debayer > Debayer All Loaded), then stack the _R, _G and _B files as separate channels; or turn alignment off for undithered frames."
        ),
        CfaStep::Drizzle => format!(
            "{n} of {m} lights are one-shot-colour frames (BAYERPAT={pattern}); drizzle resamples across the colour sites and has no CFA mode. Calibrate the lights, debayer them (Processing > Debayer > Debayer All Loaded), then drizzle the _R, _G and _B files as separate channels."
        ),
    })
}

pub fn refuse_cfa_frames(paths: &[String], step: CfaStep) -> Result<()> {
    let patterns: Vec<Option<BayerPattern>> = paths
        .iter()
        .map(|p| load_plane_header(&ImageRef::parse(p)).ok().and_then(|h| cfa_pattern(&h)))
        .collect();
    if let Some(text) = cfa_refusal(&patterns, step) {
        bail!("{text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    use crate::core::compose::drizzle_rgb::drizzle_rgb;
    use crate::core::stacking::calibration::{drizzle_from_paths, stack_from_paths};
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::compose::{DrizzleRgbConfig, WhiteBalance};
    use crate::types::stacking::{DrizzleConfig, StackConfig};

    fn frame(index: usize) -> Array2<f32> {
        Array2::from_shape_fn((32, 32), |(y, x)| {
            let blob = |cy: f64, cx: f64| {
                2000.0 * (-((y as f64 - cy).powi(2) + (x as f64 - cx).powi(2)) / 3.0).exp()
            };
            let ramp = ((x * 7 + y * 3 + index) % 5) as f32;
            100.0 + ramp + (blob(8.0, 20.0) + blob(22.0, 9.0) + blob(25.0, 26.0)) as f32
        })
    }

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in cards {
            h.set(k, (*v).to_string());
        }
        h
    }

    fn write_frames(dir: &tempfile::TempDir, prefix: &str, count: usize, cards: &[(&str, &str)]) -> Vec<String> {
        (0..count)
            .map(|i| {
                let path = dir.path().join(format!("{prefix}{i}.fits")).to_str().unwrap().to_string();
                write_fits_mono(&path, &frame(i), Some(&header(cards))).unwrap();
                path
            })
            .collect()
    }

    fn cfa_frames(dir: &tempfile::TempDir, prefix: &str, count: usize) -> Vec<String> {
        write_frames(dir, prefix, count, &[("BAYERPAT", "RGGB"), ("EXPTIME", "60.0")])
    }

    fn plain_frames(dir: &tempfile::TempDir, prefix: &str, count: usize) -> Vec<String> {
        write_frames(dir, prefix, count, &[("EXPTIME", "60.0")])
    }

    #[test]
    fn cfa_pattern_reads_bayerpat_or_colortyp() {
        assert_eq!(cfa_pattern(&header(&[("BAYERPAT", "'RGGB'")])), Some(BayerPattern::Rggb));
        assert_eq!(cfa_pattern(&header(&[("COLORTYP", "GRBG")])), Some(BayerPattern::Grbg));
        assert_eq!(cfa_pattern(&header(&[("COLORTYP", "RGB")])), None);
        assert_eq!(cfa_pattern(&header(&[("COLORTYP", "MONO")])), None);
        assert_eq!(cfa_pattern(&HduHeader::empty()), None);
    }

    #[test]
    fn refusal_names_the_count_and_the_pattern() {
        let patterns = [Some(BayerPattern::Rggb), None, Some(BayerPattern::Rggb)];
        let align = cfa_refusal(&patterns, CfaStep::Align).expect("align refusal");
        for needle in ["2 of 3 lights", "BAYERPAT=RGGB", "Debayer All Loaded", "turn alignment off"] {
            assert!(align.contains(needle), "missing {needle:?} in {align}");
        }
        let drizzle = cfa_refusal(&patterns, CfaStep::Drizzle).expect("drizzle refusal");
        for needle in ["2 of 3 lights", "BAYERPAT=RGGB", "no CFA mode", "Debayer All Loaded"] {
            assert!(drizzle.contains(needle), "missing {needle:?} in {drizzle}");
        }
        assert!(!drizzle.contains("turn alignment off"), "{drizzle}");
        assert_eq!(cfa_refusal(&[None, None], CfaStep::Align), None);
        assert_eq!(cfa_refusal(&[], CfaStep::Drizzle), None);
    }

    #[test]
    fn aligned_stack_refuses_cfa_lights_before_loading_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let paths = cfa_frames(&dir, "cfa", 3);
        let truncated = std::fs::OpenOptions::new().write(true).open(&paths[2]).unwrap();
        truncated.set_len(2880).unwrap();
        let config = StackConfig { align: true, ..StackConfig::default() };
        let err = format!("{:#}", stack_from_paths(&paths, &config, None).unwrap_err());
        assert!(err.contains("3 of 3 lights"), "{err}");
        assert!(!err.contains("Failed to load"), "{err}");
    }

    #[test]
    fn unaligned_stack_accepts_cfa_lights() {
        let dir = tempfile::tempdir().unwrap();
        let paths = cfa_frames(&dir, "cfa", 3);
        let config = StackConfig { align: false, ..StackConfig::default() };
        let result = stack_from_paths(&paths, &config, None).unwrap();
        assert_eq!(result.frame_count, 3);
    }

    #[test]
    fn debayered_lights_pass_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let paths = plain_frames(&dir, "light_bilinear_RGGB_R", 3);
        let config = StackConfig { align: true, ..StackConfig::default() };
        let result = stack_from_paths(&paths, &config, None);
        assert!(result.is_ok(), "{:#}", result.err().unwrap());
    }

    #[test]
    fn drizzle_refuses_cfa_lights_even_without_alignment() {
        let dir = tempfile::tempdir().unwrap();
        let paths = cfa_frames(&dir, "cfa", 2);
        let config = DrizzleConfig { align: false, ..DrizzleConfig::default() };
        let err = format!("{:#}", drizzle_from_paths(&paths, &config, None).unwrap_err());
        assert!(err.contains("drizzle resamples"), "{err}");
        assert!(err.contains("2 of 2 lights"), "{err}");
    }

    #[test]
    fn rgb_drizzle_refuses_cfa_lights() {
        let dir = tempfile::tempdir().unwrap();
        let r = cfa_frames(&dir, "r", 2);
        let g = cfa_frames(&dir, "g", 2);
        let png = dir.path().join("rgb.png");
        let config = DrizzleRgbConfig {
            white_balance: WhiteBalance::None,
            auto_stretch: false,
            align: false,
            drizzle: DrizzleConfig { align: false, ..DrizzleConfig::default() },
            ..DrizzleRgbConfig::default()
        };
        let err = format!(
            "{:#}",
            drizzle_rgb(Some(&r), Some(&g), None, png.to_str().unwrap(), None, &config).unwrap_err()
        );
        assert!(err.contains("4 of 4 lights"), "{err}");
        assert!(err.contains("drizzle resamples"), "{err}");
        assert!(!png.exists(), "the PNG was written despite the refusal");
    }

    #[test]
    fn a_missing_light_is_left_to_the_loader() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = plain_frames(&dir, "plain", 1);
        paths.push(dir.path().join("missing.fits").to_str().unwrap().to_string());
        let config = StackConfig { align: true, ..StackConfig::default() };
        let err = format!("{:#}", stack_from_paths(&paths, &config, None).unwrap_err());
        assert!(err.contains("Failed to"), "{err}");
        assert!(!err.contains("one-shot-colour"), "{err}");
    }
}
