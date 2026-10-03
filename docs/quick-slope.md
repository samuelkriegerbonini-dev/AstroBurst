# Quick slope for JWST `_uncal` ramps

AstroBurst 0.6.6 opens JWST Level-1b `_uncal` files (4-D `SCI`: columns, rows, groups, integrations) as ramps and can turn one integration into a `_qslope.fits` product in DN/s. The product is a decision aid for looking at an exposure before `calwebb_detector1` has run. It is not a rate file and must not be used as one.

## What the product is not

No superbias, linearity, dark, FFT IRS2 reference-pixel correction, weighted fit, segment fitting, snowball handling, neighbour growth of saturation, per-pixel SATURATION reference, variances or ERR plane, rateints, CRDS lookup, or any claim of parity with `calwebb_detector1`. The primary header carries `ABPROC = qslope` and HISTORY cards that say so, and none of `CAL_VER`, `S_RAMP`, `DATAMODL`, `NEXTEND` or `CRDS_CTX`.

## Input the ramp code recognises

The merged primary + `SCI` header must say it is a ramp: `NAXIS = 4` with `NGROUPS` or `NINTS`, or `DATAMODL` in `Level1bModel` / `RampModel`, or a 3-D stack whose `NGROUPS` equals `NAXIS3` with no spectral `CTYPE3`. `NGROUPS x NINTS` must equal `NAXIS3 x NAXIS4`. The group time is `TGROUP`, else `TFRAME x (NFRAMES + GROUPGAP)`; without either the slope is refused (`EXPTIME`, `EXPOSURE`, `EFFINTTM` and `XPOSURE` are never read). NIRSpec IRS2 frames are recognised by `NRS_NORM` and `NRS_REF`; the orientation comes from `DETECTOR` (NRS2 is the reversed layout, as in `jwst`), and a `FASTAXIS` that disagrees with it is an error. Frames with the IRS2 cards but only 2048 rows are treated as already stripped.

Roman L1 ASDF `[n, h, w]` arrays are recognised as ramps of resultants for inspection (frame stepping, pixel series, resultant times from `read_pattern`); the quick slope itself is FITS-only in this release.

## Algorithm

