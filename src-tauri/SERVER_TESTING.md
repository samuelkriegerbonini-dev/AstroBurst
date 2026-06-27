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
| `POST /sessions` when `ASTROBURST_SESSION_MAX` reached | `503 service_unavailable` with message showing the configured cap |
| `X-Request-Id: my-id` on any request | Response echoes `x-request-id: my-id` |
