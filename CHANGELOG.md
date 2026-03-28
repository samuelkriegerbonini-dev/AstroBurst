# Changelog

All notable changes to AstroBurst will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.2] - 2026-03-28
### Added

#### Synthetic FITS Generation
- `core/synth/` module (645 lines): star field generation with configurable count and flux distribution, Gaussian/Moffat PSF modeling, Poisson/Gaussian/readout noise injection, and pipeline orchestration
- `cmd/synth.rs` with `generate_synth_cmd` and `generate_synth_stack_cmd` Tauri commands
- `SynthPanel.tsx` (227 lines) frontend with star count, PSF sigma, noise level controls, and preview
- `synth.service.ts` service layer
- Synth tab in PreviewPanel side panel alongside Processing, Compose, Stacking, Config

#### Narrowband Palette System
- `PaletteType` enum in `core/metadata/header_discovery.rs`: SHO (Hubble Palette), HOO, HOS, NaturalColor, Custom with `from_str_loose()` parser
- `palette_channels()` routing function mapping narrowband filter to RGB channels per palette type
- `suggest_palette_with_type()` with per-palette channel assignment logic, replacing the SHO-only `suggest_palette()`
- Custom palette mode returns all files as unmapped for fully manual assignment
- `SmartChannelMapper.tsx` gains palette preset selector dropdown (`PALETTE_PRESETS`)
- `detect_narrowband_filters` command accepts optional `palette` parameter
- `header.service.ts` passes palette selection through to backend

#### Live Composite STF Re-stretch
- `restretch_composite_cmd` in `cmd/compose.rs`: reads pre-stretch data from composite cache, applies per-channel STF with optional SCNR, renders preview PNG without re-running the full compose pipeline
- `clear_composite_cache_cmd` for explicit cache invalidation
- `CompositeStfPanel` in `PreviewTab.tsx` with per-channel shadow/midtone/highlight sliders, linked/unlinked mode toggle, reset-to-auto button, and debounced re-stretch (300ms)
- `PreviewContext.tsx` expanded with composite STF state (`compositeStfR/G/B`), linked flag, auto-STF reference values, `compositePreviewUrl`, `isShowingComposite`, `clearComposite`
- `compose.service.ts` gains `restretchComposite()` and `clearCompositeCache()` functions

#### Export Aligned Channels
- `export_aligned_channels_cmd` in `cmd/compose.rs`: loads, harmonizes, and aligns R/G/B channels, then exports each as individual FITS with WCS headers updated for the applied offset (`CRPIX1/2` adjusted by dx/dy)
- `exportAlignedChannels()` service function with `ExportAlignedOptions` (align method, copy WCS, copy metadata)
- `ExportPanel.tsx` gains Aligned Channels export section

#### PNG Export Pipeline
- `export_png` command: single-channel PNG export with optional STF stretch, 8-bit or 16-bit output
- `export_rgb_png` command: composite RGB PNG export with per-channel STF parameters, reading from composite cache with STF application
- `ExportPngOptions` and `ExportRgbPngOptions` interfaces in `export.service.ts`
- `ExportPanel.tsx` gains PNG export controls with bit depth selector and STF toggle
- Frontend passes `compositeStf` from `RgbContext` through `ExportTab` to `ExportPanel` to `exportRgbPng` service

#### 16-bit Rendering
- `render_grayscale_16bit()` in `infra/render/grayscale.rs`: linear stretch to 16-bit PNG with `write_png_l16()` helper
- `render_stretched_8bit()` and `render_stretched_16bit()`: pre-stretched [0,1] data to 8/16-bit PNG
- `render_rgb_16bit()` in `infra/render/rgb.rs`: 16-bit per-channel RGB PNG with parallel row processing

#### Multi-BITPIX FITS Export
- `write_fits_mono_bitpix()` in `infra/fits/writer.rs`: supports BITPIX 16 (with auto BZERO/BSCALE), -32 (float32), -64 (float64)
- `compute_bzero_bscale()` for optimal 16-bit quantization from data range
- `write_i16_slice_as_be()` and `write_f64_slice_as_be()` chunked big-endian writers
- `export_fits` command now honors `bitpix` parameter (was hardcoded to -32)

#### Cache Enhancements
- `insert_synthetic()` on `GLOBAL_IMAGE_CACHE`: stores pre-computed channel data (from compose pipeline) as synthetic cache entries with associated stats, enabling re-stretch without recompose
- `remove()` method with proper byte accounting for targeted cache eviction
- Composite keys (`COMPOSITE_KEY_R/G/B`) shared from `types/constants.rs` across compose and image commands

