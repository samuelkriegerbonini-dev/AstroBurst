<p align="center">
  <img src="src/assets/logo.png" alt="AstroBurst Logo" width="128" />
</p>

<h1 align="center">AstroBurst</h1>

<p align="center">
  <strong>High-Performance Astronomical Image Processor</strong><br>
  <em>Built with Rust · Tauri · WebGPU · SIMD</em>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-0.1.0-blue.svg" alt="Version">
  <img src="https://img.shields.io/badge/rust-1.75+-orange.svg" alt="Rust">
  <img src="https://img.shields.io/badge/tauri-2.0-purple.svg" alt="Tauri">
  <img src="https://img.shields.io/badge/license-MIT-green.svg" alt="License">
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg" alt="Platform">
</p>

---

AstroBurst is a native desktop application for processing astronomical FITS images. It combines a high-performance Rust backend with a modern React frontend, targeting both professional astronomers and advanced astrophotographers.

## Features

### Core Processing

- **FITS I/O** — Read/write FITS files with memory-mapped extraction, ZIP archive transparency, and full header preservation
- **Batch Processing** — Parallel processing of hundreds of frames using Rayon thread pool
- **Asinh Stretch** — Automatic normalization with astronomically-correct arcsinh transfer function
- **STF (Screen Transfer Function)** — Real-time parametric stretch with shadow/midtone/highlight controls
- **FITS Export** — Write processed data back to FITS with WCS and metadata preservation

### Calibration & Stacking

- **Bias/Dark/Flat Calibration** — Full calibration pipeline with automatic master frame generation
- **Sigma-Clipped Stacking** — Iterative sigma rejection with configurable thresholds
- **Drizzle Integration** — Sub-pixel stacking with Square/Gaussian/Lanczos3 kernels, configurable scale and pixfrac
- **Drizzle RGB** — Integrated RGB drizzle pipeline that processes all three channels and composes in a single operation
- **Auto-Alignment** — Cross-correlation based frame registration

### Color & Composition

- **RGB Composition** — Combine narrowband or broadband channels with per-channel STF
- **Hubble Palette Auto-Mapping** — Automatic detection of Hα/[OIII]/[SII] filters from FITS headers and filenames
- **White Balance** — Auto, manual, and none modes
- **SCNR** — Subtractive Chromatic Noise Reduction (Average/Maximum Neutral methods)

### Analysis

- **Histogram & Statistics** — 512-bin histogram with median, mean, σ, MAD computation
- **FFT Power Spectrum** — Full 2D Fourier analysis for noise characterization
- **Star Detection** — PSF-fitting centroid detection with flux, FWHM, and SNR metrics
- **Header Explorer** — Categorized FITS header browser with search, filter detection, and copy support

### Astrometry

- **WCS Transform** — Pixel ↔ World coordinate conversion using CD matrix
- **Field of View** — Automatic FOV computation with corner coordinates
- **Plate Solving** — Remote solving via astrometry.net API integration

### Spectroscopy (IFU/Datacube)

- **Cube Processing** — Full and lazy (memory-mapped) datacube extraction
- **Frame Navigation** — Single-frame extraction with global normalization
- **Spectrum Extraction** — Click-to-extract spectrum at any spatial position
- **Wavelength Calibration** — Automatic axis construction from FITS WCS keywords

### Visualization

