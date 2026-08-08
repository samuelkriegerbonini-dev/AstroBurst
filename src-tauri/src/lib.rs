pub mod types;
pub mod math;
pub mod infra;
pub mod core;

#[cfg(feature = "tauri")]
mod cmd;

#[cfg(feature = "tauri")]
use tauri::Manager;

#[cfg(feature = "tauri")]
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

#[cfg(feature = "tauri")]
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
                let output_dir = data_dir.join("output");
                let _ = std::fs::remove_dir_all(&output_dir);
                let _ = std::fs::create_dir_all(&output_dir);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cmd::io::process_fits,
            cmd::io::process_fits_full,
            cmd::io::get_raw_pixels_preview,
            cmd::io::get_raw_rgb_pixels_preview,
            cmd::export::export_fits,
            cmd::export::export_fits_rgb,
            cmd::export::export_png,
            cmd::export::export_rgb_png,
            cmd::compose::lrgb_combine_composite_cmd,
            cmd::metadata::get_header,
            cmd::metadata::get_full_header,
            cmd::metadata::get_fits_extensions,
            cmd::metadata::get_header_by_hdu,
            cmd::metadata::detect_narrowband_filters,
            cmd::analysis::compute_histogram,
            cmd::analysis::compute_fft_spectrum,
            cmd::analysis::detect_stars,
            cmd::analysis::detect_stars_composite,
            cmd::analysis::measure_photometry_cmd,
            cmd::analysis::analyze_subframes_cmd,
            cmd::visualization::apply_stf_render,
            cmd::visualization::generate_tiles,
            cmd::visualization::generate_tiles_rgb,
            cmd::stacking::calibrate,
            cmd::stacking::stack,
            cmd::stacking::drizzle_stack,
            cmd::stacking::drizzle_rgb_cmd,
            cmd::stacking::run_pipeline_cmd,
            cmd::compose::restretch_composite_cmd,
            cmd::compose::clear_composite_cache_cmd,
            cmd::compose::export_aligned_channels_cmd,
            cmd::compose::update_composite_channel_cmd,
            cmd::compose::blend_channels_cmd,
            cmd::compose::align_channels_cmd,
            cmd::compose::crop_channels_cmd,
            cmd::compose::calibrate_and_scnr_cmd,
            cmd::compose::compute_auto_wb_cmd,
            cmd::compose::reset_wb_cmd,
            cmd::processing::resample_fits_cmd,
            cmd::processing::debayer_fits_cmd,
            cmd::processing::debayer_batch_cmd,
            cmd::processing::deconvolve_rl_cmd,
            cmd::processing::extract_background_cmd,
            cmd::processing::extract_background_batch_cmd,
            cmd::processing::wavelet_denoise_cmd,
            cmd::processing::apply_arcsinh_stretch_cmd,
            cmd::processing::apply_ghs_stretch_cmd,
            cmd::processing::masked_stretch_cmd,
            cmd::processing::remove_stars_cmd,
            cmd::processing::remove_stars_composite_cmd,
            cmd::processing::arcsinh_stretch_composite_cmd,
            cmd::processing::ghs_stretch_composite_cmd,
            cmd::processing::masked_stretch_composite_cmd,
            cmd::processing::apply_tone_composite_cmd,
            cmd::cube::process_cube_cmd,
            cmd::cube::process_cube_lazy_cmd,
            cmd::cube::get_cube_info,
            cmd::cube::get_cube_frame,
            cmd::cube::get_cube_spectrum,
            cmd::astrometry::plate_solve_cmd,
            cmd::astrometry::get_wcs_info,
            cmd::astrometry::pixel_to_world_cmd,
            cmd::astrometry::check_pointing_overlap_cmd,
            cmd::psf::estimate_psf_cmd,
            cmd::spcc::spcc_calibrate_cmd,
            cmd::config::get_config,
            cmd::config::update_config,
            cmd::config::save_api_key,
            cmd::config::get_api_key,
            cmd::synth::generate_synth_cmd,
            cmd::synth::generate_synth_stack_cmd,
            cmd::output::get_output_dir_info,
            cmd::output::cleanup_output_cmd,
            cmd::output::cancel_progress_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AstroBurst");
}
