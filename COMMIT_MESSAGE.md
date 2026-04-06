You are a Senior Rust Performance Engineer working on AstroBurst, a high-performance
desktop astrophotography application (Rust/Tauri backend + React/TypeScript frontend).

You will receive the backend codebase as a zip. Your task is to optimize the
blend_channels_cmd pipeline which is the main bottleneck in RGB composition.

================================================================================
PROJECT STATUS
================================================================================

The codebase has been through 6 refactoring sessions (S1-S6) that extracted shared
math/imaging primitives, eliminated code duplication, and centralized constants.
A full-stack audit was completed and all critical/warning findings were fixed
(contract mismatches, orphaned commands, dead code). The codebase compiles clean.

The current performance issue: blend_channels_cmd in cmd/compose/blend.rs is slow
for large images (4000x4000+). Profiling trace identified 4 concrete bottlenecks.

================================================================================
BLEND PIPELINE TRACE (blend_channels_cmd, cmd/compose/blend.rs)
================================================================================

For a 3-channel 4000x4000 image (16M pixels/channel, 64MB/channel as f32):

STEP 1 (L146-149): Load N channels from cache/disk
Cost: Fast if cached, seconds if from FITS disk

STEP 2 (L151): Clone all arrays: entries.iter().map(|e| e.arr().to_owned())
Cost: 192MB allocation (3 x 64MB)
NOTE: Necessary if resample needed, wasteful if dims already match

STEP 3 (L157-162): Resample mismatched dimensions via bicubic
Cost: O(rows*cols*16) per channel. Heavy but necessary.

STEP 4 (L178): blend_channels() - weighted sum
Cost: Fast, already par_chunks_mut via rayon. Not a bottleneck.

>>> BOTTLENECK B1 (L180-182 + L219-221): Stats computed 6x instead of 3x <<<

    L180: let stats_r = compute_image_stats(&r);   // 1st time
    L181: let stats_g = compute_image_stats(&g);   // 1st time
    L182: let stats_b = compute_image_stats(&b);   // 1st time

    L219: let (sr, _) = analyze(&*arc_r);           // 2nd time (analyze calls compute_image_stats internally)
    L220: let (sg, _) = analyze(&*arc_g);           // 2nd time
    L221: let (sb, _) = analyze(&*arc_b);           // 2nd time

    Each compute_image_stats does:
      - Parallel min/max/sum scan: O(n)
      - Allocate Vec<f32> with all valid pixels: 64MB per channel
      - exact_median_mut (introselect, O(n) average)
      - exact_mad_mut (abs deviations + introselect, O(n) average)
    Total: 6 calls x (64MB alloc + 2 introselects) = ~384MB temp + 12 passadas

    FIX B1: Use stats from L180-182 directly in auto_stf. auto_stf only needs
    ImageStats (median, sigma, min, max). The histogram from analyze() is
    discarded (assigned to _). Eliminate L219-221 entirely.

    BUT: When linked_stf is true (L223-227), helpers::compute_linked_stf takes
    3 ImageStats. Verify it doesn't need Histogram.

>>> BOTTLENECK B2 (L199-210): Luminance FITS written always <<<

    let lum_data: Vec<f32> = r_sl.iter().zip(g_sl).zip(b_sl)
        .map(|((&rv, &gv), &bv)| rv * 0.2126 + gv * 0.7152 + bv * 0.0722)
        .collect();
    write_fits_mono(&lum_fits_path, &lum, None)?;

    Cost: 64MB allocation + sequential zip + disk I/O
    The lum_fits_path IS returned in the JSON response, but no frontend service
    actually uses it. Frontend services/compose.ts BlendResult type doesn't even
    have a lum_fits_path field.

    FIX B2: Remove the luminance FITS write entirely. If needed in the future
    (e.g., for LRGB), add it as a separate on-demand command.

