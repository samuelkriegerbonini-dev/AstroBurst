> Contributed by Jae-Joon Lee — https://github.com/leejjoon

# Testing astroburst-server

Two levels of testing are available: automated Rust unit/integration tests and manual curl-based end-to-end testing against a running server.

---

## Automated tests

The server binary has 10 Axum oneshot tests that run without a network socket. They cover the critical paths: health, session 404, slot 404, job status, job cancellation, SSE 409, semaphore 429, TTL logic, and config defaults.

```bash
cargo test \
  --no-default-features \
  --features server,astrometry-net,asdf-full,vizier \
  --bin astroburst-server
```

Expected output:

```
running 10 tests
test tests::config_env_var_parsed_and_invalid_falls_back ... ok
test tests::ttl_skips_session_with_running_job ... ok
test tests::unknown_session_returns_404 ... ok
test tests::semaphore_full_returns_429 ... ok
test tests::request_id_echoed ... ok
test tests::get_job_returns_running_status ... ok
test tests::health_returns_ok ... ok
test tests::sse_second_subscriber_returns_409 ... ok
test tests::fits_header_unknown_slot_returns_404 ... ok
test tests::cancel_job_sets_cancelled ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured
```

To run only a single test by name:

```bash
cargo test --no-default-features --features server,astrometry-net,asdf-full,vizier \
  --bin astroburst-server -- tests::health_returns_ok --nocapture
```

---

## Manual end-to-end testing with curl

Start the server in one terminal:

```bash
cargo run --bin astroburst-server --no-default-features --features server,astrometry-net,asdf-full,vizier
```

All examples below assume `BASE=http://localhost:8080`.

### 1. Health check

```bash
curl -s http://localhost:8080/health | jq .
```

Expected:
```json
{
  "status": "ok",
  "version": "0.5.3",
  "sessions_active": 0,
  "sessions_total": 0
}
```

### 2. Create a session

```bash
SID=$(curl -s -X POST http://localhost:8080/sessions | jq -r .session_id)
echo "session: $SID"
```

### 3. Open a FITS file

The path must exist on the server's filesystem:

```bash
curl -s -X POST http://localhost:8080/sessions/$SID/fits/open \
  -H 'content-type: application/json' \
  -d "{\"path\":\"/path/to/image.fits\", \"slot\":\"img\"}" | jq .
```

Expected:
```json
{
  "slot": "img",
  "dims": [2048, 2048],
  "stats": { "min": 0.0, "max": 65535.0, "median": 412.3, ... },
  "stf": { "shadow": 0.001, "midtone": 0.22, "highlight": 1.0 },
  "header": { "OBJECT": "M51", "FILTER": "Ha" }
}
```

### 4. Render as PNG

Auto-STF stretch, saved to a local file:

```bash
curl -s -X POST http://localhost:8080/sessions/$SID/image/stf \
  -H 'content-type: application/json' \
  -d '{"slot":"img"}' \
  -o /tmp/preview.png && echo "saved"
```

Pixel crop:

```bash
curl -s -X POST http://localhost:8080/sessions/$SID/image/viewport \
  -H 'content-type: application/json' \
  -d '{"slot":"img","x":0,"y":0,"w":512,"h":512}' \
  -o /tmp/crop.png
```

### 5. FITS header

```bash
curl -s -X POST http://localhost:8080/sessions/$SID/fits/header \
  -H 'content-type: application/json' \
  -d '{"slot":"img"}' | jq '.cards[:5]'
```

### 6. Sigma-clip stack (async job)

```bash
JID=$(curl -s -X POST http://localhost:8080/sessions/$SID/stacking/stack \
  -H 'content-type: application/json' \
  -d '{
    "paths": ["/data/f1.fits", "/data/f2.fits", "/data/f3.fits"],
    "result_slot": "stacked",
    "align": true
  }' | jq -r .job_id)

echo "job: $JID"
```

Poll until done:

```bash
while true; do
  STATUS=$(curl -s http://localhost:8080/sessions/$SID/jobs/$JID | jq -r .status)
  PCT=$(curl -s http://localhost:8080/sessions/$SID/jobs/$JID | jq -r .pct)
  echo "$STATUS $PCT%"
  [ "$STATUS" != "running" ] && break
  sleep 2
done
```

