<p align="center">
  <img src="src/assets/logo.png" alt="AstroBurst Logo" width="128" />
</p>

<h1 align="center">AstroBurst</h1>

<p align="center">
  <strong>High-Performance Astronomical Image Processor</strong><br>
  <em>Rust + Tauri + WebGPU. Process JWST, Hubble, and Roman Space Telescope data at native speed.</em>
</p>

<p align="center">
  <a href="https://github.com/samuelkriegerbonini-dev/AstroBurst/releases"><img src="https://img.shields.io/github/v/release/samuelkriegerbonini-dev/AstroBurst?style=flat-square&color=blue" alt="Release"></a>
  <a href="https://github.com/samuelkriegerbonini-dev/AstroBurst/actions"><img src="https://img.shields.io/github/actions/workflow/status/samuelkriegerbonini-dev/AstroBurst/build.yml?style=flat-square" alt="Build"></a>
  <img src="https://img.shields.io/badge/rust-1.75+-orange.svg?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/tauri-2.10-purple.svg?style=flat-square" alt="Tauri">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/samuelkriegerbonini-dev/AstroBurst?style=flat-square&color=green" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg?style=flat-square" alt="Platform">
  <a href="https://ko-fi.com/astroburst"><img src="https://img.shields.io/badge/Ko--fi-Support-FF5E5B?style=flat-square&logo=ko-fi" alt="Ko-fi"></a>
</p>

<p align="center">
  <a href="#installation">Install</a> &middot;
  <a href="#features">Features</a> &middot;
  <a href="#quick-start">Quick Start</a> &middot;
  <a href="#getting-data">Data</a> &middot;
  <a href="#usage">Usage</a> &middot;
  <a href="#architecture">Architecture</a> &middot;
  <a href="#roadmap">Roadmap</a> &middot;
  <a href="#contributing">Contributing</a>
</p>

---

AstroBurst is an open-source desktop app for processing astronomical images. Drop in your FITS or ASDF files, compose RGB from narrowband channels, stack with sigma clipping, and export. Everything runs locally on your machine with no cloud dependencies.

It's built on Rust for the heavy lifting, React for the interface, and WebGPU for real-time preview. The result is a tool that opens a 2 GB IFU datacube in 300 ms, processes 10 frames at 1.4 GB/s, and renders STF adjustments in 8 ms on GPU.

> **What's new in v0.5**: **Drizzle integration** in the ComposeWizard (per-channel super-resolution with scale/pixfrac/kernel), **quality-weighted stacking** driven by the subframe selector, **valid-FITS export** with restored WCS/metadata header preservation plus `PROGRAM`/`HISTORY` provenance cards documenting the processing, and a **shared-luminance star mask** for the masked stretch (no chromatic halos). Major numerical-correctness pass on the alignment/stacking core — phase-correlation sign + DC-pedestal fix (alignment now works on real sky-background data), flux-conserving drizzle weighting, and sub-pixel registration sign fix. Hardened FITS/ASDF parsing against malformed files (checked arithmetic, no overflow/panics), merged the Star Mask step into Stretch, de-duplicated the Export panel, and surfaced previously swallowed errors. 305 backend tests green. See the [changelog](CHANGELOG.md).
>
> **v0.4.x highlights**: 11-step ComposeWizard pipeline with tone curves, non-destructive composite (ORIG/KEY dual cache), masked stretch with star protection, SPCC color calibration, star-based affine alignment with rotation correction, stability white balance (MAD/median reference), SCNR luminance redistribution (BT.709), blend presets (SHO/HOO/Foraxx/Dynamic HOO/Hubble Legacy), live composite STF re-stretch, synthetic FITS generation, subframe selector with per-channel star metrics, and a full numerical correctness audit. The frontend was refactored into 12 domain services with shared UI primitives.

## Screenshots

<p align="center">
  <img src="docs/screenshots/01-rgb-composite.png" alt="Finished JWST Pillars of Creation narrowband composite" width="100%">
</p>
<p align="center"><em>Finished narrowband composite of the <strong>Pillars of Creation</strong> (Hα / OIII / SII → SHO) with the spline tone-curve editor open in the final <strong>Adjust</strong> step. Built end-to-end in the ComposeWizard — the panels below walk through that exact pipeline, in order, on this same dataset.</em></p>