#### UI Improvements
- `Slider.tsx` click-to-edit: clicking the value display opens an inline text input for direct numeric entry; commits on Enter/blur, cancels on Escape; optional `hint` prop
- `AdvancedImageViewer.tsx` retry with backoff: up to 3 retries with 200/600/1500ms delays on image load failure; cache-busting query params on retry; manual retry button on persistent failure; loading spinner overlay
- `DropZone.tsx` drag type guard: ignores non-file drag events (e.g. dragging channel chips) by checking for `Files` or `application/x-tauri-file` in `dataTransfer.types`
- `MetadataFileList.tsx` filter section gains labeled header (`SlidersHorizontal` icon + "Filter" text) and improved chip layout
- `ProcessingTab.tsx` pill styling extracted to CSS classes (`ab-processing-pill`, `ab-processing-pill-dot`), flex-wrap for narrow panels
- Asset protocol in `lib.rs`: retries file read once after 100ms on failure, adds `Cache-Control: no-store, must-revalidate` to prevent stale cached previews; 404 responses also get `Cache-Control: no-store`

### Fixed

#### Composite Cache Empty Without Auto-Stretch
- `core/compose/rgb.rs` `process_rgb()` only saved pre-stretch channel data when `auto_stretch=true`, and `cmd/compose.rs` gated cache insertion on `auto_stretch`
- Result: `export_fits_rgb` fell back to raw disk files, losing alignment, white balance, SCNR, and dimension harmonization
- Fixed by always saving pre-stretch data (`pre_stretch_r/g/b`, `stats_wb_r/g/b` fields added to `ProcessedRgb`) and always populating the composite cache

#### ASDF Fallback Path in RGB Export
- `cmd/image.rs` `export_fits_rgb` fallback: G/B channels used `load_fits_array()` (FITS-only) while R used `extract_image_resolved()` (ASDF+FITS)
- All three channels now use `extract_image_resolved()` for consistent ASDF/FITS dispatch

#### PSF Subpixel Peak Determinant Guard Inverted
- `core/imaging/psf_estimation.rs` `subpixel_peak()` determinant guard was `det > 0.0`, which rejects valid peaks (positive definite Hessian) and accepts saddle points
- Changed to `det < 0.0`

#### Dark PNG Export from Composite
- `cmd/image.rs` `export_rgb_png` rendered linear cache data without STF stretch, producing near-black PNGs
- Added per-channel STF stretch support with `apply_stf_stretch` flag and per-channel shadow/midtone/highlight parameters

#### PSF FWHM Measurement Rewritten
- Previous implementation used 1D line scans along X and Y axes for half-maximum crossings, inaccurate for non-axis-aligned PSFs and noisy data
- Replaced with moment-based measurement: second-order weighted intensity moments within a 12px radius, eigenvalue decomposition via trace/determinant for major/minor axes, FWHM from `2*sqrt(2*ln(2)) * sqrt(lambda)`
- Subpixel peak estimation via quadratic interpolation of the 3x3 Hessian neighborhood
- Local background estimation (annular median at 10px radius) subtracted before threshold calculation

#### SIMD Log Approximation Precision
- `math/simd.rs` `fast_log_avx2` used a degree-3 polynomial for `log2(m)` on [1, 2), giving ~4 bits of mantissa precision (~16 ULP)
- Replaced with Cephes `logf` approach: mantissa in [0.5, 1.0), range reduction to [sqrt(0.5), sqrt(2)], degree-8 minimax polynomial computing `ln(1+f) = f - 0.5*f^2 + f^3*P(f)`
- Peak error: ~1.4e-8 (< 1 ULP for f32, ~24 bits). SIMD and scalar paths now converge to the same precision

#### Drizzle Sigma Clipping Constant Drift
- `core/stacking/drizzle.rs` `DrizzleAccumulator::finalize` used hardcoded `1.4826` instead of `MAD_TO_SIGMA` constant
- Now imports and uses `MAD_TO_SIGMA` from `types::constants`

#### Composite Cache Keys Duplicated as String Literals
- `cmd/image.rs` used inline `"__composite_r/g/b"` strings in 6 locations while `cmd/compose.rs` defined them as file-local `const`
- Constants moved to `types/constants.rs` and shared across both modules

#### TypeScript Type Widening in File List
- `App.tsx` `toMetadataFiles` fallback `status: "queued" as const` widened to `string` in `.map()` union context, causing TS2322 against `MetadataFile[]`
- Cast fallback `as MetadataFile`; added `f.error ?? undefined` for `null`-to-`undefined` conversion

### Changed

#### Performance
- `core/alignment/affine.rs` triangle descriptor generation parallelized: `into_par_iter().flat_map()` replaces triple nested sequential loop for top-50 stars
- `core/analysis/star_detection.rs` tile-based background estimation parallelized: tile coordinates collected first, then `par_iter().filter_map()` for sigma-clipped stats per tile
- `core/imaging/stats.rs` `compute_image_stats` rewritten: per-chunk `Vec<f32>` accumulation replaced with `par_chunks().map().reduce()` for min/max/sum/count, eliminating intermediate pixel storage and concatenation
- `core/compose/rgb.rs` `channel_or_synth` rewritten: two alt channel clones + heap allocation for average replaced with `Zip::par_for_each` writing directly to pre-allocated output