### 7. SSE progress stream

Subscribe to live progress events (alternative to polling):

```bash
curl -N http://localhost:8080/sessions/$SID/jobs/$JID/stream
```

Events arrive as:
```
event: progress
data: {"type":"progress","pct":45,"stage":"aligning"}

event: complete
data: {"type":"complete"}
```

### 8. Cancel a job

```bash
curl -s -X DELETE http://localhost:8080/sessions/$SID/jobs/$JID | jq .status
# "cancelled"
```

### 9. Drizzle

```bash
JID=$(curl -s -X POST http://localhost:8080/sessions/$SID/stacking/drizzle \
  -H 'content-type: application/json' \
  -d '{
    "paths": ["/data/f1.fits", "/data/f2.fits"],
    "result_slot": "drizzled",
    "scale": 2.0,
    "pixfrac": 0.7,
    "kernel": "lanczos3"
  }' | jq -r .job_id)
```

### 10. Full narrowband pipeline

Calibrate + stack Ha and OIII in one call:

```bash
JID=$(curl -s -X POST http://localhost:8080/sessions/$SID/pipeline/run \
  -H 'content-type: application/json' \
  -d '{
    "channels": [
      { "label": "Ha",   "paths": ["/data/ha_1.fits", "/data/ha_2.fits"] },
      { "label": "OIII", "paths": ["/data/oiii_1.fits", "/data/oiii_2.fits"] }
    ],
    "bias_paths": ["/data/bias.fits"],
    "dark_paths": ["/data/dark.fits"],
    "flat_paths": ["/data/flat.fits"],
    "result_prefix": "pipe/"
  }' | jq -r .job_id)

# Stream progress
curl -N http://localhost:8080/sessions/$SID/jobs/$JID/stream
```

Result slots after completion: `pipe/Ha`, `pipe/OIII`.

---

## 11. v2 API — sessions & image lifecycle

The `/v2` surface is mounted alongside (and reuses the state of) the v1 routes.
It tracks images by a bare `image_ref` (`img_0`, `img_1`, ...) scoped to the
session by the URL path. This walkthrough exercises open → images → status →
hdu → delete against a real FITS fixture.

```bash
# Create a v2 session (identical handler to POST /sessions).
SID=$(curl -s -X POST http://localhost:8080/v2/sessions | jq -r .session_id)

# Open a file — becomes the active ref. Returns dims / stats / wcs_present / header.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/open \
  -H 'content-type: application/json' \
  -d '{"path":"/data/obs/coadd_r.fits"}' \
  | jq '{ref, active_ref, dims, wcs_present, median: .stats.median}'
# => { "ref": "img_0", "active_ref": "img_0", "dims": [W,H], "wcs_present": true, ... }

# List every image ref registered in the session.
curl -s http://localhost:8080/v2/sessions/$SID/images | jq '{active_ref, count, refs: [.images[].image_ref]}'

# Session status: active ref, image count, cache footprint.
curl -s http://localhost:8080/v2/sessions/$SID | jq .

# Switch HDU — creates a NEW ref (img_1) from the active ref's source file,
# without touching the original img_0. img_1 becomes active.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/hdu \
  -H 'content-type: application/json' \
  -d '{"hdu":1}' | jq '{ref, active_ref, dims, extname}'

# Both refs are still listed; img_0 remains addressable.
curl -s http://localhost:8080/v2/sessions/$SID/images | jq '.count'   # => 2

# Keepalive is a no-op (SessionExtractor already refreshed the TTL on resolve).
curl -s -X POST http://localhost:8080/v2/sessions/$SID/keepalive | jq .

# Delete the session; a subsequent request 404s.
curl -s -o /dev/null -w "%{http_code}\n" -X DELETE http://localhost:8080/v2/sessions/$SID  # => 204
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8080/v2/sessions/$SID            # => 404
```

Opening a file whose header carries no WCS returns `wcs_present: false`; a later
slice's `POST .../cutout` with a `sky` region against such an image will return
`400 wcs_required`, and an over-large `pixel`/`sky` region returns
`400 region_out_of_bounds` (with a `hint` naming the image extent) unless
`clip: true` is passed.

### WCS coordinate transforms (issue #3)

