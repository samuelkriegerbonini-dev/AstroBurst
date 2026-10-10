use anyhow::{bail, Result};

use crate::core::stacking::frame_cards::{FrameCards, FrameKind};

const EXPOSURE_TOLERANCE: f64 = 0.01;
const FLAT_EXPOSURE_SPREAD_WARN: f64 = 0.20;
const BIAS_EXPOSURE_MAX_S: f64 = 0.5;
const FLAT_DARK_EXPOSURE_TOLERANCE: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterKind {
    Bias,
    Dark,
    Flat,
    FlatDark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    Warn,
    Refuse,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConsistencyReport {
    pub warnings: Vec<String>,
}

impl MasterKind {
    pub fn label(self) -> &'static str {
        match self {
            MasterKind::Bias => "bias",
            MasterKind::Dark => "dark",
            MasterKind::Flat => "flat",
            MasterKind::FlatDark => "flat-dark",
        }
    }

    fn title(self) -> &'static str {
        match self {
            MasterKind::Bias => "Bias",
            MasterKind::Dark => "Dark",
            MasterKind::Flat => "Flat",
            MasterKind::FlatDark => "Flat-dark",
        }
    }

    fn accepts(self, frame: FrameKind) -> bool {
        match self {
            MasterKind::Bias => frame == FrameKind::Bias,
            MasterKind::Dark => frame == FrameKind::Dark,
            MasterKind::Flat => frame == FrameKind::Flat,
            MasterKind::FlatDark => matches!(frame, FrameKind::FlatDark | FrameKind::Dark | FrameKind::Bias),
        }
    }
}

pub fn format_sig4(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (3 - magnitude).max(0) as usize;
    let text = format!("{:.*}", decimals, value);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

pub fn file_label(path: &str) -> String {
    let source = crate::types::image_ref::ImageRef::parse(path).path;
    std::path::Path::new(&source)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

fn file_list(frames: &[&FrameCards]) -> String {
    frames.iter().map(|f| file_label(&f.path)).collect::<Vec<_>>().join(", ")
}

pub fn true_median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 0 { (sorted[mid - 1] + sorted[mid]) / 2.0 } else { sorted[mid] })
}

fn exposures(frames: &[FrameCards]) -> Vec<f64> {
    frames.iter().filter_map(|f| f.exposure_s).collect()
}

pub fn median_exposure(frames: &[FrameCards]) -> Option<f64> {
    true_median(&exposures(frames))
}

fn relative_difference(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        return f64::INFINITY;
    }
    ((a - b) / b).abs()
}

pub fn imagetyp_verdict(kind: MasterKind, frame: FrameKind) -> Verdict {
    if frame == FrameKind::Unknown || kind.accepts(frame) {
        return Verdict::Accept;
    }
    let refused = kind != MasterKind::Flat && matches!(frame, FrameKind::Light | FrameKind::Flat);
    if refused {
        Verdict::Refuse
    } else {
        Verdict::Warn
    }
}

fn check_imagetyp(kind: MasterKind, frames: &[FrameCards], warnings: &mut Vec<String>) -> Result<()> {
    for frame in frames {
        let value = frame.imagetyp.as_deref().unwrap_or("");
        match imagetyp_verdict(kind, frame.frame_kind) {
            Verdict::Accept => {}
            Verdict::Warn => warnings.push(format!(
                "{} has IMAGETYP='{}' in the {} row; it is used as a {} frame.",
                file_label(&frame.path),
                value,
                kind.label(),
                kind.label()
            )),
            Verdict::Refuse => bail!(
                "{} has IMAGETYP='{}' but was given as a {} frame.",
                file_label(&frame.path),
                value,
                kind.label()
            ),
        }
    }
    Ok(())
}

