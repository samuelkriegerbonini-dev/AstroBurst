> Contributed by Jae-Joon Lee — https://github.com/leejjoon

# AstroBurst Headless Server

`astroburst-server` is a standalone HTTP server that exposes the full AstroBurst imaging pipeline over REST. It is designed to run on a remote GPU workstation and be accessed through an SSH tunnel by a local agent or script.

---

## Building

```bash
cd src-tauri
cargo build --release \
  --no-default-features \
  --features server,astrometry-net,asdf-full,vizier
```

The binary is at `target/release/astroburst-server`.

For development (faster compile, no optimisations):

```bash
cargo run --bin astroburst-server \
  --no-default-features \
  --features server,astrometry-net,asdf-full,vizier
```

---

## Running

### Local

```bash
./target/release/astroburst-server
```

Startup log:

```
INFO  astroburst_server] AstroBurst Headless Server v0.5.3
INFO  astroburst_server] Listening on 127.0.0.1:8080
INFO  astroburst_server] Session TTL: 900s, Max sessions: 8
INFO  astroburst_server] Per-session cache: 32 entries / 2048 MiB
INFO  astroburst_server] Cleanup interval: 60s
```

### Remote GPU workstation via SSH tunnel

On the remote machine:

```bash
./astroburst-server
```

On your local machine:

```bash
ssh -L 8080:localhost:8080 user@remote-host
```

All API calls go to `http://localhost:8080` from the local machine — the SSH tunnel forwards them securely.

---

## Configuration

All knobs are set via environment variables. Unset variables use the defaults shown. An invalid value emits a warning and falls back to the default — the server never fails to start because of a bad env var.

| Variable | Default | Description |
|---|---|---|
| `ASTROBURST_BIND` | `127.0.0.1:8080` | TCP address to listen on |
| `ASTROBURST_SESSION_TTL` | `900` | Idle seconds before a session is evicted |
| `ASTROBURST_SESSION_MAX` | `8` | Maximum concurrent sessions |
| `ASTROBURST_JOBS_MAX` | `4` | Maximum concurrent CPU-bound jobs |
| `ASTROBURST_CACHE_MAX_ENTRIES` | `32` | Per-session image cache slot limit |
| `ASTROBURST_CACHE_MAX_BYTES` | `2147483648` | Per-session image cache memory limit (bytes) |
| `ASTROBURST_CLEANUP_INTERVAL` | `60` | Seconds between idle-session sweep runs (values below 1 are raised to 1) |
| `ASTROBURST_LOG_LEVEL` | `info` | Log level (`trace`/`debug`/`info`/`warn`/`error`) |

`RUST_LOG` takes precedence over `ASTROBURST_LOG_LEVEL` when both are set.

Example — run with tighter limits and verbose logging:

```bash
ASTROBURST_SESSION_MAX=2 \
ASTROBURST_SESSION_TTL=300 \
ASTROBURST_LOG_LEVEL=debug \
./astroburst-server
```

---

## Session model

Every client starts with `POST /sessions` to obtain a `session_id`. All subsequent endpoints are scoped under `/sessions/:sid/`. A session holds:

- An **image cache** (LRU, configurable size) — loaded images live here under a slot key.
- A **job registry** — long-running operations (stacking, drizzle, pipeline) create a job and return immediately; the caller polls or streams progress.

Sessions expire after `ASTROBURST_SESSION_TTL` seconds of inactivity. Sessions with at least one running job are never evicted; this includes a cancelled job whose worker has not exited yet. The maximum number of concurrent sessions is `ASTROBURST_SESSION_MAX`; a `POST /sessions` when the cap is reached returns `503 Service Unavailable`. `DELETE /v2/sessions/:sid` removes the session and cancels every job it still runs.

---

## API reference

### Response format

All JSON responses follow this envelope:

```json
{ "success": false, "error": { "code": "not_found", "message": "slot ghost not in cache" } }
```

On success the envelope is absent — handlers return their data directly as the top-level JSON object.

Errors use standard HTTP status codes:

| Status | Code | Meaning |
|---|---|---|
| 400 | `bad_request` | Missing or invalid parameter (including an out-of-range HDU and an empty or out-of-image viewport) |
| 404 | `not_found` | Session, slot, job, or file on disk does not exist |
| 409 | `conflict` | SSE stream already has a subscriber |
| 429 | `too_many_requests` | All job semaphore slots occupied; the message is `job queue full (max N concurrent)` with N = `ASTROBURST_JOBS_MAX` |
| 503 | `service_unavailable` | Session cap reached |
| 500 | `internal_error` | Unexpected server error |

Every response includes an `X-Request-Id` header (echoed from the request, or a generated UUID).

---

### Health

#### `GET /health`

