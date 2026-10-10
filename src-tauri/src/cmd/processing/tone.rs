use std::time::Instant;

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, image_ref, invalidate_written, load_from_cache_or_disk, output_stem,
    render_and_save_as, resolve_output_dir, try_extract_rgb_resolved, write_derived_fits, OutputValues,
    ResolvedRgbImage, MAX_PREVIEW_DIM,
};
use crate::cmd::helpers;
use crate::cmd::processing::curves::{build_spline, is_curve_identity, ToneCurveInput, ToneLevelsInput};
use crate::cmd::processing::local_contrast::{is_display_referred, source_header};
use crate::core::imaging::curves::{apply_curve, apply_levels, LevelsParams, SplineLut};
use crate::core::imaging::push_history;
use crate::core::imaging::stats::compute_image_stats;
use crate::core::imaging::stf::{apply_stf_f32, auto_stf, AutoStfConfig, ImageStats, StfParams};
use crate::infra::fits::writer::write_fits_rgb;
use crate::types::constants::{
    RES_CURVES_APPLIED, RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_INPUT_NORMALIZED, RES_IS_RGB,
    RES_LEVELS_APPLIED, RES_PNG_PATH,
};
use crate::types::header::HduHeader;

pub(crate) const TONE_IDENTITY_ERROR: &str = "Nothing to apply: the levels and the curve are both identity.";
pub(crate) const TONE_CUBE_ERROR: &str = "Curves work on a 2-D image; select a plane of the cube first.";
const SUFFIX_TONE: &str = "tone";

struct ToneOps {
    levels: Option<LevelsParams>,
    curve: Option<(SplineLut, usize)>,
}

impl ToneOps {
    fn parse(levels: Option<&ToneLevelsInput>, curve: Option<&ToneCurveInput>) -> anyhow::Result<Self> {
        let levels = levels.map(LevelsParams::from).filter(|l| !l.is_identity());
        let curve = curve
            .filter(|c| !is_curve_identity(c))
            .map(|c| (build_spline(c), c.points.len()));
        if levels.is_none() && curve.is_none() {
            anyhow::bail!(TONE_IDENTITY_ERROR);
        }
        Ok(Self { levels, curve })
    }

    fn apply(&self, display: &Array2<f32>) -> Array2<f32> {
        match (&self.levels, &self.curve) {
            (Some(l), Some((lut, _))) => apply_curve(&apply_levels(display, l), lut),
            (Some(l), None) => apply_levels(display, l),
            (None, Some((lut, _))) => apply_curve(display, lut),
            (None, None) => display.clone(),
        }
    }

    fn history(&self) -> String {
        let l = self.levels.clone().unwrap_or_default();
        format!(
            "tone: levels black {:.4} gamma {:.4} white {:.4}; curve {} points",
            l.black,
            l.gamma,
            l.white,
            self.curve.as_ref().map_or(0, |(_, n)| *n)
        )
    }
}

fn stf_history(stf: &StfParams) -> String {
    format!(
        "tone: linear input stretched with auto-STF (shadow {:.4} midtone {:.4} highlight {:.4})",
        stf.shadow, stf.midtone, stf.highlight
    )
}

fn tone_header(source: Option<&HduHeader>, stf: Option<&StfParams>, ops: &ToneOps) -> HduHeader {
    let mut header = derived_output_header(source, SUFFIX_TONE, OutputValues::DisplayReferred);
    if let Some(stf) = stf {
        push_history(&mut header, &stf_history(stf));
    }
    push_history(&mut header, &ops.history());
    header
}

fn keep_nonfinite(input: &Array2<f32>, mut toned: Array2<f32>) -> Array2<f32> {
    ndarray::Zip::from(&mut toned).and(input).for_each(|t, &v| {
        if !v.is_finite() {
            *t = f32::NAN;
        }
    });
    toned
}

fn normalized_json(stats: Option<&ImageStats>) -> serde_json::Value {
    match stats {
        Some(s) => json!({ "min": s.min, "max": s.max }),
        None => serde_json::Value::Null,
    }
}

pub(super) fn is_cube_header(header: Option<&HduHeader>) -> bool {
    header
        .and_then(|h| h.get_i64("NAXIS3"))
        .is_some_and(|n| n > 1)
}

