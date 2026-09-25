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
    let header = pipeline::frame_header(&config.noise, 0, config.cadence_seconds);
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
            write_derived_fits(gt_path, &ground_truth, Some(&header))
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
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateStackArgs {
    pub config: SynthConfig,
    pub output_dir: String,
    pub prefix: String,
}

#[command]
pub async fn generate_synth_stack_cmd(args: GenerateStackArgs) -> Result<SynthResult, String> {
    let config = args.config;
    let noise = config.noise.clone();
    let cadence_seconds = config.cadence_seconds;
    let (width, height) = (config.field.width, config.field.height);

    let (frames, _gt, stars) =
        tokio::task::spawn_blocking(move || pipeline::generate_stack(&config))
            .await
            .map_err(|e| format!("Task failed: {}", e))?
            .map_err(|e| format!("{:#}", e))?;

    let dir = std::path::Path::new(&args.output_dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create dir: {}", e))?;

    for (i, frame) in frames.iter().enumerate() {
        let path = dir.join(format!("{}_{:04}.fits", args.prefix, i));
        let header = pipeline::frame_header(&noise, i, cadence_seconds);
        write_derived_fits(path.to_str().unwrap_or("frame.fits"), frame, Some(&header))
            .map_err(|e| format!("Failed to save frame {}: {}", i, e))?;
    }

    Ok(SynthResult {
        width,
        height,
        star_count: stars.len(),
        output_path: Some(args.output_dir),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::load_cached_full;
    use crate::core::synth::pipeline::FieldType;

    fn small_config() -> SynthConfig {
        let mut config = SynthConfig::default();
        config.field.width = 32;
        config.field.height = 32;
        config.field.n_stars = 5;
        config
    }

    #[tokio::test]
    async fn a_stack_of_zero_frames_is_an_error_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("stack");
        let mut config = small_config();
        config.n_frames = 0;
        let args = GenerateStackArgs {
            config,
            output_dir: out.to_str().unwrap().to_string(),
            prefix: "synth".to_string(),
        };
        let err = generate_synth_stack_cmd(args).await.expect_err("zero frames must be refused");
        assert!(err.contains("Frame count 0"), "{err}");
        assert!(!out.exists(), "an empty output directory was left behind");
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
    async fn written_frames_carry_the_exposure_gain_and_unit() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("frame.fits").to_str().unwrap().to_string();
        let gt = dir.path().join("truth.fits").to_str().unwrap().to_string();
        let args = GenerateSynthArgs {
            config: small_config(),
            output_path: out.clone(),
            save_catalog: false,
            catalog_path: None,
            save_ground_truth: true,
            ground_truth_path: Some(gt.clone()),
        };
        generate_synth_cmd(args).await.unwrap();
        for path in [&out, &gt] {
            let entry = load_cached_full(path).unwrap();
            let header = entry.header().expect("header");
            assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
            assert_eq!(header.get_f64("GAIN"), Some(1.5));
            assert_eq!(header.get("BUNIT").map(|v| v.trim().trim_matches('\'').trim()), Some("ADU"));
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
