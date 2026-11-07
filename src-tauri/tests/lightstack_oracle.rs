#[path = "lightstack_oracle/d1.rs"]
mod d1;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use astroburst_lib::core::alignment::wcs_reproject::{prefilter_coarse_target, reproject_to_grid};
use astroburst_lib::core::astrometry::wcs::WcsTransform;
use astroburst_lib::core::cubeprep::align::{
    align_image, choose_reference, plan_alignment, AlignGrid, AlignInput, AlignOptions,
    ReferenceCandidate, ReferencePolicy, ResampleMethod,
};
use astroburst_lib::core::cubeprep::cube::{
    apply_nan_mask, assemble_cube, brightest_pixel, cube_base_header, drop_invalid_planes,
    planes_from_header, remove_filter, subcube, update_metadata, Cube, CubeInput, CubeLayout,
    CubeOptions, CubePlane, MaskMode, PlaneOrder, DEFAULT_FRAC_VALID,
};
use astroburst_lib::core::cubeprep::footprint::{
    common_polygon, image_footprint, lightstack_square_rect, lightstack_wcs, polygon_contains,
    square_polygon, CropFrame, CropShape, SkyPoint,
};
use astroburst_lib::core::cubeprep::kernel::{
    build_kernel, diagnose_kernel, kernel_file_name, wiener, EpsMode, KernelMethod, KernelOptions,
    KernelShape,
};
use astroburst_lib::core::cubeprep::match_cube::{
    lightstack_history, match_cube, MatchMode, MatchOptions, PlaneKernel,
};
use astroburst_lib::core::cubeprep::psf_io::{
    decode_planes_f64, fit_gaussian, image_hdus, lightstack_pad, lightstack_resample, load_psf,
    prepare_psf, wcs_scale, PsfPrepareOptions, PsfResampleMethod, GAUSSIAN_SIGMA_TO_FWHM,
};
use astroburst_lib::core::imaging::convolve::{convolve_fft, ConvolveOptions, FftPlanCache};
use astroburst_lib::core::imaging::cutout::fraction_on_image;
use astroburst_lib::core::imaging::psf_estimation::{
    build_empirical_psf, stars_from_regions, EmpiricalPsfOptions, PsfStack, PsfStar, RadiusUnit,
};
use astroburst_lib::core::imaging::region::PhysicalMap;
use astroburst_lib::core::imaging::region_file::parse_reg_with_physical;
use astroburst_lib::core::metadata::filter_wavelengths::{
    FilterCandidate, FilterRegistry, FilterSelection, FilterSource,
};
use astroburst_lib::core::metadata::units::{
    apply_area_correction, apply_conversion, area_correction, classify_unit, unit_conversion,
    AreaRule, PixelGeometry, TargetUnit, UnitClass,
};
use astroburst_lib::infra::fits::reader::{
    extract_header_by_index, extract_image_mmap_by_index, extract_image_planes_by_index,
    extract_int_plane_by_index, parse_header_at,
};
use astroburst_lib::infra::fits::writer::write_fits_cube;
use astroburst_lib::infra::progress::ProgressHandle;
use astroburst_lib::infra::wcs_source::header_wcs;
use astroburst_lib::math::window::TaperWindow;
use astroburst_lib::service::cubeprep_recipe::{run_recipe, Recipe, RecipeReport};
use astroburst_lib::types::header::HduHeader;
use ndarray::Array2;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MANIFEST: &str = "manifest.json";
const RECIPE: &str = "recipe_d1.json";
const STARS_RECIPE: &str = "recipe_d1_stars.json";
const ORACLE_DIR_VAR: &str = "ASTROBURST_ORACLE_DIR";
const WRITE_FIXTURES_VAR: &str = "ASTROBURST_WRITE_LIGHTSTACK_FIXTURES";
const FIXTURE_BUDGET_BYTES: u64 = 3_100_000;
const LIGHTSTACK_VERSION: &str = "0.4.0";
const LIGHTSTACK_CODE_SHA256: &str =
    "f43772588e34d39a12ad38a1b6ab9be90681605605c71949e5cb3b42e7c43357";
const ORACLE_VERSIONS: [&str; 10] = [
    "python",
    "lightstack",
    "numpy",
    "scipy",
    "astropy",
    "reproject",
    "photutils",
    "regions",
    "matplotlib",
    "pandas",
];
const RECIPE_STEPS: [&str; 9] = [
    "units", "crop", "align", "cube", "edit", "psfs", "kernels", "match", "export",
];
const ORACLE_FUNCTION_PREFIXES: [&str; 2] = ["lightstack.", "photutils."];
const SCI: usize = 1;
const ERR: usize = 2;
const SKY_MJY_SR: f64 = 0.25;
const SKY_RADIUS_ARCSEC: f64 = 1.25;
const DQ: usize = 3;
const DO_NOT_USE: u32 = 1;
const STAR_APERTURE_ARCSEC: f64 = 0.25;
const CROP_TIE_PX: f64 = 1e-8;
const LIGHTSTACK_PRINTED_FACTOR: f64 = 5e-5;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("lightstack")
}

fn d1_dir() -> PathBuf {
    fixtures_dir().join("d1")
}

fn oracle_d1_dir() -> PathBuf {
    fixtures_dir().join("oracle_d1")
}

fn d1_stars_dir() -> PathBuf {
    fixtures_dir().join("d1_stars")
}

fn oracle_d1_stars_dir() -> PathBuf {
    fixtures_dir().join("oracle_d1_stars")
}

fn read_json(path: &Path) -> Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn files_under(root: &Path) -> BTreeMap<String, PathBuf> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in
            std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>();
                found.insert(relative.join("/"), path);
            }
        }
    }
    found
}

fn manifests_under(root: &Path) -> Vec<PathBuf> {
    files_under(root)
        .into_values()
        .filter(|path| path.file_name().is_some_and(|name| name == MANIFEST))
        .collect()
}

