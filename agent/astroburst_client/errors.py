# AstroBurst Python client — contributed by Jae-Joon Lee <https://github.com/leejjoon>
"""
Error types for the AstroBurst client.

The server returns errors as:
    {"success": false, "error": {"code": "<code>", "message": "<msg>"}}

Every non-2xx response raises an AstroBurstError subclass keyed on `code`.
The request-id from the X-Request-Id header is threaded into the exception so
callers can correlate it with server-side logs.
"""
from __future__ import annotations

from typing import Any, Dict, Optional


class AstroBurstError(Exception):
    """Base class for all astroburst-server errors."""

    def __init__(
        self,
        message: str,
        *,
        code: str = "unknown",
        status: int = 0,
        request_id: Optional[str] = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.code = code
        self.status = status
        self.request_id = request_id

    def __repr__(self) -> str:
        rid = f", request_id={self.request_id!r}" if self.request_id else ""
        return (
            f"{type(self).__name__}(status={self.status}, code={self.code!r}, "
            f"message={self.message!r}{rid})"
        )


class BadRequestError(AstroBurstError):
    """400 bad_request — missing or invalid parameter."""


class NotFoundError(AstroBurstError):
    """404 not_found — session, slot, or job does not exist."""


class ConflictError(AstroBurstError):
    """409 conflict — SSE stream already has a subscriber."""


class TooManyRequestsError(AstroBurstError):
    """429 too_many_requests — all job semaphore slots occupied."""


class ServiceUnavailableError(AstroBurstError):
    """503 service_unavailable — session cap reached."""


class InternalServerError(AstroBurstError):
    """500 internal_error — unexpected server error."""


_CODE_MAP: Dict[str, type] = {
    "bad_request": BadRequestError,
    "not_found": NotFoundError,
    "conflict": ConflictError,
    "too_many_requests": TooManyRequestsError,
    "service_unavailable": ServiceUnavailableError,
    "internal_error": InternalServerError,
}


def error_from_response(
    status: int,
    body: Any,
    request_id: Optional[str],
) -> AstroBurstError:
    """
    Build the most specific AstroBurstError from a non-2xx response.

    `body` should be the parsed JSON dict, but we guard against non-dict
    bodies (e.g. plain text or HTML from a proxy) so the caller never has
    to handle a secondary parse error.
    """
    code = "unknown"
    message = f"HTTP {status}"

    if isinstance(body, dict):
        err = body.get("error") or {}
        if isinstance(err, dict):
            code = err.get("code", "unknown")
            message = err.get("message", message)

    cls = _CODE_MAP.get(code, AstroBurstError)
    return cls(message, code=code, status=status, request_id=request_id)
