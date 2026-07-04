"""Declarative crawl: BFS link-following, dedup, depth/page caps, domain filter,
and per-URL error capture — with `core.fetch_raw` faked (no network) and no bus."""

from __future__ import annotations

import asyncio
import threading
import time

import pytest
from scrapling import Selector

from src import core, crawl

GRAPH = {
    "https://site.com/": "<html><body><h1>Home</h1>"
    '<a href="/a">A</a><a href="/b">B</a><a href="https://other.com/x">ext</a></body></html>',
    "https://site.com/a": '<html><body><h1>A</h1><a href="/">home</a><a href="/c">C</a></body></html>',
    "https://site.com/b": "<html><body><h1>B</h1></body></html>",
    "https://site.com/c": "<html><body><h1>C</h1></body></html>",
}


class FakePage:
    def __init__(self, url: str, html: str):
        self._sel = Selector(content=html, url=url)
        self.status = 200
        self.url = url
        self.headers = {"content-type": "text/html"}
        self.cookies = {}
        self.encoding = "utf-8"
        self.html_content = self._sel.html_content

    def css(self, *a, **k):
        return self._sel.css(*a, **k)

    def xpath(self, *a, **k):
        return self._sel.xpath(*a, **k)

    def get_all_text(self, *a, **k):
        return self._sel.get_all_text(*a, **k)

    def urljoin(self, u):
        return self._sel.urljoin(u)


def _patch(monkeypatch, graph=GRAPH, raise_on=None):
    def fake_fetch_raw(cfg, payload, tier):
        url = payload["url"]
        if raise_on and raise_on in url:
            raise RuntimeError("boom")
        return FakePage(url, graph[url])

    monkeypatch.setattr(core, "fetch_raw", fake_fetch_raw)


CFG = {"max_bulk_concurrency": 3}
SEL = [{"name": "h", "css": "h1"}]


def test_crawl_follows_same_domain_links(monkeypatch):
    _patch(monkeypatch)
    out = asyncio.run(
        crawl.run_crawl(CFG, None, {"start_urls": ["https://site.com/"], "selectors": SEL, "max_depth": 2})
    )
    stats = out["stats"]
    assert stats["crawled"] == 4  # /, /a, /b, /c — NOT other.com
    assert stats["items"] == 4
    assert stats["errors"] == 0
    assert stats["stopped"] == "done"
    titles = {i["extracted"]["h"] for i in out["items"]}
    assert titles == {"Home", "A", "B", "C"}


def test_crawl_respects_max_pages(monkeypatch):
    _patch(monkeypatch)
    out = asyncio.run(
        crawl.run_crawl(CFG, None, {"start_urls": ["https://site.com/"], "selectors": SEL, "max_pages": 2})
    )
    assert out["stats"]["crawled"] == 2
    assert out["stats"]["stopped"] == "max_pages"


def test_crawl_max_depth_zero_visits_only_start(monkeypatch):
    _patch(monkeypatch)
    out = asyncio.run(
        crawl.run_crawl(CFG, None, {"start_urls": ["https://site.com/"], "selectors": SEL, "max_depth": 0})
    )
    assert out["stats"]["crawled"] == 1


def test_crawl_allowed_domains_filter(monkeypatch):
    # Allow only a domain that never appears → nothing beyond the start page follows.
    _patch(monkeypatch)
    out = asyncio.run(
        crawl.run_crawl(
            CFG,
            None,
            {"start_urls": ["https://site.com/"], "selectors": SEL, "allowed_domains": ["nope.com"]},
        )
    )
    assert out["stats"]["crawled"] == 1


def test_crawl_captures_per_url_errors(monkeypatch):
    _patch(monkeypatch, raise_on="/b")
    out = asyncio.run(
        crawl.run_crawl(CFG, None, {"start_urls": ["https://site.com/"], "selectors": SEL, "max_depth": 2})
    )
    assert out["stats"]["errors"] == 1
    assert out["stats"]["items"] == 3  # / , /a, /c succeed; /b errors


def test_crawl_requires_start_urls():
    with pytest.raises(ValueError, match="start_urls"):
        asyncio.run(crawl.run_crawl(CFG, None, {}))


