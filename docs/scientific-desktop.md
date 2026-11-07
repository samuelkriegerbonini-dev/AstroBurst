# Desktop multiband preparation and SAGUI

**Cube Prep** prepares calibrated multiband images inside AstroBurst. **SAGUI** groups their pixels by spectral energy distribution (SED) using the official R package. Open either tool from the dock or search its name in the command palette; neither requires an image already open in Files.

Cube Prep uses the same recipe runner as the headless API. SAGUI runs in a separate R process with a private package library. Cube Prep itself does not require R.

## Prepare a cube

1. In **Cube Prep**, choose **Add FITS images** or **Use opened images**. Set the dataset and filter for every image. Filename suggestions are only suggestions: verify them against the observations. **Inspect inputs** reports the source unit and whether ERR and DQ extensions exist.
2. Choose **Scientific MEF for regional SEDs / SAGUI** under **Apply defaults**. Keep **Units** at `Jy/pixel` for SAGUI. Select an alignment reference and a cube reference, and choose a persistent **Working folder**.
3. Configure the enabled steps. For bands with different PSFs, enable and configure **PSFs**, **Kernels** and **Match**. Check the resulting PSFs, residuals and diagnostics before calling the cube suitable for regional photometry.
4. Select **Validate** to resolve inputs, filters, parameters and dependencies without running the pipeline. Read its warnings and effective recipe. Then select **Validate & run**.
5. Inspect the step reports and product list. Use **Open final cube** to load the FITS product in Files, or **Send to SAGUI** to check the current cube and open the SAGUI tool with its path selected. This does not start segmentation. The common PSF target is copied only when the final cube follows a complete matching step with no unmatched band; otherwise it remains blank. In SAGUI, **Use Cube Prep result** performs the same input check and selection.

Changing an input or recipe parameter clears the displayed result associated with the previous settings. Its files remain in the previous run folder.

### Profiles and available steps

The initial **LIGHTSTACK 0.4.0 compatibility** profile makes its choices explicit: conversion to Jy per pixel, bilinear alignment, a one-pixel border crop, the LIGHTSTACK area rule, wavelength ordering and the LIGHTSTACK cube layout. That layout stores SCI only. Use it for comparisons with the reference pipeline while accounting for its discarded ERR/DQ companions.

The scientific profile selects MEF output and the physical pixel-area rule: scale per-pixel flux for a change of pixel area, or keep surface brightness when the selected unit is `MJy/sr`. It also selects AstroBurst crop rounding and matching behavior, and enables DQ use for an existing invalid-pixel editing step. Applying a profile changes these settings on enabled steps; it does not add missing PSF inputs, enable every step, invent uncertainties or certify the result. SAGUI requires per-pixel flux, so convert a surface-brightness cube before segmentation.

| Step | Purpose and checks |
|---|---|
| Crop | Common coverage, reference footprint, sky region or DS9 region; inspect bounds and rounding. |
| Units | Convert calibrated units using the available metadata; an unknown calibration is a validation problem. |
| Align | Reproject to the selected reference grid; choose interpolation, pixel-area treatment and border crop. |
| Cube | Assemble the ordered bands and their filter metadata; choose MEF to retain available companions. |
| Edit | Remove filters, crop, update metadata, apply a reference mask or remove invalid support. |
| PSFs | Read supplied PSFs or estimate them from stars; verify sampling, centering and the target PSF. |
| Kernels | Build matching kernels and inspect their diagnostics before applying them. |
| Match | Homogenize bands to the chosen reference PSF; allowing unmatched bands does not establish a common PSF. |
| Export | Write the selected final cube to a named FITS product within the run folder. |

For empirical PSFs, the image names must identify their filters and the selected data directory must contain the corresponding `stars_<lowercase-filter>.reg` DS9 star-box selections. A broad PSF reference is a starting choice, not proof that every kernel is scientifically acceptable.

