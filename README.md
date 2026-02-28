<p align="center">
  <img src="src/assets/logo.png" alt="AstroBurst Logo" width="128" />
</p>

<h1 align="center">AstroBurst</h1>

<p align="center">
  <strong>High-Performance Astronomical Image Processor</strong><br>
  <em>The first FITS processor built on the Rust · Tauri · WebGPU stack</em>
</p>

<p align="center">
  <a href="https://github.com/samuelkriegerbonini-dev/AstroBurst/releases"><img src="https://img.shields.io/github/v/release/samuelkriegerbonini-dev/AstroBurst?style=flat-square&color=blue" alt="Release"></a>
  <a href="https://github.com/samuelkriegerbonini-dev/AstroBurst/actions"><img src="https://img.shields.io/github/actions/workflow/status/samuelkriegerbonini-dev/AstroBurst/build.yml?style=flat-square" alt="Build"></a>
  <img src="https://img.shields.io/badge/rust-1.75+-orange.svg?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/tauri-2.0-purple.svg?style=flat-square" alt="Tauri">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/samuelkriegerbonini-dev/AstroBurst?style=flat-square&color=green" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg?style=flat-square" alt="Platform">
</p>

<p align="center">
  <a href="#installation">Install</a> ·
  <a href="#features">Features</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="#usage">Usage</a> ·
  <a href="#contributing">Contributing</a>
</p>

---

AstroBurst is a native desktop application for processing astronomical FITS images. It combines a high-performance Rust backend with a modern React frontend, delivering near-native performance with a fraction of the memory footprint of legacy tools — targeting both professional astronomers and advanced astrophotographers.

<!-- 
## Screenshots

> TODO: Add screenshots of AstroBurst in action
> - Main interface with FITS preview
> - Hubble palette composition
> - Star detection overlay
> - Spectroscopy datacube viewer
-->

## Installation

### Download (Recommended)

Download the latest release for your platform:

| Platform | Architecture | Download |
|----------|-------------|----------|
| **macOS** | Apple Silicon (M1+) | [`.dmg` (aarch64)](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **macOS** | Intel | [`.dmg` (x86_64)](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **Linux** | x86_64 | [`.deb`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) · [`.rpm`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) · [`.AppImage`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **Linux** | ARM64 | [`.deb`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) · [`.AppImage`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **Windows** | x86_64 | [`.msi`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) · [`.exe`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |

### One-Line Install

**macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/samuelkriegerbonini-dev/AstroBurst/main/scripts/install-macos.sh | bash
```

**Linux:**
```bash
curl -fsSL https://raw.githubusercontent.com/samuelkriegerbonini-dev/AstroBurst/main/scripts/install-linux.sh | bash
```

### Build from Source

```bash
git clone https://github.com/samuelkriegerbonini-dev/AstroBurst.git
cd AstroBurst
pnpm install
pnpm tauri build
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed build instructions and platform-specific dependencies.

## Features

### Core Processing
- **FITS I/O** — Memory-mapped extraction, ZIP transparency, full header preservation
- **Batch Processing** — Parallel processing of hundreds of frames via Rayon
- **Asinh Stretch** — Astronomically-correct arcsinh transfer function
- **STF** — Real-time parametric stretch with shadow/midtone/highlight controls
- **FITS Export** — Write processed data back with WCS and metadata preservation

### Calibration & Stacking
- **Bias/Dark/Flat Calibration** — Full pipeline with automatic master frame generation
- **Sigma-Clipped Stacking** — Iterative sigma rejection with configurable thresholds
- **Drizzle Integration** — Sub-pixel stacking with Square/Gaussian/Lanczos3 kernels
- **Drizzle RGB** — Integrated RGB pipeline that processes all channels in a single operation
- **Auto-Alignment** — Cross-correlation based frame registration

### Color & Composition
- **RGB Composition** — Combine narrowband or broadband channels with per-channel STF
- **Hubble Palette** — Automatic Hα/[OIII]/[SII] detection from FITS headers and filenames
- **White Balance** — Auto, manual, and none modes
- **SCNR** — Subtractive Chromatic Noise Reduction

### Analysis
- **Histogram & Statistics** — 512-bin histogram with median, mean, σ, MAD
- **FFT Power Spectrum** — Full 2D Fourier analysis for noise characterization
- **Star Detection** — PSF-fitting centroid detection with flux, FWHM, and SNR
- **Header Explorer** — Categorized FITS header browser with search and filter detection

### Astrometry
- **WCS Transform** — Pixel ↔ World coordinate conversion (CD matrix)
- **Field of View** — Automatic FOV computation with corner coordinates
- **Plate Solving** — Remote solving via astrometry.net API

### Spectroscopy (IFU/Datacube)
- **Cube Processing** — Full and lazy (memory-mapped) datacube extraction
- **Frame Navigation** — Single-frame extraction with global normalization
- **Spectrum Extraction** — Click-to-extract spectrum at any spatial position
- **Wavelength Calibration** — Automatic axis construction from FITS WCS keywords

### Visualization
- **WebGPU Rendering** — GPU-accelerated STF stretch via compute shaders
- **Deep Zoom Tiles** — Multi-resolution tile pyramid for large images
- **Zero-Copy IPC** — Binary pixel transfer via Tauri IPC Response (no JSON/base64)

## Quick Start

### Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust | 1.75+ | [rustup.rs](https://rustup.rs/) |
| Node.js | 18+ | [nodejs.org](https://nodejs.org/) |
| pnpm | latest | `npm install -g pnpm` |

### Development

```bash
git clone https://github.com/samuelkriegerbonini-dev/AstroBurst.git
cd AstroBurst

pnpm install        # Install frontend dependencies
pnpm tauri dev      # Run in development mode
```

Or use the Makefile:

```bash
make setup    # Install dependencies
make dev      # Start dev server
make test     # Run Rust tests
make check    # Format + lint + clippy
make build    # Build release binary
```

### Plate Solving (Optional)

1. Get a free API key at [astrometry.net](https://nova.astrometry.net/api_help)
2. In AstroBurst → Settings → save your API key
3. Available in the Star Detection panel

## Usage

1. **Open** — Drag & drop FITS files (.fits, .fit, .fts) or browse via file picker. ZIP archives are supported.
2. **Process** — Files auto-process: FITS extraction → asinh normalization → PNG preview → histogram.
3. **Inspect** — Click any file for image preview, STF controls, GPU rendering toggle, histogram, FFT spectrum, and FITS header explorer.
4. **Analyze** — Star detection, datacube spectrum extraction, or plate solving.
5. **Stack** — Sigma-clipped stacking or drizzle with sub-pixel resolution.
6. **Compose** — RGB composition with Hubble palette auto-mapping and SCNR.
7. **Export** — Save as PNG, FITS (with WCS/metadata), or batch ZIP.

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                    Frontend (React)                   │
│  TypeScript · Framer Motion · WebGPU · Canvas 2D     │
├──────────────────────────────────────────────────────┤
│                   Tauri IPC Bridge                    │
│              JSON Commands + Binary IPC               │
├──────────────────────────────────────────────────────┤
│                   Backend (Rust)                      │
│                                                       │
│  commands/       Tauri command handlers (thin layer) │
│  domain/         Core algorithms & business logic    │
│  utils/          SIMD, IPC, mmap, tiles, render      │
│  shaders/        WebGPU compute shaders (WGSL)       │
└──────────────────────────────────────────────────────┘
```

<details>
<summary><strong>Command Reference (35 commands, 8 modules)</strong></summary>

| Module | Commands | Description |
|--------|----------|-------------|
| image | `process_fits`, `process_batch`, `get_raw_pixels`, `get_raw_pixels_binary`, `export_fits`, `export_fits_rgb` | Image I/O and manipulation |
| metadata | `get_header`, `get_full_header`, `detect_narrowband_filters` | FITS header and filter discovery |
| analysis | `compute_histogram`, `compute_fft_spectrum`, `detect_stars` | Statistical analysis and star detection |
| visualization | `apply_stf_render`, `generate_tiles`, `get_tile` | STF stretch and deep zoom |
| cube | `process_cube_cmd`, `process_cube_lazy_cmd`, `get_cube_info`, `get_cube_frame`, `get_cube_spectrum` | IFU/datacube processing |
| astrometry | `plate_solve_cmd`, `get_wcs_info`, `pixel_to_world`, `world_to_pixel` | WCS transforms and plate solving |
| stacking | `calibrate`, `stack`, `drizzle_stack_cmd`, `drizzle_rgb_cmd`, `compose_rgb_cmd`, `run_pipeline_cmd` | Calibration, stacking, and composition |
| config | `get_config`, `update_config`, `save_api_key`, `get_api_key` | Application state management |

</details>

## Performance

Benchmarked on Apple M2 Pro / 16GB:

| Operation | Time |
|-----------|------|
| Open 2GB IFU cube | 0.3s |
| Stack 20 frames (sigma-clip) | 2.9s |
| Drizzle 2× (10 frames) | 4.2s |
| Drizzle RGB 2× (3×10 frames) | 6.8s |
| RGB compose + align | 1.8s |

Binary size: ~15MB

## Sample Data

The [`tests/sample-data/`](exampleFits/sample-data/) directory includes HST/WFPC2 narrowband FITS images for testing:

| File | Filter | λ | Use |
|------|--------|---|-----|
| `502nmos.fits` | [OIII] | 502nm | Hubble palette blue channel |
| `656nmos.fits` | Hα | 656nm | Hubble palette green channel |
| `673nmos.fits` | [SII] | 673nm | Hubble palette red channel |

1600×1600 float32 images from the Eagle Nebula region. Public domain (NASA/ESA). Tracked via [Git LFS](https://git-lfs.github.com/).

```bash
git lfs install
git lfs pull
```

## Tech Stack

| Layer | Technology |
|-------|------------|
| Backend | Rust, Tauri v2, ndarray, Rayon, memmap2, rustfft |
| Frontend | React 19, TypeScript, Framer Motion, Tailwind CSS v4 |
| GPU | WebGPU compute shaders (WGSL) + Canvas 2D fallback |
| IPC | Tauri JSON commands + binary `ipc::Response` |
| Build | Vite, Cargo |
| CI/CD | GitHub Actions (macOS, Linux, Windows) |

## Known Limitations (v0.1.0)

- Single-HDU FITS only (multi-extension support planned)
- Grayscale processing pipeline (color FITS via RGB composition)
- Plate solving requires internet connection (local solver planned)
- No undo/redo system yet
- WebGPU requires Chromium 113+ engine

## Roadmap

- [ ] Multi-extension FITS (MEF) support
- [ ] Local plate solving (offline astrometry)
- [ ] Undo/redo system
- [ ] Plugin architecture
- [ ] Mosaic composition
- [ ] Noise reduction (wavelet / multiscale)
- [ ] Live stacking mode
- [ ] INDI/ASCOM telescope integration

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting a PR.

```bash
make test     # Run tests
make check    # Format + lint + clippy
make dev      # Development with hot reload
```

See also: [Code of Conduct](CODE_OF_CONDUCT.md) · [Security Policy](SECURITY.md)

## License

[MIT](LICENSE) © 2026 Samuel Krieger Bonini

---

<p align="center">
  <em>Built by astronomers, for astronomers.</em>
</p>
