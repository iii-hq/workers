"""Screenshot tiling helper + content-block return shape (no real browser)."""

from __future__ import annotations

import asyncio
import base64
import io

from PIL import Image

from src import core
from src.handlers import create_handlers

CFG = {"defaults": {"headless": True, "network_idle": False, "proxy": "", "include_html": False}}


def _png(width: int, height: int) -> bytes:
    buf = io.BytesIO()
    Image.new("RGB", (width, height), "white").save(buf, format="PNG")
    return buf.getvalue()


def _size(data: bytes) -> tuple[int, int]:
    return Image.open(io.BytesIO(data)).size


def test_tile_small_image_untouched():
    tiles, w, h, truncated = core.tile_screenshot(_png(800, 600), "png")
    assert (w, h) == (800, 600)
    assert len(tiles) == 1
    assert _size(tiles[0]) == (800, 600)
    assert truncated is False


def test_tile_downscales_wide_image():
    tiles, w, h, _ = core.tile_screenshot(_png(2048, 1000), "png")
    assert (w, h) == (1024, 500)
    assert _size(tiles[0]) == (1024, 500)


def test_tile_splits_tall_image():
    tiles, w, h, truncated = core.tile_screenshot(_png(1000, 4000), "png")
    assert (w, h) == (1000, 4000)
    assert [_size(t)[1] for t in tiles] == [1536, 1536, 928]
    assert truncated is False


def test_tile_caps_at_six_and_flags_truncation():
    tiles, _, _, truncated = core.tile_screenshot(_png(100, 20000), "png")
    assert len(tiles) == 6
    assert truncated is True


def test_tile_keeps_jpeg_format():
    src = io.BytesIO()
    Image.new("RGB", (200, 100), "white").save(src, format="JPEG")
    tiles, _, _, _ = core.tile_screenshot(src.getvalue(), "jpeg")
    assert Image.open(io.BytesIO(tiles[0])).format == "JPEG"


class FakeShotPage:
    url = "https://example.com/final"

    def __init__(self, data: bytes):
        self._data = data
        self.shot_kwargs: dict = {}

    def screenshot(self, **kwargs) -> bytes:
        self.shot_kwargs = kwargs
        return self._data


def _patch_fetch(monkeypatch, page: FakeShotPage):
    def fake_fetch(url, **kwargs):
        kwargs["page_action"](page)

    monkeypatch.setattr("scrapling.fetchers.DynamicFetcher.fetch", staticmethod(fake_fetch))


def test_screenshot_returns_content_blocks(monkeypatch):
    page = FakeShotPage(_png(320, 200))
    _patch_fetch(monkeypatch, page)
    res = core.do_screenshot(CFG, {"url": "https://example.com"})
    assert "image_base64" not in res
    assert res["mime"] == "image/png"
    assert res["url"] == "https://example.com/final"
    blocks = res["content"]
    assert [b["type"] for b in blocks] == ["image", "text"]
    assert blocks[0]["mime"] == "image/png"
    assert _size(base64.b64decode(blocks[0]["data"])) == (320, 200)
    assert "screenshot of https://example.com/final — 320x200px, 1 tile(s)," in blocks[-1]["text"]
    assert page.shot_kwargs == {"type": "png", "full_page": False}


def test_screenshot_tall_page_multi_tile_caption(monkeypatch):
    _patch_fetch(monkeypatch, FakeShotPage(_png(100, 20000)))
    res = core.do_screenshot(CFG, {"url": "https://example.com", "full_page": True})
    images = [b for b in res["content"] if b["type"] == "image"]
    assert len(images) == 6
    assert "6 tile(s)" in res["content"][-1]["text"]
    assert "truncated" in res["content"][-1]["text"]


def test_screenshot_handler_passthrough(monkeypatch):
    _patch_fetch(monkeypatch, FakeShotPage(_png(50, 50)))
    h = create_handlers(lambda: CFG)
    res = asyncio.run(h["screenshot"]({"url": "https://example.com"}))
    assert res["content"][0]["type"] == "image"
