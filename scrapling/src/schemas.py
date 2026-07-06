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
# Array items may be null too — an `attr` miss inside an `all` query yields null.
_RESULT = {"type": ["array", "string", "null"], "items": {"type": ["string", "null"]}}

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
        "content": {**_STR, "description": "markdown/text render when `format` requested"},
        "format": _STR,
        "captured_xhr": {"type": "array", "items": _OBJECT},
        "results": {"type": "array", "items": _OBJECT},
        "error": _STR,
    },
}

_BULK_TARGET = {"url": _STR, "urls": _STR_LIST}
# Post-fetch content rendering (reuses Scrapling's Convertor): compact the page
# to markdown/text instead of dumping raw HTML.
_CONTENT_OUT = {
    "format": {"type": "string", "enum": ["markdown", "text"], "description": "render page body to this format"},
    "main_content_only": {**_BOOL, "description": "strip nav/scripts/hidden before rendering"},
}
_COMMON_OUT = {"selectors": _SELECTORS, "include_html": _BOOL, **_CONTENT_OUT}

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
        "proxies": {**_OBJECT, "description": 'per-scheme proxies, e.g. {"https": "http://..."}'},
        "proxy_auth": {"type": "array", "items": _STR, "description": "[user, password]"},
        "impersonate": {**_STR, "description": "TLS/UA fingerprint, e.g. 'chrome'"},
        "timeout": {**_NUM, "description": "seconds (HTTP fetcher)"},
        "follow_redirects": _BOOL,
        "max_redirects": {"type": "integer"},
        "stealthy_headers": _BOOL,
        "http3": _BOOL,
        "verify": _BOOL,
        "retries": {"type": "integer"},
        "retry_delay": _NUM,
        **_COMMON_OUT,
    },
}

