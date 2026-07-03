"""Pure, synchronous Scrapling wrappers — no iii bus, no asyncio.

Everything here is import-safe without the browsers: the `scrapling.fetchers`
imports live inside the fetch functions, so parsing (`parse_html`,
`apply_selectors`) and the register surface work with only the base `scrapling`
package installed (browsers are fetched by `scrapling install` in the image).

The `handlers` module wraps these in `asyncio.to_thread` and adds bulk/config.
"""

from __future__ import annotations

import base64
import re
from typing import Any

# Cap element-search results so a broad filter can't return a giant payload.
_MAX_FIND_ITEMS = 100

# JSON-safe, useful kwargs forwarded to each Scrapling fetcher. Deliberately
# excludes callbacks (page_action/page_setup), host paths (executable_path,
# user_data_dir), and non-JSON objects (proxy_rotator, cert, selector_config).
_HTTP_GET_KEYS = frozenset(
    {
        "headers",
        "params",
        "cookies",
        "proxy",
        "proxies",
        "proxy_auth",
        "impersonate",
        "timeout",
        "follow_redirects",
        "max_redirects",
        "stealthy_headers",
        "retries",
        "retry_delay",
        "http3",
        "verify",
        "auth",
    }
)
_HTTP_DATA_KEYS = _HTTP_GET_KEYS | {"data", "json"}
_BROWSER_COMMON = frozenset(
    {
        "headless",
        "network_idle",
        "load_dom",
        "timeout",
        "wait",
        "wait_selector",
        "wait_selector_state",
        "disable_resources",
        "proxy",
        "useragent",
        "cookies",
        "google_search",
        "block_ads",
        "blocked_domains",
        "real_chrome",
        "cdp_url",
        "capture_xhr",
        "locale",
        "timezone_id",
        "extra_headers",
        "dns_over_https",
        "retries",
        "retry_delay",
        "extra_flags",
        "additional_args",
        "max_pages",
    }
)
_STEALTHY_KEYS = _BROWSER_COMMON | {
    "solve_cloudflare",
    "block_webrtc",
    "hide_canvas",
    "allow_webgl",
}
_DYNAMIC_KEYS = _BROWSER_COMMON


# ---- parsing --------------------------------------------------------------


def parse_html(html: str, *, adaptive: bool = False, domain: str | None = None):
    from scrapling import Selector

    if adaptive:
        from . import storage

        # Smart Element Tracking: persist/relocate element identities keyed by
        # (registrable-domain, identifier). `url` supplies the domain key.
        return Selector(
            content=html,
            adaptive=True,
            storage_args={"storage_file": storage.db_path(), "url": domain or ""},
        )
    return Selector(content=html)


def _query_one(sel, query: str, is_css: bool, adaptive: bool, auto_save: bool, identifier: str):
    """Run a css/xpath query, threading adaptive relocation kwargs when enabled."""
    if adaptive:
        kw = {"identifier": identifier, "adaptive": True, "auto_save": auto_save}
        return sel.css(query, **kw) if is_css else sel.xpath(query, **kw)
    return sel.css(query) if is_css else sel.xpath(query)


def _pull(node, spec: dict[str, Any]) -> Any:
    attr = spec.get("attr")
    if attr:
        return node.attrib.get(attr)
    if spec.get("html"):
        return str(node.html_content)
    return str(node.get_all_text())


def apply_selectors(
    sel, selectors: list[dict[str, Any]], *, adaptive: bool = False, auto_save: bool = True
) -> dict[str, Any]:
    """Run a declarative selector list against a Selector/Response.

    Each spec: {name, css|xpath|regex, attr?, html?, all?}. `all` returns a list
    over every match; otherwise the first match's value (or None). When
    `adaptive`, css/xpath matches are saved/relocated under `identifier=name`
    (requires `sel` built with `parse_html(..., adaptive=True)`).
    """
    out: dict[str, Any] = {}
    for spec in selectors or []:
        name = spec["name"]
        want_all = bool(spec.get("all"))
        if spec.get("regex"):
            text = sel.get_all_text()
            out[name] = (
                [str(m) for m in text.re(spec["regex"])] if want_all else (_none_or_str(text.re_first(spec["regex"])))
            )
            continue
        query = spec.get("css") or spec.get("xpath")
        if not query:
            out[name] = [] if want_all else None
            continue
        nodes = _query_one(sel, query, bool(spec.get("css")), adaptive, auto_save, name)
        if want_all:
            out[name] = [_pull(n, spec) for n in nodes]
        else:
            first = nodes.first
            out[name] = _pull(first, spec) if first is not None else None
    return out


