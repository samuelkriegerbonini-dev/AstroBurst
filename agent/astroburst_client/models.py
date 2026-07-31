# AstroBurst Python client — contributed by Jae-Joon Lee <https://github.com/leejjoon>
"""
Typed response models for the AstroBurst server API.

All models are plain dataclasses with `from_dict` classmethods so they can be
constructed directly from the JSON the server returns.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Tuple


@dataclass
class Stats:
    """Per-image pixel statistics returned by fits/open."""

    min: float
    max: float
    median: float
    mean: float
    sigma: float
    mad: float
    valid_count: int

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "Stats":
        return cls(
            min=float(d["min"]),
            max=float(d["max"]),
            median=float(d["median"]),
            mean=float(d["mean"]),
            sigma=float(d["sigma"]),
            mad=float(d["mad"]),
            valid_count=int(d["valid_count"]),
        )


@dataclass
class Stf:
    """Screen Transfer Function parameters (shadow / midtone / highlight)."""

    shadow: float
    midtone: float
    highlight: float

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "Stf":
        return cls(
            shadow=float(d["shadow"]),
            midtone=float(d["midtone"]),
            highlight=float(d["highlight"]),
        )


@dataclass
class OpenResult:
    """Response from POST /sessions/:sid/fits/open."""

    slot: str
    dims: Tuple[int, int]
    stats: Stats
    stf: Stf
    header: Dict[str, Any]

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "OpenResult":
        dims_raw = d["dims"]
        return cls(
            slot=d["slot"],
            dims=(int(dims_raw[0]), int(dims_raw[1])),
            stats=Stats.from_dict(d["stats"]),
            stf=Stf.from_dict(d["stf"]),
            header=d.get("header") or {},
        )

    @property
    def width(self) -> int:
        return self.dims[0]

    @property
    def height(self) -> int:
        return self.dims[1]


@dataclass
class HeaderCard:
    """A single FITS header keyword-value pair."""

    key: str
    value: Any

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "HeaderCard":
        return cls(key=d["key"], value=d["value"])


@dataclass
class HeaderResult:
    """Response from POST /sessions/:sid/fits/header."""

    slot: str
    total_cards: int
    cards: List[HeaderCard]
    index: Dict[str, Any]

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "HeaderResult":
        return cls(
            slot=d["slot"],
            total_cards=int(d["total_cards"]),
            cards=[HeaderCard.from_dict(c) for c in d.get("cards", [])],
            index=d.get("index") or {},
        )


@dataclass
class JobStatus:
    """Job status snapshot returned by GET/DELETE /sessions/:sid/jobs/:jid."""

    id: str
    action: str
    status: str
    pct: int
    started_at: Optional[int]
    completed_at: Optional[int]

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "JobStatus":
        return cls(
            id=d["id"],
            action=d["action"],
            status=d["status"],
            pct=int(d.get("pct") or 0),
            started_at=d.get("started_at"),
            completed_at=d.get("completed_at"),
        )

    @property
    def is_terminal(self) -> bool:
        """True when the job has reached a final state (done/error/cancelled)."""
        return self.status in {"done", "error", "cancelled"}

    @property
    def is_done(self) -> bool:
        return self.status == "done"

    @property
    def is_error(self) -> bool:
        return self.status == "error"

    @property
    def is_cancelled(self) -> bool:
        return self.status == "cancelled"


@dataclass
class HealthResult:
    """Response from GET /health."""

    status: str
    version: str
    sessions_active: int
    sessions_total: int

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "HealthResult":
        return cls(
            status=d["status"],
            version=d["version"],
            sessions_active=int(d["sessions_active"]),
            sessions_total=int(d["sessions_total"]),
        )
