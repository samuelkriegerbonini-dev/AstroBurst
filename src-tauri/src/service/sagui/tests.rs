use super::*;
use crate::core::cubeprep::cube::{CubeLayout, CubePlane, DqPlanes};
use crate::types::header::HduHeader;

fn cube() -> Cube {
    let mut header = HduHeader::empty();
    for (key, value) in [("NAXIS", "3"), ("NAXIS1", "3"), ("NAXIS2", "2"), ("NAXIS3", "2"), ("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CTYPE3", "FILTER"), ("CRPIX1", "2"), ("CRPIX2", "1"), ("CRVAL1", "53"), ("CRVAL2", "-27"), ("CD1_1", "-0.00001"), ("CD1_2", "0"), ("CD2_1", "0"), ("CD2_2", "0.00001"), ("BUNIT", "Jy")] { header.set(key, value.into()); }
    let planes = vec![Array2::from_shape_vec((2, 3), vec![1., 2., 3., 10., 20., 30.]).unwrap(), Array2::from_shape_vec((2, 3), vec![101., 102., 103., 110., 120., 130.]).unwrap()];
    let filters = ["F150W", "F200W"].iter().enumerate().map(|(i, name)| CubePlane { filter: name.to_string(), dataset: "JWST".into(), instrument: "NIRCam".into(), pivot_um: Some(1.5 + i as f64 * 0.5), pixscale_arcsec: Some(0.036), bunit_in: "Jy".into(), fluxconv: Some(1.0), psf_fwhm_arcsec: Some(0.1), kernel: "known".into() }).collect();
    Cube { header, planes, filters, err: Some(vec![Array2::from_elem((2, 3), 2.); 2]), dq: Some(DqPlanes { planes: vec![Array2::zeros((2, 3)); 2], signed: false }) }
}