#### Code Quality
- 7 new Tauri commands: `restretch_composite_cmd`, `clear_composite_cache_cmd`, `export_aligned_channels_cmd`, `export_png`, `export_rgb_png`, `generate_synth_cmd`, `generate_synth_stack_cmd`
- 4 removed commands: `get_raw_pixels_binary`, `get_tile`, `pixel_to_world`, `world_to_pixel`
- Net command count: 42 to 45
- WCS cache in `cmd/astrometry.rs` removed (34 lines of `LazyLock<RwLock<WcsCache>>` with manual capacity management), relying on `GLOBAL_IMAGE_CACHE` instead
- `cmd/compose.rs` STF JSON keys replaced with `RES_SHADOW`/`RES_MIDTONE`/`RES_HIGHLIGHT` constants; per-channel STF returned as `stf_r`/`stf_g`/`stf_b` objects
- `cmd/image.rs` hardcoded string keys (`"apply_stf"`, `"copy_wcs"`, `"copy_metadata"`, `"file_size_bytes"`) replaced with shared constants
- `cmd/psf.rs` and `cmd/pipeline.rs` migrated to shared constants for all JSON keys
- `cmd/visualization.rs` dead `get_tile` command removed (19 lines)
- `infrastructure/tauri/index.ts` `withDirInvoke` narrowed to non-exported; `getPreviewUrl` export removed from barrel
- `shared/types/index.ts` cleaned: removed unused re-exports (`ResampleResult`, `StarDetectionResult`, `Star`, `RgbComposeResult`, `DrizzleRgbResult`, `DrizzleRgbOptions`, `CubeInfo`, `WorldCoord`, `PixelCoord`, `FileStatus`, `AppConfig`, `ApiKeyResult`)
- `shared/types/astrometry.types.ts` removed `WorldCoord` and `PixelCoord` interfaces (corresponding commands removed)
- Dead type files removed: `compose.types.ts`, `cube.types.ts`
- 20+ new constants in `types/constants.rs` for JSON keys, composite keys, and PSF result fields
- Backend grew from ~16,200 to ~17,900 lines; frontend from ~12,650 to ~13,700 lines
- `APP_VERSION` updated to `v0.4.2`

## [0.4.1] -- 2026-03-25

### Fixed

#### Constants Swap
- `types/constants.rs` `DIMENSIONS` had value `"align_method"` and vice-versa, causing inverted JSON keys in compose and export responses
- `export_aligned_channels_cmd` in `cmd/compose.rs` also used the constants in reversed positions (DIMENSIONS for method, ALIGN_METHOD for dims)

#### Double Flat Normalization
- `core/stacking/calibration.rs` `divide_flat` re-normalized by median after `create_master_flat` (domain) had already normalized by mean
- Effective division was `pixel / (flat * inv_mean * inv_median)` instead of `pixel / flat_normalized`
- Removed re-normalization from `divide_flat`, trusting `create_master_flat`

#### Export Dimension Mismatch with Mixed SW/LW JWST Data
- `cmd/image.rs` `export_fits_rgb` and `export_rgb_png` loaded original paths without harmonizing dimensions
- With mixed SW/LW (e.g. F444W 5657x2207 + F200W 11471x5993), header declared R dims but data contained 3 arrays of different sizes, causing "Unexpected end of file" errors
- Now uses composite cache (`__composite_r/g/b`) first (already aligned), with `resample_image` fallback to largest dimension
- `infra/fits/writer.rs` validates R/G/B dimensions match before writing

#### FITS Reader Crash on Truncated Trailing HDU
- `infra/fits/reader.rs` `scan_all_hdus` bailed unconditionally if remaining bytes < BLOCK_SIZE
- Now checks `offset + BLOCK_SIZE > mmap.len()` before parse; if parse fails and HDUs already exist, breaks gracefully
- Same guard applied to `extract_cube_mmap`

#### Composite Preview Not Showing in Main View
- `PreviewPanel.tsx` `useAdvancedViewer` condition `!useGpu || !rawPixels` was always true when GPU off, forcing `AdvancedImageViewer` which ignores `compositePreviewUrl`
- Fixed to `!compositePreviewUrl && (!useGpu || !rawPixels)`

#### GPU Button Starts Purple (Appears Active When Not)
- `PreviewPanel.tsx` `probeGpu` auto-enabled GPU: `if (available) setUseGpu(true)`, showing purple button without user interaction
- Removed auto-enable; GPU starts off (gray), user opts in explicitly

#### Custom Palette Does Nothing Visible
- `SmartChannelMapper.tsx` selecting "Custom" only cleared `autoMapSource` but kept auto-mapped channels in slots
- Now clears all channel assignments (`L/R/G/B = null`) when Custom is selected, giving empty slots for manual assignment

### Changed