fn manifest_errors(manifest_path: &Path) -> Vec<String> {
    let dir = manifest_path.parent().unwrap();
    let manifest = read_json(manifest_path);
    let mut errors = Vec::new();
    let mut listed = BTreeSet::new();
    for entry in manifest["files"]
        .as_array()
        .unwrap_or_else(|| panic!("{}: no files list", manifest_path.display()))
    {
        let name = entry["path"].as_str().unwrap().to_owned();
        let path = dir.join(&name);
        match std::fs::read(&path) {
            Ok(bytes) => {
                if Some(bytes.len() as u64) != entry["bytes"].as_u64() {
                    errors.push(format!(
                        "{}: {} bytes, manifest says {}",
                        path.display(),
                        bytes.len(),
                        entry["bytes"]
                    ));
                }
                if Some(sha256_hex(&bytes).as_str()) != entry["sha256"].as_str() {
                    errors.push(format!(
                        "{}: sha256 {} differs from the manifest",
                        path.display(),
                        sha256_hex(&bytes)
                    ));
                }
            }
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
        if !listed.insert(name.clone()) {
            errors.push(format!(
                "{}: {name} is listed twice",
                manifest_path.display()
            ));
        }
    }
    for name in files_under(dir).into_keys().filter(|name| name != MANIFEST) {
        if !listed.contains(&name) {
            errors.push(format!(
                "{}: {name} is not listed in the manifest",
                dir.display()
            ));
        }
    }
    if manifest["kind"] == "scipy_reference" {
        let fixtures = dir.parent().unwrap();
        let created_by = manifest["created_by"].as_str().unwrap_or("its generator");
        for input in manifest["inputs"].as_array().into_iter().flatten() {
            let name = input["path"].as_str().unwrap();
            let unchanged = std::fs::read(fixtures.join(name)).is_ok_and(|bytes| {
                Some(bytes.len() as u64) == input["bytes"].as_u64()
                    && Some(sha256_hex(&bytes).as_str()) == input["sha256"].as_str()
            });
            if !unchanged {
                errors.push(format!(
                    "{}: input {name} differs from the manifest; re-run {created_by}",
                    manifest_path.display()
                ));
            }
        }
    }
    errors
}

fn oracle_manifest_errors(manifest: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if manifest["kind"] != "lightstack_oracle" {
        errors.push(format!(
            "kind is {}, not lightstack_oracle",
            manifest["kind"]
        ));
    }
    for package in ORACLE_VERSIONS {
        if !manifest["versions"][package]
            .as_str()
            .is_some_and(|version| !version.trim().is_empty())
        {
            errors.push(format!("versions.{package} is missing"));
        }
    }
    if manifest["versions"]["lightstack"] != LIGHTSTACK_VERSION {
        errors.push(format!(
            "versions.lightstack is {}, not {LIGHTSTACK_VERSION}",
            manifest["versions"]["lightstack"]
        ));
    }
    if manifest["manifest_version"]
        .as_u64()
        .is_some_and(|version| version >= 2)
    {
        let source = &manifest["lightstack_source"];
        if source["code_sha256"] != LIGHTSTACK_CODE_SHA256 {
            errors.push(format!(
                "lightstack_source.code_sha256 is {}, not {LIGHTSTACK_CODE_SHA256}",
                source["code_sha256"]
            ));
        }
        if source["dirty"] == true {
            errors.push("lightstack_source.dirty is true".to_owned());
        }
        if !source["modified_since_install"]
            .as_array()
            .is_some_and(|files| files.is_empty())
        {
            errors.push(format!(
                "lightstack_source.modified_since_install is {}, not an empty list",
                source["modified_since_install"]
            ));
        }
    }
    let steps = manifest["steps"].as_array().cloned().unwrap_or_default();
    if steps.is_empty() {
        errors.push("the manifest records no step".to_owned());
    }
    for step in &steps {
        for call in step["calls"].as_array().cloned().unwrap_or_default() {
            let function = call["function"].as_str().unwrap_or_default();
            if !ORACLE_FUNCTION_PREFIXES
                .iter()
                .any(|prefix| function.starts_with(prefix))
            {
                errors.push(format!(
                    "step {}: call {function:?} is neither LIGHTSTACK nor photutils",
                    step["step"]
                ));
            }
            if !call["wall_time_s"]
                .as_f64()
                .is_some_and(|seconds| seconds >= 0.0)
            {
                errors.push(format!(
                    "step {}: {function} has no wall time",
                    step["step"]
                ));
            }
            for file in call["inputs"]
                .as_array()
                .into_iter()
                .chain(call["outputs"].as_array())
                .flatten()
            {
                if !file["sha256"].as_str().is_some_and(|hex| hex.len() == 64) {
                    errors.push(format!(
                        "step {}: {function} records {} without a sha256",
                        step["step"], file["path"]
                    ));
                }
            }
        }
    }
    errors
}

fn open(path: &Path) -> File {
    File::open(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn plane_data(path: &Path, hdu: usize) -> (HduHeader, Vec<f32>) {
    let read = extract_image_mmap_by_index(&open(path), hdu)
        .unwrap_or_else(|error| panic!("{}[{hdu}]: {error:#}", path.display()));
    let data = read.image.as_standard_layout().iter().copied().collect();
    (read.header, data)
}

fn f64_card(header: &HduHeader, key: &str) -> f64 {
    header
        .get_f64(key)
        .unwrap_or_else(|| panic!("card {key} is missing"))
}

fn relative_difference(got: f64, expected: f64) -> f64 {
    (got - expected).abs() / expected.abs()
}

fn star_positions(truth: &Value) -> BTreeMap<String, [f64; 2]> {
    truth["stars"]
        .as_array()
        .unwrap()
        .iter()
        .map(|star| {
            let position = &star["position_deg"];
            (
                star["id"].as_str().unwrap().to_owned(),
                [position[0].as_f64().unwrap(), position[1].as_f64().unwrap()],
            )
        })
        .collect()
}

fn sigma_at(plane: &Value, x: usize) -> f64 {
    let size = plane["shape"][1].as_u64().unwrap() as f64;
    plane["noise"]["sigma0_jy"].as_f64().unwrap() * (0.85 + 0.30 * x as f64 / (size - 1.0))
}

fn in_rect(rect: &Value, x: usize, y: usize) -> bool {
    let bound = |axis: &str, end: usize| rect[axis][end].as_u64().unwrap() as usize;
    (bound("x", 0)..bound("x", 1)).contains(&x) && (bound("y", 0)..bound("y", 1)).contains(&y)
}

fn oracle_calls<'a>(manifest: &'a Value, step: &str) -> Vec<&'a Value> {
    manifest["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|record| record["step"] == step)
        .flat_map(|record| record["calls"].as_array().unwrap())
        .collect()
}

#[test]
fn scipy_reference_manifests_detect_a_changed_input() {
    let scratch = tempfile::tempdir().unwrap();
    let kernels = scratch.path().join("kernels");
    std::fs::create_dir_all(&kernels).unwrap();
    for name in [MANIFEST, "kernel_goldens.json"] {
        std::fs::copy(fixtures_dir().join("kernels").join(name), kernels.join(name)).unwrap();
    }
    let manifest = read_json(&kernels.join(MANIFEST));
    assert_eq!(manifest["kind"], "scipy_reference");
    let inputs: Vec<String> = manifest["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| input["path"].as_str().unwrap().to_owned())
        .collect();
    for input in &inputs {
        let copy = scratch.path().join(input);
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::copy(fixtures_dir().join(input), &copy).unwrap();
    }
    assert_eq!(
        manifest_errors(&kernels.join(MANIFEST)),
        Vec::<String>::new(),
        "an unchanged copy of the kernels goldens and their D1 PSF inputs"
    );
    let flipped = scratch.path().join(&inputs[0]);
    let mut bytes = std::fs::read(&flipped).unwrap();
    bytes[2880 + 8] ^= 0x01;
    std::fs::write(&flipped, bytes).unwrap();
    let errors = manifest_errors(&kernels.join(MANIFEST));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains(&inputs[0]) && errors[0].contains("scripts/oracle/kernel_goldens.py"),
        "the error names the stale input and the script that regenerates the goldens: {}",
        errors[0]
    );
}

#[test]
fn oracle_manifest_version_two_requires_the_lightstack_code_fingerprint() {
    let mut manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    assert_eq!(
        oracle_manifest_errors(&manifest),
        Vec::<String>::new(),
        "the committed version 1 manifest"
    );
    manifest["manifest_version"] = Value::from(2);
    let errors = oracle_manifest_errors(&manifest);
    assert!(
        errors.iter().any(|error| error.contains("code_sha256"))
            && errors.iter().any(|error| error.contains("modified_since_install")),
        "version 2 without the fingerprint fields: {errors:?}"
    );
    manifest["lightstack_source"] = serde_json::json!({
        "kind": "index",
        "code_sha256": LIGHTSTACK_CODE_SHA256,
        "dirty": false,
        "modified_since_install": [],
    });
    assert_eq!(oracle_manifest_errors(&manifest), Vec::<String>::new());
    manifest["lightstack_source"]["dirty"] = Value::from(true);
    manifest["lightstack_source"]["code_sha256"] = Value::from("0".repeat(64));
    manifest["lightstack_source"]["modified_since_install"] = serde_json::json!(["lightstack/psf.py"]);
    let errors = oracle_manifest_errors(&manifest);
    assert_eq!(errors.len(), 3, "{errors:?}");
}

#[test]
fn fixtures_match_manifest_sha256() {
    let manifests = manifests_under(&fixtures_dir());
    assert!(
        manifests.contains(&d1_dir().join(MANIFEST)),
        "no D1 manifest among {manifests:?}"
    );
    let errors: Vec<String> = manifests
        .iter()
        .flat_map(|manifest| manifest_errors(manifest))
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn lightstack_fixtures_stay_under_budget() {
    let total: u64 = files_under(&fixtures_dir())
        .values()
        .map(|path| path.metadata().unwrap().len())
        .sum();
    let headroom = FIXTURE_BUDGET_BYTES as i64 - total as i64;
    eprintln!(
        "tests/fixtures/lightstack holds {total} bytes of the {FIXTURE_BUDGET_BYTES} budget, headroom {headroom}"
    );
    assert!(
        total < FIXTURE_BUDGET_BYTES,
        "tests/fixtures/lightstack holds {total} bytes, the budget is {FIXTURE_BUDGET_BYTES} (headroom {headroom}); raise FIXTURE_BUDGET_BYTES on purpose or drop a golden"
    );
}

fn listed_files(manifest: &Value) -> Vec<(String, String)> {
    manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["path"].as_str().unwrap().to_owned(),
                entry["sha256"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn d1_generator_reproduces_the_committed_fixtures() {
    let scratch = tempfile::tempdir().unwrap();
    let produced = d1::generate(scratch.path());
    let committed = read_json(&d1_dir().join(MANIFEST));
    assert_eq!(
        listed_files(&produced),
        listed_files(&committed),
        "the generator no longer writes the committed D1 bytes"
    );
    assert!(
        produced == committed,
        "the generator truth differs from the committed D1 manifest"
    );
}

fn fixture_writes_allowed(target: &Path) -> bool {
    let allowed = std::env::var_os(WRITE_FIXTURES_VAR).is_some_and(|value| value == "1");
    if !allowed {
        eprintln!(
            "{} left unchanged: set {WRITE_FIXTURES_VAR}=1 to rewrite it",
            target.display()
        );
    }
    allowed
}

#[test]
#[ignore = "writes tests/fixtures/lightstack/d1 with ASTROBURST_WRITE_LIGHTSTACK_FIXTURES=1; run it only to change D1 on purpose"]
fn regenerate_d1_fixtures() {
    if fixture_writes_allowed(&d1_dir()) {
        d1::generate(&d1_dir());
    }
}

#[test]
fn d1_star_field_generator_reproduces_the_committed_fixture() {
    let scratch = tempfile::tempdir().unwrap();
    let produced = d1::generate_star_field(scratch.path());
    let committed = read_json(&d1_stars_dir().join(MANIFEST));
    assert_eq!(
        listed_files(&produced),
        listed_files(&committed),
        "the generator no longer writes the committed D1 star field bytes"
    );
    assert!(
        produced == committed,
        "the generator truth differs from the committed D1 star field manifest"
    );
}

#[test]
#[ignore = "writes tests/fixtures/lightstack/d1_stars with ASTROBURST_WRITE_LIGHTSTACK_FIXTURES=1; run it only to change the star field on purpose"]
fn regenerate_d1_star_field_fixture() {
    if fixture_writes_allowed(&d1_stars_dir()) {
        d1::generate_star_field(&d1_stars_dir());
    }
}

#[test]
fn committed_regenerate_lines_set_the_write_opt_in() {
    for dir in [d1_dir(), d1_stars_dir()] {
        let manifest = read_json(&dir.join(MANIFEST));
        let line = manifest["regenerate"].as_str().unwrap_or_default();
        assert!(
            line.starts_with(&format!("{WRITE_FIXTURES_VAR}=1 ")),
            "{}: the regenerate line {line:?} does not set {WRITE_FIXTURES_VAR}=1",
            dir.display()
        );
    }
}

#[test]
fn d1_star_positions_and_aperture_fluxes_match_the_truth() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let positions = star_positions(&truth);
    let mut checked = 0;
    for plane in truth["planes"].as_array().unwrap() {
        let path = d1_dir().join(plane["file"].as_str().unwrap());
        let (header, sci) = plane_data(&path, SCI);
        assert_eq!(
            header.get("EXTNAME").map(str::trim),
            Some("SCI"),
            "{}",
            path.display()
        );
        let wcs =
            header_wcs(&header).unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
        let size = plane["shape"][1].as_u64().unwrap() as usize;
        let jy_per_unit = plane["unit"]["jy_per_unit"].as_f64().unwrap();
        for star in plane["stars"].as_array().unwrap() {
            let id = star["id"].as_str().unwrap();
            let [ra, dec] = positions[id];
            let (x, y) = (star["x"].as_f64().unwrap(), star["y"].as_f64().unwrap());
            let (wx, wy) = wcs.world_to_pixel(ra, dec);
            assert!(
                (wx - x).abs() < 1e-6 && (wy - y).abs() < 1e-6,
                "{} {id}: AstroBurst WCS puts the star at ({wx}, {wy}), the truth at ({x}, {y})",
                plane["filter"]
            );
            let radius = star["aperture_radius_px"].as_f64().unwrap();
            let mut measured = 0.0;
            for (index, value) in sci.iter().enumerate() {
                let (px, py) = ((index % size) as f64, (index / size) as f64);
                if (px - x).powi(2) + (py - y).powi(2) <= radius * radius {
                    assert!(
                        value.is_finite(),
                        "{} {id}: NaN inside the aperture",
                        plane["filter"]
                    );
                    measured += *value as f64 * jy_per_unit;
                }
            }
            let expected = star["aperture_model_jy"].as_f64().unwrap();
            let snr = expected / star["aperture_sigma_jy"].as_f64().unwrap();
            assert!(
                snr > 100.0,
                "{} {id}: S/N {snr} is below the S-class floor",
                plane["filter"]
            );
            assert!(
                relative_difference(measured, expected) < 0.01,
                "{} {id}: aperture flux {measured} Jy, truth {expected} Jy (S/N {snr})",
                plane["filter"]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 30);
}

#[test]
fn d1_err_plane_matches_the_noise_and_dq_marks_every_nan() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    for (index, plane) in truth["planes"].as_array().unwrap().iter().enumerate() {
        let filter = plane["filter"].as_str().unwrap();
        let path = d1_dir().join(plane["file"].as_str().unwrap());
        let (_, sci) = plane_data(&path, SCI);
        let (_, err) = plane_data(&path, ERR);
        let dq = extract_int_plane_by_index(&open(&path), DQ)
            .unwrap_or_else(|error| panic!("{filter}: {error:#}"));
        let dq: Vec<u32> = dq.bits.as_standard_layout().iter().copied().collect();
        let size = plane["shape"][1].as_u64().unwrap() as usize;
        let border = plane["nan_border_px"].as_u64().unwrap() as usize;
        let jy_per_unit = plane["unit"]["jy_per_unit"].as_f64().unwrap();
        let model = d1::render(index).model_jy;
        let holes = plane["holes"].as_array().unwrap();
        let flagged: BTreeMap<(usize, usize), u32> = plane["flagged"]
            .as_array()
            .unwrap()
            .iter()
            .map(|flag| {
                (
                    (
                        flag["x"].as_u64().unwrap() as usize,
                        flag["y"].as_u64().unwrap() as usize,
                    ),
                    flag["dq"].as_u64().unwrap() as u32,
                )
            })
            .collect();
        let (mut count, mut sum, mut sum_sq, mut nan) = (0usize, 0.0f64, 0.0f64, 0usize);
        for (pixel, value) in sci.iter().enumerate() {
            let (x, y) = (pixel % size, pixel / size);
            let on_border = x < border || y < border || x >= size - border || y >= size - border;
            let in_hole = holes.iter().any(|hole| in_rect(hole, x, y));
            let expected_dq = if on_border {
                plane["border_dq"].as_u64().unwrap() as u32
            } else if in_hole {
                holes.iter().find(|hole| in_rect(hole, x, y)).unwrap()["dq"]
                    .as_u64()
                    .unwrap() as u32
            } else {
                flagged.get(&(x, y)).copied().unwrap_or(0)
            };
            assert_eq!(dq[pixel], expected_dq, "{filter}: DQ at ({x}, {y})");
            assert_eq!(
                value.is_nan(),
                dq[pixel] & DO_NOT_USE != 0,
                "{filter}: SCI NaN and DQ DO_NOT_USE disagree at ({x}, {y})"
            );
            assert_eq!(
                value.is_nan(),
                err[pixel].is_nan(),
                "{filter}: SCI and ERR NaN disagree at ({x}, {y})"
            );
            if value.is_nan() {
                nan += 1;
                continue;
            }
            let sigma_unit = sigma_at(plane, x) / jy_per_unit;
            assert!(
                relative_difference(err[pixel] as f64, sigma_unit) < 1e-6,
                "{filter}: ERR at ({x}, {y})"
            );
            let pull = (*value as f64 - model[pixel] / jy_per_unit) / err[pixel] as f64;
            count += 1;
            sum += pull;
            sum_sq += pull * pull;
        }
        let interior = (size - 2 * border).pow(2);
        let hole_pixels: usize = holes
            .iter()
            .map(|hole| {
                let span = |axis: &str| {
                    (hole[axis][1].as_u64().unwrap() - hole[axis][0].as_u64().unwrap()) as usize
                };
                span("x") * span("y")
            })
            .sum();
        assert_eq!(
            nan,
            size * size - interior + hole_pixels,
            "{filter}: NaN pixel count"
        );
        let mean = sum / count as f64;
        let std = (sum_sq / count as f64 - mean * mean).sqrt();
        assert!(
            mean.abs() < 0.1 && (std - 1.0).abs() < 0.05,
            "{filter}: noise pulls have mean {mean} and std {std}"
        );
    }
}

#[test]
fn d1_headers_psfs_and_recipe_agree_with_the_truth() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    for plane in truth["planes"].as_array().unwrap() {
        let filter = plane["filter"].as_str().unwrap();
        let path = d1_dir().join(plane["file"].as_str().unwrap());
        let (header, _) = plane_data(&path, SCI);
        assert_eq!(header.get("FILTER").map(str::trim), Some(filter));
        assert_eq!(
            header.get("BUNIT").map(str::trim),
            plane["unit"]["bunit"].as_str()
        );
        let jy_per_unit = plane["unit"]["jy_per_unit"].as_f64().unwrap();
        let scale = plane["pixel_scale_arcsec"].as_f64().unwrap();
        match plane["unit"]["bunit"].as_str().unwrap() {
            "MJy/sr" => {
                let pixar_sr = f64_card(&header, "PIXAR_SR");
                assert!(
                    relative_difference(
                        pixar_sr,
                        (scale * std::f64::consts::PI / 648_000.0).powi(2)
                    ) < 1e-12,
                    "{filter}: PIXAR_SR"
                );
                assert!(
                    relative_difference(jy_per_unit, 1e6 * pixar_sr) < 1e-12,
                    "{filter}: MJy/sr to Jy/pixel factor"
                );
            }
            "ELECTRONS/S" => {
                let photfnu = f64_card(&header, "PHOTFNU");
                let from_photflam = f64_card(&header, "PHOTFLAM")
                    * f64_card(&header, "PHOTPLAM").powi(2)
                    / 2.99792458e18
                    / 1e-23;
                assert!(
                    relative_difference(photfnu, from_photflam) < 1e-12,
                    "{filter}: PHOTFNU and PHOTFLAM disagree"
                );
                assert!(
                    relative_difference(jy_per_unit, photfnu) < 1e-12,
                    "{filter}: e-/s to Jy/pixel factor"
                );
            }
            "Jy" => {
                assert_eq!(jy_per_unit, 1.0);
                assert!(
                    header.get("PIXAR_SR").is_none() && header.get("PHOTFNU").is_none(),
                    "{filter}: Jy/pixel plane carries conversion cards"
                );
            }
            other => panic!("{filter}: unexpected BUNIT {other}"),
        }
        for psf in plane["psf"]["files"].as_array().unwrap() {
            let psf_path = d1_dir().join(psf["path"].as_str().unwrap());
            let bytes = std::fs::read(&psf_path).unwrap();
            let parsed = parse_header_at(&bytes, 0).unwrap();
            let side = psf["side"].as_u64().unwrap() as usize;
            assert_eq!(
                parsed.header.get_i64("BITPIX"),
                Some(-64),
                "{}",
                psf_path.display()
            );
            assert_eq!(
                (
                    parsed.header.get_i64("NAXIS1"),
                    parsed.header.get_i64("NAXIS2")
                ),
                (Some(side as i64), Some(side as i64))
            );
            assert_eq!(side % 2, 1, "{}: even side", psf_path.display());
            assert_eq!(
                parsed.header.get_f64("PIXSCALE"),
                psf["pixel_scale_arcsec"].as_f64()
            );
            let values: Vec<f64> = bytes[parsed.data_start..parsed.data_start + side * side * 8]
                .chunks_exact(8)
                .map(|chunk| f64::from_be_bytes(chunk.try_into().unwrap()))
                .collect();
            let total: f64 = values.iter().sum();
            assert!(
                (total - 1.0).abs() < 1e-12,
                "{}: PSF sums to {total}",
                psf_path.display()
            );
            let peak = values
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(
                peak,
                side * side / 2,
                "{}: the peak is not the central pixel",
                psf_path.display()
            );
        }
    }
    let recipe = read_json(&d1_dir().join(RECIPE));
    let inputs: Vec<(&str, &str, &str)> = recipe["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| {
            (
                input["path"].as_str().unwrap(),
                input["dataset"].as_str().unwrap(),
                input["filter"].as_str().unwrap(),
            )
        })
        .collect();
    let planes: Vec<(&str, &str, &str)> = truth["planes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plane| {
            (
                plane["file"].as_str().unwrap(),
                plane["dataset"].as_str().unwrap(),
                plane["filter"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(inputs, planes);
    for step in recipe["steps"].as_array().unwrap() {
        assert!(
            RECIPE_STEPS.contains(&step["step"].as_str().unwrap()),
            "unknown recipe step {}",
            step["step"]
        );
    }
    let psf_files: BTreeSet<&str> = recipe["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| step["step"] == "psfs")
        .flat_map(|step| step["files"].as_array().unwrap())
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    for path in &psf_files {
        assert!(
            d1_dir().join(path).is_file(),
            "the recipe names a missing PSF {path}"
        );
    }
    assert_eq!(psf_files.len(), planes.len(), "one PSF per plane");
}

fn reproject_onto(
    image: &Array2<f32>,
    header: &HduHeader,
    wcs: &WcsTransform,
    reference: &WcsTransform,
    size: usize,
) -> Array2<f32> {
    let ratio = reference.pixel_scale_arcsec() / wcs.pixel_scale_arcsec();
    let reduced =
        prefilter_coarse_target(image, header, ratio).and_then(|(reduced, reduced_header, _)| {
            header_wcs(&reduced_header)
                .ok()
                .map(|reduced_wcs| (reduced, reduced_wcs))
        });
    match reduced {
        Some((reduced, reduced_wcs)) => {
            reproject_to_grid(&reduced, &reduced_wcs, reference, size, size).0
        }
        None => reproject_to_grid(image, wcs, reference, size, size).0,
    }
}

fn finite_sum(image: &Array2<f32>) -> f64 {
    image
        .iter()
        .filter(|value| value.is_finite())
        .map(|value| f64::from(*value))
        .sum()
}

fn median_beyond(
    image: &Array2<f32>,
    wcs: &WcsTransform,
    centre: [f64; 2],
    radius_arcsec: f64,
) -> f64 {
    let (cx, cy) = wcs.world_to_pixel(centre[0], centre[1]);
    let radius_px = radius_arcsec / wcs.pixel_scale_arcsec();
    let mut values: Vec<f64> = image
        .indexed_iter()
        .filter(|((y, x), value)| {
            value.is_finite() && (*x as f64 - cx).hypot(*y as f64 - cy) > radius_px
        })
        .map(|(_, value)| f64::from(*value))
        .collect();
    assert!(
        values.len() > 20,
        "only {} pixels in the region",
        values.len()
    );
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn percentile_range(image: &Array2<f32>) -> f64 {
    let mut values: Vec<f64> = image
        .iter()
        .filter(|value| value.is_finite())
        .map(|value| f64::from(*value))
        .collect();
    values.sort_by(f64::total_cmp);
    let at = |fraction: f64| values[((values.len() - 1) as f64 * fraction).round() as usize];
    at(0.999) - at(0.001)
}

#[test]
fn d1_units_convert_to_jy_per_pixel_with_the_truth_factor() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let mut checked = 0;
    for plane in truth["planes"].as_array().unwrap() {
        let filter = plane["filter"].as_str().unwrap();
        let path = d1_dir().join(plane["file"].as_str().unwrap());
        let sci = extract_image_mmap_by_index(&open(&path), SCI).unwrap();
        let err = extract_image_mmap_by_index(&open(&path), ERR).unwrap();
        let wcs = header_wcs(&sci.header).unwrap();
        let conversion = unit_conversion(&sci.header, Some(&wcs), TargetUnit::JyPerPixel)
            .unwrap_or_else(|error| panic!("{filter}: {error}"));
        let jy_per_unit = plane["unit"]["jy_per_unit"].as_f64().unwrap();
        assert!(
            relative_difference(conversion.factor, jy_per_unit) < 1e-12,
            "{filter}: factor {} against the truth {jy_per_unit}",
            conversion.factor
        );
        assert!(
            conversion.warnings.is_empty(),
            "{filter}: {:?}",
            conversion.warnings
        );
        let (mut image, mut sigma) = (sci.image.clone(), err.image.clone());
        let (mut header, mut err_header) = (sci.header.clone(), err.header.clone());
        apply_conversion(
            &conversion,
            &mut image,
            Some(&mut sigma),
            &mut header,
            Some(&mut err_header),
        );
        for written in [&header, &err_header] {
            assert_eq!(
                classify_unit(written).class,
                UnitClass::PerPixelPhysical { jy_per_unit: 1.0 },
                "{filter}"
            );
        }
        for ((_, x), value) in sigma.indexed_iter() {
            if value.is_finite() {
                assert!(
                    relative_difference(f64::from(*value), sigma_at(plane, x)) < 2e-6,
                    "{filter}: ERR at column {x} is {value} Jy"
                );
            }
        }
        for star in plane["stars"].as_array().unwrap() {
            let (x, y) = (star["x"].as_f64().unwrap(), star["y"].as_f64().unwrap());
            let radius = star["aperture_radius_px"].as_f64().unwrap();
            let measured: f64 = image
                .indexed_iter()
                .filter(|((py, px), _)| (*px as f64 - x).hypot(*py as f64 - y) <= radius)
                .map(|(_, value)| f64::from(*value))
                .sum();
            let expected = star["aperture_model_jy"].as_f64().unwrap();
            assert!(
                relative_difference(measured, expected) < 0.01,
                "{filter} {}: {measured} Jy against the truth {expected} Jy",
                star["id"]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 30);
}

#[test]
fn d1_area_rule_conserves_flux_per_pixel_and_keeps_surface_brightness() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let planes = truth["planes"].as_array().unwrap();
    let reference = planes
        .iter()
        .find(|plane| plane["filter"] == truth["reference_filter"])
        .unwrap();
    let (reference_header, _) =
        plane_data(&d1_dir().join(reference["file"].as_str().unwrap()), SCI);
    let reference_wcs = header_wcs(&reference_header).unwrap();
    let reference_size = reference["shape"][1].as_u64().unwrap() as usize;
    let centre = &truth["field_centre_deg"];
    let centre = [centre[0].as_f64().unwrap(), centre[1].as_f64().unwrap()];
    let mut checked = 0;
    for (index, plane) in planes.iter().enumerate() {
        if plane["pixel_scale_arcsec"].as_f64() != Some(0.02) {
            continue;
        }
        let filter = plane["filter"].as_str().unwrap();
        let (header, _) = plane_data(&d1_dir().join(plane["file"].as_str().unwrap()), SCI);
        let wcs = header_wcs(&header).unwrap();
        let size = plane["shape"][1].as_u64().unwrap() as usize;
        let jy_per_unit = plane["unit"]["jy_per_unit"].as_f64().unwrap();
        let model = d1::render(index).model_jy;
        let input_jy: f64 = model.iter().sum();
        let native = Array2::from_shape_fn((size, size), |(y, x)| {
            (model[y * size + x] / jy_per_unit) as f32
        });
        let geometry = PixelGeometry::between(&wcs, &reference_wcs);
        assert!(
            relative_difference(geometry.area_ratio, 4.0) < 1e-9,
            "{filter}: {geometry:?}"
        );

        let mut lightstack_header = header.clone();
        let to_jy =
            unit_conversion(&lightstack_header, Some(&wcs), TargetUnit::JyPerPixel).unwrap();
        let mut per_pixel = native.clone();
        apply_conversion(&to_jy, &mut per_pixel, None, &mut lightstack_header, None);
        let mut lightstack = reproject_onto(
            &per_pixel,
            &lightstack_header,
            &wcs,
            &reference_wcs,
            reference_size,
        );
        let scale = area_correction(&lightstack_header, None, &geometry).unwrap();
        assert_eq!(scale.rule, AreaRule::Scale, "{filter}");
        apply_area_correction(&scale, &mut lightstack, None, &mut lightstack_header);
        let lightstack_jy = finite_sum(&lightstack);
        assert!(
            relative_difference(lightstack_jy, input_jy) < 1e-3,
            "{filter}: convert then align gives {lightstack_jy} Jy, the truth {input_jy} Jy"
        );

        let sky = native.mapv(|value| value + SKY_MJY_SR as f32);
        let mut sky_header = header.clone();
        let mut aligned_sky =
            reproject_onto(&sky, &sky_header, &wcs, &reference_wcs, reference_size);
        let keep = area_correction(&sky_header, None, &geometry).unwrap();
        assert_eq!(
            (keep.rule, keep.data_factor),
            (AreaRule::Keep, 1.0),
            "{filter}"
        );
        apply_area_correction(&keep, &mut aligned_sky, None, &mut sky_header);
        let sky_in = median_beyond(&sky, &wcs, centre, SKY_RADIUS_ARCSEC);
        let sky_out = median_beyond(&aligned_sky, &reference_wcs, centre, SKY_RADIUS_ARCSEC);
        assert!(
            relative_difference(sky_in, SKY_MJY_SR) < 1e-6,
            "{filter}: the outskirts hold source light ({sky_in} MJy/sr)"
        );
        assert!(
            relative_difference(sky_out, sky_in) < 1e-3,
            "{filter}: median sky {sky_out} MJy/sr after alignment, {sky_in} MJy/sr before"
        );
        assert!(
            relative_difference(
                f64_card(&sky_header, "PIXAR_SR"),
                4.0 * f64_card(&header, "PIXAR_SR")
            ) < 1e-9,
            "{filter}: PIXAR_SR follows the new pixel"
        );

        let mut astroburst_header = header.clone();
        let mut astroburst = reproject_onto(
            &native,
            &astroburst_header,
            &wcs,
            &reference_wcs,
            reference_size,
        );
        apply_area_correction(&keep, &mut astroburst, None, &mut astroburst_header);
        let to_jy_after = unit_conversion(
            &astroburst_header,
            Some(&reference_wcs),
            TargetUnit::JyPerPixel,
        )
        .unwrap();
        assert!(
            to_jy_after.warnings.is_empty(),
            "{filter}: {:?}",
            to_jy_after.warnings
        );
        apply_conversion(
            &to_jy_after,
            &mut astroburst,
            None,
            &mut astroburst_header,
            None,
        );
        let astroburst_jy = finite_sum(&astroburst);
        assert!(
            relative_difference(astroburst_jy, input_jy) < 1e-3,
            "{filter}: align then convert gives {astroburst_jy} Jy, the truth {input_jy} Jy"
        );
        let worst = lightstack
            .iter()
            .zip(astroburst.iter())
            .filter(|(a, b)| a.is_finite() && b.is_finite())
            .map(|(a, b)| f64::from((a - b).abs()))
            .fold(0.0, f64::max);
        assert!(
            worst <= 1e-3 * percentile_range(&lightstack),
            "{filter}: the two paths differ by {worst} Jy"
        );
        checked += 1;
    }
    assert_eq!(checked, 2);
}

#[test]
fn d1_filters_are_identified_from_their_headers_and_ordered_by_pivot() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let registry = FilterRegistry::builtin();
    let mut headers = Vec::new();
    for plane in truth["planes"].as_array().unwrap() {
        let path = d1_dir().join(plane["file"].as_str().unwrap());
        let (header, _) = plane_data(&path, SCI);
        let path = path.to_str().unwrap().to_owned();
        let found = registry.identify(Some(&header), &path, None).unwrap();
        let dataset = plane["dataset"].as_str().unwrap();
        assert_eq!(found.entry.dataset, dataset);
        assert_eq!(found.entry.filter, plane["filter"].as_str().unwrap());
        assert!(found
            .entry
            .instrument
            .eq_ignore_ascii_case(plane["instrument"].as_str().unwrap()));
        assert_eq!(
            found.source,
            FilterSource::Header {
                keyword: "FILTER".into()
            }
        );
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        assert_eq!(
            registry
                .identify_file_name(&path, dataset)
                .unwrap()
                .entry
                .filter,
            found.entry.filter
        );
        headers.push((path, header));
    }
    let candidates: Vec<FilterCandidate> = headers
        .iter()
        .rev()
        .map(|(path, header)| FilterCandidate {
            path,
            header: Some(header),
        })
        .collect();
    let selection = ["JWST", "HST"].map(|dataset| FilterSelection {
        dataset: dataset.into(),
        ..FilterSelection::default()
    });
    let order: Vec<String> = registry
        .select(&candidates, &selection)
        .unwrap()
        .into_iter()
        .map(|chosen| chosen.identification.entry.filter)
        .collect();
    assert_eq!(
        order,
        ["F814W", "F150W", "F200W", "F277W", "F356W", "F444W"]
    );
}

struct D1Plane {
    filter: String,
    header: HduHeader,
    wcs: WcsTransform,
    side: usize,
}

fn d1_planes() -> Vec<D1Plane> {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    truth["planes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plane| {
            let (header, _) = plane_data(&d1_dir().join(plane["file"].as_str().unwrap()), SCI);
            let wcs = header_wcs(&header).unwrap();
            D1Plane {
                filter: plane["filter"].as_str().unwrap().to_owned(),
                header,
                wcs,
                side: plane["shape"][1].as_u64().unwrap() as usize,
            }
        })
        .collect()
}

fn recipe_crop() -> (SkyPoint, f64) {
    let recipe = read_json(&d1_dir().join(RECIPE));
    let crop = recipe["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["step"] == "crop")
        .unwrap()
        .clone();
    (
        SkyPoint {
            ra: crop["ra"].as_f64().unwrap(),
            dec: crop["dec"].as_f64().unwrap(),
        },
        crop["size_arcsec"].as_f64().unwrap(),
    )
}

struct CropEdge<'a> {
    name: &'a str,
    position: f64,
    ours: i64,
    theirs: i64,
    upper: bool,
}

fn lightstack_edge(position: f64, upper: bool, side: usize) -> i64 {
    let rounded = if upper {
        position.ceil()
    } else {
        position.floor()
    };
    (rounded as i64).clamp(0, side as i64)
}

fn astroburst_crop_centre(wcs: &WcsTransform, center: SkyPoint) -> (f64, f64) {
    wcs.world_to_pixel_batch(&[(center.ra, center.dec)])[0]
}

fn check_crop_edge(filter: &str, edge: &CropEdge, side: usize, other: &str) -> Option<String> {
    let CropEdge {
        name,
        position,
        ours,
        theirs,
        upper,
    } = *edge;
    let rounding = if upper { "ceil" } else { "floor" };
    assert_eq!(
        ours,
        lightstack_edge(position, upper, side),
        "{filter} {name}: AstroBurst's edge is not the {rounding} of its own continuous position {position} px"
    );
    let nearest = position.round();
    if (position - nearest).abs() > CROP_TIE_PX {
        assert_eq!(
            ours, theirs,
            "{filter} {name}: AstroBurst's continuous edge {position} px is not within {CROP_TIE_PX} px of an integer, so its rounding is unambiguous and {other} must agree"
        );
        return None;
    }
    let tie = nearest as i64;
    let either = if upper {
        [tie, tie + 1]
    } else {
        [tie - 1, tie]
    }
    .map(|bound| bound.clamp(0, side as i64));
    assert!(
        either.contains(&theirs),
        "{filter} {name}: a tie: AstroBurst's continuous edge {position} px lies within {CROP_TIE_PX} px of {tie}, so the {rounding} of {other} may be either of {either:?}; it gave {theirs}, AstroBurst {ours}"
    );
    Some(format!(
        "{filter} {name} at {position:?} px: AstroBurst {ours}, {other} {theirs}"
    ))
}

#[test]
fn d1_lightstack_crop_bounds_follow_crop_from_radec() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let (center, size) = recipe_crop();
    let mut ties = Vec::new();
    for (index, (plane, data)) in truth["planes"]
        .as_array()
        .unwrap()
        .iter()
        .zip(d1_planes())
        .enumerate()
    {
        let wcs = lightstack_wcs(&data.header).unwrap();
        let got = lightstack_square_rect(&wcs, (data.side, data.side), center, size).unwrap();
        let (x, y) = astroburst_crop_centre(&wcs, center);
        let half = size / wcs.pixel_scale_arcsec() / 2.0;
        let [tx, ty] = d1::world_to_pixel(index, [center.ra, center.dec]);
        let truth_half = size / plane["pixel_scale_arcsec"].as_f64().unwrap() / 2.0;
        let gap = (x - tx).abs().max((y - ty).abs());
        eprintln!(
            "{}: AstroBurst's crop centre is {gap:.2e} px from the generator's",
            data.filter
        );
        assert!(
            gap < 1e-6,
            "{}: AstroBurst puts the crop centre at ({x}, {y}), the generator at ({tx}, {ty})",
            data.filter
        );
        let formula = |position: f64, upper: bool| lightstack_edge(position, upper, data.side);
        let edges = [
            CropEdge {
                name: "x_min",
                position: x - half,
                ours: got.x0,
                theirs: formula(tx - truth_half, false),
                upper: false,
            },
            CropEdge {
                name: "x_max",
                position: x + half,
                ours: got.x0 + got.width as i64,
                theirs: formula(tx + truth_half, true),
                upper: true,
            },
            CropEdge {
                name: "y_min",
                position: y - half,
                ours: got.y0,
                theirs: formula(ty - truth_half, false),
                upper: false,
            },
            CropEdge {
                name: "y_max",
                position: y + half,
                ours: got.y0 + got.height as i64,
                theirs: formula(ty + truth_half, true),
                upper: true,
            },
        ];
        ties.extend(edges.iter().filter_map(|edge| {
            check_crop_edge(
                &data.filter,
                edge,
                data.side,
                "the LIGHTSTACK formula on the generator's position",
            )
        }));
    }
    eprintln!("crop edges judged as ties: {ties:#?}");
}

fn f277w_crop_matches_crop_from_radec(
    name: &str,
    edit: impl FnOnce(&mut HduHeader),
    astropy_scale: f64,
    [x_min, x_max, y_min, y_max]: [i64; 4],
) -> Vec<String> {
    let (center, size) = recipe_crop();
    let f277w = d1_planes()
        .into_iter()
        .find(|plane| plane.filter == "F277W")
        .unwrap();
    let mut header = f277w.header;
    edit(&mut header);
    let wcs = lightstack_wcs(&header).unwrap();
    let rect = lightstack_square_rect(&wcs, (f277w.side, f277w.side), center, size).unwrap();
    let (x, y) = astroburst_crop_centre(&wcs, center);
    let scale = wcs.pixel_scale_arcsec();
    let half = size / scale / 2.0;
    let edges = [
        CropEdge {
            name: "x_min",
            position: x - half,
            ours: rect.x0,
            theirs: x_min,
            upper: false,
        },
        CropEdge {
            name: "x_max",
            position: x + half,
            ours: rect.x0 + rect.width as i64,
            theirs: x_max,
            upper: true,
        },
        CropEdge {
            name: "y_min",
            position: y - half,
            ours: rect.y0,
            theirs: y_min,
            upper: false,
        },
        CropEdge {
            name: "y_max",
            position: y + half,
            ours: rect.y0 + rect.height as i64,
            theirs: y_max,
            upper: true,
        },
    ];
    let ties: Vec<String> = edges
        .iter()
        .filter_map(|edge| check_crop_edge(name, edge, f277w.side, "crop_from_radec"))
        .collect();
    assert!(
        (scale - astropy_scale).abs() < 1e-12,
        "{name}: AstroBurst reads {scale} arcsec/px, astropy {astropy_scale}"
    );
    eprintln!("{name}: crop edges judged as ties: {ties:#?}");
    ties
}

#[test]
fn d1_crop_on_a_cd_matrix_without_its_diagonal_follows_crop_from_radec() {
    f277w_crop_matches_crop_from_radec(
        "F277W with only CD1_2 and CD2_1",
        |header| {
            header.remove("CD1_1");
            header.remove("CD2_2");
            header.set("CD1_2", "-1.1111111111111E-05".to_string());
            header.set("CD2_1", "-1.1111111111111E-05".to_string());
        },
        0.0399999999999996,
        [8, 54, 8, 55],
    );
}

#[test]
fn d1_crop_on_cd_next_to_pc_and_cdelt_follows_crop_from_radec() {
    f277w_crop_matches_crop_from_radec(
        "F277W with CD and PC/CDELT",
        |header| {
            for (key, value) in [
                ("PC1_1", "-1.0"),
                ("PC1_2", "0.0"),
                ("PC2_1", "0.0"),
                ("PC2_2", "1.0"),
                ("CDELT1", "2.22222222222222E-05"),
                ("CDELT2", "2.22222222222222E-05"),
            ] {
                header.set(key, value.to_string());
            }
        },
        0.07999999999999992,
        [20, 43, 20, 43],
    );
}

#[test]
fn d1_crop_off_the_half_pixel_grid_has_no_tie() {
    let ties = f277w_crop_matches_crop_from_radec(
        "F277W with CRPIX 32.3/32.7",
        |header| {
            header.set("CRPIX1", "32.3".to_string());
            header.set("CRPIX2", "32.7".to_string());
        },
        0.04,
        [8, 54, 9, 55],
    );
    assert!(
        ties.is_empty(),
        "a crop centre 0.2 px off the half-pixel grid leaves no edge within {CROP_TIE_PX} px of a boundary: {ties:?}"
    );
}

#[test]
fn d1_common_footprint_holds_every_star_and_fits_inside_each_file() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let planes = d1_planes();
    let footprints: Vec<Vec<SkyPoint>> = planes
        .iter()
        .map(|plane| image_footprint(&plane.wcs, (plane.side, plane.side)).unwrap())
        .collect();
    let common = common_polygon(&footprints).unwrap();
    let (center, size) = recipe_crop();
    let square = CropShape::Polygon {
        vertices: square_polygon(center, size),
    };
    for (id, [ra, dec]) in star_positions(&truth) {
        assert!(
            polygon_contains(&common, SkyPoint { ra, dec }),
            "star {id} is outside the common footprint"
        );
    }
    for (index, plane) in planes.iter().enumerate() {
        let frame = CropFrame {
            header: &plane.header,
            wcs: &plane.wcs,
            dims: (plane.side, plane.side),
        };
        let common_rect = CropShape::Polygon {
            vertices: common.clone(),
        }
        .rect(&frame)
        .unwrap();
        assert_eq!(
            fraction_on_image(&common_rect, plane.side, plane.side),
            1.0,
            "{}: {common_rect:?}",
            plane.filter
        );
        let square_rect = square.rect(&frame).unwrap();
        for star in truth["planes"][index]["stars"].as_array().unwrap() {
            let (x, y) = (star["x"].as_f64().unwrap(), star["y"].as_f64().unwrap());
            for (position, start, length) in [
                (x, square_rect.x0, square_rect.width),
                (y, square_rect.y0, square_rect.height),
            ] {
                assert!(
                    position >= start as f64 && position < (start + length as i64) as f64,
                    "{} star {}: {position} outside {square_rect:?}",
                    plane.filter,
                    star["id"]
                );
            }
        }
    }
}

fn centroid_near(image: &Array2<f32>, x: f64, y: f64, radius: f64) -> (f64, f64) {
    let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for ((py, px), value) in image.indexed_iter() {
        let (px, py) = (px as f64, py as f64);
        if value.is_finite() && (px - x).hypot(py - y) <= radius {
            let weight = f64::from(*value);
            sum += weight;
            sx += weight * px;
            sy += weight * py;
        }
    }
    (sx / sum, sy / sum)
}

fn aperture_sum(image: &Array2<f32>, x: f64, y: f64, radius: f64) -> f64 {
    image
        .indexed_iter()
        .filter(|((py, px), value)| {
            value.is_finite() && (*px as f64 - x).hypot(*py as f64 - y) <= radius
        })
        .map(|(_, value)| f64::from(*value))
        .sum()
}

fn sip_pixel_area(plane: &Value, x: f64, y: f64) -> f64 {
    let wcs = &plane["wcs"];
    let Some(sip) = wcs["sip"].as_object() else {
        return 1.0;
    };
    let u = x + 1.0 - wcs["crpix"][0].as_f64().unwrap();
    let v = y + 1.0 - wcs["crpix"][1].as_f64().unwrap();
    let partials = |terms: &Value| {
        terms
            .as_array()
            .unwrap()
            .iter()
            .fold([0.0, 0.0], |[du, dv], term| {
                let (p, q, c) = (
                    term[0].as_i64().unwrap() as i32,
                    term[1].as_i64().unwrap() as i32,
                    term[2].as_f64().unwrap(),
                );
                [
                    du + if p > 0 {
                        c * f64::from(p) * u.powi(p - 1) * v.powi(q)
                    } else {
                        0.0
                    },
                    dv + if q > 0 {
                        c * f64::from(q) * u.powi(p) * v.powi(q - 1)
                    } else {
                        0.0
                    },
                ]
            })
    };
    let ([fu, fv], [gu, gv]) = (partials(&sip["a"]), partials(&sip["b"]));
    (1.0 + fu) * (1.0 + gv) - fv * gu
}

fn physical_aperture_sum(image: &Array2<f32>, plane: &Value, x: f64, y: f64, radius: f64) -> f64 {
    image
        .indexed_iter()
        .filter(|((py, px), value)| {
            value.is_finite() && (*px as f64 - x).hypot(*py as f64 - y) <= radius
        })
        .map(|((py, px), value)| f64::from(*value) * sip_pixel_area(plane, px as f64, py as f64))
        .sum()
}

fn physical_total(model: &[f64], plane: &Value) -> f64 {
    let side = plane["shape"][1].as_u64().unwrap() as usize;
    model
        .iter()
        .enumerate()
        .map(|(pixel, value)| {
            value * sip_pixel_area(plane, (pixel % side) as f64, (pixel / side) as f64)
        })
        .sum()
}

#[test]
fn d1_exact_alignment_to_the_coarsest_grid_conserves_flux_and_lands_stars() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let mut planes = d1_planes();
    let mut images = Vec::new();
    for (index, (plane, data)) in truth["planes"]
        .as_array()
        .unwrap()
        .iter()
        .zip(planes.iter_mut())
        .enumerate()
    {
        let model = d1::render(index).model_jy;
        let jy_per_unit = plane["unit"]["jy_per_unit"].as_f64().unwrap();
        let side = data.side;
        let mut image = Array2::from_shape_fn((side, side), |(y, x)| {
            (model[y * side + x] / jy_per_unit) as f32
        });
        let conversion =
            unit_conversion(&data.header, Some(&data.wcs), TargetUnit::JyPerPixel).unwrap();
        apply_conversion(&conversion, &mut image, None, &mut data.header, None);
        images.push((image, physical_total(&model, plane)));
    }
    let candidates: Vec<ReferenceCandidate> = planes
        .iter()
        .map(|plane| ReferenceCandidate {
            path: &plane.filter,
            scale_arcsec: plane.wcs.pixel_scale_arcsec(),
            filter: Some(&plane.filter),
        })
        .collect();
    let choice = choose_reference(&candidates, &ReferencePolicy::Coarsest).unwrap();
    assert_eq!(planes[choice.index].filter, truth["reference_filter"]);
    let reference = &planes[choice.index];
    let grid = AlignGrid {
        label: &reference.filter,
        header: &reference.header,
        wcs: &reference.wcs,
        rows: reference.side,
        cols: reference.side,
    };
    let exact = AlignOptions::default();
    let bilinear = AlignOptions {
        method: ResampleMethod::Bilinear,
        ..AlignOptions::default()
    };
    let stars = truth["planes"][choice.index]["stars"].as_array().unwrap();
    let reference_scale = truth["planes"][choice.index]["pixel_scale_arcsec"]
        .as_f64()
        .unwrap();
    for ((plane, (image, total_jy)), truth_plane) in planes
        .iter()
        .zip(&images)
        .zip(truth["planes"].as_array().unwrap())
    {
        let input = AlignInput {
            image,
            header: &plane.header,
            wcs: &plane.wcs,
        };
        let plan = plan_alignment(&input, &grid, &exact).unwrap();
        assert_eq!(
            plan.reprojected,
            !["F277W", "F444W"].contains(&plane.filter.as_str()),
            "{}",
            plane.filter
        );
        let aligned = align_image(&input, &grid, &exact, &plan);
        let ratio = finite_sum(&aligned.image) / total_jy;
        assert!(
            (ratio - 1.0).abs() < 1e-3,
            "{}: exact flux ratio {ratio}",
            plane.filter
        );
        let reported = aligned.report.flux_ratio.unwrap();
        assert!(
            (reported - 1.0).abs() < 1e-3,
            "{}: reported flux ratio {reported}",
            plane.filter
        );
        assert!(
            !aligned.report.pixel_area_corrected,
            "{}: flat-fielded values are per nominal pixel",
            plane.filter
        );
        for star in stars {
            let (x, y) = (
                star["x"].as_f64().unwrap() - 1.0,
                star["y"].as_f64().unwrap() - 1.0,
            );
            let (cx, cy) = centroid_near(&aligned.image, x, y, 4.0);
            assert!(
                (cx - x).hypot(cy - y) < 0.15,
                "{} star {}: centroid ({cx}, {cy}) against ({x}, {y})",
                plane.filter,
                star["id"]
            );
            let native = truth_plane["stars"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| candidate["id"] == star["id"])
                .unwrap();
            let (nx, ny) = (native["x"].as_f64().unwrap(), native["y"].as_f64().unwrap());
            let native_scale = truth_plane["pixel_scale_arcsec"].as_f64().unwrap();
            let before = physical_aperture_sum(
                image,
                truth_plane,
                nx,
                ny,
                STAR_APERTURE_ARCSEC / native_scale,
            );
            let after = aperture_sum(&aligned.image, x, y, STAR_APERTURE_ARCSEC / reference_scale);
            let pixel_area = sip_pixel_area(truth_plane, nx, ny);
            eprintln!(
                "{} star {}: aperture ratio {:.5}, input pixel area {pixel_area:.5}",
                plane.filter,
                star["id"],
                after / before
            );
            assert!(
                (after / before - 1.0).abs() < 0.01,
                "{} star {}: aligned aperture flux {after} against {before} (pixel area {pixel_area})",
                plane.filter,
                star["id"]
            );
        }
        let interpolated = align_image(
            &input,
            &grid,
            &bilinear,
            &plan_alignment(&input, &grid, &bilinear).unwrap(),
        );
        eprintln!(
            "{}: total flux ratio exact {ratio:.7}, bilinear {:.7}",
            plane.filter,
            finite_sum(&interpolated.image) / total_jy
        );
    }
}

#[test]
fn d1_bilinear_coordinate_map_matches_the_generator_wcs() {
    let planes = d1_planes();
    let reference_index = planes
        .iter()
        .position(|plane| plane.filter == "F444W")
        .unwrap();
    let reference = &planes[reference_index];
    let grid = AlignGrid {
        label: "F444W",
        header: &reference.header,
        wcs: &reference.wcs,
        rows: reference.side,
        cols: reference.side,
    };
    let options = AlignOptions {
        method: ResampleMethod::Bilinear,
        border_crop: 0,
        area_rule: Some(AreaRule::Keep),
        ..AlignOptions::default()
    };
    for (index, plane) in planes.iter().enumerate() {
        let side = plane.side;
        let ramp = |axis: usize| {
            Array2::from_shape_fn((side, side), |(y, x)| if axis == 0 { x } else { y } as f32)
        };
        let mut coordinates = plane.header.clone();
        coordinates.set("BUNIT", "MJy/sr".to_owned());
        let mapped: Vec<Array2<f32>> = (0..2)
            .map(|axis| {
                let image = ramp(axis);
                let input = AlignInput {
                    image: &image,
                    header: &coordinates,
                    wcs: &plane.wcs,
                };
                let plan = plan_alignment(&input, &grid, &options).unwrap();
                align_image(&input, &grid, &options, &plan).image
            })
            .collect();
        let mut worst = 0.0f64;
        for ((y, x), value) in mapped[0].indexed_iter() {
            let [ra, dec] = d1::pixel_to_world(reference_index, x as f64, y as f64);
            let [sx, sy] = d1::world_to_pixel(index, [ra, dec]);
            let interior = |v: f64| v > 0.5 && v < side as f64 - 1.5;
            if !(interior(sx) && interior(sy)) {
                continue;
            }
            let error = (f64::from(*value) - sx)
                .abs()
                .max((f64::from(mapped[1][[y, x]]) - sy).abs());
            worst = worst.max(error);
        }
        eprintln!(
            "{}: worst coordinate-map error {worst:.2e} px",
            plane.filter
        );
        assert!(
            worst < 1e-3,
            "{}: the coordinate map is off by {worst} px",
            plane.filter
        );
    }
}

fn same_bits(a: &Array2<f32>, b: &Array2<f32>) -> bool {
    a.dim() == b.dim()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.to_bits() == y.to_bits())
}

fn card(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|value| value.trim().trim_matches('\'').trim().to_string())
}

struct D1CubeInputs {
    inputs: Vec<CubeInput>,
    natives: Vec<Array2<f32>>,
    reference: usize,
}

fn d1_cube_inputs() -> D1CubeInputs {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let registry = FilterRegistry::builtin();
    let mut planes = d1_planes();
    let mut natives = Vec::new();
    for (index, data) in planes.iter_mut().enumerate() {
        let model = d1::render(index).model_jy;
        let jy_per_unit = truth["planes"][index]["unit"]["jy_per_unit"]
            .as_f64()
            .unwrap();
        let side = data.side;
        let mut image = Array2::from_shape_fn((side, side), |(y, x)| {
            (model[y * side + x] / jy_per_unit) as f32
        });
        let conversion =
            unit_conversion(&data.header, Some(&data.wcs), TargetUnit::JyPerPixel).unwrap();
        apply_conversion(&conversion, &mut image, None, &mut data.header, None);
        natives.push(image);
    }
    let candidates: Vec<ReferenceCandidate> = planes
        .iter()
        .map(|plane| ReferenceCandidate {
            path: &plane.filter,
            scale_arcsec: plane.wcs.pixel_scale_arcsec(),
            filter: Some(&plane.filter),
        })
        .collect();
    let reference = choose_reference(&candidates, &ReferencePolicy::Coarsest)
        .unwrap()
        .index;
    let grid_plane = &planes[reference];
    let grid = AlignGrid {
        label: &grid_plane.filter,
        header: &grid_plane.header,
        wcs: &grid_plane.wcs,
        rows: grid_plane.side,
        cols: grid_plane.side,
    };
    let options = AlignOptions::default();
    let inputs = planes
        .iter()
        .zip(&natives)
        .map(|(plane, image)| {
            let input = AlignInput {
                image,
                header: &plane.header,
                wcs: &plane.wcs,
            };
            let plan = plan_alignment(&input, &grid, &options).unwrap();
            let aligned = align_image(&input, &grid, &options, &plan);
            let wcs = header_wcs(&aligned.header).unwrap();
            let entry = registry
                .identify(Some(&plane.header), &plane.filter, None)
                .unwrap()
                .entry;
            let cube_plane = CubePlane::from_entry(&entry, &aligned.header, &wcs);
            CubeInput {
                label: plane.filter.clone(),
                sci: aligned.image,
                err: None,
                dq: None,
                header: aligned.header,
                wcs,
                plane: cube_plane,
            }
        })
        .collect();
    D1CubeInputs {
        inputs,
        natives,
        reference,
    }
}

#[test]
fn d1_cube_stacks_the_aligned_planes_in_pivot_order_with_the_lightstack_keywords() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let registry = FilterRegistry::builtin();
    let D1CubeInputs {
        inputs,
        natives,
        reference,
    } = d1_cube_inputs();
    let aligned: Vec<Array2<f32>> = inputs.iter().map(|input| input.sci.clone()).collect();
    let reference_header = inputs[reference].header.clone();
    let reference_wcs = header_wcs(&reference_header).unwrap();
    let options = CubeOptions {
        layout: CubeLayout::Lightstack,
        order: PlaneOrder::Wavelength,
    };
    let assembled = assemble_cube(
        inputs,
        reference,
        cube_base_header(&reference_header),
        options,
    )
    .unwrap();
    assert!(assembled.warnings.is_empty(), "{:?}", assembled.warnings);
    let cube = &assembled.cube;
    let expected: Vec<&str> = truth["planes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plane| plane["filter"].as_str().unwrap())
        .collect();
    let filters: Vec<&str> = cube
        .filters
        .iter()
        .map(|plane| plane.filter.as_str())
        .collect();
    assert_eq!(
        filters, expected,
        "the D1 planes are already in pivot order"
    );
    assert_eq!(assembled.order, [0, 1, 2, 3, 4, 5]);
    for (z, &index) in assembled.order.iter().enumerate() {
        assert!(
            same_bits(&cube.planes[z], &aligned[index]),
            "{}: the cube plane is not a copy of the aligned image",
            filters[z]
        );
    }

    let header = &cube.header;
    for (z, plane) in truth["planes"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            card(header, &format!("FILTER{}", z + 1)).as_deref(),
            plane["filter"].as_str()
        );
        assert_eq!(
            card(header, &format!("DATASET{}", z + 1)).as_deref(),
            plane["dataset"].as_str()
        );
    }
    assert_eq!(header.get_i64("NFILTERS"), Some(6));
    assert_eq!(header.get_i64("NAXIS3"), Some(6));
    assert_eq!(
        card(header, "FILTERS").as_deref(),
        Some("F814W,F150W,F200W,F277W,F356W,F444W")
    );
    assert_eq!(
        card(header, "DATASETS").as_deref(),
        Some("HST,JWST,JWST,JWST,JWST,JWST")
    );
    assert_eq!(card(header, "BUNIT").as_deref(), Some("Jy"));
    assert_eq!(card(header, "CTYPE3").as_deref(), Some("FILTER"));
    assert_eq!(
        (
            header.get_f64("CRPIX3"),
            header.get_f64("CRVAL3"),
            header.get_f64("CDELT3"),
            header.get_i64("WCSAXES")
        ),
        (Some(1.0), Some(0.0), Some(1.0), Some(3))
    );
    let cube_wcs = header_wcs(header).unwrap();
    for (x, y) in [(0.0, 0.0), (61.0, 61.0), (30.5, 12.25)] {
        let (a, b) = (
            cube_wcs.pixel_to_world(x, y),
            reference_wcs.pixel_to_world(x, y),
        );
        assert!(
            (a.ra - b.ra).abs() < 1e-12 && (a.dec - b.dec).abs() < 1e-12,
            "the cube WCS moved pixel ({x}, {y})"
        );
    }
    let f814w = &cube.filters[0];
    assert_eq!(
        (f814w.bunit_in.as_str(), f814w.dataset.as_str()),
        ("ELECTRONS/S", "HST")
    );
    let truth_factor = truth["planes"][0]["unit"]["jy_per_unit"].as_f64().unwrap();
    assert!(relative_difference(f814w.fluxconv.unwrap(), truth_factor) < 1e-12);

    let reference_scale = truth["planes"][reference]["pixel_scale_arcsec"]
        .as_f64()
        .unwrap();
    let stars = truth["planes"][reference]["stars"].as_array().unwrap();
    for (z, &index) in assembled.order.iter().enumerate() {
        let truth_plane = &truth["planes"][index];
        let native_scale = truth_plane["pixel_scale_arcsec"].as_f64().unwrap();
        for star in stars {
            let (x, y) = (
                star["x"].as_f64().unwrap() - 1.0,
                star["y"].as_f64().unwrap() - 1.0,
            );
            let native = truth_plane["stars"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| candidate["id"] == star["id"])
                .unwrap();
            let before = physical_aperture_sum(
                &natives[index],
                truth_plane,
                native["x"].as_f64().unwrap(),
                native["y"].as_f64().unwrap(),
                STAR_APERTURE_ARCSEC / native_scale,
            );
            let after = aperture_sum(
                &cube.planes[z],
                x,
                y,
                STAR_APERTURE_ARCSEC / reference_scale,
            );
            assert!(
                (after / before - 1.0).abs() < 0.01,
                "{} star {}: cube aperture flux {after} against the native {before}",
                filters[z],
                star["id"]
            );
        }
    }

    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("d1_cube.fits");
    write_fits_cube(path.to_str().unwrap(), &cube.planes, Some(&cube.header)).unwrap();
    let file = open(&path);
    let written = extract_header_by_index(&file, 0).unwrap();
    assert_eq!(written.get_i64("BITPIX"), Some(-32));
    assert_eq!(card(&written, "DATASETS"), card(header, "DATASETS"));
    let back = extract_image_planes_by_index(&file, 0).unwrap();
    assert!(back
        .iter()
        .zip(&cube.planes)
        .all(|(read, plane)| same_bits(read, plane)));
    let labels = planes_from_header(&written, 6, &registry).unwrap();
    let pivots: Vec<f64> = labels.iter().map(|plane| plane.pivot_um.unwrap()).collect();
    assert!(
        pivots.windows(2).all(|pair| pair[0] < pair[1]),
        "{pivots:?}"
    );
    assert!((pivots[5] - 4.404315).abs() < 1e-12);
}

