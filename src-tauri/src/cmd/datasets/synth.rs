use serde::{Deserialize, Serialize};
use tauri::command;

use crate::cmd::common::write_derived_fits;
use crate::core::synth::pipeline::{self, SynthConfig, SynthResult};

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateSynthArgs {
    pub config: SynthConfig,
    pub output_path: String,
    pub save_catalog: bool,
    pub catalog_path: Option<String>,
    pub save_ground_truth: bool,
    pub ground_truth_path: Option<String>,
}

fn validate_output_paths(args: &GenerateSynthArgs) -> anyhow::Result<()> {
    let catalog = args.catalog_path.as_deref().filter(|_| args.save_catalog);
    let truth = args.ground_truth_path.as_deref().filter(|_| args.save_ground_truth);
    if catalog == Some(args.output_path.as_str()) {
        anyhow::bail!("Catalog path {} is the same as the output path; choose a different catalog file name.", args.output_path);
    }
    if truth == Some(args.output_path.as_str()) {
        anyhow::bail!("Ground truth path {} is the same as the output path; choose a different ground truth file name.", args.output_path);
    }
    if let (Some(catalog), Some(truth)) = (catalog, truth) {
        if catalog == truth {
            anyhow::bail!("Catalog path and ground truth path are both {}; choose different file names.", catalog);
        }
    }
    Ok(())
}

#[command]
pub async fn generate_synth_cmd(args: GenerateSynthArgs) -> Result<SynthResult, String> {
    validate_output_paths(&args).map_err(|e| format!("{:#}", e))?;
    let config = args.config;
    let header = pipeline::synth_header(&config, 0, None);
    let truth_header = pipeline::ground_truth_header(&config);
    let noise = config.noise.clone();

    let (noisy, ground_truth, stars) =
        tokio::task::spawn_blocking(move || pipeline::generate(&config))
            .await
            .map_err(|e| format!("Task failed: {}", e))?
            .map_err(|e| format!("{:#}", e))?;

    write_derived_fits(&args.output_path, &noisy, Some(&header))
        .map_err(|e| format!("Failed to save FITS: {}", e))?;

    if args.save_ground_truth {
        if let Some(gt_path) = &args.ground_truth_path {
            write_derived_fits(gt_path, &ground_truth, Some(&truth_header))
                .map_err(|e| format!("Failed to save ground truth: {}", e))?;
        }
    }

    if args.save_catalog {
        if let Some(cat_path) = &args.catalog_path {
            pipeline::save_catalog(&stars, &noise, cat_path)
                .map_err(|e| format!("Failed to save catalog: {}", e))?;
        }
    }

    Ok(SynthResult {
        width: noisy.dim().1 as u32,
        height: noisy.dim().0 as u32,
        star_count: stars.len(),
        output_path: Some(args.output_path),
        frames_manifest_path: None,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateStackArgs {
    pub config: SynthConfig,
    pub output_dir: String,
    pub prefix: String,
    #[serde(default)]
    pub save_catalog: bool,
    #[serde(default)]
    pub save_ground_truth: bool,
}

fn is_frame_file_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('_'))
        .and_then(|rest| rest.strip_suffix(".fits"))
        .is_some_and(|index| index.len() >= 4 && index.bytes().all(|b| b.is_ascii_digit()))
}

fn earliest_existing_frame(dir: &std::path::Path, prefix: &str) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| is_frame_file_name(name, prefix))
        .min()
}

fn validate_output_dir(output_dir: &str, prefix: &str) -> anyhow::Result<()> {
    if output_dir.trim().is_empty() {
        anyhow::bail!("The output folder is empty; choose a folder for the frames.");
    }
    let dir = std::path::Path::new(output_dir);
    if dir.is_dir() {
        if let Some(frame) = earliest_existing_frame(dir, prefix) {
            anyhow::bail!("Output folder {output_dir} already holds frames from an earlier run ({frame}); choose an empty folder or a new folder name.");
        }
        return Ok(());
    }
    if dir.exists() {
        anyhow::bail!("Output folder {output_dir} is an existing file; choose a folder or a new folder name.");
    }
    let parent_exists = dir.parent().is_some_and(|p| p.as_os_str().is_empty() || p.is_dir());
    if !parent_exists {
        anyhow::bail!("Output folder {output_dir} cannot be created because its parent folder does not exist; choose an existing folder.");
    }
    Ok(())
}

struct StackOutputs {
    star_count: usize,
    manifest_path: String,
}

