# AstroBurst Python client — contributed by Jae-Joon Lee <https://github.com/leejjoon>
"""
Async Python client for astroburst-server.

Quick start::

    import asyncio
    from astroburst_client import AstroBurstClient

    async def main():
        async with AstroBurstClient("http://localhost:8080") as client:
            session = await client.create_session()
            result  = await session.open("/data/m51.fits", slot="ha")
            print(result.dims, result.stats.median)
            png, stf = await session.stf("ha")
            with open("preview.png", "wb") as f:
                f.write(png)

    asyncio.run(main())
"""
from __future__ import annotations

import asyncio
import json as _json
from typing import (
    TYPE_CHECKING,
    Any,
    AsyncIterator,
    Dict,
    List,
    Optional,
    Tuple,
    Union,
)

import httpx

from .errors import AstroBurstError, error_from_response
from .models import HeaderResult, HealthResult, JobStatus, OpenResult, Stf

if TYPE_CHECKING:
    pass


class Job:
    """
    Handle for a long-running server job (stack / drizzle / pipeline).

    Obtained from ``Session.stack()``, ``Session.drizzle()``, or
    ``Session.pipeline()``.  Tracks the output slot(s) so callers know where
    to find results without parsing the 202 body themselves.
    """

    def __init__(
        self,
        client: "AstroBurstClient",
        sid: str,
        job_id: str,
        *,
        slot: Optional[str] = None,
        slots: Optional[List[str]] = None,
    ) -> None:
        self._client = client
        self._sid = sid
        self.job_id = job_id
        self.slot = slot
        self.slots = slots or (([slot] if slot else []))

    async def poll(self) -> JobStatus:
        """Fetch the current job status (single HTTP round-trip)."""
        data = await self._client._request_json(
            "GET", f"/sessions/{self._sid}/jobs/{self.job_id}"
        )
        return JobStatus.from_dict(data)

    async def cancel(self) -> JobStatus:
        """Cancel the job.  No-op if already finished."""
        data = await self._client._request_json(
            "DELETE", f"/sessions/{self._sid}/jobs/{self.job_id}"
        )
        return JobStatus.from_dict(data)

    async def wait(
        self,
        *,
        interval: float = 2.0,
        timeout: Optional[float] = None,
    ) -> JobStatus:
        """
        Poll until the job reaches a terminal state.

        Parameters
        ----------
        interval:
            Seconds between poll requests (default 2 s).
        timeout:
            Maximum seconds to wait.  ``None`` means wait forever.

        Returns
        -------
        JobStatus
            The final status (done / cancelled).

        Raises
        ------
        AstroBurstError
            If the job ends in ``error`` status.
        asyncio.TimeoutError
            If *timeout* is set and elapsed.
        """
        async def _poll_loop() -> JobStatus:
            while True:
                status = await self.poll()
                if status.is_terminal:
                    if status.is_error:
                        raise AstroBurstError(
                            f"Job {self.job_id!r} ended with error",
                            code="job_error",
                            status=0,
                        )
                    return status
                await asyncio.sleep(interval)

        if timeout is not None:
            return await asyncio.wait_for(_poll_loop(), timeout=timeout)
        return await _poll_loop()

    async def stream(self) -> AsyncIterator[Dict[str, Any]]:
        """
        Subscribe to real-time SSE progress events.

        Yields decoded event ``data`` dicts with a ``type`` key:
        - ``{"type": "progress", "pct": 42, "stage": "aligning"}``
        - ``{"type": "complete"}``
        - ``{"type": "error", "message": "..."}``

        Stops automatically after a ``complete`` or ``error`` event.

        **Only one subscriber per job is allowed.**  A second concurrent call
        to ``stream()`` on the same job returns a ``ConflictError`` (409).
        Use ``wait()`` (polling) when multiple callers need job status.

        Yields
        ------
        dict
            Decoded SSE event data.
        """
        url = f"/sessions/{self._sid}/jobs/{self.job_id}/stream"
        async with self._client._http.stream(
            "GET", url, timeout=None
        ) as response:
            if response.status_code != 200:
                body: Any = {}
                try:
                    await response.aread()
                    body = response.json()
                except Exception:
                    pass
                rid = response.headers.get("x-request-id")
                raise error_from_response(response.status_code, body, rid)

            async for line in response.aiter_lines():
                if not line.startswith("data:"):
                    continue
                payload_str = line[len("data:"):].strip()
                if not payload_str:
                    continue
                try:
                    event = _json.loads(payload_str)
                except _json.JSONDecodeError:
                    continue
                yield event
                if event.get("type") in {"complete", "error"}:
                    return


