use serde::{Deserialize, Serialize};
use tauri::command;

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

#[command]
pub async fn generate_synth_cmd(args: GenerateSynthArgs) -> Result<SynthResult, String> {
    let config = args.config;

    let (noisy, ground_truth, stars) =
        tokio::task::spawn_blocking(move || pipeline::generate(&config))
            .await
            .map_err(|e| format!("Task failed: {}", e))?;

    pipeline::save_fits(&noisy, &args.output_path)
        .map_err(|e| format!("Failed to save FITS: {}", e))?;

    if args.save_ground_truth {
        if let Some(gt_path) = &args.ground_truth_path {
            pipeline::save_fits(&ground_truth, gt_path)
                .map_err(|e| format!("Failed to save ground truth: {}", e))?;
        }
    }

    if args.save_catalog {
        if let Some(cat_path) = &args.catalog_path {
            pipeline::save_catalog(&stars, cat_path)
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

    let (frames, _gt, stars) =
        tokio::task::spawn_blocking(move || pipeline::generate_stack(&config))
            .await
            .map_err(|e| format!("Task failed: {}", e))?;

    let dir = std::path::Path::new(&args.output_dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create dir: {}", e))?;

    for (i, frame) in frames.iter().enumerate() {
        let path = dir.join(format!("{}_{:04}.fits", args.prefix, i));
        pipeline::save_fits(frame, path.to_str().unwrap_or("frame.fits"))
            .map_err(|e| format!("Failed to save frame {}: {}", i, e))?;
    }

    let first = &frames[0];
    Ok(SynthResult {
        width: first.dim().1 as u32,
        height: first.dim().0 as u32,
        star_count: stars.len(),
        output_path: Some(args.output_dir),
    })
}
