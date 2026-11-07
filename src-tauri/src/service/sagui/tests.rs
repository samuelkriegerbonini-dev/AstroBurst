use super::*;
use crate::core::cubeprep::cube::{CubeLayout, CubePlane, DqPlanes};
use crate::core::imaging::dq_flags::DqTable;
use crate::types::header::HduHeader;

fn cube_sized(width: usize, height: usize, planes: Vec<Vec<f32>>) -> Cube {
    let mut header = HduHeader::empty();
    for (key, value) in [("NAXIS", "3".to_string()), ("NAXIS1", width.to_string()), ("NAXIS2", height.to_string()), ("NAXIS3", planes.len().to_string()), ("CTYPE1", "RA---TAN".into()), ("CTYPE2", "DEC--TAN".into()), ("CTYPE3", "FILTER".into()), ("CRPIX1", "2".into()), ("CRPIX2", "1".into()), ("CRVAL1", "53".into()), ("CRVAL2", "-27".into()), ("CD1_1", "-0.00001".into()), ("CD1_2", "0".into()), ("CD2_1", "0".into()), ("CD2_2", "0.00001".into()), ("BUNIT", "Jy".into())] { header.set(key, value); }
    let bands = planes.len();
    let planes: Vec<Array2<f32>> = planes.into_iter().map(|values| Array2::from_shape_vec((height, width), values).unwrap()).collect();
    let filters = ["F150W", "F200W", "F277W"].iter().take(bands).enumerate().map(|(i, name)| CubePlane { filter: name.to_string(), dataset: "JWST".into(), instrument: "NIRCam".into(), pivot_um: Some(1.5 + i as f64 * 0.5), pixscale_arcsec: Some(0.036), bunit_in: "Jy".into(), fluxconv: Some(1.0), psf_fwhm_arcsec: Some(0.1), kernel: "known".into() }).collect();
    Cube { header, planes, filters, err: Some(vec![Array2::from_elem((height, width), 2.); bands]), dq: Some(DqPlanes { planes: vec![Array2::zeros((height, width)); bands], signed: false }) }
}

fn cube() -> Cube {
    cube_sized(3, 2, vec![vec![1., 2., 3., 10., 20., 30.], vec![101., 102., 103., 110., 120., 130.]])
}

fn config_for(input: &str, output_dir: &Path, rscript: &str, library: &str) -> SaguiConfig {
    SaguiConfig { input_path: input.into(), output_dir: output_dir.to_string_lossy().into_owned(), rscript_path: rscript.into(), library_path: library.into(), n_regions: 2, starlet: false, starlet_scales: vec![1, 2], starlet_threshold: 2.5, dq_exclude_bits: 1, psf_target: "known".into(), mode: "ward".into(), knn_k: 2, sed_scaling: "median_centre".into() }
}

fn delta(size: usize) -> Array2<f32> {
    let mut map = Array2::zeros((size, size));
    map[(size / 2, size / 2)] = 1.0;
    map
}

fn lateral() -> Array2<f32> {
    let mut map = Array2::zeros((3, 3));
    map[(1, 1)] = 1.0;
    map[(1, 0)] = 0.5;
    map[(1, 2)] = 0.5;
    map
}

fn matched(planes: Vec<Array2<f32>>) -> NoiseCorrelation {
    NoiseCorrelation { half: planes[0].dim().0 / 2, source: NOISCORR_SOURCE_MATCH.into(), planes }
}

fn quiet() -> ProgressHandle { ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1) }

fn recording(total: u64) -> (ProgressHandle, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink: crate::infra::progress::ProgressSink = { let seen = std::sync::Arc::clone(&seen); std::sync::Arc::new(move |payload: &crate::infra::progress::ProgressPayload| seen.lock().unwrap().push(payload.stage.clone())) };
    (ProgressHandle::with_sink(sink, total), seen)
}

fn brute_force_variance(labels: &[u32], width: usize, region: u32, err: &Array2<f32>, map: &Array2<f32>) -> f64 {
    let half = (map.dim().0 / 2) as isize;
    let mut total = 0.0;
    for p in (0..labels.len()).filter(|&p| labels[p] == region) {
        for q in (0..labels.len()).filter(|&q| labels[q] == region) {
            let (dy, dx) = ((q / width) as isize - (p / width) as isize, (q % width) as isize - (p % width) as isize);
            if dy.abs() > half || dx.abs() > half { continue; }
            total += f64::from(err[(p / width, p % width)]) * f64::from(err[(q / width, q % width)]) * f64::from(map[((half + dy) as usize, (half + dx) as usize)]);
        }
    }
    total
}

fn fits_card(key: &str, value: &str) -> Vec<u8> {
    let mut bytes = format!("{key:<8}= {value}").into_bytes();
    bytes.resize(80, b' ');
    bytes
}

fn append_noise_correlation(path: &Path, planes: &[Array2<f32>], ncorrk: usize, source: &str) {
    let size = planes[0].dim().0;
    let mut cards = Vec::new();
    for (key, value) in [("XTENSION", "'IMAGE   '".to_string()), ("BITPIX", "-32".into()), ("NAXIS", "3".into()), ("NAXIS1", size.to_string()), ("NAXIS2", size.to_string()), ("NAXIS3", planes.len().to_string()), ("PCOUNT", "0".into()), ("GCOUNT", "1".into()), ("EXTNAME", "'NOISCORR'".into()), ("NCORRK", ncorrk.to_string()), ("NCORRSRC", format!("'{source}'"))] { cards.extend(fits_card(key, &value)); }
    let mut end = b"END".to_vec();
    end.resize(80, b' ');
    cards.extend(end);
    while cards.len() % 2880 != 0 { cards.push(b' '); }
    let mut data: Vec<u8> = planes.iter().flat_map(|plane| plane.iter().flat_map(|value| value.to_be_bytes())).collect();
    while data.len() % 2880 != 0 { data.push(0); }
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(&cards).unwrap();
    file.write_all(&data).unwrap();
}