#[test]
fn d1_cube_edits_follow_the_lightstack_rules_and_keep_the_dataset_keys() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let D1CubeInputs {
        inputs, reference, ..
    } = d1_cube_inputs();
    let base = cube_base_header(&inputs[reference].header);
    let options = CubeOptions {
        layout: CubeLayout::Mef,
        order: PlaneOrder::Wavelength,
    };
    let cube = assemble_cube(inputs, reference, base, options)
        .unwrap()
        .cube;

    for (plane, label) in cube.planes.iter().zip(&cube.filters) {
        let finite: Vec<f32> = plane.iter().copied().filter(|v| v.is_finite()).collect();
        let above = finite.iter().filter(|&&v| v > 0.0).count() as f64 / finite.len() as f64;
        eprintln!(
            "{}: {:.4} of the finite pixels above 0",
            label.filter, above
        );
    }
    let (valid, removed) = drop_invalid_planes(&cube, 0.0, DEFAULT_FRAC_VALID, false).unwrap();
    assert!(removed.is_empty(), "{removed:?}");
    assert_eq!(valid.planes.len(), 6);
    let mut blank = cube.clone();
    blank.planes[2].fill(0.0);
    let (_, removed) = drop_invalid_planes(&blank, 0.0, DEFAULT_FRAC_VALID, false).unwrap();
    assert_eq!(removed, ["F200W"]);

    let without_hst = remove_filter(&cube, "F814W").unwrap();
    assert_eq!(
        card(&without_hst.header, "DATASETS").as_deref(),
        Some("JWST,JWST,JWST,JWST,JWST")
    );
    assert_eq!(
        card(&without_hst.header, "DATASET1").as_deref(),
        Some("JWST"),
        "LIGHTSTACK would leave DATASET1 = HST"
    );
    assert!(without_hst.header.get("DATASET6").is_none());

    let (bx, by) = brightest_pixel(&cube.planes).unwrap();
    let stars = truth["planes"][reference]["stars"].as_array().unwrap();
    let positions = star_positions(&truth);
    let distance = |star: &Value| {
        (star["x"].as_f64().unwrap() - 1.0 - bx as f64)
            .hypot(star["y"].as_f64().unwrap() - 1.0 - by as f64)
    };
    let nearest = stars
        .iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .unwrap();
    assert!(
        distance(nearest) < 1.0,
        "brightest pixel ({bx}, {by}) is {} px from star {}",
        distance(nearest),
        nearest["id"]
    );
    let mut described = cube.clone();
    let centre = update_metadata(&mut described, Some(0.303), None, true).unwrap();
    assert_eq!((centre.x, centre.y), (Some(bx), Some(by)));
    let [ra, dec] = positions[nearest["id"].as_str().unwrap()];
    let separation_arcsec =
        ((centre.ra - ra) * dec.to_radians().cos()).hypot(centre.dec - dec) * 3600.0;
    assert!(separation_arcsec < 0.04, "{separation_arcsec} arcsec");
    assert_eq!(
        card(&described.header, "CEN_TYPE").as_deref(),
        Some("BRIGHTEST")
    );

    let star = &stars[0];
    let (sx, sy) = (
        star["x"].as_f64().unwrap() - 1.0,
        star["y"].as_f64().unwrap() - 1.0,
    );
    let (x0, y0) = ((sx - 8.0).floor() as i64, (sy - 8.0).floor() as i64);
    let cut = subcube(&cube, x0, x0 + 16, y0, y0 + 16).unwrap();
    let radius = STAR_APERTURE_ARCSEC / 0.04;
    for (full, part) in cube.planes.iter().zip(&cut.planes) {
        assert_eq!(
            aperture_sum(full, sx, sy, radius),
            aperture_sum(part, sx - x0 as f64, sy - y0 as f64, radius)
        );
    }
    let (a, b) = (
        cube.wcs().unwrap().pixel_to_world(sx, sy),
        cut.wcs()
            .unwrap()
            .pixel_to_world(sx - x0 as f64, sy - y0 as f64),
    );
    assert!((a.ra - b.ra).abs() < 1e-12 && (a.dec - b.dec).abs() < 1e-12);

    let any_nan = |y: usize, x: usize| cube.planes.iter().any(|plane| !plane[[y, x]].is_finite());
    let nan_pixels = cube.planes[0]
        .indexed_iter()
        .filter(|((y, x), _)| any_nan(*y, *x))
        .count();
    assert!(nan_pixels > 0, "the rotated grids leave uncovered corners");
    let mut masked = cube.clone();
    apply_nan_mask(&mut masked, MaskMode::Union, 0, &[1; 6]).unwrap();
    for (z, plane) in masked.planes.iter().enumerate() {
        for ((y, x), value) in plane.indexed_iter() {
            if any_nan(y, x) {
                assert!(value.is_nan(), "{} ({x}, {y})", cube.filters[z].filter);
            } else {
                assert_eq!(value.to_bits(), cube.planes[z][[y, x]].to_bits());
            }
        }
    }
    eprintln!(
        "union NaN mask: {nan_pixels} of {} pixels",
        cube.planes[0].len()
    );
}

