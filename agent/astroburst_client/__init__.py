# AstroBurst Python client — contributed by Jae-Joon Lee <https://github.com/leejjoon>
"""
astroburst-client — async Python client for astroburst-server.

Quickstart::

    import asyncio
    from astroburst_client import AstroBurstClient

    async def main():
        async with AstroBurstClient("http://localhost:8080") as client:
            session = await client.create_session()
            result  = await session.open("/data/m51.fits", slot="ha")
            print(f"{result.width}x{result.height}, median={result.stats.median:.1f}")

            png, stf = await session.stf("ha")
            with open("preview.png", "wb") as f:
                f.write(png)

    asyncio.run(main())

See ``src-tauri/SERVER.md`` in the repository for the full REST API reference.
"""

__version__ = "0.1.0"

from .client import AstroBurstClient, Job, Session
from .errors import (
    AstroBurstError,
    BadRequestError,
    ConflictError,
    InternalServerError,
    NotFoundError,
    ServiceUnavailableError,
    TooManyRequestsError,
)
from .models import (
    HeaderCard,
    HeaderResult,
    HealthResult,
    JobStatus,
    OpenResult,
    Stats,
    Stf,
)
from ._images import to_pillow

__all__ = [
    "AstroBurstClient",
    "Session",
    "Job",
    "AstroBurstError",
    "BadRequestError",
    "NotFoundError",
    "ConflictError",
    "TooManyRequestsError",
    "ServiceUnavailableError",
    "InternalServerError",
    "HealthResult",
    "OpenResult",
    "Stats",
    "Stf",
    "HeaderCard",
    "HeaderResult",
    "JobStatus",
    "to_pillow",
]