fn saved_bundle_from(csv: fn(&[Region]) -> String) -> tempfile::TempDir {
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
    let regions = photometry(&cube, &labels, None, &quiet()).unwrap();
    fs::write(dir.path().join("regions.csv"), csv(&regions)).unwrap();
    let config = config_for("unavailable-original.fits", dir.path(), "unavailable-Rscript", "unavailable-library");
    fs::write(dir.path().join("config.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    let result = SaguiResult { run_id: "saved-fixture".into(), output_dir: "old-output".into(), labels_path: "old-labels".into(), mask_path: "old-mask".into(), table_path: "old-table".into(), manifest_path: "old-manifest".into(), width: 3, height: 2, unit: "Jy".into(), labels, regions, warnings: vec![] };
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
    let files = ["labels.fits", "support.fits", "regions.csv", "config.json", "labels.bin", "support.bin"].iter().map(|name| json!({ "path": name, "sha256": sha256(&dir.path().join(name), &progress).unwrap() })).collect::<Vec<_>>();
    fs::write(dir.path().join("manifest.json"), serde_json::to_vec(&json!({ "schema_version": 1, "status": "complete", "config": config, "result": result, "files": files })).unwrap()).unwrap();
    dir
}

fn saved_bundle() -> tempfile::TempDir { saved_bundle_from(regions_csv) }

#[test]
fn saved_results_reopen_without_r_or_original_inputs_and_use_current_bundle_paths() {
    let dir = saved_bundle();
    let restored = saved::load(dir.path().join("manifest.json").to_str().unwrap()).unwrap();
    assert_eq!(restored.result.labels, [1, 1, 1, 2, 2, 2]);
    assert_eq!(restored.result.regions[0].bands[0].flux, 6.0);
    assert_eq!(Path::new(&restored.result.labels_path).parent().unwrap(), plain(&dir.path().canonicalize().unwrap()));
    assert!(!restored.result.output_dir.starts_with(r"\\?\"));
    assert_eq!(restored.config.rscript_path, "unavailable-Rscript");
    assert!(restored.result.warnings.last().unwrap().contains("not revalidated"));
}

#[test]
fn saved_results_written_before_the_correlated_error_column_reopen_with_the_legacy_csv() {
    let dir = saved_bundle_from(saved::legacy_regions_csv);
    assert!(fs::read_to_string(dir.path().join("regions.csv")).unwrap().starts_with("region,n_pixels,filter,pivot_um,flux,error,unit\n"));
    let manifest_path = dir.path().join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    for region in manifest["result"]["regions"].as_array_mut().unwrap() {
        for key in ["n_components", "min_snr", "min_snr_lower_bound"] { region.as_object_mut().unwrap().remove(key); }
        for band in region["bands"].as_array_mut().unwrap() { band.as_object_mut().unwrap().remove("error_correlated"); }
    }
    manifest["config"].as_object_mut().unwrap().remove("sed_scaling");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let config_path = dir.path().join("config.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config.as_object_mut().unwrap().remove("sed_scaling");
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
    manifest["files"].as_array_mut().unwrap().iter_mut().find(|file| file["path"] == "config.json").unwrap()["sha256"] = json!(sha256(&config_path, &progress).unwrap());
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let restored = saved::load(manifest_path.to_str().unwrap()).unwrap();
    assert_eq!(restored.result.regions[0].bands[0].error_correlated, None);
    let snr = 6.0 / 12.0_f64.sqrt();
    assert_eq!((restored.result.regions[0].n_components, restored.result.regions[0].min_snr, restored.result.regions[0].min_snr_lower_bound), (1, Some(snr), Some(snr / 3.0_f64.sqrt())));
    assert_eq!(restored.config.sed_scaling, "median_centre");
}

#[test]
fn saved_results_reject_changed_files_unverified_tables_incomplete_jobs_and_path_escape() {
    let dir = saved_bundle();
    let path = dir.path().join("manifest.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for case in 0..8 {
        let mut changed = original.clone();
        match case {
            0 => changed["status"] = json!("running"),
            1 => changed["result"]["regions"][0]["bands"][0]["flux"] = json!(999.0),
            2 => changed["result"]["labels"][0] = json!(2),
            3 => changed["files"][0]["path"] = json!("../labels.fits"),
            4 => changed["files"] = json!([]),
            5 => changed["result"]["regions"][0]["n_components"] = json!(2),
            6 => changed["result"]["regions"][0]["min_snr"] = json!(1e9),
            _ => changed["result"]["regions"][0]["n_pixels"] = json!(4),
        }
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(saved::load(path.to_str().unwrap()).is_err(), "case {case}");
    }
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    fs::write(dir.path().join("regions.csv"), "changed").unwrap();
    assert!(saved::load(path.to_str().unwrap()).unwrap_err().to_string().contains("changed"));
    let missing = dir.path().join("missing/manifest.json");
    assert!(format!("{:#}", saved::load(missing.to_str().unwrap()).unwrap_err()).contains("missing"));
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
fn regions_csv_bytes_are_pinned() {
    let regions = photometry(&cube(), &[1, 1, 1, 2, 2, 2], None, &quiet()).unwrap();
    let expected = "region,n_pixels,filter,pivot_um,flux,error,error_correlated_jy,unit\n1,3,\"F150W\",1.5,6,3.4641016151377544,,Jy\n1,3,\"F200W\",2,306,3.4641016151377544,,Jy\n2,3,\"F150W\",1.5,60,3.4641016151377544,,Jy\n2,3,\"F200W\",2,360,3.4641016151377544,,Jy\n";
    assert_eq!(regions_csv(&regions), expected);
    assert_eq!(saved::legacy_regions_csv(&regions), expected.replace("error,error_correlated_jy,unit", "error,unit").replace(",,Jy", ",Jy"));
    let mut quoted = regions.clone();
    quoted[0].bands[0].filter = "F1\"50W".into();
    quoted[0].bands[0].error_correlated = Some(0.25);
    assert!(regions_csv(&quoted).contains("1,3,\"F1\"\"50W\",1.5,6,3.4641016151377544,0.25,Jy\n"));
}

#[test]
fn validate_maps_counts_support_on_excluded_and_unlabelled() {
    let labels = [1, 0, 2, 0];
    let eligible = [true, true, true, false];
    assert_eq!(map_counts(&labels, &[1, 1, 1, 1], &eligible), MapCounts { pixels: 4, eligible_pixels: 3, excluded_by_policy: 1, support_pixels: 4, labelled_pixels: 2, support_unlabelled: 2, support_on_excluded: 1 });
    assert!(validate_maps(&labels, &[1, 1, 1, 1], &eligible, 2).unwrap_err().to_string().contains("excluded"));
    validate_maps(&labels, &[1, 1, 1, 0], &eligible, 2).unwrap();
    assert_eq!(map_counts(&labels, &[1, 1, 1, 0], &eligible).support_on_excluded, 0);
    let warnings = map_warnings(&map_counts(&labels, &[1, 1, 1, 1], &eligible), 3);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].starts_with("2 support pixels were left unlabelled") && warnings[1].starts_with("3 excluded pixels were marked as support by SAGUI's upstream mask"), "{warnings:?}");
    assert_eq!(map_warnings(&map_counts(&labels, &[1, 1, 1, 0], &eligible), 0).len(), 1);
    assert!(map_warnings(&map_counts(&[1, 2, 1, 0], &[1, 1, 1, 0], &eligible), 0).is_empty());
}

#[test]
fn sparse_labels_above_n_regions_report_a_disconnected_graph() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_for("input.fits", dir.path(), "Rscript", "library");
    config.mode = "sparse".into();
    let error = check_connected(&[1, 0, 3, 2], &config).unwrap_err().to_string();
    assert!(error.contains("disconnected") && error.contains("nearest-neighbour") && error.contains("3 clusters for 2 requested") && error.contains("knn_k 2"), "{error}");
    check_connected(&[1, 0, 2, 2], &config).unwrap();
    config.mode = "ward".into();
    check_connected(&[1, 0, 3, 2], &config).unwrap();
}

#[test]
fn a_zero_err_plane_warns_and_keeps_the_regional_error() {
    let mut cube = cube();
    cube.err.as_mut().unwrap()[0] = Array2::zeros((2, 3));
    cube.planes[0][(0, 1)] = f32::NAN;
    let info = validate_cube(&cube).unwrap();
    assert!(info.warnings.iter().any(|w| w.starts_with("F150W: 5 pixels with finite SCI have ERR = 0") && w.contains("zero variance")), "{:?}", info.warnings);
    assert!(!info.warnings.iter().any(|w| w.starts_with("F200W:")), "{:?}", info.warnings);
    let regions = photometry(&cube, &[1, 0, 1, 2, 2, 2], None, &quiet()).unwrap();
    assert_eq!(regions[0].bands[0].error, Some(0.0));
    assert_eq!(regions[1].bands[0].error, Some(0.0));
    assert_eq!(regions[1].bands[1].error, Some(12.0_f64.sqrt()));
    let clean = cube_sized(3, 2, vec![vec![1., 2., 3., 10., 20., 30.], vec![101., 102., 103., 110., 120., 130.]]);
    assert!(validate_cube(&clean).unwrap().warnings.iter().all(|w| !w.contains("ERR = 0")));
}

#[test]
fn ambiguous_mjy_is_refused_as_unrecognised_and_mjy_per_sr_as_surface_brightness() {
    let mut accepted = cube();
    accepted.header.set("BUNIT", "MJy/pixel".into());
    assert_eq!(validate_cube(&accepted).unwrap().unit, "MJy/pixel");
    for (unit, expected) in [("MJY/pixel", "not a recognised per-pixel flux density"), ("MJY", "not a recognised per-pixel flux density"), ("Mjy", "not a recognised per-pixel flux density"), ("kJy", "not a recognised per-pixel flux density"), ("MJy/sr", "surface brightness"), ("Jy/arcsec2", "surface brightness")] {
        let mut cube = cube();
        cube.header.set("BUNIT", unit.into());
        let error = validate_cube(&cube).unwrap_err().to_string();
        assert!(error.contains(expected) && error.contains(unit), "{unit}: {error}");
        assert_eq!(error.contains("Cube Prep cannot convert it either"), expected.starts_with("not"), "{unit}: {error}");
        assert_eq!(error.contains("MJy (exact case), the words milli, micro, nano and mega"), expected.starts_with("not"), "{unit}: {error}");
    }
    let mut counts = cube();
    counts.header.set("BUNIT", "ELECTRONS/S".into());
    counts.header.set("PHOTFLAM", "1E-19".into());
    let error = validate_cube(&counts).unwrap_err().to_string();
    assert!(error.contains("calibrated counts") && error.contains("PHOTFLAM") && error.contains("Cube Prep"), "{error}");
}

#[test]
fn create_run_dir_names_the_folder_in_its_error() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("afile.txt"), "x").unwrap();
    let blocked = dir.path().join("afile.txt").join("runs");
    let error = format!("{:#}", create_run_dir(blocked.to_str().unwrap(), "run").unwrap_err());
    assert!(error.contains("afile.txt") && error.contains("runs"), "{error}");
    let work = create_run_dir(dir.path().to_str().unwrap(), "run").unwrap();
    assert!(work.is_dir() && work.ends_with("sagui-run") && !work.to_string_lossy().starts_with(r"\\?\"));
    let again = format!("{:#}", create_run_dir(dir.path().to_str().unwrap(), "run").unwrap_err());
    assert!(again.contains("sagui-run") && again.contains("already exists"), "{again}");
}

#[test]
fn manifest_provenance_records_seed_and_clustering() {
    assert_eq!(SAGUI_SEED, 1);
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_for("input.fits", dir.path(), "Rscript", "library");
    let ward = parse_bridge_markers("ASTROBURST_CLUSTER\t185\t408480\t65536\n");
    let centre = clustering_provenance(&config, &ward);
    assert_eq!(centre, json!({ "cluster_pretransform": "none", "sed_scaling": "median_centre", "scale_fn": "sagui::median_scale (x - median(x))", "distance": "euclidean", "linkage": "ward.D2", "hclust_limit": 65536 }));
    config.sed_scaling = "median_mad".into();
    let mad = clustering_provenance(&config, &ward);
    assert_eq!(mad["sed_scaling"], "median_mad");
    assert!(mad["scale_fn"].as_str().unwrap().contains("MAD"), "{mad}");
    config.mode = "sparse".into();
    config.knn_k = 40;
    let sparse = clustering_provenance(&config, &parse_bridge_markers("ASTROBURST_CLUSTER\t185\tNA\tNA\n"));
    assert_eq!((sparse["linkage"].as_str(), sparse["knn_k"].as_u64(), sparse.get("hclust_limit"), sparse["distance"].as_str()), (Some("sparse Ward (kNN graph)"), Some(40), None, Some("euclidean")));
}

#[test]
fn region_extra_fields_default_deserialise_from_a_0_7_9_manifest_literal() {
    let region: Region = serde_json::from_str(r#"{"region":1,"n_pixels":91,"bands":[{"filter":"F814W","pivot_um":0.804553,"flux":2.1585090387077344e-05,"error":1.3534254511266251e-09}]}"#).unwrap();
    assert_eq!((region.n_components, region.min_snr, region.min_snr_lower_bound, region.bands[0].error_correlated), (0, None, None, None));
    let literal = r#"{"input_path":"a","output_dir":"b","rscript_path":"c","library_path":"d","n_regions":6,"starlet":true,"starlet_scales":[1,2,3],"starlet_threshold":3.0,"dq_exclude_bits":4294967295,"psf_target":"F444W","mode":"ward","knn_k":40}"#;
    let config: SaguiConfig = serde_json::from_str(literal).unwrap();
    assert_eq!(config.sed_scaling, "median_centre");
    assert!(serde_json::from_str::<SaguiConfig>(&literal.replace("\"knn_k\":40", "\"knn_k\":40,\"min_region_pixels\":1")).is_err());
    let explicit: SaguiConfig = serde_json::from_str(&literal.replace("\"knn_k\":40", "\"knn_k\":40,\"sed_scaling\":\"median_mad\"")).unwrap();
    assert_eq!(explicit.sed_scaling, "median_mad");
}

#[test]
fn validate_config_accepts_only_known_sed_scalings() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_for("input.fits", dir.path(), "Rscript", "library");
    validate_config(&config, 100).unwrap();
    config.sed_scaling = "median_mad".into();
    validate_config(&config, 100).unwrap();
    config.sed_scaling = "copula".into();
    assert!(validate_config(&config, 100).unwrap_err().to_string().contains("sed_scaling"));
}

#[test]
fn correlated_regional_error_sums_err_pairs_through_the_noise_autocorrelation() {
    let labels = [1, 1, 1, 2, 2, 2];
    let correlation = matched(vec![lateral(), delta(3)]);
    let regions = photometry(&cube(), &labels, Some(&correlation), &quiet()).unwrap();
    assert_eq!(regions[0].bands[0].error, Some(12.0_f64.sqrt()));
    assert!((regions[0].bands[0].error_correlated.unwrap() - 20.0_f64.sqrt()).abs() < 1e-12, "{:?}", regions[0].bands[0]);
    assert!((regions[1].bands[0].error_correlated.unwrap() - 20.0_f64.sqrt()).abs() < 1e-12);
    assert_eq!(regions[0].bands[1].error_correlated, Some(12.0_f64.sqrt()));
    let mut wide = Array2::from_elem((9, 9), 7.0_f32);
    for (y, x, weight) in [(4, 4, 1.0), (4, 3, 0.5), (4, 5, 0.5), (4, 2, 0.25), (4, 6, 0.25), (3, 4, 0.1), (5, 4, 0.1), (3, 3, 0.05), (3, 5, 0.05), (5, 3, 0.05), (5, 5, 0.05), (3, 2, 0.02), (3, 6, 0.02), (5, 2, 0.02), (5, 6, 0.02)] { wide[(y, x)] = weight; }
    let mixed_labels = [1, 1, 2, 1, 2, 2];
    let cropped = photometry(&cube(), &mixed_labels, Some(&matched(vec![wide.clone(), delta(9)])), &quiet()).unwrap();
    let err = &cube().err.unwrap()[0];
    for (index, region) in [(0, 1), (1, 2)] {
        let expected = brute_force_variance(&mixed_labels, 3, region, err, &wide).sqrt();
        assert!((cropped[index].bands[0].error_correlated.unwrap() - expected).abs() < 1e-12 * expected, "{:?} vs {expected}", cropped[index].bands[0]);
        assert_eq!(cropped[index].bands[1].error_correlated, cropped[index].bands[1].error);
    }
    let mut partly_matched = cube();
    partly_matched.filters[1].kernel = String::new();
    let resampling_only = NoiseCorrelation { source: NOISCORR_SOURCE_ALIGN.into(), ..matched(vec![lateral(), delta(3)]) };
    let regions = photometry(&partly_matched, &labels, Some(&resampling_only), &quiet()).unwrap();
    assert_eq!((regions[0].bands[0].error_correlated, regions[0].bands[1].error_correlated), (None, Some(12.0_f64.sqrt())));
    assert_eq!(regions[0].bands[0].error, Some(12.0_f64.sqrt()));
    let mut invalid = cube();
    invalid.err.as_mut().unwrap()[0][(0, 1)] = f32::NAN;
    let regions = photometry(&invalid, &labels, Some(&correlation), &quiet()).unwrap();
    assert_eq!((regions[0].bands[0].error, regions[0].bands[0].error_correlated), (None, None));
    assert!(regions[1].bands[0].error_correlated.is_some());
    let mut scaled = cube();
    scaled.header.set("BUNIT", "mJy/pixel".into());
    let regions = photometry(&scaled, &labels, Some(&correlation), &quiet()).unwrap();
    assert!((regions[0].bands[0].error_correlated.unwrap() - 20.0_f64.sqrt() * 1e-3).abs() < 1e-15);
    assert!(photometry(&cube(), &labels, None, &quiet()).unwrap().iter().all(|r| r.bands.iter().all(|b| b.error_correlated.is_none())));
}

#[test]
fn correlated_photometry_reports_each_region_and_band_and_stops_when_cancelled() {
    let correlation = matched(vec![lateral(), delta(3)]);
    let (progress, seen) = recording(5);
    photometry(&cube(), &[1, 1, 1, 2, 2, 2], Some(&correlation), &progress).unwrap();
    assert_eq!(*seen.lock().unwrap(), ["Measuring correlated regional error: region 1 of 2, F150W", "Measuring correlated regional error: region 1 of 2, F200W", "Measuring correlated regional error: region 2 of 2, F150W", "Measuring correlated regional error: region 2 of 2, F200W"]);
    let cancelled = quiet().with_cancel_source(std::sync::Arc::new(|| true));
    let error = photometry(&cube(), &[1, 1, 1, 2, 2, 2], Some(&correlation), &cancelled).unwrap_err().to_string();
    assert!(error.contains("cancelled") && error.contains("not published"), "{error}");
}

#[test]
fn noise_correlation_extension_is_read_when_well_formed_and_reported_when_absent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(path.to_str().unwrap(), &cube(), CubeLayout::Mef, "test").unwrap();
    let (_, info, correlation) = load_input(path.to_str().unwrap()).unwrap();
    assert!(correlation.is_none() && !info.has_noise_correlation);
    assert!(info.warnings.iter().any(|w| w.contains("NOISCORR") && w.contains("absent")), "{:?}", info.warnings);
    append_noise_correlation(&path, &[delta(3), delta(3)], 3, "match");
    let (_, info, correlation) = load_input(path.to_str().unwrap()).unwrap();
    let correlation = correlation.unwrap();
    assert_eq!((correlation.half, correlation.planes.len(), correlation.planes[1][(1, 1)], correlation.source.as_str()), (1, 2, 1.0, "match"));
    assert!(info.has_noise_correlation && info.warnings.iter().all(|w| !w.contains("absent")), "{:?}", info.warnings);
    assert!(info.warnings[0].contains("NCORRSRC='match'") && info.warnings[0].contains("3x3"), "{:?}", info.warnings);
    let malformed = dir.path().join("malformed.fits");
    crate::service::cubeprep::write_cube_file(malformed.to_str().unwrap(), &cube(), CubeLayout::Mef, "test").unwrap();
    append_noise_correlation(&malformed, &[delta(3), delta(3)], 5, "align");
    let (_, info, correlation) = load_input(malformed.to_str().unwrap()).unwrap();
    assert!(correlation.is_none() && !info.has_noise_correlation);
    assert!(info.warnings.iter().any(|w| w.contains("NOISCORR") && w.contains("NCORRK")), "{:?}", info.warnings);
    assert!(info.warnings.iter().all(|w| !w.contains("absent")), "{:?}", info.warnings);
    assert!(inspect_input(path.to_str().unwrap()).unwrap().has_noise_correlation);
    let unknown_source = dir.path().join("unknown-source.fits");
    crate::service::cubeprep::write_cube_file(unknown_source.to_str().unwrap(), &cube(), CubeLayout::Mef, "test").unwrap();
    append_noise_correlation(&unknown_source, &[delta(3), delta(3)], 3, "resample");
    let (_, info, correlation) = load_input(unknown_source.to_str().unwrap()).unwrap();
    assert!(correlation.is_none() && info.warnings.iter().any(|w| w.contains("NCORRSRC") && w.contains("resample")), "{:?}", info.warnings);
    let align_only = dir.path().join("align-only.fits");
    let mut partly_matched = cube();
    partly_matched.filters[1].kernel = String::new();
    crate::service::cubeprep::write_cube_file(align_only.to_str().unwrap(), &partly_matched, CubeLayout::Mef, "test").unwrap();
    append_noise_correlation(&align_only, &[delta(3), delta(3)], 3, "align");
    let (_, info, correlation) = load_input(align_only.to_str().unwrap()).unwrap();
    assert!(correlation.is_some() && info.has_noise_correlation);
    let withheld: Vec<&String> = info.warnings.iter().filter(|w| w.starts_with("error_correlated is left empty")).collect();
    assert_eq!(withheld.len(), 1, "{:?}", info.warnings);
    assert!(withheld[0].contains("F150W") && !withheld[0].contains("F200W") && withheld[0].contains("NCORRSRC='align'"), "{}", withheld[0]);
}

#[test]
fn noise_correlation_maps_must_hold_q00_one_to_float32_precision_and_finite_values() {
    let dir = tempfile::tempdir().unwrap();
    let read = |name: &str, planes: &[Array2<f32>]| {
        let path = dir.path().join(name);
        crate::service::cubeprep::write_cube_file(path.to_str().unwrap(), &cube(), CubeLayout::Mef, "test").unwrap();
        append_noise_correlation(&path, planes, 3, "match");
        let (_, info, correlation) = load_input(path.to_str().unwrap()).unwrap();
        (correlation, info.warnings)
    };
    let mut rounded = lateral();
    rounded[(1, 1)] = 1.0 + f32::EPSILON;
    let (accepted, warnings) = read("rounded.fits", &[delta(3), rounded]);
    assert!(accepted.is_some_and(|maps| maps.planes[1][(1, 0)] == 0.5), "{warnings:?}");
    let mut loose = delta(3);
    loose[(1, 1)] = 1.0005;
    let mut unnormalised = delta(3);
    unnormalised[(1, 1)] = 0.5;
    let mut undefined = lateral();
    undefined[(0, 2)] = f32::NAN;
    for (name, planes, bad) in [("loose.fits", [delta(3), loose], 1), ("unnormalised.fits", [unnormalised, delta(3)], 0), ("undefined.fits", [delta(3), undefined], 1)] {
        let (correlation, warnings) = read(name, &planes);
        assert!(correlation.is_none(), "{name}");
        assert!(warnings.iter().any(|w| w.starts_with("NOISCORR extension is malformed") && w.contains(&format!("plane {bad}")) && w.contains("error_correlated stays empty")), "{name}: {warnings:?}");
    }
}

#[test]
fn support_per_region_warning_uses_the_psf_target_noise_equivalent_area_only() {
    let cube = cube();
    let warning = support_per_region_warning(&cube, "F200W", 6, 2).unwrap();
    assert!(warning.contains("3.0 pixels per region") && warning.contains("17.5 pixels") && warning.contains("F200W") && warning.contains("fewer regions"), "{warning}");
    assert!(support_per_region_warning(&cube, "F200W", 100, 2).is_none());
    let mut wider = cube.clone();
    wider.filters[1].psf_fwhm_arcsec = Some(0.2);
    let unnamed = support_per_region_warning(&wider, "F999W", 6, 2).unwrap();
    assert!(unnamed.starts_with("The PSF-size check could not run") && unnamed.contains("PSF target F999W") && unnamed.contains("3.0 pixels per region") && !unnamed.contains("69.9"), "{unnamed}");
    let mut unscaled = cube.clone();
    unscaled.filters[1].pixscale_arcsec = None;
    let skipped = support_per_region_warning(&unscaled, "F200W", 6, 2).unwrap();
    assert!(skipped.starts_with("The PSF-size check could not run") && skipped.contains("PSF target F200W"), "{skipped}");
}

#[test]
fn support_per_region_check_reports_that_it_could_not_run_when_every_filters_psf_fwhm_is_nan() {
    let mut unknown = cube();
    for plane in &mut unknown.filters { plane.psf_fwhm_arcsec = None; plane.pixscale_arcsec = Some(0.04); }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(path.to_str().unwrap(), &unknown, CubeLayout::Mef, "test").unwrap();
    let (loaded, _, _) = load_input(path.to_str().unwrap()).unwrap();
    assert!(loaded.filters.iter().all(|plane| plane.psf_fwhm_arcsec.is_none() && plane.pixscale_arcsec == Some(0.04)), "{:?}", loaded.filters);
    let warning = support_per_region_warning(&loaded, "F200W", 185, 20).unwrap();
    assert!(warning.starts_with("The PSF-size check could not run") && warning.contains("PSF target F200W") && warning.contains(&format!("{:.1} pixels per region", 185.0 / 20.0)) && warning.contains("noise-equivalent area"), "{warning}");
    assert!(support_per_region_warning(&loaded, "F200W", 185, 2).unwrap().contains("92.5 pixels per region"));
}

#[test]
fn dq_exclusion_report_counts_finite_pixels_per_plane_and_bit() {
    let mut cube = cube();
    cube.dq.as_mut().unwrap().planes[0][(0, 0)] = 1;
    cube.dq.as_mut().unwrap().planes[1][(1, 1)] = 4 | 1;
    cube.dq.as_mut().unwrap().planes[0][(0, 2)] = 1;
    cube.planes[0][(0, 2)] = f32::NAN;
    let flags = dq_flag_counts(&cube);
    assert_eq!(flags.iter().map(|plane| (plane.filter.as_str(), plane.table)).collect::<Vec<_>>(), [("F150W", "jwst"), ("F200W", "jwst")]);
    let mut mixed = cube.clone();
    mixed.filters[0].dataset = "HST".into();
    mixed.dq.as_mut().unwrap().planes[0][(1, 0)] = 4;
    mixed.dq.as_mut().unwrap().planes[1][(1, 0)] = 4;
    assert_eq!((plane_dq_table(&mixed, &mixed.filters[0]), plane_dq_table(&mixed, &mixed.filters[1])), (DqTable::Hst, DqTable::Jwst));
    let mut unnamed = mixed.filters[1].clone();
    unnamed.dataset = String::new();
    assert_eq!(plane_dq_table(&mixed, &unnamed), DqTable::Unknown);
    let mut telescope = mixed.clone();
    telescope.header.set("TELESCOP", "'JWST'".into());
    assert_eq!(plane_dq_table(&telescope, &unnamed), DqTable::Jwst);
    let mixed_flags = dq_flag_counts(&mixed);
    assert_eq!((mixed_flags[0].table, mixed_flags[1].table), ("hst", "jwst"));
    assert_eq!(mixed_flags[0].flagged_finite, BTreeMap::from([("REED_SOLOMON".to_string(), 1), ("BAD_DETECTOR".to_string(), 1)]));
    assert_eq!(mixed_flags[1].flagged_finite, BTreeMap::from([("DO_NOT_USE".to_string(), 1), ("JUMP_DET".to_string(), 2)]));
    let (mixed_report, mixed_warnings) = dq_exclusion_report(&mixed, 4, &eligible(&mixed, 4));
    assert_eq!((mixed_report["planes"][0]["table"].as_str(), mixed_report["planes"][1]["table"].as_str(), mixed_report.get("table")), (Some("hst"), Some("jwst"), None));
    assert!(mixed_warnings[0].starts_with("F150W:") && mixed_warnings[0].contains("BAD_DETECTOR 1") && mixed_warnings[1].contains("JUMP_DET 2"), "{mixed_warnings:?}");
    assert_eq!(flags[0].flagged_finite, BTreeMap::from([("DO_NOT_USE".to_string(), 1)]));
    assert_eq!(flags[1].flagged_finite, BTreeMap::from([("DO_NOT_USE".to_string(), 1), ("JUMP_DET".to_string(), 1)]));
    let (report, warnings) = dq_exclusion_report(&cube, 1, &eligible(&cube, 1));
    assert_eq!((report["finite_pixels"].as_u64(), report["excluded_pixels"].as_u64(), report["planes"][0]["table"].as_str()), (Some(5), Some(2), Some("jwst")));
    assert_eq!(report["planes"][0]["removed_pixels"], 1);
    assert_eq!(report["planes"][1]["flags"]["DO_NOT_USE"], 1);
    assert!(report["planes"][1]["flags"].get("JUMP_DET").is_none(), "{report}");
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].starts_with("F150W: DQ policy 0x00000001 removes 1 finite pixel") && warnings[0].contains("DO_NOT_USE 1"), "{}", warnings[0]);
    let (report, warnings) = dq_exclusion_report(&cube, 4, &eligible(&cube, 4));
    assert_eq!(report["excluded_pixels"], 1);
    assert_eq!(warnings.len(), 1);
    let mut plain_cube = cube.clone();
    plain_cube.dq = None;
    assert!(dq_flag_counts(&plain_cube).is_empty());
    assert_eq!(dq_exclusion_report(&plain_cube, 1, &eligible(&plain_cube, 1)).0["planes"], json!([]));
}