#### Math / Astrophysics
- Sigma clipping in `calibration_pipeline.rs` `sigma_clipped_mean_stack` and `drizzle.rs` `DrizzleAccumulator::finalize` replaced mean/stddev with median/MAD (Stetson 1987, HST DrizzlePac standard) using `select_nth_unstable_by` for in-place selection

#### Performance
- `core/stacking/drizzle.rs` `drizzle_frame` parallelized: rows computed via `into_par_iter`, contributions collected in parallel, push sequential; eliminates the dominant sequential bottleneck for 130+ frame stacks at 2048x2048 scale=2x
- `core/imaging/wavelet.rs` buffer reallocation eliminated: `mem::take` + `vec![0.0; npix]` replaced with `mem::swap` + `par_iter_mut` zero in-place; zero heap allocations in decompose loop
- `domain/calibration.rs` `median_combine_row_major` micro-allocations eliminated: 16.7M `Vec::with_capacity(n)` + free for 4096x4096 with 20 frames replaced with `par_chunks_mut(cols)` per row, buffer reused via `vals.clear()`
- `core/imaging/stats.rs` `compute_image_stats` double pixel storage eliminated: per-chunk Vecs + concatenation replaced with `par_chunks` reduce for min/max/sum (no pixel storage), then single pass to collect valid pixels
- `core/compose/rgb.rs` `channel_or_synth` unnecessary clones eliminated: two alt channel clones + third allocation for average replaced with `Zip::par_for_each` writing directly to output
- `infra/fits/writer.rs` BufWriter increased from 256KB to 2MB, reducing syscall count for large JWST files (500MB+)

#### Code Quality
- 24 new constants added to `types/constants.rs` for JSON response keys
- 4 cmd files cleaned (`compose.rs`, `image.rs`, `psf.rs`, `pipeline.rs`): zero remaining hardcoded string keys in `json!()` macros
- Removed dead constants: `COPY_WS` (typo of `COPY_WCS`), `COPY_CRPIX`, `COPY_CRVAL`, `COPY_CD`
- Fixed `COPY_WS` usage in `export_fits_rgb` (was emitting `"copy_ws"` instead of `"copy_wcs"`)

## [0.4.0] -- 2026-03-20

### Added

#### Star-Based Affine Alignment
- Triangle asterism matching (`core/alignment/affine.rs`, 741 lines) with configurable star limit (80), minimum triangle side (20px), tolerance-based ratio matching, and angular vertex sorting for consistent pairing
- RANSAC robust estimation (500 iterations) supporting full affine (6-DOF) and rigid (4-DOF) transform fitting with automatic fallback: affine -> rigid -> phase correlation -> identity
- Sanity checks on computed transforms: max 25% offset, max 5 degrees rotation, scale range 0.85-1.15, max 3px residual, min 30% inlier ratio
- `warp_image` function using bicubic Catmull-Rom interpolation (imported from `resample` module, eliminating code duplication) with parallel row processing via Rayon
- Dual alignment mode in RGB compose: `AlignMethod::PhaseCorrelation` (default, sub-pixel bicubic) and `AlignMethod::Affine` (star-based, handles rotation)
- `align_method` parameter in `compose_rgb_cmd` Tauri command ("phase_correlation" or "affine")
- Alignment method selector in RgbComposePanel frontend: "Phase Correlation (sub-pixel)" and "Star-based Affine (rotation)"

#### Improved White Balance
- Stability-based auto white balance replacing fixed G-channel reference: selects the channel with lowest MAD/median ratio (coefficient of variation) as reference, preventing noise amplification in narrowband data where G is not always the most stable channel
- Applied consistently in both `core/compose/rgb.rs` and `core/compose/drizzle_rgb.rs`
- Frontend WB label updated from "Auto (Median)" to "Auto (Stability)"

#### Improved SCNR (Green Noise Removal)
- Luminance redistribution to R and B channels when `preserve_luminance` is enabled: lost luminance from green reduction is redistributed proportionally to R and B using ITU-R BT.709 weights, replacing the previous circular adjustment that only modified G
- Pre-computed BT.709 constants (`LUM_R`, `LUM_G`, `LUM_B`, `INV_RB_WEIGHT`) for zero per-pixel division overhead
- Signature changed to `(&mut r, &mut g, &mut b)` for in-place modification of all three channels
- `amount` blending applied before luminance redistribution for correct ordering

#### Plate Solving
- Auto-downsample for large images in `cmd/astrometry.rs`: images >2048px are area-downsampled to a temporary FITS before upload, preventing HTTP 413 errors on JWST-sized files (200+ MB)
- Robust JSON parsing in `domain/plate_solve.rs`: replaced `reqwest::Response::json()` with `.text()` + `serde_json::from_str()` to handle astrometry.net's non-standard `text/plain` content-type responses
- API key validation: early bail with clear error message when no key is configured
- Per-stage logging: session creation, file upload size, job polling progress, and solve result (RA/Dec/scale/orientation)
- Star detection on full-resolution image before downsample for maximum astrometric precision