#[test]
fn oracle_d1_crop_bounds_match_crop_from_radec() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let (center, size) = recipe_crop();
    let planes = d1_planes();
    let crops = oracle_calls(&manifest, "crop");
    assert_eq!(
        crops.len(),
        planes.len(),
        "one crop_from_radec call per plane"
    );
    let mut ties = Vec::new();
    for call in crops {
        assert_eq!(call["function"], "lightstack.crop.crop_from_radec");
        let metrics = &call["metrics"];
        let plane = planes
            .iter()
            .find(|plane| metrics["filter"] == plane.filter.as_str())
            .unwrap();
        let wcs = lightstack_wcs(&plane.header).unwrap();
        let rect = lightstack_square_rect(&wcs, (plane.side, plane.side), center, size).unwrap();
        let (x, y) = astroburst_crop_centre(&wcs, center);
        let half = size / wcs.pixel_scale_arcsec() / 2.0;
        let integer = |value: &Value| value.as_f64().unwrap() as i64;
        let (x_min, y_min) = (integer(&metrics["x_min"]), integer(&metrics["y_min"]));
        let (rows, cols) = (integer(&metrics["shape"][0]), integer(&metrics["shape"][1]));
        let edges = [
            CropEdge {
                name: "x_min",
                position: x - half,
                ours: rect.x0,
                theirs: x_min,
                upper: false,
            },
            CropEdge {
                name: "x_max",
                position: x + half,
                ours: rect.x0 + rect.width as i64,
                theirs: x_min + cols,
                upper: true,
            },
            CropEdge {
                name: "y_min",
                position: y - half,
                ours: rect.y0,
                theirs: y_min,
                upper: false,
            },
            CropEdge {
                name: "y_max",
                position: y + half,
                ours: rect.y0 + rect.height as i64,
                theirs: y_min + rows,
                upper: true,
            },
        ];
        ties.extend(edges.iter().filter_map(|edge| {
            check_crop_edge(&plane.filter, edge, plane.side, "crop_from_radec")
        }));
    }
    eprintln!("crop edges judged as ties on AstroBurst's continuous positions: {ties:#?}");
}

#[test]
fn oracle_d1_tan_reference_meridian_crops_match_on_a_one_ulp_tie() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let (center, size) = recipe_crop();
    let planes = d1_planes();
    for filter in ["F277W", "F444W"] {
        let plane = planes.iter().find(|plane| plane.filter == filter).unwrap();
        let call = oracle_calls(&manifest, "crop").into_iter().find(|call| call["metrics"]["filter"] == filter).unwrap();
        let metrics = &call["metrics"];
        let wcs = lightstack_wcs(&plane.header).unwrap();
        let rect = lightstack_square_rect(&wcs, (plane.side, plane.side), center, size).unwrap();
        let (x, y) = astroburst_crop_centre(&wcs, center);
        let excess = (center.dec - f64_card(&plane.header, "CRVAL2")) / f64_card(&plane.header, "CD2_2");
        assert_eq!(
            x, 31.5,
            "{filter}: the crop centre sits on the half-pixel grid in x, so x_min 9 and x_max 54 are exact ties"
        );
        assert!(
            ((y - 31.5) - excess).abs() < 1e-13,
            "{filter}: the crop centre is {:e} px above the half-pixel grid in y, not (dec - CRVAL2) / CD2_2 = {excess:e} px",
            y - 31.5
        );
        assert!(
            excess > 0.0 && excess < 1e-9,
            "{filter}: the stored CRVAL2 is one ulp south of the crop Dec, which puts y_min and y_max {excess:e} px above the boundaries 9 and 54"
        );
        assert_eq!((rect.x0, rect.y0, rect.width, rect.height), (
            metrics["x_min"].as_f64().unwrap() as i64,
            metrics["y_min"].as_f64().unwrap() as i64,
            metrics["shape"][1].as_u64().unwrap() as usize,
            metrics["shape"][0].as_u64().unwrap() as usize,
        ), "{filter}: the D1 reference-meridian crop is a tie: AstroBurst rounds the y edges {excess:e} px above 9 and 54 to 9 and 55, and crop_from_radec rounded the same way because its wcslib centre lands +1.46e-9 px above the boundary under astropy 8.0.2 (-3.16e-10 px for a zero CRVAL offset), so a regenerated golden may give y_min 8 with AstroBurst unchanged");
        assert_eq!(
            (rect.width, rect.height),
            (45, 46),
            "{filter}: 45 by 46 holds only while the y edges stay a hair above the pixel boundaries (a one-ulp tie, not WCS geometry); the compatibility cube's 44 rows by 43 columns follow from it"
        );
    }
}

#[test]
fn oracle_d1_units_match_counts_s_to_jy_and_mjy_sr_to_jy() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let units = oracle_step(&manifest, "units");
    assert_eq!(units["parameters"]["to"], TargetUnit::JyPerPixel.label());
    let mut converted = Vec::new();
    for call in oracle_calls(&manifest, "units") {
        let metrics = &call["metrics"];
        let filter = metrics["filter"].as_str().unwrap();
        let [input] = &call["inputs"].as_array().unwrap()[..] else {
            panic!("{filter}: one 01_crop input")
        };
        let [output] = &call["outputs"].as_array().unwrap()[..] else {
            panic!("{filter}: one 02_units output")
        };
        let crop = extract_image_mmap_by_index(&open(&oracle_entry_path(input)), 0).unwrap();
        let (mut header, mut image) = (crop.header.clone(), crop.image.clone());
        let wcs = header_wcs(&header).ok();
        let conversion = unit_conversion(&header, wcs.as_ref(), TargetUnit::JyPerPixel)
            .unwrap_or_else(|error| panic!("{filter}: {error}"));
        assert_eq!(
            conversion.bunit_in.as_deref(),
            metrics["bunit_in"].as_str(),
            "{filter}: input BUNIT"
        );
        let (recorded, bound) = match (
            metrics["factor"].as_f64(),
            metrics["factor_printed"].as_f64(),
        ) {
            (Some(factor), None) => (factor, 1e-12),
            (None, Some(printed)) => (printed, 5e-7),
            other => panic!("{filter}: the call records the factors {other:?}"),
        };
        assert!(
            relative_difference(conversion.factor, recorded) <= bound,
            "{filter}: AstroBurst converts by x{:e}, LIGHTSTACK by x{recorded:e}",
            conversion.factor
        );
        if let Some(method) = metrics["method"].as_str() {
            assert!(
                conversion.method.starts_with(method),
                "{filter}: AstroBurst converts by {}, LIGHTSTACK by {method}",
                conversion.method
            );
        }
        apply_conversion(&conversion, &mut image, None, &mut header, None);
        let (golden_header, mut planes) = f64_planes(&oracle_entry_path(output));
        assert_eq!(
            planes.len(),
            1,
            "{filter}: one image in the 02_units golden"
        );
        assert_eq!(
            card(&header, "BUNIT"),
            card(&golden_header, "BUNIT"),
            "{filter}: output BUNIT"
        );
        let error = worst_relative(filter, &image.mapv(f64::from), &planes.remove(0));
        eprintln!(
            "{filter}: x{:e} by {}, {error:.2e} of the max abs value against {}",
            conversion.factor, conversion.method, call["function"]
        );
        assert!(error <= 2e-6, "{filter}: {error:e} (F class)");
        converted.push(filter.to_string());
    }
    assert_eq!(converted, ["F814W", "F150W", "F200W", "F444W"]);
    let crops = oracle_calls(&manifest, "crop");
    let passed_through: Vec<&str> = units["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|note| {
            note.as_str()
                .unwrap()
                .strip_suffix(": BUNIT Jy, passed through")
        })
        .collect();
    assert_eq!(passed_through, ["F277W", "F356W"]);
    for filter in passed_through {
        let crop = crops
            .iter()
            .find(|call| call["metrics"]["filter"] == filter)
            .unwrap_or_else(|| panic!("{filter}: no crop_from_radec call"));
        let header = extract_image_mmap_by_index(&open(&oracle_entry_path(&crop["outputs"][0])), 0)
            .unwrap()
            .header;
        let conversion = unit_conversion(
            &header,
            header_wcs(&header).ok().as_ref(),
            TargetUnit::JyPerPixel,
        )
        .unwrap();
        assert_eq!(
            conversion.factor, 1.0,
            "{filter}: LIGHTSTACK passed it through"
        );
    }
}

struct OracleAlignment {
    call: Value,
    filters: Vec<String>,
    inputs: Vec<(HduHeader, Array2<f32>)>,
    outputs: Vec<(HduHeader, Array2<f32>)>,
    reference: usize,
    reference_wcs: WcsTransform,
}

impl OracleAlignment {
    fn load() -> Self {
        let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
        let align = oracle_calls(&manifest, "align");
        assert_eq!(align.len(), 1, "one align_reproject_fits call");
        let call = align[0].clone();
        let golden = |entry: &Value| {
            let path = oracle_file(entry);
            let read = extract_image_mmap_by_index(&open(&path), 0)
                .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
            (read.header, read.image)
        };
        let filters: Vec<String> = call["arguments"]["fits_list"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item[2].as_str().unwrap().to_owned())
            .collect();
        let inputs: Vec<(HduHeader, Array2<f32>)> = call["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(golden)
            .collect();
        let outputs = call["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(golden)
            .collect();
        let reference = filters
            .iter()
            .position(|filter| call["metrics"]["reference"] == filter.as_str())
            .unwrap();
        let reference_wcs = header_wcs(&inputs[reference].0).unwrap();
        Self {
            call,
            filters,
            inputs,
            outputs,
            reference,
            reference_wcs,
        }
    }

    fn grid(&self) -> AlignGrid<'_> {
        let (header, image) = &self.inputs[self.reference];
        let (rows, cols) = image.dim();
        AlignGrid {
            label: &self.filters[self.reference],
            header,
            wcs: &self.reference_wcs,
            rows,
            cols,
        }
    }

    fn options(&self, area_rule: AreaRule) -> AlignOptions {
        AlignOptions {
            method: ResampleMethod::Bilinear,
            reference: ReferencePolicy::Filter {
                filter: self.filters[self.reference].clone(),
            },
            border_crop: self.call["arguments"]["crop"].as_u64().unwrap() as usize,
            area_rule: Some(area_rule),
            per_actual_pixel: false,
        }
    }

    fn plane_metrics(&self, filter: &str) -> &Value {
        &self.call["metrics"]["planes"][filter]
    }
}

#[test]
fn oracle_d1_bilinear_alignment_matches_align_reproject_fits() {
    let alignment = OracleAlignment::load();
    let grid = alignment.grid();
    let options = alignment.options(AreaRule::Lightstack);
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let positions = star_positions(&truth);
    let mut classes = BTreeSet::new();
    for (((filter, (header, image)), (output_header, expected)), output) in alignment
        .filters
        .iter()
        .zip(&alignment.inputs)
        .zip(&alignment.outputs)
        .zip(alignment.call["outputs"].as_array().unwrap())
    {
        let wcs = header_wcs(header).unwrap();
        let input = AlignInput {
            image,
            header,
            wcs: &wcs,
        };
        let plan = plan_alignment(&input, &grid, &options).unwrap();
        assert_eq!(
            Some(plan.reprojected),
            alignment.plane_metrics(filter)["reprojected"].as_bool(),
            "{filter}: AstroBurst and align_reproject_fits disagree on whether the plane is reprojected"
        );
        let got = align_image(&input, &grid, &options, &plan).image;
        assert_eq!(got.dim(), expected.dim(), "{filter}");
        let class = if !plan.reprojected {
            AlignClass::Exact
        } else if header.get("A_ORDER").is_some() {
            AlignClass::Equivalent
        } else {
            AlignClass::Float
        };
        classes.insert(class);
        let error = assert_align_class(filter, class, &got, &oracle_file(output));
        match class {
            AlignClass::Exact => {
                eprintln!("{filter}: E, not reprojected, identical to align_reproject_fits");
            }
            AlignClass::Float => {
                eprintln!(
                    "{filter}: F, TAN, max difference against align_reproject_fits {error:.2e} of the max abs value over every pixel, NaN masks equal"
                );
            }
            AlignClass::Equivalent => {
                let worst = error;
                let native = truth["planes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|plane| plane["filter"] == filter.as_str())
                    .unwrap();
                let output_wcs = header_wcs(output_header).unwrap();
                let radius = native["stars"][0]["aperture_radius_px"].as_f64().unwrap()
                    * native["pixel_scale_arcsec"].as_f64().unwrap()
                    / output_wcs.pixel_scale_arcsec();
                let mut apertures = Vec::new();
                for (id, [ra, dec]) in &positions {
                    let (x, y) = output_wcs.world_to_pixel(*ra, *dec);
                    let Some(theirs) = interior_aperture_sum(expected, x, y, radius) else {
                        continue;
                    };
                    let ours = interior_aperture_sum(&got, x, y, radius).unwrap();
                    let difference = relative_difference(ours, theirs);
                    apertures.push(format!("{id} {difference:.2e}"));
                    assert!(
                        difference <= 1e-3,
                        "{filter} star {id}: aperture sum {ours} Jy against LIGHTSTACK {theirs} Jy (I class 0.1 percent)"
                    );
                }
                eprintln!(
                    "{filter}: I, SIP, max interior difference against align_reproject_fits {worst:.2e} of the 0.1 to 99.9 percentile range, interior NaN masks equal; relative aperture-sum differences within {radius:.2} px: {apertures:?}"
                );
                assert!(
                    !apertures.is_empty(),
                    "{filter}: no D1 star has its aperture inside the interior"
                );
            }
        }
    }
    assert_eq!(
        classes.into_iter().collect::<Vec<_>>(),
        [AlignClass::Exact, AlignClass::Float, AlignClass::Equivalent],
        "the D1 alignment exercises the E, F and I classes"
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AlignClass {
    Exact,
    Float,
    Equivalent,
}

fn identical(a: &Array2<f32>, b: &Array2<f32>) -> bool {
    a.dim() == b.dim()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()))
}

fn identical_f64(a: &Array2<f64>, b: &Array2<f64>) -> bool {
    a.dim() == b.dim()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()))
}

