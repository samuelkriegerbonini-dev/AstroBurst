pub const BLOCK_SIZE: usize = 2880;

pub const HEADER_NAXIS1: &str = "NAXIS1";
pub const HEADER_NAXIS2: &str = "NAXIS2";

pub const PADDING_THRESHOLD: f32 = 1e-7;
pub const MAD_TO_SIGMA: f64 = 1.4826;
pub const HISTOGRAM_BINS: usize = 65536;
pub const HISTOGRAM_BINS_DISPLAY: usize = 512;

pub const MIN_GRID_SIZE: usize = 3;
pub const MAX_GRID_SIZE: usize = 32;
pub const MIN_POLY_DEGREE: usize = 1;
pub const MAX_POLY_DEGREE: usize = 5;
pub const MIN_ITERATIONS: usize = 1;
pub const MAX_ITERATIONS: usize = 10;
pub const MODE_DIVIDE: &str = "divide";
pub const DEFAULT_STEM: &str = "bg";

pub const PROGRESS_EVENT: &str = "background-progress";
pub const EVENT_DECONV_PROGRESS: &str = "deconv-progress";
pub const EVENT_DRIZZLE_RGB_PROGRESS: &str = "drizzle-rgb-progress";
pub const EVENT_CALIBRATE_PROGRESS: &str = "calibrate-progress";
pub const EVENT_STACK_PROGRESS: &str = "stack-progress";
pub const EVENT_WAVELET_PROGRESS: &str = "wavelet-progress";

pub const PROGRESS_STEPS: usize = 4;

pub const RES_ELAPSED_MS: &str = "elapsed_ms";

pub const RES_DIMENSIONS: &str = "dimensions";
pub const RES_WIDTH: &str = "width";
pub const RES_HEIGHT: &str = "height";
pub const RES_NAXIS1: &str = "naxis1";
pub const RES_NAXIS2: &str = "naxis2";
pub const RES_NAXIS3: &str = "naxis3";
pub const RES_NAXIS: &str = "naxis";
pub const RES_OUTPUT_DIMS: &str = "output_dims";
pub const RES_INPUT_DIMS: &str = "input_dims";
pub const RES_ORIGINAL_DIMENSIONS: &str = "original_dimensions";

pub const RES_PNG_PATH: &str = "png_path";
pub const RES_FITS_PATH: &str = "fits_path";
pub const RES_OUTPUT_PATH: &str = "output_path";
pub const RES_CORRECTED_PNG: &str = "corrected_png";
pub const RES_MODEL_PNG: &str = "model_png";
pub const RES_CORRECTED_FITS: &str = "corrected_fits";
pub const RES_PATH: &str = "path";
pub const RES_FILE_PATH: &str = "file_path";
pub const RES_FILE_NAME: &str = "file_name";

pub const RES_MIN: &str = "min";
pub const RES_MAX: &str = "max";
pub const RES_DATA_MIN: &str = "data_min";
pub const RES_DATA_MAX: &str = "data_max";
pub const RES_MEDIAN: &str = "median";
pub const RES_MEAN: &str = "mean";
pub const RES_SIGMA: &str = "sigma";
pub const RES_MAD: &str = "mad";
pub const RES_TOTAL_PIXELS: &str = "total_pixels";
pub const RES_STATS: &str = "stats";
pub const RES_STATS_R: &str = "stats_r";
pub const RES_STATS_G: &str = "stats_g";
pub const RES_STATS_B: &str = "stats_b";

pub const RES_AUTO_STF: &str = "auto_stf";
pub const RES_STF: &str = "stf";
pub const RES_SHADOW: &str = "shadow";
pub const RES_MIDTONE: &str = "midtone";
pub const RES_HIGHLIGHT: &str = "highlight";

pub const RES_HISTOGRAM: &str = "histogram";
pub const RES_BINS: &str = "bins";
pub const RES_BIN_COUNT: &str = "bin_count";
pub const RES_BIN_EDGES: &str = "bin_edges";

pub const RES_PIXELS_B64: &str = "pixels_b64";

pub const RES_CENTER_RA: &str = "center_ra";
pub const RES_CENTER_DEC: &str = "center_dec";
pub const RES_PIXEL_SCALE_ARCSEC: &str = "pixel_scale_arcsec";
pub const RES_FOV_W_ARCMIN: &str = "field_of_view_w_arcmin";
pub const RES_FOV_H_ARCMIN: &str = "field_of_view_h_arcmin";
pub const RES_FOV_ARCMIN: &str = "fov_arcmin";
pub const RES_WCS_UPDATES: &str = "wcs_updates";
pub const RES_WCS_PARAMS: &str = "wcs_params";
pub const RES_WCS_CRPIX1: &str = "crpix1";
pub const RES_WCS_CRPIX2: &str = "crpix2";
pub const RES_WCS_CRVAL1: &str = "crval1";
pub const RES_WCS_CRVAL2: &str = "crval2";
pub const RES_WCS_CD: &str = "cd";
pub const RES_WCS_PROJECTION: &str = "projection";

