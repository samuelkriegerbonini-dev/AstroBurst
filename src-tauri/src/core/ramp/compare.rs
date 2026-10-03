use std::fs::File;
use std::ops::Range;

use anyhow::{bail, Context, Result};
use ndarray::{s, Array2, ArrayView2, Axis, Zip};
use serde::Serialize;

use crate::core::ramp::irs2::Irs2Layout;
use crate::core::ramp::quick_slope::{DQ_DO_NOT_USE, DQ_JUMP_DET, DQ_SATURATED};
use crate::infra::fits::reader::{extract_image_mmap_by_index, extract_int_plane_by_index, list_extensions, read_primary_header, HduInfo};
use crate::math::median::{f32_cmp, f64_cmp};

pub const RATE_BIN_EDGES: [f64; 8] = [f64::NEG_INFINITY, 0.05, 0.3, 1.0, 3.0, 10.0, 30.0, f64::INFINITY];
pub const FAINT_BIN_COUNT: usize = 2;
pub const RATIO_MIN_RATE: f32 = 0.3;
pub const ZERO_SHIFT_MAX_DY: usize = 8;
pub const ZERO_SHIFT_MIN_CORR: f64 = 0.99;
pub const ROW_PROFILE_PERCENTILE: f64 = 0.90;
pub const ROW_PROFILE_MIN_PIXELS: usize = 16;
pub const SATURATED_RULE: &str = "all_groups (stcal 1.15.2)";
pub const HISTOGRAM_BINS: usize = 80;
pub const REL_HIST_RANGE: f64 = 0.2;
pub const REL_HIST_MIN_RATE: f32 = 1.0;
pub const DELTA_HIST_RANGE: f64 = 0.5;
pub const DELTA_HIST_MAX_RATE: f32 = 0.05;
pub const PEARSON_MIN_ROWS: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct CompareBin {
    pub lo: f64,
    pub hi: f64,
    pub n: usize,
    pub median_rate: f64,
    pub median_err: f64,
    pub median_delta: f64,
    pub median_rel: f64,
    pub p16_delta: f64,
    pub p84_delta: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AmpBins {
    pub amplifier: usize,
    pub science_rows: (usize, usize),
    pub bins: Vec<CompareBin>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ShiftCheck {
    pub best_dy: i64,
    pub corr_at_zero: f64,
    pub corr_best: f64,
    pub second_dy: i64,
    pub second_corr: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct FlagConfusion {
    pub tp: usize,
    pub fp: usize,
    pub fn_: usize,
    pub recall: f64,
    pub precision: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Histogram {
    pub edges: Vec<f64>,
    pub counts: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RateComparison {
    pub detector: Option<String>,
    pub good_pixels: usize,
    pub quick_flagged_in_good: usize,
    pub bins: Vec<CompareBin>,
    pub amp_bins: Vec<AmpBins>,
    pub zero_shift: ShiftCheck,
    pub jump: FlagConfusion,
    pub saturated: FlagConfusion,
    pub saturated_rule: &'static str,
    pub saturated_any_group: FlagConfusion,
    pub rel_hist: Histogram,
    pub delta_hist: Histogram,
    pub science_rows: Option<(usize, usize)>,
}

pub struct OfficialRate {
    pub sci: Array2<f32>,
    pub err: Option<Array2<f32>>,
    pub dq: Array2<u32>,
    pub detector: Option<String>,
}

pub struct QuickSlopePlanes {
    pub sci: Array2<f32>,
    pub dq: Array2<u32>,
    pub ngood: Array2<i32>,
}

fn hdu_named(hdus: &[HduInfo], name: &str) -> Option<usize> {
    hdus.iter()
        .find(|h| h.extname.as_deref().is_some_and(|n| n.trim().trim_matches('\'').trim().eq_ignore_ascii_case(name)))
        .map(|h| h.index)
}

fn required_hdu(hdus: &[HduInfo], name: &str, path: &str) -> Result<usize> {
    hdu_named(hdus, name).with_context(|| format!("{path} has no {name} extension"))
}

fn check_plane_shape(name: &str, shape: (usize, usize), reference: (usize, usize), path: &str) -> Result<()> {
    if shape != reference {
        bail!("{path}: {name} is {} x {} but SCI is {} x {}", shape.0, shape.1, reference.0, reference.1);
    }
    Ok(())
}

pub fn read_official_rate(path: &str) -> Result<OfficialRate> {
    let file = File::open(path).with_context(|| format!("Failed to open {path}"))?;
    let hdus = list_extensions(&file)?;
    let sci = extract_image_mmap_by_index(&file, required_hdu(&hdus, "SCI", path)?)?.image;
    let err = match hdu_named(&hdus, "ERR") {
        Some(index) => Some(extract_image_mmap_by_index(&file, index)?.image),
        None => None,
    };
    let dq = extract_int_plane_by_index(&file, required_hdu(&hdus, "DQ", path)?)?.bits;
    check_plane_shape("DQ", dq.dim(), sci.dim(), path)?;
    if let Some(err) = &err {
        check_plane_shape("ERR", err.dim(), sci.dim(), path)?;
    }
    let detector = read_primary_header(path)?
        .get("DETECTOR")
        .map(|d| d.trim().trim_matches('\'').trim().to_string())
        .filter(|d| !d.is_empty());
    Ok(OfficialRate { sci, err, dq, detector })
}

pub fn read_qslope(path: &str) -> Result<QuickSlopePlanes> {
    let file = File::open(path).with_context(|| format!("Failed to open {path}"))?;
    let hdus = list_extensions(&file)?;
    let sci = extract_image_mmap_by_index(&file, required_hdu(&hdus, "SCI", path)?)?.image;
    let dq = extract_int_plane_by_index(&file, required_hdu(&hdus, "DQ", path)?)?.bits;
    let ngood = extract_int_plane_by_index(&file, required_hdu(&hdus, "NGOOD", path)?)?.bits.mapv(|v| v as i32);
    check_plane_shape("DQ", dq.dim(), sci.dim(), path)?;
    check_plane_shape("NGOOD", ngood.dim(), sci.dim(), path)?;
    Ok(QuickSlopePlanes { sci, dq, ngood })
}

fn nearest_rank(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((p * sorted.len() as f64).floor() as usize).min(sorted.len() - 1)]
}

fn sorted_values(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(f64_cmp);
    values
}

fn nearest_rank_f32(sorted: &[f32], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((p * sorted.len() as f64).floor() as usize).min(sorted.len() - 1)] as f64
}

fn sorted_f32(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(f32_cmp);
    values
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < PEARSON_MIN_ROWS {
        return f64::NAN;
    }
    let mean_a = a[..n].iter().sum::<f64>() / n as f64;
    let mean_b = b[..n].iter().sum::<f64>() / n as f64;
    let (mut cov, mut var_a, mut var_b) = (0.0, 0.0, 0.0);
    for (x, y) in a[..n].iter().zip(&b[..n]) {
        let (dx, dy) = (x - mean_a, y - mean_b);
        cov += dx * dy;
        var_a += dx * dx;
        var_b += dy * dy;
    }
    if var_a <= 0.0 || var_b <= 0.0 {
        return f64::NAN;
    }
    cov / (var_a * var_b).sqrt()
}

pub fn row_profile(img: ArrayView2<f32>, good: ArrayView2<bool>) -> Vec<f64> {
    img.axis_iter(Axis(0))
        .zip(good.axis_iter(Axis(0)))
        .map(|(row, ok)| {
            let values: Vec<f64> = row.iter().zip(ok.iter()).filter(|(_, g)| **g).map(|(v, _)| *v as f64).collect();
            if values.len() < ROW_PROFILE_MIN_PIXELS {
                return f64::NAN;
            }
            nearest_rank(&sorted_values(values), ROW_PROFILE_PERCENTILE)
        })
        .collect()
}

pub fn zero_shift_check(quick: ArrayView2<f32>, official: ArrayView2<f32>, official_dq: ArrayView2<u32>, max_dy: usize) -> ShiftCheck {
    let good = Zip::from(&quick)
        .and(&official)
        .and(&official_dq)
        .map_collect(|q, o, d| *d == 0 && q.is_finite() && o.is_finite());
    let quick_profile = row_profile(quick, good.view());
    let official_profile = row_profile(official, good.view());
    let rows = quick_profile.len() as i64;
    let max_dy = max_dy as i64;
    let correlations: Vec<(i64, f64)> = (-max_dy..=max_dy)
        .map(|dy| {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            for y in 0..rows {
                let shifted = y + dy;
                if shifted < 0 || shifted >= rows {
                    continue;
                }
                let (q, r) = (quick_profile[y as usize], official_profile[shifted as usize]);
                if q.is_finite() && r.is_finite() {
                    a.push(q);
                    b.push(r);
                }
            }
            (dy, pearson(&a, &b))
        })
        .collect();
    let corr_at_zero = correlations.iter().find(|(dy, _)| *dy == 0).map_or(f64::NAN, |(_, c)| *c);
    let mut ranked: Vec<(i64, f64)> = correlations.into_iter().filter(|(_, c)| c.is_finite()).collect();
    ranked.sort_by(|a, b| f64_cmp(&b.1, &a.1).then(a.0.abs().cmp(&b.0.abs())));
    let (best_dy, corr_best) = ranked.first().copied().unwrap_or((0, f64::NAN));
    let (second_dy, second_corr) = ranked.get(1).copied().unwrap_or((0, f64::NAN));
    let passed = best_dy == 0 && corr_at_zero.is_finite() && corr_at_zero >= ZERO_SHIFT_MIN_CORR;
    ShiftCheck { best_dy, corr_at_zero, corr_best, second_dy, second_corr, passed }
}

struct WindowedPair<'a> {
    quick_sci: ArrayView2<'a, f32>,
    quick_dq: ArrayView2<'a, u32>,
    quick_ngood: ArrayView2<'a, i32>,
    official_sci: ArrayView2<'a, f32>,
    official_err: Option<ArrayView2<'a, f32>>,
    official_dq: ArrayView2<'a, u32>,
    good: Array2<bool>,
}

fn is_faint_bin(hi: f64) -> bool {
    hi <= RATE_BIN_EDGES[FAINT_BIN_COUNT]
}

fn bin_index(edges: &[f64], rate: f64) -> Option<usize> {
    edges.windows(2).position(|w| rate >= w[0] && rate < w[1])
}

struct BinSamples {
    deltas: Vec<f32>,
    rates: Vec<f32>,
    errs: Vec<f32>,
    rels: Vec<f32>,
}

impl BinSamples {
    fn with_capacity(n: usize, keep_err: bool, keep_rel: bool) -> Self {
        Self {
            deltas: Vec::with_capacity(n),
            rates: Vec::with_capacity(n),
            errs: Vec::with_capacity(if keep_err { n } else { 0 }),
            rels: Vec::with_capacity(if keep_rel { n } else { 0 }),
        }
    }
}

fn bins_over(pair: &WindowedPair, rows: Range<usize>, edges: &[f64]) -> Vec<CompareBin> {
    let bin_count = edges.len().saturating_sub(1);
    let cols = pair.quick_sci.ncols();
    let good_bin = |y: usize, x: usize| pair.good[[y, x]].then(|| bin_index(edges, pair.official_sci[[y, x]] as f64)).flatten();
    let mut counts = vec![0usize; bin_count];
    for y in rows.clone() {
        for x in 0..cols {
            if let Some(k) = good_bin(y, x) {
                counts[k] += 1;
            }
        }
    }
    let keep_err = pair.official_err.is_some();
    let mut samples: Vec<BinSamples> = counts
        .iter()
        .enumerate()
        .map(|(k, n)| BinSamples::with_capacity(*n, keep_err, !is_faint_bin(edges[k + 1])))
        .collect();
    for y in rows {
        for x in 0..cols {
            let Some(k) = good_bin(y, x) else { continue };
            let rate = pair.official_sci[[y, x]];
            let delta = pair.quick_sci[[y, x]] - rate;
            let bin = &mut samples[k];
            bin.deltas.push(delta);
            bin.rates.push(rate);
            if let Some(err) = &pair.official_err {
                bin.errs.push(err[[y, x]]);
            }
            if !is_faint_bin(edges[k + 1]) {
                bin.rels.push(delta / rate);
            }
        }
    }
    samples
        .into_iter()
        .enumerate()
        .filter(|(_, bin)| !bin.deltas.is_empty())
        .map(|(k, bin)| {
            let delta = sorted_f32(bin.deltas);
            let rate = sorted_f32(bin.rates);
            let err = sorted_f32(bin.errs);
            let rel = sorted_f32(bin.rels);
            CompareBin {
                lo: edges[k],
                hi: edges[k + 1],
                n: delta.len(),
                median_rate: nearest_rank_f32(&rate, 0.5),
                median_err: nearest_rank_f32(&err, 0.5),
                median_delta: nearest_rank_f32(&delta, 0.5),
                median_rel: nearest_rank_f32(&rel, 0.5),
                p16_delta: nearest_rank_f32(&delta, 0.16),
                p84_delta: nearest_rank_f32(&delta, 0.84),
            }
        })
        .collect()
}

fn good_pairs<'a>(pair: &'a WindowedPair<'a>, rows: usize, cols: usize, keep: impl Fn(f32) -> bool + 'a) -> impl Iterator<Item = (f64, f64)> + 'a {
    (0..rows)
        .flat_map(move |y| (0..cols).map(move |x| (y, x)))
        .filter(move |&(y, x)| pair.good[[y, x]] && keep(pair.official_sci[[y, x]]))
        .map(move |(y, x)| (pair.quick_sci[[y, x]] as f64, pair.official_sci[[y, x]] as f64))
}