fn assert_align_class(label: &str, class: AlignClass, got: &Array2<f32>, golden: &Path) -> f64 {
    let expected = first_image(golden).1;
    assert_eq!(
        got.dim(),
        expected.dim(),
        "{label}: shape against {}",
        golden.display()
    );
    match class {
        AlignClass::Exact => {
            assert!(
                identical(got, &expected),
                "{label}: a plane that is not reprojected must be LIGHTSTACK's copy read at float32, bit for bit (E class)"
            );
            0.0
        }
        AlignClass::Float => {
            let reference = f64_planes(golden).1.remove(0);
            let error = worst_relative(label, &got.mapv(f64::from), &reference);
            assert!(
                error <= 2e-6,
                "{label}: {error:e} of the max abs value (F class)"
            );
            error
        }
        AlignClass::Equivalent => {
            let range = percentile_range(&expected);
            let (rows, cols) = got.dim();
            let mut worst = 0.0f64;
            let mut one_sided = Vec::new();
            for ((y, x), value) in got.indexed_iter() {
                let interior = (2..rows - 2).contains(&y) && (2..cols - 2).contains(&x);
                let reference_value = expected[[y, x]];
                if !interior {
                    continue;
                }
                if value.is_finite() != reference_value.is_finite() {
                    one_sided.push((x, y, *value, reference_value));
                } else if value.is_finite() {
                    let difference = f64::from((value - reference_value).abs());
                    worst = worst.max(difference / range);
                    assert!(
                        difference <= 1e-3 * range,
                        "{label} ({x}, {y}): {value} against LIGHTSTACK {reference_value} (I class)"
                    );
                }
            }
            assert!(
                one_sided.is_empty(),
                "{label}: the interior NaN masks differ (E class) at {} pixels; first (x, y, AstroBurst, LIGHTSTACK): {:?}",
                one_sided.len(),
                &one_sided[..one_sided.len().min(4)]
            );
            worst
        }
    }
}

fn interior_aperture_sum(image: &Array2<f32>, x: f64, y: f64, radius: f64) -> Option<f64> {
    let (rows, cols) = image.dim();
    image
        .indexed_iter()
        .filter(|((py, px), _)| (*px as f64 - x).hypot(*py as f64 - y) <= radius)
        .map(|((py, px), value)| {
            let interior = (2..rows - 2).contains(&py) && (2..cols - 2).contains(&px);
            (interior && value.is_finite()).then(|| f64::from(*value))
        })
        .sum()
}

fn oracle_file(entry: &Value) -> PathBuf {
    oracle_d1_dir().join(entry["path"].as_str().unwrap())
}

fn oracle_cube(entry: &Value, registry: &FilterRegistry) -> Cube {
    let path = oracle_file(entry);
    let file = open(&path);
    let header = extract_header_by_index(&file, 0).unwrap();
    let planes = extract_image_planes_by_index(&file, 0)
        .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
    let filters = planes_from_header(&header, planes.len(), registry).unwrap();
    Cube {
        header,
        planes,
        err: None,
        dq: None,
        filters,
    }
}

fn same_card(got: Option<&str>, expected: &str) -> bool {
    let clean = |text: &str| text.trim().trim_matches('\'').trim().to_string();
    match got {
        None => false,
        Some(got) => match (got.trim().parse::<f64>(), expected.trim().parse::<f64>()) {
            (Ok(a), Ok(b)) => (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0),
            _ => clean(got) == clean(expected),
        },
    }
}

#[test]
fn oracle_d1_cube_matches_build_datacube() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let registry = FilterRegistry::builtin();
    let calls = oracle_calls(&manifest, "cube");
    assert_eq!(calls.len(), 1, "one build_datacube call");
    let call = calls[0];
    let selected = call["arguments"]["selected_fits"].as_array().unwrap();
    let reference_file = call["arguments"]["reference_file"].as_str().unwrap();
    let reference = selected
        .iter()
        .position(|item| item[0].as_str() == Some(reference_file))
        .unwrap();
    let inputs: Vec<CubeInput> = call["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .zip(selected)
        .map(|(entry, item)| {
            let path = oracle_file(entry);
            let read = extract_image_mmap_by_index(&open(&path), 0).unwrap();
            let wcs = header_wcs(&read.header).unwrap();
            let (dataset, filter) = (item[1].as_str().unwrap(), item[2].as_str().unwrap());
            let entry = registry.entry(dataset, filter).unwrap().clone();
            let plane = CubePlane::from_entry(&entry, &read.header, &wcs);
            CubeInput {
                label: filter.to_owned(),
                sci: read.image,
                err: None,
                dq: None,
                header: read.header,
                wcs,
                plane,
            }
        })
        .collect();
    let base = cube_base_header(&inputs[reference].header);
    let options = CubeOptions {
        layout: CubeLayout::Lightstack,
        order: PlaneOrder::Given,
    };
    let ours = assemble_cube(inputs, reference, base, options)
        .unwrap()
        .cube;
    let golden = oracle_cube(&call["outputs"][0], &registry);
    assert_eq!(ours.planes.len(), golden.planes.len());
    for (z, (a, b)) in ours.planes.iter().zip(&golden.planes).enumerate() {
        assert!(
            same_bits(a, b),
            "plane {z}: LIGHTSTACK float64 data read at float32 must equal the cube plane"
        );
    }
    let astropy_wcs_defaults = [
        "LONPOLE", "LATPOLE", "MJDREF", "DATEREF", "MJDREFI", "MJDREFF",
    ];
    let mut missing = Vec::new();
    for (key, value) in &golden.header.cards {
        let key = key.trim();
        if matches!(
            key,
            "SIMPLE" | "BITPIX" | "NAXIS" | "EXTEND" | "HISTORY" | "COMMENT"
        ) || key.starts_with("NAXIS")
        {
            continue;
        }
        if !same_card(ours.header.get(key), value) {
            missing.push(format!(
                "{key} = {value} (AstroBurst {:?})",
                ours.header.get(key)
            ));
        }
    }
    let unexpected: Vec<&String> = missing
        .iter()
        .filter(|line| !astropy_wcs_defaults.iter().any(|key| line.starts_with(key)))
        .collect();
    eprintln!("keywords LIGHTSTACK writes that differ: {missing:?}");
    assert!(unexpected.is_empty(), "{unexpected:?}");
}

#[test]
fn oracle_d1_drop_invalid_matches_build_valid_datacube() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let registry = FilterRegistry::builtin();
    let calls = oracle_calls(&manifest, "edit");
    let call = calls
        .iter()
        .find(|call| call["function"] == "lightstack.datacube.build_valid_datacube")
        .expect("a build_valid_datacube call");
    let source = oracle_cube(&call["inputs"][0], &registry);
    let golden = oracle_cube(&call["outputs"][0], &registry);
    let (valid, removed) = drop_invalid_planes(
        &source,
        call["arguments"]["threshold"].as_f64().unwrap(),
        call["arguments"]["frac_valid"].as_f64().unwrap(),
        false,
    )
    .unwrap();
    let expected_removed: Vec<&str> = call["metrics"]["removed_filters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    assert_eq!(removed, expected_removed);
    assert_eq!(valid.planes.len(), golden.planes.len());
    for (a, b) in valid.planes.iter().zip(&golden.planes) {
        assert!(same_bits(a, b));
    }
    for key in ["NFILTERS", "FILTERS"] {
        assert_eq!(card(&valid.header, key), card(&golden.header, key), "{key}");
    }
    for (index, plane) in valid.filters.iter().enumerate() {
        let n = index + 1;
        assert_eq!(
            card(&golden.header, &format!("FILTER{n}")).as_deref(),
            Some(plane.filter.as_str())
        );
        let theirs = card(&golden.header, &format!("DATASET{n}"))
            .or_else(|| card(&golden.header, &format!("HIERARCH DATASET{n}")));
        if theirs.as_deref() != Some(plane.dataset.as_str()) {
            assert_eq!(
                theirs.as_deref(),
                Some(source.filters[index].dataset.as_str()),
                "LIGHTSTACK leaves DATASET{n} as it was before the removal"
            );
        }
    }
}

#[test]
fn oracle_manifest_records_library_versions() {
    let mut errors = Vec::new();
    for (goldens, data, recipe) in [
        (oracle_d1_dir(), d1_dir(), RECIPE),
        (oracle_d1_stars_dir(), d1_stars_dir(), STARS_RECIPE),
    ] {
        let manifest = read_json(&goldens.join(MANIFEST));
        let folder = goldens.file_name().unwrap().to_string_lossy().into_owned();
        errors.extend(
            oracle_manifest_errors(&manifest)
                .into_iter()
                .map(|error| format!("{folder}: {error}")),
        );
        let recipe_bytes = std::fs::read(data.join(recipe)).unwrap();
        if manifest["recipe"]["sha256"].as_str() != Some(sha256_hex(&recipe_bytes).as_str()) {
            errors.push(format!(
                "{folder}: the goldens were not produced from {}",
                data.join(recipe).display()
            ));
        }
        let committed: BTreeMap<String, String> = listed_files(&read_json(&data.join(MANIFEST)))
            .into_iter()
            .collect();
        let recipe_inputs: Vec<&Value> = manifest["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        let call_inputs: Vec<&Value> = manifest["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|step| step["calls"].as_array().into_iter().flatten())
            .flat_map(|call| call["inputs"].as_array().into_iter().flatten())
            .filter(|input| input["root"] == "data")
            .collect();
        for (what, inputs) in [
            ("recipe input", recipe_inputs),
            ("data-rooted call input", call_inputs),
        ] {
            if inputs.is_empty() {
                errors.push(format!(
                    "{folder}: no {what} was checked against {}",
                    data.display()
                ));
            }
            for input in inputs {
                let committed_file = input["path"].as_str().and_then(|path| committed.get(path));
                if committed_file.is_none_or(|sha256| input["sha256"].as_str() != Some(sha256)) {
                    errors.push(format!(
                        "{folder}: {what} {input} is not a committed file of {}",
                        data.display()
                    ));
                }
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn oracle_reproduces_the_d1_area_factor_and_gaussian_psf_fwhm() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let align = oracle_calls(&manifest, "align");
    assert_eq!(align.len(), 1, "one align_reproject_fits call");
    for plane in truth["planes"].as_array().unwrap() {
        let filter = plane["filter"].as_str().unwrap();
        let metrics = &align[0]["metrics"]["planes"][filter];
        let ratio = (0.04 / plane["pixel_scale_arcsec"].as_f64().unwrap()).powi(2);
        if ratio == 1.0 {
            assert!(metrics["area_correction"].is_null(), "{filter}: {metrics}");
        } else {
            assert_eq!(
                metrics["area_correction"].as_f64(),
                Some(ratio),
                "{filter}: {metrics}"
            );
        }
    }
    assert_eq!(
        align[0]["metrics"]["planes"]["F277W"]["reprojected"], false,
        "F277W shares the F444W grid"
    );
    let alignment = OracleAlignment::load();
    let grid = alignment.grid();
    let (lightstack, keep) = (
        alignment.options(AreaRule::Lightstack),
        alignment.options(AreaRule::Keep),
    );
    for (filter, (header, image)) in alignment.filters.iter().zip(&alignment.inputs) {
        let metrics = alignment.plane_metrics(filter);
        let wcs = header_wcs(header).unwrap();
        let input = AlignInput {
            image,
            header,
            wcs: &wcs,
        };
        let plan = plan_alignment(&input, &grid, &lightstack).unwrap();
        assert_eq!(
            Some(plan.reprojected),
            metrics["reprojected"].as_bool(),
            "{filter}"
        );
        let planned = plan
            .area
            .as_ref()
            .map(|area| area.data_factor)
            .filter(|factor| *factor != 1.0);
        let recorded = metrics["area_correction"].as_f64();
        assert_eq!(
            planned.is_some(),
            recorded.is_some(),
            "{filter}: AstroBurst plans the LIGHTSTACK area factor {planned:?}, align_reproject_fits applied {recorded:?}"
        );
        let scaled = finite_sum(&align_image(&input, &grid, &lightstack, &plan).image);
        let unscaled = finite_sum(
            &align_image(
                &input,
                &grid,
                &keep,
                &plan_alignment(&input, &grid, &keep).unwrap(),
            )
            .image,
        );
        let applied = scaled / unscaled;
        let expected = recorded.unwrap_or(1.0);
        eprintln!(
            "{filter}: area factor planned {planned:?}, applied x{applied:.12}, align_reproject_fits {recorded:?}"
        );
        for (what, factor) in [("planned", planned.unwrap_or(1.0)), ("applied", applied)] {
            assert!(
                (factor - expected).abs() <= LIGHTSTACK_PRINTED_FACTOR,
                "{filter}: AstroBurst {what} the area factor x{factor}, align_reproject_fits printed x{expected}"
            );
        }
    }
    let gaussian = truth["planes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plane| plane["psf"]["model"] == "gaussian")
        .unwrap();
    let scale = gaussian["pixel_scale_arcsec"].as_f64().unwrap();
    let sigma_px = gaussian["psf"]["sigma_arcsec"].as_f64().unwrap() / scale;
    let subsampling = truth["subsampling"].as_f64().unwrap();
    let pixel_variance = (subsampling * subsampling - 1.0) / (12.0 * subsampling * subsampling);
    let expected =
        2.0 * (2.0 * std::f64::consts::LN_2).sqrt() * (sigma_px * sigma_px + pixel_variance).sqrt();
    let fwhm = oracle_calls(&manifest, "psfs")
        .into_iter()
        .find(|call| {
            call["function"] == "lightstack.psf.measure_psf_fwhm"
                && call["metrics"]["filter"] == gaussian["filter"]
        })
        .and_then(|call| call["metrics"]["fwhm_px"].as_f64())
        .expect("measure_psf_fwhm of the Gaussian PSF");
    eprintln!(
        "{}: measure_psf_fwhm {fwhm:.5} px against the pixelated Gaussian {expected:.5} px ({:+.3}%)",
        gaussian["filter"].as_str().unwrap(),
        100.0 * (fwhm / expected - 1.0)
    );
    assert!(
        relative_difference(fwhm, expected) < 0.005,
        "measure_psf_fwhm gave {fwhm} px, the pixelated Gaussian has {expected} px"
    );
}

#[test]
#[ignore = "needs ASTROBURST_ORACLE_DIR: a folder holding scripts/oracle/lightstack_oracle.py --out folders; checks only manifest sha256, versions and call records, not AstroBurst output"]
fn oracle_real_data_manifests_match_their_files_and_versions() {
    let root = std::env::var_os(ORACLE_DIR_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!(
                "{ORACLE_DIR_VAR} is unset: produce one folder per recipe with docs/notCommit/oracle/.venv313/Scripts/python.exe -I scripts/oracle/lightstack_oracle.py --recipe <recipe.json> --data <folder the recipe paths are relative to> --out <folder>, then point {ORACLE_DIR_VAR} at their parent; this test checks only the manifests (sha256 of every file, library versions, call records), not AstroBurst's output"
            )
        });
    let manifests = manifests_under(&root);
    assert!(
        !manifests.is_empty(),
        "no {MANIFEST} under {}",
        root.display()
    );
    let errors: Vec<String> = manifests
        .iter()
        .flat_map(|path| {
            let mut errors = manifest_errors(path);
            errors.extend(
                oracle_manifest_errors(&read_json(path))
                    .into_iter()
                    .map(|error| format!("{}: {error}", path.display())),
            );
            errors
        })
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

fn encircled_energy(psf: &Array2<f64>, radius_px: f64) -> f64 {
    let (rows, cols) = psf.dim();
    let (cy, cx) = ((rows as f64 - 1.0) / 2.0, (cols as f64 - 1.0) / 2.0);
    psf.indexed_iter()
        .filter(|((r, c), _)| (*c as f64 - cx).hypot(*r as f64 - cy) <= radius_px)
        .map(|(_, value)| *value)
        .sum()
}

#[test]
fn d1_psfs_resampled_to_the_cube_scale_match_the_truth_psfs() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let cube_scale = truth["cube_pixel_scale_arcsec"].as_f64().unwrap();
    let mut checked = 0;
    for plane in truth["planes"].as_array().unwrap() {
        let filter = plane["filter"].as_str().unwrap();
        let files = plane["psf"]["files"].as_array().unwrap();
        let at = |scale: f64| {
            files
                .iter()
                .find(|file| file["pixel_scale_arcsec"].as_f64() == Some(scale))
                .map(|file| d1_dir().join(file["path"].as_str().unwrap()))
        };
        let (Some(fine), Some(coarse)) = (at(0.02), at(cube_scale)) else {
            continue;
        };
        let source = load_psf(&fine, None).unwrap();
        let reference = load_psf(&coarse, None).unwrap();
        let options = |method| PsfPrepareOptions {
            target_pixel_scale_arcsec: Some(cube_scale),
            target_shape: Some(reference.data.dim()),
            method,
            recentre: true,
        };
        let area = prepare_psf(&source, &options(PsfResampleMethod::Area)).unwrap();
        let lightstack = prepare_psf(&source, &options(PsfResampleMethod::Lightstack)).unwrap();
        for (prepared, method) in [(&area, "area"), (&lightstack, "lightstack")] {
            assert_eq!(
                prepared.data.dim(),
                reference.data.dim(),
                "{filter} {method}"
            );
            assert!(
                (prepared.data.sum() - 1.0).abs() < 1e-12,
                "{filter} {method}: sum {}",
                prepared.data.sum()
            );
            assert!(
                prepared.recentred_by_px.is_none(),
                "{filter} {method}: the D1 PSFs are centred"
            );
        }
        let fwhm = |psf: &Array2<f64>| fit_gaussian(psf).unwrap().fwhm_px * cube_scale;
        let (truth_fwhm, area_fwhm, lightstack_fwhm) = (
            fwhm(&reference.data),
            fwhm(&area.data),
            fwhm(&lightstack.data),
        );
        let peak = reference.data.iter().fold(0.0f64, |acc, &v| acc.max(v));
        let worst = |psf: &Array2<f64>| {
            psf.iter()
                .zip(reference.data.iter())
                .fold(0.0f64, |acc, (a, b)| acc.max((a - b).abs()))
                / peak
        };
        eprintln!(
            "{filter}: Gaussian FWHM of the truth {truth_fwhm:.5}\", area {area_fwhm:.5}\" ({:+.3}%), lightstack {lightstack_fwhm:.5}\" ({:+.3}%); max pixel difference over the peak: area {:.2e}, lightstack {:.2e}",
            100.0 * (area_fwhm / truth_fwhm - 1.0),
            100.0 * (lightstack_fwhm / truth_fwhm - 1.0),
            worst(&area.data),
            worst(&lightstack.data)
        );
        assert!(
            relative_difference(area_fwhm, truth_fwhm) < 0.02,
            "{filter}: area-resampled FWHM {area_fwhm} against the truth {truth_fwhm} (S-class 2 percent)"
        );
        let model_fwhm_px = plane["psf"]["fwhm_arcsec"].as_f64().unwrap() / cube_scale;
        for factor in [1.0, 1.5, 2.0, 3.0, 4.0] {
            let radius = factor * model_fwhm_px;
            let (expected, got) = (
                encircled_energy(&reference.data, radius),
                encircled_energy(&area.data, radius),
            );
            assert!(
                (got - expected).abs() <= 0.01,
                "{filter}: encircled energy at {factor} FWHM {got} against the truth {expected}"
            );
        }
        assert!(
            lightstack_fwhm < area_fwhm,
            "{filter}: LIGHTSTACK's spline samples skip the coarse pixel integration, so its FWHM {lightstack_fwhm} should be below {area_fwhm}"
        );
        checked += 1;
    }
    assert_eq!(checked, 2, "F150W and F200W have 20 mas PSFs");
}

fn physical_box_stars(region_text: &str) -> Vec<PsfStar> {
    let parsed = parse_reg_with_physical(region_text, None, &PhysicalMap::identity()).unwrap();
    stars_from_regions(&parsed.regions)
}

#[test]
fn d1_empirical_psf_from_a_star_field_matches_the_truth_psf() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let planes = truth["planes"].as_array().unwrap();
    let index = planes
        .iter()
        .position(|plane| plane["filter"] == "F444W")
        .unwrap();
    let field = d1::star_field(index);
    let image = Array2::from_shape_vec((field.size, field.size), field.sci.clone()).unwrap();
    let boxes: String = field
        .stars
        .iter()
        .map(|star| {
            format!(
                "box({:.4},{:.4},9,9,0) # text={{{}}}\n",
                star.position[0] + 1.0,
                star.position[1] + 1.0,
                star.name
            )
        })
        .collect();
    let stars = physical_box_stars(&format!(
        "# Region file format: DS9 version 4.1\nglobal color=green\nphysical\n{boxes}"
    ));
    let options = EmpiricalPsfOptions {
        cutout_size: 31,
        background_border: 5,
        normalization_radius: 0.48,
        normalization_radius_unit: RadiusUnit::Arcsec,
        psf_size: field.truth_side,
        ..EmpiricalPsfOptions::default()
    };
    let built = build_empirical_psf(&image, &stars, Some(field.pixel_scale), &options).unwrap();
    assert!(
        (built.normalization_radius_px - 12.0).abs() < 1e-9,
        "0.48 arcsec at 0.04 arcsec per pixel is {} px",
        built.normalization_radius_px
    );
    assert_eq!(built.stars.len(), field.stars.len());
    for (star, report) in field.stars.iter().zip(&built.stars) {
        assert_eq!(report.name, star.name);
        if matches!(star.name, "f5" | "edge") {
            let reason = report.rejected.as_deref().unwrap_or_else(|| {
                panic!(
                    "{}: a NaN hole or the image edge lies inside its aperture",
                    star.name
                )
            });
            assert!(
                reason.contains("normalization aperture"),
                "{}: {reason}",
                star.name
            );
            continue;
        }
        assert!(report.used, "{}: {:?}", star.name, report.rejected);
        let offset = (report.centre_x.unwrap() - star.position[0])
            .hypot(report.centre_y.unwrap() - star.position[1]);
        let noise_over_background = (report.background_std.unwrap() / field.sigma_jy - 1.0).abs();
        eprintln!(
            "{}: {:?} centre off the truth by {offset:.4} px ({:+.4}, {:+.4}), border std over the noise sigma {:.3}",
            star.name,
            report.centroid.unwrap(),
            report.centre_x.unwrap() - star.position[0],
            report.centre_y.unwrap() - star.position[1],
            report.background_std.unwrap() / field.sigma_jy
        );
        assert!(offset < 0.1, "{}: centre off by {offset} px", star.name);
        assert!(
            noise_over_background < 0.25,
            "{}: border std {} against the noise {}",
            star.name,
            report.background_std.unwrap(),
            field.sigma_jy
        );
    }
    assert_eq!(built.stars_used(), 8);
    let truth_psf = Array2::from_shape_vec(
        (field.truth_side, field.truth_side),
        field.truth_psf.clone(),
    )
    .unwrap();
    assert_eq!(built.psf.dim(), truth_psf.dim());
    assert!((built.psf.sum() - 1.0).abs() < 1e-12);
    let worst = worst_relative("F444W star field", &built.psf, &truth_psf);
    let (ours_fwhm, truth_fwhm) = (
        fit_gaussian(&built.psf).unwrap().fwhm_px,
        fit_gaussian(&truth_psf).unwrap().fwhm_px,
    );
    eprintln!(
        "F444W star field: max |empirical - truth| {worst:.2e} of the peak; Gaussian FWHM {ours_fwhm:.4} px against {truth_fwhm:.4} px ({:+.3}%)",
        100.0 * (ours_fwhm / truth_fwhm - 1.0)
    );
    assert!(
        worst < 0.01,
        "residual {worst:e} of the peak (S-class 1 percent)"
    );
    assert!(
        relative_difference(ours_fwhm, truth_fwhm) < 0.01,
        "FWHM {ours_fwhm} px against the truth {truth_fwhm} px"
    );
    let model_fwhm_px = planes[index]["psf"]["fwhm_arcsec"].as_f64().unwrap() / field.pixel_scale;
    for factor in [1.0, 1.5, 2.0, 2.5] {
        let radius = factor * model_fwhm_px;
        let (expected, got) = (
            encircled_energy(&truth_psf, radius),
            encircled_energy(&built.psf, radius),
        );
        eprintln!("EE at {factor} FWHM: {got:.5} against {expected:.5}");
        assert!(
            (got - expected).abs() <= 0.005,
            "encircled energy at {factor} FWHM {got} against the truth {expected}"
        );
    }
}

fn star_fwhm_px(image: &Array2<f64>, x: f64, y: f64) -> f64 {
    let (cx, cy) = (x.round() as usize, y.round() as usize);
    let cutout = image
        .slice(ndarray::s![cy - 5..=cy + 5, cx - 5..=cx + 5])
        .to_owned();
    fit_gaussian(&cutout).unwrap().fwhm_px
}

#[test]
fn d1_gaussian_kernel_brings_f277w_stars_to_the_target_fwhm_and_keeps_their_flux() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let planes = truth["planes"].as_array().unwrap();
    let plane = planes
        .iter()
        .find(|plane| plane["psf"]["model"] == "gaussian")
        .unwrap();
    let target = planes
        .iter()
        .find(|plane| plane["filter"] == truth["reference_filter"])
        .unwrap();
    let scale = plane["pixel_scale_arcsec"].as_f64().unwrap();
    let size = plane["shape"][1].as_u64().unwrap() as usize;
    let (_, sci) = plane_data(&d1_dir().join(plane["file"].as_str().unwrap()), SCI);
    let image = Array2::from_shape_vec((size, size), sci)
        .unwrap()
        .mapv(f64::from);
    let sigma_native = plane["psf"]["sigma_arcsec"].as_f64().unwrap() / scale;
    let target_fwhm_px = target["psf"]["fwhm_arcsec"].as_f64().unwrap() / scale;
    let sigma_kernel =
        ((target_fwhm_px / GAUSSIAN_SIGMA_TO_FWHM).powi(2) - sigma_native.powi(2)).sqrt();
    let kernel = Array2::from_shape_fn((21, 21), |(r, c)| {
        let (dy, dx) = (r as f64 - 10.0, c as f64 - 10.0);
        (-(dx * dx + dy * dy) / (2.0 * sigma_kernel * sigma_kernel)).exp()
    });
    let kernel = &kernel / kernel.sum();
    let result = convolve_fft(
        &image,
        &kernel,
        &ConvolveOptions::lightstack(),
        &mut FftPlanCache::new(),
    )
    .unwrap();
    let report = &result.report;
    assert!(report.nan_pixels > 0);
    assert_eq!(
        report.filled_nan_pixels, report.nan_pixels,
        "LIGHTSTACK's options interpolate every NaN"
    );
    assert!(report
        .warnings
        .iter()
        .all(|warning| !warning.contains("negative lobes")));
    let matched = result.image.mapv(|v| v as f32);
    let radius = 2.0 * target_fwhm_px;
    let kernel_fwhm = GAUSSIAN_SIGMA_TO_FWHM * sigma_kernel;
    for star in plane["stars"].as_array().unwrap() {
        let id = star["id"].as_str().unwrap();
        let (x, y) = (star["x"].as_f64().unwrap(), star["y"].as_f64().unwrap());
        let (native, after) = (
            star_fwhm_px(&image, x, y),
            star_fwhm_px(&result.image, x, y),
        );
        let expected = native.hypot(kernel_fwhm);
        let flux = star["flux_jy"].as_f64().unwrap();
        let aperture = aperture_sum(&matched, x, y, radius);
        eprintln!(
            "F277W {id}: FWHM {native:.4} px -> {after:.4} px (expected {expected:.4}, {:+.3}%), aperture {aperture:e} Jy against {flux:e} Jy ({:+.3}%)",
            100.0 * (after / expected - 1.0),
            100.0 * (aperture / flux - 1.0)
        );
        assert!(
            relative_difference(after, expected) < 0.02,
            "{id}: matched FWHM {after} px against {expected} px"
        );
        assert!(
            relative_difference(aperture, flux) < 0.01,
            "{id}: matched aperture flux {aperture} Jy against the truth {flux} Jy"
        );
    }
}

struct KernelPin {
    filter: &'static str,
    residual_max: f64,
    residual_rms: f64,
    encircled_energy_difference: f64,
    source_fwhm_px: f64,
    matched_fwhm_px: f64,
    negative_fraction: f64,
    warnings: &'static [&'static str],
}