- **WebGPU Rendering** — GPU-accelerated STF stretch via compute shaders, Canvas 2D fallback
- **Deep Zoom Tiles** — Multi-resolution tile pyramid generation for large images
- **Zero-Copy IPC** — Binary pixel transfer via Tauri IPC Response (no JSON/base64 overhead)

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
│  commands/                                            │
│  ├── image.rs          Processing & I/O              │
│  ├── metadata.rs       Header & Discovery            │
│  ├── analysis.rs       Math & Star Detection         │
│  ├── visualization.rs  STF & Tile Generation         │
│  ├── cube.rs           Spectral Datacubes            │
│  ├── astrometry.rs     WCS & Plate Solving           │
│  ├── stacking.rs       Calibration & Stacking        │
│  └── config.rs         App State                     │
│                                                       │
│  domain/               Core Algorithms                │
│  ├── stf.rs            Screen Transfer Function      │
│  ├── drizzle.rs        Drizzle Stacking              │
│  ├── drizzle_rgb.rs    RGB Drizzle Pipeline          │
│  ├── plate_solve.rs    Star Detection + Astrometry   │
│  ├── rgb_compose.rs    Color Composition             │
│  ├── calibration.rs    Bias/Dark/Flat Pipeline       │
│  ├── fft.rs            FFT Power Spectrum            │
│  ├── wcs.rs            World Coordinate System       │
│  └── ...                                              │
│                                                       │
│  utils/                Infrastructure                 │
│  ├── mmap.rs           Memory-Mapped FITS Extraction │
│  ├── simd.rs           SIMD-Accelerated Statistics   │
│  ├── ipc.rs            Zero-Copy Binary Transfer     │
│  ├── tiles.rs          Deep Zoom Pyramid             │
│  └── render.rs         PNG Rendering                 │
└──────────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

- Rust 1.75+
- Node.js 18+
- pnpm (recommended) or npm

### Build & Run

```bash
git clone https://github.com/your-username/astroburst.git
cd astroburst

# Install frontend dependencies
pnpm install

# Run in development mode
pnpm tauri dev

# Build release binary
pnpm tauri build
```

### Plate Solving (Optional)

AstroBurst integrates with astrometry.net for plate solving. To enable:

1. Get a free API key at https://nova.astrometry.net/api_help
2. In AstroBurst, go to Settings → save your API key
3. The plate solver is available in the Star Detection panel

## Usage

1. **Open files** — Drag & drop FITS files (.fits, .fit, .fts) or browse via file picker. ZIP archives containing FITS are supported transparently.

2. **Processing** — Files are automatically processed: FITS extraction → asinh normalization → PNG preview → histogram analysis.

3. **Inspect** — Click any processed file to see:
   - Image preview with STF controls (shadow/midtone/highlight sliders)
   - Toggle GPU rendering for real-time stretch
   - Full histogram with auto-stretch parameters
   - FFT power spectrum
   - FITS header explorer with categorized view

4. **Analyze** — Run star detection, extract spectra from datacubes, or solve plate astrometry.

5. **Stack** — Select multiple frames for sigma-clipped stacking or drizzle integration with sub-pixel resolution.

6. **Drizzle RGB** — Assign R/G/B channel groups, configure drizzle parameters, and process all channels with composition in a single operation.

7. **Compose** — Assign channels for RGB composition with auto white balance and SCNR green removal.

8. **Export** — Save as PNG, FITS (with original WCS/metadata), or batch ZIP.

## Command Reference

AstroBurst exposes 35 Tauri commands organized into 8 modules:

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

## Tech Stack

| Component | Technology |
|-----------|------------|
| Backend | Rust, Tauri v2, ndarray, Rayon, memmap2, rustfft |
| Frontend | React 18, TypeScript, Framer Motion, Tailwind CSS |
| GPU | WebGPU compute shaders (WGSL) with Canvas 2D fallback |
| IPC | Tauri JSON commands + binary ipc::Response for pixel data |
| Build | Vite, Cargo |

## Performance

| Operation | Time |
|-----------|------|
| Open 2GB IFU cube | 0.3s |
| Stack 20 frames sigma-clip | 2.9s |
| Drizzle 2× (10 frames) | 4.2s |
| Drizzle RGB 2× (3×10 frames) | 6.8s |
| RGB compose + align | 1.8s |

Binary size: ~15MB

## Known Limitations (v0.1.0)

- Single-HDU FITS only (multi-extension support planned)
- Grayscale processing pipeline (color FITS via RGB composition)
- Plate solving requires internet connection (local solver planned)
- No undo/redo system yet
- WebGPU requires compatible browser engine (Chromium 113+)

## Contributing

Contributions welcome. Please open an issue first for major changes.

```bash
# Run tests
cargo test

# Run with logging
RUST_LOG=debug pnpm tauri dev
```

## License

MIT — see [LICENSE](LICENSE) for details.

---

<p align="center">
  <em>Built by astronomers, for astronomers.</em>
</p>
