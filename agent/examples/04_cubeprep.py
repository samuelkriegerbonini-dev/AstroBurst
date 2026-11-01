from __future__ import annotations

import asyncio
import json
import os
import sys
from pathlib import Path
from typing import Optional

import httpx

from astroburst_client import AstroBurstClient
from astroburst_client.errors import AstroBurstError

USAGE = """usage: python 04_cubeprep.py RECIPE.json [SERVER_URL]

Runs a cube preparation recipe (recipe_version 1, the format of
src-tauri/tests/fixtures/lightstack/d1/recipe_d1.json) as a server job and
prints its steps, warnings and cube. Recipe paths are files on the server:
absolute, or relative to the server's ASTROBURST_SERVER_ROOT. Set
ASTROBURST_TOKEN when the server asks for a token."""


def http_client(base_url: str) -> Optional[httpx.AsyncClient]:
    token = os.environ.get("ASTROBURST_TOKEN", "").strip()
    if not token:
        return None
    return httpx.AsyncClient(
        base_url=base_url,
        timeout=httpx.Timeout(30.0),
        headers={"Authorization": f"Bearer {token}"},
    )


async def main(recipe_path: Path, base_url: str) -> int:
    recipe = json.loads(recipe_path.read_text(encoding="utf-8"))
    http = http_client(base_url)
    try:
        async with AstroBurstClient(base_url, client=http) as client:
            registry = await client.cubeprep_filters()
            print(f"{len(registry['filters'])} filters known in {', '.join(registry['datasets'])}")
            session = await client.create_session()
            job = await session.cubeprep(recipe)
            print(f"job {job.job_id} writes to {job.output_dir}")
            async for event in job.stream():
                if event.get("type") == "progress":
                    print(f"  {event.get('pct', 0):3d}%  {event.get('stage', '')}")
            answer = await job.outcome()
    finally:
        if http is not None:
            await http.aclose()
    outcome = answer["outcome"]
    if outcome is None:
        print(f"the job ended {answer['job']['status']} without an outcome")
        return 1
    if outcome["error"] is not None:
        print(f"error [{outcome.get('error_code', 'uncoded')}]: {outcome['error']}")
        if "error_hint" in outcome:
            print(f"hint: {outcome['error_hint']}")
        print(f"the steps that ran are kept in {job.output_dir}")
        return 1
    result = outcome["result"]
    for step in result["steps"]:
        print(f"{step['directory']}: {len(step['warnings'])} warnings in {step['elapsed_ms']} ms")
    for warning in outcome["warnings"]:
        print(f"warning: {warning}")
    print(f"{len(result['files'])} files in {result['output_dir']}")
    print(f"cube: {result['cube']}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3):
        print(USAGE)
        sys.exit(2)
    server = sys.argv[2] if len(sys.argv) == 3 else "http://localhost:8080"
    try:
        sys.exit(asyncio.run(main(Path(sys.argv[1]), server)))
    except AstroBurstError as error:
        print(f"refused [{error.code}]: {error.message}", file=sys.stderr)
        if error.hint:
            print(f"hint: {error.hint}", file=sys.stderr)
        sys.exit(1)
