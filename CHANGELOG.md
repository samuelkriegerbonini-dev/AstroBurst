# Changelog
All notable changes to AstroBurst will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Analysis is now five dock tools with their own strip icons instead of one tool with Image / Sources / Cube tabs: Image (histogram and STF, statistics, pixels, regions, profiles, contours, FFT, deep zoom), Astrometry (star detection, plate solve, catalog, targets, observation geometry), Photometry (aperture photometry, photometry table, time series), Cube (ramp, spectrum with moments and Fit lines, PV) and Log (the measurement log, with the entry count as a badge on its icon and in its label, "Log (3 entries)"); each docks, moves and opens side by side like every other tool, detected stars are shared through a store so the Photometry table sees the stars found in Astrometry even when the two sit in different anchors, the four per-file tools keep their inputs across file switches as long as one of them is open (as the single Analysis tool did), and when the selected file changes an open Image or Cube switches to the one that fits the file (Cube for cubes and ramps) and remembers the one picked per file; saved layouts that contain the old `analysis` tool are migrated in place (the four tools take its position, Log goes to Bottom Right, an open Analysis reopens as Image); "Open in Spectroscopy" on an x1d row opens the Cube tool; the strip scrolls when the tools do not fit and a moved button is scrolled into view

## [0.6.6] - 2026-10-03

