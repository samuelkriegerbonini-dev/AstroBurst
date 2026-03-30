feat: v0.4.5 - non-destructive composite pipeline, STF consistency, Windows output fix

## Non-Destructive Pipeline
- ORIG/KEY dual-layer cache: blend writes immutable ORIG, all downstream ops reconstruct from it
- calibrate_composite_cmd reads ORIG, applies WB factors, writes KEY (idempotent)
- apply_scnr_cmd reads ORIG, applies WB then SCNR, writes KEY (idempotent)
- reset_wb_cmd copies ORIG to KEY (one-click undo)
- Saturation warning banner when WB factors > 1.3
- WB slider max reduced from 2.0 to 1.5

## STF Rendering Consistency
- GPU shader: symmetric |D| < epsilon denominator guard replacing max(D, epsilon)
- CPU worker: same guard + padding threshold <= 1e-7 matching Rust is_valid_pixel
- GPU, CPU, and Rust now produce pixel-identical STF output
- Worker buffer copied before transfer to prevent race on concurrent STF drag
- renderStfInWorker Promise rejects on null instead of hanging forever

## Blend Preset Spectral Resolver
- Presets (SHO, HOO, Foraxx, etc.) resolve by wavelength instead of positional fallback
- Both preset weights and filled bins sorted by wavelength descending, then zipped
- Works with any bin configuration: narrowband, broadband, JWST filters, custom

## Export Pipeline
- ExportStep reads compositeStfR/G/B from RenderContext (was identity/black)
- StretchStep propagates STF params to RenderContext via onResult callback
- ComposeWizard handleRestretchPreview forwards STF to setCompositeStf
- Affects both single PNG export and ZIP bundle

## Windows Output Path (os error 5)
- All 17 hardcoded "./output" replaced with await getOutputDir() (appDataDir)
- Compose steps, Processing panels, Stacking panels, Analysis, Spectroscopy
- ProcessingTab resolves once via useEffect, passes to 5 child panels as prop

## Star Detection & Overlay
- Star count display: stars.length replacing undefined starResult?.count
- Overlay circles: canvas set to display:block before getBoundingClientRect
- Merged draw + display effects into single effect with correct sequencing
- FWHM metric: true median replacing arithmetic mean

## Spectroscopy
- Wavelength unit auto-detection from CUNIT3 header
- 15 unit conversions: M->um, ANGSTROM->nm, HZ->GHz, etc.
- Axis labels and hover tooltips reflect actual converted units
- Collapse button relabeled "Collapse Mean" matching backend behavior

## Cube Navigation
- CubeFrameNav resets to frame 0 on file change
- getCubeFrame removed from useCallback dependency arrays

## Analysis
- HistogramPanel ResizeObserver cancels RAF on disconnect
- flushStfIpc recursion capped at 3 failures with queueMicrotask break
- DeepZoomViewer generateTiles removed from useCallback deps

## Performance
- blend_channels parallelized: par_iter_mut().zip() replacing sequential loop

## Dependencies
- tauri 2 -> 2.10, rustfft 6.2 -> 6.4, tauri-build 2 -> 2.5
- @tauri-apps/api ^2.1 -> ^2.10, @tauri-apps/cli ^2.1 -> ^2.10
- Removed unused config crate (~150 transitive crates)
- vizier feature flag for Gaia DR3 TAP (reqwest/blocking)
- asdf-full promoted to default features

## Docs
- CHANGELOG v0.4.5 entry
- LaTeX tech doc updated to v0.4.5 (690 lines)
- Sample data README restructured with ComposeWizard workflow

NEW UI