Returns server status and session counts. No session required.

```json
{
  "status": "ok",
  "version": "0.5.3",
  "sessions_active": 2,
  "sessions_total": 17
}
```

---

### Sessions

#### `POST /sessions`

Create a new session.

**Response `201 Created`:**
```json
{ "session_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890" }
```

Returns `503` when `ASTROBURST_SESSION_MAX` is reached.

---

### FITS I/O

#### `POST /sessions/:sid/fits/open`

Load a FITS or ASDF file from the server's filesystem into the session cache.

**Body:**
```json
{
  "path": "/data/m51_ha.fits",
  "slot": "ha"
}
```

`slot` is the cache key used by subsequent endpoints. Defaults to `path` if omitted. A path that does not exist returns `404`. Opening an existing slot again reloads it from disk.

`stats` is computed over every finite pixel (exact zeros and negative values included), the same rule as `/v2/.../stats`. `stf` is derived from the valid-pixel statistics, which leave out NaN/Inf and exact-zero padding.

**Response `200`:**
```json
{
  "slot": "ha",
  "dims": [2048, 2048],
  "stats": { "min": 0.0, "max": 65535.0, "median": 412.3, "mean": 580.1, "sigma": 210.4, "mad": 98.2, "valid_count": 4194304 },
  "stf": { "shadow": 0.001, "midtone": 0.22, "highlight": 1.0 },
  "header": { "OBJECT": "M51", "FILTER": "Ha", "EXPTIME": 300.0 }
}
```

#### `POST /sessions/:sid/fits/header`

Return the full header for a slot already in the cache. Returns `404` if the slot was loaded without header data — re-open with `fits/open`.

**Body:**
```json
{ "slot": "ha" }
```

**Response `200`:**
```json
{
  "slot": "ha",
  "total_cards": 42,
  "cards": [{ "key": "OBJECT", "value": "M51" }, ...],
  "index": { "OBJECT": "M51", "FILTER": "Ha" }
}
```

---

### Rendering

All render endpoints return a raw PNG (`Content-Type: image/png`). The STF auto-render also sets `X-Stf-Params` on the response.

#### `POST /sessions/:sid/image/stf`

Auto-stretch the image using the Screen Transfer Function and return a PNG.

**Body:**
```json
{ "slot": "ha" }
```

#### `POST /sessions/:sid/image/render`

Render with explicit STF parameters.

**Body:**
```json
{ "slot": "ha", "shadow": 0.001, "midtone": 0.22, "highlight": 1.0 }
```

#### `POST /sessions/:sid/image/viewport`

Render a pixel-coordinate crop as a PNG.

**Body:**
```json
{ "slot": "ha", "x": 512, "y": 512, "w": 256, "h": 256 }
```

`w` and `h` must be at least 1 and `(x, y)` must lie inside the image; otherwise the request is a `400`. A crop that runs past the right or bottom edge is clipped to the image.

---

### Jobs

Long-running operations (stacking, drizzle, pipeline) return `202 Accepted` immediately with a `job_id`. Use these endpoints to track progress.

#### `GET /sessions/:sid/jobs/:jid`

Poll job status.

**Response `200`:**
```json
{
  "id": "...",
  "action": "stack",
  "status": "running",
  "pct": 42,
  "started_at": 1718000000000,
  "completed_at": null
}
```

`status` is one of: `running`, `done`, `error`, `cancelled`.

#### `DELETE /sessions/:sid/jobs/:jid`

Cancel a running job. Returns the same shape as GET. The status becomes `cancelled` at once and the job's SSE stream receives its terminal `cancelled` event. The worker stops at its next cancellation check: between frames while frames or calibration masters load, and inside the combine (per frame, per row chunk and per channel for stacking, drizzle and the pipeline). The job's queue slot is released when the worker exits, so a new job may briefly get `429` right after a cancel.

No-op if the job already finished (`done`, `error` or `cancelled`): the status is not changed. A cancel that arrives while the finished result is being stored is also ignored and the job ends `done`.

#### `GET /sessions/:sid/jobs/:jid/stream`

Subscribe to real-time SSE progress. Events have a `type` field:

```
event: progress
data: {"type":"progress","pct":45,"stage":"loading"}

event: complete
data: {"type":"complete"}

event: error
data: {"type":"error","message":"file not found: /data/bad.fits"}

event: cancelled
data: {"type":"cancelled"}
```

Every job ends with exactly one terminal event (`complete`, `error` or `cancelled`), after which the server closes the stream. Progress events can be dropped when the client reads slowly; the terminal event is always delivered. A worker that panics ends with an `error` event whose message starts with `job worker panicked:`.

Only one subscriber per job is allowed. A second `GET` to the same stream returns `409 Conflict`.