fn saved_bundle() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let cube = cube();
    let labels = vec![1_u32, 1, 1, 2, 2, 2];
    let mask = vec![1_u32; 6];
    let mut header = cube_base_header(&cube.header);
    header.set("BUNIT", "1".into());
    for (name, values) in [("labels", &labels), ("support", &mask)] {
        fs::write(dir.path().join(format!("{name}.bin")), values.iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>()).unwrap();
        let map = Array2::from_shape_vec((2, 3), values.clone()).unwrap();
        write_derived_mef(dir.path().join(format!("{name}.fits")).to_str().unwrap(), None, &[ImageHdu { data: HduData::U32(&map), header: Some(&header), extname: name, extver: 1 }]).unwrap();
    }
    let regions = photometry(&cube, &labels).unwrap();
    let mut csv = String::from("region,n_pixels,filter,pivot_um,flux,error,unit\n");
    for region in &regions { for band in &region.bands {
        csv.push_str(&format!("{},{},{},{},{},{},Jy\n", region.region, region.n_pixels, csv_text(&band.filter), band.pivot_um, band.flux, band.error.map(|value| value.to_string()).unwrap_or_default()));
    } }
    fs::write(dir.path().join("regions.csv"), csv).unwrap();
    let config = SaguiConfig { input_path: "unavailable-original.fits".into(), output_dir: dir.path().to_string_lossy().into_owned(), rscript_path: "unavailable-Rscript".into(), library_path: "unavailable-library".into(), n_regions: 2, starlet: false, starlet_scales: vec![1, 2], starlet_threshold: 2.5, dq_exclude_bits: 1, psf_target: "known".into(), mode: "ward".into(), knn_k: 2 };
    fs::write(dir.path().join("config.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    let result = SaguiResult { run_id: "saved-fixture".into(), output_dir: "old-output".into(), labels_path: "old-labels".into(), mask_path: "old-mask".into(), table_path: "old-table".into(), manifest_path: "old-manifest".into(), width: 3, height: 2, unit: "Jy".into(), labels, regions, warnings: vec![] };
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
    let files = ["labels.fits", "support.fits", "regions.csv", "config.json", "labels.bin", "support.bin"].iter().map(|name| json!({ "path": name, "sha256": sha256(&dir.path().join(name), &progress).unwrap() })).collect::<Vec<_>>();
    fs::write(dir.path().join("manifest.json"), serde_json::to_vec(&json!({ "schema_version": 1, "status": "complete", "config": config, "result": result, "files": files })).unwrap()).unwrap();
    dir
}

#[test]
fn saved_results_reopen_without_r_or_original_inputs_and_use_current_bundle_paths() {
    let dir = saved_bundle();
    let restored = saved::load(dir.path().join("manifest.json").to_str().unwrap()).unwrap();
    assert_eq!(restored.result.labels, [1, 1, 1, 2, 2, 2]);
    assert_eq!(restored.result.regions[0].bands[0].flux, 6.0);
    assert_eq!(Path::new(&restored.result.labels_path).parent().unwrap(), dir.path().canonicalize().unwrap());
    assert_eq!(restored.config.rscript_path, "unavailable-Rscript");
    assert!(restored.result.warnings.last().unwrap().contains("not revalidated"));
}

#[test]
fn saved_results_reject_changed_files_unverified_tables_incomplete_jobs_and_path_escape() {
    let dir = saved_bundle();
    let path = dir.path().join("manifest.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for case in 0..6 {
        let mut changed = original.clone();
        match case {
            0 => changed["status"] = json!("running"),
            1 => changed["result"]["regions"][0]["bands"][0]["flux"] = json!(999.0),
            2 => changed["result"]["labels"][0] = json!(2),
            3 => changed["files"][0]["path"] = json!("../labels.fits"),
            4 => changed["files"] = json!([]),
            _ => changed["result"]["regions"][0]["n_pixels"] = json!(4),
        }
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(saved::load(path.to_str().unwrap()).is_err(), "case {case}");
    }
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    fs::write(dir.path().join("regions.csv"), "changed").unwrap();
    assert!(saved::load(path.to_str().unwrap()).unwrap_err().to_string().contains("changed"));
}

#[test]
fn saved_results_reject_fits_maps_disagreeing_with_the_table_even_with_updated_hashes() {
    for name in ["labels.fits", "support.fits"] {
        let dir = saved_bundle();
        let path = dir.path().join(name);
        let header = extract_header_by_index(&File::open(&path).unwrap(), 1).unwrap();
        let map = Array2::<u32>::zeros((2, 3));
        write_derived_mef(path.to_str().unwrap(), None, &[ImageHdu { data: HduData::U32(&map), header: Some(&header), extname: "MAP", extver: 1 }]).unwrap();
        let manifest_path = dir.path().join("manifest.json");
        let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
        manifest["files"].as_array_mut().unwrap().iter_mut().find(|file| file["path"] == name).unwrap()["sha256"] = json!(sha256(&path, &progress).unwrap());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(saved::load(manifest_path.to_str().unwrap()).is_err(), "{name}");
    }
}

#[test]
fn non_square_binary_preserves_x_y_band_order_and_mask_without_changing_photometry() {
    let cube = cube();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("science.bin");
    write_science(&path, &cube, &[true, false, true, true, true, true]).unwrap();
    let values: Vec<f64> = fs::read(path).unwrap().chunks_exact(8).map(|v| f64::from_le_bytes(v.try_into().unwrap())).collect();
    assert_eq!(values.len(), 12);
    assert_eq!(values[0], 1.0);
    assert!(values[1].is_nan());
    assert_eq!(values[2], 3.0);
    assert_eq!(values[3], 10.0);
    assert_eq!(values[5], 30.0);
    assert_eq!(values[6], 101.0);
    assert!(values[7].is_nan());
    assert_eq!(values[11], 130.0);
    assert_eq!(cube.planes[0][(0, 1)], 2.0);
}

#[test]
fn original_calibrated_regional_photometry_preserves_labels_and_unknown_errors() {
    let mut cube = cube();
    let labels = [2, 0, 1, 2, 1, 1];
    let result = photometry(&cube, &labels).unwrap();
    assert_eq!(result.iter().map(|r| (r.region, r.n_pixels)).collect::<Vec<_>>(), [(1, 3), (2, 2)]);
    assert_eq!(result[0].bands[0].flux, 53.0);
    assert_eq!(result[0].bands[1].flux, 353.0);
    assert_eq!(result[0].bands[0].error, Some(12.0_f64.sqrt()));
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
        cube.err.as_mut().unwrap()[0][(1, 1)] = invalid;
        let result = photometry(&cube, &labels).unwrap();
        assert_eq!(result[0].bands[0].error, None);
        assert_eq!(result[1].bands[0].error, Some(8.0_f64.sqrt()));
        assert_eq!(result[0].bands[0].flux, 53.0);
    }
    cube.err = None;
    assert!(photometry(&cube, &labels).unwrap().iter().all(|r| r.bands.iter().all(|b| b.error.is_none())));
}

