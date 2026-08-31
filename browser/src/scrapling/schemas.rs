//! The 19 scrapling::* request/response schemas, mirrored byte-for-byte from
//! scrapling/src/schemas.py (goldens are generated FROM that file — this
//! module must match it, key order included; serde_json preserve_order).

use serde_json::{json, Value};

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request: Value,
    pub response: Value,
}

fn selector_item() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "css": {"type": "string"},
            "xpath": {"type": "string"},
            "regex": {"type": "string"},
            "attr": {"type": "string", "description": "extract this attribute instead of text"},
            "html": {"type": "boolean", "description": "extract inner HTML instead of text"},
            "all": {"type": "boolean", "description": "return every match as a list"},
        },
        "required": ["name"],
    })
}

fn selectors() -> Value {
    json!({"type": "array", "items": selector_item()})
}

// css/xpath/regex results: first-match string, all-matches array, or null.
fn result_schema() -> Value {
    json!({"type": ["array", "string", "null"], "items": {"type": ["string", "null"]}})
}

fn adaptive_props() -> Vec<(&'static str, Value)> {
    vec![
        (
            "adaptive",
            json!({"type": "boolean", "description": "relocate elements after a site change via saved identities"}),
        ),
        (
            "auto_save",
            json!({"type": "boolean", "description": "save matched identities (defaults on when adaptive)"}),
        ),
        (
            "adaptive_domain",
            json!({"type": "string", "description": "page URL/domain that keys saved identities"}),
        ),
    ]
}

fn with_adaptive(mut base: serde_json::Map<String, Value>) -> Value {
    for (k, v) in adaptive_props() {
        base.insert(k.to_string(), v);
    }
    Value::Object(base)
}

fn extract_request() -> Value {
    let props = json!({
        "html": {"type": "string"},
        "selectors": selectors(),
    });
    json!({
        "type": "object",
        "properties": with_adaptive(props.as_object().unwrap().clone()),
        "required": ["html", "selectors"],
    })
}

fn query_request() -> Value {
    let props = json!({
        "html": {"type": "string"},
        "query": {"type": "string"},
        "first": {"type": "boolean"},
        "attr": {"type": "string"},
        "identifier": {"type": "string", "description": "stable key for the saved element"},
    });
    json!({
        "type": "object",
        "properties": with_adaptive(props.as_object().unwrap().clone()),
        "required": ["html", "query"],
    })
}

fn regex_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "html": {"type": "string"},
            "pattern": {"type": "string"},
            "first": {"type": "boolean"},
        },
        "required": ["html", "pattern"],
    })
}

fn find_similar_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "html": {"type": "string"},
            "anchor": {"type": "string", "description": "CSS selector to one example element"},
            "similarity_threshold": {"type": "number"},
            "match_text": {"type": "boolean"},
            "selectors": selectors(),
        },
        "required": ["html", "anchor"],
    })
}

// One matched element: text, inner HTML, attributes, and the auto-generated
// CSS/XPath selectors that locate it.
fn element() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tag": {"type": "string"},
            "text": {"type": "string"},
            "html": {"type": "string"},
            "attrs": {"type": "object"},
            "css": {"type": "string"},
            "xpath": {"type": "string"},
        },
    })
}

fn elements_response() -> Value {
    json!({
        "type": "object",
        "properties": {
            "count": {"type": "integer"},
            "items": {"type": "array", "items": element()},
        },
    })
}

fn find_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "html": {"type": "string"},
            "tag": {
                "type": ["string", "array"],
                "items": {"type": "string"},
                "description": "tag name or list of tag names",
            },
            "attrs": {"type": "object", "description": "attribute filters, e.g. {\"class\": \"card\"}"},
            "text_regex": {"type": "string", "description": "keep only elements whose text matches this regex"},
            "first": {"type": "boolean"},
            "limit": {"type": "integer"},
        },
        "required": ["html"],
    })
}

fn find_by_text_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "html": {"type": "string"},
            "text": {"type": "string"},
            "partial": {"type": "boolean", "description": "match elements that contain the text"},
            "case_sensitive": {"type": "boolean"},
            "clean_match": {"type": "boolean", "description": "ignore surrounding/collapsing whitespace"},
            "first": {"type": "boolean"},
            "limit": {"type": "integer"},
        },
        "required": ["html", "text"],
    })
}

