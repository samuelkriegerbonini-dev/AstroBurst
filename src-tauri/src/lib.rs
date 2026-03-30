pub mod types;
pub mod math;
pub mod infra;
pub mod core;

mod cmd;
mod domain;

use tauri::Manager;

fn urlencoding_decode(input: &str) -> String {
    let mut result = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                &input[i + 1..i + 3], 16
            ) {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())

        .register_asynchronous_uri_scheme_protocol("asset", |_ctx, request, responder| {
            let raw_path = request.uri().path().to_string();
            let path = urlencoding_decode(&raw_path);

            let path = if cfg!(windows) && path.starts_with('/') {
                path[1..].to_string()
            } else {
                path
            };

            std::thread::spawn(move || {
                const RETRY_DELAYS: &[u64] = &[100, 300, 800];
                let mut data = std::fs::read(&path);
                if data.is_err() {
                    for &delay in RETRY_DELAYS {
                        std::thread::sleep(std::time::Duration::from_millis(delay));
                        data = std::fs::read(&path);
                        if data.is_ok() {
                            break;
                        }
                    }
                }

                match data {
                    Ok(bytes) => {
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
                            .header("Cache-Control", "no-store, must-revalidate")
                            .body(bytes)
                            .unwrap();
                        responder.respond(response);
                    }
                    Err(_) => {
                        let response = tauri::http::Response::builder()
                            .status(404)
                            .header("Cache-Control", "no-store")
                            .body(Vec::new())
                            .unwrap();
                        responder.respond(response);
                    }
                }
            });
        })
        .setup(|app| {
            if let Some(data_dir) = app.path().app_data_dir().ok() {
                if !data_dir.exists() {
                    let _ = std::fs::create_dir_all(&data_dir);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cmd::image::process_fits,
            cmd::image::process_fits_full,
            cmd::image::get_raw_pixels_preview,
            cmd::image::export_fits,
            cmd::image::export_fits_rgb,
            cmd::image::export_png,
            cmd::image::export_rgb_png,
            cmd::metadata::get_header,
            cmd::metadata::get_full_header,
            cmd::metadata::get_fits_extensions,
            cmd::metadata::get_header_by_hdu,
            cmd::metadata::detect_narrowband_filters,
            cmd::analysis::compute_histogram,
            cmd::analysis::compute_fft_spectrum,
            cmd::analysis::detect_stars,
            cmd::visualization::apply_stf_render,
            cmd::visualization::generate_tiles,
            cmd::stacking::calibrate,
            cmd::stacking::stack,
            cmd::drizzle::drizzle_stack_cmd,
            cmd::drizzle::drizzle_rgb_cmd,
            cmd::compose::compose_rgb_cmd,
            cmd::compose::restretch_composite_cmd,
            cmd::compose::clear_composite_cache_cmd,
            cmd::compose::export_aligned_channels_cmd,
            cmd::compose::update_composite_channel_cmd,
            cmd::compose::blend_channels_cmd,
            cmd::compose::align_channels_cmd,
            cmd::compose::apply_scnr_cmd,
            cmd::compose::calibrate_composite_cmd,
            cmd::compose::compute_auto_wb_cmd,
            cmd::compose::reset_wb_cmd,
            cmd::resample::resample_fits_cmd,
            cmd::deconvolution::deconvolve_rl_cmd,
            cmd::background::extract_background_cmd,
            cmd::wavelet::wavelet_denoise_cmd,
            cmd::pipeline::run_pipeline_cmd,
            cmd::cube::process_cube_cmd,
            cmd::cube::process_cube_lazy_cmd,
            cmd::cube::get_cube_info,
            cmd::cube::get_cube_frame,
            cmd::cube::get_cube_spectrum,
            cmd::astrometry::plate_solve_cmd,
            cmd::astrometry::get_wcs_info,
            cmd::psf::estimate_psf_cmd,
            cmd::stretch::apply_arcsinh_stretch_cmd,
            cmd::stretch::masked_stretch_cmd,
            cmd::spcc::spcc_calibrate_cmd,
            cmd::config::get_config,
            cmd::config::update_config,
            cmd::config::save_api_key,
            cmd::config::get_api_key,
            cmd::synth::generate_synth_cmd,
            cmd::synth::generate_synth_stack_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AstroBurst");
}