>>> BOTTLENECK B3 (L234-236): STF creates 3 intermediate full-size arrays <<<

    let r_out = apply_stf_f32(&*arc_r, &stf_r, &stats_r);  // allocates 64MB
    let g_out = apply_stf_f32(&*arc_g, &stf_g, &stats_g);  // allocates 64MB
    let b_out = apply_stf_f32(&*arc_b, &stf_b, &stats_b);  // allocates 64MB
    helpers::render_rgb_preview(&r_out, &g_out, &b_out, &png_path, MAX_PREVIEW_DIM)?;
    // r_out, g_out, b_out are immediately dropped after render

    Cost: 192MB allocation for arrays used only as input to render_rgb_preview
    The render_rgb_preview function downsamples to MAX_PREVIEW_DIM (typically 2048)
    and converts to u8. So we allocate 192MB just to read a few sampled pixels.

    FIX B3: Fuse STF application into the render step. Instead of materializing
    3 full arrays, pass STF closures that transform f32->u8 per-pixel during
    the downsample loop inside render_rgb_preview.

    EXISTING CODE TO LEVERAGE:
    - stf.rs already has StfTransform struct with .apply(f64) -> f64
    - stf.rs already has apply_stf() that does f32 -> u8 (but allocates Vec<u8>)
    - The pattern is: StfTransform::new(params, stats), then tx.apply(v as f64)

    PROPOSED: Add to stf.rs:
      pub fn make_stf_u8_fn(params: &StfParams, stats: &ImageStats) -> impl Fn(f32) -> u8
    Returns a closure capturing pre-computed StfTransform fields.

    Then add a variant of render_rgb_preview (or parametrize existing):
      pub fn render_rgb_preview_with_transform(
          r: &Array2<f32>, g: &Array2<f32>, b: &Array2<f32>,
          r_fn: impl Fn(f32)->u8 + Sync,
          g_fn: impl Fn(f32)->u8 + Sync,
          b_fn: impl Fn(f32)->u8 + Sync,
          path: &str, max_dim: usize,
      ) -> Result<()>
    The downsample loop applies the transform per-pixel:
      row_buf[o]     = r_fn(r_slice[si]);
      row_buf[o + 1] = g_fn(g_slice[si]);
      row_buf[o + 2] = b_fn(b_slice[si]);

STEP (L188-194): Cache insert x6
Cost: 6 Arc clone + hashmap insert. Negligible.

>>> MINOR: B4 (L151): Unnecessary .to_owned() when dims already match <<<

    let mut arrays: Vec<Array2<f32>> = entries.iter().map(|e| e.arr().to_owned()).collect();
    // Then checks dims and resamples if needed

    FIX B4: Check dims first. If all match, use Arc references directly instead
    of cloning. Only clone+resample the ones that differ. This avoids 192MB of
    copies in the common case where all channels have the same dimensions.

================================================================================
ESTIMATED IMPACT
================================================================================

For 3-channel 4000x4000 (common astro resolution):

Current peak memory: ~835MB
After B1 (eliminate double stats): -384MB temp, -3 full scans
After B2 (remove lum FITS): -64MB alloc, -disk I/O
After B3 (fused STF+render): -192MB alloc
After B4 (skip clone when dims match): -192MB alloc (common case)

Optimized peak memory: ~200MB (4x reduction)
Estimated wall-clock improvement: 40-60% for large images

================================================================================
FILES TO MODIFY
================================================================================

1. core/imaging/stf.rs (221 lines)
   ADD: pub fn make_stf_u8_fn(params, stats) -> impl Fn(f32) -> u8

2. cmd/helpers.rs (251 lines)
   ADD: pub(crate) fn render_rgb_preview_with_transform(r, g, b, r_fn, g_fn, b_fn, path, max_dim)
   The existing render_rgb_preview remains for the no-stretch path (L242).

3. cmd/compose/blend.rs (345 lines)
   MODIFY: blend_channels_cmd to apply B1, B2, B3, B4

FILES NOT TO MODIFY:
- core/imaging/stats.rs (compute_image_stats is fine, just called too many times)
- core/compose/channel_blend.rs (blend_channels is already fast)
- Any Session 1-6 files

================================================================================
IMPLEMENTATION DETAILS
================================================================================

TASK 1: Add make_stf_u8_fn to core/imaging/stf.rs