fn find_by_regex_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "html": {"type": "string"},
            "pattern": {"type": "string"},
            "case_sensitive": {"type": "boolean"},
            "clean_match": {"type": "boolean"},
            "first": {"type": "boolean"},
            "limit": {"type": "integer"},
        },
        "required": ["html", "pattern"],
    })
}

fn describe_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "html": {"type": "string"},
            "query": {"type": "string"},
            "kind": {"type": "string", "enum": ["css", "xpath"]},
        },
        "required": ["html", "query"],
    })
}

// element properties + full_css/full_xpath/classes/parent_tag/children/siblings.
fn describe_response() -> Value {
    let mut props = element()["properties"].as_object().unwrap().clone();
    props.insert("full_css".to_string(), json!({"type": "string"}));
    props.insert("full_xpath".to_string(), json!({"type": "string"}));
    props.insert(
        "classes".to_string(),
        json!({"type": "array", "items": {"type": "string"}}),
    );
    props.insert(
        "parent_tag".to_string(),
        json!({"type": ["string", "null"]}),
    );
    props.insert("children".to_string(), json!({"type": "integer"}));
    props.insert("siblings".to_string(), json!({"type": "integer"}));
    json!({
        "type": "object",
        "properties": {
            "found": {"type": "boolean"},
            "element": {
                "type": "object",
                "properties": Value::Object(props),
            },
        },
    })
}

fn markdown_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "html": {"type": "string"},
            "format": {"type": "string", "enum": ["markdown", "text", "html"]},
            "css_selector": {"type": "string", "description": "convert only the subtree matching this CSS selector"},
            "main_content_only": {"type": "boolean", "description": "strip nav/scripts/hidden nodes first"},
        },
        "required": ["html"],
    })
}

// --- shared fragments for fetch/stealthy-fetch/dynamic-fetch/session-fetch/crawl
// (mirrors schemas.py's _BULK_TARGET/_BROWSER_WAIT/_CONTENT_OUT/_COMMON_OUT) ---

fn obj(props: Vec<(&'static str, Value)>) -> Value {
    Value::Object(props.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn bulk_target_props() -> Vec<(&'static str, Value)> {
    vec![
        ("url", json!({"type": "string"})),
        (
            "urls",
            json!({"type": "array", "items": {"type": "string"}}),
        ),
    ]
}

fn browser_wait_props() -> Vec<(&'static str, Value)> {
    vec![
        ("headless", json!({"type": "boolean"})),
        ("network_idle", json!({"type": "boolean"})),
        ("load_dom", json!({"type": "boolean"})),
        (
            "timeout",
            json!({"type": "number", "description": "milliseconds (browser fetcher)"}),
        ),
        (
            "wait",
            json!({"type": "number", "description": "extra ms to wait after load"}),
        ),
        ("wait_selector", json!({"type": "string"})),
        (
            "wait_selector_state",
            json!({"type": "string", "enum": ["attached", "detached", "visible", "hidden"]}),
        ),
        ("disable_resources", json!({"type": "boolean"})),
        ("block_ads", json!({"type": "boolean"})),
        (
            "blocked_domains",
            json!({"type": "array", "items": {"type": "string"}}),
        ),
        ("proxy", json!({"type": "string"})),
        ("useragent", json!({"type": "string"})),
        ("cookies", json!({"type": "object"})),
        ("extra_headers", json!({"type": "object"})),
        ("google_search", json!({"type": "boolean"})),
        ("capture_xhr", json!({"type": "string"})),
        ("locale", json!({"type": "string"})),
        ("timezone_id", json!({"type": "string"})),
        ("dns_over_https", json!({"type": "boolean"})),
        (
            "extra_flags",
            json!({"type": "array", "items": {"type": "string"}}),
        ),
        ("max_pages", json!({"type": "integer"})),
        ("retries", json!({"type": "integer"})),
        ("retry_delay", json!({"type": "number"})),
    ]
}

// Post-fetch content rendering (reuses Scrapling's Convertor): compact the
// page to markdown/text instead of dumping raw HTML.
fn content_out_props() -> Vec<(&'static str, Value)> {
    vec![
        (
            "format",
            json!({"type": "string", "enum": ["markdown", "text"], "description": "render page body to this format"}),
        ),
        (
            "main_content_only",
            json!({"type": "boolean", "description": "strip nav/scripts/hidden before rendering"}),
        ),
        (
            "css_selector",
            json!({"type": "string", "description": "scope the render to this CSS subtree (e.g. a page's content div)"}),
        ),
    ]
}

// selectors/include_html + content_out: shared tail of every fetch-family request.
fn common_out_props() -> Vec<(&'static str, Value)> {
    let mut props = vec![
        ("selectors", selectors()),
        ("include_html", json!({"type": "boolean"})),
    ];
    props.extend(content_out_props());
    props
}

// fetch/stealthy-fetch/dynamic-fetch/session-fetch: identical page/extraction output.
fn fetch_response() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": ["integer", "null"]},
            "url": {"type": "string"},
            "headers": {"type": "object"},
            "cookies": {"type": "object"},
            "encoding": {"type": ["string", "null"]},
            "extracted": {"type": "object"},
            "html": {"type": "string"},
            "content": {"type": "string", "description": "markdown/text render when `format` requested"},
            "format": {"type": "string"},
            "captured_xhr": {"type": "array", "items": {"type": "object"}},
            "results": {"type": "array", "items": {"type": "object"}},
            "error": {"type": "string"},
        },
    })
}

