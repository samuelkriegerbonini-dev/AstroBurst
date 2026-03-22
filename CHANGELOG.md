# Changelog

All notable changes to AstroBurst will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- macOS and Linux installer scripts
- GitHub Actions CI/CD pipeline
- Sample HST narrowband FITS files for testing
- Open-source community files (CONTRIBUTING, CODE_OF_CONDUCT, SECURITY)

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

[Unreleased]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/tag/v0.1.0
