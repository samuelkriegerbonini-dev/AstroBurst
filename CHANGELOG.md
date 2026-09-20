# Changelog
All notable changes to AstroBurst will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Phase 0+1 (viewer maturity)**
  - Display controls: scale modes (`mtf`, `linear`, `log`, `sqrt`, `asinh`, `power`), limit modes (`minmax`, `zscale`, `percentile`, `user`), 9 colormaps (`gray`, `viridis`, `inferno`, `magma`, `plasma`, `cividis`, `heat`, `cool`, `rainbow`) plus invert; the GPU path samples a 256x1 LUT texture and the worker fallback reproduces the same bytes; stretch, limits and the byte rule live once in `core/imaging/scale.rs`, shared by the desktop commands (`compute_scale_limits_cmd`, `get_colormap_lut_cmd`) and the server
  - Pixel readout with `BUNIT` (FITS) and ASDF `unit` mapping via `probe_pixel_cmd`, sharing `core/imaging/pixel_probe.rs` with the server `/v2/.../pixel` endpoint (value, unit, box min/max/mean/median, pixel and NaN counts)
  - Coordinate frames for the cursor readout: ICRS, FK5 (J2000), Galactic and Ecliptic (J2000), in sexagesimal or decimal format, via `core/astrometry/frames.rs` and the `frame` argument of `pixel_to_world_cmd`
  - CI: `backend-tests-unix` matrix (ubuntu/macos, `cargo test --all-features`) and `frontend-tests` (vitest) jobs; `vitest` added with `pnpm test` / `pnpm test:watch` and smoke tests for `pixelMapping`, `filterWavelengths` and the binary IPC parsers
- **Phase 2 (planes, DQ)**
  - Image references: every path-based command accepts `<path>#hdu=<n>` (any FITS HDU) or `<path>#array=<key>` (any ASDF array, dotted keys such as `roman.dq`); the ref is the cache key and output files derive their stem from it (`jw01234_cal.fits#hdu=3` -> `jw01234_cal_hdu3.png`, `r0000.asdf#array=roman.dq` -> `r0000_roman_dq.png`); parsing lives in `types/image_ref.rs`, one loader (`infra/image_source.rs`) serves the desktop commands and the server
  - Lossless integer planes: BITPIX 8/16/32 HDUs and int8..int32/uint8..uint32/bool8 ASDF arrays decode bit-exactly (`IntPlane`, BZERO 32768/2147483648 handled without float rounding) alongside the float view; cached next to the pixels and counted in the cache budget
  - DQ flag tables: JWST (32 bits), HST (15 bits), Roman and unknown instruments use the JWST convention; table selected from `TELESCOP`/`INSTRUME` and the ASDF `meta.telescope`/`meta.instrument.name` cards; `get_dq_flag_table_cmd` returns the flags, default overlay mask and exclusion mask
  - Companion readout: DQ/ERR planes resolved by `EXTNAME`+`EXTVER` (FITS) or sibling array keys (ASDF); `probe_pixel_cmd` and the server `/v2/.../pixel` add `dq` (`bits`, `value`, `names`, `table`, `text` such as `3: DO_NOT_USE | SATURATED`) and `err` (`value`, `unit`)
  - DQ overlay: `get_dq_mask_preview` returns a binary OR-reduced mask grid (16-byte LE header: width, height, mask, table id) on the same grid as `get_raw_pixels_preview`; raw previews and mask previews share `preview_dims`/`cell_range`
  - DQ-masked statistics: `compute_histogram(excludeDq)` and `measure_photometry_cmd(excludeDq)` treat `DO_NOT_USE` (JWST convention) or the documented HST bad-pixel bits as NaN and report `masked` / `dq_excluded`
  - Server: `open` and `hdu` accept `array` (exactly one of `hdu`/`array`); image metadata and responses gain `array`, `plane_ref` and `is_dq`
- **Phase 3 (regions)**
  - Interactive regions in the viewer: circle, ellipse, box, annulus, polygon, line and point, drawn on a dedicated overlay layer with selection, move and resize handles, rotation for box and ellipse, `Delete` to remove and `Escape` to cancel; the tool returns to select after a shape is committed
  - DS9 region files: `regions_import_cmd` / `regions_export_cmd` read and write `.reg` in the `image`, `icrs` and `fk5` systems (degrees or sexagesimal, radii in image pixels or arcsec/arcmin/degree, `global`/local properties, comments); sky regions convert through the WCS
  - Per-region statistics (`region_stats_cmd`): count, sum, mean, median, MAD, sigma, min, max, sigma-clipped mean and median, plus background annulus subtraction with net sum and SNR; DQ-excluded pixels are reported separately
  - `radial_profile_cmd` (mean, median, standard deviation and cumulative sum per integer radius, optional background annulus) and `line_cut_cmd` (bilinear samples along a line) with canvas plots in the Analysis tab
  - Region geometry lives once in `core/imaging/region.rs` (DS9 convention: a pixel belongs to a region when its centre is inside; a circle of radius 5 at an integer centre covers 81 pixels); the server `v2` region, stats and cutout endpoints use the same shapes
  - Regions persist per file in the browser store and are listed, edited and exported from the Analysis tab
