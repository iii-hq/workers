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
