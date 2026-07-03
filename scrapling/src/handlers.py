"""Async scrapling::* handlers over the pure `core` ops.

Fetchers are synchronous and (for browsers) blocking, so each runs in a worker
thread via `asyncio.to_thread` to keep the SDK event loop responsive. A fetch
with a `urls` array fans out concurrently under a semaphore; a per-url error is
captured so one bad URL doesn't sink the batch. Parse-only ops are cheap and run
inline. Handlers need no iii client — Scrapling calls are pure request→response.
"""

from __future__ import annotations

import asyncio
from typing import Any, Awaitable, Callable

from . import core

FetchOp = Callable[[dict[str, Any], dict[str, Any]], dict[str, Any]]


def _require_target(payload: dict[str, Any]) -> None:
    if not payload.get("url") and not payload.get("urls"):
        raise ValueError("provide `url` or `urls`")


async def _run_fetch(cfg: dict[str, Any], payload: dict[str, Any], op: FetchOp) -> dict[str, Any]:
    _require_target(payload)
    urls = payload.get("urls")
    if not urls:
        return await asyncio.to_thread(op, cfg, payload)

    sem = asyncio.Semaphore(int(cfg.get("max_bulk_concurrency", 5)))

    async def one(url: str) -> dict[str, Any]:
        single = {k: v for k, v in payload.items() if k != "urls"}
        single["url"] = url
        async with sem:
            try:
                return await asyncio.to_thread(op, cfg, single)
            except Exception as exc:  # noqa: BLE001 - surface per-url, don't sink the batch
                return {"url": url, "error": str(exc)}

    results = await asyncio.gather(*[one(u) for u in urls])
    return {"results": list(results)}


def create_handlers(
    get_cfg: Callable[[], dict[str, Any]],
) -> dict[str, Callable[[dict[str, Any]], Awaitable[dict[str, Any]]]]:
    async def fetch(payload: dict[str, Any]) -> dict[str, Any]:
        return await _run_fetch(get_cfg(), payload, core.do_fetch)

    async def stealthy_fetch(payload: dict[str, Any]) -> dict[str, Any]:
        return await _run_fetch(get_cfg(), payload, core.do_stealthy)

    async def dynamic_fetch(payload: dict[str, Any]) -> dict[str, Any]:
        return await _run_fetch(get_cfg(), payload, core.do_dynamic)

    async def screenshot(payload: dict[str, Any]) -> dict[str, Any]:
        if not payload.get("url"):
            raise ValueError("provide `url`")
        return await asyncio.to_thread(core.do_screenshot, get_cfg(), payload)

    async def extract(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_extract(payload)

    async def css(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_query(payload, "css")

    async def xpath(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_query(payload, "xpath")

    async def regex(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_regex(payload)

    async def find_similar(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_find_similar(payload)

    return {
        "fetch": fetch,
        "stealthy_fetch": stealthy_fetch,
        "dynamic_fetch": dynamic_fetch,
        "screenshot": screenshot,
        "extract": extract,
        "css": css,
        "xpath": xpath,
        "regex": regex,
        "find_similar": find_similar,
    }