- **Phase 4 (processing parity)**
  - Pixel rejection for stacking (`rejection`): sigma clipping, Winsorized sigma clipping, linear fit clipping, percentile clipping, min/max or none, with mean, median, min or max combination (`combine`), selectable in the Stack tab, the compose wizard, the calibration pipeline and the headless server (`POST /sessions/:sid/stacking/stack`, `POST /sessions/:sid/pipeline/run`); unknown names return 400
  - Frame normalization before integration (`normalization`: none, additive, multiplicative, additive with scaling, multiplicative with scaling) computed on the pixels finite in every frame, scale+offset rejection normalization, and optional low/high rejection maps written as `<name>_rejection_low.fits` / `<name>_rejection_high.fits`; the rejection kernel lives once in `core/stacking/combine.rs` and the batch pipeline uses it (per-frame rejection counts kept)
  - Master bias, dark and flat frames are integrated with the same kernel (`MasterConfig`, Winsorized sigma clipping 4/3 and averaging by default, flats scaled to the first frame's median) instead of a plain per-pixel median
  - Cosmetic Correction (Stacking tab and calibration pipeline, desktop and server): hot/cold pixels from a master dark with separate sigma thresholds, auto-detection against the local median with star-core protection, PixInsight-style defect lists (`Point x y`, `Col x [y0 y1]`, `Row y [x0 x1]`, 0-based) validated line by line, CFA-aware replacement by the median or mean of unflagged neighbours blended by an amount, batch mode over loaded lights, flagged/replaced counts and a warning when the frame carries a DQ plane; in the pipeline the step runs after dark subtraction and before flat division
  - Background extraction gains a Spline (DBE) model: a regularised thin-plate spline through box samples with automatic grid placement, Point regions as manual samples, star and outlier rejection, adjustable smoothing, subtract/divide modes and median-preserving normalization; writes `<name>_dbe_corrected.fits` and `<name>_dbe_model.fits` with the source WCS/metadata and an `ABPROC` provenance card, and overlays the sample boxes on the preview
  - PixelMath: an expression language evaluated per pixel over the target image (`$T`) and named image slots, with arithmetic, comparison and logical operators, `iif`, `~` inversion, per-pixel functions (abs, sqrt, exp, ln, log, log2, pow, min, max, floor, ceil, round, trunc, sign, clip, rescale) and image statistics (mean, med, mdev, sdev, adev, min, max) over finite pixels; NaN-preserving semantics, optional truncate/rescale to [0, 1], output to a new `<name>_pixelmath.fits` with `ABPROC`/`PMEXPR` provenance cards; the panel validates the expression live with a caret on the failing token and binds slots to loaded files
  - Local Histogram Equalization (CLAHE with circular or square kernel, contrast limit, 8/10/12-bit histograms, blend amount) and HDR Multiscale Transform (2-8 wavelet layers, iterations, overdrive, inverted mode, lightness or per-channel mode, star-core deringing) for stretched mono images and the RGB composite, applied on luminance so hue is preserved; HDRMT refuses linear input and explains that a stretch must come first; both report progress, can be cancelled and write FITS with an `ABPROC` card
  - Wavelet noise reduction gains a per-scale detail bias (MLT-style, -1 to +3); the à-trous decomposition (`atrous_decompose`, `atrous_reconstruct`, `atrous_reconstruct_with_bias`) and the noise estimators (`noise_sigma_mad`, iterative `k_sigma_noise`) are public for other multiscale tools
  - Statistics panel (Analysis tab) with an exact table (count, count %, mean, median, avgDev, MAD, sqrt(BWMV), stdDev, variance, min, max, sum, NaN, excluded) in raw, [0, 1] or 16-bit units, per R/G/B channel for composites, with selected-region and DQ-exclusion options and copy-as-CSV; zero and negative pixels are included and no histogram approximation is used
  - Noise evaluation: k-sigma multiresolution noise in the Statistics panel, `noise: true` on `POST /v2/sessions/:sid/stats`, `evaluate_noise_batch_cmd` for frame lists, a Noise column in the Subframe Selector, and a "Weight frames by noise (1/sigma^2)" toggle in the Stack tab that multiplies with subframe weights when present
- **Phase 5 (science analysis)**
  - Photometric calibration read from the header (`core/metadata/photcal.rs`): JWST `MJy/sr` (with `PIXAR_SR`, or a WCS-derived pixel area flagged as derived) and `DN/s` via `PHOTMJSR`, HST `PHOTFLAM`/`PHOTPLAM`/`PHOTZPT` (STmag and ABmag, `ELECTRONS` divided by `EXPTIME`), Roman `conversion_megajanskys`/`pixelarea_steradians`, and generic `MAGZERO`/`MAGZPT`/`ZPT`/`PHOTZP`/`ZEROPT` zero points; the Photometry panel reports AB magnitude with error, flux in Jy, surface brightness and the convention used
  - Aperture photometry with fractional edge-pixel weights (shared `core/analysis/aperture.rs`, 5x5 sub-pixels by default), ERR-plane error propagation (or sky noise plus an optional Poisson term), the SATURATED DQ bit, masked-pixel counts, a curve-of-growth aperture correction with the total magnitude, and a warning on data carrying an `ABPROC` provenance card; region statistics on images with an ERR plane report the summed error and the inverse-variance weighted mean
  - Full FITS spectral axis (`core/astrometry/spectral.rs`): WAVE/AWAV/FREQ/VRAD/VOPT/VELO/ZOPT kinds, `CDELT3`, `CD3_3` or `PC3_3` steps, lenient `CUNIT3` spellings with FITS default units, `RESTWAV`/`RESTFRQ`, `SPECSYS`/`VELOSYS`, explicit errors for `-LOG`/`-TAB` axes; MUSE `CD3_3` and ALMA FREQ cubes now get a correct axis
  - Air/vacuum wavelength conversion (Greisen et al. 2006, FITS Paper III eq. 65), optical/radio/relativistic velocity axes from a rest wavelength, and a barycentric/heliocentric radial-velocity correction for ground-based headers (site from `OBSGEO-X/Y/Z`, `SITELAT`/`SITELONG` or `LAT-OBS`/`LONG-OBS`, mid-exposure from `MJD-AVG`, `EXPMID`, `DATE-AVG` or `DATE-OBS`+`EXPTIME`/2, Meeus low-precision ephemeris, about 0.02 km/s); spectra already in BARYCENT/HELIOCEN/LSRK/LSRD/GALACTOC/LOCALGRP/CMBDIPOL/SOURCE frames report a zero shift, GEOCENTR skips the diurnal term, spacecraft headers use `VELOSYS` or refuse with the reason; FITS date parsing, JD/MJD and GMST helpers (`core/astrometry/time.rs`); spectral axis controls (vacuum/air wavelength, frequency, velocity with correction) in the Spectroscopy panel, backed by `spectral_axis_cmd`, `velocity_axis_cmd` and `radial_velocity_correction_cmd`
  - Spectral-cube science: region (aperture) spectra over circle, box, ellipse or polygon regions with optional annulus sky subtraction, in native units and in Jy for `MJy/sr` cubes (`get_cube_spectrum_region_cmd`); channel-range collapse (sum, mean, median) selected with a range brush and saved as a 2D FITS with the spectral axis removed (`collapse_cube_range_cmd`); moment maps M0, M1 and M2 with a per-pixel continuum fit from two side windows, SNR masking and a velocity axis from the rest wavelength and convention, written as three 2D FITS with units (`moment_maps_cmd`); `get_cube_info` returns the parsed spectral axis
  - Science cutout export (Export > Cutout): a multi-extension FITS (SCI, ERR, DQ) from the selected box region or a manual centre/size in pixels or arcseconds, with `CRPIX` shifted and `LTV1`/`LTV2`/`LTM1_1`/`LTM2_2` recorded, NaN padding outside the parent, padded DQ pixels flagged `DO_NOT_USE | NON_SCIENCE` (HST: `REPLACED_FILL | MASKED`), a synthetic DQ when the file has none, observation provenance kept in the primary header and a per-plane memory budget; the cutout core (`core/imaging/cutout.rs`) is shared with the server endpoint, and the FITS writer can write plain multi-HDU IMAGE files from float32, int32 and uint32 arrays (`write_mef_images`)
  - WCS coordinate grid overlay (ICRS, FK5, galactic or ecliptic) toggled from the display bar with frame and density choices: iso-lines traced in world coordinates through the WCS, sexagesimal-friendly steps, RA 0h wrap and polar fields handled, edge labels that never overlap; `grid_lines_cmd` and `POST /v2/sessions/:sid/wcs/grid` return the polylines, labels and steps; a non-persisted overlay-layer store (`src/utils/overlayStore.ts`, `OverlayLayer.tsx`) shared by the CPU and GPU viewers
  - Gaia DR3 catalog panel: VizieR cone search around the image centre with proper motions propagated from J2016.0 to the observation date, rows placed on the image with an overlay layer and labels, full-field cross-match of detected stars with astrometric residuals (median dRA/dDec, rms) and a photometric zero point in G, BP or RP with an optional colour term (3-sigma clipped, informational when the header already carries a flux calibration), RFC-4180 CSV export of catalog rows, measured sources and matches, and one-click copy of rows into Point regions for `.reg` export; a 16-entry in-memory response cache shared with SPCC
- **Search Everywhere** command palette (`Ctrl+K` / double `Shift`, IntelliJ-style): fuzzy search over processed files, tool panels (Headers/Analysis/Processing/Stacking/Synth/Export/Settings), and actions (open files/folder, toggle panels, ZIP export, new batch); full keyboard navigation, status-bar entry point
- Right tool panel is now resizable (280–640px); all splitter positions (sidebar, right panel, compose panel height) persist across sessions via `localStorage` (`src/utils/layout.ts`)
- IntelliJ-style splitters: zero-width in layout with a 7px grab area, accent highlight on hover (delayed) and while dragging, double-click resets to the default size
- Tool windows (Files sidebar, right tool panel, Compose bottom panel) animate open/close (200ms slide, content anchored to its edge); hidden panels are `inert` and unmount after the transition; tool switches cross-fade
- `prefers-reduced-motion` support: all animations/transitions collapse to instants
- Flatpak/Flathub submission (PR pending approval)
- Export panel accessible from PreviewPanel bottom strip (Download icon, lazy-loaded ExportTab)
- WCS engine replaced with the [wcs](https://github.com/cds-astro/wcs-rs) crate (wcs-rs + mapproj), expanding projection support from TAN/SIN/ARC/CAR to ~20 FITS projections (adds STG, ZEA, ZPN, AIR, AZP, SZP, CYP, CEA, MER, SFL, PAR, MOL, AIT, conic COP/COD/COE/COO, HPX) and proper CD/PC/CDELT matrix handling; `WcsTransform`'s public API is unchanged, SIP distortion is still applied by AstroBurst's own math (see Fixed)
- New `pixel_to_world_cmd` Tauri command backing the cursor RA/Dec readout, replacing a duplicated client-side pixel<->sky implementation (`src/utils/wcstransform.ts`, now removed) with the same wcs-rs-backed engine used everywhere else

### Fixed
- Regions were invisible in the GPU viewer: the overlay measured its host container in a layout effect that runs before React attaches the parent ref, so the canvas backing store stayed 1x1 while still consuming pointer events (toolbar live, nothing drawn, nothing selectable). The host is now resolved from the canvas itself, so the layer is sized and observed on the first commit
- Regions were misplaced in the tile viewer: the rendered preview size was never reset when the displayed image changed, so a region could be mapped with the previous file's raster width against the new file's FITS dimensions, and the in-progress draft was global instead of per file, painting one image's shape over another
- Drawing a region left the drawing tool active, so clicking the new selection handles started another shape instead of selecting; left-drag panning is no longer swallowed when the gesture does not hit a region
- WCS headers using the `PC` matrix convention (no `CD` keywords, e.g. ASDF/Roman-derived FITS) were silently mis-transformed: the old hand-rolled WCS math only ever read `CD1_1..CD2_2` or fell back to `CDELT`+`CROTA2`, ignoring `PC` entirely
- Cross-checked against `mapproj` 0.4.0's own SIP polynomial evaluator, which has a confirmed bug (advances polynomial powers by repeated squaring instead of by degree) that silently produces wildly wrong sky coordinates for any SIP-distorted header; AstroBurst's WCS wrapper strips the `-SIP` CTYPE suffix before constructing the wcs-rs engine so its broken SIP path never runs, and applies/inverts SIP with its own (previously existing, still correct) math instead

#### Export Pipeline (3 critical fixes)
- `export_rgb_png` cache hit branch applied auto-STF even when `applyStfStretch = false`, corrupting linear exports; now renders raw linear data directly via `render_rgb` / `render_rgb_16bit`
- `"__composite__"` sentinel string was sent as a real file path to the backend, causing silent failures or crashes on cache miss; frontend now sends `null` paths for composite mode
- FITS composite export dropped WCS headers (CRPIX, CRVAL, CD matrix) when channel paths resolved to null; backend now resolves header from the first available source path in cache

#### Star Detection
- `detect_stars` NaN guard: `v.is_finite() && v > 1e-7` protects against zero-padded alignment borders
- Early return for degenerate images (`rows < 3 || cols < 3`)
- `peak_val.max(v)` replaces manual comparison
- Sort comparator uses `unwrap_or(std::cmp::Ordering::Equal)` to prevent NaN panic

#### Subframe Selector
- `SubframeSelectorPanel` receives `string[]` (file paths) instead of `ProcessedFile[]`, fixing TypeScript type mismatch

#### ComposeWizard State
- `calibrate_and_scnr_cmd` and `reset_wb_cmd` now render preview with auto-STF via `render_rgb_preview_with_stf`, eliminating washed-out/near-black previews after WB operations
- `apply_tone_composite_cmd` reads from `COMPOSITE_KEY` and renders preview only (no cache write), preserving linear calibrated data

### Changed
- The default stacking sigma-low threshold is 4.0 (was 3.0) to match PixInsight; sigma-high stays 3.0; the default output normalization is additive with scaling and the default rejection normalization is scale+offset (the stacking result is in the reference frame's units)
- Subframe quality weights from the Subframes tab now reach the Stack tab (`StackingPanel` never passed them before)
- Server `stats` responses gain `avg_dev`, `bwmv_sqrt`, `std_dev`, `count_fraction` and `nan_count`, computed on finite pixels only; the pipeline `run` accept body gains `warnings`
- Star photometry integrates the aperture with fractional edge-pixel weights instead of the centre-in-circle rule; `StarPhotometry.snr` is `net_flux / flux_err` under the new error model and `aperture_pixels` counts every pixel with a non-zero weight; the background annulus inner edge is exclusive
- `measure_photometry_cmd` gains an optional `gain` argument and returns `photcal` and `warnings`; region statistics accept the ERR companion automatically
- The server cutout response gains `ltv1`/`ltv2`; `build_wavelength_axis` now returns no axis for non-linear or unparseable spectral axes instead of a wrong linear one
- The SPCC VizieR parser maps columns by name through the shared Gaia TSV parser, tolerating missing optional columns
- `process_fits` / `process_fits_full` responses gain `image_ref` (canonical ref) and `plane` (`kind`, `index`, `key`, `extname`, `extver`, `is_dq`, `is_err`, `dq_ref`, `err_ref`, `source_path`, `dq_table`)
- `get_fits_extensions` rows gain `extver`, `ref`, `kind` (`hdu`/`array`), `is_dq`, `is_err` and also list ASDF arrays; `get_header_by_hdu` rejects ASDF files ("ASDF arrays have no HDU index")
- `compute_scale_limits_cmd` rejects percentile `low >= high`, percentiles outside `0..=100`, non-finite values and user `vmin >= vmax` instead of silently producing an empty range
- Server `render` accepts the `user` scale algorithm (`manual` kept as an alias, echoed back as `user`) and the new colormaps (`inferno`, `magma`, `plasma`, `cividis`, `heat`, `cool`, `rainbow`); stretch and colormap math now imported from `core/` instead of a server-local copy
- Server `pixel` response gains a top-level `unit` (from `BUNIT`) and `neighborhood.median`
- Server `pix2sky` accepts an optional `frame` (`icrs`, `fk5`, `galactic`, `ecliptic`) and echoes it in the response
- **IntelliJ-style neutral chrome**: structural borders, separators, panel headers, scrollbars and progress tracks moved from teal-tinted literals to neutral semantic tokens (`--ab-border`, `--ab-bg-hover`, `--ab-bg-active`, `--ab-text-1..4`); the teal accent is now reserved for interactive states (active tool, selection, focus, actions)
- Flat headers: preview/app header gradients replaced with flat panel surfaces; strip buttons use a neutral active background with accent-colored icon/label
- ExportStep detects STF identity (`Math.abs(midtone - 0.5) > 1e-4`) before composite PNG export; sends `applyStfStretch: false` when identity, activating backend auto-stretch

## [0.5.8] - 2026-09-11

Code audit of the math, GPU flow, image-processing and performance paths (21 module groups, every finding reproduced by hand before being fixed), plus a validation of the ASDF reader against the ASDF Standard 1.5/1.6.

### Fixed

#### Math / processing core
- Drizzle Lanczos3 kernel was evaluated in output-pixel units, so at scale 3 (and any integer scale with zero offsets) 8/9 of the output pixels received zero weight and rendered as holes; the kernel argument and support now scale with the output grid
- Sigma-clip combine and the batch pipeline stack used a 1e-10 sigma floor when the MAD is 0 (more than half the samples tied), rejecting every value that differed from the median; they now fall back to the mean absolute deviation and skip rejection when the scale is truly zero
- Star removal applied the soft star mask twice (push-pull fill already blends by the mask), leaving `m - m^2` of the star flux in the starless image at every mask edge
- GHS stretch with D = 0 returned the raw, un-normalized data instead of the normalized identity, producing a white preview and export after the composite was cached
- Wavelet reconstruction clamped negative pixels to 0 and rewrote NaN as 0
- Phase-correlation refinement discarded a confident coarse shift whenever the fixed centre crop was featureless (result fell back to identity/noise); the coarse estimate is now kept when the refine pass has no signal
- Star detection deblended every saturated flat-top star into several detections; equal-valued adjacent maxima now collapse to a single seed
- SPCC aborted the app (index out of bounds) when the R/G/B channel images differed in size; it now returns an error
- Cube frame previews rendered every pixel at or below the median as pure black; the asinh normalization now maps the full stretch into (0, 1]
- Viewer zoom-to-level computed the anchor translation with the unclamped scale, so the image drifted at the zoom limits

#### GPU / preview flow
- IPC preview extrema were computed over all finite pixels while the auto-STF parameters came from padding-aware statistics, so the GPU/CPU-worker render and the canonical PNG disagreed; both now use the same validity rule
- NaN/invalid pixels were rewritten to 0.0 before upload and rendered gray instead of black on the GPU and in the worker
- Linked STF on the GPU normalized each channel by its own range while the Rust linked STF uses the combined range; the uniform buffer now carries the combined range when the three channel STFs are identical
- Display-referred (toned/stretched) RGB previews were re-normalized per channel by their min/max instead of clamped to [0, 1]
- A failed WebGPU init (no adapter, device creation error) was cached for the whole session; the retry path now re-probes
- `render_rgb_preview` fell back to a PNG with default compression for any image up to 4096 px on the interactive restretch path
- Plate-solve star/annotation overlay was scaled to the container box instead of the letterboxed image
- Mono tile pyramid applied a second percentile stretch on top of the auto-STF
- Linear grayscale exports zeroed every pixel at or below 1e-7 while the min/max included negatives

#### Image I/O
- Quantized GZIP_1/GZIP_2 tiles (integer payload) were decoded as raw f32 bit patterns; the integer width is now inferred from the tile byte count
- ZBITPIX ±64 was rejected for every codec, including GZIP_2 files written by AstroBurst itself
- Rice decoder indexed the compressed stream unchecked and panicked on short or corrupt tiles; it now returns an error
- Constant tiles quantized with scale 1 under SUBTRACTIVE_DITHER_1 gained ±0.5 noise on decode; constant rows are now stored losslessly
- Header cards with keys longer than 8 characters (ASDF metadata) were truncated into colliding, malformed FITS cards on export; non-FITS keys are skipped
- RGB export ignored the caller's channel paths whenever a composite was cached; wizard cache keys now count as real channel sources, so per-channel ZIP exports no longer return the composite three times
- SPCC channel lookup in the colour-balance step always resolved the narrowband bins, never R/G/B

#### ASDF reader (validated against the ASDF Standard)
- Blocks are now memory-mapped and decompressed lazily: only the block referenced by the selected array is decoded (Roman L2 files carry six or more full-frame arrays), and uncompressed contiguous arrays are borrowed instead of copied three times
- `#ASDF BLOCK INDEX` trailer, CRLF line endings, space padding after the tree, `header_size` larger than 48 and `allocated_size` padding are handled per spec
- STREAMED blocks (flag 0x1) and `shape: ['*', …]` are supported
- Negative strides (flipped views) were silently ignored and the array read as contiguous
- External (exploded) block references fail with an explicit error instead of silently loading no image; inline `data:` arrays are decoded
- gWCS chains under `roman.meta.wcs` are resolved; the gWCS `Shift` offset is converted from 0-based gwcs pixels to 1-based FITS CRPIX
- YAML-tagged metadata subtrees (most Roman/JWST `meta` entries) are flattened into the header instead of being dropped; header card order is deterministic
- An ASDF without an image array now reports an error so the companion-FITS fallback runs
- Allocation sizes from untrusted block headers and shapes are bounded

#### Headless server
- Region, histogram, cutout and pixel endpoints validated their inputs after arithmetic that could wrap or allocate unbounded memory; bins, cutout size and box parity are now checked up front
- Statistics and histograms silently discarded every pixel ≤ 1e-7, giving wrong results for bias-subtracted or difference images
- Binning by a non-divisible factor averaged overlapping windows; it now crops to whole blocks
- Viewport auto-STF derived from crop statistics was applied with global normalization, rendering the crop black or saturated
- Re-opening an image under an existing `name` returned the stale cached array while overwriting its metadata
- Per-session LRU eviction is now reflected in `meta`, `list_images` and `active_ref`
- Job cancellation is observed by workers before results are published and terminal state transitions are atomic, so a late worker cannot overwrite `cancelled` (the compute slot is still held until the running stage finishes)
- Render path stretch/clip counting runs in parallel

### Changed
- Batch stacking pipeline loads and stacks one channel at a time and returns 2048 px previews instead of full-resolution base64 masters
- Drizzle scatters contributions directly into per-band accumulators in parallel instead of materialising every contribution as a tuple; frames that need no cropping are borrowed instead of cloned
- Cube global statistics sample at most 32 frames with a pixel stride bounded to ~8M samples
- Composite RGB preview from the calibration pipeline uses a shared robust stretch instead of a per-channel min/max
- Wizard cache entries are cleared when alignment re-runs, so pinned `__wizard_ch_` entries no longer bypass the byte budget
- Version 0.5.8

## [0.5.5] - 2026-06-26

### Added
- **WebGPU RGB preview**: GPU-only `GpuRgbRenderer` renders full RGB composites from three `r32float` textures with per-channel STF in the WGSL shader (MTF identical to the mono shader, CPU worker, and Rust backend). Routed through PreviewTab/PreviewPanel; falls back to the PNG composite when WebGPU is unavailable, no adapter is found, or the device is lost.
- **Live per-channel RGB STF** (`RgbStfPanel`, Analysis tab): per-channel shadow/midtone restretch the GPU composite live with no backend round-trip; the mono STF slider skips its backend render while a GPU preview is active.
- Backend command `get_raw_rgb_pixels_preview(path?, max_dim)`: linear 3-channel float preview with per-channel min/max, from a native RGB FITS or the composite cache (planar R|G|B with a 32-byte header). Shared `encode_channel_preview` factored out of the mono encoder (byte-identical).
- **Canonical filter→wavelength table** (`FILTER_WAVELENGTHS_NM` + `filter_to_wavelength_nm` in `types/constants.rs`, mirrored by `utils/filterWavelengths.ts`): normalizes CLEAR / separators / dual filters, replacing the two duplicated JWST tables.
- **Dynamic channel Auto-Map** (Compose ▸ Channels): each non-narrowband/non-RGB filter gets its own wavelength-labelled channel (e.g. NIRCam F090W–F444W → five distinct channels), grouped by nm.
- **Auto (λ) blend**: spreads any number of channels across R/G/B by wavelength; auto-applies once for untouched broadband stacks, never overriding a palette or manual weights.

### Changed
- Compose tool moved from the right tool strip to the left strip.
- Preview downsample replaced nearest-neighbor with a contrast-weighted hybrid (mean↔max) area kernel that preserves faint point sources while anti-aliasing the background.
- `parseRawPixelBuffer` accepts an `ArrayBufferView`, validates length, and stays zero-copy in the normal case.
- The GPU/CPU toggle surfaces the fallback reason (no WebGPU / no adapter / init failed / device lost / GPU error) via tooltip.

### Fixed
- **STF GPU/CPU parity**: the CPU fallback worker no longer hard-zeros `raw ≤ 1e-7` (was blacking out background-subtracted pixels the GPU shader maps to gray); epsilons aligned to 1e-8 so GPU and CPU render identically.
- **Preview normalization**: `data_min`/`data_max` now describe the full image (shared `scan_extrema`) instead of the nearest-neighbor decimated subset, so the live preview matches the canonical PNG.
- **File switch in GPU mode**: `loadRawPixels` gained a force flag so switching files no longer leaves stale previous-file pixels blocking the fetch.
- **Device loss / uncaptured errors**: `GpuSingleton` exposes `onGpuLost` + an uncaptured-error handler; `GpuRenderer` drops the dead device's resources and falls back to the CPU worker instead of a permanently blank canvas.
- **Texture-limit guard**: an oversized image degrades to the CPU worker instead of an unobserved validation error and a blank canvas.
- **Uniforms**: reused scratch buffer + skip-when-unchanged guard (no per-frame allocation; cache invalidated on uniformBuffer recreation).
- Fixed OIII / Hα / SII filter-regex false positives.

### Removed
- Orphaned ZNCC WebGPU shader (`zncc_align.wgsl`) and its CONTRIBUTING reference; the unreachable client-side downsample worker; unused imports.

## [0.4.6] - 2026-04-06

### Added

#### Tone Curves (AdjustStep)
- Spline-based curve editor with per-channel (R/G/B) and linked RGB modes
- Double-click to add control points, right-click to remove
- Non-destructive: reads linear data from COMPOSITE_KEY, applies STF + curves for preview only
- Curves state carried from StretchStep via CompositeContext

#### Auto-STF Preview for Linear Data
- All linear-domain commands (blend, calibrate, WB, SCNR, reset) render preview with linked auto-STF
- Uses existing `make_stf_u8_fn` + `render_rgb_preview_with_stf`
- Eliminates washed-out/near-black previews after blend and color balance

#### Narrowband Detection via Blend Preset
- `isNarrowbandWorkflow` checks blend preset (SHO, HOO, Foraxx) as fallback when bin IDs are r/g/b
- SCNR warning badge correctly shows NARROWBAND for SHO data in RGB bins

#### Auto-STF Propagation
- `blend_channels_cmd`, `calibrate_and_scnr_cmd`, `calibrate_composite_cmd`, `reset_wb_cmd` return `auto_stf` in response
- StretchStep sliders initialize from auto-STF instead of identity
- ColorBalanceStep propagates post-WB auto-STF to CompositeContext
- StretchStep re-initializes on re-blend via ref-based comparison

#### Cache-Only Intermediate Processing
- `align_channels_cmd`, `stack`, `calibrate`, `extract_background_cmd` store results in GLOBAL_IMAGE_CACHE
- Only PNG previews written to disk; FITS output only on explicit export

### Changed

#### Linear Pipeline Preservation
- `apply_tone_composite_cmd` no longer writes to COMPOSITE_KEY cache
- STF + curves are preview-only; linear calibrated data preserved

#### Safe StretchStep Reset
- Reset restores STF sliders to auto-computed values without touching cache
- No longer calls `resetWb` (which destroyed WB+SCNR data)

#### AdjustStep STF Passthrough
- Reads compositeStf from CompositeContext instead of using auto-STF
- Custom stretch from StretchStep preserved in curves preview

#### ExportStep
- PNG export errors on cache miss instead of silent fallback to raw files
- FITS export passes header source path only (data from cache)

#### Asset Protocol
- Path safety with canonicalize + starts_with against app_data_dir
- Windows URL decode compatibility
- Async runtime, NotFound-only retry, selective cleanup, CORS restored

### Fixed
- ColorBalanceStep auto-WB infinite loop (removed onWbChange from useEffect deps)
- CubeFrameNav service return type: `output_path` (was `png_path`)
- CubeDims type: `width/height/frames` matching backend constants
- SpectroscopyPanel, ExportTab cubeDims usage aligned with backend
- PipelineResult type matches BatchPipelineStats from Rust
- StackOptions: added `align`, `maxIterations`; CalibrateOptions: added plural paths, `darkExposureRatio`
- BackgroundResult: added `corrected_fits`; ExportResult: added `channels`
- PlateSolveResult: added `success`; resetWb return type fixed
- PreviewContext duplicate export removed
- 22 missing lucide-react declarations added
- ComposeWizard useMemo deps corrected
- 39 TypeScript strict errors resolved (0 remaining)

[0.4.6]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.5...v0.4.6


## [0.4.5] - 2026-03-29

### Added

#### Non-Destructive Composite Pipeline
- Immutable original cache (`COMPOSITE_ORIG_R/G/B`) written on blend, compose, and RGB FITS load; all downstream operations (WB, SCNR, re-stretch) reconstruct from originals, making the entire post-blend pipeline idempotent
- `reset_wb_cmd` Tauri command restores composite to post-blend state without re-running blend
- "Reset WB" button in CalibrateStep (appears when factors differ from neutral)
- "Reset to Blend" button in StretchStep for one-click return to clean composite
- Saturation warning banner in StretchStep when WB factors exceed 1.3

#### Idempotent SCNR
- `apply_scnr_cmd` now accepts `r_factor/g_factor/b_factor` and reconstructs from ORIG: applies WB first, then SCNR, writes to working keys; repeated SCNR calls produce identical results
- ColorStep passes current WB factors alongside SCNR parameters

#### Blend Preset Positional Fallback
- SHO, HOO, Foraxx, Hubble Legacy, and Dynamic HOO presets now work with any bin configuration (r/g/b, custom JWST filters, etc.) via positional weight mapping when named channelIds don't match filled bins

#### Spectroscopy Wavelength Unit Conversion
- Automatic unit detection from `CUNIT3` header (M, CM, NM, ANGSTROM, HZ, GHz, KM/S, etc.)
- Display conversion: JWST NIRSpec meters shown as um, HST STIS Angstroms as nm, radio Hz as GHz
- Axis labels and hover tooltips reflect actual converted units instead of hardcoded "um"

#### Vizier Feature Flag
- `vizier` Cargo feature enabling Gaia DR3 TAP queries for real SPCC calibration via reqwest
- Resolves `#[allow(unexpected_cfgs)]` warning in `spcc.rs`

### Changed

#### STF Rendering Consistency
- GPU shader `mtf()` rewritten with symmetric zero-protection (`abs(b) < 1e-8` guard) replacing `max(b, 1e-8)` that inverted the transfer function for midtone < 0.5
- CPU worker STF now filters padding pixels (`<= 1e-7`) matching Rust `is_valid_pixel` threshold, and uses the same `abs(b)` denominator guard as the GPU shader
- GPU, CPU worker, and Rust backend now produce pixel-identical STF output

#### Export Pipeline
- ExportStep reads `compositeStfR/G/B` from RenderContext instead of identity params; exported PNG/ZIP now matches the stretched preview the user sees
- Affects both single-file export and ZIP bundle export

#### Star Detection
- FWHM metric changed from arithmetic mean to true median, consistent with PixInsight, ASTAP, and PHD2 conventions

#### Cube Navigation
- CubeFrameNav resets to frame 0 and stops playback on file change, preventing out-of-bounds slider state
- "Collapse Sum" button relabeled to "Collapse Mean" to match actual backend operation (`collapse_mean`)

#### Dependency Updates
- `tauri` 2 > 2.10, `tauri-build` 2 > 2.5, `rustfft` 6.2 > 6.4
- `@tauri-apps/api` ^2.1 > ^2.10, `@tauri-apps/cli` ^2.1 > ^2.10, `@tauri-apps/plugin-dialog` ^2.1 > ^2.6
- `asdf-full` (bzip2 + lz4_flex) promoted to default features
- Removed unused `config` crate dependency (~150 transitive crates eliminated)

#### WB Slider Range
- CalibrateStep slider max reduced from 2.0 to 1.5; useful range is 0.7-1.3, values above 1.5 caused irreversible clipping in previous versions

### Fixed
- AnalysisTab panels (PlateSolve, FFT, Spectroscopy) now use `effectivePath` for composite-aware operation instead of `file?.path`
- HistogramPanel ResizeObserver cleanup now cancels pending RAF on unmount
- AnalysisTab `flushStfIpc` recursion capped at 3 consecutive failures with `queueMicrotask` break
- `renderStfInWorker` Promise no longer hangs indefinitely on null result
- Downsample worker receives buffer copy via transfer, preventing data corruption during concurrent STF drag
- DeepZoomViewer `generateTiles` removed from useCallback deps (stable module import)

[0.4.5]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.2...v0.4.5


## [0.4.2] - 2026-03-27

### Fixed
- Drizzle finalize: MAD constant hardcoded as `1.4826` instead of shared `MAD_TO_SIGMA`; divergence risk with sigma clipping
- Composite cache keys duplicated as string literals in `cmd/image.rs`; moved to `types/constants.rs`
- `compose_rgb_cmd` STF JSON keys hardcoded instead of using `RES_SHADOW/MIDTONE/HIGHLIGHT` constants

[0.4.2]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.1...v0.4.2


## [0.4.1] - 2026-03-25

### Fixed

#### Numerical
- Median/MAD: NaN values sort to end via `partial_cmp().unwrap_or(Ordering::Equal)`
- FFT power spectrum: Hann window applied before FFT; DC component excluded from magnitude
- Phase correlation: confidence score uses peak/mean ratio of cross-power spectrum
- Richardson-Lucy: Tikhonov regularization denominator uses `max(otf_mag_sq, lambda)` instead of `otf_mag_sq + lambda`
- Polynomial background: Vandermonde basis uses `(x - mean) / std` instead of raw pixel coordinates

#### Performance
- Batch processing: `par_iter` for concurrent file processing
- FFT analysis: transpose + Zip parallel for row-major layout
- Cache: Arc zero-copy for image data sharing
- Base64: uses engine v0.22 API (`STANDARD.encode`)
- Sigma clipping: first iteration uses median/MAD (Stetson 1987), subsequent use mean/stddev

#### Code Quality
- 310 constants in `types/constants.rs` replacing all hardcoded string keys
- Command count increased from 22 to 37
- Architecture diagram updated to reflect new domain modules

[0.4.1]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.0...v0.4.1


## [0.4.0] - 2026-03-22

### Added
- Star-based affine alignment (triangle asterism matching + RANSAC, 500 iterations)
- Stability-based auto white balance (lowest MAD/median channel as reference)
- SCNR luminance redistribution (ITU-R BT.709 weights to R and B)
- ComposeWizard 10-step pipeline (Channels, Stack, BG, Align, Blend, Color, Mask, Stretch, Adjust, Export)
- Masked stretch with star protection (iterative MTF, configurable growth/softness)
- SPCC spectrophotometric color calibration with optional Gaia DR3 TAP
- Synthetic FITS generator (star field, PSF, CCD noise model, vignetting)
- Live composite STF re-stretch (per-channel without re-composing)
- Narrowband palette presets (SHO, HOO, Foraxx, Dynamic HOO, Hubble Legacy)
- PSF enhancement: moment-based FWHM with subpixel peak estimation (quadratic 2D Hessian)

### Changed
- Alignment default: FFT phase correlation (sub-pixel, O(n log n)) with automatic affine fallback
- Frontend refactored into 12 domain services with shared UI primitives
- useBackend.ts split into 11 services/ + infrastructure/tauri/ layer
- PreviewPanel layout: unified sidebar left + bottom panel

[0.4.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.3.0...v0.4.0


## [0.3.0] - 2026-03-14

### Added
- Richardson-Lucy deconvolution (FFT-based, Tikhonov regularization)
- Polynomial background extraction with sigma-clipped grid sampling
- Wavelet denoise (a trous algorithm)
- ASDF format support (first non-Python implementation)
- Roman Space Telescope data model traversal with gWCS extraction
- zlib/bzip2/lz4 decompression for ASDF binary blocks
- FFT phase correlation alignment (40x speedup over ZNCC)
- Smart pipeline: auto-detects 2D/3D data
- Dimension-tolerant stacking with crop-to-intersection
- Auto-resample for mixed SW/LW NIRCam data
- IntelliJ-style panel layout

[0.3.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.2.0...v0.3.0


## [0.2.0] - 2026-03-07

### Added
- Multi-extension FITS with auto SCI HDU selection
- Drizzle stacking with flat contiguous accumulator (Square/Gaussian/Lanczos3)
- Drizzle RGB pipeline for multi-channel composition
- Hubble palette auto-mapping from FITS headers
- AdvancedImageViewer with zoom presets and pan
- Binary IPC for GPU pixels (16-byte header + raw f32, zero JSON/base64)

[0.2.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.1.0...v0.2.0


## [0.1.0] - 2026-02-28

### Added
- FITS I/O with memory-mapped extraction and ZIP transparency
- Batch processing with Rayon thread pool
- Asinh stretch and STF (Screen Transfer Function)
- Bias/Dark/Flat calibration pipeline
- Sigma-clipped stacking with configurable thresholds
- RGB composition with white balance (auto/manual/none) and SCNR
- 512-bin histogram with median, mean, sigma, MAD
- Star detection with PSF-fitting (flux, FWHM, SNR)
- FITS header summary table
- WCS transforms (pixel <-> world coordinates)
- Plate solving via astrometry.net API
- IFU/datacube processing with spectrum extraction
- WebGPU compute shader rendering with Canvas 2D fallback
- Deep zoom tile pyramid for large images
- Zero-copy binary IPC via Tauri Response
- FITS export with WCS/metadata preservation
- Cross-correlation auto-alignment

[0.1.0]: https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/tag/v0.1.0

[Unreleased]: https://github.com/samuelkriegerbonini-dev/AstroBurst/compare/v0.4.6...HEAD
