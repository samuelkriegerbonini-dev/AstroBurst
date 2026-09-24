use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};

use ndarray::Array2;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, load_cached, load_cached_full, output_stem, render_and_save_as,
    resolve_output_dir, write_derived_fits, OutputValues, RenderOutput, HEADER_DISPLAY_REFERRED, MAX_PREVIEW_DIM,
};
use crate::cmd::helpers;
use crate::core::imaging::local_contrast::{lhe_rgb, lhe_with_progress, LheConfig};
use crate::infra::cache::ImageEntry;
use crate::infra::progress::ProgressHandle;
use crate::types::constants::{RES_DIMENSIONS, RES_ELAPSED_MS, RES_FITS_PATH, RES_PNG_PATH};
use crate::types::header::HduHeader;

pub const EVENT_LHE_PROGRESS: &str = "lhe-progress";
pub const SUFFIX_LHE: &str = "lhe";
pub const ABPROC_LHE: &str = "lhe";
pub const COMPOSITE_STRETCH_REQUIRED: &str =
    "No stretched composite is loaded: run a stretch on the composite first";

pub(crate) fn is_display_referred(source: Option<&HduHeader>) -> bool {
    source
        .and_then(|h| h.get(HEADER_DISPLAY_REFERRED))
        .is_some_and(|v| v.trim().trim_matches('\'').trim() == "T")
}

pub(crate) fn output_values_for(source: Option<&HduHeader>, fallback: OutputValues) -> OutputValues {
    if is_display_referred(source) { OutputValues::DisplayReferred } else { fallback }
}

pub(crate) fn processed_header(source: Option<&HduHeader>, abproc: &str) -> HduHeader {
    derived_output_header(source, abproc, output_values_for(source, OutputValues::Rescaled))
}

pub(crate) fn save_contrast_output(
    arr: &Array2<f32>,
    path: &str,
    output_dir: &str,
    suffix: &str,
    abproc: &str,
    source: Option<&HduHeader>,
) -> anyhow::Result<(RenderOutput, String)> {
    let rendered = render_and_save_as(arr, path, output_dir, suffix, false, output_values_for(source, OutputValues::Rescaled))?;
    let fits_path = processed_fits_path(path, output_dir, suffix);
    write_derived_fits(&fits_path, arr, Some(&processed_header(source, abproc)))?;
    Ok((rendered, fits_path))
}

type Planes = [Arc<Array2<f32>>; 3];
type Triplet = (Array2<f32>, Array2<f32>, Array2<f32>);
type ContrastFn = Arc<dyn Fn(&Array2<f32>, &Array2<f32>, &Array2<f32>) -> anyhow::Result<Triplet> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompositeTier {
    Toned,
    Stretched,
}

#[derive(Clone)]
struct ContrastStep {
    op: &'static str,
    apply: ContrastFn,
}

struct ContrastRun {
    tier: CompositeTier,
    steps: Vec<ContrastStep>,
    last_input: Planes,
    output: [Weak<Array2<f32>>; 3],
}

impl ContrastRun {
    fn holds(&self, tier: CompositeTier, current: &Planes) -> bool {
        self.tier == tier
            && self.output.iter().zip(current).all(|(out, cur)| out.upgrade().is_some_and(|o| Arc::ptr_eq(&o, cur)))
    }
}

static LAST_CONTRAST_RUN: LazyLock<Mutex<Option<ContrastRun>>> = LazyLock::new(|| Mutex::new(None));

fn lock_last_run() -> MutexGuard<'static, Option<ContrastRun>> {
    LAST_CONTRAST_RUN.lock().unwrap_or_else(|e| e.into_inner())
}

fn planes_of((r, g, b): (ImageEntry, ImageEntry, ImageEntry)) -> Planes {
    [r.data_arc(), g.data_arc(), b.data_arc()]
}

fn load_tier(tier: CompositeTier) -> Option<Planes> {
    match tier {
        CompositeTier::Toned => helpers::load_composite_toned(),
        CompositeTier::Stretched => helpers::load_composite_stretched(),
    }
    .map(planes_of)
}

fn displayed_composite() -> anyhow::Result<(CompositeTier, Planes)> {
    [CompositeTier::Toned, CompositeTier::Stretched]
        .into_iter()
        .find_map(|tier| load_tier(tier).map(|planes| (tier, planes)))
        .ok_or_else(|| anyhow::anyhow!(COMPOSITE_STRETCH_REQUIRED))
}

fn next_chain(last: Option<&ContrastRun>, tier: CompositeTier, current: Planes, step: ContrastStep) -> (Vec<ContrastStep>, Planes) {
    match last.filter(|run| run.holds(tier, &current)) {
        Some(run) if run.steps.last().is_some_and(|s| s.op == step.op) => {
            let mut steps = run.steps.clone();
            steps.pop();
            steps.push(step);
            (steps, run.last_input.clone())
        }
        Some(run) => {
            let mut steps = run.steps.clone();
            steps.push(step);
            (steps, current)
        }
        None => (vec![step], current),
    }
}

