#!/usr/bin/env python3
# AstroBurst Python client — contributed by Jae-Joon Lee <https://github.com/leejjoon>
"""
Example 03 — Full narrowband calibration + stacking pipeline.

Usage:
    python 03_pipeline.py \\
        --ha   /data/ha_001.fits /data/ha_002.fits \\
        --oiii /data/oiii_001.fits /data/oiii_002.fits \\
        [--bias /data/bias.fits] \\
        [--dark /data/dark.fits] \\
        [--flat /data/flat_ha.fits] \\
        [--prefix "pipe/"]

Progress is tracked via polling.  Final result slots are printed on completion.
"""
from __future__ import annotations

import argparse
import asyncio
import sys
from pathlib import Path

from astroburst_client import AstroBurstClient
from astroburst_client.errors import AstroBurstError


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--ha",   nargs="+", metavar="PATH", help="Hα light frames")
    p.add_argument("--oiii", nargs="+", metavar="PATH", help="[OIII] light frames")
    p.add_argument("--sii",  nargs="+", metavar="PATH", help="[SII] light frames")
    p.add_argument("--bias", nargs="+", metavar="PATH", help="Bias calibration frames")
    p.add_argument("--dark", nargs="+", metavar="PATH", help="Dark calibration frames")
    p.add_argument("--flat", nargs="+", metavar="PATH", help="Flat calibration frames")
    p.add_argument("--prefix", default="pipe/", help="Result slot prefix (default: pipe/)")
    p.add_argument("--sigma-low",  type=float, default=None)
    p.add_argument("--sigma-high", type=float, default=None)
    p.add_argument("--normalize", action="store_true", default=None)
    return p.parse_args()


async def main(args: argparse.Namespace) -> None:
    channels = []
    if args.ha:
        channels.append({"label": "Ha", "paths": args.ha})
    if args.oiii:
        channels.append({"label": "OIII", "paths": args.oiii})
    if args.sii:
        channels.append({"label": "SII", "paths": args.sii})

    if not channels:
        print(
            "Provide at least one channel (--ha, --oiii, --sii).",
            file=sys.stderr,
        )
        sys.exit(1)

    async with AstroBurstClient("http://localhost:8080") as client:
        health = await client.health()
        print(f"Server v{health.version}")

        session = await client.create_session()
        print(f"Session: {session.sid}")

        print(
            f"Submitting pipeline: {len(channels)} channel(s), "
            f"prefix={args.prefix!r}"
        )

        job = await session.pipeline(
            channels,
            bias_paths=args.bias or None,
            dark_paths=args.dark or None,
            flat_paths=args.flat or None,
            sigma_low=args.sigma_low,
            sigma_high=args.sigma_high,
            normalize=args.normalize if args.normalize else None,
            result_prefix=args.prefix,
        )
        print(f"Job {job.job_id!r} submitted")
        print(f"Expected result slots: {job.slots}")

        print("\nPolling for completion...")
        last_pct = -1
        while True:
            status = await job.poll()
            if status.pct != last_pct:
                print(f"  {status.pct:3d}%  status={status.status}", flush=True)
                last_pct = status.pct
            if status.is_terminal:
                break
            await asyncio.sleep(3.0)

        print(f"\nFinal status: {status.status}")
        if status.is_error:
            print("Pipeline ended with error.", file=sys.stderr)
            sys.exit(1)

        for slot in job.slots:
            try:
                png, stf = await session.stf(slot)
                safe_name = slot.replace("/", "_")
                out = Path(f"{safe_name}.png")
                out.write_bytes(png)
                print(
                    f"  {slot!r} → {out}  "
                    f"({len(png):,} bytes, "
                    f"midtone={stf.midtone:.4f})"
                )
            except AstroBurstError as e:
                print(f"  Could not render {slot!r}: [{e.code}] {e.message}")


if __name__ == "__main__":
    args = parse_args()
    try:
        asyncio.run(main(args))
    except AstroBurstError as e:
        print(f"Error [{e.code}]: {e.message}", file=sys.stderr)
        sys.exit(1)