_BROWSER_WAIT = {
    "headless": _BOOL,
    "network_idle": _BOOL,
    "load_dom": _BOOL,
    "timeout": {**_NUM, "description": "milliseconds (browser fetcher)"},
    "wait": {**_NUM, "description": "extra ms to wait after load"},
    "wait_selector": _STR,
    "wait_selector_state": {"type": "string", "enum": ["attached", "detached", "visible", "hidden"]},
    "disable_resources": _BOOL,
    "block_ads": _BOOL,
    "blocked_domains": _STR_LIST,
    "proxy": _STR,
    "useragent": _STR,
    "cookies": _OBJECT,
    "extra_headers": _OBJECT,
    "google_search": _BOOL,
    "capture_xhr": _STR,
    "locale": _STR,
    "timezone_id": _STR,
    "dns_over_https": _BOOL,
    "extra_flags": _STR_LIST,
    "max_pages": {"type": "integer"},
    "retries": {"type": "integer"},
    "retry_delay": _NUM,
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
# Harness content blocks: image tiles + a trailing text caption. The harness
# forwards `content` verbatim so the model sees images, never base64 text.
_SCREENSHOT_RESPONSE = {
    "type": "object",
    "properties": {
        "content": {
            "type": "array",
            "description": "image blocks (one per tile, width<=1024/height<=1536) + a text caption",
            "items": {
                "type": "object",
                "properties": {
                    "type": {"type": "string", "enum": ["image", "text"]},
                    "mime": _STR,
                    "data": {**_STR, "description": "base64 image bytes (image blocks)"},
                    "text": _STR,
                },
                "required": ["type"],
            },
        },
        "mime": _STR,
        "url": _STR,
    },
}

# Smart Element Tracking (adaptive relocation). `adaptive_domain` keys the saved
# identities per site; `identifier` names a single css/xpath match (extract keys
# each match by its selector `name`).
_ADAPTIVE = {
    "adaptive": {**_BOOL, "description": "relocate elements after a site change via saved identities"},
    "auto_save": {**_BOOL, "description": "save matched identities (defaults on when adaptive)"},
    "adaptive_domain": {**_STR, "description": "page URL/domain that keys saved identities"},
}

_EXTRACT_REQUEST = {
    "type": "object",
    "properties": {"html": _STR, "selectors": _SELECTORS, **_ADAPTIVE},
    "required": ["html", "selectors"],
}
_EXTRACT_RESPONSE = {"type": "object", "properties": {"extracted": _OBJECT}}

_QUERY_REQUEST = {
    "type": "object",
    "properties": {
        "html": _STR,
        "query": _STR,
        "first": _BOOL,
        "attr": _STR,
        "identifier": {**_STR, "description": "stable key for the saved element"},
        **_ADAPTIVE,
    },
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

# One matched element: text, inner HTML, attributes, and the auto-generated
# CSS/XPath selectors that locate it.
_ELEMENT = {
    "type": "object",
    "properties": {"tag": _STR, "text": _STR, "html": _STR, "attrs": _OBJECT, "css": _STR, "xpath": _STR},
}
_ELEMENTS_RESPONSE = {
    "type": "object",
    "properties": {"count": {"type": "integer"}, "items": {"type": "array", "items": _ELEMENT}},
}

_FIND_REQUEST = {
    "type": "object",
    "properties": {
        "html": _STR,
        "tag": {"type": ["string", "array"], "items": _STR, "description": "tag name or list of tag names"},
        "attrs": {**_OBJECT, "description": 'attribute filters, e.g. {"class": "card"}'},
        "text_regex": {**_STR, "description": "keep only elements whose text matches this regex"},
        "first": _BOOL,
        "limit": {"type": "integer"},
    },
    "required": ["html"],
}

_FIND_BY_TEXT_REQUEST = {
    "type": "object",
    "properties": {
        "html": _STR,
        "text": _STR,
        "partial": {**_BOOL, "description": "match elements that contain the text"},
        "case_sensitive": _BOOL,
        "clean_match": {**_BOOL, "description": "ignore surrounding/collapsing whitespace"},
        "first": _BOOL,
        "limit": {"type": "integer"},
    },
    "required": ["html", "text"],
}

_FIND_BY_REGEX_REQUEST = {
    "type": "object",
    "properties": {
        "html": _STR,
        "pattern": _STR,
        "case_sensitive": _BOOL,
        "clean_match": _BOOL,
        "first": _BOOL,
        "limit": {"type": "integer"},
    },
    "required": ["html", "pattern"],
}

_DESCRIBE_REQUEST = {
    "type": "object",
    "properties": {
        "html": _STR,
        "query": _STR,
        "kind": {"type": "string", "enum": ["css", "xpath"]},
    },
    "required": ["html", "query"],
}
_DESCRIBE_RESPONSE = {
    "type": "object",
    "properties": {
        "found": _BOOL,
        "element": {
            "type": "object",
            "properties": {
                **_ELEMENT["properties"],
                "full_css": _STR,
                "full_xpath": _STR,
                "classes": _STR_LIST,
                "parent_tag": {"type": ["string", "null"]},
                "children": {"type": "integer"},
                "siblings": {"type": "integer"},
            },
        },
    },
}

_MARKDOWN_REQUEST = {
    "type": "object",
    "properties": {
        "html": _STR,
        "format": {"type": "string", "enum": ["markdown", "text", "html"]},
        "css_selector": {**_STR, "description": "convert only the subtree matching this CSS selector"},
        "main_content_only": {**_BOOL, "description": "strip nav/scripts/hidden nodes first"},
    },
    "required": ["html"],
}
_MARKDOWN_RESPONSE = {
    "type": "object",
    "properties": {"format": _STR, "content": _STR},
}

# Persistent sessions: open once, reuse cookies/browser across fetches, close.
_SESSION_OPEN_REQUEST = {
    "type": "object",
    "properties": {
        "type": {"type": "string", "enum": ["http", "dynamic", "stealthy"], "description": "session engine"},
        "impersonate": _STR,
        "headers": _OBJECT,
        "proxy": _STR,
        "proxies": _OBJECT,
        "headless": _BOOL,
        "useragent": _STR,
        "solve_cloudflare": _BOOL,
        "real_chrome": _BOOL,
        "timeout": _NUM,
        # capture_xhr is a browser-session CONSTRUCTOR option — set it here, not
        # per-fetch (Scrapling ignores it on session.fetch()).
        "capture_xhr": {**_STR, "description": "regex; capture matching XHRs (browser sessions)"},
    },
}
_SESSION_OPEN_RESPONSE = {
    "type": "object",
    "properties": {"session_id": _STR, "type": _STR},
}

_SESSION_FETCH_REQUEST = {
    "type": "object",
    "properties": {
        "session_id": _STR,
        "url": _STR,
        # method/params/data/json apply to HTTP sessions; a browser session
        # navigates GET and honors `headers` (mapped to the page's extra headers).
        "method": {"type": "string", "enum": ["get", "post", "put", "delete"]},
        "headers": _OBJECT,
        "params": _OBJECT,
        "data": _OBJECT,
        "json": _OBJECT,
        "wait_selector": _STR,
        **_COMMON_OUT,
    },
    "required": ["session_id", "url"],
}

_SESSION_CLOSE_REQUEST = {
    "type": "object",
    "properties": {"session_id": _STR},
    "required": ["session_id"],
}
_SESSION_CLOSE_RESPONSE = {"type": "object", "properties": {"closed": _BOOL}}

_SESSION_LIST_REQUEST = {
    "type": "object",
    "properties": {
        "type": {"type": "string", "enum": ["http", "dynamic", "stealthy"], "description": "filter by type"}
    },
}
_SESSION_LIST_RESPONSE = {
    "type": "object",
    "properties": {
        "sessions": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "session_id": _STR,
                    "type": _STR,
                    "created_at": _NUM,
                    "last_used": _NUM,
                    "idle_s": _NUM,
                },
            },
        }
    },
}