def _none_or_str(v: Any) -> Any:
    return None if v is None else str(v)


# ---- serialization --------------------------------------------------------


def _as_dict(v: Any) -> dict[str, Any]:
    if not v:
        return {}
    if isinstance(v, dict):
        return v
    try:
        return dict(v)
    except (TypeError, ValueError):
        return {}


def render_content(page, fmt: str, main_content_only: bool = False, css_selector: str | None = None) -> str:
    """HTML → markdown / text / html via Scrapling's `Convertor`. Accepts any
    `Selector`/`Response` (or a duck-typed page with `.html_content`)."""
    from scrapling import Selector
    from scrapling.core.shell import Convertor

    sel = page if isinstance(page, Selector) else parse_html(str(page.html_content))
    return "".join(
        Convertor._extract_content(
            sel, extraction_type=fmt, css_selector=css_selector, main_content_only=main_content_only
        )
    ).strip()


def serialize_page(page, payload: dict[str, Any], include_html: bool) -> dict[str, Any]:
    out: dict[str, Any] = {
        "status": getattr(page, "status", None),
        "url": str(getattr(page, "url", "") or ""),
        "headers": _as_dict(getattr(page, "headers", None)),
        "cookies": _as_dict(getattr(page, "cookies", None)),
        "encoding": getattr(page, "encoding", None),
    }
    selectors = payload.get("selectors")
    if selectors:
        out["extracted"] = apply_selectors(page, selectors)
    captured = getattr(page, "captured_xhr", None)
    if captured:
        out["captured_xhr"] = captured
    fmt = payload.get("format")
    if fmt in ("markdown", "text"):
        out["format"] = fmt
        out["content"] = render_content(page, fmt, bool(payload.get("main_content_only", False)))
    if include_html:
        out["html"] = str(page.html_content)
    return out


# ---- fetchers -------------------------------------------------------------


def _pick(payload: dict[str, Any], allowed: frozenset[str]) -> dict[str, Any]:
    return {k: v for k, v in payload.items() if k in allowed and v is not None}


def _default(kwargs: dict[str, Any], cfg: dict[str, Any], key: str) -> None:
    if key not in kwargs:
        val = cfg.get("defaults", {}).get(key)
        if val not in (None, ""):
            kwargs[key] = val


def fetch_raw(cfg: dict[str, Any], payload: dict[str, Any], tier: str = "http"):
    """Fetch a URL with the chosen tier and return the RAW Scrapling page (so the
    caller can both serialize it and follow its links). `tier` ∈ http|stealthy|dynamic."""
    if tier == "http":
        from scrapling.fetchers import Fetcher

        method = (payload.get("method") or "get").lower()
        if method not in ("get", "post", "put", "delete"):
            raise ValueError(f"unsupported method: {method}")
        keys = _HTTP_GET_KEYS if method == "get" else _HTTP_DATA_KEYS
        kwargs = _pick(payload, keys)
        _default(kwargs, cfg, "impersonate")
        _default(kwargs, cfg, "proxy")
        return getattr(Fetcher, method)(payload["url"], **kwargs)
    if tier == "stealthy":
        from scrapling.fetchers import StealthyFetcher

        return StealthyFetcher.fetch(payload["url"], **_browser_kwargs(cfg, payload, _STEALTHY_KEYS))
    if tier == "dynamic":
        from scrapling.fetchers import DynamicFetcher

        return DynamicFetcher.fetch(payload["url"], **_browser_kwargs(cfg, payload, _DYNAMIC_KEYS))
    raise ValueError(f"unknown fetcher tier: {tier!r} (use http|stealthy|dynamic)")


def extract_links(page) -> list[str]:
    """Absolute hrefs of every <a> on the page (resolved against the page URL)."""
    out: list[str] = []
    for a in page.css("a"):
        href = a.attrib.get("href")
        if not href:
            continue
        try:
            out.append(str(page.urljoin(href)))
        except Exception:  # noqa: BLE001 - skip un-joinable hrefs (mailto:, javascript:, …)
            continue
    return out


def do_fetch(cfg: dict[str, Any], payload: dict[str, Any]) -> dict[str, Any]:
    return serialize_page(fetch_raw(cfg, payload, "http"), payload, _include_html(cfg, payload))


def do_stealthy(cfg: dict[str, Any], payload: dict[str, Any]) -> dict[str, Any]:
    return serialize_page(fetch_raw(cfg, payload, "stealthy"), payload, _include_html(cfg, payload))


