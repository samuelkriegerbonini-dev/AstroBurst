use anyhow::{Context, Result};

pub use crate::core::imaging::calibration_pipeline::DarkGroup;
use crate::core::imaging::calibration_pipeline::CalibrationMasters;
use crate::core::stacking::calibration::{
    create_master_bias_cancellable, create_master_dark_cancellable, create_master_flat_cancellable,
    create_master_flat_dark_cancellable,
};
use crate::core::stacking::consistency::{
    channel_prefix, check_flat_darks_vs_flats, check_light_binning, check_lights, check_master_set, file_label,
    format_sig4, median_exposure, MasterKind,
};
use crate::core::stacking::frame_cards::{read_frame_cards, FrameCards};
use crate::core::stacking::CancelCheck;

pub const TEMP_GAP_C: f64 = 3.0;
pub const TEMP_SPREAD_WARN_C: f64 = 2.0;
pub const TEMP_DELTA_WARN_C: f64 = 2.0;
pub const MIN_GROUP_FRAMES: usize = 5;
const FLAT_VS_DARK_EXPOSURE_TOLERANCE: f64 = 0.01;

#[derive(Debug, Clone, Copy)]
pub struct MasterRequest<'a> {
    pub bias: &'a [String],
    pub darks: &'a [String],
    pub flats: &'a [String],
    pub flat_darks: &'a [String],
}

#[derive(Debug, Clone, Default)]
pub struct MasterFrames {
    pub bias: Vec<FrameCards>,
    pub darks: Vec<FrameCards>,
    pub flats: Vec<FrameCards>,
    pub flat_darks: Vec<FrameCards>,
}