pub fn make_stf_u8_fn(params: &StfParams, stats: &ImageStats) -> impl Fn(f32) -> u8 {
let tx = StfTransform::new(params, stats);
move |v: f32| -> u8 {
if !v.is_finite() || v <= 1e-7 {
return 0;
}
(tx.apply(v as f64) * 255.0).round().clamp(0.0, 255.0) as u8
}
}

NOTE: StfTransform is currently private. Either make it pub(crate) or keep
make_stf_u8_fn in the same file (preferred, simpler).

Also add a no-op identity version for the non-stretch path:

pub fn make_clamp_u8_fn() -> impl Fn(f32) -> u8 {
|v: f32| -> u8 {
(v.clamp(0.0, 1.0) * 255.0) as u8
}
}

TASK 2: Add render_rgb_preview_with_transform to cmd/helpers.rs

Same logic as existing render_rgb_preview but the inner loop uses closures:

    for dx in 0..pw {
        let sx = ((dx as f64) * x_ratio).min((cols - 1) as f64) as usize;
        let si = src_base + sx;
        let o = dx * 3;
        row_buf[o]     = r_fn(r_slice[si]);
        row_buf[o + 1] = g_fn(g_slice[si]);
        row_buf[o + 2] = b_fn(b_slice[si]);
    }

For the full-size path (no downscaling needed), same pattern applies.

TASK 3: Optimize blend_channels_cmd in cmd/compose/blend.rs

Apply all 4 fixes:

B4: Before cloning, check if all dims match. If yes, work with Arc refs.
Only .to_owned() the channels that need resampling.

B1: Remove the analyze() calls at L219-221.
Use stats_r/g/b from L180-182 directly:
if linked {
let stf = helpers::compute_linked_stf(&stats_r, &stats_g, &stats_b, &cfg);
} else {
stf_r = auto_stf(&stats_r, &cfg);
stf_g = auto_stf(&stats_g, &cfg);
stf_b = auto_stf(&stats_b, &cfg);
}

      VERIFY: compute_linked_stf signature accepts &ImageStats (not histogram).

B2: Remove the entire luminance FITS block (L199-210).
Remove lum_fits_path from the JSON response.

B3: Replace apply_stf_f32 + render_rgb_preview with fused path:
let r_fn = make_stf_u8_fn(&stf_r, &stats_r);
let g_fn = make_stf_u8_fn(&stf_g, &stats_g);
let b_fn = make_stf_u8_fn(&stf_b, &stats_b);
render_rgb_preview_with_transform(&*arc_r, &*arc_g, &*arc_b, r_fn, g_fn, b_fn, &png_path, MAX_PREVIEW_DIM)?;

      For the no-stretch path (L238-242):
        let identity = make_clamp_u8_fn();
        render_rgb_preview_with_transform(&*arc_r, &*arc_g, &*arc_b, identity, identity, identity, &png_path, MAX_PREVIEW_DIM)?;

      NOTE: make_clamp_u8_fn returns impl Fn which is not Clone. Either:
        a) Call make_clamp_u8_fn() 3 times, or
        b) Use a single closure with |v| (v.clamp(0.0,1.0) * 255.0) as u8 inline

================================================================================
CRITICAL RULES
================================================================================

- ZERO COMMENTS in generated Rust code
- Preserve all existing tests
- Preserve all public API signatures
- The JSON response shape of blend_channels_cmd must stay the same EXCEPT:
    - "lum_fits_path" field can be removed (no frontend consumer)
- Do NOT modify compute_image_stats, exact_median_mut, or other math/ primitives
- Do NOT modify the blend_channels function in channel_blend.rs
- The existing render_rgb_preview must remain (used by restretch_composite_cmd etc.)
- Use pub(crate) for new cmd/helpers functions

================================================================================
OUTPUT FORMAT
================================================================================

Output 3 complete files in order:
1. core/imaging/stf.rs (extended: add make_stf_u8_fn + make_clamp_u8_fn)
2. cmd/helpers.rs (extended: add render_rgb_preview_with_transform)
3. cmd/compose/blend.rs (optimized: all 4 fixes applied)

Each file: complete, compilable, drop-in replacement. No explanations, only code.