def do_dynamic(cfg: dict[str, Any], payload: dict[str, Any]) -> dict[str, Any]:
    return serialize_page(fetch_raw(cfg, payload, "dynamic"), payload, _include_html(cfg, payload))


def do_screenshot(cfg: dict[str, Any], payload: dict[str, Any]) -> dict[str, Any]:
    fetcher = payload.get("fetcher", "dynamic")
    image_type = payload.get("format", "png")
    captured: dict[str, Any] = {}

    def _capture(page: Any) -> None:
        shot = {"type": image_type, "full_page": bool(payload.get("full_page", False))}
        captured["bytes"] = page.screenshot(**shot)
        captured["url"] = page.url

    if fetcher == "stealthy":
        from scrapling.fetchers import StealthyFetcher as F

        kwargs = _browser_kwargs(cfg, payload, _STEALTHY_KEYS)
    else:
        from scrapling.fetchers import DynamicFetcher as F

        kwargs = _browser_kwargs(cfg, payload, _DYNAMIC_KEYS)
    kwargs["page_action"] = _capture
    F.fetch(payload["url"], **kwargs)

    if "bytes" not in captured:
        raise RuntimeError("screenshot capture failed")
    return {
        "image_base64": base64.b64encode(captured["bytes"]).decode("ascii"),
        "mime": f"image/{image_type}",
        "url": captured.get("url", payload["url"]),
    }


def _browser_kwargs(cfg: dict[str, Any], payload: dict[str, Any], allowed: frozenset[str]) -> dict[str, Any]:
    kwargs = _pick(payload, allowed)
    _default(kwargs, cfg, "headless")
    _default(kwargs, cfg, "network_idle")
    _default(kwargs, cfg, "proxy")
    return kwargs


def _include_html(cfg: dict[str, Any], payload: dict[str, Any]) -> bool:
    v = payload.get("include_html")
    if v is None:
        return bool(cfg.get("defaults", {}).get("include_html", False))
    return bool(v)


# ---- parse-only ops (no network) ------------------------------------------


def op_extract(payload: dict[str, Any]) -> dict[str, Any]:
    adaptive = bool(payload.get("adaptive"))
    sel = parse_html(payload["html"], adaptive=adaptive, domain=payload.get("adaptive_domain"))
    return {
        "extracted": apply_selectors(
            sel,
            payload.get("selectors") or [],
            adaptive=adaptive,
            auto_save=bool(payload.get("auto_save", adaptive)),
        )
    }


def op_query(payload: dict[str, Any], kind: str) -> dict[str, Any]:
    adaptive = bool(payload.get("adaptive"))
    sel = parse_html(payload["html"], adaptive=adaptive, domain=payload.get("adaptive_domain"))
    query = payload["query"]
    nodes = _query_one(
        sel,
        query,
        kind == "css",
        adaptive,
        bool(payload.get("auto_save", True)),
        payload.get("identifier") or query,
    )
    attr = payload.get("attr")
    spec = {"attr": attr} if attr else {}
    if payload.get("first"):
        first = nodes.first
        return {"result": _pull(first, spec) if first is not None else None}
    return {"result": [_pull(n, spec) for n in nodes]}


def op_regex(payload: dict[str, Any]) -> dict[str, Any]:
    text = parse_html(payload["html"]).get_all_text()
    pattern = payload["pattern"]
    if payload.get("first"):
        return {"result": _none_or_str(text.re_first(pattern))}
    return {"result": [str(m) for m in text.re(pattern)]}


def op_find_similar(payload: dict[str, Any]) -> dict[str, Any]:
    sel = parse_html(payload["html"])
    anchor = sel.css(payload["anchor"]).first
    if anchor is None:
        return {"count": 0, "items": []}
    similar = anchor.find_similar(
        similarity_threshold=payload.get("similarity_threshold", 0.2),
        match_text=bool(payload.get("match_text", False)),
    )
    elements = [anchor, *list(similar)]
    sub = payload.get("selectors")
    items = [apply_selectors(el, sub) if sub else _default_item(el) for el in elements]
    return {"count": len(items), "items": items}


def _default_item(el) -> dict[str, Any]:
    return {"text": str(el.get_all_text()), "html": str(el.html_content)}


# ---- element search & introspection (no network) --------------------------


def _attrs(el) -> dict[str, str]:
    return {str(k): str(v) for k, v in dict(el.attrib).items()}