const D1_TARGET_FWHM_PX: f64 = 3.470962718563258;
const D1_TUTORIAL_KERNEL_PINS: [KernelPin; 5] = [
    KernelPin {
        filter: "F814W",
        residual_max: 0.163247039656284,
        residual_rms: 0.009009066057347763,
        encircled_energy_difference: 0.019421501874558378,
        source_fwhm_px: 2.7970197514109083,
        matched_fwhm_px: 3.842143338919234,
        negative_fraction: 0.38095238095238093,
        warnings: &[
            "the source PSF convolved with the kernel differs from the target by up to 16.32 percent of its peak (RMS 0.901 percent)",
            "the matched FWHM 3.842 px differs from the target FWHM 3.471 px by +10.69 percent",
            "the encircled energy of the matched PSF differs from the target by up to 0.0194 beyond one FWHM",
        ],
    },
    KernelPin {
        filter: "F150W",
        residual_max: 0.025797861771782095,
        residual_rms: 0.0014347199819372263,
        encircled_energy_difference: 0.008391057133931734,
        source_fwhm_px: 1.2138023524833936,
        matched_fwhm_px: 3.5115876756385194,
        negative_fraction: 0.29024943310657597,
        warnings: &[
            "the source PSF convolved with the kernel differs from the target by up to 2.58 percent of its peak (RMS 0.143 percent)",
        ],
    },
    KernelPin {
        filter: "F200W",
        residual_max: 0.0399177186246489,
        residual_rms: 0.002185678430596558,
        encircled_energy_difference: 0.011247293097084499,
        source_fwhm_px: 1.614703751906119,
        matched_fwhm_px: 3.537941611811592,
        negative_fraction: 0.34467120181405897,
        warnings: &[
            "the source PSF convolved with the kernel differs from the target by up to 3.99 percent of its peak (RMS 0.219 percent)",
            "the encircled energy of the matched PSF differs from the target by up to 0.0112 beyond one FWHM",
        ],
    },
    KernelPin {
        filter: "F277W",
        residual_max: 0.07350806505105269,
        residual_rms: 0.003994899370860249,
        encircled_energy_difference: 0.008551804269569918,
        source_fwhm_px: 2.3970607481600745,
        matched_fwhm_px: 3.6255440067282776,
        negative_fraction: 0.32653061224489793,
        warnings: &[
            "the source PSF convolved with the kernel differs from the target by up to 7.35 percent of its peak (RMS 0.399 percent)",
            "the matched FWHM 3.626 px differs from the target FWHM 3.471 px by +4.45 percent",
        ],
    },
    KernelPin {
        filter: "F356W",
        residual_max: 0.2863592544641417,
        residual_rms: 0.016159043921685663,
        encircled_energy_difference: 0.04547987181558677,
        source_fwhm_px: 3.4708684032980845,
        matched_fwhm_px: 4.209660401560967,
        negative_fraction: 0.36281179138321995,
        warnings: &[
            "the source PSF convolved with the kernel differs from the target by up to 28.64 percent of its peak (RMS 1.616 percent)",
            "the matched FWHM 4.210 px differs from the target FWHM 3.471 px by +21.28 percent",
            "the encircled energy of the matched PSF differs from the target by up to 0.0455 beyond one FWHM",
        ],
    },
];

fn d1_psf_at(plane: &Value, scale: f64) -> Array2<f64> {
    let file = plane["psf"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["pixel_scale_arcsec"].as_f64() == Some(scale))
        .unwrap_or_else(|| panic!("{}: no PSF at {scale} arcsec", plane["filter"]));
    load_psf(&d1_dir().join(file["path"].as_str().unwrap()), None)
        .unwrap()
        .data
}

fn tutorial_kernel_options() -> KernelOptions {
    KernelOptions {
        method: KernelMethod::Wiener {
            eps: 0.05,
            eps_mode: EpsMode::Absolute,
            shape: KernelShape::default(),
        },
        auto: false,
    }
}

fn automatic_kernel_options() -> KernelOptions {
    KernelOptions {
        auto: true,
        ..KernelOptions::default()
    }
}

#[test]
fn d1_automatic_kernels_match_every_psf_to_f444w_within_one_percent() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let scale = truth["cube_pixel_scale_arcsec"].as_f64().unwrap();
    let planes = truth["planes"].as_array().unwrap();
    let reference = planes
        .iter()
        .find(|plane| plane["filter"] == truth["reference_filter"])
        .unwrap();
    let target = d1_psf_at(reference, scale);
    let window = KernelOptions {
        method: KernelMethod::Window {
            window: TaperWindow::CosineBell { alpha: 0.7 },
            regularization: 1e-4,
        },
        auto: false,
    };
    let mut checked = 0;
    for plane in planes
        .iter()
        .filter(|plane| plane["filter"] != truth["reference_filter"])
    {
        let filter = plane["filter"].as_str().unwrap();
        let source = d1_psf_at(plane, scale);
        let tuned = build_kernel(&source, &target, &automatic_kernel_options()).unwrap();
        let lightstack = build_kernel(&source, &target, &tutorial_kernel_options()).unwrap();
        let photutils = build_kernel(&source, &target, &window).unwrap();
        let d = &tuned.diagnostics;
        let summary = |kernel: &astroburst_lib::core::cubeprep::kernel::MatchingKernel| {
            let d = &kernel.diagnostics;
            format!(
                "residual max {:.2e} rms {:.2e}, EE {:.4}, FWHM {:.3} px, sum {:.6}, {:.0}% negative",
                d.residual_max,
                d.residual_rms,
                d.encircled_energy_difference,
                d.matched_fwhm_px.unwrap(),
                d.sum,
                100.0 * d.negative_fraction
            )
        };
        eprintln!(
            "{filter} to F444W (Gaussian FWHM {:.5} px to {:.5} px): automatic eps {:e}: {}; LIGHTSTACK eps 0.05: {}; CosineBell(0.7): {}",
            d.source_fwhm_px.unwrap(),
            d.target_fwhm_px.unwrap(),
            tuned.auto.as_ref().unwrap().chosen,
            summary(&tuned),
            summary(&lightstack),
            summary(&photutils)
        );
        assert!(
            d.residual_max < 0.01,
            "{filter}: matched PSF residual {} of the peak (S-class 1 percent)",
            d.residual_max
        );
        assert!(
            d.encircled_energy_difference <= 0.01,
            "{filter}: encircled energy difference {} (S-class 0.01)",
            d.encircled_energy_difference
        );
        assert!(
            relative_difference(d.matched_fwhm_px.unwrap(), d.target_fwhm_px.unwrap()) < 0.02,
            "{filter}: {d:?}"
        );
        assert!(d.warnings.is_empty(), "{filter}: {:?}", d.warnings);
        assert_eq!(diagnose_kernel(&source, &target, &tuned.data).unwrap(), *d);
        checked += 1;
    }
    assert_eq!(checked, 5);
}

fn d1_cube_on_the_reference_grid(truth: &Value, psf_of: impl Fn(usize) -> usize) -> (Cube, usize) {
    let planes = truth["planes"].as_array().unwrap();
    let reference = planes
        .iter()
        .position(|plane| plane["filter"] == truth["reference_filter"])
        .unwrap();
    let side = planes[reference]["shape"][1].as_u64().unwrap() as usize;
    let grids = d1_planes();
    let registry = FilterRegistry::builtin();
    let grid = &grids[reference];
    let filters = grids
        .iter()
        .map(|plane| {
            let entry = registry
                .identify(Some(&plane.header), &plane.filter, None)
                .unwrap()
                .entry;
            CubePlane::from_entry(&entry, &grid.header, &grid.wcs)
        })
        .collect();
    let images = (0..planes.len())
        .map(|index| {
            let model = d1::render_scene(index, reference, psf_of(index)).model_jy;
            Array2::from_shape_vec((side, side), model)
                .unwrap()
                .mapv(|v| v as f32)
        })
        .collect();
    let cube = Cube {
        header: grid.header.clone(),
        planes: images,
        err: None,
        dq: None,
        filters,
    };
    (cube, reference)
}

fn d1_kernels(truth: &Value, options: &KernelOptions) -> Vec<PlaneKernel> {
    let scale = truth["cube_pixel_scale_arcsec"].as_f64().unwrap();
    let planes = truth["planes"].as_array().unwrap();
    let reference = truth["reference_filter"].as_str().unwrap();
    let target_plane = planes
        .iter()
        .find(|plane| plane["filter"] == reference)
        .unwrap();
    let target = d1_psf_at(target_plane, scale);
    planes
        .iter()
        .filter(|plane| plane["filter"] != reference)
        .map(|plane| {
            let filter = plane["filter"].as_str().unwrap();
            let kernel = build_kernel(&d1_psf_at(plane, scale), &target, options).unwrap();
            PlaneKernel {
                filter: filter.to_string(),
                label: kernel_file_name(filter, reference),
                data: kernel.data,
                pixel_scale_arcsec: Some(scale),
            }
        })
        .collect()
}

#[test]
fn d1_psf_matched_stars_reach_the_f444w_fwhm_and_keep_their_colours() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let scale = truth["cube_pixel_scale_arcsec"].as_f64().unwrap();
    let planes = truth["planes"].as_array().unwrap();
    let (cube, reference) = d1_cube_on_the_reference_grid(&truth, |index| index);
    let (ideal, _) = d1_cube_on_the_reference_grid(&truth, |_| reference);
    let reference_filter = truth["reference_filter"].as_str().unwrap();
    let options = MatchOptions {
        reference: reference_filter.to_string(),
        mode: MatchMode::Astroburst,
        allow_unmatched: false,
    };
    let automatic = match_cube(
        &cube,
        &d1_kernels(&truth, &automatic_kernel_options()),
        &options,
        &[],
    )
    .unwrap();
    let compat = MatchOptions {
        mode: MatchMode::Lightstack,
        ..options.clone()
    };
    let tutorial = match_cube(
        &cube,
        &d1_kernels(&truth, &tutorial_kernel_options()),
        &compat,
        &[],
    )
    .unwrap();
    assert!(same_bits(
        &automatic.cube.planes[reference],
        &cube.planes[reference]
    ));
    let radius = STAR_APERTURE_ARCSEC / scale;
    let reference_stars = planes[reference]["stars"].as_array().unwrap();
    let f64_image = |image: &Array2<f32>| image.mapv(f64::from);
    let (mut worst_fwhm, mut worst_colour) = (0.0f64, 0.0f64);
    let (mut tutorial_fwhm, mut tutorial_colour) = (0.0f64, 0.0f64);
    for (index, plane) in planes.iter().enumerate().filter(|(i, _)| *i != reference) {
        let filter = plane["filter"].as_str().unwrap();
        for star in reference_stars {
            let id = star["id"].as_str().unwrap();
            let (x, y) = (star["x"].as_f64().unwrap(), star["y"].as_f64().unwrap());
            let target_fwhm = star_fwhm_px(&f64_image(&cube.planes[reference]), x, y);
            let native = star_fwhm_px(&f64_image(&cube.planes[index]), x, y);
            let matched = star_fwhm_px(&f64_image(&automatic.cube.planes[index]), x, y);
            let lightstack = star_fwhm_px(&f64_image(&tutorial.cube.planes[index]), x, y);
            let colour = |images: &[Array2<f32>]| {
                aperture_sum(&images[index], x, y, radius)
                    / aperture_sum(&images[reference], x, y, radius)
            };
            let flux = |plane: &Value| {
                plane["stars"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|other| other["id"] == star["id"])
                    .unwrap()["flux_jy"]
                    .as_f64()
                    .unwrap()
            };
            let sed_colour = flux(plane) / flux(&planes[reference]);
            let (expected, ours, theirs) = (
                colour(&ideal.planes),
                colour(&automatic.cube.planes),
                colour(&tutorial.cube.planes),
            );
            eprintln!(
                "{filter} {id}: FWHM {native:.3} px -> {matched:.3} px against {target_fwhm:.3} px ({:+.2}%), LIGHTSTACK eps 0.05 {lightstack:.3} px ({:+.2}%); aperture colour {ours:.5} against {expected:.5} at F444W resolution ({:+.3}%), LIGHTSTACK {theirs:.5} ({:+.3}%), star SED {sed_colour:.5}",
                100.0 * (matched / target_fwhm - 1.0),
                100.0 * (lightstack / target_fwhm - 1.0),
                100.0 * (ours / expected - 1.0),
                100.0 * (theirs / expected - 1.0)
            );
            worst_fwhm = worst_fwhm.max(relative_difference(matched, target_fwhm));
            worst_colour = worst_colour.max(relative_difference(ours, expected));
            tutorial_fwhm = tutorial_fwhm.max(relative_difference(lightstack, target_fwhm));
            tutorial_colour = tutorial_colour.max(relative_difference(theirs, expected));
            assert!(
                relative_difference(matched, target_fwhm) < 0.02,
                "{filter} {id}: matched FWHM {matched} px against {target_fwhm} px (S-class 2 percent)"
            );
            assert!(
                relative_difference(ours, expected) < 0.005,
                "{filter} {id}: colour {ours} against {expected} (S-class 0.5 percent)"
            );
        }
    }
    eprintln!(
        "worst over the stars: automatic kernels FWHM {:.2}% and colour {:.3}%; LIGHTSTACK eps 0.05 FWHM {:.2}% and colour {:.3}%",
        100.0 * worst_fwhm,
        100.0 * worst_colour,
        100.0 * tutorial_fwhm,
        100.0 * tutorial_colour
    );
}