fn confusion(pairs: impl Iterator<Item = (bool, bool)>) -> FlagConfusion {
    let mut out = FlagConfusion::default();
    for (quick, official) in pairs {
        match (quick, official) {
            (true, true) => out.tp += 1,
            (true, false) => out.fp += 1,
            (false, true) => out.fn_ += 1,
            (false, false) => {}
        }
    }
    let ratio = |num: usize, den: usize| if den == 0 { f64::NAN } else { num as f64 / den as f64 };
    out.recall = ratio(out.tp, out.tp + out.fn_);
    out.precision = ratio(out.tp, out.tp + out.fp);
    out
}

fn histogram(values: impl Iterator<Item = f64>, half_range: f64, bins: usize) -> Histogram {
    let width = 2.0 * half_range / bins as f64;
    let edges = (0..=bins).map(|i| -half_range + width * i as f64).collect();
    let mut counts = vec![0usize; bins];
    for v in values {
        if !v.is_finite() || v < -half_range || v > half_range {
            continue;
        }
        let index = (((v + half_range) / width).floor() as usize).min(bins - 1);
        counts[index] += 1;
    }
    Histogram { edges, counts }
}

pub fn compare_with_rate(
    quick: &QuickSlopePlanes,
    official: &OfficialRate,
    science_rows: Option<(usize, usize)>,
    layout: Option<&Irs2Layout>,
) -> Result<RateComparison> {
    let (rows, cols) = quick.sci.dim();
    if official.sci.dim() != (rows, cols) {
        bail!(
            "quick slope is {rows} x {cols} but the official rate is {} x {}: the two products do not describe the same frame",
            official.sci.nrows(),
            official.sci.ncols()
        );
    }
    for (name, dim) in [("quick DQ", quick.dq.dim()), ("quick NGOOD", quick.ngood.dim()), ("official DQ", official.dq.dim())] {
        if dim != (rows, cols) {
            bail!("{name} is {} x {} but SCI is {rows} x {cols}", dim.0, dim.1);
        }
    }
    let (r0, r1) = science_rows.unwrap_or((0, rows));
    if r0 >= r1 || r1 > rows {
        bail!("science row window {r0}..{r1} is empty or exceeds the {rows} stripped rows");
    }
    let window = s![r0..r1, ..];
    let quick_sci = quick.sci.slice(window);
    let quick_dq = quick.dq.slice(window);
    let official_sci = official.sci.slice(window);
    let official_dq = official.dq.slice(window);
    let good = Zip::from(&quick_sci)
        .and(&quick_dq)
        .and(&official_sci)
        .and(&official_dq)
        .map_collect(|q, qd, o, od| *od == 0 && q.is_finite() && o.is_finite() && qd & DQ_DO_NOT_USE == 0);
    let pair = WindowedPair {
        quick_sci,
        quick_dq,
        quick_ngood: quick.ngood.slice(window),
        official_sci,
        official_err: official.err.as_ref().map(|e| e.slice(window)),
        official_dq,
        good,
    };
    let good_pixels = pair.good.iter().filter(|g| **g).count();
    let quick_flagged_in_good = Zip::from(&pair.good)
        .and(&pair.quick_dq)
        .fold(0usize, |acc, g, qd| acc + (*g && qd & (DQ_JUMP_DET | DQ_SATURATED) != 0) as usize);
    let window_rows = 0..(r1 - r0);
    let bins = bins_over(&pair, window_rows, &RATE_BIN_EDGES);
    let amp_bins = layout.map_or_else(Vec::new, |layout| {
        (0..layout.science_outputs())
            .map(|amplifier| {
                let (a0, a1) = layout.science_rows_of_amplifier(amplifier);
                let (lo, hi) = (a0.max(r0), a1.min(r1));
                let bins = if lo < hi { bins_over(&pair, (lo - r0)..(hi - r0), &RATE_BIN_EDGES[..=FAINT_BIN_COUNT]) } else { Vec::new() };
                AmpBins { amplifier, science_rows: (lo, hi.max(lo)), bins }
            })
            .collect()
    });
    let zero_shift = zero_shift_check(pair.quick_sci, pair.official_sci, pair.official_dq, ZERO_SHIFT_MAX_DY);
    let both_finite = |y: usize, x: usize| pair.quick_sci[[y, x]].is_finite() && pair.official_sci[[y, x]].is_finite();
    let flags = |quick_rule: &dyn Fn(u32, i32) -> bool, official_bit: u32, finite_only: bool| {
        confusion(
            (0..r1 - r0)
                .flat_map(|y| (0..cols).map(move |x| (y, x)))
                .filter(|&(y, x)| !finite_only || both_finite(y, x))
                .map(|(y, x)| (quick_rule(pair.quick_dq[[y, x]], pair.quick_ngood[[y, x]]), pair.official_dq[[y, x]] & official_bit != 0)),
        )
    };
    let jump = flags(&|dq, _| dq & DQ_JUMP_DET != 0, DQ_JUMP_DET, true);
    let saturated = flags(&|dq, ngood| dq & DQ_SATURATED != 0 && ngood == 0, DQ_SATURATED, false);
    let saturated_any_group = flags(&|dq, _| dq & DQ_SATURATED != 0, DQ_SATURATED, false);
    let rel_hist = histogram(good_pairs(&pair, r1 - r0, cols, |o| o >= REL_HIST_MIN_RATE).map(|(q, o)| (q - o) / o), REL_HIST_RANGE, HISTOGRAM_BINS);
    let delta_hist = histogram(good_pairs(&pair, r1 - r0, cols, |o| o < DELTA_HIST_MAX_RATE).map(|(q, o)| q - o), DELTA_HIST_RANGE, HISTOGRAM_BINS);
    Ok(RateComparison {
        detector: official.detector.clone(),
        good_pixels,
        quick_flagged_in_good,
        bins,
        amp_bins,
        zero_shift,
        jump,
        saturated,
        saturated_rule: SATURATED_RULE,
        saturated_any_group,
        rel_hist,
        delta_hist,
        science_rows,
    })
}

