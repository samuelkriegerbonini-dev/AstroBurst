use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use astroburst_lib::core::synth::pipeline::{self, SynthConfig};
use astroburst_lib::infra::progress::ProgressHandle;
use astroburst_lib::types::constants::STAGE_RENDER;
use astroburst_lib::types::error::AppError as CoreError;

use super::error::ScienceError;
use super::jobs::{run_science_job, Finished, Product};
use super::state::ScienceState;
use crate::config::ServerConfig;
use crate::error::AppError;
use crate::extractors::SessionExtractor;
use crate::state::AppState;

#[derive(Deserialize)]
pub(crate) struct SynthParams {
    config: SynthConfig,
    #[serde(default)]
    stack: bool,
    name: Option<String>,
    #[serde(default)]
    save_ground_truth: bool,
}

pub(crate) async fn synth(
    SessionExtractor(session): SessionExtractor,
    State(state): State<AppState>,
    Extension(science): Extension<Arc<ScienceState>>,
    Json(params): Json<SynthParams>,
) -> Result<(StatusCode, Json<Value>), ScienceError> {
    let SynthParams {
        config,
        stack,
        name,
        save_ground_truth,
    } = params;
    config
        .validate()
        .map_err(|e| AppError::BadRequest(format!("{e:#}")))?;
    let frames = if stack { config.n_frames } else { 1 };
    if stack {
        pipeline::check_stack_pixel_budget(config.field.width, config.field.height, frames)
            .map_err(|e| AppError::BadRequest(format!("{e:#}")))?;
    }
    fit_session_cache(
        &state.config,
        &config,
        frames as usize + usize::from(save_ground_truth),
    )?;
    let name = name.unwrap_or_else(|| session.next_free_ref("synth"));
    run_science_job(
        &state,
        &session,
        &science,
        "synth",
        u64::from(frames),
        move |_, progress| {
            if stack {
                stack_products(&config, &name, save_ground_truth, progress)
            } else {
                field_products(&config, &name, save_ground_truth, progress)
            }
        },
    )
}

fn fit_session_cache(
    limits: &ServerConfig,
    config: &SynthConfig,
    images: usize,
) -> Result<(), ScienceError> {
    let image_bytes =
        config.field.width as usize * config.field.height as usize * std::mem::size_of::<f32>();
    let bytes = image_bytes.saturating_mul(images);
    let (message, variable) = if images > limits.cache_max_entries {
        (
            format!(
                "this request makes {images} images; a session cache holds at most {}",
                limits.cache_max_entries
            ),
            "ASTROBURST_CACHE_MAX_ENTRIES",
        )
    } else if bytes > limits.cache_max_bytes {
        (
            format!("this request makes {images} images of {bytes} bytes in all; a session cache holds at most {} bytes", limits.cache_max_bytes),
            "ASTROBURST_CACHE_MAX_BYTES",
        )
    } else {
        return Ok(());
    };
    Err(AppError::BadRequestWithHint {
        code: "bad_request",
        message,
        hint: Some(format!(
            "ask for fewer or smaller images, or start the server with a larger {variable}"
        )),
    }
    .into())
}

fn field_products(
    config: &SynthConfig,
    name: &str,
    save_ground_truth: bool,
    progress: &ProgressHandle,
) -> anyhow::Result<Finished> {
    progress.report(0, 1, STAGE_RENDER);
    let (noisy, ground_truth, stars) = pipeline::generate(config)?;
    let mut products = vec![Product {
        name: name.to_owned(),
        image: noisy,
        header: pipeline::synth_header(config, 0, None),
    }];
    if save_ground_truth {
        products.push(Product {
            name: format!("{name}_truth"),
            image: ground_truth,
            header: pipeline::ground_truth_header(config),
        });
    }
    let catalog = pipeline::catalog_csv(&stars, &config.noise);
    Ok(Finished {
        result: summary(config, 1, stars.len(), catalog, None),
        warnings: Vec::new(),
        products,
    })
}

fn stack_products(
    config: &SynthConfig,
    name: &str,
    save_ground_truth: bool,
    progress: &ProgressHandle,
) -> anyhow::Result<Finished> {
    let plan = pipeline::prepare_stack(config)?;
    let frames = plan.n_frames();
    let mut products = Vec::with_capacity(frames as usize + usize::from(save_ground_truth));
    for i in 0..frames {
        if progress.is_cancelled() {
            return Err(CoreError::Cancelled.into());
        }
        products.push(Product {
            name: format!("{name}_{i:04}"),
            image: plan.frame(i),
            header: plan.header(i),
        });
        progress.report(u64::from(i + 1), u64::from(frames), STAGE_RENDER);
    }
    if save_ground_truth {
        products.push(Product {
            name: format!("{name}_truth"),
            image: plan.ground_truth_in_frame_units(),
            header: plan.ground_truth_header(),
        });
    }
    let catalog = pipeline::catalog_csv(&plan.stars, &config.noise);
    Ok(Finished {
        result: summary(
            config,
            frames,
            plan.stars.len(),
            catalog,
            Some(plan.manifest_csv()),
        ),
        warnings: Vec::new(),
        products,
    })
}

fn summary(
    config: &SynthConfig,
    frames: u32,
    star_count: usize,
    catalog: String,
    manifest: Option<String>,
) -> Value {
    json!({
        "width": config.field.width,
        "height": config.field.height,
        "star_count": star_count,
        "frames": frames,
        "catalog_csv": catalog,
        "manifest_csv": manifest,
    })
}