fn refuse_mixed<T: PartialEq>(
    kind: MasterKind,
    frames: &[FrameCards],
    card: &str,
    pick: impl Fn(&FrameCards) -> Option<T>,
    show: impl Fn(&T) -> String,
) -> Result<()> {
    let Some(first) = frames.iter().find_map(|f| pick(f)) else {
        return Ok(());
    };
    let odd: Vec<&FrameCards> = frames.iter().filter(|f| pick(f).is_some_and(|v| v != first)).collect();
    if let Some(other) = odd.first().and_then(|f| pick(f)) {
        bail!(
            "{} frames mix {} ({} and {}): {}.",
            kind.title(),
            card,
            show(&first),
            show(&other),
            file_list(&odd)
        );
    }
    Ok(())
}

fn check_camera_settings(kind: MasterKind, frames: &[FrameCards]) -> Result<()> {
    refuse_mixed(
        kind,
        frames,
        "binning",
        |f| f.xbinning.zip(f.ybinning),
        |(x, y)| format!("{x}x{y}"),
    )?;
    refuse_mixed(kind, frames, "GAIN", |f| f.gain, |g| format_sig4(*g))?;
    refuse_mixed(kind, frames, "OFFSET", |f| f.offset, |o| format_sig4(*o))
}

fn check_missing_exposures(kind: MasterKind, frames: &[FrameCards], warnings: &mut Vec<String>) {
    let missing: Vec<&FrameCards> = frames.iter().filter(|f| f.exposure_s.is_none()).collect();
    if missing.is_empty() || missing.len() == frames.len() {
        return;
    }
    warnings.push(format!(
        "{} of {} {} frames have no exposure card: {}; the exposure checks skipped them.",
        missing.len(),
        frames.len(),
        kind.label(),
        file_list(&missing)
    ));
}

fn exposure_spread(values: &[f64]) -> Option<(f64, f64, f64, f64)> {
    let median = true_median(values)?;
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let spread = if median > 0.0 { (max - min) / median } else { 0.0 };
    Some((min, max, median, spread))
}

fn check_exposures(kind: MasterKind, frames: &[FrameCards], warnings: &mut Vec<String>) -> Result<()> {
    match kind {
        MasterKind::Bias => {
            for frame in frames {
                if let Some(exp) = frame.exposure_s.filter(|e| *e > BIAS_EXPOSURE_MAX_S) {
                    warnings.push(format!(
                        "{} is a {} s frame in the Bias row; bias frames should be the shortest exposure the camera allows.",
                        file_label(&frame.path),
                        format_sig4(exp)
                    ));
                }
            }
            Ok(())
        }
        MasterKind::Dark => {
            check_missing_exposures(kind, frames, warnings);
            let values = exposures(frames);
            if let Some((min, max, _, spread)) = exposure_spread(&values) {
                if spread > EXPOSURE_TOLERANCE {
                    let timed: Vec<&FrameCards> = frames.iter().filter(|f| f.exposure_s.is_some()).collect();
                    bail!(
                        "Darks have mixed exposures ({} s to {} s); build one master per exposure or drop the odd frames: {}.",
                        format_sig4(min),
                        format_sig4(max),
                        file_list(&timed)
                    );
                }
            }
            Ok(())
        }
        MasterKind::Flat => {
            check_missing_exposures(kind, frames, warnings);
            let values = exposures(frames);
            if let Some((min, max, median, spread)) = exposure_spread(&values) {
                if spread > FLAT_EXPOSURE_SPREAD_WARN {
                    warnings.push(format!(
                        "Flats have exposures from {} s to {} s; the dark scaling uses their median ({} s).",
                        format_sig4(min),
                        format_sig4(max),
                        format_sig4(median)
                    ));
                }
            }
            Ok(())
        }
        MasterKind::FlatDark => {
            check_missing_exposures(kind, frames, warnings);
            Ok(())
        }
    }
}

fn check_flat_filters(frames: &[FrameCards]) -> Result<()> {
    let Some(first) = frames.iter().find_map(|f| f.filter.clone()) else {
        return Ok(());
    };
    let odd: Vec<&FrameCards> = frames.iter().filter(|f| f.filter.as_ref().is_some_and(|v| *v != first)).collect();
    if let Some(other) = odd.first().and_then(|f| f.filter.clone()) {
        bail!(
            "Flats mix filters ({} and {}); use one flat set per filter: {}.",
            first,
            other,
            file_list(&odd)
        );
    }
    Ok(())
}

