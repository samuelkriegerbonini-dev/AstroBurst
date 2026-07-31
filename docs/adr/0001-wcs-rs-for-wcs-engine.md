# Use wcs-rs for the WCS linear/projection math, but not its SIP support

AstroBurst's old WCS engine (`WcsTransform`, `src-tauri/src/core/astrometry/wcs.rs`) hand-rolled TAN/SIN/ARC/CAR projection math and never read the FITS `PC` matrix convention — silently wrong for ASDF/Roman-derived headers, which emit `PC`+`CDELT` instead of `CD`. We replaced the linear (CD/PC/CDELT) transform and celestial projection/deprojection with the [`wcs`](https://github.com/cds-astro/wcs-rs) crate (wcs-rs + mapproj, crates.io), which supports ~20 FITS projections and handles `PC` correctly, while keeping `WcsTransform`'s public API unchanged.

We do **not** use wcs-rs's own SIP distortion support, even though it exists: `mapproj` 0.4.0's SIP polynomial evaluator has a confirmed bug (it advances polynomial powers by repeated squaring instead of by degree), caught by cross-checking `pixel_to_world` output against astropy — it silently produces sky coordinates off by tens of degrees, not just imprecise ones, for any SIP-distorted header. This matches wcs-rs's own README admission that SIP support is "not tested." `WcsTransform::from_header` strips any `-SIP` CTYPE suffix before constructing the wcs-rs engine so this path never runs, and applies/inverts SIP with AstroBurst's own (pre-existing) polynomial math instead.

## Consequences

- If a future `wcs`/`mapproj` upgrade fixes the SIP bug, the `-SIP`-suffix-stripping workaround and the hand-rolled `sip_forward`/`sip_inverse` methods become optional, not mandatory — worth revisiting then, not before.
- `pixel_scale_arcsec`/`field_of_view` still compute from a CD matrix synthesized directly from the header (mirroring wcs-rs's own CD > PC×CDELT > CDELT+CROTA2 priority) rather than asking the engine, so these numbers stay bit-identical for CD-based files while becoming correct for PC-only ones.
