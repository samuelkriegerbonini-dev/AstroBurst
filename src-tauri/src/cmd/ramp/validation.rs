use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::Value;

use super::qslope::{compare_rate_files, pixel_fit_json, quick_slope_files};
use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
use crate::core::ramp::compare::{read_official_rate, read_qslope, RATIO_MIN_RATE, REL_HIST_MIN_RATE};
use crate::core::ramp::quick_slope::{QuickSlopeParams, RefCorrection, DQ_DO_NOT_USE};
use crate::core::ramp::test_support::{write_synthetic_uncal, SyntheticUncal};
use crate::math::median::exact_median_mut;

const PAIR_DIR_ENV: &str = "ASTROBURST_RAMP_PAIR_DIR";
const PAIR_FILTER_ENV: &str = "ASTROBURST_RAMP_PAIR_GLOB";
const UNCAL_FILE_SUFFIX: &str = "_uncal.fits";
const RATE_FILE_SUFFIX: &str = "_rate.fits";
const BAND_CHECK_MARKER: &str = "00001_nrs2";
const BAND_UNCAL_ROWS: [usize; 2] = [1300, 1500];
const GATE_FAINT_MAX_ABS_DELTA: f64 = 0.03;
const GATE_BRIGHT_REL_LOW: f64 = -0.01;
const GATE_BRIGHT_REL_HIGH: f64 = 0.03;
const GATE_BRIGHT_MIN_N: u64 = 50;
const GATE_MIN_GOOD_PIXELS: u64 = 3_500_000;
const GATE_BAND_OFF_DELTA: [f64; 2] = [-0.26, -0.16];
const GATE_BAND_CORRECTED_DELTA: [f64; 2] = [-0.03, 0.03];
const BRIGHT_BIN_LOWER_EDGES: [f64; 3] = [1.0, 3.0, 10.0];
const FAINT_BIN_UPPER_EDGE: f64 = 0.05;
const RATIO_TOLERANCES: [f64; 2] = [0.02, 0.05];
const RATIO_MIN_RATES: [f32; 2] = [RATIO_MIN_RATE, REL_HIST_MIN_RATE];
const PIXEL_FIT_PROBE: (usize, usize) = (1000, 1500);
const THREE_INTEGRATIONS: usize = 3;
const SYNTHETIC_COLS: usize = 8;
const SYNTHETIC_ROWS: usize = 64;
const SYNTHETIC_NGROUPS: usize = 10;
const SYNTHETIC_PEDESTAL_DN: f32 = 1000.0;
const SYNTHETIC_TGROUP_S: f64 = 10.0;
const SYNTHETIC_SLOPE_STEP: f32 = 0.5;
const SYNTHETIC_COLUMN_SLOPE: f32 = 0.1;
const SLOPE_TOLERANCE: f32 = 1e-4;

fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

fn num(value: &Value) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
}