#[test]
fn regional_flux_and_errors_are_converted_from_mjy_per_pixel_to_jy() {
    let mut cube = cube();
    cube.header.set("BUNIT", "mJy/pixel".into());
    let regions = photometry(&cube, &[1, 0, 0, 1, 0, 0]).unwrap();
    assert_eq!(regions[0].bands[0].flux, 0.011);
    assert_eq!(regions[0].bands[0].error, Some(8.0_f64.sqrt() * 1e-3));
    assert_eq!(cube.planes[0][(0, 0)], 1.0);
}

#[test]
fn finite_support_preserves_zero_and_excludes_high_dq_only_by_explicit_policy() {
    let mut cube = cube();
    cube.planes[0][(0, 0)] = 0.;
    cube.planes[1][(0, 1)] = f32::INFINITY;
    cube.dq.as_mut().unwrap().planes[1][(1, 2)] = 0x80000000;
    assert_eq!(eligible(&cube, 0), [true, false, true, true, true, true]);
    assert_eq!(eligible(&cube, 0x80000000), [true, false, true, true, true, false]);
}

#[test]
fn input_contract_rejects_spectral_axis_unknown_units_companion_shape_and_duplicate_filters() {
    let valid = cube();
    let info = validate_cube(&valid).unwrap();
    assert_eq!((info.width, info.height, info.bands.len()), (3, 2, 2));
    for (key, value) in [("CTYPE3", "WAVE"), ("BUNIT", "MJy/sr"), ("BUNIT", "count")] {
        let mut invalid = cube();
        invalid.header.set(key, value.into());
        assert!(validate_cube(&invalid).is_err(), "{key}={value}");
    }
    let mut invalid = cube();
    invalid.err.as_mut().unwrap()[0] = Array2::zeros((3, 2));
    assert!(validate_cube(&invalid).unwrap_err().to_string().contains("ERR"));
    let mut invalid = cube();
    invalid.filters[1].filter = invalid.filters[0].filter.clone();
    assert!(validate_cube(&invalid).is_err());
}

#[test]
fn mef_contract_reads_filters_err_dq_and_preserves_non_square_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(path.to_str().unwrap(), &cube(), CubeLayout::Mef, "test").unwrap();
    let info = inspect_input(path.to_str().unwrap()).unwrap();
    assert_eq!((info.width, info.height), (3, 2));
    assert_eq!(info.bands.iter().map(|b| b.filter.as_str()).collect::<Vec<_>>(), ["F150W", "F200W"]);
    assert!(info.has_err && info.has_dq);
}

