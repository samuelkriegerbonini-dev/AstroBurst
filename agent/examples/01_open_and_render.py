#!/usr/bin/env python3
# AstroBurst Python client — contributed by Jae-Joon Lee <https://github.com/leejjoon>
"""
Example 01 — Open a FITS file and render it.

Usage:
    python 01_open_and_render.py /path/to/image.fits [slot_name]

The server must be running and reachable at http://localhost:8080.
Start it with:
    cd src-tauri
    cargo run --no-default-features --features server,astrometry-net,asdf-full,vizier
"""
from __future__ import annotations

import asyncio
import sys
from pathlib import Path

from astroburst_client import AstroBurstClient, to_pillow
from astroburst_client.errors import AstroBurstError


async def main(fits_path: str, slot: str = "img") -> None:
    async with AstroBurstClient("http://localhost:8080") as client:
        health = await client.health()
        print(f"Server v{health.version}, sessions active: {health.sessions_active}")

        session = await client.create_session()
        print(f"Session: {session.sid}")

        result = await session.open(fits_path, slot=slot)
        print(
            f"Opened  : {fits_path!r}\n"
            f"Slot    : {result.slot}\n"
            f"Size    : {result.width} x {result.height} px\n"
            f"Median  : {result.stats.median:.2f}\n"
            f"Sigma   : {result.stats.sigma:.2f}\n"
            f"STF     : shadow={result.stf.shadow:.4f}  "
            f"midtone={result.stf.midtone:.4f}  "
            f"highlight={result.stf.highlight:.4f}"
        )

        png, stf = await session.stf(slot)
        out = Path("preview_stf.png")
        out.write_bytes(png)
        print(f"Saved STF render → {out}  ({len(png):,} bytes)")

        png2 = await session.render(slot, stf.shadow, stf.midtone, stf.highlight)
        out2 = Path("preview_render.png")
        out2.write_bytes(png2)
        print(f"Saved manual render → {out2}  ({len(png2):,} bytes)")

        crop_w = min(256, result.width)
        crop_h = min(256, result.height)
        crop = await session.viewport(slot, 0, 0, crop_w, crop_h)
        out3 = Path("preview_crop.png")
        out3.write_bytes(crop)
        print(f"Saved {crop_w}x{crop_h} crop → {out3}")

        try:
            img = to_pillow(png)
            print(f"PIL image: {img.size}  mode={img.mode}")
        except RuntimeError as e:
            print(f"(Pillow not installed: {e})")

        hdr = await session.header(slot)
        print(f"\nFITS header: {hdr.total_cards} cards")
        for card in hdr.cards[:5]:
            print(f"  {card.key:8s} = {card.value!r}")
        if hdr.total_cards > 5:
            print(f"  ... ({hdr.total_cards - 5} more)")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    fits = sys.argv[1]
    slot_name = sys.argv[2] if len(sys.argv) > 2 else "img"
    try:
        asyncio.run(main(fits, slot_name))
    except AstroBurstError as e:
        print(f"Error [{e.code}]: {e.message}", file=sys.stderr)
        sys.exit(1)
