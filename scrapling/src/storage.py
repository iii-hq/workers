"""Adaptive-element storage location for Scrapling's Smart Element Tracking.

Adaptive relocation persists each saved element's identity to a SQLite file
keyed by (registrable-domain, identifier). For that to survive across calls the
DB must live at a stable path — resolved here from config and created on boot.

Host-persistence caveat: a `deploy: image` worker runs as a local subprocess, so
this file persists across calls and worker restarts on the same host, but NOT
across a fresh container redeploy (there is no manifest volume). Point
`adaptive_storage_path` at a durable location if you need cross-deploy identities.
"""

from __future__ import annotations

import os
from pathlib import Path

DEFAULT_PATH = "./data/scrapling/elements.db"

_db_path = DEFAULT_PATH


def configure(path: str | None) -> str:
    """Set the adaptive DB path (call once at boot) and ensure its parent exists."""
    global _db_path
    _db_path = os.path.expanduser(path or DEFAULT_PATH)
    _ensure_parent(_db_path)
    return _db_path


def db_path() -> str:
    """Current adaptive DB path; makes the parent dir lazily (tests skip configure)."""
    _ensure_parent(_db_path)
    return _db_path


def _ensure_parent(path: str) -> None:
    Path(path).expanduser().parent.mkdir(parents=True, exist_ok=True)