def serialize_element(el) -> dict[str, Any]:
    """Compact, JSON-safe view of a matched element: text, HTML, attributes, and
    the auto-generated CSS/XPath selectors that locate it (Scrapling's
    `generate_*_selector`)."""
    return {
        "tag": el.tag,
        "text": str(el.get_all_text(strip=True)),
        "html": str(el.html_content),
        "attrs": _attrs(el),
        "css": str(el.generate_css_selector),
        "xpath": str(el.generate_xpath_selector),
    }


def _as_element_list(result) -> list:
    """`find_by_text`/`find_by_regex` return a single `Selector` (first_match) or
    a `Selectors` list; normalize both (and the empty case) to a plain list."""
    if isinstance(result, list):  # Selectors is a list subclass
        return list(result)
    return [result] if result is not None else []


def _bounded(items: list, payload: dict[str, Any]) -> list:
    if payload.get("first"):
        return items[:1]
    limit = payload.get("limit")
    ceiling = min(int(limit), _MAX_FIND_ITEMS) if limit else _MAX_FIND_ITEMS
    return items[:ceiling]


def op_find(payload: dict[str, Any]) -> dict[str, Any]:
    """BeautifulSoup-style search: filter by tag(s) and/or attributes, optionally
    keeping only elements whose text matches `text_regex`. Declarative forms only
    (no callables)."""
    sel = parse_html(payload["html"])
    args: list[Any] = []
    tag = payload.get("tag")
    if tag:
        args.append(list(tag) if isinstance(tag, list) else tag)
    attrs = payload.get("attrs")
    if attrs:
        args.append({str(k): str(v) for k, v in attrs.items()})
    text_regex = payload.get("text_regex")
    if text_regex:
        args.append(re.compile(text_regex))
    if not args:
        raise ValueError("provide at least one of `tag`, `attrs`, `text_regex`")
    matches = list(sel.find_all(*args))
    items = [serialize_element(el) for el in _bounded(matches, payload)]
    return {"count": len(matches), "items": items}


def op_find_by_text(payload: dict[str, Any]) -> dict[str, Any]:
    sel = parse_html(payload["html"])
    result = sel.find_by_text(
        payload["text"],
        first_match=False,
        partial=bool(payload.get("partial", False)),
        case_sensitive=bool(payload.get("case_sensitive", False)),
        clean_match=bool(payload.get("clean_match", True)),
    )
    matches = _as_element_list(result)
    items = [serialize_element(el) for el in _bounded(matches, payload)]
    return {"count": len(matches), "items": items}


def op_find_by_regex(payload: dict[str, Any]) -> dict[str, Any]:
    sel = parse_html(payload["html"])
    result = sel.find_by_regex(
        payload["pattern"],
        first_match=False,
        case_sensitive=bool(payload.get("case_sensitive", False)),
        clean_match=bool(payload.get("clean_match", True)),
    )
    matches = _as_element_list(result)
    items = [serialize_element(el) for el in _bounded(matches, payload)]
    return {"count": len(matches), "items": items}


def op_describe(payload: dict[str, Any]) -> dict[str, Any]:
    """Full identity + structure of the first css/xpath match: text, HTML, attrs,
    both short and full generated selectors, class list, and parent/child/sibling
    context — everything needed to write a stable selector for the element."""
    sel = parse_html(payload["html"])
    query = payload["query"]
    node = (sel.css(query) if payload.get("kind", "css") == "css" else sel.xpath(query)).first
    if node is None:
        return {"found": False}
    parent = node.parent
    element = {
        **serialize_element(node),
        "full_css": str(node.generate_full_css_selector),
        "full_xpath": str(node.generate_full_xpath_selector),
        "classes": [c for c in (node.attrib.get("class") or "").split() if c],
        "parent_tag": parent.tag if parent is not None else None,
        "children": len(node.children),
        "siblings": len(node.siblings),
    }
    return {"found": True, "element": element}


def op_to_markdown(payload: dict[str, Any]) -> dict[str, Any]:
    """Convert HTML to compact Markdown (default), plain text, or cleaned HTML via
    Scrapling's `Convertor` — the same engine behind `scrapling extract *.md`."""
    fmt = payload.get("format", "markdown")
    if fmt not in ("markdown", "text", "html"):
        raise ValueError(f"unsupported format: {fmt}")
    sel = parse_html(payload["html"])
    content = render_content(
        sel,
        fmt,
        main_content_only=bool(payload.get("main_content_only", False)),
        css_selector=payload.get("css_selector"),
    )
    return {"format": fmt, "content": content}