pub fn ratio_image(quick_sci: &Array2<f32>, official: &OfficialRate) -> Array2<f32> {
    Zip::from(quick_sci)
        .and(&official.sci)
        .and(&official.dq)
        .map_collect(|q, o, d| if *d != 0 || o.abs() < RATIO_MIN_RATE { f32::NAN } else { q / o })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ramp::quick_slope::{DQ_DO_NOT_USE, DQ_JUMP_DET, DQ_SATURATED};
    use crate::core::ramp::test_support::{irs2_truth_slope, truth_rate_planes, write_synthetic_rate, SYNTHETIC_ERR};
    use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};

    struct Pixel {
        quick: f32,
        rate: f32,
        quick_dq: u32,
        rate_dq: u32,
        ngood: i32,
    }

    fn clean(quick: f32, rate: f32) -> Pixel {
        Pixel { quick, rate, quick_dq: 0, rate_dq: 0, ngood: 10 }
    }

    fn planes(rows: usize, cols: usize, pixel: impl Fn(usize, usize) -> Pixel) -> (QuickSlopePlanes, OfficialRate) {
        let mut quick = QuickSlopePlanes { sci: Array2::zeros((rows, cols)), dq: Array2::zeros((rows, cols)), ngood: Array2::zeros((rows, cols)) };
        let mut official = OfficialRate { sci: Array2::zeros((rows, cols)), err: Some(Array2::from_elem((rows, cols), 0.06)), dq: Array2::zeros((rows, cols)), detector: Some("NRS1".into()) };
        for y in 0..rows {
            for x in 0..cols {
                let p = pixel(y, x);
                quick.sci[[y, x]] = p.quick;
                quick.dq[[y, x]] = p.quick_dq;
                quick.ngood[[y, x]] = p.ngood;
                official.sci[[y, x]] = p.rate;
                official.dq[[y, x]] = p.rate_dq;
            }
        }
        (quick, official)
    }

    fn truth_image(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| irs2_truth_slope(y, x) + 0.01 * ((y * 7 + x * 3) % 5) as f32)
    }

    fn all_good(rows: usize, cols: usize) -> Array2<u32> {
        Array2::zeros((rows, cols))
    }

    fn bin_for(bins: &[CompareBin], lo: f64) -> &CompareBin {
        bins.iter().find(|b| b.lo == lo).unwrap_or_else(|| panic!("no bin starting at {lo}: {bins:?}"))
    }

    #[test]
    fn zero_shift_passes_on_an_identical_image_with_row_structure() {
        let truth = truth_image(200, 32);
        let check = zero_shift_check(truth.view(), truth.view(), all_good(200, 32).view(), ZERO_SHIFT_MAX_DY);
        assert_eq!(check.best_dy, 0);
        assert!(check.corr_at_zero > 0.999, "{check:?}");
        assert!(check.corr_best > 0.999);
        assert!(check.passed, "{check:?}");
        assert_ne!(check.second_dy, 0);
        assert!(check.second_corr < check.corr_at_zero);
    }

    #[test]
    fn zero_shift_finds_a_planted_three_row_shift_and_fails() {
        let truth = truth_image(200, 32);
        let shifted = Array2::from_shape_fn((200, 32), |(y, x)| truth[[(y + 3).min(199), x]]);
        let check = zero_shift_check(shifted.view(), truth.view(), all_good(200, 32).view(), ZERO_SHIFT_MAX_DY);
        assert_eq!(check.best_dy, 3, "{check:?}");
        assert!(!check.passed);
        assert!(check.corr_best > 0.99 && check.corr_at_zero < check.corr_best);
        let other_way = zero_shift_check(truth.view(), shifted.view(), all_good(200, 32).view(), ZERO_SHIFT_MAX_DY);
        assert_eq!(other_way.best_dy, -3, "{other_way:?}");
    }

    #[test]
    fn zero_shift_fails_on_a_flat_image_instead_of_passing_by_accident() {
        let flat = Array2::from_elem((100, 32), 1.5f32);
        let check = zero_shift_check(flat.view(), flat.view(), all_good(100, 32).view(), ZERO_SHIFT_MAX_DY);
        assert!(check.corr_at_zero.is_nan() && check.corr_best.is_nan(), "{check:?}");
        assert!(!check.passed);
        let narrow = truth_image(100, 8);
        let too_few_columns = zero_shift_check(narrow.view(), narrow.view(), all_good(100, 8).view(), ZERO_SHIFT_MAX_DY);
        assert!(!too_few_columns.passed, "{ROW_PROFILE_MIN_PIXELS} good pixels are needed per row");
        let profile = row_profile(narrow.view(), Array2::from_elem((100, 8), true).view());
        assert!(profile.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn bins_reproduce_the_critic_statistics_on_a_synthetic_pair() {
        let rates: [f32; 20] = [0.01, 0.02, 0.03, 0.1, 0.2, 0.25, 0.4, 0.5, 0.6, 0.7, 0.8, 1.5, 2.0, 2.5, 4.0, 5.0, 6.0, 15.0, 20.0, 25.0];
        let (quick, official) = planes(4, 5, |y, x| {
            let rate = rates[y * 5 + x];
            clean(1.01 * rate + 0.01, rate)
        });
        let cmp = compare_with_rate(&quick, &official, None, None).unwrap();
        assert_eq!(cmp.good_pixels, 20);
        assert_eq!(cmp.bins.len(), 6, "the empty [30, inf) bin is not emitted: {:?}", cmp.bins);
        assert!(cmp.bins.iter().all(|b| b.lo < 30.0));
        for (k, bin) in cmp.bins.iter().enumerate() {
            assert_eq!((bin.lo, bin.hi), (RATE_BIN_EDGES[k], RATE_BIN_EDGES[k + 1]));
            assert_eq!(bin.n, if k == 2 { 5 } else { 3 });
            let expected_delta = 0.01 * bin.median_rate + 0.01;
            assert!((bin.median_delta - expected_delta).abs() < 2e-6, "bin {k}: {bin:?}");
            assert!((bin.median_err - 0.06).abs() < 1e-6);
            assert!(bin.p16_delta <= bin.median_delta && bin.median_delta <= bin.p84_delta);
            if k < FAINT_BIN_COUNT {
                assert!(bin.median_rel.is_nan(), "faint bins have no meaningful relative error: {bin:?}");
            } else {
                let expected_rel = 0.01 + 0.01 / bin.median_rate;
                assert!((bin.median_rel - expected_rel).abs() < 1e-6, "bin {k}: {bin:?}");
            }
        }
        assert_eq!(bin_for(&cmp.bins, 1.0).median_rate, 2.0);
        assert_eq!(bin_for(&cmp.bins, 0.3).median_rate as f32, 0.6);
        assert!(cmp.amp_bins.is_empty());
        assert_eq!(cmp.science_rows, None);
        assert_eq!(cmp.detector.as_deref(), Some("NRS1"));
    }

    #[test]
    fn bins_without_an_err_plane_report_nan_median_err_and_keep_the_other_statistics() {
        let (quick, mut official) = planes(4, 8, |y, x| {
            let rate = 1.5 + 0.01 * y as f32 - 0.002 * x as f32;
            clean(rate + 0.01, rate)
        });
        official.err = None;
        let cmp = compare_with_rate(&quick, &official, None, None).unwrap();
        assert_eq!(cmp.bins.len(), 1);
        let bin = &cmp.bins[0];
        assert_eq!((bin.lo, bin.hi, bin.n), (1.0, 3.0, 32));
        assert!(bin.median_err.is_nan());
        assert!((bin.median_delta - 0.01).abs() < 1e-6, "{bin:?}");
        assert!(bin.median_rel.is_finite() && bin.median_rate.is_finite());
    }

    #[test]
    fn amp_bins_are_computed_over_the_amplifier_science_rows() {
        let layout = Irs2Layout::new(3200, 16, 4, 5, false).unwrap();
        let (quick, official) = planes(2048, 8, |y, _| {
            let offset = if (512..1024).contains(&y) { 0.1 } else { 0.0 };
            clean(0.01 + offset, 0.01)
        });
        let cmp = compare_with_rate(&quick, &official, None, Some(&layout)).unwrap();
        assert_eq!(cmp.amp_bins.len(), 4);
        for (k, amp) in cmp.amp_bins.iter().enumerate() {
            assert_eq!(amp.amplifier, k);
            assert_eq!(amp.science_rows, (512 * k, 512 * k + 512));
            assert_eq!(amp.bins.len(), 1, "only the populated faint bin is emitted");
            assert!(amp.bins.iter().all(|b| b.hi <= 0.3));
            let expected = if k == 1 { 0.1 } else { 0.0 };
            assert!((amp.bins[0].median_delta - expected).abs() < 1e-6, "amplifier {k}: {:?}", amp.bins[0]);
            assert_eq!(amp.bins[0].n, 512 * 8);
        }
        let full = bin_for(&cmp.bins, f64::NEG_INFINITY);
        assert!(full.median_delta.abs() < 0.03, "the full-frame faint median hides the amplifier offset: {full:?}");
        assert_eq!(full.n, 2048 * 8);
    }

    #[test]
    fn good_pixels_exclude_official_flags_non_finite_values_and_quick_do_not_use() {
        let (quick, official) = planes(4, 4, |y, x| match (y, x) {
            (0, 0) => Pixel { quick: 1.0, rate: 1.0, quick_dq: 0, rate_dq: 1, ngood: 10 },
            (0, 1) => Pixel { quick: f32::NAN, rate: 1.0, quick_dq: 0, rate_dq: 0, ngood: 10 },
            (0, 2) => Pixel { quick: 1.0, rate: f32::INFINITY, quick_dq: 0, rate_dq: 0, ngood: 10 },
            (0, 3) => Pixel { quick: 1.0, rate: 1.0, quick_dq: DQ_DO_NOT_USE | DQ_SATURATED, rate_dq: 0, ngood: 0 },
            (1, 0) => Pixel { quick: 1.0, rate: 1.0, quick_dq: DQ_JUMP_DET, rate_dq: 0, ngood: 9 },
            (1, 1) => Pixel { quick: 1.0, rate: 1.0, quick_dq: DQ_SATURATED, rate_dq: 0, ngood: 6 },
            _ => clean(1.0, 1.0),
        });
        let cmp = compare_with_rate(&quick, &official, None, None).unwrap();
        assert_eq!(cmp.good_pixels, 12);
        assert_eq!(cmp.quick_flagged_in_good, 2);
        assert_eq!(cmp.bins.len(), 1);
        assert_eq!(cmp.bins[0].n, 12);
    }

    #[test]
    fn confusion_counts_true_false_and_missed_jump_flags_with_recall_and_precision() {
        let (quick, official) = planes(2, 5, |y, x| match (y, x) {
            (0, 0) | (0, 1) | (0, 2) => Pixel { quick: 1.0, rate: 1.0, quick_dq: DQ_JUMP_DET, rate_dq: DQ_JUMP_DET, ngood: 9 },
            (0, 3) => Pixel { quick: 1.0, rate: 1.0, quick_dq: DQ_JUMP_DET, rate_dq: 0, ngood: 9 },
            (0, 4) | (1, 0) => Pixel { quick: 1.0, rate: 1.0, quick_dq: 0, rate_dq: DQ_JUMP_DET, ngood: 10 },
            (1, 1) => Pixel { quick: f32::NAN, rate: 1.0, quick_dq: DQ_JUMP_DET | DQ_DO_NOT_USE, rate_dq: DQ_JUMP_DET, ngood: 1 },
            _ => clean(1.0, 1.0),
        });
        let cmp = compare_with_rate(&quick, &official, None, None).unwrap();
        assert_eq!((cmp.jump.tp, cmp.jump.fp, cmp.jump.fn_), (3, 1, 2));
        assert!((cmp.jump.recall - 0.6).abs() < 1e-12);
        assert!((cmp.jump.precision - 0.75).abs() < 1e-12);
        assert!(cmp.saturated.recall.is_nan() && cmp.saturated.precision.is_nan());
        assert_eq!(cmp.saturated_rule, SATURATED_RULE);
    }

    #[test]
    fn saturated_confusion_uses_the_all_groups_rule_and_reports_any_group_separately() {
        let (quick, official) = planes(1, 5, |_, x| match x {
            0 => Pixel { quick: 1.0, rate: 1.0, quick_dq: DQ_SATURATED, rate_dq: DQ_SATURATED, ngood: 5 },
            1 => Pixel { quick: f32::NAN, rate: f32::NAN, quick_dq: DQ_SATURATED | DQ_DO_NOT_USE, rate_dq: DQ_SATURATED | DQ_DO_NOT_USE, ngood: 0 },
            2 => Pixel { quick: 1.0, rate: 1.0, quick_dq: DQ_SATURATED, rate_dq: 0, ngood: 7 },
            3 => Pixel { quick: f32::NAN, rate: f32::NAN, quick_dq: DQ_SATURATED | DQ_DO_NOT_USE, rate_dq: DQ_SATURATED | DQ_DO_NOT_USE, ngood: 0 },
            _ => clean(1.0, 1.0),
        });
        let cmp = compare_with_rate(&quick, &official, None, None).unwrap();
        assert_eq!((cmp.saturated.tp, cmp.saturated.fp, cmp.saturated.fn_), (2, 0, 1), "all-groups saturation lives on NaN pixels, which still count");
        assert_eq!((cmp.saturated_any_group.tp, cmp.saturated_any_group.fp, cmp.saturated_any_group.fn_), (3, 1, 0));
        assert!((cmp.saturated.recall - 2.0 / 3.0).abs() < 1e-12 && (cmp.saturated.precision - 1.0).abs() < 1e-12);
        assert!((cmp.saturated_any_group.recall - 1.0).abs() < 1e-12);
        assert!((cmp.saturated_any_group.precision - 3.0 / 4.0).abs() < 1e-12);
        assert_eq!(cmp.good_pixels, 2, "the DQ-only statistics do not change the good-pixel rule");
    }

    #[test]
    fn science_row_windows_restrict_every_statistic() {
        let (quick, official) = planes(100, 32, |y, x| {
            let rate = 1.5 + irs2_truth_slope(y, x % 4);
            if y < 50 {
                Pixel { quick: rate + 1.0, rate, quick_dq: DQ_JUMP_DET, rate_dq: 0, ngood: 9 }
            } else {
                clean(rate, rate)
            }
        });
        let lower = compare_with_rate(&quick, &official, Some((50, 100)), None).unwrap();
        assert_eq!(lower.science_rows, Some((50, 100)));
        assert_eq!(lower.good_pixels, 50 * 32);
        assert_eq!(lower.quick_flagged_in_good, 0);
        assert_eq!(lower.bins.len(), 1);
        assert!(lower.bins[0].median_delta.abs() < 1e-6);
        assert_eq!(lower.jump.fp, 0);
        assert!(lower.zero_shift.passed, "{:?}", lower.zero_shift);
        assert_eq!(lower.rel_hist.counts.iter().sum::<usize>(), 50 * 32);
        let upper = compare_with_rate(&quick, &official, Some((0, 50)), None).unwrap();
        assert_eq!(upper.quick_flagged_in_good, 50 * 32);
        assert!((upper.bins[0].median_delta - 1.0).abs() < 1e-6);
        assert_eq!(upper.jump.fp, 50 * 32);
        assert!(compare_with_rate(&quick, &official, Some((90, 200)), None).is_err());
        assert!(compare_with_rate(&quick, &official, Some((60, 60)), None).is_err());
        let (narrow, _) = planes(10, 32, |_, _| clean(1.0, 1.0));
        let err = compare_with_rate(&narrow, &official, None, None).unwrap_err().to_string();
        assert!(err.contains("10") && err.contains("100"), "{err}");
    }

    #[test]
    fn the_ratio_image_is_nan_where_the_official_rate_is_flagged_or_faint() {
        let (quick, official) = planes(2, 3, |y, x| match (y, x) {
            (0, 0) => Pixel { quick: 2.0, rate: 1.0, quick_dq: 0, rate_dq: 4, ngood: 10 },
            (0, 1) => clean(0.2, 0.2),
            (0, 2) => clean(-0.5, -0.25),
            (1, 0) => clean(3.0, 2.0),
            (1, 1) => clean(0.6, 0.3),
            _ => clean(1.0, -1.0),
        });
        let ratio = ratio_image(&quick.sci, &official);
        assert!(ratio[[0, 0]].is_nan() && ratio[[0, 1]].is_nan() && ratio[[0, 2]].is_nan());
        assert_eq!(ratio[[1, 0]], 1.5);
        assert_eq!(ratio[[1, 1]], 2.0);
        assert_eq!(ratio[[1, 2]], -1.0);
    }

    #[test]
    fn the_synthetic_rate_fixture_round_trips_through_read_official_rate() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Irs2Layout::new(3200, 16, 4, 5, true).unwrap();
        let (sci, err, dq) = truth_rate_planes(&layout, 8);
        let path = write_synthetic_rate(&dir.path().join("truth_rate.fits"), &sci, &err, &dq, "NRS2");
        let official = read_official_rate(&path).unwrap();
        assert_eq!(official.sci.dim(), (2048, 8));
        assert_eq!(official.sci, sci);
        assert_eq!(official.err.as_ref().unwrap()[[5, 5]], SYNTHETIC_ERR);
        assert!(official.dq.iter().all(|d| *d == 0));
        assert_eq!(official.detector.as_deref(), Some("NRS2"));
    }

    #[test]
    fn read_official_rate_finds_planes_by_extname_and_decodes_the_dq_bzero() {
        let dir = tempfile::tempdir().unwrap();
        let rate_path = dir.path().join("synthetic_rate.fits");
        let dq_raw: Vec<i32> = (0..12).map(|i| (if i == 5 { 4i64 } else if i == 7 { 3 } else { 0 } - 2147483648) as i32).collect();
        write_test_mef(
            &rate_path,
            &[("DETECTOR", "'NRS2'".into()), ("TELESCOP", "'JWST'".into())],
            &[
                TestHdu { extname: Some("ERR"), extver: Some(1), cols: 4, rows: 3, data: HduData::F32((0..12).map(|i| i as f32 * 0.5).collect()), extra_cards: vec![] },
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: 4, rows: 3, data: HduData::F32((0..12).map(|i| i as f32).collect()), extra_cards: vec![("BUNIT", "'DN/s'".into())] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: 4, rows: 3, data: HduData::I32(dq_raw), extra_cards: vec![("BZERO", "2147483648".into()), ("BSCALE", "1".into())] },
            ],
        );
        let official = read_official_rate(rate_path.to_str().unwrap()).unwrap();
        assert_eq!(official.sci.dim(), (3, 4));
        assert_eq!(official.sci[[1, 1]], 5.0);
        assert_eq!(official.err.as_ref().unwrap()[[1, 1]], 2.5);
        assert_eq!(official.dq[[1, 1]], 4);
        assert_eq!(official.dq[[1, 3]], 3);
        assert_eq!(official.dq[[0, 0]], 0);
        assert_eq!(official.detector.as_deref(), Some("NRS2"));

        let qslope_path = dir.path().join("synthetic_qslope.fits");
        write_test_mef(
            &qslope_path,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: 4, rows: 3, data: HduData::F32((0..12).map(|i| i as f32).collect()), extra_cards: vec![] },
                TestHdu { extname: Some("NGOOD"), extver: Some(1), cols: 4, rows: 3, data: HduData::I32((0..12).map(|i| 10 - i).collect()), extra_cards: vec![] },
                TestHdu { extname: Some("DQ"), extver: Some(1), cols: 4, rows: 3, data: HduData::I32((0..12).map(|i| (i as i64 % 3 - 2147483648) as i32).collect()), extra_cards: vec![("BZERO", "2147483648".into())] },
                TestHdu { extname: Some("NOISE"), extver: Some(1), cols: 4, rows: 3, data: HduData::F32(vec![0.1; 12]), extra_cards: vec![] },
            ],
        );
        let quick = read_qslope(qslope_path.to_str().unwrap()).unwrap();
        assert_eq!(quick.sci[[2, 3]], 11.0);
        assert_eq!(quick.ngood[[2, 3]], -1);
        assert_eq!(quick.dq[[0, 2]], 2);
        let missing = dir.path().join("no_dq.fits");
        write_test_mef(&missing, &[], &[TestHdu { extname: Some("SCI"), extver: Some(1), cols: 2, rows: 2, data: HduData::F32(vec![1.0; 4]), extra_cards: vec![] }]);
        assert!(read_official_rate(missing.to_str().unwrap()).is_err());
    }
}