pub fn check_master_set(kind: MasterKind, frames: &[FrameCards]) -> Result<ConsistencyReport> {
    let mut warnings = Vec::new();
    check_imagetyp(kind, frames, &mut warnings)?;
    check_camera_settings(kind, frames)?;
    check_exposures(kind, frames, &mut warnings)?;
    if kind == MasterKind::Flat {
        check_flat_filters(frames)?;
    }
    Ok(ConsistencyReport { warnings })
}

pub fn check_flat_darks_vs_flats(flat_darks: &[FrameCards], flats: &[FrameCards]) -> Option<String> {
    let fd = median_exposure(flat_darks)?;
    let flat = median_exposure(flats)?;
    (relative_difference(fd, flat) > FLAT_DARK_EXPOSURE_TOLERANCE).then(|| {
        format!(
            "Flat-darks are {} s but the flats are {} s; flat-darks should match the flat exposure.",
            format_sig4(fd),
            format_sig4(flat)
        )
    })
}

pub(crate) fn channel_prefix(channel: &str) -> String {
    if channel.is_empty() {
        String::new()
    } else {
        format!("Channel '{channel}': ")
    }
}

pub fn dark_unscaled_warning(channel: &str, light_exp: f64, dark_exp: f64) -> String {
    format!(
        "{}lights are {} s but the darks are {} s and there is no master bias, so the dark cannot be scaled; the lights get the unscaled {} s dark. Add bias frames or darks of {} s.",
        channel_prefix(channel),
        format_sig4(light_exp),
        format_sig4(dark_exp),
        format_sig4(dark_exp),
        format_sig4(light_exp)
    )
}

pub fn exposures_differ(light_exp: f64, dark_exp: f64) -> bool {
    relative_difference(light_exp, dark_exp) > EXPOSURE_TOLERANCE
}

fn setting_warning(
    channel: &str,
    card: &str,
    lights: &[FrameCards],
    reference: &[FrameCards],
    kind: MasterKind,
    pick: impl Fn(&FrameCards) -> Option<f64>,
) -> Option<String> {
    let light = lights.iter().find_map(&pick)?;
    let master = reference.iter().find_map(&pick)?;
    (light != master).then(|| {
        format!(
            "{}lights have {}={} but the {} frames have {}={}; calibration frames must match the lights' camera settings.",
            channel_prefix(channel),
            card,
            format_sig4(light),
            kind.label(),
            card,
            format_sig4(master)
        )
    })
}

pub fn check_lights(
    channel: &str,
    lights: &[FrameCards],
    bias: &[FrameCards],
    darks: &[FrameCards],
    flats: &[FrameCards],
) -> Vec<String> {
    let mut warnings = Vec::new();
    if bias.is_empty() && !darks.is_empty() {
        if let (Some(light_exp), Some(dark_exp)) = (median_exposure(lights), median_exposure(darks)) {
            if exposures_differ(light_exp, dark_exp) {
                warnings.push(dark_unscaled_warning(channel, light_exp, dark_exp));
            }
        }
    }
    if let (Some(light_filter), Some(flat_filter)) =
        (lights.iter().find_map(|f| f.filter.clone()), flats.iter().find_map(|f| f.filter.clone()))
    {
        if light_filter != flat_filter {
            warnings.push(if channel.is_empty() {
                format!("Lights (filter {light_filter}) are flat-fielded with filter {flat_filter} flats.")
            } else {
                format!("Channel '{channel}' (filter {light_filter}) is flat-fielded with filter {flat_filter} flats.")
            });
        }
    }
    let (reference, kind) = if darks.is_empty() { (bias, MasterKind::Bias) } else { (darks, MasterKind::Dark) };
    warnings.extend(setting_warning(channel, "GAIN", lights, reference, kind, |f| f.gain));
    warnings.extend(setting_warning(channel, "OFFSET", lights, reference, kind, |f| f.offset));
    warnings
}