1. Strip. With an IRS2 layout (3200 rows for the owner's `NRSIRS2RAPID` files) the reference output and the interleaved reference rows are dropped and the 2048 science rows are scattered through `science_row_of`, so the product has the rate's shape and row order. Without the cards the frame keeps its size.
2. Reference offsets (`ref_correction = auto | amplifier`). Per amplifier band (640 uncal rows), one offset per (20-row block, group, column) is the median of the interleaved reference rows inside a 200-row window centred on the block (`ref_window_rows`, `null` for the whole band). It is subtracted from every science sample of that block before the fit. `off` keeps the strip and skips the correction (the product then carries the per-group bias drift: about -0.21 DN/s on NRS2 exposure 00001 rows 1300-1500).
3. Band scale. One robust scale of group differences per amplifier band (every third science row, every fifth column, pooled |d - median|, times 1.4826). It floors the per-pixel MAD so the 9-difference MAD of a 10-group ramp does not flag about 4 % of clean pixels (measured 11.9 to 14.1 DN on the owner's data).
4. Pixel fit. Groups from the first `raw >= sat_dn` (default 62258 DN) or non-finite value onwards are dropped (`SATURATED` when it was saturation). With at least four usable groups a difference whose |d - median| exceeds `jump_k x max(pixel MAD, scale_floor_dn, band scale)` marks `JUMP_DET` and the slope becomes the median of the unflagged differences; otherwise an unweighted OLS over the usable groups (median of differences below `min_groups_ols`). Fewer than two usable groups set `DO_NOT_USE`. `NOISE` is the OLS slope error for a per-sample sigma of scale / sqrt(2), or the median-efficiency form on the median paths.

## Product

`<stem>_qslope.fits` (`<stem>_intNNN_qslope.fits` when `NINTS > 1`) with four image extensions: `SCI` (f32, `BUNIT = DN/s`), `NGOOD` (i32, groups used), `DQ` (u32: `DO_NOT_USE 1`, `SATURATED 2`, `JUMP_DET 4`, the JWST bits), `NOISE` (f32, DN/s). The primary keeps the uncal cards needed to rebuild the layout (`ABNFAST`, `NRS_NORM`, `NRS_REF`, `NOUTPUTS`, `DETECTOR`, `FASTAXIS`, `TGROUP`, ...) plus `ABPROC`, `ABQSVER`, `ABINTEG`, `ABNGROUP`, `ABTGROUP`, `ABSATDN`, `ABJUMPK`, `ABREFCOR`, `ABREFWIN`. A PNG preview is written next to it. The Compare panel takes this product and an explicit `_rate.fits` and writes `<stem>_ratio.fits/.png`.

## Validation against the official rates

Measured on 2026-10-03 with the 0.6.6 kernel (`cargo test --release`, LTO off, codegen-units 16) on the eight program-1266 NIRSpec IFU `NRSIRS2RAPID` pairs (`jw01266005001_02103_0000{1,2,3,4}_nrs{1,2}`, 10 groups, 1 integration, rates from `jwst` 1.20.2 / `stcal` 1.15.2, `jwst_1464.pmap`). Default parameters, full frame, good = official `DQ == 0`, both values finite, quick `DQ` without `DO_NOT_USE`. `delta = quick - rate` in DN/s, `rel = delta / rate`. Per-amplifier deltas are the two faint bins `[-inf, 0.05)` and `[0.05, 0.3)` of each of the four science outputs.

| Pair | faint delta [-inf,0.05) | [0.05,0.3) | max per-amp abs delta | rel [1,3) | rel [3,10) | rel [10,30) | zero-shift corr at dy 0 (runner-up) | good pixels | quick flags in good |
|---|---|---|---|---|---|---|---|---|---|
| 00001 NRS1 | +0.0094 | +0.0106 | 0.0114 | +2.47 % | +1.73 % | +1.33 % | 0.99979 (0.945 at +1) | 4,000,308 | 0.152 % |
| 00001 NRS2 | -0.0000 | +0.0026 | 0.0066 | +2.53 % | +1.83 % | +1.32 % | 0.99996 (0.974 at +1) | 3,974,683 | 0.171 % |
| 00002 NRS1 | +0.0072 | +0.0089 | 0.0115 | +2.34 % | +1.83 % | +1.34 % | 0.99993 (0.893 at -1) | 4,006,134 | 0.148 % |
| 00002 NRS2 | +0.0028 | +0.0060 | 0.0123 | +1.81 % | +1.77 % | +1.37 % | 0.99998 (0.935 at -1) | 3,972,120 | 0.163 % |
| 00003 NRS1 | +0.0078 | +0.0091 | 0.0114 | +2.45 % | +1.77 % | +1.33 % | 0.99992 (0.903 at +1) | 3,998,779 | 0.152 % |
| 00003 NRS2 | +0.0027 | +0.0056 | 0.0095 | +2.66 % | +1.93 % | +1.33 % | 0.99997 (0.948 at -1) | 3,969,714 | 0.169 % |
| 00004 NRS1 | +0.0077 | +0.0074 | 0.0201 | +2.17 % | +1.72 % | +1.23 % | 0.99989 (0.905 at -1) | 4,000,493 | 0.134 % |
| 00004 NRS2 | +0.0012 | +0.0039 | 0.0114 | +1.27 % | +1.50 % | +1.26 % | 0.99997 (0.940 at -1) | 3,959,399 | 0.185 % |

Ratio statistics on the same good pixels, for official rates of at least 0.3 DN/s and at least 1 DN/s:

| Pair | n (>= 0.3) | median quick/rate | MAD of rel | within 2 % | within 5 % | n (>= 1) | median quick/rate | MAD of rel | within 2 % | within 5 % |
|---|---|---|---|---|---|---|---|---|---|---|
| 00001 NRS1 | 157,087 | 1.0264 | 0.0327 | 27.7 % | 58.3 % | 60,890 | 1.0205 | 0.0165 | 42.9 % | 83.0 % |
| 00001 NRS2 | 287,949 | 1.0249 | 0.0321 | 28.6 % | 59.3 % | 119,053 | 1.0207 | 0.0162 | 43.2 % | 82.3 % |
| 00002 NRS1 | 169,103 | 1.0208 | 0.0319 | 29.8 % | 60.4 % | 74,862 | 1.0187 | 0.0152 | 45.3 % | 83.9 % |
| 00002 NRS2 | 255,395 | 1.0173 | 0.0280 | 34.0 % | 65.1 % | 127,310 | 1.0161 | 0.0144 | 49.6 % | 86.5 % |
| 00003 NRS1 | 166,874 | 1.0214 | 0.0304 | 30.3 % | 61.3 % | 75,735 | 1.0197 | 0.0152 | 44.8 % | 83.4 % |
| 00003 NRS2 | 264,723 | 1.0225 | 0.0277 | 30.9 % | 62.8 % | 135,849 | 1.0207 | 0.0150 | 43.6 % | 82.8 % |
| 00004 NRS1 | 161,373 | 1.0180 | 0.0311 | 31.2 % | 61.8 % | 68,395 | 1.0176 | 0.0146 | 47.9 % | 85.6 % |
| 00004 NRS2 | 289,513 | 1.0132 | 0.0329 | 32.8 % | 61.5 % | 124,354 | 1.0129 | 0.0145 | 52.8 % | 87.9 % |

The +1.3 to +2.7 % excess at 1-30 DN/s is the expected signature of the missing linearity correction and of the unweighted fit; the per-pixel scatter (MAD 1.5 % above 1 DN/s) is dominated by the official pipeline's weighting and jump handling, not by the offset. Flag overlap, report only: `JUMP_DET` recall 0.41-0.48 and precision 0.72-0.84 (the official flag also marks neighbours and snowballs); `SATURATED` under the stcal 1.15.2 all-groups rule recall 0.10-0.11 and precision 0.09-0.16 (the fixed 62258 DN threshold is not the per-pixel CRDS saturation reference and the official flag grows into neighbours).

Band reproduction on NRS2 exposure 00001, uncal rows [1300, 1500) (science rows 1040-1200): faint median delta -0.2066 DN/s with `ref_correction = off` and +0.0067 DN/s with the default. The strip is in phase with the official product and the correction is active.

Gates applied by the ignored test and by the Compare panel's verdict: zero shift (`best_dy = 0`, correlation of the 90th-percentile row profiles at least 0.99); every per-amplifier faint median delta within 0.03 DN/s; full-frame median rel in `[1,3)`, `[3,10)`, `[10,30)` within [-1 %, +3 %]; the band reproduction within [-0.26, -0.16] off and [-0.03, +0.03] corrected; at least 3.5 M good pixels per detector. All eight pairs pass.

## Performance

Release build without LTO, owner's machine (24 threads), warm page cache, per pair: `ramp_quick_slope_cmd` end to end 256-291 ms (decode, correction, fit, FITS and PNG), `ramp_compare_rate_cmd` full frame 2348-2859 ms, `ramp_pixel_fit_cmd` 25-32 ms (it decodes the pixel's whole amplifier band to reproduce the product's offsets and band scale, and its slope is bit-identical to the product). Peak paged memory of the test process on one pair: 189 MB (34 MB baseline, +154 MB), peak working set 297 MB including the file-backed mmap pages.

## Reproducing the measurement

```
cd src-tauri
ASTROBURST_RAMP_PAIR_DIR=<directory with *_uncal.fits and sibling *_rate.fits> \
  cargo test --release --lib owner_pairs -- --ignored --nocapture
```

`ASTROBURST_RAMP_PAIR_GLOB=<substring>` restricts the run to matching file names (the test fails when nothing matches). The test writes its products into a temporary directory and never next to the input files.

## Known limitations

- NIRSpec IRS2 only for the reference correction (`NOUTPUTS = 5`, `NRS_NORM` even, interleaving along NAXIS2); other detectors are sloped without any reference correction and say so in `warnings`.
- One integration per run; `rateints`-style stacking is not provided.
- The saturation threshold is a single number; the reference-based per-pixel saturation of the pipeline is out of scope.
- `NOISE` is a slope error from the group-difference scale, not a propagated variance plane.