#[test]
fn region_components_and_snr_are_derived_from_the_label_map() {
    let labels = [1, 0, 2, 0, 1, 0, 2, 0, 1];
    assert_eq!(component_sizes(&labels, 3, 1), [3]);
    assert_eq!(component_sizes(&labels, 3, 2), [1, 1]);
    assert_eq!(component_sizes(&[1, 1, 0, 0, 0, 0, 0, 0, 1], 3, 1), [2, 1]);
    let cube = cube_sized(3, 3, vec![vec![1., 2., 3., 4., 5., 6., 7., 8., 9.], vec![10., 20., 30., 40., 50., 60., 70., 80., 90.]]);
    let regions = photometry(&cube, &labels, None, &quiet()).unwrap();
    assert_eq!((regions[0].n_components, regions[1].n_components), (1, 2));
    assert_eq!(regions[0].min_snr, Some(15.0 / 12.0_f64.sqrt()));
    assert_eq!(regions[0].min_snr_lower_bound, Some(15.0 / 12.0_f64.sqrt() / 3.0_f64.sqrt()));
    assert_eq!(fragment_warning(&labels, 3).as_deref(), Some("Region 2 contains 1-pixel fragments (8-connected); SAGUI groups pixels by spectrum, not by position"));
    assert_eq!(fragment_warning(&[2, 0, 1, 0, 0, 0, 0, 0, 0], 3).as_deref(), Some("Regions 1, 2 contain 1-pixel fragments (8-connected); SAGUI groups pixels by spectrum, not by position"));
    let mut unknown = cube.clone();
    unknown.err.as_mut().unwrap()[0][(0, 0)] = f32::NAN;
    let regions = photometry(&unknown, &labels, None, &quiet()).unwrap();
    assert_eq!((regions[0].bands[0].error, regions[0].min_snr, regions[0].min_snr_lower_bound), (None, None, None));
    assert_eq!(regions[1].min_snr, Some(10.0 / 8.0_f64.sqrt()));
    let mut zero = cube.clone();
    zero.err.as_mut().unwrap()[0] = Array2::zeros((3, 3));
    let regions = photometry(&zero, &labels, None, &quiet()).unwrap();
    assert_eq!(regions[0].min_snr, Some(150.0 / 12.0_f64.sqrt()));
    let mut negative = zero.clone();
    negative.planes[0].mapv_inplace(|value| -value);
    let regions = photometry(&negative, &labels, None, &quiet()).unwrap();
    assert_eq!((regions[0].bands[0].flux, regions[0].bands[0].error, regions[0].min_snr, regions[0].min_snr_lower_bound), (-15.0, Some(0.0), None, None));
    let mut silent = zero.clone();
    silent.err.as_mut().unwrap()[1] = Array2::zeros((3, 3));
    assert!(photometry(&silent, &labels, None, &quiet()).unwrap().iter().all(|r| r.min_snr.is_none()));
    zero.err = None;
    assert!(photometry(&zero, &labels, None, &quiet()).unwrap().iter().all(|r| r.min_snr.is_none()));
    assert!(fragment_warning(&[1, 1, 1, 2, 2, 2, 0, 0, 0], 3).is_none());
}