pub fn check_light_binning(channel: &str, lights: &[FrameCards], masters: &[(MasterKind, &[FrameCards])]) -> Result<()> {
    let Some(light) = lights.iter().find_map(|f| f.xbinning.zip(f.ybinning)) else {
        return Ok(());
    };
    for (kind, frames) in masters {
        if let Some(master) = frames.iter().find_map(|f| f.xbinning.zip(f.ybinning)) {
            if master != light {
                bail!(
                    "{}lights are binned {}x{} but the {} frames are binned {}x{}; calibration frames must match the lights' binning.",
                    channel_prefix(channel),
                    light.0,
                    light.1,
                    kind.label(),
                    master.0,
                    master.1
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stacking::frame_cards::frame_cards_from_header;
    use crate::types::header::HduHeader;

    fn cards(path: &str, exposure: Option<f64>) -> FrameCards {
        FrameCards {
            path: path.to_string(),
            exposure_s: exposure,
            filter: None,
            imagetyp: None,
            frame_kind: FrameKind::Unknown,
            gain: None,
            offset: None,
            xbinning: None,
            ybinning: None,
            temp_c: None,
            cfa: false,
        }
    }

    fn typed(path: &str, imagetyp: &str) -> FrameCards {
        let mut header = HduHeader::empty();
        header.set("IMAGETYP", format!("'{imagetyp}'"));
        frame_cards_from_header(path, &header)
    }

    #[test]
    fn mixed_dark_exposures_are_refused_with_the_file_names() {
        let darks = [cards("C:/darks/dark60.fits", Some(60.0)), cards("C:/darks/dark300.fits", Some(300.0))];
        let err = check_master_set(MasterKind::Dark, &darks).unwrap_err().to_string();
        assert!(err.contains("Darks have mixed exposures (60 s to 300 s)"), "{err}");
        assert!(err.contains("dark60.fits") && err.contains("dark300.fits"), "{err}");

        let close = [cards("a.fits", Some(300.0)), cards("b.fits", Some(301.0))];
        assert!(check_master_set(MasterKind::Dark, &close).unwrap().warnings.is_empty());

        let partial = [cards("a.fits", Some(300.0)), cards("b.fits", None), cards("c.fits", Some(300.0))];
        let report = check_master_set(MasterKind::Dark, &partial).unwrap();
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].starts_with("1 of 3 dark frames have no exposure card: b.fits;"), "{}", report.warnings[0]);
    }

    #[test]
    fn mixed_filter_flats_are_refused() {
        let mut ha = cards("flat_ha.fits", Some(2.0));
        ha.filter = Some("Ha".into());
        let mut lum = cards("flat_l.fits", Some(2.0));
        lum.filter = Some("L".into());
        let err = check_master_set(MasterKind::Flat, &[ha.clone(), lum]).unwrap_err().to_string();
        assert!(err.starts_with("Flats mix filters (Ha and L)"), "{err}");
        assert!(err.contains("flat_l.fits"), "{err}");

        let unfiltered = cards("flat_x.fits", Some(2.0));
        assert!(check_master_set(MasterKind::Flat, &[ha.clone(), unfiltered, ha]).unwrap().warnings.is_empty());
    }

    #[test]
    fn binning_gain_offset_mismatch_is_refused() {
        let mut a = cards("d1.fits", Some(300.0));
        let mut b = cards("d2.fits", Some(300.0));
        a.xbinning = Some(1);
        a.ybinning = Some(1);
        b.xbinning = Some(2);
        b.ybinning = Some(2);
        let err = check_master_set(MasterKind::Dark, &[a.clone(), b]).unwrap_err().to_string();
        assert!(err.starts_with("Dark frames mix binning (1x1 and 2x2): d2.fits."), "{err}");

        let mut c = a.clone();
        c.path = "d3.fits".into();
        a.gain = Some(100.0);
        c.gain = Some(120.0);
        let err = check_master_set(MasterKind::Dark, &[a.clone(), c.clone()]).unwrap_err().to_string();
        assert!(err.starts_with("Dark frames mix GAIN (100 and 120): d3.fits."), "{err}");

        c.gain = Some(100.0);
        a.offset = Some(30.0);
        c.offset = Some(50.0);
        let err = check_master_set(MasterKind::Bias, &[a.clone(), c.clone()]).unwrap_err().to_string();
        assert!(err.starts_with("Bias frames mix OFFSET (30 and 50): d3.fits."), "{err}");

        c.offset = Some(30.0);
        assert!(check_master_set(MasterKind::Dark, &[a, c]).unwrap().warnings.is_empty());
    }

    #[test]
    fn imagetyp_light_in_darks_is_refused() {
        let err = check_master_set(MasterKind::Dark, &[typed("light1.fits", "Light")]).unwrap_err().to_string();
        assert_eq!(err, "light1.fits has IMAGETYP='light' but was given as a dark frame.");
        let err = check_master_set(MasterKind::FlatDark, &[typed("flat1.fits", "Flat")]).unwrap_err().to_string();
        assert_eq!(err, "flat1.fits has IMAGETYP='flat' but was given as a flat-dark frame.");
        let err = check_master_set(MasterKind::Bias, &[typed("f.fits", "Flat Field")]).unwrap_err().to_string();
        assert!(err.contains("given as a bias frame"), "{err}");
        assert_eq!(imagetyp_verdict(MasterKind::Dark, FrameKind::Light), Verdict::Refuse);
        assert_eq!(imagetyp_verdict(MasterKind::Flat, FrameKind::Dark), Verdict::Warn);
    }

    #[test]
    fn imagetyp_dark_and_bias_in_flat_darks_are_accepted() {
        let flat_darks = [
            typed("fd1.fits", "Dark"),
            typed("fd2.fits", "DARK"),
            typed("fd3.fits", "Dark Frame"),
            typed("fd4.fits", "Bias"),
            typed("fd5.fits", "Flat Dark"),
        ];
        let report = check_master_set(MasterKind::FlatDark, &flat_darks).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let report = check_master_set(MasterKind::Dark, &[typed("d1.fits", "Flat Dark")]).unwrap();
        assert_eq!(report.warnings, vec!["d1.fits has IMAGETYP='flat dark' in the dark row; it is used as a dark frame.".to_string()]);

        let report = check_master_set(MasterKind::Flat, &[typed("sky1.fits", "Light")]).unwrap();
        assert_eq!(report.warnings, vec!["sky1.fits has IMAGETYP='light' in the flat row; it is used as a flat frame.".to_string()]);

        for kind in [MasterKind::Bias, MasterKind::Dark, MasterKind::Flat, MasterKind::FlatDark] {
            let report = check_master_set(kind, &[cards("plain.fits", None)]).unwrap();
            assert!(report.warnings.is_empty(), "{kind:?}: {:?}", report.warnings);
            assert_eq!(imagetyp_verdict(kind, FrameKind::Unknown), Verdict::Accept);
        }
    }

    #[test]
    fn flat_exposure_spread_warns_but_passes() {
        let flats = [cards("flat2.fits", Some(2.0)), cards("flat3.fits", Some(3.0))];
        let report = check_master_set(MasterKind::Flat, &flats).unwrap();
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert_eq!(
            report.warnings[0],
            "Flats have exposures from 2 s to 3 s; the dark scaling uses their median (2.5 s)."
        );

        let even = [cards("flat2.fits", Some(2.0)), cards("flat2b.fits", Some(2.2))];
        assert!(check_master_set(MasterKind::Flat, &even).unwrap().warnings.is_empty());

        let long_bias = [cards("bias0.fits", None), cards("bias1.fits", Some(1.5))];
        let report = check_master_set(MasterKind::Bias, &long_bias).unwrap();
        assert_eq!(
            report.warnings,
            vec!["bias1.fits is a 1.5 s frame in the Bias row; bias frames should be the shortest exposure the camera allows.".to_string()]
        );
    }

    #[test]
    fn lights_vs_darks_warn_when_exposures_differ_without_bias() {
        let lights = [cards("l1.fits", Some(120.0)), cards("l2.fits", Some(120.0))];
        let darks = [cards("d1.fits", Some(300.0)), cards("d2.fits", Some(300.0))];
        let bias = [cards("b1.fits", None)];
        let warnings = check_lights("R", &lights, &[], &darks, &[]);
        assert_eq!(
            warnings,
            vec!["Channel 'R': lights are 120 s but the darks are 300 s and there is no master bias, so the dark cannot be scaled; the lights get the unscaled 300 s dark. Add bias frames or darks of 120 s.".to_string()]
        );
        let plain = check_lights("", &lights, &[], &darks, &[]);
        assert_eq!(plain, vec!["lights are 120 s but the darks are 300 s and there is no master bias, so the dark cannot be scaled; the lights get the unscaled 300 s dark. Add bias frames or darks of 120 s.".to_string()]);
        assert!(check_lights("R", &lights, &bias, &darks, &[]).is_empty());
        assert!(check_lights("R", &lights, &[], &lights, &[]).is_empty());
        assert!(check_lights("R", &lights, &[], &[], &[]).is_empty());
    }

    #[test]
    fn lights_vs_darks_gain_mismatch_warns() {
        let mut light = cards("l1.fits", Some(300.0));
        light.gain = Some(100.0);
        light.offset = Some(30.0);
        let mut dark = cards("d1.fits", Some(300.0));
        dark.gain = Some(120.0);
        dark.offset = Some(30.0);
        let warnings = check_lights("L", &[light.clone()], &[], &[dark.clone()], &[]);
        assert_eq!(
            warnings,
            vec!["Channel 'L': lights have GAIN=100 but the dark frames have GAIN=120; calibration frames must match the lights' camera settings.".to_string()]
        );

        dark.gain = Some(100.0);
        assert!(check_lights("L", &[light.clone()], &[], &[dark.clone()], &[]).is_empty());

        dark.offset = Some(50.0);
        let warnings = check_lights("L", &[light.clone()], &[], &[dark.clone()], &[]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("OFFSET=30") && warnings[0].contains("OFFSET=50"), "{}", warnings[0]);

        let mut bias = cards("b1.fits", None);
        bias.gain = Some(120.0);
        let warnings = check_lights("L", &[light], &[bias], &[], &[]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("the bias frames have GAIN=120"), "{}", warnings[0]);
    }

    #[test]
    fn channel_filter_differs_from_flats_warns() {
        let mut light_header = HduHeader::empty();
        light_header.set("FILTNAM1", "'F502N'".to_string());
        light_header.set("EXPTIME", "1100".to_string());
        let mut flat_header = HduHeader::empty();
        flat_header.set("FILTER", "'F656N'".to_string());
        flat_header.set("EXPTIME", "2".to_string());
        let lights = [frame_cards_from_header("502nmos.fits", &light_header)];
        let flats = [frame_cards_from_header("flat656.fits", &flat_header)];
        let warnings = check_lights("R", &lights, &[], &[], &flats);
        assert_eq!(warnings, vec!["Channel 'R' (filter F502N) is flat-fielded with filter F656N flats.".to_string()]);

        let mut same_header = HduHeader::empty();
        same_header.set("FILTER", "'f502n'".to_string());
        let same = [frame_cards_from_header("flat502.fits", &same_header)];
        assert!(check_lights("R", &lights, &[], &[], &same).is_empty());
    }

    #[test]
    fn flat_dark_exposure_far_from_the_flats_warns() {
        let flats = [cards("flat1.fits", Some(2.0)), cards("flat2.fits", Some(2.0))];
        let far = [cards("fd1.fits", Some(2.5))];
        let text = check_flat_darks_vs_flats(&far, &flats).expect("warning");
        assert_eq!(text, "Flat-darks are 2.5 s but the flats are 2 s; flat-darks should match the flat exposure.");
        let near = [cards("fd1.fits", Some(2.05))];
        assert_eq!(check_flat_darks_vs_flats(&near, &flats), None);
        assert_eq!(check_flat_darks_vs_flats(&[], &flats), None);
    }
}