pub(super) fn has_only_cube_planes(path: &str) -> bool {
    let r = image_ref(path);
    if r.is_synthetic() {
        return false;
    }
    std::fs::File::open(&r.path)
        .ok()
        .and_then(|file| crate::infra::fits::reader::list_extensions(&file).ok())
        .is_some_and(|hdus| {
            let mut planes = hdus.iter().filter(|h| h.has_data).peekable();
            planes.peek().is_some() && planes.all(|h| h.naxis >= 3 && h.naxis3 > 1)
        })
}

fn response(
    png_path: String,
    fits_path: String,
    dims: (usize, usize),
    is_rgb: bool,
    ops: &ToneOps,
    stats: Option<&ImageStats>,
    t0: Instant,
) -> serde_json::Value {
    let (rows, cols) = dims;
    json!({
        RES_PNG_PATH: png_path,
        RES_FITS_PATH: fits_path,
        RES_DIMENSIONS: [cols, rows],
        RES_IS_RGB: is_rgb,
        RES_LEVELS_APPLIED: ops.levels.is_some(),
        RES_CURVES_APPLIED: ops.curve.is_some(),
        RES_INPUT_NORMALIZED: normalized_json(stats),
        RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
    })
}

fn run_tone_mono(path: &str, output_dir: &str, ops: &ToneOps, t0: Instant) -> anyhow::Result<serde_json::Value> {
    let entry = match load_from_cache_or_disk(path) {
        Ok(entry) => entry,
        Err(_) if has_only_cube_planes(path) => anyhow::bail!(TONE_CUBE_ERROR),
        Err(e) => return Err(e),
    };
    let source = source_header(path, &entry);
    if is_cube_header(source.as_ref()) {
        anyhow::bail!(TONE_CUBE_ERROR);
    }

    let (toned, stf) = if is_display_referred(source.as_ref()) {
        (keep_nonfinite(entry.arr(), ops.apply(entry.arr())), None)
    } else {
        let stats = compute_image_stats(entry.arr());
        let stf = auto_stf(&stats, &AutoStfConfig::default());
        let toned = ops.apply(&apply_stf_f32(entry.arr(), &stf, &stats));
        (keep_nonfinite(entry.arr(), toned), Some((stf, stats)))
    };

    let ro = render_and_save_as(&toned, path, output_dir, SUFFIX_TONE, false, OutputValues::DisplayReferred)?;
    let header = tone_header(source.as_ref(), stf.as_ref().map(|(s, _)| s), ops);
    let fits_path = format!("{}/{}_{}.fits", output_dir, output_stem(path), SUFFIX_TONE);
    write_derived_fits(&fits_path, &toned, Some(&header))?;
    Ok(response(ro.png_path, fits_path, ro.dims, false, ops, stf.as_ref().map(|(_, s)| s), t0))
}

fn run_tone_rgb(rgb: &ResolvedRgbImage, path: &str, output_dir: &str, ops: &ToneOps, t0: Instant) -> anyhow::Result<serde_json::Value> {
    let source = Some(&rgb.header);
    let planes = [&rgb.r, &rgb.g, &rgb.b];
    let (toned, stf) = if is_display_referred(source) {
        (planes.map(|p| keep_nonfinite(p, ops.apply(p))), None)
    } else {
        let stats = planes.map(compute_image_stats);
        let (stf, combined) =
            helpers::compute_linked_stf_with_stats(&stats[0], &stats[1], &stats[2], &AutoStfConfig::default());
        (planes.map(|p| keep_nonfinite(p, ops.apply(&apply_stf_f32(p, &stf, &combined)))), Some((stf, combined)))
    };
    let [r, g, b] = toned;

    let stem = output_stem(path);
    let png_path = format!("{}/{}_{}.png", output_dir, stem, SUFFIX_TONE);
    let fits_path = format!("{}/{}_{}.fits", output_dir, stem, SUFFIX_TONE);
    let unit = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    helpers::render_rgb_preview_with_stf(&r, &g, &b, unit, unit, unit, &png_path, MAX_PREVIEW_DIM)?;

    let header = tone_header(source, stf.as_ref().map(|(s, _)| s), ops);
    crate::core::cube::cache::GLOBAL_CUBE_CACHE.invalidate(&fits_path);
    write_fits_rgb(&fits_path, &r, &g, &b, Some(&header))?;
    invalidate_written(&fits_path);
    Ok(response(png_path, fits_path, r.dim(), true, ops, stf.as_ref().map(|(_, s)| s), t0))
}