<p align="center">
  <img src="docs/screenshots/02-channels-mapping.png" alt="Channel assignment" width="100%">
</p>
<p align="center"><em><strong>1 · Channels.</strong> The three narrowband frames (502 nm OIII, 656 nm Hα, 673 nm SII) mapped to channels by wavelength with the SmartChannelMapper — per-bin file slots, custom-wavelength channels, and Auto Map. With one frame per channel here, the optional <strong>2 · Stack</strong> step (sigma-clipped integration, subframe selector, drizzle) is skipped.</em></p>

<p align="center">
  <img src="docs/screenshots/03-align.png" alt="Channel alignment" width="100%">
</p>
<p align="center"><em><strong>3 · Align.</strong> Sub-pixel channel registration — Phase Correlation (FFT, default) or star-based affine for rotation/scale, with an automatic fallback chain — so the three narrowband layers overlay exactly.</em></p>

<p align="center">
  <img src="docs/screenshots/04-crop.png" alt="Crop to common field" width="100%">
</p>
<p align="center"><em><strong>4 · Crop.</strong> Trim to the field the channels share after alignment, with manual margins or auto-detected borders to drop the non-overlapping edges left by registration.</em></p>

<p align="center">
  <img src="docs/screenshots/05-background-extraction.png" alt="Background extraction" width="100%">
</p>
<p align="center"><em><strong>5 · Background.</strong> Gradient and light-pollution removal via a grid-sampled polynomial surface with sigma-clipped rejection of stars and nebulosity, in subtract or divide mode — applied per channel.</em></p>

<p align="center">
  <img src="docs/screenshots/06-blend-composite.png" alt="Channel blending into the SHO composite" width="100%">
</p>
<p align="center"><em><strong>6 · Blend.</strong> The SHO palette (SII→R, Hα→G, OIII→B) mapped through a per-channel weight matrix — the Hα-dominated green you see here is exactly what the next step neutralizes. Presets: RGB, SHO, Hubble Legacy, HOO, Dynamic HOO, Foraxx.</em></p>

<p align="center">
  <img src="docs/screenshots/07-color-balance-scnr.png" alt="Color balance and SCNR" width="100%">
</p>
<p align="center"><em><strong>7 · Color.</strong> White balance (Auto / SPCC / Manual / None) plus SCNR green-excess reduction (average/maximum neutral) with luminance redistribution — taming the green SHO cast into a balanced palette. Includes a narrowband-aware warning.</em></p>

<p align="center">
  <img src="docs/screenshots/08-masked-stretch.png" alt="Masked stretch with shared star mask" width="100%">
</p>
<p align="center"><em><strong>8 · Stretch.</strong> Masked Stretch (star-protected) with target background, mask growth, and a shared-luminance star mask to avoid chromatic halos — then the <strong>9 · Adjust</strong> tone curves produce the finished composite shown at the top. Also offers arcsinh and per-channel STF.</em></p>

<details>
<summary><strong>Analysis, processing tools &amp; more results</strong></summary>

<p align="center">
  <img src="docs/screenshots/09-analysis-platesolve.png" alt="Analysis panel" width="100%">
</p>
<p align="center"><em><strong>Analysis.</strong> Star detection with FWHM/SNR and overlay circles, plate solving (RA/Dec, pixel scale, field of view), an FFT power spectrum with a Hann window, and a 16K-bin histogram with auto-STF.</em></p>

<p align="center">
  <img src="docs/screenshots/10-wavelet-denoise.png" alt="Wavelet denoise" width="100%">
</p>
<p align="center"><em><strong>Wavelet denoise.</strong> À-trous wavelet noise reduction with per-scale sigma thresholds (fine detail through very-large structures) and a soft/linear threshold option that preserves faint signal.</em></p>

<p align="center">
  <img src="docs/screenshots/11-psf-estimation.png" alt="PSF estimation" width="100%">
</p>
<p align="center"><em><strong>PSF estimation.</strong> Empirical point-spread-function measurement from sampled stars — count, cutout radius, max ellipticity, and saturation rejection — feeding the deconvolution kernel.</em></p>

<p align="center">
  <img src="docs/screenshots/12-deconvolution.png" alt="Richardson-Lucy deconvolution" width="100%">