fn write_stack(config: &SynthConfig, output_dir: &str, prefix: &str, save_catalog: bool, save_ground_truth: bool) -> anyhow::Result<StackOutputs> {
    let plan = pipeline::prepare_stack(config)?;
    let dir = std::path::Path::new(output_dir);
    std::fs::create_dir_all(dir)
        .map_err(|e| anyhow::anyhow!("Could not create the output folder {output_dir}: {e}"))?;
    for i in 0..plan.n_frames() {
        let path = dir.join(format!("{prefix}_{i:04}.fits"));
        let frame = plan.frame(i);
        write_derived_fits(&path.to_string_lossy(), &frame, Some(&plan.header(i)))
            .map_err(|e| anyhow::anyhow!("Failed to save frame {i} to {}: {e}", path.display()))?;
    }
    let manifest = dir.join(format!("{prefix}_frames.csv"));
    std::fs::write(&manifest, plan.manifest_csv())
        .map_err(|e| anyhow::anyhow!("Failed to save the frame manifest {}: {e}", manifest.display()))?;
    if save_catalog {
        let path = dir.join(format!("{prefix}_catalog.csv"));
        pipeline::save_catalog(&plan.stars, &config.noise, &path.to_string_lossy())
            .map_err(|e| anyhow::anyhow!("Failed to save catalog {}: {e}", path.display()))?;
    }
    if save_ground_truth {
        let path = dir.join(format!("{prefix}_groundtruth.fits"));
        write_derived_fits(&path.to_string_lossy(), &plan.ground_truth_in_frame_units(), Some(&plan.ground_truth_header()))
            .map_err(|e| anyhow::anyhow!("Failed to save ground truth {}: {e}", path.display()))?;
    }
    Ok(StackOutputs { star_count: plan.stars.len(), manifest_path: manifest.to_string_lossy().into_owned() })
}