#### Frontend Refactoring (Phases 1-8)
- `useBackend.ts` (monolithic 41-command hook) split into 11 domain-specific services: `compose.service.ts`, `fits.service.ts`, `analysis.service.ts`, `header.service.ts`, `cube.service.ts`, `stretch.service.ts`, `visualization.service.ts`, `stacking.service.ts`, `config.service.ts`, `astrometry.service.ts`, `export.service.ts`
- Shared `infrastructure/tauri/` IPC layer providing `safeInvoke`, `withPreview`, and `isTauri` helpers
- 18 JSX files converted to TSX with full type annotations
- Monolithic `types.ts` split into domain-specific type files (`compose.types.ts`, `fits.types.ts`, `stacking.types.ts`, etc.)
- `GpuContext.js` migrated to TypeScript
- 8 shared UI primitives built: Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader
- 7 remaining panels refactored to use shared primitives, ~1,370 lines removed total
- Backward-compatibility shims removed after migration verified

### Fixed

#### Numerical Correctness
- NaN ordering in median and MAD computation (`math/median.rs`): NaN values are now sorted to the end (Greater), preventing corrupted median/MAD/sigma-clipping across all operations that depend on sorted data
- Hann window missing 2pi factor (`core/analysis/fft.rs`): window function was `sin(pi*i/N)` instead of correct `0.5*(1-cos(2pi*i/N))`, producing a triangular window instead of Hann; affected FFT power spectrum display and phase correlation accuracy
- Polynomial background basis function (`core/imaging/background.rs`): replaced iterative division-based power computation with correct `y.powi(y_pow) * x.powi(x_pow)`, fixing numerical drift at higher polynomial degrees (3+) that caused visible artifacts in background model
- Richardson-Lucy deconvolution Tikhonov regularization (`core/analysis/deconvolution.rs`): moved regularization from ratio computation to update step as multiplicative damping `inv_reg = 1/(1+lambda)`, which is numerically stable and matches the standard RL formulation; fixed operator precedence in threshold clamp `(orig * (1-t)).max(0)` vs `orig * (1-t).max(0)`
- Phase correlation confidence metric: replaced `peak/mean` with proper z-score `(peak-mean)/sigma`, providing a statistically meaningful signal-to-noise measure; added `n < 2` guard to prevent division by zero in variance computation

#### Performance
- FFT planner hoisted out of `correlate_single` in phase correlation: single `FftPlanner` instance is created once in `phase_correlate` and passed through the coarse-refine pipeline, eliminating redundant twiddle factor computation on each call
- Peak finding parallelized in phase correlation surface: `par_iter().reduce_with()` replaces sequential double loop, significant for large FFT surfaces (512x512+)
- Confidence computation parallelized: `par_iter().sum()` for mean and variance
- RANSAC mask allocation hoisted out of loop (`core/alignment/affine.rs`): single `Vec<bool>` reused via `fill(false)` + `copy_from_slice` instead of allocating 500 vectors per RANSAC run
- Drizzle accumulator flat storage (`core/stacking/drizzle.rs`): replaced `Vec<Vec<f32>>` (one heap allocation per pixel) with flat `Vec<f32>` + `Vec<u16>` count array, eliminating millions of small allocations for typical 4K+ output images
- Sigma-clipped combine deviation buffer reuse (`core/stacking/combine.rs`): `devs` vector allocated once and reused via `clear()`+`extend()` per pixel instead of allocating per iteration
- Cube mean computation z-order traversal (`math/simd.rs`): replaced per-pixel column extraction (cache-hostile) with per-slice iteration (cache-friendly), plus contiguous slice fast-path

#### Alignment Correctness
- Sub-pixel offset return order in `compute_subpixel_offset` (`core/stacking/align.rs`): was returning `(sub_dx, sub_dy)`, now correctly returns `(sub_dy, sub_dx)` matching the (row, col) convention used by ndarray; callers in `stacking/drizzle.rs` updated to destructure as `(dy, dx)`
- Removed artificial `abs() > 1e-7` threshold in ZNCC alignment correlation that rejected valid near-zero astronomical background pixels

#### Calibration
- Flat field normalization (`core/stacking/calibration.rs`): flat frame is now normalized by its own median before division, preventing scale-dependent artifacts when flat values are far from 1.0

#### Plate Solving
- `cmd/astrometry.rs` import path: `crate::domain::config` (non-existent module) replaced with `crate::infra::config` where `load_api_key` and `load_config` actually live
- `detect_stars` call signature: was `detect_stars(&result.pixels, naxis1, naxis2, None)`, corrected to `detect_stars(&result.image, 5.0)` matching actual API (`&Array2<f32>, f64 -> DetectionResult`)
- `solve_astrometry_net` missing feature gate: call now wrapped in `#[cfg(feature = "astrometry-net")]` with fallback to `solve_offline_placeholder` when feature is disabled
- Astrometry.net API response parsing: `.json().await?` fails on responses with `Content-Type: text/plain`; replaced with `.text().await?` + `serde_json::from_str()` across all 6 API calls (login, upload, submission status, job status, calibration, job info)
- HTTP 413 Payload Too Large on JWST images: added automatic area-downsample to 2048px max before upload, reducing typical payload from 200+ MB to ~16 MB