#[test]
fn d1_aligned_cube_matched_keeps_star_colours_within_half_a_percent() {
    let truth = read_json(&d1_dir().join(MANIFEST))["truth"].clone();
    let scale = truth["cube_pixel_scale_arcsec"].as_f64().unwrap();
    let planes = truth["planes"].as_array().unwrap();
    let D1CubeInputs {
        inputs, reference, ..
    } = d1_cube_inputs();
    let reference_header = inputs[reference].header.clone();
    let cube = assemble_cube(
        inputs,
        reference,
        cube_base_header(&reference_header),
        CubeOptions {
            layout: CubeLayout::Lightstack,
            order: PlaneOrder::Wavelength,
        },
    )
    .unwrap()
    .cube;
    let (ideal, _) = d1_cube_on_the_reference_grid(&truth, |_| reference);
    let options = MatchOptions {
        reference: truth["reference_filter"].as_str().unwrap().to_string(),
        mode: MatchMode::Astroburst,
        allow_unmatched: false,
    };
    let matched = match_cube(
        &cube,
        &d1_kernels(&truth, &automatic_kernel_options()),
        &options,
        &[],
    )
    .unwrap();
    let radius = STAR_APERTURE_ARCSEC / scale;
    let f64_image = |image: &Array2<f32>| image.mapv(f64::from);
    for (index, plane) in planes.iter().enumerate().filter(|(i, _)| *i != reference) {
        let filter = plane["filter"].as_str().unwrap();
        for star in planes[reference]["stars"].as_array().unwrap() {
            let id = star["id"].as_str().unwrap();
            let (x, y) = (star["x"].as_f64().unwrap(), star["y"].as_f64().unwrap());
            let target_fwhm = star_fwhm_px(&f64_image(&cube.planes[reference]), x, y);
            let aligned = star_fwhm_px(&f64_image(&cube.planes[index]), x, y);
            let after = star_fwhm_px(&f64_image(&matched.cube.planes[index]), x, y);
            let colour = |images: &[Array2<f32>]| {
                aperture_sum(&images[index], x, y, radius)
                    / aperture_sum(&images[reference], x, y, radius)
            };
            let (ours, expected) = (colour(&matched.cube.planes), colour(&ideal.planes));
            eprintln!(
                "aligned {filter} {id}: FWHM {aligned:.3} px -> {after:.3} px against {target_fwhm:.3} px ({:+.2}%), aperture colour {ours:.5} against {expected:.5} ({:+.3}%)",
                100.0 * (after / target_fwhm - 1.0),
                100.0 * (ours / expected - 1.0)
            );
            assert!(
                relative_difference(ours, expected) < 0.005,
                "{filter} {id}: colour {ours} against {expected} (S-class 0.5 percent)"
            );
            assert!(
                after > 0.98 * target_fwhm,
                "{filter} {id}: matched FWHM {after} px below the reference {target_fwhm} px"
            );
        }
    }
}

fn oracle_entry_path(entry: &Value) -> PathBuf {
    oracle_entry_path_in(entry, &d1_dir(), &oracle_d1_dir())
}

fn oracle_entry_path_in(entry: &Value, data: &Path, out: &Path) -> PathBuf {
    let root = if entry["root"] == "data" { data } else { out };
    root.join(entry["path"].as_str().unwrap())
}

fn f64_planes(path: &Path) -> (HduHeader, Vec<Array2<f64>>) {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let label = path.display().to_string();
    let hdus = image_hdus(&bytes, &label).unwrap();
    let (_, parsed) = hdus.first().unwrap_or_else(|| panic!("{label}: no image"));
    (
        parsed.header.clone(),
        decode_planes_f64(&bytes, parsed, &label).unwrap(),
    )
}

fn worst_relative(label: &str, got: &Array2<f64>, expected: &Array2<f64>) -> f64 {
    assert_eq!(
        got.dim(),
        expected.dim(),
        "{label}: AstroBurst's shape differs from the reference"
    );
    let scale = expected
        .iter()
        .filter(|v| v.is_finite())
        .fold(0.0f64, |acc, v| acc.max(v.abs()));
    assert!(
        scale > 0.0,
        "{label}: the reference has no finite non-zero value"
    );
    let mut worst = 0.0f64;
    for (((row, col), g), e) in got.indexed_iter().zip(expected.iter()) {
        if g.is_finite() && e.is_finite() {
            worst = worst.max((g - e).abs());
        } else {
            assert!(
                (g.is_nan() && e.is_nan()) || g == e,
                "{label} ({col}, {row}): AstroBurst {g} where the reference has {e}"
            );
        }
    }
    worst / scale
}

#[test]
fn oracle_d1_psf_resampling_matches_resample_psf_to_pixel_scale() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let calls = oracle_calls(&manifest, "psfs");
    let resampled: Vec<&Value> = calls
        .iter()
        .copied()
        .filter(|call| call["function"] == "lightstack.psf.resample_psf_to_pixel_scale")
        .collect();
    assert_eq!(
        resampled.len(),
        2,
        "F150W and F200W are resampled from 20 mas"
    );
    for call in resampled {
        let filter = call["metrics"]["filter"].as_str().unwrap();
        let arguments = &call["arguments"];
        let source_scale = arguments["source_pixel_scale"].as_f64().unwrap();
        let target_scale = arguments["target_pixel_scale"].as_f64().unwrap();
        let shape = (
            arguments["output_shape"][0].as_u64().unwrap() as usize,
            arguments["output_shape"][1].as_u64().unwrap() as usize,
        );
        let source = load_psf(&oracle_entry_path(&call["inputs"][0]), None).unwrap();
        let golden = f64_planes(&oracle_entry_path(&call["outputs"][0]))
            .1
            .remove(0);
        let padding = lightstack_pad(&source.data, source_scale, target_scale, shape).unwrap();
        let ours =
            lightstack_resample(&padding.padded, source_scale, target_scale, Some(shape)).unwrap();
        let error = worst_relative(filter, &ours, &golden);
        eprintln!(
            "{filter}: max difference against resample_psf_to_pixel_scale {error:.2e} of the peak"
        );
        assert!(error <= 1e-9, "{filter}: {error:e} of the peak (N-class)");
        let lightstack_fwhm = calls
            .iter()
            .find(|other| {
                other["function"] == "lightstack.psf.measure_psf_fwhm"
                    && other["metrics"]["filter"] == filter
            })
            .and_then(|other| other["metrics"]["fwhm_px"].as_f64())
            .expect("measure_psf_fwhm of the resampled PSF");
        let ours_fwhm = fit_gaussian(&ours).unwrap().fwhm_px;
        assert!(
            relative_difference(ours_fwhm, lightstack_fwhm) < 0.005,
            "{filter}: Gaussian FWHM {ours_fwhm} px against measure_psf_fwhm {lightstack_fwhm} px"
        );
    }
}

fn same_json(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn first_image(path: &Path) -> (HduHeader, Array2<f32>) {
    let file = open(path);
    (0..8)
        .find_map(|hdu| extract_image_mmap_by_index(&file, hdu).ok())
        .map(|loaded| (loaded.header, loaded.image))
        .unwrap_or_else(|| panic!("{}: no image HDU", path.display()))
}

#[test]
fn oracle_d1_empirical_psf_matches_build_empirical_psf() {
    let manifest = read_json(&oracle_d1_stars_dir().join(MANIFEST));
    let registry = FilterRegistry::builtin();
    let path = |entry: &Value| oracle_entry_path_in(entry, &d1_stars_dir(), &oracle_d1_stars_dir());
    let recipe = &oracle_step(&manifest, "psfs")["parameters"];
    let defaults = serde_json::to_value(EmpiricalPsfOptions::default()).unwrap();
    let calls: Vec<&Value> = oracle_calls(&manifest, "psfs")
        .into_iter()
        .filter(|call| call["function"] == "lightstack.psf.build_empirical_psf")
        .collect();
    assert!(
        !calls.is_empty(),
        "the oracle recipe builds at least one empirical PSF"
    );
    for call in calls {
        let filter = call["metrics"]["filter"].as_str().unwrap();
        let arguments = &call["arguments"];
        assert_eq!(
            arguments["oversampling_order"].as_u64(),
            Some(3),
            "{filter}: only the cubic oversampling of the default recipe is ported"
        );
        for (key, default) in defaults.as_object().unwrap() {
            if key == "stack" {
                continue;
            }
            let recorded = arguments.get(key).unwrap_or_else(|| {
                panic!("{filter}: the effective arguments of build_empirical_psf lack {key}")
            });
            let (expected, source) = match recipe.get(key) {
                Some(set) => (set, "the recipe sets"),
                None => (default, "AstroBurst's default is"),
            };
            assert!(
                same_json(recorded, expected),
                "{filter} {key}: build_empirical_psf ran with {recorded}, {source} {expected}"
            );
        }
        let options: EmpiricalPsfOptions = serde_json::from_value(arguments.clone()).unwrap();
        assert_eq!(
            options.stack,
            PsfStack::Median,
            "{filter}: build_empirical_psf median-stacks the cutouts"
        );
        let inputs: Vec<PathBuf> = call["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(path)
            .collect();
        let dataset = arguments["dataset"].as_str().unwrap();
        let named_for_the_filter: Vec<&PathBuf> = inputs
            .iter()
            .filter(|input| {
                input.extension().is_some_and(|ext| ext == "fits")
                    && registry
                        .identify_file_name(input.to_str().unwrap(), dataset)
                        .is_ok_and(|found| found.entry.filter == filter)
            })
            .collect();
        let [fits] = named_for_the_filter[..] else {
            panic!("{filter}: get_psf_input_files pairs one FITS named for the filter, the inputs hold {named_for_the_filter:?}");
        };
        let region_name = format!("stars_{}.reg", filter.to_lowercase());
        let region = inputs
            .iter()
            .find(|input| {
                input
                    .file_name()
                    .is_some_and(|name| name == region_name.as_str())
            })
            .unwrap_or_else(|| panic!("{filter}: no {region_name} among the inputs"));
        let (header, image) = first_image(fits);
        let region_text = std::fs::read_to_string(region).unwrap();
        let stars: Vec<PsfStar> = physical_box_stars(&region_text)
            .into_iter()
            .map(|star| PsfStar {
                x: star.x + 1.0,
                y: star.y + 1.0,
                ..star
            })
            .collect();
        let built = build_empirical_psf(&image, &stars, wcs_scale(&header), &options).unwrap();
        assert_eq!(
            built.stars_used(),
            stars.len(),
            "{filter}: LIGHTSTACK uses every listed star"
        );
        let metrics = &call["metrics"];
        assert_eq!(
            metrics["nstars"].as_u64(),
            Some(stars.len() as u64),
            "{filter}: stars in build_empirical_psf"
        );
        let half = options.cutout_size as f64 / 2.0;
        let mut largest = [0.0f64; 4];
        for report in &built.stars {
            let name = report.name.as_str();
            let centre = &metrics["centers"][name];
            assert_eq!(
                serde_json::to_value(report.centroid).unwrap(),
                centre["method"],
                "{filter} {name}: centroid method"
            );
            let offset = [
                report.centre_x.unwrap() - (report.x - half).ceil() - centre["x"].as_f64().unwrap(),
                report.centre_y.unwrap() - (report.y - half).ceil() - centre["y"].as_f64().unwrap(),
            ];
            let relative = |ours: Option<f64>, key: &str| {
                relative_difference(ours.unwrap(), metrics[key][name].as_f64().unwrap())
            };
            let differences = [
                offset[0].abs().max(offset[1].abs()),
                relative(report.background, "backgrounds"),
                relative(report.background_std, "background_stds"),
                relative(report.aperture_flux, "fluxes"),
            ];
            eprintln!(
                "{filter} {name}: centre {:+.2e} {:+.2e} px, relative background {:.2e}, background std {:.2e}, aperture flux {:.2e} against build_empirical_psf",
                offset[0], offset[1], differences[1], differences[2], differences[3]
            );
            for (quantity, difference) in [
                "centre (px)",
                "relative background",
                "relative background std",
                "relative aperture flux",
            ]
            .into_iter()
            .zip(differences)
            {
                assert!(
                    difference <= 1e-9,
                    "{filter} {name}: {quantity} differs from build_empirical_psf by {difference:e} (N class, 1e-9)"
                );
            }
            for (worst, difference) in largest.iter_mut().zip(differences) {
                *worst = worst.max(difference);
            }
        }
        let golden = f64_planes(&path(&call["outputs"][0])).1.remove(0);
        let error = worst_relative(filter, &built.psf, &golden);
        let (ours_fwhm, golden_fwhm) = (
            fit_gaussian(&built.psf).unwrap().fwhm_px,
            fit_gaussian(&golden).unwrap().fwhm_px,
        );
        let energies: Vec<(f64, f64, f64)> = [1.0, 1.5, 2.0, 3.0]
            .into_iter()
            .map(|factor| {
                let radius = factor * golden_fwhm;
                (
                    factor,
                    encircled_energy(&built.psf, radius),
                    encircled_energy(&golden, radius),
                )
            })
            .collect();
        let ee_difference = energies
            .iter()
            .map(|(_, got, expected)| (got - expected).abs())
            .fold(0.0f64, f64::max);
        eprintln!(
            "{filter}: {} stars; largest per-star difference: centre {:.2e} px, background {:.2e}, background std {:.2e}, aperture flux {:.2e}; PSF max difference {error:.2e} of the peak, Gaussian FWHM {ours_fwhm:.6} px against {golden_fwhm:.6} px ({:.2e} relative), EE at 1 to 3 FWHM within {ee_difference:.2e}",
            stars.len(),
            largest[0],
            largest[1],
            largest[2],
            largest[3],
            relative_difference(ours_fwhm, golden_fwhm)
        );
        assert!(error <= 1e-9, "{filter}: {error:e} of the peak (N class)");
        assert!(error < 1e-3, "{filter}: {error:e} of the peak (T12)");
        assert!(
            relative_difference(ours_fwhm, golden_fwhm) < 0.01,
            "{filter}: FWHM {ours_fwhm} px against {golden_fwhm} px"
        );
        for (factor, got, expected) in energies {
            assert!(
                (got - expected).abs() <= 0.005,
                "{filter}: encircled energy at {factor} FWHM {got} against {expected}"
            );
        }
        if let Some(entry) = call["outputs"].as_array().unwrap().get(1) {
            let golden = f64_planes(&path(entry)).1.remove(0);
            let oversampled = built
                .oversampled
                .as_ref()
                .expect("the oracle wrote an oversampled PSF");
            let error = worst_relative(&format!("{filter} oversampled"), oversampled, &golden);
            assert!(
                error <= 1e-9,
                "{filter}: oversampled PSF {error:e} of the peak (N class)"
            );
        }
    }
}

#[test]
fn oracle_d1_fft_convolution_matches_apply_kernel() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let calls = oracle_calls(&manifest, "match");
    assert_eq!(calls.len(), 1, "one psf_match_datacube call");
    let call = calls[0];
    let reference = call["arguments"]["ref_filter"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    let inputs = call["inputs"].as_array().unwrap();
    let (header, planes) = f64_planes(&oracle_entry_path(&inputs[0]));
    let (_, matched) = f64_planes(&oracle_entry_path(&call["outputs"][0]));
    assert_eq!(planes.len(), matched.len());
    let mut parity_plans = FftPlanCache::<f64>::new();
    let mut fast_plans = FftPlanCache::<f32>::new();
    for (index, (plane, expected)) in planes.iter().zip(&matched).enumerate() {
        let filter = header
            .get(&format!("FILTER{}", index + 1))
            .unwrap()
            .trim()
            .to_ascii_lowercase();
        if filter == reference {
            assert_eq!(plane.dim(), expected.dim(), "{filter}");
            assert!(
                plane
                    .iter()
                    .zip(expected.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{filter}: the reference plane is copied"
            );
            continue;
        }
        let kernel_name = format!("kernel_{filter}_to_{reference}.fits");
        let kernel_entry = inputs
            .iter()
            .find(|entry| entry["path"].as_str().unwrap().ends_with(&kernel_name))
            .unwrap_or_else(|| panic!("{kernel_name} is not among the inputs"));
        let kernel = f64_planes(&oracle_entry_path(kernel_entry)).1.remove(0);
        let options = ConvolveOptions::lightstack();
        let parity = convolve_fft(plane, &kernel, &options, &mut parity_plans).unwrap();
        let error = worst_relative(&format!("{filter} float64"), &parity.image, expected);
        let fast = convolve_fft(
            &plane.mapv(|v| v as f32),
            &kernel,
            &options,
            &mut fast_plans,
        )
        .unwrap();
        let fast_error = worst_relative(
            &format!("{filter} float32"),
            &fast.image.mapv(f64::from),
            expected,
        );
        eprintln!(
            "{filter}: max difference against apply_kernel {error:.2e} (float64), {fast_error:.2e} (float32) of the max abs value"
        );
        assert!(error <= 1e-9, "{filter}: {error:e} (N-class)");
        assert!(fast_error <= 2e-6, "{filter}: {fast_error:e} (F-class)");
    }
}

#[test]
fn oracle_d1_kernels_match_build_kernel() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let calls = oracle_calls(&manifest, "kernels");
    let built: Vec<&Value> = calls
        .iter()
        .copied()
        .filter(|call| call["function"] == "lightstack.psf.build_kernel")
        .collect();
    let saved: Vec<&Value> = calls
        .iter()
        .copied()
        .filter(|call| call["function"] == "lightstack.psf.save_kernel")
        .collect();
    assert_eq!(built.len(), 5, "one kernel per filter but the reference");
    assert_eq!(saved.len(), built.len());
    for (call, save) in built.iter().zip(&saved) {
        let filter = call["metrics"]["filter"].as_str().unwrap();
        let arguments = &call["arguments"];
        let shape = (
            arguments["shape"][0].as_u64().unwrap() as usize,
            arguments["shape"][1].as_u64().unwrap() as usize,
        );
        let eps = arguments["eps"].as_f64().unwrap();
        let source = f64_planes(&oracle_entry_path(&call["inputs"][0]))
            .1
            .remove(0);
        let target = f64_planes(&oracle_entry_path(&call["inputs"][1]))
            .1
            .remove(0);
        let output = &save["outputs"][0];
        assert!(output["path"]
            .as_str()
            .unwrap()
            .ends_with(&kernel_file_name(filter, "F444W")));
        let golden = f64_planes(&oracle_entry_path(output)).1.remove(0);
        let ours = wiener(&source, &target, shape, eps).unwrap();
        let error = worst_relative(filter, &ours, &golden);
        let diagnostics = diagnose_kernel(&source, &target, &ours).unwrap();
        eprintln!(
            "{filter}: max difference against build_kernel {error:.2e} of the peak; matched PSF residual {:.3} of the peak, EE difference {:.4}",
            diagnostics.residual_max, diagnostics.encircled_energy_difference
        );
        assert!(error <= 1e-9, "{filter}: {error:e} (N-class)");
        let metrics = &call["metrics"];
        assert!((metrics["sum"].as_f64().unwrap() - ours.sum()).abs() < 1e-12);
        assert_eq!(
            metrics["negative_fraction"].as_f64().unwrap(),
            diagnostics.negative_fraction
        );
        eprintln!("{filter}: {diagnostics:?}");
        let pin = D1_TUTORIAL_KERNEL_PINS
            .iter()
            .find(|pin| pin.filter == filter)
            .unwrap_or_else(|| panic!("{filter}: no pinned diagnostics"));
        let fwhm = |value: Option<f64>| value.unwrap_or_else(|| panic!("{filter}: no Gaussian fit"));
        for (name, got, expected, bound) in [
            ("residual_max", diagnostics.residual_max, pin.residual_max, 1e-9),
            ("residual_rms", diagnostics.residual_rms, pin.residual_rms, 1e-9),
            (
                "encircled_energy_difference",
                diagnostics.encircled_energy_difference,
                pin.encircled_energy_difference,
                1e-9,
            ),
            ("negative_fraction", diagnostics.negative_fraction, pin.negative_fraction, 1e-9),
            ("source_fwhm_px", fwhm(diagnostics.source_fwhm_px), pin.source_fwhm_px, 1e-7),
            ("target_fwhm_px", fwhm(diagnostics.target_fwhm_px), D1_TARGET_FWHM_PX, 1e-7),
            ("matched_fwhm_px", fwhm(diagnostics.matched_fwhm_px), pin.matched_fwhm_px, 1e-7),
        ] {
            assert!(
                relative_difference(got, expected) <= bound,
                "{filter}: {name} {got:?} moved from the pinned {expected:?} by more than {bound:e} relative"
            );
        }
        assert_eq!(
            diagnostics.warnings, pin.warnings,
            "{filter}: the kernel warnings users read changed"
        );
    }
}

#[test]
fn oracle_d1_psf_match_reproduces_psf_match_datacube() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let calls = oracle_calls(&manifest, "match");
    assert_eq!(calls.len(), 1, "one psf_match_datacube call");
    let call = calls[0];
    let reference = call["arguments"]["ref_filter"].as_str().unwrap();
    let inputs = call["inputs"].as_array().unwrap();
    let registry = FilterRegistry::builtin();
    let cube = oracle_cube(&inputs[0], &registry);
    let kernels: Vec<PlaneKernel> = cube
        .filters
        .iter()
        .filter_map(|plane| {
            let name = kernel_file_name(&plane.filter, reference);
            inputs
                .iter()
                .find(|entry| entry["path"].as_str().unwrap().ends_with(&name))
                .map(|entry| PlaneKernel {
                    filter: plane.filter.clone(),
                    label: name,
                    data: f64_planes(&oracle_entry_path(entry)).1.remove(0),
                    pixel_scale_arcsec: None,
                })
        })
        .collect();
    let options = MatchOptions {
        reference: reference.to_string(),
        mode: MatchMode::Lightstack,
        allow_unmatched: true,
    };
    let matched = match_cube(&cube, &kernels, &options, &[]).unwrap();
    let (header, expected) = f64_planes(&oracle_entry_path(&call["outputs"][0]));
    assert_eq!(expected.len(), matched.cube.planes.len());
    for (plane, (ours, theirs)) in cube
        .filters
        .iter()
        .zip(matched.cube.planes.iter().zip(&expected))
    {
        let error = worst_relative(&plane.filter, &ours.mapv(f64::from), theirs);
        eprintln!(
            "{}: max difference against psf_match_datacube {error:.2e} of the max abs value",
            plane.filter
        );
        assert!(error <= 2e-6, "{}: {error:e} (F-class)", plane.filter);
    }
    let unmatched: Vec<&str> = matched
        .planes
        .iter()
        .filter(|plane| plane.kernel.is_none() && !plane.filter.eq_ignore_ascii_case(reference))
        .map(|plane| plane.filter.as_str())
        .collect();
    let reported: Vec<&str> = call["metrics"]["unmatched"]
        .as_array()
        .unwrap()
        .iter()
        .map(|filter| filter.as_str().unwrap())
        .collect();
    assert_eq!(unmatched, reported);
    let history: Vec<String> = header
        .cards
        .iter()
        .filter(|(key, _)| key == "HISTORY")
        .map(|(_, value)| value.trim().to_string())
        .collect();
    assert!(
        history.contains(&lightstack_history(reference)),
        "{history:?}"
    );
}

fn oracle_step<'a>(manifest: &'a Value, step: &str) -> &'a Value {
    manifest["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["step"] == step)
        .unwrap_or_else(|| panic!("the manifest has no {step} step"))
}