fn fetch_request() -> Value {
    let mut props = bulk_target_props();
    props.extend([
        ("method", json!({"type": "string", "enum": ["get", "post", "put", "delete"]})),
        ("headers", json!({"type": "object"})),
        ("params", json!({"type": "object"})),
        ("data", json!({"type": "object"})),
        ("json", json!({"type": "object"})),
        ("cookies", json!({"type": "object"})),
        ("proxy", json!({"type": "string"})),
        (
            "proxies",
            json!({"type": "object", "description": "per-scheme proxies, e.g. {\"https\": \"http://...\"}"}),
        ),
        (
            "proxy_auth",
            json!({"type": "array", "items": {"type": "string"}, "description": "[user, password]"}),
        ),
        ("impersonate", json!({"type": "string", "description": "TLS/UA fingerprint, e.g. 'chrome'"})),
        ("timeout", json!({"type": "number", "description": "seconds (HTTP fetcher)"})),
        ("follow_redirects", json!({"type": "boolean"})),
        ("max_redirects", json!({"type": "integer"})),
        ("stealthy_headers", json!({"type": "boolean"})),
        ("http3", json!({"type": "boolean"})),
        ("verify", json!({"type": "boolean"})),
        ("retries", json!({"type": "integer"})),
        ("retry_delay", json!({"type": "number"})),
    ]);
    props.extend(common_out_props());
    json!({"type": "object", "properties": obj(props)})
}

fn stealthy_fetch_request() -> Value {
    let mut props = bulk_target_props();
    props.extend(browser_wait_props());
    props.extend([
        ("solve_cloudflare", json!({"type": "boolean"})),
        ("block_webrtc", json!({"type": "boolean"})),
        ("hide_canvas", json!({"type": "boolean"})),
        ("allow_webgl", json!({"type": "boolean"})),
    ]);
    props.extend(common_out_props());
    json!({"type": "object", "properties": obj(props)})
}

fn dynamic_fetch_request() -> Value {
    let mut props = bulk_target_props();
    props.extend(browser_wait_props());
    props.extend([
        ("real_chrome", json!({"type": "boolean"})),
        ("cdp_url", json!({"type": "string"})),
    ]);
    props.extend(common_out_props());
    json!({"type": "object", "properties": obj(props)})
}

fn screenshot_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {"type": "string"},
            "fetcher": {"type": "string", "enum": ["dynamic", "stealthy"]},
            "full_page": {"type": "boolean"},
            "format": {"type": "string", "enum": ["png", "jpeg"]},
            "headless": {"type": "boolean"},
            "network_idle": {"type": "boolean"},
            "timeout": {"type": "number"},
            "wait_selector": {"type": "string"},
            "proxy": {"type": "string"},
        },
        "required": ["url"],
    })
}

// Harness content blocks: image tiles + a trailing text caption. The harness
// forwards `content` verbatim so the model sees images, never base64 text.
fn screenshot_response() -> Value {
    json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "array",
                "description": "image blocks (one per tile, width<=1024/height<=1536) + a text caption",
                "items": {
                    "type": "object",
                    "properties": {
                        "type": {"type": "string", "enum": ["image", "text"]},
                        "mime": {"type": "string"},
                        "data": {"type": "string", "description": "base64 image bytes (image blocks)"},
                        "text": {"type": "string"},
                    },
                    "required": ["type"],
                },
            },
            "mime": {"type": "string"},
            "url": {"type": "string"},
        },
    })
}