pub const RES_SAMPLE_COUNT: &str = "sample_count";
pub const RES_RMS_RESIDUAL: &str = "rms_residual";

pub const RES_ITERATIONS_RUN: &str = "iterations_run";
pub const RES_CONVERGENCE: &str = "convergence";

pub const RES_STRETCH_FACTOR: &str = "stretch_factor";

pub const RES_SCALES_PROCESSED: &str = "scales_processed";
pub const RES_NOISE_ESTIMATE: &str = "noise_estimate";

pub const RES_FRAME_COUNT: &str = "frame_count";
pub const RES_FRAME_COUNT_R: &str = "frame_count_r";
pub const RES_FRAME_COUNT_G: &str = "frame_count_g";
pub const RES_FRAME_COUNT_B: &str = "frame_count_b";
pub const RES_REJECTED_PIXELS: &str = "rejected_pixels";
pub const RES_OFFSETS: &str = "offsets";
pub const RES_SCALE: &str = "scale";
pub const RES_DY: &str = "dy";
pub const RES_DX: &str = "dx";

pub const RES_HAS_BIAS: &str = "has_bias";
pub const RES_HAS_DARK: &str = "has_dark";
pub const RES_HAS_FLAT: &str = "has_flat";

pub const RES_SCNR_APPLIED: &str = "scnr_applied";
pub const RES_OFFSET_G: &str = "offset_g";
pub const RES_OFFSET_B: &str = "offset_b";
pub const RES_DIMENSION_INFO: &str = "dimension_info";

pub const RES_FRAMES: &str = "frames";
pub const RES_BITPIX: &str = "bitpix";
pub const RES_FRAME_INDEX: &str = "frame_index";
pub const RES_SPECTRUM: &str = "spectrum";
pub const RES_SPECTRAL_CLASSIFICATION: &str = "spectral_classification";
pub const RES_IS_SPECTRAL: &str = "is_spectral";
pub const RES_SPECTRAL_REASON: &str = "reason";
pub const RES_AXIS_TYPE: &str = "axis_type";
pub const RES_AXIS_UNIT: &str = "axis_unit";
pub const RES_CHANNEL_COUNT: &str = "channel_count";
pub const RES_WAVELENGTHS: &str = "wavelengths";

pub const RES_HEADER: &str = "header";
pub const RES_CARDS: &str = "cards";
pub const RES_TOTAL_CARDS: &str = "total_cards";
pub const RES_CATEGORIES: &str = "categories";
pub const RES_KEY: &str = "key";
pub const RES_VALUE: &str = "value";
pub const RES_EXTENSIONS: &str = "extensions";
pub const RES_INDEX: &str = "index";
pub const RES_EXTNAME: &str = "extname";
pub const RES_HAS_DATA: &str = "has_data";

pub const RES_FILTER: &str = "filter";
pub const RES_FILTER_ID: &str = "filter_id";
pub const RES_FILTER_DETECTION: &str = "filter_detection";
pub const RES_FILTERS: &str = "filters";
pub const RES_HUBBLE_CHANNEL: &str = "hubble_channel";
pub const RES_CONFIDENCE: &str = "confidence";
pub const RES_MATCHED_KEYWORD: &str = "matched_keyword";
pub const RES_MATCHED_VALUE: &str = "matched_value";
pub const RES_FILENAME_HINT: &str = "filename_hint";
pub const RES_PALETTE: &str = "palette";

pub const RES_SAVED: &str = "saved";
pub const RES_SERVICE: &str = "service";
pub const DEFAULT_API_KEY_SERVICE: &str = "astrometry";
pub const DEFAULT_ASTROMETRY_API_URL: &str = "https://nova.astrometry.net";

pub const DEFAULT_WB_VALUE: f64 = 1.0;
pub const DEFAULT_SCNR_AMOUNT: f32 = 1.0;
pub const MAX_DIMENSION_RATIO: f64 = 8.0;
pub const WB_MODE_MANUAL: &str = "manual";
pub const WB_MODE_NONE: &str = "none";
pub const SCNR_METHOD_MAXIMUM: &str = "maximum";

pub const SUFFIX_DECONV: &str = "deconv";

pub const DEFAULT_DRIZZLE_SCALE: f64 = 2.0;
pub const DEFAULT_DRIZZLE_PIXFRAC: f64 = 0.7;
pub const DEFAULT_DRIZZLE_SIGMA: f32 = 3.0;
pub const DEFAULT_DRIZZLE_SIGMA_ITERS: usize = 5;
pub const KERNEL_GAUSSIAN: &str = "gaussian";
pub const KERNEL_LANCZOS3: &str = "lanczos3";
pub const KERNEL_LANCZOS: &str = "lanczos";

pub const STAGE_RENDER: &str = "render";
pub const STAGE_SAVE: &str = "save";