fn count(value: &Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

fn edge(value: &Value, infinite: &str) -> String {
    value.as_f64().map(|v| format!("{v}")).unwrap_or_else(|| infinite.to_string())
}

fn bin_label(bin: &Value) -> String {
    format!("[{},{})", edge(&bin["lo"], "-inf"), edge(&bin["hi"], "inf"))
}

fn rel_text(value: f64) -> String {
    if value.is_finite() {
        format!("{value:+.4}")
    } else {
        "nan".to_string()
    }
}

fn bins_of(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn print_bins(indent: &str, bins: &Value) {
    for bin in bins_of(bins) {
        println!(
            "{indent}{:<13} | n={:>8} | rate {:>8.4} | err {:>7.4} | delta {:+.4} | rel {} | p16..p84 {:+.3}..{:+.3}",
            bin_label(bin),
            count(&bin["n"]),
            num(&bin["median_rate"]),
            num(&bin["median_err"]),
            num(&bin["median_delta"]),
            rel_text(num(&bin["median_rel"])),
            num(&bin["p16_delta"]),
            num(&bin["p84_delta"]),
        );
    }
}

fn print_confusion(label: &str, confusion: &Value) {
    println!(
        "  {label}: tp={} fp={} fn={} recall={:.4} precision={:.4}",
        count(&confusion["tp"]),
        count(&confusion["fp"]),
        count(&confusion["fn_"]),
        num(&confusion["recall"]),
        num(&confusion["precision"]),
    );
}

fn faint_bin(bins: &Value) -> Option<&Value> {
    bins_of(bins).iter().find(|bin| (num(&bin["hi"]) - FAINT_BIN_UPPER_EDGE).abs() < 1e-9)
}

fn faint_median_delta(comparison: &Value) -> f64 {
    faint_bin(&comparison["bins"]).map(|bin| num(&bin["median_delta"])).unwrap_or(f64::NAN)
}

fn within(value: f64, range: [f64; 2]) -> bool {
    value.is_finite() && value >= range[0] && value <= range[1]
}

fn gate_failures(name: &str, comparison: &Value) -> Vec<String> {
    let mut failures = Vec::new();
    let shift = &comparison["zero_shift"];
    if shift["passed"] != Value::Bool(true) {
        failures.push(format!(
            "{name}: G1 zero shift failed: best_dy={} corr_at_zero={:.5} corr_best={:.5}",
            shift["best_dy"],
            num(&shift["corr_at_zero"]),
            num(&shift["corr_best"])
        ));
    }
    let amp_bins = bins_of(&comparison["amp_bins"]);
    if amp_bins.is_empty() {
        failures.push(format!("{name}: G2 has no amp_bins: the product carries no IRS2 layout"));
    }
    for amp in amp_bins {
        for bin in bins_of(&amp["bins"]) {
            let delta = num(&bin["median_delta"]);
            if !(delta.abs() <= GATE_FAINT_MAX_ABS_DELTA) {
                failures.push(format!(
                    "{name}: G2 amplifier {} bin {} median_delta {delta:+.4} exceeds {GATE_FAINT_MAX_ABS_DELTA}",
                    amp["amplifier"],
                    bin_label(bin)
                ));
            }
        }
    }
    for bin in bins_of(&comparison["bins"]) {
        let lo = num(&bin["lo"]);
        if !BRIGHT_BIN_LOWER_EDGES.iter().any(|edge| (edge - lo).abs() < 1e-9) || count(&bin["n"]) < GATE_BRIGHT_MIN_N {
            continue;
        }
        let rel = num(&bin["median_rel"]);
        if !within(rel, [GATE_BRIGHT_REL_LOW, GATE_BRIGHT_REL_HIGH]) {
            failures.push(format!(
                "{name}: G3 bin {} median_rel {rel:+.4} outside [{GATE_BRIGHT_REL_LOW}, {GATE_BRIGHT_REL_HIGH}]",
                bin_label(bin)
            ));
        }
    }
    let good = count(&comparison["good_pixels"]);
    if good < GATE_MIN_GOOD_PIXELS {
        failures.push(format!("{name}: G5 good_pixels {good} below {GATE_MIN_GOOD_PIXELS}"));
    }
    failures
}

struct RatioStats {
    min_rate: f32,
    n: usize,
    median_ratio: f64,
    mad_rel: f64,
    within: [f64; 2],
}

fn ratio_statistics(qslope_path: &str, rate_path: &str) -> Vec<RatioStats> {
    let quick = read_qslope(qslope_path).unwrap();
    let official = read_official_rate(rate_path).unwrap();
    assert_eq!(quick.sci.dim(), official.sci.dim(), "quick and official planes differ in shape");
    RATIO_MIN_RATES
        .iter()
        .map(|&min_rate| {
            let mut rel: Vec<f32> = quick
                .sci
                .iter()
                .zip(official.sci.iter())
                .zip(official.dq.iter().zip(quick.dq.iter()))
                .filter(|((q, o), (dq_official, dq_quick))| {
                    **dq_official == 0 && *dq_quick & DQ_DO_NOT_USE == 0 && q.is_finite() && o.is_finite() && **o >= min_rate
                })
                .map(|((q, o), _)| q / o - 1.0)
                .collect();
            let n = rel.len();
            if n == 0 {
                return RatioStats { min_rate, n, median_ratio: f64::NAN, mad_rel: f64::NAN, within: [f64::NAN; 2] };
            }
            let median_rel = exact_median_mut(&mut rel);
            let mut deviations: Vec<f32> = rel.iter().map(|r| (*r as f64 - median_rel).abs() as f32).collect();
            let mad_rel = exact_median_mut(&mut deviations);
            let within = RATIO_TOLERANCES.map(|tol| rel.iter().filter(|r| (r.abs() as f64) <= tol).count() as f64 / n as f64);
            RatioStats { min_rate, n, median_ratio: 1.0 + median_rel, mad_rel, within }
        })
        .collect()
}

fn owner_pairs(dir: &Path, filter: Option<&str>) -> (Vec<(String, String)>, Vec<String>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{PAIR_DIR_ENV}={}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    entries.sort();
    let mut listing = Vec::new();
    let mut pairs = Vec::new();
    for path in entries {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        listing.push(name.clone());
        if !name.ends_with(UNCAL_FILE_SUFFIX) || filter.is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let stem = &name[..name.len() - UNCAL_FILE_SUFFIX.len()];
        let rate = dir.join(format!("{stem}{RATE_FILE_SUFFIX}"));
        if rate.exists() {
            pairs.push((forward_slashes(&path), forward_slashes(&rate)));
        }
    }
    (pairs, listing)
}

fn print_comparison(comparison: &Value) {
    println!(
        "compare: elapsed_ms={} good_pixels={} quick_flagged_in_good={} ({:.3} %) science_rows={} detector={}",
        count(&comparison["elapsed_ms"]),
        count(&comparison["good_pixels"]),
        count(&comparison["quick_flagged_in_good"]),
        100.0 * count(&comparison["quick_flagged_in_good"]) as f64 / count(&comparison["good_pixels"]).max(1) as f64,
        comparison["science_rows"],
        comparison["detector"],
    );
    print_bins("  ", &comparison["bins"]);
    for amp in bins_of(&comparison["amp_bins"]) {
        println!("  amplifier {} science_rows {}:", amp["amplifier"], amp["science_rows"]);
        print_bins("    ", &amp["bins"]);
    }
    let shift = &comparison["zero_shift"];
    println!(
        "  zero_shift: best_dy={} corr_at_zero={:.5} corr_best={:.5} second_dy={} second_corr={:.5} passed={}",
        shift["best_dy"],
        num(&shift["corr_at_zero"]),
        num(&shift["corr_best"]),
        shift["second_dy"],
        num(&shift["second_corr"]),
        shift["passed"],
    );
    print_confusion("JUMP_DET", &comparison["jump"]);
    print_confusion("SATURATED all-groups", &comparison["saturated"]);
    print_confusion("SATURATED any-group", &comparison["saturated_any_group"]);
    println!("  saturated_rule={}", comparison["saturated_rule"]);
}

#[test]
#[ignore]
fn owner_pairs_quick_slope_matches_the_official_rate_within_the_published_tolerances() {
    let Some(dir) = std::env::var(PAIR_DIR_ENV).ok().filter(|d| !d.trim().is_empty()) else {
        println!("skipped: {PAIR_DIR_ENV} is not set");
        return;
    };
    let filter = std::env::var(PAIR_FILTER_ENV).ok().filter(|f| !f.is_empty());
    let (pairs, listing) = owner_pairs(Path::new(&dir), filter.as_deref());
    assert!(
        !pairs.is_empty(),
        "{PAIR_DIR_ENV}={dir} with {PAIR_FILTER_ENV}={filter:?} matched no (uncal, rate) pair; the directory holds {listing:#?}"
    );
    let products = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    for (index, (uncal, rate)) in pairs.iter().enumerate() {
        let name = file_name(uncal);
        println!("=== pair {}/{}: {name}", index + 1, pairs.len());
        let out_dir = forward_slashes(&products.path().join(format!("pair{index}")));
        let started = Instant::now();
        let slope = quick_slope_files(uncal, &out_dir, 0, &QuickSlopeParams::default(), None)
            .unwrap_or_else(|e| panic!("{name}: quick_slope_files: {e:#}"));
        let slope_wall_ms = started.elapsed().as_millis();
        println!(
            "slope: elapsed_ms={} wall_ms={slope_wall_ms} detector={} readpatt={} tgroup_s={} ({}) dimensions={} ref_corrected={} stripped={}",
            count(&slope["elapsed_ms"]),
            slope["detector"],
            slope["readpatt"],
            num(&slope["tgroup_s"]),
            slope["tgroup_source"],
            slope["dimensions"],
            slope["ref_corrected"],
            slope["stripped"],
        );
        println!("  counts={} band_scales_dn={} warnings={}", slope["counts"], slope["band_scales_dn"], slope["warnings"]);
        let sibling = slope["rate_sibling"].as_str().map(Path::new);
        assert_eq!(sibling, Some(Path::new(rate)), "{name}: rate_sibling must name the sibling rate");
        let qslope = slope["fits_path"].as_str().unwrap().to_string();
        let (probe_x, probe_y) = PIXEL_FIT_PROBE;
        let started = Instant::now();
        let pixel = pixel_fit_json(uncal, probe_x, probe_y, 0, &QuickSlopeParams::default())
            .unwrap_or_else(|e| panic!("{name}: pixel_fit_json: {e:#}"));
        let pixel_wall_ms = started.elapsed().as_millis();
        let science_row = pixel["science_row"].as_u64().unwrap_or_else(|| panic!("{name}: probe pixel is not a science row")) as usize;
        let product_value = read_qslope(&qslope).unwrap().sci[[science_row, probe_x]];
        println!(
            "pixel_fit ({probe_x}, {probe_y}): wall_ms={pixel_wall_ms} amplifier={} science_row={science_row} band_scale_dn={} fit={} flagged_diffs={} product_sci={product_value}",
            pixel["amplifier"], pixel["band_scale_dn"], pixel["fit"], pixel["flagged_diffs"]
        );
        let pixel_slope = pixel["fit"]["slope"].as_f64().map(|v| v as f32);
        assert_eq!(pixel_slope, Some(product_value), "{name}: pixel fit and product SCI must agree at the probe");
        let started = Instant::now();
        let comparison = compare_rate_files(&qslope, rate, &out_dir, None).unwrap_or_else(|e| panic!("{name}: compare_rate_files: {e:#}"));
        println!("compare wall_ms={}", started.elapsed().as_millis());
        print_comparison(&comparison);
        for stats in ratio_statistics(&qslope, rate) {
            println!(
                "  ratio(official DQ==0, rate>={}): n={} median(quick/rate)={:.5} MAD(rel)={:.5} within 2%={:.4} within 5%={:.4}",
                stats.min_rate, stats.n, stats.median_ratio, stats.mad_rel, stats.within[0], stats.within[1]
            );
        }
        failures.extend(gate_failures(&name, &comparison));
        if name.contains(BAND_CHECK_MARKER) {
            let band_default = compare_rate_files(&qslope, rate, &out_dir, Some(BAND_UNCAL_ROWS))
                .unwrap_or_else(|e| panic!("{name}: band compare (default): {e:#}"));
            let off_dir = forward_slashes(&products.path().join(format!("pair{index}_off")));
            let off_params = QuickSlopeParams { ref_correction: RefCorrection::Off, ..QuickSlopeParams::default() };
            let off = quick_slope_files(uncal, &off_dir, 0, &off_params, None).unwrap_or_else(|e| panic!("{name}: quick_slope_files off: {e:#}"));
            let band_off = compare_rate_files(off["fits_path"].as_str().unwrap(), rate, &off_dir, Some(BAND_UNCAL_ROWS))
                .unwrap_or_else(|e| panic!("{name}: band compare (off): {e:#}"));
            let delta_off = faint_median_delta(&band_off);
            let delta_default = faint_median_delta(&band_default);
            println!(
                "band rows {:?} -> science_rows {}: off faint median_delta={delta_off:+.4} (gate {:?}); corrected faint median_delta={delta_default:+.4} (gate {:?}); off dimensions={} warnings={}",
                BAND_UNCAL_ROWS, band_default["science_rows"], GATE_BAND_OFF_DELTA, GATE_BAND_CORRECTED_DELTA, off["dimensions"], off["warnings"]
            );
            println!("band off bins:");
            print_bins("  ", &band_off["bins"]);
            println!("band corrected bins:");
            print_bins("  ", &band_default["bins"]);
            if !within(delta_off, GATE_BAND_OFF_DELTA) {
                failures.push(format!("{name}: G4 off faint median_delta {delta_off:+.4} outside {GATE_BAND_OFF_DELTA:?}"));
            }
            if !within(delta_default, GATE_BAND_CORRECTED_DELTA) {
                failures.push(format!("{name}: G4 corrected faint median_delta {delta_default:+.4} outside {GATE_BAND_CORRECTED_DELTA:?}"));
            }
        }
        GLOBAL_CUBE_CACHE.invalidate(uncal);
    }
    assert!(failures.is_empty(), "gate failures:\n{}", failures.join("\n"));
}

fn integration_slope(integration: usize, x: usize) -> f32 {
    SYNTHETIC_SLOPE_STEP * (integration + 1) as f32 + SYNTHETIC_COLUMN_SLOPE * x as f32
}

#[test]
fn a_ramp_with_three_integrations_slopes_each_integration_independently() {
    let data = tempfile::tempdir().unwrap();
    let products = tempfile::tempdir().unwrap();
    let spec = SyntheticUncal {
        cols: SYNTHETIC_COLS,
        rows: SYNTHETIC_ROWS,
        ngroups: SYNTHETIC_NGROUPS,
        nints: THREE_INTEGRATIONS,
        irs2: None,
        tgroup_s: SYNTHETIC_TGROUP_S,
        detector: "NRS1",
        write_fastaxis: false,
    };
    let uncal = write_synthetic_uncal(&data.path().join("three_integrations_uncal.fits"), &spec, |i, g, _y, x| {
        SYNTHETIC_PEDESTAL_DN + integration_slope(i, x) * (SYNTHETIC_TGROUP_S * g as f64) as f32
    });
    let out_dir = forward_slashes(&products.path().join("products"));
    for integration in 0..THREE_INTEGRATIONS {
        let result = quick_slope_files(&uncal, &out_dir, integration, &QuickSlopeParams::default(), None).unwrap();
        assert_eq!(result["integration"].as_u64(), Some(integration as u64));
        assert_eq!(result["nints"].as_u64(), Some(THREE_INTEGRATIONS as u64));
        let fits_path = result["fits_path"].as_str().unwrap();
        assert!(
            fits_path.ends_with(&format!("three_integrations_int{:03}_qslope.fits", integration + 1)),
            "{fits_path}"
        );
        let planes = read_qslope(fits_path).unwrap();
        assert_eq!(planes.sci.dim(), (SYNTHETIC_ROWS, SYNTHETIC_COLS));
        for y in [0, SYNTHETIC_ROWS / 2, SYNTHETIC_ROWS - 1] {
            for x in 0..SYNTHETIC_COLS {
                let expected = integration_slope(integration, x);
                let got = planes.sci[[y, x]];
                assert!((got - expected).abs() < SLOPE_TOLERANCE, "integration {integration} pixel ({x}, {y}): got {got}, expected {expected}");
                assert_eq!(planes.dq[[y, x]], 0);
                assert_eq!(planes.ngood[[y, x]], SYNTHETIC_NGROUPS as i32);
            }
        }
    }
    let third = quick_slope_files(&uncal, &out_dir, 2, &QuickSlopeParams::default(), None).unwrap();
    let third_planes = read_qslope(third["fits_path"].as_str().unwrap()).unwrap();
    let first = quick_slope_files(&uncal, &out_dir, 0, &QuickSlopeParams::default(), None).unwrap();
    let first_planes = read_qslope(first["fits_path"].as_str().unwrap()).unwrap();
    let difference = third_planes.sci[[1, 0]] - first_planes.sci[[1, 0]];
    assert!((difference - 2.0 * SYNTHETIC_SLOPE_STEP).abs() < SLOPE_TOLERANCE, "{difference}");
}