**Import recipe**, **Save recipe** and **Recipe editor** expose recipe version 1. The editor supports repeated steps and custom ordering; select **Apply recipe** before validation. Relative input paths are resolved from the selected input base folder, or from the imported recipe's directory. Saved validated recipes contain resolved paths, so review paths when moving a recipe to another computer. See the [Cube Prep API reference](server-api.md) for the shared runner contract.

## Install or recover the SAGUI runtime

This integration requires **R 4.6.2** and SAGUI commit **`56fd3ea776d7c00e056368498da9412bd58b84bb`**. The upstream package version at that commit is `0.0.0.9000`; the version alone does not identify the required source. Upstream SAGUI accepts other R versions, but AstroBurst currently checks this exact R version and commit.

Install R separately from the [R Project's CRAN distribution](https://cran.r-project.org/). On Windows, install [Rtools45](https://cran.r-project.org/bin/windows/Rtools/rtools45/rtools.html) before building SAGUI. The managed installer checks that a native R extension can be compiled. This feature does not install or bundle R or the system compiler.

1. Expand **R and SAGUI installation** and use **Browse** beside **Rscript executable** to select the installed executable.
2. If a compatible private library already exists, select it under **Private R package library** and choose **Check R installation**. This check works before selecting a science cube.
3. For a new or damaged library, choose a persistent **Library installation folder**, then **Install / repair SAGUI library**. An internet connection is required to retrieve the pinned SAGUI source and its dependencies.
4. Wait for **Private SAGUI library installed and verified**. The app selects the returned library only after a separate runtime check succeeds. Use **Show installation files and logs** to inspect the installation record.

Every attempt creates a new `install-<run-id>/library` beneath the chosen installation folder. Choose a parent folder outside any existing SAGUI package library; a destination inside a previous library is rejected. Repair does not overwrite the previous library. A failure or cancellation preserves diagnostics and leaves the previous library selection intact. Keep successful private libraries unchanged while using them for analysis.

The managed CPU profile installs required dependencies without optional `Suggests` packages and excludes `torch` and `fastcluster`. Before publishing a ready installation, it checks exact Ward, starlet and sparse Ward on a non-square fixture, checks determinism with a fixed seed, independently inspects the runtime, and records package versions and integrity hashes. These checks establish that the selected implementation can run; they do not validate a particular science observation.

The installation records the dependency versions actually obtained. It is not a universal dependency lock for future installation dates or other platforms. Preserve the private library and its manifest when exact reproduction matters.

## Segment and inspect regional SEDs

1. Select **Use Cube Prep result**, **Use selected FITS**, or browse to a prepared multiband cube. Select the whole file, rather than an individual HDU or plane.
2. Enter the **Number of regions** and choose **Exact Ward** or **Sparse Ward (approximation)**. Exact Ward has quadratic memory demand in the pixel count. The displayed estimate uses the full footprint before masking; the runner checks memory for the selected mode before starting R. For large fields, crop the input or explicitly choose sparse mode and its nearest-neighbor count. Sparse mode is a different approximation and is never selected silently.
3. Enter the **Verified common PSF target**. This field records a target already established by your preparation and diagnostics. A name such as `F444W` does not verify PSF matching, including for an external cube.
4. Configure **Use starlet morphology mask**, its scales and threshold. Choose **DQ bits to exclude (decimal mask)** according to the instrument and analysis. `4294967295` excludes every flagged bit; `0` ignores DQ flags. Non-finite science values remain excluded in either case.
5. Select a persistent **Working folder**, then **Check cube and runtime**. Inspect dimensions, filters, pivot wavelengths, units, ERR/DQ availability, memory estimate and warnings. Select **Validate & segment** to execute the official package.
6. Click a colored region or select its identifier in **Regions and SEDs** to inspect the regional fluxes and uncertainties. The map uses X to the right and Y upwards; label zero is background. The SED display retains negative fluxes and marks unknown uncertainties explicitly.

The categorical preview may be reduced in resolution for display. The exported FITS maps preserve the full input grid.

### Scientific input and measurement contract

- The cube must contain at least two photometric bands, `CTYPE3=FILTER`, a valid celestial WCS, unique filter names with positive pivot wavelengths, and calibrated per-pixel physical units. An IFU spectral cube is not this multiband input format.
- ERR and DQ, when declared, must load as 3-D companions with the same dimensions as SCI. ERR must explicitly carry the same calibrated unit as SCI. A supplied companion WCS must match the science grid; an absent companion WCS produces a warning that same-index alignment is being assumed.
- Eligible support requires finite SCI in every band and no excluded DQ bit in any band. Starlet can further restrict the support. Missing DQ means that only finite-value support can be checked. DQ flags have instrument-specific meaning; preserving flags does not itself choose which flags should exclude a pixel.
- Regional photometry sums the original calibrated SCI values within each region, rather than the transformed values used for clustering, and converts the sum and its uncertainty to Jy. The result's unit is used by both the SED viewer and CSV; the manifest separately records the original input unit. For example, a summed input of 11 mJy per pixel becomes a regional flux of 0.011 Jy. Zero and negative finite fluxes remain valid measurements.
- Regional uncertainty is `sqrt(sum(ERR^2))` under an independent-pixel assumption. If a used uncertainty is missing or invalid, the regional uncertainty remains unknown; it is not replaced by zero. Missing ERR does not prevent segmentation, but does prevent known regional errors.
- Alignment and PSF convolution propagate available variance and DQ, but introduce covariance between output pixels that an ERR plane cannot represent. Since 0.8.0 a scientific MEF cube records it as a `NOISCORR` extension, one normalized noise autocorrelation map per plane, and SAGUI adds `error_correlated_jy`, the regional error with that correlation applied. It stays empty when the cube has no `NOISCORR`, and for PSF-matched planes whose map covers alignment only; then only the independent-pixel error is known, and it underestimates the true uncertainty. The LIGHTSTACK layout never carries `NOISCORR`.

Unit prefixes preserve their physical meaning: `mJy` is millijansky and `MJy` is megajansky. In Cube Prep, `mJy/sr` and `MJy/sr` therefore differ by a factor of one billion before conversion. The legacy photometry calibration supports the megajansky convention and refuses milli input; convert it explicitly in Cube Prep. Spectral calculations reject an explicitly incompatible ERR unit when using ERR. Missing ERR BUNIT retains the documented assumption that ERR uses the SCI unit; no scale conversion is invented.

## Run files, cancellation and reopening

Cube Prep writes a separate `cubeprep-<name>-<run-id>` folder for each run. It contains `recipe.json`, numbered step directories and, after successful completion, `result.json`. The report contains step parameters, diagnostics, warnings, software/platform information, dependency and output hashes, and the final cube path. Input hashes are checked before and after execution; this is not an atomic snapshot, so leave inputs unchanged during a run. A matching hash identifies bytes, not scientific validity.

SAGUI writes a separate `sagui-<run-id>` folder. The main products are:

| File | Contents |
|---|---|
| `labels.fits` | Full-resolution region identifiers in the `LABELS` extension; zero is background. |
| `support.fits` | Segmentation support in the `SUPPORT` extension. |
| `regions.csv` | Region and band identifiers, pixel counts, pivot wavelengths, summed flux, independent-pixel uncertainty and, from 0.8.0, `error_correlated_jy`, all in Jy; unknown errors are empty. Results saved before 0.8.0 still reopen. |
| `manifest.json` | Completion state, input/runtime/library identities, configuration, output hashes, platform and measurement limitations. |
| `config.json`, `bridge.R` | Parameters and the R bridge used for the job. |
| `packages.tsv`, `session.txt` | Installed package and R session details. |
| `stdout.log`, `stderr.log` | Runtime output and diagnostics. |

The SAGUI manifest is published with `status: complete` only after output and identity checks. Installation manifests instead use `status: ready` and include self-test and compiler logs; failed installation attempts retain `failure.json` where the attempt directory was created. Partial products or logs without the corresponding successful manifest are not completed results.

Use **Cancel** to request cancellation of a running preparation, installation or segmentation. Progress remains associated with that run; partial files and logs are retained for diagnosis. Closing a dock panel does not cancel its active job. Restarting the app does not resume an interrupted job.

Cube Prep saves its draft, working folder and latest report locally. SAGUI saves its settings and installation folder; its interactive result is held for the current app session. Editing settings clears the associated displayed results. Saved FITS products can always be reopened through Files. Use **Show photometry CSV**, **Save photometry as…** and **Show manifest** to inspect or copy completed outputs.

After restarting the app, choose **Open previous SAGUI result** and select the completed job's `manifest.json`. The backend verifies the saved products and their integrity before restoring the region map, SEDs and saved settings. A verification failure leaves the result unavailable and reports the problem. Opening a saved result does not execute R or require a current R installation; starting a new segmentation checks the configured runtime again. The restored working folder is the job's parent, so a new run creates a separate sibling directory. The interface identifies the loaded result with **Previous result opened and verified**.

When inspecting a processed FITS image, **Headers** offers **Displayed product** and **Source input**. The product view reads the product's own header and HDUs without switching the source image's active HDU. PNG previews and composites have no FITS product header; inspect the source explicitly when needed.

## Spectral Collapse and zero-valued samples

### Scientific manifests and interactive inspection

Completed Fit lines maps, Moment Maps, Collapse, Quick Slope and RATE comparisons expose **Show manifest**. It reveals the result's `provenance.json` in its folder. Keep this manifest with all result files; exporting the displayed FITS image alone does not copy the manifest. Spectral map results also show the input SHA-256 prefix and verification method when reported by the backend. The full hash and input reference remain available in the hash tooltip and manifest. RAMP source identities are listed in its manifest, including both Quick Slope and official RATE inputs for a comparison.

These calculations hash the original input before opening it and again before publishing the successful manifest. Scientific acquisition reopens the cube and its ERR/DQ companions and bypasses the decompression cache for gzip input. A detected input change prevents successful publication. Each run uses a separate result folder, preserving previous completed products. Hashes identify file contents; they do not establish calibration quality or scientific validity.

This is a check at two points in time, not an immutable input snapshot. A transient rewrite that restores the original bytes between the checks can escape detection. Leave inputs unchanged while calculating.

Interactive Fit lines spaxel inspection and RAMP pixel inspection reopen their readers without a full content hash. Plain FITS headers are read again, but gzip inspection can reuse the fast decompression cache based on path, size and modification time. A replacement preserving those attributes can therefore leave an interactive gzip inspection stale. These inspection responses carry no SHA-256 proof; calculate the exported maps or products for the stronger acquisition and publication checks described above.

### Zero-valued samples

The Cube tool's **Collapse zeros** selection applies to both full-cube Collapse and **Collapse range**. The desktop default, **Include measured zeros**, treats finite zero-valued science samples as measurements. **Exclude zero padding (legacy)** removes exact zeros and is appropriate only when those values are known to mark padding. Both choices exclude non-finite science values. Changing the choice clears earlier Collapse results and prevents pending results from the previous choice from appearing.

The valid SCI samples determine the sample count used for the associated uncertainty calculation. With measured zeros included, a zero-valued SCI sample whose ERR is invalid leaves the uncertainty unknown. It does not silently disappear from the reduction.

SUM, MEAN and MEDIAN reduce samples. They do not weight channels by wavelength or velocity width and should not be interpreted as integrated spectral line fluxes. The effective zero policy is recorded in the response, provenance and FITS `ABZERO`/HISTORY metadata. API callers that omit `zeroPolicy` retain the legacy exclusion behavior; the desktop passes its choice explicitly.

### Shared DQ policy for spectral maps

**Science DQ** controls Fit lines, Moment Maps and both full-cube and range Collapse. The desktop default, **Exclude DO_NOT_USE (bit 0)**, rejects samples carrying that bit; other valid flag bits do not exclude samples under this policy. Unreadable DQ values are also excluded: FITS BLANK, and floating-point values that are non-finite, fractional, negative or at least `2^24`, are invalid rather than usable flags. Choose **Ignore DQ flags** explicitly when the analysis requires it. The Fit lines checkbox uses this same setting, rather than a separate mask. This policy is independent of the SAGUI decimal bitmask described above.

Changing the shared DQ policy clears the affected completed results and rejects pending responses produced with the previous setting. The result summary reports the effective policy, whether it was applied, the DQ HDU and excluded-sample count when available. A requested policy does not imply that a usable DQ extension exists: missing DQ leaves the calculation available with a warning and `dq_applied: false`. Read the backend notes for invalid DQ values, FITS BLANK handling or floating-point fallbacks.

Desktop calls send `do_not_use` or `ignore` explicitly. API calls may omit the policy for compatibility: Moments and Collapse retain their legacy ignore behavior, while Fit derives it from its existing `use_dq` setting. An explicit Fit `dq_policy` takes precedence over `use_dq`. Collapse's separate zero policy remains in effect.

## Validation checkpoint and limits

Recorded on 2026-11-02 for this implementation:

| Checkpoint | Observed result | Scope |
|---|---|---|
| Desktop frontend base | 265 tests passed | Cube Prep/SAGUI state, service contracts, plots and dock integration. |
| Headers and installation frontend | 25 tests passed | Active/source headers, async invalidation, SAGUI installation, state and service behavior. This overlaps the base scope and must not be added to it as a unique-test total. |
| Official SAGUI reference | 1 test passed in 7.06 s | `official_sagui_matches_direct_non_square_segmentation_and_regional_sed`, comparing a synthetic non-square case and regional photometry with direct official R execution. |
| Expanded live SAGUI validation | 13 tests passed, none failed or ignored, in 14.28 s | Includes direct non-square segmentation/SED comparison, sparse and starlet comparison on a 23 × 17 fixture, and R process cancellation. This includes the earlier official reference test. |
| SAGUI saved-product validation | 17 tests passed, none failed or ignored | Includes the live R tests above and verified restoration of saved maps, photometry and configuration. |
| Integrated frontend after DQ and manifests | 401 files, 6210 tests passed | `pnpm exec vitest run --reporter=dot`; TypeScript check also exited 0. |
| Integrated Rust library after P0 corrections | 4105 passed, 0 failed, 112 ignored | `cargo test --manifest-path src-tauri/Cargo.toml --lib`; ignored tests were not run by this command. |
| Server integration | 258 passed, 0 failed | `cargo test --manifest-path src-tauri/Cargo.toml --features server --bin astroburst-server`. |
| Synthetic Brγ reference | 3 passed, 0 failed | Six synthetic spaxels compared with SciPy; see [reference and limits](../src-tauri/tests/fixtures/spectral/README.md). |
| Windows release build after P0 corrections | Exit 0; Vite 10.23 s, optimized Rust release 12 min 13 s, MSI 29.16 MiB | `pnpm tauri build --bundles msi`; log `entregas/p0-desktop-release-build.log`. The observed desktop checks are described below. |
| Synthetic Brγ desktop products | 155 checks passed, 0 failed; eight files hashed | Independent Astropy 8.0.2/NumPy 2.5.4 validation of SCI/ERR, masks, counts, WCS, units and provenance from the two desktop Collapse runs; `entregas/brgamma-desktop-validation.md`. |

The current Windows artifacts are `src-tauri/target/release/astroburst.exe`, SHA-256 `1E34A099751232FFBE32359F4A99EACBF118C1EFFB4ACC3D19EE3E97113F4968`, and `src-tauri/target/release/bundle/msi/AstroBurst_0.7.8_x64_en-US.msi`, SHA-256 `32D39FEA4DD9F88B494FBD69D39A59C8E044ACC6D6E37929F1578D53DDF3FDE5`. These hashes identify the 2026-11-02 P0 build and supersede earlier artifacts at the same paths. The JavaScript chunk-size warning remains non-blocking. A successful build does not establish a successful clean-machine installation.

The new release was opened directly from the recorded executable. In the desktop, the synthetic 3 × 2 × 121 Brγ cube completed **Collapse Mean** with measured zeros included and **Exclude DO_NOT_USE (bit 0)**: the panel reported DQ HDU 3, 123 excluded samples, the input SHA prefix and **Show manifest**. Changing to **Ignore DQ flags** cleared the old output and returned the preview to the source before recalculation; the second result reported **DQ ignored by choice**. Independent validation found SCI and ERR bit-identical in float32 to the expected values and to a NumPy calculation from the input FITS. The masked result has one NaN in each of SCI and ERR; the ignored-DQ result has none. This is a synthetic measurement check, not an observed Brγ data validation.

In the same release, **Open previous SAGUI result** restored the saved synthetic job's configuration and showed **Previous result opened and verified**. Its `labels.fits` opened successfully. Selecting region 1 (91 pixels) and then region 2 (34 pixels) updated the SED. This verifies restoration and inspection of that saved result; it is not a new segmentation or an observational validation.

The live reference tests are opt-in. They use `ASTROBURST_SAGUI_RSCRIPT` and `ASTROBURST_SAGUI_LIBRARY` to select an existing compatible runtime. The expanded checkpoint was run with `cargo test --manifest-path src-tauri/Cargo.toml --lib service::sagui::tests -- --include-ignored --nocapture`. Frontend tests use controlled service responses; they do not establish a successful package download or native desktop interaction on a clean machine.

This checkpoint does not certify clean-machine Windows installation, Linux desktop/runtime installation, cross-platform determinism, public JADES science results, arbitrary external PSFs, or general equivalence of all Cube Prep operations to LIGHTSTACK. The non-square synthetic SAGUI comparison does not establish the accuracy of real-galaxy segmentation or compensate for unknown input covariance. Validate those properties with the relevant data and reference workflow before treating them as established.

## Implementation and attribution

The desktop adapters are in [the Cube Prep service](../src/services/cubeprep.ts) and [the SAGUI service](../src/services/sagui.ts). Cube Prep calls [the shared recipe runner](../src-tauri/src/service/cubeprep_recipe.rs). SAGUI input validation, regional photometry and publication live in [the SAGUI service](../src-tauri/src/service/sagui/mod.rs), with the [R bridge](../src-tauri/src/service/sagui/bridge.R) and [managed installer](../src-tauri/src/service/sagui/install.rs) alongside it.

LIGHTSTACK is by Andressa Wille and Thallis Pessi. SAGUI is by Rafael S. de Souza. See [LIGHTSTACK](https://github.com/AndressaWille/lightstack), the [SAGUI source at the pinned commit](https://github.com/RafaelSdeSouza/sagui/tree/56fd3ea776d7c00e056368498da9412bd58b84bb), its [package metadata](https://github.com/RafaelSdeSouza/sagui/blob/56fd3ea776d7c00e056368498da9412bd58b84bb/DESCRIPTION) and [copyright notice](https://github.com/RafaelSdeSouza/sagui/blob/56fd3ea776d7c00e056368498da9412bd58b84bb/LICENSE). Both projects use the MIT license. AstroBurst carries their credits and texts in [third-party notices](../src-tauri/licenses/THIRD-PARTY-NOTICES.md); R and each installed dependency retain their own licenses.