#[derive(Debug, Clone, Default)]
pub struct MasterReport {
    pub warnings: Vec<String>,
    pub dark_exposure_s: Option<f64>,
    pub bias_frames: usize,
    pub dark_frames: usize,
    pub flat_frames: usize,
    pub flat_dark_frames: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DarkGroupPlan {
    pub members: Vec<usize>,
    pub temp_c: Option<f64>,
    pub min_c: f64,
    pub max_c: f64,
}

pub fn format_temp(value: f64) -> String {
    format!("{:.1}", (value * 10.0).round() / 10.0 + 0.0)
}

pub fn read_master_frames(req: &MasterRequest) -> MasterFrames {
    let read = |paths: &[String]| paths.iter().map(|p| read_frame_cards(p)).collect();
    MasterFrames {
        bias: read(req.bias),
        darks: read(req.darks),
        flats: read(req.flats),
        flat_darks: read(req.flat_darks),
    }
}

fn group_mean(members: &[usize], temps: &[f64]) -> f64 {
    members.iter().map(|&i| temps[i]).sum::<f64>() / members.len() as f64
}

fn merge_small_groups(groups: &mut Vec<Vec<usize>>, temps: &[f64]) {
    while groups.len() > 1 {
        let Some(small) =
            (0..groups.len()).filter(|&g| groups[g].len() < MIN_GROUP_FRAMES).min_by_key(|&g| groups[g].len())
        else {
            break;
        };
        let mean = group_mean(&groups[small], temps);
        let prev = small.checked_sub(1);
        let next = (small + 1 < groups.len()).then_some(small + 1);
        let target = match (prev, next) {
            (Some(p), Some(n)) => {
                let to_prev = (mean - group_mean(&groups[p], temps)).abs();
                let to_next = (mean - group_mean(&groups[n], temps)).abs();
                if to_prev <= to_next { p } else { n }
            }
            (Some(p), None) => p,
            (None, Some(n)) => n,
            (None, None) => break,
        };
        let members = groups.remove(small);
        let target = if target > small { target - 1 } else { target };
        groups[target].extend(members);
        groups[target].sort_unstable();
    }
}

pub fn group_darks(darks: &[FrameCards]) -> Vec<DarkGroupPlan> {
    if darks.is_empty() {
        return Vec::new();
    }
    let temps: Option<Vec<f64>> = darks.iter().map(|d| d.temp_c).collect();
    let Some(temps) = temps else {
        return vec![DarkGroupPlan { members: (0..darks.len()).collect(), temp_c: None, min_c: 0.0, max_c: 0.0 }];
    };
    let mut order: Vec<usize> = (0..darks.len()).collect();
    order.sort_by(|&a, &b| temps[a].total_cmp(&temps[b]));
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &i in &order {
        match groups.last_mut() {
            Some(last) if temps[i] - temps[*last.last().expect("non-empty group")] <= TEMP_GAP_C => last.push(i),
            _ => groups.push(vec![i]),
        }
    }
    merge_small_groups(&mut groups, &temps);
    groups
        .into_iter()
        .map(|mut members| {
            members.sort_unstable();
            let values: Vec<f64> = members.iter().map(|&i| temps[i]).collect();
            DarkGroupPlan {
                temp_c: Some(group_mean(&members, &temps)),
                min_c: values.iter().copied().fold(f64::INFINITY, f64::min),
                max_c: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                members,
            }
        })
        .collect()
}

fn spread_warnings(plans: &[DarkGroupPlan]) -> Vec<String> {
    plans
        .iter()
        .filter(|plan| plan.temp_c.is_some() && plan.max_c - plan.min_c > TEMP_SPREAD_WARN_C)
        .map(|plan| {
            format!(
                "Darks in the {} °C group span {} to {} °C ({} °C); one master is built from all of them. Dark current changes ~2x per 7 °C, so enable dark optimisation or split the library at set points.",
                format_temp(plan.temp_c.unwrap_or_default()),
                format_temp(plan.min_c),
                format_temp(plan.max_c),
                format_temp(plan.max_c - plan.min_c)
            )
        })
        .collect()
}

fn flat_unscaled_dark_warning(frames: &MasterFrames) -> Option<String> {
    if frames.darks.is_empty() || frames.flats.is_empty() || !frames.bias.is_empty() || !frames.flat_darks.is_empty() {
        return None;
    }
    let dark_exp = median_exposure(&frames.darks)?;
    let flat_exp = median_exposure(&frames.flats)?;
    if dark_exp <= 0.0 || ((flat_exp - dark_exp) / dark_exp).abs() <= FLAT_VS_DARK_EXPOSURE_TOLERANCE {
        return None;
    }
    let excess = if dark_exp > flat_exp {
        format!(
            "{} s of thermal signal and amp glow are over-subtracted from the {} s flats (sub-percent for cooled cameras, up to a few percent in glow corners)",
            format_sig4(dark_exp - flat_exp),
            format_sig4(flat_exp)
        )
    } else {
        format!(
            "{} s of thermal signal and amp glow remain in the {} s flats (under-subtracted; sub-percent for cooled cameras, up to a few percent in glow corners)",
            format_sig4(flat_exp - dark_exp),
            format_sig4(flat_exp)
        )
    };
    Some(format!(
        "Flats are calibrated with the unscaled {} s light dark because there is no master bias and no flat-darks: the bias level is removed correctly, but {}. Add flat-dark frames of {} s or bias frames for an exact flat.",
        format_sig4(dark_exp),
        excess,
        format_sig4(flat_exp)
    ))
}

pub fn cfa_mixed_warning(channel: &str, lights: &[FrameCards]) -> Option<String> {
    let cfa = lights.iter().filter(|l| l.cfa).count();
    (cfa > 0 && cfa < lights.len()).then(|| {
        format!(
            "{}{} of {} lights carry a Bayer pattern and the others do not; cosmetic correction uses the CFA lattice for all of them.",
            channel_prefix(channel),
            cfa,
            lights.len()
        )
    })
}

pub fn check_master_frames(frames: &MasterFrames) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    for (kind, set) in [
        (MasterKind::Bias, &frames.bias),
        (MasterKind::Dark, &frames.darks),
        (MasterKind::Flat, &frames.flats),
        (MasterKind::FlatDark, &frames.flat_darks),
    ] {
        warnings.extend(check_master_set(kind, set)?.warnings);
    }
    warnings.extend(check_flat_darks_vs_flats(&frames.flat_darks, &frames.flats));
    warnings.extend(flat_unscaled_dark_warning(frames));
    warnings.extend(spread_warnings(&group_darks(&frames.darks)));
    Ok(warnings)
}

