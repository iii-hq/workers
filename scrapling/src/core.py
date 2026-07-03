"""Pure, synchronous Scrapling wrappers — no iii bus, no asyncio.

Everything here is import-safe without the browsers: the `scrapling.fetchers`
imports live inside the fetch functions, so parsing (`parse_html`,
`apply_selectors`) and the register surface work with only the base `scrapling`
package installed (browsers are fetched by `scrapling install` in the image).

The `handlers` module wraps these in `asyncio.to_thread` and adds bulk/config.
"""

from __future__ import annotations

import base64
from typing import Any

# JSON-safe, useful kwargs forwarded to each Scrapling fetcher. Deliberately
# excludes callbacks (page_action/page_setup), host paths (executable_path,
# user_data_dir), and non-JSON objects (proxy_rotator, cert, selector_config).
_HTTP_GET_KEYS = frozenset(
    {
        "headers",
        "params",
        "cookies",
        "proxy",
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
    }
)
_STEALTHY_KEYS = _BROWSER_COMMON | {
    "solve_cloudflare",
    "block_webrtc",
    "hide_canvas",
    "allow_webgl",
}
_DYNAMIC_KEYS = _BROWSER_COMMON | {"extra_flags"}


# ---- parsing --------------------------------------------------------------


def parse_html(html: str):
    from scrapling import Selector

    return Selector(content=html)


def _pull(node, spec: dict[str, Any]) -> Any:
    attr = spec.get("attr")
    if attr:
        return node.attrib.get(attr)
    if spec.get("html"):
        return str(node.html_content)
    return str(node.get_all_text())


def apply_selectors(sel, selectors: list[dict[str, Any]]) -> dict[str, Any]:
    """Run a declarative selector list against a Selector/Response.

    Each spec: {name, css|xpath|regex, attr?, html?, all?}. `all` returns a list
    over every match; otherwise the first match's value (or None).
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
        nodes = sel.css(query) if spec.get("css") else sel.xpath(query)
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


def do_fetch(cfg: dict[str, Any], payload: dict[str, Any]) -> dict[str, Any]:
    from scrapling.fetchers import Fetcher

    method = (payload.get("method") or "get").lower()
    if method not in ("get", "post", "put", "delete"):
        raise ValueError(f"unsupported method: {method}")
    keys = _HTTP_GET_KEYS if method == "get" else _HTTP_DATA_KEYS
    kwargs = _pick(payload, keys)
    _default(kwargs, cfg, "impersonate")
    _default(kwargs, cfg, "proxy")
    page = getattr(Fetcher, method)(payload["url"], **kwargs)
    return serialize_page(page, payload, _include_html(cfg, payload))


def do_stealthy(cfg: dict[str, Any], payload: dict[str, Any]) -> dict[str, Any]:
    from scrapling.fetchers import StealthyFetcher

    kwargs = _browser_kwargs(cfg, payload, _STEALTHY_KEYS)
    page = StealthyFetcher.fetch(payload["url"], **kwargs)
    return serialize_page(page, payload, _include_html(cfg, payload))


def do_dynamic(cfg: dict[str, Any], payload: dict[str, Any]) -> dict[str, Any]:
    from scrapling.fetchers import DynamicFetcher

    kwargs = _browser_kwargs(cfg, payload, _DYNAMIC_KEYS)
    page = DynamicFetcher.fetch(payload["url"], **kwargs)
    return serialize_page(page, payload, _include_html(cfg, payload))


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
    sel = parse_html(payload["html"])
    return {"extracted": apply_selectors(sel, payload.get("selectors") or [])}


def op_query(payload: dict[str, Any], kind: str) -> dict[str, Any]:
    sel = parse_html(payload["html"])
    query = payload["query"]
    nodes = sel.css(query) if kind == "css" else sel.xpath(query)
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
