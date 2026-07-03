"""Typed request/response JSON Schemas + the function registry.

Every function ships a typed request AND response schema (no untyped/AnyValue) —
the publish pipeline runs `--assert-typed-schemas`. `FUNCTIONS` is the single
source of truth that `main.py` iterates to register the worker surface.
"""

from __future__ import annotations

from typing import Any

_OBJECT: dict[str, Any] = {"type": "object"}
_STR = {"type": "string"}
_BOOL = {"type": "boolean"}
_NUM = {"type": "number"}
_STR_LIST = {"type": "array", "items": {"type": "string"}}
# css/xpath/regex results: first-match string, all-matches array, or null.
_RESULT = {"type": ["array", "string", "null"], "items": {"type": "string"}}

_SELECTOR_ITEM = {
    "type": "object",
    "properties": {
        "name": _STR,
        "css": _STR,
        "xpath": _STR,
        "regex": _STR,
        "attr": {**_STR, "description": "extract this attribute instead of text"},
        "html": {**_BOOL, "description": "extract inner HTML instead of text"},
        "all": {**_BOOL, "description": "return every match as a list"},
    },
    "required": ["name"],
}
_SELECTORS = {"type": "array", "items": _SELECTOR_ITEM}

# Fetch output: single page, or {results:[...]} when called with `urls`.
_FETCH_RESPONSE = {
    "type": "object",
    "properties": {
        "status": {"type": ["integer", "null"]},
        "url": _STR,
        "headers": _OBJECT,
        "cookies": _OBJECT,
        "encoding": {"type": ["string", "null"]},
        "extracted": _OBJECT,
        "html": _STR,
        "captured_xhr": {"type": "array", "items": _OBJECT},
        "results": {"type": "array", "items": _OBJECT},
        "error": _STR,
    },
}

_BULK_TARGET = {"url": _STR, "urls": _STR_LIST}
_COMMON_OUT = {"selectors": _SELECTORS, "include_html": _BOOL}

_FETCH_REQUEST = {
    "type": "object",
    "properties": {
        **_BULK_TARGET,
        "method": {"type": "string", "enum": ["get", "post", "put", "delete"]},
        "headers": _OBJECT,
        "params": _OBJECT,
        "data": _OBJECT,
        "json": _OBJECT,
        "cookies": _OBJECT,
        "proxy": _STR,
        "impersonate": {**_STR, "description": "TLS/UA fingerprint, e.g. 'chrome'"},
        "timeout": {**_NUM, "description": "seconds (HTTP fetcher)"},
        "follow_redirects": _BOOL,
        "stealthy_headers": _BOOL,
        **_COMMON_OUT,
    },
}

_BROWSER_WAIT = {
    "headless": _BOOL,
    "network_idle": _BOOL,
    "timeout": {**_NUM, "description": "milliseconds (browser fetcher)"},
    "wait": {**_NUM, "description": "extra ms to wait after load"},
    "wait_selector": _STR,
    "wait_selector_state": {"type": "string", "enum": ["attached", "detached", "visible", "hidden"]},
    "disable_resources": _BOOL,
    "block_ads": _BOOL,
    "proxy": _STR,
    "useragent": _STR,
    "cookies": _OBJECT,
    "google_search": _BOOL,
    "capture_xhr": _STR,
}

_STEALTHY_REQUEST = {
    "type": "object",
    "properties": {
        **_BULK_TARGET,
        **_BROWSER_WAIT,
        "solve_cloudflare": _BOOL,
        "block_webrtc": _BOOL,
        "hide_canvas": _BOOL,
        "allow_webgl": _BOOL,
        **_COMMON_OUT,
    },
}

_DYNAMIC_REQUEST = {
    "type": "object",
    "properties": {
        **_BULK_TARGET,
        **_BROWSER_WAIT,
        "real_chrome": _BOOL,
        "cdp_url": _STR,
        "extra_flags": _STR_LIST,
        **_COMMON_OUT,
    },
}

