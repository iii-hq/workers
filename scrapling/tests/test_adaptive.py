"""Adaptive Smart Element Tracking: save an element identity, then relocate it
after the page's CSS path changes. Uses a temp DB — no network, no browsers."""

from __future__ import annotations

from src import core, storage

DOMAIN = "http://shop.example"
V1 = '<html><body><div class="price-box"><span class="amount">$42</span></div></body></html>'
# Same element, redesigned markup: the old `span.amount` selector no longer hits.
V2 = '<html><body><section class="pricing"><span class="new-amount">$42</span></section></body></html>'


def _use_temp_db(tmp_path):
    storage.configure(str(tmp_path / "elements.db"))


def test_storage_configure_creates_parent(tmp_path):
    path = storage.configure(str(tmp_path / "nested" / "dir" / "el.db"))
    assert path.endswith("el.db")
    assert (tmp_path / "nested" / "dir").is_dir()


def test_css_adaptive_relocates_after_layout_change(tmp_path):
    _use_temp_db(tmp_path)
    # First run: the selector matches and the identity is saved.
    first = core.op_query(
        {"html": V1, "query": "span.amount", "identifier": "price", "adaptive": True, "adaptive_domain": DOMAIN},
        "css",
    )
    assert first["result"] == ["$42"]

    # Plain (non-adaptive) query on the redesigned page misses.
    plain = core.op_query({"html": V2, "query": "span.amount"}, "css")
    assert plain["result"] == []

    # Adaptive relocation finds the moved element via the saved identity.
    relocated = core.op_query(
        {"html": V2, "query": "span.amount", "identifier": "price", "adaptive": True, "adaptive_domain": DOMAIN},
        "css",
    )
    assert relocated["result"] == ["$42"]


def test_extract_adaptive_relocates_by_selector_name(tmp_path):
    _use_temp_db(tmp_path)
    sel = [{"name": "price", "css": "span.amount"}]
    core.op_extract({"html": V1, "selectors": sel, "adaptive": True, "adaptive_domain": DOMAIN})

    out = core.op_extract({"html": V2, "selectors": sel, "adaptive": True, "adaptive_domain": DOMAIN})
    assert out["extracted"]["price"] == "$42"


def test_non_adaptive_extract_unaffected(tmp_path):
    _use_temp_db(tmp_path)
    out = core.op_extract({"html": V1, "selectors": [{"name": "price", "css": "span.amount"}]})
    assert out["extracted"]["price"] == "$42"
