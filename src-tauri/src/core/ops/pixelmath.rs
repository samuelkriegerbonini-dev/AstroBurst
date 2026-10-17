use anyhow::{anyhow, bail, Context};
use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::imaging::stats::finite_output_stats;
use crate::core::pixelmath::{compile, evaluate, OutputOptions, PixelMathError};
use crate::infra::cache::ImageEntry;
use crate::types::image::ImageStats;

pub const TARGET_SYMBOL: &str = "$T";

const ZERO_FRACTION_WARNING: f64 = 0.05;
const CHANNEL_LABELS: [&str; 3] = ["R", "G", "B"];

pub type SlotLoader<'a> = &'a (dyn Fn(&str) -> anyhow::Result<ImageEntry> + Sync);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PixelMathSlot {
    pub name: String,
    #[serde(rename = "ref", alias = "path")]
    pub path: String,
}

pub fn pixelmath_error(err: PixelMathError) -> anyhow::Error {
    match err.position {
        Some(position) => anyhow!("pixelmath: {} (position {})", err.message, position),
        None => anyhow!("pixelmath: {}", err.message),
    }
}

pub fn slot_names_with_target(names: &[String]) -> Vec<String> {
    std::iter::once(TARGET_SYMBOL.to_string())
        .chain(names.iter().cloned())
        .collect()
}

fn exact_zero_fraction(arr: &Array2<f32>) -> f64 {
    let (finite, zeros) = arr.iter().fold((0usize, 0usize), |(finite, zeros), &v| {
        if v.is_finite() {
            (finite + 1, zeros + usize::from(v == 0.0))
        } else {
            (finite, zeros)
        }
    });
    if finite == 0 {
        0.0
    } else {
        zeros as f64 / finite as f64
    }
}

fn plane_count(entry: &ImageEntry) -> Option<i64> {
    entry
        .header()
        .and_then(|h| h.get_i64("NAXIS3"))
        .filter(|&n| n > 1)
}

pub fn entry_warnings(label: &str, entry: &ImageEntry) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(planes) = plane_count(entry) {
        warnings.push(format!(
            "{} is plane 1 of {}; the other planes were not read",
            label, planes
        ));
    }
    if entry.plane_info().is_some_and(|info| info.is_dq) {
        warnings.push(format!(
            "{} is a data-quality plane read as 32-bit float; flag values above 16777216 lose their low bits, so only zero/non-zero tests are exact",
            label
        ));
    }
    warnings
}

pub fn padding_warning(label: &str, arr: &Array2<f32>, reduced: bool) -> Option<String> {
    if !reduced {
        return None;
    }
    let zeros = exact_zero_fraction(arr);
    if zeros < ZERO_FRACTION_WARNING {
        return None;
    }
    Some(format!(
        "{} is {:.0}% exact zeros, the value mosaics use as padding; med, mean, sdev, mdev, adev, min and max of {} include those pixels",
        label,
        zeros * 100.0,
        label
    ))
}

pub struct ChannelPixelMath {
    pub results: [Array2<f32>; 3],
    pub stats: [Option<ImageStats>; 3],
    pub non_finite: [usize; 3],
    pub warnings: Vec<String>,
}

