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

from . import core, crawl, sessions

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
    iii: Any = None,
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

    async def find(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_find(payload)

    async def find_by_text(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_find_by_text(payload)

    async def find_by_regex(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_find_by_regex(payload)

    async def describe(payload: dict[str, Any]) -> dict[str, Any]:
        return core.op_describe(payload)

    async def to_markdown(payload: dict[str, Any]) -> dict[str, Any]:
        return await asyncio.to_thread(core.op_to_markdown, payload)

    # Sessions: the registry blocks on a per-session thread, so offload to a
    # pool thread to keep the SDK event loop responsive.
    async def session_open(payload: dict[str, Any]) -> dict[str, Any]:
        stype = payload.get("type", "http")
        config = {k: v for k, v in payload.items() if k != "type"}
        return await asyncio.to_thread(sessions.registry().open, stype, config)

    async def session_fetch(payload: dict[str, Any]) -> dict[str, Any]:
        if not payload.get("session_id"):
            raise ValueError("provide `session_id`")
        if not payload.get("url"):
            raise ValueError("provide `url`")
        return await asyncio.to_thread(sessions.registry().fetch, payload["session_id"], payload)

    async def session_close(payload: dict[str, Any]) -> dict[str, Any]:
        if not payload.get("session_id"):
            raise ValueError("provide `session_id`")
        return await asyncio.to_thread(sessions.registry().close, payload["session_id"])

    async def session_list(payload: dict[str, Any]) -> dict[str, Any]:
        return await asyncio.to_thread(sessions.registry().list, payload.get("type"))

    async def crawl_handler(payload: dict[str, Any]) -> dict[str, Any]:
        return await crawl.run_crawl(get_cfg(), iii, payload)

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
        "find": find,
        "find_by_text": find_by_text,
        "find_by_regex": find_by_regex,
        "describe": describe,
        "to_markdown": to_markdown,
        "session_open": session_open,
        "session_fetch": session_fetch,
        "session_close": session_close,
        "session_list": session_list,
        "crawl": crawl_handler,
    }