#[command]
pub async fn generate_synth_stack_cmd(args: GenerateStackArgs) -> Result<SynthResult, String> {
    validate_output_dir(&args.output_dir, &args.prefix).map_err(|e| format!("{:#}", e))?;
    let config = args.config;
    let (width, height) = (config.field.width, config.field.height);
    let output_dir = args.output_dir.clone();
    let prefix = args.prefix;
    let (save_catalog, save_ground_truth) = (args.save_catalog, args.save_ground_truth);

    let outputs = tokio::task::spawn_blocking(move || write_stack(&config, &output_dir, &prefix, save_catalog, save_ground_truth))
        .await
        .map_err(|e| format!("Task failed: {}", e))?
        .map_err(|e| format!("{:#}", e))?;

    Ok(SynthResult {
        width,
        height,
        star_count: outputs.star_count,
        output_path: Some(args.output_dir),
        frames_manifest_path: Some(outputs.manifest_path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::load_cached_full;
    use crate::core::synth::pipeline::{FieldType, FrameVariation, GROUND_TRUTH_NOTE, OFFSET_CONVENTION};
    use crate::types::header::HduHeader;

    fn small_config() -> SynthConfig {
        let mut config = SynthConfig::default();
        config.field.width = 32;
        config.field.height = 32;
        config.field.n_stars = 5;
        config
    }

    fn stack_args(config: SynthConfig, output_dir: &std::path::Path) -> GenerateStackArgs {
        GenerateStackArgs {
            config,
            output_dir: output_dir.to_str().unwrap().to_string(),
            prefix: "synth".to_string(),
            save_catalog: false,
            save_ground_truth: false,
        }
    }

    fn file_names(dir: &std::path::Path) -> Vec<String> {
        let mut written: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        written.sort();
        written
    }

    fn header_of(path: &std::path::Path) -> HduHeader {
        load_cached_full(path.to_str().unwrap()).unwrap().header().expect("header").clone()
    }

    fn text_card(header: &HduHeader, key: &str) -> Option<String> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim().to_string())
    }

    #[tokio::test]
    async fn a_stack_of_zero_frames_is_an_error_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("stack");
        let mut config = small_config();
        config.n_frames = 0;
        let err = generate_synth_stack_cmd(stack_args(config, &out)).await.expect_err("zero frames must be refused");
        assert!(err.contains("Frame count 0"), "{err}");
        assert!(!out.exists(), "an empty output directory was left behind");
    }

    #[tokio::test]
    async fn an_output_folder_that_is_an_existing_file_is_refused_before_generating() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("stack");
        std::fs::write(&out, b"not a folder").unwrap();
        let err = generate_synth_stack_cmd(stack_args(small_config(), &out)).await.expect_err("a file path must be refused");
        assert!(err.contains("is an existing file"), "{err}");
        assert!(err.contains("stack"), "{err}");
        assert!(out.is_file(), "the existing file must be left untouched");
        assert!(!dir.path().join("stack").join("synth_0000.fits").exists());
    }

    #[tokio::test]
    async fn an_output_folder_with_a_missing_parent_is_refused_before_generating() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("missing").join("stack");
        let err = generate_synth_stack_cmd(stack_args(small_config(), &out)).await.expect_err("a missing parent must be refused");
        assert!(err.contains("parent folder"), "{err}");
        assert!(err.contains("stack"), "{err}");
        assert!(!dir.path().join("missing").exists(), "nothing may be created");
    }

    #[tokio::test]
    async fn a_stack_over_the_pixel_budget_is_refused_and_creates_no_folder() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("stack");
        let mut config = small_config();
        config.field.width = 16384;
        config.field.height = 16384;
        config.n_frames = 1024;
        let err = generate_synth_stack_cmd(stack_args(config, &out)).await.expect_err("an over-budget stack must be refused");
        assert!(err.contains("the limit is"), "{err}");
        assert!(!out.exists(), "an empty output directory was left behind");
    }

    #[tokio::test]
    async fn stack_frames_are_written_one_per_index_into_the_chosen_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = small_config();
        config.n_frames = 3;
        let res = generate_synth_stack_cmd(stack_args(config, dir.path())).await.unwrap();
        assert_eq!(res.star_count, 5);
        assert_eq!((res.width, res.height), (32, 32));
        let manifest = dir.path().join("synth_frames.csv");
        assert_eq!(res.frames_manifest_path.as_deref(), manifest.to_str());
        assert_eq!(file_names(dir.path()), ["synth_0000.fits", "synth_0001.fits", "synth_0002.fits", "synth_frames.csv"]);
        let rows: Vec<String> = std::fs::read_to_string(&manifest).unwrap().lines().map(str::to_string).collect();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], "frame,dx,dy,fwhm_factor,sky_factor,transparency,n_cosmic_rays,date_obs");
        let first = load_cached_full(dir.path().join("synth_0000.fits").to_str().unwrap()).unwrap();
        let last = load_cached_full(dir.path().join("synth_0002.fits").to_str().unwrap()).unwrap();
        let mjd = |e: &crate::infra::cache::ImageEntry| e.header().unwrap().get_f64("MJD-OBS").unwrap();
        let step = (mjd(&last) - mjd(&first)) / 2.0 * 86400.0;
        assert!((step - 310.0).abs() < 1e-3, "cadence step {step} s");
    }

    #[tokio::test]
    async fn stack_mode_writes_the_catalog_ground_truth_and_manifest_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = small_config();
        config.n_frames = 2;
        config.frame_variation = FrameVariation::default();
        let mut args = stack_args(config, dir.path());
        args.save_catalog = true;
        args.save_ground_truth = true;
        let res = generate_synth_stack_cmd(args).await.unwrap();
        assert_eq!(res.star_count, 5);
        assert_eq!(
            file_names(dir.path()),
            ["synth_0000.fits", "synth_0001.fits", "synth_catalog.csv", "synth_frames.csv", "synth_groundtruth.fits"]
        );
        let catalog = std::fs::read_to_string(dir.path().join("synth_catalog.csv")).unwrap();
        assert_eq!(catalog.lines().count(), 6);
        assert!(catalog.starts_with("id,x,y,z,flux_e,flux_adu,temperature\n"));
        let truth = header_of(&dir.path().join("synth_groundtruth.fits"));
        assert_eq!(text_card(&truth, "SYNTHGT").as_deref(), Some(GROUND_TRUTH_NOTE));
        assert_eq!(truth.get("SYNFRAME"), None);
        assert_eq!(text_card(&truth, "SYNFIELD").as_deref(), Some("uniform"));
        let manifest = std::fs::read_to_string(dir.path().join("synth_frames.csv")).unwrap();
        let second: Vec<&str> = manifest.lines().nth(2).unwrap().split(',').collect();
        assert_eq!(second[0], "1");
        assert_ne!(second[1], "0.0000");
        let frame1 = header_of(&dir.path().join("synth_0001.fits"));
        assert!((frame1.get_f64("SYNDX").unwrap() - second[1].parse::<f64>().unwrap()).abs() < 1e-4);
    }

    #[test]
    fn stack_args_without_save_flags_still_deserialize() {
        let json = serde_json::json!({
            "config": {
                "field": {"width": 16, "height": 16, "n_stars": 1, "flux_min": 2000.0, "flux_max": 500000.0, "seed": 1},
                "field_type": "Uniform",
                "psf_type": {"Gaussian": {"fwhm": 3.0}},
                "noise": {"gain": 1.5, "readout_noise": 8.0, "sky_background": 200.0, "dark_current": 0.05, "exposure_time": 300.0, "bias_level": 1000.0, "seed": 1001},
                "apply_vignette": false,
                "vignette_strength": 0.3,
                "n_frames": 2
            },
            "output_dir": "C:/tmp/stack",
            "prefix": "synth"
        });
        let args: GenerateStackArgs = serde_json::from_value(json).unwrap();
        assert!(!args.save_catalog && !args.save_ground_truth);
        assert_eq!(args.config.cadence_seconds, 310.0);
        assert_eq!(args.config.noise.seed, Some(1001));
        assert!(args.config.frame_variation.enabled);
    }

    #[test]
    fn a_frame_file_name_is_the_prefix_an_underscore_at_least_four_digits_and_fits() {
        for name in ["synth_0000.fits", "synth_0063.fits", "synth_10000.fits"] {
            assert!(is_frame_file_name(name, "synth"), "{name}");
        }
        for name in [
            "synth_000.fits",
            "synth_abcd.fits",
            "synth_0000.fits.bak",
            "synth_0000.fit",
            "other_0000.fits",
            "synth.fits",
            "mysynth_0000.fits",
            "synth_0000",
            "synth__0000.fits",
        ] {
            assert!(!is_frame_file_name(name, "synth"), "{name}");
        }
    }

    #[test]
    fn only_frames_of_the_same_prefix_count_as_an_earlier_run() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("other_0000.fits"), b"x").unwrap();
        assert_eq!(earliest_existing_frame(dir.path(), "synth"), None);
        std::fs::write(dir.path().join("synth_0002.fits"), b"x").unwrap();
        std::fs::write(dir.path().join("synth_0001.fits"), b"x").unwrap();
        assert_eq!(earliest_existing_frame(dir.path(), "synth").as_deref(), Some("synth_0001.fits"));
        assert_eq!(earliest_existing_frame(&dir.path().join("missing"), "synth"), None);
    }

    #[tokio::test]
    async fn a_second_run_into_a_folder_that_holds_frames_is_refused_and_the_first_run_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let output_dir = dir.path().to_str().unwrap().to_string();
        let mut first = small_config();
        first.n_frames = 3;
        generate_synth_stack_cmd(stack_args(first, dir.path())).await.unwrap();
        let before = std::fs::read(dir.path().join("synth_0000.fits")).unwrap();
        let mut second = small_config();
        second.n_frames = 1;
        second.field.n_stars = 2;
        second.field.seed += 1;
        let err = generate_synth_stack_cmd(stack_args(second, dir.path())).await.expect_err("a folder holding frames from an earlier run must be refused");
        assert!(err.contains("already holds frames from an earlier run"), "{err}");
        assert!(err.contains("synth_0000.fits"), "{err}");
        assert!(err.contains(&output_dir), "{err}");
        assert_eq!(file_names(dir.path()), ["synth_0000.fits", "synth_0001.fits", "synth_0002.fits", "synth_frames.csv"]);
        assert_eq!(std::fs::read(dir.path().join("synth_0000.fits")).unwrap(), before, "the first run was overwritten");
        let mut other_prefix = stack_args(small_config(), dir.path());
        other_prefix.prefix = "field".to_string();
        generate_synth_stack_cmd(other_prefix).await.unwrap();
        assert!(dir.path().join("field_0000.fits").is_file());
    }

    #[tokio::test]
    async fn a_king_cluster_without_a_tidal_radius_is_an_error_instead_of_a_hang() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = small_config();
        config.field_type = FieldType::KingCluster { core_radius: 50.0, tidal_radius: 0.0 };
        let args = GenerateSynthArgs {
            config,
            output_path: dir.path().join("king.fits").to_str().unwrap().to_string(),
            save_catalog: false,
            catalog_path: None,
            save_ground_truth: false,
            ground_truth_path: None,
        };
        let err = generate_synth_cmd(args).await.expect_err("a zero tidal radius must be refused");
        assert!(err.contains("Tidal radius"), "{err}");
    }

    #[tokio::test]
    async fn written_frames_carry_the_exposure_gain_unit_and_provenance_cards() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("frame.fits");
        let gt = dir.path().join("truth.fits");
        let mut config = small_config();
        config.field.seed = 4242;
        config.psf_type = crate::core::synth::pipeline::PsfType::Moffat { fwhm: 2.5, beta: 4.0 };
        let args = GenerateSynthArgs {
            config,
            output_path: out.to_str().unwrap().to_string(),
            save_catalog: false,
            catalog_path: None,
            save_ground_truth: true,
            ground_truth_path: Some(gt.to_str().unwrap().to_string()),
        };
        let res = generate_synth_cmd(args).await.unwrap();
        assert_eq!(res.frames_manifest_path, None);
        for path in [&out, &gt] {
            let header = header_of(path);
            assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
            assert_eq!(header.get_f64("GAIN"), Some(1.5));
            assert_eq!(header.get_f64("EGAIN"), Some(1.5));
            assert_eq!(header.get_f64("RDNOISE"), Some(8.0));
            assert_eq!(text_card(&header, "BUNIT").as_deref(), Some("ADU"));
            assert_eq!(header.get_i64("SYNSEED"), Some(4242));
            assert_eq!(text_card(&header, "SYNFIELD").as_deref(), Some("uniform"));
            assert_eq!(text_card(&header, "SYNPSF").as_deref(), Some("moffat"));
            assert_eq!(header.get_f64("SYNFWHM"), Some(2.5));
            assert_eq!(header.get("SYNFRAME"), None);
            assert_eq!(header.get("SYNDX"), None);
        }
        assert_eq!(header_of(&out).get("SYNTHGT"), None);
        assert_eq!(text_card(&header_of(&gt), "SYNTHGT").as_deref(), Some(GROUND_TRUTH_NOTE));

        let stack_dir = dir.path().join("stack");
        let mut config = small_config();
        config.n_frames = 2;
        config.frame_variation = FrameVariation::default();
        generate_synth_stack_cmd(stack_args(config, &stack_dir)).await.unwrap();
        for i in 0..2i64 {
            let header = header_of(&stack_dir.join(format!("synth_{i:04}.fits")));
            assert_eq!(header.get_i64("SYNFRAME"), Some(i));
            assert_eq!(header.get_i64("SYNSEED"), Some(42));
            assert!(header.get_f64("SYNDX").is_some() && header.get_f64("SYNDY").is_some());
            assert_eq!(text_card(&header, "SYNOFFS").as_deref(), Some(OFFSET_CONVENTION));
            assert_eq!(header.get_f64("RDNOISE"), Some(8.0));
            assert_eq!(text_card(&header, "SYNPSF").as_deref(), Some("gaussian"));
            if i == 0 {
                assert_eq!(header.get_f64("SYNDX"), Some(0.0));
                assert_eq!(header.get_f64("SYNDY"), Some(0.0));
            } else {
                assert_ne!(header.get_f64("SYNDX"), Some(0.0));
            }
        }
    }

    #[tokio::test]
    async fn a_catalog_path_equal_to_the_output_path_is_refused_and_the_image_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("field1").to_str().unwrap().to_string();
        let args = GenerateSynthArgs {
            config: small_config(),
            output_path: out.clone(),
            save_catalog: true,
            catalog_path: Some(out.clone()),
            save_ground_truth: false,
            ground_truth_path: None,
        };
        let err = generate_synth_cmd(args).await.expect_err("a catalog path equal to the output path must be refused");
        assert!(err.contains("Catalog path"), "{err}");
        assert!(!std::path::Path::new(&out).exists(), "nothing may be written when the paths collide");
    }

    #[tokio::test]
    async fn a_ground_truth_path_equal_to_the_output_path_is_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("field1.fits").to_str().unwrap().to_string();
        let args = GenerateSynthArgs {
            config: small_config(),
            output_path: out.clone(),
            save_catalog: false,
            catalog_path: None,
            save_ground_truth: true,
            ground_truth_path: Some(out.clone()),
        };
        let err = generate_synth_cmd(args).await.expect_err("a ground truth path equal to the output path must be refused");
        assert!(err.contains("Ground truth path"), "{err}");
        assert!(!std::path::Path::new(&out).exists(), "nothing may be written when the paths collide");
    }

    #[tokio::test]
    async fn a_catalog_path_equal_to_the_ground_truth_path_is_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("field1.fits").to_str().unwrap().to_string();
        let shared = dir.path().join("field1_side").to_str().unwrap().to_string();
        let args = GenerateSynthArgs {
            config: small_config(),
            output_path: out.clone(),
            save_catalog: true,
            catalog_path: Some(shared.clone()),
            save_ground_truth: true,
            ground_truth_path: Some(shared.clone()),
        };
        let err = generate_synth_cmd(args).await.expect_err("one path for catalog and ground truth must be refused");
        assert!(err.contains("both"), "{err}");
        assert!(!std::path::Path::new(&out).exists() && !std::path::Path::new(&shared).exists(), "nothing may be written when the paths collide");
    }
}
