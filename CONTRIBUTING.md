# Contributing to AstroBurst

Thank you for your interest in contributing to AstroBurst! This guide will help you get started.

## Development Setup

### Prerequisites

- **Rust** 1.75+ -- [Install via rustup](https://rustup.rs/)
- **Node.js** 18+ -- [Download](https://nodejs.org/)
- **pnpm** -- `npm install -g pnpm`
- **Tauri CLI** -- `cargo install tauri-cli`

### Platform-Specific Dependencies

**Linux (Debian/Ubuntu):**
```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf \
  libssl-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev
```

**macOS:**
```bash
xcode-select --install
```

### Building

```bash
git clone https://github.com/samuelkriegerbonini-dev/AstroBurst.git
cd AstroBurst
pnpm install
pnpm tauri dev
```

## How to Contribute

### Reporting Bugs

- Use the [Bug Report](https://github.com/samuelkriegerbonini-dev/AstroBurst/issues/new?template=bug_report.md) template
- Include your OS, hardware info, and steps to reproduce
- Attach the problematic FITS/ASDF file if possible (or describe its format, NAXIS, BITPIX, extension count)

### Suggesting Features

- Use the [Feature Request](https://github.com/samuelkriegerbonini-dev/AstroBurst/issues/new?template=feature_request.md) template
- Describe the use case from an astronomer/astrophotographer perspective

### Submitting Code

1. **Sign the CLA** -- On your first pull request, the CLA Assistant bot will ask you to review and accept our [Contributor License Agreement](CLA.md). This is a one-time step.
2. Fork the repository
3. Create a feature branch: `git checkout -b feature/my-feature`
4. Make your changes
5. Run tests: `cd src-tauri && cargo test --all-features`
6. Run formatting: `cargo fmt`
7. Run linter: `cargo clippy --all-features -- -D warnings`
8. Commit with a descriptive message
9. Push and open a Pull Request

### Code Style

**Rust:**
- Follow `rustfmt` defaults
- No comments in generated code
- No unsafe code without justification and a `// SAFETY:` comment
- Use `anyhow::Result` for command error handling
- Keep Tauri commands thin -- `cmd/` parses input, delegates to `domain/` or `core/`, and formats output
- Pure algorithms go in `core/` (no Tauri dependencies), orchestration in `domain/`, I/O in `infra/`
- No em dashes in text or documentation -- use `--` instead

**TypeScript/React:**
- Follow ESLint configuration
- Use functional components with hooks
- Tailwind for styling, no CSS modules
- Use `useBackend()` hook for all Tauri command calls
- Lazy-load heavy panels with `React.lazy()` + `Suspense`

### Testing with Sample Data

The `tests/sample-data/` directory contains HST narrowband FITS files for testing:

| File | Filter | Wavelength | Description |
|------|--------|------------|-------------|
| `502nmos.fits` | [OIII] | 502nm | Oxygen III emission |
| `656nmos.fits` | H-alpha | 656nm | Hydrogen alpha emission |
| `673nmos.fits` | [SII] | 673nm | Sulfur II emission |

These are 1600x1600 float32 images from the Hubble Space Telescope (WFPC2), ideal for testing Hubble palette composition, stacking, and calibration workflows.

For ASDF format testing, use publicly available Roman Space Telescope simulated data from STScI (`.asdf` files). AstroBurst auto-dispatches `.asdf` files through the same command layer as `.fits`.

For JWST testing, download NIRCam i2d files from MAST (Proposal 2739 is a good starting point with multiple filters on the Pillars of Creation).

## Architecture Overview

```
src-tauri/src/
+-- cmd/         # Tauri IPC command handlers (42 commands, thin layer)
+-- core/        # Pure algorithms, no Tauri deps (bounded contexts)
|   +-- analysis/    # star detection, FFT, deconvolution
|   +-- astrometry/  # plate solve, WCS transforms
|   +-- compose/     # RGB compose, drizzle RGB
|   +-- cube/        # eager + lazy cube processing
|   +-- imaging/     # STF, normalize, background, wavelet, resample, stats
|   +-- metadata/    # header discovery, filter detection
|   +-- stacking/    # sigma-clip, drizzle, align, calibration
+-- domain/      # Orchestration layer (delegates to core/)
+-- infra/       # I/O, caching, rendering, external formats
|   +-- asdf/        # ASDF parser, blocks, tree, converter
|   +-- fits/        # mmap reader, writer, dispatcher
|   +-- render/      # grayscale, RGB, tile pyramid
|   +-- cache.rs     # LRU image cache (Arc zero-copy)
|   +-- ipc.rs       # binary IPC for GPU pixel transfer
|   +-- progress.rs  # progress events via Tauri emit
+-- math/        # SIMD, median, sigma clipping
+-- types/       # shared data structures, constants, error types
+-- shaders/     # WebGPU compute shaders (WGSL)

src/             # Frontend (React + TypeScript + Tailwind)
+-- components/
|   +-- preview/     # tab content (Processing, Compose, Stacking, etc.)
+-- context/         # 6 split PreviewContexts for granular re-renders
+-- hooks/           # useBackend, useFileQueue, useTimer, useProgress
+-- utils/           # types, validation, constants, STF worker
```

**Key principles:**
- `cmd/` should only parse input, delegate to `domain/` or `core/`, and format output
- `core/` contains pure algorithms with no Tauri or I/O dependencies
- `domain/` is a thin orchestration layer that wires `core/` with `infra/`
- `infra/` handles all I/O: FITS/ASDF parsing, rendering, caching, progress events
- All image data stays in f32/f64 -- no integer quantization at any stage

## Priority Areas for Contributions

These are the areas where contributions would have the most impact right now:

**High priority:**
- Multi-extension FITS ERR/DQ/VAR error propagation through the processing pipeline
- MAST API integration for direct JWST/HST data download
- Star removal algorithms
- ASDF format testing with real Roman Space Telescope simulated data

**Medium priority:**
- Photometric calibration via Gaia DR3 cross-match
- WebGPU compute shader pipeline expansion (more operations on GPU)
- Additional deconvolution algorithms (Wiener, MEM)
- Mosaic stitching for multi-pointing observations

**Always welcome:**
- Test data curation (public FITS from MAST, ESA archives)
- Documentation, tutorials, and processing guides
- Bug reports with reproducible steps and sample data
- Platform-specific packaging and testing (especially Linux ARM64)

## Contributor License Agreement

All contributions require signing our [Contributor License Agreement (CLA)](CLA.md). This grants the maintainer the necessary rights to manage the project's licensing. The CLA Assistant bot will guide you through the process on your first pull request.

## License

AstroBurst is licensed under the [GNU General Public License v3.0](LICENSE). By contributing, you agree to the terms outlined in the CLA.
