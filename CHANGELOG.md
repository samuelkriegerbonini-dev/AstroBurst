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

## [0.2.0] — 2026-03-01

### Added
- Multi-Extension FITS (MEF) support with automatic SCI extension selection and primary header merging
- HDU scanner with `HduInfo` metadata for all extensions in a file
- Header merge logic combining primary and extension headers with keyword deduplication
- Bicubic resampling module (`domain/resample.rs`) using Catmull-Rom interpolation (α = −0.5) with Rayon row-parallelism
- `resample_fits_cmd` Tauri command for standalone FITS resampling with WCS header update
- Auto-resample in batch pipeline — detects resolution groups (>1.5× area ratio) and resamples larger group to match smaller, preserving original files as `{name}_resampled.fits`
- WCS header update on resample — scales CRPIX1/2, CD matrix (CD1_1/CD1_2/CD2_1/CD2_2), and CDELT1/2 proportionally to dimension change
- `HduHeader::set()` and `HduHeader::set_f64()` methods for mutable keyword updates
- Auto-resample checkbox in processing toolbar with progress indicator
- `ResampleBadge` component showing original → resampled dimensions with tooltip
- `FILE_RESAMPLED` reducer action in `useFileQueue` for tracking resampled files
- FITS writer (`domain/fits_writer.rs`) with `write_fits_image` (mono) and `write_fits_rgb` (3-plane) output, WCS/observation metadata copy, and FITS-standard header formatting
- Narrowband filter detection (`domain/header_discovery.rs`) — regex-based identification of Hα, [OIII], [SII] from FITS keywords (FILTER, FILTNAM, INSTRUME), wavelength values (WAVELEN, CRVAL3), and filename patterns
- Hubble Palette (SHO) auto-suggestion with confidence scoring (High/Medium/Low) based on keyword source
- `suggest_palette()` function for automatic R/G/B channel assignment from multiple files
- `detect_narrowband_filters` Tauri command for batch filter detection
- Header Explorer panel with categorized keyword browser (Observation, Instrument, Image, WCS, Processing), keyword search, value copy, and filter detection badge with channel assign button
- FFT power spectrum panel with 2D Fourier analysis, log-magnitude heatmap colormap, frequency coordinate readout on hover, and DC/max magnitude display
- Dimension harmonization in `rgb_compose` with configurable tolerance (default 100px, minimum 1% of image size) — auto-crops channels with small size differences instead of rejecting
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
- HeaderExplorerPanel infinite re-render loop — `onLoadHeader` callback reference changed every render due to `useBackend()` creating new function instances; fixed with `useRef` for callback stability and `prevPathRef` guard to only trigger on actual file path changes
- UI freeze during auto-resample — added `yieldToUI()` calls between resample iterations to allow React to paint progress updates

### Changed
- Histogram upgraded from 512 to 16384 bins for better dynamic range representation
- Tauri command count increased from 22 to 37
- Architecture diagram updated to reflect new domain modules (resample, fits_writer, header_discovery, fft, scnr)

## [0.1.0] — 2026-02-28

### Added
- FITS I/O with memory-mapped extraction and ZIP transparency
- Batch processing with Rayon thread pool
- Asinh stretch and STF (Screen Transfer Function)
- Bias/Dark/Flat calibration pipeline
- Sigma-clipped stacking with configurable thresholds
- Drizzle integration (Square/Gaussian/Lanczos3 kernels)
- RGB composition with white balance (auto/manual/none) and SCNR
- 512-bin histogram with median, mean, σ, MAD
- Star detection with PSF-fitting (flux, FWHM, SNR)
- FITS header summary table
- WCS transforms (pixel ↔ world coordinates)
- Plate solving via astrometry.net API
- IFU/datacube processing with spectrum extraction
- WebGPU compute shader rendering with Canvas 2D fallback
- Deep zoom tile pyramid for large images
- Zero-copy binary IPC via Tauri Response
- FITS export with WCS/metadata preservation
- Cross-correlation auto-alignment

[Unreleased]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/tag/v0.1.0
