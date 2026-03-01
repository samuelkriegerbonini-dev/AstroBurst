mod commands;
mod domain;
mod model;
mod utils;

pub use crate::utils::dispatcher;

use crate::domain::config_manager;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())

        .register_asynchronous_uri_scheme_protocol("asset", |_ctx, request, responder| {
            let path = percent_encoding::percent_decode_str(request.uri().path())
                .decode_utf8_lossy()
                .to_string();

            let path = if cfg!(windows) && path.starts_with('/') {
                path[1..].to_string()
            } else {
                path
            };

            std::thread::spawn(move || {
                match std::fs::read(&path) {
                    Ok(data) => {
                        let mime = if path.ends_with(".png") {
                            "image/png"
                        } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
                            "image/jpeg"
                        } else if path.ends_with(".webp") {
                            "image/webp"
                        } else {
                            "application/octet-stream"
                        };

                        let response = tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", mime)
                            .header("Access-Control-Allow-Origin", "*")
                            .body(data)
                            .unwrap();
                        responder.respond(response);
                    }
                    Err(_) => {
                        let response = tauri::http::Response::builder()
                            .status(404)
                            .body(Vec::new())
                            .unwrap();
                        responder.respond(response);
                    }
                }
            });
        })
        .setup(|app| {
            if let Some(data_dir) = app.path().app_data_dir().ok() {
                config_manager::init_config_dir(&data_dir);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::image::process_fits,
            commands::image::process_batch,
            commands::image::get_raw_pixels,
            commands::image::get_raw_pixels_binary,
            commands::image::export_fits,
            commands::image::export_fits_rgb,
            commands::metadata::get_header,
            commands::metadata::get_full_header,
            commands::metadata::detect_narrowband_filters,
            commands::analysis::compute_histogram,
            commands::analysis::compute_fft_spectrum,
            commands::analysis::detect_stars,
            commands::visualization::apply_stf_render,
            commands::visualization::generate_tiles,
            commands::visualization::get_tile,
            commands::cube::process_cube_cmd,
            commands::cube::process_cube_lazy_cmd,
            commands::cube::get_cube_info,
            commands::cube::get_cube_frame,
            commands::cube::get_cube_spectrum,
            commands::astrometry::plate_solve_cmd,
            commands::astrometry::get_wcs_info,
            commands::astrometry::pixel_to_world,
            commands::astrometry::world_to_pixel,
            commands::stacking::calibrate,
            commands::stacking::stack,
            commands::stacking::drizzle_stack_cmd,
            commands::stacking::drizzle_rgb_cmd,
            commands::stacking::compose_rgb_cmd,
            commands::stacking::run_pipeline_cmd,
            commands::config::get_config,
            commands::config::update_config,
            commands::config::save_api_key,
            commands::config::get_api_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}