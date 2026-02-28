# Sample FITS Data

This directory contains narrowband FITS images from the **Hubble Space Telescope** (WFPC2 instrument) for testing AstroBurst's processing pipelines.

## Files

| File | Filter | λ (nm) | Description |
|------|--------|--------|-------------|
| `502nmos.fits` | [OIII] | 502 | Oxygen III emission line |
| `656nmos.fits` | Hα | 656 | Hydrogen alpha emission line |
| `673nmos.fits` | [SII] | 673 | Sulfur II emission line |

## Specifications

- **Instrument:** HST/WFPC2 (Detector 4)
- **Dimensions:** 1600 × 1600 pixels
- **BITPIX:** -32 (IEEE 754 single-precision float)
- **File size:** ~9.8 MB each
- **WCS:** TAN projection with CD matrix
- **Origin:** STScI-STSDAS
- **Target region:** RA ≈ 274.71°, Dec ≈ -13.82° (Eagle Nebula / M16 region)

## Test Scenarios

These files are ideal for testing:

1. **FITS I/O** — Standard single-HDU float32 images
2. **Hubble Palette** — Classic SHO mapping ([SII]→R, Hα→G, [OIII]→B)
3. **RGB Composition** — Three-channel narrowband combination
4. **Auto-alignment** — Slight WCS offsets between frames
5. **STF Stretch** — Low dynamic range astronomical data
6. **Header Explorer** — Rich FITS headers with WCS keywords
7. **Star Detection** — Point sources in nebular background
8. **Histogram Analysis** — Typical astronomical pixel distributions

## Usage

```bash
# Load all three for Hubble palette composition
# In AstroBurst: File → Open → select all three files
# Then: RGB Compose → Auto-detect narrowband filters

# Or via CLI for testing
cargo test --all-features
```

## Source

Data from the Hubble Legacy Archive (HLA) / Space Telescope Science Institute (STScI).
These images are in the public domain as works of the U.S. government (NASA/ESA).
