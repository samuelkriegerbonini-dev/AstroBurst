## [0.4.6] - 2025-04-06

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