class Session:
    """
    A server-side session that holds an image cache and a job registry.

    Obtain a Session by calling ``AstroBurstClient.create_session()``.
    All methods are coroutines and must be awaited.
    """

    def __init__(self, client: "AstroBurstClient", sid: str) -> None:
        self._client = client
        self.sid = sid

    def _url(self, path: str) -> str:
        return f"/sessions/{self.sid}/{path.lstrip('/')}"

    async def open(
        self,
        path: str,
        slot: Optional[str] = None,
    ) -> OpenResult:
        """
        Load a FITS or ASDF file from the **server's** filesystem into the
        session cache.

        Parameters
        ----------
        path:
            Absolute path on the server machine (e.g. ``"/data/m51.fits"``).
        slot:
            Cache key for subsequent calls.  Defaults to *path* if omitted.

        Returns
        -------
        OpenResult
            Dimensions, statistics, suggested STF parameters, and a compact
            header dict.
        """
        body: Dict[str, Any] = {"path": path}
        if slot is not None:
            body["slot"] = slot
        data = await self._client._request_json(
            "POST", self._url("fits/open"), json=body
        )
        return OpenResult.from_dict(data)

    async def header(self, slot: str) -> HeaderResult:
        """
        Return the full FITS header for a slot already in the cache.

        The slot must have been opened via :meth:`open` first.  Raises
        ``NotFoundError`` if the slot does not exist or was opened without
        header data.
        """
        data = await self._client._request_json(
            "POST", self._url("fits/header"), json={"slot": slot}
        )
        return HeaderResult.from_dict(data)

    async def stf(self, slot: str) -> Tuple[bytes, Stf]:
        """
        Auto-stretch using the Screen Transfer Function and return a PNG.

        Returns
        -------
        (png_bytes, stf_params)
            Raw PNG bytes and the STF parameters used (from the
            ``X-Stf-Params`` response header).
        """
        resp = await self._client._request(
            "POST", self._url("image/stf"), json={"slot": slot}
        )
        png = resp.content
        stf_header = resp.headers.get("x-stf-params", "{}")
        try:
            stf_dict = _json.loads(stf_header)
        except _json.JSONDecodeError:
            stf_dict = {}
        stf = Stf.from_dict(stf_dict) if stf_dict else Stf(0.0, 0.5, 1.0)
        return png, stf

    async def render(
        self,
        slot: str,
        shadow: float,
        midtone: float,
        highlight: float,
    ) -> bytes:
        """
        Render with explicit STF parameters.

        Returns
        -------
        bytes
            Raw PNG bytes.
        """
        resp = await self._client._request(
            "POST",
            self._url("image/render"),
            json={"slot": slot, "shadow": shadow, "midtone": midtone, "highlight": highlight},
        )
        return resp.content

    async def viewport(
        self,
        slot: str,
        x: int,
        y: int,
        w: int,
        h: int,
        *,
        shadow: Optional[float] = None,
        midtone: Optional[float] = None,
        highlight: Optional[float] = None,
    ) -> bytes:
        """
        Render a pixel-coordinate crop as a PNG.

        Returns
        -------
        bytes
            Raw PNG bytes.
        """
        body: Dict[str, Any] = {"slot": slot, "x": x, "y": y, "w": w, "h": h}
        if shadow is not None:
            body["shadow"] = shadow
        if midtone is not None:
            body["midtone"] = midtone
        if highlight is not None:
            body["highlight"] = highlight
        resp = await self._client._request(
            "POST", self._url("image/viewport"), json=body
        )
        return resp.content

    async def stack(
        self,
        paths: List[str],
        *,
        result_slot: Optional[str] = None,
        sigma_low: Optional[float] = None,
        sigma_high: Optional[float] = None,
        max_iterations: Optional[int] = None,
        align: Optional[bool] = None,
        weights: Optional[List[float]] = None,
    ) -> Job:
        """
        Sigma-clip stack a list of FITS files.

        Returns immediately with a :class:`Job` handle.  Call
        ``job.wait()`` or ``job.stream()`` to track progress.

        Parameters
        ----------
        paths:
            Absolute paths on the server machine.
        result_slot:
            Cache key for the stacked result (default: ``"stacked"``).
        """
        body: Dict[str, Any] = {"paths": paths}
        if result_slot is not None:
            body["result_slot"] = result_slot
        if sigma_low is not None:
            body["sigma_low"] = sigma_low
        if sigma_high is not None:
            body["sigma_high"] = sigma_high
        if max_iterations is not None:
            body["max_iterations"] = max_iterations
        if align is not None:
            body["align"] = align
        if weights is not None:
            body["weights"] = weights
        data = await self._client._request_json(
            "POST", self._url("stacking/stack"), json=body
        )
        return Job(
            self._client,
            self.sid,
            data["job_id"],
            slot=data.get("slot"),
        )

    async def drizzle(
        self,
        paths: List[str],
        *,
        result_slot: Optional[str] = None,
        scale: Optional[float] = None,
        pixfrac: Optional[float] = None,
        kernel: Optional[str] = None,
        sigma_low: Optional[float] = None,
        sigma_high: Optional[float] = None,
        align: Optional[bool] = None,
    ) -> Job:
        """
        Drizzle-combine a list of FITS files.

        Returns immediately with a :class:`Job` handle.

        Parameters
        ----------
        kernel:
            One of ``"square"`` (default), ``"gaussian"``, ``"lanczos3"``.
        """
        body: Dict[str, Any] = {"paths": paths}
        if result_slot is not None:
            body["result_slot"] = result_slot
        if scale is not None:
            body["scale"] = scale
        if pixfrac is not None:
            body["pixfrac"] = pixfrac
        if kernel is not None:
            body["kernel"] = kernel
        if sigma_low is not None:
            body["sigma_low"] = sigma_low
        if sigma_high is not None:
            body["sigma_high"] = sigma_high
        if align is not None:
            body["align"] = align
        data = await self._client._request_json(
            "POST", self._url("stacking/drizzle"), json=body
        )
        return Job(
            self._client,
            self.sid,
            data["job_id"],
            slot=data.get("slot"),
        )

    async def pipeline(
        self,
        channels: List[Dict[str, Any]],
        *,
        dark_paths: Optional[List[str]] = None,
        flat_paths: Optional[List[str]] = None,
        bias_paths: Optional[List[str]] = None,
        sigma_low: Optional[float] = None,
        sigma_high: Optional[float] = None,
        normalize: Optional[bool] = None,
        result_prefix: Optional[str] = None,
    ) -> Job:
        """
        Calibrate and stack one or more narrowband channels in a single pass.

        Returns immediately with a :class:`Job` handle whose ``slots`` list
        names the result cache keys (e.g. ``["pipe/Ha", "pipe/OIII"]``).

        Parameters
        ----------
        channels:
            List of ``{"label": str, "paths": [str, ...]}`` dicts.
        result_prefix:
            String prepended to each channel label to form the cache slot key.
            E.g. ``"pipe/"`` → slots ``"pipe/Ha"``, ``"pipe/OIII"``.
        """
        body: Dict[str, Any] = {"channels": channels}
        if dark_paths is not None:
            body["dark_paths"] = dark_paths
        if flat_paths is not None:
            body["flat_paths"] = flat_paths
        if bias_paths is not None:
            body["bias_paths"] = bias_paths
        if sigma_low is not None:
            body["sigma_low"] = sigma_low
        if sigma_high is not None:
            body["sigma_high"] = sigma_high
        if normalize is not None:
            body["normalize"] = normalize
        if result_prefix is not None:
            body["result_prefix"] = result_prefix
        data = await self._client._request_json(
            "POST", self._url("pipeline/run"), json=body
        )
        return Job(
            self._client,
            self.sid,
            data["job_id"],
            slots=data.get("slots"),
        )

    async def get_job(self, job_id: str) -> JobStatus:
        """Poll an existing job by ID."""
        data = await self._client._request_json(
            "GET", f"/sessions/{self.sid}/jobs/{job_id}"
        )
        return JobStatus.from_dict(data)

    async def cancel_job(self, job_id: str) -> JobStatus:
        """Cancel an existing job by ID."""
        data = await self._client._request_json(
            "DELETE", f"/sessions/{self.sid}/jobs/{job_id}"
        )
        return JobStatus.from_dict(data)


