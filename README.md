<p align="center">
  <img src="src/assets/logo.png" alt="AstroBurst Logo" width="128" />
</p>

<h1 align="center">AstroBurst</h1>

<p align="center">
  <strong>A desktop viewer and processing app for FITS and ASDF images, with an optional headless REST API.</strong>
</p>

<p align="center">
  <a href="https://github.com/samuelkriegerbonini-dev/AstroBurst/releases"><img src="https://img.shields.io/github/v/release/samuelkriegerbonini-dev/AstroBurst?style=flat-square&color=blue" alt="Release"></a>
  <a href="https://github.com/samuelkriegerbonini-dev/AstroBurst/actions"><img src="https://img.shields.io/github/actions/workflow/status/samuelkriegerbonini-dev/AstroBurst/ci.yml?style=flat-square" alt="CI"></a>
  <img src="https://img.shields.io/badge/tauri-2.10-purple.svg?style=flat-square" alt="Tauri">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/samuelkriegerbonini-dev/AstroBurst?style=flat-square&color=green" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg?style=flat-square" alt="Platform">
  <a href="https://ko-fi.com/astroburst"><img src="https://img.shields.io/badge/Ko--fi-Support-FF5E5B?style=flat-square&logo=ko-fi" alt="Ko-fi"></a>
</p>

<p align="center">
  <a href="#what-it-is">What it is</a> &middot;
  <a href="#feature-tour">Features</a> &middot;
  <a href="#install-and-build">Install</a> &middot;
  <a href="#quick-start">Quick start</a> &middot;
  <a href="#headless-server-and-python-client">Server</a> &middot;
  <a href="#supported-formats">Formats</a> &middot;
  <a href="#how-it-compares">Compare</a> &middot;
  <a href="#project-status">Status</a>
</p>

---

<p align="center">
  <img src="docs/screenshots/hero.png" alt="AstroBurst processing the Pillars of Creation: file panel, live preview, histogram and analysis tools" width="100%">
</p>

**Latest:** v0.5.8, plus unreleased work on the viewer: display controls shared with the headless server, any-HDU / any-array image references, DQ decoding and overlays, and interactive regions with DS9 `.reg` import and export. Full history in [CHANGELOG.md](CHANGELOG.md).

## What it is

AstroBurst opens FITS and ASDF images, displays them, measures them, and composes and exports them. The Rust backend does the processing, React draws the interface, Tauri packages it as a desktop app. A WebGPU shader applies the display stretch and colormap; that is the only place the GPU is used. Everything else, including stacking, alignment, drizzle, deconvolution and compose, runs on the CPU in Rust.

Three reader paths:

- **Quick-look.** Open any FITS HDU or ASDF array, pick a stretch, limits and colormap, read pixel values with their units and WCS position, draw regions and measure them. Start at [Viewer, regions and pixel readout](#viewer-regions-and-pixel-readout).
- **JWST and Roman.** Native ASDF, per-HDU selection, DQ flag decoding and overlays, DQ-masked statistics and photometry, ERR companion readout. Start at [Data quality and units](#data-quality-and-units).
- **Astrophotography.** A ten-step compose wizard that takes narrowband or broadband channels through stacking, alignment, background, blend, colour, stretch and export. Start at [Compose wizard](#compose-wizard).

Image processing runs locally. Two optional features reach the network: plate solving uploads the frame to nova.astrometry.net, and spectrophotometric colour calibration queries Gaia DR3 through VizieR.

## Feature tour

### Viewer, regions and pixel readout

<p align="center">
  <img src="docs/screenshots/27-regions-and-pixel-readout.png" alt="A box region on a JWST NIRCam mosaic with the display controls bar and the pixel readout panel" width="100%">
</p>
<p align="center"><em>A box region selected over a JWST NIRCam F187N mosaic. Top: the display-controls bar, here on MTF stretch, min/max limits, gray colormap inverted. Right: the readout reporting the pixel as 45.4738 &plusmn; 0.7261 MJy/sr with its ICRS sexagesimal position.</em></p>

**Display controls.** Six stretch curves: MTF, linear, log, sqrt, asinh with an adjustable softening parameter, and power with an adjustable exponent. Four limit algorithms: min/max, zscale (IRAF-style, adjustable contrast), percentile (1 to 99.5 by default) and explicit user vmin/vmax. Nine colormaps (gray, viridis, inferno, magma, plasma, cividis, heat, cool, rainbow) with an invert toggle. The desktop app applies them through a LUT texture in a WebGPU shader and falls back to a CPU worker when WebGPU is unavailable or the device is lost; the headless server computes the same stretch and limits on the CPU and returns a PNG.

**Regions.** Draw circles, ellipses, boxes (both rotatable), annuli, polygons, lines and points, in pixel or sky coordinates. Measure a source by putting a circle on it and a background annulus around it: the annulus is fitted and subtracted from the region statistics, with optional sigma clipping and NaN-safe throughout. Check focus with a radial profile from a circle or annulus. Cut across a filament, a diffraction spike or a detector artefact with a line region and read the profile along it. Regions import and export as DS9 `.reg` files (format 4.1), keeping colour, width, label, dash and include/exclude, so shapes move between AstroBurst and DS9. Coordinate systems and shapes the reader does not handle (fk4, galactic, ecliptic, b1950, amplifier, detector, tile; text, vector, ruler, compass, projection, the panda family, composite) are reported as warnings instead of being dropped silently.

Region statistics, cutouts, histograms and rendering are available on the headless server too. Radial profiles, line cuts and `.reg` import/export are desktop-only.

**Readout.** Hovering reports the pixel value with its BUNIT unit (or the ASDF `unit` field), the matching value from an ERR companion array as `value ± err`, neighbourhood statistics over a box (min, max, mean, median, NaN count), and the decoded DQ flags. Cursor coordinates are shown in ICRS, FK5 (J2000), galactic or ecliptic (J2000), sexagesimal or decimal, with right ascension in hours for the equatorial frames and degrees for the others.

### Data quality and units

**Any HDU, any array.** An image reference of the form `path#hdu=3` or `path#array=roman.dq` makes any FITS extension or any ASDF array the displayed image, not only the auto-detected SCI plane. Every path-based command accepts the reference, so DQ and ERR planes can be displayed and measured like any other image. Pick one from the HDU panel in the desktop app, or over HTTP on the server.

**DQ flags.** Data-quality extensions are decoded against named bit tables, selected from the header: JWST (32 flags), HST, and the JWST convention for Roman. Files from other instruments fall back to the JWST bit names, labelled as such. Hovering a pixel names the flags set on it instead of showing a raw integer. Flagged pixels can be painted over the image as a mask overlay, and statistics, histograms, photometry, region statistics and profiles can all exclude them with one toggle. Integer extensions are read through a lossless integer plane, so individual flag bits survive exactly; the display and processing paths stay in f32/f64.

The DQ overlay and DQ exclusion are desktop-only. The server pixel endpoint returns decoded DQ, but its stats and histogram endpoints do not yet accept the exclusion flag.

### Colour calibration

<p align="center">
  <img src="docs/screenshots/28-spcc-gpu-composite.png" alt="Spectrophotometric colour calibration panel over an RGB composite" width="100%">
</p>
<p align="center"><em>Spectrophotometric colour calibration against Gaia DR3 with a selectable white reference and SCNR, on an RGB composite rendering through the GPU display path.</em></p>

Spectrophotometric colour calibration solves channel gains against a real Gaia DR3 cone search through VizieR (the `vizier` feature, on by default), falling back to a synthetic catalogue when the feature is off. Auto white balance picks the channel with the lowest noise (MAD over median) as the reference rather than always green. SCNR removes green excess and redistributes the lost luminance to red and blue with BT.709 weights.

### Compose wizard

<p align="center">
  <img src="docs/screenshots/10-export-final.png" alt="The finished SHO composite of the Pillars of Creation on the export step" width="100%">
</p>
<p align="center"><em>The export step with a finished SHO narrowband composite. White balance and SCNR are baked in from the calibrated linear composite and the stretch is applied at write time.</em></p>

Ten steps: channels, stack, align, crop, background, blend, colour, stretch, adjust, export. Filters are detected from headers and mapped to channel bins by wavelength. Stacking is sigma-clipped, with a subframe selector and optional drizzle. Alignment is sub-pixel phase correlation by default or star-based affine (triangle asterism with RANSAC, 2000 iterations) for rotation, with an automatic fallback chain of affine, rigid, phase correlation, identity. Background correction has four families: per-channel polynomial surface, linked shared gradient, neutralize (sky pedestal only) and de-band for 1/f striping (rows, columns, both or an auto-detected axis), offered as seven options in the selector. Blend presets (SHO, HOO, Dynamic HOO, Foraxx, Hubble Legacy, RGB, plus wavelength-spreading Auto and Balanced) resolve by spectral wavelength rather than bin order. Stretch offers star removal (starless image plus a separate stars layer), masked stretch with star protection, GHS, arcsinh and linked or per-channel STF. Adjust applies monotone Fritsch-Carlson spline tone curves. The composite is non-destructive: white balance and SCNR always reconstruct from an immutable original cache.

### Processing and analysis

<p align="center">
  <img src="docs/screenshots/09-ghs-stretch-analysis.png" alt="The analysis strip beside a GHS stretch: plate solution, star detection and photometry" width="100%">
</p>
<p align="center"><em>The analysis strip beside a GHS stretch in progress: a plate-solved field with labelled annotations, star detection with FWHM and SNR overlays, interactive photometry, an FFT power spectrum and the histogram readout.</em></p>

Richardson-Lucy deconvolution (FFT-based, Tikhonov regularization, deringing) with a synthetic or empirical PSF. Empirical PSF estimation with moment-based FWHM by eigenvalue decomposition and subpixel peak interpolation. A-trous wavelet denoise with per-scale thresholds. Bias, dark and flat calibration with median-combined masters, EXPTIME-ratio dark scaling and strict master-to-light dimension validation. OSC debayer (RGGB, BGGR, GRBG, GBRG, honouring XBAYROFF and YBAYROFF). Star detection with flux, FWHM and SNR, and interactive photometry with optional Gaia DR3 cross-match. A 64K-bin histogram, downsampled to 512 bins for display, with auto-STF. FFT power spectrum. For IFU cubes, click-to-extract spectra with automatic wavelength unit conversion; cubes open through a memory-mapped lazy reader, so the initial open does not read the whole file.

<p align="center">
  <img src="docs/screenshots/14-header-explorer.png" alt="The FITS header explorer with categorized keywords and the extension list" width="100%">
</p>
<p align="center"><em>The header explorer: FITS keywords grouped into image, observation, instrument, WCS and other, with search, per-keyword copy, and the extension list for multi-extension files.</em></p>

Plate solving goes through astrometry.net and needs a free nova.astrometry.net API key entered in Settings; large images are auto-downsampled and the result is rescaled to full resolution. WCS is handled by the [wcs](https://github.com/cds-astro/wcs-rs) crate (about twenty FITS projections, CD/PC/CDELT conventions) with SIP distortion applied in the wrapper; see [ADR 0001](docs/adr/0001-wcs-rs-for-wcs-engine.md). A synthetic data generator produces star fields with configurable distributions, PSF models, a CCD noise model and a ground-truth catalogue CSV for validating photometry and alignment.

More screens are in [`docs/screenshots/`](docs/screenshots).

## Install and build

| Platform | Download |
|----------|----------|
| **macOS** (universal: Apple Silicon and Intel) | [`.dmg`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **Windows** | [`.msi`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) / [`.exe`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |
| **Linux** | [`.deb`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) / [`.AppImage`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) / [`.rpm`](https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/latest) |

Install scripts:

```bash
# macOS
curl -fsSL https://raw.githubusercontent.com/samuelkriegerbonini-dev/AstroBurst/main/scripts/install-macos.sh | bash

# Linux (Debian/Ubuntu)
curl -fsSL https://raw.githubusercontent.com/samuelkriegerbonini-dev/AstroBurst/main/scripts/install-linux.sh | bash
```

Build from source:

```bash
git clone https://github.com/samuelkriegerbonini-dev/AstroBurst.git
cd AstroBurst
pnpm install
pnpm tauri dev
```

`make dev`, `make build`, `make test` and `make lint` wrap the common dev, build, test and lint commands. CI builds and tests with current stable Rust, Node.js 22 and pnpm 10; the Tauri CLI tracks 2.10. Platform prerequisites are listed in [CONTRIBUTING.md](CONTRIBUTING.md#development-setup). WebGPU (Vulkan, Metal or DX12) is used for the preview when available; without it the app renders through a CPU worker.

## Quick start

1. **Drop** a file, a ZIP (nested ZIPs are followed up to a depth limit) or a folder into the window. `.fits`, `.fit`, `.fts`, `.asdf` and `.zip` are recognised.
2. **Look.** Click a file for its preview, histogram and headers. Set stretch, limits and colormap in the display bar, or hit Auto STF. Hover for value, unit, ERR, DQ and coordinates. Use the HDU panel to switch to another extension or array.
3. **Measure.** Draw a region, add a background annulus, read the statistics. Plot a radial profile or a line cut. Import or export DS9 `.reg` files.
4. **Compose.** Open the Compose tab and walk the ten steps.
5. **Process.** Debayer, background extraction, wavelet denoise, PSF estimation, deconvolution and the stretch tools sit in the Processing tab; calibration, subframe gating, stacking and drizzle in the Stacking tab.
6. **Export.** PNG at 8 or 16 bits, FITS with preserved WCS and metadata, or a ZIP bundle of all channels plus the composite.

Three HST/WFPC2 narrowband frames of the Eagle Nebula (M16) ship in [`exampleFits/sample-data/`](exampleFits/sample-data) for a first run: [OIII] 502 nm, H-alpha 656 nm and [SII] 673 nm, public domain (NASA/ESA). A ground-truth catalogue for photometry checks is in `exampleFits/sample-data/synthetic_catalog.csv`. For your own data, MAST and the ESA Hubble Science Archive publish JWST and HST `_cal.fits` and `_i2d.fits` products, and Roman `.asdf` products under instrument WFI.

## Headless server and Python client

The headless server and the v2 API were contributed by [Jae-Joon Lee](https://github.com/leejjoon).

`astroburst-server` is a separate Axum binary built behind the `server` feature. It shares the FITS and ASDF readers, the display controls and the region code with the desktop app. It renders on the CPU: there is no GPU dependency anywhere in the Rust crate. Run it on whatever machine holds the data and reach it over an SSH tunnel.

```bash
cd src-tauri
cargo run --bin astroburst-server \
  --no-default-features \
  --features server,astrometry-net,asdf-full,vizier
# Listening on 127.0.0.1:8080
```

Two API generations are served. **v1**: sessions, FITS open and header, render, auto-STF and viewport, stacking, drizzle and pipeline runs, with async jobs you can poll, cancel or follow as a server-sent event stream. **v2**: session lifecycle with keepalive, image open and HDU switching, structure, header and WCS inspection, cutouts by pixel box or sky position or region shape, block binning, pixel-to-sky and sky-to-pixel and angular separation, pixel probe, statistics, histogram, and a render endpoint taking the same display settings as the desktop bar.

The async Python client in [`agent/`](agent/README.md) wraps the v1 surface. The v2 endpoints are HTTP-only for now and have no Python method.

```python
import asyncio
from astroburst_client import AstroBurstClient

async def main():
    async with AstroBurstClient("http://localhost:8080") as client:
        session = await client.create_session()
        result = await session.open("/data/m16_ha.fits", slot="ha")
        print(f"{result.width}x{result.height}  median={result.stats.median:.1f}")
        png, stf = await session.stf("ha")
        open("preview.png", "wb").write(png)

asyncio.run(main())
```

Install with `pip install -e ./agent` (add `[image]` for the Pillow helper). Runnable examples: [`agent/examples/`](agent/examples). Full route list and configuration: [src-tauri/SERVER.md](src-tauri/SERVER.md). Client reference: [agent/README.md](agent/README.md).

## Supported formats

**FITS.** Memory-mapped reading. Multi-extension files with auto SCI selection, or any extension by `path#hdu=<n>`. Tile-compressed HDUs are decoded natively: ZCMPTYPE `RICE_1`, `GZIP_1`, `GZIP_2` and `NOCOMPRESS`, including quantized floats and plane-aligned compressed cubes (ZTILE3=1). Not supported: `PLIO_1`, `HCOMPRESS_1`, non-plane-aligned 3D tiling (ZTILE3>1), per-pixel null bitmaps and per-tile ZBLANK columns.

**ASDF.** A native reader written in Rust, so Roman and JWST pipeline products open without a Python environment: block index, YAML tree, science-array discovery, and gWCS extracted and approximated to a standard projection for readout. zlib decompression is always available; bzip2 and lz4 come with the `asdf-full` feature, which is on by default but must be listed explicitly in `--no-default-features` builds. Not supported: external (exploded) block files, arrays of rank other than 2 or 3, and dtypes outside the decoded set. Each failure reports a named error rather than passing silently.

**Containers.** ZIP archives, including nested ones up to a depth limit, and whole directories.

**Export.** PNG at 8 or 16 bits, FITS with BITPIX 16, float32 or float64 and optional Rice compression at a configurable quantize level, WCS and metadata preservation with `PROGRAM` and `HISTORY` provenance cards, RGB FITS cubes, and a ZIP bundle. Processing stays in f32/f64 with no quantization; quantization happens only where you ask for it, in 8/16-bit PNG export, integer-BITPIX or Rice FITS export, and the 8-bit deep-zoom tiles.

## How it compares

AstroBurst sits between a viewer and a processing suite, and is behind the established tools in both directions.

- **DS9** is the reference FITS viewer. AstroBurst matches a useful part of it: stretch and limit algorithms, colormaps, frames, region shapes with DS9 `.reg` exchange, and per-region statistics. DS9 is ahead on frame management and blinking, catalogue and image-server integration, contours, 3D rendering, and interoperability through SAMP and XPA, none of which AstroBurst has.
- **jdaviz** is the JWST and Roman quick-look toolset from STScI. AstroBurst overlaps on ASDF, DQ decoding and unit-aware readout, and opens those files without Python. jdaviz is far ahead on spectroscopy: Specviz, Cubeviz and Mosviz offer line fitting, model fitting and cube analysis that AstroBurst does not attempt, and it lives inside a notebook where results are scriptable end to end.
- **Siril** is the closest free processing comparison. Both do calibration, registration, stacking and stretching. Siril has a mature scripting language, a much larger user base and years of field testing; AstroBurst has no scripting language yet.
- **PixInsight** is the commercial reference for processing depth. It has PixelMath, mosaic stitching, a process container and history model, and an enormous body of published workflows. AstroBurst has none of those.

Where AstroBurst is genuinely different: a single desktop app that reads ASDF natively without Python, decodes DQ against named instrument bit tables, and exposes the same readers, display controls and region code through a headless REST API.

## Project status

Pre-1.0, developed by one maintainer with outside contributions. Formats, command names and the REST API may change before 1.0.

The current version is 0.5.8. The FITS and ASDF readers, display path, compose wizard and export are used daily. CI runs five jobs on every pull request and on pushes to main: typecheck plus a production build, eslint with zero warnings allowed, `cargo test --lib` on Windows, `cargo test --all-features` on Ubuntu and macOS (which covers the headless server), and the vitest suite. The repository contains 850 Rust test functions (779 `#[test]` and 71 `#[tokio::test]`) and 188 frontend test cases across 15 vitest files.

Known limits:

- There is no benchmark suite. No performance figure is published because none is reproducible from this repository.
- The GPU is used for display only. Every algorithm runs on the CPU in Rust.
- Plate solving needs a nova.astrometry.net account and network access; SPCC needs VizieR.
- Radial profiles, line cuts, `.reg` import and export, the DQ overlay and DQ-masked statistics are desktop-only. The compose wizard has no server routes.
- The Python client covers the v1 API only.
- No SAMP or XPA interoperability, no PixelMath, no mosaic stitching, no plugin or scripting system.

Next up: MAST API access, a before/after preview per wizard step, levels and selective saturation, bad-pixel correction. Longer term: GPU compute for stacking and alignment, plugin hooks.

## Contributing

Setup, code style, tests, the architecture overview and the current priority areas are in [CONTRIBUTING.md](CONTRIBUTING.md), so there is one list to keep current. Contributions require the [CLA](CLA.md). Please read the [Code of Conduct](CODE_OF_CONDUCT.md), and report security issues through [SECURITY.md](SECURITY.md).

The headless server, the v2 API, the Python client, the WCS engine migration, FITS tile compression, zscale and the colormaps were contributed by [Jae-Joon Lee](https://github.com/leejjoon).

AstroBurst is free and GPLv3. If it helps your work, you can [support development on Ko-fi](https://ko-fi.com/astroburst).

<!-- SUPPORTERS:START -->
<!-- SUPPORTERS:END -->

## License

GPLv3. See [LICENSE](LICENSE).

---

<p align="center">
  <sub>Created by <a href="https://github.com/samuelkriegerbonini-dev">Samuel Krieger</a></sub>
</p>