fn store_tier(tier: CompositeTier, r: Array2<f32>, g: Array2<f32>, b: Array2<f32>) -> Option<Planes> {
    match tier {
        CompositeTier::Toned => helpers::insert_composite_toned(r, g, b),
        CompositeTier::Stretched => helpers::insert_composite_stretched(r, g, b),
    }
    load_tier(tier)
}

fn record_run(tier: CompositeTier, steps: Vec<ContrastStep>, last_input: Planes, stored: Option<Planes>) {
    *lock_last_run() = stored.map(|output| ContrastRun {
        tier,
        steps,
        last_input,
        output: output.each_ref().map(Arc::downgrade),
    });
}

pub(crate) fn forget_composite_contrast() {
    *lock_last_run() = None;
}

pub(crate) fn run_composite_contrast(
    op: &'static str,
    output_dir: &str,
    apply: impl Fn(&Array2<f32>, &Array2<f32>, &Array2<f32>) -> anyhow::Result<Triplet> + Send + Sync + 'static,
) -> anyhow::Result<serde_json::Value> {
    let (tier, current) = displayed_composite()?;
    let step = ContrastStep { op, apply: Arc::new(apply) };
    let (steps, input) = next_chain(lock_last_run().as_ref(), tier, current, step.clone());

    let t0 = std::time::Instant::now();
    let (r, g, b) = (step.apply)(&input[0], &input[1], &input[2])?;
    let elapsed_ms = t0.elapsed().as_millis() as u64;
    let (rows, cols) = r.dim();

    let png_path = composite_png_path(output_dir, op);
    helpers::render_rgb_preview(&r, &g, &b, &png_path, MAX_PREVIEW_DIM)?;
    let stored = store_tier(tier, r, g, b);
    record_run(tier, steps, input, stored);

    Ok(json!({
        RES_PNG_PATH: png_path,
        RES_ELAPSED_MS: elapsed_ms,
        RES_DIMENSIONS: [cols, rows],
    }))
}

pub(crate) fn store_tone_result(r: Array2<f32>, g: Array2<f32>, b: Array2<f32>, png_path: &str) -> anyhow::Result<Vec<&'static str>> {
    let toned = load_tier(CompositeTier::Toned);
    let steps = toned
        .and_then(|current| lock_last_run().as_ref().filter(|run| run.holds(CompositeTier::Toned, &current)).map(|run| run.steps.clone()))
        .unwrap_or_default();

    let mut planes = (r, g, b);
    let mut last_input = None;
    for step in &steps {
        let next = (step.apply)(&planes.0, &planes.1, &planes.2)
            .map_err(|e| e.context(format!("Could not re-apply the composite {} step on the new tone settings", step.op)))?;
        last_input = Some(std::mem::replace(&mut planes, next));
    }

    helpers::render_rgb_preview(&planes.0, &planes.1, &planes.2, png_path, MAX_PREVIEW_DIM)?;
    let stored = store_tier(CompositeTier::Toned, planes.0, planes.1, planes.2);
    let reapplied = steps.iter().map(|s| s.op).collect();
    match last_input {
        Some((ir, ig, ib)) => record_run(CompositeTier::Toned, steps, [Arc::new(ir), Arc::new(ig), Arc::new(ib)], stored),
        None => *lock_last_run() = None,
    }
    Ok(reapplied)
}

pub(crate) fn source_header(path: &str, entry: &ImageEntry) -> Option<HduHeader> {
    if let Some(h) = entry.header() {
        return Some(h.clone());
    }
    load_cached_full(path)
        .ok()
        .and_then(|e| e.header().cloned())
}

pub(crate) fn render_linear_output(
    arr: &Array2<f32>,
    path: &str,
    entry: &ImageEntry,
    output_dir: &str,
    suffix: &str,
) -> anyhow::Result<RenderOutput> {
    let values = output_values_for(source_header(path, entry).as_ref(), OutputValues::Linear);
    render_and_save_as(arr, path, output_dir, suffix, true, values)
}

pub(crate) fn processed_fits_path(path: &str, output_dir: &str, suffix: &str) -> String {
    format!("{}/{}_{}.fits", output_dir, output_stem(path), suffix)
}

pub(crate) fn composite_png_path(output_dir: &str, suffix: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}/composite_{}_{}.png", output_dir, suffix, ts)
}

