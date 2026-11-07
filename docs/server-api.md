# AstroBurst headless server API

The headless server (`astroburst-server`), its v1 and v2 APIs and the async Python client were contributed by [Jae-Joon Lee](https://github.com/leejjoon) (@leejjoon). His files carry his attribution line and keep it.

This file is the reference for every route the server registers today, v1 and v2. [`src-tauri/SERVER.md`](../src-tauri/SERVER.md) is the contributor's original v1 guide and stays as he wrote it; where the two differ, this file follows the current code. A test, `src-tauri/src/server/route_docs.rs`, fails when this file and the routers disagree (`src-tauri/src/server/router.rs` and the science router it merges, `src-tauri/src/server/science/mod.rs`; any other merged, nested or service router is reported as unreadable): a registered route without a `METHOD /path` heading or a row in the route index, a heading or row for a route that is not registered, or a wrong route count. It also checks a few limits stated here (the drizzle clamps, the body limit, `HEAD` on the job stream) against the code. Example values in the JSON blocks are illustrative, not measurements.

## Contents

1. [Overview](#1-overview)
2. [Building and starting the server](#2-building-and-starting-the-server)
3. [Configuration](#3-configuration)
4. [Security](#4-security)
5. [Sessions, images and references](#5-sessions-images-and-references)
6. [Jobs](#6-jobs)
7. [Errors](#7-errors)
8. [Route index](#8-route-index)
9. [Service routes](#9-service-routes)
10. [v1 routes](#10-v1-routes)
11. [v2 routes](#11-v2-routes)
12. [Region specifications](#12-region-specifications)
13. [Planned routes](#13-planned-routes)
14. [Python client](#14-python-client)
15. [Credits and licence](#15-credits-and-licence)

## 1. Overview

`astroburst-server` is a standalone HTTP server (Axum) built from the same Rust crate as the desktop app, behind the `server` feature. It shares the FITS and ASDF readers, the display stretch and colormap code, the WCS engine (including the JWST and Roman gWCS evaluator), the region geometry and the stacking, drizzle and calibration engines with the desktop app. It renders on the CPU and has no GPU dependency. Images live in per-session caches on the server; clients send JSON and receive JSON or PNG.

One process serves two API generations:

- **v1** (paths without a version prefix): sessions, open and header by *slot*, STF renders, and three background jobs (stack, drizzle and the calibration pipeline) with status, cancel and server-sent events.
- **v2** (paths under `/v2`): session lifecycle with keepalive, image *references* with HDU or ASDF-array selection, structure, header and WCS inspection, coordinate transforms and a coordinate grid, cutouts, block binning, a pixel probe, exact statistics with noise evaluation, histograms and a render that takes the desktop display settings; also the job routes with each job's outcome, the plane list of FITS and ASDF files, deleting a ref and downloading it as FITS, a folder listing under the path roots, the server's capabilities, seeded synthetic star fields with their truth catalogue, and cube preparation recipes (the LIGHTSTACK workflow, from crops to a PSF-matched cube) with the filter registry they name planes by.

Both generations use the same sessions and the same per-session image cache, so a v1 slot can be read by a v2 route through its `ref`, and a v2 ref can be rendered by a v1 route through its `slot`.

Conventions:

- Pixel coordinates are 0-based, `x` is the column and `y` the row; integer values are pixel centres. A pixel belongs to a region when its centre lies inside it (the DS9 rule). DS9 `.reg` files count from 1; the server never does. The one exception is `crpix` in the WCS summary, which keeps the FITS header's 1-based value.
- Sky coordinates are in degrees, ICRS unless a `frame` field says otherwise.
- `dims` is always `[width, height]`.
- Non-finite numbers (NaN, infinity) are serialised as `null`.
- Request bodies are `application/json`. Fields the server does not know are ignored; fields of the wrong type are refused by the framework (section 7).
- Method and option names (`align_method`, `kernel`, `rejection`, `combine`, `normalization`, `rejection_normalization`, `frame`, `scale.algorithm`, `scale.stretch`, `colormap`) are trimmed and case-insensitive. Tagged fields (`type` of a region or of a separation, `shape`, `system`, a defect `kind`, cosmetic `replacement`) and the bin `method` must be written exactly as shown, in lower case.
- Every response carries an `X-Request-Id` header: the one the client sent, or a new UUID.
- Every `GET` route also answers `HEAD` by running the `GET` handler. On `GET /sessions/:sid/jobs/:jid/stream` that takes the job's one subscription (section 6), so do not probe the stream with `HEAD`.

## 2. Building and starting the server

Both feature sets below build the same routes (section 8).

The recommended build is the full feature set, the desktop's default features without `tauri`, with the server's own release profile:

```bash
cd src-tauri
cargo build --profile release-server \
  --no-default-features \
  --features server,astrometry-net,asdf-full,vizier,mast
```

The contributor's original command (`src-tauri/SERVER.md`) stays valid; it leaves out `mast` and uses `--release`:

```bash
cd src-tauri
cargo build --release \
  --no-default-features \
  --features server,astrometry-net,asdf-full,vizier
```

The binary is `target/release-server/astroburst-server` (`target/release/` with `--release`; `astroburst-server.exe` on Windows). For development: `cargo run --bin astroburst-server --no-default-features --features server,astrometry-net,asdf-full,vizier,mast`.

| Feature | Effect on the server |
|---|---|
| `server` | Required: the binary, Axum, sessions and jobs |
| `asdf-full` | Reads bzip2 and lz4 compressed ASDF blocks (zlib is always available) |
| `vizier` | Gaia DR3 cone queries through VizieR in the library; no server route queries a catalogue yet |
| `astrometry-net` | The astrometry.net client is desktop-only today; the feature changes nothing on the server |
| `mast` | MAST archive engine; the server reads the MAST settings from the environment at start-up (section 3); no server route uses MAST yet, and `GET /v2/capabilities` reports only whether `ASTROBURST_MAST_TOKEN` is set |

Start-up log:

```
[2026-10-23T12:00:00Z INFO  astroburst_server] AstroBurst Headless Server v0.8.0
[2026-10-23T12:00:00Z INFO  astroburst_server] Listening on 127.0.0.1:8080
[2026-10-23T12:00:00Z INFO  astroburst_server] Session TTL: 900s, Max sessions: 8
[2026-10-23T12:00:00Z INFO  astroburst_server] Per-session cache: 32 entries / 2048 MiB
[2026-10-23T12:00:00Z INFO  astroburst_server] Cleanup interval: 60s
```

Remote use: keep the default loopback bind on the machine that holds the data and reach it through an SSH tunnel, `ssh -L 8080:localhost:8080 user@remote-host`; the client then talks to `http://localhost:8080`.

Notes:

- The `release-server` profile (`src-tauri/Cargo.toml`) is the desktop's `release` profile with `panic = "unwind"`, so the job runner's panic guard (section 6) works in that build: a panicking job ends with an `error` event and the server keeps running. The `release` profile, which the desktop app ships with, sets `panic = "abort"`; a server built with `--release` ends the whole process on a panic inside a job.
- At start-up the server creates the Gaia catalogue cache folder (`ASTROBURST_CATALOG_CACHE_DIR`, section 3) if it does not exist.

## 3. Configuration

All settings are environment variables. An invalid server setting prints `WARN: <VARIABLE>=<value> is invalid (...); using default ...` on standard error and falls back to the default; the server never refuses to start because of one (`server/config.rs`).

| Variable | Default | Meaning |
|---|---|---|
| `ASTROBURST_BIND` | `127.0.0.1:8080` | Address and port to listen on |
| `ASTROBURST_SESSION_TTL` | `900` | Idle seconds before a session is evicted |
| `ASTROBURST_SESSION_MAX` | `8` | Maximum concurrent sessions |
| `ASTROBURST_JOBS_MAX` | `4` | Maximum concurrent background jobs, for the whole server |
| `ASTROBURST_CACHE_MAX_ENTRIES` | `32` | Images per session cache |
| `ASTROBURST_CACHE_MAX_BYTES` | `2147483648` | Bytes per session cache; also the cap for one cutout |
| `ASTROBURST_CLEANUP_INTERVAL` | `60` | Seconds between idle-session sweeps; values below 1 become 1 |
| `ASTROBURST_LOG_LEVEL` | `info` | `trace`, `debug`, `info`, `warn`, `error`; `RUST_LOG` wins when set |

Guard and path roots (section 4). The token, the `Host` check and the roots of the file paths that requests send (`fits/open`, `/v2/open`, the v1 stack, drizzle and pipeline jobs and cube preparation recipes) take effect when `ASTROBURST_BIND` is not a loopback address; on a loopback bind those stay open as before and the server only logs a warning when `ASTROBURST_TOKEN` is set. `GET /v2/fs` and `GET /v2/sessions/:sid/planes` keep to the roots on every bind. An empty value counts as unset.

| Variable | Default | Meaning |
|---|---|---|
| `ASTROBURST_TOKEN` | none | Bearer token every request must carry (`Authorization: Bearer <token>`); without it a non-loopback bind answers every request with `503 guard_unconfigured` |
| `ASTROBURST_ALLOWED_HOSTS` | none | Comma-separated host names or addresses (a port is ignored, IPv6 in brackets or bare) that the `Host` header may name, besides `localhost`, `127.0.0.1`, `[::1]` and the bind address; an entry that is not a host name is logged and ignored |
| `ASTROBURST_SERVER_ROOT` | the working directory | Path root: on a non-loopback bind `fits/open`, `/v2/open`, the v1 stack, drizzle and pipeline jobs and cube preparation recipes accept only files under a root; the relative paths of a cube preparation recipe are read from this folder; on every bind `GET /v2/fs` lists only folders under a root (this one by default) and `GET /v2/sessions/:sid/planes` reads only source files under a root. A value that is not an existing folder is logged at start-up and is then not a root, with no fallback to the working directory: until it is fixed, paths are accepted only under `ASTROBURST_OUTPUT_ROOT` or `ASTROBURST_MAST_DOWNLOAD_DIR` when those name existing folders, and nowhere when they do not. Not to be confused with `ASTROBURST_DATA_DIR`, the app data folder |
| `ASTROBURST_OUTPUT_ROOT` | `<ASTROBURST_SERVER_ROOT>/out` | Second path root: each cube preparation job writes its folder here (the first job creates it), and the planned routes will write their products here; a root only once the folder exists |
| `ASTROBURST_MAST_DOWNLOAD_DIR` | `<ASTROBURST_SERVER_ROOT>/mast` | Third path root, for the planned MAST downloads (`mast` builds only); a root only once the folder exists |
| `ASTROBURST_MAST_TOKEN` | none | Read by `mast` builds for the planned MAST routes (section 13); no route uses it yet, `GET /v2/capabilities` reports only whether it is set, and it is never printed |

Engine variables that the server process also reads:

| Variable | Default | Meaning |
|---|---|---|
| `ASTROBURST_MEMORY_BUDGET_MB` | detected available memory | Memory budget in MiB for the stack and drizzle memory checks; a positive number |
| `ASTROBURST_IO_MODE` | `auto` | File access: `auto`, `mmap` or `read`; an invalid value logs a warning and uses `auto` |
| `ASTROBURST_VIZIER_URL` | `https://vizier.cds.unistra.fr/viz-bin/asu-tsv` | VizieR ASU-TSV endpoint, read at start-up; no current route queries it |
| `ASTROBURST_CATALOG_CACHE_DIR` | `astroburst/catalog-cache` under the OS cache folder | Disk cache of Gaia DR3 cones, read and created at start-up. The default is the folder the desktop app uses on that machine; point it elsewhere for a server other people use, and at a temporary folder for tests. The cache is never shipped or pre-filled |

MAST variables, read at start-up only by a server built with `mast`. An invalid value is logged as a warning and ignored. No server route uses MAST yet; the settings are in place for the planned MAST routes (section 13).

| Variable | Accepted values | Meaning |
|---|---|---|
| `ASTROBURST_MAST_URL` | `https://<host>.stsci.edu` or `http://127.0.0.1:<port>` | MAST API base URL |
| `ASTROBURST_MAST_AUTH_URL` | same rule | MAST authentication base URL |
| `ASTROBURST_MAST_TIMEOUT_MS` | positive integer | Request timeout override in milliseconds |
| `ASTROBURST_MAST_RETRY_MS` | positive integer | Retry back-off in milliseconds |
| `ASTROBURST_TEST_DATA_DIR` | absolute path | Moves the whole AstroBurst configuration folder (the config file and the stored API keys) to `<path>/config`; honoured only together with an accepted `ASTROBURST_MAST_URL` |

Example, tighter limits and verbose logs:

```bash
ASTROBURST_SESSION_MAX=2 ASTROBURST_SESSION_TTL=300 ASTROBURST_LOG_LEVEL=debug ./astroburst-server
```

## 4. Security

Read this before exposing the server to anything but your own machine. The guard is one layer in front of every route, v1 and v2 (`server/science/guard.rs`).

- **Loopback bind** (the default, `127.0.0.1:8080`, or any `127.x` or `::1` address): open as before. No token, no `Host` check, and the `path` and `paths` fields are not confined: any file the server process can read can be opened or stacked. Without the `Host` check a web page that rebinds its own DNS name to `127.0.0.1` can reach a loopback server from the browser of the same machine; do not keep the server running while browsing untrusted sites.
- **Any other bind** (for example `0.0.0.0:8080`), checked in this order for every route, `/health` included:
  1. `ASTROBURST_TOKEN` must be set, else every request answers `503 guard_unconfigured`.
  2. The `Host` header (and the authority of an absolute request URI) must name `localhost`, `127.0.0.1`, `[::1]`, the bind address or an entry of `ASTROBURST_ALLOWED_HOSTS`; a missing or foreign `Host` is `421 host_not_allowed`. This also blocks DNS rebinding.
  3. `Authorization: Bearer <token>` must match `ASTROBURST_TOKEN` (the scheme is case-insensitive; both tokens are compared as SHA-256 digests in constant time, so the timing reveals neither the token nor its length), else `401 unauthorized` with `WWW-Authenticate: Bearer`.
  4. Every file path a request sends must resolve inside a path root: `ASTROBURST_SERVER_ROOT`, `ASTROBURST_OUTPUT_ROOT` and, in `mast` builds, `ASTROBURST_MAST_DOWNLOAD_DIR`. That is the `path` of `POST /sessions/:sid/fits/open` and `POST /v2/sessions/:sid/open`, the `paths` of `POST /sessions/:sid/stacking/stack` and `.../stacking/drizzle`, each channel's `paths` and the `bias_paths`, `dark_paths`, `flat_paths` and `flat_dark_paths` of `.../pipeline/run`, and every path of a cube preparation recipe (each input `path`, a crop `region_file`, the PSF `files` and an empirical `data_dir`, then the image and star file an empirical PSF finds in that folder) once its relative paths are read from `ASTROBURST_SERVER_ROOT`. One path outside refuses the whole request before any other field is checked and before a job slot is taken. The path is resolved by the file system before the comparison, so `..`, symbolic links, Windows junctions, `\\?\` drive prefixes and drive-relative forms all end where the file really is; a path that resolves outside every root, or cannot be resolved, is `403 path_outside_root`. A missing file inside a root keeps its `404`. The message names the path that was sent, never a root. On Windows a path that names another machine or a device is refused before the file system is asked, so the server never contacts the host it names: `\\server\share` and `//server/share`, `\\?\UNC\...`, `\\.\...`, `\??\...`, a `\\?\` form other than a drive letter, and any other path that starts with two separators. The one exception is a path on the network share (`\\server\share`, compared without regard to case) of a root that itself lies on that share; a UNC path to a folder that is also a local root is refused.
- **Every bind, loopback included:** `GET /v2/fs` lists only a folder that resolves inside a path root, and `GET /v2/sessions/:sid/planes` reads only a ref whose source file resolves inside one; anything else is `403 path_outside_root`, checked the same way as above. On a loopback bind a file outside the roots can still be opened, but `/planes` refuses it.
- Not checked again on any bind: `POST /v2/sessions/:sid/hdu`, which re-opens the source file of a ref that was already opened, and `GET /v2/sessions/:sid/images/:ref/fits`, which writes the pixels the session already holds and reads the ERR and DQ planes from that same source file. On a non-loopback bind that file passed the check above when it was opened, and v1 job results come only from checked paths.
- Error bodies and logs never contain a token.
- Python client: pass an `httpx.AsyncClient` that carries the header, `AstroBurstClient(base_url, client=httpx.AsyncClient(base_url=..., headers={"Authorization": "Bearer <token>"}))`.
- `GET /v2/fs` answers with the full path of the folder it lists, so an authorised client learns where the path roots are; refusals never name a root.
- Prefer the loopback bind and an SSH tunnel for remote work; if you bind to another interface, set `ASTROBURST_TOKEN`, `ASTROBURST_ALLOWED_HOSTS` and `ASTROBURST_SERVER_ROOT`.
- AstroBurst is AGPLv3. The network clause (section 13 of the licence) applies when a modified server is offered to others over a network.

## 5. Sessions, images and references

**Lifecycle.**

- `POST /sessions` and `POST /v2/sessions` create a session (the same handler) and answer `201` with `{"session_id": "<uuid>"}`, or `503 service_unavailable` when `ASTROBURST_SESSION_MAX` sessions exist.
- Every session-scoped request refreshes the session's last-access time; `POST /v2/sessions/:sid/keepalive` does only that. A request to an unknown session is `404 not_found`.
- A sweep every `ASTROBURST_CLEANUP_INTERVAL` seconds removes sessions idle for longer than `ASTROBURST_SESSION_TTL`. A session with an active job is never evicted, including a cancelled job whose worker has not exited yet (`server/session.rs`, `server/job.rs`).
- `DELETE /v2/sessions/:sid` removes the session at once and cancels its jobs. There is no v1 delete route.

**Image cache.** Each session has an LRU cache bounded by `ASTROBURST_CACHE_MAX_ENTRIES` and `ASTROBURST_CACHE_MAX_BYTES`. An evicted image is gone; the v2 metadata of evicted refs is dropped the next time the session or its image list is read. `DELETE /v2/sessions/:sid/images/:ref` frees one image at once.

**Slots and refs.** v1 calls a cache key a *slot*, v2 calls it a *ref*; both are keys of the same cache.

- v2 records metadata for the refs its own routes create (`source` path, plane, `dims`, `wcs_present`, `extname`, `is_dq`) and lists them at `GET /v2/sessions/:sid/images`.
- A v1 slot (from `fits/open` or a v1 job) is readable by every v2 route that takes `ref`, but it is not listed by `/images`, has no metadata and no source file. Results of v1 jobs are stored without a FITS header, so `/v2/.../header` has nothing to report for them and `/wcs` answers `present: false`.

**Active ref.** `open`, `hdu`, `cutout` and `bin` make their result the active ref. Routes with an optional `ref` use the active ref when it is omitted; without one they answer `400 bad_request` with `no active image in this session; open a file first`.

**Names.** Default ref names are `img_N` (open, hdu), `cutout_N`, `bin_N` and `synth_N`, with `N` a per-session counter that skips names in use; a `name` field chooses one. Reusing a name replaces that image.

**Planes.** A FITS file can be opened at any extension (`hdu`, the 0-based HDU index) and an ASDF file at any array (`array`, a dotted key such as `roman.dq`); give at most one of the two. The plane that was loaded is reported as `plane_ref`, `path#hdu=<n>` or `path#array=<key>`, the same reference form the desktop app accepts. Without either field the science plane is picked automatically.

**Derived refs.** Cutouts, binned images, v1 job results and the products of v2 jobs (section 6) are computed in memory. Their WCS comes only from their own header: a cutout keeps a shifted copy of the parent's header WCS, a binned image has no header and no WCS, a v1 job result has no header, a v2 job product carries the header its route wrote. A gWCS (JWST `_cal`, Roman ASDF) is evaluated only for refs opened from the file itself. A cutout records no source file, so `/structure` and `/hdu` refuse it; a binned ref records its parent's source file, so `/structure` lists that file and `/hdu` re-opens it. `/planes` takes only a ref opened from a file (`open` or `hdu`) and answers `400 derived_ref_unsupported` for any derived ref, binned ones included.

## 6. Jobs

Stack, drizzle and pipeline (v1), synthetic data (`POST /v2/sessions/:sid/synth`, section 11.9) and cube preparation (`POST /v2/sessions/:sid/cubeprep/jobs`, section 11.10) run as background jobs. The request is validated first (names, numbers, weights and the one-shot-colour check where it applies); then a slot of the server-wide job pool (`ASTROBURST_JOBS_MAX`) is taken, and if none is free the answer is `429 too_many_requests` (`job queue full (max N concurrent)`). The pipeline inspects its frames after taking the slot, so a full pool answers `429` before the frames are read, and a refused frame set releases the slot at once. An accepted job answers `202` with its `job_id`.

The routes below serve every job of the session, whichever route started it, and each of the three v1 job routes has a v2 alias under `/v2/sessions/:sid/jobs/:jid` with the same handler. An unknown job is `404 not_found` with the message `job <jid> not found`. Job records end with their session, and a session keeps at most 256 finished jobs: when a v2 job finishes, and on every `GET .../outcome`, the server drops the oldest finished jobs of each session beyond the newest 256 (by completion time) together with their outcomes. A running job is never dropped. Without v2 jobs or outcome reads nothing is dropped.

### GET /sessions/:sid/jobs/:jid

Status snapshot (`server/handlers/jobs.rs`):

```json
{ "id": "6f0c2a8e-3d1b-4f5e-9a7c-1b2d3e4f5a6b", "action": "stack", "status": "running", "pct": 42, "started_at": 1792769423186, "completed_at": null }
```

`action` is `stack`, `drizzle`, `pipeline`, `synth` or `cubeprep`. `status` is `running`, `done`, `error` or `cancelled`. `pct` stays at 99 or below until the job is `done`, which sets it to 100. Times are milliseconds since the Unix epoch.

### DELETE /sessions/:sid/jobs/:jid

Cancels the job and answers `200` with the status snapshot. The job is marked `cancelled` at once and the terminal `cancelled` event is sent. The worker stops at its next cancellation check; its pool slot is released only when it exits, so a new job can briefly get `429` right after a cancel. A cancel after the job finished, or while its result is being stored, changes nothing and returns the current snapshot (`server/job.rs`).

### GET /sessions/:sid/jobs/:jid/stream

Server-sent events (`text/event-stream`). One subscriber per job; a second subscription answers `409 conflict`. A `HEAD` request on this route runs the same handler and takes that one subscription, so a later `GET` answers `409` and nobody can read the job's events; check a job with `GET /sessions/:sid/jobs/:jid` instead. Events are buffered from the moment the job starts, so a subscriber that connects late still receives them. Event names and data:

```
event: progress
data: {"type":"progress","pct":45,"stage":"loading"}

event: complete
data: {"type":"complete"}

event: error
data: {"type":"error","message":"Failed to load /data/bad.fits: ..."}

event: cancelled
data: {"type":"cancelled"}
```

Every job ends with exactly one terminal event (`complete`, `error` or `cancelled`), after which the stream closes; progress events can be dropped when the client reads slowly, the terminal event never is. A `: keep-alive` comment is sent every 30 seconds. A worker that panics in a debug build ends with an `error` event whose message starts with `job worker panicked:`; a worker that returns without a result ends with `job worker ended without a result`.

**Results.** A finished v1 job leaves its image in the session cache under the slot names returned in the `202` body. The status route carries no result payload, no warnings and no error text; for a v1 job the error text is only in the SSE `error` event. A v2 job records its result, warnings, products and error text, read with `GET /v2/sessions/:sid/jobs/:jid/outcome`.

### GET /v2/sessions/:sid/jobs/:jid

Same handler as `GET /sessions/:sid/jobs/:jid`, the same status snapshot (`server/science/mod.rs` registers the alias).

### DELETE /v2/sessions/:sid/jobs/:jid

Same handler as `DELETE /sessions/:sid/jobs/:jid`. A v2 job cancelled before it stores its products publishes nothing and records no outcome, even when its worker finishes later; a `synth` stack stops between frames.

### GET /v2/sessions/:sid/jobs/:jid/stream

Same handler as `GET /sessions/:sid/jobs/:jid/stream`, with the same events. The one subscription per job is shared by both paths: after a subscription on either path, `GET` on the other answers `409 conflict`. `HEAD` takes the subscription here too.

### GET /v2/sessions/:sid/jobs/:jid/outcome

What a v2 job produced (`server/science/outcome.rs`). Example for a finished `synth` job:

```json
{
  "job": { "id": "6f0c2a8e-3d1b-4f5e-9a7c-1b2d3e4f5a6b", "action": "synth", "status": "done", "pct": 100, "started_at": 1792769423186, "completed_at": 1792769423911 },
  "outcome": {
    "result": { "width": 512, "height": 512, "star_count": 200, "frames": 1, "catalog_csv": "id,x,y,z,flux_e,flux_adu,temperature\n0,103.6182,12.0496,0.0000,4210.7795,2807.1863,8137.4\n", "manifest_csv": null },
    "warnings": [],
    "products": [ { "ref": "synth_0", "present": true }, { "ref": "synth_0_truth", "present": false } ],
    "error": null
  }
}
```

- `job` is the status snapshot of `GET /sessions/:sid/jobs/:jid`.
- `outcome` is `null` until the job has stored its result, after a cancel, for a v1 job (stack, drizzle, pipeline) and for a job whose worker panicked (its message is only in the SSE `error` event). A v2 job records its outcome just before its status turns `done` or `error`: a v2 job that reads `done` always has one, and for a moment an outcome can show while `status` still reads `running`.
- `result` is the route's own result object (each route documents it). A failed job has `result: null`, no warnings, no products, and `error` set to the same text as the SSE `error` event. When the failure has a code, `error_code` holds it, with `error_hint` when there is advice to give (cube preparation, section 11.10); both keys are absent otherwise.
- `products` lists the refs the job published, in order. They are listed by `GET /v2/sessions/:sid/images` with `source: null` and read by every route that takes a `ref`. `present` says whether the session cache still holds an image under that name: products can be evicted by the per-session cache (section 5) or deleted.
- An unknown job, or one dropped by the 256-job limit above, is `404 not_found` (`job <jid> not found`).

## 7. Errors

Every error from a handler uses one envelope (`server/error.rs`); successful responses have no envelope.

```json
{ "success": false, "error": { "code": "wcs_required", "message": "image img_0 has no usable WCS: ...", "hint": "the header must carry a valid WCS (CTYPE/CRPIX/CRVAL/CD...)" } }
```

| Status | `code` | When |
|---|---|---|
| 400 | `bad_request` | Missing or invalid parameter, unknown method name, refused frames, HDU out of range, no active image |
| 400 | `wcs_required` | A sky region or a WCS route on an image without a usable celestial WCS |
| 400 | `region_out_of_bounds` | A region outside the image, a cutout larger than the cache budget, pixel points that do not project onto the sky |
| 400 | `pixel_out_of_bounds` | `/v2/sessions/:sid/pixel` outside the image |
| 400 | `not_implemented` | `render_png: true` on `/v2/sessions/:sid/histogram` |
| 400 | `derived_ref_unsupported` | `/planes` on a ref that was not opened from a file (a cutout, binned image, job product or v1 slot) |
| 401 | `unauthorized` | Non-loopback bind: missing or wrong `Authorization: Bearer` token (section 4); the answer carries `WWW-Authenticate: Bearer` |
| 403 | `path_outside_root` | Non-loopback bind: a `path` of `fits/open` or `/v2/open`, or a path of a v1 stack, drizzle or pipeline body or of a cube preparation recipe, that resolves outside every path root; any bind: a `dir` of `/v2/fs`, or the source file of a ref given to `/planes`, outside every root; on Windows also a path that names another machine or a device (section 4) |
| 400 | `recipe_version_unsupported`, `unknown_step`, `unknown_parameter`, `filter_duplicate`, `recipe_invalid` | A cube preparation recipe refused (section 11.10) |
| 404 | `not_found` | Session, slot, ref, job, folder or file does not exist |
| 404 | `file_not_found` | A file or folder that a cube preparation recipe names does not exist |
| 409 | `conflict` | Second SSE subscriber for a job |
| 421 | `host_not_allowed` | Non-loopback bind: missing or foreign `Host` header (section 4) |
| 429 | `too_many_requests` | Job pool full |
| 503 | `service_unavailable` | Session cap reached |
| 503 | `guard_unconfigured` | Non-loopback bind without `ASTROBURST_TOKEN` (section 4) |
| 500 | `internal_error` | Unexpected failure; the message carries the error chain |

`hint` appears only where a handler sets one: with `region_out_of_bounds`, `pixel_out_of_bounds` and `not_implemented`; usually with `wcs_required` (not when the image's WCS has a degenerate pixel scale); and with some `bad_request` answers of the v2 routes (an unknown sky frame, stretch, colormap or scale algorithm, a bin `method` other than `mean`, a bin `factor` larger than the image, and out-of-range parameters such as `density`, `bins`, `range`, `box` or `sigma_clip`); and with every cube preparation refusal but `recipe_invalid`. Treat `hint` as optional. A job that fails after it was accepted answers no error envelope: its code, when it has one, is the outcome's `error_code` (section 6).

The guard's refusals (`401`, `403`, `421` and `503 guard_unconfigured`, section 4) come before the handler too, and they use the envelope. Other requests rejected before a handler runs get the framework's own plain-text answer, not the envelope: a body larger than 2 MiB (2,097,152 bytes, axum's default limit; the router sets no other) (`413`), a body that is not valid JSON (`400`), a missing `Content-Type: application/json` (`415`), JSON of the wrong shape, such as a string where a number is expected, a negative number for an unsigned field or an unknown region `type` (`422`), a malformed query string (`400`), an unknown path (`404`, empty body) and a wrong method (`405`, empty body). Clients should handle both forms.

## 8. Route index

45 method and path pairs (`server/router.rs` and the science router it merges, `server/science/mod.rs`). `GET /v2/capabilities` lists the same pairs in the same order.

| # | Method | Path | Answer | Section |
|---|---|---|---|---|
| 1 | GET | [`/health`](#get-health) | JSON | 9 |
| 2 | POST | [`/sessions`](#post-sessions) | 201 JSON | 10.1 |
| 3 | POST | [`/sessions/:sid/fits/open`](#post-sessionssidfitsopen) | JSON | 10.2 |
| 4 | POST | [`/sessions/:sid/fits/header`](#post-sessionssidfitsheader) | JSON | 10.2 |
| 5 | POST | [`/sessions/:sid/image/render`](#post-sessionssidimagerender) | PNG | 10.3 |
| 6 | POST | [`/sessions/:sid/image/stf`](#post-sessionssidimagestf) | PNG | 10.3 |
| 7 | POST | [`/sessions/:sid/image/viewport`](#post-sessionssidimageviewport) | PNG | 10.3 |
| 8 | GET | [`/sessions/:sid/jobs/:jid`](#get-sessionssidjobsjid) | JSON | 6 |
| 9 | DELETE | [`/sessions/:sid/jobs/:jid`](#delete-sessionssidjobsjid) | JSON | 6 |
| 10 | GET | [`/sessions/:sid/jobs/:jid/stream`](#get-sessionssidjobsjidstream) | SSE | 6 |
| 11 | POST | [`/sessions/:sid/stacking/stack`](#post-sessionssidstackingstack) | 202 job | 10.4 |
| 12 | POST | [`/sessions/:sid/stacking/drizzle`](#post-sessionssidstackingdrizzle) | 202 job | 10.4 |
| 13 | POST | [`/sessions/:sid/pipeline/run`](#post-sessionssidpipelinerun) | 202 job | 10.4 |
| 14 | POST | [`/v2/sessions`](#post-v2sessions) | 201 JSON | 11.1 |
| 15 | GET | [`/v2/sessions/:sid`](#get-v2sessionssid) | JSON | 11.1 |
| 16 | DELETE | [`/v2/sessions/:sid`](#delete-v2sessionssid) | 204 | 11.1 |
| 17 | POST | [`/v2/sessions/:sid/keepalive`](#post-v2sessionssidkeepalive) | JSON | 11.1 |
| 18 | POST | [`/v2/sessions/:sid/open`](#post-v2sessionssidopen) | JSON | 11.2 |
| 19 | POST | [`/v2/sessions/:sid/hdu`](#post-v2sessionssidhdu) | JSON | 11.2 |
| 20 | GET | [`/v2/sessions/:sid/images`](#get-v2sessionssidimages) | JSON | 11.2 |
| 21 | GET | [`/v2/sessions/:sid/structure`](#get-v2sessionssidstructure) | JSON | 11.3 |
| 22 | GET | [`/v2/sessions/:sid/header`](#get-v2sessionssidheader) | JSON | 11.3 |
| 23 | GET | [`/v2/sessions/:sid/wcs`](#get-v2sessionssidwcs) | JSON | 11.3 |
| 24 | POST | [`/v2/sessions/:sid/cutout`](#post-v2sessionssidcutout) | JSON | 11.5 |
| 25 | POST | [`/v2/sessions/:sid/wcs/pix2sky`](#post-v2sessionssidwcspix2sky) | JSON | 11.4 |
| 26 | POST | [`/v2/sessions/:sid/wcs/sky2pix`](#post-v2sessionssidwcssky2pix) | JSON | 11.4 |
| 27 | POST | [`/v2/sessions/:sid/wcs/separation`](#post-v2sessionssidwcsseparation) | JSON | 11.4 |
| 28 | POST | [`/v2/sessions/:sid/wcs/grid`](#post-v2sessionssidwcsgrid) | JSON | 11.4 |
| 29 | POST | [`/v2/sessions/:sid/bin`](#post-v2sessionssidbin) | JSON | 11.5 |
| 30 | POST | [`/v2/sessions/:sid/pixel`](#post-v2sessionssidpixel) | JSON | 11.6 |
| 31 | POST | [`/v2/sessions/:sid/stats`](#post-v2sessionssidstats) | JSON | 11.6 |
| 32 | POST | [`/v2/sessions/:sid/histogram`](#post-v2sessionssidhistogram) | JSON | 11.6 |
| 33 | POST | [`/v2/sessions/:sid/render`](#post-v2sessionssidrender) | PNG | 11.7 |
| 34 | GET | [`/v2/sessions/:sid/jobs/:jid`](#get-v2sessionssidjobsjid) | JSON | 6 |
| 35 | DELETE | [`/v2/sessions/:sid/jobs/:jid`](#delete-v2sessionssidjobsjid) | JSON | 6 |
| 36 | GET | [`/v2/sessions/:sid/jobs/:jid/stream`](#get-v2sessionssidjobsjidstream) | SSE | 6 |
| 37 | GET | [`/v2/sessions/:sid/jobs/:jid/outcome`](#get-v2sessionssidjobsjidoutcome) | JSON | 6 |
| 38 | GET | [`/v2/sessions/:sid/planes`](#get-v2sessionssidplanes) | JSON | 11.3 |
| 39 | DELETE | [`/v2/sessions/:sid/images/:ref`](#delete-v2sessionssidimagesref) | 204 | 11.2 |
| 40 | GET | [`/v2/sessions/:sid/images/:ref/fits`](#get-v2sessionssidimagesreffits) | FITS | 11.2 |
| 41 | GET | [`/v2/fs`](#get-v2fs) | JSON | 11.8 |
| 42 | GET | [`/v2/capabilities`](#get-v2capabilities) | JSON | 9 |
| 43 | POST | [`/v2/sessions/:sid/synth`](#post-v2sessionssidsynth) | 202 job | 11.9 |
| 44 | GET | [`/v2/cubeprep/filters`](#get-v2cubeprepfilters) | JSON | 11.10 |
| 45 | POST | [`/v2/sessions/:sid/cubeprep/jobs`](#post-v2sessionssidcubeprepjobs) | 202 job | 11.10 |

## 9. Service routes

### GET /health

No session needed (`server/router.rs`).

```json
{ "status": "ok", "version": "0.8.0", "sessions_active": 2, "sessions_total": 17 }
```

`sessions_total` counts every session created since start.

### GET /v2/capabilities

What this server offers (`server/science/capabilities.rs`). No session needed.

```json
{
  "version": "0.8.0",
  "features": { "mast": true, "astrometry_net": true, "vizier": true, "asdf_full": true },
  "limits": { "session_max": 8, "session_ttl_seconds": 900, "jobs_max": 4, "cache_max_entries": 32, "cache_max_bytes": 2147483648 },
  "routes": [ { "method": "GET", "path": "/health" }, { "method": "POST", "path": "/sessions" } ],
  "mast_token": { "present": false }
}
```

- `features` are the Cargo features the binary was built with (section 2).
- `limits` are the values in force of `ASTROBURST_SESSION_MAX`, `ASTROBURST_SESSION_TTL`, `ASTROBURST_JOBS_MAX`, `ASTROBURST_CACHE_MAX_ENTRIES` and `ASTROBURST_CACHE_MAX_BYTES` (section 3).
- `routes` lists every method and path pair of the route index (section 8), in its order; a test checks the list against the router.
- `mast_token` is present only in a build with `mast`. It says whether `ASTROBURST_MAST_TOKEN` is set; the token itself is never returned. Whether MAST accepts it is not checked yet.
- The offline catalogue status is not reported: no route reads a catalogue yet.

## 10. v1 routes

### 10.1 Sessions

#### POST /sessions

No body. Same handler as `POST /v2/sessions` (section 5). `201`:

```json
{ "session_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890" }
```

`503 service_unavailable` with `session cap reached (max 8)` when the cap is reached.

### 10.2 Files

#### POST /sessions/:sid/fits/open

Loads a FITS or ASDF file from the server's file system into the session cache (`server/handlers/io.rs`). v1 always loads the automatically chosen science plane; use `POST /v2/sessions/:sid/open` to choose an HDU or an ASDF array.

```json
{ "path": "/data/m16/656nmos.fits", "slot": "ha" }
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `path` | string | required | Path on the server: `.fits`, `.fit`, `.fts`, their `.gz` forms or `.asdf`; a `.zip` opens its first supported image. On a non-loopback bind it must resolve inside a path root, else `403 path_outside_root` (section 4) |
| `slot` | string | the path | Cache key; opening an existing slot again reloads it |

`200`:

```json
{
  "slot": "ha",
  "dims": [1600, 1600],
  "stats": { "min": -12.4, "max": 4021.7, "median": 61.2, "mad": 9.8, "sigma": 14.5, "mean": 70.3, "valid_count": 2560000 },
  "stf": { "shadow": 0.0012, "midtone": 0.21, "highlight": 1.0 },
  "header": { "TELESCOP": "HST", "INSTRUME": "WFPC2", "FILTER": "F656N" }
}
```

`stats` covers every finite pixel, zeros and negatives included; `sigma` is 1.4826 times the MAD, not the standard deviation. `stf` is the auto-stretch suggestion, computed from the valid-pixel statistics that leave out NaN, infinity and exact-zero padding. `header` maps each value keyword of the loaded plane to its value as a string (an empty object when the plane has no value keywords). A missing file is `404`; an HDU or array problem is `400`; any other read failure is `500 internal_error`.

#### POST /sessions/:sid/fits/header

Full header of a slot already in the cache (`server/handlers/io.rs`).

```json
{ "slot": "ha" }
```

`200`:

```json
{ "slot": "ha", "total_cards": 42, "cards": [{ "key": "TELESCOP", "value": "HST" }, { "key": "HISTORY", "value": "calibrated with calwf2" }], "index": { "FILTER": "F656N", "TELESCOP": "HST" } }
```

`cards` lists the cards in file order, `COMMENT` and `HISTORY` lines included; `index` maps each value keyword to its value. `404` when the slot is not cached, or when it has no header (v1 job results, binned refs): re-open the file with `fits/open`.

### 10.3 Rendering

All three return `Content-Type: image/png`, 8-bit greyscale, through the MTF screen transfer function (`server/handlers/render.rs`). `shadow`, `midtone` and `highlight` are in 0 to 1, with `shadow` and `highlight` relative to the image's valid-pixel minimum and maximum; NaN, infinity and exact-zero pixels render black. An unknown slot is `404`.

#### POST /sessions/:sid/image/render

Render with explicit STF parameters; all three are required.

```json
{ "slot": "ha", "shadow": 0.0012, "midtone": 0.21, "highlight": 1.0 }
```

#### POST /sessions/:sid/image/stf

Auto-stretch render.

```json
{ "slot": "ha" }
```

The applied parameters are returned in the `X-Stf-Params` header, for example `{"shadow":0.0012,"midtone":0.21,"highlight":1.0}`.

#### POST /sessions/:sid/image/viewport

Render a pixel crop.

```json
{ "slot": "ha", "x": 512, "y": 512, "w": 256, "h": 256, "shadow": 0.0012, "midtone": 0.21, "highlight": 1.0 }
```

`x`, `y`, `w` and `h` are non-negative integers; `w` and `h` must be at least 1 and `(x, y)` must lie inside the image (`400` otherwise); a crop running past the right or bottom edge is clipped. With all three STF fields the crop is stretched with them against the whole image's statistics; with any of them missing the crop gets its own auto-stretch.

### 10.4 Stacking and pipeline

These three routes start a job (section 6) and answer `202`. Their inputs are FITS files on the server (`.gz` accepted, `path#hdu=<n>` selects an extension); ASDF and ZIP inputs are not read by the job routes. On a non-loopback bind every path of the body must resolve inside a path root; one path outside answers `403 path_outside_root` before any other field is checked (section 4). Their results are stored in the session cache without a FITS header and without the per-frame alignment report; the planned v2 routes of group G6 return both.

#### POST /sessions/:sid/stacking/stack

Stack a list of files with pixel rejection (`server/handlers/stacking.rs`).

```json
{
  "paths": ["/data/ha_001.fits", "/data/ha_002.fits", "/data/ha_003.fits"],
  "result_slot": "ha_stacked",
  "rejection": "winsorized_sigma_clip",
  "combine": "mean",
  "sigma_low": 4.0,
  "sigma_high": 3.0,
  "align": true,
  "align_method": "phase_correlation"
}
```

| Field | Default | Accepted values and limits |
|---|---|---|
| `paths` | required | Non-empty list of files |
| `result_slot` | `"stacked"` | Cache key of the result |
| `sigma_low`, `sigma_high` | 4.0, 3.0 | Finite and greater than 0 |
| `max_iterations` | 5 | |
| `align` | `true` | With alignment, one-shot-colour (Bayer or X-Trans) lights are refused with `400` |
| `align_method` | `phase_correlation` | `phase_correlation` or `affine` (star-based); any other value is `400 bad_request` naming both, for example `unknown align_method 'zncc' (supported: phase_correlation, affine)`, and no job is queued |
| `weights` | none | One finite, non-negative weight per path |
| `rejection` | `sigma_clip` | `none`, `sigma_clip` (`sigma`), `winsorized_sigma_clip` (`winsorized`), `linear_fit_clip` (`linear_fit`), `percentile_clip` (`percentile`), `min_max` (`minmax`) |
| `combine` | `mean` | `mean` (`average`), `median`, `min` (`minimum`), `max` (`maximum`) |
| `normalization` | `additive_scaling` | `none`, `additive`, `multiplicative`, `additive_scaling` (`additive_with_scaling`), `multiplicative_scaling` (`multiplicative_with_scaling`) |
| `rejection_normalization` | `scale_offset` | `none`, `scale_offset` (`scale+offset`) |
| `winsor_cutoff` | 5.0 | Finite |
| `percentile_low`, `percentile_high` | 0.2, 0.1 | Finite |
| `minmax_low`, `minmax_high` | 1, 1 | With `min_max`, their sum must be below the number of frames |
| `rejection_maps` | `false` | Accepted; the v1 route stores only the stacked image |

Names in parentheses are accepted aliases. An unknown name in any of these fields is `400` listing the accepted names, checked before the job pool, so it is refused even when the pool is full. Progress stages: `loading` (0 to 60 %), `stacking` (60 %), `storing` (90 %). The memory check runs inside the job: a refusal ends the job in `error`, a warning is written to the server log only.

`202`:

```json
{ "job_id": "6f0c2a8e-3d1b-4f5e-9a7c-1b2d3e4f5a6b", "status": "running", "slot": "ha_stacked" }
```

#### POST /sessions/:sid/stacking/drizzle

Drizzle-combine a list of files (`server/handlers/stacking.rs`).

```json
{ "paths": ["/data/ha_001.fits", "/data/ha_002.fits"], "result_slot": "ha_drizzled", "scale": 2.0, "pixfrac": 0.7, "kernel": "lanczos3" }
```

| Field | Default | Notes |
|---|---|---|
| `paths` | required | Non-empty; fewer than 2 frames ends the job in `error`; one-shot-colour lights are always refused (`400`). Frames of different sizes are cropped to the smallest height and width (the first rows and columns are kept) when no height or width differs by more than 5 % of the larger of the smallest height and the smallest width, rounded down to whole pixels; a larger difference ends the job in `error` |
| `result_slot` | `"drizzled"` | |
| `scale`, `pixfrac` | 2.0, 0.7 | Finite and greater than 0, else `400`. The engine then runs with `scale` clamped to 1 to 4 and `pixfrac` clamped to 0.1 to 1, without a warning: `"scale": 8` gives an image 4 times the input size, `"pixfrac": 0.01` runs as 0.1 |
| `kernel` | `square` | `square`, `gaussian`, `lanczos3` (alias `lanczos`); any other value is `400 bad_request` naming them, for example `unknown kernel 'turbo' (supported: square, gaussian, lanczos3, lanczos)`, and no job is queued |
| `sigma_low`, `sigma_high` | 3.0, 3.0 | Finite and greater than 0 |
| `align` | `true` | Phase correlation, cross-checked against a star-triangle match when both frames have enough stars, so a rotated frame is registered by the star match instead of a wrong shift |
| `rejection` | `sigma_clip` | Same names as the stack route |

`202`: `{ "job_id": "...", "status": "running", "slot": "ha_drizzled" }`. Stages: `loading`, `drizzling`, `storing`.

#### POST /sessions/:sid/pipeline/run

Calibrate and stack one or more channels in one pass (`server/handlers/pipeline.rs`).

```json
{
  "channels": [
    { "label": "Ha",   "paths": ["/data/ha_001.fits", "/data/ha_002.fits"] },
    { "label": "OIII", "paths": ["/data/oiii_001.fits", "/data/oiii_002.fits"] }
  ],
  "bias_paths": ["/data/bias_001.fits"],
  "dark_paths": ["/data/dark_300s_001.fits"],
  "flat_paths": ["/data/flat_ha_001.fits"],
  "flat_dark_paths": [],
  "result_prefix": "pipe/",
  "cosmetic": { "use_master_dark": true, "dark_hot_sigma": 3.0 }
}
```

| Field | Default | Notes |
|---|---|---|
| `channels` | required | Non-empty list of `{label, paths}`; the result slot of a channel is `result_prefix + label` |
| `bias_paths`, `dark_paths`, `flat_paths`, `flat_dark_paths` | none | Calibration frames; masters are built from them |
| `sigma_low`, `sigma_high` | 2.5, 3.0 | Finite and greater than 0 |
| `normalize` | `true` | Normalise frames before stacking |
| `align` | `true` | With alignment, one-shot-colour lights are refused (`400`); phase correlation is cross-checked against a star match as in drizzle |
| `rejection`, `combine` | `sigma_clip`, `mean` | Same names as the stack route |
| `cosmetic` | none | Cosmetic correction after dark subtraction; `{}` turns it on with the defaults: `use_master_dark` (`true`), `dark_hot_sigma` (3.0), `dark_cold_sigma`, `auto_hot_sigma`, `auto_cold_sigma`, `defects` (`[{"kind": "point", "x", "y"}`, `{"kind": "column", "x", "y0"?, "y1"?}`, `{"kind": "row", "y", "x0"?, "x1"?}]`), `cfa`, `amount` (1.0), `replacement` (`median` or `mean`) |
| `dark_optimize` | `false` | Scale the dark by noise minimisation |
| `result_prefix` | `""` | |

After the pool slot is taken the frames are inspected: darks of different exposures, flats of different filters, mixed binning, mixed `GAIN` or `OFFSET` in a master set, lights binned unlike the masters and light frames in a calibration row are refused with `400`; softer problems come back as `warnings`. Stages: `building masters` (5 %), `loading lights` (15 %), `stacking` (25 %), `storing` (90 %).

`202`:

```json
{ "job_id": "...", "status": "running", "slots": ["pipe/Ha", "pipe/OIII"], "warnings": [] }
```

## 11. v2 routes

### 11.1 Sessions

#### POST /v2/sessions

No body. Same handler as `POST /sessions`: `201 {"session_id": "..."}`, `503` at the cap.

#### GET /v2/sessions/:sid

Session summary (`server/v2/sessions.rs`).

```json
{ "session_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890", "active_ref": "img_0", "image_count": 3, "cache_bytes": 30720000 }
```

`active_ref` is `null` when there is none or it was evicted. `image_count` counts the v2 refs; `cache_bytes` estimates the whole session cache, v1 slots included.

#### DELETE /v2/sessions/:sid

Removes the session and cancels its jobs. `204`, no body.

#### POST /v2/sessions/:sid/keepalive

No body. Refreshes the idle timer: `{"session_id": "a1b2c3d4-...", "status": "ok"}`.

### 11.2 Images

#### POST /v2/sessions/:sid/open

Opens a plane of a file as a new ref and makes it active (`server/v2/images.rs`).

```json
{ "path": "/data/jw02739/jw02739-o001_t001_nircam_clear-f444w_i2d.fits", "hdu": 1, "name": "f444w" }
```

| Field | Default | Notes |
|---|---|---|
| `path` | required | FITS (also `.gz`) or ASDF on the server; a `.zip` opens its first supported image. On a non-loopback bind it must resolve inside a path root, else `403 path_outside_root` (section 4) |
| `hdu` | auto | FITS extension index; exclusive with `array` |
| `array` | auto | ASDF array key, for example `roman.dq`; exclusive with `hdu` |
| `name` | `img_N` | Ref name; an existing ref of that name is replaced |

`200`, the image description:

```json
{
  "ref": "f444w",
  "active_ref": "f444w",
  "dims": [7065, 4177],
  "hdu": 1,
  "array": null,
  "plane_ref": "/data/jw02739/jw02739-o001_t001_nircam_clear-f444w_i2d.fits#hdu=1",
  "is_dq": false,
  "extname": "SCI",
  "wcs_present": true,
  "stats": { "min": -0.41, "max": 1390.8, "median": 0.53, "mad": 0.15, "sigma": 0.23, "mean": 0.98, "valid_count": 28745012 },
  "header": { "TELESCOP": "JWST", "INSTRUME": "NIRCAM", "BUNIT": "MJy/sr" }
}
```

`hdu` or `array` names the plane that was loaded, also when it was chosen automatically. `stats` and `header` are as in `fits/open`. `404` when the file does not exist; `400` for both `hdu` and `array`, `hdu` on an ASDF file, `array` on a FITS file or an HDU out of range.

#### POST /v2/sessions/:sid/hdu

Re-opens the active ref's source file at another plane, as a new ref that becomes active (`server/v2/images.rs`). Body `{"hdu": 3}` or `{"array": "roman.dq"}`, optional `name`. Exactly one of `hdu` and `array` is required. The response is the image description of `open`. `400` when there is no active ref or the active ref has no source file (a cutout or a v1 slot).

#### GET /v2/sessions/:sid/images

Lists the refs v2 knows about, sorted by name (`server/v2/images.rs`).

```json
{
  "active_ref": "cutout_0",
  "count": 2,
  "images": [
    { "image_ref": "cutout_0", "source": null, "hdu": null, "array": null, "plane_ref": "cutout_0", "is_dq": false, "width": 512, "height": 512, "wcs_present": true, "extname": "SCI" },
    { "image_ref": "f444w", "source": "/data/jw02739/..._i2d.fits", "hdu": 1, "array": null, "plane_ref": "/data/jw02739/..._i2d.fits#hdu=1", "is_dq": false, "width": 7065, "height": 4177, "wcs_present": true, "extname": "SCI" }
  ]
}
```

#### DELETE /v2/sessions/:sid/images/:ref

Frees one image of the session (`server/science/images.rs`): its cache entry and its v2 metadata, and the active ref when it was this one, so the session then has no active ref. Any cache key can be freed, a v1 slot included (percent-encode a `/` in the name). `204`, no body; `404 not_found` (`image ref <ref> not found in session`) when the session holds no image of that name. A deleted job product reports `present: false` in its job's outcome.

#### GET /v2/sessions/:sid/images/:ref/fits

Downloads a ref as a FITS file (`server/science/images.rs`): `Content-Type: application/fits`, `Content-Length`, and `Content-Disposition: attachment; filename="<name>.fits"`. The name is built from the ref: every character other than an ASCII letter, a digit or `-` becomes `_`, a run of `_` becomes one, `_` at either end is dropped, the result is cut to 64 characters, and an empty result becomes `plane` (ref `a__b` downloads as `a_b.fits`). Float pixels are the 32-bit floats the session holds, written with `BITPIX = -32`, so the file reads back bit for bit, NaN and negative zero included.

- A ref opened from a file whose plane has ERR or DQ companions (the planes `pixel` reports, section 11.6) downloads as a multi-extension file: an empty primary HDU, then `SCI` (the ref's pixels and header), `ERR` and `DQ` (the companion planes and their headers), each with `EXTVER = 1`. DQ keeps its integer values: unsigned 32-bit (`BITPIX = 32`, `BZERO = 2147483648`) or signed as in the source. The companions are read from the ref's source file into the session cache, as `pixel` does.
- A DQ plane opened by itself downloads as an empty primary HDU and one `DQ` extension (`EXTVER = 1`) with the plane's header and its integer values, written as in the multi-extension case, so every flag bit is kept (DQ 2147483649 stays 2147483649).
- Any other ref (a plane without companions, an ERR plane opened by itself, a derived ref, a v1 slot) downloads as one image HDU with the ref's header when it has one. That file also gets `PROGRAM = 'AstroBurst <version>'`, unless the header names a `PROGRAM`, and one `HISTORY Processed with AstroBurst <version>` card.

The layout cards of the stored header (`BITPIX`, `NAXISn`, `BZERO`, `BSCALE` and the like) are written anew for the new file. Each extension of a multi-extension download gets its own `EXTNAME` and `EXTVER`; a one-HDU download keeps the `EXTNAME` and `EXTVER` of the ref's header, when it has them, in its primary header as they were. The file is written to a temporary file that is deleted once the download ends. `404 not_found` for an unknown ref.

### 11.3 Inspection

The four routes take the query parameter `ref` (alias `image`), defaulting to the active ref.

#### GET /v2/sessions/:sid/structure

HDU list of the ref's source FITS file (`server/v2/inspect.rs`). Example: `GET /v2/sessions/<sid>/structure?ref=f444w`.

```json
{
  "ref": "f444w",
  "count": 3,
  "hdus": [
    { "index": 0, "extname": "PRIMARY", "extver": 1, "naxis": 0, "shape": [], "bitpix": 8, "dtype": "uint8", "has_data": false },
    { "index": 1, "extname": "SCI", "extver": 1, "naxis": 2, "shape": [4177, 7065], "bitpix": -32, "dtype": "float32", "has_data": true },
    { "index": 2, "extname": "ERR", "extver": 1, "naxis": 2, "shape": [4177, 7065], "bitpix": -32, "dtype": "float32", "has_data": true }
  ]
}
```

`shape` is in numpy order (rows first) and lists at most three axes. `dtype` is `uint8`, `int16`, `int32`, `int64`, `float32`, `float64` or `unknown`. FITS only: a ref opened from an ASDF file is `400`, and so is a ref with no source file (a cutout, a v1 slot or a name v2 does not know). `GET /v2/sessions/:sid/planes` lists ASDF arrays too.

#### GET /v2/sessions/:sid/planes

Every HDU of the FITS file, or every array of the ASDF file, that holds the ref's plane (`server/science/inspect.rs`), as the engine's plane list (`list_planes`, the one the desktop's extension list reads) reports them. Example: `GET /v2/sessions/<sid>/planes?ref=f444w`.

```json
{
  "ref": "f444w",
  "format": "fits",
  "count": 4,
  "planes": [
    { "index": 0, "extname": "PRIMARY", "extver": null, "naxis": 0, "shape": [], "bitpix": 8, "has_data": false, "plane_ref": "/data/jw02739/..._i2d.fits#hdu=0", "is_dq": false, "is_err": false },
    { "index": 1, "extname": "SCI", "extver": 1, "naxis": 2, "shape": [4177, 7065], "bitpix": -32, "has_data": true, "plane_ref": "/data/jw02739/..._i2d.fits#hdu=1", "is_dq": false, "is_err": false },
    { "index": 2, "extname": "ERR", "extver": 1, "naxis": 2, "shape": [4177, 7065], "bitpix": -32, "has_data": true, "plane_ref": "/data/jw02739/..._i2d.fits#hdu=2", "is_dq": false, "is_err": true },
    { "index": 3, "extname": "DQ", "extver": 1, "naxis": 2, "shape": [4177, 7065], "bitpix": 32, "has_data": true, "plane_ref": "/data/jw02739/..._i2d.fits#hdu=3", "is_dq": true, "is_err": false }
  ]
}
```

- `format` is `fits` or `asdf`. For an ASDF file, `extname` is the array's dotted key and `plane_ref` uses `#array=<key>`; the list is the engine's, so a Roman file also lists its `roman` group key next to `roman.data` and `roman.dq`.
- `plane_ref` opens that plane with `POST /v2/sessions/:sid/open` (`path` and `hdu` or `array`) or anywhere the desktop accepts a reference.
- `is_dq` and `is_err` follow the names the engine treats as DQ and ERR planes, in any case: `DQ` or `MASK`, or a key whose last dotted part is `dq`; `ERR`, or a key whose last dotted part is `err`.
- `shape` is in numpy order and lists at most four axes. `bitpix` is the FITS `BITPIX`; for an ASDF array it is the `BITPIX` of its data type, the same for a signed and an unsigned type, so no `dtype` is derived from it.
- The ref must be opened from a file (`open` or `hdu`): any derived ref is `400 derived_ref_unsupported`, and a source file outside every path root is `403 path_outside_root`, on any bind (section 4). A source file deleted since it was opened is `404`.

#### GET /v2/sessions/:sid/header

Header cards of the ref's plane (`server/v2/inspect.rs`). `keys` is an optional comma-separated list of case-insensitive glob patterns (`*`, `?`), for example `GET /v2/sessions/<sid>/header?ref=f444w&keys=EXPTIME,FILT*,DATE-*`. The first occurrence of each keyword is kept, so only the first `HISTORY` or `COMMENT` line appears; the keys of `cards` come back in alphabetical order (use `fits/header` for file order).

```json
{ "ref": "f444w", "count": 2, "cards": { "EXPTIME": { "value": "1803.776" }, "FILTER": { "value": "F444W" } } }
```

`400` when the image carries no header (v1 job results, binned refs); `404` for an unknown ref.

#### GET /v2/sessions/:sid/wcs

WCS summary (`server/v2/inspect.rs`). Without a header or a usable celestial WCS: `{"ref": "...", "present": false}`. `404` for an unknown ref.

```json
{
  "ref": "f444w",
  "present": true,
  "projection": "TAN",
  "crpix": [3533.0, 2089.0],
  "crval": [274.7106, -13.8296],
  "cd": [[-1.7e-5, 0.0], [0.0, 1.7e-5]],
  "pixel_scale_arcsec": 0.0629,
  "pixel_scale_x_arcsec": 0.0629,
  "pixel_scale_y_arcsec": 0.0629,
  "rotation_deg": 0.0,
  "flipped": false,
  "parity": "normal",
  "sip_present": false,
  "sip_max_err_px": null,
  "sip_inv_err_px": null,
  "north_vec": [0.0, 1.0],
  "east_vec": [-1.0, 0.0],
  "wcs_kind": "header",
  "gwcs_steps": null,
  "gwcs_frames": null,
  "gwcs_source": null,
  "gwcs_vs_header_sip_max_mas": null,
  "gwcs_vs_header_sip_max_px": null,
  "gwcs_refusal": null
}
```

`crpix` keeps the FITS 1-based convention; `cd` is in degrees per pixel; `north_vec` and `east_vec` are unit vectors in pixel space (`null` when they cannot be computed); `parity` is `normal` or `flipped`. For a JWST `_cal` or a Roman ASDF image opened from its file, `projection` reads `gWCS`, `wcs_kind` is `gwcs` and the `gwcs_*` fields describe the transform chain and its distance from the header SIP; `gwcs_refusal` explains why a gWCS was not used. There is no image centre or field of view in this summary yet.

### 11.4 Coordinates

All four take an optional `ref` (alias `image_ref`; `/grid` also `image`) and, except the sky form of `/separation`, need a celestial WCS (`400 wcs_required` otherwise). Points are `[x, y]` pixels or `[ra, dec]` degrees. `on_image` is true when the point lies within `[-0.5, width - 0.5) x [-0.5, height - 0.5)`.

A request body may not exceed 2 MiB (section 7), so one `points` list holds roughly 70,000 to 100,000 pairs, depending on how many digits each number is written with; split longer lists across requests.

#### POST /v2/sessions/:sid/wcs/pix2sky

Pixel to sky (`server/v2/wcs.rs`).

```json
{ "points": [[100.0, 200.0], [3500.5, 2088.0]], "frame": "galactic" }
```

`frame`: `icrs` (default), `fk5`, `galactic`, `ecliptic`; another name is `400` with the list in `hint`. `ra` and `dec` hold the longitude and latitude in that frame; a point that does not project, such as one outside a gWCS bounding box, returns `null` coordinates.

```json
{ "ref": "f444w", "frame": "galactic", "count": 2, "results": [ { "x": 100.0, "y": 200.0, "ra": 16.95, "dec": 0.79, "on_image": true } ] }
```

#### POST /v2/sessions/:sid/wcs/sky2pix

Sky to pixel (`server/v2/wcs.rs`). ICRS input only; a `frame` field is ignored.

```json
{ "points": [[274.7106, -13.8296]] }
```

```json
{ "ref": "f444w", "count": 1, "results": [ { "ra": 274.7106, "dec": -13.8296, "x": 3532.0, "y": 2088.0, "on_image": true } ] }
```

#### POST /v2/sessions/:sid/wcs/separation

Angular separation (`server/v2/wcs.rs`). Two forms, chosen by `type`:

```json
{ "type": "sky", "a": [274.70, -13.83], "b": [274.72, -13.81] }
```

```json
{ "type": "pixel", "a": [100, 200], "b": [400, 600], "ref": "f444w" }
```

```json
{ "separation_deg": 0.0275, "separation_arcmin": 1.65, "separation_arcsec": 99.0 }
```

The sky form needs no image. The pixel form adds `ref`, `a_sky` and `b_sky`; a point that does not project is `400 region_out_of_bounds`. There is no position angle yet.

#### POST /v2/sessions/:sid/wcs/grid

Coordinate grid lines and edge labels (`server/v2/wcs.rs`), computed by the same `core::astrometry::grid::wcs_grid` as the desktop grid.

```json
{ "frame": "icrs", "density": 3 }
```

`frame` as in `pix2sky`; `density` 1 to 5 (default 3, `400` outside). Response:

```json
{
  "ref": "f444w",
  "frame": "icrs",
  "lines": [ { "kind": "lon", "value_deg": 274.70, "points": [[12.4, 0.0], [13.1, 50.0]], "label": "18h18m48s" } ],
  "labels": [ { "x": 12.4, "y": 0.0, "text": "18h18m48s", "edge": "bottom", "kind": "lon" } ],
  "lon_step_deg": 0.0125,
  "lat_step_deg": 0.0167,
  "notes": []
}
```

`kind` is `lon` or `lat`; `edge` is `left`, `right`, `top` or `bottom`.

### 11.5 Cutouts and binning

#### POST /v2/sessions/:sid/cutout

Cuts a region into a new ref that becomes active (`server/v2/cutout.rs`).

```json
{ "region": { "type": "sky", "ra": 274.7106, "dec": -13.8296, "size_arcmin": [1.0, 0.8] }, "name": "pillar_tip", "preserve_wcs": true, "mask_outside": false }
```

| Field | Default | Notes |
|---|---|---|
| `region` | required | Any region form of section 12; a shape is cut on its bounding box |
| `ref` (alias `image_ref`) | active ref | Source image |
| `name` | `cutout_N` | |
| `preserve_wcs` | `true` | Keep the parent header with `CRPIX` shifted and `LTV1`/`LTV2` recorded; `false` gives an empty header and no WCS |
| `mask_outside` | `false` | With a shape region, pixels outside the shape become NaN |

The response is the image description of `open` (with `hdu` and `array` `null` and `plane_ref` equal to the ref name) plus `fraction_on_image`, `region` (`{x, y, width, height}` and, for a shape, `shape`), `ltv1` and `ltv2`. A pixel box may run past the edges or lie wholly outside: pixels outside the parent are NaN (a cutout wholly outside reports `fraction_on_image: 0` and `valid_count: 0`) and `clip` is not used. A sky box wholly outside the image, or whose centre does not project, is `400 region_out_of_bounds`; a cutout larger than `ASTROBURST_CACHE_MAX_BYTES` is `400 region_out_of_bounds`; a sky region without WCS is `400 wcs_required`. The cutout is in memory only; `GET /v2/sessions/:sid/images/:ref/fits` downloads its pixels and header as one image HDU, while the science cutout file with ERR and DQ planes is desktop-only today.

#### POST /v2/sessions/:sid/bin

Block-mean binning into a new ref that becomes active (`server/v2/bin.rs`).

```json
{ "factor": 4, "method": "mean", "name": "f444w_bin4" }
```

| Field | Default | Notes |
|---|---|---|
| `factor` | required | Integer of at least 1, no larger than the smaller image dimension (`400` otherwise) |
| `method` | `"mean"` | Only `mean` is accepted (`400` otherwise) |
| `ref` (alias `image_ref`) | active ref | Source image |
| `name` | `bin_N` | |

NaN pixels are left out of each block (a block with no finite pixel is NaN), and trailing rows or columns that do not fill a block are dropped. The result has no header and no WCS.

```json
{ "ref": "f444w_bin4", "active_ref": "f444w_bin4", "from_ref": "f444w", "factor": 4, "method": "mean", "dims": [1766, 1044], "wcs_present": false, "stats": { "min": -0.1, "max": 410.2, "median": 0.53, "mad": 0.07, "sigma": 0.11, "mean": 0.97, "valid_count": 1843704 } }
```

### 11.6 Measurement

#### POST /v2/sessions/:sid/pixel

Pixel value with neighbourhood statistics, units, sky position and companion planes (`server/v2/pixel.rs`), from the same `core::imaging::pixel_probe` as the desktop readout.

```json
{ "x": 2831.4, "y": 1155.0, "box": 5 }
```

`x` and `y` are rounded to the nearest pixel centre; `box` is an odd positive integer (default 1); `ref` (alias `image_ref`) defaults to the active ref.

```json
{
  "ref": "f444w",
  "x": 2831, "y": 1155,
  "value": 688.07, "unit": "MJy/sr",
  "box": 5,
  "neighborhood": { "min": 512.2, "max": 1390.8, "mean": 734.5, "median": 688.07, "n_pixels": 25, "n_nan": 0 },
  "dq": { "bits": 0, "value": 0, "names": [], "table": "jwst", "text": "0: GOOD" },
  "err": { "value": 0.27, "unit": "MJy/sr" },
  "wavelength": null,
  "sky": { "ra": 274.71, "dec": -13.83 }
}
```

`value` is `null` for a NaN pixel; `unit` comes from `BUNIT`; `sky` is ICRS, `null` without a WCS. `dq`, `err` and `wavelength` are `null` when the file has no such companion plane (`DQ`, `ERR`, a same-size `WAVELENGTH` extension, or the ASDF siblings); companions exist only for refs opened from a file. The DQ `table` is `jwst`, `roman`, `hst` or `unknown`; a DQ plane of an unknown instrument reports raw `BITn` names. Outside the image: `400 pixel_out_of_bounds`; an even `box`: `400 bad_request`.

#### POST /v2/sessions/:sid/stats

Exact statistics over finite pixels, optionally in a region, with sigma clipping, percentiles and noise evaluation (`server/v2/stats.rs`).

```json
{ "region": { "type": "shape", "shape": "circle", "x": 256.1, "y": 256.2, "r": 6 }, "sigma_clip": { "sigma": 3.0, "maxiters": 5 }, "percentiles": [5, 50, 95], "noise": true }
```

| Field | Default | Notes |
|---|---|---|
| `ref` (alias `image_ref`) | active ref | |
| `region` | whole image | Section 12; a pixel or sky box must fit and a shape must not run past the edge unless `clip: true` |
| `sigma_clip` | none | `sigma` (3.0, finite and greater than 0), `maxiters` (5, 1 to 100) |
| `percentiles` | none | Up to 100 values in percent, 0 to 100 |
| `noise` | `false` | k-sigma multiresolution noise evaluation |

Response fields:

- `ref`; `region`: the resolved box `{x, y, width, height, clipped}`, or for a shape the shape itself with its pixel `bounds` (`{x0, y0, x1, y1}`) and `clipped`.
- `min`, `max`, `median`, `mad`, `mean`, `valid_count`, and `sigma` (1.4826 times the MAD).
- `std_dev` (sample standard deviation), `avg_dev` (mean absolute deviation from the median), `bwmv_sqrt` (square root of the biweight midvariance), `count_fraction` (share of the region's pixels that are finite), and `n_nan` and `nan_count` (both the number of non-finite pixels in the region).
- With a shape: `sum` of the finite pixels inside and the geometric `area` in pixels.
- With `sigma_clip`: a `clipped` block `{mean, median, std, n_rejected}`.
- With `percentiles`: a list of `{percentile, value}`.
- With `noise`: a `noise` block `{sigma, fraction, iterations, method}` and `noise_note`; a region with fewer than 64 finite pixels returns `noise: null` and the reason in `noise_note`.

Zeros and negatives are included. DQ exclusion is not available on the server yet.

#### POST /v2/sessions/:sid/histogram

Histogram of finite pixels (`server/v2/histogram.rs`).

```json
{ "bins": 256, "range": [0.0, 5.0], "log_counts": false }
```

| Field | Default | Notes |
|---|---|---|
| `ref` (alias `image_ref`) | active ref | |
| `region` | whole image | Section 12 |
| `bins` | 256 | 1 to 65536 |
| `range` | 0.1 and 99.9 percentiles of the data | `[lo, hi]`, finite, `lo < hi`; values outside the range are not counted |
| `log_counts` | `false` | Return `ln(1 + count)` instead of counts |
| `render_png` | none | `true` is `400 not_implemented` |

```json
{ "ref": "f444w", "region": { "x": 0, "y": 0, "width": 7065, "height": 4177, "clipped": false }, "bins": [12, 40, 311], "bin_edges": [0.0, 0.0195, 0.039, 0.0586], "min": 0.0, "max": 5.0, "log_counts": false, "range_source": "explicit", "mode": 0.0488 }
```

`bin_edges` has `bins + 1` entries; `mode` is the centre of the fullest bin (`null` when every bin is empty); `range_source` is `auto` or `explicit`.

### 11.7 Render

#### POST /v2/sessions/:sid/render

Renders a ref, or a region of it, to an RGB PNG with the desktop display controls (`server/v2/render/handler.rs`).

```json
{
  "region": { "type": "sky", "ra": 274.7106, "dec": -13.8296, "size_arcmin": 2.0 },
  "scale": { "algorithm": "percentile", "percentile": [1, 99.5], "stretch": "asinh", "asinh_a": 0.08 },
  "colormap": "magma",
  "invert_cmap": false,
  "max_dim": 1024,
  "overlays": [ { "type": "crosshair", "ra": 274.7106, "dec": -13.8296 }, { "type": "scalebar", "length_arcsec": 10 } ]
}
```

| Field | Default | Notes |
|---|---|---|
| `ref` (aliases `image_ref`, `image`) | active ref | |
| `region` | whole image | Section 12; a shape renders its bounding box. A region partly outside is clamped to the image; a region wholly outside shrinks to one pixel row or column at the nearest edge (one pixel at a corner); `clip` is not used; a sky region still needs a WCS |
| `scale.algorithm` | `zscale` | `minmax`, `zscale`, `percentile`, `user` (alias `manual`) |
| `scale.vmin`, `scale.vmax` | data minimum and maximum | Used by `user`; finite, `vmin < vmax` |
| `scale.percentile` (alias `percentiles`) | `[1, 99.5]` | Used by `percentile`; 0 to 100, low below high |
| `scale.zscale_contrast` | 0.25 | Used by `zscale` |
| `scale.stretch` | `linear` | `linear`, `log`, `sqrt`, `asinh`, `power` |
| `scale.asinh_a`, `scale.power` | 0.1, 2.0 | |
| `scale.symmetric`, `scale.centre` (alias `center`) | `false`, 0 | Mirror the limits about the centre (residual and velocity maps); the absolute value of the centre at most 1e38 |
| `colormap` | `gray` | `gray` (alias `grey`), `viridis`, `inferno`, `magma`, `plasma`, `cividis`, `heat`, `cool`, `rainbow`, `rdbu`, `coolwarm`, `bwr`, `seismic` |
| `invert_cmap` | `false` | No-data pixels keep the colormap's no-data colour |
| `max_dim` | none | A long side above it is area-binned by `ceil(long / max_dim)`, ignoring padding |
| `overlays` | `[]` | `crosshair` (`x`/`y` in image pixels, or `ra`/`dec` with a WCS), drawn red across the PNG; `scalebar` (`length_arcsec`, needs a WCS), drawn white at the bottom left; other types, and overlays that cannot be placed, are silently ignored |

The limits are computed on the displayed (binned) pixels, leaving out NaN, infinity and exact-zero padding. An unknown algorithm, stretch or colormap is `400` with the accepted names in `hint`.

The PNG comes with an `X-Render-Resolved` header holding the parameters actually used:

```json
{
  "ref": "f444w",
  "region": { "x": 2563, "y": 1612, "w": 1908, "h": 1908 },
  "region_clipped": false,
  "vmin": 0.21, "vmax": 48.7,
  "scale_algorithm": "percentile",
  "symmetric": false, "centre": null, "notes": [],
  "stretch": "asinh", "colormap": "magma",
  "binning_applied": 2,
  "png_scale_arcsec_per_px": 0.1258,
  "clipped_fraction": { "below_vmin": 0.01, "above_vmax": 0.005 }
}
```

`png_scale_arcsec_per_px` is `null` without a WCS; `notes` explains a degenerate symmetric range or a symmetric range under a non-linear stretch.

### 11.8 Files

#### GET /v2/fs

Lists one folder under the path roots, not recursively (`server/science/fs.rs`). Query parameters: `dir` (default: `ASTROBURST_SERVER_ROOT`) and `glob`. Example: `GET /v2/fs?dir=/data/m16&glob=*.fits`.

```json
{
  "dir": "/data/m16",
  "count": 3,
  "truncated": false,
  "entries": [
    { "name": "656nmos.fits", "kind": "fits", "bytes": 10535040, "modified_ms": 1792769423186 },
    { "name": "night2", "kind": "dir", "bytes": null, "modified_ms": 1792769400000 },
    { "name": "roman_wfi.asdf", "kind": "asdf", "bytes": 67239424, "modified_ms": 1792769411000 }
  ]
}
```

- `dir` is the folder actually listed, with links and `..` resolved (on Windows in the `\\?\C:\...` form); join it with an entry's `name` to get a path for `open`.
- `kind` comes from the name: `dir`; `fits` for `.fits`, `.fit`, `.fts`, `.fz` and `.fits.gz`, `.fit.gz`, `.fts.gz`; `asdf` for `.asdf`; anything else is `other`, `.zip` included. Case is ignored.
- `bytes` is the file size (`null` for a folder); `modified_ms` is the last change in milliseconds since the Unix epoch, `null` when the file system has none. Links are not followed: a link is reported as itself (never `dir`, with the link's own size and time) and its `kind` comes from its name; list the folder it points to by that folder's own path.
- `glob` filters files by name, with `*` (any run of characters) and `?` (one character), in any case; folders are always listed. Names that start with `.`, and names that are not valid UTF-8, are left out.
- Entries are sorted by name (byte order). A listing stops at 5000 entries, with `truncated: true`.
- A `dir` that resolves outside every path root is `403 path_outside_root` on any bind (section 4); a missing folder is `404 not_found`; a file is `400 bad_request`.

### 11.9 Synthetic data

#### POST /v2/sessions/:sid/synth

Renders a seeded synthetic star field, or a stack of dithered frames of it, as refs with the truth catalogue in the job's outcome (`server/science/synth.rs`), through the same `core::synth::pipeline` the desktop's Synthetic Generator uses: the same `config` gives the same pixels and catalogue, bit for bit. It runs as a job (section 6) and answers `202 {"job_id": "...", "status": "running"}`.

```json
{
  "config": {
    "field": { "width": 512, "height": 512, "n_stars": 200, "flux_min": 2000.0, "flux_max": 500000.0, "seed": 42 },
    "field_type": "Uniform",
    "psf_type": { "Gaussian": { "fwhm": 3.0 } },
    "noise": { "gain": 1.5, "readout_noise": 8.0, "sky_background": 200.0, "dark_current": 0.05, "exposure_time": 300.0, "bias_level": 1000.0, "seed": 1001 },
    "apply_vignette": false,
    "vignette_strength": 0.3,
    "n_frames": 1
  },
  "stack": false,
  "save_ground_truth": true
}
```

| Field | Default | Notes |
|---|---|---|
| `config` | required | The Synthetic Generator's settings, in the form the desktop sends them. `field_type` is `"Uniform"`, `{"KingCluster": {"core_radius", "tidal_radius"}}` or `{"ExponentialDisk": {"scale_length", "inclination_deg"}}`; `psf_type` is `{"Gaussian": {"fwhm"}}`, `{"Moffat": {"fwhm", "beta"}}` or `{"Airy": {"lambda_over_d"}}`; `field.seed` places the stars and `noise.seed` (optional) changes the noise. Optional: `cadence_seconds` (default `exposure_time` + 10), `frame_variation` (`enabled`, `dither_px`, `fwhm_jitter`, `sky_jitter`, `transparency_jitter`, `cosmic_rays_per_megapixel`; used by a stack of more than one frame) |
| `stack` | `false` | `false`: one frame, `n_frames` is ignored. `true`: `n_frames` frames, the first undithered |
| `name` | `synth_N` | The ref of the frame; with `stack`, the prefix of the frame refs |
| `save_ground_truth` | `false` | Also publish the noise-free star image, in the units of the frames, as `<name>_truth`. Nothing is written to disk |

Products: one frame is `<name>`; a stack is `<name>_0000`, `<name>_0001`, ... (four digits, as the desktop names its files); then `<name>_truth` when asked. Each carries the desktop's header cards: `EXPTIME`, `GAIN`, `EGAIN`, `RDNOISE`, `BUNIT = 'ADU'`, `DATE-OBS`, `MJD-OBS`, `SYNSEED`, `SYNFIELD`, `SYNPSF` and `SYNFWHM`, plus `SYNFRAME`, `SYNDX`, `SYNDY` and `SYNOFFS` on stack frames, and `SYNTHGT` on the ground truth. The outcome's `result`:

```json
{ "width": 512, "height": 512, "star_count": 200, "frames": 1, "catalog_csv": "id,x,y,z,flux_e,flux_adu,temperature\n0,103.6182,12.0496,0.0000,4210.7795,2807.1863,8137.4\n", "manifest_csv": null }
```

- `catalog_csv` is the truth catalogue, one row per star after the header row: `x` and `y` in 0-based pixels of the undithered frame, the flux in electrons and in the ADU of the frames (`flux_e / gain`), the temperature in kelvin.
- `manifest_csv` is the per-frame table of a stack (`frame,dx,dy,fwhm_factor,sky_factor,transparency,n_cosmic_rays,date_obs`; a star of frame `i` sits at its catalogue position plus `dx`, `dy`); `null` for one frame.
- Refused with `400 bad_request` before a job slot is taken: a `config` the desktop refuses (the message names the field, for example `Field size 0x512 is outside 1..16384 pixels`), a stack over the desktop's pixel budget (2^30 pixels in all), and products that do not fit one session cache, more images than `ASTROBURST_CACHE_MAX_ENTRIES` or more bytes (4 per pixel) than `ASTROBURST_CACHE_MAX_BYTES`, with that variable in `hint`. The engine's frame-count check (1 to 1024 frames) runs inside the job, so a stack of 0 frames ends the job in `error`.
- A stack stops between frames when the job is cancelled. Generating a frame replaces any ref of the same name, and adding the products can evict older refs from the session cache.

### 11.10 Cube preparation

Runs a cube preparation recipe as a job: the LIGHTSTACK workflow of crop, unit conversion, alignment, cube assembly, cube edits, PSF preparation, matching kernels and PSF matching (`server/science/cubeprep.rs`, `service/cubeprep_recipe.rs`). Each step calls the engine function of the desktop's cube preparation command for that step. A recipe is the JSON file the LIGHTSTACK oracle reads (`scripts/oracle/lightstack_oracle.py`; example: `src-tauri/tests/fixtures/lightstack/d1/recipe_d1.json`), and with its defaults it follows LIGHTSTACK 0.4.0 (crop rounding, area rule, cube layout and matching mode) except for the differences listed at the end of this section. A test runs the D1 recipe from LIGHTSTACK's own crops and compares the cube with the one LIGHTSTACK wrote: the NaN masks are identical, the planes without SIP distortion agree within 2e-6 of their peak (measured 5e-8) and the SIP plane within 1e-3 of its 0.1 to 99.9 percentile range (measured 8e-5).

#### GET /v2/cubeprep/filters

The filter registry that names the planes of a cube: the built-in filters and the custom datasets of the AstroBurst configuration file (`filter_datasets`). A recipe's `dataset` and `filter` values and the image names of an empirical PSF are looked up in it, and `pivot_angstrom` orders the planes. No session needed.

```json
{
  "datasets": ["JWST", "HST", "LSST", "JPAS", "JPLUS", "SWIFT", "FDS", "ROMAN"],
  "filters": [
    { "dataset": "JWST", "filter": "F444W", "instrument": "NIRCam", "group": "LW", "pivot_angstrom": 44043.15, "aliases": [] },
    { "dataset": "HST", "filter": "F814W", "instrument": "ACS", "group": "ACS", "pivot_angstrom": 8045.53, "aliases": [] }
  ]
}
```

#### POST /v2/sessions/:sid/cubeprep/jobs

The body is the recipe:

```json
{
  "recipe_version": 1,
  "name": "d1",
  "inputs": [
    { "path": "d1/d1_HST_F814W.fits", "dataset": "HST", "filter": "F814W" },
    { "path": "d1/d1_JWST_F444W.fits", "dataset": "JWST", "filter": "F444W" }
  ],
  "steps": [
    { "step": "crop", "mode": "radec", "ra": 53.1625, "dec": -27.7914, "size_arcsec": 1.8 },
    { "step": "units", "to": "Jy/pixel" },
    { "step": "align", "reference": "F444W", "method": "bilinear", "border_crop": 1 },
    { "step": "cube", "reference": "F444W", "order": "wavelength", "output": "d1_cube.fits" },
    { "step": "edit", "action": "drop_invalid", "threshold": 0.0, "frac_valid": 0.01 },
    { "step": "psfs", "source": "files", "reference": "F444W", "pixel_scale_arcsec": 0.04, "files": [
      { "filter": "F814W", "path": "d1/psf/d1_psf_HST_F814W_40mas.fits" },
      { "filter": "F444W", "path": "d1/psf/d1_psf_JWST_F444W_40mas.fits" }
    ] },
    { "step": "kernels", "reference": "F444W", "method": "wiener", "eps": 0.05, "shape": "reference" },
    { "step": "match", "reference": "F444W" }
  ]
}
```

- `recipe_version` (required) must be `1`; `name` is optional; `inputs` (at least one) gives each image's `path`, `dataset` and `filter`, all required, one input per filter (compared without regard to case); `steps` (at least one) run in order, each named by `step`.
- Paths are files on the server: absolute, or relative to `ASTROBURST_SERVER_ROOT`. The oracle reads them relative to its `--data` folder, so a recipe written for the oracle runs unchanged when the server root is that folder.
- A key the recipe format does not know is refused wherever it appears (top level, input or step), so a misspelt parameter never falls back to its default.

Steps, their keys and defaults; a filter value names the `filter` of an input:

| `step` | Keys |
|---|---|
| `units` | `to`: `"Jy/pixel"`, or `"MJy/sr"` (AstroBurst only; LIGHTSTACK converts to Jy/pixel) |
| `crop` | `mode` and its keys: `radec` (`ra` and `dec` in degrees, `size_arcsec`), `reference` (`reference`, the filter whose footprint the others are cut to), `common` (the footprint every input covers), `region` (`region_file`, a DS9 file of boxes, and `index`, the box to use, default `0`), `region_common` (`region_file`; the box every box of the file covers) or `pixel` (`x_start`, `x_end`, `y_start`, `y_end`; 0-based, the ends excluded). `rounding`: `lightstack` (default) or `astroburst` |
| `align` | `reference` (required, a filter: the grid every image is resampled onto); `method`: `bilinear` (default), `bicubic` or `exact`; `border_crop`: pixels cut from each side, default `1`; `area_rule`: `lightstack` (default), `scale` or `keep` |
| `cube` | `reference`: a filter, default the `align` reference; `order`: `wavelength` (default, by pivot) or `given`; `output`: a `.fits` file name, default `cube.fits`; `layout`: `lightstack` (default, the planes in the primary HDU) or `mef` (SCI, with ERR and DQ cubes when the inputs carry them; `align` writes SCI only) |
| `edit` | `action` and its keys: `drop_invalid` (`threshold`, default `0`; `frac_valid`, default `0.01`; `use_dq`, default `false`), `remove_filter` (`filter`), `cut` (`x_start`, `x_end`, `y_start`, `y_end`), `cut_sky` (`ra`, `dec`, `size_arcsec`, `rounding`), `header` (`redshift`, `ra_center`, `dec_center`, `use_brightest_pixel`) or `reference_nan_mask` (`reference_slice`, `mode`) |
| `psfs` | `source` and its keys: `files` (`files`, a list of `filter` and `path`) or `empirical` (`dataset`; `filters`; `data_dir`, default the server root; the keys of the desktop's empirical PSF options: `cutout_size`, `background_border`, `background_sigma`, `clip_negative`, `noise_threshold`, `radial_cleaning`, `cleaning_sigma`, `cleaning_consecutive_bins`, `normalization_radius`, `normalization_radius_unit`, `psf_size`, `oversampling`, `stack`; `oversampling_order` may only be `3`). An empirical PSF is built from the one FITS file in `data_dir` whose name the registry reads as that filter, with the stars of `stars_<filter>.reg` (lower case) beside it. `pixel_scale_arcsec` resamples every PSF to that scale and then needs `reference`, the filter whose PSF sets the size; `recentre` (0.8.0, default `false`): recentre each PSF on its centroid, otherwise the report states the shift a kernel will apply |
| `kernels` | `reference` (required, a filter with a PSF); `method` (required): `wiener` (`eps`, default `0.001`; `eps_mode`: `absolute` (default) or `relative`; `shape`: `reference` (default), `max` (pads the smaller PSF to the larger size on each axis, 0.8.0) or `[rows, cols]`, both odd) or `window` (the desktop's taper window keys and `regularization`); `auto`: let the desktop's automatic search choose `eps` or the window, default `false` |
| `match` | `reference` (required, the filter the kernels lead to); `mode`: `lightstack` (default) or `astroburst`; `allow_unmatched`: default `false` |
| `export` | `output`: a `.fits` file name; copies the current cube under that name |

Inputs must name a dataset and its canonical filter from the registry; matching is case-insensitive, and aliases are refused with the canonical name in the error. Separate `psfs` steps accumulate filters; a later PSF for the same filter replaces the earlier one. Empirical PSF options and kernel window parameters are checked before accepting the job.

Rules, checked in the oracle's order: a `cube` needs a `reference` or an earlier `align`; `edit`, `match` and `export` need an earlier `cube`; `kernels` needs a PSF for its `reference`, and `match` needs kernels to its `reference`; PSFs name input filters, one PSF per filter in each step; sizes and scales must be positive; every file and folder the recipe names must exist, and an empirical PSF needs, in `data_dir`, one FITS file named for each of its filters and that filter's `stars_<filter>.reg`.

Refusals come before a job slot is taken and before anything is written:

| Status | `code` | When |
|---|---|---|
| 400 | `recipe_version_unsupported` | `recipe_version` is not `1` |
| 400 | `unknown_step` | A `step` name that is not one of the nine above; `hint` lists them |
| 400 | `unknown_parameter` | A key the recipe format does not know; `hint` lists the keys accepted in that place |
| 400 | `filter_duplicate` | Two inputs with the same filter |
| 400 | `recipe_invalid` | A missing key, a value of the wrong type, a broken rule, or two FITS files in `data_dir` named for one filter of an empirical PSF; the message names the place, for example `steps[0] (crop): size_arcsec must be positive` |
| 403 | `path_outside_root` | Non-loopback bind: a path of the recipe, or the image or star file an empirical PSF finds in `data_dir`, resolves outside the path roots (section 4) |
| 404 | `file_not_found` | A file or folder of the recipe does not exist, or `data_dir` lacks an empirical PSF's image or star file |

An accepted recipe answers `202`:

```json
{ "job_id": "066490f5-e5d8-417e-8d46-8761798d0ca1", "status": "running", "output_dir": "/data/out/cubeprep-066490f5-e5d8-417e-8d46-8761798d0ca1" }
```

The job writes into `output_dir`, a new folder `cubeprep-<job_id>` under `ASTROBURST_OUTPUT_ROOT`:

- `recipe.json`: the recipe as run, with every default filled in and every path resolved; sending it again to this API repeats the run. This saved form includes engine extension keys; remove those keys before passing it to the Python oracle.
- `01_crop`, `02_units`, ...: one folder per step, numbered in recipe order, with that step's files. Their names follow the desktop's (`d1_HST_F814W_cutout_9_8_46x46_Jy_aligned_bilinear.fits`); a server process that already wrote a file of the same name in an earlier job adds a number to the new one (`d1_cube_3_valid_psfmatched.fits`), so read the names from the outcome.
- `result.json`: the outcome's `result`, written once every step has finished.

The job's progress `stage` is the folder of the step being run (`03_align`), and `pct` advances one step at a time; the final stage is `done`. A cancel stops the job before its next step, or between the files of a `units` step, a `pixel` crop or an empirical `psfs` step; a file being written is finished first. After a cancel or an error the folder keeps what was written, so the steps that ran can be inspected; the server never deletes it. No route runs one step on an earlier job's folder: image outputs from crop, units or align can be inputs of a new recipe. An existing cube cannot be used to resume directly at edit or match; those steps require a cube built in the new recipe.

The outcome's `result` (section 6):

```json
{
  "recipe_version": 1,
  "name": "d1",
  "output_dir": "/data/out/cubeprep-066490f5-e5d8-417e-8d46-8761798d0ca1",
  "cube": "/data/out/cubeprep-066490f5-e5d8-417e-8d46-8761798d0ca1/08_match/d1_cube_valid_psfmatched.fits",
  "steps": [
    { "index": 1, "step": "crop", "directory": "01_crop", "parameters": { "step": "crop", "mode": "radec", "ra": 53.1625, "dec": -27.7914, "size_arcsec": 1.8, "rounding": "lightstack" }, "report": { "...": "..." }, "warnings": [], "elapsed_ms": 69 }
  ],
  "files": [ { "path": "01_crop/d1_HST_F814W_cutout_9_8_46x46.fits", "bytes": 46080 }, { "path": "recipe.json", "bytes": 3182 } ],
  "warnings": [ "07_kernels: F356W: the matched FWHM 4.210 px differs from the target FWHM 3.471 px by +21.28 percent" ],
  "elapsed_ms": 291
}
```

- `cube` is the cube after the last step that wrote one (`cube`, `edit`, `match` or `export`); `null` without a `cube` step.
- `steps[].parameters` are the step's keys as run, defaults included; `report` is the report of the desktop command the step called (crop rectangles, conversion factors, alignment geometry, PSF preparation, kernel diagnostics, matched planes).
- `files` lists every file under `output_dir` except `result.json`, by path relative to it with `/`, in name order.
- Each `files` entry also carries `sha256`, computed from the final file bytes. Existing `path` and `bytes` fields remain unchanged. The execution lock is not a product and is excluded.
- `provenance` has its own `schema_version: 1`, software version, operating system, architecture, start time, `recipe_sha256`, resolved dependencies and the effective filter registry. Dependencies include science inputs, supplied PSFs, crop regions, and the FITS and DS9 files selected for empirical PSFs. Each dependency carries its roles, size and SHA-256. The recipe hash identifies the exact saved `recipe.json`, including resolved paths; it is not independent of file location.
- Input SHA-256 values and the filter registry are checked before and after execution. Input caches are invalidated before processing. A persistent change fails with `input_changed`; no successful `result.json` is published. These checkpoints are not an atomic input snapshot: keep inputs unchanged throughout the run. Hashes establish file identity, not scientific accuracy or eligibility for automatic cache reuse.
- `result.json` is published atomically after product hashing and a final cancellation check. Cancelled or failed runs retain their partial files without a success manifest. Since 0.8.0 they write `failure.json` instead, with the stage, step index, code, message and hint, the validation advisories and the warnings of the steps that finished; the execution lock records the process id, start time and software version. A cancel stops a running step at its next input or plane boundary.
- MEF cubes carry a `NOISCORR` extension (0.8.0): the normalized noise autocorrelation of each plane after alignment or matching, with the `NCORRK`, `NCORRSRC` and `NCORRALN` cards, and the `FILTERS` table gains `NOISCORR_ALIGN`, `NOISCORR_MATCH` and `MATCH`. Kernel files carry `SRCFWHM`, `TGTFWHM`, `MATFWHM` and `KSUMSQ`. The LIGHTSTACK layout carries none of them. Hashing checks cancellation between read blocks; progress stages include `hashing_inputs`, `verifying_inputs` and `hashing_products`. Every run must use a new or empty output folder, with exclusive reservation during execution.
- `warnings` collects the steps' warnings, each prefixed with its step folder, and the outcome's `warnings` repeat them. The kernel and matching warnings judge the PSF match (residuals, FWHM, encircled energy, negative kernel lobes, values near NaN holes); read them before using a cube.
- `products` stays empty: the files are on disk, not refs. `output_dir` lies under a path root, so `POST /v2/sessions/:sid/open` opens a file of it and `GET /v2/fs?dir=<output_dir>` lists it.

The job checks the recipe again before its first step, so a file removed after the request ends the job in `error` with the refusal's code in `error_code`. A step that fails ends the job in `error`, and the `error` text starts with its folder (`08_match: kernel_missing: no matching kernel to F444W for F150W; ...`). A failure of the matching engine also sets the outcome's `error_code`: `kernel_missing` (a plane of the cube has no kernel to the reference; `error_hint` says how to change the recipe), `kernel_scale` (a kernel sampled at another pixel scale than the cube) or `convolution_failed`.

Differences from LIGHTSTACK 0.4.0 under the defaults:

- `match`: LIGHTSTACK copies a plane without a kernel unmatched and goes on; here the job stops with `kernel_missing` unless `allow_unmatched` is `true`.
- An empirical PSF reads the DS9 star positions as 1-based, as DS9 writes them; LIGHTSTACK reads them one pixel off.
- A `pixel` crop that runs past the edge keeps its size: the outside is NaN and flagged in DQ, with a warning.
- `area_rule` `lightstack` multiplies by the pixel-area ratio as LIGHTSTACK does, which conserves the flux of an image in Jy/pixel but not the surface brightness of an image in MJy/sr; the step warns in that case. `scale` treats both correctly.
- A crop edge within 1e-8 px of a pixel boundary can round to either side, and AstroBurst and LIGHTSTACK compute it with different WCS libraries. The D1 test data puts two crop edges on such ties: the D1 cube is 44 by 44 pixels here and 44 by 43 in LIGHTSTACK, while from the same crops the results agree as stated above.

The engine reads the files through the library's own image cache (up to 32 images and 2 GiB for the whole process), apart from the session caches (section 5); that memory stays in use after the job until later reads replace it.

## 12. Region specifications

Used by `cutout`, `stats`, `histogram` and `render` (`server/v2/region.rs`). The form is chosen by `type`.

**Pixel box.**

```json
{ "type": "pixel", "x": 100, "y": 200, "width": 64, "height": 48, "clip": false }
```

`x`, `y` is the bottom-left corner in 0-based pixels (may be negative); `width` and `height` are positive integers.

**Sky box.**

```json
{ "type": "sky", "ra": 274.7106, "dec": -13.8296, "size_arcmin": [2.0, 1.5], "clip": false }
```

Centre in ICRS degrees; `size_arcmin` is one number (square) or `[width, height]`. Needs a WCS.

**Shape.**

```json
{ "type": "shape", "shape": "ellipse", "x": 512.0, "y": 400.0, "rx": 30, "ry": 12, "angle": 35, "system": "image", "clip": false }
```

| `shape` | Fields |
|---|---|
| `circle` | `x`, `y`, `r` |
| `ellipse` | `x`, `y`, `rx`, `ry`, `angle` (0) |
| `box` | `x`, `y` (centre), `width`, `height`, `angle` (0) |
| `annulus` | `x`, `y`, `r_inner`, `r_outer` |
| `polygon` | `points: [[x, y], ...]`, at least 3 |
| `line` | `x1`, `y1`, `x2`, `y2` |
| `point` | `x`, `y` |

Radii, widths and heights must be positive, `r_inner` at least 0 and below `r_outer`; angles are in degrees. `system` is `image` (default), `fk5` or `icrs`; in a sky system positions are degrees, lengths arcseconds and angles are sky angles, converted through the WCS. `physical` is parsed but refused (`400`). Shapes follow the DS9 rule: a circle of radius 5 at an integer centre covers 81 pixels.

`clip` (default `false`) is read by `stats` and `histogram`: a region that runs past the edge is clamped instead of refused with `400 region_out_of_bounds`; a region that misses the image entirely is always refused.

## 13. Planned routes

Not implemented yet; paths and bodies may change. Most planned routes call the engine function the desktop command already uses, so a route and the desktop return the same numbers, and their bodies use the desktop command's argument names. The foundation routes (G0: job outcome, v2 job routes, planes, ref delete and FITS download, file listing, capabilities), the G0 guard (token, `Host` allowlist and path roots for every route that takes a file path, section 4) the synthetic data route (G9) and the cube preparation recipes are in place (sections 6, 9 and 11). Groups in planned order:

- **G1 Image measurement and photometry:** photometry (single, batch and gain model), star detection, empirical PSF, region statistics with a background annulus, radial, line and surface-brightness profiles, contours, pixel table, DQ flag table, batch noise evaluation, FFT power spectrum; `exclude_dq` on `/stats` and `/histogram`.
- **G2 Astrometry, WCS and catalogues:** Gaia DR3 cone search, cross-match and CSV; observation geometry; DS9 `.reg` import and export; pointing overlap; plate solving of one frame or a list with the desktop's offline catalogue tiers; the image centre and field of view on `/wcs`, `frame` on `/wcs/sky2pix`, `position_angle_deg` on `/wcs/separation`.
- **G3 Spectroscopy and cubes:** cube description, channel, spaxel and region spectra, collapse, moments, line-fit maps, position-velocity diagrams, line measurement, spectral axis, barycentric correction, JWST x1d and 1D FITS spectra.
- **G4 Time domain and detector-level data:** time-series photometry, JWST ramp inspection, slope fits and comparison with the official `_rate`.
- **G5 MAST archive** (needs `mast`): status, missions, name resolution, search, products, previews and thumbnails, header peek, presets, downloads as jobs, cache and footprints; the MAST token from `ASTROBURST_MAST_TOKEN` only.
- **G6 Calibration and integration:** v2 stack and drizzle with the desktop header cards, per-frame alignment report and warnings; pipeline, calibration, cosmetic correction, debayer, CFA extraction and Drizzle RGB.
- **G7 Post-processing:** the processing operations (background, wavelet denoise, Richardson-Lucy, arcsinh, GHS, masked stretch, LHE, HDRMT, curves and levels, PixelMath), star removal, rotate and flip, resample, reproject, crop, compose steps and SPCC.
- **G8 I/O and export:** save, the science cutout file with ERR and DQ planes, RGB render and RGB FITS export, compression, overlay rendering, filter detection, 16-bit output on `/render` and optional sample sets.

## 14. Python client

The async client in `agent/` (package `astroburst-client`, contributed by Jae-Joon Lee) wraps the v1 routes: sessions, `fits/open` and `fits/header`, the three renders, stack, drizzle, pipeline and jobs with polling, cancel and SSE streaming. Of the v2 routes it wraps cube preparation (`Session.cubeprep(recipe)`, which returns a `Job` with `output_dir`, and `AstroBurstClient.cubeprep_filters()`) and the job outcome (`Job.outcome()`); an `AstroBurstError` carries the server's `hint`. The other v2 routes have no Python method yet; call them with any HTTP client. Installation, examples and the client reference are in [`agent/README.md`](../agent/README.md).

Install with `pip install -e ./agent` (add `[image]` for the Pillow helper). A first session:

```python
import asyncio
from astroburst_client import AstroBurstClient

async def main():
    async with AstroBurstClient("http://localhost:8080") as client:
        session = await client.create_session()
        result = await session.open("/data/m16_ha.fits", slot="ha")
        print(f"{result.width}x{result.height}  median={result.stats.median:.1f}")
        png, stf = await session.stf("ha")
        open("preview.png", "wb").write(png)

asyncio.run(main())
```

Runnable examples: [`agent/examples/`](../agent/examples); `04_cubeprep.py` runs a recipe file and prints its steps, warnings and cube, or the error code and hint.

## 15. Credits and licence

- The headless server (`src-tauri/src/server/`), the v1 and v2 APIs, the Python client (`agent/`), `src-tauri/SERVER.md` and `src-tauri/SERVER_TESTING.md` were contributed by [Jae-Joon Lee](https://github.com/leejjoon). He also contributed the WCS engine migration, FITS tile compression, zscale and the first colormaps (gray and viridis).
- `src-tauri/SERVER.md` remains the contributor's original v1 guide; this document is the complete reference for the v1 and v2 routes and lists the planned ones.
- AstroBurst is free software under the AGPLv3; see [LICENSE](../LICENSE). Third-party notices: [src-tauri/licenses/THIRD-PARTY-NOTICES.md](../src-tauri/licenses/THIRD-PARTY-NOTICES.md).