pub fn check_channel_lights(
    frames: &MasterFrames,
    channel: &str,
    lights: &[FrameCards],
    cosmetic_enabled: bool,
) -> Result<Vec<String>> {
    check_light_binning(
        channel,
        lights,
        &[
            (MasterKind::Bias, frames.bias.as_slice()),
            (MasterKind::Dark, frames.darks.as_slice()),
            (MasterKind::Flat, frames.flats.as_slice()),
            (MasterKind::FlatDark, frames.flat_darks.as_slice()),
        ],
    )?;
    let mut warnings = check_lights(channel, lights, &frames.bias, &frames.darks, &frames.flats);
    if cosmetic_enabled {
        warnings.extend(cfa_mixed_warning(channel, lights));
    }
    Ok(warnings)
}

fn first_largest<T>(items: &[T], frames: impl Fn(&T) -> usize) -> Option<usize> {
    items.iter().enumerate().rev().max_by_key(|(_, item)| frames(item)).map(|(i, _)| i)
}

pub fn largest_group(groups: &[DarkGroup]) -> Option<usize> {
    first_largest(groups, |g| g.frames)
}

pub fn build_calibration_masters(
    req: &MasterRequest,
    cancelled: CancelCheck,
) -> Result<(CalibrationMasters, MasterReport)> {
    let frames = read_master_frames(req);
    build_masters_from_frames(req, &frames, cancelled)
}

pub fn build_masters_from_frames(
    req: &MasterRequest,
    frames: &MasterFrames,
    cancelled: CancelCheck,
) -> Result<(CalibrationMasters, MasterReport)> {
    let warnings = check_master_frames(frames)?;

    let bias = match req.bias.is_empty() {
        true => None,
        false => Some(create_master_bias_cancellable(req.bias, cancelled).context("bias")?),
    };

    let mut dark_groups: Vec<DarkGroup> = Vec::new();
    for plan in group_darks(&frames.darks) {
        let paths: Vec<String> = plan.members.iter().map(|&i| req.darks[i].clone()).collect();
        let cards: Vec<FrameCards> = plan.members.iter().map(|&i| frames.darks[i].clone()).collect();
        let master = create_master_dark_cancellable(&paths, bias.as_ref(), cancelled).context("dark")?;
        dark_groups.push(DarkGroup {
            temp_c: plan.temp_c,
            exposure_s: median_exposure(&cards),
            frames: paths.len(),
            master,
        });
    }
    if let Some(first) = dark_groups.first() {
        let dims = first.master.dim();
        if let Some(odd) = dark_groups.iter().find(|g| g.master.dim() != dims) {
            anyhow::bail!(
                "dark: the {} C dark group is {:?} but the {} C group is {:?}; all darks must share one frame size",
                first.temp_c.map(format_temp).unwrap_or_default(),
                dims,
                odd.temp_c.map(format_temp).unwrap_or_default(),
                odd.master.dim()
            );
        }
    }
    let dark = largest_group(&dark_groups).map(|i| dark_groups[i].master.clone());

    let flat_dark = match req.flat_darks.is_empty() {
        true => None,
        false => Some(create_master_flat_dark_cancellable(req.flat_darks, cancelled).context("flat-dark")?),
    };

    let dark_exposure_s = median_exposure(&frames.darks);
    let flat = match req.flats.is_empty() {
        true => None,
        false => Some(
            create_master_flat_cancellable(
                req.flats,
                bias.as_ref(),
                dark.as_ref(),
                flat_dark.as_ref(),
                dark_exposure_s,
                cancelled,
            )
            .context("flat")?,
        ),
    };

    Ok((
        CalibrationMasters { dark, flat, bias, dark_groups },
        MasterReport {
            warnings,
            dark_exposure_s,
            bias_frames: req.bias.len(),
            dark_frames: req.darks.len(),
            flat_frames: req.flats.len(),
            flat_dark_frames: req.flat_darks.len(),
        },
    ))
}

const TEMP_DELTA_ADVICE: &str = "dark current changes ~2x per 7 °C, so enable dark optimisation or add darks near";