</p>
<p align="center"><em><strong>Deconvolution.</strong> FFT-based Richardson-Lucy with Tikhonov regularization, configurable PSF (synthetic or empirical), iterations, and deringing.</em></p>

<p align="center">
  <img src="docs/screenshots/13-arcsinh-stretch.png" alt="Arcsinh stretch" width="100%">
</p>
<p align="center"><em><strong>Arcsinh stretch.</strong> Flux-preserving asinh stretch with a single intensity factor, chainable after denoise or deconvolution and reading the previous tool's output.</em></p>

<p align="center">
  <img src="docs/screenshots/14-header-explorer.png" alt="FITS header explorer" width="100%">
</p>
<p align="center"><em><strong>Headers.</strong> Multi-extension (MEF) selector with auto SCI detection and a categorized FITS keyword browser (observation, instrument, WCS, other).</em></p>

<p align="center">
  <img src="docs/screenshots/15-export-compare.png" alt="Export and before/after compare" width="100%">
</p>
<p align="center"><em><strong>10 · Export &amp; compare.</strong> Before/after inspection against the original, with PNG (8/16-bit) and RGB-FITS export — FITS now carries valid WCS/metadata headers plus <code>PROGRAM</code>/<code>HISTORY</code> provenance cards documenting the applied processing.</em></p>

<p align="center">
  <img src="docs/screenshots/16-synthetic-fits.png" alt="Synthetic FITS generator" width="100%">
</p>
<p align="center"><em><strong>Synthetic data.</strong> Configurable star-field generator with a PSF model (Gaussian/Moffat) and a CCD noise model (gain, read noise, sky background, exposure) for testing calibration and alignment.</em></p>

<p align="center">
  <img src="docs/screenshots/17-calibration-pipeline.png" alt="Calibration pipeline" width="100%">
</p>
<p align="center"><em><strong>Calibration.</strong> Bias / dark / flat correction with median-combined masters and per-channel R/G/B assignment, with crop-to-intersection for mismatched dimensions.</em></p>

<p align="center">
  <img src="docs/screenshots/18-supernova-composite.png" alt="JWST supernova remnant composite" width="100%">
</p>
<p align="center"><em><strong>More results.</strong> The same end-to-end pipeline on a JWST supernova remnant — violet/pink filaments and orange knots over a dense star field — finished with the tone-curve editor.</em></p>

</details>

## Features

### ComposeWizard (10-Step Pipeline)
1. **Channels**: Auto-detect narrowband filters from headers, drag-and-drop assignment, JWST wavelength mapping, custom bins
2. **Stack**: Per-channel sigma-clipped stacking with a subframe selector (star count, FWHM, SNR, and a per-frame quality weight that drives **weighted integration**), auto-align, and optional **drizzle** super-resolution (scale, pixfrac, kernel)
3. **Align**: Phase correlation (sub-pixel) or star-based affine (rotation/scale), automatic fallback
4. **Crop**: Auto-detect valid data intersection across aligned channels, border scan with configurable margins
5. **Background**: Polynomial surface extraction with sigma-clipped grid sampling (subtract/divide)
6. **Blend**: Preset palettes (SHO, HOO, Foraxx, Dynamic HOO, Hubble Legacy, RGB) with spectral wavelength resolver for any bin configuration
7. **Color**: Stability-based auto white balance (MAD/median reference) + SCNR green removal (average/maximum neutral) with BT.709 luminance redistribution. Narrowband warning for SHO/HOO workflows.
8. **Stretch**: Masked stretch with star protection (growth/protection + shared-luminance mask), per-channel STF, or arcsinh
9. **Adjust**: Spline-based tone curves with per-channel R/G/B and linked RGB modes
10. **Export**: PNG (8/16-bit), FITS RGB with WCS/metadata preservation and `PROGRAM`/`HISTORY` provenance cards, ZIP bundle with all channels + composite