#[tauri::command]
pub async fn apply_tone_cmd(
    path: String,
    output_dir: String,
    levels: Option<ToneLevelsInput>,
    curve: Option<ToneCurveInput>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        let ops = ToneOps::parse(levels.as_ref(), curve.as_ref())?;
        let output_dir = resolve_output_dir(&output_dir)?;
        let rgb = if image_ref(&path).is_synthetic() { None } else { try_extract_rgb_resolved(&path)? };
        match rgb {
            Some(rgb) => run_tone_rgb(&rgb, &path, &output_dir, &ops, t0),
            None => run_tone_mono(&path, &output_dir, &ops, t0),
        }
    })
}

#[cfg(test)]
pub(crate) fn write_cube(path: &std::path::Path, cols: usize, rows: usize, depth: usize) {
    use std::io::Write;

    use crate::types::constants::BLOCK_SIZE;

    let mut header = Vec::new();
    let mut push = |key: &str, value: &str| {
        let mut card = format!("{key:<8}= {value:>20}").into_bytes();
        card.resize(80, b' ');
        header.extend_from_slice(&card);
    };
    push("SIMPLE", "T");
    push("BITPIX", "-32");
    push("NAXIS", "3");
    push("NAXIS1", &cols.to_string());
    push("NAXIS2", &rows.to_string());
    push("NAXIS3", &depth.to_string());
    push("CTYPE3", "'WAVE'");
    push("END", "");
    while header.len() % BLOCK_SIZE != 0 {
        header.push(b' ');
    }
    let mut data: Vec<u8> = (0..cols * rows * depth).flat_map(|i| (i as f32 + 1.0).to_be_bytes()).collect();
    while data.len() % BLOCK_SIZE != 0 {
        data.push(0);
    }
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(&header).unwrap();
    file.write_all(&data).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cmd::common::{cached_header, extract_image_resolved, HEADER_ABPROC, HEADER_DISPLAY_REFERRED};
    use crate::infra::fits::writer::write_fits_mono;

    struct Lcg(u64);

    impl Lcg {
        fn uniform(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }

        fn gaussian(&mut self) -> f32 {
            let s: f32 = (0..4).map(|_| self.uniform()).sum();
            (s - 2.0) * 3.0f32.sqrt()
        }
    }

    fn star_field(seed: u64) -> Array2<f32> {
        let mut rng = Lcg(seed);
        let mut arr = Array2::from_shape_fn((64, 64), |_| 1000.0 + 5.0 * rng.gaussian());
        for (y, x) in [(10usize, 12usize), (40, 20), (25, 50), (55, 58)] {
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let peak = if dy == 0 && dx == 0 { 20000.0 } else { 6000.0 };
                    arr[[(y as i64 + dy) as usize, (x as i64 + dx) as usize]] = peak;
                }
            }
        }
        arr
    }

    fn levels() -> ToneLevelsInput {
        ToneLevelsInput { black: 0.1, gamma: 1.0, white: 0.9 }
    }

    fn curve() -> ToneCurveInput {
        ToneCurveInput { points: vec![[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]] }
    }

    fn expected_tone(display: &Array2<f32>) -> Array2<f32> {
        let leveled = apply_levels(display, &LevelsParams::from(&levels()));
        apply_curve(&leveled, &SplineLut::from_points(&[(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)]))
    }

    fn assert_close(actual: &Array2<f32>, expected: &Array2<f32>, tol: f32) {
        assert_eq!(actual.dim(), expected.dim());
        let worst = actual
            .iter()
            .zip(expected.iter())
            .map(|(a, e)| (a - e).abs())
            .fold(0.0f32, f32::max);
        assert!(worst <= tol, "max |diff| {worst}");
    }

    fn history(header: &HduHeader) -> Vec<String> {
        header
            .cards
            .iter()
            .filter(|(k, _)| k.trim() == "HISTORY")
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn text<'a>(header: &'a HduHeader, key: &str) -> Option<&'a str> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim())
    }

    #[tokio::test]
    async fn apply_tone_on_a_linear_mono_file_stretches_then_applies_levels_and_curve() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("linear_sky.fits").to_str().unwrap().to_string();
        let input = star_field(11);
        write_fits_mono(&src, &input, None).unwrap();

        let value = apply_tone_cmd(src.clone(), out.clone(), Some(levels()), Some(curve())).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        let written = extract_image_resolved(&fits).unwrap().arr;
        let header = cached_header(&fits).unwrap();

        let stats = compute_image_stats(&input);
        let stf = auto_stf(&stats, &AutoStfConfig::default());
        let expected = expected_tone(&apply_stf_f32(&input, &stf, &stats));
        assert_close(&written, &expected, 1e-4);

        let out_stats = compute_image_stats(&written);
        assert!((0.1..=0.6).contains(&out_stats.median), "median {}", out_stats.median);
        let positive = written.iter().filter(|v| **v > 0.0).count();
        assert!(positive * 2 >= written.len(), "{positive} of {} pixels > 0", written.len());

        assert_eq!(text(&header, HEADER_DISPLAY_REFERRED), Some("T"));
        assert_eq!(text(&header, HEADER_ABPROC), Some("tone"));
        let notes = history(&header);
        assert!(notes.iter().any(|h| h.contains("linear input stretched with auto-STF")), "{notes:?}");
        assert!(notes.iter().all(|h| h.len() <= 72), "{notes:?}");
        assert!(notes.join(" ").contains(&stf_history(&stf)), "{notes:?}");
        assert_eq!(value[RES_IS_RGB], false);
        assert_eq!(value[RES_LEVELS_APPLIED], true);
        assert_eq!(value[RES_CURVES_APPLIED], true);
        assert_eq!(value[RES_DIMENSIONS], json!([64, 64]));
        assert!((value[RES_INPUT_NORMALIZED]["max"].as_f64().unwrap() - stats.max).abs() < 1e-6, "{value}");
        assert!(std::path::Path::new(value[RES_PNG_PATH].as_str().unwrap()).exists());
    }

    #[tokio::test]
    async fn apply_tone_keeps_display_referred_input_unstretched() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("display.fits").to_str().unwrap().to_string();
        let input = Array2::from_shape_fn((32, 32), |(y, x)| ((y * 32 + x) as f32) / 1023.0);
        let mut header = HduHeader::empty();
        header.set(HEADER_DISPLAY_REFERRED, "T".to_string());
        write_fits_mono(&src, &input, Some(&header)).unwrap();

        let value = apply_tone_cmd(src, out, Some(levels()), Some(curve())).await.unwrap();
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        let written = extract_image_resolved(&fits).unwrap().arr;
        let header = cached_header(&fits).unwrap();

        assert_close(&written, &expected_tone(&input), 1e-6);
        let notes = history(&header);
        assert!(notes.iter().all(|h| !h.contains("auto-STF")), "{notes:?}");
        assert!(notes.iter().any(|h| h.contains("levels black 0.1000 gamma 1.0000 white 0.9000; curve 3 points")), "{notes:?}");
        assert!(value[RES_INPUT_NORMALIZED].is_null(), "{value}");
        assert_eq!(text(&header, HEADER_DISPLAY_REFERRED), Some("T"));
    }

    #[tokio::test]
    async fn apply_tone_rejects_identity_and_cubes() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("flat.fits").to_str().unwrap().to_string();
        write_fits_mono(&src, &star_field(3), None).unwrap();

        let none = apply_tone_cmd(src.clone(), out.clone(), None, None).await;
        assert_eq!(none.unwrap_err(), TONE_IDENTITY_ERROR);
        let identity_levels = ToneLevelsInput { black: 0.0, gamma: 1.0, white: 1.0 };
        let identity_curve = ToneCurveInput { points: vec![[0.0, 0.0], [1.0, 1.0]] };
        let explicit = apply_tone_cmd(src.clone(), out.clone(), Some(identity_levels), Some(identity_curve)).await;
        assert_eq!(explicit.unwrap_err(), TONE_IDENTITY_ERROR);

        let cube = dir.path().join("cube.fits");
        write_cube(&cube, 8, 6, 5);
        let refused = apply_tone_cmd(cube.to_str().unwrap().to_string(), out.clone(), Some(levels()), None).await;
        assert_eq!(refused.unwrap_err(), TONE_CUBE_ERROR);
        let plane = apply_tone_cmd(format!("{}#hdu=0", cube.to_str().unwrap()), out, Some(levels()), None).await;
        assert_eq!(plane.unwrap_err(), TONE_CUBE_ERROR);
    }

    #[tokio::test]
    async fn apply_tone_on_an_rgb_fits_writes_three_planes() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("colour.fits").to_str().unwrap().to_string();
        let mut r = star_field(21);
        r[[2, 3]] = f32::NAN;
        let g = star_field(22).mapv(|v| v * 1.5);
        let b = star_field(23).mapv(|v| v * 0.5);
        write_fits_rgb(&src, &r, &g, &b, None).unwrap();

        let value = apply_tone_cmd(src, out, Some(levels()), Some(curve())).await.unwrap();
        assert_eq!(value[RES_IS_RGB], true);
        assert_eq!(value[RES_DIMENSIONS], json!([64, 64]));
        let fits = value[RES_FITS_PATH].as_str().unwrap().to_string();
        let written = try_extract_rgb_resolved(&fits).unwrap().expect("a 3-plane RGB FITS");

        let (sr, sg, sb) = (compute_image_stats(&r), compute_image_stats(&g), compute_image_stats(&b));
        let (stf, combined) = helpers::compute_linked_stf_with_stats(&sr, &sg, &sb, &AutoStfConfig::default());
        assert_close(&written.b, &expected_tone(&apply_stf_f32(&b, &stf, &combined)), 1e-4);
        assert_close(&written.r, &expected_tone(&apply_stf_f32(&r, &stf, &combined)), 1e-4);

        let notes = history(&written.header);
        assert_eq!(notes.iter().filter(|h| h.contains("auto-STF")).count(), 1, "{notes:?}");
        assert!(notes.iter().all(|h| h.len() <= 72), "{notes:?}");
        assert!(notes.join(" ").contains(&stf_history(&stf)), "{notes:?}");
        assert!(written.r[[2, 3]].is_nan(), "{}", written.r[[2, 3]]);
        assert_eq!(written.r.iter().filter(|v| v.is_nan()).count(), 1);
        assert_eq!(text(&written.header, HEADER_DISPLAY_REFERRED), Some("T"));
        assert_eq!(text(&written.header, HEADER_ABPROC), Some("tone"));
        assert!(std::path::Path::new(value[RES_PNG_PATH].as_str().unwrap()).exists());
    }

    #[tokio::test]
    async fn tone_keeps_nan_padding_as_nan() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let holes = [(0usize, 0usize), (3, 4), (15, 9)];

        let display_src = dir.path().join("display_nan.fits").to_str().unwrap().to_string();
        let mut display = Array2::from_shape_fn((16, 16), |(y, x)| ((y * 16 + x) as f32) / 255.0);
        for ix in holes {
            display[ix] = f32::NAN;
        }
        let mut header = HduHeader::empty();
        header.set(HEADER_DISPLAY_REFERRED, "T".to_string());
        write_fits_mono(&display_src, &display, Some(&header)).unwrap();

        let linear_src = dir.path().join("linear_nan.fits").to_str().unwrap().to_string();
        let mut linear = star_field(5);
        for ix in holes {
            linear[ix] = f32::NAN;
        }
        write_fits_mono(&linear_src, &linear, None).unwrap();

        for src in [display_src, linear_src] {
            let value = apply_tone_cmd(src.clone(), out.clone(), Some(levels()), Some(curve())).await.unwrap();
            let written = extract_image_resolved(value[RES_FITS_PATH].as_str().unwrap()).unwrap().arr;
            for ix in holes {
                assert!(written[ix].is_nan(), "{src}: {:?} -> {}", ix, written[ix]);
            }
            assert_eq!(written.iter().filter(|v| !v.is_finite()).count(), holes.len(), "{src}");
        }
    }

    #[test]
    fn cube_detection_ignores_files_with_a_two_d_plane() {
        use crate::core::cube::lazy::test_support::{f32_samples, image_hdu, write_bytes};

        let dir = tempfile::tempdir().unwrap();
        let mixed = dir.path().join("image_then_cube.fits");
        let mut bytes = image_hdu("SIMPLE", &[8, 6], -32, &[("EXTEND", "T")], f32_samples(8, 6, 1, |_, y, x| (y * 8 + x) as f32));
        bytes.extend(image_hdu("XTENSION", &[8, 6, 5], -32, &[("EXTNAME", "'CUBE'")], f32_samples(8, 6, 5, |z, _, _| z as f32)));
        write_bytes(&mixed, &bytes);
        assert!(!has_only_cube_planes(mixed.to_str().unwrap()));

        let bare = dir.path().join("bare_cube.fits");
        write_cube(&bare, 8, 6, 5);
        assert!(has_only_cube_planes(bare.to_str().unwrap()));

        let headerless = dir.path().join("empty_primary_then_cube.fits");
        let mut bytes = image_hdu("SIMPLE", &[], 8, &[("EXTEND", "T")], Vec::new());
        bytes.extend(image_hdu("XTENSION", &[8, 6, 5], -32, &[("EXTNAME", "'SCI'")], f32_samples(8, 6, 5, |z, _, _| z as f32)));
        write_bytes(&headerless, &bytes);
        assert!(has_only_cube_planes(headerless.to_str().unwrap()));
    }
}