fn temperature_delta_warning(channel: &str, far: &[(&FrameCards, f64)], group_temp: f64) -> String {
    match far {
        [(light, light_temp)] => format!(
            "{}{} at {} °C uses the {} °C dark group (ΔT {} °C); {} {} °C.",
            channel_prefix(channel),
            file_label(&light.path),
            format_temp(*light_temp),
            format_temp(group_temp),
            format_temp((light_temp - group_temp).abs()),
            TEMP_DELTA_ADVICE,
            format_temp(*light_temp)
        ),
        _ => {
            let min = far.iter().map(|(_, t)| *t).fold(f64::INFINITY, f64::min);
            let max = far.iter().map(|(_, t)| *t).fold(f64::NEG_INFINITY, f64::max);
            let delta = far.iter().map(|(_, t)| (t - group_temp).abs()).fold(0.0, f64::max);
            format!(
                "{}{} lights at {} to {} °C use the {} °C dark group (ΔT up to {} °C); {} {} to {} °C.",
                channel_prefix(channel),
                far.len(),
                format_temp(min),
                format_temp(max),
                format_temp(group_temp),
                format_temp(delta),
                TEMP_DELTA_ADVICE,
                format_temp(min),
                format_temp(max)
            )
        }
    }
}

fn assign_by_temperature(
    groups: &[(Option<f64>, usize)],
    lights: &[FrameCards],
    channel: &str,
) -> (Vec<Option<usize>>, Vec<String>) {
    let Some(largest) = first_largest(groups, |g| g.1) else {
        return (vec![None; lights.len()], Vec::new());
    };
    let matching_enabled = groups.iter().any(|g| g.0.is_some());
    if !matching_enabled {
        return (vec![Some(largest); lights.len()], Vec::new());
    }
    let mut far: Vec<Vec<(&FrameCards, f64)>> = vec![Vec::new(); groups.len()];
    let mut unknown = 0usize;
    let assigned: Vec<Option<usize>> = lights
        .iter()
        .map(|light| {
            let Some(light_temp) = light.temp_c else {
                unknown += 1;
                return Some(largest);
            };
            let nearest = groups
                .iter()
                .enumerate()
                .filter_map(|(i, g)| g.0.map(|t| (i, t)))
                .min_by(|(_, a), (_, b)| (light_temp - a).abs().total_cmp(&(light_temp - b).abs()))
                .map(|(i, _)| i)
                .unwrap_or(largest);
            if let Some(group_temp) = groups[nearest].0 {
                if (light_temp - group_temp).abs() > TEMP_DELTA_WARN_C {
                    far[nearest].push((light, light_temp));
                }
            }
            Some(nearest)
        })
        .collect();
    let mut warnings: Vec<String> = far
        .iter()
        .zip(groups)
        .filter_map(|(lights, group)| {
            let group_temp = group.0.filter(|_| !lights.is_empty())?;
            Some(temperature_delta_warning(channel, lights, group_temp))
        })
        .collect();
    if unknown > 0 && groups.len() > 1 {
        warnings.push(format!(
            "{}{} lights have no CCD-TEMP/SET-TEMP; they use the {} °C dark group ({} frames).",
            channel_prefix(channel),
            unknown,
            groups[largest].0.map(format_temp).unwrap_or_else(|| "unknown".to_string()),
            groups[largest].1
        ));
    }
    (assigned, warnings)
}

pub fn assign_dark_groups(groups: &[DarkGroup], lights: &[FrameCards], channel: &str) -> (Vec<Option<usize>>, Vec<String>) {
    let sizes: Vec<(Option<f64>, usize)> = groups.iter().map(|g| (g.temp_c, g.frames)).collect();
    assign_by_temperature(&sizes, lights, channel)
}

