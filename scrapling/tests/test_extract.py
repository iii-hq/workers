"""Pure parse logic against static HTML — no network, no browsers."""

from __future__ import annotations

from src import core

HTML = (
    "<html><body>"
    '<h1 class="t">Hello</h1>'
    '<ul><li><a href="/a">Apple</a></li><li><a href="/b">Banana</a></li></ul>'
    "<p>price 42 usd then 99</p>"
    "</body></html>"
)


def test_css_all_text():
    out = core.op_query({"html": HTML, "query": "li a"}, "css")
    assert out["result"] == ["Apple", "Banana"]


def test_css_first_text():
    out = core.op_query({"html": HTML, "query": "li a", "first": True}, "css")
    assert out["result"] == "Apple"


def test_css_first_attr():
    out = core.op_query({"html": HTML, "query": "li a", "first": True, "attr": "href"}, "css")
    assert out["result"] == "/a"


def test_css_no_match_returns_empty_list_or_none():
    assert core.op_query({"html": HTML, "query": ".nope"}, "css")["result"] == []
    assert core.op_query({"html": HTML, "query": ".nope", "first": True}, "css")["result"] is None


def test_xpath_first():
    out = core.op_query({"html": HTML, "query": "//h1", "first": True}, "xpath")
    assert out["result"] == "Hello"


def test_regex_first_and_all():
    assert core.op_regex({"html": HTML, "pattern": r"price (\d+)", "first": True})["result"] == "42"
    assert core.op_regex({"html": HTML, "pattern": r"\d+"})["result"] == ["42", "99"]


def test_extract_declarative_mix():
    out = core.op_extract(
        {
            "html": HTML,
            "selectors": [
                {"name": "title", "css": "h1", "all": False},
                {"name": "links", "css": "li a", "attr": "href", "all": True},
                {"name": "names", "css": "li a", "all": True},
                {"name": "price", "regex": r"price (\d+)"},
                {"name": "first_li_html", "css": "li", "html": True},
            ],
        }
    )
    e = out["extracted"]
    assert e["title"] == "Hello"
    assert e["links"] == ["/a", "/b"]
    assert e["names"] == ["Apple", "Banana"]
    assert e["price"] == "42"
    assert "<a href=" in e["first_li_html"]


def test_extract_missing_query_is_null_or_empty():
    out = core.op_extract({"html": HTML, "selectors": [{"name": "x"}, {"name": "y", "all": True}]})
    assert out["extracted"] == {"x": None, "y": []}


def test_find_similar_includes_anchor_plus_similar():
    out = core.op_find_similar({"html": HTML, "anchor": "li"})
    texts = [item["text"] for item in out["items"]]
    assert out["count"] == 2
    assert texts == ["Apple", "Banana"]


def test_find_similar_with_subselectors():
    out = core.op_find_similar(
        {"html": HTML, "anchor": "li", "selectors": [{"name": "href", "css": "a", "attr": "href"}]}
    )
    assert [i["href"] for i in out["items"]] == ["/a", "/b"]


def test_find_similar_no_anchor():
    out = core.op_find_similar({"html": HTML, "anchor": ".nope"})
    assert out == {"count": 0, "items": []}


# ---- Phase 1: element search + introspection + markdown --------------------


def test_find_by_tag():
    out = core.op_find({"html": HTML, "tag": "a"})
    assert out["count"] == 2
    assert [i["text"] for i in out["items"]] == ["Apple", "Banana"]
    assert out["items"][0]["attrs"]["href"] == "/a"
    assert out["items"][0]["css"].endswith("a")


def test_find_by_attrs():
    out = core.op_find({"html": HTML, "attrs": {"class": "t"}})
    assert out["count"] == 1
    assert out["items"][0]["text"] == "Hello"


def test_find_text_regex_filter_and_first():
    out = core.op_find({"html": HTML, "tag": "a", "text_regex": "Ap", "first": True})
    assert out["count"] == 1
    assert out["items"][0]["text"] == "Apple"


def test_find_requires_a_filter():
    import pytest

    with pytest.raises(ValueError):
        core.op_find({"html": HTML})


def test_find_limit_zero_returns_no_items_but_full_count():
    out = core.op_find({"html": HTML, "tag": "a", "limit": 0})
    assert out["items"] == []
    assert out["count"] == 2  # count still reports the true total matched


def test_find_negative_limit_clamped_to_zero_not_slice_from_end():
    out = core.op_find({"html": HTML, "tag": "a", "limit": -1})
    assert out["items"] == []  # NOT items[:-1] (which would drop just the last)


def test_find_by_text_exact_and_partial():
    assert core.op_find_by_text({"html": HTML, "text": "Apple"})["count"] == 1
    partial = core.op_find_by_text({"html": HTML, "text": "Ap", "partial": True})
    assert partial["count"] >= 1
    assert core.op_find_by_text({"html": HTML, "text": "Zebra"})["count"] == 0


def test_find_by_regex():
    out = core.op_find_by_regex({"html": HTML, "pattern": r"price \d+"})
    assert out["count"] == 1
    assert "price 42" in out["items"][0]["text"]


def test_describe_matched_element():
    out = core.op_describe({"html": HTML, "query": "h1"})
    assert out["found"] is True
    el = out["element"]
    assert el["tag"] == "h1"
    assert el["classes"] == ["t"]
    assert el["css"] and el["full_css"] and el["xpath"]
    assert el["parent_tag"] == "body"


def test_describe_no_match():
    assert core.op_describe({"html": HTML, "query": ".nope"}) == {"found": False}


def test_to_markdown_and_text():
    md = core.op_to_markdown({"html": HTML, "format": "markdown"})
    assert md["format"] == "markdown"
    assert "Hello" in md["content"]
    txt = core.op_to_markdown({"html": HTML, "format": "text"})
    assert "Apple" in txt["content"] and "<" not in txt["content"]


def test_to_markdown_rejects_unknown_format():
    import pytest

    with pytest.raises(ValueError):
        core.op_to_markdown({"html": HTML, "format": "pdf"})