### Changed
- `ChannelStats::from(&ImageStats)` trait impl replaces standalone conversion functions in compose modules
- Offsets in `ProcessedRgb` and `RgbComposeResult` are `(f64, f64)` preserving sub-pixel precision (were `(i32, i32)` in the feature branch)
- `RgbComposeConfig` gains `align_method: AlignMethod` field (default: `PhaseCorrelation`)
- `types/constants.rs` gains `DEFAULT_API_KEY_SERVICE` and `DEFAULT_ASTROMETRY_API_URL`
- `cmd/astrometry.rs` uses `crate::infra::config` (correct module path) with constants instead of hardcoded strings
- `AffineAlignMethod` enum (internal to `affine.rs`) disambiguated from config-level `AlignMethod` to prevent naming conflicts
- Frontend `compose.types.ts` gains `DimensionCrop`, `AlignMethod`, full `RgbComposeResult` fields
- Frontend offset display uses `.toFixed(2)` for sub-pixel precision
- `useBackend.ts` replaced by 11 domain-specific services in `services/` with shared `infrastructure/tauri/` layer
- Removed dead `domain/pipeline.rs` (142 lines, zero callers after `cmd/pipeline.rs` was rewritten to use `core::imaging::calibration_pipeline` directly)

## [0.3.0] -- 2026-03-08

### Added

#### Image Enhancement
- Richardson-Lucy deconvolution (`core/analysis/deconvolution.rs`, 394 lines) -- FFT-based iterative deconvolution with Gaussian PSF generation, configurable iterations, Tikhonov regularization, deringing with threshold control, and convergence tracking
- `deconvolve_rl_cmd` Tauri command with progress events (`deconv-progress`) and FITS + PNG output
- DeconvolutionPanel frontend (298 lines) with PSF sigma/size sliders, iteration control, regularization, deringing toggle, and real-time progress
- Background extraction (`core/imaging/background.rs`, 502 lines) -- polynomial surface fitting with configurable grid size (3-32), polynomial degree (1-5), sigma-clipped sampling, and subtract/divide correction modes
- `extract_background_cmd` Tauri command outputting corrected image, background model PNG, and corrected FITS with RMS residual stats
- BackgroundPanel frontend (213 lines) with grid size, polynomial degree, sigma clip, iteration controls, and model preview
- Wavelet denoise (`core/imaging/wavelet.rs`, 359 lines) -- a trous wavelet transform with per-scale sigma thresholds, linear/nonlinear modes, MAD noise estimation, and configurable scale count (1-8)
- `wavelet_denoise_cmd` Tauri command with per-scale progress events (`wavelet-progress`)
- WaveletPanel frontend (276 lines) with scale count, per-scale threshold sliders, linear toggle, and noise estimate display
- ProcessingTab (`preview/ProcessingTab.tsx`) -- unified processing chain view with Background --> Denoise --> Deconvolution pipeline, chain indicator showing processing steps, and reset per stage

#### ASDF Format Support
- ASDF parser (`infra/asdf/parser.rs`, 196 lines) -- reads ASDF preamble, YAML tree (via serde_yaml), and binary blocks with magic validation
- ASDF block reader (`infra/asdf/blocks.rs`, 201 lines) -- supports zlib, bzip2, and lz4 block decompression with checksum fields
- ASDF tree traversal (`infra/asdf/tree.rs`, 239 lines) -- NdArrayMeta extraction (shape, dtype, byte order, block source), DType parsing (float32/64, int8/16/32/64, uint8/16/32), WcsInfo extraction from both classic WCS and gWCS (Roman Space Telescope)
- ASDF-to-image converter (`infra/asdf/converter.rs`, 285 lines) -- Roman Space Telescope data model traversal (`roman.datamodels.maker_utils`), multi-key array search (`data`, `roman.data`, nested science arrays), pixel type conversion to f32, and metadata extraction
- AsdfImage-to-Array2 bridge (`infra/asdf_bridge.rs`, 95 lines) -- transparent conversion with HduHeader synthesis from ASDF metadata and WCS fields
- Auto-dispatch in `extract_image_resolved` and `load_cached` -- `.asdf` files detected by extension and routed through ASDF pipeline transparently alongside FITS
- Frontend ASDF support -- `.asdf` added to `VALID_EXTENSIONS` in validation.ts, DropZone and EmptyState updated with `.asdf` labels
- New Cargo dependencies: `serde_yaml`, `flate2`, `bzip2`, `lz4_flex`