#[test]
fn companion_contract_refuses_mismatched_error_units_and_grid() {
    let cube = cube();
    let mut header = cube.header.clone();
    assert!(validate_companion_header(&cube, &header, "ERR").unwrap().is_empty());
    header.set("BUNIT", "mJy".into());
    assert!(validate_companion_header(&cube, &header, "ERR").unwrap_err().to_string().contains("BUNIT"));
    header.set("BUNIT", "Jy".into());
    header.set("CRPIX1", "3".into());
    for name in ["ERR", "DQ"] { assert!(validate_companion_header(&cube, &header, name).unwrap_err().to_string().contains("WCS")); }
    let mut inherited = HduHeader::empty();
    assert!(validate_companion_header(&cube, &inherited, "ERR").is_err());
    inherited.set("BUNIT", "Jy".into());
    assert_eq!(validate_companion_header(&cube, &inherited, "ERR").unwrap().len(), 1);
    for key in ["CRPIX2", "CRVAL2", "CD2_2", "PC2_2", "A_ORDER"] {
        let mut partial = inherited.clone();
        partial.set(key, "2".into());
        assert!(validate_companion_header(&cube, &partial, "ERR").is_err(), "{key}");
    }
}

#[test]
fn maps_cannot_publish_excluded_out_of_range_missing_or_malformed_labels() {
    assert!(validate_maps(&[1, 0, 2], &[1, 0, 1], &[true, false, true], 2).is_ok());
    for (labels, mask) in [(vec![1, 2, 0], vec![1, 1, 1]), (vec![1, 0, 3], vec![1, 0, 1]), (vec![1, 0, 1], vec![1, 0, 1]), (vec![1, 0, 2], vec![1, 0, 0]), (vec![1, 0, 2], vec![1, 0, 2])] { assert!(validate_maps(&labels, &mask, &[true, false, true], 2).is_err()); }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("labels.bin");
    fs::write(&path, [1, 0, 0, 0, 2]).unwrap();
    assert!(read_integers(&path, 1).is_err());
}

#[test]
fn runtime_identity_refuses_unpinned_versions_optional_backends_and_missing_runtime() {
    let good = format!("ASTROBURST_R\t{}\t0.0.0.9000\t{}\n", runtime::R_VERSION, runtime::SAGUI_COMMIT);
    assert!(runtime::parse_runtime(&good).unwrap().ready);
    assert!(!runtime::parse_runtime(&good.replace(runtime::R_VERSION, "4.1.0")).unwrap().ready);
    assert!(!runtime::parse_runtime(&good.replace(runtime::SAGUI_COMMIT, "other")).unwrap().ready);
    assert!(!runtime::parse_runtime(&format!("{good}ASTROBURST_ISSUE\tCPU profile excludes torch\n")).unwrap().ready);
    assert!(runtime::parse_runtime("not R").is_err());
    assert!(!runtime::inspect("missing-Rscript", "missing-library").ready);
}