pub fn evaluate_per_channel(
    expression: &str,
    slots: &[PixelMathSlot],
    opts: &OutputOptions,
    channels: [&Array2<f32>; 3],
    load: SlotLoader<'_>,
) -> anyhow::Result<ChannelPixelMath> {
    let user_names: Vec<String> = slots.iter().map(|s| s.name.clone()).collect();
    let slot_names = slot_names_with_target(&user_names);
    let program = compile(expression, &slot_names).map_err(pixelmath_error)?;
    let referenced = program.referenced_slots();
    let reduced = program.reduced_slots();
    let (rows, cols) = channels[0].dim();

    let mut entries: Vec<Option<ImageEntry>> = Vec::with_capacity(slots.len());
    let mut warnings: Vec<String> = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        if !referenced[i + 1] {
            entries.push(None);
            continue;
        }
        let entry = load(&slot.path)
            .with_context(|| format!("slot {} = {}", slot.name, slot.path))?;
        let (slot_rows, slot_cols) = entry.arr().dim();
        if (slot_rows, slot_cols) != (rows, cols) {
            bail!(
                "pixelmath: slot {} is {}x{} but the composite is {}x{} (width x height)",
                slot.name,
                slot_cols,
                slot_rows,
                cols,
                rows
            );
        }
        warnings.extend(entry_warnings(&slot.name, &entry));
        warnings.extend(padding_warning(&slot.name, entry.arr(), reduced[i + 1]));
        entries.push(Some(entry));
    }

    let mut results = Vec::with_capacity(3);
    let mut stats = Vec::with_capacity(3);
    let mut non_finite = Vec::with_capacity(3);
    for (channel, label) in channels.into_iter().zip(CHANNEL_LABELS) {
        warnings.extend(padding_warning(&format!("{} ({})", TARGET_SYMBOL, label), channel, reduced[0]));
        let arrays: Vec<&Array2<f32>> = std::iter::once(channel)
            .chain(entries.iter().map(|e| e.as_ref().map(|e| e.arr()).unwrap_or(channel)))
            .collect();
        let result = evaluate(&program, &arrays, opts).map_err(pixelmath_error)?;
        let (channel_stats, channel_non_finite) = finite_output_stats(&result);
        results.push(result);
        stats.push(channel_stats);
        non_finite.push(channel_non_finite);
    }

    let [r, g, b] = <[Array2<f32>; 3]>::try_from(results).map_err(|_| anyhow!("pixelmath: expected three channels"))?;
    let [sr, sg, sb] = <[Option<ImageStats>; 3]>::try_from(stats).map_err(|_| anyhow!("pixelmath: expected three channels"))?;
    let [nr, ng, nb] = <[usize; 3]>::try_from(non_finite).map_err(|_| anyhow!("pixelmath: expected three channels"))?;
    Ok(ChannelPixelMath {
        results: [r, g, b],
        stats: [sr, sg, sb],
        non_finite: [nr, ng, nb],
        warnings,
    })
}

pub struct TargetPixelMath {
    pub result: Array2<f32>,
    pub referenced: Vec<bool>,
    pub slots: Vec<Option<ImageEntry>>,
    pub warnings: Vec<String>,
}

impl TargetPixelMath {
    pub fn referenced_inputs<'a>(&'a self, target: &'a Array2<f32>) -> Vec<&'a Array2<f32>> {
        std::iter::once(target)
            .chain(self.slots.iter().map(|e| e.as_ref().map(|e| e.arr()).unwrap_or(target)))
            .zip(&self.referenced)
            .filter(|(_, used)| **used)
            .map(|(arr, _)| arr)
            .collect()
    }
}

pub fn evaluate_target(
    expression: &str,
    slots: &[PixelMathSlot],
    opts: &OutputOptions,
    target: &Array2<f32>,
    load: SlotLoader<'_>,
) -> anyhow::Result<TargetPixelMath> {
    let user_names: Vec<String> = slots.iter().map(|s| s.name.clone()).collect();
    let slot_names = slot_names_with_target(&user_names);
    let program = compile(expression, &slot_names).map_err(pixelmath_error)?;
    let referenced = program.referenced_slots();
    let reduced = program.reduced_slots();

    let mut entries: Vec<Option<ImageEntry>> = Vec::with_capacity(slots.len());
    for (i, slot) in slots.iter().enumerate() {
        if !referenced[i + 1] {
            entries.push(None);
            continue;
        }
        let entry = load(&slot.path)
            .with_context(|| format!("slot {} = {}", slot.name, slot.path))?;
        entries.push(Some(entry));
    }

    let mut warnings: Vec<String> = Vec::new();
    warnings.extend(padding_warning(TARGET_SYMBOL, target, reduced.first().copied().unwrap_or(false)));
    for (i, entry) in entries.iter().enumerate() {
        let Some(entry) = entry else { continue };
        warnings.extend(entry_warnings(&slot_names[i + 1], entry));
        warnings.extend(padding_warning(&slot_names[i + 1], entry.arr(), reduced.get(i + 1).copied().unwrap_or(false)));
    }

    let arrays: Vec<&Array2<f32>> = std::iter::once(target)
        .chain(entries.iter().map(|e| e.as_ref().map(|e| e.arr()).unwrap_or(target)))
        .collect();
    let result = evaluate(&program, &arrays, opts).map_err(pixelmath_error)?;
    Ok(TargetPixelMath { result, referenced, slots: entries, warnings })
}