class AstroBurstClient:
    """
    Async HTTP client for astroburst-server.

    Use as an async context manager to ensure the underlying HTTP connection
    pool is closed::

        async with AstroBurstClient("http://localhost:8080") as client:
            health = await client.health()
            session = await client.create_session()
            ...

    Or manage the lifetime manually::

        client = AstroBurstClient()
        try:
            ...
        finally:
            await client.aclose()

    Parameters
    ----------
    base_url:
        Server URL.  Defaults to ``"http://localhost:8080"`` (the SSH-tunnel
        convention described in SERVER.md).
    timeout:
        Default request timeout in seconds for JSON endpoints.  Render
        endpoints use the same timeout.  SSE streaming disables the timeout.
    client:
        Inject an existing ``httpx.AsyncClient`` (useful for testing).
    """

    def __init__(
        self,
        base_url: str = "http://localhost:8080",
        *,
        timeout: float = 30.0,
        client: Optional[httpx.AsyncClient] = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout
        self._owns_client = client is None
        self._http = client or httpx.AsyncClient(
            base_url=self._base_url,
            timeout=httpx.Timeout(timeout),
        )

    async def __aenter__(self) -> "AstroBurstClient":
        return self

    async def __aexit__(self, *_: Any) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        """Close the underlying HTTP connection pool."""
        if self._owns_client:
            await self._http.aclose()

    async def _request(
        self,
        method: str,
        path: str,
        *,
        json: Optional[Any] = None,
    ) -> httpx.Response:
        """
        Send an HTTP request and return the raw response.

        Raises an :class:`AstroBurstError` subclass on any non-2xx status.
        """
        resp = await self._http.request(method, path, json=json)
        if resp.status_code < 200 or resp.status_code >= 300:
            rid = resp.headers.get("x-request-id")
            try:
                body = resp.json()
            except Exception:
                body = {}
            raise error_from_response(resp.status_code, body, rid)
        return resp

    async def _request_json(
        self,
        method: str,
        path: str,
        *,
        json: Optional[Any] = None,
    ) -> Dict[str, Any]:
        """Like ``_request`` but parses and returns the JSON body."""
        resp = await self._request(method, path, json=json)
        return resp.json()

    async def health(self) -> HealthResult:
        """
        Check server health.

        No session required.  Returns server version and session counters.
        """
        data = await self._request_json("GET", "/health")
        return HealthResult.from_dict(data)

    async def create_session(self) -> Session:
        """
        Create a new server-side session.

        Returns
        -------
        Session
            A session object scoped to the new session ID.

        Raises
        ------
        ServiceUnavailableError
            When the server-configured session cap (``ASTROBURST_SESSION_MAX``)
            has been reached.
        """
        resp = await self._http.post("/sessions")
        rid = resp.headers.get("x-request-id")
        if resp.status_code != 201:
            try:
                body = resp.json()
            except Exception:
                body = {}
            raise error_from_response(resp.status_code, body, rid)
        sid = resp.json()["session_id"]
        return Session(self, sid)
