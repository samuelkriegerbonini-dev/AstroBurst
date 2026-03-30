# Sample FITS Data

Narrowband FITS images from the **Hubble Space Telescope** (WFPC2) for testing AstroBurst's processing pipelines.

## Files

| File | Filter | Wavelength | Hubble Palette | Description |
|------|--------|-----------|----------------|-------------|
| `502nmos.fits` | [OIII] | 502 nm | Blue | Oxygen III emission |
| `656nmos.fits` | Hα | 656 nm | Green | Hydrogen alpha emission |
| `673nmos.fits` | [SII] | 673 nm | Red | Sulfur II emission |

## Specifications

| Property | Value |
|----------|-------|
| Instrument | HST/WFPC2 (Detector 4) |
| Dimensions | 1600 x 1600 px |
| BITPIX | -32 (float32) |
| File size | ~9.8 MB each |
| Projection | TAN with CD matrix |
| Origin | STScI-STSDAS |
| Target | Eagle Nebula / M16 (RA 274.71, Dec -13.82) |

## Test Coverage

These files exercise the following AstroBurst features:

- **FITS I/O**: single-HDU float32, BSCALE/BZERO identity
- **Narrowband detection**: FILTER keyword matching for [OIII], Hα, [SII]
- **Hubble Palette (SHO)**: [SII] to R, Hα to G, [OIII] to B
- **Channel blending**: 3-channel narrowband with preset weights
- **Phase correlation alignment**: slight WCS offsets between frames
- **Star detection**: point sources against nebular background
- **Auto-STF**: low dynamic range typical of narrowband data
- **Background extraction**: gradient from mosaic edges
- **SCNR**: green excess from dominant Hα signal
- **Header explorer**: rich WCS + observation keywords

## Quick Start

Open all three files in AstroBurst, then use the ComposeWizard:

1. **Channels**: Auto Map detects filters and assigns SHO bins
2. **Align**: Phase correlation or affine (slight offsets present)
3. **Blend**: Select SHO preset (or HOO, Foraxx, etc.)
4. **Calibrate**: Auto WB normalizes channel medians
5. **Stretch**: Masked stretch with star protection
6. **Export**: PNG 16-bit or RGB FITS

## License

Public domain. NASA/ESA Hubble Legacy Archive, Space Telescope Science Institute (STScI).