### Core Pipeline
- **FITS + ASDF**: Memory-mapped I/O. MEF with auto SCI selection. First non-Python ASDF reader (zlib/bzip2/lz4, Roman Space Telescope gWCS). ZIP transparency. Multi-BITPIX export (16/float32/float64).
- **Non-destructive composite**: ORIG/KEY dual-layer cache. WB and SCNR reconstruct from immutable originals every time. Reset to blend with one click.
- **Dual alignment**: FFT phase correlation (sub-pixel, O(n log n)) as default, star-based affine (triangle asterism + RANSAC, 500 iterations) for rotation. Automatic fallback chain: affine > rigid > phase correlation > identity.
- **STF consistency**: GPU shader, CPU worker, and Rust backend produce pixel-identical output via symmetric denominator protection and unified padding threshold.
- **Calibration**: Bias, dark, flat correction with median-combined masters. Crop-to-intersection for mismatched dimensions.
- **Smart Pipeline**: Auto-detects 2D images vs 3D cubes per file and routes accordingly.

### Enhancement
- **Deconvolution**: Richardson-Lucy (FFT-based, Tikhonov regularization, deringing)
- **Background**: Polynomial surface fitting with sigma-clipped grid sampling
- **Wavelet**: A trous multi-scale denoise with per-scale thresholds (up to 8 scales)
- **Stretch**: Arcsinh stretch, masked stretch with star protection, per-channel STF
- **SPCC**: Spectrophotometric color calibration with optional Gaia DR3 TAP (vizier feature)

### Analysis
- 16K-bin histogram with auto-STF
- FFT power spectrum with Hann window
- Star detection (flux, median FWHM, SNR) with overlay circles
- Empirical PSF estimation with moment-based FWHM (eigenvalue decomposition, subpixel peak interpolation)
- Narrowband filter auto-detection (H-alpha, [OIII], [SII]) with multi-palette support (SHO, HOO, HOS, NaturalColor, Custom)
- Spectroscopy with automatic wavelength unit conversion (M, NM, ANGSTROM, HZ)
- Full header explorer with categorized keyword browser

### Synthetic Data
- Star field generation with configurable count and flux distribution
- PSF modeling (Gaussian, Moffat)
- Noise injection (Poisson, Gaussian, readout)
- Stack generation for testing calibration and alignment pipelines

### Rendering
- WebGPU compute shader for real-time STF preview (8 ms at 4K)
- Binary IPC with zero base64 overhead
- Deep zoom tile pyramid with percentile-based stretch
- Canvas 2D fallback

### Astrometry
- Plate solving via astrometry.net (auto-downsample for large images)
- WCS coordinate readout and pixel/world conversion

## Installation

### Download