All three run against an image ref (the request's optional `ref`, else the
session's active ref) and are batch-capable. `on_image` compares the pixel
coordinates against the image's NAXIS1/NAXIS2 extent.

```bash
# Pixel -> sky. Each result carries {x, y, ra, dec, on_image}.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/wcs/pix2sky \
  -H 'content-type: application/json' \
  -d '{"points": [[3,3], [100,100]]}' | jq '.results'

# Sky -> pixel (round-trips the above within float tolerance).
curl -s -X POST http://localhost:8080/v2/sessions/$SID/wcs/sky2pix \
  -H 'content-type: application/json' \
  -d '{"points": [[150.0, 2.0]]}' | jq '.results'

# Angular separation — two sky points (no WCS needed)...
curl -s -X POST http://localhost:8080/v2/sessions/$SID/wcs/separation \
  -H 'content-type: application/json' \
  -d '{"type":"sky","a":[150.0,2.0],"b":[150.0,3.0]}' | jq .
# => { "separation_deg": 1.0, "separation_arcmin": 60.0, "separation_arcsec": 3600.0 }

# ...or two pixel points, resolved to sky via the image's WCS first.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/wcs/separation \
  -H 'content-type: application/json' \
  -d '{"type":"pixel","a":[3,3],"b":[3,4]}' | jq .
```

Calling `pix2sky` / `sky2pix` (or a `pixel` separation) against an image with no
usable WCS header returns `400 wcs_required` — never a panic or 500.

### Inspection — structure / header / WCS summary (issue #4)

Read-only "what am I looking at?" endpoints. All accept an optional `?ref=`
(alias `?image=`) query param; without it they target the session's active ref.

```bash
# Structure — every HDU of the active ref's source file (index, extname,
# shape [ny, nx], BITPIX, dtype, has_data). First call on an unfamiliar file.
curl -s http://localhost:8080/v2/sessions/$SID/structure | jq '.hdus'

# Header — the full set of cached cards as {KEY: {value}} ...
curl -s http://localhost:8080/v2/sessions/$SID/header | jq '.cards'
# ... an explicit subset ...
curl -s "http://localhost:8080/v2/sessions/$SID/header?keys=CTYPE1,CRVAL1" | jq '.cards'
# ... or a glob for the full WCS matrix.
curl -s "http://localhost:8080/v2/sessions/$SID/header?keys=CD*_*" | jq '.cards'

# WCS summary — projection, CRPIX/CRVAL, CD, per-axis pixel scale (arcsec/px),
# rotation (deg E of N), parity, SIP presence.
curl -s http://localhost:8080/v2/sessions/$SID/wcs | jq .
# => { "present": true, "projection": "TAN", "pixel_scale_x_arcsec": 0.36,
#      "rotation_deg": 0.0, "parity": "normal", "sip_present": false, ... }
```

Unlike the `/wcs/*` transform routes, `GET /wcs` treats a missing WCS as a
normal state: an image with no usable WCS header returns `200 {"present":
false}` — not a 4xx/5xx — so an agent can branch cleanly on whether sky
coordinates are available.

### Cutout — region crop into a derived ref (issue #5)

`POST .../cutout` crops a pixel- or sky-specified region of the active (or an
explicit `ref`) image into a **new derived `image_ref`** (auto-named `cutout_N`
unless `name` is given), which becomes the session's active ref. The parent's
`CRPIX1/2` are shifted by the crop origin so the cutout's own WCS stays correct.
Unlike the strict resolver, cutout tolerates partial (or full) miss of the parent
frame: off-parent pixels are NaN and the response reports `fraction_on_image`.
There is no `save`/FITS-write field — a cutout only ever lives in the session.

```bash
# Pixel region: crop cols 2..6, rows 2..6.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/cutout \
  -H 'content-type: application/json' \
  -d '{"region":{"type":"pixel","x":2,"y":2,"width":4,"height":4}}' | jq .
# => { "ref":"cutout_0", "dims":[4,4], "fraction_on_image":1.0, "wcs_present":true, ... }

# Sky region: centered on RA/Dec with an angular size (arcmin; scalar or [w,h]).
curl -s -X POST http://localhost:8080/v2/sessions/$SID/cutout \
  -H 'content-type: application/json' \
  -d '{"region":{"type":"sky","ra":150.0,"dec":2.0,"size_arcmin":[3.0,3.0]}}' | jq .

# Partial overlap is NOT an error — off-image pixels are NaN, fraction_on_image < 1.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/cutout \
  -H 'content-type: application/json' \
  -d '{"region":{"type":"pixel","x":6,"y":6,"width":4,"height":4}}' | jq .fraction_on_image
# => 0.25   (only the 2×2 corner of a 4×4 request lands on an 8×8 image)
```

A `sky` region against an image with no usable WCS returns `400 wcs_required`;
`preserve_wcs:false` drops the WCS from the cutout header instead of shifting it.

### Block-average rebinning (issue #6)

```bash
# Block-average the active (or `ref`-named) image by an integer factor,
# producing a new derived ref at out_dims = in_dims / factor. NaN pixels within
# a block are ignored (not propagated). Only method "mean" is supported.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/bin \
  -H 'content-type: application/json' \
  -d '{"factor":4,"method":"mean"}' | jq .
# => { "ref": "bin_0", "active_ref": "bin_0", "from_ref": "img_0",
#      "factor": 4, "method": "mean", "dims": [W/4, H/4], ... }
```

`method: "sum"` is in the API doc's spec but intentionally unsupported here: it
returns `400 bad_request` rather than approximating it as `mean * factor²`
(inexact on edge blocks when dims aren't divisible by `factor`). A `factor`
larger than the smaller image dimension (empty output) is also a `bad_request`.

### Pixel point-query (issue #7)

The "is that white speck real or a hot pixel?" sanity check: value at a pixel,
plus min/max/mean over the surrounding `box`×`box` neighborhood, and the sky
coordinate when the image carries a WCS. A real source has elevated neighbors;
a single hot pixel does not.

```bash
# Value + 5×5 neighborhood stats + sky coordinate.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/pixel \
  -H 'content-type: application/json' \
  -d '{"x":3,"y":3,"box":5}' | jq .
# => { "ref":"img_0", "x":3, "y":3, "value":..., "box":5,
#      "neighborhood": {"min":.., "max":.., "mean":.., "n_pixels":25, "n_nan":0},
#      "sky": {"ra":150.0, "dec":2.0} }
```

The `box` window is clipped to the image at edges (`n_pixels` reports how many
in-bounds pixels were counted), NaNs are skipped from the stats and counted in
`n_nan`, and `sky` is `null` when the image has no usable WCS (not an error).
An out-of-bounds `(x, y)` returns `400 pixel_out_of_bounds` — never a panic.

### Region-scoped stats (issue #8)

`POST /v2/sessions/:sid/stats` computes pixel statistics over a region (or the
full frame when `region` is omitted). The unclipped block always mirrors
`compute_image_stats`; `sigma_clip` adds a `clipped` block and `percentiles`
adds nearest-rank (non-interpolated) percentiles.

```bash
# Full-frame stats.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/stats \
  -H 'content-type: application/json' -d '{}' | jq .

# Region stats + sigma-clip + percentiles.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/stats \
  -H 'content-type: application/json' \
  -d '{
        "region": {"type":"pixel","x":2,"y":2,"width":4,"height":4},
        "sigma_clip": {"sigma": 3.0, "maxiters": 5},
        "percentiles": [16, 50, 84]
      }' | jq .
# => { ... "min","max","median","mad","sigma","mean","valid_count","n_nan",
#      "clipped": {"mean","median","std","n_rejected"},
#      "percentiles": [{"percentile":16,"value":...}, ...] }
```

Omitting `sigma_clip` (or passing `null`) drops the `clipped` block; an empty /
absent `percentiles` drops that block. An out-of-bounds region returns
`400 region_out_of_bounds` unless the region carries `clip: true`, which clamps
it to the image bounds (`region.clipped: true` in the response).

Reference values in the automated test were cross-checked with
`uv run --with astropy --with numpy` against the same 4×4 fixture region.

---

### Region-scoped histogram (issue #9)

`POST /v2/sessions/:sid/histogram` builds a fixed-`bins` histogram (default 256)
of the valid pixels in a region (or the full frame). Two handler-side additions
sit on top of the core `build_histogram`:

- **auto-range** — when `range` is omitted/`null`, the value range is a robust
  0.1–99.9th-percentile window rather than the raw min/max, so a single hot
  pixel can't blow out the whole range (`range_source: "auto"`). An explicit
  `range: [lo, hi]` is used verbatim (`range_source: "explicit"`).
- **`log_counts`** — when true, each bin count `c` is returned as `ln(1 + c)`
  (empty bins stay `0`) instead of the raw integer count.

```bash
# Auto-ranged 256-bin histogram over the full frame.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/histogram \
  -H 'content-type: application/json' -d '{}' | jq '{range_source, min, max, mode}'

# Region histogram, explicit range, log-transformed counts.
curl -s -X POST http://localhost:8080/v2/sessions/$SID/histogram \
  -H 'content-type: application/json' \
  -d '{
        "region": {"type":"pixel","x":2,"y":2,"width":4,"height":4},
        "bins": 200,
        "range": [0, 1000],
        "log_counts": true
      }' | jq .
# => { "ref","region","bins":[...],"bin_edges":[...],"min","max",
#      "log_counts","range_source","mode" }
```

An out-of-bounds region returns `400 region_out_of_bounds` unless it carries
`clip: true`. PNG plot rendering (`render_png`) is not implemented in this
slice: requesting it is an explicit `400 not_implemented`, never a silent no-op.

### Render — the agent-facing PNG endpoint (issue #13)

`POST /v2/sessions/:sid/render` is the highest-volume call: it turns the active
(or named) image into an RGB8 PNG for the *render → look → adjust → render*
loop. The body is the raw PNG; a JSON `x-render-resolved` **response header**
echoes back the fully-resolved parameters (the `vmin`/`vmax` zscale actually
chose, the shown region, binning factor, clipped fractions) so the agent can
reason numerically and reproduce/tweak the render. An empty body renders the
full frame with zscale + linear + gray defaults.

- **Scale** — `algorithm`: `zscale` (default) | `minmax` | `percentile`
  (`percentile: [lo, hi]`, 0–100) | `manual` (`vmin`/`vmax` passthrough).
- **Stretch** — `linear` (default) | `log` | `sqrt` | `asinh` (`asinh_a`) |
  `power` (`power`), composed after `vmin`/`vmax` normalization.
- **Colormap** — `gray` (default) | `viridis`; `invert_cmap` flips it.
- **`max_dim`** — long side of the PNG; the region is area-binned so
  `max(w, h) <= max_dim` (`binning_applied` reports the factor; `1` = native).
- **Region** — always clamps silently to the on-image intersection (never
  `region_out_of_bounds`): panning near an edge shows what is there.
- **Overlays** — `crosshair` (`x`/`y` pixel or `ra`/`dec` sky) and `scalebar`
  (`length_arcsec`), drawn on the final post-binning buffer. A `scalebar` on a
  WCS-less image is silently omitted. Stateless: never mints a ref or changes
  `active_ref`.

```bash
# Default full-frame render; inspect the resolved parameters from the header.
curl -s -D - -o /tmp/r.png -X POST http://localhost:8080/v2/sessions/$SID/render \
  -H 'content-type: application/json' -d '{}' | grep -i x-render-resolved

# Manual scale + asinh + viridis, downsampled to 1024px, with overlays.
curl -s -o /tmp/r2.png -X POST http://localhost:8080/v2/sessions/$SID/render \
  -H 'content-type: application/json' \
  -d '{
        "region": {"type":"pixel","x":100,"y":100,"width":2048,"height":2048},
        "scale": {"algorithm":"manual","vmin":-0.01,"vmax":0.15,
                  "stretch":"asinh","asinh_a":0.05},
        "colormap": "viridis",
        "max_dim": 1024,
        "overlays": [{"type":"crosshair","ra":150.0,"dec":2.0},
                     {"type":"scalebar","length_arcsec":30}]
      }'
```

Unknown `colormap`/`stretch`/scale `algorithm` names are `400 bad_request` with
a hint listing the supported vocabulary.

---

## Testing configuration

Verify env-var overrides work before deploying to a remote machine:

```bash
# Tight limits — 3rd session must return 503
ASTROBURST_SESSION_MAX=2 cargo run --bin astroburst-server \
  --no-default-features \
  --features server,astrometry-net,asdf-full,vizier &

sleep 2

curl -s -X POST http://localhost:8080/sessions | jq .session_id   # ok
curl -s -X POST http://localhost:8080/sessions | jq .session_id   # ok
curl -s -X POST http://localhost:8080/sessions | jq .             # 503

kill %1
```

Verify an invalid value falls back gracefully (warning on stderr, default used):

```bash
ASTROBURST_SESSION_MAX=notanumber cargo run --bin astroburst-server \
  --no-default-features \
  --features server,astrometry-net,asdf-full,vizier 2>&1 | head -5
# WARN: ASTROBURST_SESSION_MAX="notanumber" is invalid ...; using default 8
```

---

## Error cases to verify manually

| Scenario | Expected |
|---|---|
| `GET /sessions/bad-id/jobs/x` | `404 not_found` |
| `POST /sessions/:sid/fits/header` with unknown slot | `404 not_found` |
| Second `GET /sessions/:sid/jobs/:jid/stream` | `409 conflict` |
| `POST /sessions/:sid/stacking/stack` with `"paths":[]` | `400 bad_request` |
| `POST /v2/sessions/:sid/bin` with `"method":"sum"` | `400 bad_request` (unsupported method) |
| `POST /sessions` when `ASTROBURST_SESSION_MAX` reached | `503 service_unavailable` with message showing the configured cap |
| `X-Request-Id: my-id` on any request | Response echoes `x-request-id: my-id` |
| `GET`/`DELETE`/`POST` on `/v2/sessions/bad-id/...` | `404 not_found` |
| `POST /v2/sessions/:sid/hdu` before any `open` | `400 bad_request` (no active image) |
| `resolve_region` on an over-large region, `clip` unset | `400 region_out_of_bounds` with a `hint` naming the image extent |
| `resolve_region` on a `sky` region against a WCS-less image | `400 wcs_required` |
| `POST /v2/sessions/:sid/wcs/pix2sky` against a WCS-less image | `400 wcs_required` |
| `POST /v2/sessions/:sid/wcs/{pix2sky,sky2pix}` before any `open` | `400 bad_request` (no active image) |
| `POST /v2/sessions/:sid/wcs/pix2sky` with a `ref` not in the session | `404 not_found` |
| `GET /v2/sessions/:sid/wcs` on a WCS-less image | `200 {"present": false}` (never an error) |
| `GET /v2/sessions/:sid/structure` on a derived/ASDF ref | `400 bad_request` (no FITS source to inspect) |
| `GET /v2/sessions/:sid/{structure,header,wcs}` before any `open` | `400 bad_request` (no active image) |
| `POST /v2/sessions/:sid/cutout` with a `sky` region against a WCS-less image | `400 wcs_required` |
| `POST /v2/sessions/:sid/cutout` with a `ref` not in the session | `404 not_found` |
| `POST /v2/sessions/:sid/cutout` with a partial-overlap region | `200 OK`, `fraction_on_image < 1.0`, off-image pixels NaN (never an error) |
| `POST /v2/sessions/:sid/stats` with an over-large region, `clip` unset | `400 region_out_of_bounds` |
| `POST /v2/sessions/:sid/stats` before any `open` | `400 bad_request` (no active image) |
| `POST /v2/sessions/:sid/stats` with a `ref` not in the session | `404 not_found` |
| `POST /v2/sessions/:sid/histogram` with an over-large region, `clip` unset | `400 region_out_of_bounds` |
| `POST /v2/sessions/:sid/histogram` with `render_png: true` | `400 not_implemented` (never a silent no-op) |
| `POST /v2/sessions/:sid/histogram` with `bins: 0` | `400 bad_request` |
| `POST /v2/sessions/:sid/histogram` before any `open` | `400 bad_request` (no active image) |
| `POST /v2/sessions/:sid/render` with a partial/fully-out-of-bounds region | `200 OK`, region silently clamped, reported in `x-render-resolved` (never an error) |
| `POST /v2/sessions/:sid/render` with `colormap`/`stretch`/scale `algorithm` unknown | `400 bad_request` (hint lists supported names) |
| `POST /v2/sessions/:sid/render` with a `scalebar` overlay on a WCS-less image | `200 OK`, scalebar silently omitted (never an error) |
| `POST /v2/sessions/:sid/render` before any `open` | `400 bad_request` (no active image) |
| `POST /v2/sessions/:sid/render` with a `ref` not in the session | `404 not_found` |