fn session_open_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "type": {"type": "string", "enum": ["http", "dynamic", "stealthy"], "description": "session engine"},
            "impersonate": {"type": "string"},
            "headers": {"type": "object"},
            "proxy": {"type": "string"},
            "proxies": {"type": "object"},
            "headless": {"type": "boolean"},
            "useragent": {"type": "string"},
            "solve_cloudflare": {"type": "boolean"},
            "real_chrome": {"type": "boolean"},
            "timeout": {"type": "number"},
            "capture_xhr": {"type": "string", "description": "regex; capture matching XHRs (browser sessions)"},
        },
    })
}

fn session_open_response() -> Value {
    json!({"type": "object", "properties": {"session_id": {"type": "string"}, "type": {"type": "string"}}})
}

fn session_fetch_request() -> Value {
    let mut props = vec![
        ("session_id", json!({"type": "string"})),
        ("url", json!({"type": "string"})),
        (
            "method",
            json!({"type": "string", "enum": ["get", "post", "put", "delete"]}),
        ),
        ("headers", json!({"type": "object"})),
        ("params", json!({"type": "object"})),
        ("data", json!({"type": "object"})),
        ("json", json!({"type": "object"})),
        ("wait_selector", json!({"type": "string"})),
    ];
    props.extend(common_out_props());
    json!({
        "type": "object",
        "properties": obj(props),
        "required": ["session_id", "url"],
    })
}

fn session_close_request() -> Value {
    json!({"type": "object", "properties": {"session_id": {"type": "string"}}, "required": ["session_id"]})
}

fn session_close_response() -> Value {
    json!({"type": "object", "properties": {"closed": {"type": "boolean"}}})
}

fn session_list_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "type": {"type": "string", "enum": ["http", "dynamic", "stealthy"], "description": "filter by type"},
        },
    })
}

fn session_list_response() -> Value {
    json!({
        "type": "object",
        "properties": {
            "sessions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "session_id": {"type": "string"},
                        "type": {"type": "string"},
                        "created_at": {"type": "number"},
                        "last_used": {"type": "number"},
                        "idle_s": {"type": "number"},
                    },
                },
            },
        },
    })
}

fn crawl_request() -> Value {
    let mut props = vec![
        (
            "start_urls",
            json!({"type": "array", "items": {"type": "string"}}),
        ),
        (
            "url",
            json!({"type": "string", "description": "single start URL (alternative to start_urls)"}),
        ),
        (
            "fetcher",
            json!({"type": "string", "enum": ["http", "stealthy", "dynamic"]}),
        ),
        ("selectors", selectors()),
        (
            "allowed_domains",
            json!({"type": "array", "items": {"type": "string"}, "description": "only follow links on these hosts"}),
        ),
        (
            "same_domain",
            json!({"type": "boolean", "description": "follow only same-host links (default true)"}),
        ),
        ("max_pages", json!({"type": "integer"})),
        ("max_depth", json!({"type": "integer"})),
        ("concurrency", json!({"type": "integer"})),
        (
            "download_delay",
            json!({"type": "number", "description": "seconds to wait between crawl rounds"}),
        ),
    ];
    props.extend(content_out_props());
    props.extend([
        ("include_html", json!({"type": "boolean"})),
        ("impersonate", json!({"type": "string"})),
        (
            "stream_name",
            json!({"type": "string", "description": "stream to emit items on (default browser::crawl)"}),
        ),
    ]);
    json!({"type": "object", "properties": obj(props)})
}

