"""Declarative crawl over the fetchers.

Scrapling's `Spider` is driven by Python `parse()` callbacks and writes host
checkpoints, so it can't be exposed over a JSON bus. `scrapling::crawl` instead
takes a declarative spec — start_urls + follow/domain rules + per-page selectors
— and BFS-crawls with the fetchers, emitting each scraped item over `stream::set`
as it's found (keyed by `group_id`) and returning a summary + a small sample.
"""

from __future__ import annotations

import asyncio
import uuid
from collections import deque
from typing import Any
from urllib.parse import urlparse

from . import core

_SAMPLE_MAX = 10
# Top-level crawl fields forwarded to every page fetch.
_FETCH_KEYS = frozenset(
    {
        "impersonate",
        "proxy",
        "headless",
        "network_idle",
        "solve_cloudflare",
        "real_chrome",
        "wait_selector",
        "timeout",
        "useragent",
    }
)


def _int(payload: dict[str, Any], key: str, default: int) -> int:
    """int() a payload field, treating an explicit JSON null as absent (an LLM
    readily emits `"max_pages": null`, which int(None) would crash on)."""
    v = payload.get(key)
    return default if v is None else int(v)


def _host(url: str) -> str:
    return urlparse(url).netloc.lower()


def _reg(host: str) -> str:
    """Fold a leading www. so apex and www count as the same site."""
    return host[4:] if host.startswith("www.") else host


def _same_site(host: str, start_hosts: set[str]) -> bool:
    rh = _reg(host)
    for sh in start_hosts:
        rs = _reg(sh)
        if rh == rs or rh.endswith("." + rs) or rs.endswith("." + rh):
            return True
    return False


def _domain_ok(url: str, same_domain: bool, start_hosts: set[str], allowed: set[str]) -> bool:
    host = _host(url)
    if not host:
        return False
    if allowed:
        return any(host == d or host.endswith("." + d) for d in allowed)
    if same_domain:
        return _same_site(host, start_hosts)
    return True


async def run_crawl(cfg: dict[str, Any], iii: Any, payload: dict[str, Any]) -> dict[str, Any]:
    start_urls = payload.get("start_urls") or ([payload["url"]] if payload.get("url") else [])
    if not start_urls:
        raise ValueError("provide `start_urls`")
    tier = payload.get("fetcher", "http")
    if tier not in ("http", "stealthy", "dynamic"):
        raise ValueError(f"unknown fetcher: {tier!r} (use http|stealthy|dynamic)")

    selectors = payload.get("selectors")
    fmt = payload.get("format")
    include_html = bool(payload.get("include_html", False))
    max_pages = _int(payload, "max_pages", 20)
    max_depth = _int(payload, "max_depth", 2)
    # Server-side ceiling: the caller can lower concurrency but not exceed the
    # configured bulk cap (browsers are RAM-hungry — an unbounded fan-out OOMs).
    ceiling = int(cfg.get("max_bulk_concurrency", 5))
    concurrency = max(1, min(_int(payload, "concurrency", ceiling), ceiling))
    delay = float(payload.get("download_delay", 0) or 0)
    same_domain = bool(payload.get("same_domain", True))
    allowed = {d.lower() for d in (payload.get("allowed_domains") or [])}
    stream_name = payload.get("stream_name") or "scrapling::crawl"
    group_id = payload.get("group_id") or uuid.uuid4().hex

    start_hosts = {_host(u) for u in start_urls}
    seen: set[str] = set(start_urls)
    frontier: deque[tuple[str, int]] = deque((u, 0) for u in start_urls)
    stats = {"crawled": 0, "items": 0, "errors": 0, "stopped": "done"}
    sample: list[dict[str, Any]] = []
    seq = 0

    async def emit(data: dict[str, Any]) -> None:
        nonlocal seq
        if iii is None:  # tests / no bus: still crawl, just don't stream
            return
        seq += 1
        try:
            await iii.trigger_async(
                {
                    "function_id": "stream::set",
                    "payload": {
                        "stream_name": stream_name,
                        "group_id": group_id,
                        "item_id": f"{group_id}-{seq:06d}",
                        "data": data,
                    },
                }
            )
        except Exception:  # noqa: BLE001 - streaming is best-effort; never sink the crawl
            pass

    def record(data: dict[str, Any]) -> None:
        if len(sample) < _SAMPLE_MAX:
            sample.append(data)

    async def visit(url: str, depth: int) -> dict[str, Any]:
        fetch_payload: dict[str, Any] = {**core._pick(payload, _FETCH_KEYS), "url": url}
        if selectors:
            fetch_payload["selectors"] = selectors
        if fmt:
            fetch_payload["format"] = fmt
        # The WHOLE fetch→parse→link-extract is guarded: a bad selector or
        # Convertor failure on one page must not sink the crawl (per-url isolation).
        try:
            page = await asyncio.to_thread(core.fetch_raw, cfg, fetch_payload, tier)
            serialized = core.serialize_page(page, fetch_payload, include_html)
            links = core.extract_links(page) if depth < max_depth else []
        except Exception as exc:  # noqa: BLE001 - surface per-url, keep crawling
            return {"url": url, "depth": depth, "error": str(exc)}
        return {"url": url, "depth": depth, "page": serialized, "links": links}

    # Bounded worker pool: keep up to `concurrency` visits in flight, starting a
    # new one the moment any finishes (no head-of-line blocking), and never
    # START more than max_pages fetches total.
    pending: set[asyncio.Task] = set()
    started = 0
    while frontier or pending:
        while frontier and len(pending) < concurrency and started < max_pages:
            url, depth = frontier.popleft()
            pending.add(asyncio.create_task(visit(url, depth)))
            started += 1
        if not pending:
            break  # frontier left but max_pages reached — stop starting
        done_set, pending = await asyncio.wait(pending, return_when=asyncio.FIRST_COMPLETED)
        for task in done_set:
            res = task.result()  # visit never raises
            stats["crawled"] += 1
            depth = res["depth"]
            if "error" in res:
                stats["errors"] += 1
                err_item = {"url": res["url"], "error": res["error"]}
                await emit(err_item)
                record(err_item)
                continue
            page = res["page"]
            item: dict[str, Any] = {"url": res["url"], "status": page.get("status")}
            if page.get("extracted"):
                item["extracted"] = page["extracted"]
            if page.get("content") is not None:
                item["content"] = page["content"]
            if include_html and page.get("html") is not None:
                item["html"] = page["html"]
            stats["items"] += 1
            await emit(item)
            record(item)
            for link in res["links"]:
                if link in seen or not _domain_ok(link, same_domain, start_hosts, allowed):
                    continue
                seen.add(link)
                frontier.append((link, depth + 1))
        if delay:
            await asyncio.sleep(delay)

    if frontier:  # only reason to exit with links unvisited is the page cap
        stats["stopped"] = "max_pages"
    return {"stats": stats, "items": sample, "stream": {"name": stream_name, "group_id": group_id}}
