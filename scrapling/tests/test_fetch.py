"""Fetch handlers with the Scrapling HTTP fetcher monkeypatched.

Exercises kwarg pass-through, config-default application, serialization, and the
bulk `urls` fan-out (incl. per-url error capture) — no real network.
"""

from __future__ import annotations

import asyncio

import pytest

from src import core
from src.handlers import create_handlers

CFG = {
    "defaults": {
        "impersonate": "chrome",
        "headless": True,
        "network_idle": False,
        "proxy": "",
        "include_html": False,
    },
    "max_bulk_concurrency": 5,
}

HTML = '<html><body><h1>Hi</h1><a href="/x">X</a></body></html>'


class FakePage:
    """Duck-types the bits of a Scrapling Response that serialize_page touches."""

    def __init__(self, html: str, **meta):
        self._sel = core.parse_html(html)
        self.status = meta.get("status", 200)
        self.url = meta.get("url", "https://example.com")
        self.headers = meta.get("headers", {"content-type": "text/html"})
        self.cookies = meta.get("cookies", {"sid": "1"})
        self.encoding = meta.get("encoding", "utf-8")
        self.html_content = self._sel.html_content

    def css(self, *a, **k):
        return self._sel.css(*a, **k)

    def xpath(self, *a, **k):
        return self._sel.xpath(*a, **k)

    def get_all_text(self, *a, **k):
        return self._sel.get_all_text(*a, **k)


def _patch_get(monkeypatch, sink: dict | None = None, *, raise_on: str | None = None):
    def fake_get(url, **kwargs):
        if sink is not None:
            sink["url"] = url
            sink["kwargs"] = kwargs
        if raise_on and raise_on in url:
            raise RuntimeError("boom")
        return FakePage(HTML, url=url)

    monkeypatch.setattr("scrapling.fetchers.Fetcher.get", staticmethod(fake_get))


def _h(monkeypatch, **patch):
    _patch_get(monkeypatch, **patch)
    return create_handlers(lambda: CFG)


def test_fetch_serializes_response(monkeypatch):
    h = _h(monkeypatch)
    res = asyncio.run(h["fetch"]({"url": "https://example.com"}))
    assert res["status"] == 200
    assert res["headers"]["content-type"] == "text/html"
    assert res["cookies"]["sid"] == "1"
    assert res["encoding"] == "utf-8"
    assert "html" not in res  # include_html default False


def test_fetch_applies_config_default_impersonate(monkeypatch):
    sink: dict = {}
    _patch_get(monkeypatch, sink)
    h = create_handlers(lambda: CFG)
    asyncio.run(h["fetch"]({"url": "https://example.com"}))
    assert sink["kwargs"]["impersonate"] == "chrome"
    assert "proxy" not in sink["kwargs"]  # empty default not forwarded


def test_fetch_payload_overrides_default_and_forwards_allowed(monkeypatch):
    sink: dict = {}
    _patch_get(monkeypatch, sink)
    h = create_handlers(lambda: CFG)
    asyncio.run(h["fetch"]({"url": "https://e.com", "impersonate": "firefox", "headers": {"a": "b"}, "timeout": 5}))
    assert sink["kwargs"]["impersonate"] == "firefox"
    assert sink["kwargs"]["headers"] == {"a": "b"}
    assert sink["kwargs"]["timeout"] == 5


def test_fetch_inline_extract(monkeypatch):
    h = _h(monkeypatch)
    res = asyncio.run(h["fetch"]({"url": "https://e.com", "selectors": [{"name": "h", "css": "h1"}]}))
    assert res["extracted"] == {"h": "Hi"}


def test_fetch_include_html(monkeypatch):
    h = _h(monkeypatch)
    res = asyncio.run(h["fetch"]({"url": "https://e.com", "include_html": True}))
    assert "<h1>Hi</h1>" in res["html"]


def test_fetch_requires_url_or_urls(monkeypatch):
    h = _h(monkeypatch)
    with pytest.raises(ValueError):
        asyncio.run(h["fetch"]({}))


def test_bulk_returns_results_array(monkeypatch):
    h = _h(monkeypatch)
    res = asyncio.run(h["fetch"]({"urls": ["https://a.com", "https://b.com"]}))
    assert len(res["results"]) == 2
    assert all(r["status"] == 200 for r in res["results"])


def test_bulk_captures_per_url_error(monkeypatch):
    _patch_get(monkeypatch, raise_on="bad")
    h = create_handlers(lambda: CFG)
    res = asyncio.run(h["fetch"]({"urls": ["https://ok.com", "https://bad.com"]}))
    errs = {r["url"]: r.get("error") for r in res["results"]}
    assert errs["https://bad.com"] == "boom"
    assert errs.get("https://ok.com") is None


def test_unsupported_method_raises(monkeypatch):
    h = _h(monkeypatch)
    with pytest.raises(ValueError):
        asyncio.run(h["fetch"]({"url": "https://e.com", "method": "patch"}))


def test_fetch_forwards_widened_http_params(monkeypatch):
    sink: dict = {}
    _patch_get(monkeypatch, sink)
    h = create_handlers(lambda: CFG)
    asyncio.run(
        h["fetch"](
            {
                "url": "https://e.com",
                "proxies": {"https": "http://p:8080"},
                "proxy_auth": ["user", "pw"],
                "http3": True,
                "max_redirects": 2,
            }
        )
    )
    kw = sink["kwargs"]
    assert kw["proxies"] == {"https": "http://p:8080"}
    assert kw["proxy_auth"] == ["user", "pw"]
    assert kw["http3"] is True
    assert kw["max_redirects"] == 2


def test_fetch_format_markdown_renders_content(monkeypatch):
    h = _h(monkeypatch)
    res = asyncio.run(h["fetch"]({"url": "https://e.com", "format": "markdown"}))
    assert res["format"] == "markdown"
    assert "Hi" in res["content"]  # the FakePage HTML has <h1>Hi</h1>
    assert "html" not in res  # format does not imply raw html


def test_fetch_format_css_selector_scopes_render(monkeypatch):
    h = _h(monkeypatch)
    res = asyncio.run(
        h["fetch"]({"url": "https://e.com", "format": "markdown", "css_selector": "h1"})
    )
    assert "Hi" in res["content"]
    assert "X" not in res["content"]  # content outside the scoped subtree is dropped
