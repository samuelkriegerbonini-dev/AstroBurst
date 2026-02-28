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

## [0.1.0] — 2026-02-28

### Added
- FITS I/O with memory-mapped extraction and ZIP transparency
- Batch processing with Rayon thread pool
- Asinh stretch and STF (Screen Transfer Function)
- Bias/Dark/Flat calibration pipeline
- Sigma-clipped stacking with configurable thresholds
- Drizzle integration (Square/Gaussian/Lanczos3 kernels)
- Drizzle RGB pipeline for multi-channel composition
- RGB composition with Hubble palette auto-mapping
- White balance (auto/manual/none) and SCNR
- 512-bin histogram with median, mean, σ, MAD
- FFT power spectrum analysis
- Star detection with PSF-fitting (flux, FWHM, SNR)
- FITS header explorer with categorized browser
- WCS transforms (pixel ↔ world coordinates)
- Plate solving via astrometry.net API
- IFU/datacube processing with spectrum extraction
- WebGPU compute shader rendering with Canvas 2D fallback
- Deep zoom tile pyramid for large images
- Zero-copy binary IPC via Tauri Response
- FITS export with WCS/metadata preservation
- Cross-correlation auto-alignment

[0.1.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/tag/v0.1.0