#[test]
fn bridge_markers_are_parsed_from_stdout() {
    let text = "ASTROBURST_R\t4.6.2\t0.1\tabc\nASTROBURST_CLUSTER\t185\t408480\t65536\nASTROBURST_WARNING\tmay page\nASTROBURST_ERROR\tNo valid spectra remain after masking.\nASTROBURST_REFUSED\tExact Ward in R is limited to 65536 clustered pixels\n";
    let report = parse_bridge_markers(text);
    assert_eq!((report.clustered_pixels, report.clustering_memory_bytes, report.hclust_limit), (Some(185), Some(408480), Some(65536)));
    assert_eq!(report.warnings, ["may page"]);
    let sparse = parse_bridge_markers("ASTROBURST_CLUSTER\t185\tNA\tNA\n");
    assert_eq!((sparse.clustered_pixels, sparse.clustering_memory_bytes, sparse.hclust_limit), (Some(185), None, None));
    assert!(parse_bridge_markers("").warnings.is_empty());
}

#[test]
fn backend_tsv_is_parsed_into_the_manifest() {
    let value = parse_backend("knn_k\trequested_Ncomp\tactual_Ncomp\tdisconnected\tvalid_pixels\n20\t3\t3\tFALSE\t391\n").unwrap();
    assert_eq!(value, json!({ "knn_k_used": 20, "requested_regions": 3, "actual_regions": 3, "disconnected": false, "valid_pixels": 391 }));
    assert!(parse_backend("bad").is_err());
    assert!(parse_backend("knn_k\trequested_Ncomp\tactual_Ncomp\tdisconnected\tvalid_pixels\n20\t3\n").is_err());
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
    let result = photometry(&cube, &labels, None, &quiet()).unwrap();
    assert_eq!(result.iter().map(|r| (r.region, r.n_pixels)).collect::<Vec<_>>(), [(1, 3), (2, 2)]);
    assert_eq!(result[0].bands[0].flux, 53.0);
    assert_eq!(result[0].bands[1].flux, 353.0);
    assert_eq!(result[0].bands[0].error, Some(12.0_f64.sqrt()));
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
        cube.err.as_mut().unwrap()[0][(1, 1)] = invalid;
        let result = photometry(&cube, &labels, None, &quiet()).unwrap();
        assert_eq!(result[0].bands[0].error, None);
        assert_eq!(result[1].bands[0].error, Some(8.0_f64.sqrt()));
        assert_eq!(result[0].bands[0].flux, 53.0);
    }
    cube.err = None;
    assert!(photometry(&cube, &labels, None, &quiet()).unwrap().iter().all(|r| r.bands.iter().all(|b| b.error.is_none())));
}