### Added
- Live IFU kinematic maps ("Line fit maps" in Analysis > Cube > Spectrum, button Fit lines): with a line range brushed on the spectrum and a rest wavelength set, every spaxel of the cube is fitted with a Gaussian over a continuum fixed from the two side windows (the continuum windows set on the spectrum, or default windows beside the range), in a fraction of a second on a NIRSpec IFU cube, with ERR-propagated errors and DQ masking, inside the viewer; the result is one 2-D map per plane, `flux` (A·σ_v·√(2π) in `<BUNIT> km/s`), `velocity`, `sigma_obs`, `sigma_corr` (only with a resolving power), `flux_err`, `v_err`, `sigma_err`, `chi2_red`, `snr`, `mask` and `ncomp`, each shown in the viewer from its plane button; Weight by ERR (on by default) weights each channel by 1/ERR² from the cube's `ERR` HDU and reports absolute-sigma errors, inflated by √χ²_red where χ²_red > 1 (without ERR the weights come from the continuum scatter and the formal errors are scaled by √(χ²/dof)), and every error includes the continuum uncertainty, the flux error also the amplitude-sigma covariance; Mask DQ channels (on by default) drops channels flagged `DO_NOT_USE` (bit 0) in the `DQ` HDU from the line and the continuum fits; the continuum is linear when both side windows hold at least 3 usable channels and falls back to a constant per spaxel otherwise (mask bit 2), so no slope is extrapolated from one window; an optional resolving power R (never prefilled) gives `sigma_corr` with the instrumental width removed (NaN and mask bit 16 where the line is unresolved), otherwise sigma is labelled observed; Components `two` or `auto` (default `one`) adds a two-Gaussian fit where the single fit passes the S/N threshold and at least 8 channels are usable, kept only when the centres are at least one channel apart, each component is wider than the sigma lower bound and narrower than a quarter of the window, carries at least 10 % of the flux and has its centre off the window bounds, and in `auto` also when ΔBIC/max(1, χ²_red) ≥ 10 (ΔBIC of the single fit over the pair, divided by the single fit's χ²_red so a misfitting single Gaussian does not accept every pair) and both amplitudes are at least 3σ; accepted pairs fill twelve `c1_*`/`c2_*` planes (c1 bluer, c2 redder, NaN where `ncomp` is 1) while the main planes keep the single fit; with Emission only off, absorption lines are mapped too (negative flux and S/N, mask bit 1 when |S/N| reaches the threshold) and an absorption pair is split into two negative components; mask bits: 1 fitted at or above the S/N threshold (default 3), 2 constant continuum, 4 channels dropped by DQ, 8 channels dropped by a non-finite or non-positive ERR, 16 unresolved, 32 fit did not converge, 64 two components rejected or not attempted, 128 two components accepted; velocities are in the axis frame named by `SPECSYS`, with no barycentric or heliocentric correction, and the notes say so together with the single-Gaussian caveat; each plane is written as `{stem}_linefit_{plane}_{z0}-{z1}.fits` (plus a PNG) in the output folder with the celestial WCS kept, `BUNIT` per plane and the `LINEFIT`, `LFREST`, `LFZ0`, `LFZ1`, `LFCONV`, `LFWEIGHT`, `LFERRHDU`, `LFDQHDU`, `LFRPOW`, `LFSIGMA`, `LFNCOMP`, `LFSNR`, `LFCONTA` and `LFCONTB` cards, and the `mask` plane also carries `LFMASKA`, `LFMASKB` (the bit meanings) and `LFMASKC` (mask 0 with `ncomp` ≥ 1: converged below the S/N threshold; `ncomp` 0: no fit); line windows shorter than 5 or longer than 400 channels are refused; each run is added to the measurement log as a `line_fit` entry with its window, continuum windows, weighting, ERR and DQ HDUs, threshold, components and resolving power (`cube_line_fit_cmd`, `cube_line_fit_spaxel_cmd`; ERR and DQ are found by `EXTNAME` with the cube's geometry)
- Spaxel inspect for the line-fit maps: a click on a displayed line-fit plane refits that spaxel with the same settings (`cube_line_fit_spaxel_cmd`) and shows a 150 px mini plot of its spectrum over the fitted span, drawn only from the arrays the backend returns: the samples used in the fit, channels outside the windows in grey, DQ- or ERR-dropped channels as crosses, ERR bars, the line window and the continuum windows as bands, the continuum dashed, each component dashed in blue (c1) and red (c2) on top of the continuum, the total model solid and a dotted marker labelled with the velocity of each component; below it a table with V, σ (observed, or corrected with the observed value in brackets), flux, amplitude, χ²_red with its degrees of freedom, S/N, ncomp, ΔBIC (raw and divided by max(1, χ²_red)), the dropped channels, the continuum (linear or constant, channels, level, scatter), the fit status (converged, iterations, weighting, mask) and the c1 (blue) and c2 (red) components; the plot has a text summary for screen readers
- Rest-frame line list: Brα 4.052262 µm, Brβ 2.625872 µm, Pfβ 4.653778 µm and He I 2.058690 µm (vacuum), 27 built-in lines in all
- Line-fit planes open with a matching display: the velocity planes (`velocity`, `c1_velocity`, `c2_velocity`) in RdBu inverted (negative velocities blue) with limits symmetric about 0, a linear stretch and percentile limits; the flux, width, error, `chi2_red` and `snr` planes in viridis; `mask` and `ncomp` in gray with min/max limits; this is a temporary override that is never saved (changes made while it is on screen are not saved either), and the previous display returns when the viewer leaves the line-fit maps: on another result, on Revert to original, on a file switch and after a reload
- Synth stack frames vary from frame to frame by default (Vary frames, stack mode only): a dither of up to ±3 px (Dither (px), 0-20), a seeing jitter of ±10 % on the FWHM (Seeing jitter %, 0-50), ±5 % sky and transparency jitter and cosmic rays (Cosmic rays toggle; round(25 × megapixels) rays per frame, each 1-3 pixels of 5,000-50,000 e-, added after the noise), each frame rendered with its own offset and PSF width; frame 0 is the reference (no offset, jitter or cosmic rays) and equals the single image of the same settings, and with Vary frames off the frames keep identical stars and differ only in noise, as before (`frame_variation` block in the config, every field optional)
- Synth stacks write a `{prefix}_frames.csv` manifest next to the frames (`frame`, `dx`, `dy`, `fwhm_factor`, `sky_factor`, `transparency`, `n_cosmic_rays`, `date_obs`), listed on the result card as Frames manifest (`frames_manifest_path` in the response)
- Synth panel: a dice button next to Seed (New random seed) picks a new seed in 0-9999 (the same seed still reproduces the output byte for byte), and Open in viewer on a single-image result adds the generated FITS to the file list, or selects it when it is already open
- Synth provenance cards: every generated frame carries `SYNSEED`, `SYNFIELD`, `SYNPSF`, `SYNFWHM` (the rendered FWHM, including the frame's seeing jitter) and `RDNOISE`, stack frames also `SYNFRAME`, `SYNDX`, `SYNDY` and `SYNOFFS = 'star = catalog + (SYNDX, SYNDY) px'`
- JWST Level-1b `_uncal` ramps and Roman L1 resultant stacks open as ramps: the frame navigation reads "group g of N, integration i of M" (ArrowLeft/ArrowRight step the group, Shift steps by 10), the Ramp section in Analysis > Cube shows the instrument, readout pattern, groups x integrations, the NIRSpec IRS2 layout and the group time (`TGROUP`, else `TFRAME x (NFRAMES + GROUPGAP)`), lists the GROUP and INT_TIMES tables, and a click on a pixel plots its raw and reference-corrected ramp with the fitted line and the saturated, unused and jump-flagged groups (`ramp_info_cmd`, `ramp_pixel_series_cmd`, `ramp_tables_cmd`, `ramp_frame_cmd`, `ramp_pixel_fit_cmd`)
- Quick slope for JWST `_uncal` ramps (Analysis > Cube > Ramp, Run quick slope): one integration becomes `<stem>_qslope.fits` with SCI (DN/s), NGOOD, DQ and NOISE extensions, offline and without reference files, after stripping the NIRSpec IRS2 reference output and interleaved reference rows and subtracting a per-amplifier reference offset, with saturation by threshold (`sat_dn`), crude jump flagging (`jump_k`) and an OLS or median slope; it is a decision aid, not a rate (no superbias, linearity, dark, FFT reference-pixel correction or weighted fit; the header carries `ABPROC = qslope` and never `CAL_VER`, `S_RAMP` or `DATAMODL`); Compare with rate reads the official `_rate.fits` next to it (or one you pick) and reports the zero-shift alignment check, the quick-minus-official deltas per rate bin and per amplifier, JUMP_DET and SATURATED recall and precision, a verdict against the published tolerances and a ratio image (`ramp_quick_slope_cmd`, `ramp_compare_rate_cmd`); measured on 8 program-1266 NIRSpec IRS2 exposures: faint additive offset within 0.011 DN/s full frame and 0.021 DN/s per amplifier, +1.2 to +2.7 % at 1-30 DN/s (details in `docs/quick-slope.md`)

### Changed
- The Analysis panel is split into three tabs: Image (histogram, statistics, pixel table, regions, profiles, contours, FFT, Deep Zoom), Sources (stars, photometry, photometry table, time series, observation geometry, catalog, targets) and Cube (ramp, spectrum, PV), with the measurement log in a collapsible "Measurement log (N)" footer under them (collapsed by default, its open state kept for the session); the chip row jumps to the panels of the active tab only; panels in a hidden tab stay mounted, so their settings and results survive a tab switch, but ignore viewer clicks (Measure on image click and Read on image click do nothing while their tab is hidden, the ramp pixel plot does not refetch, and Deep Zoom closes); a cube or ramp opens on Cube and any other file on Image, the tab you pick is remembered per file for the session (with a fallback to the default when that tab is unavailable), Cube is disabled with "Open a data cube or a ramp" when the file has neither, switching tabs scrolls to the top of the new tab, and the tab bar works from the keyboard (arrow keys wrap and skip the disabled tab, Home and End)
- Synth: the exposure slider reads "Exposure (s)" with the hint "sets EXPTIME and dark current; star flux and sky are per-frame totals" (star flux and sky have been per-frame totals since 0.6.1; only dark current scales with the exposure)
- Synth stacks: without an explicit `cadence_seconds` the cadence is the exposure plus 10 s, so DATE-OBS and MJD-OBS no longer describe overlapping exposures (it was a fixed 60 s against the default 300 s exposure); an explicit cadence is still honoured
- Synth seeds: the panel sends one seed and the backend derives the field, noise, flat, frame and cosmic-ray streams from it (SplitMix64 mix of the seed, the purpose and the frame index) instead of `seed + 1000` and `seed + 7919 × frame`; a `noise.seed` sent by an older caller is mixed in rather than used raw; every seed now gives a different image than before
- Synth: Stars is the number of stars inside the frame: King and disk fields redraw a position until it lands on the frame (a disk that cannot fit is refused with an error instead of hanging), so the catalog and the result card list only in-frame stars (a 500-star disk on a 512×512 frame used to put 242 of them off the image and still count them)
- A 4-D FITS image (`NAXIS4` > 1) with `NGROUPS`/`NINTS` cards, or `DATAMODL` `Level1bModel`/`RampModel`, opens as a ramp instead of a plain cube of NINTS x NGROUPS planes labelled by channel, and the primary header supplies the ramp cards (the WCS and units still come from the `SCI` header)

### Fixed
- Synth images all looked alike: since 0.6.1 star flux is total electrons per frame, but the default flux range stayed at 100-50,000 e-, so about 97 % of the stars sat below the noise (14-16 of 500 above 5σ at the defaults) whatever the distribution, PSF, star count or flux max; the defaults are now 2,000-500,000 e- in the panel and in the Rust `FieldConfig::default`, and a test checks that at least half of the stars peak above 5σ of the background noise at the default settings
- Synth vignetting multiplied only the star image, so the sky never darkened towards the corners; the flat now multiplies stars and sky (`(star + sky) × flat + dark`, never dark current or bias), and the ground truth stays the noise-free, pre-flat star image without sky, which its new `SYNTHGT` card states
- Regenerating a synthetic image over a file open in a viewer tab kept showing the old image; `generate_synth_cmd` outputs (the FITS and, when saved, the ground truth) are now announced, also when the command fails after writing them, so the open tab reloads
- Synth stack mode ignored Save star catalog (.csv) and Save ground truth; it now writes `{prefix}_catalog.csv` (frame 0 positions) and `{prefix}_groundtruth.fits` (frame 0) when they are on (`generate_synth_stack_cmd` takes `save_catalog` and `save_ground_truth`, default false)
- The flat's pixel-response pattern changed on every synth stack frame; one flat is computed per stack and used by every frame
- Synth stacks of seeds 7919 apart shared their noise realisations (frame i of seed s used the noise seed of frame 0 of seed s + 7919 × i)
- The Synth result card showed the live slider size instead of the generated one; it now shows the width and height of the generated image and says "Settings changed since generation" when a parameter changes afterwards

## [0.6.5.0] - 2026-10-02

### Added
- The Stack-tab Pipeline writes its stacked masters to the output folder (one FITS and one auto-STF PNG per channel, plus the RGB quick-look PNG) and returns their paths (`run_pipeline_cmd` now takes `outputDir` and `name`; the response adds `masters`, `rgb_png_path` and `rgb_dimensions` and keeps the base64 previews)
- Compose wizard Align preview: after Align the step shows a colour overlay of the aligned channels (reference in red; with two channels the second one in cyan, with three the others in green and blue; with more than three, G and B pickers choose the two shown) with an After/Before toggle and a blink of each channel against the reference; aligned stars come out white, residual offsets as coloured fringes and missing data as a dark checkerboard; the overlay zooms and pans, and a 1:1 button shows one preview pixel per screen pixel so small offsets stay visible on large frames; the offsets read `Δx`/`Δy` in pixels and the phase-correlation confidence reads `SNR` (`channel_overlay_preview_cmd`, which writes PNGs only and leaves the composite untouched)
- Compose wizard Crop editor: the step shows the aligned channels with a draggable, resizable crop box synced to the four margin fields (arrow keys move 1 px, Shift+arrow 10 px), pre-filled from auto-detection, with an Auto button to restore it; Apply crops to the margins shown (`detect_crop_bounds_cmd`, which shares the auto-crop bounds with `crop_channels_cmd`)
- Dockable tool windows (IntelliJ-style): drag any tool icon to one of the four strip groups (Left Top and Bottom Left on the left strip, Right Top and Bottom Right on the right strip) or to another position inside its group; during the drag a blue rectangle shows where the panel will open, a label next to the pointer reads "Move to Bottom Left" etc. and a marker shows the insertion point; Escape or a release away from the strips cancels, and keyboard shortcuts are ignored until the drop; right-click on an icon, or Shift+F10 / the Menu key on a focused icon, opens a menu with Move to Left Top / Bottom Left / Right Top / Bottom Right, Move up, Move down and Reset layout (arrow keys, Home/End, Enter, Escape); a moved panel keeps its contents, state and scroll position, and a tool dropped on a group whose panel is open opens in its place; Reset layout (also "Reset Layout" in the command palette) restores the default groups, order and sizes with Files and Compose open and every other panel closed; the layout (groups, order, open panels and sizes) persists across sessions in `localStorage` (`ab.layout.dock.v1`), and the splitter sizes saved by earlier versions are carried over
- The bottom area under the viewer holds two tools side by side, one docked at Bottom Left and one at Bottom Right, with a splitter between them (25-75 % of the width; each half keeps its tool's minimum width while the area is wide enough, and narrower contents scroll inside their half); a double click on any splitter resets that size
- Compose wizard dependency warnings: the Align run button and Crop's Apply/Skip list the later results a re-run discards ("Running Align discards: Crop, BG, Blend", "Apply or Skip discards: BG, Blend"); BG shows a notice when two or more channels are filled and none is aligned; Blend's "never aligned" warning and amber stage pill (tooltip "not aligned") now cover every filled channel without an Align result, including one that went through BG without Align, instead of only raw and stacked channels
- JWST pipeline x1d next to the cube spectrum: the Spectroscopy comparison section has "Compare with pipeline x1d" (the `_x1d.fits` next to the cube) and "Pick x1d…" (any FITS holding an `EXTRACT1D` table, with or without the `_x1d` suffix) plus an EXTVER select when the file holds more than one `EXTRACT1D` table, and plots the table as one more entry, labelled for example "x1d (nrs1, G235H/F170LP)", on the cube's channel grid: rows are resampled linearly in vacuum µm and never across a DQ-flagged row, the entry reads "N rows · M DQ rows dropped", FLUX (Jy) is plotted in the sum and Jy views and SURF_BRIGHT in the mean view, and a note says the pipeline aperture differs from the region; the joined CSV gains `x1d_flux_jy` and `x1d_flux_err_jy` columns and a `# x1d:` provenance line, the measurement log records `x1d_path`, `x1d_hdu` and `x1d_dq_rows_dropped`, and a load in progress can be stopped with ×; in the Files list an x1d row now reads "1D spectrum table (EXTRACT1D): open it from the Spectroscopy panel of its cube" with an "Open in Spectroscopy" button, disabled with "load <stem>_s3d.fits first" until its `_s3d` cube is loaded, which selects the cube, opens Analysis and queues the comparison (`read_x1d_spectrum_cmd`, which reads only the `EXTRACT1D` data block through a new BINTABLE column reader and refuses a table whose header declares more data than the file holds, a `TFIELDS` outside 0-999 or a requested column name that appears twice)
- Wavelength readout from a `WAVELENGTH` extension (NIRSpec `_cal` frames): when the image has a same-size `WAVELENGTH` image HDU, the status strip shows the wavelength at the cursor ("λ 1.6577 um", unit from that HDU's `BUNIT`, `um` without one, nothing on a NaN pixel), the pixel table's new plane select (SCI, ERR, WAVELENGTH, replacing the "Show ERR" toggle; a plane the file lacks is disabled) shows the λ grid at six significant digits and copies it as CSV at full precision, every cell tooltip gains a λ line, and logged pixel entries carry `wavelength` and `wavelength_unit` (`probe_pixel_cmd` returns `wavelength: {value, unit}`, `pixel_table_cmd` returns `wavelength`, `wavelength_stats` and `wavelength_unit`, and the server `POST /v2/sessions/:sid/pixel` returns `wavelength`, each `null` without the plane)
- Exclude regions: circles, ellipses, boxes, annuli and polygons have an `incl`/`excl` toggle in the Regions panel (the DS9 `-` prefix, kept by `.reg` import and export), and the pixels inside every exclude region are left out of the other regions' statistics and background annuli and of radial profiles, line cuts and surface-brightness profiles, merged with the DQ mask when DQ exclusion is on; the `n` cell's `(−N dq)` became `(−N excl)` and counts both causes, the Regions and Profiles panels show a "regions excluded" badge, an exclude region's own row is measured on its own pixels (its log row says so in a note), the background-annulus list offers only include annuli (an exclude annulus already chosen as the background stays listed), and region-statistics and profile log rows record `region_excluded_px` (profile rows also `exclude_regions`, the number of exclude shapes sent) (`region_stats_cmd`, `radial_profile_cmd`, `line_cut_cmd` and `sb_profile_cmd` take `exclude`, at most 512 shapes, and return `region_excluded`, the profiles also `dq_excluded`; an invalid exclude shape such as a zero radius or a polygon with fewer than three points is skipped and reported in `notes` as "N exclude regions skipped: invalid shape" instead of failing the call)
- Colorbar under the mono GPU viewer: a strip of the current colormap (honouring invert) with value ticks placed through the current stretch and limits (under MTF, log and the other non-linear stretches the ticks sit at rounded values spread over the whole bar, with the ends at vmin and vmax), the `BUNIT` unit (none on a processed result), a no-data swatch, a centre tick when symmetric limits were applied, a dimmed state while new limits are computed (except under MTF) and the value under the pointer on hover; a "colorbar" checkbox in the display bar hides it (remembered across sessions; Reset display turns it back on), and it is not shown on the CPU viewer or for RGB and composite views
- Compose Blend: Match levels (on by default when a narrowband filter is loaded, from a narrowband bin, a header filter code or `PUPIL`) measures the median and the 99.5th percentile of each used channel one at a time through `compute_scale_limits_cmd` (the Run button reads "Measuring levels 1/3"), multiplies each channel's blend weights by the largest signal (p99.5 minus median) over its own, so every scale is at least 1, shows the scales in a "Level match:" line, writes one `Level match` HISTORY line per channel on FITS export, and refuses with a message when a channel has no signal above the sky; SPCC factors are divided by the scale of the channel feeding each plane and follow a later Blend that changes the scales, and when the divided factors would leave the 0.01-100 apply range all three are multiplied by one common factor (colour ratios unchanged), which the note under SPCC states; with Match levels off the `blend_channels_cmd` payload is unchanged, and a new channel set restores the default
- Plate Solve: "Write WCS to image" writes `<name>_wcs.fits` with the astrometry.net TAN-SIP solution from the job's `wcs_file` (rescaled to the original grid when the upload was downsampled), keeping the pixels, units and calibration cards, replacing every old celestial WCS card (CD, PC, CDELT, CROTA, SIP, PV) and setting `ABPROC='platesolved'`, and shows it in the viewer as "Plate-solved WCS", so the grid, compass, Targets, photometry and the Gaia catalog search and cross-match use it; the file goes to the app's output folder, which is emptied at every start, so use Export to keep it; the button is disabled before a solve and when astrometry.net returned no WCS file, and a multi-plane image or cards without a usable celestial WCS are refused with nothing written (`write_solved_wcs_cmd`; `plate_solve_cmd` returns `wcs_cards`)

### Changed
- Info is a docked panel in the Left Top group: opening it replaces Files in the left column (and opening Files replaces Info) instead of floating over the file list, it takes the full column height, and Escape no longer closes it
- Synth, Export and Settings open at Bottom Right, in the bottom area next to Compose, instead of in the right column; opening a second bottom tool halves the bottom area
- Strip layout: Files and Info sit at the top of the left strip and Comp at its bottom (the divider between them is gone); the right strip is shown even without a selected file, with the tools that need a file dimmed: they can still be moved, and their panels open once a file is selected
- New Batch no longer reopens Compose or closes Info; the panel layout is left as it is
- The viewer-minimum clamps (applied when a window resize or a newly opened column or bottom tool would leave the viewer under 320 × 200 px) no longer overwrite the saved panel sizes; only dragging a splitter or double-clicking it saves a size
- The left column can be widened to 640 px (was 480); a side column is capped at 60 % of the window width, or 40 % when both side columns are open, and is never narrower than its tool's minimum (180 px for Files and Info, 280 px for the other tools)
- The command palette has one "Open … Panel" / "Hide … Panel" action per tool, Files, Compose and Info included (replacing their "Show/Hide Files, Compose and Info Panel" actions), plus "Reset Layout"
- In a short bottom panel the file list keeps at least one row visible and the panel scrolls instead; the Settings panel has inner padding; the Align step's preview column narrows to 240 px so it fits a 280 px column
- Compose wizard: a channel with two or more frames (after Stack's exclusions) must be stacked, or reduced to one file, before Align, BG, Blend, Color, Stretch and Export open; their pills say so ("stack Hα and OIII first (2+ frames each), or keep one file per channel"), and a channel whose frames are all excluded locks them with "all Hα frames are excluded; re-include one in Stack"; Color, Stretch and Export stay open once a composite exists
- Publishing a Stack-tab result to the selected file while the Compose composite is on screen parks the composite, the same way "Back to file" does, so the result is visible; "Show composite" brings the composite back
- Star rings, labels and plate-solve annotations keep a readable stroke and label size when the viewer is zoomed out, and redraw when the zoom changes
- Star Detection shows BG, σ and flux with significant digits instead of fixed decimals, and its "composite" source title says detection uses the linear composite planes, not the display stretch or curves
- The Compose wizard Crop step no longer has the "Auto-detect borders"/"Manual margins" select: Apply always crops every channel to the box shown, which starts at the auto-detected borders, so channels with no region valid in every channel now crop to the full grid instead of failing with "Auto-crop found no valid overlapping region"
- In CPU mode a click on a cube frame with the pan tool now extracts its spectrum, as it already did in GPU mode, and the CPU viewer shows the same "Click to extract spectrum" hint on cubes
- Export FITS with "Apply current STF stretch" writes a display-referred header: `ABPROC='stf_export'`, `ABDISP=T`, no `BUNIT`, `PHOT*`, `PIXAR_SR`, zero-point or saturation cards, the WCS kept when Copy WCS (coordinates) is on (the default), and two `HISTORY` cards with the STF parameters and "pixel values are display-referred in [0,1]", so region fluxes and photometry report the file as uncalibrated; `export_fits` returns `display_referred`, read from the header actually written (also true for a copy of an LHE, HDRMT or PixelMath result), and the Export panel shows it as a Values cell ("display-referred (ABDISP=T)" or "linear"); with STF off the file is written as before
- Compose Color step: SCNR is no longer switched on automatically when SPCC is unavailable for the loaded filters, and SPCC is refused for medium-band filters (an `F…M` code) and for filters outside 380-830 nm (the codes are read from `FILTER`, `FILTER1`, `FILTER2`, `FILTNAM1`, `FILTNAM2`, `PUPIL` and the file name, so a narrowband `PUPIL` such as F470N is caught too), with a reason that names the file, the filter and its wavelength ("SPCC models visible light (380-830 nm) only; f444w in R is F444W at 4440 nm."); a narrowband input is still reported first; the Auto WB error suggests SPCC only when it can run
- Stacking with alignment (also the Compose Stack step, which always aligns), the Pipeline with alignment (the default) and drizzle, including Drizzle RGB, refuse one-shot-colour lights (`BAYERPAT` or `COLORTYP` in the header) before any pixel is loaded, with a message that names the count and the pattern and says how to proceed: calibrate, debayer (Processing > Debayer > Debayer All Loaded) and stack or drizzle the `_R`, `_G` and `_B` files as separate channels (the stacking and Pipeline message also offers turning alignment off for undithered frames); the headless server's stack, drizzle and pipeline routes refuse them the same way (400) before queuing a job
- Narrowband composites (for example the HST SHO sample) now blend level-matched by default; turn Match levels off in Blend for the previous look
- The Catalog panel searches Gaia DR3 and cross-matches on the image on screen, as Photometry and Targets already did, so after Write WCS, or with any processed result that has a FITS, it uses that file's WCS; its results are cleared when the image on screen changes, and the cross-match log row keeps the loaded file's name

### Removed
- The floating Info popover (Info is now a docked panel, see Changed)

### Fixed
- The Pipeline result never reached the central viewer: completing a run now shows "Pipeline RGB" (or the first master) on the file selected when the run started, and the R, G, B and RGB result pills show that output in the viewer, the channels as FITS with display controls; a failed re-run no longer leaves the previous run's pills active, and the in-panel RGB preview uses the RGB buffer's own size
- Detect Stars on the colour composite (Compose composite or an RGB FITS view, GPU and CPU) drew no rings or labels because the composite view had no overlay layer
- Star detection on the colour composite normalised the luminance by its min/max, which turned the exact-zero mosaic padding into fake sky, biased the background and σ low and roughly doubled the detections; the luminance now keeps the planes' native units, padding stays excluded, and mismatched channel sizes return an error instead of panicking
- Composite star results were kept after Blend, Crop or a composite step replaced the composite, and plate-solve labels from the selected file's grid would have been drawn on a Compose composite with a different grid
- The Linux AppImage failed the AppImage catalog test: Tauri CLI 2.10.1 wrote `.DirIcon` as an absolute symlink into the build machine's AppDir, which dangles once the AppImage is mounted, and saved `AppRun.wrapped` as mode 0770, so the app could not start for any user other than the one who mounted it (firejail, root-mounted extractions); `@tauri-apps/cli` is now 2.12.0, which writes a relative `.DirIcon` and a world-executable `AppRun.wrapped` (the `.deb` contents are unchanged)
- Cube frames and other PNG-only results (Debayer preview, Drizzle and Pipeline RGB) were hard to use in GPU mode, where they fell back to a shrink-to-fit image with no zoom, pan or regions, so small cube channels stayed tiny (the same image also stood in for any FITS while its GPU pixels loaded or after they failed to load); whenever the GPU display is not on screen they now open in the CPU viewer with zoom, pan, regions and overlays, a click on a cube frame still extracts its spectrum, and the GPU toggle and display controls say why the GPU display is not shown (PNG-only result, GPU check still running, GPU image still loading or failed to load)
- Zooming out of a colour preview larger than four times the viewer jumped to 25 % instead of stopping at the fit
- A quick double click on the zoom buttons of a colour preview also fired the viewer's double-click zoom, so two fast clicks on zoom-in jumped to 300 % or snapped back to the fit
- Run buttons with a cyan, rose, blue or purple accent had no background and read as plain text, because the stylesheet styled only six accents: Apply Crop, Apply Color Balance and Apply Curves in the Compose wizard, Stack All, Search Gaia DR3, Place (Targets), Run SPCC, Run Masked Stretch, Drizzle RGB and Synth Generate now show their accent background
- Cyan, blue and rose toggles and cyan and rose sliders showed the default teal, the Masked Stretch and HDRMT "Using output from" banners had no accent colour, and the Crop step's margin fields had no input style (they used an `ab-input` class that was never defined)
- A Compose channel whose first file was excluded in Stack's frame selection still sent that file to Align, BG and Blend and to the exported header; the first file that is not excluded is used, changing the exclusions clears the results built on the previous input, and re-assigning a channel's files drops its exclusions
- An Align that finished after its inputs changed (re-stack, re-assignment, exclusion change or Reset during the run) stored its stale result; it is now discarded with "Inputs changed while aligning; run Align again", and a Reset during a run leaves no "Aligning…" state behind
- The Crop step loaded, previewed and cropped the channels while an Align was replacing them; it now shows "Align is running… Crop reloads when it finishes." and reloads when Align ends
- Crop after the aligned channels had left memory failed with an operating-system "file not found" error; it now says "Aligned channels are no longer in memory; run Align again", like the border detection
- Compose Export without a blended composite no longer clears a composite built elsewhere (an RGB file view or the Processing tab)
- The Stack-tab Pipeline and Drizzle RGB ignored the Subframe Selector; after Apply Selection they now leave out the rejected frames (Pipeline rows read "N files (M excluded)", Drizzle RGB rows are tagged "culled", and the counters, the "Run Pipeline (N lights)" button and the channels Drizzle RGB treats as ready use only the kept frames) and list them ("M excluded by Subframe Selector: " and up to five file names) with a "Use all frames" action that lifts the cull in that panel only, keeping the Stack section's weights and rejected tags and the other panel's cull; a new Apply culls both again, and any change to the loaded file list forgets the selection everywhere
- A plate-solve reply without RA, Dec or a positive pixel scale is reported as an error ("astrometry.net calibration for job N has no usable ra; the solve cannot be used") instead of a solve at RA = Dec = 0

## [0.6.4] - 2026-09-26

### Added
- Processing-tab composite mode: while the colour composite built in Compose is on screen, Background (polynomial and spline), Denoise, PSF, Deconvolution, Stretch, Masked Stretch, LHE and HDRMT process its three channels instead of the selected mono file, update the view, and show colour before/after previews (per-channel numbers as `R · G · B`); the composite has its own processing chain with the same rules as the file chain (re-running a step drops the later ones), a "Revert to original" that restores the composite exactly as it was before the first step, and a restart notice when Blend or a wizard step replaced the composite meanwhile (`cmd::compose::composite_chain`, `composite_{background,wavelet_denoise,deconvolve_rl,estimate_psf}_cmd`, `pixelmath_composite_cmd`, `composite_chain_{reset,state}_cmd`, optional `chainInput`/`displayStf` on the arcsinh, masked, LHE and HDRMT composite commands)
- PixelMath target selector while the composite is on screen: "File" runs on the selected file and says the composite does not change, "Composite (per channel)" evaluates the expression on R, G and B with `$T` bound to each channel
- "Show composite" button in the preview header: selecting another file or "Back to file" now parks the Compose composite with its STF, and the button brings it back (an `rgb_composite_*` preview older than the 600 s sweep is re-rendered)

### Changed
- Masked Stretch protection follows each pixel's brightness above the pass background inside the star mask and is applied over log-spaced steps towards the target background; Iterations sets the number of steps again (more steps protect stars more), the result reports the passes applied and whether the target was reached, and an input already at or above the target is left unchanged
- Deconvolution with "Empirical PSF" uses the kernel estimated in the PSF tab when there is one, on files and on the composite (`psfKernel`, validated and normalised), results report `psf_source`, and the chain indicator shows "PSF" before Deconv only when the kernel was used
- The wizard Stretch "Link channels" toggle reads and writes the composite linked flag, so later renders from linear data and the STF panel follow the same choice
- Removed the Headers "Assign" channel sync into the composite (`update_composite_channel_cmd`, `replace_composite_channel`), unreachable since composite mode; the Headers assignment still drives RGB export

### Fixed
- Processing panels showed grayscale previews of the selected file while the colour composite was on screen, and their steps never reached a composite built in Compose (the channel sync only ran for files assigned in Headers)
- Masked Stretch turned every detected star and the sky around it into a dark disc, because protected pixels kept 85% of their linear value; the discs reached the FITS output, PixelMath and later steps, on files, the composite and the wizard
- Richardson-Lucy on data without negative pixels treated mosaic padding as zero flux and left rims along the padding, and wavelet Denoise leaked values into the padding; both now fill the padding with the median of the valid pixels and restore it to exactly 0
- Selecting another file reset the composite STF to identity while the Compose composite stayed ready, so later composite renders came out nearly black
- Processing panels stayed enabled on an RGB FITS and always failed with a 3D-cube error; they are disabled with the reason, and LHE/HDRMT no longer process a leftover Compose composite from an RGB file's view
- An RGB FITS reloaded in place was shown as the Compose composite

## [0.6.3] - 2026-09-25

### Added
- **Science round 2**
  - Rest-frame line list (`src/utils/lineList.ts`, `LineListControls` in the Spectroscopy panel): 23 built-in lines (Balmer, Paschen and Brackett hydrogen lines, [OII], [OIII], [NII], [SII], He I, H2 1-0 S(1), [FeII], CO 1-0/2-1/3-2, HCN, HCO+, [CII] 158 um, HI 21 cm; vacuum um, radio lines as rest GHz) drawn on the spectrum plot at a typed redshift or systemic velocity in every axis mode, in the frame of the velocity axis, with family toggles and thinned labels; a click on a line sets the rest wavelength of the line measurement
  - Spectrum export and comparison (`src/utils/spectrumExport.ts`, `spectrumCompare.ts`, `useSpectrumComparisonStore`, `SpectrumComparisonSection` in the Spectroscopy panel): Copy/Save CSV of the displayed pixel or region spectrum with provenance lines (file, source, ICRS centre, stored and applied velocity frame with shift, flux unit) and columns for channel, vacuum wavelength, the displayed axis, flux, flux unit and flux in Jy (`get_cube_spectrum` now returns `flux_jy` for MJy/sr cubes); a comparison section that plots every included area region (the first 16) and the last clicked pixel together, normalised to peak, median or the continuum windows or offset, with a legend and a joined CSV on the common axis
  - Observation geometry (`observation_geometry_cmd`, `core::astrometry::geometry`, `cmd::geometry`, Observation geometry panel): leap-second table with UTC/TT/TDB, BJD_TDB and HJD_UTC light-time corrections (Meeus low-precision Earth and giant-planet solar reflex, within 1 s of astropy), local sidereal time, hour angle, unrefracted altitude and azimuth on the mean equinox of date, Kasten-Young airmass, parallactic angle, Sun and topocentric Moon altitude, Moon illumination and separation; site and target overrides in the panel (the site is remembered across files and applied ahead of every header site, with the replaced header site named in the notes); the time series carries a per-frame `geometry` block and a BJD_TDB light-curve axis
  - Position-velocity diagram (`pv_diagram_cmd`, `core::cube::pv`, `cmd::pv`, PV panel): bilinear sampling of a cube along a line region with a step and an averaging width, offsets centred on the slit in arcsec from the CD matrix (pixels without WCS), a median-subtracted intensity-weighted ridge with the peak channel per offset, written as a derived FITS with `CTYPE1='OFFSET'` and the native spectral axis plus `PVLINE*` provenance, shown in the viewer with the display controls and plotted in the panel with a CSV export
  - Diverging colormaps and symmetric limits (`core::imaging::colormap`, `core::imaging::scale`, display bar): `rdbu`, `coolwarm`, `bwr` and `seismic` (matplotlib knots, shown as RdBu, coolwarm, bwr, seismic) in the desktop LUT texture, the worker fallback and the server `/v2` render, `Colormap::ALL` has 13 entries and `get_colormap_lut_cmd` returns `nodata`; `compute_scale_limits_cmd` and the server `scale` block accept `symmetric` and `centre` (alias `center`, default 0, refused beyond +-1e38): after the chosen algorithm resolves (lo, hi) the pair becomes (c - a, c + a) with a = max(|lo - c|, |hi - c|), a degenerate pair falls back to a half-width of max(1, |c| * 2^-20) with a note, display-referred images skip it with a note, and the desktop reply and the server's resolved JSON echo `symmetric`, `centre` and `notes`; a symmetric checkbox and centre input in the display bar (enabling symmetric under `mtf` switches the stretch to `linear`; the centre lands on the colormap centre only under a linear stretch, and both the display bar and the server's `resolved.notes` say so)
  - Measurement log (`src/utils/measurementLog.ts`, `useMeasurementLog`, Measurement log panel at the end of the Analysis tab): every photometry click, batch run, line measurement, time series, cross-match, spectrum export, comparison CSV and PV run is logged with its timestamp (UTC), file, measured image, DQ handling, unit, parameters, values and region; region statistics, profiles, sky separation, the pixel table and the statistics panel log on request; the log survives file switches, filters by kind or file, and copies or saves as a wide CSV
- Regions panel: a "Delete points (N)" header button removes every Point region of the current file at once after an inline confirmation, in one store commit that also clears the selection and any background reference to a removed point, so points added from the photometry table, the target list or a catalogue no longer have to be deleted one by one
- Histogram "Full | Sky" toggle in the Analysis tab: Sky bins median − 5σ to median + 50σ through the new optional `lo`/`hi` window of `compute_histogram`, and the S/M/H markers map through the window and sit pinned on its edge when outside it
- Status strip under both viewers with the cursor's pixel position, the pixel value with its unit and the ICRS RA/Dec; Esc closes the Info popover, which stays inside the Files column while the sidebar is open
- Sticky row of chips at the top of the Analysis column that jump to each panel
- Press-and-hold "Original" button in the GPU viewer that shows the loaded file under the same zoom and pan while a processed result is on screen, and a Compare wipe between the original and the corrected image in Background Extraction
- Frame progress, elapsed time and a cancel button in the Compose Stack step; a cancellation (also from the Stack tab) shows "Stacking cancelled." instead of an error and stops Stack All
- "Show apertures" toggle and "Clear" button in the Photometry table; Clear removes the table and its aperture overlay and "star N" labels from the image

### Changed
- `time_series_photometry_cmd` accepts optional `target_ra`, `target_dec`, `site_lat`, `site_lon` and `site_height` overrides; barycentric headers (`TIMEREF = SOLARSYSTEM`, `BJDREF*`) are reported as BJD_TDB without re-correction; `header_target_coordinates` also reads `RA_OBJ`/`DEC_OBJ`; the light curve uses BJD_TDB as its default time axis when every frame has one, and its CSV gains `bjd_tdb`, `hjd_utc`, `airmass_computed`, `altitude_deg` and `parallactic_angle_deg` columns beside the header `airmass`
- No-data pixels (NaN, infinity, exact 0) take a per-colormap no-data colour that ignores `invert`: the first LUT entry for the nine sequential maps and neutral grey `#404040` for the diverging maps; the desktop shader, the worker fallback and the server PNG now agree (the desktop used to paint inverted padding with the last LUT entry); `astro-image-api.md` states that `mask.nan_color` is not implemented
- Click photometry no longer queries Gaia DR3 by default (the choice is remembered), shows the local measurement at once, and reports a failed query, an empty cone or a missing WCS as warnings instead of a silently missing Gaia box
- Compose Align tags a channel left unshifted after a low-confidence match as "not registered" with a suggestion to try the other method, and names a fallback method ("via rigid"); `align_channels_cmd` reports `registered` on every channel
- Export tab errors and the "Saved" row appear under the section whose button produced them, and a failed cutout clears the previous cutout result
- Pan and Crosshair are two always-visible viewer toolbar buttons, 1:1 is labelled "1:1" so it no longer looks like the box region tool, and the Photometry and Pixel table hints say "Select Crosshair in the viewer toolbar"
- The preview header's "Reset" is now "Revert to original" and the display bar's is "Reset display", so the two buttons no longer look alike
- Aperture, annulus and catalogue-radius inputs use short placeholders with full tooltips and a caption stating the defaults (r_ap = 1.5 × FWHM, sky 2–3 × r_ap, half the image diagonal)
- Pixel coordinates are marked 0-based in the hover readouts, region row tooltips, the pixel table, the photometry centroid, the photometry table and the star detection table, with a note that DS9 and .reg files are 1-based
- Every robust sigma (Info panel, histogram, region rows, Compose info bar) reads σ(MAD) with the tooltip "1.4826 × MAD (robust)"
- Small images open fitted to the viewport (up to 16x), and every viewer shows crisp pixels at any zoom above 1x
- Ticking grid or compass on an image whose WCS cannot draw them shows an amber "(no WCS)" note in the display bar with the reason in its tooltip
- Wheel and touchpad zoom follow the size of the wheel delta (about 15% per mouse notch), so gentle touchpad gestures no longer race through the zoom range
- Hovering a locked Compose step names what it is waiting for (for example "needs a channel with 2+ frames", "run Align first")
- Config's "Clean up" is disabled under the size cap ("under the size cap, nothing to remove") instead of asking to confirm a deletion it would not perform
- HST WFPC2 frames show their filter (FILTNAM1/FILTNAM2, for example F656N) in the Files list, the search, the palette and the Compose channel chips instead of nothing or the filter-wheel number; a FILTER holding a central wavelength (656.3, 5007) is still shown and auto-mapped
- The command palette finds the Settings tool by "Config" and has a "Show Info Panel" / "Hide Info Panel" action
- The command palette and the keyboard shortcut sheet are modal dialogs for assistive technology: focus stays inside them and returns where it was when they close, the palette announces the selected row, and the sheet takes keyboard focus so keys no longer reach the control behind it
- The Compose bin Select menu opens in a floating layer that stays inside the window, so the right-hand bins are no longer clipped, and it is keyboard-operable (arrow keys, Home, End, Escape, Tab)

### Fixed
- Deep Zoom opened as a black column instead of a full-screen viewer: the overlay was trapped by the identity transform the tool column keeps after its fade-in animation, so it took the size of the scrolled Analysis column with the image and its controls off-screen; it is now rendered at the document body, the panel header shows the image size as `W×H` instead of a literal escape sequence, and the viewer closes when the image on screen changes
- SPCC failed on compose-wizard channels with `Failed to open __wizard_ch_r_bg (os error 2)`: aligned, cropped and background-corrected wizard channels now carry the FITS header that matches their pixel grid (the reference grid after Align, CRPIX moved by the crop origin, rescaled on resample), SPCC reads the WCS from the channel itself, and a channel without a celestial WCS is refused with a message naming the channel and asking for a plate-solved image
- Synth stack mode no longer runs out of memory and closes the app: each frame is written to disk as soon as it is rendered, a stack above 1,073,741,824 pixels (width × height × frames) is refused before anything is generated, and the panel shows the estimated disk size next to Frames and disables Generate with the reason when the stack is over that budget
- SPCC in the Compose Color step is disabled as "SPCC (broadband RGB only)", with a one-line reason, when an input would come from the Hα, OIII or SII bin or a frame in the R, G or B bin is narrowband (header filter code, backend filter detection or file name), instead of applying meaningless factors to the composite
- The Compose composite FITS export keeps the WCS of a background-corrected or cropped wizard channel, rescaled to the composite grid; `export_fits_rgb` reports `wcs_written` and the Export step warns when no celestial WCS was written
- Stretch (arcsinh, GHS, masked) and Star Removal outputs of a wizard channel held in memory keep the channel's WCS and observation cards (DATE-OBS, FILTER, OBJECT) instead of a bare header
- The SPCC result (factors, stars, catalogue and synthetic-catalogue warning) stays on screen after a run, the mode stays on SPCC and the note says to click Apply Color Balance; when the Gaia DR3 query fails, the Catalog row says the built-in Bp-Rp fallback was used and why
- Star Detection shows "200 of N (brightest)" when only the 200 brightest sources were kept, and the Photometry table source reads "Detected stars (200 of N)"; `detect_stars` and `detect_stars_composite` report `n_detected`, the count before the cap
- The Header Explorer lists every card in header order, including all repeated HISTORY and COMMENT cards under Processing, shows the categories in a fixed order, shows "N of M" only while searching, and copies commentary cards as `HISTORY <text>` without "="
- Leaving the Analysis tool no longer discards its results: Analysis stays mounted for the current file while another tool is shown or the column is closed, so overlays, statistics, photometry and the light curve survive and a running time series is not cancelled; while hidden, Photometry and the Pixel table ignore image clicks and the cursor, and Deep Zoom closes
- "Back to file" no longer deletes the wizard's colour composite, so Color, Stretch, Adjust and Export keep working afterwards; when the blended planes are gone, composite commands say "The colour composite is no longer in memory; run Blend again." instead of naming internal cache keys
- Plate solve pre-fills the scale range from the header pixel scale (0.8x to 1.25x, arcsec/px); the scale fields keep what is typed and flag an empty, non-positive or inverted range inline instead of snapping it back
- After a region is drawn and the tool returns to Select, a click on empty sky reaches the viewer again (click photometry, pixel table, cube spectrum); only clicks on a region or a handle are kept by the region layer
- A failed WebGPU probe falls back to the interactive CPU viewer for the session without changing the saved GPU preference, and a failed "click to retry" no longer saves GPU as preferred
- The CPU viewer shows the display bar: grid and compass work, stretch, limits, colormap, invert and symmetric are disabled with "needs GPU rendering", and the Analysis histogram STF is no longer locked by a saved non-mtf stretch while the GPU display is not on screen
- Zoom %, the zoom presets and 1:1 are measured in FITS pixels instead of preview-texture pixels, and a "preview N px (r:1)" badge pointing to Deep Zoom appears when the viewer shows a downsampled texture
- Pinning a Files search as a chip matches the same name, filter and instrument text as the search, so a "wfpc2" or "F200W" chip no longer empties the file list, the preview, the palette file list and the ZIP export
- "New Batch" in the command palette arms the same "Discard N processed files?" confirmation as the footer button instead of discarding the session at once
- Compose "Reset Wizard" asks "Reset all steps?" and needs a second click within 4 s when any frame is assigned or any step is complete
- The measurement log's Clear asks inline ("Clear N rows?", or "Clear all N rows, not only the M shown?" under a filter) with Confirm and Cancel and reverts after 5 s, and the panel says the log is kept for this session only
- The viewer toolbar wraps instead of clipping the zoom presets and region tools in narrow layouts; the duplicate "Reset View" button is gone and the zoom readout comes before the presets
- The FFT panel drops a spectrum that arrives after a file or view change, labels "(from W×H)" per axis and only when the spectrum was downsampled, and shows the hover frequency in cyc/px; the `compute_fft_spectrum` header carries the padded size per axis and a downsampled flag
- Statistics dims the table, disables CSV and says "Settings or region changed: press Compute" when the region or the options change, and turns "Selected region only" off when the selection disappears
- The histogram footer, the FFT footer and the Synth Airy slider show μ, σ, × and λ/D instead of literal `\u` escapes; a test scans every component for such escapes
- The CPU viewer reports the hovered pixel in pan mode too, so Info, RA/Dec and the Pixel table follow the cursor as in the GPU viewer
- Drop, the file dialog and Select Folder share one file check; a folder with no loadable files at its top level, or a folder that cannot be read, shows a banner saying why and no longer switches to an empty workspace
- The Compose Export step stays locked until a channel is assigned or a composite exists, a new export clears the previous "Export complete" box, and the disabled ZIP button looks disabled
- Compose Stretch disables Auto STF without a blended composite and says what it needs instead of promising a per-channel STF
- Observation geometry shows the LST as "197.6932 deg (13:10:46.4 h)" instead of labelling the sexagesimal hours as degrees
- Counts in Synth, Stacking, Drizzle and the calibration pipeline use a locale-independent thousands separator
- Synth stack mode picks its output folder with a folder dialog and writes the frames straight into it; an existing file or a folder whose parent does not exist is refused before anything is generated
- After the first Align, the Compose next-step hint points to Crop instead of BG

## [0.6.2] - 2026-09-25

### Added
- **Science measurement features**
  - Interactive `ProfilePlot`: hover readout, wheel zoom and drag pan on x, log and inverted y, points, error bars, dashed model curves, labelled reference lines, CSV and PNG export; cross-match residual and zero-point plots in the Gaia catalog panel
  - Aperture photometry controls (aperture radius, sky annulus, gain), curve of growth with EE50/EE80 radii (`PhotometryConfig.sky_annulus`, `StarPhotometry.growth_curve`, `ee50_radius`, `ee80_radius`), and `measure_photometry_batch_cmd` behind a Photometry table panel that measures detected stars, Point regions or pasted positions, draws the apertures on the image and exports CSV
  - Calibrated region statistics: every region reports flux in Jy, AB magnitude, surface brightness in mag/arcsec^2, area in arcsec^2, RA/Dec of the centre and the sky position angle (`RegionStats.calibrated`, `entry_calibration`), with sigma-clipping controls and a region CSV in the Regions panel
  - Sky geometry: `world_to_pixel_cmd`, `sky_separation_cmd` (separation and position angle east of north), `WcsTransform::orientation` (rotation, parity, per-axis scale, projection, SIP, north and east vectors) reported by `get_wcs_info` and the server `wcs` route; a Targets panel that overlays a pasted or CSV RA/Dec list; separation and position angle for line regions; a compass and scale bar overlay
  - Spectral line measurement over the brushed range (`measure_spectral_line_cmd`): flux, equivalent width, centroid, sigma/FWHM, SNR, velocities, an optional Gaussian fit with formal errors from the new bounded Gauss-Newton fitter (`math::gauss_newton`), and the continuum and model drawn on the spectrum
  - Contour overlay (`contour_lines_cmd`, `core::imaging::contour`): marching squares with NaN-aware binning and smoothing, list/linear/log/sqrt/sigma levels, per-level visibility, export of closed contours to Polygon regions
  - Elliptical surface-brightness profile (`sb_profile_cmd`, `elliptical_profile`): mag/arcsec^2 against semi-major axis in arcsec, R50/R80/R90, Petrosian radius, total AB magnitude, in the Radial profile panel
  - Time-series aperture photometry across loaded frames (`time_series_photometry_cmd`) with drift tracking, a differential light curve against a comparison ensemble, per-frame table and CSV; the synthetic stack writes DATE-OBS and MJD-OBS with a cadence
  - Pixel table panel (`pixel_table_cmd`): N x N raw values with ERR and decoded DQ flag names, on click or following the cursor
  - Spectrum-linked channel navigation: the displayed channel is marked on the spectrum, double-click and arrow keys change it, playback loops, and a committed channel is published as a FITS frame so regions, statistics, photometry and contours work on it

### Changed
- `WcsTransform::pixel_to_world` and `world_to_pixel` return NaN for non-finite input instead of reaching the projection engine

## [0.6.0-preview] - 2026-09-20

Preview of the 0.6 line: the viewer maturity phases (0 to 3), the processing-parity round (Phase 4) and the science round (Phase 5) below, plus the fixes to their review findings. The Windows bundle reports 0.6.0 because MSI versions cannot carry a pre-release identifier.

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
- The GPU viewer now shows the result of every mono processing step (background, denoise, deconvolution, stretch, masked stretch, LHE, HDRMT, PixelMath): the processed FITS becomes the raw-pixel source of the WebGPU renderer and the histogram and auto-STF are recomputed for it, instead of the GPU path keeping the original pixels while only the CPU path switched to the rendered PNG; Reset in the Processing tab returns both paths to the original image and the processed source is remembered per file like the rendered preview
- PixelMath binds image slots automatically: the loaded files other than the target become `A`, `B`, `C`, ... when the panel opens, choosing an example such as `(A + B + C) / 3` binds the symbols it references, and an "unknown symbol" error offers a one-click "Bind ... to loaded files" action instead of leaving `$T` as the only symbol
- **Review follow-ups of Phases 4 and 5**
  - Spacecraft `VELOSYS` is applied with the JWST sign convention (positive = observer receding, so the correction is `-VELOSYS`), stated in the correction notes
  - Saturation for photometry and the catalog zero point comes from `SATURATE`/`SATLEVEL`/`SATURATION`/`DATAMAX`/`MAXLIN` when present, else from the image maximum with a flat-top rule (two aperture pixels at the level), so the brightest star of an unsaturated field is no longer excluded; the source is reported
  - `aperture_pixels` is the effective pixel count (rounded sum of sub-pixel weights, close to pi r^2) instead of every partially covered pixel
  - The Gaia zero-point fit reports `n_without_colour`; an empty VizieR body is an empty result instead of an error; CSV numbers are written with the same formatting on disk and in the clipboard copy; the catalog panel's numeric inputs can be cleared and retyped
  - Drizzle (mono, RGB and the server route) applies pixel rejection before scattering (`rejection` with the shared kernel, `sigma_low`/`sigma_high`), instead of accepting and ignoring the sigma parameters
  - Calibration pipeline: optional dark-frame optimization (`dark_optimize`) finds the dark scale in [0, 3] that minimises the robust background noise (golden-section search on 1.4826 * MAD) and reports `dark_scale_min/max/mean`; an invalid cosmetic configuration fails before any master frame is built; the noise-weight hint in the Stack tab is cleared when the selection changes
  - Cosmetic auto-detect groups outlier candidates into 8-connected clusters (same-colour lattice for CFA) and flags clusters of up to 3 pixels, so adjacent hot pairs are repaired while star cores are left alone; a master dark or image whose robust scale collapses to zero falls back to the mean absolute deviation instead of flagging every pixel above the median
  - Noise evaluation on a region with fewer than 64 finite pixels returns `null` with a `noise_note` (panel, command and server) instead of a sigma of 0; the server `stats` rectangle path counts every non-finite pixel like the shape path; the Statistics panel ignores stale responses
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