#[test]
fn oracle_d1_astroburst_kernels_through_match_cube_reproduce_psf_match_datacube() {
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let options: KernelOptions =
        serde_json::from_value(oracle_step(&manifest, "kernels")["parameters"].clone()).unwrap();
    assert_eq!(options, tutorial_kernel_options());
    let kernel_scale = oracle_step(&manifest, "psfs")["parameters"]["pixel_scale_arcsec"].as_f64();
    let calls = oracle_calls(&manifest, "match");
    let [call] = calls[..] else {
        panic!(
            "one psf_match_datacube call, the manifest holds {}",
            calls.len()
        )
    };
    let reference = call["arguments"]["ref_filter"].as_str().unwrap();
    let kernels: Vec<PlaneKernel> = oracle_calls(&manifest, "kernels")
        .into_iter()
        .filter(|call| call["function"] == "lightstack.psf.build_kernel")
        .map(|call| {
            let filter = call["metrics"]["filter"].as_str().unwrap();
            let [source, target] = [0, 1].map(|index| {
                f64_planes(&oracle_entry_path(&call["inputs"][index]))
                    .1
                    .remove(0)
            });
            let kernel = build_kernel(&source, &target, &options).unwrap();
            let shape = &call["arguments"]["shape"];
            let side = |axis: usize| shape[axis].as_u64().unwrap() as usize;
            assert_eq!(
                kernel.data.dim(),
                (side(0), side(1)),
                "{filter}: kernel shape"
            );
            assert_eq!(
                kernel.eps_absolute,
                call["arguments"]["eps"].as_f64(),
                "{filter}: eps"
            );
            PlaneKernel {
                filter: filter.to_string(),
                label: kernel_file_name(filter, reference),
                data: kernel.data,
                pixel_scale_arcsec: kernel_scale,
            }
        })
        .collect();
    assert_eq!(kernels.len(), 5, "one kernel per filter but the reference");
    let registry = FilterRegistry::builtin();
    let cube = oracle_cube(&call["inputs"][0], &registry);
    let options = MatchOptions {
        reference: reference.to_string(),
        mode: MatchMode::Lightstack,
        allow_unmatched: false,
    };
    let matched = match_cube(&cube, &kernels, &options, &[]).unwrap();
    assert!(
        !matched
            .warnings
            .iter()
            .any(|warning| warning.contains("pixel scale")),
        "the kernel sampling was checked against the cube: {:?}",
        matched.warnings
    );
    for plane in &matched.planes {
        let label = (!plane.filter.eq_ignore_ascii_case(reference))
            .then(|| kernel_file_name(&plane.filter, reference));
        assert_eq!(plane.kernel, label, "{}: kernel applied", plane.filter);
    }
    let (_, expected) = f64_planes(&oracle_entry_path(&call["outputs"][0]));
    assert_eq!(expected.len(), matched.cube.planes.len());
    for (plane, (ours, theirs)) in cube
        .filters
        .iter()
        .zip(matched.cube.planes.iter().zip(&expected))
    {
        let error = worst_relative(&plane.filter, &ours.mapv(f64::from), theirs);
        eprintln!(
            "{}: AstroBurst's kernels through match_cube against psf_match_datacube {error:.2e} of the max abs value",
            plane.filter
        );
        assert!(error <= 2e-6, "{}: {error:e} (F-class)", plane.filter);
    }
}

struct PlaneAgreement {
    one_sided: usize,
    interior_one_sided: usize,
    worst_of_max: f64,
    interior_worst_of_range: f64,
}

fn plane_agreement(got: &Array2<f64>, expected: &Array2<f64>, border: usize) -> PlaneAgreement {
    assert_eq!(got.dim(), expected.dim());
    let (rows, cols) = expected.dim();
    let finite: Vec<f64> = expected
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    let scale = finite
        .iter()
        .fold(0.0f64, |acc, value| acc.max(value.abs()));
    let mut sorted = finite.clone();
    sorted.sort_by(f64::total_cmp);
    let at = |fraction: f64| sorted[((sorted.len() - 1) as f64 * fraction).round() as usize];
    let range = at(0.999) - at(0.001);
    let mut agreement = PlaneAgreement {
        one_sided: 0,
        interior_one_sided: 0,
        worst_of_max: 0.0,
        interior_worst_of_range: 0.0,
    };
    for (((row, col), ours), theirs) in got.indexed_iter().zip(expected.iter()) {
        let interior =
            (border..rows - border).contains(&row) && (border..cols - border).contains(&col);
        if ours.is_finite() != theirs.is_finite() {
            agreement.one_sided += 1;
            agreement.interior_one_sided += usize::from(interior);
        } else if ours.is_finite() {
            let difference = (ours - theirs).abs();
            agreement.worst_of_max = agreement.worst_of_max.max(difference / scale);
            if interior {
                agreement.interior_worst_of_range =
                    agreement.interior_worst_of_range.max(difference / range);
            }
        }
    }
    agreement
}

fn assert_cube_plane(
    label: &str,
    distorted: bool,
    ours: &Array2<f64>,
    theirs: &Array2<f64>,
) -> PlaneAgreement {
    assert_eq!(ours.dim(), theirs.dim(), "{label}: plane shape");
    let agreement = plane_agreement(ours, theirs, 2);
    assert_eq!(agreement.one_sided, 0, "{label}: the NaN masks differ");
    if distorted {
        assert!(
            agreement.interior_worst_of_range <= 1e-3,
            "{label}: {:e} (I class)",
            agreement.interior_worst_of_range
        );
    } else {
        assert!(
            agreement.worst_of_max <= 2e-6,
            "{label}: {:e} (F class)",
            agreement.worst_of_max
        );
    }
    agreement
}

const CUBE_CARDS: [&str; 8] = [
    "BUNIT", "CTYPE3", "CUNIT1", "CUNIT2", "RADESYS", "NFILTERS", "FILTERS", "DATASETS",
];

fn celestial_matrix(header: &HduHeader) -> [[f64; 2]; 2] {
    let cd = |i: usize, j: usize| header.get_f64(&format!("CD{i}_{j}"));
    if [cd(1, 1), cd(1, 2), cd(2, 1), cd(2, 2)]
        .iter()
        .any(Option::is_some)
    {
        let element = |i: usize, j: usize| cd(i, j).unwrap_or(0.0);
        return [[element(1, 1), element(1, 2)], [element(2, 1), element(2, 2)]];
    }
    let pc = |i: usize, j: usize| {
        header
            .get_f64(&format!("PC{i}_{j}"))
            .unwrap_or(if i == j { 1.0 } else { 0.0 })
    };
    let cdelt = |i: usize| header.get_f64(&format!("CDELT{i}")).unwrap_or(1.0);
    [
        [pc(1, 1) * cdelt(1), pc(1, 2) * cdelt(1)],
        [pc(2, 1) * cdelt(2), pc(2, 2) * cdelt(2)],
    ]
}

fn history_lines(header: &HduHeader) -> Vec<String> {
    header
        .cards
        .iter()
        .filter(|(key, _)| key == "HISTORY")
        .map(|(_, value)| value.trim().to_string())
        .collect()
}

fn assert_cube_header_matches(label: &str, ours: &HduHeader, golden: &HduHeader) {
    let planes = golden
        .get_i64("NFILTERS")
        .unwrap_or_else(|| panic!("{label}: the golden has no NFILTERS"));
    let mut keys: Vec<String> = CUBE_CARDS.iter().map(|key| key.to_string()).collect();
    for n in 1..=planes {
        keys.push(format!("FILTER{n}"));
        keys.push(format!("DATASET{n}"));
    }
    for key in keys {
        let expected = golden
            .get(&key)
            .unwrap_or_else(|| panic!("{label}: the golden has no {key}"));
        assert!(
            same_card(ours.get(&key), expected),
            "{label}: {key} is {:?}, the golden has {expected:?}",
            ours.get(&key)
        );
    }
    let (got, expected) = (celestial_matrix(ours), celestial_matrix(golden));
    let scale = expected
        .iter()
        .flatten()
        .fold(0.0f64, |acc, value| acc.max(value.abs()));
    for (i, j) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        assert!(
            (got[i][j] - expected[i][j]).abs() <= 1e-12 * scale,
            "{label}: celestial matrix element {}_{} is {} where the golden's PC times CDELT gives {}",
            i + 1,
            j + 1,
            got[i][j],
            expected[i][j]
        );
    }
    let ours_history = history_lines(ours);
    for line in history_lines(golden) {
        assert!(
            ours_history.iter().any(|our| our.contains(&line)),
            "{label}: the golden HISTORY line {line:?} is in none of ours: {ours_history:?}"
        );
    }
}

fn assert_runner_steps_match_the_goldens(
    recipe: &Recipe,
    report: &RecipeReport,
    manifest: &Value,
    distorted: &[&str],
) {
    let path_of = |value: &Value| PathBuf::from(value.as_str().unwrap());
    let match_call = oracle_calls(manifest, "match")[0];
    let reference = match_call["arguments"]["ref_filter"].as_str().unwrap();
    let align_call = oracle_calls(manifest, "align")[0];
    let align_filters: Vec<&str> = align_call["arguments"]["fits_list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item[2].as_str().unwrap())
        .collect();
    let align_golden = |filter: &str, side: &str| {
        let index = align_filters
            .iter()
            .position(|known| *known == filter)
            .unwrap_or_else(|| panic!("{filter}: not in align_reproject_fits"));
        oracle_entry_path(&align_call[side][index])
    };
    let built_kernels: Vec<&Value> = oracle_calls(manifest, "kernels")
        .into_iter()
        .filter(|call| call["function"] == "lightstack.psf.build_kernel")
        .collect();
    let saved_kernels: Vec<&Value> = oracle_calls(manifest, "kernels")
        .into_iter()
        .filter(|call| call["function"] == "lightstack.psf.save_kernel")
        .collect();
    let mut aligned: BTreeMap<String, PathBuf> = BTreeMap::new();
    for step in &report.steps {
        let at = |filter: &str| format!("{} {filter}", step.directory);
        let files = || {
            recipe
                .inputs
                .iter()
                .zip(step.report["files"].as_array().unwrap())
        };
        match step.step {
            "crop" => {
                for (input, file) in files() {
                    let label = at(&input.filter);
                    let call = oracle_calls(manifest, "crop")
                        .into_iter()
                        .find(|call| call["metrics"]["filter"] == input.filter.as_str())
                        .unwrap_or_else(|| panic!("{label}: no crop_from_radec call"));
                    let ours = first_image(&path_of(&file["output_path"])).1;
                    let theirs = first_image(&oracle_entry_path(&call["outputs"][0])).1;
                    assert_eq!(ours.dim(), theirs.dim(), "{label}: crop shape");
                    assert!(
                        identical(&ours, &theirs),
                        "{label}: the crop is not crop_from_radec's bit for bit"
                    );
                    eprintln!("{label}: identical to crop_from_radec, {:?}", ours.dim());
                }
            }
            "units" => {
                for (input, file) in files() {
                    let label = at(&input.filter);
                    let (ours_header, mut ours) = f64_planes(&path_of(&file["output_path"]));
                    let (golden_header, mut theirs) =
                        f64_planes(&align_golden(&input.filter, "inputs"));
                    let error = worst_relative(&label, &ours.remove(0), &theirs.remove(0));
                    assert!(
                        error <= 2e-6,
                        "{label}: {error:e} of the max abs value against the plane LIGHTSTACK aligned (F class)"
                    );
                    let bunit = golden_header.get("BUNIT").unwrap();
                    assert!(
                        same_card(ours_header.get("BUNIT"), bunit),
                        "{label}: BUNIT is {:?}, LIGHTSTACK wrote {bunit:?}",
                        ours_header.get("BUNIT")
                    );
                    eprintln!(
                        "{label}: F, {error:.2e} of the max abs value, BUNIT {}",
                        card(&ours_header, "BUNIT").unwrap()
                    );
                }
            }
            "align" => {
                for (input, file) in files() {
                    let label = at(&input.filter);
                    let output = path_of(&file["output_path"]);
                    let class = if distorted.contains(&input.filter.as_str()) {
                        AlignClass::Equivalent
                    } else {
                        AlignClass::Float
                    };
                    let error = assert_align_class(
                        &label,
                        class,
                        &first_image(&output).1,
                        &align_golden(&input.filter, "outputs"),
                    );
                    eprintln!("{label}: {class:?}, {error:.2e} against align_reproject_fits");
                    aligned.insert(input.filter.clone(), output);
                }
            }
            "cube" | "edit" | "match" => {
                let strings = |value: &Value| -> Vec<String> {
                    value
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| item.as_str().map(str::to_owned).unwrap_or_else(|| item["filter"].as_str().unwrap().to_owned()))
                        .collect()
                };
                let (golden, filters) = match step.step {
                    "cube" => (
                        &oracle_calls(manifest, "cube")[0]["outputs"][0],
                        strings(&step.report["planes"]),
                    ),
                    "edit" => {
                        let call = oracle_calls(manifest, "edit")
                            .into_iter()
                            .find(|call| call["function"] == "lightstack.datacube.build_valid_datacube")
                            .unwrap_or_else(|| panic!("{}: no build_valid_datacube call", step.directory));
                        (&call["outputs"][0], strings(&step.report["filters"]))
                    }
                    _ => (&match_call["outputs"][0], strings(&step.report["planes"])),
                };
                let (ours_header, ours) = f64_planes(&path_of(&step.report["output_path"]));
                let (golden_header, theirs) = f64_planes(&oracle_entry_path(golden));
                assert_eq!(ours.len(), filters.len(), "{}: planes", step.directory);
                assert_eq!(theirs.len(), filters.len(), "{}: golden planes", step.directory);
                for ((filter, ours), theirs) in filters.iter().zip(&ours).zip(&theirs) {
                    let label = at(filter);
                    if step.step != "match" {
                        let own = f64_planes(&aligned[filter]).1.remove(0);
                        assert!(
                            identical_f64(ours, &own),
                            "{label}: not the runner's own aligned plane bit for bit"
                        );
                    }
                    let agreement =
                        assert_cube_plane(&label, distorted.contains(&filter.as_str()), ours, theirs);
                    eprintln!(
                        "{label}: {}, max difference {:.2e} of the max abs value, interior {:.2e} of the 0.1 to 99.9 percentile range",
                        if distorted.contains(&filter.as_str()) { "I" } else { "F" },
                        agreement.worst_of_max,
                        agreement.interior_worst_of_range
                    );
                }
                assert_cube_header_matches(&step.directory, &ours_header, &golden_header);
                eprintln!("{}: header cards, celestial matrix and HISTORY agree", step.directory);
            }
            "psfs" => {
                for psf in step.report["prepared"]["psfs"].as_array().unwrap() {
                    let filter = psf["filter"].as_str().unwrap();
                    let label = at(filter);
                    let golden = if filter == reference {
                        &built_kernels[0]["inputs"][1]
                    } else {
                        &built_kernels
                            .iter()
                            .find(|call| call["metrics"]["filter"] == filter)
                            .unwrap_or_else(|| panic!("{label}: no build_kernel call"))["inputs"][0]
                    };
                    let ours = f64_planes(&path_of(&psf["output_path"])).1.remove(0);
                    let theirs = f64_planes(&oracle_entry_path(golden)).1.remove(0);
                    let error = worst_relative(&label, &ours, &theirs);
                    assert!(
                        error <= 1e-9,
                        "{label}: {error:e} of the peak against the PSF LIGHTSTACK fed to build_kernel (N class)"
                    );
                    eprintln!("{label}: N, {error:.2e} of the peak");
                }
            }
            "kernels" => {
                for kernel in step.report["kernels"].as_array().unwrap() {
                    let filter = kernel["filter"].as_str().unwrap();
                    let label = at(filter);
                    let name = kernel_file_name(filter, reference);
                    let golden = saved_kernels
                        .iter()
                        .find_map(|call| {
                            let output = &call["outputs"][0];
                            output["path"].as_str().unwrap().ends_with(&name).then_some(output)
                        })
                        .unwrap_or_else(|| panic!("{label}: no save_kernel output named {name}"));
                    let ours = f64_planes(&path_of(&kernel["output_path"])).1.remove(0);
                    let theirs = f64_planes(&oracle_entry_path(golden)).1.remove(0);
                    assert_eq!(ours.dim(), theirs.dim(), "{label}: kernel shape");
                    let error = worst_relative(&label, &ours, &theirs);
                    assert!(
                        error <= 1e-9,
                        "{label}: {error:e} of the peak against build_kernel (N class)"
                    );
                    eprintln!("{label}: N, {:?}, {error:.2e} of the peak", ours.dim());
                }
            }
            "export" => {}
            other => panic!("{}: no golden comparison for the {other} step", step.directory),
        }
    }
}

#[test]
fn the_d1_recipe_from_lightstacks_crops_reproduces_its_psf_matched_cube() {
    let mut value = read_json(&d1_dir().join(RECIPE));
    for input in value["inputs"].as_array_mut().unwrap() {
        let cropped = oracle_d1_dir()
            .join("01_crop")
            .join(input["path"].as_str().unwrap());
        input["path"] = Value::from(cropped.to_string_lossy().into_owned());
    }
    assert_eq!(
        value["steps"].as_array_mut().unwrap().remove(0)["step"],
        "crop"
    );
    assert_d1_recipe_reproduces_psf_matched_cube(value);
}

#[test]
fn the_complete_d1_recipe_reproduces_the_lightstack_psf_matched_cube() {
    assert_d1_recipe_reproduces_psf_matched_cube(read_json(&d1_dir().join(RECIPE)));
}

fn assert_d1_recipe_reproduces_psf_matched_cube(value: Value) {
    let recipe = Recipe::from_value(&value).unwrap().resolved(&d1_dir());
    recipe.validate().unwrap();
    let out = tempfile::tempdir().unwrap();
    let report = run_recipe(
        &recipe,
        &out.path().join("d1"),
        &ProgressHandle::with_sink(Arc::new(|_| {}), 1),
    )
    .unwrap();
    let manifest = read_json(&oracle_d1_dir().join(MANIFEST));
    let calls = oracle_calls(&manifest, "match");
    let (header, expected) = f64_planes(&oracle_entry_path(&calls[0]["outputs"][0]));
    let (_, got) = f64_planes(Path::new(report.cube.as_deref().unwrap()));
    assert_eq!(got.len(), expected.len());
    let planes = planes_from_header(&header, expected.len(), &FilterRegistry::builtin()).unwrap();
    let distorted: Vec<&str> = recipe
        .inputs
        .iter()
        .filter(|input| {
            extract_header_by_index(
                &open(&d1_dir().join(Path::new(&input.path).file_name().unwrap())),
                SCI,
            )
            .unwrap()
            .get("A_ORDER")
            .is_some()
        })
        .map(|input| input.filter.as_str())
        .collect();
    assert_eq!(distorted, ["F814W"], "D1 holds one SIP plane");
    assert_runner_steps_match_the_goldens(&recipe, &report, &manifest, &distorted);
    for ((plane, ours), theirs) in planes.iter().zip(&got).zip(&expected) {
        let agreement = assert_cube_plane(
            &plane.filter,
            distorted.contains(&plane.filter.as_str()),
            ours,
            theirs,
        );
        eprintln!(
            "{}: one-sided NaN {} (interior {}), max difference {:.2e} of the max abs value, interior {:.2e} of the 0.1 to 99.9 percentile range",
            plane.filter, agreement.one_sided, agreement.interior_one_sided, agreement.worst_of_max, agreement.interior_worst_of_range
        );
    }
}