#### Pipeline & Stacking
- Smart pipeline (`domain/pipeline.rs`) -- auto-detects 2D images vs 3D cubes per file; 2D files routed to `extract_image_mmap` with asinh normalize + PNG render + FITS export, 3D files routed to `cube::process_cube`; `SingleResult` enum with `#[serde(tag = "type")]` discriminator
- `run_pipeline_cmd` updated to extract `png_path` (from first 2D result) and `collapsed_path` (from first 3D result) for frontend preview
- PipelinePanel frontend (277 lines) with file selector, frame step slider, workflow summary (calibration/stack config), per-result expandable details, and calibrated file integration
- Crop-to-intersection stacking (`core/stacking/combine.rs`) -- replaces hard `bail!` on dimension mismatch with automatic crop to `min_rows x min_cols` intersection across all frames before alignment and sigma-clipped combination
- StackingTab (`preview/StackingTab.tsx`) -- tabbed view with Calibrate, Stack, and Pipeline sections

#### Lazy Cube Processing
- `LazyCube` struct (`domain/lazy_cube.rs`, 367 lines) -- mmap-backed random-access cube with LRU frame cache (default 64 frames), on-demand frame decoding, and global statistics computation
- `process_cube_lazy_cmd` Tauri command for memory-efficient cube processing
- `CubeFrameNav` component for frame-by-frame navigation in cube data

#### Architecture & Layout
- IntelliJ-style panel layout in PreviewPanel -- persistent center preview, collapsible bottom tab bar (Info | Analysis | Headers | Export), collapsible right side panel (Processing | Compose | Stacking | Config), right icon bar with active indicators
- 6 split PreviewContexts (`context/PreviewContext.tsx`) -- FileCtx, HistCtx, CubeCtx, RgbCtx, RenderCtx, RawPixelsCtx, NarrowbandCtx for granular re-render control
- `core/` layer extraction -- pure bounded contexts (imaging, stacking, compose, analysis, astrometry, cube, metadata) separated from `domain/` orchestration layer
- Resizable panels with drag handles -- bottom panel (100-500px) and side panel (280-600px) with smooth cursor feedback
- Tab-based lazy loading with `React.lazy()` and `Suspense` for all panel content (PreviewTab, AnalysisTab, ProcessingTab, ComposeTab, HeadersTab, ExportTab, StackingTab, ConfigTab)

#### Image Cache
- LRU image cache (`infra/cache.rs`, 295 lines) -- `Arc<CachedImage>` zero-copy sharing, `RwLock`-based thread safety, configurable capacity, `get_or_load` and `get_or_load_full` (with header) variants
- `load_from_cache_or_disk` helper for cache-first loading in processing commands

#### Configuration
- Persistent app configuration (`infra/config.rs`) -- JSON config file in platform config dir (`~/.config/astroburst/config.json`), field-level update via `update_config_field`, API key storage per service
- `get_config`, `update_config`, `save_api_key`, `get_api_key` Tauri commands
- ConfigPanel frontend (`preview/ConfigTab.tsx`) for settings management

#### Rendering
- Percentile-based tile stretch (`infra/render/tiles.rs`) -- `percentile_bounds(P0.1, P99.9)` replaces raw min/max for tile pyramid generation, preventing outliers/cosmic rays from collapsing the display range to black
- Progress events for deconvolution, background extraction, wavelet denoise, calibration, and stacking via `ProgressHandle` with stage labels and percentage tracking

### Fixed
- Pipeline crash on 2D FITS files -- pipeline exclusively called `cube::process_cube` which requires NAXIS=3; now auto-detects dimensionality and routes 2D images through `extract_image_mmap`
- DeepZoom viewer rendering completely black -- frontend `getTileUrl` generated path `${outputDir}/tiles/${level}/${x}/${y}.png` but backend saves tiles as `${outputDir}/${level}/${x}_${y}.png`; fixed both the frontend path template and the `get_tile` command
- Stacking bail on dimension mismatch -- `stack_images` rejected frames with even slightly different dimensions (e.g., JWST 4179x7059 vs 4177x7065); replaced with crop-to-intersection
- Tile pyramid black on JWST data -- `generate_tile_pyramid` used raw `find_minmax_simd` which includes outliers/cosmic rays in the stretch range, mapping all science pixels to ~0; replaced with percentile clipping (P0.1/P99.9) filtering invalid pixels
- `get_tile` command path inconsistency -- returned `{}/tiles/{}/{}/{}.png` while generator creates `{}/{}/{}_{}.png`

### Changed
- `SingleResult` enum in pipeline now uses `#[serde(tag = "type")]` with `Cube`, `Image`, and `Err` variants instead of `Ok`/`Err`
- Tauri command count increased from 37 to 42 (added `deconvolve_rl_cmd`, `extract_background_cmd`, `wavelet_denoise_cmd`, `get_config`, `update_config`)
- Backend codebase grew from ~8,400 to ~12,200 lines of Rust across 40+ modules
- Frontend codebase grew from ~4,700 to ~10,000 lines across 45+ components
- `useBackend.ts` now exposes 41 command wrappers (21 currently unwired to UI)
- `ndarray` slicing used for stacking crop (`s![..min_rows, ..min_cols]`) instead of dimension validation bail
- Tile renderer uses `percentile_bounds` instead of `find_minmax_simd` for global stretch range

