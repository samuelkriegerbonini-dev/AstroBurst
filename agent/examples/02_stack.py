# AstroBurst Python client — contributed by Jae-Joon Lee <https://github.com/leejjoon>
#!/usr/bin/env python3
"""
Example 02 — Sigma-clip stack with live SSE progress, then render.

Usage:
    python 02_stack.py /data/f1.fits /data/f2.fits /data/f3.fits ...

Pass at least two FITS paths (absolute paths on the server machine).
The server must be running at http://localhost:8080.

Two progress styles are demonstrated:
  1. SSE streaming (first-subscriber takes the stream)
  2. Wait/poll fallback (shown in the comment at the bottom)
"""
from __future__ import annotations

import asyncio
import sys
from pathlib import Path

from astroburst_client import AstroBurstClient
from astroburst_client.errors import AstroBurstError, ConflictError


async def main(paths: list[str]) -> None:
    if len(paths) < 2:
        print("Need at least 2 FITS paths.", file=sys.stderr)
        sys.exit(1)

    async with AstroBurstClient("http://localhost:8080") as client:
        session = await client.create_session()
        print(f"Session: {session.sid}")

        job = await session.stack(
            paths,
            result_slot="stacked",
            align=True,
            sigma_low=3.0,
            sigma_high=3.0,
        )
        print(f"Job {job.job_id!r} submitted, result slot → {job.slot!r}")

        print("\nProgress (SSE):")
        try:
            async for event in job.stream():
                t = event.get("type", "?")
                if t == "progress":
                    pct = event.get("pct", 0)
                    stage = event.get("stage", "")
                    bar = "#" * (pct // 5) + "." * (20 - pct // 5)
                    print(f"\r  [{bar}] {pct:3d}%  {stage:<20s}", end="", flush=True)
                elif t == "complete":
                    print(f"\r  [{'#'*20}] 100%  done              ")
                elif t == "error":
                    print(f"\n  ERROR: {event.get('message','?')}")
                    sys.exit(1)
        except ConflictError:
            print("  (SSE slot taken, falling back to polling)")
            status = await job.wait(interval=2.0)
            print(f"  Final status: {status.status}")

        status = await job.poll()
        print(f"\nFinal status: {status.status}  pct={status.pct}")

        if status.is_done and job.slot:
            png, stf = await session.stf(job.slot)
            out = Path("stacked.png")
            out.write_bytes(png)
            print(f"Saved render → {out}  ({len(png):,} bytes)")
            print(
                f"Auto-STF: shadow={stf.shadow:.4f}  "
                f"midtone={stf.midtone:.4f}  "
                f"highlight={stf.highlight:.4f}"
            )


if __name__ == "__main__":
    paths = sys.argv[1:]
    if not paths:
        print(__doc__)
        sys.exit(1)
    try:
        asyncio.run(main(paths))
    except AstroBurstError as e:
        print(f"Error [{e.code}]: {e.message}", file=sys.stderr)
        sys.exit(1)