# Declarative crawl: BFS from start_urls, extract per page, stream items.
_CRAWL_REQUEST = {
    "type": "object",
    "properties": {
        "start_urls": _STR_LIST,
        "url": {**_STR, "description": "single start URL (alternative to start_urls)"},
        "fetcher": {"type": "string", "enum": ["http", "stealthy", "dynamic"]},
        "selectors": _SELECTORS,
        "allowed_domains": {**_STR_LIST, "description": "only follow links on these hosts"},
        "same_domain": {**_BOOL, "description": "follow only same-host links (default true)"},
        "max_pages": {"type": "integer"},
        "max_depth": {"type": "integer"},
        "concurrency": {"type": "integer"},
        "download_delay": {**_NUM, "description": "seconds to wait between crawl rounds"},
        "format": {"type": "string", "enum": ["markdown", "text"]},
        "include_html": _BOOL,
        "impersonate": _STR,
        "stream_name": {**_STR, "description": "stream to emit items on (default scrapling::crawl)"},
    },
}
_CRAWL_RESPONSE = {
    "type": "object",
    "properties": {
        "stats": {
            "type": "object",
            "properties": {
                "crawled": {"type": "integer"},
                "items": {"type": "integer"},
                "errors": {"type": "integer"},
                "stopped": _STR,
            },
        },
        "items": {"type": "array", "items": _OBJECT, "description": "a small sample of streamed items"},
        "stream": {
            "type": "object",
            "properties": {"name": _STR, "group_id": _STR},
            "description": "read the full item stream via stream::on with this name + group_id",
        },
    },
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
        "description": "Capture a page screenshot as image content blocks via a browser fetcher (dynamic or stealthy).",
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
    {
        "id": "scrapling::find",
        "handler": "find",
        "description": "Find elements by tag/attribute filters (+ optional text regex); BeautifulSoup-style.",
        "request": _FIND_REQUEST,
        "response": _ELEMENTS_RESPONSE,
    },
    {
        "id": "scrapling::find-by-text",
        "handler": "find_by_text",
        "description": "Find elements whose visible text matches a string (exact or `partial`).",
        "request": _FIND_BY_TEXT_REQUEST,
        "response": _ELEMENTS_RESPONSE,
    },
    {
        "id": "scrapling::find-by-regex",
        "handler": "find_by_regex",
        "description": "Find elements whose visible text matches a regex pattern.",
        "request": _FIND_BY_REGEX_REQUEST,
        "response": _ELEMENTS_RESPONSE,
    },
    {
        "id": "scrapling::describe",
        "handler": "describe",
        "description": "Describe the first css/xpath match: attrs, generated selectors, class list, DOM context.",
        "request": _DESCRIBE_REQUEST,
        "response": _DESCRIBE_RESPONSE,
    },
    {
        "id": "scrapling::to-markdown",
        "handler": "to_markdown",
        "description": "Convert HTML to compact Markdown (or text/html); optional CSS scope + main-content clean.",
        "request": _MARKDOWN_REQUEST,
        "response": _MARKDOWN_RESPONSE,
    },
    {
        "id": "scrapling::session-open",
        "handler": "session_open",
        "description": "Open a persistent HTTP/browser session; returns a session_id that reuses cookies + state.",
        "request": _SESSION_OPEN_REQUEST,
        "response": _SESSION_OPEN_RESPONSE,
    },
    {
        "id": "scrapling::session-fetch",
        "handler": "session_fetch",
        "description": "Fetch a URL on an open session (reuses its cookies/browser); same page/extraction output.",
        "request": _SESSION_FETCH_REQUEST,
        "response": _FETCH_RESPONSE,
    },
    {
        "id": "scrapling::session-close",
        "handler": "session_close",
        "description": "Close a session and free its browser/connection.",
        "request": _SESSION_CLOSE_REQUEST,
        "response": _SESSION_CLOSE_RESPONSE,
    },
    {
        "id": "scrapling::session-list",
        "handler": "session_list",
        "description": "List open sessions with their type and idle time.",
        "request": _SESSION_LIST_REQUEST,
        "response": _SESSION_LIST_RESPONSE,
    },
    {
        "id": "scrapling::crawl",
        "handler": "crawl",
        "description": "BFS-crawl from start_urls (follow same-domain links), extract per page, stream items.",
        "request": _CRAWL_REQUEST,
        "response": _CRAWL_RESPONSE,
    },
]
