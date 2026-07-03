"""Persistent Scrapling sessions over the bus.

A live session — a curl_cffi HTTP client, or a Camoufox/Chromium browser — can't
be serialized, so it lives in an in-process registry keyed by session id, not in
`state::`. Browser objects are **thread-affine** (Playwright), so each session
owns a dedicated single-worker thread; every open/fetch/close for that session
runs on that one thread. The registry caps concurrent sessions and (optionally)
reaps idle ones so an abandoned browser can't leak forever.

`main.py` calls `close_all()` on shutdown — the daemon `iii.shutdown()` will not
tear down live browsers for you.
"""

from __future__ import annotations

import threading
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from typing import Any

from . import core

# Session-constructor params for the HTTP session (curl_cffi). Browser sessions
# reuse the fetcher allowlists from `core` (DynamicSession/StealthySession take
# the same kwargs as the one-shot fetchers).
_HTTP_SESSION_INIT = frozenset(
    {
        "impersonate",
        "http3",
        "stealthy_headers",
        "proxy",
        "proxies",
        "proxy_auth",
        "timeout",
        "headers",
        "retries",
        "retry_delay",
        "follow_redirects",
        "max_redirects",
        "verify",
    }
)

_SESSION_TYPES = ("http", "dynamic", "stealthy")


class _Entry:
    def __init__(self, sid: str, stype: str, obj: Any, entered: Any, executor: ThreadPoolExecutor):
        self.id = sid
        self.type = stype
        self.obj = obj  # the Session instance (browser fetch target)
        self.entered = entered  # __enter__ result (HTTP logic object with get/post/…)
        self.executor = executor
        self.created_at = time.time()
        self.last_used = self.created_at


def _construct(stype: str, config: dict[str, Any]) -> tuple[Any, Any]:
    """Build + enter a session. Runs on the session's own thread (browser affinity)."""
    if stype == "http":
        from scrapling.fetchers import FetcherSession

        obj = FetcherSession(**core._pick(config, _HTTP_SESSION_INIT))
    elif stype == "dynamic":
        from scrapling.fetchers import DynamicSession

        obj = DynamicSession(**core._pick(config, core._DYNAMIC_KEYS))
    elif stype == "stealthy":
        from scrapling.fetchers import StealthySession

        obj = StealthySession(**core._pick(config, core._STEALTHY_KEYS))
    else:
        raise ValueError(f"unknown session type: {stype!r} (use http|dynamic|stealthy)")
    entered = obj.__enter__()
    return obj, entered


def _do_fetch(entry: _Entry, payload: dict[str, Any]) -> dict[str, Any]:
    entry.last_used = time.time()
    url = payload["url"]
    if entry.type == "http":
        method = (payload.get("method") or "get").lower()
        if method not in ("get", "post", "put", "delete"):
            raise ValueError(f"unsupported method: {method}")
        keys = core._HTTP_GET_KEYS if method == "get" else core._HTTP_DATA_KEYS
        page = getattr(entry.entered, method)(url, **core._pick(payload, keys))
    else:
        allowed = core._STEALTHY_KEYS if entry.type == "stealthy" else core._DYNAMIC_KEYS
        page = entry.obj.fetch(url, **core._pick(payload, allowed))
    return core.serialize_page(page, payload, bool(payload.get("include_html", False)))


def _teardown(entry: _Entry) -> None:
    try:
        entry.obj.__exit__(None, None, None)
    except Exception:  # noqa: BLE001 - best-effort close; never raise on teardown
        pass


def _shutdown_entry(entry: _Entry) -> None:
    try:
        entry.executor.submit(_teardown, entry).result(timeout=30)
    except Exception:  # noqa: BLE001
        pass
    finally:
        entry.executor.shutdown(wait=False)


class Registry:
    """Thread-safe registry of live sessions. Each session runs on its own thread."""

    def __init__(self, max_sessions: int = 8, idle_timeout: float | None = None):
        self._lock = threading.RLock()
        self._entries: dict[str, _Entry] = {}
        self.max_sessions = max_sessions
        self.idle_timeout = idle_timeout
        self._stop = threading.Event()
        self._reaper: threading.Thread | None = None

    def open(self, stype: str, config: dict[str, Any]) -> dict[str, Any]:
        self._reap_idle()
        with self._lock:
            if len(self._entries) >= self.max_sessions:
                raise ValueError(f"session limit reached ({self.max_sessions}); close one first")
        sid = uuid.uuid4().hex
        executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="scrapling-sess")
        try:
            obj, entered = executor.submit(_construct, stype, config).result()
        except BaseException:
            executor.shutdown(wait=False)
            raise
        entry = _Entry(sid, stype, obj, entered, executor)
        with self._lock:
            self._entries[sid] = entry
        return {"session_id": sid, "type": stype}

    def fetch(self, sid: str, payload: dict[str, Any]) -> dict[str, Any]:
        entry = self._require(sid)
        return entry.executor.submit(_do_fetch, entry, payload).result()

    def close(self, sid: str) -> dict[str, Any]:
        with self._lock:
            entry = self._entries.pop(sid, None)
        if entry is None:
            return {"closed": False}
        _shutdown_entry(entry)
        return {"closed": True}

    def list(self, type_filter: str | None = None) -> dict[str, Any]:
        now = time.time()
        with self._lock:
            entries = list(self._entries.values())
        return {
            "sessions": [
                {
                    "session_id": e.id,
                    "type": e.type,
                    "created_at": e.created_at,
                    "last_used": e.last_used,
                    "idle_s": round(now - e.last_used, 1),
                }
                for e in entries
                if type_filter is None or e.type == type_filter
            ]
        }

    def close_all(self) -> None:
        self._stop.set()
        with self._lock:
            ids = list(self._entries.keys())
        for sid in ids:
            self.close(sid)

    def start_reaper(self) -> None:
        if self.idle_timeout and self._reaper is None:
            self._reaper = threading.Thread(target=self._reaper_loop, name="scrapling-reaper", daemon=True)
            self._reaper.start()

    def _reaper_loop(self) -> None:
        interval = min(self.idle_timeout or 60, 60)
        while not self._stop.wait(interval):
            self._reap_idle()

    def _reap_idle(self) -> None:
        if not self.idle_timeout:
            return
        cutoff = time.time() - self.idle_timeout
        with self._lock:
            stale = [e.id for e in self._entries.values() if e.last_used < cutoff]
        for sid in stale:
            self.close(sid)

    def _require(self, sid: str) -> _Entry:
        with self._lock:
            entry = self._entries.get(sid)
        if entry is None:
            raise ValueError(f"unknown session: {sid}")
        return entry


_registry: Registry | None = None


def setup(max_sessions: int = 8, idle_timeout: float | None = None) -> Registry:
    """Create the process-wide session registry. Call once at boot."""
    global _registry
    _registry = Registry(max_sessions=max_sessions, idle_timeout=idle_timeout)
    _registry.start_reaper()
    return _registry


def registry() -> Registry:
    if _registry is None:
        raise RuntimeError("sessions.setup() was not called")
    return _registry
