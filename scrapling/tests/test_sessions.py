"""Persistent sessions: registry lifecycle, per-session thread affinity, reuse,
cap, and teardown — with the Scrapling session classes faked (no network/browser)."""

from __future__ import annotations

import threading

import pytest

from src import core, sessions

MAIN_THREAD = threading.get_ident()
HTML = "<html><body><h1>Hi</h1></body></html>"


class FakePage:
    def __init__(self, url):
        self._sel = core.parse_html(HTML)
        self.status = 200
        self.url = url
        self.headers = {"content-type": "text/html"}
        self.cookies = {"sid": "1"}
        self.encoding = "utf-8"
        self.html_content = self._sel.html_content

    def css(self, *a, **k):
        return self._sel.css(*a, **k)

    def xpath(self, *a, **k):
        return self._sel.xpath(*a, **k)

    def get_all_text(self, *a, **k):
        return self._sel.get_all_text(*a, **k)


class FakeHttpSession:
    """Duck-types FetcherSession: __enter__ returns a logic object with get/…"""

    def __init__(self, **cfg):
        self.cfg = cfg
        self.entered = False
        self.closed = False
        self.calls = 0
        self.construct_tid = threading.get_ident()

    def __enter__(self):
        self.entered = True
        return self  # logic object == self (has get/post/put/delete)

    def __exit__(self, *_a):
        self.closed = True
        self.close_tid = threading.get_ident()

    def get(self, url, **kw):
        self.calls += 1
        self.fetch_tid = threading.get_ident()
        return FakePage(url)

    post = put = delete = get


@pytest.fixture
def patch_http(monkeypatch):
    holder = {}

    def factory(**cfg):
        holder["session"] = FakeHttpSession(**cfg)
        return holder["session"]

    monkeypatch.setattr("scrapling.fetchers.FetcherSession", factory)
    return holder


def test_open_fetch_reuse_close(patch_http):
    reg = sessions.Registry(max_sessions=4)
    opened = reg.open("http", {"impersonate": "chrome"})
    sid = opened["session_id"]
    assert opened["type"] == "http"

    r1 = reg.fetch(sid, {"url": "https://a.com"})
    r2 = reg.fetch(sid, {"url": "https://b.com", "selectors": [{"name": "h", "css": "h1"}]})
    assert r1["status"] == 200
    assert r2["extracted"] == {"h": "Hi"}

    session = patch_http["session"]
    assert session.calls == 2  # same live session reused across both fetches
    assert reg.close(sid) == {"closed": True}
    assert session.closed is True
    assert reg.close(sid) == {"closed": False}  # idempotent


def test_all_ops_run_on_one_dedicated_thread(patch_http):
    reg = sessions.Registry()
    sid = reg.open("http", {})["session_id"]
    reg.fetch(sid, {"url": "https://a.com"})
    reg.close(sid)
    s = patch_http["session"]
    # construct, fetch, teardown all on the SAME thread — and NOT the caller's.
    assert s.construct_tid == s.fetch_tid == s.close_tid
    assert s.construct_tid != MAIN_THREAD


def test_max_sessions_enforced(patch_http):
    reg = sessions.Registry(max_sessions=1)
    reg.open("http", {})
    with pytest.raises(ValueError, match="session limit"):
        reg.open("http", {})


def test_fetch_unknown_session_raises(patch_http):
    reg = sessions.Registry()
    with pytest.raises(ValueError, match="unknown session"):
        reg.fetch("nope", {"url": "https://a.com"})


def test_unknown_type_rejected():
    reg = sessions.Registry()
    with pytest.raises(ValueError, match="unknown session type"):
        reg.open("carrier-pigeon", {})


def test_list_and_close_all(patch_http):
    reg = sessions.Registry()
    a = reg.open("http", {})["session_id"]
    b = reg.open("http", {})["session_id"]
    listed = reg.list()["sessions"]
    assert {s["session_id"] for s in listed} == {a, b}
    assert all(s["type"] == "http" and "idle_s" in s for s in listed)
    reg.close_all()
    assert reg.list()["sessions"] == []