#[test]
fn regional_flux_and_errors_are_converted_from_mjy_per_pixel_to_jy() {
    let mut cube = cube();
    cube.header.set("BUNIT", "mJy/pixel".into());
    let regions = photometry(&cube, &[1, 0, 0, 1, 0, 0], None, &quiet()).unwrap();
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
    assert_eq!(info.dq_flags.len(), 2);
    assert!(info.dq_flags.iter().all(|plane| plane.flagged_finite.is_empty()));
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
    for (labels, mask) in [(vec![1, 2, 0], vec![1, 1, 1]), (vec![1, 0, 3], vec![1, 0, 1]), (vec![1, 0, 1], vec![1, 0, 1]), (vec![1, 0, 2], vec![1, 0, 0]), (vec![1, 0, 2], vec![1, 0, 2]), (vec![1, 0, 2], vec![1, 1, 1])] { assert!(validate_maps(&labels, &mask, &[true, false, true], 2).is_err(), "{labels:?} {mask:?}"); }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("labels.bin");
    fs::write(&path, [1, 0, 0, 0, 2]).unwrap();
    assert!(read_integers(&path, 1).is_err());
    assert!(format!("{:#}", read_integers(&dir.path().join("absent.bin"), 1).unwrap_err()).contains("absent.bin"));
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
    assert_eq!(footprint_memory(1000, 4, "ward", 40), 1000 * 4 * 64 + 128 * 1024 * 1024);
    assert_eq!(footprint_memory(1000, 4, "sparse", 40), 1000 * 4 * 64 + 1000 * 40 * 128 + 128 * 1024 * 1024);
    assert_eq!(footprint_memory(u64::MAX, 4, "sparse", 40), u64::MAX);
    for id in ["../escape", "a/b", "a\\b", "", "a b", "a:stream"] { assert!(!valid_run_id(id)); }
    assert!(valid_run_id("run_2026-11-02"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known.txt");
    fs::write(&path, "abc").unwrap();
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
    assert_eq!(sha256(&path, &progress).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let missing = dir.path().join("moved.fits");
    let mut config = config_for(missing.to_str().unwrap(), dir.path(), "Rscript", "library");
    config.output_dir = dir.path().to_string_lossy().into_owned();
    let error = format!("{:#}", run(config, "missing-input".into(), &progress).unwrap_err());
    assert!(error.contains("moved.fits") && error.contains("input cube"), "{error}");
}

#[test]
fn input_hash_errors_name_the_cube_but_let_cancellation_through() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("moved.fits");
    let error = format!("{:#}", hash_input(&missing, &quiet(), "is not readable").unwrap_err());
    assert!(error.starts_with("The input cube ") && error.contains("moved.fits") && error.contains("is not readable"), "{error}");
    let present = dir.path().join("present.fits");
    fs::write(&present, "abc").unwrap();
    assert_eq!(hash_input(&present, &quiet(), "is not readable").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let cancelled = quiet().with_cancel_source(std::sync::Arc::new(|| true));
    assert_eq!(format!("{:#}", hash_input(&present, &cancelled, "is no longer readable; results were not published").unwrap_err()), "SAGUI cancelled during integrity verification");
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

const REFERENCE_SCRIPT: &str = "args <- commandArgs(trailingOnly = TRUE)\n.libPaths(c(args[[1]], .Library), include.site = FALSE)\nsetwd(args[[2]])\nnx <- as.integer(args[[3]])\nny <- as.integer(args[[4]])\nnb <- as.integer(args[[5]])\ncube <- array(readBin('science.bin', what = 'double', n = nx * ny * nb, size = 8, endian = 'little'), c(nx, ny, nb))\nscales <- as.integer(strsplit(args[[8]], ',', fixed = TRUE)[[1]])\nparams <- list(input = cube, Ncomp = as.integer(args[[6]]), use_starlet_mask = identical(args[[7]], 'true'), starlet_J = max(scales), starlet_scales = scales, denoise_k = as.numeric(args[[9]]))\nif (identical(args[[12]], 'median_mad')) params['scale_fn'] <- list(NULL)\nset.seed(1L)\nif (args[[10]] == 'sparse') { params$knn_k <- as.integer(args[[11]]); params$auto_k <- FALSE; fit <- do.call(sagui::segment_regions_large, params) } else { fit <- do.call(sagui::segment_regions, params) }\nlabels <- as.integer(fit$cluster_map)\nlabels[is.na(labels)] <- 0L\nwriteBin(labels, 'reference-labels.bin', size = 4, endian = 'little')\nwriteBin(as.integer(fit$mask), 'reference-mask.bin', size = 4, endian = 'little')\n";

fn live_environment() -> (String, String) {
    (std::env::var("ASTROBURST_SAGUI_RSCRIPT").expect("ASTROBURST_SAGUI_RSCRIPT"), std::env::var("ASTROBURST_SAGUI_LIBRARY").expect("ASTROBURST_SAGUI_LIBRARY"))
}

fn direct_reference(rscript: &str, library: &str, work: &Path, config: &SaguiConfig, width: usize, height: usize, bands: usize) -> (Vec<u32>, Vec<u32>) {
    fs::write(work.join("reference.R"), REFERENCE_SCRIPT).unwrap();
    let mut command = std::process::Command::new(rscript);
    command.arg("--vanilla").arg(work.join("reference.R")).arg(library).arg(work)
        .arg(width.to_string()).arg(height.to_string()).arg(bands.to_string()).arg(config.n_regions.to_string()).arg(config.starlet.to_string())
        .arg(config.starlet_scales.iter().map(usize::to_string).collect::<Vec<_>>().join(",")).arg(config.starlet_threshold.to_string())
        .arg(&config.mode).arg(config.knn_k.to_string()).arg(&config.sed_scaling);
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    let logs = tempfile::tempdir().unwrap();
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 1);
    runtime::execute(command, logs.path(), &progress, None).unwrap();
    let pixels = width * height;
    (read_integers(&work.join("reference-labels.bin"), pixels).unwrap(), read_integers(&work.join("reference-mask.bin"), pixels).unwrap())
}

fn assert_same_partition(ours: &[u32], reference: &[u32]) {
    assert_eq!(ours.len(), reference.len());
    let mut pairing = BTreeMap::new();
    for (&label, &expected) in ours.iter().zip(reference) {
        assert_eq!(label == 0, expected == 0);
        if label > 0 { assert_eq!(*pairing.entry(label).or_insert(expected), expected); }
    }
    assert_eq!(pairing.values().collect::<HashSet<_>>().len(), pairing.len());
}

fn wide_field(width: usize, height: usize, bands: usize, sources: &[(f64, f64, f64)]) -> Cube {
    let mut state = 0x9e3779b97f4a7c15_u64;
    let mut noise = move || { state ^= state << 13; state ^= state >> 7; state ^= state << 17; (state >> 11) as f64 / (1_u64 << 53) as f64 - 0.5 };
    let planes = (0..bands).map(|b| (0..width * height).map(|i| {
        let (x, y) = ((i % width) as f64, (i / width) as f64);
        let signal: f64 = sources.iter().map(|&(sx, sy, amplitude)| amplitude * (1.0 + 0.3 * b as f64) * (-((x - sx).powi(2) + (y - sy).powi(2)) / 18.0).exp()).sum();
        (0.01 + signal + 0.002 * noise()) as f32
    }).collect()).collect();
    let mut cube = cube_sized(width, height, planes);
    cube.err = Some(vec![Array2::from_elem((height, width), 0.001); bands]);
    cube
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn official_sagui_matches_direct_non_square_segmentation_and_regional_sed() {
    let (rscript, library) = live_environment();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(path.to_str().unwrap(), &cube(), CubeLayout::Mef, "test").unwrap();
    append_noise_correlation(&path, &[delta(3), delta(3)], 3, "match");
    let mut config = config_for(path.to_str().unwrap(), dir.path(), &rscript, &library);
    config.psf_target = "known synthetic PSF".into();
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
        assert_eq!(band.error_correlated, band.error);
        assert_eq!(region.n_pixels, fields[4].parse::<usize>().unwrap());
    }
    let file = File::open(&result.labels_path).unwrap();
    let header = extract_header_by_index(&file, 1).unwrap();
    assert_eq!((header.get_i64("NAXIS1"), header.get_i64("NAXIS2")), (Some(3), Some(2)));
    assert!(same_grid(&cube().wcs().unwrap(), (2, 3), &header_wcs(&header).unwrap(), (2, 3)));
    assert!(!result.output_dir.starts_with(r"\\?\"));
    assert!(fs::read_to_string(&result.table_path).unwrap().starts_with("region,n_pixels,filter,pivot_um,flux,error,error_correlated_jy,unit\n"));
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&result.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["status"], "complete");
    assert_eq!(manifest["seed"], 1);
    assert_eq!(manifest["clustering"]["linkage"], "ward.D2");
    assert_eq!(manifest["clustered_pixels"], 6);
    assert_eq!(manifest["map_counts"]["support_pixels"], 6);
    assert_eq!((manifest["noise_correlation"]["size"].as_u64(), manifest["noise_correlation"]["source"].as_str(), manifest["noise_correlation"]["withheld"].as_array().map(Vec::len)), (Some(3), Some("match"), Some(0)));
    assert!(manifest["input"]["path"].as_str().unwrap().starts_with(r"\\?\") == false);
    assert_eq!(read_integers(&Path::new(&result.output_dir).join("support-upstream.bin"), 6).unwrap(), vec![1; 6]);
    for file in manifest["files"].as_array().unwrap() {
        assert_eq!(sha256(&Path::new(&result.output_dir).join(file["path"].as_str().unwrap()), &progress).unwrap(), file["sha256"].as_str().unwrap());
    }
    assert!(manifest["files"].as_array().unwrap().iter().any(|file| file["path"] == "support-upstream.bin"));
    assert!(result.warnings.iter().any(|w| w.contains("pixels per region")), "{:?}", result.warnings);
    let reopened = saved::load(&result.manifest_path).unwrap();
    assert_eq!(reopened.result.labels, result.labels);
    assert_eq!(reopened.result.unit, "Jy");
    assert_eq!(reopened.result.regions[0].bands[0].flux, result.regions[0].bands[0].flux);
    assert_eq!(reopened.result.regions[0].bands[0].error_correlated, result.regions[0].bands[0].error_correlated);
    fs::write(&result.table_path, "modified").unwrap();
    assert!(saved::load(&result.manifest_path).unwrap_err().to_string().contains("changed"));
}

fn fixture_23x17() -> Cube {
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
    cube
}

fn starlet_config(input: &Path, output: &Path, rscript: &str, library: &str, mode: &str) -> SaguiConfig {
    let mut config = config_for(input.to_str().unwrap(), output, rscript, library);
    config.n_regions = 3;
    config.starlet = true;
    config.starlet_scales = vec![1, 2, 3];
    config.starlet_threshold = 1.5;
    config.psf_target = "known synthetic PSF".into();
    config.mode = mode.into();
    config.knn_k = 20;
    config
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn official_sagui_sparse_and_starlet_match_direct_reference() {
    let (rscript, library) = live_environment();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(input.to_str().unwrap(), &fixture_23x17(), CubeLayout::Mef, "test").unwrap();
    for mode in ["ward", "sparse"] {
        let config = starlet_config(&input, dir.path(), &rscript, &library, mode);
        let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 5);
        let result = run(config.clone(), mode.into(), &progress).unwrap();
        let work = Path::new(&result.output_dir);
        let (labels, mask) = direct_reference(&rscript, &library, work, &config, 23, 17, 2);
        assert_eq!(mask, read_integers(&work.join("support.bin"), 391).unwrap());
        assert_eq!(mask, read_integers(&work.join("support-upstream.bin"), 391).unwrap());
        assert_same_partition(&result.labels, &labels);
        let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&result.manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["clustered_pixels"].as_u64().unwrap() as usize, mask.iter().filter(|&&m| m == 1).count());
        if mode == "sparse" {
            assert_eq!(manifest["backend"]["knn_k_used"], 20);
            assert_eq!(manifest["backend"]["disconnected"], false);
            assert_eq!((manifest["clustering"]["linkage"].as_str(), manifest["clustering"]["knn_k"].as_u64(), manifest["clustering"].get("hclust_limit"), manifest["clustering_memory_bytes"].is_null()), (Some("sparse Ward (kNN graph)"), Some(20), None, true));
        } else {
            assert!(manifest["backend"].is_null());
            assert_eq!((manifest["clustering"]["linkage"].as_str(), manifest["clustering"]["hclust_limit"].as_u64()), (Some("ward.D2"), Some(65536)));
        }
    }
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn median_mad_scaling_matches_a_direct_scale_fn_null_call_and_is_recorded() {
    let (rscript, library) = live_environment();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(input.to_str().unwrap(), &fixture_23x17(), CubeLayout::Mef, "test").unwrap();
    let mut config = starlet_config(&input, dir.path(), &rscript, &library, "ward");
    config.sed_scaling = "median_mad".into();
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 5);
    let result = run(config.clone(), "median-mad".into(), &progress).unwrap();
    let work = Path::new(&result.output_dir);
    let (labels, _) = direct_reference(&rscript, &library, work, &config, 23, 17, 2);
    assert_same_partition(&result.labels, &labels);
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&result.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["clustering"]["sed_scaling"], "median_mad");
    assert_eq!(manifest["config"]["sed_scaling"], "median_mad");
    let reopened = saved::load(&result.manifest_path).unwrap();
    assert_eq!(reopened.config.sed_scaling, "median_mad");
    let mut centred = config.clone();
    centred.sed_scaling = "median_centre".into();
    let centred_result = run(centred, "median-centre".into(), &progress).unwrap();
    assert_ne!(centred_result.labels, result.labels);
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn sparse_knn_2_reports_the_disconnected_graph() {
    let (rscript, library) = live_environment();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(input.to_str().unwrap(), &fixture_23x17(), CubeLayout::Mef, "test").unwrap();
    let mut config = starlet_config(&input, dir.path(), &rscript, &library, "sparse");
    config.knn_k = 2;
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 5);
    let error = format!("{:#}", run(config, "knn-2".into(), &progress).unwrap_err());
    eprintln!("knn_k 2 failure: {error}");
    assert!(error.contains("disconnected") && error.contains("nearest-neighbour"), "{error}");
    assert!(error.starts_with("Rscript failed (exit code: 1): ") && error.contains("; full log: ") && error.ends_with("stderr.log"), "{error}");
    let work = dir.path().join("sagui-knn-2");
    assert!(work.join("failure.json").exists() && work.join("backend.tsv").exists() && !work.join("manifest.json").exists());
    assert!(fs::read_to_string(work.join("stdout.log")).unwrap().lines().any(|line| line.starts_with("ASTROBURST_ERROR\t")));
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn exact_ward_refuses_more_than_65536_support_pixels_before_dist() {
    let (rscript, library) = live_environment();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.fits");
    crate::service::cubeprep::write_cube_file(input.to_str().unwrap(), &wide_field(300, 300, 2, &[(150.0, 150.0, 1.0)]), CubeLayout::Mef, "test").unwrap();
    let mut config = config_for(input.to_str().unwrap(), dir.path(), &rscript, &library);
    config.n_regions = 3;
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 5);
    let error = format!("{:#}", run(config, "too-many".into(), &progress).unwrap_err());
    eprintln!("300x300 refusal: {error}");
    assert!(error.contains("65536") && error.contains("90000") && error.contains("sparse Ward") && error.contains("upper bound"), "{error}");
    let work = dir.path().join("sagui-too-many");
    assert!(work.join("failure.json").exists() && !work.join("labels.bin").exists());
    let stdout = fs::read_to_string(work.join("stdout.log")).unwrap();
    assert!(stdout.lines().any(|line| line.starts_with("ASTROBURST_CLUSTER\t90000\t")) && stdout.lines().any(|line| line.starts_with("ASTROBURST_REFUSED\t")), "{stdout}");
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn exact_ward_accepts_a_400x400_field_whose_starlet_support_is_small() {
    let (rscript, library) = live_environment();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.fits");
    let cube = wide_field(400, 400, 3, &[(80.0, 90.0, 1.0), (300.0, 120.0, 0.6), (150.0, 310.0, 0.8), (330.0, 330.0, 0.5)]);
    crate::service::cubeprep::write_cube_file(input.to_str().unwrap(), &cube, CubeLayout::Mef, "test").unwrap();
    let mut config = config_for(input.to_str().unwrap(), dir.path(), &rscript, &library);
    config.n_regions = 4;
    config.starlet = true;
    config.starlet_scales = vec![1, 2, 3];
    config.starlet_threshold = 3.0;
    let (progress, stages) = recording(5);
    let result = run(config.clone(), "wide".into(), &progress).unwrap();
    let stages = stages.lock().unwrap().clone();
    assert!(stages.iter().any(|stage| stage.starts_with("Running official SAGUI in isolated R - R running ")), "{stages:?}");
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&result.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["clustering"]["hclust_limit"], 65536);
    let clustered = manifest["clustered_pixels"].as_u64().unwrap();
    eprintln!("400x400 exact Ward: clustered_pixels={clustered} clustering_memory_bytes={} estimated_memory_bytes={} map_counts={}", manifest["clustering_memory_bytes"], manifest["estimated_memory_bytes"], manifest["map_counts"]);
    assert!(clustered > 0 && clustered < 65536, "{clustered}");
    assert!(manifest["estimated_memory_bytes"].as_u64().unwrap() > 12 * 160000 * 159999, "{}", manifest["estimated_memory_bytes"]);
    assert_eq!(manifest["clustering_memory_bytes"].as_u64().unwrap(), 12 * clustered * (clustered - 1));
    let work = Path::new(&result.output_dir);
    let (labels, mask) = direct_reference(&rscript, &library, work, &config, 400, 400, 3);
    assert_eq!(mask, read_integers(&work.join("support.bin"), 160000).unwrap());
    assert_same_partition(&result.labels, &labels);
}

#[test]
#[ignore = "Requires pinned R, the SAGUI private library and the saved D1 job under entregas/desktop-release"]
fn saved_d1_job_reruns_with_byte_identical_labels_and_support() {
    let (rscript, library) = live_environment();
    let saved = Path::new(env!("CARGO_MANIFEST_DIR")).join("../entregas/desktop-release/sagui-20261102095201012-qxlu6dq");
    let mut config: SaguiConfig = serde_json::from_slice(&fs::read(saved.join("config.json")).unwrap()).unwrap();
    assert_eq!(config.sed_scaling, "median_centre");
    config.rscript_path = rscript;
    config.library_path = library;
    let dir = tempfile::tempdir().unwrap();
    config.output_dir = dir.path().to_string_lossy().into_owned();
    let progress = ProgressHandle::with_sink(std::sync::Arc::new(|_| {}), 5);
    let result = run(config, "d1-rerun".into(), &progress).unwrap();
    let work = Path::new(&result.output_dir);
    assert_eq!(fs::read(work.join("labels.bin")).unwrap(), fs::read(saved.join("labels.bin")).unwrap());
    assert_eq!(fs::read(work.join("support.bin")).unwrap(), fs::read(saved.join("support.bin")).unwrap());
    assert_eq!(fs::read(work.join("support-upstream.bin")).unwrap(), fs::read(saved.join("support.bin")).unwrap());
    let saved_manifest: serde_json::Value = serde_json::from_slice(&fs::read(saved.join("manifest.json")).unwrap()).unwrap();
    let saved_regions: Vec<Region> = serde_json::from_value(saved_manifest["result"]["regions"].clone()).unwrap();
    assert_eq!(result.regions.len(), saved_regions.len());
    for (ours, theirs) in result.regions.iter().zip(&saved_regions) {
        assert_eq!((ours.region, ours.n_pixels), (theirs.region, theirs.n_pixels));
        for (band, expected) in ours.bands.iter().zip(&theirs.bands) {
            assert_eq!((band.flux, band.error, band.error_correlated), (expected.flux, expected.error, None));
        }
    }
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&result.manifest_path).unwrap()).unwrap();
    eprintln!("D1 rerun: clustered_pixels={} map_counts={} dq_exclusion.excluded_pixels={} warnings={:?}", manifest["clustered_pixels"], manifest["map_counts"], manifest["dq_exclusion"]["excluded_pixels"], result.warnings);
    assert_eq!(manifest["clustered_pixels"], 185);
    assert_eq!(manifest["map_counts"]["support_pixels"], 185);
    assert_eq!(manifest["map_counts"]["upstream_support_on_excluded"], 0);
    assert_eq!(manifest["dq_exclusion"]["planes"].as_array().unwrap().iter().map(|plane| plane["table"].as_str().unwrap()).collect::<Vec<_>>(), ["hst", "jwst", "jwst", "jwst", "jwst", "jwst"]);
    assert!(result.warnings.iter().any(|w| w.starts_with("The PSF-size check could not run") && w.contains("PSF target F444W") && w.contains("30.8 pixels per region")), "{:?}", result.warnings);
    let reopened = saved::load(saved.join("manifest.json").to_str().unwrap()).unwrap();
    assert_eq!(reopened.result.labels, result.labels);
    assert_eq!(reopened.config.sed_scaling, "median_centre");
    let summary = |regions: &[Region]| regions.iter().map(|r| (r.n_components, r.min_snr, r.min_snr_lower_bound)).collect::<Vec<_>>();
    assert_eq!(summary(&reopened.result.regions), summary(&result.regions));
    assert!(result.regions.iter().all(|r| r.n_components >= 1 && r.min_snr.is_some()));
    assert!(!reopened.result.output_dir.starts_with(r"\\?\"));
}

#[test]
#[ignore = "Requires pinned R and SAGUI private library"]
fn excluded_pixels_marked_by_the_upstream_mask_leave_the_published_support() {
    let (rscript, library) = live_environment();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.fits");
    let mut cube = fixture_23x17();
    cube.planes[0][(1, 1)] = f32::NAN;
    cube.dq.as_mut().unwrap().planes[1][(15, 21)] = 1;
    crate::service::cubeprep::write_cube_file(input.to_str().unwrap(), &cube, CubeLayout::Mef, "test").unwrap();
    let mut config = starlet_config(&input, dir.path(), &rscript, &library, "ward");
    config.starlet = false;
    let result = run(config.clone(), "holes".into(), &quiet()).unwrap();
    let work = Path::new(&result.output_dir);
    let valid = eligible(&cube, config.dq_exclude_bits);
    let (labels, mask) = direct_reference(&rscript, &library, work, &config, 23, 17, 2);
    let support = read_integers(&work.join("support.bin"), 391).unwrap();
    assert_eq!(read_integers(&work.join("support-upstream.bin"), 391).unwrap(), mask);
    assert_eq!(support, mask.iter().zip(&valid).map(|(&marked, &kept)| u32::from(marked == 1 && kept)).collect::<Vec<_>>());
    assert_eq!(mask.iter().zip(&valid).filter(|&(&marked, &kept)| marked == 1 && !kept).count(), 2, "{mask:?}");
    assert_same_partition(&result.labels, &labels);
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&result.manifest_path).unwrap()).unwrap();
    assert_eq!((manifest["map_counts"]["upstream_support_on_excluded"].as_u64(), manifest["map_counts"]["excluded_by_policy"].as_u64(), manifest["map_counts"]["support_on_excluded"].as_u64()), (Some(2), Some(2), Some(0)));
    assert!(result.warnings.iter().any(|w| w.starts_with("2 excluded pixels were marked as support by SAGUI's upstream mask")), "{:?}", result.warnings);
    let published = crate::infra::fits::reader::extract_int_plane_by_index(&File::open(&result.mask_path).unwrap(), 1).unwrap();
    assert!(published.bits.iter().eq(support.iter()));
    assert!(saved::load(&result.manifest_path).is_ok());
}

#[test]
#[ignore = "Requires local Rscript"]
fn r_process_cancellation_stops_the_r_work_before_it_finishes() {
    let rscript = std::env::var("ASTROBURST_SAGUI_RSCRIPT").expect("ASTROBURST_SAGUI_RSCRIPT");
    let dir = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(rscript);
    command.args(["--vanilla", "-e", "writeLines(as.character(Sys.getpid()),'pid.txt');Sys.sleep(3);writeLines('finished','marker.txt')"]).current_dir(dir.path());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    let pid = dir.path().join("pid.txt");
    let start = std::time::Instant::now();
    let progress = quiet().with_cancel_source(std::sync::Arc::new(move || pid.exists()));
    let error = runtime::execute(command, dir.path(), &progress, None).unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error:#}");
    assert!(start.elapsed().as_secs() < 10);
    std::thread::sleep(std::time::Duration::from_secs(5));
    assert!(dir.path().join("pid.txt").exists() && !dir.path().join("marker.txt").exists());
}