pub fn dark_group_warnings(frames: &MasterFrames, lights: &[FrameCards], channel: &str) -> Vec<String> {
    let sizes: Vec<(Option<f64>, usize)> =
        group_darks(&frames.darks).iter().map(|p| (p.temp_c, p.members.len())).collect();
    assign_by_temperature(&sizes, lights, channel).1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::imaging::calibration_pipeline::{calibrate_light, calibrate_light_with_cosmetic};
    use crate::core::stacking::calibration::create_master_dark;
    use crate::core::stacking::frame_cards::frame_cards_from_header;
    use crate::core::stacking::never_cancelled;
    use crate::types::header::HduHeader;
    use ndarray::Array2;

    fn write_frame(dir: &tempfile::TempDir, name: &str, frame: &Array2<f32>, cards: &[(&str, &str)]) -> String {
        let mut header = HduHeader::empty();
        for (key, value) in cards {
            header.set(key, (*value).to_string());
        }
        let path = dir.path().join(name).to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&path, frame, Some(&header)).unwrap();
        path
    }

    fn constant(dir: &tempfile::TempDir, name: &str, value: f32, cards: &[(&str, &str)]) -> String {
        write_frame(dir, name, &Array2::from_elem((4, 4), value), cards)
    }

    fn two_level_flat() -> Array2<f32> {
        Array2::from_shape_fn((4, 4), |(_, x)| if x < 2 { 1000.0 } else { 2000.0 })
    }

    fn dark_at(dir: &tempfile::TempDir, i: usize, temp: f64) -> String {
        constant(dir, &format!("dark{i}.fits"), 500.0 + i as f32, &[("EXPTIME", "300"), ("CCD-TEMP", &format!("{temp:.2}"))])
    }

    fn light_cards(path: &str, temp: Option<f64>, cfa: bool) -> FrameCards {
        let mut header = HduHeader::empty();
        header.set("EXPTIME", "120".to_string());
        if let Some(t) = temp {
            header.set("CCD-TEMP", format!("{t:.2}"));
        }
        if cfa {
            header.set("BAYERPAT", "'RGGB'".to_string());
        }
        frame_cards_from_header(path, &header)
    }

    fn only_darks(darks: &[String]) -> MasterRequest<'_> {
        MasterRequest { bias: &[], darks, flats: &[], flat_darks: &[] }
    }

    #[test]
    fn flat_dark_replaces_bias_and_scaled_dark() {
        let dir = tempfile::tempdir().unwrap();
        let flat = two_level_flat();
        let flats = [
            write_frame(&dir, "flat0.fits", &flat, &[("EXPTIME", "2")]),
            write_frame(&dir, "flat1.fits", &flat, &[("EXPTIME", "2")]),
        ];
        let flat_darks = [
            constant(&dir, "fd0.fits", 100.0, &[("EXPTIME", "2")]),
            constant(&dir, "fd1.fits", 100.0, &[("EXPTIME", "2")]),
        ];
        let darks = [
            constant(&dir, "d0.fits", 600.0, &[("EXPTIME", "300")]),
            constant(&dir, "d1.fits", 600.0, &[("EXPTIME", "300")]),
        ];
        let req = MasterRequest { bias: &[], darks: &darks, flats: &flats, flat_darks: &flat_darks };
        let (masters, report) = build_calibration_masters(&req, &never_cancelled).unwrap();
        let master = masters.flat.expect("master flat");
        let ratio = master[[0, 3]] / master[[0, 0]];
        let expected = (2000.0 - 100.0) / (1000.0 - 100.0);
        assert!((ratio - expected).abs() < 1e-4, "ratio {ratio}, expected {expected}");
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(
            (report.bias_frames, report.dark_frames, report.flat_frames, report.flat_dark_frames),
            (0, 2, 2, 2)
        );
        assert_eq!(report.dark_exposure_s, Some(300.0));
        assert_eq!(masters.dark_groups.len(), 1);
        assert_eq!((masters.dark_groups[0].frames, masters.dark_groups[0].temp_c), (2, None));
        assert_eq!(masters.dark_groups[0].exposure_s, Some(300.0));
    }

    #[test]
    fn flats_with_unscaled_light_dark_and_no_bias_warn() {
        let dir = tempfile::tempdir().unwrap();
        let flat = two_level_flat();
        let flats = [write_frame(&dir, "flat0.fits", &flat, &[("EXPTIME", "2")])];
        let darks = [
            constant(&dir, "d0.fits", 600.0, &[("EXPTIME", "300")]),
            constant(&dir, "d1.fits", 600.0, &[("EXPTIME", "300")]),
        ];
        let req = MasterRequest { bias: &[], darks: &darks, flats: &flats, flat_darks: &[] };
        let (masters, report) = build_calibration_masters(&req, &never_cancelled).unwrap();
        let master = masters.flat.expect("master flat");
        let ratio = master[[0, 3]] / master[[0, 0]];
        let expected = (2000.0 - 600.0) / (1000.0 - 600.0);
        assert!((ratio - expected).abs() < 1e-4, "ratio {ratio}, expected {expected}");
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        let warning = &report.warnings[0];
        assert!(warning.contains("unscaled 300 s light dark"), "{warning}");
        assert!(warning.contains("298 s of thermal signal"), "{warning}");
        assert!(warning.contains("from the 2 s flats"), "{warning}");
        assert!(warning.ends_with("Add flat-dark frames of 2 s or bias frames for an exact flat."), "{warning}");

        let long_flats = [write_frame(&dir, "flat300.fits", &flat, &[("EXPTIME", "300")])];
        let req = MasterRequest { bias: &[], darks: &darks, flats: &long_flats, flat_darks: &[] };
        let (_, report) = build_calibration_masters(&req, &never_cancelled).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let short_darks = [
            constant(&dir, "d10a.fits", 600.0, &[("EXPTIME", "10")]),
            constant(&dir, "d10b.fits", 600.0, &[("EXPTIME", "10")]),
        ];
        let flats30 = [write_frame(&dir, "flat30.fits", &flat, &[("EXPTIME", "30")])];
        let req = MasterRequest { bias: &[], darks: &short_darks, flats: &flats30, flat_darks: &[] };
        let (_, report) = build_calibration_masters(&req, &never_cancelled).unwrap();
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        let warning = &report.warnings[0];
        assert!(warning.contains("unscaled 10 s light dark"), "{warning}");
        assert!(warning.contains("20 s of thermal signal and amp glow remain in the 30 s flats"), "{warning}");
        assert!(!warning.contains("-20") && !warning.contains("over-subtracted"), "{warning}");
        assert!(warning.ends_with("Add flat-dark frames of 30 s or bias frames for an exact flat."), "{warning}");
    }

    #[test]
    fn mismatched_master_dims_are_refused_before_subtraction() {
        let dir = tempfile::tempdir().unwrap();
        let big = |name: &str, value: f32, cards: &[(&str, &str)]| {
            write_frame(&dir, name, &Array2::from_elem((8, 8), value), cards)
        };
        let flats = [big("flat0.fits", 1000.0, &[("EXPTIME", "2")]), big("flat1.fits", 1000.0, &[("EXPTIME", "2")])];
        let flat_darks = [
            constant(&dir, "fd0.fits", 100.0, &[("EXPTIME", "2")]),
            constant(&dir, "fd1.fits", 100.0, &[("EXPTIME", "2")]),
        ];
        let req = MasterRequest { bias: &[], darks: &[], flats: &flats, flat_darks: &flat_darks };
        let err = format!("{:#}", build_calibration_masters(&req, &never_cancelled).unwrap_err());
        assert!(err.contains("flat-dark") && err.contains("(4, 4)") && err.contains("(8, 8)"), "{err}");

        let darks = [big("d0.fits", 600.0, &[("EXPTIME", "300")]), big("d1.fits", 600.0, &[("EXPTIME", "300")])];
        let bias = [constant(&dir, "b0.fits", 100.0, &[]), constant(&dir, "b1.fits", 100.0, &[])];
        let req = MasterRequest { bias: &bias, darks: &darks, flats: &[], flat_darks: &[] };
        let err = format!("{:#}", build_calibration_masters(&req, &never_cancelled).unwrap_err());
        assert!(err.contains("bias") && err.contains("(4, 4)") && err.contains("(8, 8)"), "{err}");
    }

    #[test]
    fn temperature_delta_warnings_are_aggregated_per_group() {
        let group = |temp: f64| DarkGroup {
            temp_c: Some(temp),
            exposure_s: Some(300.0),
            frames: 5,
            master: Array2::from_elem((4, 4), 1.0),
        };
        let groups = [group(-20.0), group(-10.0)];
        let lights = [
            light_cards("a.fits", Some(-14.0), false),
            light_cards("cold.fits", Some(-17.0), false),
            light_cards("b.fits", Some(-13.0), false),
            light_cards("near.fits", Some(-10.5), false),
        ];
        let (assigned, warnings) = assign_dark_groups(&groups, &lights, "R");
        assert_eq!(assigned, vec![Some(1), Some(0), Some(1), Some(1)]);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings[0].starts_with("Channel 'R': cold.fits at -17.0 °C uses the -20.0 °C dark group (ΔT 3.0 °C)"),
            "{}",
            warnings[0]
        );
        assert_eq!(
            warnings[1],
            "Channel 'R': 2 lights at -14.0 to -13.0 °C use the -10.0 °C dark group (ΔT up to 4.0 °C); dark current changes ~2x per 7 °C, so enable dark optimisation or add darks near -14.0 to -13.0 °C."
        );

        let dark = |i: usize, temp: f64| light_cards(&format!("d{i}.fits"), Some(temp), false);
        let darks: Vec<FrameCards> = (0..5).map(|i| dark(i, -20.0)).chain((5..10).map(|i| dark(i, -10.0))).collect();
        let frames = MasterFrames { darks, ..Default::default() };
        assert_eq!(dark_group_warnings(&frames, &lights, "R"), warnings);
    }

    #[test]
    fn darks_split_only_at_gaps_above_three_degrees() {
        let dir = tempfile::tempdir().unwrap();
        let temps = [-20.2, -19.6, -19.9, -20.4, -19.8, -10.1, -9.8, -10.3, -9.9, -10.0];
        let darks: Vec<String> = temps.iter().enumerate().map(|(i, &t)| dark_at(&dir, i, t)).collect();
        let (masters, report) = build_calibration_masters(&only_darks(&darks), &never_cancelled).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(masters.dark_groups.len(), 2, "{:?}", masters.dark_groups.iter().map(|g| g.temp_c).collect::<Vec<_>>());
        let means: Vec<String> = masters.dark_groups.iter().map(|g| format_temp(g.temp_c.unwrap())).collect();
        assert_eq!(means, vec!["-20.0".to_string(), "-10.0".to_string()]);
        assert_eq!((masters.dark_groups[0].frames, masters.dark_groups[1].frames), (5, 5));
        assert_eq!(masters.dark_groups[0].exposure_s, Some(300.0));
        assert_eq!(masters.dark.as_ref(), Some(&masters.dark_groups[0].master));
        assert_ne!(masters.dark_groups[0].master, masters.dark_groups[1].master);

        let lights = [
            light_cards("warm.fits", Some(-9.0), false),
            light_cards("mid.fits", Some(-14.0), false),
            light_cards("none.fits", None, false),
        ];
        let (assigned, warnings) = assign_dark_groups(&masters.dark_groups, &lights, "R");
        assert_eq!(assigned, vec![Some(1), Some(1), Some(0)]);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings[0].starts_with("Channel 'R': mid.fits at -14.0 °C uses the -10.0 °C dark group (ΔT 4.0 °C); dark current changes ~2x per 7 °C"),
            "{}",
            warnings[0]
        );
        assert!(warnings[0].ends_with("add darks near -14.0 °C."), "{}", warnings[0]);
        assert_eq!(
            warnings[1],
            "Channel 'R': 1 lights have no CCD-TEMP/SET-TEMP; they use the -20.0 °C dark group (5 frames)."
        );
    }

    #[test]
    fn drifting_temperatures_give_one_group_and_a_spread_warning() {
        let dir = tempfile::tempdir().unwrap();
        let darks: Vec<String> = (0..20).map(|i| dark_at(&dir, i, -3.0 + 0.45 * i as f64)).collect();
        let (masters, report) = build_calibration_masters(&only_darks(&darks), &never_cancelled).unwrap();
        assert_eq!(masters.dark_groups.len(), 1);
        assert_eq!(masters.dark_groups[0].frames, 20);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].starts_with("Darks in the "), "{}", report.warnings[0]);
        assert!(report.warnings[0].contains("span -3.0 to 5.6 °C"), "{}", report.warnings[0]);
        assert!(report.warnings[0].contains("one master is built from all of them"), "{}", report.warnings[0]);
        let expected = create_master_dark(&darks, None).unwrap();
        assert_eq!(masters.dark.as_ref(), Some(&expected));

        let lights = [light_cards("l0.fits", Some(1.0), false), light_cards("l1.fits", None, false)];
        let (assigned, warnings) = assign_dark_groups(&masters.dark_groups, &lights, "L");
        assert_eq!(assigned, vec![Some(0), Some(0)]);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn small_groups_merge_into_their_neighbour() {
        let dir = tempfile::tempdir().unwrap();
        let mut darks: Vec<String> = (0..12).map(|i| dark_at(&dir, i, -20.0)).collect();
        darks.extend((12..14).map(|i| dark_at(&dir, i, -12.0)));
        let (masters, report) = build_calibration_masters(&only_darks(&darks), &never_cancelled).unwrap();
        assert_eq!(masters.dark_groups.len(), 1);
        assert_eq!(masters.dark_groups[0].frames, 14);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].contains("span -20.0 to -12.0 °C (8.0 °C)"), "{}", report.warnings[0]);

        let cards: Vec<FrameCards> = darks.iter().map(|p| read_frame_cards(p)).collect();
        let plans = group_darks(&cards);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].members, (0..14).collect::<Vec<_>>());

        let three_way = [
            light_cards("a.fits", Some(-20.0), false),
            light_cards("b.fits", Some(-20.1), false),
            light_cards("c.fits", Some(-16.0), false),
            light_cards("d.fits", Some(-12.0), false),
            light_cards("e.fits", Some(-12.2), false),
            light_cards("f.fits", Some(-4.0), false),
            light_cards("g.fits", Some(-4.1), false),
            light_cards("h.fits", Some(-4.2), false),
            light_cards("i.fits", Some(-3.9), false),
            light_cards("j.fits", Some(-4.3), false),
        ];
        let plans = group_darks(&three_way);
        assert_eq!(plans.len(), 2, "{plans:?}");
        assert_eq!(plans[0].members, vec![0, 1, 2, 3, 4]);
        assert_eq!(plans[1].members, vec![5, 6, 7, 8, 9]);
    }

    #[test]
    fn calibrated_light_uses_its_own_dark_group() {
        let group = |value: f32, temp: f64| DarkGroup {
            temp_c: Some(temp),
            exposure_s: Some(300.0),
            frames: 5,
            master: Array2::from_elem((4, 4), value),
        };
        let masters = CalibrationMasters {
            dark: Some(Array2::from_elem((4, 4), 100.0)),
            flat: None,
            bias: None,
            dark_groups: vec![group(100.0, -20.0), group(200.0, -10.0)],
        };
        let light = Array2::from_elem((4, 4), 1000.0f32);
        let own = calibrate_light(&light, &masters, 1.0, Some(1));
        assert_eq!(own, Array2::from_elem((4, 4), 800.0));
        let default = calibrate_light(&light, &masters, 1.0, None);
        assert_eq!(default, Array2::from_elem((4, 4), 900.0));
        let (with_cosmetic, replaced) = calibrate_light_with_cosmetic(&light, &masters, 1.0, Some(1), None);
        assert_eq!((with_cosmetic, replaced), (own, 0));
        assert_eq!(calibrate_light(&light, &masters, 1.0, Some(7)), default);
        assert_eq!(largest_group(&masters.dark_groups), Some(0));
    }

    #[test]
    fn channel_checks_refuse_binning_mismatch_and_warn_on_mixed_cfa() {
        let mut dark = light_cards("d.fits", None, false);
        dark.xbinning = Some(2);
        dark.ybinning = Some(2);
        let frames = MasterFrames { darks: vec![dark], ..Default::default() };
        let mut light = light_cards("l.fits", None, false);
        light.xbinning = Some(1);
        light.ybinning = Some(1);
        let err = check_channel_lights(&frames, "R", &[light], false).unwrap_err().to_string();
        assert!(err.contains("binned 1x1 but the dark frames are binned 2x2"), "{err}");

        let lights = [light_cards("a.fits", None, true), light_cards("b.fits", None, false), light_cards("c.fits", None, true)];
        let warnings = check_channel_lights(&MasterFrames::default(), "R", &lights, true).unwrap();
        assert_eq!(
            warnings,
            vec!["Channel 'R': 2 of 3 lights carry a Bayer pattern and the others do not; cosmetic correction uses the CFA lattice for all of them.".to_string()]
        );
        assert!(check_channel_lights(&MasterFrames::default(), "R", &lights, false).unwrap().is_empty());
        assert!(check_channel_lights(&MasterFrames::default(), "R", &lights[..1], true).unwrap().is_empty());
    }
}