#[tauri::command]
pub async fn lhe_cmd(
    app: tauri::AppHandle,
    path: String,
    output_dir: String,
    config: LheConfig,
) -> Result<serde_json::Value, String> {
    let progress = ProgressHandle::new(&app, EVENT_LHE_PROGRESS, 1);
    let progress_clone = progress.clone();

    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        let entry = load_cached(&path)?;
        let header = source_header(&path, &entry);

        let t0 = std::time::Instant::now();
        let result = lhe_with_progress(entry.arr(), &config, Some(&progress_clone))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let (ro, fits_path) =
            save_contrast_output(&result.image, &path, &output_dir, SUFFIX_LHE, ABPROC_LHE, header.as_ref())?;
        let (rows, cols) = ro.dims;

        Ok(json!({
            RES_PNG_PATH: ro.png_path,
            RES_FITS_PATH: fits_path,
            RES_ELAPSED_MS: elapsed_ms,
            RES_DIMENSIONS: [cols, rows],
        }))
    })
}

#[tauri::command]
pub async fn lhe_composite_cmd(
    output_dir: String,
    config: LheConfig,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        run_composite_contrast(SUFFIX_LHE, &output_dir, move |r, g, b| lhe_rgb(r, g, b, &config))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cmd::common::HEADER_ABPROC;
    use crate::cmd::processing::{hdrmt_composite_cmd, ABPROC_HDRMT, SUFFIX_HDR};
    use crate::core::imaging::hdr::{hdrmt_rgb, HdrConfig};

    #[test]
    fn processed_header_adds_abproc_and_keeps_source_cards() {
        let mut source = HduHeader::empty();
        source.set("CRPIX1", "128.0".to_string());
        source.set("OBJECT", "M42".to_string());
        let header = processed_header(Some(&source), ABPROC_LHE);
        assert_eq!(header.get(HEADER_ABPROC), Some("lhe"));
        assert_eq!(header.get("CRPIX1"), Some("128.0"));
        assert_eq!(header.get("OBJECT"), Some("M42"));
        assert!(header.cards.iter().any(|(k, _)| k == HEADER_ABPROC));

        let bare = processed_header(None, "hdrmt");
        assert_eq!(bare.get(HEADER_ABPROC), Some("hdrmt"));
        assert_eq!(bare.cards.len(), 1);
    }

    #[test]
    fn a_non_linear_contrast_output_no_longer_claims_the_source_calibration() {
        let mut source = HduHeader::empty();
        for (k, v) in [
            ("BUNIT", "'MJy/sr'"),
            ("PIXAR_SR", "2.1E-13"),
            ("PHOTMJSR", "1.5"),
            ("MAGZERO", "28.0"),
            ("SATURATE", "60000"),
            ("CRVAL1", "83.8"),
            ("CHECKSUM", "'abc'"),
        ] {
            source.set(k, v.to_string());
        }
        for abproc in [ABPROC_LHE, "hdrmt"] {
            let header = processed_header(Some(&source), abproc);
            for dropped in ["BUNIT", "PIXAR_SR", "PHOTMJSR", "MAGZERO", "SATURATE", "CHECKSUM"] {
                assert!(header.get(dropped).is_none(), "{dropped} survived {abproc}");
            }
            assert_eq!(header.get("CRVAL1"), Some("83.8"), "WCS dropped by {abproc}");
            assert!(header.get(HEADER_DISPLAY_REFERRED).is_none());
        }

        source.set(HEADER_DISPLAY_REFERRED, "T".to_string());
        assert_eq!(processed_header(Some(&source), ABPROC_LHE).get(HEADER_DISPLAY_REFERRED), Some("T"));
    }

    fn stretched_plane() -> Array2<f32> {
        Array2::from_shape_fn((12, 10), |(y, x)| 0.05 + (y * 10 + x) as f32 / 150.0)
    }

    fn png_pixels(path: &str) -> Vec<u8> {
        image::open(path).unwrap().to_luma8().into_raw()
    }

    #[test]
    fn a_contrast_output_of_display_referred_data_is_previewed_as_computed() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = dir.path().join("m42_arcsinh.fits").to_str().unwrap().to_string();
        let data = stretched_plane();
        let mut display = HduHeader::empty();
        display.set(HEADER_DISPLAY_REFERRED, "T".to_string());

        let (rendered, fits_path) = save_contrast_output(&data, &src, &out, SUFFIX_LHE, ABPROC_LHE, Some(&display)).unwrap();
        let as_computed: Vec<u8> = data.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        assert_eq!(png_pixels(&rendered.png_path), as_computed, "the LHE preview was stretched again");
        let written = crate::cmd::common::cached_header(&fits_path).unwrap();
        assert!(is_display_referred(Some(&written)), "the FITS lost its display-referred flag");
        assert_eq!(written.get(HEADER_ABPROC).map(|v| v.trim().trim_matches('\'').trim()), Some(ABPROC_LHE));

        let (linear, _) = save_contrast_output(&data, &src, &out, SUFFIX_HDR, ABPROC_HDRMT, None).unwrap();
        assert_ne!(png_pixels(&linear.png_path), as_computed, "linear data must keep the auto-STF preview");
    }

    #[tokio::test]
    async fn forgetting_the_composite_contrast_run_releases_the_input_it_holds() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let planes = [composite_plane(4), composite_plane(5), composite_plane(6)];
        helpers::insert_composite_stretched(planes[0].clone(), planes[1].clone(), planes[2].clone());
        let run = lhe_composite_cmd(dir.path().to_str().unwrap().to_string(), LheConfig { kernel_radius: 3, ..LheConfig::default() }).await;
        let held = lock_last_run().is_some();
        forget_composite_contrast();
        let released = lock_last_run().is_none();
        helpers::clear_composite_derived();
        run.unwrap();
        assert!(held, "precondition: the run keeps its input for a re-run");
        assert!(released, "the last composite LHE/HDRMT input stayed in memory");
    }

    fn composite_plane(seed: usize) -> Array2<f32> {
        Array2::from_shape_fn((16, 16), |(y, x)| ((y * 16 + x + seed * 5) % 37) as f32 / 40.0 + 0.05)
    }

    fn toned_tier() -> Planes {
        load_tier(CompositeTier::Toned).expect("toned tier")
    }

    fn same_planes(actual: &Planes, expected: &(Array2<f32>, Array2<f32>, Array2<f32>)) -> bool {
        actual[0].as_ref() == &expected.0 && actual[1].as_ref() == &expected.1 && actual[2].as_ref() == &expected.2
    }

    #[tokio::test]
    async fn composite_contrast_works_on_the_toned_image_and_a_re_run_replaces_its_previous_result() {
        let _guard = helpers::composite_test_lock().await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let stretched = [composite_plane(1), composite_plane(2), composite_plane(3)];
        let toned = stretched.clone().map(|p| p.mapv(f32::sqrt));
        helpers::insert_composite_stretched(stretched[0].clone(), stretched[1].clone(), stretched[2].clone());
        helpers::insert_composite_toned(toned[0].clone(), toned[1].clone(), toned[2].clone());

        let first = LheConfig { kernel_radius: 3, ..LheConfig::default() };
        let second = LheConfig { kernel_radius: 5, amount: 0.5, ..LheConfig::default() };
        let hdr = HdrConfig { layers: 2, ..HdrConfig::default() };

        let run_first = lhe_composite_cmd(out.clone(), first.clone()).await;
        let after_first = load_tier(CompositeTier::Toned);
        let stretched_after = load_tier(CompositeTier::Stretched);
        let run_second = lhe_composite_cmd(out.clone(), second.clone()).await;
        let after_second = load_tier(CompositeTier::Toned);
        let run_hdr = hdrmt_composite_cmd(out, hdr.clone()).await;
        let after_hdr = toned_tier();
        helpers::clear_composite_derived();
        run_first.unwrap();
        run_second.unwrap();
        run_hdr.unwrap();

        let expected_first = lhe_rgb(&toned[0], &toned[1], &toned[2], &first).unwrap();
        assert!(same_planes(&after_first.expect("the curves tier was dropped"), &expected_first), "LHE ran on the pre-curve stretch");
        let stretched_after = stretched_after.expect("stretched tier");
        assert!(stretched_after.iter().zip(&stretched).all(|(a, e)| a.as_ref() == e), "the base stretch was overwritten");

        let expected_second = lhe_rgb(&toned[0], &toned[1], &toned[2], &second).unwrap();
        assert!(same_planes(&after_second.expect("toned tier"), &expected_second), "the re-run compounded on its own output");

        let expected_hdr = hdrmt_rgb(&expected_second.0, &expected_second.1, &expected_second.2, &hdr).unwrap();
        assert!(same_planes(&after_hdr, &expected_hdr), "HDR after LHE must chain on the LHE result");
    }

    #[test]
    fn output_paths_follow_the_stem_suffix_convention() {
        assert_eq!(
            processed_fits_path("C:/d/jw_cal.fits#hdu=3", "C:/out", SUFFIX_LHE),
            "C:/out/jw_cal_hdu3_lhe.fits"
        );
        let png = composite_png_path("C:/out", SUFFIX_LHE);
        assert!(png.starts_with("C:/out/composite_lhe_"));
        assert!(png.ends_with(".png"));
    }

    #[test]
    fn lhe_config_deserialises_camel_case_with_defaults() {
        let cfg: LheConfig = serde_json::from_str(r#"{"kernelRadius":32,"histBits":10}"#).unwrap();
        assert_eq!(cfg.kernel_radius, 32);
        assert_eq!(cfg.hist_bits, 10);
        assert_eq!(cfg.contrast_limit, 2.0);
        assert_eq!(cfg.amount, 1.0);
        assert!(cfg.circular);
    }
}