fn crawl_response() -> Value {
    json!({
        "type": "object",
        "properties": {
            "stats": {
                "type": "object",
                "properties": {
                    "crawled": {"type": "integer"},
                    "items": {"type": "integer"},
                    "errors": {"type": "integer"},
                    "stopped": {"type": "string"},
                },
            },
            "items": {"type": "array", "items": {"type": "object"}, "description": "a small sample of streamed items"},
            "stream": {
                "type": "object",
                "properties": {"name": {"type": "string"}, "group_id": {"type": "string"}},
                "description": "read the full item stream via stream::on with this name + group_id",
            },
        },
    })
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        FunctionSpec {
            function_id: "browser::fetch",
            description: "Fast HTTP fetch, TLS impersonation: get/post/put/delete, inline extraction, bulk `urls`.",
            request: fetch_request(),
            response: fetch_response(),
        },
        FunctionSpec {
            function_id: "browser::stealthy-fetch",
            description: "Camoufox stealth browser: solves Cloudflare, hardens WebRTC/canvas; extraction + bulk.",
            request: stealthy_fetch_request(),
            response: fetch_response(),
        },
        FunctionSpec {
            function_id: "browser::dynamic-fetch",
            description: "Playwright/Chromium fetch: JS render, waits, XHR capture, CDP; extraction + bulk.",
            request: dynamic_fetch_request(),
            response: fetch_response(),
        },
        FunctionSpec {
            function_id: "browser::screenshot-url",
            description: "Capture a page screenshot as image content blocks via a browser fetcher (dynamic or stealthy).",
            request: screenshot_request(),
            response: screenshot_response(),
        },
        FunctionSpec {
            function_id: "browser::extract",
            description: "Parse HTML with a selector list (css/xpath/regex, text/attr/html, all-or-first).",
            request: extract_request(),
            response: json!({"type": "object", "properties": {"extracted": {"type": "object"}}}),
        },
        FunctionSpec {
            function_id: "browser::css",
            description: "One CSS query over HTML; first-or-all; `attr` pulls an attribute else text.",
            request: query_request(),
            response: json!({"type": "object", "properties": {"result": result_schema()}}),
        },
        FunctionSpec {
            function_id: "browser::xpath",
            description: "One XPath query over HTML; first-or-all; `attr` pulls an attribute else text.",
            request: query_request(),
            response: json!({"type": "object", "properties": {"result": result_schema()}}),
        },
        FunctionSpec {
            function_id: "browser::regex",
            description: "Run a regex over the visible text of provided HTML; `first` returns the first match, else all.",
            request: regex_request(),
            response: json!({"type": "object", "properties": {"result": result_schema()}}),
        },
        FunctionSpec {
            function_id: "browser::find-similar",
            description: "Structural auto-match: given one example element, return it plus similar elements.",
            request: find_similar_request(),
            response: json!({
                "type": "object",
                "properties": {
                    "count": {"type": "integer"},
                    "items": {"type": "array", "items": {"type": "object"}},
                },
            }),
        },
        FunctionSpec {
            function_id: "browser::find",
            description: "Find elements by tag/attribute filters (+ optional text regex); BeautifulSoup-style.",
            request: find_request(),
            response: elements_response(),
        },
        FunctionSpec {
            function_id: "browser::find-by-text",
            description: "Find elements whose visible text matches a string (exact or `partial`).",
            request: find_by_text_request(),
            response: elements_response(),
        },
        FunctionSpec {
            function_id: "browser::find-by-regex",
            description: "Find elements whose visible text matches a regex pattern.",
            request: find_by_regex_request(),
            response: elements_response(),
        },
        FunctionSpec {
            function_id: "browser::describe",
            description: "Describe the first css/xpath match: attrs, generated selectors, class list, DOM context.",
            request: describe_request(),
            response: describe_response(),
        },
        FunctionSpec {
            function_id: "browser::to-markdown",
            description: "Convert HTML to compact Markdown (or text/html); optional CSS scope + main-content clean.",
            request: markdown_request(),
            response: json!({"type": "object", "properties": {"format": {"type": "string"}, "content": {"type": "string"}}}),
        },
        FunctionSpec {
            function_id: "browser::session-open",
            description: "Open a persistent scraping session (HTTP or headless fetcher) whose session_id reuses cookies and state across fetches. Renders nothing the user can see; to open a page in the visible browser use browser::sessions::start.",
            request: session_open_request(),
            response: session_open_response(),
        },
        FunctionSpec {
            function_id: "browser::session-fetch",
            description: "Fetch a URL on an open scraping session (reuses its cookies/state); returns page content, shows nothing. For a page the user should see, use browser::sessions::start + browser::navigate.",
            request: session_fetch_request(),
            response: fetch_response(),
        },
        FunctionSpec {
            function_id: "browser::session-close",
            description: "Close a session and free its browser/connection.",
            request: session_close_request(),
            response: session_close_response(),
        },
        FunctionSpec {
            function_id: "browser::session-list",
            description: "List open sessions with their type and idle time.",
            request: session_list_request(),
            response: session_list_response(),
        },
        FunctionSpec {
            function_id: "browser::crawl",
            description: "BFS-crawl from start_urls (follow same-domain links), extract per page, stream items.",
            request: crawl_request(),
            response: crawl_response(),
        },
    ]
}
