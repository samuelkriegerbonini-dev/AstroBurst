// FITS block and card layout
pub const BLOCK_SIZE: usize = 2880;
pub const CARD_SIZE: usize = 80;
pub const CARDS_PER_BLOCK: usize = BLOCK_SIZE / CARD_SIZE;

// Header keyword names used for FITS axis lookups
pub const HEADER_NAXIS1: &str = "NAXIS1";
pub const HEADER_NAXIS2: &str = "NAXIS2";

// Image processing thresholds and defaults
pub const PADDING_THRESHOLD: f32 = 1e-7;
pub const MAD_TO_SIGMA: f64 = 1.4826;
pub const HISTOGRAM_BINS: usize = 65536;
pub const HISTOGRAM_BINS_DISPLAY: usize = 512;

// Background extraction grid and polynomial bounds
pub const MIN_GRID_SIZE: usize = 3;
pub const MAX_GRID_SIZE: usize = 32;
pub const MIN_POLY_DEGREE: usize = 1;
pub const MAX_POLY_DEGREE: usize = 5;
pub const MIN_ITERATIONS: usize = 1;
pub const MAX_ITERATIONS: usize = 10;
pub const MODE_DIVIDE: &str = "divide";
pub const DEFAULT_STEM: &str = "bg";

// Tauri event names emitted during long-running operations
pub const PROGRESS_EVENT: &str = "background-progress";
pub const EVENT_DECONV_PROGRESS: &str = "deconv-progress";
pub const EVENT_DRIZZLE_PROGRESS: &str = "drizzle-progress";
pub const EVENT_DRIZZLE_RGB_PROGRESS: &str = "drizzle-rgb-progress";
pub const EVENT_CALIBRATE_PROGRESS: &str = "calibrate-progress";
pub const EVENT_STACK_PROGRESS: &str = "stack-progress";
pub const EVENT_WAVELET_PROGRESS: &str = "wavelet-progress";

// Progress step count for background extraction
pub const PROGRESS_STEPS: usize = 4;

// ---- JSON response keys: shared across all Tauri commands ----

// Timing
pub const RES_ELAPSED_MS: &str = "elapsed_ms";

// Spatial dimensions and coordinates
pub const RES_DIMENSIONS: &str = "dimensions";
pub const RES_WIDTH: &str = "width";
pub const RES_HEIGHT: &str = "height";
pub const RES_X: &str = "x";
pub const RES_Y: &str = "y";
pub const RES_NAXIS1: &str = "naxis1";
pub const RES_NAXIS2: &str = "naxis2";
pub const RES_NAXIS3: &str = "naxis3";
pub const RES_NAXIS: &str = "naxis";
pub const RES_OUTPUT_DIMS: &str = "output_dims";
pub const RES_INPUT_DIMS: &str = "input_dims";
pub const RES_ORIGINAL_DIMENSIONS: &str = "original_dimensions";

// File paths returned in responses
pub const RES_PNG_PATH: &str = "png_path";
pub const RES_FITS_PATH: &str = "fits_path";
pub const RES_OUTPUT_PATH: &str = "output_path";
pub const RES_CORRECTED_PNG: &str = "corrected_png";
pub const RES_MODEL_PNG: &str = "model_png";
pub const RES_CORRECTED_FITS: &str = "corrected_fits";
pub const RES_WEIGHT_MAP_PATH: &str = "weight_map_path";
pub const RES_TILE_PATH: &str = "tile_path";
pub const RES_PATH: &str = "path";
pub const RES_FILE_PATH: &str = "file_path";
pub const RES_FILE_NAME: &str = "file_name";
pub const RES_COLLAPSED_PATH: &str = "collapsed_path";

// Image statistics
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

// STF (Screen Transfer Function) parameters
pub const RES_AUTO_STF: &str = "auto_stf";
pub const RES_STF: &str = "stf";
pub const RES_SHADOW: &str = "shadow";
pub const RES_MIDTONE: &str = "midtone";
pub const RES_HIGHLIGHT: &str = "highlight";

// Histogram response keys
pub const RES_HISTOGRAM: &str = "histogram";
pub const RES_BINS: &str = "bins";
pub const RES_BIN_COUNT: &str = "bin_count";
pub const RES_BIN_EDGES: &str = "bin_edges";

// FFT / power spectrum
pub const RES_PIXELS_B64: &str = "pixels_b64";
pub const RES_DC_MAGNITUDE: &str = "dc_magnitude";
pub const RES_MAX_MAGNITUDE: &str = "max_magnitude";

