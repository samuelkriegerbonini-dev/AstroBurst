# Contributing to AstroBurst

Thank you for your interest in contributing to AstroBurst! This guide will help you get started.

## Development Setup

### Prerequisites

- **Rust** 1.75+ — [Install via rustup](https://rustup.rs/)
- **Node.js** 18+ — [Download](https://nodejs.org/)
- **pnpm** — `npm install -g pnpm`
- **Tauri CLI** — `cargo install tauri-cli`

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
- Attach the problematic FITS file if possible (or describe its format)

### Suggesting Features

- Use the [Feature Request](https://github.com/samuelkriegerbonini-dev/AstroBurst/issues/new?template=feature_request.md) template
- Describe the use case from an astronomer/astrophotographer perspective

### Submitting Code

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Make your changes
4. Run tests: `cd src-tauri && cargo test --all-features`
5. Run formatting: `cargo fmt`
6. Run linter: `cargo clippy --all-features -- -D warnings`
7. Commit with a descriptive message
8. Push and open a Pull Request

### Code Style

**Rust:**
- Follow `rustfmt` defaults
- No unsafe code without justification and a `// SAFETY:` comment
- Use `anyhow::Result` for command error handling
- Keep Tauri commands thin — business logic goes in `domain/`

**TypeScript/React:**
- Follow ESLint configuration
- Use functional components with hooks
- Tailwind for styling, no CSS modules

### Testing with Sample Data

The `tests/sample-data/` directory contains HST narrowband FITS files for testing:

| File | Filter | Wavelength | Description |
|------|--------|------------|-------------|
| `502nmos.fits` | [OIII] | 502nm | Oxygen III emission |
| `656nmos.fits` | Hα | 656nm | Hydrogen alpha emission |
| `673nmos.fits` | [SII] | 673nm | Sulfur II emission |

These are 1600×1600 float32 images from the Hubble Space Telescope (WFPC2), ideal for testing Hubble palette composition, stacking, and calibration workflows.

## Architecture Overview

```
src-tauri/src/
├── commands/    # Tauri IPC command handlers (thin layer)
├── domain/      # Core algorithms and business logic
├── utils/       # Infrastructure (SIMD, IPC, memory mapping)
├── model/       # Data structures
└── shaders/     # WebGPU compute shaders (WGSL)
```

**Key principle:** Commands in `commands/` should only parse input, delegate to `domain/`, and format output. All processing logic lives in `domain/`.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