pub const FILE_DRIZZLE_RGB_PNG: &str = "drizzle_rgb.png";
pub const FILE_DRIZZLE_RGB_FITS: &str = "drizzle_rgb.fits";

pub const RESAMPLED: &str = "resampled";
pub const LRGB_APPLIED: &str = "lrgb_applied";

pub const COMPOSITE_KEY_R: &str = "__composite_r";
pub const COMPOSITE_KEY_G: &str = "__composite_g";
pub const COMPOSITE_KEY_B: &str = "__composite_b";

pub const COMPOSITE_ORIG_R: &str = "__composite_orig_r";
pub const COMPOSITE_ORIG_G: &str = "__composite_orig_g";
pub const COMPOSITE_ORIG_B: &str = "__composite_orig_b";

pub const STF_R: &str = "stf_r";
pub const STF_G: &str = "stf_g";
pub const STF_B: &str = "stf_b";
pub const CHANNELS: &str = "channels";
pub const DIMENSIONS: &str = "dimensions";
pub const ALIGN_METHOD: &str = "align_method";
pub const COPY_WCS: &str = "copy_wcs";

pub const RES_FILE_SIZE_BYTES: &str = "file_size_bytes";
pub const RES_APPLY_STF: &str = "apply_stf";
pub const RES_COPY_METADATA: &str = "copy_metadata";
pub const RES_BIT_DEPTH: &str = "bit_depth";
pub const RES_LABEL: &str = "label";
pub const RES_CHANNEL_PREVIEWS: &str = "channel_previews";
pub const RES_RGB_PREVIEW: &str = "rgb_preview";
pub const RES_CHANNEL: &str = "channel";
pub const RES_OFFSET: &str = "offset";
pub const RES_X: &str = "x";
pub const RES_Y: &str = "y";
pub const RES_PEAK: &str = "peak";
pub const RES_FLUX: &str = "flux";
pub const RES_FWHM: &str = "fwhm";
pub const RES_ELLIPTICITY: &str = "ellipticity";
pub const RES_SNR: &str = "snr";
pub const RES_KERNEL_SIZE: &str = "kernel_size";
pub const RES_AVERAGE_FWHM: &str = "average_fwhm";
pub const RES_AVERAGE_ELLIPTICITY: &str = "average_ellipticity";
pub const RES_SPREAD_PIXELS: &str = "spread_pixels";
pub const RES_STARS_USED: &str = "stars_used";
pub const RES_STARS_REJECTED: &str = "stars_rejected";
pub const RES_KERNEL: &str = "kernel";

pub const RES_STARS_MASKED: &str = "stars_masked";
pub const RES_MASK_COVERAGE: &str = "mask_coverage";
pub const RES_FINAL_BACKGROUND: &str = "final_background";
pub const RES_CONVERGED: &str = "converged";
pub const RES_R_FACTOR: &str = "r_factor";
pub const RES_G_FACTOR: &str = "g_factor";
pub const RES_B_FACTOR: &str = "b_factor";
pub const RES_STARS_MATCHED: &str = "stars_matched";
pub const RES_STARS_TOTAL: &str = "stars_total";
pub const RES_AVG_COLOR_INDEX: &str = "avg_color_index";
pub const RES_WHITE_REF: &str = "white_reference";
pub const RES_CATALOG_NAME: &str = "catalog_name";

pub const SUFFIX_MASKED_STRETCH: &str = "masked_stretch";

pub const RES_BLEND_PRESET: &str = "blend_preset";

pub const RES_WB_APPLIED: &str = "wb_applied";

pub const DEFAULT_OUTPUT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub const RES_COMPOSITE_DIMS: &str = "composite_dims";
pub const RES_CURVES_APPLIED: &str = "curves_applied";
pub const RES_LEVELS_APPLIED: &str = "levels_applied";
pub const RES_STF_APPLIED: &str = "stf_applied";
pub const RES_CLEANED_BYTES: &str = "cleaned_bytes";
pub const RES_CLEANED_FILES: &str = "cleaned_files";
pub const RES_FILE_COUNT: &str = "file_count";
pub const RES_OUTPUT_DIR: &str = "output_dir";
pub const RES_TOTAL_SIZE: &str = "total_size";

pub const WIZARD_CACHE_PREFIX: &str = "__wizard_ch_";

pub fn wizard_cache_key(bin_id: &str, stage: &str) -> String {
    format!("{}{}{}", WIZARD_CACHE_PREFIX, bin_id, stage)
}

pub fn wizard_aligned_key(bin_id: &str) -> String {
    wizard_cache_key(bin_id, "_aligned")
}

pub fn wizard_cropped_key(bin_id: &str) -> String {
    wizard_cache_key(bin_id, "_cropped")
}

pub fn wizard_bg_key(bin_id: &str) -> String {
    wizard_cache_key(bin_id, "_bg")
}

pub const STAR_MASK_KEY: &str = "__star_mask";

pub const RES_CACHE_KEYS: &str = "cache_keys";
pub const RES_PERSIST_TO_DISK: &str = "persist_to_disk";
