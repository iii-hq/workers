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


def _host(url: str) -> str:
    return urlparse(url).netloc.lower()


def _domain_ok(url: str, same_domain: bool, start_hosts: set[str], allowed: set[str]) -> bool:
    host = _host(url)
    if not host:
        return False
    if allowed:
        return any(host == d or host.endswith("." + d) for d in allowed)
    if same_domain:
        return host in start_hosts
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
    max_pages = int(payload.get("max_pages", 20))
    max_depth = int(payload.get("max_depth", 2))
    concurrency = max(1, int(payload.get("concurrency", cfg.get("max_bulk_concurrency", 5))))
    delay = float(payload.get("download_delay", 0) or 0)
    same_domain = bool(payload.get("same_domain", True))
    allowed = {d.lower() for d in (payload.get("allowed_domains") or [])}
    stream_name = payload.get("stream_name") or "scrapling::crawl"
    group_id = payload.get("group_id") or uuid.uuid4().hex

    start_hosts = {_host(u) for u in start_urls}
    seen: set[str] = set(start_urls)
    queue: list[tuple[str, int]] = [(u, 0) for u in start_urls]
    sem = asyncio.Semaphore(concurrency)
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

    async def visit(url: str, depth: int) -> dict[str, Any]:
        async with sem:
            fetch_payload: dict[str, Any] = {**core._pick(payload, _FETCH_KEYS), "url": url}
            if selectors:
                fetch_payload["selectors"] = selectors
            if fmt:
                fetch_payload["format"] = fmt
            try:
                page = await asyncio.to_thread(core.fetch_raw, cfg, fetch_payload, tier)
            except Exception as exc:  # noqa: BLE001 - surface per-url, keep crawling
                return {"url": url, "error": str(exc)}
            serialized = core.serialize_page(page, fetch_payload, include_html)
            links = core.extract_links(page) if depth < max_depth else []
            return {"url": url, "page": serialized, "links": links}

    while queue and stats["crawled"] < max_pages:
        budget = max_pages - stats["crawled"]
        batch = queue[: min(concurrency, budget)]
        queue = queue[len(batch) :]
        results = await asyncio.gather(*[visit(u, d) for u, d in batch])
        for (url, depth), res in zip(batch, results):
            stats["crawled"] += 1
            if "error" in res:
                stats["errors"] += 1
                await emit({"url": url, "error": res["error"]})
                continue
            page = res["page"]
            item: dict[str, Any] = {"url": res["url"], "status": page.get("status")}
            if page.get("extracted"):
                item["extracted"] = page["extracted"]
            if page.get("content") is not None:
                item["content"] = page["content"]
            stats["items"] += 1
            await emit(item)
            if len(sample) < _SAMPLE_MAX:
                sample.append(item)
            for link in res["links"]:
                if link in seen or not _domain_ok(link, same_domain, start_hosts, allowed):
                    continue
                seen.add(link)
                queue.append((link, depth + 1))
        if delay:
            await asyncio.sleep(delay)

    if queue and stats["crawled"] >= max_pages:
        stats["stopped"] = "max_pages"
    return {"stats": stats, "items": sample, "stream": {"name": stream_name, "group_id": group_id}}