_SCREENSHOT_REQUEST = {
    "type": "object",
    "properties": {
        "url": _STR,
        "fetcher": {"type": "string", "enum": ["dynamic", "stealthy"]},
        "full_page": _BOOL,
        "format": {"type": "string", "enum": ["png", "jpeg"]},
        "headless": _BOOL,
        "network_idle": _BOOL,
        "timeout": _NUM,
        "wait_selector": _STR,
        "proxy": _STR,
    },
    "required": ["url"],
}
_SCREENSHOT_RESPONSE = {
    "type": "object",
    "properties": {"image_base64": _STR, "mime": _STR, "url": _STR},
}

_EXTRACT_REQUEST = {
    "type": "object",
    "properties": {"html": _STR, "selectors": _SELECTORS, "adaptive": _BOOL},
    "required": ["html", "selectors"],
}
_EXTRACT_RESPONSE = {"type": "object", "properties": {"extracted": _OBJECT}}

_QUERY_REQUEST = {
    "type": "object",
    "properties": {"html": _STR, "query": _STR, "first": _BOOL, "attr": _STR},
    "required": ["html", "query"],
}
_QUERY_RESPONSE = {"type": "object", "properties": {"result": _RESULT}}

_REGEX_REQUEST = {
    "type": "object",
    "properties": {"html": _STR, "pattern": _STR, "first": _BOOL},
    "required": ["html", "pattern"],
}

_FIND_SIMILAR_REQUEST = {
    "type": "object",
    "properties": {
        "html": _STR,
        "anchor": {**_STR, "description": "CSS selector to one example element"},
        "similarity_threshold": _NUM,
        "match_text": _BOOL,
        "selectors": _SELECTORS,
    },
    "required": ["html", "anchor"],
}
_FIND_SIMILAR_RESPONSE = {
    "type": "object",
    "properties": {"count": {"type": "integer"}, "items": {"type": "array", "items": _OBJECT}},
}


# id, handler key (see handlers.create_handlers), description, request, response
FUNCTIONS: list[dict[str, Any]] = [
    {
        "id": "scrapling::fetch",
        "handler": "fetch",
        "description": "Fast HTTP fetch, TLS impersonation: get/post/put/delete, inline extraction, bulk `urls`.",
        "request": _FETCH_REQUEST,
        "response": _FETCH_RESPONSE,
    },
    {
        "id": "scrapling::stealthy-fetch",
        "handler": "stealthy_fetch",
        "description": "Camoufox stealth browser: solves Cloudflare, hardens WebRTC/canvas; extraction + bulk.",
        "request": _STEALTHY_REQUEST,
        "response": _FETCH_RESPONSE,
    },
    {
        "id": "scrapling::dynamic-fetch",
        "handler": "dynamic_fetch",
        "description": "Playwright/Chromium fetch: JS render, waits, XHR capture, CDP; extraction + bulk.",
        "request": _DYNAMIC_REQUEST,
        "response": _FETCH_RESPONSE,
    },
    {
        "id": "scrapling::screenshot",
        "handler": "screenshot",
        "description": "Capture a page screenshot (base64) via a browser fetcher (dynamic or stealthy).",
        "request": _SCREENSHOT_REQUEST,
        "response": _SCREENSHOT_RESPONSE,
    },
    {
        "id": "scrapling::extract",
        "handler": "extract",
        "description": "Parse HTML with a selector list (css/xpath/regex, text/attr/html, all-or-first).",
        "request": _EXTRACT_REQUEST,
        "response": _EXTRACT_RESPONSE,
    },
    {
        "id": "scrapling::css",
        "handler": "css",
        "description": "One CSS query over HTML; first-or-all; `attr` pulls an attribute else text.",
        "request": _QUERY_REQUEST,
        "response": _QUERY_RESPONSE,
    },
    {
        "id": "scrapling::xpath",
        "handler": "xpath",
        "description": "One XPath query over HTML; first-or-all; `attr` pulls an attribute else text.",
        "request": _QUERY_REQUEST,
        "response": _QUERY_RESPONSE,
    },
    {
        "id": "scrapling::regex",
        "handler": "regex",
        "description": "Run a regex over the visible text of provided HTML; `first` returns the first match, else all.",
        "request": _REGEX_REQUEST,
        "response": _QUERY_RESPONSE,
    },
    {
        "id": "scrapling::find-similar",
        "handler": "find_similar",
        "description": "Structural auto-match: given one example element, return it plus similar elements.",
        "request": _FIND_SIMILAR_REQUEST,
        "response": _FIND_SIMILAR_RESPONSE,
    },
]