def test_crawl_streams_each_item(monkeypatch):
    _patch(monkeypatch)
    emitted = []

    class FakeIII:
        async def trigger_async(self, req):
            emitted.append(req)
            return None

    out = asyncio.run(
        crawl.run_crawl(
            CFG, FakeIII(), {"start_urls": ["https://site.com/"], "selectors": SEL, "stream_name": "crawl-x"}
        )
    )
    assert len(emitted) == 4
    first = emitted[0]["payload"]
    assert first["stream_name"] == "crawl-x"
    assert first["group_id"] == out["stream"]["group_id"]
    assert "data" in first


WWW_GRAPH = {
    "https://example.com/": '<html><body><a href="https://www.example.com/a">A</a></body></html>',
    "https://www.example.com/a": "<html><body><h1>A</h1></body></html>",
}


def test_crawl_same_domain_treats_www_as_same_site(monkeypatch):
    _patch(monkeypatch, graph=WWW_GRAPH)
    out = asyncio.run(crawl.run_crawl(CFG, None, {"start_urls": ["https://example.com/"], "selectors": SEL}))
    assert out["stats"]["crawled"] == 2  # apex seed follows the www link


FRAG_GRAPH = {
    "https://s.com/": '<html><body><a href="#x">x</a><a href="#y">y</a><a href="/p">p</a></body></html>',
    "https://s.com/p": "<html><body><h1>P</h1></body></html>",
}


def test_crawl_strips_fragment_links(monkeypatch):
    _patch(monkeypatch, graph=FRAG_GRAPH)
    out = asyncio.run(crawl.run_crawl(CFG, None, {"start_urls": ["https://s.com/"]}))
    # #x/#y fold onto the (already-seen) start URL, so only / and /p are fetched.
    assert out["stats"]["crawled"] == 2


def test_crawl_caps_concurrency_at_config(monkeypatch):
    peak = {"cur": 0, "max": 0}
    lock = threading.Lock()

    def fake(cfg, payload, tier):
        with lock:
            peak["cur"] += 1
            peak["max"] = max(peak["max"], peak["cur"])
        time.sleep(0.02)
        with lock:
            peak["cur"] -= 1
        return FakePage(payload["url"], "<html><body></body></html>")

    monkeypatch.setattr(core, "fetch_raw", fake)
    seeds = [f"https://site.com/s{i}" for i in range(12)]
    # concurrency: 100 requested, but CFG.max_bulk_concurrency=3 is the ceiling.
    out = asyncio.run(crawl.run_crawl(CFG, None, {"start_urls": seeds, "concurrency": 100}))
    assert out["stats"]["crawled"] == 12
    assert peak["max"] <= 3  # cap enforced (would be up to 12 unclamped)


def test_crawl_tolerates_null_numeric_fields(monkeypatch):
    _patch(monkeypatch)
    out = asyncio.run(
        crawl.run_crawl(
            CFG,
            None,
            {"start_urls": ["https://site.com/"], "max_pages": None, "max_depth": None, "concurrency": None},
        )
    )
    assert out["stats"]["crawled"] >= 1  # did not crash on int(None)


def test_crawl_error_rows_included_in_sample(monkeypatch):
    _patch(monkeypatch, raise_on="/b")
    out = asyncio.run(
        crawl.run_crawl(CFG, None, {"start_urls": ["https://site.com/"], "selectors": SEL, "max_depth": 2})
    )
    errs = [i for i in out["items"] if i.get("error")]
    assert any(i["url"].endswith("/b") for i in errs)


def test_crawl_include_html_returns_html(monkeypatch):
    _patch(monkeypatch)
    out = asyncio.run(
        crawl.run_crawl(CFG, None, {"start_urls": ["https://site.com/"], "include_html": True, "max_depth": 0})
    )
    assert "<h1>Home</h1>" in out["items"][0]["html"]


def test_crawl_one_bad_page_does_not_abort_crawl(monkeypatch):
    # A selector/parse failure on one page must be captured per-URL, not sink the run.
    def fake(cfg, payload, tier):
        return FakePage(payload["url"], GRAPH[payload["url"]])

    monkeypatch.setattr(core, "fetch_raw", fake)

    real_serialize = core.serialize_page

    def flaky_serialize(page, payload, include_html):
        if page.url.endswith("/b"):
            raise ValueError("bad selector on this page")
        return real_serialize(page, payload, include_html)

    monkeypatch.setattr(core, "serialize_page", flaky_serialize)
    out = asyncio.run(
        crawl.run_crawl(CFG, None, {"start_urls": ["https://site.com/"], "selectors": SEL, "max_depth": 2})
    )
    assert out["stats"]["crawled"] == 4  # crawl completed despite /b raising
    assert out["stats"]["errors"] == 1