#[test]
fn memory_and_run_paths_fail_closed_at_limits() {
    assert!(memory_estimate(10000, 4, "ward", 40) > memory_estimate(1000, 4, "ward", 40));
    assert_eq!(memory_estimate(u64::MAX, 4, "ward", 40), u64::MAX);
    for id in ["../escape", "a/b", "a\\b", "", "a b", "a:stream"] { assert!(!valid_run_id(id)); }
    assert!(valid_run_id("run_2026-11-02"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known.txt");
    fs::write(&path, "abc").unwrap();
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
    assert_eq!(sha256(&path, &progress).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

#[test]
fn integrity_checks_observe_cancellation_and_library_content_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.rdb");
    fs::write(&path, "abc").unwrap();
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
    let before = library_fingerprint(dir.path(), &progress).unwrap();
    fs::write(&path, "def").unwrap();
    assert_ne!(before, library_fingerprint(dir.path(), &progress).unwrap());
    let cancelled = progress.with_cancel_source(std::sync::Arc::new(|| true));
    assert!(sha256(&path, &cancelled).unwrap_err().to_string().contains("cancelled"));
    assert!(library_fingerprint(dir.path(), &cancelled).is_err());
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn official_sagui_matches_direct_non_square_segmentation_and_regional_sed() {
    let rscript = std::env::var("ASTROBURST_SAGUI_RSCRIPT").expect("ASTROBURST_SAGUI_RSCRIPT");
    let library = std::env::var("ASTROBURST_SAGUI_LIBRARY").expect("ASTROBURST_SAGUI_LIBRARY");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(path.to_str().unwrap(), &cube(), CubeLayout::Mef, "test").unwrap();
    let config = SaguiConfig { input_path: path.to_string_lossy().into_owned(), output_dir: dir.path().to_string_lossy().into_owned(), rscript_path: rscript.clone(), library_path: library.clone(), n_regions: 2, starlet: false, starlet_scales: vec![1, 2], starlet_threshold: 2.5, dq_exclude_bits: 1, psf_target: "known synthetic PSF".into(), mode: "ward".into(), knn_k: 2 };
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 5);
    let result = run(config, "official-reference".into(), &progress).unwrap();
    let work = dir.path().join("direct");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("reference.R"), include_str!("reference-test.R")).unwrap();
    let mut command = runtime::command(&rscript, &library, &work).unwrap();
    command.args(["inspect", &library, work.to_str().unwrap()]);
    runtime::execute(command, &work, &progress, None).unwrap();
    let mut direct = std::process::Command::new(&rscript);
    direct.arg("--vanilla").arg(work.join("reference.R")).arg(&library).arg(&work);
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; direct.creation_flags(0x08000000); }
    runtime::execute(direct, &work, &progress, None).unwrap();
    let reference = read_integers(&work.join("reference-labels.bin"), 6).unwrap();
    for i in 0..6 { for j in 0..6 { assert_eq!(result.labels[i] == result.labels[j], reference[i] == reference[j]); } }
    let sed = fs::read_to_string(work.join("reference-sed.tsv")).unwrap();
    for line in sed.lines().skip(1) {
        let fields: Vec<_> = line.split('\t').collect();
        let official_label: u32 = fields[0].parse().unwrap();
        let position = reference.iter().position(|&label| label == official_label).unwrap();
        let region = result.regions.iter().find(|r| r.region == result.labels[position]).unwrap();
        let wavelength: f64 = fields[1].parse().unwrap();
        let band = region.bands.iter().find(|b| b.pivot_um == wavelength).unwrap();
        let expected_flux: f64 = fields[2].parse().unwrap();
        let expected_err: f64 = fields[3].parse().unwrap();
        assert!((band.flux - expected_flux).abs() < 1e-10);
        assert!((band.error.unwrap() - expected_err).abs() < 1e-12);
        assert_eq!(region.n_pixels, fields[4].parse::<usize>().unwrap());
    }
    let file = File::open(&result.labels_path).unwrap();
    let header = extract_header_by_index(&file, 1).unwrap();
    assert_eq!((header.get_i64("NAXIS1"), header.get_i64("NAXIS2")), (Some(3), Some(2)));
    assert!(same_grid(&cube().wcs().unwrap(), (2, 3), &header_wcs(&header).unwrap(), (2, 3)));
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&result.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["status"], "complete");
    for file in manifest["files"].as_array().unwrap() {
        assert_eq!(sha256(&Path::new(&result.output_dir).join(file["path"].as_str().unwrap()), &progress).unwrap(), file["sha256"].as_str().unwrap());
    }
    let reopened = saved::load(&result.manifest_path).unwrap();
    assert_eq!(reopened.result.labels, result.labels);
    assert_eq!(reopened.result.unit, "Jy");
    assert_eq!(reopened.result.regions[0].bands[0].flux, result.regions[0].bands[0].flux);
    fs::write(&result.table_path, "modified").unwrap();
    assert!(saved::load(&result.manifest_path).unwrap_err().to_string().contains("changed"));
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn official_sagui_sparse_and_starlet_match_direct_reference() {
    let rscript = std::env::var("ASTROBURST_SAGUI_RSCRIPT").expect("ASTROBURST_SAGUI_RSCRIPT");
    let library = std::env::var("ASTROBURST_SAGUI_LIBRARY").expect("ASTROBURST_SAGUI_LIBRARY");
    let dir = tempfile::tempdir().unwrap();
    let mut cube = cube();
    cube.header.set("NAXIS1", "23".into());
    cube.header.set("NAXIS2", "17".into());
    cube.planes = (0..2).map(|b| Array2::from_shape_fn((17, 23), |(y, x)| {
        let g1 = (-((x as f64 - 7.0).powi(2) + (y as f64 - 8.0).powi(2)) / 16.0).exp();
        let g2 = (-((x as f64 - 16.0).powi(2) + (y as f64 - 9.0).powi(2)) / 12.0).exp();
        (0.01 + [3.0, 8.0][b] * g1 + [5.0, 2.0][b] * g2) as f32
    })).collect();
    cube.err = Some(vec![Array2::from_elem((17, 23), 0.01); 2]);
    cube.dq = Some(DqPlanes { planes: vec![Array2::zeros((17, 23)); 2], signed: false });
    let input = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(input.to_str().unwrap(), &cube, CubeLayout::Mef, "test").unwrap();
    for mode in ["ward", "sparse"] {
        let config = SaguiConfig { input_path: input.to_string_lossy().into_owned(), output_dir: dir.path().to_string_lossy().into_owned(), rscript_path: rscript.clone(), library_path: library.clone(), n_regions: 3, starlet: true, starlet_scales: vec![1, 2, 3], starlet_threshold: 1.5, dq_exclude_bits: 1, psf_target: "known synthetic PSF".into(), mode: mode.into(), knn_k: 20 };
        let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 5);
        let result = run(config, mode.into(), &progress).unwrap();
        let work = Path::new(&result.output_dir);
        fs::write(work.join("reference.R"), "args <- commandArgs(trailingOnly=TRUE)\n.libPaths(c(args[[1]], .Library), include.site=FALSE)\nsetwd(args[[2]])\ncube <- array(readBin('science.bin', what='double', n=23*17*2, size=8, endian='little'), c(23L,17L,2L))\nparams <- list(input=cube,Ncomp=3L,use_starlet_mask=TRUE,starlet_J=3L,starlet_scales=1:3,denoise_k=1.5)\nif(args[[3]] == 'sparse') { params$knn_k <- 20L;params$auto_k <- FALSE;fit <- do.call(sagui::segment_regions_large,params) } else { fit <- do.call(sagui::segment_regions,params) }\nlabels <- as.integer(fit$cluster_map)\nlabels[is.na(labels)] <- 0L\nwriteBin(labels,'reference-labels.bin',size=4,endian='little')\nwriteBin(as.integer(fit$mask),'reference-mask.bin',size=4,endian='little')\n").unwrap();
        let mut command = std::process::Command::new(&rscript);
        command.arg("--vanilla").arg(work.join("reference.R")).arg(&library).arg(work).arg(mode);
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
        let direct_logs = dir.path().join(format!("direct-{mode}"));
        fs::create_dir(&direct_logs).unwrap();
        runtime::execute(command, &direct_logs, &progress, None).unwrap();
        let labels = read_integers(&work.join("reference-labels.bin"), 391).unwrap();
        assert_eq!(read_integers(&work.join("reference-mask.bin"), 391).unwrap(), read_integers(&work.join("support.bin"), 391).unwrap());
        for i in 0..391 { assert_eq!(labels[i] == 0, result.labels[i] == 0); for j in 0..391 { assert_eq!(labels[i] == labels[j], result.labels[i] == result.labels[j]); } }
    }
}

#[test]
#[ignore = "Requires local Rscript"]
fn r_process_cancellation_returns_without_waiting_for_computation() {
    let rscript = std::env::var("ASTROBURST_SAGUI_RSCRIPT").expect("ASTROBURST_SAGUI_RSCRIPT");
    let dir = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(rscript);
    command.args(["--vanilla", "-e", "Sys.sleep(60)"]);
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    let start = std::time::Instant::now();
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1).with_cancel_source(std::sync::Arc::new(move || start.elapsed().as_millis() > 500));
    let error = runtime::execute(command, dir.path(), &progress, None).unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(start.elapsed().as_secs() < 10);
}