| Platform | Download |
|----------|----------|
| **macOS** (Apple Silicon) | [`.dmg`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **macOS** (Intel) | [`.dmg`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **Linux** | [`.deb`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) / [`.AppImage`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) / [`.rpm`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **Windows** | [`.msi`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) / [`.exe`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |

### One-liner

```bash
# macOS
curl -fsSL https://raw.githubusercontent.com/samuelkriegerbonini-dev/AstroBurst/main/scripts/install-macos.sh | bash

# Linux (Debian/Ubuntu)
curl -fsSL https://raw.githubusercontent.com/samuelkriegerbonini-dev/AstroBurst/main/scripts/install-linux.sh | bash
```

### Build from source

```bash
git clone https://github.com/samuelkriegerbonini-dev/AstroBurst.git
cd AstroBurst
cargo tauri dev
```

Requires Rust 1.75+, Node.js 18+, Tauri CLI v2.10. WebGPU needs Vulkan/Metal/DX12.

## Quick Start

1. **Drop files** into the window (`.fits`, `.fit`, `.asdf`, or `.zip`). They're processed automatically.
2. **Explore**: Click a file to see its preview, histogram, and headers. Tweak STF sliders or hit Auto STF.
3. **ComposeWizard**: Open the Compose tab and follow the 11 steps. Auto-Map detects filters, blend presets resolve by wavelength, WB and SCNR are non-destructive.
4. **Processing**: Background extraction, wavelet denoise, deconvolution, and stretch are available as standalone tools in the Processing tab.
5. **Export**: PNG (8/16-bit) or FITS with preserved WCS metadata. ZIP bundle for all channels + composite.

## Getting Data

AstroBurst works with publicly available data from NASA and ESA archives. No account required for most downloads.

### MAST (JWST, Hubble, Roman)

The [MAST Portal](https://mast.stsci.edu/search/ui/#/jwst) is the primary source for JWST and Hubble data. To find calibrated images ready for processing:

1. Go to **[MAST Search](https://mast.stsci.edu/search/ui/#/jwst)**
2. Search by target name (e.g., "M51", "Carina Nebula", "Cassiopeia A")
3. Filter by **Instrument**: NIRCam or MIRI
4. Filter by **Product Level**: Level 2b/2c (calibrated single exposures) or Level 3 (combined mosaics)
5. Download the `_cal.fits` or `_i2d.fits` files

**Quick links to popular targets:**

| Target | Filters | Link |
|--------|---------|------|
| Pillars of Creation | NIRCam F090W/F187N/F200W/F335M/F444W | [MAST](https://mast.stsci.edu/search/ui/#/jwst/results?resolve=true&target=M16) |
| Carina Nebula | NIRCam F090W/F200W/F444W | [MAST](https://mast.stsci.edu/search/ui/#/jwst/results?resolve=true&target=Carina%20Nebula) |
| Cassiopeia A | NIRCam F162M/F356W/F444W | [MAST](https://mast.stsci.edu/search/ui/#/jwst/results?resolve=true&target=Cassiopeia%20A) |
| M51 Whirlpool | NIRCam F115W/F200W/F335M/F444W | [MAST](https://mast.stsci.edu/search/ui/#/jwst/results?resolve=true&target=M51) |
| Jupiter | NIRCam F164N/F212N/F360M | [MAST](https://mast.stsci.edu/search/ui/#/jwst/results?resolve=true&target=Jupiter) |

For Roman Space Telescope simulated data (`.asdf` files), filter by instrument **WFI** on the MAST portal.

### ESA Hubble Archive

The [ESA Hubble Science Archive](https://hst.esac.esa.int/ehst/) provides an alternative interface for HST data. Useful for narrowband imaging (H-alpha, [OIII], [SII]) that works well with the automatic SHO palette detection.

### Sample Data

The repository includes three HST/WFPC2 narrowband FITS files in `tests/sample-data/` for quick testing: [OIII] 502nm, H-alpha 656nm, and [SII] 673nm of the Eagle Nebula (M16). Public domain (NASA/ESA).

## Usage

### JWST NIRCam

NIRCam has two detector resolutions: short-wave (~0.031"/px, ~14K) and long-wave (~0.063"/px, ~7K). When you compose RGB with mixed SW+LW filters, AstroBurst detects the mismatch and auto-resamples the smaller channels via bicubic interpolation. WCS headers are updated so astrometry stays valid.

**Recommended combinations:**

| Type | R | G | B |
|------|---|---|---|
| Full range (auto-resampled) | F444W | F200W | F115W |
| LW only (same resolution) | F444W | F335M | F300M |
| SW only (highest res) | F200W | F150W | F115W |

### Hubble Narrowband

HST narrowband filters are auto-detected from FITS headers. Six blend presets are available: SHO (Hubble Palette), HOO, Dynamic HOO, Foraxx, Hubble Legacy, and RGB. The spectral wavelength resolver maps presets to any bin configuration by sorting both preset weights and filled bins by wavelength. SHO maps [SII] 673nm to R, H-alpha 656nm to G, [OIII] 502nm to B. Custom bins with wavelength metadata (e.g., JWST filters) are resolved automatically.

### Roman Space Telescope (ASDF)

AstroBurst is the first non-Python tool with native ASDF support. Roman `.asdf` files are loaded transparently through the same pipeline as FITS, including zlib/bzip2/lz4 decompression and gWCS extraction.

### 3D Cubes

For IFU data (NAXIS3 > 1), click anywhere on the preview to extract the spectrum at that pixel. Wavelength calibration is read from WCS headers when available, with automatic unit conversion (meters, nanometers, Angstroms, Hz, etc.).

## Architecture

```
Frontend (React 19 + TypeScript 5.7 + Tailwind v4)
+-- 12 domain services (compose, fits, analysis, header, synth, ...)
+-- 8 shared UI primitives (Slider, Toggle, RunButton, ...)
+-- 8 split PreviewContexts for minimal re-renders
+-- 10-step ComposeWizard with useReducer state machine
+-- infrastructure/tauri/ IPC layer (safeInvoke, withPreview, getOutputDir)
+-- Lazy-loaded panels via React.lazy + Suspense
         |
         | Tauri Commands (47)
         v
Backend (Rust + Tauri v2.10)
+-- cmd/     47 command handlers across 12 modules
+-- core/    alignment (phase correlation + affine), analysis,
|            astrometry (WCS, plate solve, SPCC), compose (RGB, blend, SCNR),
|            cube, imaging (STF, deconv, wavelet, background, masked stretch,
|            star mask, PSF), metadata, stacking, synth
+-- domain/  orchestration (calibration, drizzle, cube, plate solve)
+-- infra/   FITS mmap, ASDF reader, ORIG/KEY dual cache, config, render, progress
+-- types/   shared types + 310 constants
+-- math/    NaN-safe median/MAD, sigma clipping, SIMD (Cephes-precision log)
```

**Design principles:**
- All pixel data stays in f32/f64. No integer quantization at any stage.
- Non-destructive composite: immutable ORIG cache, all WB/SCNR reconstruct from originals.
- Dual alignment: FFT phase correlation (sub-pixel) as default, star-based affine (triangle asterism + RANSAC) for rotation. Automatic fallback chain.
- White balance picks the channel with lowest noise (MAD/median) as reference, not always G.
- SCNR redistributes lost luminance to R/B via BT.709 weights instead of adding it back to G.
- STF rendering is pixel-identical across GPU (WGSL), CPU worker (JS), and Rust backend.
- Blend presets resolve by spectral wavelength, not positional index.
- Drizzle uses flat contiguous storage instead of per-pixel heap allocations.
- Sigma clipping uses median/MAD (Stetson 1987) on first iteration, mean/stddev on subsequent.
- NaN values sort to end everywhere: median, MAD, sigma clipping, auto-STF.
- Binary IPC for GPU pixels: 16-byte header + raw f32 array, zero JSON/base64.
- SIMD log uses Cephes degree-8 minimax polynomial with range reduction (< 1 ULP for f32).
- All JSON response keys use shared constants from `types/constants.rs`. Zero hardcoded string keys.
- Output paths resolve via `appDataDir()` on all platforms (no `./output` relative paths).

## Roadmap

| Version | What | Status |
|:--------|:-----|:-------|
| **v0.4** | ComposeWizard, non-destructive pipeline, tone curves, masked stretch, SPCC, affine alignment, stability WB, SCNR redistribution, blend presets, STF consistency, cache-only processing, numerical audit | Released |
| **v0.5** | Drizzle in the wizard, quality-weighted stacking, valid-FITS export with WCS/metadata preservation + `PROGRAM`/`HISTORY` provenance, shared-luminance star mask, alignment/stacking correctness fixes (phase correlation, flux-conserving drizzle, sub-pixel registration), malformed-file parsing hardening | **Released** |
| **v0.6** | FITS ERR/DQ/VAR propagation, MAST API, levels, selective saturation, star removal | Next |
| **v0.7** | Gaia DR3 photometric calibration, PixelMath, mosaic stitching, gradient-aware background | Planned |
| **v1.0** | Full WebGPU pipeline, WASM plugins, Python scripting | Planned |

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

Some areas where help would be especially valuable:
- FITS ERR/DQ/VAR error propagation through the processing chain
- MAST API integration for direct JWST/HST data access
- Levels and selective saturation controls
- Star removal algorithms
- Gaia DR3 cross-match for photometric calibration
- Mosaic stitching with WCS-aware reprojection
- WebGPU compute shaders for stacking and alignment
- Test data curation from public archives (MAST, ESA)

## Support

AstroBurst is free, open-source, with no subscriptions and no feature locks.

If it helps your work, consider supporting development:

<p align="center">
  <a href="https://ko-fi.com/astroburst">
    <img src="https://img.shields.io/badge/Ko--fi-Support%20AstroBurst-FF5E5B?style=for-the-badge&logo=ko-fi" alt="Support on Ko-fi">
  </a>
</p>

See [membership tiers](https://ko-fi.com/astroburst/tiers) for details.

## Supporters

<!-- SUPPORTERS:START -->
*Be the first to support!*
<!-- SUPPORTERS:END -->

## License

GPLv3. See [LICENSE](LICENSE).

---

<p align="center">
  <sub>Created by <a href="https://github.com/samuelkriegerbonini-dev">Samuel Krieger</a></sub>
</p>
