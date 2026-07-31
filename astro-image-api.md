# AstroImage Analysis API — v1.0

**A stateful FITS image analysis service designed for AI astronomer agents.**

This API lets an agent open large 2D FITS images once, then iteratively inspect, render, measure, and analyze them through cheap incremental calls. It is built around the reality of agentic work: the agent cannot "see" a FITS file directly, so it works in a loop of *render → look at PNG → adjust parameters → render again*, interleaved with quantitative calls (statistics, histograms, photometry) that guide those adjustments.

---

## Table of Contents

1. [Design Principles for Agent Use](#1-design-principles-for-agent-use)
2. [Sessions & File Management](#2-sessions--file-management)
3. [Inspection: Headers, WCS, Structure](#3-inspection-headers-wcs-structure)
4. [Rendering: PNG Generation](#4-rendering-png-generation)
5. [Pixel Statistics & Histograms](#5-pixel-statistics--histograms)
6. [Cutouts & Cropping](#6-cutouts--cropping)
7. [Coordinate Transformations](#7-coordinate-transformations)
8. [Background Estimation](#8-background-estimation)
9. [Source Detection](#9-source-detection)
10. [Photometry](#10-photometry)
11. [PSF / Image Quality](#11-psf--image-quality)
12. [Profiles & Cross-Sections](#12-profiles--cross-sections)
13. [Image Arithmetic & Combination](#13-image-arithmetic--combination)
14. [Alignment & Reprojection](#14-alignment--reprojection)
15. [Filtering & Cosmetic Cleaning](#15-filtering--cosmetic-cleaning)
16. [Catalogs & Cross-Matching](#16-catalogs--cross-matching)
17. [Regions & Annotations](#17-regions--annotations)
18. [Errors & Conventions](#18-errors--conventions)
19. [Worked Agent Workflows](#19-worked-agent-workflows)
20. [Endpoint Classification: Difficulty × Usage Frequency](#20-endpoint-classification-difficulty--usage-frequency)
21. [Implementation Plan](#21-implementation-plan)
22. [Basing the Tool on AstroBurst: Feasibility Assessment](#22-basing-the-tool-on-astroburst-feasibility-assessment)
23. [Adopting the CDS Rust Stack: fitsrs, wcs-rs, mapproj](#23-adopting-the-cds-rust-stack-fitsrs-wcs-rs-mapproj)

---

## 1. Design Principles for Agent Use

- **Stateful sessions.** Opening a multi-GB mosaic is expensive. A session pins the file (memory-mapped) server-side; all subsequent calls reference `session_id` and are fast.
- **Everything returns JSON; images return JSON + an image reference.** Rendered PNGs are returned as a URL (`image_url`) and optionally inline base64 (`return_base64=true`) so a multimodal agent can look at them directly. Every render response *also* echoes back the fully resolved parameters (the actual `vmin`/`vmax` that zscale computed, the pixel region shown, the WCS of the corners), so the agent can reason numerically about what it just saw.
- **Idempotent, parameter-complete calls.** Any render can be reproduced by resending the echoed parameter block, tweaked. The intended loop is: render → inspect → tweak one parameter → render.
- **Quantitative fallbacks everywhere.** When a picture is ambiguous (e.g., "is the scale clipping the faint wisps?"), the agent should drop to `/stats`, `/histogram`, or `/profile` rather than guessing from pixels in a PNG.
- **Pixel convention.** All pixel coordinates are **0-indexed, (x, y) = (column, row)**, matching numpy slicing transposed to FITS display convention. Sky coordinates are ICRS degrees unless stated. Regions are specified as `[x0, y0, width, height]` or by sky center + size.
- **Downsampling is explicit.** Any render of a region larger than `max_render_px` (default 2048×2048) is automatically binned; the response reports `binning_applied` so the agent knows one PNG pixel ≠ one detector pixel.

Base URL: `https://<host>/api/v1`. All bodies are `application/json`. Authentication: `Authorization: Bearer <token>`.

---

## 2. Sessions & File Management

### `POST /sessions`
Open a FITS file and create an analysis session.

**Request**
```json
{
  "source": "file:///data/obs/coadd_r_2026-03-14.fits",   // or s3://, https://
  "hdu": null,              // null = auto-select first image HDU
  "readonly": true,
  "label": "NGC 4151 r-band coadd"
}
```

**Response `201`**
```json
{
  "session_id": "sx_9f3a12",
  "file": "coadd_r_2026-03-14.fits",
  "hdu_index": 1,
  "shape": [21600, 21600],          // [ny, nx]
  "dtype": "float32",
  "bunit": "electron/s",
  "wcs_present": true,
  "sky_footprint": {
    "center": {"ra": 182.6357, "dec": 39.4058},
    "corners_radec": [[182.94,39.10],[182.33,39.10],[182.33,39.71],[182.94,39.71]],
    "pixel_scale_arcsec": 0.263
  },
  "expires_at": "2026-07-04T22:00:00Z"
}
```

### `GET /sessions/{id}` — session status, memory footprint, open HDU.
### `POST /sessions/{id}/hdu` — switch active HDU: `{"hdu": 2}` (e.g., move from SCI to WEIGHT or MASK extension).
### `DELETE /sessions/{id}` — close and free resources.
### `POST /sessions/{id}/keepalive` — extend TTL (default TTL 60 min idle).

**Derived products.** Several endpoints (cutout, arithmetic, align) create new in-memory images. These are addressable as `image_ref` strings, e.g. `"sx_9f3a12:cutout_003"`, and accepted anywhere a session's image is implied via an optional `"image": "<image_ref>"` field. `GET /sessions/{id}/images` lists them; `POST /sessions/{id}/images/{ref}/save` writes one to FITS.

---

## 3. Inspection: Headers, WCS, Structure

### `GET /sessions/{id}/structure`
List all HDUs: index, name, type (image/table), shape, dtype. First call an agent should make on an unfamiliar file — modern survey products often hide the science array in extension 1 and carry MASK/WEIGHT/VARIANCE alongside.

### `GET /sessions/{id}/header?hdu=1&keys=EXPTIME,FILTER,DATE-OBS,AIRMASS,GAIN`
Returns requested cards (or full header if `keys` omitted) as `{key: {value, comment}}`. Supports wildcards: `keys=CD*_*` for the WCS matrix.

### `GET /sessions/{id}/wcs`
Parsed WCS summary: projection type, reference pixel/coordinates, CD matrix, pixel scale (arcsec/px, both axes), rotation angle (deg E of N), parity (flipped or not), and SIP/TPV distortion presence. Agents should check `rotation_deg` and `parity` before interpreting PNG orientation.

---

## 4. Rendering: PNG Generation

The workhorse endpoint. Designed for the iterative look-adjust-look loop.

### `POST /sessions/{id}/render`

**Request (all fields optional except none — an empty body renders the full frame with defaults)**
```json
{
  "image": null,                      // image_ref; null = session's active HDU
  "region": {                         // omit for full frame (auto-binned)
    "type": "pixel",                  // "pixel" | "sky"
    "x": 8000, "y": 11200,            // pixel: lower-left corner...
    "width": 1024, "height": 1024
    // sky alternative: {"type":"sky","ra":182.6357,"dec":39.4058,"size_arcmin":3.0}
  },
  "scale": {
    "algorithm": "zscale",            // zscale | minmax | percentile | manual
    "stretch": "linear",              // linear | log | sqrt | asinh | power | histeq
    "vmin": null, "vmax": null,       // used when algorithm = manual
    "percentile": [1.0, 99.5],        // used when algorithm = percentile
    "asinh_a": 0.1,                   // softening for asinh
    "power": 2.0,                     // exponent for power stretch
    "zscale_contrast": 0.25
  },
  "colormap": "heat",                 // gray | heat | viridis | inferno | plasma | cool | rainbow | bone
  "invert_cmap": false,
  "output": {
    "max_px": 1024,                   // long side of the PNG; server bins/interp as needed
    "return_base64": false,
    "format": "png"                   // png | jpeg
  },
  "orientation": "sky",               // "sky" (N up, E left) | "pixel" (raw array orientation)
  "overlays": [                       // optional; see §17 for full overlay grammar
    {"type": "compass"},
    {"type": "scalebar", "length_arcsec": 30},
    {"type": "grid", "system": "radec", "spacing_arcsec": 60},
    {"type": "crosshair", "ra": 182.6357, "dec": 39.4058},
    {"type": "markers", "source": "detections:run_007", "shape": "circle", "radius_px": 8, "label": "id"}
  ],
  "mask": {"nan_color": "#404040", "apply_dq_mask": false}
}
```

**Response `200`**
```json
{
  "image_url": "https://<host>/renders/sx_9f3a12/r_00042.png",
  "render_id": "r_00042",
  "resolved": {
    "region_px": [8000, 11200, 1024, 1024],
    "region_sky": {"center": {"ra": 182.6401, "dec": 39.4212}, "size_arcmin": [4.49, 4.49]},
    "vmin": -0.0123, "vmax": 0.4871,        // ← actual values zscale chose
    "stretch": "linear",
    "binning_applied": 1,                    // 1 = native resolution
    "png_scale_arcsec_per_px": 0.263,
    "orientation": {"north_angle_deg": 0.0, "east": "left"},
    "clipped_fraction": {"below_vmin": 0.021, "above_vmax": 0.004}
  }
}
```

**Notes for agents**
- `resolved.vmin/vmax` is the key feedback signal. If a zscale render looks washed out or too hard, take these numbers, consult `/histogram`, and re-render with `algorithm: "manual"` and adjusted `vmin`/`vmax`. This is *the* canonical loop (see §19, Workflow A).
- `clipped_fraction` tells you how much of the pixel distribution is saturated to pure black/white in the PNG — a large `above_vmax` with a faint-target science case means you are probably fine; a large `below_vmin` means faint structure may be hidden.
- Use `stretch: "asinh"` with small `asinh_a` (0.01–0.1) to show faint outskirts and bright cores simultaneously; `log` for nebulosity; `histeq` as a quick "show me everything" diagnostic (never for judging relative brightness).
- To compare two renders fairly, hold `vmin/vmax/stretch` fixed (manual) and vary only the region or image.

### `POST /sessions/{id}/render/rgb`
Compose a 3-color PNG from three aligned images (session HDUs or `image_ref`s):
```json
{
  "r": {"image": "sx_i_band:base", "scale": {"algorithm": "percentile", "percentile": [1, 99]}},
  "g": {"image": "sx_r_band:base"},
  "b": {"image": "sx_g_band:base"},
  "region": {"type": "sky", "ra": 182.6357, "dec": 39.4058, "size_arcmin": 4},
  "mode": "lupton",                  // lupton (asinh RGB) | independent
  "lupton": {"stretch": 0.5, "Q": 8}
}
```
Inputs must share a pixel grid; if not, call `/align` first (§14). Response format matches `/render`.

---

## 5. Pixel Statistics & Histograms

### `POST /sessions/{id}/stats`
```json
{
  "region": {"type": "pixel", "x": 8000, "y": 11200, "width": 1024, "height": 1024},
  "sigma_clip": {"sigma": 3.0, "maxiters": 5},     // null = no clipping
  "percentiles": [0.5, 1, 5, 25, 50, 75, 95, 99, 99.5],
  "exclude_nan": true
}
```
**Response**
```json
{
  "npix": 1048576, "n_valid": 1046301, "n_nan": 2275,
  "min": -0.812, "max": 5931.2,
  "mean": 0.0142, "median": 0.0031, "std": 1.882,
  "clipped": {"mean": 0.0029, "median": 0.0030, "std": 0.0186, "n_rejected": 8412},
  "percentiles": {"0.5": -0.041, "1": -0.036, "...": "...", "99.5": 0.912},
  "mad_std": 0.0179
}
```
`clipped.std` ≈ background RMS in an emptyish region — the number to use when choosing detection thresholds or a manual `vmin ≈ median − 1σ`, `vmax ≈ median + kσ`.

### `POST /sessions/{id}/histogram`
```json
{
  "region": {"type": "pixel", "x": 8000, "y": 11200, "width": 1024, "height": 1024},
  "bins": 200,
  "range": null,                  // null = robust auto-range (0.1–99.9 pct); or [lo, hi]
  "log_counts": true,
  "render_png": true              // also return a plot the agent can look at
}
```
**Response:** bin edges, counts, mode estimate, plus `image_url` of the plot when requested. Typical agent use: find where the sky mode sits and where the source tail begins, then set `vmin` just below the mode and `vmax` at the knee of the bright tail.

### `POST /sessions/{id}/pixel`
Point query: `{"x": 8123, "y": 11302, "box": 5}` → value at pixel, plus min/max/mean of the surrounding box and the sky coordinate. Cheap sanity check ("is that white speck real or a hot pixel?" — a single hot pixel has no elevated neighbors).

---

## 6. Cutouts & Cropping

### `POST /sessions/{id}/cutout`
Create a true data cutout (not a PNG) as a derived image, optionally saved as FITS.

```json
{
  "region": {"type": "sky", "ra": 182.6357, "dec": 39.4058, "size_arcmin": [3.0, 3.0]},
  "preserve_wcs": true,
  "save": {"path": "/data/out/ngc4151_r_3am.fits", "overwrite": false}   // optional
}
```
**Response:** `{"image_ref": "sx_9f3a12:cutout_003", "shape": [684, 684], "wcs": {...}, "saved_path": "..."}`.

All analysis endpoints accept the `image_ref`, so heavy work (source detection, photometry, filtering) can be done on the small cutout instead of the full mosaic. Partial-overlap behavior: pixels outside the parent frame are NaN; response includes `fraction_on_image`.

### `POST /sessions/{id}/bin`
Block-average or block-sum rebinning: `{"factor": 4, "method": "mean"}` → derived image. Useful for finding low-surface-brightness structure before zooming in at native resolution.

---

## 7. Coordinate Transformations

### `POST /sessions/{id}/wcs/pix2sky` and `/wcs/sky2pix`
Batch-capable:
```json
{"coords": [[8123, 11302], [10000, 10000]]}          // pix2sky
{"coords": [{"ra": 182.6357, "dec": 39.4058}]}       // sky2pix
```
Response includes `on_image: true/false` per point. `sky2pix` also accepts sexagesimal strings (`"12:10:32.6 +39:24:21"`) and resolvable object names (`{"name": "NGC 4151"}`, resolved via Sesame/SIMBAD; response echoes the resolved ICRS position).

### `POST /sessions/{id}/wcs/separation`
Angular separation and position angle between two sky points or two pixels.

---

## 8. Background Estimation

### `POST /sessions/{id}/background`
2D background and RMS maps (photutils `Background2D`-style):
```json
{
  "image": null,
  "box_size": 128, "filter_size": 3,
  "estimator": "sextractor",          // sextractor | median | mmm
  "mask_sources": true,               // sigma-clip + dilate a source mask first
  "outputs": ["background", "rms"]    // each becomes an image_ref
}
```
**Response:** `{"background_ref": "...:bkg_001", "rms_ref": "...:rms_001", "global_median_bkg": 0.0030, "global_median_rms": 0.0181}` plus optional quicklook PNGs.

### `POST /sessions/{id}/background/subtract`
`{"background_ref": "...:bkg_001"}` → new background-subtracted `image_ref`. Shortcut: `{"mode": "auto"}` runs estimate+subtract in one call with defaults.

---

## 9. Source Detection

### `POST /sessions/{id}/detect`
```json
{
  "image": "sx_9f3a12:cutout_003",
  "method": "segmentation",           // segmentation (SEP/SExtractor-like) | daofind | peaks
  "threshold_sigma": 1.5,             // over local background RMS
  "background": "auto",               // "auto" | image_ref | "none" (already subtracted)
  "npixels_min": 5,
  "deblend": {"enabled": true, "nlevels": 32, "contrast": 0.005},
  "fwhm_px": 3.5,                     // required for daofind
  "filters": {"max_sources": 5000, "border_px": 10, "max_ellipticity": null},
  "store_as": "detections:run_007"
}
```
**Response**
```json
{
  "catalog_id": "detections:run_007",
  "n_sources": 412,
  "columns": ["id","x","y","ra","dec","flux","peak","fwhm_px","a","b","theta","ellipticity","snr","flags"],
  "sources": [ {"id": 1, "x": 233.1, "y": 411.7, "ra": 182.6512, "dec": 39.4102,
                "flux": 15.32, "peak": 2.11, "fwhm_px": 3.4, "snr": 88.2, "flags": 0}, "..." ],
  "truncated": false
}
```
Stored catalogs are referenced by `catalog_id` in overlays (§4/§17), photometry (§10), and cross-matching (§16). `GET /sessions/{id}/catalogs/{catalog_id}?sort=-flux&limit=50&format=json|csv` pages through results.

Agent tip: always render the detections overlaid (`overlays: [{"type":"markers","source":"detections:run_007"}]`) before trusting them — threshold too low shows rings of noise detections around bright stars; too high misses the target.

---

## 10. Photometry

### `POST /sessions/{id}/photometry/aperture`
```json
{
  "image": null,
  "positions": {"catalog_id": "detections:run_007"},   // or explicit list of {x,y} / {ra,dec}
  "apertures": {"shape": "circle", "r_px": [3, 5, 8]},  // multiple radii → curve of growth
  "annulus": {"r_in_px": 12, "r_out_px": 18, "estimator": "sigma_clipped_median"},
  "recentroid": true,
  "zeropoint": 27.85,                 // optional; null → instrumental mags only
  "gain": 4.1,                        // for shot-noise term in errors
  "output_columns": ["flux", "flux_err", "mag", "mag_err", "sky_per_px", "aperture_area", "flags"]
}
```
**Response:** per-source rows for each aperture radius; flags mark NaN contamination, edge truncation, saturation (`peak > SATURATE`), and crowding (neighbor inside annulus). Elliptical and rectangular apertures supported via `shape: "ellipse"` (`a_px, b_px, theta_deg`).

### `POST /sessions/{id}/photometry/psf`
PSF-fitting photometry for crowded fields. Builds an effective PSF from bright unsaturated stars (or accepts `psf_ref` from §11), then fits groups:
```json
{"positions": {"catalog_id": "detections:run_007"}, "psf": "auto", "fit_shape_px": 11, "group_radius_px": 8}
```
Response includes fitted fluxes, fit χ², and a `residual_ref` image the agent should render to verify subtraction quality.

### `POST /sessions/{id}/photometry/curve_of_growth`
Single target, radii sweep → table + plot PNG. Use to pick the aperture radius (where the curve flattens) and to check for PSF anomalies.

---

## 11. PSF / Image Quality

### `POST /sessions/{id}/psf/measure`
Seeing/FWHM estimate over the frame:
```json
{"star_selection": "auto", "n_stars_max": 200, "model": "moffat", "map_grid": [4, 4]}
```
**Response:** global median FWHM (px and arcsec), Moffat β, ellipticity median and PA, per-grid-cell FWHM map (JSON + optional rendered map PNG), and the list of stars used. Detects PSF spatial variation and trailing (elongated PSF at consistent PA ⇒ tracking error).

### `POST /sessions/{id}/psf/build`
Construct an empirical PSF stamp (`image_ref`) from selected stars — feed to PSF photometry or subtraction.

### `POST /sessions/{id}/psf/fit_single`
Fit one source: `{"x": 233.1, "y": 411.7, "model": "gaussian|moffat", "box": 21}` → centroid, FWHM (x/y), ellipticity, PA, peak, local background, and a data/model/residual triptych PNG. The standard "is this a star or is it extended / a cosmic ray?" check: CRs are sharper than the PSF; galaxies broader.

---

## 12. Profiles & Cross-Sections

### `POST /sessions/{id}/profile/radial`
```json
{"center": {"ra": 182.6357, "dec": 39.4058}, "r_max_arcsec": 60, "bin_arcsec": 1.0,
 "azimuth_range_deg": null, "statistic": "median", "background_subtract": "annulus", "render_png": true}
```
Returns r vs. surface brightness (with errors), half-light radius estimate, and a log-log plot PNG. Essential for galaxy profiles, halo detection, and verifying flat sky around targets.

### `POST /sessions/{id}/profile/line`
Arbitrary line cut: `{"start": {"x":100,"y":100}, "end": {"x":400,"y":250}, "width_px": 3}` → distance vs. value table + plot. Use for trap columns, satellite trails, gradient checks, spiral-arm cuts.

### `POST /sessions/{id}/profile/box`
Collapse a rectangle along x or y (`statistic: mean|median|sum`) — row/column defect diagnosis, sky gradient along an axis.

---

## 13. Image Arithmetic & Combination

### `POST /sessions/{id}/arith`
Expression evaluation over registered images:
```json
{
  "expr": "(a - b) / c",
  "operands": {"a": "sx_9f3a12:base", "b": "sx_dark:base", "c": "sx_flat:norm"},
  "store_as": "calibrated_001",
  "require_same_grid": true
}
```
Supported: `+ - * /`, scalar constants, `sqrt`, `log10`, `abs`, `clip(x, lo, hi)`, `where(cond, x, y)`. NaN-propagating. Classic uses: dark subtraction, flat fielding, difference imaging sanity checks (`a - b` of aligned epochs), SNR maps (`sci / rms`).

### `POST /combine`  *(session-independent; takes a list of sources or image_refs)*
```json
{
  "inputs": ["sx_e1:aligned", "sx_e2:aligned", "sx_e3:aligned"],
  "method": "median",                // mean | median | sum | sigma_clip_mean
  "sigma_clip": {"sigma": 3.0},
  "weights": "inverse_variance",     // null | inverse_variance | explicit list
  "store_in_session": "sx_e1", "store_as": "stack_r"
}
```
Inputs must be on a common grid (see §14). Response reports per-pixel input counts (`nmap_ref`) so the agent can see coverage edges.

---

## 14. Alignment & Reprojection

### `POST /sessions/{id}/align`
Reproject this session's image onto a target grid:
```json
{
  "target": {"session": "sx_ref"},    // or explicit {"wcs_header": {...}, "shape": [ny,nx]}
  "method": "wcs_interp",             // wcs_interp (WCS-based reprojection) | star_match (asterism matching when WCS is bad/absent)
  "order": "bilinear",                // nearest | bilinear | bicubic | flux_conserving
  "store_as": "aligned_to_ref"
}
```
`star_match` returns the fitted affine transform (shift, rotation, scale) and residual RMS so the agent can judge registration quality (aim ≲0.1–0.2 px for difference imaging).

### `POST /sessions/{id}/astrometry/verify`
Cross-match detected stars against Gaia (§16) and report WCS offset (ΔRA, ΔDec), rotation, and scale residuals. `POST /astrometry/refine` applies the correction to a copy of the WCS. `POST /astrometry/solve` does blind plate solving when the header WCS is missing or untrustworthy.

---

## 15. Filtering & Cosmetic Cleaning

### `POST /sessions/{id}/filter`
```json
{"kind": "gaussian", "sigma_px": 2.0, "store_as": "smooth2"}
```
Kinds: `gaussian` (sigma_px), `median` (size_px), `boxcar` (size_px), `unsharp` (sigma_px, amount), `tophat` (radius_px). Smoothed copies are how you make low-surface-brightness features visible: `filter` → `render` with asinh.

### `POST /sessions/{id}/cosmicrays`
L.A.Cosmic-style CR rejection: `{"gain": 4.1, "readnoise": 6.0, "sigclip": 4.5, "store_as": "crclean"}` → cleaned `image_ref` + CR mask `image_ref` + count. Only meaningful on single exposures, not median stacks.

### `POST /sessions/{id}/mask`
Build/apply boolean masks: from DQ extensions, value thresholds (`"expr": "img > 60000"` for saturation), regions (§17), or catalog footprints (mask out stars). Masks are `image_ref`s usable in `stats`, `detect`, `photometry` (`"mask": "<ref>"` field accepted by those endpoints).

---

## 16. Catalogs & Cross-Matching

### `POST /catalogs/query`
Cone search of external catalogs:
```json
{"catalog": "gaia_dr3", "ra": 182.6357, "dec": 39.4058, "radius_arcmin": 5,
 "columns": ["source_id","ra","dec","phot_g_mean_mag","pmra","pmdec"],
 "filters": {"phot_g_mean_mag": "< 20"}, "store_as": "gaia_field"}
```
Supported: `gaia_dr3`, `panstarrs_dr2`, `2mass`, `sdss_dr17`, `usno_b1`, `simbad`, `ned`, plus `user_upload` (`POST /catalogs/upload` with CSV).

### `POST /catalogs/crossmatch`
```json
{"left": "detections:run_007", "right": "gaia_field", "radius_arcsec": 1.0, "join": "best_symmetric"}
```
Returns matched table with separations, plus unmatched-left (transient candidates / artifacts) and unmatched-right (non-detections) lists. The standard "is my new source real and known?" step.

### `POST /catalogs/{id}/photcal`
Fit a photometric zeropoint by matching instrumental magnitudes to a reference catalog band; returns ZP, scatter, color term (optional), and the outlier list.

---

## 17. Regions & Annotations

### `POST /sessions/{id}/regions`
Define named regions (circle, ellipse, box, polygon, annulus) in pixel or sky coordinates; import/export DS9 `.reg` (`format: "ds9"`). Regions can be used as: analysis footprints (`"region": {"type":"named","name":"blob_A"}` accepted by stats/histogram/detect/photometry), mask sources (§15), and overlays.

### Overlay grammar (used in `/render`)
| type | key fields |
|---|---|
| `markers` | `source` (catalog_id) or `points`, `shape` (circle/cross/diamond), `radius_px`, `color`, `label` (column name) |
| `apertures` | `positions`, `r_px` / ellipse params, `annulus` — draws exactly what photometry measured |
| `region` | `name` or inline region spec |
| `compass` | N/E arrows (auto from WCS) |
| `scalebar` | `length_arcsec` |
| `grid` | `system: radec|pixel`, `spacing_arcsec` |
| `crosshair` | `ra/dec` or `x/y` |
| `contours` | `image` (ref, e.g. a smoothed copy or another band), `levels` or `nlevels+sigma_based`, `color` |
| `text` | `x/y` or `ra/dec`, `text` |

Contour overlays are the standard way to compare two bands without an RGB composite (e.g., radio contours on optical).

---

## 18. Errors & Conventions

**HTTP codes:** `400` bad parameters (message names the offending field), `404` unknown session/ref/catalog, `409` grid mismatch (arith/combine on unaligned images — response suggests `/align`), `410` session expired, `413` region too large for the endpoint (response gives the limit and suggests `/bin` or a smaller region), `422` analysis failed (e.g., zero stars for PSF fit — response says why), `500` internal.

**Error body**
```json
{"error": "region_out_of_bounds",
 "message": "Requested region [22000, 11200, 1024, 1024] exceeds image x-extent 21600.",
 "hint": "Image shape is [21600, 21600]; clip x to <= 20576 or use region.clip=true."}
```
Every error carries a machine-readable `error` code and, where possible, a `hint` with corrected parameters — agents should read hints before retrying.

**Units:** pixel values in native `BUNIT`; sky coordinates ICRS degrees; angles East of North; sizes suffixed `_px`, `_arcsec`, `_arcmin` explicitly everywhere. **NaN policy:** NaNs are preserved and reported (`n_nan`), never silently zeroed.

**Reproducibility:** every response includes `request_echo` (the fully-resolved request after defaulting) and analysis responses include `provenance` (image_ref lineage). An agent can rebuild any product from the transcript.

---

## 19. Worked Agent Workflows

### Workflow A — Iterative display scaling (the canonical loop)
The exact scenario the API is designed around: the agent wants to inspect faint structure near a target.

```
1. POST /sessions                              → session sx_1, shape 21600², pixel scale 0.263"
2. POST /sessions/sx_1/wcs/sky2pix             → target NGC 4151 at (10812, 10630)
3. POST /sessions/sx_1/render
     region: sky, 4', scale: zscale, cmap heat → PNG #1; resolved vmin=-0.012, vmax=0.487
   Agent looks: core saturated white, faint tidal wisp invisible; clipped_fraction.below_vmin = 0.02
4. POST /sessions/sx_1/render
     scale: manual, vmin=-0.012, vmax=0.15, stretch asinh (a=0.05)
   Agent looks: better, but background looks noisy-gray — vmin may be too deep
5. POST /sessions/sx_1/histogram (same region, log_counts)  → sky mode at 0.0030, σ≈0.018
6. POST /sessions/sx_1/render
     vmin = 0.003 − 1σ = -0.015, vmax = 0.003 + 25σ = 0.45, stretch asinh
   Agent looks: wisp clearly visible, core structure retained → done; records final scaling in notes
```

### Workflow B — "Is this smudge real?"
```
1. /render zoomed stamp on the smudge (zscale, gray)          → looks extended
2. /pixel with box=7                                          → peak 0.9, neighbors elevated → not a hot pixel
3. /psf/fit_single at the position                            → FWHM 6.1 px vs field PSF 3.4 px → extended
4. /sessions/{id}/hdu → switch to MASK extension; /pixel      → DQ=0, not a known defect
5. /catalogs/query simbad + /catalogs/crossmatch              → no counterpart within 5"
6. /photometry/aperture with annulus sky                      → mag 21.8 ± 0.1 (ZP from /photcal)
7. /render with crosshair + scalebar overlays                 → finder chart for the report
```

### Workflow C — Quick-look calibration + stack of 3 exposures
```
For each exposure: POST /sessions → /arith (dark, flat) → /cosmicrays
/astrometry/verify on each (Gaia)  → offsets < 0.3", OK
/align exposures 2,3 onto exposure 1 grid (flux_conserving)
POST /combine (median)             → stack_r; render nmap_ref to check coverage
/psf/measure on stack              → seeing 1.1" FWHM, ellipticity 0.04 → good night
/background auto-subtract → /detect (1.5σ, deblend) → /photometry/aperture → /photcal vs PS1
```

### Workflow D — Low-surface-brightness hunt
```
/bin factor 4 (mean) → /filter gaussian σ=3 on the binned image
/render full frame, asinh a=0.02, manual vmin at sky mode, invert gray cmap
Agent spots candidate streamer → /cutout native-res region → /profile/radial and /profile/line across it
Overlay contours of the smoothed image on the native render for the figure
```

---

## 20. Endpoint Classification: Difficulty × Usage Frequency

> **Implementation language: Rust.** This changes the difficulty landscape materially. In Python, most endpoints are thin wrappers over astropy/photutils; in Rust, the battle-tested numerical astronomy layer largely does not exist and must be obtained via **FFI to proven C libraries** or **ported from reference implementations**. Ratings below are for the Rust build.

### 20.0 Rust ecosystem strategy

The single most important architectural decision. We use a **three-layer approach**:

**Layer 1 — FFI to battle-tested C libraries** (where correctness is hardest to reproduce and the C library is small, stable, and universally trusted):

| C library | Role | Rust access | Status |
|---|---|---|---|
| **cfitsio** | FITS I/O, memmap, tile-compressed FITS | `fitsio` crate (maintained bindings) | Ready to use |
| **wcslib** | All WCS: projections, SIP/TPV distortion | **Custom `wcslib-sys` via bindgen** — no maintained crate exists; budgeted in Phase 0 | Build ourselves |
| **SEP** (SExtractor core) | Background2D, segmentation detection + deblending, aperture photometry w/ exact pixel overlap | **Custom `sep-sys` via bindgen** — SEP is a small, clean, dependency-free C lib designed for embedding | Build ourselves |

Binding SEP is the highest-leverage decision in the whole plan: one FFI effort covers §8 (background), §9 (detection), and the geometric core of §10 (aperture photometry) with exactly the algorithms astronomers already trust.

**Layer 2 — Pure Rust** (well-specified algorithms, or places where Rust's performance is the point):

| Concern | Crates / approach |
|---|---|
| HTTP server, sessions | `axum` + `tokio`; session registry in `DashMap`; CPU-bound work via `tokio::task::spawn_blocking` + `rayon` |
| Arrays & math | `ndarray` (+ `ndarray-stats`), `rayon` for parallel tiles, `rustfft` if FFT convolution needed |
| FITS memmap slices | cfitsio section reads; `memmap2` for raw paths |
| Rendering | `image` crate for PNG; **zscale, all stretches (asinh/log/sqrt/power/histeq), and colormap LUTs ported from astropy/matplotlib reference code** — well-specified, straightforward to port, must be regression-tested against the originals |
| Plots (histogram, profiles, triptychs) | `plotters` |
| Statistics | sigma-clipping, MAD, percentiles — implement directly (small, testable) |
| Model fitting (Gaussian/Moffat PSF) | `levenberg-marquardt` or `argmin` |
| Cross-match | k-d tree via `kiddo` on unit-sphere xyz |
| Safe expression eval (`/arith`) | `evalexpr` with a whitelisted function set — never a general interpreter |
| Catalog queries | `reqwest` against TAP/ADQL (Gaia) and VizieR/SIMBAD HTTP endpoints; on-disk cone cache |
| DS9 region parsing | `nom` parser, written in-house (no crate exists) |

**Layer 3 — Escape hatches** (external processes for infra-heavy or research-grade components; keeps the Rust core honest):

- **Blind astrometry** → subprocess call to `solve-field` (astrometry.net binary + index files in the container). Wrapping a subprocess is ★★; reimplementing quad-hashing in Rust is ★★★★★ and worth nobody's time.
- **PSF-fitting photometry & cosmic-ray rejection (initially)** → optional **Python sidecar microservice** (photutils/astroscrappy behind a tiny HTTP shim), same JSON contract, so the API surface is stable while Rust ports are deferred until usage justifies them. The sidecar is an explicit, quarantined dependency — not a creeping one.

**Testing doctrine for a Rust port:** use the Python stack as an **oracle**. A CI job runs astropy/photutils/SEP-python on the golden FITS files and freezes reference outputs (zscale limits, stats, detection catalogs, aperture fluxes) as JSON fixtures; the Rust implementation must match within stated tolerances (bit-exact for stats; <1e-4 relative for photometry; identical zscale vmin/vmax to 4 significant figures). This converts "did we port it right?" from a judgment call into a test suite.

### Difficulty scale (Rust)

| Rating | Meaning | Typical effort |
|---|---|---|
| ★ | Trivial — plumbing over existing layer | < 1 day |
| ★★ | Straightforward — small algorithm or FFI plumbing | 1–3 days |
| ★★★ | Moderate — port a reference algorithm, or nontrivial FFI + validation | ~1 week |
| ★★★★ | Hard — substantial algorithm implementation, tuning, or async job machinery | 1–2 weeks |
| ★★★★★ | Very hard — research-grade tuning or heavy infrastructure; use an escape hatch if possible | 2+ weeks |

Frequency scale (VH/H/M/L) is unchanged from the usage analysis: agent call volume is dominated by the render/stats/histogram loop.

### 20.1 Classification table (Rust)

Δ marks endpoints whose difficulty changed vs. the Python-stack estimate.

| Endpoint | § | Difficulty | Δ | Freq | Rust implementation | Notes |
|---|---|---|---|---|---|---|
| `POST /sessions` (open) | 2 | ★★ | | VH | `fitsio` + DashMap registry | cfitsio handles memmap/compressed FITS |
| `GET /sessions/{id}` / `keepalive` / `DELETE` | 2 | ★ | | M | axum plumbing | |
| `POST /sessions/{id}/hdu` | 2 | ★ | | M | `fitsio` | |
| `GET /images` / `save` | 2 | ★★ | ▲ | M | image_ref registry, `fitsio` write | Header/WCS reconstruction on save |
| `GET /structure` | 3 | ★ | | H | `fitsio` HDU iteration | |
| `GET /header` | 3 | ★ | | H | `fitsio` + glob matching | |
| `GET /wcs` | 3 | ★★ | | H | `wcslib-sys` | Parity/rotation math on CD matrix |
| `POST /render` (core) | 4 | ★★★★ | ▲ | **VH** | zscale + stretch + LUT ports, `image` PNG, rayon-tiled binning | Highest-volume endpoint; algorithms are well-specified ports, tested against astropy oracle |
| `/render` overlays: crosshair, scalebar, compass | 4/17 | ★★ | | H | draw primitives on RGBA buffer | |
| `/render` overlays: markers, apertures, grid, text | 4/17 | ★★★ | ▲ | H | in-house draw layer + font raster (`ab_glyph`) | Text/label rendering is the fiddly part |
| `/render` overlays: contours | 4/17 | ★★★ | | M | marching-squares (`contour-rs` or port) | |
| `POST /render/rgb` | 4 | ★★★ | | L–M | Lupton asinh port | Small, well-documented algorithm |
| `POST /stats` | 5 | ★★ | ▲ | **VH** | ndarray + own sigma-clip | Bit-exact vs astropy fixture |
| `POST /histogram` | 5 | ★★ | | **VH** | ndarray + `plotters` | |
| `POST /pixel` | 5 | ★ | | VH | ndarray slice | |
| `POST /cutout` | 6 | ★★ | | VH | slice + CRPIX shift via wcslib | |
| `POST /bin` | 6 | ★ | | M | ndarray reshape-reduce | |
| `POST /wcs/pix2sky` / `sky2pix` | 7 | ★★ | ▲ | VH | `wcslib-sys` batch calls | Name resolution: `reqwest` → Sesame (★★, Phase 2) |
| `POST /wcs/separation` | 7 | ★ | | M | haversine on unit vectors | |
| `POST /background` / `subtract` | 8 | ★★ | | H | **SEP FFI** (`sep_background`) | Falls out of the SEP binding |
| `POST /detect` (segmentation) | 9 | ★★★ | | H | **SEP FFI** (`sep_extract`) + catalog registry | Deblending comes with SEP |
| `POST /detect` (daofind, peaks) | 9 | ★★★ | ▲ | M | port DAOFIND kernel from photutils | Peaks variant is ★ |
| `GET /catalogs/{id}` | 9 | ★★ | | H | in-memory table + CSV via `csv` crate | |
| `POST /photometry/aperture` | 10 | ★★★ | | H | **SEP FFI** (`sep_sum_circle/ellipse/circann`) + own error model | SEP provides exact pixel-overlap geometry; gain/sky/flag logic is ours |
| `POST /photometry/curve_of_growth` | 10 | ★★ | | M | loop over SEP apertures + `plotters` | |
| `POST /photometry/psf` | 10 | ★★★★★ | | L–M | **Python sidecar (photutils)** initially; Rust port only on demonstrated demand | Grouping + convergence QC is research-grade |
| `POST /psf/measure` (seeing map) | 11 | ★★★★ | | M | own star selection + LM Moffat fits, rayon per-cell | Auto star selection is the tuning burden |
| `POST /psf/build` (ePSF) | 11 | ★★★★ | ▲ | L | ePSF stacking port, or sidecar | Defer |
| `POST /psf/fit_single` | 11 | ★★★ | ▲ | H | `levenberg-marquardt` Gaussian/Moffat + triptych render | High value; validate vs astropy.modeling fixtures |
| `POST /profile/radial` | 12 | ★★ | | M | radius binning on ndarray | |
| `POST /profile/line` / `box` | 12 | ★★ | | M | own bilinear sampler along the cut | |
| `POST /arith` | 13 | ★★ | ▼ | M | `evalexpr` + whitelisted fns over ndarray | Safer & simpler than Python eval-sandboxing |
| `POST /combine` | 13 | ★★★ | | M | rayon chunked reduce; streaming for many large inputs | Memory strategy is the work |
| `POST /align` (wcs_interp, bilinear) | 14 | ★★★★ | ▲ | M | own reprojection: target-grid → wcslib sky → source-pix → bilinear sample; rayon tiles | No `reproject` equivalent; bilinear first |
| `POST /align` (flux-conserving) | 14 | ★★★★★ | ▲ | L | pixel-overlap reprojection | Defer to Phase 4; bilinear covers most agent use |
| `POST /align` (star_match) | 14 | ★★★★★ | ▲ | L–M | asterism/triangle matching from scratch (astroalign port) | No crate exists; defer, demand-driven |
| `POST /astrometry/verify` / `refine` | 14 | ★★★ | | M | detect + Gaia TAP + own least-squares offset/rot/scale fit | |
| `POST /astrometry/solve` (blind) | 14 | ★★★ | ▼ | L | **subprocess `solve-field`** + index files in container | Coding is easy; ops (≈50 GB indexes) is the real cost |
| `POST /filter` | 15 | ★★ | ▲ | M–H | separable Gaussian/boxcar convolution, own f32 median filter | `imageproc` is u8/u16-centric; write for f32 |
| `POST /cosmicrays` | 15 | ★★★★ | ▲ | L–M | L.A.Cosmic port, or **sidecar (astroscrappy)** initially | Sidecar first, port on demand |
| `POST /mask` | 15 | ★★★ | | M | boolean ndarray + region resolver plumbing | Touches many endpoints |
| `POST /catalogs/query` | 16 | ★★★ | | H | `reqwest` TAP/ADQL (Gaia) + VizieR/SIMBAD; disk cone-cache | Retry/timeout/caching discipline |
| `POST /catalogs/upload` | 16 | ★ | | L | `csv` crate | |
| `POST /catalogs/crossmatch` | 16 | ★★ | | H | `kiddo` k-d tree on unit-sphere xyz | `best_symmetric` join logic |
| `POST /catalogs/{id}/photcal` | 16 | ★★★ | | M | sigma-clipped robust fit | |
| `POST /regions` (+ DS9 import/export) | 17 | ★★★★ | ▲ | M | `nom` DS9 grammar + region resolver | Parser written in-house |

### 20.2 Priority quadrants (Rust)

```
                          HIGH FREQUENCY
                               │
   [BUILD FIRST]               │        [BUILD FIRST — SENIOR ATTENTION]
   stats, histogram, pixel     │        render core (★★★★, port-and-verify)
   header, structure, cutout   │        SEP FFI cluster: background +
   sessions, pix2sky/sky2pix   │          detect + aperture photometry
   crossmatch, bin             │        catalogs/query (network discipline)
                               │        wcslib-sys bindings (enables 15+ endpoints)
 EASY ─────────────────────────┼─────────────────────────────────── HARD
   arith, upload, separation   │        psf photometry → SIDECAR
   curve_of_growth, profiles   │        star_match, flux-conserving align → DEFER
   filter, peaks               │        blind solve → SUBPROCESS
   [FILL IN AS NEEDED]         │        cosmicrays, ePSF → SIDECAR then port
                               │        [ESCAPE-HATCH, DON'T HAND-BUILD]
                          LOW FREQUENCY
```

The Rust-specific reading: the top-right quadrant now contains **two infrastructure investments** (wcslib bindings, SEP bindings) that don't map to single endpoints but unlock ~20 of them. The bottom-right quadrant's correct answer is almost never "write it in Rust now" — it's sidecar, subprocess, or defer.

---

## 21. Implementation Plan

Target: **single static-ish binary** (axum service) + container with cfitsio/wcslib/SEP shared libs, astrometry.net binaries, and the optional Python sidecar as a separate container. One engineer fluent in Rust *and* the astronomy stack; add ~30% if those are two people pairing.

### Phase 0 — Infrastructure & FFI foundations (Weeks 1–3; grew from 1 week in the Python plan)

The FFI layer is new critical-path work and is front-loaded deliberately.

- **`wcslib-sys` + safe `wcs-rs` wrapper**: bindgen, RAII around `wcsprm`, batch pix↔sky, SIP/TPV verified against the golden mosaic. *This blocks ~15 endpoints; do it first.*
- **`sep-sys` + safe wrapper**: bindgen over SEP; background, extract, sum_circle/ellipse/circann round-tripped against sep-python fixtures.
- **Service skeleton**: axum + tokio; `spawn_blocking`→rayon bridge for CPU work; session manager (DashMap, TTL eviction, per-session RwLock, memory budget); `image_ref` and catalog registries; shared **region resolver** (any region spec → array view + WCS) used by ~15 endpoints.
- **Error framework**: machine-readable codes, `hint` generation, `request_echo` middleware.
- **Oracle test harness**: Python CI job freezes astropy/photutils/sep reference outputs on 4 golden FITS files as JSON fixtures; Rust tests assert tolerance-bounded agreement. Perceptual-hash tests for PNGs.
- **Build/packaging**: Dockerfile linking cfitsio/wcslib/SEP; document that fully-static musl builds are not worth fighting for given the C dependencies — glibc + shared libs in a slim image is fine.

**Exit criteria:** round-trip pix↔sky on the SIP golden file matches astropy < 1 mas; SEP-rust detection catalog on the crowded field is row-identical to sep-python.

### Phase 1 — MVP: the look-and-measure loop (Weeks 4–6)
**Goal: Workflow A (§19) end-to-end. ~70% of call volume.**

- Sessions (open/status/keepalive/close/hdu), `structure`, `header`, `wcs`.
- **Render core**: zscale port (validated to 4 sig figs vs astropy), all stretches, colormap LUTs baked as const tables, rayon-tiled auto-binning, resolved-parameter echo, `clipped_fraction`; overlays limited to crosshair + scalebar.
- `stats` (bit-exact sigma-clip vs fixtures), `histogram` (+ plotters PNG), `pixel`.
- `cutout`, `bin`, `pix2sky`/`sky2pix` (static name table pre-Sesame).

**Exit criteria:** agent converges on good scaling in ≤ 5 renders on all golden files; warm 1k×1k render **< 100 ms** (Rust should beat the 500 ms Python target by ~5×; treat this as a regression budget, not a stretch goal); 20 concurrent sessions on the big mosaic stay inside the memory budget.

### Phase 2 — Detection, photometry, catalogs (Weeks 7–10)
**Goal: Workflow B. ~90% of call volume.**

- `background`/`subtract` and `detect` (segmentation) directly on the Phase-0 SEP wrapper; catalog paging/CSV; **ship `markers`/`apertures` overlays in the same milestone** — detection without visual verification is untrustworthy for an agent.
- `photometry/aperture` (SEP geometry + our gain/sky/flag error model, validated < 0.5% vs photutils fixtures), `curve_of_growth`.
- `psf/fit_single` (LM Gaussian/Moffat + triptych).
- `catalogs/query` (Gaia TAP + SIMBAD first, with disk cone-cache, timeouts, retry), `crossmatch` (kiddo), `upload`; Sesame name resolution in `sky2pix`.
- `filter`, `profile/line`/`box`.

**Exit criteria:** Workflow B reproducible; detection completeness/purity benchmarked on the crowded golden field; catalog queries degrade gracefully (cached results + explicit `stale: true` flag) when the network is down.

### Phase 3 — Multi-image & science depth (Weeks 11–15)
**Goal: Workflows C and D.**

- `arith` (evalexpr whitelist), `combine` (rayon chunked/streaming reduce + nmap).
- **Reprojection engine** (`align` wcs_interp, bilinear): target-grid → wcslib → source-pix → sample, rayon-tiled, behind the **async job pattern** (`202 {job_id}` + `GET /jobs/{id}`) for full-frame calls.
- `astrometry/verify`/`refine`; `psf/measure` seeing maps; `profile/radial`; `mask` + mask plumbing into stats/detect/photometry; `regions` with the nom DS9 parser; `contours`/`grid`/`text` overlays; `photcal`; `daofind`/`peaks`.
- **Stand up the Python sidecar** (one container: photutils PSF photometry + astroscrappy CR rejection behind the API's JSON contract) so `/photometry/psf` and `/cosmicrays` ship as endpoints now, in Rust later only if usage justifies.

**Exit criteria:** 3-exposure calibrate-align-stack matches a ccdproc reference reduction; seeing map within 5% of manual measurement; sidecar failure returns a clean `503 {error: "sidecar_unavailable"}`, never a hang.

### Phase 4 — Specialist & performance (Weeks 16+, demand-driven)

- `astrometry/solve` via `solve-field` subprocess (the work is index-file ops: ~50 GB, bake into a volume).
- `render/rgb` (Lupton port); remaining catalogs (PS1, SDSS, 2MASS, USNO, NED); color terms in `photcal`.
- Rust ports of sidecar functions **only if transcripts show volume**: CR rejection first (self-contained algorithm), PSF photometry last.
- `align` star_match and flux-conserving reprojection if difference-imaging use cases materialize.
- Performance pass: render tile cache keyed on (image_ref, region, binning), histogram sampling for full-frame calls, SIMD review of stretch/LUT hot loop.

### Cross-cutting guidance (Rust-specific)

- **Async discipline:** axum handlers must never run CPU-bound ndarray work on the tokio executor — everything numeric goes through `spawn_blocking` into a rayon pool sized to physical cores. This is the #1 latency footgun in a Rust image service.
- **FFI safety:** all cfitsio/wcslib/SEP calls live behind safe wrappers in their own crates with `unsafe` confined and Miri/valgrind runs in CI. wcslib's `wcsprm` is not thread-safe per instance — pool or clone per task.
- **dtype policy:** ingest any FITS dtype (incl. BSCALE/BZERO ints), promote to `f32` for analysis by default, `f64` opt-in per session for precision-sensitive photometry. Document it; astronomers will ask.
- **The oracle suite is the project's spine.** Every ported algorithm (zscale, stretches, sigma-clip, DAOFIND, Lupton, L.A.Cosmic) lands with its astropy-fixture test in the same PR. No fixture, no merge.
- **Ship measurement + its visualization in the same phase** (detect+markers, photometry+apertures overlay, background+quicklook) — unchanged from the language-agnostic plan, and still the most important product rule.
- **Escape hatches are features, not debt**, as long as they sit behind the stable JSON contract: agents cannot tell whether `/photometry/psf` is Rust or sidecar, which is exactly the point.

### Effort summary

| Phase | Duration | Endpoints | Cum. call-volume coverage | New vs Python plan |
|---|---|---|---|---|
| 0 | 3 wk | 0 (infra + FFI) | — | +2 wk (wcslib/SEP bindings, oracle harness) |
| 1 | 3 wk | 12 | ~70% | +1 wk (zscale/stretch/LUT ports) |
| 2 | 4 wk | ~14 | ~90% | +1 wk (photometry error model, TAP client from scratch) |
| 3 | 5 wk | ~13 | ~97% | +1 wk (reprojection engine, DS9 parser); sidecar absorbs PSF-phot/CR |
| 4 | demand-driven | ~6 | ~100% | solve = subprocess (cheaper); star_match (dearer) |

Roughly **15 weeks to a scientifically complete Rust service** — about 50% more than the Python estimate, with the premium concentrated in Phase 0 FFI work and algorithm ports. What you buy for it: ~5–10× faster renders on the hot loop, real multi-session concurrency without a GIL, a single deployable service with predictable memory, and compile-time safety around the session/image-ref state machine that a long-running agent-facing server sorely needs.

---

*End of document. Endpoint set: 40 endpoints across 16 functional groups. Rust stack: axum, tokio, rayon, ndarray, fitsio (cfitsio), wcslib-sys (in-house), sep-sys (in-house), image, plotters, levenberg-marquardt, kiddo, evalexpr, nom, reqwest — plus solve-field subprocess and an optional photutils/astroscrappy sidecar.*

## 22. Basing the Tool on AstroBurst: Feasibility Assessment

*Assessment date: 2026-07-04, against `samuelkriegerbonini-dev/AstroBurst` v0.5.6 (main). Method: source review of the full Rust backend, test-suite run, and a live smoke test of the headless server.*

### 22.1 What AstroBurst is

An open-source Rust/Tauri/WebGPU desktop application for processing JWST/HST/Roman imagery (FITS + ASDF), ~31k LOC of backend Rust, GPL-3.0-or-later, active development (last push the day before this assessment), CI, and a large backend test suite.

**Verified in this assessment:**
- **Test suite: 401/402 pass** on a fresh clone (Rust 1.89, `--features server`). The single failure is the ASDF compression reference suite, which requires the optional `asdf-full` (bzip2/lz4) feature that was deliberately excluded from the build — a feature-gating artifact, not a code-health problem.
- **Live smoke test of the agent loop passed**: start `astroburst-server` → `POST /sessions` → `POST /sessions/{sid}/fits/open` on a real HST WFPC2 frame (1600×1600, `RA---TAN` WCS) → `POST /image/stf` returned a full-frame auto-stretched PNG **with the resolved STF parameters echoed in an `x-stf-params` response header** → `POST /image/viewport {x,y,w,h}` returned a 256×256 stamp PNG. That is our §19 Workflow A skeleton, already running.

### 22.2 The decisive finding: it is already becoming this tool

Three things make AstroBurst far more than a random Rust astro codebase for our purposes:

1. **A headless axum server already exists** behind a `server` cargo feature: session registry (DashMap, TTL, per-session memory budget — the exact Phase-0 design in §21), async jobs with SSE progress streaming and cancellation, and endpoints for sessions, FITS open/header, render (manual MTF), auto-STF, viewport stamps, stacking, drizzle, and a declarative calibration pipeline.
2. **A Python agent client ships in-repo** (`agent/astroburst_client`, async httpx, typed models, examples) — the consumer side of our API is already prototyped.
3. **The repo's own `roadmap_to_agent.md`** targets precisely our end state: a pure headless backend driven programmatically by an AI agent, with the Tauri desktop app as one client among others. It also documents that `core` and `math` (the algorithm layers) contain **zero Tauri references** — the decoupling work is largely done; the remaining coupling sits in ~15 desktop command handlers and one progress emitter.

### 22.3 Capability map against our API spec

**Already there (core algorithms, and in several cases HTTP endpoints too):**

| Our spec section | AstroBurst status |
|---|---|
| §2 Sessions, jobs | ✅ Server endpoints exist (sessions, TTL, jobs + SSE + cancel) |
| §3 Header inspection | ✅ Endpoint exists; pure-Rust FITS reader (memmap2, BZERO/BSCALE/BLANK) + writer; **bonus: ASDF (JWST/Roman) support we hadn't planned** |
| §4 Render | 🟡 Partial: full-frame + viewport `{x,y,w,h}` stamp rendering, auto-STF with resolved-parameter echo, tile-pyramid renderer, GHS stretch. Missing: zscale/asinh/log/percentile scaling vocabulary, colormaps (L8 grayscale only), sky-coordinate regions, overlays, N-up orientation |
| §5 Stats/histogram | 🟡 Core functions exist (`compute_image_stats`, `compute_histogram`, sigma-clip in `math/`); no HTTP endpoints yet |
| §7 WCS | 🟡 Own implementation: **TAN and SIN projections + SIP distortion**, batch pix↔world; no endpoints yet; no TPV or other projections |
| §8 Background | 🟡 Gradient extraction, linked mode, sky neutralize, 1/f de-band — different flavor from Background2D but real |
| §9 Detection | 🟡 Star detection (sigma threshold, roundness, tile background) — star-centric; no segmentation/deblending for extended sources |
| §10 Photometry | 🟡 `measure_star`: centroid, net flux w/ annulus sky, instrumental mag, SNR, FWHM, optional **Gaia DR3 cross-match (VizieR cone search client exists)** — single-star interactive, not batch, no exact-overlap geometry or gain-based error model |
| §11 PSF | 🟡 PSF estimation module, subframe FWHM/eccentricity/SNR quality scoring |
| §13 Combine | ✅ Sigma-clip stacking, drizzle (multi-kernel), full bias/dark/flat calibration pipeline — **our Phase-3 stacking work, done and battle-tested** |
| §14 Alignment | 🟡 Phase correlation with Foroosh sub-pixel + star-based affine (reflection-rejecting), NaN-aware bicubic resampling; **plate solve via astrometry.net web service**. No WCS-based reprojection onto an arbitrary target grid |
| §15 Filtering | 🟡 Wavelet, deconvolution, star removal, resample; no plain Gaussian/median filter endpoint, no L.A.Cosmic |
| §16 Catalogs | 🟡 VizieR/Gaia client (used by SPCC color calibration); no general cone-search/crossmatch/photcal endpoints |

**Genuinely missing (our differentiators to build):** the render vocabulary agents need (zscale + manual vmin/vmax + colormaps + sky regions + overlays + `clipped_fraction`), stats/histogram/pixel/WCS/cutout *endpoints*, segmentation detection + batch exact-overlap aperture photometry (the SEP FFI plan stands), catalog registry/crossmatch/photcal, `arith`/masks/DS9 regions/profiles, WCS reprojection, cosmic rays.

**Gaps to verify or fix in the base itself:** no RICE/HCOMPRESS tile-compressed FITS support (no hits in the reader — ground-based `.fits.fz` archives won't open; plan a decompression shim or a cfitsio-backed fallback path), WCS limited to TAN/SIN+SIP (fine for HST/JWST stamps; DECam/ZTF-style TPV mosaics need work — extend in Rust or revisit `wcslib-sys` for exotic projections only).

### 22.4 License and governance

- **GPL-3.0-or-later** (not AGPL). Consequences: a fork stays GPL; if we distribute binaries, source must be offered; **running it as an internal or hosted service triggers no copyleft obligation** (no network clause). Acceptable for an open research tool; a blocker only if a proprietary distributable were the goal.
- CONTRIBUTING + **CLA** exist, CI runs typecheck + the backend tests. Risk: effectively a single primary author (bus factor) — mitigated by the fact that the headless-server contribution path is demonstrably open, and by keeping our fork mergeable.

### 22.5 Recommendation: yes — fork, upstream-first

Base the tool on AstroBurst rather than starting fresh. The overlap is not incidental: session/job/render server plumbing, pure-Rust FITS I/O proven on real HST/JWST data by ~400 tests, and the entire calibration/alignment/stacking layer are exactly the expensive parts of §21 Phases 0, 1, and 3. Two of our hardest Phase-0 line items dissolve: **no cfitsio FFI needed** (their reader replaces it, pending the compressed-FITS shim) and **no server skeleton to build**. The strategic fit is unusual — their roadmap wants to become what we are specifying, so much of our work is upstreamable, which keeps the fork thin and the maintenance burden shared.

**Structure:** fork; execute their own roadmap's workspace split (`astroburst-core` library crate — mechanical, per their analysis); build our agent API as an expanded server crate on top. Upstream general-purpose pieces (stats/WCS/cutout endpoints, render vocabulary, detection); keep agent-specific opinions (our JSON envelope, `request_echo`, catalog registry semantics) in the fork if upstream tastes differ. Sign the CLA early and open a coordination issue before large PRs.

### 22.6 Revised plan (deltas to §21)

| Phase | Was (greenfield Rust) | Now (AstroBurst fork) |
|---|---|---|
| 0 | 3 wk — FFI (cfitsio/wcslib/SEP), server skeleton, oracle harness | **1–1.5 wk** — fork + workspace split, oracle harness (still non-negotiable: validate their FITS/WCS/stats against astropy fixtures), our error envelope + `request_echo` middleware. cfitsio dropped; wcslib deferred pending projection needs; SEP kept for Phase 2 |
| 1 | 3 wk — render core + stats/WCS endpoints from scratch | **2 wk** — extend existing render path: zscale port, stretch vocabulary, colormap LUTs, sky regions, `clipped_fraction`, crosshair/scalebar overlays; add stats/histogram/pixel/cutout/pix2sky/sky2pix endpoints over existing core functions |
| 2 | 4 wk | **3–3.5 wk** — SEP FFI for segmentation + exact apertures (unchanged); catalog registry/crossmatch/photcal builds on their VizieR client; `psf/fit_single` builds on their PSF module |
| 3 | 5 wk | **2.5–3 wk** — stacking/calibration/drizzle already done; remaining: WCS reprojection (reuse their NaN-aware bicubic sampler), `arith`, masks, DS9 regions, profiles, `astrometry/verify` (plate solve exists), remaining overlays |
| 4 | demand-driven | unchanged (sidecar for PSF-fit photometry & CR rejection; `solve-field` local subprocess as an offline alternative to their web-service solver) |

**Net: ~15 weeks → ~9–10 weeks**, with calibration, drizzle, deconvolution, star removal, and ASDF support arriving for free. The oracle test harness remains the spine of the effort: adopting 31k LOC of someone else's numerics makes independent validation against the astropy reference *more* important, not less.


## 23. Adopting the CDS Rust Stack: fitsrs, wcs-rs, mapproj

*Assessment date: 2026-07-04, against fitsrs 0.4.1, wcs 0.4.2, mapproj 0.4.0 (crates.io releases). Method: source review plus an empirical test program compiled against the released crates, run on AstroBurst's sample HST WFPC2 frame, with astropy 7.x as the numerical oracle.*

### 23.1 What the stack is

Three crates from CDS Strasbourg, designed to work together (fitsrs re-exports wcs-rs; `HDU::wcs()` builds a WCS directly from a parsed header; wcs-rs delegates projection math to mapproj). They power **Aladin Lite v3**, i.e. they parse real survey data in production daily. All three are **MIT/Apache-2.0 dual-licensed** — permissive, and compatible with embedding in the GPL AstroBurst fork.

| Crate | Role | Notable scope |
|---|---|---|
| `fitsrs` | Pure-Rust FITS reader | Multi-HDU, images + bintables, gzip, async, streaming reads for files larger than memory, tiled-compression convention (GZIP/GZIP2/RICE declared). **No writer.** No HIERARCH, no ASCII tables, no H_COMPRESS/PLIO |
| `wcs` (wcs-rs) | FITS WCS keyword interpretation | CD/PC/CDELT handling, SIP distortion, `TryFrom<&fitsrs Header>` |
| `mapproj` | Projection math | **~23 projections**: full zenithal set (TAN, SIN, STG, ARC, ZEA, AZP, SZP, ZPN, AIR, NCP, FEYE), cylindrical (CAR, CEA, CYP, MER), pseudo-cylindrical (AIT, MOL, PAR, SFL), conic (COD, COE, COO, COP), HPX. No TPV/TNX/ZPX (true of every Rust option short of wcslib FFI) |

### 23.2 Empirical results

A Rust test program (fitsrs + wcs-rs) against the 1600×1600 HST WFPC2 frame, with astropy as oracle:

1. **Uncompressed pixel reads: bit-exact.** All probed pixels identical to astropy to full f32 precision.
2. **WCS: exact, with a convention to pin.** Raw pix→sky initially disagreed with astropy(origin=0) by a constant ~0.14″; re-running the oracle with origin=1 produced agreement **to all 8 printed decimals** on every test point. Conclusion: wcs-rs `ImgXY` uses the FITS 1-based convention; our API's 0-indexed pixels need a +1 shim at the boundary. Numerics are validated; this is exactly the class of silent bug the §21 oracle harness exists to catch.
3. **RICE tile-compression: currently broken on standard input.** On an astropy-generated `RICE_1` file (both quantized-float and lossless int16 variants): debug builds **panic with integer overflow** inside the ported cfitsio RICE decoder; release builds (where overflow wraps) run but return a **short read (2,457,600 of 2,560,000 pixels) with wrong values** (e.g. 6334 vs oracle 135). The README's own caveats gesture at limited testing here. Until this is fixed upstream, fitsrs does not actually close AstroBurst's `.fits.fz` gap, despite the feature being on the box.

### 23.3 Recommendation: split the verdict

**wcs-rs + mapproj: adopt now — clear win.** AstroBurst's in-house WCS supports two projections (TAN, SIN) + SIP; the CDS stack supports ~23 + SIP with numerics we just validated exactly against astropy, maintained by an institution rather than a side module of an imaging app. AstroBurst already funnels all WCS use through one internal type (`core/astrometry/wcs.rs::WcsTransform`), so this is a contained swap behind an existing seam: keep the `WcsTransform` API, back it with wcs-rs, add the 0-/1-based shim, and extend the oracle fixtures to cover a projection zoo (TAN, SIN, ZEA, AIT, CAR at minimum, ± SIP). Estimated effort ~1 week including tests. This removes the `wcslib-sys` question from the plan entirely except for TPV/TNX-distorted survey mosaics (DECam/ZTF style), which remain the one case that would force wcslib FFI or an in-Rust TPV extension — decide only if that data actually shows up.

**fitsrs: adopt selectively, not yet as the primary reader.** Three concrete blockers for "primary":
- **No writer** — cutout/save, derived-product export, and the stacking pipeline all need FITS output; AstroBurst's writer stays regardless.
- **Performance model mismatch** — fitsrs is an iterator/`Read`-based streaming parser; AstroBurst's reader is memmap + zero-copy slicing, which is what makes the <100 ms viewport budget on multi-GB mosaics plausible. Replacing memmap slices with a full-stream decode per region would be a regression on our hottest path.
- **The RICE defect above.**

What fitsrs should do for us *now*: (a) **bintable reading** — FITS catalogs (Gaia dumps, SExtractor outputs) for §16 endpoints, which AstroBurst cannot parse at all; (b) gzip and async ingestion paths. What it should do *after an upstream fix*: become the **compressed-FITS ingestion path** (decode `.fits.fz` once on open → hand the plane to the memmap-style in-memory pipeline). We have a minimal reproducer (astropy `CompImageHDU`, RICE_1, 1600×1600 f32/i16); filing it upstream — ideally with the `wrapping_add` patch and a fixture — is a cheap, high-goodwill contribution to a responsive team whose README explicitly invites use-case reports.

**Strategically, converging on the CDS stack is the right long-term direction**: it is the only institutionally-maintained Rust astronomy I/O ecosystem, it connects us to cds-healpix/MOC crates (future HiPS/coverage features), and every piece we upstream reduces our fork's surface. The sequencing that respects the evidence: **wcs-rs/mapproj in Phase 1** (folded into the render/WCS-endpoint work, net-neutral on schedule since it deletes in-house projection code), **fitsrs for bintables in Phase 2**, **fitsrs for compressed FITS when upstream is fixed** — with every swap gated by the oracle suite, which this assessment has now demonstrated catching both a convention mismatch and a real decoder bug on its first outing.

