> Contributed by Jae-Joon Lee — https://github.com/leejjoon

# astroburst-client

Async Python client for [astroburst-server](../src-tauri/SERVER.md) — the
headless REST API for the AstroBurst astronomical imaging pipeline.

## Prerequisites

Start the server first:

```bash
cd src-tauri
cargo run --bin astroburst-server --no-default-features --features server,astrometry-net,asdf-full,vizier
# Listening on 127.0.0.1:8080
```

If the server is on a **remote GPU workstation**, open an SSH tunnel:

```bash
ssh -L 8080:localhost:8080 user@remote-host
```

All client calls go to `http://localhost:8080`; the tunnel forwards them
securely.

## Installation

```bash
pip install -e ./agent              # core — only requires httpx
pip install -e "./agent[image]"     # adds Pillow for to_pillow()
```

## Quickstart

```python
import asyncio
from astroburst_client import AstroBurstClient

async def main():
    async with AstroBurstClient("http://localhost:8080") as client:
        health = await client.health()
        print(f"Server v{health.version}")

        session = await client.create_session()

        # Load a FITS file (absolute path on the server machine)
        result = await session.open("/data/m51_ha.fits", slot="ha")
        print(f"{result.width}x{result.height}  median={result.stats.median:.1f}")

        # Auto-stretch render
        png, stf = await session.stf("ha")
        with open("preview.png", "wb") as f:
            f.write(png)

        # Stack multiple frames (async job)
        job = await session.stack(
            ["/data/ha_001.fits", "/data/ha_002.fits", "/data/ha_003.fits"],
            result_slot="stacked",
            align=True,
        )
        # Stream live SSE progress
        async for event in job.stream():
            if event["type"] == "progress":
                print(f"  {event['pct']}%")
            elif event["type"] == "complete":
                print("  done!")

        # Or just wait (polling)
        # status = await job.wait(interval=2.0)

asyncio.run(main())
```

## API overview

### `AstroBurstClient`

| Method | Description |
|---|---|
| `health()` | `GET /health` — server status, version, session counters |
| `create_session()` | `POST /sessions` — returns a `Session` |

### `Session`

| Method | Description |
|---|---|
| `open(path, slot=None)` | Load a FITS/ASDF file → `OpenResult` |
| `header(slot)` | Full FITS header → `HeaderResult` |
| `stf(slot)` | Auto-stretch PNG → `(bytes, Stf)` |
| `render(slot, shadow, midtone, highlight)` | Explicit STF render → `bytes` |
| `viewport(slot, x, y, w, h, ...)` | Pixel crop render → `bytes` |
| `stack(paths, ...)` | Sigma-clip stack → `Job` (202) |
| `drizzle(paths, ...)` | Drizzle combine → `Job` (202) |
| `pipeline(channels, ...)` | Calibrate + stack pipeline → `Job` (202) |
| `get_job(job_id)` | Poll any job by ID → `JobStatus` |
| `cancel_job(job_id)` | Cancel any job by ID → `JobStatus` |

### `Job`

| Method | Description |
|---|---|
| `poll()` | Single status poll → `JobStatus` |
| `cancel()` | Cancel the job → `JobStatus` |
| `wait(interval, timeout)` | Poll until done → `JobStatus` |
| `stream()` | SSE iterator → `AsyncIterator[dict]` |
| `slot` | Result cache key (stack / drizzle) |
| `slots` | Result cache keys (pipeline) |

### Render methods → images

All render methods return raw `bytes` (PNG).  Convert to Pillow when
`astroburst-client[image]` is installed:

```python
from astroburst_client import to_pillow

png, _ = await session.stf("ha")
img = to_pillow(png)    # PIL.Image.Image
img.show()
```

### Error handling

```python
from astroburst_client import NotFoundError, ServiceUnavailableError

try:
    result = await session.open("/nonexistent.fits")
except NotFoundError as e:
    print(f"Not found: {e.message}")  # e.code == "not_found"
except ServiceUnavailableError:
    print("Session cap reached — try again later")
```

Full error hierarchy:

| Class | HTTP | Code |
|---|---|---|
| `BadRequestError` | 400 | `bad_request` |
| `NotFoundError` | 404 | `not_found` |
| `ConflictError` | 409 | `conflict` (SSE double-subscribe) |
| `TooManyRequestsError` | 429 | `too_many_requests` |
| `ServiceUnavailableError` | 503 | `service_unavailable` |
| `InternalServerError` | 500 | `internal_error` |

All inherit from `AstroBurstError` which carries `.code`, `.message`,
`.status`, and `.request_id` (for correlating with server logs).

## Examples

```bash
# Example 01 — open a single file and render
python agent/examples/01_open_and_render.py /path/to/image.fits

# Example 02 — stack several frames with SSE progress bar
python agent/examples/02_stack.py /data/f1.fits /data/f2.fits /data/f3.fits

# Example 03 — full narrowband pipeline
python agent/examples/03_pipeline.py \
    --ha   /data/ha_001.fits /data/ha_002.fits \
    --oiii /data/oiii_001.fits /data/oiii_002.fits \
    --bias /data/bias.fits \
    --dark /data/dark.fits \
    --flat /data/flat.fits \
    --prefix "pipe/"
```

## Full API reference

See [src-tauri/SERVER.md](../src-tauri/SERVER.md) for the complete REST API
documentation, configuration environment variables, session model, and curl
examples.