---

### Stacking

Both endpoints return `202 Accepted` and start a background job. Parameters are validated before the job is queued, so a bad request gets `400` even when the queue is full. Progress stages: `loading` (0 to 60 %), `stacking` or `drizzling` (60 %), `storing` (90 %).

#### `POST /sessions/:sid/stacking/stack`

Sigma-clip stack a list of FITS files.

**Body:**
```json
{
  "paths": ["/data/ha_001.fits", "/data/ha_002.fits", "/data/ha_003.fits"],
  "result_slot": "ha_stacked",
  "sigma_low": 3.0,
  "sigma_high": 3.0,
  "max_iterations": 5,
  "align": true,
  "weights": [1.0, 1.0, 0.5]
}
```

All fields except `paths` are optional. `result_slot` defaults to `"stacked"`.

Further optional fields: `align_method` (`"affine"`; any other value uses phase correlation), `rejection` (`none`, `sigma_clip`, `winsorized_sigma_clip`, `linear_fit_clip`, `percentile_clip`, `min_max`), `combine` (`mean`, `median`, `min`, `max`), `normalization` (`none`, `additive`, `multiplicative`, `additive_scaling`, `multiplicative_scaling`), `rejection_normalization` (`none`, `scale_offset`), `winsor_cutoff`, `percentile_low`, `percentile_high`, `minmax_low`, `minmax_high`, `rejection_maps`. An unknown method name is a `400` that lists the supported names.

Limits (each violation is a `400`):
- `sigma_low` and `sigma_high` must be finite numbers greater than 0; `winsor_cutoff`, `percentile_low` and `percentile_high` must be finite.
- `weights`, when given, must hold exactly one weight per path, each finite and not negative.
- With `rejection: "min_max"`, `minmax_low + minmax_high` must be smaller than the number of frames.

**Response `202`:**
```json
{ "job_id": "...", "status": "running", "slot": "ha_stacked" }
```

#### `POST /sessions/:sid/stacking/drizzle`

Drizzle-combine a list of FITS files.

**Body:**
```json
{
  "paths": ["/data/ha_001.fits", "/data/ha_002.fits"],
  "result_slot": "ha_drizzled",
  "scale": 2.0,
  "pixfrac": 0.7,
  "kernel": "lanczos3",
  "sigma_low": 3.0,
  "sigma_high": 3.0,
  "align": true
}
```

All fields except `paths` are optional. `result_slot` defaults to `"drizzled"`.
`kernel` is one of: `"square"` (default), `"gaussian"`, `"lanczos3"`. `rejection` takes the same names as the stack endpoint.
`scale`, `pixfrac`, `sigma_low` and `sigma_high` must be finite numbers greater than 0 (`400` otherwise). Drizzle needs at least 2 frames; with one frame the job ends in `error`.

**Response `202`:**
```json
{ "job_id": "...", "status": "running", "slot": "ha_drizzled" }
```

---

### Pipeline

#### `POST /sessions/:sid/pipeline/run`

Calibrate and stack one or more narrowband channels in a single pass. Builds calibration masters (bias, dark, flat) from supplied frames, then stacks each channel's lights. Returns `202 Accepted`.

**Body:**
```json
{
  "channels": [
    { "label": "Ha",   "paths": ["/data/ha_001.fits",   "/data/ha_002.fits"] },
    { "label": "OIII", "paths": ["/data/oiii_001.fits", "/data/oiii_002.fits"] }
  ],
  "bias_paths":  ["/data/bias_001.fits"],
  "dark_paths":  ["/data/dark_300s.fits"],
  "flat_paths":  ["/data/flat_ha.fits"],
  "sigma_low":   2.5,
  "sigma_high":  3.0,
  "normalize":   true,
  "result_prefix": "pipe/"
}
```

All calibration arrays and numeric fields are optional. `result_prefix` prepends a string to each channel label to form the cache slot key (e.g. `"pipe/Ha"`, `"pipe/OIII"`).

Further optional fields: `align` (default `true`), `rejection` and `combine` (same names as the stack endpoint), `cosmetic` (cosmetic-correction settings) and `dark_optimize` (default `false`). `sigma_low` and `sigma_high` must be finite numbers greater than 0, and an unknown method name or an invalid `cosmetic` block is a `400`; all of this is checked before the job is queued.

Progress stages: `building masters` (5 %), `loading lights` (15 %), `stacking` (25 %), `storing` (90 %). A cancel stops the job between master and light frames and inside the per-channel combine.

**Response `202`:**
```json
{ "job_id": "...", "status": "running", "slots": ["pipe/Ha", "pipe/OIII"], "warnings": [] }
```

`warnings` is non-empty when `cosmetic` is set and some lights carry a DQ plane.