## [0.2.0] -- 2026-03-01

### Added
- Multi-Extension FITS (MEF) support with automatic SCI extension selection and primary header merging
- HDU scanner with `HduInfo` metadata for all extensions in a file
- Header merge logic combining primary and extension headers with keyword deduplication
- Bicubic resampling module (`domain/resample.rs`) using Catmull-Rom interpolation with Rayon row-parallelism
- `resample_fits_cmd` Tauri command for standalone FITS resampling with WCS header update
- Auto-resample in batch pipeline -- detects resolution groups (>1.5x area ratio) and resamples larger group to match smaller, preserving original files as `{name}_resampled.fits`
- WCS header update on resample -- scales CRPIX1/2, CD matrix (CD1_1/CD1_2/CD2_1/CD2_2), and CDELT1/2 proportionally to dimension change
- `HduHeader::set()` and `HduHeader::set_f64()` methods for mutable keyword updates
- Auto-resample checkbox in processing toolbar with progress indicator
- `ResampleBadge` component showing original --> resampled dimensions with tooltip
- `FILE_RESAMPLED` reducer action in `useFileQueue` for tracking resampled files
- FITS writer (`domain/fits_writer.rs`) with `write_fits_image` (mono) and `write_fits_rgb` (3-plane) output, WCS/observation metadata copy, and FITS-standard header formatting
- Narrowband filter detection (`domain/header_discovery.rs`) -- regex-based identification of H-alpha, [OIII], [SII] from FITS keywords (FILTER, FILTNAM, INSTRUME), wavelength values (WAVELEN, CRVAL3), and filename patterns
- Hubble Palette (SHO) auto-suggestion with confidence scoring (High/Medium/Low) based on keyword source
- `suggest_palette()` function for automatic R/G/B channel assignment from multiple files
- `detect_narrowband_filters` Tauri command for batch filter detection
- Header Explorer panel with categorized keyword browser (Observation, Instrument, Image, WCS, Processing), keyword search, value copy, and filter detection badge with channel assign button
- FFT power spectrum panel with 2D Fourier analysis, log-magnitude heatmap colormap, frequency coordinate readout on hover, and DC/max magnitude display
- Dimension harmonization in `rgb_compose` with configurable tolerance (default 100px, minimum 1% of image size) -- auto-crops channels with small size differences instead of rejecting
- `DimensionCrop` metadata in RGB compose result tracking original and cropped dimensions per channel
- Drizzle RGB pipeline (`domain/drizzle_rgb.rs`) combining per-channel drizzle stacking with RGB composition, white balance, and SCNR
- Concurrent batch processing with 3 workers and `requestAnimationFrame` yields for UI responsiveness
- Binary IPC (`get_raw_pixels_binary`) for zero-copy f32 pixel transfer to WebGPU renderer
- `extract_image_mmap_by_index()` for loading specific HDU by index
- `list_extensions()` for querying all HDUs in a FITS file
- Folder selection dialog for batch loading all FITS files from a directory
- Confetti animation on batch completion
- Version display (v0.2.0) in footer

### Fixed
- HeaderExplorerPanel infinite re-render loop -- `onLoadHeader` callback reference changed every render due to `useBackend()` creating new function instances; fixed with `useRef` for callback stability and `prevPathRef` guard to only trigger on actual file path changes
- UI freeze during auto-resample -- added `yieldToUI()` calls between resample iterations to allow React to paint progress updates

### Changed
- Histogram upgraded from 512 to 16384 bins for better dynamic range representation
- Tauri command count increased from 22 to 37
- Architecture diagram updated to reflect new domain modules (resample, fits_writer, header_discovery, fft, scnr)

## [0.1.0] -- 2026-02-28

### Added
- FITS I/O with memory-mapped extraction and ZIP transparency
- Batch processing with Rayon thread pool
- Asinh stretch and STF (Screen Transfer Function)
- Bias/Dark/Flat calibration pipeline
- Sigma-clipped stacking with configurable thresholds
- Drizzle integration (Square/Gaussian/Lanczos3 kernels)
- RGB composition with white balance (auto/manual/none) and SCNR
- 512-bin histogram with median, mean, sigma, MAD
- Star detection with PSF-fitting (flux, FWHM, SNR)
- FITS header summary table
- WCS transforms (pixel <-> world coordinates)
- Plate solving via astrometry.net API
- IFU/datacube processing with spectrum extraction
- WebGPU compute shader rendering with Canvas 2D fallback
- Deep zoom tile pyramid for large images
- Zero-copy binary IPC via Tauri Response
- FITS export with WCS/metadata preservation
- Cross-correlation auto-alignment

[Unreleased]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.2...HEAD
[0.4.2]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/tag/v0.1.0