// Astrometry / WCS
pub const RES_CENTER_RA: &str = "center_ra";
pub const RES_CENTER_DEC: &str = "center_dec";
pub const RES_PIXEL_SCALE_ARCSEC: &str = "pixel_scale_arcsec";
pub const RES_FOV_W_ARCMIN: &str = "field_of_view_w_arcmin";
pub const RES_FOV_H_ARCMIN: &str = "field_of_view_h_arcmin";
pub const RES_RA: &str = "ra";
pub const RES_DEC: &str = "dec";
pub const RES_WCS_UPDATES: &str = "wcs_updates";

// Background extraction
pub const RES_SAMPLE_COUNT: &str = "sample_count";
pub const RES_RMS_RESIDUAL: &str = "rms_residual";

// Deconvolution
pub const RES_ITERATIONS_RUN: &str = "iterations_run";
pub const RES_CONVERGENCE: &str = "convergence";

// Arcsinh stretch
pub const RES_STRETCH_FACTOR: &str = "stretch_factor";

// Wavelet denoise
pub const RES_SCALES_PROCESSED: &str = "scales_processed";
pub const RES_NOISE_ESTIMATE: &str = "noise_estimate";

// Stacking and drizzle
pub const RES_FRAME_COUNT: &str = "frame_count";
pub const RES_FRAME_COUNT_R: &str = "frame_count_r";
pub const RES_FRAME_COUNT_G: &str = "frame_count_g";
pub const RES_FRAME_COUNT_B: &str = "frame_count_b";
pub const RES_REJECTED_PIXELS: &str = "rejected_pixels";
pub const RES_OFFSETS: &str = "offsets";
pub const RES_SCALE: &str = "scale";
pub const RES_DY: &str = "dy";
pub const RES_DX: &str = "dx";

// Calibration flags
pub const RES_HAS_BIAS: &str = "has_bias";
pub const RES_HAS_DARK: &str = "has_dark";
pub const RES_HAS_FLAT: &str = "has_flat";

// RGB compose
pub const RES_SCNR_APPLIED: &str = "scnr_applied";
pub const RES_OFFSET_G: &str = "offset_g";
pub const RES_OFFSET_B: &str = "offset_b";
pub const RES_DIMENSION_CROP: &str = "dimension_crop";

// IFU cube
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

// FFT display
pub const RES_DISPLAY_WIDTH: &str = "display_width";
pub const RES_DISPLAY_HEIGHT: &str = "display_height";
pub const RES_ORIGINAL_SIZE: &str = "original_size";
pub const RES_WINDOWED: &str = "windowed";

// FITS header / metadata
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

// Narrowband filter detection
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

// Batch processing
pub const RES_RESULTS: &str = "results";
pub const RES_ERROR: &str = "error";

// Config / API key storage
pub const RES_SAVED: &str = "saved";
pub const RES_SERVICE: &str = "service";

// RGB compose defaults
pub const DEFAULT_WB_VALUE: f64 = 1.0;
pub const DEFAULT_SCNR_AMOUNT: f32 = 1.0;
pub const DEFAULT_DIMENSION_TOLERANCE: usize = 100;
pub const WB_MODE_MANUAL: &str = "manual";
pub const WB_MODE_NONE: &str = "none";
pub const SCNR_METHOD_MAXIMUM: &str = "maximum";
pub const DEFAULT_RGB_COMPOSITE_FILENAME: &str = "rgb_composite.png";

// Deconvolution defaults
pub const SUFFIX_DECONV: &str = "deconv";

// Drizzle defaults
pub const DEFAULT_DRIZZLE_SCALE: f64 = 2.0;
pub const DEFAULT_DRIZZLE_PIXFRAC: f64 = 0.7;
pub const DEFAULT_DRIZZLE_SIGMA: f32 = 3.0;
pub const DEFAULT_DRIZZLE_SIGMA_ITERS: usize = 5;
pub const KERNEL_GAUSSIAN: &str = "gaussian";
pub const KERNEL_LANCZOS3: &str = "lanczos3";
pub const KERNEL_LANCZOS: &str = "lanczos";

// Drizzle render stages
pub const STAGE_RENDER: &str = "render";
pub const STAGE_SAVE: &str = "save";

// Drizzle output filenames
pub const FILE_DRIZZLE_RESULT_PNG: &str = "drizzle_result.png";
pub const FILE_DRIZZLE_RESULT_FITS: &str = "drizzle_result.fits";
pub const FILE_DRIZZLE_WEIGHTS_PNG: &str = "drizzle_weights.png";
pub const FILE_DRIZZLE_RGB_PNG: &str = "drizzle_rgb.png";
pub const FILE_DRIZZLE_RGB_FITS: &str = "drizzle_rgb.fits";

// IFU cube filesystem conventions
pub const EXT_ZIP: &str = ".zip";
pub const DIR_FRAMES: &str = "frames";
pub const FILE_COLLAPSED_MEAN: &str = "collapsed_mean.png";
pub const FILE_COLLAPSED_MEDIAN: &str = "collapsed_median.png";

pub const RESAMPLED: &str = "resampled";
